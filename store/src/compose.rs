// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::HashSet;

use lettre::message::header::ContentType;
use lettre::message::{Attachment, Mailbox, Mailboxes, MultiPart, SinglePart};

use crate::model::{Account, Conversation, Draft, Message, Person};
use crate::subject;

pub fn reply(
    conversation: &Conversation,
    account: &Account,
    body: String,
    target: Option<&Message>,
) -> Draft {
    let mut draft = Draft {
        account: account.key(),
        body,
        subject: format!("Re: {}", subject::normalize(&conversation.title))
            .trim()
            .to_string(),
        ..Draft::default()
    };
    let Some(latest) = target.or(conversation.last()) else {
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
    if draft.to.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty() {
        return Err("Add someone to send this to.".into());
    }
    assemble(draft, account, None)
}

pub(crate) fn unsent(draft: &Draft, account: &Account) -> Result<Vec<u8>, String> {
    let me = mailbox(&account.me())?;
    let envelope = lettre::address::Envelope::new(Some(me.email.clone()), vec![me.email])
        .map_err(|error| error.to_string())?;
    Ok(assemble(draft, account, Some(envelope))?.0.formatted())
}

fn assemble(
    draft: &Draft,
    account: &Account,
    envelope: Option<lettre::address::Envelope>,
) -> Result<(lettre::Message, String), String> {
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
    if let Some(envelope) = envelope {
        builder = builder.envelope(envelope);
    }
    if let Some(emoji) = &draft.reaction {
        builder = builder.header(ZirbeReaction(emoji.clone()));
    }
    if let Some(to) = mailboxes(&draft.to)? {
        builder = builder.mailbox(lettre::message::header::To::from(to));
    }
    if let Some(cc) = mailboxes(&draft.cc)? {
        builder = builder.mailbox(lettre::message::header::Cc::from(cc));
    }
    if let Some(bcc) = mailboxes(&draft.bcc)? {
        builder = builder.mailbox(lettre::message::header::Bcc::from(bcc));
    }
    if let Some(parent) = &draft.in_reply_to {
        builder = builder.in_reply_to(parent.clone());
    }
    if !draft.references.is_empty() {
        builder = builder.references(draft.references.join(" "));
    }
    let text = draft.body.replace("\r\n", "\n");
    let built = if draft.attachments.is_empty() {
        builder.header(ContentType::TEXT_PLAIN).body(text)
    } else {
        let mut parts = MultiPart::mixed().singlepart(SinglePart::plain(text));
        for file in &draft.attachments {
            let kind = ContentType::parse(content_type(&file.name))
                .unwrap_or(ContentType::parse("application/octet-stream").expect("a content type"));
            parts =
                parts.singlepart(Attachment::new(file.name.clone()).body(file.bytes.clone(), kind));
        }
        builder.multipart(parts)
    };
    let message = built.map_err(|error| format!("the message could not be built: {error}"))?;
    Ok((message, id))
}

#[derive(Clone)]
struct ZirbeReaction(String);

impl lettre::message::header::Header for ZirbeReaction {
    fn name() -> lettre::message::header::HeaderName {
        lettre::message::header::HeaderName::new_from_ascii_str("X-Zirbe-Reaction")
    }

    fn parse(value: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(ZirbeReaction(value.to_string()))
    }

    fn display(&self) -> lettre::message::header::HeaderValue {
        lettre::message::header::HeaderValue::new(Self::name(), self.0.clone())
    }
}

pub fn reaction(
    conversation: &Conversation,
    account: &Account,
    target: &Message,
    emoji: &str,
) -> Draft {
    let excerpt: String = {
        let text = target.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let base = if text.is_empty() {
            target.subject.trim().to_string()
        } else {
            text
        };
        if base.chars().count() > 40 {
            format!(
                "{}\u{2026}",
                base.chars().take(40).collect::<String>().trim_end()
            )
        } else {
            base
        }
    };
    let body = if excerpt.is_empty() {
        format!("Reacted {emoji}")
    } else {
        format!("Reacted {emoji} to \u{201C}{excerpt}\u{201D}")
    };
    let mut draft = reply(conversation, account, body, None);
    if let Some(id) = &target.message_id {
        draft.references = target.references.clone();
        if draft.references.last() != Some(id) {
            draft.references.push(id.clone());
        }
        draft.in_reply_to = Some(id.clone());
    }
    draft.reaction = Some(emoji.to_string());
    draft
}

fn content_type(name: &str) -> &'static str {
    let extension = name
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_ascii_lowercase())
        .unwrap_or_default();
    match extension.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "txt" | "text" => "text/plain",
        "csv" => "text/csv",
        "htm" | "html" => "text/html",
        "ics" => "text/calendar",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "mp4" => "video/mp4",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Outgoing;
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
            in_inbox: true,
            flagged: false,
            pinned: false,
            copies: Vec::new(),
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
        let draft = reply(&conversation(vec![latest]), &account(), "Yes".into(), None);
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
        let draft = reply(&conversation(vec![mine]), &account(), String::new(), None);
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

    #[test]
    fn keeps_bcc_out_of_the_headers_and_attaches_files() {
        let draft = Draft {
            account: "me@example.org".into(),
            to: vec![Person {
                name: None,
                address: "ana@example.com".into(),
            }],
            bcc: vec![Person {
                name: None,
                address: "secret@example.com".into(),
            }],
            subject: "Files".into(),
            body: "Here.".into(),
            attachments: vec![Outgoing {
                name: "notes.txt".into(),
                bytes: b"first".to_vec(),
            }],
            ..Draft::default()
        };
        let (message, _) = build(&draft, &account()).expect("a message");
        let envelope: Vec<String> = message
            .envelope()
            .to()
            .iter()
            .map(ToString::to_string)
            .collect();
        assert!(envelope.contains(&"secret@example.com".to_string()));
        let raw = String::from_utf8(message.formatted()).expect("text");
        assert!(!raw.to_ascii_lowercase().contains("bcc:"), "{raw}");
        assert!(raw.contains("multipart/mixed"));
        assert!(raw.contains("filename=\"notes.txt\""), "{raw}");
    }

    #[test]
    fn a_reaction_round_trips_through_its_header() {
        let mut target = message("a", &[], "ana@example.com", 1);
        target.text = "Lunch at noon?".into();
        let thread = conversation(vec![target.clone()]);
        let draft = super::reaction(&thread, &account(), &target, "👍");
        assert_eq!(draft.body, "Reacted 👍 to “Lunch at noon?”");
        assert_eq!(draft.in_reply_to.as_deref(), Some("<a>"));
        let (message, _) = build(&draft, &account()).expect("a message");
        let raw = message.formatted();
        let parsed = mail_parser::MessageParser::default()
            .parse(&raw)
            .expect("parsed");
        assert_eq!(crate::content::reaction(&parsed).as_deref(), Some("👍"));
    }
}
