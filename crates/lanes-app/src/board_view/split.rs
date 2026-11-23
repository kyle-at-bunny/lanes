use gpui_kit::component::resizable::{ResizablePanelEvent, ResizableState, h_resizable, resizable_panel};
use gpui_kit::*;
use lanes_core::ui_prefs::UiPrefs;

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
        let prefs = UiPrefs { panel_width: Some(width.round()) };
        if prefs != self.ui_prefs && prefs.save(&self.board.home).is_ok() {
            self.ui_prefs = prefs;
        }
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
        let default_width = f32::from(window.viewport_size().width) * 0.5;
        let width = self.ui_prefs.panel_width.unwrap_or(default_width).max(DETAIL_MIN_WIDTH);
        let has_panel = panel.is_some();
        h_resizable("board-split")
            .with_state(&self.split)
            .child(resizable_panel().size_range(px(BOARD_MIN_WIDTH)..Pixels::MAX).child(board))
            .child(
                resizable_panel()
                    .visible(has_panel)
                    .size(px(width))
                    .size_range(px(DETAIL_MIN_WIDTH)..Pixels::MAX)
                    .child(panel.unwrap_or_else(|| div().into_any_element())),
            )
    }
}
