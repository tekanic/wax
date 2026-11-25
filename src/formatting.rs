//! Markdown formatting in the composer: the toolbar, its shortcuts and the
//! preview toggle. Every change is a plain text edit, so ⌘Z undoes it.

use super::*;
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Bold,
    Italic,
    Strike,
    Link,
    Code,
    Bullets,
    Numbers,
    Quote,
    Table,
}

impl Format {
    fn label(self) -> &'static str {
        match self {
            Format::Bold => "B",
            Format::Italic => "I",
            Format::Strike => "S",
            Format::Link => "Link",
            Format::Code => "Code",
            Format::Bullets => "• List",
            Format::Numbers => "1. List",
            Format::Quote => "Quote",
            Format::Table => "Table",
        }
    }

    fn keys(self) -> &'static str {
        match self {
            Format::Bold => "⌘B",
            Format::Italic => "⌘I",
            Format::Strike => "⌘⇧X",
            Format::Link => "⌘K",
            Format::Code => "⌘E",
            Format::Bullets => "⌘⇧8",
            Format::Numbers => "⌘⇧7",
            Format::Quote => "⌘'",
            Format::Table => "⌥⌘T  ·  paste from a spreadsheet, tab between cells",
        }
    }
}

/// Byte range where `old` and `new` differ (on char boundaries), so an edit
/// replaces only that and the editor's undo history stays meaningful.
fn diff_range(old: &str, new: &str) -> (Range<usize>, Range<usize>) {
    let mut pre = old.bytes().zip(new.bytes()).take_while(|(a, b)| a == b).count();
    while !old.is_char_boundary(pre) || !new.is_char_boundary(pre) {
        pre -= 1;
    }
    let max_suf = old.len().min(new.len()) - pre;
    let mut suf = old.bytes().rev().zip(new.bytes().rev()).take_while(|(a, b)| a == b).count().min(max_suf);
    while !old.is_char_boundary(old.len() - suf) || !new.is_char_boundary(new.len() - suf) {
        suf -= 1;
    }
    (pre..old.len() - suf, pre..new.len() - suf)
}

/// Put `block` on its own lines at `at` (blank line before and after, so
/// Markdown sees it as a block). Returns the text and where the block starts.
pub fn insert_block(text: &str, at: usize, block: &str) -> (String, usize) {
    let at = text[..at.min(text.len())].rfind('\n').map(|i| i + 1).unwrap_or(0).max(at.min(text.len()));
    let before = &text[..at];
    let after = &text[at..];
    let lead = if before.is_empty() || before.ends_with("\n\n") {
        ""
    } else if before.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    let trail = if after.starts_with("\n\n") || after.is_empty() { "\n" } else if after.starts_with('\n') { "\n" } else { "\n\n" };
    let start = at + lead.len();
    (format!("{before}{lead}{block}{trail}{after}"), start)
}

impl MailApp {
    /// Replace the body with `new` as one undoable edit, then select.
    pub fn apply_body_edit(&mut self, new: String, select: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        let body = c.body.clone();
        body.update(cx, |s, cx| {
            let old = s.value().to_string();
            let (old_r, new_r) = diff_range(&old, &new);
            if !old_r.is_empty() || !new_r.is_empty() {
                s.set_selected_range(old_r, cx);
                s.replace(new[new_r].to_string(), window, cx);
            }
            s.set_selected_range(select, cx);
        });
        cx.notify();
    }

    /// ⌘V in the message: a copied spreadsheet range becomes a table.
    pub fn smart_paste(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let tsv = cx.read_from_clipboard().and_then(|i| i.text()).and_then(|t| table::from_tsv(&t));
        let Some((c, tbl)) = self.composer.as_ref().zip(tsv) else {
            window.dispatch_action(Box::new(gpui_kit::component::input::Paste), cx);
            return;
        };
        let (old, sel) = {
            let s = c.body.read(cx);
            (s.value().to_string(), s.selected_range())
        };
        // A selection is replaced by the table.
        let base = format!("{}{}", &old[..sel.start.min(old.len())], &old[sel.end.min(old.len())..]);
        let (out, start) = insert_block(&base, sel.start, &tbl);
        let end = start + tbl.len();
        self.apply_body_edit(out, end..end, window, cx);
        self.status = "Pasted as a table: tab moves between cells".into();
        self.status_is_error = false;
    }

    /// Tab in a table: tidy it and go to the next cell. Elsewhere, indent.
    pub fn table_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        let moved = self.composer.as_ref().and_then(|c| {
            let s = c.body.read(cx);
            let sel = s.selected_range();
            table::tab(&s.value(), sel.end, back)
        });
        match moved {
            Some((out, sel)) => self.apply_body_edit(out, sel, window, cx),
            None if back => window.dispatch_action(Box::new(gpui_kit::component::input::OutdentInline), cx),
            None => window.dispatch_action(Box::new(gpui_kit::component::input::IndentInline), cx),
        }
    }

    pub fn format(&mut self, f: Format, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        if c.preview {
            return;
        }
        // A copied URL becomes the link target.
        let clip = cx
            .read_from_clipboard()
            .and_then(|i| i.text())
            .map(|t| t.trim().to_string())
            .filter(|t| (t.starts_with("http://") || t.starts_with("https://") || t.starts_with("mailto:")) && !t.contains(char::is_whitespace));
        let body = c.body.clone();
        body.update(cx, |s, cx| {
            let old = s.value().to_string();
            let sel = s.selected_range();
            let sel = sel.start.min(old.len())..sel.end.min(old.len());
            let (new, select) = match f {
                Format::Bold => md::toggle_wrap(&old, sel, "**"),
                Format::Italic => md::toggle_wrap(&old, sel, "_"),
                Format::Strike => md::toggle_wrap(&old, sel, "~~"),
                Format::Code if old[sel.clone()].contains('\n') => {
                    let block = format!("```\n{}\n```", old[sel.clone()].trim_end_matches('\n'));
                    let out = format!("{}{block}{}", &old[..sel.start], &old[sel.end..]);
                    let end = sel.start + block.len();
                    (out, end..end)
                }
                Format::Code => md::toggle_wrap(&old, sel, "`"),
                Format::Bullets => md::toggle_line_prefix(&old, sel, "- "),
                Format::Numbers => md::toggle_line_prefix(&old, sel, "1. "),
                Format::Quote => md::toggle_line_prefix(&old, sel, "> "),
                Format::Table => {
                    let (out, at) = insert_block(&old, sel.end, &table::blank(3, 2));
                    // Select the first header so typing renames it.
                    (out, at + 2..at + 2 + "Column 1".len())
                }
                Format::Link => {
                    let label = if sel.is_empty() { "link text".to_string() } else { old[sel.clone()].to_string() };
                    let url = clip.clone().unwrap_or_else(|| "https://".to_string());
                    let link = format!("[{label}]({url})");
                    let out = format!("{}{link}{}", &old[..sel.start], &old[sel.end..]);
                    // Select what still needs typing: the text, or the URL.
                    let select = if sel.is_empty() {
                        sel.start + 1..sel.start + 1 + label.len()
                    } else if clip.is_none() {
                        let at = sel.start + label.len() + 3;
                        at..at + url.len()
                    } else {
                        let end = sel.start + link.len();
                        end..end
                    };
                    (out, select)
                }
            };
            let (old_r, new_r) = diff_range(&old, &new);
            if !old_r.is_empty() || !new_r.is_empty() {
                s.set_selected_range(old_r, cx);
                s.replace(new[new_r].to_string(), window, cx);
            }
            s.set_selected_range(select, cx);
        });
        body.read(cx).focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub fn toggle_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &mut self.composer else { return };
        c.preview = !c.preview;
        if c.preview {
            c.preview_focus.focus(window, cx);
        } else {
            c.body.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// HTML to show in the web view while previewing or reviewing a send.
    pub fn compose_html(&self, cx: &App) -> Option<((String, String), String)> {
        let draft: Draft = if self.pending_send.is_some() || self.after_send.is_some() {
            self.review_draft.clone()?
        } else {
            self.composer.as_ref().filter(|c| c.preview)?.draft(cx)
        };
        let sig = self.signature_by_id(draft.signature);
        let (_, html) = compose::render_bodies(&draft, sig, true);
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        html.hash(&mut h);
        Some((("compose".into(), h.finish().to_string()), html))
    }

    pub fn render_format_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let preview = self.composer.as_ref().is_some_and(|c| c.preview);
        let mono = self.composer.as_ref().is_some_and(|c| c.mono);
        let button = |id: &'static str, f: Format| {
            let label = div()
                .child(f.label())
                .when(f == Format::Bold, |d| d.font_weight(FontWeight::BOLD))
                .when(f == Format::Italic, |d| d.italic())
                .when(f == Format::Strike, |d| d.line_through());
            div()
                .id(id)
                .h(px(26.))
                .px_2()
                .flex()
                .items_center()
                .rounded_md()
                .text_sm()
                .text_color(if preview { pal().muted } else { pal().subtle })
                .when(!preview, |d| d.cursor_pointer().hover(|d| d.bg(pal().popover_hover).text_color(pal().text)))
                .tooltip(move |window, cx| Tooltip::new(f.keys()).build(window, cx))
                .on_click(cx.listener(move |this, _, window, cx| this.format(f, window, cx)))
                .child(label)
        };
        let sep = || div().w(px(1.)).h(px(16.)).mx_1().bg(pal().border);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_0p5()
            .child(button("fmt-bold", Format::Bold))
            .child(button("fmt-italic", Format::Italic))
            .child(button("fmt-strike", Format::Strike))
            .child(sep())
            .child(button("fmt-link", Format::Link))
            .child(button("fmt-code", Format::Code))
            .child(sep())
            .child(button("fmt-bullets", Format::Bullets))
            .child(button("fmt-numbers", Format::Numbers))
            .child(button("fmt-quote", Format::Quote))
            .child(button("fmt-table", Format::Table))
            .child(div().flex_1())
            .child(
                div()
                    .id("fmt-mono")
                    .h(px(26.))
                    .px_2()
                    .mr_1()
                    .flex()
                    .items_center()
                    .rounded_md()
                    .text_xs()
                    .font_family(if mono { "Menlo" } else { "Helvetica" })
                    .cursor_pointer()
                    .text_color(pal().subtle)
                    .hover(|d| d.bg(pal().popover_hover).text_color(pal().text))
                    .tooltip(|window, cx| Tooltip::new("Font for this message  ⌘⇧M").build(window, cx))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_mono(cx)))
                    .child(if mono { "Monospaced" } else { "Proportional" }),
            )
            .child(div().text_xs().text_color(pal().muted).mr_2().child("Markdown"))
            .child(
                div()
                    .id("fmt-preview")
                    .h(px(26.))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .when(preview, |d| d.bg(pal().accent).text_color(pal().on_accent))
                    .when(!preview, |d| d.text_color(pal().subtle).hover(|d| d.bg(pal().popover_hover).text_color(pal().text)))
                    .tooltip(|window, cx| Tooltip::new("⌘⇧P").build(window, cx))
                    .on_click(cx.listener(|this, _, window, cx| this.toggle_preview(window, cx)))
                    .child(if preview { "Edit" } else { "Preview" }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{diff_range, insert_block};

    #[test]
    fn blocks_get_their_own_lines() {
        let (t, at) = insert_block("Hi Jim,\nsee below", 7, "| a |");
        assert_eq!(t, "Hi Jim,\n\n| a |\n\nsee below");
        assert_eq!(&t[at..at + 5], "| a |");
        let (t, _) = insert_block("", 0, "| a |");
        assert_eq!(t, "| a |\n");
    }

    #[test]
    fn finds_the_changed_span() {
        let (o, n) = diff_range("make this bold", "make **this** bold");
        assert_eq!(o, 5..9);
        assert_eq!(&"make **this** bold"[n], "**this**");
        let (o, n) = diff_range("héllo", "héllo!");
        assert_eq!((o, n), (6..6, 6..7));
    }
}

impl MailApp {
    /// This message only; the default lives in Settings.
    pub fn toggle_mono(&mut self, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.composer {
            c.mono = !c.mono;
            self.schedule_autosave(cx);
            cx.notify();
        }
    }

    pub fn set_signature(&mut self, id: Option<i64>, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.composer {
            c.signature = id;
            self.schedule_autosave(cx);
            cx.notify();
        }
    }

    /// ⌘⇧S: None → each signature in turn → None.
    pub fn cycle_signature(&mut self, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        let account = if c.account.is_empty() { self.account.to_string() } else { c.account.clone() };
        let ids: Vec<Option<i64>> =
            std::iter::once(None).chain(self.signatures_for(&account).iter().map(|s| Some(s.id))).collect();
        let at = ids.iter().position(|i| *i == c.signature).unwrap_or(0);
        let next = ids[(at + 1) % ids.len()];
        self.set_signature(next, cx);
    }

    /// Under the editor: which signature goes out, switchable in one click.
    pub fn render_signature_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(c) = &self.composer else { return div() };
        let pill = |id: SharedString, label: SharedString, on: bool, accent: Option<u32>| {
            div()
                .id(id)
                .h(px(24.))
                .px_2p5()
                .flex()
                .items_center()
                .gap_1p5()
                .rounded_full()
                .text_xs()
                .cursor_pointer()
                .border_1()
                .when(on, |d| d.bg(pal().selected).border_color(pal().accent).text_color(pal().text))
                .when(!on, |d| d.border_color(pal().border).text_color(pal().subtle).hover(|d| d.bg(pal().hover)))
                .children(accent.map(|a| div().size(px(7.)).rounded_full().bg(rgb(a))))
                .child(label)
        };
        let mut pills = div().flex().flex_wrap().items_center().gap_1p5().child(
            pill("sig-pick-none".into(), "None".into(), c.signature.is_none(), None)
                .on_click(cx.listener(|this, _, _, cx| this.set_signature(None, cx))),
        );
        // Only the signatures of the account the message goes out from.
        let account = if c.account.is_empty() { self.account.to_string() } else { c.account.clone() };
        let mine = self.signatures_for(&account);
        let none_yet = mine.is_empty();
        for s in mine {
            let id = s.id;
            pills = pills.child(
                pill(format!("sig-pick-{id}").into(), s.label.clone().into(), c.signature == Some(id), Some(s.accent))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_signature(Some(id), cx))),
            );
        }
        pills = pills.child(
            div()
                .id("sig-edit")
                .ml_1()
                .text_xs()
                .text_color(pal().accent)
                .cursor_pointer()
                .hover(|d| d.underline())
                .on_click(cx.listener(move |this, _, w, cx| {
                    let current = this.composer.as_ref().and_then(|c| c.signature);
                    let account = this
                        .composer
                        .as_ref()
                        .map(|c| c.account.clone())
                        .filter(|a| !a.is_empty())
                        .unwrap_or_else(|| this.account.to_string());
                    this.open_settings(settings::Pane::Signatures, w, cx);
                    this.show_signatures_of(account, w, cx);
                    if let Some(id) = current {
                        this.edit_signature(id, w, cx);
                    }
                }))
                .child(if none_yet { "Design a signature…" } else { "Edit…" }),
        );
        let card = self.signature_by_id(c.signature).filter(|_| !c.preview).map(|s| self.signature_card(s));
        div()
            .flex_none()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(div().text_xs().text_color(pal().muted).child("Signature"))
                    .child(pills)
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(pal().muted).font_family("Menlo").child("⌘⇧S")),
            )
            .children(card.map(|card| div().px_4().py_3().rounded_md().bg(rgb(0xffffff)).child(card)))
    }
}

impl MailApp {
    /// Switch the From address; the signature follows that address's
    /// default.
    pub fn set_from(&mut self, from: Option<(String, String)>, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        let kind = c.kind;
        let account = if c.account.is_empty() { self.account.to_string() } else { c.account.clone() };
        let who = self.identity_in(&account, from.as_ref());
        let sig = self.default_signature(&who, kind);
        if let Some(c) = &mut self.composer {
            c.from = if who.is_main() { None } else { Some((who.name.clone(), who.email.clone())) };
            c.signature = sig;
        }
        self.schedule_autosave(cx);
        cx.notify();
    }

    /// Send from another account (or one of its From addresses). A draft
    /// already saved in the old account's Drafts stays there: nothing is
    /// deleted from an account the message has left.
    pub fn set_sender(&mut self, account: String, from: Option<(String, String)>, cx: &mut Context<Self>) {
        let here = self.account.to_string();
        if let Some(c) = &mut self.composer {
            let current = if c.account.is_empty() { here } else { c.account.clone() };
            if !current.eq_ignore_ascii_case(&account) {
                c.account = account;
                c.server_draft = None;
            }
        }
        self.set_from(from, cx);
    }

    /// The From row: every account's address and its extra From addresses;
    /// only shown once there's more than one to pick from.
    pub fn render_from_row(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let c = self.composer.as_ref()?;
        let account = if c.account.is_empty() { self.account.to_string() } else { c.account.clone() };
        let mut all = Vec::new();
        for a in &self.accounts {
            all.extend(self.identities_for(&a.email));
        }
        if all.is_empty() {
            all = self.identities_for(&account);
        }
        if all.len() < 2 {
            return None;
        }
        let current = self.identity_in(&account, c.from.as_ref());
        let mut pills = div().flex().flex_wrap().items_center().gap_1p5();
        for who in all {
            let on = who.account.eq_ignore_ascii_case(&account) && who.email.eq_ignore_ascii_case(&current.email);
            let from = Some((who.name.clone(), who.email.clone()));
            let owner = who.account.clone();
            pills = pills.child(
                div()
                    .id(SharedString::from(format!("from-{}-{}", who.account, who.email)))
                    .h(px(24.))
                    .px_2p5()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .text_xs()
                    .cursor_pointer()
                    .border_1()
                    .when(on, |d| d.bg(pal().selected).border_color(pal().accent).text_color(pal().text))
                    .when(!on, |d| d.border_color(pal().border).text_color(pal().subtle).hover(|d| d.bg(pal().hover)))
                    .on_click(cx.listener(move |this, _, _, cx| this.set_sender(owner.clone(), from.clone(), cx)))
                    .child(who.email.clone()),
            );
        }
        Some(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_3()
                .child(div().w(px(56.)).flex_none().text_xs().text_color(pal().muted).child("From"))
                .child(pills),
        )
    }
}
