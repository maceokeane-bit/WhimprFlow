//! Parse and describe push-to-talk hotkey strings for macOS CGEventTap matching.
//!
//! Format: `modifier+key` with `+` separators, case-insensitive.
//! Modifiers: `fn`, `command`/`cmd`, `control`/`ctrl`, `option`/`alt`, `shift`.
//! Keys: single letter `a`–`z`, or named `space`, `escape`/`esc`.
//!
//! Examples: `option+w`, `fn`, `control+shift+d`.

use serde::{Deserialize, Serialize};

/// macOS CGEventFlags masks (same values CoreGraphics uses).
pub mod flags {
    pub const SHIFT: u64 = 0x0002_0000;
    pub const CONTROL: u64 = 0x0004_0000;
    pub const ALT: u64 = 0x0008_0000;
    pub const COMMAND: u64 = 0x0010_0000;
    pub const FN: u64 = 0x0080_0000;
}

/// A resolved push-to-talk binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HotkeyBinding {
    /// `true` for the hardware Fn/Globe key (keycode 63, flagsChanged).
    pub is_fn: bool,
    /// Virtual keycode for letter/named keys (ignored when `is_fn`).
    pub keycode: u32,
    /// Required modifier flags that must be set when the key fires.
    pub modifiers: u64,
    /// Original string for display.
    pub label: String,
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        parse_hotkey("option+w").unwrap()
    }
}

pub fn default_ptt_hotkey() -> String {
    "option+w".to_string()
}

/// Presets shown in the Settings UI.
pub const PRESETS: &[(&str, &str)] = &[
    ("option+w", "Option + W (recommended)"),
    ("fn", "Fn / Globe key"),
    ("control+option", "Control + Option"),
    ("command+shift+space", "Command + Shift + Space"),
    ("right-option", "Right Option (hold)"),
];

fn keycode_for_name(name: &str) -> Option<u32> {
    match name {
        "space" => Some(49),
        "escape" | "esc" => Some(53),
        "return" | "enter" => Some(36),
        "tab" => Some(48),
        "delete" | "backspace" => Some(51),
        "right-option" | "right_option" => Some(61),
        "left-option" | "left_option" => Some(58),
        "right-control" | "right_control" => Some(62),
        "left-control" | "left_control" => Some(59),
        s if s.len() == 1 => {
            let c = s.chars().next()?;
            letter_keycode(c)
        }
        _ => None,
    }
}

fn letter_keycode(c: char) -> Option<u32> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a' => 0,
        's' => 1,
        'd' => 2,
        'f' => 3,
        'h' => 4,
        'g' => 5,
        'z' => 6,
        'x' => 7,
        'c' => 8,
        'v' => 9,
        'b' => 11,
        'q' => 12,
        'w' => 13,
        'e' => 14,
        'r' => 15,
        'y' => 16,
        't' => 17,
        '1' => 18,
        '2' => 19,
        '3' => 20,
        '4' => 21,
        '6' => 22,
        '5' => 23,
        '9' => 25,
        '7' => 26,
        '8' => 28,
        '0' => 29,
        'o' => 31,
        'u' => 32,
        'i' => 34,
        'p' => 35,
        'l' => 37,
        'j' => 38,
        'k' => 40,
        'n' => 45,
        'm' => 46,
        _ => return None,
    })
}

fn modifier_flag(token: &str) -> Option<u64> {
    match token {
        "fn" | "globe" => Some(flags::FN),
        "command" | "cmd" | "⌘" => Some(flags::COMMAND),
        "control" | "ctrl" | "⌃" => Some(flags::CONTROL),
        "option" | "alt" | "⌥" => Some(flags::ALT),
        "shift" | "⇧" => Some(flags::SHIFT),
        _ => None,
    }
}

/// Parse a hotkey string. Returns an error message on failure.
pub fn parse_hotkey(raw: &str) -> Result<HotkeyBinding, String> {
    let label = raw.trim().to_lowercase();
    if label.is_empty() {
        return Err("hotkey is empty".into());
    }
    let parts: Vec<&str> = label.split('+').map(str::trim).filter(|s| !s.is_empty()).collect();
    if parts.is_empty() {
        return Err("hotkey is empty".into());
    }

    // Bare `fn`.
    if parts.len() == 1 && parts[0] == "fn" {
        return Ok(HotkeyBinding {
            is_fn: true,
            keycode: 63,
            modifiers: 0,
            label: label.clone(),
        });
    }

    // Modifier-only hold (e.g. right-option, control+option).
    let last = parts[parts.len() - 1];
    if let Some(kc) = keycode_for_name(last) {
        let mut modifiers = 0u64;
        for p in &parts[..parts.len() - 1] {
            modifiers |= modifier_flag(p).ok_or_else(|| format!("unknown modifier '{p}'"))?;
        }
        // Standalone modifier key (right-option with no other key).
        if parts.len() == 1 && (last.contains("option") || last.contains("control")) {
            return Ok(HotkeyBinding {
                is_fn: false,
                keycode: kc,
                modifiers: 0,
                label: label.clone(),
            });
        }
        return Ok(HotkeyBinding {
            is_fn: false,
            keycode: kc,
            modifiers,
            label: label.clone(),
        });
    }

    Err(format!("unknown key '{last}'"))
}

/// Whether `event_flags` contains all required modifiers (ignoring Fn for combo keys).
pub fn modifiers_match(required: u64, event_flags: u64) -> bool {
    if required == 0 {
        return true;
    }
    (event_flags & required) == required
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_option_w() {
        let b = parse_hotkey("option+w").unwrap();
        assert_eq!(b.keycode, 13);
        assert_eq!(b.modifiers, flags::ALT);
        assert!(!b.is_fn);
    }

    #[test]
    fn parses_fn() {
        let b = parse_hotkey("fn").unwrap();
        assert!(b.is_fn);
    }
}
