use super::*;
use crate::model::{display_date, make_preview, Attachment};
use crate::store::{message_key, Reminder, Store};
use anyhow::bail;
use std::path::PathBuf;

pub const WORK: &str = "alex@lumenhouse.studio";
pub const HOME: &str = "alex.morgan@example.com";
const ME: &str = "Alex Morgan";

pub fn on() -> bool {
    std::env::var_os("WAX_DEMO").is_some()
}

pub fn view() -> String {
    std::env::var("WAX_DEMO_VIEW").unwrap_or_default()
}

struct Msg {
    account: &'static str,
    boxes: &'static [&'static str],
    from: (&'static str, &'static str),
    to: &'static [(&'static str, &'static str)],
    subject: &'static str,
    mins: i64,
    unread: bool,
    starred: bool,
    thread: Option<&'static str>,
    body: &'static str,
    html: Option<&'static str>,
    files: &'static [(&'static str, &'static str, u64)],
}

const INBOX: &str = "INBOX";
const SENT: &str = "[Gmail]/Sent Mail";
const ALL: &str = "[Gmail]/All Mail";
const READING: &str = "Reading";
const RECORDS: &str = "Records";

const NEWSLETTER: &str = r#"<div style="font-family:Georgia,serif;max-width:600px;margin:0 auto;color:#1d2330">
<p style="font:600 12px Helvetica,sans-serif;letter-spacing:.08em;color:#8a6d3b;margin:0 0 6px">THE MORNING LEDGER · THURSDAY</p>
<h1 style="font-size:30px;line-height:1.15;margin:0 0 14px">Rates, rails and rain</h1>
<p style="font-size:17px;line-height:1.6">Good morning. Three things worth your coffee today: the central bank holds steady, the coastal line reopens after two years of work, and a wet weekend is coming for most of the east.</p>
<h2 style="font-size:20px;margin:26px 0 8px">1. Rates hold, for now</h2>
<p style="font-size:17px;line-height:1.6">Policymakers left rates where they were and hinted that the next move depends on spring hiring numbers.</p>
<h2 style="font-size:20px;margin:26px 0 8px">2. The coastal line is back</h2>
<p style="font-size:17px;line-height:1.6">Trains run again from Monday, forty minutes faster end to end than before the rebuild.</p>
<h2 style="font-size:20px;margin:26px 0 8px">3. Bring an umbrella</h2>
<p style="font-size:17px;line-height:1.6">Two storms are lining up for Saturday and Sunday. Plan the barbecue for next week.</p>
</div>"#;

fn messages() -> Vec<Msg> {
    vec![
        Msg {
            account: WORK,
            boxes: &[INBOX, ALL],
            from: ("Priya Shah", "priya@lumenhouse.studio"),
            to: &[(ME, WORK)],
            subject: "Re: Spring catalogue proofs",
            mins: 6,
            unread: true,
            starred: false,
            thread: Some("t-catalogue"),
            body: "Perfect, thanks Alex. I've folded your notes into v4 and sent it to the printer for a hard proof.\n\nTwo small things before we sign off:\n\n- Page 12: the caption still says \"oak\", the table is walnut.\n- The back cover QR code needs the new short link.\n\nCan you check both by Friday noon? Jordan says the press slot is Monday at 7.\n\nPriya",
            html: None,
            files: &[("catalogue-v4-proof.pdf", "application/pdf", 8_400_000)],
        },
        Msg {
            account: WORK,
            boxes: &[SENT, ALL],
            from: (ME, WORK),
            to: &[("Priya Shah", "priya@lumenhouse.studio")],
            subject: "Re: Spring catalogue proofs",
            mins: 95,
            unread: false,
            starred: false,
            thread: Some("t-catalogue"),
            body: "Looks great. A few notes:\n\n1. **Cover**: can we try the lighter paper stock?\n2. Spreads 6–7 feel tight; drop one product shot.\n3. Prices go in the insert, not the pages.\n\nOtherwise good to go.\n\nAlex",
            html: None,
            files: &[],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, ALL],
            from: ("Priya Shah", "priya@lumenhouse.studio"),
            to: &[(ME, WORK)],
            subject: "Spring catalogue proofs",
            mins: 260,
            unread: false,
            starred: false,
            thread: Some("t-catalogue"),
            body: "Hi Alex,\n\nFirst full proofs of the spring catalogue are attached. Have a look when you can and send notes by tomorrow if possible.\n\nPriya",
            html: None,
            files: &[("catalogue-v3.pdf", "application/pdf", 7_900_000)],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, ALL],
            from: ("Jordan Lee", "jordan@harbor-print.com"),
            to: &[(ME, WORK)],
            subject: "Quote for 2,000 catalogues",
            mins: 41,
            unread: true,
            starred: true,
            thread: Some("t-quote"),
            body: "Hi Alex,\n\nQuote attached for 2,000 copies, 48 pages, matte cover. Turnaround is 6 working days from proof sign-off.\n\nIf you go to 3,000 the unit price drops by about 18%.\n\nBest,\nJordan",
            html: None,
            files: &[("Quote-LH-2041.pdf", "application/pdf", 212_000)],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, "Suspicious", ALL],
            from: ("PayPal", "service@paypa1-help.net"),
            to: &[(ME, WORK)],
            subject: "Your account has been limited",
            mins: 73,
            unread: true,
            starred: false,
            thread: Some("t-phish"),
            body: "Dear customer,\n\nWe noticed unusual activity and have limited your account. Verify your details within 24 hours to avoid permanent suspension.\n\nVerify now: https://paypa1-help.net/secure\n\nPayPal Security",
            html: None,
            files: &[],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, ALL],
            from: ("Sam Okafor", "sam@okafor.design"),
            to: &[(ME, WORK)],
            subject: "Logo options, round 3",
            mins: 130,
            unread: false,
            starred: false,
            thread: Some("t-logo"),
            body: "Alex,\n\nThree directions attached. My pick is B: it holds up at favicon size and on the van.\n\nHappy to jump on a call tomorrow.\n\nSam",
            html: None,
            files: &[("logo-a.png", "image/png", 180_000), ("logo-b.png", "image/png", 164_000), ("logo-c.png", "image/png", 171_000)],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, READING, ALL],
            from: ("The Morning Ledger", "hello@morningledger.news"),
            to: &[(ME, WORK)],
            subject: "Rates, rails and rain",
            mins: 190,
            unread: true,
            starred: false,
            thread: Some("t-ledger"),
            body: "Good morning. Three things worth your coffee today.",
            html: Some(NEWSLETTER),
            files: &[],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, ALL],
            from: ("Ruth Alvarez", "ruth@alvarez-law.com"),
            to: &[(ME, WORK)],
            subject: "Lease: redlines from the landlord",
            mins: 330,
            unread: false,
            starred: false,
            thread: Some("t-lease"),
            body: "Alex,\n\nThe landlord came back with redlines on sections 4 and 9. Nothing alarming, but 9.2 shifts repair costs to you. I've marked my suggested reply in the margin.\n\nRuth",
            html: None,
            files: &[("Lease-redlines.docx", "application/vnd.openxmlformats-officedocument.wordprocessingml.document", 96_000)],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, RECORDS, ALL],
            from: ("Northline Air", "trips@northlineair.com"),
            to: &[(ME, WORK)],
            subject: "Your trip to Lisbon: check-in opens tomorrow",
            mins: 410,
            unread: false,
            starred: false,
            thread: Some("t-trip"),
            body: "Hi Alex,\n\nCheck-in for flight NL 482 to Lisbon opens tomorrow at 9:10.\n\nConfirmation: Q7XK2P\nSeat: 14A",
            html: None,
            files: &[],
        },
        Msg {
            account: WORK,
            boxes: &[INBOX, ALL],
            from: ("Dev Patel", "dev@lumenhouse.studio"),
            to: &[(ME, WORK), ("Priya Shah", "priya@lumenhouse.studio")],
            subject: "Shop launch checklist",
            mins: 600,
            unread: false,
            starred: false,
            thread: Some("t-launch"),
            body: "Team,\n\nHere's where we are for Tuesday:\n\n| Task | Owner | Status |\n|---|---|---|\n| Product photos | Sam | Done |\n| Copy | Priya | In review |\n| Payments test | Dev | Wednesday |\n\nShout if anything's missing.\n\nDev",
            html: None,
            files: &[],
        },
        Msg {
            account: WORK,
            boxes: &["Lobby", ALL],
            from: ("Taylor Brooks", "taylor@brookshire-ventures.com"),
            to: &[(ME, WORK)],
            subject: "Quick intro",
            mins: 150,
            unread: true,
            starred: false,
            thread: Some("t-intro"),
            body: "Hi Alex, I run a small fund and love what Lumen House does. Open to a 15-minute chat?",
            html: None,
            files: &[],
        },
        Msg {
            account: WORK,
            boxes: &["Snoozed", ALL],
            from: ("Greenway Bank", "statements@greenwaybank.com"),
            to: &[(ME, WORK)],
            subject: "Your November statement is ready",
            mins: 1500,
            unread: false,
            starred: false,
            thread: Some("t-bank"),
            body: "Your November statement is ready to view.",
            html: None,
            files: &[],
        },
        Msg {
            account: HOME,
            boxes: &[INBOX, ALL],
            from: ("Mia Chen", "mia.chen@example.org"),
            to: &[(ME, HOME)],
            subject: "Dinner Saturday?",
            mins: 22,
            unread: true,
            starred: false,
            thread: Some("t-dinner"),
            body: "We're doing the new Lebanese place on Saturday at 7. You in? Bring Jo!",
            html: None,
            files: &[],
        },
        Msg {
            account: HOME,
            boxes: &[INBOX, ALL],
            from: ("Riverside Climbing", "hello@riversideclimbing.club"),
            to: &[(ME, HOME)],
            subject: "Your membership renews next week",
            mins: 880,
            unread: false,
            starred: false,
            thread: Some("t-climb"),
            body: "Hi Alex, your annual membership renews on the 3rd. Nothing to do if you're staying with us.",
            html: None,
            files: &[],
        },
    ]
}

const BULK: &[(&str, &str, &str, u32, bool)] = &[
    ("The Morning Ledger", "hello@morningledger.news", "Morning briefing", 64, true),
    ("Field Notes Weekly", "weekly@fieldnotes.blog", "This week in design", 31, true),
    ("Studio Supply Co.", "deals@studiosupply.shop", "New arrivals, 20% off", 48, true),
    ("Parcelpoint", "updates@parcelpoint.com", "Your package is on its way", 22, false),
    ("Tracker", "notify@tracker-app.io", "3 tasks due today", 37, true),
    ("Greenway Bank", "statements@greenwaybank.com", "Your statement is ready", 12, false),
    ("Northline Air", "trips@northlineair.com", "Deals from your city", 18, true),
];

fn envelope(account: &str, mailbox: &str, id: String, mid: String, from: (&str, &str), to: &[(&str, &str)], subject: &str, ts: i64, unread: bool, starred: bool, thread: Option<String>) -> Envelope {
    Envelope {
        mailbox: mailbox.to_string(),
        id,
        message_id: Some(mid),
        from: from.0.to_string().into(),
        from_email: from.1.to_string().into(),
        subject: subject.to_string().into(),
        ts,
        date: display_date(ts),
        preview: Default::default(),
        body: None,
        html: None,
        unread,
        starred,
        important: false,
        attachments: Vec::new(),
        account: account.to_string().into(),
        to: to.iter().map(|(n, e)| (n.to_string(), e.to_string())).collect(),
        thread,
        labels: Vec::new(),
    }
}

fn mailboxes() -> Vec<String> {
    [INBOX, "Lobby", READING, RECORDS, "[Gmail]/Starred", "[Gmail]/Important", SENT, "[Gmail]/Drafts", ALL, "[Gmail]/Spam", "[Gmail]/Trash", "Screened Out", "Snoozed", "Suspicious", "Clients", "Travel"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

fn under_home(p: &std::path::Path) -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    !home.is_empty() && home.contains("wax-demo") && p.starts_with(&home)
}

pub fn make() -> Result<()> {
    let config = crate::files::config_dir();
    let data = crate::files::data_dir();
    if !on() || !under_home(&config) || !under_home(&data) {
        bail!("--make-demo only runs with WAX_DEMO=1 and HOME set to a folder named wax-demo");
    }
    let real_plugins = std::env::var("WAX_DEMO_PLUGINS").map(PathBuf::from).ok();
    std::fs::remove_dir_all(&config).ok();
    std::fs::remove_dir_all(&data).ok();
    std::fs::create_dir_all(&config)?;
    std::fs::create_dir_all(&data)?;

    for (order, (email, badge)) in [(WORK, "L"), (HOME, "")].into_iter().enumerate() {
        crate::accounts::save(&crate::accounts::Account {
            email: email.to_string(),
            name: ME.to_string(),
            auth: crate::accounts::Auth::Google,
            order: order as i64 + 1,
            sig_new: None,
            sig_reply: None,
            color: None,
            badge: badge.to_string(),
        })?;
    }
    std::fs::write(
        crate::accounts::rules_file(WORK),
        "\"hello@morningledger.news\" = \"Reading\"\n\"weekly@fieldnotes.blog\" = \"Reading\"\n\"trips@northlineair.com\" = \"Records\"\n\"statements@greenwaybank.com\" = \"Records\"\n\"deals@studiosupply.shop\" = \"Screened Out\"\n",
    )?;
    let mut settings = crate::settings::Settings::default();
    settings.theme_mode = crate::theme::Mode::Dark;
    settings.save()?;
    std::fs::write(data.join("window.txt"), "windowed 80 80 1440 900\n")?;

    let mut store = Store::open()?;
    let now = chrono::Utc::now().timestamp();
    for account in [WORK, HOME] {
        store.save_mailboxes(account, &mailboxes())?;
    }
    let mut n = 0u32;
    for m in messages() {
        n += 1;
        let ts = now - m.mins * 60;
        let mid = format!("<demo-{n}@{}>", m.from.1.split('@').nth(1).unwrap_or("example.com"));
        let files: Vec<Attachment> = m.files.iter().map(|(name, mime, size)| Attachment { name: name.to_string(), mime: mime.to_string(), size: *size }).collect();
        for (i, b) in m.boxes.iter().enumerate() {
            let id = format!("{}", 1000 + n * 10 + i as u32);
            let e = envelope(m.account, b, id.clone(), mid.clone(), m.from, m.to, m.subject, ts, m.unread, m.starred, m.thread.map(str::to_string));
            store.upsert_envelopes(m.account, std::slice::from_ref(&e))?;
            store.save_body(m.account, b, &id, m.body, m.html, &make_preview(m.body), &files)?;
            if m.from.1 == "service@paypa1-help.net" && *b == INBOX {
                store.add_warning(
                    m.account,
                    &message_key(&e),
                    "phishing",
                    "Possible phishing: it's named PayPal but comes from paypa1-help.net, which isn't PayPal's; it pushes you to act (\u{201c}within 24 hours\u{201d})",
                )?;
            }
            if *b == "Snoozed" {
                store.add_reminder(&Reminder {
                    id: 0,
                    account: m.account.to_string(),
                    kind: "snooze".into(),
                    message_ids: vec![mid.clone()],
                    thread: m.thread.map(str::to_string),
                    subject: m.subject.to_string(),
                    created: now,
                    due: now + 86_400 * 2,
                })?;
            }
        }
    }
    let mut bulk = Vec::new();
    for (name, email, subject, count, unread_often) in BULK {
        for k in 0..*count {
            n += 1;
            let ts = now - 86_400 * (2 + k as i64 * 3) - (n as i64 * 97) % 3600;
            let unread = *unread_often && k % 5 != 0;
            bulk.push(envelope(WORK, ALL, format!("{}", 50_000 + n), format!("<bulk-{n}@demo>"), (name, email), &[(ME, WORK)], subject, ts, unread, false, Some(format!("t-bulk-{n}"))));
        }
    }
    store.upsert_envelopes(WORK, &bulk)?;
    store.add_sent_contacts(WORK, &[("Priya Shah".into(), "priya@lumenhouse.studio".into(), now), ("Jordan Lee".into(), "jordan@harbor-print.com".into(), now), ("Sam Okafor".into(), "sam@okafor.design".into(), now)])?;

    if let Some(src) = real_plugins {
        let dest = crate::wasm_host::plugins_dir();
        for id in ["sweep", "screener", "snooze", "phishing", "filter-rules"] {
            let from = src.join(id).join("plugin.wasm");
            if from.exists() {
                std::fs::create_dir_all(dest.join(id))?;
                std::fs::copy(&from, dest.join(id).join("plugin.wasm"))?;
            }
        }
        std::fs::write(dest.join("screener").join("config.toml"), format!("screen = \"{WORK}\"\n")).ok();
        let mut host = crate::wasm_host::Host::new()?;
        host.load();
        for p in &host.plugins {
            let asked = p.requested();
            crate::wasm_host::write_grants(&p.dir, true, &asked, &asked)?;
            std::fs::write(p.dir.join("scanned"), p.version()).ok();
        }
    }
    println!("Demo mail written to {} and {}", config.display(), data.display());
    Ok(())
}

impl MailApp {
    pub fn demo_online(&mut self, cx: &mut Context<Self>) {
        self.online = true;
        self.loading = false;
        self.status = SharedString::default();
        if let Some(store) = &self.store {
            let store = store.lock().unwrap();
            for m in &self.mailboxes {
                let list = store.envelopes(&self.account, &m.name, 10_000).unwrap_or_default();
                let unread = list.iter().filter(|e| e.unread).count() as u64;
                self.label_counts.insert(m.name.to_string(), (list.len() as u64, unread));
            }
        }
        cx.notify();
    }

    pub fn demo_stage_later(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            for _ in 0..40 {
                cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                let ready = this.update(cx, |this, _| this.wasm.is_some() && !this.envelopes.is_empty()).unwrap_or(false);
                if ready {
                    break;
                }
            }
            cx.background_executor().timer(std::time::Duration::from_millis(600)).await;
            this.update_in(cx, |this, window, cx| this.demo_stage(window, cx)).ok();
        })
        .detach();
    }

    fn demo_pick(&mut self, subject: &str, cx: &mut Context<Self>) {
        if let Some(ix) = self.envelopes.iter().position(|e| e.subject.as_ref() == subject) {
            self.select(ix, cx);
        }
    }

    fn demo_stage(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let v = view();
        match v.as_str() {
            "light" => {
                self.settings.theme_mode = crate::theme::Mode::Light;
                self.use_theme(false, cx);
                self.demo_pick("Re: Spring catalogue proofs", cx);
            }
            "phishing" => self.demo_pick("Your account has been limited", cx),
            "newsletter" => self.demo_pick("Rates, rails and rain", cx),
            "unified" => {
                self.open_unified(mail::INBOX, cx);
            }
            "reply" => {
                self.demo_pick("Re: Spring catalogue proofs", cx);
                if let Some(e) = self.reading().cloned() {
                    let text = e.body.as_ref().map(|b| b.to_string()).unwrap_or_default();
                    let orig = compose::Original {
                        from: vec![(e.from.to_string(), e.from_email.to_string())],
                        to: vec![(ME.to_string(), WORK.to_string())],
                        message_id: e.message_id.clone(),
                        subject: e.subject.to_string(),
                        date: "Thu, Nov 26, 2026 at 11:02 AM".into(),
                        quoted: text.clone(),
                        text,
                        ..Default::default()
                    };
                    let mut d = compose::prefill(Kind::Reply, &orig, &[WORK.to_string()]);
                    d.account = WORK.to_string();
                    d.body = format!("Both fixed:\n\n- Caption on page 12 now says **walnut**.\n- QR code points at the new short link.\n\nGood to print.{}", d.body);
                    let d = self.with_defaults(d);
                    self.open_composer(d, window, cx);
                }
            }
            "sweep" => self.open_sweep(window, cx),
            "plugins" => self.open_settings(settings::Pane::Plugins, window, cx),
            "settings" => self.open_settings(settings::Pane::Writing, window, cx),
            _ => self.demo_pick("Re: Spring catalogue proofs", cx),
        }
        cx.notify();
    }
}
