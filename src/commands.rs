use super::*;
use wasm_host::{Command, CommandScope};

pub struct CommandMenu {
    pub plugin: String,
    pub command: Command,
    pub message: Envelope,
    pub targets: Vec<Envelope>,
    pub input: Entity<InputState>,
    pub ix: usize,
    pub _sub: Subscription,
}

#[derive(Clone)]
enum Row {
    Typed(String),
    Choice(String, String, String),
}

impl CommandMenu {
    fn rows(&self, cx: &App) -> Vec<Row> {
        let typed = self.input.read(cx).value().trim().to_string();
        let mut rows = Vec::new();
        if !typed.is_empty() {
            rows.push(Row::Typed(typed.clone()));
        }
        let needle = typed.to_lowercase();
        for c in &self.command.choices {
            if needle.is_empty() || c.title.to_lowercase().contains(&needle) {
                rows.push(Row::Choice(c.id.clone(), c.title.clone(), c.detail.clone()));
            }
        }
        rows
    }
}

pub fn parse_key(binding: &str) -> Option<(bool, String)> {
    let parts: Vec<&str> = binding.split('-').collect();
    let (key, mods) = parts.split_last()?;
    let mut shift = false;
    for m in mods {
        match *m {
            "shift" => shift = true,
            _ => return None,
        }
    }
    (!key.is_empty()).then(|| (shift, key.to_lowercase()))
}

impl MailApp {
    pub fn refresh_commands(&mut self, cx: &mut Context<Self>) {
        let Some(host) = self.wasm.clone() else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (cmds, problems) = cx.background_executor().spawn(async move { host.lock().unwrap().commands() }).await;
            this.update(cx, |this, cx| {
                this.plugin_commands = cmds;
                for p in &problems {
                    crate::errlog::record("", p);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn is_own(&self, e: &Envelope) -> bool {
        e.mailbox.eq_ignore_ascii_case(mail::SENT) || self.own_addresses(&self.account_of(e)).contains(&e.from_email.to_lowercase())
    }

    pub fn command_applies(&self, c: &Command, e: &Envelope) -> bool {
        match c.scope {
            CommandScope::Any => true,
            CommandScope::Sent => self.is_own(e),
            CommandScope::Received => !self.is_own(e),
        }
    }

    pub fn reader_commands(&self, e: &Envelope) -> Vec<(String, Command)> {
        self.plugin_commands.iter().filter(|(_, c)| c.in_message_toolbar && self.command_applies(c, e)).cloned().collect()
    }

    pub fn command_for_key(&self, shift: bool, key: &str) -> Option<(String, String)> {
        let e = self.reading()?;
        self.plugin_commands
            .iter()
            .find(|(_, c)| c.keybinding.as_deref().and_then(parse_key).is_some_and(|(s, k)| s == shift && k == key) && self.command_applies(c, e))
            .map(|(p, c)| (p.clone(), c.id.clone()))
    }

    pub fn list_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let m = &ev.keystroke.modifiers;
        if m.control || m.alt || m.platform || m.function || ev.is_held {
            return false;
        }
        if self.composer.is_some()
            || self.settings_view.is_some()
            || self.sweep.is_some()
            || self.label_picker.is_some()
            || self.row_menu.is_some()
            || self.command_menu.is_some()
            || self.search.read(cx).focus_handle(cx).is_focused(window)
        {
            return false;
        }
        let Some((plugin, id)) = self.command_for_key(m.shift, &ev.keystroke.key.to_lowercase()) else { return false };
        self.invoke_command(&plugin, &id, window, cx);
        true
    }

    pub fn invoke_command(&mut self, plugin: &str, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(message) = self.reading().cloned() else { return };
        let row = self.envelopes.get(self.selected).cloned();
        let targets = if row.as_ref().is_some_and(|r| r.key() == message.key()) { self.targets() } else { vec![message.clone()] };
        let Some(host) = self.wasm.clone() else { return };
        let (plugin, id) = (plugin.to_string(), id.to_string());
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            let (cmds, _) = cx.background_executor().spawn(async move { host.lock().unwrap().commands() }).await;
            this.update_in(cx, |this, window, cx| {
                let Some(command) = cmds.iter().find(|(p, c)| *p == plugin && c.id == id).map(|(_, c)| c.clone()) else { return };
                this.plugin_commands = cmds;
                if command.choices.is_empty() && command.text_hint.is_none() {
                    this.run_plugin_command(plugin, command, None, message, targets, cx);
                } else {
                    this.open_command_menu(plugin, command, message, targets, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn open_command_menu(&mut self, plugin: String, command: Command, message: Envelope, targets: Vec<Envelope>, window: &mut Window, cx: &mut Context<Self>) {
        let hint = command.text_hint.clone().unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(hint));
        input.update(cx, |s, cx| s.focus(window, cx));
        let sub = cx.subscribe(&input, |this, _, ev: &InputEvent, cx| {
            if let InputEvent::Change = ev {
                if let Some(m) = &mut this.command_menu {
                    m.ix = 0;
                }
                cx.notify();
            }
        });
        self.command_menu = Some(CommandMenu { plugin, command, message, targets, input, ix: 0, _sub: sub });
        cx.notify();
    }

    pub fn close_command_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.command_menu.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        }
    }

    pub fn command_menu_move(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(m) = &self.command_menu else { return };
        let n = m.rows(cx).len();
        if n == 0 {
            return;
        }
        if let Some(m) = &mut self.command_menu {
            m.ix = (m.ix as isize + delta).rem_euclid(n as isize) as usize;
        }
        cx.notify();
    }

    pub fn command_menu_accept(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(m) = &self.command_menu else { return };
        let rows = m.rows(cx);
        let Some(row) = rows.get(m.ix.min(rows.len().saturating_sub(1))).cloned() else { return };
        let choice = match row {
            Row::Typed(t) => format!("text:{t}"),
            Row::Choice(id, _, _) => id,
        };
        let Some(m) = self.command_menu.take() else { return };
        self.focus.focus(window, cx);
        self.run_plugin_command(m.plugin, m.command, Some(choice), m.message, m.targets, cx);
    }

    fn run_plugin_command(&mut self, plugin: String, command: Command, choice: Option<String>, message: Envelope, targets: Vec<Envelope>, cx: &mut Context<Self>) {
        let Some(host) = self.wasm.clone() else { return };
        let account = self.account_of(&message).to_string();
        let wit = plugins::to_wit(&account, &message);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (actions, problems) = cx
                .background_executor()
                .spawn(async move { host.lock().unwrap().run_command(&plugin, &command.id, choice.as_deref(), Some(&wit)) })
                .await;
            this.update(cx, |this, cx| this.command_actions(&account, message, targets, actions, problems, cx)).ok();
        })
        .detach();
    }

    fn command_actions(&mut self, account: &str, message: Envelope, targets: Vec<Envelope>, actions: Vec<wasm_host::Action>, problems: Vec<String>, cx: &mut Context<Self>) {
        use wasm_host::Action as A;
        let mut other = Vec::new();
        for a in actions {
            match a {
                A::SnoozeUntil(t) => match chrono::DateTime::parse_from_rfc3339(&t) {
                    Ok(at) => self.snooze(account, targets.clone(), at.timestamp(), cx),
                    Err(_) => self.say_error(format!("The plugin gave a time Wax can't read: {t}"), cx),
                },
                A::FollowUp(t) => match chrono::DateTime::parse_from_rfc3339(&t) {
                    Ok(at) => self.follow_up(account, message.clone(), at.timestamp(), cx),
                    Err(_) => self.say_error(format!("The plugin gave a time Wax can't read: {t}"), cx),
                },
                A::Notify(text) => {
                    self.status = text.into();
                    self.status_is_error = false;
                }
                a => other.push(a),
            }
        }
        if let Some(first) = problems.first() {
            for p in &problems {
                crate::errlog::record(account, p);
            }
            self.say_error(first.clone(), cx);
        }
        if !other.is_empty() {
            let Some(session) = self.preview_of(account) else { return };
            cx.background_executor()
                .spawn(async move {
                    for e in targets {
                        plugins::apply_actions(&session, &e.mailbox, &e.id, &other).ok();
                    }
                })
                .detach();
        }
        cx.notify();
    }

    pub fn say_error(&mut self, text: String, cx: &mut Context<Self>) {
        self.status = text.into();
        self.status_is_error = true;
        cx.notify();
    }

    pub fn render_command_buttons(&self, e: &Envelope, cx: &mut Context<Self>) -> Vec<AnyElement> {
        self.reader_commands(e)
            .into_iter()
            .map(|(plugin, c)| {
                let key = c.keybinding.as_deref().and_then(parse_key).map(|(s, k)| if s { format!("⇧{}", k.to_uppercase()) } else { k }).unwrap_or_default();
                let label = if key.is_empty() { c.title.clone() } else { format!("{}  {key}", c.title) };
                let id = c.id.clone();
                action_button(SharedString::from(format!("cmd-{plugin}-{id}")), label, false)
                    .on_click(cx.listener(move |this, _, w, cx| this.invoke_command(&plugin, &id, w, cx)))
                    .into_any_element()
            })
            .collect()
    }

    pub fn render_command_menu(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let m = self.command_menu.as_ref()?;
        let rows = m.rows(cx);
        let ix = m.ix.min(rows.len().saturating_sub(1));
        let mut list = div().id("choice-rows").max_h(px(320.)).overflow_y_scroll().flex().flex_col().py_1();
        for (i, row) in rows.iter().enumerate() {
            let on = i == ix;
            let (title, detail, typed) = match row {
                Row::Typed(t) => (format!("“{t}”"), String::new(), true),
                Row::Choice(_, t, d) => (t.clone(), d.clone(), false),
            };
            list = list.child(
                div()
                    .id(("choice-row", i))
                    .mx_1p5()
                    .px_2()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .rounded_md()
                    .text_sm()
                    .cursor_pointer()
                    .text_color(rgb(if typed { ACCENT } else { TEXT }))
                    .when(on, |d| d.bg(rgb(SELECTED)))
                    .when(!on, |d| d.hover(|d| d.bg(rgb(HOVER))))
                    .on_click(cx.listener(move |this, _, w, cx| {
                        if let Some(m) = &mut this.command_menu {
                            m.ix = i;
                        }
                        this.command_menu_accept(w, cx)
                    }))
                    .child(div().min_w_0().truncate().child(title))
                    .child(div().flex_none().text_xs().text_color(rgb(MUTED)).child(detail)),
            );
        }
        let n = m.targets.len();
        let what = if n > 1 { format!("{} · {n} messages", m.command.title) } else { m.command.title.clone() };
        Some(
            deferred(
                anchored().position(point(px(SIDEBAR_WIDTH + 12.), px(70.))).child(
                    div()
                        .id("choice-menu")
                        .key_context("ChoiceMenu")
                        .w(px(LIST_WIDTH - 24.))
                        .rounded_lg()
                        .bg(rgb(0x24242b))
                        .border_1()
                        .border_color(rgb(0x3d3d47))
                        .shadow_lg()
                        .flex()
                        .flex_col()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(div().px_3().pt_2().text_xs().text_color(rgb(MUTED)).child(what))
                        .child(div().p_2().child(Input::new(&m.input).bg(rgb(SEARCH_BG)).border_color(rgb(SEARCH_BORDER))))
                        .child(list)
                        .child(
                            div()
                                .px_3()
                                .py_1p5()
                                .border_t_1()
                                .border_color(rgb(0x34343d))
                                .text_xs()
                                .text_color(rgb(MUTED))
                                .child("⏎ pick · esc close"),
                        ),
                ),
            )
            .with_priority(2),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::parse_key;

    #[test]
    fn reads_plugin_keys() {
        assert_eq!(parse_key("z"), Some((false, "z".into())));
        assert_eq!(parse_key("shift-z"), Some((true, "z".into())));
        assert_eq!(parse_key("cmd-z"), None);
        assert_eq!(parse_key(""), None);
    }
}
