//! Misbehaves on purpose: subject "loop" spins forever, "grab" tries to
//! allocate far past the memory limit. Used by the host's tests only.

wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

use wax::plugin::types::Permission;

struct Runaway;

impl Guest for Runaway {
    fn manifest() -> Manifest {
        Manifest {
            id: "test-runaway".into(),
            name: "Runaway".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes],
            network_hosts: vec![],
            scan_existing: false,
        }
    }

    fn on_message_received(message: Envelope, _reason: CheckReason) -> Vec<Action> {
        if message.subject == "loop" {
            let mut n: u64 = 0;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        }
        if message.subject == "grab" {
            let mut v: Vec<Vec<u8>> = Vec::new();
            loop {
                v.push(vec![1u8; 8 << 20]);
                std::hint::black_box(&v);
            }
        }
        vec![Action::Archive]
    }

    fn commands() -> Vec<Command> {
        Vec::new()
    }

    fn run_command(_id: String, _choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        Err("no commands".into())
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

export!(Runaway);
