//! Picks up hand edits to the files in ~/.config/wax while Wax runs, the
//! way Zed does: save keymap.toml or settings.toml and the change applies.

use crate::{accounts, files, identity, keys, MailApp};
use gpui_kit::*;
use std::path::Path;
use std::time::{Duration, SystemTime};

const EVERY: Duration = Duration::from_secs(2);

/// When each kind of file last changed.
#[derive(Clone, PartialEq, Default)]
pub struct Seen {
    keymap: Option<SystemTime>,
    settings: Option<SystemTime>,
    /// account.toml, identities and signature.toml files: (path, mtime).
    others: Vec<(String, SystemTime)>,
    /// plugin.wasm and grants.toml in each plugin folder.
    plugins: Vec<(String, SystemTime)>,
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Every .toml file up to two folders down.
fn tomls(dir: &Path, depth: u8, out: &mut Vec<(String, SystemTime)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if path.is_dir() && depth > 0 {
            tomls(&path, depth - 1, out);
        } else if path.extension().is_some_and(|x| x == "toml") && !path.ends_with("rules.toml") {
            if let Some(t) = modified(&path) {
                out.push((path.to_string_lossy().into_owned(), t));
            }
        }
    }
}

pub fn look() -> Seen {
    let dir = files::config_dir();
    let mut others = Vec::new();
    for sub in ["accounts", "identities", "signatures"] {
        tomls(&dir.join(sub), 2, &mut others);
    }
    others.sort();
    let mut plugins = Vec::new();
    if let Ok(rd) = std::fs::read_dir(crate::wasm_host::plugins_dir()) {
        for dir in rd.flatten().map(|e| e.path()) {
            for f in ["plugin.wasm", "grants.toml"] {
                if let Some(t) = modified(&dir.join(f)) {
                    plugins.push((dir.join(f).to_string_lossy().into_owned(), t));
                }
            }
        }
    }
    plugins.sort();
    Seen { keymap: modified(&keys::keymap_file()), settings: modified(&files::settings_file()), others, plugins }
}

impl MailApp {
    /// Check the config files every two seconds and apply what changed.
    pub fn watch_config(&mut self, cx: &mut Context<Self>) {
        self.config_seen = look();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
            cx.background_executor().timer(EVERY).await;
            let now = cx.background_executor().spawn(async { look() }).await;
            let Ok(()) = this.update(cx, |this, cx| this.config_changed(now, cx)) else { return };
        })
        .detach();
    }

    fn config_changed(&mut self, now: Seen, cx: &mut Context<Self>) {
        let before = std::mem::replace(&mut self.config_seen, now.clone());
        if before == now {
            return;
        }
        if before.keymap != now.keymap {
            let errors = keys::apply(cx);
            if errors.is_empty() {
                self.status = "Shortcuts updated from keymap.toml".into();
                self.status_is_error = false;
            } else {
                self.status = errors[0].clone().into();
                self.status_is_error = true;
            }
        }
        if before.plugins != now.plugins {
            self.reload_plugins(cx);
        }
        if before.settings != now.settings {
            self.settings = crate::settings::Settings::load();
            self.use_account_settings();
        }
        if before.others != now.others {
            self.accounts = accounts::load();
            self.identities = identity::load();
            self.signatures = files::load_signatures();
            self.assign_signatures();
            self.use_account_settings();
        }
        cx.notify();
    }
}
