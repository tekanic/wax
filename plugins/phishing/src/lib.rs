wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

mod check;

use check::{Body, Contact, Mail, THRESHOLD};
use wax::plugin::host::{config, knows_sender, read_message, senders};
use wax::plugin::types::Permission;

struct Phishing;

impl Guest for Phishing {
    fn manifest() -> Manifest {
        Manifest {
            id: "phishing".into(),
            name: "Phishing warnings".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ReadBodies, Permission::ModifyMail],
            network_hosts: vec![],
            scan_existing: true,
        }
    }

    fn on_message_received(m: Envelope, _reason: CheckReason) -> Vec<Action> {
        let Some(from) = m.from.first() else { return Vec::new() };
        if knows_sender(&m) {
            return Vec::new();
        }
        let settings = check::settings(&config());
        let contacts: Vec<Contact> = senders(&m.account, 300)
            .unwrap_or_default()
            .into_iter()
            .map(|s| Contact { name: s.name, email: s.email })
            .collect();
        let name = from.name.clone().unwrap_or_default();
        let me: Vec<String> = std::iter::once(m.account.clone()).chain(m.to.iter().map(|a| a.email.clone())).collect();
        let mail = Mail { name: &name, email: &from.email, subject: &m.subject, me: &me };
        let mut v = check::check(&mail, &contacts, &settings);
        if let Ok(c) = read_message(&m.account, &m.mailbox, &m.id) {
            let links: Vec<(String, String)> = c.links.into_iter().map(|l| (l.text, l.url)).collect();
            let b = Body { text: &c.text, links: &links, headers: &c.headers, attachments: &c.attachments };
            let more = check::check_body(&mail, &b, &contacts, &settings, check::subject_pressure(&m.subject));
            v.score += more.score;
            v.reasons.extend(more.reasons);
        }
        if v.score < THRESHOLD {
            return Vec::new();
        }
        let who = if name.is_empty() { from.email.clone() } else { name };
        let why = v.reasons.join("; ");
        vec![
            Action::AddLabel(settings.label.clone()),
            Action::Warn(format!("Possible phishing: {why}")),
            Action::Notify(format!("Possible phishing from {who}: {why}")),
        ]
    }

    fn commands() -> Vec<Command> {
        Vec::new()
    }

    fn run_command(_id: String, _choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        Err("Phishing warnings has no commands".into())
    }

    fn panel() -> Option<Panel> {
        None
    }

    fn get_sender_view(_account: String) -> Option<SenderView> {
        None
    }

    fn check_draft(_text: String) -> Vec<wax::plugin::types::DraftIssue> {
        Vec::new()
    }

    fn on_sender_key(_account: String, _who: Sender, _key: String) -> Result<Vec<Action>, String> {
        Ok(Vec::new())
    }
}

export!(Phishing);
