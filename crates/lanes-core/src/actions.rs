use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use minijinja::{Environment, context};

use crate::artifacts::ArtifactStore;
use crate::config::{ActionDef, ArtifactType, BoardConfig};
use crate::paths::{LanesHome, expand_tilde};
use crate::ticket::Ticket;
use crate::workdir;

const OUTPUT_TAIL_LINES: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAction {
    pub label: String,
    pub command: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionOutcome {
    pub success: bool,
    pub output: String,
}

/// Renders an action's templates against the ticket. The cwd defaults to the ticket's session folder for its column, then the folder a new session would use.
pub fn prepare(home: &LanesHome, config: &BoardConfig, ticket: &Ticket, action: &ActionDef) -> Result<PreparedAction> {
    let column = config.column(&ticket.column);
    let resolved = match column {
        Some(column) => workdir::resolve(ticket, column)?,
        None => None,
    };
    let default_cwd = ticket
        .session(&ticket.column)
        .map(|record| record.cwd.clone())
        .or(resolved)
        .unwrap_or_else(|| home.root().to_path_buf());
    let env = Environment::new();
    let ctx = context! {
        ticket => context! {
            id => &ticket.id,
            title => &ticket.title,
            description => &ticket.description,
            jira_url => ticket.jira.as_ref().map(|j| j.url.as_str()),
        },
        column => context! { id => &ticket.column, title => column.map(|c| c.title()).unwrap_or(&ticket.column) },
        cwd => default_cwd.to_string_lossy(),
        artifacts => artifact_values(home, config, &ticket.id)?,
    };
    let command = env.render_str(&action.command, &ctx).with_context(|| format!("rendering action '{}'", action.label))?;
    let cwd = match &action.cwd {
        Some(template) => expand_tilde(&env.render_str(template, &ctx)?),
        None => default_cwd,
    };
    Ok(PreparedAction { label: action.label.clone(), command, cwd })
}

/// Text artifacts by value, file artifacts by path, for use in action templates.
fn artifact_values(home: &LanesHome, config: &BoardConfig, ticket_id: &str) -> Result<BTreeMap<String, String>> {
    let store = ArtifactStore::new(home, config, ticket_id);
    let mut values = BTreeMap::new();
    for (name, def) in &config.artifacts {
        let value = match def.kind {
            ArtifactType::File => store.file_path(name, def).to_string_lossy().into_owned(),
            ArtifactType::Text => store.read(name)?.unwrap_or_default(),
        };
        values.insert(name.clone(), value);
    }
    Ok(values)
}

pub fn run(action: &PreparedAction) -> Result<ActionOutcome> {
    let output = shell(&action.command)
        .current_dir(&action.cwd)
        .output()
        .with_context(|| format!("running '{}' in {}", action.command, action.cwd.display()))?;
    let combined = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    let lines: Vec<&str> = combined.lines().filter(|l| !l.trim().is_empty()).collect();
    let tail = lines[lines.len().saturating_sub(OUTPUT_TAIL_LINES)..].join("\n");
    Ok(ActionOutcome { success: output.status.success(), output: tail })
}

#[cfg(windows)]
fn shell(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut process = Command::new("cmd");
    process.arg("/C").raw_arg(command).creation_flags(CREATE_NO_WINDOW);
    process
}

#[cfg(not(windows))]
fn shell(command: &str) -> Command {
    let mut process = Command::new("sh");
    process.args(["-c", command]);
    process
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::TicketStore;

    #[test]
    fn renders_templates_and_runs_in_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let board = format!(
            r#"
[artifacts.branch]
type = "text"
[artifacts.plan]
type = "file"
[[columns]]
id = "dev"
cwd = "{}"
[[columns.actions]]
label = "Show branch"
command = "echo {{{{ ticket.id }}}} {{{{ artifacts.branch }}}}"
[[actions]]
label = "Everywhere"
command = "echo hi"
"#,
            crate::paths::portable(&work)
        );
        let config = BoardConfig::parse(&board).unwrap();
        let ticket = TicketStore::new(home.clone()).create(Some("T-7"), "t", "", "dev").unwrap();
        ArtifactStore::new(&home, &config, "T-7").write("branch", "feat/x").unwrap();

        let actions = config.actions_for("dev");
        assert_eq!(actions.iter().map(|a| a.label.as_str()).collect::<Vec<_>>(), ["Show branch", "Everywhere"]);
        let prepared = prepare(&home, &config, &ticket, actions[0]).unwrap();
        assert_eq!(prepared.command, "echo T-7 feat/x");
        assert_eq!(prepared.cwd, work);

        let outcome = run(&prepared).unwrap();
        assert!(outcome.success);
        assert_eq!(outcome.output.trim(), "T-7 feat/x");
    }
}
