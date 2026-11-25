//! The plugin host: WebAssembly components in ~/.config/wax/plugins.
//!
//!     ~/.config/wax/plugins/<id>/plugin.wasm    the component
//!     ~/.config/wax/plugins/<id>/grants.toml    on/off and what you allowed
//!     ~/.config/wax/plugins/<id>/config.toml    the plugin's own settings
//!
//! Every call gets a fresh instance with no files, no network, no
//! environment, a fuel budget (so a loop can't hang Wax) and a memory cap.
//! A plugin that traps is switched off for this run and reported. A new
//! plugin starts switched off: grants.toml lists what it asks for, and
//! Settings → Plugins turns it on.

use anyhow::{anyhow, Result};
use std::path::{Path, PathBuf};
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

mod bindings {
    wasmtime::component::bindgen!({ world: "plugin", path: "wit/plugin.wit", additional_derives: [PartialEq] });
}

use bindings::wax;
/// The generated bindings for one instance.
type Plugin_ = bindings::Plugin;
pub use wax::plugin::types::{Action, Address, Envelope, Manifest, Permission, Sender, SenderView, ViewKey};

/// wasmtime has its own error type.
fn w<T>(r: wasmtime::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow!("{e:#}"))
}

/// Instructions one call may run (roughly; Wasmtime's fuel unit).
const FUEL: u64 = 500_000_000;
/// Memory one instance may grow to.
const MEMORY: usize = 64 << 20;

pub fn plugins_dir() -> PathBuf {
    crate::files::config_dir().join("plugins")
}

pub fn permission_name(p: Permission) -> &'static str {
    match p {
        Permission::ReadEnvelopes => "read-envelopes",
        Permission::ReadBodies => "read-bodies",
        Permission::ModifyMail => "modify-mail",
        Permission::Compose => "compose",
        Permission::Network => "network",
        Permission::Storage => "storage",
    }
}

/// What a permission lets a plugin do, for Settings.
pub fn permission_about(p: Permission) -> &'static str {
    match p {
        Permission::ReadEnvelopes => "see who new mail is from, to and its subject",
        Permission::ReadBodies => "read message text",
        Permission::ModifyMail => "label, move, archive, star and mark mail read",
        Permission::Compose => "open drafts for you to send",
        Permission::Network => "reach the internet",
        Permission::Storage => "keep its own notes",
    }
}

struct State {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
    dir: PathBuf,
    grants: Vec<Permission>,
    logs: Vec<String>,
    /// The mail cache, for `senders` and `knows-sender`.
    store: Option<crate::SharedStore>,
    arrival: Arrival,
}

/// What `knows-sender` needs about the batch of new mail being handled.
#[derive(Clone, Default)]
pub struct Arrival {
    /// Your addresses for the account: its own and its From identities.
    pub mine: Vec<String>,
    /// (mailbox, id) of every message in the batch, and their Message-IDs.
    pub batch: Vec<(String, String)>,
    pub batch_mids: Vec<String>,
}

impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.wasi, table: &mut self.table }
    }
}

impl wax::plugin::types::Host for State {}

impl wax::plugin::host::Host for State {
    fn log(&mut self, message: String) {
        if self.logs.len() < 20 {
            self.logs.push(message.chars().take(300).collect());
        }
    }
    fn config(&mut self) -> String {
        std::fs::read_to_string(self.dir.join("config.toml")).unwrap_or_default()
    }
    fn message_text(&mut self, _account: String, _mailbox: String, _id: String) -> Result<String, String> {
        Err("not available yet".into())
    }
    fn http_get(&mut self, _url: String) -> Result<String, String> {
        Err("not available yet".into())
    }
    fn senders(&mut self, account: String, limit: u32) -> Result<Vec<Sender>, String> {
        if !self.grants.contains(&Permission::ReadEnvelopes) {
            return Err("needs read-envelopes".into());
        }
        let store = self.store.as_ref().ok_or("the mail cache isn't open yet")?;
        let stats = store.lock().unwrap().sender_stats(&account, crate::ALL_MAIL, &account, limit.min(5000)).map_err(|e| format!("{e:#}"))?;
        let rules = crate::rules::load(&account).map(|(r, _)| r).unwrap_or_default();
        Ok(stats
            .into_iter()
            .map(|s| Sender {
                rule: rules.get(&s.email.to_lowercase()).cloned().or(s.rule),
                email: s.email,
                name: s.name,
                count: s.count,
                unread: s.unread,
                latest: s.last_ts,
            })
            .collect())
    }
    fn knows_sender(&mut self, message: Envelope) -> bool {
        if !self.grants.contains(&Permission::ReadEnvelopes) {
            return false;
        }
        let account = message.account.clone();
        let Some(email) = message.from.first().map(|a| a.email.to_lowercase()) else { return false };
        if crate::plugins::same_side(&email, &self.arrival.mine) {
            return true;
        }
        if crate::rules::load(&account).is_some_and(|(r, _)| r.contains_key(&email)) {
            return true;
        }
        let Some(store) = &self.store else { return false };
        // Outside a batch (a test, a command), leave this message out.
        let mut batch = self.arrival.batch.clone();
        if batch.is_empty() {
            batch.push((message.mailbox.clone(), message.id.clone()));
        }
        let mids = if self.arrival.batch.is_empty() { message.message_id.into_iter().collect() } else { self.arrival.batch_mids.clone() };
        store.lock().unwrap().sender_known(&account, &email, &batch, &mids)
    }
    fn storage_get(&mut self, key: String) -> Option<String> {
        if !self.grants.contains(&Permission::Storage) {
            return None;
        }
        let map = crate::files::parse(&std::fs::read_to_string(self.dir.join("storage.toml")).unwrap_or_default());
        map.get(&key).cloned()
    }
    fn storage_set(&mut self, key: String, value: String) {
        if !self.grants.contains(&Permission::Storage) || key.contains(['\n', '=', '"']) {
            return;
        }
        let path = self.dir.join("storage.toml");
        let mut map = crate::files::parse(&std::fs::read_to_string(&path).unwrap_or_default());
        map.insert(key, value);
        let text: String = map.iter().map(|(k, v)| format!("{k} = {}\n", crate::files::quote(v))).collect();
        std::fs::write(path, text).ok();
    }
}

/// One plugin on disk.
pub struct Plugin {
    pub id: String,
    pub dir: PathBuf,
    pub manifest: Option<Manifest>,
    pub enabled: bool,
    pub grants: Vec<Permission>,
    /// Why it isn't running (failed to load, or trapped this run).
    pub broken: Option<String>,
    component: Option<Component>,
}

impl Plugin {
    pub fn name(&self) -> String {
        self.manifest.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| self.id.clone())
    }
    pub fn requested(&self) -> Vec<Permission> {
        self.manifest.as_ref().map(|m| m.permissions.clone()).unwrap_or_default()
    }
    pub fn running(&self) -> bool {
        self.enabled && self.broken.is_none() && self.component.is_some()
    }
}

/// All plugins, compiled once.
pub struct Host {
    engine: Engine,
    linker: Linker<State>,
    pub plugins: Vec<Plugin>,
    /// The mail cache, handed to plugins through `senders`.
    pub store: Option<crate::SharedStore>,
    /// Set by the caller for each batch of new mail.
    pub arrival: Arrival,
}

impl Host {
    pub fn new() -> Result<Self> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = w(Engine::new(&config))?;
        let mut linker: Linker<State> = Linker::new(&engine);
        w(wasmtime_wasi::p2::add_to_linker_sync(&mut linker))?;
        w(Plugin_::add_to_linker::<State, HasSelf<State>>(&mut linker, |s| s))?;
        Ok(Host { engine, linker, plugins: Vec::new(), store: None, arrival: Arrival::default() })
    }

    /// Read every plugin folder. A plugin seen for the first time gets a
    /// grants.toml that keeps it switched off.
    pub fn load(&mut self) {
        self.load_from(&plugins_dir())
    }

    fn load_from(&mut self, root: &Path) {
        let mut found = Vec::new();
        let dirs = std::fs::read_dir(root).map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<_>>());
        for dir in dirs.unwrap_or_default() {
            if !dir.join("plugin.wasm").exists() {
                continue;
            }
            let id = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let mut plugin = Plugin { id, dir: dir.clone(), manifest: None, enabled: false, grants: Vec::new(), broken: None, component: None };
            match Component::from_file(&self.engine, dir.join("plugin.wasm")) {
                Ok(c) => plugin.component = Some(c),
                Err(e) => plugin.broken = Some(format!("could not load: {e:#}")),
            }
            if plugin.component.is_some() {
                match self.call(&plugin, |bindings, store| bindings.call_manifest(store)) {
                    Ok((m, _)) => plugin.manifest = Some(m),
                    Err(e) => plugin.broken = Some(format!("{e:#}")),
                }
            }
            let (enabled, grants) = read_grants(&dir, &plugin.requested());
            plugin.enabled = enabled;
            plugin.grants = grants;
            found.push(plugin);
        }
        found.sort_by(|a, b| a.id.cmp(&b.id));
        self.plugins = found;
    }

    /// Run one export on a fresh, sandboxed instance. Returns its result
    /// and whatever it logged.
    fn call<T>(&self, plugin: &Plugin, f: impl FnOnce(&Plugin_, &mut Store<State>) -> wasmtime::Result<T>) -> Result<(T, Vec<String>)> {
        let component = plugin.component.as_ref().ok_or_else(|| anyhow!("not loaded"))?;
        let state = State {
            wasi: WasiCtx::builder().build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new().memory_size(MEMORY).instances(16).build(),
            dir: plugin.dir.clone(),
            grants: plugin.grants.clone(),
            logs: Vec::new(),
            store: self.store.clone(),
            arrival: self.arrival.clone(),
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|s| &mut s.limits);
        w(store.set_fuel(FUEL))?;
        let bindings = w(Plugin_::instantiate(&mut store, component, &self.linker))?;
        let out = w(f(&bindings, &mut store))?;
        Ok((out, std::mem::take(&mut store.data_mut().logs)))
    }

    /// Run one export of plugin `i`, collecting what it logged. A plugin
    /// that traps, loops or runs out of memory is switched off for this run.
    fn run<T>(&mut self, i: usize, problems: &mut Vec<String>, f: impl FnOnce(&Plugin_, &mut Store<State>) -> wasmtime::Result<T>) -> Option<T> {
        let p = &self.plugins[i];
        match self.call(p, f) {
            Ok((out, logs)) => {
                for l in logs {
                    problems.push(format!("{}: {l}", p.name()));
                }
                Some(out)
            }
            Err(e) => {
                let why = format!("{e:#}");
                problems.push(format!("{} stopped and was switched off until Wax restarts: {why}", p.name()));
                self.plugins[i].broken = Some(why);
                None
            }
        }
    }

    /// Keep the actions plugin `i` was allowed to take.
    fn permitted(&self, i: usize, actions: Vec<Action>, problems: &mut Vec<String>) -> Vec<Action> {
        let p = &self.plugins[i];
        let (ok, refused): (Vec<Action>, Vec<Action>) = actions.into_iter().partition(|a| allowed(a, &p.grants));
        if !refused.is_empty() {
            problems.push(format!("{}: {} action(s) refused (not allowed in Settings → Plugins)", p.name(), refused.len()));
        }
        ok
    }

    fn reads(&self, i: usize) -> bool {
        let p = &self.plugins[i];
        p.running() && p.grants.contains(&Permission::ReadEnvelopes)
    }

    /// Ask every running plugin about a new message. Returns (plugin id,
    /// actions it's allowed to take) and problems to report.
    pub fn arrived(&mut self, message: &Envelope) -> (Vec<(String, Vec<Action>)>, Vec<String>) {
        let mut out = Vec::new();
        let mut problems = Vec::new();
        for i in 0..self.plugins.len() {
            if !self.reads(i) {
                continue;
            }
            if let Some(actions) = self.run(i, &mut problems, |b, s| b.call_on_message_received(s, message)) {
                let ok = self.permitted(i, actions, &mut problems);
                if !ok.is_empty() {
                    out.push((self.plugins[i].id.clone(), ok));
                }
            }
        }
        (out, problems)
    }

    /// The first running plugin's sender view (the Sweep), as (plugin id, view).
    pub fn sender_view(&mut self, account: &str) -> (Option<(String, SenderView)>, Vec<String>) {
        let mut problems = Vec::new();
        for i in 0..self.plugins.len() {
            if !self.reads(i) {
                continue;
            }
            if let Some(Some(view)) = self.run(i, &mut problems, |b, s| b.call_get_sender_view(s, account)) {
                return (Some((self.plugins[i].id.clone(), view)), problems);
            }
        }
        (None, problems)
    }

    /// A view key pressed on a sender row of plugin `id`'s view.
    pub fn sender_key(&mut self, id: &str, account: &str, who: &Sender, key: &str) -> (Vec<Action>, Vec<String>) {
        let mut problems = Vec::new();
        let Some(i) = self.plugins.iter().position(|p| p.id == id) else { return (Vec::new(), problems) };
        if !self.reads(i) {
            return (Vec::new(), problems);
        }
        match self.run(i, &mut problems, |b, s| b.call_on_sender_key(s, account, who, key)) {
            Some(Ok(actions)) => {
                let ok = self.permitted(i, actions, &mut problems);
                (ok, problems)
            }
            Some(Err(e)) => {
                problems.push(format!("{}: {e}", self.plugins[i].name()));
                (Vec::new(), problems)
            }
            None => (Vec::new(), problems),
        }
    }
}

fn allowed(a: &Action, grants: &[Permission]) -> bool {
    match a {
        Action::Notify(_) => true,
        Action::OpenDraft(_) => grants.contains(&Permission::Compose),
        _ => grants.contains(&Permission::ModifyMail),
    }
}

/// grants.toml: `enabled = true` and `permissions = "read-envelopes modify-mail"`.
/// Missing → written switched off, listing what the plugin asks for.
fn read_grants(dir: &Path, requested: &[Permission]) -> (bool, Vec<Permission>) {
    let path = dir.join("grants.toml");
    let Ok(text) = std::fs::read_to_string(&path) else {
        write_grants(dir, false, &[], requested).ok();
        return (false, Vec::new());
    };
    let m = crate::files::parse(&text);
    let enabled = m.get("enabled").is_some_and(|v| v == "true");
    let named: Vec<String> = m.get("permissions").map(|v| v.split_whitespace().map(str::to_string).collect()).unwrap_or_default();
    // Only what the plugin asked for can be granted.
    let grants = requested.iter().copied().filter(|p| named.iter().any(|n| n == permission_name(*p))).collect();
    (enabled, grants)
}

pub fn write_grants(dir: &Path, enabled: bool, grants: &[Permission], requested: &[Permission]) -> Result<()> {
    let names = |ps: &[Permission]| ps.iter().map(|p| permission_name(*p)).collect::<Vec<_>>().join(" ");
    let text = format!(
        "# What this plugin may do. Settings → Plugins changes it too.\n\
         # It asks for: {}\n\
         enabled = {enabled}\n\
         permissions = {}\n",
        if requested.is_empty() { "nothing".to_string() } else { names(requested) },
        crate::files::quote(&names(grants))
    );
    std::fs::write(dir.join("grants.toml"), text)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::wax::plugin::types::SenderDecision;
    use super::{allowed, write_grants, Action, Address, Envelope, Host, Permission, Sender};
    use std::path::{Path, PathBuf};

    /// Copy a built test plugin into a scratch plugins folder. Skips (returns
    /// None) when the .wasm hasn't been built: scripts/build-plugins.sh.
    fn install(root: &Path, crate_dir: &str, file: &str, on: Option<&[Permission]>, config: &str) -> Option<()> {
        let wasm = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("plugins").join(crate_dir).join("target/wasm32-wasip2/release").join(file);
        if !wasm.exists() {
            eprintln!("skipping: {} not built", wasm.display());
            return None;
        }
        let dir = root.join(crate_dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::copy(&wasm, dir.join("plugin.wasm")).unwrap();
        std::fs::write(dir.join("config.toml"), config).unwrap();
        if let Some(g) = on {
            write_grants(&dir, true, g, g).unwrap();
        }
        Some(())
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wax-plugin-test-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn mail(subject: &str, from: &str) -> Envelope {
        Envelope {
            account: "me@example.com".into(),
            mailbox: "INBOX".into(),
            id: "1".into(),
            message_id: None,
            from: vec![Address { name: None, email: from.into() }],
            to: Vec::new(),
            subject: subject.into(),
            date: None,
            flags: Vec::new(),
            labels: Vec::new(),
        }
    }

    #[test]
    fn actions_need_their_permission() {
        assert!(allowed(&Action::Notify("hi".into()), &[]));
        assert!(!allowed(&Action::Archive, &[Permission::ReadEnvelopes]));
        assert!(allowed(&Action::Archive, &[Permission::ModifyMail]));
    }

    #[test]
    fn new_plugins_start_switched_off() {
        let root = scratch("off");
        let Some(()) = install(&root, "filter-rules", "wax_filter_rules.wasm", None, "subject contains \"x\" => archive\n") else { return };
        let mut h = Host::new().unwrap();
        h.load_from(&root);
        assert_eq!(h.plugins.len(), 1);
        assert!(!h.plugins[0].enabled);
        assert!(root.join("filter-rules/grants.toml").exists());
        let (acts, _) = h.arrived(&mail("x", "a@b.c"));
        assert!(acts.is_empty());
    }

    #[test]
    fn filter_rules_runs_in_the_sandbox() {
        let root = scratch("rules");
        let g = [Permission::ReadEnvelopes, Permission::ModifyMail];
        let cfg = "subject contains \"invoice\" => label \"Receipts\"\nfrom ends-with \"@spam.test\" => archive\n";
        let Some(()) = install(&root, "filter-rules", "wax_filter_rules.wasm", Some(&g), cfg) else { return };
        let mut h = Host::new().unwrap();
        h.load_from(&root);
        assert_eq!(h.plugins[0].name(), "Filter rules");
        assert!(h.plugins[0].running(), "{:?}", h.plugins[0].broken);
        let (acts, problems) = h.arrived(&mail("Your Invoice #4", "shop@example.com"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(acts, vec![("filter-rules".to_string(), vec![Action::AddLabel("Receipts".into())])]);
        let (acts, _) = h.arrived(&mail("hello", "x@spam.test"));
        assert_eq!(acts[0].1, vec![Action::Archive]);
        let (acts, _) = h.arrived(&mail("hello", "friend@example.com"));
        assert!(acts.is_empty());
    }

    #[test]
    fn actions_beyond_the_grants_are_refused() {
        let root = scratch("refused");
        // Runaway asks only to read; its Archive must not get through.
        let Some(()) = install(&root, "test-runaway", "wax_test_runaway.wasm", Some(&[Permission::ReadEnvelopes]), "") else { return };
        let mut h = Host::new().unwrap();
        h.load_from(&root);
        let (acts, problems) = h.arrived(&mail("hi", "a@b.c"));
        assert!(acts.is_empty());
        assert!(problems[0].contains("refused"), "{problems:?}");
    }

    #[test]
    fn a_runaway_plugin_is_stopped_and_switched_off() {
        let root = scratch("runaway");
        let Some(()) = install(&root, "test-runaway", "wax_test_runaway.wasm", Some(&[Permission::ReadEnvelopes]), "") else { return };
        for subject in ["loop", "grab"] {
            let mut h = Host::new().unwrap();
            h.load_from(&root);
            let started = std::time::Instant::now();
            let (acts, problems) = h.arrived(&mail(subject, "a@b.c"));
            assert!(acts.is_empty());
            assert!(problems[0].contains("stopped"), "{subject}: {problems:?}");
            assert!(started.elapsed().as_secs() < 20, "{subject} took {:?}", started.elapsed());
            assert!(!h.plugins[0].running());
            // Once off, it isn't called again.
            let (_, problems) = h.arrived(&mail(subject, "a@b.c"));
            assert!(problems.is_empty());
        }
    }

    fn sender(email: &str, rule: Option<&str>) -> Sender {
        Sender { email: email.into(), name: "Shop".into(), count: 40, unread: 30, latest: 0, rule: rule.map(str::to_string) }
    }

    #[test]
    fn sweep_plugin_view_and_keys() {
        let root = scratch("sweep");
        let g = [Permission::ReadEnvelopes, Permission::ModifyMail];
        let cfg = "r = \"Reading\"  # newsletters\nn = \"News\"\ni = \"keep\"\n";
        let Some(()) = install(&root, "sweep", "wax_sweep.wasm", Some(&g), cfg) else { return };
        let mut h = Host::new().unwrap();
        h.load_from(&root);
        // No mail cache in the test: the view opens empty and says why.
        let (view, problems) = h.sender_view("me@example.com");
        let (id, view) = view.expect("a sender view");
        assert_eq!(id, "sweep");
        assert!(view.rows.is_empty());
        assert!(problems[0].contains("cache"), "{problems:?}");
        let keys: Vec<(String, String, String)> = view.keys.iter().map(|k| (k.key.clone(), k.label.clone(), k.about.clone())).collect();
        assert_eq!(keys, [("r".into(), "Reading".into(), "newsletters".into()), ("n".into(), "News".into(), "".into()), ("i".into(), "Keep".into(), "".into())]);
        let (acts, problems) = h.sender_key("sweep", "me@example.com", &sender("shop@x.com", None), "n");
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(acts, vec![Action::FileSender(SenderDecision { email: "shop@x.com".into(), name: "Shop".into(), count: 40, destination: Some("News".into()) })]);
        let (acts, _) = h.sender_key("sweep", "me@example.com", &sender("shop@x.com", None), "i");
        assert!(matches!(&acts[..], [Action::FileSender(SenderDecision { destination: None, .. })]));
        // Same decision again, or a key it doesn't have: nothing.
        assert!(h.sender_key("sweep", "me@example.com", &sender("shop@x.com", Some("News")), "n").0.is_empty());
        assert!(h.sender_key("sweep", "me@example.com", &sender("shop@x.com", None), "z").0.is_empty());
    }

    #[test]
    fn sweep_needs_modify_mail_to_file() {
        let root = scratch("sweep-ro");
        let Some(()) = install(&root, "sweep", "wax_sweep.wasm", Some(&[Permission::ReadEnvelopes]), "") else { return };
        let mut h = Host::new().unwrap();
        h.load_from(&root);
        let (acts, problems) = h.sender_key("sweep", "me@example.com", &sender("shop@x.com", None), "r");
        assert!(acts.is_empty());
        assert!(problems[0].contains("refused"), "{problems:?}");
    }

    /// A cached message, as the app keeps them.
    fn cached(mailbox: &str, id: &str, from: &str, ts: i64) -> crate::model::Envelope {
        crate::model::Envelope {
            mailbox: mailbox.into(),
            id: id.into(),
            message_id: Some(format!("<{id}@x>")),
            from: "Someone".into(),
            from_email: from.to_string().into(),
            subject: "hi".into(),
            ts,
            date: "".into(),
            preview: "".into(),
            body: None,
            html: None,
            unread: true,
            starred: false,
            important: false,
            attachments: Vec::new(),
            account: Default::default(),
        }
    }

    #[test]
    fn screener_holds_strangers_only() {
        let root = scratch("screener");
        let g = [Permission::ReadEnvelopes, Permission::ModifyMail, Permission::Storage];
        let Some(()) = install(&root, "screener", "wax_screener.wasm", Some(&g), "screen = \"me@acme.test\"\n") else { return };
        let me = "me@acme.test";
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let store = crate::store::Store::open_at(&root.join("cache.sqlite")).unwrap();
        // Old mail from a friend, someone you wrote to, and today's batch.
        let batch = vec![
            cached("INBOX", "10", "new@shop.test", now),
            cached("INBOX", "11", "new@shop.test", now),
            cached("INBOX", "12", "friend@else.test", now),
            cached("INBOX", "13", "pen.pal@else.test", now),
            cached("INBOX", "14", "boss@acme.test", now),
            cached("INBOX", "15", "late@shop.test", now - 5 * 86400),
        ];
        let mut all = batch.clone();
        all.push(cached("[Gmail]/All Mail", "1", "friend@else.test", now - 86400 * 30));
        store.upsert_envelopes_ref(me, &all).unwrap();
        store.add_sent_contacts(me, &[("Pal".into(), "pen.pal@else.test".into(), now)]).unwrap();
        let mut h = Host::new().unwrap();
        h.store = Some(std::sync::Arc::new(std::sync::Mutex::new(store)));
        h.load_from(&root);
        assert!(h.plugins[0].running(), "{:?}", h.plugins[0].broken);
        h.arrival = super::Arrival {
            mine: vec![me.into()],
            batch: batch.iter().map(|e| (e.mailbox.clone(), e.id.clone())).collect(),
            batch_mids: batch.iter().filter_map(|e| e.message_id.clone()).collect(),
        };
        let held = |h: &mut Host, e: &crate::model::Envelope| {
            let date = chrono::DateTime::from_timestamp(e.ts, 0).map(|d| d.to_rfc3339());
            let m = Envelope { date, ..mail(&e.subject, &e.from_email) };
            let m = Envelope { account: me.into(), id: e.id.clone(), message_id: e.message_id.clone(), ..m };
            let (acts, problems) = h.arrived(&m);
            assert!(problems.is_empty(), "{problems:?}");
            acts.iter().any(|(_, a)| a.contains(&Action::MoveTo("Lobby".into())))
        };
        assert!(held(&mut h, &batch[0]), "a stranger");
        assert!(held(&mut h, &batch[1]), "the stranger's second message doesn't vouch for the first");
        assert!(!held(&mut h, &batch[2]), "you have older mail from them");
        assert!(!held(&mut h, &batch[3]), "you've written to them");
        assert!(!held(&mut h, &batch[4]), "a colleague");
        assert!(!held(&mut h, &batch[5]), "more than 3 days old");
        // Another account isn't screened.
        let other = Envelope { account: "other@x.test".into(), ..mail("hi", "new@shop.test") };
        assert!(h.arrived(&other).0.is_empty());
    }
}
