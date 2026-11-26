use std::collections::{BTreeMap, BTreeSet};

pub const MIN_SPAM: u32 = 10;
pub const MIN_GOOD: u32 = 20;
const MAX_TOKENS: usize = 6000;
const KEEP_TOKENS: usize = 4500;
const CLUES: usize = 20;

#[derive(Default, Debug, PartialEq)]
pub struct Model {
    pub spam: u32,
    pub good: u32,
    pub learned_existing: u32,
    pub tokens: BTreeMap<String, (u32, u32)>,
}

pub struct Message<'a> {
    pub from_email: &'a str,
    pub from_name: &'a str,
    pub subject: &'a str,
    pub text: &'a str,
    pub hosts: Vec<String>,
    pub attachments: &'a [String],
}

fn words(text: &str, prefix: &str, out: &mut BTreeSet<String>) {
    for w in text.split(|c: char| !c.is_alphanumeric() && c != '$' && c != '%') {
        let w = w.to_lowercase();
        let n = w.chars().count();
        if (3..=20).contains(&n) && !w.chars().all(|c| c.is_ascii_digit()) {
            out.insert(format!("{prefix}{w}"));
        }
    }
}

pub fn tokens(m: &Message) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let email = m.from_email.to_lowercase();
    if let Some((_, domain)) = email.rsplit_once('@') {
        out.insert(format!("d.{domain}"));
        let parts: Vec<&str> = domain.split('.').collect();
        if parts.len() > 2 {
            out.insert(format!("d.{}", parts[parts.len() - 2..].join(".")));
        }
        out.insert(format!("t.{}", parts.last().unwrap_or(&"")));
    }
    words(m.from_name, "n.", &mut out);
    words(m.subject, "s.", &mut out);
    let text: String = m.text.chars().take(4000).collect();
    words(&text, "", &mut out);
    for h in &m.hosts {
        out.insert(format!("u.{}", h.to_lowercase()));
    }
    for a in m.attachments {
        if let Some((_, ext)) = a.rsplit_once('.') {
            out.insert(format!("a.{}", ext.to_lowercase()));
        }
    }
    out.retain(|t| !t.contains([';', ' ', '|']));
    out
}

impl Model {
    pub fn parse(s: &str) -> Model {
        let mut m = Model::default();
        let (head, body) = s.split_once('|').unwrap_or((s, ""));
        let mut nums = head.split_whitespace().map(|n| n.parse::<u32>().unwrap_or(0));
        m.spam = nums.next().unwrap_or(0);
        m.good = nums.next().unwrap_or(0);
        m.learned_existing = nums.next().unwrap_or(0);
        for entry in body.split(';') {
            let mut p = entry.split(' ');
            if let (Some(t), Some(s), Some(g)) = (p.next(), p.next(), p.next()) {
                if !t.is_empty() {
                    m.tokens.insert(t.to_string(), (s.parse().unwrap_or(0), g.parse().unwrap_or(0)));
                }
            }
        }
        m
    }

    pub fn write(&self) -> String {
        let body: Vec<String> = self.tokens.iter().map(|(t, (s, g))| format!("{t} {s} {g}")).collect();
        format!("{} {} {}|{}", self.spam, self.good, self.learned_existing, body.join(";"))
    }

    pub fn ready(&self) -> bool {
        self.spam >= MIN_SPAM && self.good >= MIN_GOOD
    }

    pub fn learn(&mut self, tokens: &BTreeSet<String>, spam: bool) {
        if spam {
            self.spam += 1;
        } else {
            self.good += 1;
        }
        for t in tokens {
            let e = self.tokens.entry(t.clone()).or_insert((0, 0));
            if spam {
                e.0 += 1;
            } else {
                e.1 += 1;
            }
        }
        if self.tokens.len() > MAX_TOKENS {
            let mut by_use: Vec<(u32, String)> = self.tokens.iter().map(|(t, (s, g))| (s + g, t.clone())).collect();
            by_use.sort();
            for (_, t) in by_use.iter().take(self.tokens.len() - KEEP_TOKENS) {
                self.tokens.remove(t);
            }
        }
    }

    fn probability(&self, t: &str) -> Option<f64> {
        let (s, g) = *self.tokens.get(t)?;
        let n = (s + g) as f64;
        let spam_rate = s as f64 / self.spam.max(1) as f64;
        let good_rate = g as f64 / self.good.max(1) as f64;
        let p = spam_rate / (spam_rate + good_rate);
        Some(((0.5 + n * p) / (1.0 + n)).clamp(0.01, 0.99))
    }

    pub fn score(&self, tokens: &BTreeSet<String>) -> (f64, Vec<String>) {
        let mut clues: Vec<(f64, &String)> = tokens.iter().filter_map(|t| self.probability(t).map(|p| (p, t))).collect();
        clues.sort_by(|a, b| (b.0 - 0.5).abs().total_cmp(&(a.0 - 0.5).abs()));
        clues.truncate(CLUES);
        if clues.is_empty() {
            return (0.5, Vec::new());
        }
        let n = clues.len();
        let ln_good: f64 = clues.iter().map(|(p, _)| p.ln()).sum();
        let ln_spam: f64 = clues.iter().map(|(p, _)| (1.0 - p).ln()).sum();
        let h = chi2q(-2.0 * ln_good, 2 * n);
        let s = chi2q(-2.0 * ln_spam, 2 * n);
        let top: Vec<String> = clues.iter().filter(|(p, _)| *p > 0.8).take(3).map(|(_, t)| plain(t)).collect();
        ((1.0 + h - s) / 2.0, top)
    }
}

fn chi2q(x2: f64, v: usize) -> f64 {
    let m = x2 / 2.0;
    let mut term = (-m).exp();
    let mut sum = term;
    for i in 1..v / 2 {
        term *= m / i as f64;
        sum += term;
    }
    sum.min(1.0)
}

pub fn plain(t: &str) -> String {
    match t.split_once('.') {
        Some(("d", d)) => format!("from {d}"),
        Some(("s", w)) => format!("\u{201c}{w}\u{201d} in the subject"),
        Some(("u", h)) => format!("links to {h}"),
        Some(("a", e)) => format!(".{e} attachment"),
        Some(("n", w)) => format!("sender named \u{201c}{w}\u{201d}"),
        Some(("t", t)) => format!(".{t} sender"),
        _ => format!("\u{201c}{t}\u{201d}"),
    }
}

pub fn threshold(config: &str) -> f64 {
    setting(config, "threshold").and_then(|v| v.parse::<f64>().ok()).map(|t| t.clamp(0.5, 0.999)).unwrap_or(0.97)
}

pub fn learns_existing(config: &str) -> bool {
    setting(config, "learn_existing").map(|v| v != "false").unwrap_or(true)
}

fn setting(config: &str, key: &str) -> Option<String> {
    config.lines().find_map(|l| {
        let l = l.split('#').next()?.trim();
        let (k, v) = l.split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"').to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg<'a>(from: &'a str, subject: &'a str, text: &'a str) -> Message<'a> {
        Message { from_email: from, from_name: "", subject, text, hosts: Vec::new(), attachments: &[] }
    }

    fn trained() -> Model {
        let mut m = Model::default();
        for i in 0..15 {
            let s = format!("Claim your free prize now {i}");
            m.learn(&tokens(&msg("winner@lucky-prizes.biz", &s, "Act now to claim your free cash prize, limited offer, click here")), true);
        }
        for i in 0..25 {
            let s = format!("Notes from Tuesday's meeting {i}");
            m.learn(&tokens(&msg("priya@lumenhouse.studio", &s, "Here are the notes from the catalogue meeting and the print schedule")), false);
        }
        m
    }

    #[test]
    fn learns_and_scores() {
        let m = trained();
        assert!(m.ready());
        let (spam, why) = m.score(&tokens(&msg("promo@lucky-prizes.biz", "Free prize inside", "Claim your cash prize now, limited offer")));
        assert!(spam > 0.97, "{spam}");
        assert!(!why.is_empty());
        let (good, _) = m.score(&tokens(&msg("jordan@lumenhouse.studio", "Catalogue print schedule", "Can we move the meeting about the print schedule?")));
        assert!(good < 0.2, "{good}");
    }

    #[test]
    fn survives_a_round_trip() {
        let m = trained();
        assert_eq!(Model::parse(&m.write()), m);
        assert_eq!(Model::parse(""), Model::default());
    }

    #[test]
    fn stays_bounded() {
        let mut m = Model::default();
        for i in 0..200 {
            let text: String = (0..40).map(|j| format!("word{i}x{j} ")).collect();
            m.learn(&tokens(&msg("a@b.test", "hi", &text)), i % 2 == 0);
        }
        assert!(m.tokens.len() <= MAX_TOKENS);
    }

    #[test]
    fn reads_its_settings() {
        assert_eq!(threshold(""), 0.97);
        assert_eq!(threshold("threshold = 0.9  # looser"), 0.9);
        assert_eq!(threshold("threshold = 3"), 0.999);
        assert!(learns_existing(""));
        assert!(!learns_existing("learn_existing = false"));
    }
}
