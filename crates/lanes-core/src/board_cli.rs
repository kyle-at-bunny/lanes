use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use clap::Subcommand;

use crate::board::Board;
use crate::config::BoardConfig;
use crate::paths::LanesHome;
use crate::ticket::Ticket;

#[derive(Debug, Subcommand)]
pub enum TicketAction {
    /// List tickets on the board.
    List {
        #[arg(long)]
        column: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Show one ticket: details, sessions and artifacts.
    Show { id: String },
    /// Create a ticket (in the first column unless --column is given).
    Create {
        title: String,
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value = "")]
        description: String,
        #[arg(long)]
        column: Option<String>,
    },
    /// Move a ticket to another column. Refuses when required outputs are missing unless --force.
    Move {
        id: String,
        column: String,
        #[arg(long)]
        force: bool,
    },
    /// Change a ticket's title or description.
    Edit {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Validate board.toml and the prompt files it references.
    Check,
}

pub fn ticket(home: &LanesHome, action: &TicketAction) -> Result<()> {
    let board = Board::open(home.clone(), std::env::current_exe()?)?;
    let output = match action {
        TicketAction::List { column, json } => list_tickets(&board, column.as_deref(), *json)?,
        TicketAction::Show { id } => show_ticket(&board, &board.tickets.load(id)?),
        TicketAction::Create { title, id, description, column } => {
            let column = column.clone().unwrap_or_else(|| board.config.first_column().id.clone());
            require_column(&board.config, &column)?;
            let ticket = board.tickets.create(id.as_deref(), title, description, &column)?;
            format!("created {} in {}", ticket.id, board.column_title(&column))
        }
        TicketAction::Move { id, column, force } => move_ticket(&board, id, column, *force)?,
        TicketAction::Edit { id, title, description } => edit_ticket(&board, id, title.as_deref(), description.as_deref())?,
    };
    println!("{output}");
    Ok(())
}

pub fn board_summary(home: &LanesHome) -> Result<()> {
    let config = BoardConfig::load(home)?;
    let tickets = crate::ticket::TicketStore::new(home.clone()).list()?;
    let mut out = format!("Board config: {}\n\nColumns (in order):\n", home.board_file().display());
    for column in &config.columns {
        let count = tickets.iter().filter(|t| t.column == column.id).count();
        let noun = if count == 1 { "ticket" } else { "tickets" };
        let _ = writeln!(out, "- {} \"{}\" ({count} {noun})", column.id, column.title());
        if let Some(cwd) = &column.cwd {
            let _ = writeln!(out, "    cwd: {cwd}  prompt: {}", column.prompt.as_deref().unwrap_or("(none)"));
        }
        if !column.consumes.is_empty() || !column.produces.is_empty() {
            let _ = writeln!(out, "    consumes: {:?}  produces: {:?}", column.consumes, column.produces);
        }
    }
    out.push_str("\nArtifacts:\n");
    for (name, def) in &config.artifacts {
        let _ = writeln!(out, "- {name} ({:?}): {}", def.kind, def.description);
    }
    println!("{out}");
    Ok(())
}

pub fn config(home: &LanesHome, action: &ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Check => {
            let problems = check_config(home);
            if problems.is_empty() {
                println!("board.toml is valid");
                return Ok(());
            }
            bail!("{}", problems.join("\n"))
        }
    }
}

pub fn check_config(home: &LanesHome) -> Vec<String> {
    let config = match BoardConfig::load(home) {
        Ok(config) => config,
        Err(err) => return vec![format!("{err:#}")],
    };
    config
        .columns
        .iter()
        .filter_map(|c| c.prompt.as_ref().map(|p| (c, p)))
        .filter(|(_, prompt)| !home.resolve(prompt).is_file())
        .map(|(column, prompt)| format!("column '{}' prompt file {} does not exist", column.id, home.resolve(prompt).display()))
        .collect()
}

fn list_tickets(board: &Board, column: Option<&str>, json: bool) -> Result<String> {
    let tickets: Vec<Ticket> = board.tickets.list()?.into_iter().filter(|t| column.is_none_or(|c| t.column == c)).collect();
    if json {
        return Ok(serde_json::to_string_pretty(&tickets)?);
    }
    if tickets.is_empty() {
        return Ok("no tickets".into());
    }
    let lines: Vec<String> = tickets.iter().map(|t| format!("{}\t[{}]\t{}", t.id, board.column_title(&t.column), t.title)).collect();
    Ok(lines.join("\n"))
}

fn show_ticket(board: &Board, ticket: &Ticket) -> String {
    let mut out = format!("{}: {}\ncolumn: {}\n", ticket.id, ticket.title, board.column_title(&ticket.column));
    if let Some(jira) = &ticket.jira {
        let _ = writeln!(out, "jira: {}", jira.url);
    }
    if !ticket.description.is_empty() {
        let _ = writeln!(out, "description:\n{}", ticket.description);
    }
    for (column, record) in &ticket.sessions {
        let _ = writeln!(out, "session[{}]: {} in {}", board.column_title(column), record.id, record.cwd.display());
    }
    let store = board.artifacts(&ticket.id);
    for name in board.config.artifacts.keys() {
        let status = if store.read(name).ok().flatten().is_some() { "present" } else { "missing" };
        let _ = writeln!(out, "artifact {name}: {status}");
    }
    out
}

fn move_ticket(board: &Board, id: &str, column: &str, force: bool) -> Result<String> {
    require_column(&board.config, column)?;
    let ticket = board.tickets.load(id)?;
    let check = board.check_move(&ticket, column)?;
    if !check.missing_outputs.is_empty() && !force {
        bail!("{id} has no {} yet; pass --force to move it anyway", check.missing_outputs.join(", "));
    }
    board.move_ticket(id, column)?;
    Ok(format!("moved {id} to {} (open it in Lanes to start or resume that stage's session)", board.column_title(column)))
}

fn edit_ticket(board: &Board, id: &str, title: Option<&str>, description: Option<&str>) -> Result<String> {
    let mut ticket = board.tickets.load(id)?;
    if let Some(title) = title {
        ticket.title = title.trim().to_string();
    }
    if let Some(description) = description {
        ticket.description = description.trim().to_string();
    }
    board.tickets.save(&ticket)?;
    Ok(format!("updated {id}"))
}

fn require_column(config: &BoardConfig, column: &str) -> Result<()> {
    let known: Vec<&str> = config.columns.iter().map(|c| c.id.as_str()).collect();
    config.column(column).map(|_| ()).with_context(|| format!("unknown column '{column}'; columns are {}", known.join(", ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn moves_respect_missing_outputs_and_config_check_finds_missing_prompts() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        std::fs::write(
            home.board_file(),
            "[artifacts.plan]\ntype = \"file\"\n[[columns]]\nid = \"plan\"\ncwd = \".\"\nprompt = \"prompts/plan.md\"\nproduces = [\"plan\"]\n[[columns]]\nid = \"dev\"\n",
        )
        .unwrap();
        let board = Board::open(home.clone(), PathBuf::from("lanes")).unwrap();
        board.tickets.create(Some("T-1"), "t", "", "plan").unwrap();

        assert!(move_ticket(&board, "T-1", "dev", false).unwrap_err().to_string().contains("--force"));
        assert!(move_ticket(&board, "T-1", "nowhere", true).unwrap_err().to_string().contains("columns are plan, dev"));
        move_ticket(&board, "T-1", "dev", true).unwrap();
        assert_eq!(board.tickets.load("T-1").unwrap().column, "dev");
        assert!(list_tickets(&board, Some("dev"), false).unwrap().starts_with("T-1\t[dev]"));

        assert_eq!(check_config(&home).len(), 1);
        std::fs::create_dir_all(dir.path().join("prompts")).unwrap();
        std::fs::write(dir.path().join("prompts/plan.md"), "x").unwrap();
        assert!(check_config(&home).is_empty());
    }
}
