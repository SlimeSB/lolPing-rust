/// Crockford base32: no I, L, O or U, so a code read out loud can't be misheard as another.
pub const ROOM_ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

const CODE_LENGTH: usize = 10;
const PREFIX: &str = "P1NG";
const MAX_INPUT: usize = 2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParsedRoomCode {
    Valid(String),
    Invalid,
    Mistyped,
}

/// Luhn mod 32 over the 9 random characters: catches every single-character error and most swaps.
pub fn check_char(nine: &[u8]) -> u8 {
    let mut factor = 2u32;
    let mut sum = 0u32;
    for &c in nine.iter().rev() {
        let mut addend = factor * alphabet_index(c);
        factor = if factor == 2 { 1 } else { 2 };
        addend = addend / 32 + (addend % 32);
        sum += addend;
    }
    ROOM_ALPHABET[((32 - (sum % 32)) % 32) as usize]
}

fn alphabet_index(c: u8) -> u32 {
    ROOM_ALPHABET.iter().position(|&a| a == c).unwrap_or(0) as u32
}

/// The canonical code (10 characters, no prefix) from 9 random bytes.
pub fn make_room_code(random9: &[u8]) -> String {
    let mut nine = String::with_capacity(CODE_LENGTH);
    for &b in random9.iter().take(9) {
        nine.push(ROOM_ALPHABET[(b & 31) as usize] as char);
    }
    let check = check_char(nine.as_bytes());
    nine.push(check as char);
    nine
}

pub fn format_room_code(canonical: &str) -> String {
    format!("PING-{}-{}", &canonical[..5], &canonical[5..])
}

fn is_code_shaped(c: &str) -> bool {
    c.len() == CODE_LENGTH && c.bytes().all(|b| ROOM_ALPHABET.contains(&b))
}

fn valid(code: &str) -> bool {
    is_code_shaped(code) && check_char(&code.as_bytes()[..9]) == code.as_bytes()[9]
}

/// Look-alike letters read as the digits they resemble: O→0, I/L→1.
fn map_lookalikes(t: &str) -> String {
    t.to_uppercase()
        .chars()
        .map(|c| match c {
            'O' => '0',
            'I' | 'L' => '1',
            c => c,
        })
        .collect()
}

/// Finds a room code in what the user typed or pasted. `strict` (the clipboard) accepts only a PING code or a lone code.
pub fn parse_room_code(text: &str, strict: bool) -> ParsedRoomCode {
    let head: String = text.chars().take(MAX_INPUT).collect();
    let tokens: Vec<(String, bool)> = head
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .map(|raw| raw.replace('-', ""))
        .filter(|raw| !raw.is_empty())
        .map(|raw| {
            let digit = raw.bytes().any(|b| b.is_ascii_digit());
            (map_lookalikes(&raw), digit)
        })
        .collect();

    let mut mistyped = false;
    let mut try_code = |c: &str, counts_as_typo: bool| -> Option<String> {
        if !is_code_shaped(c) {
            return None;
        }
        if valid(c) {
            return Some(c.to_string());
        }
        if counts_as_typo {
            mistyped = true;
        }
        None
    };

    // A PING code: the token boundaries must fall where the display form's do.
    for i in 0..tokens.len() {
        if !tokens[i].0.starts_with(PREFIX) {
            continue;
        }
        let mut joined = String::new();
        for token in &tokens[i..] {
            joined.push_str(&token.0);
            if ![4, 9, 14].contains(&joined.len()) {
                break;
            }
            if joined.len() == 14 {
                if let Some(code) = try_code(&joined[4..], true) {
                    return ParsedRoomCode::Valid(code);
                }
                break;
            }
        }
    }

    let whole: String = tokens.iter().map(|t| t.0.as_str()).collect();
    let whole_digit = tokens.iter().any(|t| t.1);
    let whole_counts = if strict { tokens.len() == 1 && whole_digit } else { tokens.len() == 1 || whole_digit };
    if whole_counts {
        let candidate = if whole.starts_with(PREFIX) && whole.len() == 14 { &whole[4..] } else { &whole };
        if let Some(code) = try_code(candidate, true) {
            return ParsedRoomCode::Valid(code);
        }
    }

    if !strict {
        for token in &tokens {
            if !token.1 {
                continue;
            }
            if let Some(code) = try_code(&token.0, false) {
                return ParsedRoomCode::Valid(code);
            }
        }
    }

    if mistyped {
        ParsedRoomCode::Mistyped
    } else {
        ParsedRoomCode::Invalid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        let code = make_room_code(&[0, 1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(code.len(), 10);
        assert_eq!(format_room_code(&code), format!("PING-{}-{}", &code[..5], &code[5..]));
        assert_eq!(parse_room_code(&code, false), ParsedRoomCode::Valid(code.clone()));
        assert_eq!(parse_room_code(&format_room_code(&code), false), ParsedRoomCode::Valid(code.clone()));
        assert_eq!(parse_room_code(&format!("PING {} {}", &code[..5], &code[5..]), false), ParsedRoomCode::Valid(code));
    }

    #[test]
    fn lookalikes_and_rejections() {
        let code = make_room_code(&[9, 8, 7, 6, 5, 4, 3, 2, 1]);
        let typed = format!("ping-{}-{}", &code[..5], &code[5..]);
        assert_eq!(parse_room_code(&typed, false), ParsedRoomCode::Valid(code.clone()));
        assert_eq!(parse_room_code("hello there", false), ParsedRoomCode::Invalid);
        assert_eq!(parse_room_code("STRAWBERRY", false), ParsedRoomCode::Mistyped);
        assert_eq!(parse_room_code("not a code at all", true), ParsedRoomCode::Invalid);
        let wrong = format!("{}X", &code[..9]);
        assert_eq!(parse_room_code(&wrong, true), ParsedRoomCode::Mistyped);
    }
}
