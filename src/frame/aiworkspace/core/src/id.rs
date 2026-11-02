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

/// The Workspace id (short-id TODO, 2026-11-01): `ws_` + `YYMM` (the UTC month it was created) + a 4-character
/// base36 code, 36⁴ codes a month per deployment. The id is also the Workspace's host label (`aiws-2611a0b3.<zone>`),
/// so it stays short and memorable; a link that must be secret carries a long parameter instead (the host is plain
/// text under TLS anyway).
pub const WORKSPACE_ID_PREFIX: &str = "ws_";
pub const WORKSPACE_CODE_SPACE: u32 = 36 * 36 * 36 * 36;
const BASE36: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

pub fn workspace_id(created_ms: i64, code: u32) -> String {
    let (y, m, _) = crate::value::civil_from_days(created_ms.div_euclid(86_400_000));
    let mut n = code % WORKSPACE_CODE_SPACE;
    let mut tail = [b'0'; 4];
    for slot in tail.iter_mut().rev() {
        *slot = BASE36[(n % 36) as usize];
        n /= 36;
    }
    format!("{WORKSPACE_ID_PREFIX}{:02}{:02}{}", y.rem_euclid(100), m, std::str::from_utf8(&tail).unwrap())
}

/// `^ws_[0-9]{2}(0[1-9]|1[0-2])[0-9a-z]{4}$`
pub fn is_workspace_id(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != WORKSPACE_ID_PREFIX.len() + 8 || !s.starts_with(WORKSPACE_ID_PREFIX) {
        return false;
    }
    let (date, code) = b[WORKSPACE_ID_PREFIX.len()..].split_at(4);
    let month = (date[2] - b'0') as u32 * 10 + (date[3].wrapping_sub(b'0')) as u32;
    date.iter().all(u8::is_ascii_digit) && (1..=12).contains(&month) && code.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// The host label of a Workspace (`aiws-<label>.<zone>`): its id without the prefix (a DNS label carries no `_`).
pub fn workspace_host_label(workspace_id: &str) -> &str {
    workspace_id.strip_prefix(WORKSPACE_ID_PREFIX).unwrap_or(workspace_id)
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
        let id = prefixed_id("ep_", &[7u8; 17]);
        assert!(is_prefixed_id("ep_", &id), "{id}");
    }

    #[test]
    fn workspace_ids() {
        // 2026-11-01T12:00Z
        let nov = 1_793_534_400_000;
        assert_eq!(workspace_id(nov, 0), "ws_26110000");
        assert_eq!(workspace_id(nov, 35), "ws_2611000z");
        assert_eq!(workspace_id(nov, 36), "ws_26110010");
        assert_eq!(workspace_id(nov, WORKSPACE_CODE_SPACE - 1), "ws_2611zzzz");
        assert_eq!(workspace_id(nov, WORKSPACE_CODE_SPACE), "ws_26110000");
        assert_eq!(workspace_host_label("ws_2611a0b3"), "2611a0b3");
        for ok in ["ws_26110000", "ws_2611zzzz", "ws_0001a0b3", "ws_9912a0b3"] {
            assert!(is_workspace_id(ok), "{ok}");
        }
        for bad in ["ws_2600a0b3", "ws_2613a0b3", "ws_2611A0B3", "ws_2611a0b", "ws_2611a0b3x", "2611a0b3", "ws_pxmyr26jfiyascwlkxwfy5nfn2", "ws_26x1a0b3", ""] {
            assert!(!is_workspace_id(bad), "{bad}");
        }
    }
}
