// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::time::{Duration, Instant};

use super::fake::{DRAFTS, Fake, SENT, SPAM, TRASH};
use super::harness::{Harness, WAIT};
use crate::{Account, Change, Draft, INBOX, MailSource, Person, Scope, Status};

const PASSWORD: &str = "right";
const ME: &str = "me@example.org";

fn account() -> Account {
    Account {
        address: ME.into(),
        imap_host: "imap.example.org".into(),
        imap_port: 993,
        smtp_host: "smtp.example.org".into(),
        smtp_port: 465,
        ..Account::default()
    }
}

fn fresh() -> Harness<Fake> {
    Harness::new(Fake::new(PASSWORD), account())
}

fn signed_in(seed: impl FnOnce(&Fake)) -> Harness<Fake> {
    let harness = fresh();
    seed(&harness.server);
    harness.sign_in(PASSWORD).expect("a sign-in");
    harness
}

fn mail(id: &str, from: &str, subject: &str, minute: u32, answers: Option<&str>) -> String {
    let threading = answers.map_or(String::new(), |parent| {
        format!("In-Reply-To: <{parent}@example.org>\r\nReferences: <{parent}@example.org>\r\n")
    });
    format!(
        "From: {from}\r\nTo: {ME}\r\nSubject: {subject}\r\n\
         Date: Mon, 16 Nov 2026 10:{minute:02}:00 +0000\r\n\
         Message-ID: <{id}@example.org>\r\n{threading}\
         Content-Type: text/plain; charset=utf-8\r\n\r\nThis is {id}.\r\n"
    )
}

#[test]
fn signing_in_fetches_the_inbox_and_sent_and_leaves_other_folders_for_later() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
        fake.deliver(SENT, &mail("b", ME, "Plans", 2, None));
        fake.deliver(SPAM, &mail("c", "win@example.net", "Prize", 3, None));
    });
    let log = harness.server.take_log();
    assert!(log.contains(&format!("sync {INBOX}")));
    assert!(log.contains(&format!("sync {SENT}")));
    assert!(!log.contains(&format!("sync {SPAM}")));
    assert_eq!(harness.titles(&Scope::AllInboxes), ["Lunch"]);
    assert!(harness.store.loaded(&harness.folder(INBOX)));
    assert!(!harness.store.loaded(&harness.folder(SPAM)));
    assert_eq!(harness.unread(SPAM), 1);
    assert!(matches!(harness.store.status(), Status::Synced(_)));
}

#[test]
fn opening_a_folder_fetches_that_folder_alone() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
        fake.deliver(SPAM, &mail("c", "win@example.net", "Prize", 3, None));
    });
    harness.server.take_log();
    let spam = harness.folder(SPAM);
    harness.store.open(&spam);
    harness.settle();
    assert_eq!(harness.server.take_log(), ["sync INBOX.spam"]);
    assert!(harness.store.loaded(&spam));
    assert_eq!(harness.titles(&spam), ["Prize"]);
}

#[test]
fn a_wrong_password_signs_nothing_in_and_saves_nothing() {
    let harness = fresh();
    let refused = harness.sign_in("wrong");
    assert!(refused.expect_err("a refusal").contains("refused"));
    assert!(harness.store.accounts().is_empty());
    assert!(harness.server.state().keyring.is_empty());
}

#[test]
fn a_password_the_server_stops_accepting_locks_the_account() {
    let harness = signed_in(|_| {});
    harness.server.state().password = "changed".into();
    harness.store.refresh();
    harness.settle();
    assert!(harness.store.locked(&harness.key()));
    assert!(matches!(harness.store.status(), Status::Failed(error) if error.contains("refused")));
    harness
        .store
        .sign_in_again(&harness.key(), "changed".into())
        .recv_timeout(WAIT)
        .expect("a reply")
        .expect("the new password");
    harness.settle();
    assert!(!harness.store.locked(&harness.key()));
    assert_eq!(harness.server.state().keyring[&harness.key()], "changed");
}

#[test]
fn trashing_hides_the_conversation_at_once_and_moves_it_on_the_server() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
        fake.deliver(INBOX, &mail("b", "ben@example.org", "Hike", 2, None));
    });
    let lunch = harness
        .held(&Scope::AllInboxes)
        .into_iter()
        .find(|c| c.title == "Lunch")
        .expect("the lunch conversation");
    harness.server.take_log();
    harness
        .store
        .change(&[lunch], &Scope::AllInboxes, Change::Trash);
    assert_eq!(harness.titles(&Scope::AllInboxes), ["Hike"]);
    harness.settle();
    assert!(
        harness
            .server
            .take_log()
            .contains(&"move INBOX [1] -> INBOX.Trash".to_string())
    );
    assert_eq!(harness.server.uids(INBOX), [2]);
    assert_eq!(harness.server.uids(TRASH).len(), 1);
    let trash = harness.folder(TRASH);
    harness.store.open(&trash);
    harness.settle();
    assert_eq!(harness.titles(&trash), ["Lunch"]);
}

#[test]
fn archiving_without_an_archive_folder_creates_one_first() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
    });
    let lunch = harness.held(&Scope::AllInboxes);
    harness.server.take_log();
    harness
        .store
        .change(&lunch, &Scope::AllInboxes, Change::Archive);
    harness.settle();
    let log = harness.server.take_log();
    let created = log
        .iter()
        .position(|entry| entry.starts_with("create "))
        .expect("a folder created");
    let moved = log
        .iter()
        .position(|entry| entry.starts_with("move INBOX [1] -> "))
        .expect("the message moved");
    assert!(created < moved);
    let archive = log[created].trim_start_matches("create ").to_string();
    assert_eq!(harness.server.uids(&archive).len(), 1);
    assert!(
        harness
            .store
            .folders(&harness.key())
            .iter()
            .any(|folder| folder.name == archive)
    );
}

#[test]
fn reading_a_conversation_marks_it_seen_on_the_server() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
        fake.deliver(
            INBOX,
            &mail("b", "ana@example.org", "Re: Lunch", 2, Some("a")),
        );
    });
    assert_eq!(harness.unread(INBOX), 2);
    let lunch = harness.held(&Scope::AllInboxes).remove(0);
    assert_eq!(lunch.unread, 2);
    harness.store.mark_read(&lunch);
    harness.settle();
    assert_eq!(harness.unread(INBOX), 0);
    assert!(
        harness.server.state().folders[INBOX]
            .messages
            .values()
            .all(|stored| stored.seen)
    );
    assert_eq!(harness.held(&Scope::AllInboxes)[0].unread, 0);
}

fn draft(harness: &Harness<Fake>) -> Draft {
    Draft {
        account: harness.key(),
        to: vec![Person {
            name: None,
            address: "ana@example.org".into(),
        }],
        subject: "Plans".into(),
        body: "Saturday?".into(),
        ..Draft::default()
    }
}

#[test]
fn a_sent_message_shows_at_once_and_its_copy_is_filed_in_sent() {
    let harness = signed_in(|_| {});
    harness
        .store
        .send(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a send reply")
        .expect("a sent message");
    assert_eq!(harness.titles(&Scope::AllInboxes), ["Plans"]);
    harness.settle();
    assert_eq!(harness.server.state().sent.len(), 1);
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.uids(SENT).len(), 1);
    let plans = harness.held(&Scope::AllInboxes);
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].messages.len(), 1);
    assert_eq!(plans[0].messages[0].mailbox, SENT);
}

#[test]
fn a_sent_copy_the_server_refuses_is_saved_on_a_later_sync() {
    let harness = signed_in(|_| {});
    harness.server.fail("append", true);
    harness
        .store
        .send(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a send reply")
        .expect("a sent message");
    harness.store.refresh();
    harness.settle();
    assert!(harness.server.uids(SENT).is_empty());
    assert!(matches!(harness.store.status(), Status::Failed(error) if error.contains("Sent")));
    harness.server.fail("append", false);
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.uids(SENT).len(), 1);
    assert!(matches!(harness.store.status(), Status::Synced(_)));
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.uids(SENT).len(), 1);
}

#[test]
fn a_message_that_also_reaches_the_inbox_is_still_saved_to_sent() {
    let harness = signed_in(|_| {});
    harness
        .store
        .send(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a send reply")
        .expect("a sent message");
    let raw = String::from_utf8(harness.server.state().sent[0].clone()).expect("UTF-8 mail");
    harness.server.deliver(INBOX, &raw);
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.uids(SENT).len(), 1);
}

#[test]
fn a_copy_the_server_filed_itself_is_not_saved_again() {
    let harness = signed_in(|_| {});
    harness.server.state().files_sent = true;
    harness
        .store
        .send(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a send reply")
        .expect("a sent message");
    harness.store.refresh();
    harness.settle();
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.uids(SENT).len(), 1);
    assert!(
        !harness
            .server
            .take_log()
            .iter()
            .any(|entry| entry.starts_with("append"))
    );
}

#[test]
fn a_reply_marks_the_message_it_answers_as_answered() {
    let harness = signed_in(|server| {
        server.deliver(INBOX, &mail("ask", "ana@example.org", "Plans", 1, None));
    });
    let reply = Draft {
        in_reply_to: Some("<ask@example.org>".into()),
        ..draft(&harness)
    };
    harness.server.take_log();
    harness
        .store
        .send(reply)
        .recv_timeout(WAIT)
        .expect("a send reply")
        .expect("a sent message");
    harness.settle();
    assert!(
        harness
            .server
            .take_log()
            .iter()
            .any(|entry| entry == &format!("flag {INBOX} [1] \\Answered true"))
    );
}

#[test]
fn a_message_the_server_does_not_take_is_reported_and_not_shown() {
    let harness = signed_in(|_| {});
    harness.server.fail("refuse", true);
    let refused = harness
        .store
        .send(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a send reply");
    assert!(refused.is_err());
    harness.settle();
    assert!(harness.titles(&Scope::AllInboxes).is_empty());
    assert!(harness.server.uids(SENT).is_empty());
}

#[test]
fn a_saved_draft_goes_to_drafts_and_replaces_the_one_before() {
    let harness = signed_in(|_| {});
    harness
        .store
        .save_draft(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a reply")
        .expect("a saved draft");
    harness.settle();
    let first = harness.server.uids(DRAFTS);
    assert_eq!(first.len(), 1);
    let again = Draft {
        body: "Sunday?".into(),
        replaces: Some((DRAFTS.into(), first[0])),
        ..draft(&harness)
    };
    harness
        .store
        .save_draft(again)
        .recv_timeout(WAIT)
        .expect("a reply")
        .expect("a saved draft");
    harness.settle();
    let second = harness.server.uids(DRAFTS);
    assert_eq!(second.len(), 1);
    assert_ne!(second, first);
}

#[test]
fn emptying_trash_keeps_mail_that_arrived_after_the_last_look() {
    let harness = signed_in(|fake| {
        fake.deliver(TRASH, &mail("a", "ana@example.org", "Old", 1, None));
        fake.deliver(TRASH, &mail("b", "ben@example.org", "Older", 2, None));
    });
    let trash = harness.folder(TRASH);
    harness.store.open(&trash);
    harness.settle();
    assert_eq!(harness.held(&trash).len(), 2);
    let late = harness
        .server
        .deliver(TRASH, &mail("c", "cy@example.org", "Late", 3, None));
    harness.server.take_log();
    harness.store.empty(&harness.key(), TRASH);
    assert!(harness.held(&trash).is_empty());
    harness.settle();
    assert_eq!(harness.server.take_log(), ["empty INBOX.Trash 2"]);
    assert_eq!(harness.server.uids(TRASH), [late]);
}

#[test]
fn a_blocked_sender_is_moved_to_junk_when_the_inbox_is_next_checked() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "pest@example.net", "Offer", 1, None));
        fake.deliver(INBOX, &mail("b", "ana@example.org", "Lunch", 2, None));
    });
    harness
        .store
        .block(&harness.key(), "Pest@Example.net", true);
    harness.settle();
    assert_eq!(harness.server.uids(INBOX), [2]);
    assert_eq!(harness.server.uids(SPAM).len(), 1);
    assert_eq!(harness.titles(&Scope::AllInboxes), ["Lunch"]);
    assert_eq!(harness.store.blocked(&harness.key()), ["pest@example.net"]);
}

#[test]
fn new_mail_announced_by_the_server_is_fetched_and_queued_as_an_arrival() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
    });
    std::thread::sleep(Duration::from_millis(200));
    harness
        .server
        .deliver(INBOX, &mail("b", "ben@example.org", "Hike", 2, None));
    let deadline = Instant::now() + WAIT;
    let mut arrivals = Vec::new();
    loop {
        arrivals.extend(harness.store.arrivals());
        let listed = harness
            .titles(&Scope::AllInboxes)
            .contains(&"Hike".to_string());
        if (!arrivals.is_empty() && listed) || Instant::now() > deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert_eq!(arrivals.len(), 1);
    assert_eq!(arrivals[0].subject, "Hike");
    assert!(
        harness
            .titles(&Scope::AllInboxes)
            .contains(&"Hike".to_string())
    );
}

#[test]
fn searching_the_server_finds_mail_outside_the_fetched_window() {
    let harness = signed_in(|fake| {
        fake.deliver(SPAM, &mail("a", "ana@example.org", "Invoice 114", 1, None));
        fake.deliver(SPAM, &mail("b", "ben@example.org", "Prize", 2, None));
    });
    let spam = harness.folder(SPAM);
    let found = harness
        .store
        .search_server(&spam, "invoice")
        .recv_timeout(WAIT)
        .expect("a reply")
        .expect("a search");
    harness.settle();
    assert_eq!(found, 1);
    assert_eq!(harness.titles(&spam), ["Invoice 114"]);
}

#[test]
fn signing_out_forgets_the_password_and_the_cached_mail() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
    });
    let cache = harness
        .dir
        .join("cache")
        .join(crate::cache::file_name(&harness.key()));
    assert!(cache.exists());
    harness.store.sign_out(&harness.account);
    harness.settle();
    assert!(harness.store.accounts().is_empty());
    assert!(harness.store.conversations().is_empty());
    assert!(harness.server.state().keyring.is_empty());
    assert!(!cache.exists());
    assert_eq!(harness.store.status(), Status::NoAccounts);
}

#[test]
fn a_trashed_conversation_stays_gone_while_the_server_refuses_and_moves_once_it_accepts() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
        fake.deliver(INBOX, &mail("b", "ben@example.org", "Hike", 2, None));
    });
    let lunch = harness
        .held(&Scope::AllInboxes)
        .into_iter()
        .find(|c| c.title == "Lunch")
        .expect("the lunch conversation");
    harness.server.fail("move", true);
    harness
        .store
        .change(&[lunch], &Scope::AllInboxes, Change::Trash);
    harness.settle();
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.titles(&Scope::AllInboxes), ["Hike"]);
    harness.server.fail("move", false);
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.uids(INBOX), [2]);
    assert_eq!(harness.server.uids(TRASH).len(), 1);
    assert_eq!(harness.titles(&Scope::AllInboxes), ["Hike"]);
}

#[test]
fn a_message_that_cannot_go_yet_waits_and_goes_once_the_server_answers() {
    let harness = signed_in(|_| {});
    harness.server.fail("send", true);
    harness
        .store
        .send(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a send reply")
        .expect("a queued message");
    harness.settle();
    let waiting = harness.held(&Scope::AllInboxes);
    assert_eq!(waiting.len(), 1);
    assert!(waiting[0].messages[0].waiting);
    assert!(harness.server.state().sent.is_empty());
    harness.server.fail("send", false);
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.state().sent.len(), 1);
    harness.store.refresh();
    harness.settle();
    assert_eq!(harness.server.uids(SENT).len(), 1);
    let sent = harness.held(&Scope::AllInboxes);
    assert_eq!(sent.len(), 1);
    assert!(!sent[0].messages[0].waiting);
}

#[test]
fn a_waiting_message_the_server_later_refuses_is_kept_in_drafts() {
    let harness = signed_in(|_| {});
    harness.server.fail("send", true);
    harness
        .store
        .send(draft(&harness))
        .recv_timeout(WAIT)
        .expect("a send reply")
        .expect("a queued message");
    harness.server.fail("send", false);
    harness.server.fail("refuse", true);
    harness.store.refresh();
    harness.settle();
    assert!(harness.server.state().sent.is_empty());
    assert_eq!(harness.server.uids(DRAFTS).len(), 1);
    assert!(harness.titles(&Scope::AllInboxes).is_empty());
    assert!(matches!(harness.store.status(), Status::Failed(error) if error.contains("Drafts")));
}

#[test]
fn a_sync_that_changes_nothing_leaves_the_cache_file_alone() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
    });
    let file = std::fs::read_dir(harness.dir.join("cache"))
        .expect("a cache folder")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .expect("an account's cache file");
    let written = |path: &std::path::Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .expect("a modified time")
    };
    let before = written(&file);
    std::thread::sleep(Duration::from_millis(20));
    harness.store.refresh();
    harness.settle();
    assert_eq!(written(&file), before);
    harness
        .server
        .deliver(INBOX, &mail("b", "ben@example.org", "Hike", 2, None));
    harness.store.refresh();
    harness.settle();
    assert_ne!(written(&file), before);
}

#[test]
fn marking_part_of_a_conversation_read_leaves_the_rest_unread() {
    let harness = signed_in(|fake| {
        fake.deliver(INBOX, &mail("a", "ana@example.org", "Lunch", 1, None));
        fake.deliver(
            INBOX,
            &mail("b", "ana@example.org", "Re: Lunch", 2, Some("a")),
        );
    });
    let mut lunch = harness
        .held(&Scope::AllInboxes)
        .into_iter()
        .find(|c| c.title == "Lunch")
        .expect("the lunch conversation");
    assert_eq!(lunch.unread, 2);
    lunch
        .copies
        .retain(|copy| copy.message_id.as_deref() == Some("<a@example.org>"));
    harness.store.mark_read(&lunch);
    harness.settle();
    let state = harness.server.state();
    let inbox = &state.folders[INBOX].messages;
    assert!(inbox[&1].seen);
    assert!(!inbox[&2].seen);
}
