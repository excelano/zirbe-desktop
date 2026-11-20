// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc as reply;
use std::time::Duration;

use crate::server::Server;
use crate::{Account, Command, Conversation, MailSource, Scope, Store};

pub(crate) const WAIT: Duration = Duration::from_secs(120);

pub(crate) struct Harness<S> {
    pub server: S,
    pub store: Store,
    pub account: Account,
    pub dir: PathBuf,
}

impl<S: Server + Clone> Harness<S> {
    pub(crate) fn new(server: S, account: Account) -> Harness<S> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zirbe-store-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        let store =
            Store::with_server(dir.join("config"), dir.join("cache"), server.clone(), || {});
        Harness {
            server,
            store,
            account,
            dir,
        }
    }
}

impl<S> Harness<S> {
    pub(crate) fn sign_in(&self, password: &str) -> Result<(), String> {
        let result = self
            .store
            .sign_in(self.account.clone(), password.into())
            .recv_timeout(WAIT)
            .expect("a sign-in reply");
        self.settle();
        result
    }

    pub(crate) fn settle(&self) {
        let (sender, receiver) = reply::channel();
        let _ = self.store.commands.send(Command::Settle(sender));
        receiver.recv_timeout(WAIT).expect("the worker to settle");
    }

    pub(crate) fn key(&self) -> String {
        self.account.key()
    }

    pub(crate) fn folder(&self, name: &str) -> Scope {
        Scope::Mailbox {
            account: self.key(),
            name: name.into(),
        }
    }

    pub(crate) fn held(&self, scope: &Scope) -> Vec<Conversation> {
        self.store
            .conversations()
            .iter()
            .filter(|conversation| scope.holds(conversation))
            .cloned()
            .collect()
    }

    pub(crate) fn titles(&self, scope: &Scope) -> Vec<String> {
        self.held(scope).into_iter().map(|c| c.title).collect()
    }

    pub(crate) fn unread(&self, name: &str) -> u32 {
        self.store
            .folders(&self.key())
            .into_iter()
            .find(|folder| folder.name == name)
            .map_or(0, |folder| folder.unread)
    }
}

impl<S> Drop for Harness<S> {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
