use gpui_kit::{App, Window};

/// Hides the window to the tray; see `platform::set_window_shown` for what that means per OS.
pub fn hide(window: &Window, cx: &App) {
    crate::platform::set_window_shown(window, cx, false);
}

pub fn show(window: &Window, cx: &App) {
    crate::platform::set_window_shown(window, cx, true);
    window.activate_window();
}
