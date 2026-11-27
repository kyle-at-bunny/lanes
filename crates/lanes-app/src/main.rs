#![cfg_attr(not(test), windows_subsystem = "windows")]

extern crate gpui_kit as gpui;

mod board_view;
mod icon_art;
mod keys;
mod palette;
mod platform;
mod pty;
mod terminal_view;
#[cfg(not(target_os = "linux"))]
mod tray;
#[cfg(target_os = "linux")]
#[path = "tray_unsupported.rs"]
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

fn run_subcommand(cli: &Cli, command: &Command) -> Result<()> {
    platform::attach_parent_console();
    run(cli, command)
}

fn run_gui(cli: &Cli) -> Result<()> {
    let board = match cli.lanes_home().and_then(|home| Board::open(home, std::env::current_exe()?)) {
        Ok(board) => board,
        Err(err) => {
            platform::show_startup_error(&format!("Lanes couldn't open the board:\n\n{err:#}\n\nFix the file and start Lanes again."));
            return Err(err);
        }
    };
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
