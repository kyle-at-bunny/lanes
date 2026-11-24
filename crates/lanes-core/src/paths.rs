use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use uuid::Uuid;

pub const HOME_ENV: &str = "LANES_HOME";
pub const TICKET_ENV: &str = "LANES_TICKET";
pub const COLUMN_ENV: &str = "LANES_COLUMN";

#[derive(Debug, Clone)]
pub struct LanesHome {
    root: PathBuf,
}

impl LanesHome {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn from_env() -> Result<Self> {
        if let Some(root) = std::env::var_os(HOME_ENV) {
            return Ok(Self::new(root));
        }
        let home = dirs::home_dir().context("could not determine the user's home directory")?;
        Ok(Self::new(home.join(".lanes")))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn board_file(&self) -> PathBuf {
        self.root.join("board.toml")
    }

    pub fn resolve(&self, relative: &str) -> PathBuf {
        let expanded = expand_tilde(relative);
        if expanded.is_absolute() { expanded } else { self.root.join(expanded) }
    }

    pub fn tickets_dir(&self) -> PathBuf {
        self.root.join("tickets")
    }

    pub fn archive_dir(&self) -> PathBuf {
        self.tickets_dir().join("_archive")
    }

    pub fn archived_ticket_dir(&self, ticket_id: &str) -> PathBuf {
        self.archive_dir().join(ticket_id)
    }

    pub fn ticket_dir(&self, ticket_id: &str) -> PathBuf {
        self.tickets_dir().join(ticket_id)
    }

    pub fn ticket_file(&self, ticket_id: &str) -> PathBuf {
        self.ticket_dir(ticket_id).join("ticket.toml")
    }

    pub fn artifacts_dir(&self, ticket_id: &str) -> PathBuf {
        self.ticket_dir(ticket_id).join("artifacts")
    }

    pub fn artifact_values_file(&self, ticket_id: &str) -> PathBuf {
        self.ticket_dir(ticket_id).join("artifacts.toml")
    }

    pub fn sessions_dir(&self, ticket_id: &str) -> PathBuf {
        self.ticket_dir(ticket_id).join("sessions")
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join("state")
    }

    pub fn hook_state_file(&self, session_id: &Uuid) -> PathBuf {
        self.state_dir().join(format!("{session_id}.hook.json"))
    }

    pub fn status_state_file(&self, session_id: &Uuid) -> PathBuf {
        self.state_dir().join(format!("{session_id}.status.json"))
    }

    pub fn session_settings_file(&self, session_id: &Uuid) -> PathBuf {
        self.state_dir().join(format!("{session_id}.settings.json"))
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
    let rest = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\"));
    match (rest, dirs::home_dir()) {
        (Some(rest), Some(home)) => home.join(rest),
        _ if path == "~" => dirs::home_dir().unwrap_or_else(|| PathBuf::from(path)),
        _ => PathBuf::from(path),
    }
}

/// Forward-slash form, which works in Git Bash, cmd and PowerShell alike.
pub fn portable(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, contents).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or_default()
}
