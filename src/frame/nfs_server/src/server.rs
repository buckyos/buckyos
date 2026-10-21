//! HTTP binding (NFSP §4): control plane `POST /nfs/v1/{method}`, batch,
//! watch SSE, the data-plane read endpoint, and the minimal tus upload area.

use crate::acl::{Actor, Level};
use crate::auth::{authenticate, Credentials};
use crate::error::*;
use crate::fsutil::{content_type_for, join_root};
use crate::namespace::{ListArgs, Node};
use crate::state::SharedState;
use crate::types::*;
use axum::body::{Body, Bytes};
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, options, patch, post, put};
use axum::Router;
use futures_util::stream::{self, Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;

const WRITE_METHODS: &[&str] = &[
    "mkdir",
    "move",
    "delete",
    "open_write",
    "abort_write",
    "commit_file",
    "bind_ref",
    "unlink",
    "set_meta",
    "create_collection",
    "collection_patch",
    "grant",
    "revoke",
    "set_acl",
];

pub fn build_router(state: SharedState) -> Router {
    let mut router = Router::new()
        .route("/nfs/v1/watch", get(watch_handler))
        .route("/nfs/v1/read/{node_id}", get(read_handler))
        .route(
            "/nfs/v1/uploads/{fb}",
            patch(upload_patch).head(upload_head).layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route("/nfs/v1/{method}", post(control_handler))
        .route("/s/{token}", get(crate::share::serving::page))
        .route("/s/{token}/meta", get(crate::share::serving::meta))
        .route("/s/{token}/unlock", post(crate::share::serving::unlock))
        .route("/s/{token}/d", get(crate::share::serving::download_root))
        .route("/s/{token}/d/{*inner}", get(crate::share::serving::download_inner))
        .route("/ndm/v1/uploads", options(crate::ndm::uploads::options).post(crate::ndm::uploads::create))
        .route(
            "/ndm/v1/uploads/{id}",
            patch(crate::ndm::uploads::patch)
                .head(crate::ndm::uploads::head)
                .delete(crate::ndm::uploads::terminate)
                .post(crate::ndm::uploads::post_override)
                .layer(axum::extract::DefaultBodyLimit::disable()),
        )
        .route("/ndm/v1/objects/lookup", get(crate::ndm::forward::lookup))
        .route("/ndm/v1/store/{method}", post(crate::ndm::forward::store_api))
        .route("/ndm/proxy/v1/rpc/{method}", post(crate::ndm::forward::proxy_rpc))
        .route("/ndm/proxy/v1/read/{*kind}", post(crate::ndm::forward::proxy_read))
        .route(
            "/ndm/proxy/v1/write/chunk/{chunk_id}",
            put(crate::ndm::forward::proxy_write).layer(axum::extract::DefaultBodyLimit::disable()),
        );
    if state.config.debug_api {
        router = router.route("/nfs/v1/debug/{action}", post(debug_handler));
    }
    router
        .layer(axum::middleware::from_fn_with_state(state.clone(), crate::share::serving::host_guard))
        .with_state(state)
}

// ---------- control plane ----------

async fn control_handler(
    State(state): State<SharedState>,
    AxPath(method): AxPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut env: Envelope = match serde_json::from_slice(&body) {
        Ok(e) => e,
        Err(e) => {
            return envelope_error(&state, invalid(format!("bad request envelope: {}", e)))
        }
    };
    if env.args.is_null() {
        env.args = json!({});
    }
    // Unknown critical extensions must be rejected (NFSP §4.4).
    for ext in &env.ext {
        if ext.critical {
            return envelope_error(
                &state,
                NfsError::new(
                    ErrorCode::UnsupportedExt,
                    format!("unknown critical extension '{}'", ext.id),
                ),
            );
        }
    }
    let creds = Credentials::from_headers(&headers);
    let actor = match authenticate(&state, &creds).await {
        Ok(a) => a,
        Err(e) => return envelope_error(&state, e),
    };
    if !(method == "hello" || method == "bye" || method.starts_with("share.")) {
        if let Err(e) = crate::auth::require_file_actor(&actor) {
            return envelope_error(&state, e);
        }
    }
    if method == "hello" {
        return match op_hello(&state, &env, actor) {
            Ok(v) => envelope_ok(&state, v),
            Err(e) => envelope_error(&state, e),
        };
    }
    let session = match session_for(&state, env.session.as_deref(), &actor) {
        Ok(s) => s,
        Err(e) => return envelope_error(&state, e),
    };
    if method.starts_with("copy_") {
        return match crate::copy::dispatch(&state, &actor, &method, &env.args, creds.bearer()).await {
            Ok(value) => envelope_ok(&state, value),
            Err(error) => envelope_error(&state, error),
        };
    }
    // Exactly-once for write ops (NFSP §6.3).
    let is_write = WRITE_METHODS.contains(&method.as_str())
        || crate::share::rpc::WRITE_METHODS.contains(&method.as_str());
    if is_write {
        let seq = match env.seq {
            Some(s) => s,
            None => {
                return envelope_error(
                    &state,
                    invalid(format!("'{}' is a write op and requires seq", method)),
                )
            }
        };
        match state.sessions.check_seq(&session, seq) {
            Ok(crate::session::SeqDisposition::Execute) => {}
            Ok(crate::session::SeqDisposition::Replay(cached)) => {
                return envelope_ok(&state, cached);
            }
            Err(e) => return envelope_error(&state, e),
        }
        let result = dispatch(&state, &actor, &method, &env, &session, creds.bearer()).await;
        return match result {
            Ok(v) => {
                state.sessions.record_seq(&session, env.seq.unwrap(), v.clone());
                envelope_ok(&state, v)
            }
            Err(e) => envelope_error(&state, e),
        };
    }
    match dispatch(&state, &actor, &method, &env, &session, creds.bearer()).await {
        Ok(v) => envelope_ok(&state, v),
        Err(e) => envelope_error(&state, e),
    }
}

/// A hello session only serves the principal that created it.
fn session_for(state: &SharedState, session: Option<&str>, actor: &Actor) -> NfsResult<String> {
    let session = session.ok_or_else(|| {
        NfsError::new(ErrorCode::PermissionDenied, "missing session; call hello first")
    })?;
    match state.sessions.actor(session) {
        Some(owner) if &owner == actor => Ok(session.to_string()),
        Some(_) => Err(NfsError::new(
            ErrorCode::PermissionDenied,
            "session belongs to another principal; hello again",
        )
        .with("reason", json!("session_principal_changed"))),
        None => Err(NfsError::new(
            ErrorCode::PermissionDenied,
            "unknown or expired session; hello again",
        )),
    }
}

fn envelope_ok(state: &SharedState, result: Value) -> Response {
    let body = json!({
        "ok": true,
        "result": result,
        "server_rev": state.bus.server_rev(),
    });
    (StatusCode::OK, axum::Json(body)).into_response()
}

fn envelope_error(state: &SharedState, e: NfsError) -> Response {
    let status = StatusCode::from_u16(e.code.http_status()).unwrap_or(StatusCode::BAD_REQUEST);
    let body = json!({
        "ok": false,
        "error": e.to_json(),
        "server_rev": state.bus.server_rev(),
    });
    (status, axum::Json(body)).into_response()
}

fn op_hello(state: &SharedState, env: &Envelope, actor: Actor) -> NfsResult<Value> {
    if let Some(versions) = env.args["versions"].as_array() {
        if !versions.iter().any(|v| v.as_str() == Some(PROTOCOL_VERSION)) {
            return Err(NfsError::new(
                ErrorCode::Unsupported,
                format!("no common protocol version (server speaks {})", PROTOCOL_VERSION),
            ));
        }
    }
    let client_features: Vec<String> = env.args["features"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let features: Vec<&str> = SERVER_FEATURES
        .iter()
        .copied()
        .filter(|f| client_features.is_empty() || client_features.iter().any(|c| c == f))
        .collect();
    let identity = json!({
        "user_id": actor.user_id(),
        "role": match &actor {
            Actor::Admin(_) => "admin",
            Actor::User(_) => "user",
            Actor::Delegated { .. } => "delegated",
            Actor::Anonymous => "anonymous",
        },
    });
    if let Err(e) = state.ensure_home(&actor) {
        log::warn!("create home for {:?} failed: {}", actor.user_id(), e.message);
    }
    let session = state.sessions.create(actor);
    Ok(json!({
        "version": PROTOCOL_VERSION,
        "session": session,
        "identity": identity,
        "features": features,
        "limits": {
            "max_batch": state.config.max_batch,
            "max_list": state.config.max_list,
            "replay_window": state.config.replay_window,
            "attr_ttl_ms": 5000,
        },
        "realms": [ {"id": "dfs", "writable": true} ],
    }))
}

async fn dispatch(
    state: &SharedState,
    actor: &Actor,
    method: &str,
    env: &Envelope,
    session: &str,
    bearer: Option<&str>,
) -> NfsResult<Value> {
    let want = WantMask::from_opt(&env.want);
    let at = env.at.as_ref();
    let args = &env.args;
    if method.starts_with("share.") {
        return crate::share::rpc::dispatch(state, actor, method, args, bearer).await;
    }
    match method {
        "bye" => {
            state.leases.release_session(session);
            state.sessions.remove(session);
            Ok(json!({}))
        }
        "resolve" | "stat" => {
            let at = at.ok_or_else(|| invalid("resolve/stat requires at"))?;
            let mut node = state.resolve_for(actor, at, Level::Traverse, "read")?;
            if let Some(name) = args["name"].as_str() {
                node = state.walk_child_for(actor, &node, name, "read")?;
            }
            state.node_info(actor, &node, &want)
        }
        "list" => {
            let at = at.ok_or_else(|| invalid("list requires at"))?;
            let node = state.resolve_for(actor, at, Level::Traverse, "read")?;
            let list_args: ListArgs = serde_json::from_value(args.clone())
                .map_err(|e| invalid(format!("bad list args: {}", e)))?;
            state.list_node(actor, &node, &list_args, &want)
        }
        "batch" => op_batch(state, actor, env).await,
        "mkdir" => state.op_mkdir(actor, at, args),
        "move" => state.op_move(actor, args),
        "delete" => state.op_delete(actor, at, args),
        "bind_ref" => state.op_bind_ref(actor, args),
        "unlink" => state.op_unlink(actor, args),
        "abort_write" => state.op_abort_write(args, session),
        "open_write" => state.op_open_write(actor, args, session),
        "commit_file" => state.op_commit_file(actor, at, args, session),
        "probe" => state.op_probe(actor, args),
        "get_meta" => state.op_get_meta(actor, at, args),
        "set_meta" => state.op_set_meta(actor, at, args),
        "search" => state.op_search(actor, args, &want),
        "open_view" => state.op_open_view(actor, args, &want),
        "create_collection" => state.op_create_collection(actor, args, &want),
        "open_collection" => state.op_open_collection(actor, args, &want),
        "collection_patch" => state.op_collection_patch(actor, args),
        "grant" => state.op_grant(actor, args),
        "revoke" => state.op_revoke(actor, args),
        "get_acl" => state.op_get_acl(actor, at, args),
        "set_acl" => state.op_set_acl(actor, at, args),
        "list_acls" => state.op_list_acls(actor),
        other => Err(NfsError::new(
            ErrorCode::Unsupported,
            format!("unknown method '{}'", other),
        )),
    }
}

// ---------- batch (COMPOUND-lite, §4.3) ----------

async fn op_batch(state: &SharedState, actor: &Actor, env: &Envelope) -> NfsResult<Value> {
    let args = &env.args;
    let start: Locator = match args.get("start") {
        Some(s) if !s.is_null() => serde_json::from_value(s.clone())
            .map_err(|_| invalid("bad batch start locator"))?,
        _ => env.at.clone().ok_or_else(|| invalid("batch requires start or at"))?,
    };
    let ops = args["ops"].as_array().ok_or_else(|| invalid("batch requires ops[]"))?;
    if ops.len() > state.config.max_batch {
        return Err(invalid(format!("batch too large (max {})", state.config.max_batch)));
    }
    let abort = args["on_error"].as_str().unwrap_or("abort") != "continue";
    let mut cursor: Node = state.resolve_for(actor, &start, Level::Traverse, "read")?;
    let mut results: Vec<Value> = Vec::new();
    let mut completed = 0usize;
    for op in ops {
        let m = op["m"].as_str().unwrap_or("");
        let want = WantMask::from_opt(&op.get("want").map(|w| {
            w.as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default()
        }));
        let op_args = match op.get("args") {
            Some(v) if !v.is_null() => v.clone(),
            _ => json!({}),
        };
        let r: NfsResult<Value> = match m {
            "walk" => {
                let step: NfsResult<Node> = if let Some(names) = op_args["names"].as_array() {
                    let mut cur = cursor.clone();
                    let mut out = Err(invalid("empty names"));
                    for n in names {
                        let name =
                            n.as_str().ok_or_else(|| invalid("bad walk name"))?;
                        cur = state.walk_child_for(actor, &cur, name, "read")?;
                        out = Ok(cur.clone());
                    }
                    out
                } else if let Some(name) = op_args["name"].as_str() {
                    state.walk_child_for(actor, &cursor, name, "read")
                } else if let Some(er) = op_args["entry_ref"].as_str() {
                    state.walk_entry_ref(&cursor, er).and_then(|n| {
                        state.require(actor, &n, Level::Traverse, "read")?;
                        Ok(n)
                    })
                } else {
                    Err(invalid("walk requires names, name or entry_ref"))
                };
                step.map(|node| {
                    cursor = node;
                    json!({"ref": state.node_ref(&cursor), "kind": cursor.kind()})
                })
            }
            "stat" | "resolve" => {
                let node = if let Some(name) = op_args["name"].as_str() {
                    state.walk_child_for(actor, &cursor, name, "read")
                } else {
                    Ok(cursor.clone())
                };
                node.and_then(|n| state.node_info(actor, &n, &want))
            }
            "list" => {
                let list_args: ListArgs = serde_json::from_value(op_args.clone())
                    .map_err(|e| invalid(format!("bad list args: {}", e)))?;
                state.list_node(actor, &cursor, &list_args, &want)
            }
            other => Err(invalid(format!(
                "batch supports walk/stat/list (write ops need their own request), got '{}'",
                other
            ))),
        };
        match r {
            Ok(v) => {
                results.push(json!({"ok": true, "result": v}));
                completed += 1;
            }
            Err(e) => {
                results.push(json!({"ok": false, "error": e.to_json()}));
                if abort {
                    break;
                }
            }
        }
    }
    Ok(json!({ "results": results, "completed": completed }))
}

// ---------- watch (SSE) ----------

#[derive(serde::Deserialize)]
struct WatchQuery {
    session: Option<String>,
    tokens: Option<String>,
}

async fn watch_handler(
    State(state): State<SharedState>,
    Query(q): Query<WatchQuery>,
    headers: HeaderMap,
) -> Response {
    let actor = match authenticate(&state, &Credentials::from_headers(&headers)).await {
        Ok(a) => a,
        Err(e) => return envelope_error(&state, e),
    };
    if let Err(e) = crate::auth::require_file_actor(&actor) {
        return envelope_error(&state, e);
    }
    if let Err(e) = session_for(&state, q.session.as_deref(), &actor) {
        return envelope_error(&state, e);
    }
    let tokens: Option<std::collections::HashSet<String>> = q.tokens.map(|t| {
        t.split(',').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect()
    });
    let rx = state.bus.subscribe();
    let watch_key = state.watch_key;
    let st = state.clone();
    // Reconnects always start with resync; no event buffering (D11).
    let first = stream::iter(vec![Ok::<SseEvent, std::convert::Infallible>(
        SseEvent::default().event("resync").data(json!({"reason":"connect"}).to_string()),
    )]);
    let rest = stream::unfold((rx, tokens), move |(mut rx, tokens)| {
        let st = st.clone();
        let actor = actor.clone();
        async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if let (Some(filter), Some(container)) = (&tokens, &ev.container) {
                        let token = crate::handle::watch_token(&watch_key, container);
                        if !filter.contains(&token) {
                            continue;
                        }
                    }
                    if ev.event != "resync" && !st.event_visible(&actor, ev.key.as_ref(), &ev.data) {
                        continue;
                    }
                    let sse = SseEvent::default()
                        .id(ev.server_rev.to_string())
                        .event(ev.event.clone())
                        .data(ev.data.to_string());
                    return Some((Ok(sse), (rx, tokens)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Buffer rolled over: the client must resync (lossy by contract).
                    let sse = SseEvent::default()
                        .event("resync")
                        .data(json!({"reason":"lagged"}).to_string());
                    return Some((Ok(sse), (rx, tokens)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
        }
    });
    Sse::new(first.chain(rest))
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)))
        .into_response()
}

// ---------- data plane read ----------

async fn read_handler(
    State(state): State<SharedState>,
    AxPath(node_id): AxPath<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    let actor = match authenticate(&state, &Credentials::from_headers(&headers)).await {
        Ok(a) => a,
        Err(e) => return envelope_error(&state, e),
    };
    if let Err(e) = crate::auth::require_file_actor(&actor) {
        return envelope_error(&state, e);
    }
    let node = match state.resolve_ref_for(&actor, &WireRef::live(node_id, 0), Level::Read, "read") {
        Ok(n) => n,
        Err(e) => return envelope_error(&state, e),
    };
    let (root, rel, name) = match &node {
        Node::Native { root, rel, meta, .. } if meta.kind != "dir" => {
            (root.clone(), rel.clone(), node.display_name())
        }
        _ => {
            return envelope_error(
                &state,
                invalid("read target must be a file"),
            )
        }
    };
    let cfg = match state.config.root(&root) {
        Some(c) => c,
        None => return envelope_error(&state, stale("root gone")),
    };
    let full = join_root(&cfg.path, &rel);
    if matches!(&node, Node::Native { meta, .. } if meta.kind == "symlink") {
        if let Err(e) = state.require_symlink_target(&actor, &full) {
            return envelope_error(&state, e);
        }
    }
    // Follow the symlink at read time; take size from the followed target.
    let meta = match tokio::fs::metadata(&full).await {
        Ok(m) => m,
        Err(e) => return envelope_error(&state, NfsError::from(e)),
    };
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Mutable local content: weak validator, never immutable (nfs_server.md §4.2).
    let etag = format!("W/\"{}-{}\"", len, mtime);
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == etag)
        .unwrap_or(false)
    {
        return StatusCode::NOT_MODIFIED.into_response();
    }
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, status) = match parse_range(range, len) {
        Ok(Some((s, e))) => (s, e, StatusCode::PARTIAL_CONTENT),
        Ok(None) => (0, len.saturating_sub(1), StatusCode::OK),
        Err(_) => {
            return (
                StatusCode::RANGE_NOT_SATISFIABLE,
                [(header::CONTENT_RANGE, format!("bytes */{}", len))],
            )
                .into_response()
        }
    };
    let mut file = match tokio::fs::File::open(&full).await {
        Ok(f) => f,
        Err(e) => return envelope_error(&state, NfsError::from(e)),
    };
    if start > 0 {
        use tokio::io::AsyncSeekExt;
        if let Err(e) = file.seek(std::io::SeekFrom::Start(start)).await {
            return envelope_error(&state, NfsError::from(e));
        }
    }
    let content_len = if len == 0 { 0 } else { end - start + 1 };
    let body = Body::from_stream(file_stream(file, content_len));
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type_for(&name))
        .header(header::CONTENT_LENGTH, content_len)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::ETAG, etag)
        .header(header::CACHE_CONTROL, "private, no-cache");
    if status == StatusCode::PARTIAL_CONTENT {
        builder = builder.header(
            header::CONTENT_RANGE,
            format!("bytes {}-{}/{}", start, end, len),
        );
    }
    if q.contains_key("download") {
        let fname = q.get("name").cloned().unwrap_or(name);
        let encoded: String =
            percent_encoding::utf8_percent_encode(&fname, percent_encoding::NON_ALPHANUMERIC)
                .to_string();
        builder = builder.header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{}", encoded),
        );
    }
    builder.body(body).unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

fn file_stream(
    file: tokio::fs::File,
    len: u64,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
    stream::unfold((file, len), |(mut file, remaining)| async move {
        if remaining == 0 {
            return None;
        }
        use tokio::io::AsyncReadExt;
        let chunk = remaining.min(64 * 1024) as usize;
        let mut buf = vec![0u8; chunk];
        match file.read(&mut buf).await {
            Ok(0) => None,
            Ok(n) => {
                buf.truncate(n);
                Some((Ok(Bytes::from(buf)), (file, remaining - n as u64)))
            }
            Err(e) => Some((Err(e), (file, 0))),
        }
    })
}

/// Parses a single-range `bytes=` header. Ok(None) = no/ignorable header.
pub(crate) fn parse_range(header: Option<&str>, len: u64) -> Result<Option<(u64, u64)>, ()> {
    let h = match header {
        Some(h) => h,
        None => return Ok(None),
    };
    let spec = h.strip_prefix("bytes=").ok_or(())?;
    if spec.contains(',') {
        // Multi-range unsupported: serve the full body instead (allowed).
        return Ok(None);
    }
    let (a, b) = spec.split_once('-').ok_or(())?;
    if len == 0 {
        return Err(());
    }
    match (a.is_empty(), b.is_empty()) {
        (true, false) => {
            // suffix: last N bytes
            let n: u64 = b.parse().map_err(|_| ())?;
            if n == 0 {
                return Err(());
            }
            let start = len.saturating_sub(n);
            Ok(Some((start, len - 1)))
        }
        (false, true) => {
            let start: u64 = a.parse().map_err(|_| ())?;
            if start >= len {
                return Err(());
            }
            Ok(Some((start, len - 1)))
        }
        (false, false) => {
            let start: u64 = a.parse().map_err(|_| ())?;
            let end: u64 = b.parse().map_err(|_| ())?;
            if start > end || start >= len {
                return Err(());
            }
            Ok(Some((start, end.min(len - 1))))
        }
        (true, true) => Err(()),
    }
}

// ---------- uploads (minimal tus) ----------

/// Upload chunks must come from the principal whose session holds the lease.
async fn upload_owner_check(state: &SharedState, fb: &str, headers: &HeaderMap) -> NfsResult<()> {
    let actor = authenticate(state, &Credentials::from_headers(headers)).await?;
    crate::auth::require_file_actor(&actor)?;
    let owner = state.leases.session_of_upload(fb).and_then(|s| state.sessions.actor(&s));
    match owner {
        Some(owner) if owner == actor => Ok(()),
        _ => Err(not_found(format!("unknown upload '{}'", fb))),
    }
}

async fn upload_head(
    State(state): State<SharedState>,
    AxPath(fb): AxPath<String>,
    headers: HeaderMap,
) -> Response {
    if let Err(e) = upload_owner_check(&state, &fb, &headers).await {
        return envelope_error(&state, e);
    }
    match state.uploads.offset(&fb) {
        Ok((offset, expected)) => {
            let mut builder = Response::builder()
                .status(StatusCode::OK)
                .header("Tus-Resumable", "1.0.0")
                .header("Upload-Offset", offset)
                .header(header::CACHE_CONTROL, "no-store");
            if let Some(e) = expected {
                builder = builder.header("Upload-Length", e);
            }
            builder.body(Body::empty()).unwrap()
        }
        Err(e) => envelope_error(&state, e),
    }
}

async fn upload_patch(
    State(state): State<SharedState>,
    AxPath(fb): AxPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(e) = upload_owner_check(&state, &fb, &headers).await {
        return envelope_error(&state, e);
    }
    let offset: u64 = match headers
        .get("Upload-Offset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
    {
        Some(o) => o,
        None => return envelope_error(&state, invalid("PATCH requires Upload-Offset")),
    };
    match state.uploads.append(&fb, offset, &body).await {
        Ok(new_offset) => Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header("Tus-Resumable", "1.0.0")
            .header("Upload-Offset", new_offset)
            .body(Body::empty())
            .unwrap(),
        Err(e) => {
            // tus clients recover from offset conflicts via HEAD; surface the
            // current offset to save the round trip.
            let mut resp = envelope_error(&state, e);
            if let Ok((cur, _)) = state.uploads.offset(&fb) {
                if let Ok(v) = cur.to_string().parse() {
                    resp.headers_mut().insert("Upload-Offset", v);
                }
            }
            resp
        }
    }
}

// ---------- debug API (gated by --debug-api) ----------

async fn debug_handler(
    State(state): State<SharedState>,
    AxPath(action): AxPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match authenticate(&state, &Credentials::from_headers(&headers)).await {
        Ok(actor) if actor.is_admin() => {}
        Ok(actor) => return envelope_error(&state, crate::access::denied("manage", &actor)),
        Err(e) => return envelope_error(&state, e),
    }
    let args: Value = if body.is_empty() {
        json!({})
    } else {
        match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(e) => return envelope_error(&state, invalid(format!("bad json: {}", e))),
        }
    };
    let result = match action.as_str() {
        "reconcile" => {
            let st = state.clone();
            match tokio::task::spawn_blocking(move || st.reconcile_now()).await {
                Ok(Ok(report)) => Ok(serde_json::to_value(report).unwrap_or(json!({}))),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(internal(format!("reconcile panicked: {}", e))),
            }
        }
        "create_view" => debug_create_view(&state, &args),
        other => Err(invalid(format!("unknown debug action '{}'", other))),
    };
    match result {
        Ok(v) => envelope_ok(&state, v),
        Err(e) => envelope_error(&state, e),
    }
}

/// Seeds a view (v1 has no AI generator; Topic content enters via this door or
/// directly through filedb in tests).
fn debug_create_view(state: &SharedState, args: &Value) -> NfsResult<Value> {
    let view_id =
        args["view_id"].as_str().ok_or_else(|| invalid("create_view requires view_id"))?;
    let title = args["title"].as_str().unwrap_or(view_id);
    let origin = args["origin"].as_str().unwrap_or("auto");
    if state.db.view_by_id(view_id)?.is_some() {
        return Err(NfsError::new(
            ErrorCode::NamespaceConflict,
            format!("view '{}' already exists", view_id),
        ));
    }
    let view = state.db.view_create(view_id, title, origin)?;
    let mut groups: HashMap<String, i64> = HashMap::new();
    if let Some(gs) = args["groups"].as_array() {
        for (i, g) in gs.iter().enumerate() {
            let label = g["label"].as_str().ok_or_else(|| invalid("group requires label"))?;
            let by = g["by"].as_str();
            let gid = state.db.view_group_create(view.node_id, label, by, i as i64)?;
            groups.insert(label.to_string(), gid);
        }
    }
    if let Some(ms) = args["members"].as_array() {
        for (i, m) in ms.iter().enumerate() {
            let path = m["path"].as_str().ok_or_else(|| invalid("member requires path"))?;
            let node = state.resolve_dfs_path(path)?;
            let (node_id, _) = state.require_stable_node(&node)?;
            let group_id = match m["group"].as_str() {
                Some(label) => Some(
                    *groups
                        .get(label)
                        .ok_or_else(|| invalid(format!("unknown group '{}'", label)))?,
                ),
                None => None,
            };
            let provenance = m.get("provenance").filter(|p| !p.is_null());
            state.db.view_base_add(view.node_id, group_id, node_id, provenance, i as i64)?;
        }
    }
    Ok(json!({
        "ref": {"type": "live", "node_id": format!("n_{}", view.node_id), "gen": 1},
        "view_id": view_id,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_parsing() {
        assert_eq!(parse_range(None, 100), Ok(None));
        assert_eq!(parse_range(Some("bytes=0-9"), 100), Ok(Some((0, 9))));
        assert_eq!(parse_range(Some("bytes=90-"), 100), Ok(Some((90, 99))));
        assert_eq!(parse_range(Some("bytes=-10"), 100), Ok(Some((90, 99))));
        assert_eq!(parse_range(Some("bytes=50-200"), 100), Ok(Some((50, 99))));
        assert_eq!(parse_range(Some("bytes=100-"), 100), Err(()));
        assert_eq!(parse_range(Some("bytes=5-2"), 100), Err(()));
        assert_eq!(parse_range(Some("bytes=0-0,5-6"), 100), Ok(None));
        assert_eq!(parse_range(Some("bytes=0-"), 0), Err(()));
    }
}
