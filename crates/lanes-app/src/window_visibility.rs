use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Hides the window completely (no taskbar button), which GPUI has no API for on Windows.
pub fn hide(window: &Window) {
    set_shown(window, false);
}

pub fn show(window: &Window) {
    set_shown(window, true);
    window.activate_window();
}

#[cfg(windows)]
fn set_shown(window: &Window, shown: bool) {
    unsafe extern "system" {
        fn ShowWindow(hwnd: isize, command: i32) -> i32;
    }
    const SW_HIDE: i32 = 0;
    const SW_RESTORE: i32 = 9;
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    if let RawWindowHandle::Win32(win32) = handle.as_raw() {
        unsafe {
            ShowWindow(win32.hwnd.get(), if shown { SW_RESTORE } else { SW_HIDE });
        }
    }
}

#[cfg(not(windows))]
fn set_shown(window: &Window, shown: bool) {
    if !shown {
        window.minimize_window();
    }
}
