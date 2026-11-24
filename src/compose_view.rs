//! The composer: input fields for one message being written, shown in the
//! reader pane. Pure state; the app wires up send, save and discard.

use crate::compose::{Draft, Kind};
use gpui_kit::component::input::{InputState, Position, TextareaState};
use gpui_kit::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

pub struct ComposerState {
    pub kind: Kind,
    pub to: Entity<InputState>,
    pub cc: Entity<InputState>,
    pub bcc: Entity<InputState>,
    /// The Bcc line is open (it opens by itself when a draft has one).
    pub show_bcc: bool,
    pub subject: Entity<InputState>,
    pub body: Entity<TextareaState>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub server_draft: Option<(String, String)>,
    /// Shown above the fields (e.g. "Add at least one recipient").
    pub error: Option<SharedString>,
    /// Address suggestions for the word being typed in To or Cc.
    pub suggestions: Vec<(String, String)>,
    pub suggest_ix: usize,
    pub suggest_for: Option<RecipientField>,
    /// Showing the rendered message instead of the Markdown source.
    pub preview: bool,
    /// Holds focus while previewing, so the composer's keys still work.
    pub preview_focus: FocusHandle,
    pub mono: bool,
    pub signature: Option<i64>,
    pub from: Option<(String, String)>,
    pub attachments: Vec<std::path::PathBuf>,
    pub account: String,
}

impl ComposerState {
    pub fn new<T: 'static>(draft: &Draft, window: &mut Window, cx: &mut Context<T>) -> Self {
        let line = |placeholder: &'static str, value: &str, window: &mut Window, cx: &mut Context<T>| {
            let value = value.to_string();
            cx.new(|cx| {
                let mut s = InputState::new(window, cx).placeholder(placeholder);
                s.set_value(value, window, cx);
                s
            })
        };
        let to = line("Recipients", &draft.to, window, cx);
        let cc = line("", &draft.cc, window, cx);
        let bcc = line("Hidden from the other recipients", &draft.bcc, window, cx);
        let subject = line("Subject", &draft.subject, window, cx);
        let body_text = draft.body.clone();
        let body = cx.new(|cx| {
            let mut s = TextareaState::new(window, cx);
            s.set_value(body_text, window, cx);
            s
        });
        Self {
            kind: draft.kind,
            to,
            cc,
            show_bcc: !draft.bcc.trim().is_empty(),
            bcc,
            subject,
            body,
            in_reply_to: draft.in_reply_to.clone(),
            references: draft.references.clone(),
            server_draft: draft.server_draft.clone(),
            error: None,
            suggestions: Vec::new(),
            preview: false,
            preview_focus: cx.focus_handle(),
            mono: draft.mono,
            signature: draft.signature,
            from: draft.from.clone(),
            attachments: draft.attachments.clone(),
            account: draft.account.clone(),
            suggest_ix: 0,
            suggest_for: None,
        }
    }

    /// Put the cursor where you'd start typing: the body for replies (above
    /// the quote), the To field otherwise.
    pub fn focus_first<T: 'static>(&self, window: &mut Window, cx: &mut Context<T>) {
        match self.kind {
            Kind::Reply | Kind::ReplyAll => {
                self.body.update(cx, |s, cx| s.set_cursor_position(Position::new(0, 0), window, cx));
            }
            Kind::New | Kind::Forward => {
                if self.to.read(cx).value().is_empty() {
                    self.to.update(cx, |s, cx| s.focus(window, cx));
                } else {
                    self.body.update(cx, |s, cx| s.set_cursor_position(Position::new(0, 0), window, cx));
                }
            }
        }
    }

    pub fn field(&self, f: RecipientField) -> &Entity<InputState> {
        match f {
            RecipientField::To => &self.to,
            RecipientField::Cc => &self.cc,
            RecipientField::Bcc => &self.bcc,
        }
    }

    /// The address being typed: text after the last separator.
    pub fn current_token(&self, f: RecipientField, cx: &App) -> String {
        let text = self.field(f).read(cx).value().to_string();
        let start = text.rfind([',', ';']).map(|i| i + 1).unwrap_or(0);
        text[start..].trim().to_string()
    }

    /// Replace the word being typed with the chosen address.
    pub fn accept<T: 'static>(&mut self, window: &mut Window, cx: &mut Context<T>) -> bool {
        let (Some(f), Some((name, email))) = (self.suggest_for, self.suggestions.get(self.suggest_ix).cloned()) else {
            return false;
        };
        let entity = self.field(f).clone();
        let text = entity.read(cx).value().to_string();
        let keep = text.rfind([',', ';']).map(|i| text[..=i].to_string()).unwrap_or_default();
        let sep = if keep.is_empty() { "" } else { " " };
        let new = format!("{keep}{sep}{}, ", crate::compose::display_address(&name, &email));
        entity.update(cx, |s, cx| {
            s.set_value(new, window, cx);
            s.focus(window, cx);
        });
        self.clear_suggestions();
        true
    }

    pub fn clear_suggestions(&mut self) {
        self.suggestions.clear();
        self.suggest_ix = 0;
        self.suggest_for = None;
    }

    pub fn draft(&self, cx: &App) -> Draft {
        Draft {
            kind: self.kind,
            to: self.to.read(cx).value().to_string(),
            cc: self.cc.read(cx).value().to_string(),
            bcc: self.bcc.read(cx).value().to_string(),
            subject: self.subject.read(cx).value().to_string(),
            body: self.body.read(cx).value().to_string(),
            in_reply_to: self.in_reply_to.clone(),
            references: self.references.clone(),
            server_draft: self.server_draft.clone(),
            mono: self.mono,
            signature: self.signature,
            from: self.from.clone(),
            attachments: self.attachments.clone(),
            account: self.account.clone(),
        }
    }
}
