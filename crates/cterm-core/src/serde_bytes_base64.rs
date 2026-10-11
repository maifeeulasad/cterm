//! `serde` (de)serialization of `Vec<u8>` as standard base64.
//!
//! Terminal streams are bytes, not text. Serializing them as a JSON array of
//! integers is correct but bloated and unreadable; base64 keeps records
//! compact and lets the webview show them without surprises. We hand-roll a
//! tiny standard-alphabet codec so `cterm-core` stays dependency-light.

use serde::{Deserialize, Deserializer, Serializer};

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const PAD: u8 = b'=';

/// Encodes bytes to a standard, padded base64 string.
#[must_use]
pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;

        out.push(ALPHABET[(n >> 18 & 0x3f) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 0x3f) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 0x3f) as usize] as char
        } else {
            PAD as char
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 0x3f) as usize] as char
        } else {
            PAD as char
        });
    }
    out
}

/// Decodes a standard, padded base64 string back to bytes.
///
/// # Errors
/// Returns an error string if the input length is not a multiple of four or
/// contains characters outside the standard alphabet.
pub fn decode(input: &str) -> Result<Vec<u8>, String> {
    let bytes = input.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("base64 length must be a multiple of 4".to_owned());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        let mut vals = [0u32; 4];
        let mut pads = 0usize;
        for (i, &c) in chunk.iter().enumerate() {
            vals[i] = if c == PAD {
                pads += 1;
                0
            } else {
                if pads != 0 {
                    return Err("base64 padding in the middle of input".to_owned());
                }
                decode_char(c)? as u32
            };
        }
        let n = (vals[0] << 18) | (vals[1] << 12) | (vals[2] << 6) | vals[3];
        out.push((n >> 16 & 0xff) as u8);
        if pads < 2 {
            out.push((n >> 8 & 0xff) as u8);
        }
        if pads < 1 {
            out.push((n & 0xff) as u8);
        }
    }
    Ok(out)
}

fn decode_char(c: u8) -> Result<u8, String> {
    match c {
        b'A'..=b'Z' => Ok(c - b'A'),
        b'a'..=b'z' => Ok(c - b'a' + 26),
        b'0'..=b'9' => Ok(c - b'0' + 52),
        b'+' => Ok(62),
        b'/' => Ok(63),
        other => Err(format!("invalid base64 character: {other:#x}")),
    }
}

/// `serde` serialize entry point (used via `#[serde(with = ...)]`).
pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&encode(bytes))
}

/// `serde` deserialize entry point (used via `#[serde(with = ...)]`).
pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s = String::deserialize(deserializer)?;
    decode(&s).map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // RFC 4648 test vectors.
        assert_eq!(encode(b""), "");
        assert_eq!(encode(b"f"), "Zg==");
        assert_eq!(encode(b"fo"), "Zm8=");
        assert_eq!(encode(b"foo"), "Zm9v");
        assert_eq!(encode(b"foob"), "Zm9vYg==");
        assert_eq!(encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn roundtrip_all_byte_values() {
        let all: Vec<u8> = (0u16..=255).map(|b| b as u8).collect();
        let encoded = encode(&all);
        assert_eq!(decode(&encoded).unwrap(), all);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(decode("Zg=").is_err()); // not multiple of 4
        assert!(decode("Zg@=").is_err()); // invalid char
        assert!(decode("Z=g=").is_err()); // padding in the middle
    }
}
