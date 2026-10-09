//! Identifier grammar (design doc §2.3). The character sets are hard constraints:
//! ids end up in SQLite JSON paths and package file names, so they are validated,
//! never escaped.

use crate::error::{Code, WsError, WsResult};

const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

/// `^[a-z0-9][a-z0-9_-]{0,63}$`
pub fn is_valid_id(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 64 {
        return false;
    }
    let first = b[0];
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return false;
    }
    b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_' || *c == b'-')
}

pub fn check_id(kind: &str, s: &str) -> WsResult<()> {
    if is_valid_id(s) {
        Ok(())
    } else {
        Err(WsError::new(Code::InvalidOperation, format!("invalid {kind}: {s:?}")))
    }
}

/// 1–128 bytes of visible ASCII.
pub fn check_idempotency_key(s: &str) -> WsResult<()> {
    if !s.is_empty() && s.len() <= 128 && s.bytes().all(|c| (0x21..=0x7e).contains(&c)) {
        Ok(())
    } else {
        Err(WsError::invalid_op("idempotency_key must be 1-128 visible ASCII bytes"))
    }
}

/// `<prefix>` + 26 lowercase base32 chars (130 bits) from caller-supplied randomness.
pub fn prefixed_id(prefix: &str, random: &[u8; 17]) -> String {
    let mut out = String::with_capacity(prefix.len() + 26);
    out.push_str(prefix);
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut n = 0;
    for byte in random {
        acc = (acc << 8) | *byte as u32;
        bits += 8;
        while bits >= 5 && n < 26 {
            out.push(BASE32[((acc >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
            n += 1;
        }
    }
    out
}

pub fn is_prefixed_id(prefix: &str, s: &str) -> bool {
    s.len() == prefix.len() + 26
        && s.starts_with(prefix)
        && s[prefix.len()..].bytes().all(|c| BASE32.contains(&c))
}

/// Entity display/lookup name: 1–128 scalar values, no `/` or control characters.
pub fn check_name(name: &str) -> WsResult<()> {
    let n = name.chars().count();
    if n == 0 || n > 128 || name.chars().any(|c| c == '/' || c.is_control()) {
        return Err(WsError::invalid_op("name must be 1-128 chars without '/' or control characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids() {
        assert!(is_valid_id("tasks"));
        assert!(is_valid_id("task-42_x"));
        assert!(!is_valid_id("-a"));
        assert!(!is_valid_id("Tasks"));
        assert!(!is_valid_id(""));
        assert!(!is_valid_id(&"a".repeat(65)));
        let id = prefixed_id("ws_", &[7u8; 17]);
        assert!(is_prefixed_id("ws_", &id), "{id}");
    }
}
