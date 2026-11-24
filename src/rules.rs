//! Sender rules, one `rules.toml` per account (accounts/<email>/):
//!
//!   [senders]
//!   "news@shop.com" = "Reading"        # Shop News
//!   "sam@client.example" = "keep"     # Sam Lee
//!
//! "keep" = always stays in the Inbox; anything else = the Gmail label new
//! mail from that sender is moved to. The app applies them while it's open.
//! The file can be edited by hand; changes are picked up on the next check
//! for mail or when the Sweep opens.

use super::*;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::time::SystemTime;

pub fn rules_file(account: &str) -> PathBuf {
    crate::accounts::rules_file(account)
}

/// (address → destination, address → sender name).
pub type Loaded = (HashMap<String, String>, HashMap<String, String>);

/// A `"quoted"` string at the start of `s`: (unescaped text, rest).
fn quoted(s: &str) -> Option<(String, &str)> {
    let s = s.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = s.char_indices();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                if let Some((_, n)) = chars.next() {
                    out.push(match n {
                        'n' => '\n',
                        't' => '\t',
                        o => o,
                    });
                }
            }
            '"' => return Some((out, &s[i + 1..])),
            c => out.push(c),
        }
    }
    None
}

pub fn parse(text: &str) -> Loaded {
    let (mut rules, mut names) = (HashMap::new(), HashMap::new());
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let (key, rest) = match quoted(line) {
            Some(k) => k,
            None => match line.split_once('=') {
                Some((k, _)) => (k.trim().to_string(), &line[k.len()..]),
                None => continue,
            },
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else { continue };
        let rest = rest.trim_start();
        let (dest, tail) = match quoted(rest) {
            Some(v) => v,
            None => {
                let end = rest.find('#').unwrap_or(rest.len());
                (rest[..end].trim().to_string(), &rest[end..])
            }
        };
        let email = key.trim().to_lowercase();
        let dest = dest.trim().to_string();
        if !email.contains('@') || dest.is_empty() {
            continue;
        }
        if let Some(name) = tail.trim().strip_prefix('#').map(str::trim).filter(|n| !n.is_empty()) {
            names.insert(email.clone(), name.to_string());
        }
        rules.insert(email, if dest.eq_ignore_ascii_case(sweep::KEEP) { sweep::KEEP.to_string() } else { dest });
    }
    (rules, names)
}

pub fn render(rules: &HashMap<String, String>, names: &HashMap<String, String>) -> String {
    let mut out = String::from(
        "# Sender rules for Wax, applied while the app is open.\n\
         #   \"address\" = \"keep\"      always stays in the Inbox\n\
         #   \"address\" = \"<label>\"   new mail is moved to that Gmail label\n\
         # Safe to edit by hand; delete a line to forget a sender.\n\n[senders]\n",
    );
    // Grouped by destination: kept senders first, then each label A-Z.
    let mut groups: BTreeMap<(bool, String), Vec<&String>> = BTreeMap::new();
    for (email, dest) in rules {
        groups.entry((dest != sweep::KEEP, dest.clone())).or_default().push(email);
    }
    for ((_, dest), mut emails) in groups {
        emails.sort();
        let heading = if dest == sweep::KEEP { "Stay in the Inbox".to_string() } else { dest.clone() };
        out.push_str(&format!("\n# {heading}\n"));
        let width = emails.iter().map(|e| e.len()).max().unwrap_or(0) + 2;
        for email in emails {
            let key = files::quote(email);
            let mut line = format!("{key:<width$} = {}", files::quote(&dest));
            if let Some(name) = names.get(email).map(|n| n.replace(['\n', '\r'], " ")).filter(|n| !n.trim().is_empty()) {
                line = format!("{line:<width2$} # {name}", width2 = width + dest.len() + 6);
            }
            out.push_str(line.trim_end());
            out.push('\n');
        }
    }
    out
}

pub fn load(account: &str) -> Option<Loaded> {
    std::fs::read_to_string(rules_file(account)).ok().map(|t| parse(&t))
}

fn modified(account: &str) -> Option<SystemTime> {
    std::fs::metadata(rules_file(account)).and_then(|m| m.modified()).ok()
}

pub fn save(account: &str, rules: &HashMap<String, String>, names: &HashMap<String, String>) -> anyhow::Result<()> {
    let path = rules_file(account);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, render(rules, names))?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

impl MailApp {
    /// Read the open account's rules.toml (first moving any rules the
    /// cache database still holds for it).
    pub fn load_rules(&mut self) {
        let acct = self.account.to_string();
        self.rules.clear();
        self.rule_names.clear();
        if acct.is_empty() {
            return;
        }
        if !rules_file(&acct).exists() {
            if let Some(store) = &self.store {
                let store = store.lock().unwrap();
                let old = store.rules(&acct).unwrap_or_default();
                if !old.is_empty() && save(&acct, &old, &HashMap::new()).is_ok() {
                    store.clear_rules(&acct).ok();
                }
            }
        }
        if let Some((rules, names)) = load(&acct) {
            self.rules = rules;
            self.rule_names = names;
        }
        self.rules_seen = modified(&acct);
    }

    /// Pick up hand edits to rules.toml.
    pub fn refresh_rules(&mut self) {
        let now = modified(&self.account);
        if now != self.rules_seen {
            if let Some((rules, names)) = load(&self.account) {
                self.rules = rules;
                self.rule_names = names;
            }
            self.rules_seen = now;
        }
    }

    /// Record (Some) or forget (None) a sender's rule and write the file.
    pub fn set_rule(&mut self, email: &str, name: &str, dest: Option<&str>) {
        self.refresh_rules();
        let email = email.to_lowercase();
        match dest {
            Some(d) => {
                self.rules.insert(email.clone(), d.to_string());
                if !name.trim().is_empty() && !name.eq_ignore_ascii_case(&email) {
                    self.rule_names.insert(email, name.trim().to_string());
                }
            }
            None => {
                self.rules.remove(&email);
                self.rule_names.remove(&email);
            }
        }
        let acct = self.account.to_string();
        if let Err(e) = save(&acct, &self.rules, &self.rule_names) {
            self.status = format!("Could not save {}: {e}", rules_file(&acct).display()).into();
            self.status_is_error = true;
        }
        self.rules_seen = modified(&acct);
    }

    /// set_rule for any account: the one on screen through its loaded
    /// rules, others straight in their file.
    pub fn set_rule_in(&mut self, account: &str, email: &str, name: &str, dest: Option<&str>) {
        if account.is_empty() || *account == *self.account {
            return self.set_rule(email, name, dest);
        }
        let (mut rules, mut names) = load(account).unwrap_or_default();
        let email = email.to_lowercase();
        match dest {
            Some(d) => {
                rules.insert(email.clone(), d.to_string());
                if !name.trim().is_empty() && !name.eq_ignore_ascii_case(&email) {
                    names.insert(email, name.trim().to_string());
                }
            }
            None => {
                rules.remove(&email);
                names.remove(&email);
            }
        }
        if let Err(e) = save(account, &rules, &names) {
            self.status = format!("Could not save {}: {e}", rules_file(account).display()).into();
            self.status_is_error = true;
        }
    }

    /// An account's rule for a sender.
    pub fn rule_in(&self, account: &str, email: &str) -> Option<String> {
        if account.is_empty() || *account == *self.account {
            self.rules.get(email).cloned()
        } else {
            load(account).and_then(|(r, _)| r.get(email).cloned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, render};
    use std::collections::HashMap;

    #[test]
    fn round_trip() {
        let mut rules = HashMap::new();
        rules.insert("news@shop.com".to_string(), "Reading".to_string());
        rules.insert("sam@client.example".to_string(), "keep".to_string());
        rules.insert("spam@x.io".to_string(), "Screened Out".to_string());
        let mut names = HashMap::new();
        names.insert("sam@client.example".to_string(), "Sam \"Sammy\" Lee # Acme".to_string());
        let text = render(&rules, &names);
        let (r2, n2) = parse(&text);
        assert_eq!(r2, rules);
        assert_eq!(n2.get("sam@client.example").map(String::as_str), Some("Sam \"Sammy\" Lee # Acme"));
        // Kept senders come first.
        assert!(text.find("sam@client.example").unwrap() < text.find("news@shop.com").unwrap());
    }

    #[test]
    fn hand_edits() {
        let (r, n) = parse("[senders]\n\"A@B.com\" = \"KEEP\"\nbad line\n\"c@d.com\"=\"Reading\"#Cee\n\"noat\" = \"Reading\"\n");
        assert_eq!(r.get("a@b.com").map(String::as_str), Some("keep"));
        assert_eq!(r.get("c@d.com").map(String::as_str), Some("Reading"));
        assert_eq!(n.get("c@d.com").map(String::as_str), Some("Cee"));
        assert_eq!(r.len(), 2);
    }
}
