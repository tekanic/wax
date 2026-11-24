//! A record of things that went wrong, kept in `errors.log` in the data
//! folder (one line each: time, account, message) and shown under
//! Settings → Problems. Status messages fade from the bar; this keeps them.
//!
//! The same message from the same account within a few minutes is written
//! once (a watcher that can't connect would otherwise log every 30 s), and
//! the file keeps its newest lines only.

use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const REPEAT_QUIET: Duration = Duration::from_secs(600);
const KEEP_LINES: usize = 1000;
const TRIM_AT_BYTES: u64 = 400 * 1024;

pub fn path() -> PathBuf {
    crate::files::data_dir().join("errors.log")
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    /// Unix seconds.
    pub ts: i64,
    pub account: String,
    pub text: String,
}

fn recent() -> &'static Mutex<Vec<(String, Instant)>> {
    static R: OnceLock<Mutex<Vec<(String, Instant)>>> = OnceLock::new();
    R.get_or_init(Default::default)
}

fn line(e: &Entry) -> String {
    let clean = |s: &str| s.replace(['\n', '\r', '\t'], " ");
    format!("{}\t{}\t{}\n", e.ts, clean(&e.account), clean(&e.text))
}

pub fn parse(text: &str) -> Vec<Entry> {
    let mut out: Vec<Entry> = text
        .lines()
        .filter_map(|l| {
            let mut parts = l.splitn(3, '\t');
            let ts = parts.next()?.parse().ok()?;
            Some(Entry { ts, account: parts.next()?.to_string(), text: parts.next()?.to_string() })
        })
        .collect();
    out.reverse(); // newest first
    out
}

/// Write one problem down. Returns false if it was a recent repeat.
pub fn record(account: &str, text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let key = format!("{account}\u{1f}{text}");
    {
        let mut r = recent().lock().unwrap();
        r.retain(|(_, t)| t.elapsed() < REPEAT_QUIET);
        if r.iter().any(|(k, _)| *k == key) {
            return false;
        }
        r.push((key, Instant::now()));
    }
    let e = Entry { ts: chrono::Utc::now().timestamp(), account: account.to_string(), text: text.to_string() };
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        f.write_all(line(&e).as_bytes()).ok();
    }
    if std::fs::metadata(&p).map(|m| m.len() > TRIM_AT_BYTES).unwrap_or(false) {
        if let Ok(text) = std::fs::read_to_string(&p) {
            let lines: Vec<&str> = text.lines().collect();
            let keep = lines[lines.len().saturating_sub(KEEP_LINES)..].join("\n") + "\n";
            std::fs::write(&p, keep).ok();
        }
    }
    true
}

pub fn load() -> Vec<Entry> {
    std::fs::read_to_string(path()).map(|t| parse(&t)).unwrap_or_default()
}

pub fn clear() {
    std::fs::write(path(), "").ok();
    recent().lock().unwrap().clear();
}

/// "Nov 24, 8:51 AM" in local time.
pub fn when(ts: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_opt(ts, 0).single().map(|d| d.format("%b %-d, %-I:%M:%S %p").to_string()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{line, parse, Entry};

    #[test]
    fn lines_round_trip() {
        let a = Entry { ts: 1, account: "me@x.com".into(), text: "Gmail refused:\nNO [NONEXISTENT]".into() };
        let b = Entry { ts: 2, account: "".into(), text: "Offline".into() };
        let text = line(&a) + &line(&b);
        let back = parse(&text);
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].text, "Offline"); // newest first
        assert_eq!(back[1].text, "Gmail refused: NO [NONEXISTENT]");
    }
}
