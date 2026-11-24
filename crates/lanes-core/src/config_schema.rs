use anyhow::{Context, Result};
use schemars::generate::SchemaSettings;

use crate::config::BoardConfig;
use crate::paths::{LanesHome, write_atomic};

/// The configuration reference, compiled in so it always matches the installed version.
pub const CONFIG_DOCS: &str = include_str!("../../../docs/configuration.md");
pub const SCHEMA_FILE: &str = "board.schema.json";
/// Taplo-based editors (e.g. VS Code's Even Better TOML) read this comment to find the schema.
pub const SCHEMA_DIRECTIVE: &str = "#:schema ./board.schema.json";

/// JSON Schema for board.toml; draft-07 because that is what TOML editor tooling supports best.
pub fn board_schema() -> String {
    let schema = SchemaSettings::draft07().into_generator().into_root_schema_for::<BoardConfig>();
    serde_json::to_string_pretty(&schema).expect("a generated schema always serializes")
}

#[derive(Debug, PartialEq, Eq)]
pub struct SchemaInstall {
    pub schema_path: std::path::PathBuf,
    pub directive_added: bool,
}

/// Writes board.schema.json next to board.toml and points board.toml at it, so editors offer completion.
pub fn install_schema(home: &LanesHome) -> Result<SchemaInstall> {
    let board_file = home.board_file();
    let board_text = std::fs::read_to_string(&board_file).with_context(|| format!("no board at {}; start Lanes once to create it", board_file.display()))?;
    let schema_path = write_schema(home)?;
    let directive_added = add_directive(&board_file, &board_text)?;
    Ok(SchemaInstall { schema_path, directive_added })
}

/// Keeps an installed schema in step with this version of Lanes; boards without one are left alone.
pub fn refresh_installed_schema(home: &LanesHome) -> Result<()> {
    let path = home.root().join(SCHEMA_FILE);
    if path.exists() && std::fs::read_to_string(&path)? != board_schema() {
        write_schema(home)?;
    }
    Ok(())
}

fn write_schema(home: &LanesHome) -> Result<std::path::PathBuf> {
    let path = home.root().join(SCHEMA_FILE);
    write_atomic(&path, board_schema().as_bytes())?;
    Ok(path)
}

fn add_directive(board_file: &std::path::Path, text: &str) -> Result<bool> {
    if has_directive(text) {
        return Ok(false);
    }
    write_atomic(board_file, format!("{SCHEMA_DIRECTIVE}\n{text}").as_bytes())?;
    Ok(true)
}

fn has_directive(text: &str) -> bool {
    text.lines().any(|line| line.trim_start().starts_with("#:schema"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BoardConfig;

    #[test]
    fn schema_describes_board_keys_from_doc_comments() {
        let schema: serde_json::Value = serde_json::from_str(&board_schema()).unwrap();
        assert_eq!(schema["$schema"], "http://json-schema.org/draft-07/schema#");
        assert!(schema["required"].as_array().unwrap().iter().any(|k| k == "columns"));
        let text = board_schema();
        assert!(text.contains("permission_mode") && text.contains("Claude Code permission mode for this stage"));
        assert!(text.contains("\"wait\"") && text.contains("\"prompt\""));
    }

    #[test]
    fn install_writes_schema_adds_directive_once_and_board_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        BoardConfig::load_or_init(&home).unwrap();
        assert!(dir.path().join(SCHEMA_FILE).exists(), "new boards get the schema");
        let (directive, without_directive) = crate::defaults::STARTER_BOARD.split_once('\n').unwrap();
        assert!(directive.starts_with(SCHEMA_DIRECTIVE));
        std::fs::write(home.board_file(), without_directive).unwrap();

        let first = install_schema(&home).unwrap();
        assert!(first.directive_added && first.schema_path.exists());
        assert!(!install_schema(&home).unwrap().directive_added);
        let board = std::fs::read_to_string(home.board_file()).unwrap();
        assert!(board.starts_with(SCHEMA_DIRECTIVE));
        assert_eq!(board.matches("#:schema").count(), 1);
        let mut config = BoardConfig::load(&home).unwrap();
        config.save_theme(&home, crate::config::ThemeSetting::Dark).unwrap();
        assert!(std::fs::read_to_string(home.board_file()).unwrap().starts_with(SCHEMA_DIRECTIVE));
    }

    #[test]
    fn refresh_rewrites_a_stale_schema_but_never_creates_one() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let path = dir.path().join(SCHEMA_FILE);
        refresh_installed_schema(&home).unwrap();
        assert!(!path.exists());
        std::fs::write(&path, "{}").unwrap();
        refresh_installed_schema(&home).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), board_schema());
    }
}
