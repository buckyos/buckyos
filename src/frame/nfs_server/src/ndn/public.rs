//! Who may put what into the two public buckets (bucket design §4.6.5).
//!
//! `public-objects` holds what this Zone chose to publish: a system service
//! (`obj://ndm/public`) asks, and the person it asks for must hold or be able
//! to read the whole object tree; the service's own read-everything right does
//! not count (Z7). `public-cache` holds what is already anonymously readable
//! elsewhere: nfs-server itself downloads it without credentials and verifies
//! the bytes before it binds anything. A local copy never stands in for that,
//! or a FileObject crafted in another Zone could point at a private chunk here.
//!
//! The public buckets have no grants and no key is ever listed to a visitor;
//! `/ndn/` answers "is this root provided", nothing else.

use crate::bucket::{spec, Session};
use crate::config::AuthMode;
use crate::ndm;
use crate::state::SharedState;
use bucket_store::{BucketError, BucketResult, BucketStore, Caller, ListRequest, PutRequest};
use buckyos_api::{nfs_public_cache_bucket, nfs_public_objects_bucket};
use ndn_lib::{percent_encode_component, ChunkList, DirObject, FileObject, ObjId, SimpleMapItem};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

pub const RES_PUBLIC: &str = "obj://ndm/public";
pub const RES_PUBLIC_CACHE: &str = "obj://ndm/public_cache";

/// Objects a tree may have before admission gives up.
const MAX_TREE_NODES: usize = 200_000;
/// Bound on a whole download of one source.
const PULL_TIMEOUT: Duration = Duration::from_secs(1800);
const MAX_CONCURRENT_PULLS: usize = 4;

pub(crate) fn principal_of(session: &Session) -> Option<ndm::Principal> {
    match session {
        Session::Open => Some(ndm::Principal::new("local".into(), "system:local".into(), "system:local".into(), None)),
        Session::User { user, app_key, app, token, .. } => Some(ndm::Principal::new(user.clone(), app_key.clone(), app.clone(), Some(token.clone()))),
        Session::Agent { subject, app, token, .. } => Some(ndm::Principal::new(subject.clone(), app.clone(), app.clone(), Some(token.clone()))),
        Session::Service { subject, app, token, .. } => Some(ndm::Principal::new(subject.clone(), app.clone(), app.clone(), Some(token.clone()))),
        Session::Device { .. } | Session::Anonymous => None,
    }
}

async fn require(state: &SharedState, session: &Session, resource: &str) -> BucketResult<()> {
    let Some(principal) = principal_of(session) else { return Err(BucketError::PermissionDenied(format!("write on {resource}"))) };
    if ndm::allowed(state, &principal, resource, "write").await {
        Ok(())
    } else {
        Err(BucketError::PermissionDenied(format!("write on {resource}")))
    }
}

/// The name a provider's or app's keys live under.
fn provider_of(session: &Session) -> BucketResult<String> {
    match session {
        Session::Open => Ok("local".into()),
        Session::Service { service, .. } => Ok(service.clone()),
        Session::User { app_key, .. } => Ok(app_key.split_once(':').map(|(_, v)| v).unwrap_or(app_key).to_string()),
        Session::Agent { app, .. } => Ok(app.split_once(':').map(|(_, v)| v).unwrap_or(app).split('@').next().unwrap_or(app).to_string()),
        _ => Err(BucketError::PermissionDenied("an app or service identity is required".into())),
    }
}

/// `requested_by` holds `id`: some session of this user uploaded or registered it.
pub(crate) fn user_holds(state: &SharedState, user: &str, id: &str) -> bool {
    let conn = state.db.lock();
    conn.query_row(
        "SELECT 1 FROM ndm_holdings WHERE principal >= ?1 AND principal < ?2 AND obj_id = ?3 LIMIT 1",
        rusqlite::params![format!("{user}|"), format!("{user}}}"), id],
        |_| Ok(()),
    )
    .is_ok()
}

async fn user_reads_every_object(state: &SharedState, user: &str) -> bool {
    match &state.config.auth {
        AuthMode::Disabled => true,
        AuthMode::TestKey { .. } => rbac::enforce(user, "system:nfs-server", ndm::RES_OBJECTS, "read", None).await,
        AuthMode::Runtime => match buckyos_api::get_buckyos_api_runtime() {
            Ok(runtime) => runtime.enforce_principal(user, "system:nfs-server", ndm::RES_OBJECTS, "read").await.unwrap_or(false),
            Err(_) => false,
        },
    }
}

/// The objects `id` directly contains; only files, chunk lists and directories have any.
pub(crate) fn parts_of(id: &ObjId, stored: &str) -> BucketResult<Vec<ObjId>> {
    let bad = |what: &str| BucketError::InvalidArgument(format!("{} is {what}", id.to_string()));
    match id.obj_type.as_str() {
        ndn_lib::OBJ_TYPE_FILE => {
            let file: FileObject = ndn_lib::load_named_obj(stored).map_err(|_| bad("not a FileObject"))?;
            if file.content.is_empty() {
                return Ok(Vec::new());
            }
            let content = ObjId::new(&file.content).map_err(|_| bad("a file without valid content"))?;
            if content.is_chunk() || content.is_chunk_list() {
                Ok(vec![content])
            } else {
                Err(bad("a file whose content is not raw data"))
            }
        }
        ndn_lib::OBJ_TYPE_CHUNK_LIST => Ok(ChunkList::from_json(stored).map_err(|_| bad("not a chunk list"))?.body.iter().map(|c| c.to_obj_id()).collect()),
        ndn_lib::OBJ_TYPE_DIR => {
            let dir: DirObject = ndn_lib::load_named_obj(stored).map_err(|_| bad("not a DirObject"))?;
            let mut out = Vec::with_capacity(dir.len());
            for item in dir.values() {
                let child = match item {
                    SimpleMapItem::ObjId(id) => id.clone(),
                    other => other.get_obj_id().map_err(|_| bad("a directory with an invalid entry"))?.0,
                };
                out.push(child);
            }
            Ok(out)
        }
        _ => Err(bad("not a file or directory tree")),
    }
}

/// Every object and chunk below `root`, itself included, passes for `user`.
async fn check_tree(state: &SharedState, store: &named_store::NamedDataMgr, user: &str, root: &ObjId) -> BucketResult<()> {
    if user_reads_every_object(state, user).await {
        return Ok(());
    }
    let mut pending = vec![root.clone()];
    let mut seen = HashSet::new();
    while let Some(id) = pending.pop() {
        if !seen.insert(id.to_string()) {
            continue;
        }
        if seen.len() > MAX_TREE_NODES {
            return Err(BucketError::InvalidArgument("the object tree is too large to publish".into()));
        }
        if !user_holds(state, user, &id.to_string()) {
            return Err(BucketError::NotHeld(format!("{} of {} is not held by {user}", id.to_string(), root.to_string())));
        }
        if !id.is_chunk() {
            let stored = store.get_object(&id).await.map_err(|_| BucketError::ContentNotAvailable(id.to_string()))?;
            pending.extend(parts_of(&id, &stored)?);
        }
    }
    Ok(())
}

fn entry_prefix(provider: &str, entry: &str) -> BucketResult<String> {
    if entry.is_empty() {
        return Err(BucketError::InvalidArgument("entry is required".into()));
    }
    // One key segment, so withdrawing `a` never touches `a/b`.
    Ok(format!("{}/{}/", provider, percent_encode_component(entry)))
}

fn parse_root(raw: &str) -> BucketResult<ObjId> {
    let root = ObjId::new(raw).map_err(|e| BucketError::InvalidArgument(format!("root_obj_id: {e}")))?;
    if root.is_file_object() || root.is_dir_object() {
        Ok(root)
    } else {
        Err(BucketError::KindMismatch("a public root is a file or directory".into()))
    }
}

/// `bucket.provide_public { entry, root_obj_id, requested_by }`.
pub async fn provide(state: &SharedState, session: &Session, entry: &str, root_obj_id: &str, requested_by: &str) -> BucketResult<Value> {
    require(state, session, RES_PUBLIC).await?;
    let prefix = entry_prefix(&provider_of(session)?, entry)?;
    let root = parse_root(root_obj_id)?;
    if requested_by.is_empty() {
        return Err(BucketError::InvalidArgument("requested_by is required".into()));
    }
    let bucket = nfs_public_objects_bucket();
    spec::ensure(state, &bucket).await?;
    let store = state.buckets.store(state).await?.clone();
    check_tree(state, store.ndm(), requested_by, &root).await?;
    let key = format!("{prefix}{}", root.to_string());
    let put = store.put(&bucket, &Caller::system(), PutRequest { key: key.clone(), obj_id: root.to_string(), ..Default::default() }).await?;
    log::info!("public object {} provided as {key} for {requested_by}", root.to_string());
    Ok(json!({"bucket_id": bucket, "key": key, "obj_id": root.to_string(), "version": put.version}))
}

async fn keys_under(store: &BucketStore, bucket: &bucket_store::BucketId, prefix: &str) -> BucketResult<Vec<bucket_store::Binding>> {
    let mut out = Vec::new();
    let mut start_after = None;
    loop {
        let req = ListRequest { prefix: prefix.to_string(), start_after, ..Default::default() };
        let page = store.list_objects(bucket, &Caller::system(), &req).await?;
        out.extend(page.items);
        match page.next_start_after {
            Some(next) => start_after = Some(next),
            None => return Ok(out),
        }
    }
}

/// `bucket.withdraw_public { entry }`: this entry's keys go; another entry publishing the same object stays.
pub async fn withdraw(state: &SharedState, session: &Session, entry: &str, root_obj_id: Option<&str>) -> BucketResult<Value> {
    require(state, session, RES_PUBLIC).await?;
    let prefix = entry_prefix(&provider_of(session)?, entry)?;
    let bucket = nfs_public_objects_bucket();
    spec::ensure(state, &bucket).await?;
    let store = state.buckets.store(state).await?.clone();
    let keep_others = root_obj_id.map(|r| format!("{prefix}{}", parse_root(r).map(|id| id.to_string()).unwrap_or_default()));
    let mut withdrawn = 0;
    for b in keys_under(&store, &bucket, &prefix).await? {
        if keep_others.as_ref().is_none_or(|k| *k == b.key) {
            store.delete(&bucket, &Caller::system(), &b.key, None, None).await?;
            withdrawn += 1;
        }
    }
    log::info!("public entry {prefix} withdrawn: {withdrawn} keys");
    Ok(json!({"withdrawn": withdrawn}))
}

/// `bucket.list_public`: what this provider has provided, for reconciling after a crash.
pub async fn list(state: &SharedState, session: &Session) -> BucketResult<Value> {
    require(state, session, RES_PUBLIC).await?;
    let prefix = format!("{}/", provider_of(session)?);
    let bucket = nfs_public_objects_bucket();
    spec::ensure(state, &bucket).await?;
    let store = state.buckets.store(state).await?.clone();
    let items: Vec<Value> = keys_under(&store, &bucket, &prefix)
        .await?
        .into_iter()
        .filter_map(|b| {
            let rest = b.key.strip_prefix(&prefix)?;
            let (entry, _) = rest.rsplit_once('/')?;
            let entry = percent_encoding::percent_decode_str(entry).decode_utf8().ok()?.into_owned();
            Some(json!({"entry": entry, "root_obj_id": b.obj_id}))
        })
        .collect();
    Ok(json!({"items": items}))
}

// ---------- public-cache ----------

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first == 0x2001 && v6.segments()[1] == 0xdb8))
        }
    }
}

/// `source` is another Zone's plain `https://<host>/ndn/<root>`; returns `<host>`.
pub fn validate_source(source: &str, root: &ObjId, zone_host: Option<&str>) -> BucketResult<String> {
    let bad = |why: &str| BucketError::InvalidArgument(format!("source: {why}"));
    let url = url::Url::parse(source).map_err(|_| bad("not a URL"))?;
    if url.scheme() != "https" {
        return Err(bad("must be https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(bad("must not carry credentials"));
    }
    if url.query().is_some() || url.fragment().is_some() {
        return Err(bad("must not carry a query"));
    }
    if url.port().is_some_and(|p| p != 443) {
        return Err(bad("must use the default port"));
    }
    let host = match url.host() {
        Some(url::Host::Domain(host)) => host.trim_end_matches('.').to_ascii_lowercase(),
        Some(_) => return Err(bad("must not be an IP address")),
        None => return Err(bad("has no host")),
    };
    if !host.contains('.') || host == "localhost" || [".localhost", ".local", ".internal", ".intranet", ".lan", ".home.arpa"].iter().any(|s| host.ends_with(s)) {
        return Err(bad("must be a public host"));
    }
    if let Some(zone) = zone_host {
        let zone = zone.trim_end_matches('.').to_ascii_lowercase();
        if host == zone || host.ends_with(&format!(".{zone}")) {
            return Err(bad("must be another Zone"));
        }
    }
    if url.path() != format!("/ndn/{}", root.to_string()) {
        return Err(bad("must be /ndn/<root_obj_id>"));
    }
    Ok(host)
}

/// A client that connects only to the public addresses checked here, follows no
/// redirect and has no credentials or cookies of any kind.
pub(crate) async fn anonymous_client(host: &str) -> Result<ndn_toolkit::CyfsNdnClient, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 443)).await.map_err(|e| format!("resolve {host}: {e}"))?.collect();
    if addrs.is_empty() || !addrs.iter().all(|a| is_public_ip(a.ip())) {
        return Err(format!("{host} does not resolve to public addresses only"));
    }
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, &addrs)
        .timeout(PULL_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    ndn_toolkit::CyfsNdnClient::builder().transport(ndn_toolkit::ReqwestCyfsTransport::new(http)).build().map_err(|e| e.to_string())
}

fn pulls() -> &'static tokio::sync::Semaphore {
    static PULLS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(MAX_CONCURRENT_PULLS);
    &PULLS
}

/// Downloads a file from `source` and verifies every part against its id, local
/// copies notwithstanding. `limit` bounds what is fetched at all.
pub async fn pull_verified(client: &ndn_toolkit::CyfsNdnClient, ndm: &named_store::NamedDataMgr, root: &ObjId, source: &str, limit: u64) -> Result<(), String> {
    let Ok(_slot) = pulls().acquire().await else { return Err("shutting down".into()) };
    let (announced, body) = client.get(source).raw().send().await.map_err(|e| e.to_string())?.object().await.map_err(|e| e.to_string())?;
    if announced != *root {
        return Err(format!("source served {}", announced.to_string()));
    }
    let size = body["size"].as_u64().ok_or_else(|| "the file object has no size".to_string())?;
    if size > limit {
        return Err(format!("{size} bytes exceed the limit of {limit}"));
    }
    client.get(source).pull_to_named_store(ndm).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// Downloads `source` anonymously and verifies it; only then does the key exist.
pub async fn pull_and_bind(state: &SharedState, client: &ndn_toolkit::CyfsNdnClient, appid: &str, root: &ObjId, source: &str) -> Result<(), String> {
    let store = state.buckets.store(state).await.map_err(|e| e.to_string())?.clone();
    let limit = state.config.public_cache_app_bytes;
    pull_verified(client, store.ndm(), root, source, limit).await?;
    let bucket = nfs_public_cache_bucket();
    spec::ensure(state, &bucket).await.map_err(|e| e.to_string())?;
    let key = format!("{appid}/{}", root.to_string());
    store
        .put(&bucket, &Caller::system(), PutRequest { key, obj_id: root.to_string(), ..Default::default() })
        .await
        .map_err(|e| e.to_string())?;
    evict_over_app_limit(&store, &bucket, appid, limit).await.map_err(|e| e.to_string())?;
    Ok(())
}

/// D25: an app's cache stays under its limit; its oldest keys go first.
async fn evict_over_app_limit(store: &BucketStore, bucket: &bucket_store::BucketId, appid: &str, limit: u64) -> BucketResult<()> {
    let mut keys = keys_under(store, bucket, &format!("{appid}/")).await?;
    let mut total: u64 = keys.iter().map(|b| b.size).sum();
    keys.sort_by_key(|b| b.created_at);
    for b in keys {
        if total <= limit {
            break;
        }
        store.delete(bucket, &Caller::system(), &b.key, None, None).await?;
        total = total.saturating_sub(b.size);
        log::info!("public cache of {appid} over limit: evicted {}", b.key);
    }
    Ok(())
}

/// `bucket.cache_public { root_obj_id, source }`: accepted now, bound once verified.
pub async fn cache(state: &SharedState, session: &Session, root_obj_id: &str, source: &str) -> BucketResult<Value> {
    require(state, session, RES_PUBLIC_CACHE).await?;
    let appid = provider_of(session)?;
    let root = ObjId::new(root_obj_id).map_err(|e| BucketError::InvalidArgument(format!("root_obj_id: {e}")))?;
    if !root.is_file_object() {
        return Err(BucketError::KindMismatch("the public cache takes files for now".into()));
    }
    let host = validate_source(source, &root, state.config.share.zone_host.as_deref())?;
    let state = Arc::clone(state);
    let source = source.to_string();
    // The pull client's futures are not Send; a blocking slot runs them to the end.
    let runtime = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || {
        runtime.block_on(async move {
            let outcome = match anonymous_client(&host).await {
                Ok(client) => pull_and_bind(&state, &client, &appid, &root, &source).await,
                Err(e) => Err(e),
            };
            match outcome {
                Ok(()) => log::info!("public cache of {appid}: {} from {host} verified and bound", root.to_string()),
                Err(e) => log::warn!("public cache of {appid}: {} from {host} not bound: {e}", root.to_string()),
            }
        })
    });
    Ok(json!({"accepted": true}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> ObjId {
        ObjId::new_by_raw("cyfile".to_string(), vec![7; 32])
    }

    #[test]
    fn a_source_is_another_zones_plain_ndn_url() {
        let root = root();
        let id = root.to_string();
        let ok = |url: String| validate_source(&url, &root, Some("my.zone.example"));
        assert_eq!(ok(format!("https://other.example/ndn/{id}")).unwrap(), "other.example");
        for bad in [
            format!("http://other.example/ndn/{id}"),
            format!("https://u:p@other.example/ndn/{id}"),
            format!("https://other.example/ndn/{id}?hint=%7B%7D"),
            format!("https://other.example:8443/ndn/{id}"),
            format!("https://127.0.0.1/ndn/{id}"),
            format!("https://[::1]/ndn/{id}"),
            format!("https://10.0.0.1/ndn/{id}"),
            format!("https://localhost/ndn/{id}"),
            format!("https://intranet/ndn/{id}"),
            format!("https://nas.local/ndn/{id}"),
            format!("https://my.zone.example/ndn/{id}"),
            format!("https://public.my.zone.example/ndn/{id}"),
            format!("https://other.example/ndn/{id}/@/content"),
            "https://other.example/ndn/cyfile:other".to_string(),
            format!("https://other.example/bucket/v1/{id}"),
        ] {
            assert!(ok(bad.clone()).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_global_addresses_are_public() {
        for ip in ["10.1.2.3", "172.16.0.1", "192.168.1.1", "127.0.0.1", "169.254.169.254", "100.64.0.1", "0.0.0.0", "::1", "fe80::1", "fc00::1", "::ffff:10.0.0.1", "2001:db8::1"] {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700::1111", "::ffff:8.8.8.8"] {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
}
