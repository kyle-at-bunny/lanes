use gpui_kit::component::resizable::{HandleEdge, resize_handle};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use lanes_core::ui_prefs::{UiPrefs, clamp_header_height};

use super::LanesApp;

/// The payload of a drag on the divider between the ticket info and the session tabs.
#[derive(Clone)]
pub(super) struct HeaderDrag;

impl Render for HeaderDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

impl LanesApp {
    /// The ticket info, at the height the user dragged it to (scrolling if it no longer fits), with the divider on its bottom edge.
    pub(super) fn render_detail_header(&self, header: Div, cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.ui_prefs.detail_header_height;
        div()
            .relative()
            .flex_none()
            .child(header.id("detail-header").when_some(height, |this, height| this.h(px(height)).overflow_y_scroll()))
            .child(self.header_divider(cx))
    }

    /// Dragging moves the divider; double-clicking it lets the ticket info fit its content again.
    fn header_divider(&self, cx: &mut Context<Self>) -> impl IntoElement {
        resize_handle::<HeaderDrag, HeaderDrag>("detail-header-divider", Axis::Vertical)
            .inside(HandleEdge::Trailing)
            .on_drag(HeaderDrag, |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.set_detail_header_height(None, cx);
                }
            }))
    }

    /// Follows a divider drag over the whole ticket panel, whose top edge is where the ticket info starts.
    pub(super) fn on_header_drag(&mut self, event: &DragMoveEvent<HeaderDrag>, cx: &mut Context<Self>) {
        let bounds = event.bounds;
        let height = clamp_header_height(f32::from(event.event.position.y), f32::from(bounds.top()), f32::from(bounds.size.height));
        if self.ui_prefs.detail_header_height != Some(height) {
            self.set_detail_header_height(Some(height), cx);
        }
    }

    fn set_detail_header_height(&mut self, height: Option<f32>, cx: &mut Context<Self>) {
        let prefs = UiPrefs { detail_header_height: height, ..self.ui_prefs.clone() };
        // Saved on every step of the drag; ui.toml is tiny and written atomically.
        let _ = prefs.save(&self.board.home);
        self.ui_prefs = prefs;
        cx.notify();
    }
}
