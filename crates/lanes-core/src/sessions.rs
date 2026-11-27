use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

use crate::agent::{self, AgentKind, TranscriptSource};
use crate::assistant;
use crate::config::BoardConfig;
use crate::paths::LanesHome;
use crate::state::{AgentStatus, StateStore};
use crate::ticket::{Ticket, TicketStore};

/// Who a session created by Lanes belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Owner {
    Ticket {
        ticket_id: String,
        column: String,
        archived: bool,
    },
    Assistant,
    /// Lanes launched it, but no ticket or the assistant links to it any more.
    Unlinked,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: Uuid,
    pub owner: Owner,
    pub cwd: Option<PathBuf>,
    pub last_active: u64,
    pub status: Option<AgentStatus>,
    pub agent: AgentKind,
    pub agent_session_id: String,
    /// The agent's own transcript, when it keeps one as a file; Lanes reports it but never deletes it.
    pub transcript: Option<PathBuf>,
    pub transcript_bytes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteReport {
    pub sessions: Vec<Uuid>,
    pub removed_state_files: usize,
}

/// Every session Lanes knows about: linked from tickets (on the board or archived), the assistant, or only from state files.
pub fn inventory(home: &LanesHome) -> Result<Vec<SessionInfo>> {
    let mut sessions = linked_sessions(home)?;
    let known: Vec<Uuid> = sessions.iter().map(|s| s.id).collect();
    sessions.extend(state_session_ids(home).into_iter().filter(|id| !known.contains(id)).map(|id| bare(id, Owner::Unlinked, None, 0, AgentKind::Claude, id.to_string())));
    let states = StateStore::new(home.clone());
    let config = BoardConfig::load(home).ok();
    for session in &mut sessions {
        let program = config.as_ref().and_then(|c| c.agent_program(session.agent).ok()).unwrap_or_default();
        enrich(home, &states, session, &program);
    }
    sessions.sort_by_key(|s| std::cmp::Reverse(s.last_active));
    Ok(sessions)
}

/// Removes Lanes' record of a session (its state files and any link to it). Claude Code's transcript is left alone.
pub fn forget(home: &LanesHome, session_id: &Uuid) -> Result<usize> {
    let store = TicketStore::new(home.clone());
    for ticket in store.list()? {
        if let Some(column) = ticket.sessions.iter().find(|(_, r)| r.id == *session_id).map(|(c, _)| c.clone()) {
            let mut ticket = ticket;
            ticket.sessions.remove(&column);
            store.save(&ticket)?;
        }
    }
    if assistant::current_session(home) == Some(*session_id) {
        assistant::reset(home)?;
    }
    Ok(remove_state_files(home, session_id))
}

/// Starts the assistant fresh next time by forgetting its current session.
pub fn reset_assistant(home: &LanesHome) -> Result<usize> {
    match assistant::current_session(home) {
        Some(id) => forget(home, &id),
        None => Ok(0),
    }
}

/// Deletes a ticket (from the board or the archive) and Lanes' records of its sessions. The caller stops running sessions first.
pub fn delete_ticket(home: &LanesHome, ticket_id: &str) -> Result<DeleteReport> {
    let (ticket, dir) = find_ticket(home, ticket_id)?;
    let sessions: Vec<Uuid> = ticket.sessions.values().map(|r| r.id).collect();
    let removed_state_files = sessions.iter().map(|id| remove_state_files(home, id)).sum();
    std::fs::remove_dir_all(&dir).with_context(|| format!("deleting {}", dir.display()))?;
    Ok(DeleteReport { sessions, removed_state_files })
}

pub fn state_files(home: &LanesHome, session_id: &Uuid) -> Vec<PathBuf> {
    let state = home.state_dir();
    ["hook", "status", "settings", "launch"].iter().map(|kind| state.join(format!("{session_id}.{kind}.json"))).collect()
}

fn remove_state_files(home: &LanesHome, session_id: &Uuid) -> usize {
    state_files(home, session_id).iter().filter(|f| std::fs::remove_file(f).is_ok()).count()
}

fn linked_sessions(home: &LanesHome) -> Result<Vec<SessionInfo>> {
    let store = TicketStore::new(home.clone());
    let tickets = store.list()?.into_iter().map(|t| (t, false)).chain(store.list_archived()?.into_iter().map(|t| (t, true)));
    let mut sessions = Vec::new();
    for (ticket, archived) in tickets {
        for (column, record) in &ticket.sessions {
            let owner = Owner::Ticket { ticket_id: ticket.id.clone(), column: column.clone(), archived };
            sessions.push(bare(record.id, owner, Some(record.cwd.clone()), record.last_entered_at, record.agent, record.agent_session_id()));
        }
    }
    if let Some((id, agent, agent_session_id)) = assistant::current(home) {
        sessions.push(bare(id, Owner::Assistant, Some(home.root().to_path_buf()), 0, agent, agent_session_id));
    }
    Ok(sessions)
}

/// Session ids from the per-session settings files Lanes writes for every launch.
fn state_session_ids(home: &LanesHome) -> Vec<Uuid> {
    let Ok(entries) = std::fs::read_dir(home.state_dir()) else { return Vec::new() };
    entries.filter_map(Result::ok).filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".settings.json").and_then(|id| id.parse().ok())).collect()
}

fn bare(id: Uuid, owner: Owner, cwd: Option<PathBuf>, last_active: u64, agent: AgentKind, agent_session_id: String) -> SessionInfo {
    SessionInfo { id, owner, cwd, last_active, status: None, agent, agent_session_id, transcript: None, transcript_bytes: 0 }
}

fn enrich(home: &LanesHome, states: &StateStore, session: &mut SessionInfo, program: &str) {
    let hook = states.hook(&session.id);
    let status_at = states.status(&session.id).map(|s| s.at);
    let settings_at = modified_secs(&home.session_settings_file(&session.id));
    session.status = hook.as_ref().map(|h| h.status);
    session.last_active = [Some(session.last_active), hook.as_ref().map(|h| h.at), status_at, settings_at].into_iter().flatten().max().unwrap_or(0);
    let reported = hook.and_then(|h| h.transcript_path);
    let cwd = session.cwd.clone().unwrap_or_default();
    let source = TranscriptSource { program, agent_session_id: &session.agent_session_id, cwd: &cwd, reported_path: reported.as_deref() };
    session.transcript = agent::get(session.agent).transcript_file(&source);
    session.transcript_bytes = session.transcript.as_deref().and_then(|p| std::fs::metadata(p).ok()).map(|m| m.len()).unwrap_or(0);
}

fn modified_secs(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    modified.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_secs())
}

fn find_ticket(home: &LanesHome, ticket_id: &str) -> Result<(Ticket, PathBuf)> {
    let store = TicketStore::new(home.clone());
    if home.ticket_file(ticket_id).exists() {
        return Ok((store.load(ticket_id)?, home.ticket_dir(ticket_id)));
    }
    let archived = store.list_archived()?.into_iter().find(|t| t.id == ticket_id).with_context(|| format!("no ticket '{ticket_id}' on the board or in the archive"))?;
    Ok((archived, home.archived_ticket_dir(ticket_id)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BoardConfig;
    use crate::session::SessionLauncher;

    fn fixture() -> (tempfile::TempDir, LanesHome, BoardConfig) {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let cwd = crate::paths::portable(dir.path());
        let config = BoardConfig::parse(&format!(
            "[defaults]\nclaude_path = \"claude\"\n[[columns]]\nid = \"plan\"\ncwd = \"{cwd}\"\n[[columns]]\nid = \"dev\"\ncwd = \"{cwd}\"\n"
        ))
        .unwrap();
        (dir, home, config)
    }

    fn start(home: &LanesHome, config: &BoardConfig, ticket_id: &str, column: &str) -> Uuid {
        let store = TicketStore::new(home.clone());
        let mut ticket = store.load(ticket_id).unwrap();
        let spec = SessionLauncher { home, config, lanes_exe: Path::new("lanes") }.prepare(&mut ticket, column).unwrap();
        store.save(&ticket).unwrap();
        spec.session_id
    }

    #[test]
    fn inventory_finds_ticket_assistant_and_unlinked_sessions() {
        let (_dir, home, config) = fixture();
        let store = TicketStore::new(home.clone());
        store.create(Some("T-1"), "t", "", "plan").unwrap();
        let plan = start(&home, &config, "T-1", "plan");
        let assistant = assistant::prepare(&home, &config, Path::new("lanes")).unwrap().session_id;
        let stray = Uuid::new_v4();
        std::fs::write(home.session_settings_file(&stray), "{}").unwrap();

        let owners: Vec<(Uuid, Owner)> = inventory(&home).unwrap().into_iter().map(|s| (s.id, s.owner)).collect();
        assert!(owners.contains(&(plan, Owner::Ticket { ticket_id: "T-1".into(), column: "plan".into(), archived: false })));
        assert!(owners.contains(&(assistant, Owner::Assistant)));
        assert!(owners.contains(&(stray, Owner::Unlinked)));
        assert_eq!(owners.len(), 3);
    }

    #[test]
    fn forget_unlinks_from_ticket_and_next_start_is_fresh() {
        let (_dir, home, config) = fixture();
        TicketStore::new(home.clone()).create(Some("T-2"), "t", "", "dev").unwrap();
        let first = start(&home, &config, "T-2", "dev");
        assert_eq!(forget(&home, &first).unwrap(), 1);
        assert!(TicketStore::new(home.clone()).load("T-2").unwrap().session("dev").is_none());
        assert_ne!(start(&home, &config, "T-2", "dev"), first);
    }

    #[test]
    fn reset_assistant_forgets_its_session() {
        let (_dir, home, config) = fixture();
        let first = assistant::prepare(&home, &config, Path::new("lanes")).unwrap().session_id;
        assert_eq!(reset_assistant(&home).unwrap(), 1);
        assert_eq!(assistant::current_session(&home), None);
        assert!(!home.session_settings_file(&first).exists());
        assert_eq!(reset_assistant(&home).unwrap(), 0);
    }

    #[test]
    fn deletes_board_and_archived_tickets_with_their_state() {
        let (_dir, home, config) = fixture();
        let store = TicketStore::new(home.clone());
        store.create(Some("T-3"), "t", "", "plan").unwrap();
        store.create(Some("T-4"), "t", "", "plan").unwrap();
        let session = start(&home, &config, "T-3", "plan");
        start(&home, &config, "T-4", "plan");
        store.archive("T-4").unwrap();

        let report = delete_ticket(&home, "T-3").unwrap();
        assert_eq!(report, DeleteReport { sessions: vec![session], removed_state_files: 1 });
        assert!(!home.ticket_dir("T-3").exists());
        assert_eq!(delete_ticket(&home, "T-4").unwrap().sessions.len(), 1);
        assert!(store.list_archived().unwrap().is_empty());
        assert!(delete_ticket(&home, "T-404").is_err());
    }
}
