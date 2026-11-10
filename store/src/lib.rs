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
mod mime;
mod model;
mod presets;
mod secrets;
mod smtp;
mod subject;
mod thread;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock, mpsc as reply};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc;

pub use compose::{parse_people, reply};
pub use model::{
    Account, Change, Conversation, Copy, Draft, FolderInfo, INBOX, Message, Person, Role, Scope,
    Status,
};
pub use presets::{Preset, for_address};

use cache::{AccountData, Mailbox};
use imap::Connection;
use mime::{Body, Fetched};

const SYNC_EVERY: Duration = Duration::from_secs(120);

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
    fn refresh(&self);
    fn email(&self, account: &str, message: &Message) -> Reply<String>;
    fn attachment(&self, account: &str, message: &Message, index: usize) -> Reply<Vec<u8>>;
}

pub struct Store {
    shared: Arc<Shared>,
    commands: mpsc::UnboundedSender<Command>,
}

struct Shared {
    published: RwLock<Snapshot>,
    status: RwLock<Status>,
    notify: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct Snapshot {
    accounts: Vec<Account>,
    conversations: Arc<Vec<Conversation>>,
    folders: HashMap<String, Vec<FolderInfo>>,
}

enum Command {
    Refresh,
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
    Email {
        account: String,
        mailbox: String,
        uid: u32,
        reply: reply::Sender<Result<String, String>>,
    },
    Attachment {
        account: String,
        mailbox: String,
        uid: u32,
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
            let keys = conversations.iter().map(|c| c.key.clone()).collect();
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
            let _ = self.commands.send(Command::Change {
                account: account.to_string(),
                keys,
                ids,
                change: change.clone(),
                targets,
            });
        }
    }

    fn refresh(&self) {
        let _ = self.commands.send(Command::Refresh);
    }

    fn email(&self, account: &str, message: &Message) -> Reply<String> {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::Email {
            account: account.to_string(),
            mailbox: message.mailbox.clone(),
            uid: message.uid,
            reply: sender,
        });
        receiver
    }

    fn attachment(&self, account: &str, message: &Message, index: usize) -> Reply<Vec<u8>> {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::Attachment {
            account: account.to_string(),
            mailbox: message.mailbox.clone(),
            uid: message.uid,
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
        for account in self.accounts.clone() {
            let path = self.data_path(&account.key());
            let data = blocking(move || cache::load_data(&path)).await;
            self.data.insert(account.key(), data);
        }
        self.publish();
        self.sync().await;
        loop {
            tokio::select! {
                command = commands.recv() => match command {
                    None => return,
                    Some(command) => self.handle(command).await,
                },
                () = tokio::time::sleep(SYNC_EVERY) => self.sync().await,
            }
        }
    }

    fn data_path(&self, account: &str) -> PathBuf {
        self.cache_dir.join(cache::file_name(account))
    }

    async fn handle(&mut self, command: Command) {
        match command {
            Command::Refresh => self.sync().await,
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
                let result = self.sign_in(account, password).await;
                let succeeded = result.is_ok();
                let _ = reply.send(result);
                if succeeded {
                    self.sync().await;
                }
            }
            Command::SignOut { account } => {
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
            Command::Email {
                account,
                mailbox,
                uid,
                reply,
            } => {
                let document = match self.raw(&account, &mailbox, uid).await {
                    Ok(raw) => blocking(move || email::document(&raw))
                        .await
                        .ok_or_else(|| "the message could not be read".to_string()),
                    Err(error) => Err(error),
                };
                let _ = reply.send(document);
            }
            Command::Attachment {
                account,
                mailbox,
                uid,
                index,
                name,
                reply,
            } => {
                let file = match self.raw(&account, &mailbox, uid).await {
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
                let mut found = thread::conversations(
                    &account.key(),
                    &account.address,
                    &data.started,
                    data.messages(),
                );
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
        *self.shared.published.write().expect("published lock") = Snapshot {
            accounts: self.accounts.clone(),
            conversations: Arc::new(conversations),
            folders,
        };
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
            if let Err(error) = self.sync_account(&account).await {
                failures.push(format!("{}: {error}", account.address));
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
            connection.sync(&host, &mut data.inbox).await?;
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
        self.data.insert(account.key(), data);
        connection.close().await;
        result
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
                data.started.insert(id);
            }
            data.unfiled.extend(local);
        }
        self.save_data(&account.key());
        self.publish();
        let _ = reply.send(Ok(()));
        if let Some(name) = sent_name
            && let Ok(mut connection) = Connection::open(&account, &password).await
        {
            let _ = connection
                .append(account.imap_host.trim(), &name, &raw)
                .await;
            connection.close().await;
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
        if matches!(
            change,
            Change::Archive | Change::Trash | Change::Junk | Change::Move(_) | Change::Delete
        ) {
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

    async fn raw(&self, account: &str, mailbox: &str, uid: u32) -> Result<Vec<u8>, String> {
        let known = self
            .account(account)
            .ok_or("that account is no longer signed in")?;
        if mailbox.is_empty() || uid == 0 {
            return Err("this message is not on the server yet".into());
        }
        let password = Self::password(account).await?;
        let host = known.imap_host.trim().to_string();
        let mut connection = Connection::open(&known, &password).await?;
        let raw = connection.whole(&host, mailbox, uid).await;
        connection.close().await;
        raw
    }
}
