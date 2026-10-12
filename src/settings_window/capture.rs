use eframe::egui;

use crate::keys::{code_to_vk, HOTKEY_PRIMARY_MODS, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};

const MODIFIER_CODES: [&str; 8] =
    ["ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight", "MetaLeft", "MetaRight"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Hint {
    Unusable,
    NotTrigger,
    NeedModifier,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Outcome {
    Ignore,
    Cancel,
    Hint(Hint),
    Capture { mods: u32, vk: u32 },
}

// The reference's keyCaptureLogic.evaluateKeyDown, over KeyboardEvent.code.
pub(super) fn evaluate(code: &str, repeat: bool, mods: u32, forbidden: &[u32], require_modifier: bool) -> Outcome {
    if repeat {
        return Outcome::Ignore;
    }
    if code == "Escape" {
        return Outcome::Cancel;
    }
    if MODIFIER_CODES.contains(&code) {
        return Outcome::Ignore;
    }
    let Some(vk) = code_to_vk(code) else {
        return Outcome::Hint(Hint::Unusable);
    };
    if forbidden.contains(&vk) {
        return Outcome::Hint(Hint::NotTrigger);
    }
    let mods = if require_modifier { mods } else { 0 };
    if require_modifier && mods & HOTKEY_PRIMARY_MODS == 0 {
        return Outcome::Hint(Hint::NeedModifier);
    }
    Outcome::Capture { mods, vk }
}

// egui tracks no Win key, so the window watches the Super keys itself.
pub(super) fn mods_of(m: egui::Modifiers, super_down: bool) -> u32 {
    let mut mods = 0;
    if m.ctrl {
        mods |= MOD_CTRL;
    }
    if m.alt {
        mods |= MOD_ALT;
    }
    if m.shift {
        mods |= MOD_SHIFT;
    }
    if super_down {
        mods |= MOD_WIN;
    }
    mods
}

// egui names keys after their physical position, like KeyboardEvent.code; the numpad merges into the main row.
pub(super) fn code_of(key: egui::Key) -> Option<&'static str> {
    use egui::Key as K;
    Some(match key {
        K::A => "KeyA",
        K::B => "KeyB",
        K::C => "KeyC",
        K::D => "KeyD",
        K::E => "KeyE",
        K::F => "KeyF",
        K::G => "KeyG",
        K::H => "KeyH",
        K::I => "KeyI",
        K::J => "KeyJ",
        K::K => "KeyK",
        K::L => "KeyL",
        K::M => "KeyM",
        K::N => "KeyN",
        K::O => "KeyO",
        K::P => "KeyP",
        K::Q => "KeyQ",
        K::R => "KeyR",
        K::S => "KeyS",
        K::T => "KeyT",
        K::U => "KeyU",
        K::V => "KeyV",
        K::W => "KeyW",
        K::X => "KeyX",
        K::Y => "KeyY",
        K::Z => "KeyZ",
        K::Num0 => "Digit0",
        K::Num1 => "Digit1",
        K::Num2 => "Digit2",
        K::Num3 => "Digit3",
        K::Num4 => "Digit4",
        K::Num5 => "Digit5",
        K::Num6 => "Digit6",
        K::Num7 => "Digit7",
        K::Num8 => "Digit8",
        K::Num9 => "Digit9",
        K::F1 => "F1",
        K::F2 => "F2",
        K::F3 => "F3",
        K::F4 => "F4",
        K::F5 => "F5",
        K::F6 => "F6",
        K::F7 => "F7",
        K::F8 => "F8",
        K::F9 => "F9",
        K::F10 => "F10",
        K::F11 => "F11",
        K::F12 => "F12",
        K::F13 => "F13",
        K::F14 => "F14",
        K::F15 => "F15",
        K::F16 => "F16",
        K::F17 => "F17",
        K::F18 => "F18",
        K::F19 => "F19",
        K::F20 => "F20",
        K::F21 => "F21",
        K::F22 => "F22",
        K::F23 => "F23",
        K::F24 => "F24",
        K::Escape => "Escape",
        K::Enter => "Enter",
        K::Tab => "Tab",
        K::Backspace => "Backspace",
        K::Space => "Space",
        K::Insert => "Insert",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::ArrowLeft => "ArrowLeft",
        K::ArrowUp => "ArrowUp",
        K::ArrowRight => "ArrowRight",
        K::ArrowDown => "ArrowDown",
        K::Minus => "Minus",
        K::Equals => "Equal",
        K::OpenBracket => "BracketLeft",
        K::CloseBracket => "BracketRight",
        K::Backslash => "Backslash",
        K::Semicolon => "Semicolon",
        K::Quote => "Quote",
        K::Comma => "Comma",
        K::Period => "Period",
        K::Slash => "Slash",
        K::Backtick => "Backquote",
        K::Plus => "NumpadAdd",
        K::ControlLeft => "ControlLeft",
        K::ControlRight => "ControlRight",
        K::AltLeft => "AltLeft",
        K::AltRight => "AltRight",
        K::ShiftLeft => "ShiftLeft",
        K::ShiftRight => "ShiftRight",
        K::SuperLeft => "MetaLeft",
        K::SuperRight => "MetaRight",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::UNSAFE_TRIGGER_VKS;

    #[test]
    fn physical_keys_reach_the_reference_vk_table() {
        let cases = [
            (egui::Key::A, 0x41),
            (egui::Key::Z, 0x5a),
            (egui::Key::Num5, 0x35),
            (egui::Key::F1, 0x70),
            (egui::Key::F24, 0x87),
            (egui::Key::Escape, 0x1b),
            (egui::Key::Space, 0x20),
            (egui::Key::Equals, 0xbb),
            (egui::Key::Backtick, 0xc0),
            (egui::Key::Semicolon, 0xba),
            (egui::Key::ArrowDown, 0x28),
            (egui::Key::Plus, 0x6b),
        ];
        for (key, vk) in cases {
            assert_eq!(code_to_vk(code_of(key).unwrap()), Some(vk), "{key:?}");
        }
        assert_eq!(code_of(egui::Key::F25), None);
        assert_eq!(code_of(egui::Key::Colon), None);
    }

    #[test]
    fn a_trigger_takes_a_bare_key_and_drops_the_modifiers() {
        assert_eq!(evaluate("KeyQ", false, 0, &UNSAFE_TRIGGER_VKS, false), Outcome::Capture { mods: 0, vk: 0x51 });
        assert_eq!(
            evaluate("KeyQ", false, MOD_CTRL | MOD_ALT, &UNSAFE_TRIGGER_VKS, false),
            Outcome::Capture { mods: 0, vk: 0x51 }
        );
    }

    #[test]
    fn escape_cancels_and_a_modifier_alone_is_ignored() {
        assert_eq!(evaluate("Escape", false, 0, &[], false), Outcome::Cancel);
        assert_eq!(evaluate("ControlLeft", false, 0, &[], false), Outcome::Ignore);
        assert_eq!(evaluate("AltRight", false, 0, &[], false), Outcome::Ignore);
        assert_eq!(evaluate("MetaRight", false, 0, &[], false), Outcome::Ignore);
    }

    #[test]
    fn a_repeated_key_is_not_a_choice() {
        assert_eq!(evaluate("KeyQ", true, 0, &[], false), Outcome::Ignore);
    }

    #[test]
    fn unknown_and_forbidden_keys_hint() {
        assert_eq!(evaluate("IntlBackslash", false, 0, &[], false), Outcome::Hint(Hint::Unusable));
        assert_eq!(evaluate("", false, 0, &[], false), Outcome::Hint(Hint::Unusable));
        assert_eq!(evaluate("Enter", false, 0, &UNSAFE_TRIGGER_VKS, false), Outcome::Hint(Hint::NotTrigger));
        assert_eq!(evaluate("KeyA", false, 0, &[0x41], false), Outcome::Hint(Hint::NotTrigger));
    }

    #[test]
    fn a_shortcut_needs_ctrl_alt_or_win() {
        assert_eq!(evaluate("KeyP", false, 0, &[], true), Outcome::Hint(Hint::NeedModifier));
        assert_eq!(evaluate("KeyP", false, MOD_SHIFT, &[], true), Outcome::Hint(Hint::NeedModifier));
        assert_eq!(
            evaluate("KeyP", false, MOD_CTRL | MOD_SHIFT, &[], true),
            Outcome::Capture { mods: MOD_CTRL | MOD_SHIFT, vk: 0x50 }
        );
        assert_eq!(
            evaluate("KeyP", false, MOD_ALT | MOD_WIN, &[], true),
            Outcome::Capture { mods: MOD_ALT | MOD_WIN, vk: 0x50 }
        );
    }

    #[test]
    fn the_win_key_counts_as_a_modifier() {
        let ctrl = egui::Modifiers { alt: false, ctrl: true, shift: false, mac_cmd: false, command: true };
        assert_eq!(mods_of(ctrl, false), MOD_CTRL);
        assert_eq!(mods_of(ctrl, true), MOD_CTRL | MOD_WIN);
        assert_eq!(mods_of(egui::Modifiers::default(), true), MOD_WIN);
    }
}
