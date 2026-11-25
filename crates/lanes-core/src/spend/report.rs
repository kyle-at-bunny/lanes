//! `lanes spend`: spend per ticket and stage, and the A/B comparison.

use std::fmt::Write as _;

use anyhow::Result;

use super::ledger::{TicketSpend, compare, format_tokens};
use crate::board::Board;
use crate::config::BoardConfig;
use crate::paths::LanesHome;

pub fn run(home: &LanesHome, only_ticket: Option<&str>) -> Result<()> {
    let board = Board::open(home.clone(), std::env::current_exe()?)?;
    let spends = collect(&board, only_ticket)?;
    println!("{}", render(&board.config, &spends));
    Ok(())
}

/// Active and archived tickets; a ticket counts as finished once archived or in the board's last column.
fn collect(board: &Board, only_ticket: Option<&str>) -> Result<Vec<TicketSpend>> {
    let last_column = board.config.columns.last().map(|c| c.id.clone());
    let active = board.tickets.list()?.into_iter().map(|t| (t, false));
    let archived = board.tickets.list_archived()?.into_iter().map(|t| (t, true));
    let spends = active
        .chain(archived)
        .filter(|(t, _)| only_ticket.is_none_or(|id| t.id.eq_ignore_ascii_case(id)))
        .map(|(ticket, archived)| {
            let finished = archived || last_column.as_deref() == Some(ticket.column.as_str());
            board.ticket_spend(&ticket, finished)
        })
        .filter(|s| s.total.requests > 0)
        .collect();
    Ok(spends)
}

pub fn render(config: &BoardConfig, spends: &[TicketSpend]) -> String {
    if spends.is_empty() {
        return "No token usage found for this board's sessions yet.".into();
    }
    let mut out = String::from("Spend in input-token equivalents (API price ratios: cache read 0.1, writes 1.25-2, output 5).\n\n");
    let _ = writeln!(out, "{:<10} {:<12} {:<9} {:>9} {:>9} {:>9} {:>5}", "ticket", "stage", "variant", "spend", "peak ctx", "sessions", "cold");
    for spend in spends {
        let variant = spend.variant.map_or("-", |v| v.label());
        for stage in &spend.stages {
            let title = config.column(&stage.column).map_or(stage.column.as_str(), |c| c.title());
            let _ = writeln!(
                out,
                "{:<10} {:<12} {:<9} {:>9} {:>9} {:>9} {:>5}",
                spend.ticket_id,
                title,
                variant,
                format_tokens(stage.usage.weighted()),
                format_tokens(stage.usage.peak_context as f64),
                stage.sessions,
                stage.usage.cold_rebuilds
            );
        }
        let done = if spend.finished { " (finished)" } else { "" };
        let _ = writeln!(out, "{:<10} {:<12} {:<9} {:>9}{done}\n", spend.ticket_id, "total", variant, format_tokens(spend.total.weighted()));
    }
    out.push_str(&render_comparison(spends));
    out
}

fn render_comparison(spends: &[TicketSpend]) -> String {
    let mut out = format!("{:<12} {:>8} {:>9} {:>18} {:>14} {:>6}\n", "arm", "tickets", "finished", "mean per finished", "mean peak ctx", "cold");
    for row in compare(spends) {
        let mean = row.mean_finished.map_or("-".to_string(), format_tokens);
        let _ =
            writeln!(out, "{:<12} {:>8} {:>9} {:>18} {:>14} {:>6}", row.label, row.tickets, row.finished, mean, format_tokens(row.mean_peak_context), row.cold_rebuilds);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spend::Variant;
    use crate::spend::ledger::StageSpend;
    use crate::spend::usage::Usage;

    #[test]
    fn renders_stage_rows_totals_and_comparison() {
        let config = BoardConfig::parse("[[columns]]\nid = \"dev\"\ntitle = \"Dev\"\ncwd = \".\"\n").unwrap();
        let usage = Usage { requests: 3, cache_read: 2_000_000, output: 10_000, peak_context: 310_000, cold_rebuilds: 1, ..Default::default() };
        let spend = TicketSpend {
            ticket_id: "T-1".into(),
            variant: Some(Variant::Lean),
            finished: true,
            stages: vec![StageSpend { column: "dev".into(), sessions: 2, usage }],
            total: usage,
        };
        let text = render(&config, &[spend]);
        assert!(text.contains("T-1        Dev          lean           250k      310k         2     1"), "{text}");
        assert!(text.contains("total        lean           250k (finished)"), "{text}");
        assert!(text.contains("lean                1         1               250k"), "{text}");
        assert_eq!(render(&config, &[]), "No token usage found for this board's sessions yet.");
    }
}
