use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::cli::format_duration;
use lanes_core::paths::now_unix;
use lanes_core::sessions::{self, Owner, SessionInfo};
use uuid::Uuid;

use super::LanesApp;

impl LanesApp {
    pub(super) fn toggle_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_sessions = !self.show_sessions;
        if self.show_sessions {
            self.show_archive = false;
            self.reload_sessions(window, cx);
        }
        cx.notify();
    }

    fn reload_sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match sessions::inventory(&self.board.home) {
            Ok(list) => self.sessions = list,
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    fn session_running(&self, id: &Uuid, cx: &App) -> bool {
        let ticket_live = self.terminals.get(id).is_some_and(|t| !t.read(cx).session.has_exited());
        let assistant_live = self.assistant.as_ref().is_some_and(|t| t.read(cx).session_id == *id && !t.read(cx).session.has_exited());
        ticket_live || assistant_live
    }

    fn stop_session(&mut self, id: Uuid, cx: &mut Context<Self>) {
        if self.assistant.as_ref().is_some_and(|t| t.read(cx).session_id == id) {
            self.stop_assistant(cx);
        }
        self.suspend_session(id, cx);
    }

    /// Stops the session and drops Lanes' link and state for it; Claude Code's own copy is untouched.
    fn forget_sessions(&mut self, ids: Vec<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let count = ids.len();
        for id in ids {
            self.stop_session(id, cx);
            if let Err(err) = sessions::forget(&self.board.home, &id) {
                self.notify_error(&err, window, cx);
            }
        }
        window.push_notification(Notification::info(format!("Forgot {count} session{}", if count == 1 { "" } else { "s" })), cx);
        self.reload_tickets(window, cx);
        self.reload_sessions(window, cx);
        cx.notify();
    }

    pub(super) fn request_delete_ticket(&mut self, ticket_id: &str, cx: &mut Context<Self>) {
        self.delete_confirm = Some(ticket_id.to_string());
        cx.notify();
    }

    fn confirm_delete_ticket(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket_id) = self.delete_confirm.take() else { return };
        let live: Vec<Uuid> = self.sessions_of(&ticket_id);
        for id in live {
            self.suspend_session(id, cx);
        }
        match sessions::delete_ticket(&self.board.home, &ticket_id) {
            Ok(report) => {
                let message = format!("Deleted {ticket_id} and Lanes' records of its {} session(s)", report.sessions.len());
                window.push_notification(Notification::success(message), cx);
            }
            Err(err) => self.notify_error(&err, window, cx),
        }
        if self.selected.as_deref() == Some(ticket_id.as_str()) {
            self.selected = None;
        }
        self.reload_tickets(window, cx);
        if let Ok(archived) = self.board.tickets.list_archived() {
            self.archived = archived;
        }
        cx.notify();
    }

    fn sessions_of(&self, ticket_id: &str) -> Vec<Uuid> {
        self.any_ticket(ticket_id).map(|t| t.sessions.values().map(|r| r.id).collect()).unwrap_or_default()
    }

    fn any_ticket(&self, ticket_id: &str) -> Option<&lanes_core::ticket::Ticket> {
        self.ticket(ticket_id).or_else(|| self.archived.iter().find(|t| t.id == ticket_id))
    }

    pub(super) fn request_assistant_reset(&mut self, cx: &mut Context<Self>) {
        self.assistant_reset_confirm = true;
        cx.notify();
    }

    fn confirm_assistant_reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.assistant_reset_confirm = false;
        self.stop_assistant(cx);
        match sessions::reset_assistant(&self.board.home) {
            Ok(_) => window.push_notification(Notification::success("Assistant reset; it starts a fresh conversation"), cx),
            Err(err) => self.notify_error(&err, window, cx),
        }
        cx.notify();
    }

    pub(super) fn render_delete_confirm(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let ticket_id = self.delete_confirm.as_ref()?;
        let sessions = self.sessions_of(ticket_id).len();
        let agents = lanes_core::agent::names(self.any_ticket(ticket_id).into_iter().flat_map(|t| t.agents()));
        Some(
            h_flex()
                .flex_wrap()
                .px_3()
                .py_2()
                .gap_3()
                .bg(cx.theme().danger.opacity(0.15))
                .child(div().flex_1().min_w_0().child(format!(
                    "Delete {ticket_id}? This stops its sessions, and permanently removes the ticket, its artifacts and timeline, and Lanes' records of its {sessions} session(s). {agents} keeps its own copy of the conversations."
                )))
                .child(Button::new("confirm-delete").label("Delete ticket").danger().small().on_click(cx.listener(|this, _, window, cx| {
                    this.confirm_delete_ticket(window, cx)
                })))
                .child(Button::new("cancel-delete").label("Cancel").ghost().small().on_click(cx.listener(|this, _, _, cx| {
                    this.delete_confirm = None;
                    cx.notify();
                }))),
        )
    }

    pub(super) fn render_assistant_reset_confirm(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if !self.assistant_reset_confirm {
            return None;
        }
        Some(
            h_flex()
                .flex_wrap()
                .gap_2()
                .p_2()
                .rounded_md()
                .bg(cx.theme().warning.opacity(0.15))
                .text_xs()
                .child(div().flex_1().min_w_0().child("Reset the assistant? Its current conversation is stopped and dropped from Lanes; the next one starts fresh."))
                .child(
                    Button::new("assistant-reset-confirm")
                        .label("Reset")
                        .warning()
                        .xsmall()
                        .on_click(cx.listener(|this, _, window, cx| this.confirm_assistant_reset(window, cx))),
                )
                .child(Button::new("assistant-reset-cancel").label("Cancel").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| {
                    this.assistant_reset_confirm = false;
                    cx.notify();
                }))),
        )
    }

    pub(super) fn render_sessions(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let unlinked: Vec<Uuid> = self.sessions.iter().filter(|s| s.owner == Owner::Unlinked).map(|s| s.id).collect();
        let rows: Vec<AnyElement> = self.sessions.iter().map(|s| self.render_session_row(s, cx)).collect();
        let empty = rows.is_empty();
        v_flex()
            .id("sessions")
            .flex_1()
            .min_w_0()
            .h_full()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_2()
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Agent sessions created by Lanes"))
                    .child(div().flex_1())
                    .child(Button::new("sessions-refresh").label("Refresh").ghost().small().on_click(cx.listener(|this, _, window, cx| {
                        this.reload_sessions(window, cx);
                        cx.notify();
                    })))
                    .when(!unlinked.is_empty(), |this| {
                        let count = unlinked.len();
                        this.child(
                            Button::new("forget-unlinked")
                                .label(format!("Forget all unlinked ({count})"))
                                .outline()
                                .small()
                                .on_click(cx.listener(move |this, _, window, cx| this.forget_sessions(unlinked.clone(), window, cx))),
                        )
                    }),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(retention_note(self.board.config.defaults.agent)))
            .when(empty, |this| this.child(div().text_sm().text_color(theme.muted_foreground).child("No sessions yet.")))
            .children(rows)
    }

    fn render_session_row(&self, session: &SessionInfo, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let running = self.session_running(&session.id, cx);
        let owner = match &session.owner {
            Owner::Ticket { ticket_id, column, archived } => {
                format!("{ticket_id} · {}{}", self.board.column_title(column), if *archived { " (archived)" } else { "" })
            }
            Owner::Assistant => "Board assistant".into(),
            Owner::Unlinked => "Unlinked".into(),
        };
        let state = match (running, session.status) {
            (true, Some(status)) => Tag::primary().xsmall().child(format!("running · {}", status.label())),
            (true, None) => Tag::primary().xsmall().child("running"),
            (false, _) => Tag::secondary().xsmall().child("not running"),
        };
        let ago = if session.last_active == 0 { "—".to_string() } else { format!("{} ago", format_duration(now_unix().saturating_sub(session.last_active))) };
        let size = match session.transcript_bytes {
            0 => "no transcript".to_string(),
            bytes if bytes < 1024 * 1024 => format!("{} KB", bytes.div_ceil(1024)),
            bytes => format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0)),
        };
        let (stop_id, forget_id) = (session.id, session.id);
        h_flex()
            .flex_wrap()
            .gap_x_3()
            .gap_y_1()
            .px_3()
            .py_2()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .text_xs()
            .child(div().w(px(220.)).flex_none().font_weight(FontWeight::SEMIBOLD).child(owner))
            .child(state)
            .child(
                div()
                    .flex_1()
                    .min_w(px(160.))
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(session.cwd.as_ref().map(|p| p.display().to_string()).unwrap_or_default()),
            )
            .child(div().w(px(80.)).flex_none().text_color(theme.muted_foreground).child(ago))
            .child(div().w(px(90.)).flex_none().text_color(theme.muted_foreground).child(size))
            .when(running, |this| {
                this.child(Button::new(SharedString::from(format!("stop-{}", session.id))).label("Stop").ghost().xsmall().on_click(cx.listener(
                    move |this, _, window, cx| {
                        this.stop_session(stop_id, cx);
                        this.reload_sessions(window, cx);
                        cx.notify();
                    },
                )))
            })
            .child(
                Button::new(SharedString::from(format!("forget-{}", session.id)))
                    .label("Forget")
                    .outline()
                    .xsmall()
                    .on_click(cx.listener(move |this, _, window, cx| this.forget_sessions(vec![forget_id], window, cx))),
            )
            .into_any_element()
    }
}

fn retention_note(kind: lanes_core::agent::AgentKind) -> String {
    let agent = lanes_core::agent::get(kind);
    let keeps = match agent.retention_days() {
        Some(days) => format!("{} keeps its own copy of each conversation and deletes it after {days} days.", agent.display_name()),
        None => format!("{} keeps its own copy of each conversation.", agent.display_name()),
    };
    format!("Forget stops a session and removes Lanes' link to it; a ticket's stage then starts fresh next time. {keeps}")
}
