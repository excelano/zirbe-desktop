// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::process::Command;
use std::sync::mpsc;

use zirbe_store::{Conversation, Reply};

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

pub fn poster(name: &str, bytes: Vec<u8>) -> Reply<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    let extension = name
        .rsplit_once('.')
        .map_or("mp4", |(_, extension)| extension)
        .to_string();
    std::thread::spawn(move || {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let source = std::env::temp_dir().join(format!(
            "zirbe-poster-{}-{stamp}.{extension}",
            std::process::id()
        ));
        let frame = std::fs::write(&source, &bytes)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                Command::new("ffmpeg")
                    .args(["-loglevel", "error", "-i"])
                    .arg(&source)
                    .args([
                        "-frames:v",
                        "1",
                        "-vf",
                        "scale=480:-2",
                        "-f",
                        "image2",
                        "-c:v",
                        "png",
                        "pipe:1",
                    ])
                    .output()
                    .map_err(|error| format!("ffmpeg did not start: {error}"))
            })
            .and_then(|output| {
                if output.status.success() && !output.stdout.is_empty() {
                    Ok(output.stdout)
                } else {
                    Err("no frame".into())
                }
            });
        let _ = std::fs::remove_file(&source);
        let _ = sender.send(frame);
    });
    receiver
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
