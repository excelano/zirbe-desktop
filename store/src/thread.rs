// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::model::{Conversation, Folder, Message, Person};
use crate::subject;

pub(crate) fn conversations(
    account: &str,
    me: &str,
    started: &BTreeSet<String>,
    messages: Vec<Message>,
) -> Vec<Conversation> {
    let messages = without_duplicates(messages);
    let mut parent: Vec<usize> = (0..messages.len()).collect();
    let mut owner: HashMap<&str, usize> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        for id in ids(message) {
            match owner.get(id) {
                Some(&other) => union(&mut parent, index, other),
                None => {
                    owner.insert(id, index);
                }
            }
        }
    }
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for index in 0..messages.len() {
        let root = find(&mut parent, index);
        groups.entry(root).or_default().push(index);
    }
    let mut found: Vec<Conversation> = groups
        .into_values()
        .filter(|members| {
            members.iter().any(|&i| {
                messages[i].folder == Folder::Inbox
                    || messages[i]
                        .message_id
                        .as_ref()
                        .is_some_and(|id| started.contains(id))
            })
        })
        .map(|members| {
            let mut thread: Vec<Message> = members.iter().map(|&i| messages[i].clone()).collect();
            thread.sort_by_key(|message| message.date);
            conversation(account, me, thread)
        })
        .collect();
    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.latest()));
    found
}

fn without_duplicates(messages: Vec<Message>) -> Vec<Message> {
    let mut seen = HashSet::new();
    let mut kept: Vec<Message> = Vec::with_capacity(messages.len());
    let mut ordered = messages;
    ordered.sort_by_key(|message| message.folder != Folder::Inbox);
    for message in ordered {
        match &message.message_id {
            Some(id) if !seen.insert(id.clone()) => {}
            _ => kept.push(message),
        }
    }
    kept
}

fn ids(message: &Message) -> impl Iterator<Item = &str> {
    message
        .message_id
        .iter()
        .chain(message.references.iter())
        .chain(message.in_reply_to.iter())
        .map(String::as_str)
        .filter(|id| id.len() > 2)
}

fn find(parent: &mut [usize], index: usize) -> usize {
    let mut root = index;
    while parent[root] != root {
        root = parent[root];
    }
    let mut at = index;
    while parent[at] != root {
        let next = parent[at];
        parent[at] = root;
        at = next;
    }
    root
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let (a, b) = (find(parent, a), find(parent, b));
    if a != b {
        parent[a.max(b)] = a.min(b);
    }
}

fn conversation(account: &str, me: &str, messages: Vec<Message>) -> Conversation {
    let first = &messages[0];
    let anchor = first
        .references
        .first()
        .or(first.in_reply_to.as_ref())
        .or(first.message_id.as_ref())
        .cloned()
        .unwrap_or_else(|| format!("{:?}:{}", first.folder, first.uid));
    let title = messages
        .iter()
        .map(|message| subject::normalize(&message.subject))
        .find(|title| !title.is_empty())
        .unwrap_or_default();
    let mut members: Vec<Person> = Vec::new();
    for message in &messages {
        for person in message.from.iter().chain(&message.to).chain(&message.cc) {
            if person.is(me) {
                continue;
            }
            match members.iter_mut().find(|known| known.is(&person.address)) {
                Some(known) if known.name.is_none() => known.name = person.name.clone(),
                Some(_) => {}
                None => members.push(person.clone()),
            }
        }
    }
    let unread = messages
        .iter()
        .filter(|message| message.folder == Folder::Inbox && !message.seen)
        .count();
    Conversation {
        key: format!("{account}\u{1F}{anchor}"),
        account: account.to_string(),
        title,
        members,
        messages,
        unread,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    pub(crate) fn message(id: &str, refs: &[&str], from: &str, minute: u32) -> Message {
        Message {
            folder: Folder::Inbox,
            uid: minute,
            message_id: Some(format!("<{id}>")),
            in_reply_to: refs.last().map(|r| format!("<{r}>")),
            references: refs.iter().map(|r| format!("<{r}>")).collect(),
            from: Some(Person {
                name: None,
                address: from.to_string(),
            }),
            to: vec![Person {
                name: None,
                address: "me@example.org".into(),
            }],
            cc: Vec::new(),
            subject: if refs.is_empty() {
                "Lunch".into()
            } else {
                "Re: Lunch".into()
            },
            date: Utc.with_ymd_and_hms(2026, 11, 9, 12, minute, 0).unwrap(),
            text: String::new(),
            attachments: Vec::new(),
            seen: false,
            partial: false,
            html: false,
        }
    }

    #[test]
    fn groups_by_references_and_orders_oldest_first() {
        let found = conversations(
            "me@example.org",
            "me@example.org",
            &BTreeSet::new(),
            vec![
                message("c", &["a", "b"], "ana@example.com", 3),
                message("a", &[], "ana@example.com", 1),
                message("x", &[], "ben@example.com", 5),
                message("b", &["a"], "ben@example.com", 2),
            ],
        );
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].messages.len(), 1);
        let lunch = &found[1];
        let order: Vec<_> = lunch.messages.iter().map(|m| m.uid).collect();
        assert_eq!(order, [1, 2, 3]);
        assert_eq!(lunch.title, "Lunch");
        assert_eq!(lunch.members.len(), 2);
        assert_eq!(lunch.unread, 3);
    }

    #[test]
    fn joins_replies_whose_root_never_arrived() {
        let found = conversations(
            "me@example.org",
            "me@example.org",
            &BTreeSet::new(),
            vec![
                message("b", &["root"], "ana@example.com", 2),
                message("c", &["root", "b"], "ben@example.com", 3),
            ],
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].key.ends_with("<root>"));
    }

    #[test]
    fn keeps_one_copy_of_a_message_in_both_folders_and_drops_sent_only_threads() {
        let mut sent = message("a", &[], "me@example.org", 1);
        sent.folder = Folder::Sent;
        let mut lonely = message("z", &[], "me@example.org", 9);
        lonely.folder = Folder::Sent;
        let inbox = message("a", &[], "me@example.org", 1);
        let found = conversations(
            "me@example.org",
            "me@example.org",
            &BTreeSet::new(),
            vec![sent, inbox, lonely],
        );
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].messages.len(), 1);
        assert_eq!(found[0].messages[0].folder, Folder::Inbox);
    }

    #[test]
    fn keeps_a_conversation_started_here_before_anyone_answers() {
        let mut opened = message("new", &[], "me@example.org", 1);
        opened.folder = Folder::Sent;
        let started = BTreeSet::from(["<new>".to_string()]);
        let found = conversations("me@example.org", "me@example.org", &started, vec![opened]);
        assert_eq!(found.len(), 1);
    }
}
