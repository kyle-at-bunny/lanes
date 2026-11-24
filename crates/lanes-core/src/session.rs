use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use serde_json::json;
use uuid::Uuid;

use crate::artifacts::ArtifactStore;
use crate::config::{BoardConfig, ColumnDef, StartMode, TerminalMode};
use crate::paths::{COLUMN_ENV, HOME_ENV, LanesHome, TICKET_ENV, expand_tilde, now_unix, portable, write_atomic};
use crate::prompt::PromptBuilder;
use crate::ticket::{SessionRecord, Ticket};
use crate::timeline::{self, EventKind};
use crate::workdir;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LaunchKind {
    New,
    Resume,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LaunchSpec {
    pub session_id: Uuid,
    pub kind: LaunchKind,
    pub program: String,
    pub args: Vec<String>,
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub title: String,
    /// Text to type into Claude's input box without sending it, once the session is ready.
    #[serde(default)]
    pub draft: Option<String>,
}

pub struct SessionLauncher<'a> {
    pub home: &'a LanesHome,
    pub config: &'a BoardConfig,
    pub lanes_exe: &'a Path,
}

impl SessionLauncher<'_> {
    /// Builds the command for the (ticket, column) session: a new one on first entry, a resume afterwards.
    /// Records the entry on the ticket; the caller saves it.
    pub fn prepare(&self, ticket: &mut Ticket, column_id: &str) -> Result<LaunchSpec> {
        let column = self.config.column(column_id).with_context(|| format!("unknown column '{column_id}'"))?;
        ensure!(column.has_session(), "column '{column_id}' has no cwd, so it does not run sessions");
        let spec = match ticket.session(column_id).cloned() {
            Some(record) => self.resume_spec(ticket, column, &record)?,
            None => self.new_spec(ticket, column)?,
        };
        self.record_entry(ticket, column, &spec)?;
        let kind = if spec.kind == LaunchKind::New { EventKind::SessionStarted } else { EventKind::SessionResumed };
        timeline::record(self.home, &ticket.id, kind, column.title())?;
        Ok(spec)
    }

    pub fn handoff_message(&self, ticket: &Ticket, column_id: &str) -> Result<String> {
        let column = self.config.column(column_id).with_context(|| format!("unknown column '{column_id}'"))?;
        self.prompts(ticket, column).handoff()
    }

    fn new_spec(&self, ticket: &Ticket, column: &ColumnDef) -> Result<LaunchSpec> {
        let session_id = Uuid::new_v4();
        let cwd = workdir::resolve(ticket, column)?.unwrap_or_else(|| expand_tilde("~"));
        ensure!(
            cwd.is_dir(),
            "working folder {} for {} in '{}' does not exist; create it, or change the folder on the ticket or column",
            cwd.display(),
            ticket.id,
            column.title()
        );
        let mode = column.start_mode(&self.config.defaults);
        let system_file = self.write_session_file(ticket, column, "system.md", &self.new_system_prompt(ticket, column, mode)?)?;
        let mut args = vec!["--session-id".into(), session_id.to_string(), "--append-system-prompt-file".into(), portable(&system_file)];
        args.extend(self.common_args(column, &session_id)?);
        if mode == StartMode::Prompt {
            args.push(self.kickoff_argument(ticket, column)?);
        }
        Ok(self.spec(ticket, column, session_id, LaunchKind::New, args, cwd))
    }

    fn new_system_prompt(&self, ticket: &Ticket, column: &ColumnDef, mode: StartMode) -> Result<String> {
        let prompts = self.prompts(ticket, column);
        match mode {
            StartMode::Wait => prompts.system_prompt_with_brief(),
            StartMode::Prompt => prompts.system_prompt(),
        }
    }

    fn resume_spec(&self, ticket: &Ticket, column: &ColumnDef, record: &SessionRecord) -> Result<LaunchSpec> {
        let mut args = vec!["--resume".into(), record.id.to_string()];
        args.extend(self.common_args(column, &record.id)?);
        let changed = self.changed_inputs(ticket, column, record)?;
        let message = match changed.is_empty() {
            true => None,
            false => Some(self.prompts(ticket, column).reentry(ticket.last_arrival_from(), &changed)?),
        };
        let mut spec = self.spec(ticket, column, record.id, LaunchKind::Resume, args, record.cwd.clone());
        if let Some(message) = message {
            self.deliver_reentry(&mut spec, column, message);
        }
        Ok(spec)
    }

    /// Wait mode leaves the note as an editable draft; an external terminal can't be typed into, so it is sent instead.
    fn deliver_reentry(&self, spec: &mut LaunchSpec, column: &ColumnDef, message: String) {
        let can_type = self.config.defaults.terminal == TerminalMode::Embedded;
        match column.start_mode(&self.config.defaults) {
            StartMode::Wait if can_type => spec.draft = Some(message),
            _ => spec.args.push(message),
        }
    }

    fn spec(&self, ticket: &Ticket, column: &ColumnDef, session_id: Uuid, kind: LaunchKind, args: Vec<String>, cwd: PathBuf) -> LaunchSpec {
        LaunchSpec {
            session_id,
            kind,
            program: self.config.defaults.claude_path.clone(),
            args,
            cwd,
            env: vec![(HOME_ENV.into(), self.home.root().to_string_lossy().into_owned()), (TICKET_ENV.into(), ticket.id.clone()), (COLUMN_ENV.into(), column.id.clone())],
            title: format!("{} · {}", ticket.id, column.title()),
            draft: None,
        }
    }

    /// Flags that are not persisted with a session, so they are passed on every launch.
    fn common_args(&self, column: &ColumnDef, session_id: &Uuid) -> Result<Vec<String>> {
        let settings = write_session_settings(self.home, session_id, self.lanes_exe)?;
        let mut args = vec!["--settings".into(), portable(&settings)];
        let defaults = &self.config.defaults;
        let optional = [
            ("--model", column.model.as_ref().or(defaults.model.as_ref())),
            ("--effort", column.effort.as_ref().or(defaults.effort.as_ref())),
            ("--permission-mode", column.permission_mode.as_ref()),
        ];
        for (flag, value) in optional {
            if let Some(value) = value {
                args.extend([flag.to_string(), value.clone()]);
            }
        }
        args.extend(column.extra_args.iter().cloned());
        Ok(args)
    }

    fn kickoff_argument(&self, ticket: &Ticket, column: &ColumnDef) -> Result<String> {
        let kickoff = self.prompts(ticket, column).kickoff()?;
        let kickoff_file = self.write_session_file(ticket, column, "kickoff.md", &kickoff)?;
        if kickoff.len() <= self.config.defaults.kickoff_inline_limit {
            return Ok(kickoff);
        }
        Ok(format!("Your ticket brief is in `{}`. Read it in full, then begin.", portable(&kickoff_file)))
    }

    fn changed_inputs(&self, ticket: &Ticket, column: &ColumnDef, record: &SessionRecord) -> Result<Vec<String>> {
        let store = ArtifactStore::new(self.home, self.config, &ticket.id);
        let mut changed = Vec::new();
        for artifact in column.consumed() {
            let current = store.hash(artifact.name)?;
            if current.is_some() && current.as_ref() != record.input_hashes.get(artifact.name) {
                changed.push(artifact.name.to_string());
            }
        }
        Ok(changed)
    }

    fn record_entry(&self, ticket: &mut Ticket, column: &ColumnDef, spec: &LaunchSpec) -> Result<()> {
        let store = ArtifactStore::new(self.home, self.config, &ticket.id);
        let input_hashes = store.snapshot(column.consumed().map(|a| a.name))?;
        let now = now_unix();
        let record = ticket.sessions.entry(column.id.clone()).or_insert_with(|| SessionRecord {
            id: spec.session_id,
            cwd: spec.cwd.clone(),
            created_at: now,
            last_entered_at: now,
            input_hashes: Default::default(),
        });
        record.last_entered_at = now;
        record.input_hashes = input_hashes;
        Ok(())
    }

    fn prompts<'b>(&'b self, ticket: &'b Ticket, column: &'b ColumnDef) -> PromptBuilder<'b> {
        PromptBuilder { home: self.home, config: self.config, ticket, column, lanes_exe: self.lanes_exe }
    }

    fn write_session_file(&self, ticket: &Ticket, column: &ColumnDef, suffix: &str, contents: &str) -> Result<PathBuf> {
        let path = self.home.sessions_dir(&ticket.id).join(format!("{}.{suffix}", column.id));
        write_atomic(&path, contents.as_bytes())?;
        Ok(path)
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

pub fn lanes_command(lanes_exe: &Path, home: &LanesHome) -> String {
    format!("{} --home {}", shell_quote(&portable(lanes_exe)), shell_quote(&portable(home.root())))
}

fn shell_quote(value: &str) -> String {
    if value.contains(' ') { format!("\"{value}\"") } else { value.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::TicketStore;

    fn fixture() -> (tempfile::TempDir, LanesHome, BoardConfig) {
        fixture_with("start = \"prompt\"")
    }

    fn fixture_with(extra_defaults: &str) -> (tempfile::TempDir, LanesHome, BoardConfig) {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let cwd = portable(dir.path());
        let board = format!(
            r#"
[defaults]
{extra_defaults}
model = "opus"
[artifacts.plan]
type = "file"
inline = true
[artifacts.notes]
type = "file"
[[columns]]
id = "backlog"
[[columns]]
id = "plan"
cwd = "{cwd}"
produces = ["plan"]
permission_mode = "plan"
[[columns]]
id = "dev"
cwd = "{cwd}"
consumes = ["plan", "notes?"]
"#
        );
        (dir, home.clone(), BoardConfig::parse(&board).unwrap())
    }

    #[test]
    fn first_entry_creates_session_and_reentry_resumes_it() {
        let (_dir, home, config) = fixture();
        let tickets = TicketStore::new(home.clone());
        let mut ticket = tickets.create(Some("T-1"), "Title", "", "plan").unwrap();
        let launcher = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("C:/bin/lanes.exe") };

        let plan = launcher.prepare(&mut ticket, "plan").unwrap();
        assert_eq!(plan.kind, LaunchKind::New);
        assert_eq!(plan.args[0], "--session-id");
        assert!(plan.args.windows(2).any(|w| w == ["--permission-mode", "plan"]));
        assert!(plan.args.windows(2).any(|w| w == ["--model", "opus"]));
        assert!(plan.args.last().unwrap().starts_with("# Ticket T-1: Title"));
        assert!(home.session_settings_file(&plan.session_id).exists());

        ArtifactStore::new(&home, &config, "T-1").write("plan", "the plan").unwrap();
        ticket.move_to("dev");
        let dev = launcher.prepare(&mut ticket, "dev").unwrap();
        assert_eq!(dev.kind, LaunchKind::New);
        assert_ne!(dev.session_id, plan.session_id);
        assert!(dev.args.last().unwrap().contains("the plan"));

        ticket.move_to("plan");
        let back = launcher.prepare(&mut ticket, "plan").unwrap();
        assert_eq!(back.kind, LaunchKind::Resume);
        assert_eq!(back.session_id, plan.session_id);
        assert_eq!(&back.args[..2], ["--resume".to_string(), plan.session_id.to_string()]);
    }

    #[test]
    fn resume_sends_reentry_only_when_inputs_changed() {
        let (_dir, home, config) = fixture();
        let mut ticket = TicketStore::new(home.clone()).create(Some("T-2"), "T", "", "dev").unwrap();
        let launcher = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("lanes") };
        ArtifactStore::new(&home, &config, "T-2").write("plan", "v1").unwrap();
        launcher.prepare(&mut ticket, "dev").unwrap();

        let unchanged = launcher.prepare(&mut ticket, "dev").unwrap();
        assert!(unchanged.args.iter().all(|a| !a.contains("changed since")));

        ArtifactStore::new(&home, &config, "T-2").write("notes", "fix x").unwrap();
        let changed = launcher.prepare(&mut ticket, "dev").unwrap();
        assert!(changed.args.last().unwrap().contains("changed since you last worked on it: notes"));
    }

    #[test]
    fn wait_mode_puts_brief_in_system_prompt_and_drafts_reentry() {
        let (_dir, home, config) = fixture_with("");
        let mut ticket = TicketStore::new(home.clone()).create(Some("T-4"), "Wait here", "Some details", "dev").unwrap();
        let launcher = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("lanes") };
        ArtifactStore::new(&home, &config, "T-4").write("plan", "the plan").unwrap();

        let new = launcher.prepare(&mut ticket, "dev").unwrap();
        assert!(new.args.iter().all(|a| !a.contains("# Ticket")), "{:?}", new.args);
        assert_eq!(new.args.last().unwrap(), "opus");
        let system = std::fs::read_to_string(home.sessions_dir("T-4").join("dev.system.md")).unwrap();
        assert!(system.contains("# Ticket T-4: Wait here") && system.contains("Some details") && system.contains("the plan"));
        assert!(new.draft.is_none());

        ArtifactStore::new(&home, &config, "T-4").write("notes", "fix x").unwrap();
        let resumed = launcher.prepare(&mut ticket, "dev").unwrap();
        assert!(resumed.draft.as_deref().unwrap().contains("changed since you last worked on it: notes"));
        assert!(resumed.args.iter().all(|a| !a.contains("changed since")));
    }

    #[test]
    fn external_terminal_sends_reentry_even_in_wait_mode() {
        let (_dir, home, config) = fixture_with("terminal = \"external\"");
        let mut ticket = TicketStore::new(home.clone()).create(Some("T-5"), "T", "", "dev").unwrap();
        let launcher = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("lanes") };
        launcher.prepare(&mut ticket, "dev").unwrap();
        ArtifactStore::new(&home, &config, "T-5").write("notes", "fix x").unwrap();
        let resumed = launcher.prepare(&mut ticket, "dev").unwrap();
        assert!(resumed.draft.is_none());
        assert!(resumed.args.last().unwrap().contains("changed since"));
    }

    #[test]
    fn columns_without_cwd_have_no_session() {
        let (_dir, home, config) = fixture();
        let mut ticket = TicketStore::new(home.clone()).create(Some("T-3"), "T", "", "backlog").unwrap();
        let launcher = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("lanes") };
        assert!(launcher.prepare(&mut ticket, "backlog").is_err());
    }
}
