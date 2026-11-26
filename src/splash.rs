use super::*;
use std::time::{Duration, Instant};

const SHORTEST: Duration = Duration::from_millis(900);
const LONGEST: Duration = Duration::from_millis(2500);
const FADE: Duration = Duration::from_millis(280);

pub struct Splash {
    started: Instant,
    leaving: bool,
}

impl MailApp {
    pub fn start_splash(&mut self, cx: &mut Context<Self>) {
        if std::env::var_os("WAX_NO_SPLASH").is_some() {
            return;
        }
        self.splash = Some(Splash { started: Instant::now(), leaving: false });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                cx.background_executor().timer(Duration::from_millis(100)).await;
                let done = this
                    .update(cx, |this, _| {
                        this.splash.as_ref().is_none_or(|s| {
                            let age = s.started.elapsed();
                            age >= LONGEST || (age >= SHORTEST && !this.loading)
                        })
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
            let leaving = this
                .update(cx, |this, cx| {
                    let Some(s) = &mut this.splash else { return false };
                    s.leaving = true;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if leaving {
                cx.background_executor().timer(FADE).await;
            }
            this.update(cx, |this, cx| {
                this.splash = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn render_splash(&self) -> Option<AnyElement> {
        let s = self.splash.as_ref()?;
        let mark = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_5()
            .child(about::icon(112.))
            .child(about::wordmark(30.).text_color(pal().text))
            .with_animation("splash-in", Animation::new(Duration::from_millis(520)).with_easing(ease_out_quint()), |d, t| {
                d.opacity(t).mt(px((1. - t) * 14.))
            });
        let screen = div()
            .id("splash")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(pal().bg)
            .flex()
            .items_center()
            .justify_center()
            .child(mark);
        Some(if s.leaving {
            screen.with_animation("splash-out", Animation::new(FADE), |d, t| d.opacity(1. - t)).into_any_element()
        } else {
            screen.into_any_element()
        })
    }
}
