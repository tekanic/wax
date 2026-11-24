//! Email signatures: several per account, each laid out visually in the
//! designer (layout, fields, accent colour, logo) and kept apart from the
//! message text, so switching one never leaves pieces behind.
//!
//! Rendered as an email-safe table. The logo travels inside the message as
//! an inline image (`cid:`), so mail clients show it without "load images".

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// Text only, under a coloured rule.
    Stacked,
    /// Logo | rule | details.
    LogoLeft,
    /// Logo above the details.
    LogoTop,
    /// Two columns (name | contact) over two logos, then a follow line.
    TwoLogos,
}

impl Layout {
    pub const ALL: [Layout; 4] = [Layout::Stacked, Layout::LogoLeft, Layout::LogoTop, Layout::TwoLogos];

    pub fn as_str(self) -> &'static str {
        match self {
            Layout::Stacked => "stacked",
            Layout::LogoLeft => "logo-left",
            Layout::LogoTop => "logo-top",
            Layout::TwoLogos => "two-logos",
        }
    }

    pub fn parse(s: &str) -> Layout {
        match s {
            "logo-left" => Layout::LogoLeft,
            "logo-top" => Layout::LogoTop,
            "two-logos" => Layout::TwoLogos,
            _ => Layout::Stacked,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Layout::Stacked => "Text",
            Layout::LogoLeft => "Logo left",
            Layout::LogoTop => "Logo on top",
            Layout::TwoLogos => "Two logos",
        }
    }
}

/// Accent colours offered in the designer.
pub const ACCENTS: [u32; 7] = [0x1a62d6, 0x1a3f7c, 0x0f8a6c, 0xc2571a, 0x7a3fc4, 0xb3261e, 0x3c3c43];

/// Signature fonts; empty means the message's own font.
pub const FONTS: [(&str, &str); 4] = [
    ("Message font", ""),
    ("Verdana", "Verdana, Geneva, sans-serif"),
    ("Helvetica", "Helvetica, Arial, sans-serif"),
    ("Georgia", "Georgia, 'Times New Roman', serif"),
];

#[derive(Clone, Debug)]
pub struct Signature {
    pub id: i64,
    /// What it's called in the picker ("Work", "Personal").
    pub label: String,
    pub layout: Layout,
    pub name: String,
    pub title: String,
    pub company: String,
    pub phone: String,
    pub email: String,
    pub website: String,
    /// Free text under the details (one or more lines).
    pub note: String,
    pub accent: u32,
    pub logo: Option<(String, Vec<u8>)>,
    /// Logo width in pixels.
    pub logo_width: u32,
    /// Where clicking the logo goes (optional).
    pub logo_link: String,
    /// The second logo, for the Two logos layout.
    pub logo2: Option<(String, Vec<u8>)>,
    pub logo2_width: u32,
    pub logo2_link: String,
    /// "Follow <company> on <social_label>" line, when `social_url` is set.
    pub social_label: String,
    pub social_url: String,
    /// CSS font stack; empty = the message's font.
    pub font: String,
    /// The account it belongs to (its address).
    pub account: String,
}

impl Signature {
    pub fn new(label: &str) -> Self {
        Signature {
            id: 0,
            label: label.to_string(),
            layout: Layout::Stacked,
            name: String::new(),
            title: String::new(),
            company: String::new(),
            phone: String::new(),
            email: String::new(),
            website: String::new(),
            note: String::new(),
            accent: ACCENTS[0],
            logo: None,
            logo_width: 72,
            logo_link: String::new(),
            logo2: None,
            logo2_width: 142,
            logo2_link: String::new(),
            social_label: String::new(),
            social_url: String::new(),
            font: String::new(),
            account: String::new(),
        }
    }

    pub fn has_logo(&self) -> bool {
        self.logo.is_some() && self.layout != Layout::Stacked
    }

    pub fn has_logo2(&self) -> bool {
        self.logo2.is_some() && self.layout == Layout::TwoLogos
    }

    /// "Follow Acme on LinkedIn".
    pub fn follow_line(&self) -> Option<(String, String)> {
        if self.social_url.trim().is_empty() {
            return None;
        }
        let who = if self.company.trim().is_empty() { "us".to_string() } else { self.company.trim().to_string() };
        let label = if self.social_label.trim().is_empty() { "LinkedIn" } else { self.social_label.trim() };
        Some((format!("Follow {who} on"), label.to_string()))
    }

    /// "Title, Company" with whichever parts exist.
    pub fn role_line(&self) -> String {
        [self.title.trim(), self.company.trim()].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(", ")
    }

    /// Phone, email and website on one line.
    pub fn contact_parts(&self) -> Vec<String> {
        [self.phone.trim(), self.email.trim(), self.website.trim()]
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect()
    }

    /// The plain-text version, after the standard "-- " separator.
    pub fn text(&self) -> String {
        let mut lines = vec!["-- ".to_string()];
        for l in [self.name.trim().to_string(), self.role_line(), self.contact_parts().join(" | ")] {
            if !l.is_empty() {
                lines.push(l);
            }
        }
        lines.extend(self.note.lines().map(str::to_string).filter(|l| !l.trim().is_empty()));
        if let Some((lead, label)) = self.follow_line() {
            lines.push(format!("{lead} {label}: {}", self.social_url.trim()));
        }
        lines.join("\n")
    }

    /// Email-safe HTML. Logos are `cid:` references when sending (the
    /// images travel inline) and data URIs when `preview`ing.
    pub fn html(&self, message_font: &str, preview: bool) -> String {
        let font = if self.font.trim().is_empty() { message_font.to_string() } else { self.font.replace('"', "'") };
        let font = font.as_str();
        let accent = format!("#{:06x}", self.accent);
        let src = |n: u8| -> String {
            match (preview, n) {
                (true, 1) => data_uri(&self.logo),
                (true, _) => data_uri(&self.logo2),
                (false, 1) => "cid:sig-logo".into(),
                (false, _) => "cid:sig-logo2".into(),
            }
        };
        let linked = |img: String, href: &str| -> String {
            if href.trim().is_empty() {
                img
            } else {
                format!("<a href=\"{}\" target=\"_blank\" style=\"text-decoration:none;border:0;\">{img}</a>", esc(&full_url(href)))
            }
        };
        if self.layout == Layout::TwoLogos {
            return self.two_logos_html(font, &accent, &src, &linked);
        }
        let link = |s: &str| -> String {
            let href = if s.contains('@') && !s.contains("://") {
                format!("mailto:{s}")
            } else if s.starts_with("http") {
                s.to_string()
            } else if s.contains('.') && !s.chars().next().is_some_and(|c| c == '+' || c == '(' || c.is_ascii_digit()) {
                format!("https://{s}")
            } else if s.chars().filter(|c| c.is_ascii_digit()).count() >= 7 {
                format!("tel:{}", s.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect::<String>())
            } else {
                return format!("<span style=\"white-space:nowrap;\">{}</span>", esc(s));
            };
            format!("<a href=\"{}\" style=\"color:{accent};text-decoration:none;white-space:nowrap;\">{}</a>", esc(&href), esc(s))
        };
        let mut details = String::new();
        if !self.name.trim().is_empty() {
            details.push_str(&format!(
                "<div style=\"font-size:15px;font-weight:700;color:#111;\">{}</div>",
                esc(self.name.trim())
            ));
        }
        let role = self.role_line();
        if !role.is_empty() {
            details.push_str(&format!("<div style=\"color:#555;\">{}</div>", esc(&role)));
        }
        let contact = self.contact_parts();
        if !contact.is_empty() {
            let sep = format!("<span style=\"color:{accent};\">&nbsp;|&nbsp;</span>");
            details.push_str(&format!(
                "<div style=\"margin-top:6px;\">{}</div>",
                contact.iter().map(|c| link(c)).collect::<Vec<_>>().join(&sep)
            ));
        }
        for l in self.note.lines().filter(|l| !l.trim().is_empty()) {
            details.push_str(&format!("<div style=\"margin-top:4px;color:#666;font-size:12px;\">{}</div>", esc(l)));
        }
        let logo = linked(
            format!(
                "<img src=\"{}\" width=\"{w}\" alt=\"\" style=\"display:block;width:{w}px;height:auto;border:0;\">",
                src(1),
                w = self.logo_width
            ),
            &self.logo_link,
        );
        let table = |inner: String| {
            format!(
                "<table cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"margin-top:18px;font-family:{font};font-size:13px;line-height:1.45;color:#333;border-collapse:collapse;\">{inner}</table>"
            )
        };
        match (self.layout, self.logo.is_some()) {
            (Layout::LogoLeft, true) => table(format!(
                "<tr><td valign=\"middle\" style=\"padding:0 14px 0 0;\">{logo}</td><td valign=\"middle\" style=\"border-left:2px solid {accent};padding:2px 0 2px 14px;\">{details}</td></tr>"
            )),
            (Layout::LogoTop, true) => table(format!(
                "<tr><td style=\"padding:0 0 10px 0;\">{logo}</td></tr><tr><td style=\"border-top:2px solid {accent};padding-top:10px;\">{details}</td></tr>"
            )),
            _ => table(format!("<tr><td style=\"border-top:2px solid {accent};padding-top:10px;\">{details}</td></tr>")),
        }
    }

    /// Two columns (name and title | p: e: w:) over two logos, then a
    /// follow line. Modelled on a classic agency signature card.
    fn two_logos_html(
        &self,
        font: &str,
        accent: &str,
        src: &dyn Fn(u8) -> String,
        linked: &dyn Fn(String, &str) -> String,
    ) -> String {
        const RULE: &str = "border-left:1px solid #e6e6e6;";
        let w1 = self.logo_width.max(140);
        let base = format!("font-family:{font};color:#000000;");
        let mut left = String::new();
        if !self.name.trim().is_empty() {
            left.push_str(&format!("<div style=\"font-size:13px;font-weight:bold;{base}\">{}</div>", esc(self.name.trim())));
        }
        if !self.title.trim().is_empty() {
            left.push_str(&format!("<div style=\"font-size:12px;{base}\">{}</div>", esc(self.title.trim())));
        }
        if !self.company.trim().is_empty() && self.follow_line().is_none() {
            left.push_str(&format!("<div style=\"font-size:12px;{base}\">{}</div>", esc(self.company.trim())));
        }
        let mut right = String::new();
        let row = |k: &str, v: String| format!("<div style=\"white-space:nowrap;font-size:12px;{base}\">{k}: {v}</div>");
        if !self.phone.trim().is_empty() {
            right.push_str(&row("p", esc(self.phone.trim())));
        }
        if !self.email.trim().is_empty() {
            let e = esc(self.email.trim());
            right.push_str(&row("e", format!("<a href=\"mailto:{e}\" style=\"text-decoration:none;color:#000000;\">{e}</a>")));
        }
        if !self.website.trim().is_empty() {
            let w = self.website.trim();
            right.push_str(&row(
                "w",
                format!("<a href=\"{}\" style=\"text-decoration:none;color:#000000;\">{}</a>", esc(&full_url(w)), esc(w)),
            ));
        }
        for l in self.note.lines().filter(|l| !l.trim().is_empty()) {
            right.push_str(&format!("<div style=\"font-size:11px;color:#666666;font-family:{font};\">{}</div>", esc(l)));
        }
        let img = |n: u8, w: u32| {
            format!(
                "<img src=\"{}\" width=\"{w}\" alt=\"\" border=\"0\" style=\"display:block;width:{w}px;height:auto;border:0;\">",
                src(n)
            )
        };
        let mut out = format!(
            "<table cellpadding=\"0\" cellspacing=\"0\" border=\"0\" style=\"margin-top:18px;border-collapse:collapse;{base}\">\
             <tr><td width=\"{w1}\" style=\"vertical-align:middle;width:{w1}px;padding:0 16px 0 0;\">{left}</td>\
             <td style=\"vertical-align:middle;padding:0 0 0 16px;{RULE}\">{right}</td></tr>"
        );
        if self.logo.is_some() || self.logo2.is_some() {
            let l1 = if self.logo.is_some() { linked(img(1, self.logo_width), &self.logo_link) } else { String::new() };
            let l2 = if self.logo2.is_some() { linked(img(2, self.logo2_width), &self.logo2_link) } else { String::new() };
            out.push_str(&format!(
                "<tr><td width=\"{w1}\" style=\"vertical-align:bottom;width:{w1}px;padding:12px 16px 0 0;\">{l1}</td>\
                 <td style=\"vertical-align:bottom;padding:12px 0 3px 16px;{RULE}\">{l2}</td></tr>"
            ));
        }
        if let Some((lead, label)) = self.follow_line() {
            out.push_str(&format!(
                "<tr><td colspan=\"2\" style=\"padding:8px 0 0 0;font-size:11px;{base}\"><span style=\"color:#888888;\">{}</span>&nbsp;&nbsp;\
                 <a href=\"{}\" target=\"_blank\" style=\"color:{accent};text-decoration:none;\">{}</a></td></tr>",
                esc(&lead),
                esc(&full_url(&self.social_url)),
                esc(&label)
            ));
        }
        out.push_str("</table>");
        out
    }
}

fn data_uri(logo: &Option<(String, Vec<u8>)>) -> String {
    use base64::Engine;
    match logo {
        Some((mime, bytes)) => format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)),
        None => String::new(),
    }
}

fn full_url(s: &str) -> String {
    let s = s.trim();
    if s.contains("://") || s.starts_with("mailto:") { s.to_string() } else { format!("https://{s}") }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Image type from a file name, for logos.
pub fn image_mime(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_html() {
        let mut s = Signature::new("work");
        s.name = "Alex Doe".into();
        s.email = "alex@example.com".into();
        s.title = "Director of R&D".into();
        s.company = "Acme".into();
        s.phone = "(201) 555-0100".into();
        s.website = "example.com".into();
        let t = s.text();
        assert!(t.starts_with("-- \n"));
        assert!(t.contains("Director of R&D, Acme"));
        let h = s.html("Menlo", false);
        assert!(h.contains("Director of R&amp;D"));
        assert!(h.contains("href=\"https://example.com\""));
        assert!(h.contains("href=\"tel:2015550100\""));
        assert!(h.contains("mailto:"));
        assert!(!h.contains("<img"), "no logo, no image");
        s.layout = Layout::LogoLeft;
        s.logo = Some(("image/png".into(), vec![1, 2, 3]));
        assert!(s.html("Menlo", false).contains("src=\"cid:sig-logo\""));
        // Two logos: both inline, both linked, follow line, own font.
        s.layout = Layout::TwoLogos;
        s.logo2 = Some(("image/png".into(), vec![4, 5]));
        s.logo_link = "example.com/team".into();
        s.social_url = "https://www.linkedin.com/company/acme/".into();
        s.font = "Verdana, Geneva, sans-serif".into();
        let h = s.html("Menlo", false);
        assert!(h.contains("cid:sig-logo2") && h.contains("href=\"https://example.com/team\""), "{h}");
        assert!(h.contains("Follow Acme on") && h.contains("Verdana") && !h.contains("Menlo"));
        assert!(s.html("Menlo", true).contains("data:image/png;base64,"));
        assert!(s.text().contains("Follow Acme on LinkedIn"));
    }
}

/// Read an HTML signature made elsewhere (Thunderbird, Apple Mail, a
/// generator site) into a signature: texts, up to two embedded logos with
/// their links, the font and a "Follow … on …" line.
pub fn import_html(html: &str, label: &str) -> Signature {
    use base64::Engine;
    let mut s = Signature::new(label);
    s.email.clear();
    // Images: data URIs, their width and the link around them.
    let mut logos: Vec<(String, Vec<u8>, u32, String)> = Vec::new();
    let mut pos = 0;
    while let Some(i) = html[pos..].find("<img").map(|i| i + pos) {
        let end = html[i..].find('>').map(|e| e + i).unwrap_or(html.len());
        let tag = &html[i..end];
        pos = end;
        let Some(src) = attr(tag, "src") else { continue };
        let Some(rest) = src.strip_prefix("data:") else { continue };
        let Some((mime, b64)) = rest.split_once(";base64,") else { continue };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(b64.trim()) else { continue };
        let width = attr(tag, "width").and_then(|w| w.trim_end_matches("px").parse().ok()).unwrap_or(120);
        // The nearest <a href> opened before this image and not yet closed.
        let before = &html[..i];
        let link = match (before.rfind("<a "), before.rfind("</a>")) {
            (Some(a), close) if close.map_or(true, |c| c < a) => {
                let tag_end = before[a..].find('>').map(|e| e + a).unwrap_or(before.len());
                attr(&before[a..tag_end], "href").unwrap_or_default()
            }
            _ => String::new(),
        };
        logos.push((mime.to_string(), bytes, width, link));
    }
    let mut logos = logos.into_iter();
    if let Some((m, b, w, l)) = logos.next() {
        s.logo = Some((m, b));
        s.logo_width = w;
        s.logo_link = l;
        s.layout = Layout::LogoLeft;
    }
    if let Some((m, b, w, l)) = logos.next() {
        s.logo2 = Some((m, b));
        s.logo2_width = w;
        s.logo2_link = l;
        s.layout = Layout::TwoLogos;
    }
    if s.logo.is_none() {
        s.layout = Layout::Stacked;
    }
    // Font: the first font-family declared.
    if let Some(i) = html.find("font-family:") {
        let f = html[i + 12..].split([';', '"']).next().unwrap_or("").trim();
        if !f.is_empty() {
            s.font = f.replace('"', "'");
        }
    }
    // Follow line: "Follow X on" then the link that follows.
    if let Some(i) = html.find("Follow ") {
        let rest = &html[i..];
        if let Some(a) = rest.find("<a ") {
            let tag_end = rest[a..].find('>').map(|e| e + a).unwrap_or(rest.len());
            let tag = &rest[a..tag_end];
            s.social_url = attr(tag, "href").unwrap_or_default();
            s.social_label = strip_tags(&rest[tag_end + 1..rest[tag_end..].find("</a>").map(|e| e + tag_end).unwrap_or(rest.len())]);
            if let Some(c) = css(tag, "color").and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok()) {
                s.accent = c;
            }
        }
        let lead = strip_tags(&rest[..rest.find("<a ").unwrap_or(0)]);
        if let Some(who) = lead.strip_prefix("Follow ").and_then(|l| l.trim().strip_suffix(" on")) {
            s.company = who.trim().to_string();
        }
    }
    // Text lines: name first, then title; p:/e:/w: rows are contact details.
    let text = strip_tags(
        &html
            .replace("</div>", "\n")
            .replace("<br>", "\n")
            .replace("<br/>", "\n")
            .replace("</td>", "\n")
            .replace("</p>", "\n"),
    );
    let mut plain = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let lower = line.to_lowercase();
        let value = |l: &str| l.split_once(':').map(|(_, v)| v.trim().to_string()).unwrap_or_default();
        if lower.starts_with("p:") || lower.starts_with("phone:") || lower.starts_with("t:") || lower.starts_with("m:") {
            s.phone = value(line);
        } else if lower.starts_with("e:") || lower.starts_with("email:") {
            s.email = value(line);
        } else if lower.starts_with("w:") || lower.starts_with("web:") {
            s.website = value(line);
        } else if line.starts_with("Follow ") {
            continue;
        } else {
            plain.push(line.to_string());
        }
    }
    let mut plain = plain.into_iter();
    if let Some(n) = plain.next() {
        s.name = n;
    }
    if let Some(t) = plain.next() {
        s.title = t;
    }
    s.note = plain.collect::<Vec<_>>().join("\n");
    s
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    let v = &tag[i..i + tag[i..].find('"')?];
    Some(v.replace("&amp;", "&"))
}

fn css(tag: &str, prop: &str) -> Option<String> {
    let style = attr(tag, "style")?;
    style
        .split(';')
        .filter_map(|d| d.split_once(':'))
        .find(|(k, _)| k.trim() == prop)
        .map(|(_, v)| v.trim().to_string())
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&nbsp;", " ").replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").trim().to_string()
}
