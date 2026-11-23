#![cfg_attr(not(test), windows_subsystem = "windows")]

extern crate gpui_kit as gpui;

mod board_view;
mod keys;
mod palette;
mod pty;
mod terminal_view;
mod tray;
mod window_visibility;

use anyhow::Result;
use clap::Parser;
use gpui_kit::*;
use lanes_core::board::Board;
use lanes_core::cli::{Cli, Command, run};

use crate::board_view::LanesApp;

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.command {
        Some(command) => run_subcommand(&cli, command),
        None => run_gui(&cli),
    }
}

/// Subcommands are invoked by claude itself, so they need the console the GUI subsystem doesn't get by default.
fn run_subcommand(cli: &Cli, command: &Command) -> Result<()> {
    attach_parent_console();
    run(cli, command)
}

fn run_gui(cli: &Cli) -> Result<()> {
    let board = Board::open(cli.lanes_home()?, std::env::current_exe()?)?;
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        bind_terminal_keys(cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(None, size(px(1500.), px(950.)), cx))),
            titlebar: Some(TitlebarOptions { title: Some("Lanes".into()), ..Default::default() }),
            window_min_size: Some(size(px(800.), px(500.))),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| LanesApp::new(board, window, cx))).expect("failed to open the Lanes window");
    });
    Ok(())
}

/// Lets Tab, Shift+Tab and Ctrl+C reach claude instead of the app's focus and copy bindings.
fn bind_terminal_keys(cx: &mut App) {
    let context = Some(terminal_view::KEY_CONTEXT);
    cx.bind_keys(["tab", "shift-tab", "ctrl-c", "escape", "enter"].map(|key| KeyBinding::new(key, NoAction {}, context)));
}

#[cfg(windows)]
fn attach_parent_console() {
    unsafe extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}
