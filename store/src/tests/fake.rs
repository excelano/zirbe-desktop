// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::Notify;

use crate::cache::Mailbox;
use crate::folders::{self, Listed};
use crate::mime::{self, Body, Fetched};
use crate::model::{Account, FolderInfo, INBOX, Role};
use crate::server::{Server, Session};

pub(crate) const SENT: &str = "INBOX.Sent";
pub(crate) const DRAFTS: &str = "INBOX.Drafts";
pub(crate) const TRASH: &str = "INBOX.Trash";
pub(crate) const SPAM: &str = "INBOX.spam";

pub(crate) struct Stored {
    pub raw: Vec<u8>,
    pub seen: bool,
    pub flagged: bool,
}

pub(crate) struct Folder {
    pub role: Option<Role>,
    pub validity: u32,
    pub next: u32,
    pub messages: BTreeMap<u32, Stored>,
}

impl Folder {
    fn new(role: Option<Role>) -> Folder {
        Folder {
            role,
            validity: 1,
            next: 1,
            messages: BTreeMap::new(),
        }
    }

    fn add(&mut self, raw: Vec<u8>, seen: bool) -> u32 {
        let uid = self.next;
        self.next += 1;
        self.messages.insert(
            uid,
            Stored {
                raw,
                seen,
                flagged: false,
            },
        );
        uid
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub password: String,
    pub folders: BTreeMap<String, Folder>,
    pub log: Vec<String>,
    pub sent: Vec<Vec<u8>>,
    pub failing: BTreeSet<&'static str>,
    pub files_sent: bool,
    pub keyring: BTreeMap<String, String>,
}

impl State {
    fn attempt(&mut self, operation: &'static str, entry: String) -> Result<(), String> {
        self.log.push(entry);
        if self.failing.contains(operation) {
            return Err(format!("the fake server failed {operation}"));
        }
        Ok(())
    }

    fn folder(&mut self, name: &str) -> Result<&mut Folder, String> {
        self.folders
            .get_mut(name)
            .ok_or_else(|| format!("the fake server has no folder {name}"))
    }
}

#[derive(Clone)]
pub(crate) struct Fake {
    state: Arc<Mutex<State>>,
    news: Arc<Notify>,
}

impl Fake {
    pub(crate) fn new(password: &str) -> Fake {
        let mut folders = BTreeMap::new();
        folders.insert(INBOX.to_string(), Folder::new(None));
        folders.insert(SENT.to_string(), Folder::new(Some(Role::Sent)));
        folders.insert(DRAFTS.to_string(), Folder::new(Some(Role::Drafts)));
        folders.insert(TRASH.to_string(), Folder::new(Some(Role::Trash)));
        folders.insert(SPAM.to_string(), Folder::new(Some(Role::Junk)));
        Fake {
            state: Arc::new(Mutex::new(State {
                password: password.to_string(),
                folders,
                ..State::default()
            })),
            news: Arc::new(Notify::new()),
        }
    }

    pub(crate) fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("fake server lock")
    }

    pub(crate) fn deliver(&self, folder: &str, raw: &str) -> u32 {
        let uid = self
            .state()
            .folder(folder)
            .expect("a folder to deliver to")
            .add(raw.as_bytes().to_vec(), false);
        if folder == INBOX {
            self.news.notify_one();
        }
        uid
    }

    pub(crate) fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut self.state().log)
    }

    pub(crate) fn uids(&self, folder: &str) -> Vec<u32> {
        self.state()
            .folders
            .get(folder)
            .map(|folder| folder.messages.keys().copied().collect())
            .unwrap_or_default()
    }

    pub(crate) fn fail(&self, operation: &'static str, on: bool) {
        let mut state = self.state();
        if on {
            state.failing.insert(operation);
        } else {
            state.failing.remove(operation);
        }
    }

    fn check(&self, account: &Account, password: &str) -> Result<(), String> {
        let state = self.state();
        if password != state.password {
            return Err(format!(
                "{} refused the address or password",
                account.imap_host
            ));
        }
        Ok(())
    }
}

impl Server for Fake {
    type Session = FakeSession;

    async fn open_keyring(&self) -> Result<(), String> {
        Ok(())
    }

    async fn password(&self, account: &str) -> Result<String, String> {
        self.state()
            .keyring
            .get(account)
            .cloned()
            .ok_or_else(|| format!("no password is saved for {account}"))
    }

    async fn save_password(&self, account: &str, password: &str) -> Result<(), String> {
        self.state()
            .keyring
            .insert(account.to_string(), password.to_string());
        Ok(())
    }

    async fn forget_password(&self, account: &str) -> Result<(), String> {
        self.state().keyring.remove(account);
        Ok(())
    }

    async fn connect(&self, account: &Account, password: &str) -> Result<FakeSession, String> {
        if self.state().failing.contains("connect") {
            return Err(format!("{} could not be reached", account.imap_host));
        }
        self.check(account, password)?;
        Ok(FakeSession { fake: self.clone() })
    }

    async fn check_smtp(&self, account: &Account, password: &str) -> Result<(), String> {
        self.check(account, password)
    }

    async fn send(
        &self,
        _account: &Account,
        _password: &str,
        message: lettre::Message,
    ) -> Result<(), String> {
        let mut state = self.state();
        state.attempt("send", "send".into())?;
        state.sent.push(message.formatted());
        if state.files_sent {
            state.folder(SENT)?.add(message.formatted(), true);
        }
        Ok(())
    }
}

pub(crate) struct FakeSession {
    fake: Fake,
}

impl FakeSession {
    fn state(&self) -> MutexGuard<'_, State> {
        self.fake.state()
    }
}

fn parse(name: &str, uid: u32, stored: &Stored) -> Option<crate::model::Message> {
    let mut message = mime::parse(&Fetched {
        mailbox: name,
        uid,
        seen: stored.seen,
        header: &stored.raw,
        body: Body::Whole,
        arrived: Some(Utc::now()),
    })?;
    message.flagged = stored.flagged;
    Some(message)
}

impl Session for FakeSession {
    async fn close(self) {}

    async fn folders(&mut self, _host: &str) -> Result<Vec<FolderInfo>, String> {
        let mut state = self.state();
        state.attempt("folders", "folders".into())?;
        let listed = state
            .folders
            .iter()
            .map(|(name, folder)| Listed {
                name: name.clone(),
                delimiter: Some(".".into()),
                selectable: true,
                role: folder.role,
            })
            .collect();
        let mut arranged = folders::arrange(listed);
        for info in &mut arranged {
            if let Some(folder) = state.folders.get(&info.name) {
                info.unread = folder.messages.values().filter(|m| !m.seen).count() as u32;
            }
        }
        Ok(arranged)
    }

    async fn sync(&mut self, _host: &str, mailbox: &mut Mailbox) -> Result<Vec<u32>, String> {
        let mut state = self.state();
        state.attempt("sync", format!("sync {}", mailbox.name))?;
        let folder = state.folder(&mailbox.name)?;
        if folder.validity != mailbox.uid_validity {
            mailbox.messages.clear();
            mailbox.found.clear();
            mailbox.uid_validity = folder.validity;
        }
        let known = !mailbox.messages.is_empty();
        mailbox
            .messages
            .retain(|uid, _| folder.messages.contains_key(uid));
        let mut fresh = Vec::new();
        for (uid, stored) in &folder.messages {
            match mailbox.messages.get_mut(uid) {
                Some(message) => {
                    message.seen = stored.seen;
                    message.flagged = stored.flagged;
                }
                None => {
                    if let Some(message) = parse(&mailbox.name, *uid, stored) {
                        mailbox.messages.insert(*uid, message);
                        fresh.push(*uid);
                    }
                }
            }
        }
        Ok(if known { fresh } else { Vec::new() })
    }

    async fn search_into(
        &mut self,
        _host: &str,
        mailbox: &mut Mailbox,
        terms: &[String],
    ) -> Result<usize, String> {
        let mut state = self.state();
        state.attempt("search", format!("search {}", mailbox.name))?;
        let folder = state.folder(&mailbox.name)?;
        let mut found = 0;
        for (uid, stored) in &folder.messages {
            let text = String::from_utf8_lossy(&stored.raw).to_lowercase();
            if terms.iter().all(|term| text.contains(&term.to_lowercase()))
                && let Some(message) = parse(&mailbox.name, *uid, stored)
            {
                mailbox.messages.insert(*uid, message);
                mailbox.found.insert(*uid);
                found += 1;
            }
        }
        Ok(found)
    }

    async fn watch(
        self,
        _host: &str,
        mailbox: &str,
        patience: Duration,
    ) -> Result<(FakeSession, u32, bool), String> {
        let news = Arc::clone(&self.fake.news);
        let woke = tokio::time::timeout(patience, news.notified())
            .await
            .is_ok();
        let exists = self
            .state()
            .folders
            .get(mailbox)
            .map_or(0, |folder| folder.messages.len() as u32);
        Ok((self, exists, woke))
    }

    async fn whole(&mut self, _host: &str, mailbox: &str, uid: u32) -> Result<Vec<u8>, String> {
        let mut state = self.state();
        state.attempt("whole", format!("whole {mailbox} {uid}"))?;
        state
            .folder(mailbox)?
            .messages
            .get(&uid)
            .map(|stored| stored.raw.clone())
            .ok_or_else(|| "the message is no longer on the server".into())
    }

    async fn set_flag(
        &mut self,
        _host: &str,
        mailbox: &str,
        uids: &[u32],
        flag: &str,
        on: bool,
    ) -> Result<(), String> {
        let mut state = self.state();
        state.attempt("flag", format!("flag {mailbox} {uids:?} {flag} {on}"))?;
        for uid in uids {
            if let Some(stored) = state.folder(mailbox)?.messages.get_mut(uid) {
                match flag {
                    "\\Seen" => stored.seen = on,
                    "\\Flagged" => stored.flagged = on,
                    _ => {}
                }
            }
        }
        Ok(())
    }

    async fn move_to(
        &mut self,
        _host: &str,
        mailbox: &str,
        uids: &[u32],
        destination: &str,
    ) -> Result<(), String> {
        let mut state = self.state();
        state.attempt("move", format!("move {mailbox} {uids:?} -> {destination}"))?;
        state.folder(destination)?;
        for uid in uids {
            if let Some(stored) = state.folder(mailbox)?.messages.remove(uid) {
                let target = state.folder(destination)?;
                let moved = target.add(stored.raw, stored.seen);
                if let Some(copy) = target.messages.get_mut(&moved) {
                    copy.flagged = stored.flagged;
                }
            }
        }
        Ok(())
    }

    async fn delete(&mut self, _host: &str, mailbox: &str, uids: &[u32]) -> Result<(), String> {
        let mut state = self.state();
        state.attempt("delete", format!("delete {mailbox} {uids:?}"))?;
        let folder = state.folder(mailbox)?;
        for uid in uids {
            folder.messages.remove(uid);
        }
        Ok(())
    }

    async fn empty(&mut self, _host: &str, mailbox: &str, highest: u32) -> Result<(), String> {
        let mut state = self.state();
        state.attempt("empty", format!("empty {mailbox} {highest}"))?;
        state
            .folder(mailbox)?
            .messages
            .retain(|uid, _| *uid > highest);
        Ok(())
    }

    async fn create(&mut self, _host: &str, mailbox: &str) -> Result<(), String> {
        let mut state = self.state();
        state.attempt("create", format!("create {mailbox}"))?;
        state
            .folders
            .entry(mailbox.to_string())
            .or_insert_with(|| Folder::new(None));
        Ok(())
    }

    async fn append(&mut self, host: &str, mailbox: &str, raw: &[u8]) -> Result<(), String> {
        self.append_with(host, mailbox, raw, "(\\Seen)").await
    }

    async fn append_with(
        &mut self,
        _host: &str,
        mailbox: &str,
        raw: &[u8],
        flags: &str,
    ) -> Result<(), String> {
        let mut state = self.state();
        state.attempt("append", format!("append {mailbox}"))?;
        state
            .folder(mailbox)?
            .add(raw.to_vec(), flags.contains("\\Seen"));
        Ok(())
    }
}
