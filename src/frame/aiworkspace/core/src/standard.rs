//! Standard business tables (控件机制 TODO G8–G10, [schemas/standard-tables.md](../../schemas/standard-tables.md)): the
//! to-do and calendar tables of the official controls are ordinary TableSources marked with a `schema_id`. Every rule
//! here is schema class (README “Validator classes”): admission only, never replay or import.

use crate::error::{WsError, WsResult};
use crate::model::{EntityRow, JsonMap, ReadCtx, TYPE_TABLE};
use crate::order_key::check_order_key;
use crate::plan::Planner;
use crate::value::{parse_date, parse_datetime_ms, FieldDef, FieldType};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const TODO_SCHEMA: &str = "buckyos.todo@1";
pub const CALENDAR_SCHEMA: &str = "buckyos.calendar@1";

type Fields = &'static [(&'static str, FieldType)];

const TODO_FIELDS: Fields = &[
    ("title", FieldType::Text),
    ("status", FieldType::Select),
    ("due", FieldType::Date),
    ("order", FieldType::Text),
    ("done_at", FieldType::Datetime),
    ("ext", FieldType::Json),
];
const CALENDAR_FIELDS: Fields = &[
    ("title", FieldType::Text),
    ("all_day", FieldType::Boolean),
    ("start", FieldType::Datetime),
    ("end", FieldType::Datetime),
    ("start_date", FieldType::Date),
    ("end_date", FieldType::Date),
    ("status", FieldType::Select),
    ("ext", FieldType::Json),
];

/// A business schema the kernel knows the standard table of (a host can create one).
pub fn is_standard_schema(id: &str) -> bool {
    matches!(id, TODO_SCHEMA | CALENDAR_SCHEMA)
}

/// The field ids of a built-in schema.
pub fn schema_field_ids(id: &str) -> Vec<String> {
    match id {
        TODO_SCHEMA => TODO_FIELDS,
        CALENDAR_SCHEMA => CALENDAR_FIELDS,
        _ => &[],
    }
    .iter()
    .map(|(f, _)| f.to_string())
    .collect()
}

/// The standard fields of a table, `None` when it is not a standard table.
pub fn standard_fields(e: &EntityRow) -> Option<Fields> {
    if e.type_id != TYPE_TABLE {
        return None;
    }
    schema_fields(e.schema_id.as_deref())
}

fn schema_fields(schema: Option<&str>) -> Option<Fields> {
    match schema {
        Some(TODO_SCHEMA) => Some(TODO_FIELDS),
        Some(CALENDAR_SCHEMA) => Some(CALENDAR_FIELDS),
        _ => None,
    }
}

fn bad(m: impl Into<String>) -> WsError {
    WsError::invalid_schema(m)
}

/// `<ns>.<name>@<major>`: dot-separated lower-case names (letters, digits, `_`, `-`; each starts with a letter), at
/// least a namespace and a name, then a major version ≥ 1.
pub fn is_schema_id(id: &str) -> bool {
    let Some((name, major)) = id.split_once('@') else { return false };
    let part = |p: &str| p.as_bytes().first().is_some_and(u8::is_ascii_lowercase) && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    id.len() <= 128 && name.split('.').count() >= 2 && name.split('.').all(part) && !major.starts_with('0') && !major.is_empty() && major.bytes().all(|b| b.is_ascii_digit())
}

/// The table that defines a user schema (B0-1): the earliest created live table carrying `schema_id` and a
/// `source.def`. Its fields (ids and types) and `def.primary_key` are the schema.
pub fn user_schema_definer(ctx: &dyn ReadCtx, schema_id: &str) -> WsResult<Option<EntityRow>> {
    let mut best: Option<EntityRow> = None;
    let mut stack = vec![crate::model::ROOT_ID.to_string()];
    while let Some(id) = stack.pop() {
        for edge in ctx.children(&id)? {
            let Some(c) = ctx.entity(&edge.child_id)?.filter(|c| c.alive()) else { continue };
            if c.type_id == TYPE_TABLE && c.schema_id.as_deref() == Some(schema_id) && crate::types::source_def(&c).is_some()
                && best.as_ref().is_none_or(|b| (c.created_seq, &c.entity_id) < (b.created_seq, &b.entity_id))
            {
                best = Some(c.clone());
            }
            stack.push(c.entity_id);
        }
    }
    Ok(best)
}

/// The fields of a user schema: `(field_id, type)` of its definer's live fields.
pub fn user_schema_fields(ctx: &dyn ReadCtx, schema_id: &str) -> WsResult<Option<(EntityRow, Vec<(String, FieldType)>)>> {
    let Some(d) = user_schema_definer(ctx, schema_id)? else { return Ok(None) };
    let fields = ctx.fields(&d.entity_id)?.into_iter().filter(|f| f.alive()).map(|f| (f.field_id, f.def.ty)).collect();
    Ok(Some((d, fields)))
}

/// Validator class: schema. A table of a registered user schema either defines it (it carries `source.def`, and no live
/// table did before) or has every field of the schema in its type. A `buckyos.` id is one of the built-in schemas.
fn check_user_table(p: &Planner, e: &EntityRow) -> WsResult<()> {
    let schema = e.schema_id.as_deref().unwrap_or_default();
    if !is_schema_id(schema) {
        return Err(bad(format!("schema_id {schema:?} must be <namespace>.<name>@<major>")));
    }
    if schema.starts_with("buckyos.") {
        return Err(bad(format!("{schema} is not a built-in schema; the buckyos. namespace has {TODO_SCHEMA} and {CALENDAR_SCHEMA} only")));
    }
    // the table being created is not in the tree yet: it defines the schema when nothing did before; a schema nobody
    // registered has no rules (a wish result must name a registered one)
    let Some((definer, fields)) = user_schema_fields(&p.ov, schema)? else { return Ok(()) };
    if definer.entity_id == e.entity_id {
        return Ok(());
    }
    let have = p.ov.fields(&e.entity_id)?;
    for (id, ty) in &fields {
        match have.iter().find(|f| f.field_id == *id && f.alive()) {
            Some(f) if f.def.ty == *ty => {}
            Some(f) => return Err(bad(format!("{schema}: field {id} must be {}, not {}", ty.as_str(), f.def.ty.as_str()))),
            None => return Err(bad(format!("{schema}: the table needs the field {id} ({})", ty.as_str()))),
        }
    }
    Ok(())
}

/// Validator class: schema. A standard table is created with every standard field in its type (the options of a
/// select may differ), and its title field is `title`. A table of a user schema defines it or follows it.
pub fn check_table(p: &Planner, before: Option<&EntityRow>, e: &EntityRow, changed: Option<&[String]>) -> WsResult<()> {
    if changed.is_none_or(|c| c.iter().any(|k| k == "source")) {
    }
    if e.schema_id.is_some() && standard_fields(e).is_none() && before.is_none() {
        return check_user_table(p, e);
    }
    let Some(standard) = standard_fields(e) else { return Ok(()) };
    let schema = e.schema_id.as_deref().unwrap_or_default();
    if before.is_none() {
        let fields = p.ov.fields(&e.entity_id)?;
        for (id, ty) in standard {
            match fields.iter().find(|f| f.field_id == *id && f.alive()) {
                Some(f) if f.def.ty == *ty => {}
                Some(f) => return Err(bad(format!("{schema}: field {id} must be {}, not {}", ty.as_str(), f.def.ty.as_str()))),
                None => return Err(bad(format!("{schema}: the table needs the field {id} ({})", ty.as_str()))),
            }
        }
    }
    if changed.is_none_or(|c| c.iter().any(|k| k == "title_field_id")) && e.payload.get("title_field_id").and_then(Value::as_str) != Some("title") {
        return Err(bad(format!("{schema}: title_field_id must be title")));
    }
    Ok(())
}

/// Validator class: schema. A standard field is never deleted (`to: None`) and keeps its type (`to`: the type a
/// migration would give it); so does a field of a user schema in a table that follows it (its definer changes the
/// schema, and the wishes reading it go back to coding).
pub fn check_field_change(p: &Planner, src: &EntityRow, field_id: &str, to: Option<FieldType>) -> WsResult<()> {
    if let (Some(schema), None) = (src.schema_id.as_deref(), standard_fields(src)) {
        if let Some((definer, fields)) = user_schema_fields(&p.ov, schema)?.filter(|(d, _)| d.entity_id != src.entity_id) {
            return match (fields.iter().find(|(id, _)| id == field_id), to) {
                (Some(_), None) => Err(bad(format!("{field_id} is a field of {schema} (defined by {}) and cannot be deleted", definer.entity_id))),
                (Some((_, ty)), Some(t)) if t != *ty => Err(bad(format!("{field_id} is a field of {schema} and stays {}", ty.as_str()))),
                _ => Ok(()),
            };
        }
        return Ok(());
    }
    let Some(standard) = standard_fields(src) else { return Ok(()) };
    let schema = src.schema_id.as_deref().unwrap_or_default();
    match (standard.iter().find(|(id, _)| *id == field_id), to) {
        (Some(_), None) => Err(bad(format!("{field_id} is a standard field of {schema} and cannot be deleted"))),
        (Some((_, ty)), Some(t)) if t != *ty => Err(bad(format!("{field_id} is a standard field of {schema} and stays {}", ty.as_str()))),
        _ => Ok(()),
    }
}

/// Note a record of a standard table that an operation inserted, changed or restored; [`check_records`] judges it once
/// the whole Commit is planned.
pub fn touch_record(p: &mut Planner, src: &EntityRow, record_id: &str) {
    if p.env.schema_checks && standard_fields(src).is_some() {
        p.standard_records.insert((src.entity_id.clone(), record_id.to_string()), p.op_index);
    }
}

/// Validator class: schema. The record rules, on the final values of every record the Commit touched — one Commit may
/// move `start` and `end` together or turn a timed event into an all-day one over several operations, and its undo
/// replays them in another order. A failure names the last operation that touched the record.
pub fn check_records(p: &Planner) -> Result<(), (WsError, usize)> {
    for ((sid, rid), &op_index) in &p.standard_records {
        let at = |e: WsError| (e, op_index);
        let Some(src) = p.ov.entity(sid).map_err(at)?.filter(|e| e.alive()) else { continue };
        let Some(standard) = standard_fields(&src) else { continue };
        let Some(rec) = p.ov.record(sid, rid).map_err(at)?.filter(|r| r.alive()) else { continue };
        let fields = p.ov.fields(sid).map_err(at)?.into_iter().filter(|f| f.alive()).map(|f| (f.field_id, f.def.ty)).collect();
        let r = Rec { values: &rec.values, fields, standard };
        let verdict = if src.schema_id.as_deref() == Some(TODO_SCHEMA) { todo_rules(&r) } else { calendar_rules(&r) };
        verdict.map_err(|(field, m)| at(bad(format!("record {rid}: {m}")).with_data(json!({ "entity_id": sid, "record_id": rid, "field_id": field }))))?;
    }
    Ok(())
}

/// A failed rule: the field it is about and the message.
type Rule = Result<(), (&'static str, String)>;

struct Rec<'a> {
    values: &'a JsonMap,
    fields: BTreeMap<String, FieldType>,
    standard: Fields,
}

impl Rec<'_> {
    /// The table has standard field `id` in its standard type (a rule on a field the table lacks is skipped).
    fn has(&self, id: &str) -> bool {
        self.standard.iter().any(|(s, t)| *s == id && self.fields.get(id) == Some(t))
    }
    /// The present, non-null value of standard field `id`.
    fn get(&self, id: &str) -> Option<&Value> {
        self.values.get(id).filter(|v| !v.is_null() && self.has(id))
    }
    fn required(&self, id: &'static str, what: &str) -> Rule {
        if self.has(id) && self.get(id).is_none() {
            return Err((id, format!("{id} is required{what}")));
        }
        Ok(())
    }
}

/// Missing inputs for an insert after schema defaults and service-populated fields are accounted for.
pub fn missing_record_fields(fields: &[FieldDef], schema: Option<&str>, draft: &JsonMap) -> Vec<String> {
    let mut effective = draft.clone();
    for field in fields {
        if let Some(value) = &field.default {
            effective.entry(field.field_id.clone()).or_insert_with(|| value.clone());
        }
    }
    let record = Rec {
        values: &effective,
        fields: fields.iter().map(|f| (f.field_id.clone(), f.ty)).collect(),
        standard: schema_fields(schema).unwrap_or(&[]),
    };
    let business = required_inputs(&record, schema);
    fields.iter().filter(|f| {
        !f.submitter && ((f.required && !f.nullable && effective.get(&f.field_id).is_none_or(Value::is_null))
            || business.iter().any(|(id, _)| *id == f.field_id && record.has(id) && record.get(id).is_none()))
    }).map(|f| f.field_id.clone()).collect()
}

fn required_inputs(record: &Rec, schema: Option<&str>) -> &'static [(&'static str, &'static str)] {
    match schema {
        Some(TODO_SCHEMA) => &[("title", ""), ("status", "")],
        Some(CALENDAR_SCHEMA) if record.get("all_day").and_then(Value::as_bool) == Some(true) =>
            &[("title", ""), ("start_date", " on an all-day event")],
        Some(CALENDAR_SCHEMA) => &[("title", ""), ("start", " on a timed event")],
        _ => &[],
    }
}

fn todo_rules(r: &Rec) -> Rule {
    for &(id, why) in required_inputs(r, Some(TODO_SCHEMA)) { r.required(id, why)?; }
    if let Some(k) = r.get("order") {
        let k = k.as_str().ok_or(("order", "order must be an order key".to_string()))?;
        check_order_key(k).map_err(|e| ("order", format!("order: {}", e.detail)))?;
    }
    let Some(ext) = r.get("ext").and_then(Value::as_object) else { return Ok(()) };
    for (k, v) in ext {
        match k.as_str() {
            "priority" => one_of(v, &["low", "normal", "high", "urgent"]),
            "tags" => list(v, 32, |x| text(x, 64)),
            "assignee" => text(v, 256),
            "reminders" => reminders(v),
            "source" => small_object(v, 4096),
            _ => Ok(()),
        }
        .map_err(|m| ("ext", format!("ext.{k}: {m}")))?;
    }
    Ok(())
}

fn calendar_rules(r: &Rec) -> Rule {
    let all_day = r.get("all_day").and_then(Value::as_bool).unwrap_or(false);
    let (kind, first, last, other) = match all_day {
        true => ("an all-day event", "start_date", "end_date", ["start", "end"]),
        false => ("a timed event", "start", "end", ["start_date", "end_date"]),
    };
    for k in other {
        if r.get(k).is_some() {
            return Err((k, format!("{k} must be absent on {kind} (it uses {first} / {last})")));
        }
    }
    for &(id, why) in required_inputs(r, Some(CALENDAR_SCHEMA)) { r.required(id, why)?; }
    // canonical datetimes and dates order as text
    if let (Some(a), Some(b)) = (r.get(first).and_then(Value::as_str), r.get(last).and_then(Value::as_str)) {
        if b < a {
            return Err((last, format!("{last} is before {first}")));
        }
    }
    let Some(ext) = r.get("ext").and_then(Value::as_object) else { return Ok(()) };
    if ext.contains_key("rrule") && ext.contains_key("recurrence_id") {
        return Err(("ext", "ext.recurrence_id overrides one occurrence of a series and never goes with ext.rrule".into()));
    }
    let start = |v: &Value| if all_day { date(v) } else { datetime(v) };
    for (k, v) in ext {
        match k.as_str() {
            "rrule" => v.as_str().ok_or_else(|| "must be a string".to_string()).and_then(check_rrule),
            "exdate" => list(v, 1000, start),
            "recurrence_id" => recurrence_id(v),
            "tz" => match v.as_str() {
                Some(s) if (1..=64).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'+' | b'/' | b'-')) => Ok(()),
                _ => Err("must be an IANA time zone name ([A-Za-z0-9_+/-], ≤ 64 characters)".into()),
            },
            "location" => text(v, 1024),
            "color" => match v.as_str().and_then(|s| s.strip_prefix('#')) {
                Some(h) if h.len() == 6 && h.bytes().all(|c| c.is_ascii_hexdigit()) => Ok(()),
                _ => Err("must be #rrggbb".into()),
            },
            "busy" => v.as_bool().map(|_| ()).ok_or_else(|| "must be a boolean".to_string()),
            "reminders" => reminders(v),
            "rule_text" => text(v, 256),
            "occurrences" => list(v, 1000, date_or_datetime),
            "covered_until" => date_or_datetime(v),
            "maintenance" => small_object(v, 4096),
            _ => Ok(()),
        }
        .map_err(|m| ("ext", format!("ext.{k}: {m}")))?;
    }
    Ok(())
}

fn one_of(v: &Value, allowed: &[&str]) -> Result<(), String> {
    match v.as_str() {
        Some(s) if allowed.contains(&s) => Ok(()),
        _ => Err(format!("must be one of {}", allowed.join(", "))),
    }
}

fn text(v: &Value, max: usize) -> Result<(), String> {
    match v.as_str() {
        Some(s) if s.chars().count() <= max => Ok(()),
        _ => Err(format!("must be a string of at most {max} characters")),
    }
}

fn list(v: &Value, max: usize, item: impl Fn(&Value) -> Result<(), String>) -> Result<(), String> {
    let a = v.as_array().filter(|a| a.len() <= max).ok_or_else(|| format!("must be a list of at most {max} items"))?;
    a.iter().enumerate().try_for_each(|(i, x)| item(x).map_err(|m| format!("[{i}] {m}")))
}

fn small_object(v: &Value, max: usize) -> Result<(), String> {
    let len = crate::canonical::canonical_json(v).map(|s| s.len()).unwrap_or(usize::MAX);
    if !v.is_object() || len > max {
        return Err(format!("must be an object of at most {max} bytes"));
    }
    Ok(())
}

fn date(v: &Value) -> Result<(), String> {
    v.as_str().and_then(parse_date).map(|_| ()).ok_or_else(|| "must be a date (YYYY-MM-DD), as the event's start_date".into())
}

fn datetime(v: &Value) -> Result<(), String> {
    v.as_str().and_then(parse_datetime_ms).map(|_| ()).ok_or_else(|| "must be an RFC 3339 datetime with offset, as the event's start".into())
}

fn date_or_datetime(v: &Value) -> Result<(), String> {
    match v.as_str() {
        Some(s) if parse_date(s).is_some() || parse_datetime_ms(s).is_some() => Ok(()),
        _ => Err("must be a date (YYYY-MM-DD) or an RFC 3339 datetime".into()),
    }
}

/// `[{ before_min: 0…40320, channel?: string ≤ 64 }]`, at most 16.
fn reminders(v: &Value) -> Result<(), String> {
    list(v, 16, |r| {
        if !r.get("before_min").and_then(Value::as_u64).is_some_and(|m| m <= 40_320) {
            return Err("before_min must be a whole number of minutes in 0…40320".into());
        }
        match r.get("channel") {
            None => Ok(()),
            Some(c) => text(c, 64).map_err(|m| format!("channel {m}")),
        }
    })
}

/// `{ master: <record_id>, original: <datetime | date> }`.
fn recurrence_id(v: &Value) -> Result<(), String> {
    if !v.get("master").and_then(Value::as_str).is_some_and(crate::id::is_valid_id) {
        return Err("master must be the record id of the series".into());
    }
    v.get("original").ok_or_else(|| "original is required".to_string()).and_then(date_or_datetime).map_err(|m| format!("original: {m}"))
}

const DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];

/// An integer as RRULE writes it (`-?digits`, no `+`) within `lo..=hi`.
fn int_in(s: &str, lo: i64, hi: i64) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || digits.len() > 4 || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    s.parse().ok().filter(|n| (lo..=hi).contains(n))
}

/// `-5…-1, 1…5`.
fn ordinal(s: &str) -> bool {
    int_in(s, -5, 5).is_some_and(|n| n != 0)
}

/// A comma-separated list of at most `max` non-empty items.
fn items<'a>(name: &str, value: &'a str, max: usize) -> Result<Vec<&'a str>, String> {
    let a: Vec<&str> = value.split(',').collect();
    if a.len() > max || a.iter().any(|x| x.is_empty()) {
        return Err(format!("{name}={value}: at most {max} comma-separated values"));
    }
    Ok(a)
}

/// `YYYYMMDD` or `YYYYMMDDTHHMMSSZ`.
fn until(s: &str) -> bool {
    let digits = |x: &str| x.bytes().all(|c| c.is_ascii_digit());
    let (d, t) = match s.len() {
        8 => (s, None),
        16 if s.as_bytes()[8] == b'T' && s.ends_with('Z') => (&s[..8], Some(&s[9..15])),
        _ => return false,
    };
    if !digits(d) || parse_date(&format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..])).is_none() {
        return false;
    }
    t.is_none_or(|t| digits(t) && &t[..2] < "24" && &t[2..4] < "60" && &t[4..] < "60")
}

/// The RRULE subset of the standard calendar (validation only; the clients expand it). `NAME=VALUE` parts separated by
/// `;`, upper case, each name at most once; anything outside the subset is refused with a message naming the part —
/// such a rule belongs in `rule_text` + `occurrences`, never silently simplified.
pub fn check_rrule(rule: &str) -> Result<(), String> {
    let mut parts: BTreeMap<&str, &str> = BTreeMap::new();
    for part in rule.split(';') {
        if !part.is_ascii() {
            return Err(format!("{part:?} is not an RRULE part"));
        }
        if part.bytes().any(|c| c.is_ascii_lowercase()) {
            return Err(format!("{part}: RRULE parts are upper case"));
        }
        let (name, value) = part.split_once('=').filter(|(n, v)| !n.is_empty() && !v.is_empty()).ok_or_else(|| format!("{part:?} is not NAME=VALUE"))?;
        if !matches!(name, "FREQ" | "INTERVAL" | "COUNT" | "UNTIL" | "BYDAY" | "BYMONTHDAY" | "BYMONTH" | "BYSETPOS" | "WKST") {
            return Err(format!("{part}: {name} is outside the supported subset (write rule_text and occurrences instead)"));
        }
        if parts.insert(name, value).is_some() {
            return Err(format!("{part}: {name} appears more than once"));
        }
    }
    let freq = *parts.get("FREQ").ok_or("FREQ is required")?;
    if !matches!(freq, "DAILY" | "WEEKLY" | "MONTHLY" | "YEARLY") {
        return Err(format!("FREQ={freq}: only DAILY, WEEKLY, MONTHLY and YEARLY are supported (write rule_text and occurrences instead)"));
    }
    let by_month = matches!(freq, "MONTHLY" | "YEARLY");
    if parts.contains_key("COUNT") && parts.contains_key("UNTIL") {
        return Err("COUNT and UNTIL never go together".into());
    }
    for (&name, &value) in &parts {
        let part = format!("{name}={value}");
        match name {
            "INTERVAL" if int_in(value, 1, 999).is_none() => return Err(format!("{part}: INTERVAL is 1…999")),
            "COUNT" if int_in(value, 1, 1000).is_none() => return Err(format!("{part}: COUNT is 1…1000")),
            "UNTIL" if !until(value) => return Err(format!("{part}: UNTIL is YYYYMMDD or YYYYMMDDTHHMMSSZ")),
            "WKST" if !DAYS.contains(&value) => return Err(format!("{part}: WKST is one of MO…SU")),
            "BYDAY" => {
                for d in items(name, value, 7)? {
                    let (ord, day) = d.split_at(d.len().saturating_sub(2));
                    if !DAYS.contains(&day) || !(ord.is_empty() || ordinal(ord)) {
                        return Err(format!("{part}: {d} is not MO…SU with an optional ordinal -5…-1, 1…5"));
                    }
                    if !ord.is_empty() && !by_month {
                        return Err(format!("{part}: an ordinal ({d}) needs FREQ=MONTHLY or YEARLY"));
                    }
                }
            }
            "BYMONTHDAY" => {
                if !by_month {
                    return Err(format!("{part}: BYMONTHDAY needs FREQ=MONTHLY or YEARLY"));
                }
                if items(name, value, 31)?.iter().any(|d| int_in(d, 1, 31).is_none() && *d != "-1") {
                    return Err(format!("{part}: days are 1…31 or -1"));
                }
            }
            "BYMONTH" => {
                if freq != "YEARLY" {
                    return Err(format!("{part}: BYMONTH needs FREQ=YEARLY"));
                }
                if items(name, value, 12)?.iter().any(|m| int_in(m, 1, 12).is_none()) {
                    return Err(format!("{part}: months are 1…12"));
                }
            }
            "BYSETPOS" => {
                if freq != "MONTHLY" || !parts.contains_key("BYDAY") {
                    return Err(format!("{part}: BYSETPOS needs FREQ=MONTHLY and BYDAY"));
                }
                if !ordinal(value) {
                    return Err(format!("{part}: BYSETPOS is one of -5…-1, 1…5"));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{check_rrule, missing_record_fields, CALENDAR_SCHEMA, TODO_SCHEMA};
    use crate::value::FieldDef;
    use serde_json::{json, Value};

    #[test]
    fn draft_asks_for_missing_values_after_defaults_and_business_rules() {
        let defs = |items: Value| items.as_array().unwrap().iter().map(|f| FieldDef::from_value(f).unwrap()).collect::<Vec<_>>();
        let fields = defs(json!([
            {"field_id":"a","name":"A","type":"text","required":true},
            {"field_id":"b","name":"B","type":"number","required":true,"default":0},
            {"field_id":"c","name":"C","type":"boolean","required":true,"default":false},
            {"field_id":"d","name":"D","type":"text","nullable":true,"required":true},
            {"field_id":"e","name":"E","type":"text","unique":true},
            {"field_id":"who","name":"Who","type":"text","submitter":true,"required":true}
        ]));
        assert_eq!(missing_record_fields(&fields, None, json!({}).as_object().unwrap()), ["a"]);
        assert!(missing_record_fields(&fields, None, json!({"a":"ok"}).as_object().unwrap()).is_empty());

        let todo = defs(json!([
            {"field_id":"title","name":"Title","type":"text"},
            {"field_id":"status","name":"Status","type":"select","options":[{"option_id":"todo","label":"Todo"}],"default":"todo"}
        ]));
        assert_eq!(missing_record_fields(&todo, Some(TODO_SCHEMA), json!({}).as_object().unwrap()), ["title"]);

        let calendar = defs(json!([
            {"field_id":"title","name":"Title","type":"text"},
            {"field_id":"all_day","name":"All day","type":"boolean"},
            {"field_id":"start","name":"Start","type":"datetime","nullable":true},
            {"field_id":"start_date","name":"First day","type":"date","nullable":true}
        ]));
        assert_eq!(missing_record_fields(&calendar, Some(CALENDAR_SCHEMA), json!({}).as_object().unwrap()), ["title", "start"]);
        assert_eq!(missing_record_fields(&calendar, Some(CALENDAR_SCHEMA), json!({"title":"Trip", "all_day":true}).as_object().unwrap()), ["start_date"]);
    }


    #[test]
    fn rrule_subset() {
        for ok in [
            "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR",
            "FREQ=MONTHLY;BYDAY=-1FR",
            "FREQ=MONTHLY;BYMONTHDAY=15;COUNT=10",
            "FREQ=YEARLY;BYMONTH=3;BYMONTHDAY=8",
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1",
            "FREQ=DAILY;INTERVAL=2;UNTIL=20261231T000000Z",
            "FREQ=DAILY",
            "FREQ=WEEKLY;INTERVAL=2;BYDAY=SA;WKST=SU;UNTIL=20270101",
            "FREQ=YEARLY;BYMONTH=11;BYDAY=4TH",
            "FREQ=MONTHLY;BYMONTHDAY=-1",
        ] {
            assert_eq!(check_rrule(ok), Ok(()), "{ok}");
        }
        for (bad, names) in [
            ("FREQ=HOURLY", "FREQ=HOURLY"),
            ("FREQ=DAILY;BYHOUR=9", "BYHOUR"),
            ("FREQ=DAILY;COUNT=3;UNTIL=20261231", "COUNT and UNTIL"),
            ("FREQ=WEEKLY;BYDAY=1MO", "1MO"),
            ("freq=daily", "freq=daily"),
            ("FREQ=DAILY;freq=WEEKLY", "freq=WEEKLY"),
            ("FREQ=DAILY;FREQ=WEEKLY", "FREQ appears more than once"),
            ("FREQ=SECONDLY", "FREQ=SECONDLY"),
            ("FREQ=MINUTELY", "FREQ=MINUTELY"),
            ("FREQ=WEEKLY;BYWEEKNO=3", "BYWEEKNO"),
            ("FREQ=DAILY;RDATE=20261231", "RDATE"),
            ("INTERVAL=2", "FREQ is required"),
            ("FREQ=DAILY;INTERVAL=0", "INTERVAL=0"),
            ("FREQ=DAILY;INTERVAL=1000", "INTERVAL=1000"),
            ("FREQ=DAILY;COUNT=1001", "COUNT=1001"),
            ("FREQ=DAILY;UNTIL=20260231", "UNTIL=20260231"),
            ("FREQ=DAILY;UNTIL=20261231T240000Z", "UNTIL=20261231T240000Z"),
            ("FREQ=DAILY;UNTIL=2026-12-31", "UNTIL=2026-12-31"),
            ("FREQ=MONTHLY;BYDAY=6MO", "6MO"),
            ("FREQ=MONTHLY;BYDAY=+1MO", "+1MO"),
            ("FREQ=MONTHLY;BYDAY=MO,,TU", "BYDAY"),
            ("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR,SA,SU,MO", "BYDAY"),
            ("FREQ=WEEKLY;BYMONTHDAY=3", "BYMONTHDAY"),
            ("FREQ=MONTHLY;BYMONTHDAY=32", "BYMONTHDAY=32"),
            ("FREQ=MONTHLY;BYMONTHDAY=-2", "BYMONTHDAY=-2"),
            ("FREQ=MONTHLY;BYMONTH=3", "BYMONTH"),
            ("FREQ=YEARLY;BYMONTH=13", "BYMONTH=13"),
            ("FREQ=MONTHLY;BYSETPOS=-1", "BYSETPOS"),
            ("FREQ=YEARLY;BYDAY=MO;BYSETPOS=1", "BYSETPOS"),
            ("FREQ=MONTHLY;BYDAY=MO;BYSETPOS=0", "BYSETPOS=0"),
            ("FREQ=WEEKLY;WKST=XX", "WKST=XX"),
            ("FREQ=DAILY;", "\"\""),
            ("", "\"\""),
            ("RRULE:FREQ=DAILY", "RRULE:FREQ"),
        ] {
            let m = check_rrule(bad).expect_err(bad);
            assert!(m.contains(names), "{bad}: {m}");
        }
    }
}
