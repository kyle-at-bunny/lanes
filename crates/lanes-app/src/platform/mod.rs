//! Desktop-app behaviour that differs by OS; `lanes_core::platform` covers processes and paths.

use gpui_kit::Modifiers;

#[cfg(not(windows))]
mod unix;
#[cfg(not(windows))]
pub use unix::{attach_parent_console, set_window_shown, show_startup_error};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{attach_parent_console, set_window_shown, show_startup_error};

/// A monospace font that ships with the OS.
pub const TERMINAL_FONT: &str = if cfg!(windows) {
    "Cascadia Mono"
} else if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "DejaVu Sans Mono"
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardKey {
    Copy,
    Paste,
}

/// Terminal copy and paste: Ctrl+Shift+C / Ctrl+V on Windows and Linux (Ctrl+C belongs to the session), Cmd+C / Cmd+V on macOS.
pub fn clipboard_key(modifiers: &Modifiers, key: &str) -> Option<ClipboardKey> {
    if cfg!(target_os = "macos") {
        return match (modifiers.platform, key) {
            (true, "c") => Some(ClipboardKey::Copy),
            (true, "v") => Some(ClipboardKey::Paste),
            _ => None,
        };
    }
    match (modifiers.control, modifiers.shift, key) {
        (true, true, "c") => Some(ClipboardKey::Copy),
        (true, _, "v") => Some(ClipboardKey::Paste),
        _ => None,
    }
}
