pub mod code;
pub mod colors;
pub mod manager;
pub mod crypto;
pub mod flood;
pub mod lan;
pub mod nostr;
pub mod relay;
pub mod protocol;
pub mod rate;
pub mod transport;

pub const MAX_NAME: usize = 16;

/// A display name that can't hide, reorder or break the text around it; `max` is in code points.
pub fn sanitize_name(raw: &str, fallback: &str, max: usize) -> String {
    let cleaned: String = raw.chars().filter(|c| !is_unsafe_char(*c)).collect();
    let name: String = cleaned.trim().chars().take(max).collect();
    let name = name.trim();
    if name.is_empty() {
        fallback.to_string()
    } else {
        name.to_string()
    }
}

/// C0/C1 controls, zero-width and bidi characters, the BOM and the blank Hangul fillers.
fn is_unsafe_char(c: char) -> bool {
    let u = c as u32;
    (0x00..=0x1f).contains(&u)
        || (0x7f..=0x9f).contains(&u)
        || matches!(u, 0xad | 0x34f | 0x61c | 0x115f | 0x1160 | 0x180e | 0x3164 | 0xfeff | 0xffa0)
        || (0x200b..=0x200f).contains(&u)
        || (0x2028..=0x202e).contains(&u)
        || (0x2060..=0x2069).contains(&u)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_lose_unsafe_characters() {
        assert_eq!(sanitize_name("  Alice  ", "Player", MAX_NAME), "Alice");
        assert_eq!(sanitize_name("\u{200b}\u{202e}", "Player", MAX_NAME), "Player");
        assert_eq!(sanitize_name("a\nb", "Player", MAX_NAME), "ab");
        assert_eq!(sanitize_name("abcdefghijklmnopq", "Player", MAX_NAME), "abcdefghijklmnop");
        assert_eq!(sanitize_name("", "Player", MAX_NAME), "Player");
    }
}
