//! Labels: the `l` picker that labels messages in a few keystrokes, and
//! Settings → Labels, where each account's labels are created, renamed and
//! deleted.
//!
//! On Gmail a label is an IMAP folder. Labelling a message is a STORE of
//! X-GM-LABELS (the message stays where it is); renaming a label renames
//! its nested labels too; deleting one leaves the messages in All Mail.

use super::*;
use std::collections::HashMap;

/// The Sweep's labels are kept by name in sender rules; they're managed
/// from the Sweep, not here.
fn sweep_label(name: &str) -> bool {
    [sweep::READING, sweep::RECORDS, sweep::SCREENED].iter().any(|l| l.eq_ignore_ascii_case(name))
}

/// How a label reads (labels you made under [Gmail]/ without the prefix).
pub fn display(name: &str) -> &str {
    name.strip_prefix("[Gmail]/").unwrap_or(name)
}

/// Labels you can put on mail: your own plus the Sweep's, from a folder list.
pub fn user_labels(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = mailbox_items(names.to_vec())
        .into_iter()
        // Gmail's own folders are "system"; labels you made under [Gmail]/
        // (e.g. [Gmail]/Archive) are still yours.
        .filter(|m| m.exists && (!m.system || sweep_label(&m.name)))
        .map(|m| m.name.to_string())
        .collect();
    out.sort_by_key(|n| display(n).to_lowercase());
    out
}

pub struct LabelPicker {
    pub account: String,
    pub targets: Vec<Envelope>,
    pub input: Entity<InputState>,
    pub labels: Vec<String>,
    /// Labels on each target (uid → labels), once fetched.
    pub current: Option<HashMap<String, Vec<String>>>,
    pub ix: usize,
    pub _sub: Subscription,
}

/// A row in the picker.
#[derive(Clone, PartialEq)]
pub enum Pick {
    Label(String),
    Create(String),
}

impl LabelPicker {
    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_string()
    }

    pub fn rows(&self, cx: &App) -> Vec<Pick> {
        let q = self.query(cx);
        let ql = q.to_lowercase();
        let mut rows: Vec<Pick> =
            self.labels.iter().filter(|l| ql.is_empty() || display(l).to_lowercase().contains(&ql)).cloned().map(Pick::Label).collect();
        if !q.is_empty() && !self.labels.iter().any(|l| display(l).eq_ignore_ascii_case(&q)) {
            rows.push(Pick::Create(q));
        }
        rows
    }

    /// Every target has `label`.
    pub fn has(&self, label: &str) -> bool {
        let Some(cur) = &self.current else { return false };
        !self.targets.is_empty()
            && self.targets.iter().all(|e| cur.get(&e.id).is_some_and(|ls| ls.iter().any(|l| l.eq_ignore_ascii_case(label))))
    }
}

/// Settings → Labels state.
#[derive(Default)]
pub struct LabelsPane {
    pub account: String,
    pub labels: Vec<String>,
    pub loading: bool,
    /// Label being renamed, and its input.
    pub renaming: Option<(String, Entity<InputState>)>,
    pub new_input: Option<Entity<InputState>>,
    pub confirm_delete: Option<String>,
    pub busy: bool,
}

impl MailApp {
    // ----- Folder lists per account -------------------------------------------

    /// An account's labels as known now (the folder list on screen, or the
    /// cache for other accounts).
    pub fn labels_known(&self, account: &str) -> Vec<String> {
        if *account == *self.account {
            let names: Vec<String> = self.mailboxes.iter().map(|m| m.name.to_string()).collect();
            return user_labels(&names);
        }
        let names = self.store.as_ref().and_then(|s| s.lock().unwrap().mailboxes(account).ok()).unwrap_or_default();
        user_labels(&names)
    }

    /// A connection for `account`, opening one if needed (blocking: call
    /// from the background).
    fn connection_for(&self, account: &str) -> impl FnOnce() -> anyhow::Result<SharedSession> + Send + 'static {
        let existing = self.session_of(account);
        let account = account.to_string();
        move || match existing {
            Some(s) => Ok(s),
            None => Ok(Arc::new(Mutex::new(Session::connect(&account)?))),
        }
    }

    // ----- The `l` picker -------------------------------------------------------

    pub fn open_label_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let targets = self.targets();
        let Some(first) = targets.first() else { return };
        let account = self.account_of(first).to_string();
        // One account's labels at a time (All Inboxes can mix accounts).
        let targets: Vec<Envelope> =
            targets.into_iter().filter(|e| *self.account_of(e) == *account && e.mailbox != "demo").collect();
        if targets.is_empty() {
            return;
        }
        self.row_menu = None;
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Label…  (type a new name to create it)"));
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe(&input, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                if let Some(p) = &mut this.label_picker {
                    p.ix = 0;
                }
                cx.notify();
            }
        });
        let labels = self.labels_known(&account);
        self.label_picker = Some(LabelPicker { account: account.clone(), targets: targets.clone(), input, labels, current: None, ix: 0, _sub: sub });
        cx.notify();
        // Which labels they already have, so the picker can tick them.
        let connect = self.connection_for(&account);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let session = connect()?;
                    let mut by_box: HashMap<String, Vec<String>> = HashMap::new();
                    for e in &targets {
                        by_box.entry(e.mailbox.clone()).or_default().push(e.id.clone());
                    }
                    let mut all = HashMap::new();
                    for (mailbox, ids) in by_box {
                        all.extend(session.lock().unwrap().labels_of(&mailbox, &ids)?);
                    }
                    anyhow::Ok(all)
                })
                .await;
            this.update(cx, |this, cx| {
                if let (Some(p), Ok(cur)) = (&mut this.label_picker, result) {
                    p.current = Some(cur);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn close_label_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.label_picker.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    pub fn label_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(p) = &self.label_picker else { return };
        let n = p.rows(cx).len();
        if n == 0 {
            return;
        }
        if let Some(p) = &mut self.label_picker {
            p.ix = (p.ix as isize + delta).rem_euclid(n as isize) as usize;
        }
        cx.notify();
    }

    /// Enter: add (or, if all have it, remove) the highlighted label;
    /// `archive` (⇧⏎) also takes the messages out of the Inbox.
    pub fn label_accept(&mut self, archive: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(p) = &self.label_picker else { return };
        let rows = p.rows(cx);
        let Some(pick) = rows.get(p.ix.min(rows.len().saturating_sub(1))).cloned() else { return };
        let (label, create) = match pick {
            Pick::Label(l) => (l, false),
            Pick::Create(l) => (l, true),
        };
        let on = create || !p.has(&label);
        let (account, targets) = (p.account.clone(), p.targets.clone());
        self.close_label_picker(window, cx);
        self.apply_label(&account, targets, &label, on, create, cx);
        if archive && on && matches!(self.folder(), actions::Folder::Inbox) {
            self.bulk(BulkKind::Archive, cx);
        }
    }

    fn apply_label(&mut self, account: &str, targets: Vec<Envelope>, label: &str, on: bool, create: bool, cx: &mut Context<Self>) {
        let n = targets.len();
        let what = if on { format!("Labelled “{label}”") } else { format!("Removed “{label}” from") };
        self.status = format!("{what} {n} message{}", if n == 1 { "" } else { "s" }).into();
        self.status_is_error = false;
        cx.notify();
        let connect = self.connection_for(account);
        let (account, label) = (account.to_string(), label.to_string());
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let l = label.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let session = connect()?;
                    let mut s = session.lock().unwrap();
                    if create {
                        s.create_label(&l)?;
                    }
                    let mut by_box: HashMap<String, Vec<String>> = HashMap::new();
                    for e in &targets {
                        by_box.entry(e.mailbox.clone()).or_default().push(e.id.clone());
                    }
                    for (mailbox, ids) in by_box {
                        s.set_label(&mailbox, &ids, &l, on)?;
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        if *this.account == *account {
                            this.refresh_folders(cx);
                            // Taken off the label you're looking at: it leaves the list.
                            if !on && !this.unified && this.active().is_some_and(|m| m.name.eq_ignore_ascii_case(&label)) {
                                this.refresh_open_folder(cx);
                            }
                        }
                    }
                    Err(err) => {
                        this.status = format!("Could not change “{label}”: {err:#}").into();
                        this.status_is_error = true;
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn render_label_picker(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let p = self.label_picker.as_ref()?;
        let rows = p.rows(cx);
        let ix = p.ix.min(rows.len().saturating_sub(1));
        let n = p.targets.len();
        let mut list = div().id("label-rows").max_h(px(300.)).overflow_y_scroll().flex().flex_col().py_1();
        if rows.is_empty() {
            list = list.child(div().px_3().py_2().text_sm().text_color(rgb(MUTED)).child("No labels yet: type a name to create one"));
        }
        for (i, row) in rows.iter().enumerate() {
            let on = i == ix;
            let (text, mark, create) = match row {
                Pick::Label(l) => (display(l).to_string(), p.has(l), false),
                Pick::Create(l) => (format!("Create “{l}”"), false, true),
            };
            list = list.child(
                div()
                    .id(("label-row", i))
                    .mx_1p5()
                    .px_2()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(rgb(if create { ACCENT } else { TEXT }))
                    .when(on, |d| d.bg(rgb(SELECTED)))
                    .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER))))
                    .on_click(cx.listener(move |this, ev: &ClickEvent, w, cx| {
                        if let Some(p) = &mut this.label_picker {
                            p.ix = i;
                        }
                        this.label_accept(ev.modifiers().shift, w, cx)
                    }))
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .text_color(rgb(ACCENT))
                            .child(if mark { "✓" } else if create { "+" } else { "" }),
                    )
                    .child(div().flex_1().min_w_0().truncate().child(text)),
            );
        }
        let who = if self.accounts.len() > 1 {
            self.accounts.iter().find(|a| a.email == p.account).map(|a| format!(" · {}", a.short())).unwrap_or_default()
        } else {
            String::new()
        };
        Some(
            deferred(
                anchored().position(point(px(SIDEBAR_WIDTH + 12.), px(70.))).child(
                    div()
                        .id("label-picker")
                        .key_context("LabelPicker")
                        .w(px(LIST_WIDTH - 24.))
                        .rounded_lg()
                        .bg(rgb(0x24242b))
                        .border_1()
                        .border_color(rgb(0x3d3d47))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            div()
                                .px_3()
                                .pt_2()
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(format!("Label {n} message{}{who}", if n == 1 { "" } else { "s" })),
                        )
                        .child(div().p_2().child(Input::new(&p.input).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER))))
                        .child(list)
                        .child(
                            div()
                                .px_3()
                                .py_1p5()
                                .border_t_1()
                                .border_color(rgb(0x34343d))
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child(if p.current.is_none() { "Checking current labels…" } else { "⏎ add or remove · ⇧⏎ label and archive · esc close" }),
                        ),
                ),
            )
            .with_priority(2),
        )
    }

    // ----- Settings → Labels ----------------------------------------------------

    pub fn labels_pane_account(&self) -> String {
        self.labels_pane.as_ref().map(|l| l.account.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| self.account.to_string())
    }

    /// Show an account's labels, reading the list fresh from the server.
    pub fn load_labels_pane(&mut self, account: String, cx: &mut Context<Self>) {
        let known = self.labels_known(&account);
        self.labels_pane = Some(LabelsPane { account: account.clone(), labels: known, loading: true, ..Default::default() });
        cx.notify();
        let connect = self.connection_for(&account);
        let store = self.store.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let a = account.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let names = connect()?.lock().unwrap().mailboxes()?;
                    if let Some(store) = store {
                        store.lock().unwrap().save_mailboxes(&a, &names)?;
                    }
                    anyhow::Ok(names)
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(pane) = this.labels_pane.as_mut().filter(|p| p.account == account) {
                    pane.loading = false;
                    match result {
                        Ok(names) => pane.labels = user_labels(&names),
                        Err(err) => {
                            this.status = format!("Could not read {account}'s labels: {err:#}").into();
                            this.status_is_error = true;
                        }
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn label_admin(&mut self, account: String, op: LabelOp, cx: &mut Context<Self>) {
        if let Some(p) = &mut self.labels_pane {
            p.busy = true;
        }
        cx.notify();
        let connect = self.connection_for(&account);
        let op2 = op.clone();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let session = connect()?;
                    let mut s = session.lock().unwrap();
                    match &op2 {
                        LabelOp::Create(n) => s.create_label(n),
                        LabelOp::Rename(a, b) => s.rename_label(a, b),
                        LabelOp::Delete(n) => s.delete_label(n),
                    }
                })
                .await;
            this.update(cx, |this, cx| {
                if let Some(p) = &mut this.labels_pane {
                    p.busy = false;
                }
                match result {
                    Ok(()) => {
                        this.status = match &op {
                            LabelOp::Create(n) => format!("Created “{n}”"),
                            LabelOp::Rename(a, b) => format!("Renamed “{a}” to “{b}”"),
                            LabelOp::Delete(n) => format!("Deleted “{n}” (its messages are still in All Mail)"),
                        }
                        .into();
                        this.status_is_error = false;
                        // Sender rules follow a renamed label.
                        if let LabelOp::Rename(a, b) = &op {
                            this.rename_rule_destination(&account, a, b);
                        }
                        if *this.account == *account {
                            this.refresh_folders(cx);
                        }
                        this.load_labels_pane(account.clone(), cx);
                    }
                    Err(err) => {
                        this.status = format!("Gmail refused: {err:#}").into();
                        this.status_is_error = true;
                        // Show what's really there now, whatever happened.
                        if *this.account == *account {
                            this.refresh_folders(cx);
                        }
                        this.load_labels_pane(account.clone(), cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn rename_rule_destination(&mut self, account: &str, from: &str, to: &str) {
        let Some((mut rules, names)) = rules::load(account) else { return };
        let mut changed = false;
        for dest in rules.values_mut() {
            if dest.eq_ignore_ascii_case(from) {
                *dest = to.to_string();
                changed = true;
            } else if let Some(rest) = dest.strip_prefix(&format!("{from}/")) {
                *dest = format!("{to}/{rest}");
                changed = true;
            }
        }
        if changed && rules::save(account, &rules, &names).is_ok() && *self.account == *account {
            self.load_rules();
        }
    }

    /// Sender rules that file mail into `label` (they'd break if it went).
    fn rules_using(&self, account: &str, label: &str) -> usize {
        rules::load(account)
            .map(|(r, _)| r.values().filter(|d| d.eq_ignore_ascii_case(label) || d.starts_with(&format!("{label}/"))).count())
            .unwrap_or(0)
    }

    fn start_rename(&mut self, label: String, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(label.clone(), window, cx);
            s
        });
        input.update(cx, |s, cx| s.focus(window, cx));
        if let Some(p) = &mut self.labels_pane {
            p.renaming = Some((label, input));
            p.confirm_delete = None;
        }
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &mut self.labels_pane else { return };
        let Some((old, input)) = p.renaming.take() else { return };
        let new = input.read(cx).value().trim().to_string();
        let account = p.account.clone();
        if new.is_empty() || new == old {
            cx.notify();
            return;
        }
        if p.labels.iter().any(|l| l.eq_ignore_ascii_case(&new)) {
            self.status = format!("There's already a label called “{new}”").into();
            self.status_is_error = true;
            cx.notify();
            return;
        }
        self.label_admin(account, LabelOp::Rename(old, new), cx);
    }

    fn start_new_label(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Label name (use / to nest: Clients/Acme)"));
        input.update(cx, |s, cx| s.focus(window, cx));
        if let Some(p) = &mut self.labels_pane {
            p.new_input = Some(input);
            p.renaming = None;
        }
        cx.notify();
    }

    fn finish_new_label(&mut self, cx: &mut Context<Self>) {
        let Some(p) = &mut self.labels_pane else { return };
        let Some(input) = p.new_input.take() else { return };
        let name = input.read(cx).value().trim().trim_matches('/').to_string();
        let account = p.account.clone();
        if name.is_empty() {
            cx.notify();
            return;
        }
        if p.labels.iter().any(|l| l.eq_ignore_ascii_case(&name)) {
            self.status = format!("“{name}” already exists").into();
            self.status_is_error = true;
            cx.notify();
            return;
        }
        self.label_admin(account, LabelOp::Create(name), cx);
    }

    fn delete_label(&mut self, label: String, cx: &mut Context<Self>) {
        let Some(p) = &mut self.labels_pane else { return };
        let account = p.account.clone();
        if p.confirm_delete.as_deref() != Some(label.as_str()) {
            p.confirm_delete = Some(label);
            cx.notify();
            return;
        }
        p.confirm_delete = None;
        let used = self.rules_using(&account, &label);
        if used > 0 {
            self.status =
                format!("{used} sender rule{} file mail into “{label}”; change them first (accounts/{account}/rules.toml)", if used == 1 { "" } else { "s" }).into();
            self.status_is_error = true;
            cx.notify();
            return;
        }
        self.label_admin(account, LabelOp::Delete(label), cx);
    }

    pub fn render_labels_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let account = self.labels_pane_account();
        let pane = self.labels_pane.as_ref();
        let labels: Vec<String> = pane.map(|p| p.labels.clone()).unwrap_or_default();
        let mut accounts = div().flex().flex_wrap().gap_1p5();
        for a in &self.accounts {
            let on = a.email == account;
            let email = a.email.clone();
            accounts = accounts.child(
                crate::settings::choice(SharedString::from(format!("lbl-acct-{}", a.email)), a.short(), on)
                    .on_click(cx.listener(move |this, _, _, cx| this.load_labels_pane(email.clone(), cx))),
            );
        }
        let mut list = div().flex().flex_col().border_1().border_color(rgb(BORDER)).rounded_md();
        let renaming = pane.and_then(|p| p.renaming.as_ref());
        let confirm = pane.and_then(|p| p.confirm_delete.clone());
        for (i, label) in labels.iter().enumerate() {
            let depth = display(label).matches('/').count();
            let protected = sweep_label(label);
            let editing = renaming.filter(|(old, _)| old == label);
            let (l1, l2) = (label.clone(), label.clone());
            let row = div()
                .id(("label-admin", i))
                .px_3()
                .h(px(40.))
                .flex()
                .items_center()
                .gap_3()
                .when(i > 0, |d| d.border_t_1().border_color(rgb(BORDER)))
                .child(div().w(px(16. * depth as f32)).flex_none())
                .child(svg().path("icons/tag.svg").size(px(14.)).flex_none().text_color(rgb(MUTED)));
            let row = match editing {
                Some((_, input)) => row
                    .child(
                        div()
                            .flex_1()
                            .key_context("LabelRename")
                            .on_action(cx.listener(|this, _: &LabelAccept, _, cx| this.finish_rename(cx)))
                            .child(Input::new(input).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER))),
                    )
                    .child(action_button("rename-save", "Save", true).on_click(cx.listener(|this, _, _, cx| this.finish_rename(cx))))
                    .child(action_button("rename-cancel", "Cancel", false).on_click(cx.listener(|this, _, _, cx| {
                        if let Some(p) = &mut this.labels_pane {
                            p.renaming = None;
                        }
                        cx.notify()
                    }))),
                None => row
                    .child(div().flex_1().min_w_0().truncate().text_sm().text_color(rgb(TEXT)).child(display(label).to_string()))
                    .when(protected, |d| d.child(div().text_xs().text_color(rgb(MUTED)).child("Used by the Sweep")))
                    .when(!protected, |d| {
                        d.child(
                            action_button(SharedString::from(format!("rename-{i}")), "Rename", false)
                                .on_click(cx.listener(move |this, _, w, cx| this.start_rename(l1.clone(), w, cx))),
                        )
                        .child(
                            action_button(
                                SharedString::from(format!("delete-{i}")),
                                if confirm.as_deref() == Some(label.as_str()) { "Click again to delete" } else { "Delete" },
                                false,
                            )
                            .on_click(cx.listener(move |this, _, _, cx| this.delete_label(l2.clone(), cx))),
                        )
                    }),
            };
            list = list.child(row);
        }
        if labels.is_empty() {
            list = list.child(div().px_3().py_3().text_sm().text_color(rgb(MUTED)).child(
                if pane.is_some_and(|p| p.loading) { "Reading labels…" } else { "No labels yet." },
            ));
        }
        let new_row = match pane.and_then(|p| p.new_input.as_ref()) {
            Some(input) => div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .w(px(360.))
                        .key_context("LabelRename")
                        .on_action(cx.listener(|this, _: &LabelAccept, _, cx| this.finish_new_label(cx)))
                        .child(Input::new(input).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER))),
                )
                .child(action_button("label-create", "Create", true).on_click(cx.listener(|this, _, _, cx| this.finish_new_label(cx))))
                .into_any_element(),
            None => div()
                .flex()
                .child(action_button("label-new", "New label", true).on_click(cx.listener(|this, _, w, cx| this.start_new_label(w, cx))))
                .into_any_element(),
        };
        div()
            .id("labels-pane")
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
                        "Each account's Gmail labels. Renaming a label renames the ones nested under it and updates sender rules; deleting one keeps its messages in All Mail. Press l on a message to label it.",
                    ))
                    .when(self.accounts.len() > 1, |d| d.child(accounts))
                    .child(new_row)
                    .child(list)
                    .when(pane.is_some_and(|p| p.busy), |d| d.child(div().text_xs().text_color(rgb(MUTED)).child("Working…"))),
            )
    }
}

#[derive(Clone)]
enum LabelOp {
    Create(String),
    Rename(String, String),
    Delete(String),
}

#[cfg(test)]
mod tests {
    use super::user_labels;

    #[test]
    fn only_your_labels() {
        let names: Vec<String> =
            ["INBOX", "[Gmail]/All Mail", "[Gmail]/Archive", "Work", "Clients/Acme", "Reading", "[Notion]"].iter().map(|s| s.to_string()).collect();
        assert_eq!(user_labels(&names), vec!["[Notion]", "[Gmail]/Archive", "Clients/Acme", "Reading", "Work"]);
    }
}
