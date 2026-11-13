// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use rodio::microphone::MicrophoneBuilder;
use zirbe_store::Outgoing;

use crate::transcribe;

const RATE: u32 = 16_000;
const QUIET: u16 = 330;
pub const LONGEST: Duration = Duration::from_secs(10 * 60);

pub struct Clip {
    pub wav: Arc<Vec<u8>>,
    pub length: Duration,
}

pub struct Recording {
    started: Instant,
    stop: Arc<AtomicBool>,
    level: Arc<Mutex<f32>>,
    done: Receiver<Result<Clip, String>>,
}

impl Recording {
    pub fn start() -> Recording {
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(Mutex::new(0.0));
        let (sender, done) = mpsc::channel();
        let (flag, meter) = (stop.clone(), level.clone());
        std::thread::spawn(move || {
            let _ = sender.send(capture(&flag, &meter));
        });
        Recording {
            started: Instant::now(),
            stop,
            level,
            done,
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    pub fn level(&self) -> f32 {
        *self.level.lock().expect("level lock")
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    pub fn finished(&self) -> Option<Result<Clip, String>> {
        self.done.try_recv().ok()
    }
}

impl Drop for Recording {
    fn drop(&mut self) {
        self.stop();
    }
}

fn capture(stop: &AtomicBool, level: &Mutex<f32>) -> Result<Clip, String> {
    let microphone = MicrophoneBuilder::new()
        .default_device()
        .and_then(|builder| builder.default_config())
        .map_err(|error| format!("no microphone: {error}"))?
        .prefer_channel_counts([1.try_into().expect("not zero")])
        .prefer_sample_rates([RATE.try_into().expect("not zero")])
        .open_stream()
        .map_err(|error| format!("the microphone did not open: {error}"))?;
    let channels = usize::from(microphone.config().channel_count.get());
    let rate = microphone.config().sample_rate.get();
    let started = Instant::now();
    let mut mono = Vec::new();
    let mut frame = Vec::with_capacity(channels);
    let mut peak = 0.0_f32;
    for sample in microphone {
        frame.push(sample);
        if frame.len() < channels {
            continue;
        }
        let value = frame.iter().sum::<f32>() / channels as f32;
        frame.clear();
        mono.push(value);
        peak = peak.max(value.abs());
        if mono.len() % (rate as usize / 20) == 0 {
            *level.lock().expect("level lock") = peak;
            peak = 0.0;
            if stop.load(Ordering::Relaxed) || started.elapsed() >= LONGEST {
                break;
            }
        }
    }
    let samples = resample(&mono, rate, RATE);
    Ok(Clip {
        length: Duration::from_secs_f64(samples.len() as f64 / f64::from(RATE)),
        wav: Arc::new(wav(&samples)),
    })
}

fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let step = f64::from(from) / f64::from(to);
    let count = (samples.len() as f64 / step) as usize;
    (0..count)
        .map(|index| {
            let at = index as f64 * step;
            let base = at as usize;
            let next = samples.get(base + 1).copied().unwrap_or(samples[base]);
            let part = (at - base as f64) as f32;
            samples[base] * (1.0 - part) + next * part
        })
        .collect()
}

fn wav(samples: &[f32]) -> Vec<u8> {
    let data = (samples.len() * 2) as u32;
    let mut bytes = Vec::with_capacity(44 + data as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data.to_le_bytes());
    for sample in samples {
        let value = (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)) as i16;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes
}

fn audible(wav: &[u8]) -> bool {
    wav.get(44..)
        .unwrap_or_default()
        .as_chunks::<2>()
        .0
        .iter()
        .any(|pair| i16::from_le_bytes(*pair).unsigned_abs() > QUIET)
}

pub struct Prepared {
    pub file: Outgoing,
    pub transcript: String,
}

pub fn prepare(wav: Arc<Vec<u8>>, words: Vec<String>) -> Receiver<Result<Prepared, String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(prepared(&wav, &words));
    });
    receiver
}

fn prepared(wav: &[u8], words: &[String]) -> Result<Prepared, String> {
    let work = std::env::temp_dir().join(format!("zirbe-voice-{}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|error| error.to_string())?;
    let source = work.join("recording.wav");
    let encoded = work.join("recording.m4a");
    std::fs::write(&source, wav).map_err(|error| error.to_string())?;
    let transcript = transcribe::setup()
        .filter(|_| audible(wav))
        .and_then(|setup| transcribe::transcribe(&setup, &source, words).ok())
        .unwrap_or_default();
    let aac = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-i"])
        .arg(&source)
        .args([
            "-ac",
            "1",
            "-c:a",
            "aac",
            "-b:a",
            "48k",
            "-movflags",
            "+faststart",
        ])
        .arg(&encoded)
        .status()
        .is_ok_and(|status| status.success())
        .then(|| std::fs::read(&encoded).ok())
        .flatten();
    let _ = std::fs::remove_dir_all(&work);
    let file = match aac {
        Some(bytes) => Outgoing {
            name: "Voice Message.m4a".into(),
            bytes,
        },
        None => Outgoing {
            name: "Voice Message.wav".into(),
            bytes: wav.to_vec(),
        },
    };
    Ok(Prepared { file, transcript })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_the_length_in_time() {
        let second: Vec<f32> = vec![0.25; 48_000];
        assert_eq!(resample(&second, 48_000, RATE).len(), 16_000);
        assert_eq!(resample(&second, RATE, RATE).len(), 48_000);
    }

    #[test]
    fn silence_is_not_audible() {
        assert!(!audible(&wav(&[0.0, 0.004, -0.005])));
        assert!(audible(&wav(&[0.0, 0.2, -0.1])));
    }

    #[test]
    fn the_wav_header_names_sixteen_kilohertz_mono() {
        let bytes = wav(&[0.0, 0.5, -0.5]);
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1);
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            RATE
        );
        assert_eq!(bytes.len(), 44 + 6);
    }
}
