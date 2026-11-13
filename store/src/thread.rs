// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::{BTreeSet, HashMap, HashSet};

use crate::model::{Account, Conversation, Copy, Message, Person};
use crate::subject;

pub(crate) fn conversations(
    me: &Account,
    started: &BTreeSet<String>,
    messages: Vec<Message>,
) -> Vec<Conversation> {
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
        .flat_map(|members| {
            let copies: Vec<&Message> = members.iter().map(|&i| &messages[i]).collect();
            let mut thread = without_duplicates(&copies);
            thread.sort_by_key(|message| message.date);
            branches(me, started, thread, &copies)
        })
        .collect();
    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.latest()));
    found
}

#[derive(Clone, Copy, PartialEq)]
enum Branch {
    Main,
    From(usize),
}

fn branches(
    me: &Account,
    started: &BTreeSet<String>,
    thread: Vec<Message>,
    copies: &[&Message],
) -> Vec<Conversation> {
    let position: HashMap<&str, usize> = thread
        .iter()
        .enumerate()
        .filter_map(|(index, message)| Some((message.message_id.as_deref()?, index)))
        .collect();
    let parents: Vec<Option<usize>> = thread
        .iter()
        .enumerate()
        .map(|(index, message)| {
            message
                .in_reply_to
                .iter()
                .chain(message.references.iter().rev())
                .filter_map(|id| position.get(id.as_str()).copied())
                .find(|&parent| parent != index)
        })
        .collect();
    let mut resolved: Vec<Option<Branch>> = vec![None; thread.len()];
    for index in 0..thread.len() {
        resolve(index, &thread, &parents, me, &mut resolved, &mut Vec::new());
    }
    let anchor_message = (0..thread.len())
        .find(|&index| resolved[index] == Some(Branch::Main))
        .map_or(&thread[0], |index| &thread[index]);
    let anchor = anchor_message
        .references
        .first()
        .or(anchor_message.in_reply_to.as_ref())
        .or(anchor_message.message_id.as_ref())
        .cloned()
        .unwrap_or_else(|| format!("{}:{}", anchor_message.mailbox, anchor_message.uid));
    let main_key = format!("{}\u{1F}{anchor}", me.key());
    let mut order: Vec<Branch> = Vec::new();
    for branch in resolved.iter().flatten() {
        if !order.contains(branch) {
            order.push(*branch);
        }
    }
    order
        .into_iter()
        .map(|branch| {
            let own: Vec<usize> = (0..thread.len())
                .filter(|&index| resolved[index] == Some(branch))
                .collect();
            let held: Vec<&Message> = copies
                .iter()
                .copied()
                .filter(|copy| {
                    own.iter().any(|&index| {
                        let message = &thread[index];
                        match (&copy.message_id, &message.message_id) {
                            (Some(a), Some(b)) => a == b,
                            (None, None) => {
                                copy.mailbox == message.mailbox && copy.uid == message.uid
                            }
                            _ => false,
                        }
                    })
                })
                .collect();
            let in_inbox = held.iter().any(|message| {
                message.in_inbox()
                    || message
                        .message_id
                        .as_ref()
                        .is_some_and(|id| started.contains(id))
            });
            let (key, context) = match branch {
                Branch::Main => (main_key.clone(), None),
                Branch::From(start) => {
                    let id = thread[start].message_id.clone().unwrap_or_default();
                    (
                        format!("{main_key}\u{1F}{id}"),
                        parents[start].map(|parent| thread[parent].clone()),
                    )
                }
            };
            let mut found = conversation(
                key,
                me,
                context,
                own.iter().map(|&index| thread[index].clone()).collect(),
                in_inbox,
            );
            found.flagged = held.iter().any(|message| message.flagged);
            found.copies = held.into_iter().map(Copy::of).collect();
            found
        })
        .collect()
}

fn resolve(
    index: usize,
    thread: &[Message],
    parents: &[Option<usize>],
    me: &Account,
    resolved: &mut [Option<Branch>],
    visiting: &mut Vec<usize>,
) -> Branch {
    if let Some(branch) = resolved[index] {
        return branch;
    }
    let branch = match parents[index] {
        Some(parent) if !visiting.contains(&parent) => {
            if splits_off(&thread[index], &thread[parent], me) {
                Branch::From(index)
            } else {
                visiting.push(index);
                let branch = resolve(parent, thread, parents, me, resolved, visiting);
                visiting.pop();
                branch
            }
        }
        _ => Branch::Main,
    };
    resolved[index] = Some(branch);
    branch
}

fn splits_off(reply: &Message, parent: &Message, me: &Account) -> bool {
    if reply.reaction.is_some() || reply.invite.is_some() {
        return false;
    }
    let reply = audience(reply, me);
    let parent = audience(parent, me);
    !reply.is_empty() && (reply.is_disjoint(&parent) || (reply.len() == 1 && parent.len() > 1))
}

fn audience(message: &Message, me: &Account) -> BTreeSet<String> {
    message
        .from
        .iter()
        .chain(&message.to)
        .chain(&message.cc)
        .filter(|person| !me.owns(&person.address))
        .map(|person| person.address.to_lowercase())
        .collect()
}

fn without_duplicates(copies: &[&Message]) -> Vec<Message> {
    let mut seen = HashSet::new();
    let mut ordered: Vec<&Message> = copies.to_vec();
    ordered.sort_by_key(|message| !message.in_inbox());
    ordered
        .into_iter()
        .filter(|message| {
            message
                .message_id
                .as_ref()
                .is_none_or(|id| seen.insert(id.clone()))
        })
        .cloned()
        .collect()
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

fn conversation(
    key: String,
    me: &Account,
    context: Option<Message>,
    own: Vec<Message>,
    in_inbox: bool,
) -> Conversation {
    let title = own
        .iter()
        .chain(&context)
        .map(|message| subject::normalize(&message.subject))
        .find(|title| !title.is_empty())
        .unwrap_or_default();
    let mut members: Vec<Person> = Vec::new();
    for message in &own {
        for person in message.from.iter().chain(&message.to).chain(&message.cc) {
            if me.owns(&person.address) {
                continue;
            }
            match members.iter_mut().find(|known| known.is(&person.address)) {
                Some(known) if known.name.is_none() => known.name = person.name.clone(),
                Some(_) => {}
                None => members.push(person.clone()),
            }
        }
    }
    let unread = own.iter().filter(|message| !message.seen).count();
    let starts_with_context = context.is_some();
    Conversation {
        key,
        account: me.key(),
        title,
        members,
        messages: context.into_iter().chain(own).collect(),
        starts_with_context,
        unread,
        in_inbox,
        flagged: false,
        pinned: false,
        copies: Vec::new(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::Scope;
    use chrono::{TimeZone, Utc};

    pub(crate) const ME: &str = "me@example.org";

    pub(crate) fn account() -> Account {
        Account {
            address: ME.into(),
            ..Account::default()
        }
    }

    pub(crate) fn message(id: &str, refs: &[&str], from: &str, minute: u32) -> Message {
        Message {
            mailbox: "INBOX".into(),
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
            quoted: false,
            attachments: Vec::new(),
            seen: false,
            partial: false,
            html: false,
            list: false,
            flagged: false,
            reaction: None,
            invite: None,
        }
    }

    #[test]
    fn groups_by_references_and_orders_oldest_first() {
        let found = conversations(
            &account(),
            &BTreeSet::new(),
            vec![
                addressed(
                    message("c", &["a", "b"], "ana@example.com", 3),
                    &[ME, "ben@example.com"],
                    &[],
                ),
                addressed(
                    message("a", &[], "ana@example.com", 1),
                    &[ME, "ben@example.com"],
                    &[],
                ),
                message("x", &[], "ben@example.com", 5),
                addressed(
                    message("b", &["a"], "ben@example.com", 2),
                    &[ME, "ana@example.com"],
                    &[],
                ),
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
            &account(),
            &BTreeSet::new(),
            vec![
                message("b", &["root"], "ana@example.com", 2),
                addressed(
                    message("c", &["root", "b"], "ben@example.com", 3),
                    &[ME, "ana@example.com"],
                    &[],
                ),
            ],
        );
        assert_eq!(found.len(), 1);
        assert!(found[0].key.ends_with("<root>"));
    }

    #[test]
    fn keeps_one_copy_of_a_message_in_both_folders_and_keeps_sent_only_threads_out_of_the_inbox() {
        let mut sent = message("a", &[], "me@example.org", 1);
        sent.mailbox = "Sent".into();
        let mut lonely = message("z", &[], "me@example.org", 9);
        lonely.mailbox = "Sent".into();
        let inbox = message("a", &[], "me@example.org", 1);
        let found = conversations(&account(), &BTreeSet::new(), vec![sent, inbox, lonely]);
        assert_eq!(found.len(), 2);
        let lunch = found.iter().find(|c| c.in_inbox).expect("the inbox thread");
        assert_eq!(lunch.messages.len(), 1);
        assert!(lunch.messages[0].in_inbox());
        let sent_only = found
            .iter()
            .find(|c| !c.in_inbox)
            .expect("the sent-only thread");
        assert!(
            Scope::Mailbox {
                account: "me@example.org".into(),
                name: "Sent".into()
            }
            .holds(sent_only)
        );
        assert!(!Scope::AllInboxes.holds(sent_only));
    }

    #[test]
    fn keeps_a_conversation_started_here_in_the_inbox_before_anyone_answers() {
        let mut opened = message("new", &[], "me@example.org", 1);
        opened.mailbox = "Sent".into();
        let started = BTreeSet::from(["<new>".to_string()]);
        let found = conversations(&account(), &started, vec![opened]);
        assert!(found[0].in_inbox);
    }

    #[test]
    fn a_message_filed_twice_shows_once_but_belongs_to_both_folders() {
        let mut sent = message("a", &[], "me@example.org", 1);
        sent.mailbox = "INBOX.Sent".into();
        let mut archived = message("a", &[], "me@example.org", 1);
        archived.mailbox = "INBOX.Archive".into();
        let found = conversations(&account(), &BTreeSet::new(), vec![sent, archived]);
        assert_eq!(found[0].messages.len(), 1);
        let archive = Scope::Mailbox {
            account: "me@example.org".into(),
            name: "INBOX.Archive".into(),
        };
        assert!(archive.holds(&found[0]));
        assert!(!found[0].in_inbox);
    }

    fn addressed(mut message: Message, to: &[&str], cc: &[&str]) -> Message {
        let people = |addresses: &[&str]| {
            addresses
                .iter()
                .map(|address| Person {
                    name: None,
                    address: address.to_string(),
                })
                .collect()
        };
        message.to = people(to);
        message.cc = people(cc);
        message
    }

    fn keys_and_uids(found: &[Conversation]) -> Vec<Vec<u32>> {
        let mut shapes: Vec<Vec<u32>> = found
            .iter()
            .map(|conversation| conversation.messages.iter().map(|m| m.uid).collect())
            .collect();
        shapes.sort();
        shapes
    }

    #[test]
    fn private_replies_to_a_blind_copied_message_become_their_own_conversations() {
        let found = conversations(
            &account(),
            &BTreeSet::new(),
            vec![
                addressed(message("root", &[], "relay@example.net", 1), &[ME], &[]),
                message("peter", &["root"], "peter@example.com", 2),
                message("joel", &["root"], "joel@example.com", 3),
                addressed(
                    message("to-joel", &["root", "joel"], ME, 4),
                    &["joel@example.com"],
                    &[],
                ),
                addressed(
                    message("to-peter", &["root", "peter"], ME, 5),
                    &["peter@example.com"],
                    &[],
                ),
            ],
        );
        assert_eq!(
            keys_and_uids(&found),
            [vec![1], vec![1, 2, 5], vec![1, 3, 4]]
        );
        let peter = found
            .iter()
            .find(|c| c.messages.iter().any(|m| m.uid == 2))
            .expect("Peter's conversation");
        assert!(peter.starts_with_context);
        assert_eq!(peter.unread, 2);
        assert_eq!(peter.members.len(), 1);
        assert!(peter.members[0].is("peter@example.com"));
        assert!(peter.copies.iter().all(|copy| copy.uid != 1));
        let root = found
            .iter()
            .find(|c| c.messages.len() == 1)
            .expect("the original");
        assert!(!root.starts_with_context);
        assert!(peter.key.starts_with(&root.key));
    }

    #[test]
    fn replies_to_everyone_stay_one_conversation_even_when_they_cross() {
        let group = ["ana@example.com", "ben@example.com"];
        let found = conversations(
            &account(),
            &BTreeSet::new(),
            vec![
                addressed(
                    message("a", &[], "ana@example.com", 1),
                    &[ME, group[1]],
                    &[],
                ),
                addressed(
                    message("b", &["a"], "ben@example.com", 2),
                    &[ME, group[0]],
                    &[],
                ),
                addressed(message("c", &["a"], ME, 3), &group, &[]),
                addressed(
                    message("d", &["a", "b"], "ana@example.com", 4),
                    &[ME, group[1]],
                    &["cy@example.com"],
                ),
                addressed(
                    message("e", &["a", "b", "d"], "cy@example.com", 5),
                    &[ME],
                    &["ana@example.com"],
                ),
            ],
        );
        assert_eq!(keys_and_uids(&found), [vec![1, 2, 3, 4, 5]]);
    }

    #[test]
    fn a_private_reply_from_a_group_starts_a_side_conversation() {
        let found = conversations(
            &account(),
            &BTreeSet::new(),
            vec![
                addressed(
                    message("a", &[], "ana@example.com", 1),
                    &[ME, "ben@example.com", "cy@example.com"],
                    &[],
                ),
                addressed(
                    message("b", &["a"], "ben@example.com", 2),
                    &[ME, "ana@example.com", "cy@example.com"],
                    &[],
                ),
                message("c", &["a"], "cy@example.com", 3),
                addressed(message("d", &["a", "c"], ME, 4), &["cy@example.com"], &[]),
            ],
        );
        assert_eq!(keys_and_uids(&found), [vec![1, 2], vec![1, 3, 4]]);
    }

    #[test]
    fn a_reaction_stays_with_the_message_it_answers() {
        let mut reaction = message("r", &["root"], "peter@example.com", 2);
        reaction.reaction = Some("👍".into());
        let found = conversations(
            &account(),
            &BTreeSet::new(),
            vec![
                addressed(message("root", &[], "relay@example.net", 1), &[ME], &[]),
                reaction,
            ],
        );
        assert_eq!(keys_and_uids(&found), [vec![1, 2]]);
    }

    #[test]
    fn an_answer_to_a_meeting_stays_with_the_invitation() {
        let mut answer = message("yes", &["invite"], "ana@example.com", 2);
        answer.invite = Some(crate::model::Invite {
            method: "REPLY".into(),
            ..Default::default()
        });
        let found = conversations(
            &account(),
            &BTreeSet::new(),
            vec![
                addressed(
                    message("invite", &[], ME, 1),
                    &["ana@example.com", "ben@example.com"],
                    &[],
                ),
                answer,
            ],
        );
        assert_eq!(keys_and_uids(&found), [vec![1, 2]]);
    }

    #[test]
    fn an_address_of_mine_is_not_someone_a_reply_can_leave_out() {
        let shared = "hello@example.org";
        let thread = vec![
            addressed(message("a", &[], "ilyas@example.com", 1), &[shared], &[]),
            addressed(
                message("b", &["a"], ME, 2),
                &["ilyas@example.com"],
                &[shared],
            ),
            message("c", &["a", "b"], "ilyas@example.com", 3),
        ];
        let mine = Account {
            aliases: vec![shared.into()],
            ..account()
        };
        assert_eq!(
            keys_and_uids(&conversations(&mine, &BTreeSet::new(), thread.clone())),
            [vec![1, 2, 3]]
        );
        assert_eq!(
            keys_and_uids(&conversations(&account(), &BTreeSet::new(), thread)),
            [vec![1, 2], vec![2, 3]]
        );
    }
}
