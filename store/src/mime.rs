// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use chrono::{DateTime, TimeZone, Utc};
use mail_parser::{AddressList, MessageParser};

use crate::content;
use crate::model::{Folder, Message, Person};

pub(crate) struct Fetched<'a> {
    pub folder: Folder,
    pub uid: u32,
    pub seen: bool,
    pub header: &'a [u8],
    pub body: Body<'a>,
    pub arrived: Option<DateTime<Utc>>,
}

pub(crate) enum Body<'a> {
    Whole,
    FirstPart {
        mime: Option<&'a [u8]>,
        data: &'a [u8],
    },
}

pub(crate) fn parse(fetched: &Fetched<'_>) -> Option<Message> {
    let parser = MessageParser::default();
    let headers = parser.parse(fetched.header)?;
    let (text, attachments, partial, html) = match &fetched.body {
        Body::Whole => (
            content::text_of(&headers),
            content::attachment_names(&headers),
            false,
            content::has_html(&headers),
        ),
        Body::FirstPart { mime, data } => {
            let mut raw = mime.unwrap_or(b"Content-Type: text/plain\r\n\r\n").to_vec();
            raw.extend_from_slice(data);
            let text = parser
                .parse(&raw)
                .map(|part| content::text_of(&part))
                .unwrap_or_default();
            (text, Vec::new(), true, true)
        }
    };
    let date = headers
        .date()
        .and_then(|date| Utc.timestamp_opt(date.to_timestamp(), 0).single())
        .or(fetched.arrived)
        .unwrap_or_else(Utc::now);
    Some(Message {
        folder: fetched.folder,
        uid: fetched.uid,
        message_id: headers.message_id().map(bracketed),
        in_reply_to: headers
            .in_reply_to()
            .and_then(|list| list.last().map(bracketed)),
        references: headers
            .references()
            .map(|list| list.iter().map(bracketed).collect())
            .unwrap_or_default(),
        from: headers
            .from()
            .and_then(|list| people(list).into_iter().next()),
        to: headers.to().map(people).unwrap_or_default(),
        cc: headers.cc().map(people).unwrap_or_default(),
        subject: headers.subject().unwrap_or_default().trim().to_string(),
        date,
        text: content::reply_part(&text),
        attachments,
        seen: fetched.seen,
        partial,
        html,
    })
}

fn bracketed(id: &str) -> String {
    format!(
        "<{}>",
        id.trim().trim_start_matches('<').trim_end_matches('>')
    )
}

fn people(list: AddressList<'_>) -> Vec<Person> {
    list.mailboxes()
        .filter_map(|mailbox| {
            let address = mailbox.address()?.trim().to_string();
            Some(Person {
                name: mailbox
                    .name()
                    .map(|name| name.trim().trim_matches('"').trim().to_string())
                    .filter(|name| !name.is_empty() && !name.eq_ignore_ascii_case(&address)),
                address,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &[u8] = b"Message-ID: <b@example.com>\r\n\
In-Reply-To: <a@example.com>\r\n\
References: <root@example.com> <a@example.com>\r\n\
From: \"Ana Lopez\" <ana@example.com>\r\n\
To: me@example.org, Ben <ben@example.net>\r\n\
Subject: Re: Lunch\r\n\
Date: Mon, 09 Nov 2026 10:00:00 -0600\r\n\
Content-Type: multipart/alternative; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Noon works.\r\n\
\r\n\
On Mon, Nov 9, 2026, Ben wrote:\r\n\
> Lunch?\r\n\
--b\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Noon works.</p>\r\n\
--b--\r\n";

    fn whole(raw: &[u8]) -> Message {
        parse(&Fetched {
            folder: Folder::Inbox,
            uid: 7,
            seen: false,
            header: raw,
            body: Body::Whole,
            arrived: None,
        })
        .expect("a message")
    }

    #[test]
    fn reads_threading_headers_people_and_the_reply_text() {
        let message = whole(REPLY);
        assert_eq!(message.message_id.as_deref(), Some("<b@example.com>"));
        assert_eq!(message.in_reply_to.as_deref(), Some("<a@example.com>"));
        assert_eq!(
            message.references,
            ["<root@example.com>", "<a@example.com>"]
        );
        let from = message.from.expect("a sender");
        assert_eq!(from.name.as_deref(), Some("Ana Lopez"));
        assert_eq!(message.to.len(), 2);
        assert_eq!(message.to[0].name, None);
        assert_eq!(message.text, "Noon works.");
        assert_eq!(message.date.to_rfc3339(), "2026-11-09T16:00:00+00:00");
    }

    #[test]
    fn falls_back_to_html_when_there_is_no_plain_part() {
        let raw = b"From: a@example.com\r\nSubject: Hi\r\nContent-Type: text/html\r\n\r\n<p>Hello <b>there</b></p>";
        assert_eq!(whole(raw).text, "Hello there");
    }

    #[test]
    fn reads_the_first_part_of_a_large_message() {
        let header = b"From: a@example.com\r\nSubject: Photos\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n";
        let message = parse(&Fetched {
            folder: Folder::Inbox,
            uid: 1,
            seen: true,
            header,
            body: Body::FirstPart {
                mime: Some(b"Content-Type: text/plain; charset=utf-8\r\n\r\n"),
                data: b"Here they are.",
            },
            arrived: None,
        })
        .expect("a message");
        assert_eq!(message.text, "Here they are.");
        assert!(message.partial);
    }
}
