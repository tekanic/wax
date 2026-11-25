use super::*;
use std::collections::HashMap;

type Key = (String, String);

pub fn thread_groups(all: Vec<Envelope>, here: &str) -> (Vec<Envelope>, HashMap<Key, Vec<Envelope>>) {
    let mut rep_of: HashMap<Key, Key> = HashMap::new();
    let mut hidden: HashMap<Key, Vec<Envelope>> = HashMap::new();
    let mut rows = Vec::new();
    for e in all {
        let account = if e.account.is_empty() { here.to_string() } else { e.account.to_string() };
        match e.thread.clone().map(|t| (account, t)) {
            Some(g) => match rep_of.get(&g) {
                Some(rep) => hidden.entry(rep.clone()).or_default().push(e),
                None => {
                    rep_of.insert(g, e.key());
                    rows.push(e);
                }
            },
            None => rows.push(e),
        }
    }
    (rows, hidden)
}

impl MailApp {
    pub fn all_messages(&self) -> Vec<Envelope> {
        let mut all = self.envelopes.clone();
        for v in self.hidden.values() {
            all.extend(v.iter().cloned());
        }
        all
    }

    pub fn group_threads(&mut self) {
        let mut all = std::mem::take(&mut self.envelopes);
        for (_, v) in self.hidden.drain() {
            all.extend(v);
        }
        let mut seen = HashSet::new();
        all.retain(|e| seen.insert(e.key()));
        all.sort_by(|a, b| b.ts.cmp(&a.ts));
        if !self.settings.conversations {
            self.envelopes = all;
            return;
        }
        let (rows, hidden) = thread_groups(all, &self.account);
        self.hidden = hidden;
        self.envelopes = rows;
        let rows: HashSet<(String, String)> = self.envelopes.iter().map(|e| e.key()).collect();
        self.marked.retain(|k| rows.contains(k));
    }

    pub fn retain_messages(&mut self, keep: impl Fn(&Envelope) -> bool) -> Vec<Envelope> {
        let mut all = std::mem::take(&mut self.envelopes);
        for (_, v) in self.hidden.drain() {
            all.extend(v);
        }
        let (kept, gone): (Vec<Envelope>, Vec<Envelope>) = all.into_iter().partition(|e| keep(e));
        self.envelopes = kept;
        self.group_threads();
        gone
    }

    pub fn row_of(&self, key: &(String, String)) -> Option<usize> {
        if let Some(ix) = self.envelopes.iter().position(|e| e.key() == *key) {
            return Some(ix);
        }
        let rep = self.hidden.iter().find(|(_, v)| v.iter().any(|e| e.key() == *key)).map(|(k, _)| k)?;
        self.envelopes.iter().position(|e| e.key() == *rep)
    }

    pub fn members(&self, row: &Envelope) -> Vec<Envelope> {
        let mut out = vec![row.clone()];
        if let Some(v) = self.hidden.get(&row.key()) {
            out.extend(v.iter().cloned());
        }
        out
    }

    pub fn row_unread(&self, e: &Envelope) -> bool {
        e.unread || self.hidden.get(&e.key()).is_some_and(|v| v.iter().any(|m| m.unread))
    }

    pub fn row_size(&self, e: &Envelope) -> usize {
        1 + self.hidden.get(&e.key()).map_or(0, Vec::len)
    }

    pub fn set_conversations(&mut self, on: bool, cx: &mut Context<Self>) {
        let keep = self.envelopes.get(self.selected).map(|e| e.key());
        self.settings.conversations = on;
        self.group_threads();
        self.selected = keep.and_then(|k| self.row_of(&k)).unwrap_or(0);
        self.scroll.scroll_to_item(self.selected, ScrollStrategy::Center);
        self.save_settings(cx);
    }

    pub fn mark_thread_read(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.envelopes.get(ix) else { return };
        let unread: Vec<Envelope> = self.hidden.get(&row.key()).map(|v| v.iter().filter(|m| m.unread).cloned().collect()).unwrap_or_default();
        for m in unread {
            self.set_unread_of(m, false, cx);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::store::tests_support::envelope;

    #[test]
    fn groups_by_account_and_thread() {
        let mk = |id: &str, account: &str, thread: Option<&str>, ts: i64| {
            let mut e = envelope("INBOX", id, "a@x.com");
            e.account = account.to_string().into();
            e.thread = thread.map(str::to_string);
            e.ts = ts;
            e
        };
        let mut list = vec![
            mk("1", "me@a", Some("t1"), 10),
            mk("2", "me@a", Some("t1"), 30),
            mk("3", "me@b", Some("t1"), 20),
            mk("4", "me@a", None, 5),
            mk("5", "", Some("t2"), 25),
        ];
        list.sort_by(|a, b| b.ts.cmp(&a.ts));
        let (rows, hidden) = super::thread_groups(list, "me@a");
        let ids: Vec<&str> = rows.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["2", "5", "3", "4"]);
        let under_2: Vec<&str> = hidden[&rows[0].key()].iter().map(|e| e.id.as_str()).collect();
        assert_eq!(under_2, ["1"]);
        assert_eq!(hidden.len(), 1);
    }
}
