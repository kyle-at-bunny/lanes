use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::*;
use lanes_core::agent::{self, AgentKind};
use lanes_core::config::SpendMode;
use lanes_core::spend::lean_features;
use lanes_core::ticket::Ticket;

use super::LanesApp;

impl LanesApp {
    /// Picks the ticket's agent from the board's choices; offered only until its first session starts.
    pub(super) fn render_agent_choice(&self, ticket: &Ticket, cx: &mut Context<Self>) -> Option<AnyElement> {
        let choices = self.board.config.agent_choices();
        if choices.len() < 2 || ticket.has_had_session() {
            return None;
        }
        let current = self.board.config.ticket_agent(ticket);
        let buttons: Vec<AnyElement> = choices
            .into_iter()
            .map(|kind| {
                let ticket_id = ticket.id.clone();
                let button = Button::new(SharedString::from(format!("agent-{}", kind.config_name()))).label(agent::get(kind).display_name()).xsmall();
                let button = if kind == current { button.primary() } else { button.ghost() };
                button.on_click(cx.listener(move |this, _, window, cx| this.choose_ticket_agent(&ticket_id, kind, window, cx))).into_any_element()
            })
            .collect();
        let label = div().text_color(cx.theme().muted_foreground).child("Agent");
        Some(h_flex().flex_wrap().items_center().gap_1().text_xs().child(label).children(buttons).into_any_element())
    }

    /// Lean sessions for this ticket on their own, before its first session; says what lean means for the ticket's agent.
    pub(super) fn render_lean_choice(&self, ticket: &Ticket, cx: &mut Context<Self>) -> Option<AnyElement> {
        if ticket.has_had_session() || self.board.config.spend.mode == SpendMode::Lean {
            return None;
        }
        let agent = agent::get(self.board.config.ticket_agent(ticket));
        let buttons: Vec<AnyElement> = [(false, "Off"), (true, "On")]
            .into_iter()
            .map(|(lean, label)| {
                let ticket_id = ticket.id.clone();
                let button = Button::new(SharedString::from(format!("lean-{label}"))).label(label).xsmall();
                let button = if ticket.lean == lean { button.primary() } else { button.ghost() };
                button.on_click(cx.listener(move |this, _, window, cx| this.choose_ticket_lean(&ticket_id, lean, window, cx))).into_any_element()
            })
            .collect();
        let theme = cx.theme();
        let features = format!("{}: {}", agent.display_name(), lean_features(&agent.capabilities()).join(", "));
        let choice = h_flex().items_center().gap_1().child(div().text_color(theme.muted_foreground).child("Lean")).children(buttons);
        Some(v_flex().gap_0p5().text_xs().child(choice).child(div().w_full().text_color(theme.muted_foreground).child(features)).into_any_element())
    }

    /// Small tags on the card for a non-default agent and for lean sessions picked for the ticket.
    pub(super) fn card_tags(&self, ticket: &Ticket) -> Vec<AnyElement> {
        let lean = ticket.lean.then(|| Tag::secondary().outline().xsmall().child("lean").into_any_element());
        self.card_agent_tag(ticket).into_iter().chain(lean).collect()
    }

    /// A tag on the card when the ticket runs a different agent from the board's default.
    fn card_agent_tag(&self, ticket: &Ticket) -> Option<AnyElement> {
        let kind = ticket.session(&ticket.column).map(|r| r.agent).unwrap_or_else(|| self.board.config.ticket_agent(ticket));
        (kind != self.board.config.defaults.agent).then(|| Tag::secondary().outline().xsmall().child(agent::get(kind).display_name()).into_any_element())
    }

    fn choose_ticket_lean(&mut self, ticket_id: &str, lean: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.board.choose_lean(ticket_id, lean) {
            self.notify_error(&err, window, cx);
        }
        self.reload_tickets(window, cx);
        cx.notify();
    }

    fn choose_ticket_agent(&mut self, ticket_id: &str, kind: AgentKind, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.board.choose_agent(ticket_id, kind) {
            self.notify_error(&err, window, cx);
        }
        self.reload_tickets(window, cx);
        cx.notify();
    }
}
