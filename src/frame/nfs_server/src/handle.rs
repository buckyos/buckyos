//! Opaque signed handles for unanchored entity nodes (nfs_server.md §3.3),
//! plus entry_ref derivation and the stateless list cursor codec.
//!
//! Handle scheme (classic NFS filehandle): `nh_<b64url(payload)>.<b64url(mac16)>`
//! where payload = JSON `{r: root_id, p: rel_path, i: ino, k: kind}` and
//! mac16 = first 16 bytes of HMAC-SHA256(key, payload). The key persists in
//! filedb so handles survive restarts; a resolve must still verify that the
//! native file id at the path matches `i`, otherwise the handle is STALE.

use crate::error::{invalid, stale, NfsResult};
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const ROOT_NODE_ID: &str = "nh_root";

/// Payload of an unanchored native handle.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeHandle {
    /// export root id
    pub r: String,
    /// relative path within the root ("" = the root dir itself)
    pub p: String,
    /// native file id (inode); 0 on platforms without one
    pub i: u64,
    /// "f" | "d" | "l"
    pub k: String,
}

fn hmac16(key: &[u8], payload: &[u8]) -> [u8; 16] {
    // HMAC-SHA256 (manual construction; only used as an integrity tag for
    // server-issued opaque handles, key is random 32 bytes).
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        let d = Sha256::digest(key);
        k[..d.len()].copy_from_slice(&d);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for i in 0..BLOCK {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let inner = Sha256::new_with_prefix(ipad).chain_update(payload).finalize();
    let outer = Sha256::new_with_prefix(opad).chain_update(inner).finalize();
    let mut out = [0u8; 16];
    out.copy_from_slice(&outer[..16]);
    out
}

pub struct HandleCodec {
    key: Vec<u8>,
}

impl HandleCodec {
    pub fn new(key: Vec<u8>) -> Self {
        HandleCodec { key }
    }

    pub fn encode(&self, h: &NativeHandle) -> String {
        let payload = serde_json::to_vec(h).expect("handle serialize");
        let mac = hmac16(&self.key, &payload);
        format!("nh_{}.{}", B64.encode(&payload), B64.encode(mac))
    }

    /// Decodes and authenticates a `nh_*` node_id. Tampered or foreign handles
    /// return STALE (the client must re-resolve from a trusted locator).
    pub fn decode(&self, node_id: &str) -> NfsResult<NativeHandle> {
        let rest = node_id
            .strip_prefix("nh_")
            .ok_or_else(|| invalid(format!("not a native handle: {}", node_id)))?;
        let (p64, m64) = rest
            .split_once('.')
            .ok_or_else(|| stale("malformed handle"))?;
        let payload = B64.decode(p64).map_err(|_| stale("malformed handle"))?;
        let mac = B64.decode(m64).map_err(|_| stale("malformed handle"))?;
        let expect = hmac16(&self.key, &payload);
        if mac.len() != 16 || !constant_time_eq(&mac, &expect) {
            return Err(stale("handle signature mismatch"));
        }
        let h: NativeHandle =
            serde_json::from_slice(&payload).map_err(|_| stale("malformed handle payload"))?;
        Ok(h)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut acc = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        acc |= x ^ y;
    }
    acc == 0
}

/// entry_ref for a native FS entry: an opaque identity token derived from
/// (root, parent_rel, name, ino). Not reversible; native entries are addressed
/// by (parent_ref, name) in mutating ops.
pub fn native_entry_ref(root: &str, parent_rel: &str, name: &str, ino: u64) -> String {
    let mut h = Sha256::new();
    h.update(root.as_bytes());
    h.update([0]);
    h.update(parent_rel.as_bytes());
    h.update([0]);
    h.update(name.as_bytes());
    h.update([0]);
    h.update(ino.to_le_bytes());
    format!("ne_{}", hex::encode(&h.finalize()[..12]))
}

/// Stable watch token for a container key (stateless: keyed hash).
pub fn watch_token(key: &[u8], container_key: &str) -> String {
    let mac = hmac16(key, container_key.as_bytes());
    format!("w_{}", hex::encode(&mac[..10]))
}

/// Stateless list cursor (NFSP §5.2): encodes revision-at-start, order and the
/// last emitted sort key. `c_<b64url(json)>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cursor {
    pub v: u8,
    /// container revision when the listing started
    pub rev: String,
    pub order: String,
    /// last sort key emitted (order-specific tuple)
    pub key: Vec<serde_json::Value>,
}

impl Cursor {
    pub fn encode(&self) -> String {
        format!("c_{}", B64.encode(serde_json::to_vec(self).expect("cursor")))
    }

    pub fn decode(s: &str) -> NfsResult<Cursor> {
        let rest = s.strip_prefix("c_").ok_or_else(|| invalid("bad cursor"))?;
        let bytes = B64.decode(rest).map_err(|_| invalid("bad cursor"))?;
        let c: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid("bad cursor"))?;
        if c.v != 1 {
            return Err(invalid("bad cursor version"));
        }
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codec() -> HandleCodec {
        HandleCodec::new(vec![7u8; 32])
    }

    #[test]
    fn handle_roundtrip() {
        let c = codec();
        let h = NativeHandle { r: "home".into(), p: "a/b.txt".into(), i: 42, k: "f".into() };
        let s = c.encode(&h);
        assert!(s.starts_with("nh_"));
        assert_eq!(c.decode(&s).unwrap(), h);
    }

    #[test]
    fn tampered_handle_is_stale() {
        let c = codec();
        let h = NativeHandle { r: "home".into(), p: "x".into(), i: 1, k: "d".into() };
        let s = c.encode(&h);
        // Flip a payload char.
        let mut bytes = s.into_bytes();
        let dot = bytes.iter().position(|&b| b == b'.').unwrap();
        bytes[dot - 1] = if bytes[dot - 1] == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(bytes).unwrap();
        let err = c.decode(&tampered).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::Stale);
        // A different key also fails.
        let c2 = HandleCodec::new(vec![9u8; 32]);
        let s = c.encode(&h);
        assert!(c2.decode(&s).is_err());
    }

    #[test]
    fn cursor_roundtrip() {
        let cur = Cursor {
            v: 1,
            rev: "d-1-2".into(),
            order: "name".into(),
            key: vec![serde_json::json!("last.txt")],
        };
        let s = cur.encode();
        assert_eq!(Cursor::decode(&s).unwrap(), cur);
        assert!(Cursor::decode("c_!!!").is_err());
        assert!(Cursor::decode("nope").is_err());
    }

    #[test]
    fn entry_ref_stable_and_distinct() {
        let a = native_entry_ref("home", "d", "x.txt", 5);
        let b = native_entry_ref("home", "d", "x.txt", 5);
        let c = native_entry_ref("home", "d", "x.txt", 6);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a.starts_with("ne_"));
    }
}
