//! Blocks: ready-made pieces of a message (a table, a callout, a link
//! button…) dragged or clicked into the composer.
//!
//! A block is a Markdown file in the blocks folder; its name is the file
//! name. `{{cursor}}` marks where the cursor goes after inserting it. A few
//! starters are written the first time; edit, add or delete files freely.
//! This is the shape a plugin's blocks will take later.

use super::*;

#[derive(Clone, Debug)]
pub struct Block {
    pub name: String,
    pub body: String,
}

const CURSOR: &str = "{{cursor}}";

const STARTERS: &[(&str, &str)] = &[
    ("Table", "| {{cursor}}Item | Qty | Notes |\n| ---- | --: | ----- |\n|      |     |       |\n|      |     |       |"),
    ("Callout", "> **Note:** {{cursor}}"),
    ("Link button", "**[{{cursor}}Open it here](https://)**"),
    ("Divider", "---\n\n{{cursor}}"),
    ("Next steps", "**Next steps**\n\n1. {{cursor}}\n2. \n3. "),
    ("Meeting times", "Would any of these work for you?\n\n- {{cursor}}\n- \n- "),
];

pub fn folder() -> std::path::PathBuf {
    store::default_path().with_file_name("blocks")
}

/// Every block in the folder, starters written on first use.
pub fn load() -> Vec<Block> {
    let dir = folder();
    if !dir.exists() {
        if std::fs::create_dir_all(&dir).is_ok() {
            for (name, body) in STARTERS {
                std::fs::write(dir.join(format!("{name}.md")), body).ok();
            }
        }
    }
    let mut out: Vec<Block> = std::fs::read_dir(&dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "md"))
                .filter_map(|p| {
                    let name = p.file_stem()?.to_string_lossy().to_string();
                    let body = std::fs::read_to_string(&p).ok()?;
                    Some(Block { name, body: body.trim_end().to_string() })
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|b| b.name.to_lowercase());
    out
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
            .bg(rgb(ACCENT))
            .text_color(rgb(0xffffff))
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
        let body = block.body.replace(CURSOR, "");
        let mark = block.body.find(CURSOR).unwrap_or(body.len());
        // The word right after the marker (a placeholder) gets selected.
        let word = body[mark..].find(|c: char| c.is_whitespace() || "|]*_)".contains(c)).unwrap_or(body.len() - mark);
        let (out, start) = formatting::insert_block(&old, at.unwrap_or(cursor), &body);
        let caret = start + mark;
        let focus = c.body.read(cx).focus_handle(cx);
        self.apply_body_edit(out, caret..caret + word, window, cx);
        focus.focus(window, cx);
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
            .bg(rgb(LIST_BG))
            .border_r_1()
            .border_color(rgb(BORDER))
            .cursor_pointer()
            .hover(|d| d.bg(rgb(HOVER)))
            .tooltip(|window, cx| Tooltip::new("Show blocks").build(window, cx))
            .on_click(cx.listener(|this, _, _, cx| this.set_blocks_open(true, cx)))
            // The label drawing is 14.5 × 38.8 at 12 px; shown at ~17 px.
            .child(
                svg()
                    .path("icons/blocks-label.svg")
                    .w(px(20.3))
                    .h(px(54.3))
                    .text_color(rgb(TEXT))
                    .group_hover("blocks-ribbon", |s| s.text_color(rgb(ACCENT))),
            )
    }

    pub fn render_blocks_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap_1();
        for (i, b) in self.blocks.iter().enumerate() {
            let block = b.clone();
            let click = b.clone();
            let first_line: String = b.body.replace(CURSOR, "").lines().next().unwrap_or("").chars().take(40).collect();
            list = list.child(
                div()
                    .id(("block", i))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(0x1f1f25))
                    .cursor_grab()
                    .hover(|d| d.border_color(rgb(0x3d3d47)).bg(rgb(HOVER)))
                    .on_drag(BlockDrag(block), |d: &BlockDrag, _, _, cx| cx.new(|_| DragChip(d.0.name.clone().into())))
                    .on_click(cx.listener(move |this, _, w, cx| this.insert_block(&click, None, w, cx)))
                    .child(div().text_sm().text_color(rgb(TEXT)).child(b.name.clone()))
                    .child(div().text_xs().truncate().font_family("Menlo").text_color(rgb(MUTED)).child(first_line)),
            );
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
            .bg(rgb(LIST_BG))
            .border_r_1()
            .border_color(rgb(BORDER))
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
                                    .hover(|d| d.bg(rgb(HOVER)))
                                    .tooltip(|window, cx| Tooltip::new("Hide blocks").build(window, cx))
                                    .on_click(cx.listener(|this, _, _, cx| this.set_blocks_open(false, cx)))
                                    .child(svg().path("icons/chevron-left.svg").size(px(16.)).text_color(rgb(SUBTLE))),
                            ),
                    )
                    .child(div().text_xs().text_color(rgb(MUTED)).child("Drag into the message, or click to add at the cursor.")),
            )
            .child(list)
            .child(
                div()
                    .id("blocks-folder")
                    .text_xs()
                    .text_color(rgb(ACCENT))
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
