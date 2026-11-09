// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use mail_parser::MessageParser;

pub(crate) fn document(raw: &[u8]) -> Option<String> {
    let message = MessageParser::default().parse(raw)?;
    let html = message
        .html_body()
        .filter(|part| part.is_content_type("text", "html"))
        .filter_map(|part| part.text())
        .collect::<Vec<_>>();
    if html.is_empty() {
        let text = message.body_text(0).unwrap_or_default();
        return Some(format!(
            "<pre style=\"white-space: pre-wrap; font: 15px sans-serif\">{}</pre>",
            escape(&text)
        ));
    }
    let mut document = html.join("\n");
    for part in message.parts() {
        let (Some(id), Some(kind)) = (part.content_id(), part.content_type()) else {
            continue;
        };
        if !kind.ctype().eq_ignore_ascii_case("image") {
            continue;
        }
        let id = id.trim().trim_start_matches('<').trim_end_matches('>');
        let uri = format!(
            "data:image/{};base64,{}",
            kind.subtype().unwrap_or("png"),
            STANDARD.encode(part.decoded())
        );
        document = replace_cid(&document, id, &uri);
    }
    Some(document)
}

fn replace_cid(html: &str, id: &str, uri: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let wanted = format!("cid:{}", id.to_ascii_lowercase());
    let mut out = String::with_capacity(html.len());
    let mut rest = 0;
    while let Some(found) = lower[rest..].find(&wanted) {
        let start = rest + found;
        let end = start + wanted.len();
        let boundary = html[end..]
            .chars()
            .next()
            .is_none_or(|next| matches!(next, '"' | '\'' | ')' | '>' | ' ' | '\t' | '\r' | '\n'));
        out.push_str(&html[rest..start]);
        out.push_str(if boundary { uri } else { &html[start..end] });
        rest = end;
    }
    out.push_str(&html[rest..]);
    out
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::{document, replace_cid};

    #[test]
    fn paints_inline_images_from_the_message() {
        let raw = b"From: a@example.com\r\nSubject: Logo\r\nContent-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<p><img src=\"cid:logo@host\"></p>\r\n--x\r\nContent-Type: image/png\r\nContent-ID: <logo@host>\r\nContent-Transfer-Encoding: base64\r\n\r\naGk=\r\n--x--\r\n";
        let html = document(raw).expect("a document");
        assert!(
            html.contains("src=\"data:image/png;base64,aGk=\""),
            "{html}"
        );
    }

    #[test]
    fn leaves_a_longer_cid_alone() {
        assert_eq!(
            replace_cid("<img src=\"CID:a\"><img src=\"cid:ab\">", "a", "data:x"),
            "<img src=\"data:x\"><img src=\"cid:ab\">"
        );
    }

    #[test]
    fn shows_a_plain_message_as_escaped_text() {
        let raw = b"From: a@example.com\r\nSubject: Hi\r\n\r\n1 < 2 & so on";
        let html = document(raw).expect("a document");
        assert!(html.contains("1 &lt; 2 &amp; so on"), "{html}");
    }
}
