use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context, Result};
use minijinja::{Environment, context};

use crate::artifacts::ArtifactStore;
use crate::config::{ArtifactRef, ArtifactType, BoardConfig, ColumnDef};
use crate::paths::{LanesHome, portable};
use crate::session::lanes_command;
use crate::ticket::Ticket;

const DEFAULT_REENTRY: &str = "Ticket {{ ticket.id }} is back in the {{ column.title }} stage{% if from %} (returned from {{ from }}){% endif %}. \
These inputs changed since you last worked on it: {{ changed | join(', ') }}. Re-read them and carry on with the user.";

pub struct PromptBuilder<'a> {
    pub home: &'a LanesHome,
    pub config: &'a BoardConfig,
    pub ticket: &'a Ticket,
    pub column: &'a ColumnDef,
    pub lanes_exe: &'a Path,
}

impl PromptBuilder<'_> {
    pub fn system_prompt(&self) -> Result<String> {
        let mut prompt = self.render_column_prompt()?;
        prompt.push_str("\n\n");
        prompt.push_str(&self.outputs_section()?);
        Ok(prompt)
    }

    /// The system prompt for `start = "wait"`: the ticket brief is context, and the user sends the first message.
    pub fn system_prompt_with_brief(&self) -> Result<String> {
        let mut prompt = self.render_column_prompt()?;
        prompt.push_str("\n\n## Ticket brief\nThis is the ticket you are working on. The user will tell you what to do first, so wait for their message.\n\n");
        prompt.push_str(&self.kickoff()?);
        prompt.push_str("\n\n");
        prompt.push_str(&self.outputs_section()?);
        Ok(prompt)
    }

    /// The ticket brief: title, Jira link, description and inputs from earlier stages.
    pub fn kickoff(&self) -> Result<String> {
        let mut text = format!("# Ticket {}: {}\n\n", self.ticket.id, self.ticket.title);
        if let Some(jira) = &self.ticket.jira {
            let _ = write!(text, "Jira: {}\n\n", jira.url);
        }
        if !self.ticket.description.is_empty() {
            let _ = write!(text, "{}\n\n", self.ticket.description);
        }
        let inputs: Vec<_> = self.column.consumed().collect();
        if !inputs.is_empty() {
            text.push_str("## Inputs from earlier stages\n\n");
            text.push_str(&self.inputs_section(&inputs)?);
        }
        Ok(text)
    }

    pub fn reentry(&self, from: Option<&str>, changed: &[String]) -> Result<String> {
        let template = self.column.reentry.as_deref().unwrap_or(DEFAULT_REENTRY);
        let mut text = self.render(template, Some(from), changed)?;
        let refs: Vec<_> = changed.iter().map(|name| ArtifactRef { name, optional: true }).collect();
        text.push_str("\n\n");
        text.push_str(&self.inputs_section(&refs)?);
        Ok(text)
    }

    pub fn handoff(&self) -> Result<String> {
        let template = self.column.handoff.as_deref().unwrap_or(
            "Please wrap up this stage now: write every stage output listed in your instructions, then tell me they are ready.",
        );
        self.render(template, None, &[])
    }

    fn render_column_prompt(&self) -> Result<String> {
        let Some(relative) = &self.column.prompt else {
            return Ok(String::new());
        };
        let path = self.home.resolve(relative);
        let template = std::fs::read_to_string(&path).with_context(|| format!("reading prompt {}", path.display()))?;
        self.render(&template, None, &[])
    }

    fn render(&self, template: &str, from: Option<Option<&str>>, changed: &[String]) -> Result<String> {
        let env = Environment::new();
        let ctx = context! {
            ticket => context! {
                id => &self.ticket.id,
                title => &self.ticket.title,
                description => &self.ticket.description,
                jira_url => self.ticket.jira.as_ref().map(|j| j.url.as_str()),
            },
            column => context! { id => &self.column.id, title => self.column.title() },
            from => from.flatten(),
            changed => changed.to_vec(),
        };
        env.render_str(template, ctx).context("rendering prompt template")
    }

    fn inputs_section(&self, refs: &[ArtifactRef<'_>]) -> Result<String> {
        let store = ArtifactStore::new(self.home, self.config, &self.ticket.id);
        let mut text = String::new();
        for artifact in refs {
            let def = store.def(artifact.name)?;
            let content = store.read(artifact.name)?;
            match (content, def.kind) {
                (None, _) if artifact.optional => {}
                (None, _) => {
                    let _ = writeln!(text, "<artifact name=\"{}\" status=\"missing\">{}</artifact>\n", artifact.name, def.description);
                }
                (Some(_), ArtifactType::File) if !def.inline => {
                    let path = portable(&store.file_path(artifact.name, def));
                    let _ = writeln!(text, "<artifact name=\"{}\" path=\"{path}\">{} (read the file)</artifact>\n", artifact.name, def.description);
                }
                (Some(content), _) => {
                    let _ = writeln!(text, "<artifact name=\"{}\" description=\"{}\">\n{}\n</artifact>\n", artifact.name, def.description, content.trim());
                }
            }
        }
        Ok(text)
    }

    fn outputs_section(&self) -> Result<String> {
        let produced: Vec<_> = self.column.produced().collect();
        if produced.is_empty() {
            return Ok(String::new());
        }
        let store = ArtifactStore::new(self.home, self.config, &self.ticket.id);
        let lanes = format!("{} artifact set --ticket {}", lanes_command(self.lanes_exe, self.home), self.ticket.id);
        let mut text = String::from(
            "## Stage outputs\nThis ticket is tracked on a kanban board. Later stages see ONLY these artifacts, not this conversation, so make them self-contained. Before this stage is finished, produce:\n",
        );
        for artifact in produced {
            let def = store.def(artifact.name)?;
            let _ = writeln!(text, "- `{}`: {}", artifact.name, def.description);
            match def.kind {
                ArtifactType::File => {
                    let _ = writeln!(text, "  Write it as a markdown file at `{}`.", portable(&store.file_path(artifact.name, def)));
                }
                ArtifactType::Text => {
                    let _ = writeln!(text, "  Save it by running `{lanes} {} \"<value>\"` (pipe multi-line values to `{lanes} {} -`).", artifact.name, artifact.name);
                }
            }
        }
        text.push_str("The user decides when the ticket moves on; tell them when the outputs are ready.\n");
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::TicketStore;

    const BOARD: &str = r#"
[artifacts.plan]
type = "file"
inline = true
description = "the plan"
[artifacts.spec]
type = "file"
description = "a long spec"
[artifacts.branch]
type = "text"
description = "branch name"
[artifacts.notes]
type = "file"
description = "review notes"
[[columns]]
id = "plan"
cwd = "."
prompt = "prompts/plan.md"
produces = ["plan", "branch"]
[[columns]]
id = "dev"
cwd = "."
consumes = ["plan", "spec", "branch", "notes?"]
"#;

    fn fixture() -> (tempfile::TempDir, LanesHome, BoardConfig, Ticket) {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        std::fs::create_dir_all(dir.path().join("prompts")).unwrap();
        std::fs::write(dir.path().join("prompts/plan.md"), "Plan {{ ticket.id }} in {{ column.title }}").unwrap();
        let config = BoardConfig::parse(BOARD).unwrap();
        let ticket = TicketStore::new(home.clone()).create(Some("T-9"), "Fix it", "Details here", "plan").unwrap();
        (dir, home, config, ticket)
    }

    #[test]
    fn system_prompt_renders_template_and_outputs() {
        let (_dir, home, config, ticket) = fixture();
        let builder = PromptBuilder { home: &home, config: &config, ticket: &ticket, column: config.column("plan").unwrap(), lanes_exe: Path::new("C:/bin/lanes.exe") };
        let prompt = builder.system_prompt().unwrap();
        assert!(prompt.starts_with("Plan T-9 in plan"));
        assert!(prompt.contains("artifacts/plan.md"));
        assert!(prompt.contains("C:/bin/lanes.exe --home"));
        assert!(prompt.contains("artifact set --ticket T-9 branch"));
    }

    #[test]
    fn system_prompt_with_brief_holds_ticket_and_inputs() {
        let (_dir, home, config, ticket) = fixture();
        ArtifactStore::new(&home, &config, "T-9").write("plan", "Step one").unwrap();
        let builder = PromptBuilder { home: &home, config: &config, ticket: &ticket, column: config.column("dev").unwrap(), lanes_exe: Path::new("lanes") };
        let prompt = builder.system_prompt_with_brief().unwrap();
        assert!(prompt.contains("## Ticket brief"));
        assert!(prompt.contains("# Ticket T-9: Fix it"));
        assert!(prompt.contains("Details here"));
        assert!(prompt.contains("Step one"));
        assert!(!builder.system_prompt().unwrap().contains("Details here"));
    }

    #[test]
    fn kickoff_inlines_links_and_flags_missing() {
        let (_dir, home, config, ticket) = fixture();
        let store = ArtifactStore::new(&home, &config, "T-9");
        store.write("plan", "Step one").unwrap();
        store.write("spec", "huge").unwrap();
        let builder = PromptBuilder { home: &home, config: &config, ticket: &ticket, column: config.column("dev").unwrap(), lanes_exe: Path::new("lanes") };
        let kickoff = builder.kickoff().unwrap();
        assert!(kickoff.contains("# Ticket T-9: Fix it"));
        assert!(!kickoff.contains("Jira:"));

        let mut linked = ticket.clone();
        linked.jira = Some(crate::jira::JiraLink { key: "PROJ-1".into(), url: "https://x/browse/PROJ-1".into() });
        let linked_builder = PromptBuilder { ticket: &linked, ..builder };
        assert!(linked_builder.kickoff().unwrap().contains("# Ticket T-9: Fix it\n\nJira: https://x/browse/PROJ-1\n\n"));
        assert!(kickoff.contains("Step one"));
        assert!(kickoff.contains("spec.md\">a long spec (read the file)"));
        assert!(kickoff.contains("name=\"branch\" status=\"missing\""));
        assert!(!kickoff.contains("notes"));
    }

    #[test]
    fn reentry_lists_changed_inputs() {
        let (_dir, home, config, ticket) = fixture();
        ArtifactStore::new(&home, &config, "T-9").write("notes", "fix the bug").unwrap();
        let builder = PromptBuilder { home: &home, config: &config, ticket: &ticket, column: config.column("dev").unwrap(), lanes_exe: Path::new("lanes") };
        let text = builder.reentry(Some("review"), &["notes".into()]).unwrap();
        assert!(text.contains("returned from review"));
        assert!(text.contains("changed since you last worked on it: notes"));
        assert!(text.contains("artifacts/notes.md\">review notes (read the file)"));
    }
}
