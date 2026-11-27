use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use uuid::Uuid;

use crate::agent::{self, AgentCli, AgentKind, LaunchRequest};
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
    /// Text to type into the agent's input box without sending it, once the session is ready.
    #[serde(default)]
    pub draft: Option<String>,
    #[serde(default)]
    pub agent: AgentKind,
    /// The agent's own id for the session, when it differs from `session_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
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
        let agent = agent::get(self.config.defaults.agent);
        let session_id = Uuid::new_v4();
        let agent_session_id = agent.new_session_id(session_id);
        let cwd = self.new_session_cwd(ticket, column)?;
        let has_system_prompt = agent.capabilities().system_prompt;
        let mode = if has_system_prompt { column.start_mode(&self.config.defaults) } else { StartMode::Prompt };
        let system_prompt = self.new_system_prompt(ticket, column, mode)?;
        let system_file = self.write_session_file(ticket, column, "system.md", &system_prompt)?;
        let initial_prompt = match (mode, has_system_prompt) {
            (StartMode::Wait, _) => None,
            (StartMode::Prompt, true) => Some(self.kickoff_argument(ticket, column)?),
            (StartMode::Prompt, false) => Some(self.first_message_with_instructions(ticket, column, &system_prompt)?),
        };
        let request = self.request(column, session_id, &agent_session_id, false, has_system_prompt.then_some(system_file.as_path()), initial_prompt);
        let args = agent.launch_args(&request)?;
        Ok(self.spec(ticket, column, agent, session_id, agent_session_id, LaunchKind::New, args, cwd))
    }

    fn new_session_cwd(&self, ticket: &Ticket, column: &ColumnDef) -> Result<PathBuf> {
        let cwd = workdir::resolve(ticket, column)?.unwrap_or_else(|| expand_tilde("~"));
        ensure!(
            cwd.is_dir(),
            "working folder {} for {} in '{}' does not exist; create it, or change the folder on the ticket or column",
            cwd.display(),
            ticket.id,
            column.title()
        );
        Ok(cwd)
    }

    fn new_system_prompt(&self, ticket: &Ticket, column: &ColumnDef, mode: StartMode) -> Result<String> {
        let prompts = self.prompts(ticket, column);
        match mode {
            StartMode::Wait => prompts.system_prompt_with_brief(),
            StartMode::Prompt => prompts.system_prompt(),
        }
    }

    fn resume_spec(&self, ticket: &Ticket, column: &ColumnDef, record: &SessionRecord) -> Result<LaunchSpec> {
        let agent = agent::get(record.agent);
        let agent_session_id = record.agent_session_id();
        let (draft, initial_prompt) = match self.reentry_message(ticket, column, record)? {
            Some(message) if self.can_draft(column) => (Some(message), None),
            message => (None, message),
        };
        let request = self.request(column, record.id, &agent_session_id, true, None, initial_prompt);
        let args = agent.launch_args(&request)?;
        let mut spec = self.spec(ticket, column, agent, record.id, agent_session_id, LaunchKind::Resume, args, record.cwd.clone());
        spec.draft = draft;
        Ok(spec)
    }

    fn reentry_message(&self, ticket: &Ticket, column: &ColumnDef, record: &SessionRecord) -> Result<Option<String>> {
        let changed = self.changed_inputs(ticket, column, record)?;
        if changed.is_empty() {
            return Ok(None);
        }
        Ok(Some(self.prompts(ticket, column).reentry(ticket.last_arrival_from(), &changed)?))
    }

    /// Wait mode leaves a re-entry note as an editable draft; an external terminal can't be typed into, so it is sent instead.
    fn can_draft(&self, column: &ColumnDef) -> bool {
        self.config.defaults.terminal == TerminalMode::Embedded && column.start_mode(&self.config.defaults) == StartMode::Wait
    }

    /// Stage settings are passed on every launch, since agents don't all keep them with a resumed session.
    fn request<'r>(
        &'r self,
        column: &'r ColumnDef,
        session_id: Uuid,
        agent_session_id: &'r str,
        resume: bool,
        system_prompt_file: Option<&'r Path>,
        initial_prompt: Option<String>,
    ) -> LaunchRequest<'r> {
        let defaults = &self.config.defaults;
        LaunchRequest {
            home: self.home,
            lanes_exe: self.lanes_exe,
            session_id,
            agent_session_id,
            resume,
            system_prompt_file,
            initial_prompt,
            model: column.model.as_deref().or(defaults.model.as_deref()),
            effort: column.effort.as_deref().or(defaults.effort.as_deref()),
            permission_mode: column.permission_mode.as_deref(),
            extra_args: &column.extra_args,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spec(
        &self,
        ticket: &Ticket,
        column: &ColumnDef,
        agent: &dyn AgentCli,
        session_id: Uuid,
        agent_session_id: String,
        kind: LaunchKind,
        args: Vec<String>,
        cwd: PathBuf,
    ) -> LaunchSpec {
        LaunchSpec {
            session_id,
            kind,
            program: self.config.agent_program(agent.kind()),
            args,
            cwd,
            env: vec![(HOME_ENV.into(), self.home.root().to_string_lossy().into_owned()), (TICKET_ENV.into(), ticket.id.clone()), (COLUMN_ENV.into(), column.id.clone())],
            title: format!("{} · {}", ticket.id, column.title()),
            draft: None,
            agent: agent.kind(),
            agent_session_id: (agent_session_id != session_id.to_string()).then_some(agent_session_id),
        }
    }

    /// For agents without a system prompt: the stage instructions and the brief together, by reference when long.
    fn first_message_with_instructions(&self, ticket: &Ticket, column: &ColumnDef, instructions: &str) -> Result<String> {
        let message = format!("{}\n\n{}", instructions.trim(), self.prompts(ticket, column).kickoff()?.trim()).trim().to_string();
        let file = self.write_session_file(ticket, column, "brief.md", &message)?;
        if message.len() <= self.config.defaults.kickoff_inline_limit {
            return Ok(message);
        }
        Ok(format!("Your instructions and ticket brief are in `{}`. Read it in full, then begin.", portable(&file)))
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
            agent: spec.agent,
            agent_session_id: spec.agent_session_id.clone(),
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
    fn opencode_sessions_send_instructions_first_and_resume_by_their_own_id() {
        let (_dir, home, config) = fixture_with("agent = \"opencode\"");
        let mut ticket = TicketStore::new(home.clone()).create(Some("T-6"), "Open it", "Details", "dev").unwrap();
        let launcher = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("lanes") };
        ArtifactStore::new(&home, &config, "T-6").write("plan", "the plan").unwrap();

        let new = launcher.prepare(&mut ticket, "dev").unwrap();
        assert_eq!(new.program, "opencode");
        assert_eq!(new.args[0], "--session");
        assert!(new.args[1].starts_with("ses_"));
        assert_eq!(new.args[2], "--prompt");
        assert!(new.args[3].contains("# Ticket T-6: Open it") && new.args[3].contains("the plan"), "{:?}", new.args);
        assert!(new.args.iter().all(|a| a != "--model"), "OpenCode ignores model");
        let record = ticket.session("dev").unwrap();
        assert_eq!((record.agent, record.agent_session_id.as_deref()), (AgentKind::OpenCode, Some(new.args[1].as_str())));

        let resumed = launcher.prepare(&mut ticket, "dev").unwrap();
        assert_eq!(resumed.args, ["--session", new.args[1].as_str()]);
        assert_eq!(resumed.session_id, new.session_id);
    }

    #[test]
    fn columns_without_cwd_have_no_session() {
        let (_dir, home, config) = fixture();
        let mut ticket = TicketStore::new(home.clone()).create(Some("T-3"), "T", "", "backlog").unwrap();
        let launcher = SessionLauncher { home: &home, config: &config, lanes_exe: Path::new("lanes") };
        assert!(launcher.prepare(&mut ticket, "backlog").is_err());
    }
}
