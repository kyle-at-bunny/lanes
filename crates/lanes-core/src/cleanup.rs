use std::path::PathBuf;

use anyhow::Result;
use uuid::Uuid;

use crate::actions::{self, ActionOutcome, PreparedAction};
use crate::config::{ActionDef, BoardConfig};
use crate::paths::LanesHome;
use crate::sessions;
use crate::ticket::{Ticket, TicketStore};
use crate::timeline::{self, EventKind};

/// Everything cleanup will touch, computed up front so the user can confirm it.
#[derive(Debug, Clone)]
pub struct CleanupPlan {
    pub ticket_id: String,
    pub session_ids: Vec<Uuid>,
    /// Who keeps the conversations Lanes leaves behind, e.g. "Claude".
    pub agent_names: String,
    pub state_files: Vec<PathBuf>,
    pub commands: Vec<PreparedAction>,
}

#[derive(Debug, Clone, Default)]
pub struct CleanupReport {
    pub removed_files: usize,
    pub command_results: Vec<(String, ActionOutcome)>,
}

impl CleanupReport {
    pub fn failures(&self) -> impl Iterator<Item = &(String, ActionOutcome)> {
        self.command_results.iter().filter(|(_, outcome)| !outcome.success)
    }
}

pub fn plan(home: &LanesHome, config: &BoardConfig, ticket: &Ticket) -> Result<CleanupPlan> {
    let session_ids: Vec<Uuid> = ticket.sessions.values().map(|r| r.id).collect();
    let state_files = session_ids.iter().flat_map(|id| sessions::state_files(home, id)).filter(|p| p.exists()).collect();
    let agent_names = crate::agent::names(ticket.agents());
    Ok(CleanupPlan { ticket_id: ticket.id.clone(), session_ids, agent_names, state_files, commands: cleanup_commands(home, config, ticket)? })
}

/// Runs cleanup after the caller has stopped the ticket's processes, then archives the ticket.
pub fn execute(home: &LanesHome, plan: &CleanupPlan) -> Result<CleanupReport> {
    let mut report = CleanupReport::default();
    for command in &plan.commands {
        let outcome = actions::run(command).unwrap_or_else(|err| ActionOutcome { success: false, output: format!("{err:#}") });
        timeline::record(home, &plan.ticket_id, EventKind::CleanupRun, format!("{} ({})", command.label, if outcome.success { "ok" } else { "failed" }))?;
        report.command_results.push((command.label.clone(), outcome));
    }
    for file in &plan.state_files {
        if std::fs::remove_file(file).is_ok() {
            report.removed_files += 1;
        }
    }
    TicketStore::new(home.clone()).archive(&plan.ticket_id)?;
    Ok(report)
}

/// One cleanup command per column the ticket had a session in, run in that session's folder.
fn cleanup_commands(home: &LanesHome, config: &BoardConfig, ticket: &Ticket) -> Result<Vec<PreparedAction>> {
    let mut commands = Vec::new();
    for column in &config.columns {
        let (Some(command), Some(record)) = (&column.cleanup, ticket.session(&column.id)) else { continue };
        let action = ActionDef { label: format!("{} cleanup", column.title()), command: command.clone(), cwd: Some(record.cwd.to_string_lossy().into_owned()) };
        let mut as_column = ticket.clone();
        as_column.column = column.id.clone();
        commands.push(actions::prepare(home, config, &as_column, &action)?);
    }
    Ok(commands)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::portable;
    use crate::session::SessionLauncher;
    use std::path::Path;

    #[test]
    fn plans_and_executes_cleanup_then_archives() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let cwd = portable(dir.path());
        let config = BoardConfig::parse(&format!(
            r#"
[[columns]]
id = "dev"
cwd = "{cwd}"
cleanup = "echo cleaning {{{{ ticket.id }}}}"
[[columns]]
id = "done"
"#
        ))
        .unwrap();
        let store = TicketStore::new(home.clone());
        let mut ticket = store.create(Some("T-5"), "t", "", "dev").unwrap();
        let spec = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("lanes") }.prepare(&mut ticket, "dev").unwrap();
        store.save(&ticket).unwrap();
        assert!(home.session_settings_file(&spec.session_id).exists());

        let plan = plan(&home, &config, &ticket).unwrap();
        assert_eq!(plan.session_ids, [spec.session_id]);
        assert_eq!(plan.state_files.len(), 1);
        assert_eq!(plan.commands[0].command, "echo cleaning T-5");

        let report = execute(&home, &plan).unwrap();
        assert_eq!(report.removed_files, 1);
        assert_eq!(report.failures().count(), 0);
        assert_eq!(report.command_results[0].1.output.trim(), "cleaning T-5");
        assert!(!home.session_settings_file(&spec.session_id).exists());
        assert!(store.list().unwrap().is_empty());
        assert_eq!(store.list_archived().unwrap()[0].id, "T-5");
    }
}
