//! Native WebKit view (via wry) for HTML email, laid over the reader pane.
//!
//! GPUI has no HTML engine, so the reader reserves a rectangle with a
//! `canvas` element and this view is positioned on top of it every frame.
//!
//! Safety defaults for mail:
//! - a Content-Security-Policy blocks scripts and every remote load
//!   (images, fonts, CSS), so tracking pixels never fire;
//! - navigation inside the view is refused; http(s)/mailto links open in
//!   the default browser or mail handler instead.

use gpui_kit::*;
use std::cell::Cell;
use std::rc::Rc;
use wry::{
    dpi::{LogicalPosition, LogicalSize},
    NewWindowResponse, Rect, WebView, WebViewBuilder,
};

/// Blocks scripts and all remote loads.
const CSP_STRICT: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
    style-src 'unsafe-inline'; img-src data:; font-src data:\">";

/// Same, but lets images (only images) load over HTTPS.
const CSP_IMAGES: &str = "<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; \
    style-src 'unsafe-inline'; img-src data: https:; font-src data:\">";

const BASE_STYLE: &str = "<style>html{background:#ffffff;color:#111;}\
    body{margin:20px;font:14px -apple-system,BlinkMacSystemFont,Helvetica,sans-serif;\
    word-wrap:break-word;}img{max-width:100%;height:auto;}</style>";

pub struct HtmlView {
    webview: Option<Rc<WebView>>,
    /// (mailbox, id, images allowed) of the document currently loaded.
    loaded: Option<(String, String, bool)>,
    last_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    visible: bool,
}

impl HtmlView {
    pub fn new() -> Self {
        Self { webview: None, loaded: None, last_bounds: Rc::new(Cell::new(None)), visible: false }
    }

    /// Show `html` for message `key`, creating the view on first use.
    pub fn show(&mut self, key: (String, String), html: &str, allow_images: bool, window: &Window) {
        let key = (key.0, key.1, allow_images);
        match &self.webview {
            None => {
                let built = WebViewBuilder::new()
                    .with_html(prepare(html, allow_images))
                    .with_bounds(rect(0., 0., 1., 1.))
                    .with_devtools(false)
                    .with_navigation_handler(|url| {
                        if url.starts_with("about:") || url.starts_with("data:") {
                            return true;
                        }
                        open_external(&url);
                        false
                    })
                    .with_new_window_req_handler(|url, _| {
                        open_external(&url);
                        NewWindowResponse::Deny
                    })
                    .build_as_child(window);
                match built {
                    Ok(webview) => {
                        self.webview = Some(Rc::new(webview));
                        self.loaded = Some(key);
                        self.last_bounds.set(None);
                        self.visible = true;
                    }
                    Err(err) => eprintln!("could not create HTML view: {err}"),
                }
            }
            Some(webview) => {
                if self.loaded.as_ref() != Some(&key) {
                    webview.load_html(&prepare(html, allow_images)).ok();
                    self.loaded = Some(key);
                }
                if !self.visible {
                    webview.set_visible(true).ok();
                    self.visible = true;
                }
            }
        }
    }

    pub fn hide(&mut self) {
        if let (Some(webview), true) = (&self.webview, self.visible) {
            webview.set_visible(false).ok();
            webview.focus_parent().ok();
            self.visible = false;
        }
    }

    /// An element that fills its space and keeps the web view over it.
    pub fn placeholder(&self) -> impl IntoElement {
        let webview = self.webview.clone();
        let last = self.last_bounds.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, _, _| {
                let Some(webview) = &webview else { return };
                if last.get() != Some(bounds) {
                    last.set(Some(bounds));
                    webview
                        .set_bounds(rect(
                            f32::from(bounds.origin.x) as f64,
                            f32::from(bounds.origin.y) as f64,
                            f32::from(bounds.size.width) as f64,
                            f32::from(bounds.size.height) as f64,
                        ))
                        .ok();
                }
            },
        )
        .size_full()
    }
}

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w.max(1.), h.max(1.)).into(),
    }
}

/// True when the HTML references remote images that the strict policy blocks.
pub fn has_remote_images(html: &str) -> bool {
    let lower = html.to_ascii_lowercase();
    lower.contains("src=\"http") || lower.contains("src='http") || lower.contains("url(http")
        || lower.contains("src=http")
}

/// Put the CSP and base styles at the top of the document's head.
fn prepare(html: &str, allow_images: bool) -> String {
    let csp = if allow_images { CSP_IMAGES } else { CSP_STRICT };
    let inject = format!("<meta charset=\"utf-8\">{csp}{BASE_STYLE}");
    let lower = html.to_ascii_lowercase();
    if let Some(head) = lower.find("<head") {
        if let Some(end) = lower[head..].find('>') {
            let at = head + end + 1;
            return format!("{}{}{}", &html[..at], inject, &html[at..]);
        }
    }
    format!("<!DOCTYPE html><html><head>{inject}</head><body>{html}</body></html>")
}

fn open_external(url: &str) {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:") {
        std::process::Command::new("open").arg(url).spawn().ok();
    }
}
