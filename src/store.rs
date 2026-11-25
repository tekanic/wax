//! Local cache: SQLite at ~/Library/Application Support/gpui-mail/cache.sqlite.
//!
//! Holds mailbox names, envelopes, previews and every body fetched so far,
//! so the app opens instantly and works offline. The server stays the source
//! of truth: each refresh upserts what it returns and drops cached rows that
//! fell out of the refreshed window.

use crate::model::{display_date, make_preview, Envelope};
use anyhow::Result;
use rusqlite::{params, Connection, Row};
use std::collections::HashSet;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct SenderStat {
    pub email: String,
    pub name: String,
    pub count: u64,
    pub unread: u64,
    pub last_ts: i64,
    /// Existing decision: "keep" or a label name.
    pub rule: Option<String>,
}

pub struct Store {
    conn: Connection,
}

pub fn default_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Library/Application Support/gpui-mail/cache.sqlite")
}

const ENVELOPE_COLUMNS: &str =
    "e.mailbox, e.id, e.message_id, e.from_name, e.from_email, e.subject, e.ts, e.unread, e.preview, b.body, b.html, e.flags, b.files, e.account, e.to_list, e.thread_id";

/// Recipients in one column: name and address split by \x1f, people by \x1e.
pub fn encode_people(people: &[(String, String)]) -> String {
    people.iter().map(|(n, e)| format!("{}\x1f{e}", n.replace(['\x1e', '\x1f'], " "))).collect::<Vec<_>>().join("\x1e")
}

pub fn decode_people(s: &str) -> Vec<(String, String)> {
    s.split('\x1e').filter(|p| !p.is_empty()).filter_map(|p| p.split_once('\x1f')).map(|(n, e)| (n.to_string(), e.to_string())).collect()
}

fn flag_bits(e: &Envelope) -> i64 {
    (e.starred as i64) | ((e.important as i64) << 1)
}

fn row_to_envelope(row: &Row) -> rusqlite::Result<Envelope> {
    let ts: i64 = row.get(6)?;
    let body: Option<String> = row.get(9)?;
    let html: Option<String> = row.get(10)?;
    Ok(Envelope {
        mailbox: row.get(0)?,
        id: row.get(1)?,
        message_id: row.get(2)?,
        from: row.get::<_, String>(3)?.into(),
        from_email: row.get::<_, String>(4)?.into(),
        subject: row.get::<_, String>(5)?.into(),
        ts,
        date: display_date(ts),
        unread: row.get(7)?,
        starred: row.get::<_, Option<i64>>(11)?.unwrap_or(0) & 1 != 0,
        important: row.get::<_, Option<i64>>(11)?.unwrap_or(0) & 2 != 0,
        attachments: row.get::<_, Option<String>>(12)?.map(|f| crate::model::Attachment::decode(&f)).unwrap_or_default(),
        account: row.get::<_, String>(13)?.into(),
        to: row.get::<_, Option<String>>(14)?.map(|s| decode_people(&s)).unwrap_or_default(),
        thread: row.get(15)?,
        labels: Vec::new(),
        // Recompute from the body when cached, so preview fixes apply to old rows.
        preview: match &body {
            Some(b) => make_preview(b).into(),
            None => row.get::<_, String>(8)?.into(),
        },
        body: body.map(Into::into),
        html: html.map(Into::into),
    })
}

impl Store {
    pub fn open() -> Result<Self> {
        Self::open_at(&default_path())
    }

    /// A cache at `path` (tests use a scratch file).
    pub fn open_at(path: &std::path::Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS mailboxes (
                 account TEXT NOT NULL, name TEXT NOT NULL,
                 PRIMARY KEY (account, name));
             CREATE TABLE IF NOT EXISTS envelopes (
                 account TEXT NOT NULL, mailbox TEXT NOT NULL, id TEXT NOT NULL,
                 message_id TEXT, from_name TEXT NOT NULL, from_email TEXT NOT NULL,
                 subject TEXT NOT NULL, ts INTEGER NOT NULL, unread INTEGER NOT NULL,
                 preview TEXT NOT NULL DEFAULT '',
                 PRIMARY KEY (account, mailbox, id));
             CREATE INDEX IF NOT EXISTS envelopes_by_date ON envelopes (account, mailbox, ts DESC);
             CREATE TABLE IF NOT EXISTS bodies (
                 account TEXT NOT NULL, mailbox TEXT NOT NULL, id TEXT NOT NULL,
                 body TEXT NOT NULL,
                 PRIMARY KEY (account, mailbox, id));
             CREATE TABLE IF NOT EXISTS compose_snapshot (
                 slot INTEGER PRIMARY KEY CHECK (slot = 1),
                 kind TEXT NOT NULL, to_list TEXT NOT NULL, cc_list TEXT NOT NULL,
                 subject TEXT NOT NULL, body TEXT NOT NULL,
                 in_reply_to TEXT, refs TEXT NOT NULL,
                 server_mailbox TEXT, server_id TEXT);
             CREATE TABLE IF NOT EXISTS sent_contacts (
                 account TEXT NOT NULL, email TEXT NOT NULL, name TEXT NOT NULL,
                 times INTEGER NOT NULL, last_ts INTEGER NOT NULL,
                 PRIMARY KEY (account, email));
             CREATE TABLE IF NOT EXISTS sender_rules (
                 account TEXT NOT NULL, email TEXT NOT NULL,
                 dest TEXT NOT NULL,        -- 'keep' or a label name
                 created INTEGER NOT NULL,
                 PRIMARY KEY (account, email));
             CREATE TABLE IF NOT EXISTS settings (
                 key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS signatures (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 account TEXT NOT NULL, label TEXT NOT NULL, layout TEXT NOT NULL,
                 name TEXT NOT NULL, title TEXT NOT NULL, company TEXT NOT NULL,
                 phone TEXT NOT NULL, email TEXT NOT NULL, website TEXT NOT NULL,
                 note TEXT NOT NULL, accent INTEGER NOT NULL,
                 logo_mime TEXT, logo BLOB, logo_width INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS sync_state (
                 account TEXT NOT NULL, mailbox TEXT NOT NULL,
                 uid_validity INTEGER NOT NULL,
                 low_uid INTEGER NOT NULL,   -- everything >= low_uid is synced
                 high_uid INTEGER NOT NULL,  -- ... up to and including high_uid
                 PRIMARY KEY (account, mailbox));",
        )?;
        // v2: bodies keep the HTML part. Bodies cached before that are
        // dropped so they get refetched with their HTML.
        if conn.execute("ALTER TABLE bodies ADD COLUMN html TEXT", []).is_ok() {
            conn.execute("DELETE FROM bodies", [])?;
        }
        // v6: signature links, second logo, follow line, font.
        for col in [
            "logo_link TEXT NOT NULL DEFAULT ''",
            "logo2_mime TEXT",
            "logo2 BLOB",
            "logo2_width INTEGER NOT NULL DEFAULT 142",
            "logo2_link TEXT NOT NULL DEFAULT ''",
            "social_label TEXT NOT NULL DEFAULT ''",
            "social_url TEXT NOT NULL DEFAULT ''",
            "font TEXT NOT NULL DEFAULT ''",
        ] {
            conn.execute(&format!("ALTER TABLE signatures ADD COLUMN {col}"), []).ok();
        }
        // v5: the composer's font and signature survive a restart.
        conn.execute("ALTER TABLE compose_snapshot ADD COLUMN mono INTEGER", []).ok();
        conn.execute("ALTER TABLE compose_snapshot ADD COLUMN signature INTEGER", []).ok();
        conn.execute("ALTER TABLE compose_snapshot ADD COLUMN from_name TEXT", []).ok();
        conn.execute("ALTER TABLE compose_snapshot ADD COLUMN from_email TEXT", []).ok();
        conn.execute("ALTER TABLE compose_snapshot ADD COLUMN attachments TEXT", []).ok();
        conn.execute("ALTER TABLE compose_snapshot ADD COLUMN bcc_list TEXT", []).ok();
        conn.execute("ALTER TABLE compose_snapshot ADD COLUMN account TEXT", []).ok();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS reminders (
                 id INTEGER PRIMARY KEY, account TEXT NOT NULL, kind TEXT NOT NULL,
                 message_ids TEXT NOT NULL, thread TEXT, subject TEXT NOT NULL DEFAULT '',
                 created INTEGER NOT NULL, due INTEGER NOT NULL);",
        )?;
        // Older versions cached "Could not load message" as a body; fetch those again.
        conn.execute("DELETE FROM bodies WHERE body LIKE 'Could not load message:%'", []).ok();
        // v8: recipients (Sent shows and searches them) and Gmail's thread id.
        conn.execute("ALTER TABLE envelopes ADD COLUMN to_list TEXT NOT NULL DEFAULT ''", []).ok();
        conn.execute("ALTER TABLE envelopes ADD COLUMN thread_id TEXT", []).ok();
        conn.execute("CREATE INDEX IF NOT EXISTS envelopes_by_thread ON envelopes (account, thread_id)", []).ok();
        // v4: star and Important, as bits (1 = starred, 2 = important).
        conn.execute("ALTER TABLE envelopes ADD COLUMN flags INTEGER NOT NULL DEFAULT 0", []).ok();
        // v7: bodies list their attachments. Refetch older ones so they do.
        if conn.execute("ALTER TABLE bodies ADD COLUMN files TEXT", []).is_ok() {
            conn.execute("DELETE FROM bodies", [])?;
        }
        // v3: bodies embed the message's own images. Refetch older ones.
        if conn.execute("ALTER TABLE bodies ADD COLUMN v3 INTEGER", []).is_ok() {
            conn.execute("DELETE FROM bodies", [])?;
        }
        // Some Workspace servers list the inbox as "Inbox"; it's INBOX here.
        conn.execute("UPDATE OR IGNORE envelopes SET mailbox = 'INBOX' WHERE mailbox != 'INBOX' AND lower(mailbox) = 'inbox'", []).ok();
        conn.execute("DELETE FROM envelopes WHERE mailbox != 'INBOX' AND lower(mailbox) = 'inbox'", []).ok();
        conn.execute("UPDATE OR IGNORE mailboxes SET name = 'INBOX' WHERE name != 'INBOX' AND lower(name) = 'inbox'", []).ok();
        conn.execute("DELETE FROM mailboxes WHERE name != 'INBOX' AND lower(name) = 'inbox'", []).ok();
        Ok(Self { conn })
    }

    /// Addresses the cache holds mail for (oldest setup first).
    pub fn cached_accounts(&self) -> Vec<String> {
        let Ok(mut stmt) = self.conn.prepare("SELECT DISTINCT account FROM mailboxes ORDER BY rowid") else { return Vec::new() };
        let Ok(rows) = stmt.query_map([], |r| r.get::<_, String>(0)) else { return Vec::new() };
        rows.filter_map(|r| r.ok()).filter(|a| a.contains('@')).collect()
    }

    pub fn mailboxes(&self, account: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT name FROM mailboxes WHERE account = ?1")?;
        let rows = stmt.query_map([account], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn save_mailboxes(&mut self, account: &str, names: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM mailboxes WHERE account = ?1", [account])?;
        for name in names {
            tx.execute("INSERT INTO mailboxes (account, name) VALUES (?1, ?2)", params![account, name])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Newest-first envelopes of one mailbox, with any cached bodies.
    pub fn envelopes(&self, account: &str, mailbox: &str, limit: u32) -> Result<Vec<Envelope>> {
        let sql = format!(
            "SELECT {ENVELOPE_COLUMNS} FROM envelopes e
             LEFT JOIN bodies b ON b.account = e.account AND b.mailbox = e.mailbox AND b.id = e.id
             WHERE e.account = ?1 AND e.mailbox = ?2
             ORDER BY e.ts DESC LIMIT ?3"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account, mailbox, limit], row_to_envelope)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Cached envelopes strictly older than `before_ts`, newest first.
    pub fn envelopes_before(&self, account: &str, mailbox: &str, before_ts: i64, limit: u32) -> Result<Vec<Envelope>> {
        let sql = format!(
            "SELECT {ENVELOPE_COLUMNS} FROM envelopes e
             LEFT JOIN bodies b ON b.account = e.account AND b.mailbox = e.mailbox AND b.id = e.id
             WHERE e.account = ?1 AND e.mailbox = ?2 AND e.ts < ?3
             ORDER BY e.ts DESC LIMIT ?4"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account, mailbox, before_ts, limit], row_to_envelope)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn count(&self, account: &str, mailbox: &str) -> Result<u64> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*) FROM envelopes WHERE account = ?1 AND mailbox = ?2",
            params![account, mailbox],
            |r| r.get::<_, i64>(0),
        )? as u64)
    }

    /// (uid_validity, low_uid, high_uid) of a mailbox's header sync.
    pub fn sync_state(&self, account: &str, mailbox: &str) -> Option<(u64, u64, u64)> {
        self.conn
            .query_row(
                "SELECT uid_validity, low_uid, high_uid FROM sync_state WHERE account = ?1 AND mailbox = ?2",
                params![account, mailbox],
                |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, i64>(1)? as u64, r.get::<_, i64>(2)? as u64)),
            )
            .ok()
    }

    pub fn set_sync_state(&self, account: &str, mailbox: &str, validity: u64, low: u64, high: u64) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO sync_state (account, mailbox, uid_validity, low_uid, high_uid)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![account, mailbox, validity as i64, low as i64, high as i64],
        )?;
        Ok(())
    }

    /// UIDs are only meaningful under one UIDVALIDITY: drop a mailbox's
    /// cached copies when it changes.
    pub fn reset_mailbox(&self, account: &str, mailbox: &str) -> Result<()> {
        for table in ["envelopes", "bodies", "sync_state"] {
            self.conn.execute(
                &format!("DELETE FROM {table} WHERE account = ?1 AND mailbox = ?2"),
                params![account, mailbox],
            )?;
        }
        Ok(())
    }

    /// Upsert a fresh server window for one mailbox and drop cached rows
    /// inside that window the server no longer returns (archived, deleted).
    pub fn save_envelopes(&mut self, account: &str, mailbox: &str, list: &[Envelope]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare(
                "INSERT INTO envelopes
                     (account, mailbox, id, message_id, from_name, from_email, subject, ts, unread, preview, flags, to_list, thread_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT (account, mailbox, id) DO UPDATE SET
                     message_id = excluded.message_id, from_name = excluded.from_name,
                     from_email = excluded.from_email, subject = excluded.subject,
                     ts = excluded.ts, unread = excluded.unread, flags = excluded.flags,
                     preview = CASE WHEN excluded.preview = '' THEN envelopes.preview ELSE excluded.preview END,
                     to_list = CASE WHEN excluded.to_list = '' THEN envelopes.to_list ELSE excluded.to_list END,
                     thread_id = COALESCE(excluded.thread_id, envelopes.thread_id)",
            )?;
            for e in list {
                upsert.execute(params![
                    account,
                    mailbox,
                    e.id,
                    e.message_id,
                    e.from.as_ref(),
                    e.from_email.as_ref(),
                    e.subject.as_ref(),
                    e.ts,
                    e.unread,
                    e.preview.as_ref(),
                    flag_bits(e),
                    encode_people(&e.to),
                    e.thread,
                ])?;
            }
            if let Some(oldest) = list.iter().map(|e| e.ts).min() {
                let keep: HashSet<&str> = list.iter().map(|e| e.id.as_str()).collect();
                let stale: Vec<String> = {
                    let mut stmt = tx.prepare(
                        "SELECT id FROM envelopes WHERE account = ?1 AND mailbox = ?2 AND ts >= ?3",
                    )?;
                    let ids = stmt.query_map(params![account, mailbox, oldest], |r| r.get::<_, String>(0))?;
                    ids.filter_map(|r| r.ok()).filter(|id| !keep.contains(id.as_str())).collect()
                };
                for id in stale {
                    tx.execute(
                        "DELETE FROM envelopes WHERE account = ?1 AND mailbox = ?2 AND id = ?3",
                        params![account, mailbox, id],
                    )?;
                    tx.execute(
                        "DELETE FROM bodies WHERE account = ?1 AND mailbox = ?2 AND id = ?3",
                        params![account, mailbox, id],
                    )?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Upsert envelopes without pruning (for search results, which are a
    /// sparse slice of a mailbox).
    pub fn upsert_envelopes(&mut self, account: &str, list: &[Envelope]) -> Result<()> {
        let tx = self.conn.transaction()?;
        {
            let mut upsert = tx.prepare(
                "INSERT INTO envelopes
                     (account, mailbox, id, message_id, from_name, from_email, subject, ts, unread, preview, flags, to_list, thread_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '', ?10, ?11, ?12)
                 ON CONFLICT (account, mailbox, id) DO UPDATE SET
                     unread = excluded.unread, subject = excluded.subject, flags = excluded.flags,
                     to_list = CASE WHEN excluded.to_list = '' THEN envelopes.to_list ELSE excluded.to_list END,
                     thread_id = COALESCE(excluded.thread_id, envelopes.thread_id)",
            )?;
            for e in list {
                upsert.execute(params![
                    account, e.mailbox, e.id, e.message_id, e.from.as_ref(),
                    e.from_email.as_ref(), e.subject.as_ref(), e.ts, e.unread, flag_bits(e),
                    encode_people(&e.to), e.thread,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// `upsert_envelopes` for callers holding a shared reference.
    pub fn upsert_envelopes_ref(&self, account: &str, list: &[Envelope]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut upsert = tx.prepare(
                "INSERT INTO envelopes
                     (account, mailbox, id, message_id, from_name, from_email, subject, ts, unread, preview, flags, to_list, thread_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, '', ?10, ?11, ?12)
                 ON CONFLICT (account, mailbox, id) DO UPDATE SET unread = excluded.unread, flags = excluded.flags,
                     to_list = CASE WHEN excluded.to_list = '' THEN envelopes.to_list ELSE excluded.to_list END,
                     thread_id = COALESCE(excluded.thread_id, envelopes.thread_id)",
            )?;
            for e in list {
                upsert.execute(params![
                    account, e.mailbox, e.id, e.message_id, e.from.as_ref(),
                    e.from_email.as_ref(), e.subject.as_ref(), e.ts, e.unread, flag_bits(e),
                    encode_people(&e.to), e.thread,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Envelopes by key with cached preview/body, for merging search hits.
    pub fn lookup(&self, account: &str, mailbox: &str, id: &str) -> Option<Envelope> {
        let sql = format!(
            "SELECT {ENVELOPE_COLUMNS} FROM envelopes e
             LEFT JOIN bodies b ON b.account = e.account AND b.mailbox = e.mailbox AND b.id = e.id
             WHERE e.account = ?1 AND e.mailbox = ?2 AND e.id = ?3"
        );
        self.conn.query_row(&sql, params![account, mailbox, id], row_to_envelope).ok()
    }

    /// Read state of a message and of its copies in other folders.
    pub fn set_unread(&self, account: &str, mailbox: &str, id: &str, message_id: Option<&str>, unread: bool) -> Result<()> {
        self.conn.execute(
            "UPDATE envelopes SET unread = ?5
             WHERE account = ?1 AND ((mailbox = ?2 AND id = ?3) OR (?4 IS NOT NULL AND message_id = ?4))",
            params![account, mailbox, id, message_id, unread],
        )?;
        Ok(())
    }

    /// Star / Important bits for every cached copy of one message.
    pub fn set_flags(&self, account: &str, e: &Envelope) -> Result<()> {
        self.conn.execute(
            "UPDATE envelopes SET flags = ?5
             WHERE account = ?1 AND ((mailbox = ?2 AND id = ?3) OR (?4 IS NOT NULL AND message_id = ?4))",
            params![account, e.mailbox, e.id, e.message_id, flag_bits(e)],
        )?;
        Ok(())
    }

    /// Autosave of the message being written (one slot), so nothing is
    /// lost if the app quits mid-sentence. `None` clears it.
    pub fn save_snapshot(&self, draft: Option<&crate::compose::Draft>) -> Result<()> {
        match draft {
            None => {
                self.conn.execute("DELETE FROM compose_snapshot", [])?;
            }
            Some(d) => {
                let (smb, sid) = d.server_draft.clone().unzip();
                self.conn.execute(
                    "INSERT OR REPLACE INTO compose_snapshot
                         (slot, kind, to_list, cc_list, subject, body, in_reply_to, refs, server_mailbox, server_id, mono, signature, from_name, from_email, attachments, bcc_list, account)
                     VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                    params![d.kind.as_str(), d.to, d.cc, d.subject, d.body, d.in_reply_to, d.references.join(" "), smb, sid, d.mono, d.signature,
                        d.from.as_ref().map(|f| f.0.clone()), d.from.as_ref().map(|f| f.1.clone()),
                        d.attachments.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>().join("\n"), d.bcc, d.account],
                )?;
            }
        }
        Ok(())
    }

    pub fn load_snapshot(&self) -> Option<crate::compose::Draft> {
        self.conn
            .query_row(
                "SELECT kind, to_list, cc_list, subject, body, in_reply_to, refs, server_mailbox, server_id, mono, signature, from_name, from_email, attachments, bcc_list, account
                 FROM compose_snapshot WHERE slot = 1",
                [],
                |r| {
                    let refs: String = r.get(6)?;
                    let smb: Option<String> = r.get(7)?;
                    let sid: Option<String> = r.get(8)?;
                    Ok(crate::compose::Draft {
                        kind: crate::compose::Kind::parse(&r.get::<_, String>(0)?),
                        to: r.get(1)?,
                        cc: r.get(2)?,
                        bcc: r.get::<_, Option<String>>(14)?.unwrap_or_default(),
                        account: r.get::<_, Option<String>>(15)?.unwrap_or_default(),
                        subject: r.get(3)?,
                        body: r.get(4)?,
                        in_reply_to: r.get(5)?,
                        references: refs.split_whitespace().map(str::to_string).collect(),
                        server_draft: smb.zip(sid),
                        mono: r.get::<_, Option<bool>>(9)?.unwrap_or(true),
                        signature: r.get(10)?,
                        from: r.get::<_, Option<String>>(11)?.zip(r.get::<_, Option<String>>(12)?),
                        attachments: r
                            .get::<_, Option<String>>(13)?
                            .map(|s| s.lines().filter(|l| !l.is_empty()).map(std::path::PathBuf::from).collect())
                            .unwrap_or_default(),
                    })
                },
            )
            .ok()
    }

    pub fn setting(&self, key: &str) -> Option<String> {
        self.conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0)).ok()
    }

    pub fn signatures(&self, account: &str) -> Result<Vec<crate::signature::Signature>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, label, layout, name, title, company, phone, email, website, note, accent,
                    logo_mime, logo, logo_width, logo_link, logo2_mime, logo2, logo2_width,
                    logo2_link, social_label, social_url, font
             FROM signatures WHERE account = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([account], |r| {
            let mime: Option<String> = r.get(11)?;
            let logo: Option<Vec<u8>> = r.get(12)?;
            Ok(crate::signature::Signature {
                id: r.get(0)?,
                label: r.get(1)?,
                layout: crate::signature::Layout::parse(&r.get::<_, String>(2)?),
                name: r.get(3)?,
                title: r.get(4)?,
                company: r.get(5)?,
                phone: r.get(6)?,
                email: r.get(7)?,
                website: r.get(8)?,
                note: r.get(9)?,
                accent: r.get::<_, i64>(10)? as u32,
                logo: mime.zip(logo),
                logo_width: r.get::<_, i64>(13)? as u32,
                logo_link: r.get(14)?,
                logo2: r.get::<_, Option<String>>(15)?.zip(r.get::<_, Option<Vec<u8>>>(16)?),
                logo2_width: r.get::<_, i64>(17)? as u32,
                logo2_link: r.get(18)?,
                social_label: r.get(19)?,
                social_url: r.get(20)?,
                font: r.get(21)?,
                account: String::new(),
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn delete_signature(&self, account: &str, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM signatures WHERE account = ?1 AND id = ?2", params![account, id])?;
        Ok(())
    }

    /// Count recipients of sent mail (people you write to).
    pub fn add_sent_contacts(&self, account: &str, list: &[(String, String, i64)]) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut up = tx.prepare(
                "INSERT INTO sent_contacts (account, email, name, times, last_ts) VALUES (?1, lower(?2), ?3, 1, ?4)
                 ON CONFLICT (account, email) DO UPDATE SET
                     times = times + 1,
                     name = CASE WHEN excluded.name != '' AND excluded.last_ts >= last_ts THEN excluded.name ELSE name END,
                     last_ts = max(last_ts, excluded.last_ts)",
            )?;
            for (name, email, ts) in list {
                up.execute(params![account, email, name.trim().trim_matches('"'), ts])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Address suggestions for `prefix` (matches name or address words).
    /// People you've written to rank first, then frequent senders;
    /// automated no-reply senders are left out.
    pub fn suggest_contacts(&self, account: &str, prefix: &str, limit: u32) -> Result<Vec<(String, String)>> {
        let p = prefix.trim().to_lowercase().replace('%', "").replace('_', "");
        if p.is_empty() {
            return Ok(Vec::new());
        }
        let starts = format!("{p}%");
        let word = format!("% {p}%");
        let mut stmt = self.conn.prepare(
            // Names: the one you used when writing, else the sender's most
            // recent (SQLite returns the bare column from the max(ts) row).
            "WITH received AS (
                 SELECT lower(from_email) AS email, from_name AS name, count(*) AS n, max(ts) AS last
                 FROM envelopes WHERE account = ?1 GROUP BY lower(from_email)
             ),
             people AS (
                 SELECT email, name, 1 AS mine, times * 20 AS score, last_ts AS last
                 FROM sent_contacts WHERE account = ?1
                 UNION ALL
                 SELECT email, name, 0 AS mine, n AS score, last FROM received
                 WHERE email NOT LIKE '%noreply%' AND email NOT LIKE '%no-reply%'
                   AND email NOT LIKE '%donotreply%' AND email NOT LIKE '%do-not-reply%'
                   AND email NOT LIKE 'notification%' AND email NOT LIKE 'mailer-daemon%'
             )
             SELECT email,
                    coalesce(max(CASE WHEN mine = 1 AND name != '' THEN name END),
                             max(CASE WHEN mine = 0 AND name != email THEN name END), '') AS name,
                    sum(score) AS score, max(last) AS last
             FROM people
             WHERE email LIKE ?2 OR lower(name) LIKE ?2 OR lower(name) LIKE ?3
                OR substr(email, instr(email, '@') + 1) LIKE ?2
             GROUP BY email ORDER BY score DESC, last DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![account, starts, word, limit], |r| {
            Ok((r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, String>(0)?))
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Forget cached copies after archive/delete. With `everywhere`, also
    /// drop copies in other folders that share a Message-ID (Gmail delete).
    pub fn remove_envelopes(&self, account: &str, items: &[(String, String, Option<String>)], everywhere: bool) -> Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for (mailbox, id, message_id) in items {
            tx.execute(
                "DELETE FROM envelopes WHERE account = ?1 AND mailbox = ?2 AND id = ?3",
                params![account, mailbox, id],
            )?;
            tx.execute(
                "DELETE FROM bodies WHERE account = ?1 AND mailbox = ?2 AND id = ?3",
                params![account, mailbox, id],
            )?;
            if let (true, Some(mid)) = (everywhere, message_id) {
                tx.execute("DELETE FROM envelopes WHERE account = ?1 AND message_id = ?2", params![account, mid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Senders ranked by volume, from the cached All Mail headers.
    pub fn sender_stats(&self, account: &str, all_mail: &str, me: &str, limit: u32) -> Result<Vec<SenderStat>> {
        let mut stmt = self.conn.prepare(
            "SELECT lower(e.from_email) AS email, e.from_name, count(*) AS n,
                    sum(e.unread) AS unread, max(e.ts) AS last, NULL
             FROM envelopes e
             WHERE e.account = ?1 AND e.mailbox = ?2 AND lower(e.from_email) != lower(?3)
               AND e.from_email != ''
             GROUP BY lower(e.from_email)
             ORDER BY n DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![account, all_mail, me, limit], |r| {
            Ok(SenderStat {
                email: r.get(0)?,
                name: r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                count: r.get::<_, i64>(2)? as u64,
                unread: r.get::<_, Option<i64>>(3)?.unwrap_or(0) as u64,
                last_ts: r.get(4)?,
                rule: r.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Newest subjects from one sender (for the Sweep preview).
    pub fn recent_subjects(&self, account: &str, all_mail: &str, email: &str, limit: u32) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT subject, ts FROM envelopes
             WHERE account = ?1 AND mailbox = ?2 AND lower(from_email) = ?3
             ORDER BY ts DESC LIMIT ?4",
        )?;
        let rows = stmt.query_map(params![account, all_mail, email, limit], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Cached UIDs in `mailbox` of messages from `email`.
    pub fn uids_from(&self, account: &str, mailbox: &str, email: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT id FROM envelopes WHERE account = ?1 AND mailbox = ?2 AND lower(from_email) = ?3",
        )?;
        let rows = stmt.query_map(params![account, mailbox, email], |r| r.get(0))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// The Screener's question: is there other mail from this sender (outside
    /// the Lobby, Spam and Trash), or have you written to them?
    /// Whether you know `email`: you've written to them, or there's other
    /// mail from them in the cache (not the Lobby, Spam or Trash, and not
    /// the messages in `batch` = (mailbox, id), nor copies of them by
    /// Message-ID). New mail is in the cache before plugins see it, so the
    /// batch has to be left out or a stranger's second message would vouch
    /// for their first.
    pub fn sender_known(&self, account: &str, email: &str, batch: &[(String, String)], batch_mids: &[String]) -> bool {
        let wrote: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM sent_contacts WHERE account = ?1 AND email = ?2", params![account, email], |r| r.get(0))
            .unwrap_or(0);
        if wrote > 0 {
            return true;
        }
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT mailbox, id, message_id FROM envelopes WHERE account = ?1 AND lower(from_email) = ?2
               AND mailbox NOT IN ('Lobby', '[Gmail]/Spam', '[Gmail]/Trash') LIMIT 500",
        ) else {
            return false;
        };
        let rows: Vec<(String, String, Option<String>)> = match stmt
            .query_map(params![account, email], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?)))
        {
            Ok(rows) => rows.flatten().collect(),
            Err(_) => return false,
        };
        rows.into_iter().any(|(mailbox, id, mid)| {
            let in_batch = batch.iter().any(|(m, i)| *i == id && m.eq_ignore_ascii_case(&mailbox));
            let copy = mid.is_some_and(|mid| batch_mids.contains(&mid));
            !in_batch && !copy
        })
    }

    /// Forget one cached message (it was moved out of `mailbox`).
    pub fn remove_envelope(&self, account: &str, mailbox: &str, id: &str) -> Result<()> {
        self.conn.execute("DELETE FROM envelopes WHERE account = ?1 AND mailbox = ?2 AND id = ?3", params![account, mailbox, id])?;
        Ok(())
    }

    /// Forget cached Inbox copies from a sender (after filing them).
    pub fn remove_inbox_from(&self, account: &str, email: &str) -> Result<()> {
        self.conn.execute(
            "DELETE FROM envelopes WHERE account = ?1 AND lower(mailbox) = 'inbox' AND lower(from_email) = ?2",
            params![account, email],
        )?;
        Ok(())
    }

    /// Rules as the cache database held them before rules.toml.
    pub fn rules(&self, account: &str) -> Result<std::collections::HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT email, dest FROM sender_rules WHERE account = ?1")?;
        let rows = stmt.query_map([account], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Rules now live in rules.toml; this clears the old copies once moved.
    pub fn clear_rules(&self, account: &str) -> Result<()> {
        self.conn.execute("DELETE FROM sender_rules WHERE account = ?1", [account])?;
        Ok(())
    }

    pub fn save_body(
        &self,
        account: &str,
        mailbox: &str,
        id: &str,
        body: &str,
        html: Option<&str>,
        preview: &str,
        files: &[crate::model::Attachment],
    ) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO bodies (account, mailbox, id, body, html, files) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![account, mailbox, id, body, html, crate::model::Attachment::encode(files)],
        )?;
        self.conn.execute(
            "UPDATE envelopes SET preview = ?4 WHERE account = ?1 AND mailbox = ?2 AND id = ?3",
            params![account, mailbox, id, preview],
        )?;
        Ok(())
    }

    /// Search every cached mailbox: sender, subject, preview and cached
    /// bodies. Gmail keeps the same message in several folders, so results
    /// are de-duplicated by Message-ID.
    pub fn search(&self, account: &str, query: &str, limit: u32) -> Result<Vec<Envelope>> {
        self.search_in(account, None, query, limit)
    }

    /// `search`, limited to one folder when `mailbox` is set (Sent).
    pub fn search_in(&self, account: &str, mailbox: Option<&str>, query: &str, limit: u32) -> Result<Vec<Envelope>> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| format!("%{}%", t.replace('%', "").replace('_', "")))
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = format!(
            "SELECT {ENVELOPE_COLUMNS} FROM envelopes e
             LEFT JOIN bodies b ON b.account = e.account AND b.mailbox = e.mailbox AND b.id = e.id
             WHERE e.account = ?1 AND (?2 IS NULL OR e.mailbox = ?2)"
        );
        for i in 0..terms.len() {
            let p = i + 3;
            sql.push_str(&format!(
                " AND (e.subject LIKE ?{p} OR e.from_name LIKE ?{p} OR e.from_email LIKE ?{p}
                      OR e.to_list LIKE ?{p} OR e.preview LIKE ?{p} OR b.body LIKE ?{p})"
            ));
        }
        sql.push_str(" ORDER BY e.ts DESC LIMIT 2000");
        let mut stmt = self.conn.prepare(&sql)?;
        let mut args: Vec<&dyn rusqlite::ToSql> = vec![&account, &mailbox];
        for t in &terms {
            args.push(t);
        }
        let rows = stmt.query_map(args.as_slice(), row_to_envelope)?;
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for e in rows.filter_map(|r| r.ok()) {
            let key = e.message_id.clone().unwrap_or_else(|| format!("{}/{}", e.mailbox, e.id));
            if seen.insert(key) {
                out.push(e);
                if out.len() as u32 >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    /// Every cached message in a conversation, oldest first. Gmail keeps a
    /// message in several folders; one copy each, preferring one whose body
    /// is cached, then one that isn't All Mail.
    pub fn thread(&self, account: &str, thread: &str) -> Result<Vec<Envelope>> {
        let sql = format!(
            "SELECT {ENVELOPE_COLUMNS} FROM envelopes e
             LEFT JOIN bodies b ON b.account = e.account AND b.mailbox = e.mailbox AND b.id = e.id
             WHERE e.account = ?1 AND e.thread_id = ?2
             ORDER BY e.ts ASC, (b.body IS NULL) ASC, (e.mailbox = '[Gmail]/All Mail') ASC LIMIT 500"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![account, thread], row_to_envelope)?;
        let mut seen = HashSet::new();
        Ok(rows
            .filter_map(|r| r.ok())
            .filter(|e| seen.insert(e.message_id.clone().unwrap_or_else(|| format!("{}/{}", e.mailbox, e.id))))
            .collect())
    }

    /// How many messages each conversation has in the cache.
    pub fn thread_counts(&self, account: &str, threads: &[String]) -> Result<std::collections::HashMap<String, usize>> {
        let mut out = std::collections::HashMap::new();
        let mut stmt = self.conn.prepare(
            "SELECT COUNT(DISTINCT COALESCE(message_id, mailbox || '/' || id)) FROM envelopes WHERE account = ?1 AND thread_id = ?2",
        )?;
        for t in threads {
            let n: i64 = stmt.query_row(params![account, t], |r| r.get(0))?;
            out.insert(t.clone(), n as usize);
        }
        Ok(out)
    }

    pub fn add_reminder(&self, r: &Reminder) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO reminders (account, kind, message_ids, thread, subject, created, due) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![r.account, r.kind, r.message_ids.join("\x1e"), r.thread, r.subject, r.created, r.due],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn reminders_due(&self, now: i64) -> Result<Vec<Reminder>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, account, kind, message_ids, thread, subject, created, due FROM reminders WHERE due <= ?1 ORDER BY due",
        )?;
        let rows = stmt.query_map(params![now], |r| {
            Ok(Reminder {
                id: r.get(0)?,
                account: r.get(1)?,
                kind: r.get(2)?,
                message_ids: r.get::<_, String>(3)?.split('\x1e').filter(|s| !s.is_empty()).map(str::to_string).collect(),
                thread: r.get(4)?,
                subject: r.get(5)?,
                created: r.get(6)?,
                due: r.get(7)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn delete_reminder(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM reminders WHERE id = ?1", params![id])?;
        Ok(())
    }

    pub fn drop_reminders_for(&self, account: &str, kind: &str, message_ids: &[String]) -> Result<()> {
        let mut stmt = self.conn.prepare("SELECT id, message_ids FROM reminders WHERE account = ?1 AND kind = ?2")?;
        let ids: Vec<i64> = stmt
            .query_map(params![account, kind], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
            .filter_map(|r| r.ok())
            .filter(|(_, mids)| mids.split('\x1e').any(|m| message_ids.iter().any(|x| x == m)))
            .map(|(id, _)| id)
            .collect();
        for id in ids {
            self.delete_reminder(id)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reminder {
    pub id: i64,
    pub account: String,
    pub kind: String,
    pub message_ids: Vec<String>,
    pub thread: Option<String>,
    pub subject: String,
    pub created: i64,
    pub due: i64,
}

#[cfg(test)]
pub mod tests_support {
    /// A plain cached message for tests.
    pub fn envelope(mailbox: &str, id: &str, from: &str) -> crate::model::Envelope {
        crate::model::Envelope {
            mailbox: mailbox.into(),
            id: id.into(),
            message_id: Some(format!("<{id}@x>")),
            from: "Someone".into(),
            from_email: from.to_string().into(),
            subject: "hi".into(),
            ts: 0,
            date: "".into(),
            preview: "".into(),
            body: None,
            html: None,
            unread: true,
            starred: false,
            important: false,
            attachments: Vec::new(),
            account: Default::default(),
            to: Vec::new(),
            thread: None,
            labels: Vec::new(),
        }
    }
}

#[cfg(test)]
mod thread_tests {
    use super::{decode_people, encode_people, tests_support::envelope, Store};

    fn scratch(name: &str) -> Store {
        let dir = std::env::temp_dir().join(format!("wax-store-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        Store::open_at(&dir.join("cache.sqlite")).unwrap()
    }

    #[test]
    fn people_round_trip() {
        let p = vec![("Ann, Jr.".to_string(), "ann@x.com".to_string()), ("".to_string(), "bob@x.com".to_string())];
        assert_eq!(decode_people(&encode_people(&p)), p);
        assert!(decode_people("").is_empty());
    }

    #[test]
    fn threads_and_sent_search() {
        let mut store = scratch("threads");
        let me = "me@x.com";
        let msg = |mailbox: &str, id: &str, from: &str, mid: &str, ts: i64, thread: &str| {
            let mut e = envelope(mailbox, id, from);
            e.message_id = Some(mid.into());
            e.ts = ts;
            e.thread = Some(thread.into());
            e
        };
        // A conversation: their message (in Inbox and All Mail), your reply
        // in Sent, and an unrelated message.
        let mut reply = msg("[Gmail]/Sent Mail", "5", me, "<r@x>", 20, "77");
        reply.to = vec![("Ann Lee".into(), "ann@shop.com".into())];
        reply.subject = "Re: the quote".into();
        store
            .upsert_envelopes(
                me,
                &[
                    msg("INBOX", "1", "ann@shop.com", "<a@x>", 10, "77"),
                    msg("[Gmail]/All Mail", "100", "ann@shop.com", "<a@x>", 10, "77"),
                    reply,
                    msg("INBOX", "2", "other@x.com", "<o@x>", 30, "88"),
                ],
            )
            .unwrap();
        let t = store.thread(me, "77").unwrap();
        let ids: Vec<(&str, &str)> = t.iter().map(|e| (e.mailbox.as_str(), e.id.as_str())).collect();
        assert_eq!(ids, [("INBOX", "1"), ("[Gmail]/Sent Mail", "5")], "one copy each, oldest first, not the All Mail copy");
        let counts = store.thread_counts(me, &["77".into(), "88".into()]).unwrap();
        assert_eq!((counts["77"], counts["88"]), (2, 1));
        // Sent search finds the reply by recipient name or address, and
        // stays in Sent.
        let found = |q: &str, mb: Option<&str>| store.search_in(me, mb, q, 10).unwrap().into_iter().map(|e| e.id).collect::<Vec<_>>();
        assert_eq!(found("ann lee", Some("[Gmail]/Sent Mail")), ["5"]);
        assert_eq!(found("shop.com", Some("[Gmail]/Sent Mail")), ["5"]);
        assert_eq!(found("quote", Some("[Gmail]/Sent Mail")), ["5"]);
        assert!(found("other", Some("[Gmail]/Sent Mail")).is_empty());
        assert_eq!(found("other", None), ["2"]);
        // A later refresh without recipients or thread keeps them.
        let mut bare = envelope("[Gmail]/Sent Mail", "5", me);
        bare.message_id = Some("<r@x>".into());
        store.upsert_envelopes(me, &[bare]).unwrap();
        let again = store.lookup(me, "[Gmail]/Sent Mail", "5").unwrap();
        assert_eq!((again.to.len(), again.thread.as_deref()), (1, Some("77")));
    }

    #[test]
    fn reminders_come_due() {
        let store = scratch("reminders");
        let r = |kind: &str, mids: &[&str], due: i64| super::Reminder {
            id: 0,
            account: "me@x.com".into(),
            kind: kind.into(),
            message_ids: mids.iter().map(|m| m.to_string()).collect(),
            thread: Some("77".into()),
            subject: "Quote".into(),
            created: 1,
            due,
        };
        store.add_reminder(&r("snooze", &["<a@x>", "<b@x>"], 100)).unwrap();
        store.add_reminder(&r("follow-up", &["<c@x>"], 200)).unwrap();
        assert!(store.reminders_due(50).unwrap().is_empty());
        let due = store.reminders_due(150).unwrap();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].message_ids, ["<a@x>", "<b@x>"]);
        store.drop_reminders_for("me@x.com", "snooze", &["<b@x>".into()]).unwrap();
        assert_eq!(store.reminders_due(1000).unwrap().len(), 1);
        let left = store.reminders_due(1000).unwrap();
        store.delete_reminder(left[0].id).unwrap();
        assert!(store.reminders_due(1000).unwrap().is_empty());
    }
}
