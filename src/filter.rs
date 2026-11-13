// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use eframe::egui::{Button, RichText, Ui};
use zirbe_store::{Conversation, Message};

use crate::media;
use crate::palette::Palette;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Seen {
    #[default]
    Any,
    Unread,
    Read,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Body {
    #[default]
    Any,
    Plain,
    Html,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sender {
    #[default]
    Any,
    People,
    Lists,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Filter {
    pub seen: Seen,
    pub body: Body,
    pub sender: Sender,
    pub flagged: bool,
    pub attachments: bool,
    pub voice: bool,
    pub video: bool,
}

impl Filter {
    pub fn active(&self) -> bool {
        *self != Filter::default()
    }

    pub fn only_unread(&self) -> bool {
        *self
            == Filter {
                seen: Seen::Unread,
                ..Filter::default()
            }
    }

    pub fn admits(&self, conversation: &Conversation) -> bool {
        let messages = &conversation.messages;
        let any = |test: fn(&Message) -> bool| messages.iter().any(test);
        let seen = match self.seen {
            Seen::Any => true,
            Seen::Unread => conversation.unread > 0,
            Seen::Read => conversation.unread == 0,
        };
        let body = match self.body {
            Body::Any => true,
            Body::Plain => !any(|message| message.html),
            Body::Html => any(|message| message.html),
        };
        let sender = match self.sender {
            Sender::Any => true,
            Sender::People => !any(|message| message.list),
            Sender::Lists => any(|message| message.list),
        };
        seen && body
            && sender
            && (!self.flagged || conversation.flagged)
            && (!self.attachments || any(|message| !message.attachments.is_empty()))
            && (!self.voice || media::carries(conversation, media::is_audio))
            && (!self.video || media::carries(conversation, media::is_video))
    }

    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        match self.seen {
            Seen::Any => {}
            Seen::Unread => parts.push("unread"),
            Seen::Read => parts.push("read"),
        }
        match self.body {
            Body::Any => {}
            Body::Plain => parts.push("plain text"),
            Body::Html => parts.push("HTML"),
        }
        match self.sender {
            Sender::Any => {}
            Sender::People => parts.push("from people"),
            Sender::Lists => parts.push("from mailing lists"),
        }
        if self.flagged {
            parts.push("flagged");
        }
        if self.attachments {
            parts.push("with attachments");
        }
        if self.voice {
            parts.push("with voice messages");
        }
        if self.video {
            parts.push("with video");
        }
        if parts.is_empty() {
            "Filter the list".into()
        } else {
            format!("Showing {}", parts.join(", "))
        }
    }

    pub fn menu(&mut self, ui: &mut Ui, palette: &Palette) {
        ui.set_min_width(200.0);
        let heading = |ui: &mut Ui, text: &str| {
            ui.label(RichText::new(text).size(11.5).color(palette.secondary));
        };
        heading(ui, "Read");
        ui.radio_value(&mut self.seen, Seen::Any, "All");
        ui.radio_value(&mut self.seen, Seen::Unread, "Unread");
        ui.radio_value(&mut self.seen, Seen::Read, "Read");
        ui.separator();
        heading(ui, "Format");
        ui.radio_value(&mut self.body, Body::Any, "All");
        ui.radio_value(&mut self.body, Body::Plain, "Plain text");
        ui.radio_value(&mut self.body, Body::Html, "HTML");
        ui.separator();
        heading(ui, "Sender");
        ui.radio_value(&mut self.sender, Sender::Any, "All");
        ui.radio_value(&mut self.sender, Sender::People, "People");
        ui.radio_value(&mut self.sender, Sender::Lists, "Mailing lists");
        ui.separator();
        ui.checkbox(&mut self.flagged, "Flagged");
        ui.checkbox(&mut self.attachments, "Has attachments");
        ui.checkbox(&mut self.voice, "Voice messages");
        ui.checkbox(&mut self.video, "Video messages");
        ui.separator();
        if ui
            .add_enabled(self.active(), Button::new("Clear Filter"))
            .clicked()
        {
            *self = Filter::default();
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use zirbe_store::Person;

    fn message(from: &str, html: bool, list: bool) -> Message {
        Message {
            mailbox: "INBOX".into(),
            uid: 1,
            message_id: None,
            in_reply_to: None,
            references: Vec::new(),
            from: Some(Person {
                name: None,
                address: from.into(),
            }),
            to: Vec::new(),
            cc: Vec::new(),
            subject: "Hi".into(),
            date: Utc::now(),
            text: String::new(),
            quoted: false,
            transcript: None,
            attachments: Vec::new(),
            seen: true,
            partial: false,
            html,
            list,
            flagged: false,
            reaction: None,
            invite: None,
        }
    }

    fn conversation(messages: Vec<Message>, unread: usize) -> Conversation {
        Conversation {
            key: "key".into(),
            account: "me@example.com".into(),
            title: "Hi".into(),
            members: Vec::new(),
            messages,
            starts_with_context: false,
            unread,
            in_inbox: true,
            flagged: false,
            pinned: false,
            copies: Vec::new(),
        }
    }

    #[test]
    fn an_empty_filter_admits_everything() {
        let filter = Filter::default();
        assert!(!filter.active());
        assert!(filter.admits(&conversation(vec![message("a@example.com", true, true)], 0)));
    }

    #[test]
    fn plain_text_means_no_message_has_html() {
        let plain = Filter {
            body: Body::Plain,
            ..Filter::default()
        };
        let mixed = conversation(
            vec![
                message("a@example.com", false, false),
                message("b@example.com", true, false),
            ],
            0,
        );
        assert!(!plain.admits(&mixed));
        assert!(plain.admits(&conversation(
            vec![message("a@example.com", false, false)],
            0
        )));
    }

    #[test]
    fn voice_and_video_look_at_the_attachments() {
        let mut memo = message("a@example.com", false, false);
        memo.attachments = vec!["Voice message.m4a".into()];
        let mut clip = message("b@example.com", false, false);
        clip.attachments = vec!["IMG_0042.MOV".into()];
        let voice = Filter {
            voice: true,
            ..Filter::default()
        };
        let video = Filter {
            video: true,
            ..Filter::default()
        };
        assert!(voice.admits(&conversation(vec![memo.clone()], 0)));
        assert!(!voice.admits(&conversation(vec![clip.clone()], 0)));
        assert!(video.admits(&conversation(vec![clip], 0)));
        assert!(!video.admits(&conversation(vec![memo], 0)));
    }

    #[test]
    fn criteria_combine() {
        let filter = Filter {
            seen: Seen::Unread,
            sender: Sender::Lists,
            ..Filter::default()
        };
        let news = vec![message("news@example.com", true, true)];
        assert!(filter.admits(&conversation(news.clone(), 1)));
        assert!(!filter.admits(&conversation(news, 0)));
        assert!(!filter.admits(&conversation(
            vec![message("a@example.com", false, false)],
            1
        )));
    }
}
