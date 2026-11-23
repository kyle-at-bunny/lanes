use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};

use crate::config::{ArtifactDef, ArtifactType, BoardConfig};
use crate::paths::{LanesHome, write_atomic};
use crate::timeline::{self, EventKind};

pub struct ArtifactStore<'a> {
    home: &'a LanesHome,
    config: &'a BoardConfig,
    ticket_id: &'a str,
}

impl<'a> ArtifactStore<'a> {
    pub fn new(home: &'a LanesHome, config: &'a BoardConfig, ticket_id: &'a str) -> Self {
        Self { home, config, ticket_id }
    }

    pub fn def(&self, name: &str) -> Result<&'a ArtifactDef> {
        self.config.artifacts.get(name).with_context(|| format!("unknown artifact '{name}'"))
    }

    pub fn file_path(&self, name: &str, def: &ArtifactDef) -> PathBuf {
        self.home.artifacts_dir(self.ticket_id).join(def.file_name_for(name))
    }

    /// The artifact's content, or None when it is missing or blank.
    pub fn read(&self, name: &str) -> Result<Option<String>> {
        let def = self.def(name)?;
        let content = match def.kind {
            ArtifactType::File => std::fs::read_to_string(self.file_path(name, def)).ok(),
            ArtifactType::Text => self.read_text_values()?.remove(name),
        };
        Ok(content.filter(|c| !c.trim().is_empty()))
    }

    pub fn write(&self, name: &str, content: &str) -> Result<()> {
        let def = self.def(name)?;
        match def.kind {
            ArtifactType::File => write_atomic(&self.file_path(name, def), content.as_bytes())?,
            ArtifactType::Text => self.write_text_value(name, content)?,
        }
        timeline::record(self.home, self.ticket_id, EventKind::ArtifactWritten, name)
    }

    pub fn hash(&self, name: &str) -> Result<Option<String>> {
        Ok(self.read(name)?.map(|content| content_hash(&content)))
    }

    pub fn snapshot<'n>(&self, names: impl IntoIterator<Item = &'n str>) -> Result<BTreeMap<String, String>> {
        let mut hashes = BTreeMap::new();
        for name in names {
            if let Some(hash) = self.hash(name)? {
                hashes.insert(name.to_string(), hash);
            }
        }
        Ok(hashes)
    }

    pub fn missing<'n>(&self, names: impl IntoIterator<Item = &'n str>) -> Result<Vec<String>> {
        let mut missing = Vec::new();
        for name in names {
            if self.read(name)?.is_none() {
                missing.push(name.to_string());
            }
        }
        Ok(missing)
    }

    fn read_text_values(&self) -> Result<BTreeMap<String, String>> {
        let path = self.home.artifact_values_file(self.ticket_id);
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let text = std::fs::read_to_string(&path)?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    fn write_text_value(&self, name: &str, content: &str) -> Result<()> {
        if !self.home.ticket_file(self.ticket_id).exists() {
            bail!("ticket '{}' does not exist", self.ticket_id);
        }
        let mut values = self.read_text_values()?;
        values.insert(name.to_string(), content.trim().to_string());
        write_atomic(&self.home.artifact_values_file(self.ticket_id), toml::to_string_pretty(&values)?.as_bytes())
    }
}

pub fn content_hash(content: &str) -> String {
    let digest = Sha256::digest(content.trim().as_bytes());
    digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
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
[[columns]]
id = "backlog"
"#;

    #[test]
    fn reads_writes_and_detects_missing() {
        let dir = tempfile::tempdir().unwrap();
        let home = LanesHome::new(dir.path());
        let config = BoardConfig::parse(BOARD).unwrap();
        TicketStore::new(home.clone()).create(Some("T-1"), "t", "", "backlog").unwrap();
        let store = ArtifactStore::new(&home, &config, "T-1");

        assert_eq!(store.missing(["plan", "branch"]).unwrap(), vec!["plan", "branch"]);
        store.write("plan", "# Plan\nDo it").unwrap();
        store.write("branch", "  feat/x  ").unwrap();
        store.write("plan", "# Plan\nDo it").unwrap();
        assert!(store.missing(["plan", "branch"]).unwrap().is_empty());
        assert_eq!(store.read("branch").unwrap().as_deref(), Some("feat/x"));

        let before = store.snapshot(["plan", "branch"]).unwrap();
        store.write("branch", "feat/y").unwrap();
        let after = store.snapshot(["plan", "branch"]).unwrap();
        assert_eq!(before["plan"], after["plan"]);
        assert_ne!(before["branch"], after["branch"]);

        store.write("branch", "   ").unwrap();
        assert_eq!(store.missing(["branch"]).unwrap(), vec!["branch"]);
        assert!(store.read("ghost").is_err());
    }
}
