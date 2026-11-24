use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{ActiveTheme, Sizable, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::artifact_view::display_path;
use lanes_core::ticket::Ticket;
use lanes_core::workdir;

use super::artifacts::PATH_DISPLAY_CHARS;

use super::LanesApp;

pub(super) struct FolderEdit {
    ticket_id: String,
    input: Entity<InputState>,
}

#[derive(Clone, Copy)]
enum Scope {
    AllStages,
    ThisStage,
}

impl LanesApp {
    fn start_folder_edit(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ticket) = self.ticket(ticket_id) else { return };
        let current = ticket.column_cwd.get(&ticket.column).or(ticket.cwd.as_ref()).cloned().unwrap_or_default();
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Folder, e.g. ~/projects/app or ~/worktrees/{{ ticket.id }}").default_value(current));
        input.update(cx, |state, cx| state.focus(window, cx));
        self.folder_edit = Some(FolderEdit { ticket_id: ticket_id.to_string(), input });
        cx.notify();
    }

    fn save_folder(&mut self, scope: Scope, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.folder_edit.take() else { return };
        let folder = edit.input.read(cx).value().to_string();
        let result = self.board.tickets.load(&edit.ticket_id).and_then(|mut ticket| {
            match scope {
                Scope::AllStages => ticket.set_cwd(&folder),
                Scope::ThisStage => ticket.set_column_cwd(&ticket.column.clone(), &folder),
            }
            self.board.tickets.save(&ticket)
        });
        match result {
            Ok(()) => window.push_notification(Notification::success("Folder saved; it applies to sessions started from now on"), cx),
            Err(err) => self.notify_error(&err, window, cx),
        }
        self.reload_tickets(window, cx);
        cx.notify();
    }

    fn clear_folder_overrides(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let result = self.board.tickets.load(ticket_id).and_then(|mut ticket| {
            ticket.set_cwd("");
            ticket.column_cwd.clear();
            self.board.tickets.save(&ticket)
        });
        if let Err(err) = result {
            self.notify_error(&err, window, cx);
        }
        self.folder_edit = None;
        self.reload_tickets(window, cx);
        cx.notify();
    }

    /// Where new sessions for this ticket's current stage will start, and which setting decided it.
    pub(super) fn render_folder_row(&self, ticket: &Ticket, cx: &mut Context<Self>) -> Option<AnyElement> {
        let column = self.board.config.column(&ticket.column)?;
        let source = workdir::source(ticket, column)?;
        let theme = cx.theme().clone();
        if let Some(edit) = self.folder_edit.as_ref().filter(|e| e.ticket_id == ticket.id) {
            return Some(self.render_folder_editor(edit, column.title(), cx).into_any_element());
        }
        let folder = match workdir::resolve(ticket, column) {
            Ok(Some(path)) => path.display().to_string(),
            Ok(None) => return None,
            Err(err) => format!("invalid folder template: {err:#}"),
        };
        let session_elsewhere = ticket.session(&ticket.column).filter(|r| r.cwd.display().to_string() != folder).map(|r| r.cwd.display().to_string());
        let has_overrides = ticket.cwd.is_some() || !ticket.column_cwd.is_empty();
        let (edit_id, clear_id) = (ticket.id.clone(), ticket.id.clone());
        Some(
            v_flex()
                .gap_0p5()
                .pt_1()
                .text_xs()
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .child(div().text_color(theme.muted_foreground).child("New sessions start in"))
                        .child(div().font_family(theme.mono_font_family.clone()).child(display_path(&folder, PATH_DISPLAY_CHARS)))
                        .child(div().text_color(theme.muted_foreground).child(format!("({})", source.describe())))
                        .child(
                            Button::new("change-folder")
                                .label("Change")
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(move |this, _, window, cx| this.start_folder_edit(&edit_id, window, cx))),
                        )
                        .when(has_overrides, |this| {
                            this.child(
                                Button::new("clear-folders")
                                    .label("Use column default")
                                    .ghost()
                                    .xsmall()
                                    .on_click(cx.listener(move |this, _, window, cx| this.clear_folder_overrides(&clear_id, window, cx))),
                            )
                        }),
                )
                .when_some(session_elsewhere, |this, existing| {
                    this.child(div().text_color(theme.warning).child(format!("This stage's existing session stays in {}.", display_path(&existing, PATH_DISPLAY_CHARS))))
                })
                .into_any_element(),
        )
    }

    fn render_folder_editor(&self, edit: &FolderEdit, column_title: &str, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .pt_1()
            .child(Input::new(&edit.input).small())
            .child(
                h_flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("folder-all")
                            .label("All stages")
                            .primary()
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| this.save_folder(Scope::AllStages, window, cx))),
                    )
                    .child(
                        Button::new("folder-stage")
                            .label(format!("{column_title} only"))
                            .outline()
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| this.save_folder(Scope::ThisStage, window, cx))),
                    )
                    .child(Button::new("folder-cancel").label("Cancel").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| {
                        this.folder_edit = None;
                        cx.notify();
                    }))),
            )
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child("Leave empty and save to remove that override. Templates like {{ ticket.id }} work."))
    }
}
