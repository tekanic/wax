use super::*;
use std::time::Duration;
use wasm_host::IssueKind;

const SHOWN: usize = 6;

#[derive(Clone, Debug)]
pub struct Found {
    pub start: usize,
    pub end: usize,
    pub word: String,
    pub spelling: bool,
    pub message: String,
    pub suggestions: Vec<String>,
}

pub fn dictionary_file() -> std::path::PathBuf {
    crate::files::config_dir().join("dictionary.txt")
}

pub fn personal_words() -> std::collections::HashSet<String> {
    std::fs::read_to_string(dictionary_file())
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_lowercase())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn name_words(recipients: &str) -> std::collections::HashSet<String> {
    recipients
        .split(|c: char| !c.is_alphanumeric() && c != '\'' && c != '-')
        .filter(|w| w.len() > 1)
        .map(str::to_lowercase)
        .collect()
}

impl MailApp {
    pub fn schedule_spellcheck(&mut self, cx: &mut Context<Self>) {
        self.spell_gen += 1;
        let generation = self.spell_gen;
        let Some(host) = self.wasm.clone() else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            cx.background_executor().timer(Duration::from_millis(700)).await;
            let Some((text, known)) = this
                .update(cx, |this, cx| {
                    let c = this.composer.as_ref().filter(|_| this.spell_gen == generation)?;
                    let text = c.body.read(cx).value().to_string();
                    let people = format!("{} {} {}", c.to.read(cx).value(), c.cc.read(cx).value(), c.bcc.read(cx).value());
                    let mut known = personal_words();
                    known.extend(name_words(&people));
                    Some((text, known))
                })
                .ok()
                .flatten()
            else {
                return;
            };
            let checked = text.clone();
            let (found, problems) = cx
                .background_executor()
                .spawn(async move {
                    let mut h = host.lock().unwrap();
                    if !h.checks_drafts() {
                        return (Vec::new(), Vec::new());
                    }
                    h.check_draft(&checked)
                })
                .await;
            for p in &problems {
                errlog::record("", p);
            }
            let found: Vec<Found> = found
                .into_iter()
                .flat_map(|(_, issues)| issues)
                .map(|i| Found {
                    start: i.start as usize,
                    end: i.end as usize,
                    word: text[i.start as usize..i.end as usize].to_string(),
                    spelling: i.kind == IssueKind::Spelling,
                    message: i.message,
                    suggestions: i.suggestions,
                })
                .filter(|f| !(f.spelling && known.contains(&f.word.to_lowercase())))
                .collect();
            this.update(cx, |this, cx| {
                let same = this.composer.as_ref().is_some_and(|c| *c.body.read(cx).value() == *text);
                if this.spell_gen == generation && same {
                    this.spell = found;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn fix_spelling(&mut self, ix: usize, with: String, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(f), Some(c)) = (self.spell.get(ix).cloned(), &self.composer) else { return };
        let text = c.body.read(cx).value().to_string();
        if text.get(f.start..f.end) != Some(f.word.as_str()) {
            self.schedule_spellcheck(cx);
            return;
        }
        let new = format!("{}{}{}", &text[..f.start], with, &text[f.end..]);
        let at = f.start + with.len();
        self.apply_body_edit(new, at..at, window, cx);
        self.spell.remove(ix);
        self.schedule_spellcheck(cx);
    }

    fn add_to_dictionary(&mut self, word: String, cx: &mut Context<Self>) {
        let path = dictionary_file();
        let mut text = std::fs::read_to_string(&path).unwrap_or_else(|_| "# Words Wax's spelling check accepts, one per line.\n".into());
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&word);
        text.push('\n');
        if let Err(e) = std::fs::write(&path, text) {
            self.status = format!("Could not save {}: {e}", path.display()).into();
            self.status_is_error = true;
        }
        let lower = word.to_lowercase();
        self.spell.retain(|f| !(f.spelling && f.word.to_lowercase() == lower));
        cx.notify();
    }

    pub fn render_spelling(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let c = self.composer.as_ref()?;
        if self.spell.is_empty() || c.preview {
            return None;
        }
        let chip = |id: SharedString| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .h(px(24.))
                .rounded_md()
                .bg(pal().button)
                .text_xs()
                .cursor_pointer()
                .hover(|d| d.bg(pal().button_hover))
        };
        let items = self.spell.iter().take(SHOWN).enumerate().map(|(ix, f)| {
            let wrong = div().text_color(if f.spelling { pal().error } else { rgb(0xd09a2a).into() }).child(f.word.clone());
            let mut row = div().flex().items_center().gap_1();
            match f.suggestions.first().cloned() {
                Some(best) => {
                    let label = best.clone();
                    row = row.child(
                        chip(format!("spell-fix-{ix}").into())
                            .tooltip({
                                let tip = f.message.clone();
                                move |w, cx| Tooltip::new(tip.clone()).build(w, cx)
                            })
                            .on_click(cx.listener(move |this, _, w, cx| this.fix_spelling(ix, best.clone(), w, cx)))
                            .child(wrong)
                            .child(div().text_color(pal().muted).child("→"))
                            .child(div().text_color(pal().text).child(label)),
                    );
                }
                None => {
                    row = row.child(
                        chip(format!("spell-word-{ix}").into())
                            .tooltip({
                                let tip = f.message.clone();
                                move |w, cx| Tooltip::new(tip.clone()).build(w, cx)
                            })
                            .child(wrong),
                    );
                }
            }
            if f.spelling {
                let word = f.word.clone();
                row = row.child(
                    div()
                        .id(SharedString::from(format!("spell-learn-{ix}")))
                        .px_1()
                        .text_xs()
                        .text_color(pal().muted)
                        .cursor_pointer()
                        .hover(|d| d.text_color(pal().accent))
                        .tooltip(|w, cx| Tooltip::new("Add to dictionary").build(w, cx))
                        .on_click(cx.listener(move |this, _, _, cx| this.add_to_dictionary(word.clone(), cx)))
                        .child("+"),
                );
            }
            row
        });
        let more = self.spell.len().saturating_sub(SHOWN);
        Some(
            div()
                .flex_none()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(div().text_xs().text_color(pal().muted).child("Check"))
                .children(items)
                .when(more > 0, |d| d.child(div().text_xs().text_color(pal().muted).child(format!("and {more} more")))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::name_words;

    #[test]
    fn recipients_names_count_as_words() {
        let w = name_words("Priya Shah <priya@lumenhouse.studio>, O'Neil-Ray <x@y.z>");
        assert!(w.contains("priya") && w.contains("shah") && w.contains("o'neil-ray"));
    }
}
