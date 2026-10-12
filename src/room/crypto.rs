use aes_gcm::aead::{Aead, Payload};
use aes_gcm::aes::cipher::consts::U12;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use sha2::Sha256;

/// Every room packet: format byte, nonce, AES-256-GCM ciphertext and tag. Larger packets are never sent or read.
pub const MAX_PACKET: usize = 1200;
const FORMAT: u8 = 0x01;
const NONCE_LEN: usize = 12;
const OVERHEAD: usize = 1 + NONCE_LEN + 16;
const AAD: [u8; 1] = [FORMAT];

pub struct RoomKeys {
    /// Encrypts every room message. Never leaves the process.
    pub msg_key: [u8; 32],
    /// The relay topic: 64 hex characters that say nothing without the code.
    pub topic: String,
}

/// scrypt makes guessing a code from a relay topic impractical; HKDF gives each key one job.
pub fn derive_room_keys(code: &str) -> RoomKeys {
    let params = scrypt::Params::new(15, 8, 1).expect("log_n 15, r 8, p 1 are valid");
    let mut master = [0u8; 32];
    scrypt::scrypt(code.as_bytes(), b"lolping/room/v1", &params, &mut master).expect("32-byte output");
    let hkdf = hkdf::Hkdf::<Sha256>::new(None, &master);
    let mut msg_key = [0u8; 32];
    hkdf.expand(b"msg", &mut msg_key).expect("32-byte key");
    // "sig-room" dates from a WebRTC design; the name stays so the keys and the test vectors don't change.
    let mut topic = [0u8; 32];
    hkdf.expand(b"sig-room", &mut topic).expect("32-byte key");
    RoomKeys { msg_key, topic: hex::encode(topic) }
}

/// Seals a plaintext into one packet. Panics when the plaintext cannot fit in a packet: callers only seal short JSON.
pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    assert!(plaintext.len() + OVERHEAD <= MAX_PACKET, "room message too large");
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::fill(&mut nonce).expect("os randomness");
    let nonce = Nonce::<U12>::try_from(&nonce[..]).expect("12-byte nonce");
    let body = cipher
        .encrypt(&nonce, Payload { msg: plaintext, aad: &AAD })
        .expect("plaintext fits");
    let mut packet = Vec::with_capacity(plaintext.len() + OVERHEAD);
    packet.push(FORMAT);
    packet.extend_from_slice(&nonce);
    packet.extend_from_slice(&body);
    packet
}

/// The plaintext, or None for anything that isn't a packet sealed with this key.
pub fn open(key: &[u8; 32], packet: &[u8]) -> Option<Vec<u8>> {
    if packet.len() < OVERHEAD || packet.len() > MAX_PACKET || packet[0] != FORMAT {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key");
    let nonce = Nonce::<U12>::try_from(&packet[1..1 + NONCE_LEN]).expect("12-byte nonce");
    cipher.decrypt(&nonce, Payload { msg: &packet[1 + NONCE_LEN..], aad: &AAD }).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Computed once with plain node:crypto (scrypt N=2^15 r=8 p=1, then HKDF-SHA256). Every OS derives the same keys.
    #[test]
    fn known_answer_vectors() {
        let keys = derive_room_keys("0000000000");
        assert_eq!(hex::encode(keys.msg_key), "2b8ef59047332409e1662fe75774f9383fcd16b655fcb3db975f0e80ad2043da");
        assert_eq!(keys.topic, "0381782f350540d4639958bed594ae3b2b1a3d11683146e542edb34db89b6910");
    }

    #[test]
    fn different_codes_give_different_keys() {
        assert_ne!(derive_room_keys("0000000000").msg_key, derive_room_keys("0000000011").msg_key);
    }

    #[test]
    fn round_trips() {
        let key = [7u8; 32];
        let msg = br#"{"t":"bye"}"#;
        assert_eq!(open(&key, &seal(&key, msg)).unwrap(), msg);
    }

    #[test]
    fn uses_a_fresh_nonce_every_time() {
        let key = [7u8; 32];
        assert_ne!(seal(&key, b"x"), seal(&key, b"x"));
    }

    #[test]
    fn rejects_any_flipped_byte() {
        let key = [7u8; 32];
        let packet = seal(&key, b"{\"t\":\"bye\"}");
        for i in 0..packet.len() {
            let mut bad = packet.clone();
            bad[i] ^= 1;
            assert!(open(&key, &bad).is_none(), "byte {i}");
        }
    }

    #[test]
    fn rejects_a_wrong_key_a_truncated_packet_and_an_oversize_packet() {
        let key = [7u8; 32];
        let packet = seal(&key, b"{\"t\":\"bye\"}");
        assert!(open(&[8u8; 32], &packet).is_none());
        assert!(open(&key, &packet[..20]).is_none());
        assert!(open(&key, &[]).is_none());
        let mut oversize = packet.clone();
        oversize.extend(std::iter::repeat_n(0, MAX_PACKET));
        assert!(open(&key, &oversize).is_none());
    }

    #[test]
    #[should_panic(expected = "room message too large")]
    fn refuses_to_seal_more_than_fits_in_a_packet() {
        seal(&[7u8; 32], &vec![0u8; MAX_PACKET]);
    }
}
