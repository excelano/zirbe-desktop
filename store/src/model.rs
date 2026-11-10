// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub address: String,
    pub name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub colour: Option<u8>,
}

pub const COLOURS: u8 = 8;

impl Account {
    pub fn key(&self) -> String {
        self.address.trim().to_lowercase()
    }

    pub fn title(&self) -> &str {
        if self.label.trim().is_empty() {
            self.address.trim()
        } else {
            self.label.trim()
        }
    }

    pub fn me(&self) -> Person {
        Person {
            name: (!self.name.trim().is_empty()).then(|| self.name.trim().to_string()),
            address: self.address.trim().to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Person {
    pub name: Option<String>,
    pub address: String,
}

impl Person {
    pub fn display(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.address)
    }

    pub fn short(&self) -> &str {
        match &self.name {
            Some(name) => name.split_whitespace().next().unwrap_or(name),
            None => self.address.split('@').next().unwrap_or(&self.address),
        }
    }

    pub fn is(&self, address: &str) -> bool {
        self.address.eq_ignore_ascii_case(address.trim())
    }
}

pub const INBOX: &str = "INBOX";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Role {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Trash,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FolderInfo {
    pub name: String,
    pub label: String,
    pub depth: usize,
    pub role: Option<Role>,
    pub selectable: bool,
    pub unread: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Scope {
    #[default]
    AllInboxes,
    Mailbox {
        account: String,
        name: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Archive,
    Trash,
    Junk,
    Move(String),
    Delete,
    Flag(bool),
    Read(bool),
    Pin(bool),
}

impl Scope {
    pub fn shows(&self, mailbox: &str) -> bool {
        match self {
            Scope::AllInboxes => mailbox.eq_ignore_ascii_case(INBOX),
            Scope::Mailbox { name, .. } if name.eq_ignore_ascii_case(INBOX) => {
                mailbox.eq_ignore_ascii_case(INBOX)
            }
            Scope::Mailbox { name, .. } => mailbox == name,
        }
    }

    pub fn holds(&self, conversation: &Conversation) -> bool {
        match self {
            Scope::AllInboxes => conversation.in_inbox,
            Scope::Mailbox { account, name } => {
                &conversation.account == account
                    && if name.eq_ignore_ascii_case(INBOX) {
                        conversation.in_inbox
                    } else {
                        conversation.copies.iter().any(|copy| &copy.mailbox == name)
                    }
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    #[serde(default)]
    pub mailbox: String,
    pub uid: u32,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from: Option<Person>,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub subject: String,
    pub date: DateTime<Utc>,
    pub text: String,
    pub attachments: Vec<String>,
    pub seen: bool,
    pub partial: bool,
    #[serde(default)]
    pub html: bool,
    #[serde(default)]
    pub flagged: bool,
    #[serde(default)]
    pub reaction: Option<String>,
    #[serde(default)]
    pub invite: Option<Invite>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invite {
    pub method: String,
    pub summary: String,
    pub start: Option<When>,
    pub end: Option<When>,
    pub location: String,
    pub organizer: String,
    pub join: Option<String>,
    pub sequence: u32,
    pub cancelled: bool,
    pub answer: Option<(String, String)>,
    pub ics: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct When {
    pub value: String,
    pub zone: Option<String>,
}

impl Message {
    pub fn in_inbox(&self) -> bool {
        self.mailbox.eq_ignore_ascii_case(INBOX)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub key: String,
    pub account: String,
    pub title: String,
    pub members: Vec<Person>,
    pub messages: Vec<Message>,
    pub unread: usize,
    pub in_inbox: bool,
    pub flagged: bool,
    pub pinned: bool,
    pub copies: Vec<Copy>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Copy {
    pub mailbox: String,
    pub uid: u32,
    pub message_id: Option<String>,
    pub sender: Option<String>,
    pub seen: bool,
    pub flagged: bool,
}

impl Copy {
    pub fn of(message: &Message) -> Copy {
        Copy {
            mailbox: message.mailbox.clone(),
            uid: message.uid,
            message_id: message.message_id.clone(),
            sender: message.from.as_ref().map(|from| from.address.clone()),
            seen: message.seen,
            flagged: message.flagged,
        }
    }
}

impl Conversation {
    pub fn last(&self) -> Option<&Message> {
        self.messages.last()
    }

    pub fn latest(&self) -> DateTime<Utc> {
        self.last()
            .map_or(DateTime::<Utc>::MIN_UTC, |message| message.date)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Syncing,
    Synced(DateTime<Utc>),
    Failed(String),
    NoAccounts,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub account: String,
    pub to: Vec<Person>,
    pub cc: Vec<Person>,
    pub bcc: Vec<Person>,
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub attachments: Vec<Outgoing>,
    pub replaces: Option<(String, u32)>,
    pub reaction: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Original {
    pub text: String,
    pub attachments: Vec<Outgoing>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outgoing {
    pub name: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrival {
    pub account: String,
    pub message_id: Option<String>,
    pub sender: String,
    pub subject: String,
    pub text: String,
}
