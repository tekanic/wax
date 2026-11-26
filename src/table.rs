//! Markdown tables made easy to write:
//! - a table copied from Excel, Numbers or Google Sheets (tab-separated on
//!   the clipboard) pastes as a Markdown table;
//! - tab inside a table lines its columns up and moves to the next cell,
//!   adding a row at the end;
//! - number columns are right-aligned.
//! Aligned columns also make the plain-text part of the message readable.

use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Align {
    None,
    Left,
    Center,
    Right,
}

/// Split a table line into its cells (pipes escaped as `\|` stay in cells).
fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = if t.ends_with('|') && !t.ends_with("\\|") { &t[..t.len() - 1] } else { t };
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cur.push_str("\\|");
            chars.next();
        } else if c == '|' {
            out.push(cur.trim().to_string());
            cur.clear();
        } else {
            cur.push(c);
        }
    }
    out.push(cur.trim().to_string());
    out
}

fn is_separator(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells.iter().all(|c| {
            let c = c.trim();
            !c.is_empty() && c.trim_matches(':').chars().all(|ch| ch == '-') && c.contains('-')
        })
}

fn align_of(cell: &str) -> Align {
    let c = cell.trim();
    match (c.starts_with(':'), c.ends_with(':')) {
        (true, true) => Align::Center,
        (true, false) => Align::Left,
        (false, true) => Align::Right,
        _ => Align::None,
    }
}

fn width(s: &str) -> usize {
    s.chars().count()
}

/// True for things like 1,234.50, $12, -3%, (450).
fn looks_numeric(s: &str) -> bool {
    let t = s.trim().trim_start_matches(['$', '€', '£', '(']).trim_end_matches(['%', ')']);
    let t = t.strip_prefix('-').unwrap_or(t);
    !t.is_empty() && t.chars().all(|c| c.is_ascii_digit() || c == ',' || c == '.') && t.chars().any(|c| c.is_ascii_digit())
}

/// A table as rows of cells plus column alignments.
pub struct Table {
    pub rows: Vec<Vec<String>>,
    pub aligns: Vec<Align>,
}

impl Table {
    pub fn parse(lines: &[&str]) -> Option<Table> {
        let parsed: Vec<Vec<String>> = lines.iter().map(|l| cells(l)).collect();
        let sep = parsed.iter().position(|c| is_separator(c));
        let (rows, aligns): (Vec<Vec<String>>, Vec<Align>) = match sep {
            Some(i) => (
                parsed.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.clone()).collect(),
                parsed[i].iter().map(|c| align_of(c)).collect(),
            ),
            None => (parsed, Vec::new()),
        };
        if rows.is_empty() {
            return None;
        }
        let mut t = Table { rows, aligns };
        t.normalize();
        Some(t)
    }

    fn cols(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0).max(self.aligns.len()).max(1)
    }

    fn normalize(&mut self) {
        let n = self.cols();
        for r in &mut self.rows {
            r.resize(n, String::new());
        }
        self.aligns.resize(n, Align::None);
    }

    /// Right-align columns whose body cells are all numbers.
    pub fn auto_align(&mut self) {
        for c in 0..self.cols() {
            let body: Vec<&String> = self.rows.iter().skip(1).map(|r| &r[c]).filter(|s| !s.trim().is_empty()).collect();
            if self.aligns[c] == Align::None && !body.is_empty() && body.iter().all(|s| looks_numeric(s)) {
                self.aligns[c] = Align::Right;
            }
        }
    }

    /// Aligned Markdown, one line per row (header, separator, body).
    pub fn render(&self) -> String {
        let n = self.cols();
        let widths: Vec<usize> = (0..n)
            .map(|c| self.rows.iter().map(|r| width(&r[c])).max().unwrap_or(0).max(3))
            .collect();
        let pad = |s: &str, w: usize, a: Align| {
            let gap = w - width(s);
            match a {
                Align::Right => format!("{}{s}", " ".repeat(gap)),
                Align::Center => format!("{}{s}{}", " ".repeat(gap / 2), " ".repeat(gap - gap / 2)),
                _ => format!("{s}{}", " ".repeat(gap)),
            }
        };
        let line = |r: &Vec<String>| {
            format!("| {} |", (0..n).map(|c| pad(&r[c], widths[c], self.aligns[c])).collect::<Vec<_>>().join(" | "))
        };
        let sep = format!(
            "| {} |",
            (0..n)
                .map(|c| {
                    let w = widths[c];
                    match self.aligns[c] {
                        Align::Left => format!(":{}", "-".repeat(w - 1)),
                        Align::Right => format!("{}:", "-".repeat(w - 1)),
                        Align::Center => format!(":{}:", "-".repeat(w - 2)),
                        Align::None => "-".repeat(w),
                    }
                })
                .collect::<Vec<_>>()
                .join(" | ")
        );
        let mut out = vec![line(&self.rows[0]), sep];
        out.extend(self.rows.iter().skip(1).map(line));
        out.join("\n")
    }

    /// Byte range, within `render()`, of cell (row, col)'s text (empty at
    /// the cell's start when it has none). `row` counts body rows from 1.
    fn cell_range(&self, rendered: &str, row: usize, col: usize) -> Range<usize> {
        let start = self.cell_offset(rendered, row, col);
        let rest = &rendered[start..];
        let mut end = rest.len();
        let bytes = rest.as_bytes();
        for (i, b) in bytes.iter().enumerate() {
            if *b == b'|' && (i == 0 || bytes[i - 1] != b'\\') || *b == b'\n' {
                end = i;
                break;
            }
        }
        let content = rest[..end].trim_end();
        start..start + content.len()
    }

    fn cell_offset(&self, rendered: &str, row: usize, col: usize) -> usize {
        let line_ix = if row == 0 { 0 } else { row + 1 };
        let mut at = 0;
        for (i, l) in rendered.split('\n').enumerate() {
            if i == line_ix {
                // Each cell begins two bytes after its pipe.
                let pipes: Vec<usize> = l.match_indices('|').map(|(p, _)| p).filter(|p| *p == 0 || &l[p - 1..*p] != "\\").collect();
                let p = pipes.get(col).copied().unwrap_or(0);
                // Skip the padding of right-aligned cells.
                let cell_start = p + 2;
                let rest = &l[cell_start.min(l.len())..];
                let lead = rest.len() - rest.trim_start_matches(' ').len();
                let content_empty = rest.trim_start().starts_with('|');
                return at + cell_start + if content_empty { 0 } else { lead };
            }
            at += l.len() + 1;
        }
        at
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Agg {
    Sum,
    Avg,
    Count,
    Min,
    Max,
}

#[derive(Clone, Debug, PartialEq)]
struct Fmt {
    prefix: String,
    suffix: String,
    decimals: usize,
    commas: bool,
}

fn plain(cell: &str) -> String {
    cell.trim().trim_matches(|c| c == '*' || c == '_').trim().trim_end_matches(':').trim().to_lowercase()
}

fn aggregate_of(label: &str) -> Option<Agg> {
    match plain(label).as_str() {
        "total" | "sum" | "subtotal" | "grand total" | "totals" => Some(Agg::Sum),
        "average" | "avg" | "mean" => Some(Agg::Avg),
        "count" => Some(Agg::Count),
        "min" | "minimum" | "lowest" => Some(Agg::Min),
        "max" | "maximum" | "highest" => Some(Agg::Max),
        _ => None,
    }
}

fn numbers_column(header: &str) -> bool {
    matches!(plain(header).as_str(), "#" | "no" | "no." | "number")
}

fn number(cell: &str) -> Option<(f64, Fmt)> {
    let t = cell.trim().trim_matches(|c| c == '*' || c == '_').trim();
    if t.is_empty() || !looks_numeric(t) {
        return None;
    }
    let negative = t.starts_with('-') || (t.starts_with('(') && t.ends_with(')'));
    let core = t.trim_start_matches(['-', '(']).trim_end_matches(')');
    let prefix: String = core.chars().take_while(|c| "$€£".contains(*c)).collect();
    let core = &core[prefix.len()..];
    let suffix = if core.ends_with('%') { "%".to_string() } else { String::new() };
    let core = core.trim_end_matches('%');
    let decimals = core.split_once('.').map(|(_, d)| d.len()).unwrap_or(0);
    let v: f64 = core.replace(',', "").parse().ok()?;
    Some((if negative { -v } else { v }, Fmt { prefix, suffix, decimals, commas: core.contains(',') }))
}

fn show(v: f64, f: &Fmt) -> String {
    let neg = v < 0.0;
    let s = format!("{:.*}", f.decimals, v.abs());
    let (int, frac) = s.split_once('.').map(|(a, b)| (a.to_string(), Some(b.to_string()))).unwrap_or((s.clone(), None));
    let int = if f.commas || int.len() > 4 {
        let digits: Vec<char> = int.chars().collect();
        let mut out = String::new();
        for (i, c) in digits.iter().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(*c);
        }
        out
    } else {
        int
    };
    let body = match frac {
        Some(fr) => format!("{int}.{fr}"),
        None => int,
    };
    format!("{}{}{body}{}", if neg { "-" } else { "" }, f.prefix, f.suffix)
}

fn merge(a: Option<Fmt>, b: &Fmt) -> Fmt {
    match a {
        None => b.clone(),
        Some(a) => Fmt {
            prefix: if a.prefix.is_empty() { b.prefix.clone() } else { a.prefix },
            suffix: if a.suffix.is_empty() { b.suffix.clone() } else { a.suffix },
            decimals: a.decimals.max(b.decimals),
            commas: a.commas || b.commas,
        },
    }
}

fn split_formula(header: &str) -> Option<(&str, &str)> {
    let (name, expr) = header.split_once('=')?;
    let expr = expr.trim();
    (!expr.is_empty()).then_some((name.trim_end(), expr))
}

struct Expr<'a> {
    s: &'a [u8],
    at: usize,
    value: &'a dyn Fn(&str) -> Option<(f64, Fmt)>,
    fmt: Option<Fmt>,
}

impl Expr<'_> {
    fn skip(&mut self) {
        while self.s.get(self.at).is_some_and(|c| c.is_ascii_whitespace()) {
            self.at += 1;
        }
    }

    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        loop {
            self.skip();
            match self.s.get(self.at) {
                Some(b'+') => {
                    self.at += 1;
                    v += self.product()?;
                }
                Some(b'-') => {
                    self.at += 1;
                    v -= self.product()?;
                }
                _ => return Some(v),
            }
        }
    }

    fn product(&mut self) -> Option<f64> {
        let mut v = self.atom()?;
        loop {
            self.skip();
            match self.s.get(self.at) {
                Some(b'*') => {
                    self.at += 1;
                    v *= self.atom()?;
                }
                Some(b'/') => {
                    self.at += 1;
                    let d = self.atom()?;
                    if d == 0.0 {
                        return None;
                    }
                    v /= d;
                }
                _ => return Some(v),
            }
        }
    }

    fn atom(&mut self) -> Option<f64> {
        self.skip();
        match *self.s.get(self.at)? {
            b'(' => {
                self.at += 1;
                let v = self.sum()?;
                self.skip();
                (self.s.get(self.at) == Some(&b')')).then(|| self.at += 1)?;
                Some(v)
            }
            b'-' => {
                self.at += 1;
                Some(-self.atom()?)
            }
            c if c.is_ascii_digit() || c == b'.' => {
                let start = self.at;
                while self.s.get(self.at).is_some_and(|c| c.is_ascii_digit() || *c == b'.') {
                    self.at += 1;
                }
                let mut v: f64 = std::str::from_utf8(&self.s[start..self.at]).ok()?.parse().ok()?;
                if self.s.get(self.at) == Some(&b'%') {
                    self.at += 1;
                    v /= 100.0;
                }
                Some(v)
            }
            c if c.is_ascii_alphabetic() || c == b'#' => {
                let start = self.at;
                while self.s.get(self.at).is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'#' || *c == b'.') {
                    self.at += 1;
                }
                let name = std::str::from_utf8(&self.s[start..self.at]).ok()?;
                let (v, f) = (self.value)(name)?;
                if f.suffix != "%" {
                    self.fmt = Some(merge(self.fmt.take(), &f));
                }
                Some(if f.suffix == "%" { v / 100.0 } else { v })
            }
            _ => None,
        }
    }
}

impl Table {
    pub fn calculate(&mut self) {
        if self.rows.len() < 2 {
            return;
        }
        let n = self.cols();
        let numbering = (0..n).find(|&c| numbers_column(&self.rows[0][c]));
        let label_col = (0..n).find(|&c| Some(c) != numbering).unwrap_or(0);
        let totals: Vec<(usize, Agg)> = (1..self.rows.len()).filter_map(|r| aggregate_of(&self.rows[r][label_col]).map(|a| (r, a))).collect();
        let body: Vec<usize> = (1..self.rows.len()).filter(|r| !totals.iter().any(|(t, _)| t == r)).collect();
        if let Some(c) = numbering {
            let mut i = 0;
            for &r in &body {
                if self.rows[r].iter().enumerate().any(|(k, cell)| k != c && !cell.trim().is_empty()) {
                    i += 1;
                    self.rows[r][c] = i.to_string();
                } else {
                    self.rows[r][c].clear();
                }
            }
            for (r, _) in &totals {
                self.rows[*r][c].clear();
            }
        }
        let headers: Vec<String> = self.rows[0].iter().map(|h| split_formula(h).map(|(name, _)| name).unwrap_or(h).to_string()).collect();
        let key = |h: &str| plain(h).replace([' ', '_'], "");
        let find = |name: &str| -> Option<usize> {
            let wanted = key(name);
            (0..n).find(|&c| key(&headers[c]) == wanted).or_else(|| {
                let b = name.as_bytes();
                (b.len() == 1 && b[0].is_ascii_alphabetic()).then(|| (b[0].to_ascii_uppercase() - b'A') as usize).filter(|&ix| ix < n)
            })
        };
        for c in 0..n {
            let Some(expr) = split_formula(&self.rows[0][c]).map(|(_, e)| e.replace('×', "*")) else { continue };
            for &r in &body {
                let row = self.rows[r].clone();
                let value = |name: &str| find(name).filter(|&k| k != c).and_then(|k| number(&row[k]));
                let mut e = Expr { s: expr.as_bytes(), at: 0, value: &value, fmt: None };
                let result = e.sum().filter(|_| {
                    e.skip();
                    e.at == e.s.len()
                });
                self.rows[r][c] = match result {
                    Some(v) if v.is_finite() => {
                        let mut f = e.fmt.unwrap_or(Fmt { prefix: String::new(), suffix: String::new(), decimals: 0, commas: false });
                        if v.fract().abs() > 1e-9 && f.decimals == 0 {
                            f.decimals = 2;
                        }
                        show(v, &f)
                    }
                    _ => String::new(),
                };
            }
        }
        for c in 0..n {
            if c == label_col || Some(c) == numbering {
                continue;
            }
            let cells: Vec<&String> = body.iter().map(|&r| &self.rows[r][c]).filter(|s| !s.trim().is_empty()).collect();
            let parsed: Vec<(f64, Fmt)> = cells.iter().filter_map(|s| number(s)).collect();
            if parsed.is_empty() || parsed.len() != cells.len() {
                continue;
            }
            let fmt = parsed.iter().fold(None, |acc, (_, f)| Some(merge(acc, f))).unwrap();
            let values: Vec<f64> = parsed.iter().map(|(v, _)| *v).collect();
            let name = plain(&headers[c]);
            let per_unit = ["price", "rate", "unit", "each", "per ", "/"].iter().any(|k| name.contains(k)) || fmt.suffix == "%";
            for (r, agg) in &totals {
                if *agg == Agg::Sum && per_unit {
                    continue;
                }
                let (v, f) = match agg {
                    Agg::Sum => (values.iter().sum(), fmt.clone()),
                    Agg::Avg => {
                        let mut f = fmt.clone();
                        f.decimals = f.decimals.max(1);
                        (values.iter().sum::<f64>() / values.len() as f64, f)
                    }
                    Agg::Count => (values.len() as f64, Fmt { prefix: String::new(), suffix: String::new(), decimals: 0, commas: false }),
                    Agg::Min => (values.iter().cloned().fold(f64::INFINITY, f64::min), fmt.clone()),
                    Agg::Max => (values.iter().cloned().fold(f64::NEG_INFINITY, f64::max), fmt.clone()),
                };
                let bold = self.rows[*r][label_col].trim().starts_with("**");
                let text = show(v, &f);
                self.rows[*r][c] = if bold { format!("**{text}**") } else { text };
            }
        }
    }

    fn strip_formulas(&mut self) -> bool {
        let mut changed = false;
        for h in self.rows[0].iter_mut() {
            if let Some((name, _)) = split_formula(h) {
                *h = name.to_string();
                changed = true;
            }
        }
        changed
    }
}

fn map_tables(text: &str, mut f: impl FnMut(&mut Table) -> bool) -> String {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].trim_start().starts_with('|') {
            out.push(lines[i].to_string());
            i += 1;
            continue;
        }
        let mut j = i;
        while j < lines.len() && lines[j].trim_start().starts_with('|') {
            j += 1;
        }
        let block = &lines[i..j];
        let changed = Table::parse(block).filter(|_| block.iter().any(|l| is_separator(&cells(l)))).and_then(|mut t| f(&mut t).then(|| t.render()));
        match changed {
            Some(r) => out.push(r),
            None => out.extend(block.iter().map(|l| l.to_string())),
        }
        i = j;
    }
    out.join("\n")
}

pub fn strip_formulas(text: &str) -> String {
    map_tables(text, |t| t.strip_formulas())
}

pub fn calculate_all(text: &str) -> String {
    map_tables(text, |t| {
        let before = t.rows.clone();
        t.calculate();
        t.rows != before
    })
}

/// Clipboard text from a spreadsheet → a Markdown table.
pub fn from_tsv(text: &str) -> Option<String> {
    let text = text.trim_end_matches(['\n', '\r']);
    let lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    if lines.len() < 2 || !lines.iter().all(|l| l.contains('\t')) {
        return None;
    }
    let rows: Vec<Vec<String>> = lines
        .iter()
        .map(|l| l.split('\t').map(|c| c.trim().replace('|', "\\|")).collect())
        .collect();
    let mut t = Table { rows, aligns: Vec::new() };
    t.normalize();
    t.auto_align();
    Some(t.render())
}

/// A blank table to start from.
pub fn blank(cols: usize, rows: usize) -> String {
    let mut all = vec![(1..=cols).map(|c| format!("Column {c}")).collect::<Vec<_>>()];
    all.extend((0..rows).map(|_| vec![String::new(); cols]));
    let mut t = Table { rows: all, aligns: Vec::new() };
    t.normalize();
    t.render()
}

/// Byte range of the table block around `offset`, if it's in one.
pub fn block_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let is_row = |l: &str| l.trim_start().starts_with('|');
    let mut starts = Vec::new();
    let mut at = 0;
    for l in text.split('\n') {
        starts.push((at, l));
        at += l.len() + 1;
    }
    let i = starts.iter().rposition(|(s, _)| *s <= offset)?;
    if !is_row(starts[i].1) {
        return None;
    }
    let mut a = i;
    while a > 0 && is_row(starts[a - 1].1) {
        a -= 1;
    }
    let mut b = i;
    while b + 1 < starts.len() && is_row(starts[b + 1].1) {
        b += 1;
    }
    Some(starts[a].0..starts[b].0 + starts[b].1.len())
}

/// Tab in a table: tidy it and move to the next cell (a new row after the
/// last cell). Returns the new text and that cell's text, selected, so
/// typing replaces it.
pub fn tab(text: &str, cursor: usize, back: bool) -> Option<(String, Range<usize>)> {
    let range = block_at(text, cursor)?;
    let block = &text[range.clone()];
    let lines: Vec<&str> = block.split('\n').collect();
    let mut table = Table::parse(&lines)?;
    table.calculate();
    // Where the cursor is: line, then cell (count pipes before it).
    let mut rel = cursor - range.start;
    let mut line_ix = 0;
    for (i, l) in lines.iter().enumerate() {
        if rel <= l.len() {
            line_ix = i;
            break;
        }
        rel -= l.len() + 1;
    }
    let sep_ix = lines.iter().position(|l| is_separator(&cells(l)));
    let row = match sep_ix {
        Some(s) if line_ix > s => line_ix - 1,
        Some(s) if line_ix == s => 1,
        _ => line_ix,
    };
    let line = lines[line_ix];
    let before = &line[..rel.min(line.len())];
    let col = before.matches('|').count().saturating_sub(if line.trim_start().starts_with('|') { 1 } else { 0 });
    let cols = table.cols();
    let (mut r, mut c) = (row, col.min(cols - 1));
    if back {
        if c > 0 {
            c -= 1;
        } else if r > 0 {
            r -= 1;
            c = cols - 1;
        }
    } else if c + 1 < cols {
        c += 1;
    } else {
        r += 1;
        c = 0;
        if r >= table.rows.len() {
            table.rows.push(vec![String::new(); cols]);
        }
    }
    let rendered = table.render();
    let cell = table.cell_range(&rendered, r, c);
    let out = format!("{}{}{}", &text[..range.start], rendered, &text[range.end..]);
    Some((out, range.start + cell.start..range.start + cell.end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreadsheet_paste() {
        let t = from_tsv("Lab\tOrders\tRevenue\nSan Leandro\t42\t$1,250.00\nCinnaminson\t7\t$310.00\n").unwrap();
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines.len(), 4);
        assert!(lines[1].ends_with(":|") || lines[1].contains("-:"), "numbers right-aligned: {t}");
        assert!(lines.iter().all(|l| width(l) == width(lines[0])), "columns line up:\n{t}");
        assert!(from_tsv("just text").is_none());
        assert!(from_tsv("one\tline").is_none());
    }

    #[test]
    fn tab_moves_and_tidies() {
        let text = "Intro\n| a | b |\n|---|---|\n| 1 | 2 |\nEnd";
        let cursor = text.find("| a").unwrap() + 2;
        let (out, sel) = tab(text, cursor, false).unwrap();
        assert!(out.starts_with("Intro\n| a   | b   |"), "{out}");
        assert_eq!(&out[sel], "b", "next cell's text is selected");
        // Last cell: a new row appears, its first (empty) cell selected.
        let last = out.find("2").unwrap();
        let (out2, sel2) = tab(&out, last, false).unwrap();
        assert_eq!(out2.matches('\n').count(), out.matches('\n').count() + 1);
        assert!(sel2.is_empty());
        assert!(out2.ends_with("\nEnd"));
        // Back: shift-tab returns to the previous cell.
        let (_, back) = tab(&out, out.find("b ").unwrap(), true).unwrap();
        assert_eq!(&out[back], "a");
    }

    fn rows(text: &str) -> Vec<Vec<String>> {
        let lines: Vec<&str> = text.lines().collect();
        Table::parse(&lines).unwrap().rows
    }

    #[test]
    fn totals_add_up_in_the_column_format() {
        let t = calculate_all("| Item | Qty | Price |\n|---|---|---|\n| Catalog | 2,000 | $1,250.00 |\n| Inserts | 500 | $310.50 |\n| **Total** | | |");
        let r = rows(&t);
        assert_eq!(r[3], ["**Total**", "**2,500**", ""], "unit prices don't add up");
        let t = calculate_all("| Item | Cost |\n|---|---|\n| A | $1,250.00 |\n| B | $310.50 |\n| Total | |");
        assert_eq!(rows(&t)[3][1], "$1,560.50");
    }

    #[test]
    fn average_count_and_more() {
        let t = calculate_all("| Lab | Days |\n|---|---|\n| A | 3 |\n| B | 4 |\n| Average | |\n| Count | |\n| Max | |");
        let r = rows(&t);
        assert_eq!(r[3][1], "3.5");
        assert_eq!(r[4][1], "2");
        assert_eq!(r[5][1], "4");
    }

    #[test]
    fn hash_column_numbers_rows() {
        let t = calculate_all("| # | Step |\n|---|---|\n| | Sign off |\n| 7 | Send list |\n| | |\n| | Confirm |");
        let r = rows(&t);
        let nums: Vec<&str> = r.iter().skip(1).map(|r| r[0].as_str()).collect();
        assert_eq!(nums, ["1", "2", "", "3"]);
    }

    #[test]
    fn column_formulas_by_name_or_letter() {
        let t = calculate_all("| Item | Qty | Price | Amount =Qty*Price |\n|---|---|---|---|\n| Catalog | 2,000 | $0.62 | |\n| Inserts | 500 | $0.10 | |\n| Total | | | |");
        let r = rows(&t);
        assert_eq!(r[1][3], "$1,240.00");
        assert_eq!(r[2][3], "$50.00");
        assert_eq!(r[3][3], "$1,290.00");
        let t = calculate_all("| A | B | C =A*B + 1 |\n|---|---|---|\n| 2 | 3 | |\n| x | 3 | |");
        let r = rows(&t);
        assert_eq!(r[1][2], "7");
        assert_eq!(r[2][2], "", "a missing number leaves it blank");
        let t = calculate_all("| Price | Tax | With tax =Price*(1+Tax) |\n|---|---|---|\n| $100.00 | 8% | |");
        assert_eq!(rows(&t)[1][2], "$108.00");
    }

    #[test]
    fn formulas_never_reach_the_recipient() {
        let text = "Hi\n\n| Qty | Price | Amount =Qty*Price |\n|---|---|---|\n| 2 | $5.00 | $10.00 |\n\nBye";
        let sent = strip_formulas(text);
        assert!(sent.contains("| Amount ") && !sent.contains('='), "{sent}");
        assert!(sent.starts_with("Hi\n\n") && sent.ends_with("\n\nBye"));
        assert_eq!(strip_formulas("plain text | with a pipe"), "plain text | with a pipe");
    }

    #[test]
    fn tab_recalculates() {
        let text = "| Item | Cost |\n|---|---|\n| A | 5 |\n| B | 7 |\n| Total | |";
        let (out, _) = tab(text, text.find("| 7").unwrap() + 2, false).unwrap();
        assert!(out.contains("| Total | 12"), "{out}");
    }

    #[test]
    fn blank_table() {
        let t = blank(3, 2);
        assert_eq!(t.lines().count(), 4);
        assert!(t.starts_with("| Column 1 |"));
    }

    #[test]
    fn renders_as_html_table() {
        let html = crate::md::to_email_html(&from_tsv("A\tB\nx\t1\n").unwrap(), true);
        assert!(html.contains("<table"), "{html}");
        assert!(html.contains("border:1px solid"), "{html}");
        // Every cell styled, aligned ones included.
        assert_eq!(html.matches("<td").count(), html.matches("<td style=\"border").count(), "{html}");
        assert!(html.contains("right"), "number column right-aligned: {html}");
    }
}
