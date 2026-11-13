// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use zirbe_store::Conversation;

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

pub fn carries(conversation: &Conversation, test: fn(&str) -> bool) -> bool {
    conversation
        .messages
        .iter()
        .flat_map(|message| &message.attachments)
        .any(|name| test(name))
}

pub fn glyph(conversation: &Conversation) -> &'static str {
    if carries(conversation, is_video) {
        "🎥 "
    } else if carries(conversation, is_audio) {
        "🎤 "
    } else {
        ""
    }
}
