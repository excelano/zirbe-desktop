// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use mail_parser::MessagePart;

pub(crate) fn text_of(message: &mail_parser::Message<'_>) -> String {
    if let Some(text) = message.body_text(0)
        && !text.trim().is_empty()
        && !message
            .text_body()
            .next()
            .is_some_and(|part| part.is_content_type("text", "html"))
    {
        return text.into_owned();
    }
    if let Some(html) = message.body_html(0) {
        return from_html(&html);
    }
    String::new()
}

pub(crate) fn has_html(message: &mail_parser::Message<'_>) -> bool {
    message
        .html_body()
        .any(|part| part.is_content_type("text", "html"))
}

pub(crate) fn attachment_names(message: &mail_parser::Message<'_>) -> Vec<String> {
    message
        .attachments()
        .filter(|part: &MessagePart<'_>| !part.is_inline() || part.attachment_name().is_some())
        .map(|part| part.attachment_name().unwrap_or("attachment").to_string())
        .collect()
}

fn from_html(html: &str) -> String {
    html2text::config::plain_no_decorate()
        .string_from_read(html.as_bytes(), 10_000)
        .unwrap_or_default()
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
    use super::reply_part;

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
