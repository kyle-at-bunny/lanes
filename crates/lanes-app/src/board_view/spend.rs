use std::collections::{HashMap, HashSet};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::config::TerminalMode;
use lanes_core::paths::now_unix;
use lanes_core::spend::ledger::{self, TicketSpend, format_tokens};
use lanes_core::ticket::Ticket;

use super::LanesApp;

/// Seconds before the selected ticket's spend is re-read from its transcripts.
const SPEND_REFRESH_SECS: u64 = 30;

/// Spend per ticket, read from Claude Code's transcripts off the UI thread.
#[derive(Default)]
pub(super) struct SpendTracker {
    cached: HashMap<String, (u64, TicketSpend)>,
    loading: HashSet<String>,
}

impl LanesApp {
    pub(super) fn refresh_selected_spend(&mut self, cx: &mut Context<Self>) {
        let Some(ticket) = self.selected.as_deref().and_then(|id| self.ticket(id)).cloned() else { return };
        let fresh = self.spend.cached.get(&ticket.id).is_some_and(|(at, _)| now_unix().saturating_sub(*at) < SPEND_REFRESH_SECS);
        if fresh || !self.spend.loading.insert(ticket.id.clone()) {
            return;
        }
        let (config, finished) = (self.board.config.clone(), self.is_last_column(&ticket.column));
        let background = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let spend = background.spawn(async move { ledger::ticket_spend(&ticket, &config, finished, |id| ledger::agent_usage(&ticket, id)) }).await;
            let _ = this.update(cx, |app, cx| {
                app.spend.loading.remove(&spend.ticket_id);
                app.spend.cached.insert(spend.ticket_id.clone(), (now_unix(), spend));
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn render_spend_row(&self, ticket: &Ticket, cx: &App) -> Option<impl IntoElement> {
        let (_, spend) = self.spend.cached.get(&ticket.id)?;
        if spend.total.requests == 0 {
            return None;
        }
        let stages: Vec<String> = spend.stages.iter().map(|s| format!("{} {}", self.board.column_title(&s.column), format_tokens(s.usage.weighted()))).collect();
        let mut text = format!("Spend {} · {} · peak ctx {}", format_tokens(spend.total.weighted()), stages.join(", "), format_tokens(spend.total.peak_context as f64));
        if self.board.config.spend.mode != lanes_core::config::SpendMode::Off || ticket.lean {
            text.push_str(&format!(" · {}", self.board.config.spend.variant(ticket).label()));
        }
        Some(div().text_xs().text_color(cx.theme().muted_foreground).child(text))
    }

    /// Lean tickets: ask a large session to summarise itself, then replace it with a fresh session seeded from that summary.
    pub(super) fn render_fresh_session_actions(&self, ticket: &Ticket, live: bool, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.board.config.spend.is_lean(ticket) {
            return None;
        }
        let record = ticket.session(&ticket.column)?;
        let summary_ready = self.board.has_fresh_summary(ticket, record.last_entered_at);
        let (handoff_id, fresh_id) = (ticket.id.clone(), ticket.id.clone());
        let row = h_flex()
            .flex_wrap()
            .gap_2()
            .when(live && !summary_ready, |this| {
                this.child(
                    Button::new("handoff-fresh")
                        .label("Hand off to a fresh session")
                        .small()
                        .tooltip("Ask this session to write a summary, so the stage can continue in a new, smaller session")
                        .on_click(cx.listener(move |this, _, window, cx| this.request_fresh_handoff(&handoff_id, window, cx))),
                )
            })
            .when(summary_ready, |this| {
                this.child(
                    Button::new("start-fresh")
                        .label("Start fresh from summary")
                        .primary()
                        .small()
                        .tooltip("End this session and start a new one for the stage, seeded with the brief, the inputs and the summary")
                        .on_click(cx.listener(move |this, _, window, cx| this.start_fresh_session(&fresh_id, window, cx))),
                )
            });
        Some(row)
    }

    /// A card hint once a lean session's context passes `[spend] nudge_at`.
    pub(super) fn nudge_label(&self, ticket: &Ticket, cx: &App) -> Option<impl IntoElement> {
        let tokens = ticket.session(&ticket.column).and_then(|r| self.board.states.status(&r.id)).and_then(|s| s.context_tokens);
        if !self.board.config.spend.should_nudge(ticket, tokens) {
            return None;
        }
        Some(div().text_color(cx.theme().warning).child(format!("{} · hand off", format_tokens(tokens? as f64))))
    }

    fn request_fresh_handoff(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.ticket(ticket_id) else { return };
        let Some(terminal) = self.live_terminal(ticket) else { return };
        match self.board.fresh_handoff_message(ticket) {
            Ok(message) => terminal.read(cx).send_text(&message),
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    fn start_fresh_session(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.ticket(ticket_id).and_then(|t| self.live_terminal(t)).map(|t| t.read(cx).session_id) {
            self.suspend_session(id, cx);
        }
        let result = self.board.start_fresh_session(ticket_id).and_then(|spec| match self.board.config.defaults.terminal {
            TerminalMode::Embedded => self.spawn_embedded(spec, window, cx),
            TerminalMode::External => self.spawn_external(&spec),
        });
        if let Err(err) = result {
            self.notify_error(&err, window, cx);
        }
        self.spend.cached.remove(ticket_id);
        self.reload_tickets(window, cx);
        cx.notify();
    }
}
