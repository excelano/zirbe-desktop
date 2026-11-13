// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use async_imap::imap_proto::{BodyParams, BodyStructure};
use chrono::{DateTime, TimeZone, Utc};
use mail_parser::{AddressList, HeaderName, MessageParser};

use crate::content;
use crate::model::{Message, Person};

pub(crate) struct Fetched<'a> {
    pub mailbox: &'a str,
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
        attachments: Vec<String>,
        html: bool,
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
        Body::FirstPart {
            mime,
            data,
            attachments,
            html,
        } => {
            let mut raw = mime.unwrap_or(b"Content-Type: text/plain\r\n\r\n").to_vec();
            raw.extend_from_slice(data);
            let text = parser
                .parse(&raw)
                .map(|part| content::text_of(&part))
                .unwrap_or_default();
            (text, attachments.clone(), true, *html)
        }
    };
    let date = headers
        .date()
        .and_then(|date| Utc.timestamp_opt(date.to_timestamp(), 0).single())
        .or(fetched.arrived)
        .unwrap_or_else(Utc::now);
    Some(Message {
        mailbox: fetched.mailbox.to_string(),
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
        quoted: content::folds_quote(&text),
        text: content::reply_part(&text),
        attachments,
        seen: fetched.seen,
        partial,
        html,
        list: headers.headers().iter().any(|header| {
            matches!(
                header.name(),
                HeaderName::ListId | HeaderName::ListUnsubscribe
            )
        }),
        flagged: false,
        reaction: content::reaction(&headers),
        invite: content::invite(&headers),
    })
}

pub(crate) fn has_html(structure: &BodyStructure<'_>) -> bool {
    match structure {
        BodyStructure::Multipart { bodies, .. } => bodies.iter().any(has_html),
        BodyStructure::Text { common, .. } => {
            common.ty.subtype.eq_ignore_ascii_case("html")
                && !common
                    .disposition
                    .as_ref()
                    .is_some_and(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"))
        }
        BodyStructure::Basic { .. } | BodyStructure::Message { .. } => false,
    }
}

pub(crate) fn attachment_names(structure: &BodyStructure<'_>) -> Vec<String> {
    let common = match structure {
        BodyStructure::Multipart { bodies, .. } => {
            return bodies.iter().flat_map(attachment_names).collect();
        }
        BodyStructure::Basic { common, .. }
        | BodyStructure::Text { common, .. }
        | BodyStructure::Message { common, .. } => common,
    };
    let disposition = common.disposition.as_ref();
    let attached =
        disposition.is_some_and(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"));
    let name = disposition
        .and_then(|disposition| parameter(&disposition.params, "filename"))
        .or_else(|| parameter(&common.ty.params, "name"));
    if attached || name.is_some() {
        vec![name.unwrap_or_else(|| "attachment".into())]
    } else {
        Vec::new()
    }
}

fn parameter(params: &BodyParams<'_>, key: &str) -> Option<String> {
    let params = params.as_deref()?;
    if let Some((_, value)) = params
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
    {
        let header = format!("Subject: {value}\r\n\r\n");
        return MessageParser::default()
            .parse(header.as_bytes())
            .and_then(|parsed| parsed.subject().map(str::to_string))
            .or_else(|| Some(value.to_string()));
    }
    let extended = format!("{key}*");
    let (_, value) = params
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(&extended))?;
    let encoded = value.splitn(3, '\'').nth(2).unwrap_or(value);
    Some(percent_decoded(encoded))
}

fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let hex = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|digits| u8::from_str_radix(digits, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
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
            mailbox: "INBOX",
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
            mailbox: "INBOX",
            uid: 1,
            seen: true,
            header,
            body: Body::FirstPart {
                mime: Some(b"Content-Type: text/plain; charset=utf-8\r\n\r\n"),
                data: b"Here they are.",
                attachments: vec!["beach.jpg".into()],
                html: false,
            },
            arrived: None,
        })
        .expect("a message");
        assert_eq!(message.text, "Here they are.");
        assert_eq!(message.attachments, ["beach.jpg"]);
        assert!(message.partial);
        assert!(!message.html);
    }

    #[test]
    fn marks_mailing_list_mail() {
        let listed = b"From: news@example.com\r\nList-Unsubscribe: <mailto:leave@example.com>\r\nSubject: News\r\n\r\nHello";
        let personal = b"From: a@example.com\r\nSubject: Hi\r\n\r\nHello";
        assert!(whole(listed).list);
        assert!(!whole(personal).list);
    }

    #[test]
    fn decodes_attachment_names_from_the_body_structure() {
        use std::borrow::Cow;
        let encoded: BodyParams<'_> = Some(vec![(
            Cow::Borrowed("name"),
            Cow::Borrowed("=?UTF-8?Q?Gr=C3=BC=C3=9Fe.pdf?="),
        )]);
        assert_eq!(parameter(&encoded, "name").as_deref(), Some("Grüße.pdf"));
        let extended: BodyParams<'_> = Some(vec![(
            Cow::Borrowed("FILENAME*"),
            Cow::Borrowed("utf-8''Gr%C3%BC%C3%9Fe.pdf"),
        )]);
        assert_eq!(
            parameter(&extended, "filename").as_deref(),
            Some("Grüße.pdf")
        );
    }

    #[test]
    fn decodes_a_japanese_encoded_subject() {
        let raw = b"From: a@example.com\r\nSubject: =?ISO-2022-JP?B?WmlyYmUgGyRCIT0bKEIgRW1haWwgYXMgQ2hhdA==?=\r\n\r\nHi";
        assert_eq!(whole(raw).subject, "Zirbe \u{2015} Email as Chat");
    }
}
