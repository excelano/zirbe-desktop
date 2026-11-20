// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::future::Future;
use std::time::Duration;

use crate::cache::Mailbox;
use crate::imap::Connection;
use crate::model::{Account, FolderInfo};
use crate::{secrets, smtp};

pub(crate) trait Server: Send + Sync + 'static {
    type Session: Session;

    fn open_keyring(&self) -> impl Future<Output = Result<(), String>> + Send;
    fn password(&self, account: &str) -> impl Future<Output = Result<String, String>> + Send;
    fn save_password(
        &self,
        account: &str,
        password: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn forget_password(&self, account: &str) -> impl Future<Output = Result<(), String>> + Send;
    fn connect(
        &self,
        account: &Account,
        password: &str,
    ) -> impl Future<Output = Result<Self::Session, String>> + Send;
    fn check_smtp(
        &self,
        account: &Account,
        password: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn send(
        &self,
        account: &Account,
        password: &str,
        message: lettre::Message,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

pub(crate) trait Session: Sized + Send {
    fn close(self) -> impl Future<Output = ()> + Send;
    fn folders(
        &mut self,
        host: &str,
    ) -> impl Future<Output = Result<Vec<FolderInfo>, String>> + Send;
    fn sync(
        &mut self,
        host: &str,
        mailbox: &mut Mailbox,
    ) -> impl Future<Output = Result<Vec<u32>, String>> + Send;
    fn search_into(
        &mut self,
        host: &str,
        mailbox: &mut Mailbox,
        terms: &[String],
    ) -> impl Future<Output = Result<usize, String>> + Send;
    fn watch(
        self,
        host: &str,
        mailbox: &str,
        patience: Duration,
    ) -> impl Future<Output = Result<(Self, u32, bool), String>> + Send;
    fn whole(
        &mut self,
        host: &str,
        mailbox: &str,
        uid: u32,
    ) -> impl Future<Output = Result<Vec<u8>, String>> + Send;
    fn set_flag(
        &mut self,
        host: &str,
        mailbox: &str,
        uids: &[u32],
        flag: &str,
        on: bool,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn move_to(
        &mut self,
        host: &str,
        mailbox: &str,
        uids: &[u32],
        destination: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn delete(
        &mut self,
        host: &str,
        mailbox: &str,
        uids: &[u32],
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn empty(
        &mut self,
        host: &str,
        mailbox: &str,
        highest: u32,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn create(
        &mut self,
        host: &str,
        mailbox: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn append(
        &mut self,
        host: &str,
        mailbox: &str,
        raw: &[u8],
    ) -> impl Future<Output = Result<(), String>> + Send;
    fn append_with(
        &mut self,
        host: &str,
        mailbox: &str,
        raw: &[u8],
        flags: &str,
    ) -> impl Future<Output = Result<(), String>> + Send;
}

pub(crate) struct Live;

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> T {
    tokio::task::spawn_blocking(work)
        .await
        .expect("a blocking task")
}

impl Server for Live {
    type Session = Connection;

    async fn open_keyring(&self) -> Result<(), String> {
        blocking(secrets::open).await
    }

    async fn password(&self, account: &str) -> Result<String, String> {
        let key = account.to_string();
        blocking(move || secrets::password(&key)).await
    }

    async fn save_password(&self, account: &str, password: &str) -> Result<(), String> {
        let (key, password) = (account.to_string(), password.to_string());
        blocking(move || secrets::save(&key, &password)).await
    }

    async fn forget_password(&self, account: &str) -> Result<(), String> {
        let key = account.to_string();
        blocking(move || secrets::forget(&key)).await
    }

    async fn connect(&self, account: &Account, password: &str) -> Result<Connection, String> {
        Connection::open(account, password).await
    }

    async fn check_smtp(&self, account: &Account, password: &str) -> Result<(), String> {
        smtp::check(account, password).await
    }

    async fn send(
        &self,
        account: &Account,
        password: &str,
        message: lettre::Message,
    ) -> Result<(), String> {
        smtp::send(account, password, message).await
    }
}
