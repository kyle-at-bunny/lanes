use anyhow::{Context, Result, bail};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::JiraConfig;
use crate::ticket::TicketStore;

const PICKER_LIMIT: u32 = 50;

/// The Jira issue a ticket was created from.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct JiraLink {
    pub key: String,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JiraIssue {
    pub key: String,
    pub summary: String,
    pub description: String,
    pub status: Option<String>,
    pub url: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub created: Vec<String>,
    pub skipped: Vec<String>,
}

pub struct JiraClient {
    site: String,
    authorization: String,
}

impl JiraClient {
    pub fn from_config(config: &JiraConfig) -> Result<Self> {
        if config.email.trim().is_empty() {
            bail!("set email in the [jira] section of board.toml to use the Jira API");
        }
        let token = std::env::var(&config.token_env)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .with_context(|| format!("set the {} environment variable to a Jira API token, then restart Lanes", config.token_env))?;
        let credentials = base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", config.email, token.trim()));
        Ok(Self { site: config.site.trim_end_matches('/').to_string(), authorization: format!("Basic {credentials}") })
    }

    pub fn issue(&self, key: &str) -> Result<JiraIssue> {
        let url = format!("{}/rest/api/3/issue/{key}?fields=summary,description,status", self.site);
        let body: Value = ureq::get(&url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .call()
            .map_err(|err| describe_error(err, key))?
            .body_mut()
            .read_json()?;
        self.parse_issue(&body)
    }

    pub fn my_open_issues(&self, jql: &str) -> Result<Vec<JiraIssue>> {
        let url = format!("{}/rest/api/3/search/jql", self.site);
        let request = json!({ "jql": jql, "fields": ["summary", "description", "status"], "maxResults": PICKER_LIMIT });
        let body: Value = ureq::post(&url)
            .header("Authorization", &self.authorization)
            .header("Accept", "application/json")
            .send_json(request)
            .map_err(|err| describe_error(err, "search"))?
            .body_mut()
            .read_json()?;
        let issues = body.get("issues").and_then(Value::as_array).context("unexpected Jira search response")?;
        issues.iter().map(|issue| self.parse_issue(issue)).collect()
    }

    fn parse_issue(&self, issue: &Value) -> Result<JiraIssue> {
        let key = issue.get("key").and_then(Value::as_str).context("Jira issue without a key")?.to_string();
        let fields = issue.get("fields").cloned().unwrap_or_default();
        Ok(JiraIssue {
            url: format!("{}/browse/{key}", self.site),
            summary: fields.get("summary").and_then(Value::as_str).unwrap_or_default().to_string(),
            description: fields.get("description").map(adf_to_text).unwrap_or_default(),
            status: fields.pointer("/status/name").and_then(Value::as_str).map(str::to_string),
            key,
        })
    }
}

fn describe_error(err: ureq::Error, what: &str) -> anyhow::Error {
    match err {
        ureq::Error::StatusCode(401) | ureq::Error::StatusCode(403) => anyhow::anyhow!("Jira rejected the credentials (check email and API token)"),
        ureq::Error::StatusCode(404) => anyhow::anyhow!("Jira has no issue {what} (or you can't see it)"),
        ureq::Error::StatusCode(code) => anyhow::anyhow!("Jira returned HTTP {code} for {what}"),
        other => anyhow::Error::new(other).context("could not reach Jira"),
    }
}

/// Accepts keys or browse URLs separated by commas or whitespace, e.g. "PROJ-1, https://x.atlassian.net/browse/PROJ-2".
pub fn parse_keys(input: &str) -> Result<Vec<String>> {
    let mut keys = Vec::new();
    for token in input.split(|c: char| c == ',' || c.is_whitespace()).filter(|t| !t.is_empty()) {
        let candidate = token.rsplit('/').next().unwrap_or(token).split('?').next().unwrap_or_default().to_ascii_uppercase();
        if !is_issue_key(&candidate) {
            bail!("'{token}' is not a Jira issue key like PROJ-123");
        }
        if !keys.contains(&candidate) {
            keys.push(candidate);
        }
    }
    Ok(keys)
}

/// Builds a ticket's Jira link from a browse URL, or from a bare key plus the board's Jira site.
pub fn link_from(input: &str, site: Option<&str>) -> Result<JiraLink> {
    let input = input.trim();
    let key = match parse_keys(input)?.as_slice() {
        [key] => key.clone(),
        [] => bail!("no Jira issue given"),
        _ => bail!("give one Jira issue, not several"),
    };
    let url = if input.starts_with("http://") || input.starts_with("https://") {
        input.split(['?', '#']).next().unwrap_or(input).trim_end_matches('/').to_string()
    } else {
        let site = site
            .filter(|s| !s.trim().is_empty())
            .with_context(|| format!("'{key}' is a bare key: pass the full browse URL, or set site in the [jira] section of board.toml"))?;
        format!("{}/browse/{key}", site.trim_end_matches('/'))
    };
    Ok(JiraLink { key, url })
}

fn is_issue_key(candidate: &str) -> bool {
    let Some((project, number)) = candidate.split_once('-') else { return false };
    project.starts_with(|c: char| c.is_ascii_alphabetic())
        && project.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !number.is_empty()
        && number.chars().all(|c| c.is_ascii_digit())
}

/// Creates a ticket per issue in `column`; issues that already exist as tickets (on the board or archived) are skipped.
pub fn import(store: &TicketStore, column: &str, issues: &[JiraIssue]) -> Result<ImportReport> {
    let mut report = ImportReport::default();
    for issue in issues {
        if store.exists(&issue.key) {
            report.skipped.push(issue.key.clone());
            continue;
        }
        let link = JiraLink { key: issue.key.clone(), url: issue.url.clone() };
        let title = if issue.summary.is_empty() { issue.key.clone() } else { issue.summary.clone() };
        store.create_linked(&issue.key, &title, &issue.description, column, link)?;
        report.created.push(issue.key.clone());
    }
    Ok(report)
}

/// Flattens Atlassian Document Format into readable markdown-ish text.
pub fn adf_to_text(node: &Value) -> String {
    match node {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        _ => block(node).trim().to_string(),
    }
}

fn block(node: &Value) -> String {
    let kind = node.get("type").and_then(Value::as_str).unwrap_or_default();
    let children = || node.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
    match kind {
        "doc" => children().iter().map(block).collect::<Vec<_>>().join(""),
        "paragraph" => format!("{}\n\n", inline_all(&children())),
        "heading" => {
            let level = node.pointer("/attrs/level").and_then(Value::as_u64).unwrap_or(2) as usize;
            format!("{} {}\n\n", "#".repeat(level), inline_all(&children()))
        }
        "bulletList" => list(&children(), |_| "- ".to_string()),
        "orderedList" => list(&children(), |i| format!("{}. ", i + 1)),
        "codeBlock" => format!("```\n{}\n```\n\n", inline_all(&children())),
        "blockquote" => children().iter().map(block).collect::<String>().lines().map(|l| format!("> {l}\n")).collect::<String>() + "\n",
        "rule" => "---\n\n".into(),
        "table" => children().iter().map(table_row).collect::<String>() + "\n",
        "panel" | "listItem" | "expand" | "nestedExpand" => children().iter().map(block).collect(),
        _ => format!("{}\n\n", inline(node)),
    }
}

fn list(items: &[Value], marker: impl Fn(usize) -> String) -> String {
    let rendered: String = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let body = block(item).trim().replace('\n', "\n  ");
            format!("{}{body}\n", marker(i))
        })
        .collect();
    rendered + "\n"
}

fn table_row(row: &Value) -> String {
    let cells = row.get("content").and_then(Value::as_array).cloned().unwrap_or_default();
    let texts: Vec<String> = cells.iter().map(|cell| block(cell).trim().replace('\n', " ")).collect();
    format!("| {} |\n", texts.join(" | "))
}

fn inline_all(nodes: &[Value]) -> String {
    nodes.iter().map(inline).collect()
}

fn inline(node: &Value) -> String {
    match node.get("type").and_then(Value::as_str).unwrap_or_default() {
        "text" => {
            let text = node.get("text").and_then(Value::as_str).unwrap_or_default();
            let href = node.get("marks").and_then(Value::as_array).and_then(|marks| {
                marks.iter().find(|m| m.get("type").and_then(Value::as_str) == Some("link")).and_then(|m| m.pointer("/attrs/href")).and_then(Value::as_str)
            });
            match href {
                Some(href) if href != text => format!("[{text}]({href})"),
                _ => text.to_string(),
            }
        }
        "hardBreak" => "\n".into(),
        "mention" | "emoji" | "status" | "date" => node.pointer("/attrs/text").and_then(Value::as_str).unwrap_or_default().to_string(),
        "inlineCard" | "blockCard" => node.pointer("/attrs/url").and_then(Value::as_str).unwrap_or_default().to_string(),
        _ => node.get("content").and_then(Value::as_array).map(|c| inline_all(c)).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LanesHome;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    #[test]
    fn parses_keys_and_browse_urls() {
        assert_eq!(parse_keys("proj-1, PROJ-2  https://x.atlassian.net/browse/OPS-33?focused=1 PROJ-1").unwrap(), ["PROJ-1", "PROJ-2", "OPS-33"]);
        assert!(parse_keys("not a key").is_err());
        assert!(parse_keys("").unwrap().is_empty());
    }

    #[test]
    fn builds_links_from_urls_and_bare_keys() {
        let from_url = link_from("https://acme.atlassian.net/browse/PROJ-12?focusedCommentId=3", None).unwrap();
        assert_eq!(from_url, JiraLink { key: "PROJ-12".into(), url: "https://acme.atlassian.net/browse/PROJ-12".into() });
        let from_key = link_from("proj-7", Some("https://acme.atlassian.net/")).unwrap();
        assert_eq!(from_key.url, "https://acme.atlassian.net/browse/PROJ-7");
        assert!(link_from("PROJ-7", None).unwrap_err().to_string().contains("full browse URL"));
        assert!(link_from("PROJ-1 PROJ-2", Some("https://x")).is_err());
        assert!(link_from("nonsense", Some("https://x")).is_err());
    }

    #[test]
    fn converts_adf_to_text() {
        let adf = json!({"type":"doc","content":[
            {"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Goal"}]},
            {"type":"paragraph","content":[{"type":"text","text":"Fix "},{"type":"text","text":"docs","marks":[{"type":"link","attrs":{"href":"https://d"}}]},{"type":"hardBreak"},{"type":"mention","attrs":{"text":"@kim"}}]},
            {"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"one"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"two"}]}]}]},
            {"type":"codeBlock","content":[{"type":"text","text":"cargo test"}]}
        ]});
        assert_eq!(adf_to_text(&adf), "## Goal\n\nFix [docs](https://d)\n@kim\n\n- one\n- two\n\n```\ncargo test\n```");
        assert_eq!(adf_to_text(&Value::Null), "");
    }

    /// Serves canned responses on a local port and records each request line plus its Authorization header.
    fn mock_jira(responses: Vec<(u16, Value)>) -> (String, std::thread::JoinHandle<Vec<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let site = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for (status, body) in responses {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let (mut length, mut auth) = (0usize, String::new());
                loop {
                    let mut header = String::new();
                    reader.read_line(&mut header).unwrap();
                    if header.trim().is_empty() {
                        break;
                    }
                    let lower = header.to_ascii_lowercase();
                    if let Some(v) = lower.strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if lower.starts_with("authorization:") {
                        auth = header.trim().to_string();
                    }
                }
                let mut body_in = vec![0; length];
                reader.read_exact(&mut body_in).unwrap();
                seen.push(format!("{} {auth} {}", line.trim(), String::from_utf8_lossy(&body_in)));
                let payload = body.to_string();
                let mut stream = stream;
                write!(stream, "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}", payload.len())
                    .unwrap();
            }
            seen
        });
        (site, handle)
    }

    fn client(site: &str) -> JiraClient {
        JiraClient { site: site.to_string(), authorization: "Basic dGVzdA==".into() }
    }

    #[test]
    fn fetches_issue_searches_and_imports_without_duplicates() {
        let issue = json!({"key":"PROJ-7","fields":{"summary":"Fix login","status":{"name":"To Do"},"description":{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Steps"}]}]}}});
        let search = json!({"issues":[issue.clone(), {"key":"PROJ-8","fields":{"summary":"","description":null}}]});
        let (site, server) = mock_jira(vec![(200, issue), (200, search), (404, json!({}))]);
        let jira = client(&site);

        let fetched = jira.issue("PROJ-7").unwrap();
        assert_eq!(
            fetched,
            JiraIssue {
                key: "PROJ-7".into(),
                summary: "Fix login".into(),
                description: "Steps".into(),
                status: Some("To Do".into()),
                url: format!("{site}/browse/PROJ-7")
            }
        );
        let mine = jira.my_open_issues("assignee = currentUser()").unwrap();
        assert_eq!(mine.len(), 2);
        assert!(jira.issue("NOPE-1").unwrap_err().to_string().contains("no issue NOPE-1"));

        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("GET /rest/api/3/issue/PROJ-7?fields=summary,description,status"));
        assert!(requests[0].contains("Basic dGVzdA=="));
        assert!(requests[1].starts_with("POST /rest/api/3/search/jql") && requests[1].contains("currentUser()"));

        let dir = tempfile::tempdir().unwrap();
        let store = TicketStore::new(LanesHome::new(dir.path()));
        assert_eq!(import(&store, "backlog", &mine).unwrap(), ImportReport { created: vec!["PROJ-7".into(), "PROJ-8".into()], skipped: vec![] });
        assert_eq!(import(&store, "backlog", &mine[..1]).unwrap().skipped, ["PROJ-7"]);
        let ticket = store.load("PROJ-7").unwrap();
        assert_eq!(ticket.jira.unwrap().url, format!("{site}/browse/PROJ-7"));
        assert_eq!(store.load("PROJ-8").unwrap().title, "PROJ-8");
    }
}
