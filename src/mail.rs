//! Live IMAP access for the spike. Read-only: message bodies are fetched
//! with BODY.PEEK, so nothing is marked read on the server.
//!
//! Logging in is per account (see accounts.rs): Sign in with Google, or
//! the older app password kept in the Keychain (service "gpui-mail").

use crate::model::Envelope;
use anyhow::{anyhow, Result};
use io_email::client::EmailClientStd;
use pimalaya_stream::tls::Tls;
use std::process::Command;
use url::Url;

pub const DRAFTS: &str = "[Gmail]/Drafts";
pub const TRASH: &str = "[Gmail]/Trash";
pub const SPAM: &str = "[Gmail]/Spam";
pub const STARRED: &str = "[Gmail]/Starred";
pub const IMPORTANT: &str = "[Gmail]/Important";
pub const INBOX: &str = "INBOX";
pub const KEYCHAIN_SERVICE: &str = "gpui-mail";

pub fn keychain_password(account: &str) -> Result<String> {
    let out = Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", account, "-w"])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!(
            "No app password in Keychain (service \"{KEYCHAIN_SERVICE}\", account {account})"
        ));
    }
    // Google shows app passwords as four groups; spaces are optional.
    Ok(String::from_utf8(out.stdout)?.trim().replace(' ', ""))
}

/// MESSAGES and UNSEEN from a `* STATUS name (MESSAGES n UNSEEN m)` line.
/// Reads the last parenthesised group, so a folder named "MESSAGES" is fine.
pub fn parse_status_counts(text: &str) -> Option<(u64, u64)> {
    let line = text.lines().find(|l| l.starts_with("* STATUS "))?;
    let inner = line[line.rfind('(')? + 1..].trim_end().trim_end_matches(')');
    let words: Vec<&str> = inner.split_whitespace().collect();
    let get = |key: &str| words.windows(2).find(|w| w[0].eq_ignore_ascii_case(key))?.get(1)?.parse().ok();
    Some((get("MESSAGES")?, get("UNSEEN")?))
}

#[cfg(test)]
mod status_tests {
    #[test]
    fn reads_status_counts() {
        let t = "* STATUS \"Screened Out\" (MESSAGES 3969 UNSEEN 77)\r\ngm3 OK Success\r\n";
        assert_eq!(super::parse_status_counts(t), Some((3969, 77)));
        let t = "* STATUS \"MESSAGES (old)\" (MESSAGES 5 UNSEEN 0)\r\n";
        assert_eq!(super::parse_status_counts(t), Some((5, 0)));
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MailboxStatus {
    pub uid_validity: u64,
    pub uid_next: u64,
    pub exists: u64,
}

/// Log in to `account`'s IMAP server. Gmail sometimes takes longer than
/// the connection's 5 s read limit to answer a sign-in; a sign-in that
/// didn't finish changed nothing, so it's simply tried again (3 tries).
fn login(account: &str) -> Result<EmailClientStd> {
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(1500 * attempt));
        }
        let url = Url::parse(crate::accounts::imap_url(account))?;
        let sasl = crate::accounts::sasl(account)?;
        match EmailClientStd::new().connect_imap(&url, &Tls::default(), false, Some(sasl), None) {
            Ok(c) => return Ok(c),
            Err(e) => {
                let text = e.to_string();
                let slow = text.contains("os error 35") || text.contains("temporarily unavailable") || text.contains("timed out");
                last = Some(anyhow!("IMAP login failed: {text}"));
                if !slow {
                    break;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("IMAP login failed")))
}

/// Most connections the app keeps open to one account at once. Gmail allows
/// about 15 per account across all your mail apps (phone, Thunderbird…);
/// staying well under it leaves room for them.
pub const MAX_CONNECTIONS: usize = 4;

fn open_counts() -> &'static std::sync::Mutex<std::collections::HashMap<String, usize>> {
    static C: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, usize>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// Connections open to `account` right now.
pub fn open_connections(account: &str) -> usize {
    open_counts().lock().unwrap().get(&account.to_lowercase()).copied().unwrap_or(0)
}

/// Holds one of an account's connection slots; gives it back when dropped.
struct Slot(String);

impl Slot {
    fn take(account: &str) -> Result<Slot> {
        let key = account.to_lowercase();
        let mut c = open_counts().lock().unwrap();
        let n = c.entry(key.clone()).or_insert(0);
        if *n >= MAX_CONNECTIONS {
            return Err(anyhow!("Wax already has {MAX_CONNECTIONS} connections open to {account}; trying again shortly"));
        }
        *n += 1;
        Ok(Slot(key))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(n) = open_counts().lock().unwrap().get_mut(&self.0) {
            *n = n.saturating_sub(1);
        }
    }
}

pub struct Session {
    client: EmailClientStd,
    _slot: Slot,
    /// The account it's logged in to; stamped on every message it returns.
    pub account: String,
}

impl Session {
    /// Log in again on a fresh connection (same account).
    fn reconnect(&mut self) -> Result<()> {
        // Same slot: the dead connection is replaced, not added to.
        self.client = login(&self.account)?;
        Ok(())
    }

    /// Run `f`; if the server had dropped the connection (idle timeout,
    /// sleep, Wi-Fi, an expired sign-in), log in again and run it once more.
    fn retry<T>(&mut self, mut f: impl FnMut(&mut Self) -> Result<T>) -> Result<T> {
        match f(self) {
            Err(e) if is_dropped(&e) => {
                self.reconnect()?;
                f(self)
            }
            other => other,
        }
    }

    fn stamp(&self, mut list: Vec<Envelope>) -> Vec<Envelope> {
        let a: gpui_kit::SharedString = self.account.clone().into();
        for e in &mut list {
            e.account = a.clone();
        }
        list
    }

    pub fn connect(account: &str) -> Result<Self> {
        let slot = Slot::take(account)?;
        let client = login(account)?;
        Ok(Self { client, _slot: slot, account: account.to_lowercase() })
    }

    /// Mailbox names, INBOX first, Gmail's virtual folders after.
    pub fn mailboxes(&mut self) -> Result<Vec<String>> {
        self.retry(|s| s.mailboxes_once())
    }

    fn mailboxes_once(&mut self) -> Result<Vec<String>> {
        let mut names: Vec<String> = self
            .client
            .list_mailboxes(false)
            .map_err(|e| anyhow!("{e}"))?
            .into_iter()
            .map(|m| if m.name.eq_ignore_ascii_case("INBOX") { "INBOX".to_string() } else { m.name })
            .filter(|n| n != "[Gmail]")
            .collect();
        names.sort_by_key(|n| (!n.eq_ignore_ascii_case("INBOX"), n.starts_with("[Gmail]"), n.to_lowercase()));
        Ok(names)
    }

    /// The most recent `count` envelopes, newest first.
    pub fn envelopes(&mut self, mailbox: &str, count: u32) -> Result<Vec<Envelope>> {
        let list = self.envelopes_unstamped(mailbox, count)?;
        Ok(self.stamp(list))
    }

    fn envelopes_unstamped(&mut self, mailbox: &str, count: u32) -> Result<Vec<Envelope>> {
        self.retry(|s| s.envelopes_unstamped_once(mailbox, count))
    }

    fn envelopes_unstamped_once(&mut self, mailbox: &str, count: u32) -> Result<Vec<Envelope>> {
        let mut list = self
            .client
            .list_envelopes(mailbox, Some(1), Some(count), false)
            .map_err(|e| anyhow!("{e}"))?;
        list.sort_by(|a, b| b.date.cmp(&a.date));
        let mut out: Vec<Envelope> = list.into_iter().map(|e| to_model(mailbox, e)).collect();
        self.annotate_labels(mailbox, &mut out)?;
        Ok(out)
    }

    /// Page `page` (1 = newest) of `size` envelopes, newest first.
    pub fn envelopes_page(&mut self, mailbox: &str, page: u32, size: u32) -> Result<Vec<Envelope>> {
        let list = self.envelopes_page_unstamped(mailbox, page, size)?;
        Ok(self.stamp(list))
    }

    fn envelopes_page_unstamped(&mut self, mailbox: &str, page: u32, size: u32) -> Result<Vec<Envelope>> {
        self.retry(|s| s.envelopes_page_unstamped_once(mailbox, page, size))
    }

    fn envelopes_page_unstamped_once(&mut self, mailbox: &str, page: u32, size: u32) -> Result<Vec<Envelope>> {
        let mut list = self
            .client
            .list_envelopes(mailbox, Some(page), Some(size), false)
            .map_err(|e| anyhow!("{e}"))?;
        list.sort_by(|a, b| b.date.cmp(&a.date));
        let mut out: Vec<Envelope> = list.into_iter().map(|e| to_model(mailbox, e)).collect();
        self.annotate_labels(mailbox, &mut out)?;
        Ok(out)
    }

    /// Fill in Gmail's Important label, which the generic envelope listing
    /// doesn't carry.
    fn annotate_labels(&mut self, mailbox: &str, list: &mut [Envelope]) -> Result<()> {
        if list.is_empty() || mailbox.eq_ignore_ascii_case(IMPORTANT) {
            return Ok(());
        }
        let ids: Vec<&str> = list.iter().map(|e| e.id.as_str()).collect();
        let bytes = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID FETCH {} (UID X-GM-LABELS)", ids.join(",")),
        ])?;
        let text = String::from_utf8_lossy(&bytes);
        let important: std::collections::HashSet<String> = text
            .lines()
            .filter(|l| l.starts_with("* ") && l.contains("\\Important"))
            .filter_map(|l| {
                let at = l.find("UID ")? + 4;
                Some(l[at..].chars().take_while(|c| c.is_ascii_digit()).collect::<String>())
            })
            .collect();
        for e in list.iter_mut() {
            e.important = important.contains(&e.id);
        }
        Ok(())
    }

    /// (UIDNEXT, unread, total) of a mailbox without opening it: cheap
    /// enough to ask every few seconds.
    pub fn status(&mut self, mailbox: &str) -> Result<(u64, u64, u64)> {
        let bytes = self.raw(&[format!("STATUS {} (UIDNEXT UNSEEN MESSAGES)", imap_quote(mailbox))])?;
        let text = String::from_utf8_lossy(&bytes);
        let line = text.lines().find(|l| l.starts_with("* STATUS")).ok_or_else(|| anyhow!("no STATUS reply"))?;
        let field = |key: &str| -> Option<u64> {
            let at = line.find(&format!("{key} "))? + key.len() + 1;
            line[at..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
        };
        Ok((
            field("UIDNEXT").ok_or_else(|| anyhow!("no UIDNEXT"))?,
            field("UNSEEN").unwrap_or(0),
            field("MESSAGES").unwrap_or(0),
        ))
    }

    /// UIDVALIDITY, UIDNEXT and message count, via a read-only EXAMINE.
    pub fn mailbox_status(&mut self, mailbox: &str) -> Result<MailboxStatus> {
        let bytes = self.raw(&[format!("EXAMINE {}", imap_quote(mailbox))])?;
        let text = String::from_utf8_lossy(&bytes);
        let bracket = |key: &str| -> Option<u64> {
            let at = text.find(&format!("[{key} "))? + key.len() + 2;
            text[at..].split(']').next()?.trim().parse().ok()
        };
        let exists = text
            .lines()
            .find_map(|l| l.strip_prefix("* ")?.strip_suffix(" EXISTS")?.trim().parse().ok())
            .unwrap_or(0);
        Ok(MailboxStatus {
            uid_validity: bracket("UIDVALIDITY").ok_or_else(|| anyhow!("no UIDVALIDITY"))?,
            uid_next: bracket("UIDNEXT").ok_or_else(|| anyhow!("no UIDNEXT"))?,
            exists,
        })
    }

    /// (total, unread) for `mailbox`, counted by the server.
    pub fn folder_counts(&mut self, mailbox: &str) -> Result<(u64, u64)> {
        let bytes = self.raw(&[format!("STATUS {} (MESSAGES UNSEEN)", imap_quote(mailbox))])?;
        parse_status_counts(&String::from_utf8_lossy(&bytes)).ok_or_else(|| anyhow!("no STATUS counts for {mailbox}"))
    }

    /// Headers (sender, subject, date, Message-ID, flags) of every message
    /// with a UID in `lo..=hi`. Read-only.
    pub fn headers_in_range(&mut self, mailbox: &str, lo: u64, hi: u64) -> Result<Vec<Envelope>> {
        let list = self.headers_in_range_unstamped(mailbox, lo, hi)?;
        Ok(self.stamp(list))
    }

    fn headers_in_range_unstamped(&mut self, mailbox: &str, lo: u64, hi: u64) -> Result<Vec<Envelope>> {
        self.retry(|s| s.headers_in_range_unstamped_once(mailbox, lo, hi))
    }

    fn headers_in_range_unstamped_once(&mut self, mailbox: &str, lo: u64, hi: u64) -> Result<Vec<Envelope>> {
        let bytes = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID FETCH {lo}:{hi} (UID FLAGS X-GM-LABELS BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE MESSAGE-ID)])"),
        ])?;
        Ok(parse_header_fetch(mailbox, &bytes))
    }

    /// Raw To/Cc/Subject/Date headers of the newest `n` messages (debugging).
    pub fn recent_headers(&mut self, mailbox: &str, n: u64) -> Result<String> {
        self.retry(|s| s.recent_headers_once(mailbox, n))
    }

    fn recent_headers_once(&mut self, mailbox: &str, n: u64) -> Result<String> {
        let st = self.mailbox_status(mailbox)?;
        let lo = st.uid_next.saturating_sub(n * 20).max(1);
        let bytes = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID FETCH {lo}:* (UID BODY.PEEK[HEADER.FIELDS (TO CC SUBJECT DATE)])"),
        ])?;
        Ok(String::from_utf8_lossy(&bytes).to_string())
    }

    /// (name, email, unix time) for every To/Cc recipient of messages with a
    /// UID in `lo..=hi` of `mailbox` (the Sent folder). Read-only.
    pub fn recipients_in_range(&mut self, mailbox: &str, lo: u64, hi: u64) -> Result<Vec<(String, String, i64)>> {
        self.retry(|s| s.recipients_in_range_once(mailbox, lo, hi))
    }

    fn recipients_in_range_once(&mut self, mailbox: &str, lo: u64, hi: u64) -> Result<Vec<(String, String, i64)>> {
        let bytes = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID FETCH {lo}:{hi} (UID BODY.PEEK[HEADER.FIELDS (TO CC DATE)])"),
        ])?;
        let mut out = Vec::new();
        for literal in fetch_literals(&bytes) {
            let Some(msg) = mail_parser::MessageParser::default().parse(literal) else { continue };
            let ts = msg.date().map(|d| d.to_timestamp()).unwrap_or(0);
            for list in [msg.to(), msg.cc()].into_iter().flatten() {
                for a in list.iter() {
                    if let Some(email) = a.address() {
                        out.push((a.name().unwrap_or("").to_string(), email.to_string(), ts));
                    }
                }
            }
        }
        Ok(out)
    }

    /// Server-side search with Gmail's own syntax (X-GM-RAW), so it finds
    /// anything the Gmail web search would. Gmail lacks IMAP SORT, which
    /// io-email's search needs, so this speaks IMAP directly: UID SEARCH,
    /// then a header-only UID FETCH for the newest `count` matches.
    pub fn search(&mut self, mailbox: &str, query: &str, count: u32) -> Result<Vec<Envelope>> {
        let list = self.search_unstamped(mailbox, query, count)?;
        Ok(self.stamp(list))
    }

    fn search_unstamped(&mut self, mailbox: &str, query: &str, count: u32) -> Result<Vec<Envelope>> {
        self.retry(|s| s.search_unstamped_once(mailbox, query, count))
    }

    fn search_unstamped_once(&mut self, mailbox: &str, query: &str, count: u32) -> Result<Vec<Envelope>> {
        let q = imap_quote(query.trim());
        let responses = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID SEARCH X-GM-RAW {q}"),
        ])?;
        let text = String::from_utf8_lossy(&responses);
        if std::env::var("GPUI_MAIL_DEBUG").is_ok() {
            eprintln!("--- search response ---\n{}", text.chars().take(1500).collect::<String>());
        }
        let mut uids: Vec<u64> = text
            .lines()
            .filter_map(|l| l.strip_prefix("* SEARCH"))
            .flat_map(|rest| rest.split_whitespace().filter_map(|n| n.parse().ok()).collect::<Vec<_>>())
            .collect();
        uids.sort_unstable();
        let newest: Vec<String> = uids.iter().rev().take(count as usize).map(|u| u.to_string()).collect();
        if newest.is_empty() {
            return Ok(Vec::new());
        }
        let fetched = self.raw(&[format!(
            "UID FETCH {} (UID FLAGS X-GM-LABELS BODY.PEEK[HEADER.FIELDS (FROM SUBJECT DATE MESSAGE-ID)])",
            newest.join(",")
        )])?;
        if std::env::var("GPUI_MAIL_DEBUG").is_ok() {
            eprintln!("--- fetch response ---\n{}", String::from_utf8_lossy(&fetched).chars().take(1200).collect::<String>());
        }
        let mut list = parse_header_fetch(mailbox, &fetched);
        list.sort_by(|a, b| b.ts.cmp(&a.ts));
        Ok(list)
    }

    /// Send tagged commands on the authenticated connection and return the
    /// raw response bytes once the last one completes.
    fn raw(&mut self, commands: &[String]) -> Result<Vec<u8>> {
        self.retry(|s| s.raw_once(commands))
    }

    fn raw_once(&mut self, commands: &[String]) -> Result<Vec<u8>> {
        use std::io::{Read, Write};
        let imap = self.client.imap.as_mut().ok_or_else(|| anyhow!("not connected"))?;
        let stream = &mut imap.inner.stream;
        let mut wire = Vec::new();
        for (i, c) in commands.iter().enumerate() {
            wire.extend_from_slice(format!("gm{i} {c}\r\n").as_bytes());
        }
        // Failing to send means nothing ran: safe to log in again and resend.
        stream.write_all(&wire).and_then(|_| stream.flush()).map_err(|e| anyhow!("connection dropped before sending ({e})"))?;
        let last = format!("gm{} ", commands.len() - 1);
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        // The connection gives up on a read after 5 s of silence; Gmail can
        // take far longer (deleting a big label), so keep waiting.
        let deadline = std::time::Instant::now() + REPLY_WAIT;
        loop {
            let n = match stream.read(&mut buf) {
                Ok(n) => n,
                Err(e)
                    if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut)
                        && std::time::Instant::now() < deadline =>
                {
                    continue
                }
                Err(e) => return Err(anyhow!("{SENT_NOTE} ({e})")),
            };
            if n == 0 {
                return Err(anyhow!("{SENT_NOTE} (connection closed)"));
            }
            out.extend_from_slice(&buf[..n]);
            if let Some(line) = completion_line(&out, &last) {
                let status = line[last.len()..].split_whitespace().next().unwrap_or("");
                if status != "OK" {
                    return Err(anyhow!("IMAP: {}", line.trim()));
                }
                // Earlier commands in the batch must have succeeded too.
                for i in 0..commands.len() - 1 {
                    let tag = format!("gm{i} ");
                    if let Some(l) = completion_line(&out, &tag) {
                        if !l[tag.len()..].starts_with("OK") {
                            return Err(anyhow!("IMAP: {}", l.trim()));
                        }
                    }
                }
                return Ok(out);
            }
        }
    }

    /// Set or clear \\Seen on one message. On Gmail this is the read state
    /// of the conversation copy in every label.
    pub fn set_seen(&mut self, mailbox: &str, id: &str, seen: bool) -> Result<()> {
        self.retry(|s| s.set_seen_once(mailbox, id, seen))
    }

    fn set_seen_once(&mut self, mailbox: &str, id: &str, seen: bool) -> Result<()> {
        use io_email::flag::types::{Flag, FlagOp, IanaFlag};
        let op = if seen { FlagOp::Add } else { FlagOp::Remove };
        self.client
            .store_flags(mailbox, &[id], &[Flag::from_iana(IanaFlag::Seen)], op)
            .map_err(|e| anyhow!("{e}"))
    }

    /// What a reply or forward needs from one message.
    pub fn original(&mut self, mailbox: &str, id: &str) -> Result<crate::compose::Original> {
        self.retry(|s| s.original_once(mailbox, id))
    }

    fn original_once(&mut self, mailbox: &str, id: &str) -> Result<crate::compose::Original> {
        use mail_parser::{Address, HeaderValue};
        let raw = self.client.get_message(mailbox, id).map_err(|e| anyhow!("{e}"))?;
        let msg = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| anyhow!("Could not parse message"))?;
        let list = |a: Option<&Address>| -> Vec<(String, String)> {
            a.map(|a| {
                a.iter()
                    .filter_map(|x| {
                        let email = x.address()?.to_string();
                        Some((x.name().unwrap_or("").to_string(), email))
                    })
                    .collect()
            })
            .unwrap_or_default()
        };
        let references = match msg.references() {
            HeaderValue::Text(t) => vec![t.to_string()],
            HeaderValue::TextList(v) => v.iter().map(|t| t.to_string()).collect(),
            _ => Vec::new(),
        };
        let date = msg
            .date()
            .and_then(|d| chrono::DateTime::from_timestamp(d.to_timestamp(), 0))
            .map(|d| d.with_timezone(&chrono::Local).format("%a, %b %-d, %Y at %-I:%M %p").to_string())
            .unwrap_or_default();
        let text = match msg.body_text(0) {
            Some(t) if !msg.text_part(0).map(|p| p.is_text_html()).unwrap_or(true) => t.trim().to_string(),
            _ => msg.body_html(0).map(|h| html_to_text(&h)).unwrap_or_default(),
        };
        Ok(crate::compose::Original {
            from: list(msg.from()),
            reply_to: list(msg.reply_to()),
            to: list(msg.to()),
            cc: list(msg.cc()),
            bcc: list(msg.bcc()),
            message_id: msg.message_id().map(str::to_string),
            references,
            subject: msg.subject().unwrap_or("").to_string(),
            date,
            files: {
                let html = msg.html_part(0).filter(|p| p.is_text_html()).and_then(|_| msg.body_html(0)).map(|h| h.into_owned());
                attachment_parts(&msg, html.as_deref())
                    .into_iter()
                    .map(|(name, _, part)| (name, part.contents().to_vec()))
                    .collect()
            },
            text,
        })
    }

    /// Store a draft in Gmail's Drafts folder; returns its UID there.
    pub fn save_draft(&mut self, raw: Vec<u8>) -> Result<String> {
        self.retry(|s| s.save_draft_once(raw.clone()))
    }

    fn save_draft_once(&mut self, raw: Vec<u8>) -> Result<String> {
        use io_email::flag::types::{Flag, IanaFlag};
        self.client
            .add_message(DRAFTS, &[Flag::from_iana(IanaFlag::Draft), Flag::from_iana(IanaFlag::Seen)], raw)
            .map_err(|e| anyhow!("{e}"))
    }

    /// UID MOVE in batches. On Gmail, moving to All Mail is "archive"
    /// (drops the Inbox/label) and moving to Trash is "delete".
    pub fn move_ids(&mut self, from: &str, to: &str, ids: &[String]) -> Result<()> {
        self.retry(|s| s.move_ids_once(from, to, ids))
    }

    /// UID MOVE in batches, on our own command path: it waits as long as
    /// Gmail needs (moving hundreds takes more than the 5 s the library's
    /// own move allows). Moving UIDs that already left is a no-op.
    fn move_ids_once(&mut self, from: &str, to: &str, ids: &[String]) -> Result<()> {
        for chunk in ids.chunks(MOVE_BATCH) {
            self.raw(&[format!("SELECT {}", imap_quote(from)), format!("UID MOVE {} {}", chunk.join(","), imap_quote(to))])?;
        }
        Ok(())
    }

    /// UID of the message with `message_id` in `mailbox`, if it's there.
    pub fn find_by_message_id(&mut self, mailbox: &str, message_id: &str) -> Result<Option<String>> {
        let bytes = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID SEARCH HEADER Message-ID {}", imap_quote(message_id)),
        ])?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(text
            .lines()
            .filter_map(|l| l.strip_prefix("* SEARCH"))
            .flat_map(|r| r.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .last())
    }

    /// `UID STORE <ids> <op>` in `mailbox`, e.g. `+FLAGS.SILENT (\Flagged)`
    /// or `-X-GM-LABELS.SILENT (\Important)`. Gmail labels are per message,
    /// so this works from whichever folder the copy is listed in.
    pub fn store_op(&mut self, mailbox: &str, ids: &[String], op: &str) -> Result<()> {
        for chunk in ids.chunks(500) {
            self.raw(&[
                format!("SELECT {}", imap_quote(mailbox)),
                format!("UID STORE {} {op}", chunk.join(",")),
            ])?;
        }
        Ok(())
    }

    /// UID COPY in batches. On Gmail, copying into a label folder adds the
    /// label (the message isn't duplicated).
    pub fn copy_ids(&mut self, from: &str, to: &str, ids: &[String]) -> Result<()> {
        self.retry(|s| s.copy_ids_once(from, to, ids))
    }

    fn copy_ids_once(&mut self, from: &str, to: &str, ids: &[String]) -> Result<()> {
        for chunk in ids.chunks(MOVE_BATCH) {
            self.raw(&[format!("SELECT {}", imap_quote(from)), format!("UID COPY {} {}", chunk.join(","), imap_quote(to))])?;
        }
        Ok(())
    }

    /// Gmail labels on messages in `mailbox`: uid → labels (Gmail's own
    /// ones like \\Inbox left out).
    pub fn labels_of(&mut self, mailbox: &str, ids: &[String]) -> Result<std::collections::HashMap<String, Vec<String>>> {
        let mut out = std::collections::HashMap::new();
        if ids.is_empty() {
            return Ok(out);
        }
        let bytes = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID FETCH {} (UID X-GM-LABELS)", ids.join(",")),
        ])?;
        for line in String::from_utf8_lossy(&bytes).lines().filter(|l| l.starts_with("* ")) {
            let Some(at) = line.find("UID ") else { continue };
            let uid: String = line[at + 4..].chars().take_while(|c| c.is_ascii_digit()).collect();
            out.insert(uid, parse_gm_labels(line).into_iter().filter(|l| !l.starts_with('\\')).collect());
        }
        Ok(out)
    }

    /// Whether a label exists right now (asks the server).
    fn has_label(&mut self, name: &str) -> Result<bool> {
        Ok(self.mailboxes()?.iter().any(|m| m == name))
    }

    /// These settle on the result rather than the reply: if the command
    /// failed or the connection broke, the label list says whether the
    /// change happened (perhaps from an earlier try).
    pub fn create_label(&mut self, name: &str) -> Result<()> {
        match self.raw(&[format!("CREATE {}", imap_quote(name))]) {
            Ok(_) => Ok(()),
            Err(e) => if self.has_label(name).unwrap_or(false) { Ok(()) } else { Err(e) },
        }
    }

    /// On Gmail renaming a label renames the labels nested under it too.
    pub fn rename_label(&mut self, from: &str, to: &str) -> Result<()> {
        match self.raw(&[format!("RENAME {} {}", imap_quote(from), imap_quote(to))]) {
            Ok(_) => Ok(()),
            Err(e) => {
                let done = self.has_label(to).unwrap_or(false) && !self.has_label(from).unwrap_or(true);
                if done { Ok(()) } else { Err(e) }
            }
        }
    }

    /// On Gmail this removes the label; the messages stay in All Mail.
    pub fn delete_label(&mut self, name: &str) -> Result<()> {
        match self.raw(&[format!("DELETE {}", imap_quote(name))]) {
            Ok(_) => Ok(()),
            Err(e) => if self.has_label(name).unwrap_or(true) { Err(e) } else { Ok(()) },
        }
    }

    /// Add (or remove) a Gmail label on messages in `mailbox`.
    pub fn set_label(&mut self, mailbox: &str, ids: &[String], label: &str, on: bool) -> Result<()> {
        let op = format!("{}X-GM-LABELS ({})", if on { "+" } else { "-" }, imap_quote(label));
        self.store_op(mailbox, ids, &op)
    }

    /// Delete messages forever. Only in Trash or Spam: anywhere else on
    /// Gmail it would just remove a label, and it's never what's meant.
    pub fn purge(&mut self, mailbox: &str, ids: &[String]) -> Result<()> {
        ensure_trash_or_spam(mailbox)?;
        for chunk in ids.chunks(500) {
            let set = chunk.join(",");
            self.raw(&[
                format!("SELECT {}", imap_quote(mailbox)),
                format!("UID STORE {set} +FLAGS.SILENT (\\Deleted)"),
                format!("UID EXPUNGE {set}"),
            ])?;
        }
        Ok(())
    }

    /// Empty Trash or Spam on the server (everything, not just what's
    /// loaded). Returns how many messages were deleted.
    pub fn empty_folder(&mut self, mailbox: &str) -> Result<u64> {
        ensure_trash_or_spam(mailbox)?;
        let n = self.mailbox_status(mailbox)?.exists;
        if n > 0 {
            self.raw(&[
                format!("SELECT {}", imap_quote(mailbox)),
                "STORE 1:* +FLAGS.SILENT (\\Deleted)".to_string(),
                "EXPUNGE".to_string(),
            ])?;
        }
        Ok(n)
    }

    /// Every UID in a mailbox (read-only).
    pub fn all_uids(&mut self, mailbox: &str) -> Result<Vec<String>> {
        let bytes = self.raw(&[format!("EXAMINE {}", imap_quote(mailbox)), "UID SEARCH ALL".to_string()])?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(text
            .lines()
            .filter_map(|l| l.strip_prefix("* SEARCH"))
            .flat_map(|rest| rest.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .collect())
    }

    /// Move everything in `mailbox` to Trash, in batches. Returns how many.
    pub fn trash_all(&mut self, mailbox: &str) -> Result<usize> {
        if is_trash_or_spam(mailbox) || mailbox.eq_ignore_ascii_case(INBOX) || mailbox.starts_with("[Gmail]/") {
            return Err(anyhow!("Not moving all of {mailbox} to Trash"));
        }
        // Gmail sometimes answers a bulk MOVE with "NO System Error" part way
        // through. Each round re-reads what's still here and carries on, so a
        // hiccup costs a pause rather than the whole job.
        let mut moved = 0;
        let mut failures = 0;
        loop {
            let uids = self.all_uids(mailbox)?;
            if uids.is_empty() {
                return Ok(moved);
            }
            let mut progressed = false;
            let mut last_err = None;
            for chunk in uids.chunks(MOVE_BATCH) {
                match self.move_ids(mailbox, TRASH, chunk) {
                    Ok(()) => {
                        moved += chunk.len();
                        progressed = true;
                    }
                    Err(e) => {
                        last_err = Some(e);
                        break;
                    }
                }
            }
            match last_err {
                None => continue,
                Some(e) => {
                    failures = if progressed { 1 } else { failures + 1 };
                    if failures >= 4 {
                        return Err(e.context(format!("moved {moved}, then Gmail kept refusing")));
                    }
                    std::thread::sleep(std::time::Duration::from_secs(3 * failures as u64));
                }
            }
        }
    }

    /// Create a label if it doesn't exist yet.
    pub fn ensure_mailbox(&mut self, name: &str) -> Result<()> {
        self.retry(|s| s.ensure_mailbox_once(name))
    }

    fn ensure_mailbox_once(&mut self, name: &str) -> Result<()> {
        let exists = self.mailboxes()?.iter().any(|m| m.eq_ignore_ascii_case(name));
        if !exists {
            self.client.create_mailbox(name).map_err(|e| anyhow!("{e}"))?;
        }
        Ok(())
    }

    /// UIDs in `mailbox` of messages from `email` (read-only search).
    pub fn uids_from(&mut self, mailbox: &str, email: &str) -> Result<Vec<String>> {
        let bytes = self.raw(&[
            format!("EXAMINE {}", imap_quote(mailbox)),
            format!("UID SEARCH FROM {}", imap_quote(email)),
        ])?;
        let text = String::from_utf8_lossy(&bytes);
        Ok(text
            .lines()
            .filter_map(|l| l.strip_prefix("* SEARCH"))
            .flat_map(|r| r.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .collect())
    }

    pub fn delete_mailbox(&mut self, name: &str) -> Result<()> {
        self.retry(|s| s.delete_mailbox_once(name))
    }

    fn delete_mailbox_once(&mut self, name: &str) -> Result<()> {
        self.client.delete_mailbox(name).map_err(|e| anyhow!("{e}"))
    }

    pub fn delete_message(&mut self, mailbox: &str, id: &str) -> Result<()> {
        self.retry(|s| s.delete_message_once(mailbox, id))
    }

    fn delete_message_once(&mut self, mailbox: &str, id: &str) -> Result<()> {
        self.client.delete_message(mailbox, id).map_err(|e| anyhow!("{e}"))
    }

    /// Body of one message: readable text (always) and the original HTML
    /// part when the message has one.
    pub fn body(&mut self, mailbox: &str, id: &str) -> Result<(String, Option<String>, Vec<crate::model::Attachment>)> {
        self.retry(|s| s.body_once(mailbox, id))
    }

    fn body_once(&mut self, mailbox: &str, id: &str) -> Result<(String, Option<String>, Vec<crate::model::Attachment>)> {
        let raw = self.client.get_message(mailbox, id).map_err(|e| anyhow!("{e}"))?;
        let msg = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| anyhow!("Could not parse message"))?;
        // mail-parser converts between text and HTML on demand; only keep
        // HTML when the message really has an HTML part.
        let html = msg
            .html_part(0)
            .filter(|p| p.is_text_html())
            .and_then(|_| msg.body_html(0))
            .map(|h| h.into_owned());
        let has_text_part = msg.text_part(0).map(|p| !p.is_text_html()).unwrap_or(false);
        let text = match (&html, has_text_part) {
            (_, true) => msg.body_text(0).map(|t| t.trim().to_string()).unwrap_or_default(),
            (Some(h), false) => html_to_text(h),
            (None, false) => "(no text body)".to_string(),
        };
        let files = attachment_parts(&msg, html.as_deref())
            .into_iter()
            .map(|(name, mime, part)| crate::model::Attachment { name, mime, size: part.contents().len() as u64 })
            .collect();
        let html = with_images(&msg, html, &text);
        Ok((text, html, files))
    }

    /// The bytes of attachment `index` (as listed by `body`).
    pub fn attachment(&mut self, mailbox: &str, id: &str, index: usize) -> Result<(String, Vec<u8>)> {
        self.retry(|s| s.attachment_once(mailbox, id, index))
    }

    fn attachment_once(&mut self, mailbox: &str, id: &str, index: usize) -> Result<(String, Vec<u8>)> {
        let raw = self.client.get_message(mailbox, id).map_err(|e| anyhow!("{e}"))?;
        let msg = mail_parser::MessageParser::default()
            .parse(&raw)
            .ok_or_else(|| anyhow!("Could not parse message"))?;
        let html = msg.html_part(0).filter(|p| p.is_text_html()).and_then(|_| msg.body_html(0)).map(|h| h.into_owned());
        let parts = attachment_parts(&msg, html.as_deref());
        let (name, _, part) = parts.into_iter().nth(index).ok_or_else(|| anyhow!("That attachment is gone"))?;
        Ok((name, part.contents().to_vec()))
    }
}

/// Send one message through Gmail's SMTP server (it files the copy in
/// Sent Mail itself) to exactly `recipients`, which may include addresses
/// not in the headers (Bcc). A fresh connection per send.
pub fn send_raw(account: &str, raw: Vec<u8>, recipients: &[String]) -> Result<()> {
    use io_smtp::rfc5321::types::{
        domain::Domain, ehlo_domain::EhloDomain, forward_path::ForwardPath, local_part::LocalPart, mailbox::Mailbox,
        reverse_path::ReversePath,
    };
    use std::borrow::Cow;
    let mailbox = |addr: &str| -> Result<Mailbox<'static>> {
        let (local, domain) = addr.rsplit_once('@').filter(|(l, d)| !l.is_empty() && !d.is_empty()).ok_or_else(|| anyhow!("Not an address: {addr}"))?;
        Ok(Mailbox {
            local_part: LocalPart(Cow::Owned(local.to_string())),
            domain: EhloDomain::Domain(Domain(Cow::Owned(domain.to_string()))),
        })
    };
    if recipients.is_empty() {
        return Err(anyhow!("No recipients"));
    }
    let forwards = recipients.iter().map(|r| mailbox(r).map(ForwardPath::from)).collect::<Result<Vec<_>>>()?;
    let reverse = ReversePath::Mailbox(mailbox(account)?);
    let url = Url::parse(crate::accounts::smtp_url(account))?;
    let sasl = crate::accounts::sasl(account)?;
    let ehlo = io_smtp::rfc5321::types::ehlo_domain::EhloDomain::from(std::net::Ipv4Addr::LOCALHOST);
    let mut client = EmailClientStd::new()
        .connect_smtp(&url, &Tls::default(), false, ehlo, Some(sasl))
        .map_err(|e| anyhow!("SMTP login failed: {e}"))?;
    let smtp = client.smtp.as_mut().ok_or_else(|| anyhow!("SMTP not connected"))?;
    smtp.inner.send(reverse, forwards, raw).map_err(|e| anyhow!("Send failed: {e}"))
}

/// Log in to the SMTP server and disconnect without sending anything.
pub fn smtp_check(account: &str) -> Result<()> {
    let url = Url::parse(crate::accounts::smtp_url(account))?;
    let sasl = crate::accounts::sasl(account)?;
    let ehlo = io_smtp::rfc5321::types::ehlo_domain::EhloDomain::from(std::net::Ipv4Addr::LOCALHOST);
    EmailClientStd::new()
        .connect_smtp(&url, &Tls::default(), false, ehlo, Some(sasl))
        .map(|_| ())
        .map_err(|e| anyhow!("SMTP login failed: {e}"))
}

/// Largest image embedded into the page (bigger ones are skipped).
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// Embed the message's own images as data: URIs, so they display without
/// any network access (the view's CSP allows data: images only):
/// - images referenced from the HTML as `cid:` are swapped in place;
/// - other images (photos sent as attachments) go in a gallery at the end.
/// Text-only messages with images get a small HTML page built around the
/// text so the gallery can show.
/// The files a person would call attachments: every attached part except
/// images the HTML shows in place (logos, inline pictures). In order, with
/// a name and a type.
pub fn attachment_parts<'a>(
    msg: &'a mail_parser::Message<'a>,
    html: Option<&str>,
) -> Vec<(String, String, &'a mail_parser::MessagePart<'a>)> {
    use mail_parser::MimeHeaders as _;
    let mut out = Vec::new();
    for (n, part) in msg.attachments().enumerate() {
        let ct = part.content_type();
        let mime = ct
            .map(|c| format!("{}/{}", c.c_type, c.c_subtype.as_deref().unwrap_or("octet-stream")).to_ascii_lowercase())
            .unwrap_or_else(|| "application/octet-stream".into());
        let cid = part.content_id().map(|c| c.trim().trim_matches(|c| c == '<' || c == '>').to_string());
        let shown_inline = match (html, &cid) {
            (Some(h), Some(c)) if !c.is_empty() => h.contains(&format!("cid:{c}")),
            _ => false,
        };
        if shown_inline || part.contents().is_empty() {
            continue;
        }
        let name = part.attachment_name().map(str::to_string).unwrap_or_else(|| {
            if mime == "message/rfc822" {
                "Forwarded message.eml".to_string()
            } else {
                let ext = mime.rsplit('/').next().unwrap_or("bin");
                format!("attachment-{}.{ext}", n + 1)
            }
        });
        out.push((name, mime, part));
    }
    out
}

fn with_images(msg: &mail_parser::Message, html: Option<String>, text: &str) -> Option<String> {
    use base64::Engine as _;
    use mail_parser::MimeHeaders as _;
    let mut html = html;
    let mut gallery: Vec<(String, String)> = Vec::new();
    for part in &msg.parts {
        let Some(ct) = part.content_type() else { continue };
        if !ct.c_type.eq_ignore_ascii_case("image") {
            continue;
        }
        let bytes = part.contents();
        if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
            continue;
        }
        let subtype = ct.c_subtype.as_deref().unwrap_or("png").to_ascii_lowercase();
        let uri = format!(
            "data:image/{subtype};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        let cid = part.content_id().map(|c| c.trim().trim_matches(|c| c == '<' || c == '>').to_string());
        let placed = match (&mut html, cid) {
            (Some(h), Some(cid)) if !cid.is_empty() => {
                let needle = format!("cid:{cid}");
                let found = h.contains(&needle);
                if found {
                    *h = h.replace(&needle, &uri);
                }
                found
            }
            _ => false,
        };
        if !placed {
            let name = part.attachment_name().unwrap_or("image").to_string();
            gallery.push((name, uri));
        }
    }
    if gallery.is_empty() {
        return html;
    }
    let mut figures = String::from(
        "<div style=\"margin-top:24px;border-top:1px solid #ddd;padding-top:12px;\
         font:12px -apple-system,Helvetica,sans-serif;color:#666\">",
    );
    for (name, uri) in &gallery {
        figures.push_str(&format!(
            "<figure style=\"margin:0 0 16px\"><img src=\"{uri}\" style=\"max-width:100%;border-radius:6px\">\
             <figcaption>{}</figcaption></figure>",
            escape_html(name)
        ));
    }
    figures.push_str("</div>");
    Some(match html {
        Some(mut h) => match h.to_ascii_lowercase().rfind("</body>") {
            Some(at) => {
                h.insert_str(at, &figures);
                h
            }
            None => h + &figures,
        },
        None => format!(
            "<div style=\"white-space:pre-wrap;font:14px -apple-system,Helvetica,sans-serif\">{}</div>{figures}",
            escape_html(text)
        ),
    })
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// IMAP quoted string.
/// The list in `X-GM-LABELS (...)`: quoted strings and bare atoms.
pub fn parse_gm_labels(line: &str) -> Vec<String> {
    let Some(at) = line.find("X-GM-LABELS (") else { return Vec::new() };
    let mut out = Vec::new();
    let mut chars = line[at + "X-GM-LABELS (".len()..].chars().peekable();
    loop {
        while chars.peek() == Some(&' ') {
            chars.next();
        }
        match chars.peek() {
            None | Some(')') => break,
            Some('"') => {
                chars.next();
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '\\' => {
                            if let Some(n) = chars.next() {
                                s.push(n)
                            }
                        }
                        '"' => break,
                        c => s.push(c),
                    }
                }
                out.push(s);
            }
            _ => {
                let mut s = String::new();
                while let Some(&c) = chars.peek() {
                    if c == ' ' || c == ')' {
                        break;
                    }
                    s.push(c);
                    chars.next();
                }
                out.push(s);
            }
        }
    }
    out
}

/// Messages per MOVE/COPY command.
const MOVE_BATCH: usize = 200;

/// How long to wait for the server to answer a command.
const REPLY_WAIT: std::time::Duration = std::time::Duration::from_secs(120);

/// The connection broke after a command went out: it may have run, so it is
/// never sent again automatically.
const SENT_NOTE: &str = "the connection broke while Gmail was working on it; it may have gone through";

/// Errors that mean the connection was already gone when we tried to use it
/// (so nothing ran and it's safe to log in again and repeat), rather than
/// that the server said no or went quiet mid-command.
pub fn is_dropped(e: &anyhow::Error) -> bool {
    let text = format!("{e:#}").to_lowercase();
    if text.contains("may have gone through") {
        return false;
    }
    ["broken pipe", "os error 32", "connection reset", "os error 54", "not connected", "dropped before sending", "* bye"]
        .iter()
        .any(|k| text.contains(k))
}

/// The only folders permanent deletion is allowed in.
pub fn is_trash_or_spam(mailbox: &str) -> bool {
    mailbox.eq_ignore_ascii_case(TRASH) || mailbox.eq_ignore_ascii_case(SPAM)
}

fn ensure_trash_or_spam(mailbox: &str) -> Result<()> {
    if is_trash_or_spam(mailbox) {
        Ok(())
    } else {
        Err(anyhow!("Refusing to delete forever outside Trash and Spam ({mailbox})"))
    }
}

pub fn imap_quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The tagged completion line for `tag` ("gm1 OK ..."), if received.
fn completion_line(bytes: &[u8], tag: &str) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    text.split("\r\n")
        .find(|l| l.starts_with(tag))
        .filter(|_| text.ends_with("\r\n"))
        .map(str::to_string)
}

/// Parse "* n FETCH (UID u FLAGS (...) BODY[HEADER.FIELDS (...)] {len}\r\n<headers>)"
/// responses into envelopes.
fn parse_header_fetch(mailbox: &str, bytes: &[u8]) -> Vec<Envelope> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(start) = find(bytes, b"* ", pos) {
        let Some(fetch) = find(bytes, b" FETCH (", start) else { break };
        // Only treat it as ours if FETCH is on the same line.
        if bytes[start..fetch].contains(&b'\n') {
            pos = start + 2;
            continue;
        }
        let Some(open) = find(bytes, b"{", fetch) else { break };
        let Some(close) = find(bytes, b"}\r\n", open) else { break };
        let len: usize = String::from_utf8_lossy(&bytes[open + 1..close]).parse().unwrap_or(0);
        let lit_start = close + 3;
        let lit_end = (lit_start + len).min(bytes.len());
        let tail_end = find(bytes, b"\r\n", lit_end).unwrap_or(bytes.len());
        let meta = format!(
            "{} {}",
            String::from_utf8_lossy(&bytes[fetch..open]),
            String::from_utf8_lossy(&bytes[lit_end..tail_end])
        );
        pos = tail_end;
        let uid = meta
            .split_whitespace()
            .skip_while(|w| w.trim_start_matches('(') != "UID")
            .nth(1)
            .map(|u| u.trim_matches(|c: char| !c.is_ascii_digit()).to_string());
        let Some(uid) = uid.filter(|u| !u.is_empty()) else { continue };
        let flags = meta.find("FLAGS (").map(|i| meta[i..].split(')').next().unwrap_or("").to_string()).unwrap_or_default();
        let unread = meta.contains("FLAGS (") && !flags.contains("\\Seen");
        let starred = flags.contains("\\Flagged");
        let important = meta.contains("\\Important") || mailbox.eq_ignore_ascii_case(IMPORTANT);
        let Some(msg) = mail_parser::MessageParser::default().parse(&bytes[lit_start..lit_end]) else {
            continue;
        };
        let sender = msg.from().and_then(|a| a.first());
        let from_email = sender.and_then(|a| a.address()).unwrap_or_default().to_string();
        let from = sender
            .and_then(|a| a.name())
            .filter(|n| !n.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| if from_email.is_empty() { "(unknown sender)".into() } else { from_email.clone() });
        let subject = msg.subject().filter(|s| !s.trim().is_empty()).unwrap_or("(no subject)").to_string();
        let ts = msg.date().map(|d| d.to_timestamp()).unwrap_or(0);
        out.push(Envelope {
            mailbox: mailbox.to_string(),
            id: uid,
            message_id: msg.message_id().map(str::to_string),
            from: from.into(),
            from_email: from_email.into(),
            subject: subject.into(),
            ts,
            date: crate::model::display_date(ts),
            preview: "".into(),
            body: None,
            html: None,
            unread,
            starred,
            important,
            attachments: Vec::new(),
            account: Default::default(),
        });
    }
    out
}

/// The literal payloads ({n}\r\n...) of a FETCH response, in order.
fn fetch_literals(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut pos = 0;
    while let Some(open) = find(bytes, b"{", pos) {
        let Some(close) = find(bytes, b"}\r\n", open) else { break };
        let Ok(len) = String::from_utf8_lossy(&bytes[open + 1..close]).parse::<usize>() else {
            pos = open + 1;
            continue;
        };
        let start = close + 3;
        let end = (start + len).min(bytes.len());
        out.push(&bytes[start..end]);
        pos = end;
    }
    out
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|i| i + from)
}

fn to_model(mailbox: &str, e: io_email::envelope::types::Envelope) -> Envelope {
    let sender = e.from.first();
    let from = sender
        .map(|a| a.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or(a.email.clone()))
        .unwrap_or_else(|| "(unknown sender)".into());
    let from_email = sender.map(|a| a.email.clone()).unwrap_or_default();
    let ts = e.date.map(|d| d.timestamp()).unwrap_or(0);
    let unread = !e.flags.iter().any(|f| f.is_seen());
    let starred = e.flags.iter().any(|f| f.is_flagged()) || mailbox.eq_ignore_ascii_case(STARRED);
    let important = e.flags.iter().any(|f| f.is_important()) || mailbox.eq_ignore_ascii_case(IMPORTANT);
    let subject = if e.subject.trim().is_empty() { "(no subject)".to_string() } else { e.subject };
    Envelope {
        mailbox: mailbox.to_string(),
        id: e.id,
        message_id: e.message_id,
        from: from.into(),
        from_email: from_email.into(),
        subject: subject.into(),
        ts,
        date: crate::model::display_date(ts),
        preview: "".into(),
        body: None,
        html: None,
        unread,
        starred,
        important,
        attachments: Vec::new(),
        account: Default::default(),
    }
}

/// HTML-to-text until the real renderer lands: keeps paragraphs, lists,
/// tables and links (as [n] footnotes), drops styles and scripts.
fn html_to_text(html: &str) -> String {
    html2text::from_read(html.as_bytes(), 100)
        .unwrap_or_else(|_| "(could not render HTML)".into())
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Headless login test: `gpui-mail --check`.
pub fn check() -> Result<()> {
    let started = std::time::Instant::now();
    let account = crate::accounts::first_email();
    let mut session = Session::connect(&account)?;
    println!("Logged in as {account} in {:?}", started.elapsed());
    let boxes = session.mailboxes()?;
    println!("{} mailboxes: {}", boxes.len(), boxes.join(", "));
    let inbox = boxes.first().cloned().unwrap_or_else(|| "INBOX".into());
    let list = session.envelopes(&inbox, 5)?;
    for e in &list {
        println!("  {} | {} | {}", e.date, e.from, e.subject);
    }
    if let Some(first) = list.first() {
        let (body, html, _) = session.body(&inbox, &first.id)?;
        println!(
            "First body: {} chars text, {} chars HTML",
            body.chars().count(),
            html.map(|h| h.len()).unwrap_or(0)
        );
    }
    println!("OK in {:?}", started.elapsed());
    Ok(())
}

#[cfg(test)]
mod label_tests {
    use super::{is_dropped, parse_gm_labels};

    #[test]
    fn connection_slots_are_capped_and_returned() {
        use super::{open_connections, Slot, MAX_CONNECTIONS};
        let acct = "cap-test@example.com";
        let held: Vec<Slot> = (0..MAX_CONNECTIONS).map(|_| Slot::take(acct).unwrap()).collect();
        assert_eq!(open_connections(acct), MAX_CONNECTIONS);
        assert!(Slot::take(acct).is_err());
        drop(held);
        assert_eq!(open_connections(acct), 0);
        assert!(Slot::take(acct).is_ok());
    }

    #[test]
    fn deletes_forever_only_in_trash_or_spam() {
        use super::{ensure_trash_or_spam, is_trash_or_spam};
        assert!(is_trash_or_spam("[Gmail]/Trash") && is_trash_or_spam("[Gmail]/Spam"));
        assert!(ensure_trash_or_spam("INBOX").is_err());
        assert!(ensure_trash_or_spam("[Gmail]/All Mail").is_err());
        assert!(ensure_trash_or_spam("Trash").is_err()); // a label called Trash isn't Gmail's Trash
    }

    #[test]
    fn knows_a_dropped_connection() {
        assert!(is_dropped(&anyhow::anyhow!("Broken pipe (os error 32)")));
        assert!(is_dropped(&anyhow::anyhow!("connection dropped before sending (Broken pipe (os error 32))")));
        // Sent, then silence or a break: never repeated.
        assert!(!is_dropped(&anyhow::anyhow!("the connection broke while Gmail was working on it; it may have gone through (Broken pipe)")));
        assert!(!is_dropped(&anyhow::anyhow!("Resource temporarily unavailable (os error 35)")));
        assert!(!is_dropped(&anyhow::anyhow!("IMAP: gm1 NO [ALREADYEXISTS] Duplicate folder name")));
    }

    #[test]
    fn reads_gmail_labels() {
        let l = parse_gm_labels(r#"* 3 FETCH (X-GM-LABELS ("\\Important" "\\Inbox" Work "[Notion]" "Clients/Acme \"A\"") UID 812)"#);
        assert_eq!(l, vec!["\\Important", "\\Inbox", "Work", "[Notion]", "Clients/Acme \"A\""]);
        assert!(parse_gm_labels("* 3 FETCH (X-GM-LABELS () UID 1)").is_empty());
    }
}
