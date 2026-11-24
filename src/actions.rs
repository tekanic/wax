//! Message actions that work in every folder.
//!
//! Keys, the right-click menu and trackpad swipes all end in `perform`, so
//! they behave the same. What an action means depends on the folder: `e`
//! archives in the Inbox but restores from Trash, `b` blocks a sender but
//! allows one back from Screened Out. The hint bar and the menu are built
//! from the same rules, so they always show what the keys will really do.

use super::*;

/// The folder being looked at, as far as actions care.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Folder {
    Inbox,
    Starred,
    Important,
    Sent,
    Drafts,
    AllMail,
    Spam,
    Trash,
    Screened,
    /// Labels, Reading, Records, search results.
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MsgAction {
    Reply,
    ReplyAll,
    Forward,
    OpenDraft,
    ToggleRead,
    ToggleStar,
    ToggleImportant,
    Archive,
    ToInbox,
    Delete,
    SenderReading,
    SenderRecords,
    BlockSender,
    AllowSender,
    Label,
    Select,
}

pub enum MenuLine {
    Heading(SharedString),
    Item { action: MsgAction, label: SharedString, keys: &'static str, danger: bool },
    Sep,
}

pub struct RowMenu {
    pub position: Point<Pixels>,
    pub lines: Vec<MenuLine>,
    /// Index into `lines` of the highlighted item.
    pub hover: Option<usize>,
    pub focus: FocusHandle,
}

/// A horizontal swipe in progress on one row.
pub struct Swipe {
    pub ix: usize,
    pub dx: f32,
    /// Bumped on every event; a settle timer only fires for the latest.
    pub generation: u64,
}

/// How far a row must travel before letting go acts on it.
pub const SWIPE_COMMIT: f32 = 90.;
const SWIPE_MAX: f32 = 150.;
const MENU_WIDTH: f32 = 252.;

fn item(action: MsgAction, label: impl Into<SharedString>, keys: &'static str) -> MenuLine {
    MenuLine::Item { action, label: label.into(), keys, danger: false }
}

fn danger(action: MsgAction, label: impl Into<SharedString>, keys: &'static str) -> MenuLine {
    MenuLine::Item { action, label: label.into(), keys, danger: true }
}

impl MailApp {
    pub fn folder(&self) -> Folder {
        if !self.query.is_empty() {
            return Folder::Other;
        }
        if self.unified {
            return Folder::Inbox;
        }
        let Some(m) = self.active() else { return Folder::Other };
        let is = |s: &str| m.name.eq_ignore_ascii_case(s);
        if is(mail::INBOX) {
            Folder::Inbox
        } else if is(mail::STARRED) {
            Folder::Starred
        } else if is(mail::IMPORTANT) {
            Folder::Important
        } else if is(SENT) {
            Folder::Sent
        } else if is(mail::DRAFTS) {
            Folder::Drafts
        } else if is(ALL_MAIL) {
            Folder::AllMail
        } else if is(mail::SPAM) {
            Folder::Spam
        } else if is(mail::TRASH) {
            Folder::Trash
        } else if is(sweep::SCREENED) {
            Folder::Screened
        } else {
            Folder::Other
        }
    }

    /// What `e` does here, and its label. None where it means nothing.
    pub fn e_action(&self) -> Option<(MsgAction, &'static str)> {
        match self.folder() {
            Folder::Trash => Some((MsgAction::ToInbox, "restore")),
            Folder::Spam => Some((MsgAction::ToInbox, "not spam")),
            Folder::Screened => Some((MsgAction::ToInbox, "move to Inbox")),
            Folder::AllMail | Folder::Sent | Folder::Drafts => None,
            _ => Some((MsgAction::Archive, "archive")),
        }
    }

    /// What `b` does here: block the sender, or let them back in.
    pub fn b_action(&self) -> Option<(MsgAction, &'static str)> {
        match self.folder() {
            Folder::Screened => Some((MsgAction::AllowSender, "allow sender")),
            Folder::Sent | Folder::Drafts => None,
            _ => Some((MsgAction::BlockSender, "block sender")),
        }
    }

    /// The hint bar for the message list, most useful first. It follows the
    /// folder, the selected message and the selection.
    pub fn list_hints(&self) -> Vec<(SharedString, SharedString)> {
        let f = self.folder();
        let mut h: Vec<(&'static str, String)> = Vec::new();
        let n = self.marked.len();
        if n > 0 {
            if let Some((_, label)) = self.e_action() {
                h.push(("e", label.into()));
            }
            h.push(("⌫", if matches!(f, Folder::Trash | Folder::Spam) { "delete forever" } else { "delete" }.into()));
            h.push(("s", "star".into()));
            h.push(("u", "read/unread".into()));
            h.push(("x", "select".into()));
            h.push(("⌘A", "select all".into()));
            h.push(("esc", "clear selection".into()));
        } else if f == Folder::Drafts {
            h.push(("⏎", "open draft".into()));
            h.push(("⌫", "discard".into()));
            h.push(("j/k", "move".into()));
            h.push(("c", "compose".into()));
        } else if let Some(e) = self.envelopes.get(self.selected) {
            h.push(("j/k", "move".into()));
            if !matches!(f, Folder::Spam | Folder::Trash | Folder::Screened) {
                h.push(("r/a", "reply/all".into()));
            }
            if let Some((_, label)) = self.e_action() {
                h.push(("e", label.into()));
            }
            h.push(("⌫", if matches!(f, Folder::Trash | Folder::Spam) { "delete forever" } else { "delete" }.into()));
            if let Some((_, label)) = self.b_action() {
                h.push(("b", label.into()));
            }
            h.push(("s", if e.starred { "unstar" } else { "star" }.into()));
            if !matches!(f, Folder::Spam | Folder::Trash) {
                h.push(("i", if e.important { "not important" } else { "important" }.into()));
            }
            h.push(("u", if e.unread { "mark read" } else { "mark unread" }.into()));
            h.push(("x", "select".into()));
        } else {
            h.push(("c", "compose".into()));
        }
        h.push(("g", "go to…".into()));
        h.push(("/", "search".into()));
        h.into_iter().map(|(k, l)| (k.into(), l.into())).collect()
    }

    pub fn perform(&mut self, action: MsgAction, window: &mut Window, cx: &mut Context<Self>) {
        self.row_menu = None;
        self.focus.focus(window, cx);
        match action {
            MsgAction::Reply => self.respond(Kind::Reply, window, cx),
            MsgAction::ReplyAll => self.respond(Kind::ReplyAll, window, cx),
            MsgAction::Forward => self.respond(Kind::Forward, window, cx),
            MsgAction::OpenDraft => self.open_draft(&OpenDraft, window, cx),
            MsgAction::ToggleRead => self.toggle_read_targets(cx),
            MsgAction::ToggleStar => self.toggle_flag(true, cx),
            MsgAction::ToggleImportant => self.toggle_flag(false, cx),
            MsgAction::Archive => self.bulk(BulkKind::Archive, cx),
            MsgAction::ToInbox => self.bulk(BulkKind::ToInbox, cx),
            MsgAction::Delete => self.bulk(BulkKind::Delete, cx),
            MsgAction::SenderReading => self.decide_sender_of_selected(sweep::READING, cx),
            MsgAction::SenderRecords => self.decide_sender_of_selected(sweep::RECORDS, cx),
            MsgAction::BlockSender => self.decide_sender_of_selected(sweep::SCREENED, cx),
            MsgAction::AllowSender => self.decide_sender_of_selected(sweep::KEEP, cx),
            MsgAction::Label => {
                self.open_label_picker(window, cx);
                return;
            }
            MsgAction::Select => {
                if let Some(e) = self.envelopes.get(self.selected) {
                    let key = e.key();
                    if !self.marked.remove(&key) {
                        self.marked.insert(key);
                    }
                }
            }
        }
        cx.notify();
    }

    /// `e`: archive, or back to the Inbox from Trash / Spam / Screened Out.
    pub fn primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.e_action() {
            Some((action, _)) => self.perform(action, window, cx),
            None => {
                self.status = "Nothing to archive here: use ⌫ to delete".into();
                self.status_is_error = true;
                cx.notify();
            }
        }
    }

    /// `b`: block the sender, or allow them back from Screened Out.
    pub fn sender_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((action, _)) = self.b_action() {
            self.perform(action, window, cx);
        }
    }

    /// Read state of the selection (or the current message).
    pub fn toggle_read_targets(&mut self, cx: &mut Context<Self>) {
        let items = self.targets();
        let Some(first) = items.first() else { return };
        let unread = !first.unread;
        for e in &items {
            let pos = self.envelopes.iter().position(|x| x.key() == e.key());
            if let Some(ix) = pos.filter(|&ix| self.envelopes[ix].unread != unread) {
                self.set_unread(ix, unread, cx);
            }
        }
    }

    /// Star (`star = true`) or Important on the selection. Instant, no undo
    /// window: pressing the key again is the undo.
    pub fn toggle_flag(&mut self, star: bool, cx: &mut Context<Self>) {
        let items = self.targets();
        let Some(first) = items.first() else { return };
        let get = |e: &Envelope| if star { e.starred } else { e.important };
        let on = !get(first);
        let keys: HashSet<(String, String)> = items.iter().map(|e| e.key()).collect();
        let mids: HashSet<String> = items.iter().filter_map(|e| e.message_id.clone()).collect();
        set_flag(&mut self.envelopes, &keys, &mids, star, on);
        let n = items.len();
        let what = match (star, on) {
            (true, true) => "Starred",
            (true, false) => "Removed the star from",
            (false, true) => "Marked important:",
            (false, false) => "Marked not important:",
        };
        self.status = format!("{what} {n} message{}", if n == 1 { "" } else { "s" }).into();
        self.status_is_error = false;
        cx.notify();
        if items.iter().any(|e| e.mailbox == "demo") {
            return;
        }
        let mut sessions: std::collections::HashMap<String, SharedSession> = Default::default();
        for e in &items {
            let a = self.account_of(e).to_string();
            let Some(s) = self.session_of(&a) else { return };
            sessions.insert(a, s);
        }
        let accounts: Vec<String> = items.iter().map(|e| self.account_of(e).to_string()).collect();
        let store = self.store.clone();
        let op = match (star, on) {
            (true, true) => "+FLAGS.SILENT (\\Flagged)",
            (true, false) => "-FLAGS.SILENT (\\Flagged)",
            (false, true) => "+X-GM-LABELS (\\Important)",
            (false, false) => "-X-GM-LABELS (\\Important)",
        };
        let mut changed: Vec<Envelope> = items.clone();
        for e in changed.iter_mut() {
            if star { e.starred = on } else { e.important = on }
        }
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut by_box: std::collections::BTreeMap<(String, String), Vec<String>> = Default::default();
                    for (e, a) in changed.iter().zip(&accounts) {
                        by_box.entry((a.clone(), e.mailbox.clone())).or_default().push(e.id.clone());
                    }
                    for ((account, mailbox), ids) in &by_box {
                        sessions[account].lock().unwrap().store_op(mailbox, ids, op)?;
                    }
                    if let Some(store) = store {
                        let store = store.lock().unwrap();
                        for (e, a) in changed.iter().zip(&accounts) {
                            store.set_flags(a, e)?;
                        }
                    }
                    anyhow::Ok(())
                })
                .await;
            if let Err(err) = result {
                this.update(cx, |this, cx| {
                    set_flag(&mut this.envelopes, &keys, &mids, star, !on);
                    this.status = format!("Gmail refused the change: {err:#}").into();
                    this.status_is_error = true;
                    cx.notify();
                })
                .ok();
            }
        })
        .detach();
    }

    /// Sweep-style decision about the selected message's sender, from any
    /// folder. Held for the undo window like the Sweep's own.
    pub fn decide_sender_of_selected(&mut self, dest: &'static str, cx: &mut Context<Self>) {
        let Some(e) = self.envelopes.get(self.selected).cloned() else { return };
        // From All Inboxes: the rule belongs to the message's own account.
        let acct = self.account_of(&e);
        let email = e.from_email.to_lowercase();
        if email.is_empty() || e.mailbox == "demo" {
            return;
        }
        let previous = self.rule_in(&acct, &email);
        if previous.as_deref() == Some(dest) {
            self.status = format!("{} is already set to {}", e.from, if dest == sweep::KEEP { "stay in the Inbox" } else { dest }).into();
            self.status_is_error = false;
            cx.notify();
            return;
        }
        let count = self
            .store
            .as_ref()
            .and_then(|s| s.lock().unwrap().uids_from(&*acct, ALL_MAIL, &email).ok())
            .map(|v| v.len() as u64)
            .unwrap_or(0);
        // Their mail leaves this view right away where the decision moves it.
        let f = self.folder();
        let hide = match dest {
            sweep::KEEP => f == Folder::Screened,
            sweep::SCREENED => matches!(f, Folder::Inbox | Folder::Important),
            _ => f == Folder::Inbox,
        };
        let mut hidden = Vec::new();
        if hide {
            let (gone, kept): (Vec<Envelope>, Vec<Envelope>) =
                std::mem::take(&mut self.envelopes).into_iter().partition(|x| x.from_email.to_lowercase() == email && x.account == e.account);
            self.envelopes = kept;
            hidden = gone;
            self.marked.clear();
            self.selected = self.selected.min(self.envelopes.len().saturating_sub(1));
            self.load_body(self.selected, cx);
        }
        self.queue_sender(acct.to_string(), email, e.from.to_string(), count, dest, previous, hidden, cx);
    }

    // ----- Right-click menu ------------------------------------------------

    /// Right-click on row `ix`. On a selected (marked) row the menu acts on
    /// the whole selection; otherwise it selects that one row.
    pub fn open_row_menu(&mut self, ix: usize, position: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(e) = self.envelopes.get(ix) else { return };
        let key = e.key();
        if !self.marked.contains(&key) {
            self.marked.clear();
        }
        self.select(ix, cx);
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        self.row_menu = Some(RowMenu { position, lines: self.menu_lines(), hover: None, focus });
        cx.notify();
    }

    pub fn close_row_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.row_menu.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    fn menu_lines(&self) -> Vec<MenuLine> {
        use MsgAction::*;
        let f = self.folder();
        let n = self.targets().len();
        let Some(e) = self.envelopes.get(self.selected) else { return Vec::new() };
        let mut v = Vec::new();
        if n > 1 {
            v.push(MenuLine::Heading(format!("{n} selected messages").into()));
        }
        if n == 1 {
            if f == Folder::Drafts {
                v.push(item(OpenDraft, "Open draft", "⏎"));
            } else {
                v.push(item(Reply, "Reply", "r"));
                v.push(item(ReplyAll, "Reply all", "a"));
                v.push(item(Forward, "Forward", "f"));
            }
            v.push(MenuLine::Sep);
        }
        v.push(item(ToggleRead, if e.unread { "Mark as read" } else { "Mark as unread" }, "u"));
        v.push(item(ToggleStar, if e.starred { "Remove star" } else { "Star" }, "s"));
        if !matches!(f, Folder::Spam | Folder::Trash) {
            v.push(item(ToggleImportant, if e.important { "Mark not important" } else { "Mark important" }, "i"));
        }
        v.push(item(Label, "Label…", "l"));
        v.push(MenuLine::Sep);
        match self.e_action() {
            Some((Archive, _)) => v.push(item(Archive, "Archive", "e")),
            Some((action, _)) => v.push(item(
                action,
                match f {
                    Folder::Trash => "Restore to Inbox",
                    Folder::Spam => "Not spam",
                    _ => "Move to Inbox",
                },
                "e",
            )),
            None => {}
        }
        v.push(danger(
            Delete,
            match f {
                Folder::Drafts => "Discard draft",
                Folder::Trash | Folder::Spam => "Delete forever",
                _ => "Delete",
            },
            "⌫",
        ));
        let email = e.from_email.to_lowercase();
        if n == 1 && !email.is_empty() && !matches!(f, Folder::Sent | Folder::Drafts) {
            let rule = self.rules.get(&email).map(String::as_str);
            v.push(MenuLine::Sep);
            v.push(MenuLine::Heading(format!("Everything from {}", e.from).into()));
            let mark = |label: &str, dest: &str| -> SharedString {
                if rule == Some(dest) { format!("{label}  ✓").into() } else { label.to_string().into() }
            };
            v.push(item(SenderReading, mark("File in Reading", sweep::READING), "⇧R"));
            v.push(item(SenderRecords, mark("File in Records", sweep::RECORDS), "⇧C"));
            match self.b_action() {
                Some((AllowSender, _)) => v.push(item(AllowSender, "Allow sender", "b")),
                _ => v.push(danger(BlockSender, "Block sender", "b")),
            }
        }
        v.push(MenuLine::Sep);
        let marked = self.marked.contains(&e.key());
        v.push(item(Select, if marked { "Deselect" } else { "Select" }, "x"));
        v
    }

    pub fn menu_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(m) = &mut self.row_menu else { return };
        let items: Vec<usize> = m
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| matches!(l, MenuLine::Item { .. }))
            .map(|(i, _)| i)
            .collect();
        if items.is_empty() {
            return;
        }
        let at = m.hover.and_then(|h| items.iter().position(|&i| i == h));
        let next = match at {
            None if delta > 0 => 0,
            None => items.len() - 1,
            Some(p) => (p as isize + delta).rem_euclid(items.len() as isize) as usize,
        };
        m.hover = Some(items[next]);
        cx.notify();
    }

    pub fn menu_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let action = self.row_menu.as_ref().and_then(|m| match m.lines.get(m.hover?) {
            Some(MenuLine::Item { action, .. }) => Some(*action),
            _ => None,
        });
        match action {
            Some(a) => self.perform(a, window, cx),
            None => self.close_row_menu(window, cx),
        }
    }

    pub fn render_row_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(m) = &self.row_menu else { return div().into_any_element() };
        // Keep the menu over the list column: the reader's HTML view is a
        // native view that would cover anything drawn over it.
        let max_x = SIDEBAR_WIDTH + LIST_WIDTH - MENU_WIDTH - 6.;
        let pos = point(px(m.position.x.as_f32().min(max_x)), m.position.y);
        let menu = div()
            .id("row-menu")
            .track_focus(&m.focus)
            .key_context("RowMenu")
            .w(px(MENU_WIDTH))
            .py_1p5()
            .rounded_lg()
            .bg(rgb(0x24242b))
            .border_1()
            .border_color(rgb(0x3d3d47))
            .shadow_lg()
            .flex()
            .flex_col()
            .text_sm()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(m.lines.iter().enumerate().map(|(i, line)| match line {
                MenuLine::Sep => div().my_1().h(px(1.)).bg(rgb(0x34343d)).into_any_element(),
                MenuLine::Heading(text) => div()
                    .px_3()
                    .pt_1()
                    .pb_0p5()
                    .text_xs()
                    .truncate()
                    .text_color(rgb(MUTED))
                    .child(text.clone())
                    .into_any_element(),
                MenuLine::Item { action, label, keys, danger } => {
                    let action = *action;
                    let hovered = m.hover == Some(i);
                    div()
                        .id(("menu-item", i))
                        .mx_1p5()
                        .px_2()
                        .h(px(26.))
                        .rounded_md()
                        .flex()
                        .items_center()
                        .justify_between()
                        .cursor_pointer()
                        .text_color(rgb(if *danger { 0xf0a0a0 } else { TEXT }))
                        .when(hovered, |d| d.bg(rgb(ACCENT)).text_color(rgb(0xffffff)))
                        .when(!hovered, |d| d.hover(|d| d.bg(rgb(0x30303a))))
                        .on_mouse_move(cx.listener(move |this, _, _, cx| {
                            if let Some(m) = &mut this.row_menu {
                                if m.hover != Some(i) {
                                    m.hover = Some(i);
                                    cx.notify();
                                }
                            }
                        }))
                        .on_click(cx.listener(move |this, _, window, cx| this.perform(action, window, cx)))
                        .child(label.clone())
                        .child(
                            div()
                                .text_xs()
                                .font_family("Menlo")
                                .text_color(rgb(if hovered { 0xdbe7ff } else { MUTED }))
                                .child(*keys),
                        )
                        .into_any_element()
                }
            }));
        div()
            .id("row-menu-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _, window, cx| this.close_row_menu(window, cx)))
            .on_mouse_down(MouseButton::Right, cx.listener(|this, _, window, cx| this.close_row_menu(window, cx)))
            .child(anchored().position(pos).snap_to_window_with_margin(px(8.)).child(menu))
            .into_any_element()
    }

    // ----- Swipes ------------------------------------------------------------

    /// What a swipe does: right = read/unread, left = the folder's `e`
    /// action (or delete where `e` means nothing).
    pub fn swipe_action(&self, dx: f32) -> Option<(MsgAction, &'static str, u32)> {
        if dx > 0. {
            let unread = self.envelopes.get(self.selected).is_some_and(|e| e.unread);
            Some((MsgAction::ToggleRead, if unread { "Read" } else { "Unread" }, ACCENT))
        } else {
            match self.e_action() {
                Some((MsgAction::Archive, _)) => Some((MsgAction::Archive, "Archive", 0x3f9b62)),
                Some((a, _)) => Some((a, "Inbox", 0x3f9b62)),
                None if self.folder() != Folder::Trash => Some((MsgAction::Delete, "Delete", 0xc24848)),
                None => None,
            }
        }
    }

    /// Two-finger horizontal scroll over a row slides it; letting go past
    /// the line acts on it. Vertical scrolls pass through to the list.
    pub fn on_row_scroll(&mut self, ix: usize, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(20.));
        let (dx, dy) = (d.x.as_f32(), d.y.as_f32());
        let ended = matches!(ev.touch_phase, TouchPhase::Ended);
        if self.row_menu.is_some() || self.composer.is_some() {
            return;
        }
        if std::time::Instant::now() < self.swipe_quiet_until {
            return;
        }
        match &mut self.swipe {
            Some(s) if s.ix == ix => {
                s.dx = (s.dx + dx).clamp(-SWIPE_MAX, SWIPE_MAX);
                s.generation += 1;
            }
            // A new gesture: only clearly sideways ones become swipes.
            _ if !ended && dx.abs() > 2. && dx.abs() > dy.abs() * 1.5 => {
                self.swipe = Some(Swipe { ix, dx, generation: 0 });
                if self.selected != ix {
                    self.select(ix, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
        if ended {
            self.finish_swipe(window, cx);
            return;
        }
        // Mice and some trackpads never send "ended": settle after a pause.
        let generation = self.swipe.as_ref().map(|s| s.generation).unwrap_or(0);
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(220)).await;
            this.update_in(cx, |this, window, cx| {
                if this.swipe.as_ref().is_some_and(|s| s.generation == generation) {
                    this.finish_swipe(window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn finish_swipe(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(s) = self.swipe.take() else { return };
        // Swallow the momentum that follows a flick.
        self.swipe_quiet_until = std::time::Instant::now() + std::time::Duration::from_millis(450);
        if s.dx.abs() >= SWIPE_COMMIT {
            if let Some((action, _, _)) = self.swipe_action(s.dx) {
                self.selected = s.ix.min(self.envelopes.len().saturating_sub(1));
                self.marked.clear();
                self.perform(action, window, cx);
            }
        }
        cx.notify();
    }

    /// Row `ix` drawn under its swipe: the action revealed behind it and the
    /// row itself slid along.
    pub fn swipe_wrap(&self, ix: usize, row: Stateful<Div>) -> Stateful<Div> {
        let Some(s) = self.swipe.as_ref().filter(|s| s.ix == ix) else { return row };
        let Some((_, label, color)) = self.swipe_action(s.dx) else { return row };
        let armed = s.dx.abs() >= SWIPE_COMMIT;
        let reveal = div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .px_5()
            .flex()
            .items_center()
            .when(s.dx > 0., |d| d.justify_start())
            .when(s.dx < 0., |d| d.justify_end())
            .bg(rgb(if armed { color } else { 0x2a2a31 }))
            .text_sm()
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(if armed { 0xffffff } else { SUBTLE }))
            .child(label);
        div()
            .id(("swipe", ix))
            .relative()
            .w_full()
            .h(px(ROW_HEIGHT))
            .overflow_hidden()
            .child(reveal)
            .child(row.relative().left(px(s.dx)).bg(rgb(LIST_BG)))
    }
}

fn set_flag(list: &mut [Envelope], keys: &HashSet<(String, String)>, mids: &HashSet<String>, star: bool, value: bool) {
    for e in list.iter_mut() {
        let same = keys.contains(&e.key())
            || e.message_id.as_ref().is_some_and(|m| mids.contains(m));
        if same {
            if star {
                e.starred = value
            } else {
                e.important = value
            }
        }
    }
}
