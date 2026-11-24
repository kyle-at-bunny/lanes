use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::ticket::Ticket;

use super::LanesApp;

const DESCRIPTION_MAX_HEIGHT: f32 = 160.;

pub(super) struct DescriptionEdit {
    ticket_id: String,
    input: Entity<TextareaState>,
}

impl LanesApp {
    fn start_description_edit(&mut self, ticket_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.ticket(ticket_id).map(|t| t.description.clone()).unwrap_or_default();
        let input = cx.new(|cx| TextareaState::new(window, cx).rows(8).placeholder("Description / context for the sessions").default_value(current));
        input.update(cx, |state, cx| state.focus(window, cx));
        self.description_edit = Some(DescriptionEdit { ticket_id: ticket_id.to_string(), input });
        cx.notify();
    }

    fn save_description_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(edit) = self.description_edit.take() else { return };
        let value = edit.input.read(cx).value().to_string();
        match self.board.tickets.set_description(&edit.ticket_id, &value) {
            Ok(updated) => self.replace_ticket(updated),
            Err(err) => self.notify_error(&err, window, cx),
        }
        cx.notify();
    }

    fn replace_ticket(&mut self, updated: Ticket) {
        if let Some(slot) = self.tickets.iter_mut().find(|t| t.id == updated.id) {
            *slot = updated;
        }
    }

    /// The ticket description in full (scrolling when long), or its editor.
    pub(super) fn render_description(&self, ticket: &Ticket, cx: &mut Context<Self>) -> AnyElement {
        match self.description_edit.as_ref().filter(|e| e.ticket_id == ticket.id) {
            Some(edit) => self.render_description_editor(edit, cx).into_any_element(),
            None => self.render_description_text(ticket, cx).into_any_element(),
        }
    }

    fn render_description_text(&self, ticket: &Ticket, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let empty = ticket.description.is_empty();
        let ticket_id = ticket.id.clone();
        let edit = Button::new("edit-description")
            .label(if empty { "Add description" } else { "Edit description" })
            .link()
            .xsmall()
            .on_click(cx.listener(move |this, _, window, cx| this.start_description_edit(&ticket_id, window, cx)));
        v_flex()
            .gap_0p5()
            .when(!empty, |this| {
                this.child(
                    div()
                        .id("ticket-description")
                        .max_h(px(DESCRIPTION_MAX_HEIGHT))
                        .overflow_y_scroll()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(ticket.description.clone()),
                )
            })
            .child(h_flex().child(edit))
    }

    fn render_description_editor(&self, edit: &DescriptionEdit, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().gap_1().child(Textarea::new(&edit.input)).child(
            h_flex()
                .gap_1()
                .child(
                    Button::new("save-description").label("Save").primary().xsmall().on_click(cx.listener(|this, _, window, cx| this.save_description_edit(window, cx))),
                )
                .child(Button::new("cancel-description").label("Cancel").ghost().xsmall().on_click(cx.listener(|this, _, _, cx| {
                    this.description_edit = None;
                    cx.notify();
                }))),
        )
    }
}
