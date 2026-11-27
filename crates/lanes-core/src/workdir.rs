use std::path::PathBuf;

use anyhow::{Context, Result};
use minijinja::{Environment, context};

use crate::config::ColumnDef;
use crate::paths::expand_tilde;
use crate::ticket::Ticket;

/// Which setting decides a session's working folder, from most to least specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CwdSource {
    TicketColumn,
    Ticket,
    Column,
}

impl CwdSource {
    pub fn describe(self) -> &'static str {
        match self {
            Self::TicketColumn => "this ticket, this stage",
            Self::Ticket => "this ticket, all stages",
            Self::Column => "column default",
        }
    }
}

/// The folder a new session for this ticket in this column starts in, or None for columns without sessions.
pub fn resolve(ticket: &Ticket, column: &ColumnDef) -> Result<Option<PathBuf>> {
    let Some((template, _)) = template_for(ticket, column) else { return Ok(None) };
    let rendered = render(template, ticket, column).with_context(|| format!("working folder template '{template}'"))?;
    Ok(Some(expand_tilde(rendered.trim()).components().collect()))
}

pub fn source(ticket: &Ticket, column: &ColumnDef) -> Option<CwdSource> {
    template_for(ticket, column).map(|(_, source)| source)
}

fn template_for<'a>(ticket: &'a Ticket, column: &'a ColumnDef) -> Option<(&'a str, CwdSource)> {
    if !column.has_session() {
        return None;
    }
    let overrides = [(ticket.column_cwd.get(&column.id), CwdSource::TicketColumn), (ticket.cwd.as_ref(), CwdSource::Ticket), (column.cwd.as_ref(), CwdSource::Column)];
    overrides.into_iter().find_map(|(value, source)| value.filter(|v| !v.trim().is_empty()).map(|v| (v.as_str(), source)))
}

fn render(template: &str, ticket: &Ticket, column: &ColumnDef) -> Result<String> {
    let link = ticket.link.as_ref();
    let ctx = context! {
        ticket => context! {
            id => &ticket.id,
            title => &ticket.title,
            link_key => link.map(|l| l.key.as_str()),
            link_url => link.map(|l| l.url.as_str()),
            jira_key => link.map(|l| l.key.as_str()),
            jira_url => link.map(|l| l.url.as_str()),
        },
        column => context! { id => &column.id, title => column.title() },
    };
    Ok(Environment::new().render_str(template, ctx)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::BoardConfig;
    use crate::paths::LanesHome;
    use crate::ticket::TicketStore;

    #[test]
    fn precedence_is_ticket_column_then_ticket_then_column_template() {
        let config = BoardConfig::parse(
            "[[columns]]\nid = \"backlog\"\n[[columns]]\nid = \"dev\"\ncwd = \"~/worktrees/{{ ticket.id }}-{{ column.id }}\"\n[[columns]]\nid = \"qa\"\ncwd = \"C:/qa\"\n",
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let mut ticket = TicketStore::new(LanesHome::new(dir.path())).create(Some("PROJ-9"), "t", "", "dev").unwrap();
        let (dev, qa, backlog) = (config.column("dev").unwrap(), config.column("qa").unwrap(), config.column("backlog").unwrap());
        let home = dirs::home_dir().unwrap();

        assert_eq!(resolve(&ticket, dev).unwrap(), Some(home.join("worktrees/PROJ-9-dev")));
        assert_eq!(source(&ticket, dev), Some(CwdSource::Column));
        assert_eq!(resolve(&ticket, backlog).unwrap(), None);

        ticket.cwd = Some("D:/repos/{{ ticket.id | lower }}".into());
        assert_eq!(resolve(&ticket, qa).unwrap(), Some(PathBuf::from("D:/repos/proj-9")));
        assert_eq!(source(&ticket, qa), Some(CwdSource::Ticket));

        ticket.column_cwd.insert("qa".into(), "E:/staging".into());
        assert_eq!(resolve(&ticket, qa).unwrap(), Some(PathBuf::from("E:/staging")));
        assert_eq!(resolve(&ticket, dev).unwrap(), Some(PathBuf::from("D:/repos/proj-9")));
        assert_eq!(source(&ticket, qa), Some(CwdSource::TicketColumn));

        ticket.cwd = Some("   ".into());
        assert_eq!(source(&ticket, dev), Some(CwdSource::Column));
    }
}
