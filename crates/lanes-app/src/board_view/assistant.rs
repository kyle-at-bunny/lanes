use std::time::SystemTime;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::assistant;
use lanes_core::config::TerminalMode;

use super::LanesApp;
use crate::pty::{GridSize, PtySession};
use crate::terminal_view::TerminalView;

impl LanesApp {
    pub(super) fn board_file_mtime(&self) -> Option<SystemTime> {
        std::fs::metadata(self.board.home.board_file()).and_then(|m| m.modified()).ok()
    }

    /// Picks up board.toml edits (by hand or by the assistant); an invalid file keeps the previous config.
    pub(super) fn reload_config_if_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mtime = self.board_file_mtime();
        if mtime == self.config_mtime {
            return;
        }
        self.config_mtime = mtime;
        match self.board.reload_config() {
            Ok(()) => {
                self.apply_theme(window, cx);
                window.push_notification(Notification::success("board.toml changed: board reloaded"), cx);
            }
            Err(err) => {
                let message = format!("board.toml has a problem, so Lanes kept the previous config:\n{err:#}");
                window.push_notification(Notification::error(message), cx);
            }
        }
        cx.notify();
    }

    pub(super) fn toggle_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_assistant = !self.show_assistant;
        if self.show_assistant {
            self.ensure_assistant(window, cx);
        }
        cx.notify();
    }

    fn ensure_assistant(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.assistant.as_ref().filter(|t| !t.read(cx).session.has_exited()) {
            let handle = terminal.read(cx).focus_handle(cx);
            return handle.focus(window, cx);
        }
        let spec = match assistant::prepare(&self.board.home, &self.board.config, &self.board.lanes_exe) {
            Ok(spec) => spec,
            Err(err) => return self.notify_error(&err, window, cx),
        };
        if self.board.config.defaults.terminal == TerminalMode::External {
            self.show_assistant = false;
            if let Err(err) = self.spawn_external(&spec) {
                self.notify_error(&err, window, cx);
            }
            return;
        }
        match PtySession::spawn(&spec, GridSize { columns: 100, rows: 40 }) {
            Ok(session) => {
                let terminal = cx.new(|cx| TerminalView::new(spec.session_id, session, cx));
                let handle = terminal.read(cx).focus_handle(cx);
                handle.focus(window, cx);
                self.assistant = Some(terminal);
            }
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    pub(super) fn stop_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(terminal) = self.assistant.take() {
            terminal.read(cx).session.kill();
        }
    }

    pub(super) fn render_assistant(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let exited = self.assistant.as_ref().is_some_and(|t| t.read(cx).session.has_exited());
        let header = v_flex()
            .p_3()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Board assistant"))
                    .child(div().flex_1())
                    .child(
                        Button::new("assistant-reset")
                            .label("Reset")
                            .ghost()
                            .xsmall()
                            .tooltip("Drop this conversation and start the assistant fresh")
                            .on_click(cx.listener(|this, _, _, cx| this.request_assistant_reset(cx))),
                    )
                    .child(Button::new("assistant-hide").label("Hide").ghost().xsmall().on_click(cx.listener(|this, _, window, cx| {
                        this.toggle_assistant(window, cx)
                    }))),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                "Ask it to create, move or edit tickets, add or change columns, or rewrite a column's prompt. Board edits reload automatically.",
            ))
            .when_some(self.render_assistant_reset_confirm(cx), |this, confirm| this.child(confirm));
        let body = match (&self.assistant, exited) {
            (Some(terminal), false) => div().flex_1().min_h_0().p_1().child(terminal.clone()).into_any_element(),
            _ => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(Button::new("assistant-start").label("Start assistant").primary().small().on_click(cx.listener(|this, _, window, cx| {
                    this.ensure_assistant(window, cx);
                    cx.notify();
                })))
                .into_any_element(),
        };
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(theme.border)
            .child(header)
            .child(body)
    }
}
