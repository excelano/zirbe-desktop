// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{Account, FolderInfo, Message};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Mailbox {
    pub name: String,
    pub uid_validity: u32,
    pub messages: BTreeMap<u32, Message>,
    #[serde(default)]
    pub found: BTreeSet<u32>,
}

const FORMAT: u32 = 4;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AccountData {
    pub format: u32,
    pub inbox: Mailbox,
    pub sent: Option<Mailbox>,
    pub others: BTreeMap<String, Mailbox>,
    pub folders: Vec<FolderInfo>,
    pub unfiled: Vec<Message>,
    pub started: BTreeSet<String>,
    pub pinned: BTreeSet<String>,
    pub unsent: Vec<UnsentCopy>,
    pub blocked: BTreeSet<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct UnsentCopy {
    pub message_id: String,
    pub raw: String,
    #[serde(default)]
    pub saved: bool,
}

impl AccountData {
    fn mailboxes_mut(&mut self) -> impl Iterator<Item = &mut Mailbox> {
        std::iter::once(&mut self.inbox)
            .chain(self.sent.as_mut())
            .chain(self.others.values_mut())
    }

    pub fn mailbox_mut(&mut self, name: &str) -> Option<&mut Mailbox> {
        self.mailboxes_mut().find(|mailbox| mailbox.name == name)
    }

    pub fn sent_name(&self) -> Option<&str> {
        self.sent.as_ref().map(|sent| sent.name.as_str())
    }

    pub fn messages(&self) -> Vec<Message> {
        std::iter::once(&self.inbox)
            .chain(self.sent.as_ref())
            .chain(self.others.values())
            .flat_map(|mailbox| mailbox.messages.values())
            .chain(&self.unfiled)
            .cloned()
            .collect()
    }
}

pub(crate) fn file_name(account: &str) -> String {
    let safe: String = account
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.json")
}

pub(crate) fn load_accounts(path: &Path) -> Vec<Account> {
    read(path).unwrap_or_default()
}

pub(crate) fn save_accounts(path: &Path, accounts: &[Account]) -> Result<(), String> {
    write(path, accounts)
}

pub(crate) fn load_data(path: &Path) -> AccountData {
    let mut data: AccountData = read(path).unwrap_or_default();
    if data.format < FORMAT {
        for mailbox in data.mailboxes_mut() {
            mailbox.messages.clear();
            mailbox.uid_validity = 0;
        }
        data.format = FORMAT;
    }
    let sent = data.sent_name().unwrap_or_default().to_string();
    for message in &mut data.unfiled {
        if message.mailbox.is_empty() {
            message.mailbox = sent.clone();
        }
    }
    data
}

pub(crate) fn save_data(path: &Path, data: &AccountData) -> Result<(), String> {
    write(path, data)
}

fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

fn write<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let partial = path.with_extension("partial");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options.open(&partial).map_err(|error| error.to_string())?;
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    fs::rename(&partial, path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::load_data;

    #[test]
    fn refetches_the_messages_of_an_older_cache_and_files_its_unsent_copies() {
        let message = |uid: u32| {
            format!(
                r#"{{"folder":"Inbox","uid":{uid},"message_id":"<{uid}@x>","in_reply_to":null,"references":[],"from":null,"to":[],"cc":[],"subject":"Hi","date":"2026-11-09T12:00:00Z","text":"","attachments":[],"seen":true,"partial":false}}"#
            )
        };
        let json = format!(
            r#"{{"inbox":{{"name":"INBOX","uid_validity":1,"messages":{{"1":{}}}}},"sent":{{"name":"INBOX.Sent","uid_validity":1,"messages":{{"2":{}}}}},"unfiled":[{}],"started":[]}}"#,
            message(1),
            message(2),
            message(0)
        );
        let path =
            std::env::temp_dir().join(format!("zirbe-cache-test-{}.json", std::process::id()));
        std::fs::write(&path, json).expect("a written cache");
        let data = load_data(&path);
        let _ = std::fs::remove_file(&path);
        assert!(data.inbox.messages.is_empty());
        assert_eq!(data.inbox.uid_validity, 0);
        let sent = data.sent.as_ref().expect("a sent mailbox");
        assert_eq!(sent.name, "INBOX.Sent");
        assert!(sent.messages.is_empty());
        assert_eq!(data.unfiled[0].mailbox, "INBOX.Sent");
        assert_eq!(data.format, super::FORMAT);
    }
}
