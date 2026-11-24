//! New mail without asking: a watcher on its own IMAP connection checks the
//! Inbox every 30 seconds (one cheap STATUS command). When new mail
//! arrives it fetches just those headers, runs your sender rules, puts the
//! rest at the top of the Inbox, sets the Dock badge to the unread count and
//! posts a notification. Every 5 minutes, or when the Inbox shrank (mail
//! archived or deleted on your phone), the open folder is refreshed too.

use super::*;

const TICK: std::time::Duration = std::time::Duration::from_secs(30);
const FIRST_TICK: std::time::Duration = std::time::Duration::from_secs(5);
const FULL_REFRESH: std::time::Duration = std::time::Duration::from_secs(300);

/// What one check found and what to do about it.
pub struct Plan {
    /// UID range of new Inbox mail to fetch.
    pub fetch: Option<(u64, u64)>,
    /// Re-read the open folder and the folder counts.
    pub refresh: bool,
}

#[derive(Default)]
pub struct WatchState {
    pub uid_next: Option<u64>,
    pub messages: u64,
    /// Unread in the Inbox at the last check (sidebar and Dock badge).
    pub unseen: u64,
    pub last_full: Option<std::time::Instant>,
    pub task: Option<Task<()>>,
}

impl MailApp {
    /// Watch every account's Inbox. Each account has its own watcher and
    /// connection; it stops when the account is removed.
    pub fn start_watches(&mut self, cx: &mut Context<Self>) {
        crate::notify::request();
        let emails: Vec<String> = self.accounts.iter().map(|a| a.email.clone()).collect();
        self.watch.retain(|k, _| emails.contains(k));
        for email in emails {
            self.start_watch(email, cx);
        }
    }

    fn start_watch(&mut self, account: String, cx: &mut Context<Self>) {
        if self.watch.get(&account).is_some_and(|w| w.task.is_some()) {
            return;
        }
        let acct = account.clone();
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let mut delay = FIRST_TICK;
            loop {
                cx.background_executor().timer(delay).await;
                delay = TICK;
                // No connection of its own: it borrows one that's open anyway
                // (the account on screen's second connection, or another
                // account's pooled one, opened here when there isn't one).
                let a = acct.clone();
                let Ok((shared, on_screen)) = this.update(cx, |this, _| (this.preview_of(&a), *this.account == *a)) else {
                    return;
                };
                let session = match shared {
                    Some(s) => s,
                    None if on_screen => continue, // still connecting
                    None => {
                        let a2 = a.clone();
                        match cx.background_executor().spawn(async move { Session::connect(&a2) }).await {
                            Ok(s) => {
                                let s: SharedSession = Arc::new(Mutex::new(s));
                                let keep = s.clone();
                                this.update(cx, |this, _| {
                                    if *this.account != *a {
                                        this.pool.entry(a.clone()).or_insert(keep);
                                    }
                                })
                                .ok();
                                s
                            }
                            Err(err) => {
                                let text = format!("Checking for new mail: {err:#}");
                                this.update(cx, |this, cx| {
                                    if errlog::record(&a, &text) {
                                        this.problems_unseen += 1;
                                        cx.notify();
                                    }
                                })
                                .ok();
                                continue;
                            }
                        }
                    }
                };
                let s = session.clone();
                let checked = cx.background_executor().spawn(async move { s.lock().unwrap().status(mail::INBOX) }).await;
                let (uid_next, unseen, messages) = match checked {
                    Ok(v) => v,
                    Err(err) => {
                        let text = format!("Checking for new mail: {err:#}");
                        let a = acct.clone();
                        this.update(cx, |this, cx| {
                            if errlog::record(&a, &text) {
                                this.problems_unseen += 1;
                                cx.notify();
                            }
                        })
                        .ok();
                        continue;
                    }
                };
                let a = acct.clone();
                let Ok(Some(plan)) = this.update(cx, |this, cx| this.watch_plan(&a, uid_next, unseen, messages, cx)) else {
                    return; // the window or the account is gone
                };
                if let Some((lo, hi)) = plan.fetch {
                    let s = session.clone();
                    let fetched = cx.background_executor().spawn(async move { s.lock().unwrap().headers_in_range(mail::INBOX, lo, hi) }).await;
                    if let Ok(list) = fetched {
                        let a = acct.clone();
                        this.update(cx, |this, cx| this.arrived(&a, list, cx)).ok();
                    }
                }
                if plan.refresh {
                    let a = acct.clone();
                    this.update(cx, |this, cx| {
                        if this.unified {
                            this.refresh_unified_account(&a, cx)
                        } else if *this.account == *a {
                            this.refresh_open_folder(cx)
                        }
                    })
                    .ok();
                }
            }
        });
        self.watch.entry(account).or_default().task = Some(task);
    }

    /// None = stop (the account was removed).
    fn watch_plan(&mut self, account: &str, uid_next: u64, unseen: u64, messages: u64, cx: &mut Context<Self>) -> Option<Plan> {
        let w = self.watch.get_mut(account)?;
        let fetch = match w.uid_next {
            Some(old) if uid_next > old => Some((old, uid_next - 1)),
            _ => None,
        };
        // Fewer messages with nothing new = archived or deleted elsewhere.
        let shrank = w.uid_next == Some(uid_next) && messages < w.messages;
        let due = w.last_full.map_or(false, |t| t.elapsed() >= FULL_REFRESH);
        let refresh = w.uid_next.is_some() && (shrank || due);
        if refresh || w.last_full.is_none() {
            w.last_full = Some(std::time::Instant::now());
        }
        let changed = w.unseen != unseen;
        w.uid_next = Some(uid_next);
        w.messages = messages;
        w.unseen = unseen;
        set_dock_badge(self.watch.values().map(|w| w.unseen).sum());
        if fetch.is_some() || refresh || changed {
            cx.notify();
        }
        Some(Plan { fetch, refresh })
    }

    /// New Inbox mail: cache it, run the rules, show and announce the rest.
    fn arrived(&mut self, account: &str, list: Vec<Envelope>, cx: &mut Context<Self>) {
        if list.is_empty() {
            return;
        }
        if let Some(store) = &self.store {
            store_upsert(&store.lock().unwrap(), account, &list).ok();
        }
        let active = *self.account == *account;
        let kept = self.dispatch_arrivals(account, list, cx);
        let in_inbox = self.query.is_empty()
            && (self.unified || (active && self.active().is_some_and(|m| m.name.eq_ignore_ascii_case(mail::INBOX))));
        if in_inbox {
            let keep = self.envelopes.get(self.selected).map(|e| e.key());
            let have: HashSet<(String, String)> = self.envelopes.iter().map(|e| e.key()).collect();
            let mut fresh: Vec<Envelope> =
                kept.iter().filter(|e| !have.contains(&e.key())).cloned().collect();
            if !fresh.is_empty() {
                fresh.append(&mut self.envelopes);
                fresh.sort_by(|a, b| b.ts.cmp(&a.ts));
                self.envelopes = fresh;
                // The message you're reading stays selected.
                if let Some(k) = keep {
                    if let Some(ix) = self.envelopes.iter().position(|e| e.key() == k) {
                        self.selected = ix;
                    }
                }
            }
        }
        let new_unread: Vec<&Envelope> = kept.iter().filter(|e| e.unread).collect();
        if !new_unread.is_empty() {
            let n = new_unread.len();
            // With several accounts, say which one.
            let tag = if self.accounts.len() > 1 {
                self.accounts.iter().find(|a| a.email == account).map(|a| format!(" · {}", a.short())).unwrap_or_default()
            } else {
                String::new()
            };
            if active {
                self.status = format!("{n} new message{}", if n == 1 { "" } else { "s" }).into();
                self.status_is_error = false;
            }
            let (title, body) = if n == 1 {
                (format!("{}{tag}", new_unread[0].from), new_unread[0].subject.to_string())
            } else {
                (
                    format!("{n} new messages{tag}"),
                    new_unread.iter().take(3).map(|e| e.from.to_string()).collect::<Vec<_>>().join(", "),
                )
            };
            cx.background_executor().spawn(async move { notify(&title, &body) }).detach();
        }
        if active {
            self.refresh_folders(cx);
        }
        cx.notify();
    }

    /// Re-read the open folder from Gmail, keeping the selection, unless
    /// you're searching or have scrolled into older mail.
    pub fn refresh_open_folder(&mut self, cx: &mut Context<Self>) {
        self.refresh_folders(cx);
        if self.query.is_empty()
            && self.sweep.is_none()
            && self.envelopes.len() <= PAGE_SIZE as usize
            && self.pending_bulk.is_empty()
            && self.pending_sweep.is_empty()
        {
            let ix = self.active_mailbox;
            self.open_mailbox(ix, cx);
        }
    }
}

/// Unread count on the Dock icon (empty at zero).
fn set_dock_badge(unseen: u64) {
    #[cfg(target_os = "macos")]
    {
        use objc2::MainThreadMarker;
        use objc2_app_kit::NSApplication;
        use objc2_foundation::NSString;
        let Some(mtm) = MainThreadMarker::new() else { return };
        let app = NSApplication::sharedApplication(mtm);
        let tile = app.dockTile();
        let label = (unseen > 0).then(|| NSString::from_str(&unseen.to_string()));
        tile.setBadgeLabel(label.as_deref());
        if std::env::var("GPUI_MAIL_DEBUG").is_ok() {
            eprintln!("dock badge now {:?}", tile.badgeLabel().map(|l| l.to_string()));
        }
    }
}

/// A macOS notification banner.
fn notify(title: &str, body: &str) {
    crate::notify::show(title, body);
}
