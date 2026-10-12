// macOS virtual key codes to the Windows virtual-key codes the rest of the app speaks.
pub const FLAG_SHIFT: u64 = 0x0002_0000;
pub const FLAG_CONTROL: u64 = 0x0004_0000;
pub const FLAG_OPTION: u64 = 0x0008_0000;
pub const FLAG_COMMAND: u64 = 0x0010_0000;

pub const DEV_LCTRL: u64 = 0x0001;
pub const DEV_LSHIFT: u64 = 0x0002;
pub const DEV_RSHIFT: u64 = 0x0004;
pub const DEV_LCMD: u64 = 0x0008;
pub const DEV_RCMD: u64 = 0x0010;
pub const DEV_LALT: u64 = 0x0020;
pub const DEV_RALT: u64 = 0x0040;
pub const DEV_RCTRL: u64 = 0x2000;

/// The four group masks and the eight left/right device bits, i.e. everything a modifier change can move.
pub const MODIFIER_FLAGS: u64 = FLAG_SHIFT
    | FLAG_CONTROL
    | FLAG_OPTION
    | FLAG_COMMAND
    | DEV_LCTRL
    | DEV_LSHIFT
    | DEV_RSHIFT
    | DEV_LCMD
    | DEV_RCMD
    | DEV_LALT
    | DEV_RALT
    | DEV_RCTRL;

const VK_LSHIFT: u32 = 0xA0;
const VK_RSHIFT: u32 = 0xA1;
const VK_LCONTROL: u32 = 0xA2;
const VK_RCONTROL: u32 = 0xA3;
const VK_LMENU: u32 = 0xA4;
const VK_RMENU: u32 = 0xA5;
const VK_LWIN: u32 = 0x5B;
const VK_RWIN: u32 = 0x5C;

/// Carbon's kVK_* to VK, in the order the reference helper lists them.
const KEYS: &[(u16, u32)] = &[
    (0x00, 0x41),
    (0x0B, 0x42),
    (0x08, 0x43),
    (0x02, 0x44),
    (0x0E, 0x45),
    (0x03, 0x46),
    (0x05, 0x47),
    (0x04, 0x48),
    (0x22, 0x49),
    (0x26, 0x4A),
    (0x28, 0x4B),
    (0x25, 0x4C),
    (0x2E, 0x4D),
    (0x2D, 0x4E),
    (0x1F, 0x4F),
    (0x23, 0x50),
    (0x0C, 0x51),
    (0x0F, 0x52),
    (0x01, 0x53),
    (0x11, 0x54),
    (0x20, 0x55),
    (0x09, 0x56),
    (0x0D, 0x57),
    (0x07, 0x58),
    (0x10, 0x59),
    (0x06, 0x5A),
    (0x1D, 0x30),
    (0x12, 0x31),
    (0x13, 0x32),
    (0x14, 0x33),
    (0x15, 0x34),
    (0x17, 0x35),
    (0x16, 0x36),
    (0x1A, 0x37),
    (0x1C, 0x38),
    (0x19, 0x39),
    (0x7A, 0x70),
    (0x78, 0x71),
    (0x63, 0x72),
    (0x76, 0x73),
    (0x60, 0x74),
    (0x61, 0x75),
    (0x62, 0x76),
    (0x64, 0x77),
    (0x65, 0x78),
    (0x6D, 0x79),
    (0x67, 0x7A),
    (0x6F, 0x7B),
    (0x69, 0x7C),
    (0x6B, 0x7D),
    (0x71, 0x7E),
    (0x6A, 0x7F),
    (0x40, 0x80),
    (0x4F, 0x81),
    (0x50, 0x82),
    (0x5A, 0x83),
    (0x24, 0x0D),
    (0x4C, 0x0D),
    (0x30, 0x09),
    (0x31, 0x20),
    (0x33, 0x08),
    (0x35, 0x1B),
    (0x75, 0x2E),
    (0x72, 0x2D),
    (0x73, 0x24),
    (0x77, 0x23),
    (0x74, 0x21),
    (0x79, 0x22),
    (0x7B, 0x25),
    (0x7E, 0x26),
    (0x7C, 0x27),
    (0x7D, 0x28),
    (0x32, 0xC0),
    (0x1B, 0xBD),
    (0x18, 0xBB),
    (0x21, 0xDB),
    (0x1E, 0xDD),
    (0x2A, 0xDC),
    (0x29, 0xBA),
    (0x27, 0xDE),
    (0x2B, 0xBC),
    (0x2F, 0xBE),
    (0x2C, 0xBF),
    (0x52, 0x60),
    (0x53, 0x61),
    (0x54, 0x62),
    (0x55, 0x63),
    (0x56, 0x64),
    (0x57, 0x65),
    (0x58, 0x66),
    (0x59, 0x67),
    (0x5B, 0x68),
    (0x5C, 0x69),
    (0x43, 0x6A),
    (0x45, 0x6B),
    (0x4E, 0x6D),
    (0x41, 0x6E),
    (0x4B, 0x6F),
    (0x39, 0x14),
    (0x38, 0xA0),
    (0x3C, 0xA1),
    (0x3B, 0xA2),
    (0x3E, 0xA3),
    (0x3A, 0xA4),
    (0x3D, 0xA5),
    (0x37, 0x5B),
    (0x36, 0x5C),
];

/// VK, its group mask, its own device bit, and the other side's.
const MODS: &[(u32, u64, u64, u64)] = &[
    (VK_LSHIFT, FLAG_SHIFT, DEV_LSHIFT, DEV_RSHIFT),
    (VK_RSHIFT, FLAG_SHIFT, DEV_RSHIFT, DEV_LSHIFT),
    (VK_LCONTROL, FLAG_CONTROL, DEV_LCTRL, DEV_RCTRL),
    (VK_RCONTROL, FLAG_CONTROL, DEV_RCTRL, DEV_LCTRL),
    (VK_LMENU, FLAG_OPTION, DEV_LALT, DEV_RALT),
    (VK_RMENU, FLAG_OPTION, DEV_RALT, DEV_LALT),
    (VK_LWIN, FLAG_COMMAND, DEV_LCMD, DEV_RCMD),
    (VK_RWIN, FLAG_COMMAND, DEV_RCMD, DEV_LCMD),
];

pub fn key_code_to_vk(key_code: u16) -> u32 {
    KEYS.iter().find(|(code, _)| *code == key_code).map_or(0, |(_, vk)| *vk)
}

pub fn vk_to_key_code(vk: u32) -> Option<u16> {
    KEYS.iter().find(|(_, code)| *code == vk).map(|(code, _)| *code)
}

pub fn is_modifier_vk(vk: u32) -> bool {
    (VK_LSHIFT..=VK_RMENU).contains(&vk) || vk == VK_LWIN || vk == VK_RWIN
}

fn is_left(vk: u32) -> bool {
    matches!(vk, VK_LSHIFT | VK_LCONTROL | VK_LMENU | VK_LWIN)
}

/// Whether `vk` is held: a group with no side bit at all counts as its left key, as synthetic events send it.
pub fn modifier_held(flags: u64, vk: u32) -> bool {
    let Some((_, group, side, other)) = MODS.iter().find(|(m, ..)| *m == vk) else {
        return false;
    };
    if flags & group == 0 {
        return false;
    }
    flags & side != 0 || (is_left(vk) && flags & other == 0)
}

pub struct KeyTransition {
    pub vk: u32,
    pub down: bool,
}

/// The key-ups and then key-downs that turn `old_flags` into `new_flags`.
pub fn modifier_transitions(old_flags: u64, new_flags: u64) -> Vec<KeyTransition> {
    let mut ups = Vec::new();
    let mut downs = Vec::new();
    for (vk, ..) in MODS {
        match (modifier_held(old_flags, *vk), modifier_held(new_flags, *vk)) {
            (true, false) => ups.push(KeyTransition { vk: *vk, down: false }),
            (false, true) => downs.push(KeyTransition { vk: *vk, down: true }),
            _ => {}
        }
    }
    ups.extend(downs);
    ups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycodes_map_to_vks() {
        assert_eq!(key_code_to_vk(0x23), 0x50);
        assert_eq!(key_code_to_vk(0x00), 0x41);
        assert_eq!(key_code_to_vk(0x1D), 0x30);
        assert_eq!(key_code_to_vk(0x35), 0x1B);
        assert_eq!(key_code_to_vk(0x7A), 0x70);
        assert_eq!(key_code_to_vk(0x5A), 0x83);
        assert_eq!(key_code_to_vk(0x7B), 0x25);
        assert_eq!(key_code_to_vk(0x3F), 0);
        assert_eq!(vk_to_key_code(0x50), Some(0x23));
        assert_eq!(vk_to_key_code(0x0D), Some(0x24));
        assert_eq!(vk_to_key_code(VK_LMENU), Some(0x3A));
        assert_eq!(vk_to_key_code(0x87), None);
    }

    #[test]
    fn every_keycode_round_trips() {
        for code in 0u16..0x80 {
            let vk = key_code_to_vk(code);
            if vk != 0 && code != 0x4C {
                assert_eq!(vk_to_key_code(vk), Some(code), "keycode {code:#x}");
            }
        }
    }

    #[test]
    fn sides_are_told_apart() {
        assert!(modifier_held(FLAG_OPTION | DEV_LALT, VK_LMENU));
        assert!(!modifier_held(FLAG_OPTION | DEV_LALT, VK_RMENU));
        assert!(modifier_held(FLAG_OPTION | DEV_RALT, VK_RMENU));
        assert!(modifier_held(FLAG_OPTION, VK_LMENU));
        assert!(!modifier_held(FLAG_OPTION, VK_RMENU));
        assert!(!modifier_held(DEV_LALT, VK_LMENU));
        assert!(modifier_held(FLAG_COMMAND | DEV_RCMD, VK_RWIN));
        assert!(modifier_held(FLAG_CONTROL | DEV_RCTRL, VK_RCONTROL));
        assert!(!modifier_held(FLAG_SHIFT | DEV_LSHIFT, 0x41));
    }

    #[test]
    fn transitions_put_the_ups_first() {
        let t = modifier_transitions(0, FLAG_OPTION | DEV_LALT);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].vk, t[0].down), (VK_LMENU, true));

        let t = modifier_transitions(FLAG_OPTION | DEV_LALT, 0);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].vk, t[0].down), (VK_LMENU, false));

        let t = modifier_transitions(FLAG_OPTION | DEV_LALT, FLAG_CONTROL | DEV_LCTRL);
        assert_eq!(t.len(), 2);
        assert_eq!((t[0].vk, t[0].down), (VK_LMENU, false));
        assert_eq!((t[1].vk, t[1].down), (VK_LCONTROL, true));

        let t = modifier_transitions(FLAG_COMMAND | DEV_LCMD, FLAG_COMMAND | DEV_LCMD | DEV_RCMD);
        assert_eq!(t.len(), 1);
        assert_eq!((t[0].vk, t[0].down), (VK_RWIN, true));

        assert!(modifier_transitions(FLAG_SHIFT | DEV_LSHIFT, FLAG_SHIFT | DEV_LSHIFT).is_empty());
    }

    #[test]
    fn modifier_vks_are_recognised() {
        for vk in [VK_LSHIFT, VK_RSHIFT, VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN] {
            assert!(is_modifier_vk(vk), "{vk:#x}");
        }
        assert!(!is_modifier_vk(0x41));
    }
}
