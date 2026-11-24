//! Open at login: a LaunchAgent in ~/Library/LaunchAgents that opens the
//! installed app when you sign in. Turning it off removes the file.

use anyhow::{anyhow, Result};
use std::path::PathBuf;

const LABEL: &str = "dev.gpui-mail.open-at-login";

fn agent_file() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_default();
    PathBuf::from(home).join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

/// The .app this binary runs from, if any.
pub fn app_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")).map(PathBuf::from)
}

pub fn enabled() -> bool {
    agent_file().exists()
}

pub fn set(on: bool) -> Result<()> {
    let file = agent_file();
    if !on {
        match std::fs::remove_file(&file) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
            _ => return Ok(()),
        }
    }
    let app = app_path().ok_or_else(|| anyhow!("Open at login works from the installed app (scripts/install.sh)"))?;
    let app = app.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;");
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(
        &file,
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>/usr/bin/open</string><string>-a</string><string>{app}</string></array>
<key>RunAtLoad</key><true/>
</dict></plist>
"#
        ),
    )?;
    Ok(())
}
