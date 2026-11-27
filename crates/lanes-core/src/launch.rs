use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::paths::{LanesHome, portable, write_atomic};
use crate::platform;
use crate::session::LaunchSpec;

impl LaunchSpec {
    pub fn save(&self, home: &LanesHome) -> Result<()> {
        write_atomic(&launch_file(home, &self.session_id), &serde_json::to_vec_pretty(self)?)
    }

    pub fn load(home: &LanesHome, session_id: &Uuid) -> Result<Self> {
        let path = launch_file(home, session_id);
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Runs the session attached to the current console; used inside external terminal tabs.
    pub fn run_attached(&self) -> Result<i32> {
        let mut command = Command::new(platform::current().resolve_program(&self.program));
        for key in inherited_session_vars() {
            command.env_remove(key);
        }
        let status =
            command.args(&self.args).current_dir(&self.cwd).envs(self.env.iter().map(|(k, v)| (k, v))).status().with_context(|| format!("starting {}", self.program))?;
        Ok(status.code().unwrap_or(1))
    }

    /// The platform's external terminal running `lanes launch <session>`, or None when there is none.
    pub fn external_terminal_command(&self, home: &LanesHome, lanes_exe: &Path) -> Option<Command> {
        let args = ["--home".to_string(), portable(home.root()), "launch".to_string(), self.session_id.to_string()];
        platform::current().external_terminal(&self.title, &self.cwd, lanes_exe, &args)
    }
}

/// Env vars a parent Claude Code session leaks when Lanes itself was started from one; children must not inherit them.
pub fn inherited_session_vars() -> Vec<String> {
    if std::env::var_os("CLAUDECODE").is_none() {
        return Vec::new();
    }
    std::env::vars_os()
        .filter_map(|(key, _)| key.into_string().ok())
        .filter(|key| {
            let upper = key.to_ascii_uppercase();
            upper == "CLAUDECODE"
                || upper == "CLAUDE_PID"
                || upper == "ANTHROPIC_BASE_URL"
                || upper.starts_with("CLAUDE_CODE_")
                || upper.starts_with("CLAUDE_AGENT_SDK_")
                || upper.starts_with("MCP_")
        })
        .collect()
}

fn launch_file(home: &LanesHome, session_id: &Uuid) -> std::path::PathBuf {
    home.state_dir().join(format!("{session_id}.launch.json"))
}
