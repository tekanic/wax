//! Mail accounts. Each is a folder in the data folder:
//!
//!   accounts/<email>/account.toml   name, sign-in, default signatures
//!   accounts/<email>/rules.toml     that account's sender rules
//!
//! Google accounts (Gmail and Google Workspace) sign in with Google; the
//! app never sees the password. "app-password" is the older Keychain setup
//! (service "gpui-mail"), kept working until the account signs in with
//! Google.

use crate::files::{data_dir, parse, quote};
use anyhow::{anyhow, Result};
use pimalaya_stream::sasl::{Sasl, SaslLogin, SaslXoauth2};
use secrecy::SecretString;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Auth {
    Google,
    AppPassword,
}

impl Auth {
    fn as_str(self) -> &'static str {
        match self {
            Auth::Google => "google",
            Auth::AppPassword => "app-password",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Account {
    pub email: String,
    /// Shown as the sender name on mail you send.
    pub name: String,
    pub auth: Auth,
    /// Sidebar order.
    pub order: i64,
    pub sig_new: Option<i64>,
    pub sig_reply: Option<i64>,
    /// Badge colour (0xRRGGBB); None = picked from PALETTE by position.
    pub color: Option<u32>,
    /// Badge text (1–2 characters); empty = the first letter of its name.
    pub badge: String,
}

/// At most two characters (an emoji counts as one), no spaces.
pub fn clean_badge(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).take(2).collect()
}

/// Account colours: distinct on a dark background, white text readable.
pub const PALETTE: [u32; 8] = [0x3b82f6, 0x16a34a, 0xea580c, 0x9333ea, 0xdb2777, 0x0d9488, 0xca8a04, 0x64748b];

impl Account {
    /// The badge colour.
    pub fn color(&self) -> u32 {
        self.color.unwrap_or(PALETTE[(self.order.max(1) as usize - 1) % PALETTE.len()])
    }

    /// The badge text: what you set, else the start of the sidebar name.
    pub fn initial(&self) -> String {
        let own = clean_badge(&self.badge);
        if !own.is_empty() {
            return own;
        }
        self.short().chars().find(|c| c.is_alphanumeric()).unwrap_or('?').to_uppercase().to_string()
    }

    pub fn new(email: &str, name: &str) -> Self {
        Account {
            email: email.to_lowercase(),
            name: name.to_string(),
            auth: Auth::Google,
            order: load().iter().map(|a| a.order).max().unwrap_or(0) + 1,
            sig_new: None,
            sig_reply: None,
            color: None,
            badge: String::new(),
        }
    }

    /// Short name for the sidebar: the part before @ for Gmail, else the
    /// domain ("example.com").
    pub fn short(&self) -> String {
        let (user, domain) = self.email.split_once('@').unwrap_or((&self.email, ""));
        if domain == "gmail.com" || domain == "googlemail.com" { user.to_string() } else { domain.to_string() }
    }

    /// Compact label for list rows in All Inboxes: "gmail", "example".
    pub fn tag(&self) -> String {
        let domain = self.email.split_once('@').map(|(_, d)| d).unwrap_or(&self.email);
        domain.split('.').next().unwrap_or(domain).to_string()
    }

    pub fn dir(&self) -> PathBuf {
        account_dir(&self.email)
    }
}

pub fn root() -> PathBuf {
    crate::files::config_dir().join("accounts")
}

fn clean(email: &str) -> String {
    email.to_lowercase().chars().map(|c| if c.is_alphanumeric() || "@.-_+".contains(c) { c } else { '-' }).collect()
}

pub fn account_dir(email: &str) -> PathBuf {
    root().join(clean(email))
}

pub fn rules_file(email: &str) -> PathBuf {
    account_dir(email).join("rules.toml")
}

fn read(path: &std::path::Path) -> Option<Account> {
    let m = parse(&std::fs::read_to_string(path).ok()?);
    let id = |k: &str| m.get(k).and_then(|v| v.parse::<i64>().ok()).filter(|i| *i != 0);
    let email = m.get("email")?.trim().to_lowercase();
    if !email.contains('@') {
        return None;
    }
    Some(Account {
        email,
        name: m.get("name").cloned().unwrap_or_default(),
        auth: if m.get("sign_in").map(String::as_str) == Some("app-password") { Auth::AppPassword } else { Auth::Google },
        order: m.get("order").and_then(|v| v.parse().ok()).unwrap_or(0),
        sig_new: id("signature_for_new"),
        sig_reply: id("signature_for_replies"),
        color: m.get("color").and_then(|c| u32::from_str_radix(c.trim().trim_start_matches('#'), 16).ok()),
        badge: m.get("badge").map(|b| clean_badge(b)).unwrap_or_default(),
    })
}

/// All accounts, in sidebar order.
pub fn load() -> Vec<Account> {
    let mut out: Vec<Account> = std::fs::read_dir(root())
        .map(|rd| rd.filter_map(|e| e.ok()).filter_map(|e| read(&e.path().join("account.toml"))).collect())
        .unwrap_or_default();
    out.sort_by(|a, b| a.order.cmp(&b.order).then(a.email.cmp(&b.email)));
    out
}

/// The first account's address ("" with none); for command-line checks.
pub fn first_email() -> std::sync::Arc<str> {
    load().first().map(|a| a.email.as_str().into()).unwrap_or_else(|| "".into())
}

pub fn find(email: &str) -> Option<Account> {
    load().into_iter().find(|a| a.email.eq_ignore_ascii_case(email))
}

pub fn save(a: &Account) -> Result<()> {
    std::fs::create_dir_all(a.dir())?;
    let sig = |v: Option<i64>| v.unwrap_or(0);
    let text = format!(
        "# A mail account in Wax.\n\
         email = {}\n\
         # The sender name on mail you send from this account.\n\
         name = {}\n\
         # \"google\" = Sign in with Google (token in the Keychain).\n\
         sign_in = {}\n\
         order = {}\n\
         # Signature ids (signatures/*/signature.toml); 0 = none.\n\
         signature_for_new = {}\n\
         signature_for_replies = {}\n\
         # Badge colour, \"#rrggbb\", and text (1-2 characters); empty = automatic.\n\
         color = {}\n\
         badge = {}\n",
        quote(&a.email),
        quote(&a.name),
        quote(a.auth.as_str()),
        a.order,
        sig(a.sig_new),
        sig(a.sig_reply),
        quote(&a.color.map(|c| format!("#{c:06x}")).unwrap_or_default()),
        quote(&a.badge)
    );
    let path = a.dir().join("account.toml");
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// Remove an account: its folder moves to accounts/.removed/ (rules kept
/// just in case) and its Google tokens are revoked and deleted.
pub fn remove(email: &str) -> Result<()> {
    let dir = account_dir(email);
    if dir.exists() {
        let bin = root().join(".removed");
        std::fs::create_dir_all(&bin)?;
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
        std::fs::rename(&dir, bin.join(format!("{}-{stamp}", clean(email))))?;
    }
    crate::oauth::sign_out(email);
    Ok(())
}

/// First run with accounts: addresses the cache already holds mail for,
/// and that have an app password in the Keychain from the old setup,
/// become accounts (until they sign in with Google). The first takes the
/// default signatures and the sender rules with it. A new install has
/// neither, and starts with no accounts.
pub fn migrate(settings: &crate::settings::Settings, cached: &[String]) {
    if root().exists() {
        return;
    }
    let mut order = 0;
    for email in cached {
        if crate::mail::keychain_password(email).is_err() {
            continue;
        }
        order += 1;
        let a = Account {
            email: email.to_lowercase(),
            name: String::new(),
            auth: Auth::AppPassword,
            order,
            sig_new: if order == 1 { settings.sig_new } else { None },
            sig_reply: if order == 1 { settings.sig_reply } else { None },
            color: None,
            badge: String::new(),
        };
        if save(&a).is_ok() && order == 1 {
            let old_rules = data_dir().join("rules.toml");
            if old_rules.exists() {
                std::fs::rename(&old_rules, rules_file(&a.email)).ok();
            }
        }
    }
}

/// How to log in to `email` over IMAP and SMTP.
pub fn sasl(email: &str) -> Result<Sasl> {
    let account = find(email).ok_or_else(|| anyhow!("{email} isn't one of your accounts"))?;
    Ok(match account.auth {
        Auth::Google => Sasl::from(SaslXoauth2 {
            username: account.email.clone(),
            token: SecretString::from(crate::oauth::access_token(&account.email)?),
        }),
        Auth::AppPassword => Sasl::from(SaslLogin {
            username: account.email.clone(),
            password: SecretString::from(crate::mail::keychain_password(&account.email)?),
        }),
    })
}

/// Gmail and Google Workspace use the same servers.
pub fn imap_url(_email: &str) -> &'static str {
    "imaps://imap.gmail.com"
}

pub fn smtp_url(_email: &str) -> &'static str {
    "smtps://smtp.gmail.com"
}

#[cfg(test)]
mod tests {
    use super::{Account, Auth};

    #[test]
    fn short_names() {
        let a = |e: &str| Account { email: e.into(), name: String::new(), auth: Auth::Google, order: 0, sig_new: None, sig_reply: None, color: None, badge: String::new() };
        assert_eq!(a("alex.doe@gmail.com").short(), "alex.doe");
        assert_eq!(a("alex@example.com").short(), "example.com");
        assert_eq!(a("alex.doe@gmail.com").tag(), "gmail");
        assert_eq!(a("alex@example.com").tag(), "example");
        assert_eq!(a("alex.doe@gmail.com").initial(), "A");
        assert_eq!(a("alex@example.com").initial(), "E");
        let mut l = a("alex@example.com");
        l.badge = " WORK ".into();
        assert_eq!(l.initial(), "WO");
        assert_eq!(super::clean_badge("✉️x"), "✉\u{fe0f}");
    }
}
