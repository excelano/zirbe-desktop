// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::path::PathBuf;

const FOLDER: &str = "zirbe";

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_default().join(FOLDER)
}

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir().unwrap_or_default().join(FOLDER)
}
