use super::*;
use std::ops::Range;

pub const CURSOR: &str = "cursor";

#[derive(Default, Clone)]
pub struct Smart {
    pub first_name: Option<String>,
    pub full_name: Option<String>,
    pub me: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub name: String,
    pub range: Range<usize>,
}

#[derive(Debug, PartialEq)]
pub struct Expanded {
    pub text: String,
    pub slots: Vec<Slot>,
    pub end: Option<usize>,
}

fn smart(name: &str, s: &Smart) -> Option<String> {
    let today = chrono::Local::now();
    match name.to_lowercase().as_str() {
        "today" | "date" => Some(today.format("%A, %B %-d").to_string()),
        "tomorrow" => Some((today + chrono::Duration::days(1)).format("%A, %B %-d").to_string()),
        "first name" | "firstname" => s.first_name.clone(),
        "recipient" | "their name" => s.full_name.clone(),
        "me" | "my name" => s.me.clone(),
        _ => None,
    }
}

pub fn expand(body: &str, s: &Smart) -> Expanded {
    let mut text = String::new();
    let mut slots = Vec::new();
    let mut end = None;
    let mut rest = body;
    while let Some(open) = rest.find("{{") {
        let Some(close) = rest[open..].find("}}").map(|c| open + c) else { break };
        text.push_str(&rest[..open]);
        let inner = rest[open + 2..close].trim();
        let (name, default) = match inner.split_once(':') {
            Some((n, d)) => (n.trim(), Some(d.trim())),
            None => (inner, None),
        };
        if name.eq_ignore_ascii_case(CURSOR) {
            end = Some(text.len());
        } else if let Some(v) = smart(name, s) {
            text.push_str(&v);
        } else if !name.is_empty() {
            let shown = default.filter(|d| !d.is_empty()).unwrap_or(name);
            let start = text.len();
            text.push_str(shown);
            slots.push(Slot { name: name.to_lowercase(), range: start..text.len() });
        }
        rest = &rest[close + 2..];
    }
    text.push_str(rest);
    Expanded { text, slots, end }
}

pub struct Fill {
    pub block: String,
    pub slots: Vec<Slot>,
    pub visited: Vec<bool>,
    pub current: usize,
    pub end: usize,
    pub text: String,
}

impl Fill {
    pub fn track(&mut self, new: &str) -> bool {
        if new == self.text {
            return true;
        }
        let (old_r, new_r) = formatting::diff_range(&self.text, new);
        let delta = new_r.len() as isize - old_r.len() as isize;
        let shift = |x: usize| (x as isize + delta) as usize;
        let cur = self.slots[self.current].range.clone();
        let inside_current = old_r.start >= cur.start && old_r.end <= cur.end;
        if !inside_current {
            let touches = self.slots.iter().any(|s| old_r.start < s.range.end && s.range.start < old_r.end);
            if touches || (old_r.start < self.end && self.end < old_r.end) {
                return false;
            }
        }
        for (i, s) in self.slots.iter_mut().enumerate() {
            if i == self.current && inside_current {
                s.range.end = shift(s.range.end);
            } else if s.range.start >= old_r.end && !(old_r.is_empty() && s.range.start == old_r.start && i < self.current) {
                s.range = shift(s.range.start)..shift(s.range.end);
            }
        }
        if self.end >= old_r.end {
            self.end = shift(self.end);
        }
        self.text = new.to_string();
        true
    }

    fn replace(&mut self, i: usize, value: &str) {
        let r = self.slots[i].range.clone();
        let delta = value.len() as isize - r.len() as isize;
        self.text.replace_range(r.clone(), value);
        for s in self.slots.iter_mut() {
            if s.range.start >= r.end && s.range.start > r.start {
                s.range = (s.range.start as isize + delta) as usize..(s.range.end as isize + delta) as usize;
            }
        }
        self.slots[i].range = r.start..r.start + value.len();
        if self.end >= r.end {
            self.end = (self.end as isize + delta) as usize;
        }
    }

    pub fn step(&mut self, back: bool) -> Option<Range<usize>> {
        let here = self.slots[self.current].clone();
        let value = self.text[here.range.clone()].to_string();
        self.visited[self.current] = true;
        for i in 0..self.slots.len() {
            if i != self.current && self.slots[i].name == here.name && !self.visited[i] {
                self.replace(i, &value);
                self.visited[i] = true;
            }
        }
        let next = if back {
            self.current.checked_sub(1)
        } else {
            (self.current + 1..self.slots.len()).find(|&i| !self.visited[i])
        };
        let i = next?;
        self.current = i;
        Some(self.slots[i].range.clone())
    }
}

pub fn shortcut_before(text: &str, cursor: usize) -> Option<Range<usize>> {
    let before = &text[..cursor];
    let start = before.rfind(char::is_whitespace).map(|i| i + before[i..].chars().next().map_or(1, char::len_utf8)).unwrap_or(0);
    (start < cursor).then_some(start..cursor)
}

impl MailApp {
    pub fn smart_values(&self, cx: &App) -> Smart {
        let Some(c) = &self.composer else { return Smart::default() };
        let to = c.to.read(cx).value().to_string();
        let first = crate::compose::parse_lenient(&to).into_iter().next();
        let full = first.as_ref().map(|(n, _)| n.trim().to_string()).filter(|n| !n.is_empty() && !n.contains('@'));
        let account = if c.account.is_empty() { self.account.to_string() } else { c.account.clone() };
        let me = c
            .from
            .as_ref()
            .map(|(n, _)| n.clone())
            .or_else(|| self.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(&account)).map(|a| a.name.clone()))
            .filter(|n| !n.trim().is_empty());
        Smart { first_name: full.as_ref().and_then(|n| n.split_whitespace().next().map(str::to_string)), full_name: full, me }
    }

    pub fn start_fill(&mut self, block: &blocks::Block, at: usize, replace: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(c) = &self.composer else { return false };
        let ex = expand(&block.body, &self.smart_values(cx));
        let old = c.body.read(cx).value().to_string();
        let base = format!("{}{}", &old[..replace.start], &old[replace.end..]);
        let at = if replace.is_empty() { at } else { replace.start };
        let (out, start) = formatting::insert_block(&base, at, &ex.text);
        let slots: Vec<Slot> = ex.slots.iter().map(|s| Slot { name: s.name.clone(), range: start + s.range.start..start + s.range.end }).collect();
        let end = start + ex.end.unwrap_or(ex.text.len());
        let focus = c.body.read(cx).focus_handle(cx);
        if slots.is_empty() {
            self.fill = None;
            self.apply_body_edit(out, end..end, window, cx);
            focus.focus(window, cx);
            return true;
        }
        let first = slots[0].range.clone();
        self.fill = Some(Fill { block: block.name.clone(), visited: vec![false; slots.len()], slots, current: 0, end, text: out.clone() });
        self.apply_body_edit(out, first, window, cx);
        focus.focus(window, cx);
        true
    }

    pub fn fill_changed(&mut self, cx: &mut Context<Self>) {
        let Some(c) = &self.composer else { return };
        let text = c.body.read(cx).value().to_string();
        if let Some(f) = &mut self.fill {
            if !f.track(&text) {
                self.fill = None;
                cx.notify();
            }
        }
    }

    pub fn fill_step(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(c) = &self.composer else { return false };
        let text = c.body.read(cx).value().to_string();
        let Some(f) = &mut self.fill else { return false };
        if !f.track(&text) {
            self.fill = None;
            return false;
        }
        let next = f.step(back);
        let new = f.text.clone();
        let select = match next {
            Some(r) => r,
            None => {
                let end = f.end.min(new.len());
                self.fill = None;
                let done = crate::table::calculate_all(&new);
                let end = (end as isize + done.len() as isize - new.len() as isize).clamp(0, done.len() as isize) as usize;
                self.apply_body_edit(done, end..end, window, cx);
                cx.notify();
                return true;
            }
        };
        self.apply_body_edit(new, select, window, cx);
        cx.notify();
        true
    }

    pub fn body_tab(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.fill_step(back, window, cx) {
            return;
        }
        if !back {
            if let Some(c) = &self.composer {
                let s = c.body.read(cx);
                let text = s.value().to_string();
                let cursor = s.selected_range();
                if cursor.is_empty() {
                    if let Some(word) = shortcut_before(&text, cursor.end) {
                        let typed = &text[word.clone()];
                        if let Some(b) = self.blocks.iter().find(|b| b.shortcut.as_deref() == Some(typed)).cloned() {
                            self.start_fill(&b, word.start, word, window, cx);
                            return;
                        }
                    }
                }
            }
        }
        self.table_tab(back, window, cx);
    }

    pub fn render_fill_hint(&self) -> Option<impl IntoElement> {
        let f = self.fill.as_ref()?;
        let distinct: Vec<&String> = f.slots.iter().map(|s| &s.name).fold(Vec::new(), |mut v, n| {
            if !v.contains(&n) {
                v.push(n);
            }
            v
        });
        let here = &f.slots[f.current].name;
        let pos = distinct.iter().position(|n| *n == here).unwrap_or(0) + 1;
        Some(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .text_xs()
                .child(div().text_color(pal().muted).child(format!("Filling \u{201c}{}\u{201d}", f.block)))
                .child(div().px_2().py_0p5().rounded_md().bg(pal().accent).text_color(pal().on_accent).child(format!("{pos} of {} \u{00b7} {here}", distinct.len())))
                .child(div().text_color(pal().muted).font_family("Menlo").child("tab next \u{00b7} \u{21e7}tab back")),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{expand, shortcut_before, Fill, Smart};

    fn smart() -> Smart {
        Smart { first_name: Some("Priya".into()), full_name: Some("Priya Shah".into()), me: Some("Alex Morgan".into()) }
    }

    #[test]
    fn expands_placeholders_and_smart_values() {
        let ex = expand("Hi {{first name}}, the {{item}} costs {{price:$0.00}}. {{item}} ships {{when}}.{{cursor}}\n{{me}}", &smart());
        assert!(ex.text.starts_with("Hi Priya, the item costs $0.00. item ships when."), "{}", ex.text);
        assert!(ex.text.ends_with("\nAlex Morgan"));
        let names: Vec<&str> = ex.slots.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["item", "price", "item", "when"]);
        assert_eq!(&ex.text[ex.slots[1].range.clone()], "$0.00");
        assert_eq!(&ex.text[..ex.end.unwrap()], "Hi Priya, the item costs $0.00. item ships when.");
        let none = expand("Hi {{first name}}", &Smart::default());
        assert_eq!(none.text, "Hi first name");
        assert_eq!(none.slots.len(), 1);
    }

    fn fill(text: &str) -> Fill {
        let ex = expand(text, &Smart::default());
        Fill { block: "t".into(), visited: vec![false; ex.slots.len()], slots: ex.slots, current: 0, end: ex.end.unwrap_or(ex.text.len()), text: ex.text }
    }

    fn typed(f: &mut Fill, value: &str) {
        let r = f.slots[f.current].range.clone();
        let new = format!("{}{}{}", &f.text[..r.start], value, &f.text[r.end..]);
        assert!(f.track(&new));
    }

    #[test]
    fn tab_walks_and_fills_repeats() {
        let mut f = fill("Dear {{client}}, your {{item}} for {{client}} is ready.{{cursor}} Bye");
        assert_eq!(&f.text[f.slots[0].range.clone()], "client");
        typed(&mut f, "Acme Ltd");
        let next = f.step(false).unwrap();
        assert_eq!(&f.text[next], "item");
        typed(&mut f, "catalog");
        assert!(f.step(false).is_none(), "the repeat was filled, so that's the end");
        assert_eq!(f.text, "Dear Acme Ltd, your catalog for Acme Ltd is ready. Bye");
        assert_eq!(&f.text[..f.end], "Dear Acme Ltd, your catalog for Acme Ltd is ready.");
    }

    #[test]
    fn typing_outside_the_block_keeps_places() {
        let mut f = fill("A {{x}} B {{y}}");
        let before = format!("Intro. {}", f.text);
        assert!(f.track(&before));
        assert_eq!(&f.text[f.slots[0].range.clone()], "x");
        assert_eq!(&f.text[f.slots[1].range.clone()], "y");
        let broken = f.text.replace("B y", "Bz");
        assert!(!f.track(&broken), "editing another placeholder by hand ends the fill");
    }

    #[test]
    fn shift_tab_goes_back() {
        let mut f = fill("{{a}} {{b}} {{c}}");
        f.step(false);
        f.step(false);
        let back = f.step(true).unwrap();
        assert_eq!(&f.text[back], "b");
    }

    #[test]
    fn finds_the_shortcut_word() {
        assert_eq!(shortcut_before("Thanks ;ty", 10), Some(7..10));
        assert_eq!(shortcut_before(";q", 2), Some(0..2));
        assert_eq!(shortcut_before("hi ", 3), None);
    }
}
