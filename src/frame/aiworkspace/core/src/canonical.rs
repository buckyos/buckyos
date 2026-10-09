//! Canonical encoding and ObjectId calculation (design doc §2.10.1, §8.1).
//!
//! The rules are the CYFS ones (JCS + SHA-256, `type:hex` ids) re-implemented
//! without ndn-lib so this crate stays buildable for wasm32. `aiworkspace-store`
//! carries the conformance test against ndn-lib itself.

use crate::error::{Code, WsError, WsResult};
use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use sha2::{Digest, Sha256};

pub const OBJ_TYPE_JSON: &str = "jobj";
pub const OBJ_TYPE_FILE: &str = "cyfile";
pub const CHUNK_TYPE_MIX256: &str = "mix256";
/// Single-chunk ceiling of the named store (`put_chunk_by_reader`).
pub const MAX_CHUNK_SIZE: u64 = 32 * 1024 * 1024;
pub const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// `type:hex` — the serde form of ndn-lib's `ObjId`. Never the base32 `Display` form.
pub type ObjId = String;

pub fn is_obj_id(s: &str) -> bool {
    match s.split_once(':') {
        Some((t, h)) => {
            !t.is_empty()
                && t.len() <= 64
                && t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
                && !h.is_empty()
                && h.len() % 2 == 0
                && h.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
        }
        None => false,
    }
}

/// `ObjId::to_filename()`: `<hex>.<type>`.
pub fn obj_id_to_filename(id: &str) -> Option<String> {
    let (t, h) = id.split_once(':')?;
    Some(format!("{h}.{t}"))
}

pub fn obj_id_from_filename(name: &str) -> Option<ObjId> {
    let (h, t) = name.split_once('.')?;
    let id = format!("{t}:{h}");
    is_obj_id(&id).then_some(id)
}

/// Reject everything JCS/JS would encode differently from `serde_jcs`:
/// integers beyond 2^53-1 and negative zero. Non-finite numbers cannot exist in
/// a `serde_json::Value`.
pub fn precheck(v: &Value) -> WsResult<()> {
    match v {
        Value::Number(n) => check_number(n),
        Value::Array(a) => a.iter().try_for_each(precheck),
        Value::Object(m) => m.values().try_for_each(precheck),
        _ => Ok(()),
    }
}

fn check_number(n: &Number) -> WsResult<()> {
    if let Some(u) = n.as_u64() {
        if u > MAX_SAFE_INTEGER {
            return Err(WsError::invalid_schema("integer beyond 2^53-1; use a decimal string"));
        }
    } else if let Some(i) = n.as_i64() {
        if i.unsigned_abs() > MAX_SAFE_INTEGER {
            return Err(WsError::invalid_schema("integer beyond 2^53-1; use a decimal string"));
        }
    } else if let Some(f) = n.as_f64() {
        if !f.is_finite() {
            return Err(WsError::invalid_schema("non-finite number"));
        }
        if f == 0.0 && f.is_sign_negative() {
            return Err(WsError::invalid_schema("negative zero"));
        }
    }
    Ok(())
}

/// Strict pre-validation followed by RFC 8785 serialization. Fails instead of
/// degrading: bad input never obtains the id of some other (empty) object.
pub fn canonical_json(v: &Value) -> WsResult<String> {
    precheck(v)?;
    serde_jcs::to_string(v).map_err(|e| WsError::invalid_schema(format!("canonicalize failed: {e}")))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

pub fn build_obj_id(obj_type: &str, canonical: &str) -> ObjId {
    format!("{obj_type}:{}", sha256_hex(canonical.as_bytes()))
}

/// Canonical text and id of a generic JSON NamedObject.
pub fn named_object(obj_type: &str, v: &Value) -> WsResult<(ObjId, String)> {
    let s = canonical_json(v)?;
    Ok((build_obj_id(obj_type, &s), s))
}

/// Verify stored text against its id: the text must already be canonical.
pub fn verify_named_object(id: &str, text: &str) -> WsResult<Value> {
    let (t, _) = id
        .split_once(':')
        .ok_or_else(|| WsError::invalid_schema(format!("bad object id {id}")))?;
    let v = parse_strict(text)?;
    let canon = canonical_json(&v)?;
    if canon != text {
        return Err(WsError::invalid_schema(format!("object {id} is not canonical JSON")));
    }
    if build_obj_id(t, text) != id {
        return Err(WsError::invalid_schema(format!("object {id} content hash mismatch")));
    }
    Ok(v)
}

fn varint(mut n: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return out;
        }
        out.push(b | 0x80);
    }
}

/// `mix256` chunk id: unsigned-varint length prefix + SHA-256.
pub fn chunk_id(data: &[u8]) -> ObjId {
    let mut raw = varint(data.len() as u64);
    raw.extend_from_slice(&Sha256::digest(data));
    format!("{CHUNK_TYPE_MIX256}:{}", hex::encode(raw))
}

/// Length encoded in a `mix256` chunk id.
pub fn chunk_len(id: &str) -> Option<u64> {
    let (t, h) = id.split_once(':')?;
    if t != CHUNK_TYPE_MIX256 {
        return None;
    }
    let raw = hex::decode(h).ok()?;
    let (mut n, mut shift) = (0u64, 0);
    for (i, b) in raw.iter().enumerate() {
        n |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return (raw.len() == i + 1 + 32).then_some(n);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

/// The deterministic single-chunk FileObject used for materialized files and
/// assets. Timestamps are fixed at 0 so equal bytes give equal ids.
pub fn file_object(data_len: u64, chunk: &str) -> WsResult<(ObjId, String)> {
    let mut m = Map::new();
    m.insert("content".into(), Value::String(chunk.to_string()));
    m.insert("create_time".into(), Value::from(0));
    m.insert("last_update_time".into(), Value::from(0));
    if data_len > 0 {
        m.insert("size".into(), Value::from(data_len));
    }
    named_object(OBJ_TYPE_FILE, &Value::Object(m))
}

// ---- strict JSON parsing: duplicate keys are an error, not last-wins ----

struct Strict;

impl<'de> DeserializeSeed<'de> for Strict {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Strict {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON")
    }
    fn visit_bool<E>(self, v: bool) -> Result<Value, E> {
        Ok(Value::Bool(v))
    }
    fn visit_i64<E>(self, v: i64) -> Result<Value, E> {
        Ok(Value::from(v))
    }
    fn visit_u64<E>(self, v: u64) -> Result<Value, E> {
        Ok(Value::from(v))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        Number::from_f64(v).map(Value::Number).ok_or_else(|| E::custom("non-finite number"))
    }
    fn visit_str<E>(self, v: &str) -> Result<Value, E> {
        Ok(Value::String(v.to_string()))
    }
    fn visit_string<E>(self, v: String) -> Result<Value, E> {
        Ok(Value::String(v))
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut out = Vec::new();
        while let Some(v) = a.next_element_seed(Strict)? {
            out.push(v);
        }
        Ok(Value::Array(out))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Value, A::Error> {
        let mut out = Map::new();
        while let Some(k) = a.next_key::<String>()? {
            let v = a.next_value_seed(Strict)?;
            if out.insert(k.clone(), v).is_some() {
                return Err(serde::de::Error::custom(format!("duplicate key {k:?}")));
            }
        }
        Ok(Value::Object(out))
    }
}

/// Parse untrusted JSON text, rejecting duplicate keys and trailing data.
pub fn parse_strict(text: &str) -> WsResult<Value> {
    let mut de = serde_json::Deserializer::from_str(text);
    let v = Strict
        .deserialize(&mut de)
        .map_err(|e| WsError::new(Code::InvalidSchema, format!("invalid JSON: {e}")))?;
    de.end().map_err(|e| WsError::new(Code::InvalidSchema, format!("invalid JSON: {e}")))?;
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rfc8785_ordering_and_numbers() {
        // Keys sort by UTF-16 code units; strings are kept as-is (no NFC).
        let v = json!({ "\u{20ac}": "Euro Sign", "\r": "Carriage Return", "\u{fb33}": "Hebrew",
                        "1": "One", "\u{1f600}": "Emoji", "\u{80}": "Control", "\u{f6}": "Latin" });
        let s = canonical_json(&v).unwrap();
        let keys: Vec<&str> = vec!["\\r", "1", "\u{80}", "\u{f6}", "\u{20ac}", "\u{1f600}", "\u{fb33}"];
        let mut pos = 0;
        for k in keys {
            let p = s[pos..].find(k).unwrap_or_else(|| panic!("{k} out of order in {s}"));
            pos += p;
        }
        assert_eq!(canonical_json(&json!({"b": 1.0, "a": 1e21, "c": 0.1})).unwrap(), r#"{"a":1e+21,"b":1,"c":0.1}"#);
        // non-NFC input is preserved byte for byte
        let decomposed = "e\u{301}";
        assert!(canonical_json(&json!(decomposed)).unwrap().contains(decomposed));
    }

    #[test]
    fn rejects() {
        assert!(canonical_json(&json!(9007199254740992u64)).is_err());
        assert!(canonical_json(&json!(-9007199254740992i64)).is_err());
        assert!(canonical_json(&json!(9007199254740991u64)).is_ok());
        assert!(canonical_json(&json!(-0.0)).is_err());
        assert!(parse_strict(r#"{"a":1,"a":2}"#).is_err());
        assert!(parse_strict(r#"{"a":{"b":1,"b":1}}"#).is_err());
        assert!(parse_strict(r#"{"a":1} x"#).is_err());
        assert!(parse_strict(r#"{"a":null,"b":[1,"x"]}"#).is_ok());
    }

    #[test]
    fn ids() {
        let (id, text) = named_object(OBJ_TYPE_JSON, &json!({"b": 1, "a": null})).unwrap();
        assert_eq!(text, r#"{"a":null,"b":1}"#);
        assert!(is_obj_id(&id));
        assert!(verify_named_object(&id, &text).is_ok());
        assert!(verify_named_object(&id, r#"{"a":null,"b":2}"#).is_err());
        assert!(verify_named_object(&id, r#"{"b":1,"a":null}"#).is_err());
        // explicit null and omitted are different objects
        assert_ne!(id, named_object(OBJ_TYPE_JSON, &json!({"b": 1})).unwrap().0);
        let c = chunk_id(b"hello");
        assert_eq!(chunk_len(&c), Some(5));
        let big = vec![0u8; 300];
        assert_eq!(chunk_len(&chunk_id(&big)), Some(300));
        assert_eq!(obj_id_from_filename(&obj_id_to_filename(&id).unwrap()).unwrap(), id);
    }
}
