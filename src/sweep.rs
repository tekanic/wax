//! Sender views (the Sweep): decide about senders, not messages.
//!
//! The Sweep itself is a plugin (plugins/sweep): it ranks senders through
//! the `senders` host call and chooses the keys and where each files a
//! sender's mail. The core draws the view, holds each `file-sender`
//! decision for the undo window, then writes the sender rule and moves the
//! mail: on Gmail the sender's mail gets the label and leaves the Inbox,
//! and the rule makes new mail from them do the same. Sender rules stay in
//! the core because the Screener and block/allow in the message list use
//! them too.

use super::*;
use crate::store::SenderStat;

pub const READING: &str = "Reading";
pub const RECORDS: &str = "Records";
pub const SCREENED: &str = "Screened Out";
pub const KEEP: &str = "keep";
/// Where the Screener plugin holds a first email from someone new. The
/// core keeps the Lobby: deciding about a sender empties theirs.
pub const LOBBY: &str = "Lobby";
const INBOX: &str = mail::INBOX;

/// Label everything from `email` with `dest` and take it out of the Inbox.
/// Covers cached All Mail copies plus whatever is in the Inbox right now
/// (new mail may not be cached yet). Returns how many messages it touched.
pub fn apply_sender(session: &SharedSession, store: &SharedStore, acct: &str, email: &str, dest: &str) -> anyhow::Result<usize> {
    if dest == KEEP {
        return Ok(0);
    }
    // Locks are taken per step so the UI never waits on a long batch.
    let all = store.lock().unwrap().uids_from(acct, ALL_MAIL, email)?;
    session.lock().unwrap().ensure_mailbox(dest)?;
    session.lock().unwrap().copy_ids(ALL_MAIL, dest, &all)?;
    let inbox = session.lock().unwrap().uids_from(INBOX, email)?;
    session.lock().unwrap().copy_ids(INBOX, dest, &inbox)?;
    session.lock().unwrap().move_ids(INBOX, ALL_MAIL, &inbox)?;
    // Blocked mail shouldn't stay in Important either.
    if dest == SCREENED && !all.is_empty() {
        session.lock().unwrap().store_op(ALL_MAIL, &all, "-X-GM-LABELS (\\Important)")?;
    }
    store.lock().unwrap().remove_inbox_from(acct, email)?;
    Ok(all.len().max(inbox.len()))
}

/// A decision about a sender empties their Lobby: back to the Inbox for
/// "keep", else into `dest`. Nothing to do when there's no Lobby.
pub fn release_lobby(session: &SharedSession, email: &str, dest: &str) -> anyhow::Result<usize> {
    let Ok(ids) = session.lock().unwrap().uids_from(LOBBY, email) else { return Ok(0) };
    if ids.is_empty() {
        return Ok(0);
    }
    if dest == KEEP {
        session.lock().unwrap().move_ids(LOBBY, INBOX, &ids)?;
    } else {
        session.lock().unwrap().ensure_mailbox(dest)?;
        session.lock().unwrap().copy_ids(LOBBY, dest, &ids)?;
        session.lock().unwrap().move_ids(LOBBY, ALL_MAIL, &ids)?;
    }
    Ok(ids.len())
}

/// Undo a block: the sender's Screened Out mail goes back to the Inbox.
pub fn unscreen_sender(session: &SharedSession, email: &str) -> anyhow::Result<usize> {
    let ids = session.lock().unwrap().uids_from(SCREENED, email)?;
    session.lock().unwrap().move_ids(SCREENED, INBOX, &ids)?;
    Ok(ids.len())
}

pub struct SweepState {
    pub rows: Vec<SenderStat>,
    pub selected: usize,
    pub scroll: UniformListScrollHandle,
    pub focus: FocusHandle,
    /// Recent subjects of the selected sender.
    pub preview: Vec<(String, i64)>,
    pub loading: bool,
    /// The plugin whose view this is, and what it showed.
    pub plugin: String,
    pub title: String,
    pub keys: Vec<wasm_host::ViewKey>,
}

/// A decision held for the undo window before Gmail is changed.
pub struct PendingSweep {
    /// The account whose rules and mail it changes.
    pub account: String,
    pub email: String,
    pub name: String,
    pub count: u64,
    pub dest: String,
    pub previous: Option<String>,
    /// Rows taken out of the list when the decision was made.
    pub hidden: Vec<Envelope>,
    pub deadline: std::time::Instant,
    pub _task: Task<()>,
}

/// Pending decisions are per account and sender.
pub fn sweep_key(account: &str, email: &str) -> String {
    format!("{account}\u{1f}{email}")
}

fn dest_label(dest: &str) -> &str {
    match dest {
        KEEP => "Keep",
        other => other,
    }
}

/// A rule's colour: the view key with the same label, if any.
fn dest_color(keys: &[wasm_host::ViewKey], dest: &str) -> Rgba {
    keys.iter().find(|k| k.label.eq_ignore_ascii_case(dest_label(dest))).map(|k| rgb(k.color)).unwrap_or(pal().subtle)
}

fn to_wit(r: &SenderStat) -> wasm_host::Sender {
    wasm_host::Sender {
        email: r.email.clone(),
        name: r.name.clone(),
        count: r.count,
        unread: r.unread,
        latest: r.last_ts,
        rule: r.rule.clone(),
    }
}

impl MailApp {
    pub fn open_sweep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            return;
        }
        let Some(host) = self.wasm.clone() else {
            self.status = "Plugins are still loading".into();
            self.status_is_error = false;
            cx.notify();
            return;
        };
        let acct = self.account.to_string();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.sweep = Some(SweepState {
            rows: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::new(),
            focus,
            preview: Vec::new(),
            loading: true,
            plugin: String::new(),
            title: "Sweep".into(),
            keys: Vec::new(),
        });
        cx.notify();
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let (view, problems) = cx.background_executor().spawn(async move { host.lock().unwrap().sender_view(&acct) }).await;
            this.update_in(cx, |this, window, cx| {
                for p in &problems {
                    crate::errlog::record(&this.account, p);
                }
                let Some((plugin, view)) = view else {
                    this.close_sweep(window, cx);
                    this.status = problems
                        .first()
                        .cloned()
                        .unwrap_or_else(|| "The Sweep is a plugin: switch it on in Settings → Plugins".into())
                        .into();
                    this.status_is_error = !problems.is_empty();
                    return;
                };
                this.refresh_rules();
                let rows: Vec<SenderStat> = view
                    .rows
                    .into_iter()
                    .map(|r| SenderStat {
                        rule: this.rules.get(&r.email).cloned().or(r.rule),
                        email: r.email,
                        name: r.name,
                        count: r.count,
                        unread: r.unread,
                        last_ts: r.latest,
                    })
                    .collect();
                if let Some(sw) = &mut this.sweep {
                    sw.rows = rows;
                    sw.loading = false;
                    sw.plugin = plugin;
                    sw.title = view.title;
                    sw.keys = view.keys;
                }
                if let Some(first) = problems.first() {
                    this.status = first.clone().into();
                    this.status_is_error = true;
                }
                this.load_sweep_preview(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn close_sweep(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sweep.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    fn load_sweep_preview(&mut self, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        let (Some(sw), Some(store)) = (&self.sweep, self.store.clone()) else { return };
        let Some(email) = sw.rows.get(sw.selected).map(|r| r.email.clone()) else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let e = email.clone();
            let preview = cx
                .background_executor()
                .spawn(async move { store.lock().unwrap().recent_subjects(&*acct, ALL_MAIL, &e, 10).unwrap_or_default() })
                .await;
            this.update(cx, |this, cx| {
                if let Some(sw) = &mut this.sweep {
                    if sw.rows.get(sw.selected).map(|r| r.email.as_str()) == Some(email.as_str()) {
                        sw.preview = preview;
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn sweep_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(sw) = &mut self.sweep else { return };
        if sw.rows.is_empty() {
            return;
        }
        let max = sw.rows.len() as isize - 1;
        sw.selected = (sw.selected as isize + delta).clamp(0, max) as usize;
        sw.scroll.scroll_to_item(sw.selected, ScrollStrategy::Center);
        self.load_sweep_preview(cx);
        cx.notify();
    }

    /// A key in the Sweep: if the plugin's view has it, ask the plugin
    /// what to do with the selected sender. Returns whether it was a view key.
    pub fn sweep_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let Some(sw) = &mut self.sweep else { return false };
        if !sw.keys.iter().any(|k| k.key.eq_ignore_ascii_case(key)) {
            return false;
        }
        let Some(who) = sw.rows.get(sw.selected).map(to_wit) else { return true };
        let plugin = sw.plugin.clone();
        if sw.selected + 1 < sw.rows.len() {
            sw.selected += 1;
            sw.scroll.scroll_to_item(sw.selected, ScrollStrategy::Center);
        }
        self.load_sweep_preview(cx);
        cx.notify();
        let Some(host) = self.wasm.clone() else { return true };
        let account = self.account.to_string();
        let key = key.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let acct = account.clone();
            let (actions, problems) = cx.background_executor().spawn(async move { host.lock().unwrap().sender_key(&plugin, &acct, &who, &key) }).await;
            this.update(cx, |this, cx| this.sweep_actions(account, actions, problems, cx)).ok();
        })
        .detach();
        true
    }

    /// Carry out what a sender view's plugin answered.
    fn sweep_actions(&mut self, account: String, actions: Vec<wasm_host::Action>, mut problems: Vec<String>, cx: &mut Context<Self>) {
        for a in actions {
            match a {
                wasm_host::Action::FileSender(d) => {
                    let dest = d.destination.unwrap_or_else(|| KEEP.to_string());
                    let email = d.email.to_lowercase();
                    let row = self.sweep.as_mut().and_then(|sw| sw.rows.iter_mut().find(|r| r.email.eq_ignore_ascii_case(&email)));
                    let previous = match row {
                        Some(r) if r.rule.as_deref() == Some(dest.as_str()) => continue,
                        Some(r) => r.rule.replace(dest.clone()),
                        None => self.rule_in(&account, &email),
                    };
                    self.queue_sender(account.clone(), email, d.name, d.count, &dest, previous, Vec::new(), cx);
                }
                wasm_host::Action::Notify(text) => {
                    self.status = text.into();
                    self.status_is_error = false;
                }
                _ => problems.push("A sender view can only file senders and notify".into()),
            }
        }
        for p in &problems {
            crate::errlog::record(&account, p);
        }
        if let Some(first) = problems.first() {
            self.status = first.clone().into();
            self.status_is_error = true;
        }
        cx.notify();
    }

    /// Hold a sender decision for the undo window, then apply it. Shared by
    /// the Sweep and the message list.
    pub fn queue_sender(
        &mut self,
        account: String,
        email: String,
        name: String,
        count: u64,
        dest: &str,
        mut previous: Option<String>,
        mut hidden: Vec<Envelope>,
        cx: &mut Context<Self>,
    ) {
        // A second decision about the same sender replaces the first.
        if let Some(pos) = self.pending_sweep.iter().position(|p| p.email == email && p.account == account) {
            let old = self.pending_sweep.remove(pos);
            previous = old.previous;
            hidden.extend(old.hidden);
        }
        self.status_is_error = false;
        let key = sweep_key(&account, &email);
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            for _ in 0..UNDO_SEND_SECS {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
            this.update(cx, |this, cx| this.commit_sweep(&key, cx)).ok();
        });
        self.pending_sweep.push(PendingSweep {
            account,
            email,
            name,
            count,
            dest: dest.to_string(),
            previous,
            hidden,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(UNDO_SEND_SECS),
            _task: task,
        });
        cx.notify();
    }

    /// `key` = sweep_key(account, sender).
    pub fn commit_sweep(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(pos) = self.pending_sweep.iter().position(|p| sweep_key(&p.account, &p.email) == key) else { return };
        let p = self.pending_sweep.remove(pos);
        let session = self.preview_of(&p.account);
        let (Some(session), Some(store)) = (session, self.store.clone()) else {
            self.revert_sweep_row(&p.email, p.previous.clone(), cx);
            self.restore_items(p.hidden, cx);
            self.status = "Offline: nothing was changed".into();
            self.status_is_error = true;
            return;
        };
        self.set_rule_in(&p.account, &p.email, &p.name, Some(&p.dest));
        let acct = p.account.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (email, dest) = (p.email.clone(), p.dest.clone());
            let unscreen = p.dest == KEEP && p.previous.as_deref() == Some(SCREENED);
            let result = cx
                .background_executor()
                .spawn(async move {
                    let n = if unscreen {
                        unscreen_sender(&session, &email)?
                    } else {
                        apply_sender(&session, &store, &acct, &email, &dest)?
                    };
                    let lobby = release_lobby(&session, &email, &dest)?;
                    anyhow::Ok(n + lobby)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(n) if unscreen => {
                        this.status = format!("{} allowed: {n} messages back in the Inbox", display_name(&p.name, &p.email)).into();
                        this.refresh_folders(cx);
                    }
                    Ok(n) if p.dest == KEEP && n > 0 => {
                        this.status = format!("{} let in: {n} message{} in the Inbox", display_name(&p.name, &p.email), if n == 1 { "" } else { "s" }).into();
                        this.refresh_folders(cx);
                    }
                    Ok(_) if p.dest == KEEP => this.status = format!("{} stays in the Inbox", display_name(&p.name, &p.email)).into(),
                    Ok(n) => {
                        this.status = format!("{} → {}: {n} messages filed", display_name(&p.name, &p.email), p.dest).into();
                        this.refresh_folders(cx);
                        // The Inbox view no longer holds their mail.
                        let (gone, here) = (p.email.clone(), this.account.to_string());
                        this.retain_messages(|e| {
                            let same_account = if e.account.is_empty() { here == p.account } else { e.account.as_ref() == p.account };
                            !(same_account && e.mailbox.eq_ignore_ascii_case(INBOX) && e.from_email.to_lowercase() == gone)
                        });
                    }
                    Err(err) => {
                        this.set_rule_in(&p.account, &p.email, &p.name, p.previous.as_deref());
                        this.revert_sweep_row(&p.email, p.previous.clone(), cx);
                        this.restore_items(p.hidden.clone(), cx);
                        this.status = format!("Could not file {}: {err:#}", p.email).into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn revert_sweep_row(&mut self, email: &str, previous: Option<String>, cx: &mut Context<Self>) {
        if let Some(sw) = &mut self.sweep {
            if let Some(row) = sw.rows.iter_mut().find(|r| r.email == email) {
                row.rule = previous;
            }
        }
        cx.notify();
    }

    /// Undo the newest pending Sweep decision. Returns false if none.
    pub fn undo_sweep(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(p) = self.pending_sweep.pop() else { return false };
        self.revert_sweep_row(&p.email, p.previous.clone(), cx);
        self.restore_items(p.hidden, cx);
        if let Some(sw) = &mut self.sweep {
            if let Some(ix) = sw.rows.iter().position(|r| r.email == p.email) {
                sw.selected = ix;
                sw.scroll.scroll_to_item(ix, ScrollStrategy::Center);
            }
        }
        self.load_sweep_preview(cx);
        self.status = format!("Undid {} → {}", display_name(&p.name, &p.email), dest_label(&p.dest)).into();
        self.status_is_error = false;
        true
    }

    /// Status text while a Sweep decision waits.
    pub fn sweep_status(&self) -> Option<String> {
        let p = self.pending_sweep.last()?;
        let left = p.deadline.saturating_duration_since(std::time::Instant::now()).as_secs() + 1;
        Some(format!(
            "{} → {} ({} messages) · ⌘Z to undo ({}s)",
            display_name(&p.name, &p.email),
            dest_label(&p.dest),
            p.count,
            left.min(UNDO_SEND_SECS)
        ))
    }

    /// New Inbox mail from senders with a rule: hide it now, file it in
    /// Gmail in the background.
    pub fn apply_rules_to_inbox(&mut self, list: Vec<Envelope>, cx: &mut Context<Self>) -> Vec<Envelope> {
        self.refresh_rules();
        if self.rules.is_empty() {
            return list;
        }
        let mut to_file: std::collections::BTreeMap<String, String> = Default::default();
        let kept: Vec<Envelope> = list
            .into_iter()
            .filter(|e| {
                let email = e.from_email.to_lowercase();
                match self.rules.get(&email) {
                    Some(dest) if dest != KEEP && e.mailbox.eq_ignore_ascii_case(INBOX) => {
                        to_file.insert(email, dest.clone());
                        false
                    }
                    _ => true,
                }
            })
            .collect();
        let session = self.preview_session.clone().or_else(|| self.session.clone());
        let acct = self.account.to_string();
        if let (false, Some(session), Some(store)) = (to_file.is_empty(), session, self.store.clone()) {
            cx.background_executor()
                .spawn(async move {
                    for (email, dest) in to_file {
                        apply_sender(&session, &store, &acct, &email, &dest).ok();
                    }
                })
                .detach();
        }
        kept
    }

    pub fn render_sweep(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(sw) = &self.sweep else { return div() };
        let total: u64 = sw.rows.iter().map(|r| r.count).sum();
        let decided = sw.rows.iter().filter(|r| r.rule.is_some()).count();
        let header = div()
            .flex_none()
            .px_5()
            .pt_4()
            .pb_3()
            .border_b_1()
            .border_color(pal().border)
            .flex()
            .items_baseline()
            .justify_between()
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(sw.title.clone()))
            .child(div().text_xs().text_color(pal().muted).child(if sw.loading {
                "Ranking senders…".to_string()
            } else {
                format!("{} senders · {} messages · {decided} decided", sw.rows.len(), total)
            }));
        let columns = div()
            .flex_none()
            .h(px(28.))
            .px_5()
            .flex()
            .items_center()
            .gap_4()
            .text_xs()
            .text_color(pal().muted)
            .border_b_1()
            .border_color(pal().border)
            .child(div().w(px(96.)).flex_none().child("DECISION"))
            .child(div().flex_1().min_w_0().child("SENDER"))
            .child(div().w(px(72.)).flex_none().text_right().child("MESSAGES"))
            .child(div().w(px(80.)).flex_none().text_right().child("UNOPENED"))
            .child(div().w(px(72.)).flex_none().text_right().child("LATEST"));
        let count = sw.rows.len();
        let list = uniform_list(
            "sweep-rows",
            count,
            cx.processor(|this, range: Range<usize>, _window, cx| this.render_sweep_rows(range, cx)),
        )
        .track_scroll(&sw.scroll)
        .size_full();
        let table = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(pal().border)
            .child(header)
            .child(columns)
            .child(div().flex_1().min_h_0().child(list));
        div()
            .track_focus(&sw.focus)
            .key_context("Sweep")
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                if ev.keystroke.modifiers.modified() || ev.is_held {
                    return;
                }
                if this.sweep_key(&ev.keystroke.key, cx) {
                    cx.stop_propagation();
                }
            }))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .bg(pal().list_bg)
            .child(table)
            .child(self.render_sweep_side())
    }

    fn render_sweep_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        let Some(sw) = &self.sweep else { return Vec::new() };
        range
            .filter_map(|ix| sw.rows.get(ix).map(|r| (ix, r.clone())))
            .map(|(ix, r)| {
                let selected = ix == sw.selected;
                let pending = self.pending_sweep.iter().any(|p| p.email == r.email);
                let pct = if r.count == 0 { 0 } else { r.unread * 100 / r.count };
                let badge = match &r.rule {
                    Some(dest) => div()
                        .px_2()
                        .rounded_sm()
                        .text_xs()
                        .text_color(dest_color(&sw.keys, dest))
                        .border_1()
                        .border_color(dest_color(&sw.keys, dest))
                        .when(pending, |d| d.opacity(0.6))
                        .child(dest_label(dest).to_string()),
                    None => div().text_xs().text_color(pal().muted).child("—"),
                };
                div()
                    .id(("sweep", ix))
                    .w_full()
                    .h(px(46.))
                    .px_5()
                    .flex()
                    .items_center()
                    .gap_4()
                    .border_b_1()
                    .border_color(pal().border)
                    .when(selected, |d| d.bg(pal().selected))
                    .when(!selected, |d| d.hover(|d| d.bg(pal().hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(sw) = &mut this.sweep {
                            sw.selected = ix;
                        }
                        this.load_sweep_preview(cx);
                        cx.notify();
                    }))
                    .child(div().w(px(96.)).flex_none().child(badge))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().text_sm().text_color(pal().text).child(display_name(&r.name, &r.email)))
                            .child(div().truncate().text_xs().text_color(pal().muted).child(r.email.clone())),
                    )
                    .child(div().w(px(72.)).flex_none().text_right().text_sm().child(r.count.to_string()))
                    .child(
                        div()
                            .w(px(80.))
                            .flex_none()
                            .text_right()
                            .text_sm()
                            .text_color(if pct >= 80 { pal().error } else if pct >= 40 { pal().warning } else { pal().subtle })
                            .child(format!("{pct}%")),
                    )
                    .child(
                        div()
                            .w(px(72.))
                            .flex_none()
                            .text_right()
                            .text_xs()
                            .text_color(pal().muted)
                            .child(model::display_date(r.last_ts)),
                    )
            })
            .collect()
    }

    fn render_sweep_side(&self) -> impl IntoElement {
        let Some(sw) = &self.sweep else { return div() };
        let row = sw.rows.get(sw.selected);
        div()
            .w(px(320.))
            .flex_none()
            .h_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_4()
            .bg(pal().bg)
            .children(row.map(|r| {
                let pct = if r.count == 0 { 0 } else { r.unread * 100 / r.count };
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(display_name(&r.name, &r.email)))
                    .child(div().text_xs().text_color(pal().muted).child(r.email.clone()))
                    .child(
                        div()
                            .pt_1()
                            .text_xs()
                            .text_color(pal().subtle)
                            .child(format!("{} messages · {pct}% never opened", r.count)),
                    )
            }))
            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(pal().muted).child("RECENT"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .children(sw.preview.iter().map(|(subject, ts)| {
                        div()
                            .flex()
                            .justify_between()
                            .gap_3()
                            .text_xs()
                            .child(div().truncate().text_color(pal().text).child(subject.clone()))
                            .child(div().flex_none().text_color(pal().muted).child(model::display_date(*ts)))
                    })),
            )
            .child(div().flex_1())
            .child(div().text_xs().font_weight(FontWeight::SEMIBOLD).text_color(pal().muted).child("FILE ALL THEIR MAIL"))
            .children(sw.keys.iter().map(|k| {
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .child(hint_chip(k.key.clone(), ""))
                    .child(div().text_color(rgb(k.color)).child(k.label.clone()))
                    .child(div().text_color(pal().muted).child(k.about.clone()))
            }))
    }
}

fn display_name(name: &str, email: &str) -> String {
    if name.trim().is_empty() { email.to_string() } else { name.trim().to_string() }
}
