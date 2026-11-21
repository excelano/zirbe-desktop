// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::harness::Harness;
use crate::cache::Mailbox;
use crate::imap::Connection;
use crate::owed::NotSent;
use crate::server::{Server, Session};
use crate::{Account, Change, Draft, INBOX, MailSource, Person, Role, Scope, smtp};

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

#[derive(Clone, Default)]
struct Scratch {
    passwords: Arc<Mutex<BTreeMap<String, String>>>,
}

impl Scratch {
    fn passwords(&self) -> MutexGuard<'_, BTreeMap<String, String>> {
        self.passwords.lock().expect("password lock")
    }
}

impl Server for Scratch {
    type Session = Connection;

    async fn open_keyring(&self) -> Result<(), String> {
        Ok(())
    }

    async fn password(&self, account: &str) -> Result<String, String> {
        self.passwords()
            .get(account)
            .cloned()
            .ok_or_else(|| format!("no password is saved for {account}"))
    }

    async fn save_password(&self, account: &str, password: &str) -> Result<(), String> {
        self.passwords()
            .insert(account.to_string(), password.to_string());
        Ok(())
    }

    async fn forget_password(&self, account: &str) -> Result<(), String> {
        self.passwords().remove(account);
        Ok(())
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
        from: Option<&str>,
        to: &[String],
        raw: &[u8],
    ) -> Result<(), NotSent> {
        smtp::send(account, password, from, to, raw).await
    }
}

fn setting(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        panic!(
            "the live tests need ZIRBE_TEST_ADDRESS, ZIRBE_TEST_HOST and ZIRBE_TEST_PASSWORD for a test account; {name} is not set"
        )
    })
}

struct Found {
    uid: u32,
    seen: bool,
}

struct Live {
    harness: Harness<Scratch>,
    password: String,
    tag: String,
    _turn: MutexGuard<'static, ()>,
}

impl Live {
    fn new() -> Live {
        let turn = ONE_AT_A_TIME
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let address = setting("ZIRBE_TEST_ADDRESS");
        let host = setting("ZIRBE_TEST_HOST");
        let preset = crate::for_address(&address).expect("an email address");
        let account = Account {
            address,
            imap_host: host.clone(),
            imap_port: preset.imap_port,
            smtp_host: host,
            smtp_port: preset.smtp_port,
            ..Account::default()
        };
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("a clock after 1970")
            .as_nanos();
        let live = Live {
            harness: Harness::new(Scratch::default(), account),
            password: setting("ZIRBE_TEST_PASSWORD"),
            tag: format!("zirbe-live-{nanos}"),
            _turn: turn,
        };
        live.harness
            .sign_in(&live.password)
            .expect("the test account to sign in");
        live
    }

    fn on_server<T>(&self, work: impl AsyncFnOnce(&mut Connection, &str) -> T) -> T {
        let account = self.harness.account.clone();
        let host = account.imap_host.clone();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
            .block_on(async {
                let mut connection = Connection::open(&account, &self.password)
                    .await
                    .expect("a connection to the test account");
                let result = work(&mut connection, &host).await;
                connection.close().await;
                result
            })
    }

    fn role(&self, role: Role) -> String {
        self.harness
            .store
            .folders(&self.harness.key())
            .into_iter()
            .find(|folder| folder.role == Some(role))
            .map(|folder| folder.name)
            .unwrap_or_else(|| panic!("the test account has no {role:?} folder"))
    }

    fn mail(&self, label: &str, minute: u32) -> String {
        format!(
            "From: Live Test <tester@example.org>\r\nTo: {}\r\nSubject: {} {label}\r\n\
             Date: Mon, 16 Nov 2026 10:{minute:02}:00 +0000\r\n\
             Message-ID: <{}-{label}@example.org>\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\r\nA live test message.\r\n",
            self.harness.account.address, self.tag, self.tag
        )
    }

    fn seed(&self, folder: &str, labels: &[&str]) {
        let mails: Vec<String> = labels
            .iter()
            .enumerate()
            .map(|(index, label)| self.mail(label, index as u32))
            .collect();
        self.on_server(async |connection, host| {
            for mail in &mails {
                connection
                    .append_with(host, folder, mail.as_bytes(), "()")
                    .await
                    .expect("a seeded message");
            }
        });
    }

    fn found(&self, folder: &str) -> Vec<Found> {
        let tag = self.tag.clone();
        self.on_server(async |connection, host| {
            let mut mailbox = Mailbox {
                name: folder.to_string(),
                ..Mailbox::default()
            };
            connection
                .sync(host, &mut mailbox)
                .await
                .expect("the folder read");
            mailbox
                .messages
                .into_iter()
                .filter(|(_, message)| message.subject.contains(&tag))
                .map(|(uid, message)| Found {
                    uid,
                    seen: message.seen,
                })
                .collect()
        })
    }

    fn tagged(&self, scope: &Scope) -> Vec<String> {
        self.harness
            .titles(scope)
            .into_iter()
            .filter(|title| title.contains(&self.tag))
            .collect()
    }

    fn eventually(&self, what: &str, done: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(90);
        while !done() {
            assert!(Instant::now() < deadline, "gave up waiting for {what}");
            std::thread::sleep(Duration::from_secs(3));
            self.harness.store.refresh();
            self.harness.settle();
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        let folders: Vec<String> = self
            .harness
            .store
            .folders(&self.harness.key())
            .into_iter()
            .filter(|folder| folder.selectable)
            .map(|folder| folder.name)
            .collect();
        for folder in folders {
            let uids: Vec<u32> = self.found(&folder).iter().map(|found| found.uid).collect();
            if !uids.is_empty() {
                self.on_server(async |connection, host| {
                    let _ = connection.delete(host, &folder, &uids).await;
                });
            }
        }
    }
}

#[test]
#[ignore = "needs the live test account"]
fn live_opening_junk_fetches_and_shows_its_mail() {
    let live = Live::new();
    let junk = live.role(Role::Junk);
    live.seed(&junk, &["one", "two", "three"]);
    let scope = live.harness.folder(&junk);
    live.harness.store.open(&scope);
    live.harness.settle();
    assert!(live.harness.store.loaded(&scope));
    assert_eq!(live.tagged(&scope).len(), 3);
}

#[test]
#[ignore = "needs the live test account"]
fn live_trashing_moves_the_message_on_the_server() {
    let live = Live::new();
    live.seed(INBOX, &["trash-me"]);
    live.harness.store.refresh();
    live.harness.settle();
    let conversation = live
        .harness
        .held(&Scope::AllInboxes)
        .into_iter()
        .find(|c| c.title.contains(&live.tag))
        .expect("the seeded conversation");
    live.harness
        .store
        .change(&[conversation], &Scope::AllInboxes, Change::Trash);
    live.harness.settle();
    assert!(live.found(INBOX).is_empty());
    assert_eq!(live.found(&live.role(Role::Trash)).len(), 1);
    assert!(live.tagged(&Scope::AllInboxes).is_empty());
}

#[test]
#[ignore = "needs the live test account"]
fn live_reading_marks_the_message_seen_on_the_server() {
    let live = Live::new();
    live.seed(INBOX, &["read-me"]);
    live.harness.store.refresh();
    live.harness.settle();
    assert!(live.found(INBOX).iter().all(|found| !found.seen));
    let conversation = live
        .harness
        .held(&Scope::AllInboxes)
        .into_iter()
        .find(|c| c.title.contains(&live.tag))
        .expect("the seeded conversation");
    live.harness.store.mark_read(&conversation);
    live.harness.settle();
    let found = live.found(INBOX);
    assert_eq!(found.len(), 1);
    assert!(found[0].seen);
}

#[test]
#[ignore = "needs the live test account"]
fn live_a_message_sent_to_yourself_lands_in_sent_and_the_inbox() {
    let live = Live::new();
    let draft = Draft {
        account: live.harness.key(),
        to: vec![Person {
            name: None,
            address: live.harness.account.address.clone(),
        }],
        subject: format!("{} sent", live.tag),
        body: "A live test message sent to itself.".into(),
        ..Draft::default()
    };
    live.harness
        .store
        .send(draft)
        .recv_timeout(super::harness::WAIT)
        .expect("a send reply")
        .expect("a sent message");
    let sent = live.role(Role::Sent);
    live.eventually("the copy to reach Sent", || live.found(&sent).len() == 1);
    live.eventually("the message to reach the inbox", || {
        !live.found(INBOX).is_empty()
    });
    live.harness.store.refresh();
    live.harness.settle();
    assert_eq!(live.tagged(&Scope::AllInboxes).len(), 1);
}

#[test]
#[ignore = "needs the live test account"]
fn live_a_saved_draft_lands_in_drafts() {
    let live = Live::new();
    let draft = Draft {
        account: live.harness.key(),
        to: vec![Person {
            name: None,
            address: live.harness.account.address.clone(),
        }],
        subject: format!("{} draft", live.tag),
        body: "A live test draft.".into(),
        ..Draft::default()
    };
    live.harness
        .store
        .save_draft(draft)
        .recv_timeout(super::harness::WAIT)
        .expect("a reply")
        .expect("a saved draft");
    live.harness.settle();
    assert_eq!(live.found(&live.role(Role::Drafts)).len(), 1);
}

#[test]
#[ignore = "needs the live test account"]
fn live_new_mail_shows_without_a_refresh() {
    let live = Live::new();
    std::thread::sleep(Duration::from_secs(5));
    live.seed(INBOX, &["arrives"]);
    let deadline = Instant::now() + Duration::from_secs(45);
    while live.tagged(&Scope::AllInboxes).is_empty() {
        assert!(
            Instant::now() < deadline,
            "the watcher never reported the new mail"
        );
        std::thread::sleep(Duration::from_secs(1));
        live.harness.settle();
    }
}

#[test]
#[ignore = "needs the live test account"]
fn live_an_unchanged_folder_is_skipped_and_a_changed_one_is_read() {
    let live = Live::new();
    live.seed(INBOX, &["watched"]);
    let tag = live.tag.clone();
    live.on_server(async |connection, host| {
        let mut mailbox = Mailbox {
            name: INBOX.to_string(),
            ..Mailbox::default()
        };
        let tagged = |mailbox: &Mailbox| {
            mailbox
                .messages
                .iter()
                .find(|(_, message)| message.subject.contains(&tag))
                .map(|(uid, message)| (*uid, message.seen))
                .expect("the seeded message")
        };
        connection
            .sync(host, &mut mailbox)
            .await
            .expect("a first read");
        let (uid, seen) = tagged(&mailbox);
        assert!(!seen);
        let first = mailbox.listed.expect("a recorded listing");
        assert!(first.modseq.is_some(), "the test server offers CONDSTORE");

        tokio::time::sleep(Duration::from_millis(1100)).await;
        connection
            .sync(host, &mut mailbox)
            .await
            .expect("a second read");
        assert_eq!(
            mailbox.listed,
            Some(first),
            "an unchanged folder was read again"
        );

        connection
            .set_flag(host, INBOX, &[uid], "\\Seen", true)
            .await
            .expect("the flag set");
        connection
            .sync(host, &mut mailbox)
            .await
            .expect("a third read");
        assert!(tagged(&mailbox).1, "the flag change was missed");
        assert_ne!(mailbox.listed, Some(first));
    });
}

#[test]
#[ignore = "needs the live test account"]
fn live_a_reaction_arrives_as_a_standard_reaction() {
    let live = Live::new();
    let draft = Draft {
        account: live.harness.key(),
        to: vec![Person {
            name: None,
            address: live.harness.account.address.clone(),
        }],
        subject: format!("Re: {} reacted", live.tag),
        body: "\u{1F44D}".into(),
        in_reply_to: Some("<zirbe-live-target@example.org>".into()),
        references: vec!["<zirbe-live-target@example.org>".into()],
        reaction: Some("\u{1F44D}".into()),
        ..Draft::default()
    };
    live.harness
        .store
        .send(draft)
        .recv_timeout(super::harness::WAIT)
        .expect("a send reply")
        .expect("a sent reaction");
    live.eventually("the reaction to reach the inbox", || {
        !live.found(INBOX).is_empty()
    });
    let uid = live.found(INBOX)[0].uid;
    let raw = live.on_server(async |connection, host| {
        connection
            .whole(host, INBOX, uid)
            .await
            .expect("the raw message")
    });
    let text = String::from_utf8_lossy(&raw).to_string();
    assert!(text.contains("Content-Disposition: reaction"), "{text}");
    let without_ours = text.replace("X-Zirbe-Reaction:", "X-Ignored:");
    let message = mail_parser::MessageParser::default()
        .parse(without_ours.as_bytes())
        .expect("a parsed reaction");
    assert_eq!(
        crate::content::reaction(&message).as_deref(),
        Some("\u{1F44D}")
    );
}

#[test]
#[ignore = "needs the live test account"]
fn live_a_newsletter_reads_as_one_conversation() {
    let live = Live::new();
    let issue = |label: &str, minute: u32| {
        format!(
            "From: Live List <list@example.org>\r\nTo: {}\r\nSubject: {} {label}\r\n\
             Date: Mon, 16 Nov 2026 10:{minute:02}:00 +0000\r\n\
             Message-ID: <{}-{label}@example.org>\r\n\
             List-Id: \"Live {}\" <{}.lists.example.org>\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\r\nThis week's news.\r\n",
            live.harness.account.address, live.tag, live.tag, live.tag, live.tag
        )
    };
    let issues = [issue("first", 1), issue("second", 2)];
    live.on_server(async |connection, host| {
        for raw in &issues {
            connection
                .append_with(host, INBOX, raw.as_bytes(), "()")
                .await
                .expect("a seeded issue");
        }
    });
    live.harness.store.refresh();
    live.harness.settle();
    let held = live.harness.held(&Scope::AllInboxes);
    let lists: Vec<_> = held
        .iter()
        .filter(|conversation| conversation.title.contains(&live.tag))
        .collect();
    assert_eq!(lists.len(), 1);
    assert_eq!(lists[0].title, format!("Live {}", live.tag));
    assert_eq!(lists[0].messages.len(), 2);
}
