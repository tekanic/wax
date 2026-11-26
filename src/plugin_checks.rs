use super::*;
use crate::store::message_key;
use std::collections::{HashMap, HashSet};
use wasm_host::{Action, CheckReason};

const CHUNK: usize = 20;
const RECENT_DAYS: i64 = 90;

pub struct Scan {
    pub plugin: String,
    pub done: usize,
    pub total: usize,
    pub labelled: usize,
}

fn keep_safe(reason: CheckReason, actions: Vec<Action>) -> Vec<Action> {
    if reason == CheckReason::Arrived {
        return actions;
    }
    actions.into_iter().filter(|a| matches!(a, Action::AddLabel(_) | Action::Warn(_))).collect()
}

pub struct Outcome {
    pub warned: Vec<(String, String)>,
    pub labelled: usize,
    pub problems: Vec<String>,
}

pub fn record(
    store: &SharedStore,
    session: Option<&SharedSession>,
    account: &str,
    results: Vec<(Envelope, Vec<(String, Vec<Action>)>)>,
    checkers: &[(String, String)],
    reason: CheckReason,
) -> Outcome {
    let mut out = Outcome { warned: Vec::new(), labelled: 0, problems: Vec::new() };
    let keys: Vec<String> = results.iter().map(|(e, _)| message_key(e)).collect();
    for (e, by_plugin) in results {
        let key = message_key(&e);
        for (plugin, actions) in by_plugin {
            let actions = keep_safe(reason, actions);
            for a in &actions {
                if let Action::Warn(text) = a {
                    store.lock().unwrap().add_warning(account, &key, &plugin, text).ok();
                    out.warned.push((key.clone(), text.clone()));
                }
            }
            let mail: Vec<Action> = actions.into_iter().filter(|a| matches!(a, Action::AddLabel(_))).collect();
            if mail.is_empty() {
                continue;
            }
            match session {
                Some(s) => match plugins::apply_actions(s, &e.mailbox, &e.id, &mail) {
                    Ok(_) => out.labelled += 1,
                    Err(err) => out.problems.push(format!("{plugin}: could not label “{}”: {err:#}", e.subject)),
                },
                None => out.problems.push(format!("{plugin}: offline, “{}” wasn't labelled", e.subject)),
            }
        }
    }
    let store = store.lock().unwrap();
    for (id, version) in checkers {
        store.mark_checked(account, &keys, id, version).ok();
    }
    out
}

impl MailApp {
    pub fn message_warnings(&self, e: &Envelope) -> Vec<String> {
        self.warnings.get(&(self.account_of(e).to_string(), message_key(e))).cloned().unwrap_or_default()
    }

    pub fn load_warnings(&mut self, e: &Envelope) {
        let Some(store) = &self.store else { return };
        let (account, key) = (self.account_of(e).to_string(), message_key(e));
        let found = store.lock().unwrap().warnings(&account, &key);
        if found.is_empty() {
            self.warnings.remove(&(account, key));
        } else {
            self.warnings.insert((account, key), found);
        }
    }

    pub fn check_opened(&mut self, e: Envelope, cx: &mut Context<Self>) {
        self.load_warnings(&e);
        if crate::demo::on() {
            return;
        }
        let (Some(host), Some(store)) = (self.wasm.clone(), self.store.clone()) else { return };
        let account = self.account_of(&e).to_string();
        let key = message_key(&e);
        if !self.opened_checks.insert((account.clone(), key.clone())) {
            return;
        }
        let session = self.preview_of(&account);
        let mine: Vec<String> = self.own_addresses(&account);
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let (a, k) = (account.clone(), key.clone());
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let mut h = host.lock().unwrap();
                    let before = h.running_checkers();
                    if before.is_empty() || store.lock().unwrap().checked_by_all(&a, &k, &before) {
                        return None;
                    }
                    h.arrival = wasm_host::Arrival {
                        mine,
                        readable: Some(HashSet::from([(e.mailbox.clone(), e.id.clone())])),
                        sessions: Arc::new(Mutex::new(session.iter().map(|s| (a.clone(), s.clone())).collect())),
                        ..Default::default()
                    };
                    let (acts, problems) = h.check(&plugins::to_wit(&a, &e), CheckReason::Opened, None);
                    let checkers = h.running_checkers();
                    h.arrival = Default::default();
                    drop(h);
                    let mut out = record(&store, session.as_ref(), &a, vec![(e, acts)], &checkers, CheckReason::Opened);
                    out.problems.extend(problems);
                    Some(out)
                })
                .await;
            this.update(cx, |this, cx| {
                let Some(out) = outcome else { return };
                for p in &out.problems {
                    crate::errlog::record(&account, p);
                }
                if !out.warned.is_empty() {
                    this.warnings.insert((account.clone(), key.clone()), out.warned.into_iter().map(|(_, t)| t).collect());
                }
                if out.labelled > 0 {
                    this.refresh_folders(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn check_leaving_spam(host: &Arc<Mutex<wasm_host::Host>>, store: &SharedStore, session: &SharedSession, account: &str, items: &[Envelope], mine: Vec<String>) -> Outcome {
        let spam: Vec<Envelope> = items.iter().filter(|e| e.mailbox.eq_ignore_ascii_case(mail::SPAM)).cloned().collect();
        if spam.is_empty() {
            return Outcome { warned: Vec::new(), labelled: 0, problems: Vec::new() };
        }
        let mut h = host.lock().unwrap();
        h.arrival = wasm_host::Arrival {
            mine,
            readable: Some(spam.iter().map(|e| (e.mailbox.clone(), e.id.clone())).collect()),
            sessions: Arc::new(Mutex::new(HashMap::from([(account.to_string(), session.clone())]))),
            ..Default::default()
        };
        let mut results = Vec::new();
        let mut problems = Vec::new();
        for e in spam {
            let (acts, mut p) = h.check(&plugins::to_wit(account, &e), CheckReason::NotSpam, None);
            problems.append(&mut p);
            results.push((e, acts));
        }
        let checkers = h.running_checkers();
        h.arrival = Default::default();
        drop(h);
        let mut out = record(store, Some(session), account, results, &checkers, CheckReason::NotSpam);
        out.problems.extend(problems);
        out
    }

    pub fn start_scan(&mut self, plugin: &str, cx: &mut Context<Self>) {
        if self.scan.is_some() {
            return;
        }
        let (Some(host), Some(store)) = (self.wasm.clone(), self.store.clone()) else { return };
        let Some((name, version, dir)) = host.lock().unwrap().plugins.iter().find(|p| p.id == plugin && p.running() && p.scans()).map(|p| (p.name(), p.version(), p.dir.clone())) else {
            return;
        };
        let todo = store.lock().unwrap().unchecked(plugin, &version).unwrap_or_default();
        let accounts: Vec<String> = self.accounts.iter().map(|a| a.email.clone()).collect();
        let todo: Vec<Envelope> = todo.into_iter().filter(|e| accounts.iter().any(|a| a == e.account.as_ref())).collect();
        let total = todo.len();
        let plugin = plugin.to_string();
        self.scan = Some(Scan { plugin: plugin.clone(), done: 0, total, labelled: 0 });
        let mut by_account: Vec<(String, Vec<Envelope>, Option<SharedSession>, Vec<String>)> = Vec::new();
        for a in &accounts {
            let list: Vec<Envelope> = todo.iter().filter(|e| e.account.as_ref() == a).cloned().collect();
            if !list.is_empty() {
                by_account.push((a.clone(), list, self.preview_of(a), self.own_addresses(a)));
            }
        }
        cx.notify();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let recent_since = chrono::Utc::now().timestamp() - RECENT_DAYS * 86_400;
            let mut problems: Vec<String> = Vec::new();
            let mut stopped = false;
            'accounts: for (account, list, session, mine) in by_account {
                let sessions: Arc<Mutex<HashMap<String, SharedSession>>> = Arc::new(Mutex::new(session.iter().map(|s| (account.clone(), s.clone())).collect()));
                for chunk in list.chunks(CHUNK) {
                    let chunk = chunk.to_vec();
                    let n = chunk.len();
                    let (h, st, p, acct, mine, sessions) = (host.clone(), store.clone(), plugin.clone(), account.clone(), mine.clone(), sessions.clone());
                    let out = cx
                        .background_executor()
                        .spawn(async move {
                            let mut h = h.lock().unwrap();
                            let version = h.plugins.iter().find(|x| x.id == p).map(|x| x.version()).unwrap_or_default();
                            h.arrival = wasm_host::Arrival {
                                mine,
                                readable: Some(chunk.iter().filter(|e| e.ts >= recent_since).map(|e| (e.mailbox.clone(), e.id.clone())).collect()),
                                sessions: sessions.clone(),
                                ..Default::default()
                            };
                            let mut results = Vec::new();
                            let mut probs = Vec::new();
                            for e in chunk {
                                let (acts, mut pr) = h.check(&plugins::to_wit(&acct, &e), CheckReason::Existing, Some(&p));
                                probs.append(&mut pr);
                                results.push((e, acts));
                            }
                            let alive = h.plugins.iter().any(|x| x.id == p && x.running());
                            h.arrival = Default::default();
                            drop(h);
                            let session = sessions.lock().unwrap().get(&acct).cloned();
                            let checkers = if alive { vec![(p.clone(), version)] } else { Vec::new() };
                            let mut out = record(&st, session.as_ref(), &acct, results, &checkers, CheckReason::Existing);
                            out.problems.extend(probs);
                            (out, alive)
                        })
                        .await;
                    let (out, running) = out;
                    if !running {
                        stopped = true;
                    }
                    problems.extend(out.problems);
                    let alive = this
                        .update(cx, |this, cx| {
                            if let Some(s) = &mut this.scan {
                                s.done += n;
                                s.labelled += out.labelled;
                                this.status = format!("{name}: checked {} of {} messages…", group_digits(s.done as u64), group_digits(s.total as u64)).into();
                                this.status_is_error = false;
                            }
                            for (key, text) in out.warned {
                                this.warnings.entry((account.clone(), key)).or_default().push(text);
                            }
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        return;
                    }
                    if stopped {
                        break 'accounts;
                    }
                }
            }
            let marked = !stopped && std::fs::write(dir.join("scanned"), &version).is_ok();
            this.update(cx, |this, cx| {
                for p in problems.iter().take(20) {
                    crate::errlog::record("", p);
                }
                let finished = this.scan.take();
                if let Some(s) = finished.as_ref().filter(|_| stopped) {
                    this.status = format!("{name} stopped after {} of {} messages (see Settings → Problems)", group_digits(s.done as u64), group_digits(s.total as u64)).into();
                    this.status_is_error = true;
                } else if let Some(s) = finished {
                    let extra = if problems.is_empty() { String::new() } else { format!(" ({} problems, see Settings → Problems)", problems.len()) };
                    this.status = format!("{name}: checked {} messages, {} flagged{extra}", group_digits(s.total as u64), s.labelled).into();
                    this.status_is_error = false;
                }
                this.refresh_folders(cx);
                if marked {
                    this.scan_if_needed(cx);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn scan_if_needed(&mut self, cx: &mut Context<Self>) {
        if crate::demo::on() {
            return;
        }
        let Some(host) = self.wasm.clone() else { return };
        let due: Vec<String> = host
            .lock()
            .unwrap()
            .plugins
            .iter()
            .filter(|p| p.running() && p.scans())
            .filter(|p| std::fs::read_to_string(p.dir.join("scanned")).map(|v| v.trim() != p.version()).unwrap_or(true))
            .map(|p| p.id.clone())
            .collect();
        if let Some(first) = due.first() {
            self.start_scan(first, cx);
        }
    }

    pub fn render_warnings(&self, e: &Envelope) -> Option<AnyElement> {
        let found = self.message_warnings(e);
        if found.is_empty() {
            return None;
        }
        Some(
            div()
                .flex_none()
                .px_3()
                .py_2()
                .rounded_md()
                .bg(pal().warning_bg)
                .border_1()
                .border_color(pal().warning_border)
                .flex()
                .flex_col()
                .gap_1()
                .text_xs()
                .text_color(pal().warning)
                .children(found.into_iter().map(|t| div().child(format!("⚠ {t}"))))
                .into_any_element(),
        )
    }
}
