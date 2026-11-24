//! Things you made live in plain files, not in the cache database, so
//! they can be backed up, synced and edited by hand, like Zed's:
//!
//!   ~/.config/wax/                (config_dir)
//!     settings.toml, keymap.toml
//!     accounts/<address>/account.toml, rules.toml
//!     signatures/<name>/signature.toml (+ logo.png, logo2.png)
//!     identities/<address>.toml
//!     blocks/<name>.md
//!
//!   ~/Library/Application Support/gpui-mail/   (data_dir)
//!     cache.sqlite (only what Gmail can give back), errors.log,
//!     google-client.json (a secret, so it stays out of ~/.config,
//!     which people often keep in a public dotfiles repo)
//! The TOML used here is the simple subset `key = "text"` / `key = 123`.

use crate::signature::{Layout, Signature};
use anyhow::{anyhow, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn data_dir() -> PathBuf {
    crate::store::default_path().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."))
}

/// Where settings live: ~/.config/wax (WAX_CONFIG_DIR overrides it).
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("WAX_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join(".config/wax")
}

/// The settings that used to live beside the cache.
const MOVED: [&str; 5] = ["settings.toml", "accounts", "signatures", "identities", "blocks"];

/// Move settings from the old folder (beside the cache) to config_dir, once.
/// Each item moves only if config_dir doesn't have it yet, so nothing there
/// is ever overwritten. Returns what moved.
pub fn migrate_config() -> Result<Vec<String>> {
    migrate_between(&data_dir(), &config_dir())
}

fn migrate_between(old: &Path, new: &Path) -> Result<Vec<String>> {
    let mut moved = Vec::new();
    for name in MOVED {
        let from = old.join(name);
        let to = new.join(name);
        if from.exists() && !to.exists() {
            std::fs::create_dir_all(new)?;
            std::fs::rename(&from, &to)?;
            moved.push(name.to_string());
        }
    }
    if !moved.is_empty() {
        let note = old.join("SETTINGS-MOVED.txt");
        std::fs::write(note, format!("Wax settings now live in {}\n", new.display())).ok();
    }
    Ok(moved)
}

pub fn signatures_dir() -> PathBuf {
    config_dir().join("signatures")
}

pub fn settings_file() -> PathBuf {
    config_dir().join("settings.toml")
}

// ----- Tiny TOML subset ------------------------------------------------------

pub fn quote(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `key = value` lines; strings unquoted and unescaped. Comments and blank
/// lines are skipped.
pub fn parse(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        let value = if let Some(inner) = v.strip_prefix('"').and_then(|s| s.rfind('"').map(|e| &s[..e])) {
            let mut s = String::new();
            let mut chars = inner.chars();
            while let Some(c) = chars.next() {
                if c == '\\' {
                    match chars.next() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some(o) => s.push(o),
                        None => {}
                    }
                } else {
                    s.push(c);
                }
            }
            s
        } else {
            v.split('#').next().unwrap_or("").trim().to_string()
        };
        out.insert(k.trim().to_string(), value);
    }
    out
}

/// Write the file next to its final name, then swap it in, so a crash never
/// leaves half a file.
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

// ----- Settings --------------------------------------------------------------

pub fn load_settings() -> Option<crate::settings::Settings> {
    let text = std::fs::read_to_string(settings_file()).ok()?;
    let m = parse(&text);
    let d = crate::settings::Settings::default();
    let flag = |k: &str, default: bool| m.get(k).map(|v| v == "true").unwrap_or(default);
    let id = |k: &str| m.get(k).and_then(|v| v.parse().ok()).filter(|i: &i64| *i != 0);
    Some(crate::settings::Settings {
        mono: flag("monospaced", d.mono),
        hide_list_when_writing: flag("hide_list_while_writing", d.hide_list_when_writing),
        blocks_open: flag("blocks_open", d.blocks_open),
        sig_new: id("signature_for_new"),
        sig_reply: id("signature_for_replies"),
    })
}

pub fn save_settings(s: &crate::settings::Settings) -> Result<()> {
    std::fs::create_dir_all(config_dir())?;
    let id = |v: Option<i64>| v.map(|i| i.to_string()).unwrap_or_else(|| "0".into());
    let text = format!(
        "# Wax settings. Edit here or in the app (⌘,).\n\
         monospaced = {}\n\
         hide_list_while_writing = {}\n\
         blocks_open = {}\n\
         # Signature ids (see signatures/*/signature.toml); 0 = none.\n\
         signature_for_new = {}\n\
         signature_for_replies = {}\n",
        s.mono,
        s.hide_list_when_writing,
        s.blocks_open,
        id(s.sig_new),
        id(s.sig_reply)
    );
    write_atomic(&settings_file(), text.as_bytes())
}

// ----- Signatures ------------------------------------------------------------

fn ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        _ => "png",
    }
}

fn mime_of(path: &Path) -> Option<&'static str> {
    crate::signature::image_mime(path)
}

/// A folder name from a signature's name.
fn folder_name(label: &str) -> String {
    let clean: String = label
        .chars()
        .map(|c| if c.is_alphanumeric() || " -_.&()".contains(c) { c } else { '-' })
        .collect();
    let clean = clean.trim().trim_matches('.').to_string();
    if clean.is_empty() { "Signature".into() } else { clean }
}

fn read_signature(dir: &Path) -> Option<Signature> {
    let m = parse(&std::fs::read_to_string(dir.join("signature.toml")).ok()?);
    let g = |k: &str| m.get(k).cloned().unwrap_or_default();
    let n = |k: &str, d: u32| m.get(k).and_then(|v| v.parse().ok()).unwrap_or(d);
    let logo = |stem: &str| {
        ["png", "jpg", "jpeg", "gif"].iter().find_map(|e| {
            let p = dir.join(format!("{stem}.{e}"));
            let bytes = std::fs::read(&p).ok()?;
            Some((mime_of(&p)?.to_string(), bytes))
        })
    };
    let mut s = Signature::new(&g("name_in_picker"));
    s.id = m.get("id").and_then(|v| v.parse().ok())?;
    if s.label.is_empty() {
        s.label = dir.file_name()?.to_string_lossy().to_string();
    }
    s.layout = Layout::parse(&g("layout"));
    s.name = g("name");
    s.title = g("title");
    s.company = g("company");
    s.phone = g("phone");
    s.email = g("email");
    s.website = g("website");
    s.note = g("note");
    s.accent = m.get("accent").and_then(|v| u32::from_str_radix(v.trim_start_matches('#'), 16).ok()).unwrap_or(s.accent);
    s.font = g("font");
    s.logo = logo("logo");
    s.logo_width = n("logo_width", 72);
    s.logo_link = g("logo_link");
    s.logo2 = logo("logo2");
    s.logo2_width = n("logo2_width", 142);
    s.logo2_link = g("logo2_link");
    s.social_label = g("follow_on");
    s.social_url = g("follow_link");
    s.account = g("account").trim().to_lowercase();
    Some(s)
}

/// Every signature, in id order (the order they were made).
pub fn load_signatures() -> Vec<Signature> {
    let mut out: Vec<Signature> = std::fs::read_dir(signatures_dir())
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| read_signature(&e.path())).collect())
        .unwrap_or_default();
    out.sort_by_key(|s| s.id);
    out
}

fn folder_of(id: i64) -> Option<PathBuf> {
    std::fs::read_dir(signatures_dir())
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| read_signature(p).is_some_and(|s| s.id == id))
}

/// Save (id 0 = new). Renames the folder when the name changed. Returns
/// the id.
pub fn save_signature(sig: &Signature) -> Result<i64> {
    let root = signatures_dir();
    std::fs::create_dir_all(&root)?;
    let mut s = sig.clone();
    if s.id == 0 {
        s.id = load_signatures().iter().map(|x| x.id).max().unwrap_or(0) + 1;
    }
    // Folder: named after the signature, unique among the others.
    let current = folder_of(s.id);
    let base = folder_name(&s.label);
    let mut name = base.clone();
    let mut n = 2;
    while root.join(&name).exists() && current.as_deref() != Some(root.join(&name).as_path()) {
        name = format!("{base} {n}");
        n += 1;
    }
    let dir = root.join(&name);
    match &current {
        Some(old) if old != &dir => std::fs::rename(old, &dir)?,
        Some(_) => {}
        None => std::fs::create_dir_all(&dir)?,
    }
    let text = format!(
        "# Signature for Wax. Logos are the image files next to this one.\n\
         id = {}\n\
         name_in_picker = {}\n\
         layout = {}  # stacked, logo-left, logo-top, two-logos\n\
         name = {}\n\
         title = {}\n\
         company = {}\n\
         phone = {}\n\
         email = {}\n\
         website = {}\n\
         note = {}\n\
         accent = \"#{:06x}\"\n\
         font = {}  # empty = the message's font\n\
         logo_width = {}\n\
         logo_link = {}\n\
         logo2_width = {}\n\
         logo2_link = {}\n\
         follow_on = {}\n\
         follow_link = {}\n\
         # The account it belongs to.\n\
         account = {}\n",
        s.id,
        quote(&s.label),
        quote(s.layout.as_str()),
        quote(&s.name),
        quote(&s.title),
        quote(&s.company),
        quote(&s.phone),
        quote(&s.email),
        quote(&s.website),
        quote(&s.note),
        s.accent,
        quote(&s.font),
        s.logo_width,
        quote(&s.logo_link),
        s.logo2_width,
        quote(&s.logo2_link),
        quote(&s.social_label),
        quote(&s.social_url),
        quote(&s.account),
    );
    write_atomic(&dir.join("signature.toml"), text.as_bytes())?;
    for (stem, logo) in [("logo", &s.logo), ("logo2", &s.logo2)] {
        // Drop old copies (maybe another format), then write the current one.
        for e in ["png", "jpg", "jpeg", "gif"] {
            let p = dir.join(format!("{stem}.{e}"));
            if logo.as_ref().map_or(true, |(m, _)| ext(m) != e) {
                std::fs::remove_file(p).ok();
            }
        }
        if let Some((mime, bytes)) = logo {
            let p = dir.join(format!("{stem}.{}", ext(mime)));
            if std::fs::read(&p).ok().as_deref() != Some(bytes.as_slice()) {
                write_atomic(&p, bytes)?;
            }
        }
    }
    Ok(s.id)
}

/// Deleted signatures go to the Trash folder inside `signatures/`, not
/// away for good.
pub fn delete_signature(id: i64) -> Result<()> {
    let dir = folder_of(id).ok_or_else(|| anyhow!("no signature {id}"))?;
    let trash = signatures_dir().join(".deleted");
    std::fs::create_dir_all(&trash)?;
    let name = format!("{}-{}", dir.file_name().unwrap_or_default().to_string_lossy(), chrono::Local::now().format("%Y%m%d-%H%M%S"));
    std::fs::rename(&dir, trash.join(name))?;
    Ok(())
}

/// One-time move out of the cache database (where the first version kept
/// them). Returns how many signatures moved.
pub fn migrate_from_db(store: &crate::store::Store) -> usize {
    let mut moved = 0;
    if !signatures_dir().exists() {
        let first = store.cached_accounts().into_iter().next().unwrap_or_default();
        if let Ok(list) = store.signatures(&first) {
            for s in &list {
                if save_signature(s).is_ok() {
                    moved += 1;
                }
            }
            if moved == list.len() {
                for s in &list {
                    store.delete_signature(&first, s.id).ok();
                }
            }
        }
    }
    if !settings_file().exists() {
        save_settings(&crate::settings::Settings::load_db(store)).ok();
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::{parse, quote};

    #[test]
    fn round_trips_text() {
        let tricky = "Line one\nShe said \"hi\" \\ bye\tend";
        let m = parse(&format!("note = {}\nwidth = 72 # px\n# comment\n", quote(tricky)));
        assert_eq!(m["note"], tricky);
        assert_eq!(m["width"], "72");
    }
}

#[cfg(test)]
mod migrate_tests {
    #[test]
    fn moves_once_and_never_overwrites() {
        let base = std::env::temp_dir().join(format!("wax-migrate-{}", std::process::id()));
        let (old, new) = (base.join("old"), base.join("new"));
        std::fs::create_dir_all(old.join("accounts/a@b.c")).unwrap();
        std::fs::write(old.join("settings.toml"), "monospaced = true").unwrap();
        std::fs::create_dir_all(new.join("blocks")).unwrap();
        std::fs::create_dir_all(old.join("blocks")).unwrap();
        let moved = super::migrate_between(&old, &new).unwrap();
        assert_eq!(moved, vec!["settings.toml", "accounts"]);
        assert!(new.join("accounts/a@b.c").is_dir() && !old.join("settings.toml").exists());
        assert!(old.join("blocks").exists(), "existing new/blocks is left alone");
        assert!(super::migrate_between(&old, &new).unwrap().is_empty());
        std::fs::remove_dir_all(base).ok();
    }
}
