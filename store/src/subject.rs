// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

const REPLY: &[&str] = &["antw", "odp", "ynt", "rif", "re", "aw", "sv", "vs"];
const FORWARD: &[&str] = &["fwd", "fw", "wg"];

pub fn normalize(subject: &str) -> String {
    prefixes(subject).1.to_string()
}

pub fn is_reply(subject: &str) -> bool {
    prefixes(subject).0 == Some(true)
}

fn prefixes(subject: &str) -> (Option<bool>, &str) {
    let mut rest = subject.trim();
    let mut first = None;
    loop {
        let lower = rest.to_ascii_lowercase();
        let found = FORWARD
            .iter()
            .map(|prefix| (prefix, false))
            .chain(REPLY.iter().map(|prefix| (prefix, true)))
            .find(|(prefix, _)| lower.starts_with(**prefix));
        let Some((prefix, reply)) = found else { break };
        let mut at = prefix.len();
        if rest[at..].starts_with('[')
            && let Some(close) = rest[at..].find(']')
            && rest[at + 1..at + close].chars().all(|c| c.is_ascii_digit())
        {
            at += close + 1;
        }
        if !rest[at..].starts_with(':') {
            break;
        }
        first.get_or_insert(reply);
        rest = rest[at + 1..].trim_start();
    }
    (first, rest)
}

#[cfg(test)]
mod tests {
    use super::{is_reply, normalize};

    #[test]
    fn tells_a_reply_from_a_forward() {
        assert!(is_reply("Re: Lunch"));
        assert!(is_reply("SV: Möte"));
        assert!(is_reply("Antw: Afspraak"));
        assert!(!is_reply("Fwd: Re: Lunch"));
        assert!(!is_reply("Lunch"));
        assert!(!is_reply("Review: draft"));
        assert_eq!(normalize("SV: VS: Möte"), "Möte");
    }

    #[test]
    fn strips_repeated_prefixes() {
        assert_eq!(normalize("Re: RE: Fwd: Lunch"), "Lunch");
        assert_eq!(normalize("Re[2]: Lunch"), "Lunch");
        assert_eq!(normalize("AW: Termin"), "Termin");
    }

    #[test]
    fn keeps_words_that_start_like_a_prefix() {
        assert_eq!(normalize("Review: draft"), "Review: draft");
        assert_eq!(normalize("Fwd lunch"), "Fwd lunch");
        assert_eq!(normalize("   "), "");
    }
}
