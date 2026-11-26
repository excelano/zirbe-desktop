// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use mail_parser::{MessagePart, PartRole};

use crate::kinds::{self, Facts, Part};
use crate::model::{Invite, List, Original, Outgoing};

pub(crate) fn text_of(message: &mail_parser::Message<'_>) -> String {
    if let Some(text) = message.body_text(0)
        && !text.trim().is_empty()
        && !message
            .text_body()
            .next()
            .is_some_and(|part| part.is_content_type("text", "html"))
    {
        if !markup(&text) {
            return text.into_owned();
        }
        if !has_html(message) {
            return from_html(&text);
        }
    }
    if let Some(html) = message.body_html(0) {
        return from_html(&html);
    }
    String::new()
}

fn markup(text: &str) -> bool {
    let start: String = text
        .trim_start()
        .chars()
        .take(512)
        .collect::<String>()
        .to_lowercase();
    start.starts_with("<!doctype html") || start.starts_with("<html") || start.contains("<body")
}

pub(crate) const REACTION_MIME: &str = "text/vnd.google.email-reaction+json";

pub(crate) fn reaction(message: &mail_parser::Message<'_>) -> Option<String> {
    if let Some(value) = message.headers().value("X-Zirbe-Reaction")
        && let Some(text) = value.as_text()
    {
        let text = text.trim();
        let decoded = if text.starts_with("=?") {
            let header = format!("Subject: {text}\r\n\r\n");
            mail_parser::MessageParser::default()
                .parse(header.as_bytes())
                .and_then(|parsed| parsed.subject().map(str::to_string))
                .unwrap_or_else(|| text.to_string())
        } else {
            text.to_string()
        };
        if !decoded.is_empty() {
            return Some(decoded);
        }
    }
    if message.in_reply_to().is_some()
        && let Some(emoji) = standard_reaction(message)
    {
        return Some(emoji);
    }
    message
        .parts()
        .find(|part| part.is_content_type("text", "vnd.google.email-reaction+json"))
        .and_then(|part| part.text().map(|text| text.into_owned()))
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|value| value.get("emoji")?.as_str().map(str::to_string))
}

fn header_text(message: &mail_parser::Message<'_>, name: &str) -> Option<String> {
    message.headers().get(name).map(|header| {
        String::from_utf8_lossy(header.raw_value())
            .trim()
            .to_ascii_lowercase()
    })
}

fn mime_of(part: &MessagePart<'_>) -> String {
    part.content_type().map_or_else(
        || "text/plain".to_string(),
        |content_type| {
            format!(
                "{}/{}",
                content_type.ctype(),
                content_type.subtype().unwrap_or_default()
            )
            .to_ascii_lowercase()
        },
    )
}

fn attached(part: &MessagePart<'_>) -> bool {
    part.content_disposition()
        .is_some_and(|disposition| disposition.ctype().eq_ignore_ascii_case("attachment"))
}

fn part_of(part: &MessagePart<'_>) -> Option<Part> {
    let mime = mime_of(part);
    let is_body = matches!(mime.as_str(), "text/plain" | "text/html") && !attached(part);
    if part.is_multipart() || is_body || mime == REACTION_MIME {
        return None;
    }
    Some(Part {
        name: part.attachment_name().map(str::to_string),
        embedded: part.content_id().is_some() && !attached(part),
        size: part.decoded_len(),
        method: part
            .content_type()
            .and_then(|content_type| content_type.attribute("method"))
            .map(str::to_ascii_uppercase),
        mime,
    })
}

pub(crate) fn facts(message: &mail_parser::Message<'_>, text: &str) -> Facts {
    let parts: Vec<Part> = message.parts().filter_map(|part| part_of(&part)).collect();
    let root = message.root_part();
    let root_mime = mime_of(&root);
    let smime_type = root
        .content_type()
        .and_then(|content_type| content_type.attribute("smime-type"));
    let precedence = header_text(message, "Precedence");
    let auto_submitted = header_text(message, "Auto-Submitted");
    Facts {
        text: !text.trim().is_empty(),
        html: has_html(message),
        newsletter: message.list_id().is_some()
            || message.headers().get("List-Unsubscribe").is_some()
            || matches!(precedence.as_deref(), Some("bulk" | "list")),
        notice: root_mime == "multipart/report"
            || parts
                .iter()
                .any(|part| part.mime == "message/disposition-notification")
            || auto_submitted.is_some_and(|value| value != "no"),
        encrypted: root_mime == "multipart/encrypted"
            || (kinds::is_smime(&root_mime) && kinds::is_encrypted_smime(smime_type)),
        reaction: reaction(message).is_some(),
        parts,
    }
}

pub(crate) fn invite(message: &mail_parser::Message<'_>) -> Option<Invite> {
    message
        .parts()
        .filter(|part| part.is_content_type("text", "calendar"))
        .find_map(|part| crate::invite::parse(&part.text()?))
}

pub(crate) fn has_html(message: &mail_parser::Message<'_>) -> bool {
    message
        .html_body()
        .any(|part| part.is_content_type("text", "html"))
}

fn listed(part: &MessagePart<'_>) -> bool {
    !part.is_inline() || part.attachment_name().is_some()
}

fn name_of(part: &MessagePart<'_>) -> String {
    part.attachment_name().unwrap_or("attachment").to_string()
}

fn files<'m>(message: &'m mail_parser::Message<'_>) -> Vec<MessagePart<'m>> {
    let media = message
        .text_body()
        .chain(message.html_body())
        .filter(|part| part.role() == PartRole::MediaBody);
    let mut parts: Vec<MessagePart<'m>> =
        message.attachments().chain(media).filter(listed).collect();
    parts.sort_by_key(MessagePart::id);
    parts.dedup_by_key(|part| part.id());
    parts
}

pub(crate) fn attachment_names(message: &mail_parser::Message<'_>) -> Vec<String> {
    files(message).iter().map(name_of).collect()
}

pub(crate) fn attachment(raw: &[u8], index: usize, name: &str) -> Option<Vec<u8>> {
    let message = mail_parser::MessageParser::default().parse(raw)?;
    let parts = files(&message);
    let part = parts
        .get(index)
        .filter(|part| name_of(part) == name)
        .or_else(|| parts.iter().find(|part| name_of(part) == name))?;
    Some(part.decoded().into_owned())
}

pub(crate) fn mailing_list(message: &mail_parser::Message<'_>) -> Option<List> {
    let listed = message.list_id()?;
    let first = listed.first()?;
    let id = first
        .address()
        .map(str::trim)
        .filter(|id| !id.is_empty())?
        .trim_matches(['<', '>'])
        .to_ascii_lowercase();
    let name = first
        .name()
        .map(|name| name.trim().trim_matches('"').trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| id.clone());
    let post = message
        .headers()
        .get("List-Post")
        .map(|header| String::from_utf8_lossy(header.raw_value()).into_owned())
        .and_then(|value| {
            bracketed(&value)
                .into_iter()
                .find_map(|link| link.strip_prefix("mailto:").map(str::to_string))
        })
        .map(|address| {
            address
                .split('?')
                .next()
                .unwrap_or_default()
                .to_ascii_lowercase()
        })
        .filter(|address| address.contains('@'));
    Some(List { id, name, post })
}

fn standard_reaction(message: &mail_parser::Message<'_>) -> Option<String> {
    let root = message.root_part();
    let part = if root.is_multipart() {
        root.children().next()?
    } else {
        root
    };
    let marked = part
        .content_disposition()
        .is_some_and(|disposition| disposition.ctype().eq_ignore_ascii_case("reaction"));
    (marked && part.is_content_type("text", "plain")).then(|| {
        part.text()
            .unwrap_or_default()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    })
}

pub(crate) fn original(raw: &[u8]) -> Option<Original> {
    let message = mail_parser::MessageParser::default().parse(raw)?;
    Some(Original {
        text: text_of(&message),
        attachments: files(&message)
            .iter()
            .map(|part| Outgoing {
                name: name_of(part),
                bytes: part.decoded().into_owned(),
            })
            .collect(),
        unsubscribe: message
            .headers()
            .get("List-Unsubscribe")
            .map(|header| bracketed(&String::from_utf8_lossy(header.raw_value())))
            .unwrap_or_default(),
    })
}

fn bracketed(value: &str) -> Vec<String> {
    value
        .split('<')
        .skip(1)
        .filter_map(|rest| rest.split_once('>'))
        .map(|(link, _)| link.split_whitespace().collect::<String>())
        .filter(|link| !link.is_empty())
        .collect()
}

fn from_html(html: &str) -> String {
    let text = html2text::config::plain_no_decorate()
        .raw_mode(true)
        .string_from_read(html.as_bytes(), 10_000)
        .unwrap_or_default();
    let mut kept: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        let rule = !line.is_empty()
            && line
                .chars()
                .all(|c| matches!(c, '─' | '━' | '═' | '-' | '_' | ' '));
        let blank = line.is_empty() || rule;
        if blank && kept.last().is_none_or(|last| last.is_empty()) {
            continue;
        }
        kept.push(if blank { "" } else { line });
    }
    kept.join("\n").trim().to_string()
}

pub(crate) fn reply_part(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let end = (0..lines.len())
        .find(|&index| lines[index].trim() == "--" || starts_quote(&lines, index))
        .unwrap_or(lines.len());
    let kept = lines[..end].join("\n");
    let kept = kept.trim();
    if kept.is_empty() {
        text.trim().to_string()
    } else {
        kept.to_string()
    }
}

pub(crate) fn folds_quote(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    reply_part(text) != text.trim() && (0..lines.len()).any(|index| starts_quote(&lines, index))
}

fn starts_quote(lines: &[&str], index: usize) -> bool {
    let trimmed = lines[index].trim();
    let next = lines.get(index + 1).map_or("", |next| next.trim());
    trimmed.starts_with("-----Original Message-----")
        || trimmed.starts_with("________________________________")
        || (trimmed.starts_with("On ") && trimmed.ends_with("wrote:"))
        || (trimmed.starts_with("On ") && next.ends_with("wrote:") && !next.starts_with('>'))
        || (trimmed.starts_with("Am ") && trimmed.ends_with("schrieb:"))
        || (trimmed.starts_with("From:") && next.starts_with("Sent:"))
        || (trimmed.starts_with('>')
            && lines[index..].iter().all(|rest| {
                let rest = rest.trim();
                rest.is_empty() || rest.starts_with('>')
            }))
}

#[cfg(test)]
mod tests {
    use super::{attachment, attachment_names, folds_quote, reply_part};

    const WITH_FILES: &[u8] = b"From: a@example.com\r\nSubject: Files\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nTwo files.\r\n--x\r\nContent-Type: text/plain; name=notes.txt\r\nContent-Disposition: attachment; filename=notes.txt\r\n\r\nfirst\r\n--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=data.bin\r\nContent-Transfer-Encoding: base64\r\n\r\nAAEC\r\n--x--\r\n";

    #[test]
    fn reads_a_plain_part_that_holds_html_as_html() {
        let raw = b"From: a@example.com\r\nSubject: News\r\nContent-Type: text/plain\r\n\r\n<html><head><meta charset=utf-8></head><body><p>Week <b>ahead</b></p></body></html>";
        let message = mail_parser::MessageParser::default()
            .parse(raw)
            .expect("a message");
        assert_eq!(super::text_of(&message), "Week ahead");
    }

    #[test]
    fn reads_html_tables_and_rules_as_plain_lines() {
        let html = "<table><tr><td>Form</td><td>Due</td></tr></table><hr><p>Done.</p>";
        assert_eq!(super::from_html(html), "Form\nDue\n\nDone.");
    }

    #[test]
    fn lists_a_picture_sent_between_text_parts() {
        let raw = b"From: a@example.com\r\nSubject: Photo\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\nHere it is\r\n--x\r\nContent-Type: image/jpeg\r\nContent-Disposition: inline; filename=IMG_0001.jpeg\r\nContent-Transfer-Encoding: base64\r\n\r\nAAEC\r\n--x\r\nContent-Type: image/png; name=drawing.png\r\nContent-Transfer-Encoding: base64\r\n\r\nAwQF\r\n--x\r\nContent-Type: text/plain\r\n\r\nSent from my iPhone\r\n--x--\r\n";
        let message = mail_parser::MessageParser::default()
            .parse(raw)
            .expect("a message");
        assert_eq!(attachment_names(&message), ["IMG_0001.jpeg", "drawing.png"]);
        assert_eq!(attachment(raw, 1, "drawing.png"), Some(vec![3, 4, 5]));
    }

    #[test]
    fn pulls_an_attachment_out_by_position_and_name() {
        let message = mail_parser::MessageParser::default()
            .parse(WITH_FILES)
            .expect("a message");
        assert_eq!(attachment_names(&message), ["notes.txt", "data.bin"]);
        assert_eq!(attachment(WITH_FILES, 1, "data.bin"), Some(vec![0, 1, 2]));
        assert_eq!(attachment(WITH_FILES, 0, "data.bin"), Some(vec![0, 1, 2]));
        assert_eq!(attachment(WITH_FILES, 0, "missing.pdf"), None);
    }

    #[test]
    fn cuts_an_attribution_and_the_quote_below_it() {
        let text =
            "Sounds good.\n\nOn Mon, 9 Nov 2026 at 10:00, Ana <ana@example.com> wrote:\n> Lunch?\n";
        assert_eq!(reply_part(text), "Sounds good.");
    }

    #[test]
    fn cuts_an_attribution_wrapped_over_two_lines() {
        let text =
            "Yes.\n\nOn Mon, 9 Nov 2026 at 10:00, Ana Lopez\n<ana@example.com> wrote:\n> Lunch?";
        assert_eq!(reply_part(text), "Yes.");
    }

    #[test]
    fn cuts_an_outlook_header_block_and_a_signature() {
        assert_eq!(
            reply_part("Done.\n\nFrom: Ana\nSent: Monday\nTo: me"),
            "Done."
        );
        assert_eq!(reply_part("Done.\n-- \nAna Lopez\nExcelano"), "Done.");
    }

    #[test]
    fn tells_a_folded_quote_from_a_cut_signature() {
        assert!(folds_quote("Yes.\n\nOn Monday, Ana wrote:\n> Lunch?"));
        assert!(folds_quote(
            "Yes.\n-- \nDavid\n\nOn Monday, Ana wrote:\n> Lunch?"
        ));
        assert!(!folds_quote("Done.\n-- \nAna Lopez\nExcelano"));
        assert!(!folds_quote("> only a quote"));
        assert!(!folds_quote("> Lunch?\nYes, noon.\n> Where?\nThe usual."));
    }

    fn parsed(raw: &str) -> Option<String> {
        let message = mail_parser::MessageParser::default()
            .parse(raw.as_bytes())
            .expect("a message");
        super::reaction(&message)
    }

    #[test]
    fn reads_a_standard_reaction_from_another_client() {
        let single = "From: a@example.org\r\nIn-Reply-To: <t@example.org>\r\n\
            Content-Type: text/plain; charset=utf-8\r\nContent-Disposition: reaction\r\n\r\n\u{1F44D}\r\n";
        assert_eq!(parsed(single).as_deref(), Some("\u{1F44D}"));
        let withdrawn = "From: a@example.org\r\nIn-Reply-To: <t@example.org>\r\n\
            Content-Type: text/plain; charset=utf-8\r\nContent-Disposition: reaction\r\n\r\n\r\n";
        assert_eq!(parsed(withdrawn).as_deref(), Some(""));
        let first_part = "From: a@example.org\r\nIn-Reply-To: <t@example.org>\r\n\
            Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\n\
            Content-Type: text/plain; charset=utf-8\r\nContent-Disposition: reaction\r\n\r\n\u{2764}\r\n\
            --b\r\nContent-Type: text/plain\r\n\r\nNice.\r\n--b--\r\n";
        assert_eq!(parsed(first_part).as_deref(), Some("\u{2764}"));
        let unanswered = "From: a@example.org\r\nContent-Type: text/plain; charset=utf-8\r\n\
            Content-Disposition: reaction\r\n\r\n\u{1F44D}\r\n";
        assert_eq!(parsed(unanswered), None);
    }

    #[test]
    fn reads_a_lists_name_id_and_posting_address() {
        let raw =
            b"From: ana@example.com\r\nList-Id: \"Hiking Club\" <hiking.lists.example.org>\r\n\
            List-Post: <mailto:Hiking@lists.example.org>\r\nSubject: Saturday\r\n\r\nWho is in?";
        let message = mail_parser::MessageParser::default()
            .parse(&raw[..])
            .expect("a message");
        assert_eq!(
            super::mailing_list(&message),
            Some(crate::model::List {
                id: "hiking.lists.example.org".into(),
                name: "Hiking Club".into(),
                post: Some("hiking@lists.example.org".into()),
            })
        );
        let bare =
            b"From: news@example.com\r\nList-Id: <news.example.com>\r\nList-Post: NO\r\n\r\nHi";
        let message = mail_parser::MessageParser::default()
            .parse(&bare[..])
            .expect("a message");
        let list = super::mailing_list(&message).expect("a list");
        assert_eq!((list.name.as_str(), list.post), ("news.example.com", None));
    }

    #[test]
    fn reads_a_lists_unsubscribe_addresses() {
        let raw = b"From: news@example.com\r\nList-Unsubscribe: <mailto:leave@example.com?subject=stop>,\r\n <https://example.com/u/\r\n 42>\r\nSubject: News\r\n\r\nHello";
        let original = super::original(raw).expect("a message");
        assert_eq!(
            original.unsubscribe,
            [
                "mailto:leave@example.com?subject=stop",
                "https://example.com/u/42"
            ]
        );
    }

    #[test]
    fn keeps_quotes_answered_inline() {
        let text = "> Lunch?\nYes, noon.\n> Where?\nThe usual.";
        assert_eq!(reply_part(text), text);
    }

    #[test]
    fn keeps_everything_when_the_whole_message_is_a_quote() {
        assert_eq!(reply_part("> only a quote"), "> only a quote");
    }
}
