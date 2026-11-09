// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::Path;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::model::{Account, Message};

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct Mailbox {
    pub name: String,
    pub uid_validity: u32,
    pub messages: BTreeMap<u32, Message>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct AccountData {
    pub inbox: Mailbox,
    pub sent: Option<Mailbox>,
    pub unfiled: Vec<Message>,
    pub started: BTreeSet<String>,
}

impl AccountData {
    pub fn messages(&self) -> Vec<Message> {
        self.inbox
            .messages
            .values()
            .chain(self.sent.iter().flat_map(|sent| sent.messages.values()))
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
    read(path).unwrap_or_default()
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
