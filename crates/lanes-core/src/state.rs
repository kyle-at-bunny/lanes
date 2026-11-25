use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::paths::{LanesHome, write_atomic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Ready,
    Working,
    NeedsInput,
    Idle,
    Exited,
}

impl AgentStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Working => "working",
            Self::NeedsInput => "needs input",
            Self::Idle => "your turn",
            Self::Exited => "exited",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HookState {
    pub status: AgentStatus,
    pub at: u64,
    pub detail: Option<String>,
    pub transcript_path: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct Limit {
    pub used_pct: f64,
    pub resets_at: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StatusSnapshot {
    pub at: u64,
    pub context_used_pct: Option<f64>,
    /// Tokens in the context window on the last request, as Claude Code reports it.
    #[serde(default)]
    pub context_tokens: Option<u64>,
    pub model: Option<String>,
    pub five_hour: Option<Limit>,
    pub seven_day: Option<Limit>,
}

#[derive(Debug, Clone)]
pub struct StateStore {
    home: LanesHome,
}

impl StateStore {
    pub fn new(home: LanesHome) -> Self {
        Self { home }
    }

    pub fn write_hook(&self, session_id: &Uuid, state: &HookState) -> Result<()> {
        write_atomic(&self.home.hook_state_file(session_id), &serde_json::to_vec(state)?)
    }

    pub fn write_status(&self, session_id: &Uuid, snapshot: &StatusSnapshot) -> Result<()> {
        write_atomic(&self.home.status_state_file(session_id), &serde_json::to_vec(snapshot)?)
    }

    pub fn hook(&self, session_id: &Uuid) -> Option<HookState> {
        read_json(&self.home.hook_state_file(session_id))
    }

    pub fn status(&self, session_id: &Uuid) -> Option<StatusSnapshot> {
        read_json(&self.home.status_state_file(session_id))
    }

    /// The most recently reported plan limits across every session.
    pub fn latest_limits(&self) -> Option<StatusSnapshot> {
        let entries = std::fs::read_dir(self.home.state_dir()).ok()?;
        entries
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().ends_with(".status.json"))
            .filter_map(|e| read_json::<StatusSnapshot>(&e.path()))
            .filter(|s| s.five_hour.is_some() || s.seven_day.is_some())
            .max_by_key(|s| s.at)
    }
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &std::path::Path) -> Option<T> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_latest_limits_across_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(LanesHome::new(dir.path()));
        let with_limits = StatusSnapshot { at: 10, model: Some("Opus".into()), five_hour: Some(Limit { used_pct: 31.0, resets_at: None }), ..Default::default() };
        let id = Uuid::new_v4();
        store.write_status(&id, &with_limits).unwrap();
        store.write_status(&Uuid::new_v4(), &StatusSnapshot { at: 20, context_used_pct: Some(1.0), ..Default::default() }).unwrap();
        assert_eq!(store.latest_limits().unwrap().five_hour.unwrap().used_pct, 31.0);
        assert_eq!(store.status(&id).unwrap().model.as_deref(), Some("Opus"));
    }
}
