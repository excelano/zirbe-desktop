// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::model::{Conversation, Message};

pub fn fold(text: &str) -> String {
    text.nfd()
        .filter(|c| !is_combining_mark(*c))
        .flat_map(char::to_lowercase)
        .collect()
}

fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
}

pub struct Query {
    terms: Vec<String>,
}

impl Query {
    pub fn new(text: &str) -> Query {
        let folded = fold(text);
        Query {
            terms: words(&folded).map(str::to_string).collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn matches_text(&self, text: &str) -> bool {
        let folded = fold(text);
        let found: Vec<&str> = words(&folded).collect();
        self.terms
            .iter()
            .all(|term| found.iter().any(|word| word.starts_with(term.as_str())))
    }

    pub fn matches_message(&self, message: &Message) -> bool {
        self.matches_text(&searchable(message))
    }

    pub fn matches(&self, conversation: &Conversation) -> bool {
        let mut text = conversation.title.clone();
        for person in &conversation.members {
            text.push(' ');
            text.push_str(person.name.as_deref().unwrap_or_default());
            text.push(' ');
            text.push_str(&person.address);
        }
        for message in &conversation.messages {
            text.push(' ');
            text.push_str(&searchable(message));
        }
        self.matches_text(&text)
    }
}

fn searchable(message: &Message) -> String {
    let mut text = message.subject.clone();
    if let Some(from) = &message.from {
        text.push(' ');
        text.push_str(from.name.as_deref().unwrap_or_default());
        text.push(' ');
        text.push_str(&from.address);
    }
    text.push(' ');
    text.push_str(&message.text);
    for name in &message.attachments {
        text.push(' ');
        text.push_str(name);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::Query;

    #[test]
    fn matches_whole_words_and_word_starts_ignoring_case_and_accents() {
        let text = "Thanks, Zoë: the Café menu for Grüße is attached.";
        assert!(Query::new("zoe").matches_text(text));
        assert!(Query::new("CAFE men").matches_text(text));
        assert!(!Query::new("grusse").matches_text(text));
        assert!(Query::new("gru").matches_text(text));
        assert!(!Query::new("enu").matches_text(text));
        assert!(!Query::new("menu invoice").matches_text(text));
    }
}
