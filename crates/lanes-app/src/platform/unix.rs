//! macOS and Linux. Best effort: compiled in CI, not exercised by tests.

use gpui_kit::Window;

pub fn attach_parent_console() {}

pub fn show_startup_error(message: &str) {
    eprintln!("{message}");
}

pub fn set_window_shown(window: &Window, shown: bool) {
    if !shown {
        window.minimize_window();
    }
}
