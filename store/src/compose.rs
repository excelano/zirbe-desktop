// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::HashSet;

use lettre::message::header::ContentType;
use lettre::message::{Mailbox, Mailboxes};

use crate::model::{Account, Conversation, Draft, Person};
use crate::subject;

pub fn reply(conversation: &Conversation, account: &Account, body: String) -> Draft {
    let mut draft = Draft {
        account: account.key(),
        body,
        subject: format!("Re: {}", subject::normalize(&conversation.title))
            .trim()
            .to_string(),
        ..Draft::default()
    };
    let Some(latest) = conversation.last() else {
        return draft;
    };
    let mut placed: HashSet<String> = HashSet::from([account.key()]);
    let from_me = latest
        .from
        .as_ref()
        .is_some_and(|from| from.is(&account.address));
    let first = if from_me { None } else { latest.from.as_ref() };
    for person in first.into_iter().chain(&latest.to) {
        if placed.insert(person.address.to_lowercase()) {
            draft.to.push(person.clone());
        }
    }
    for person in &latest.cc {
        if placed.insert(person.address.to_lowercase()) {
            draft.cc.push(person.clone());
        }
    }
    if draft.to.is_empty() && draft.cc.is_empty() {
        draft.to.push(account.me());
    }
    draft.references = latest.references.clone();
    if let Some(id) = &latest.message_id {
        if draft.references.last() != Some(id) {
            draft.references.push(id.clone());
        }
        draft.in_reply_to = Some(id.clone());
    }
    draft
}

pub fn parse_people(text: &str) -> Result<Vec<Person>, String> {
    text.split([',', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mailbox: Mailbox = part
                .parse()
                .map_err(|_| format!("“{part}” is not an email address"))?;
            Ok(person(&mailbox))
        })
        .collect()
}

fn person(mailbox: &Mailbox) -> Person {
    Person {
        name: mailbox.name.clone(),
        address: mailbox.email.to_string(),
    }
}

fn mailbox(person: &Person) -> Result<Mailbox, String> {
    let address = person
        .address
        .parse()
        .map_err(|_| format!("“{}” is not an email address", person.address))?;
    Ok(Mailbox::new(person.name.clone(), address))
}

fn mailboxes(people: &[Person]) -> Result<Option<Mailboxes>, String> {
    if people.is_empty() {
        return Ok(None);
    }
    let mut list = Mailboxes::new();
    for person in people {
        list.push(mailbox(person)?);
    }
    Ok(Some(list))
}

pub(crate) fn build(draft: &Draft, account: &Account) -> Result<(lettre::Message, String), String> {
    if draft.to.is_empty() && draft.cc.is_empty() {
        return Err("Add someone to send this to.".into());
    }
    let domain = account
        .key()
        .rsplit_once('@')
        .map_or("localhost".into(), |(_, d)| d.to_string());
    let id = format!("<{}@{domain}>", uuid::Uuid::new_v4());
    let mut builder = lettre::Message::builder()
        .from(mailbox(&account.me())?)
        .subject(draft.subject.trim())
        .message_id(Some(id.clone()))
        .date_now();
    if let Some(to) = mailboxes(&draft.to)? {
        builder = builder.mailbox(lettre::message::header::To::from(to));
    }
    if let Some(cc) = mailboxes(&draft.cc)? {
        builder = builder.mailbox(lettre::message::header::Cc::from(cc));
    }
    if let Some(parent) = &draft.in_reply_to {
        builder = builder.in_reply_to(parent.clone());
    }
    if !draft.references.is_empty() {
        builder = builder.references(draft.references.join(" "));
    }
    let message = builder
        .header(ContentType::TEXT_PLAIN)
        .body(draft.body.replace("\r\n", "\n"))
        .map_err(|error| format!("the message could not be built: {error}"))?;
    Ok((message, id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::thread::tests::message;

    fn account() -> Account {
        Account {
            address: "me@example.org".into(),
            name: "Me Myself".into(),
            imap_host: "imap.example.org".into(),
            imap_port: 993,
            smtp_host: "smtp.example.org".into(),
            smtp_port: 587,
        }
    }

    fn conversation(messages: Vec<crate::model::Message>) -> Conversation {
        Conversation {
            key: "k".into(),
            account: "me@example.org".into(),
            title: "Lunch".into(),
            members: Vec::new(),
            messages,
            unread: 0,
        }
    }

    #[test]
    fn replies_to_everyone_but_me_and_threads_under_the_latest() {
        let mut latest = message("b", &["a"], "ana@example.com", 2);
        latest.to.push(Person {
            name: None,
            address: "ben@example.com".into(),
        });
        latest.cc.push(Person {
            name: None,
            address: "ME@example.org".into(),
        });
        latest.cc.push(Person {
            name: None,
            address: "cy@example.com".into(),
        });
        let draft = reply(&conversation(vec![latest]), &account(), "Yes".into());
        let to: Vec<_> = draft.to.iter().map(|p| p.address.as_str()).collect();
        let cc: Vec<_> = draft.cc.iter().map(|p| p.address.as_str()).collect();
        assert_eq!(to, ["ana@example.com", "ben@example.com"]);
        assert_eq!(cc, ["cy@example.com"]);
        assert_eq!(draft.in_reply_to.as_deref(), Some("<b>"));
        assert_eq!(draft.references, ["<a>", "<b>"]);
        assert_eq!(draft.subject, "Re: Lunch");
    }

    #[test]
    fn a_reply_to_my_own_message_goes_to_its_recipients() {
        let mine = message("a", &[], "me@example.org", 1);
        let mut mine = mine;
        mine.to = vec![Person {
            name: None,
            address: "ana@example.com".into(),
        }];
        let draft = reply(&conversation(vec![mine]), &account(), String::new());
        assert_eq!(draft.to.len(), 1);
        assert_eq!(draft.to[0].address, "ana@example.com");
    }

    #[test]
    fn builds_a_plain_text_message_with_threading_headers() {
        let draft = Draft {
            account: "me@example.org".into(),
            to: parse_people("Ana Lopez <ana@example.com>, ben@example.com").unwrap(),
            subject: "Re: Lunch".into(),
            body: "Noon.".into(),
            in_reply_to: Some("<b@x>".into()),
            references: vec!["<a@x>".into(), "<b@x>".into()],
            ..Draft::default()
        };
        let (message, id) = build(&draft, &account()).unwrap();
        let text = String::from_utf8(message.formatted()).unwrap();
        assert!(id.ends_with("@example.org>"));
        assert!(text.contains("In-Reply-To: <b@x>"));
        assert!(text.contains("References: <a@x> <b@x>"));
        assert!(text.contains("From: \"Me Myself\" <me@example.org>"));
        assert!(text.contains("To: \"Ana Lopez\" <ana@example.com>, ben@example.com"));
    }

    #[test]
    fn refuses_a_message_to_nobody_and_a_bad_address() {
        assert!(build(&Draft::default(), &account()).is_err());
        assert!(parse_people("ana@example.com, not an address").is_err());
    }
}
