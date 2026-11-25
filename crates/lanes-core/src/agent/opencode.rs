//! OpenCode (`opencode`). Its TUI continues or creates a session by id (`--session`, ids start with "ses"), and takes
//! a first message with `--prompt`. It has no system-prompt flag, hooks or status line here, so the stage prompt goes
//! into the first message and cards show "running" rather than live status. Transcripts come from
//! `opencode session export`.

use anyhow::{Result, bail};
use serde_json::Value;
use uuid::Uuid;

use super::{AgentCli, AgentKind, Capabilities, LaunchRequest, TranscriptSource};
use crate::transcript::{Speaker, TranscriptEntry, entry};

pub struct OpenCode;

impl AgentCli for OpenCode {
    fn kind(&self) -> AgentKind {
        AgentKind::OpenCode
    }

    fn display_name(&self) -> &'static str {
        "OpenCode"
    }

    fn default_program(&self) -> &'static str {
        "opencode"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities { live_state: false, usage: false, transcript: true, system_prompt: false }
    }

    fn new_session_id(&self, session_id: Uuid) -> String {
        format!("ses_{}", session_id.simple())
    }

    fn launch_args(&self, request: &LaunchRequest<'_>) -> Result<Vec<String>> {
        let mut args = vec!["--session".to_string(), request.agent_session_id.to_string()];
        args.extend(request.extra_args.iter().cloned());
        if let Some(prompt) = &request.initial_prompt {
            args.extend(["--prompt".to_string(), prompt.clone()]);
        }
        Ok(args)
    }

    fn ignored_settings(&self) -> &'static [&'static str] {
        &["model", "effort", "permission_mode"]
    }

    fn read_transcript(&self, source: &TranscriptSource<'_>) -> Vec<TranscriptEntry> {
        export(source).map(|json| parse_export(&json)).unwrap_or_default()
    }
}

fn export(source: &TranscriptSource<'_>) -> Result<Value> {
    let platform = crate::platform::current();
    let program = platform.resolve_program(source.program);
    let output = platform.quiet_command(&program.to_string_lossy()).args(["session", "export", source.agent_session_id]).current_dir(source.cwd).output()?;
    if !output.status.success() {
        bail!("opencode session export: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

/// OpenCode 2.x exports `{info, messages: [...]}` where a user message has `text` and an assistant message a `content`
/// list; older exports wrapped each message as `{info: {role}, parts: [...]}`. Both are read.
fn parse_export(json: &Value) -> Vec<TranscriptEntry> {
    let messages = json.get("messages").unwrap_or(json).as_array().cloned().unwrap_or_default();
    messages.iter().flat_map(message_entries).collect()
}

fn message_entries(message: &Value) -> Vec<TranscriptEntry> {
    let role = message.get("type").or_else(|| message.pointer("/info/role")).and_then(Value::as_str);
    let speaker = match role {
        Some("user") => Speaker::User,
        Some("assistant") => Speaker::Assistant,
        _ => return Vec::new(),
    };
    if let Some(text) = message.get("text").and_then(Value::as_str) {
        return vec![entry(speaker, text)];
    }
    let blocks = message.get("content").or_else(|| message.get("parts")).and_then(Value::as_array).cloned().unwrap_or_default();
    blocks.iter().filter_map(|block| block_entry(speaker, block)).collect()
}

fn block_entry(speaker: Speaker, block: &Value) -> Option<TranscriptEntry> {
    let flagged = |key: &str| block.get(key).and_then(Value::as_bool).unwrap_or(false);
    let kind = block.get("type")?.as_str()?;
    if kind == "text" {
        let text = block.get("text")?.as_str()?;
        return (!flagged("synthetic") && !flagged("ignored")).then(|| entry(speaker, text));
    }
    if !kind.contains("tool") {
        return None;
    }
    let tool = ["tool", "name"].iter().find_map(|key| block.get(*key).and_then(Value::as_str)).unwrap_or("tool");
    let title = block.pointer("/state/title").or_else(|| block.get("title")).and_then(Value::as_str);
    let detail = title.map(str::to_string).unwrap_or_else(|| summarize_input(block.pointer("/state/input").or_else(|| block.get("input"))));
    Some(entry(Speaker::Tool, &format!("{tool} {detail}")))
}

fn summarize_input(input: Option<&Value>) -> String {
    let Some(Value::Object(fields)) = input else { return String::new() };
    let preferred = ["command", "filePath", "file_path", "pattern", "description", "url"];
    match preferred.iter().find_map(|key| fields.get(*key)).or_else(|| fields.values().next()) {
        Some(Value::String(text)) => text.lines().next().unwrap_or_default().chars().take(160).collect(),
        Some(other) => other.to_string().chars().take(160).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;
    use crate::paths::LanesHome;

    #[test]
    fn sessions_use_opencode_ids_and_the_brief_goes_in_as_the_first_message() {
        let id = Uuid::parse_str("6f0c9d3c-8f6b-4d7f-9a43-2b7c5a0e1f11").unwrap();
        let agent_id = OpenCode.new_session_id(id);
        assert_eq!(agent_id, "ses_6f0c9d3c8f6b4d7f9a432b7c5a0e1f11");
        let home = LanesHome::new(Path::new("."));
        let request = LaunchRequest {
            home: &home,
            lanes_exe: Path::new("lanes"),
            session_id: id,
            agent_session_id: &agent_id,
            resume: false,
            system_prompt_file: None,
            initial_prompt: Some("# Ticket T-1".into()),
            model: Some("opus"),
            effort: None,
            permission_mode: Some("plan"),
            extra_args: &[],
            max_context: None,
        };
        assert_eq!(OpenCode.launch_args(&request).unwrap(), ["--session", agent_id.as_str(), "--prompt", "# Ticket T-1"]);
    }

    #[test]
    fn parses_an_opencode_2_export() {
        let export = json!({
            "info": {"id": "ses_1", "tokens": {"input": 7606}},
            "messages": [
                {"id": "msg_1", "type": "user", "text": "Reply with just the word ok.", "files": []},
                {"id": "msg_2", "type": "assistant", "content": [{"type": "text", "text": "ok"}, {"type": "tool", "name": "bash", "input": {"command": "ls"}}]},
                {"id": "msg_3", "type": "idle", "outcome": "succeeded"}
            ]
        });
        assert_eq!(
            parse_export(&export),
            vec![
                TranscriptEntry { speaker: Speaker::User, text: "Reply with just the word ok.".into() },
                TranscriptEntry { speaker: Speaker::Assistant, text: "ok".into() },
                TranscriptEntry { speaker: Speaker::Tool, text: "bash ls".into() },
            ]
        );
    }

    #[test]
    fn parses_exported_messages_and_tool_calls() {
        let export = json!({
            "info": {"id": "ses_1"},
            "messages": [
                {"info": {"role": "user"}, "parts": [{"type": "text", "text": "Fix the login"}, {"type": "text", "text": "hidden", "synthetic": true}]},
                {"info": {"role": "assistant"}, "parts": [
                    {"type": "step-start"},
                    {"type": "text", "text": "Looking now."},
                    {"type": "tool", "tool": "bash", "state": {"status": "completed", "input": {"command": "git status\nmore"}, "title": "git status"}},
                    {"type": "tool", "tool": "read", "state": {"status": "running", "input": {"filePath": "src/login.ts"}}}
                ]}
            ]
        });
        assert_eq!(
            parse_export(&export),
            vec![
                TranscriptEntry { speaker: Speaker::User, text: "Fix the login".into() },
                TranscriptEntry { speaker: Speaker::Assistant, text: "Looking now.".into() },
                TranscriptEntry { speaker: Speaker::Tool, text: "bash git status".into() },
                TranscriptEntry { speaker: Speaker::Tool, text: "read src/login.ts".into() },
            ]
        );
    }
}
