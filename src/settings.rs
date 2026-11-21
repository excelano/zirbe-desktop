// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub open_html: bool,
    pub load_images: bool,
    pub notify: bool,
    pub preview: u8,
    pub trusted_images: Vec<String>,
    pub undo_seconds: u8,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            open_html: false,
            load_images: false,
            notify: true,
            preview: 2,
            trusted_images: Vec::new(),
            undo_seconds: 0,
        }
    }
}

impl Settings {
    pub fn images_for(&self, sender: Option<&str>) -> bool {
        self.load_images || sender.is_some_and(|sender| self.trusts(sender))
    }

    pub fn trusts(&self, sender: &str) -> bool {
        let sender = sender.trim().to_lowercase();
        let domain = sender.rsplit_once('@').map_or("", |(_, domain)| domain);
        self.trusted_images.iter().any(|trusted| {
            if trusted.contains('@') {
                *trusted == sender
            } else {
                domain == trusted || domain.ends_with(&format!(".{trusted}"))
            }
        })
    }

    pub fn trust(&mut self, entry: &str) {
        let entry = entry.trim().to_lowercase();
        if !entry.is_empty() && !self.trusted_images.contains(&entry) {
            self.trusted_images.push(entry);
            self.trusted_images.sort();
        }
    }

    pub fn undo_delay(&self) -> std::time::Duration {
        std::time::Duration::from_secs(u64::from(self.undo_seconds))
    }

    pub fn load() -> Settings {
        std::fs::read_to_string(paths::config_dir().join("settings.json"))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) {
        let dir = paths::config_dir();
        if std::fs::create_dir_all(&dir).is_ok()
            && let Ok(text) = serde_json::to_string_pretty(self)
        {
            let _ = std::fs::write(dir.join("settings.json"), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Settings;

    #[test]
    fn trusts_a_sender_or_a_domain_and_its_subdomains() {
        let mut settings = Settings::default();
        settings.trust("News@Example.com");
        settings.trust("shop.example.org");
        assert!(settings.images_for(Some("news@example.com")));
        assert!(!settings.images_for(Some("other@example.com")));
        assert!(settings.images_for(Some("deals@shop.example.org")));
        assert!(settings.images_for(Some("a@mail.shop.example.org")));
        assert!(!settings.images_for(Some("a@example.org")));
        assert!(!settings.images_for(None));
    }
}
