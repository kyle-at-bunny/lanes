use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::paths::{LanesHome, write_atomic};

/// Per-board window layout preferences, kept apart from board.toml so dragging a divider never rewrites the user's config.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct UiPrefs {
    /// Width in pixels of the right-hand ticket/assistant panel.
    pub panel_width: Option<f32>,
}

impl UiPrefs {
    pub fn load(home: &LanesHome) -> Self {
        std::fs::read_to_string(file(home)).ok().and_then(|text| toml::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, home: &LanesHome) -> Result<()> {
        write_atomic(&file(home), toml::to_string(self)?.as_bytes())
    }
}

fn file(home: &LanesHome) -> std::path::PathBuf {
    home.root().join("ui.toml")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_or_broken_file_falls_back_to_defaults_and_saves_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        assert_eq!(UiPrefs::load(&home), UiPrefs::default());
        std::fs::write(dir.path().join("ui.toml"), "not = [valid").unwrap();
        assert_eq!(UiPrefs::load(&home), UiPrefs::default());
        UiPrefs { panel_width: Some(612.0) }.save(&home).unwrap();
        assert_eq!(UiPrefs::load(&home).panel_width, Some(612.0));
    }
}
