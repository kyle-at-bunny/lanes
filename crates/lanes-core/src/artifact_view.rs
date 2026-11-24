use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Result;

use crate::artifacts::ArtifactStore;
use crate::cli::format_duration;
use crate::config::{ArtifactRef, ArtifactType, BoardConfig, ColumnDef};
use crate::paths::{LanesHome, expand_tilde};
use crate::ticket::Ticket;
use crate::timeline::{self, EventKind};

/// When an artifact was last written, and by which stage (a column id) or `timeline::USER`, if known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    pub by: Option<String>,
    pub at: u64,
}

impl Provenance {
    /// e.g. "written by Plan / Arch · 5m ago", or "updated 5m ago" when the author is unknown.
    pub fn describe(&self, config: &BoardConfig, now: u64) -> String {
        let elapsed = now.saturating_sub(self.at);
        let ago = if elapsed < 60 { "just now".to_string() } else { format!("{} ago", format_duration(elapsed)) };
        match self.by.as_deref() {
            Some(timeline::USER) => format!("written by you · {ago}"),
            Some(column) => format!("written by {} · {ago}", config.column(column).map(ColumnDef::title).unwrap_or(column)),
            None => format!("updated {ago}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ArtifactEntry {
    pub name: String,
    pub kind: ArtifactType,
    pub description: String,
    pub required: bool,
    pub content: Option<String>,
    pub path: Option<PathBuf>,
    pub provenance: Option<Provenance>,
}

impl ArtifactEntry {
    pub fn is_present(&self) -> bool {
        self.content.is_some()
    }

    pub fn is_missing_required(&self) -> bool {
        self.required && !self.is_present()
    }
}

/// A ticket's artifacts grouped by how its current column uses them.
#[derive(Debug, Clone, Default)]
pub struct ArtifactPanel {
    pub inputs: Vec<ArtifactEntry>,
    pub outputs: Vec<ArtifactEntry>,
    pub other: Vec<ArtifactEntry>,
}

impl ArtifactPanel {
    pub fn load(home: &LanesHome, config: &BoardConfig, ticket: &Ticket) -> Result<Self> {
        let store = ArtifactStore::new(home, config, &ticket.id);
        let writes = latest_writes(home, &ticket.id);
        let entry = |artifact: ArtifactRef<'_>| load_entry(&store, &writes, artifact);
        let column = config.column(&ticket.column);
        let outputs: Vec<ArtifactRef<'_>> = column.map(|c| c.produced().collect()).unwrap_or_default();
        let is_output = |name: &str| outputs.iter().any(|o| o.name == name);
        let inputs: Vec<ArtifactRef<'_>> = column.map(ColumnDef::consumed).into_iter().flatten().filter(|a| !is_output(a.name)).collect();
        let is_input = |name: &str| inputs.iter().any(|i| i.name == name);
        let other: Vec<ArtifactRef<'_>> =
            config.artifacts.keys().filter(|name| !is_output(name) && !is_input(name)).map(|name| ArtifactRef { name, optional: true }).collect();
        Ok(Self {
            inputs: inputs.into_iter().map(entry).collect::<Result<_>>()?,
            outputs: outputs.iter().copied().map(entry).collect::<Result<_>>()?,
            other: other.into_iter().map(entry).collect::<Result<_>>()?,
        })
    }

    /// e.g. "Inputs 2/3 · Outputs 0/1"; groups the column doesn't have are left out.
    pub fn summary(&self) -> String {
        let count = |entries: &[ArtifactEntry]| format!("{}/{}", entries.iter().filter(|e| e.is_present()).count(), entries.len());
        let mut parts = Vec::new();
        if !self.inputs.is_empty() {
            parts.push(format!("Inputs {}", count(&self.inputs)));
        }
        if !self.outputs.is_empty() {
            parts.push(format!("Outputs {}", count(&self.outputs)));
        }
        parts.join(" · ")
    }

    pub fn find(&self, name: &str) -> Option<&ArtifactEntry> {
        self.inputs.iter().chain(&self.outputs).chain(&self.other).find(|e| e.name == name)
    }
}

fn load_entry(store: &ArtifactStore<'_>, writes: &BTreeMap<String, Provenance>, artifact: ArtifactRef<'_>) -> Result<ArtifactEntry> {
    let def = store.def(artifact.name)?;
    let path = (def.kind == ArtifactType::File).then(|| store.file_path(artifact.name, def));
    let content = store.read(artifact.name)?;
    let recorded = writes.get(artifact.name).cloned();
    let provenance = match (&content, &path) {
        (None, _) => None,
        (Some(_), Some(path)) => newer_file_edit(path, recorded),
        (Some(_), None) => recorded,
    };
    Ok(ArtifactEntry { name: artifact.name.to_string(), kind: def.kind, description: def.description.clone(), required: !artifact.optional, content, path, provenance })
}

/// A file edited outside any session (e.g. in an editor) is newer than its last recorded write and has no known author.
fn newer_file_edit(path: &std::path::Path, recorded: Option<Provenance>) -> Option<Provenance> {
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let mtime = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs());
    match (recorded, mtime) {
        (Some(recorded), Some(mtime)) if mtime > recorded.at + 2 => Some(Provenance { by: None, at: mtime }),
        (Some(recorded), _) => Some(recorded),
        (None, mtime) => mtime.map(|at| Provenance { by: None, at }),
    }
}

/// The most recent `artifact_written` event per artifact name.
pub fn latest_writes(home: &LanesHome, ticket_id: &str) -> BTreeMap<String, Provenance> {
    timeline::read(home, ticket_id)
        .into_iter()
        .filter(|event| event.kind == EventKind::ArtifactWritten)
        .map(|event| (event.detail, Provenance { by: event.by, at: event.at }))
        .collect()
}

/// One repository row of the `branch` artifact: `<absolute repo path> | <branch> | <PR URL or "no PR">`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchRow {
    pub repo: String,
    pub branch: String,
    pub pr: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentLine {
    Branch(BranchRow),
    Text(String),
}

pub fn parse_lines(content: &str) -> Vec<ContentLine> {
    content.lines().map(|line| parse_branch_row(line).map(ContentLine::Branch).unwrap_or_else(|| ContentLine::Text(line.to_string()))).collect()
}

fn parse_branch_row(line: &str) -> Option<BranchRow> {
    let parts: Vec<&str> = line.split('|').map(str::trim).collect();
    let [repo, branch, pr] = parts.as_slice() else { return None };
    if !is_local_path(repo) || branch.is_empty() || branch.contains(char::is_whitespace) {
        return None;
    }
    let pr = match *pr {
        pr if pr.eq_ignore_ascii_case("no PR") => None,
        pr if is_url(pr) => Some(pr.to_string()),
        _ => return None,
    };
    Some(BranchRow { repo: repo.to_string(), branch: branch.to_string(), pr })
}

/// A run of artifact text: plain, a URL to open in the browser, or a local path to open.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Segment {
    Text(String),
    Url(String),
    Path(String),
}

const LEADING_PUNCTUATION: &[char] = &['`', '\'', '"', '(', '<', '['];
const TRAILING_PUNCTUATION: &[char] = &['`', '\'', '"', ')', '>', ']', '.', ',', ';', ':', '!', '?'];

/// Splits text into plain runs, URLs and local paths, keeping surrounding punctuation as text.
pub fn segments(text: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    for piece in text.split_inclusive(char::is_whitespace) {
        let word = piece.trim_end();
        let (lead, rest) = word.split_at(word.len() - word.trim_start_matches(LEADING_PUNCTUATION).len());
        let core = rest.trim_end_matches(TRAILING_PUNCTUATION);
        let link = match core {
            core if is_url(core) => Some(Segment::Url(core.to_string())),
            core if is_local_path(core) => Some(Segment::Path(core.to_string())),
            _ => None,
        };
        match link {
            Some(link) => {
                push_text(&mut out, lead);
                out.push(link);
                push_text(&mut out, &piece[lead.len() + core.len()..]);
            }
            None => push_text(&mut out, piece),
        }
    }
    out
}

fn push_text(out: &mut Vec<Segment>, text: &str) {
    if text.is_empty() {
        return;
    }
    match out.last_mut() {
        Some(Segment::Text(existing)) => existing.push_str(text),
        _ => out.push(Segment::Text(text.to_string())),
    }
}

pub fn is_url(text: &str) -> bool {
    let rest = text.strip_prefix("https://").or_else(|| text.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty())
}

/// Windows drive paths, UNC paths and `~/` paths.
pub fn is_local_path(text: &str) -> bool {
    let bytes = text.as_bytes();
    let drive = bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/');
    let unc = text.starts_with("\\\\") && text.len() > 2;
    let home = (text.starts_with("~/") || text.starts_with("~\\")) && text.len() > 2;
    drive || unc || home
}

/// Shortens a long path for display by keeping its last components, e.g. `…\demo\app`.
pub fn display_path(path: &str, max_chars: usize) -> String {
    if path.chars().count() <= max_chars {
        return path.to_string();
    }
    let separator = if path.contains('\\') { '\\' } else { '/' };
    let mut tail: Vec<&str> = Vec::new();
    let mut length = 1;
    for part in path.rsplit(['\\', '/']).filter(|p| !p.is_empty()) {
        if !tail.is_empty() && length + part.chars().count() + 1 > max_chars {
            break;
        }
        length += part.chars().count() + 1;
        tail.push(part);
    }
    tail.reverse();
    format!("…{separator}{}", tail.join(&separator.to_string()))
}

/// The folder to open for a path: the path itself if it is a folder, otherwise the folder containing it.
pub fn folder_for(path: &str) -> PathBuf {
    let path = expand_tilde(path);
    match path.is_dir() {
        true => path,
        false => path.parent().map(PathBuf::from).unwrap_or(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::TicketStore;

    const BOARD: &str = r#"
[artifacts.plan]
type = "file"
[artifacts.branch]
type = "text"
[artifacts.notes]
type = "file"
[artifacts.qa]
type = "text"
[[columns]]
id = "plan"
cwd = "."
produces = ["plan"]
[[columns]]
id = "dev"
cwd = "."
consumes = ["plan", "notes?"]
produces = ["branch"]
"#;

    #[test]
    fn groups_artifacts_by_current_column_with_provenance() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let config = BoardConfig::parse(BOARD).unwrap();
        let ticket = TicketStore::new(home.clone()).create(Some("T-1"), "t", "", "dev").unwrap();
        ArtifactStore::new(&home, &config, "T-1").write_by("plan", "# Plan", Some("plan")).unwrap();

        let panel = ArtifactPanel::load(&home, &config, &ticket).unwrap();
        let names = |entries: &[ArtifactEntry]| entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        assert_eq!(names(&panel.inputs), ["plan", "notes"]);
        assert_eq!(names(&panel.outputs), ["branch"]);
        assert_eq!(names(&panel.other), ["qa"]);
        assert!(!panel.inputs[1].required && panel.outputs[0].is_missing_required());
        assert_eq!(panel.summary(), "Inputs 1/2 · Outputs 0/1");
        assert_eq!(panel.find("plan").unwrap().provenance.as_ref().unwrap().by.as_deref(), Some("plan"));
        assert!(panel.find("branch").unwrap().provenance.is_none());

        let by = |by: Option<&str>| Provenance { by: by.map(str::to_string), at: 1000 }.describe(&config, 1000 + 300);
        assert_eq!(by(Some("plan")), "written by plan · 5m ago");
        assert_eq!(by(Some(timeline::USER)), "written by you · 5m ago");
        assert_eq!(by(None), "updated 5m ago");
        assert_eq!(Provenance { by: None, at: 1000 }.describe(&config, 1010), "updated just now");
    }

    #[test]
    fn parses_branch_rows_and_keeps_other_lines_as_text() {
        let content = "C:\\repos\\app | feat/x | https://github.com/o/app/pull/7\n~/repos/lib | main | no PR\nsee notes | x | y";
        let lines = parse_lines(content);
        assert_eq!(
            lines[0],
            ContentLine::Branch(BranchRow { repo: "C:\\repos\\app".into(), branch: "feat/x".into(), pr: Some("https://github.com/o/app/pull/7".into()) })
        );
        assert_eq!(lines[1], ContentLine::Branch(BranchRow { repo: "~/repos/lib".into(), branch: "main".into(), pr: None }));
        assert_eq!(lines[2], ContentLine::Text("see notes | x | y".into()));
        assert_eq!(parse_lines("D:/a | two words | no PR")[0], ContentLine::Text("D:/a | two words | no PR".into()));
    }

    #[test]
    fn finds_urls_and_paths_in_text() {
        let found = segments("PR (https://x.io/pull/1), files in `C:\\repo\\src`. ok");
        assert_eq!(
            found,
            [
                Segment::Text("PR (".into()),
                Segment::Url("https://x.io/pull/1".into()),
                Segment::Text("), files in `".into()),
                Segment::Path("C:\\repo\\src".into()),
                Segment::Text("`. ok".into()),
            ]
        );
        assert_eq!(segments("plain words"), [Segment::Text("plain words".into())]);
        assert!(!is_local_path("a/b") && !is_url("https://"));
    }

    #[test]
    fn display_path_keeps_the_tail() {
        assert_eq!(display_path("C:\\repos\\app", 40), "C:\\repos\\app");
        assert_eq!(display_path("C:\\Users\\someone\\AppData\\Local\\Temp\\demo\\app", 16), "…\\Temp\\demo\\app");
        assert_eq!(display_path("~/a/very-long-folder-name-here", 10), "…/very-long-folder-name-here");
    }

    #[test]
    fn folder_for_files_is_their_parent() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        assert_eq!(folder_for(&file.to_string_lossy()), dir.path());
        assert_eq!(folder_for(&dir.path().to_string_lossy()), dir.path());
    }
}
