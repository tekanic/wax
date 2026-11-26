use std::ops::Range;

pub const WORDS: &str = include_str!("../dict/words.txt");
const MAX_ISSUES: usize = 60;
const KEYBOARD: [&str; 3] = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
const ABBREVIATIONS: [&str; 14] = ["e.g", "i.e", "etc", "vs", "approx", "mr", "mrs", "ms", "dr", "st", "no", "fig", "inc", "ltd"];
const DOUBLED_OK: [&str; 2] = ["had", "that"];

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Kind {
    Spelling,
    Grammar,
}

#[derive(Debug, PartialEq)]
pub struct Issue {
    pub range: Range<usize>,
    pub kind: Kind,
    pub message: String,
    pub suggestions: Vec<String>,
}

pub struct Dictionary<'a> {
    words: Vec<&'a str>,
}

impl<'a> Dictionary<'a> {
    pub fn new(text: &'a str) -> Self {
        Dictionary { words: text.lines().collect() }
    }

    pub fn has(&self, w: &str) -> bool {
        self.words.binary_search(&w).is_ok()
    }

    fn knows(&self, w: &str) -> bool {
        let w = w.replace('\u{2019}', "'");
        let base = w.strip_suffix("'s").unwrap_or(&w);
        let lower = base.to_lowercase();
        self.has(base) || self.has(&lower) || (capitalized(base) && self.has(&capitalize(&lower))) || self.has(&w)
    }
}

fn capitalized(w: &str) -> bool {
    let mut c = w.chars();
    c.next().is_some_and(char::is_uppercase) && c.all(|c| !c.is_uppercase())
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

fn skip(w: &str) -> bool {
    let letters = w.chars().filter(|c| c.is_alphabetic()).count();
    letters < 2
        || w.len() > 24
        || w.chars().any(|c| c.is_ascii_digit() || c == '_')
        || w.chars().filter(|c| c.is_uppercase()).count() > 1
        || w.chars().skip(1).any(char::is_uppercase)
}

fn hidden_spans(text: &str) -> Vec<Range<usize>> {
    let mut spans = Vec::new();
    let mut pos = 0;
    let mut fence: Option<usize> = None;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        let start = pos;
        pos += line.len();
        if t.starts_with("```") || t.starts_with("~~~") {
            match fence.take() {
                Some(s) => spans.push(s..pos),
                None => fence = Some(start),
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        let trimmed = t.trim_end();
        if (trimmed.starts_with("On ") && trimmed.ends_with("wrote:")) || trimmed.contains("Forwarded message") {
            spans.push(start..text.len());
            break;
        }
        if t.starts_with("    ") {
            spans.push(start..pos);
            continue;
        }
        let mut in_code: Option<usize> = None;
        for (i, c) in line.char_indices() {
            if c == '`' {
                match in_code.take() {
                    Some(s) => spans.push(start + s..start + i + 1),
                    None => in_code = Some(i),
                }
            }
        }
        for (i, _) in line.match_indices("](") {
            let end = line[i..].find(')').map(|e| i + e + 1).unwrap_or(line.len());
            spans.push(start + i..start + end);
        }
        let mut word_start = 0;
        for piece in line.split_inclusive(char::is_whitespace) {
            let w = piece.trim_end();
            if w.contains("://") || w.starts_with("www.") || w.contains('@') || w.contains('/') {
                spans.push(start + word_start..start + word_start + w.len());
            }
            word_start += piece.len();
        }
    }
    if let Some(s) = fence {
        spans.push(s..text.len());
    }
    spans
}

fn words(text: &str) -> Vec<(Range<usize>, &str)> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (k, &(i, c)) in chars.iter().enumerate() {
        let joiner = (c == '\'' || c == '\u{2019}' || c == '-')
            && start.is_some()
            && chars.get(k + 1).is_some_and(|&(_, n)| n.is_alphabetic());
        if is_word(c) || joiner {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push((s..i, &text[s..i]));
        }
    }
    if let Some(s) = start {
        out.push((s..text.len(), &text[s..]));
    }
    out
}

fn near(a: char, b: char) -> bool {
    KEYBOARD.iter().any(|row| {
        let (x, y) = (row.find(a), row.find(b));
        matches!((x, y), (Some(x), Some(y)) if x.abs_diff(y) == 1)
    }) || KEYBOARD.windows(2).any(|rows| {
        let (x, y) = (rows[0].find(a).or(rows[1].find(a)), rows[0].find(b).or(rows[1].find(b)));
        matches!((x, y), (Some(x), Some(y)) if x.abs_diff(y) <= 1) && rows[0].contains(a) != rows[0].contains(b)
    })
}

pub fn suggest(dict: &Dictionary, word: &str) -> Vec<String> {
    let lower = word.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    let n = chars.len();
    let letters: Vec<char> = "abcdefghijklmnopqrstuvwxyz'".chars().collect();
    let mut found: Vec<(u32, String)> = Vec::new();
    let mut try_word = |w: String, cost: u32| {
        let cap = capitalize(&w);
        let hit = if dict.has(&w) { Some(w) } else if dict.has(&cap) { Some(cap) } else { None };
        if let Some(h) = hit {
            if h.to_lowercase() != lower && !found.iter().any(|(_, f)| *f == h) {
                found.push((cost, h));
            }
        }
    };
    for i in 0..n.saturating_sub(1) {
        let mut c = chars.clone();
        c.swap(i, i + 1);
        try_word(c.iter().collect(), 1);
    }
    for i in 0..n {
        for &l in &letters {
            if l != chars[i] {
                let mut c = chars.clone();
                c[i] = l;
                try_word(c.iter().collect(), if near(chars[i], l) { 1 } else { 3 } + (i == 0) as u32 * 2);
            }
        }
    }
    for i in 0..n {
        let mut c = chars.clone();
        let removed = c.remove(i);
        let doubled = (i > 0 && chars[i - 1] == removed) || chars.get(i + 1) == Some(&removed);
        try_word(c.iter().collect(), if doubled { 1 } else { 2 });
    }
    for i in 0..=n {
        for &l in &letters {
            let mut c = chars.clone();
            c.insert(i, l);
            let doubles = (i > 0 && chars[i - 1] == l) || chars.get(i) == Some(&l);
            try_word(c.iter().collect(), if doubles { 0 } else { 2 });
        }
    }
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.len().abs_diff(n).cmp(&b.1.len().abs_diff(n))).then(a.1.cmp(&b.1)));
    let original_capital = capitalized(word);
    found
        .into_iter()
        .take(3)
        .map(|(_, w)| if original_capital { capitalize(&w) } else { w })
        .collect()
}

fn wants_an(next: &str) -> Option<bool> {
    let w = next.to_lowercase();
    let first = w.chars().next()?;
    if !first.is_alphabetic() || (next.len() > 1 && next.chars().all(|c| c.is_uppercase())) {
        return None;
    }
    let you = ["uni", "use", "usu", "uti", "ure", "uro", "ubiq", "eu", "ewe", "one", "once", "ufo", "uk", "us"];
    let silent_h = ["hour", "honest", "honor", "honour", "heir", "herb"];
    if you.iter().any(|p| w.starts_with(p)) {
        return Some(false);
    }
    if silent_h.iter().any(|p| w.starts_with(p)) {
        return Some(true);
    }
    Some("aeiou".contains(first))
}

fn sentence_end_before(text: &str, at: usize, previous: Option<&str>) -> bool {
    let before = text[..at].trim_end_matches([' ', '\t']);
    if before.len() == text[..at].len() {
        return false;
    }
    let Some(last) = before.chars().last() else { return false };
    if !matches!(last, '.' | '!' | '?') || before.ends_with("..") {
        return false;
    }
    !previous.is_some_and(|p| ABBREVIATIONS.contains(&p.to_lowercase().as_str()) || p.chars().count() == 1)
}

pub fn check(text: &str, dict: &Dictionary) -> Vec<Issue> {
    let hidden = hidden_spans(text);
    let shown = |r: &Range<usize>| !hidden.iter().any(|h| h.start < r.end && r.start < h.end);
    let all = words(text);
    let mut issues = Vec::new();
    for (k, (r, w)) in all.iter().enumerate() {
        if issues.len() >= MAX_ISSUES {
            break;
        }
        if !shown(r) {
            continue;
        }
        let prev = k.checked_sub(1).map(|p| &all[p]);
        let next = all.get(k + 1);
        let gap = |a: &Range<usize>, b: &Range<usize>| &text[a.end..b.start];
        if let Some((pr, pw)) = prev {
            if pw.eq_ignore_ascii_case(w) && gap(pr, r).chars().all(|c| c == ' ') && !gap(pr, r).is_empty() && !DOUBLED_OK.contains(&w.to_lowercase().as_str()) {
                issues.push(Issue { range: pr.start..r.end, kind: Kind::Grammar, message: format!("\u{201c}{w}\u{201d} twice in a row"), suggestions: vec![pw.to_string()] });
                continue;
            }
        }
        let lower = w.to_lowercase();
        if (lower == "a" || lower == "an") && next.is_some_and(|(nr, _)| gap(r, nr) == " ") {
            if let Some(an) = next.and_then(|(_, nw)| wants_an(nw)) {
                let right = if an { "an" } else { "a" };
                if lower != right {
                    let fix = if w.starts_with('A') { capitalize(right) } else { right.to_string() };
                    let message = if an { "Use \u{201c}an\u{201d} before a vowel sound" } else { "Use \u{201c}a\u{201d} before a consonant sound" };
                    issues.push(Issue { range: r.clone(), kind: Kind::Grammar, message: message.into(), suggestions: vec![fix] });
                }
            }
            continue;
        }
        if ["i", "i'm", "i've", "i'd", "i'll", "i\u{2019}m", "i\u{2019}ve", "i\u{2019}d", "i\u{2019}ll"].contains(w) {
            issues.push(Issue { range: r.clone(), kind: Kind::Grammar, message: "Capitalize \u{201c}I\u{201d}".into(), suggestions: vec![capitalize(w)] });
            continue;
        }
        if w.chars().next().is_some_and(char::is_lowercase) && sentence_end_before(text, r.start, prev.map(|(_, p)| *p)) {
            issues.push(Issue { range: r.clone(), kind: Kind::Grammar, message: "Start the sentence with a capital letter".into(), suggestions: vec![capitalize(w)] });
            continue;
        }
        let starts_sentence = match prev {
            None => true,
            Some((pr, pw)) => gap(pr, r).contains('\n') || sentence_end_before(text, r.start, Some(pw)),
        };
        if skip(w) || dict.knows(w) || (capitalized(w) && !starts_sentence) {
            continue;
        }
        if w.contains('-') && w.split('-').all(|p| p.is_empty() || dict.knows(p) || skip(p)) {
            continue;
        }
        issues.push(Issue { range: r.clone(), kind: Kind::Spelling, message: "Possible typo".into(), suggestions: suggest(dict, w) });
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Vec<(String, Kind, Vec<String>)> {
        let d = Dictionary::new(WORDS);
        check(text, &d).into_iter().map(|i| (text[i.range].to_string(), i.kind, i.suggestions)).collect()
    }

    #[test]
    fn finds_typos_with_fixes() {
        let found = run("I will recieve the packge tomorow.");
        assert_eq!(found.len(), 3, "{found:?}");
        assert_eq!(found[0].0, "recieve");
        assert_eq!(found[0].2[0], "receive");
        assert_eq!(found[1].2[0], "package");
        assert_eq!(found[2].2[0], "tomorrow");
    }

    #[test]
    fn accepts_ordinary_writing() {
        let ok = "Hi Priya,\n\nThanks for sending the proofs. I'm happy with the walnut cover, and Jordan's quote looks fine. Let's print 2,000 copies on Friday!\n\nBest,\nAlex";
        assert!(run(ok).is_empty(), "{:?}", run(ok));
    }

    #[test]
    fn leaves_code_links_and_quotes_alone() {
        let text = "See `cargo xtsk` and [the docs](https://exmple.com/qwzx) or mail bob@exmple.com.\n\n```\nlet xyzzy = 1;\n```\n\nOn Thu, Jordan Lee wrote:\n> Teh quote has typoz.\n";
        assert!(run(text).is_empty(), "{:?}", run(text));
    }

    #[test]
    fn checks_callouts_you_write() {
        let found = run("> **Note:** Jordan neds the files.");
        assert_eq!(found[0].0, "neds");
        assert_eq!(found[0].2[0], "needs");
    }

    #[test]
    fn grammar_slips() {
        let found = run("This is a apple and an banana. i think the the plan works. it does.");
        let texts: Vec<&str> = found.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(texts, ["a", "an", "i", "the the", "it"], "{found:?}");
        assert_eq!(found[0].2, ["an"]);
        assert_eq!(found[1].2, ["a"]);
        assert_eq!(found[3].2, ["the"]);
        assert!(found.iter().all(|f| f.1 == Kind::Grammar));
    }

    #[test]
    fn a_and_an_by_sound() {
        assert!(run("It was an honest mistake, a useful one, an hour long, a unique case.").is_empty());
        assert!(run("We met at an FBI office.").is_empty());
    }

    #[test]
    fn abbreviations_do_not_end_sentences() {
        assert!(run("Bring snacks, e.g. fruit. Talk to Dr. smith").iter().all(|f| f.0 != "fruit"));
    }

    #[test]
    fn names_and_acronyms_pass() {
        assert!(run("Ask NASA or the IRS about GPUI and macOS.").is_empty(), "{:?}", run("Ask NASA or the IRS about GPUI and macOS."));
    }
}
