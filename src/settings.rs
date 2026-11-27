// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zirbe_store::{Alert, Kind};

use crate::paths;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Who {
    #[default]
    Everyone,
    Favorites,
    Nobody,
}

impl Who {
    pub fn label(self) -> &'static str {
        match self {
            Who::Everyone => "Everyone",
            Who::Favorites => "Favorites",
            Who::Nobody => "Nobody",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub open_html: bool,
    pub load_images: bool,
    pub notify: bool,
    #[serde(deserialize_with = "known_kinds")]
    pub notify_kinds: BTreeMap<Kind, Who>,
    pub favorites: Vec<String>,
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
            notify_kinds: BTreeMap::new(),
            favorites: Vec::new(),
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
        matches(&self.trusted_images, sender)
    }

    pub fn trust(&mut self, entry: &str) {
        add(&mut self.trusted_images, entry);
    }

    pub fn is_favorite(&self, sender: &str) -> bool {
        matches(&self.favorites, sender)
    }

    pub fn favor(&mut self, entry: &str) {
        add(&mut self.favorites, entry);
    }

    pub fn unfavor(&mut self, entry: &str) {
        let entry = normalized(entry);
        self.favorites.retain(|favorite| *favorite != entry);
    }

    pub fn who(&self, kind: Kind) -> Who {
        self.notify_kinds.get(&kind).copied().unwrap_or_default()
    }

    pub fn set_who(&mut self, kind: Kind, who: Who) {
        if who == Who::Everyone {
            self.notify_kinds.remove(&kind);
        } else {
            self.notify_kinds.insert(kind, who);
        }
    }

    pub fn notifies(&self, kinds: &[Kind], address: Option<&str>, alert: Alert) -> bool {
        match alert {
            Alert::Mute => false,
            Alert::Always => true,
            Alert::Default => kinds.iter().any(|&kind| match self.who(kind) {
                Who::Everyone => true,
                Who::Favorites => address.is_some_and(|address| self.is_favorite(address)),
                Who::Nobody => false,
            }),
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

fn known_kinds<'de, D>(deserializer: D) -> Result<BTreeMap<Kind, Who>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let entries = BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
    Ok(entries
        .into_iter()
        .filter_map(|(kind, who)| {
            let kind = serde_json::from_value(serde_json::Value::String(kind)).ok()?;
            Some((kind, serde_json::from_value(who).ok()?))
        })
        .collect())
}

fn matches(entries: &[String], sender: &str) -> bool {
    let sender = sender.trim().to_lowercase();
    let domain = sender.rsplit_once('@').map_or("", |(_, domain)| domain);
    entries.iter().any(|entry| {
        if entry.contains('@') {
            *entry == sender
        } else {
            domain == entry || domain.ends_with(&format!(".{entry}"))
        }
    })
}

fn normalized(entry: &str) -> String {
    let entry = entry.trim();
    entry.strip_prefix('@').unwrap_or(entry).to_lowercase()
}

fn add(entries: &mut Vec<String>, entry: &str) {
    let entry = normalized(entry);
    if !entry.is_empty() && !entries.contains(&entry) {
        entries.push(entry);
        entries.sort();
    }
}

#[cfg(test)]
mod tests {
    use zirbe_store::{Alert, Kind};

    use super::{Settings, Who};

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

    #[test]
    fn defaults_notify_for_everything() {
        let settings = Settings::default();
        for kind in Kind::ALL {
            assert!(settings.notifies(&[kind], None, Alert::Default));
        }
    }

    #[test]
    fn text_from_favorites_and_all_audio() {
        let mut settings = Settings::default();
        settings
            .notify_kinds
            .insert(Kind::PlainText, Who::Favorites);
        settings
            .notify_kinds
            .insert(Kind::FormattedText, Who::Favorites);
        settings.favor("family.example");
        let text = [Kind::PlainText];
        assert!(settings.notifies(&text, Some("mom@family.example"), Alert::Default));
        assert!(!settings.notifies(&text, Some("boss@work.example"), Alert::Default));
        let with_audio = [Kind::PlainText, Kind::Audio];
        assert!(settings.notifies(&with_audio, Some("boss@work.example"), Alert::Default));
    }

    #[test]
    fn mute_beats_a_favorite() {
        let mut settings = Settings::default();
        settings.favor("mom@family.example");
        assert!(!settings.notifies(&[Kind::PlainText], Some("mom@family.example"), Alert::Mute));
    }

    #[test]
    fn always_beats_nobody() {
        let mut settings = Settings::default();
        for kind in Kind::ALL {
            settings.notify_kinds.insert(kind, Who::Nobody);
        }
        assert!(!settings.notifies(&[Kind::PlainText], None, Alert::Default));
        assert!(settings.notifies(&[Kind::PlainText], None, Alert::Always));
    }

    #[test]
    fn missing_address_is_not_a_favorite() {
        let mut settings = Settings::default();
        settings
            .notify_kinds
            .insert(Kind::PlainText, Who::Favorites);
        settings.favor("family.example");
        assert!(!settings.notifies(&[Kind::PlainText], None, Alert::Default));
    }

    #[test]
    fn choosing_a_who_round_trips_and_everyone_is_the_absence() {
        let mut settings = Settings::default();
        assert_eq!(settings.who(Kind::Audio), Who::Everyone);
        settings.set_who(Kind::Audio, Who::Nobody);
        settings.set_who(Kind::Pictures, Who::Favorites);
        settings.favor("Mom@Family.example");
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), settings);
        settings.set_who(Kind::Audio, Who::Everyone);
        assert!(!settings.notify_kinds.contains_key(&Kind::Audio));
        settings.unfavor("mom@family.example");
        assert!(!settings.is_favorite("mom@family.example"));
    }

    #[test]
    fn a_leading_at_sign_names_a_domain() {
        let mut settings = Settings::default();
        settings.favor(" @Family.example");
        settings.trust("@shop.example.org");
        assert_eq!(settings.favorites, ["family.example"]);
        assert!(settings.is_favorite("mom@family.example"));
        assert!(settings.trusts("deals@shop.example.org"));
        settings.unfavor("@family.example");
        assert!(settings.favorites.is_empty());
    }

    #[test]
    fn an_unknown_kind_or_who_is_dropped_and_the_rest_loads() {
        let text = r#"{
            "notify": false,
            "notify_kinds": {"Audio": "Nobody", "Holograms": "Favorites", "Video": "Sometimes"},
            "favorites": ["family.example"],
            "undo_seconds": 5
        }"#;
        let settings: Settings = serde_json::from_str(text).unwrap();
        assert!(!settings.notify);
        assert_eq!(settings.who(Kind::Audio), Who::Nobody);
        assert_eq!(settings.who(Kind::Video), Who::Everyone);
        assert_eq!(settings.notify_kinds.len(), 1);
        assert_eq!(settings.favorites, ["family.example"]);
        assert_eq!(settings.undo_seconds, 5);
    }

    #[test]
    fn old_settings_file_loads_with_defaults() {
        let settings: Settings = serde_json::from_str(r#"{"notify": true}"#).unwrap();
        assert!(settings.notify_kinds.is_empty());
        assert!(settings.favorites.is_empty());
    }
}
