// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

pub fn normalize(subject: &str) -> String {
    let mut rest = subject.trim();
    loop {
        let lower = rest.to_ascii_lowercase();
        let prefix = ["re", "fwd", "fw", "aw", "wg"]
            .iter()
            .find(|prefix| lower.starts_with(*prefix))
            .map(|prefix| prefix.len());
        let Some(mut at) = prefix else { break };
        if rest[at..].starts_with('[')
            && let Some(close) = rest[at..].find(']')
            && rest[at + 1..at + close].chars().all(|c| c.is_ascii_digit())
        {
            at += close + 1;
        }
        if !rest[at..].starts_with(':') {
            break;
        }
        rest = rest[at + 1..].trim_start();
    }
    rest.to_string()
}

#[cfg(test)]
mod tests {
    use super::normalize;

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
