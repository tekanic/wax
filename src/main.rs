//! gpui-mail — Phase 0 spike.
//!
//! Three-pane mail UI on GPUI: mailboxes | virtualized thread list | reader.
//! Connects read-only to one IMAP account (app password from the Keychain);
//! falls back to 10,000 generated messages if that fails.
//! Keys: j / down = next, k / up = previous, cmd-q = quit.

mod accounts;
mod account_view;
mod actions;
mod assets;
mod attach;
mod blocks;
mod compose;
mod errlog;
mod compose_view;
mod config_watch;
mod files;
mod formatting;
mod html_view;
mod identity;
mod live;
mod login;
mod notify;
mod keys;
mod labels;
mod mail;
mod md;
mod model;
mod oauth;
mod problems;
mod rules;
mod store;
mod settings;
mod signature;
mod sweep;
mod table;

use gpui_kit::prelude::*;
use gpui_kit::*;
use html_view::HtmlView;
use mail::Session;
use compose::{Draft, Kind};
use compose_view::{ComposerState, RecipientField};
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea};
use formatting::Format;
use gpui_kit::component::tooltip::Tooltip;
use model::{make_preview, Envelope};
use std::collections::HashSet;
use store::Store;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use keys::*;

const PAGE_SIZE: u32 = 200;
/// Rows whose previews are fetched ahead of time.
const PREFETCH_AHEAD: usize = 24;
const SERVER_SEARCH_DELAY: std::time::Duration = std::time::Duration::from_millis(450);
const SERVER_SEARCH_LIMIT: u32 = 100;
/// UIDs per background-sync batch (Gmail UIDs are sparse: ~10% used).
const SYNC_WINDOW: u64 = 5_000;
const ALL_MAIL: &str = "[Gmail]/All Mail";
const SENT: &str = "[Gmail]/Sent Mail";
const ROW_HEIGHT: f32 = 78.;
const SIDEBAR_WIDTH: f32 = 210.;
const LIST_WIDTH: f32 = 360.;

// Palette (spike only; Phase 1 moves to the gpui-component theme).
const BG: u32 = 0x1b1b1f;
const SIDEBAR: u32 = 0x151518;
const LIST_BG: u32 = 0x18181c;
const BORDER: u32 = 0x2a2a31;
const HOVER: u32 = 0x23232a;
const TEXT: u32 = 0xe8e8ec;
const SUBTLE: u32 = 0xb4b4bd;
const MUTED: u32 = 0x82828d;
const ACCENT: u32 = 0x4f8ff7;
const SELECTED: u32 = 0x1f2a40;
const MARKED: u32 = 0x1a2233;
const ERROR: u32 = 0xf87171;
const SEARCH_BG: u32 = 0x26262e;
const SEARCH_BORDER: u32 = 0x3a3a44;

/// Icon (and optional tint) for a folder in the sidebar.
fn folder_icon(name: &str, system: bool) -> (&'static str, Option<u32>) {
    if !system {
        return ("icons/tag.svg", None);
    }
    match name {
        n if n.eq_ignore_ascii_case("INBOX") => ("icons/inbox.svg", None),
        sweep::READING => ("icons/book-open.svg", Some(0x4f8ff7)),
        sweep::RECORDS => ("icons/receipt.svg", Some(0xc9a227)),
        sweep::SCREENED => ("icons/ban.svg", Some(0xd06a6a)),
        "[Gmail]/Starred" => ("icons/star.svg", None),
        "[Gmail]/Important" => ("icons/flag.svg", None),
        "[Gmail]/Sent Mail" => ("icons/send.svg", None),
        "[Gmail]/Drafts" => ("icons/file-pen.svg", None),
        "[Gmail]/All Mail" => ("icons/mails.svg", None),
        "[Gmail]/Spam" => ("icons/octagon-alert.svg", None),
        "[Gmail]/Trash" => ("icons/trash.svg", None),
        _ => ("icons/tag.svg", None),
    }
}

/// A key chip and what it does.
fn hint_chip(keys: impl Into<SharedString>, label: impl Into<SharedString>) -> impl IntoElement {
    let (keys, label): (SharedString, SharedString) = (keys.into(), label.into());
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap_1p5()
        .child(
            div()
                .px_1p5()
                .rounded_sm()
                .bg(rgb(0x2c2c35))
                .border_1()
                .border_color(rgb(0x41414c))
                .text_color(rgb(TEXT))
                .font_family("Menlo")
                .child(keys),
        )
        .child(div().text_color(rgb(SUBTLE)).child(label))
}

fn store_upsert(store: &Store, account: &str, list: &[Envelope]) -> anyhow::Result<()> {
    store.upsert_envelopes_ref(account, list)
}

fn log_timing(what: &str, started: std::time::Instant) {
    if std::env::var("GPUI_MAIL_DEBUG").is_ok() {
        eprintln!("{what}: {:?}", started.elapsed());
    }
}

/// Gmail system folders, in the order a mail client shows them.
const SYSTEM_ORDER: &[(&str, &str)] = &[
    ("INBOX", "Inbox"),
    ("Reading", "Reading"),
    ("Records", "Records"),
    ("[Gmail]/Starred", "Starred"),
    ("[Gmail]/Important", "Important"),
    ("[Gmail]/Sent Mail", "Sent"),
    ("[Gmail]/Drafts", "Drafts"),
    ("[Gmail]/All Mail", "All Mail"),
    ("[Gmail]/Spam", "Spam"),
    ("[Gmail]/Trash", "Trash"),
    ("Screened Out", "Screened Out"),
];

#[derive(Clone)]
struct MailboxItem {
    /// Name used on the server.
    name: SharedString,
    /// Name shown in the sidebar.
    label: SharedString,
    system: bool,
    /// False for Sweep destinations not created in Gmail yet.
    exists: bool,
}

/// 3969 → "3,969".
fn group_digits(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn mailbox_items(names: Vec<String>) -> Vec<MailboxItem> {
    let mut system: Vec<(usize, MailboxItem)> = Vec::new();
    let mut labels: Vec<MailboxItem> = Vec::new();
    for name in names {
        let known = SYSTEM_ORDER
            .iter()
            .position(|(server, _)| server.eq_ignore_ascii_case(&name));
        match known {
            Some(order) => system.push((
                order,
                MailboxItem { name: name.into(), label: SYSTEM_ORDER[order].1.into(), system: true, exists: true },
            )),
            None => {
                let label = name.trim_start_matches("[Gmail]/").to_string();
                labels.push(MailboxItem { name: name.into(), label: label.into(), system: false, exists: true });
            }
        }
    }
    // The Sweep's destinations are always listed, even before first use.
    for dest in [sweep::READING, sweep::RECORDS, sweep::SCREENED] {
        if !system.iter().any(|(_, m)| m.name.eq_ignore_ascii_case(dest)) {
            if let Some(order) = SYSTEM_ORDER.iter().position(|(n, _)| *n == dest) {
                system.push((order, MailboxItem { name: dest.into(), label: dest.into(), system: true, exists: false }));
            }
        }
    }
    system.sort_by_key(|(order, _)| *order);
    labels.sort_by_key(|m| m.label.to_lowercase());
    system.into_iter().map(|(_, m)| m).chain(labels).collect()
}

/// Seconds a sent message waits before it really goes (⌘Z cancels).
const UNDO_SEND_SECS: u64 = 8;

/// Overlay state once the countdown ends: honest about the network.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AfterSend {
    /// Handed to SMTP, not yet confirmed.
    Sending,
    /// Gmail accepted it.
    Sent,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BulkKind {
    Archive,
    Delete,
    /// Back to the Inbox from Trash, Spam or Screened Out.
    ToInbox,
    /// Delete forever (from Trash or Spam).
    Purge,
}

impl BulkKind {
    fn dest(self) -> &'static str {
        match self {
            BulkKind::Archive => ALL_MAIL,
            BulkKind::Delete => mail::TRASH,
            BulkKind::ToInbox => mail::INBOX,
            BulkKind::Purge => mail::TRASH,
        }
    }

    fn done(self) -> &'static str {
        match self {
            BulkKind::Archive => "Archived",
            BulkKind::Delete => "Deleted",
            BulkKind::ToInbox => "Moved to the Inbox:",
            BulkKind::Purge => "Deleting forever:",
        }
    }
}

/// Archive/delete held back for the undo window. The rows are already
/// gone from the list; Gmail isn't touched until the window ends.
struct PendingBulk {
    id: u64,
    kind: BulkKind,
    items: Vec<Envelope>,
    deadline: std::time::Instant,
    _task: Task<()>,
}

struct PendingSend {
    draft: Draft,
    deadline: std::time::Instant,
    _task: Task<()>,
}

/// A small text button.
fn action_button(id: impl Into<ElementId>, label: impl Into<SharedString>, primary: bool) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .when(primary, |d| d.bg(rgb(ACCENT)).text_color(rgb(0xffffff)).hover(|d| d.bg(rgb(0x6aa1f9))))
        .when(!primary, |d| d.bg(rgb(0x3a3a44)).text_color(rgb(TEXT)).hover(|d| d.bg(rgb(0x55555f))))
        .child(label.into())
}

type SharedSession = Arc<Mutex<Session>>;
type SharedStore = Arc<Mutex<Store>>;

struct MailApp {
    /// The account on screen ("" until one is added).
    account: Arc<str>,
    accounts: Vec<accounts::Account>,
    /// A Google sign-in waiting in the browser (set to cancel it).
    signing_in: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// "Empty Trash" was clicked once and waits for the second click.
    empty_armed: bool,
    /// Connections of the accounts not on screen (for All Inboxes and
    /// acting on their messages), and those being opened.
    pool: std::collections::HashMap<String, SharedSession>,
    pool_connecting: HashSet<String>,
    /// The list shows every account's Inbox together.
    unified: bool,
    /// The status text as last drawn, and when it appeared (so it fades).
    status_seen: SharedString,
    /// Failed attempts to connect the account on screen (for backing off).
    connect_attempts: u32,
    /// Problems logged since Settings → Problems was last opened.
    problems_unseen: usize,
    /// Settings → Problems: the log as read, and the account it's filtered to.
    problems: Vec<errlog::Entry>,
    problems_filter: Option<String>,
    /// The `l` label picker, when open.
    label_picker: Option<labels::LabelPicker>,
    /// Settings → Labels.
    labels_pane: Option<labels::LabelsPane>,
    session: Option<SharedSession>,
    /// Second connection used only to prefetch previews, so it never
    /// blocks the message you clicked.
    preview_session: Option<SharedSession>,
    store: Option<SharedStore>,
    mailboxes: Vec<MailboxItem>,
    active_mailbox: usize,
    /// What the list shows: a mailbox, or search results.
    envelopes: Vec<Envelope>,
    selected: usize,
    loading: bool,
    online: bool,
    prefetching: HashSet<String>,
    search: Entity<InputState>,
    query: String,
    /// A Gmail-side search for `query` is in flight.
    server_searching: bool,
    /// The folder has older messages than the ones listed.
    has_more: bool,
    loading_more: bool,
    /// Progress of the background header sync, shown under the status.
    sync_note: Option<SharedString>,
    status: SharedString,
    status_is_error: bool,
    scroll: UniformListScrollHandle,
    focus: FocusHandle,
    html_view: HtmlView,
    /// Messages whose remote images the user chose to load.
    images_allowed: HashSet<(String, String)>,
    /// Show shortcut hints in the bottom bar (cmd-/ toggles).
    show_hints: bool,
    /// The message being written, shown in place of the reader.
    composer: Option<ComposerState>,
    compose_subs: Vec<Subscription>,
    autosave_gen: u64,
    /// A sent message waiting out its undo window.
    pending_send: Option<PendingSend>,
    /// Focus target of the countdown overlay (its keys live there).
    countdown_focus: FocusHandle,
    /// What the review banner shows after the countdown.
    after_send: Option<AfterSend>,
    /// The message being reviewed during (and right after) its countdown.
    review_draft: Option<Draft>,
    /// Rows marked with x / cmd-a, by (mailbox, id).
    marked: HashSet<(String, String)>,
    /// Archive/delete batches waiting out their undo window (newest last).
    pending_bulk: Vec<PendingBulk>,
    next_bulk_id: u64,
    /// The Sweep view, when open.
    sweep: Option<sweep::SweepState>,
    pending_sweep: Vec<sweep::PendingSweep>,
    /// Sender rules (email → "keep" or label), applied to new Inbox mail.
    rules: std::collections::HashMap<String, String>,
    /// Sender names shown beside rules in rules.toml.
    rule_names: std::collections::HashMap<String, String>,
    /// rules.toml's modification time when last read.
    rules_seen: Option<std::time::SystemTime>,
    /// Message counts of the Sweep's labels, shown in the sidebar.
    /// Server counts per folder of the account on screen: (total, unread).
    label_counts: std::collections::HashMap<String, (u64, u64)>,
    /// When the files in ~/.config/wax last changed (config_watch.rs).
    config_seen: config_watch::Seen,
    /// App defaults (font, layout, default signatures).
    settings: settings::Settings,
    signatures: Vec<signature::Signature>,
    /// Extra From addresses (the account's own is implied).
    identities: Vec<identity::Identity>,
    /// Blocks offered beside the composer.
    blocks: Vec<blocks::Block>,
    /// The settings screen, when open.
    settings_view: Option<settings::SettingsView>,
    /// The new-mail watcher.
    /// One new-mail watcher per account.
    watch: std::collections::HashMap<String, live::WatchState>,
    /// Right-click menu over the message list.
    row_menu: Option<actions::RowMenu>,
    /// Row being swiped sideways.
    swipe: Option<actions::Swipe>,
    /// Ignore sideways scrolling until then (momentum after a swipe).
    swipe_quiet_until: std::time::Instant,
    _subscriptions: Vec<Subscription>,
}

impl MailApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("Search mail");
            // Dev convenience: GPUI_MAIL_SEARCH="terms" opens with a search.
            if let Ok(q) = std::env::var("GPUI_MAIL_SEARCH") {
                state.set_value(q, window, cx);
            }
            state
        });
        let subscription = cx.subscribe_in(&search, window, |this, state, event, _window, cx| {
            if let InputEvent::Change = event {
                let query = state.read(cx).value().trim().to_string();
                this.run_search(query, cx);
            }
        });

        let pending = cx.observe_pending_input(window, |_, _, cx| cx.notify());
        let store = Store::open().ok().map(|s| Arc::new(Mutex::new(s)));
        let mut app = Self {
            account: "".into(),
            accounts: Vec::new(),
            signing_in: None,
            empty_armed: false,
            pool: Default::default(),
            pool_connecting: HashSet::new(),
            unified: false,
            label_picker: None,
            labels_pane: None,
            status_seen: SharedString::default(),
            problems_unseen: 0,
            connect_attempts: 0,
            problems: Vec::new(),
            problems_filter: None,
            session: None,
            preview_session: None,
            store,
            mailboxes: mailbox_items(vec!["INBOX".into()]),
            active_mailbox: 0,
            envelopes: Vec::new(),
            selected: 0,
            loading: true,
            online: false,
            prefetching: HashSet::new(),
            search,
            query: String::new(),
            server_searching: false,
            has_more: false,
            loading_more: false,
            sync_note: None,
            status: "Connecting…".into(),
            status_is_error: false,
            scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            html_view: HtmlView::new(),
            images_allowed: HashSet::new(),
            show_hints: true,
            composer: None,
            compose_subs: Vec::new(),
            autosave_gen: 0,
            pending_send: None,
            countdown_focus: cx.focus_handle(),
            after_send: None,
            review_draft: None,
            marked: HashSet::new(),
            pending_bulk: Vec::new(),
            next_bulk_id: 0,
            sweep: None,
            pending_sweep: Vec::new(),
            rules: Default::default(),
            rule_names: Default::default(),
            rules_seen: None,
            label_counts: Default::default(),
            config_seen: Default::default(),
            settings: settings::Settings::default(),
            signatures: Vec::new(),
            identities: Vec::new(),
            blocks: Vec::new(),
            settings_view: None,
            watch: Default::default(),
            row_menu: None,
            swipe: None,
            swipe_quiet_until: std::time::Instant::now(),
            _subscriptions: vec![subscription, pending],
        };
        if let Some(store) = &app.store {
            let moved = files::migrate_from_db(&store.lock().unwrap());
            if moved > 0 {
                app.status = format!("Moved {moved} signature(s) to {}", files::signatures_dir().display()).into();
            }
        }
        app.settings = settings::Settings::load();
        let cached = app.store.as_ref().map(|s| s.lock().unwrap().cached_accounts()).unwrap_or_default();
        accounts::migrate(&app.settings, &cached);
        app.accounts = accounts::load();
        app.signatures = files::load_signatures();
        app.identities = identity::load();
        app.assign_signatures();
        app.watch_config(cx);
        match app.accounts.first().map(|a| a.email.clone()) {
            Some(first) => {
                app.account = first.as_str().into();
                app.use_account_settings();
                app.load_rules();
                app.load_from_cache(cx);
                app.connect(cx);
                app.start_watches(cx);
            }
            None => {
                // A new install: nothing to show until an account is added.
                app.loading = false;
                app.status = "Add an account to get started".into();
                app.open_settings(settings::Pane::Accounts, window, cx);
            }
        }
        // An empty autosave isn't worth reopening.
        let unsent = app.store.as_ref().and_then(|s| s.lock().unwrap().load_snapshot()).filter(|d| !d.is_blank());
        if let Some(draft) = unsent {
            app.open_composer(draft, window, cx);
            app.status = "Restored the message you were writing".into();
        }
        // Dev convenience: GPUI_MAIL_COMPOSE=file.md opens the composer on
        // it (GPUI_MAIL_PREVIEW=1 starts in preview).
        if let Some(body) = std::env::var("GPUI_MAIL_COMPOSE").ok().and_then(|p| std::fs::read_to_string(p).ok()) {
            let mut d = Draft::new();
            d.to = "test@example.com".into();
            d.subject = "Markdown test".into();
            d.body = body;
            let d = app.with_defaults(d);
            app.open_composer(d, window, cx);
            if std::env::var("GPUI_MAIL_PREVIEW").is_ok() {
                app.toggle_preview(window, cx);
            }
        }
        // Dev convenience: GPUI_MAIL_OPEN=settings | signatures.
        match std::env::var("GPUI_MAIL_OPEN").as_deref() {
            Ok("settings") => app.open_settings(settings::Pane::Writing, window, cx),
            Ok("signatures") => app.open_settings(settings::Pane::Signatures, window, cx),
            Ok("addresses") => app.open_settings(settings::Pane::Addresses, window, cx),
            Ok("accounts") => app.open_settings(settings::Pane::Accounts, window, cx),
            _ => {}
        }
        let initial = app.search.read(cx).value().trim().to_string();
        if !initial.is_empty() {
            app.run_search(initial, cx);
        }
        app
    }

    fn active(&self) -> Option<&MailboxItem> {
        self.mailboxes.get(self.active_mailbox)
    }

    /// Show whatever the cache has right away, before the network answers.
    fn load_from_cache(&mut self, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        let Some(store) = self.store.clone() else { return };
        let store = store.lock().unwrap();
        let names = store.mailboxes(&*acct).unwrap_or_default();
        if names.is_empty() {
            return;
        }
        self.mailboxes = mailbox_items(names);
        self.active_mailbox = 0;
        if let Some(first) = self.active().map(|m| m.name.to_string()) {
            let cached = store.envelopes(&*acct, &first, PAGE_SIZE).unwrap_or_default();
            drop(store);
            if !cached.is_empty() {
                self.status = "Updating…".into();
                self.set_envelopes(cached, cx);
            }
        }
    }

    fn connect(&mut self, cx: &mut Context<Self>) {
        if self.account.is_empty() {
            return;
        }
        let acct = self.account.clone();
        let for_preview = acct.clone();
        // The preview/search connection logs in in parallel with the main one.
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let started = std::time::Instant::now();
            let a = for_preview.clone();
            let preview = cx
                .background_executor()
                .spawn(async move { Session::connect(&a) })
                .await;
            let Ok(preview) = preview else { return };
            this.update(cx, |this, cx| {
                if this.account != for_preview {
                    return; // switched to another account meanwhile
                }
                log_timing("preview connection", started);
                this.preview_session = Some(Arc::new(Mutex::new(preview)));
                // Catch up on work that was waiting for it.
                this.prefetch(0..PREFETCH_AHEAD, cx);
                if !this.query.is_empty() {
                    let query = this.query.clone();
                    this.search_server(query, cx);
                }
            })
            .ok();
        })
        .detach();

        let started = std::time::Instant::now();
        let store = self.store.clone();
        let first = self.active().map(|m| m.name.to_string());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let a = acct.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let acct = a;
                    let mut session = Session::connect(&*acct)?;
                    let names = session.mailboxes()?;
                    let items = mailbox_items(names.clone());
                    // Keep showing the folder the cache opened on.
                    let target = first
                        .filter(|f| items.iter().any(|m| m.name.as_ref() == f))
                        .or_else(|| items.first().map(|m| m.name.to_string()))
                        .unwrap_or_else(|| "INBOX".into());
                    let fresh = session.envelopes(&target, PAGE_SIZE)?;
                    let list = match &store {
                        Some(store) => {
                            let mut store = store.lock().unwrap();
                            store.save_mailboxes(&*acct, &names)?;
                            store.save_envelopes(&*acct, &target, &fresh)?;
                            store.envelopes(&*acct, &target, PAGE_SIZE)?
                        }
                        None => fresh,
                    };
                    anyhow::Ok((session, items, target, list))
                })
                .await;
            this.update(cx, |this, cx| match result {
                _ if this.account != acct => {} // switched to another account meanwhile
                Ok((session, items, target, list)) => {
                    this.connect_attempts = 0;
                    this.session = Some(Arc::new(Mutex::new(session)));
                    this.online = true;
                    this.mailboxes = items;
                    this.active_mailbox = this
                        .mailboxes
                        .iter()
                        .position(|m| m.name.as_ref() == target)
                        .unwrap_or(0);
                    log_timing("main connection + refresh", started);
                    this.start_sync(cx);
                    this.refresh_folders(cx);
                    this.status = acct.to_string().into();
                    this.status_is_error = false;
                    if this.query.is_empty() {
                        let list = this.apply_rules_to_inbox(list, cx);
                        this.set_envelopes(list, cx);
                    } else {
                        // A search typed while connecting: ask Gmail now.
                        let query = this.query.clone();
                        this.search_server(query, cx);
                    }
                }
                Err(err) => {
                    this.status_is_error = true;
                    // Try again by itself: 15 s, 30 s, 1 min, then every 2 min.
                    // (Gmail refuses logins while too many connections are open
                    // across all your mail apps; that passes.)
                    this.connect_attempts += 1;
                    let wait = 15u64 << (this.connect_attempts - 1).min(3);
                    let again = acct.clone();
                    cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                        cx.background_executor().timer(std::time::Duration::from_secs(wait)).await;
                        this.update(cx, |this, cx| {
                            if this.account == again && this.session.is_none() {
                                this.connect(cx);
                            }
                        })
                        .ok();
                    })
                    .detach();
                    if this.envelopes.is_empty() {
                        this.status = format!("Could not connect to {acct}: {err:#}").into();
                        this.loading = false;
                        cx.notify();
                    } else {
                        this.status = format!("Offline, showing cached mail. {err:#}").into();
                        this.loading = false;
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn set_envelopes(&mut self, envelopes: Vec<Envelope>, cx: &mut Context<Self>) {
        // Keep the selected message selected across a refresh.
        let keep = self
            .envelopes
            .get(self.selected)
            .map(|e| e.key());
        self.has_more = self.query.is_empty() && envelopes.len() >= PAGE_SIZE as usize;
        self.loading_more = false;
        self.envelopes = envelopes;
        self.prefetching.clear();
        self.loading = false;
        self.selected = keep
            .and_then(|k| self.envelopes.iter().position(|e| e.key() == k))
            .unwrap_or(0);
        self.scroll.scroll_to_item(self.selected, ScrollStrategy::Top);
        self.load_body(self.selected, cx);
        self.prefetch(0..PREFETCH_AHEAD, cx);
        cx.notify();
    }

    fn run_search(&mut self, query: String, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        if query == self.query {
            return;
        }
        self.query = query.clone();
        if query.is_empty() {
            self.selected = 0;
            self.envelopes.clear();
            self.open_mailbox(self.active_mailbox, cx);
            return;
        }
        let query_for_server = query.clone();
        self.server_searching = false;
        let Some(store) = self.store.clone() else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let q = query.clone();
            let result = cx
                .background_executor()
                .spawn(async move { store.lock().unwrap().search(&*acct, &q, PAGE_SIZE * 2) })
                .await;
            this.update(cx, |this, cx| {
                if this.query != query {
                    return; // a newer keystroke superseded this search
                }
                this.selected = 0;
                this.envelopes.clear();
                this.set_envelopes(result.unwrap_or_default(), cx);
            })
            .ok();
        })
        .detach();
        self.search_server(query_for_server, cx);
    }

    /// After a pause in typing, ask Gmail too (it sees the whole mailbox,
    /// not just the cache) and merge its hits into the local results.
    fn search_server(&mut self, query: String, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        let Some(session) = self.preview_session.clone().or_else(|| self.session.clone()) else {
            return;
        };
        let mailbox = self
            .mailboxes
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case("[Gmail]/All Mail"))
            .or(self.active())
            .map(|m| m.name.to_string());
        let Some(mailbox) = mailbox else { return };
        let store = self.store.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(SERVER_SEARCH_DELAY).await;
            let current = this
                .update(cx, |this, cx| {
                    let current = this.query == query;
                    if current {
                        this.server_searching = true;
                        cx.notify();
                    }
                    current
                })
                .unwrap_or(false);
            if !current {
                return;
            }
            let q = query.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let hits = session.lock().unwrap().search(&mailbox, &q, SERVER_SEARCH_LIMIT)?;
                    let Some(store) = store else { return anyhow::Ok(hits) };
                    let mut store = store.lock().unwrap();
                    store.upsert_envelopes(&*acct, &hits)?;
                    // Pick up previews/bodies already cached for these messages.
                    anyhow::Ok(
                        hits.into_iter()
                            .map(|h| store.lookup(&*acct, &h.mailbox, &h.id).unwrap_or(h))
                            .collect::<Vec<_>>(),
                    )
                })
                .await;
            this.update(cx, |this, cx| {
                if this.query != query {
                    return;
                }
                this.server_searching = false;
                match result {
                    Ok(hits) => this.merge_results(hits, cx),
                    Err(err) => {
                        this.status = format!("Gmail search failed: {err:#}").into();
                        this.status_is_error = true;
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn merge_results(&mut self, hits: Vec<Envelope>, cx: &mut Context<Self>) {
        let key = |e: &Envelope| e.message_id.clone().unwrap_or_else(|| format!("{}/{}", e.mailbox, e.id));
        let mut seen: HashSet<String> = self.envelopes.iter().map(key).collect();
        let mut merged = self.envelopes.clone();
        for hit in hits {
            if seen.insert(key(&hit)) {
                merged.push(hit);
            }
        }
        merged.sort_by(|a, b| b.ts.cmp(&a.ts));
        self.set_envelopes(merged, cx);
    }

    fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |state, cx| state.set_value("", window, cx));
        self.run_search(String::new(), cx);
        self.focus.focus(window, cx);
    }

    fn on_clear_search(&mut self, _: &ClearSearch, window: &mut Window, cx: &mut Context<Self>) {
        if !self.marked.is_empty() {
            self.marked.clear();
            cx.notify();
            return;
        }
        if !self.query.is_empty() || !self.search.read(cx).value().is_empty() {
            self.clear_search(window, cx);
        }
    }

    // ----- Writing -----------------------------------------------------

    fn open_composer(&mut self, draft: Draft, window: &mut Window, cx: &mut Context<Self>) {
        let state = ComposerState::new(&draft, window, cx);
        let mut subs = Vec::new();
        for (input, field) in [
            (state.to.clone(), RecipientField::To),
            (state.cc.clone(), RecipientField::Cc),
            (state.bcc.clone(), RecipientField::Bcc),
        ] {
            subs.push(cx.subscribe(&input, move |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.schedule_autosave(cx);
                    this.update_suggestions(field, cx);
                }
                InputEvent::Blur => {
                    if let Some(c) = &mut this.composer {
                        if c.suggest_for == Some(field) {
                            c.clear_suggestions();
                            cx.notify();
                        }
                    }
                }
                _ => {}
            }));
        }
        subs.push(cx.subscribe(&state.subject, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.schedule_autosave(cx);
            }
        }));
        subs.push(cx.subscribe(&state.body, |this, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.schedule_autosave(cx);
            }
        }));
        state.focus_first(window, cx);
        self.reload_blocks();
        self.composer = Some(state);
        self.compose_subs = subs;
        self.save_snapshot_now(cx);
        cx.notify();
    }

    /// ⌘⇧B: open the Bcc line and put the cursor in it; if it's open and
    /// empty, close it again.
    fn toggle_bcc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        if c.show_bcc && c.bcc.read(cx).value().trim().is_empty() {
            self.hide_bcc(window, cx);
        } else {
            self.show_bcc(window, cx);
        }
    }

    /// Open the Bcc line and put the cursor in it.
    fn show_bcc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.composer {
            c.show_bcc = true;
            let bcc = c.bcc.clone();
            bcc.update(cx, |s, cx| s.focus(window, cx));
            cx.notify();
        }
    }

    /// Close the Bcc line. Anything typed in it is removed, so nothing is
    /// sent to an address you can no longer see.
    fn hide_bcc(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &mut self.composer {
            c.show_bcc = false;
            if c.suggest_for == Some(RecipientField::Bcc) {
                c.clear_suggestions();
            }
            let (bcc, cc) = (c.bcc.clone(), c.cc.clone());
            bcc.update(cx, |s, cx| s.set_value("", window, cx));
            cc.update(cx, |s, cx| s.focus(window, cx));
            self.schedule_autosave(cx);
            cx.notify();
        }
    }

    /// Look up addresses for the word being typed in To, Cc or Bcc.
    fn update_suggestions(&mut self, field: RecipientField, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        let Some(c) = &mut self.composer else { return };
        let token = c.current_token(field, cx);
        if token.chars().count() < 1 || token.ends_with('>') {
            if c.suggest_for == Some(field) {
                c.clear_suggestions();
                cx.notify();
            }
            return;
        }
        let Some(store) = self.store.clone() else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let t = token.clone();
            let found = cx
                .background_executor()
                .spawn(async move { store.lock().unwrap().suggest_contacts(&*acct, &t, 6).unwrap_or_default() })
                .await;
            this.update(cx, |this, cx| {
                let Some(c) = &mut this.composer else { return };
                if c.current_token(field, cx) != token {
                    return; // typing moved on
                }
                // Nothing to suggest once the typed text is exactly an address.
                let exact = found.len() == 1 && found[0].1.eq_ignore_ascii_case(&token);
                if found.is_empty() || exact {
                    c.clear_suggestions();
                } else {
                    c.suggestions = found;
                    c.suggest_ix = 0;
                    c.suggest_for = Some(field);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn suggest_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(c) = &mut self.composer else { return };
        if c.suggestions.is_empty() {
            cx.propagate();
            return;
        }
        let n = c.suggestions.len() as isize;
        c.suggest_ix = ((c.suggest_ix as isize + delta).rem_euclid(n)) as usize;
        cx.notify();
    }

    fn accept_suggestion(&mut self, _: &AcceptSuggestion, window: &mut Window, cx: &mut Context<Self>) {
        let accepted = match &mut self.composer {
            Some(c) => c.accept(window, cx),
            None => false,
        };
        if accepted {
            cx.notify();
        } else {
            // No list open: tab keeps its usual meaning.
            window.focus_next(cx);
        }
    }

    fn schedule_autosave(&mut self, cx: &mut Context<Self>) {
        self.autosave_gen += 1;
        let generation = self.autosave_gen;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_millis(800)).await;
            this.update(cx, |this, cx| {
                if this.autosave_gen == generation {
                    this.save_snapshot_now(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn save_snapshot_now(&self, cx: &mut Context<Self>) {
        let (Some(c), Some(store)) = (&self.composer, self.store.clone()) else { return };
        let draft = c.draft(cx);
        cx.background_executor()
            .spawn(async move { store.lock().unwrap().save_snapshot(Some(&draft)).ok() })
            .detach();
    }

    fn clear_snapshot(&self, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        cx.background_executor()
            .spawn(async move { store.lock().unwrap().save_snapshot(None).ok() })
            .detach();
    }

    fn close_composer_ui(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Draft> {
        let draft = self.composer.take().map(|c| c.draft(cx));
        self.compose_subs.clear();
        self.autosave_gen += 1; // cancel pending autosaves
        self.focus.focus(window, cx);
        cx.notify();
        draft
    }

    fn compose(&mut self, _: &Compose, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(c) = &self.composer {
            c.focus_first(window, cx);
            return;
        }
        let d = self.with_defaults(Draft::new());
        self.open_composer(d, window, cx);
    }

    /// Reply, reply all or forward the selected message.
    fn respond(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            return;
        }
        let Some(e) = self.envelopes.get(self.selected) else { return };
        // From All Inboxes, answer from the message's own account.
        let account = self.account_of(e).to_string();
        let (mailbox, id) = (e.mailbox.clone(), e.id.clone());
        let Some(session) = self.session_of(&account) else { return };
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let result = cx
                .background_executor()
                .spawn(async move { session.lock().unwrap().original(&mailbox, &id) })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(orig) => {
                    let mine: Vec<String> = this.identities_for(&account).into_iter().map(|i| i.email).collect();
                    let mut d = compose::prefill(kind, &orig, &mine);
                    d.account = account.clone();
                    // Answer from the address it was sent to.
                    if kind != Kind::Forward {
                        let all = this.identities_for(&account);
                        if let Some(i) = identity::for_reply(&all, &orig.to, &orig.cc).filter(|i| !i.is_main()) {
                            d.from = Some((i.name.clone(), i.email.clone()));
                        }
                    }
                    let draft = this.with_defaults(d);
                    this.open_composer(draft, window, cx);
                }
                Err(err) => {
                    this.status = format!("Could not open the message: {err:#}").into();
                    this.status_is_error = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Enter on a message in Drafts reopens it in the composer.
    fn open_draft(&mut self, _: &OpenDraft, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() || self.active().map(|m| m.name.as_ref()) != Some(mail::DRAFTS) {
            return;
        }
        let Some(e) = self.envelopes.get(self.selected) else { return };
        let Some(session) = self.session.clone() else { return };
        let (mailbox, id) = (e.mailbox.clone(), e.id.clone());
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let (mb, i) = (mailbox.clone(), id.clone());
            let result = cx
                .background_executor()
                .spawn(async move { session.lock().unwrap().original(&mb, &i) })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(orig) => {
                    let d = this.with_defaults(compose::from_server_draft(&orig, &mailbox, &id));
                    this.open_composer(d, window, cx)
                }
                Err(err) => {
                    this.status = format!("Could not open the draft: {err:#}").into();
                    this.status_is_error = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Esc: close and keep the draft in Gmail's Drafts folder.
    fn close_compose(&mut self, _: &CloseCompose, window: &mut Window, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        if let Some(c) = &mut self.composer {
            if !c.suggestions.is_empty() {
                c.clear_suggestions();
                cx.notify();
                return;
            }
        }
        let Some(draft) = self.close_composer_ui(window, cx) else { return };
        if draft.is_blank() {
            self.clear_snapshot(cx);
            return;
        }
        // A draft belongs to the account it was written in, even if
        // another one is on screen now (rare: that connects on the spot).
        let acct: Arc<str> = if draft.account.is_empty() || *draft.account == *acct { acct } else { draft.account.as_str().into() };
        let session = if acct == self.account {
            self.session.clone()
        } else {
            Session::connect(&acct).ok().map(|s| Arc::new(Mutex::new(s)))
        };
        let Some(session) = session else {
            self.status = "Offline: the draft stays on this Mac until you reconnect".into();
            return;
        };
        let draft = self.with_sender(draft, &acct);
        let store = self.store.clone();
        self.status = "Saving draft…".into();
        self.status_is_error = false;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let (raw, _) = compose::build_message(&draft, &*acct, false, None)?;
                    let mut session = session.lock().unwrap();
                    session.save_draft(raw)?;
                    if let Some((mb, id)) = &draft.server_draft {
                        session.delete_message(mb, id).ok();
                    }
                    if let Some(store) = store {
                        store.lock().unwrap().save_snapshot(None)?;
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => this.status = "Draft saved to Gmail".into(),
                    Err(err) => {
                        this.status = format!("Draft kept on this Mac; saving to Gmail failed: {err:#}").into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn discard_compose(&mut self, _: &DiscardCompose, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.close_composer_ui(window, cx) else { return };
        self.clear_snapshot(cx);
        self.status = "Draft discarded".into();
        self.status_is_error = false;
        if let (Some((mb, id)), Some(session)) = (draft.server_draft, self.session.clone()) {
            cx.background_executor()
                .spawn(async move { session.lock().unwrap().delete_message(&mb, &id).ok() })
                .detach();
        }
    }

    /// ⌘⏎: check, then hold the message for the undo window before sending.
    fn send_message(&mut self, _: &SendMessage, window: &mut Window, cx: &mut Context<Self>) {
        let Some(c) = &mut self.composer else { return };
        let draft = c.draft(cx);
        if let Err(err) = compose::check_sendable(&draft) {
            c.error = Some(format!("{err:#}").into());
            cx.notify();
            return;
        }
        if self.pending_send.is_some() {
            c.error = Some("Another message is still in its undo window; try again in a moment".into());
            cx.notify();
            return;
        }
        self.close_composer_ui(window, cx);
        self.save_snapshot_for(&draft, cx);
        let task = cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            for _ in 0..UNDO_SEND_SECS {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
            this.update_in(cx, |this, window, cx| this.flush_send(window, cx)).ok();
        });
        self.review_draft = Some(draft.clone());
        self.pending_send = Some(PendingSend {
            draft,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(UNDO_SEND_SECS),
            _task: task,
        });
        self.countdown_focus.focus(window, cx);
        cx.notify();
    }

    /// ⏎ on the countdown: skip the rest of the wait.
    fn send_now(&mut self, _: &SendNow, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending_send.is_some() {
            self.flush_send(window, cx);
        }
    }

    /// While a message waits out its countdown (and briefly after), the
    /// reader pane shows it read-only for a last review, with the countdown
    /// as a banner on top. Nothing is covered.
    fn render_send_review(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use std::time::{Duration, Instant};
        let after = if self.pending_send.is_some() { None } else { self.after_send };
        let go = after.is_some();
        let sent = after == Some(AfterSend::Sent);
        let left = match &self.pending_send {
            Some(p) => (p.deadline.saturating_duration_since(Instant::now()).as_millis() as u64 + 999) / 1000,
            None => 0,
        }
        .clamp(0, UNDO_SEND_SECS);
        // White, then red 3, orange 2, yellow 1; green once sent.
        let color = match (after, left) {
            (Some(AfterSend::Sent), _) => 0x34d058,
            (Some(AfterSend::Sending), _) => 0xffffff,
            (None, 3) => 0xff3b30,
            (None, 2) => 0xff9500,
            (None, 1) => 0xffd60a,
            _ => 0xffffff,
        };
        let lamps = div().flex().flex_none().gap_1p5().children((0..UNDO_SEND_SECS).map(move |i| {
            let lit = go || i < left;
            let (fill, ring) = match (sent, go, lit) {
                (true, _, _) => (0x34d058, 0x7ee2a0),
                (false, true, _) => (0x2a1212, 0x3d1d1d),
                (false, false, true) => (0xff2d2d, 0xff8a8a),
                (false, false, false) => (0x2a1212, 0x3d1d1d),
            };
            div().size(px(14.)).rounded_full().bg(rgb(fill)).border_1().border_color(rgb(ring))
        }));
        let (label, caption): (SharedString, &str) = match after {
            Some(AfterSend::Sent) => ("Sent".into(), ""),
            Some(AfterSend::Sending) => ("Sending…".into(), ""),
            None => (left.to_string().into(), "Sending in"),
        };
        let number = div()
            .w(px(if go { 150. } else { 64. }))
            .h(px(64.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .child(
                div()
                    .font_family("Menlo")
                    .font_weight(FontWeight::BLACK)
                    .italic()
                    .text_color(rgb(color))
                    .child(label)
                    .with_animation(
                        ("countdown", match after { Some(AfterSend::Sent) => 98, Some(AfterSend::Sending) => 99, None => left as usize }),
                        Animation::new(Duration::from_millis(850)).with_easing(ease_out_quint()),
                        move |el, d| {
                            // One pulse per second: punch in, settle, fade a little.
                            let size = match after {
                                Some(AfterSend::Sent) => 26. + 8. * d,
                                Some(AfterSend::Sending) => 24.,
                                None => 60. - 18. * d,
                            };
                            let fade = if after.is_some() { 0.0 } else { 0.3 * d };
                            el.text_size(px(size)).opacity(1.0 - fade)
                        },
                    ),
            );
        let banner = div()
            .track_focus(&self.countdown_focus)
            .key_context("Countdown")
            .flex_none()
            .mx_6()
            .mt_4()
            .px_4()
            .py_2()
            .rounded_lg()
            .bg(rgb(0x121216))
            .border_1()
            .border_color(rgb(if sent { 0x34d058 } else { 0x3d3d47 }))
            .flex()
            .items_center()
            .gap_4()
            .child(lamps)
            .when(!caption.is_empty(), |d| {
                d.child(div().flex_none().text_xs().font_family("Menlo").text_color(rgb(MUTED)).child(caption.to_uppercase()))
            })
            .child(number)
            .child(div().flex_1())
            .when(!go, |d| {
                d.child(
                    action_button("countdown-cancel", "Cancel  esc", false)
                        .on_click(cx.listener(|this, _, w, cx| this.undo_send(&UndoSend, w, cx))),
                )
                .child(
                    action_button("countdown-send", "Send now  ⏎", true)
                        .on_click(cx.listener(|this, _, w, cx| this.send_now(&SendNow, w, cx))),
                )
            });
        let draft = self.review_draft.clone().unwrap_or_else(Draft::new);
        let field = |label: &'static str, value: String| {
            div()
                .flex()
                .gap_3()
                .text_sm()
                .child(div().w(px(56.)).flex_none().text_xs().text_color(rgb(MUTED)).pt(px(2.)).child(label))
                .child(div().flex_1().min_w_0().text_color(rgb(TEXT)).child(value))
        };
        let subject = if draft.subject.trim().is_empty() { "(no subject)".to_string() } else { draft.subject.clone() };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(banner)
            .child(
                div()
                    .flex_none()
                    .px_8()
                    .pt_5()
                    .pb_3()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(field("To", draft.to.clone()))
                    .when(!draft.cc.trim().is_empty(), |d| d.child(field("Cc", draft.cc.clone())))
                    .when(!draft.bcc.trim().is_empty(), |d| d.child(field("Bcc", draft.bcc.clone())))
                    .child(field("Subject", subject))
                    .when(!draft.attachments.is_empty(), |d| {
                        let names: Vec<String> = draft
                            .attachments
                            .iter()
                            .map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
                            .collect();
                        d.child(field("Files", names.join(", ")))
                    }),
            )
            // The message as it will arrive (rendered Markdown).
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .mx_6()
                    .mb_5()
                    .rounded_md()
                    .overflow_hidden()
                    .child(self.html_view.placeholder()),
            )
    }

    fn save_snapshot_for(&self, draft: &Draft, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let draft = draft.clone();
        cx.background_executor()
            .spawn(async move { store.lock().unwrap().save_snapshot(Some(&draft)).ok() })
            .detach();
    }

    /// The From line: the chosen address, or the account with its sender name.
    fn with_sender(&self, mut d: Draft, account: &str) -> Draft {
        if d.from.is_none() {
            let name = self.accounts.iter().find(|a| a.email == account).map(|a| a.name.clone()).unwrap_or_default();
            d.from = Some((name, account.to_string()));
        }
        d
    }

    /// The undo window is over: send for real.
    fn flush_send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        let Some(pending) = self.pending_send.take() else { return };
        let draft = pending.draft;
        // Sent through the account it was written in.
        let acct: Arc<str> = if draft.account.is_empty() { acct } else { draft.account.as_str().into() };
        let session = if acct == self.account { self.session.clone() } else { None };
        let draft = self.with_sender(draft, &acct);
        self.after_send = Some(AfterSend::Sending);
        self.focus.focus(window, cx);
        let store = self.store.clone();
        let sig = self.signature_by_id(draft.signature).cloned();
        self.status = "Sending…".into();
        self.status_is_error = false;
        cx.notify();
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let d = draft.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let (raw, _) = compose::build_message(&d, &*acct, true, sig.as_ref())?;
                    mail::send_raw(&*acct, raw, &compose::envelope_recipients(&d)?)?;
                    // Sent: the Gmail draft it came from and the autosave go.
                    if let (Some((mb, id)), Some(session)) = (&d.server_draft, session) {
                        session.lock().unwrap().delete_message(mb, id).ok();
                    }
                    if let Some(store) = store {
                        store.lock().unwrap().save_snapshot(None).ok();
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => {
                    let subject = if draft.subject.trim().is_empty() { "(no subject)" } else { draft.subject.trim() };
                    this.status = format!("Sent: {subject}").into();
                    this.status_is_error = false;
                    this.after_send = Some(AfterSend::Sent);
                    cx.notify();
                    cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                        cx.background_executor().timer(std::time::Duration::from_millis(1100)).await;
                        this.update(cx, |this, cx| {
                            this.after_send = None;
                            this.review_draft = None;
                            cx.notify();
                        })
                        .ok();
                    })
                    .detach();
                }
                Err(err) => {
                    this.after_send = None;
                    this.review_draft = None;
                    this.open_composer(draft, window, cx);
                    if let Some(c) = &mut this.composer {
                        c.error = Some(format!("Not sent: {err:#}").into());
                    }
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn undo_send(&mut self, _: &UndoSend, window: &mut Window, cx: &mut Context<Self>) {
        // Dropping the pending task cancels the send.
        let Some(pending) = self.pending_send.take() else {
            // Otherwise undo the newest Sweep decision or archive/delete.
            let sweep_newer = match (self.pending_sweep.last(), self.pending_bulk.last()) {
                (Some(s), Some(b)) => s.deadline >= b.deadline,
                (Some(_), None) => true,
                _ => false,
            };
            if sweep_newer && self.undo_sweep(cx) {
                return;
            }
            if let Some(bulk) = self.pending_bulk.pop() {
                let n = bulk.items.len();
                let what = bulk.kind.done();
                self.restore_items(bulk.items, cx);
                self.status = format!("Undone: {what} {n} message{}", if n == 1 { "" } else { "s" }).into();
                self.status_is_error = false;
            }
            return;
        };
        self.review_draft = None;
        self.open_composer(pending.draft, window, cx);
        self.status = "Send cancelled; back to editing".into();
        self.status_is_error = false;
    }

    fn render_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(c) = &self.composer else { return div() };
        let field = |label: &'static str, input: AnyElement| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_3()
                .child(div().w(px(56.)).flex_none().text_xs().text_color(rgb(MUTED)).child(label))
                .child(div().flex_1().min_w_0().child(input))
        };
        let line = |e: &Entity<InputState>| {
            Input::new(e)
                .bg(rgb(SEARCH_BG))
                .border_color(rgb(SEARCH_BORDER))
                .text_color(rgb(0xffffff))
                .into_any_element()
        };
        div()
            .key_context("Composer")
            // Files dragged in from Finder are attached.
            .drag_over::<ExternalPaths>(|s, _, _, _| s.bg(rgb(0x1d2333)))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| this.add_attachments(paths.paths().to_vec(), cx)))
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_3()
            .px_8()
            .py_6()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_between()
                    .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(c.kind.title()))
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                action_button("attach", "Attach  ⌘⇧A", false)
                                    .on_click(cx.listener(|this, _, _, cx| this.pick_attachments(cx))),
                            )
                            .child(
                                action_button("discard", "Discard", false)
                                    .on_click(cx.listener(|this, _, w, cx| this.discard_compose(&DiscardCompose, w, cx))),
                            )
                            .child(
                                action_button("send", "Send  ⌘⏎", true)
                                    .on_click(cx.listener(|this, _, w, cx| this.send_message(&SendMessage, w, cx))),
                            ),
                    ),
            )
            .children(c.error.clone().map(|e| {
                div()
                    .flex_none()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(0x3b1f22))
                    .text_color(rgb(ERROR))
                    .text_sm()
                    .child(e)
            }))
            .children(self.render_from_row(cx))
            .child(field("To", line(&c.to)).key_context("Recipients"))
            .children(self.render_suggestions(RecipientField::To, cx))
            .child(
                field("Cc", line(&c.cc)).key_context("Recipients").when(!c.show_bcc, |d| {
                    d.child(
                        div()
                            .id("show-bcc")
                            .flex_none()
                            .text_xs()
                            .text_color(rgb(SUBTLE))
                            .cursor_pointer()
                            .hover(|d| d.text_color(rgb(TEXT)))
                            .tooltip(|window, cx| Tooltip::new("Add Bcc  ⌘⇧B").build(window, cx))
                            .on_click(cx.listener(|this, _, w, cx| this.show_bcc(w, cx)))
                            .child("Bcc"),
                    )
                }),
            )
            .children(self.render_suggestions(RecipientField::Cc, cx))
            .when(c.show_bcc, |d| {
                d.child(
                    field("Bcc", line(&c.bcc)).key_context("Recipients").child(
                        div()
                            .id("hide-bcc")
                            .flex_none()
                            .size(px(22.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_sm()
                            .text_color(rgb(SUBTLE))
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                            .tooltip(|window, cx| Tooltip::new("Remove Bcc  ⌘⇧B").build(window, cx))
                            .on_click(cx.listener(|this, _, w, cx| this.hide_bcc(w, cx)))
                            .child("×"),
                    ),
                )
            })
            .children(self.render_suggestions(RecipientField::Bcc, cx))
            .child(field("Subject", line(&c.subject)))
            .children(self.render_outgoing(cx))
            .child(self.render_format_bar(cx))
            .child(if c.preview {
                div()
                    .track_focus(&c.preview_focus)
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(SEARCH_BORDER))
                    .child(self.html_view.placeholder())
                    .into_any_element()
            } else {
                div()
                    .id("compose-body")
                    .key_context("ComposeBody")
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .drag_over::<blocks::BlockDrag>(|s, _, _, _| s.border_2().border_color(rgb(ACCENT)))
                    .on_drop(cx.listener(|this, d: &blocks::BlockDrag, w, cx| this.drop_block(d, w, cx)))
                    .child(
                    Textarea::new(&c.body)
                        .h_full()
                        .bg(rgb(SEARCH_BG))
                        .border_color(rgb(SEARCH_BORDER))
                        .text_color(rgb(TEXT))
                        .when(c.mono, |t| t.font_family("Menlo")),
                )
                .into_any_element()
            })
            .child(self.render_signature_strip(cx))
    }

    fn render_suggestions(&self, field: RecipientField, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let c = self.composer.as_ref()?;
        if c.suggest_for != Some(field) || c.suggestions.is_empty() {
            return None;
        }
        Some(
            div()
                .ml(px(68.))
                .flex_none()
                .flex()
                .flex_col()
                .py_1()
                .rounded_md()
                .bg(rgb(0x24242b))
                .border_1()
                .border_color(rgb(0x3d3d47))
                .children(c.suggestions.iter().enumerate().map(|(ix, (name, email))| {
                    let active = ix == c.suggest_ix;
                    div()
                        .id(("suggestion", ix))
                        .px_3()
                        .py_1()
                        .flex()
                        .items_baseline()
                        .gap_2()
                        .cursor_pointer()
                        .when(active, |d| d.bg(rgb(SELECTED)))
                        .hover(|d| d.bg(rgb(HOVER)))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if let Some(c) = &mut this.composer {
                                c.suggest_ix = ix;
                                c.accept(window, cx);
                                cx.notify();
                            }
                        }))
                        .when(!name.is_empty(), |d| {
                            d.child(div().text_sm().text_color(rgb(TEXT)).child(name.clone()))
                        })
                        .child(div().text_xs().text_color(rgb(MUTED)).child(email.clone()))
                })),
        )
    }

    /// Re-read the folder list and the Sweep labels' counts (after a Sweep
    /// decision creates or fills a label).
    fn refresh_folders(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.preview_session.clone().or_else(|| self.session.clone()) else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut s = session.lock().unwrap();
                    let names = s.mailboxes()?;
                    let mut counts = std::collections::HashMap::new();
                    // The big catch-alls are left out: their totals are noise.
                    const SKIP: [&str; 5] = ["[Gmail]/All Mail", "[Gmail]/Important", "[Gmail]/Starred", "[Gmail]/Sent Mail", "[Gmail]/Drafts"];
                    for name in names.iter().filter(|n| !SKIP.iter().any(|k| k.eq_ignore_ascii_case(n))) {
                        if let Ok(c) = s.folder_counts(name) {
                            counts.insert(name.clone(), c);
                        }
                    }
                    anyhow::Ok((names, counts))
                })
                .await;
            let Ok((names, counts)) = result else { return };
            this.update(cx, |this, cx| {
                let active = this.active().map(|m| m.name.clone());
                this.mailboxes = mailbox_items(names);
                if let Some(a) = active {
                    if let Some(ix) = this.mailboxes.iter().position(|m| m.name.eq_ignore_ascii_case(&a)) {
                        this.active_mailbox = ix;
                    }
                }
                this.label_counts = counts;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ----- Bulk selection, archive, delete --------------------------------

    fn toggle_mark(&mut self, _: &ToggleMark, _: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.envelopes.get(self.selected) else { return };
        let key = e.key();
        if !self.marked.remove(&key) {
            self.marked.insert(key);
        }
        // Move on, so x x x marks a run of messages.
        if self.selected + 1 < self.envelopes.len() {
            self.selected += 1;
            self.scroll.scroll_to_item(self.selected, ScrollStrategy::Center);
            self.load_body(self.selected, cx);
        }
        cx.notify();
    }

    fn mark_all(&mut self, _: &MarkAll, _: &mut Window, cx: &mut Context<Self>) {
        let all: HashSet<(String, String)> =
            self.envelopes.iter().map(|e| e.key()).collect();
        // Second cmd-a clears.
        self.marked = if self.marked == all { HashSet::new() } else { all };
        cx.notify();
    }

    fn targets(&self) -> Vec<Envelope> {
        if self.marked.is_empty() {
            self.envelopes.get(self.selected).cloned().into_iter().collect()
        } else {
            self.envelopes
                .iter()
                .filter(|e| self.marked.contains(&e.key()))
                .cloned()
                .collect()
        }
    }

    fn bulk(&mut self, kind: BulkKind, cx: &mut Context<Self>) {
        let items = self.targets();
        if items.is_empty() || items.iter().any(|e| e.mailbox == "demo") {
            return;
        }
        // Deleting what's already in Trash or Spam deletes it forever (still
        // held for the undo window first).
        let kind = if kind == BulkKind::Delete && items.iter().all(|e| mail::is_trash_or_spam(&e.mailbox)) {
            BulkKind::Purge
        } else {
            kind
        };
        let cannot = |e: &Envelope| match kind {
            // Archive = leave the Inbox or a label; meaningless elsewhere.
            BulkKind::Archive => [ALL_MAIL, mail::TRASH, mail::DRAFTS, "[Gmail]/Spam", "[Gmail]/Sent Mail"]
                .iter()
                .any(|m| e.mailbox.eq_ignore_ascii_case(m)),
            BulkKind::Delete => e.mailbox.eq_ignore_ascii_case(mail::TRASH),
            BulkKind::ToInbox => e.mailbox.eq_ignore_ascii_case(mail::INBOX),
            BulkKind::Purge => !mail::is_trash_or_spam(&e.mailbox),
        };
        if items.iter().any(cannot) {
            self.status = match kind {
                BulkKind::Archive => "Archive works in the Inbox and labels (use delete here)".into(),
                BulkKind::Delete => "Already in Trash; Gmail empties it after 30 days".into(),
                BulkKind::ToInbox => "Already in the Inbox".into(),
                BulkKind::Purge => "Delete forever works in Trash and Spam".into(),
            };
            self.status_is_error = true;
            cx.notify();
            return;
        }
        let keys: HashSet<(String, String)> = items.iter().map(|e| e.key()).collect();
        let selected_key = self.envelopes.get(self.selected).map(|e| e.key());
        let first_removed = self
            .envelopes
            .iter()
            .position(|e| keys.contains(&e.key()))
            .unwrap_or(self.selected);
        self.envelopes.retain(|e| !keys.contains(&e.key()));
        self.marked.clear();
        // Keep the cursor near where the removed rows were.
        self.selected = match selected_key.filter(|k| !keys.contains(k)) {
            Some(k) => self.envelopes.iter().position(|e| e.key() == k).unwrap_or(0),
            None => first_removed.min(self.envelopes.len().saturating_sub(1)),
        };
        self.load_body(self.selected, cx);
        self.status_is_error = false;

        let id = self.next_bulk_id;
        self.next_bulk_id += 1;
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            for _ in 0..UNDO_SEND_SECS {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
            this.update(cx, |this, cx| this.commit_bulk(id, cx)).ok();
        });
        self.pending_bulk.push(PendingBulk {
            id,
            kind,
            items,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(UNDO_SEND_SECS),
            _task: task,
        });
        cx.notify();
    }

    /// The undo window is over: apply it in Gmail, then in the cache.
    fn commit_bulk(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(pos) = self.pending_bulk.iter().position(|p| p.id == id) else { return };
        let pending = self.pending_bulk.remove(pos);
        let kind = pending.kind;
        let items = pending.items;
        let mut sessions: std::collections::HashMap<String, SharedSession> = Default::default();
        for e in &items {
            let a = self.account_of(e).to_string();
            if let Some(s) = self.session_of(&a) {
                sessions.insert(a, s);
            }
        }
        if items.iter().any(|e| !sessions.contains_key(&*self.account_of(e))) {
            self.restore_items(items, cx);
            self.status = "Offline: nothing was changed".into();
            self.status_is_error = true;
            return;
        }
        let store = self.store.clone();
        let accounts: Vec<String> = items.iter().map(|e| self.account_of(e).to_string()).collect();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let work = items.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let dest = kind.dest();
                    let mut by_box: std::collections::BTreeMap<(String, String), Vec<String>> = Default::default();
                    for (e, a) in work.iter().zip(&accounts) {
                        by_box.entry((a.clone(), e.mailbox.clone())).or_default().push(e.id.clone());
                    }
                    for ((account, mailbox), ids) in &by_box {
                        let mut s = sessions[account].lock().unwrap();
                        if kind == BulkKind::Purge {
                            s.purge(mailbox, ids)?;
                        } else {
                            s.move_ids(mailbox, dest, ids)?;
                        }
                    }
                    if let Some(store) = store {
                        let mut per: std::collections::BTreeMap<String, Vec<(String, String, Option<String>)>> = Default::default();
                        for (e, a) in work.iter().zip(&accounts) {
                            per.entry(a.clone()).or_default().push((e.mailbox.clone(), e.id.clone(), e.message_id.clone()));
                        }
                        let store = store.lock().unwrap();
                        for (account, keys) in &per {
                            store.remove_envelopes(account, keys, kind == BulkKind::Delete)?;
                        }
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| {
                if let Err(err) = result {
                    this.restore_items(items, cx);
                    this.status = format!("Gmail refused the change; restored: {err:#}").into();
                    this.status_is_error = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Put rows back into the list they came from (sorted newest first).
    fn restore_items(&mut self, items: Vec<Envelope>, cx: &mut Context<Self>) {
        let visible = |e: &Envelope| {
            !self.query.is_empty()
                || (self.unified && e.mailbox.eq_ignore_ascii_case(mail::INBOX))
                || (!self.unified && self.active().map(|m| m.name.as_ref()) == Some(e.mailbox.as_str()))
        };
        let back: Vec<Envelope> = items.into_iter().filter(|e| visible(e)).collect();
        let current = self.envelopes.get(self.selected).map(|e| e.key());
        self.envelopes.extend(back);
        self.envelopes.sort_by(|a, b| b.ts.cmp(&a.ts));
        if let Some(k) = current {
            self.selected = self.envelopes.iter().position(|e| e.key() == k).unwrap_or(0);
        }
        cx.notify();
    }

    fn focus_list(&mut self, _: &FocusList, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        cx.notify();
    }

    fn toggle_hints(&mut self, _: &ToggleHints, _: &mut Window, cx: &mut Context<Self>) {
        self.show_hints = !self.show_hints;
        cx.notify();
    }

    fn go_to(&mut self, server_name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_sweep(window, cx);
        let Some(ix) = self.mailboxes.iter().position(|m| m.name.eq_ignore_ascii_case(server_name)) else {
            return;
        };
        if !self.query.is_empty() {
            self.search.update(cx, |s, cx| s.set_value("", window, cx));
        }
        self.open_mailbox(ix, cx);
    }

    fn focus_search(&mut self, _: &FocusSearch, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Fetch bodies (and so previews) for a range of rows, one at a time,
    /// on the preview connection.
    fn prefetch(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let end = range.end.min(self.envelopes.len());
        let start = range.start.min(end);
        let pkey = |e: &Envelope| format!("{}\u{1f}{}", e.key().0, e.id);
        let jobs: Vec<(String, String, String)> = self.envelopes[start..end]
            .iter()
            .filter(|e| e.body.is_none() && !self.prefetching.contains(&pkey(e)))
            .filter(|e| self.preview_of(&e.account).is_some())
            .map(|e| (e.account.to_string(), e.mailbox.clone(), e.id.clone()))
            .collect();
        if jobs.is_empty() {
            return;
        }
        self.prefetching.extend(self.envelopes[start..end].iter().map(pkey));
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            for (account, mailbox, id) in jobs {
                let session = this
                    .update(cx, |this, _| {
                        let wanted = this.envelopes.iter().any(|e| e.is(&account, &mailbox, &id) && e.body.is_none());
                        if wanted { this.preview_of(&account) } else { None }
                    })
                    .ok()
                    .flatten();
                let Some(session) = session else { continue };
                let (mb, fetch_id) = (mailbox.clone(), id.clone());
                let result = cx
                    .background_executor()
                    .spawn(async move { session.lock().unwrap().body(&mb, &fetch_id) })
                    .await;
                let Ok((text, html, files)) = result else { continue };
                if this.update(cx, |this, cx| this.apply_body(&account, &mailbox, &id, text, html, files, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// The connection to act on an account's messages with: the main one
    /// for the account on screen, else its pooled one.
    fn session_of(&self, account: &str) -> Option<SharedSession> {
        if account.is_empty() || *account == *self.account {
            self.session.clone()
        } else {
            self.pool.get(account).cloned()
        }
    }

    /// For background fetches: the account on screen's preview connection.
    fn preview_of(&self, account: &str) -> Option<SharedSession> {
        if account.is_empty() || *account == *self.account {
            self.preview_session.clone().or_else(|| self.session.clone())
        } else {
            self.pool.get(account).cloned()
        }
    }

    /// The account a message belongs to (older rows: the one on screen).
    fn account_of(&self, e: &Envelope) -> Arc<str> {
        if e.account.is_empty() { self.account.clone() } else { e.account.as_ref().into() }
    }

    /// Connect the accounts that aren't on screen (for All Inboxes).
    fn fill_pool(&mut self, cx: &mut Context<Self>) {
        let wanted: Vec<String> = self
            .accounts
            .iter()
            .map(|a| a.email.clone())
            .filter(|e| *e != *self.account && !self.pool.contains_key(e) && !self.pool_connecting.contains(e))
            .collect();
        for account in wanted {
            self.pool_connecting.insert(account.clone());
            cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
                let a = account.clone();
                let result = cx.background_executor().spawn(async move { Session::connect(&a) }).await;
                this.update(cx, |this, cx| {
                    this.pool_connecting.remove(&account);
                    match result {
                        Ok(s) if *this.account != *account => {
                            this.pool.insert(account.clone(), Arc::new(Mutex::new(s)));
                            if this.unified {
                                this.refresh_unified_account(&account, cx);
                            }
                        }
                        Ok(_) => {}
                        Err(err) => {
                            this.status = format!("{account}: {err:#}").into();
                            this.status_is_error = true;
                            cx.notify();
                        }
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    fn apply_body(
        &mut self,
        account: &str,
        mailbox: &str,
        id: &str,
        text: String,
        html: Option<String>,
        files: Vec<model::Attachment>,
        cx: &mut Context<Self>,
    ) {
        let acct = account.to_string();
        let preview = make_preview(&text);
        if let Some(store) = self.store.clone() {
            let (mb, id, body, h, pv, f) =
                (mailbox.to_string(), id.to_string(), text.clone(), html.clone(), preview.clone(), files.clone());
            cx.background_executor()
                .spawn(async move {
                    store.lock().unwrap().save_body(&*acct, &mb, &id, &body, h.as_deref(), &pv, &f).ok();
                })
                .detach();
        }
        if let Some(e) = self.envelopes.iter_mut().find(|e| e.is(account, mailbox, id)) {
            if e.body.is_some() {
                return;
            }
            e.preview = preview.into();
            e.body = Some(text.into());
            e.html = html.map(Into::into);
            e.attachments = files;
            cx.notify();
        }
    }

    fn open_mailbox(&mut self, ix: usize, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        let Some(item) = self.mailboxes.get(ix).cloned() else { return };
        if !item.exists {
            self.active_mailbox = ix;
            self.query.clear();
            self.envelopes.clear();
            self.loading = false;
            self.has_more = false;
            self.status = format!("Nothing in {} yet: file senders here with the Sweep (g w)", item.label).into();
            self.status_is_error = false;
            cx.notify();
            return;
        }
        self.empty_armed = false;
        let was_unified = std::mem::replace(&mut self.unified, false);
        let switching = was_unified || ix != self.active_mailbox || !self.query.is_empty() || self.envelopes.is_empty();
        self.active_mailbox = ix;
        if !self.query.is_empty() {
            self.query.clear();
        }
        // Cached copy first: instant.
        if switching {
            let cached = self
                .store
                .as_ref()
                .and_then(|s| s.lock().unwrap().envelopes(&*acct, &item.name, PAGE_SIZE).ok())
                .unwrap_or_default();
            self.selected = 0;
            self.envelopes.clear();
            if cached.is_empty() {
                self.loading = true;
                cx.notify();
            } else {
                self.set_envelopes(cached, cx);
            }
        }
        // Then refresh from the server.
        let Some(session) = self.session.clone() else { return };
        let store = self.store.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mailbox = item.name.to_string();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let fresh = session.lock().unwrap().envelopes(&mailbox, PAGE_SIZE)?;
                    match store {
                        Some(store) => {
                            let mut store = store.lock().unwrap();
                            store.save_envelopes(&*acct, &mailbox, &fresh)?;
                            store.envelopes(&*acct, &mailbox, PAGE_SIZE)
                        }
                        None => Ok(fresh),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if this.active().map(|m| &m.name) != Some(&item.name) || !this.query.is_empty() {
                    return; // user moved on
                }
                match result {
                    Ok(list) => {
                        let list = this.apply_rules_to_inbox(list, cx);
                        this.set_envelopes(list, cx)
                    }
                    Err(err) => {
                        this.loading = false;
                        this.status = format!("{err:#}").into();
                        this.status_is_error = true;
                        cx.notify();
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    fn load_body(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(envelope) = self.envelopes.get(ix) else { return };
        if envelope.body.is_some() {
            return;
        }
        let account = self.account_of(envelope).to_string();
        let Some(session) = self.session_of(&account) else { return };
        let (mailbox, id) = (envelope.mailbox.clone(), envelope.id.clone());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (mb, fetch_id) = (mailbox.clone(), id.clone());
            let result = cx
                .background_executor()
                .spawn(async move { session.lock().unwrap().body(&mb, &fetch_id) })
                .await;
            this.update(cx, |this, cx| {
                let (text, html, files) =
                    result.unwrap_or_else(|err| (format!("Could not load message: {err:#}"), None, Vec::new()));
                this.apply_body(&account, &mailbox, &id, text, html, files, cx);
            })
            .ok();
        })
        .detach();
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix >= self.envelopes.len() {
            return;
        }
        self.selected = ix;
        if self.envelopes[ix].unread {
            self.set_unread(ix, false, cx);
        }
        self.scroll.scroll_to_item(ix, ScrollStrategy::Center);
        self.load_body(ix, cx);
        self.prefetch(ix + 1..ix + 1 + PREFETCH_AHEAD / 2, cx);
        cx.notify();
    }

    /// Append the next window of older messages: from the cache when it has
    /// them (works offline), otherwise from Gmail.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        if self.unified {
            return; // All Inboxes shows each account's newest page
        }
        if self.loading_more || !self.has_more || !self.query.is_empty() || self.loading {
            return;
        }
        let Some(item) = self.active().cloned() else { return };
        let oldest_ts = self.envelopes.last().map(|e| e.ts).unwrap_or(i64::MAX);
        let page = self.envelopes.len() as u32 / PAGE_SIZE + 1;
        let store = self.store.clone();
        let session = self.session.clone();
        self.loading_more = true;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mailbox = item.name.to_string();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let cached = |store: &Option<SharedStore>| {
                        store
                            .as_ref()
                            .map(|s| s.lock().unwrap().envelopes_before(&*acct, &mailbox, oldest_ts, PAGE_SIZE))
                            .transpose()
                            .map(Option::unwrap_or_default)
                    };
                    let older = cached(&store)?;
                    if older.len() >= PAGE_SIZE as usize {
                        return anyhow::Ok((older, true));
                    }
                    let Some(session) = session else { return anyhow::Ok((older, false)) };
                    let fresh = session.lock().unwrap().envelopes_page(&mailbox, page, PAGE_SIZE)?;
                    let more = fresh.len() >= PAGE_SIZE as usize;
                    match &store {
                        Some(s) => {
                            s.lock().unwrap().upsert_envelopes(&*acct, &fresh)?;
                            Ok((cached(&store)?, more))
                        }
                        None => Ok((fresh.into_iter().filter(|e| e.ts < oldest_ts).collect(), more)),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                this.loading_more = false;
                if this.active().map(|m| &m.name) != Some(&item.name) || !this.query.is_empty() {
                    return;
                }
                match result {
                    Ok((older, more)) => {
                        let have: HashSet<(String, String)> =
                            this.envelopes.iter().map(|e| e.key()).collect();
                        let added: Vec<Envelope> = older
                            .into_iter()
                            .filter(|e| !have.contains(&e.key()))
                            .collect();
                        this.has_more = more && !added.is_empty();
                        this.envelopes.extend(added);
                    }
                    Err(err) => {
                        this.has_more = false;
                        this.status = format!("Could not load older mail: {err:#}").into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Background header sync of All Mail on its own connection: new mail
    /// above the high-water mark first, then older UID windows down to 1.
    /// Resumable: progress lives in the cache's sync_state table.
    fn start_sync(&mut self, cx: &mut Context<Self>) {
        let acct = self.account.clone();
        let Some(store) = self.store.clone() else { return };
        if !self.mailboxes.iter().any(|m| m.name.as_ref() == ALL_MAIL) {
            return;
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(std::time::Duration::from_secs(3)).await;
            // False once the window is gone or another account is on screen:
            // the sync stops (it picks up where it left off next time).
            let on_screen = acct.clone();
            let note = move |this: &WeakEntity<Self>, cx: &mut AsyncApp, text: String| {
                this.update(cx, |this, cx| {
                    if this.account != on_screen {
                        return false;
                    }
                    if (text.contains("unavailable") || text.contains("paused")) && errlog::record(&on_screen, &text) {
                        this.problems_unseen += 1;
                    }
                    this.sync_note = Some(text.into());
                    cx.notify();
                    true
                })
                .unwrap_or(false)
            };
            let st = store.clone();
            let a = acct.clone();
            let setup = cx
                .background_executor()
                .spawn(async move {
                    let acct = a;
                    let mut session = Session::connect(&*acct)?;
                    let status = session.mailbox_status(ALL_MAIL)?;
                    let store = st.lock().unwrap();
                    let (low, high) = match store.sync_state(&*acct, ALL_MAIL) {
                        Some((validity, low, high)) if validity == status.uid_validity => (low, high),
                        Some(_) => {
                            store.reset_mailbox(&*acct, ALL_MAIL)?;
                            (status.uid_next, status.uid_next - 1)
                        }
                        None => (status.uid_next, status.uid_next - 1),
                    };
                    store.set_sync_state(&*acct, ALL_MAIL, status.uid_validity, low, high)?;
                    anyhow::Ok((Arc::new(Mutex::new(session)), status, low, high))
                })
                .await;
            let (session, status, mut low, mut high) = match setup {
                Ok(v) => v,
                Err(err) => {
                    note(&this, cx, format!("Background sync unavailable: {err:#}"));
                    return;
                }
            };

            // One batch: fetch headers in lo..=hi, store them, save progress.
            let batch = |lo: u64, hi: u64, new_low: u64, new_high: u64| {
                let (session, store, acct) = (session.clone(), store.clone(), acct.clone());
                async move {
                    let list = session.lock().unwrap().headers_in_range(ALL_MAIL, lo, hi)?;
                    let store = store.lock().unwrap();
                    store_upsert(&store, &acct, &list)?;
                    store.set_sync_state(&*acct, ALL_MAIL, status.uid_validity, new_low, new_high)?;
                    anyhow::Ok(store.count(&*acct, ALL_MAIL)?)
                }
            };

            // New mail since the last run.
            while high + 1 < status.uid_next {
                let hi = (high + SYNC_WINDOW).min(status.uid_next - 1);
                match cx.background_executor().spawn(batch(high + 1, hi, low, hi)).await {
                    Ok(_) => high = hi,
                    Err(err) => {
                        note(&this, cx, format!("Sync paused: {err:#}"));
                        return;
                    }
                }
            }
            // People you write to: To/Cc of Sent Mail, for address suggestions.
            {
                let (session, store, acct) = (session.clone(), store.clone(), acct.clone());
                let synced = cx
                    .background_executor()
                    .spawn(async move {
                        let mut session = session.lock().unwrap();
                        let st = session.mailbox_status(SENT)?;
                        let key = "contacts:[Gmail]/Sent Mail";
                        let store = store.lock().unwrap();
                        let mut done = match store.sync_state(&*acct, key) {
                            Some((v, _, high)) if v == st.uid_validity => high,
                            _ => 0,
                        };
                        while done + 1 < st.uid_next {
                            let hi = (done + SYNC_WINDOW).min(st.uid_next - 1);
                            let people = session.recipients_in_range(SENT, done + 1, hi)?;
                            store.add_sent_contacts(&*acct, &people)?;
                            store.set_sync_state(&*acct, key, st.uid_validity, 1, hi)?;
                            done = hi;
                        }
                        anyhow::Ok(())
                    })
                    .await;
                if let Err(err) = synced {
                    note(&this, cx, format!("Contacts sync paused: {err:#}"));
                }
            }
            // Older mail, newest window first.
            while low > 1 {
                let lo = low.saturating_sub(SYNC_WINDOW).max(1);
                match cx.background_executor().spawn(batch(lo, low - 1, lo, high)).await {
                    Ok(count) => {
                        low = lo;
                        let pct = (count * 100 / status.exists.max(1)).min(99);
                        if !note(&this, cx, format!("Syncing older mail: {count} of {} ({pct}%)", status.exists)) {
                            return;
                        }
                    }
                    Err(err) => {
                        note(&this, cx, format!("Sync paused: {err:#}"));
                        return;
                    }
                }
                cx.background_executor().timer(std::time::Duration::from_millis(150)).await;
            }
            let total = store.lock().unwrap().count(&*acct, ALL_MAIL).unwrap_or(0);
            note(&this, cx, format!("All Mail synced: {total} messages"));
        })
        .detach();
    }

    /// Change read state locally (every copy in the list), in the cache and
    /// on the server; roll back if the server refuses.
    fn set_unread(&mut self, ix: usize, unread: bool, cx: &mut Context<Self>) {
        let Some(e) = self.envelopes.get(ix) else { return };
        let acct = self.account_of(e);
        let (mailbox, id, message_id) = (e.mailbox.clone(), e.id.clone(), e.message_id.clone());
        self.apply_unread(&acct, &mailbox, &id, message_id.as_deref(), unread);
        cx.notify();
        if mailbox == "demo" {
            return;
        }
        let Some(session) = self.session_of(&acct) else { return };
        let store = self.store.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (mb, i, mid, a) = (mailbox.clone(), id.clone(), message_id.clone(), acct.clone());
            let result = cx
                .background_executor()
                .spawn(async move {
                    session.lock().unwrap().set_seen(&mb, &i, !unread)?;
                    if let Some(store) = store {
                        store.lock().unwrap().set_unread(&a, &mb, &i, mid.as_deref(), unread)?;
                    }
                    anyhow::Ok(())
                })
                .await;
            if let Err(err) = result {
                this.update(cx, |this, cx| {
                    this.apply_unread(&acct, &mailbox, &id, message_id.as_deref(), !unread);
                    this.status = format!("Could not update read state: {err:#}").into();
                    this.status_is_error = true;
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    fn apply_unread(&mut self, account: &str, mailbox: &str, id: &str, message_id: Option<&str>, unread: bool) {
        let here = self.account.clone();
        let mut flipped: Vec<String> = Vec::new();
        for e in self.envelopes.iter_mut() {
            let mine = if e.account.is_empty() { *here == *account } else { e.account.as_ref() == account };
            let same = e.is(account, mailbox, id)
                || (mine && message_id.is_some() && e.message_id.as_deref() == message_id);
            if same {
                if e.unread != unread && *here == *account {
                    flipped.push(e.mailbox.clone());
                }
                e.unread = unread;
            }
        }
        // Keep the sidebar's unread counts in step until the next refresh.
        for mb in flipped {
            if let Some(c) = self.label_counts.get_mut::<str>(&mb) {
                c.1 = if unread { c.1 + 1 } else { c.1.saturating_sub(1) };
            }
        }
    }

    fn toggle_read(&mut self, _: &ToggleRead, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_read_targets(cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected + 1, cx);
    }

    fn select_prev(&mut self, _: &SelectPrev, _: &mut Window, cx: &mut Context<Self>) {
        self.select(self.selected.saturating_sub(1), cx);
    }

    fn render_mailbox_row(&self, ix: usize, item: &MailboxItem, unread: usize, quiet: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let active = ix == self.active_mailbox && self.query.is_empty() && self.sweep.is_none() && !self.unified;
        let (icon, tint) = folder_icon(&item.name, item.system);
        // Only the landing folders carry color at rest; quiet ones stay grey.
        let tint = if quiet && !active { None } else { tint };
        // Counts from the server: "unread / total", or just the total when
        // all is read. Folders without server counts (All Mail and the like)
        // show the open folder's loaded unread.
        let count = match self.label_counts.get(item.name.as_ref()) {
            Some(&(total, unseen)) => Some((unseen, Some(total))),
            None if active && unread > 0 => Some((unread as u64, None)),
            None => None,
        };
        let count = count.map(|(unseen, total)| {
            let mut row = div().flex().flex_none().items_center().gap(px(3.)).text_xs();
            if unseen > 0 {
                row = row.child(
                    div()
                        .text_color(rgb(if active { ACCENT } else { SUBTLE }))
                        .font_weight(FontWeight::MEDIUM)
                        .child(group_digits(unseen)),
                );
                if total.is_some() {
                    row = row.child(div().text_color(rgb(MUTED)).child("/"));
                }
            }
            if let Some(total) = total {
                row = row.child(div().text_color(rgb(MUTED)).child(group_digits(total)));
            }
            row
        });
        let text = if active { TEXT } else if quiet || !item.exists { MUTED } else { SUBTLE };
        div()
            .id(("mailbox", ix))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(30.))
            .px(px(10.))
            .rounded(px(6.))
            .text_color(rgb(text))
            .when(active, |d| d.bg(rgb(SELECTED)).font_weight(FontWeight::MEDIUM))
            .when(!active, |d| d.hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT))))
            .on_click(cx.listener(move |this, _, window, cx| {
                if !this.query.is_empty() {
                    this.search.update(cx, |s, cx| s.set_value("", window, cx));
                }
                this.close_sweep(window, cx);
                this.open_mailbox(ix, cx);
            }))
            .child(
                svg()
                    .path(icon)
                    .size(px(16.))
                    .flex_none()
                    .text_color(rgb(if active { tint.unwrap_or(ACCENT) } else { tint.unwrap_or(MUTED) }))
                    .when(!item.exists, |d| d.opacity(0.5)),
            )
            .child(div().flex_1().min_w_0().truncate().child(item.label.clone()))
            .children(count)
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let unread = self.envelopes.iter().filter(|e| e.unread).count();
        // Groups by meaning: where mail lands, your mail, out of the way.
        let group_of = |m: &MailboxItem| -> u8 {
            let n = m.name.as_ref();
            if !m.system {
                3
            } else if n.eq_ignore_ascii_case("INBOX") || n == sweep::READING || n == sweep::RECORDS {
                0
            } else if n == sweep::SCREENED || n == "[Gmail]/Spam" || n == "[Gmail]/Trash" {
                2
            } else {
                1
            }
        };
        let mut folders = div().flex().flex_col().gap(px(2.));
        let mut list = div()
            .id("mailbox-list")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(2.))
            .px(px(8.))
            .pt(px(12.))
            .pb(px(12.));
        for group in 0..=3u8 {
            let members: Vec<(usize, &MailboxItem)> =
                self.mailboxes.iter().enumerate().filter(|(_, m)| group_of(m) == group).collect();
            if members.is_empty() {
                continue;
            }
            if group > 0 {
                folders = folders.child(div().h(px(14.)).flex_none());
            }
            if group == 3 {
                folders = folders.child(
                    div()
                        .flex_none()
                        .px(px(10.))
                        .pb(px(4.))
                        .text_xs()
                        .text_color(rgb(MUTED))
                        .child("Labels"),
                );
            }
            for (ix, item) in members {
                folders = folders.child(self.render_mailbox_row(ix, item, unread, group == 2, cx));
            }
        }
        // The account on screen's folders scroll; the accounts above don't,
        // so a short window still shows every account.
        if let Some(a) = self.current_account() {
            list = list.child(
                div()
                    .flex_none()
                    .px(px(10.))
                    .pb(px(6.))
                    .text_xs()
                    .text_color(rgb(MUTED))
                    .truncate()
                    .child(format!("Folders · {}", a.short())),
            );
            list = list.child(folders);
        }
        let accounts_box = self.render_accounts_box(cx);
        let sweeping = self.sweep.is_some();
        // The Sweep is a tool, not a folder: pinned below the list.
        let sweep_row = div()
            .id("sweep-entry")
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .flex_1()
            .h(px(34.))
            .px(px(10.))
            .rounded(px(6.))
            .border_1()
            .border_color(rgb(if sweeping { ACCENT } else { BORDER }))
            .text_color(rgb(if sweeping { TEXT } else { SUBTLE }))
            .when(sweeping, |d| d.bg(rgb(SELECTED)))
            .when(!sweeping, |d| d.hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT))))
            .on_click(cx.listener(|this, _, w, cx| this.open_sweep(w, cx)))
            .child(svg().path("icons/wand-sparkles.svg").size(px(16.)).flex_none().text_color(rgb(ACCENT)))
            .child(div().flex_1().child("Sweep"))
            .child(div().flex_none().text_xs().font_family("Menlo").text_color(rgb(MUTED)).child("g w"));
        div()
            .w(px(SIDEBAR_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(SIDEBAR))
            .border_r_1()
            .border_color(rgb(BORDER))
            .text_sm()
            .child(accounts_box)
            .child(list)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_1p5()
                    .mx(px(8.))
                    .mb(px(10.))
                    .child(sweep_row)
                    .child(
                        div()
                            .id("settings-entry")
                            .size(px(34.))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(6.))
                            .border_1()
                            .border_color(rgb(if self.settings_view.is_some() { ACCENT } else { BORDER }))
                            .text_color(rgb(SUBTLE))
                            .hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT)))
                            .tooltip(|window, cx| Tooltip::new("Settings  ⌘,").build(window, cx))
                            .on_click(cx.listener(|this, _, w, cx| this.open_settings(settings::Pane::Writing, w, cx)))
                            .child(svg().path("icons/sliders-horizontal.svg").size(px(16.)).text_color(rgb(SUBTLE))),
                    ),
            )
    }

    /// Bottom bar: shortcut hints for what you're doing (left), status and
    /// sync progress (right).
    fn render_bar(&self, mode: Mode, cx: &mut Context<Self>) -> impl IntoElement {
        let hints: Vec<(SharedString, SharedString)> = if !self.show_hints {
            Vec::new()
        } else if mode == Mode::List {
            let mut h = self.list_hints();
            h.push(("⌘/".into(), "hide hints".into()));
            h
        } else {
            cx.global::<Shortcuts>()
                .hints
                .iter()
                .filter(|h| h.mode == mode || h.mode == Mode::Global)
                .map(|h| (h.keys.clone().into(), h.label.into()))
                .collect()
        };
        let status = match &self.pending_send {
            Some(p) => {
                let left = p.deadline.saturating_duration_since(std::time::Instant::now()).as_secs() + 1;
                div()
                    .truncate()
                    .text_color(rgb(ACCENT))
                    .child(format!("Sending in {}s · ⌘Z to undo", left.min(UNDO_SEND_SECS)))
            }
            None if self.sweep_status().is_some() => div()
                .truncate()
                .text_color(rgb(ACCENT))
                .child(self.sweep_status().unwrap_or_default()),
            None => match self.pending_bulk.last() {
                Some(b) => {
                    let left = b.deadline.saturating_duration_since(std::time::Instant::now()).as_secs() + 1;
                    let n = b.items.len();
                    let verb = b.kind.done();
                    div()
                        .truncate()
                        .text_color(rgb(ACCENT))
                        .child(format!(
                            "{verb} {n} message{} · ⌘Z to undo ({}s)",
                            if n == 1 { "" } else { "s" },
                            left.min(UNDO_SEND_SECS)
                        ))
                }
                None if self.status.is_empty() && !self.online && !self.loading && !self.accounts.is_empty() => {
                    div().truncate().text_color(rgb(MUTED)).child("Offline: showing saved mail")
                }
                None => div()
                    .truncate()
                    .text_color(rgb(if self.status_is_error { ERROR } else { MUTED }))
                    .child(self.status.clone()),
            },
        };
        let problems = self.problems_unseen;
        div()
            .flex_none()
            .h(px(30.))
            .px_3()
            .flex()
            .items_center()
            .justify_between()
            .gap_6()
            .bg(rgb(SIDEBAR))
            .border_t_1()
            .border_color(rgb(BORDER))
            .text_xs()
            .child(
                // Least useful hints are last, so a narrow window drops them.
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .items_center()
                    .gap_4()
                    .children(hints.into_iter().map(|(k, l)| hint_chip(k, l))),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .max_w(px(480.))
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .child(status)
                    .children(
                        self.sync_note
                            .clone()
                            .map(|n| div().flex_none().text_color(rgb(MUTED)).child(n)),
                    )
                    .when(problems > 0, |d| {
                        d.child(
                            div()
                                .id("problems-chip")
                                .flex_none()
                                .px_1p5()
                                .rounded_sm()
                                .bg(rgb(0x3b1f22))
                                .text_color(rgb(ERROR))
                                .cursor_pointer()
                                .hover(|d| d.bg(rgb(0x4a2529)))
                                .tooltip(|window, cx| Tooltip::new("Problems: see what went wrong").build(window, cx))
                                .on_click(cx.listener(|this, _, w, cx| this.open_settings(settings::Pane::Problems, w, cx)))
                                .child(format!("⚠ {problems}")),
                        )
                    }),
            )
    }

    /// A new status message: errors go in the log; every message fades
    /// from the bar after a while (errors stay longer).
    fn track_status(&mut self, cx: &mut Context<Self>) {
        if self.status == self.status_seen {
            return;
        }
        self.status_seen = self.status.clone();
        if self.status.is_empty() {
            return;
        }
        if self.status_is_error && errlog::record(&self.account, &self.status) {
            self.problems_unseen += 1;
            if self.settings_view.as_ref().is_some_and(|v| v.pane == settings::Pane::Problems) {
                self.problems = errlog::load();
            }
        }
        let shown = self.status.clone();
        let wait = std::time::Duration::from_secs(if self.status_is_error { 15 } else { 6 });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| {
                if this.status == shown {
                    this.status = SharedString::default();
                    this.status_seen = SharedString::default();
                    this.status_is_error = false;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The menu shown while `g` waits for its second key.
    fn render_go_menu(&self, cx: &App) -> impl IntoElement {
        let hints: Vec<Hint> = cx
            .global::<Shortcuts>()
            .hints
            .iter()
            .filter(|h| h.mode == Mode::Go)
            .cloned()
            .collect();
        // Over the message list, never the reader: the HTML view would cover it.
        div()
            .absolute()
            .left(px(SIDEBAR_WIDTH + 12.))
            .bottom(px(40.))
            .w(px(200.))
            .p_3()
            .rounded_lg()
            .bg(rgb(0x24242b))
            .border_1()
            .border_color(rgb(0x3d3d47))
            .shadow_lg()
            .flex()
            .flex_col()
            .gap_2()
            .text_xs()
            .child(div().text_color(rgb(MUTED)).font_weight(FontWeight::SEMIBOLD).child("GO TO"))
            .children(hints.into_iter().map(|h| hint_chip(h.keys, h.label)))
    }

    fn render_rows(&mut self, range: Range<usize>, cx: &mut Context<Self>) -> Vec<Stateful<Div>> {
        // All Inboxes says which account each message is in.
        let shorts: std::collections::HashMap<String, (String, u32)> = if self.unified {
            self.accounts.iter().map(|a| (a.email.clone(), (a.tag(), a.color()))).collect()
        } else {
            Default::default()
        };
        if range.end + 30 >= self.envelopes.len() {
            self.load_more(cx);
        }
        range
            .map(|ix| {
                let e = &self.envelopes[ix];
                let selected = ix == self.selected;
                let marked = self.marked.contains(&e.key());
                let row = div()
                    .id(("thread", ix))
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .flex()
                    .flex_row()
                    .overflow_hidden()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .when(marked && !selected, |d| d.bg(rgb(MARKED)))
                    .when(selected, |d| d.bg(rgb(SELECTED)))
                    .when(!selected && !marked, |d| d.hover(|d| d.bg(rgb(HOVER))))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ix, cx)))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, ev: &MouseDownEvent, window, cx| {
                            this.open_row_menu(ix, ev.position, window, cx)
                        }),
                    )
                    .on_scroll(cx.listener(move |this, ev: &ScrollEvent, window, cx| {
                        this.on_row_scroll(ix, ev, window, cx)
                    }))
                    // Selection bar.
                    .child(div().w(px(3.)).h_full().flex_none().when(selected, |d| d.bg(rgb(ACCENT))))
                    // Unread dot column.
                    .child(
                        div()
                            .w(px(18.))
                            .flex_none()
                            .pt(px(17.))
                            .flex()
                            .justify_center()
                            .when(marked, |d| {
                                d.pt(px(12.)).child(
                                    div()
                                        .size(px(14.))
                                        .rounded_sm()
                                        .bg(rgb(ACCENT))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .text_xs()
                                        .text_color(rgb(0xffffff))
                                        .child("✓"),
                                )
                            })
                            .when(e.unread && !marked, |d| d.child(div().size(px(7.)).rounded_full().bg(rgb(ACCENT)))),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pt(px(10.))
                            .pr_4()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .flex_none()
                                    .h(px(20.))
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .font_weight(if e.unread { FontWeight::BOLD } else { FontWeight::MEDIUM })
                                            .child(e.from.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .flex()
                                            .items_center()
                                            .gap_1p5()
.when_some(shorts.get(e.account.as_ref()).cloned(), |d, (tag, color)| {
                                                d.child(
                                                    div()
                                                        .px(px(5.))
                                                        .rounded_sm()
                                                        .bg(rgba((color << 8) | 0x33))
                                                        .text_xs()
                                                        .text_color(rgb(color))
                                                        .child(tag),
                                                )
                                            })
                                            .when(e.important, |d| {
                                                d.child(svg().path("icons/flag.svg").size(px(12.)).text_color(rgb(0xd9893b)))
                                            })
                                            .when(e.starred, |d| {
                                                d.child(svg().path("icons/star.svg").size(px(12.)).text_color(rgb(0xe3b341)))
                                            })
                                            .child(div().text_xs().text_color(rgb(MUTED)).child(e.date.clone())),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .h(px(20.))
                                    .truncate()
                                    .text_color(rgb(if e.unread { TEXT } else { SUBTLE }))
                                    .when(e.unread, |d| d.font_weight(FontWeight::MEDIUM))
                                    .child(e.subject.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .h(px(18.))
                                    .truncate()
                                    .text_xs()
                                    .text_color(rgb(MUTED))
                                    .child(e.preview.clone()),
                            ),
                    );
                self.swipe_wrap(ix, row)
            })
            .collect()
    }

    /// In Trash or Spam: empty it (everything on the server), after a second click.
    fn render_empty_button(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let folder = self.active().filter(|_| !self.unified && self.query.is_empty() && self.online)?;
        let screened = folder.name.eq_ignore_ascii_case(sweep::SCREENED);
        if !mail::is_trash_or_spam(&folder.name) && !screened {
            return None;
        }
        let what = if folder.name.eq_ignore_ascii_case(mail::TRASH) { "Trash" } else { "Spam" };
        let armed = self.empty_armed;
        let (idle, confirm) = if screened {
            ("Move all to Trash…".to_string(), "Move everything here to Trash".to_string())
        } else {
            (format!("Empty {what}…"), format!("Delete everything in {what} forever"))
        };
        Some(
            div()
                .px_1()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    action_button("empty-folder", if armed { confirm } else { idle }, false)
                    .when(armed, |d| d.bg(rgb(0x5a2024)).text_color(rgb(0xffd6d6)))
                    .on_click(cx.listener(|this, _, _, cx| this.empty_folder(cx))),
                )
                .when(armed, |d| {
                    d.child(
                        div()
                            .id("empty-cancel")
                            .text_xs()
                            .text_color(rgb(MUTED))
                            .cursor_pointer()
                            .hover(|d| d.text_color(rgb(TEXT)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.empty_armed = false;
                                cx.notify()
                            }))
                            .child("Cancel"),
                    )
                }),
        )
    }

    fn empty_folder(&mut self, cx: &mut Context<Self>) {
        if !self.empty_armed {
            self.empty_armed = true;
            cx.notify();
            return;
        }
        self.empty_armed = false;
        let Some(mailbox) = self
            .active()
            .map(|m| m.name.to_string())
            .filter(|m| mail::is_trash_or_spam(m) || m.eq_ignore_ascii_case(sweep::SCREENED))
        else {
            return;
        };
        let Some(session) = self.session.clone() else { return };
        let (store, acct) = (self.store.clone(), self.account.to_string());
        let to_trash = !mail::is_trash_or_spam(&mailbox);
        self.status = if to_trash { "Moving to Trash…".into() } else { "Emptying…".into() };
        self.status_is_error = false;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (mb, a) = (mailbox.clone(), acct.clone());
            let result = cx
                .background_executor()
                .spawn(async move {
                    let n = if to_trash {
                        session.lock().unwrap().trash_all(&mb)? as u64
                    } else {
                        session.lock().unwrap().empty_folder(&mb)?
                    };
                    if let Some(store) = store {
                        let store = store.lock().unwrap();
                        store.reset_mailbox(&a, &mb)?;
                        if to_trash {
                            // Trash's cache is stale now; it's read fresh next time.
                            store.reset_mailbox(&a, mail::TRASH)?;
                        }
                    }
                    anyhow::Ok(n)
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(n) => {
                        let s = if n == 1 { "" } else { "s" };
                        this.status = if to_trash {
                            format!("Moved {n} message{s} to Trash (Gmail empties it after 30 days)")
                        } else {
                            format!("Deleted {n} message{s} forever")
                        }
                        .into();
                        this.refresh_folders(cx);
                        this.status_is_error = false;
                        if *this.account == *acct && this.active().is_some_and(|m| m.name == mailbox) {
                            this.envelopes.clear();
                            this.selected = 0;
                        }
                    }
                    Err(err) => {
                        this.status = if to_trash {
                            format!("Could not move them all to Trash: {err:#}. Press it again to finish the rest.")
                        } else {
                            format!("Could not empty it: {err:#}")
                        }
                        .into();
                        this.status_is_error = true;
                        this.refresh_open_folder(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn render_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count = self.envelopes.len();
        let unread = self.envelopes.iter().filter(|e| e.unread).count();
        let searching = !self.query.is_empty();
        let title: SharedString = if searching {
            "Search results".into()
        } else if self.unified {
            "All Inboxes".into()
        } else {
            self.active().map(|m| m.label.clone()).unwrap_or_else(|| "Mail".into())
        };
        let subtitle: SharedString = if !self.marked.is_empty() {
            format!("{} selected", self.marked.len()).into()
        } else if self.loading_more {
            format!("{count} messages · loading older…").into()
        } else if searching && self.server_searching {
            format!("{count} cached · searching Gmail…").into()
        } else if searching {
            format!("{count} matches").into()
        } else if self.loading {
            "Loading…".into()
        } else if unread > 0 {
            format!("{count} messages · {unread} unread").into()
        } else {
            format!("{count} messages").into()
        };
        div()
            .w(px(LIST_WIDTH))
            .flex_none()
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(LIST_BG))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .key_context("SearchBox")
                    .flex_none()
                    .px_3()
                    .pt_3()
                    .pb_2()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(BORDER))
                    .child(
                        Input::new(&self.search)
                            .when(!self.search.read(cx).value().is_empty(), |input| {
                                input.suffix(
                                    div()
                                        .id("clear-search")
                                        .size(px(20.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .bg(rgb(0x3a3a44))
                                        .text_xs()
                                        .text_color(rgb(0xe8e8ec))
                                        .cursor_pointer()
                                        .hover(|d| d.bg(rgb(0x55555f)).text_color(rgb(0xffffff)))
                                        .on_click(cx.listener(|this, _, window, cx| this.clear_search(window, cx)))
                                        .child("✕"),
                                )
                            })
                            .bg(rgb(SEARCH_BG))
                            .border_color(rgb(SEARCH_BORDER))
                            .text_color(rgb(0xffffff)),
                    )
                    .child(
                        div()
                            .px_1()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .child(div().text_base().font_weight(FontWeight::SEMIBOLD).child(title))
                            .child(div().text_xs().text_color(rgb(MUTED)).child(subtitle)),
                    )
                    .children(self.render_empty_button(cx)),
            )
            .child(
                div().flex_1().min_h_0().child(
                    uniform_list(
                        "threads",
                        count,
                        cx.processor(|this, range, _window, cx| this.render_rows(range, cx)),
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                ),
            )
    }

    fn render_reader(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let reader = div().flex_1().min_w_0().h_full().flex().flex_col().overflow_hidden();
        let Some(e) = self.envelopes.get(self.selected) else {
            return reader
                .items_center()
                .justify_center()
                .text_color(rgb(MUTED))
                .child(if self.loading { "Loading…" } else { "No messages" });
        };
        let body = e.body.clone().unwrap_or_else(|| "Loading…".into());
        let has_html = e.html.is_some();
        let initial: SharedString = e
            .from
            .chars()
            .find(|c| c.is_alphanumeric())
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".into())
            .into();
        let header = div()
                        .flex_none()
                        .max_w(px(760.))
                        .px_8()
                        .pt_6()
                        .pb_4()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            div()
                                .text_xl()
                                .font_weight(FontWeight::SEMIBOLD)
                                .line_height(rems(1.75))
                                .child(e.subject.clone()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(
                                    div()
                                        .size(px(36.))
                                        .flex_none()
                                        .rounded_full()
                                        .bg(rgb(SELECTED))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(ACCENT))
                                        .child(initial),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(div().truncate().font_weight(FontWeight::MEDIUM).child(e.from.clone()))
                                        .child(div().truncate().text_xs().text_color(rgb(MUTED)).child(e.from_email.clone())),
                                )
                                .child(div().flex_none().text_xs().text_color(rgb(MUTED)).child(e.date.clone())),
                        )
                        .children(self.render_attachments(e, cx))
                        .child(div().h(px(1.)).bg(rgb(BORDER)));
        if has_html {
            let key = e.key();
            let blocked = !self.images_allowed.contains(&key)
                && e.html.as_ref().map(|h| html_view::has_remote_images(h)).unwrap_or(false);
            let bar = blocked.then(|| {
                div()
                    .flex_none()
                    .mx_6()
                    .mb_2()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(rgb(SEARCH_BG))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .text_xs()
                    .text_color(rgb(SUBTLE))
                    .child("Remote images are blocked to protect your privacy.")
                    .child(
                        div()
                            .id("load-images")
                            .flex_none()
                            .px_2()
                            .py_1()
                            .rounded_md()
                            .bg(rgb(0x3a3a44))
                            .text_color(rgb(TEXT))
                            .cursor_pointer()
                            .hover(|d| d.bg(rgb(0x55555f)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.images_allowed.insert(key.clone());
                                cx.notify();
                            }))
                            .child("Load images"),
                    )
            });
            // The web view is laid over this placeholder.
            return reader.child(header).children(bar).child(
                div()
                    .flex_1()
                    .min_h_0()
                    .mx_6()
                    .mb_6()
                    .rounded_lg()
                    .overflow_hidden()
                    .bg(rgb(0xffffff))
                    .child(self.html_view.placeholder()),
            );
        }
        reader.child(
            div()
                .id("reader")
                .size_full()
                .overflow_y_scroll()
                .child(header)
                .child(
                    div()
                        .max_w(px(760.))
                        .px_8()
                        .pb_6()
                        .text_sm()
                        .line_height(rems(1.45))
                        .text_color(rgb(TEXT))
                        .child(body),
                ),
        )
    }
}

impl Render for MailApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.track_status(cx);
        let html = self
            .envelopes
            .get(self.selected)
            .filter(|_| self.composer.is_none() && self.pending_send.is_none() && self.after_send.is_none() && self.sweep.is_none())
            .and_then(|e| e.html.clone().map(|h| (e.key(), h)));
        let own = if self.settings_view.is_some() {
            self.settings_html()
        } else {
            self.compose_html(cx).filter(|_| self.sweep.is_none())
        };
        let html = html.filter(|_| self.settings_view.is_none());
        match (own, html) {
            // Your own message: its images are yours, let them load.
            (Some((key, html)), _) => self.html_view.show(key, &html, true, window),
            (None, Some((key, html))) => {
                let allow = self.images_allowed.contains(&key);
                self.html_view.show(key, &html, allow, window)
            }
            (None, None) => self.html_view.hide(),
        }
        let counting_down = self.pending_send.is_some() || self.after_send.is_some();
        let mode = if self.label_picker.is_some() {
            Mode::Labels
        } else if self.row_menu.is_some() {
            Mode::Menu
        } else if self.settings_view.is_some() {
            Mode::Settings
        } else if counting_down {
            Mode::Countdown
        } else if self.sweep.is_some() && !self.search.read(cx).focus_handle(cx).is_focused(window) {
            Mode::Sweep
        } else if self.search.read(cx).focus_handle(cx).is_focused(window) {
            Mode::Search
        } else if self.composer.as_ref().is_some_and(|c| !c.suggestions.is_empty()) {
            Mode::Recipients
        } else if self.composer.is_some() {
            Mode::Writing
        } else {
            Mode::List
        };
        let main_panes = self.settings_view.is_none() && self.sweep.is_none();
        // Writing (and its send review) takes the message list's space.
        let writing_wide = self.settings.hide_list_when_writing && (self.composer.is_some() || counting_down);
        let go_pending = window
            .pending_input_keystrokes()
            .map(|k| k.len() == 1 && k[0].key == "g" && !k[0].modifiers.modified())
            .unwrap_or(false);
        // Views that cover the list mark the whole window, so list keys stay
        // off wherever focus lands (a click on blank space focuses the root).
        let mut root_context = KeyContext::default();
        root_context.add("MailApp");
        if self.settings_view.is_some() {
            root_context.add("Settings");
        }
        if self.sweep.is_some() {
            root_context.add("Sweep");
        }
        if self.composer.is_some() {
            root_context.add("Composer");
        }
        if self.label_picker.is_some() {
            root_context.add("LabelPicker");
        }
        div()
            .key_context(root_context)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_prev))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::on_clear_search))
            .on_action(cx.listener(Self::toggle_read))
            .on_action(cx.listener(Self::focus_list))
            .on_action(cx.listener(Self::compose))
            .on_action(cx.listener(|this, _: &Reply, w, cx| this.respond(Kind::Reply, w, cx)))
            .on_action(cx.listener(|this, _: &ReplyAll, w, cx| this.respond(Kind::ReplyAll, w, cx)))
            .on_action(cx.listener(|this, _: &Forward, w, cx| this.respond(Kind::Forward, w, cx)))
            .on_action(cx.listener(Self::open_draft))
            .on_action(cx.listener(Self::send_message))
            .on_action(cx.listener(Self::close_compose))
            .on_action(cx.listener(Self::discard_compose))
            .on_action(cx.listener(Self::undo_send))
            .on_action(cx.listener(|this, _: &SuggestNext, _, cx| this.suggest_move(1, cx)))
            .on_action(cx.listener(|this, _: &SuggestPrev, _, cx| this.suggest_move(-1, cx)))
            .on_action(cx.listener(Self::accept_suggestion))
            .on_action(cx.listener(Self::send_now))
            .on_action(cx.listener(Self::toggle_mark))
            .on_action(cx.listener(Self::mark_all))
            .on_action(cx.listener(|this, _: &Archive, w, cx| this.primary(w, cx)))
            .on_action(cx.listener(|this, _: &ToggleStar, w, cx| this.perform(actions::MsgAction::ToggleStar, w, cx)))
            .on_action(cx.listener(|this, _: &ToggleImportant, w, cx| this.perform(actions::MsgAction::ToggleImportant, w, cx)))
            .on_action(cx.listener(|this, _: &SenderKey, w, cx| this.sender_key(w, cx)))
            .on_action(cx.listener(|this, _: &SenderToReading, w, cx| this.perform(actions::MsgAction::SenderReading, w, cx)))
            .on_action(cx.listener(|this, _: &SenderToRecords, w, cx| this.perform(actions::MsgAction::SenderRecords, w, cx)))
            .on_action(cx.listener(|this, _: &FormatBold, w, cx| this.format(Format::Bold, w, cx)))
            .on_action(cx.listener(|this, _: &FormatItalic, w, cx| this.format(Format::Italic, w, cx)))
            .on_action(cx.listener(|this, _: &FormatStrike, w, cx| this.format(Format::Strike, w, cx)))
            .on_action(cx.listener(|this, _: &FormatLink, w, cx| this.format(Format::Link, w, cx)))
            .on_action(cx.listener(|this, _: &FormatCode, w, cx| this.format(Format::Code, w, cx)))
            .on_action(cx.listener(|this, _: &FormatBullets, w, cx| this.format(Format::Bullets, w, cx)))
            .on_action(cx.listener(|this, _: &FormatNumbers, w, cx| this.format(Format::Numbers, w, cx)))
            .on_action(cx.listener(|this, _: &FormatQuote, w, cx| this.format(Format::Quote, w, cx)))
            .on_action(cx.listener(|this, _: &TogglePreview, w, cx| this.toggle_preview(w, cx)))
            .on_action(cx.listener(|this, _: &ToggleMono, _, cx| this.toggle_mono(cx)))
            .on_action(cx.listener(|this, _: &AttachFile, _, cx| this.pick_attachments(cx)))
            .on_action(cx.listener(|this, _: &ShowBcc, w, cx| this.toggle_bcc(w, cx)))
            .on_action(cx.listener(|this, _: &FormatTable, w, cx| this.format(Format::Table, w, cx)))
            .on_action(cx.listener(|this, _: &SmartPaste, w, cx| this.smart_paste(w, cx)))
            .on_action(cx.listener(|this, _: &TableTab, w, cx| this.table_tab(false, w, cx)))
            .on_action(cx.listener(|this, _: &TableBackTab, w, cx| this.table_tab(true, w, cx)))
            .on_action(cx.listener(|this, _: &CycleSignature, _, cx| this.cycle_signature(cx)))
            .on_action(cx.listener(|this, _: &OpenSettings, w, cx| this.open_settings(settings::Pane::Writing, w, cx)))
            .on_action(cx.listener(|this, _: &CloseSettings, w, cx| this.close_settings(w, cx)))
            .on_action(cx.listener(|this, _: &MenuNext, _, cx| this.menu_move(1, cx)))
            .on_action(cx.listener(|this, _: &MenuPrev, _, cx| this.menu_move(-1, cx)))
            .on_action(cx.listener(|this, _: &MenuAccept, w, cx| this.menu_accept(w, cx)))
            .on_action(cx.listener(|this, _: &CloseMenu, w, cx| this.close_row_menu(w, cx)))
            .on_action(cx.listener(|this, _: &Delete, _, cx| this.bulk(BulkKind::Delete, cx)))
            .on_action(cx.listener(|this, _: &OpenSweep, w, cx| this.open_sweep(w, cx)))
            .on_action(cx.listener(|this, _: &CloseSweep, w, cx| this.close_sweep(w, cx)))
            .on_action(cx.listener(|this, _: &SweepNext, _, cx| this.sweep_move(1, cx)))
            .on_action(cx.listener(|this, _: &SweepPrev, _, cx| this.sweep_move(-1, cx)))
            .on_action(cx.listener(|this, _: &FileReading, _, cx| this.sweep_decide(sweep::READING, cx)))
            .on_action(cx.listener(|this, _: &FileRecords, _, cx| this.sweep_decide(sweep::RECORDS, cx)))
            .on_action(cx.listener(|this, _: &ScreenOut, _, cx| this.sweep_decide(sweep::SCREENED, cx)))
            .on_action(cx.listener(|this, _: &KeepSender, _, cx| this.sweep_decide(sweep::KEEP, cx)))
            .on_action(cx.listener(Self::toggle_hints))
            .on_action(cx.listener(|this, _: &OpenLabels, w, cx| this.open_label_picker(w, cx)))
            .on_action(cx.listener(|this, _: &LabelNext, _, cx| this.label_move(1, cx)))
            .on_action(cx.listener(|this, _: &LabelPrev, _, cx| this.label_move(-1, cx)))
            .on_action(cx.listener(|this, _: &LabelAccept, w, cx| this.label_accept(false, w, cx)))
            .on_action(cx.listener(|this, _: &LabelAcceptArchive, w, cx| this.label_accept(true, w, cx)))
            .on_action(cx.listener(|this, _: &CloseLabels, w, cx| this.close_label_picker(w, cx)))
            .on_action(cx.listener(|this, _: &GoInbox, w, cx| this.go_to("INBOX", w, cx)))
            .on_action(cx.listener(|this, _: &GoStarred, w, cx| this.go_to("[Gmail]/Starred", w, cx)))
            .on_action(cx.listener(|this, _: &GoSent, w, cx| this.go_to("[Gmail]/Sent Mail", w, cx)))
            .on_action(cx.listener(|this, _: &GoDrafts, w, cx| this.go_to("[Gmail]/Drafts", w, cx)))
            .on_action(cx.listener(|this, _: &GoAllMail, w, cx| this.go_to(ALL_MAIL, w, cx)))
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_row()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .text_sm()
            .relative()
            .flex_col()
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_row()
                    .child(self.render_sidebar(cx))
                    .when(self.settings_view.is_some(), |d| d.child(self.render_settings(cx)))
                    .when(self.settings_view.is_none() && self.sweep.is_some(), |d| d.child(self.render_sweep(cx)))
                    .when(main_panes && !writing_wide, |d| d.child(self.render_list(cx)))
                    .when(main_panes && writing_wide && self.composer.is_some(), |d| {
                        if self.settings.blocks_open {
                            d.child(self.render_blocks_panel(cx).into_any_element())
                        } else {
                            d.child(self.render_blocks_ribbon(cx).into_any_element())
                        }
                    })
                    .when(main_panes, |d| d.child(if counting_down {
                        self.render_send_review(cx).into_any_element()
                    } else if self.composer.is_some() {
                        self.render_composer(cx).into_any_element()
                    } else {
                        self.render_reader(cx).into_any_element()
                    })),
            )
            .child(self.render_bar(mode, cx))
            .when(go_pending, |d| d.child(self.render_go_menu(cx)))
            .when(self.row_menu.is_some(), |d| d.child(self.render_row_menu(cx)))
            .children(self.render_label_picker(cx))

    }
}

fn main() {
    // Settings moved to ~/.config/wax; bring old ones along before anything reads them.
    match files::migrate_config() {
        Ok(moved) if !moved.is_empty() => {
            errlog::record("", &format!("Moved settings to {}: {}", files::config_dir().display(), moved.join(", ")));
        }
        Err(e) => {
            errlog::record("", &format!("Could not move settings to {}: {e:#}", files::config_dir().display()));
        }
        _ => {}
    }
    let acct = crate::accounts::first_email();
    let args: Vec<String> = std::env::args().collect();
    if let Some(pos) = args.iter().position(|a| a == "--search") {
        let query = args[pos + 1..].join(" ");
        let started = std::time::Instant::now();
        let store = Store::open().expect("open cache");
        let hits = store.search(&*acct, &query, 20).expect("search");
        println!("{} hits for {query:?} in {:?}", hits.len(), started.elapsed());
        for e in hits {
            println!("  {:>9} | {:<22} | {} [{}]", e.date, e.from, e.subject, e.mailbox);
        }
        let started = std::time::Instant::now();
        match Session::connect(&*acct).and_then(|mut s| s.search("[Gmail]/All Mail", &query, 10)) {
            Ok(hits) => {
                println!("Gmail: {} hits in {:?}", hits.len(), started.elapsed());
                for e in hits {
                    println!("  {:>9} | {:<22} | {}", e.date, e.from, e.subject);
                }
            }
            Err(err) => println!("Gmail search failed: {err:#}"),
        }
        return;
    }
    // --mark <mailbox> <uid> read|unread : set read state from the shell.
    if let Some(pos) = args.iter().position(|a| a == "--mark") {
        let (mailbox, id, state) = (&args[pos + 1], &args[pos + 2], &args[pos + 3]);
        let result = Session::connect(&*acct).and_then(|mut s| {
            s.set_seen(mailbox, id, state == "read")?;
            let now = s.envelopes(mailbox, 200)?;
            Ok(now.into_iter().find(|e| &e.id == id).map(|e| e.unread))
        });
        match result {
            Ok(Some(unread)) => println!("{mailbox}/{id} is now {}", if unread { "unread" } else { "read" }),
            Ok(None) => println!("{mailbox}/{id} not in the newest 200"),
            Err(err) => println!("failed: {err:#}"),
        }
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--suggest") {
        let prefix = args.get(pos + 1).cloned().unwrap_or_default();
        let started = std::time::Instant::now();
        let store = Store::open().expect("open cache");
        let found = store.suggest_contacts(&*acct, &prefix, 6).expect("suggest");
        println!("{} for {prefix:?} in {:?}", found.len(), started.elapsed());
        for (name, email) in found {
            println!("  {}", compose::display_address(&name, &email));
        }
        return;
    }
    if args.iter().any(|a| a == "--last-sent") {
        match Session::connect(&*acct).and_then(|mut s| s.recent_headers("[Gmail]/Sent Mail", 3)) {
            Ok(text) => {
                let tail: Vec<&str> = text.lines().collect();
                println!("{}", tail[tail.len().saturating_sub(24)..].join("\n"));
            }
            Err(err) => println!("{err:#}"),
        }
        return;
    }
    // --archive-roundtrip <inbox uid>: archive one message, verify, restore.
    if let Some(pos) = args.iter().position(|a| a == "--archive-roundtrip") {
        let uid = args[pos + 1].clone();
        let result = (|| -> anyhow::Result<()> {
            let mut s = Session::connect(&*acct)?;
            let inbox = s.envelopes("INBOX", 200)?;
            let e = inbox.iter().find(|e| e.id == uid).ok_or_else(|| anyhow::anyhow!("uid not in newest 200"))?;
            let mid = e.message_id.clone().ok_or_else(|| anyhow::anyhow!("no Message-ID"))?;
            println!("archiving: {} | {}", e.from, e.subject);
            s.move_ids("INBOX", ALL_MAIL, &[uid.clone()])?;
            println!("in INBOX after archive: {:?}", s.find_by_message_id("INBOX", &mid)?);
            let in_all = s.find_by_message_id(ALL_MAIL, &mid)?.ok_or_else(|| anyhow::anyhow!("not in All Mail!"))?;
            println!("in All Mail: uid {in_all}");
            s.move_ids(ALL_MAIL, "INBOX", &[in_all])?;
            println!("back in INBOX: {:?}", s.find_by_message_id("INBOX", &mid)?);
            Ok(())
        })();
        if let Err(err) = result {
            println!("failed: {err:#}");
        }
        return;
    }
    // --sweep-roundtrip <email>: file a sender's mail in Records, verify,
    // then put everything back (and drop the label if the test made it).
    if let Some(pos) = args.iter().position(|a| a == "--sweep-roundtrip") {
        let email = args[pos + 1].to_lowercase();
        let result = (|| -> anyhow::Result<()> {
            let session: SharedSession = Arc::new(Mutex::new(Session::connect(&*acct)?));
            let store: SharedStore = Arc::new(Mutex::new(Store::open()?));
            let label = sweep::RECORDS;
            let had_label = session.lock().unwrap().mailboxes()?.iter().any(|m| m.eq_ignore_ascii_case(label));
            let inbox_before = session.lock().unwrap().uids_from("INBOX", &email)?;
            let mids: Vec<String> = {
                let list = session.lock().unwrap().envelopes("INBOX", 200)?;
                list.into_iter().filter(|e| inbox_before.contains(&e.id)).filter_map(|e| e.message_id).collect()
            };
            println!("{email}: {} in Inbox ({} Message-IDs), label existed: {had_label}", inbox_before.len(), mids.len());
            let n = sweep::apply_sender(&session, &store, &acct, &email, label)?;
            let in_label = session.lock().unwrap().uids_from(label, &email)?;
            let inbox_after = session.lock().unwrap().uids_from("INBOX", &email)?;
            println!("filed {n}: {} now labelled {label}, {} left in Inbox", in_label.len(), inbox_after.len());
            // Put it all back.
            session.lock().unwrap().move_ids(label, ALL_MAIL, &in_label)?;
            for mid in &mids {
                // Bind first: an `if let` on a lock() would hold the guard
                // through the block and deadlock on the next lock().
                let found = session.lock().unwrap().find_by_message_id(ALL_MAIL, mid)?;
                if let Some(uid) = found {
                    session.lock().unwrap().move_ids(ALL_MAIL, "INBOX", &[uid])?;
                }
            }
            let restored = session.lock().unwrap().uids_from("INBOX", &email)?;
            let still_labelled = session.lock().unwrap().uids_from(label, &email)?;
            println!("restored: {} in Inbox, {} still labelled", restored.len(), still_labelled.len());
            if !had_label && session.lock().unwrap().uids_from(label, "@")?.is_empty() {
                session.lock().unwrap().delete_mailbox(label)?;
                println!("removed the {label} label the test created");
            }
            Ok(())
        })();
        if let Err(err) = result {
            println!("failed: {err:#}");
        }
        return;
    }
    // Small read/repair tools: --where <message-id>, --move <from> <to> <uid>.
    if let Some(pos) = args.iter().position(|a| a == "--where") {
        let mid = &args[pos + 1];
        let result = Session::connect(&*acct).and_then(|mut s| {
            let boxes = s.mailboxes()?;
            let mut hits = Vec::new();
            for b in ["INBOX", ALL_MAIL, sweep::RECORDS, sweep::READING, sweep::SCREENED] {
                if b == "INBOX" || boxes.iter().any(|x| x.eq_ignore_ascii_case(b)) {
                    hits.push(format!("{b}: {:?}", s.find_by_message_id(b, mid)?));
                } else {
                    hits.push(format!("{b}: (no such label)"));
                }
            }
            Ok(hits)
        });
        match result {
            Ok(h) => h.iter().for_each(|l| println!("{l}")),
            Err(e) => println!("failed: {e:#}"),
        }
        return;
    }
    // Star and Important on the newest Inbox message, flipped and restored.
    if args.iter().any(|a| a == "--flags-roundtrip") {
        let run = || -> anyhow::Result<()> {
            let mut s = Session::connect(&*acct)?;
            let e = s.envelopes(mail::INBOX, 1)?.into_iter().next().ok_or_else(|| anyhow::anyhow!("empty Inbox"))?;
            let uid: u64 = e.id.parse()?;
            let show = |s: &mut Session, what: &str| -> anyhow::Result<()> {
                let h = s.headers_in_range(mail::INBOX, uid, uid)?;
                let x = h.first().ok_or_else(|| anyhow::anyhow!("uid {uid} gone"))?;
                println!("{what:<22} starred={} important={}  {}", x.starred, x.important, x.subject);
                Ok(())
            };
            show(&mut s, "start")?;
            let ids = vec![e.id.clone()];
            let star = if e.starred { ["-", "+"] } else { ["+", "-"] };
            s.store_op(mail::INBOX, &ids, &format!("{}FLAGS.SILENT (\\Flagged)", star[0]))?;
            show(&mut s, "star flipped")?;
            s.store_op(mail::INBOX, &ids, &format!("{}FLAGS.SILENT (\\Flagged)", star[1]))?;
            show(&mut s, "star restored")?;
            let start = s.headers_in_range(mail::INBOX, uid, uid)?[0].important;
            let imp = if start { ["-", "+"] } else { ["+", "-"] };
            s.store_op(mail::INBOX, &ids, &format!("{}X-GM-LABELS (\\Important)", imp[0]))?;
            show(&mut s, "important flipped")?;
            s.store_op(mail::INBOX, &ids, &format!("{}X-GM-LABELS (\\Important)", imp[1]))?;
            show(&mut s, "important restored")?;
            Ok(())
        };
        if let Err(e) = run() {
            println!("failed: {e:#}");
        }
        return;
    }
    // Import an HTML signature (e.g. from Thunderbird): --import-signature <file> <name>
    if let Some(pos) = args.iter().position(|a| a == "--import-signature") {
        let (file, label) = (&args[pos + 1], args.get(pos + 2).map(String::as_str).unwrap_or("Imported"));
        let run = || -> anyhow::Result<()> {
            let html = std::fs::read_to_string(file)?;
            let sig = signature::import_html(&html, label);
            let id = files::save_signature(&sig)?;
            println!(
                "saved #{id} {:?}: layout {}, name {:?}, title {:?}, phone {:?}, email {:?}, font {:?}, \
                 logo {} ({}px → {:?}), logo2 {} ({}px → {:?}), follow {:?} → {:?}",
                sig.label, sig.layout.as_str(), sig.name, sig.title, sig.phone, sig.email, sig.font,
                sig.logo.as_ref().map_or(0, |l| l.1.len()), sig.logo_width, sig.logo_link,
                sig.logo2.as_ref().map_or(0, |l| l.1.len()), sig.logo2_width, sig.logo2_link,
                sig.follow_line(), sig.social_url
            );
            Ok(())
        };
        if let Err(e) = run() {
            println!("failed: {e:#}");
        }
        return;
    }
    if let Some(pos) = args.iter().position(|a| a == "--move") {
        let (from, to, uid) = (&args[pos + 1], &args[pos + 2], args[pos + 3].clone());
        match Session::connect(&*acct).and_then(|mut s| s.move_ids(from, to, &[uid])) {
            Ok(()) => println!("moved"),
            Err(e) => println!("failed: {e:#}"),
        }
        return;
    }
    if args.iter().any(|a| a == "--label-counts") {
        let result = Session::connect(&*acct).and_then(|mut s| {
            let mut out = Vec::new();
            for b in [sweep::READING, sweep::RECORDS, sweep::SCREENED, "INBOX"] {
                out.push(format!("{b}: {}", s.mailbox_status(b).map(|st| st.exists.to_string()).unwrap_or_else(|e| format!("{e:#}"))));
            }
            for who in ["news@example.com", "digest@example.com", "alerts@example.com"] {
                out.push(format!("Inbox from {who}: {}", s.uids_from("INBOX", who)?.len()));
            }
            Ok(out)
        });
        match result {
            Ok(lines) => lines.iter().for_each(|l| println!("{l}")),
            Err(e) => println!("failed: {e:#}"),
        }
        return;
    }
    if args.iter().any(|a| a == "--smtp-check") {
        match mail::smtp_check(&*acct) {
            Ok(()) => println!("SMTP login OK (nothing sent)"),
            Err(err) => println!("{err:#}"),
        }
        return;
    }
    if std::env::args().any(|a| a == "--check") {
        if let Err(err) = mail::check() {
            eprintln!("Check failed: {err:#}");
            std::process::exit(1);
        }
        return;
    }
    gpui_kit::application().with_assets(assets::AppAssets).run(|cx: &mut App| {
        gpui_kit::init(cx);
        // Dark component theme so inputs match the app.
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, None, cx);
        // Bindings gpui-kit registered for its own widgets; kept when ours reload.
        let base: Vec<KeyBinding> = cx.key_bindings().borrow().bindings().cloned().collect();
        cx.set_global(keys::BaseBindings(base));
        for e in keys::apply(cx) {
            errlog::record("", &e);
        }
        cx.on_action(|_: &Quit, cx| cx.quit());

        let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Wax".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let view = cx.new(|cx| MailApp::new(window, cx));
            let focus = view.read(cx).focus.clone();
            window.focus(&focus, cx);
            view
        })
        .expect("failed to open window");
        cx.activate(true);
    });
}
