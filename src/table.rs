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
