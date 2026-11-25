//! Filter rules: if-this-then-that for new mail, one rule per line in the
//! plugin's config.toml (~/.config/wax/plugins/filter-rules/config.toml):
//!
//!     subject contains "invoice"   => label "Receipts"
//!     from ends-with "@github.com" => move "Dev"
//!     from is "news@shop.com"      => archive
//!     subject starts-with "[ci]"   => read
//!
//! Fields: from, to, subject. Tests: contains, is, starts-with, ends-with
//! (all ignore case). Actions: label "<name>", move "<name>", archive,
//! read, star. Every rule that matches applies, in order.

wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

use wax::plugin::host::{config, log};
use wax::plugin::types::Permission;

struct FilterRules;

#[derive(Debug, PartialEq)]
struct Rule {
    field: String,
    test: String,
    text: String,
    act: Act,
}

#[derive(Debug, PartialEq, Clone)]
enum Act {
    Label(String),
    Move(String),
    Archive,
    Read,
    Star,
}

/// `word "quoted text"` → (word, quoted text, rest).
fn quoted(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    let s = s.strip_prefix('"')?;
    let end = s.find('"')?;
    Some((s[..end].to_string(), &s[end + 1..]))
}

fn parse_line(line: &str) -> Result<Option<Rule>, String> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let (when, then) = line.split_once("=>").ok_or("missing =>")?;
    let mut words = when.trim().splitn(3, char::is_whitespace);
    let field = words.next().unwrap_or("").to_lowercase();
    let test = words.next().unwrap_or("").to_lowercase();
    if !["from", "to", "subject"].contains(&field.as_str()) {
        return Err(format!("unknown field \"{field}\" (from, to, subject)"));
    }
    if !["contains", "is", "starts-with", "ends-with"].contains(&test.as_str()) {
        return Err(format!("unknown test \"{test}\" (contains, is, starts-with, ends-with)"));
    }
    let (text, _) = quoted(words.next().unwrap_or("")).ok_or("put the text in quotes")?;
    let then = then.trim();
    let (verb, rest) = then.split_once(char::is_whitespace).unwrap_or((then, ""));
    let act = match verb.to_lowercase().as_str() {
        "label" => Act::Label(quoted(rest).ok_or("label needs a \"name\"")?.0),
        "move" => Act::Move(quoted(rest).ok_or("move needs a \"name\"")?.0),
        "archive" => Act::Archive,
        "read" => Act::Read,
        "star" => Act::Star,
        other => return Err(format!("unknown action \"{other}\" (label, move, archive, read, star)")),
    };
    Ok(Some(Rule { field, test, text: text.to_lowercase(), act }))
}

fn matches(rule: &Rule, values: &[String]) -> bool {
    values.iter().any(|v| {
        let v = v.to_lowercase();
        match rule.test.as_str() {
            "contains" => v.contains(&rule.text),
            "is" => v == rule.text,
            "starts-with" => v.starts_with(&rule.text),
            "ends-with" => v.ends_with(&rule.text),
            _ => false,
        }
    })
}

fn decide(rules_text: &str, from: &[String], to: &[String], subject: &str) -> (Vec<Act>, Vec<String>) {
    let mut acts = Vec::new();
    let mut problems = Vec::new();
    for (n, line) in rules_text.lines().enumerate() {
        match parse_line(line) {
            Ok(Some(rule)) => {
                let values: Vec<String> = match rule.field.as_str() {
                    "from" => from.to_vec(),
                    "to" => to.to_vec(),
                    _ => vec![subject.to_string()],
                };
                if matches(&rule, &values) && !acts.contains(&rule.act) {
                    acts.push(rule.act.clone());
                }
            }
            Ok(None) => {}
            Err(e) => problems.push(format!("config.toml line {}: {e}", n + 1)),
        }
    }
    (acts, problems)
}

impl Guest for FilterRules {
    fn manifest() -> Manifest {
        Manifest {
            id: "filter-rules".into(),
            name: "Filter rules".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ModifyMail],
            network_hosts: vec![],
        }
    }

    fn on_message_received(message: Envelope) -> Vec<Action> {
        let addr = |a: &wax::plugin::types::Address| {
            let mut v = vec![a.email.clone()];
            if let Some(n) = &a.name {
                v.push(n.clone());
            }
            v
        };
        let from: Vec<String> = message.from.iter().flat_map(addr).collect();
        let to: Vec<String> = message.to.iter().flat_map(addr).collect();
        let (acts, problems) = decide(&config(), &from, &to, &message.subject);
        for p in problems {
            log(&p);
        }
        acts.into_iter()
            .map(|a| match a {
                Act::Label(l) => Action::AddLabel(l),
                Act::Move(l) => Action::MoveTo(l),
                Act::Archive => Action::Archive,
                Act::Read => Action::MarkRead(true),
                Act::Star => Action::Star(true),
            })
            .collect()
    }

    fn commands() -> Vec<Command> {
        Vec::new()
    }

    fn run_command(_id: String, _choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        Err("Filter rules has no commands".into())
    }

    fn panel() -> Option<Panel> {
        None
    }

    fn get_sender_view(_account: String) -> Option<SenderView> {
        None
    }

    fn on_sender_key(_account: String, _who: Sender, _key: String) -> Result<Vec<Action>, String> {
        Ok(Vec::new())
    }
}

export!(FilterRules);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules() {
        let text = "# receipts\nsubject contains \"Invoice\" => label \"Receipts\"\nfrom ends-with \"@github.com\" => move \"Dev\"\nfrom is \"x\" => explode\n";
        let (acts, problems) = decide(text, &["noreply@GitHub.com".into()], &[], "Your invoice #3");
        assert_eq!(acts, vec![Act::Label("Receipts".into()), Act::Move("Dev".into())]);
        assert_eq!(problems.len(), 1);
    }
}
