use std::path::PathBuf;

use anyhow::{Context, Result};

use crate::actions::{self, PreparedAction};
use crate::artifacts::ArtifactStore;
use crate::config::{ActionDef, BoardConfig};
use crate::timeline::{self, EventKind};
use crate::paths::LanesHome;
use crate::session::{LaunchSpec, SessionLauncher};
use crate::state::StateStore;
use crate::ticket::{Ticket, TicketStore};

/// What the user should know before a move; moving is never blocked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MoveCheck {
    pub forward: bool,
    pub missing_outputs: Vec<String>,
}

pub struct Board {
    pub home: LanesHome,
    pub config: BoardConfig,
    pub tickets: TicketStore,
    pub states: StateStore,
    pub lanes_exe: PathBuf,
}

impl Board {
    pub fn open(home: LanesHome, lanes_exe: PathBuf) -> Result<Self> {
        let config = BoardConfig::load_or_init(&home)?;
        Ok(Self { tickets: TicketStore::new(home.clone()), states: StateStore::new(home.clone()), home, config, lanes_exe })
    }

    pub fn reload_config(&mut self) -> Result<()> {
        self.config = BoardConfig::load(&self.home)?;
        Ok(())
    }

    pub fn artifacts<'a>(&'a self, ticket_id: &'a str) -> ArtifactStore<'a> {
        ArtifactStore::new(&self.home, &self.config, ticket_id)
    }

    pub fn check_move(&self, ticket: &Ticket, to: &str) -> Result<MoveCheck> {
        let from_index = self.config.column_index(&ticket.column);
        let to_index = self.config.column_index(to).with_context(|| format!("unknown column '{to}'"))?;
        let forward = from_index.is_some_and(|from| to_index > from);
        let missing_outputs = match self.config.column(&ticket.column) {
            Some(from) if forward => {
                let required = from.produced().filter(|a| !a.optional).map(|a| a.name);
                self.artifacts(&ticket.id).missing(required)?
            }
            _ => Vec::new(),
        };
        Ok(MoveCheck { forward, missing_outputs })
    }

    pub fn move_ticket(&self, ticket_id: &str, to: &str) -> Result<Ticket> {
        let mut ticket = self.tickets.load(ticket_id)?;
        let from = self.column_title(&ticket.column);
        ticket.move_to(to);
        self.tickets.save(&ticket)?;
        timeline::record(&self.home, ticket_id, EventKind::Moved, format!("{from} → {}", self.column_title(to)))?;
        Ok(ticket)
    }

    pub fn prepare_action(&self, ticket: &Ticket, action: &ActionDef) -> Result<PreparedAction> {
        actions::prepare(&self.home, &self.config, ticket, action)
    }

    pub fn record_action(&self, ticket_id: &str, label: &str, success: bool) -> Result<()> {
        let result = if success { "ok" } else { "failed" };
        timeline::record(&self.home, ticket_id, EventKind::ActionRun, format!("{label} ({result})"))
    }

    pub fn column_title(&self, column_id: &str) -> String {
        self.config.column(column_id).map(|c| c.title().to_string()).unwrap_or_else(|| column_id.to_string())
    }

    /// Prepares the session for the ticket's current column and persists the session record.
    pub fn prepare_session(&self, ticket_id: &str) -> Result<LaunchSpec> {
        let mut ticket = self.tickets.load(ticket_id)?;
        let column = ticket.column.clone();
        let spec = self.launcher().prepare(&mut ticket, &column)?;
        self.tickets.save(&ticket)?;
        Ok(spec)
    }

    pub fn handoff_message(&self, ticket: &Ticket) -> Result<String> {
        self.launcher().handoff_message(ticket, &ticket.column)
    }

    fn launcher(&self) -> SessionLauncher<'_> {
        SessionLauncher { home: &self.home, config: &self.config, lanes_exe: &self.lanes_exe }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_moves_warn_about_missing_required_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        std::fs::write(
            home.board_file(),
            r#"
[artifacts.plan]
type = "file"
[artifacts.extra]
type = "text"
[[columns]]
id = "plan"
produces = ["plan", "extra?"]
[[columns]]
id = "dev"
"#,
        )
        .unwrap();
        let board = Board::open(home, PathBuf::from("lanes")).unwrap();
        let ticket = board.tickets.create(None, "x", "", "plan").unwrap();

        let check = board.check_move(&ticket, "dev").unwrap();
        assert_eq!(check, MoveCheck { forward: true, missing_outputs: vec!["plan".into()] });

        board.artifacts(&ticket.id).write("plan", "done").unwrap();
        assert!(board.check_move(&ticket, "dev").unwrap().missing_outputs.is_empty());

        let moved = board.move_ticket(&ticket.id, "dev").unwrap();
        assert_eq!(board.check_move(&moved, "plan").unwrap(), MoveCheck::default());
    }
}
