//! Fractional index keys for sibling order (design doc §3.1). Shared by backend and
//! the WASM replica. Keys are base-36 digit strings compared bytewise and never end
//! in `0`. Exhausting their bounded precision requires sibling rebalancing.

use crate::error::{Code, WsError, WsResult};

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
const MAX_KEY_LEN: usize = 64;
const APPEND_WIDTH: usize = 8;

pub fn check_order_key(k: &str) -> WsResult<()> {
    let b = k.as_bytes();
    let ok = !b.is_empty()
        && b.len() <= MAX_KEY_LEN
        && b.iter().all(|c| c.is_ascii_digit() || c.is_ascii_lowercase())
        && *b.last().unwrap() != b'0';
    if ok {
        Ok(())
    } else {
        Err(WsError::invalid_op(format!("invalid order_key: {k:?}")))
    }
}

fn idx(c: u8) -> usize {
    DIGITS.iter().position(|d| *d == c).unwrap_or(0)
}

fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        let mut n = 0;
        while n < b.len() && a.get(n).copied().unwrap_or(b'0') == b[n] {
            n += 1;
        }
        if n > 0 {
            let mut out = b[..n].to_vec();
            let a_rest = if a.len() > n { &a[n..] } else { &[][..] };
            out.extend(midpoint(a_rest, Some(&b[n..])));
            return out;
        }
    }
    let da = a.first().map(|c| idx(*c)).unwrap_or(0);
    let db = b.map(|b| idx(b[0])).unwrap_or(DIGITS.len());
    if db - da > 1 {
        vec![DIGITS[(da + db + 1) / 2]]
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        vec![b[0]]
    } else {
        let mut out = vec![DIGITS[da]];
        let a_rest = if a.len() > 1 { &a[1..] } else { &[][..] };
        out.extend(midpoint(a_rest, None));
        out
    }
}

fn exhausted() -> WsError {
    WsError::sub(Code::InvalidOperation, "ORDER_KEY_EXHAUSTED", "order key space exhausted; rebalance siblings")
}

fn after(a: &str) -> WsResult<Vec<u8>> {
    let mut out = a.as_bytes().to_vec();
    out.resize(out.len().max(APPEND_WIDTH), b'0');
    if let Some(i) = out.iter().rposition(|c| *c != b'z') {
        out[i] = DIGITS[idx(out[i]) + 1];
        out[i + 1..].fill(b'0');
        if out.last() == Some(&b'0') {
            *out.last_mut().unwrap() = b'1';
        }
    } else {
        if out.len() == MAX_KEY_LEN {
            return Err(exhausted());
        }
        out.resize((out.len() + APPEND_WIDTH).min(MAX_KEY_LEN), b'0');
        *out.last_mut().unwrap() = b'1';
    }
    Ok(out)
}

/// A key strictly between `a` and `b` (`None` = open end). Requires `a < b`.
pub fn order_key_between(a: Option<&str>, b: Option<&str>) -> WsResult<String> {
    if let Some(a) = a {
        check_order_key(a)?;
    }
    if let Some(b) = b {
        check_order_key(b)?;
    }
    if let (Some(a), Some(b)) = (a, b) {
        if a >= b {
            return Err(WsError::invalid_op("order_key_between requires a < b"));
        }
    }
    let out = match (a, b) {
        (Some(a), None) => after(a)?,
        _ => midpoint(a.unwrap_or("").as_bytes(), b.map(|s| s.as_bytes())),
    };
    let out = String::from_utf8(out).unwrap();
    check_order_key(&out).map_err(|_| exhausted())?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn between() {
        let mut lo: Option<String> = None;
        let mut keys = vec![];
        for _ in 0..50 {
            let k = order_key_between(lo.as_deref(), None).unwrap();
            if let Some(l) = &lo {
                assert!(l < &k);
            }
            keys.push(k.clone());
            lo = Some(k);
        }
        // repeatedly bisect the first gap
        let (a, mut b) = (keys[0].clone(), keys[1].clone());
        for _ in 0..200 {
            let m = order_key_between(Some(&a), Some(&b)).unwrap();
            assert!(a < m && m < b, "{a} {m} {b}");
            check_order_key(&m).unwrap();
            if m.len() > 60 {
                break;
            }
            b = m;
        }
        let first = order_key_between(None, Some(&keys[0])).unwrap();
        assert!(first < keys[0]);
        assert!(order_key_between(Some("b"), Some("a")).is_err());
        assert!(check_order_key("a0").is_err());
    }

    #[test]
    fn continuous_append_keeps_compact_valid_keys() {
        let mut last = order_key_between(None, None).unwrap();
        for _ in 0..100_000 {
            let next = order_key_between(Some(&last), None).unwrap();
            check_order_key(&next).unwrap();
            assert!(last < next);
            assert!(next.len() <= APPEND_WIDTH);
            last = next;
        }
    }

    #[test]
    fn append_preserves_order_after_existing_keys() {
        let keys = ["1".to_string(), "a9".to_string(), "az".to_string(), "z".repeat(8), "z".repeat(60), format!("{}z", "0".repeat(63)), format!("{}y", "z".repeat(63))];
        for key in keys {
            let next = order_key_between(Some(&key), None).unwrap();
            check_order_key(&next).unwrap();
            assert!(key < next, "{key} -> {next}");
        }
    }

    #[test]
    fn exhausted_key_space_is_reported_without_invalid_output() {
        let max = "z".repeat(MAX_KEY_LEN);
        let min = format!("{}1", "0".repeat(MAX_KEY_LEN - 1));
        let lower = format!("{}1", "a".repeat(MAX_KEY_LEN - 1));
        let upper = format!("{}2", "a".repeat(MAX_KEY_LEN - 1));
        for (a, b) in [(Some(max.as_str()), None), (None, Some(min.as_str())), (Some(lower.as_str()), Some(upper.as_str()))] {
            let err = order_key_between(a, b).unwrap_err();
            assert_eq!(err.code, Code::InvalidOperation);
            assert_eq!(err.sub, Some("ORDER_KEY_EXHAUSTED"));
        }
    }

    #[test]
    fn invalid_inputs_remain_distinct_from_exhaustion() {
        for key in ["".to_string(), "a0".to_string(), "A".to_string(), "z".repeat(MAX_KEY_LEN + 1)] {
            for (a, b) in [(Some(key.as_str()), None), (None, Some(key.as_str()))] {
                let err = order_key_between(a, b).unwrap_err();
                assert_eq!(err.code, Code::InvalidOperation);
                assert_eq!(err.sub, None);
            }
        }
    }
}
