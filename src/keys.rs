//! Every keyboard shortcut, defined once.
//!
//! The same table produces the key bindings, the hint bar at the bottom
//! of the window and the `g` menu, so they can never drift apart.

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
        FileReading,
        FileRecords,
        ScreenOut,
        KeepSender,
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
    /// Typing a label name in Settings → Labels.
    LabelEdit,
    /// Anywhere in the window.
    Global,
}

impl Mode {
    fn context(self) -> Option<&'static str> {
        match self {
            // Single-letter keys must not fire while typing in search.
            // ... nor while a menu is open (it has its own keys).
            // ... and never behind Settings, the Sweep or the composer, where
            // a stray letter would act on a message you can't see.
            Mode::List => Some("MailApp && !Input && !RowMenu && !Settings && !Sweep && !Composer && !LabelPicker"),
            Mode::Go => Some("MailApp && !Input && !RowMenu && !Settings && !Composer && !LabelPicker"),
            Mode::Search => Some("SearchBox > Input"),
            Mode::Writing => Some("Composer"),
            Mode::Recipients => Some("Recipients > Input"),
            Mode::Countdown => Some("Countdown"),
            Mode::Sweep => Some("Sweep"),
            Mode::Menu => Some("RowMenu"),
            Mode::Settings => Some("Settings"),
            Mode::Body => Some("ComposeBody > Input"),
            Mode::Labels => Some("LabelPicker > Input"),
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
    pub keys: &'static str,
    pub label: &'static str,
}

/// The table: bindings to register, and hints to draw.
pub struct Shortcuts {
    pub bindings: Vec<KeyBinding>,
    pub hints: Vec<Hint>,
}

impl Global for Shortcuts {}

struct Builder(Shortcuts);

impl Builder {
    /// Bind `keystrokes` to `action` in `mode`, and show `keys`/`label` in
    /// the hints when `hint` is given.
    fn bind<A: Action>(&mut self, keystrokes: &'static str, action: A, mode: Mode, hint: Option<(&'static str, &'static str)>) -> &mut Self {
        self.0.bindings.push(KeyBinding::new(keystrokes, action, mode.context()));
        if let Some((keys, label)) = hint {
            self.0.hints.push(Hint { mode, keys, label });
        }
        self
    }

    /// A hint with no binding of its own (e.g. a prefix key).
    fn hint(&mut self, mode: Mode, keys: &'static str, label: &'static str) -> &mut Self {
        self.0.hints.push(Hint { mode, keys, label });
        self
    }
}

pub fn keymap() -> Shortcuts {
    use Mode::*;
    let mut b = Builder(Shortcuts { bindings: Vec::new(), hints: Vec::new() });
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
        .bind("r", FileReading, Sweep, Some(("r", "Reading")))
        .bind("c", FileRecords, Sweep, Some(("c", "Records")))
        .bind("x", ScreenOut, Sweep, Some(("x", "screen out")))
        .bind("i", KeepSender, Sweep, Some(("i", "keep")))
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
    b.0
}
