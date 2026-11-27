use gpui_kit::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Subcommands are run by the agent CLI, so they need the console the GUI subsystem doesn't get by default.
pub fn attach_parent_console() {
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

/// Errors before the window exists would otherwise vanish, since the GUI build has no console.
pub fn show_startup_error(message: &str) {
    unsafe extern "system" {
        fn MessageBoxW(hwnd: isize, text: *const u16, caption: *const u16, kind: u32) -> i32;
    }
    const MB_ICONERROR: u32 = 0x10;
    let wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (text, caption) = (wide(message), wide("Lanes"));
    unsafe {
        MessageBoxW(0, text.as_ptr(), caption.as_ptr(), MB_ICONERROR);
    }
}

/// Hides the window completely (no taskbar button), which GPUI has no API for on Windows.
pub fn set_window_shown(window: &Window, shown: bool) {
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
