use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::paths::{LanesHome, now_unix, write_atomic};

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
    pub model: Option<String>,
    pub five_hour: Option<Limit>,
    pub seven_day: Option<Limit>,
}

/// Maps a Claude Code hook payload to the agent state it implies.
pub fn hook_state(event: &Value) -> Option<HookState> {
    let name = event.get("hook_event_name")?.as_str()?;
    let notification = event.get("notification_type").and_then(Value::as_str);
    let status = match (name, notification) {
        ("SessionStart", _) => AgentStatus::Ready,
        ("UserPromptSubmit" | "PostToolUse", _) => AgentStatus::Working,
        ("Notification", Some("permission_prompt" | "elicitation_dialog")) => AgentStatus::NeedsInput,
        ("Notification", Some("idle_prompt")) | ("Stop" | "StopFailure", _) => AgentStatus::Idle,
        ("SessionEnd", _) => AgentStatus::Exited,
        _ => return None,
    };
    Some(HookState {
        status,
        at: now_unix(),
        detail: ["message", "error"].iter().find_map(|key| event.get(*key).and_then(Value::as_str)).map(str::to_string),
        transcript_path: event.get("transcript_path").and_then(Value::as_str).map(str::to_string),
    })
}

pub fn status_snapshot(input: &Value) -> StatusSnapshot {
    let limit = |key: &str| {
        let window = input.get("rate_limits")?.get(key)?;
        Some(Limit {
            used_pct: window.get("used_percentage")?.as_f64()?,
            resets_at: window.get("resets_at").and_then(Value::as_u64),
        })
    };
    StatusSnapshot {
        at: now_unix(),
        context_used_pct: input.pointer("/context_window/used_percentage").and_then(Value::as_f64),
        model: input.pointer("/model/display_name").and_then(Value::as_str).map(str::to_string),
        five_hour: limit("five_hour"),
        seven_day: limit("seven_day"),
    }
}

/// The file a `PostToolUse` hook reports Claude as having written or edited.
pub fn edited_file(event: &Value) -> Option<&str> {
    if event.get("hook_event_name")?.as_str()? != "PostToolUse" {
        return None;
    }
    let tool = event.get("tool_name")?.as_str()?;
    if !matches!(tool, "Write" | "Edit" | "MultiEdit") {
        return None;
    }
    event.pointer("/tool_input/file_path")?.as_str()
}

pub fn session_id_of(input: &Value) -> Option<Uuid> {
    input.get("session_id")?.as_str()?.parse().ok()
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
    use serde_json::json;

    #[test]
    fn maps_hook_events_to_states() {
        let status = |v: Value| hook_state(&v).map(|s| s.status);
        assert_eq!(status(json!({"hook_event_name": "UserPromptSubmit"})), Some(AgentStatus::Working));
        assert_eq!(
            status(json!({"hook_event_name": "Notification", "notification_type": "permission_prompt"})),
            Some(AgentStatus::NeedsInput)
        );
        assert_eq!(status(json!({"hook_event_name": "Stop"})), Some(AgentStatus::Idle));
        assert_eq!(status(json!({"hook_event_name": "StopFailure", "error": "auth"})), Some(AgentStatus::Idle));
        assert_eq!(status(json!({"hook_event_name": "PreCompact"})), None);
    }

    #[test]
    fn finds_files_written_by_tools() {
        let event = |tool: &str| json!({"hook_event_name": "PostToolUse", "tool_name": tool, "tool_input": {"file_path": "C:\\b\\plan.md"}});
        assert_eq!(edited_file(&event("Write")), Some("C:\\b\\plan.md"));
        assert_eq!(edited_file(&event("Edit")), Some("C:\\b\\plan.md"));
        assert_eq!(edited_file(&event("Read")), None);
        assert_eq!(edited_file(&json!({"hook_event_name": "Stop", "tool_name": "Write"})), None);
    }

    #[test]
    fn parses_status_line_input_and_finds_latest_limits() {
        let input = json!({
            "session_id": "6f0c9d3c-8f6b-4d7f-9a43-2b7c5a0e1f11",
            "model": {"display_name": "Opus"},
            "context_window": {"used_percentage": 42.5},
            "rate_limits": {"five_hour": {"used_percentage": 31.0, "resets_at": 1790000000u64}, "seven_day": {"used_percentage": 12.0}}
        });
        let snapshot = status_snapshot(&input);
        assert_eq!(snapshot.context_used_pct, Some(42.5));
        assert_eq!(snapshot.five_hour, Some(Limit { used_pct: 31.0, resets_at: Some(1790000000) }));

        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(LanesHome::new(dir.path()));
        let id = session_id_of(&input).unwrap();
        store.write_status(&id, &snapshot).unwrap();
        store.write_status(&Uuid::new_v4(), &status_snapshot(&json!({"context_window": {"used_percentage": 1.0}}))).unwrap();
        assert_eq!(store.latest_limits().unwrap().five_hour.unwrap().used_pct, 31.0);
        assert_eq!(store.status(&id).unwrap().model.as_deref(), Some("Opus"));
    }
}
