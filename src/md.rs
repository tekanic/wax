//! Messages are written in Markdown and sent as both:
//! - text/plain: the Markdown source, which already reads well as text;
//! - text/html: GitHub-flavored Markdown rendered with inline styles (mail
//!   clients drop <style> blocks), keeping every line break you typed.

const SANS: &str = "-apple-system,BlinkMacSystemFont,'Segoe UI',Helvetica,Arial,sans-serif";
const MONO: &str = "'SF Mono',Menlo,Consolas,'Liberation Mono','Courier New',monospace";

pub fn font(mono: bool) -> &'static str {
    if mono { MONO } else { SANS }
}

/// Markdown → an HTML fragment ready to be an email body.
pub fn to_email_html(source: &str, mono: bool) -> String {
    let (family, size) = if mono { (MONO, 13) } else { (SANS, 14) };
    let options = markdown::Options::gfm();
    let html = markdown::to_html_with_options(source, &options).unwrap_or_else(|_| escape_text(source));
    let html = keep_line_breaks(&html);
    format!(
        "<div style=\"font-family:{family};font-size:{size}px;line-height:1.5;color:#1f1f1f;\">{}</div>",
        style_tags(&html)
    )
}

/// Email isn't a web page: one Enter is a new line, not a space.
fn keep_line_breaks(html: &str) -> String {
    let mut out = String::with_capacity(html.len() + 64);
    let mut in_p = 0usize;
    let mut in_pre = 0usize;
    let mut rest = html;
    while let Some(i) = rest.find(['<', '\n']) {
        let (before, at) = rest.split_at(i);
        out.push_str(before);
        if at.starts_with('\n') {
            if in_p > 0 && in_pre == 0 {
                out.push_str("<br>\n");
            } else {
                out.push('\n');
            }
            rest = &at[1..];
            continue;
        }
        let end = at.find('>').map(|e| e + 1).unwrap_or(at.len());
        let tag = &at[..end];
        let name: String = tag
            .trim_start_matches(['<', '/'])
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        let closing = tag.starts_with("</");
        let counter = match name.as_str() {
            "p" | "li" | "td" | "th" => Some(&mut in_p),
            "pre" => Some(&mut in_pre),
            _ => None,
        };
        if let Some(c) = counter {
            if closing {
                *c = c.saturating_sub(1);
            } else {
                *c += 1;
            }
        }
        out.push_str(tag);
        rest = &at[end..];
    }
    out.push_str(rest);
    // A <br> right before a closing tag adds a stray blank line.
    out.replace("<br>\n</li>", "\n</li>")
}

fn style_tags(html: &str) -> String {
    const STYLES: &[(&str, &str)] = &[
        ("<p>", "<p style=\"margin:0 0 12px 0;\">"),
        ("<h1>", "<h1 style=\"font-size:22px;margin:16px 0 8px 0;\">"),
        ("<h2>", "<h2 style=\"font-size:18px;margin:16px 0 8px 0;\">"),
        ("<h3>", "<h3 style=\"font-size:16px;margin:14px 0 6px 0;\">"),
        ("<ul>", "<ul style=\"margin:0 0 12px 0;padding-left:24px;\">"),
        ("<ol>", "<ol style=\"margin:0 0 12px 0;padding-left:24px;\">"),
        (
            "<blockquote>",
            "<blockquote style=\"margin:0 0 12px 0;padding:0 0 0 12px;border-left:3px solid #d0d0d7;color:#555;\">",
        ),
        (
            "<pre>",
            "<pre style=\"margin:0 0 12px 0;padding:10px 12px;background:#f4f4f6;border-radius:6px;font-size:13px;overflow:auto;\">",
        ),
        ("<code>", "<code style=\"font-family:Menlo,Consolas,monospace;font-size:13px;background:#f1f1f4;padding:1px 4px;border-radius:4px;\">"),
        ("<table>", "<table style=\"border-collapse:collapse;margin:0 0 12px 0;\">"),
        ("<th>", "<th style=\"border:1px solid #d0d0d7;padding:4px 8px;text-align:left;background:#f4f4f6;\">"),
        ("<th align=", "<th style=\"border:1px solid #d0d0d7;padding:4px 8px;background:#f4f4f6;\" align="),
        ("<td>", "<td style=\"border:1px solid #d0d0d7;padding:4px 8px;\">"),
        ("<td align=", "<td style=\"border:1px solid #d0d0d7;padding:4px 8px;\" align="),
        ("<hr />", "<hr style=\"border:none;border-top:1px solid #d0d0d7;margin:16px 0;\" />"),
        ("<a href=", "<a style=\"color:#1a62d6;\" href="),
    ];
    let mut s = html.to_string();
    for (from, to) in STYLES {
        s = s.replace(from, to);
    }
    // Code inside a block keeps the block's background, not a second chip.
    s.replace(
        "<pre style=\"margin:0 0 12px 0;padding:10px 12px;background:#f4f4f6;border-radius:6px;font-size:13px;overflow:auto;\"><code style=\"font-family:Menlo,Consolas,monospace;font-size:13px;background:#f1f1f4;padding:1px 4px;border-radius:4px;\"",
        "<pre style=\"margin:0 0 12px 0;padding:10px 12px;background:#f4f4f6;border-radius:6px;font-size:13px;overflow:auto;\"><code style=\"font-family:Menlo,Consolas,monospace;font-size:13px;\"",
    )
}

fn escape_text(s: &str) -> String {
    format!("<p>{}</p>", s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;"))
}

/// Wrap the selection in `mark` (e.g. `**`), or unwrap it if it already is.
/// Returns the new text and the byte range to select afterwards.
pub fn toggle_wrap(text: &str, sel: std::ops::Range<usize>, mark: &str) -> (String, std::ops::Range<usize>) {
    let m = mark.len();
    let (a, b) = (sel.start.min(text.len()), sel.end.min(text.len()));
    let wrapped = a >= m && text.get(a - m..a) == Some(mark) && text.get(b..b + m) == Some(mark);
    if wrapped {
        let out = format!("{}{}{}", &text[..a - m], &text[a..b], &text[b + m..]);
        (out, a - m..b - m)
    } else {
        let out = format!("{}{mark}{}{mark}{}", &text[..a], &text[a..b], &text[b..]);
        (out, a + m..b + m)
    }
}

/// Put `prefix` at the start of every line the selection touches (or take
/// it off if all of them have it). For lists and quotes.
pub fn toggle_line_prefix(text: &str, sel: std::ops::Range<usize>, prefix: &str) -> (String, std::ops::Range<usize>) {
    let start = text[..sel.start.min(text.len())].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let end = text[sel.end.min(text.len())..].find('\n').map(|i| sel.end + i).unwrap_or(text.len());
    let block = &text[start..end];
    let all = block.lines().all(|l| l.starts_with(prefix));
    let new_block: Vec<String> = block
        .split('\n')
        .map(|l| if all { l.strip_prefix(prefix).unwrap_or(l).to_string() } else { format!("{prefix}{l}") })
        .collect();
    let new_block = new_block.join("\n");
    let out = format!("{}{}{}", &text[..start], new_block, &text[end..]);
    let new_end = start + new_block.len();
    (out, if sel.is_empty() { new_end..new_end } else { start..new_end })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_and_keeps_breaks() {
        let html = to_email_html("Hi Sam,\nThanks for **this**.\n\n- one\n- two\n\nSee https://example.com", false);
        assert!(html.contains("Hi Sam,<br>"), "{html}");
        assert!(html.contains("<strong>this</strong>"));
        assert!(html.contains("<li>one</li>"));
        assert!(html.contains("href=\"https://example.com\""));
    }

    #[test]
    fn code_blocks_keep_newlines() {
        let html = to_email_html("```\na\nb\n```", false);
        assert!(!html.contains("a<br>"), "{html}");
    }

    #[test]
    fn quotes_render() {
        let html = to_email_html("On Mon, Sam wrote:\n> first\n> second", false);
        assert!(html.contains("<blockquote"));
        assert!(html.contains("first<br>"), "{html}");
    }

    #[test]
    fn wrap_and_unwrap() {
        let (t, r) = toggle_wrap("make this bold", 5..9, "**");
        assert_eq!(t, "make **this** bold");
        assert_eq!(&t[r.clone()], "this");
        let (t, _) = toggle_wrap(&t, r, "**");
        assert_eq!(t, "make this bold");
    }

    #[test]
    fn list_prefix() {
        let (t, _) = toggle_line_prefix("a\nb\nc", 0..3, "- ");
        assert_eq!(t, "- a\n- b\nc");
        let (t, _) = toggle_line_prefix(&t, 0..7, "- ");
        assert_eq!(t, "a\nb\nc");
    }
}
