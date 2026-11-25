//! Spend per ticket and stage, and the A/B comparison between variants.

use uuid::Uuid;

use super::Variant;
use super::usage::Usage;
use crate::config::BoardConfig;
use crate::ticket::Ticket;

#[derive(Debug, Clone, PartialEq)]
pub struct StageSpend {
    pub column: String,
    pub sessions: usize,
    pub usage: Usage,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TicketSpend {
    pub ticket_id: String,
    pub variant: Option<Variant>,
    pub finished: bool,
    pub stages: Vec<StageSpend>,
    pub total: Usage,
}

/// Usage of one of the ticket's sessions, as reported by the agent that ran it.
pub fn agent_usage(ticket: &Ticket, session_id: &Uuid) -> Option<Usage> {
    let (kind, agent_session_id) = ticket.session_agent(session_id)?;
    crate::agent::get(kind).token_usage(&agent_session_id)
}

/// Adds up every session each stage of the ticket has had, in board column order.
pub fn ticket_spend(ticket: &Ticket, config: &BoardConfig, finished: bool, usage_of: impl Fn(&Uuid) -> Option<Usage>) -> TicketSpend {
    let mut stages: Vec<StageSpend> = stage_columns(ticket, config).into_iter().map(|column| stage_spend(ticket, column, &usage_of)).collect();
    stages.retain(|s| s.usage.requests > 0);
    let mut total = Usage::default();
    for stage in &stages {
        total.add(&stage.usage);
    }
    TicketSpend { ticket_id: ticket.id.clone(), variant: ticket.spend_variant, finished, stages, total }
}

fn stage_columns(ticket: &Ticket, config: &BoardConfig) -> Vec<String> {
    let mut columns: Vec<String> = ticket.sessions.keys().cloned().chain(ticket.retired_sessions.iter().map(|r| r.column.clone())).collect();
    columns.sort_by_key(|c| (config.column_index(c).unwrap_or(usize::MAX), c.clone()));
    columns.dedup();
    columns
}

fn stage_spend(ticket: &Ticket, column: String, usage_of: &impl Fn(&Uuid) -> Option<Usage>) -> StageSpend {
    let ids = ticket.stage_session_ids(&column);
    let mut usage = Usage::default();
    for id in &ids {
        if let Some(session) = usage_of(id) {
            usage.add(&session);
        }
    }
    StageSpend { column, sessions: ids.len(), usage }
}

#[derive(Debug, Clone, PartialEq)]
pub struct VariantSummary {
    pub label: &'static str,
    pub tickets: usize,
    pub finished: usize,
    pub total: f64,
    pub mean_finished: Option<f64>,
    pub mean_peak_context: f64,
    pub cold_rebuilds: u64,
}

/// One row per arm (plus tickets that started before the test), comparing finished tickets' mean spend.
pub fn compare(tickets: &[TicketSpend]) -> Vec<VariantSummary> {
    let arms: [(&'static str, Option<Variant>); 3] = [("lean", Some(Variant::Lean)), ("baseline", Some(Variant::Baseline)), ("not in test", None)];
    arms.into_iter()
        .map(|(label, variant)| summarize(label, tickets.iter().filter(|t| t.variant == variant && t.total.requests > 0).collect()))
        .filter(|s| s.tickets > 0)
        .collect()
}

fn summarize(label: &'static str, tickets: Vec<&TicketSpend>) -> VariantSummary {
    let finished: Vec<_> = tickets.iter().filter(|t| t.finished).collect();
    let finished_total: f64 = finished.iter().map(|t| t.total.weighted()).sum();
    VariantSummary {
        label,
        tickets: tickets.len(),
        finished: finished.len(),
        total: tickets.iter().map(|t| t.total.weighted()).sum(),
        mean_finished: (!finished.is_empty()).then(|| finished_total / finished.len() as f64),
        mean_peak_context: tickets.iter().map(|t| t.total.peak_context as f64).sum::<f64>() / tickets.len().max(1) as f64,
        cold_rebuilds: tickets.iter().map(|t| t.total.cold_rebuilds).sum(),
    }
}

/// Compact token counts for tables and cards: 950, 12k, 3.4M.
pub fn format_tokens(tokens: f64) -> String {
    match tokens {
        t if t >= 1_000_000.0 => format!("{:.1}M", t / 1_000_000.0),
        t if t >= 1_000.0 => format!("{:.0}k", t / 1_000.0),
        t => format!("{t:.0}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::LanesHome;
    use crate::ticket::{SessionRecord, TicketStore};
    use std::collections::HashMap;

    const BOARD: &str = "[[columns]]\nid = \"plan\"\ncwd = \".\"\n[[columns]]\nid = \"dev\"\ncwd = \".\"\n[[columns]]\nid = \"done\"\n";

    fn record(id: Uuid) -> SessionRecord {
        SessionRecord { id, cwd: ".".into(), created_at: 0, last_entered_at: 0, input_hashes: Default::default(), agent: Default::default(), agent_session_id: None }
    }

    fn usage(read: u64, peak: u64, cold: u64) -> Usage {
        Usage { requests: 1, cache_read: read, peak_context: peak, cold_rebuilds: cold, ..Default::default() }
    }

    #[test]
    fn sums_current_and_retired_sessions_per_stage_in_column_order() {
        let dir = tempfile::tempdir().unwrap();
        let config = BoardConfig::parse(BOARD).unwrap();
        let mut ticket = TicketStore::new(LanesHome::new(dir.path())).create(Some("T-1"), "t", "", "dev").unwrap();
        let (plan, dev_old, dev_new) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        ticket.sessions.insert("dev".into(), record(dev_old));
        ticket.retire_session("dev");
        ticket.sessions.insert("dev".into(), record(dev_new));
        ticket.sessions.insert("plan".into(), record(plan));
        let usages = HashMap::from([(plan, usage(10_000, 50_000, 0)), (dev_old, usage(20_000, 300_000, 1)), (dev_new, usage(5_000, 90_000, 0))]);

        let spend = ticket_spend(&ticket, &config, false, |id| usages.get(id).copied());
        let stages: Vec<_> = spend.stages.iter().map(|s| (s.column.as_str(), s.sessions, s.usage.cache_read)).collect();
        assert_eq!(stages, [("plan", 1, 10_000), ("dev", 2, 25_000)]);
        assert_eq!((spend.total.cache_read, spend.total.peak_context, spend.total.cold_rebuilds), (35_000, 300_000, 1));
    }

    #[test]
    fn compares_finished_tickets_per_arm() {
        let ticket = |variant, finished, read| TicketSpend {
            ticket_id: "T".into(),
            variant,
            finished,
            stages: Vec::new(),
            total: Usage { requests: 1, cache_read: read, peak_context: 100_000, ..Default::default() },
        };
        let tickets = [
            ticket(Some(Variant::Lean), true, 10_000_000),
            ticket(Some(Variant::Lean), true, 20_000_000),
            ticket(Some(Variant::Lean), false, 50_000_000),
            ticket(Some(Variant::Baseline), true, 40_000_000),
            ticket(None, true, 1_000),
        ];
        let rows = compare(&tickets);
        let summary: Vec<_> = rows.iter().map(|r| (r.label, r.tickets, r.finished, r.mean_finished)).collect();
        assert_eq!(summary, [("lean", 3, 2, Some(1_500_000.0)), ("baseline", 1, 1, Some(4_000_000.0)), ("not in test", 1, 1, Some(100.0))]);
    }

    #[test]
    fn formats_token_counts() {
        assert_eq!(format_tokens(950.0), "950");
        assert_eq!(format_tokens(12_400.0), "12k");
        assert_eq!(format_tokens(3_440_000.0), "3.4M");
    }
}
