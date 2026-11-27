use gpui_kit::Window;

/// Hides the window to the tray; see `platform::set_window_shown` for what that means per OS.
pub fn hide(window: &Window) {
    crate::platform::set_window_shown(window, false);
}

pub fn show(window: &Window) {
    crate::platform::set_window_shown(window, true);
    window.activate_window();
}
