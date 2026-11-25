use super::*;
use crate::store::Reminder;

pub const SNOOZED: &str = "Snoozed";
pub const SNOOZE: &str = "snooze";
pub const FOLLOW_UP: &str = "follow-up";
const TICK: std::time::Duration = std::time::Duration::from_secs(30);

pub enum Woke {
    Back,
    Replied,
}

pub fn describe(due: i64) -> String {
    let Some(at) = chrono::DateTime::from_timestamp(due, 0).map(|d| d.with_timezone(&chrono::Local)) else { return String::new() };
    let today = chrono::Local::now().date_naive();
    let day = match (at.date_naive() - today).num_days() {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        2..=6 => at.format("%a").to_string(),
        _ => at.format("%b %-d").to_string(),
    };
    let time = if at.format("%M").to_string() == "00" { at.format("%-I %p") } else { at.format("%-I:%M %p") };
    format!("{day} {time}")
}

fn bring_back(s: &mut Session, mids: &[String], unlabel: bool) -> anyhow::Result<usize> {
    let mut n = 0;
    for mid in mids {
        let uids = s.uids_by_message_id(ALL_MAIL, mid)?;
        if uids.is_empty() {
            continue;
        }
        s.copy_ids(ALL_MAIL, mail::INBOX, &uids)?;
        if unlabel {
            s.store_op(ALL_MAIL, &uids, &format!("-X-GM-LABELS ({})", mail::imap_quote(SNOOZED)))?;
        }
        s.store_op(ALL_MAIL, &uids, "-FLAGS.SILENT (\\Seen)")?;
        n += uids.len();
    }
    Ok(n)
}

pub fn wake(s: &mut Session, r: &Reminder, own: &[String]) -> anyhow::Result<Woke> {
    if r.kind == FOLLOW_UP {
        if let Some(thread) = &r.thread {
            let replied = s.thread(thread, 100)?.iter().any(|m| m.ts > r.created && !own.contains(&m.from_email.to_lowercase()));
            if replied {
                return Ok(Woke::Replied);
            }
        }
        bring_back(s, &r.message_ids, false)?;
        return Ok(Woke::Back);
    }
    bring_back(s, &r.message_ids, true)?;
    Ok(Woke::Back)
}

impl MailApp {
    pub fn watch_reminders(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| loop {
            if this.update(cx, |this, cx| this.check_reminders(cx)).is_err() {
                return;
            }
            cx.background_executor().timer(TICK).await;
        })
        .detach();
    }

    pub fn check_reminders(&mut self, cx: &mut Context<Self>) {
        if self.checking_reminders {
            return;
        }
        let Some(store) = self.store.clone() else { return };
        let now = chrono::Utc::now().timestamp();
        let due = store.lock().unwrap().reminders_due(now).unwrap_or_default();
        if due.is_empty() {
            return;
        }
        let own: std::collections::HashMap<String, Vec<String>> =
            due.iter().map(|r| (r.account.clone(), self.own_addresses(&r.account))).collect();
        self.checking_reminders = true;
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let st = store.clone();
            let results = cx
                .background_executor()
                .spawn(async move {
                    let mut by_account: std::collections::BTreeMap<String, Vec<Reminder>> = Default::default();
                    for r in due {
                        by_account.entry(r.account.clone()).or_default().push(r);
                    }
                    let mut out = Vec::new();
                    for (account, list) in by_account {
                        let mut session = match Session::connect(&account) {
                            Ok(s) => s,
                            Err(e) => {
                                out.extend(list.into_iter().map(|r| (r, Err(format!("{e:#}")))));
                                continue;
                            }
                        };
                        for r in list {
                            let result = wake(&mut session, &r, &own[&account]).map_err(|e| format!("{e:#}"));
                            if result.is_ok() {
                                st.lock().unwrap().delete_reminder(r.id).ok();
                            }
                            out.push((r, result));
                        }
                    }
                    out
                })
                .await;
            this.update(cx, |this, cx| {
                this.checking_reminders = false;
                let mut back = 0;
                for (r, result) in &results {
                    match result {
                        Ok(Woke::Back) => {
                            back += 1;
                            let (title, body) = if r.kind == FOLLOW_UP {
                                ("No reply yet".to_string(), r.subject.clone())
                            } else {
                                ("Back from snooze".to_string(), r.subject.clone())
                            };
                            let (t, b) = (title.clone(), body.clone());
                            cx.background_executor().spawn(async move { crate::notify::show(&t, &b) }).detach();
                            this.status = format!("{title}: {body}").into();
                            this.status_is_error = false;
                        }
                        Ok(Woke::Replied) => {}
                        Err(e) => {
                            if crate::errlog::record(&r.account, &format!("Reminder for “{}”: {e}", r.subject)) {
                                this.problems_unseen += 1;
                            }
                        }
                    }
                }
                if back > 0 {
                    if this.unified {
                        for a in this.accounts.iter().map(|a| a.email.clone()).collect::<Vec<_>>() {
                            this.refresh_unified_account(&a, cx);
                        }
                    } else {
                        this.refresh_open_folder(cx);
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn snooze(&mut self, account: &str, targets: Vec<Envelope>, due: i64, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let mids: Vec<String> = targets.iter().filter_map(|e| e.message_id.clone()).collect();
        if mids.is_empty() {
            return self.say_error("Can't snooze this message: it has no Message-ID".into(), cx);
        }
        let now = chrono::Utc::now().timestamp();
        let subject = targets.first().map(|e| e.subject.to_string()).unwrap_or_default();
        store.lock().unwrap().drop_reminders_for(account, SNOOZE, &mids).ok();
        let r = Reminder {
            id: 0,
            account: account.to_string(),
            kind: SNOOZE.into(),
            message_ids: mids,
            thread: targets.first().and_then(|e| e.thread.clone()),
            subject,
            created: now,
            due,
        };
        let id = match store.lock().unwrap().add_reminder(&r) {
            Ok(id) => id,
            Err(e) => return self.say_error(format!("Could not snooze: {e:#}"), cx),
        };
        if due <= now + 30 {
            self.status = "Back in the Inbox".into();
            self.status_is_error = false;
            self.check_reminders(cx);
            return;
        }
        let keys: HashSet<(String, String)> = targets.iter().map(|e| e.key()).collect();
        let in_inbox = matches!(self.folder(), actions::Folder::Inbox);
        let hidden = if in_inbox { self.retain_messages(|e| !keys.contains(&e.key())) } else { Vec::new() };
        self.selected = self.selected.min(self.envelopes.len().saturating_sub(1));
        self.load_body(self.selected, cx);
        self.load_conversation(cx);
        self.status = format!("Snoozed until {}", describe(due)).into();
        self.status_is_error = false;
        cx.notify();
        let Some(session) = self.session_of(account) else { return };
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let st = store.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut s = session.lock().unwrap();
                    s.ensure_mailbox(SNOOZED)?;
                    let mut by_box: std::collections::BTreeMap<String, Vec<String>> = Default::default();
                    for e in &targets {
                        by_box.entry(e.mailbox.clone()).or_default().push(e.id.clone());
                    }
                    for (mailbox, ids) in by_box {
                        s.copy_ids(&mailbox, SNOOZED, &ids)?;
                        if mailbox.eq_ignore_ascii_case(mail::INBOX) {
                            s.move_ids(mail::INBOX, ALL_MAIL, &ids)?;
                        }
                    }
                    anyhow::Ok(())
                })
                .await;
            this.update(cx, |this, cx| match result {
                Ok(()) => this.refresh_folders(cx),
                Err(e) => {
                    st.lock().unwrap().delete_reminder(id).ok();
                    this.restore_items(hidden, cx);
                    this.say_error(format!("Could not snooze: {e:#}"), cx);
                }
            })
            .ok();
        })
        .detach();
    }

    pub fn follow_up(&mut self, account: &str, message: Envelope, due: i64, cx: &mut Context<Self>) {
        let Some(store) = self.store.clone() else { return };
        let Some(mid) = message.message_id.clone() else {
            return self.say_error("Can't follow up on this message: it has no Message-ID".into(), cx);
        };
        if message.thread.is_none() {
            return self.say_error("Can't follow up yet: Wax hasn't learned this conversation from Gmail; try again in a moment".into(), cx);
        }
        let r = Reminder {
            id: 0,
            account: account.to_string(),
            kind: FOLLOW_UP.into(),
            message_ids: vec![mid.clone()],
            thread: message.thread.clone(),
            subject: message.subject.to_string(),
            created: message.ts,
            due,
        };
        let store = store.lock().unwrap();
        store.drop_reminders_for(account, FOLLOW_UP, &[mid]).ok();
        match store.add_reminder(&r) {
            Ok(_) => {
                self.status = format!("If nobody replies by {}, this comes back to your Inbox", describe(due)).into();
                self.status_is_error = false;
            }
            Err(e) => {
                self.status = format!("Could not set the reminder: {e:#}").into();
                self.status_is_error = true;
            }
        }
        cx.notify();
    }
}
