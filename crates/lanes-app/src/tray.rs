use anyhow::Result;
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

/// Three white lanes on a blue rounded square, with an orange dot when something needs attention.
fn render_icon(attention: bool) -> Result<Icon> {
    let size = ICON_SIZE as i32;
    let mut rgba = Vec::with_capacity((ICON_SIZE * ICON_SIZE * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            rgba.extend_from_slice(&icon_pixel(x, y, size, attention));
        }
    }
    Ok(Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE)?)
}

fn icon_pixel(x: i32, y: i32, size: i32, attention: bool) -> [u8; 4] {
    const BLUE: [u8; 4] = [0x3b, 0x82, 0xf6, 0xff];
    const WHITE: [u8; 4] = [0xff, 0xff, 0xff, 0xff];
    const ORANGE: [u8; 4] = [0xf9, 0x73, 0x16, 0xff];
    const CLEAR: [u8; 4] = [0, 0, 0, 0];
    let in_dot = |cx: i32, cy: i32, r: i32| (x - cx).pow(2) + (y - cy).pow(2) <= r * r;
    if attention && in_dot(size - 7, 6, 6) {
        return ORANGE;
    }
    if !in_rounded_square(x, y, size, 6) {
        return CLEAR;
    }
    let lane = [8, 15, 22].iter().any(|&left| (left..left + 3).contains(&x));
    if lane && (7..size - 7).contains(&y) { WHITE } else { BLUE }
}

fn in_rounded_square(x: i32, y: i32, size: i32, radius: i32) -> bool {
    let nearest_x = x.clamp(radius, size - 1 - radius);
    let nearest_y = y.clamp(radius, size - 1 - radius);
    (x - nearest_x).pow(2) + (y - nearest_y).pow(2) <= radius * radius
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_has_lanes_corners_and_attention_dot() {
        assert_eq!(icon_pixel(0, 0, 32, false)[3], 0);
        assert_eq!(icon_pixel(9, 16, 32, false), [0xff, 0xff, 0xff, 0xff]);
        assert_eq!(icon_pixel(12, 16, 32, false)[2], 0xf6);
        assert_eq!(icon_pixel(25, 6, 32, true), [0xf9, 0x73, 0x16, 0xff]);
        assert_ne!(icon_pixel(25, 6, 32, false), [0xf9, 0x73, 0x16, 0xff]);
    }
}
