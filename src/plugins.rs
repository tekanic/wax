//! The one path new mail takes before you see it.
//!
//! 1. Sender rules (built in) file mail from senders you've decided about.
//! 2. The WebAssembly plugins (the Screener, Filter rules…) see each new
//!    Inbox message that's left, in the background. Whatever they move or
//!    archive never appears in the Inbox or as a notification.
//! 3. What's left is shown and announced (live.rs `show_arrivals`), and the
//!    plugins' actions are carried out over IMAP.
//!
//! Plugins never touch Gmail themselves: they return actions and the core
//! carries them out.

use super::*;
use std::collections::{BTreeMap, HashMap};

/// A message that just arrived, as plugins see it. (Every field is part of
/// the plugin interface, used or not by today's built-ins.)
#[allow(dead_code)]
pub struct Arrival<'a> {
    pub account: &'a str,
    pub email: &'a str,
    pub mailbox: &'a str,
    pub ts: i64,
}

/// What the core knows about the sender.
pub struct Facts<'a> {
    /// The sender's rule: "keep", "Reading", "Screened Out"…
    pub rule: Option<&'a str>,
}

/// What a plugin wants done with the message.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Take the sender's new mail out of the Inbox and label it `dest`.
    File(String),
}

pub trait Plugin {
    /// For logs and, later, the plugin list in Settings.
    #[allow(dead_code)]
    fn name(&self) -> &'static str;
    fn arrived(&self, msg: &Arrival, facts: &Facts) -> Option<Act>;
}

/// Sender rules (rules.toml): file new Inbox mail where its sender belongs.
/// Built in because the Sweep, the Screener's Lobby and block/allow in the
/// message list all write these rules.
pub struct SenderRules;

impl Plugin for SenderRules {
    fn name(&self) -> &'static str {
        "sender-rules"
    }
    fn arrived(&self, msg: &Arrival, facts: &Facts) -> Option<Act> {
        let rule = facts.rule?;
        (rule != sweep::KEEP && msg.mailbox.eq_ignore_ascii_case(mail::INBOX)).then(|| Act::File(rule.to_string()))
    }
}

pub fn builtins() -> Vec<Box<dyn Plugin>> {
    vec![Box::new(SenderRules)]
}

/// The part of "do you know them" that needs no database: one of your own
/// addresses, or the same domain as the account (a colleague). Shared mail
/// domains (gmail.com and friends) don't count.
pub fn same_side(sender: &str, mine: &[String]) -> bool {
    const PUBLIC: [&str; 8] =
        ["gmail.com", "googlemail.com", "outlook.com", "hotmail.com", "yahoo.com", "icloud.com", "me.com", "aol.com"];
    let domain = |e: &str| e.rsplit_once('@').map(|(_, d)| d.to_lowercase()).unwrap_or_default();
    let sender = sender.to_lowercase();
    mine.iter().any(|m| {
        let m = m.to_lowercase();
        m == sender || (domain(&m) == domain(&sender) && !domain(&m).is_empty() && !PUBLIC.contains(&domain(&m).as_str()))
    })
}

impl MailApp {
    /// New mail for `account`: ask the plugins, carry out what they say,
    /// and return the messages that stay where they are.
    pub fn dispatch_arrivals(&mut self, account: &str, list: Vec<Envelope>, cx: &mut Context<Self>) -> Vec<Envelope> {
        let active = *self.account == *account;
        if active {
            self.refresh_rules();
        }
        let rules: HashMap<String, String> = if active { self.rules.clone() } else { rules::load(account).map(|(r, _)| r).unwrap_or_default() };
        let plugins = builtins();
        let mut to_file: BTreeMap<String, String> = BTreeMap::new();
        let kept: Vec<Envelope> = list
            .into_iter()
            .filter(|e| {
                let email = e.from_email.to_lowercase();
                if email.is_empty() {
                    return true;
                }
                if let Some(dest) = to_file.get(&email) {
                    // Same sender twice in one batch: same answer.
                    return !e.mailbox.eq_ignore_ascii_case(mail::INBOX) || dest.is_empty();
                }
                let facts = Facts { rule: rules.get(&email).map(String::as_str) };
                let msg = Arrival { account, email: &email, mailbox: &e.mailbox, ts: e.ts };
                match plugins.iter().find_map(|p| p.arrived(&msg, &facts)) {
                    Some(Act::File(dest)) => {
                        to_file.insert(email, dest);
                        false
                    }
                    None => true,
                }
            })
            .collect();
        if to_file.is_empty() {
            return kept;
        }
        let session = if active { self.preview_session.clone().or_else(|| self.session.clone()) } else { None };
        let (store, acct) = (self.store.clone(), account.to_string());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let errors = cx
                .background_executor()
                .spawn(async move {
                    let Some(store) = store else { return vec!["no cache".to_string()] };
                    let session = match session {
                        Some(s) => s,
                        None => match Session::connect(&acct) {
                            Ok(s) => Arc::new(Mutex::new(s)),
                            Err(e) => return vec![format!("{e:#}")],
                        },
                    };
                    let mut errors = Vec::new();
                    for (email, dest) in to_file {
                        if let Err(e) = sweep::apply_sender(&session, &store, &acct, &email, &dest) {
                            errors.push(format!("Could not file new mail from {email}: {e:#}"));
                        }
                    }
                    errors
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(first) = errors.first() {
                    this.status = first.clone().into();
                    this.status_is_error = true;
                }
                this.refresh_folders(cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
        kept
    }
}

/// The plugin interface's view of a message.
fn to_wit(account: &str, e: &Envelope) -> wasm_host::Envelope {
    let mut flags = Vec::new();
    if !e.unread {
        flags.push("\\Seen".to_string());
    }
    if e.starred {
        flags.push("\\Flagged".to_string());
    }
    let name = (!e.from.is_empty() && e.from != e.from_email).then(|| e.from.to_string());
    wasm_host::Envelope {
        account: account.to_string(),
        mailbox: e.mailbox.clone(),
        id: e.id.clone(),
        message_id: e.message_id.clone(),
        from: vec![wasm_host::Address { name, email: e.from_email.to_string() }],
        to: Vec::new(),
        subject: e.subject.to_string(),
        date: chrono::DateTime::from_timestamp(e.ts, 0).map(|d| d.to_rfc3339()),
        flags,
        labels: Vec::new(),
    }
}

/// Carry out one plugin's actions on one message. Labels and flags go
/// first; moving or archiving last, since that takes it out of `mailbox`.
fn apply_actions(session: &SharedSession, mailbox: &str, id: &str, actions: &[wasm_host::Action]) -> anyhow::Result<(bool, Vec<String>)> {
    use wasm_host::Action as A;
    let ids = [id.to_string()];
    let mut notes = Vec::new();
    let mut moved = false;
    let rank = |a: &A| matches!(a, A::MoveTo(_) | A::Archive) as u8;
    let mut ordered: Vec<&A> = actions.iter().collect();
    ordered.sort_by_key(|a| rank(a));
    for a in ordered {
        let mut s = session.lock().unwrap();
        match a {
            A::AddLabel(l) => {
                s.ensure_mailbox(l)?;
                s.copy_ids(mailbox, l, &ids)?;
            }
            A::RemoveLabel(l) => s.store_op(mailbox, &ids, &format!("-X-GM-LABELS ({})", mail::imap_quote(l)))?,
            A::MarkRead(on) => s.store_op(mailbox, &ids, if *on { "+FLAGS.SILENT (\\Seen)" } else { "-FLAGS.SILENT (\\Seen)" })?,
            A::Star(on) => s.store_op(mailbox, &ids, if *on { "+FLAGS.SILENT (\\Flagged)" } else { "-FLAGS.SILENT (\\Flagged)" })?,
            A::MoveTo(l) if !moved => {
                s.ensure_mailbox(l)?;
                s.copy_ids(mailbox, l, &ids)?;
                s.move_ids(mailbox, ALL_MAIL, &ids)?;
                moved = true;
            }
            A::Archive if !moved => {
                s.move_ids(mailbox, ALL_MAIL, &ids)?;
                moved = true;
            }
            A::MoveTo(_) | A::Archive => {}
            A::Notify(text) => notes.push(text.clone()),
            A::SnoozeUntil(_) => notes.push("snooze isn't available to plugins yet".into()),
            A::OpenDraft(_) => notes.push("opening drafts isn't available to plugins yet".into()),
            A::FileSender(_) => notes.push("file-sender works only from a sender view".into()),
        }
    }
    Ok((moved, notes))
}

/// Whether a plugin's actions take the message out of the Inbox.
fn leaves_inbox(actions: &[wasm_host::Action]) -> bool {
    actions.iter().any(|a| matches!(a, wasm_host::Action::MoveTo(_) | wasm_host::Action::Archive))
}

impl MailApp {
    /// New mail left after sender rules goes to the WebAssembly plugins
    /// first; then what they didn't move is shown and announced, and their
    /// actions are carried out. Without plugins it's shown at once.
    pub fn plugin_arrivals(&mut self, account: &str, kept: Vec<Envelope>, cx: &mut Context<Self>) {
        let inbox: Vec<Envelope> = kept.iter().filter(|e| e.mailbox.eq_ignore_ascii_case(mail::INBOX)).cloned().collect();
        let loading = self.wasm.is_none();
        let none_running = self.wasm.as_ref().is_some_and(|h| !h.lock().unwrap().plugins.iter().any(|p| p.running()));
        if inbox.is_empty() || none_running {
            return self.show_arrivals(account, kept, cx);
        }
        let mine: Vec<String> = std::iter::once(account.to_string()).chain(self.identities_for(account).into_iter().map(|i| i.email)).collect();
        let arrival = wasm_host::Arrival {
            mine,
            batch: kept.iter().map(|e| (e.mailbox.clone(), e.id.clone())).collect(),
            batch_mids: kept.iter().filter_map(|e| e.message_id.clone()).collect(),
        };
        let active = *self.account == *account;
        let session = if active { self.preview_session.clone().or_else(|| self.session.clone()) } else { None };
        let (store, acct) = (self.store.clone(), account.to_string());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            // Mail that arrives while the plugins are still loading waits
            // for them (a few seconds at most).
            let mut host = None;
            for _ in 0..(if loading { 50 } else { 1 }) {
                host = this.update(cx, |this, _| this.wasm.clone()).ok().flatten();
                if host.is_some() {
                    break;
                }
                cx.background_executor().timer(std::time::Duration::from_millis(200)).await;
            }
            let Some(host) = host else {
                this.update(cx, |this, cx| this.show_arrivals(&acct, kept, cx)).ok();
                return;
            };
            // Decide, and take what leaves the Inbox out of the cache now so
            // a refresh can't show it before Gmail has moved it.
            let a = acct.clone();
            let st = store.clone();
            let (work, mut problems) = cx
                .background_executor()
                .spawn(async move {
                    let mut problems = Vec::new();
                    let mut work = Vec::new();
                    let mut h = host.lock().unwrap();
                    h.arrival = arrival;
                    for e in &inbox {
                        let (acts, mut p) = h.arrived(&to_wit(&a, e));
                        problems.append(&mut p);
                        let actions: Vec<wasm_host::Action> = acts.into_iter().flat_map(|(_, a)| a).collect();
                        if !actions.is_empty() {
                            work.push((e.mailbox.clone(), e.id.clone(), actions));
                        }
                    }
                    h.arrival = Default::default();
                    drop(h);
                    if let Some(store) = &st {
                        for (mailbox, id, actions) in &work {
                            if leaves_inbox(actions) {
                                store.lock().unwrap().remove_envelope(&a, mailbox, id).ok();
                            }
                        }
                    }
                    (work, problems)
                })
                .await;
            let gone: HashSet<(String, String)> =
                work.iter().filter(|(_, _, a)| leaves_inbox(a)).map(|(m, i, _)| (m.clone(), i.clone())).collect();
            let shown: Vec<Envelope> = kept.into_iter().filter(|e| !gone.contains(&(e.mailbox.clone(), e.id.clone()))).collect();
            let notes: Vec<String> = work
                .iter()
                .flat_map(|(_, _, acts)| acts.iter().filter_map(|a| if let wasm_host::Action::Notify(t) = a { Some(t.clone()) } else { None }))
                .collect();
            this.update(cx, |this, cx| {
                this.show_arrivals(&acct, shown, cx);
                if let Some(first) = notes.first() {
                    this.status = if notes.len() > 1 { format!("{first} (and {} more)", notes.len() - 1) } else { first.clone() }.into();
                    this.status_is_error = false;
                }
            })
            .ok();
            if work.is_empty() {
                this.update(cx, |this, cx| this.report_plugin_problems(&problems, cx)).ok();
                return;
            }
            // Carry out the actions.
            let a = acct.clone();
            let mut more = cx
                .background_executor()
                .spawn(async move {
                    let mut problems = Vec::new();
                    let session = match session {
                        Some(s) => s,
                        None => match Session::connect(&a) {
                            Ok(s) => Arc::new(Mutex::new(s)),
                            Err(e) => return vec![format!("Plugins: could not connect to {a}: {e:#}")],
                        },
                    };
                    for (mailbox, id, actions) in work {
                        if let Err(e) = apply_actions(&session, &mailbox, &id, &actions) {
                            problems.push(format!("Plugins: Gmail refused a change: {e:#}"));
                        }
                    }
                    problems
                })
                .await;
            problems.append(&mut more);
            this.update(cx, |this, cx| {
                this.report_plugin_problems(&problems, cx);
                if active {
                    this.refresh_folders(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn report_plugin_problems(&mut self, problems: &[String], cx: &mut Context<Self>) {
        for p in problems {
            crate::errlog::record(&self.account, p);
        }
        if let Some(first) = problems.first() {
            self.status = first.clone().into();
            self.status_is_error = true;
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{same_side, Act, Arrival, Facts, Plugin, SenderRules};

    #[test]
    fn rules_file_inbox_mail() {
        let msg = Arrival { account: "a@x.com", email: "n@shop.com", mailbox: "INBOX", ts: 0 };
        assert_eq!(SenderRules.arrived(&msg, &Facts { rule: Some("Reading") }), Some(Act::File("Reading".into())));
        assert_eq!(SenderRules.arrived(&msg, &Facts { rule: Some("keep") }), None);
        let sent = Arrival { mailbox: "[Gmail]/Sent Mail", ..msg };
        assert_eq!(SenderRules.arrived(&sent, &Facts { rule: Some("Reading") }), None);
    }

    #[test]
    fn colleagues_and_own_addresses_are_known() {
        let mine = vec!["me@acme.com".to_string(), "me@gmail.com".to_string()];
        assert!(same_side("boss@ACME.com", &mine));
        assert!(same_side("me@gmail.com", &mine));
        assert!(!same_side("stranger@gmail.com", &mine), "gmail.com is not a work domain");
        assert!(!same_side("x@other.com", &mine));
    }
}
