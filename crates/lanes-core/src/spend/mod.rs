//! Token-saving features for stage sessions, the per-ticket A/B assignment, and spend accounting.

pub mod ledger;
pub mod report;
pub mod usage;

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::Capabilities;
use crate::config::ColumnDef;
use crate::paths::LanesHome;
use crate::ticket::Ticket;

/// Claude Code rejects `--autocompact` windows below this.
pub const MIN_MAX_CONTEXT: u32 = 100_000;

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(default)]
pub struct SpendConfig {
    /// "off" runs every session as Claude Code's defaults; "lean" turns the features on for every ticket; "ab" turns them on for half of the new tickets so `lanes spend` can compare.
    pub mode: SpendMode,
    /// Lean sessions auto-compact at this many tokens of context instead of Claude Code's default (close to the model's window).
    pub max_context: u32,
    /// Lean sessions suggest handing off to a fresh session once their context passes this many tokens.
    pub nudge_at: u32,
}

impl Default for SpendConfig {
    fn default() -> Self {
        Self { mode: SpendMode::Off, max_context: 300_000, nudge_at: 250_000 }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SpendMode {
    #[default]
    Off,
    Lean,
    Ab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    Baseline,
    Lean,
}

impl Variant {
    pub fn label(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Lean => "lean",
        }
    }
}

impl SpendConfig {
    /// The variant a ticket's sessions run as: lean if the ticket asked for it, otherwise as the mode decides.
    pub fn variant(&self, ticket: &Ticket) -> Variant {
        if ticket.lean {
            return Variant::Lean;
        }
        match self.mode {
            SpendMode::Off => Variant::Baseline,
            SpendMode::Lean => Variant::Lean,
            SpendMode::Ab => ticket.spend_variant.unwrap_or(Variant::Baseline),
        }
    }

    pub fn is_lean(&self, ticket: &Ticket) -> bool {
        self.variant(ticket) == Variant::Lean
    }

    /// In an A/B test, puts a ticket that has never had a session into whichever arm has fewer tickets.
    /// Tickets that already had sessions stay unassigned, since part of their spend predates the test.
    pub fn assign_variant(&self, ticket: &mut Ticket, others: &[Ticket]) -> bool {
        let never_started = ticket.sessions.is_empty() && ticket.retired_sessions.is_empty();
        if self.mode != SpendMode::Ab || ticket.spend_variant.is_some() || ticket.lean || !never_started {
            return false;
        }
        let count = |variant| others.iter().filter(|t| t.id != ticket.id && t.spend_variant == Some(variant)).count();
        let variant = if count(Variant::Lean) <= count(Variant::Baseline) { Variant::Lean } else { Variant::Baseline };
        ticket.spend_variant = Some(variant);
        true
    }

    pub fn max_context(&self, column: &ColumnDef) -> u32 {
        column.max_context.unwrap_or(self.max_context).max(MIN_MAX_CONTEXT)
    }

    pub fn should_nudge(&self, ticket: &Ticket, context_tokens: Option<u64>) -> bool {
        self.is_lean(ticket) && context_tokens.is_some_and(|tokens| tokens >= u64::from(self.nudge_at))
    }
}

/// What lean does for an agent's sessions, in words for the UI and `config check`; hand-off works with every agent.
pub fn lean_features(capabilities: &Capabilities) -> Vec<&'static str> {
    let supported = [
        (capabilities.context_limit, "a context cap"),
        (capabilities.compaction_hook, "the brief again after compaction"),
        (capabilities.usage, "a nudge when context grows"),
        (true, "hand-off to a fresh session"),
    ];
    supported.into_iter().filter(|(on, _)| *on).map(|(_, feature)| feature).collect()
}

/// Where a stage session writes the summary its fresh successor starts from.
pub fn summary_path(home: &LanesHome, ticket_id: &str, column: &str) -> PathBuf {
    home.sessions_dir(ticket_id).join(format!("{column}.summary.md"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ticket::TicketStore;

    fn tickets(n: usize) -> (tempfile::TempDir, Vec<Ticket>) {
        let dir = tempfile::tempdir().unwrap();
        let store = TicketStore::new(LanesHome::new(dir.path()));
        let list = (0..n).map(|i| store.create(Some(&format!("T-{i}")), "t", "", "dev").unwrap()).collect();
        (dir, list)
    }

    #[test]
    fn a_ticket_can_be_lean_on_its_own_and_stays_out_of_the_test() {
        let (_dir, mut list) = tickets(1);
        let ticket = &mut list[0];
        ticket.lean = true;
        let off = SpendConfig::default();
        assert!(off.is_lean(ticket));
        let ab = SpendConfig { mode: SpendMode::Ab, ..SpendConfig::default() };
        assert!(!ab.assign_variant(ticket, &[]));
        assert_eq!((ticket.spend_variant, ab.variant(ticket)), (None, Variant::Lean));
    }

    #[test]
    fn lean_features_follow_the_agents_capabilities() {
        assert_eq!(lean_features(&crate::agent::get(crate::agent::AgentKind::Claude).capabilities()).len(), 4);
        assert_eq!(lean_features(&crate::agent::get(crate::agent::AgentKind::OpenCode).capabilities()), ["hand-off to a fresh session"]);
    }

    #[test]
    fn ab_mode_balances_new_tickets_and_skips_started_ones() {
        let config = SpendConfig { mode: SpendMode::Ab, ..Default::default() };
        let (_dir, mut list) = tickets(4);
        let mut assigned = Vec::new();
        for i in 0..3 {
            let others = list.clone();
            assert!(config.assign_variant(&mut list[i], &others));
            assigned.push(list[i].spend_variant.unwrap());
        }
        assert_eq!(assigned, [Variant::Lean, Variant::Baseline, Variant::Lean]);
        assert!(!config.assign_variant(&mut list[0], &[]), "assignment is sticky");

        let mut started = list[3].clone();
        started.sessions.insert(
            "dev".into(),
            crate::ticket::SessionRecord {
                id: uuid::Uuid::new_v4(),
                cwd: ".".into(),
                created_at: 0,
                last_entered_at: 0,
                input_hashes: Default::default(),
                agent: Default::default(),
                agent_session_id: None,
            },
        );
        assert!(!config.assign_variant(&mut started, &list));
        assert_eq!(config.variant(&started), Variant::Baseline);
    }

    #[test]
    fn mode_decides_variant_and_nudge() {
        let (_dir, mut list) = tickets(1);
        let ticket = &mut list[0];
        ticket.spend_variant = Some(Variant::Lean);
        let off = SpendConfig::default();
        assert_eq!(off.variant(ticket), Variant::Baseline);
        assert!(!off.should_nudge(ticket, Some(400_000)));
        let lean = SpendConfig { mode: SpendMode::Lean, ..Default::default() };
        assert!(lean.should_nudge(ticket, Some(250_000)));
        assert!(!lean.should_nudge(ticket, Some(249_999)));
        assert!(!lean.should_nudge(ticket, None));
    }

    #[test]
    fn max_context_respects_column_override_and_claude_minimum() {
        let config = SpendConfig::default();
        let mut column = ColumnDef::default();
        assert_eq!(config.max_context(&column), 300_000);
        column.max_context = Some(50_000);
        assert_eq!(config.max_context(&column), MIN_MAX_CONTEXT);
    }
}
