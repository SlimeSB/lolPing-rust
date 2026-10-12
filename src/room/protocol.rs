use serde_json::{json, Value};

use crate::emotes::is_emote_ref_shape;
use crate::pings::is_ping_id;
use crate::room::colors::is_tag_color;
use crate::room::sanitize_name;

pub use crate::room::MAX_NAME;

/// Bumped when room messages change incompatibly: peers on another version are shown as "needs update".
pub const PROTOCOL_VERSION: u32 = 1;
/// Display numbers: 1 is the primary display, up to this many are addressable.
pub const MAX_DISPLAY: u32 = 16;
const MAX_APP_VERSION: usize = 64;
const MAX_NAME_INPUT: usize = 256;
/// A JSON number is an IEEE double: past this an integer is no longer exact, and the reference rejects it.
const MAX_SAFE: f64 = 9_007_199_254_740_991.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Status {
    On,
    Paused,
    Muted,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::On => "on",
            Status::Paused => "paused",
            Status::Muted => "muted",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        match s {
            "on" => Some(Status::On),
            "paused" => Some(Status::Paused),
            "muted" => Some(Status::Muted),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RoomMessage {
    Presence { peer: String, seq: u64, ts: u64, proto: u32, app: String, name: String, color: u32, status: Status },
    Ping { peer: String, seq: u64, ts: u64, ping: String, d: u32, x: f64, y: f64 },
    Emote { peer: String, seq: u64, ts: u64, e: String, d: u32, x: f64, y: f64 },
    Bye { peer: String, seq: u64, ts: u64 },
}

impl RoomMessage {
    pub fn peer(&self) -> &str {
        match self {
            RoomMessage::Presence { peer, .. } | RoomMessage::Ping { peer, .. } | RoomMessage::Emote { peer, .. } | RoomMessage::Bye { peer, .. } => peer,
        }
    }

    pub fn seq(&self) -> u64 {
        match self {
            RoomMessage::Presence { seq, .. } | RoomMessage::Ping { seq, .. } | RoomMessage::Emote { seq, .. } | RoomMessage::Bye { seq, .. } => *seq,
        }
    }

    pub fn ts(&self) -> u64 {
        match self {
            RoomMessage::Presence { ts, .. } | RoomMessage::Ping { ts, .. } | RoomMessage::Emote { ts, .. } | RoomMessage::Bye { ts, .. } => *ts,
        }
    }
}

/// Compact UTF-8 JSON, with a ping's or emote's x and y rounded to 4 decimals.
pub fn encode_message(m: &RoomMessage) -> Vec<u8> {
    let out = match m {
        RoomMessage::Presence { peer, seq, ts, proto, app, name, color, status } => json!({
            "t": "presence", "peer": peer, "seq": seq, "ts": ts,
            "proto": proto, "app": app, "name": name, "color": color, "status": status.as_str(),
        }),
        RoomMessage::Ping { peer, seq, ts, ping, d, x, y } => json!({
            "t": "ping", "peer": peer, "seq": seq, "ts": ts,
            "ping": ping, "d": d, "x": round4(*x), "y": round4(*y),
        }),
        RoomMessage::Emote { peer, seq, ts, e, d, x, y } => json!({
            "t": "emote", "peer": peer, "seq": seq, "ts": ts,
            "e": e, "d": d, "x": round4(*x), "y": round4(*y),
        }),
        RoomMessage::Bye { peer, seq, ts } => json!({ "t": "bye", "peer": peer, "seq": seq, "ts": ts }),
    };
    serde_json::to_vec(&out).expect("a JSON value always serializes")
}

pub fn decode_message(bytes: &[u8]) -> Option<RoomMessage> {
    validate_message(&serde_json::from_slice(bytes).ok()?)
}

/// Builds a fresh message from known fields only, or None when anything is missing or out of range.
pub fn validate_message(raw: &Value) -> Option<RoomMessage> {
    let obj = raw.as_object()?;
    let peer = obj.get("peer")?.as_str()?;
    if !is_peer(peer) {
        return None;
    }
    let seq = uint(obj.get("seq")?)?;
    let ts = uint(obj.get("ts")?)?;
    let peer = peer.to_string();
    match obj.get("t")?.as_str()? {
        "presence" => {
            let proto = uint(obj.get("proto")?)?;
            let app = obj.get("app")?.as_str()?;
            if proto > u32::MAX as u64 || app.chars().count() > MAX_APP_VERSION {
                return None;
            }
            let name = obj.get("name")?.as_str()?;
            let color = uint(obj.get("color")?)?;
            if !is_tag_color(color as i64) {
                return None;
            }
            let status = Status::parse(obj.get("status")?.as_str()?)?;
            let name: String = name.chars().take(MAX_NAME_INPUT).collect();
            Some(RoomMessage::Presence {
                peer,
                seq,
                ts,
                proto: proto as u32,
                app: app.to_string(),
                name: sanitize_name(&name, "Player", MAX_NAME),
                color: color as u32,
                status,
            })
        }
        "ping" => {
            let ping = obj.get("ping")?.as_str()?;
            if !is_ping_id(ping) {
                return None;
            }
            let (d, x, y) = display_and_spot(obj)?;
            Some(RoomMessage::Ping { peer, seq, ts, ping: ping.to_string(), d, x, y })
        }
        "emote" => {
            let e = obj.get("e")?.as_str()?;
            if !is_emote_ref_shape(e) {
                return None;
            }
            let (d, x, y) = display_and_spot(obj)?;
            Some(RoomMessage::Emote { peer, seq, ts, e: e.to_string(), d, x, y })
        }
        "bye" => Some(RoomMessage::Bye { peer, seq, ts }),
        _ => None,
    }
}

/// True only for a plain x.y.z version lower than `version`: a dev tool or a prerelease is never called old.
pub fn is_older_than(app: &str, version: &str) -> bool {
    let (Some(a), Some(v)) = (plain_version(app), plain_version(version)) else {
        return false;
    };
    for i in 0..3 {
        if a[i] != v[i] {
            return a[i] < v[i];
        }
    }
    false
}

fn display_and_spot(obj: &serde_json::Map<String, Value>) -> Option<(u32, f64, f64)> {
    let d = uint(obj.get("d")?)?;
    if !(1..=MAX_DISPLAY as u64).contains(&d) {
        return None;
    }
    let x = unit(obj.get("x")?)?;
    let y = unit(obj.get("y")?)?;
    Some((d as u32, x, y))
}

fn is_peer(s: &str) -> bool {
    s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The reference's `Number.isSafeInteger` and `>= 0`: integral, non-negative and exactly representable.
fn uint(v: &Value) -> Option<u64> {
    let f = v.as_f64()?;
    ((0.0..=MAX_SAFE).contains(&f) && f.fract() == 0.0).then_some(f as u64)
}

/// A position or fraction: a JSON number inside 0..=1.
fn unit(v: &Value) -> Option<f64> {
    let f = v.as_f64()?;
    (0.0..=1.0).contains(&f).then_some(f)
}

fn round4(v: f64) -> f64 {
    (v * 10_000.0).round() / 10_000.0
}

/// Three decimal groups of digits, in the JS `Number` sense so "0.10.0" sorts above "0.9.0".
fn plain_version(s: &str) -> Option<[f64; 3]> {
    let digits = |p: &str| -> Option<f64> { (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())).then(|| p.parse().ok())? };
    let mut parts = s.split('.');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    Some([digits(a)?, digits(b)?, digits(c)?])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::room::crypto::{open, seal};
    use serde_json::Map;

    fn with(base: Value, extra: Value) -> Value {
        let mut obj: Map<String, Value> = base.as_object().unwrap().clone();
        obj.extend(extra.as_object().unwrap().iter().map(|(k, v)| (k.clone(), v.clone())));
        Value::Object(obj)
    }

    fn presence(extra: Value) -> Value {
        let base = json!({
            "t": "presence", "peer": "0123456789abcdef", "seq": 4, "ts": 1_780_000_000_000u64,
            "proto": 1, "app": "0.3.0", "name": "Alex", "color": 3, "status": "on",
        });
        if extra.is_null() {
            base
        } else {
            with(base, extra)
        }
    }

    fn ping(extra: Value) -> Value {
        let base = json!({
            "t": "ping", "peer": "0123456789abcdef", "seq": 4, "ts": 1_780_000_000_000u64,
            "ping": "danger", "d": 2, "x": 0.25, "y": 1,
        });
        with(base, extra)
    }

    fn emote(extra: Value) -> Value {
        let base = json!({
            "t": "emote", "peer": "0123456789abcdef", "seq": 4, "ts": 1_780_000_000_000u64,
            "e": "gg", "d": 2, "x": 0.25, "y": 1,
        });
        with(base, extra)
    }

    fn base_message(t: &str) -> RoomMessage {
        match t {
            "presence" => RoomMessage::Presence {
                peer: "0123456789abcdef".into(),
                seq: 4,
                ts: 1_780_000_000_000,
                proto: 1,
                app: "0.3.0".into(),
                name: "Alex".into(),
                color: 3,
                status: Status::On,
            },
            "ping" => RoomMessage::Ping { peer: "0123456789abcdef".into(), seq: 4, ts: 1_780_000_000_000, ping: "danger".into(), d: 2, x: 0.25, y: 1.0 },
            "emote" => RoomMessage::Emote { peer: "0123456789abcdef".into(), seq: 4, ts: 1_780_000_000_000, e: "gg".into(), d: 2, x: 0.25, y: 1.0 },
            _ => RoomMessage::Bye { peer: "0123456789abcdef".into(), seq: 4, ts: 1_780_000_000_000 },
        }
    }

    #[test]
    fn round_trips_every_kind() {
        for t in ["presence", "ping", "emote", "bye"] {
            let m = base_message(t);
            assert_eq!(decode_message(&encode_message(&m)), Some(m), "{t}");
        }
    }

    #[test]
    fn rounds_x_and_y_to_4_decimals() {
        for t in ["ping", "emote"] {
            let mut m = base_message(t);
            match &mut m {
                RoomMessage::Ping { x, y, .. } | RoomMessage::Emote { x, y, .. } => {
                    *x = 0.123456;
                    *y = 0.987654;
                }
                _ => unreachable!(),
            }
            let decoded = decode_message(&encode_message(&m)).unwrap();
            let (x, y) = match decoded {
                RoomMessage::Ping { x, y, .. } | RoomMessage::Emote { x, y, .. } => (x, y),
                _ => unreachable!(),
            };
            assert_eq!((x, y), (0.1235, 0.9877), "{t}");
        }
    }

    #[test]
    fn rejects_bytes_that_are_not_json() {
        assert!(decode_message(b"{nope").is_none());
        assert!(decode_message(&[0xff, 0xfe]).is_none());
    }

    #[test]
    fn rejects_out_of_range_messages() {
        let cases: [(&str, Value); 22] = [
            ("unknown type", presence(json!({ "t": "hello" }))),
            ("short peer", presence(json!({ "peer": "abc" }))),
            ("upper-case peer", presence(json!({ "peer": "0123456789ABCDEF" }))),
            ("negative seq", presence(json!({ "seq": -1 }))),
            ("fractional seq", presence(json!({ "seq": 1.5 }))),
            ("unsafe seq", presence(json!({ "seq": 1_152_921_504_606_846_976u64 }))),
            ("fractional ts", presence(json!({ "ts": 1_780_000_000_000.5 }))),
            ("unknown ping", ping(json!({ "ping": "hold" }))),
            ("display 0", ping(json!({ "d": 0 }))),
            ("display 17", ping(json!({ "d": 17 }))),
            ("fractional display", ping(json!({ "d": 1.5 }))),
            ("x below 0", ping(json!({ "x": -0.01 }))),
            ("x above 1", ping(json!({ "x": 1.01 }))),
            ("y over the top", ping(json!({ "y": 1e308 }))),
            ("string x", ping(json!({ "x": "0.5" }))),
            ("colour 8", presence(json!({ "color": 8 }))),
            ("colour -1", presence(json!({ "color": -1 }))),
            ("colour as CSS", presence(json!({ "color": "red" }))),
            ("unknown status", presence(json!({ "status": "away" }))),
            ("huge app version", presence(json!({ "app": "x".repeat(65) }))),
            ("name not a string", presence(json!({ "name": 42 }))),
            ("fractional proto", presence(json!({ "proto": 1.5 }))),
        ];
        for (label, raw) in cases {
            assert!(validate_message(&raw).is_none(), "{label}");
        }
    }

    #[test]
    fn rejects_non_objects() {
        let presence = presence(Value::Null);
        for raw in [Value::Null, json!(3), json!("x"), json!([]), json!([presence])] {
            assert!(validate_message(&raw).is_none(), "{raw}");
        }
    }

    #[test]
    fn keeps_known_fields_only() {
        let expected = base_message("ping");
        assert_eq!(validate_message(&ping(json!({ "extra": "ignored" }))), Some(expected));
    }

    #[test]
    fn accepts_a_presence_from_a_newer_protocol() {
        let raw = presence(json!({ "proto": 2 }));
        let Some(RoomMessage::Presence { proto, .. }) = validate_message(&raw) else {
            panic!("rejected");
        };
        assert_eq!(proto, 2);
    }

    #[test]
    fn sanitises_the_name() {
        let raw = presence(json!({ "name": "\u{202e}Alex\u{0}" }));
        let Some(RoomMessage::Presence { name, .. }) = validate_message(&raw) else {
            panic!("rejected");
        };
        assert_eq!(name, "Alex");
    }

    #[test]
    fn validates_bundled_and_custom_emotes() {
        let custom = format!("c:{}", "0123456789abcdef".repeat(2));
        for e in ["gg", custom.as_str()] {
            let m = RoomMessage::Emote { peer: "0123456789abcdef".into(), seq: 4, ts: 1_780_000_000_000, e: e.into(), d: 2, x: 0.25, y: 1.0 };
            assert_eq!(validate_message(&emote(json!({ "e": e }))), Some(m.clone()));
            assert_eq!(decode_message(&encode_message(&m)), Some(m));
        }
        // A well-formed slug this version does not know still comes from a newer peer.
        assert!(validate_message(&emote(json!({ "e": "brand_new_2" }))).is_some());
    }

    #[test]
    fn rejects_out_of_shape_emotes() {
        let cases: [(&str, Value); 14] = [
            ("empty ref", emote(json!({ "e": "" }))),
            ("upper-case slug", emote(json!({ "e": "GG" }))),
            ("slug with a path", emote(json!({ "e": "../gg" }))),
            ("slug over 40 characters", emote(json!({ "e": "a".repeat(41) }))),
            ("short custom hash", emote(json!({ "e": "c:0123" }))),
            ("upper-case custom hash", emote(json!({ "e": format!("c:{}", "ABCDEF0123456789".repeat(2)) }))),
            ("non-string ref", emote(json!({ "e": 7 }))),
            ("missing ref", emote(json!({ "e": null }))),
            ("display 0", emote(json!({ "d": 0 }))),
            ("display 17", emote(json!({ "d": 17 }))),
            ("x above 1", emote(json!({ "x": 1.5 }))),
            ("x below 0", emote(json!({ "x": -0.01 }))),
            ("y over the top", emote(json!({ "y": 1e308 }))),
            ("string y", emote(json!({ "y": "0.5" }))),
        ];
        for (label, raw) in cases {
            assert!(validate_message(&raw).is_none(), "{label}");
        }
    }

    #[test]
    fn is_older_than_only_for_a_plain_lower_version() {
        assert!(is_older_than("0.4.0", "0.5.0"));
        assert!(is_older_than("0.4.9", "0.5.0"));
        assert!(!is_older_than("0.5.0", "0.5.0"));
        assert!(!is_older_than("0.5.1", "0.5.0"));
        assert!(!is_older_than("1.0.0", "0.5.0"));
        assert!(!is_older_than("0.10.0", "0.9.0"));
        assert!(is_older_than("0.9.0", "0.10.0"));
        for app in ["room-peer", "0.4", "", "0.4.0-beta", "v0.4.0", "0.4.0.1", "0.-4.0"] {
            assert!(!is_older_than(app, "0.5.0"), "{app}");
        }
    }

    #[test]
    fn names_lose_unsafe_characters_and_fall_back() {
        assert_eq!(sanitize_name("\u{7}A\u{200b}l\u{202a}e\u{2066}x\u{7f}\u{85}\u{feff}", "Player", MAX_NAME), "Alex");
        assert_eq!(sanitize_name("A\u{2028}l\u{2029}e\u{61c}x\u{180e}\u{ad}\u{34f}", "Player", MAX_NAME), "Alex");
        assert_eq!(sanitize_name("\u{115f}\u{1160}\u{3164}\u{ffa0}", "Player", MAX_NAME), "Player");
        assert_eq!(sanitize_name("  Bob  ", "Player", MAX_NAME), "Bob");
        assert_eq!(sanitize_name(&"🎮".repeat(20), "Player", MAX_NAME), "🎮".repeat(16));
        assert_eq!(sanitize_name(&"x".repeat(40), "Player", 24), "x".repeat(24));
        assert_eq!(sanitize_name("\u{200b}", "Darren", MAX_NAME), "Darren");
    }

    // The acceptance test for the packet layer: a peer seals a presence message and reads it back.
    #[test]
    fn a_sealed_presence_round_trips_and_a_flipped_byte_does_not_open() {
        let keys = crate::room::crypto::derive_room_keys("0000000000");
        let m = base_message("presence");
        let packet = seal(&keys.msg_key, &encode_message(&m));
        assert_eq!(decode_message(&open(&keys.msg_key, &packet).unwrap()), Some(m));

        let mut bad = packet.clone();
        bad[30] ^= 1;
        assert!(open(&keys.msg_key, &bad).is_none());
    }
}
