use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

const MAX_ENTRY_CHARS: usize = 4000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Speaker {
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TranscriptEntry {
    pub speaker: Speaker,
    pub text: String,
}

/// Where Claude Code keeps a session's transcript: the hook-reported path, else the per-cwd project folder, else any project folder.
pub fn locate(session_id: &Uuid, cwd: &Path, reported: Option<&str>) -> Option<PathBuf> {
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

pub fn project_dir_name(cwd: &Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

pub fn load(path: &Path) -> Vec<TranscriptEntry> {
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    parse(&text)
}

pub fn parse(jsonl: &str) -> Vec<TranscriptEntry> {
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

fn entry(speaker: Speaker, text: &str) -> TranscriptEntry {
    let trimmed = text.trim();
    let text = match trimmed.char_indices().nth(MAX_ENTRY_CHARS) {
        Some((cut, _)) => format!("{}… [truncated]", &trimmed[..cut]),
        None => trimmed.to_string(),
    };
    TranscriptEntry { speaker, text }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let entries = parse(&jsonl);
        assert_eq!(
            entries,
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
}
