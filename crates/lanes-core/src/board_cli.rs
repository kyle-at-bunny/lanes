use std::fmt::Write as _;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

use crate::board::Board;
use crate::config::BoardConfig;
use crate::config_schema;
use crate::link::{self, TicketLink};
use crate::paths::LanesHome;
use crate::ticket::Ticket;
use crate::workdir;

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
        #[command(flatten)]
        settings: SettingsArgs,
    },
    /// Move a ticket to another column. Refuses when required outputs are missing unless --force.
    Move {
        id: String,
        column: String,
        #[arg(long)]
        force: bool,
    },
    /// Change a ticket's title, description or working folders.
    Edit {
        id: String,
        #[arg(long)]
        title: Option<String>,
        #[arg(long)]
        description: Option<String>,
        #[command(flatten)]
        settings: SettingsArgs,
    },
}

/// Ticket settings shared by create and edit. Folder changes apply to sessions started afterwards.
#[derive(Debug, Default, Args)]
pub struct SettingsArgs {
    /// Folder for this ticket's new sessions in every stage ("" clears it). Templates like {{ ticket.id }} work.
    #[arg(long)]
    pub cwd: Option<String>,
    /// Folder for one stage, as COLUMN=FOLDER ("COLUMN=" clears it). Repeatable.
    #[arg(long = "column-cwd", value_name = "COLUMN=FOLDER")]
    pub column_cwd: Vec<String>,
    /// Link the ticket to its work item in your tracker: the item's URL ("" removes the link).
    #[arg(long, alias = "jira")]
    pub link: Option<String>,
    /// The short name shown for the link, when it can't be worked out from the URL (e.g. "SUP-12").
    #[arg(long = "link-key")]
    pub link_key: Option<String>,
}

/// Validated settings, checked before anything is written.
struct TicketChanges {
    cwd: Option<String>,
    column_cwd: Vec<(String, String)>,
    link: Option<Option<TicketLink>>,
}

impl TicketChanges {
    fn parse(config: &BoardConfig, settings: &SettingsArgs) -> Result<Self> {
        let mut column_cwd = Vec::new();
        for entry in &settings.column_cwd {
            let (column, folder) = entry.split_once('=').with_context(|| format!("--column-cwd expects COLUMN=FOLDER, got '{entry}'"))?;
            require_column(config, column.trim())?;
            column_cwd.push((column.trim().to_string(), folder.to_string()));
        }
        let link = match settings.link.as_deref().map(str::trim) {
            None => None,
            Some("") => Some(None),
            Some(url) => Some(Some(link::link_from(url, settings.link_key.as_deref())?)),
        };
        Ok(Self { cwd: settings.cwd.clone(), column_cwd, link })
    }

    fn apply(self, ticket: &mut Ticket) {
        if let Some(cwd) = &self.cwd {
            ticket.set_cwd(cwd);
        }
        for (column, folder) in &self.column_cwd {
            ticket.set_column_cwd(column, folder);
        }
        if let Some(link) = self.link {
            ticket.link = link;
        }
    }

    fn changes_folders(&self) -> bool {
        self.cwd.is_some() || !self.column_cwd.is_empty()
    }
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Validate board.toml and the prompt files it references.
    Check,
    /// Print the JSON Schema for board.toml.
    Schema {
        /// Instead, write board.schema.json next to board.toml and point board.toml at it, for editor completion.
        #[arg(long)]
        write: bool,
    },
    /// Print the configuration reference for this version of Lanes.
    Docs,
}

pub fn ticket(home: &LanesHome, action: &TicketAction) -> Result<()> {
    let board = Board::open(home.clone(), std::env::current_exe()?)?;
    let output = match action {
        TicketAction::List { column, json } => list_tickets(&board, column.as_deref(), *json)?,
        TicketAction::Show { id } => show_ticket(&board, &board.tickets.load(id)?),
        TicketAction::Create { title, id, description, column, settings } => {
            let column = column.clone().unwrap_or_else(|| board.config.first_column().id.clone());
            require_column(&board.config, &column)?;
            let changes = TicketChanges::parse(&board.config, settings)?;
            let mut ticket = board.tickets.create(id.as_deref(), title, description, &column)?;
            changes.apply(&mut ticket);
            board.tickets.save(&ticket)?;
            format!("created {} in {}", ticket.id, board.column_title(&column))
        }
        TicketAction::Move { id, column, force } => move_ticket(&board, id, column, *force)?,
        TicketAction::Edit { id, title, description, settings } => edit_ticket(&board, id, title.as_deref(), description.as_deref(), settings)?,
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
            if !problems.is_empty() {
                bail!("{}", problems.join("\n"))
            }
            for warning in BoardConfig::load(home).map(|config| config_warnings(&config)).unwrap_or_default() {
                println!("warning: {warning}");
            }
            println!("board.toml is valid");
        }
        ConfigAction::Schema { write: false } => println!("{}", config_schema::board_schema()),
        ConfigAction::Schema { write: true } => println!("{}", describe_install(&config_schema::install_schema(home)?)),
        ConfigAction::Docs => println!("{}", config_schema::CONFIG_DOCS),
    }
    Ok(())
}

fn describe_install(install: &config_schema::SchemaInstall) -> String {
    let directive = if install.directive_added {
        format!("added `{}` to the top of board.toml", config_schema::SCHEMA_DIRECTIVE)
    } else {
        "board.toml already points at a schema".into()
    };
    format!("wrote {}; {directive}. Lanes keeps the file up to date from now on.", install.schema_path.display())
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

/// Settings that are valid but have no effect with the board's agent.
pub fn config_warnings(config: &BoardConfig) -> Vec<String> {
    let agent = crate::agent::get(config.defaults.agent);
    let missing_program = config.agent_program(config.defaults.agent).err().map(|err| err.to_string());
    let ignored = agent.ignored_settings();
    let defaults = &config.defaults;
    let board_level = [("model", defaults.model.is_some()), ("effort", defaults.effort.is_some())];
    let mut warnings: Vec<String> =
        board_level.iter().filter(|(key, set)| *set && ignored.contains(key)).map(|(key, _)| format!("{} ignores [defaults] {key}", agent.display_name())).collect();
    for column in &config.columns {
        let set = [("model", column.model.is_some()), ("effort", column.effort.is_some()), ("permission_mode", column.permission_mode.is_some())];
        warnings.extend(
            set.iter().filter(|(key, set)| *set && ignored.contains(key)).map(|(key, _)| format!("{} ignores {key} in column '{}'", agent.display_name(), column.id)),
        );
    }
    if !agent.capabilities().system_prompt && config.columns.iter().any(|c| c.has_session() && c.start_mode(defaults) == crate::config::StartMode::Wait) {
        warnings.push(format!(
            "{} has no system prompt, so new sessions send the stage prompt and brief as the first message even with start = \"wait\"",
            agent.display_name()
        ));
    }
    if config.spend.mode != crate::spend::SpendMode::Off && !agent.capabilities().usage {
        warnings.push(format!("{} doesn't report its context size or token usage, so [spend] has no effect on its sessions", agent.display_name()));
    }
    warnings.extend(missing_program);
    warnings
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
    if let Some(link) = &ticket.link {
        let _ = writeln!(out, "link: {} {}", link.key, link.url);
    }
    if !ticket.description.is_empty() {
        let _ = writeln!(out, "description:\n{}", ticket.description);
    }
    for (column, record) in &ticket.sessions {
        let _ = writeln!(out, "session[{}]: {} in {}", board.column_title(column), record.id, record.cwd.display());
    }
    for column in board.config.columns.iter().filter(|c| c.has_session()) {
        if let (Ok(Some(folder)), Some(source)) = (workdir::resolve(ticket, column), workdir::source(ticket, column)) {
            let _ = writeln!(out, "new {} sessions start in: {} ({})", column.title(), folder.display(), source.describe());
        }
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

fn edit_ticket(board: &Board, id: &str, title: Option<&str>, description: Option<&str>, settings: &SettingsArgs) -> Result<String> {
    let mut ticket = board.tickets.load(id)?;
    let changes = TicketChanges::parse(&board.config, settings)?;
    let note = if changes.changes_folders() { " (folder changes apply to sessions started from now on)" } else { "" };
    if let Some(title) = title {
        ticket.title = title.trim().to_string();
    }
    if let Some(description) = description {
        ticket.description = description.trim().to_string();
    }
    changes.apply(&mut ticket);
    board.tickets.save(&ticket)?;
    Ok(format!("updated {id}{note}"))
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
    fn create_and_edit_set_links_and_bad_links_create_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        std::fs::write(home.board_file(), "[[columns]]\nid = \"backlog\"\n").unwrap();
        let board = Board::open(home.clone(), PathBuf::from("lanes")).unwrap();
        let with_link = |value: &str, key: Option<&str>| SettingsArgs { link: Some(value.into()), link_key: key.map(str::to_string), ..Default::default() };

        let changes = TicketChanges::parse(&board.config, &with_link("https://dev.azure.com/org/p/_workitems/edit/45", None)).unwrap();
        let mut ticket = board.tickets.create(Some("T-5"), "t", "", "backlog").unwrap();
        changes.apply(&mut ticket);
        board.tickets.save(&ticket).unwrap();
        assert_eq!(board.tickets.load("T-5").unwrap().link.unwrap().key, "#45");

        edit_ticket(&board, "T-5", None, None, &with_link("", None)).unwrap();
        assert!(board.tickets.load("T-5").unwrap().link.is_none());
        edit_ticket(&board, "T-5", None, None, &with_link("https://help.example/case/77", Some("SUP-77"))).unwrap();
        assert_eq!(board.tickets.load("T-5").unwrap().link.unwrap().key, "SUP-77");

        assert!(TicketChanges::parse(&board.config, &with_link("PROJ-5", None)).is_err());
        assert_eq!(board.tickets.list().unwrap().len(), 1);
    }

    #[test]
    fn old_jira_links_and_board_sections_still_load() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        std::fs::write(home.board_file(), "[jira]\nsite = \"https://acme.atlassian.net\"\n[[columns]]\nid = \"backlog\"\n").unwrap();
        let board = Board::open(home.clone(), PathBuf::from("lanes")).unwrap();
        board.tickets.create(Some("T-1"), "t", "", "backlog").unwrap();
        let file = home.ticket_file("T-1");
        let old = std::fs::read_to_string(&file).unwrap() + "\n[jira]\nkey = \"PROJ-1\"\nurl = \"https://acme.atlassian.net/browse/PROJ-1\"\n";
        std::fs::write(&file, old).unwrap();
        let mut ticket = board.tickets.load("T-1").unwrap();
        assert_eq!(ticket.link.as_ref().unwrap().key, "PROJ-1");
        board.tickets.save(&ticket).unwrap();
        ticket = board.tickets.load("T-1").unwrap();
        assert!(std::fs::read_to_string(&file).unwrap().contains("[link]"));
        assert_eq!(ticket.link.unwrap().url, "https://acme.atlassian.net/browse/PROJ-1");
    }

    #[test]
    fn warns_about_settings_the_agent_ignores() {
        let config = BoardConfig::parse(
            "[defaults]
agent = \"opencode\"
model = \"opus\"
[agents.opencode]
path = \"C:/oc/opencode.exe\"
[[columns]]
id = \"dev\"
cwd = \".\"
permission_mode = \"plan\"
",
        )
        .unwrap();
        let warnings = config_warnings(&config);
        assert!(warnings.iter().any(|w| w == "OpenCode ignores [defaults] model"), "{warnings:?}");
        assert!(warnings.iter().any(|w| w == "OpenCode ignores permission_mode in column 'dev'"), "{warnings:?}");
        assert!(warnings.iter().any(|w| w.contains("first message")));
        let lean = BoardConfig::parse(
            "[defaults]
agent = \"opencode\"
[agents.opencode]
path = \"oc.exe\"
[spend]
mode = \"lean\"
[[columns]]
id = \"dev\"
",
        )
        .unwrap();
        assert!(config_warnings(&lean).iter().any(|w| w.contains("[spend] has no effect")));
        assert!(
            config_warnings(
                &BoardConfig::parse(
                    "[defaults]
model = \"opus\"
claude_path = \"claude\"
[[columns]]
id = \"dev\"
cwd = \".\"
"
                )
                .unwrap()
            )
            .is_empty()
        );
    }

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
