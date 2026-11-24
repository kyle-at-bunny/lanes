use std::collections::{BTreeSet, HashMap, HashSet};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::artifact_view::{Segment, segments};
use crate::artifacts::ArtifactStore;
use crate::config::BoardConfig;
use crate::paths::LanesHome;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrState {
    Open,
    Draft,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Checks {
    None,
    Passing,
    Pending,
    Failing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    pub url: String,
    pub number: u64,
    pub state: PrState,
    pub checks: Checks,
}

impl PrStatus {
    pub fn is_open(&self) -> bool {
        matches!(self.state, PrState::Open | PrState::Draft)
    }

    /// e.g. "open · checks failing", "draft", "merged".
    pub fn label(&self) -> String {
        let state = match self.state {
            PrState::Open => "open",
            PrState::Draft => "draft",
            PrState::Merged => "merged",
            PrState::Closed => "closed",
        };
        let checks = match (self.is_open(), self.checks) {
            (false, _) | (_, Checks::None) => "",
            (true, Checks::Passing) => " · checks passing",
            (true, Checks::Pending) => " · checks running",
            (true, Checks::Failing) => " · checks failing",
        };
        format!("{state}{checks}")
    }
}

/// The canonical `https://<host>/<owner>/<repo>/pull/<n>` form of a pull request URL, or None if it isn't one.
pub fn normalize_pr_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let parts: Vec<&str> = rest.split(['/', '?', '#']).collect();
    let [host, owner, repo, "pull", number, ..] = parts.as_slice() else { return None };
    let valid = !host.is_empty() && !owner.is_empty() && !repo.is_empty() && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    valid.then(|| format!("https://{host}/{owner}/{repo}/pull/{number}"))
}

/// Every pull request linked from the ticket's artifacts, in order of first appearance.
pub fn pr_urls(home: &LanesHome, config: &BoardConfig, ticket_id: &str) -> Vec<String> {
    let store = ArtifactStore::new(home, config, ticket_id);
    let mut seen = BTreeSet::new();
    config
        .artifacts
        .keys()
        .filter_map(|name| store.read(name).ok().flatten())
        .flat_map(|content| content.lines().flat_map(segments).collect::<Vec<_>>())
        .filter_map(|segment| match segment {
            Segment::Url(url) => normalize_pr_url(&url),
            _ => None,
        })
        .filter(|url| seen.insert(url.clone()))
        .collect()
}

/// Asks the GitHub CLI for the pull request's state and check results.
pub fn fetch(url: &str) -> Result<PrStatus> {
    let output =
        gh().args(["pr", "view", url, "--json", "number,state,isDraft,statusCheckRollup"]).output().context("running gh (is the GitHub CLI installed and on PATH?)")?;
    if !output.status.success() {
        bail!("gh pr view {url}: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    parse_gh_json(url, &serde_json::from_slice(&output.stdout)?)
}

pub fn parse_gh_json(url: &str, json: &Value) -> Result<PrStatus> {
    let number = json.get("number").and_then(Value::as_u64).context("gh output has no PR number")?;
    let state = match (json.get("state").and_then(Value::as_str), json.get("isDraft").and_then(Value::as_bool)) {
        (Some("MERGED"), _) => PrState::Merged,
        (Some("CLOSED"), _) => PrState::Closed,
        (_, Some(true)) => PrState::Draft,
        _ => PrState::Open,
    };
    let checks = json.get("statusCheckRollup").and_then(Value::as_array).map(|items| rollup(items)).unwrap_or(Checks::None);
    Ok(PrStatus { url: url.to_string(), number, state, checks })
}

/// Combines check runs and commit statuses: any failure wins, then anything still running.
fn rollup(items: &[Value]) -> Checks {
    let field = |item: &Value, key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut result = Checks::None;
    for item in items {
        let (status, conclusion, state) = (field(item, "status"), field(item, "conclusion"), field(item, "state"));
        let check = match (status.as_str(), conclusion.as_str(), state.as_str()) {
            (_, "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE", _) | (_, _, "FAILURE" | "ERROR") => Checks::Failing,
            ("QUEUED" | "IN_PROGRESS" | "WAITING" | "PENDING" | "REQUESTED", _, _) | (_, _, "PENDING" | "EXPECTED") => Checks::Pending,
            _ => Checks::Passing,
        };
        result = match (result, check) {
            (Checks::Failing, _) | (_, Checks::Failing) => Checks::Failing,
            (Checks::Pending, _) | (_, Checks::Pending) => Checks::Pending,
            _ => Checks::Passing,
        };
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Pending,
    Failing,
    Done,
}

/// A card badge such as "2 PRs · 1 failing".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSummary {
    pub text: String,
    pub tone: Tone,
}

pub fn summarize(statuses: &[&PrStatus]) -> Option<PrSummary> {
    if statuses.is_empty() {
        return None;
    }
    let count = |pred: &dyn Fn(&PrStatus) -> bool| statuses.iter().filter(|s| pred(s)).count();
    let failing = count(&|s| s.is_open() && s.checks == Checks::Failing);
    let running = count(&|s| s.is_open() && s.checks == Checks::Pending);
    let open = count(&|s| s.is_open());
    let merged = count(&|s| s.state == PrState::Merged);
    let (detail, tone) = match () {
        _ if failing > 0 => (format!("{failing} failing"), Tone::Failing),
        _ if running > 0 => (format!("{running} running"), Tone::Pending),
        _ if open > 0 => (format!("{open} open"), Tone::Neutral),
        _ if merged == statuses.len() => ("merged".to_string(), Tone::Done),
        _ => ("closed".to_string(), Tone::Done),
    };
    let noun = if statuses.len() == 1 { "PR" } else { "PRs" };
    Some(PrSummary { text: format!("{} {noun} · {detail}", statuses.len()), tone })
}

const OPEN_TTL: u64 = 60;
const FINISHED_TTL: u64 = 600;
const ERROR_TTL: u64 = 300;

struct Entry {
    fetched_at: u64,
    result: Result<PrStatus, String>,
}

/// Fetched PR statuses, refreshed on a schedule that depends on whether the PR can still change.
#[derive(Default)]
pub struct PrCache {
    entries: HashMap<String, Entry>,
    in_flight: HashSet<String>,
}

impl PrCache {
    /// The URLs that are due for a (re)fetch and not already being fetched; marks them in flight.
    pub fn claim_stale(&mut self, urls: impl IntoIterator<Item = String>, now: u64) -> Vec<String> {
        let mut claimed = Vec::new();
        for url in urls {
            if self.in_flight.contains(&url) || self.is_fresh(&url, now) {
                continue;
            }
            self.in_flight.insert(url.clone());
            claimed.push(url);
        }
        claimed
    }

    pub fn finish(&mut self, url: String, result: Result<PrStatus, String>, now: u64) {
        self.in_flight.remove(&url);
        self.entries.insert(url, Entry { fetched_at: now, result });
    }

    pub fn status(&self, url: &str) -> Option<&PrStatus> {
        self.entries.get(url)?.result.as_ref().ok()
    }

    pub fn error(&self, url: &str) -> Option<&str> {
        self.entries.get(url)?.result.as_ref().err().map(String::as_str)
    }

    pub fn statuses<'a>(&'a self, urls: &[String]) -> Vec<&'a PrStatus> {
        urls.iter().filter_map(|url| self.status(url)).collect()
    }

    fn is_fresh(&self, url: &str, now: u64) -> bool {
        let Some(entry) = self.entries.get(url) else { return false };
        let ttl = match &entry.result {
            Ok(status) if status.is_open() => OPEN_TTL,
            Ok(_) => FINISHED_TTL,
            Err(_) => ERROR_TTL,
        };
        now.saturating_sub(entry.fetched_at) < ttl
    }
}

#[cfg(windows)]
fn gh() -> Command {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("gh");
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn gh() -> Command {
    Command::new("gh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::TicketStore;
    use serde_json::json;

    fn pr(number: u64, state: PrState, checks: Checks) -> PrStatus {
        PrStatus { url: format!("https://github.com/o/r/pull/{number}"), number, state, checks }
    }

    #[test]
    fn normalizes_pull_request_urls() {
        assert_eq!(normalize_pr_url("https://github.com/o/r/pull/12/files?x=1").as_deref(), Some("https://github.com/o/r/pull/12"));
        assert_eq!(normalize_pr_url("https://ghe.corp/o/r/pull/3#top").as_deref(), Some("https://ghe.corp/o/r/pull/3"));
        assert_eq!(normalize_pr_url("https://github.com/o/r/issues/12"), None);
        assert_eq!(normalize_pr_url("https://github.com/o/r/pull/abc"), None);
    }

    #[test]
    fn collects_pr_links_from_every_artifact() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let config = BoardConfig::parse("[artifacts.branch]\ntype = \"text\"\n[artifacts.notes]\ntype = \"file\"\n[[columns]]\nid = \"dev\"\n").unwrap();
        TicketStore::new(home.clone()).create(Some("T-1"), "t", "", "dev").unwrap();
        let store = ArtifactStore::new(&home, &config, "T-1");
        store.write("branch", "C:\\a | feat/x | https://github.com/o/a/pull/1\nC:\\b | main | no PR").unwrap();
        store.write("notes", "See (https://github.com/o/b/pull/2/files) and https://github.com/o/a/pull/1. Docs: https://x.io").unwrap();
        assert_eq!(pr_urls(&home, &config, "T-1"), ["https://github.com/o/a/pull/1", "https://github.com/o/b/pull/2"]);
    }

    #[test]
    fn parses_gh_output_and_rolls_up_checks() {
        let parse = |json: Value| parse_gh_json("u", &json).unwrap();
        let run = |status: &str, conclusion: &str| json!({"__typename": "CheckRun", "status": status, "conclusion": conclusion});
        let merged = parse(json!({"number": 7, "state": "MERGED", "isDraft": false, "statusCheckRollup": []}));
        assert_eq!((merged.state, merged.checks), (PrState::Merged, Checks::None));
        let draft = parse(json!({"number": 8, "state": "OPEN", "isDraft": true, "statusCheckRollup": [run("COMPLETED", "SUCCESS"), run("IN_PROGRESS", "")]}));
        assert_eq!((draft.state, draft.checks), (PrState::Draft, Checks::Pending));
        let failing = parse(
            json!({"number": 9, "state": "OPEN", "isDraft": false, "statusCheckRollup": [run("IN_PROGRESS", ""), {"__typename": "StatusContext", "state": "FAILURE"}]}),
        );
        assert_eq!(failing.checks, Checks::Failing);
        let passing = parse(json!({"number": 10, "state": "OPEN", "statusCheckRollup": [run("COMPLETED", "SKIPPED"), {"state": "SUCCESS"}]}));
        assert_eq!((passing.checks, passing.label().as_str()), (Checks::Passing, "open · checks passing"));
        assert!(parse_gh_json("u", &json!({"state": "OPEN"})).is_err());
    }

    #[test]
    fn summarizes_for_the_card_badge() {
        let (failing, running, open) = (pr(1, PrState::Open, Checks::Failing), pr(2, PrState::Draft, Checks::Pending), pr(3, PrState::Open, Checks::Passing));
        let (merged, closed) = (pr(4, PrState::Merged, Checks::Failing), pr(5, PrState::Closed, Checks::None));
        let text = |list: &[&PrStatus]| summarize(list).map(|s| (s.text, s.tone));
        assert_eq!(text(&[&failing, &running]), Some(("2 PRs · 1 failing".into(), Tone::Failing)));
        assert_eq!(text(&[&running, &open]), Some(("2 PRs · 1 running".into(), Tone::Pending)));
        assert_eq!(text(&[&open, &merged]), Some(("2 PRs · 1 open".into(), Tone::Neutral)));
        assert_eq!(text(&[&merged]), Some(("1 PR · merged".into(), Tone::Done)));
        assert_eq!(text(&[&merged, &closed]), Some(("2 PRs · closed".into(), Tone::Done)));
        assert_eq!(text(&[]), None);
    }

    #[test]
    fn cache_refetches_by_state_and_skips_in_flight() {
        let mut cache = PrCache::default();
        let urls = || vec!["a".to_string(), "b".to_string()];
        assert_eq!(cache.claim_stale(urls(), 1000), ["a", "b"]);
        assert!(cache.claim_stale(urls(), 1000).is_empty());
        cache.finish("a".into(), Ok(PrStatus { url: "a".into(), ..pr(1, PrState::Open, Checks::None) }), 1000);
        cache.finish("b".into(), Ok(PrStatus { url: "b".into(), ..pr(2, PrState::Merged, Checks::None) }), 1000);
        assert!(cache.claim_stale(urls(), 1030).is_empty());
        assert_eq!(cache.claim_stale(urls(), 1061), ["a"]);
        cache.finish("a".into(), Err("gh missing".into()), 1061);
        assert_eq!(cache.error("a"), Some("gh missing"));
        assert_eq!(cache.statuses(&urls()).len(), 1);
        assert_eq!(cache.claim_stale(urls(), 1700), ["a", "b"]);
    }
}
