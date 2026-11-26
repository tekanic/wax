use crate::keys::ShowPlugins;
use crate::theme::pal;
use gpui_kit::prelude::*;
use gpui_kit::*;
use std::sync::{Arc, LazyLock};

const ICON_PNG: &[u8] = include_bytes!("../assets/brand/wax-icon-1024.png");
pub const WORDMARK: &str = "brand/wax-wordmark.svg";
const WORDMARK_RATIO: f32 = 1783. / 556.;
const WAX: u32 = 0xA11D2E;
const WAX_DEEP: u32 = 0x5C0E1A;
const SITE: &str = "https://tekanic.github.io/wax/";

static ICON: LazyLock<Arc<Image>> = LazyLock::new(|| Arc::new(Image::from_bytes(ImageFormat::Png, ICON_PNG.to_vec())));

pub fn icon(size: f32) -> Img {
    img(ICON.clone()).size(px(size)).flex_none()
}

pub fn wordmark(height: f32) -> Svg {
    svg().path(WORDMARK).h(px(height)).w(px(height * WORDMARK_RATIO)).flex_none()
}

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

struct AboutWindow(AnyWindowHandle);

impl Global for AboutWindow {}

pub struct MainWindow(pub AnyWindowHandle);

impl Global for MainWindow {}

pub struct About;

impl Render for About {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = pal();
        let button = |id: &'static str, label: &'static str, primary: bool| {
            div()
                .id(id)
                .px_3p5()
                .py_2()
                .rounded_lg()
                .text_sm()
                .cursor_pointer()
                .when(primary, |d| d.bg(rgb(WAX)).text_color(rgb(0xffffff)).hover(|d| d.bg(rgb(WAX_DEEP))))
                .when(!primary, |d| d.bg(p.popover).border_1().border_color(p.border_strong).text_color(p.text).hover(|d| d.bg(p.popover_hover)))
                .child(label)
        };
        div()
            .size_full()
            .bg(p.bg)
            .text_color(p.text)
            .flex()
            .flex_col()
            .items_center()
            .pt(px(34.))
            .pb_5()
            .px_8()
            .gap(px(10.))
            .child(icon(128.))
            .child(wordmark(36.).text_color(p.text).mt_1())
            .child(div().mt_2().text_sm().text_color(p.muted).child(format!("Version {}", version())))
            .child(div().mt_1().text_sm().text_center().text_color(p.subtle).child("A small mail app. Everything beyond mail is a plugin."))
            .child(
                div()
                    .mt_3()
                    .flex()
                    .gap_2p5()
                    .child(button("about-settings", "Open settings file", false).on_click(|_, _, _| open_settings_file()))
                    .child(button("about-plugins", "Browse plugins", true).on_click(cx.listener(|_, _, window, cx| {
                        show_plugins(cx);
                        window.remove_window();
                    }))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_0p5()
                    .text_xs()
                    .text_color(p.muted)
                    .child("© 2026 Tekanics LLC · Open source under the GPL-3.0")
                    .child(
                        div()
                            .id("about-site")
                            .text_color(p.accent)
                            .cursor_pointer()
                            .hover(|d| d.underline())
                            .on_click(|_, _, cx| cx.open_url(SITE))
                            .child("tekanic.github.io/wax"),
                    ),
            )
    }
}

fn open_settings_file() {
    let path = crate::files::settings_file();
    if !path.exists() {
        crate::settings::Settings::load().save().ok();
    }
    std::process::Command::new("open").arg("-t").arg(&path).spawn().ok();
}

fn show_plugins(cx: &mut App) {
    let Some(main) = cx.try_global::<MainWindow>().map(|m| m.0) else { return };
    main.update(cx, |_, window, cx| {
        window.activate_window();
        window.dispatch_action(Box::new(ShowPlugins), cx);
    })
    .ok();
}

pub fn open(cx: &mut App) {
    if let Some(existing) = cx.try_global::<AboutWindow>().map(|a| a.0) {
        if existing.update(cx, |_, window, _| window.activate_window()).is_ok() {
            return;
        }
    }
    let size = size(px(420.), px(440.));
    let over = cx
        .try_global::<MainWindow>()
        .map(|m| m.0)
        .and_then(|main| main.update(cx, |_, window, cx| (window.bounds(), window.display(cx).map(|d| d.id()))).ok());
    let bounds = match over {
        Some((main, _)) => Bounds::new(main.center() - point(size.width / 2., size.height / 2.), size),
        None => Bounds::centered(None, size, cx),
    };
    let options = WindowOptions {
        display_id: over.and_then(|(_, d)| d),
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions { title: Some("About Wax".into()), appears_transparent: true, ..Default::default() }),
        is_resizable: false,
        is_minimizable: false,
        ..Default::default()
    };
    if let Ok((handle, _)) = gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| About)) {
        cx.set_global(AboutWindow(handle));
    }
}
