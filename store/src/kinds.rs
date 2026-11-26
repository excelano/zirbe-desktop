// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Kind {
    PlainText,
    FormattedText,
    Pictures,
    Audio,
    Video,
    OtherAttachments,
    Invitations,
    InvitationReplies,
    Reactions,
    Newsletters,
    Notices,
    AnythingElse,
}

impl Kind {
    pub const ALL: [Kind; 12] = [
        Kind::PlainText,
        Kind::FormattedText,
        Kind::Pictures,
        Kind::Audio,
        Kind::Video,
        Kind::OtherAttachments,
        Kind::Invitations,
        Kind::InvitationReplies,
        Kind::Reactions,
        Kind::Newsletters,
        Kind::Notices,
        Kind::AnythingElse,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Kind::PlainText => "Plain text",
            Kind::FormattedText => "Formatted text",
            Kind::Pictures => "Pictures",
            Kind::Audio => "Audio",
            Kind::Video => "Video",
            Kind::OtherAttachments => "Other attachments",
            Kind::Invitations => "Invitations",
            Kind::InvitationReplies => "Invitation replies",
            Kind::Reactions => "Reactions",
            Kind::Newsletters => "Newsletters and lists",
            Kind::Notices => "Automatic notices",
            Kind::AnythingElse => "Anything else",
        }
    }
}

#[derive(Clone)]
pub(crate) struct Part {
    pub mime: String,
    pub name: Option<String>,
    pub embedded: bool,
    pub size: usize,
    pub method: Option<String>,
}

pub(crate) struct Facts {
    pub text: bool,
    pub html: bool,
    pub newsletter: bool,
    pub notice: bool,
    pub encrypted: bool,
    pub reaction: bool,
    pub parts: Vec<Part>,
}

const EMBEDDED_PICTURE_LIMIT: usize = 100_000;

const SIGNATURES: [&str; 3] = [
    "application/pgp-signature",
    "application/pkcs7-signature",
    "application/x-pkcs7-signature",
];

pub(crate) fn is_smime(mime: &str) -> bool {
    matches!(mime, "application/pkcs7-mime" | "application/x-pkcs7-mime")
}

pub(crate) fn is_encrypted_smime(smime_type: Option<&str>) -> bool {
    smime_type.is_none_or(|kind| {
        !kind.eq_ignore_ascii_case("signed-data") && !kind.eq_ignore_ascii_case("certs-only")
    })
}

fn ends_with_any(name: &str, extensions: &[&str]) -> bool {
    let lower = name.to_ascii_lowercase();
    extensions
        .iter()
        .any(|extension| lower.ends_with(extension))
}

pub fn is_audio(name: &str) -> bool {
    ends_with_any(
        name,
        &[
            ".m4a", ".mp3", ".aac", ".wav", ".ogg", ".oga", ".opus", ".flac",
        ],
    )
}

pub fn is_video(name: &str) -> bool {
    ends_with_any(name, &[".mp4", ".mov", ".m4v", ".webm", ".3gp", ".mkv"])
}

fn is_picture(name: &str) -> bool {
    ends_with_any(
        name,
        &[
            ".jpg", ".jpeg", ".png", ".gif", ".webp", ".heic", ".heif", ".bmp", ".tiff",
        ],
    )
}

fn is_calendar_file(part: &Part, name: &str) -> bool {
    part.mime == "application/ics" || ends_with_any(name, &[".ics"])
}

fn kind_of(part: &Part, has_calendar: bool) -> Option<Kind> {
    let name = part.name.as_deref().unwrap_or("");
    if SIGNATURES.contains(&part.mime.as_str()) {
        None
    } else if part.mime == "text/calendar" {
        Some(match part.method.as_deref() {
            Some("REPLY" | "COUNTER") => Kind::InvitationReplies,
            _ => Kind::Invitations,
        })
    } else if is_calendar_file(part, name) {
        (!has_calendar).then_some(Kind::Invitations)
    } else if part.mime.starts_with("image/") || is_picture(name) {
        (!(part.embedded && part.size < EMBEDDED_PICTURE_LIMIT)).then_some(Kind::Pictures)
    } else if part.mime.starts_with("audio/") || is_audio(name) {
        Some(Kind::Audio)
    } else if part.mime.starts_with("video/") || is_video(name) {
        Some(Kind::Video)
    } else {
        Some(Kind::OtherAttachments)
    }
}

pub(crate) fn classify(facts: &Facts) -> Vec<Kind> {
    if facts.encrypted {
        return vec![Kind::AnythingElse];
    }
    if facts.notice {
        return vec![Kind::Notices];
    }
    let has_calendar = facts.parts.iter().any(|part| part.mime == "text/calendar");
    let mut kinds: Vec<Kind> = facts
        .parts
        .iter()
        .filter_map(|part| kind_of(part, has_calendar))
        .collect();
    if facts.reaction {
        kinds.push(Kind::Reactions);
    }
    if facts.newsletter {
        kinds.push(Kind::Newsletters);
    }
    if facts.text
        && !facts.reaction
        && !facts.newsletter
        && !kinds
            .iter()
            .any(|kind| matches!(kind, Kind::Invitations | Kind::InvitationReplies))
    {
        kinds.push(if facts.html {
            Kind::FormattedText
        } else {
            Kind::PlainText
        });
    }
    kinds.sort();
    kinds.dedup();
    if kinds.is_empty() {
        kinds.push(Kind::AnythingElse);
    }
    kinds
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            text: true,
            html: false,
            newsletter: false,
            notice: false,
            encrypted: false,
            reaction: false,
            parts: Vec::new(),
        }
    }

    fn part(mime: &str, name: Option<&str>) -> Part {
        Part {
            mime: mime.to_string(),
            name: name.map(str::to_string),
            embedded: false,
            size: 1_000,
            method: None,
        }
    }

    fn image(embedded: bool, size: usize) -> Part {
        Part {
            embedded,
            size,
            ..part("image/png", None)
        }
    }

    fn calendar(method: Option<&str>) -> Part {
        Part {
            method: method.map(str::to_string),
            ..part("text/calendar", None)
        }
    }

    #[test]
    fn plain_text_message() {
        assert_eq!(classify(&facts()), vec![Kind::PlainText]);
        let formatted = Facts {
            html: true,
            ..facts()
        };
        assert_eq!(classify(&formatted), vec![Kind::FormattedText]);
    }

    #[test]
    fn text_with_a_voice_clip() {
        let message = Facts {
            parts: vec![part("audio/mp4", Some("voice.m4a"))],
            ..facts()
        };
        assert_eq!(classify(&message), vec![Kind::PlainText, Kind::Audio]);
    }

    #[test]
    fn signed_mail_is_only_text() {
        let message = Facts {
            parts: vec![
                part("application/pgp-signature", None),
                part("application/pkcs7-signature", None),
            ],
            ..facts()
        };
        assert_eq!(classify(&message), vec![Kind::PlainText]);
    }

    #[test]
    fn small_embedded_image_is_not_a_picture() {
        let message = Facts {
            html: true,
            parts: vec![image(true, 8_000)],
            ..facts()
        };
        assert_eq!(classify(&message), vec![Kind::FormattedText]);
    }

    #[test]
    fn large_embedded_image_is_a_picture() {
        let message = Facts {
            html: true,
            parts: vec![image(true, 2_000_000)],
            ..facts()
        };
        assert_eq!(
            classify(&message),
            vec![Kind::FormattedText, Kind::Pictures]
        );
    }

    #[test]
    fn attached_image_is_a_picture_of_any_size() {
        let message = Facts {
            parts: vec![image(false, 8_000)],
            ..facts()
        };
        assert!(classify(&message).contains(&Kind::Pictures));
    }

    #[test]
    fn invitations_are_not_text() {
        let reply = Facts {
            parts: vec![calendar(Some("REPLY"))],
            ..facts()
        };
        assert_eq!(classify(&reply), vec![Kind::InvitationReplies]);
        let request = Facts {
            html: true,
            parts: vec![calendar(Some("REQUEST"))],
            ..facts()
        };
        assert_eq!(classify(&request), vec![Kind::Invitations]);
        let unknown = Facts {
            parts: vec![calendar(None)],
            ..facts()
        };
        assert!(classify(&unknown).contains(&Kind::Invitations));
    }

    #[test]
    fn newsletter_is_not_text() {
        let newsletter = Facts {
            newsletter: true,
            ..facts()
        };
        assert_eq!(classify(&newsletter), vec![Kind::Newsletters]);
        let with_pdf = Facts {
            newsletter: true,
            parts: vec![part("application/pdf", Some("issue.pdf"))],
            ..facts()
        };
        assert_eq!(
            classify(&with_pdf),
            vec![Kind::OtherAttachments, Kind::Newsletters]
        );
    }

    #[test]
    fn reaction_is_only_a_reaction() {
        let message = Facts {
            reaction: true,
            ..facts()
        };
        assert_eq!(classify(&message), vec![Kind::Reactions]);
    }

    #[test]
    fn notice_is_exclusive() {
        let message = Facts {
            notice: true,
            parts: vec![part("application/pdf", Some("receipt.pdf"))],
            ..facts()
        };
        assert_eq!(classify(&message), vec![Kind::Notices]);
    }

    #[test]
    fn encrypted_is_anything_else() {
        let message = Facts {
            encrypted: true,
            parts: vec![part("application/octet-stream", None)],
            ..facts()
        };
        assert_eq!(classify(&message), vec![Kind::AnythingElse]);
    }

    #[test]
    fn empty_message_is_anything_else() {
        let message = Facts {
            text: false,
            ..facts()
        };
        assert_eq!(classify(&message), vec![Kind::AnythingElse]);
    }
}
