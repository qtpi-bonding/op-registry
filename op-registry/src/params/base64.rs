//! Standard-alphabet base64 with required padding.

use super::UsageError;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn invalid(reason: &str) -> UsageError {
    UsageError::InvalidField {
        name: "base64".to_owned(),
        reason: reason.to_owned(),
    }
}

fn sextet(c: u8) -> Option<u32> {
    ALPHABET.iter().position(|a| *a == c).map(|p| p as u32)
}

pub fn decode(text: &str) -> Result<Vec<u8>, UsageError> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err(invalid(
            "length is not a multiple of 4 (padding is required)",
        ));
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let groups = bytes.len() / 4;
    for (g, chunk) in bytes.chunks(4).enumerate() {
        let pad = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 || (pad > 0 && g + 1 != groups) {
            return Err(invalid("misplaced padding"));
        }
        let mut n = 0u32;
        for c in &chunk[..4 - pad] {
            n = (n << 6)
                | sextet(*c).ok_or_else(|| invalid("character outside the base64 alphabet"))?;
        }
        n <<= 6 * pad as u32;
        out.extend_from_slice(&n.to_be_bytes()[1..4 - pad]);
    }
    Ok(out)
}
