//! Mail model shared by the IMAP layer, the local store and the UI.

use gpui_kit::SharedString;

#[derive(Clone)]
pub struct Envelope {
    /// Server mailbox this copy lives in.
    pub mailbox: String,
    /// Backend id (IMAP UID for live mail).
    pub id: String,
    /// RFC 5322 Message-ID, used to de-duplicate Gmail's copies.
    pub message_id: Option<String>,
    pub from: SharedString,
    pub from_email: SharedString,
    pub subject: SharedString,
    /// Unix seconds; 0 when unknown.
    pub ts: i64,
    /// Display date derived from `ts`.
    pub date: SharedString,
    /// One-line preview; empty until the body has been fetched once.
    pub preview: SharedString,
    /// Loaded lazily, then cached.
    pub body: Option<SharedString>,
    /// Original HTML part, when the message has one.
    pub html: Option<SharedString>,
    pub unread: bool,
    /// Gmail star (\\Flagged).
    pub starred: bool,
    /// Gmail's Important label.
    pub important: bool,
    /// Files attached; known once the body has been fetched.
    pub attachments: Vec<Attachment>,
    /// The account it belongs to.
    pub account: SharedString,
    /// To and Cc as (name, address), kept in the cache: Sent shows and
    /// searches them.
    pub to: Vec<(String, String)>,
    /// Gmail's conversation id (X-GM-THRID): messages that share it are
    /// one thread, your replies included.
    pub thread: Option<String>,
    /// Gmail labels as the server lists them (system ones start with a
    /// backslash, e.g. "\\Important"). Newly fetched mail only, like `to`.
    pub labels: Vec<String>,
}

impl Envelope {
    /// Identity in the app. Message numbers (UIDs) only mean something
    /// within one account's folder, so the account is part of it.
    pub fn key(&self) -> (String, String) {
        (format!("{}\u{1f}{}", self.account, self.mailbox), self.id.clone())
    }

    pub fn is(&self, account: &str, mailbox: &str, id: &str) -> bool {
        self.id == id && self.mailbox == mailbox && self.account.as_ref() == account
    }
}

/// A file attached to a message (its bytes are fetched when opened).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub name: String,
    pub mime: String,
    pub size: u64,
}

impl Attachment {
    /// One per line: name, type, size, tab-separated (for the cache).
    pub fn encode(list: &[Attachment]) -> String {
        list.iter()
            .map(|a| format!("{}\t{}\t{}", a.name.replace(['\t', '\n'], " "), a.mime, a.size))
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn decode(s: &str) -> Vec<Attachment> {
        s.lines()
            .filter_map(|l| {
                let mut f = l.split('\t');
                Some(Attachment { name: f.next()?.to_string(), mime: f.next()?.to_string(), size: f.next()?.parse().ok()? })
            })
            .collect()
    }
}

/// "340 KB", "2.4 MB".
pub fn human_size(bytes: u64) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{} KB", (b + 512) / 1024),
        b => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}

/// Mail-client style dates: time today, weekday this week, "Nov 21" this
/// year, "11/21/25" older.
pub fn display_date(ts: i64) -> SharedString {
    use chrono::{Local, TimeZone};
    let Some(d) = Local.timestamp_opt(ts, 0).single().filter(|_| ts > 0) else {
        return "".into();
    };
    let now = Local::now();
    let days = (now.date_naive() - d.date_naive()).num_days();
    let text = if days == 0 {
        d.format("%-I:%M %p").to_string()
    } else if (1..7).contains(&days) {
        d.format("%a").to_string()
    } else if d.format("%Y").to_string() == now.format("%Y").to_string() {
        d.format("%b %-d").to_string()
    } else {
        d.format("%-m/%-d/%y").to_string()
    };
    text.into()
}

/// One readable line for the thread list: no image markers, URLs or
/// footnote numbers.
pub fn make_preview(text: &str) -> String {
    let text = clean_text(text);
    let mut out = String::new();
    let mut in_image = false;
    for word in text.split_whitespace() {
        if in_image {
            in_image = !word.ends_with(']');
            continue;
        }
        if word.starts_with("[image:") || word.starts_with("[Image:") || word.starts_with("[cid:") {
            in_image = !word.ends_with(']');
            continue;
        }
        // Markdown-style links: keep the label, drop the URL.
        if word.contains("](") {
            let label = word.split("](").next().unwrap_or("").trim_start_matches('[');
            if !label.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(label);
            }
            continue;
        }
        let bare = word.trim_matches(|c| matches!(c, '<' | '>' | '(' | ')' | '[' | ']' | '"' | '\''));
        if bare.starts_with("http://") || bare.starts_with("https://") || bare.starts_with("mailto:")
            || bare.starts_with("cid:")
        {
            continue;
        }
        let digits = word.trim_start_matches('[').trim_end_matches(']');
        if word.starts_with('[') && !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.len() > 160 {
            break;
        }
    }
    out
}

/// Decode the HTML entities marketing mail leaves in its text part and
/// drop invisible padding characters (zero-width joiners and friends).
fn clean_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let end = tail.find(';').filter(|&e| e <= 10);
        let decoded = end.and_then(|e| {
            let name = &tail[1..e];
            let c = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                "zwnj" | "zwj" | "shy" => Some('\u{200C}'),
                _ if name.starts_with("#x") || name.starts_with("#X") => {
                    u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32)
                }
                _ if name.starts_with('#') => name[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e + 1))
        });
        match decoded {
            Some((c, len)) => {
                out.push(c);
                rest = &tail[len..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out.chars()
        .filter(|c| !matches!(*c, '\u{034F}' | '\u{00AD}' | '\u{200B}'..='\u{200F}' | '\u{2060}' | '\u{FEFF}'))
        .collect()
}

