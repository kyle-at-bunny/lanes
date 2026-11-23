use std::time::Duration;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::*;

use super::{CardSession, LanesApp};
use crate::tray::{Tray, TrayCommand};
use crate::window_visibility;
use lanes_core::state::AgentStatus;

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
        window_visibility::hide(window);
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
                TrayCommand::Show => window_visibility::show(window),
                TrayCommand::Quit => self.request_quit(window, cx),
            }
        }
    }

    pub(super) fn request_quit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let running = self.running_sessions(cx);
        if running == 0 {
            return self.quit_now(cx);
        }
        window_visibility::show(window);
        self.quit_confirm = Some(running);
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

    fn running_sessions(&self, cx: &App) -> usize {
        let tickets = self.terminals.values().filter(|t| !t.read(cx).session.has_exited()).count();
        tickets + usize::from(self.assistant_running(cx))
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
        Some(
            h_flex()
                .px_3()
                .py_2()
                .gap_3()
                .bg(cx.theme().danger.opacity(0.15))
                .child(div().flex_1().min_w_0().child(format!("{running} {noun} still running. Quitting stops them; you can resume them later from their cards.")))
                .child(Button::new("quit-anyway").label("Quit anyway").danger().small().on_click(cx.listener(|this, _, _, cx| this.quit_now(cx))))
                .child(Button::new("cancel-quit").label("Cancel").ghost().small().on_click(cx.listener(|this, _, _, cx| {
                    this.quit_confirm = None;
                    cx.notify();
                }))),
        )
    }
}
