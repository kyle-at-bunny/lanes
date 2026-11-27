use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::defaults;
use crate::paths::{LanesHome, write_atomic};

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
/// A Lanes board: board.toml in the board folder (~/.lanes by default).
pub struct BoardConfig {
    /// Settings that apply to every column unless a column overrides them.
    #[serde(default)]
    pub defaults: Defaults,
    /// Artifacts that stages produce and consume, keyed by name.
    #[serde(default)]
    pub artifacts: BTreeMap<String, ArtifactDef>,
    /// The board's columns, left to right.
    pub columns: Vec<ColumnDef>,
    /// Actions shown on every ticket, in addition to its column's own.
    #[serde(default)]
    pub actions: Vec<ActionDef>,
    /// Per-agent settings, e.g. [agents.claude] path = "claude".
    #[serde(default)]
    pub agents: AgentsConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct AgentsConfig {
    pub claude: AgentConfig,
    pub opencode: AgentConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct AgentConfig {
    /// The executable; a bare name is looked up on PATH (and ~/.local/bin).
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct Defaults {
    /// Which agent CLI runs new sessions. Existing sessions keep the agent that created them.
    pub agent: crate::agent::AgentKind,
    /// Claude Code executable; same as [agents.claude] path, which wins when both are set. Unset: `claude` is looked up on PATH.
    pub claude_path: Option<String>,
    /// Default --model for every session, e.g. "opus" or "sonnet".
    pub model: Option<String>,
    /// Default --effort for every session.
    pub effort: Option<String>,
    /// Where sessions run: inside Lanes, or in an external terminal (Windows Terminal tabs on Windows).
    pub terminal: TerminalMode,
    /// With start = "prompt", briefs longer than this many characters are written to a file for the agent to read.
    pub kickoff_inline_limit: usize,
    /// App colour scheme; the top-bar button updates this.
    pub theme: ThemeSetting,
    /// Terminal colour scheme.
    pub terminal_theme: TerminalTheme,
    /// Offer "Finish & clean up" automatically when a ticket lands in the last column.
    pub cleanup_on_done: bool,
    /// Whether new sessions wait for the user or start working on the brief straight away.
    pub start: StartMode,
    /// Where pull request status for links in artifacts comes from: "github" (the gh CLI) or "none".
    pub pr_provider: crate::pr_status::PrProviderKind,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StartMode {
    /// The brief goes into the system prompt and the session opens on an empty input box.
    #[default]
    Wait,
    /// The brief is sent as the first message, so the agent starts working immediately.
    Prompt,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
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
            agent: crate::agent::AgentKind::Claude,
            claude_path: None,
            model: None,
            effort: None,
            terminal: TerminalMode::Embedded,
            kickoff_inline_limit: 6000,
            theme: ThemeSetting::System,
            terminal_theme: TerminalTheme::Match,
            cleanup_on_done: false,
            start: StartMode::Wait,
            pr_provider: crate::pr_status::PrProviderKind::Github,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TerminalMode {
    /// Run sessions inside Lanes.
    Embedded,
    /// Open each session in an external terminal: a Windows Terminal tab, Terminal.app on macOS, or $TERMINAL on Linux.
    External,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactType {
    /// A markdown file in the ticket's artifacts folder.
    File,
    /// A short value saved with `lanes artifact set`.
    Text,
    /// A self-contained HTML page in the ticket's artifacts folder, opened in the default browser.
    Html,
}

impl ArtifactType {
    /// Whether the artifact lives in a file (markdown or HTML) rather than as a stored value.
    pub fn is_file(self) -> bool {
        matches!(self, Self::File | Self::Html)
    }
}

/// Something one stage produces and later stages consume.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct ArtifactDef {
    /// "file", "text" or "html".
    #[serde(rename = "type")]
    pub kind: ArtifactType,
    /// Shown to both sides: the producing stage is told what to write, the consuming stage what it is reading.
    #[serde(default)]
    pub description: String,
    /// Markdown files only: paste the content into the next stage's brief instead of just giving its path.
    #[serde(default)]
    pub inline: bool,
    /// File and HTML artifacts: name inside the ticket's artifacts folder (default "<name>.md", or "<name>.html").
    #[serde(default, rename = "path")]
    pub file_name: Option<String>,
}

impl ArtifactDef {
    pub fn file_name_for(&self, name: &str) -> String {
        let extension = if self.kind == ArtifactType::Html { "html" } else { "md" };
        self.file_name.clone().unwrap_or_else(|| format!("{name}.{extension}"))
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
/// A board column; columns with a `cwd` run an agent session per ticket.
pub struct ColumnDef {
    /// Unique identifier used in ticket files and the CLI; changing it orphans tickets in the column.
    pub id: String,
    /// Display name (defaults to `id`).
    pub title: Option<String>,
    /// Folder the stage's sessions run in (template, `~` expanded); omit for holding lanes like Backlog and Done.
    pub cwd: Option<String>,
    /// Prompt template relative to the board folder, appended to the agent's system prompt (or sent as its first message) when a session is created.
    pub prompt: Option<String>,
    /// Artifact names this stage reads; a trailing `?` marks one as optional.
    #[serde(default)]
    pub consumes: Vec<String>,
    /// Artifact names this stage writes; a trailing `?` marks one as optional.
    #[serde(default)]
    pub produces: Vec<String>,
    /// Overrides `[defaults] model` for this stage.
    pub model: Option<String>,
    /// Overrides `[defaults] effort` for this stage.
    pub effort: Option<String>,
    /// Permission mode for this stage, e.g. "plan" or "acceptEdits" (Claude Code only).
    pub permission_mode: Option<String>,
    /// Extra agent CLI arguments, e.g. ["--add-dir", "~/projects/shared"].
    #[serde(default)]
    pub extra_args: Vec<String>,
    /// Template for the message used when a ticket returns to this stage with changed inputs.
    pub reentry: Option<String>,
    /// Template for the message the Request handoff button sends, asking the session to write its outputs.
    pub handoff: Option<String>,
    /// Action buttons shown on tickets in this column.
    #[serde(default)]
    pub actions: Vec<ActionDef>,
    /// Command run during cleanup in this column's session folder, e.g. removing a worktree.
    pub cleanup: Option<String>,
    /// Overrides `[defaults] start` for this column.
    pub start: Option<StartMode>,
}

/// A user-triggered command button; `command` and `cwd` are minijinja templates.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct ActionDef {
    /// Button label.
    pub label: String,
    /// Command run through the system shell, cmd.exe on Windows and sh elsewhere (template); `~` isn't expanded in it.
    pub command: String,
    /// Folder to run it in (template, `~` expanded); defaults to the ticket's session folder.
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

    pub fn start_mode(&self, defaults: &Defaults) -> StartMode {
        self.start.unwrap_or(defaults.start)
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
    /// The program that runs `kind` sessions: the configured path if set, else the agent's program found on PATH.
    pub fn agent_program(&self, kind: crate::agent::AgentKind) -> Result<String> {
        self.agent_program_with(kind, |name| crate::platform::current().find_on_path(name))
    }

    fn agent_program_with(&self, kind: crate::agent::AgentKind, find_on_path: impl Fn(&str) -> Option<std::path::PathBuf>) -> Result<String> {
        if let Some(path) = self.configured_agent_path(kind) {
            return Ok(path);
        }
        let agent = crate::agent::get(kind);
        match find_on_path(agent.default_program()) {
            Some(found) => Ok(found.to_string_lossy().into_owned()),
            None => bail!("{} isn't on PATH; set [agents.{}] path to its executable", agent.default_program(), kind.config_name()),
        }
    }

    fn configured_agent_path(&self, kind: crate::agent::AgentKind) -> Option<String> {
        let configured = match kind {
            crate::agent::AgentKind::Claude => self.agents.claude.path.clone().or_else(|| self.defaults.claude_path.clone()),
            crate::agent::AgentKind::OpenCode => self.agents.opencode.path.clone(),
        };
        configured.filter(|p| !p.trim().is_empty())
    }

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
    fn agent_program_uses_a_configured_path_else_finds_the_agent_on_path() {
        use crate::agent::AgentKind::{Claude, OpenCode};
        use std::path::PathBuf;
        let base = "[[columns]]\nid = \"a\"\n";
        let found = |name: &str| Some(PathBuf::from(format!("/bin/{name}")));
        let missing = |_: &str| None;
        let plain = BoardConfig::parse(base).unwrap();
        assert_eq!(plain.agent_program_with(Claude, found).unwrap(), "/bin/claude");
        assert_eq!(plain.agent_program_with(OpenCode, found).unwrap(), "/bin/opencode");
        let err = plain.agent_program_with(OpenCode, missing).unwrap_err().to_string();
        assert_eq!(err, "opencode isn't on PATH; set [agents.opencode] path to its executable");

        let legacy = BoardConfig::parse(&format!("[defaults]\nclaude_path = \"C:/bin/claude.exe\"\n{base}")).unwrap();
        assert_eq!(legacy.agent_program_with(Claude, missing).unwrap(), "C:/bin/claude.exe");
        let both =
            BoardConfig::parse(&format!("[defaults]\nclaude_path = \"old\"\n[agents.claude]\npath = \"new\"\n[agents.opencode]\npath = \"C:/oc/opencode.exe\"\n{base}"))
                .unwrap();
        assert_eq!(both.agent_program_with(Claude, found).unwrap(), "new");
        assert_eq!(both.agent_program_with(OpenCode, found).unwrap(), "C:/oc/opencode.exe");
        assert_eq!(both.defaults.agent, Claude);
    }
    #[test]
    fn parses_columns_and_artifacts() {
        let config = BoardConfig::parse(BOARD).unwrap();
        assert_eq!(config.columns.len(), 3);
        assert!(!config.column("backlog").unwrap().has_session());
        assert_eq!(config.artifacts["notes"].file_name_for("notes"), "review-notes.md");
        assert_eq!(config.artifacts["plan"].file_name_for("plan"), "plan.md");
        let html = BoardConfig::parse("[artifacts.mockup]\ntype = \"html\"\n[[columns]]\nid = \"a\"\n").unwrap();
        assert_eq!(html.artifacts["mockup"].file_name_for("mockup"), "mockup.html");
        assert!(ArtifactType::Html.is_file() && ArtifactType::File.is_file() && !ArtifactType::Text.is_file());
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
    fn start_mode_defaults_to_wait_and_columns_override_it() {
        let config = BoardConfig::parse("[[columns]]\nid = \"a\"\n[[columns]]\nid = \"qa\"\nstart = \"prompt\"\n").unwrap();
        assert_eq!(config.column("a").unwrap().start_mode(&config.defaults), StartMode::Wait);
        assert_eq!(config.column("qa").unwrap().start_mode(&config.defaults), StartMode::Prompt);
        let prompting = BoardConfig::parse("[defaults]\nstart = \"prompt\"\n[[columns]]\nid = \"a\"\n").unwrap();
        assert_eq!(prompting.column("a").unwrap().start_mode(&prompting.defaults), StartMode::Prompt);
    }

    #[test]
    fn starter_board_is_valid() {
        BoardConfig::parse(defaults::STARTER_BOARD).unwrap();
    }
}
