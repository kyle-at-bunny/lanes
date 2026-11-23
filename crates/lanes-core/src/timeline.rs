use std::io::Write;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::paths::{LanesHome, now_unix};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Created,
    Moved,
    SessionStarted,
    SessionResumed,
    ArtifactWritten,
    ActionRun,
    CleanupRun,
    Archived,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct TimelineEvent {
    pub at: u64,
    pub kind: EventKind,
    pub detail: String,
}

/// Append-only per-ticket event log (JSON lines), so the CLI and the GUI can both write without clobbering ticket.toml.
pub fn record(home: &LanesHome, ticket_id: &str, kind: EventKind, detail: impl Into<String>) -> Result<()> {
    let path = home.ticket_dir(ticket_id).join("events.jsonl");
    std::fs::create_dir_all(home.ticket_dir(ticket_id))?;
    let event = TimelineEvent { at: now_unix(), kind, detail: detail.into() };
    let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{}", serde_json::to_string(&event)?)?;
    Ok(())
}

pub fn read(home: &LanesHome, ticket_id: &str) -> Vec<TimelineEvent> {
    let path = home.ticket_dir(ticket_id).join("events.jsonl");
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    let mut events: Vec<TimelineEvent> = text.lines().filter_map(|line| serde_json::from_str(line).ok()).collect();
    events.sort_by_key(|e| e.at);
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_reads_events_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        assert!(read(&home, "T-1").is_empty());
        record(&home, "T-1", EventKind::Created, "in backlog").unwrap();
        record(&home, "T-1", EventKind::Moved, "backlog → plan").unwrap();
        std::fs::OpenOptions::new().append(true).open(home.ticket_dir("T-1").join("events.jsonl")).unwrap().write_all(b"not json\n").unwrap();
        let events = read(&home, "T-1");
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].kind, EventKind::Moved);
        assert_eq!(events[1].detail, "backlog → plan");
    }
}
