use std::time::Duration;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::*;

use super::{CardSession, LanesApp};
use crate::terminal_view::TerminalView;
use crate::tray::{Tray, TrayCommand};
use crate::window_visibility;
use lanes_core::state::AgentStatus;

const ACTIVE_OUTPUT_WINDOW: Duration = Duration::from_secs(10);

impl LanesApp {
    /// Closing the window hides it to the tray so sessions keep running; only Quit ends them.
    pub(super) fn install_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match Tray::new() {
            Ok(tray) => self.tray = Some(tray),
            Err(err) => self.notify_error(&err.context("tray icon unavailable; closing the window will quit"), window, cx),
        }
        let this = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| this.update(cx, |app, cx| app.close_requested(window, cx)).unwrap_or(true));
        self.start_tray_loop(cx);
    }

    fn close_requested(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.quitting || self.tray.is_none() {
            self.stop_all_sessions(cx);
            return true;
        }
        window_visibility::hide(window, cx);
        false
    }

    fn start_tray_loop(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_millis(150)).await;
                if this.update_in(cx, |app, window, cx| app.poll_tray(window, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn poll_tray(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let commands = self.tray.as_ref().map(Tray::poll).unwrap_or_default();
        for command in commands {
            match command {
                TrayCommand::Show => window_visibility::show(window, cx),
                TrayCommand::Quit => self.request_quit(window, cx),
            }
        }
    }

    /// Quits straight away unless Claude is mid-turn somewhere; idle and waiting sessions are just stopped (they stay resumable).
    pub(super) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let working = self.working_sessions(cx);
        if working == 0 {
            return self.quit_now(cx);
        }
        window_visibility::show(window, cx);
        self.quit_confirm = Some(working);
        cx.notify();
    }

    fn quit_now(&mut self, cx: &mut Context<Self>) {
        self.quitting = true;
        self.stop_all_sessions(cx);
        cx.quit();
    }

    fn stop_all_sessions(&mut self, cx: &mut Context<Self>) {
        for (_, terminal) in self.terminals.drain() {
            terminal.read(cx).session.kill();
        }
        self.stop_assistant(cx);
    }

    fn working_sessions(&self, cx: &App) -> usize {
        self.working_agents(cx).len()
    }

    /// The agent of each session that is mid-turn.
    fn working_agents(&self, cx: &App) -> Vec<lanes_core::agent::AgentKind> {
        let assistant = self.assistant.iter().filter(|t| self.is_working(t.read(cx))).map(|_| self.assistant_agent());
        let tickets = self.terminals.values().filter(|t| self.is_working(t.read(cx))).filter_map(|t| self.session_agent(&t.read(cx).session_id));
        tickets.chain(assistant).collect()
    }

    fn session_agent(&self, session_id: &uuid::Uuid) -> Option<lanes_core::agent::AgentKind> {
        self.tickets.iter().flat_map(|t| t.sessions.values()).find(|r| r.id == *session_id).map(|r| r.agent)
    }

    /// Mid-turn: the hooks say working AND the terminal is still redrawing, which filters out a stale
    /// "working" left behind when a turn is interrupted without a Stop hook.
    fn is_working(&self, view: &TerminalView) -> bool {
        let hook_working = self.board.states.hook(&view.session_id).is_some_and(|h| h.status == AgentStatus::Working);
        !view.session.has_exited() && hook_working && view.session.quiet_for() < ACTIVE_OUTPUT_WINDOW
    }

    pub(super) fn update_tray_badge(&mut self, cx: &App) {
        let waiting = self.tickets.iter().filter(|t| matches!(self.card_session(t, cx), CardSession::Live(Some(AgentStatus::NeedsInput)))).count();
        if let Some(tray) = self.tray.as_mut() {
            tray.set_waiting(waiting);
        }
    }

    pub(super) fn render_quit_confirm(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let running = self.quit_confirm?;
        let noun = if running == 1 { "session is" } else { "sessions are" };
        let agents = lanes_core::agent::names(self.working_agents(cx));
        let message = format!("{agents} is working in {running} {noun}. Quitting interrupts it; you can resume from the ticket's card later.");
        Some(
            h_flex()
                .px_3()
                .py_2()
                .gap_3()
                .bg(cx.theme().danger.opacity(0.15))
                .child(div().flex_1().min_w_0().child(message))
                .child(Button::new("quit-anyway").label("Quit anyway").danger().small().on_click(cx.listener(|this, _, _, cx| this.quit_now(cx))))
                .child(Button::new("cancel-quit").label("Cancel").ghost().small().on_click(cx.listener(|this, _, _, cx| {
                    this.quit_confirm = None;
                    cx.notify();
                }))),
        )
    }
}
