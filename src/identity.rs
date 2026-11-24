//! Identities: the From addresses one account can send as, like
//! Thunderbird's. Each has its own name and its own default signatures,
//! so a work address goes out with its work signature and the Gmail
//! address with another, from the same account.
//!
//! The account's own address is the main identity; its signatures are the
//! defaults under Settings → Writing. Extra ones are files in
//! `identities/<email>.toml`. Gmail only sends from an extra address once
//! it's added under Gmail Settings → Accounts → "Send mail as"; until then
//! it rewrites the From to the account address.

use crate::files::{data_dir, parse, quote};
use anyhow::Result;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    /// 0 = the account's own address.
    pub id: i64,
    pub name: String,
    pub email: String,
    pub sig_new: Option<i64>,
    pub sig_reply: Option<i64>,
    /// The account it sends through ("" = the first account).
    pub account: String,
}

impl Identity {
    pub fn is_main(&self) -> bool {
        self.id == 0
    }

    /// "Name <email>" for the From picker.
    pub fn label(&self) -> String {
        if self.name.trim().is_empty() { self.email.clone() } else { format!("{} <{}>", self.name.trim(), self.email) }
    }
}

pub fn dir() -> PathBuf {
    data_dir().join("identities")
}

fn file_name(email: &str) -> String {
    let clean: String = email.chars().map(|c| if c.is_alphanumeric() || "@.-_+".contains(c) { c } else { '-' }).collect();
    format!("{}.toml", if clean.is_empty() { "identity".into() } else { clean })
}

fn read(path: &std::path::Path) -> Option<Identity> {
    let m = parse(&std::fs::read_to_string(path).ok()?);
    let id = |k: &str| m.get(k).and_then(|v| v.parse::<i64>().ok()).filter(|i| *i != 0);
    Some(Identity {
        id: m.get("id")?.parse().ok().filter(|i: &i64| *i > 0)?,
        name: m.get("name").cloned().unwrap_or_default(),
        email: m.get("email").cloned().unwrap_or_default(),
        sig_new: id("signature_for_new"),
        sig_reply: id("signature_for_replies"),
        account: m.get("account").map(|a| a.trim().to_lowercase()).unwrap_or_default(),
    })
}

/// The extra identities (not the main one), in the order they were added.
pub fn load() -> Vec<Identity> {
    let mut out: Vec<Identity> = std::fs::read_dir(dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "toml"))
                .filter_map(|p| read(&p))
                .collect()
        })
        .unwrap_or_default();
    out.sort_by_key(|i| i.id);
    out
}

fn path_of(id: i64) -> Option<PathBuf> {
    std::fs::read_dir(dir())
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| read(p).is_some_and(|i| i.id == id))
}

/// Save (id 0 = new). The file follows the email address. Returns the id.
pub fn save(identity: &Identity) -> Result<i64> {
    std::fs::create_dir_all(dir())?;
    let mut i = identity.clone();
    if i.id == 0 {
        i.id = load().iter().map(|x| x.id).max().unwrap_or(0) + 1;
    }
    let target = dir().join(file_name(&i.email));
    if let Some(old) = path_of(i.id).filter(|p| p != &target) {
        std::fs::remove_file(old).ok();
    }
    let sig = |v: Option<i64>| v.unwrap_or(0);
    let text = format!(
        "# A From address for Wax. Gmail sends from it only after it's added\n\
         # under Gmail Settings → Accounts → \"Send mail as\".\n\
         id = {}\n\
         name = {}\n\
         email = {}\n\
         # The account it sends through.\n\
         account = {}\n\
         # Signature ids (signatures/*/signature.toml); 0 = none.\n\
         signature_for_new = {}\n\
         signature_for_replies = {}\n",
        i.id,
        quote(&i.name),
        quote(&i.email),
        quote(&i.account),
        sig(i.sig_new),
        sig(i.sig_reply)
    );
    let tmp = target.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &target)?;
    Ok(i.id)
}

pub fn delete(id: i64) -> Result<()> {
    if let Some(p) = path_of(id) {
        std::fs::remove_file(p)?;
    }
    Ok(())
}

/// The identity a reply should come from: the first of ours the original
/// was addressed to.
pub fn for_reply<'a>(all: &'a [Identity], to: &[(String, String)], cc: &[(String, String)]) -> Option<&'a Identity> {
    to.iter()
        .chain(cc)
        .find_map(|(_, e)| all.iter().find(|i| i.email.eq_ignore_ascii_case(e.trim())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_comes_from_the_address_it_was_sent_to() {
        let main = Identity { id: 0, name: "M".into(), email: "me@gmail.com".into(), sig_new: None, sig_reply: None, account: String::new() };
        let work = Identity { id: 1, name: "M".into(), email: "Alex@Example.com".into(), sig_new: Some(2), sig_reply: Some(2), account: String::new() };
        let all = vec![main, work];
        let to = vec![("".to_string(), "sam@client.example".to_string()), ("".into(), "alex@example.com".into())];
        assert_eq!(for_reply(&all, &to, &[]).map(|i| i.id), Some(1));
        assert_eq!(for_reply(&all, &[("".into(), "x@y.com".into())], &[]), None);
    }
}
