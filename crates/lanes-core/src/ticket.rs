use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::jira::JiraLink;
use crate::paths::{LanesHome, now_unix, write_atomic};
use crate::timeline::{self, EventKind};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub column: String,
    pub created_at: u64,
    /// Working folder for this ticket's new sessions in every stage, overriding the column's `cwd`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub history: Vec<Move>,
    #[serde(default)]
    pub sessions: BTreeMap<String, SessionRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jira: Option<JiraLink>,
    /// Per-column working folders, taking precedence over `cwd` for that column.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub column_cwd: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Move {
    pub from: Option<String>,
    pub to: String,
    pub at: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SessionRecord {
    pub id: Uuid,
    pub cwd: PathBuf,
    pub created_at: u64,
    pub last_entered_at: u64,
    /// Content hash of each consumed artifact when the session last ran, to detect changes on re-entry.
    #[serde(default)]
    pub input_hashes: BTreeMap<String, String>,
}

impl Ticket {
    pub fn session(&self, column: &str) -> Option<&SessionRecord> {
        self.sessions.get(column)
    }

    pub fn move_to(&mut self, column: &str) {
        if self.column == column {
            return;
        }
        self.history.push(Move { from: Some(self.column.clone()), to: column.to_string(), at: now_unix() });
        self.column = column.to_string();
    }

    /// Sets the all-stages working folder; blank clears it.
    pub fn set_cwd(&mut self, folder: &str) {
        let folder = folder.trim();
        self.cwd = (!folder.is_empty()).then(|| folder.to_string());
    }

    /// Sets one stage's working folder; blank clears it.
    pub fn set_column_cwd(&mut self, column: &str, folder: &str) {
        match folder.trim() {
            "" => self.column_cwd.remove(column),
            folder => self.column_cwd.insert(column.to_string(), folder.to_string()),
        };
    }

    pub fn last_arrival_from(&self) -> Option<&str> {
        self.history.iter().rev().find(|m| m.to == self.column).and_then(|m| m.from.as_deref())
    }
}

#[derive(Debug, Clone)]
pub struct TicketStore {
    home: LanesHome,
}

impl TicketStore {
    pub fn new(home: LanesHome) -> Self {
        Self { home }
    }

    pub fn list(&self) -> Result<Vec<Ticket>> {
        list_in(&self.home.tickets_dir())
    }

    /// Like `list`, but skips ticket files that can't be read instead of failing, so one bad file doesn't hide the board.
    pub fn scan(&self) -> Result<TicketScan> {
        scan_in(&self.home.tickets_dir())
    }

    pub fn list_archived(&self) -> Result<Vec<Ticket>> {
        list_in(&self.home.archive_dir())
    }

    /// Moves the ticket folder (artifacts, session map, timeline) out of the board into tickets/_archive.
    pub fn archive(&self, id: &str) -> Result<()> {
        let target = self.home.archived_ticket_dir(id);
        if target.exists() {
            bail!("an archived ticket '{id}' already exists");
        }
        timeline::record(&self.home, id, EventKind::Archived, "finished and cleaned up")?;
        std::fs::create_dir_all(self.home.archive_dir())?;
        std::fs::rename(self.home.ticket_dir(id), &target).with_context(|| format!("archiving ticket {id}"))
    }

    pub fn restore(&self, id: &str) -> Result<()> {
        if self.home.ticket_dir(id).exists() {
            bail!("ticket '{id}' is already on the board");
        }
        std::fs::rename(self.home.archived_ticket_dir(id), self.home.ticket_dir(id)).with_context(|| format!("restoring ticket {id}"))?;
        timeline::record(&self.home, id, EventKind::Restored, "restored from archive")
    }

    pub fn load(&self, id: &str) -> Result<Ticket> {
        load_ticket(&self.home.ticket_file(id))
    }

    /// Replaces the description, re-reading the file first so changes made meanwhile (e.g. by the CLI) are kept.
    pub fn set_description(&self, id: &str, description: &str) -> Result<Ticket> {
        let mut ticket = self.load(id)?;
        ticket.description = description.trim().to_string();
        self.save(&ticket)?;
        Ok(ticket)
    }

    pub fn save(&self, ticket: &Ticket) -> Result<()> {
        write_atomic(&self.home.ticket_file(&ticket.id), toml::to_string_pretty(ticket)?.as_bytes())
    }

    pub fn create(&self, id: Option<&str>, title: &str, description: &str, column: &str) -> Result<Ticket> {
        let id = match id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(id) => self.validate_new_id(id)?,
            None => self.next_id()?,
        };
        let ticket = Ticket {
            id,
            title: title.trim().to_string(),
            description: description.trim().to_string(),
            column: column.to_string(),
            created_at: now_unix(),
            history: vec![Move { from: None, to: column.to_string(), at: now_unix() }],
            cwd: None,
            sessions: BTreeMap::new(),
            jira: None,
            column_cwd: BTreeMap::new(),
        };
        self.save(&ticket)?;
        timeline::record(&self.home, &ticket.id, EventKind::Created, format!("created in {column}"))?;
        Ok(ticket)
    }

    pub fn create_linked(&self, id: &str, title: &str, description: &str, column: &str, link: JiraLink) -> Result<Ticket> {
        let mut ticket = self.create(Some(id), title, description, column)?;
        ticket.jira = Some(link);
        self.save(&ticket)?;
        Ok(ticket)
    }

    pub fn exists(&self, id: &str) -> bool {
        self.home.ticket_file(id).exists() || self.home.archived_ticket_dir(id).exists()
    }

    fn validate_new_id(&self, id: &str) -> Result<String> {
        if id.starts_with('_') || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            bail!("ticket id '{id}' may only contain letters, digits, '-' and '_', and may not start with '_'");
        }
        if self.home.ticket_file(id).exists() {
            bail!("ticket '{id}' already exists");
        }
        if self.home.archived_ticket_dir(id).exists() {
            bail!("ticket '{id}' already exists in the archive");
        }
        Ok(id.to_string())
    }

    fn next_id(&self) -> Result<String> {
        let number = |ticket: Ticket| ticket.id.strip_prefix("T-").and_then(|n| n.parse::<u32>().ok());
        let highest = self.list()?.into_iter().chain(self.list_archived()?).filter_map(number).max().unwrap_or(0);
        Ok(format!("T-{:03}", highest + 1))
    }
}

/// Tickets that loaded, plus one message per ticket file that couldn't be read or parsed.
#[derive(Debug, Default)]
pub struct TicketScan {
    pub tickets: Vec<Ticket>,
    pub unreadable: Vec<String>,
}

fn list_in(dir: &std::path::Path) -> Result<Vec<Ticket>> {
    let scan = scan_in(dir)?;
    match scan.unreadable.into_iter().next() {
        Some(problem) => anyhow::bail!(problem),
        None => Ok(scan.tickets),
    }
}

fn scan_in(dir: &std::path::Path) -> Result<TicketScan> {
    let mut scan = TicketScan::default();
    if !dir.exists() {
        return Ok(scan);
    }
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path().join("ticket.toml");
        if path.exists() {
            match load_ticket(&path) {
                Ok(ticket) => scan.tickets.push(ticket),
                Err(err) => scan.unreadable.push(format!("{err:#}")),
            }
        }
    }
    scan.tickets.sort_by(|a, b| a.created_at.cmp(&b.created_at).then_with(|| a.id.cmp(&b.id)));
    Ok(scan)
}

fn load_ticket(path: &std::path::Path) -> Result<Ticket> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_overrides_round_trip_through_ticket_toml() {
        let dir = tempfile::tempdir().unwrap();
        let store = TicketStore::new(LanesHome::new(dir.path()));
        let mut ticket = store.create(Some("T-1"), "t", "", "dev").unwrap();
        ticket.cwd = Some("D:/repos/app".into());
        ticket.column_cwd.insert("qa".into(), "E:/staging".into());
        store.save(&ticket).unwrap();
        let loaded = store.load("T-1").unwrap();
        assert_eq!(loaded.cwd.as_deref(), Some("D:/repos/app"));
        assert_eq!(loaded.column_cwd["qa"], "E:/staging");
    }

    #[test]
    fn archives_and_restores_without_reusing_ids() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let store = TicketStore::new(home.clone());
        let first = store.create(None, "One", "", "done").unwrap();
        store.archive(&first.id).unwrap();
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.list_archived().unwrap()[0].id, "T-001");
        assert_eq!(store.create(None, "Two", "", "backlog").unwrap().id, "T-002");
        assert!(store.create(Some("T-001"), "dup", "", "backlog").is_err());
        assert!(store.create(Some("_archive"), "bad", "", "backlog").is_err());

        store.restore("T-001").unwrap();
        assert_eq!(store.list().unwrap().len(), 2);
        assert!(store.list_archived().unwrap().is_empty());
        let kinds: Vec<_> = timeline::read(&home, "T-001").into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, [EventKind::Created, EventKind::Archived, EventKind::Restored]);
    }

    #[test]
    fn creates_lists_and_moves_tickets() {
        let dir = tempfile::tempdir().unwrap();
        let store = TicketStore::new(LanesHome::new(dir.path()));
        let first = store.create(None, "First", "", "backlog").unwrap();
        let second = store.create(Some("PROJ-12"), "Second", "desc", "backlog").unwrap();
        let third = store.create(None, "Third", "", "backlog").unwrap();
        assert_eq!(first.id, "T-001");
        assert_eq!(second.id, "PROJ-12");
        assert_eq!(third.id, "T-002");
        assert!(store.create(Some("PROJ-12"), "dup", "", "backlog").is_err());
        assert!(store.create(Some("bad id"), "x", "", "backlog").is_err());

        let mut ticket = store.load("PROJ-12").unwrap();
        ticket.move_to("plan");
        ticket.move_to("dev");
        ticket.move_to("plan");
        store.save(&ticket).unwrap();
        let reloaded = store.load("PROJ-12").unwrap();
        assert_eq!(reloaded.column, "plan");
        assert_eq!(reloaded.last_arrival_from(), Some("dev"));
        assert_eq!(store.list().unwrap().len(), 3);
    }

    #[test]
    fn set_description_trims_and_keeps_other_fields() {
        let dir = tempfile::tempdir().unwrap();
        let store = TicketStore::new(LanesHome::new(dir.path()));
        let mut ticket = store.create(Some("T-1"), "title", "old", "dev").unwrap();
        ticket.cwd = Some("D:/repos/app".into());
        store.save(&ticket).unwrap();

        let updated = store.set_description("T-1", "  line one\nline two  \n").unwrap();
        assert_eq!(updated.description, "line one\nline two");
        let reloaded = store.load("T-1").unwrap();
        assert_eq!(reloaded.description, "line one\nline two");
        assert_eq!(reloaded.cwd.as_deref(), Some("D:/repos/app"));
    }

    #[test]
    fn scan_skips_unreadable_tickets_that_list_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let store = TicketStore::new(home.clone());
        store.create(Some("T-1"), "good", "", "dev").unwrap();
        let broken = home.ticket_dir("T-2");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join("ticket.toml"), "id = \"T-2\"").unwrap();

        let scan = store.scan().unwrap();
        assert_eq!(scan.tickets.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["T-1"]);
        assert_eq!(scan.unreadable.len(), 1);
        assert!(scan.unreadable[0].contains("T-2") && scan.unreadable[0].contains("missing field `title`"));
        assert!(store.list().is_err());
    }
}
