use anyhow::Result;

use crate::icon_art;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const ICON_SIZE: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    Show,
    Quit,
}

/// The notification-area icon: left click or "Show Lanes" restores the window, "Quit" exits for real.
pub struct Tray {
    icon: TrayIcon,
    show_id: MenuId,
    quit_id: MenuId,
    waiting: usize,
}

impl Tray {
    pub fn new() -> Result<Self> {
        let show = MenuItem::new("Show Lanes", true, None);
        let quit = MenuItem::new("Quit Lanes", true, None);
        let menu = Menu::new();
        menu.append(&show)?;
        menu.append(&PredefinedMenuItem::separator())?;
        menu.append(&quit)?;
        let icon = TrayIconBuilder::new()
            .with_icon(render_icon(false)?)
            .with_tooltip("Lanes")
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .build()?;
        Ok(Self { icon, show_id: show.id().clone(), quit_id: quit.id().clone(), waiting: 0 })
    }

    pub fn poll(&self) -> Vec<TrayCommand> {
        let mut commands = Vec::new();
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            let left_click = matches!(
                event,
                TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                    | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. }
            );
            if left_click {
                commands.push(TrayCommand::Show);
            }
        }
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == self.show_id {
                commands.push(TrayCommand::Show);
            } else if event.id == self.quit_id {
                commands.push(TrayCommand::Quit);
            }
        }
        commands
    }

    /// Shows a badge and a tooltip count while sessions are waiting on the user.
    pub fn set_waiting(&mut self, waiting: usize) {
        if waiting == self.waiting {
            return;
        }
        let tooltip = match waiting {
            0 => "Lanes".to_string(),
            1 => "Lanes: 1 session waiting for you".to_string(),
            n => format!("Lanes: {n} sessions waiting for you"),
        };
        if let Ok(icon) = render_icon(waiting > 0) {
            let _ = self.icon.set_icon(Some(icon));
        }
        let _ = self.icon.set_tooltip(Some(tooltip));
        self.waiting = waiting;
    }
}

fn render_icon(attention: bool) -> Result<Icon> {
    Ok(Icon::from_rgba(icon_art::rgba(ICON_SIZE, attention), ICON_SIZE, ICON_SIZE)?)
}
