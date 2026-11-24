//! Sign in with Google, the way desktop mail apps do it: the browser opens
//! Google's sign-in page, Google sends the browser back to a one-time
//! address on this Mac (http://127.0.0.1:<port>), and the app trades the
//! code it receives for tokens. No password is ever typed into the app.
//!
//! The refresh token (long-lived) goes in the Keychain, service
//! "gpui-mail-google", one entry per address. Access tokens (an hour) are
//! kept in memory and renewed as needed. IMAP and SMTP log in with them
//! using XOAUTH2.
//!
//! The OAuth client is your own Google Cloud project's "Desktop app"
//! client: its downloaded JSON is copied to `google-client.json` in the
//! data folder. (Google treats a desktop client's secret as not secret.)

use anyhow::{anyhow, bail, Context as _, Result};
use base64::Engine as _;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
/// Full mail access (IMAP + SMTP) plus who you are.
const SCOPES: &str = "https://mail.google.com/ openid email profile";
pub const KEYCHAIN_SERVICE: &str = "gpui-mail-google";
/// How long to wait for you in the browser.
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);

pub struct Client {
    pub id: String,
    pub secret: String,
}

pub fn client_file() -> PathBuf {
    crate::files::data_dir().join("google-client.json")
}

fn parse_client(text: &str) -> Result<Client> {
    let v: serde_json::Value = serde_json::from_str(text).context("Not a JSON file")?;
    if v.get("web").is_some() {
        bail!("That's a \"Web application\" client. Create a \"Desktop app\" client in Google Cloud → Clients and choose its file.");
    }
    let c = v.get("installed").unwrap_or(&v);
    let get = |k: &str| c.get(k).and_then(|x| x.as_str()).map(str::to_string);
    match (get("client_id"), get("client_secret")) {
        (Some(id), Some(secret)) if id.ends_with(".apps.googleusercontent.com") => Ok(Client { id, secret }),
        _ => bail!("That isn't a Google OAuth client file (client_secret_….json from Google Cloud → Clients)"),
    }
}

/// A release build can carry the project's own client (set at build time:
/// GPUI_MAIL_GOOGLE_CLIENT_ID / GPUI_MAIL_GOOGLE_CLIENT_SECRET), so users
/// just sign in. Without one, each user brings a client from their own
/// Google Cloud project (Settings → Accounts walks through it).
pub fn built_in_client() -> Option<Client> {
    match (option_env!("GPUI_MAIL_GOOGLE_CLIENT_ID"), option_env!("GPUI_MAIL_GOOGLE_CLIENT_SECRET")) {
        (Some(id), Some(secret)) if !id.is_empty() => Some(Client { id: id.into(), secret: secret.into() }),
        _ => None,
    }
}

/// Your own client file wins over the built-in one.
pub fn load_client() -> Option<Client> {
    std::fs::read_to_string(client_file()).ok().and_then(|t| parse_client(&t).ok()).or_else(built_in_client)
}

/// Check and keep the client JSON you downloaded from Google Cloud.
pub fn import_client(path: &Path) -> Result<()> {
    let text = std::fs::read_to_string(path)?;
    parse_client(&text)?;
    std::fs::create_dir_all(crate::files::data_dir())?;
    std::fs::write(client_file(), text)?;
    Ok(())
}

// ----- Keychain ----------------------------------------------------------------

/// Stored through `security -i` so the token never appears in a process list.
fn keychain_set(account: &str, secret: &str) -> Result<()> {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let mut child = std::process::Command::new("security")
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    writeln!(
        child.stdin.take().unwrap(),
        "add-generic-password -U -s \"{}\" -a \"{}\" -l \"gpui-mail (Google)\" -w \"{}\"",
        KEYCHAIN_SERVICE,
        esc(account),
        esc(secret)
    )?;
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!("Keychain: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

fn keychain_get(account: &str) -> Option<String> {
    let out = std::process::Command::new("security")
        .args(["find-generic-password", "-s", KEYCHAIN_SERVICE, "-a", account, "-w"])
        .output()
        .ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|s| !s.is_empty())
}

fn keychain_delete(account: &str) {
    std::process::Command::new("security")
        .args(["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", account])
        .output()
        .ok();
}

pub fn is_signed_in(email: &str) -> bool {
    keychain_get(email).is_some()
}

// ----- Tokens --------------------------------------------------------------------

fn cache() -> &'static Mutex<HashMap<String, (String, Instant)>> {
    static C: OnceLock<Mutex<HashMap<String, (String, Instant)>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn post_form(url: &str, form: &[(&str, &str)]) -> Result<serde_json::Value> {
    let resp = ureq::post(url).timeout(Duration::from_secs(30)).send_form(form);
    let text = match resp {
        Ok(r) => r.into_string()?,
        Err(ureq::Error::Status(_, r)) => {
            let body = r.into_string().unwrap_or_default();
            let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
            let code = v.get("error").and_then(|e| e.as_str()).unwrap_or("error").to_string();
            let desc = v.get("error_description").and_then(|e| e.as_str()).unwrap_or("").to_string();
            return Err(anyhow!(TokenError { code, desc }));
        }
        Err(e) => return Err(anyhow!("Could not reach Google: {e}")),
    };
    Ok(serde_json::from_str(&text)?)
}

#[derive(Debug)]
pub struct TokenError {
    pub code: String,
    pub desc: String,
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.desc.is_empty() { write!(f, "Google said {}", self.code) } else { write!(f, "Google said {}: {}", self.code, self.desc) }
    }
}

impl std::error::Error for TokenError {}

fn remember(email: &str, v: &serde_json::Value) -> Option<String> {
    let token = v.get("access_token")?.as_str()?.to_string();
    let secs = v.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(3600);
    // Renew a minute early so a login never races the expiry.
    let until = Instant::now() + Duration::from_secs(secs.saturating_sub(60));
    cache().lock().unwrap().insert(email.to_lowercase(), (token.clone(), until));
    Some(token)
}

/// A current access token for `email`, renewed with the stored refresh
/// token when needed.
pub fn access_token(email: &str) -> Result<String> {
    let key = email.to_lowercase();
    if let Some((t, until)) = cache().lock().unwrap().get(&key) {
        if Instant::now() < *until {
            return Ok(t.clone());
        }
    }
    let client = load_client().ok_or_else(|| anyhow!("Google sign-in isn't set up: choose the client file in Settings → Accounts"))?;
    let refresh = keychain_get(&key).ok_or_else(|| anyhow!("{email} isn't signed in: Settings → Accounts → Sign in with Google"))?;
    let v = post_form(
        TOKEN_URL,
        &[("client_id", &client.id), ("client_secret", &client.secret), ("refresh_token", &refresh), ("grant_type", "refresh_token")],
    )
    .map_err(|e| match e.downcast_ref::<TokenError>() {
        Some(t) if t.code == "invalid_grant" => {
            anyhow!("Google signed {email} out (password change, revoked access, or the app is still in Testing). Sign in again in Settings → Accounts.")
        }
        _ => e,
    })?;
    remember(&key, &v).ok_or_else(|| anyhow!("Google sent no access token"))
}

/// Forget the account's tokens here and ask Google to revoke them.
pub fn sign_out(email: &str) {
    let key = email.to_lowercase();
    if let Some(refresh) = keychain_get(&key) {
        ureq::post(REVOKE_URL).timeout(Duration::from_secs(10)).send_form(&[("token", &refresh)]).ok();
    }
    keychain_delete(&key);
    cache().lock().unwrap().remove(&key);
}

// ----- Signing in ------------------------------------------------------------------

fn random_b64(n: usize) -> String {
    let mut buf = vec![0u8; n];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf)).expect("random bytes");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(buf)
}

pub fn pkce_challenge(verifier: &str) -> String {
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(verifier.as_bytes()))
}

pub fn auth_url(client_id: &str, redirect: &str, challenge: &str, state: &str, hint: Option<&str>) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    q.append_pair("client_id", client_id)
        .append_pair("redirect_uri", redirect)
        .append_pair("response_type", "code")
        .append_pair("scope", SCOPES)
        .append_pair("code_challenge", challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("state", state)
        .append_pair("access_type", "offline")
        // Always ask, so Google always returns a refresh token.
        .append_pair("prompt", "consent select_account");
    if let Some(h) = hint {
        q.append_pair("login_hint", h);
    }
    format!("{AUTH_URL}?{}", q.finish())
}

/// The query of the browser's request line ("GET /?code=…&state=… HTTP/1.1").
pub fn callback_params(request: &str) -> HashMap<String, String> {
    let target = request.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("");
    let query = target.split_once('?').map(|(_, q)| q).unwrap_or("");
    url::form_urlencoded::parse(query.as_bytes()).into_owned().collect()
}

/// Email and name from Google's id_token (read, not verified: it came
/// straight from Google over TLS in answer to our own request).
pub fn id_claims(id_token: &str) -> Option<(String, String)> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let email = v.get("email")?.as_str()?.to_lowercase();
    let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
    Some((email, name))
}

pub struct SignedIn {
    pub email: String,
    pub name: String,
}

const DONE_PAGE: &str = "<!doctype html><meta charset=utf-8><title>Wax</title>\
<body style=\"font:15px -apple-system,sans-serif;background:#16161a;color:#e6e6ea;display:grid;place-items:center;height:90vh\">\
<div><h2 style=\"font-weight:600\">Signed in</h2><p>You can close this tab and go back to Wax.</p></div>";

fn fail_page(msg: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>Wax</title>\
<body style=\"font:15px -apple-system,sans-serif;background:#16161a;color:#e6e6ea;display:grid;place-items:center;height:90vh\">\
<div><h2 style=\"font-weight:600\">Not signed in</h2><p>{}</p></div>",
        msg.replace('<', "&lt;")
    )
}

/// Run the whole browser sign-in. Blocks until you finish, cancel, or five
/// minutes pass; `cancel` stops it early.
pub fn sign_in(hint: Option<&str>, cancel: Arc<AtomicBool>) -> Result<SignedIn> {
    let client = load_client().ok_or_else(|| anyhow!("Choose your Google client file first"))?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let verifier = random_b64(48);
    let state = random_b64(16);
    let url = auth_url(&client.id, &redirect, &pkce_challenge(&verifier), &state, hint);
    std::process::Command::new("open").arg(&url).spawn()?;

    let deadline = Instant::now() + SIGN_IN_TIMEOUT;
    let (mut stream, params) = loop {
        if cancel.load(Ordering::Relaxed) {
            bail!("Sign-in cancelled");
        }
        if Instant::now() > deadline {
            bail!("Sign-in timed out");
        }
        match listener.accept() {
            Ok((mut s, _)) => {
                s.set_nonblocking(false)?;
                s.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut buf = [0u8; 8192];
                let n = s.read(&mut buf).unwrap_or(0);
                let params = callback_params(&String::from_utf8_lossy(&buf[..n]));
                // Browsers also ask for /favicon.ico and the like.
                if params.contains_key("code") || params.contains_key("error") {
                    break (s, params);
                }
                s.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").ok();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.into()),
        }
    };
    let respond = |s: &mut std::net::TcpStream, page: &str| {
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            page.len()
        );
        s.write_all(head.as_bytes()).ok();
        s.write_all(page.as_bytes()).ok();
    };
    if let Some(err) = params.get("error") {
        let msg = if err == "access_denied" { "Access wasn't allowed.".to_string() } else { format!("Google said {err}.") };
        respond(&mut stream, &fail_page(&msg));
        bail!("{msg}");
    }
    if params.get("state") != Some(&state) {
        respond(&mut stream, &fail_page("The reply didn't match this sign-in. Try again."));
        bail!("Sign-in reply didn't match (state)");
    }
    let code = params.get("code").cloned().unwrap_or_default();
    let result = (|| {
        let v = post_form(
            TOKEN_URL,
            &[
                ("client_id", &client.id),
                ("client_secret", &client.secret),
                ("code", &code),
                ("code_verifier", &verifier),
                ("redirect_uri", &redirect),
                ("grant_type", "authorization_code"),
            ],
        )?;
        let scope = v.get("scope").and_then(|s| s.as_str()).unwrap_or("");
        if !scope.contains("https://mail.google.com/") {
            bail!("Mail access wasn't granted. Sign in again and tick \"Read, compose, send…\" on Google's page.");
        }
        let (email, name) = v
            .get("id_token")
            .and_then(|t| t.as_str())
            .and_then(id_claims)
            .ok_or_else(|| anyhow!("Google didn't say which account this is"))?;
        let refresh = v
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .ok_or_else(|| anyhow!("Google sent no refresh token; remove gpui-mail at myaccount.google.com/permissions and try again"))?;
        keychain_set(&email, refresh)?;
        remember(&email, &v);
        Ok(SignedIn { email, name })
    })();
    match &result {
        Ok(_) => respond(&mut stream, DONE_PAGE),
        Err(e) => respond(&mut stream, &fail_page(&format!("{e:#}"))),
    }
    result
}

#[cfg(test)]
mod tests {
    use super::{auth_url, callback_params, id_claims, parse_client, pkce_challenge};
    use base64::Engine as _;

    #[test]
    fn pkce_is_sha256_base64url() {
        // base64url(SHA-256(verifier)), no padding; checked against Python's hashlib.
        assert_eq!(pkce_challenge("gpui-mail-test-verifier-0123456789-abcdefghijklmnop"), "Kw9TlozVMpq-rV5K_vuur9YzxOGrcISJpLtpX-WBrLw");
    }

    #[test]
    fn url_and_callback() {
        let u = auth_url("abc.apps.googleusercontent.com", "http://127.0.0.1:5555", "ch", "st", Some("me@x.com"));
        assert!(u.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A5555"));
        assert!(u.contains("scope=https%3A%2F%2Fmail.google.com%2F+openid+email+profile"));
        assert!(u.contains("access_type=offline") && u.contains("login_hint=me%40x.com"));
        let p = callback_params("GET /?state=st&code=4%2F0Ab&scope=email HTTP/1.1\r\nHost: x\r\n\r\n");
        assert_eq!(p.get("code").map(String::as_str), Some("4/0Ab"));
        assert_eq!(p.get("state").map(String::as_str), Some("st"));
    }

    #[test]
    fn reads_id_token_and_client() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"email":"Me@Example.com","name":"Alex"}"#);
        assert_eq!(id_claims(&format!("h.{payload}.s")), Some(("me@example.com".into(), "Alex".into())));
        let c = parse_client(r#"{"installed":{"client_id":"1-x.apps.googleusercontent.com","client_secret":"GOCSPX-y"}}"#).unwrap();
        assert_eq!(c.secret, "GOCSPX-y");
        assert!(parse_client(r#"{"installed":{"client_id":"nope"}}"#).is_err());
        assert!(parse_client(r#"{"web":{"client_id":"1-x.apps.googleusercontent.com","client_secret":"s"}}"#).is_err());
    }
}
