// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use crate::model::{Invite, When};

const MEETINGS: [&str; 4] = [
    "teams.microsoft.com",
    "meet.google.com",
    "zoom.us",
    "webex.com",
];

fn unfold(ics: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for line in ics.lines() {
        match (line.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(line.to_string()),
        }
    }
    lines
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

struct Line<'a> {
    name: String,
    params: Vec<(String, &'a str)>,
    value: &'a str,
}

fn split(line: &str) -> Option<Line<'_>> {
    let mut quoted = false;
    let colon = line.char_indices().find_map(|(at, c)| {
        if c == '"' {
            quoted = !quoted;
        }
        (c == ':' && !quoted).then_some(at)
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.to_ascii_uppercase();
    let params = parts
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| (key.to_ascii_uppercase(), value.trim_matches('"')))
        .collect();
    Some(Line {
        name,
        params,
        value,
    })
}

fn param<'a>(line: &Line<'a>, key: &str) -> Option<&'a str> {
    line.params
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| *value)
}

fn meeting_link(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '<' | '>' | '"' | '(' | ')'))
        .filter(|word| word.starts_with("https://"))
        .find(|word| MEETINGS.iter().any(|host| word.contains(host)))
        .map(|word| word.trim_end_matches(['.', ',', ';']).to_string())
}

pub(crate) fn parse(ics: &str) -> Option<Invite> {
    let mut invite = Invite {
        ics: ics.to_string(),
        ..Invite::default()
    };
    let mut in_event = false;
    let mut seen_event = false;
    let mut link_sources = Vec::new();
    for raw in unfold(ics) {
        let Some(line) = split(&raw) else {
            continue;
        };
        match (line.name.as_str(), line.value) {
            ("METHOD", method) => invite.method = method.trim().to_ascii_uppercase(),
            ("BEGIN", "VEVENT") if !seen_event => in_event = true,
            ("END", "VEVENT") if in_event => {
                in_event = false;
                seen_event = true;
            }
            _ if !in_event => {}
            ("SUMMARY", value) => invite.summary = unescape(value),
            ("LOCATION", value) => {
                invite.location = unescape(value);
                link_sources.push(invite.location.clone());
            }
            ("DESCRIPTION", value) => link_sources.push(unescape(value)),
            ("URL" | "X-MICROSOFT-SKYPETEAMSMEETINGURL" | "X-GOOGLE-CONFERENCE", value) => {
                link_sources.insert(0, unescape(value))
            }
            ("DTSTART", value) => {
                invite.start = Some(When {
                    value: value.trim().to_string(),
                    zone: param(&line, "TZID").map(str::to_string),
                })
            }
            ("DTEND", value) => {
                invite.end = Some(When {
                    value: value.trim().to_string(),
                    zone: param(&line, "TZID").map(str::to_string),
                })
            }
            ("ORGANIZER", value) => {
                invite.organizer = param(&line, "CN")
                    .map(str::to_string)
                    .unwrap_or_else(|| value.trim_start_matches("mailto:").to_string())
            }
            ("SEQUENCE", value) => invite.sequence = value.trim().parse().unwrap_or(0),
            ("STATUS", value) => invite.cancelled |= value.trim().eq_ignore_ascii_case("CANCELLED"),
            ("ATTENDEE", value) if invite.method == "REPLY" => {
                let who = param(&line, "CN")
                    .map(str::to_string)
                    .unwrap_or_else(|| value.trim_start_matches("mailto:").to_string());
                if let Some(answer) = param(&line, "PARTSTAT") {
                    invite.answer = Some((who, answer.to_ascii_uppercase()));
                }
            }
            _ => {}
        }
    }
    if !seen_event {
        return None;
    }
    invite.cancelled |= invite.method == "CANCEL";
    invite.join = link_sources.iter().find_map(|text| meeting_link(text));
    Some(invite)
}

#[cfg(test)]
mod tests {
    use super::parse;

    const REQUEST: &str = "BEGIN:VCALENDAR\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nSUMMARY:Troop committee\\, November\r\nDTSTART;TZID=Central Standard Time:20261115T190000\r\nDTEND;TZID=Central Standard Time:20261115T200000\r\nORGANIZER;CN=\"Harris, J.R.\":mailto:jr@example.com\r\nLOCATION:Room 4\r\nDESCRIPTION:Join here: https://teams.microsoft.com/l/meetup-join/abc\r\n 123 or call in.\r\nSEQUENCE:1\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[test]
    fn reads_an_invitation() {
        let invite = parse(REQUEST).expect("an invite");
        assert_eq!(invite.summary, "Troop committee, November");
        assert_eq!(invite.organizer, "Harris, J.R.");
        assert_eq!(invite.location, "Room 4");
        assert_eq!(
            invite.join.as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/abc123")
        );
        let start = invite.start.expect("a start");
        assert_eq!(start.value, "20261115T190000");
        assert_eq!(start.zone.as_deref(), Some("Central Standard Time"));
        assert_eq!(invite.sequence, 1);
        assert!(!invite.cancelled);
    }

    #[test]
    fn reads_a_reply_and_a_cancellation() {
        let reply = "BEGIN:VCALENDAR\nMETHOD:REPLY\nBEGIN:VEVENT\nATTENDEE;PARTSTAT=ACCEPTED;CN=Ana Lopez:mailto:ana@example.com\nSUMMARY:Lunch\nDTSTART:20261116T170000Z\nEND:VEVENT\nEND:VCALENDAR\n";
        let invite = parse(reply).expect("a reply");
        assert_eq!(
            invite.answer,
            Some(("Ana Lopez".to_string(), "ACCEPTED".to_string()))
        );
        let cancel = reply.replace("METHOD:REPLY", "METHOD:CANCEL");
        assert!(parse(&cancel).expect("a cancellation").cancelled);
    }
}
