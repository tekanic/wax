pub const THRESHOLD: u32 = 3;

const TLDS: &str = include_str!("tlds.txt");

pub fn real_tld(domain: &str) -> bool {
    let Some(tld) = domain.rsplit('.').next().filter(|t| !t.is_empty()) else { return false };
    let tld = tld.to_ascii_uppercase();
    if matches!(tld.as_str(), "LOCAL" | "INTERNAL" | "LAN" | "HOME" | "LOCALDOMAIN" | "CORP") {
        return true;
    }
    TLDS.lines().any(|l| !l.starts_with('#') && l.trim() == tld)
}

const PUBLIC: [&str; 16] = [
    "gmail.com",
    "googlemail.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "yahoo.com",
    "icloud.com",
    "me.com",
    "aol.com",
    "proton.me",
    "protonmail.com",
    "msn.com",
    "mail.com",
    "email.com",
    "ymail.com",
    "gmx.com",
];

pub const BRANDS: &[(&str, &[&str])] = &[
    ("paypal", &["paypal.com"]),
    ("apple", &["apple.com", "icloud.com", "itunes.com", "mac.com"]),
    ("icloud", &["apple.com", "icloud.com"]),
    ("microsoft", &["microsoft.com", "microsoftonline.com", "outlook.com", "live.com", "office.com", "office365.com", "sharepoint.com", "onedrive.com", "microsoft365.com", "microsoftadvertising.com", "skype.com", "xbox.com", "azure.com", "bing.com", "msn.com", "linkedin.com", "github.com"]),
    ("office 365", &["microsoft.com", "office.com", "office365.com", "microsoftonline.com"]),
    ("outlook", &["microsoft.com", "outlook.com", "live.com"]),
    ("amazon", &["amazon.com", "amazon.co.uk", "amazon.ca", "amazonses.com", "amazonaws.com", "aws.com", "audible.com", "primevideo.com", "amazon.jobs"]),
    ("google", &["google.com", "gmail.com", "youtube.com", "googlemail.com", "fitbit.com", "nest.com", "withgoogle.com"]),
    ("gmail", &["google.com", "gmail.com"]),
    ("netflix", &["netflix.com"]),
    ("docusign", &["docusign.com", "docusign.net"]),
    ("chase", &["chase.com", "jpmorgan.com"]),
    ("bank of america", &["bankofamerica.com", "bofa.com", "ealerts.bankofamerica.com"]),
    ("wells fargo", &["wellsfargo.com"]),
    ("citibank", &["citi.com", "citibank.com"]),
    ("capital one", &["capitalone.com"]),
    ("american express", &["americanexpress.com", "aexp.com"]),
    ("amex", &["americanexpress.com", "aexp.com"]),
    ("coinbase", &["coinbase.com"]),
    ("venmo", &["venmo.com"]),
    ("zelle", &["zellepay.com"]),
    ("fedex", &["fedex.com"]),
    ("ups", &["ups.com"]),
    ("usps", &["usps.com", "usps.gov"]),
    ("dhl", &["dhl.com"]),
    ("irs", &["irs.gov"]),
    ("dropbox", &["dropbox.com", "dropboxmail.com"]),
    ("linkedin", &["linkedin.com"]),
    ("facebook", &["facebook.com", "facebookmail.com", "meta.com"]),
    ("instagram", &["instagram.com", "mail.instagram.com"]),
    ("adobe", &["adobe.com"]),
    ("intuit", &["intuit.com"]),
    ("quickbooks", &["intuit.com", "quickbooks.com"]),
    ("norton", &["norton.com", "nortonlifelock.com"]),
    ("mcafee", &["mcafee.com"]),
    ("geek squad", &["bestbuy.com"]),
];

const PRESSURE: [(&str, u32); 36] = [
    ("verify your account", 2),
    ("verify your identity", 2),
    ("confirm your account", 2),
    ("account suspended", 2),
    ("account has been suspended", 2),
    ("account will be closed", 2),
    ("unusual sign-in", 1),
    ("unusual activity", 1),
    ("password expires", 2),
    ("password expiry", 2),
    ("gift card", 2),
    ("wire transfer", 2),
    ("payment failed", 1),
    ("invoice overdue", 1),
    ("overdue invoice", 1),
    ("action required", 1),
    ("urgent", 1),
    ("final notice", 1),
    ("quick favor", 2),
    ("quick favour", 2),
    ("are you available", 1),
    ("at your desk", 2),
    ("bank details", 2),
    ("new bank account", 2),
    ("payment details", 2),
    ("shared a document", 1),
    ("voicemail", 1),
    ("blocked your account", 2),
    ("account has been blocked", 2),
    ("account is blocked", 2),
    ("will be deleted", 2),
    ("storage is full", 2),
    ("storage full", 2),
    ("renew your subscription", 1),
    ("payment declined", 1),
    ("suspicious activity", 1),
];

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Settings {
    pub label: String,
    pub trust: Vec<String>,
    pub brands: Vec<(String, Vec<String>)>,
}

pub fn settings(text: &str) -> Settings {
    let mut s = Settings { label: "Suspicious".into(), ..Default::default() };
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let v = v.trim();
        let v = v.strip_prefix('"').and_then(|v| v.split_once('"')).map(|(v, _)| v).unwrap_or(v).trim();
        match k.trim() {
            "label" if !v.is_empty() => s.label = v.to_string(),
            "trust" if !v.is_empty() => s.trust.push(v.to_lowercase()),
            "brand" => {
                if let Some((name, domains)) = v.split_once(':') {
                    let domains: Vec<String> = domains.split_whitespace().map(str::to_lowercase).collect();
                    if !name.trim().is_empty() && !domains.is_empty() {
                        s.brands.push((name.trim().to_lowercase(), domains));
                    }
                }
            }
            _ => {}
        }
    }
    s
}

pub struct Mail<'a> {
    pub name: &'a str,
    pub email: &'a str,
    pub subject: &'a str,
    pub me: &'a [String],
}

pub struct Contact {
    pub name: String,
    pub email: String,
}

#[derive(Debug, PartialEq)]
pub struct Verdict {
    pub score: u32,
    pub reasons: Vec<String>,
}

pub fn domain(email: &str) -> String {
    email.rsplit_once('@').map(|(_, d)| d.trim().trim_end_matches('>').to_lowercase()).unwrap_or_default()
}

pub fn base(domain: &str) -> String {
    let parts: Vec<&str> = domain.split('.').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 2 {
        return parts.join(".");
    }
    let n = parts.len();
    let second = parts[n - 2];
    if parts[n - 1].len() == 2 && matches!(second, "co" | "com" | "org" | "net" | "gov" | "ac" | "edu") {
        parts[n - 3..].join(".")
    } else {
        parts[n - 2..].join(".")
    }
}

fn belongs(domain: &str, owner: &str) -> bool {
    domain == owner || domain.ends_with(&format!(".{owner}"))
}

fn has_word(hay: &str, word: &str) -> bool {
    let hay = hay.to_lowercase();
    let mut start = 0;
    while let Some(i) = hay[start..].find(word) {
        let at = start + i;
        let before = hay[..at].chars().last().is_none_or(|c| !c.is_alphanumeric());
        let after = hay[at + word.len()..].chars().next().is_none_or(|c| !c.is_alphanumeric());
        if before && after {
            return true;
        }
        start = at + word.len();
    }
    false
}

fn skeleton(s: &str) -> String {
    s.replace("rn", "m").replace("vv", "w").chars().map(|c| match c {
        '0' => 'o',
        '1' | 'i' | '|' => 'l',
        '5' => 's',
        '3' => 'e',
        _ => c,
    }).collect()
}

pub fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=b.len() {
        d[0][j] = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

fn lookalike(sender: &str, real: &str) -> bool {
    if sender == real {
        return false;
    }
    let name = |d: &str| d.split('.').next().unwrap_or("").to_string();
    let (s, r) = (name(sender), name(real));
    if r.len() < 4 {
        return false;
    }
    if skeleton(&s) == skeleton(&r) && s != r {
        return true;
    }
    let same_tld = sender.split_once('.').map(|x| x.1) == real.split_once('.').map(|x| x.1);
    r.len() >= 5 && same_tld && distance(&s, &r) == 1
}

fn brand_of(name: &str, settings: &Settings) -> Option<(String, Vec<String>)> {
    settings
        .brands
        .iter()
        .cloned()
        .chain(BRANDS.iter().map(|(n, d)| (n.to_string(), d.iter().map(|x| x.to_string()).collect())))
        .find(|(brand, _)| has_word(name, brand))
}

fn all_brands(settings: &Settings) -> Vec<(String, Vec<String>)> {
    settings.brands.iter().cloned().chain(BRANDS.iter().map(|(n, d)| (n.to_string(), d.iter().map(|x| x.to_string()).collect()))).collect()
}

fn title(s: &str) -> String {
    s.split(' ').map(|w| {
        let mut c = w.chars();
        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    }).collect::<Vec<_>>().join(" ")
}

pub struct Body<'a> {
    pub text: &'a str,
    pub links: &'a [(String, String)],
    pub headers: &'a [(String, String)],
    pub attachments: &'a [String],
}

const PROGRAMS: [&str; 14] = ["exe", "scr", "js", "jse", "vbs", "vbe", "bat", "cmd", "msi", "iso", "img", "jar", "hta", "lnk"];
const PAGES: [&str; 5] = ["htm", "html", "shtml", "svg", "xhtml"];

fn header<'a>(headers: &'a [(String, String)], name: &str) -> impl Iterator<Item = &'a str> {
    let name = name.to_lowercase();
    headers.iter().filter(move |(k, _)| k.to_lowercase() == name).map(|(_, v)| v.as_str())
}

pub fn host(url: &str) -> Option<String> {
    let rest = url.trim().split_once("://").filter(|(scheme, _)| matches!(scheme.to_lowercase().as_str(), "http" | "https"))?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = if host.starts_with('[') { host.split(']').next()?.trim_start_matches('[') } else { host.split(':').next()? };
    let host = host.trim_end_matches('.').to_lowercase();
    (!host.is_empty()).then_some(host)
}

fn shown_domain(text: &str) -> Option<String> {
    let t = text.trim().trim_end_matches(['/', '.']).to_lowercase();
    if t.is_empty() || t.contains(char::is_whitespace) || !t.contains('.') {
        return None;
    }
    let with_scheme = if t.contains("://") { t } else { format!("https://{t}") };
    host(&with_scheme).filter(|h| h.contains('.') && h.rsplit('.').next().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic())))
}

fn is_ip(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok()
}

const MAX_LINKS: usize = 300;
const MAX_HOSTS: usize = 40;
const MAX_TEXT: usize = 40_000;

pub fn check_body(m: &Mail, b: &Body, contacts: &[Contact], settings: &Settings, subject_pressure: bool) -> Verdict {
    let mut v = Verdict { score: 0, reasons: Vec::new() };
    let mut add = |n: u32, why: String| {
        v.score += n;
        v.reasons.push(why);
    };
    let dom = domain(m.email);
    if dom.is_empty() || settings.trust.iter().any(|t| belongs(&dom, t)) {
        return v;
    }
    let site = base(&dom);
    if let Some(auth) = header(b.headers, "authentication-results").find(|a| a.to_lowercase().contains("mx.google.com")) {
        let a = auth.to_lowercase();
        if a.contains("dmarc=fail") {
            add(3, format!("Gmail couldn't confirm it really came from {dom} (DMARC failed)"));
        } else if (a.contains("spf=fail") || a.contains("dkim=fail")) && !a.contains("dkim=pass") {
            add(1, format!("Gmail couldn't fully check it came from {dom}"));
        }
    }
    if let Some(reply) = header(b.headers, "reply-to").next() {
        let rd = domain(reply.split(',').next().unwrap_or(reply));
        let rb = base(&rd);
        if !rd.is_empty() && rb != site {
            if PUBLIC.contains(&rb.as_str()) && !PUBLIC.contains(&site.as_str()) {
                add(2, format!("replies go to a personal {rb} address, not {dom}"));
            } else {
                add(1, format!("replies go to {rd}, not {dom}"));
            }
        }
    }
    let brands = all_brands(settings);
    let mut known: Vec<String> = contacts
        .iter()
        .map(|c| base(&domain(&c.email)))
        .filter(|d| !d.is_empty() && !PUBLIC.contains(&d.as_str()))
        .collect();
    known.sort();
    known.dedup();
    let mut seen: Vec<String> = Vec::new();
    let protected = |d: &str| brands.iter().any(|(_, ds)| ds.iter().any(|x| belongs(d, x))) || known.iter().any(|k| belongs(d, k));
    let (mut mismatch, mut ip, mut look) = (false, false, false);
    for (text, url) in b.links.iter().take(MAX_LINKS) {
        let Some(h) = host(url) else { continue };
        let hb = base(&h);
        let fresh = !seen.contains(&h);
        if fresh && seen.len() < MAX_HOSTS {
            seen.push(h.clone());
        }
        if !ip && is_ip(&h) {
            ip = true;
            add(2, format!("a link goes to a bare internet address ({h})"));
        }
        if !mismatch {
            if let Some(shown) = shown_domain(text) {
                let sb = base(&shown);
                if sb != hb && sb != site && protected(&shown) && !protected(&h) {
                    mismatch = true;
                    add(3, format!("a link shows {shown} but goes to {h}"));
                }
            }
        }
        if !look && fresh && seen.last() == Some(&h) && !is_ip(&h) {
            let like = brands.iter().find(|(_, ds)| !ds.iter().any(|d| belongs(&h, d)) && ds.iter().any(|d| lookalike(&hb, &base(d))));
            if let Some((brand, ds)) = like {
                look = true;
                add(3, format!("a link goes to {h}, which looks like {}'s {}", title(brand), base(&ds[0])));
            } else if let Some(k) = known.iter().find(|k| hb != **k && lookalike(&hb, k)) {
                look = true;
                add(3, format!("a link goes to {h}, which looks like {k}"));
            }
        }
    }
    let ext = |name: &str| name.rsplit_once('.').map(|(_, e)| e.trim().to_lowercase()).unwrap_or_default();
    if let Some(f) = b.attachments.iter().find(|f| PROGRAMS.contains(&ext(f).as_str())) {
        add(3, format!("it attaches a program ({f})"));
    } else if let Some(f) = b.attachments.iter().find(|f| PAGES.contains(&ext(f).as_str())) {
        add(2, format!("it attaches a web page ({f})"));
    }
    if !subject_pressure {
        let mut end = b.text.len().min(MAX_TEXT);
        while !b.text.is_char_boundary(end) {
            end -= 1;
        }
        let text = b.text[..end].to_lowercase();
        if let Some((phrase, _)) = PRESSURE.iter().filter(|(_, n)| *n >= 2).find(|(p, _)| text.contains(p)) {
            add(1, format!("the message pushes you to act (\u{201c}{phrase}\u{201d})"));
        }
    }
    v
}

pub fn subject_pressure(subject: &str) -> bool {
    let s = subject.to_lowercase();
    PRESSURE.iter().any(|(p, _)| s.contains(p))
}

pub fn check(m: &Mail, contacts: &[Contact], settings: &Settings) -> Verdict {
    let mut v = Verdict { score: 0, reasons: Vec::new() };
    let mut add = |n: u32, why: String| {
        v.score += n;
        v.reasons.push(why);
    };
    let dom = domain(m.email);
    if dom.is_empty() || settings.trust.iter().any(|t| belongs(&dom, t)) {
        return Verdict { score: 0, reasons: Vec::new() };
    }
    let site = base(&dom);
    let brands = all_brands(settings);
    let owns = |domains: &[String]| domains.iter().any(|d| belongs(&dom, d));

    if dom.split('.').any(|l| l.starts_with("xn--")) {
        add(3, format!("its domain {dom} uses look-alike letters"));
    }
    if !real_tld(&dom) {
        add(3, format!("{dom} isn't a real internet domain"));
    }
    let shown = m.name.trim().trim_matches(['"', '\'']).to_lowercase();
    if !shown.is_empty() {
        let pretends = m.me.iter().map(|a| a.to_lowercase()).any(|a| {
            let local = a.split('@').next().unwrap_or("").to_string();
            (shown == a || shown == local) && !a.eq_ignore_ascii_case(m.email)
        });
        if pretends {
            add(3, format!("it's named as if it came from you ({}), but it came from {dom}", m.name.trim()));
        }
    }
    if let Some(at) = m.name.find('@') {
        let start = m.name[..at].rfind(|c: char| c.is_whitespace() || c == '<' || c == '(' || c == '"').map_or(0, |i| i + 1);
        let shown = domain(m.name[start..].split(|c: char| c.is_whitespace() || c == '>' || c == ')' || c == '"').next().unwrap_or(""));
        if !shown.is_empty() && base(&shown) != site {
            add(3, format!("its name shows an address at {shown}, but it came from {dom}"));
        }
    }
    let claimed = brand_of(m.name, settings);
    if let Some((brand, domains)) = &claimed {
        if !owns(domains) {
            add(3, format!("it says it's from {}, but it came from {dom}", title(brand)));
        }
    }
    let real_brand = claimed.as_ref().is_some_and(|(_, d)| owns(d));
    let personal = PUBLIC.contains(&site.as_str());
    for (brand, domains) in &brands {
        if personal || owns(domains) {
            continue;
        }
        if domains.iter().any(|d| lookalike(&site, &base(d))) {
            add(3, format!("{dom} looks like {}'s {}", title(brand), base(&domains[0])));
            break;
        }
        if brand.len() >= 5 && !brand.contains(' ') && site.split(['.', '-']).any(|part| part == brand.as_str()) {
            add(2, format!("{dom} uses {}'s name but isn't theirs", title(brand)));
            break;
        }
    }
    let mut seen = std::collections::HashSet::new();
    for c in contacts {
        let cd = base(&domain(&c.email));
        if cd.is_empty() || cd == site || PUBLIC.contains(&cd.as_str()) || PUBLIC.contains(&site.as_str()) || !seen.insert(cd.clone()) {
            continue;
        }
        if lookalike(&site, &cd) {
            add(3, format!("{dom} looks like {cd}, which you get mail from"));
            break;
        }
    }
    let name = m.name.trim().to_lowercase();
    if !real_brand && name.split_whitespace().count() >= 2 {
        if let Some(c) = contacts.iter().find(|c| c.name.trim().to_lowercase() == name && base(&domain(&c.email)) != site && !contacts.iter().any(|o| o.name.trim().to_lowercase() == name && base(&domain(&o.email)) == site)) {
            add(2, format!("it uses the name {} but not their address ({})", c.name.trim(), c.email));
        }
    }
    let subject = m.subject.to_lowercase();
    if let Some((phrase, n)) = PRESSURE.iter().find(|(p, _)| subject.contains(p)) {
        add(*n, format!("the subject pushes you to act (\u{201c}{phrase}\u{201d})"));
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(m: (&str, &str), links: &[(&str, &str)], headers: &[(&str, &str)], files: &[&str], text: &str) -> Verdict {
        let links: Vec<(String, String)> = links.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let headers: Vec<(String, String)> = headers.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        let files: Vec<String> = files.iter().map(|f| f.to_string()).collect();
        let contacts = vec![Contact { name: "Dana Smith".into(), email: "dsmith@emco.com".into() }];
        check_body(
            &Mail { name: m.0, email: m.1, subject: "", me: &[] },
            &Body { text, links: &links, headers: &headers, attachments: &files },
            &contacts,
            &settings(""),
            false,
        )
    }

    #[test]
    fn content_signals() {
        let s = |v: Verdict| v.score;
        let shop = ("Shop", "news@shop.example");
        assert_eq!(s(body(shop, &[("https://www.paypal.com/signin", "https://evil.example/x")], &[], &[], "")), 3);
        assert_eq!(s(body(shop, &[("www.emco.com", "http://emco-results.example/x")], &[], &[], "")), 3);
        assert_eq!(s(body(shop, &[("Shop now", "https://click.esp.example/r?u=1"), ("www.shop.example", "https://click.esp.example/r?u=2")], &[], &[], "")), 0);
        assert_eq!(s(body(shop, &[("www.nytimes.com", "https://substack.com/redirect/1")], &[], &[], "")), 0);
        assert_eq!(s(body(shop, &[("www.paypal.com", "https://www.paypal.com/x")], &[], &[], "")), 0);
        assert_eq!(s(body(shop, &[("Sign in", "https://paypa1.com/login")], &[], &[], "")), 3);
        assert_eq!(s(body(shop, &[("Results", "https://ernco.com/r")], &[], &[], "")), 3);
        assert_eq!(s(body(shop, &[("here", "http://185.12.4.9/a")], &[], &[], "")), 2);
        assert_eq!(s(body(shop, &[], &[("Authentication-Results", "mx.google.com; dkim=pass; spf=pass; dmarc=fail (p=REJECT)")], &[], "")), 3);
        assert_eq!(s(body(shop, &[], &[("Authentication-Results", "mx.google.com; dkim=pass header.i=@shop.example; spf=softfail; dmarc=pass")], &[], "")), 0);
        assert_eq!(s(body(shop, &[], &[("Authentication-Results", "mx.google.com; spf=fail; dmarc=none")], &[], "")), 1);
        assert_eq!(s(body(shop, &[], &[("Reply-To", "Boss <boss.urgent@gmail.com>")], &[], "")), 2);
        assert_eq!(s(body(shop, &[], &[("Reply-To", "reply@shop-esp.example")], &[], "")), 1);
        assert_eq!(s(body(shop, &[], &[("Reply-To", "help@mail.shop.example")], &[], "")), 0);
        assert_eq!(s(body(shop, &[], &[], &["Invoice.pdf.exe"], "")), 3);
        assert_eq!(s(body(shop, &[], &[], &["Remittance.HTML"], "")), 2);
        assert_eq!(s(body(shop, &[], &[], &["report.pdf"], "Please verify your account today")), 1);
        assert_eq!(host("https://user@Login.Example.com:8443/a?b"), Some("login.example.com".into()));
        assert_eq!(host("mailto:x@y.z"), None);
        assert_eq!(shown_domain("https://www.paypal.com/signin"), Some("www.paypal.com".into()));
        assert_eq!(shown_domain("Click here"), None);
        assert_eq!(shown_domain("v1.2"), None);
    }

    fn run(name: &str, email: &str, subject: &str) -> Verdict {
        let contacts = vec![
            Contact { name: "Dana Smith".into(), email: "dsmith@emco.com".into() },
            Contact { name: "Jim Lane".into(), email: "jlane@emco.com".into() },
            Contact { name: "Shop".into(), email: "news@gmail.com".into() },
        ];
        let me = vec!["pat@example.com".to_string()];
        check(&Mail { name, email, subject, me: &me }, &contacts, &settings("trust = \"partner.example\"\nbrand = \"Acme Bank: acmebank.com\"\n"))
    }

    fn flagged(v: &Verdict) -> bool {
        v.score >= THRESHOLD
    }

    #[test]
    fn flags_impersonation() {
        let v = run("PayPal", "service@paypa1-help.net", "Your receipt");
        assert!(flagged(&v), "{v:?}");
        assert!(v.reasons[0].contains("says it's from Paypal"), "{v:?}");
        assert!(flagged(&run("Amazon Support", "help@arnazon.com", "Order")));
        assert!(flagged(&run("Billing", "billing@paypal-secure-login.com", "Action required")));
        assert!(flagged(&run("Dana Smith", "dan.s@freemail.xyz", "Quick favor")));
        assert!(!flagged(&run("Dana Smith", "dan.s@freemail.xyz", "Photo log")));
        assert!(flagged(&run("Lab", "results@ernco.com", "Results")));
        assert!(flagged(&run("service@paypal.com", "x@random.ru", "hi")));
        assert!(flagged(&run("Support", "help@xn--pypal-4ve.com", "hi")));
        assert!(flagged(&run("Acme Bank", "alerts@acme-bank-alerts.com", "hi")));
        let blocked = run("pat", "alert-357-934232@qdibo.khw", "We've blocked your account! 🚫Your photos and videos will bedeleted on 23-11-2026 ⚠️ Renewyour subscription for free now!");
        assert!(flagged(&blocked), "{blocked:?}");
        assert!(blocked.reasons.iter().any(|r| r.contains("isn't a real internet domain")), "{blocked:?}");
        assert!(blocked.reasons.iter().any(|r| r.contains("as if it came from you")), "{blocked:?}");
        assert!(flagged(&run("GmailSupportTeam", "alert-357-934232@gvxdk.bij", "We've Blocked Your Account!")));
    }

    #[test]
    fn leaves_real_mail_alone() {
        for (name, email, subject) in [
            ("PayPal", "service@paypal.com", "You've got money"),
            ("Amazon.com", "shipment-tracking@amazon.com", "Your package"),
            ("Apple", "no_reply@email.apple.com", "Your receipt"),
            ("Google", "no-reply@accounts.google.com", "Security alert"),
            ("Microsoft account team", "account-security-noreply@accountprotection.microsoft.com", "Unusual sign-in activity"),
            ("Wells Fargo", "alerts@notify.wellsfargo.com", "Your statement"),
            ("Dana Smith", "dsmith@emco.com", "Photo log"),
            ("Partner", "a@mail.partner.example", "Verify your account"),
            ("Group Updates", "news@updates.example.com", "This week"),
            ("Burger Barn", "deals@burgerbarn.com", "Half off"),
            ("The Weekly Byte", "weeklybyte@substack.com", "How teams build software"),
            ("Garden Monthly", "news@gardenmonthly.com", "Urgent: your roses"),
            ("Helpdesk", "it@nowhere.biz", "Verify your account now"),
            ("Microsoft OneDrive", "email@mail.onedrive.com", "Back up your files"),
            ("HostCo Billing", "billing@hostco.com", "Renewal Notice"),
            ("Pal", "someone@gmail.com", "hi"),
            ("Corner Shop", "cornershop@ymail.com", "T1"),
            ("QuickBooks for Mac", "IntuitNotices@e.intuit.com", "Urgent Notice"),
            ("Microsoft Advertising", "noreply@microsoftadvertising.com", "[REGISTER NOW] AI readiness"),
            ("Jamie Rivera", "jamie@mail.dealsdigest.xyz", "New listings this week"),
            ("City Library", "alerts@libraryinfo.org", "Reservation Cancellation"),
            ("Pat Example", "pat@example.com", "note to self"),
            ("Cron", "root@macbook.local", "backup done"),
            ("na", "na@ip-10-0-0-12.ec2.internal", "alarm"),
            ("Amazon Web Services", "health@aws.com", "Action may be required"),
            ("Google Fitbit", "noreply@e.fitbit.com", "Fitbit Air is here"),
        ] {
            let v = run(name, email, subject);
            assert!(!flagged(&v), "{name} <{email}>: {v:?}");
        }
    }

    #[test]
    fn domains() {
        assert_eq!(base("mail.service.paypal.com"), "paypal.com");
        assert_eq!(base("shop.amazon.co.uk"), "amazon.co.uk");
        assert_eq!(distance("paypal", "paypla"), 1);
        assert_eq!(distance("amazon", "arnazon"), 2);
        assert!(lookalike("arnazon.com", "amazon.com"));
        assert!(lookalike("paypa1.com", "paypal.com"));
        assert!(!lookalike("paypal.com", "paypal.com"));
        assert!(!lookalike("ups.com", "usps.com"));
        assert!(!lookalike("appriver.com", "appgyver.com"));
    }

    #[test]
    fn reads_settings() {
        let s = settings("label = \"Phishy\"\ntrust = \"a.com\"\n# trust = \"b.com\"\nbrand = \"Acme: acme.com acme.net\"\n");
        assert_eq!(s.label, "Phishy");
        assert_eq!(s.trust, ["a.com"]);
        assert_eq!(s.brands, [("acme".to_string(), vec!["acme.com".to_string(), "acme.net".to_string()])]);
        assert_eq!(settings("").label, "Suspicious");
    }
}


