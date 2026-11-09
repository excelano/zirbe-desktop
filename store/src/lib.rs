// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

#![forbid(unsafe_code)]

mod cache;
mod compose;
mod content;
mod email;
mod imap;
mod mime;
mod model;
mod presets;
mod secrets;
mod smtp;
mod subject;
mod thread;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock, mpsc as reply};
use std::time::Duration;

use chrono::Utc;
use tokio::sync::mpsc;

pub use compose::{parse_people, reply};
pub use model::{Account, Conversation, Draft, Folder, Message, Person, Status};
pub use presets::{Preset, for_address};

use cache::{AccountData, Mailbox};
use imap::Connection;
use mime::{Body, Fetched};

const SYNC_EVERY: Duration = Duration::from_secs(120);

pub type Reply<T = ()> = reply::Receiver<Result<T, String>>;

pub trait MailSource {
    fn status(&self) -> Status;
    fn accounts(&self) -> Vec<Account>;
    fn conversations(&self) -> Arc<Vec<Conversation>>;
    fn send(&self, draft: Draft) -> Reply;
    fn mark_read(&self, conversation: &Conversation);
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
}

enum Command {
    Refresh,
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
    MarkRead {
        account: String,
        uids: Vec<u32>,
    },
    Email {
        account: String,
        folder: Folder,
        uid: u32,
        reply: reply::Sender<Result<String, String>>,
    },
    Attachment {
        account: String,
        folder: Folder,
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

    fn mark_read(&self, conversation: &Conversation) {
        let uids: Vec<u32> = conversation
            .messages
            .iter()
            .filter(|message| message.folder == Folder::Inbox && !message.seen)
            .map(|message| message.uid)
            .collect();
        if !uids.is_empty() {
            let _ = self.commands.send(Command::MarkRead {
                account: conversation.account.clone(),
                uids,
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
            folder: message.folder,
            uid: message.uid,
            reply: sender,
        });
        receiver
    }

    fn attachment(&self, account: &str, message: &Message, index: usize) -> Reply<Vec<u8>> {
        let (sender, receiver) = reply::channel();
        let _ = self.commands.send(Command::Attachment {
            account: account.to_string(),
            folder: message.folder,
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
            Command::MarkRead { account, uids } => self.mark_read(&account, &uids).await,
            Command::Email {
                account,
                folder,
                uid,
                reply,
            } => {
                let document = match self.raw(&account, folder, uid).await {
                    Ok(raw) => blocking(move || email::document(&raw))
                        .await
                        .ok_or_else(|| "the message could not be read".to_string()),
                    Err(error) => Err(error),
                };
                let _ = reply.send(document);
            }
            Command::Attachment {
                account,
                folder,
                uid,
                index,
                name,
                reply,
            } => {
                let file = match self.raw(&account, folder, uid).await {
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
                Some(thread::conversations(
                    &account.key(),
                    &account.address,
                    &data.started,
                    data.messages(),
                ))
            })
            .flatten()
            .collect();
        conversations.sort_by_key(|conversation| std::cmp::Reverse(conversation.latest()));
        *self.shared.published.write().expect("published lock") = Snapshot {
            accounts: self.accounts.clone(),
            conversations: Arc::new(conversations),
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
        self.set_status(Status::Syncing);
        let mut failures = Vec::new();
        for account in self.accounts.clone() {
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
        let result = async {
            if data.inbox.name.is_empty() {
                data.inbox.name = "INBOX".into();
            }
            connection
                .sync(&host, Folder::Inbox, &mut data.inbox)
                .await?;
            if data.sent.is_none()
                && let Some(name) = connection.sent_name(&host).await?
            {
                data.sent = Some(Mailbox {
                    name,
                    ..Mailbox::default()
                });
            }
            if let Some(sent) = data.sent.as_mut() {
                connection.sync(&host, Folder::Sent, sent).await?;
            }
            Ok(())
        }
        .await;
        let filed: HashSet<String> = data
            .inbox
            .messages
            .values()
            .chain(data.sent.iter().flat_map(|sent| sent.messages.values()))
            .filter_map(|message| message.message_id.clone())
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
        let local = mime::parse(&Fetched {
            folder: Folder::Sent,
            uid: 0,
            seen: true,
            header: &raw,
            body: Body::Whole,
            arrived: Some(Utc::now()),
        });
        let mut sent_name = None;
        if let Some(data) = self.data.get_mut(&account.key()) {
            if draft.in_reply_to.is_none() {
                data.started.insert(id);
            }
            data.unfiled.extend(local);
            sent_name = data.sent.as_ref().map(|sent| sent.name.clone());
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

    async fn mark_read(&mut self, account: &str, uids: &[u32]) {
        let Some(known) = self.account(account) else {
            return;
        };
        let Some(data) = self.data.get_mut(account) else {
            return;
        };
        for uid in uids {
            if let Some(message) = data.inbox.messages.get_mut(uid) {
                message.seen = true;
            }
        }
        let inbox = data.inbox.name.clone();
        self.save_data(account);
        self.publish();
        if let Ok(password) = Self::password(account).await
            && let Ok(mut connection) = Connection::open(&known, &password).await
        {
            let _ = connection
                .mark_seen(known.imap_host.trim(), &inbox, uids)
                .await;
            connection.close().await;
        }
    }

    async fn raw(&self, account: &str, folder: Folder, uid: u32) -> Result<Vec<u8>, String> {
        let known = self
            .account(account)
            .ok_or("that account is no longer signed in")?;
        let data = self
            .data
            .get(account)
            .ok_or("that account has no mail yet")?;
        let mailbox = match folder {
            Folder::Inbox => Some(data.inbox.name.clone()),
            Folder::Sent => data.sent.as_ref().map(|sent| sent.name.clone()),
        }
        .filter(|name| !name.is_empty() && uid != 0)
        .ok_or("this message is not on the server yet")?;
        let password = Self::password(account).await?;
        let host = known.imap_host.trim().to_string();
        let mut connection = Connection::open(&known, &password).await?;
        let raw = connection.whole(&host, &mailbox, uid).await;
        connection.close().await;
        raw
    }
}
