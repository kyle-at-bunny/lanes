use alacritty_terminal::term::color::Colors;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Rgb};
use gpui_kit::component::ActiveTheme;
use gpui_kit::{App, Global, Hsla};
use lanes_core::config::TerminalTheme;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
    ansi: [u32; 16],
}

pub const DARK: Palette = Palette {
    foreground: hex(0xd4d4d4),
    background: hex(0x1a1b1e),
    cursor: hex(0xaeafad),
    ansi: [
        0x1a1b1e, 0xf14c4c, 0x23d18b, 0xe5c07b, 0x3b8eea, 0xd670d6, 0x29b8db, 0xd4d4d4, //
        0x666666, 0xf47171, 0x5cf2ab, 0xf5f543, 0x6aa9f2, 0xe68be6, 0x4fd6f0, 0xffffff,
    ],
};

pub const LIGHT: Palette = Palette {
    foreground: hex(0x383a42),
    background: hex(0xfafafa),
    cursor: hex(0x526fff),
    ansi: [
        0x383a42, 0xe45649, 0x50a14f, 0xc18401, 0x4078f2, 0xa626a4, 0x0184bc, 0xa0a1a7, //
        0x696c77, 0xca1243, 0x3e953a, 0x986801, 0x2b5fd9, 0x8b1d8a, 0x01729e, 0x090a0b,
    ],
};

/// The board's `terminal_theme` setting, shared with every terminal view.
#[derive(Debug, Clone, Copy, Default)]
pub struct TerminalThemeChoice(pub TerminalTheme);

impl Global for TerminalThemeChoice {}

/// The palette terminals should use right now, given the app theme and the terminal_theme setting.
pub fn active(cx: &App) -> &'static Palette {
    let forced_dark = cx.try_global::<TerminalThemeChoice>().is_some_and(|c| c.0 == TerminalTheme::Dark);
    if forced_dark || cx.theme().is_dark() { &DARK } else { &LIGHT }
}

impl Palette {
    pub fn resolve(&self, color: Color, overrides: &Colors, default: Rgb) -> Rgb {
        match color {
            Color::Spec(rgb) => rgb,
            Color::Indexed(index) => overrides[index as usize].unwrap_or_else(|| self.indexed(index)),
            Color::Named(name) => overrides[name].unwrap_or_else(|| self.named(name, default)),
        }
    }

    fn named(&self, name: NamedColor, default: Rgb) -> Rgb {
        match name {
            NamedColor::Foreground | NamedColor::BrightForeground => self.foreground,
            NamedColor::Background => self.background,
            NamedColor::Cursor => self.cursor,
            NamedColor::DimForeground => dim(self.foreground),
            other if (other as usize) < 16 => hex(self.ansi[other as usize]),
            other => {
                let index = other as usize;
                let dim_range = NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize;
                if dim_range.contains(&index) { dim(hex(self.ansi[index - NamedColor::DimBlack as usize])) } else { default }
            }
        }
    }

    fn indexed(&self, index: u8) -> Rgb {
        match index {
            0..=15 => hex(self.ansi[index as usize]),
            16..=231 => {
                let i = index - 16;
                let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
                Rgb { r: level(i / 36), g: level((i / 6) % 6), b: level(i % 6) }
            }
            _ => {
                let gray = 8 + (index - 232) * 10;
                Rgb { r: gray, g: gray, b: gray }
            }
        }
    }
}

pub fn to_hsla(rgb: Rgb) -> Hsla {
    gpui_kit::rgb(((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32).into()
}

const fn hex(value: u32) -> Rgb {
    Rgb { r: (value >> 16) as u8, g: (value >> 8) as u8, b: value as u8 }
}

fn dim(rgb: Rgb) -> Rgb {
    Rgb { r: (rgb.r as u16 * 2 / 3) as u8, g: (rgb.g as u16 * 2 / 3) as u8, b: (rgb.b as u16 * 2 / 3) as u8 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_named_indexed_and_overridden_colors() {
        let colors = Colors::default();
        assert_eq!(DARK.resolve(Color::Named(NamedColor::Background), &colors, DARK.foreground), DARK.background);
        assert_eq!(LIGHT.resolve(Color::Named(NamedColor::Foreground), &colors, LIGHT.background), LIGHT.foreground);
        assert_eq!(DARK.resolve(Color::Indexed(1), &colors, DARK.foreground), hex(0xf14c4c));
        assert_eq!(DARK.resolve(Color::Indexed(16), &colors, DARK.foreground), Rgb { r: 0, g: 0, b: 0 });
        assert_eq!(DARK.resolve(Color::Indexed(255), &colors, DARK.foreground), Rgb { r: 238, g: 238, b: 238 });

        let mut overridden = Colors::default();
        overridden[NamedColor::Foreground] = Some(hex(0x123456));
        assert_eq!(DARK.resolve(Color::Named(NamedColor::Foreground), &overridden, DARK.background), hex(0x123456));
    }
}
