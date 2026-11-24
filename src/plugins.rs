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
