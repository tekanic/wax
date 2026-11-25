//! The one path new mail takes before you see it.
//!
//! Every message that arrives becomes an `Arrival`. The core works out the
//! facts plugins may rely on (the sender's rule, whether you know them) and
//! asks each plugin in turn; the first to answer decides. Plugins never
//! touch Gmail: they return an `Act`, and `dispatch_arrivals` carries it out
//! the same way for all of them. These built-in plugins are written
//! against the same shape the sandboxed ones will get (wit/plugin.wit), so
//! moving them out later changes where they run, not what they say.

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
    /// You have mail from them already, have written to them, they share
    /// your work domain, or it's one of your own addresses.
    pub known: bool,
    /// The Screener is on for this account, since this time.
    pub screener: bool,
    pub screener_since: i64,
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

/// Screener: a first email from someone new waits in the Lobby until you
/// decide (e lets them in, b screens them out).
pub struct Screener;

/// Mail older than this when it arrives is never held: it isn't "new".
const SCREEN_RECENT_SECS: i64 = 3 * 24 * 3600;

impl Plugin for Screener {
    fn name(&self) -> &'static str {
        "screener"
    }
    fn arrived(&self, msg: &Arrival, facts: &Facts) -> Option<Act> {
        let fresh = msg.ts >= facts.screener_since && msg.ts >= chrono::Utc::now().timestamp() - SCREEN_RECENT_SECS;
        let hold = facts.screener
            && fresh
            && facts.rule.is_none()
            && !facts.known
            && msg.mailbox.eq_ignore_ascii_case(mail::INBOX);
        hold.then(|| Act::File(sweep::LOBBY.to_string()))
    }
}

pub fn builtins() -> Vec<Box<dyn Plugin>> {
    vec![Box::new(SenderRules), Box::new(Screener)]
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
        let mine: Vec<String> = std::iter::once(account.to_string()).chain(self.identities_for(account).into_iter().map(|i| i.email)).collect();
        let plugins = builtins();
        let screener = self.accounts.iter().any(|a| a.email.eq_ignore_ascii_case(account) && a.screener);
        let screener_since = self.settings.screener_since;
        let mut to_file: BTreeMap<String, String> = BTreeMap::new();
        let mut held = 0;
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
                let known = same_side(&email, &mine)
                    || self.store.as_ref().is_some_and(|s| s.lock().unwrap().sender_known(account, &email, &e.mailbox, &e.id, e.message_id.as_deref()));
                let facts = Facts { rule: rules.get(&email).map(String::as_str), known, screener, screener_since };
                let msg = Arrival { account, email: &email, mailbox: &e.mailbox, ts: e.ts };
                match plugins.iter().find_map(|p| p.arrived(&msg, &facts)) {
                    Some(Act::File(dest)) => {
                        if dest == sweep::LOBBY {
                            held += 1;
                        }
                        to_file.insert(email, dest);
                        false
                    }
                    None => true,
                }
            })
            .collect();
        self.run_wasm_plugins(account, &kept, cx);
        if to_file.is_empty() {
            return kept;
        }
        if held > 0 && active {
            self.status = format!("{held} message{} from someone new waiting in the Lobby", if held == 1 { "" } else { "s" }).into();
            self.status_is_error = false;
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
                        let done = if dest == sweep::LOBBY {
                            sweep::hold_in_lobby(&session, &store, &acct, &email)
                        } else {
                            sweep::apply_sender(&session, &store, &acct, &email, &dest)
                        };
                        if let Err(e) = done {
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

impl MailApp {
    /// New Inbox mail that stayed after the built-ins goes to the
    /// WebAssembly plugins, in the background.
    fn run_wasm_plugins(&mut self, account: &str, kept: &[Envelope], cx: &mut Context<Self>) {
        let Some(host) = self.wasm.clone() else { return };
        if !host.lock().unwrap().plugins.iter().any(|p| p.running()) {
            return;
        }
        let msgs: Vec<Envelope> = kept.iter().filter(|e| e.mailbox.eq_ignore_ascii_case(mail::INBOX)).cloned().collect();
        if msgs.is_empty() {
            return;
        }
        let active = *self.account == *account;
        let session = if active { self.preview_session.clone().or_else(|| self.session.clone()) } else { None };
        let acct = account.to_string();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (changed, notes, problems) = cx
                .background_executor()
                .spawn(async move {
                    let mut problems = Vec::new();
                    let mut work = Vec::new();
                    for e in &msgs {
                        let (acts, mut p) = host.lock().unwrap().arrived(&to_wit(&acct, e));
                        problems.append(&mut p);
                        for (_, a) in acts {
                            work.push((e.mailbox.clone(), e.id.clone(), a));
                        }
                    }
                    let mut notes = Vec::new();
                    let mut changed = false;
                    if work.is_empty() {
                        return (changed, notes, problems);
                    }
                    let session = match session {
                        Some(s) => s,
                        None => match Session::connect(&acct) {
                            Ok(s) => Arc::new(Mutex::new(s)),
                            Err(e) => {
                                problems.push(format!("Plugins: could not connect to {acct}: {e:#}"));
                                return (changed, notes, problems);
                            }
                        },
                    };
                    for (mailbox, id, actions) in work {
                        match apply_actions(&session, &mailbox, &id, &actions) {
                            Ok((moved, mut n)) => {
                                changed |= moved || !actions.is_empty();
                                notes.append(&mut n);
                            }
                            Err(e) => problems.push(format!("Plugins: Gmail refused a change: {e:#}")),
                        }
                    }
                    (changed, notes, problems)
                })
                .await;
            this.update(cx, |this, cx| {
                for p in &problems {
                    crate::errlog::record(&this.account, p);
                }
                if let Some(first) = problems.first() {
                    this.status = first.clone().into();
                    this.status_is_error = true;
                } else if let Some(n) = notes.first() {
                    this.status = n.clone().into();
                    this.status_is_error = false;
                }
                if changed && active {
                    this.refresh_open_folder(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{same_side, Act, Arrival, Facts, Plugin, Screener, SenderRules};

    fn facts(rule: Option<&str>, known: bool, on: bool, since: i64) -> Facts<'_> {
        Facts { rule, known, screener: on, screener_since: since }
    }

    #[test]
    fn screener_holds_only_new_strangers() {
        let now = chrono::Utc::now().timestamp();
        let since = now - 60;
        let msg = Arrival { account: "a@x.com", email: "new@shop.com", mailbox: "INBOX", ts: now };
        assert_eq!(Screener.arrived(&msg, &facts(None, false, true, since)), Some(Act::File("Lobby".into())));
        assert_eq!(Screener.arrived(&msg, &facts(None, true, true, since)), None, "known sender");
        assert_eq!(Screener.arrived(&msg, &facts(Some("keep"), false, true, since)), None, "decided already");
        let old = Arrival { ts: now - 3600, ..msg };
        assert_eq!(Screener.arrived(&old, &facts(None, false, true, since)), None, "arrived before it was switched on");
        let msg = Arrival { ts: now, ..old };
        assert_eq!(Screener.arrived(&msg, &facts(None, false, false, since)), None, "off for this account");
    }

    #[test]
    fn rules_file_inbox_mail() {
        let msg = Arrival { account: "a@x.com", email: "n@shop.com", mailbox: "INBOX", ts: 0 };
        assert_eq!(SenderRules.arrived(&msg, &facts(Some("Reading"), true, false, 0)), Some(Act::File("Reading".into())));
        assert_eq!(SenderRules.arrived(&msg, &facts(Some("keep"), true, false, 0)), None);
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
