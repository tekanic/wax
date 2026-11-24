//! Writing mail: what a draft holds, how replies and forwards are
//! prefilled, address parsing, and building the RFC 5322 message.

use anyhow::{anyhow, Result};

/// Name on outgoing mail.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    New,
    Reply,
    ReplyAll,
    Forward,
}

impl Kind {
    pub fn title(self) -> &'static str {
        match self {
            Kind::New => "New message",
            Kind::Reply => "Reply",
            Kind::ReplyAll => "Reply all",
            Kind::Forward => "Forward",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::New => "new",
            Kind::Reply => "reply",
            Kind::ReplyAll => "reply-all",
            Kind::Forward => "forward",
        }
    }
    pub fn parse(s: &str) -> Kind {
        match s {
            "reply" => Kind::Reply,
            "reply-all" => Kind::ReplyAll,
            "forward" => Kind::Forward,
            _ => Kind::New,
        }
    }
}

/// Everything needed to rebuild a composer (autosave, undo send).
#[derive(Clone, Debug)]
pub struct Draft {
    pub kind: Kind,
    pub to: String,
    pub cc: String,
    /// Blind copies: delivered, never shown to the other recipients.
    pub bcc: String,
    /// The account it's written in and sent through ("" = the one on screen).
    pub account: String,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    /// Server copy in Gmail's Drafts folder this draft replaces, if any.
    pub server_draft: Option<(String, String)>,
    /// Sent in a monospaced font.
    pub mono: bool,
    /// Signature shown under the message (and above any quote).
    pub signature: Option<i64>,
    /// (name, email) to send as; None = the account's own address.
    pub from: Option<(String, String)>,
    /// Files to attach.
    pub attachments: Vec<std::path::PathBuf>,
}

impl Draft {
    pub fn new() -> Self {
        Draft {
            kind: Kind::New,
            to: String::new(),
            cc: String::new(),
            bcc: String::new(),
            account: String::new(),
            subject: String::new(),
            body: String::new(),
            in_reply_to: None,
            references: Vec::new(),
            server_draft: None,
            mono: true,
            signature: None,
            from: None,
            attachments: Vec::new(),
        }
    }

    pub fn is_blank(&self) -> bool {
        self.to.trim().is_empty() && self.cc.trim().is_empty() && self.bcc.trim().is_empty() && self.subject.trim().is_empty() && self.body.trim().is_empty()
            && self.attachments.is_empty()
    }
}

/// What a reply or forward needs from the original message.
#[derive(Clone, Debug, Default)]
pub struct Original {
    pub from: Vec<(String, String)>,
    pub reply_to: Vec<(String, String)>,
    pub to: Vec<(String, String)>,
    pub cc: Vec<(String, String)>,
    /// Only present on our own drafts.
    pub bcc: Vec<(String, String)>,
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub subject: String,
    /// Human date, e.g. "Mon, Nov 23, 2026 at 2:37 PM".
    pub date: String,
    pub text: String,
    /// (name, bytes) of attached files.
    pub files: Vec<(String, Vec<u8>)>,
}

/// Prefill a draft from the message being answered or forwarded.
/// `mine` = every address that is me (the account and its identities),
/// left out of reply recipients.
pub fn prefill(kind: Kind, orig: &Original, mine: &[String]) -> Draft {
    let mut d = Draft::new();
    d.kind = kind;
    let sender = orig.from.first().map(|(n, e)| display_address(n, e)).unwrap_or_default();
    match kind {
        Kind::New => {}
        Kind::Reply | Kind::ReplyAll => {
            let primary = if orig.reply_to.is_empty() { &orig.from } else { &orig.reply_to };
            let mut to: Vec<(String, String)> = primary.clone();
            let mut cc: Vec<(String, String)> = Vec::new();
            if kind == Kind::ReplyAll {
                to.extend(orig.to.iter().cloned());
                cc.extend(orig.cc.iter().cloned());
            }
            let not_me = |(_, e): &(String, String)| !mine.iter().any(|m| e.eq_ignore_ascii_case(m));
            let mut seen = std::collections::HashSet::new();
            let mut keep = |list: Vec<(String, String)>| -> Vec<(String, String)> {
                list.into_iter()
                    .filter(not_me)
                    .filter(|(_, e)| seen.insert(e.to_ascii_lowercase()))
                    .collect()
            };
            let mut to = keep(to);
            let cc = keep(cc);
            if to.is_empty() {
                // Replying to your own message: answer the original recipients.
                to = orig.to.clone();
            }
            d.to = join_addresses(&to);
            d.cc = join_addresses(&cc);
            d.subject = prefixed("Re:", &orig.subject);
            d.body = format!("\n\nOn {}, {} wrote:\n{}", orig.date, sender, quote(&orig.text));
            d.in_reply_to = orig.message_id.clone();
            d.references = orig.references.clone();
            if let Some(id) = &orig.message_id {
                d.references.push(id.clone());
            }
        }
        Kind::Forward => {
            d.subject = prefixed("Fwd:", &orig.subject);
            d.attachments = crate::attach::stage(&orig.files);
            d.body = format!(
                "\n\n---------- Forwarded message ---------\nFrom: {}\nDate: {}\nSubject: {}\nTo: {}\n\n{}",
                sender,
                orig.date,
                orig.subject,
                join_addresses(&orig.to),
                orig.text
            );
        }
    }
    d
}

/// Reopen a message from Gmail's Drafts folder for editing.
pub fn from_server_draft(orig: &Original, mailbox: &str, id: &str) -> Draft {
    let mut d = Draft::new();
    d.to = join_addresses(&orig.to);
    d.cc = join_addresses(&orig.cc);
    d.bcc = join_addresses(&orig.bcc);
    d.subject = orig.subject.clone();
    d.body = orig.text.clone();
    d.references = orig.references.clone();
    d.server_draft = Some((mailbox.to_string(), id.to_string()));
    d.attachments = crate::attach::stage(&orig.files);
    d
}

fn prefixed(prefix: &str, subject: &str) -> String {
    let s = subject.trim();
    if s.to_ascii_lowercase().starts_with(&prefix.to_ascii_lowercase()) {
        s.to_string()
    } else {
        format!("{prefix} {s}")
    }
}

fn quote(text: &str) -> String {
    text.lines()
        .map(|l| if l.is_empty() { ">".to_string() } else { format!("> {l}") })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn display_address(name: &str, email: &str) -> String {
    if name.trim().is_empty() || name == email {
        email.to_string()
    } else if name.contains(',') || name.contains('"') {
        format!("\"{}\" <{email}>", name.replace('"', ""))
    } else {
        format!("{name} <{email}>")
    }
}

fn join_addresses(list: &[(String, String)]) -> String {
    list.iter().map(|(n, e)| display_address(n, e)).collect::<Vec<_>>().join(", ")
}

/// Parse "Name <a@b.c>, d@e.f; \"Last, First\" <g@h.i>" into (name, email).
pub fn parse_addresses(field: &str) -> Result<Vec<(String, String)>> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut in_angle = false;
    let mut parts = Vec::new();
    for c in field.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            '<' => in_angle = true,
            '>' => in_angle = false,
            ',' | ';' if !in_quotes && !in_angle => {
                parts.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    parts.push(current);
    for part in parts {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, email) = match (part.rfind('<'), part.rfind('>')) {
            (Some(a), Some(b)) if a < b => (part[..a].trim().trim_matches('"').trim().to_string(), part[a + 1..b].trim().to_string()),
            _ => (String::new(), part.trim_matches('"').to_string()),
        };
        let valid = email.contains('@')
            && !email.starts_with('@')
            && !email.ends_with('@')
            && !email.contains(char::is_whitespace);
        if !valid {
            return Err(anyhow!("\"{part}\" doesn't look like an email address"));
        }
        out.push((name, email));
    }
    Ok(out)
}

fn parse_lenient(field: &str) -> Vec<(String, String)> {
    field
        .split([',', ';'])
        .filter_map(|p| parse_addresses(p).ok())
        .flatten()
        .collect()
}

/// The RFC 5322 bytes for a draft, ready for SMTP or a Drafts APPEND.
/// `strict`: refuse bad addresses (sending). Drafts pass `false` and keep
/// whatever parses, so a half-typed address never blocks saving.
/// Split a body into what you wrote and the quoted original (reply or
/// forward), so the signature can go between them.
pub fn split_quote(body: &str) -> (&str, &str) {
    let mut at = body.find("---------- Forwarded message ---------");
    let mut pos = 0;
    for line in body.split_inclusive('\n') {
        let next = body[pos + line.len()..].lines().next().unwrap_or("");
        if line.starts_with("On ") && line.trim_end().ends_with("wrote:") && next.starts_with('>') {
            at = Some(at.map_or(pos, |a| a.min(pos)));
            break;
        }
        pos += line.len();
    }
    match at {
        Some(i) => (&body[..i], &body[i..]),
        None => (body, ""),
    }
}

/// text/plain and text/html for a draft and its signature.
pub fn render_bodies(d: &Draft, sig: Option<&crate::signature::Signature>, preview: bool) -> (String, String) {
    let (mine, quoted) = split_quote(&d.body);
    let font = crate::md::font(d.mono);
    let mut text = mine.trim_end().to_string();
    let mut html = crate::md::to_email_html(mine, d.mono);
    if let Some(sig) = sig {
        text.push_str("\n\n");
        text.push_str(&sig.text());
        html.push_str(&sig.html(font, preview));
    }
    if !quoted.is_empty() {
        text.push_str("\n\n");
        text.push_str(quoted.trim_end());
        html.push_str("<br>");
        html.push_str(&crate::md::to_email_html(quoted, d.mono));
    }
    text.push('\n');
    (text, html)
}

pub fn build_message(
    d: &Draft,
    from_email: &str,
    strict: bool,
    sig: Option<&crate::signature::Signature>,
) -> Result<(Vec<u8>, String)> {
    use mail_builder::MessageBuilder;
    let parse = |f: &str| if strict { parse_addresses(f) } else { Ok(parse_lenient(f)) };
    let to = parse(&d.to)?;
    let cc = parse(&d.cc)?;
    let bcc = parse(&d.bcc)?;
    let message_id = format!(
        "{}.{}@gpui-mail.local",
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    );
    // An empty name must not become `"" <addr>`.
    let to_addr = |list: Vec<(String, String)>| -> mail_builder::headers::address::Address<'static> {
        let items: Vec<mail_builder::headers::address::Address<'static>> = list
            .into_iter()
            .map(|(n, e)| if n.trim().is_empty() { e.into() } else { (n, e).into() })
            .collect();
        items.into()
    };
    let to = if to.is_empty() { None } else { Some(to_addr(to)) };
    let cc = if cc.is_empty() { None } else { Some(to_addr(cc)) };
    // The Bcc header is kept in drafts so they reopen with it. A message
    // being sent never carries it (Gmail would deliver it as written);
    // blind recipients go only in the SMTP envelope (`envelope_recipients`).
    let bcc = if bcc.is_empty() || strict { None } else { Some(to_addr(bcc)) };
    let mut b = MessageBuilder::new()
        .from(match d.from.clone() {
            Some((name, email)) if !name.trim().is_empty() => mail_builder::headers::address::Address::from((name, email)),
            Some((_, email)) => email.into(),
            None => from_email.to_string().into(),
        })
        .subject(d.subject.trim().to_string())
        .message_id(message_id.clone())
        .date(chrono::Utc::now().timestamp())
        ;
    let (text, html) = render_bodies(d, sig, false);
    b = b.text_body(text).html_body(html);
    if let Some((mime, bytes)) = sig.filter(|s| s.has_logo()).and_then(|s| s.logo.clone()) {
        b = b.inline(mime, "sig-logo", bytes);
    }
    if let Some((mime, bytes)) = sig.filter(|s| s.has_logo2()).and_then(|s| s.logo2.clone()) {
        b = b.inline(mime, "sig-logo2", bytes);
    }
    for path in &d.attachments {
        match std::fs::read(path) {
            Ok(bytes) => {
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "attachment".into());
                b = b.attachment(crate::attach::mime_for(path), name, bytes);
            }
            // A draft keeps its place even if a file went missing; a send stops.
            Err(e) if strict => return Err(anyhow!("Could not read {}: {e}", path.display())),
            Err(_) => {}
        }
    }
    if let Some(to) = to {
        b = b.to(to);
    }
    if let Some(cc) = cc {
        b = b.cc(cc);
    }
    if let Some(bcc) = bcc {
        b = b.bcc(bcc);
    }
    if let Some(id) = &d.in_reply_to {
        b = b.in_reply_to(id.clone());
    }
    if !d.references.is_empty() {
        b = b.references(d.references.clone());
    }
    Ok((b.write_to_vec()?, message_id))
}

/// Every address the message goes to: To, Cc and Bcc, without repeats.
pub fn envelope_recipients(d: &Draft) -> Result<Vec<String>> {
    let mut all: Vec<String> = Vec::new();
    for field in [&d.to, &d.cc, &d.bcc] {
        for (_, email) in parse_addresses(field)? {
            if !all.iter().any(|e| e.eq_ignore_ascii_case(&email)) {
                all.push(email);
            }
        }
    }
    Ok(all)
}

/// Problems that should stop a send, in plain words.
pub fn check_sendable(d: &Draft) -> Result<()> {
    parse_addresses(&d.to)?;
    parse_addresses(&d.cc)?;
    parse_addresses(&d.bcc).map_err(|e| anyhow!("Bcc: {e}"))?;
    if envelope_recipients(d)?.is_empty() {
        return Err(anyhow!("Add at least one recipient"));
    }
    if d.subject.trim().is_empty() && d.body.trim().is_empty() && d.attachments.is_empty() {
        return Err(anyhow!("The message is empty"));
    }
    crate::attach::check(&d.attachments)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sends_markdown_as_text_and_html() {
        let mut d = Draft::new();
        d.to = "jim@example.com".into();
        d.subject = "Update".into();
        d.body = "Hi Jim,\nThis is **done**.".into();
        let (raw, _) = build_message(&d, "me@example.com", true, None).unwrap();
        let msg = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert!(msg.body_text(0).unwrap().contains("**done**"), "plain part keeps the Markdown");
        let html = msg.body_html(0).unwrap();
        assert!(html.contains("<strong>done</strong>"), "{html}");
        assert!(html.contains("Hi Jim,<br>"), "{html}");
    }

    #[test]
    fn signature_goes_above_the_quote_with_its_logo() {
        let mut d = Draft::new();
        d.to = "jim@example.com".into();
        d.body = "Sounds good.\n\nOn Mon, Nov 23, Sam wrote:\n> Can you send it?".into();
        let mut sig = crate::signature::Signature::new("Work");
        sig.layout = crate::signature::Layout::LogoLeft;
        sig.logo = Some(("image/png".into(), vec![0x89, b'P', b'N', b'G']));
        let (text, html) = render_bodies(&d, Some(&sig), false);
        let (s, q) = (text.find("-- \n").unwrap(), text.find("On Mon").unwrap());
        assert!(text.find("Sounds good").unwrap() < s && s < q, "{text}");
        assert!(html.find("cid:sig-logo").unwrap() < html.find("blockquote").unwrap());
        let (raw, _) = build_message(&d, "me@example.com", true, Some(&sig)).unwrap();
        let msg = mail_parser::MessageParser::default().parse(&raw).unwrap();
        use mail_parser::MimeHeaders;
        let logo = msg.attachments().find(|a| a.content_id() == Some("sig-logo"));
        assert!(logo.is_some(), "logo travels inline");
        // Monospace reaches the HTML.
        assert!(html.contains("Menlo"));
    }

    #[test]
    fn sends_from_the_chosen_identity() {
        let mut d = Draft::new();
        d.to = "jim@example.com".into();
        d.body = "Hi".into();
        d.from = Some(("Alex Doe".into(), "alex@example.com".into()));
        let (raw, _) = build_message(&d, "me@gmail.com", true, None).unwrap();
        let msg = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(msg.from().and_then(|a| a.first()).and_then(|a| a.address()), Some("alex@example.com"));
        d.from = None;
        let (raw, _) = build_message(&d, "me@gmail.com", true, None).unwrap();
        let msg = mail_parser::MessageParser::default().parse(&raw).unwrap();
        assert_eq!(msg.from().and_then(|a| a.first()).and_then(|a| a.address()), Some("me@gmail.com"));
    }

    #[test]
    fn bcc_stays_hidden() {
        let mut d = Draft::new();
        d.to = "jim@example.com".into();
        d.bcc = "Boss <boss@example.com>, JIM@example.com".into();
        d.body = "hi".into();
        assert!(check_sendable(&d).is_ok());
        assert_eq!(envelope_recipients(&d).unwrap(), vec!["jim@example.com", "boss@example.com"]);
        let (sent, _) = build_message(&d, "me@gmail.com", true, None).unwrap();
        let sent = String::from_utf8_lossy(&sent).to_lowercase();
        assert!(!sent.contains("bcc:") && !sent.contains("boss@"), "Bcc leaked into the sent message");
        let (draft, _) = build_message(&d, "me@gmail.com", false, None).unwrap();
        let msg = mail_parser::MessageParser::default().parse(&draft).unwrap();
        assert!(msg.bcc().is_some(), "drafts keep Bcc");
        // Bcc alone is enough to send.
        d.to.clear();
        assert!(check_sendable(&d).is_ok());
    }

    #[test]
    fn attaches_files() {
        use mail_parser::MimeHeaders;
        let dir = std::env::temp_dir().join(format!("gpui-mail-att-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Quarterly report.pdf");
        std::fs::write(&file, b"%PDF-1.4 test").unwrap();
        let mut d = Draft::new();
        d.to = "jim@example.com".into();
        d.body = "Attached.".into();
        d.attachments = vec![file.clone()];
        assert!(check_sendable(&d).is_ok());
        let (raw, _) = build_message(&d, "me@gmail.com", true, None).unwrap();
        let msg = mail_parser::MessageParser::default().parse(&raw).unwrap();
        let a = msg.attachments().next().expect("one attachment");
        assert_eq!(a.attachment_name(), Some("Quarterly report.pdf"));
        assert_eq!(a.contents(), b"%PDF-1.4 test");
        assert_eq!(a.content_type().map(|c| c.c_subtype.as_deref().unwrap_or("")), Some("pdf"));
        // A missing file stops a send.
        std::fs::remove_file(&file).unwrap();
        assert!(check_sendable(&d).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn splits_forwards_too() {
        let (mine, quoted) = split_quote("FYI\n\n---------- Forwarded message ---------\nFrom: x");
        assert_eq!(mine, "FYI\n\n");
        assert!(quoted.starts_with("----"));
        assert_eq!(split_quote("just me").1, "");
    }

    #[test]
    fn parses_addresses() {
        let list = parse_addresses("Jane <jane@x.com>, bob@y.org; \"Doe, John\" <jd@z.net>").unwrap();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0], ("Jane".into(), "jane@x.com".into()));
        assert_eq!(list[1], ("".into(), "bob@y.org".into()));
        assert_eq!(list[2], ("Doe, John".into(), "jd@z.net".into()));
        assert!(parse_addresses("not an address").is_err());
        assert!(parse_addresses("  ").unwrap().is_empty());
    }

    #[test]
    fn reply_all_drops_me_and_duplicates() {
        let orig = Original {
            from: vec![("Ann".into(), "ann@a.com".into())],
            to: vec![("Me".into(), "me@g.com".into()), ("Bo".into(), "bo@b.com".into())],
            cc: vec![("Ann".into(), "ANN@a.com".into()), ("Cy".into(), "cy@c.com".into())],
            message_id: Some("m1@a.com".into()),
            references: vec!["m0@a.com".into()],
            subject: "Plans".into(),
            date: "Mon".into(),
            text: "hi\n\nthere".into(),
            ..Default::default()
        };
        let d = prefill(Kind::ReplyAll, &orig, &["me@g.com".to_string()]);
        assert_eq!(d.to, "Ann <ann@a.com>, Bo <bo@b.com>");
        assert_eq!(d.cc, "Cy <cy@c.com>");
        assert_eq!(d.subject, "Re: Plans");
        assert_eq!(d.references, vec!["m0@a.com".to_string(), "m1@a.com".to_string()]);
        assert!(d.body.contains("> hi\n>\n> there"));
        let f = prefill(Kind::Forward, &orig, &["me@g.com".to_string()]);
        assert_eq!(f.subject, "Fwd: Plans");
        assert!(f.to.is_empty());
    }

    #[test]
    fn builds_rfc5322() {
        let mut d = Draft::new();
        d.to = "Ann <ann@a.com>".into();
        d.subject = "Hello".into();
        d.body = "Body text".into();
        d.in_reply_to = Some("m1@a.com".into());
        let (raw, id) = build_message(&d, "me@g.com", true, None).unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(text.contains("To: \"Ann\" <ann@a.com>") || text.contains("To: Ann <ann@a.com>"));
        assert!(text.contains("Subject: Hello"));
        assert!(text.contains("In-Reply-To: <m1@a.com>"));
        assert!(text.contains(&id));
        let mut bare = Draft::new();
        bare.to = "bob@y.org, Ann <ann@a.com>".into();
        bare.subject = "x".into();
        let (raw, _) = build_message(&bare, "me@g.com", true, None).unwrap();
        let text = String::from_utf8(raw).unwrap();
        assert!(!text.contains("\"\""), "{text}");
        assert!(text.contains("bob@y.org"));
    }
}
