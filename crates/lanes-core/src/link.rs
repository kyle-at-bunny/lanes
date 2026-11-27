//! A ticket's link to its work item in whatever tracker the team uses (Jira, Azure DevOps, GitHub, Linear, ...).

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct TicketLink {
    /// Short name shown on the card, e.g. "PROJ-12", "#4521", "app#12".
    pub key: String,
    pub url: String,
}

/// Builds a link from a work item URL; `key` overrides the short name worked out from the URL.
pub fn link_from(url: &str, key: Option<&str>) -> Result<TicketLink> {
    let url = url.trim();
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("'{url}' is not a link: pass the work item's full URL");
    }
    let key = match key.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => key.to_string(),
        None => key_from_url(url).ok_or_else(|| anyhow::anyhow!("can't tell the work item's key from '{url}': pass one with --link-key"))?,
    };
    Ok(TicketLink { key, url: url.to_string() })
}

/// The work item's short name from common tracker URL shapes.
pub fn key_from_url(url: &str) -> Option<String> {
    let (path, query) = url.split_once('?').map_or((url, ""), |(path, query)| (path, query));
    let path = path.split('#').next().unwrap_or(path);
    let segments: Vec<&str> = path.split('/').skip(3).filter(|s| !s.is_empty()).collect();
    let after = |marker: &str| segments.iter().position(|s| *s == marker).and_then(|i| segments.get(i + 1)).copied();
    let query_item = query.split('&').find_map(|pair| pair.strip_prefix("workitem=")).filter(|n| is_number(n));

    if let Some(key) = after("browse").filter(|k| is_issue_key(k)) {
        return Some(key.to_ascii_uppercase());
    }
    if let Some(number) = after("edit").filter(|n| segments.contains(&"_workitems") && is_number(n)).or(query_item) {
        return Some(format!("#{number}"));
    }
    if let Some(i) = segments.iter().position(|s| *s == "issues" || *s == "pull") {
        let number = segments.get(i + 1).filter(|n| is_number(n))?;
        let repo = segments[..i].iter().rev().find(|s| **s != "-")?;
        return Some(format!("{repo}#{number}"));
    }
    if let Some(key) = after("issue").filter(|k| is_issue_key(k)) {
        return Some(key.to_ascii_uppercase());
    }
    segments.last().filter(|s| is_issue_key(s)).map(|s| s.to_ascii_uppercase())
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_digit())
}

fn is_issue_key(candidate: &str) -> bool {
    let Some((project, number)) = candidate.split_once('-') else { return false };
    project.starts_with(|c: char| c.is_ascii_alphabetic()) && project.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && is_number(number)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn works_out_keys_from_common_trackers() {
        let key = |url: &str| key_from_url(url);
        assert_eq!(key("https://acme.atlassian.net/browse/PROJ-12?focusedCommentId=3").as_deref(), Some("PROJ-12"));
        assert_eq!(key("https://dev.azure.com/org/project/_workitems/edit/4521").as_deref(), Some("#4521"));
        assert_eq!(key("https://dev.azure.com/org/project/_boards/board/t/Team/Stories?workitem=88").as_deref(), Some("#88"));
        assert_eq!(key("https://github.com/org/app/issues/12").as_deref(), Some("app#12"));
        assert_eq!(key("https://gitlab.com/group/app/-/issues/7").as_deref(), Some("app#7"));
        assert_eq!(key("https://linear.app/team/issue/ENG-42/fix-login").as_deref(), Some("ENG-42"));
        assert_eq!(key("https://tracker.example/items/OPS-9").as_deref(), Some("OPS-9"));
        assert_eq!(key("https://example.com/some/page"), None);
    }

    #[test]
    fn needs_a_url_and_a_key_from_somewhere() {
        assert_eq!(link_from(" https://x/browse/A-1 ", None).unwrap(), TicketLink { key: "A-1".into(), url: "https://x/browse/A-1".into() });
        assert_eq!(link_from("https://example.com/page", Some("SUP-3")).unwrap().key, "SUP-3");
        assert!(link_from("https://example.com/page", None).is_err());
        assert!(link_from("PROJ-1", None).is_err());
    }
}
