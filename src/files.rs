// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fs;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{SystemTime, UNIX_EPOCH};

use zirbe_store::Reply;

fn folder() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(crate::APP_ID)
        .join("attachments")
}

pub fn clear() {
    let _ = fs::remove_dir_all(folder());
}

fn file_name(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let clean = clean.trim().trim_start_matches('.');
    if clean.is_empty() {
        "attachment".into()
    } else {
        clean.into()
    }
}

pub fn open(name: &str, bytes: &[u8]) -> Result<(), String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let directory = folder().join(format!("{}-{stamp}", std::process::id()));
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(&directory)
        .map_err(|error| format!("{} could not be created: {error}", directory.display()))?;
    let path = directory.join(file_name(name));
    fs::write(&path, bytes)
        .map_err(|error| format!("{} could not be written: {error}", path.display()))?;
    open::that_detached(&path).map_err(|error| error.to_string())
}

pub fn save(name: String, bytes: Vec<u8>) -> Reply {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let chosen = rfd::FileDialog::new()
            .set_file_name(file_name(&name))
            .save_file();
        let result = match chosen {
            None => Ok(()),
            Some(path) => fs::write(&path, &bytes)
                .map_err(|error| format!("{} could not be written: {error}", path.display())),
        };
        let _ = sender.send(result);
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::file_name;

    #[test]
    fn keeps_a_name_inside_its_folder() {
        assert_eq!(file_name("../../.bashrc"), "_.._.bashrc");
        assert_eq!(file_name("..."), "attachment");
        assert_eq!(file_name("Grüße.pdf"), "Grüße.pdf");
    }
}
