//! App defaults and the settings screen (⌘,): how new messages are
//! written, and the signature designer.

use super::*;
use crate::identity::Identity;
use crate::signature::{Layout, Signature, ACCENTS, FONTS};

#[derive(Clone, Debug)]
pub struct Settings {
    /// New messages use a monospaced font (each message can switch).
    pub mono: bool,
    /// Writing takes the message list's space.
    pub hide_list_when_writing: bool,
    /// The Blocks pane is open (else a thin ribbon) while writing.
    pub blocks_open: bool,
    /// Signature for new messages and forwards.
    pub sig_new: Option<i64>,
    /// Signature for replies.
    pub sig_reply: Option<i64>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { mono: true, hide_list_when_writing: true, blocks_open: false, sig_new: None, sig_reply: None }
    }
}

impl Settings {
    /// From settings.toml (defaults if there's none yet).
    pub fn load() -> Self {
        files::load_settings().unwrap_or_default()
    }

    /// The first version kept settings in the cache database.
    pub fn load_db(store: &Store) -> Self {
        let d = Settings::default();
        let flag = |k: &str, default: bool| store.setting(k).map(|v| v == "1").unwrap_or(default);
        let id = |k: &str| store.setting(k).and_then(|v| v.parse().ok());
        Settings {
            mono: flag("compose.mono", d.mono),
            hide_list_when_writing: flag("compose.hide_list", d.hide_list_when_writing),
            blocks_open: d.blocks_open,
            sig_new: id("signature.new"),
            sig_reply: id("signature.reply"),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        files::save_settings(self)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Accounts,
    Writing,
    Labels,
    Problems,
    Addresses,
    Signatures,
}

pub struct SettingsView {
    pub pane: Pane,
    pub focus: FocusHandle,
    pub editor: Option<SigEditor>,
    /// Settings → Accounts: the account being edited, and one waiting for
    /// a second click to be removed.
    pub account: Option<crate::account_view::AccountEditor>,
    pub confirm_remove: Option<String>,
    /// Settings → Signatures shows this account's signatures.
    pub sig_account: String,
    /// The From address being edited: (identity id, name input, email input).
    pub address: Option<AddressEditor>,
}

pub struct AddressEditor {
    pub id: i64,
    pub name: Entity<InputState>,
    pub email: Entity<InputState>,
    _subs: Vec<Subscription>,
}

/// The signature being designed: its fields as inputs.
pub struct SigEditor {
    pub id: i64,
    pub inputs: Vec<(&'static str, Entity<InputState>)>,
    _subs: Vec<Subscription>,
}

const FIELDS: [(&str, &str); 12] = [
    ("Signature name", "e.g. Work, Personal"),
    ("Name", "Your name"),
    ("Title", "Job title"),
    ("Company", "Company"),
    ("Phone", "(555) 555-0100"),
    ("Email", "you@example.com"),
    ("Website", "example.com"),
    ("Note", "A line under the details"),
    ("Logo links to", "example.com (optional)"),
    ("Second logo links to", "example.com (optional)"),
    ("Follow us on", "LinkedIn"),
    ("Follow link", "linkedin.com/company/… (shows the line)"),
];

fn field_value(s: &Signature, i: usize) -> &str {
    match i {
        0 => &s.label,
        1 => &s.name,
        2 => &s.title,
        3 => &s.company,
        4 => &s.phone,
        5 => &s.email,
        6 => &s.website,
        7 => &s.note,
        8 => &s.logo_link,
        9 => &s.logo2_link,
        10 => &s.social_label,
        _ => &s.social_url,
    }
}

fn set_field(s: &mut Signature, i: usize, v: String) {
    match i {
        0 => s.label = v,
        1 => s.name = v,
        2 => s.title = v,
        3 => s.company = v,
        4 => s.phone = v,
        5 => s.email = v,
        6 => s.website = v,
        7 => s.note = v,
        8 => s.logo_link = v,
        9 => s.logo2_link = v,
        10 => s.social_label = v,
        _ => s.social_url = v,
    }
}

const SAMPLE_BODY: &str = "Hi Jim,\n\nThis is how your signature will look under a message.\n\nThanks,";

impl MailApp {
    pub fn signature_by_id(&self, id: Option<i64>) -> Option<&Signature> {
        let id = id?;
        self.signatures.iter().find(|s| s.id == id)
    }

    /// Defaults for a freshly opened composer.
    pub fn with_defaults(&self, mut d: Draft) -> Draft {
        d.mono = self.settings.mono;
        if d.account.is_empty() {
            d.account = self.account.to_string();
        }
        let who = self.identity_in(&d.account, d.from.as_ref());
        d.signature = self.default_signature(&who, d.kind);
        d
    }

    /// The open account's address first, then its extra identities.
    pub fn all_identities(&self) -> Vec<Identity> {
        self.identities_for(&self.account.clone())
    }

    /// An account's own address first, then the extra From addresses that
    /// send through it.
    pub fn identities_for(&self, account: &str) -> Vec<Identity> {
        let a = self.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(account));
        let main = Identity {
            id: 0,
            name: a.map(|a| a.name.clone()).unwrap_or_default(),
            email: account.to_string(),
            sig_new: a.and_then(|a| a.sig_new),
            sig_reply: a.and_then(|a| a.sig_reply),
            account: account.to_string(),
        };
        std::iter::once(main).chain(self.account_identities_for(account)).collect()
    }

    pub fn account_identities(&self) -> Vec<Identity> {
        self.account_identities_for(&self.account.clone())
    }

    /// Extra From addresses that send through an account (older files
    /// without an account belong to the first one).
    pub fn account_identities_for(&self, account: &str) -> Vec<Identity> {
        let first = self.accounts.first().map(|a| a.email.as_str()).unwrap_or("");
        self.identities
            .iter()
            .filter(|i| {
                let owner = if i.account.is_empty() { first } else { i.account.as_str() };
                owner.eq_ignore_ascii_case(account)
            })
            .map(|i| Identity { account: account.to_string(), ..i.clone() })
            .collect()
    }

    /// The identity a From value belongs to (the main one for None).
    pub fn identity_in(&self, account: &str, from: Option<&(String, String)>) -> Identity {
        let all = self.identities_for(account);
        from.and_then(|(_, e)| all.iter().find(|i| i.email.eq_ignore_ascii_case(e)).cloned())
            .unwrap_or_else(|| all[0].clone())
    }

    pub fn default_signature(&self, who: &Identity, kind: Kind) -> Option<i64> {
        let sig = match kind {
            Kind::Reply | Kind::ReplyAll => who.sig_reply,
            _ => who.sig_new,
        };
        let account = if who.account.is_empty() { self.account.to_string() } else { who.account.clone() };
        sig.filter(|id| self.signatures.iter().any(|s| s.id == *id && s.account.eq_ignore_ascii_case(&account)))
    }

    fn save_settings(&mut self, cx: &mut Context<Self>) {
        // Default signatures belong to the open account.
        self.save_account_signatures();
        if let Err(e) = self.settings.save() {
            self.status = format!("Could not save settings: {e:#}").into();
            self.status_is_error = true;
        }
        cx.notify();
    }

    pub fn open_settings(&mut self, pane: Pane, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() && pane == Pane::Writing {
            // Keep it simple: settings replace the reading panes only.
        }
        self.sweep = None;
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let sig_account = self.account.to_string();
        self.settings_view =
            Some(SettingsView { pane, focus, editor: None, address: None, account: None, confirm_remove: None, sig_account });
        if pane == Pane::Signatures {
            if let Some(id) = self.first_signature_shown() {
                self.edit_signature(id, window, cx);
            }
        }
        if pane == Pane::Labels {
            self.load_labels_pane(self.account.to_string(), cx);
        }
        if pane == Pane::Problems {
            self.problems = crate::errlog::load();
            self.problems_unseen = 0;
        }
        cx.notify();
    }

    pub fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_view.take().is_some() {
            match &self.composer {
                Some(c) => c.body.read(cx).focus_handle(cx).focus(window, cx),
                None => self.focus.focus(window, cx),
            }
            cx.notify();
        }
    }

    fn set_pane(&mut self, pane: Pane, window: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = &mut self.settings_view else { return };
        v.pane = pane;
        if pane == Pane::Labels {
            self.load_labels_pane(self.account.to_string(), cx);
        }
        if pane == Pane::Problems {
            self.problems = crate::errlog::load();
            self.problems_unseen = 0;
        }
        let Some(v) = &mut self.settings_view else { return };
        if pane == Pane::Signatures && v.editor.is_none() {
            if let Some(id) = self.first_signature_shown() {
                self.edit_signature(id, window, cx);
            }
        }
        cx.notify();
    }

    pub fn edit_signature(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sig) = self.signatures.iter().find(|s| s.id == id).cloned() else { return };
        let mut inputs = Vec::new();
        let mut subs = Vec::new();
        for (i, (label, placeholder)) in FIELDS.iter().enumerate() {
            let value = field_value(&sig, i).to_string();
            let input = cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(*placeholder);
                s.set_value(value, window, cx);
                s
            });
            subs.push(cx.subscribe(&input, move |this, state, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = state.read(cx).value().to_string();
                    this.update_signature(id, cx, |s| set_field(s, i, v));
                }
            }));
            inputs.push((*label, input));
        }
        if let Some(v) = &mut self.settings_view {
            v.pane = Pane::Signatures;
            v.editor = Some(SigEditor { id, inputs, _subs: subs });
        }
        cx.notify();
    }

    /// Change a signature and save it.
    fn update_signature(&mut self, id: i64, cx: &mut Context<Self>, f: impl FnOnce(&mut Signature)) {
        let Some(sig) = self.signatures.iter_mut().find(|s| s.id == id) else { return };
        f(sig);
        if let Err(e) = files::save_signature(sig) {
            self.status = format!("Could not save the signature: {e:#}").into();
            self.status_is_error = true;
        }
        cx.notify();
    }

    pub fn new_signature(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let account = self.sig_account();
        let mine = self.signatures_for(&account);
        let n = mine.len() + 1;
        let mut sig = Signature::new(if n == 1 { "Work" } else { "New signature" });
        if let Some(a) = self.accounts.iter().find(|a| a.email == account) {
            sig.name = a.name.clone();
            sig.email = a.email.clone();
        }
        // Start from the account's last one, so a variant is quick to make.
        if let Some(last) = mine.last() {
            sig = Signature { id: 0, label: format!("{} copy", last.label), ..(*last).clone() };
        }
        sig.account = account.clone();
        drop(mine);
        match files::save_signature(&sig) {
            Ok(id) => sig.id = id,
            Err(e) => {
                self.status = format!("Could not create a signature: {e:#}").into();
                self.status_is_error = true;
                return;
            }
        }
        let id = sig.id;
        self.signatures.push(sig);
        if n == 1 {
            // An account's first signature becomes its default.
            self.set_sig_default(&account, true, Some(id));
            self.set_sig_default(&account, false, Some(id));
        }
        self.edit_signature(id, window, cx);
    }

    fn delete_signature(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        files::delete_signature(id).ok();
        self.signatures.retain(|s| s.id != id);
        for a in self.accounts.iter().map(|a| a.email.clone()).collect::<Vec<_>>() {
            let (n, r) = self.sig_defaults(&a);
            if n == Some(id) {
                self.set_sig_default(&a, true, None);
            }
            if r == Some(id) {
                self.set_sig_default(&a, false, None);
            }
        }
        if let Some(c) = &mut self.composer {
            if c.signature == Some(id) {
                c.signature = None;
            }
        }
        if let Some(v) = &mut self.settings_view {
            v.editor = None;
        }
        if let Some(first) = self.first_signature_shown() {
            self.edit_signature(first, window, cx);
        }
        cx.notify();
    }

    // ----- Signatures per account ---------------------------------------------

    pub fn signatures_for(&self, account: &str) -> Vec<&Signature> {
        self.signatures.iter().filter(|s| s.account.eq_ignore_ascii_case(account)).collect()
    }

    /// The account Settings → Signatures is showing.
    pub fn sig_account(&self) -> String {
        self.settings_view
            .as_ref()
            .map(|v| v.sig_account.clone())
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| self.account.to_string())
    }

    fn first_signature_shown(&self) -> Option<i64> {
        self.signatures_for(&self.sig_account()).first().map(|s| s.id)
    }

    pub fn sig_defaults(&self, account: &str) -> (Option<i64>, Option<i64>) {
        self.accounts.iter().find(|a| a.email == account).map(|a| (a.sig_new, a.sig_reply)).unwrap_or((None, None))
    }

    /// Set an account's default signature for new messages (or replies).
    pub fn set_sig_default(&mut self, account: &str, new: bool, id: Option<i64>) {
        let Some(a) = self.accounts.iter_mut().find(|a| a.email == account) else { return };
        if new { a.sig_new = id } else { a.sig_reply = id }
        let a = a.clone();
        if let Err(e) = crate::accounts::save(&a) {
            self.status = format!("Could not save the account: {e:#}").into();
            self.status_is_error = true;
        }
        if *a.email == *self.account {
            self.settings.sig_new = a.sig_new;
            self.settings.sig_reply = a.sig_reply;
        }
    }

    pub fn show_signatures_of(&mut self, account: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(v) = &mut self.settings_view {
            v.sig_account = account;
            v.editor = None;
        }
        if let Some(id) = self.first_signature_shown() {
            self.edit_signature(id, window, cx);
        }
        cx.notify();
    }

    /// Signatures made before they belonged to accounts go to the account
    /// with their email address (else the first); an account that was a From
    /// address before takes that address's default signatures.
    pub fn assign_signatures(&mut self) {
        let first = self.accounts.first().map(|a| a.email.clone()).unwrap_or_default();
        if first.is_empty() {
            return;
        }
        let emails: Vec<String> = self.accounts.iter().map(|a| a.email.clone()).collect();
        for s in self.signatures.iter_mut().filter(|s| s.account.is_empty()) {
            let e = s.email.trim().to_lowercase();
            s.account = if emails.contains(&e) { e } else { first.clone() };
            files::save_signature(s).ok();
        }
        for a in self.accounts.clone() {
            if a.sig_new.is_some() || a.sig_reply.is_some() {
                continue;
            }
            let Some(was) = self.identities.iter().find(|i| i.email.eq_ignore_ascii_case(&a.email)).cloned() else { continue };
            let ok = |id: Option<i64>| id.filter(|id| self.signatures.iter().any(|s| s.id == *id && s.account == a.email));
            let (n, r) = (ok(was.sig_new), ok(was.sig_reply));
            if n.is_some() || r.is_some() {
                self.set_sig_default(&a.email, true, n);
                self.set_sig_default(&a.email, false, r);
            }
        }
    }

    fn choose_logo(&mut self, id: i64, second: bool, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Use as logo".into()),
        });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let Some(path) = paths.into_iter().next() else { return };
            let result = match signature::image_mime(&path) {
                None => Err("Use a PNG, JPEG or GIF image".to_string()),
                Some(mime) => match std::fs::read(&path) {
                    Ok(bytes) if bytes.len() > 512 * 1024 => Err("That image is over 512 KB; use a smaller one".into()),
                    Ok(bytes) => Ok((mime.to_string(), bytes)),
                    Err(e) => Err(format!("Could not read it: {e}")),
                },
            };
            this.update(cx, |this, cx| match result {
                Ok(logo) => this.update_signature(id, cx, |s| {
                    if second {
                        s.logo2 = Some(logo);
                        s.layout = Layout::TwoLogos;
                    } else {
                        s.logo = Some(logo);
                        if s.layout == Layout::Stacked {
                            s.layout = Layout::LogoLeft;
                        }
                    }
                }),
                Err(msg) => {
                    this.status = msg.into();
                    this.status_is_error = true;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The web view shows the signature under a sample message.
    pub fn settings_html(&self) -> Option<((String, String), String)> {
        let v = self.settings_view.as_ref().filter(|v| v.pane == Pane::Signatures)?;
        let sig = self.signature_by_id(v.editor.as_ref().map(|e| e.id))?;
        let mut d = Draft::new();
        d.body = SAMPLE_BODY.into();
        d.mono = self.settings.mono;
        let (_, body) = compose::render_bodies(&d, None, true);
        let html = format!("{body}{}", sig.html(md::font(d.mono), true));
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        html.hash(&mut h);
        Some((("signature".into(), h.finish().to_string()), html))
    }
}

// ----- Rendering -----------------------------------------------------------

/// One option in a row of choices; the chosen one is filled.
pub fn choice(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Stateful<Div> {
    div()
        .id(id.into())
        .h(px(28.))
        .px_3()
        .flex()
        .items_center()
        .rounded_md()
        .text_sm()
        .cursor_pointer()
        .border_1()
        .when(on, |d| d.bg(rgb(SELECTED)).border_color(rgb(ACCENT)).text_color(rgb(TEXT)))
        .when(!on, |d| d.border_color(rgb(BORDER)).text_color(rgb(SUBTLE)).hover(|d| d.bg(rgb(HOVER)).text_color(rgb(TEXT))))
        .child(label.into())
}

/// A setting: what it is on the left, the control on the right.
pub fn setting_row(title: &'static str, about: &'static str, control: impl IntoElement) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_8()
        .py_4()
        .border_b_1()
        .border_color(rgb(BORDER))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .min_w_0()
                .child(div().text_sm().text_color(rgb(TEXT)).child(title))
                .child(div().text_xs().text_color(rgb(MUTED)).child(about)),
        )
        .child(div().flex().flex_none().gap_1p5().child(control))
}

/// Small drawing of a layout, for the layout picker.
fn layout_thumb(layout: Layout, accent: u32) -> Div {
    let bar = |w: f32, strong: bool| div().h(px(4.)).w(px(w)).rounded_sm().bg(rgb(if strong { 0x9a9aa6 } else { 0x55555f }));
    let lines = div().flex().flex_col().gap(px(3.)).child(bar(34., true)).child(bar(26., false)).child(bar(30., false));
    let logo = div().size(px(16.)).rounded_sm().bg(rgb(accent));
    match layout {
        Layout::Stacked => div().flex().flex_col().gap(px(4.)).child(div().h(px(2.)).w(px(40.)).bg(rgb(accent))).child(lines),
        Layout::LogoLeft => div()
            .flex()
            .items_center()
            .gap(px(4.))
            .child(logo)
            .child(div().w(px(2.)).h(px(20.)).bg(rgb(accent)))
            .child(lines),
        Layout::LogoTop => div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(logo)
            .child(div().h(px(2.)).w(px(40.)).bg(rgb(accent)))
            .child(lines),
        Layout::TwoLogos => {
            let col = |a: f32, b: f32| div().flex().flex_col().gap(px(3.)).child(bar(a, true)).child(bar(b, false));
            let rule = || div().w(px(1.)).h(px(14.)).bg(rgb(0x6a6a74));
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(div().flex().items_center().gap(px(4.)).child(col(24., 18.)).child(rule()).child(col(22., 26.)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(div().w(px(24.)).h(px(8.)).rounded_sm().bg(rgb(accent)))
                        .child(rule())
                        .child(div().w(px(22.)).h(px(8.)).rounded_sm().bg(rgb(0x9a9aa6))),
                )
        }
    }
}

impl MailApp {
    pub fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(v) = &self.settings_view else { return div() };
        let tab = |id: &'static str, label: &'static str, pane: Pane| {
            let on = v.pane == pane;
            div()
                .id(id)
                .h(px(30.))
                .px_3()
                .flex()
                .items_center()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(on, |d| d.bg(rgb(SELECTED)).text_color(rgb(TEXT)))
                .when(!on, |d| d.text_color(rgb(SUBTLE)).hover(|d| d.bg(rgb(HOVER))))
                .on_click(cx.listener(move |this, _, w, cx| this.set_pane(pane, w, cx)))
                .child(label)
        };
        let header = div()
            .flex_none()
            .h(px(52.))
            .px_6()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).mr_4().child("Settings"))
            .child(tab("pane-accounts", "Accounts", Pane::Accounts))
            .child(tab("pane-writing", "Writing", Pane::Writing))
            .child(tab("pane-addresses", "From addresses", Pane::Addresses))
            .child(tab("pane-labels", "Labels", Pane::Labels))
            .child(tab("pane-problems", "Problems", Pane::Problems))
            .child(tab("pane-signatures", "Signatures", Pane::Signatures))
            .child(div().flex_1())
            .child(
                action_button("settings-done", "Done  esc", false)
                    .on_click(cx.listener(|this, _, w, cx| this.close_settings(w, cx))),
            );
        let body = match v.pane {
            Pane::Accounts => self.render_accounts_pane(cx).into_any_element(),
            Pane::Writing => self.render_writing_pane(cx).into_any_element(),
            Pane::Labels => self.render_labels_pane(cx).into_any_element(),
            Pane::Problems => self.render_problems_pane(cx).into_any_element(),
            Pane::Signatures => self.render_signatures_pane(cx).into_any_element(),
            Pane::Addresses => self.render_addresses_pane(cx).into_any_element(),
        };
        div()
            .track_focus(&v.focus)
            .key_context("Settings")
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .child(header)
            .child(div().flex_1().min_h_0().child(body))
    }

    fn render_writing_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let s = &self.settings;
        let font = div()
            .flex()
            .gap_1p5()
            .child(
                choice("font-mono", "Monospaced", s.mono)
                    .font_family("Menlo")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.settings.mono = true;
                        this.save_settings(cx)
                    })),
            )
            .child(choice("font-sans", "Proportional", !s.mono).on_click(cx.listener(|this, _, _, cx| {
                this.settings.mono = false;
                this.save_settings(cx)
            })));
        let hide = div()
            .flex()
            .gap_1p5()
            .child(choice("hide-on", "Hide it", s.hide_list_when_writing).on_click(cx.listener(|this, _, _, cx| {
                this.settings.hide_list_when_writing = true;
                this.save_settings(cx)
            })))
            .child(choice("hide-off", "Keep it", !s.hide_list_when_writing).on_click(cx.listener(|this, _, _, cx| {
                this.settings.hide_list_when_writing = false;
                this.save_settings(cx)
            })));
        div()
            .id("writing-pane")
            .size_full()
            .overflow_y_scroll()
            .px_8()
            .py_4()
            .child(
                div()
                    .max_w(px(760.))
                    .flex()
                    .flex_col()
                    .child(setting_row("Font", "Used while writing and in the message you send. Each message can switch.", font))
                    .child(setting_row("Message list while writing", "Hiding it gives the message more room.", hide))
                    .child(div().pt_4().text_sm().text_color(rgb(MUTED)).child(
                        "Signatures belong to accounts: design them and pick each account's defaults under Signatures.",
                    )),
            )
    }

    fn render_signatures_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let v = self.settings_view.as_ref();
        let editing = v.and_then(|v| v.editor.as_ref());
        let current = editing.map(|e| e.id);
        // Left: the list.
        let mut list = div().w(px(200.)).flex_none().h_full().p_3().flex().flex_col().gap_1().border_r_1().border_color(rgb(BORDER));
        let shown = self.sig_account();
        if self.accounts.len() > 1 {
            let mut pick = div().flex().flex_col().gap_1().pb_2().mb_2().border_b_1().border_color(rgb(BORDER));
            pick = pick.child(div().px_1().pb_1().text_xs().text_color(rgb(MUTED)).child("Account"));
            for a in &self.accounts {
                let on = a.email == shown;
                let email = a.email.clone();
                pick = pick.child(
                    div()
                        .id(SharedString::from(format!("sig-acct-{}", a.email)))
                        .px_3()
                        .py_1()
                        .rounded_md()
                        .text_sm()
                        .truncate()
                        .cursor_pointer()
                        .text_color(rgb(if on { TEXT } else { SUBTLE }))
                        .when(on, |d| d.bg(rgb(SELECTED)))
                        .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER))))
                        .on_click(cx.listener(move |this, _, w, cx| this.show_signatures_of(email.clone(), w, cx)))
                        .child(a.short()),
                );
            }
            list = list.child(pick);
        }
        let (def_new, def_reply) = self.sig_defaults(&shown);
        for sig in self.signatures_for(&shown) {
            let id = sig.id;
            let on = current == Some(id);
            let uses: Vec<&str> = [(def_new == Some(id), "new"), (def_reply == Some(id), "replies")]
                .into_iter()
                .filter(|(b, _)| *b)
                .map(|(_, l)| l)
                .collect();
            list = list.child(
                div()
                    .id(("sig", id as u64))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(on, |d| d.bg(rgb(SELECTED)))
                    .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER))))
                    .on_click(cx.listener(move |this, _, w, cx| this.edit_signature(id, w, cx)))
                    .child(div().size(px(8.)).rounded_full().flex_none().bg(rgb(sig.accent)))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(div().text_sm().truncate().text_color(rgb(TEXT)).child(sig.label.clone()))
                            .when(!uses.is_empty(), |d| {
                                d.child(div().text_xs().text_color(rgb(MUTED)).child(format!("Default for {}", uses.join(" and "))))
                            }),
                    ),
            );
        }
        list = list.child(
            div()
                .id("sig-new")
                .mt_1()
                .px_3()
                .py_2()
                .rounded_md()
                .cursor_pointer()
                .text_sm()
                .text_color(rgb(ACCENT))
                .hover(|d| d.bg(rgb(HOVER)))
                .on_click(cx.listener(|this, _, w, cx| this.new_signature(w, cx)))
                .child("+ New signature"),
        );
        let Some((editor, sig)) = editing.zip(self.signature_by_id(current)) else {
            return div().size_full().flex().child(list).child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(rgb(MUTED))
                    .child("Make a signature with “+ New signature”."),
            );
        };
        let id = sig.id;
        let sig_acct = sig.account.clone();
        let label = |text: &'static str| div().text_xs().text_color(rgb(MUTED)).font_weight(FontWeight::MEDIUM).child(text);
        let input = |i: usize| {
            Input::new(&editor.inputs[i].1).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER)).text_color(rgb(0xffffff))
        };
        let field = |i: usize| div().flex().flex_col().gap_1().child(label(editor.inputs[i].0)).child(input(i));
        // Layout cards.
        let mut layouts = div().flex().flex_wrap().gap_2();
        for l in Layout::ALL {
            let on = sig.layout == l;
            layouts = layouts.child(
                div()
                    .id(SharedString::from(format!("layout-{}", l.as_str())))
                    .w(px(160.))
                    .p_2()
                    .rounded_md()
                    .border_1()
                    .cursor_pointer()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(on, |d| d.border_color(rgb(ACCENT)).bg(rgb(SELECTED)))
                    .when(!on, |d| d.border_color(rgb(BORDER)).hover(|d| d.bg(rgb(HOVER))))
                    .on_click(cx.listener(move |this, _, _, cx| this.update_signature(id, cx, |s| s.layout = l)))
                    .child(div().h(px(44.)).flex().items_center().justify_center().child(layout_thumb(l, sig.accent)))
                    .child(div().text_xs().text_center().text_color(rgb(if on { TEXT } else { SUBTLE })).child(l.label())),
            );
        }
        let mut swatches = div().flex().gap_2();
        for c in ACCENTS {
            let on = sig.accent == c;
            swatches = swatches.child(
                div()
                    .id(("accent", c as u64))
                    .size(px(22.))
                    .rounded_full()
                    .cursor_pointer()
                    .bg(rgb(c))
                    .border_2()
                    .border_color(rgb(if on { 0xffffff } else { BG }))
                    .on_click(cx.listener(move |this, _, _, cx| this.update_signature(id, cx, |s| s.accent = c))),
            );
        }
        let sizes = div().flex().gap_1p5().children([("Small", 56u32), ("Medium", 72), ("Large", 96)].into_iter().map(|(name, w)| {
            choice(SharedString::from(format!("logo-{w}")), name, sig.logo_width == w)
                .on_click(cx.listener(move |this, _, _, cx| this.update_signature(id, cx, |s| s.logo_width = w)))
        }));
        let logo_controls = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                action_button("logo-choose", if sig.logo.is_some() { "Replace image…" } else { "Choose image…" }, false)
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_logo(id, false, cx))),
            )
            .when(sig.logo.is_some(), |d| {
                d.child(
                    action_button("logo-remove", "Remove", false)
                        .on_click(cx.listener(move |this, _, _, cx| this.update_signature(id, cx, |s| s.logo = None))),
                )
            });
        let two = sig.layout == Layout::TwoLogos;
        let logo2_controls = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                action_button("logo2-choose", if sig.logo2.is_some() { "Replace second…" } else { "Choose second image…" }, false)
                    .on_click(cx.listener(move |this, _, _, cx| this.choose_logo(id, true, cx))),
            )
            .when(sig.logo2.is_some(), |d| {
                d.child(
                    action_button("logo2-remove", "Remove", false)
                        .on_click(cx.listener(move |this, _, _, cx| this.update_signature(id, cx, |s| s.logo2 = None))),
                )
            });
        let fonts = div().flex().flex_wrap().gap_1p5().children(FONTS.iter().enumerate().map(|(i, (name, stack))| {
            let stack = stack.to_string();
            choice(("sig-font", i), *name, sig.font == stack)
                .when(!stack.is_empty(), |d| d.font_family(name.to_string()))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let stack = stack.clone();
                    this.update_signature(id, cx, |s| s.font = stack)
                }))
        }));
        let defaults = div()
            .flex()
            .gap_1p5()
            .child(choice("def-new", "New messages", def_new == Some(id)).on_click(cx.listener({
                let acct = sig_acct.clone();
                move |this, _, _, cx| {
                    let on = this.sig_defaults(&acct).0 == Some(id);
                    this.set_sig_default(&acct, true, if on { None } else { Some(id) });
                    cx.notify()
                }
            })))
            .child(choice("def-reply", "Replies", def_reply == Some(id)).on_click(cx.listener({
                let acct = sig_acct.clone();
                move |this, _, _, cx| {
                    let on = this.sig_defaults(&acct).1 == Some(id);
                    this.set_sig_default(&acct, false, if on { None } else { Some(id) });
                    cx.notify()
                }
            })));
        let section = |title: &'static str| div().pt_3().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(rgb(TEXT)).child(title);
        let form = div()
            .id("sig-form")
            .w(px(380.))
            .flex_none()
            .h_full()
            .overflow_y_scroll()
            .px_5()
            .py_4()
            .flex()
            .flex_col()
            .gap_3()
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(field(0))
            .child(section("Layout"))
            .child(layouts)
            .child(section("Details"))
            .child(field(1))
            .child(div().flex().gap_2().child(div().flex_1().child(field(2))).child(div().flex_1().child(field(3))))
            .child(field(4))
            .child(field(5))
            .child(field(6))
            .child(field(7))
            .child(section("Font"))
            .child(fonts)
            .child(section("Colour"))
            .child(swatches)
            .child(section(if two { "Logos" } else { "Logo" }))
            .child(logo_controls)
            .when(sig.logo.is_some(), |d| d.child(sizes))
            .child(field(8))
            .when(two, |d| d.child(logo2_controls).child(field(9)))
            .child(section("Follow line"))
            .child(div().flex().gap_2().child(div().w(px(110.)).child(field(10))).child(div().flex_1().child(field(11))))
            .when(sig.logo.is_some() && sig.layout == Layout::Stacked, |d| {
                d.child(div().text_xs().text_color(rgb(MUTED)).child("The Text layout leaves the logo out."))
            })
            .child(section("Use by default for"))
            .child(defaults)
            .child(
                div().pt_4().child(
                    action_button("sig-delete", "Delete signature", false)
                        .text_color(rgb(ERROR))
                        .on_click(cx.listener(move |this, _, w, cx| this.delete_signature(id, w, cx))),
                ),
            );
        let preview = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .p_5()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().text_xs().text_color(rgb(MUTED)).child("How it arrives"))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .rounded_md()
                    .overflow_hidden()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .child(self.html_view.placeholder()),
            );
        div().size_full().flex().child(list).child(form).child(preview)
    }

    /// A compact native drawing of a signature, under the composer.
    pub fn signature_card(&self, sig: &Signature) -> Div {
        let accent = sig.accent;
        let mut details = div().flex().flex_col().gap_0p5().text_xs();
        if !sig.name.trim().is_empty() {
            details = details.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(rgb(0x111114)).child(sig.name.clone()));
        }
        let role = sig.role_line();
        if !role.is_empty() {
            details = details.child(div().text_color(rgb(0x45454d)).child(role));
        }
        let contact = sig.contact_parts();
        if !contact.is_empty() {
            details = details.child(div().text_color(rgb(accent)).child(contact.join("  |  ")));
        }
        for l in sig.note.lines().filter(|l| !l.trim().is_empty()) {
            details = details.child(div().text_color(rgb(0x7a7a84)).child(l.to_string()));
        }
        let logo = sig.logo.as_ref().filter(|_| sig.has_logo()).and_then(|(mime, bytes)| {
            let format = match mime.as_str() {
                "image/png" => ImageFormat::Png,
                "image/jpeg" => ImageFormat::Jpeg,
                "image/gif" => ImageFormat::Gif,
                _ => return None,
            };
            let w = sig.logo_width as f32 * 0.75;
            Some(img(std::sync::Arc::new(Image::from_bytes(format, bytes.clone()))).w(px(w)).flex_none())
        });
        let image = |logo: &Option<(String, Vec<u8>)>, w: u32| {
            logo.as_ref().and_then(|(mime, bytes)| {
                let format = match mime.as_str() {
                    "image/png" => ImageFormat::Png,
                    "image/jpeg" => ImageFormat::Jpeg,
                    "image/gif" => ImageFormat::Gif,
                    _ => return None,
                };
                Some(img(std::sync::Arc::new(Image::from_bytes(format, bytes.clone()))).w(px(w as f32 * 0.75)).flex_none())
            })
        };
        if sig.layout == Layout::TwoLogos {
            let rule = || div().w(px(1.)).self_stretch().bg(rgb(0xdedee3));
            let left_w = px(sig.logo_width.max(140) as f32 * 0.75);
            let mut left = div().w(left_w).flex_none().flex().flex_col().text_xs();
            if !sig.name.trim().is_empty() {
                left = left.child(div().text_sm().font_weight(FontWeight::SEMIBOLD).text_color(rgb(0x111114)).child(sig.name.clone()));
            }
            if !sig.title.trim().is_empty() {
                left = left.child(div().text_color(rgb(0x45454d)).child(sig.title.clone()));
            }
            let mut right = div().flex().flex_col().text_xs().text_color(rgb(0x45454d));
            for (k, v) in [("p", &sig.phone), ("e", &sig.email), ("w", &sig.website)] {
                if !v.trim().is_empty() {
                    right = right.child(format!("{k}: {}", v.trim()));
                }
            }
            let mut card = div()
                .flex()
                .flex_col()
                .gap_2()
                .child(div().flex().gap_3().child(left).child(rule()).child(right));
            if sig.logo.is_some() || sig.logo2.is_some() {
                card = card.child(
                    div()
                        .flex()
                        .items_end()
                        .gap_3()
                        .child(div().w(left_w).flex_none().children(image(&sig.logo, sig.logo_width)))
                        .child(rule())
                        .children(image(&sig.logo2, sig.logo2_width)),
                );
            }
            if let Some((lead, label)) = sig.follow_line() {
                card = card.child(
                    div()
                        .flex()
                        .gap_1p5()
                        .text_xs()
                        .child(div().text_color(rgb(0x7a7a84)).child(lead))
                        .child(div().text_color(rgb(accent)).child(label)),
                );
            }
            return card;
        }
        match (sig.layout, logo) {
            (Layout::LogoLeft, Some(logo)) => div()
                .flex()
                .items_center()
                .gap_3()
                .child(logo)
                .child(div().w(px(2.)).self_stretch().bg(rgb(accent)))
                .child(details),
            (Layout::LogoTop, Some(logo)) => div()
                .flex()
                .flex_col()
                .gap_2()
                .child(logo)
                .child(div().h(px(2.)).w(px(160.)).bg(rgb(accent)))
                .child(details),
            _ => div().flex().flex_col().gap_2().child(div().h(px(2.)).w(px(160.)).bg(rgb(accent))).child(details),
        }
    }
}

// ----- From addresses (identities) -----------------------------------------

impl MailApp {
    pub fn edit_address(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(who) = self.identities.iter().find(|i| i.id == id).cloned() else { return };
        let input = |value: String, placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                s.set_value(value, window, cx);
                s
            })
        };
        let name = input(who.name.clone(), "Name shown to recipients", window, cx);
        let email = input(who.email.clone(), "you@example.com", window, cx);
        let mut subs = Vec::new();
        for (field, entity) in [(0, name.clone()), (1, email.clone())] {
            subs.push(cx.subscribe(&entity, move |this, state, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    let v = state.read(cx).value().trim().to_string();
                    this.update_address(id, cx, |i| if field == 0 { i.name = v } else { i.email = v });
                }
            }));
        }
        if let Some(v) = &mut self.settings_view {
            v.pane = Pane::Addresses;
            v.address = Some(AddressEditor { id, name, email, _subs: subs });
        }
        cx.notify();
    }

    fn update_address(&mut self, id: i64, cx: &mut Context<Self>, f: impl FnOnce(&mut Identity)) {
        let Some(who) = self.identities.iter_mut().find(|i| i.id == id) else { return };
        f(who);
        // An address without an @ isn't saved yet (it's being typed).
        if who.email.contains('@') {
            if let Err(e) = identity::save(who) {
                self.status = format!("Could not save the address: {e:#}").into();
                self.status_is_error = true;
            }
        }
        cx.notify();
    }

    fn new_address(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut who = Identity {
            id: 0,
            name: self.current_account().map(|a| a.name.clone()).unwrap_or_default(),
            email: String::new(),
            sig_new: None,
            sig_reply: None,
            account: self.account.to_string(),
        };
        who.id = self.identities.iter().map(|i| i.id).max().unwrap_or(0) + 1;
        let id = who.id;
        self.identities.push(who);
        self.edit_address(id, window, cx);
    }

    fn delete_address(&mut self, id: i64, cx: &mut Context<Self>) {
        identity::delete(id).ok();
        self.identities.retain(|i| i.id != id);
        if let Some(v) = &mut self.settings_view {
            v.address = None;
        }
        cx.notify();
    }

    fn render_addresses_pane(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let editing = self.settings_view.as_ref().and_then(|v| v.address.as_ref());
        let sig_name = |id: Option<i64>| {
            id.and_then(|id| self.signatures.iter().find(|s| s.id == id)).map(|s| s.label.clone()).unwrap_or_else(|| "no signature".into())
        };
        let row = |who: &Identity, on: bool| {
            div()
                .px_4()
                .py_3()
                .rounded_md()
                .border_1()
                .border_color(rgb(if on { ACCENT } else { BORDER }))
                .when(on, |d| d.bg(rgb(SELECTED)))
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .min_w_0()
                        .child(div().text_sm().text_color(rgb(TEXT)).child(who.label()))
                        .child(div().text_xs().text_color(rgb(MUTED)).child(format!(
                            "New: {} · Replies: {}",
                            sig_name(who.sig_new),
                            sig_name(who.sig_reply)
                        ))),
                )
        };
        let all = self.all_identities();
        let main = row(&all[0], false).child(div().text_xs().text_color(rgb(MUTED)).child("Account · signatures under Signatures"));
        let mut list = div().flex().flex_col().gap_2().child(main);
        for who in &self.account_identities() {
            let id = who.id;
            let on = editing.is_some_and(|e| e.id == id);
            list = list.child(
                row(who, on)
                    .id(("address", id as u64))
                    .cursor_pointer()
                    .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER))))
                    .on_click(cx.listener(move |this, _, w, cx| this.edit_address(id, w, cx)))
                    .child(div().text_xs().text_color(rgb(ACCENT)).child(if on { "Editing" } else { "Edit" })),
            );
            if let (true, Some(e)) = (on, editing) {
                list = list.child(self.render_address_editor(who, e, cx));
            }
        }
        list = list.child(
            div()
                .id("address-new")
                .mt_1()
                .text_sm()
                .text_color(rgb(ACCENT))
                .cursor_pointer()
                .hover(|d| d.underline())
                .on_click(cx.listener(|this, _, w, cx| this.new_address(w, cx)))
                .child("+ Add a From address"),
        );
        div()
            .id("addresses-pane")
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
                        "Extra addresses the account on screen sends as, each with its own signatures. Replies come from the address a message was sent to.",
                    ))
                    .child(list)
                    .child(div().text_xs().text_color(rgb(MUTED)).child(
                        "Gmail sends from an extra address only after it's added under Gmail Settings → Accounts → “Send mail as”. Until then Gmail replaces it with your Gmail address.",
                    )),
            )
    }

    fn render_address_editor(&self, who: &Identity, e: &AddressEditor, cx: &mut Context<Self>) -> impl IntoElement {
        let id = who.id;
        let input = |entity: &Entity<InputState>| {
            Input::new(entity).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER)).text_color(rgb(0xffffff))
        };
        let label = |t: &'static str| div().text_xs().text_color(rgb(MUTED)).child(t);
        let picks = |which: &'static str, current: Option<i64>, cx: &mut Context<Self>| {
            let mut row = div().flex().flex_wrap().gap_1p5().child(
                choice(SharedString::from(format!("addr-{which}-none")), "None", current.is_none()).on_click(cx.listener(
                    move |this, _, _, cx| this.update_address(id, cx, |i| if which == "new" { i.sig_new = None } else { i.sig_reply = None }),
                )),
            );
            for s in self.signatures_for(&self.account) {
                let sid = s.id;
                row = row.child(
                    choice(SharedString::from(format!("addr-{which}-{sid}")), s.label.clone(), current == Some(sid)).on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.update_address(id, cx, |i| if which == "new" { i.sig_new = Some(sid) } else { i.sig_reply = Some(sid) })
                        }),
                    ),
                );
            }
            row
        };
        let new_picks = picks("new", who.sig_new, cx);
        let reply_picks = picks("reply", who.sig_reply, cx);
        div()
            .ml_4()
            .pl_4()
            .py_2()
            .border_l_2()
            .border_color(rgb(ACCENT))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().flex().flex_col().gap_1().child(label("Name")).child(input(&e.name)))
                    .child(div().flex_1().flex().flex_col().gap_1().child(label("Email address")).child(input(&e.email))),
            )
            .child(div().flex().flex_col().gap_1().child(label("Signature for new messages")).child(new_picks))
            .child(div().flex().flex_col().gap_1().child(label("Signature for replies")).child(reply_picks))
            .child(
                div().flex().child(
                    action_button("address-delete", "Remove address", false)
                        .text_color(rgb(ERROR))
                        .on_click(cx.listener(move |this, _, _, cx| this.delete_address(id, cx))),
                ),
            )
    }
}
