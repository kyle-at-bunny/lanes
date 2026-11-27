//! Claude Code: Lanes picks the session id, injects hooks and a status line per launch with `--settings`, and reads
//! transcripts from `~/.claude/projects`.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde_json::{Value, json};
use uuid::Uuid;

use super::{AgentCli, AgentKind, Capabilities, LaunchRequest, TranscriptSource};
use crate::paths::{LanesHome, now_unix, portable, write_atomic};
use crate::session::lanes_command;
use crate::state::{AgentStatus, HookState, Limit, StatusSnapshot};
use crate::transcript::{Speaker, TranscriptEntry, entry};

const DEFAULT_RETENTION_DAYS: u64 = 30;

pub struct Claude;

impl AgentCli for Claude {
    fn kind(&self) -> AgentKind {
        AgentKind::Claude
    }

    fn display_name(&self) -> &'static str {
        "Claude"
    }

    fn default_program(&self) -> &'static str {
        "claude"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { live_state: true, usage: true, transcript: true, system_prompt: true }
    }

    /// Claude Code doesn't keep these flags with a resumed session, so they go on every launch.
    fn launch_args(&self, request: &LaunchRequest<'_>) -> Result<Vec<String>> {
        let mut args = match (request.resume, request.system_prompt_file) {
            (true, _) => vec!["--resume".into(), request.agent_session_id.to_string()],
            (false, Some(file)) => vec!["--session-id".into(), request.agent_session_id.to_string(), "--append-system-prompt-file".into(), portable(file)],
            (false, None) => vec!["--session-id".into(), request.agent_session_id.to_string()],
        };
        let settings = write_session_settings(request.home, &request.session_id, request.lanes_exe)?;
        args.extend(["--settings".into(), portable(&settings)]);
        let optional = [("--model", request.model), ("--effort", request.effort), ("--permission-mode", request.permission_mode)];
        for (flag, value) in optional {
            if let Some(value) = value {
                args.extend([flag.to_string(), value.to_string()]);
            }
        }
        args.extend(request.extra_args.iter().cloned());
        args.extend(request.initial_prompt.clone());
        Ok(args)
    }

    fn assistant_args(&self, lanes_command: &str) -> Vec<String> {
        vec!["--allowedTools".into(), format!("Bash({lanes_command}:*)")]
    }

    fn parse_hook(&self, event: &Value) -> Option<(Uuid, HookState)> {
        Some((session_id_of(event)?, hook_state(event)?))
    }

    fn edited_file<'v>(&self, event: &'v Value) -> Option<&'v str> {
        edited_file(event)
    }

    fn parse_status_line(&self, input: &Value) -> Option<(Uuid, StatusSnapshot)> {
        Some((session_id_of(input)?, status_snapshot(input)))
    }

    /// Chains to the user's own status line command when they have one, so Lanes doesn't replace it.
    fn status_line_text(&self, raw: &str, snapshot: &StatusSnapshot) -> String {
        match user_status_line_command() {
            Some(command) => run_user_status_line(&command, raw).unwrap_or_else(|_| default_status_line(snapshot)),
            None => default_status_line(snapshot),
        }
    }

    fn transcript_file(&self, source: &TranscriptSource<'_>) -> Option<PathBuf> {
        locate(source.agent_session_id, source.cwd, source.reported_path)
    }

    fn read_transcript(&self, source: &TranscriptSource<'_>) -> Vec<TranscriptEntry> {
        self.transcript_file(source).map(|path| load(&path)).unwrap_or_default()
    }

    /// `cleanupPeriodDays` in ~/.claude/settings.json.
    fn retention_days(&self) -> Option<u64> {
        let read = || -> Option<u64> {
            let settings: Value = serde_json::from_slice(&std::fs::read(claude_settings_file()?).ok()?).ok()?;
            settings.get("cleanupPeriodDays")?.as_u64()
        };
        Some(read().unwrap_or(DEFAULT_RETENTION_DAYS))
    }

    /// Only when Lanes runs inside a Claude Code session (`CLAUDECODE` is set): its session, SDK and MCP variables make child sessions misbehave.
    fn leaked_env_vars(&self, names: &[String]) -> Vec<String> {
        if !names.iter().any(|name| name.eq_ignore_ascii_case("CLAUDECODE")) {
            return Vec::new();
        }
        let leaked = |name: &str| {
            let upper = name.to_ascii_uppercase();
            ["CLAUDECODE", "CLAUDE_PID", "ANTHROPIC_BASE_URL"].contains(&upper.as_str())
                || ["CLAUDE_CODE_", "CLAUDE_AGENT_SDK_", "MCP_"].iter().any(|prefix| upper.starts_with(prefix))
        };
        names.iter().filter(|name| leaked(name)).cloned().collect()
    }
}

const HOOK_EVENTS: &[&str] = &["SessionStart", "UserPromptSubmit", "PostToolUse", "Notification", "Stop", "StopFailure", "SessionEnd"];

/// Per-session settings injected with `--settings`: state hooks plus the status line feed.
pub fn write_session_settings(home: &LanesHome, session_id: &Uuid, lanes_exe: &Path) -> Result<PathBuf> {
    let base = lanes_command(lanes_exe, home);
    let hook = json!([{ "hooks": [{ "type": "command", "command": format!("{base} hook"), "timeout": 10 }] }]);
    let hooks: serde_json::Map<_, _> = HOOK_EVENTS.iter().map(|event| (event.to_string(), hook.clone())).collect();
    let settings = json!({
        "hooks": hooks,
        "statusLine": { "type": "command", "command": format!("{base} statusline"), "padding": 0 },
    });
    let path = home.session_settings_file(session_id);
    write_atomic(&path, serde_json::to_string_pretty(&settings)?.as_bytes())?;
    Ok(path)
}

fn session_id_of(input: &Value) -> Option<Uuid> {
    input.get("session_id")?.as_str()?.parse().ok()
}

/// Maps a Claude Code hook payload to the agent state it implies.
fn hook_state(event: &Value) -> Option<HookState> {
    let name = event.get("hook_event_name")?.as_str()?;
    let notification = event.get("notification_type").and_then(Value::as_str);
    let status = match (name, notification) {
        ("SessionStart", _) => AgentStatus::Ready,
        ("UserPromptSubmit" | "PostToolUse", _) => AgentStatus::Working,
        ("Notification", Some("permission_prompt" | "elicitation_dialog")) => AgentStatus::NeedsInput,
        ("Notification", Some("idle_prompt")) | ("Stop" | "StopFailure", _) => AgentStatus::Idle,
        ("SessionEnd", _) => AgentStatus::Exited,
        _ => return None,
    };
    Some(HookState {
        status,
        at: now_unix(),
        detail: ["message", "error"].iter().find_map(|key| event.get(*key).and_then(Value::as_str)).map(str::to_string),
        transcript_path: event.get("transcript_path").and_then(Value::as_str).map(str::to_string),
    })
}

/// The file a `PostToolUse` hook reports Claude as having written or edited.
fn edited_file(event: &Value) -> Option<&str> {
    if event.get("hook_event_name")?.as_str()? != "PostToolUse" {
        return None;
    }
    let tool = event.get("tool_name")?.as_str()?;
    if !matches!(tool, "Write" | "Edit" | "MultiEdit") {
        return None;
    }
    event.pointer("/tool_input/file_path")?.as_str()
}

fn status_snapshot(input: &Value) -> StatusSnapshot {
    let limit = |key: &str| {
        let window = input.get("rate_limits")?.get(key)?;
        Some(Limit { used_pct: window.get("used_percentage")?.as_f64()?, resets_at: window.get("resets_at").and_then(Value::as_u64) })
    };
    StatusSnapshot {
        at: now_unix(),
        context_used_pct: input.pointer("/context_window/used_percentage").and_then(Value::as_f64),
        model: input.pointer("/model/display_name").and_then(Value::as_str).map(str::to_string),
        five_hour: limit("five_hour"),
        seven_day: limit("seven_day"),
    }
}

fn default_status_line(snapshot: &StatusSnapshot) -> String {
    let mut parts = vec![snapshot.model.clone().unwrap_or_else(|| "claude".into())];
    if let Some(ctx) = snapshot.context_used_pct {
        parts.push(format!("ctx {ctx:.0}%"));
    }
    if let Some(limit) = snapshot.five_hour {
        let reset = limit.resets_at.map(|at| format!(" (resets in {})", crate::cli::format_duration(at.saturating_sub(now_unix())))).unwrap_or_default();
        parts.push(format!("5h {:.0}%{reset}", limit.used_pct));
    }
    parts.join(" · ")
}

fn claude_settings_file() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude").join("settings.json"))
}

fn user_status_line_command() -> Option<String> {
    let settings: Value = serde_json::from_slice(&std::fs::read(claude_settings_file()?).ok()?).ok()?;
    settings.pointer("/statusLine/command")?.as_str().map(str::to_string)
}

/// Claude Code runs status line commands with bash, so the chained one does too.
fn run_user_status_line(command: &str, input: &str) -> Result<String> {
    let mut child = Command::new("bash").args(["-c", command]).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn()?;
    child.stdin.take().context("no stdin")?.write_all(input.as_bytes())?;
    let output = child.wait_with_output()?;
    Ok(String::from_utf8_lossy(&output.stdout).trim_end().to_string())
}

/// Where Claude Code keeps a session's transcript: the hook-reported path, else the per-cwd project folder, else any project folder.
fn locate(session_id: &str, cwd: &Path, reported: Option<&str>) -> Option<PathBuf> {
    if let Some(path) = reported.map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let projects = dirs::home_dir()?.join(".claude").join("projects");
    let file_name = format!("{session_id}.jsonl");
    let expected = projects.join(project_dir_name(cwd)).join(&file_name);
    if expected.is_file() {
        return Some(expected);
    }
    std::fs::read_dir(&projects).ok()?.filter_map(Result::ok).map(|e| e.path().join(&file_name)).find(|p| p.is_file())
}

fn project_dir_name(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

fn load(path: &Path) -> Vec<TranscriptEntry> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    parse(&text)
}

fn parse(jsonl: &str) -> Vec<TranscriptEntry> {
    jsonl.lines().filter_map(|line| serde_json::from_str::<Value>(line).ok()).flat_map(|line| entries_for(&line)).collect()
}

fn entries_for(line: &Value) -> Vec<TranscriptEntry> {
    let is_flagged = |key: &str| line.get(key).and_then(Value::as_bool).unwrap_or(false);
    let speaker = match line.get("type").and_then(Value::as_str) {
        Some("user") => Speaker::User,
        Some("assistant") => Speaker::Assistant,
        _ => return Vec::new(),
    };
    if is_flagged("isMeta") || is_flagged("isSidechain") {
        return Vec::new();
    }
    match line.pointer("/message/content") {
        Some(Value::String(text)) => vec![entry(speaker, text)],
        Some(Value::Array(blocks)) => blocks.iter().filter_map(|block| block_entry(speaker, block)).collect(),
        _ => Vec::new(),
    }
}

fn block_entry(speaker: Speaker, block: &Value) -> Option<TranscriptEntry> {
    match block.get("type")?.as_str()? {
        "text" => Some(entry(speaker, block.get("text")?.as_str()?)),
        "tool_use" => {
            let name = block.get("name")?.as_str()?;
            Some(entry(Speaker::Tool, &format!("{name} {}", summarize_input(block.get("input")))))
        }
        _ => None,
    }
}

fn summarize_input(input: Option<&Value>) -> String {
    let Some(Value::Object(fields)) = input else { return String::new() };
    let preferred = ["command", "file_path", "pattern", "description", "url", "prompt"];
    let value = preferred.iter().find_map(|key| fields.get(*key)).or_else(|| fields.values().next());
    match value {
        Some(Value::String(text)) => text.lines().next().unwrap_or_default().chars().take(160).collect(),
        Some(other) => other.to_string().chars().take(160).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_a_parent_claude_sessions_variables_only_inside_one() {
        let names = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let inside = names(&["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "MCP_TIMEOUT", "ANTHROPIC_BASE_URL", "PATH", "ANTHROPIC_API_KEY"]);
        assert_eq!(Claude.leaked_env_vars(&inside), names(&["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT", "MCP_TIMEOUT", "ANTHROPIC_BASE_URL"]));
        assert!(Claude.leaked_env_vars(&names(&["CLAUDE_CODE_ENTRYPOINT", "PATH"])).is_empty());
    }

    #[test]
    fn maps_hook_events_to_states() {
        let status = |v: Value| hook_state(&v).map(|s| s.status);
        assert_eq!(status(json!({"hook_event_name": "UserPromptSubmit"})), Some(AgentStatus::Working));
        assert_eq!(status(json!({"hook_event_name": "Notification", "notification_type": "permission_prompt"})), Some(AgentStatus::NeedsInput));
        assert_eq!(status(json!({"hook_event_name": "Stop"})), Some(AgentStatus::Idle));
        assert_eq!(status(json!({"hook_event_name": "StopFailure", "error": "auth"})), Some(AgentStatus::Idle));
        assert_eq!(status(json!({"hook_event_name": "PreCompact"})), None);
        let id = "6f0c9d3c-8f6b-4d7f-9a43-2b7c5a0e1f11";
        let (session, state) = Claude.parse_hook(&json!({"session_id": id, "hook_event_name": "Stop"})).unwrap();
        assert_eq!((session.to_string(), state.status), (id.to_string(), AgentStatus::Idle));
    }

    #[test]
    fn finds_files_written_by_tools() {
        let event = |tool: &str| json!({"hook_event_name": "PostToolUse", "tool_name": tool, "tool_input": {"file_path": "C:\\b\\plan.md"}});
        assert_eq!(edited_file(&event("Write")), Some("C:\\b\\plan.md"));
        assert_eq!(edited_file(&event("Edit")), Some("C:\\b\\plan.md"));
        assert_eq!(edited_file(&event("Read")), None);
        assert_eq!(edited_file(&json!({"hook_event_name": "Stop", "tool_name": "Write"})), None);
    }

    #[test]
    fn parses_status_line_input() {
        let input = json!({
            "session_id": "6f0c9d3c-8f6b-4d7f-9a43-2b7c5a0e1f11",
            "model": {"display_name": "Opus"},
            "context_window": {"used_percentage": 42.5},
            "rate_limits": {"five_hour": {"used_percentage": 31.0, "resets_at": 1790000000u64}, "seven_day": {"used_percentage": 12.0}}
        });
        let (_, snapshot) = Claude.parse_status_line(&input).unwrap();
        assert_eq!(snapshot.context_used_pct, Some(42.5));
        assert_eq!(snapshot.model.as_deref(), Some("Opus"));
        assert_eq!(snapshot.five_hour, Some(Limit { used_pct: 31.0, resets_at: Some(1790000000) }));
        assert!(Claude.parse_status_line(&json!({"model": {}})).is_none());
    }

    #[test]
    fn parses_messages_tools_and_skips_noise() {
        let jsonl = [
            r#"{"type":"mode","mode":"normal"}"#,
            r##"{"type":"user","message":{"role":"user","content":"# Ticket T-1: Do it"}}"##,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"hidden"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"On it."},{"type":"tool_use","name":"Bash","input":{"command":"git status\nmore"}}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"clean"}]}}"#,
            r#"{"type":"assistant","isSidechain":true,"message":{"content":"subagent"}}"#,
            "garbage",
        ]
        .join("\n");
        assert_eq!(
            parse(&jsonl),
            vec![
                TranscriptEntry { speaker: Speaker::User, text: "# Ticket T-1: Do it".into() },
                TranscriptEntry { speaker: Speaker::Assistant, text: "On it.".into() },
                TranscriptEntry { speaker: Speaker::Tool, text: "Bash git status".into() },
            ]
        );
    }

    #[test]
    fn project_dir_matches_claude_encoding() {
        assert_eq!(project_dir_name(Path::new(r"C:\Users\me\projects\app")), "C--Users-me-projects-app");
    }

    #[test]
    fn launch_args_put_the_initial_prompt_last() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let id = Uuid::new_v4();
        let extra = vec!["--add-dir".to_string(), "x".to_string()];
        let request = LaunchRequest {
            home: &home,
            lanes_exe: Path::new("lanes"),
            session_id: id,
            agent_session_id: &id.to_string(),
            resume: true,
            system_prompt_file: None,
            initial_prompt: Some("hello".into()),
            model: Some("opus"),
            effort: None,
            permission_mode: Some("plan"),
            extra_args: &extra,
        };
        let args = Claude.launch_args(&request).unwrap();
        assert_eq!(&args[..2], ["--resume".to_string(), id.to_string()]);
        assert_eq!(&args[2..3], ["--settings".to_string()]);
        assert_eq!(&args[4..], ["--model", "opus", "--permission-mode", "plan", "--add-dir", "x", "hello"]);
    }
}
