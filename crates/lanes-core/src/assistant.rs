use std::path::Path;

use anyhow::{Context, Result};
use minijinja::{Environment, context};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::agent::{self, AgentKind, LaunchRequest};
use crate::config::BoardConfig;
use crate::paths::{HOME_ENV, LanesHome, portable, write_atomic};
use crate::session::{LaunchKind, LaunchSpec, lanes_command};

pub const PROMPT_FILE: &str = "prompts/assistant.md";
const DEFAULT_PROMPT: &str = include_str!("../assets/prompts/assistant.md");
/// Appended after the user's editable prompt, so command and config docs stay current across upgrades.
const REFERENCE: &str = include_str!("../assets/assistant_reference.md");

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AssistantRecord {
    session_id: Uuid,
    #[serde(default)]
    agent: AgentKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_session_id: Option<String>,
}

/// The board assistant: one persistent agent session in the board folder that operates and reconfigures Lanes.
pub fn prepare(home: &LanesHome, config: &BoardConfig, lanes_exe: &Path) -> Result<LaunchSpec> {
    let lanes = lanes_command(lanes_exe, home);
    let record = load_record(home);
    let kind = if record.is_some() { LaunchKind::Resume } else { LaunchKind::New };
    let record = match record {
        Some(record) => record,
        None => new_record(config.defaults.agent),
    };
    let agent = agent::get(record.agent);
    let agent_session_id = record.agent_session_id.clone().unwrap_or_else(|| record.session_id.to_string());
    let system_prompt = match kind {
        LaunchKind::New => Some(write_system_prompt(home, &lanes)?),
        LaunchKind::Resume => None,
    };
    let extra_args = agent.assistant_args(&lanes);
    let request = LaunchRequest {
        home,
        lanes_exe,
        session_id: record.session_id,
        agent_session_id: &agent_session_id,
        resume: kind == LaunchKind::Resume,
        system_prompt_file: system_prompt.as_deref(),
        initial_prompt: None,
        model: config.defaults.model.as_deref(),
        effort: None,
        permission_mode: None,
        extra_args: &extra_args,
        max_context: None,
    };
    let args = agent.launch_args(&request)?;
    write_atomic(&record_file(home), toml::to_string(&record)?.as_bytes())?;
    Ok(LaunchSpec {
        session_id: record.session_id,
        kind,
        program: config.agent_program(record.agent)?,
        args,
        cwd: home.root().to_path_buf(),
        env: vec![(HOME_ENV.into(), home.root().to_string_lossy().into_owned())],
        title: "Board assistant".into(),
        draft: None,
        agent: record.agent,
        agent_session_id: record.agent_session_id.clone(),
    })
}

fn new_record(kind: AgentKind) -> AssistantRecord {
    let session_id = Uuid::new_v4();
    let agent_session_id = agent::get(kind).new_session_id(session_id);
    AssistantRecord { session_id, agent: kind, agent_session_id: (agent_session_id != session_id.to_string()).then_some(agent_session_id) }
}

/// Forgets the assistant's session so the next launch starts a fresh conversation.
pub fn reset(home: &LanesHome) -> Result<()> {
    match std::fs::remove_file(record_file(home)) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
        _ => Ok(()),
    }
}

pub fn current_session(home: &LanesHome) -> Option<Uuid> {
    load_record(home).map(|r| r.session_id)
}

/// The assistant's session: Lanes' id, its agent and the agent's own id.
pub fn current(home: &LanesHome) -> Option<(Uuid, AgentKind, String)> {
    load_record(home).map(|r| (r.session_id, r.agent, r.agent_session_id.unwrap_or_else(|| r.session_id.to_string())))
}

fn write_system_prompt(home: &LanesHome, lanes: &str) -> Result<std::path::PathBuf> {
    let template_path = home.resolve(PROMPT_FILE);
    if !template_path.exists() {
        write_atomic(&template_path, DEFAULT_PROMPT.as_bytes())?;
    }
    let template = format!("{}\n\n{REFERENCE}", std::fs::read_to_string(&template_path)?.trim_end());
    let rendered = Environment::new()
        .render_str(&template, context! { lanes => lanes, home => portable(home.root()), board_file => portable(&home.board_file()) })
        .context("rendering the assistant prompt")?;
    let path = home.state_dir().join("assistant.system.md");
    write_atomic(&path, rendered.as_bytes())?;
    Ok(path)
}

fn record_file(home: &LanesHome) -> std::path::PathBuf {
    home.root().join("assistant.toml")
}

fn load_record(home: &LanesHome) -> Option<AssistantRecord> {
    toml::from_str(&std::fs::read_to_string(record_file(home)).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_launch_creates_session_then_resumes_until_reset() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let config = BoardConfig::parse("[defaults]\nclaude_path = \"claude\"\n[[columns]]\nid = \"backlog\"\n").unwrap();
        let exe = Path::new("C:/bin/lanes.exe");

        let first = prepare(&home, &config, exe).unwrap();
        assert_eq!(first.kind, LaunchKind::New);
        assert_eq!(first.cwd, dir.path());
        assert!(first.args.iter().any(|a| a.starts_with("Bash(C:/bin/lanes.exe --home ") && a.ends_with(":*)")));
        let prompt = std::fs::read_to_string(home.state_dir().join("assistant.system.md")).unwrap();
        assert!(prompt.contains("C:/bin/lanes.exe --home") && prompt.contains("board.toml"));
        assert!(prompt.contains("{{ ticket.id }}"), "literal template syntax survives rendering");
        assert!(prompt.contains("You are the Lanes board assistant") && prompt.contains("--link <work item URL>"), "user prompt then built-in reference");
        assert!(home.resolve(PROMPT_FILE).exists());

        let again = prepare(&home, &config, exe).unwrap();
        assert_eq!((again.kind, again.session_id), (LaunchKind::Resume, first.session_id));
        assert_eq!(current_session(&home), Some(first.session_id));

        std::fs::write(home.resolve(PROMPT_FILE), "Custom house rules.").unwrap();
        reset(&home).unwrap();
        let fresh = prepare(&home, &config, exe).unwrap();
        assert_eq!(fresh.kind, LaunchKind::New);
        assert_ne!(fresh.session_id, first.session_id);
        let customised = std::fs::read_to_string(home.state_dir().join("assistant.system.md")).unwrap();
        assert!(customised.starts_with("Custom house rules.") && customised.contains("# Lanes reference"));
    }
}
