pub struct PingDef {
    pub id: &'static str,
    pub name: &'static str,
    pub icon: &'static str,
    /// Two entries = All In's clashing halves.
    pub art: &'static [&'static str],
    pub sound: &'static str,
    pub color: u32,
}

pub const ALL_PINGS: &[PingDef] = &[
    PingDef { id: "danger", name: "Danger", icon: "pingwheel_retreatrender", art: &["pingwheel_retreatrender"], sound: "SRP_12", color: 0xff3b55 },
    PingDef { id: "push", name: "Push", icon: "push", art: &["pingwheel_pushrender"], sound: "SRP_9", color: 0x35d27a },
    PingDef { id: "omw", name: "On My Way", icon: "omw", art: &["omw"], sound: "OnMyWay", color: 0x2e8bff },
    PingDef { id: "allin", name: "All In", icon: "all_in", art: &["pingwheel_allin_render_01", "pingwheel_allin_render_02"], sound: "SRP_4", color: 0xf2b320 },
    PingDef { id: "assist", name: "Assist Me", icon: "assist_me", art: &["assist_me"], sound: "ComeHere", color: 0x56d43c },
    PingDef { id: "needvision", name: "Need Vision", icon: "need_vision", art: &["need_vision"], sound: "SRP_7", color: 0x56d43c },
    PingDef { id: "missing", name: "Enemy Missing", icon: "enemy_missing", art: &["enemy_missing"], sound: "MIA", color: 0xf2c518 },
    PingDef { id: "enemyvision", name: "Enemy Vision", icon: "enemy_vision", art: &["pingwheel_enemyvisionrender"], sound: "AreaIsWarded", color: 0xff3b55 },
    PingDef { id: "generic", name: "Generic", icon: "generic_ping", art: &["pingwheel_basicpingrender"], sound: "Base", color: 0x2e8bff },
    PingDef { id: "bait", name: "Bait", icon: "pingwheel_baitrender", art: &["pingwheel_baitrender"], sound: "SRP_11", color: 0xefbc12 },
    PingDef { id: "visioncleared", name: "Vision Cleared", icon: "vision_cleared", art: &["pingwheel_visionclearedrender"], sound: "SRP_6", color: 0x2b80ff },
];

/// The wheel as League ships it, top (N) then clockwise.
pub const DEFAULT_WHEEL: [&str; 8] = [
    "danger",
    "push",
    "omw",
    "allin",
    "assist",
    "needvision",
    "missing",
    "enemyvision",
];

pub const DEFAULT_CLICK_PING: &str = "generic";
pub const WHEEL_SLOTS: usize = 8;
pub const TICK_SOUND: &str = "button";
pub const FALLBACK_TEXTURE: &str = "generic_ping";

pub fn ping_by_id(id: &str) -> Option<&'static PingDef> {
    ALL_PINGS.iter().find(|p| p.id == id)
}

pub fn is_ping_id(id: &str) -> bool {
    ping_by_id(id).is_some()
}

pub fn all_sound_names() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for p in ALL_PINGS {
        if !out.contains(&p.sound) {
            out.push(p.sound);
        }
    }
    if !out.contains(&TICK_SOUND) {
        out.push(TICK_SOUND);
    }
    out
}

pub fn all_texture_names() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for p in ALL_PINGS {
        for name in std::iter::once(p.icon).chain(p.art.iter().copied()) {
            if !out.contains(&name) {
                out.push(name);
            }
        }
    }
    if !out.contains(&FALLBACK_TEXTURE) {
        out.push(FALLBACK_TEXTURE);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_lookups_agree() {
        for p in ALL_PINGS {
            assert_eq!(ping_by_id(p.id).unwrap().id, p.id);
            assert!(is_ping_id(p.id));
        }
        assert!(!is_ping_id("Vision Cleared"));
        assert_eq!(ALL_PINGS.len(), 11);
        assert_eq!(DEFAULT_WHEEL.len(), WHEEL_SLOTS);
        assert!(DEFAULT_WHEEL.iter().all(|id| is_ping_id(id)));
        assert!(is_ping_id(DEFAULT_CLICK_PING));
    }

    #[test]
    fn asset_names_are_unique() {
        let sounds = all_sound_names();
        assert!(sounds.contains(&TICK_SOUND));
        assert_eq!(sounds.len(), sounds.iter().collect::<std::collections::HashSet<_>>().len());
        let textures = all_texture_names();
        assert!(textures.contains(&FALLBACK_TEXTURE));
        assert_eq!(textures.len(), textures.iter().collect::<std::collections::HashSet<_>>().len());
        assert_eq!(textures.iter().filter(|t| **t == "pingwheel_allin_render_01").count(), 1);
    }
}
