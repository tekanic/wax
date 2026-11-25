//! Accounts in the app: switching the account on screen, Settings →
//! Accounts (add with Google, rename, sign in again, remove), and the
//! account headers in the sidebar.

use super::*;
use crate::accounts::{self as acc, Account, Auth};
use crate::settings::setting_row;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct AccountEditor {
    pub email: String,
    pub name: Entity<InputState>,
    pub badge: Entity<InputState>,
    pub _subs: Vec<Subscription>,
}

impl MailApp {
    pub fn current_account(&self) -> Option<&Account> {
        self.accounts.iter().find(|a| *a.email == *self.account)
    }

    /// The open account's default signatures, as Settings → Writing shows them.
    pub fn use_account_settings(&mut self) {
        let (n, r) = self.current_account().map(|a| (a.sig_new, a.sig_reply)).unwrap_or((None, None));
        self.settings.sig_new = n;
        self.settings.sig_reply = r;
    }

    /// Write the open account's default signatures back to its file.
    pub fn save_account_signatures(&mut self) {
        let (n, r) = (self.settings.sig_new, self.settings.sig_reply);
        let acct = self.account.clone();
        if let Some(a) = self.accounts.iter_mut().find(|a| *a.email == *acct) {
            a.sig_new = n;
            a.sig_reply = r;
            if let Err(e) = acc::save(a) {
                self.status = format!("Could not save the account: {e:#}").into();
                self.status_is_error = true;
            }
        }
    }

    pub fn reload_accounts(&mut self, cx: &mut Context<Self>) {
        self.accounts = acc::load();
        self.start_watches(cx);
        cx.notify();
    }

    /// Show another account. Its cached mail appears at once while it
    /// connects; decisions still waiting out their undo time are done first.
    pub fn switch_account(&mut self, email: &str, cx: &mut Context<Self>) {
        if *self.account == *email {
            return;
        }
        for id in self.pending_bulk.iter().map(|b| b.id).collect::<Vec<_>>() {
            self.commit_bulk(id, cx);
        }
        for k in self.pending_sweep.iter().map(|p| sweep::sweep_key(&p.account, &p.email)).collect::<Vec<_>>() {
            self.commit_sweep(&k, cx);
        }
        let old = std::mem::replace(&mut self.account, email.into());
        if let Some(s) = self.session.take() {
            if !old.is_empty() {
                self.pool.insert(old.to_string(), s);
            }
        }
        self.pool.remove(email);
        self.preview_session = None;
        self.unified = false;
        self.connect_attempts = 0;
        self.online = false;
        self.mailboxes = mailbox_items(vec!["INBOX".into()]);
        self.active_mailbox = 0;
        self.envelopes.clear();
        self.selected = 0;
        self.marked.clear();
        self.prefetching.clear();
        self.label_counts.clear();
        self.has_more = false;
        self.sweep = None;
        self.row_menu = None;
        self.sync_note = None;
        self.query.clear();
        self.loading = true;
        self.use_account_settings();
        self.load_rules();
        self.status = format!("Connecting to {email}…").into();
        self.status_is_error = false;
        self.load_from_cache(cx);
        self.connect(cx);
        cx.notify();
    }

    // ----- Signing in ----------------------------------------------------------

    /// Browser sign-in; `hint` pre-fills the address (signing in again).
    pub fn add_google_account(&mut self, hint: Option<String>, cx: &mut Context<Self>) {
        if self.signing_in.is_some() {
            return;
        }
        if oauth::load_client().is_none() {
            self.status = "Choose your Google client file first (Settings → Accounts)".into();
            self.status_is_error = true;
            cx.notify();
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.signing_in = Some(cancel.clone());
        self.status = "Finish signing in in your browser…".into();
        self.status_is_error = false;
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx.background_executor().spawn(async move { oauth::sign_in(hint.as_deref(), cancel) }).await;
            this.update(cx, |this, cx| {
                this.signing_in = None;
                match result {
                    Ok(who) => {
                        let existing = acc::find(&who.email);
                        let mut a = existing.clone().unwrap_or_else(|| Account::new(&who.email, &who.name));
                        a.auth = Auth::Google;
                        if a.name.trim().is_empty() {
                            a.name = who.name.clone();
                        }
                        if let Err(e) = acc::save(&a) {
                            this.status = format!("Could not save the account: {e:#}").into();
                            this.status_is_error = true;
                            return;
                        }
                        this.reload_accounts(cx);
                        this.status = if existing.is_some() {
                            format!("{} is signed in with Google", a.email)
                        } else {
                            format!("Added {}", a.email)
                        }
                        .into();
                        this.status_is_error = false;
                        if this.account.is_empty() {
                            this.switch_account(&a.email, cx);
                        } else if *this.account == *a.email {
                            // Reconnect with the new sign-in.
                            this.account = "".into();
                            this.switch_account(&a.email, cx);
                        }
                    }
                    Err(e) => {
                        this.status = format!("Not signed in: {e:#}").into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if let Some(flag) = self.signing_in.take() {
            flag.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    pub fn choose_google_client(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use this client".into()),
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            this.update(cx, |this, cx| {
                match oauth::import_client(&path) {
                    Ok(()) => {
                        this.status = "Google sign-in is ready".into();
                        this.status_is_error = false;
                    }
                    Err(e) => {
                        this.status = format!("{e:#}").into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    // ----- Editing -------------------------------------------------------------

    fn edit_account(&mut self, email: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = self.accounts.iter().find(|a| a.email == email).cloned() else { return };
        let name = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder("Name shown on mail you send");
            s.set_value(a.name.clone(), window, cx);
            s
        });
        let who = a.email.clone();
        let sub = cx.subscribe(&name, move |this, state, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let v = state.read(cx).value().trim().to_string();
                if let Some(a) = this.accounts.iter_mut().find(|a| a.email == who) {
                    a.name = v;
                    acc::save(a).ok();
                }
                cx.notify();
            }
        });
        let auto = Account { badge: String::new(), ..a.clone() }.initial();
        let badge = cx.new(|cx| {
            let mut s = InputState::new(window, cx).placeholder(format!("{auto}  (automatic)"));
            s.set_value(a.badge.clone(), window, cx);
            s
        });
        let who2 = a.email.clone();
        let badge_sub = cx.subscribe(&badge, move |this, state, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                let v = acc::clean_badge(&state.read(cx).value());
                if let Some(a) = this.accounts.iter_mut().find(|a| a.email == who2) {
                    a.badge = v;
                    acc::save(a).ok();
                }
                cx.notify();
            }
        });
        if let Some(v) = &mut self.settings_view {
            v.account = Some(AccountEditor { email: a.email.clone(), name, badge, _subs: vec![sub, badge_sub] });
            v.confirm_remove = None;
        }
        cx.notify();
    }

    fn set_account_color(&mut self, email: &str, color: Option<u32>, cx: &mut Context<Self>) {
        if let Some(a) = self.accounts.iter_mut().find(|a| a.email == email) {
            a.color = color;
            if let Err(e) = acc::save(a) {
                self.status = format!("Could not save the account: {e:#}").into();
                self.status_is_error = true;
            }
        }
        cx.notify();
    }

    fn remove_account(&mut self, email: &str, cx: &mut Context<Self>) {
        let confirmed = self.settings_view.as_ref().is_some_and(|v| v.confirm_remove.as_deref() == Some(email));
        if !confirmed {
            if let Some(v) = &mut self.settings_view {
                v.confirm_remove = Some(email.to_string());
            }
            cx.notify();
            return;
        }
        if let Err(e) = acc::remove(email) {
            self.status = format!("Could not remove {email}: {e:#}").into();
            self.status_is_error = true;
            cx.notify();
            return;
        }
        if let Some(v) = &mut self.settings_view {
            v.confirm_remove = None;
            v.account = None;
        }
        self.watch.remove(email);
        self.pool.remove(email);
        self.reload_accounts(cx);
        self.status = format!("Removed {email}").into();
        self.status_is_error = false;
        if *self.account == *email {
            match self.accounts.first().map(|a| a.email.clone()) {
                Some(next) => self.switch_account(&next, cx),
                None => {
                    self.account = "".into();
                    self.session = None;
                    self.preview_session = None;
                    self.envelopes.clear();
                    self.mailboxes.clear();
                }
            }
        }
        cx.notify();
    }

    // ----- Settings → Accounts -------------------------------------------------

    pub fn render_accounts_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.settings_view.as_ref();
        let editing = view.and_then(|v| v.account.as_ref());
        let confirm = view.and_then(|v| v.confirm_remove.clone());
        let has_client = oauth::load_client().is_some();
        let mut list = div().flex().flex_col().gap_2();
        for a in &self.accounts {
            let email = a.email.clone();
            let on = editing.is_some_and(|e| e.email == a.email);
            let signed_in = a.auth == Auth::Google && oauth::is_signed_in(&a.email);
            let state = match (a.auth, signed_in) {
                (Auth::Google, true) => "Signed in with Google".to_string(),
                (Auth::Google, false) => "Signed out: sign in again".to_string(),
                (Auth::AppPassword, _) => "Old app-password setup: sign in with Google to switch".to_string(),
            };
            let needs_sign_in = !signed_in;
            let removing = confirm.as_deref() == Some(a.email.as_str());
            let (e1, e2, e3) = (email.clone(), email.clone(), email.clone());
            let row = div()
                .id(SharedString::from(format!("acct-{email}")))
                .px_4()
                .py_3()
                .rounded_md()
                .border_1()
                .border_color(rgb(if on { ACCENT } else { BORDER }))
                .when(on, |d| d.bg(rgb(SELECTED)))
                .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER))))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _, w, cx| this.edit_account(&e1, w, cx)))
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .child(account_badge(&a.initial(), a.color(), 26.))
                .child(
                    div()
                        .flex_1()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .text_sm()
                                .child(div().text_color(rgb(TEXT)).child(a.email.clone()))
                                .when(*self.account == *a.email, |d| d.child(div().text_color(rgb(MUTED)).child("· on screen"))),
                        )
                        .child(div().text_xs().text_color(rgb(if needs_sign_in { 0xe0a050 } else { MUTED })).child({
                            let open = mail::open_connections(&a.email);
                            format!(
                                "{} · {} · {open} of {} connections open",
                                if a.name.trim().is_empty() { "No sender name".to_string() } else { a.name.clone() },
                                state,
                                mail::MAX_CONNECTIONS
                            )
                        })),
                )
                .child(
                    div()
                        .flex()
                        .flex_none()
                        .gap_1p5()
                        .when(needs_sign_in && has_client, |d| {
                            d.child(action_button(SharedString::from(format!("signin-{email}")), "Sign in with Google", true).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.add_google_account(Some(e2.clone()), cx)
                                }),
                            ))
                        })
                        .child(
                            action_button(
                                SharedString::from(format!("remove-{email}")),
                                if removing { "Click again to remove" } else { "Remove" },
                                false,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.remove_account(&e3, cx)
                            })),
                        ),
                );
            list = list.child(row);
            if let (true, Some(ed)) = (on, editing) {
                list = list.child(
                    div()
                        .px_4()
                        .py_3()
                        .rounded_md()
                        .bg(rgb(0x1b1b20))
                        .flex()
                        .flex_col()
                        .child(setting_row(
                            "Sender name",
                            "Shown to people you write to from this account.",
                            div().w(px(280.)).child(Input::new(&ed.name).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER))),
                        ))
                        .child(setting_row(
                            "Badge",
                            "One or two characters (an emoji works); empty = automatic.",
                            div()
                                .flex()
                                .items_center()
                                .gap_3()
                                .child(account_badge(&a.initial(), a.color(), 26.))
                                .child(div().w(px(120.)).child(Input::new(&ed.badge).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER)))),
                        ))
                        .child(setting_row("Colour", "Its badge in the sidebar and its tag in All Inboxes.", {
                            let mut swatches = div().flex().items_center().gap_1p5();
                            let current = a.color;
                            let auto_color = Account { color: None, ..a.clone() }.color();
                            let auto_email = a.email.clone();
                            swatches = swatches.child(
                                crate::settings::choice(SharedString::from(format!("color-auto-{}", a.email)), "Automatic", current.is_none())
                                    .on_click(cx.listener(move |this, _, _, cx| this.set_account_color(&auto_email, None, cx)))
                                    .child(div().ml_1p5().size(px(10.)).rounded_full().bg(rgb(auto_color))),
                            );
                            for (i, c) in acc::PALETTE.iter().enumerate() {
                                let c = *c;
                                let on = current == Some(c);
                                let email = a.email.clone();
                                swatches = swatches.child(
                                    div()
                                        .id(SharedString::from(format!("color-{}-{i}", a.email)))
                                        .size(px(22.))
                                        .rounded_full()
                                        .bg(rgb(c))
                                        .cursor_pointer()
                                        .border_2()
                                        .border_color(rgb(if on { 0xffffff } else { 0x1b1b20 }))
                                        .hover(|d| d.border_color(rgb(0xb8b8c0)))
                                        .on_click(cx.listener(move |this, _, _, cx| this.set_account_color(&email, Some(c), cx))),
                                );
                            }
                            swatches
                        }))
                        .child(div().pt_3().text_xs().text_color(rgb(MUTED)).child(
                            "This account's signatures: Signatures. Its labels: Labels. Extra From addresses: From addresses. Sender rules: accounts/<address>/rules.toml.",
                        )),
                );
            }
        }
        if confirm.is_some() {
            list = list.child(div().text_xs().text_color(rgb(MUTED)).child(
                "Removing signs the account out of Google here and puts its settings and rules in accounts/.removed. Its mail stays in Gmail.",
            ));
        }
        let add = if self.signing_in.is_some() {
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(div().text_sm().text_color(rgb(SUBTLE)).child("Waiting for you to finish in the browser…"))
                .child(action_button("signin-cancel", "Cancel", false).on_click(cx.listener(|this, _, _, cx| this.cancel_sign_in(cx))))
        } else {
            div().flex().child(
                action_button("account-add", "Add a Google account", true)
                    .on_click(cx.listener(|this, _, _, cx| this.add_google_account(None, cx))),
            )
        };
        let client_box = if has_client {
            div().flex().items_center().gap_2().text_xs().text_color(rgb(MUTED)).child(
                if oauth::client_file().exists() { "Google sign-in uses your own Google Cloud client." } else { "Google sign-in is built in." },
            ).when(oauth::client_file().exists(), |d| {
                d.child(
                    div()
                        .id("client-change")
                        .text_color(rgb(ACCENT))
                        .cursor_pointer()
                        .hover(|d| d.underline())
                        .on_click(cx.listener(|this, _, _, cx| this.choose_google_client(cx)))
                        .child("Change…"),
                )
            })
        } else {
            let step = |n: &str, t: &str| {
                div().flex().gap_2().text_sm().child(div().w(px(18.)).flex_none().text_color(rgb(MUTED)).child(n.to_string())).child(
                    div().flex_1().min_w_0().text_color(rgb(SUBTLE)).child(t.to_string()),
                )
            };
            div()
                .p_4()
                .rounded_md()
                .border_1()
                .border_color(rgb(BORDER))
                .flex()
                .flex_col()
                .gap_2()
                .child(div().text_sm().text_color(rgb(TEXT)).child("Set up Google sign-in (once)"))
                .child(div().text_xs().text_color(rgb(MUTED)).child(
                    "This build has no Google client of its own, so it uses one from your Google Cloud project. Free, about 10 minutes.",
                ))
                .child(step("1", "console.cloud.google.com: create a project and turn on the Gmail API."))
                .child(step("2", "Google Auth Platform: Branding (any app name), Audience: External, Data access: add https://mail.google.com/."))
                .child(step("3", "Clients: create a Desktop app client and download its JSON."))
                .child(step("4", "Audience: Publish app. In Testing, Google signs you out every 7 days."))
                .child(
                    div().pt_1().flex().child(
                        action_button("client-choose", "Choose the client file…", true)
                            .on_click(cx.listener(|this, _, _, cx| this.choose_google_client(cx))),
                    ),
                )
        };
        div()
            .id("accounts-pane")
            .size_full()
            .overflow_y_scroll()
            .px_8()
            .py_5()
            .child(
                div()
                    .max_w(px(760.))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_sm().text_color(rgb(SUBTLE)).child(
                        "Your mail accounts. Each has its own sender name, signatures, From addresses and sender rules. Gmail and Google Workspace accounts sign in with Google; the app never sees your password.",
                    ))
                    .child(client_box)
                    .when(!self.accounts.is_empty(), |d| d.child(list))
                    .when(has_client, |d| d.child(add)),
            )
    }

    // ----- All Inboxes, All Sent -------------------------------------------------

    /// Every account's Inbox (or Sent) in one list, newest first. Cached
    /// mail shows at once; each account is then refreshed on its own
    /// connection.
    pub fn open_unified(&mut self, mailbox: &'static str, cx: &mut Context<Self>) {
        if self.accounts.is_empty() {
            return;
        }
        self.unified = true;
        self.unified_box = mailbox;
        self.sweep = None;
        self.row_menu = None;
        self.query.clear();
        self.marked.clear();
        self.selected = 0;
        let mut all = Vec::new();
        if let Some(store) = &self.store {
            let store = store.lock().unwrap();
            for a in &self.accounts {
                all.extend(store.envelopes(&a.email, mailbox, PAGE_SIZE).unwrap_or_default());
            }
        }
        all.sort_by(|a, b| b.ts.cmp(&a.ts));
        self.envelopes.clear();
        self.set_envelopes(all, cx);
        self.has_more = false;
        self.fill_pool(cx);
        for a in self.accounts.iter().map(|a| a.email.clone()).collect::<Vec<_>>() {
            self.refresh_unified_account(&a, cx);
        }
        cx.notify();
    }

    /// Re-read one account's Inbox (or Sent) into the combined list.
    pub fn refresh_unified_account(&mut self, account: &str, cx: &mut Context<Self>) {
        let Some(session) = self.preview_of(account) else { return };
        let Some(store) = self.store.clone() else { return };
        let account = account.to_string();
        let mailbox = self.unified_box;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let a = account.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let fresh = session.lock().unwrap().envelopes(mailbox, PAGE_SIZE)?;
                    let mut store = store.lock().unwrap();
                    store.save_envelopes(&a, mailbox, &fresh)?;
                    store.envelopes(&a, mailbox, PAGE_SIZE)
                })
                .await;
            this.update(cx, |this, cx| {
                if !this.unified || this.unified_box != mailbox || !this.query.is_empty() {
                    return;
                }
                match result {
                    Ok(list) => {
                        let everything = this.all_messages();
                        let mut merged: Vec<Envelope> =
                            everything.iter().filter(|e| e.account.as_ref() != account).cloned().collect();
                        let loaded: std::collections::HashMap<(String, String), Envelope> = everything
                            .iter()
                            .filter(|e| e.account.as_ref() == account && e.body.is_some())
                            .map(|e| (e.key(), e.clone()))
                            .collect();
                        merged.extend(list.into_iter().map(|e| loaded.get(&e.key()).cloned().map(|old| Envelope { unread: e.unread, starred: e.starred, important: e.important, to: e.to.clone(), thread: e.thread.clone(), ..old }).unwrap_or(e)));
                        merged.sort_by(|a, b| b.ts.cmp(&a.ts));
                        this.set_envelopes(merged, cx);
                        this.has_more = false;
                    }
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

    /// Clicking an account: show it (or its Inbox, coming from All Inboxes).
    pub fn open_account(&mut self, email: &str, cx: &mut Context<Self>) {
        if *self.account != *email {
            self.switch_account(email, cx);
        } else if self.unified {
            let ix = self.mailboxes.iter().position(|m| m.name.eq_ignore_ascii_case(mail::INBOX)).unwrap_or(0);
            self.open_mailbox(ix, cx);
        }
    }

    // ----- Sidebar -------------------------------------------------------------

    /// All Inboxes and the accounts, pinned above the folder list.
    pub fn render_accounts_box(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut b = div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(2.))
            .px(px(8.))
            .pt(px(10.))
            .pb(px(8.))
            .border_b_1()
            .border_color(rgb(BORDER));
        if self.accounts.is_empty() {
            return b.child(
                div()
                    .id("no-accounts")
                    .px(px(10.))
                    .py_2()
                    .text_xs()
                    .text_color(rgb(ACCENT))
                    .cursor_pointer()
                    .hover(|d| d.underline())
                    .on_click(cx.listener(|this, _, w, cx| this.open_settings(crate::settings::Pane::Accounts, w, cx)))
                    .child("Add an account…"),
            );
        }
        if self.accounts.len() > 1 {
            let on = self.unified && self.unified_box == mail::INBOX && self.sweep.is_none();
            let unseen: u64 = self.watch.values().map(|w| w.unseen).sum();
            b = b.child(
                div()
                    .id("all-inboxes")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_color(rgb(if on { TEXT } else { SUBTLE }))
                    .when(on, |d| d.bg(rgb(SELECTED)).font_weight(FontWeight::MEDIUM))
                    .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT))))
                    .on_click(cx.listener(|this, _, w, cx| {
                        if !this.query.is_empty() {
                            this.search.update(cx, |s, cx| s.set_value("", w, cx));
                        }
                        this.close_sweep(w, cx);
                        this.open_unified(mail::INBOX, cx)
                    }))
                    .child(svg().path("icons/inbox.svg").size(px(16.)).flex_none().text_color(rgb(ACCENT)))
                    .child(div().flex_1().child("All Inboxes"))
                    .when(unseen > 0, |d| d.child(div().flex_none().text_xs().text_color(rgb(ACCENT)).child(unseen.to_string()))),
            );
            let on = self.unified && self.unified_box == mail::SENT && self.sweep.is_none();
            b = b.child(
                div()
                    .id("all-sent")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_color(rgb(if on { TEXT } else { SUBTLE }))
                    .when(on, |d| d.bg(rgb(SELECTED)).font_weight(FontWeight::MEDIUM))
                    .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT))))
                    .on_click(cx.listener(|this, _, w, cx| {
                        if !this.query.is_empty() {
                            this.search.update(cx, |s, cx| s.set_value("", w, cx));
                        }
                        this.close_sweep(w, cx);
                        this.open_unified(mail::SENT, cx)
                    }))
                    .child(svg().path("icons/send.svg").size(px(16.)).flex_none().text_color(rgb(ACCENT)))
                    .child(div().flex_1().child("All Sent")),
            );
        }
        for a in &self.accounts {
            b = b.child(self.render_account_header(a, cx));
        }
        b
    }

    /// One row per account; the one on screen has its folders under it.
    pub fn render_account_header(&self, a: &Account, cx: &mut Context<Self>) -> impl IntoElement {
        let here = *self.account == *a.email;
        let on = here && !self.unified;
        let unseen = self.watch.get(&a.email).map_or(0, |w| w.unseen);
        let email = a.email.clone();
        let initial = a.initial();
        let color = a.color();
        div()
            .id(SharedString::from(format!("side-acct-{}", a.email)))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(30.))
            .px(px(10.))
            .rounded(px(6.))
            .cursor_pointer()
            .text_color(rgb(if on { TEXT } else { SUBTLE }))
            .when(on, |d| d.bg(rgb(SELECTED)))
            .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT))))
            .tooltip({
                let email = a.email.clone();
                move |window, cx| Tooltip::new(email.clone()).build(window, cx)
            })
            .on_click(cx.listener(move |this, _, w, cx| {
                if !this.query.is_empty() {
                    this.search.update(cx, |s, cx| s.set_value("", w, cx));
                }
                this.close_sweep(w, cx);
                this.open_account(&email, cx)
            }))
            .child(account_badge(&initial, color, 18.))
            .child(div().flex_1().min_w_0().truncate().when(here, |d| d.font_weight(FontWeight::MEDIUM)).child(a.short()))
            .when(unseen > 0, |d| d.child(div().flex_none().text_xs().text_color(rgb(MUTED)).child(unseen.to_string())))
    }
}


/// A round badge with the account's letter, centred both ways (the line
/// box is the badge's height, so the glyph sits in the middle).
pub fn account_badge(initial: &str, color: u32, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(rgb(color))
        .text_color(rgb(0xffffff))
        .text_size(px(size * if initial.chars().count() > 1 { 0.42 } else { 0.55 }))
        .line_height(px(size))
        .font_weight(FontWeight::BOLD)
        .child(div().text_center().child(initial.to_string()))
}
