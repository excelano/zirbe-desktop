// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use mail_parser::MessagePart;

use crate::model::{Invite, Original, Outgoing};

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
    message
        .parts()
        .find(|part| part.is_content_type("text", "vnd.google.email-reaction+json"))
        .and_then(|part| part.text().map(|text| text.into_owned()))
        .and_then(|json| serde_json::from_str::<serde_json::Value>(&json).ok())
        .and_then(|value| value.get("emoji")?.as_str().map(str::to_string))
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

pub(crate) fn attachment_names(message: &mail_parser::Message<'_>) -> Vec<String> {
    message
        .attachments()
        .filter(listed)
        .map(|part| name_of(&part))
        .collect()
}

pub(crate) fn attachment(raw: &[u8], index: usize, name: &str) -> Option<Vec<u8>> {
    let message = mail_parser::MessageParser::default().parse(raw)?;
    let parts: Vec<MessagePart<'_>> = message.attachments().filter(listed).collect();
    let part = parts
        .get(index)
        .filter(|part| name_of(part) == name)
        .or_else(|| parts.iter().find(|part| name_of(part) == name))?;
    Some(part.decoded().into_owned())
}

pub(crate) fn original(raw: &[u8]) -> Option<Original> {
    let message = mail_parser::MessageParser::default().parse(raw)?;
    Some(Original {
        text: text_of(&message),
        attachments: message
            .attachments()
            .filter(listed)
            .map(|part| Outgoing {
                name: name_of(&part),
                bytes: part.decoded().into_owned(),
            })
            .collect(),
    })
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
    let mut end = lines.len();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let next = lines.get(index + 1).map_or("", |next| next.trim());
        if trimmed == "--"
            || trimmed.starts_with("-----Original Message-----")
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
        {
            end = index;
            break;
        }
    }
    let kept = lines[..end].join("\n");
    let kept = kept.trim();
    if kept.is_empty() {
        text.trim().to_string()
    } else {
        kept.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{attachment, attachment_names, reply_part};

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
    fn keeps_quotes_answered_inline() {
        let text = "> Lunch?\nYes, noon.\n> Where?\nThe usual.";
        assert_eq!(reply_part(text), text);
    }

    #[test]
    fn keeps_everything_when_the_whole_message_is_a_quote() {
        assert_eq!(reply_part("> only a quote"), "> only a quote");
    }
}
