use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command as Process, Stdio};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use serde_json::Value;

use crate::artifacts::ArtifactStore;
use crate::board_cli::{self, ConfigAction, TicketAction};
use crate::config::BoardConfig;
use crate::paths::{LanesHome, TICKET_ENV, now_unix};
use crate::session::LaunchSpec;
use crate::state::{StateStore, StatusSnapshot, hook_state, session_id_of, status_snapshot};

#[derive(Debug, Parser)]
#[command(name = "lanes", about = "Kanban board for interactive Claude Code sessions")]
pub struct Cli {
    /// Board directory (defaults to $LANES_HOME or ~/.lanes).
    #[arg(long, global = true)]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Record a Claude Code hook event (hook JSON on stdin).
    Hook,
    /// Claude Code status line command (status JSON on stdin).
    Statusline,
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
        Command::Hook => {
            if let Err(err) = record_hook(&home) {
                eprintln!("lanes hook: {err:#}");
            }
            Ok(())
        }
        Command::Statusline => {
            if let Err(err) = status_line(&home) {
                eprintln!("lanes statusline: {err:#}");
            }
            Ok(())
        }
        Command::Artifact { action } => artifact(&home, action),
        Command::Launch { session_id } => std::process::exit(LaunchSpec::load(&home, session_id)?.run_attached()?),
        Command::Ticket { action } => board_cli::ticket(&home, action),
        Command::Board => board_cli::board_summary(&home),
        Command::Config { action } => board_cli::config(&home, action),
    }
}

fn record_hook(home: &LanesHome) -> Result<()> {
    let input = read_stdin_json()?;
    let (Some(session_id), Some(state)) = (session_id_of(&input), hook_state(&input)) else {
        return Ok(());
    };
    StateStore::new(home.clone()).write_hook(&session_id, &state)
}

fn status_line(home: &LanesHome) -> Result<()> {
    let raw = read_stdin()?;
    let input: Value = serde_json::from_str(&raw)?;
    let snapshot = status_snapshot(&input);
    if let Some(session_id) = session_id_of(&input) {
        StateStore::new(home.clone()).write_status(&session_id, &snapshot)?;
    }
    let line = match user_status_line_command() {
        Some(command) => run_user_status_line(&command, &raw).unwrap_or_else(|_| default_status_line(&snapshot)),
        None => default_status_line(&snapshot),
    };
    print!("{line}");
    Ok(())
}

fn default_status_line(snapshot: &StatusSnapshot) -> String {
    let mut parts = vec![snapshot.model.clone().unwrap_or_else(|| "claude".into())];
    if let Some(ctx) = snapshot.context_used_pct {
        parts.push(format!("ctx {ctx:.0}%"));
    }
    if let Some(limit) = snapshot.five_hour {
        let reset = limit.resets_at.map(|at| format!(" (resets in {})", format_duration(at.saturating_sub(now_unix())))).unwrap_or_default();
        parts.push(format!("5h {:.0}%{reset}", limit.used_pct));
    }
    parts.join(" · ")
}

pub fn format_duration(seconds: u64) -> String {
    let (hours, minutes) = (seconds / 3600, (seconds % 3600) / 60);
    if hours > 0 { format!("{hours}h{minutes:02}m") } else { format!("{minutes}m") }
}

/// The user's own status line command from ~/.claude/settings.json, so lanes chains to it instead of replacing it.
fn user_status_line_command() -> Option<String> {
    let path = dirs::home_dir()?.join(".claude").join("settings.json");
    let settings: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
    settings.pointer("/statusLine/command")?.as_str().map(str::to_string)
}

fn run_user_status_line(command: &str, input: &str) -> Result<String> {
    let mut child = Process::new("bash").args(["-c", command]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
    child.stdin.take().context("no stdin")?.write_all(input.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim_end().to_string())
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
            ArtifactStore::new(home, &config, &ticket).write(name, &value)?;
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
    explicit
        .map(str::to_string)
        .or_else(|| std::env::var(TICKET_ENV).ok())
        .context("no ticket given: pass --ticket or set LANES_TICKET")
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
