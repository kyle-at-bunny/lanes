use std::path::Path;

use anyhow::{Context, Result};
use minijinja::{Environment, context};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::config::BoardConfig;
use crate::paths::{HOME_ENV, LanesHome, portable, write_atomic};
use crate::session::{LaunchKind, LaunchSpec, lanes_command, write_session_settings};

pub const PROMPT_FILE: &str = "prompts/assistant.md";
const DEFAULT_PROMPT: &str = include_str!("../assets/prompts/assistant.md");
/// Appended after the user's editable prompt, so command and config docs stay current across upgrades.
const REFERENCE: &str = include_str!("../assets/assistant_reference.md");

#[derive(Debug, Clone, Deserialize, Serialize)]
struct AssistantRecord {
    session_id: Uuid,
}

/// The board assistant: one persistent Claude session in the board folder that operates and reconfigures Lanes.
pub fn prepare(home: &LanesHome, config: &BoardConfig, lanes_exe: &Path) -> Result<LaunchSpec> {
    let lanes = lanes_command(lanes_exe, home);
    let (session_id, kind) = match load_record(home) {
        Some(record) => (record.session_id, LaunchKind::Resume),
        None => (Uuid::new_v4(), LaunchKind::New),
    };
    let mut args = match kind {
        LaunchKind::Resume => vec!["--resume".into(), session_id.to_string()],
        LaunchKind::New => vec!["--session-id".into(), session_id.to_string(), "--append-system-prompt-file".into(), write_system_prompt(home, &lanes)?],
    };
    let settings = write_session_settings(home, &session_id, lanes_exe)?;
    args.extend(["--settings".into(), portable(&settings), "--allowedTools".into(), format!("Bash({lanes}:*)")]);
    if let Some(model) = &config.defaults.model {
        args.extend(["--model".into(), model.clone()]);
    }
    write_atomic(&record_file(home), toml::to_string(&AssistantRecord { session_id })?.as_bytes())?;
    Ok(LaunchSpec {
        session_id,
        kind,
        program: config.defaults.claude_path.clone(),
        args,
        cwd: home.root().to_path_buf(),
        env: vec![(HOME_ENV.into(), home.root().to_string_lossy().into_owned())],
        title: "Board assistant".into(),
        draft: None,
    })
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

fn write_system_prompt(home: &LanesHome, lanes: &str) -> Result<String> {
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
    Ok(portable(&path))
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
        let config = BoardConfig::parse("[[columns]]\nid = \"backlog\"\n").unwrap();
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
