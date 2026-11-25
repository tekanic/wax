wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

mod check;

use check::{Contact, Mail, THRESHOLD};
use wax::plugin::host::{config, knows_sender, senders};
use wax::plugin::types::Permission;

struct Phishing;

impl Guest for Phishing {
    fn manifest() -> Manifest {
        Manifest {
            id: "phishing".into(),
            name: "Phishing warnings".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ModifyMail],
            network_hosts: vec![],
        }
    }

    fn on_message_received(m: Envelope) -> Vec<Action> {
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
        let v = check::check(&Mail { name: &name, email: &from.email, subject: &m.subject }, &contacts, &settings);
        if v.score < THRESHOLD {
            return Vec::new();
        }
        let who = if name.is_empty() { from.email.clone() } else { name };
        vec![
            Action::AddLabel(settings.label.clone()),
            Action::Notify(format!("Possible phishing from {who}: {}", v.reasons.join("; "))),
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

    fn on_sender_key(_account: String, _who: Sender, _key: String) -> Result<Vec<Action>, String> {
        Ok(Vec::new())
    }
}

export!(Phishing);
