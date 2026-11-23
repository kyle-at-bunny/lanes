use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::defaults;
use crate::paths::{LanesHome, write_atomic};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BoardConfig {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub artifacts: BTreeMap<String, ArtifactDef>,
    pub columns: Vec<ColumnDef>,
    /// Actions shown on every ticket, in addition to its column's own.
    #[serde(default)]
    pub actions: Vec<ActionDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira: Option<JiraConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JiraConfig {
    /// e.g. https://your-site.atlassian.net
    pub site: String,
    pub email: String,
    /// Name of the env var holding the API token; the token itself never goes in board.toml.
    #[serde(default = "default_token_env")]
    pub token_env: String,
    /// JQL for the "my open issues" picker.
    #[serde(default = "default_open_issues_jql")]
    pub open_issues_jql: String,
}

fn default_token_env() -> String {
    "LANES_JIRA_TOKEN".into()
}

fn default_open_issues_jql() -> String {
    "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC".into()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct Defaults {
    pub claude_path: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub terminal: TerminalMode,
    pub kickoff_inline_limit: usize,
    pub theme: ThemeSetting,
    pub terminal_theme: TerminalTheme,
    /// Offer "Finish & clean up" automatically when a ticket lands in the last column.
    pub cleanup_on_done: bool,
    /// Also delete Claude's transcript files during cleanup (always confirmed first).
    pub delete_transcripts: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSetting {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeSetting {
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalTheme {
    /// Follow the app's light/dark mode.
    #[default]
    Match,
    /// Keep the terminal dark even when the app is light.
    Dark,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            claude_path: "claude".into(),
            model: None,
            effort: None,
            terminal: TerminalMode::Embedded,
            kickoff_inline_limit: 6000,
            theme: ThemeSetting::System,
            terminal_theme: TerminalTheme::Match,
            cleanup_on_done: false,
            delete_transcripts: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalMode {
    Embedded,
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactType {
    File,
    Text,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ArtifactDef {
    #[serde(rename = "type")]
    pub kind: ArtifactType,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub inline: bool,
    #[serde(default, rename = "path")]
    pub file_name: Option<String>,
}

impl ArtifactDef {
    pub fn file_name_for(&self, name: &str) -> String {
        self.file_name.clone().unwrap_or_else(|| format!("{name}.md"))
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ColumnDef {
    pub id: String,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub prompt: Option<String>,
    #[serde(default)]
    pub consumes: Vec<String>,
    #[serde(default)]
    pub produces: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
    #[serde(default)]
    pub extra_args: Vec<String>,
    pub reentry: Option<String>,
    pub handoff: Option<String>,
    #[serde(default)]
    pub actions: Vec<ActionDef>,
    /// Command run during cleanup in this column's session folder, e.g. removing a worktree.
    pub cleanup: Option<String>,
}

/// A user-triggered command button; `command` and `cwd` are minijinja templates.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ActionDef {
    pub label: String,
    pub command: String,
    pub cwd: Option<String>,
}

impl ColumnDef {
    pub fn title(&self) -> &str {
        self.title.as_deref().unwrap_or(&self.id)
    }

    /// Columns without a working directory (backlog, done) never get a session.
    pub fn has_session(&self) -> bool {
        self.cwd.is_some()
    }

    pub fn consumed(&self) -> impl Iterator<Item = ArtifactRef<'_>> {
        self.consumes.iter().map(|raw| ArtifactRef::parse(raw))
    }

    pub fn produced(&self) -> impl Iterator<Item = ArtifactRef<'_>> {
        self.produces.iter().map(|raw| ArtifactRef::parse(raw))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactRef<'a> {
    pub name: &'a str,
    pub optional: bool,
}

impl<'a> ArtifactRef<'a> {
    pub fn parse(raw: &'a str) -> Self {
        match raw.strip_suffix('?') {
            Some(name) => Self { name, optional: true },
            None => Self { name: raw, optional: false },
        }
    }
}

impl BoardConfig {
    pub fn load_or_init(home: &LanesHome) -> Result<Self> {
        if !home.board_file().exists() {
            defaults::write_starter_board(home)?;
        }
        Self::load(home)
    }

    pub fn load(home: &LanesHome) -> Result<Self> {
        let path = home.board_file();
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let config = Self::parse(&text).with_context(|| format!("in {}", path.display()))?;
        Ok(config)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        config.validate()?;
        Ok(config)
    }

    /// Persists the theme by editing only `defaults.theme`, so the user's comments and layout survive.
    pub fn save_theme(&mut self, home: &LanesHome, theme: ThemeSetting) -> Result<()> {
        let path = home.board_file();
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut document: toml_edit::DocumentMut = text.parse().with_context(|| format!("parsing {}", path.display()))?;
        let defaults = document.entry("defaults").or_insert_with(toml_edit::table);
        let defaults = defaults.as_table_like_mut().context("[defaults] in board.toml is not a table")?;
        defaults.insert("theme", toml_edit::value(theme.as_str()));
        write_atomic(&path, document.to_string().as_bytes())?;
        self.defaults.theme = theme;
        Ok(())
    }

    pub fn column(&self, id: &str) -> Option<&ColumnDef> {
        self.columns.iter().find(|c| c.id == id)
    }

    pub fn column_index(&self, id: &str) -> Option<usize> {
        self.columns.iter().position(|c| c.id == id)
    }

    pub fn first_column(&self) -> &ColumnDef {
        &self.columns[0]
    }

    pub fn actions_for<'a>(&'a self, column_id: &str) -> Vec<&'a ActionDef> {
        let own = self.column(column_id).map(|c| c.actions.iter()).into_iter().flatten();
        own.chain(self.actions.iter()).collect()
    }

    pub fn validate(&self) -> Result<()> {
        let mut problems = Vec::new();
        self.check_columns(&mut problems);
        self.check_artifact_refs(&mut problems);
        if problems.is_empty() {
            return Ok(());
        }
        bail!("invalid board config:\n  - {}", problems.join("\n  - "))
    }

    fn check_columns(&self, problems: &mut Vec<String>) {
        if self.columns.is_empty() {
            problems.push("at least one [[columns]] entry is required".into());
        }
        let mut seen = HashSet::new();
        for column in &self.columns {
            if column.id.trim().is_empty() {
                problems.push("a column has an empty id".into());
            } else if !seen.insert(column.id.as_str()) {
                problems.push(format!("duplicate column id '{}'", column.id));
            }
        }
    }

    fn check_artifact_refs(&self, problems: &mut Vec<String>) {
        for column in &self.columns {
            for artifact in column.consumed().chain(column.produced()) {
                if !self.artifacts.contains_key(artifact.name) {
                    problems.push(format!("column '{}' references unknown artifact '{}'", column.id, artifact.name));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOARD: &str = r#"
[artifacts.plan]
type = "file"
description = "the plan"
inline = true

[artifacts.branch]
type = "text"

[[columns]]
id = "backlog"

[[columns]]
id = "plan"
cwd = "~/work"
produces = ["plan"]

[[columns]]
id = "dev"
cwd = "~/work"
consumes = ["plan", "notes?"]
produces = ["branch"]

[artifacts.notes]
type = "file"
path = "review-notes.md"
"#;

    #[test]
    fn parses_columns_and_artifacts() {
        let config = BoardConfig::parse(BOARD).unwrap();
        assert_eq!(config.columns.len(), 3);
        assert!(!config.column("backlog").unwrap().has_session());
        assert_eq!(config.artifacts["notes"].file_name_for("notes"), "review-notes.md");
        assert_eq!(config.artifacts["plan"].file_name_for("plan"), "plan.md");
        let consumed: Vec<_> = config.column("dev").unwrap().consumed().collect();
        assert_eq!(consumed[1], ArtifactRef { name: "notes", optional: true });
    }

    #[test]
    fn rejects_unknown_artifact_and_duplicate_column() {
        let text = r#"
[[columns]]
id = "a"
consumes = ["ghost"]
[[columns]]
id = "a"
"#;
        let err = BoardConfig::parse(text).unwrap_err().to_string();
        assert!(err.contains("unknown artifact 'ghost'"), "{err}");
        assert!(err.contains("duplicate column id 'a'"), "{err}");
    }

    #[test]
    fn saving_theme_preserves_comments_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        std::fs::write(home.board_file(), "# my board\n[defaults]\nmodel = \"opus\" # keep me\n\n[[columns]]\nid = \"a\"\n").unwrap();
        let mut config = BoardConfig::load(&home).unwrap();
        assert_eq!(config.defaults.theme, ThemeSetting::System);

        config.save_theme(&home, ThemeSetting::Dark).unwrap();
        let text = std::fs::read_to_string(home.board_file()).unwrap();
        assert!(text.contains("# my board") && text.contains("# keep me"), "{text}");
        assert_eq!(BoardConfig::load(&home).unwrap().defaults.theme, ThemeSetting::Dark);
        assert_eq!(ThemeSetting::Dark.next(), ThemeSetting::System);
    }

    #[test]
    fn starter_board_is_valid() {
        BoardConfig::parse(defaults::STARTER_BOARD).unwrap();
    }
}
