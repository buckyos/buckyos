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

/// `proc.start { program: source.refresh@1, params: { entity_id } }`: the run is created and refreshed on a worker; a
/// Source with a sync task has its Agent run the task first.
pub fn start(state: &Arc<AppState>, caller: &Caller, ws_id: &str, params: &Value, idem: &str) -> WsResult<Value> {
    let entity_id = params.get("entity_id").and_then(Value::as_str).ok_or_else(|| WsError::invalid_op("entity_id required"))?.to_string();
    let run = state.with_ws(ws_id, |ws| ws.source_refresh_start(caller, &entity_id, idem))?;
    if run["replayed"] != json!(true) {
        let (st, w, rid) = (state.clone(), ws_id.to_string(), run["run_id"].as_str().unwrap_or("").to_string());
        tokio::runtime::Handle::current().spawn(async move { refresh(st, w, rid, true).await });
    }
    Ok(run)
}

/// Fetch, then apply (one Commit of origin `source`); a failure leaves the entity as it was and the run says why.
/// `run_sync`: a triggered refresh of a Source with `sync` runs the task on its Agent before it reads the output; the
/// timer only reads it again (the Agent host keeps the task's schedule).
pub async fn refresh(state: Arc<AppState>, ws_id: String, run_id: String, run_sync: bool) {
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
    let outcome = async {
        if let Some(sync) = source.get("sync").filter(|s| run_sync && s.is_object()) {
            run_sync_task(&state, &ws_id, sync).await?;
        }
        fetch(&state, &ws_id, &consumer, &source, &principal).await
    }
    .await;
    let (st, w, r) = (state.clone(), ws_id.clone(), run_id.clone());
    let _ = tokio::task::spawn_blocking(move || {
        st.with_ws(&w, |ws| match outcome {
            Ok(Got { fetched, bytes, root, version }) => {
                if let Err(e) = ws.add_traffic(bytes) {
                    ws.source_fail(&r, &e)?;
                    return Err(e);
                }
                ws.source_apply(&r, &fetched, bytes, root.as_deref(), &version).map(|_| ())
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
            tokio::runtime::Handle::current().spawn(async move { refresh(st, w, run_id, false).await });
        }
    }
}

/// How long a triggered refresh waits for a sync task's run (`sync.run { wait_ms }`, at most what the Agent host allows).
const SYNC_WAIT_MS: u64 = 120_000;

/// `source.sync { agent_id, task_id }` (余下任务 §8.2): the workspace owner's Agent runs the task now — `sync.run` through
/// the Agent gateway with this service's own session, which the Agent accepts for `sync.run` / `sync.get` — and the
/// refresh then reads the output file. An Agent that is not the current owner's, unreachable, or refusing (a disabled
/// task) fails the refresh with `SYNC_UNAVAILABLE`; a run that failed fails it with the program's reason. Either way the
/// data stays at its last valid version.
async fn run_sync_task(state: &Arc<AppState>, ws_id: &str, sync: &Value) -> WsResult<()> {
    let unavailable = |m: String| WsError::sub(Code::DependencyUnavailable, "SYNC_UNAVAILABLE", m);
    let (agent_id, task_id) = (sync["agent_id"].as_str().unwrap_or(""), sync["task_id"].as_str().unwrap_or(""));
    let base = state.limits.agent_gateway.clone().ok_or_else(|| unavailable("no Agent is reachable from this deployment: the sync task cannot run".into()))?;
    let (st, w) = (state.clone(), ws_id.to_string());
    let owner = tokio::task::spawn_blocking(move || st.with_ws(&w, |ws| ws.owner())).await.map_err(|e| WsError::io(e.to_string()))??.unwrap_or_default();
    let agent = buckyos_api::AgentId::parse(agent_id).map_err(|e| unavailable(format!("{agent_id} is no Agent id: {e}")))?;
    let as_agent = state.auth.caller_for(&agent.agent_did().to_string()).await;
    if !as_agent.acting_for.as_ref().is_some_and(|o| o.principal == owner) {
        return Err(unavailable(format!("{agent_id} is not an Agent of this workspace's owner ({owner})")));
    }
    let token = match buckyos_api::get_buckyos_api_runtime() {
        Ok(runtime) => Some(runtime.get_session_token().await),
        Err(_) => None,
    };
    let client = kRPC::kRPC::new_with_timeout_secs(&format!("{base}/{agent_id}"), token, SYNC_WAIT_MS / 1000 + 30);
    let answer = client
        .call("sync.run", json!({ "task_id": task_id, "wait_ms": SYNC_WAIT_MS }))
        .await
        .map_err(|e| unavailable(format!("the sync task {task_id} of {agent_id} did not run: {e}")))?;
    let run = &answer["run"];
    if run["status"] == json!("failed") {
        let why = match &run["error"] {
            Value::String(s) => s.clone(),
            Value::Null => "no reason given".to_string(),
            other => other.to_string(),
        };
        return Err(WsError::new(Code::DependencyUnavailable, format!("the sync program failed: {}", why.chars().take(2000).collect::<String>())).with_data(json!({ "run": run })));
    }
    Ok(())
}

/// What one fetch got: the content, the bytes it cost, the content root a fixed Source resolved to, and what the
/// source said of the data (`business_time`, `produced_at`, `program`, `truncated`: the parts of `data_version` a
/// refresh does not know by itself).
struct Got {
    fetched: Fetched,
    bytes: u64,
    root: Option<String>,
    version: Value,
}

/// Where a Source's content comes from: another Workspace of this service (read directly as the `public`
/// subject), an HTTP URL fetched without credentials, or a file of the workspace owner's home. A table's rows are then
/// cut to `def.limit` and identified by its primary key (B5-1, B7-3).
async fn fetch(state: &Arc<AppState>, ws_id: &str, consumer: &str, source: &Value, principal: &str) -> WsResult<Got> {
    let (fetched, bytes, root, mut version) = fetch_raw(state, ws_id, consumer, source, principal).await?;
    let def = &source["def"];
    // a publication's rows keep the publisher's record ids; any other table is identified by its key here
    let identified = |records: &[Value]| !records.is_empty() && records.iter().all(|r| r["record_id"].is_string());
    let fetched = match fetched {
        Fetched::Table { fields, records } if source["kind"] != json!("workspace") && !identified(&records) => {
            let (records, truncated) = keyed(&fields, records, def)?;
            if truncated {
                version["truncated"] = json!(true);
            }
            Fetched::Table { fields, records }
        }
        Fetched::Table { fields, mut records } => {
            if let Some(n) = def["limit"].as_u64().filter(|n| records.len() as u64 > *n) {
                records.truncate(n as usize);
                version["truncated"] = json!(true);
            }
            Fetched::Table { fields, records }
        }
        other => other,
    };
    Ok(Got { fetched, bytes, root, version })
}

/// The rows of a fetched table cut to `def.limit` (file order) and identified by the primary key: `def.primary_key`,
/// else an `id` column; each `record_id` is [`record_id_for_key`] of the key values. A row without its key, or two rows
/// with one key, fail the refresh and write nothing. With neither, the rows have no `record_id`: the refresh numbers
/// them (owner 2026-11-16: an auto-increment id when there is no suitable key, `Workspace::source_ops`).
fn keyed(fields: &[Value], mut rows: Vec<Value>, def: &Value) -> WsResult<(Vec<Value>, bool)> {
    let truncated = def["limit"].as_u64().is_some_and(|n| rows.len() as u64 > n);
    if let Some(n) = def["limit"].as_u64() {
        rows.truncate(n as usize);
    }
    let key: Vec<String> = match def["primary_key"].as_array() {
        Some(k) => k.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None if fields.iter().any(|f| f["field_id"] == json!("id")) => vec!["id".to_string()],
        None => return Ok((rows, truncated)),
    };
    if let Some(k) = key.iter().find(|k| !fields.iter().any(|f| f["field_id"] == json!(k))) {
        return Err(WsError::sub(Code::InvalidSchema, "PRIMARY_KEY_REQUIRED", format!("the primary key field {k} is not among the source's fields")).with_data(json!({ "field_id": k })));
    }
    let mut seen = std::collections::BTreeMap::new();
    for (i, r) in rows.iter_mut().enumerate() {
        let values: Vec<Value> = key.iter().map(|k| r["values"].get(k).cloned().unwrap_or(Value::Null)).collect();
        if values.iter().any(|v| v.is_null() || v.as_str() == Some("")) {
            return Err(WsError::sub(Code::InvalidSchema, "PRIMARY_KEY_REQUIRED", format!("row {} has no value for the primary key {}", i + 1, key.join(", "))).with_data(json!({ "row": i + 1 })));
        }
        let id = aiworkspace_core::wish::record_id_for_key(&values);
        if let Some(first) = seen.insert(id.clone(), i + 1) {
            return Err(WsError::sub(Code::InvalidSchema, "DUPLICATE_KEY", format!("rows {first} and {} have the same primary key {}", i + 1, json!(values))).with_data(json!({ "rows": [first, i + 1], "key": values })));
        }
        r["record_id"] = json!(id);
    }
    Ok((rows, truncated))
}

async fn fetch_raw(state: &Arc<AppState>, ws_id: &str, consumer: &str, source: &Value, principal: &str) -> WsResult<(Fetched, u64, Option<String>, Value)> {
    if source["kind"] == json!("zone") {
        return zone_file(state, ws_id, consumer, source).await;
    }
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
            return Ok((from_envelope(&envelope)?, bytes, root, json!({})));
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
        return Ok((from_envelope(&envelope)?, body.len() as u64, root, json!({})));
    }
    let url = source["url"].as_str().ok_or_else(|| WsError::invalid_op("source.url required"))?;
    let (body, ctype) = http_get(state, url).await?;
    let bytes = body.len() as u64;
    Ok((parse_body(&body, &ctype, url)?, bytes, None, json!({})))
}

/// The format of what a sync program leaves for a Zone source (B3-1, “pull”).
pub const SYNC_FORMAT: &str = "aiws.sync.v1";

/// A file of the workspace owner's home (`source.path` = `/home/<owner>/…`), read as that owner (B3-1): only the
/// current owner's home — a copied workspace reads its new owner's — never through a link out of it, at most the
/// fetch's size and time. The file is plain JSON / CSV / Markdown (as a URL's body), or a sync program's
/// `aiws.sync.v1` output: `status: failed` fails the refresh with the program's own reason.
async fn zone_file(state: &Arc<AppState>, ws_id: &str, consumer: &str, source: &Value) -> WsResult<(Fetched, u64, Option<String>, Value)> {
    let unavailable = |m: String| WsError::new(Code::DependencyUnavailable, m);
    let path = source["path"].as_str().unwrap_or("").to_string();
    let segs: Vec<String> = aiworkspace_core::types::zone_path_segments(&path)?.into_iter().map(str::to_string).collect();
    let (st, w) = (state.clone(), ws_id.to_string());
    let owner = tokio::task::spawn_blocking(move || st.with_ws(&w, |ws| ws.owner())).await.map_err(|e| WsError::io(e.to_string()))??.unwrap_or_default();
    if segs[1] != owner {
        return Err(WsError::denied(format!("{path} is not in the home of this workspace's owner ({owner}): set the source to a file of /home/{owner}/")));
    }
    let root = state.limits.zone_root.clone().ok_or_else(|| unavailable("Zone files are not readable on this deployment".into()))?;
    let read = tokio::task::spawn_blocking(move || -> WsResult<Vec<u8>> {
        let home = root.join("home").join(&segs[1]);
        let file = segs[2..].iter().fold(home.clone(), |p, s| p.join(s));
        let missing = |e: std::io::Error| unavailable(format!("{path} cannot be read: {e}"));
        let (home, file) = (home.canonicalize().map_err(missing)?, file.canonicalize().map_err(missing)?);
        if !file.starts_with(&home) || !file.is_file() {
            return Err(unavailable(format!("{path} is not a file of the owner's home")));
        }
        let len = std::fs::metadata(&file).map_err(missing)?.len();
        if len as usize > MAX_FETCH_BYTES {
            return Err(WsError::limit(format!("{path} is larger than {MAX_FETCH_BYTES} bytes")));
        }
        std::fs::read(&file).map_err(missing)
    });
    let body = tokio::time::timeout(std::time::Duration::from_secs(FETCH_TIMEOUT_SECS), read).await.map_err(|_| unavailable(format!("reading the file took longer than {FETCH_TIMEOUT_SECS} s")))?.map_err(|e| WsError::io(e.to_string()))??;
    let bytes = body.len() as u64;
    let name = source["path"].as_str().unwrap_or("");
    let envelope = name.to_ascii_lowercase().ends_with(".json").then(|| serde_json::from_slice::<Value>(&body).ok()).flatten().filter(|v| v["format"] == json!(SYNC_FORMAT));
    let Some(env) = envelope else { return Ok((parse_body(&body, "", name)?, bytes, None, json!({}))) };
    let program = env["program"].clone();
    match env["status"].as_str() {
        Some("ok") => {}
        Some("failed") => {
            let why = env["error"].as_str().unwrap_or("the sync program reported a failure").chars().take(2000).collect::<String>();
            return Err(unavailable(format!("the sync program failed: {why}")).with_data(json!({ "program": program, "produced_at": env["produced_at"] })));
        }
        _ => return Err(WsError::invalid_schema(format!("{name}: {SYNC_FORMAT} needs status ok or failed"))),
    }
    let records: Vec<Value> = env["records"].as_array().ok_or_else(|| WsError::invalid_schema(format!("{name}: {SYNC_FORMAT} needs records (a list of objects)")))?
        .iter().map(|r| r.as_object().map(|o| json!({ "values": o })).ok_or_else(|| WsError::invalid_schema(format!("{name}: every record is an object of field id → value")))).collect::<WsResult<_>>()?;
    let fields: Vec<Value> = match env.get("fields").filter(|f| !f.is_null()) {
        Some(f) => f.as_array().cloned().ok_or_else(|| WsError::invalid_schema(format!("{name}: fields must be a list of field definitions")))?,
        None => {
            let (st, w, id) = (state.clone(), ws_id.to_string(), consumer.to_string());
            tokio::task::spawn_blocking(move || st.with_ws(&w, |ws| ws.read_ctx(|ctx| Ok(ctx.fields(&id)?.into_iter().filter(|f| f.alive()).map(|f| f.def.to_value()).collect::<Vec<_>>()))))
                .await.map_err(|e| WsError::io(e.to_string()))??
        }
    };
    let mut version = json!({ "program": program });
    for k in ["business_time", "produced_at"] {
        if let Some(t) = env[k].as_str().filter(|t| aiworkspace_core::value::parse_datetime_ms(t).is_some()) {
            version[k] = json!(t);
        }
    }
    if env["truncated"] == json!(true) {
        version["truncated"] = json!(true);
    }
    Ok((Fetched::Table { fields, records }, bytes, None, version))
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

/// A fetched body by media type (or the extension of `url`, a URL or a Zone path): JSON array of objects or CSV →
/// table; Markdown / text → rich text; image → asset.
fn parse_body(body: &[u8], ctype: &str, url: &str) -> WsResult<Fetched> {
    let ctype = ctype.split(';').next().unwrap_or("").trim().to_ascii_lowercase();
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

/// Rows of `{ column: value }` → a table whose field ids come from the column names; [`keyed`] gives the record ids.
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
    let records: Vec<Value> = rows.iter().map(|r| {
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
        json!({ "values": values })
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
