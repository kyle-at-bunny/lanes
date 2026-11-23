#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Mods {
    fn xterm_param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }

    fn any(self) -> bool {
        self.ctrl || self.alt || self.shift
    }
}

/// Encodes a key press as the bytes an xterm-compatible terminal would send.
pub fn encode_key(key: &str, key_char: Option<&str>, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    if let Some(bytes) = encode_named(key, mods, app_cursor) {
        return Some(bytes);
    }
    if mods.ctrl && !mods.shift {
        if let Some(byte) = control_byte(key) {
            return Some(with_alt(mods, vec![byte]));
        }
    }
    let text = key_char.filter(|c| !c.is_empty()).or_else(|| (key.chars().count() == 1).then_some(key))?;
    Some(with_alt(mods, text.as_bytes().to_vec()))
}

pub fn bracketed_paste(text: &str, enabled: bool) -> Vec<u8> {
    let normalized = text.replace("\r\n", "\r").replace('\n', "\r");
    if enabled { format!("\x1b[200~{normalized}\x1b[201~").into_bytes() } else { normalized.into_bytes() }
}

fn encode_named(key: &str, mods: Mods, app_cursor: bool) -> Option<Vec<u8>> {
    let seq: String = match key {
        "enter" if mods.shift || mods.alt => "\x1b\r".into(),
        "enter" => "\r".into(),
        "tab" if mods.shift => "\x1b[Z".into(),
        "tab" => "\t".into(),
        "escape" => "\x1b".into(),
        "backspace" if mods.ctrl => "\x08".into(),
        "backspace" if mods.alt => "\x1b\x7f".into(),
        "backspace" => "\x7f".into(),
        "space" if mods.ctrl => "\x00".into(),
        "up" => cursor_key('A', mods, app_cursor),
        "down" => cursor_key('B', mods, app_cursor),
        "right" => cursor_key('C', mods, app_cursor),
        "left" => cursor_key('D', mods, app_cursor),
        "home" => cursor_key('H', mods, app_cursor),
        "end" => cursor_key('F', mods, app_cursor),
        "insert" => tilde_key(2, mods),
        "delete" => tilde_key(3, mods),
        "pageup" => tilde_key(5, mods),
        "pagedown" => tilde_key(6, mods),
        "f1" => ss3_key('P', mods),
        "f2" => ss3_key('Q', mods),
        "f3" => ss3_key('R', mods),
        "f4" => ss3_key('S', mods),
        "f5" => tilde_key(15, mods),
        "f6" => tilde_key(17, mods),
        "f7" => tilde_key(18, mods),
        "f8" => tilde_key(19, mods),
        "f9" => tilde_key(20, mods),
        "f10" => tilde_key(21, mods),
        "f11" => tilde_key(23, mods),
        "f12" => tilde_key(24, mods),
        _ => return None,
    };
    Some(seq.into_bytes())
}

fn cursor_key(code: char, mods: Mods, app_cursor: bool) -> String {
    match (mods.any(), app_cursor) {
        (true, _) => format!("\x1b[1;{}{code}", mods.xterm_param()),
        (false, true) => format!("\x1bO{code}"),
        (false, false) => format!("\x1b[{code}"),
    }
}

fn ss3_key(code: char, mods: Mods) -> String {
    if mods.any() { format!("\x1b[1;{}{code}", mods.xterm_param()) } else { format!("\x1bO{code}") }
}

fn tilde_key(number: u8, mods: Mods) -> String {
    if mods.any() { format!("\x1b[{number};{}~", mods.xterm_param()) } else { format!("\x1b[{number}~") }
}

fn control_byte(key: &str) -> Option<u8> {
    let mut chars = key.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else { return None };
    match c.to_ascii_lowercase() {
        c @ 'a'..='z' => Some(c as u8 - b'a' + 1),
        '@' | '2' => Some(0),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' => Some(0x1e),
        '_' | '-' | '7' => Some(0x1f),
        '8' => Some(0x7f),
        _ => None,
    }
}

fn with_alt(mods: Mods, mut bytes: Vec<u8>) -> Vec<u8> {
    if mods.alt {
        bytes.insert(0, 0x1b);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { ctrl: false, alt: false, shift: false };
    const CTRL: Mods = Mods { ctrl: true, alt: false, shift: false };
    const SHIFT: Mods = Mods { ctrl: false, alt: false, shift: true };
    const ALT: Mods = Mods { ctrl: false, alt: true, shift: false };

    fn enc(key: &str, key_char: Option<&str>, mods: Mods, app: bool) -> Vec<u8> {
        encode_key(key, key_char, mods, app).unwrap()
    }

    #[test]
    fn encodes_text_and_control_keys() {
        assert_eq!(enc("a", Some("a"), NONE, false), b"a");
        assert_eq!(enc("a", Some("A"), SHIFT, false), b"A");
        assert_eq!(enc("c", None, CTRL, false), [3]);
        assert_eq!(enc("x", Some("x"), ALT, false), b"\x1bx");
        assert_eq!(enc("enter", None, NONE, false), b"\r");
        assert_eq!(enc("enter", None, SHIFT, false), b"\x1b\r");
        assert_eq!(enc("tab", None, SHIFT, false), b"\x1b[Z");
        assert_eq!(enc("backspace", None, NONE, false), [0x7f]);
    }

    #[test]
    fn encodes_cursor_and_function_keys() {
        assert_eq!(enc("up", None, NONE, false), b"\x1b[A");
        assert_eq!(enc("up", None, NONE, true), b"\x1bOA");
        assert_eq!(enc("left", None, CTRL, true), b"\x1b[1;5D");
        assert_eq!(enc("delete", None, NONE, false), b"\x1b[3~");
        assert_eq!(enc("f1", None, NONE, false), b"\x1bOP");
        assert_eq!(enc("f5", None, SHIFT, false), b"\x1b[15;2~");
        assert_eq!(encode_key("shift", None, SHIFT, false), None);
    }

    #[test]
    fn paste_normalizes_newlines_and_brackets() {
        assert_eq!(bracketed_paste("a\r\nb\nc", false), b"a\rb\rc");
        assert_eq!(bracketed_paste("x", true), b"\x1b[200~x\x1b[201~");
    }
}
