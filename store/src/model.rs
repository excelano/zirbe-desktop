// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub address: String,
    pub name: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
}

impl Account {
    pub fn key(&self) -> String {
        self.address.trim().to_lowercase()
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

impl Scope {
    pub fn holds(&self, conversation: &Conversation) -> bool {
        match self {
            Scope::AllInboxes => conversation.in_inbox,
            Scope::Mailbox { account, name } => {
                &conversation.account == account
                    && if name.eq_ignore_ascii_case(INBOX) {
                        conversation.in_inbox
                    } else {
                        conversation
                            .messages
                            .iter()
                            .any(|message| &message.mailbox == name)
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
    pub subject: String,
    pub body: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
}
