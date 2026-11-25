//! Token usage for one Claude Code session, read from its transcript and its subagents' transcripts.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use uuid::Uuid;

/// Relative API prices, so different token kinds add up to one "input-token equivalent" figure.
const WEIGHT_INPUT: f64 = 1.0;
const WEIGHT_WRITE_5M: f64 = 1.25;
const WEIGHT_WRITE_1H: f64 = 2.0;
const WEIGHT_READ: f64 = 0.1;
const WEIGHT_OUTPUT: f64 = 5.0;
/// A main-thread request after this long idle that mostly rewrites its context is a cold cache rebuild.
const COLD_GAP_MS: i64 = 60 * 60 * 1000;
const REBUILD_MIN_CONTEXT: u64 = 20_000;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Usage {
    pub requests: u64,
    pub input: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
    pub output: u64,
    pub peak_context: u64,
    pub cold_rebuilds: u64,
    pub cold_rebuild_tokens: u64,
}

impl Usage {
    /// Spend in input-token equivalents at API price ratios.
    pub fn weighted(&self) -> f64 {
        self.input as f64 * WEIGHT_INPUT
            + self.cache_write_5m as f64 * WEIGHT_WRITE_5M
            + self.cache_write_1h as f64 * WEIGHT_WRITE_1H
            + self.cache_read as f64 * WEIGHT_READ
            + self.output as f64 * WEIGHT_OUTPUT
    }

    pub fn add(&mut self, other: &Usage) {
        self.requests += other.requests;
        self.input += other.input;
        self.cache_write_5m += other.cache_write_5m;
        self.cache_write_1h += other.cache_write_1h;
        self.cache_read += other.cache_read;
        self.output += other.output;
        self.peak_context = self.peak_context.max(other.peak_context);
        self.cold_rebuilds += other.cold_rebuilds;
        self.cold_rebuild_tokens += other.cold_rebuild_tokens;
    }

    fn record(&mut self, request: &Request) {
        self.requests += 1;
        self.input += request.input;
        self.cache_write_5m += request.write_5m;
        self.cache_write_1h += request.write_1h;
        self.cache_read += request.read;
        self.output += request.output;
        self.peak_context = self.peak_context.max(request.context());
    }
}

#[derive(Debug, Clone, Default)]
struct Request {
    at_ms: i64,
    input: u64,
    write_5m: u64,
    write_1h: u64,
    read: u64,
    output: u64,
}

impl Request {
    fn context(&self) -> u64 {
        self.input + self.write_5m + self.write_1h + self.read
    }

    fn is_rebuild(&self) -> bool {
        self.context() >= REBUILD_MIN_CONTEXT && self.read * 2 < self.context()
    }
}

/// Usage for a session, or `None` when Claude Code has no transcript for it (never run, or cleaned up).
pub fn session_usage(session_id: &Uuid) -> Option<Usage> {
    let main = find_transcript(&claude_projects_dir()?, session_id)?;
    let mut usage = thread_usage(&std::fs::read_to_string(&main).ok()?, true);
    for subagent in subagent_transcripts(&main) {
        if let Ok(text) = std::fs::read_to_string(subagent) {
            usage.add(&thread_usage(&text, false));
        }
    }
    Some(usage)
}

/// Usage of one transcript; cold rebuilds are only counted on the main thread, where idle gaps are the user's.
pub fn thread_usage(transcript: &str, main_thread: bool) -> Usage {
    let requests = parse_requests(transcript);
    let mut usage = Usage::default();
    let mut previous_at: Option<i64> = None;
    for request in &requests {
        usage.record(request);
        let cold = previous_at.is_some_and(|at| request.at_ms - at >= COLD_GAP_MS) && request.is_rebuild();
        if main_thread && cold {
            usage.cold_rebuilds += 1;
            usage.cold_rebuild_tokens += request.write_5m + request.write_1h + request.input;
        }
        previous_at = Some(request.at_ms);
    }
    usage
}

/// Assistant entries in time order, once per API message: Claude Code writes one entry per content block with the same usage.
fn parse_requests(transcript: &str) -> Vec<Request> {
    let mut by_message: HashMap<String, Request> = HashMap::new();
    for line in transcript.lines().filter(|l| l.contains("\"assistant\"") && l.contains("\"usage\"")) {
        let Ok(entry) = serde_json::from_str::<Value>(line) else { continue };
        if entry.get("type").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(message_id) = entry.pointer("/message/id").and_then(Value::as_str) else { continue };
        let Some(usage) = entry.pointer("/message/usage") else { continue };
        let at_ms = entry.get("timestamp").and_then(Value::as_str).and_then(parse_timestamp_ms).unwrap_or(0);
        by_message.insert(message_id.to_string(), request_from(usage, at_ms));
    }
    let mut requests: Vec<_> = by_message.into_values().collect();
    requests.sort_by_key(|r| r.at_ms);
    requests
}

fn request_from(usage: &Value, at_ms: i64) -> Request {
    let field = |path: &str| usage.pointer(path).and_then(Value::as_u64);
    let total_write = field("/cache_creation_input_tokens").unwrap_or(0);
    let write_5m = field("/cache_creation/ephemeral_5m_input_tokens");
    let write_1h = field("/cache_creation/ephemeral_1h_input_tokens");
    let (write_5m, write_1h) = match (write_5m, write_1h) {
        (None, None) => (total_write, 0),
        (five, one) => (five.unwrap_or(0), one.unwrap_or(0)),
    };
    Request {
        at_ms,
        input: field("/input_tokens").unwrap_or(0),
        write_5m,
        write_1h,
        read: field("/cache_read_input_tokens").unwrap_or(0),
        output: field("/output_tokens").unwrap_or(0),
    }
}

fn claude_projects_dir() -> Option<PathBuf> {
    let config = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).or_else(|| dirs::home_dir().map(|h| h.join(".claude")))?;
    Some(config.join("projects"))
}

/// Claude Code files each session under a folder named after its working directory, so look in all of them.
fn find_transcript(projects: &Path, session_id: &Uuid) -> Option<PathBuf> {
    let file_name = format!("{session_id}.jsonl");
    std::fs::read_dir(projects).ok()?.filter_map(Result::ok).map(|entry| entry.path().join(&file_name)).find(|path| path.is_file())
}

fn subagent_transcripts(main: &Path) -> Vec<PathBuf> {
    let dir = main.with_extension("").join("subagents");
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|ext| ext == "jsonl")).collect()
}

/// Parses Claude Code's `2026-11-24T08:54:47.032Z` timestamps to Unix milliseconds.
fn parse_timestamp_ms(text: &str) -> Option<i64> {
    let (date, time) = text.strip_suffix('Z')?.split_once('T')?;
    let mut ymd = date.split('-').map(|p| p.parse::<i64>().ok());
    let (year, month, day) = (ymd.next()??, ymd.next()??, ymd.next()??);
    let (hms, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut parts = hms.split(':').map(|p| p.parse::<i64>().ok());
    let (hour, minute, second) = (parts.next()??, parts.next()??, parts.next()??);
    let millis = format!("{fraction:0<3}")[..3].parse::<i64>().ok()?;
    let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    Some(seconds * 1000 + millis)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(id: &str, at: &str, write_1h: u64, read: u64, output: u64) -> String {
        json!({
            "type": "assistant",
            "timestamp": at,
            "message": {"id": id, "usage": {
                "input_tokens": 10, "cache_creation_input_tokens": write_1h, "cache_read_input_tokens": read, "output_tokens": output,
                "cache_creation": {"ephemeral_5m_input_tokens": 0, "ephemeral_1h_input_tokens": write_1h}
            }}
        })
        .to_string()
    }

    #[test]
    fn parses_timestamps() {
        assert_eq!(parse_timestamp_ms("1970-01-01T00:00:00.000Z"), Some(0));
        assert_eq!(parse_timestamp_ms("2026-11-24T08:54:47.032Z"), Some(1_795_510_487_032));
        assert_eq!(parse_timestamp_ms("2024-02-29T12:00:00Z"), Some(1_709_208_000_000));
        assert_eq!(parse_timestamp_ms("yesterday"), None);
    }

    #[test]
    fn counts_each_message_once_and_flags_cold_rebuilds() {
        let transcript = [
            entry("m1", "2026-11-24T08:00:00.000Z", 30_000, 0, 100),
            entry("m1", "2026-11-24T08:00:00.500Z", 30_000, 0, 100),
            entry("m2", "2026-11-24T08:05:00.000Z", 2_000, 30_000, 50),
            r#"{"type":"user","message":{"content":"hi"}}"#.to_string(),
            entry("m3", "2026-11-24T10:00:00.000Z", 40_000, 0, 50),
            entry("m4", "2026-11-24T11:30:00.000Z", 500, 41_000, 50),
        ]
        .join("\n");
        let usage = thread_usage(&transcript, true);
        assert_eq!(usage.requests, 4);
        assert_eq!(usage.cache_write_1h, 72_500);
        assert_eq!(usage.peak_context, 41_510);
        assert_eq!((usage.cold_rebuilds, usage.cold_rebuild_tokens), (1, 40_010), "m1 starts the session and m4 reads a warm cache");
        assert_eq!(thread_usage(&transcript, false).cold_rebuilds, 0);
    }

    #[test]
    fn weights_token_kinds_by_price() {
        let usage = Usage { input: 100, cache_write_5m: 100, cache_write_1h: 100, cache_read: 1000, output: 10, ..Default::default() };
        assert_eq!(usage.weighted(), 100.0 + 125.0 + 200.0 + 100.0 + 50.0);
    }

    #[test]
    fn older_transcripts_without_a_write_breakdown_count_as_5m_writes() {
        let usage = thread_usage(r#"{"type":"assistant","timestamp":"2026-01-01T00:00:00Z","message":{"id":"a","usage":{"cache_creation_input_tokens":80}}}"#, true);
        assert_eq!((usage.cache_write_5m, usage.cache_write_1h), (80, 0));
    }

    #[test]
    fn finds_transcripts_and_subagents_in_any_project_folder() {
        let dir = tempfile::tempdir().unwrap();
        let id = Uuid::new_v4();
        let project = dir.path().join("C--work-repo");
        std::fs::create_dir_all(project.join(id.to_string()).join("subagents")).unwrap();
        std::fs::write(project.join(format!("{id}.jsonl")), "").unwrap();
        std::fs::write(project.join(id.to_string()).join("subagents").join("agent-1.jsonl"), "").unwrap();
        let main = find_transcript(dir.path(), &id).unwrap();
        assert_eq!(main, project.join(format!("{id}.jsonl")));
        assert_eq!(subagent_transcripts(&main).len(), 1);
        assert!(find_transcript(dir.path(), &Uuid::new_v4()).is_none());
    }
}
