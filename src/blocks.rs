//! Blocks: ready-made pieces of a message (a table, a callout, a link
//! button…) dragged or clicked into the composer.
//!
//! A block is a Markdown file in the blocks folder; its name is the file
//! name. `{{cursor}}` marks where the cursor goes after inserting it. A few
//! starters are written the first time; edit, add or delete files freely.
//! This is the shape a plugin's blocks will take later.

use super::*;
use gpui_kit::component::input::{InputState, TextareaState};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Block {
    pub name: String,
    pub body: String,
    pub shortcut: Option<String>,
    pub file: PathBuf,
}

const CURSOR: &str = "{{cursor}}";

const STARTERS: &[(&str, &str)] = &[
    ("Table", "| {{cursor}}Item | Qty | Notes |\n| ---- | --: | ----- |\n|      |     |       |\n|      |     |       |"),
    ("Callout", "> **Note:** {{cursor}}"),
    ("Link button", "**[{{cursor}}Open it here](https://)**"),
    ("Divider", "---\n\n{{cursor}}"),
    ("Next steps", "**Next steps**\n\n1. {{cursor}}\n2. \n3. "),
    ("Meeting times", "Would any of these work for you?\n\n- {{cursor}}\n- \n- "),
    ("Quote", "---\nshortcut: ;quote\n---\nHi {{first name}},\n\nHere's the quote for {{project}}:\n\n| # | Item | Qty | Price | Amount =Qty*Price |\n| - | ---- | --: | ----: | ----------------: |\n|   | {{item}} | {{qty:1}} | {{price:$0.00}} |  |\n|   | **Total** |  |  |  |\n\n{{cursor}}"),
    ("Thanks", "---\nshortcut: ;ty\n---\nThanks, {{first name}}! {{cursor}}"),
];

pub fn folder() -> PathBuf {
    crate::files::config_dir().join("blocks")
}

pub fn parse(text: &str) -> (Option<String>, String) {
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n").or_else(|| rest.find("\n---").filter(|&e| e + 4 == rest.len())) {
            let shortcut = rest[..end]
                .lines()
                .find_map(|l| l.strip_prefix("shortcut:"))
                .map(|v| v.trim().trim_matches('"').to_string())
                .filter(|v| !v.is_empty());
            let body = rest.get(end + 5..).unwrap_or("");
            return (shortcut, body.trim_end().to_string());
        }
    }
    (None, text.trim_end().to_string())
}

pub fn file_text(shortcut: &str, body: &str) -> String {
    let shortcut = shortcut.trim();
    if shortcut.is_empty() {
        format!("{}\n", body.trim_end())
    } else {
        format!("---\nshortcut: {shortcut}\n---\n{}\n", body.trim_end())
    }
}

fn file_name(name: &str) -> String {
    let clean: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { ' ' } else { c }).collect();
    let clean = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() { "Block".into() } else { clean }
}

fn write_starters(dir: &std::path::Path) {
    let seen_file = dir.join(".starters");
    let seen: Vec<String> = std::fs::read_to_string(&seen_file).unwrap_or_default().lines().map(str::to_string).collect();
    for (name, body) in STARTERS {
        let path = dir.join(format!("{name}.md"));
        if !seen.iter().any(|s| s == name) && !path.exists() {
            std::fs::write(path, body).ok();
        }
    }
    let all: Vec<&str> = STARTERS.iter().map(|(n, _)| *n).collect();
    std::fs::write(seen_file, all.join("\n") + "\n").ok();
}

/// Every block in the folder, starters written on first use.
pub fn load() -> Vec<Block> {
    let dir = folder();
    if std::fs::create_dir_all(&dir).is_ok() {
        write_starters(&dir);
    }
    let mut out: Vec<Block> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .filter_map(|p| {
                    let name = p.file_stem()?.to_string_lossy().to_string();
                    let (shortcut, body) = parse(&std::fs::read_to_string(&p).ok()?);
                    Some(Block { name, body, shortcut, file: p })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|b| b.name.to_lowercase());
    out
}

pub struct BlockEditor {
    pub name: Entity<InputState>,
    pub shortcut: Entity<InputState>,
    pub body: Entity<TextareaState>,
    pub file: Option<PathBuf>,
    pub error: Option<SharedString>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub text: String,
    pub label: String,
}

fn is_money(w: &str) -> bool {
    let t = w.trim_start_matches(['$', '€', '£']);
    w.len() > t.len() && t.chars().any(|c| c.is_ascii_digit()) && t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.')
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

pub fn suggestions(text: &str, first_name: Option<&str>) -> Vec<Suggestion> {
    let open: Vec<(usize, usize)> = text.match_indices("{{").filter_map(|(i, _)| text[i..].find("}}").map(|e| (i, i + e + 2))).collect();
    let inside = |at: usize| open.iter().any(|(a, b)| at >= *a && at < *b);
    let mut out: Vec<Suggestion> = Vec::new();
    let mut push = |text: String, label: &str| {
        if !out.iter().any(|s| s.text == text) {
            let n = out.iter().filter(|s| s.label == label || s.label.starts_with(&format!("{label} "))).count();
            let label = if n == 0 { label.to_string() } else { format!("{label} {}", n + 1) };
            out.push(Suggestion { text, label });
        }
    };
    let words: Vec<(usize, &str)> = text
        .split_whitespace()
        .map(|w| (w.as_ptr() as usize - text.as_ptr() as usize, w.trim_matches(|c: char| ",.;:!?()\"'".contains(c))))
        .filter(|(at, w)| !w.is_empty() && !inside(*at))
        .collect();
    for (i, (_, w)) in words.iter().enumerate() {
        let lower = w.to_lowercase();
        if is_money(w) {
            push(w.to_string(), "amount");
        } else if w.contains('@') && w.contains('.') {
            push(w.to_string(), "email");
        } else if DAYS.contains(&lower.as_str()) {
            push(w.to_string(), "day");
        } else if MONTHS.iter().any(|m| lower.starts_with(m)) && lower.len() >= 3 && lower.chars().all(char::is_alphabetic) {
            if let Some((_, next)) = words.get(i + 1).filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit()) && !n.is_empty()) {
                push(format!("{w} {next}"), "date");
            }
        } else if first_name.is_some_and(|f| f.eq_ignore_ascii_case(w)) {
            push(w.to_string(), "first name");
        }
    }
    out.truncate(4);
    out
}

pub fn make_placeholder(text: &str, sel: std::ops::Range<usize>) -> (String, std::ops::Range<usize>) {
    let chosen = text[sel.clone()].trim();
    let name = if chosen.is_empty() { "name" } else { chosen };
    let token = format!("{{{{{name}}}}}");
    let out = format!("{}{}{}", &text[..sel.start], token, &text[sel.end..]);
    (out, sel.start + 2..sel.start + 2 + name.len())
}

/// What's being dragged.
#[derive(Clone)]
pub struct BlockDrag(pub Block);

/// The chip that follows the pointer while dragging.
pub struct DragChip(SharedString);

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_3()
            .py_1p5()
            .rounded_md()
            .bg(pal().accent)
            .text_color(pal().on_accent)
            .text_sm()
            .shadow_lg()
            .child(self.0.clone())
    }
}

impl MailApp {
    pub fn reload_blocks(&mut self) {
        self.blocks = load();
    }

    /// Insert at `at` (a byte offset in the body), or at the cursor.
    pub fn insert_block(&mut self, block: &Block, at: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        if c.preview {
            self.toggle_preview(window, cx);
        }
        let Some(c) = &self.composer else { return };
        let (old, cursor) = {
            let s = c.body.read(cx);
            (s.value().to_string(), s.selected_range().end)
        };
        let at = at.unwrap_or(cursor);
        let ex = crate::fill::expand(&block.body, &self.smart_values(cx));
        if !ex.slots.is_empty() {
            self.start_fill(block, at, at..at, window, cx);
            return;
        }
        let body = ex.text;
        let mark = ex.end.unwrap_or(body.len());
        let word = body[mark..].find(|c: char| c.is_whitespace() || "|]*_)".contains(c)).unwrap_or(body.len() - mark);
        let (out, start) = formatting::insert_block(&old, at, &body);
        let caret = start + mark;
        let focus = c.body.read(cx).focus_handle(cx);
        self.apply_body_edit(out, caret..caret + word, window, cx);
        focus.focus(window, cx);
    }

    pub fn open_block_editor(&mut self, name: String, shortcut: String, body: String, file: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let line = |placeholder: &'static str, value: String, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                s.set_value(value, window, cx);
                s
            })
        };
        let name = line("Name, e.g. Quote follow-up", name, window, cx);
        let shortcut = line("Shortcut, e.g. ;quote (optional)", shortcut, window, cx);
        let body = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx);
            s.set_value(body, window, cx);
            s
        });
        let focus = name.clone();
        self.block_editor = Some(BlockEditor { name, shortcut, body, file, error: None });
        self.settings.blocks_open = true;
        focus.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub fn save_selection_as_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        let s = c.body.read(cx);
        let text = s.value().to_string();
        let sel = s.selected_range();
        let chosen = text.get(sel.start.min(text.len())..sel.end.min(text.len())).unwrap_or("").trim().to_string();
        let name: String = chosen.split_whitespace().take(4).collect::<Vec<_>>().join(" ").chars().take(40).collect();
        let name = name.trim_end_matches(|c: char| !c.is_alphanumeric()).to_string();
        self.open_block_editor(name, String::new(), chosen, None, window, cx);
    }

    pub fn edit_block(&mut self, block: &Block, window: &mut Window, cx: &mut Context<Self>) {
        self.open_block_editor(block.name.clone(), block.shortcut.clone().unwrap_or_default(), block.body.clone(), Some(block.file.clone()), window, cx);
    }

    pub fn close_block_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.block_editor = None;
        if let Some(c) = &self.composer {
            let f = c.body.read(cx).focus_handle(cx);
            f.focus(window, cx);
        }
        cx.notify();
    }

    fn save_block_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let blocks = self.blocks.clone();
        let Some(e) = &mut self.block_editor else { return };
        let typed = e.name.read(cx).value().trim().to_string();
        let name = file_name(&typed);
        let shortcut = e.shortcut.read(cx).value().trim().to_string();
        let body = e.body.read(cx).value().to_string();
        let problem: Option<String> = if typed.is_empty() {
            Some("Give the block a name".into())
        } else if body.trim().is_empty() {
            Some("The block is empty".into())
        } else if shortcut.contains(char::is_whitespace) {
            Some("A shortcut is one word, like ;quote".into())
        } else if let Some(b) = blocks.iter().find(|b| !shortcut.is_empty() && b.shortcut.as_deref() == Some(shortcut.as_str()) && Some(&b.file) != e.file.as_ref()) {
            Some(format!("{shortcut} already opens \u{201c}{}\u{201d}", b.name))
        } else {
            let path = folder().join(format!("{name}.md"));
            (e.file.as_ref() != Some(&path) && path.exists()).then(|| format!("There's already a block called \u{201c}{name}\u{201d}"))
        };
        if let Some(problem) = problem {
            e.error = Some(problem.into());
            cx.notify();
            return;
        }
        let path = folder().join(format!("{name}.md"));
        if let Err(err) = std::fs::create_dir_all(folder()).and_then(|_| std::fs::write(&path, file_text(&shortcut, &body))) {
            e.error = Some(format!("Could not save: {err}").into());
            cx.notify();
            return;
        }
        if let Some(old) = e.file.take().filter(|o| *o != path) {
            std::fs::remove_file(old).ok();
        }
        self.reload_blocks();
        let how = if shortcut.is_empty() { String::new() } else { format!(": type {shortcut} then tab") };
        self.status = format!("Saved the block \u{201c}{name}\u{201d}{how}").into();
        self.status_is_error = false;
        self.close_block_editor(window, cx);
    }

    fn delete_block_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(file) = self.block_editor.as_ref().and_then(|e| e.file.clone()) else { return };
        let bin = folder().join(".deleted");
        std::fs::create_dir_all(&bin).ok();
        if let Some(n) = file.file_name() {
            std::fs::rename(&file, bin.join(n)).ok();
        }
        self.reload_blocks();
        self.status = "Block deleted (kept in blocks/.deleted)".into();
        self.status_is_error = false;
        self.close_block_editor(window, cx);
    }

    fn editor_placeholder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &self.block_editor else { return };
        let body = e.body.clone();
        body.update(cx, |s, cx| {
            let text = s.value().to_string();
            let sel = s.selected_range();
            let sel = sel.start.min(text.len())..sel.end.min(text.len());
            let (out, select) = make_placeholder(&text, sel);
            s.set_value(out, window, cx);
            s.set_selected_range(select, cx);
            s.focus(window, cx);
        });
        cx.notify();
    }

    fn editor_suggestion(&mut self, sug: Suggestion, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = &self.block_editor else { return };
        let body = e.body.clone();
        body.update(cx, |s, cx| {
            let out = s.value().replace(&sug.text, &format!("{{{{{}}}}}", sug.label));
            s.set_value(out, window, cx);
        });
        cx.notify();
    }

    /// A block dropped on the message: it goes where it was dropped.
    pub fn drop_block(&mut self, drag: &BlockDrag, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        let point = window.mouse_position();
        let body = c.body.clone();
        // The line under the pointer: the last line that starts above it.
        let at = body.update(cx, |s, _| {
            let text = s.value().to_string();
            let starts = std::iter::once(0).chain(text.match_indices('\n').map(|(i, _)| i + 1));
            let mut best = None;
            for o in starts {
                let top = s
                    .range_to_bounds(&(o..o))
                    .or_else(|| s.range_to_bounds(&(o..(o + 1).min(text.len()))))
                    .map(|b| b.origin.y);
                match top {
                    Some(y) if y <= point.y => best = Some(o),
                    Some(_) => break,
                    None => {}
                }
            }
            if std::env::var("GPUI_MAIL_DEBUG").is_ok() {
                eprintln!("drop at {point:?} -> line start {best:?}");
            }
            best
        });
        let block = drag.0.clone();
        self.insert_block(&block, at, window, cx);
    }

    /// The column beside the composer: blocks to drag or click in.
    /// Open or fold the Blocks pane (remembered between launches).
    pub fn set_blocks_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.settings.blocks_open = open;
        self.settings.save().ok();
        cx.notify();
    }

    /// Folded: a thin ribbon with "Blocks" running up it; click to open.
    pub fn render_blocks_ribbon(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("blocks-ribbon")
            .group("blocks-ribbon")
            .w(px(34.))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .bg(pal().list_bg)
            .border_r_1()
            .border_color(pal().border)
            .cursor_pointer()
            .hover(|d| d.bg(pal().hover))
            .tooltip(|window, cx| Tooltip::new("Show blocks").build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| this.set_blocks_open(true, cx)))
            // The label drawing is 14.5 × 38.8 at 12 px; shown at ~17 px.
            .child(
                svg()
                    .path("icons/blocks-label.svg")
                    .w(px(20.3))
                    .h(px(54.3))
                    .text_color(pal().text)
                    .group_hover("blocks-ribbon", |s| s.text_color(pal().accent)),
            )
    }

    fn render_block_editor(&self, e: &BlockEditor, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.composer.as_ref().and_then(|_| self.smart_values(cx).first_name);
        let sugs = suggestions(&e.body.read(cx).value(), first.as_deref());
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px_2p5()
                .h(px(26.))
                .flex()
                .items_center()
                .rounded_md()
                .text_xs()
                .cursor_pointer()
                .when(primary, |d| d.bg(pal().accent).text_color(pal().on_accent))
                .when(!primary, |d| d.bg(pal().button).text_color(pal().text).hover(|d| d.bg(pal().button_hover)))
                .child(label)
        };
        let line = |entity: &Entity<InputState>| Input::new(entity).bg(pal().search_bg).border_color(pal().search_border).text_color(pal().text);
        div()
            .flex()
            .flex_col()
            .gap_2()
            .flex_1()
            .min_h_0()
            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(if e.file.is_some() { "Edit block" } else { "New block" }))
            .child(line(&e.name))
            .child(line(&e.shortcut))
            .child(
                div()
                    .flex_1()
                    .min_h(px(160.))
                    .child(Textarea::new(&e.body).h_full().font_family("Menlo").bg(pal().search_bg).border_color(pal().search_border).text_color(pal().text)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1p5()
                    .child(button("block-make-placeholder", "Make placeholder", false).on_click(cx.listener(|this, _, w, cx| this.editor_placeholder(w, cx))))
                    .children(sugs.into_iter().enumerate().map(|(i, sg)| {
                        let label = format!("{} \u{2192} {{{{{}}}}}", sg.text, sg.label);
                        div()
                            .id(("block-sug", i))
                            .px_2()
                            .h(px(26.))
                            .flex()
                            .items_center()
                            .rounded_md()
                            .border_1()
                            .border_color(pal().border)
                            .text_xs()
                            .text_color(pal().subtle)
                            .cursor_pointer()
                            .hover(|d| d.border_color(pal().accent).text_color(pal().text))
                            .on_click(cx.listener(move |this, _, w, cx| this.editor_suggestion(sg.clone(), w, cx)))
                            .child(label)
                    })),
            )
            .child(div().text_xs().text_color(pal().muted).child(
                "Select text and press Make placeholder. Inserting the block selects each {{placeholder}} in turn: tab moves on. {{first name}}, {{today}} and {{me}} fill themselves; {{cursor}} is where you end up.",
            ))
            .children(e.error.clone().map(|err| div().text_xs().text_color(pal().error).child(err)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .child(button("block-save", "Save", true).on_click(cx.listener(|this, _, w, cx| this.save_block_editor(w, cx))))
                    .child(button("block-cancel", "Cancel", false).on_click(cx.listener(|this, _, w, cx| this.close_block_editor(w, cx))))
                    .child(div().flex_1())
                    .when(e.file.is_some(), |d| {
                        d.child(
                            div()
                                .id("block-delete")
                                .text_xs()
                                .text_color(pal().error)
                                .cursor_pointer()
                                .hover(|d| d.underline())
                                .on_click(cx.listener(|this, _, w, cx| this.delete_block_editor(w, cx)))
                                .child("Delete"),
                        )
                    }),
            )
    }

    pub fn render_blocks_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap_1();
        for (i, b) in self.blocks.iter().enumerate() {
            let block = b.clone();
            let click = b.clone();
            let first_line: String = b.body.replace(CURSOR, "").lines().next().unwrap_or("").chars().take(40).collect();
            let edit = b.clone();
            list = list.child(
                div()
                    .id(("block", i))
                    .group("block-card")
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(pal().border)
                    .bg(pal().card)
                    .cursor_grab()
                    .hover(|d| d.border_color(pal().border_strong).bg(pal().hover))
                    .on_drag(BlockDrag(block), |d: &BlockDrag, _, _, cx| cx.new(|_| DragChip(d.0.name.clone().into())))
                    .on_click(cx.listener(move |this, _, w, cx| this.insert_block(&click, None, w, cx)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().flex_1().min_w_0().truncate().text_sm().text_color(pal().text).child(b.name.clone()))
                            .children(b.shortcut.clone().map(|k| div().text_xs().font_family("Menlo").text_color(pal().muted).child(k)))
                            .child(
                                div()
                                    .id(("block-edit", i))
                                    .size(px(20.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .invisible()
                                    .group_hover("block-card", |d| d.visible())
                                    .hover(|d| d.bg(pal().button_hover))
                                    .tooltip(|w, cx| Tooltip::new("Edit block").build(w, cx))
                                    .on_click(cx.listener(move |this, _, w, cx| {
                                        cx.stop_propagation();
                                        this.edit_block(&edit, w, cx);
                                    }))
                                    .child(svg().path("icons/file-pen.svg").size(px(13.)).text_color(pal().subtle)),
                            ),
                    )
                    .child(div().text_xs().truncate().font_family("Menlo").text_color(pal().muted).child(first_line)),
            );
        }
        if let Some(e) = &self.block_editor {
            return div()
                .id("blocks-panel")
                .w(px(340.))
                .flex_none()
                .h_full()
                .px_3()
                .py_4()
                .flex()
                .flex_col()
                .bg(pal().list_bg)
                .border_r_1()
                .border_color(pal().border)
                .child(self.render_block_editor(e, cx));
        }
        div()
            .id("blocks-panel")
            .w(px(220.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .px_3()
            .py_4()
            .flex()
            .flex_col()
            .gap_3()
            .bg(pal().list_bg)
            .border_r_1()
            .border_color(pal().border)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_0p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child("Blocks"))
                            .child(
                                div()
                                    .id("blocks-collapse")
                                    .size(px(24.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .hover(|d| d.bg(pal().hover))
                                    .tooltip(|window, cx| Tooltip::new("Hide blocks").build(window, cx))
                                    .on_click(cx.listener(|this, _, _, cx| this.set_blocks_open(false, cx)))
                                    .child(svg().path("icons/chevron-left.svg").size(px(16.)).text_color(pal().subtle)),
                            ),
                    )
                    .child(div().text_xs().text_color(pal().muted).child("Drag into the message, or click to add at the cursor. Select text and press \u{2318}\u{21e7}K to make a block of it.")),
            )
            .child(list)
            .child(
                div()
                    .id("blocks-new")
                    .text_xs()
                    .text_color(pal().accent)
                    .cursor_pointer()
                    .hover(|d| d.underline())
                    .on_click(cx.listener(|this, _, w, cx| this.save_selection_as_block(w, cx)))
                    .child("New block\u{2026}"),
            )
            .child(
                div()
                    .id("blocks-folder")
                    .text_xs()
                    .text_color(pal().accent)
                    .cursor_pointer()
                    .hover(|d| d.underline())
                    .on_click(cx.listener(|this, _, _, cx| {
                        std::process::Command::new("open").arg(folder()).spawn().ok();
                        this.status = "Blocks are Markdown files; changes show up next time you write".into();
                        this.status_is_error = false;
                        cx.notify();
                    }))
                    .child("Edit blocks in Finder…"),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{file_text, make_placeholder, parse, suggestions, STARTERS};

    #[test]
    fn shortcut_lives_in_a_small_header() {
        let text = file_text(";ty", "Thanks, {{first name}}!");
        assert_eq!(text, "---\nshortcut: ;ty\n---\nThanks, {{first name}}!\n");
        assert_eq!(parse(&text), (Some(";ty".into()), "Thanks, {{first name}}!".into()));
        assert_eq!(parse("Plain block\n"), (None, "Plain block".into()));
        assert_eq!(file_text("", "x"), "x\n");
        let divider = STARTERS.iter().find(|(n, _)| *n == "Divider").unwrap().1;
        assert_eq!(parse(divider).0, None, "a Markdown rule isn't a header");
    }

    #[test]
    fn suggests_placeholders() {
        let s = suggestions("Hi Priya, the total is $2,400 due Friday, Nov 28. Mail ap@acme.test or pay $300.", Some("Priya"));
        let pairs: Vec<(&str, &str)> = s.iter().map(|s| (s.text.as_str(), s.label.as_str())).collect();
        assert_eq!(pairs, [("Priya", "first name"), ("$2,400", "amount"), ("Friday", "day"), ("Nov 28", "date")]);
        assert!(suggestions("Due {{amount}} on {{day}}", None).is_empty());
    }

    #[test]
    fn placeholder_from_a_selection() {
        let (out, sel) = make_placeholder("Pay $300 now", 4..8);
        assert_eq!(out, "Pay {{$300}} now");
        assert_eq!(&out[sel], "$300");
        let (out, sel) = make_placeholder("Hi ", 3..3);
        assert_eq!(&out[sel], "name");
    }

    #[test]
    fn quote_starter_fills_and_totals() {
        let body = parse(STARTERS.iter().find(|(n, _)| *n == "Quote").unwrap().1);
        assert_eq!(body.0.as_deref(), Some(";quote"));
        let smart = crate::fill::Smart { first_name: Some("Priya".into()), ..Default::default() };
        let ex = crate::fill::expand(&body.1, &smart);
        assert!(ex.text.starts_with("Hi Priya,"));
        let mut text = ex.text.clone();
        for (slot, value) in ex.slots.iter().zip(["spring catalog", "Catalog print run", "2,000", "$0.62"]).rev() {
            text.replace_range(slot.range.clone(), value);
        }
        let done = crate::table::calculate_all(&text);
        let line = |label: &str| done.lines().find(|l| l.contains(label)).unwrap_or_default().to_string();
        assert!(line("Catalog print run").starts_with("| 1 "), "{done}");
        assert!(line("Catalog print run").contains("$1,240.00"), "{done}");
        let total = line("**Total**");
        assert!(total.contains("**$1,240.00**") && total.contains("**2,000**") && !total.contains("$0.62"), "{done}");
        let sent = crate::table::strip_formulas(&done);
        assert!(!sent.contains("=Qty"), "{sent}");
    }
}
