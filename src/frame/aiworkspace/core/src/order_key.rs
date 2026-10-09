//! Fractional index keys for sibling order (design doc §3.1). Shared by backend and
//! the WASM replica. Keys are base-36 digit strings compared bytewise and never end
//! in `0`, so a key strictly between any two distinct keys always exists.

use crate::error::{WsError, WsResult};

const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

pub fn check_order_key(k: &str) -> WsResult<()> {
    let b = k.as_bytes();
    let ok = !b.is_empty()
        && b.len() <= 64
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
    let out = midpoint(a.unwrap_or("").as_bytes(), b.map(|s| s.as_bytes()));
    Ok(String::from_utf8(out).unwrap())
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
            check_order_key(&m).unwrap_or_else(|_| assert!(m.len() > 64));
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
}
