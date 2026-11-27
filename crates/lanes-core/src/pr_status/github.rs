//! GitHub and GitHub Enterprise pull requests, through the GitHub CLI (`gh`).

use std::process::Command;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use super::{Checks, PrProvider, PrState, PrStatus, Review};

pub struct GitHubCli;

impl PrProvider for GitHubCli {
    fn normalize(&self, url: &str) -> Option<String> {
        normalize_pr_url(url)
    }

    fn short_name(&self, url: &str) -> Option<String> {
        short_name(url)
    }

    fn fetch(&self, url: &str) -> Result<PrStatus> {
        fetch(url)
    }
}

/// The canonical `https://<host>/<owner>/<repo>/pull/<n>` form of a pull request URL, or None if it isn't one.
fn normalize_pr_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://")?;
    let parts: Vec<&str> = rest.split(['/', '?', '#']).collect();
    let [host, owner, repo, "pull", number, ..] = parts.as_slice() else { return None };
    let valid = !host.is_empty() && !owner.is_empty() && !repo.is_empty() && !number.is_empty() && number.chars().all(|c| c.is_ascii_digit());
    valid.then(|| format!("https://{host}/{owner}/{repo}/pull/{number}"))
}

/// A compact name for a pull request link, e.g. "app#12".
fn short_name(url: &str) -> Option<String> {
    let normalized = normalize_pr_url(url)?;
    let (_, _, repo, number) = pr_parts(&normalized)?;
    Some(format!("{repo}#{number}"))
}

/// Asks the GitHub CLI for the pull request's state, checks and review, plus its unresolved threads while it's open.
fn fetch(url: &str) -> Result<PrStatus> {
    let mut status = parse_gh_json(url, &run_gh(&["pr", "view", url, "--json", "number,state,isDraft,statusCheckRollup,reviewDecision"])?)?;
    if status.is_open() {
        status.unresolved = fetch_unresolved(url).ok();
    }
    Ok(status)
}

const THREADS_QUERY: &str =
    "query($owner:String!,$name:String!,$number:Int!){repository(owner:$owner,name:$name){pullRequest(number:$number){reviewThreads(first:100){nodes{isResolved}}}}}";

/// Unresolved review threads come only from the GraphQL API; `gh pr view` doesn't expose them.
fn fetch_unresolved(url: &str) -> Result<u32> {
    let (host, owner, repo, number) = pr_parts(url).context("not a pull request URL")?;
    let query = format!("query={THREADS_QUERY}");
    let (owner, repo, number) = (format!("owner={owner}"), format!("name={repo}"), format!("number={number}"));
    let json = run_gh(&["api", "graphql", "--hostname", host, "-f", &query, "-F", &owner, "-F", &repo, "-F", &number])?;
    parse_unresolved(&json)
}

fn run_gh(args: &[&str]) -> Result<Value> {
    let output = gh().args(args).output().context("running gh (is the GitHub CLI installed and on PATH?)")?;
    if !output.status.success() {
        bail!("gh {}: {}", args.first().copied().unwrap_or_default(), String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn pr_parts(url: &str) -> Option<(&str, &str, &str, &str)> {
    let mut parts = url.strip_prefix("https://")?.split('/');
    let (host, owner, repo, _, number) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
    Some((host, owner, repo, number))
}

pub fn parse_unresolved(json: &Value) -> Result<u32> {
    let threads = json.pointer("/data/repository/pullRequest/reviewThreads/nodes").and_then(Value::as_array).context("gh output has no review threads")?;
    Ok(threads.iter().filter(|t| t.get("isResolved").and_then(Value::as_bool) == Some(false)).count() as u32)
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
    let review = match json.get("reviewDecision").and_then(Value::as_str) {
        Some("APPROVED") => Review::Approved,
        Some("CHANGES_REQUESTED") => Review::ChangesRequested,
        Some("REVIEW_REQUIRED") => Review::Required,
        _ => Review::None,
    };
    Ok(PrStatus { url: url.to_string(), number, state, checks, review, unresolved: None })
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

fn gh() -> Command {
    crate::platform::current().quiet_command("gh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalizes_pull_request_urls() {
        assert_eq!(normalize_pr_url("https://github.com/o/r/pull/12/files?x=1").as_deref(), Some("https://github.com/o/r/pull/12"));
        assert_eq!(normalize_pr_url("https://ghe.corp/o/r/pull/3#top").as_deref(), Some("https://ghe.corp/o/r/pull/3"));
        assert_eq!(normalize_pr_url("https://github.com/o/r/issues/12"), None);
        assert_eq!(normalize_pr_url("https://github.com/o/r/pull/abc"), None);
        assert_eq!(short_name("https://github.com/o/app/pull/12/files").as_deref(), Some("app#12"));
        assert_eq!(short_name("https://github.com/o/app/issues/12"), None);
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
    fn parses_review_decision_and_unresolved_threads() {
        let review = |decision: Value| parse_gh_json("u", &json!({"number": 1, "state": "OPEN", "reviewDecision": decision})).unwrap().review;
        assert_eq!(review(json!("APPROVED")), Review::Approved);
        assert_eq!(review(json!("CHANGES_REQUESTED")), Review::ChangesRequested);
        assert_eq!(review(json!("REVIEW_REQUIRED")), Review::Required);
        assert_eq!(review(json!("")), Review::None);
        let threads =
            json!({"data": {"repository": {"pullRequest": {"reviewThreads": {"nodes": [{"isResolved": true}, {"isResolved": false}, {"isResolved": false}]}}}}});
        assert_eq!(parse_unresolved(&threads).unwrap(), 2);
        assert!(parse_unresolved(&json!({"errors": []})).is_err());
        assert_eq!(pr_parts("https://ghe.corp/o/r/pull/3"), Some(("ghe.corp", "o", "r", "3")));
    }
}
