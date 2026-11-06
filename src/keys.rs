//! NSH keystrokes to libghostty key events.
//!
//! `nsh_core::on_key` reports a keystroke as written (`"ctrl+c"`), and
//! `libghostty-vt`'s encoder turns a physical key plus modifiers into the
//! bytes the shell expects, following the terminal's modes.

use libghostty_vt::key::{Key, Mods};

/// The keystrokes the terminal surface listens to.
pub const TERMINAL_KEYS: &[&str] = &[
    "enter",
    "tab",
    "escape",
    "backspace",
    "delete",
    "insert",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "ctrl+a",
    "ctrl+b",
    "ctrl+c",
    "ctrl+d",
    "ctrl+e",
    "ctrl+f",
    "ctrl+g",
    "ctrl+h",
    "ctrl+i",
    "ctrl+j",
    "ctrl+k",
    "ctrl+l",
    "ctrl+m",
    "ctrl+n",
    "ctrl+o",
    "ctrl+p",
    "ctrl+r",
    "ctrl+s",
    "ctrl+t",
    "ctrl+u",
    "ctrl+v",
    "ctrl+w",
    "ctrl+x",
    "ctrl+y",
    "ctrl+z",
    "ctrl+space",
    "ctrl+backspace",
    "alt+backspace",
];

/// `"ctrl+shift+left"` and the rest to a key and its modifiers.
pub fn parse(keystroke: &str) -> Option<(Key, Mods)> {
    let mut parts: Vec<&str> = keystroke.split('+').collect();
    let base = parts.pop()?;
    let mut mods = Mods::empty();
    for part in parts {
        match part.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= Mods::CTRL,
            "shift" => mods |= Mods::SHIFT,
            "alt" | "option" | "opt" => mods |= Mods::ALT,
            "super" | "cmd" | "command" | "win" => mods |= Mods::SUPER,
            _ => {}
        }
    }
    Some((key(base)?, mods))
}

fn key(base: &str) -> Option<Key> {
    let lower = base.to_ascii_lowercase();
    let key = match lower.as_str() {
        "enter" | "return" => Key::Enter,
        "tab" => Key::Tab,
        "escape" | "esc" => Key::Escape,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "insert" => Key::Insert,
        "up" => Key::ArrowUp,
        "down" => Key::ArrowDown,
        "left" => Key::ArrowLeft,
        "right" => Key::ArrowRight,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "space" => Key::Space,
        "minus" => Key::Minus,
        "equal" | "plus" => Key::Equal,
        "comma" => Key::Comma,
        "period" => Key::Period,
        "slash" => Key::Slash,
        "semicolon" => Key::Semicolon,
        "quote" => Key::Quote,
        "backslash" => Key::Backslash,
        "bracketleft" => Key::BracketLeft,
        "bracketright" => Key::BracketRight,
        "backquote" => Key::Backquote,
        single => {
            let ch = single.chars().next()?;
            return match ch {
                'a'..='z' => Some(letter(ch)),
                '0'..='9' => Some(digit(ch)),
                _ => None,
            };
        }
    };
    Some(key)
}

fn letter(ch: char) -> Key {
    const LETTERS: [Key; 26] = [
        Key::A, Key::B, Key::C, Key::D, Key::E, Key::F, Key::G, Key::H, Key::I, Key::J, Key::K,
        Key::L, Key::M, Key::N, Key::O, Key::P, Key::Q, Key::R, Key::S, Key::T, Key::U, Key::V,
        Key::W, Key::X, Key::Y, Key::Z,
    ];
    LETTERS[(ch as u8 - b'a') as usize]
}

fn digit(ch: char) -> Key {
    const DIGITS: [Key; 10] = [
        Key::Digit0,
        Key::Digit1,
        Key::Digit2,
        Key::Digit3,
        Key::Digit4,
        Key::Digit5,
        Key::Digit6,
        Key::Digit7,
        Key::Digit8,
        Key::Digit9,
    ];
    DIGITS[(ch as u8 - b'0') as usize]
}
