use anyhow::Result;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::cleanup::{self, CleanupPlan, CleanupReport};

use super::LanesApp;

impl LanesApp {
    pub(super) fn is_last_column(&self, column_id: &str) -> bool {
        self.board.config.columns.last().is_some_and(|c| c.id == column_id)
    }

    /// Works out what cleanup would do and asks the user to confirm it.
    pub(super) fn request_cleanup(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.ticket(ticket_id) else { return };
        match cleanup::plan(&self.board.home, &self.board.config, ticket) {
            Ok(plan) => self.cleanup_confirm = Some(plan),
            Err(err) => self.notify_error(&err, window, cx),
        }
        cx.notify();
    }

    fn confirm_cleanup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(plan) = self.cleanup_confirm.take() else { return };
        for session_id in &plan.session_ids {
            self.suspend_session(*session_id, cx);
        }
        if self.selected.as_deref() == Some(plan.ticket_id.as_str()) {
            self.selected = None;
        }
        let home = self.board.home.clone();
        let background = cx.background_executor().clone();
        let ticket_id = plan.ticket_id.clone();
        cx.spawn_in(window, async move |this, cx| {
            let report = background.spawn(async move { cleanup::execute(&home, &plan) }).await;
            let _ = this.update_in(cx, |app, window, cx| app.report_cleanup(&ticket_id, report, window, cx));
        })
        .detach();
        cx.notify();
    }

    fn report_cleanup(&mut self, ticket_id: &str, report: Result<CleanupReport>, window: &mut Window, cx: &mut Context<Self>) {
        let report = match report {
            Ok(report) => report,
            Err(err) => return self.notify_error(&err, window, cx),
        };
        let failures: Vec<String> = report.failures().map(|(label, outcome)| format!("{label} failed:\n{}", outcome.output)).collect();
        let notification = if failures.is_empty() {
            Notification::success(format!("{ticket_id} finished and archived ({})", plural(report.removed_files, "file", "files") + " removed"))
        } else {
            Notification::warning(format!("{ticket_id} archived, but:\n{}", failures.join("\n")))
        };
        window.push_notification(notification, cx);
        self.reload_tickets(window, cx);
        self.reload_archive(window, cx);
        cx.notify();
    }

    pub(super) fn toggle_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_archive = !self.show_archive;
        if self.show_archive {
            self.show_sessions = false;
            self.reload_archive(window, cx);
        }
        cx.notify();
    }

    fn reload_archive(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.board.tickets.list_archived() {
            Ok(archived) => self.archived = archived,
            Err(err) => self.notify_error(&err, window, cx),
        }
    }

    fn restore_ticket(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = self.board.tickets.restore(ticket_id) {
            return self.notify_error(&err, window, cx);
        }
        self.reload_archive(window, cx);
        self.reload_tickets(window, cx);
        window.push_notification(Notification::info(format!("{ticket_id} restored to the board")), cx);
        cx.notify();
    }

    pub(super) fn render_cleanup_confirm(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let plan = self.cleanup_confirm.as_ref()?;
        let open_prs = self.open_pr_warnings(&plan.ticket_id).into_iter().map(|line| div().font_weight(FontWeight::SEMIBOLD).child(line).into_any_element());
        Some(
            h_flex()
                .px_3()
                .py_2()
                .gap_3()
                .items_start()
                .bg(cx.theme().warning.opacity(0.15))
                .child(v_flex().flex_1().min_w_0().gap_0p5().text_sm().children(describe_plan(plan)).children(open_prs))
                .child(
                    Button::new("confirm-cleanup")
                        .label("Finish & clean up")
                        .warning()
                        .small()
                        .on_click(cx.listener(|this, _, window, cx| this.confirm_cleanup(window, cx))),
                )
                .child(Button::new("cancel-cleanup").label("Cancel").ghost().small().on_click(cx.listener(|this, _, _, cx| {
                    this.cleanup_confirm = None;
                    cx.notify();
                }))),
        )
    }

    pub(super) fn render_archive(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows: Vec<_> = self
            .archived
            .iter()
            .map(|ticket| {
                let id = ticket.id.clone();
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border)
                    .child(div().w(px(90.)).flex_none().text_xs().text_color(theme.muted_foreground).child(ticket.id.clone()))
                    .child(div().flex_1().min_w_0().text_sm().truncate().child(ticket.title.clone()))
                    .child(div().flex_none().text_xs().text_color(theme.muted_foreground).child(plural(ticket.sessions.len(), "stage session", "stage sessions")))
                    .child(
                        Button::new(SharedString::from(format!("restore-{}", ticket.id)))
                            .label("Restore")
                            .outline()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, window, cx| this.restore_ticket(&id, window, cx))),
                    )
                    .child({
                        let id = ticket.id.clone();
                        Button::new(SharedString::from(format!("delete-{}", ticket.id)))
                            .label("Delete")
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(move |this, _, _, cx| this.request_delete_ticket(&id, cx)))
                    })
                    .into_any_element()
            })
            .collect();
        let empty = rows.is_empty();
        v_flex()
            .id("archive")
            .flex_1()
            .min_w_0()
            .h_full()
            .p_3()
            .gap_2()
            .overflow_y_scroll()
            .child(div().font_weight(FontWeight::SEMIBOLD).child("Archived tickets"))
            .when(empty, |this| {
                this.child(div().text_sm().text_color(theme.muted_foreground).child("Nothing archived yet. Finished tickets land here after \"Finish & clean up\"."))
            })
            .children(rows)
    }
}

fn plural(count: usize, one: &str, many: &str) -> String {
    format!("{count} {}", if count == 1 { one } else { many })
}

fn describe_plan(plan: &CleanupPlan) -> Vec<AnyElement> {
    let mut lines = vec![format!("Finish {}? This will:", plan.ticket_id)];
    let sessions = plural(plan.session_ids.len(), "session", "sessions");
    let files = plural(plan.state_files.len(), "Lanes state file", "Lanes state files");
    lines.push(format!("• stop the ticket's running sessions ({sessions} recorded) and remove {files}"));
    lines.extend(plan.commands.iter().map(|c| format!("• run {}: {}", c.label, c.command)));
    lines.push("• move the ticket to the archive (artifacts and timeline are kept; you can restore it)".into());
    lines.push("• leave Claude Code's own session data alone (it expires under Claude's retention setting)".into());
    lines.into_iter().map(|line| div().child(line).into_any_element()).collect()
}
