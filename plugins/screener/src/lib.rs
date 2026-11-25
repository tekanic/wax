//! The Screener: a first email from someone new waits in the Lobby until
//! you decide. Wax keeps the Lobby itself: letting someone in or screening
//! them out writes a sender rule and empties their Lobby.
//!
//! config.toml lists the accounts to screen. An account is screened from
//! the first new mail seen after it was added (remembered in storage), so
//! switching it on never sweeps up old mail.

wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

use wax::plugin::host::{config, knows_sender, storage_get, storage_set};
use wax::plugin::types::Permission;

const LOBBY: &str = "Lobby";
/// Mail older than this when it arrives isn't "new" and is never held.
const RECENT_SECS: i64 = 3 * 24 * 3600;

struct Screener;

/// The accounts in config.toml (`screen = "you@example.com"`), lowercased.
fn accounts(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            let (k, v) = l.split_once('=')?;
            if l.starts_with('#') || k.trim() != "screen" {
                return None;
            }
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|v| v.split_once('"')).map(|(v, _)| v).unwrap_or(v);
            v.contains('@').then(|| v.to_lowercase())
        })
        .collect()
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Unix seconds from RFC 3339, e.g. "2026-11-24T21:13:00+00:00".
fn parse_date(s: &str) -> Option<i64> {
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, mo, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let rest = s.get(19..)?.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match rest {
        "Z" | "z" | "" => 0,
        r => {
            let sign = if r.starts_with('-') { -1 } else { 1 };
            let oh: i64 = r.get(1..3)?.parse().ok()?;
            let om: i64 = r.get(4..6)?.parse().ok()?;
            sign * (oh * 3600 + om * 60)
        }
    };
    Some(days(y, mo, d) * 86400 + h * 3600 + mi * 60 + se - offset)
}

/// Whether a message is new enough to hold: after the account was
/// switched on, and from the last few days.
fn fresh(ts: i64, since: i64, now: i64) -> bool {
    ts >= since && ts >= now - RECENT_SECS
}

impl Guest for Screener {
    fn manifest() -> Manifest {
        Manifest {
            id: "screener".into(),
            name: "Screener".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ModifyMail, Permission::Storage],
            network_hosts: vec![],
            scan_existing: false,
        }
    }

    fn on_message_received(m: Envelope, reason: CheckReason) -> Vec<Action> {
        if reason != CheckReason::Arrived {
            return Vec::new();
        }
        let account = m.account.to_lowercase();
        if !m.mailbox.eq_ignore_ascii_case("INBOX") || !accounts(&config()).contains(&account) {
            return Vec::new();
        }
        let now = now();
        let ts = m.date.as_deref().and_then(parse_date).unwrap_or(now);
        let key = format!("since.{account}");
        let since = match storage_get(&key).and_then(|v| v.parse().ok()) {
            Some(s) => s,
            None => {
                storage_set(&key, &ts.min(now).to_string());
                ts.min(now)
            }
        };
        if !fresh(ts, since, now) || knows_sender(&m) {
            return Vec::new();
        }
        let who = m.from.first().map(|a| a.name.clone().unwrap_or_else(|| a.email.clone())).unwrap_or_default();
        vec![Action::MoveTo(LOBBY.into()), Action::Notify(format!("{who} is new: waiting in the Lobby"))]
    }

    fn commands() -> Vec<Command> {
        Vec::new()
    }

    fn run_command(_id: String, _choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        Err("Screener has no commands".into())
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

export!(Screener);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_accounts() {
        let text = "# screen = \"no@x.com\"\nscreen = \"Me@Gmail.com\"\nscreen = work@acme.com\nother = \"x@y.z\"\n";
        assert_eq!(accounts(text), ["me@gmail.com", "work@acme.com"]);
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_date("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_date("2026-11-24T21:13:00+00:00"), Some(1795554780));
        assert_eq!(parse_date("2026-11-24T16:13:00-05:00"), Some(1795554780));
        assert_eq!(parse_date("2026-11-24T21:13:00.250Z"), Some(1795554780));
        assert_eq!(parse_date("nonsense"), None);
    }

    #[test]
    fn holds_only_fresh_mail() {
        let now = 1_000_000;
        assert!(fresh(now, now - 60, now));
        assert!(!fresh(now - 120, now - 60, now), "before it was switched on");
        assert!(!fresh(now - 4 * 86400, 0, now), "more than 3 days old");
    }
}
