//! macOS and Linux. Best effort: compiled in CI, not exercised by tests.

use gpui_kit::{App, Window};

pub fn attach_parent_console() {}

pub fn show_startup_error(message: &str) {
    eprintln!("{message}");
}

/// macOS hides the whole app (it stays in the Dock and menu bar); Linux has no tray, so this only minimises.
pub fn set_window_shown(window: &Window, cx: &App, shown: bool) {
    let mac = cfg!(target_os = "macos");
    match shown {
        true if mac => cx.activate(true),
        true => {}
        false if mac => cx.hide(),
        false => window.minimize_window(),
    }
}
