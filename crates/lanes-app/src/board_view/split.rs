use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::resizable::{ResizablePanelEvent, ResizableState, h_resizable, resizable_panel};
use gpui_kit::*;
use lanes_core::ui_prefs::{self, UiPrefs};

use super::{BOARD_MIN_WIDTH, DETAIL_MIN_WIDTH, LanesApp};

const RIGHT_PANEL: usize = 1;

impl LanesApp {
    /// The board/panel divider: its state survives re-renders, and every drag is saved to ui.toml.
    pub(super) fn create_split(cx: &mut Context<Self>) -> (Entity<ResizableState>, Subscription) {
        let split = cx.new(|_| ResizableState::default());
        let subscription = cx.subscribe(&split, |app: &mut Self, split, _: &ResizablePanelEvent, cx| {
            let width = split.read(cx).sizes().get(RIGHT_PANEL).map(|w| f32::from(*w));
            app.save_panel_width(width);
        });
        (split, subscription)
    }

    fn save_panel_width(&mut self, width: Option<f32>) {
        let Some(width) = width.filter(|w| *w >= DETAIL_MIN_WIDTH) else { return };
        let prefs = UiPrefs { panel_width: Some(width.round()), ..self.ui_prefs.clone() };
        if prefs != self.ui_prefs && prefs.save(&self.board.home).is_ok() {
            self.ui_prefs = prefs;
        }
    }

    pub(super) fn is_panel_wide(&self, cx: &App) -> bool {
        let (panel, container) = panel_and_container_width(&self.split, cx);
        panel.is_some_and(|panel| ui_prefs::is_panel_wide(panel, container))
    }

    /// Switches the right-hand panel between its wide and narrow layouts.
    pub(super) fn toggle_panel_width(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(panel), container) = panel_and_container_width(&self.split, cx) else { return };
        resize_right_panel(&self.split, ui_prefs::toggled_panel_width(panel, container), window, cx);
    }

    pub(super) fn render_right_panel(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.show_assistant {
            return Some(self.render_assistant(cx).into_any_element());
        }
        let id = self.selected.clone()?;
        Some(self.render_detail(&id, window, cx).into_any_element())
    }

    /// Board on the left; the ticket panel or the assistant on the right, at the width the user last dragged to.
    pub(super) fn render_split(&self, board: AnyElement, panel: Option<AnyElement>, window: &Window) -> impl IntoElement {
        let default_width = f32::from(window.viewport_size().width) * ui_prefs::DEFAULT_PANEL_SHARE;
        let width = self.ui_prefs.panel_width.unwrap_or(default_width).max(DETAIL_MIN_WIDTH);
        let has_panel = panel.is_some();
        let split = h_resizable("board-split").with_state(&self.split).child(resizable_panel().size_range(px(BOARD_MIN_WIDTH)..Pixels::MAX).child(board)).child(
            resizable_panel().visible(has_panel).size(px(width)).size_range(px(DETAIL_MIN_WIDTH)..Pixels::MAX).child(panel.unwrap_or_else(|| div().into_any_element())),
        );
        div().relative().size_full().child(split).child(divider_reset_listener(self.split.clone()))
    }
}

fn panel_and_container_width(split: &Entity<ResizableState>, cx: &App) -> (Option<f32>, f32) {
    let state = split.read(cx);
    (state.sizes().get(RIGHT_PANEL).map(|w| f32::from(*w)), f32::from(state.container_size()))
}

fn resize_right_panel(split: &Entity<ResizableState>, width: f32, window: &mut Window, cx: &mut App) {
    split.update(cx, |state, cx| state.resize_panel(RIGHT_PANEL, px(width), window, cx));
}

/// Double-clicking the divider puts it back in the middle; the library's handle takes no click listeners, so this watches the whole split.
fn divider_reset_listener(split: Entity<ResizableState>) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || event.button != MouseButton::Left || event.click_count != 2 || !bounds.contains(&event.position) {
                    return;
                }
                let (Some(panel), container) = panel_and_container_width(&split, cx) else { return };
                let divider_x = f32::from(bounds.right()) - panel;
                if panel > 0.0 && ui_prefs::is_on_divider(f32::from(event.position.x), divider_x) {
                    resize_right_panel(&split, ui_prefs::default_panel_width(container), window, cx);
                }
            });
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

impl LanesApp {
    pub(super) fn panel_width_button(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (label, tooltip) = if self.is_panel_wide(cx) { ("Narrower", "Give the board more room") } else { ("Wider", "Give this panel more room") };
        Button::new("panel-width").label(label).ghost().xsmall().tooltip(tooltip).on_click(cx.listener(|this, _, window, cx| this.toggle_panel_width(window, cx)))
    }
}
