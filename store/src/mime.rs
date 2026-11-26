// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use async_imap::imap_proto::{
    BodyContentCommon, BodyContentSinglePart, BodyParams, BodyStructure, ContentEncoding,
};
use chrono::{DateTime, TimeZone, Utc};
use mail_parser::{AddressList, HeaderName, MessageParser};

use crate::content;
use crate::kinds::{self, Part};
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
        parts: Vec<Part>,
        encrypted: bool,
        notice: bool,
    },
}

pub(crate) fn parse(fetched: &Fetched<'_>) -> Option<Message> {
    let parser = MessageParser::default();
    let headers = parser.parse(fetched.header)?;
    let (text, attachments, partial, html, facts) = match &fetched.body {
        Body::Whole => {
            let text = content::text_of(&headers);
            let facts = content::facts(&headers, &text);
            (
                text,
                content::attachment_names(&headers),
                false,
                facts.html,
                facts,
            )
        }
        Body::FirstPart {
            mime,
            data,
            attachments,
            html,
            parts,
            encrypted,
            notice,
        } => {
            let mut raw = mime.unwrap_or(b"Content-Type: text/plain\r\n\r\n").to_vec();
            raw.extend_from_slice(data);
            let text = parser
                .parse(&raw)
                .map(|part| content::text_of(&part))
                .unwrap_or_default();
            let mut facts = content::facts(&headers, &text);
            facts.html = *html;
            facts.encrypted |= *encrypted;
            facts.notice |= *notice;
            let (reactions, others): (Vec<Part>, Vec<Part>) = parts
                .iter()
                .cloned()
                .partition(|part| part.mime == content::REACTION_MIME);
            facts.reaction |= !reactions.is_empty();
            facts.parts = others;
            (text, attachments.clone(), true, *html, facts)
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
        transcript: None,
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
        mailing_list: content::mailing_list(&headers),
        flagged: false,
        reaction: content::reaction(&headers),
        invite: content::invite(&headers),
        waiting: false,
        kinds: kinds::classify(&facts),
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

fn common_of<'s, 'a>(structure: &'s BodyStructure<'a>) -> &'s BodyContentCommon<'a> {
    match structure {
        BodyStructure::Multipart { common, .. }
        | BodyStructure::Basic { common, .. }
        | BodyStructure::Text { common, .. }
        | BodyStructure::Message { common, .. } => common,
    }
}

fn mime_of(common: &BodyContentCommon<'_>) -> String {
    format!("{}/{}", common.ty.ty, common.ty.subtype).to_ascii_lowercase()
}

fn attached(common: &BodyContentCommon<'_>) -> bool {
    common
        .disposition
        .as_ref()
        .is_some_and(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"))
}

fn decoded_size(part: &BodyContentSinglePart<'_>) -> usize {
    let octets = part.octets as usize;
    match part.transfer_encoding {
        ContentEncoding::Base64 => octets * 3 / 4,
        _ => octets,
    }
}

pub(crate) fn parts_of(structure: &BodyStructure<'_>) -> Vec<Part> {
    let (common, other) = match structure {
        BodyStructure::Multipart { bodies, .. } => {
            return bodies.iter().flat_map(parts_of).collect();
        }
        BodyStructure::Basic { common, other, .. }
        | BodyStructure::Text { common, other, .. }
        | BodyStructure::Message { common, other, .. } => (common, other),
    };
    let mime = mime_of(common);
    let is_body = matches!(mime.as_str(), "text/plain" | "text/html") && !attached(common);
    if is_body {
        return Vec::new();
    }
    let name = common
        .disposition
        .as_ref()
        .and_then(|disposition| parameter(&disposition.params, "filename"))
        .or_else(|| parameter(&common.ty.params, "name"));
    let method = parameter(&common.ty.params, "method").map(|method| method.to_ascii_uppercase());
    vec![Part {
        mime,
        name,
        embedded: other.id.is_some() && !attached(common),
        size: decoded_size(other),
        method,
    }]
}

pub(crate) fn is_encrypted(structure: &BodyStructure<'_>) -> bool {
    let common = common_of(structure);
    let mime = mime_of(common);
    mime == "multipart/encrypted"
        || (kinds::is_smime(&mime)
            && kinds::is_encrypted_smime(parameter(&common.ty.params, "smime-type").as_deref()))
}

pub(crate) fn is_notice(structure: &BodyStructure<'_>) -> bool {
    mime_of(common_of(structure)) == "multipart/report"
        || parts_of(structure)
            .iter()
            .any(|part| part.mime == "message/disposition-notification")
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
    use crate::kinds::Kind;
    use async_imap::imap_proto::{ContentDisposition, ContentType};

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
                parts: Vec::new(),
                encrypted: false,
                notice: false,
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

    const GMAIL_REPLY: &[u8] = b"From: a@example.com\r\n\
Subject: Re: Lunch\r\n\
Content-Type: multipart/related; boundary=\"r\"\r\n\
\r\n\
--r\r\n\
Content-Type: multipart/alternative; boundary=\"a\"\r\n\
\r\n\
--a\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Noon works.\r\n\
--a\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Noon works.</p>\r\n\
--a--\r\n\
--r\r\n\
Content-Type: image/png; name=logo.png\r\n\
Content-Transfer-Encoding: base64\r\n\
Content-ID: <logo@example.com>\r\n\
\r\n\
@PNG@\r\n\
--r--\r\n";

    fn gmail_reply() -> Vec<u8> {
        let png = "QUJDREVG".repeat(1_100);
        String::from_utf8_lossy(GMAIL_REPLY)
            .replace("@PNG@", &png)
            .into_bytes()
    }

    #[test]
    fn gmail_reply_with_a_signature_logo_is_formatted_text() {
        assert_eq!(whole(&gmail_reply()).kinds, [Kind::FormattedText]);
    }

    #[test]
    fn inline_photo_is_a_picture() {
        let photo = "QUJDREVG".repeat(20_000);
        let raw = format!(
            "From: a@example.com\r\nSubject: Photo\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
--m\r\nContent-Type: text/plain\r\n\r\nLook.\r\n\
--m\r\nContent-Type: image/jpeg; name=IMG_0001.jpg\r\nContent-Disposition: inline\r\n\
Content-Transfer-Encoding: base64\r\nContent-ID: <photo@example.com>\r\n\r\n{photo}\r\n--m--\r\n"
        );
        assert!(whole(raw.as_bytes()).kinds.contains(&Kind::Pictures));
    }

    #[test]
    fn out_of_office_is_a_notice() {
        let raw = b"From: a@example.com\r\nSubject: Away\r\nAuto-Submitted: auto-replied\r\n\r\nBack Monday.";
        assert_eq!(whole(raw).kinds, [Kind::Notices]);
    }

    #[test]
    fn precedence_bulk_is_a_newsletter() {
        let raw = b"From: a@example.com\r\nSubject: Deals\r\nPrecedence: bulk\r\n\r\nBuy.";
        assert_eq!(whole(raw).kinds, [Kind::Newsletters]);
    }

    fn params(pairs: &[(&'static str, &'static str)]) -> BodyParams<'static> {
        use std::borrow::Cow;
        Some(
            pairs
                .iter()
                .map(|(key, value)| (Cow::Borrowed(*key), Cow::Borrowed(*value)))
                .collect(),
        )
    }

    fn common(
        ty: &'static str,
        subtype: &'static str,
        pairs: &[(&'static str, &'static str)],
        disposition: Option<&'static str>,
    ) -> BodyContentCommon<'static> {
        use std::borrow::Cow;
        BodyContentCommon {
            ty: ContentType {
                ty: Cow::Borrowed(ty),
                subtype: Cow::Borrowed(subtype),
                params: params(pairs),
            },
            disposition: disposition.map(|ty| ContentDisposition {
                ty: Cow::Borrowed(ty),
                params: None,
            }),
            language: None,
            location: None,
        }
    }

    fn single(
        id: Option<&'static str>,
        encoding: ContentEncoding<'static>,
        octets: u32,
    ) -> BodyContentSinglePart<'static> {
        use std::borrow::Cow;
        BodyContentSinglePart {
            id: id.map(Cow::Borrowed),
            md5: None,
            description: None,
            transfer_encoding: encoding,
            octets,
        }
    }

    fn leaf(
        ty: &'static str,
        subtype: &'static str,
        pairs: &[(&'static str, &'static str)],
        disposition: Option<&'static str>,
        id: Option<&'static str>,
        encoding: ContentEncoding<'static>,
        octets: u32,
    ) -> BodyStructure<'static> {
        let common = common(ty, subtype, pairs, disposition);
        let other = single(id, encoding, octets);
        if ty == "TEXT" {
            BodyStructure::Text {
                common,
                other,
                lines: 1,
                extension: None,
            }
        } else {
            BodyStructure::Basic {
                common,
                other,
                extension: None,
            }
        }
    }

    fn multipart(
        subtype: &'static str,
        pairs: &[(&'static str, &'static str)],
        bodies: Vec<BodyStructure<'static>>,
    ) -> BodyStructure<'static> {
        BodyStructure::Multipart {
            common: common("MULTIPART", subtype, pairs, None),
            bodies,
            extension: None,
        }
    }

    fn body_text() -> BodyStructure<'static> {
        leaf(
            "TEXT",
            "PLAIN",
            &[],
            None,
            None,
            ContentEncoding::SevenBit,
            20,
        )
    }

    fn first_part(structure: &BodyStructure<'static>) -> Message {
        let header = b"From: a@example.com\r\nSubject: Big\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n";
        parse(&Fetched {
            mailbox: "INBOX",
            uid: 1,
            seen: true,
            header,
            body: Body::FirstPart {
                mime: Some(b"Content-Type: text/plain; charset=utf-8\r\n\r\n"),
                data: b"Hello there.",
                attachments: attachment_names(structure),
                html: has_html(structure),
                parts: parts_of(structure),
                encrypted: is_encrypted(structure),
                notice: is_notice(structure),
            },
            arrived: None,
        })
        .expect("a message")
    }

    fn embedded_image(octets: u32) -> BodyStructure<'static> {
        multipart(
            "MIXED",
            &[],
            vec![
                body_text(),
                leaf(
                    "IMAGE",
                    "PNG",
                    &[("name", "logo.png")],
                    Some("inline"),
                    Some("<logo@example.com>"),
                    ContentEncoding::Base64,
                    octets,
                ),
            ],
        )
    }

    #[test]
    fn large_voice_message_is_audio() {
        let structure = multipart(
            "MIXED",
            &[],
            vec![
                body_text(),
                leaf(
                    "AUDIO",
                    "MP4",
                    &[("name", "voice.m4a")],
                    Some("attachment"),
                    None,
                    ContentEncoding::Base64,
                    4_000_000,
                ),
            ],
        );
        let parts = parts_of(&structure);
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].mime, "audio/mp4");
        assert_eq!(parts[0].name.as_deref(), Some("voice.m4a"));
        assert_eq!(parts[0].size, 3_000_000);
        assert!(!parts[0].embedded);
        assert_eq!(first_part(&structure).kinds, [Kind::PlainText, Kind::Audio]);
    }

    #[test]
    fn small_embedded_logo_on_the_first_part_path_is_formatted_text() {
        let structure = embedded_image(8_000);
        assert!(parts_of(&structure)[0].embedded);
        let message = first_part(&embedded_image(8_000));
        assert_eq!(message.kinds, [Kind::PlainText]);
    }

    #[test]
    fn base64_octets_count_as_decoded_size_at_the_picture_edge() {
        assert_eq!(parts_of(&embedded_image(120_000))[0].size, 90_000);
        assert_eq!(
            first_part(&embedded_image(120_000)).kinds,
            [Kind::PlainText]
        );
        assert_eq!(
            first_part(&embedded_image(140_000)).kinds,
            [Kind::PlainText, Kind::Pictures]
        );
    }

    #[test]
    fn attached_image_with_a_content_id_is_not_embedded() {
        let structure = multipart(
            "MIXED",
            &[],
            vec![
                body_text(),
                leaf(
                    "IMAGE",
                    "JPEG",
                    &[],
                    Some("attachment"),
                    Some("<photo@example.com>"),
                    ContentEncoding::SevenBit,
                    5_000,
                ),
            ],
        );
        assert!(!parts_of(&structure)[0].embedded);
        assert!(first_part(&structure).kinds.contains(&Kind::Pictures));
    }

    #[test]
    fn report_structure_is_a_notice() {
        let structure = multipart(
            "REPORT",
            &[("report-type", "delivery-status")],
            vec![
                body_text(),
                leaf(
                    "MESSAGE",
                    "DELIVERY-STATUS",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    300,
                ),
                leaf(
                    "MESSAGE",
                    "RFC822",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    2_000_000,
                ),
            ],
        );
        assert!(is_notice(&structure));
        assert_eq!(first_part(&structure).kinds, [Kind::Notices]);
    }

    #[test]
    fn disposition_notification_structure_is_a_notice() {
        let structure = multipart(
            "MIXED",
            &[],
            vec![
                body_text(),
                leaf(
                    "MESSAGE",
                    "DISPOSITION-NOTIFICATION",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    200,
                ),
            ],
        );
        assert!(is_notice(&structure));
        assert!(!is_notice(&embedded_image(8_000)));
    }

    #[test]
    fn encrypted_structure_is_anything_else() {
        let pgp = multipart(
            "ENCRYPTED",
            &[("protocol", "application/pgp-encrypted")],
            vec![
                leaf(
                    "APPLICATION",
                    "PGP-ENCRYPTED",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    11,
                ),
                leaf(
                    "APPLICATION",
                    "OCTET-STREAM",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    5_000,
                ),
            ],
        );
        assert!(is_encrypted(&pgp));
        assert_eq!(first_part(&pgp).kinds, [Kind::AnythingElse]);
        let enveloped = leaf(
            "APPLICATION",
            "PKCS7-MIME",
            &[("smime-type", "enveloped-data")],
            None,
            None,
            ContentEncoding::Base64,
            9_000,
        );
        assert!(is_encrypted(&enveloped));
        let signed_only = leaf(
            "APPLICATION",
            "PKCS7-MIME",
            &[("smime-type", "signed-data")],
            None,
            None,
            ContentEncoding::Base64,
            9_000,
        );
        assert!(!is_encrypted(&signed_only));
        assert!(!is_encrypted(&embedded_image(8_000)));
    }

    #[test]
    fn calendar_method_comes_from_the_structure() {
        let invitation = |method| {
            multipart(
                "ALTERNATIVE",
                &[],
                vec![
                    body_text(),
                    leaf(
                        "TEXT",
                        "CALENDAR",
                        &[("method", method)],
                        None,
                        None,
                        ContentEncoding::SevenBit,
                        900,
                    ),
                ],
            )
        };
        let reply = invitation("reply");
        assert_eq!(parts_of(&reply)[0].method.as_deref(), Some("REPLY"));
        assert_eq!(first_part(&reply).kinds, [Kind::InvitationReplies]);
        assert_eq!(
            first_part(&invitation("REQUEST")).kinds,
            [Kind::Invitations]
        );
    }

    #[test]
    fn signature_parts_add_no_kind_on_the_first_part_path() {
        let structure = multipart(
            "SIGNED",
            &[],
            vec![
                body_text(),
                leaf(
                    "APPLICATION",
                    "PGP-SIGNATURE",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    800,
                ),
            ],
        );
        assert_eq!(first_part(&structure).kinds, [Kind::PlainText]);
    }

    #[test]
    fn gmail_reaction_structure_is_only_a_reaction() {
        let structure = multipart(
            "ALTERNATIVE",
            &[],
            vec![
                body_text(),
                leaf(
                    "TEXT",
                    "VND.GOOGLE.EMAIL-REACTION+JSON",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    40,
                ),
                leaf(
                    "TEXT",
                    "HTML",
                    &[],
                    None,
                    None,
                    ContentEncoding::SevenBit,
                    200,
                ),
            ],
        );
        assert_eq!(first_part(&structure).kinds, [Kind::Reactions]);
    }

    fn mixed(parts: &str) -> Vec<u8> {
        format!(
            "From: a@example.com\r\nSubject: S\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
--m\r\nContent-Type: text/plain\r\n\r\nHello.\r\n{parts}--m--\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn gmail_reaction_message_is_only_a_reaction() {
        let raw = b"From: a@example.com\r\nSubject: Re: Lunch\r\nContent-Type: multipart/alternative; boundary=\"a\"\r\n\r\n\
--a\r\nContent-Type: text/plain\r\n\r\nReacted.\r\n\
--a\r\nContent-Type: text/vnd.google.email-reaction+json\r\n\r\n{\"emoji\":\"\xf0\x9f\x91\x8d\",\"version\":1}\r\n\
--a\r\nContent-Type: text/html\r\n\r\n<p>Reacted.</p>\r\n--a--\r\n";
        assert_eq!(whole(raw).kinds, [Kind::Reactions]);
    }

    #[test]
    fn delivery_report_is_a_notice_whatever_it_carries() {
        let raw = b"From: a@example.com\r\nSubject: Undelivered\r\nContent-Type: multipart/report; report-type=delivery-status; boundary=\"r\"\r\n\r\n\
--r\r\nContent-Type: text/plain\r\n\r\nCould not deliver.\r\n\
--r\r\nContent-Type: message/delivery-status\r\n\r\nReporting-MTA: dns; example.com\r\n\r\n\
--r\r\nContent-Type: application/pdf; name=a.pdf\r\nContent-Disposition: attachment\r\n\r\nxx\r\n--r--\r\n";
        assert_eq!(whole(raw).kinds, [Kind::Notices]);
    }

    #[test]
    fn disposition_notification_part_is_a_notice() {
        let raw = mixed(
            "--m\r\nContent-Type: message/disposition-notification\r\n\r\nFinal-Recipient: rfc822; me@example.org\r\n\r\n",
        );
        assert_eq!(whole(&raw).kinds, [Kind::Notices]);
    }

    #[test]
    fn encrypted_message_is_anything_else() {
        let raw = b"From: a@example.com\r\nSubject: Secret\r\nContent-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=\"e\"\r\n\r\n\
--e\r\nContent-Type: application/pgp-encrypted\r\n\r\nVersion: 1\r\n\
--e\r\nContent-Type: application/octet-stream\r\n\r\nxxxx\r\n--e--\r\n";
        assert_eq!(whole(raw).kinds, [Kind::AnythingElse]);
        let smime = b"From: a@example.com\r\nSubject: Secret\r\nContent-Type: application/pkcs7-mime; smime-type=enveloped-data; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\nQUJDREVG\r\n";
        assert_eq!(whole(smime).kinds, [Kind::AnythingElse]);
    }

    #[test]
    fn calendar_method_decides_invitation_or_reply() {
        let calendar = |method: &str| {
            mixed(&format!(
                "--m\r\nContent-Type: text/calendar; method={method}\r\n\r\nBEGIN:VCALENDAR\r\nMETHOD:{method}\r\nEND:VCALENDAR\r\n"
            ))
        };
        assert_eq!(whole(&calendar("REQUEST")).kinds, [Kind::Invitations]);
        assert_eq!(whole(&calendar("REPLY")).kinds, [Kind::InvitationReplies]);
    }

    #[test]
    fn signature_part_adds_no_kind() {
        let raw = mixed(
            "--m\r\nContent-Type: application/pgp-signature; name=signature.asc\r\n\r\n-----BEGIN PGP SIGNATURE-----\r\n",
        );
        assert_eq!(whole(&raw).kinds, [Kind::PlainText]);
    }

    #[test]
    fn notice_and_newsletter_headers_suppress_other_kinds() {
        let raw = b"From: a@example.com\r\nSubject: Away\r\nAuto-Submitted: auto-generated\r\nList-Id: <l.example.com>\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
--m\r\nContent-Type: text/plain\r\n\r\nBack soon.\r\n\
--m\r\nContent-Type: audio/mp4; name=a.m4a\r\nContent-Disposition: attachment\r\n\r\nxx\r\n--m--\r\n";
        assert_eq!(whole(raw).kinds, [Kind::Notices]);
        let no = b"From: a@example.com\r\nSubject: Hi\r\nAuto-Submitted: no\r\n\r\nHello.";
        assert_eq!(whole(no).kinds, [Kind::PlainText]);
    }

    fn gmail_invitation(method: &str) -> Vec<u8> {
        format!(
            "From: a@example.com\r\nSubject: Invitation\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
--m\r\nContent-Type: multipart/alternative; boundary=\"a\"\r\n\r\n\
--a\r\nContent-Type: text/plain\r\n\r\nLunch on Friday.\r\n\
--a\r\nContent-Type: text/html\r\n\r\n<p>Lunch on Friday.</p>\r\n\
--a\r\nContent-Type: text/calendar; charset=UTF-8; method={method}\r\n\r\nBEGIN:VCALENDAR\r\nMETHOD:{method}\r\nEND:VCALENDAR\r\n--a--\r\n\
--m\r\nContent-Type: application/ics; name=\"invite.ics\"\r\nContent-Disposition: attachment; filename=\"invite.ics\"\r\n\r\nBEGIN:VCALENDAR\r\nMETHOD:{method}\r\nEND:VCALENDAR\r\n--m--\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn gmail_invite_ics_copy_adds_no_kind() {
        assert_eq!(
            whole(&gmail_invitation("REPLY")).kinds,
            [Kind::InvitationReplies]
        );
        assert_eq!(
            whole(&gmail_invitation("REQUEST")).kinds,
            [Kind::Invitations]
        );
    }

    #[test]
    fn lone_ics_attachment_is_an_invitation() {
        let raw = mixed(
            "--m\r\nContent-Type: application/ics; name=\"invite.ics\"\r\nContent-Disposition: attachment\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        );
        assert_eq!(whole(&raw).kinds, [Kind::Invitations]);
        let named = mixed(
            "--m\r\nContent-Type: application/octet-stream; name=\"meeting.ICS\"\r\nContent-Disposition: attachment\r\n\r\nBEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        );
        assert_eq!(whole(&named).kinds, [Kind::Invitations]);
    }

    #[test]
    fn gmail_invite_ics_copy_adds_no_kind_on_the_first_part_path() {
        let invitation = |method| {
            multipart(
                "MIXED",
                &[],
                vec![
                    multipart(
                        "ALTERNATIVE",
                        &[],
                        vec![
                            body_text(),
                            leaf(
                                "TEXT",
                                "CALENDAR",
                                &[("method", method)],
                                None,
                                None,
                                ContentEncoding::SevenBit,
                                900,
                            ),
                        ],
                    ),
                    leaf(
                        "APPLICATION",
                        "ICS",
                        &[("name", "invite.ics")],
                        Some("attachment"),
                        None,
                        ContentEncoding::Base64,
                        1_200,
                    ),
                ],
            )
        };
        assert_eq!(
            first_part(&invitation("REPLY")).kinds,
            [Kind::InvitationReplies]
        );
        assert_eq!(
            first_part(&invitation("REQUEST")).kinds,
            [Kind::Invitations]
        );
        let lone = multipart(
            "MIXED",
            &[],
            vec![
                body_text(),
                leaf(
                    "APPLICATION",
                    "ICS",
                    &[("name", "invite.ics")],
                    Some("attachment"),
                    None,
                    ContentEncoding::Base64,
                    1_200,
                ),
            ],
        );
        assert_eq!(first_part(&lone).kinds, [Kind::Invitations]);
    }

    #[test]
    fn every_smime_type_but_signatures_is_encrypted() {
        let smime = |subtype, smime_type| {
            leaf(
                "APPLICATION",
                subtype,
                &[("smime-type", smime_type)],
                None,
                None,
                ContentEncoding::Base64,
                9_000,
            )
        };
        assert!(is_encrypted(&smime("X-PKCS7-MIME", "enveloped-data")));
        assert!(is_encrypted(&smime("PKCS7-MIME", "authEnveloped-data")));
        assert!(!is_encrypted(&smime("X-PKCS7-MIME", "signed-data")));
        assert!(!is_encrypted(&smime("PKCS7-MIME", "certs-only")));
        let legacy = b"From: a@example.com\r\nSubject: Secret\r\nContent-Type: application/x-pkcs7-mime; smime-type=enveloped-data; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\nQUJDREVG\r\n";
        assert_eq!(whole(legacy).kinds, [Kind::AnythingElse]);
        let authenticated = b"From: a@example.com\r\nSubject: Secret\r\nContent-Type: application/pkcs7-mime; smime-type=authEnveloped-data; name=smime.p7m\r\nContent-Transfer-Encoding: base64\r\n\r\nQUJDREVG\r\n";
        assert_eq!(whole(authenticated).kinds, [Kind::AnythingElse]);
    }
}
