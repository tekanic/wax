//! Conversations: the reader shows every message in the selected message's
//! Gmail thread (X-GM-THRID), your replies included, as a strip above the
//! message. Clicking one reads it in place; the list itself stays one row
//! per message, with the conversation's size beside the name.
//!
//! The thread comes from the cache at once, then from Gmail's All Mail
//! (`Session::thread`), which also has the replies you sent from other
//! clients.

use super::*;
use std::collections::HashMap;

/// Most messages of one conversation fetched from Gmail.
const THREAD_LIMIT: usize = 100;

pub struct Conversation {
    pub account: String,
    pub thread: String,
    /// The list row it was opened from.
    pub list_key: (String, String),
    /// Oldest first, one copy of each message.
    pub items: Vec<Envelope>,
    /// The message the reader shows when it isn't the list row's.
    pub open: Option<usize>,
}

impl Conversation {
    /// Whether `e` is the message the list row stands for.
    fn is_row(&self, e: &Envelope, row: &Envelope) -> bool {
        match (&e.message_id, &row.message_id) {
            (Some(a), Some(b)) => a == b,
            _ => e.mailbox == row.mailbox && e.id == row.id,
        }
    }
}

/// A short name for a person: their name, else the part before the @.
fn short(name: &str, email: &str) -> String {
    let n = name.trim().trim_matches('"').trim();
    // "Last, First" → First.
    let n = match n.split_once(',') {
        Some((_, first)) if !first.trim().is_empty() => first.trim(),
        Some((last, _)) => last.trim(),
        None => n,
    };
    if !n.is_empty() {
        return n.split_whitespace().next().unwrap_or(n).to_string();
    }
    email.split('@').next().unwrap_or(email).to_string()
}

/// "To: Ann, Bob +2" for a message you sent.
pub fn recipients_line(to: &[(String, String)]) -> String {
    let names: Vec<String> = to.iter().map(|(n, e)| short(n, e)).collect();
    match names.len() {
        0 => "To: (no recipients)".into(),
        1..=3 => format!("To: {}", names.join(", ")),
        n => format!("To: {} +{}", names[..2].join(", "), n - 2),
    }
}

impl MailApp {
    /// The message the reader shows: one picked in the conversation strip,
    /// or the selected row.
    pub fn reading(&self) -> Option<&Envelope> {
        let row = self.envelopes.get(self.selected)?;
        match &self.conversation {
            Some(c) if c.list_key == row.key() => c.open.and_then(|i| c.items.get(i)).or(Some(row)),
            _ => Some(row),
        }
    }

    /// Your addresses for an account: its own and its From identities.
    pub fn own_addresses(&self, account: &str) -> Vec<String> {
        std::iter::once(account.to_lowercase())
            .chain(self.identities_for(account).into_iter().map(|i| i.email.to_lowercase()))
            .collect()
    }

    /// The name a list row shows: recipients for mail you sent.
    pub fn row_title(&self, e: &Envelope) -> SharedString {
        if e.mailbox.eq_ignore_ascii_case(mail::SENT) && !e.to.is_empty() {
            return recipients_line(&e.to).into();
        }
        e.from.clone()
    }

    /// How many messages the row's conversation has (when more than one).
    pub fn row_thread_count(&self, e: &Envelope) -> Option<usize> {
        let cached = e.thread.clone().and_then(|t| self.thread_counts.get(&(self.account_of(e).to_string(), t)).copied()).unwrap_or(0);
        let n = cached.max(self.row_size(e));
        (n > 1).then_some(n)
    }

    /// Count the conversations of the rows on screen (from the cache).
    pub fn refresh_thread_counts(&mut self) {
        let Some(store) = self.store.clone() else { return };
        let mut by_account: HashMap<String, Vec<String>> = HashMap::new();
        for e in &self.envelopes {
            if let Some(t) = &e.thread {
                by_account.entry(self.account_of(e).to_string()).or_default().push(t.clone());
            }
        }
        let store = store.lock().unwrap();
        self.thread_counts.clear();
        for (account, mut threads) in by_account {
            threads.sort();
            threads.dedup();
            if let Ok(counts) = store.thread_counts(&account, &threads) {
                self.thread_counts.extend(counts.into_iter().map(|(t, n)| ((account.clone(), t), n)));
            }
        }
    }

    /// Show the selected row's conversation: the cache now, Gmail after.
    pub fn load_conversation(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.envelopes.get(self.selected).cloned() else {
            self.conversation = None;
            return;
        };
        let Some(thread) = row.thread.clone() else {
            self.conversation = None;
            return;
        };
        let account = self.account_of(&row).to_string();
        if let Some(c) = &mut self.conversation {
            if c.account == account && c.thread == thread {
                // Same conversation, another of its rows: show that one.
                if c.list_key != row.key() {
                    c.list_key = row.key();
                    c.open = None;
                }
                return;
            }
        }
        let cached = self.store.as_ref().and_then(|s| s.lock().unwrap().thread(&account, &thread).ok()).unwrap_or_default();
        self.conversation = Some(Conversation { account: account.clone(), thread: thread.clone(), list_key: row.key(), items: cached, open: None });
        let (Some(session), Some(store)) = (self.preview_of(&account), self.store.clone()) else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // Moving through the list quickly: only ask Gmail where you stop.
            cx.background_executor().timer(std::time::Duration::from_millis(350)).await;
            let still = this
                .update(cx, |this, _| this.conversation.as_ref().is_some_and(|c| c.account == account && c.thread == thread))
                .unwrap_or(false);
            if !still {
                return;
            }
            let (a, t) = (account.clone(), thread.clone());
            let result = cx
                .background_executor()
                .spawn(async move {
                    let fresh = session.lock().unwrap().thread(&t, THREAD_LIMIT)?;
                    let mut store = store.lock().unwrap();
                    store.upsert_envelopes(&a, &fresh)?;
                    store.thread(&a, &t)
                })
                .await;
            this.update(cx, |this, cx| {
                let Ok(items) = result else { return };
                let n = items.len();
                if let Some(c) = &mut this.conversation {
                    if c.account == account && c.thread == thread {
                        // Keep the message being read open.
                        let reading = c.open.and_then(|i| c.items.get(i)).map(|e| (e.mailbox.clone(), e.id.clone(), e.message_id.clone()));
                        c.items = items;
                        c.open = reading.and_then(|(m, id, mid)| {
                            c.items.iter().position(|e| (mid.is_some() && e.message_id == mid) || (e.mailbox == m && e.id == id))
                        });
                        this.thread_counts.insert((account.clone(), thread.clone()), n);
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Read message `i` of the conversation in place.
    pub fn open_in_conversation(&mut self, i: usize, cx: &mut Context<Self>) {
        let Some(row) = self.envelopes.get(self.selected).cloned() else { return };
        let Some(c) = &mut self.conversation else { return };
        let Some(e) = c.items.get(i).cloned() else { return };
        c.open = if c.is_row(&e, &row) { None } else { Some(i) };
        if c.open.is_some() && e.body.is_none() {
            let account = c.account.clone();
            if let Some(session) = self.session_of(&account) {
                let (mailbox, id) = (e.mailbox.clone(), e.id.clone());
                cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                    let (mb, fid) = (mailbox.clone(), id.clone());
                    let result = cx.background_executor().spawn(async move { session.lock().unwrap().body(&mb, &fid) }).await;
                    this.update(cx, |this, cx| match result {
                        Ok((text, html, files)) => this.apply_body(&account, &mailbox, &id, text, html, files, cx),
                        Err(err) => this.show_body_error(&account, &mailbox, &id, format!("Could not load message: {err:#}"), cx),
                    })
                    .ok();
                })
                .detach();
            }
        }
        cx.notify();
    }

    /// A fetched body also fills the conversation's copy of the message.
    pub fn apply_body_to_conversation(&mut self, account: &str, mailbox: &str, id: &str, text: &str, html: &Option<String>, files: &[model::Attachment]) {
        let Some(c) = &mut self.conversation else { return };
        if c.account != account {
            return;
        }
        for e in c.items.iter_mut().filter(|e| e.mailbox == mailbox && e.id == id && e.body.is_none()) {
            e.preview = make_preview(text).into();
            e.body = Some(text.to_string().into());
            e.html = html.clone().map(Into::into);
            e.attachments = files.to_vec();
        }
    }

    /// Every message of the conversation, above the one being read.
    pub fn render_conversation_strip(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = self.envelopes.get(self.selected)?;
        let c = self.conversation.as_ref().filter(|c| c.list_key == row.key() && c.items.len() > 1)?;
        let mine = self.own_addresses(&c.account);
        let shown = c.open.unwrap_or_else(|| c.items.iter().position(|e| c.is_row(e, row)).unwrap_or(usize::MAX));
        let mut list = div().flex().flex_col();
        for (i, e) in c.items.iter().enumerate() {
            let me = mine.contains(&e.from_email.to_lowercase());
            let who = if me { format!("You · {}", recipients_line(&e.to)) } else { e.from.to_string() };
            let on = i == shown;
            list = list.child(
                div()
                    .id(("conv", i))
                    .flex()
                    .flex_col()
                    .px_3()
                    .py(px(6.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(on, |d| d.bg(pal().selected))
                    .when(!on, |d| d.hover(|d| d.bg(pal().hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.open_in_conversation(i, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .text_xs()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                                    .text_color(if me { pal().accent } else { pal().text })
                                    .child(who),
                            )
                            .child(div().flex_none().text_color(pal().muted).child(e.date.clone())),
                    )
                    .when(!e.preview.is_empty(), |d| d.child(div().truncate().text_xs().text_color(pal().muted).child(e.preview.clone()))),
            );
        }
        Some(
            div()
                .flex_none()
                .max_w(px(760.))
                .mx_6()
                .mt_4()
                .flex()
                .flex_col()
                .gap_1()
                .child(div().px_3().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(pal().muted).child(format!("Conversation · {} messages", c.items.len())))
                .child(div().id("conv-list").max_h(px(220.)).overflow_y_scroll().child(list))
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::recipients_line;

    #[test]
    fn names_recipients_briefly() {
        let p = |n: &str, e: &str| (n.to_string(), e.to_string());
        assert_eq!(recipients_line(&[p("Ann Lee", "ann@x.com")]), "To: Ann");
        assert_eq!(recipients_line(&[p("", "bob@x.com"), p("\"Cy\"", "cy@x.com")]), "To: bob, Cy");
        let four = [p("A", "a@x"), p("B", "b@x"), p("C", "c@x"), p("D", "d@x")];
        assert_eq!(recipients_line(&four), "To: A, B +2");
        assert_eq!(recipients_line(&[]), "To: (no recipients)");
        assert_eq!(recipients_line(&[p("Smith, Dana", "d@x.com"), p("Ward,", "w@x.com")]), "To: Dana, Ward");
    }
}

fn reader_button(id: &'static str, icon: &'static str, label: &'static str, key: &'static str, compact: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .h(px(28.))
        .px_2p5()
        .rounded_md()
        .border_1()
        .border_color(pal().border)
        .text_sm()
        .text_color(pal().text)
        .cursor_pointer()
        .hover(|d| d.bg(pal().hover))
        .child(svg().path(icon).size(px(14.)).flex_none().text_color(pal().subtle))
        .child(label)
        .when(!compact, |d| d.child(div().text_xs().font_family("Menlo").text_color(pal().muted).child(key)))
}

impl MailApp {
    pub fn render_reader_actions(&self, e: &Envelope, cx: &mut Context<Self>) -> Div {
        let compact = self.compact();
        let mut row = div().flex().flex_wrap().items_center().gap_2();
        if e.mailbox == mail::DRAFTS && !self.unified {
            return row.child(action_button("edit-draft", "Edit  ⏎", true).on_click(cx.listener(|this, _, w, cx| this.open_draft(&OpenDraft, w, cx))));
        }
        row = row
            .child(reader_button("reply", "icons/reply.svg", "Reply", "r", compact).on_click(cx.listener(|this, _, w, cx| this.respond(Kind::Reply, w, cx))))
            .child(
                reader_button("reply-all", "icons/reply-all.svg", "Reply all", "a", compact)
                    .on_click(cx.listener(|this, _, w, cx| this.respond(Kind::ReplyAll, w, cx))),
            )
            .child(reader_button("forward", "icons/forward.svg", "Forward", "f", compact).on_click(cx.listener(|this, _, w, cx| this.respond(Kind::Forward, w, cx))));
        row.children(self.render_command_buttons(e, cx))
    }
}

