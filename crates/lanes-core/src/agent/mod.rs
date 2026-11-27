//! The coding agent CLI that runs each session (Claude Code today), behind one interface.
//!
//! Lanes keys everything on its own session UUID. An agent may use the same id (Claude Code accepts it) or its own,
//! which is kept on the session record. Capabilities an agent lacks degrade: no live state means cards just show
//! "running", no transcript means the Transcript tab says so.

use std::path::{Path, PathBuf};

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::paths::LanesHome;
use crate::state::{HookState, StatusSnapshot};
use crate::transcript::TranscriptEntry;

mod claude;
mod opencode;

pub use claude::Claude;
pub use opencode::OpenCode;

/// The `agent` setting: which CLI runs sessions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema, clap::ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum AgentKind {
    /// Claude Code (`claude`).
    #[default]
    Claude,
    /// OpenCode (`opencode`).
    #[serde(rename = "opencode")]
    #[value(name = "opencode")]
    OpenCode,
}

impl AgentKind {
    /// The name used in board.toml, e.g. "opencode".
    pub fn config_name(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::OpenCode => "opencode",
        }
    }
}

/// What an agent can report back to Lanes beyond running in a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Working / needs input / your turn, from hooks.
    pub live_state: bool,
    /// Context window usage and plan limits, from a status line.
    pub usage: bool,
    /// A readable conversation for the Transcript tab.
    pub transcript: bool,
    /// The stage prompt can be appended to the system prompt; without it the brief is sent as the first message.
    pub system_prompt: bool,
}

/// Everything a launch needs, independent of how a particular CLI spells it.
pub struct LaunchRequest<'a> {
    pub home: &'a LanesHome,
    pub lanes_exe: &'a Path,
    /// Lanes' own id for the session; state files are keyed on it.
    pub session_id: Uuid,
    /// The agent's id for the session (often the same).
    pub agent_session_id: &'a str,
    pub resume: bool,
    /// New sessions only: the stage prompt (and, in wait mode, the ticket brief) to append to the system prompt.
    pub system_prompt_file: Option<&'a Path>,
    /// A message to send as soon as the session starts: the kickoff brief, or a re-entry note.
    pub initial_prompt: Option<String>,
    pub model: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub permission_mode: Option<&'a str>,
    pub extra_args: &'a [String],
}

/// Where to find a session's conversation.
pub struct TranscriptSource<'a> {
    /// The agent's executable, for agents that export transcripts through their CLI.
    pub program: &'a str,
    pub agent_session_id: &'a str,
    pub cwd: &'a Path,
    /// A path the agent reported itself (e.g. through a hook), if any.
    pub reported_path: Option<&'a str>,
}

pub trait AgentCli: Sync {
    fn kind(&self) -> AgentKind;
    /// Short name for labels, e.g. "Claude".
    fn display_name(&self) -> &'static str;
    /// The program looked up on PATH when `[agents.<kind>] path` isn't set.
    fn default_program(&self) -> &'static str;
    fn capabilities(&self) -> Capabilities;

    /// The agent's id for a new session Lanes has given `session_id`.
    fn new_session_id(&self, session_id: Uuid) -> String {
        session_id.to_string()
    }

    /// Command-line arguments for a new or resumed session (may write per-session files the CLI reads).
    fn launch_args(&self, request: &LaunchRequest<'_>) -> Result<Vec<String>>;

    /// Stage settings (`model`, `effort`, `permission_mode`) this agent has no way to apply; `config check` warns about them.
    fn ignored_settings(&self) -> &'static [&'static str] {
        &[]
    }

    /// Extra arguments for the board assistant, e.g. pre-approving the `lanes` CLI.
    fn assistant_args(&self, _lanes_command: &str) -> Vec<String> {
        Vec::new()
    }

    /// The state implied by an event the agent reported through `lanes hook`, with the Lanes session it belongs to.
    fn parse_hook(&self, _event: &Value) -> Option<(Uuid, HookState)> {
        None
    }

    /// A file the agent says it just wrote, from the same event; used to attribute artifact edits.
    fn edited_file<'v>(&self, _event: &'v Value) -> Option<&'v str> {
        None
    }

    /// Usage the agent reported through `lanes statusline`, with the Lanes session it belongs to.
    fn parse_status_line(&self, _input: &Value) -> Option<(Uuid, StatusSnapshot)> {
        None
    }

    /// What `lanes statusline` prints back to the agent.
    fn status_line_text(&self, _raw: &str, _snapshot: &StatusSnapshot) -> String {
        String::new()
    }

    fn transcript_file(&self, _source: &TranscriptSource<'_>) -> Option<PathBuf> {
        None
    }

    fn read_transcript(&self, _source: &TranscriptSource<'_>) -> Vec<TranscriptEntry> {
        Vec::new()
    }

    /// How many days the agent keeps its own transcripts, if it says.
    fn retention_days(&self) -> Option<u64> {
        None
    }

    /// Of the variable names in Lanes' own environment, those a parent session of this agent leaked (when Lanes was started from one); no child session may inherit them.
    fn leaked_env_vars(&self, _names: &[String]) -> Vec<String> {
        Vec::new()
    }
}

pub fn get(kind: AgentKind) -> &'static dyn AgentCli {
    match kind {
        AgentKind::Claude => &Claude,
        AgentKind::OpenCode => &OpenCode,
    }
}

pub fn all() -> [&'static dyn AgentCli; 2] {
    [&Claude, &OpenCode]
}

/// The agents' names for a sentence, e.g. "Claude" or "Claude and OpenCode"; "The agent" when there are none.
pub fn names(kinds: impl IntoIterator<Item = AgentKind>) -> String {
    let mut seen: Vec<AgentKind> = Vec::new();
    for kind in kinds {
        if !seen.contains(&kind) {
            seen.push(kind);
        }
    }
    let names: Vec<&str> = seen.iter().map(|kind| get(*kind).display_name()).collect();
    match names.as_slice() {
        [] => "The agent".to_string(),
        [one] => one.to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_each_agent_once_for_a_sentence() {
        assert_eq!(names([]), "The agent");
        assert_eq!(names([AgentKind::Claude, AgentKind::Claude]), "Claude");
        assert_eq!(names([AgentKind::OpenCode, AgentKind::Claude, AgentKind::OpenCode]), "OpenCode and Claude");
    }

    #[test]
    fn every_kind_has_an_adapter_of_that_kind() {
        for kind in <AgentKind as clap::ValueEnum>::value_variants() {
            assert_eq!(get(*kind).kind(), *kind);
        }
    }
}
