wit_bindgen::generate!({ world: "plugin", path: "../../wit/plugin.wit" });

mod when;

use wax::plugin::host::{config, utc_offset};
use wax::plugin::types::{CommandChoice, CommandScope, Permission};
use when::{weekday, Clock};

const SNOOZE: &str = "snooze";
const FOLLOW_UP: &str = "follow-up";

struct Snooze;

fn setting(text: &str, key: &str, default: i64) -> i64 {
    text.lines()
        .filter_map(|l| l.split('#').next()?.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .and_then(|(_, v)| v.trim().parse().ok())
        .filter(|h| (0..24).contains(h))
        .unwrap_or(default)
}

fn clock() -> Clock {
    let cfg = config();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    Clock { now, offset: utc_offset() as i64, morning: setting(&cfg, "morning", 8), evening: setting(&cfg, "evening", 18) }
}

fn presets(id: &str, c: &Clock) -> Vec<(&'static str, &'static str, i64)> {
    let l = c.local();
    let morning = c.morning * 3600;
    let mut out = Vec::new();
    if id == SNOOZE {
        if l.secs < (c.evening - 3) * 3600 {
            out.push(("later", "Later today", c.now + 3 * 3600));
        }
        if l.secs < (c.evening - 1) * 3600 {
            out.push(("evening", "This evening", c.at(l.days, c.evening * 3600)));
        }
        out.push(("tomorrow", "Tomorrow", c.at(l.days + 1, morning)));
        let wd = weekday(l.days);
        if (1..=4).contains(&wd) {
            out.push(("weekend", "This weekend", c.at(l.days + 6 - wd, morning)));
        }
        out.push(("next-week", "Next week", c.at(l.days + (8 - wd).rem_euclid(7).max(1), morning)));
        out.push(("now", "Back to the Inbox now", c.now));
    } else {
        out.push(("tomorrow", "Tomorrow", c.at(l.days + 1, morning)));
        out.push(("3-days", "In 3 days", c.at(l.days + 3, morning)));
        out.push(("week", "In a week", c.at(l.days + 7, morning)));
        out.push(("2-weeks", "In 2 weeks", c.at(l.days + 14, morning)));
    }
    out
}

fn command(id: &str, title: &str, key: &str, scope: CommandScope, c: &Clock) -> Command {
    Command {
        id: id.into(),
        title: title.into(),
        keybinding: Some(key.into()),
        in_message_toolbar: true,
        scope,
        choices: presets(id, c)
            .into_iter()
            .map(|(id, title, t)| CommandChoice { id: id.into(), title: title.into(), detail: if id == "now" { String::new() } else { c.describe(t) } })
            .collect(),
        text_hint: Some("or type a time: fri 9am, 12/3, in 2 days".into()),
    }
}

fn due(id: &str, choice: &str, c: &Clock) -> Result<i64, String> {
    if let Some(text) = choice.strip_prefix("text:") {
        return when::parse(text, c);
    }
    presets(id, c).into_iter().find(|(p, _, _)| *p == choice).map(|(_, _, t)| t).ok_or_else(|| format!("No choice \"{choice}\""))
}

impl Guest for Snooze {
    fn manifest() -> Manifest {
        Manifest {
            id: "snooze".into(),
            name: "Snooze & follow-up".into(),
            version: env!("CARGO_PKG_VERSION").into(),
            permissions: vec![Permission::ReadEnvelopes, Permission::ModifyMail],
            network_hosts: vec![],
            scan_existing: false,
        }
    }

    fn on_message_received(_message: Envelope, _reason: CheckReason) -> Vec<Action> {
        Vec::new()
    }

    fn commands() -> Vec<Command> {
        let c = clock();
        vec![command(SNOOZE, "Snooze", "z", CommandScope::Any, &c), command(FOLLOW_UP, "Remind if no reply", "w", CommandScope::Sent, &c)]
    }

    fn run_command(id: String, choice: Option<String>, _message: Option<Envelope>) -> Result<Vec<Action>, String> {
        let c = clock();
        let choice = choice.ok_or("Pick a time")?;
        let t = due(&id, &choice, &c)?;
        let at = c.rfc3339(t);
        match id.as_str() {
            SNOOZE => Ok(vec![Action::SnoozeUntil(at)]),
            FOLLOW_UP if t > c.now => Ok(vec![Action::FollowUp(at)]),
            FOLLOW_UP => Err("Pick a time in the future".into()),
            _ => Err(format!("No command \"{id}\"")),
        }
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

export!(Snooze);

#[cfg(test)]
mod tests {
    use super::*;
    use when::{days_from_civil, DAY};

    fn at(y: i64, m: i64, d: i64, h: i64, min: i64) -> Clock {
        Clock { now: days_from_civil(y, m, d) * DAY + h * 3600 + min * 60 + 5 * 3600, offset: -5 * 3600, morning: 8, evening: 18 }
    }

    #[test]
    fn snooze_presets_fit_the_time_of_day() {
        let names = |c: &Clock| presets(SNOOZE, c).into_iter().map(|(n, _, _)| n).collect::<Vec<_>>();
        assert_eq!(names(&at(2026, 11, 25, 13, 30)), ["later", "evening", "tomorrow", "weekend", "next-week", "now"]);
        assert_eq!(names(&at(2026, 11, 25, 17, 30)), ["tomorrow", "weekend", "next-week", "now"]);
        assert_eq!(names(&at(2026, 11, 28, 10, 0)), ["later", "evening", "tomorrow", "next-week", "now"]);
        let c = at(2026, 11, 25, 13, 30);
        assert_eq!(c.rfc3339(due(SNOOZE, "weekend", &c).unwrap()), "2026-11-28T08:00:00-05:00");
        assert_eq!(c.rfc3339(due(SNOOZE, "next-week", &c).unwrap()), "2026-11-30T08:00:00-05:00");
        assert_eq!(c.rfc3339(due(FOLLOW_UP, "3-days", &c).unwrap()), "2026-11-28T08:00:00-05:00");
        assert_eq!(c.rfc3339(due(SNOOZE, "text:fri 9am", &c).unwrap()), "2026-11-27T09:00:00-05:00");
        assert!(due(SNOOZE, "nope", &c).is_err());
    }

    #[test]
    fn reads_hours_from_config() {
        assert_eq!(setting("morning = 7\nevening = 20 # late\n", "evening", 18), 20);
        assert_eq!(setting("morning = 7\n", "morning", 8), 7);
        assert_eq!(setting("morning = 30\n", "morning", 8), 8);
        assert_eq!(setting("", "morning", 8), 8);
    }
}
