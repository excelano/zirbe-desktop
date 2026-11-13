// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use rodio::{Decoder, MixerDeviceSink, Player, Source};

pub fn is_audio(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".m4a", ".mp3", ".aac", ".wav", ".ogg", ".oga", ".flac"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

pub struct Sound<'a> {
    pub key: &'a str,
    pub loading: bool,
    pub playing: bool,
    pub position: Duration,
    pub duration: Option<Duration>,
}

struct Track {
    key: String,
    name: String,
    bytes: Arc<Vec<u8>>,
    duration: Option<Duration>,
    player: Player,
}

#[derive(Default)]
pub struct Audio {
    output: Option<MixerDeviceSink>,
    track: Option<Track>,
    loading: Option<String>,
}

impl Audio {
    pub fn sound(&self) -> Option<Sound<'_>> {
        if let Some(key) = &self.loading {
            return Some(Sound {
                key,
                loading: true,
                playing: false,
                position: Duration::ZERO,
                duration: None,
            });
        }
        let track = self.track.as_ref()?;
        let finished = track.player.empty();
        Some(Sound {
            key: &track.key,
            loading: false,
            playing: !finished && !track.player.is_paused(),
            position: if finished {
                Duration::ZERO
            } else {
                track.player.get_pos()
            },
            duration: track.duration,
        })
    }

    pub fn is_busy(&self) -> bool {
        self.sound()
            .is_some_and(|sound| sound.playing || sound.loading)
    }

    pub fn toggle(&mut self, key: &str) -> bool {
        let Some(track) = self.track.as_ref().filter(|track| track.key == key) else {
            return false;
        };
        if track.player.empty() {
            let (key, name, bytes) = (track.key.clone(), track.name.clone(), track.bytes.clone());
            return self.play(&key, &name, bytes).is_ok();
        }
        if track.player.is_paused() {
            track.player.play();
        } else {
            track.player.pause();
        }
        true
    }

    pub fn loading(&mut self, key: &str) {
        self.stop();
        self.loading = Some(key.to_string());
    }

    pub fn play(&mut self, key: &str, name: &str, bytes: Arc<Vec<u8>>) -> Result<(), String> {
        self.stop();
        if self.output.is_none() {
            let mut output = rodio::DeviceSinkBuilder::open_default_sink()
                .map_err(|error| format!("no sound output: {error}"))?;
            output.log_on_drop(false);
            self.output = Some(output);
        }
        let output = self.output.as_ref().expect("opened above");
        let hint = name
            .rsplit('.')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let decoder = Decoder::builder()
            .with_byte_len(bytes.len() as u64)
            .with_data(Cursor::new(Shared(bytes.clone())))
            .with_seekable(true)
            .with_hint(&hint)
            .build()
            .map_err(|error| error.to_string())?;
        let duration = decoder.total_duration();
        let player = Player::connect_new(output.mixer());
        player.append(decoder);
        self.track = Some(Track {
            key: key.to_string(),
            name: name.to_string(),
            bytes,
            duration,
            player,
        });
        Ok(())
    }

    pub fn seek(&mut self, key: &str, fraction: f32) {
        if let Some(track) = self.track.as_ref().filter(|track| track.key == key)
            && let Some(duration) = track.duration
            && !track.player.empty()
        {
            let _ = track
                .player
                .try_seek(duration.mul_f32(fraction.clamp(0.0, 1.0)));
        }
    }

    pub fn stop(&mut self) {
        self.loading = None;
        if let Some(track) = self.track.take() {
            track.player.stop();
        }
    }
}

struct Shared(Arc<Vec<u8>>);

impl AsRef<[u8]> for Shared {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}
