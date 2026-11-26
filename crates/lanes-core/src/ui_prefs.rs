use std::collections::BTreeSet;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::paths::{LanesHome, write_atomic};

/// Per-board window layout preferences, kept apart from board.toml so dragging a divider never rewrites the user's config.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, Serialize)]
pub struct UiPrefs {
    /// Width in pixels of the right-hand ticket/assistant panel.
    pub panel_width: Option<f32>,
    /// Height in pixels of the ticket info above the session tabs; unset means it fits its content.
    #[serde(default)]
    pub detail_header_height: Option<f32>,
    /// Artifact names whose cards are collapsed in the Artifacts tab, on every ticket.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub collapsed_artifacts: BTreeSet<String>,
}

impl UiPrefs {
    pub fn load(home: &LanesHome) -> Self {
        std::fs::read_to_string(file(home)).ok().and_then(|text| toml::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, home: &LanesHome) -> Result<()> {
        write_atomic(&file(home), toml::to_string(self)?.as_bytes())
    }

    pub fn is_artifact_collapsed(&self, name: &str) -> bool {
        self.collapsed_artifacts.contains(name)
    }

    pub fn toggle_artifact_collapsed(&mut self, name: &str) {
        if !self.collapsed_artifacts.remove(name) {
            self.collapsed_artifacts.insert(name.to_string());
        }
    }
}

/// Shares of the board area the right-hand panel takes in its default, wide and narrow layouts.
pub const DEFAULT_PANEL_SHARE: f32 = 0.5;
pub const WIDE_PANEL_SHARE: f32 = 0.7;
pub const NARROW_PANEL_SHARE: f32 = 0.35;
/// How far from the divider line, in pixels, a double-click still counts as on the divider.
pub const DIVIDER_HIT_SLOP: f32 = 6.0;

pub fn is_panel_wide(panel_width: f32, container_width: f32) -> bool {
    panel_width > container_width * DEFAULT_PANEL_SHARE
}

/// The width the wide/narrow toggle switches to: narrow from a wide panel, wide otherwise.
pub fn toggled_panel_width(panel_width: f32, container_width: f32) -> f32 {
    let share = if is_panel_wide(panel_width, container_width) { NARROW_PANEL_SHARE } else { WIDE_PANEL_SHARE };
    (container_width * share).round()
}

pub fn default_panel_width(container_width: f32) -> f32 {
    (container_width * DEFAULT_PANEL_SHARE).round()
}

/// The smallest heights the ticket info and the session below it can be dragged to.
pub const MIN_HEADER_HEIGHT: f32 = 60.0;
pub const MIN_SESSION_HEIGHT: f32 = 160.0;

/// The ticket info height for a divider dragged to `pointer_y`, keeping both parts of a `panel_height` panel usable.
pub fn clamp_header_height(pointer_y: f32, panel_top: f32, panel_height: f32) -> f32 {
    let max = (panel_height - MIN_SESSION_HEIGHT).max(MIN_HEADER_HEIGHT);
    (pointer_y - panel_top).clamp(MIN_HEADER_HEIGHT, max).round()
}

pub fn is_on_divider(pointer_x: f32, divider_x: f32) -> bool {
    (pointer_x - divider_x).abs() <= DIVIDER_HIT_SLOP
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
        UiPrefs { panel_width: Some(612.0), detail_header_height: Some(240.0), ..Default::default() }.save(&home).unwrap();
        assert_eq!(UiPrefs::load(&home), UiPrefs { panel_width: Some(612.0), detail_header_height: Some(240.0), ..Default::default() });
        std::fs::write(
            dir.path().join("ui.toml"),
            "panel_width = 500.0
",
        )
        .unwrap();
        assert_eq!(UiPrefs::load(&home), UiPrefs { panel_width: Some(500.0), ..Default::default() });
    }

    #[test]
    fn toggle_switches_between_wide_and_narrow_and_reset_halves() {
        assert_eq!(toggled_panel_width(500.0, 1000.0), 700.0);
        assert_eq!(toggled_panel_width(700.0, 1000.0), 350.0);
        assert_eq!(toggled_panel_width(350.0, 1000.0), 700.0);
        assert_eq!(default_panel_width(1001.0), 501.0);
        assert!(is_panel_wide(501.0, 1000.0) && !is_panel_wide(500.0, 1000.0));
    }

    #[test]
    fn collapsed_artifacts_toggle_and_survive_a_save() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let mut prefs = UiPrefs::default();
        prefs.toggle_artifact_collapsed("plan");
        assert!(prefs.is_artifact_collapsed("plan") && !prefs.is_artifact_collapsed("branch"));
        prefs.save(&home).unwrap();
        assert!(UiPrefs::load(&home).is_artifact_collapsed("plan"));
        prefs.toggle_artifact_collapsed("plan");
        assert!(!prefs.is_artifact_collapsed("plan"));
    }

    #[test]
    fn header_height_follows_the_pointer_but_leaves_room_for_both_parts() {
        assert_eq!(clamp_header_height(400.0, 100.0, 900.0), 300.0);
        assert_eq!(clamp_header_height(110.0, 100.0, 900.0), MIN_HEADER_HEIGHT);
        assert_eq!(clamp_header_height(1000.0, 100.0, 900.0), 900.0 - MIN_SESSION_HEIGHT);
        assert_eq!(clamp_header_height(500.0, 100.0, 150.0), MIN_HEADER_HEIGHT);
    }

    #[test]
    fn divider_hit_allows_a_few_pixels_either_side() {
        assert!(is_on_divider(506.0, 500.0) && is_on_divider(494.0, 500.0));
        assert!(!is_on_divider(507.0, 500.0));
    }
}
