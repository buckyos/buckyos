//! `/ndn/` through nfs-server (bucket design §4.6.5, §21 N4, N8, N10–N12, N14):
//! knowing an ObjId grants nothing, public entries need the requester to hold the
//! whole tree, the public cache only takes what a real anonymous download proved.

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use ndn_lib::{NamedObject, NdnHint, ObjId};
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::{AppState, SharedState};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = "g, owner, admin\ng, alice, users\ng, bob, users\ng, mallory, users\ng, ood1, ood\ng, app:photos, app\ng, system:control-panel, system\n";
        let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, groups);
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim()).await.unwrap();
    })
    .await;
}

fn token(sub: &str, kind: TokenPrincipalKind, target: &str) -> String {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(sub.into()),
        appid: None,
        sudo: false,
        extra: Default::default(),
    };
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(&mut token, &AuthTarget::from_canonical_key(target).unwrap(), TokenUse::Session).unwrap();
    token.generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap()).unwrap()
}

fn ui(name: &str) -> String {
    token(name, TokenPrincipalKind::User, "system:control-panel")
}

fn via(name: &str, app: &str) -> String {
    token(name, TokenPrincipalKind::User, &format!("app:{app}@{name}"))
}

/// A system service as it really authenticates: a device running it.
fn service(name: &str) -> String {
    token("ood1", TokenPrincipalKind::System, &format!("system:{name}"))
}

struct Server {
    base: String,
    state: SharedState,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn start_with(tune: impl FnOnce(&mut ServerConfig)) -> Server {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("exports");
    std::fs::create_dir_all(data.join("home/alice")).unwrap();
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: data.join("home") }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.ndn_anonymous_bytes_per_sec = 0;
    config.bucket_specs = serde_json::from_value(json!({
        "photos": {
            "storage_quota_mib": 16,
            "buckets": {
                "library": {"kind": "data", "data_policy": "documents"},
                "index": {"kind": "data", "data_policy": "app_data"},
                "related": {"kind": "data", "data_policy": "ambient"}
            }
        }
    }))
    .unwrap();
    tune(&mut config);
    let state = AppState::new(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::server::build_router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, state, http: reqwest::Client::new(), _tmp: tmp }
}

async fn start() -> Server {
    start_with(|_| {}).await
}

fn chunk_of(data: &[u8]) -> ndn_lib::ChunkId {
    ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap()
}

fn file_of(name: &str, data: &[u8]) -> (ObjId, String) {
    ndn_lib::FileObject::new(name.into(), data.len() as u64, chunk_of(data).to_string()).gen_obj_id()
}

impl Server {
    async fn call(&self, auth: &str, method: &str, params: Value) -> Result<Value, (String, Value)> {
        let body = json!({"method": method, "params": params, "sys": [1, auth]});
        let v: Value = self.http.post(format!("{}/kapi/nfs-server", self.base)).json(&body).send().await.unwrap().json().await.unwrap();
        match v.get("result") {
            Some(r) => Ok(r.clone()),
            None => Err((v["code"].as_str().unwrap_or_default().to_string(), v)),
        }
    }

    /// `user` uploads `data` as a file through the NDM gateway: it holds the whole tree afterwards.
    async fn upload(&self, user: &str, name: &str, data: &[u8]) -> ObjId {
        let auth = via(user, "photos");
        let (id, body) = file_of(name, data);
        let status = self
            .http
            .put(format!("{}/ndm/proxy/v1/write/chunk/{}", self.base, chunk_of(data).to_string()))
            .header("NDM-Chunk-Size", data.len().to_string())
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .bearer_auth(&auth)
            .send()
            .await
            .unwrap()
            .status();
        assert!(status.is_success(), "{status}");
        let manifest = json!({"objects": [{"obj_id": id.to_string(), "obj_data": body}], "roots": [id.to_string()]});
        let resp = self.http.post(format!("{}/ndm/v1/import/finalize", self.base)).bearer_auth(&auth).json(&manifest).send().await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        id
    }

    fn ndn(&self, id: &ObjId) -> String {
        format!("{}/ndn/{}", self.base, id.to_string())
    }

    fn ndn_hint(&self, id: &ObjId, hint: &NdnHint) -> String {
        format!("{}?{}", self.ndn(id), hint.to_query().unwrap())
    }

    async fn status(&self, url: &str) -> StatusCode {
        self.http.get(url).send().await.unwrap().status()
    }

    async fn provide(&self, entry: &str, id: &ObjId, requested_by: &str) -> Result<Value, (String, Value)> {
        self.call(&service("homestation"), "bucket.provide_public", json!({"entry": entry, "root_obj_id": id.to_string(), "requested_by": requested_by})).await
    }
}

#[tokio::test]
async fn knowing_an_object_id_reads_nothing() {
    let srv = start().await;
    let id = srv.upload("alice", "secret.txt", b"secret").await;
    assert_eq!(srv.status(&srv.ndn(&id)).await, StatusCode::NOT_FOUND);
    // No credential of any kind counts here: not even the holder's own session.
    let with_bearer = srv.http.get(srv.ndn(&id)).bearer_auth(ui("alice")).send().await.unwrap();
    assert_eq!(with_bearer.status(), StatusCode::NOT_FOUND);
    let cookie = format!("buckyos_session_token={}", ui("owner"));
    let with_cookie = srv.http.get(srv.ndn(&id)).header("cookie", cookie).send().await.unwrap();
    assert_eq!(with_cookie.status(), StatusCode::NOT_FOUND);
    assert_eq!(with_bearer.headers()["referrer-policy"], "no-referrer");
    assert_eq!(with_bearer.headers()["x-content-type-options"], "nosniff");
    assert_eq!(with_bearer.headers()["cache-control"], "no-store");
    // Unknown objects read the same.
    let missing = srv.ndn(&file_of("x", b"never stored").0);
    let refused = srv.http.get(&missing).send().await.unwrap();
    assert_eq!(refused.status(), StatusCode::NOT_FOUND);
    assert_eq!(refused.text().await.unwrap(), srv.http.get(srv.ndn(&id)).send().await.unwrap().text().await.unwrap());
    // Read only; a malformed hint says so without naming any object.
    for method in [reqwest::Method::PUT, reqwest::Method::PATCH, reqwest::Method::POST, reqwest::Method::DELETE] {
        let r = srv.http.request(method, srv.ndn(&id)).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(r.headers()["allow"], "GET, HEAD");
    }
    assert_eq!(srv.status(&format!("{}?hint=%7Bnope", srv.ndn(&id))).await, StatusCode::BAD_REQUEST);
    assert_eq!(srv.status(&format!("{}/@/content?hint=%7B%22path%22%3A%22x%22%7D", srv.ndn(&id))).await, StatusCode::BAD_REQUEST);
    assert_eq!(srv.status(&format!("{}/ndn/not-an-id", srv.base)).await, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_public_entry_needs_the_requester_to_hold_the_whole_tree() {
    let srv = start().await;
    let data = b"published for everyone";
    let id = srv.upload("alice", "post.txt", data).await;

    // Only a system service may publish; a person or an app may not, whoever holds it.
    for auth in [ui("alice"), via("alice", "photos"), ui("owner")] {
        let (code, _) = srv.call(&auth, "bucket.provide_public", json!({"entry": "e1", "root_obj_id": id.to_string(), "requested_by": "alice"})).await.unwrap_err();
        assert_eq!(code, "permission_denied");
    }
    // The service's own read-everything right does not count: bob holds nothing.
    let (code, _) = srv.provide("hs/e1", &id, "bob").await.unwrap_err();
    assert_eq!(code, "not_held");
    assert_eq!(srv.status(&srv.ndn(&id)).await, StatusCode::NOT_FOUND);

    srv.provide("hs/e1", &id, "alice").await.unwrap();
    let r = srv.http.get(srv.ndn(&id)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(r.headers()["content-type"], "text/plain; charset=utf-8");
    assert_eq!(r.headers()["etag"], format!("\"{}\"", id.to_string()));
    assert_eq!(r.bytes().await.unwrap().as_ref(), data);
    let ranged = srv.http.get(srv.ndn(&id)).header("range", "bytes=1-3").send().await.unwrap();
    assert_eq!(ranged.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(ranged.bytes().await.unwrap().as_ref(), b"ubl");
    let head = srv.http.head(srv.ndn(&id)).send().await.unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(head.headers()["content-length"], data.len().to_string().as_str());
    // A path from the root reaches the chunk; a stranger's chunk through it does not.
    let chunk = chunk_of(data).to_obj_id();
    let path = NdnHint { path: Some(format!("{}/@/content", id.to_string())), token: None };
    assert_eq!(srv.http.get(srv.ndn_hint(&chunk, &path)).send().await.unwrap().bytes().await.unwrap().as_ref(), data);
    assert_eq!(srv.status(&srv.ndn(&chunk)).await, StatusCode::NOT_FOUND, "a chunk is no root");
    let other = srv.upload("bob", "bob.txt", b"bob private").await;
    let walk = NdnHint { path: Some(format!("{}/@/content", id.to_string())), token: None };
    assert_eq!(srv.status(&srv.ndn_hint(&other, &walk)).await, StatusCode::NOT_FOUND);
    // A bucket key of the public bucket is not reachable by itself.
    let bucket = nfs_public_objects_bucket();
    let plane = srv.http.get(format!("{}/bucket/v1/{bucket}/homestation/x", srv.base)).send().await.unwrap();
    assert_eq!(plane.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_forged_file_pointing_at_someone_elses_chunk_is_refused() {
    let srv = start().await;
    let secret = b"bob's private chunk";
    srv.upload("bob", "bob.txt", secret).await;

    // Mallory registers a FileObject naming bob's chunk; the gateway refuses it...
    let auth = via("mallory", "photos");
    let (forged, body) = file_of("cover.jpg", secret);
    let manifest = json!({"objects": [{"obj_id": forged.to_string(), "obj_data": body}], "roots": [forged.to_string()]});
    let resp = srv.http.post(format!("{}/ndm/v1/import/finalize", srv.base)).bearer_auth(&auth).json(&manifest).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // ...and even a recorded holding of the object alone does not open its content.
    let store = srv.state.share.store(&srv.state.config).await.unwrap().clone();
    store.put_object(&forged, &body).await.unwrap();
    let principal = nfs_server::ndm::Principal::new("mallory".into(), "app:photos".into(), "app:photos@mallory".into(), None);
    nfs_server::ndm::record_holding(&srv.state, &principal, &forged, "register");
    let (code, _) = srv.provide("hs/evil", &forged, "mallory").await.unwrap_err();
    assert_eq!(code, "not_held");
    assert_eq!(srv.status(&srv.ndn(&forged)).await, StatusCode::NOT_FOUND);
    // The real holder may publish it (and an administrator may read every object).
    srv.provide("hs/bob", &forged, "owner").await.unwrap();
    assert_eq!(srv.status(&srv.ndn(&forged)).await, StatusCode::OK);
}

#[tokio::test]
async fn withdrawing_one_entry_leaves_the_other() {
    let srv = start().await;
    let id = srv.upload("alice", "both.txt", b"in two entries").await;
    srv.provide("hs/a", &id, "alice").await.unwrap();
    srv.provide("hs/a/nested", &id, "alice").await.unwrap();
    let hs = service("homestation");
    let listed = srv.call(&hs, "bucket.list_public", json!({})).await.unwrap();
    let mut entries: Vec<&str> = listed["items"].as_array().unwrap().iter().map(|i| i["entry"].as_str().unwrap()).collect();
    entries.sort();
    assert_eq!(entries, ["hs/a", "hs/a/nested"]);
    srv.call(&hs, "bucket.withdraw_public", json!({"entry": "hs/a"})).await.unwrap();
    assert_eq!(srv.status(&srv.ndn(&id)).await, StatusCode::OK, "still public through the other entry");
    // Another service cannot touch these entries; it has its own namespace.
    let other = service("msg-center");
    assert_eq!(srv.call(&other, "bucket.withdraw_public", json!({"entry": "hs/a/nested"})).await.unwrap()["withdrawn"], 0);
    assert_eq!(srv.status(&srv.ndn(&id)).await, StatusCode::OK);
    srv.call(&hs, "bucket.withdraw_public", json!({"entry": "hs/a/nested"})).await.unwrap();
    assert_eq!(srv.status(&srv.ndn(&id)).await, StatusCode::NOT_FOUND);
    assert_eq!(srv.call(&ui("alice"), "bucket.withdraw_public", json!({"entry": "x"})).await.unwrap_err().0, "permission_denied");
}

#[tokio::test]
async fn a_token_reads_only_the_current_binding_of_its_key() {
    let srv = start().await;
    let alice = via("alice", "photos");
    let bucket = "user/photos/alice/library";
    let id = srv.upload("alice", "a.txt", b"attachment").await;
    srv.call(&alice, "bucket.put", json!({"bucket_id": bucket, "key": "k", "obj_id": id.to_string()})).await.unwrap();
    let expires = bucket_store::now_ms() + 60_000;
    let issued = srv.call(&alice, "bucket.create_access_token", json!({"bucket_id": bucket, "key": "k", "actions": ["read"], "expires_at": expires})).await.unwrap();
    let token = issued["token"].as_str().unwrap().to_string();
    let hint = NdnHint { path: None, token: Some(token.clone()) };

    let r = srv.http.get(srv.ndn_hint(&id, &hint)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "private");
    assert_eq!(r.bytes().await.unwrap().as_ref(), b"attachment");
    // The whole object graph of that root is covered; nothing else is.
    let chunk = chunk_of(b"attachment").to_obj_id();
    let walk = NdnHint { path: Some(format!("{}/@/content", id.to_string())), token: Some(token.clone()) };
    assert_eq!(srv.status(&srv.ndn_hint(&chunk, &walk)).await, StatusCode::OK);
    let other = srv.upload("alice", "b.txt", b"other file").await;
    assert_eq!(srv.status(&srv.ndn_hint(&other, &hint)).await, StatusCode::NOT_FOUND);
    // Without the token, or with a junk one: the same 404.
    assert_eq!(srv.status(&srv.ndn(&id)).await, StatusCode::NOT_FOUND);
    assert_eq!(srv.status(&srv.ndn_hint(&id, &NdnHint { path: None, token: Some("bkt_nope".into()) })).await, StatusCode::NOT_FOUND);
    // Rebinding the key takes the old root away.
    srv.call(&alice, "bucket.put", json!({"bucket_id": bucket, "key": "k", "obj_id": other.to_string()})).await.unwrap();
    assert_eq!(srv.status(&srv.ndn_hint(&id, &hint)).await, StatusCode::NOT_FOUND);
    assert_eq!(srv.status(&srv.ndn_hint(&other, &hint)).await, StatusCode::OK);
    srv.call(&alice, "bucket.revoke_access_token", json!({"token_id": issued["token_id"]})).await.unwrap();
    assert_eq!(srv.status(&srv.ndn_hint(&other, &hint)).await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_data_plane_tells_anonymous_callers_nothing() {
    let srv = start().await;
    let alice = via("alice", "photos");
    let real = "user/photos/alice/library";
    let id = srv.upload("alice", "a.txt", b"bytes").await;
    srv.call(&alice, "bucket.put", json!({"bucket_id": real, "key": "k", "obj_id": id.to_string()})).await.unwrap();
    let issued = srv.call(&alice, "bucket.create_access_token", json!({"bucket_id": real, "key": "k", "actions": ["read"], "expires_at": bucket_store::now_ms() + 60_000})).await.unwrap();
    let token = issued["token"].as_str().unwrap();

    let existing = srv.http.get(format!("{}/bucket/v1/{real}/k", srv.base)).send().await.unwrap();
    let missing_bucket = "user/photos/carol/library";
    let missing = srv.http.get(format!("{}/bucket/v1/{missing_bucket}/k", srv.base)).send().await.unwrap();
    assert_eq!(existing.status(), StatusCode::FORBIDDEN);
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    assert_eq!(existing.text().await.unwrap(), missing.text().await.unwrap());
    // Probing created nothing, even for a bucket a spec declares.
    let store = srv.state.buckets.store(&srv.state).await.unwrap();
    assert!(store.bucket(&missing_bucket.parse().unwrap()).is_err());
    let junk = srv.http.get(format!("{}/bucket/v1/{missing_bucket}/k?token=bkt_nope", srv.base)).send().await.unwrap();
    assert_eq!(junk.status(), StatusCode::UNAUTHORIZED);
    assert!(store.bucket(&missing_bucket.parse().unwrap()).is_err());
    let wrong = srv.http.get(format!("{}/bucket/v1/{missing_bucket}/k?token={token}", srv.base)).send().await.unwrap();
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    assert!(store.bucket(&missing_bucket.parse().unwrap()).is_err());

    // A session that does not verify does not get in the way of a good token.
    let ok = srv.http.get(format!("{}/bucket/v1/{real}/k?token={token}", srv.base)).bearer_auth("not-a-session").send().await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    assert_eq!(ok.headers()["cache-control"], "private, no-cache");
    assert_eq!(ok.bytes().await.unwrap().as_ref(), b"bytes");
    let without = srv.http.get(format!("{}/bucket/v1/{real}/k", srv.base)).bearer_auth("not-a-session").send().await.unwrap();
    assert_eq!(without.status(), StatusCode::FORBIDDEN);
    // A token bound to a principal needs that principal: an unverifiable session is none.
    let bound = srv.call(&alice, "bucket.create_access_token", json!({"bucket_id": real, "key": "k", "actions": ["read"], "expires_at": bucket_store::now_ms() + 60_000, "bind_user_id": "alice"})).await.unwrap();
    let denied = srv.http.get(format!("{}/bucket/v1/{real}/k?token={}", srv.base, bound["token"].as_str().unwrap())).bearer_auth("not-a-session").send().await.unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn the_public_cache_only_takes_what_a_real_download_proved() {
    let origin = start().await;
    let here = start_with(|c| c.public_cache_app_bytes = 64).await;
    let data = b"public elsewhere";
    let id = origin.upload("alice", "shared.txt", data).await;
    origin.provide("hs/p", &id, "alice").await.unwrap();
    let client = ndn_toolkit::CyfsNdnClient::new();
    let bucket = nfs_public_cache_bucket();

    nfs_server::ndn::public::pull_and_bind(&here.state, &client, "photos", &id, &origin.ndn(&id)).await.unwrap();
    let r = here.http.get(here.ndn(&id)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert_eq!(r.headers()["cache-control"], "public, max-age=31536000, immutable");
    assert_eq!(r.bytes().await.unwrap().as_ref(), data);
    let store = here.state.buckets.store(&here.state).await.unwrap();
    let binding = store.get(&bucket, &bucket_store::Caller::system(), &format!("photos/{}", id.to_string()), None).await.unwrap();
    assert_eq!(binding.obj_id.as_deref(), Some(id.to_string().as_str()));

    // A source that wants a credential binds nothing.
    let private = origin.upload("alice", "private.txt", b"not public").await;
    assert!(nfs_server::ndn::public::pull_and_bind(&here.state, &client, "photos", &private, &origin.ndn(&private)).await.is_err());
    assert_eq!(here.status(&here.ndn(&private)).await, StatusCode::NOT_FOUND);

    // A source offering the object but not its bytes binds nothing, however
    // complete the local copy is: a crafted FileObject cannot borrow a private chunk.
    let victim = b"a private chunk of this zone";
    here.upload("bob", "victim.txt", victim).await;
    let crafted = origin.upload("alice", "nice.txt", victim).await;
    origin.provide("craft", &crafted, "alice").await.unwrap();
    assert_eq!(origin.status(&origin.ndn(&crafted)).await, StatusCode::OK);
    let origin_store = origin.state.share.store(&origin.state.config).await.unwrap();
    origin_store.remove_chunk(&chunk_of(victim)).await.unwrap();
    let raw = origin.http.get(format!("{}?resp=raw", origin.ndn(&crafted))).send().await.unwrap();
    assert_eq!(raw.status(), StatusCode::OK, "the source still offers the object");
    assert_eq!(origin.status(&origin.ndn(&crafted)).await, StatusCode::NOT_FOUND, "but not its bytes");
    let here_store = here.state.share.store(&here.state.config).await.unwrap();
    assert!(here_store.have_chunk(&chunk_of(victim)).await, "this zone has them");
    assert!(nfs_server::ndn::public::pull_and_bind(&here.state, &client, "photos", &crafted, &origin.ndn(&crafted)).await.is_err());
    assert_eq!(here.status(&here.ndn(&crafted)).await, StatusCode::NOT_FOUND);

    // Over its limit an app loses its oldest keys, and larger objects are never fetched.
    let second = origin.upload("alice", "second.txt", &[b'x'; 40]).await;
    origin.provide("hs/s", &second, "alice").await.unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    nfs_server::ndn::public::pull_and_bind(&here.state, &client, "photos", &second, &origin.ndn(&second)).await.unwrap();
    assert_eq!(here.status(&here.ndn(&second)).await, StatusCode::OK);
    assert_eq!(here.status(&here.ndn(&id)).await, StatusCode::OK, "16 + 40 fits in 64");
    let third = origin.upload("alice", "third.txt", &[b'y'; 40]).await;
    origin.provide("hs/t", &third, "alice").await.unwrap();
    tokio::time::sleep(Duration::from_millis(5)).await;
    nfs_server::ndn::public::pull_and_bind(&here.state, &client, "photos", &third, &origin.ndn(&third)).await.unwrap();
    assert_eq!(here.status(&here.ndn(&id)).await, StatusCode::NOT_FOUND, "the oldest key went");
    assert_eq!(here.status(&here.ndn(&third)).await, StatusCode::OK);
    let big = origin.upload("alice", "big.txt", &[b'z'; 65]).await;
    origin.provide("hs/b", &big, "alice").await.unwrap();
    assert!(nfs_server::ndn::public::pull_and_bind(&here.state, &client, "photos", &big, &origin.ndn(&big)).await.is_err());
    assert_eq!(here.status(&here.ndn(&third)).await, StatusCode::OK, "another app's quota is not its problem");

    // The API validates what the worker never gets to see, before any connection.
    let auth = via("alice", "photos");
    let (code, _) = here.call(&auth, "bucket.cache_public", json!({"root_obj_id": id.to_string(), "source": format!("https://127.0.0.1/ndn/{}", id.to_string())})).await.unwrap_err();
    assert_eq!(code, "invalid_argument");
    let (code, _) = here.call(&auth, "bucket.cache_public", json!({"root_obj_id": id.to_string(), "source": origin.ndn(&id)})).await.unwrap_err();
    assert_eq!(code, "invalid_argument");
}

#[tokio::test]
async fn publicly_provided_objects_can_be_bound_directly() {
    let srv = start().await;
    let alice = via("alice", "photos");
    let bob = via("bob", "photos");
    let id = srv.upload("alice", "a.txt", b"open to all").await;
    let private = srv.upload("alice", "b.txt", b"alice only").await;
    srv.provide("hs/e", &id, "alice").await.unwrap();
    let bob_bucket = "user/photos/bob/library";
    srv.call(&bob, "bucket.put", json!({"bucket_id": bob_bucket, "key": "from-alice", "obj_id": id.to_string()})).await.unwrap();
    let (code, _) = srv.call(&bob, "bucket.put", json!({"bucket_id": bob_bucket, "key": "stolen", "obj_id": private.to_string()})).await.unwrap_err();
    assert_eq!(code, "not_held");
    assert!(srv.call(&alice, "bucket.put", json!({"bucket_id": "user/photos/alice/library", "key": "own", "obj_id": private.to_string()})).await.is_ok());
}

#[tokio::test]
async fn anonymous_egress_is_limited_and_token_reads_are_not() {
    let srv = start_with(|c| c.ndn_anonymous_bytes_per_sec = 100_000).await;
    let data = vec![b'd'; 300_000];
    let id = srv.upload("alice", "big.bin", &data).await;
    let alice = via("alice", "photos");
    let bucket = "user/photos/alice/library";
    srv.call(&alice, "bucket.put", json!({"bucket_id": bucket, "key": "big", "obj_id": id.to_string()})).await.unwrap();
    let issued = srv.call(&alice, "bucket.create_access_token", json!({"bucket_id": bucket, "key": "big", "actions": ["read"], "expires_at": bucket_store::now_ms() + 60_000})).await.unwrap();
    let hint = NdnHint { path: None, token: Some(issued["token"].as_str().unwrap().to_string()) };

    let started = Instant::now();
    let body = srv.http.get(srv.ndn_hint(&id, &hint)).send().await.unwrap().bytes().await.unwrap();
    assert_eq!(body.len(), data.len());
    assert!(started.elapsed() < Duration::from_millis(500), "token read took {:?}", started.elapsed());

    srv.provide("hs/big", &id, "alice").await.unwrap();
    let started = Instant::now();
    let body = srv.http.get(srv.ndn(&id)).send().await.unwrap().bytes().await.unwrap();
    assert_eq!(body.len(), data.len());
    assert!(started.elapsed() >= Duration::from_millis(1500), "anonymous read took {:?}", started.elapsed());
}

#[tokio::test]
async fn ambient_content_comes_back_from_its_origin() {
    let origin = start().await;
    let here = start().await;
    let data = b"nice to have";
    let id = origin.upload("alice", "x.txt", data).await;
    origin.provide("hs/x", &id, "alice").await.unwrap();
    // Only the description is here, as a following bucket leaves it.
    let store = here.state.share.store(&here.state.config).await.unwrap().clone();
    let (file_id, body) = file_of("x.txt", data);
    assert_eq!(file_id, id);
    store.put_object(&id, &body).await.unwrap();
    let bob = via("bob", "photos");
    let bucket = "user/photos/bob/related";
    let source = format!("cyfs://{}.origin.example", id.to_base32());
    let principal = nfs_server::ndm::Principal::new("bob".into(), "app:photos".into(), "app:photos@bob".into(), None);
    nfs_server::ndm::record_holding(&here.state, &principal, &id, "register");
    let origin_field = json!({"author": "did:web:alice.origin.example", "source": source, "collected_at": 1});
    here.call(&bob, "bucket.put", json!({"bucket_id": bucket, "key": "k", "obj_id": id.to_string(), "origin": origin_field})).await.unwrap();

    // The content is missing: a read says "wait" and starts the pull.
    let url = format!("{}/bucket/v1/{bucket}/k", here.base);
    let first = here.http.get(&url).bearer_auth(&bob).send().await.unwrap();
    assert_eq!(first.status(), StatusCode::ACCEPTED);
    assert_eq!(first.headers()["retry-after"], "5");
    // `origin.example` has no address here, so the pull fails and the key stays.
    let mut status = StatusCode::ACCEPTED;
    for _ in 0..100 {
        status = here.http.get(&url).bearer_auth(&bob).send().await.unwrap().status();
        if status != StatusCode::ACCEPTED {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(status, StatusCode::NOT_FOUND);
    let gone: Value = here.http.get(&url).bearer_auth(&bob).send().await.unwrap().json().await.unwrap();
    assert_eq!(gone["error"], "origin_unavailable");
    let head = here.call(&bob, "bucket.head", json!({"bucket_id": bucket, "key": "k"})).await.unwrap();
    assert_eq!(head["availability"], "remote");

    // A pull that works makes it readable, verified against the same id.
    let client = ndn_toolkit::CyfsNdnClient::new();
    nfs_server::ndn::public::pull_verified(&client, &store, &id, &origin.ndn(&id), 1024).await.unwrap();
    let read = here.http.get(&url).bearer_auth(&bob).send().await.unwrap();
    assert_eq!(read.status(), StatusCode::OK);
    assert_eq!(read.bytes().await.unwrap().as_ref(), data);
    let head = here.call(&bob, "bucket.head", json!({"bucket_id": bucket, "key": "k"})).await.unwrap();
    assert_eq!(head["availability"], "local");
    assert_eq!(here.call(&bob, "bucket.prefetch", json!({"bucket_id": bucket, "prefix": ""})).await.unwrap()["triggered"], 0);
}

#[test]
fn an_origin_is_pulled_only_from_ndn() {
    use nfs_server::ndn::ambient::source_url;
    let id = file_of("x", b"x").0;
    let want = format!("https://z.example/ndn/{}", id.to_string());
    assert_eq!(source_url(&format!("cyfs://{}.z.example", id.to_base32()), &id).as_deref(), Some(want.as_str()));
    assert_eq!(source_url(&want, &id).as_deref(), Some(want.as_str()));
    let other = file_of("y", b"y").0;
    assert_eq!(source_url(&format!("cyfs://{}.z.example", other.to_base32()), &id), None, "another object");
    assert_eq!(source_url(&format!("cyfs://{}", id.to_base32()), &id), None, "no zone to ask");
    assert_eq!(source_url(&format!("cyfs://{}.z.example/inner", id.to_base32()), &id), None);
    assert_eq!(source_url("obj://homestation/home/alice/feed", &id), None);
    assert_eq!(source_url("did:web:z.example", &id), None);
}
