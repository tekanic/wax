//! Every keyboard shortcut, defined once.
//!
//! The same table produces the key bindings, the hint bar at the bottom
//! of the window and the `g` menu, so they can never drift apart.
//!
//! ~/.config/wax/keymap.toml changes any of them: a section per place
//! (`[list]`, `[writing]`…), lines like `"e" = "archive"`, and `"none"` to
//! switch a key off. Wax writes the file with every default commented out
//! and picks up changes when it's saved.

use gpui_kit::*;

actions!(
    mail,
    [
        SelectNext,
        SelectPrev,
        FocusSearch,
        FocusList,
        ClearSearch,
        ToggleRead,
        ToggleHints,
        GoInbox,
        GoStarred,
        GoSent,
        GoDrafts,
        GoAllMail,
        Compose,
        Reply,
        ReplyAll,
        Forward,
        OpenDraft,
        SendMessage,
        CloseCompose,
        SaveDraft,
        ChoiceNext,
        ChoicePrev,
        ChoiceAccept,
        CloseChoices,
        DiscardCompose,
        UndoSend,
        SuggestNext,
        SuggestPrev,
        AcceptSuggestion,
        SendNow,
        ToggleMark,
        MarkAll,
        Archive,
        Delete,
        OpenSweep,
        CloseSweep,
        SweepNext,
        SweepPrev,
        ToggleStar,
        FormatBold,
        FormatItalic,
        FormatStrike,
        FormatLink,
        FormatCode,
        FormatBullets,
        FormatNumbers,
        FormatQuote,
        TogglePreview,
        ToggleMono,
        AttachFile,
        ShowBcc,
        FormatTable,
        SmartPaste,
        TableTab,
        TableBackTab,
        CycleSignature,
        OpenSettings,
        CloseSettings,
        ToggleImportant,
        SenderKey,
        SenderToReading,
        SenderToRecords,
        MenuNext,
        MenuPrev,
        MenuAccept,
        CloseMenu,
        OpenLabels,
        LabelNext,
        LabelPrev,
        LabelAccept,
        LabelAcceptArchive,
        CloseLabels,
        Quit,
    ]
);

/// Where a shortcut applies.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Moving around the folders and message list.
    List,
    /// Typing in the search box.
    Search,
    /// Second key after `g`.
    Go,
    /// Writing a message.
    Writing,
    /// Address suggestions open under To or Cc.
    Recipients,
    /// The send countdown overlay.
    Countdown,
    /// The Sweep (senders table).
    Sweep,
    /// A message's right-click menu.
    Menu,
    /// The settings screen.
    Settings,
    /// Inside the message text (paste and tab know about tables).
    Body,
    /// The label picker (`l`).
    Labels,
    /// A plugin command's menu (Snooze, Remind if no reply).
    Choices,
    /// Typing a label name in Settings → Labels.
    LabelEdit,
    /// Anywhere in the window.
    Global,
}

impl Mode {
    pub const ALL: [Mode; 14] = [
        Mode::List,
        Mode::Go,
        Mode::Search,
        Mode::Writing,
        Mode::Body,
        Mode::Recipients,
        Mode::Labels,
        Mode::Choices,
        Mode::LabelEdit,
        Mode::Sweep,
        Mode::Menu,
        Mode::Countdown,
        Mode::Settings,
        Mode::Global,
    ];

    /// Section name in keymap.toml.
    pub fn section(self) -> &'static str {
        match self {
            Mode::List => "list",
            Mode::Search => "search",
            Mode::Go => "go",
            Mode::Writing => "writing",
            Mode::Recipients => "recipients",
            Mode::Countdown => "countdown",
            Mode::Sweep => "sweep",
            Mode::Menu => "menu",
            Mode::Settings => "settings",
            Mode::Body => "body",
            Mode::Labels => "labels",
            Mode::Choices => "choices",
            Mode::LabelEdit => "label_edit",
            Mode::Global => "global",
        }
    }

    fn about(self) -> &'static str {
        match self {
            Mode::List => "moving around folders and the message list",
            Mode::Search => "typing in the search box",
            Mode::Go => "after g (two keys, e.g. \"g i\")",
            Mode::Writing => "writing a message",
            Mode::Recipients => "while address suggestions are open",
            Mode::Countdown => "during the send countdown",
            Mode::Sweep => "in the Sweep",
            Mode::Menu => "in a message's right-click menu",
            Mode::Settings => "in Settings",
            Mode::Body => "inside the message text",
            Mode::Labels => "in the label picker (l)",
            Mode::Choices => "in a command's menu (z snooze, w remind if no reply)",
            Mode::LabelEdit => "typing a label name in Settings",
            Mode::Global => "anywhere in the window",
        }
    }

    fn context(self) -> Option<&'static str> {
        match self {
            // Single-letter keys must not fire while typing in search.
            // ... nor while a menu is open (it has its own keys).
            // ... and never behind Settings, the Sweep or the composer, where
            // a stray letter would act on a message you can't see.
            Mode::List => Some("MailApp && !Input && !RowMenu && !Settings && !Sweep && !Composer && !LabelPicker && !ChoiceMenu"),
            Mode::Go => Some("MailApp && !Input && !RowMenu && !Settings && !Composer && !LabelPicker && !ChoiceMenu"),
            Mode::Search => Some("SearchBox > Input"),
            Mode::Writing => Some("Composer"),
            Mode::Recipients => Some("Recipients > Input"),
            Mode::Countdown => Some("Countdown"),
            Mode::Sweep => Some("Sweep"),
            Mode::Menu => Some("RowMenu"),
            Mode::Settings => Some("Settings"),
            Mode::Body => Some("ComposeBody > Input"),
            Mode::Labels => Some("LabelPicker > Input"),
            Mode::Choices => Some("ChoiceMenu > Input"),
            Mode::LabelEdit => Some("LabelRename > Input"),
            Mode::Global => None,
        }
    }
}

/// What the hint bar and the `g` menu show for one shortcut.
#[derive(Clone, Debug)]
pub struct Hint {
    pub mode: Mode,
    /// Key(s) as drawn in the chip, e.g. "j/k" or "⏎".
    pub keys: String,
    pub label: &'static str,
    /// The action it describes (None for a prefix like `g`).
    action: Option<String>,
}

/// The table: bindings to register, and hints to draw.
pub struct Shortcuts {
    /// (keys, action, where) after keymap.toml.
    entries: Vec<(String, Box<dyn Action>, Mode)>,
    pub hints: Vec<Hint>,
    /// Lines of keymap.toml that couldn't be used.
    pub errors: Vec<String>,
}

impl Shortcuts {
    /// The key bindings to register. A key gpui can't read is skipped and
    /// added to `errors`.
    pub fn bindings(&mut self, cx: &App) -> Vec<KeyBinding> {
        let mut out = Vec::new();
        for (keys, action, mode) in &self.entries {
            let context = mode.context().map(|c| std::rc::Rc::new(KeyBindingContextPredicate::parse(c).expect("built-in context")));
            match KeyBinding::load(keys, action.boxed_clone(), context, false, None, cx) {
                Ok(b) => out.push(b),
                Err(_) => self.errors.push(format!("keymap.toml: \"{keys}\" isn't a key Wax understands")),
            }
        }
        out
    }
}

impl Global for Shortcuts {}

/// One default shortcut.
struct Def {
    keys: &'static str,
    action: Box<dyn Action>,
    mode: Mode,
}

struct Builder {
    defs: Vec<Def>,
    hints: Vec<Hint>,
}

impl Builder {
    /// Bind `keystrokes` to `action` in `mode`, and show `keys`/`label` in
    /// the hints when `hint` is given.
    fn bind<A: Action>(&mut self, keystrokes: &'static str, action: A, mode: Mode, hint: Option<(&'static str, &'static str)>) -> &mut Self {
        let name = action_name(&action);
        self.defs.push(Def { keys: keystrokes, action: Box::new(action), mode });
        if let Some((keys, label)) = hint {
            self.hints.push(Hint { mode, keys: keys.into(), label, action: Some(name) });
        }
        self
    }

    /// A hint with no binding of its own (e.g. a prefix key).
    fn hint(&mut self, mode: Mode, keys: &'static str, label: &'static str) -> &mut Self {
        self.hints.push(Hint { mode, keys: keys.into(), label, action: None });
        self
    }
}

/// "mail::ToggleRead" → "toggle_read".
fn action_name(action: &dyn Action) -> String {
    let name = action.name().rsplit("::").next().unwrap_or("");
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// "cmd-shift-a" → "⌘⇧A", for hint chips.
fn pretty(keys: &str) -> String {
    keys.split_whitespace()
        .map(|stroke| {
            let mut out = String::new();
            let parts: Vec<&str> = stroke.split('-').collect();
            for (i, part) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                out.push_str(&match (*part, last) {
                    ("cmd", false) => "⌘".into(),
                    ("shift", false) => "⇧".into(),
                    ("alt", false) => "⌥".into(),
                    ("ctrl", false) => "⌃".into(),
                    ("enter", _) => "⏎".into(),
                    ("escape", _) => "esc".into(),
                    ("backspace", _) => "⌫".into(),
                    ("up", _) => "↑".into(),
                    ("down", _) => "↓".into(),
                    (p, _) if parts.len() > 1 => p.to_uppercase(),
                    (p, _) => p.to_string(),
                });
            }
            out
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize(keys: &str) -> String {
    keys.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

/// keymap.toml's lines: (where, keys, action name), plus lines it couldn't read.
fn parse_user(text: &str) -> (Vec<(Mode, String, String)>, Vec<String>) {
    let mut out = Vec::new();
    let mut errors = Vec::new();
    let mut mode = None;
    for (n, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            mode = Mode::ALL.iter().copied().find(|m| m.section() == name.trim());
            if mode.is_none() {
                errors.push(format!("keymap.toml line {}: no section called [{}]", n + 1, name.trim()));
            }
            continue;
        }
        let unquote = |s: &str| s.trim().trim_matches('"').to_string();
        let Some((k, v)) = line.split_once('=') else {
            errors.push(format!("keymap.toml line {}: expected \"keys\" = \"action\"", n + 1));
            continue;
        };
        let v = v.split('#').next().unwrap_or("");
        match mode {
            Some(m) => out.push((m, normalize(&unquote(k)), unquote(v))),
            None => errors.push(format!("keymap.toml line {}: put it under a section such as [list]", n + 1)),
        }
    }
    (out, errors)
}

pub fn keymap_file() -> std::path::PathBuf {
    crate::files::config_dir().join("keymap.toml")
}

/// keymap.toml with every default listed, commented out.
fn default_file(defs: &[Def]) -> String {
    let mut out = String::from(
        "# Wax keyboard shortcuts. Wax picks up changes when you save.\n\
         # Uncomment a line and change the keys, or add your own under a section.\n\
         # Keys: \"e\", \"shift-r\", \"cmd-shift-a\", \"g i\" (one key, then another).\n\
         # \"none\" switches a key off: \"s\" = \"none\"\n",
    );
    for mode in Mode::ALL {
        let here: Vec<&Def> = defs.iter().filter(|d| d.mode == mode).collect();
        if here.is_empty() {
            continue;
        }
        out.push_str(&format!("\n# {}\n[{}]\n", mode.about(), mode.section()));
        for d in here {
            out.push_str(&format!("# \"{}\" = \"{}\"\n", d.keys, action_name(d.action.as_ref())));
        }
    }
    out
}

/// The shortcuts, with keymap.toml applied (written first if missing).
pub fn keymap() -> Shortcuts {
    let (defs, hints) = defaults();
    let file = keymap_file();
    if !file.exists() {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        std::fs::write(&file, default_file(&defs)).ok();
    }
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    build(defs, hints, &text)
}

fn build(defs: Vec<Def>, mut hints: Vec<Hint>, user_text: &str) -> Shortcuts {
    let (user, mut errors) = parse_user(user_text);
    // Every action by name, from the defaults.
    let mut known: Vec<(String, Box<dyn Action>)> = Vec::new();
    for d in &defs {
        let name = action_name(d.action.as_ref());
        if !known.iter().any(|(n, _)| *n == name) {
            known.push((name, d.action.boxed_clone()));
        }
    }
    let mut entries: Vec<(String, Box<dyn Action>, Mode)> = Vec::new();
    for d in &defs {
        let replaced = user.iter().any(|(m, k, _)| *m == d.mode && *k == normalize(d.keys));
        if !replaced {
            entries.push((d.keys.to_string(), d.action.boxed_clone(), d.mode));
        }
    }
    let mut added: Vec<(Mode, String, String)> = Vec::new();
    for (mode, keys, name) in user {
        if name == "none" {
            continue;
        }
        let Some((_, action)) = known.iter().find(|(n, _)| *n == name) else {
            errors.push(format!("keymap.toml: no action called \"{name}\" (see the commented list)"));
            continue;
        };
        entries.push((keys.clone(), action.boxed_clone(), mode));
        added.push((mode, keys, name));
    }
    // Hint chips follow their action to its new key; a hint whose only key
    // was switched off disappears.
    let bound = |mode: Mode, action: &str, defs: &[Def]| {
        defs.iter().any(|d| d.mode == mode && action_name(d.action.as_ref()) == action)
    };
    hints.retain_mut(|h| {
        let Some(action) = h.action.clone() else { return true };
        if let Some((_, keys, _)) = added.iter().find(|(m, _, a)| *m == h.mode && *a == action) {
            h.keys = pretty(keys);
            return true;
        }
        bound(h.mode, &action, &defs) && entries.iter().any(|(_, a, m)| *m == h.mode && action_name(a.as_ref()) == action)
    });
    Shortcuts { entries, hints, errors }
}

fn defaults() -> (Vec<Def>, Vec<Hint>) {
    use Mode::*;
    let mut b = Builder { defs: Vec::new(), hints: Vec::new() };
    b
        // Message list
        .bind("j", SelectNext, List, Some(("j/k", "move")))
        .bind("k", SelectPrev, List, None)
        .bind("down", SelectNext, List, None)
        .bind("up", SelectPrev, List, None)
        .bind("u", ToggleRead, List, Some(("u", "read/unread")))
        .bind("shift-cmd-u", ToggleRead, List, None)
        .bind("/", FocusSearch, List, Some(("/", "search")))
        .bind("c", Compose, List, Some(("c", "compose")))
        .bind("r", Reply, List, Some(("r/a", "reply/all")))
        .bind("a", ReplyAll, List, None)
        .bind("f", Forward, List, Some(("f", "forward")))
        .bind("enter", OpenDraft, List, None)
        .bind("cmd-z", UndoSend, List, None)
        .bind("x", ToggleMark, List, Some(("x", "mark")))
        .bind("cmd-a", MarkAll, List, None)
        .bind("e", Archive, List, Some(("e", "archive")))
        .bind("shift-3", Delete, List, Some(("⌫", "delete")))
        .bind("backspace", Delete, List, None)
        .bind("delete", Delete, List, None)
        .bind("s", ToggleStar, List, None)
        .bind("i", ToggleImportant, List, None)
        .bind("b", SenderKey, List, None)
        .bind("shift-r", SenderToReading, List, None)
        .bind("shift-c", SenderToRecords, List, None)
        .bind("l", OpenLabels, List, Some(("l", "label")))
        .hint(List, "g", "go to…")
        .bind("escape", ClearSearch, List, None)
        // Go to folder: `g` then a letter (Gmail's keys)
        .bind("g i", GoInbox, Go, Some(("i", "Inbox")))
        .bind("g s", GoStarred, Go, Some(("s", "Starred")))
        .bind("g t", GoSent, Go, Some(("t", "Sent")))
        .bind("g d", GoDrafts, Go, Some(("d", "Drafts")))
        .bind("g a", GoAllMail, Go, Some(("a", "All Mail")))
        .bind("g w", OpenSweep, Go, Some(("w", "Sweep")))
        // Sweep
        .bind("j", SweepNext, Sweep, Some(("j/k", "move")))
        .bind("k", SweepPrev, Sweep, None)
        .bind("down", SweepNext, Sweep, None)
        .bind("up", SweepPrev, Sweep, None)
        .bind("escape", CloseSweep, Sweep, Some(("esc", "close")))
        // Right-click menu
        .bind("down", MenuNext, Menu, Some(("↑↓", "choose")))
        .bind("j", MenuNext, Menu, None)
        .bind("up", MenuPrev, Menu, None)
        .bind("k", MenuPrev, Menu, None)
        .bind("enter", MenuAccept, Menu, Some(("⏎", "do it")))
        .bind("escape", CloseMenu, Menu, Some(("esc", "close")))
        // Writing
        .bind("cmd-enter", SendMessage, Writing, Some(("⌘⏎", "send")))
        .bind("cmd-s", SaveDraft, Writing, Some(("⌘S", "save draft")))
        .bind("escape", CloseCompose, Writing, Some(("esc", "save draft & close")))
        .bind("shift-cmd-backspace", DiscardCompose, Writing, Some(("⇧⌘⌫", "discard")))
        .bind("cmd-b", FormatBold, Writing, Some(("⌘B/⌘I", "bold/italic")))
        .bind("cmd-i", FormatItalic, Writing, None)
        .bind("cmd-shift-x", FormatStrike, Writing, None)
        .bind("cmd-k", FormatLink, Writing, Some(("⌘K", "link")))
        .bind("cmd-e", FormatCode, Writing, None)
        .bind("cmd-shift-8", FormatBullets, Writing, Some(("⌘⇧8", "list")))
        .bind("cmd-shift-7", FormatNumbers, Writing, None)
        .bind("cmd-'", FormatQuote, Writing, None)
        .bind("cmd-shift-p", TogglePreview, Writing, Some(("⌘⇧P", "preview")))
        .bind("cmd-shift-s", CycleSignature, Writing, Some(("⌘⇧S", "signature")))
        .bind("cmd-shift-m", ToggleMono, Writing, None)
        .bind("cmd-shift-a", AttachFile, Writing, Some(("⌘⇧A", "attach")))
        .bind("cmd-shift-b", ShowBcc, Writing, Some(("⌘⇧B", "bcc on/off")))
        .bind("cmd-alt-t", FormatTable, Writing, None)
        .bind("cmd-v", SmartPaste, Body, None)
        .bind("tab", TableTab, Body, None)
        .bind("shift-tab", TableBackTab, Body, None)
        .hint(Writing, "tab", "next field")
        // Address suggestions (keys act only while the list is open)
        .bind("down", SuggestNext, Recipients, Some(("↑↓", "choose")))
        .bind("up", SuggestPrev, Recipients, None)
        .bind("enter", AcceptSuggestion, Recipients, Some(("⏎", "add")))
        .bind("tab", AcceptSuggestion, Recipients, None)
        .hint(Recipients, "esc", "dismiss")
        // Label picker
        .bind("down", LabelNext, Labels, Some(("↑↓", "choose")))
        .bind("up", LabelPrev, Labels, None)
        .bind("enter", LabelAccept, Labels, Some(("⏎", "add / remove")))
        .bind("shift-enter", LabelAcceptArchive, Labels, Some(("⇧⏎", "label & archive")))
        .bind("escape", CloseLabels, Labels, Some(("esc", "close")))
        .bind("down", ChoiceNext, Choices, Some(("↑↓", "choose")))
        .bind("up", ChoicePrev, Choices, None)
        .bind("enter", ChoiceAccept, Choices, Some(("⏎", "pick")))
        .bind("escape", CloseChoices, Choices, Some(("esc", "close")))
        .bind("enter", LabelAccept, LabelEdit, None)
        // Send countdown
        .bind("escape", UndoSend, Countdown, Some(("esc", "cancel")))
        .bind("cmd-z", UndoSend, Countdown, None)
        .bind("enter", SendNow, Countdown, Some(("⏎", "send now")))
        // Search box
        .bind("enter", FocusList, Search, Some(("⏎", "results")))
        .bind("down", FocusList, Search, None)
        .bind("escape", ClearSearch, Search, Some(("esc", "clear")))
        // Settings
        .bind("escape", CloseSettings, Settings, Some(("esc", "close")))
        // Anywhere
        .bind("cmd-,", OpenSettings, Global, None)
        .bind("cmd-f", FocusSearch, Global, None)
        .bind("cmd-/", ToggleHints, Global, Some(("⌘/", "hide hints")))
        .bind("cmd-q", Quit, Global, None);
    (b.defs, b.hints)
}

#[cfg(test)]
mod tests {
    use super::{action_name, parse_user, pretty, Mode, ToggleRead};

    #[test]
    fn names_and_pretty_keys() {
        assert_eq!(action_name(&ToggleRead), "toggle_read");
        assert_eq!(pretty("cmd-shift-a"), "⌘⇧A");
        assert_eq!(pretty("g i"), "g i");
        let (user, errors) = parse_user("[list]\n\"E\" = \"archive\" # note\n\"s\" = \"none\"\n[nope]\nx = y\n");
        assert_eq!(user, vec![(Mode::List, "e".into(), "archive".into()), (Mode::List, "s".into(), "none".into())]);
        assert_eq!(errors.len(), 2);
    }
}

/// Key bindings that aren't Wax's (gpui-kit's widgets), kept across reloads.
pub struct BaseBindings(pub Vec<KeyBinding>);

impl Global for BaseBindings {}

/// (Re)load keymap.toml and register every binding. Returns the lines of
/// keymap.toml that couldn't be used.
pub fn apply(cx: &mut App) -> Vec<String> {
    let mut shortcuts = keymap();
    let ours = shortcuts.bindings(cx);
    let base = cx.try_global::<BaseBindings>().map(|b| b.0.clone()).unwrap_or_default();
    cx.clear_key_bindings();
    cx.bind_keys(base);
    cx.bind_keys(ours);
    let errors = shortcuts.errors.clone();
    cx.set_global(shortcuts);
    errors
}
