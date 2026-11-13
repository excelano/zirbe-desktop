// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};

pub const INSTALL: &str = "sudo apt install whisper.cpp-tools whisper.cpp-model-small";
const MODEL: &str = "whisper.cpp/ggml-small.bin";
const VAD: &str = "whisper.cpp/ggml-silero.bin";

pub struct Setup {
    model: PathBuf,
    vad: Option<PathBuf>,
}

pub fn setup() -> Option<Setup> {
    let dirs = data_dirs(
        std::env::var_os("XDG_DATA_HOME"),
        std::env::var_os("XDG_DATA_DIRS"),
    );
    on_path("whisper-cli").then_some(())?;
    Some(Setup {
        model: find(&dirs, MODEL)?,
        vad: find(&dirs, VAD),
    })
}

fn data_dirs(home: Option<std::ffi::OsString>, system: Option<std::ffi::OsString>) -> Vec<PathBuf> {
    let home = home
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".local/share")));
    let system = system
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter()
        .chain(std::env::split_paths(&system))
        .collect()
}

fn find(dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn on_path(command: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(command).is_file()))
}

fn language(values: [Option<String>; 3]) -> String {
    values
        .into_iter()
        .flatten()
        .find(|value| !value.is_empty())
        .and_then(|value| {
            let code = value.split(['_', '.', '@']).next().unwrap_or_default();
            let valid =
                (2..=3).contains(&code.len()) && code.chars().all(|c| c.is_ascii_lowercase());
            valid.then(|| code.to_string())
        })
        .unwrap_or_else(|| "auto".into())
}

pub fn transcribe(setup: &Setup, audio: &Path, words: &[String]) -> Result<String, String> {
    let locale = ["LC_ALL", "LC_MESSAGES", "LANG"].map(|name| std::env::var(name).ok());
    let mut command = Command::new("whisper-cli");
    command.arg("-m").arg(&setup.model).args([
        "-l",
        &language(locale),
        "-nt",
        "-np",
        "-mc",
        "0",
        "-sns",
    ]);
    if let Some(vad) = &setup.vad {
        command.arg("--vad").arg("-vm").arg(vad);
    }
    if !words.is_empty() {
        command.arg("--prompt").arg(words.join(", "));
    }
    let output = command
        .arg("-f")
        .arg(audio)
        .output()
        .map_err(|error| format!("whisper-cli did not start: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(clean(&String::from_utf8_lossy(&output.stdout)))
}

pub fn received(
    name: String,
    bytes: Vec<u8>,
    words: Vec<String>,
) -> Receiver<Result<String, String>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(heard(&name, &bytes, &words));
    });
    receiver
}

fn heard(name: &str, bytes: &[u8], words: &[String]) -> Result<String, String> {
    let setup = setup().ok_or("transcription is not installed")?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let work = std::env::temp_dir().join(format!("zirbe-heard-{}-{stamp}", std::process::id()));
    std::fs::create_dir_all(&work).map_err(|error| error.to_string())?;
    let extension = name
        .rsplit_once('.')
        .map_or("bin", |(_, extension)| extension);
    let source = work.join(format!("received.{extension}"));
    let speech = work.join("speech.wav");
    let result = std::fs::write(&source, bytes)
        .map_err(|error| error.to_string())
        .and_then(|()| {
            let converted = Command::new("ffmpeg")
                .args(["-loglevel", "error", "-y", "-i"])
                .arg(&source)
                .args(["-vn", "-ac", "1", "-ar", "16000"])
                .arg(&speech)
                .status()
                .map_err(|error| format!("ffmpeg did not start: {error}"))?;
            if !converted.success() {
                return Err("ffmpeg could not read the recording".into());
            }
            transcribe(&setup, &speech, words)
        });
    let _ = std::fs::remove_dir_all(&work);
    result
}

fn clean(output: &str) -> String {
    let mut kept = String::with_capacity(output.len());
    let mut closing = None;
    for c in output.chars() {
        match closing {
            Some(end) if c == end => closing = None,
            Some(_) => {}
            None => match c {
                '[' => closing = Some(']'),
                '(' => closing = Some(')'),
                '*' => closing = Some('*'),
                _ => kept.push(c),
            },
        }
    }
    let text = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().any(char::is_alphabetic) {
        text
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning_drops_sound_labels_and_empty_results() {
        assert_eq!(
            clean(" Hello [MUSIC] there (laughs)\n *cough* friend.\n"),
            "Hello there friend."
        );
        assert_eq!(clean(" [BLANK_AUDIO]\n"), "");
        assert_eq!(clean(" ... \n"), "");
    }

    #[test]
    fn the_language_comes_from_the_first_locale_set() {
        assert_eq!(language([None, None, Some("de_DE.UTF-8".into())]), "de");
        assert_eq!(
            language([Some("fr_CA".into()), None, Some("en_US".into())]),
            "fr"
        );
        assert_eq!(language([None, None, Some("C.UTF-8".into())]), "auto");
        assert_eq!(language([None, None, None]), "auto");
    }

    #[test]
    fn the_users_data_directory_comes_before_the_system() {
        let dirs = data_dirs(
            Some("/home/a/.local/share".into()),
            Some("/opt/share:/usr/share".into()),
        );
        assert_eq!(
            dirs,
            [
                PathBuf::from("/home/a/.local/share"),
                PathBuf::from("/opt/share"),
                PathBuf::from("/usr/share")
            ]
        );
    }
}
