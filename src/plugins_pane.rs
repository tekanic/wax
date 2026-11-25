//! Settings → Plugins: what's installed, what each asks for, on/off.

use super::*;
use crate::settings::{choice, setting_row};
use std::path::PathBuf;
use crate::wasm_host::{self, permission_about, write_grants};

impl MailApp {
    /// Load (or reload) the plugins in the background.
    pub fn reload_plugins(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let host = cx
                .background_executor()
                .spawn(async move {
                    let mut h = wasm_host::Host::new()?;
                    h.store = store;
                    h.load();
                    anyhow::Ok(h)
                })
                .await;
            this.update(cx, |this, cx| {
                match host {
                    Ok(h) => {
                        for p in h.plugins.iter().filter(|p| p.broken.is_some()) {
                            crate::errlog::record("", &format!("Plugin {}: {}", p.name(), p.broken.clone().unwrap_or_default()));
                        }
                        this.wasm = Some(Arc::new(Mutex::new(h)));
                        this.refresh_commands(cx);
                    }
                    Err(e) => {
                        this.status = format!("Plugins could not start: {e:#}").into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_plugin_enabled(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        let Some(host) = self.wasm.clone() else { return };
        let mut host = host.lock().unwrap();
        let Some(p) = host.plugins.iter_mut().find(|p| p.id == id) else { return };
        let requested = p.requested();
        // Switching on grants what it asked for; off grants nothing.
        let grants = if on { requested.clone() } else { Vec::new() };
        match write_grants(&p.dir, on, &grants, &requested) {
            Ok(()) => {
                p.enabled = on;
                p.grants = grants;
                self.status = format!("{} is {}", p.name(), if on { "on" } else { "off" }).into();
                self.status_is_error = false;
            }
            Err(e) => {
                self.status = format!("Could not save {}: {e:#}", p.dir.join("grants.toml").display()).into();
                self.status_is_error = true;
            }
        }
        cx.notify();
    }

    pub fn render_plugins_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let folder = wasm_host::plugins_dir();
        let show = {
            let folder = folder.clone();
            choice("plugins-folder", "Show folder", false).on_click(cx.listener(move |_, _, _, _| {
                std::fs::create_dir_all(&folder).ok();
                std::process::Command::new("open").arg(&folder).spawn().ok();
            }))
        };
        let mut list = div().max_w(px(760.)).flex().flex_col().child(setting_row(
            "Plugins folder",
            "Each plugin is a folder here holding plugin.wasm. Plugins run sandboxed: no files, no network, limited time and memory, and they only change mail in the ways you allow.",
            show,
        ));
        let plugins: Vec<(String, String, String, bool, Option<String>, Vec<&'static str>, PathBuf)> = self
            .wasm
            .as_ref()
            .map(|h| {
                h.lock()
                    .unwrap()
                    .plugins
                    .iter()
                    .map(|p| {
                        let version = p.manifest.as_ref().map(|m| m.version.clone()).unwrap_or_default();
                        (p.id.clone(), p.name(), version, p.enabled, p.broken.clone(), p.requested().into_iter().map(permission_about).collect(), p.dir.clone())
                    })
                    .collect()
            })
            .unwrap_or_default();
        if plugins.is_empty() {
            list = list.child(div().pt_4().text_sm().text_color(rgb(MUTED)).child(if self.wasm.is_none() {
                "Loading plugins…"
            } else {
                "No plugins yet. Put a plugin's folder in the plugins folder and it shows up here, switched off."
            }));
        }
        for (id, name, version, enabled, broken, asks, dir) in plugins {
            let (on_id, off_id) = (id.clone(), id.clone());
            let config = dir.join("config.toml");
            let toggle = div()
                .flex()
                .gap_1p5()
                .child(choice(SharedString::from(format!("plug-cfg-{id}")), "Edit config", false).on_click(cx.listener(move |_, _, _, _| {
                    if !config.exists() {
                        std::fs::write(&config, "# Settings for this plugin.\n").ok();
                    }
                    std::process::Command::new("open").arg("-t").arg(&config).spawn().ok();
                })))
                .child(choice(SharedString::from(format!("plug-on-{id}")), "On", enabled).on_click(cx.listener(move |this, _, _, cx| this.set_plugin_enabled(&on_id, true, cx))))
                .child(choice(SharedString::from(format!("plug-off-{id}")), "Off", !enabled).on_click(cx.listener(move |this, _, _, cx| this.set_plugin_enabled(&off_id, false, cx))));
            let mut about = div().flex().flex_col().gap_1().min_w_0().child(
                div().flex().items_baseline().gap_2().child(div().text_sm().text_color(rgb(TEXT)).child(name)).child(div().text_xs().text_color(rgb(MUTED)).child(format!("{id} {version}"))),
            );
            about = about.child(div().text_xs().text_color(rgb(SUBTLE)).child(if asks.is_empty() {
                "Asks for nothing".to_string()
            } else {
                format!("Asks to: {}", asks.join("; "))
            }));
            if let Some(why) = broken {
                about = about.child(div().text_xs().text_color(rgb(0xe06c75)).child(format!("Not running: {why}")));
            }
            list = list.child(
                div().flex().items_center().justify_between().gap_8().py_4().border_b_1().border_color(rgb(BORDER)).child(about).child(toggle),
            );
        }
        div().id("plugins-pane").size_full().overflow_y_scroll().px_8().py_4().child(list)
    }
}
