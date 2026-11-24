use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::paths::{LanesHome, portable, write_atomic};
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

    /// Runs claude attached to the current console; used inside external terminal tabs.
    pub fn run_attached(&self) -> Result<i32> {
        let mut command = Command::new(&self.program);
        for key in inherited_session_vars() {
            command.env_remove(key);
        }
        let status =
            command.args(&self.args).current_dir(&self.cwd).envs(self.env.iter().map(|(k, v)| (k, v))).status().with_context(|| format!("starting {}", self.program))?;
        Ok(status.code().unwrap_or(1))
    }

    /// A Windows Terminal invocation that opens a tab running `lanes launch <session>`.
    pub fn external_terminal_command(&self, home: &LanesHome, lanes_exe: &Path) -> Command {
        let mut command = Command::new("wt.exe");
        command.args(["-w", "0", "new-tab", "--title", &self.title.replace(';', ","), "-d"]).arg(&self.cwd).arg(lanes_exe).args([
            "--home",
            &portable(home.root()),
            "launch",
            &self.session_id.to_string(),
        ]);
        command
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
