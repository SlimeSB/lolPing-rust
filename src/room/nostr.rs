use k256::schnorr::{Signature, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Ephemeral (20000–29999): relays forward these and don't store them.
pub const ROOM_EVENT_KIND: u32 = 24747;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NostrEvent {
    pub id: String,
    pub pubkey: String,
    pub created_at: u64,
    pub kind: u32,
    pub tags: Vec<Vec<String>>,
    pub content: String,
    pub sig: String,
}

pub struct KeyPair {
    secret: [u8; 32],
    pub pubkey: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayMessage {
    Event { sub_id: String, event: NostrEvent },
    Ok { event_id: String, ok: bool, message: String },
    Eose { sub_id: String },
    Closed { sub_id: String, message: String },
    Notice { message: String },
}

pub fn make_key_pair() -> KeyPair {
    let mut secret = [0u8; 32];
    loop {
        getrandom::fill(&mut secret).expect("os randomness");
        if let Ok(signing) = SigningKey::from_slice(&secret) {
            return KeyPair { pubkey: hex::encode(signing.verifying_key().to_bytes()), secret };
        }
    }
}

impl KeyPair {
    pub fn sign(&self, kind: u32, tags: Vec<Vec<String>>, content: String, created_at: u64) -> NostrEvent {
        let mut aux = [0u8; 32];
        getrandom::fill(&mut aux).expect("os randomness");
        self.sign_with(kind, tags, content, created_at, &aux)
    }

    // A pinned nonce seed, so the vector test can reproduce one signature.
    fn sign_with(&self, kind: u32, tags: Vec<Vec<String>>, content: String, created_at: u64, aux: &[u8; 32]) -> NostrEvent {
        let id = Sha256::digest(serialize_event(&self.pubkey, created_at, kind, &tags, &content).as_bytes());
        let signing = SigningKey::from_slice(&self.secret).expect("a key pair holds a valid secret");
        let sig = signing.sign_raw(&id, aux).expect("schnorr sign");
        NostrEvent {
            id: hex::encode(id),
            pubkey: self.pubkey.clone(),
            created_at,
            kind,
            tags,
            content,
            sig: hex::encode(sig.to_bytes()),
        }
    }
}

pub fn serialize_event(pubkey: &str, created_at: u64, kind: u32, tags: &[Vec<String>], content: &str) -> String {
    serde_json::to_string(&(0u8, pubkey, created_at, kind, tags, content)).expect("no maps")
}

pub fn verify_event(event: &NostrEvent) -> bool {
    let serialized = serialize_event(&event.pubkey, event.created_at, event.kind, &event.tags, &event.content);
    if hex::encode(Sha256::digest(serialized.as_bytes())) != event.id {
        return false;
    }
    let (Ok(pubkey), Ok(sig), Ok(id)) = (hex::decode(&event.pubkey), hex::decode(&event.sig), hex::decode(&event.id)) else {
        return false;
    };
    let (Ok(verifying), Ok(signature)) = (VerifyingKey::from_slice(&pubkey), Signature::from_slice(&sig)) else {
        return false;
    };
    verifying.verify_raw(&id, &signature).is_ok()
}

pub fn req_message(sub_id: &str, topic: &str, since: u64) -> String {
    #[derive(Serialize)]
    struct Filter<'a> {
        kinds: [u32; 1],
        #[serde(rename = "#x")]
        x: [&'a str; 1],
        since: u64,
        limit: u32,
    }
    let filter = Filter { kinds: [ROOM_EVENT_KIND], x: [topic], since, limit: 0 };
    serde_json::to_string(&("REQ", sub_id, filter)).expect("no maps")
}

pub fn event_message(event: &NostrEvent) -> String {
    serde_json::to_string(&("EVENT", event)).expect("no maps")
}

pub fn close_message(sub_id: &str) -> String {
    serde_json::to_string(&("CLOSE", sub_id)).expect("no maps")
}

pub fn parse_relay_message(text: &str) -> Option<RelayMessage> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let message = value.as_array()?;
    match message.first()?.as_str()? {
        "EVENT" => Some(RelayMessage::Event {
            sub_id: message.get(1)?.as_str()?.to_string(),
            event: to_event(message.get(2)?)?,
        }),
        "OK" => {
            let event_id = message.get(1)?.as_str()?;
            if !is_hex(event_id, 64) {
                return None;
            }
            Some(RelayMessage::Ok {
                event_id: event_id.to_string(),
                ok: message.get(2)?.as_bool()?,
                message: optional(message.get(3))?,
            })
        }
        "EOSE" => Some(RelayMessage::Eose { sub_id: message.get(1)?.as_str()?.to_string() }),
        "CLOSED" => Some(RelayMessage::Closed {
            sub_id: message.get(1)?.as_str()?.to_string(),
            message: optional(message.get(2))?,
        }),
        "NOTICE" => Some(RelayMessage::Notice { message: message.get(1)?.as_str()?.to_string() }),
        _ => None,
    }
}

fn to_event(raw: &serde_json::Value) -> Option<NostrEvent> {
    let event: NostrEvent = serde_json::from_value(raw.clone()).ok()?;
    if !is_hex(&event.id, 64) || !is_hex(&event.pubkey, 64) || !is_hex(&event.sig, 128) {
        return None;
    }
    Some(event)
}

fn optional(value: Option<&serde_json::Value>) -> Option<String> {
    match value {
        None => Some(String::new()),
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PUBKEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn bytes(hex_text: &str) -> Vec<u8> {
        hex::decode(hex_text).expect("hex")
    }

    #[test]
    fn serializes_per_nip01() {
        let serialized = serialize_event(PUBKEY, 1_700_000_000, ROOM_EVENT_KIND, &[vec!["x".into(), "ab".into()]], "hi");
        assert_eq!(serialized, format!("[0,\"{PUBKEY}\",1700000000,24747,[[\"x\",\"ab\"]],\"hi\"]"));
    }

    #[test]
    fn ids_an_event_by_the_sha256_of_its_serialization() {
        let keys = make_key_pair();
        let event = keys.sign(ROOM_EVENT_KIND, vec![vec!["x".into(), "ab".into()]], "hi".into(), 1_700_000_000);
        let expected = hex::encode(Sha256::digest(
            serialize_event(&keys.pubkey, 1_700_000_000, ROOM_EVENT_KIND, &[vec!["x".into(), "ab".into()]], "hi").as_bytes(),
        ));
        assert_eq!(event.id, expected);
        assert_eq!(event.pubkey, keys.pubkey);
        assert_eq!(event.created_at, 1_700_000_000);
        assert_eq!(event.kind, ROOM_EVENT_KIND);
        assert_eq!(event.tags, vec![vec!["x".to_string(), "ab".to_string()]]);
        assert_eq!(event.content, "hi");
    }

    #[test]
    fn matches_bip340_test_vector_zero() {
        let mut secret = [0u8; 32];
        secret[31] = 3;
        let signing = SigningKey::from_slice(&secret).expect("valid secret");
        assert_eq!(hex::encode(signing.verifying_key().to_bytes()), "f9308a019258c31049344f85f89d5229b531c845836f99b08601f113bce036f9");
        let signature = signing.sign_raw(&[0u8; 32], &[0u8; 32]).expect("sign");
        assert_eq!(
            hex::encode(signature.to_bytes()),
            "e907831f80848d1069a5371b402410364bdf1c5f8307b0084c55f1ce2dca821525f66a4a85ea8b71e482a74f382d2ce5ebeee8fdb2172f477df4900d310536c0"
        );
        let verifying = VerifyingKey::from_slice(&signing.verifying_key().to_bytes()).expect("x-only key");
        assert!(verifying.verify_raw(&[0u8; 32], &signature).is_ok());
    }

    #[test]
    fn signs_events_that_verify_with_a_fresh_key_each_time() {
        let keys = make_key_pair();
        let event = keys.sign(ROOM_EVENT_KIND, Vec::new(), String::new(), 1_700_000_000);
        assert_eq!(keys.pubkey.len(), 64);
        assert_eq!(event.sig.len(), 128);
        assert!(verify_event(&event));
        assert_ne!(make_key_pair().pubkey, keys.pubkey);
    }

    #[test]
    fn rejects_a_tampered_event() {
        let keys = make_key_pair();
        let event = keys.sign(ROOM_EVENT_KIND, vec![vec!["x".into(), "ab".into()]], "hi".into(), 1_700_000_000);

        let mut other_content = event.clone();
        other_content.content = "ho".into();
        assert!(!verify_event(&other_content));

        let mut other_id = event.clone();
        other_id.id = "00".repeat(32);
        assert!(!verify_event(&other_id));

        let mut other_sig = event.clone();
        other_sig.sig = "00".repeat(64);
        assert!(!verify_event(&other_sig));

        let mut other_pubkey = event.clone();
        other_pubkey.pubkey = make_key_pair().pubkey;
        assert!(!verify_event(&other_pubkey));

        let mut other_time = event.clone();
        other_time.created_at += 1;
        assert!(!verify_event(&other_time));
    }

    #[test]
    fn builds_the_messages_sent_to_relays() {
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&req_message("lolping", "ab", 1_700_000_000)).unwrap(),
            serde_json::json!(["REQ", "lolping", {"kinds": [24747], "#x": ["ab"], "since": 1700000000, "limit": 0}]),
        );
        assert_eq!(
            req_message("lolping", "ab", 1_700_000_000),
            "[\"REQ\",\"lolping\",{\"kinds\":[24747],\"#x\":[\"ab\"],\"since\":1700000000,\"limit\":0}]",
        );
        let event = make_key_pair().sign(ROOM_EVENT_KIND, Vec::new(), "c".into(), 1);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&event_message(&event)).unwrap(),
            serde_json::json!(["EVENT", event]),
        );
        assert_eq!(close_message("lolping"), "[\"CLOSE\",\"lolping\"]");
    }

    #[test]
    fn parses_each_relay_message_type() {
        let event = make_key_pair().sign(ROOM_EVENT_KIND, vec![vec!["x".into(), "ab".into()]], "hi".into(), 1_700_000_000);
        assert_eq!(
            parse_relay_message(&serde_json::to_string(&("EVENT", "s", &event)).unwrap()),
            Some(RelayMessage::Event { sub_id: "s".into(), event: event.clone() }),
        );
        assert_eq!(
            parse_relay_message(&serde_json::to_string(&("OK", &event.id, false, "rate-limited: slow down")).unwrap()),
            Some(RelayMessage::Ok { event_id: event.id.clone(), ok: false, message: "rate-limited: slow down".into() }),
        );
        assert_eq!(
            parse_relay_message(&serde_json::to_string(&("OK", &event.id, true)).unwrap()),
            Some(RelayMessage::Ok { event_id: event.id.clone(), ok: true, message: String::new() }),
        );
        assert_eq!(parse_relay_message("[\"EOSE\",\"s\"]"), Some(RelayMessage::Eose { sub_id: "s".into() }));
        assert_eq!(
            parse_relay_message("[\"CLOSED\",\"s\",\"error: shutting down\"]"),
            Some(RelayMessage::Closed { sub_id: "s".into(), message: "error: shutting down".into() }),
        );
        assert_eq!(parse_relay_message("[\"NOTICE\",\"hello\"]"), Some(RelayMessage::Notice { message: "hello".into() }));
    }

    #[test]
    fn rejects_junk() {
        let event = make_key_pair().sign(ROOM_EVENT_KIND, vec![vec!["x".into(), "ab".into()]], "hi".into(), 1_700_000_000);
        let patch = |field: &str, replacement: serde_json::Value| {
            let mut value = serde_json::to_value(&event).unwrap();
            value[field] = replacement;
            serde_json::to_string(&("EVENT", "s", value)).unwrap()
        };
        let cases = vec![
            "not json".to_string(),
            "{\"a\":1}".to_string(),
            "[]".to_string(),
            "[\"AUTH\",\"challenge\"]".to_string(),
            "[\"EVENT\",\"s\"]".to_string(),
            "[\"EOSE\"]".to_string(),
            patch("id", serde_json::json!("xyz")),
            patch("sig", serde_json::json!("ab")),
            patch("pubkey", serde_json::json!("A".repeat(64))),
            patch("created_at", serde_json::json!(1.5)),
            patch("kind", serde_json::json!("24747")),
            patch("tags", serde_json::json!([["x", 1]])),
            patch("tags", serde_json::json!("x")),
            patch("content", serde_json::json!(5)),
            serde_json::to_string(&("OK", &event.id, "yes", "")).unwrap(),
            "[\"OK\",\"short\",true,\"\"]".to_string(),
        ];
        for case in cases {
            assert_eq!(parse_relay_message(&case), None, "{case}");
        }
    }

    #[test]
    fn ignores_unknown_event_keys() {
        let event = make_key_pair().sign(ROOM_EVENT_KIND, Vec::new(), "hi".into(), 1);
        let text = serde_json::to_string(&("EVENT", "s", &event)).unwrap().replacen("{\"id\"", "{\"__proto__\":{\"polluted\":1},\"id\"", 1);
        let Some(RelayMessage::Event { event: parsed, .. }) = parse_relay_message(&text) else {
            panic!("expected an event")
        };
        assert_eq!(parsed, event);
        assert_eq!(
            serde_json::to_value(&parsed).unwrap().as_object().unwrap().len(),
            7,
        );
    }

    #[test]
    fn decodes_the_hex_of_a_signature() {
        let event = make_key_pair().sign(ROOM_EVENT_KIND, Vec::new(), String::new(), 0);
        assert_eq!(bytes(&event.sig).len(), 64);
        assert_eq!(bytes(&event.pubkey).len(), 32);
        assert_eq!(bytes(&event.id).len(), 32);
    }
}
