//! Linux: the tray crate needs a GTK event loop GPUI doesn't run, so there's no tray and closing the window quits.

use anyhow::{Result, bail};

pub enum TrayCommand {
    Show,
    Quit,
}

pub struct Tray;

impl Tray {
    pub fn new() -> Result<Self> {
        bail!("the tray icon isn't supported on this platform")
    }

    pub fn poll(&self) -> Vec<TrayCommand> {
        Vec::new()
    }

    pub fn set_waiting(&mut self, _waiting: usize) {}
}
