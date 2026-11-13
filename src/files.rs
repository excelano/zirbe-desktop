// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui;
use zirbe_store::{Outgoing, Reply};

pub const LARGE: usize = 25 * 1024 * 1024;

fn folder() -> PathBuf {
    dirs::runtime_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(crate::APP_ID)
}

pub fn private(kind: &str) -> Result<PathBuf, String> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    let directory = folder()
        .join(kind)
        .join(format!("{}-{stamp}", std::process::id()));
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(&directory)
        .map_err(|error| format!("{} could not be created: {error}", directory.display()))?;
    Ok(directory)
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
    let path = private("attachments")?.join(file_name(name));
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

pub fn pick() -> Receiver<Vec<Outgoing>> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let chosen = rfd::FileDialog::new().pick_files().unwrap_or_default();
        let _ = sender.send(chosen.iter().filter_map(|path| read(path)).collect());
    });
    receiver
}

fn read(path: &Path) -> Option<Outgoing> {
    Some(Outgoing {
        name: file_name(&path.file_name()?.to_string_lossy()),
        bytes: fs::read(path).ok()?,
    })
}

pub fn dropped(ctx: &egui::Context) -> Vec<Outgoing> {
    ctx.input(|input| {
        input
            .raw
            .dropped_files
            .iter()
            .filter_map(|file| {
                Some(Outgoing {
                    name: file_name(&file.path().file_name()?.to_string_lossy()),
                    bytes: file.bytes().ok()?,
                })
            })
            .collect()
    })
}

pub fn size(files: &[Outgoing]) -> usize {
    files.iter().map(|file| file.bytes.len()).sum()
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
