//! 系统 URL 的纯函数层：语法归一化、`obj://` host 段校验、RBAC 资源 key、对象事件 kevent key、
//! path DID。规范是 `doc/key url.md` 第一部分；websdk `object_url.ts` 实现同一规则，两端共用
//! `testdata/object_url_vectors.json`。需要 I/O 的发现（Card、服务查询、挂载映射）不在这里。

use std::collections::BTreeMap;
use std::fmt;

use name_lib::DID;
use ndn_lib::ObjId;
use serde_json::Value;

/// key url.md §4.1 登记的 current-zone 服务名；未登记的服务名报错。
pub const OBJ_SERVICE_NAMES: &[&str] = &[
    "config",
    "dfs",
    "taskmgr",
    "kmsg",
    "bucket",
    "ndm",
    "msg-center",
    "aiworkspace",
    "homestation",
    "opendan",
    "aicc",
];

/// 对象事件 kevent key 的固定前缀（key url.md §7）。
pub const OBJECT_EVENT_PREFIX: &str = "/obj";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ObjectUrlError(pub String);

fn err<T>(message: impl Into<String>) -> Result<T, ObjectUrlError> {
    Err(ObjectUrlError(message.into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjHost {
    /// current zone 的系统服务，不展开、不带 Zone。
    Service(String),
    /// 带 Zone 的实体 DID canonical hostname（agent-id、user-id、group DID host）。
    Entity(String),
}

impl ObjHost {
    pub fn as_str(&self) -> &str {
        match self {
            ObjHost::Service(name) | ObjHost::Entity(name) => name,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectUrl {
    /// `file://<device_id>/abs/path`；`device_id = None` 是空 host（含 `localhost`）。
    File {
        device_id: Option<String>,
        path: String,
    },
    /// 路径型 `cyfs:///path`（`zone = None`，含 `cyfs://_/path`）或 `cyfs://<zone>/path`。
    CyfsPath { zone: Option<String>, path: String },
    /// ObjId 型 `cyfs://<objid>[.<zone>]/<inner>`。
    CyfsObject {
        obj_id: ObjId,
        source_zone: Option<String>,
        inner_path: String,
    },
    /// `obj://<host>/<path>?<query>`；path 为空表示对象根，query 已按 key 排序。
    Obj {
        host: ObjHost,
        path: String,
        query: BTreeMap<String, String>,
    },
    /// `https://` / `http://`，原样保留。
    Web { url: String },
    /// `buckyos://`，原样保留。
    BuckyOS { url: String },
}

impl ObjectUrl {
    pub fn scheme(&self) -> &'static str {
        match self {
            ObjectUrl::File { .. } => "file",
            ObjectUrl::CyfsPath { .. } | ObjectUrl::CyfsObject { .. } => "cyfs",
            ObjectUrl::Obj { .. } => "obj",
            ObjectUrl::Web { .. } => "https",
            ObjectUrl::BuckyOS { .. } => "buckyos",
        }
    }

    pub fn obj_host(&self) -> Option<&ObjHost> {
        match self {
            ObjectUrl::Obj { host, .. } => Some(host),
            _ => None,
        }
    }

    pub fn obj_segments(&self) -> Vec<String> {
        match self {
            ObjectUrl::Obj { path, .. } => decoded_segments(path),
            _ => Vec::new(),
        }
    }

    /// 同一对象去掉 query 后的 URL（快照 / 视图选择器不参与对象身份与授权）。
    pub fn without_query(&self) -> ObjectUrl {
        match self {
            ObjectUrl::Obj { host, path, .. } => ObjectUrl::Obj {
                host: host.clone(),
                path: path.clone(),
                query: BTreeMap::new(),
            },
            other => other.clone(),
        }
    }

    /// RBAC 资源 key（RBAC 权限设计"基于 URL 的资源 key 布局"）：别名归一化后唯一，不含 query。
    /// `cyfs:///P` 与当前 Zone 的 `cyfs://<zone>/P` 归到 `obj://dfs/P`；File、ObjId 与 Web 不由 RBAC 资源 key 表达。
    pub fn rbac_resource_key(&self, current_zone: Option<&str>) -> Option<String> {
        match self {
            ObjectUrl::Obj { host, path, .. } => Some(format!("obj://{}{}", host.as_str(), path)),
            ObjectUrl::CyfsPath { zone, path } => {
                let local = match (zone, current_zone) {
                    (None, _) => true,
                    (Some(zone), Some(current)) => zone.eq_ignore_ascii_case(current),
                    (Some(_), None) => false,
                };
                local.then(|| format!("obj://dfs{path}"))
            }
            _ => None,
        }
    }

    /// 对象事件 kevent key：`/obj/<host>/<seg...>/<event>`，段名不合法时 `x` + 小写 hex。
    pub fn object_event_key(&self, event: &str) -> Result<String, ObjectUrlError> {
        let ObjectUrl::Obj { host, path, .. } = self else {
            return err("object events are only defined for obj:// urls");
        };
        if event.is_empty() || !event.chars().all(is_plain_event_char) || event.starts_with('x') {
            return err(format!("invalid object event name '{event}'"));
        }
        let mut key = object_event_base(host, path);
        key.push('/');
        key.push_str(event);
        if key.len() > 256 || key.split('/').skip(1).count() > 8 {
            return err(format!("object event key too long: {key}"));
        }
        Ok(key)
    }

    /// 订阅该对象及其子对象全部事件的 kevent pattern。
    pub fn object_event_pattern(&self) -> Result<String, ObjectUrlError> {
        let ObjectUrl::Obj { host, path, .. } = self else {
            return err("object events are only defined for obj:// urls");
        };
        Ok(format!("{}/**", object_event_base(host, path)))
    }

    /// path DID（D8）：服务对象 `<zone_did>:<service>:<seg...>`，实体对象 `<entity_did>:<seg...>`。
    pub fn object_did(&self, zone_did: &DID) -> Option<DID> {
        let ObjectUrl::Obj { host, path, .. } = self else {
            return None;
        };
        let (base, mut segments) = match host {
            ObjHost::Service(name) => (zone_did.clone(), vec![name.clone()]),
            ObjHost::Entity(entity) => (DID::from_str(entity).ok()?, Vec::new()),
        };
        segments.extend(path.split('/').filter(|seg| !seg.is_empty()).map(|seg| seg.replace(':', "%3A")));
        if segments.is_empty() {
            return Some(base);
        }
        Some(DID::new(&base.method, &format!("{}:{}", base.id, segments.join(":"))))
    }
}

impl fmt::Display for ObjectUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ObjectUrl::File { device_id, path } => {
                write!(f, "file://{}{}", device_id.as_deref().unwrap_or(""), path)
            }
            ObjectUrl::CyfsPath { zone, path } => {
                write!(f, "cyfs://{}{}", zone.as_deref().unwrap_or(""), path)
            }
            ObjectUrl::CyfsObject {
                obj_id,
                source_zone,
                inner_path,
            } => {
                write!(f, "cyfs://{}", obj_id.to_base32())?;
                if let Some(zone) = source_zone {
                    write!(f, ".{zone}")?;
                }
                write!(f, "{}", if inner_path.is_empty() { "/" } else { inner_path })
            }
            ObjectUrl::Obj { host, path, query } => {
                write!(f, "obj://{}{}", host.as_str(), path)?;
                if !query.is_empty() {
                    let query = query
                        .iter()
                        .map(|(k, v)| format!("{}={}", encode_obj_query_component(k), encode_obj_query_component(v)))
                        .collect::<Vec<_>>()
                        .join("&");
                    write!(f, "?{query}")?;
                }
                Ok(())
            }
            ObjectUrl::Web { url } | ObjectUrl::BuckyOS { url } => f.write_str(url),
        }
    }
}

pub fn is_obj_service_name(name: &str) -> bool {
    OBJ_SERVICE_NAMES.contains(&name)
}

/// 严格解析并归一化（key url.md 第一部分）。缩写（`//config/x`、`/abs`）不在这里展开，见 [`expand_input_shorthand`]。
pub fn parse_object_url(input: &str) -> Result<ObjectUrl, ObjectUrlError> {
    let input = input.trim();
    let Some((scheme, rest)) = input.split_once("://") else {
        return err(format!("'{input}' has no scheme"));
    };
    let scheme = scheme.to_ascii_lowercase();
    match scheme.as_str() {
        "https" | "http" => Ok(ObjectUrl::Web {
            url: url::Url::parse(input)
                .map_err(|e| ObjectUrlError(format!("invalid web url '{input}': {e}")))?
                .to_string(),
        }),
        "buckyos" => Ok(ObjectUrl::BuckyOS {
            url: input.to_string(),
        }),
        "file" => {
            let (authority, path, query) = split_authority(rest);
            if query.is_some() {
                return err("file url does not take query or fragment");
            }
            let device_id = match authority.to_ascii_lowercase().as_str() {
                "" | "localhost" => None,
                host if is_device_id(host) => Some(host.to_string()),
                host => return err(format!("invalid device id '{host}' in file url")),
            };
            if path.is_empty() {
                return err("file url requires an absolute path");
            }
            Ok(ObjectUrl::File {
                device_id,
                path: normalize_path(path, false)?,
            })
        }
        "cyfs" => {
            let (authority, path, query) = split_authority(rest);
            if query.is_some() {
                return err("cyfs url does not take query or fragment");
            }
            let authority = authority.to_ascii_lowercase();
            if authority.is_empty() || authority == "_" {
                if path.is_empty() {
                    return err("cyfs path url requires a path");
                }
                return Ok(ObjectUrl::CyfsPath {
                    zone: None,
                    path: normalize_path(path, false)?,
                });
            }
            let (first_label, rest_labels) = authority
                .split_once('.')
                .map(|(a, b)| (a, Some(b)))
                .unwrap_or((authority.as_str(), None));
            if let Some(obj_id) = strict_objid_label(first_label) {
                if let Some(zone) = rest_labels {
                    if !is_hostname(zone) {
                        return err(format!("invalid source zone '{zone}'"));
                    }
                }
                return Ok(ObjectUrl::CyfsObject {
                    obj_id,
                    source_zone: rest_labels.map(str::to_string),
                    inner_path: if path.is_empty() || path == "/" {
                        String::new()
                    } else {
                        normalize_path(path, false)?
                    },
                });
            }
            if !is_hostname(&authority) || !authority.contains('.') {
                return err(format!("'{authority}' is neither a zone host nor an ObjId"));
            }
            if path.is_empty() {
                return err("cyfs path url requires a path");
            }
            Ok(ObjectUrl::CyfsPath {
                zone: Some(authority),
                path: normalize_path(path, false)?,
            })
        }
        "obj" => {
            let (authority, path, query) = split_authority(rest);
            let host = parse_obj_host(&authority.to_ascii_lowercase())?;
            let path = if path.is_empty() || path == "/" {
                String::new()
            } else {
                normalize_path(path, true)?
            };
            let query = match query {
                Some(query) => parse_query(query)?,
                None => BTreeMap::new(),
            };
            Ok(ObjectUrl::Obj { host, path, query })
        }
        other => err(format!("unsupported scheme '{other}'")),
    }
}

/// 只在声明采用缩写语法的输入入口（CLI、Prompt 环境）使用（key url.md §8）。
pub fn expand_input_shorthand(input: &str) -> String {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix("//") {
        let service = rest.split(['/', '?']).next().unwrap_or_default();
        if is_obj_service_name(service) {
            return format!("obj://{rest}");
        }
        return input.to_string();
    }
    if input.starts_with('/') {
        return format!("file://{input}");
    }
    input.to_string()
}

/// Zone 级 default device（key url.md §3）：值是 device id 的 JSON 字符串。
pub const DEFAULT_DEVICE_KEY: &str = "system/default_device";

/// file url 所属设备（D1–D3）：显式 host；空 host 时有来源 runtime 取其设备，没有来源上下文才取 Zone default device。
pub fn file_url_device<'a>(
    url: &'a ObjectUrl,
    source_device: Option<&'a str>,
    default_device: Option<&'a str>,
) -> Option<&'a str> {
    match url {
        ObjectUrl::File { device_id: Some(device), .. } => Some(device.as_str()),
        ObjectUrl::File { device_id: None, .. } => source_device.or(default_device),
        _ => None,
    }
}

/// 设备的节点配置 key：其中 `node_did` 是该设备 node executor 的目标（`ExecutorRuntimeTarget.node_did`）。
pub fn device_node_config_key(device_id: &str) -> String {
    format!("nodes/{device_id}/config")
}

/// 解析 `obj://` host 段：服务名或带 Zone 的实体 hostname，其余报错（key url.md §4）。
pub fn parse_obj_host(host: &str) -> Result<ObjHost, ObjectUrlError> {
    if host.is_empty() {
        return err("obj url requires a host");
    }
    if is_obj_service_name(host) {
        return Ok(ObjHost::Service(host.to_string()));
    }
    if !host.contains('.') {
        return err(format!(
            "'{host}' is not a registered obj:// service name; type shorthands are not supported"
        ));
    }
    if !is_hostname(host) {
        return err(format!("'{host}' is not a valid entity hostname"));
    }
    let first_label = host.split('.').next().unwrap_or_default();
    if is_obj_service_name(first_label) {
        return err(format!(
            "'{host}' expands a service name; write obj://{first_label}/ for the current zone or use https across zones"
        ));
    }
    if strict_objid_label(first_label).is_some() {
        return err("NamedObject ids do not go into obj:// host; use cyfs://<objid>/");
    }
    Ok(ObjHost::Entity(host.to_string()))
}

/// 由 `obj://` host 段与已解码路径段直接构造对象事件 kevent key（key url.md §7），不经解析。
pub fn obj_event_key<S: AsRef<str>>(host: &str, segments: &[S], event: &str) -> String {
    format!("{}/{event}", obj_event_base(host, segments))
}

/// 订阅该对象及其子对象全部事件的 kevent pattern。
pub fn obj_event_pattern<S: AsRef<str>>(host: &str, segments: &[S]) -> String {
    format!("{}/**", obj_event_base(host, segments))
}

fn obj_event_base<S: AsRef<str>>(host: &str, segments: &[S]) -> String {
    let mut key = format!("{OBJECT_EVENT_PREFIX}/{}", kevent_segment(host));
    for segment in segments {
        key.push('/');
        key.push_str(&kevent_segment(segment.as_ref()));
    }
    key
}

/// 由已解码的路径段构造服务对象 URL。
pub fn obj_service_url<S: AsRef<str>>(service: &str, segments: &[S]) -> String {
    format!("obj://{}{}", service, encode_segments(segments))
}

/// 由实体 DID 与已解码路径段构造实体对象 URL；host 为 DID 的 canonical hostname。
pub fn obj_entity_url<S: AsRef<str>>(entity: &DID, segments: &[S]) -> String {
    format!("obj://{}{}", entity.to_host_name(), encode_segments(segments))
}

/// `obj://config/<key>`、`/config/<key>`、`/<key>` → system-config key（与 sys_config_service 一致）。
pub fn config_key_from_url(input: &str) -> &str {
    let mut key = input.strip_prefix("obj://config/").unwrap_or(input);
    if let Some(rest) = key.strip_prefix("/config/") {
        key = rest;
    }
    key.trim_start_matches('/').trim_start_matches('\\')
}

pub fn config_obj_url(key: &str) -> String {
    format!("obj://config/{}", config_key_from_url(key))
}

/// 规范化 JSON 序列化：对象键排序、无空白；query 中的 selector 等 JSON 值用它（key url.md §6）。
pub fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<_> = map.iter().collect();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let body = entries
                .into_iter()
                .map(|(k, v)| format!("{}:{}", Value::String(k.clone()), canonical_json(v)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(items) => format!(
            "[{}]",
            items.iter().map(canonical_json).collect::<Vec<_>>().join(",")
        ),
        other => other.to_string(),
    }
}

/// kevent 段编码：`[A-Za-z0-9_-]` 且不以 `x` 开头的段原样保留，其余为 `x` + 小写 hex（不会与原样段冲突）。
pub fn kevent_segment(segment: &str) -> String {
    if !segment.is_empty() && segment.chars().all(is_plain_event_char) && !segment.starts_with('x') {
        return segment.to_string();
    }
    let mut out = String::with_capacity(segment.len() * 2 + 1);
    out.push('x');
    for byte in segment.as_bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn object_event_base(host: &ObjHost, path: &str) -> String {
    obj_event_base(host.as_str(), &decoded_segments(path))
}

fn is_plain_event_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'
}

fn split_authority(rest: &str) -> (&str, &str, Option<&str>) {
    let (before_query, query) = match rest.find(['?', '#']) {
        Some(pos) => (&rest[..pos], Some(&rest[pos..])),
        None => (rest, None),
    };
    let query = query.map(|q| {
        let q = q.split('#').next().unwrap_or_default();
        q.strip_prefix('?').unwrap_or(q)
    });
    let query = query.filter(|q| !q.is_empty());
    match before_query.find('/') {
        Some(pos) => (&before_query[..pos], &before_query[pos..], query),
        None => (before_query, "", query),
    }
}

/// 路径段 percent 归一：unreserved 解码，其余大写 hex；拒绝空段、`.`、`..`。
fn normalize_path(path: &str, trim_trailing_slash: bool) -> Result<String, ObjectUrlError> {
    if !path.starts_with('/') {
        return err(format!("path '{path}' must be absolute"));
    }
    let mut body = &path[1..];
    if trim_trailing_slash {
        body = body.strip_suffix('/').unwrap_or(body);
    }
    let mut out = String::new();
    let raw_segments: Vec<&str> = body.split('/').collect();
    let last = raw_segments.len() - 1;
    for (index, raw) in raw_segments.into_iter().enumerate() {
        if raw.is_empty() && index == last && !trim_trailing_slash {
            out.push('/');
            continue;
        }
        let decoded = percent_decode(raw)?;
        if decoded.is_empty() || decoded == "." || decoded == ".." {
            return err(format!("ambiguous path segment in '{path}'"));
        }
        out.push('/');
        out.push_str(&encode_obj_segment(&decoded));
    }
    Ok(out)
}

fn decoded_segments(path: &str) -> Vec<String> {
    path.split('/')
        .filter(|seg| !seg.is_empty())
        .map(|seg| percent_decode(seg).unwrap_or_else(|_| seg.to_string()))
        .collect()
}

fn encode_segments<S: AsRef<str>>(segments: &[S]) -> String {
    segments
        .iter()
        .map(|seg| format!("/{}", encode_obj_segment(seg.as_ref())))
        .collect()
}

fn parse_query(query: &str) -> Result<BTreeMap<String, String>, ObjectUrlError> {
    let mut out = BTreeMap::new();
    for pair in query.split('&').filter(|pair| !pair.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(&key.replace('+', " "))?;
        let mut value = percent_decode(&value.replace('+', " "))?;
        if key == "selector" {
            let parsed: Value = serde_json::from_str(&value)
                .map_err(|e| ObjectUrlError(format!("selector must be JSON: {e}")))?;
            value = canonical_json(&parsed);
        }
        if out.insert(key.clone(), value).is_some() {
            return err(format!("query key '{key}' repeats"));
        }
    }
    Ok(out)
}

fn percent_decode(input: &str) -> Result<String, ObjectUrlError> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = input
                .get(i + 1..i + 3)
                .ok_or_else(|| ObjectUrlError(format!("invalid escape in '{input}'")))?;
            out.push(
                u8::from_str_radix(hex, 16)
                    .map_err(|_| ObjectUrlError(format!("invalid escape in '{input}'")))?,
            );
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ObjectUrlError(format!("'{input}' is not UTF-8")))
}

fn encode_with(input: &str, keep: impl Fn(u8) -> bool) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        if keep(*byte) {
            out.push(*byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)
}

/// 单个已解码路径段的规范编码（与 [`parse_object_url`] 的归一结果一致）。
pub fn encode_obj_segment(segment: &str) -> String {
    encode_with(segment, |b| is_unreserved(b) || b"!$&'()*+,;=:@".contains(&b))
}

/// Canonical encoding of a query key or value.
pub fn encode_obj_query_component(component: &str) -> String {
    encode_with(component, |b| is_unreserved(b) || b"!$'()*,;:@/".contains(&b))
}

fn is_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

fn is_device_id(host: &str) -> bool {
    is_hostname(host)
}

/// ObjId 的严格判定：base32 往返一致、类型名合法、hash 至少 16 字节。不靠"看起来像 hash"。
fn strict_objid_label(label: &str) -> Option<ObjId> {
    if label.len() < 26 {
        return None;
    }
    let obj_id = ObjId::new(label).ok()?;
    let type_ok = !obj_id.obj_type.is_empty()
        && obj_id
            .obj_type
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-');
    (type_ok && obj_id.obj_hash.len() >= 16 && obj_id.to_base32() == label).then_some(obj_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(serde::Deserialize)]
    struct Vector {
        input: String,
        #[serde(default)]
        shorthand: bool,
        #[serde(default)]
        error: bool,
        #[serde(default)]
        canonical: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        resource_key: Option<String>,
        #[serde(default)]
        event: Option<String>,
        #[serde(default)]
        event_key: Option<String>,
    }

    fn kind_of(url: &ObjectUrl) -> &'static str {
        match url {
            ObjectUrl::File { .. } => "file",
            ObjectUrl::CyfsPath { .. } => "cyfs_path",
            ObjectUrl::CyfsObject { .. } => "cyfs_object",
            ObjectUrl::Obj {
                host: ObjHost::Service(_),
                ..
            } => "obj_service",
            ObjectUrl::Obj {
                host: ObjHost::Entity(_),
                ..
            } => "obj_entity",
            ObjectUrl::Web { .. } => "web",
            ObjectUrl::BuckyOS { .. } => "buckyos",
        }
    }

    #[test]
    fn shared_vectors() {
        let vectors: Vec<Vector> =
            serde_json::from_str(include_str!("../testdata/object_url_vectors.json")).unwrap();
        for v in vectors {
            let input = if v.shorthand {
                expand_input_shorthand(&v.input)
            } else {
                v.input.clone()
            };
            let parsed = parse_object_url(&input);
            if v.error {
                assert!(parsed.is_err(), "{} should be rejected, got {:?}", v.input, parsed);
                continue;
            }
            let parsed = parsed.unwrap_or_else(|e| panic!("{} failed: {e}", v.input));
            if let Some(canonical) = &v.canonical {
                assert_eq!(&parsed.to_string(), canonical, "canonical of {}", v.input);
                assert_eq!(parse_object_url(canonical).unwrap(), parsed, "roundtrip {}", v.input);
            }
            if let Some(kind) = &v.kind {
                assert_eq!(kind_of(&parsed), kind, "kind of {}", v.input);
            }
            if let Some(key) = &v.resource_key {
                assert_eq!(
                    parsed.rbac_resource_key(Some("test.buckyos.io")).as_deref(),
                    Some(key.as_str()),
                    "resource key of {}",
                    v.input
                );
            }
            if let (Some(event), Some(key)) = (&v.event, &v.event_key) {
                assert_eq!(&parsed.object_event_key(event).unwrap(), key, "event key of {}", v.input);
            }
        }
    }

    #[test]
    fn file_device_follows_host_then_source_then_default() {
        let explicit = parse_object_url("file://node2/tmp/a").unwrap();
        let empty = parse_object_url("file:///tmp/a").unwrap();
        assert_eq!(file_url_device(&explicit, Some("agent-box"), Some("ood1")), Some("node2"));
        assert_eq!(file_url_device(&empty, Some("agent-box"), Some("ood1")), Some("agent-box"));
        assert_eq!(file_url_device(&empty, None, Some("ood1")), Some("ood1"));
        assert_eq!(file_url_device(&empty, None, None), None);
        assert_eq!(device_node_config_key("ood1"), "nodes/ood1/config");
    }

    #[test]
    fn objid_host_roundtrip() {
        let obj_id = ObjId::new_by_raw("cyfile".to_string(), vec![7u8; 32]);
        let url = format!("cyfs://{}.test.buckyos.io/a/b.txt", obj_id.to_base32());
        let parsed = parse_object_url(&url).unwrap();
        assert_eq!(parsed.to_string(), url);
        let ObjectUrl::CyfsObject { obj_id: parsed_id, source_zone, .. } = parsed else {
            panic!("not an ObjId url");
        };
        assert_eq!(parsed_id, obj_id);
        assert_eq!(source_zone.as_deref(), Some("test.buckyos.io"));
        assert!(parse_object_url(&format!("obj://{}/x", obj_id.to_base32())).is_err());
    }

    #[test]
    fn path_did_and_builders() {
        let zone = DID::from_str("did:web:test.buckyos.io").unwrap();
        let feed = parse_object_url("obj://homestation/home/alice/feed").unwrap();
        assert_eq!(
            feed.object_did(&zone).unwrap().to_string(),
            "did:web:test.buckyos.io:homestation:home:alice:feed"
        );
        let session = parse_object_url("obj://xiaobai.test.buckyos.io/sessions/s1").unwrap();
        assert_eq!(
            session.object_did(&zone).unwrap().to_string(),
            "did:web:xiaobai.test.buckyos.io:sessions:s1"
        );
        assert_eq!(
            obj_service_url("msg-center", &["owners", "did:bns:alice", "sessions", "a b"]),
            "obj://msg-center/owners/did:bns:alice/sessions/a%20b"
        );
        assert_eq!(config_key_from_url("obj://config/users/alice/settings"), "users/alice/settings");
        assert_eq!(config_obj_url("/config/boot/config"), "obj://config/boot/config");
    }

    #[test]
    fn canonical_selector_ignores_key_order_and_whitespace() {
        let a = parse_object_url(
            r#"obj://aiworkspace/ws_1/entities/e1?selector={"b":1,%20"a":{"y":2,"x":[1,2]}}&mode=live"#,
        )
        .unwrap();
        let b = parse_object_url(
            r#"obj://aiworkspace/ws_1/entities/e1?mode=live&selector={"a":{"x":[1,2],"y":2},"b":1}"#,
        )
        .unwrap();
        assert_eq!(a.to_string(), b.to_string());
        assert_eq!(canonical_json(&json!({"b": 1, "a": true})), r#"{"a":true,"b":1}"#);
    }
}
