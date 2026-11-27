//! Pull request status for links found in artifacts. Fetching is provider-specific (see `PrProvider`); the model, card summary and cache are shared.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use anyhow::Result;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::artifact_view::{Segment, segments};
use crate::artifacts::ArtifactStore;
use crate::config::BoardConfig;
use crate::paths::LanesHome;

mod github;
mod none;

pub use github::GitHubCli;
pub use none::NoPullRequests;

/// Where pull request status comes from.
pub trait PrProvider: Send + Sync {
    /// The canonical form of one of this provider's pull request links, or None for any other URL (which then shows no status).
    fn normalize(&self, url: &str) -> Option<String>;
    /// A compact name for a pull request link, e.g. "app#12".
    fn short_name(&self, url: &str) -> Option<String>;
    /// Fetches the current status of a normalized pull request link; called off the UI thread.
    fn fetch(&self, url: &str) -> Result<PrStatus>;
}

/// The `pr_provider` setting: which service pull request links are checked against.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PrProviderKind {
    /// GitHub (and GitHub Enterprise) through the GitHub CLI, `gh`.
    #[default]
    Github,
    /// Don't check pull requests: links stay plain links.
    None,
}

pub fn provider(kind: PrProviderKind) -> Arc<dyn PrProvider> {
    match kind {
        PrProviderKind::Github => Arc::new(GitHubCli),
        PrProviderKind::None => Arc::new(NoPullRequests),
    }
}

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

/// GitHub's review decision, which accounts for required reviewers and branch protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Review {
    None,
    Required,
    Approved,
    ChangesRequested,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrStatus {
    pub url: String,
    pub number: u64,
    pub state: PrState,
    pub checks: Checks,
    pub review: Review,
    /// Review threads not yet resolved; None when they couldn't be fetched or the PR is finished.
    pub unresolved: Option<u32>,
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

    /// e.g. "approved", "changes requested"; only for open PRs.
    pub fn review_label(&self) -> Option<&'static str> {
        match (self.is_open(), self.review) {
            (false, _) | (_, Review::None) => None,
            (true, Review::Required) => Some("review needed"),
            (true, Review::Approved) => Some("approved"),
            (true, Review::ChangesRequested) => Some("changes requested"),
        }
    }

    /// e.g. "2 unresolved comments"; only when an open PR has some.
    pub fn unresolved_label(&self) -> Option<String> {
        let count = self.unresolved.filter(|n| *n > 0 && self.is_open())?;
        Some(format!("{count} unresolved comment{}", if count == 1 { "" } else { "s" }))
    }

    /// Everything known about the PR in one line, e.g. "open · checks passing · approved · 1 unresolved comment".
    pub fn full_label(&self) -> String {
        let parts = [Some(self.label()), self.review_label().map(str::to_string), self.unresolved_label()];
        parts.into_iter().flatten().collect::<Vec<_>>().join(" · ")
    }
}

/// Every pull request the provider recognises in the ticket's artifacts, in order of first appearance.
pub fn pr_urls(home: &LanesHome, config: &BoardConfig, ticket_id: &str, provider: &dyn PrProvider) -> Vec<String> {
    let store = ArtifactStore::new(home, config, ticket_id);
    let mut seen = BTreeSet::new();
    config
        .artifacts
        .keys()
        .filter_map(|name| store.read(name).ok().flatten())
        .flat_map(|content| content.lines().flat_map(segments).collect::<Vec<_>>())
        .filter_map(|segment| match segment {
            Segment::Url(url) => provider.normalize(&url),
            _ => None,
        })
        .filter(|url| seen.insert(url.clone()))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Pending,
    Failing,
    Done,
}

/// A card badge such as "2 PRs · 1 failing · changes requested · 3 unresolved".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrSummary {
    /// Separate chips, so a narrow card can wrap them: the count and state, then review, then unresolved comments.
    pub parts: Vec<String>,
    pub tone: Tone,
}

impl PrSummary {
    pub fn text(&self) -> String {
        self.parts.join(" · ")
    }
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
    let review = review_summary(statuses);
    let unresolved: u32 = statuses.iter().filter(|s| s.is_open()).filter_map(|s| s.unresolved).sum();
    let mut parts = vec![format!("{} {noun} · {detail}", statuses.len())];
    parts.extend(review.map(str::to_string));
    if unresolved > 0 {
        parts.push(format!("{unresolved} unresolved"));
    }
    let tone = match (tone, review, unresolved) {
        (Tone::Failing, _, _) | (_, Some("changes requested"), _) => Tone::Failing,
        (Tone::Pending, _, _) | (_, _, 1..) => Tone::Pending,
        (tone, _, _) => tone,
    };
    Some(PrSummary { parts, tone })
}

/// The review state worth a card's room: changes requested on any open PR, or every open PR approved.
fn review_summary(statuses: &[&PrStatus]) -> Option<&'static str> {
    let open: Vec<_> = statuses.iter().filter(|s| s.is_open()).collect();
    match () {
        _ if open.iter().any(|s| s.review == Review::ChangesRequested) => Some("changes requested"),
        _ if !open.is_empty() && open.iter().all(|s| s.review == Review::Approved) => Some("approved"),
        _ => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::TicketStore;

    fn pr(number: u64, state: PrState, checks: Checks) -> PrStatus {
        PrStatus { url: format!("https://github.com/o/r/pull/{number}"), number, state, checks, review: Review::None, unresolved: None }
    }

    #[test]
    fn collects_pr_links_from_every_artifact_unless_the_provider_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let config = BoardConfig::parse("[artifacts.branch]\ntype = \"text\"\n[artifacts.notes]\ntype = \"file\"\n[[columns]]\nid = \"dev\"\n").unwrap();
        TicketStore::new(home.clone()).create(Some("T-1"), "t", "", "dev").unwrap();
        let store = ArtifactStore::new(&home, &config, "T-1");
        store.write("branch", "C:\\a | feat/x | https://github.com/o/a/pull/1\nC:\\b | main | no PR").unwrap();
        store.write("notes", "See (https://github.com/o/b/pull/2/files) and https://github.com/o/a/pull/1. Docs: https://x.io").unwrap();
        let github = provider(PrProviderKind::Github);
        assert_eq!(pr_urls(&home, &config, "T-1", github.as_ref()), ["https://github.com/o/a/pull/1", "https://github.com/o/b/pull/2"]);
        assert!(pr_urls(&home, &config, "T-1", provider(PrProviderKind::None).as_ref()).is_empty());
    }

    #[test]
    fn review_and_comment_labels_only_apply_to_open_prs() {
        let open = PrStatus { review: Review::ChangesRequested, unresolved: Some(1), ..pr(1, PrState::Open, Checks::Passing) };
        assert_eq!((open.review_label(), open.unresolved_label().as_deref()), (Some("changes requested"), Some("1 unresolved comment")));
        assert_eq!(open.full_label(), "open · checks passing · changes requested · 1 unresolved comment");
        let merged = PrStatus { state: PrState::Merged, ..open.clone() };
        assert_eq!((merged.review_label(), merged.unresolved_label()), (None, None));
        let clean = PrStatus { review: Review::Approved, unresolved: Some(0), ..open };
        assert_eq!((clean.review_label(), clean.unresolved_label()), (Some("approved"), None));
    }

    #[test]
    fn summarizes_for_the_card_badge() {
        let (failing, running, open) = (pr(1, PrState::Open, Checks::Failing), pr(2, PrState::Draft, Checks::Pending), pr(3, PrState::Open, Checks::Passing));
        let (merged, closed) = (pr(4, PrState::Merged, Checks::Failing), pr(5, PrState::Closed, Checks::None));
        let text = |list: &[&PrStatus]| summarize(list).map(|s| (s.text(), s.tone));
        assert_eq!(text(&[&failing, &running]), Some(("2 PRs · 1 failing".into(), Tone::Failing)));
        assert_eq!(text(&[&running, &open]), Some(("2 PRs · 1 running".into(), Tone::Pending)));
        assert_eq!(text(&[&open, &merged]), Some(("2 PRs · 1 open".into(), Tone::Neutral)));
        assert_eq!(text(&[&merged]), Some(("1 PR · merged".into(), Tone::Done)));
        assert_eq!(text(&[&merged, &closed]), Some(("2 PRs · closed".into(), Tone::Done)));
        assert_eq!(text(&[]), None);
        assert_eq!(summarize(&[&failing, &running]).unwrap().parts, ["2 PRs · 1 failing"]);

        let reviewed = |review, unresolved| PrStatus { review, unresolved, ..pr(6, PrState::Open, Checks::Passing) };
        let (changes, approved, commented) = (reviewed(Review::ChangesRequested, Some(2)), reviewed(Review::Approved, Some(0)), reviewed(Review::Approved, Some(3)));
        assert_eq!(text(&[&changes]), Some(("1 PR · 1 open · changes requested · 2 unresolved".into(), Tone::Failing)));
        assert_eq!(summarize(&[&changes]).unwrap().parts, ["1 PR · 1 open", "changes requested", "2 unresolved"]);
        assert_eq!(text(&[&approved]), Some(("1 PR · 1 open · approved".into(), Tone::Neutral)));
        assert_eq!(text(&[&commented, &merged]), Some(("2 PRs · 1 open · approved · 3 unresolved".into(), Tone::Pending)));
        assert_eq!(text(&[&approved, &open]), Some(("2 PRs · 2 open".into(), Tone::Neutral)));
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
