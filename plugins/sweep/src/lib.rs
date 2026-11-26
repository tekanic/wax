//! The Sweep: tidy a mailbox by deciding about senders, not messages.
//!
//! Wax ranks senders by volume from its cache; this plugin chooses the keys
//! and where each one files a sender's mail. The core draws the list, holds
//! each decision for its undo window, then writes the sender rule and moves
//! the mail. Keys and labels come from config.toml.

wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

use wax::plugin::host::{config, log, senders};
use wax::plugin::types::{Permission, SenderDecision, ViewKey};

const KEEP: &str = "keep";

struct Sweep;

/// One key from config.toml.
#[derive(Debug, Clone, PartialEq)]
struct Key {
    key: String,
    dest: String,
    about: String,
}

struct Settings {
    keys: Vec<Key>,
    limit: u32,
}

fn unquote(s: &str) -> Option<&str> {
    let s = s.trim();
    s.strip_prefix('"').and_then(|s| s.strip_suffix('"'))
}

fn parse(text: &str) -> Settings {
    let mut keys = Vec::new();
    let mut limit = 500;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (body, about) = match line.find(" #").or_else(|| line.find("\t#")) {
            Some(i) => (&line[..i], line[i..].trim_start().trim_start_matches('#').trim()),
            None => (line, ""),
        };
        let Some((k, v)) = body.split_once('=') else { continue };
        let k = k.trim();
        if k == "limit" {
            limit = v.trim().parse().unwrap_or(limit).clamp(1, 5000);
            continue;
        }
        let Some(dest) = unquote(v) else { continue };
        // A key is one character; later lines replace earlier ones.
        if k.chars().count() != 1 || dest.is_empty() {
            continue;
        }
        let key = k.to_lowercase();
        keys.retain(|x: &Key| x.key != key);
        keys.push(Key { key, dest: dest.to_string(), about: about.to_string() });
    }
    if keys.is_empty() {
        keys = parse(DEFAULTS).keys;
    }
    Settings { keys, limit }
}

const DEFAULTS: &str = "r = \"Reading\" # newsletters\nc = \"Records\" # receipts, notices\nx = \"Screened Out\" # never see them\ni = \"keep\" # stays in the Inbox\n";

fn settings() -> Settings {
    parse(&config())
}

fn label(dest: &str) -> String {
    if dest.eq_ignore_ascii_case(KEEP) { "Keep".into() } else { dest.to_string() }
}

fn color(dest: &str, ix: usize) -> u32 {
    match dest.to_lowercase().as_str() {
        "reading" => 0x4f8ff7,
        "records" => 0xc9a227,
        "screened out" => 0xf87171,
        KEEP => 0x34d058,
        _ => [0xb48ead, 0x56b6c2, 0xe5a050, 0x98c379][ix % 4],
    }
}

impl Guest for Sweep {
    fn manifest() -> Manifest {
        Manifest {
            id: "sweep".into(),
            name: "Sweep".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ModifyMail],
            network_hosts: vec![],
            scan_existing: false,
        }
    }

    fn on_message_received(_message: Envelope, _reason: CheckReason) -> Vec<Action> {
        // New mail from a decided sender is filed by their sender rule.
        Vec::new()
    }

    fn commands() -> Vec<Command> {
        Vec::new()
    }

    fn run_command(_id: String, _choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        Err("Sweep has no commands".into())
    }

    fn panel() -> Option<Panel> {
        None
    }

    fn get_sender_view(account: String) -> Option<SenderView> {
        let s = settings();
        let rows = senders(&account, s.limit).unwrap_or_else(|e| {
            log(&format!("couldn't rank senders: {e}"));
            Vec::new()
        });
        let keys = s
            .keys
            .iter()
            .enumerate()
            .map(|(ix, k)| ViewKey { key: k.key.clone(), label: label(&k.dest), about: k.about.clone(), color: color(&k.dest, ix) })
            .collect();
        Some(SenderView { title: "Sweep".into(), keys, rows })
    }

    fn check_draft(_text: String) -> Vec<wax::plugin::types::DraftIssue> {
        Vec::new()
    }

    fn on_sender_key(_account: String, who: Sender, key: String) -> Result<Vec<Action>, String> {
        let s = settings();
        let Some(k) = s.keys.iter().find(|k| k.key.eq_ignore_ascii_case(&key)) else { return Ok(Vec::new()) };
        // Already decided this way: nothing to do.
        if who.rule.as_deref().is_some_and(|r| r.eq_ignore_ascii_case(&k.dest)) {
            return Ok(Vec::new());
        }
        let destination = (!k.dest.eq_ignore_ascii_case(KEEP)).then(|| k.dest.clone());
        Ok(vec![Action::FileSender(SenderDecision { email: who.email, name: who.name, count: who.count, destination })])
    }
}

export!(Sweep);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_keys_and_comments() {
        let s = parse("# hi\nr = \"Reading\"   # newsletters\nn = \"News\"\nlimit = 20\nbad line\nlong = \"x\"\n");
        assert_eq!(s.limit, 20);
        assert_eq!(s.keys, vec![
            Key { key: "r".into(), dest: "Reading".into(), about: "newsletters".into() },
            Key { key: "n".into(), dest: "News".into(), about: "".into() },
        ]);
    }

    #[test]
    fn defaults_when_empty() {
        let keys: Vec<String> = parse("").keys.into_iter().map(|k| k.key).collect();
        assert_eq!(keys, ["r", "c", "x", "i"]);
    }
}
