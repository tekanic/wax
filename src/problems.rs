//! Settings → Problems: what went wrong, newest first, from errors.log.

use super::*;
use crate::errlog;

const CLEAR_KEY: &str = "\u{1f}clear-problems";

impl MailApp {
    fn problems_shown(&self) -> Vec<&errlog::Entry> {
        self.problems
            .iter()
            .filter(|e| self.problems_filter.as_deref().map_or(true, |f| e.account.eq_ignore_ascii_case(f)))
            .collect()
    }

    fn copy_problems(&mut self, cx: &mut Context<Self>) {
        let text: String = self
            .problems_shown()
            .iter()
            .map(|e| format!("{}  {}  {}\n", errlog::when(e.ts), if e.account.is_empty() { "-" } else { &e.account }, e.text))
            .collect();
        let n = text.lines().count();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
        self.status = format!("Copied {n} problem{}", if n == 1 { "" } else { "s" }).into();
        self.status_is_error = false;
        cx.notify();
    }

    fn clear_problems(&mut self, cx: &mut Context<Self>) {
        let armed = self.settings_view.as_ref().is_some_and(|v| v.confirm_remove.as_deref() == Some(CLEAR_KEY));
        if let Some(v) = &mut self.settings_view {
            v.confirm_remove = if armed { None } else { Some(CLEAR_KEY.to_string()) };
        }
        if armed {
            errlog::clear();
            self.problems.clear();
            self.problems_unseen = 0;
        }
        cx.notify();
    }

    pub fn render_problems_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.problems_shown();
        let armed = self.settings_view.as_ref().is_some_and(|v| v.confirm_remove.as_deref() == Some(CLEAR_KEY));
        let tag = |account: &str| -> String {
            self.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(account)).map(|a| a.tag()).unwrap_or_else(|| {
                if account.is_empty() { "app".into() } else { account.to_string() }
            })
        };
        // Filter: all, or one account.
        let mut filters = div().flex().flex_wrap().gap_1p5().child(
            crate::settings::choice("prob-all", "All accounts", self.problems_filter.is_none())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.problems_filter = None;
                    cx.notify()
                })),
        );
        if self.accounts.len() > 1 {
            for a in &self.accounts {
                let email = a.email.clone();
                let on = self.problems_filter.as_deref() == Some(a.email.as_str());
                filters = filters.child(
                    crate::settings::choice(SharedString::from(format!("prob-{}", a.email)), a.short(), on).on_click(cx.listener(
                        move |this, _, _, cx| {
                            this.problems_filter = Some(email.clone());
                            cx.notify()
                        },
                    )),
                );
            }
        }
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .child(filters)
            .child(div().flex_1())
            .child(action_button("prob-copy", "Copy", false).on_click(cx.listener(|this, _, _, cx| this.copy_problems(cx))))
            .child(action_button("prob-file", "Show file", false).on_click(cx.listener(|_, _, _, _| {
                let p = errlog::path();
                if !p.exists() {
                    std::fs::write(&p, "").ok();
                }
                std::process::Command::new("open").arg("-R").arg(p).spawn().ok();
            })))
            .child(
                action_button("prob-clear", if armed { "Click again to clear" } else { "Clear" }, false)
                    .on_click(cx.listener(|this, _, _, cx| this.clear_problems(cx))),
            );
        let mut list = div().flex().flex_col().border_1().border_color(rgb(BORDER)).rounded_md();
        if shown.is_empty() {
            list = list.child(div().px_4().py_4().text_sm().text_color(rgb(MUTED)).child("Nothing has gone wrong."));
        }
        for (i, e) in shown.iter().enumerate().take(500) {
            let text = e.text.clone();
            list = list.child(
                div()
                    .id(("problem", i))
                    .px_4()
                    .py_2p5()
                    .flex()
                    .items_start()
                    .gap_3()
                    .when(i > 0, |d| d.border_t_1().border_color(rgb(BORDER)))
                    .child(div().w(px(150.)).flex_none().text_xs().text_color(rgb(MUTED)).pt(px(2.)).child(errlog::when(e.ts)))
                    .child(
                        div()
                            .flex_none()
                            .px(px(5.))
                            .rounded_sm()
                            .bg(rgb(0x2a2a33))
                            .text_xs()
                            .text_color(rgb(SUBTLE))
                            .child(tag(&e.account)),
                    )
                    .child(div().flex_1().min_w_0().text_sm().text_color(rgb(TEXT)).child(e.text.clone()))
                    .child(
                        div()
                            .id(("problem-copy", i))
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(ACCENT))
                            .cursor_pointer()
                            .hover(|d| d.underline())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                                this.status = "Copied".into();
                                this.status_is_error = false;
                                cx.notify();
                            }))
                            .child("Copy"),
                    ),
            );
        }
        div()
            .id("problems-pane")
            .size_full()
            .overflow_y_scroll()
            .px_8()
            .py_5()
            .child(
                div()
                    .max_w(px(900.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_sm().text_color(rgb(SUBTLE)).child(
                        "Errors from every account, newest first. They fade from the bottom bar; they're kept here (errors.log in the app's data folder) so you can look into them or send them along.",
                    ))
                    .child(toolbar)
                    .child(list),
            )
    }
}
