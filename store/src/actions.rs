// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use crate::model::{Change, Conversation, FolderInfo, INBOX, Role, Scope};

pub(crate) fn targets(
    conversation: &Conversation,
    scope: &Scope,
    change: &Change,
) -> Vec<(String, u32)> {
    let on_server = conversation
        .copies
        .iter()
        .filter(|copy| copy.uid != 0 && !copy.mailbox.is_empty());
    let chosen: Vec<_> = match change {
        Change::Archive | Change::Trash | Change::Junk | Change::Move(_) | Change::Delete => {
            on_server
                .filter(|copy| scope.shows(&copy.mailbox))
                .collect()
        }
        Change::Flag(true) => {
            let all: Vec<_> = on_server.collect();
            let latest = all.iter().rev().find(|copy| scope.shows(&copy.mailbox));
            latest.or(all.last()).copied().into_iter().collect()
        }
        Change::Flag(false) => on_server.filter(|copy| copy.flagged).collect(),
        Change::Read(true) => on_server.filter(|copy| !copy.seen).collect(),
        Change::Read(false) => on_server
            .rev()
            .find(|copy| {
                copy.sender
                    .as_ref()
                    .is_some_and(|sender| !sender.eq_ignore_ascii_case(&conversation.account))
            })
            .into_iter()
            .collect(),
        Change::Pin(_) => Vec::new(),
    };
    chosen
        .into_iter()
        .map(|copy| (copy.mailbox.clone(), copy.uid))
        .collect()
}

pub(crate) fn role_folder(folders: &[FolderInfo], role: Role) -> Option<String> {
    folders
        .iter()
        .find(|folder| folder.role == Some(role) && folder.selectable)
        .map(|folder| folder.name.clone())
}

pub(crate) fn new_folder_name(folders: &[FolderInfo], leaf: &str) -> String {
    let nested = folders.iter().any(|folder| {
        folder.name.len() > INBOX.len() + 1
            && folder.name[..INBOX.len()].eq_ignore_ascii_case(INBOX)
            && folder.name[INBOX.len()..].starts_with('.')
    });
    if nested {
        format!("{INBOX}.{leaf}")
    } else {
        leaf.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Person;
    use crate::thread::tests::message;

    fn conversation(messages: Vec<crate::model::Message>) -> Conversation {
        Conversation {
            key: "k".into(),
            account: "me@example.org".into(),
            title: "Lunch".into(),
            members: Vec::new(),
            unread: 0,
            in_inbox: true,
            flagged: false,
            pinned: false,
            copies: messages.iter().map(crate::model::Copy::of).collect(),
            messages,
            starts_with_context: false,
        }
    }

    #[test]
    fn moves_only_the_messages_in_the_folder_being_viewed() {
        let inbox = message("a", &[], "ana@example.com", 1);
        let mut sent = message("b", &["a"], "me@example.org", 2);
        sent.mailbox = "INBOX.Sent".into();
        let thread = conversation(vec![inbox, sent]);
        assert_eq!(
            targets(&thread, &Scope::AllInboxes, &Change::Archive),
            [("INBOX".to_string(), 1)]
        );
        let sent_view = Scope::Mailbox {
            account: "me@example.org".into(),
            name: "INBOX.Sent".into(),
        };
        assert_eq!(
            targets(&thread, &sent_view, &Change::Trash),
            [("INBOX.Sent".to_string(), 2)]
        );
    }

    #[test]
    fn marks_unread_the_latest_message_someone_else_sent() {
        let first = message("a", &[], "ana@example.com", 1);
        let second = message("b", &["a"], "ben@example.com", 2);
        let mut mine = message("c", &["a", "b"], "me@example.org", 3);
        mine.from = Some(Person {
            name: None,
            address: "Me@Example.org".into(),
        });
        let thread = conversation(vec![first, second, mine]);
        assert_eq!(
            targets(&thread, &Scope::AllInboxes, &Change::Read(false)),
            [("INBOX".to_string(), 2)]
        );
    }

    #[test]
    fn creates_archive_beside_the_other_folders() {
        let folder = |name: &str| FolderInfo {
            name: name.into(),
            label: name.into(),
            depth: 0,
            role: None,
            selectable: true,
            unread: 0,
        };
        assert_eq!(
            new_folder_name(&[folder("INBOX"), folder("INBOX.Sent")], "Archive"),
            "INBOX.Archive"
        );
        assert_eq!(
            new_folder_name(&[folder("INBOX"), folder("Sent")], "Archive"),
            "Archive"
        );
    }
}
