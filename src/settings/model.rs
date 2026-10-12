use std::collections::HashSet;

use serde::{Serialize, Serializer};
use serde_json::{Map, Value};

use crate::emotes::{
    custom_hash, is_custom_ref, normalize_click_emote, normalize_emote_wheel, CustomEmote, EmoteRef,
    CUSTOM_NAME_MAX, MAX_CUSTOM_EMOTES,
};
use crate::i18n::{Lang, LanguagePref, LANG_ZH_CN, LANGUAGE_PREFS};
use crate::keys::{vk_label, Hotkey, HOTKEY_PRIMARY_MODS, MOD_ALT, MOD_CTRL};
use crate::pings::{ping_by_id, DEFAULT_CLICK_PING, DEFAULT_WHEEL, WHEEL_SLOTS};
use crate::room::code::{parse_room_code, ParsedRoomCode};
use crate::room::colors::is_tag_color;
use crate::room::{sanitize_name, MAX_NAME};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum NamedTrigger {
    Alt,
    Ctrl,
    Shift,
    Win,
    Capslock,
    Mouse4,
    Mouse5,
}

pub const NAMED_TRIGGERS: [NamedTrigger; 7] = [
    NamedTrigger::Alt,
    NamedTrigger::Ctrl,
    NamedTrigger::Shift,
    NamedTrigger::Win,
    NamedTrigger::Capslock,
    NamedTrigger::Mouse4,
    NamedTrigger::Mouse5,
];

/// The keyboard triggers offered on Windows: macOS can't stop Caps Lock toggling, so it isn't in the mac list.
pub const KEYBOARD_TRIGGERS: [NamedTrigger; 5] = [
    NamedTrigger::Alt,
    NamedTrigger::Ctrl,
    NamedTrigger::Shift,
    NamedTrigger::Win,
    NamedTrigger::Capslock,
];

impl NamedTrigger {
    pub fn as_str(self) -> &'static str {
        match self {
            NamedTrigger::Alt => "alt",
            NamedTrigger::Ctrl => "ctrl",
            NamedTrigger::Shift => "shift",
            NamedTrigger::Win => "win",
            NamedTrigger::Capslock => "capslock",
            NamedTrigger::Mouse4 => "mouse4",
            NamedTrigger::Mouse5 => "mouse5",
        }
    }

    pub fn parse(s: &str) -> Option<NamedTrigger> {
        NAMED_TRIGGERS.iter().copied().find(|t| t.as_str() == s)
    }
}

impl Serialize for NamedTrigger {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TriggerKey {
    Named(NamedTrigger),
    Vk { vk: u32 },
}

impl Serialize for TriggerKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Vk {
            vk: u32,
        }
        match self {
            TriggerKey::Named(t) => t.serialize(s),
            TriggerKey::Vk { vk } => Vk { vk: *vk }.serialize(s),
        }
    }
}

/// The emote key is a trigger key, or Off: the emote wheel can't be opened then, nor can emote click.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EmoteTrigger {
    Off,
    Key(TriggerKey),
}

impl Serialize for EmoteTrigger {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            EmoteTrigger::Off => s.serialize_str("off"),
            EmoteTrigger::Key(k) => k.serialize(s),
        }
    }
}

/// Per-machine defaults for the room profile: the OS username and a random tag colour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub color: u32,
}

impl Default for Identity {
    fn default() -> Self {
        Self { name: "Player".to_string(), color: 0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub version: u32,
    pub enabled_on_start: bool,
    pub trigger: TriggerKey,
    pub drag_threshold_px: u32,
    pub click_ping: bool,
    pub click_ping_id: &'static str,
    pub wheel: Vec<&'static str>,
    pub toggle_hotkey: Hotkey,
    pub ping_size_px: u32,
    pub ping_duration_s: f64,
    pub volume: u32,
    pub muted: bool,
    pub tick_sound: bool,
    pub launch_at_startup: bool,
    #[serde(serialize_with = "serialize_lang")]
    pub language: LanguagePref,
    pub display_name: String,
    pub tag_color: u32,
    pub incoming_ping_limit: u32,
    pub allow_internet: bool,
    pub rejoin_room: bool,
    pub last_room_code: Option<String>,
    pub room_muted: bool,
    pub emote_trigger: EmoteTrigger,
    pub emote_click: bool,
    pub emote_wheel: Vec<EmoteRef>,
    pub click_emote_id: EmoteRef,
    pub emote_size_px: u32,
    pub emote_sound: bool,
    pub custom_emotes: Vec<CustomEmote>,
    pub auto_update_check: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            enabled_on_start: true,
            trigger: TriggerKey::Named(NamedTrigger::Alt),
            drag_threshold_px: 8,
            click_ping: false,
            click_ping_id: DEFAULT_CLICK_PING,
            wheel: DEFAULT_WHEEL.to_vec(),
            toggle_hotkey: Hotkey { mods: MOD_CTRL | MOD_ALT, vk: 0x50 },
            ping_size_px: 110,
            ping_duration_s: 3.2,
            volume: 70,
            muted: false,
            tick_sound: true,
            launch_at_startup: false,
            language: LanguagePref::Auto,
            display_name: "Player".to_string(),
            tag_color: 0,
            incoming_ping_limit: 5,
            allow_internet: true,
            rejoin_room: true,
            last_room_code: None,
            room_muted: false,
            emote_trigger: EmoteTrigger::Key(TriggerKey::Named(NamedTrigger::Ctrl)),
            emote_click: false,
            emote_wheel: crate::emotes::DEFAULT_EMOTE_WHEEL.iter().map(|s| s.to_string()).collect(),
            click_emote_id: crate::emotes::DEFAULT_CLICK_EMOTE.to_string(),
            emote_size_px: 150,
            emote_sound: true,
            custom_emotes: Vec::new(),
            auto_update_check: true,
        }
    }
}

#[derive(Clone, Copy)]
pub struct Limit {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

pub struct Limits {
    pub drag_threshold_px: Limit,
    pub ping_size_px: Limit,
    pub ping_duration_s: Limit,
    pub volume: Limit,
    pub incoming_ping_limit: Limit,
    pub emote_size_px: Limit,
}

pub const LIMITS: Limits = Limits {
    drag_threshold_px: Limit { min: 2.0, max: 40.0, step: 1.0 },
    ping_size_px: Limit { min: 60.0, max: 220.0, step: 1.0 },
    ping_duration_s: Limit { min: 1.0, max: 8.0, step: 0.1 },
    volume: Limit { min: 0.0, max: 100.0, step: 1.0 },
    incoming_ping_limit: Limit { min: 0.0, max: 20.0, step: 1.0 },
    emote_size_px: Limit { min: 80.0, max: 300.0, step: 1.0 },
};

fn serialize_lang<S: Serializer>(lang: &LanguagePref, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(lang_str(*lang))
}

fn lang_str(lang: LanguagePref) -> &'static str {
    match lang {
        LanguagePref::Auto => "auto",
        LanguagePref::En => "en",
        LanguagePref::ZhCn => "zh-CN",
    }
}

fn num(raw: Option<&Value>, limit: Limit, default: f64) -> f64 {
    let Some(v) = raw.and_then(Value::as_f64).filter(|v| v.is_finite()) else {
        return default;
    };
    let clamped = v.min(limit.max).max(limit.min);
    let rounded = (clamped / limit.step).round() * limit.step;
    let text = format!("{:.*}", if limit.step < 1.0 { 1 } else { 0 }, rounded);
    text.parse().unwrap_or(default)
}

fn as_int(raw: Option<&Value>) -> Option<i64> {
    let n = raw?.as_f64()?;
    (n.fract() == 0.0).then_some(n as i64)
}

fn is_vk(v: &Value) -> Option<u32> {
    let n = v.as_f64()?;
    (n.fract() == 0.0).then_some(n as i64).filter(|n| (1..=254).contains(n)).map(|n| n as u32)
}

fn trigger_key(raw: Option<&Value>, fallback: TriggerKey) -> TriggerKey {
    if let Some(named) = raw.and_then(Value::as_str).and_then(NamedTrigger::parse) {
        return TriggerKey::Named(named);
    }
    if let Some(vk) = raw.and_then(Value::as_object).and_then(|o| o.get("vk")).and_then(is_vk) {
        return TriggerKey::Vk { vk };
    }
    fallback
}

/// The emote key as stored, before the clash rules: Off, or a trigger key with Ctrl as the fallback.
fn emote_key(raw: Option<&Value>) -> EmoteTrigger {
    if raw.and_then(Value::as_str) == Some("off") {
        EmoteTrigger::Off
    } else {
        EmoteTrigger::Key(trigger_key(raw, TriggerKey::Named(NamedTrigger::Ctrl)))
    }
}

/// The Caps Lock key's code: the named trigger and a custom { vk: 0x14 } are one physical key.
const CAPS_LOCK_VK: u32 = 0x14;

/// The key code a trigger stands for when it is a custom key or Caps Lock; None for every other trigger.
pub fn trigger_vk(t: EmoteTrigger) -> Option<u32> {
    match t {
        EmoteTrigger::Key(TriggerKey::Vk { vk }) => Some(vk),
        EmoteTrigger::Key(TriggerKey::Named(NamedTrigger::Capslock)) => Some(CAPS_LOCK_VK),
        _ => None,
    }
}

pub fn same_trigger(a: EmoteTrigger, b: EmoteTrigger) -> bool {
    match (trigger_vk(a), trigger_vk(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

/// The ping trigger and the emote key can't be one key, and the emote key can't be the pause shortcut's key.
pub fn triggers_clash(ping: TriggerKey, emote: EmoteTrigger, toggle: Hotkey) -> bool {
    if emote == EmoteTrigger::Off {
        return false;
    }
    let vk = trigger_vk(emote);
    same_trigger(EmoteTrigger::Key(ping), emote) || vk.is_some_and(|v| v == toggle.vk)
}

/// Whether applying `patch` to `current` leaves those three keys clashing, judged as normalize will read them.
pub fn patch_clashes(current: &Settings, patch: &Map<String, Value>) -> bool {
    let base = serde_json::to_value(current).expect("Settings serializes");
    let pick = |k: &str| patch.get(k).or_else(|| base.get(k)).unwrap_or(&Value::Null);
    let trigger = trigger_key(Some(pick("trigger")), TriggerKey::Named(NamedTrigger::Alt));
    let emote = emote_key(Some(pick("emoteTrigger")));
    let toggle = hotkey(Some(pick("toggleHotkey")));
    triggers_clash(trigger, emote, toggle)
}

fn hotkey(raw: Option<&Value>) -> Hotkey {
    let default = Hotkey { mods: MOD_CTRL | MOD_ALT, vk: 0x50 };
    let Some(o) = raw.and_then(Value::as_object) else {
        return default;
    };
    let (Some(mods), Some(vk)) = (as_int(o.get("mods")), o.get("vk").and_then(is_vk)) else {
        return default;
    };
    // Needs Ctrl, Alt or Win: a Shift-only hotkey would swallow ordinary capital letters system-wide.
    if (1..=15).contains(&mods) && (mods as u32 & HOTKEY_PRIMARY_MODS) != 0 {
        Hotkey { mods: mods as u32, vk }
    } else {
        default
    }
}

fn room_code(raw: Option<&Value>) -> Option<String> {
    let text = raw.and_then(Value::as_str)?;
    match parse_room_code(text, false) {
        ParsedRoomCode::Valid(code) => Some(code),
        _ => None,
    }
}

fn language(raw: Option<&Value>) -> LanguagePref {
    let Some(text) = raw.and_then(Value::as_str) else {
        return LanguagePref::Auto;
    };
    LANGUAGE_PREFS.iter().copied().find(|p| lang_str(*p) == text).unwrap_or(LanguagePref::Auto)
}

fn tag_color(raw: Option<&Value>) -> Option<u32> {
    as_int(raw).filter(|n| is_tag_color(*n)).map(|n| n as u32)
}

/// Valid entries only, rebuilt field by field: well-formed unique ids, a boolean flag, a safe name.
fn custom_emotes(raw: Option<&Value>) -> Vec<CustomEmote> {
    let Some(items) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<CustomEmote> = Vec::new();
    for e in items {
        if out.len() >= MAX_CUSTOM_EMOTES {
            break;
        }
        let Some(o) = e.as_object() else { continue };
        let Some(id) = o.get("id").and_then(Value::as_str) else { continue };
        if !is_custom_ref(id) {
            continue;
        }
        let Some(name) = o.get("name").and_then(Value::as_str) else { continue };
        let Some(animated) = o.get("animated").and_then(Value::as_bool) else { continue };
        if out.iter().any(|c| c.id == id) {
            continue;
        }
        // A name with nothing left shows the first characters of its id, which reads the same in every language.
        let hash = custom_hash(id);
        let fallback = &hash[..hash.len().min(6)];
        let name: String = name.chars().take(256).collect();
        out.push(CustomEmote { id: id.to_string(), name: sanitize_name(&name, fallback, CUSTOM_NAME_MAX), animated });
    }
    out
}

fn is_valid_wheel(v: &Value) -> bool {
    let Some(items) = v.as_array() else { return false };
    if items.len() != WHEEL_SLOTS {
        return false;
    }
    let mut seen = HashSet::with_capacity(WHEEL_SLOTS);
    items.iter().all(|item| item.as_str().and_then(ping_by_id).is_some_and(|p| seen.insert(p.id)))
}

fn wheel_ids(v: &Value) -> Vec<&'static str> {
    v.as_array().into_iter().flatten().filter_map(|i| i.as_str().and_then(ping_by_id)).map(|p| p.id).collect()
}

fn ping_id(raw: Option<&Value>) -> Option<&'static str> {
    raw?.as_str().and_then(ping_by_id).map(|p| p.id)
}

/// Turns anything (parsed JSON, IPC payloads) into a complete, valid Settings object.
pub fn normalize(raw: &Value, identity: &Identity) -> Settings {
    let empty = Map::new();
    let r = raw.as_object().unwrap_or(&empty);
    let get = |k: &str| r.get(k);

    let fallback_name = sanitize_name(&identity.name, "Player", MAX_NAME);
    let ping_trigger = trigger_key(get("trigger"), TriggerKey::Named(NamedTrigger::Alt));
    let toggle_hotkey = hotkey(get("toggleHotkey"));
    let emote = emote_key(get("emoteTrigger"));
    let custom = custom_emotes(get("customEmotes"));
    Settings {
        version: 1,
        enabled_on_start: bool_of(get("enabledOnStart"), true),
        trigger: ping_trigger,
        drag_threshold_px: num(get("dragThresholdPx"), LIMITS.drag_threshold_px, 8.0) as u32,
        click_ping: bool_of(get("clickPing"), false),
        click_ping_id: ping_id(get("clickPingId")).unwrap_or(DEFAULT_CLICK_PING),
        wheel: match get("wheel") {
            Some(v) if is_valid_wheel(v) => wheel_ids(v),
            _ => DEFAULT_WHEEL.to_vec(),
        },
        toggle_hotkey,
        ping_size_px: num(get("pingSizePx"), LIMITS.ping_size_px, 110.0) as u32,
        ping_duration_s: num(get("pingDurationS"), LIMITS.ping_duration_s, 3.2),
        volume: num(get("volume"), LIMITS.volume, 70.0) as u32,
        muted: bool_of(get("muted"), false),
        tick_sound: bool_of(get("tickSound"), true),
        launch_at_startup: bool_of(get("launchAtStartup"), false),
        language: language(get("language")),
        display_name: match get("displayName").and_then(Value::as_str) {
            Some(name) => sanitize_name(name, &fallback_name, MAX_NAME),
            None => fallback_name,
        },
        tag_color: tag_color(get("tagColor")).unwrap_or_else(|| if is_tag_color(identity.color as i64) { identity.color } else { 0 }),
        incoming_ping_limit: num(get("incomingPingLimit"), LIMITS.incoming_ping_limit, 5.0) as u32,
        allow_internet: bool_of(get("allowInternet"), true),
        rejoin_room: bool_of(get("rejoinRoom"), true),
        last_room_code: room_code(get("lastRoomCode")),
        room_muted: bool_of(get("roomMuted"), false),
        emote_trigger: if triggers_clash(ping_trigger, emote, toggle_hotkey) { EmoteTrigger::Off } else { emote },
        emote_click: bool_of(get("emoteClick"), false),
        emote_wheel: normalize_emote_wheel(get("emoteWheel").unwrap_or(&Value::Null), &custom),
        click_emote_id: normalize_click_emote(get("clickEmoteId").unwrap_or(&Value::Null), &custom),
        emote_size_px: num(get("emoteSizePx"), LIMITS.emote_size_px, 150.0) as u32,
        emote_sound: bool_of(get("emoteSound"), true),
        custom_emotes: custom,
        auto_update_check: bool_of(get("autoUpdateCheck"), true),
    }
}

fn bool_of(raw: Option<&Value>, default: bool) -> bool {
    raw.and_then(Value::as_bool).unwrap_or(default)
}

pub fn merge(current: &Settings, patch: &Map<String, Value>, identity: &Identity) -> Settings {
    let mut base = serde_json::to_value(current).expect("Settings serializes");
    if let Some(o) = base.as_object_mut() {
        for (k, v) in patch {
            o.insert(k.clone(), v.clone());
        }
    }
    normalize(&base, identity)
}

/// A patch from the settings window: imported emotes are left out, as they change only through importing and removing.
pub fn renderer_patch(raw: &Value) -> Map<String, Value> {
    let Some(o) = raw.as_object() else {
        return Map::new();
    };
    let mut patch = o.clone();
    patch.remove("customEmotes");
    patch
}

pub fn trigger_label(t: &TriggerKey, lang: Lang) -> String {
    match t {
        TriggerKey::Vk { vk } => vk_label(*vk),
        TriggerKey::Named(NamedTrigger::Mouse4) => mouse_label(lang, 4),
        TriggerKey::Named(NamedTrigger::Mouse5) => mouse_label(lang, 5),
        TriggerKey::Named(NamedTrigger::Alt) => "Alt".to_string(),
        TriggerKey::Named(NamedTrigger::Ctrl) => "Ctrl".to_string(),
        TriggerKey::Named(NamedTrigger::Shift) => "Shift".to_string(),
        TriggerKey::Named(NamedTrigger::Win) => "Win".to_string(),
        TriggerKey::Named(NamedTrigger::Capslock) => "Caps Lock".to_string(),
    }
}

fn mouse_label(lang: Lang, n: u32) -> String {
    if lang == LANG_ZH_CN {
        format!("鼠标侧键 {n}")
    } else {
        format!("Mouse {n}")
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverlaySettings {
    pub ping_size_px: u32,
    pub ping_duration_s: f64,
    pub volume: u32,
    pub muted: bool,
    pub tick_sound: bool,
    pub wheel: Vec<&'static str>,
    pub click_ping_id: &'static str,
    pub emote_wheel: Vec<EmoteRef>,
    pub click_emote_id: EmoteRef,
    pub emote_size_px: u32,
    pub emote_sound: bool,
    pub custom_emotes: Vec<CustomEmote>,
}

pub fn overlay_settings(s: &Settings) -> OverlaySettings {
    OverlaySettings {
        ping_size_px: s.ping_size_px,
        ping_duration_s: s.ping_duration_s,
        volume: s.volume,
        muted: s.muted,
        tick_sound: s.tick_sound,
        wheel: s.wheel.clone(),
        click_ping_id: s.click_ping_id,
        emote_wheel: s.emote_wheel.clone(),
        click_emote_id: s.click_emote_id.clone(),
        emote_size_px: s.emote_size_px,
        emote_sound: s.emote_sound,
        custom_emotes: s.custom_emotes.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::emotes::{DEFAULT_CLICK_EMOTE, DEFAULT_EMOTE_WHEEL};
    use serde_json::json;

    fn identity() -> Identity {
        Identity::default()
    }

    fn n(v: Value) -> Settings {
        normalize(&v, &identity())
    }

    #[test]
    fn defaults_round_trip() {
        let defaults = Settings::default();
        let value = serde_json::to_value(&defaults).unwrap();
        assert_eq!(normalize(&value, &identity()), defaults);

        let keys: Vec<&str> = value.as_object().unwrap().keys().map(String::as_str).collect();
        assert!(keys.contains(&"pingDurationS"));
        assert!(keys.contains(&"emoteTrigger"));
        assert!(keys.contains(&"autoUpdateCheck"));
        assert!(!keys.contains(&"updateNotifiedVersion"));
    }

    #[test]
    fn clamps_every_numeric_limit() {
        assert_eq!(n(json!({ "dragThresholdPx": 1 })).drag_threshold_px, 2);
        assert_eq!(n(json!({ "dragThresholdPx": 999 })).drag_threshold_px, 40);
        assert_eq!(n(json!({ "dragThresholdPx": 8.6 })).drag_threshold_px, 9);

        assert_eq!(n(json!({ "pingSizePx": 10 })).ping_size_px, 60);
        assert_eq!(n(json!({ "pingSizePx": 9999 })).ping_size_px, 220);

        assert_eq!(n(json!({ "pingDurationS": 0.1 })).ping_duration_s, 1.0);
        assert_eq!(n(json!({ "pingDurationS": 99 })).ping_duration_s, 8.0);
        assert_eq!(n(json!({ "pingDurationS": 3.24 })).ping_duration_s, 3.2);
        assert_eq!(n(json!({ "pingDurationS": 3.26 })).ping_duration_s, 3.3);

        assert_eq!(n(json!({ "volume": -5 })).volume, 0);
        assert_eq!(n(json!({ "volume": 1000 })).volume, 100);

        assert_eq!(n(json!({ "incomingPingLimit": -1 })).incoming_ping_limit, 0);
        assert_eq!(n(json!({ "incomingPingLimit": 21 })).incoming_ping_limit, 20);

        assert_eq!(n(json!({ "emoteSizePx": 1 })).emote_size_px, 80);
        assert_eq!(n(json!({ "emoteSizePx": 301 })).emote_size_px, 300);

        assert_eq!(n(json!({ "volume": "loud" })).volume, 70);
        assert_eq!(n(json!({ "pingDurationS": null })).ping_duration_s, 3.2);
    }

    #[test]
    fn garbage_fields_fall_back() {
        let s = n(json!({
            "trigger": "bogus",
            "emoteTrigger": { "nope": 1 },
            "language": "xx",
            "tagColor": 99,
            "lastRoomCode": "not a code",
            "toggleHotkey": { "mods": 4, "vk": 0 },
            "wheel": ["danger"],
            "clickPingId": "nope",
            "displayName": 7,
            "unknownField": { "a": 1 },
        }));
        assert_eq!(s.trigger, TriggerKey::Named(NamedTrigger::Alt));
        assert_eq!(s.emote_trigger, EmoteTrigger::Key(TriggerKey::Named(NamedTrigger::Ctrl)));
        assert_eq!(s.language, LanguagePref::Auto);
        assert_eq!(s.tag_color, 0);
        assert_eq!(s.last_room_code, None);
        assert_eq!(s.toggle_hotkey, Hotkey { mods: MOD_CTRL | MOD_ALT, vk: 0x50 });
        assert_eq!(s.wheel, DEFAULT_WHEEL.to_vec());
        assert_eq!(s.click_ping_id, DEFAULT_CLICK_PING);
        assert_eq!(s.display_name, "Player");
    }

    #[test]
    fn identity_supplies_the_machine_defaults() {
        let machine = Identity { name: "小明".to_string(), color: 5 };
        let s = normalize(&Value::Null, &machine);
        assert_eq!(s.display_name, "小明");
        assert_eq!(s.tag_color, 5);

        let s = normalize(&json!({ "displayName": "Ann", "tagColor": 2 }), &machine);
        assert_eq!(s.display_name, "Ann");
        assert_eq!(s.tag_color, 2);
    }

    #[test]
    fn trigger_clashes_turn_the_emote_key_off() {
        assert_eq!(n(json!({ "trigger": "alt", "emoteTrigger": "alt" })).emote_trigger, EmoteTrigger::Off);
        assert_eq!(
            n(json!({ "trigger": "alt", "emoteTrigger": "ctrl" })).emote_trigger,
            EmoteTrigger::Key(TriggerKey::Named(NamedTrigger::Ctrl))
        );
        // Caps Lock and { vk: 0x14 } are one physical key
        assert_eq!(n(json!({ "trigger": { "vk": 20 }, "emoteTrigger": "capslock" })).emote_trigger, EmoteTrigger::Off);
        assert_eq!(n(json!({ "trigger": "capslock", "emoteTrigger": { "vk": 20 } })).emote_trigger, EmoteTrigger::Off);
        // the pause shortcut's key
        assert_eq!(
            n(json!({ "toggleHotkey": { "mods": 3, "vk": 66 }, "emoteTrigger": { "vk": 66 } })).emote_trigger,
            EmoteTrigger::Off
        );
        // a mouse trigger is never the shortcut's key
        assert_eq!(
            n(json!({ "trigger": "mouse4", "emoteTrigger": "mouse5", "toggleHotkey": { "mods": 3, "vk": 66 } })).emote_trigger,
            EmoteTrigger::Key(TriggerKey::Named(NamedTrigger::Mouse5))
        );
    }

    #[test]
    fn patch_clashes_judges_the_merged_keys() {
        let current = Settings::default();
        let mut patch = Map::new();

        patch.insert("emoteTrigger".to_string(), json!("alt"));
        assert!(patch_clashes(&current, &patch));

        patch.insert("emoteTrigger".to_string(), json!("shift"));
        assert!(!patch_clashes(&current, &patch));

        patch.insert("emoteTrigger".to_string(), json!({ "vk": 0x50 }));
        assert!(patch_clashes(&current, &patch));

        patch.insert("emoteTrigger".to_string(), json!("off"));
        assert!(!patch_clashes(&current, &patch));
    }

    #[test]
    fn wheel_normalises_a_gone_custom_emote() {
        let gone = "c:00000000000000000000000000000000";
        let mut wheel: Vec<String> = DEFAULT_EMOTE_WHEEL.iter().map(|s| s.to_string()).collect();
        wheel[0] = gone.to_string();
        let s = n(json!({ "emoteWheel": wheel }));
        assert_eq!(s.emote_wheel.len(), WHEEL_SLOTS);
        assert_eq!(s.emote_wheel[0], DEFAULT_EMOTE_WHEEL[0]);

        // an imported emote that is still there stays where it is
        let kept = "c:11111111111111111111111111111111";
        let mut wheel: Vec<String> = DEFAULT_EMOTE_WHEEL.iter().map(|s| s.to_string()).collect();
        wheel[1] = kept.to_string();
        let s = n(json!({
            "emoteWheel": wheel,
            "customEmotes": [{ "id": kept, "name": "mine", "animated": false }],
            "clickEmoteId": kept,
        }));
        assert_eq!(s.emote_wheel[1], kept);
        assert_eq!(s.click_emote_id, kept);
    }

    #[test]
    fn a_reference_file_loads_unchanged() {
        let raw: Value = serde_json::from_str(REFERENCE_FILE).unwrap();
        let loaded = normalize(&raw, &identity());
        assert_eq!(loaded, reference_settings());

        let written = serde_json::to_value(&loaded).unwrap();
        assert_eq!(normalize(&written, &identity()), loaded);
    }

    fn reference_settings() -> Settings {
        Settings {
            version: 1,
            enabled_on_start: false,
            trigger: TriggerKey::Named(NamedTrigger::Mouse4),
            drag_threshold_px: 20,
            click_ping: true,
            click_ping_id: "omw",
            wheel: DEFAULT_WHEEL.to_vec(),
            toggle_hotkey: Hotkey { mods: MOD_CTRL | MOD_ALT, vk: 0x50 },
            ping_size_px: 150,
            ping_duration_s: 2.5,
            volume: 40,
            muted: true,
            tick_sound: false,
            launch_at_startup: true,
            language: LanguagePref::ZhCn,
            display_name: "好友".to_string(),
            tag_color: 3,
            incoming_ping_limit: 10,
            allow_internet: false,
            rejoin_room: false,
            last_room_code: Some("3F8K29WXY8".to_string()),
            room_muted: true,
            emote_trigger: EmoteTrigger::Key(TriggerKey::Vk { vk: 66 }),
            emote_click: true,
            emote_wheel: DEFAULT_EMOTE_WHEEL.iter().map(|s| s.to_string()).collect(),
            click_emote_id: DEFAULT_CLICK_EMOTE.to_string(),
            emote_size_px: 200,
            emote_sound: false,
            custom_emotes: vec![CustomEmote {
                id: "c:0123456789abcdef0123456789abcdef".to_string(),
                name: "我的表情".to_string(),
                animated: false,
            }],
            auto_update_check: false,
        }
    }

    const REFERENCE_FILE: &str = r#"{
  "version": 1,
  "enabledOnStart": false,
  "trigger": "mouse4",
  "dragThresholdPx": 20,
  "clickPing": true,
  "clickPingId": "omw",
  "wheel": ["danger", "push", "omw", "allin", "assist", "needvision", "missing", "enemyvision"],
  "toggleHotkey": { "mods": 3, "vk": 80 },
  "pingSizePx": 150,
  "pingDurationS": 2.5,
  "volume": 40,
  "muted": true,
  "tickSound": false,
  "launchAtStartup": true,
  "language": "zh-CN",
  "displayName": "好友",
  "tagColor": 3,
  "incomingPingLimit": 10,
  "allowInternet": false,
  "rejoinRoom": false,
  "lastRoomCode": "PING-3F8K2-9WXY8",
  "roomMuted": true,
  "emoteTrigger": { "vk": 66 },
  "emoteClick": true,
  "emoteWheel": ["unworthy_cn", "poro_snax", "sad_kitten", "cheeky_poro", "angry_kitty", "gg_heart", "bee_happy", "bee_mad"],
  "clickEmoteId": "facepalm",
  "emoteSizePx": 200,
  "emoteSound": false,
  "customEmotes": [{ "id": "c:0123456789abcdef0123456789abcdef", "name": "我的表情", "animated": false }],
  "autoUpdateCheck": false,
  "updateNotifiedVersion": "0.5.0"
}"#;
}
