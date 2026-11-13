//! Sources on the service (内核架构改进 D12, D24, D26; M4): refresh runs, the credential-less public URL fetcher
//! (the only in-service Source executor), the publication HTTP surface (`/pub/<workspace>/<name>`; on the
//! Workspace's own host `aiws-<id>.<zonehost>/<workspace_name>/<name>` it is reached through `pages`), and the timer
//! that refreshes Sources on schedule.

use crate::AppState;
use aiworkspace_core::markdown::{from_richtext, to_richtext};
use aiworkspace_core::value::format_utc_ms;
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::source::{Fetched, PUB_FORMAT};
use aiworkspace_store::Caller;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::Arc;

const MAX_FETCH_BYTES: usize = 32 * 1024 * 1024;
const FETCH_TIMEOUT_SECS: u64 = 30;

/// `proc.start { program: source.refresh@1, params: { entity_id } }`: the run is created and refreshed on a worker.
pub fn start(state: &Arc<AppState>, caller: &Caller, ws_id: &str, params: &Value, idem: &str) -> WsResult<Value> {
    let entity_id = params.get("entity_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("entity_id required"))?.to_string();
    let run = state.with_ws(ws_id, |ws| ws.source_refresh_start(caller, &entity_id, idem))?;
    if run["replayed"] != json!(true) {
        let (st, w, rid) = (state.clone(), ws_id.to_string(), run["run_id"].as_str().unwrap_or("").to_string());
        tokio::runtime::Handle::current().spawn(async move { refresh(st, w, rid).await });
    }
    Ok(run)
}

/// Fetch, then apply (one Commit of origin `source`); a failure leaves the entity as it was and the run says why.
pub async fn refresh(state: Arc<AppState>, ws_id: String, run_id: String) {
    let (st, w, r) = (state.clone(), ws_id.clone(), run_id.clone());
    let run = match tokio::task::spawn_blocking(move || st.with_ws(&w, |ws| ws.source_run(&r))).await {
        Ok(Ok(run)) => run,
        _ => return,
    };
    let source = run["params"]["source"].clone();
    let principal = run["principal"].as_str().unwrap_or("").to_string();
    let consumer = run["params"]["entity_id"].as_str().unwrap_or("").to_string();
    let (st, w, r) = (state.clone(), ws_id.clone(), run_id.clone());
    let _ = tokio::task::spawn_blocking(move || st.with_ws(&w, |ws| ws.source_run_set(&r, "running", None))).await;
    let outcome = fetch(&state, &ws_id, &consumer, &source, &principal).await;
    let (st, w, r) = (state.clone(), ws_id.clone(), run_id.clone());
    let _ = tokio::task::spawn_blocking(move || {
        st.with_ws(&w, |ws| match outcome {
            Ok((fetched, bytes, root)) => {
                if let Err(e) = ws.add_traffic(bytes) {
                    ws.source_fail(&r, &e)?;
                    return Err(e);
                }
                ws.source_apply(&r, &fetched, bytes, root.as_deref()).map(|_| ())
            }
            Err(e) => {
                ws.source_fail(&r, &e)?;
                Err(e)
            }
        })
    })
    .await;
}

/// The timer: every Source whose interval elapsed is refreshed as the workspace owner (D24: a scheduled refresh
/// has no session; only credential-less Sources run here, `auth_mode: hosted` ones wait for an Agent).
pub fn tick(state: &Arc<AppState>) {
    let Ok(ids) = state.svc.all_workspace_ids() else { return };
    for ws_id in ids {
        // a session's run Workspace refreshes its Sources only while the session runs: afterwards it is a record
        // (第四期规划 §7.1, P4-08: the replay never reads today's data as the class's)
        if let Ok(Some(s)) = state.svc.session_of_run(&ws_id) {
            if !s.is_live() {
                continue;
            }
        }
        let due = state.with_ws(&ws_id, |ws| {
            let owner = ws.owner()?.unwrap_or_default();
            let mut out = Vec::new();
            for id in ws.due_sources()? {
                let (_, source) = ws.source_of(&id)?;
                if source["auth_mode"] == json!("hosted") {
                    continue;
                }
                let idem = format!("source-timer/{id}/{}", (ws.clock)() / 60_000);
                if let Ok(run) = ws.source_refresh_start(&Caller::internal(&owner), &id, &idem) {
                    if run["replayed"] != json!(true) {
                        out.push(run["run_id"].as_str().unwrap_or("").to_string());
                    }
                }
            }
            Ok(out)
        });
        for run_id in due.unwrap_or_default() {
            let (st, w) = (state.clone(), ws_id.clone());
            tokio::runtime::Handle::current().spawn(async move { refresh(st, w, run_id).await });
        }
    }
}

/// Where a Source's content comes from: another Workspace of this service (read directly as the `public`
/// subject), or an HTTP URL fetched without credentials. Returns the content, the bytes it cost, and the
/// content root a fixed Source resolved to.
async fn fetch(state: &Arc<AppState>, ws_id: &str, consumer: &str, source: &Value, principal: &str) -> WsResult<(Fetched, u64, Option<String>)> {
    let own = own_publication(state, source);
    let source = own.as_ref().unwrap_or(source);
    let kind = source["kind"].as_str().unwrap_or("");
    let fixed_root = source["version"]["content_root"].as_str().map(str::to_string);
    if kind == "workspace" {
        let src_ws = source["workspace_id"].as_str().unwrap_or("").to_string();
        let entity = source["entity_id"].as_str().unwrap_or("").to_string();
        let this_zone = source["zone"].as_str().is_none_or(|z| own_zone(state).as_deref() == Some(z));
        let local = this_zone && state.svc.workspace(&src_ws).is_ok();
        if local {
            let referrer = json!({ "workspace_id": ws_id, "entity_id": consumer, "principal": principal });
            let (st, ws, ent, root, fixed) = (state.clone(), src_ws.clone(), entity.clone(), fixed_root.clone(), source["mode"] == json!("fixed"));
            let envelope = tokio::task::spawn_blocking(move || {
                st.with_ws(&ws, |w| {
                    let v = match &root {
                        Some(r) => w.public_fixed_content(&ent, r)?,
                        None => w.public_content(&ent, fixed, true)?,
                    };
                    if let Some((_, entity_id)) = w.publication(&ent)? {
                        let _ = w.register_referrer(&entity_id, &referrer);
                    }
                    Ok(v)
                })
            })
            .await
            .map_err(|e| WsError::io(e.to_string()))??;
            let bytes = envelope.to_string().len() as u64;
            let root = envelope["content_root"].as_str().map(str::to_string);
            return Ok((from_envelope(&envelope)?, bytes, root));
        }
        // another Zone: its publication over HTTP (D24), the entity id as the name (any name resolves by id)
        let zone = source["zone"].as_str().ok_or_else(|| WsError::new(Code::DependencyUnavailable, "the source workspace is not on this service and names no zone"))?;
        let mut url = source["url"].as_str().map(str::to_string).unwrap_or_else(|| format!("https://aiws-{}.{zone}/-/{entity}", host_label(&src_ws)));
        url.push_str(if url.contains('?') { "&format=json" } else { "?format=json" });
        if let Some(r) = &fixed_root {
            url.push_str(&format!("&fixed={r}"));
        } else if source["mode"] == json!("fixed") {
            url.push_str("&fixed=1");
        }
        let (body, _ctype) = http_get(state, &url).await?;
        let envelope: Value = serde_json::from_slice(&body).map_err(|e| WsError::new(Code::DependencyUnavailable, format!("the publication did not answer JSON: {e}")))?;
        if envelope["format"] != json!(PUB_FORMAT) {
            return Err(WsError::new(Code::DependencyUnavailable, "the URL is not an AI Workspace publication"));
        }
        let root = envelope["content_root"].as_str().map(str::to_string);
        // register as a referrer, best effort
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().map_err(|e| WsError::io(e.to_string()))?;
        let base = url.split('?').next().unwrap_or("").to_string();
        let _ = client.post(format!("{base}/referrers")).json(&json!({ "zone": own_zone(state), "workspace_id": ws_id, "entity_id": consumer, "principal": principal })).send().await;
        return Ok((from_envelope(&envelope)?, body.len() as u64, root));
    }
    let url = source["url"].as_str().ok_or_else(|| WsError::invalid_op("source.url required"))?;
    let (body, ctype) = http_get(state, url).await?;
    let bytes = body.len() as u64;
    Ok((parse_body(&body, &ctype, source)?, bytes, None))
}

/// A `kind: url` Source whose URL is a publication of this service — `aiws-<id>.<zonehost>/<workspace>/<name>` or
/// `<zonehost>/kapi/aiworkspace/pub/<ws>/<name>` — is that publication read directly (D26: same Zone, no HTTP).
fn own_publication(state: &Arc<AppState>, source: &Value) -> Option<Value> {
    if source["kind"] != json!("url") {
        return None;
    }
    let url = reqwest::Url::parse(source["url"].as_str()?).ok()?;
    let zone = own_zone(state)?;
    let host = url.host_str()?;
    let segs: Vec<String> = url.path_segments()?.filter(|s| !s.is_empty()).map(|s| percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()).collect();
    let (src_ws, name) = match host.strip_suffix(&format!(".{zone}")).and_then(|h| h.strip_prefix("aiws-")) {
        Some(label) if segs.len() == 2 => (format!("ws_{label}"), segs[1].clone()),
        None if host == zone && segs.len() == 5 && segs[..3] == ["kapi", "aiworkspace", "pub"] => (segs[3].clone(), segs[4].clone()),
        _ => return None,
    };
    state.svc.workspace(&src_ws).ok()?;
    let mut own = json!({ "kind": "workspace", "workspace_id": src_ws, "entity_id": name, "mode": source.get("mode").cloned().unwrap_or(json!("live")) });
    for key in ["version", "name"] {
        if let Some(v) = source.get(key) {
            own[key] = v.clone();
        }
    }
    Some(own)
}

/// This service's Zone host, from the public base `https://aiws-{workspace_id}.<zonehost>`.
fn own_zone(state: &Arc<AppState>) -> Option<String> {
    let probe = reqwest::Url::parse(&state.limits.public_base.as_ref()?.replace("{workspace_id}", "x")).ok()?;
    probe.host_str()?.strip_prefix("aiws-x.").map(str::to_string)
}

/// One GET without credentials, bounded in time and size.
/// Redirects one fetch follows, each hop checked like the first.
const MAX_REDIRECTS: usize = 5;

/// Whether an address is on the public Internet: not loopback, private, link-local, shared (CGNAT), multicast,
/// unspecified, broadcast or documentation space (权限需求 R38).
fn public_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_multicast() || v4.is_unspecified() || v4.is_broadcast() || v4.is_documentation()
                || o[0] == 0 || (o[0] == 100 && (o[1] & 0xc0) == 64) || (o[0] == 192 && o[1] == 0 && o[2] == 0) || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80 || seg[0] == 0x2001 && seg[1] == 0x0db8)
        }
    }
}

/// The address one hop goes to: a public one, resolved now and pinned for the request (no rebinding in between);
/// never the Zone's own hosts. Credentials in the URL are refused earlier (`check_absolute_url`) and here again.
async fn public_address(state: &Arc<AppState>, url: &reqwest::Url) -> WsResult<std::net::SocketAddr> {
    let refused = |m: String| WsError::new(Code::DependencyUnavailable, m);
    if !matches!(url.scheme(), "http" | "https") || !url.username().is_empty() || url.password().is_some() {
        return Err(refused("a Source fetches http(s) URLs without credentials".into()));
    }
    let host = url.host_str().ok_or_else(|| refused("the URL names no host".into()))?.trim_end_matches('.').to_ascii_lowercase();
    if let Some(zone) = own_zone(state) {
        if host == zone || host.ends_with(&format!(".{zone}")) {
            return Err(refused(format!("{host} is this Zone: read its data through a workspace Source, not a URL")));
        }
    }
    if host == "localhost" || host.ends_with(".localhost") {
        return Err(refused(format!("{host} is not a public address")));
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> = match url.host() {
        Some(url::Host::Ipv4(ip)) => vec![std::net::SocketAddr::new(ip.into(), port)],
        Some(url::Host::Ipv6(ip)) => vec![std::net::SocketAddr::new(ip.into(), port)],
        _ => tokio::net::lookup_host((host.as_str(), port)).await.map_err(|e| refused(format!("{host} does not resolve: {e}")))?.collect(),
    };
    let first = *addrs.first().ok_or_else(|| refused(format!("{host} does not resolve")))?;
    if !state.limits.fetch_private {
        if let Some(bad) = addrs.iter().find(|a| !public_ip(a.ip())) {
            return Err(refused(format!("{host} resolves to {} which is not a public address", bad.ip())));
        }
    }
    Ok(first)
}

/// One GET without credentials, bounded in time and size, to public addresses only; redirects are followed by hand so
/// that every hop is checked (R38).
async fn http_get(state: &Arc<AppState>, url: &str) -> WsResult<(Vec<u8>, String)> {
    let unavailable = |m: String| WsError::new(Code::DependencyUnavailable, m);
    let mut current = reqwest::Url::parse(url).map_err(|e| unavailable(format!("invalid URL: {e}")))?;
    let mut hops = 0;
    let resp = loop {
        let addr = public_address(state, &current).await?;
        let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS)).redirect(reqwest::redirect::Policy::none());
        if let Some(url::Host::Domain(name)) = current.host() {
            builder = builder.resolve(name, addr);
        }
        let client = builder.build().map_err(|e| WsError::io(e.to_string()))?;
        let resp = client.get(current.clone()).header(header::ACCEPT, "application/json, text/csv, text/markdown, text/plain, image/*;q=0.9, */*;q=0.5").send().await.map_err(|e| unavailable(format!("fetch failed: {e}")))?;
        if !resp.status().is_redirection() {
            break resp;
        }
        hops += 1;
        if hops > MAX_REDIRECTS {
            return Err(unavailable(format!("more than {MAX_REDIRECTS} redirects")));
        }
        let location = resp.headers().get(header::LOCATION).and_then(|v| v.to_str().ok()).ok_or_else(|| unavailable("a redirect without a location".into()))?;
        current = current.join(location).map_err(|e| unavailable(format!("invalid redirect: {e}")))?;
    };
    if !resp.status().is_success() {
        return Err(unavailable(format!("the source answered HTTP {}", resp.status().as_u16())));
    }
    let ctype = resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    if resp.content_length().is_some_and(|l| l as usize > MAX_FETCH_BYTES) {
        return Err(WsError::limit(format!("the source is larger than {MAX_FETCH_BYTES} bytes")));
    }
    let body = resp.bytes().await.map_err(|e| unavailable(format!("fetch interrupted: {e}")))?;
    if body.len() > MAX_FETCH_BYTES {
        return Err(WsError::limit(format!("the source is larger than {MAX_FETCH_BYTES} bytes")));
    }
    Ok((body.to_vec(), ctype))
}

/// A publication envelope → typed content.
fn from_envelope(v: &Value) -> WsResult<Fetched> {
    let c = &v["content"];
    match v["type_id"].as_str().unwrap_or("") {
        "buckyos.table-source" => Ok(Fetched::Table {
            fields: c["fields"].as_array().cloned().unwrap_or_default(),
            records: c["records"].as_array().cloned().unwrap_or_default(),
        }),
        "buckyos.richtext" => Ok(Fetched::RichText { doc: c["doc"].clone() }),
        "buckyos.asset-ref" => {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD.decode(c["bytes"].as_str().unwrap_or("")).map_err(|e| WsError::new(Code::DependencyUnavailable, format!("asset bytes: {e}")))?;
            Ok(Fetched::Asset { bytes, file_name: c["file_name"].as_str().map(str::to_string) })
        }
        t => Err(WsError::new(Code::DependencyUnavailable, format!("a {t} publication cannot be a Source in this version"))),
    }
}

/// A fetched body by media type: JSON array of objects or CSV → table; Markdown / text → rich text; image → asset.
fn parse_body(body: &[u8], ctype: &str, source: &Value) -> WsResult<Fetched> {
    let ctype = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
    let url = source["url"].as_str().unwrap_or("");
    let ext = url.split('?').next().unwrap_or("").rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if ctype.starts_with("image/") {
        return Ok(Fetched::Asset { bytes: body.to_vec(), file_name: url.split('?').next().and_then(|u| u.rsplit('/').next()).map(str::to_string) });
    }
    let text = std::str::from_utf8(body).map_err(|_| WsError::new(Code::DependencyUnavailable, "the source is not text"))?;
    if ctype.contains("json") || ext == "json" {
        let v: Value = serde_json::from_str(text).map_err(|e| WsError::new(Code::DependencyUnavailable, format!("the source is not JSON: {e}")))?;
        if v["format"] == json!(PUB_FORMAT) {
            return from_envelope(&v);
        }
        let rows = v.as_array().cloned().or_else(|| v["records"].as_array().cloned()).or_else(|| v["rows"].as_array().cloned()).or_else(|| v["data"].as_array().cloned())
            .ok_or_else(|| WsError::new(Code::DependencyUnavailable, "the JSON source is not an array of objects"))?;
        return Ok(table_from_rows(rows.iter().filter_map(|r| r.as_object().map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect())).collect()));
    }
    if ctype == "text/csv" || ext == "csv" {
        return Ok(table_from_rows(parse_csv(text)));
    }
    let prefix = format!("src{}", &aiworkspace_core::canonical::sha256_hex(url.as_bytes())[..6]);
    let doc = to_richtext(text, &prefix, &|_| None)?;
    Ok(Fetched::RichText { doc })
}

/// Rows of `{ column: value }` → a table whose field ids come from the column names and whose record ids
/// follow an `id` column or the row position (stable across refreshes as long as the order is).
fn table_from_rows(rows: Vec<Vec<(String, Value)>>) -> Fetched {
    let get = |r: &Vec<(String, Value)>, k: &str| r.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    let mut columns: Vec<String> = Vec::new();
    for r in &rows {
        for (k, _) in r {
            if !columns.contains(k) {
                columns.push(k.clone());
            }
        }
    }
    let field_id = |name: &str, i: usize| {
        let id = aiworkspace_store::source::pub_name(name);
        if id.is_empty() { format!("f{}", i + 1) } else { id }
    };
    let fields: Vec<Value> = columns.iter().enumerate().map(|(i, name)| {
        let numeric = rows.iter().filter_map(|r| get(r, name)).filter(|v| !v.is_null()).all(|v| v.is_number() || v.as_str().is_some_and(|s| s.trim().parse::<f64>().is_ok()));
        let any = rows.iter().any(|r| get(r, name).is_some_and(|v| !v.is_null()));
        let ty = if numeric && any { "number" } else if rows.iter().filter_map(|r| get(r, name)).all(|v| v.is_boolean() || v.is_null()) && any { "boolean" } else { "text" };
        json!({ "field_id": field_id(name, i), "name": name, "type": ty })
    }).collect();
    let records: Vec<Value> = rows.iter().enumerate().map(|(i, r)| {
        let rid = get(r, "id").and_then(|v| v.as_str().map(str::to_string)).filter(|s| aiworkspace_core::id::is_valid_id(s)).unwrap_or_else(|| format!("r{}", i + 1));
        let mut values = Map::new();
        for (ci, name) in columns.iter().enumerate() {
            let Some(v) = get(r, name) else { continue };
            if v.is_null() { continue }
            let f = &fields[ci];
            let v = match f["type"].as_str() {
                Some("number") => v.as_f64().map(|n| json!(n)).or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()).map(|n| json!(n))).unwrap_or(Value::Null),
                Some("boolean") => v.clone(),
                _ => match &v { Value::String(s) => json!(s), other => json!(other.to_string()) },
            };
            if !v.is_null() {
                values.insert(f["field_id"].as_str().unwrap_or("").to_string(), v);
            }
        }
        json!({ "record_id": rid, "values": values })
    }).collect();
    Fetched::Table { fields, records }
}

/// Minimal CSV (RFC 4180 quoting), first row = header.
fn parse_csv(text: &str) -> Vec<Vec<(String, Value)>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let (mut row, mut cell, mut quoted) = (Vec::new(), String::new(), false);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, quoted) {
            ('"', true) => { if chars.peek() == Some(&'"') { cell.push('"'); chars.next(); } else { quoted = false } }
            ('"', false) => quoted = true,
            (',', false) => { row.push(std::mem::take(&mut cell)); }
            ('\r', false) => {}
            ('\n', false) => { row.push(std::mem::take(&mut cell)); rows.push(std::mem::take(&mut row)); }
            (c, _) => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    let mut it = rows.into_iter().filter(|r| !(r.len() == 1 && r[0].is_empty()));
    let Some(header) = it.next() else { return Vec::new() };
    it.map(|r| header.iter().zip(r.into_iter().chain(std::iter::repeat(String::new()))).map(|(h, v)| (h.trim().to_string(), json!(v))).collect::<Vec<(String, Value)>>()).collect()
}

// ---- the publication HTTP surface (T4-4, T4-9)

/// `https://aiws-<label>.<zonehost>` when the deployment knows its host. The host label is the workspace id without
/// its `ws_` prefix (`aiws-2611a0b3.<zone>`): a DNS label carries no `_`, and the gateway folds only
/// `aiws-[a-z0-9]+` onto the service.
pub fn public_base(state: &Arc<AppState>, ws_id: &str) -> Option<String> {
    state.limits.public_base.as_ref().map(|b| b.replace("{workspace_id}", host_label(ws_id)))
}

use aiworkspace_core::id::workspace_host_label as host_label;

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The published content as a page a crawler can read (T4-9): title, then the table or the text.
fn prerender(v: &Value) -> String {
    let title = html_escape(v["title"].as_str().unwrap_or(v["name"].as_str().unwrap_or("")));
    let body = match v["type_id"].as_str().unwrap_or("") {
        "buckyos.table-source" => {
            let fields: Vec<&Value> = v["content"]["fields"].as_array().into_iter().flatten().collect();
            let mut h = String::from("<table><thead><tr>");
            for f in &fields {
                h.push_str(&format!("<th>{}</th>", html_escape(f["name"].as_str().unwrap_or(""))));
            }
            h.push_str("</tr></thead><tbody>");
            for r in v["content"]["records"].as_array().into_iter().flatten() {
                h.push_str("<tr>");
                for f in &fields {
                    let cell = &r["values"][f["field_id"].as_str().unwrap_or("")];
                    let text = match cell { Value::String(s) => s.clone(), Value::Null => String::new(), other => other.to_string() };
                    h.push_str(&format!("<td>{}</td>", html_escape(&text)));
                }
                h.push_str("</tr>");
            }
            h.push_str("</tbody></table>");
            h
        }
        "buckyos.richtext" => from_richtext(&v["content"]["doc"]).split("\n\n").filter(|p| !p.trim().is_empty()).map(|p| format!("<p>{}</p>", html_escape(p))).collect::<Vec<_>>().join("\n"),
        _ => format!("<pre>{}</pre>", html_escape(&serde_json::to_string_pretty(&v["content"]).unwrap_or_default())),
    };
    format!("<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title><meta name=\"description\" content=\"{title}\"><meta name=\"robots\" content=\"index,follow\"></head><body><article><h1>{title}</h1>\n{body}\n</article></body></html>")
}

fn wants_html(headers: &HeaderMap, q: &HashMap<String, String>) -> bool {
    if q.get("format").is_some_and(|f| f == "json") {
        return false;
    }
    headers.get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html"))
}

pub(crate) async fn published(state: &Arc<AppState>, ws_id: &str, name: &str, q: &HashMap<String, String>, headers: &HeaderMap) -> Response {
    let fixed = q.get("fixed").cloned();
    let (st, w, n) = (state.clone(), ws_id.to_string(), name.to_string());
    let result = tokio::task::spawn_blocking(move || {
        st.with_ws(&w, |ws| match fixed.as_deref() {
            Some(root) if root != "1" => {
                let mut v = ws.public_fixed_content(&n, root)?;
                v["name"] = json!(n);
                Ok(v)
            }
            Some(_) => ws.public_content(&n, true, true),
            None => ws.public_content(&n, false, true),
        })
    })
    .await
    .unwrap_or_else(|e| Err(WsError::io(e.to_string())));
    match result {
        Ok(mut v) => {
            if let Some(base) = public_base(state, ws_id) {
                v["url"] = json!(format!("{base}/{}/{}", v["workspace_id"].as_str().map(|_| state.with_ws(ws_id, |w| Ok(w.workspace_slug())).unwrap_or_default()).unwrap_or_default(), name));
            }
            if wants_html(headers, q) {
                ([(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()), (header::CACHE_CONTROL, "public, max-age=60".to_string())], prerender(&v)).into_response()
            } else {
                ([(header::CONTENT_TYPE, "application/json".to_string()), (header::CACHE_CONTROL, "public, max-age=60".to_string()), (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*".to_string())], v.to_string()).into_response()
            }
        }
        Err(e) => {
            let status = match e.code { Code::NotFound | Code::PermissionDenied | Code::VersionNotRetained => StatusCode::NOT_FOUND, _ => StatusCode::INTERNAL_SERVER_ERROR };
            (status, Json(json!({ "ok": false, "error": e.to_json() }))).into_response()
        }
    }
}

/// `GET /kapi/aiworkspace/pub/{workspace_id}/{name}`: the publication (JSON, or HTML for a browser / crawler).
pub async fn pub_get(State(state): State<Arc<AppState>>, Path((ws_id, name)): Path<(String, String)>, Query(q): Query<HashMap<String, String>>, headers: HeaderMap) -> Response {
    published(&state, &ws_id, &name, &q, &headers).await
}

/// `POST /kapi/aiworkspace/pub/{workspace_id}/{name}/referrers { zone?, workspace_id, principal? }`: a consumer in
/// another Zone registers that it reads the publication (weak, T4-6).
pub async fn pub_referrer(State(state): State<Arc<AppState>>, Path((ws_id, name)): Path<(String, String)>, Json(body): Json<Value>) -> Response {
    if !body.get("workspace_id").is_some_and(Value::is_string) {
        return (StatusCode::BAD_REQUEST, "workspace_id required").into_response();
    }
    let result = tokio::task::spawn_blocking(move || {
        state.with_ws(&ws_id, |ws| {
            let (_, entity_id) = ws.publication(&name)?.ok_or_else(|| WsError::not_found("no such publication"))?;
            ws.register_referrer(&entity_id, &json!({ "zone": body["zone"], "workspace_id": body["workspace_id"], "entity_id": body["entity_id"], "principal": body["principal"], "registered_at": format_utc_ms((ws.clock)()) }))
        })
    })
    .await
    .unwrap_or_else(|e| Err(WsError::io(e.to_string())));
    match result {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::NOT_FOUND, Json(json!({ "ok": false, "error": e.to_json() }))).into_response(),
    }
}

/// The workspace id of an `aiws-<id>.<zonehost>` host, from the forwarded host (HTTP/2 carries it as :authority).
pub(crate) fn host_workspace(headers: &HeaderMap) -> Option<String> {
    let host = headers.get("x-forwarded-host").or_else(|| headers.get(header::HOST)).and_then(|v| v.to_str().ok())?;
    let host = host.split(':').next().unwrap_or("");
    let label = host.strip_prefix("aiws-")?.split('.').next()?;
    let id = format!("ws_{label}");
    aiworkspace_core::id::is_workspace_id(&id).then_some(id)
}
