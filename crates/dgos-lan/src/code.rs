//! Access code and session token encodings (technical design §2).

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};

const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// 128 bits in 5-bit symbols: 25 full symbols and one with 2 zero pad bits.
pub const CODE_CHARS: usize = 26;

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("the OS random source is unavailable");
    bytes
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Crockford Base32, 26 characters.
pub fn encode_code(raw: &[u8; 16]) -> String {
    let mut out = String::with_capacity(CODE_CHARS);
    let (mut acc, mut bits) = (0u32, 0u32);
    for byte in raw {
        acc = (acc << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(CROCKFORD[((acc >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(CROCKFORD[((acc << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Groups of four joined by `-` for display.
pub fn group(code: &str) -> String {
    code.as_bytes()
        .chunks(4)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect::<Vec<_>>()
        .join("-")
}

/// Inverse of [`encode_code`]. `-` and whitespace are ignored, case-insensitive,
/// `I`/`L` read as `1` and `O` as `0`. Anything else (wrong length, invalid
/// symbol, non-zero pad bits) is `None`, which callers treat as a mismatch.
pub fn decode_code(input: &str) -> Option<[u8; 16]> {
    let mut out = [0u8; 16];
    let (mut acc, mut bits, mut len, mut symbols) = (0u32, 0u32, 0usize, 0usize);
    for ch in input.chars() {
        if ch == '-' || ch.is_whitespace() {
            continue;
        }
        let value = match ch.to_ascii_uppercase() {
            'O' => 0,
            'I' | 'L' => 1,
            upper => CROCKFORD.iter().position(|c| *c as char == upper)? as u32,
        };
        symbols += 1;
        if symbols > CODE_CHARS {
            return None;
        }
        acc = (acc << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            if len == out.len() {
                return None;
            }
            out[len] = (acc >> bits) as u8;
            len += 1;
            acc &= (1 << bits) - 1;
        }
    }
    (symbols == CODE_CHARS && len == 16 && bits == 2 && acc == 0).then_some(out)
}

/// 256-bit opaque session token for the Cookie value, and its lookup hash.
pub fn new_token() -> (String, [u8; 32]) {
    let raw: [u8; 32] = random_bytes();
    (URL_SAFE_NO_PAD.encode(raw), sha256(&raw))
}

/// The lookup hash of a Cookie value; `None` unless it decodes to 32 bytes.
pub fn token_hash(value: &str) -> Option<[u8; 32]> {
    let raw = URL_SAFE_NO_PAD.decode(value).ok()?;
    (raw.len() == 32).then(|| sha256(&raw))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip_and_tolerate_grouping_case_and_lookalikes() {
        for _ in 0..200 {
            let raw: [u8; 16] = random_bytes();
            let code = encode_code(&raw);
            assert_eq!(code.len(), CODE_CHARS);
            assert_eq!(decode_code(&code), Some(raw));
            assert_eq!(decode_code(&group(&code)), Some(raw));
            assert_eq!(
                decode_code(&format!(" {} ", group(&code).to_lowercase())),
                Some(raw)
            );
        }
        let zero = encode_code(&[0; 16]);
        assert_eq!(decode_code(&zero.replace('0', "o")), Some([0; 16]));
        let ones = encode_code(&[0x08; 16]);
        assert_eq!(decode_code(&ones), Some([0x08; 16]));
    }

    #[test]
    fn malformed_codes_do_not_decode() {
        let code = encode_code(&[0xAB; 16]);
        assert_eq!(decode_code(&code[..25]), None);
        assert_eq!(decode_code(&format!("{code}0")), None);
        assert_eq!(decode_code(&code.replacen(&code[..1], "U", 1)), None);
        // Non-zero pad bits: the last symbol of a valid code always has them clear.
        let mut padded = code.clone();
        padded.pop();
        padded.push('Z');
        assert_eq!(decode_code(&padded), None);
        assert_eq!(decode_code(""), None);
    }

    #[test]
    fn grouping_is_fours_with_dashes() {
        let code = encode_code(&[0; 16]);
        let grouped = group(&code);
        assert_eq!(
            grouped.split('-').map(str::len).collect::<Vec<_>>(),
            [4, 4, 4, 4, 4, 4, 2]
        );
    }

    #[test]
    fn tokens_are_256_bit_and_hash_only_when_well_formed() {
        let (token, hash) = new_token();
        assert_eq!(token.len(), 43);
        assert_eq!(token_hash(&token), Some(hash));
        assert_eq!(token_hash("short"), None);
        assert_eq!(token_hash(&format!("{token}A")), None);
        assert_ne!(new_token().0, token);
    }
}
