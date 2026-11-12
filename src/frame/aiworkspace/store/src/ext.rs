//! Extension controls (扩展控件 P0, README “Extension controls”): a control's source directory — `component.json`
//! plus its entry files — is checked and composed into a `buckyos.block-def`, and one build Commit writes the
//! definition, its source snapshot (`buckyos.dir`) and the `derived.build` record; nothing is written when any of it
//! fails. Exports read the snapshot back as a bundle (the AIWS manifest plus files), imports build library files.

use crate::workspace::{random_id, Caller, CommitOpts, Workspace};
use aiworkspace_core::access::Cap;
use aiworkspace_core::canonical::{canonical_json, chunk_id, file_object, sha256_hex};
use aiworkspace_core::materialize::ObjectSource;
use aiworkspace_core::model::*;
use aiworkspace_core::order_key::order_key_between;
use aiworkspace_core::types::{
    check_catalog, check_def_bindings, check_member_name, ident_string, is_event_name, is_renderer_id, is_slot_name, COMPOSITE, HTML_API_LATEST, HTML_API_VERSION, HTML_BIND_API,
    MANIFEST_VERSION, MAX_AGENT_BYTES, MAX_BLOCK_DEF_BYTES, MAX_DEF_DEPS, MAX_EVENTS, MAX_SLOTS,
};
use aiworkspace_core::{Code, WsError, WsResult};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Recorded in `derived.build.builder_version` and the input digest: a new composition rule is a new version.
pub const BUILDER_VERSION: &str = "aiws-ext-build/1";
/// The interface file of a control's source directory.
pub const COMPONENT_FILE: &str = "component.json";
/// The folder under `data` builds and imports default to.
pub const EXTENSIONS_FOLDER: &str = "extensions";
/// Files of one source directory (names may be relative paths, Q23).
pub const MAX_SOURCE_FILES: usize = 64;
/// The composition file of a composite control.
pub const COMPOSITION_FILE: &str = "composition.json";
/// Inline (`base64`) source bytes one build may carry.
pub const MAX_INLINE_BYTES: usize = 4 * 1024 * 1024;
/// The declarative interpreter's vocabulary version (`host_api` of a declarative control).
pub const DECLARATIVE_API_VERSION: u64 = 1;

/// One file of a source directory: its name, the FileObject holding it and its bytes.
#[derive(Debug, Clone)]
pub struct SourceFile {
    pub name: String,
    pub obj_id: String,
    pub bytes: Vec<u8>,
}

/// A file as a client hands it over: an object it may read, or inline bytes.
#[derive(Debug, Clone)]
pub enum IncomingFile {
    Object { name: String, obj_id: String },
    Inline { name: String, bytes: Vec<u8> },
}

impl IncomingFile {
    pub fn name(&self) -> &str {
        match self {
            IncomingFile::Object { name, .. } | IncomingFile::Inline { name, .. } => name,
        }
    }

    /// `[{ name, obj_id } | { name, base64 }]` of `ext.build`.
    pub fn parse_list(v: Option<&Value>) -> WsResult<Vec<IncomingFile>> {
        use base64::Engine;
        let list = v.and_then(Value::as_array).ok_or_else(|| WsError::invalid_op("files must be an array of { name, obj_id } or { name, base64 }"))?;
        let mut out = Vec::new();
        let mut inline = 0usize;
        for f in list {
            let name = f.get("name").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("files[].name required"))?.to_string();
            match (f.get("obj_id").and_then(Value::as_str), f.get("base64").and_then(Value::as_str)) {
                (Some(obj_id), None) => out.push(IncomingFile::Object { name, obj_id: obj_id.to_string() }),
                (None, Some(b64)) => {
                    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).map_err(|e| WsError::invalid_op(format!("files[{name}].base64: {e}")))?;
                    inline += bytes.len();
                    if inline > MAX_INLINE_BYTES {
                        return Err(WsError::limit(format!("inline source files are limited to {MAX_INLINE_BYTES} bytes per build")));
                    }
                    out.push(IncomingFile::Inline { name, bytes });
                }
                _ => return Err(WsError::invalid_op(format!("files[{name}]: give obj_id or base64"))),
            }
        }
        Ok(out)
    }
}

/// What one build composes from a source directory, before it meets the Workspace.
#[derive(Debug, Clone)]
pub struct Composed {
    pub ident: String,
    pub name: String,
    pub kind: String,
    /// The definition payload without `version`, `source_dir` and `provenance` (the build adds those).
    pub payload: Map<String, Value>,
    /// The source directory's members: `{ <name>: { kind: file, object_id } }`.
    pub members: Map<String, Value>,
    pub input_digest: String,
    /// `sha256:` over the files' contents (the AIWS manifest's control digest).
    pub digest: String,
    /// The identities `component.json` `deps` names (resolved against the Workspace by the build).
    pub deps: Vec<String>,
    /// The identities a composite's slots accept as `cell:<ident>`.
    pub slot_cells: Vec<String>,
    /// A composite's re-raised events: `(name, slot, child event)`, resolved against the slots' controls by the build.
    pub bubbles: Vec<(String, String, String)>,
    pub warnings: Vec<Value>,
}

/// One control of a build or an import.
#[derive(Debug, Clone)]
pub struct BuildInput {
    pub files: Vec<SourceFile>,
    /// `{ pkg_id, version, digest }` of an imported package.
    pub provenance: Option<Value>,
    /// Brought along because another item depends on it (an import): a definition of that identity already in the
    /// folder, whatever its source, satisfies it.
    pub implicit: bool,
}

#[derive(Debug, Clone, Default)]
pub struct BuildOpts {
    pub folder_id: Option<String>,
    /// `(def_rev, dir_rev)` the client last built or pulled.
    pub expect: Option<(u64, u64)>,
    pub force: bool,
    pub dry_run: bool,
    /// An import from the library: an existing identity is reused when it came from the same package content and
    /// refused otherwise; never rebuilt.
    pub import: bool,
    /// A source upgrade (P2, Q30): an identity already here with another provenance is rebuilt from the given files
    /// (no base `expect`), with the new provenance.
    pub upgrade: bool,
    /// `<ident>: Migration` applied to the instances of a definition whose rebuild narrows their contract.
    pub migration: Map<String, Value>,
    pub idempotency_key: Option<String>,
}

// ---- diagnostics

struct Diags(Vec<Value>);

impl Diags {
    fn error(&mut self, file: &str, message: impl Into<String>) {
        self.0.push(json!({ "file": file, "severity": "error", "message": message.into() }));
    }
    fn warn(&mut self, file: &str, message: impl Into<String>) {
        self.0.push(json!({ "file": file, "severity": "warning", "message": message.into() }));
    }
    fn missing(&mut self, file: &str, message: impl Into<String>) {
        self.0.push(json!({ "file": file, "severity": "error", "code": "DEP_MISSING", "message": message.into() }));
    }
    fn json_error(&mut self, file: &str, e: &serde_json::Error) {
        self.0.push(json!({ "file": file, "line": e.line(), "col": e.column(), "severity": "error", "message": e.to_string() }));
    }
    fn has_errors(&self) -> bool {
        self.0.iter().any(|d| d["severity"] == "error")
    }
}

/// `BUILD_FAILED` with its diagnostics.
pub fn build_failed(diagnostics: Vec<Value>) -> WsError {
    let first = diagnostics.iter().find(|d| d["severity"] == "error").map(|d| format!("{}: {}", d["file"].as_str().unwrap_or(""), d["message"].as_str().unwrap_or(""))).unwrap_or_default();
    WsError::new(Code::BuildFailed, format!("the control does not build: {first}")).with_data(json!({ "diagnostics": diagnostics }))
}

fn sha256_digest(v: &Value) -> String {
    format!("sha256:{}", sha256_hex(canonical_json(v).unwrap_or_default().as_bytes()))
}

/// The id of the FileObject holding `bytes` (what staging them gives).
pub fn file_object_id(bytes: &[u8]) -> WsResult<String> {
    Ok(file_object(bytes.len() as u64, &chunk_id(bytes))?.0)
}

/// The input digest of a build: the files by name and object, the manifest version, the builder.
pub fn input_digest(files: &[(String, String)], manifest_version: u64) -> String {
    let mut sorted = files.to_vec();
    sorted.sort();
    sha256_digest(&json!({ "files": sorted, "manifest_version": manifest_version, "builder_version": BUILDER_VERSION }))
}

/// The content digest of a control (AIWS manifest `digest`): `[[name, sha256 hex of its bytes], …]` sorted by name.
pub fn content_digest<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> String {
    let mut list: Vec<(String, String)> = files.into_iter().map(|(n, b)| (n.to_string(), sha256_hex(b))).collect();
    list.sort();
    sha256_digest(&json!(list))
}

// ---- component.json

const COMPONENT_KEYS: &[&str] = &[
    "manifest_version", "ident", "kind", "title", "description", "host_api", "accepts", "allow_no_source", "default_size", "config_schema", "bindings", "entry", "assets",
    "catalog", "agent", "deps", "events", "actions", "on_bind", "on_init_control",
];
const ACTION_HANDLER_KINDS: &[&str] = &["write", "wish", "open", "notify", "config", "forward"];

/// (manifest 4, 控件机制 TODO H2) `actions [{ id, title, icon?, modes?, needs?, run }]`: what the host offers on the near
/// toolbar of an instance (in view mode too), also before an HTML control runs — the host runs `run` (handlers as in
/// `on`, plus `config` writing the instance's properties and `forward` handing it to the running HTML; strings may hold
/// `{{now}}`, the service's clock).
fn check_actions(v: &Value, file: &str, d: &mut Diags) {
    let Some(list) = v.as_array().filter(|l| l.len() <= 16) else {
        d.error(file, "actions: at most 16 of { id, title, icon?, modes?, needs?, run }");
        return;
    };
    let mut ids = BTreeSet::new();
    for (i, a) in list.iter().enumerate() {
        let at = format!("actions[{i}]");
        let Some(o) = a.as_object() else {
            d.error(file, format!("{at} must be an object"));
            continue;
        };
        for k in o.keys() {
            if !matches!(k.as_str(), "id" | "title" | "icon" | "modes" | "needs" | "run" | "description") {
                d.error(file, format!("{at}: unknown key {k} (id, title, icon, modes, needs, run, description)"));
            }
        }
        if !o.get("id").and_then(Value::as_str).is_some_and(|id| is_event_name(id) && ids.insert(id.to_string())) {
            d.error(file, format!("{at}.id: a unique name [a-z][a-z0-9_]{{0,31}}"));
        }
        if !o.get("title").and_then(Value::as_str).is_some_and(|t| !t.is_empty() && t.chars().count() <= 64) {
            d.error(file, format!("{at}.title: 1…64 characters"));
        }
        if o.get("modes").is_some_and(|m| !m.as_array().is_some_and(|m| m.iter().all(|x| matches!(x.as_str(), Some("edit" | "view"))))) {
            d.error(file, format!("{at}.modes: edit and / or view"));
        }
        if o.get("needs").is_some_and(|m| !m.as_array().is_some_and(|m| m.iter().all(|x| matches!(x.as_str(), Some("read" | "comment" | "append" | "update" | "structure" | "delete"))))) {
            d.error(file, format!("{at}.needs: capabilities (update, structure …)"));
        }
        let handlers: Vec<&Value> = match o.get("run") {
            Some(Value::Array(h)) if !h.is_empty() && h.len() <= 4 => h.iter().collect(),
            Some(h @ Value::Object(_)) => vec![h],
            _ => {
                d.error(file, format!("{at}.run: a handler or 1…4 of them"));
                vec![]
            }
        };
        for h in handlers {
            if !h.get("kind").and_then(Value::as_str).is_some_and(|k| ACTION_HANDLER_KINDS.contains(&k)) {
                d.error(file, format!("{at}.run: kind is one of {}", ACTION_HANDLER_KINDS.join(", ")));
            }
        }
    }
}
/// What only `manifest_version` 2 may use (P1).
const V2_HINT: &str = "needs manifest_version 2 (raise it in component.json)";
/// What only `manifest_version` 3 may use (P2 / P3).
const V3_HINT: &str = "needs manifest_version 3 (raise it in component.json)";
/// What only `manifest_version` 4 may use (控件机制 TODO G13, H2).
const V4_HINT: &str = "needs manifest_version 4 (raise it in component.json)";
/// What only `manifest_version` 5 may use (控件 on_bind TODO).
const V5_HINT: &str = "needs manifest_version 5 (raise it in component.json)";
const PARAM_TYPES: &[&str] = &["string", "number", "boolean", "object", "entity"];

/// `events { <name>: { title?, description?, params?: { <p>: { type, required?, description? } } } }` of a render control
/// (Q26): ≤ 16 names `[a-z][a-z0-9_]{0,31}`, ≤ 16 params each.
fn check_events(v: &Value, file: &str, d: &mut Diags) -> Map<String, Value> {
    let Some(o) = v.as_object() else {
        d.error(file, "events must be an object of name → { title?, description?, params? }");
        return Map::new();
    };
    if o.len() > MAX_EVENTS {
        d.error(file, format!("at most {MAX_EVENTS} events"));
    }
    for (name, e) in o {
        let at = format!("events.{name}");
        if !is_event_name(name) {
            d.error(file, format!("{at}: an event name is [a-z][a-z0-9_]{{0,31}}"));
        }
        let Some(e) = e.as_object() else {
            d.error(file, format!("{at} must be an object"));
            continue;
        };
        for (k, x) in e {
            match k.as_str() {
                "title" if x.as_str().is_some_and(|t| t.chars().count() <= 256) => {}
                "description" if x.as_str().is_some_and(|t| t.chars().count() <= 4000) => {}
                "params" => match x.as_object() {
                    Some(ps) if ps.len() <= 16 => {
                        for (pn, pd) in ps {
                            if !is_event_name(pn) {
                                d.error(file, format!("{at}.params.{pn}: a param name is [a-z][a-z0-9_]{{0,31}}"));
                            }
                            let ok = pd.as_object().is_some_and(|o| {
                                o.get("type").and_then(Value::as_str).is_some_and(|t| PARAM_TYPES.contains(&t))
                                    && o.iter().all(|(k, v)| match k.as_str() {
                                        "type" => true,
                                        "required" => v.is_boolean(),
                                        "description" => v.as_str().is_some_and(|t| t.chars().count() <= 1000),
                                        _ => false,
                                    })
                            });
                            if !ok {
                                d.error(file, format!("{at}.params.{pn}: {{ type: {}, required?, description? }}", PARAM_TYPES.join(" | ")));
                            }
                        }
                    }
                    _ => d.error(file, format!("{at}.params: an object of at most 16 params")),
                },
                "title" | "description" => d.error(file, format!("{at}.{k}: too long or not a text")),
                _ => d.error(file, format!("{at}: unknown key {k} (title, description, params)")),
            }
        }
    }
    o.clone()
}
const SCHEMA_TYPES: &[&str] = &["object", "array", "string", "number", "integer", "boolean", "null"];

/// The JSON Schema subset instance properties may use (A7): `type`, `enum`, `default`, `required`, `title`,
/// `description`, `minimum`, `maximum`, `minLength`, `maxLength`, nested through `properties` and `items`.
pub fn check_config_schema(v: &Value, path: &str, out: &mut Vec<String>) {
    let Some(o) = v.as_object() else {
        out.push(format!("{path}: a schema is an object"));
        return;
    };
    for (k, x) in o {
        let at = format!("{path}/{k}");
        let ok = match k.as_str() {
            "type" => x.as_str().is_some_and(|t| SCHEMA_TYPES.contains(&t)) || x.as_array().is_some_and(|l| !l.is_empty() && l.iter().all(|t| t.as_str().is_some_and(|t| SCHEMA_TYPES.contains(&t)))),
            "enum" => x.is_array(),
            "default" => true,
            "required" => x.as_array().is_some_and(|l| l.iter().all(Value::is_string)),
            "title" | "description" => x.is_string(),
            "minimum" | "maximum" => x.is_number(),
            "minLength" | "maxLength" => x.as_u64().is_some(),
            "properties" => match x.as_object() {
                Some(props) => {
                    for (name, s) in props {
                        check_config_schema(s, &format!("{at}/{name}"), out);
                    }
                    true
                }
                None => false,
            },
            "items" => {
                check_config_schema(x, &at, out);
                true
            }
            _ => {
                out.push(format!("{at}: keyword {k} is outside the supported subset (type, enum, default, required, title, description, minimum, maximum, minLength, maxLength, properties, items)"));
                continue;
            }
        };
        if !ok {
            out.push(format!("{at}: bad value"));
        }
    }
}

const ITEM_KINDS: &[&str] = &["text", "value", "metric", "bar", "list", "badge", "button"];
const ACTION_COMMANDS: &[&str] = &["open_source", "activate_editor", "open_definition"];
const INSPECTOR_KINDS: &[&str] = &["text", "number", "select", "boolean", "color"];

/// The declarative vocabulary (phase two §10.3): what the interpreter can draw. `bind` reads the main source, or a
/// named binding with `<binding>.<expr>` (Q18).
fn check_declarative(spec: &Value, bindings: &Map<String, Value>, events: &Map<String, Value>, manifest_version: u64, file: &str, d: &mut Diags) {
    let Some(o) = spec.as_object() else {
        d.error(file, "a declarative definition is a JSON object");
        return;
    };
    let items = |list: Option<&Value>, at: &str, d: &mut Diags| {
        let Some(list) = list else { return };
        let Some(list) = list.as_array() else {
            d.error(file, format!("{at} must be an array"));
            return;
        };
        for (i, it) in list.iter().enumerate() {
            let at = format!("{at}[{i}]");
            let Some(it) = it.as_object() else {
                d.error(file, format!("{at} must be an object"));
                continue;
            };
            match it.get("kind").and_then(Value::as_str) {
                Some("button") if manifest_version < 3 => d.error(file, format!("{at}: a button {V3_HINT}")),
                Some("button") => {
                    if !it.get("label").and_then(Value::as_str).is_some_and(|l| !l.is_empty() && l.chars().count() <= 64) {
                        d.error(file, format!("{at}.label: 1…64 characters"));
                    }
                    match it.get("emit").and_then(Value::as_str) {
                        Some(e) if events.contains_key(e) => {}
                        Some(e) => d.0.push(json!({ "file": file, "severity": "error", "code": "EVENT_UNDECLARED", "message": format!("{at}.emit {e:?} is not an event of component.json") })),
                        None => d.error(file, format!("{at}.emit: the event the button raises")),
                    }
                    if it.get("params").is_some_and(|p| !p.is_object()) {
                        d.error(file, format!("{at}.params must be an object"));
                    }
                }
                Some(k) if ITEM_KINDS.contains(&k) => {}
                k => d.error(file, format!("{at}.kind {k:?}: one of {}", ITEM_KINDS.join(", "))),
            }
            for k in it.keys() {
                if !matches!(k.as_str(), "kind" | "text" | "label" | "bind" | "by" | "format" | "limit" | "emit" | "params") {
                    d.warn(file, format!("{at}: key {k} is ignored by the interpreter"));
                }
            }
            if let Some(f) = it.get("format").filter(|f| !f.as_str().is_some_and(|f| matches!(f, "text" | "number" | "percent" | "currency"))) {
                d.error(file, format!("{at}.format {f}: text, number, percent or currency"));
            }
            if let Some(bind) = it.get("bind").and_then(Value::as_str) {
                if let Some((prefix, _)) = bind.split_once('.') {
                    if prefix != "props" && !bindings.contains_key(prefix) {
                        d.warn(file, format!("{at}.bind {bind:?}: {prefix} is not a declared binding, so it reads the field {bind:?} of the main source"));
                    }
                }
            }
        }
    };
    for (k, v) in o {
        match k.as_str() {
            "layout" => {
                if !v.as_str().is_some_and(|l| matches!(l, "stack" | "row")) {
                    d.error(file, "layout: stack or row");
                }
            }
            "items" => items(Some(v), "items", d),
            "view" => items(v.get("items"), "view.items", d),
            "accent" => {
                if !v.is_string() {
                    d.error(file, "accent must be a colour string");
                }
            }
            "actions" => {
                for (i, a) in v.as_array().into_iter().flatten().enumerate() {
                    if !a.get("command").and_then(Value::as_str).is_some_and(|c| ACTION_COMMANDS.contains(&c)) {
                        d.error(file, format!("actions[{i}].command: one of {}", ACTION_COMMANDS.join(", ")));
                    }
                    if !a.get("id").is_some_and(Value::is_string) || !a.get("label").is_some_and(Value::is_string) {
                        d.error(file, format!("actions[{i}] needs id and label"));
                    }
                }
            }
            "inspector" => {
                for (i, f) in v.as_array().into_iter().flatten().enumerate() {
                    if !f.get("kind").and_then(Value::as_str).is_some_and(|c| INSPECTOR_KINDS.contains(&c)) {
                        d.error(file, format!("inspector[{i}].kind: one of {}", INSPECTOR_KINDS.join(", ")));
                    }
                    if !f.get("key").is_some_and(Value::is_string) || !f.get("label").is_some_and(Value::is_string) {
                        d.error(file, format!("inspector[{i}] needs key and label"));
                    }
                }
            }
            _ => d.warn(file, format!("key {k} is not part of the declarative vocabulary")),
        }
    }
    if o.get("items").is_none() && o.get("view").is_none() {
        d.warn(file, "no items: the control draws nothing");
    }
}

/// `{{config.<key>}}` placeholders in a string of a composition: each names a property of `config_schema`.
fn check_placeholders(text: &str, at: &str, props: &Map<String, Value>, file: &str, d: &mut Diags) {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else {
            d.error(file, format!("{at}: unclosed {{{{"));
            return;
        };
        let inner = rest[start + 2..start + len].trim();
        match inner.strip_prefix("config.") {
            Some(key) if props.contains_key(key) => {}
            Some(key) => d.error(file, format!("{at}: {{{{config.{key}}}}} names no property of config_schema")),
            None => d.error(file, format!("{at}: only {{{{config.<key>}}}} may be used here")),
        }
        rest = &rest[start + len + 2..];
    }
}

/// A number of a composition (`spacing`, `padding`, `radius`): within [0, max], or a `{{config.<key>}}` string.
fn check_dimension(v: &Value, at: &str, max: f64, props: &Map<String, Value>, file: &str, d: &mut Diags) {
    match v {
        Value::Number(n) if n.as_f64().is_some_and(|n| (0.0..=max).contains(&n)) => {}
        Value::String(t) if t.trim().starts_with("{{") && t.trim().ends_with("}}") => check_placeholders(t, at, props, file, d),
        _ => d.error(file, format!("{at}: a number in [0, {max}] or {{{{config.<key>}}}}")),
    }
}

/// `composition.json` of a composite (K3, README “Composite controls”): `layout` (an algorithm the host ships:
/// `stack` | `fixed`), `slots` (where user Blocks go: what they accept, how many, where), `chrome` (what the host draws
/// around them). String values may say `{{config.<key>}}` for a property of `config_schema`. Answers the identities
/// slots accept as `cell:<ident>`.
/// What a composition declares besides its shape: the controls its slots accept, the events it re-raises from a slot's
/// control (name, slot, event) and the events a projection's role raises (name, projection, role).
type CompositionRefs = (Vec<String>, Vec<(String, String, String)>, Vec<(String, String, String)>);

fn check_composition(spec: &Value, config_schema: Option<&Value>, manifest_version: u64, file: &str, d: &mut Diags) -> CompositionRefs {
    let mut cells = Vec::new();
    let mut bubbles = Vec::new();
    let mut projected = Vec::new();
    let empty = Map::new();
    let props = config_schema.and_then(|s| s.get("properties")).and_then(Value::as_object).unwrap_or(&empty);
    let Some(o) = spec.as_object() else {
        d.error(file, "a composition is a JSON object { layout, slots, chrome?, events? }");
        return (cells, bubbles, projected);
    };
    for k in o.keys() {
        match k.as_str() {
            "layout" | "slots" | "chrome" => {}
            "events" if manifest_version < 3 => d.error(file, format!("events {V3_HINT}")),
            "events" => {}
            "projections" | "media" if manifest_version < 4 => d.error(file, format!("{k} {V4_HINT}")),
            "projections" | "media" => {}
            _ => d.error(file, format!("unknown key {k} (a composition says layout, slots, chrome, events, projections and media)")),
        }
    }
    let mut algorithm = "stack";
    match o.get("layout").and_then(Value::as_object) {
        None => d.error(file, "layout { algorithm: stack | fixed | grid } required"),
        Some(l) => {
            match l.get("algorithm").and_then(Value::as_str) {
                Some(a @ ("stack" | "fixed")) => algorithm = a,
                Some("grid") if manifest_version < 3 => d.error(file, format!("layout.algorithm grid {V3_HINT}")),
                Some("grid") => {
                    algorithm = "grid";
                    if !l.get("columns").and_then(Value::as_u64).is_some_and(|n| (1..=12).contains(&n)) {
                        d.error(file, "layout.columns: an integer in [1, 12] (grid)");
                    }
                }
                a => d.error(file, format!("layout.algorithm {a:?}: stack, fixed or grid (the algorithms the host ships)")),
            }
            for (k, v) in l {
                match k.as_str() {
                    "algorithm" => {}
                    "direction" if algorithm == "stack" => {
                        if !matches!(v.as_str(), Some("column" | "row")) {
                            d.error(file, "layout.direction: column or row");
                        }
                    }
                    "spacing" if algorithm == "stack" => check_dimension(v, "layout.spacing", 2000.0, props, file, d),
                    "columns" if algorithm == "grid" => {}
                    "gap" if algorithm == "grid" => check_dimension(v, "layout.gap", 2000.0, props, file, d),
                    "columns" | "gap" => d.error(file, format!("layout.{k} belongs to the grid layout")),
                    "padding" => match v.as_object() {
                        Some(sides) => {
                            for (side, n) in sides {
                                if !matches!(side.as_str(), "t" | "r" | "b" | "l") {
                                    d.error(file, format!("layout.padding.{side}: t, r, b or l"));
                                }
                                check_dimension(n, &format!("layout.padding.{side}"), 2000.0, props, file, d);
                            }
                        }
                        None => check_dimension(v, "layout.padding", 2000.0, props, file, d),
                    },
                    "direction" | "spacing" => d.error(file, format!("layout.{k} belongs to the stack layout")),
                    _ => d.error(file, format!("layout: unknown key {k}")),
                }
            }
        }
    }
    let slots = match o.get("slots").and_then(Value::as_object) {
        Some(s) => s.clone(),
        None => {
            d.error(file, "slots { <name>: { accepts?, min?, max?, placement? } } required");
            Map::new()
        }
    };
    if slots.len() > MAX_SLOTS {
        d.error(file, format!("at most {MAX_SLOTS} slots"));
    }
    for (name, slot) in &slots {
        let at = format!("slots.{name}");
        if !is_slot_name(name) {
            d.error(file, format!("{at}: a slot name is [a-z][a-z0-9_]{{0,31}}"));
        }
        let Some(slot) = slot.as_object() else {
            d.error(file, format!("{at} must be an object"));
            continue;
        };
        for (k, v) in slot {
            match k.as_str() {
                "accepts" => match v.as_array().filter(|a| !a.is_empty() && a.len() <= 32) {
                    None => d.error(file, format!("{at}.accepts: 1…32 entries (cell:<ident>, a view type, container)")),
                    Some(list) => {
                        for a in list {
                            match a.as_str() {
                                Some(c) if c.starts_with("cell:") => match c[5..].split_once('#').map(|(did, n)| ident_string(&json!({ "author_did": did, "name": n }))) {
                                    Some(Ok(id)) => cells.push(id),
                                    _ => d.error(file, format!("{at}.accepts {c:?}: cell:<author_did>#<name>")),
                                },
                                Some("container") => {}
                                Some(view) if is_renderer_id(view) => {}
                                _ => d.error(file, format!("{at}.accepts {a}: cell:<ident>, a view type or container")),
                            }
                        }
                    }
                },
                "min" => {
                    if !v.as_u64().is_some_and(|n| n <= 64) {
                        d.error(file, format!("{at}.min: an integer in [0, 64]"));
                    }
                }
                "max" => {
                    if !v.as_u64().is_some_and(|n| (1..=1000).contains(&n)) {
                        d.error(file, format!("{at}.max: an integer in [1, 1000]"));
                    }
                }
                "placement" => match v.as_str() {
                    Some("stack") if algorithm == "grid" => {}
                    _ if algorithm == "grid" => d.error(file, format!("{at}.placement: a grid places every child in a cell (stack)")),
                    Some("free") => {}
                    Some("stack" | "top" | "bottom") if algorithm == "stack" => {}
                    Some("stack" | "top" | "bottom") => d.error(file, format!("{at}.placement: a fixed layout keeps every child where it is (free)")),
                    _ => d.error(file, format!("{at}.placement: stack, top, bottom or free")),
                },
                "title" => {
                    if !v.as_str().is_some_and(|t| t.chars().count() <= 256) {
                        d.error(file, format!("{at}.title: at most 256 characters"));
                    }
                }
                "visible" if manifest_version < 4 => d.error(file, format!("{at}.visible {V4_HINT}")),
                "visible" => {
                    // shown to each viewer for a while from their first opening (控件机制 TODO G19): a presentation only
                    let ok = v.get("from").and_then(Value::as_str) == Some("first_open") && v.as_object().is_some_and(|o| o.keys().all(|k| k == "from" || k == "for_ms"));
                    if !ok {
                        d.error(file, format!("{at}.visible: {{ from: \"first_open\", for_ms }}"));
                    }
                    match v.get("for_ms") {
                        Some(n) if n.as_u64().is_some_and(|n| (1..=366 * 86_400_000).contains(&n)) => {}
                        Some(Value::String(t)) => check_placeholders(t, &format!("{at}.visible.for_ms"), props, file, d),
                        _ => d.error(file, format!("{at}.visible.for_ms: milliseconds (≤ a year) or {{{{config.<key>}}}}")),
                    }
                }
                _ => d.error(file, format!("{at}: unknown key {k}")),
            }
        }
        if let (Some(min), Some(max)) = (slot.get("min").and_then(Value::as_u64), slot.get("max").and_then(Value::as_u64)) {
            if min > max {
                d.error(file, format!("{at}: min exceeds max"));
            }
        }
    }
    if let Some(c) = o.get("chrome") {
        match c.as_object() {
            None => d.error(file, "chrome must be an object"),
            Some(c) => {
                for (k, v) in c {
                    let at = format!("chrome.{k}");
                    match k.as_str() {
                        "background" | "border" => match v.as_str() {
                            Some(t) if t.chars().count() <= 64 => check_placeholders(t, &at, props, file, d),
                            _ => d.error(file, format!("{at}: a colour (≤ 64 characters) or {{{{config.<key>}}}}")),
                        },
                        "radius" => check_dimension(v, &at, 200.0, props, file, d),
                        "empty" => match v.as_str() {
                            Some(t) if t.chars().count() <= 1000 => check_placeholders(t, &at, props, file, d),
                            _ => d.error(file, format!("{at}: a text of at most 1000 characters")),
                        },
                        "title" => {
                            let Some(t) = v.as_object() else {
                                d.error(file, format!("{at}: {{ text, slot? }}"));
                                continue;
                            };
                            for (tk, tv) in t {
                                match tk.as_str() {
                                    "text" => match tv.as_str() {
                                        Some(x) if x.chars().count() <= 256 => check_placeholders(x, "chrome.title.text", props, file, d),
                                        _ => d.error(file, "chrome.title.text: at most 256 characters"),
                                    },
                                    "slot" => {
                                        if !tv.as_str().is_some_and(|s| slots.contains_key(s)) {
                                            d.error(file, format!("chrome.title.slot {tv}: a slot of this composition"));
                                        }
                                    }
                                    _ => d.error(file, format!("chrome.title: unknown key {tk}")),
                                }
                            }
                            if !t.contains_key("text") {
                                d.error(file, "chrome.title.text required");
                            }
                        }
                        _ => d.error(file, format!("chrome: unknown key {k} (background, radius, border, title, empty)")),
                    }
                }
            }
        }
    }
    if let Some(ev) = o.get("events").filter(|_| manifest_version >= 3) {
        match ev.as_object() {
            None => d.error(file, "events must be an object of name → { from: { slot, event }, title?, description? }"),
            Some(ev) => {
                if ev.len() > MAX_EVENTS {
                    d.error(file, format!("at most {MAX_EVENTS} events"));
                }
                for (name, e) in ev {
                    let at = format!("events.{name}");
                    if !is_event_name(name) {
                        d.error(file, format!("{at}: an event name is [a-z][a-z0-9_]{{0,31}}"));
                    }
                    let from = e.get("from");
                    // (manifest 4) a press on a role of a projection's template raises it (控件机制 TODO G13)
                    if let Some(projection) = from.and_then(|f| f.get("projection")) {
                        let role = from.and_then(|f| f.get("role")).and_then(Value::as_str).unwrap_or("body");
                        match projection.as_str() {
                            _ if manifest_version < 4 => d.error(file, format!("{at}.from.projection {V4_HINT}")),
                            Some(p) if o.get("projections").and_then(|ps| ps.get(p)).is_some() && is_slot_name(role) => projected.push((name.clone(), p.to_string(), role.to_string())),
                            _ => d.error(file, format!("{at}.from: {{ projection, role? }} (a projection of this composition and a role of its template)")),
                        }
                        check_event_keys(e, &at, manifest_version, file, d);
                        continue;
                    }
                    let (slot, event) = (from.and_then(|f| f.get("slot")).and_then(Value::as_str), from.and_then(|f| f.get("event")).and_then(Value::as_str));
                    match (slot, event) {
                        (Some(sl), Some(ev)) if slots.contains_key(sl) && is_event_name(ev) => bubbles.push((name.clone(), sl.to_string(), ev.to_string())),
                        (Some(sl), Some(_)) if !slots.contains_key(sl) => d.error(file, format!("{at}.from.slot {sl:?} is not a slot of this composition")),
                        _ => d.error(file, format!("{at}.from: {{ slot, event }} (a slot of this composition and an event of its controls)")),
                    }
                    check_event_keys(e, &at, manifest_version, file, d);
                }
            }
        }
    }
    if let Some(ps) = o.get("projections").filter(|_| manifest_version >= 4) {
        check_projections(ps, props, file, d);
    }
    if let Some(m) = o.get("media").filter(|_| manifest_version >= 4) {
        // (G17) the media Block of a slot plays where `config.<anchor>` says
        let slot_ok = m.get("slot").and_then(Value::as_str).is_some_and(|sl| slots.contains_key(sl));
        let prop = |k: &str| m.get(k).and_then(Value::as_str).is_some_and(|p| props.is_empty() || props.contains_key(p));
        if !slot_ok || !prop("anchor") || (m.get("loop").is_some() && !prop("loop")) || !m.as_object().is_some_and(|o| o.keys().all(|k| matches!(k.as_str(), "slot" | "anchor" | "loop"))) {
            d.error(file, "media: { slot (a slot of this composition), anchor (a config property), loop? (a config property) }");
        }
    }
    cells.sort();
    cells.dedup();
    (cells, bubbles, projected)
}

/// A composite event's keys: `from`, `title`, `description` and (manifest 4) `on` — the handler(s) it runs when the
/// instance binds none of its own (a photo vote's pick toggles the picker's choice as soon as it is inserted).
fn check_event_keys(e: &Value, at: &str, manifest_version: u64, file: &str, d: &mut Diags) {
    for (k, v) in e.as_object().into_iter().flatten() {
        match k.as_str() {
            "from" | "title" | "description" => {}
            "on" if manifest_version < 4 => d.error(file, format!("{at}.on {V4_HINT}")),
            "on" => {
                if let Err(err) = aiworkspace_core::types::check_on_shape(&json!({ "e": v })) {
                    d.error(file, format!("{at}.on: {}", err.detail.replace("on.e: ", "")));
                }
            }
            _ => d.error(file, format!("{at}: unknown key {k} (from, title, description, on)")),
        }
    }
}

/// (manifest 4, 控件机制 TODO G2, G13) `projections { <name>: { source, mark?, template, layout, placement?, max_items? } }`:
/// Blocks projected from data — the host's projection, the shape checked here.
fn check_projections(v: &Value, props: &Map<String, Value>, file: &str, d: &mut Diags) {
    let Some(ps) = v.as_object() else {
        d.error(file, "projections: an object of name → { source, template, layout }");
        return;
    };
    if ps.len() > 8 {
        d.error(file, "at most 8 projections");
    }
    for (name, p) in ps {
        let at = format!("projections.{name}");
        if !is_slot_name(name) {
            d.error(file, format!("{at}: a projection name is [a-z][a-z0-9_]{{0,31}}"));
        }
        let Some(p) = p.as_object() else {
            d.error(file, format!("{at} must be an object"));
            continue;
        };
        for k in p.keys() {
            if !matches!(k.as_str(), "source" | "mark" | "template" | "layout" | "placement" | "max_items") {
                d.error(file, format!("{at}: unknown key {k} (source, mark, template, layout, placement, max_items)"));
            }
        }
        let src = p.get("source");
        let binding_ok = |b: Option<&Value>| b.and_then(Value::as_str).is_some_and(|b| b.len() <= 32 && b.chars().next().is_some_and(|c| c.is_ascii_lowercase()));
        match src.and_then(|s| s.get("kind")).and_then(Value::as_str) {
            Some("records" | "members") if binding_ok(src.and_then(|s| s.get("binding"))) => {}
            Some("counts") if binding_ok(src.and_then(|s| s.get("binding"))) && src.and_then(|s| s.get("field")).and_then(Value::as_str).is_some() => {
                if let Some(of) = src.and_then(|s| s.get("of")) {
                    if !of.as_str().is_some_and(|o| ps.contains_key(o) && o != name) {
                        d.error(file, format!("{at}.source.of: another projection of this composition"));
                    }
                }
                if src.and_then(|s| s.get("top")).is_some_and(|t| !t.as_u64().is_some_and(|n| (1..=100).contains(&n))) {
                    d.error(file, format!("{at}.source.top: 1…100"));
                }
            }
            Some("viewer") => {}
            _ => d.error(file, format!("{at}.source: {{ kind: records | members | counts | viewer, binding (a binding name), field? (counts) }}")),
        }
        if let Some(m) = p.get("mark") {
            let ok = binding_ok(m.get("binding")) && m.get("field").and_then(Value::as_str).is_some() && m.get("as").and_then(Value::as_str).is_some_and(is_slot_name);
            if !ok {
                d.error(file, format!("{at}.mark: {{ binding, field, as }}"));
            }
        }
        match p.get("template").and_then(|t| t.get("nodes")).and_then(Value::as_array) {
            Some(nodes) if !nodes.is_empty() && nodes.len() <= 32 => {
                let mut ids = BTreeSet::new();
                for (i, n) in nodes.iter().enumerate() {
                    let id = n.get("id").and_then(Value::as_str);
                    let view = n.get("view").and_then(Value::as_str);
                    if !id.is_some_and(is_slot_name) || !ids.insert(id.unwrap_or_default().to_string()) {
                        d.error(file, format!("{at}.template.nodes[{i}].id: a unique name [a-z][a-z0-9_]{{0,31}}"));
                    }
                    if !matches!(view, Some("label" | "shape" | "asset")) {
                        d.error(file, format!("{at}.template.nodes[{i}].view: label, shape or asset (the projected Blocks; Q1)"));
                    }
                    if n.get("role").is_some_and(|r| !r.as_str().is_some_and(is_slot_name)) {
                        d.error(file, format!("{at}.template.nodes[{i}].role: a name [a-z][a-z0-9_]{{0,31}}"));
                    }
                    if let Some(c) = n.get("config").and_then(Value::as_object) {
                        for (k, v) in c {
                            if let Some(t) = v.as_str() {
                                check_projection_placeholders(t, &format!("{at}.template.nodes[{i}].config.{k}"), props, file, d);
                            }
                        }
                    }
                }
            }
            _ => d.error(file, format!("{at}.template: {{ nodes: [1…32 of {{ id, view: label | shape | asset, box?, config?, when?, role? }}] }}")),
        }
        match p.get("layout") {
            Some(l) if matches!(l.get("algorithm").and_then(Value::as_str), Some("list" | "grid")) && l.get("item").and_then(|i| i.get("h")).is_some() => {
                if l.get("algorithm").and_then(Value::as_str) == Some("grid") && l.get("columns").is_none() {
                    d.error(file, format!("{at}.layout.columns required (grid)"));
                }
            }
            _ => d.error(file, format!("{at}.layout: {{ algorithm: list | grid, columns? (grid), gap?, item: {{ w?, h }} }}")),
        }
        if p.get("placement").is_some_and(|v| !matches!(v.as_str(), Some("top" | "stack" | "bottom"))) {
            d.error(file, format!("{at}.placement: top, stack or bottom"));
        }
        if p.get("max_items").is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=1000).contains(&n))) {
            d.error(file, format!("{at}.max_items: 1…1000"));
        }
    }
}

/// A template value's placeholders: `{{item.…}}`, `{{viewer.…}}` (the data), `{{config.<key>}}` (a property).
fn check_projection_placeholders(text: &str, at: &str, props: &Map<String, Value>, file: &str, d: &mut Diags) {
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        let Some(len) = rest[start..].find("}}") else { break };
        let inner = rest[start + 2..start + len].trim();
        match inner.split_once('.') {
            Some(("item" | "viewer", path)) if !path.is_empty() => {}
            Some(("config", key)) if props.is_empty() || props.contains_key(key) => {}
            _ => d.error(file, format!("{at}: {{{{{inner}}}}} — a placeholder is {{{{item.<field>}}}}, {{{{viewer.<key>}}}} or {{{{config.<property>}}}}")),
        }
        rest = &rest[start + len + 2..];
    }
}

fn text_file<'a>(files: &'a BTreeMap<&str, &SourceFile>, name: &str, what: &str, d: &mut Diags) -> Option<&'a str> {
    match files.get(name) {
        None => {
            d.error(COMPONENT_FILE, format!("{what} {name:?} is not in the directory"));
            None
        }
        Some(f) => match std::str::from_utf8(&f.bytes) {
            Ok(t) => Some(t),
            Err(_) => {
                d.error(name, "not UTF-8 text");
                None
            }
        },
    }
}

/// Check a source directory and compose its definition (step 1 of a build; nothing of the Workspace is read).
pub fn compose(files: &[SourceFile]) -> Result<Composed, Vec<Value>> {
    let mut d = Diags(Vec::new());
    if files.is_empty() || files.len() > MAX_SOURCE_FILES {
        d.error("", format!("a source directory holds 1…{MAX_SOURCE_FILES} files"));
        return Err(d.0);
    }
    let mut by_name: BTreeMap<&str, &SourceFile> = BTreeMap::new();
    for f in files {
        if check_member_name(&f.name).is_err() || f.name.split('/').any(|seg| seg.starts_with('.')) {
            d.error(&f.name, "a file of a source directory: a relative path whose parts do not start with '.'");
        } else if by_name.insert(&f.name, f).is_some() {
            d.error(&f.name, "listed twice");
        }
    }
    let Some(component) = by_name.get(COMPONENT_FILE) else {
        d.error(COMPONENT_FILE, "missing: every control directory has a component.json");
        return Err(d.0);
    };
    let c: Value = match serde_json::from_slice(&component.bytes) {
        Ok(v) => v,
        Err(e) => {
            d.json_error(COMPONENT_FILE, &e);
            return Err(d.0);
        }
    };
    let Some(c) = c.as_object() else {
        d.error(COMPONENT_FILE, "must be a JSON object");
        return Err(d.0);
    };
    let f = COMPONENT_FILE;
    for k in c.keys() {
        if !COMPONENT_KEYS.contains(&k.as_str()) {
            d.error(f, format!("unknown key {k} (manifest_version {MANIFEST_VERSION} knows {})", COMPONENT_KEYS.join(", ")));
        }
    }
    let manifest_version = match c.get("manifest_version").and_then(Value::as_u64) {
        Some(v) if (1..=MANIFEST_VERSION).contains(&v) => v,
        Some(v) => {
            d.error(f, format!("manifest_version {v} is newer than this service reads ({MANIFEST_VERSION})"));
            return Err(d.0);
        }
        None => {
            d.error(f, format!("manifest_version (1…{MANIFEST_VERSION}) required"));
            return Err(d.0);
        }
    };
    let ident = match c.get("ident").map(ident_string) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            d.error(f, e.detail);
            String::new()
        }
        None => {
            d.error(f, "ident { author_did, name } required");
            String::new()
        }
    };
    let name = c.get("ident").and_then(|i| i.get("name")).and_then(Value::as_str).unwrap_or("").to_string();
    let kind = c.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
    match kind.as_str() {
        "declarative" | "html" => {}
        COMPOSITE if manifest_version < 2 => d.error(f, format!("kind composite {V2_HINT}")),
        COMPOSITE => {
            for k in ["accepts", "allow_no_source"] {
                if c.contains_key(k) {
                    d.error(f, format!("{k}: a composite is a container of Blocks, not a Cell showing data"));
                }
            }
        }
        _ => d.error(f, "kind must be declarative, html or composite"),
    }
    let mut payload = Map::new();
    payload.insert("def_id".into(), json!(name));
    payload.insert("kind".into(), json!(kind));
    if let Some(i) = c.get("ident") {
        payload.insert("ident".into(), i.clone());
    }
    payload.insert("manifest_version".into(), json!(manifest_version));
    match c.get("title").and_then(Value::as_str) {
        Some(t) if !t.is_empty() && t.chars().count() <= 256 => {
            payload.insert("title".into(), json!(t));
        }
        _ => d.error(f, "title: 1…256 characters"),
    }
    match c.get("description") {
        None => {}
        Some(Value::String(t)) if t.chars().count() <= 4000 => {
            payload.insert("description".into(), json!(t));
        }
        Some(_) => d.error(f, "description: at most 4000 characters"),
    }
    let host_api = match c.get("host_api") {
        None => None,
        Some(v) => match v.as_u64().filter(|n| *n >= 1) {
            Some(n) => Some(n),
            None => {
                d.error(f, "host_api must be a positive integer");
                None
            }
        },
    };
    if let Some(a) = c.get("accepts") {
        if a.as_array().is_some_and(|l| l.iter().all(|t| t.as_str().is_some_and(is_data_type))) {
            payload.insert("accepts".into(), a.clone());
        } else {
            d.error(f, "accepts must list data types (buckyos.table-source, buckyos.record, …)");
        }
    }
    if let Some(a) = c.get("allow_no_source") {
        if a.is_boolean() {
            payload.insert("allow_no_source".into(), a.clone());
        } else {
            d.error(f, "allow_no_source must be a boolean");
        }
    }
    if let Some(s) = c.get("default_size") {
        let dim = |k: &str| s.get(k).and_then(Value::as_f64).is_some_and(|n| n > 0.0 && n.is_finite());
        if dim("w") && dim("h") && s.as_object().is_some_and(|o| o.len() == 2) {
            payload.insert("default_size".into(), s.clone());
        } else {
            d.error(f, "default_size must be { w, h } (positive)");
        }
    }
    if let Some(s) = c.get("config_schema") {
        let mut problems = Vec::new();
        check_config_schema(s, "config_schema", &mut problems);
        for p in &problems {
            d.error(f, p.clone());
        }
        if problems.is_empty() {
            payload.insert("config_schema".into(), s.clone());
        }
    }
    let bindings = c.get("bindings").and_then(Value::as_object).cloned().unwrap_or_default();
    if let Some(b) = c.get("bindings") {
        match check_def_bindings(b) {
            Ok(()) if bindings.contains_key("props") => d.error(f, "bindings: `props` is reserved (bind \"props.<key>\" reads the main record)"),
            Ok(()) => {
                payload.insert("bindings".into(), b.clone());
            }
            Err(e) => d.error(f, format!("bindings: {}", e.detail)),
        }
    }
    if let Some(cat) = c.get("catalog") {
        match check_catalog(cat) {
            Ok(()) => {
                payload.insert("catalog".into(), cat.clone());
            }
            Err(e) => d.error(f, e.detail),
        }
    }
    let mut deps: Vec<String> = Vec::new();
    match c.get("deps") {
        None => {}
        Some(v) if v.as_array().is_some_and(Vec::is_empty) => {}
        Some(_) if manifest_version < 2 => d.error(f, format!("deps {V2_HINT}")),
        Some(Value::Array(list)) if list.len() <= MAX_DEF_DEPS => {
            for dep in list {
                match dep.as_str().and_then(|x| x.split_once('#')).map(|(did, n)| ident_string(&json!({ "author_did": did, "name": n }))) {
                    Some(Ok(id)) if id == ident => d.error(f, "deps: a control cannot depend on itself"),
                    Some(Ok(id)) if deps.contains(&id) => d.error(f, format!("deps lists {id} twice")),
                    Some(Ok(id)) => deps.push(id),
                    _ => d.error(f, format!("deps: {dep} is not an identity <author_did>#<name>")),
                }
            }
        }
        Some(_) => d.error(f, format!("deps: at most {MAX_DEF_DEPS} identities <author_did>#<name>")),
    }
    let events = match c.get("events") {
        None => Map::new(),
        Some(_) if manifest_version < 3 => {
            d.error(f, format!("events {V3_HINT}"));
            Map::new()
        }
        Some(_) if kind == COMPOSITE => {
            d.error(f, "events: a composite re-raises its children's events, declared in composition.json");
            Map::new()
        }
        Some(v) => check_events(v, f, &mut d),
    };
    if !events.is_empty() {
        payload.insert("events".into(), Value::Object(events.clone()));
    }
    match c.get("actions") {
        None => {}
        Some(_) if manifest_version < 4 => d.error(f, format!("actions {V4_HINT}")),
        Some(v) => {
            check_actions(v, f, &mut d);
            payload.insert("actions".into(), v.clone());
        }
    }
    // (manifest 5) what a composite binds when placed, child controls after it, and its last check (控件 on_bind TODO §2.6);
    // an HTML control decides it in its script (§2.9, host API 5)
    if c.contains_key("on_bind") || c.contains_key("on_init_control") {
        if manifest_version < 5 {
            d.error(f, format!("on_bind / on_init_control {V5_HINT}"));
        } else if kind == "html" {
            match aiworkspace_core::types::check_script_events(c.get("on_bind"), c.get("on_init_control")) {
                Err(e) => d.error(f, e.detail),
                Ok(()) if host_api.unwrap_or(HTML_API_VERSION) < HTML_BIND_API => {
                    d.error(f, format!("on_bind / on_init_control \"script\": aiws.on('bind') / aiws.on('init_control') need host_api {HTML_BIND_API}"))
                }
                Ok(()) => {
                    for k in ["on_bind", "on_init_control"] {
                        if let Some(v) = c.get(k) {
                            payload.insert(k.into(), v.clone());
                        }
                    }
                }
            }
        } else if kind != COMPOSITE {
            d.error(f, "on_bind / on_init_control: declared steps are for a composite, \"script\" for an HTML control");
        } else {
            let empty = Map::new();
            let decls = c.get("bindings").and_then(Value::as_object).unwrap_or(&empty);
            let actions: Vec<String> = c.get("actions").and_then(Value::as_array).into_iter().flatten().filter_map(|a| a.get("id").and_then(Value::as_str)).map(str::to_string).collect();
            match aiworkspace_core::types::check_bind_steps(c.get("on_bind"), c.get("on_init_control"), decls, &actions) {
                Ok(()) => {
                    for k in ["on_bind", "on_init_control"] {
                        if let Some(v) = c.get(k) {
                            payload.insert(k.into(), v.clone());
                        }
                    }
                }
                Err(e) => d.error(f, e.detail),
            }
        }
    }
    let mut slot_cells = Vec::new();
    let mut bubbles = Vec::new();
    let mut named: BTreeSet<String> = BTreeSet::from([COMPONENT_FILE.to_string()]);
    match (kind.as_str(), c.get("entry")) {
        ("html", Some(Value::Object(e))) => {
            let mut html = Map::new();
            for (k, v) in e {
                let Some(file) = v.as_str().filter(|_| matches!(k.as_str(), "html" | "css" | "js")) else {
                    d.error(f, format!("entry.{k}: html, css and js name files"));
                    continue;
                };
                named.insert(file.to_string());
                if let Some(t) = text_file(&by_name, file, &format!("entry.{k}"), &mut d) {
                    html.insert(k.clone(), json!(t));
                }
            }
            if !html.contains_key("html") && !e.contains_key("html") {
                d.error(f, "entry.html (the markup file) required");
            }
            let api = host_api.unwrap_or(HTML_API_VERSION);
            if !(HTML_API_VERSION..=HTML_API_LATEST).contains(&api) {
                d.error(f, format!("host_api {api}: the HTML Block host speaks API {HTML_API_VERSION}…{HTML_API_LATEST}"));
            } else if api >= HTML_BIND_API && manifest_version < 5 {
                d.error(f, format!("host_api {api} {V5_HINT}"));
            } else if api >= 3 && manifest_version < 3 {
                d.error(f, format!("host_api {api} {V3_HINT}"));
            } else if api < 3 && !events.is_empty() {
                d.warn(f, "events: an HTML control raises them with aiws.emit, which needs host_api 3");
            }
            html.insert("api_version".into(), json!(api));
            payload.insert("html".into(), Value::Object(html));
        }
        ("declarative", Some(Value::String(file))) => {
            named.insert(file.clone());
            if let Some(t) = text_file(&by_name, file, "entry", &mut d) {
                match serde_json::from_str::<Value>(t) {
                    Ok(spec) => {
                        check_declarative(&spec, &bindings, &events, manifest_version, file, &mut d);
                        payload.insert("declarative".into(), spec);
                    }
                    Err(e) => d.json_error(file, &e),
                }
            }
            if host_api.is_some_and(|a| a != DECLARATIVE_API_VERSION) {
                d.error(f, format!("host_api: the declarative interpreter is version {DECLARATIVE_API_VERSION}"));
            }
        }
        (COMPOSITE, Some(Value::String(file))) => {
            named.insert(file.clone());
            if let Some(t) = text_file(&by_name, file, "entry", &mut d) {
                match serde_json::from_str::<Value>(t) {
                    Ok(spec) => {
                        let projected;
                        (slot_cells, bubbles, projected) = check_composition(&spec, c.get("config_schema"), manifest_version, file, &mut d);
                        // a projection's role raises its event with the item it was pressed on (控件机制 TODO G13)
                        if !projected.is_empty() {
                            let mut evs = Map::new();
                            for (name, projection, role) in &projected {
                                let mut e = json!({ "from": { "projection": projection, "role": role },
                                                    "params": { "item": { "type": "string", "required": true }, "projection": { "type": "string" } } });
                                for k in ["title", "description", "on"] {
                                    if let Some(v) = spec["events"][name].get(k) {
                                        e[k] = v.clone();
                                    }
                                }
                                evs.insert(name.clone(), e);
                            }
                            payload.insert("events".into(), Value::Object(evs));
                        }
                        payload.insert("composition".into(), spec);
                    }
                    Err(e) => d.json_error(file, &e),
                }
            }
            if host_api.is_some() {
                d.error(f, "host_api: a composite runs no code of its own (its layout and chrome are the host's)");
            }
        }
        ("html", _) => d.error(f, "entry must be { html, css?, js? } for an html control"),
        ("declarative", _) => d.error(f, "entry must name the declarative JSON file"),
        (COMPOSITE, _) => d.error(f, format!("entry must name the composition file ({COMPOSITION_FILE})")),
        _ => {}
    }
    for a in c.get("assets").and_then(Value::as_array).into_iter().flatten() {
        match a.as_str() {
            Some(n) if by_name.contains_key(n) => {
                named.insert(n.to_string());
            }
            Some(n) => d.error(f, format!("asset {n:?} is not in the directory")),
            None => d.error(f, "assets lists file names"),
        }
    }
    if c.get("assets").is_some_and(|a| !a.is_array()) {
        d.error(f, "assets must be an array of file names");
    }
    if let Some(a) = c.get("agent") {
        match a.as_str() {
            Some(file) => {
                named.insert(file.to_string());
                if let Some(t) = text_file(&by_name, file, "agent", &mut d) {
                    if t.len() > MAX_AGENT_BYTES {
                        d.error(file, format!("a self-description is at most {MAX_AGENT_BYTES} bytes"));
                    } else {
                        payload.insert("agent".into(), json!(t));
                    }
                }
            }
            None => d.error(f, "agent names a Markdown file"),
        }
    }
    for n in by_name.keys() {
        if !named.contains(*n) {
            d.warn(n, "not named by component.json: kept in the source snapshot only");
        }
    }
    if !is_renderer_id(&name) {
        // reported by ident already; keep composing for further diagnostics
    } else if canonical_json(&Value::Object(payload.clone())).map(|s| s.len()).unwrap_or(usize::MAX) > MAX_BLOCK_DEF_BYTES - 4096 {
        d.error(f, format!("the composed definition exceeds {} bytes (inline markup, script, style and self-description)", MAX_BLOCK_DEF_BYTES - 4096));
    }
    if d.has_errors() {
        return Err(d.0);
    }
    let pairs: Vec<(String, String)> = files.iter().map(|f| (f.name.clone(), f.obj_id.clone())).collect();
    let members: Map<String, Value> = files.iter().map(|f| (f.name.clone(), json!({ "kind": "file", "object_id": f.obj_id }))).collect();
    Ok(Composed {
        ident,
        name,
        kind,
        payload,
        members,
        input_digest: input_digest(&pairs, manifest_version),
        digest: content_digest(files.iter().map(|f| (f.name.as_str(), f.bytes.as_slice()))),
        deps,
        slot_cells,
        bubbles,
        warnings: d.0,
    })
}

/// The controls of one build or import with what each builds on first (Kahn's order; a cycle keeps the given order
/// and is the core's to refuse).
fn dependency_order(mut items: Vec<(Composed, Option<Value>, bool)>) -> Vec<(Composed, Option<Value>, bool)> {
    let mut out = Vec::with_capacity(items.len());
    while !items.is_empty() {
        let pending: BTreeSet<String> = items.iter().map(|(c, ..)| c.ident.clone()).collect();
        let ready = items.iter().position(|(c, ..)| c.deps.iter().all(|d| !pending.contains(d) || *d == c.ident)).unwrap_or(0);
        out.push(items.remove(ready));
    }
    out
}

/// The identities a composition's slots accept as `cell:<ident>`.
pub fn slot_cells_of(composition: &Value) -> Vec<String> {
    let mut d = Diags(Vec::new());
    check_composition(composition, None, MANIFEST_VERSION, "", &mut d).0
}

/// How a rebuild narrows the contract the instances rely on (README “Compatibility with instances”).
pub fn narrowing(old: &Map<String, Value>, new: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    if old.get("kind") != new.get("kind") {
        out.push(format!("kind changes from {} to {}", old.get("kind").unwrap_or(&Value::Null), new.get("kind").unwrap_or(&Value::Null)));
    }
    if let (Some(a), Some(b)) = (old.get("accepts").and_then(Value::as_array), new.get("accepts").and_then(Value::as_array)) {
        for t in a.iter().filter(|t| !b.contains(t)) {
            out.push(format!("accepts no longer lists {t}"));
        }
    }
    let empty = Map::new();
    let ob = old.get("bindings").and_then(Value::as_object).unwrap_or(&empty);
    let nb = new.get("bindings").and_then(Value::as_object).unwrap_or(&empty);
    let required = |b: &Value| b.get("required") == Some(&json!(true));
    let many = |b: &Value| b.get("many") == Some(&json!(true));
    for (name, decl) in ob {
        match nb.get(name) {
            None => out.push(format!("binding {name} is removed")),
            Some(n) if required(n) && !required(decl) => out.push(format!("binding {name} becomes required")),
            Some(n) if many(decl) && !many(n) => out.push(format!("binding {name} no longer takes several")),
            Some(n) if n.get("schema_id").is_some() && n.get("schema_id") != decl.get("schema_id") => {
                out.push(format!("binding {name} now takes only {}", n["schema_id"]))
            }
            Some(_) => {}
        }
    }
    for (name, decl) in nb {
        if !ob.contains_key(name) && required(decl) {
            out.push(format!("binding {name} is new and required"));
        }
    }
    let req = |m: &Map<String, Value>| -> BTreeSet<String> {
        m.get("config_schema").and_then(|s| s.get("required")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
    };
    let (or, nr) = (req(old), req(new));
    for k in nr.difference(&or) {
        out.push(format!("config_schema requires {k}"));
    }
    // an instance binds handlers to events (P2): an event that goes, or a param it no longer carries, leaves them dangling
    let (oe, ne) = (object_of(old, "events"), object_of(new, "events"));
    for (name, decl) in &oe {
        match ne.get(name) {
            None => out.push(format!("event {name} is removed")),
            Some(n) => {
                for p in decl.get("params").and_then(Value::as_object).into_iter().flatten().map(|(p, _)| p) {
                    if !n.get("params").and_then(Value::as_object).is_some_and(|ps| ps.contains_key(p)) {
                        out.push(format!("event {name} no longer carries {p}"));
                    }
                }
            }
        }
    }
    // a composite's slots hold the instances' Blocks (P1): a slot that goes or accepts less strands them
    let slots = |m: &Map<String, Value>| m.get("composition").and_then(|c| c.get("slots")).and_then(Value::as_object).cloned();
    if let (Some(os), Some(ns)) = (slots(old), slots(new)) {
        for (name, slot) in &os {
            let Some(n) = ns.get(name) else {
                out.push(format!("slot {name} is removed"));
                continue;
            };
            match (slot.get("accepts").and_then(Value::as_array), n.get("accepts").and_then(Value::as_array)) {
                (_, None) => {}
                (None, Some(_)) => out.push(format!("slot {name} no longer accepts any Block")),
                (Some(a), Some(b)) => {
                    for t in a.iter().filter(|t| !b.contains(t)) {
                        out.push(format!("slot {name} no longer accepts {t}"));
                    }
                }
            }
        }
    }
    out
}

/// The names of an object key of a payload.
fn object_of(payload: &Map<String, Value>, key: &str) -> Map<String, Value> {
    payload.get(key).and_then(Value::as_object).cloned().unwrap_or_default()
}

/// `{ rename?, drop?, fill? }` of a migration applied to one object key: the new object and the properties that changed
/// (`null` = removed), as `entity.set_props` writes them.
fn migrate_object(current: &Map<String, Value>, m: Option<&Value>) -> (Map<String, Value>, Map<String, Value>) {
    let mut next = current.clone();
    let Some(m) = m else { return (next, Map::new()) };
    for (old, new) in m.get("rename").and_then(Value::as_object).into_iter().flatten() {
        if let (Some(v), Some(new)) = (next.remove(old), new.as_str()) {
            next.insert(new.to_string(), v);
        }
    }
    for name in m.get("drop").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        next.remove(name);
    }
    for (name, v) in m.get("fill").and_then(Value::as_object).into_iter().flatten() {
        next.entry(name.clone()).or_insert_with(|| v.clone());
    }
    let mut changed = Map::new();
    for k in current.keys().chain(next.keys()) {
        if current.get(k) != next.get(k) {
            changed.insert(k.clone(), next.get(k).cloned().unwrap_or(Value::Null));
        }
    }
    (next, changed)
}

/// The `{{event.<p>}}` placeholders of a handler (what it reads from the event).
fn event_params_used(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::String(t) => {
            let mut rest = t.as_str();
            while let Some(i) = rest.find("{{event.") {
                let tail = &rest[i + 8..];
                let end = tail.find("}}").unwrap_or(tail.len());
                out.insert(tail[..end].trim().to_string());
                rest = &tail[end..];
            }
        }
        Value::Array(a) => a.iter().for_each(|x| event_params_used(x, out)),
        Value::Object(o) => o.values().for_each(|x| event_params_used(x, out)),
        _ => {}
    }
}

/// Does a Block fit one `accepts` entry of a slot (`cell:<ident>`, a view type, `container`)?
fn child_fits(ctx: &dyn ReadCtx, child: &EntityRow, accepts: &[Value]) -> WsResult<bool> {
    for a in accepts.iter().filter_map(Value::as_str) {
        let ok = match a.strip_prefix("cell:") {
            Some(ident) => match child.payload.get("def_ref").and_then(|r| r.get("entity_id")).and_then(Value::as_str) {
                Some(d) => ctx.entity(d)?.is_some_and(|d| d.name.as_deref() == Some(ident)),
                None => false,
            },
            None if a == "container" => child.type_id == TYPE_CONTAINER,
            None => child.payload.get("view").and_then(|v| v.get("type")).and_then(Value::as_str) == Some(a),
        };
        if ok {
            return Ok(true);
        }
    }
    Ok(false)
}

/// What keeps instance `inst` (its `config`, `bindings`, `on` and, for a composite, the slots of its children after a
/// migration) from fitting the contract `new` of its definition (README “Compatibility per instance”).
fn instance_issues(ctx: &dyn ReadCtx, old: &Map<String, Value>, new: &Map<String, Value>, inst: &EntityRow, state: &Map<String, Value>, slots: &[(EntityRow, Option<String>)]) -> WsResult<Vec<String>> {
    let mut out = Vec::new();
    if old.get("kind") != new.get("kind") {
        out.push(format!("kind changes from {} to {}", old.get("kind").unwrap_or(&Value::Null), new.get("kind").unwrap_or(&Value::Null)));
    }
    let decl = object_of(new, "bindings");
    let bound = object_of(state, "bindings");
    for name in bound.keys().filter(|n| !decl.contains_key(*n)) {
        out.push(format!("binds {name}, which the control no longer declares"));
    }
    for (name, d) in &decl {
        if d.get("required") == Some(&json!(true)) && !bound.contains_key(name) {
            out.push(format!("leaves the required binding {name} unbound"));
        }
        if d.get("many") != Some(&json!(true)) && bound.get(name).is_some_and(Value::is_array) {
            out.push(format!("binds several to {name}, which takes one"));
        }
    }
    if let (Some(accepts), Some(src)) = (new.get("accepts").and_then(Value::as_array), inst.payload.get("source_ref").and_then(|r| r.get("entity_id")).and_then(Value::as_str)) {
        if let Some(t) = ctx.entity(src)? {
            if !accepts.iter().any(|a| a.as_str() == Some(t.type_id.as_str())) {
                out.push(format!("shows a {} the control no longer accepts", t.type_id));
            }
        }
    }
    let config = object_of(state, "config");
    let schema = new.get("config_schema");
    for name in schema.and_then(|s| s.get("required")).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        let has_default = schema.and_then(|s| s.get("properties")).and_then(|p| p.get(name)).is_some_and(|p| p.get("default").is_some());
        if !config.contains_key(name) && !has_default {
            out.push(format!("has no value for the required property {name}"));
        }
    }
    let events = object_of(new, "events");
    for (name, handlers) in object_of(state, "on") {
        match events.get(&name) {
            None => out.push(format!("binds the event {name}, which the control no longer raises")),
            Some(ev) => {
                let mut used = BTreeSet::new();
                event_params_used(&handlers, &mut used);
                let params = ev.get("params").and_then(Value::as_object);
                for p in used.iter().filter(|p| !params.is_some_and(|ps| ps.contains_key(*p))) {
                    out.push(format!("reads {{{{event.{p}}}}} of {name}, which the event no longer carries"));
                }
            }
        }
    }
    if let Some(ns) = new.get("composition").and_then(|c| c.get("slots")).and_then(Value::as_object) {
        for (child, slot) in slots {
            let Some(slot) = slot else { continue };
            match ns.get(slot) {
                None => out.push(format!("holds {} in the slot {slot}, which the control no longer has", child.entity_id)),
                Some(def) => {
                    if let Some(accepts) = def.get("accepts").and_then(Value::as_array) {
                        if !child_fits(ctx, child, accepts)? {
                            out.push(format!("holds {} in the slot {slot}, which no longer accepts it", child.entity_id));
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Keys a build owns on a definition (everything else of an older payload is removed by a rebuild).
const BUILD_OWNED_EXTRA: &[&str] = &["version", "source_dir", "provenance"];

impl Workspace {
    /// Stage the files of a build and read their bytes: objects the caller may read (the server judged that at the
    /// NDM gateway) become staged assets of this Workspace, inline bytes are stored first. `dry_run` stages nothing.
    pub fn ext_stage_files(&mut self, caller: &Caller, files: Vec<IncomingFile>, max_bytes: u64, dry_run: bool) -> WsResult<Vec<SourceFile>> {
        if files.len() > MAX_SOURCE_FILES {
            return Err(build_failed(vec![json!({ "file": "", "severity": "error", "message": format!("a source directory holds at most {MAX_SOURCE_FILES} files") })]));
        }
        let mut out = Vec::new();
        for f in files {
            match f {
                IncomingFile::Object { name, obj_id } => {
                    if !obj_id.starts_with("cyfile:") {
                        return Err(WsError::invalid_op(format!("{name}: {obj_id} is not a FileObject")));
                    }
                    if self.objects.file_size(&obj_id)? > max_bytes {
                        return Err(WsError::limit(format!("{name}: a source file is limited to {max_bytes} bytes")));
                    }
                    if !dry_run {
                        self.stage_object(caller, &obj_id, max_bytes)?;
                    }
                    let bytes = self.objects.get_file(&obj_id)?;
                    out.push(SourceFile { name, obj_id, bytes });
                }
                IncomingFile::Inline { name, bytes } => {
                    let obj_id = if dry_run {
                        file_object_id(&bytes)?
                    } else {
                        self.stage_asset(caller, &bytes)?["object_id"].as_str().map(str::to_string).ok_or_else(|| WsError::io("staging gave no object id"))?
                    };
                    out.push(SourceFile { name, obj_id, bytes });
                }
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// The folder builds go to: `folder_id`, or `data/extensions` (created by the build when missing — the op is
    /// returned to go into the same Commit).
    fn ext_folder(&self, now: &str, folder_id: Option<&str>) -> WsResult<(String, Option<Value>)> {
        let ctx = self.ctx(now);
        if let Some(id) = folder_id {
            let e = ctx.entity(id)?.filter(|e| e.alive()).ok_or_else(|| WsError::not_found(format!("folder {id} not found")))?;
            if e.type_id != TYPE_CONTAINER || e.payload.get("kind").and_then(Value::as_str) != Some("folder") {
                return Err(WsError::invalid_op(format!("{id} is not a folder of the data tree")));
            }
            return Ok((id.to_string(), None));
        }
        let children = ctx.children(DATA_ID)?;
        for edge in &children {
            if let Some(e) = ctx.entity(&edge.child_id)?.filter(|e| e.alive()) {
                if e.name.as_deref() == Some(EXTENSIONS_FOLDER) && e.type_id == TYPE_CONTAINER {
                    return Ok((e.entity_id, None));
                }
            }
        }
        let id = random_id("ext");
        let key = order_key_between(children.last().map(|e| e.order_key.as_str()), None)?;
        Ok((id.clone(), Some(json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": DATA_ID, "order_key": key,
                                     "name": EXTENSIONS_FOLDER, "payload": { "kind": "folder", "title": "Extensions" } }))))
    }

    /// The live definition of `ident` in `folder`.
    fn ext_existing(&self, now: &str, folder: &str, ident: &str) -> WsResult<Option<EntityRow>> {
        let ctx = self.ctx(now);
        for edge in ctx.children(folder)? {
            if let Some(e) = ctx.entity(&edge.child_id)?.filter(|e| e.alive()) {
                if e.type_id == TYPE_BLOCK_DEF && e.name.as_deref() == Some(ident) {
                    return Ok(Some(e));
                }
            }
        }
        Ok(None)
    }

    /// The definitions that build on `def_id` (`def_dep`): `(identity, entity id)`.
    pub fn ext_dependents(&self, def_id: &str) -> WsResult<Vec<(String, String)>> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for r in ctx.refs_to(def_id)? {
            if r.kind == "def_dep" {
                if let Some(d) = ctx.entity(&r.src_entity_id)?.filter(|d| d.alive()) {
                    out.push((d.name.clone().unwrap_or_else(|| d.entity_id.clone()), d.entity_id));
                }
            }
        }
        out.sort();
        out.dedup();
        Ok(out)
    }

    /// A live control definition of this Workspace.
    pub fn ext_definition(&self, id: &str) -> WsResult<EntityRow> {
        let now = self.now();
        self.ctx(&now).entity(id)?.filter(|d| d.alive() && d.type_id == TYPE_BLOCK_DEF).ok_or_else(|| WsError::not_found(format!("definition {id} not found")))
    }

    /// Cells and composite containers that use a definition (its instances).
    pub fn ext_instances(&self, def_id: &str) -> WsResult<Vec<Value>> {
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut out = Vec::new();
        for r in ctx.refs_to(def_id)? {
            if r.kind != "def" {
                continue;
            }
            if let Some(c) = ctx.entity(&r.src_entity_id)?.filter(|c| c.alive() && (c.type_id == TYPE_CELL || c.type_id == TYPE_CONTAINER)) {
                let parent = ctx.edge(&c.entity_id)?.map(|e| e.parent_id);
                out.push(json!({ "entity_id": c.entity_id, "title": c.payload.get("title"), "parent_id": parent }));
            }
        }
        Ok(out)
    }

    /// The instances of `def` under its new contract `new` (P2, Q30): the migration's writes — one `entity.set_props` per
    /// key (`config`, `bindings`, `on` ← `events`), one `tree.move { slot }` per child of a renamed or moved slot — and,
    /// for each instance that still does not fit, an error diagnostic naming it. Answers `(ops, plan, diagnostics)`.
    fn ext_fit(&self, now: &str, def: &EntityRow, new: &Map<String, Value>, migration: Option<&Value>) -> WsResult<(Vec<Value>, Vec<Value>, Vec<Value>)> {
        let ctx = self.ctx(now);
        let (mut ops, mut plan, mut diags) = (Vec::new(), Vec::new(), Vec::new());
        let mut seen = BTreeSet::new();
        let moves: Map<String, Value> = ["rename", "move"].iter().filter_map(|k| migration.and_then(|m| m.get("slots")).and_then(|s| s.get(*k)).and_then(Value::as_object)).flat_map(|o| o.clone()).collect();
        for r in ctx.refs_to(&def.entity_id)? {
            if r.kind != "def" || !seen.insert(r.src_entity_id.clone()) {
                continue;
            }
            let Some(inst) = ctx.entity(&r.src_entity_id)?.filter(|c| c.alive() && (c.type_id == TYPE_CELL || c.type_id == TYPE_CONTAINER)) else { continue };
            let mut state = inst.payload.clone();
            let mut inst_ops = Vec::new();
            for (key, part) in [("config", "config"), ("bindings", "bindings"), ("on", "events")] {
                let (next, changed) = migrate_object(&object_of(&inst.payload, key), migration.and_then(|m| m.get(part)));
                if changed.is_empty() {
                    continue;
                }
                inst_ops.push(json!({ "op": "entity.set_props", "entity_id": inst.entity_id, "key": key, "props": changed }));
                if next.is_empty() {
                    state.remove(key);
                } else {
                    state.insert(key.to_string(), Value::Object(next));
                }
            }
            let mut slots = Vec::new();
            if inst.type_id == TYPE_CONTAINER {
                for edge in ctx.children(&inst.entity_id)? {
                    let Some(child) = ctx.entity(&edge.child_id)?.filter(|c| c.alive()) else { continue };
                    let slot = child.payload.get("slot").and_then(Value::as_str).map(str::to_string);
                    let target = slot.as_ref().and_then(|s| moves.get(s)).and_then(Value::as_str).map(str::to_string);
                    if let Some(t) = &target {
                        let mut mv = json!({ "op": "tree.move", "entity_id": child.entity_id, "new_parent_id": inst.entity_id, "order_key": edge.order_key, "slot": t });
                        if let Some(pl) = &edge.placement {
                            mv["placement"] = pl.clone();
                        }
                        inst_ops.push(mv);
                    }
                    slots.push((child, target.or(slot)));
                }
            }
            let issues = instance_issues(&ctx, &def.payload, new, &inst, &state, &slots)?;
            if !issues.is_empty() {
                let parent = ctx.edge(&inst.entity_id)?.map(|e| e.parent_id);
                diags.push(json!({ "file": COMPONENT_FILE, "severity": "error", "message": format!("instance {}: {}", inst.entity_id, issues.join("; ")),
                                   "instances": [{ "entity_id": inst.entity_id, "title": inst.payload.get("title"), "parent_id": parent }] }));
            }
            if !inst_ops.is_empty() {
                plan.push(json!({ "instance": inst.entity_id, "ops": inst_ops }));
                ops.extend(inst_ops);
            }
        }
        Ok((ops, plan, diags))
    }

    /// The content digest of a definition's source snapshot (what `provenance.digest` records for an import), with
    /// its members `name → object id`.
    fn ext_snapshot(&self, dir: &EntityRow) -> WsResult<(String, BTreeMap<String, String>)> {
        let mut files = Vec::new();
        let mut ids = BTreeMap::new();
        for (name, m) in dir.payload.get("members").and_then(Value::as_object).into_iter().flatten() {
            let Some(oid) = m.get("object_id").and_then(Value::as_str) else { continue };
            files.push((name.clone(), self.objects.get_file(oid)?));
            ids.insert(name.clone(), oid.to_string());
        }
        Ok((content_digest(files.iter().map(|(n, b)| (n.as_str(), b.as_slice()))), ids))
    }

    /// `ext.build` / `ext.import` after staging: compose every control, plan one Commit, commit it (README
    /// “Extension controls”). One input is a build, several an import.
    pub fn ext_build(&mut self, caller: &Caller, inputs: Vec<BuildInput>, opts: &BuildOpts) -> WsResult<Value> {
        // rights are the Commit's (structure on the folder, update on the definition and its directory)
        self.require_ws_any(caller)?;
        let mut diagnostics = Vec::new();
        let mut composed = Vec::new();
        for input in &inputs {
            match compose(&input.files) {
                Ok(c) => composed.push((c, input.provenance.clone(), input.implicit)),
                Err(d) => diagnostics.extend(d),
            }
        }
        if diagnostics.iter().any(|d| d["severity"] == "error") {
            return Err(build_failed(diagnostics));
        }
        let mut seen = BTreeSet::new();
        for (c, ..) in &composed {
            if !seen.insert(c.ident.clone()) {
                return Err(WsError::invalid_op(format!("{} is listed twice", c.ident)));
            }
        }
        // what a control builds on goes first (an import brings a package's dependencies in the same Commit)
        let composed = dependency_order(composed);
        let now = self.now();
        let (folder, folder_op) = self.ext_folder(&now, opts.folder_id.as_deref())?;
        // every identity this Commit leaves in the folder → (definition, kind): what deps and slots resolve against
        let mut known: BTreeMap<String, (String, String)> = BTreeMap::new();
        {
            let ctx = self.ctx(&now);
            for edge in ctx.children(&folder).unwrap_or_default() {
                if let Some(e) = ctx.entity(&edge.child_id)?.filter(|e| e.alive() && e.type_id == TYPE_BLOCK_DEF) {
                    if let Some(n) = e.name.clone() {
                        known.insert(n, (e.entity_id.clone(), e.payload.get("kind").and_then(Value::as_str).unwrap_or("").to_string()));
                    }
                }
            }
        }
        let mut ops: Vec<Value> = folder_op.into_iter().collect();
        let mut preconditions = Vec::new();
        let mut controls = Vec::new();
        let mut last_key: Option<String> = {
            let ctx = self.ctx(&now);
            ctx.children(&folder).ok().and_then(|c| c.last().map(|e| e.order_key.clone()))
        };
        let mut next_key = || -> WsResult<String> {
            let k = order_key_between(last_key.as_deref(), None)?;
            last_key = Some(k.clone());
            Ok(k)
        };
        let built_at = now.clone();
        let mut failed = Diags(Vec::new());
        // the events each identity declares, for composites re-raising them (this Commit's builds first)
        let mut batch_events: BTreeMap<String, Map<String, Value>> = BTreeMap::new();
        let mut plan = Vec::new();
        let mut upgraded: Vec<String> = Vec::new();
        for (c, provenance, implicit) in composed {
            let existing = self.ext_existing(&now, &folder, &c.ident)?;
            let record = json!({ "build": { "input_digest": c.input_digest, "builder_version": BUILDER_VERSION, "built_at": built_at } });
            let mut payload = c.payload.clone();
            if let Some(p) = &provenance {
                payload.insert("provenance".into(), p.clone());
            }
            // deps → the definitions of this folder (Q16); `cell:<ident>` slots name render controls known here
            let mut deps = Vec::new();
            for dep in &c.deps {
                match known.get(dep) {
                    Some((id, _)) => deps.push(json!({ "ident": dep, "entity_id": id })),
                    None => failed.missing(COMPONENT_FILE, format!("deps: {dep} is not in this Workspace (build or import it first)")),
                }
            }
            for cell in &c.slot_cells {
                match known.get(cell) {
                    None if c.deps.contains(cell) => {}
                    None => failed.missing(COMPOSITION_FILE, format!("a slot accepts cell:{cell}, which is neither in deps nor in this Workspace")),
                    Some((_, kind)) if kind == COMPOSITE => failed.error(COMPOSITION_FILE, format!("a slot accepts cell:{cell}, but {cell} is a composite (a container, not a Cell)")),
                    Some(_) => {}
                }
            }
            if !deps.is_empty() {
                payload.insert("deps".into(), Value::Array(deps));
            }
            // a composite's events re-raise those of the controls its slots accept (Q26): the child event's params
            if !c.bubbles.is_empty() {
                let comp = payload.get("composition").cloned().unwrap_or(Value::Null);
                // the events a projection raises were written when the composition was read
                let mut evs = payload.get("events").and_then(Value::as_object).cloned().unwrap_or_default();
                for (name, slot, child_event) in &c.bubbles {
                    let accepted: Vec<String> = comp["slots"][slot]["accepts"].as_array().into_iter().flatten().filter_map(Value::as_str).filter_map(|a| a.strip_prefix("cell:")).map(str::to_string).collect();
                    let mut found = None;
                    for ident in &accepted {
                        let declared = match batch_events.get(ident) {
                            Some(e) => Some(e.clone()),
                            None => match known.get(ident) {
                                Some((id, _)) => self.ctx(&now).entity(id)?.map(|d| object_of(&d.payload, "events")),
                                None => None,
                            },
                        };
                        if let Some(decl) = declared.and_then(|e| e.get(child_event).cloned()) {
                            found = Some(decl);
                            break;
                        }
                    }
                    let Some(decl) = found else {
                        failed.0.push(json!({ "file": COMPOSITION_FILE, "severity": "error", "code": "EVENT_UNDECLARED",
                                              "message": format!("events.{name}: no control the slot {slot} accepts (cell:<ident>) raises {child_event}") }));
                        continue;
                    };
                    let mut e = json!({ "from": { "slot": slot, "event": child_event } });
                    for k in ["title", "description"] {
                        if let Some(v) = comp["events"][name].get(k).or_else(|| decl.get(k)) {
                            e[k] = v.clone();
                        }
                    }
                    if let Some(ps) = decl.get("params") {
                        e["params"] = ps.clone();
                    }
                    evs.insert(name.clone(), e);
                }
                payload.insert("events".into(), Value::Object(evs));
            }
            batch_events.insert(c.ident.clone(), object_of(&payload, "events"));
            let Some(def) = existing else {
                // a new identity: its directory, its definition, its record
                let (dir_id, def_id) = (random_id("src"), random_id("def"));
                ops.push(json!({ "op": "entity.create", "entity_id": dir_id, "type_id": TYPE_DIR, "parent_id": folder, "order_key": next_key()?, "payload": { "members": c.members } }));
                payload.insert("version".into(), json!(1));
                payload.insert("source_dir".into(), json!({ "entity_id": dir_id }));
                ops.push(json!({ "op": "entity.create", "entity_id": def_id, "type_id": TYPE_BLOCK_DEF, "parent_id": folder, "order_key": next_key()?, "name": c.ident, "payload": payload }));
                ops.push(json!({ "op": "entity.set_derived", "entity_id": def_id, "derived": record }));
                known.insert(c.ident.clone(), (def_id.clone(), c.kind.clone()));
                controls.push(json!({ "ident": c.ident, "def_entity_id": def_id, "dir_entity_id": dir_id, "action": "created", "version": 1, "build": record["build"], "warnings": c.warnings }));
                continue;
            };
            let dir = def.payload.get("source_dir").and_then(|d| d.get("entity_id")).and_then(Value::as_str).map(str::to_string);
            let dir_row = match &dir {
                Some(id) => self.ctx(&now).entity(id)?.filter(|e| e.alive() && e.type_id == TYPE_DIR),
                None => None,
            };
            let current_digest = def.derived.as_ref().and_then(|d| d.get("build")).and_then(|b| b.get("input_digest")).and_then(Value::as_str);
            let old_prov = def.payload.get("provenance").cloned().unwrap_or(Value::Null);
            let same_source = provenance.as_ref().is_some_and(|p| p.get("pkg_id") == old_prov.get("pkg_id") && p.get("digest") == old_prov.get("digest"));
            if opts.import && !opts.upgrade {
                // a dependency brought along is met by whatever definition of that identity is here
                if !same_source && !implicit {
                    return Err(WsError::sub(Code::InvalidOperation, "EXT_IDENT_EXISTS", format!("{} is already in this Workspace from another source; upgrade it with ext.upgrade", c.ident))
                        .with_data(json!({ "ident": c.ident, "def_entity_id": def.entity_id, "provenance": old_prov,
                                           "library_version": provenance.as_ref().and_then(|p| p.get("version")) })));
                }
                controls.push(json!({ "ident": c.ident, "def_entity_id": def.entity_id, "dir_entity_id": dir, "action": "satisfied" }));
                continue;
            }
            if opts.upgrade {
                // the same package content, or a dependency held from another source: left as it is
                if same_source || (implicit && provenance.as_ref().and_then(|p| p.get("pkg_id")) != old_prov.get("pkg_id")) {
                    controls.push(json!({ "ident": c.ident, "def_entity_id": def.entity_id, "dir_entity_id": dir, "action": if same_source { "unchanged" } else { "satisfied" },
                                          "version": def.payload.get("version") }));
                    continue;
                }
                // local changes since the import are never overwritten silently
                if !opts.force {
                    let (snapshot_digest, old_files) = match &dir_row {
                        Some(d) => self.ext_snapshot(d)?,
                        None => (String::new(), BTreeMap::new()),
                    };
                    if old_prov.get("digest").and_then(Value::as_str) != Some(snapshot_digest.as_str()) {
                        let new_files: BTreeMap<&String, &str> = c.members.iter().filter_map(|(n, m)| m.get("object_id").and_then(Value::as_str).map(|o| (n, o))).collect();
                        let added: Vec<&String> = new_files.keys().filter(|n| !old_files.contains_key(**n)).copied().collect();
                        let removed: Vec<&String> = old_files.keys().filter(|n| !new_files.contains_key(n)).collect();
                        let changed: Vec<&String> = old_files.iter().filter(|(n, o)| new_files.get(n).is_some_and(|x| x != o)).map(|(n, _)| n).collect();
                        return Err(WsError::sub(Code::InvalidOperation, "EXT_LOCAL_CHANGES", format!("{} was changed here since it was imported; upgrading would replace those changes (force to replace them)", c.ident))
                            .with_data(json!({ "ident": c.ident, "def_entity_id": def.entity_id, "imported_digest": old_prov.get("digest"), "snapshot_digest": snapshot_digest,
                                               "files": { "added": added, "removed": removed, "changed": changed } })));
                    }
                }
                upgraded.push(format!("{} {}→{}", c.ident, old_prov.get("version").and_then(Value::as_str).unwrap_or("local"),
                                      provenance.as_ref().and_then(|p| p.get("version")).and_then(Value::as_str).unwrap_or("?")));
            }
            let dir_rev = dir_row.as_ref().map_or(0, |d| d.content_rev);
            match (opts.expect, opts.force || opts.upgrade) {
                (_, true) => {}
                (Some((def_rev, want_dir)), false) => {
                    if def_rev != def.content_rev || want_dir != dir_rev {
                        return Err(WsError::sub(Code::RevisionConflict, "BUILD_BASE_STALE", format!("{} was built since your base (definition rev {def_rev}, now {}): pull, merge, build again", c.ident, def.content_rev))
                            .with_data(json!({ "def_entity_id": def.entity_id, "def_rev": def.content_rev, "dir_rev": dir_rev })));
                    }
                    preconditions.push(json!({ "target": { "entity_id": def.entity_id }, "expect": { "rev": def.content_rev } }));
                    if let Some(d) = &dir_row {
                        preconditions.push(json!({ "target": { "entity_id": d.entity_id }, "expect": { "rev": d.content_rev } }));
                    }
                }
                (None, false) => {
                    return Err(WsError::sub(Code::RevisionConflict, "BUILD_BASE_REQUIRED", format!("{} already exists here: pull it first (or force)", c.ident))
                        .with_data(json!({ "def_entity_id": def.entity_id, "def_rev": def.content_rev, "dir_rev": dir_rev })));
                }
            }
            if current_digest == Some(c.input_digest.as_str()) && dir_row.is_some() && !opts.force && !(opts.upgrade && !same_source) {
                controls.push(json!({ "ident": c.ident, "def_entity_id": def.entity_id, "dir_entity_id": dir, "action": "unchanged",
                                      "def_rev": def.content_rev, "dir_rev": dir_rev, "version": def.payload.get("version"), "build": record["build"], "warnings": c.warnings }));
                continue;
            }
            // the contract instances and dependent definitions rely on (Q16: dependents are judged with it); instances
            // are judged one by one after the migration (P2)
            let narrowed = narrowing(&def.payload, &payload);
            let dependents = self.ext_dependents(&def.entity_id)?;
            for m in &narrowed {
                for (ident, id) in &dependents {
                    failed.0.push(json!({ "file": ident, "severity": "error", "message": format!("{m}: {ident} builds on the current contract"), "dependents": [{ "ident": ident, "def_entity_id": id }] }));
                }
            }
            let migration = opts.migration.get(&c.ident).or_else(|| opts.migration.get("*"));
            let (fit_ops, fit_plan, fit_diags) = self.ext_fit(&now, &def, &payload, migration)?;
            failed.0.extend(fit_diags);
            let migrated: Vec<Value> = fit_plan.iter().map(|p| p["instance"].clone()).collect();
            plan.extend(fit_plan);
            if c.kind == COMPOSITE {
                for (ident, id) in &dependents {
                    let cells = self.ctx(&now).entity(id)?.and_then(|d| d.payload.get("composition").cloned()).map(|comp| slot_cells_of(&comp)).unwrap_or_default();
                    if cells.contains(&c.ident) {
                        failed.0.push(json!({ "file": ident, "severity": "error", "message": format!("{ident} slots accept cell:{}, which a composite cannot be", c.ident), "dependents": [{ "ident": ident, "def_entity_id": id }] }));
                    }
                }
            }
            // the snapshot: the same directory switched to the new members, or a first one for an inline definition
            let dir_id = match &dir_row {
                Some(d) => {
                    ops.push(json!({ "op": "dir.replace", "entity_id": d.entity_id, "members": c.members, "expect": { "rev": d.content_rev } }));
                    d.entity_id.clone()
                }
                None => {
                    let id = random_id("src");
                    ops.push(json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_DIR, "parent_id": folder, "order_key": next_key()?, "payload": { "members": c.members } }));
                    id
                }
            };
            let version = def.payload.get("version").and_then(Value::as_u64).unwrap_or(1) + 1;
            payload.insert("version".into(), json!(version));
            payload.insert("source_dir".into(), json!({ "entity_id": dir_id }));
            let mut set = Vec::new();
            for (k, v) in &payload {
                if def.payload.get(k) != Some(v) {
                    set.push(json!({ "key": k, "value": v, "expect": { "rev": def.key_rev(k) } }));
                }
            }
            let mut unset = Vec::new();
            for k in def.payload.keys() {
                // what a build owns stays unless it is replaced: `provenance` keeps tracing an import rebuilt locally
                if !payload.contains_key(k) && !BUILD_OWNED_EXTRA.contains(&k.as_str()) {
                    unset.push(json!({ "key": k, "expect": { "rev": def.key_rev(k) } }));
                }
            }
            if !unset.is_empty() {
                ops.push(json!({ "op": "entity.unset_keys", "entity_id": def.entity_id, "keys": unset }));
            }
            if !set.is_empty() {
                ops.push(json!({ "op": "entity.set_keys", "entity_id": def.entity_id, "keys": set }));
            }
            ops.push(json!({ "op": "entity.set_derived", "entity_id": def.entity_id, "derived": record }));
            // the instances follow the definition they now fit (its slots and keys are in place before them)
            ops.extend(fit_ops);
            known.insert(c.ident.clone(), (def.entity_id.clone(), c.kind.clone()));
            let mut done = json!({ "ident": c.ident, "def_entity_id": def.entity_id, "dir_entity_id": dir_id, "action": "rebuilt", "version": version, "build": record["build"], "warnings": c.warnings });
            if !migrated.is_empty() {
                done["migrated"] = json!(migrated);
            }
            if opts.upgrade {
                done["action"] = json!("upgraded");
                done["from"] = old_prov.get("version").cloned().unwrap_or(Value::Null);
                done["to"] = provenance.as_ref().and_then(|p| p.get("version")).cloned().unwrap_or(Value::Null);
            }
            controls.push(done);
        }
        if failed.has_errors() {
            return Err(build_failed(failed.0));
        }
        let warnings: Vec<Value> = controls.iter().flat_map(|c| c["warnings"].as_array().cloned().unwrap_or_default()).collect();
        if ops.is_empty() || opts.dry_run {
            let changed = controls.iter().any(|c| c["action"] != "unchanged" && c["action"] != "satisfied");
            let mut out = json!({ "ok": true, "status": if opts.dry_run { "checked" } else { "unchanged" }, "unchanged": !changed && !opts.dry_run, "controls": controls, "diagnostics": warnings });
            if opts.dry_run {
                out["would_commit"] = json!(ops.len());
                out["plan"] = json!(plan);
            }
            return Ok(self.ext_single(out, &inputs));
        }
        let message = if upgraded.is_empty() { format!("build {}", controls.iter().filter_map(|c| c["ident"].as_str()).collect::<Vec<_>>().join(", ")) } else { format!("upgrade {}", upgraded.join(", ")) };
        let key = opts.idempotency_key.clone().unwrap_or_else(|| random_id("ext-build/"));
        let req = json!({ "protocol_version": PROTOCOL_VERSION, "workspace_id": self.workspace_id, "epoch": self.epoch, "idempotency_key": key,
                          "message": message.chars().take(1000).collect::<String>(), "preconditions": preconditions, "operations": ops });
        let outcome = self.commit(&req, caller, &CommitOpts { as_build: true, ..Default::default() });
        if outcome["status"] != "accepted" {
            return Err(commit_error(&outcome));
        }
        let seq = outcome["seq"].as_u64().unwrap_or(self.head_seq);
        for c in controls.iter_mut() {
            if matches!(c["action"].as_str(), Some("created" | "rebuilt" | "upgraded")) {
                c["def_rev"] = json!(seq);
                c["dir_rev"] = json!(seq);
            }
            if let Some(o) = c.as_object_mut() {
                o.remove("warnings");
            }
        }
        let out = json!({ "ok": true, "status": "accepted", "commit_id": outcome["commit_id"], "seq": seq, "folder_id": folder, "controls": controls, "diagnostics": warnings });
        Ok(self.ext_single(out, &inputs))
    }

    /// A single build answers its control's fields at the top level too.
    fn ext_single(&self, mut out: Value, inputs: &[BuildInput]) -> Value {
        if inputs.len() == 1 {
            if let Some(c) = out["controls"].get(0).cloned() {
                for k in ["ident", "def_entity_id", "dir_entity_id", "version", "def_rev", "dir_rev", "build", "action", "migrated", "from", "to"] {
                    if let Some(v) = c.get(k) {
                        out[k] = v.clone();
                    }
                }
                if c["action"] == "unchanged" {
                    out["unchanged"] = json!(true);
                }
            }
        }
        out
    }

    /// `ext.export { entity_ids, purpose }`: the bundle of the chosen definitions (README “Extension controls”).
    pub fn ext_export(&self, caller: &Caller, entity_ids: &[String], purpose: &str) -> WsResult<Value> {
        let access = self.require_ws_any(caller)?;
        let cap = match purpose {
            "package" => Cap::Export,
            "pull" => Cap::Read,
            p => return Err(WsError::invalid_op(format!("purpose {p:?}: package or pull"))),
        };
        if entity_ids.is_empty() || entity_ids.len() > 256 {
            return Err(WsError::invalid_op("entity_ids: 1…256 definitions or instances"));
        }
        let now = self.now();
        let ctx = self.ctx(&now);
        let mut defs: Vec<EntityRow> = Vec::new();
        for id in entity_ids {
            let e = ctx.entity(id)?.filter(|e| e.alive()).ok_or_else(|| WsError::not_found(format!("{id} not found")))?;
            if !access.can(&ctx, id, Cap::Read)? {
                return Err(WsError::not_found(format!("{id} not found")));
            }
            let def = match e.type_id.as_str() {
                TYPE_BLOCK_DEF => e,
                TYPE_CELL | TYPE_CONTAINER => {
                    let d = e.payload.get("def_ref").and_then(|r| r.get("entity_id")).and_then(Value::as_str).ok_or_else(|| WsError::invalid_op(format!("{id} is not an instance of a control")))?;
                    ctx.entity(d)?.filter(|d| d.alive() && d.type_id == TYPE_BLOCK_DEF).ok_or_else(|| WsError::not_found(format!("the definition of {id} is gone")))?
                }
                _ => return Err(WsError::invalid_op(format!("{id} is neither a control definition nor an instance"))),
            };
            if !access.can(&ctx, &def.entity_id, cap)? {
                return Err(WsError::denied(format!("{} capability required on {}", aiworkspace_core::access::cap_name(cap), def.entity_id)));
            }
            if !defs.iter().any(|d| d.entity_id == def.entity_id) {
                defs.push(def);
            }
        }
        // a package carries what its controls build on (def_dep, Q16), once; a pull is about the chosen ones only
        if purpose == "package" {
            let mut i = 0;
            while i < defs.len() {
                let deps: Vec<String> = defs[i].payload.get("deps").and_then(Value::as_array).into_iter().flatten().filter_map(|d| d.get("entity_id").and_then(Value::as_str).map(str::to_string)).collect();
                for id in deps {
                    if defs.iter().any(|d| d.entity_id == id) {
                        continue;
                    }
                    let dep = ctx.entity(&id)?.filter(|d| d.alive() && d.type_id == TYPE_BLOCK_DEF).ok_or_else(|| WsError::new(Code::ReferenceBroken, format!("{} depends on {id}, which is gone", defs[i].entity_id)))?;
                    if !access.can(&ctx, &id, cap)? {
                        return Err(WsError::denied(format!("{} capability required on {id} (a dependency of {})", aiworkspace_core::access::cap_name(cap), defs[i].entity_id)));
                    }
                    defs.push(dep);
                }
                i += 1;
            }
        }
        let mut controls = Vec::new();
        let mut files_out = Vec::new();
        let mut manifest_controls = Vec::new();
        let mut dirs_used = BTreeSet::new();
        let (mut html_api, mut decl_api) = (0u64, 0u64);
        for def in defs {
            let name = def.payload.get("def_id").and_then(Value::as_str).unwrap_or("control").to_string();
            let ident = match def.payload.get("ident").map(ident_string) {
                Some(Ok(s)) => s,
                _ => format!("did:aiws:{}#{name}", self.workspace_id),
            };
            let mut dir_name = format!("controls/{name}/");
            let mut n = 2;
            while !dirs_used.insert(dir_name.clone()) {
                dir_name = format!("controls/{name}-{n}/");
                n += 1;
            }
            let dir = def.payload.get("source_dir").and_then(|d| d.get("entity_id")).and_then(Value::as_str).and_then(|id| ctx.entity(id).ok().flatten()).filter(|d| d.alive() && d.type_id == TYPE_DIR);
            let mut files: Vec<Value> = Vec::new();
            let mut hashed: Vec<(String, Vec<u8>)> = Vec::new();
            match &dir {
                Some(d) => {
                    for (fname, m) in d.payload.get("members").and_then(Value::as_object).into_iter().flatten() {
                        let Some(oid) = m.get("object_id").and_then(Value::as_str) else { continue };
                        let bytes = self.objects.get_file(oid)?;
                        files.push(json!({ "name": fname, "obj_id": oid, "size": bytes.len() }));
                        hashed.push((fname.clone(), bytes));
                    }
                }
                None => {
                    // never built from source (a wish result, a template): its source is synthesized from the payload
                    for (fname, text) in synthesize_sources(&def.payload, &ident) {
                        files.push(json!({ "name": fname, "size": text.len(), "text": text }));
                        hashed.push((fname, text.into_bytes()));
                    }
                }
            }
            let digest = content_digest(hashed.iter().map(|(n, b)| (n.as_str(), b.as_slice())));
            let kind = def.payload.get("kind").and_then(Value::as_str).unwrap_or("").to_string();
            match kind.as_str() {
                "html" => html_api = html_api.max(def.payload.get("html").and_then(|h| h.get("api_version")).and_then(Value::as_u64).unwrap_or(HTML_API_VERSION)),
                COMPOSITE => {}
                _ => decl_api = decl_api.max(DECLARATIVE_API_VERSION),
            }
            let version = def.payload.get("version").and_then(Value::as_u64).unwrap_or(1);
            let mut mc = json!({ "ident": ident, "name": name, "kind": kind, "version": version, "digest": digest, "dir": dir_name });
            let deps: Vec<Value> = def.payload.get("deps").and_then(Value::as_array).into_iter().flatten().filter_map(|d| d.get("ident").cloned()).collect();
            if !deps.is_empty() {
                mc["deps"] = json!(deps);
            }
            for k in ["title", "description", "catalog"] {
                if let Some(v) = def.payload.get(k) {
                    mc[k] = v.clone();
                }
            }
            let control_deps = mc["deps"].clone();
            manifest_controls.push(mc);
            for f in &files {
                let mut flat = f.clone();
                flat["path"] = json!(format!("{dir_name}{}", f["name"].as_str().unwrap_or("")));
                if let Some(o) = flat.as_object_mut() {
                    o.remove("name");
                }
                files_out.push(flat);
            }
            controls.push(json!({ "entity_id": def.entity_id, "ident": ident, "kind": kind, "version": version, "def_rev": def.content_rev,
                                  "dir_rev": dir.as_ref().map_or(0, |d| d.content_rev), "built": dir.is_some(), "digest": digest, "dir": dir_name, "files": files,
                                  "provenance": def.payload.get("provenance"), "deps": control_deps,
                                  "built_at": def.derived.as_ref().and_then(|d| d.pointer("/build/built_at")).cloned() }));
        }
        let mut host_api = Map::new();
        if html_api > 0 {
            host_api.insert("html".into(), json!(html_api));
        }
        if decl_api > 0 {
            host_api.insert("declarative".into(), json!(decl_api));
        }
        let manifest = json!({ "manifest_version": MANIFEST_VERSION, "purpose": "aiws", "host_api": host_api, "controls": manifest_controls, "workspace_templates": [] });
        Ok(json!({ "ok": true, "manifest": manifest, "controls": controls, "files": files_out }))
    }
}

impl Workspace {
    /// The bytes of a package export (`ext.export { purpose: package }`): the manifest, every file by bundle path
    /// (`aiws-manifest.json` included) and the newest build time of its controls in Unix seconds (`ext.pack` stamps
    /// the AppDoc with it, so the same bundle packs the same).
    /// `templates` (P3) are `[{ surfaces, data?, template }]`: each one's snapshot goes under `templates/<id>/`, its item into
    /// `workspace_templates`, and the controls it needs join the chosen ones.
    pub fn ext_bundle_files(&self, caller: &Caller, entity_ids: &[String], templates: &[Value]) -> WsResult<(Value, BTreeMap<String, Vec<u8>>, Option<u64>)> {
        let mut ids = entity_ids.to_vec();
        let mut items = Vec::new();
        let mut template_files = BTreeMap::new();
        for t in templates {
            let list = |k: &str| -> Vec<String> { t.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() };
            let (item, controls, files) = self.ext_export_template(caller, &list("surfaces"), &list("data"), t.get("template").unwrap_or(&Value::Null))?;
            if items.iter().any(|i: &Value| i["id"] == item["id"]) {
                return Err(WsError::invalid_op(format!("template {} is listed twice", item["id"])));
            }
            for c in controls {
                if !ids.contains(&c) {
                    ids.push(c);
                }
            }
            items.push(item);
            template_files.extend(files);
        }
        let bundle = if ids.is_empty() {
            json!({ "manifest": { "manifest_version": MANIFEST_VERSION, "purpose": "aiws", "host_api": {}, "controls": [], "workspace_templates": [] }, "controls": [], "files": [] })
        } else {
            self.ext_export(caller, &ids, "package")?
        };
        let mut files = BTreeMap::new();
        for f in bundle["files"].as_array().into_iter().flatten() {
            let path = f["path"].as_str().unwrap_or("").to_string();
            let bytes = match (f.get("obj_id").and_then(Value::as_str), f.get("text").and_then(Value::as_str)) {
                (Some(oid), _) => self.objects.get_file(oid)?,
                (None, Some(t)) => t.as_bytes().to_vec(),
                _ => continue,
            };
            files.insert(path, bytes);
        }
        files.extend(template_files);
        let mut manifest = bundle["manifest"].clone();
        manifest["workspace_templates"] = json!(items);
        let mut text = serde_json::to_string_pretty(&manifest).map_err(|e| WsError::io(e.to_string()))?;
        text.push('\n');
        files.insert(crate::library::MANIFEST_FILE.to_string(), text.into_bytes());
        let built = bundle["controls"].as_array().into_iter().flatten().filter_map(|c| c["built_at"].as_str().and_then(aiworkspace_core::value::parse_datetime_ms).map(|ms| (ms / 1000).max(0) as u64)).max();
        Ok((manifest, files, built))
    }
}

/// The source files of a definition that was never built (component.json plus its inline entry files).
pub fn synthesize_sources(payload: &Map<String, Value>, ident: &str) -> Vec<(String, String)> {
    let (author, name) = ident.split_once('#').unwrap_or(("", ident));
    let mut c = json!({ "manifest_version": MANIFEST_VERSION, "ident": { "author_did": author, "name": name }, "kind": payload.get("kind"),
                        "title": payload.get("title").cloned().unwrap_or(json!(name)) });
    let deps: Vec<Value> = payload.get("deps").and_then(Value::as_array).into_iter().flatten().filter_map(|d| d.get("ident").cloned()).collect();
    if !deps.is_empty() {
        c["deps"] = json!(deps);
    }
    for k in ["description", "accepts", "allow_no_source", "default_size", "config_schema", "bindings", "catalog"] {
        if let Some(v) = payload.get(k) {
            c[k] = v.clone();
        }
    }
    let mut out = Vec::new();
    match payload.get("kind").and_then(Value::as_str) {
        Some("html") => {
            let h = payload.get("html").cloned().unwrap_or(json!({}));
            let mut entry = Map::new();
            for (k, file) in [("html", "view.html"), ("css", "style.css"), ("js", "behavior.js")] {
                if let Some(t) = h.get(k).and_then(Value::as_str).filter(|t| k == "html" || !t.is_empty()) {
                    entry.insert(k.into(), json!(file));
                    out.push((file.to_string(), t.to_string()));
                }
            }
            if let Some(api) = h.get("api_version") {
                c["host_api"] = api.clone();
            }
            c["entry"] = Value::Object(entry);
        }
        Some(COMPOSITE) => {
            c["entry"] = json!(COMPOSITION_FILE);
            out.push((COMPOSITION_FILE.into(), serde_json::to_string_pretty(payload.get("composition").unwrap_or(&json!({}))).unwrap_or_default()));
        }
        _ => {
            c["entry"] = json!("declarative.json");
            out.push(("declarative.json".into(), serde_json::to_string_pretty(payload.get("declarative").unwrap_or(&json!({}))).unwrap_or_default()));
        }
    }
    if let Some(a) = payload.get("agent").and_then(Value::as_str) {
        c["agent"] = json!("AGENT.md");
        out.push(("AGENT.md".into(), a.to_string()));
    }
    out.push((COMPONENT_FILE.into(), serde_json::to_string_pretty(&c).unwrap_or_default()));
    out.sort();
    out
}

/// The error a refused build Commit answers: its code, and a stale base said in build words.
fn commit_error(outcome: &Value) -> WsError {
    let code = outcome["code"].as_str().and_then(Code::parse).unwrap_or(Code::InvalidOperation);
    if outcome["status"] == "conflict" {
        return WsError::sub(Code::RevisionConflict, "BUILD_BASE_STALE", "the definition or its source changed meanwhile: pull, merge, build again").with_data(json!({ "commit": outcome }));
    }
    let detail = outcome["errors"][0]["detail"].as_str().unwrap_or("the build Commit was refused").to_string();
    WsError::new(code, detail).with_data(json!({ "commit": outcome }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digests_match_the_client_vector() {
        // buckyos-websdk tests/aiws_ext.test.ts computes the same value
        assert_eq!(content_digest([("b.json", b"{}".as_slice()), ("a.txt", b"hello".as_slice())]), "sha256:febb1057a0ccca4402f38caf1f5441717e6e56ef19188621177b68c1593c9488");
    }

    #[test]
    fn config_schema_subset() {
        let mut out = Vec::new();
        check_config_schema(&json!({ "type": "object", "required": ["a"], "properties": { "a": { "type": "string", "maxLength": 9, "enum": ["x"] }, "n": { "type": ["number", "null"], "minimum": 0 } } }), "s", &mut out);
        assert!(out.is_empty(), "{out:?}");
        check_config_schema(&json!({ "properties": { "a": { "pattern": "x", "type": "date" } }, "oneOf": [] }), "s", &mut out);
        assert_eq!(out.len(), 3, "{out:?}");
    }

    #[test]
    fn narrowing_names_what_instances_lose() {
        let old = json!({ "kind": "declarative", "accepts": ["buckyos.record", "buckyos.table-source"], "bindings": { "a": {}, "b": { "required": true } }, "config_schema": { "required": ["x"] } });
        let new = json!({ "kind": "declarative", "accepts": ["buckyos.record"], "bindings": { "a": { "required": true }, "c": { "required": true } }, "config_schema": { "required": ["x", "y"] } });
        let n = narrowing(old.as_object().unwrap(), new.as_object().unwrap());
        assert_eq!(n.len(), 5, "{n:?}");
        assert!(narrowing(new.as_object().unwrap(), new.as_object().unwrap()).is_empty());
    }
}
