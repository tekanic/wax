wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

mod check;

use wax::plugin::types::{IssueKind, Permission};

struct Spelling;

impl Guest for Spelling {
    fn manifest() -> Manifest {
        Manifest {
            id: "spelling".into(),
            name: "Spelling and grammar".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadDrafts],
            network_hosts: vec![],
            scan_existing: false,
        }
    }

    fn on_message_received(_m: Envelope, _reason: CheckReason) -> Vec<Action> {
        Vec::new()
    }

    fn commands() -> Vec<Command> {
        Vec::new()
    }

    fn run_command(_id: String, _choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        Ok(Vec::new())
    }

    fn panel() -> Option<Panel> {
        None
    }

    fn get_sender_view(_account: String) -> Option<SenderView> {
        None
    }

    fn check_draft(text: String) -> Vec<DraftIssue> {
        let dict = check::Dictionary::new(check::WORDS);
        check::check(&text, &dict)
            .into_iter()
            .map(|i| DraftIssue {
                start: i.range.start as u32,
                end: i.range.end as u32,
                kind: match i.kind {
                    check::Kind::Spelling => IssueKind::Spelling,
                    check::Kind::Grammar => IssueKind::Grammar,
                },
                message: i.message,
                suggestions: i.suggestions,
            })
            .collect()
    }

    fn on_sender_key(_account: String, _who: Sender, _key: String) -> Result<Vec<Action>, String> {
        Ok(Vec::new())
    }
}

export!(Spelling);
