// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use serde::{Deserialize, Serialize};

use crate::cache::AccountData;

pub(crate) const GIVE_UP_AFTER: u32 = 5;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Owed {
    pub changes: Vec<OwedChange>,
    pub sends: Vec<OwedSend>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct OwedChange {
    pub mailbox: String,
    pub uid_validity: u32,
    pub uids: Vec<u32>,
    pub action: Action,
    #[serde(default)]
    pub failures: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum Action {
    Flag(bool),
    Read(bool),
    Delete,
    Move { to: String, create: bool },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct OwedSend {
    pub message_id: String,
    pub raw: String,
    pub from: Option<String>,
    pub to: Vec<String>,
    pub answers: Option<String>,
}

pub(crate) enum NotSent {
    Later(String),
    Never(String),
}

pub(crate) fn apply_locally(data: &mut AccountData) {
    let changes = data.owed.changes.clone();
    for change in changes {
        let Some(mailbox) = data.mailbox_mut(&change.mailbox) else {
            continue;
        };
        if mailbox.uid_validity != change.uid_validity {
            continue;
        }
        for uid in &change.uids {
            match &change.action {
                Action::Flag(on) => {
                    if let Some(message) = mailbox.messages.get_mut(uid) {
                        message.flagged = *on;
                    }
                }
                Action::Read(on) => {
                    if let Some(message) = mailbox.messages.get_mut(uid) {
                        message.seen = *on;
                    }
                }
                Action::Delete | Action::Move { .. } => {
                    mailbox.messages.remove(uid);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Action, OwedChange, apply_locally};
    use crate::cache::AccountData;

    #[test]
    fn an_owed_move_keeps_its_messages_out_of_a_fresh_listing() {
        let mut data: AccountData = serde_json::from_str(
            r#"{"inbox":{"name":"INBOX","uid_validity":3,"messages":{"7":{"uid":7,"message_id":"<a@x>","in_reply_to":null,"references":[],"from":null,"to":[],"cc":[],"subject":"Hi","date":"2026-11-09T12:00:00Z","text":"","attachments":[],"seen":false,"partial":false},"8":{"uid":8,"message_id":"<b@x>","in_reply_to":null,"references":[],"from":null,"to":[],"cc":[],"subject":"Hi","date":"2026-11-09T12:00:00Z","text":"","attachments":[],"seen":false,"partial":false}}}}"#,
        )
        .expect("account data");
        let owe = |uid_validity, action| OwedChange {
            mailbox: "INBOX".into(),
            uid_validity,
            uids: vec![7],
            action,
            failures: 0,
        };
        data.owed.changes.push(owe(
            3,
            Action::Move {
                to: "Trash".into(),
                create: false,
            },
        ));
        data.owed.changes.push(owe(2, Action::Read(true)));
        apply_locally(&mut data);
        assert!(!data.inbox.messages.contains_key(&7));
        assert!(!data.inbox.messages[&8].seen);
    }
}
