use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::Value;

use crate::agent::{self, AgentKind};
use crate::artifacts::ArtifactStore;
use crate::board::Board;
use crate::board_cli::{self, ConfigAction, TicketAction};
use crate::config::BoardConfig;
use crate::paths::{COLUMN_ENV, LanesHome, TICKET_ENV};
use crate::session::LaunchSpec;
use crate::spend;
use crate::state::StateStore;

#[derive(Debug, Parser)]
#[command(name = "lanes", about = "Kanban board for interactive coding agent sessions")]
pub struct Cli {
    /// Board directory (defaults to $LANES_HOME or ~/.lanes).
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Record an agent hook event (the agent's hook JSON on stdin).
    Hook {
        /// Which agent sent it.
        #[arg(long, value_enum, default_value_t = AgentKind::Claude)]
        agent: AgentKind,
    },
    /// Status line command for agents that have one (status JSON on stdin).
    Statusline {
        #[arg(long, value_enum, default_value_t = AgentKind::Claude)]
        agent: AgentKind,
    },
    /// Read or write a ticket artifact.
    Artifact {
        #[command(subcommand)]
        action: ArtifactAction,
    },
    /// Run a prepared session in this console (used by external terminal tabs).
    Launch { session_id: uuid::Uuid },
    /// List, show, create, move or edit tickets.
    Ticket {
        #[command(subcommand)]
        action: TicketAction,
    },
    /// Summarise the board: columns, their settings, artifacts and ticket counts.
    Board,
    /// Show token spend per ticket and stage, and compare the arms of a `[spend] mode = "ab"` test.
    Spend {
        /// Only this ticket.
        #[arg(long)]
        ticket: Option<String>,
    },
    /// Validate the board configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Debug, Subcommand)]
pub enum ArtifactAction {
    /// Set an artifact; pass "-" or omit the value to read it from stdin.
    Set {
        name: String,
        value: Option<String>,
        #[arg(long)]
        ticket: Option<String>,
    },
    /// Print an artifact's current content.
    Get {
        name: String,
        #[arg(long)]
        ticket: Option<String>,
    },
}

impl Cli {
    pub fn lanes_home(&self) -> Result<LanesHome> {
        match &self.home {
            Some(root) => Ok(LanesHome::new(root)),
            None => LanesHome::from_env(),
        }
    }
}

/// Runs a subcommand. Hook and status line failures never surface to Claude Code, since a failing hook can block it.
pub fn run(cli: &Cli, command: &Command) -> Result<()> {
    let home = cli.lanes_home()?;
    match command {
        Command::Hook { agent } => {
            if let Err(err) = record_hook(&home, *agent) {
                eprintln!("lanes hook: {err:#}");
            }
            Ok(())
        }
        Command::Statusline { agent } => {
            if let Err(err) = status_line(&home, *agent) {
                eprintln!("lanes statusline: {err:#}");
            }
            Ok(())
        }
        Command::Artifact { action } => artifact(&home, action),
        Command::Launch { session_id } => std::process::exit(LaunchSpec::load(&home, session_id)?.run_attached()?),
        Command::Ticket { action } => board_cli::ticket(&home, action),
        Command::Board => board_cli::board_summary(&home),
        Command::Spend { ticket } => spend::report::run(&home, ticket.as_deref()),
        Command::Config { action } => board_cli::config(&home, action),
    }
}

fn record_hook(home: &LanesHome, kind: AgentKind) -> Result<()> {
    let input = read_stdin_json()?;
    let agent = agent::get(kind);
    record_artifact_edit(home, agent.edited_file(&input))?;
    if agent.is_compaction_start(&input) {
        print_compaction_reminder(home)?;
    }
    let Some((session_id, state)) = agent.parse_hook(&input) else {
        return Ok(());
    };
    StateStore::new(home.clone()).write_hook(&session_id, &state)
}

/// SessionStart hook output becomes context, so this puts the ticket brief back after compaction.
fn print_compaction_reminder(home: &LanesHome) -> Result<()> {
    let (Ok(ticket), Ok(column)) = (std::env::var(TICKET_ENV), std::env::var(COLUMN_ENV)) else {
        return Ok(());
    };
    let board = Board::open(home.clone(), std::env::current_exe()?)?;
    if let Some(reminder) = board.compaction_reminder(&ticket, &column)? {
        print!("{reminder}");
    }
    Ok(())
}

/// Attributes a session's direct edit of an artifact file to its stage.
fn record_artifact_edit(home: &LanesHome, edited: Option<&str>) -> Result<()> {
    let (Some(path), Ok(ticket)) = (edited, std::env::var(TICKET_ENV)) else {
        return Ok(());
    };
    let config = BoardConfig::load(home)?;
    let column = std::env::var(COLUMN_ENV).ok();
    ArtifactStore::new(home, &config, &ticket).record_file_write(Path::new(path), column.as_deref())?;
    Ok(())
}

fn status_line(home: &LanesHome, kind: AgentKind) -> Result<()> {
    let raw = read_stdin()?;
    let agent = agent::get(kind);
    let Some((session_id, snapshot)) = agent.parse_status_line(&serde_json::from_str(&raw)?) else {
        return Ok(());
    };
    StateStore::new(home.clone()).write_status(&session_id, &snapshot)?;
    print!("{}", agent.status_line_text(&raw, &snapshot));
    Ok(())
}

pub fn format_duration(seconds: u64) -> String {
    let (hours, minutes) = (seconds / 3600, (seconds % 3600) / 60);
    if hours > 0 { format!("{hours}h{minutes:02}m") } else { format!("{minutes}m") }
}

fn artifact(home: &LanesHome, action: &ArtifactAction) -> Result<()> {
    let config = BoardConfig::load(home)?;
    match action {
        ArtifactAction::Set { name, value, ticket } => {
            let ticket = resolve_ticket(ticket.as_deref())?;
            let value = match value.as_deref() {
                None | Some("-") => read_stdin()?,
                Some(value) => value.to_string(),
            };
            let column = std::env::var(COLUMN_ENV).ok();
            ArtifactStore::new(home, &config, &ticket).write_by(name, &value, column.as_deref())?;
            println!("saved artifact '{name}' for ticket {ticket}");
        }
        ArtifactAction::Get { name, ticket } => {
            let ticket = resolve_ticket(ticket.as_deref())?;
            match ArtifactStore::new(home, &config, &ticket).read(name)? {
                Some(content) => println!("{content}"),
                None => bail!("artifact '{name}' has not been produced for ticket {ticket}"),
            }
        }
    }
    Ok(())
}

fn resolve_ticket(explicit: Option<&str>) -> Result<String> {
    explicit.map(str::to_string).or_else(|| std::env::var(TICKET_ENV).ok()).context("no ticket given: pass --ticket or set LANES_TICKET")
}

fn read_stdin() -> Result<String> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    Ok(text)
}

fn read_stdin_json() -> Result<Value> {
    Ok(serde_json::from_str(&read_stdin()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_artifact_set_with_global_home() {
        let cli = Cli::try_parse_from(["lanes", "--home", "C:/b", "artifact", "set", "--ticket", "T-1", "branch", "feat/x"]).unwrap();
        assert_eq!(cli.home, Some(PathBuf::from("C:/b")));
        assert!(matches!(
            cli.command,
            Some(Command::Artifact { action: ArtifactAction::Set { ref name, ref value, ref ticket } })
                if name == "branch" && value.as_deref() == Some("feat/x") && ticket.as_deref() == Some("T-1")
        ));
    }

    #[test]
    fn formats_durations() {
        assert_eq!(format_duration(7_500), "2h05m");
        assert_eq!(format_duration(59), "0m");
    }
}
