use serde::{Deserialize, Serialize};

pub const MOD_CTRL: u32 = 1;
pub const MOD_ALT: u32 = 2;
pub const MOD_SHIFT: u32 = 4;
pub const MOD_WIN: u32 = 8;

/// A toggle hotkey needs Ctrl, Alt or Win: Shift alone is ordinary typing.
pub const HOTKEY_PRIMARY_MODS: u32 = MOD_CTRL | MOD_ALT | MOD_WIN;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Hotkey {
    pub mods: u32,
    pub vk: u32,
}

/// Keys a custom trigger must never use: the helper swallows the trigger key system-wide.
pub const UNSAFE_TRIGGER_VKS: [u32; 9] = [
    0x0d, // Enter
    0x09, // Tab
    0x08, // Backspace
    0x20, // Space
    0x1b, // Escape
    0x25, // ArrowLeft
    0x26, // ArrowUp
    0x27, // ArrowRight
    0x28, // ArrowDown
];

const NAMED_CODES: &[(&str, u32)] = &[
    ("Space", 0x20), ("Enter", 0x0d), ("Tab", 0x09), ("Backspace", 0x08), ("Escape", 0x1b),
    ("Insert", 0x2d), ("Delete", 0x2e), ("Home", 0x24), ("End", 0x23), ("PageUp", 0x21),
    ("PageDown", 0x22), ("ArrowLeft", 0x25), ("ArrowUp", 0x26), ("ArrowRight", 0x27),
    ("ArrowDown", 0x28), ("CapsLock", 0x14), ("Pause", 0x13), ("ScrollLock", 0x91),
    ("PrintScreen", 0x2c), ("ContextMenu", 0x5d), ("Backquote", 0xc0), ("Minus", 0xbd),
    ("Equal", 0xbb), ("BracketLeft", 0xdb), ("BracketRight", 0xdd), ("Backslash", 0xdc),
    ("Semicolon", 0xba), ("Quote", 0xde), ("Comma", 0xbc), ("Period", 0xbe), ("Slash", 0xbf),
    ("NumpadMultiply", 0x6a), ("NumpadAdd", 0x6b), ("NumpadSubtract", 0x6d), ("NumpadDecimal", 0x6e),
    ("NumpadDivide", 0x6f),
];

/// KeyboardEvent.code → Windows virtual-key code. Modifiers and unknown codes → None.
pub fn code_to_vk(code: &str) -> Option<u32> {
    if let Some(rest) = code.strip_prefix("Key") {
        let mut chars = rest.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_uppercase() {
                return Some(c as u32);
            }
        }
    }
    if let Some(d) = code.strip_prefix("Digit").and_then(single_digit) {
        return Some(0x30 + d);
    }
    if let Some(d) = code.strip_prefix("Numpad").and_then(single_digit) {
        return Some(0x60 + d);
    }
    if let Some(rest) = code.strip_prefix('F') {
        if (1..=2).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_digit()) {
            let n: u32 = rest.parse().unwrap();
            if (1..=24).contains(&n) {
                return Some(0x6f + n);
            }
        }
    }
    NAMED_CODES.iter().find(|(name, _)| *name == code).map(|(_, vk)| *vk)
}

fn single_digit(s: &str) -> Option<u32> {
    let mut chars = s.chars();
    match (chars.next(), chars.next()) {
        (Some(d), None) if d.is_ascii_digit() => Some(d as u32 - '0' as u32),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub const CTRL_LABEL: &str = "⌃ Control";
#[cfg(not(target_os = "macos"))]
pub const CTRL_LABEL: &str = "Ctrl";

#[cfg(target_os = "macos")]
pub const ALT_LABEL: &str = "⌥ Option";
#[cfg(not(target_os = "macos"))]
pub const ALT_LABEL: &str = "Alt";

#[cfg(target_os = "macos")]
pub const SHIFT_LABEL: &str = "⇧ Shift";
#[cfg(not(target_os = "macos"))]
pub const SHIFT_LABEL: &str = "Shift";

#[cfg(target_os = "macos")]
pub const WIN_LABEL: &str = "⌘ Command";
#[cfg(not(target_os = "macos"))]
pub const WIN_LABEL: &str = "Win";

#[cfg(target_os = "macos")]
pub const CAPS_LABEL: &str = "⇪ Caps Lock";
#[cfg(not(target_os = "macos"))]
pub const CAPS_LABEL: &str = "Caps Lock";

const VK_LABELS: &[(u32, &str)] = &[
    (0x20, "Space"), (0x0d, "Enter"), (0x09, "Tab"), (0x08, "Backspace"), (0x1b, "Esc"),
    (0x2d, "Insert"), (0x2e, "Delete"), (0x24, "Home"), (0x23, "End"), (0x21, "Page Up"),
    (0x22, "Page Down"), (0x25, "←"), (0x26, "↑"), (0x27, "→"), (0x28, "↓"), (0x14, CAPS_LABEL),
    (0x13, "Pause"), (0x91, "Scroll Lock"), (0x2c, "Print Screen"), (0x5d, "Menu"), (0xc0, "`"),
    (0xbd, "-"), (0xbb, "="), (0xdb, "["), (0xdd, "]"), (0xdc, "\\"), (0xba, ";"), (0xde, "'"),
    (0xbc, ","), (0xbe, "."), (0xbf, "/"), (0x6a, "Num *"), (0x6b, "Num +"), (0x6d, "Num -"),
    (0x6e, "Num ."), (0x6f, "Num /"),
];

pub fn vk_label(vk: u32) -> String {
    if (0x41..=0x5a).contains(&vk) || (0x30..=0x39).contains(&vk) {
        return (vk as u8 as char).to_string();
    }
    if (0x60..=0x69).contains(&vk) {
        return format!("Num {}", vk - 0x60);
    }
    if (0x70..=0x87).contains(&vk) {
        return format!("F{}", vk - 0x6f);
    }
    match VK_LABELS.iter().find(|(code, _)| *code == vk) {
        Some((_, label)) => (*label).to_string(),
        None => format!("Key {vk:02X}"),
    }
}

const MOD_LABELS: [&str; 4] = [CTRL_LABEL, ALT_LABEL, SHIFT_LABEL, WIN_LABEL];

pub fn mod_labels(mods: u32) -> Vec<&'static str> {
    let mut out = Vec::new();
    if mods & MOD_CTRL != 0 {
        out.push(MOD_LABELS[0]);
    }
    if mods & MOD_ALT != 0 {
        out.push(MOD_LABELS[1]);
    }
    if mods & MOD_SHIFT != 0 {
        out.push(MOD_LABELS[2]);
    }
    if mods & MOD_WIN != 0 {
        out.push(MOD_LABELS[3]);
    }
    out
}

pub fn hotkey_parts(h: Hotkey) -> Vec<String> {
    let mut out: Vec<String> = mod_labels(h.mods).into_iter().map(str::to_string).collect();
    out.push(vk_label(h.vk));
    out
}

/// "Ctrl + Alt + P".
pub fn hotkey_label(h: Hotkey) -> String {
    hotkey_parts(h).join(" + ")
}

pub fn event_mods(ctrl: bool, alt: bool, shift: bool, meta: bool) -> u32 {
    (if ctrl { MOD_CTRL } else { 0 })
        | (if alt { MOD_ALT } else { 0 })
        | (if shift { MOD_SHIFT } else { 0 })
        | (if meta { MOD_WIN } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_and_labels() {
        assert_eq!(code_to_vk("KeyP"), Some(0x50));
        assert_eq!(code_to_vk("Digit7"), Some(0x37));
        assert_eq!(code_to_vk("Numpad3"), Some(0x63));
        assert_eq!(code_to_vk("F24"), Some(0x87));
        assert_eq!(code_to_vk("F25"), None);
        assert_eq!(code_to_vk("CapsLock"), Some(0x14));
        assert_eq!(code_to_vk("ShiftLeft"), None);

        assert_eq!(vk_label(0x50), "P");
        assert_eq!(vk_label(0x14), "Caps Lock");
        assert_eq!(vk_label(0x61), "Num 1");
        assert_eq!(vk_label(0x70), "F1");
        assert_eq!(vk_label(0x8a), "Key 8A");

        let h = Hotkey { mods: MOD_CTRL | MOD_ALT, vk: 0x50 };
        assert_eq!(hotkey_label(h), format!("{} + {} + P", MOD_LABELS[0], MOD_LABELS[1]));
        assert_eq!(event_mods(true, false, true, false), MOD_CTRL | MOD_SHIFT);
    }
}
