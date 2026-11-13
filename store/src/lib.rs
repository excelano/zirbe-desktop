// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#![forbid(unsafe_code)]

mod actions;
mod cache;
mod compose;
mod content;
mod email;
mod folders;
mod imap;
mod invite;
mod mime;
mod model;
mod presets;
mod search;
mod secrets;
mod smtp;
mod subject;
mod thread;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock, mpsc as reply};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc;

pub use compose::{parse_people, reaction, reply};
pub use model::{
    Account, Arrival, COLOURS, Change, Conversation, Copy, Draft, FolderInfo, INBOX, Invite,
    Message, Original, Outgoing, Person, Role, Scope, Status, When,
};
pub use presets::{Preset, for_address};
pub use search::Query;
pub use subject::normalize as subject_of;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use cache::{AccountData, Mailbox, UnsentCopy};
use imap::Connection;
use mime::{Body, Fetched};

const SYNC_EVERY: Duration = Duration::from_secs(120);
// Short enough that a server never parks the idling connection, which on
// cPanel's Dovecot stops new-mail notices after about thirty seconds.
const WATCH_PATIENCE: Duration = Duration::from_secs(25);
const WATCH_RETRY: Duration = Duration::from_secs(60);

pub type Reply<T = ()> = reply::Receiver<Result<T, String>>;

type Target = (String, u32);

pub trait MailSource {
    fn status(&self) -> Status;
    fn accounts(&self) -> Vec<Account>;
    fn conversations(&self) -> Arc<Vec<Conversation>>;
    fn folders(&self, account: &str) -> Vec<FolderInfo>;
    fn open(&self, scope: &Scope);
    fn send(&self, draft: Draft) -> Reply;
    fn mark_read(&self, conversation: &Conversation);
    fn change(&self, conversations: &[Conversation], scope: &Scope, change: Change);
    fn empty(&self, account: &str, mailbox: &str);
    fn arrivals(&self) -> Vec<Arrival>;
    fn search_server(&self, scope: &Scope, text: &str) -> Reply<usize>;
    fn original(&self, account: &str, message: &Message) -> Reply<Original>;
    fn locked(&self, account: &str) -> bool;
    fn blocked(&self, account: &str) -> Vec<String>;
    fn update_account(&self, account: Account);
    fn block(&self, account: &str, address: &str, on: bool);
    fn save_draft(&self, draft: Draft) -> Reply;
    fn refresh(&self);
    fn transcribed(&self, account: &str, message: &Message, text: String);
    fn email(&self, account: &str, message: &Message) -> Reply<String>;
    fn attachment(&self, account: &str, message: &Message, index: usize) -> Reply<Vec<u8>>;
}

struct Located {
    mailbox: String,
    uid: u32,
    message_id: Option<String>,
}

impl Located {
    fn of(message: &Message) -> Located {
        Located {
            mailbox: message.mailbox.clone(),
            uid: message.uid,
            message_id: message.message_id.clone(),
        }
    }
}

pub struct Store {
    shared: Arc<Shared>,
    commands: mpsc::UnboundedSender<Command>,
}

struct Shared {
    published: RwLock<Snapshot>,
    status: RwLock<Status>,
    arrivals: Mutex<Vec<Arrival>>,
    leaving: Mutex<HashSet<(String, String)>>,
    notify: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn hide(&self, account: &str, keys: &[String]) {
        self.leaving
            .lock()
            .expect("leaving lock")
            .extend(keys.iter().map(|key| (account.to_string(), key.clone())));
        let mut published = self.published.write().expect("published lock");
        published.conversations = self.present(published.conversations.as_ref().clone());
    }

    fn unhide(&self, account: &str, keys: &[String]) {
        self.leaving
            .lock()
            .expect("leaving lock")
            .retain(|(owner, key)| owner != account || !keys.contains(key));
    }

    fn present(&self, conversations: Vec<Conversation>) -> Arc<Vec<Conversation>> {
        let leaving = self.leaving.lock().expect("leaving lock");
        Arc::new(
            conversations
                .into_iter()
                .filter(|c| !leaving.contains(&(c.account.clone(), c.key.clone())))
                .collect(),
        )
    }
}

#[derive(Default)]
struct Snapshot {
    accounts: Vec<Account>,
    conversations: Arc<Vec<Conversation>>,
    folders: HashMap<String, Vec<FolderInfo>>,
    locked: BTreeSet<String>,
    blocked: HashMap<String, Vec<String>>,
}

enum Command {
    Refresh,
    Transcribed {
        account: String,
        message_id: String,
        text: String,
    },
    UpdateAccount(Account),
    Block {
        account: String,
        address: String,
        on: bool,
    },
    SignInAgain {
        account: String,
        password: String,
        reply: reply::Sender<Result<(), String>>,
    },
    Original {
        account: String,
        at: Located,
        reply: reply::Sender<Result<Original, String>>,
    },
    SaveDraft {
        draft: Draft,
        reply: reply::Sender<Result<(), String>>,
    },
    SearchServer {
        places: Vec<(String, String)>,
        terms: Vec<String>,
        reply: reply::Sender<Result<usize, String>>,
    },
    Arrived(String),
    Open(Scope),
    SignIn {
        account: Account,
        password: String,
        reply: reply::Sender<Result<(), String>>,
    },
    SignOut {
        account: String,
    },
    Send {
        draft: Draft,
        reply: reply::Sender<Result<(), String>>,
    },
    Change {
        account: String,
        keys: Vec<String>,
        ids: Vec<String>,
        change: Change,
        targets: Vec<Target>,
    },
    Empty {
        account: String,
        mailbox: String,
        keys: Vec<String>,
        highest: u32,
    },
    Email {
        account: String,
        at: Located,
        reply: reply::Sender<Result<String, String>>,
    },
    Attachment {
        account: String,
        at: Located,
        index: usize,
        name: String,
        reply: reply::Sender<Result<Vec<u8>, String>>,
    },
}

impl Store {
    pub fn open(
        config_dir: PathBuf,
        cache_dir: PathBuf,
        notify: impl Fn() + Send + Sync + 'static,
    ) -> Store {
        let shared = Arc::new(Shared {
            published: RwLock::new(Snapshot::default()),
            status: RwLock::new(Status::Syncing),
            arrivals: Mutex::new(Vec::new()),
            leaving: Mutex::new(HashSet::new()),
            notify: Box::new(notify),
        });
        let (commands, receiver) = mpsc::unbounded_channel();
        let worker = Worker {
            shared: Arc::clone(&shared),
            accounts_path: config_dir.join("accounts.json"),
            cache_dir,
            accounts: Vec::new(),
            data: HashMap::new(),
            open: Scope::AllInboxes,
            own: commands.clone(),
            watchers: HashMap::new(),
            locked: BTreeSet::new(),
        };
        std::thread::Builder::new()
            .name("mail sync".into())
            .spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("a runtime for mail sync")
                    .block_on(worker.run(receiver));
            })
            .expect("a thread for mail sync");
        Store { shared, commands }
    }

    pub fn sign_in(&self, account: Account, password: String) -> Reply {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::SignIn {
            account,
            password,
            reply: sender,
        });
        receiver
    }

    pub fn sign_in_again(&self, account: &str, password: String) -> Reply {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::SignInAgain {
            account: account.to_string(),
            password,
            reply: sender,
        });
        receiver
    }

    pub fn sign_out(&self, account: &Account) {
        let _ = self.commands.send(Command::SignOut {
            account: account.key(),
        });
    }
}

impl MailSource for Store {
    fn status(&self) -> Status {
        self.shared.status.read().expect("status lock").clone()
    }

    fn accounts(&self) -> Vec<Account> {
        self.shared
            .published
            .read()
            .expect("published lock")
            .accounts
            .clone()
    }

    fn conversations(&self) -> Arc<Vec<Conversation>> {
        Arc::clone(
            &self
                .shared
                .published
                .read()
                .expect("published lock")
                .conversations,
        )
    }

    fn send(&self, draft: Draft) -> Reply {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::Send {
            draft,
            reply: sender,
        });
        receiver
    }

    fn folders(&self, account: &str) -> Vec<FolderInfo> {
        self.shared
            .published
            .read()
            .expect("published lock")
            .folders
            .get(account)
            .cloned()
            .unwrap_or_default()
    }

    fn open(&self, scope: &Scope) {
        let _ = self.commands.send(Command::Open(scope.clone()));
    }

    fn search_server(&self, scope: &Scope, text: &str) -> Reply<usize> {
        let places = match scope {
            Scope::AllInboxes => self
                .accounts()
                .iter()
                .map(|account| (account.key(), INBOX.to_string()))
                .collect(),
            Scope::Mailbox { account, name } => vec![(account.clone(), name.clone())],
        };
        let terms = text.split_whitespace().map(str::to_string).collect();
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::SearchServer {
            places,
            terms,
            reply: sender,
        });
        receiver
    }

    fn locked(&self, account: &str) -> bool {
        self.shared
            .published
            .read()
            .expect("published lock")
            .locked
            .contains(account)
    }

    fn blocked(&self, account: &str) -> Vec<String> {
        self.shared
            .published
            .read()
            .expect("published lock")
            .blocked
            .get(account)
            .cloned()
            .unwrap_or_default()
    }

    fn update_account(&self, account: Account) {
        let _ = self.commands.send(Command::UpdateAccount(account));
    }

    fn block(&self, account: &str, address: &str, on: bool) {
        let _ = self.commands.send(Command::Block {
            account: account.to_string(),
            address: address.trim().to_lowercase(),
            on,
        });
    }

    fn original(&self, account: &str, message: &Message) -> Reply<Original> {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::Original {
            account: account.to_string(),
            at: Located::of(message),
            reply: sender,
        });
        receiver
    }

    fn save_draft(&self, draft: Draft) -> Reply {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::SaveDraft {
            draft,
            reply: sender,
        });
        receiver
    }

    fn arrivals(&self) -> Vec<Arrival> {
        std::mem::take(&mut *self.shared.arrivals.lock().expect("arrivals lock"))
    }

    fn mark_read(&self, conversation: &Conversation) {
        self.change(
            std::slice::from_ref(conversation),
            &Scope::AllInboxes,
            Change::Read(true),
        );
    }

    fn change(&self, conversations: &[Conversation], scope: &Scope, change: Change) {
        let mut by_account: BTreeMap<&str, Vec<&Conversation>> = BTreeMap::new();
        for conversation in conversations {
            by_account
                .entry(&conversation.account)
                .or_default()
                .push(conversation);
        }
        for (account, conversations) in by_account {
            let keys: Vec<String> = conversations.iter().map(|c| c.key.clone()).collect();
            let ids = conversations
                .iter()
                .flat_map(|c| &c.copies)
                .filter_map(|copy| copy.message_id.clone())
                .collect();
            let targets: Vec<Target> = conversations
                .iter()
                .flat_map(|c| actions::targets(c, scope, &change))
                .collect();
            if targets.is_empty() && !matches!(change, Change::Pin(_)) {
                continue;
            }
            if change.leaves() {
                self.shared.hide(account, &keys);
            }
            let _ = self.commands.send(Command::Change {
                account: account.to_string(),
                keys,
                ids,
                change: change.clone(),
                targets,
            });
        }
        (self.shared.notify)();
    }

    fn empty(&self, account: &str, mailbox: &str) {
        let conversations = self.conversations();
        let held: Vec<&Conversation> = conversations
            .iter()
            .filter(|c| c.account == account && c.copies.iter().any(|copy| copy.mailbox == mailbox))
            .collect();
        let Some(highest) = held
            .iter()
            .flat_map(|c| &c.copies)
            .filter(|copy| copy.mailbox == mailbox)
            .map(|copy| copy.uid)
            .max()
        else {
            return;
        };
        let keys: Vec<String> = held.iter().map(|c| c.key.clone()).collect();
        self.shared.hide(account, &keys);
        let _ = self.commands.send(Command::Empty {
            account: account.to_string(),
            mailbox: mailbox.to_string(),
            keys,
            highest,
        });
        (self.shared.notify)();
    }

    fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }

    fn transcribed(&self, account: &str, message: &Message, text: String) {
        if let Some(message_id) = message.message_id.clone() {
            let _ = self.commands.send(Command::Transcribed {
                account: account.to_string(),
                message_id,
                text,
            });
        }
    }

    fn email(&self, account: &str, message: &Message) -> Reply<String> {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::Email {
            account: account.to_string(),
            at: Located::of(message),
            reply: sender,
        });
        receiver
    }

    fn attachment(&self, account: &str, message: &Message, index: usize) -> Reply<Vec<u8>> {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::Attachment {
            account: account.to_string(),
            at: Located::of(message),
            index,
            name: message.attachments.get(index).cloned().unwrap_or_default(),
            reply: sender,
        });
        receiver
    }
}

struct Worker {
    shared: Arc<Shared>,
    accounts_path: PathBuf,
    cache_dir: PathBuf,
    accounts: Vec<Account>,
    data: HashMap<String, AccountData>,
    open: Scope,
    own: mpsc::UnboundedSender<Command>,
    watchers: HashMap<String, tokio::task::JoinHandle<()>>,
    locked: BTreeSet<String>,
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .expect("a blocking task")
}

impl Worker {
    async fn run(mut self, mut commands: mpsc::UnboundedReceiver<Command>) {
        if let Err(error) = blocking(secrets::open).await {
            self.set_status(Status::Failed(error));
        }
        let path = self.accounts_path.clone();
        self.accounts = blocking(move || cache::load_accounts(&path)).await;
        if self.colour_accounts() {
            self.save_accounts();
        }
        for account in self.accounts.clone() {
            let path = self.data_path(&account.key());
            let data = blocking(move || cache::load_data(&path)).await;
            self.data.insert(account.key(), data);
        }
        self.publish();
        for account in self.accounts.clone() {
            self.watch(account);
        }
        let mut ticker = tokio::time::interval(SYNC_EVERY);
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    None => return,
                    Some(command) => self.handle(command).await,
                },
                _ = ticker.tick() => self.sync().await,
            }
        }
    }

    fn watch(&mut self, account: Account) {
        let own = self.own.clone();
        let key = account.key();
        let watcher = tokio::spawn(async move {
            let host = account.imap_host.trim().to_string();
            loop {
                let key = account.key();
                let opened = match blocking(move || secrets::password(&key)).await {
                    Ok(password) => Connection::open(&account, &password).await,
                    Err(error) => Err(error),
                };
                let Ok(mut connection) = opened else {
                    tokio::time::sleep(WATCH_RETRY).await;
                    continue;
                };
                let mut known = None;
                while let Ok((next, exists, news)) =
                    connection.watch(&host, INBOX, WATCH_PATIENCE).await
                {
                    connection = next;
                    let changed = news || known.is_some_and(|known| known != exists);
                    known = Some(exists);
                    if changed {
                        tokio::time::sleep(Duration::from_millis(500)).await;
                        if own.send(Command::Arrived(account.key())).is_err() {
                            return;
                        }
                    }
                }
                tokio::time::sleep(WATCH_RETRY).await;
            }
        });
        if let Some(previous) = self.watchers.insert(key, watcher) {
            previous.abort();
        }
    }

    fn colour_accounts(&mut self) -> bool {
        let mut changed = false;
        for index in 0..self.accounts.len() {
            if self.accounts[index].colour.is_some() {
                continue;
            }
            let taken: BTreeSet<u8> = self.accounts.iter().filter_map(|a| a.colour).collect();
            let free = (0..model::COLOURS)
                .find(|colour| !taken.contains(colour))
                .unwrap_or(index as u8 % model::COLOURS);
            self.accounts[index].colour = Some(free);
            changed = true;
        }
        changed
    }

    async fn sign_in_again(&mut self, account: &str, password: String) -> Result<(), String> {
        let known = self
            .account(account)
            .ok_or("that account is no longer signed in")?;
        Connection::open(&known, &password).await?.close().await;
        smtp::check(&known, &password).await?;
        let key = known.key();
        blocking(move || secrets::save(&key, &password)).await?;
        self.locked.remove(account);
        self.publish();
        Ok(())
    }

    fn data_path(&self, account: &str) -> PathBuf {
        self.cache_dir.join(cache::file_name(account))
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Refresh => self.sync().await,
            Command::Transcribed {
                account,
                message_id,
                text,
            } => {
                if let Some(data) = self.data.get_mut(&account) {
                    data.transcribed(&message_id, &text);
                }
                self.save_data(&account);
                self.publish();
            }
            Command::UpdateAccount(changed) => {
                if let Some(known) = self
                    .accounts
                    .iter_mut()
                    .find(|known| known.key() == changed.key())
                {
                    known.label = changed.label.trim().to_string();
                    known.colour = changed.colour;
                    known.aliases = changed.aliases.clone();
                }
                self.save_accounts();
                self.publish();
            }
            Command::Block {
                account,
                address,
                on,
            } => {
                let mine = self
                    .account(&account)
                    .is_some_and(|known| known.key() == address);
                if let Some(data) = self.data.get_mut(&account)
                    && !mine
                {
                    if on {
                        data.blocked.insert(address);
                    } else {
                        data.blocked.remove(&address);
                    }
                }
                self.save_data(&account);
                self.publish();
                if on && let Some(known) = self.account(&account) {
                    self.sync_accounts(vec![known]).await;
                }
            }
            Command::SignInAgain {
                account,
                password,
                reply,
            } => {
                let result = self.sign_in_again(&account, password).await;
                let succeeded = result.is_ok();
                let _ = reply.send(result);
                if succeeded && let Some(known) = self.account(&account) {
                    self.sync_accounts(vec![known]).await;
                }
            }
            Command::Original { account, at, reply } => {
                let original = match self.raw(&account, &at).await {
                    Ok(raw) => blocking(move || content::original(&raw))
                        .await
                        .ok_or_else(|| "the message could not be read".to_string()),
                    Err(error) => Err(error),
                };
                let _ = reply.send(original);
            }
            Command::SaveDraft { draft, reply } => {
                let saved = self.save_draft(draft).await;
                let _ = reply.send(saved);
            }
            Command::SearchServer {
                places,
                terms,
                reply,
            } => {
                let found = self.search_server(places, terms).await;
                let _ = reply.send(found);
            }
            Command::Arrived(account) => {
                if let Some(known) = self.account(&account) {
                    self.sync_accounts(vec![known]).await;
                }
            }
            Command::Open(scope) => {
                self.open = scope;
                if let Scope::Mailbox { account, .. } = &self.open
                    && let Some(known) = self.account(account)
                {
                    self.sync_accounts(vec![known]).await;
                }
            }
            Command::SignIn {
                account,
                password,
                reply,
            } => {
                let added = account.clone();
                let result = self.sign_in(account, password).await;
                let succeeded = result.is_ok();
                let _ = reply.send(result);
                if succeeded {
                    self.watch(added);
                    self.sync().await;
                }
            }
            Command::SignOut { account } => {
                if let Some(watcher) = self.watchers.remove(&account) {
                    watcher.abort();
                }
                let key = account.clone();
                let _ = blocking(move || secrets::forget(&key)).await;
                self.accounts.retain(|known| known.key() != account);
                self.data.remove(&account);
                let _ = std::fs::remove_file(self.data_path(&account));
                self.save_accounts();
                self.publish();
                if self.accounts.is_empty() {
                    self.set_status(Status::NoAccounts);
                }
            }
            Command::Send { draft, reply } => self.send(draft, reply).await,
            Command::Change {
                account,
                keys,
                ids,
                change,
                targets,
            } => self.change(&account, keys, ids, change, targets).await,
            Command::Empty {
                account,
                mailbox,
                keys,
                highest,
            } => self.empty(&account, &mailbox, &keys, highest).await,
            Command::Email { account, at, reply } => {
                let document = match self.raw(&account, &at).await {
                    Ok(raw) => blocking(move || email::document(&raw))
                        .await
                        .ok_or_else(|| "the message could not be read".to_string()),
                    Err(error) => Err(error),
                };
                let _ = reply.send(document);
            }
            Command::Attachment {
                account,
                at,
                index,
                name,
                reply,
            } => {
                let file = match self.raw(&account, &at).await {
                    Ok(raw) => blocking(move || content::attachment(&raw, index, &name))
                        .await
                        .ok_or_else(|| "the attachment is no longer in the message".to_string()),
                    Err(error) => Err(error),
                };
                let _ = reply.send(file);
            }
        }
    }

    fn set_status(&self, status: Status) {
        *self.shared.status.write().expect("status lock") = status;
        (self.shared.notify)();
    }

    fn publish(&self) {
        let mut conversations: Vec<Conversation> = self
            .accounts
            .iter()
            .filter_map(|account| {
                let data = self.data.get(&account.key())?;
                let mut found = thread::conversations(account, &data.started, data.messages());
                for conversation in &mut found {
                    conversation.pinned = data.pinned.contains(&conversation.key);
                }
                Some(found)
            })
            .flatten()
            .collect();
        conversations.sort_by_key(|conversation| {
            (
                !conversation.pinned,
                std::cmp::Reverse(conversation.latest()),
            )
        });
        let folders = self
            .data
            .iter()
            .map(|(account, data)| (account.clone(), data.folders.clone()))
            .collect();
        let blocked = self
            .data
            .iter()
            .map(|(account, data)| (account.clone(), data.blocked.iter().cloned().collect()))
            .collect();
        let mut published = self.shared.published.write().expect("published lock");
        *published = Snapshot {
            accounts: self.accounts.clone(),
            conversations: self.shared.present(conversations),
            folders,
            locked: self.locked.clone(),
            blocked,
        };
        drop(published);
        (self.shared.notify)();
    }

    fn save_accounts(&self) {
        let _ = cache::save_accounts(&self.accounts_path, &self.accounts);
    }

    fn save_data(&self, account: &str) {
        if let Some(data) = self.data.get(account) {
            let _ = cache::save_data(&self.data_path(account), data);
        }
    }

    fn account(&self, key: &str) -> Option<Account> {
        self.accounts
            .iter()
            .find(|account| account.key() == key)
            .cloned()
    }

    async fn password(account: &str) -> Result<String, String> {
        let key = account.to_string();
        blocking(move || secrets::password(&key)).await
    }

    async fn sign_in(&mut self, account: Account, password: String) -> Result<(), String> {
        if self.account(&account.key()).is_some() {
            return Err(format!("{} is already signed in", account.address.trim()));
        }
        Connection::open(&account, &password).await?.close().await;
        smtp::check(&account, &password).await?;
        let key = account.key();
        blocking(move || secrets::save(&key, &password)).await?;
        self.data.insert(account.key(), AccountData::default());
        self.accounts.push(account);
        self.colour_accounts();
        self.save_accounts();
        self.publish();
        Ok(())
    }

    async fn sync(&mut self) {
        if self.accounts.is_empty() {
            self.set_status(Status::NoAccounts);
            return;
        }
        self.sync_accounts(self.accounts.clone()).await;
    }

    async fn sync_accounts(&mut self, accounts: Vec<Account>) {
        self.set_status(Status::Syncing);
        let mut failures = Vec::new();
        for account in accounts {
            match self.sync_account(&account).await {
                Ok(()) => {
                    self.locked.remove(&account.key());
                }
                Err(error) => {
                    if error.contains("refused the address or password") {
                        self.locked.insert(account.key());
                    }
                    failures.push(format!("{}: {error}", account.address));
                }
            }
            self.save_data(&account.key());
            self.publish();
        }
        self.set_status(if failures.is_empty() {
            Status::Synced(Utc::now())
        } else {
            Status::Failed(failures.join("\n"))
        });
    }

    async fn sync_account(&mut self, account: &Account) -> Result<(), String> {
        let password = Self::password(&account.key()).await?;
        let host = account.imap_host.trim().to_string();
        let mut connection = Connection::open(account, &password).await?;
        let mut data = self.data.remove(&account.key()).unwrap_or_default();
        let open = match &self.open {
            Scope::Mailbox { account: key, name } if *key == account.key() => Some(name.clone()),
            _ => None,
        };
        let result = async {
            if data.inbox.name.is_empty() {
                data.inbox.name = INBOX.into();
            }
            let folders = connection.folders(&host).await?;
            if data.sent.is_none()
                && let Some(sent) = folders
                    .iter()
                    .find(|folder| folder.role == Some(Role::Sent))
            {
                data.sent = Some(Mailbox {
                    name: sent.name.clone(),
                    ..Mailbox::default()
                });
            }
            data.others
                .retain(|name, _| folders.iter().any(|folder| &folder.name == name));
            data.folders = folders;
            let arrived = connection.sync(&host, &mut data.inbox).await?;
            if !data.blocked.is_empty()
                && let Some(junk) = actions::role_folder(&data.folders, Role::Junk)
            {
                let unwanted: Vec<u32> =
                    data.inbox
                        .messages
                        .iter()
                        .filter(|(_, message)| {
                            message.from.as_ref().is_some_and(|from| {
                                data.blocked.contains(&from.address.to_lowercase())
                            })
                        })
                        .map(|(uid, _)| *uid)
                        .collect();
                if !unwanted.is_empty() {
                    connection.move_to(&host, INBOX, &unwanted, &junk).await?;
                    for uid in &unwanted {
                        data.inbox.messages.remove(uid);
                    }
                }
            }
            let mut fresh: Vec<Arrival> = arrived
                .iter()
                .filter_map(|uid| data.inbox.messages.get(uid))
                .filter(|message| !message.seen && message.reaction.is_none())
                .filter(|message| {
                    message
                        .from
                        .as_ref()
                        .is_none_or(|from| !account.owns(&from.address))
                })
                .map(|message| Arrival {
                    account: account.key(),
                    message_id: message.message_id.clone(),
                    sender: message
                        .from
                        .as_ref()
                        .map_or("Someone".into(), |from| from.display().to_string()),
                    subject: message.subject.clone(),
                    text: message.text.chars().take(200).collect(),
                })
                .collect();
            if !fresh.is_empty() {
                self.shared
                    .arrivals
                    .lock()
                    .expect("arrivals lock")
                    .append(&mut fresh);
            }
            if let Some(sent) = data.sent.as_mut() {
                connection.sync(&host, sent).await?;
            }
            if let Some(name) = open {
                if data.mailbox_mut(&name).is_none()
                    && data
                        .folders
                        .iter()
                        .any(|folder| folder.name == name && folder.selectable)
                {
                    data.others.insert(
                        name.clone(),
                        Mailbox {
                            name: name.clone(),
                            ..Mailbox::default()
                        },
                    );
                }
                if let Some(mailbox) = data.others.get_mut(&name) {
                    connection.sync(&host, mailbox).await?;
                }
            }
            Ok(())
        }
        .await;
        let filed: HashSet<String> = data
            .messages()
            .into_iter()
            .filter(|message| message.uid != 0)
            .filter_map(|message| message.message_id)
            .collect();
        data.unfiled.retain(|message| {
            message
                .message_id
                .as_ref()
                .is_none_or(|id| !filed.contains(id))
        });
        data.unsent.retain(|copy| !filed.contains(&copy.message_id));
        let mut unsaved = None;
        if result.is_ok() && data.unsent.iter().any(|copy| !copy.saved) {
            match data.sent_name().map(str::to_string) {
                None => unsaved = Some("this account has no Sent folder".to_string()),
                Some(sent) => {
                    for copy in data.unsent.iter_mut().filter(|copy| !copy.saved) {
                        let raw = STANDARD.decode(&copy.raw).unwrap_or_default();
                        match connection.append(&host, &sent, &raw).await {
                            Ok(()) => copy.saved = true,
                            Err(error) => unsaved = Some(error),
                        }
                    }
                }
            }
        }
        self.data.insert(account.key(), data);
        connection.close().await;
        result.and(match unsaved {
            Some(error) => Err(format!(
                "a sent message's copy for the Sent folder is still not saved ({error}); Zirbe will try again"
            )),
            None => Ok(()),
        })
    }

    async fn send(&mut self, draft: Draft, reply: reply::Sender<Result<(), String>>) {
        let Some(account) = self.account(&draft.account) else {
            let _ = reply.send(Err("that account is no longer signed in".into()));
            return;
        };
        let (message, id) = match compose::build(&draft, &account) {
            Ok(built) => built,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        let password = match Self::password(&account.key()).await {
            Ok(password) => password,
            Err(error) => {
                let _ = reply.send(Err(error));
                return;
            }
        };
        let raw = message.formatted();
        if let Err(error) = smtp::send(&account, &password, message).await {
            let _ = reply.send(Err(error));
            return;
        }
        let sent_name = self
            .data
            .get(&account.key())
            .and_then(|data| data.sent_name())
            .map(str::to_string);
        let local = mime::parse(&Fetched {
            mailbox: sent_name.as_deref().unwrap_or_default(),
            uid: 0,
            seen: true,
            header: &raw,
            body: Body::Whole,
            arrived: Some(Utc::now()),
        });
        if let Some(data) = self.data.get_mut(&account.key()) {
            if draft.in_reply_to.is_none() {
                data.started.insert(id.clone());
            }
            data.unfiled.extend(local);
            data.unsent.push(UnsentCopy {
                message_id: id.clone(),
                raw: STANDARD.encode(&raw),
                saved: false,
            });
        }
        self.save_data(&account.key());
        self.publish();
        let _ = reply.send(Ok(()));
        let saved = match sent_name {
            None => Err("this account has no Sent folder".to_string()),
            Some(name) => match Connection::open(&account, &password).await {
                Ok(mut connection) => {
                    let appended = connection
                        .append(account.imap_host.trim(), &name, &raw)
                        .await;
                    connection.close().await;
                    appended
                }
                Err(error) => Err(error),
            },
        };
        match saved {
            Ok(()) => {
                if let Some(copy) = self
                    .data
                    .get_mut(&account.key())
                    .and_then(|data| data.unsent.iter_mut().find(|copy| copy.message_id == id))
                {
                    copy.saved = true;
                }
                self.save_data(&account.key());
            }
            Err(error) => self.set_status(Status::Failed(format!(
                "{}: sent, but the copy for the Sent folder was not saved ({error}); Zirbe will try again",
                account.address
            ))),
        }
        if let Some((mailbox, uid)) = draft.replaces
            && let Ok(mut connection) = Connection::open(&account, &password).await
        {
            let _ = connection
                .delete(account.imap_host.trim(), &mailbox, &[uid])
                .await;
            connection.close().await;
            if let Some(data) = self.data.get_mut(&account.key())
                && let Some(cached) = data.mailbox_mut(&mailbox)
            {
                cached.messages.remove(&uid);
            }
            self.save_data(&account.key());
            self.publish();
        }
    }

    async fn change(
        &mut self,
        account: &str,
        keys: Vec<String>,
        ids: Vec<String>,
        change: Change,
        targets: Vec<Target>,
    ) {
        if change.leaves() {
            self.shared.unhide(account, &keys);
        }
        let Some(known) = self.account(account) else {
            return;
        };
        let Some(data) = self.data.get_mut(account) else {
            return;
        };
        let destination = match &change {
            Change::Archive => Some(
                actions::role_folder(&data.folders, Role::Archive)
                    .ok_or_else(|| actions::new_folder_name(&data.folders, "Archive")),
            ),
            Change::Trash | Change::Junk => {
                let role = if change == Change::Trash {
                    Role::Trash
                } else {
                    Role::Junk
                };
                match actions::role_folder(&data.folders, role) {
                    Some(name) => Some(Ok(name)),
                    None => {
                        let what = if role == Role::Trash { "Trash" } else { "Junk" };
                        self.set_status(Status::Failed(format!(
                            "{}: this account has no {what} folder",
                            known.address
                        )));
                        self.publish();
                        return;
                    }
                }
            }
            Change::Move(name) => Some(Ok(name.clone())),
            _ => None,
        };
        let mut by_mailbox: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        for (name, uid) in &targets {
            let Some(mailbox) = data.mailbox_mut(name) else {
                continue;
            };
            let Some(message) = mailbox.messages.get_mut(uid) else {
                continue;
            };
            let was_unread = !message.seen;
            match &change {
                Change::Flag(on) => message.flagged = *on,
                Change::Read(on) => message.seen = *on,
                _ => {
                    mailbox.messages.remove(uid);
                }
            }
            by_mailbox.entry(name.clone()).or_default().push(*uid);
            let unread_change: i64 = match &change {
                Change::Read(true) if was_unread => -1,
                Change::Read(false) if !was_unread => 1,
                Change::Flag(_) | Change::Read(_) | Change::Pin(_) => 0,
                _ if was_unread => -1,
                _ => 0,
            };
            if let Some(folder) = data.folders.iter_mut().find(|folder| &folder.name == name) {
                folder.unread = (folder.unread as i64 + unread_change).max(0) as u32;
            }
        }
        if change.leaves() {
            for id in &ids {
                data.started.remove(id);
            }
        }
        if let Change::Pin(on) = &change {
            for key in keys {
                if *on {
                    data.pinned.insert(key);
                } else {
                    data.pinned.remove(&key);
                }
            }
        }
        self.save_data(account);
        self.publish();
        if by_mailbox.is_empty() {
            return;
        }
        let host = known.imap_host.trim().to_string();
        let result: Result<bool, String> = async {
            let password = Self::password(account).await?;
            let mut connection = Connection::open(&known, &password).await?;
            let outcome = async {
                let mut created = false;
                let destination = match destination {
                    Some(Ok(name)) => Some(name),
                    Some(Err(name)) => {
                        connection.create(&host, &name).await?;
                        created = true;
                        Some(name)
                    }
                    None => None,
                };
                for (mailbox, uids) in &by_mailbox {
                    match (&change, &destination) {
                        (Change::Flag(on), _) => {
                            connection
                                .set_flag(&host, mailbox, uids, "\\Flagged", *on)
                                .await?
                        }
                        (Change::Read(on), _) => {
                            connection
                                .set_flag(&host, mailbox, uids, "\\Seen", *on)
                                .await?
                        }
                        (Change::Delete, _) => connection.delete(&host, mailbox, uids).await?,
                        (_, Some(destination)) => {
                            connection
                                .move_to(&host, mailbox, uids, destination)
                                .await?
                        }
                        _ => {}
                    }
                }
                Ok::<bool, String>(created)
            }
            .await;
            connection.close().await;
            outcome
        }
        .await;
        match result {
            Ok(true) => self.sync_accounts(vec![known]).await,
            Ok(false) => {}
            Err(error) => {
                self.set_status(Status::Failed(format!("{}: {error}", known.address)));
                self.sync_accounts(vec![known]).await;
            }
        }
    }

    async fn empty(&mut self, account: &str, name: &str, keys: &[String], highest: u32) {
        self.shared.unhide(account, keys);
        let (Some(known), Some(data)) = (self.account(account), self.data.get_mut(account)) else {
            self.publish();
            return;
        };
        let unread = data.mailbox_mut(name).map(|mailbox| {
            mailbox.messages.retain(|uid, _| *uid > highest);
            mailbox.found.retain(|uid| *uid > highest);
            mailbox
                .messages
                .values()
                .filter(|message| !message.seen)
                .count() as u32
        });
        if let Some(folder) = data.folders.iter_mut().find(|folder| folder.name == name) {
            folder.unread = unread.unwrap_or(0);
        }
        self.save_data(account);
        self.publish();
        let host = known.imap_host.trim().to_string();
        let result = async {
            let password = Self::password(account).await?;
            let mut connection = Connection::open(&known, &password).await?;
            let emptied = connection.empty(&host, name, highest).await;
            connection.close().await;
            emptied
        }
        .await;
        if let Err(error) = result {
            self.set_status(Status::Failed(format!("{}: {error}", known.address)));
            self.sync_accounts(vec![known]).await;
        }
    }

    async fn save_draft(&mut self, draft: Draft) -> Result<(), String> {
        let replaces = draft.replaces.clone();
        let known = self
            .account(&draft.account)
            .ok_or("that account is no longer signed in")?;
        let drafts = self
            .data
            .get(&known.key())
            .and_then(|data| actions::role_folder(&data.folders, Role::Drafts))
            .ok_or("this account has no Drafts folder")?;
        let raw = compose::unsent(&draft, &known)?;
        let password = Self::password(&known.key()).await?;
        let host = known.imap_host.trim().to_string();
        let mut connection = Connection::open(&known, &password).await?;
        let saved = async {
            connection
                .append_with(&host, &drafts, &raw, "(\\Draft \\Seen)")
                .await?;
            if let Some((mailbox, uid)) = &replaces {
                connection.delete(&host, mailbox, &[*uid]).await?;
            }
            Ok::<(), String>(())
        }
        .await;
        connection.close().await;
        saved?;
        if let Some((mailbox, uid)) = replaces
            && let Some(data) = self.data.get_mut(&known.key())
            && let Some(cached) = data.mailbox_mut(&mailbox)
        {
            cached.messages.remove(&uid);
        }
        self.sync_accounts(vec![known]).await;
        Ok(())
    }

    async fn search_server(
        &mut self,
        places: Vec<(String, String)>,
        terms: Vec<String>,
    ) -> Result<usize, String> {
        let mut total = 0;
        for (account, name) in places {
            let Some(known) = self.account(&account) else {
                continue;
            };
            let password = Self::password(&account).await?;
            let host = known.imap_host.trim().to_string();
            let mut connection = Connection::open(&known, &password).await?;
            let Some(data) = self.data.get_mut(&account) else {
                connection.close().await;
                continue;
            };
            if data.mailbox_mut(&name).is_none() {
                data.others.insert(
                    name.clone(),
                    Mailbox {
                        name: name.clone(),
                        ..Mailbox::default()
                    },
                );
            }
            let found = match data.mailbox_mut(&name) {
                Some(mailbox) => connection.search_into(&host, mailbox, &terms).await,
                None => Ok(0),
            };
            connection.close().await;
            total += found?;
            self.save_data(&account);
        }
        self.publish();
        Ok(total)
    }

    async fn raw(&self, account: &str, at: &Located) -> Result<Vec<u8>, String> {
        let known = self
            .account(account)
            .ok_or("that account is no longer signed in")?;
        if at.uid == 0 {
            return self
                .data
                .get(&known.key())
                .and_then(|data| {
                    data.unsent
                        .iter()
                        .find(|copy| Some(&copy.message_id) == at.message_id.as_ref())
                })
                .and_then(|copy| STANDARD.decode(&copy.raw).ok())
                .ok_or_else(|| "this message is not on the server yet".into());
        }
        let password = Self::password(account).await?;
        let host = known.imap_host.trim().to_string();
        let mut connection = Connection::open(&known, &password).await?;
        let raw = connection.whole(&host, &at.mailbox, at.uid).await;
        connection.close().await;
        raw
    }
}
