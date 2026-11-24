//! Attachments.
//!
//! Received: listed under the sender. Click opens the file (it's saved to a
//! temporary folder first); the arrow saves it to Downloads; "Save all"
//! saves every one. Saved files carry macOS's quarantine mark, as a browser
//! download would, so Gatekeeper checks them; programs and scripts are
//! never opened directly, only shown in Finder.
//!
//! Sent: the composer's Attach button, ⌘⇧A, or files dragged in from
//! Finder. Forwards carry the original's files.

use super::*;
use crate::model::human_size;
use std::path::{Path, PathBuf};

/// Gmail refuses messages over 25 MB (attachments grow ~4/3 when encoded).
pub const MAX_TOTAL_BYTES: u64 = 25 * 1024 * 1024 * 3 / 4;

/// Opening these would run something: show them in Finder instead.
const RUNNABLE: &[&str] = &[
    "app", "command", "sh", "bash", "zsh", "pkg", "mpkg", "dmg", "exe", "bat", "cmd", "js", "jar", "scpt",
    "applescript", "workflow", "terminal", "tool", "py", "rb", "pl", "action", "osax", "prefpane", "webloc",
];

fn is_runnable(name: &str) -> bool {
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RUNNABLE.contains(&e.to_ascii_lowercase().as_str()))
}

/// A file name with no path tricks in it.
fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let clean: String = base.chars().map(|c| if c.is_control() || c == ':' { '-' } else { c }).collect();
    let clean = clean.trim().trim_start_matches('.').to_string();
    if clean.is_empty() { "attachment".into() } else { clean }
}

/// `dir/name`, or `dir/name 2.ext` etc. when taken.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let name = safe_name(name);
    let first = dir.join(&name);
    if !first.exists() {
        return first;
    }
    let p = Path::new(&name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("attachment");
    let ext = p.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_default();
    (2..).map(|n| dir.join(format!("{stem} {n}{ext}"))).find(|p| !p.exists()).unwrap()
}

/// Write a received file and mark it as downloaded from the internet.
pub fn write_received(dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = unique_path(dir, name);
    std::fs::write(&path, bytes)?;
    let stamp = format!("0081;{:x};Wax;", chrono::Utc::now().timestamp());
    std::process::Command::new("xattr").args(["-w", "com.apple.quarantine", &stamp]).arg(&path).output().ok();
    Ok(path)
}

fn downloads() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join("Downloads")
}

fn temp_dir(tag: &str) -> PathBuf {
    std::env::temp_dir().join("gpui-mail").join(tag)
}

/// MIME type from a file name, for sending.
pub fn mime_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("heic") => "image/heic",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("txt" | "md" | "log") => "text/plain",
        Some("csv") => "text/csv",
        Some("html" | "htm") => "text/html",
        Some("ics") => "text/calendar",
        Some("eml") => "message/rfc822",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("zip") => "application/zip",
        Some("doc") => "application/msword",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        Some("xls") => "application/vnd.ms-excel",
        Some("xlsx") => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        Some("ppt") => "application/vnd.ms-powerpoint",
        Some("pptx") => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        Some("mp4") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        _ => "application/octet-stream",
    }
}

/// Write files that arrived with a message (for forwarding or reopening a
/// draft) so the composer can attach them like any other file.
pub fn stage(files: &[(String, Vec<u8>)]) -> Vec<PathBuf> {
    let dir = temp_dir(&format!("outgoing-{}", chrono::Utc::now().timestamp_millis()));
    files.iter().filter_map(|(name, bytes)| write_received(&dir, name, bytes).ok()).collect()
}

fn file_size(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Problems that should stop a send.
pub fn check(paths: &[PathBuf]) -> anyhow::Result<()> {
    for p in paths {
        if !p.is_file() {
            anyhow::bail!("{} is no longer there; remove it or attach it again", p.display());
        }
    }
    let total: u64 = paths.iter().map(|p| file_size(p)).sum();
    if total > MAX_TOTAL_BYTES {
        anyhow::bail!(
            "Attachments come to {}; Gmail's limit is about {}. Share a link instead.",
            human_size(total),
            human_size(MAX_TOTAL_BYTES)
        );
    }
    Ok(())
}

fn file_chip(name: &str, size: u64) -> Div {
    div()
        .h(px(30.))
        .pl_2()
        .pr_1()
        .flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(0x202026))
        .text_xs()
        .child(svg().path("icons/file.svg").size(px(14.)).flex_none().text_color(rgb(SUBTLE)))
        .child(div().max_w(px(220.)).truncate().text_color(rgb(TEXT)).child(name.to_string()))
        .child(div().flex_none().text_color(rgb(MUTED)).child(human_size(size)))
}

impl MailApp {
    // ----- Received ----------------------------------------------------------

    /// Fetch attachment `index` of the selected message; then open it, or
    /// save it to Downloads.
    pub fn fetch_attachment(&mut self, index: usize, open: bool, cx: &mut Context<Self>) {
        let Some(e) = self.envelopes.get(self.selected) else { return };
        let account = self.account_of(e).to_string();
        let Some(session) = self.preview_of(&account) else { return };
        let (mailbox, id) = (e.mailbox.clone(), e.id.clone());
        let label = e.attachments.get(index).map(|a| a.name.clone()).unwrap_or_default();
        self.status = format!("{} {label}…", if open { "Opening" } else { "Saving" }).into();
        self.status_is_error = false;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let (name, bytes) = session.lock().unwrap().attachment(&mailbox, &id, index)?;
                    let dir = if open { temp_dir(&format!("{mailbox}-{id}").replace('/', "_")) } else { downloads() };
                    let path = write_received(&dir, &name, &bytes)?;
                    let runnable = is_runnable(&name);
                    if open && !runnable {
                        std::process::Command::new("open").arg(&path).spawn()?;
                    } else {
                        std::process::Command::new("open").arg("-R").arg(&path).spawn()?;
                    }
                    anyhow::Ok((path, runnable))
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok((path, true)) => {
                        this.status = format!("{} could run a program, so it's shown in Finder instead of opened", path.display()).into();
                        this.status_is_error = true;
                    }
                    Ok((path, false)) if !open => {
                        this.status = format!("Saved to {}", path.display()).into();
                        this.status_is_error = false;
                    }
                    Ok(_) => {
                        this.status = "Opened".into();
                        this.status_is_error = false;
                    }
                    Err(err) => {
                        this.status = format!("Could not get the attachment: {err:#}").into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn save_all_attachments(&mut self, cx: &mut Context<Self>) {
        let n = self.envelopes.get(self.selected).map_or(0, |e| e.attachments.len());
        for i in 0..n {
            self.fetch_attachment(i, false, cx);
        }
    }

    /// The row of files under the sender, if the message has any.
    pub fn render_attachments(&self, e: &Envelope, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if e.attachments.is_empty() {
            return None;
        }
        let total: u64 = e.attachments.iter().map(|a| a.size).sum();
        let mut row = div().flex().flex_wrap().gap_2();
        for (i, a) in e.attachments.iter().enumerate() {
            row = row.child(
                file_chip(&a.name, a.size)
                    .id(("att", i))
                    .cursor_pointer()
                    .hover(|d| d.border_color(rgb(0x3d3d47)).bg(rgb(HOVER)))
                    .tooltip(|window, cx| Tooltip::new("Open").build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.fetch_attachment(i, true, cx)))
                    .child(
                        div()
                            .id(("att-save", i))
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .hover(|d| d.bg(rgb(0x3a3a44)))
                            .tooltip(|window, cx| Tooltip::new("Save to Downloads").build(window, cx))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.fetch_attachment(i, false, cx)
                            }))
                            .child(svg().path("icons/download.svg").size(px(13.)).text_color(rgb(SUBTLE))),
                    ),
            );
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child(svg().path("icons/paperclip.svg").size(px(13.)).text_color(rgb(MUTED)))
                        .child(format!(
                            "{} attachment{} · {}",
                            e.attachments.len(),
                            if e.attachments.len() == 1 { "" } else { "s" },
                            human_size(total)
                        ))
                        .when(e.attachments.len() > 1, |d| {
                            d.child(
                                div()
                                    .id("att-save-all")
                                    .ml_2()
                                    .text_color(rgb(ACCENT))
                                    .cursor_pointer()
                                    .hover(|d| d.underline())
                                    .on_click(cx.listener(|this, _, _, cx| this.save_all_attachments(cx)))
                                    .child("Save all"),
                            )
                        }),
                )
                .child(row),
        )
    }

    // ----- Sending ---------------------------------------------------------

    pub fn add_attachments(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(c) = &mut self.composer else { return };
        let mut added = 0;
        for p in paths.into_iter().filter(|p| p.is_file()) {
            if !c.attachments.contains(&p) {
                c.attachments.push(p);
                added += 1;
            }
        }
        if added > 0 {
            let total: u64 = c.attachments.iter().map(|p| file_size(p)).sum();
            self.status_is_error = total > MAX_TOTAL_BYTES;
            self.status = if self.status_is_error {
                format!("Attachments now {}: over Gmail's limit of about {}", human_size(total), human_size(MAX_TOTAL_BYTES)).into()
            } else {
                format!("Attached {added} file{}", if added == 1 { "" } else { "s" }).into()
            };
            self.schedule_autosave(cx);
        }
        cx.notify();
    }

    pub fn pick_attachments(&mut self, cx: &mut Context<Self>) {
        if self.composer.is_none() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            this.update(cx, |this, cx| this.add_attachments(paths, cx)).ok();
        })
        .detach();
    }

    fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.composer {
            if index < c.attachments.len() {
                c.attachments.remove(index);
                self.schedule_autosave(cx);
                cx.notify();
            }
        }
    }

    /// Files attached to the message being written.
    pub fn render_outgoing(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let c = self.composer.as_ref()?;
        if c.attachments.is_empty() {
            return None;
        }
        let mut row = div().flex().flex_wrap().gap_2();
        for (i, p) in c.attachments.iter().enumerate() {
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            row = row.child(
                file_chip(&name, file_size(p)).child(
                    div()
                        .id(("out-remove", i))
                        .size(px(22.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_sm()
                        .text_color(rgb(MUTED))
                        .cursor_pointer()
                        .hover(|d| d.bg(rgb(0x3a3a44)).text_color(rgb(TEXT)))
                        .tooltip(|window, cx| Tooltip::new("Remove").build(window, cx))
                        .on_click(cx.listener(move |this, _, _, cx| this.remove_attachment(i, cx)))
                        .child("×"),
                ),
            );
        }
        Some(
            div()
                .flex()
                .flex_none()
                .items_start()
                .gap_3()
                .child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .pt(px(7.))
                        .child(svg().path("icons/paperclip.svg").size(px(14.)).text_color(rgb(MUTED))),
                )
                .child(row),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{is_runnable, safe_name, unique_path};

    #[test]
    fn names_are_safe() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\\\x\\\\report.pdf"), "report.pdf");
        assert_eq!(safe_name(".hidden"), "hidden");
        assert!(is_runnable("Invoice.pdf.app"));
        assert!(is_runnable("setup.PKG"));
        assert!(!is_runnable("Invoice.pdf"));
    }

    #[test]
    fn no_overwrites() {
        let dir = std::env::temp_dir().join(format!("gpui-mail-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.pdf"), b"x").unwrap();
        assert_eq!(unique_path(&dir, "a.pdf").file_name().unwrap(), "a 2.pdf");
        std::fs::remove_dir_all(&dir).ok();
    }
}
