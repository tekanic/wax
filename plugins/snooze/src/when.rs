#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clock {
    pub now: i64,
    pub offset: i64,
    pub morning: i64,
    pub evening: i64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Local {
    pub days: i64,
    pub secs: i64,
}

pub const DAY: i64 = 86_400;

pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { yoe + era * 400 + 1 } else { yoe + era * 400 }, m, d)
}

pub fn weekday(days: i64) -> i64 {
    (days + 4).rem_euclid(7)
}

impl Clock {
    pub fn local(&self) -> Local {
        let t = self.now + self.offset;
        Local { days: t.div_euclid(DAY), secs: t.rem_euclid(DAY) }
    }

    pub fn at(&self, days: i64, secs: i64) -> i64 {
        days * DAY + secs - self.offset
    }

    pub fn rfc3339(&self, t: i64) -> String {
        let l = t + self.offset;
        let (y, m, d) = civil_from_days(l.div_euclid(DAY));
        let s = l.rem_euclid(DAY);
        let off = self.offset.abs();
        format!(
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
            s / 3600,
            s % 3600 / 60,
            s % 60,
            if self.offset < 0 { '-' } else { '+' },
            off / 3600,
            off % 3600 / 60
        )
    }

    pub fn describe(&self, t: i64) -> String {
        let l = t + self.offset;
        let days = l.div_euclid(DAY);
        let s = l.rem_euclid(DAY);
        let today = self.local().days;
        let (h, m) = (s / 3600, s % 3600 / 60);
        let hour = match h % 12 {
            0 => 12,
            h => h,
        };
        let time = if m == 0 { format!("{hour} {}", if h < 12 { "AM" } else { "PM" }) } else { format!("{hour}:{m:02} {}", if h < 12 { "AM" } else { "PM" }) };
        let day = match days - today {
            0 => "today".to_string(),
            1 => "tomorrow".to_string(),
            2..=6 => WEEKDAYS[weekday(days) as usize].0.to_string(),
            _ => {
                let (_, mo, d) = civil_from_days(days);
                format!("{} {d}", MONTHS[mo as usize - 1].0)
            }
        };
        format!("{day} {time}")
    }
}

const WEEKDAYS: [(&str, &str); 7] =
    [("Sun", "sunday"), ("Mon", "monday"), ("Tue", "tuesday"), ("Wed", "wednesday"), ("Thu", "thursday"), ("Fri", "friday"), ("Sat", "saturday")];

const MONTHS: [(&str, &str); 12] = [
    ("Jan", "january"),
    ("Feb", "february"),
    ("Mar", "march"),
    ("Apr", "april"),
    ("May", "may"),
    ("Jun", "june"),
    ("Jul", "july"),
    ("Aug", "august"),
    ("Sep", "september"),
    ("Oct", "october"),
    ("Nov", "november"),
    ("Dec", "december"),
];

fn weekday_of(word: &str) -> Option<i64> {
    WEEKDAYS.iter().position(|(_, full)| word.len() >= 3 && full.starts_with(word)).map(|i| i as i64)
}

fn month_of(word: &str) -> Option<i64> {
    MONTHS.iter().position(|(_, full)| word.len() >= 3 && full.starts_with(word)).map(|i| i as i64 + 1)
}

fn time_of(word: &str) -> Option<i64> {
    match word {
        "noon" => return Some(12 * 3600),
        "midnight" => return Some(0),
        _ => {}
    }
    let (digits, pm) = if let Some(d) = word.strip_suffix("am").or_else(|| word.strip_suffix('a')) {
        (d, Some(false))
    } else if let Some(d) = word.strip_suffix("pm").or_else(|| word.strip_suffix('p')) {
        (d, Some(true))
    } else {
        (word, None)
    };
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h.parse::<i64>().ok()?, m.parse::<i64>().ok()?),
        None if pm.is_some() => (digits.parse::<i64>().ok()?, 0),
        None => return None,
    };
    if m >= 60 {
        return None;
    }
    let h = match pm {
        Some(true) if (1..=12).contains(&h) => h % 12 + 12,
        Some(false) if (1..=12).contains(&h) => h % 12,
        None if (0..=23).contains(&h) => h,
        _ => return None,
    };
    Some(h * 3600 + m * 60)
}

fn date_of(word: &str, today: i64) -> Option<i64> {
    let (y, m, d) = if let Some((y, rest)) = word.split_once('-').filter(|(y, _)| y.len() == 4) {
        let (m, d) = rest.split_once('-')?;
        (y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
    } else {
        let mut parts = word.split('/');
        let m: i64 = parts.next()?.parse().ok()?;
        let d: i64 = parts.next()?.parse().ok()?;
        let y = match parts.next() {
            Some(y) => {
                let y: i64 = y.parse().ok()?;
                if y < 100 { 2000 + y } else { y }
            }
            None => {
                let (y, _, _) = civil_from_days(today);
                if days_from_civil(y, m, d) < today { y + 1 } else { y }
            }
        };
        (y, m, d)
    };
    ((1..=12).contains(&m) && (1..=31).contains(&d)).then(|| days_from_civil(y, m, d))
}

fn month_day(month: i64, day: i64, today: i64) -> Option<i64> {
    if !(1..=31).contains(&day) {
        return None;
    }
    let (y, _, _) = civil_from_days(today);
    let this = days_from_civil(y, month, day);
    Some(if this < today { days_from_civil(y + 1, month, day) } else { this })
}

pub fn parse(text: &str, c: &Clock) -> Result<i64, String> {
    let text = text.trim().to_lowercase();
    let words: Vec<&str> = text.split(|ch: char| ch.is_whitespace() || ch == ',').filter(|w| !w.is_empty() && *w != "at" && *w != "on").collect();
    if words.is_empty() {
        return Err("Type a time, e.g. \"fri 9am\"".into());
    }
    if words == ["now"] {
        return Ok(c.now);
    }
    if words[0] == "in" {
        let n: i64 = words.get(1).and_then(|w| w.parse().ok()).ok_or("Try \"in 2 days\" or \"in 3 hours\"")?;
        let unit = words.get(2).copied().unwrap_or("");
        let secs = match unit.trim_end_matches('s') {
            "min" | "minute" | "m" => 60,
            "hour" | "hr" | "h" => 3600,
            "day" | "d" => return Ok(c.at(c.local().days + n, c.morning * 3600)),
            "week" | "wk" | "w" => return Ok(c.at(c.local().days + 7 * n, c.morning * 3600)),
            _ => return Err(format!("Don't know \"{unit}\": try minutes, hours, days or weeks")),
        };
        return Ok(c.now + n * secs);
    }
    let local = c.local();
    let today = local.days;
    let mut day: Option<i64> = None;
    let mut time: Option<i64> = None;
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        let next = words.get(i + 1).copied();
        if let Some(t) = time_of(w) {
            time = Some(t);
        } else if let Some(t) = next.and_then(|n| matches!(n, "am" | "pm").then(|| time_of(&format!("{w}{n}"))).flatten()) {
            time = Some(t);
            i += 1;
        } else if w == "today" {
            day = Some(today);
        } else if w == "tonight" {
            day = Some(today);
            time = time.or(Some(c.evening * 3600));
        } else if matches!(w, "tomorrow" | "tmrw" | "tmr" | "tom") {
            day = Some(today + 1);
        } else if w == "next" && next == Some("week") {
            day = Some(today + (8 - weekday(today)).rem_euclid(7).max(1));
            i += 1;
        } else if w == "weekend" || (w == "this" && next == Some("weekend")) {
            let sat = today + (6 - weekday(today)).rem_euclid(7);
            day = Some(if sat == today && local.secs >= c.morning * 3600 { today + 7 } else { sat });
            if w == "this" {
                i += 1;
            }
        } else if w == "next" {
        } else if let Some(wd) = weekday_of(w) {
            let ahead = (wd - weekday(today)).rem_euclid(7);
            day = Some(today + if ahead == 0 { 7 } else { ahead });
        } else if let Some(m) = month_of(w) {
            let d = next.and_then(|n| n.trim_end_matches(|ch: char| ch.is_alphabetic()).parse::<i64>().ok()).ok_or(format!("Which day of {w}?"))?;
            day = Some(month_day(m, d, today).ok_or(format!("No day {d} in {w}"))?);
            i += 1;
        } else if let (Ok(d), Some(m)) = (w.trim_end_matches(|ch: char| ch.is_alphabetic()).parse::<i64>(), next.and_then(month_of)) {
            day = Some(month_day(m, d, today).ok_or(format!("No day {d} there"))?);
            i += 1;
        } else if let Some(d) = date_of(w, today) {
            day = Some(d);
        } else {
            return Err(format!("Couldn't read \"{w}\" as a time"));
        }
        i += 1;
    }
    let t = match (day, time) {
        (Some(d), Some(t)) => c.at(d, t),
        (Some(d), None) => c.at(d, c.morning * 3600),
        (None, Some(t)) => {
            let at = c.at(today, t);
            if at > c.now { at } else { c.at(today + 1, t) }
        }
        (None, None) => return Err("Type a time, e.g. \"fri 9am\"".into()),
    };
    if t <= c.now {
        return Err("That time has already passed".into());
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> Clock {
        let wed_1330_est = days_from_civil(2026, 11, 25) * DAY + 13 * 3600 + 30 * 60 + 5 * 3600;
        Clock { now: wed_1330_est, offset: -5 * 3600, morning: 8, evening: 18 }
    }

    fn show(t: Result<i64, String>) -> String {
        match t {
            Ok(t) => clock().rfc3339(t),
            Err(e) => format!("error: {e}"),
        }
    }

    #[test]
    fn reads_times() {
        let c = clock();
        let p = |s: &str| show(parse(s, &c));
        assert_eq!(weekday(c.local().days), 3);
        assert_eq!(p("tomorrow"), "2026-11-26T08:00:00-05:00");
        assert_eq!(p("tomorrow 9:30pm"), "2026-11-26T21:30:00-05:00");
        assert_eq!(p("fri 9am"), "2026-11-27T09:00:00-05:00");
        assert_eq!(p("Friday at 9 am"), "2026-11-27T09:00:00-05:00");
        assert_eq!(p("wed"), "2026-12-02T08:00:00-05:00");
        assert_eq!(p("tonight"), "2026-11-25T18:00:00-05:00");
        assert_eq!(p("5pm"), "2026-11-25T17:00:00-05:00");
        assert_eq!(p("9am"), "2026-11-26T09:00:00-05:00");
        assert_eq!(p("17:15"), "2026-11-25T17:15:00-05:00");
        assert_eq!(p("12/3"), "2026-12-03T08:00:00-05:00");
        assert_eq!(p("12/3 5pm"), "2026-12-03T17:00:00-05:00");
        assert_eq!(p("1/5"), "2027-01-05T08:00:00-05:00");
        assert_eq!(p("2027-02-01 noon"), "2027-02-01T12:00:00-05:00");
        assert_eq!(p("dec 3"), "2026-12-03T08:00:00-05:00");
        assert_eq!(p("3 december 2pm"), "2026-12-03T14:00:00-05:00");
        assert_eq!(p("next week"), "2026-11-30T08:00:00-05:00");
        assert_eq!(p("this weekend"), "2026-11-28T08:00:00-05:00");
        assert_eq!(p("in 2 days"), "2026-11-27T08:00:00-05:00");
        assert_eq!(p("in 3 hours"), "2026-11-25T16:30:00-05:00");
        assert_eq!(p("in 1 week"), "2026-12-02T08:00:00-05:00");
        assert!(p("11/1").starts_with("2027-11-01"));
        assert!(p("today 9am").starts_with("error: That time has already passed"));
        assert!(p("blah").starts_with("error: Couldn't read \"blah\""));
        assert!(p("").starts_with("error"));
        assert_eq!(parse("now", &c), Ok(c.now));
    }

    #[test]
    fn describes_times() {
        let c = clock();
        assert_eq!(c.describe(parse("tomorrow", &c).unwrap()), "tomorrow 8 AM");
        assert_eq!(c.describe(parse("5pm", &c).unwrap()), "today 5 PM");
        assert_eq!(c.describe(parse("fri 9:30am", &c).unwrap()), "Fri 9:30 AM");
        assert_eq!(c.describe(parse("12/3", &c).unwrap()), "Dec 3 8 AM");
    }

    #[test]
    fn calendar_round_trips() {
        for d in [-1000, 0, 19_000, 20_517, 30_000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
        assert_eq!(weekday(days_from_civil(1970, 1, 1)), 4);
    }
}
