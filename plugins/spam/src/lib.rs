wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

mod model;

use model::{tokens, Message, Model};
use wax::plugin::host::{config, knows_sender, read_message, storage_get, storage_set};
use wax::plugin::types::Permission;

const SPAM: &str = "[Gmail]/Spam";
const SCREENED: &str = "Screened Out";
const EXISTING_SPAM: u32 = 300;
const EXISTING_GOOD: u32 = 400;
const SETTLED_SECS: i64 = 2 * 86_400;

struct Spam;

fn load() -> Model {
    Model::parse(&storage_get("model").unwrap_or_default())
}

fn host_of(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let host = rest.split(['/', '?', '#', ':']).next()?;
    (!host.is_empty()).then(|| host.trim_start_matches("www.").to_string())
}

fn read(m: &Envelope) -> (String, Vec<String>, Vec<String>) {
    match read_message(&m.account, &m.mailbox, &m.id) {
        Ok(c) => {
            let mut hosts: Vec<String> = c.links.iter().filter_map(|l| host_of(&l.url)).collect();
            hosts.sort();
            hosts.dedup();
            (c.text, hosts, c.attachments)
        }
        Err(_) => (String::new(), Vec::new(), Vec::new()),
    }
}

fn of(m: &Envelope) -> std::collections::BTreeSet<String> {
    let (text, hosts, attachments) = read(m);
    let from = m.from.first();
    tokens(&Message {
        from_email: from.map(|a| a.email.as_str()).unwrap_or(""),
        from_name: from.and_then(|a| a.name.as_deref()).unwrap_or(""),
        subject: &m.subject,
        text: &text,
        hosts,
        attachments: &attachments,
    })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    era * 146097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719468
}

fn age(date: Option<&str>) -> Option<i64> {
    let s = date?;
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let t = days(n(0..4)?, n(5..7)?, n(8..10)?) * 86_400 + n(11..13)? * 3600 + n(14..16)? * 60;
    Some(now() - t)
}

fn learn_from(m: &Envelope, spam: bool, mut model: Model) {
    model.learn(&of(m), spam);
    storage_set("model", &model.write());
}

fn existing(m: &Envelope) {
    if !model::learns_existing(&config()) {
        return;
    }
    let model = load();
    if model.learned_existing >= EXISTING_SPAM + EXISTING_GOOD {
        return;
    }
    let screened = m.mailbox.eq_ignore_ascii_case(SCREENED);
    let settled = age(m.date.as_deref()).is_some_and(|a| a > SETTLED_SECS);
    let lobby = m.mailbox.eq_ignore_ascii_case("Lobby") || m.labels.iter().any(|l| l.eq_ignore_ascii_case("Lobby"));
    let mut model = model;
    if screened && model.spam < EXISTING_SPAM {
        model.learned_existing += 1;
        learn_from(m, true, model);
    } else if !screened && !lobby && settled && model.good < EXISTING_GOOD {
        model.learned_existing += 1;
        learn_from(m, false, model);
    }
}

fn judge(m: &Envelope) -> Vec<Action> {
    let model = load();
    if !model.ready() || knows_sender(m) {
        return Vec::new();
    }
    let (score, why) = model.score(&of(m));
    if score < model::threshold(&config()) {
        return Vec::new();
    }
    let pct = (score * 100.0).floor().min(99.0);
    let because = if why.is_empty() { String::new() } else { format!(" ({})", why.join(", ")) };
    vec![
        Action::MoveTo(SPAM.into()),
        Action::Warn(format!("Moved to Spam by the spam filter, {pct:.0}% sure{because}. If it isn't spam, press e and the filter learns from it.")),
    ]
}

impl Guest for Spam {
    fn manifest() -> Manifest {
        Manifest {
            id: "spam".into(),
            name: "Spam filter".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ReadBodies, Permission::ModifyMail, Permission::Storage],
            network_hosts: vec![],
            scan_existing: true,
        }
    }

    fn on_message_received(m: Envelope, reason: CheckReason) -> Vec<Action> {
        match reason {
            CheckReason::Arrived => judge(&m),
            CheckReason::MarkedSpam => {
                learn_from(&m, true, load());
                Vec::new()
            }
            CheckReason::NotSpam => {
                learn_from(&m, false, load());
                Vec::new()
            }
            CheckReason::Existing => {
                existing(&m);
                Vec::new()
            }
            CheckReason::Opened => Vec::new(),
        }
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

    fn check_draft(_text: String) -> Vec<wax::plugin::types::DraftIssue> {
        Vec::new()
    }

    fn on_sender_key(_account: String, _who: Sender, _key: String) -> Result<Vec<Action>, String> {
        Ok(Vec::new())
    }
}

export!(Spam);
