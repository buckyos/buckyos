//! Typed values (design doc §3.4.2): the single implementation shared by table
//! fields and RecordObject properties. Values are normalized on write; what is
//! stored is always the canonical form. Invalid input is rejected, never coerced.

use crate::canonical::MAX_SAFE_INTEGER;
use crate::error::{Code, WsError, WsResult};
use crate::id::is_valid_id;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Text,
    Boolean,
    Number,
    Decimal,
    Date,
    Datetime,
    Select,
    MultiSelect,
    ObjectRef,
    /// A money amount with its ISO 4217 currency (表格导入 TODO §6, format 1.1).
    Currency,
}

impl FieldType {
    pub fn as_str(&self) -> &'static str {
        match self {
            FieldType::Text => "text",
            FieldType::Boolean => "boolean",
            FieldType::Number => "number",
            FieldType::Decimal => "decimal",
            FieldType::Date => "date",
            FieldType::Datetime => "datetime",
            FieldType::Select => "select",
            FieldType::MultiSelect => "multi_select",
            FieldType::ObjectRef => "object_ref",
            FieldType::Currency => "currency",
        }
    }
    pub fn sortable(&self) -> bool {
        !matches!(self, FieldType::MultiSelect | FieldType::ObjectRef)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OptionDef {
    pub option_id: String,
    pub label: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDef {
    pub field_id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: FieldType,
    #[serde(default, skip_serializing_if = "is_false")]
    pub required: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub nullable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub unique: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `human` (default, omitted) | `program`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maintained_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<OptionDef>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_types: Option<Vec<String>>,
    /// The default currency of a `currency` field (ISO 4217 code), given to values written without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
}

pub const TEXT_MAX_UTF16: usize = 65_536;
/// The scale of a `currency` field that does not give one.
pub const CURRENCY_DEFAULT_SCALE: u8 = 2;

/// An ISO 4217 code: three uppercase ASCII letters (the list itself is not checked).
pub fn is_currency_code(s: &str) -> bool {
    s.len() == 3 && s.bytes().all(|c| c.is_ascii_uppercase())
}

impl FieldDef {
    pub fn from_value(v: &Value) -> WsResult<FieldDef> {
        let mut def: FieldDef = serde_json::from_value(v.clone())
            .map_err(|e| WsError::invalid_schema(format!("bad field definition: {e}")))?;
        def.validate()?;
        if def.maintained_by.as_deref() == Some("human") {
            def.maintained_by = None;
        }
        Ok(def)
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("field def")
    }

    pub fn validate(&self) -> WsResult<()> {
        let bad = |m: String| Err(WsError::invalid_schema(m));
        if !is_valid_id(&self.field_id) {
            return bad(format!("invalid field_id {:?}", self.field_id));
        }
        if self.name.is_empty() || self.name.chars().count() > 128 {
            return bad("field name must be 1-128 chars".into());
        }
        match (self.ty, self.scale) {
            (FieldType::Decimal, Some(s)) if s <= 18 => {}
            (FieldType::Decimal, _) => return bad("decimal field needs scale 0-18".into()),
            (FieldType::Currency, Some(s)) if s > 18 => return bad("currency scale must be 0-18".into()),
            (FieldType::Currency, _) => {}
            (_, Some(_)) => return bad("scale is only valid on decimal and currency fields".into()),
            _ => {}
        }
        let is_select = matches!(self.ty, FieldType::Select | FieldType::MultiSelect);
        match (&self.options, is_select) {
            (Some(opts), true) => {
                let mut seen = std::collections::BTreeSet::new();
                for o in opts {
                    if !is_valid_id(&o.option_id) || !seen.insert(&o.option_id) {
                        return bad(format!("invalid or duplicate option_id {:?}", o.option_id));
                    }
                    if o.label.is_empty() {
                        return bad("option label must not be empty".into());
                    }
                }
            }
            (None, true) => return bad("select field needs options".into()),
            (Some(_), false) => return bad("options are only valid on select fields".into()),
            (None, false) => {}
        }
        if self.unique
            && !matches!(self.ty, FieldType::Text | FieldType::Number | FieldType::Decimal | FieldType::Date)
        {
            return bad("unique is only supported on text/number/decimal/date".into());
        }
        if self.target_types.is_some() && self.ty != FieldType::ObjectRef {
            return bad("target_types is only valid on object_ref fields".into());
        }
        match (&self.currency, self.ty) {
            (Some(c), FieldType::Currency) if is_currency_code(c) => {}
            (Some(c), FieldType::Currency) => return bad(format!("currency must be an ISO 4217 code, got {c:?}")),
            (None, FieldType::Currency) => return bad("currency field needs its default currency".into()),
            (Some(_), _) => return bad("currency is only valid on currency fields".into()),
            (None, _) => {}
        }
        if let Some(m) = &self.maintained_by {
            if m != "human" && m != "program" {
                return bad("maintained_by must be human or program".into());
            }
        }
        Ok(())
    }

    pub fn has_option(&self, id: &str) -> bool {
        self.options.as_ref().is_some_and(|o| o.iter().any(|x| x.option_id == id))
    }
    pub fn option_index(&self, id: &str) -> Option<usize> {
        self.options.as_ref()?.iter().position(|x| x.option_id == id)
    }
    pub fn option_label(&self, id: &str) -> Option<&str> {
        self.options.as_ref()?.iter().find(|x| x.option_id == id).map(|x| x.label.as_str())
    }
    /// The scale a `currency` field's amounts are written with.
    pub fn currency_scale(&self) -> u8 {
        self.scale.unwrap_or(CURRENCY_DEFAULT_SCALE)
    }

    /// Normalize a non-absent value. `null` passes only on nullable fields.
    /// `object_ref` is normalized structurally; target existence is the caller's check.
    pub fn normalize(&self, v: &Value) -> WsResult<Value> {
        if is_formula(v) {
            // a formula of this field's return type (D18): saved as given, evaluated by readers, never here
            return check_formula(v).map(|_| v.clone()).map_err(|m| WsError::invalid_schema(format!("field {}: {m}", self.field_id)));
        }
        if v.is_null() {
            return if self.nullable {
                Ok(Value::Null)
            } else {
                Err(WsError::invalid_schema(format!("field {} is not nullable", self.field_id)))
            };
        }
        let bad = |m: &str| WsError::invalid_schema(format!("field {}: {m}", self.field_id));
        match self.ty {
            FieldType::Text => {
                let s = v.as_str().ok_or_else(|| bad("expected string"))?;
                if s.encode_utf16().count() > TEXT_MAX_UTF16 {
                    return Err(WsError::limit(format!("field {}: text too long", self.field_id)));
                }
                Ok(v.clone())
            }
            FieldType::Boolean => v.is_boolean().then(|| v.clone()).ok_or_else(|| bad("expected boolean")),
            FieldType::Number => normalize_number(v).map_err(|m| bad(&m)),
            FieldType::Decimal => {
                let s = v.as_str().ok_or_else(|| bad("expected decimal string"))?;
                normalize_decimal(s, self.scale.unwrap_or(0)).map(Value::String).map_err(|m| bad(&m))
            }
            FieldType::Date => {
                let s = v.as_str().ok_or_else(|| bad("expected date string"))?;
                parse_date(s).map(|_| v.clone()).ok_or_else(|| bad("expected YYYY-MM-DD"))
            }
            FieldType::Datetime => {
                let s = v.as_str().ok_or_else(|| bad("expected datetime string"))?;
                normalize_datetime(s)
                    .map(Value::String)
                    .ok_or_else(|| bad("expected RFC 3339 datetime with offset or Z"))
            }
            FieldType::Select => {
                let s = v.as_str().ok_or_else(|| bad("expected option id"))?;
                if self.has_option(s) {
                    Ok(v.clone())
                } else {
                    Err(bad("unknown option"))
                }
            }
            FieldType::MultiSelect => {
                let a = v.as_array().ok_or_else(|| bad("expected array of option ids"))?;
                let mut set = std::collections::BTreeSet::new();
                for x in a {
                    let s = x.as_str().ok_or_else(|| bad("expected option id"))?;
                    if !self.has_option(s) {
                        return Err(bad("unknown option"));
                    }
                    set.insert(s.to_string());
                }
                Ok(Value::Array(set.into_iter().map(Value::String).collect()))
            }
            FieldType::ObjectRef => normalize_reference(v),
            FieldType::Currency => self.normalize_currency(v).map_err(|m| bad(&m)),
        }
    }

    /// `{ amount, currency }` (amount a decimal string or a number), or a bare amount in the field's default
    /// currency; stored as `{ "amount": "<decimal of the field's scale>", "currency": "<code>" }` — the currency is
    /// always written out, so changing the field's default never changes a stored value. Symbols and names (`$12`,
    /// `USD 12`) are the importer's and the editor's to resolve, not the core's.
    fn normalize_currency(&self, v: &Value) -> Result<Value, String> {
        let (amount, code) = match v {
            Value::Object(o) => {
                if o.keys().any(|k| k != "amount" && k != "currency") {
                    return Err("currency value takes only amount and currency".into());
                }
                let code = o.get("currency").and_then(Value::as_str).ok_or("currency value needs its currency code")?;
                if !is_currency_code(code) {
                    return Err("currency must be an ISO 4217 code".into());
                }
                (o.get("amount").ok_or("currency value needs its amount")?, code.to_string())
            }
            Value::String(_) | Value::Number(_) => (v, self.currency.clone().ok_or("currency field has no default currency")?),
            _ => return Err("expected { amount, currency } or an amount".into()),
        };
        let text = match amount {
            Value::String(s) => s.clone(),
            Value::Number(_) => plain_number(amount)?,
            _ => return Err("amount must be a decimal string or a number".into()),
        };
        Ok(json!({ "amount": normalize_decimal(&text, self.currency_scale())?, "currency": code }))
    }
}

/// A JSON number as plain decimal text (no exponent).
fn plain_number(v: &Value) -> Result<String, String> {
    let s = if let Some(i) = v.as_i64() {
        i.to_string()
    } else if let Some(u) = v.as_u64() {
        u.to_string()
    } else {
        let f = v.as_f64().filter(|f| f.is_finite()).ok_or("not a finite number")?;
        format!("{f}")
    };
    if s.contains('e') || s.contains('E') {
        return Err("number not representable as plain decimal".into());
    }
    Ok(s)
}

fn normalize_number(v: &Value) -> Result<Value, String> {
    let n = v.as_number().ok_or("expected number")?;
    if let Some(u) = n.as_u64() {
        return if u > MAX_SAFE_INTEGER { Err("integer beyond 2^53-1; use decimal".into()) } else { Ok(v.clone()) };
    }
    if let Some(i) = n.as_i64() {
        return if i.unsigned_abs() > MAX_SAFE_INTEGER {
            Err("integer beyond 2^53-1; use decimal".into())
        } else {
            Ok(v.clone())
        };
    }
    let f = n.as_f64().ok_or("expected finite number")?;
    if !f.is_finite() {
        return Err("expected finite number".into());
    }
    if f == 0.0 {
        return Ok(json!(0));
    }
    if f.fract() == 0.0 && f.abs() <= MAX_SAFE_INTEGER as f64 {
        return Ok(json!(f as i64)); // 3.0 and 3 are the same stored value
    }
    if f.fract() == 0.0 && f.abs() > MAX_SAFE_INTEGER as f64 {
        return Err("integer beyond 2^53-1; use decimal".into());
    }
    Ok(v.clone())
}

/// `^-?\d+(\.\d+)?$`, ≤ 38 significant digits, fraction ≤ scale (never rounded).
/// Canonical: no redundant leading zeros, fraction padded to exactly `scale`,
/// negative zero written as zero.
pub fn normalize_decimal(s: &str, scale: u8) -> Result<String, String> {
    let (neg, body) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => (i, f),
        None => (body, ""),
    };
    let digits = |x: &str| !x.is_empty() && x.bytes().all(|c| c.is_ascii_digit());
    if !digits(int) || (body.contains('.') && !digits(frac)) {
        return Err("invalid decimal syntax".into());
    }
    let frac_trim = frac.trim_end_matches('0');
    if frac_trim.len() > scale as usize {
        return Err(format!("more than {scale} fraction digits"));
    }
    let int_trim = int.trim_start_matches('0');
    if int_trim.len() + frac_trim.len() > 38 {
        return Err("more than 38 significant digits".into());
    }
    let int_out = if int_trim.is_empty() { "0" } else { int_trim };
    let mut out = String::new();
    let is_zero = int_trim.is_empty() && frac_trim.is_empty();
    if neg && !is_zero {
        out.push('-');
    }
    out.push_str(int_out);
    if scale > 0 {
        out.push('.');
        out.push_str(frac_trim);
        for _ in frac_trim.len()..scale as usize {
            out.push('0');
        }
    }
    Ok(out)
}

/// Exact comparison of two canonical decimals (possibly of different scale).
pub fn cmp_decimal(a: &str, b: &str) -> Ordering {
    fn parts(s: &str) -> (bool, &str, &str) {
        let (neg, body) = match s.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let (i, f) = body.split_once('.').unwrap_or((body, ""));
        (neg, i.trim_start_matches('0'), f.trim_end_matches('0'))
    }
    let (na, ia, fa) = parts(a);
    let (nb, ib, fb) = parts(b);
    let za = ia.is_empty() && fa.is_empty();
    let zb = ib.is_empty() && fb.is_empty();
    let (na, nb) = (na && !za, nb && !zb);
    let mag = ia.len().cmp(&ib.len()).then_with(|| ia.cmp(ib)).then_with(|| fa.cmp(fb));
    match (na, nb) {
        (false, false) => mag,
        (true, true) => mag.reverse(),
        (true, false) => Ordering::Less,
        (false, true) => Ordering::Greater,
    }
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => 28 + is_leap(y) as i64,
    }
}
fn num(s: &str) -> Option<i64> {
    (!s.is_empty() && s.bytes().all(|c| c.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// `YYYY-MM-DD`, a valid Gregorian date in years 0001–9999.
pub fn parse_date(s: &str) -> Option<(i64, i64, i64)> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let (y, m, d) = (num(&s[0..4])?, num(&s[5..7])?, num(&s[8..10])?);
    ((1..=9999).contains(&y) && (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m)).then_some((y, m, d))
}

/// Days since 1970-01-01 (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Format milliseconds since the epoch as `YYYY-MM-DDTHH:MM:SS.mmmZ`.
pub fn format_utc_ms(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y,
        m,
        d,
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// Parse an RFC 3339 instant that carries `Z` or a numeric offset.
pub fn parse_datetime_ms(s: &str) -> Option<i64> {
    let (date, rest) = (s.get(0..10)?, s.get(10..)?);
    let (y, m, d) = parse_date(date)?;
    let rest = rest.strip_prefix('T').or_else(|| rest.strip_prefix('t'))?;
    let (time, off_min) = if let Some(t) = rest.strip_suffix('Z').or_else(|| rest.strip_suffix('z')) {
        (t, 0)
    } else {
        let pos = rest.rfind(['+', '-'])?;
        let (t, off) = rest.split_at(pos);
        let sign = if off.starts_with('-') { -1 } else { 1 };
        let off = &off[1..];
        if off.len() != 5 || off.as_bytes()[2] != b':' {
            return None;
        }
        let (oh, om) = (num(&off[0..2])?, num(&off[3..5])?);
        if oh > 23 || om > 59 {
            return None;
        }
        (t, sign * (oh * 60 + om))
    };
    let (hms, frac) = time.split_once('.').unwrap_or((time, ""));
    if hms.len() != 8 || hms.as_bytes()[2] != b':' || hms.as_bytes()[5] != b':' {
        return None;
    }
    let (h, mi, sec) = (num(&hms[0..2])?, num(&hms[3..5])?, num(&hms[6..8])?);
    if h > 23 || mi > 59 || sec > 59 || (time.contains('.') && num(frac).is_none()) {
        return None;
    }
    let mut ms = 0;
    for (i, c) in frac.bytes().take(3).enumerate() {
        ms += (c - b'0') as i64 * [100, 10, 1][i];
    }
    Some(days_from_civil(y, m, d) * 86_400_000 + h * 3_600_000 + mi * 60_000 + sec * 1000 + ms - off_min * 60_000)
}

pub fn normalize_datetime(s: &str) -> Option<String> {
    let ms = parse_datetime_ms(s)?;
    let out = format_utc_ms(ms);
    parse_date(&out[0..10]).map(|_| out)
}

/// Structural normalization of a persisted reference (design doc §2.4).
/// Internal references carry no `workspace_id`; `version` defaults to live head.
pub fn normalize_reference(v: &Value) -> WsResult<Value> {
    let bad = |m: &str| WsError::invalid_schema(format!("bad reference: {m}"));
    let o = v.as_object().ok_or_else(|| bad("expected object"))?;
    for k in o.keys() {
        if !matches!(k.as_str(), "workspace_id" | "entity_id" | "version" | "selector") {
            return Err(bad("unknown key"));
        }
    }
    let entity_id = o.get("entity_id").and_then(Value::as_str).ok_or_else(|| bad("entity_id required"))?;
    if !is_valid_id(entity_id) {
        return Err(bad("invalid entity_id"));
    }
    let mut out = Map::new();
    if let Some(ws) = o.get("workspace_id") {
        let ws = ws.as_str().filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| bad("workspace_id"))?;
        out.insert("workspace_id".into(), json!(ws));
    }
    out.insert("entity_id".into(), json!(entity_id));
    let version = match o.get("version") {
        None => json!({ "mode": "live_head" }),
        Some(ver) => {
            let mode = ver.get("mode").and_then(Value::as_str).ok_or_else(|| bad("version.mode required"))?;
            match mode {
                "live_head" => json!({ "mode": "live_head" }),
                "fixed_revision" => {
                    let has_obj = ver.get("object_id").and_then(Value::as_str).is_some_and(crate::canonical::is_obj_id);
                    let has_src = ver.get("source_revision").is_some_and(Value::is_string);
                    if has_obj == has_src {
                        return Err(bad("fixed_revision needs exactly one of object_id / source_revision"));
                    }
                    ver.clone()
                }
                "published_channel" => {
                    ver.get("channel").and_then(Value::as_str).ok_or_else(|| bad("channel required"))?;
                    ver.clone()
                }
                _ => return Err(bad("unknown version.mode")),
            }
        }
    };
    out.insert("version".into(), version);
    if let Some(sel) = o.get("selector") {
        let kind = sel.get("kind").and_then(Value::as_str).ok_or_else(|| bad("selector.kind required"))?;
        if kind != "entity" {
            out.insert("selector".into(), sel.clone());
        }
    }
    Ok(Value::Object(out))
}

/// The frozen reference object (内核架构改进 D10, D03; `schemas/reference.schema.json`):
/// `{ zone?, workspace_id?, entity_id, selector?, mode: live | fixed, version?: { epoch, seq } | { content_root, path? } }`.
/// Accepts that shape, or any of the older vocabularies it replaces — a persisted reference
/// (`version.mode: live_head | fixed_revision | published_channel`), a wish input
/// (`version.mode: follow | fixed`) — and returns the frozen shape. A `fixed` version that only
/// names a cell revision or a URL source revision has no identity of its own and is refused.
pub fn canonical_reference(v: &Value) -> WsResult<Value> {
    let bad = |m: &str| WsError::invalid_schema(format!("bad reference: {m}"));
    let o = v.as_object().ok_or_else(|| bad("expected object"))?;
    for k in o.keys() {
        if !matches!(k.as_str(), "zone" | "workspace_id" | "entity_id" | "selector" | "mode" | "version" | "name" | "label") {
            return Err(bad(&format!("unknown key {k}")));
        }
    }
    let entity_id = o.get("entity_id").and_then(Value::as_str).filter(|s| is_valid_id(s)).ok_or_else(|| bad("entity_id required"))?;
    let mut out = Map::new();
    if let Some(z) = o.get("zone") {
        let z = z.as_str().filter(|s| !s.is_empty() && s.len() <= 256).ok_or_else(|| bad("zone"))?;
        out.insert("zone".into(), json!(z));
    }
    if let Some(ws) = o.get("workspace_id") {
        let ws = ws.as_str().filter(|s| !s.is_empty() && s.len() <= 64).ok_or_else(|| bad("workspace_id"))?;
        out.insert("workspace_id".into(), json!(ws));
    }
    out.insert("entity_id".into(), json!(entity_id));
    if let Some(sel) = o.get("selector").filter(|s| !s.is_null()) {
        let kind = sel.get("kind").and_then(Value::as_str).ok_or_else(|| bad("selector.kind required"))?;
        if kind != "entity" {
            out.insert("selector".into(), sel.clone());
        }
    }
    let fixed_root = |root: &str, path: Option<&Value>| -> WsResult<Value> {
        if !crate::canonical::is_obj_id(root) {
            return Err(bad("version.content_root must be an object id"));
        }
        let mut v = json!({ "content_root": root });
        if let Some(p) = path {
            let p = p.as_str().filter(|s| !s.is_empty() && s.len() <= 1024).ok_or_else(|| bad("version.path"))?;
            v["path"] = json!(p);
        }
        Ok(v)
    };
    let (mode, version): (&str, Option<Value>) = match (o.get("mode").and_then(Value::as_str), o.get("version")) {
        // the frozen shape
        (Some("live"), None) => ("live", None),
        (Some("live"), Some(_)) => return Err(bad("a live reference carries no version")),
        (Some("fixed"), Some(ver)) => {
            let m = ver.as_object().ok_or_else(|| bad("version must be an object"))?;
            if let Some(root) = m.get("content_root") {
                if m.keys().any(|k| !matches!(k.as_str(), "content_root" | "path")) {
                    return Err(bad("version: content_root takes only path"));
                }
                ("fixed", Some(fixed_root(root.as_str().unwrap_or(""), m.get("path"))?))
            } else {
                let epoch = m.get("epoch").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad("version needs { epoch, seq } or { content_root, path? }"))?;
                let seq = m.get("seq").and_then(Value::as_u64).ok_or_else(|| bad("version.seq must be a non-negative integer"))?;
                if m.len() != 2 {
                    return Err(bad("version: epoch takes only seq"));
                }
                ("fixed", Some(json!({ "epoch": epoch, "seq": seq })))
            }
        }
        (Some("fixed"), None) => return Err(bad("a fixed reference needs its version")),
        (Some(m), _) => return Err(bad(&format!("unknown mode {m}"))),
        // the older vocabularies: `version.mode` decides
        (None, ver) => {
            let m = ver.and_then(|v| v.get("mode")).and_then(Value::as_str).unwrap_or("live_head");
            match m {
                "live_head" | "follow" | "published_channel" => ("live", None),
                "fixed_revision" | "fixed" => {
                    let ver = ver.unwrap();
                    match ver.get("object_id").and_then(Value::as_str) {
                        Some(root) => ("fixed", Some(fixed_root(root, None)?)),
                        None if ver.get("seq").and_then(Value::as_u64).is_some() => {
                            let epoch = ver.get("epoch").and_then(Value::as_str).ok_or_else(|| bad("a fixed { seq } needs its epoch"))?;
                            ("fixed", Some(json!({ "epoch": epoch, "seq": ver["seq"] })))
                        }
                        None if ver.get("source_revision").is_some() => return Err(bad("a URL source revision is a query entry's own version, not a Workspace version (§6.1)")),
                        None => return Err(bad("a fixed cell revision (rev / hash) is a read-set precondition, not a reference version")),
                    }
                }
                m => return Err(bad(&format!("unknown version.mode {m}"))),
            }
        }
    };
    out.insert("mode".into(), json!(mode));
    if let Some(v) = version {
        out.insert("version".into(), v);
    }
    Ok(Value::Object(out))
}

/// The persisted (`live_head` / `fixed_revision`) form of a frozen reference, for the code paths that
/// still speak it. A `{ epoch, seq }` version has no persisted form yet (T1-12 resolves it through retention).
pub fn reference_from_canonical(v: &Value) -> WsResult<Value> {
    let mut out = Map::new();
    for k in ["workspace_id", "entity_id", "selector"] {
        if let Some(x) = v.get(k) {
            out.insert(k.into(), x.clone());
        }
    }
    let version = match (v["mode"].as_str(), v.get("version")) {
        (Some("live"), _) => json!({ "mode": "live_head" }),
        (Some("fixed"), Some(ver)) if ver.get("content_root").is_some() => json!({ "mode": "fixed_revision", "object_id": ver["content_root"] }),
        (Some("fixed"), Some(ver)) => return Err(WsError::new(Code::VersionNotRetained, format!("version {} of epoch {} is not retained by a checkpoint or clone", ver["seq"], ver["epoch"].as_str().unwrap_or("?")))),
        _ => return Err(WsError::invalid_schema("bad reference: mode required")),
    };
    out.insert("version".into(), version);
    Ok(Value::Object(out))
}

/// `{ "$formula": { lang, version, expr } }` — the one value shape a typed key may hold instead of a value
/// of its type (内核架构改进 D18): kept verbatim, not evaluated by the backend, treated as empty by sorting,
/// filtering and uniqueness.
pub fn is_formula(v: &Value) -> bool {
    v.as_object().is_some_and(|o| o.len() == 1 && o.contains_key("$formula"))
}

pub const FORMULA_EXPR_MAX_CHARS: usize = 4096;

pub fn check_formula(v: &Value) -> Result<(), String> {
    let f = v.get("$formula").and_then(Value::as_object).ok_or("$formula must be an object")?;
    for k in f.keys() {
        if !matches!(k.as_str(), "lang" | "version" | "expr") {
            return Err(format!("$formula: unknown key {k}"));
        }
    }
    if f.get("lang").and_then(Value::as_str) != Some("aiws") {
        return Err("$formula.lang must be \"aiws\"".into());
    }
    if !f.get("version").and_then(Value::as_str).is_some_and(|s| !s.is_empty() && s.len() <= 16) {
        return Err("$formula.version must be a short string".into());
    }
    match f.get("expr").and_then(Value::as_str) {
        Some(e) if !e.trim().is_empty() && e.chars().count() <= FORMULA_EXPR_MAX_CHARS => Ok(()),
        _ => Err(format!("$formula.expr must be a non-empty string of at most {FORMULA_EXPR_MAX_CHARS} characters")),
    }
}

pub fn reference_entity_id(v: &Value) -> Option<&str> {
    v.get("entity_id").and_then(Value::as_str)
}
pub fn reference_is_local(v: &Value) -> bool {
    v.get("workspace_id").is_none()
}

/// Total order between two present, non-null, canonical values of a sortable type.
pub fn compare(def: &FieldDef, a: &Value, b: &Value) -> Ordering {
    match def.ty {
        FieldType::Text | FieldType::Date | FieldType::Datetime => {
            // canonical date/datetime strings sort chronologically; text sorts by scalar values
            a.as_str().unwrap_or("").cmp(b.as_str().unwrap_or(""))
        }
        FieldType::Number => a
            .as_f64()
            .unwrap_or(0.0)
            .partial_cmp(&b.as_f64().unwrap_or(0.0))
            .unwrap_or(Ordering::Equal),
        FieldType::Decimal => cmp_decimal(a.as_str().unwrap_or("0"), b.as_str().unwrap_or("0")),
        FieldType::Boolean => a.as_bool().unwrap_or(false).cmp(&b.as_bool().unwrap_or(false)),
        FieldType::Select => {
            let ia = a.as_str().and_then(|s| def.option_index(s)).unwrap_or(usize::MAX);
            let ib = b.as_str().and_then(|s| def.option_index(s)).unwrap_or(usize::MAX);
            ia.cmp(&ib)
        }
        FieldType::MultiSelect | FieldType::ObjectRef => Ordering::Equal,
        // grouped by code, then by amount (different currencies are never converted)
        FieldType::Currency => currency_parts(a).0.cmp(currency_parts(b).0).then_with(|| cmp_decimal(currency_parts(a).1, currency_parts(b).1)),
    }
}

/// `(code, amount)` of a stored currency value.
pub fn currency_parts(v: &Value) -> (&str, &str) {
    (v.get("currency").and_then(Value::as_str).unwrap_or(""), v.get("amount").and_then(Value::as_str).unwrap_or("0"))
}

/// Equality used by `eq`/`ne`/`in` and unique checks.
pub fn values_equal(def: &FieldDef, a: &Value, b: &Value) -> bool {
    match def.ty {
        FieldType::Number => a.as_f64() == b.as_f64(),
        FieldType::Decimal => cmp_decimal(a.as_str().unwrap_or(""), b.as_str().unwrap_or("")) == Ordering::Equal,
        FieldType::ObjectRef => {
            reference_entity_id(a) == reference_entity_id(b) && a.get("workspace_id") == b.get("workspace_id")
        }
        FieldType::Currency => {
            let (ca, aa) = currency_parts(a);
            let (cb, ab) = currency_parts(b);
            ca == cb && cmp_decimal(aa, ab) == Ordering::Equal
        }
        _ => a == b,
    }
}

/// Strict conversions for `table.migrate_field` (design doc §3.4.6).
pub fn migrate_value(from: &FieldDef, to: &FieldDef, v: &Value) -> Result<Value, String> {
    if v.is_null() {
        return if to.nullable { Ok(Value::Null) } else { Err("null not allowed".into()) };
    }
    let conv = match (from.ty, to.ty) {
        (FieldType::Text, FieldType::Date) => v.clone(),
        (FieldType::Text, FieldType::Number) => {
            let s = v.as_str().unwrap_or("").trim_matches(|c: char| c.is_ascii_whitespace());
            let ok = {
                let body = s.strip_prefix('-').unwrap_or(s);
                let (i, f) = body.split_once('.').unwrap_or((body, "0"));
                !i.is_empty() && i.bytes().all(|c| c.is_ascii_digit()) && !f.is_empty() && f.bytes().all(|c| c.is_ascii_digit())
            };
            if !ok {
                return Err("not a number".into());
            }
            let f: f64 = s.parse().map_err(|_| "not a number".to_string())?;
            serde_json::Number::from_f64(f).map(Value::Number).ok_or("not a finite number")?
        }
        (FieldType::Text, FieldType::Decimal) => {
            json!(v.as_str().unwrap_or("").trim_matches(|c: char| c.is_ascii_whitespace()))
        }
        (FieldType::Number, FieldType::Decimal) => json!(plain_number(v)?),
        // the amount in the field's default currency
        (FieldType::Number, FieldType::Currency) => json!(plain_number(v)?),
        (FieldType::Decimal, FieldType::Currency) => v.clone(),
        // the currency is dropped (the field manager warns about values not in the default currency first)
        (FieldType::Currency, FieldType::Decimal) => json!(currency_parts(v).1),
        (FieldType::Currency, FieldType::Text) => {
            let (code, amount) = currency_parts(v);
            json!(format!("{code} {amount}"))
        }
        // another scale: as decimal → decimal; the currency stays
        (FieldType::Currency, FieldType::Currency) => {
            let (code, amount) = currency_parts(v);
            json!({ "amount": amount, "currency": code })
        }
        (FieldType::Select, FieldType::Text) => {
            json!(from.option_label(v.as_str().unwrap_or("")).ok_or("unknown option")?)
        }
        // another scale: widening pads, narrowing fails for values with more fraction digits
        (FieldType::Decimal, FieldType::Decimal) => v.clone(),
        _ => return Err("unsupported".into()),
    };
    to.normalize(&conv).map_err(|e| e.detail)
}

pub fn migration_supported(from: FieldType, to: FieldType) -> bool {
    matches!(
        (from, to),
        (FieldType::Text, FieldType::Date)
            | (FieldType::Text, FieldType::Number)
            | (FieldType::Text, FieldType::Decimal)
            | (FieldType::Number, FieldType::Decimal)
            | (FieldType::Decimal, FieldType::Decimal)
            | (FieldType::Select, FieldType::Text)
            | (FieldType::Number, FieldType::Currency)
            | (FieldType::Decimal, FieldType::Currency)
            | (FieldType::Currency, FieldType::Decimal)
            | (FieldType::Currency, FieldType::Text)
            | (FieldType::Currency, FieldType::Currency)
    )
}

pub fn err_unknown_field(field_id: &str) -> WsError {
    WsError::new(Code::NotFound, format!("unknown field {field_id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(v: Value) -> FieldDef {
        FieldDef::from_value(&v).unwrap()
    }

    #[test]
    fn decimal() {
        assert_eq!(normalize_decimal("1200", 2).unwrap(), "1200.00");
        assert_eq!(normalize_decimal("001200.5", 2).unwrap(), "1200.50");
        assert_eq!(normalize_decimal("-0.00", 2).unwrap(), "0.00");
        assert_eq!(normalize_decimal("-0.10", 2).unwrap(), "-0.10");
        assert_eq!(normalize_decimal("7", 0).unwrap(), "7");
        assert!(normalize_decimal("1.005", 2).is_err());
        assert!(normalize_decimal("1.", 2).is_err());
        assert!(normalize_decimal(".5", 2).is_err());
        assert!(normalize_decimal("1e3", 2).is_err());
        assert!(normalize_decimal(&"9".repeat(39), 0).is_err());
        assert_eq!(cmp_decimal("10.00", "9.50"), Ordering::Greater);
        assert_eq!(cmp_decimal("-10.00", "9.50"), Ordering::Less);
        assert_eq!(cmp_decimal("-10.00", "-9.50"), Ordering::Less);
        assert_eq!(cmp_decimal("1.50", "1.5"), Ordering::Equal);
    }

    #[test]
    fn dates() {
        assert!(parse_date("2026-02-28").is_some());
        assert!(parse_date("2026-02-29").is_none());
        assert!(parse_date("2024-02-29").is_some());
        assert!(parse_date("0000-01-01").is_none());
        assert!(parse_date("2026-1-01").is_none());
        assert_eq!(normalize_datetime("2026-10-04T16:00:00+08:00").unwrap(), "2026-10-04T08:00:00.000Z");
        assert_eq!(normalize_datetime("2026-10-04T08:00:00.1234Z").unwrap(), "2026-10-04T08:00:00.123Z");
        assert!(normalize_datetime("2026-10-04T08:00:00").is_none());
        assert!(normalize_datetime("2026-10-04 08:00:00Z").is_none());
        assert_eq!(format_utc_ms(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(civil_from_days(days_from_civil(2026, 10, 20)), (2026, 10, 20));
    }

    #[test]
    fn typed() {
        let n = def(json!({"field_id":"n","name":"n","type":"number"}));
        assert_eq!(n.normalize(&json!(-0.0)).unwrap(), json!(0));
        assert_eq!(n.normalize(&json!(3.0)).unwrap(), json!(3));
        assert!(n.normalize(&json!(9007199254740992u64)).is_err());
        assert!(n.normalize(&json!("3")).is_err());
        assert!(n.normalize(&Value::Null).is_err());
        let s = def(json!({"field_id":"s","name":"s","type":"multi_select","nullable":true,
            "options":[{"option_id":"b","label":"B"},{"option_id":"a","label":"A"}]}));
        assert_eq!(s.normalize(&json!(["b", "a", "b"])).unwrap(), json!(["a", "b"]));
        assert!(s.normalize(&json!(["c"])).is_err());
        assert_eq!(s.normalize(&Value::Null).unwrap(), Value::Null);
        let t = def(json!({"field_id":"t","name":"t","type":"text"}));
        assert_eq!(t.normalize(&json!("")).unwrap(), json!(""));
        assert_eq!(t.normalize(&json!(" e\u{301} ")).unwrap(), json!(" e\u{301} "));
        assert!(FieldDef::from_value(&json!({"field_id":"d","name":"d","type":"decimal"})).is_err());
        assert!(FieldDef::from_value(&json!({"field_id":"d","name":"d","type":"select"})).is_err());
        assert!(FieldDef::from_value(&json!({"field_id":"d","name":"d","type":"boolean","unique":true})).is_err());
        let r = def(json!({"field_id":"r","name":"r","type":"object_ref"}));
        assert_eq!(
            r.normalize(&json!({"entity_id":"tasks"})).unwrap(),
            json!({"entity_id":"tasks","version":{"mode":"live_head"}})
        );
    }

    #[test]
    fn migrate() {
        let text = def(json!({"field_id":"x","name":"x","type":"text"}));
        let date = def(json!({"field_id":"x","name":"x","type":"date"}));
        let dec = def(json!({"field_id":"x","name":"x","type":"decimal","scale":2}));
        let numd = def(json!({"field_id":"x","name":"x","type":"number"}));
        assert_eq!(migrate_value(&text, &date, &json!("2026-10-20")).unwrap(), json!("2026-10-20"));
        assert!(migrate_value(&text, &date, &json!("10/20/2026")).is_err());
        assert_eq!(migrate_value(&text, &dec, &json!(" 12.5 ")).unwrap(), json!("12.50"));
        assert_eq!(migrate_value(&numd, &dec, &json!(12.5)).unwrap(), json!("12.50"));
        assert!(migrate_value(&numd, &dec, &json!(0.001)).is_err());
        assert_eq!(migrate_value(&text, &numd, &json!("42")).unwrap(), json!(42));
        assert!(migrate_value(&text, &numd, &json!("4e2")).is_err());
    }

    #[test]
    fn currency() {
        let c = def(json!({"field_id":"c","name":"c","type":"currency","currency":"USD"}));
        assert_eq!(c.currency_scale(), 2);
        // a bare amount takes the default currency; an object keeps its own
        assert_eq!(c.normalize(&json!("200")).unwrap(), json!({"amount":"200.00","currency":"USD"}));
        assert_eq!(c.normalize(&json!(12.5)).unwrap(), json!({"amount":"12.50","currency":"USD"}));
        assert_eq!(c.normalize(&json!(-3)).unwrap(), json!({"amount":"-3.00","currency":"USD"}));
        assert_eq!(c.normalize(&json!({"amount":"500","currency":"CNY"})).unwrap(), json!({"amount":"500.00","currency":"CNY"}));
        assert_eq!(c.normalize(&json!({"amount":10,"currency":"EUR"})).unwrap(), json!({"amount":"10.00","currency":"EUR"}));
        // symbols, names and loose text are the importer's job
        for bad in [json!("$12"), json!("USD 12"), json!("12.345"), json!({"amount":"1"}), json!({"currency":"USD"}),
            json!({"amount":"1","currency":"usd"}), json!({"amount":"1","currency":"US"}), json!({"amount":"1","currency":"USD","x":1}),
            json!(true), json!(["1"])] {
            assert!(c.normalize(&bad).is_err(), "{bad} must be refused");
        }
        assert!(c.normalize(&Value::Null).is_err());
        let jpy = def(json!({"field_id":"j","name":"j","type":"currency","currency":"JPY","scale":0}));
        assert_eq!(jpy.normalize(&json!("300")).unwrap(), json!({"amount":"300","currency":"JPY"}));
        assert!(jpy.normalize(&json!("300.5")).is_err());
        // definitions
        assert!(FieldDef::from_value(&json!({"field_id":"c","name":"c","type":"currency"})).is_err());
        assert!(FieldDef::from_value(&json!({"field_id":"c","name":"c","type":"currency","currency":"usd"})).is_err());
        assert!(FieldDef::from_value(&json!({"field_id":"c","name":"c","type":"currency","currency":"USD","scale":19})).is_err());
        assert!(FieldDef::from_value(&json!({"field_id":"c","name":"c","type":"currency","currency":"USD","unique":true})).is_err());
        assert!(FieldDef::from_value(&json!({"field_id":"d","name":"d","type":"decimal","scale":2,"currency":"USD"})).is_err());
        assert!(FieldDef::from_value(&json!({"field_id":"t","name":"t","type":"text","scale":2})).is_err());
        assert!(FieldType::Currency.sortable());
        // order: by code, then amount; equality needs the same code
        let v = |a: &str, k: &str| json!({"amount":a,"currency":k});
        assert_eq!(compare(&c, &v("500.00", "CNY"), &v("10.00", "USD")), Ordering::Less);
        assert_eq!(compare(&c, &v("9.50", "USD"), &v("10.00", "USD")), Ordering::Less);
        assert_eq!(compare(&c, &v("-1.00", "USD"), &v("0.00", "USD")), Ordering::Less);
        assert!(values_equal(&c, &v("10.00", "USD"), &v("10.0", "USD")));
        assert!(!values_equal(&c, &v("10.00", "USD"), &v("10.00", "EUR")));
    }

    #[test]
    fn currency_migrate() {
        let usd = def(json!({"field_id":"x","name":"x","type":"currency","currency":"USD"}));
        let usd0 = def(json!({"field_id":"x","name":"x","type":"currency","currency":"USD","scale":0}));
        let dec = def(json!({"field_id":"x","name":"x","type":"decimal","scale":2}));
        let num = def(json!({"field_id":"x","name":"x","type":"number"}));
        let text = def(json!({"field_id":"x","name":"x","type":"text"}));
        assert_eq!(migrate_value(&dec, &usd, &json!("12.50")).unwrap(), json!({"amount":"12.50","currency":"USD"}));
        assert_eq!(migrate_value(&num, &usd, &json!(7)).unwrap(), json!({"amount":"7.00","currency":"USD"}));
        assert!(migrate_value(&num, &usd, &json!(0.001)).is_err());
        assert_eq!(migrate_value(&usd, &dec, &json!({"amount":"5.00","currency":"EUR"})).unwrap(), json!("5.00"));
        assert_eq!(migrate_value(&usd, &text, &json!({"amount":"5.00","currency":"EUR"})).unwrap(), json!("EUR 5.00"));
        assert_eq!(migrate_value(&usd, &usd0, &json!({"amount":"5.00","currency":"JPY"})).unwrap(), json!({"amount":"5","currency":"JPY"}));
        assert!(migrate_value(&usd, &usd0, &json!({"amount":"5.50","currency":"USD"})).is_err());
        assert!(migration_supported(FieldType::Decimal, FieldType::Currency));
        assert!(!migration_supported(FieldType::Text, FieldType::Currency));
        assert!(!migration_supported(FieldType::Currency, FieldType::Number));
    }
}
