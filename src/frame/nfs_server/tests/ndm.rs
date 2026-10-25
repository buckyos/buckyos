//! The NDM gateway in nfs-server (share design D18/D19): tus uploads owned by
//! the verified caller, resume, termination and expiry, object registration
//! checks, reads limited to what a caller holds or may read, and the RBAC
//! separation of uploads, reads, retention and management. Tokens are signed
//! with the test key of tests/acl.rs; RBAC runs the real model and default
//! policy with test group lines.

use base64::Engine;
use buckyos_api::*;
use ndn_lib::NamedObject;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::{AppState, SharedState};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::path::PathBuf;

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
const AGENT: &str = "did:web:helper.test.zone";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = format!(
            "g, owner, admin\ng, alice, users\ng, bob, users\ng, app:testapp, app\ng, app:otherapp, app\ng, aicc, frame\ng, {AGENT}, agent\ng, app:helper, agent_read_write\ng, app:helper, agent_runtime\ng, app:helper, agent\ng, system:control-panel, system\n"
        );
        let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, &groups);
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

fn user(name: &str) -> Option<String> {
    Some(token(name, TokenPrincipalKind::User, "system:control-panel"))
}

struct Server {
    base: String,
    state: SharedState,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn start(ttl_secs: u64) -> Server {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("exports");
    std::fs::create_dir_all(data.join("home")).unwrap();
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: data.join("home") }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.ndm.session_ttl_secs = ttl_secs;
    let state = AppState::new(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::server::build_router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, state, http: reqwest::Client::new(), _tmp: tmp }
}

fn chunk_of(data: &[u8]) -> ndn_lib::ChunkId {
    ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap()
}

fn file_of(name: &str, data: &[u8]) -> (ndn_lib::ObjId, String) {
    ndn_lib::FileObject::new(name.into(), data.len() as u64, chunk_of(data).to_string()).gen_obj_id()
}

fn staging(srv: &Server) -> PathBuf {
    srv.state.config.ndm_staging_dir()
}

fn count(srv: &Server, sql: &str) -> i64 {
    rusqlite::Connection::open(srv.state.config.db_path()).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

impl Server {
    fn req(&self, method: Method, path: &str, auth: &Option<String>) -> reqwest::RequestBuilder {
        let r = self.http.request(method, format!("{}{}", self.base, path)).header("Tus-Resumable", "1.0.0");
        match auth {
            Some(t) => r.bearer_auth(t),
            None => r,
        }
    }

    async fn create(&self, auth: &Option<String>, data: &[u8], name: &str) -> reqwest::Response {
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        let chunk = chunk_of(data).to_string();
        let meta = format!(
            "app_id {},logical_path {},file_name {},file_hash {},chunk_index {},chunk_hash {}",
            b64("default"),
            b64(&format!("default/{chunk}")),
            b64(name),
            b64(&file_of(name, data).0.to_string()),
            b64("0"),
            b64(&chunk)
        );
        self.req(Method::POST, "/ndm/v1/uploads", auth)
            .header("Upload-Length", data.len().to_string())
            .header("Upload-Metadata", meta)
            .send()
            .await
            .unwrap()
    }

    async fn patch(&self, auth: &Option<String>, location: &str, offset: usize, data: &[u8]) -> reqwest::Response {
        self.req(Method::PATCH, location, auth)
            .header("Content-Type", "application/offset+octet-stream")
            .header("Upload-Offset", offset.to_string())
            .body(data.to_vec())
            .send()
            .await
            .unwrap()
    }

    async fn store_call(&self, auth: &Option<String>, method: &str, body: Value) -> (StatusCode, Value) {
        self.rpc(auth, &format!("/ndm/v1/store/{method}"), body).await
    }

    async fn proxy_call(&self, auth: &Option<String>, method: &str, body: Value) -> (StatusCode, Value) {
        self.rpc(auth, &format!("/ndm/proxy/v1/rpc/{method}"), body).await
    }

    async fn rpc(&self, auth: &Option<String>, path: &str, body: Value) -> (StatusCode, Value) {
        let mut r = self.http.post(format!("{}{}", self.base, path)).json(&body);
        if let Some(t) = auth {
            r = r.bearer_auth(t);
        }
        let resp = r.send().await.unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    }

    async fn write_chunk(&self, auth: &Option<String>, data: &[u8]) -> StatusCode {
        let mut r = self
            .http
            .put(format!("{}/ndm/proxy/v1/write/chunk/{}", self.base, chunk_of(data).to_string()))
            .header("NDM-Chunk-Size", data.len().to_string())
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec());
        if let Some(t) = auth {
            r = r.bearer_auth(t);
        }
        r.send().await.unwrap().status()
    }
}

async fn open(srv: &Server, who: &Option<String>, id: &ndn_lib::ObjId) -> StatusCode {
    srv.rpc(who, "/ndm/proxy/v1/read/object/open", json!({"obj_id": id.to_string()})).await.0
}

fn header(r: &reqwest::Response, name: &str) -> String {
    r.headers().get(name).map(|v| v.to_str().unwrap().to_string()).unwrap_or_default()
}

#[tokio::test]
async fn uploads_belong_to_their_session_and_resume() {
    let srv = start(3600).await;
    let (alice, bob, owner) = (user("alice"), user("bob"), user("owner"));
    let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();

    // Anonymous callers get nothing.
    assert_eq!(srv.create(&None, &data, "a.bin").await.status(), StatusCode::UNAUTHORIZED);
    let (status, _) = srv.store_call(&None, "is_object_exist", json!({"obj_id": file_of("a.bin", &data).0.to_string()})).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let created = srv.create(&alice, &data, "a.bin").await;
    assert_eq!(created.status(), StatusCode::CREATED);
    let location = header(&created, "location");
    // The same chunk again finds the same session (idempotent create).
    assert_eq!(header(&srv.create(&alice, &data, "a.bin").await, "location"), location);
    // Bob's identical upload is his own session.
    let bobs = srv.create(&bob, &data, "a.bin").await;
    assert_ne!(header(&bobs, "location"), location);

    let first = srv.patch(&alice, &location, 0, &data[..40_000]).await;
    assert_eq!(first.status(), StatusCode::NO_CONTENT);
    assert_eq!(header(&first, "upload-offset"), "40000");
    for (method, who) in [(Method::HEAD, &bob), (Method::DELETE, &bob)] {
        assert_eq!(srv.req(method, &location, who).send().await.unwrap().status(), StatusCode::NOT_FOUND, "someone else's session");
    }
    assert_eq!(srv.patch(&bob, &location, 40_000, &data[40_000..]).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(srv.patch(&alice, &location, 10, &data[10..20]).await.status(), StatusCode::CONFLICT);
    let head = srv.req(Method::HEAD, &location, &alice).send().await.unwrap();
    assert_eq!(header(&head, "upload-offset"), "40000");

    // Resume after a restart of the server state: sessions live in filedb.
    let _restarted = AppState::new(srv.state.config.clone()).unwrap();
    assert_eq!(count(&srv, "SELECT MAX(offset) FROM ndm_uploads"), 40_000);
    let done = srv.patch(&alice, &location, 40_000, &data[40_000..]).await;
    assert_eq!(done.status(), StatusCode::NO_CONTENT);
    assert_eq!(header(&done, "ndm-chunk-status"), "completed");
    assert_eq!(header(&done, "ndm-chunk-object-id"), chunk_of(&data).to_obj_id().to_string());

    // Registering the FileObject needs the chunk to be the caller's own upload.
    let (file_id, file_body) = file_of("a.bin", &data);
    let put = json!({"obj_id": file_id.to_string(), "obj_data": file_body});
    let (status, err) = srv.store_call(&user("owner"), "put_object", json!({"obj_id": file_id.to_string(), "obj_data": "{\"name\":\"x\"}"})).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the body must hash to the id: {err}");
    let (status, err) = srv.store_call(&bob, "put_object", put.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "bob did not upload the chunk: {err}");
    assert_eq!(srv.store_call(&alice, "put_object", put.clone()).await.0, StatusCode::NO_CONTENT);

    // Reads: the uploader and holders of the general read right only.
    let read = json!({"obj_id": file_id.to_string()});
    assert_eq!(srv.store_call(&alice, "get_object", read.clone()).await.0, StatusCode::OK);
    assert_eq!(srv.store_call(&bob, "get_object", read.clone()).await.0, StatusCode::FORBIDDEN);
    assert_eq!(srv.store_call(&owner, "get_object", read.clone()).await.0, StatusCode::OK);
    let lookup = srv
        .http
        .get(format!("{}/ndm/v1/objects/lookup?scope=app&quick_hash={}", srv.base, file_id.to_string()))
        .bearer_auth(bob.clone().unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(lookup.status(), StatusCode::OK, "existence checks serve upload dedupe");
    assert_eq!(lookup.json::<Value>().await.unwrap()["reusable"], false);

    // A finished upload holds its chunk, the registration its object, through a GC.
    let store = srv.state.share.store(&srv.state.config).await.unwrap();
    let id = store.get_store_ids().await.pop().unwrap();
    let report = store.get_store(&id).await.unwrap().lock().await.gc_round(0).await.unwrap();
    assert_eq!(report.evicted_objects, 0, "{report:?}");
    assert!(store.is_object_exist(&file_id).await.unwrap());
}

#[tokio::test]
async fn import_finalize_requires_complete_owned_dependencies_and_is_retryable() {
    let srv = start(3600).await;
    let (alice, bob) = (user("alice"), user("bob"));
    let data = b"finalize file contents";
    let (id, body) = file_of("ready.txt", data);
    let manifest = json!({"objects": [{"obj_id": id.to_string(), "obj_data": body}], "roots": [id.to_string()]});
    let endpoint = "/ndm/v1/import/finalize";
    assert_eq!(srv.rpc(&None, endpoint, manifest.clone()).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(srv.rpc(&alice, endpoint, manifest.clone()).await.0, StatusCode::CONFLICT);
    assert!(srv.write_chunk(&alice, data).await.is_success());
    assert_eq!(srv.rpc(&bob, endpoint, manifest.clone()).await.0, StatusCode::CONFLICT);
    for _ in 0..2 {
        let (status, result) = srv.rpc(&alice, endpoint, manifest.clone()).await;
        assert_eq!(status, StatusCode::OK, "{result}");
        assert_eq!(result["ready"], true);
        assert_eq!(result["roots"][0]["obj_id"], id.to_string());
    }
    assert_eq!(open(&srv, &alice, &id).await, StatusCode::OK);
    assert_eq!(open(&srv, &bob, &id).await, StatusCode::FORBIDDEN);
    let store = srv.state.share.store(&srv.state.config).await.unwrap();
    let retention = store.debug_dump_expand_state(&id).await.unwrap();
    assert!(retention.has_recursive_pin && retention.children_expanded);
    let reused = srv.create(&bob, data, "known.txt").await;
    assert_eq!(header(&reused, "upload-offset"), "0", "knowing a chunk hash must not grant its bytes");
    let mut corrupt = manifest;
    corrupt["objects"][0]["obj_data"] = json!("{}");
    assert_eq!(srv.rpc(&alice, endpoint, corrupt).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn import_finalize_handles_chunk_lists_in_dependency_order() {
    let srv = start(3600).await;
    let alice = user("alice");
    let first = b"first chunk";
    let second = b"second chunk";
    for bytes in [first.as_slice(), second.as_slice()] {
        assert!(srv.write_chunk(&alice, bytes).await.is_success());
    }
    let (list, list_body) = ndn_lib::ChunkList::from_chunk_list(vec![chunk_of(first), chunk_of(second)]).unwrap().gen_obj_id();
    let (file, file_body) = ndn_lib::FileObject::new("chunks.txt".into(), (first.len() + second.len()) as u64, list.to_string()).gen_obj_id();
    let manifest = json!({"objects": [
        {"obj_id": file.to_string(), "obj_data": file_body},
        {"obj_id": list.to_string(), "obj_data": list_body}
    ], "roots": [file.to_string()], "qcid_links": [{"qcid": "bad", "content_id": list.to_string(), "size": 23}]});
    let (status, result) = srv.rpc(&alice, "/ndm/v1/import/finalize", manifest).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["ready"], true);
    assert_eq!(result["warnings"].as_array().unwrap().len(), 1);
    let store = srv.state.share.store(&srv.state.config).await.unwrap();
    let (mut reader, _) = store.open_reader(&file, None).await.unwrap();
    let mut bytes = Vec::new();
    tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut bytes).await.unwrap();
    assert_eq!(bytes, [first.as_slice(), second.as_slice()].concat());
}

#[tokio::test]
async fn import_holdings_follow_the_user_and_canonical_app_instance() {
    let srv = start(3600).await;
    let alice = Some(token("alice", TokenPrincipalKind::User, "app:testapp@alice"));
    let shared = Some(token("alice", TokenPrincipalKind::User, "app:testapp@bob"));
    let bob = Some(token("bob", TokenPrincipalKind::User, "app:testapp@bob"));
    let data = b"same bytes, separate authorized holdings";
    let (id, body) = file_of("shared.txt", data);
    let manifest = json!({"objects": [{"obj_id": id.to_string(), "obj_data": body}], "roots": [id.to_string()]});
    assert!(srv.write_chunk(&alice, data).await.is_success());
    assert_eq!(srv.rpc(&alice, "/ndm/v1/import/finalize", manifest.clone()).await.0, StatusCode::OK);
    for other in [&shared, &bob] {
        assert_eq!(srv.rpc(other, "/ndm/v1/import/finalize", manifest.clone()).await.0, StatusCode::CONFLICT);
        assert_eq!(open(&srv, other, &id).await, StatusCode::FORBIDDEN);
    }
    assert!(srv.write_chunk(&shared, data).await.is_success());
    assert_eq!(srv.rpc(&shared, "/ndm/v1/import/finalize", manifest).await.0, StatusCode::OK);
    assert_eq!(open(&srv, &shared, &id).await, StatusCode::OK);
    assert_eq!(open(&srv, &bob, &id).await, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn empty_tus_upload_is_ready_without_a_patch() {
    let srv = start(3600).await;
    let alice = user("alice");
    let upload = srv.create(&alice, b"", "empty").await;
    assert!(upload.status().is_success());
    assert_eq!(header(&upload, "ndm-chunk-object-id"), chunk_of(b"").to_string());
    let (id, body) = file_of("empty", b"");
    let (status, result) = srv.rpc(&alice, "/ndm/v1/import/finalize", json!({"objects":[{"obj_id":id.to_string(),"obj_data":body}],"roots":[id.to_string()]})).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["ready"], true);
}

#[tokio::test]
async fn qcid_reuse_preserves_single_chunk_identity_and_checks_sampled_bytes() {
    let srv = start(3600).await;
    let alice = user("alice");
    let data = vec![71; 48 * 1024];
    assert!(srv.write_chunk(&alice, &data).await.is_success());
    let qcid = ndn_lib::calc_quick_hash_by_buffer(&data).unwrap().to_string();
    let (file, body) = file_of("medium.bin", &data);
    let mut manifest = json!({"objects":[{"obj_id":file.to_string(),"obj_data":body}],"roots":[file.to_string()],
        "qcid_links":[{"qcid": qcid,"content_id": chunk_of(&data).to_string(),"size":data.len()}]});
    let (status, result) = srv.rpc(&alice, "/ndm/v1/import/finalize", manifest.clone()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["warnings"], json!([]));
    let lookup: Value = srv.http.get(format!("{}/ndm/v1/objects/lookup?scope=app&quick_hash={qcid}", srv.base)).bearer_auth(alice.as_ref().unwrap()).send().await.unwrap().json().await.unwrap();
    assert_eq!(lookup["reusable"], true, "{lookup}");
    assert_eq!(lookup["content_id"], chunk_of(&data).to_string());
    assert_eq!(srv.http.get(format!("{}/ndm/v1/objects/lookup?scope=invalid&quick_hash={qcid}", srv.base)).bearer_auth(alice.as_ref().unwrap()).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    manifest["qcid_links"][0]["qcid"] = json!(ndn_lib::calc_quick_hash_by_buffer(&vec![72; data.len()]).unwrap().to_string());
    let (_, invalid) = srv.rpc(&alice, "/ndm/v1/import/finalize", manifest).await;
    assert_eq!(invalid["ready"], true);
    assert_eq!(invalid["warnings"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn stale_local_links_require_bytes_or_reuse_a_committed_copy() {
    for committed in [false, true] {
        let srv = start(3600).await;
        let owner = user("owner");
        let data = b"attachment survives a stale local link only with real bytes";
        let chunk = chunk_of(data);
        let source = srv._tmp.path().join("source.png");
        tokio::fs::write(&source, data).await.unwrap();
        let (qcid, metadata) = ndn_lib::calculate_qcid_from_file_with_metadata(&source).await.unwrap();
        let info = named_store::ChunkLocalInfo::new(source.to_string_lossy().into_owned(), qcid.to_string(), &metadata, None);
        let store = srv.state.share.store(&srv.state.config).await.unwrap();
        if committed {
            store.put_chunk(&chunk, data).await.unwrap();
        }
        store.add_chunk_by_link_to_local_file(&chunk, data.len() as u64, &info).await.unwrap();
        let (id, body) = file_of("attachment.png", data);
        let manifest = json!({"objects":[{"obj_id":id.to_string(),"obj_data":body}],"roots":[id.to_string()],
            "qcid_links":[{"qcid":qcid.to_string(),"content_id":chunk.to_string(),"size":data.len()}]});
        let endpoint = "/ndm/v1/import/finalize";
        assert_eq!(srv.rpc(&owner, endpoint, manifest.clone()).await.0, StatusCode::OK);
        tokio::fs::remove_file(&source).await.unwrap();

        assert_eq!(store.is_object_stored(&id, None).await.unwrap(), committed);
        for hash in [qcid.to_string(), chunk.to_string()] {
            let lookup: Value = srv.req(Method::GET, &format!("/ndm/v1/objects/lookup?scope=app&quick_hash={hash}"), &owner)
                .send().await.unwrap().json().await.unwrap();
            assert_eq!(lookup["state"], if committed { "completed" } else { "not_exist" }, "{lookup}");
        }
        if !committed {
            let (status, rejected) = srv.rpc(&owner, endpoint, manifest.clone()).await;
            assert_eq!(status, StatusCode::CONFLICT, "{rejected}");
            assert_ne!(rejected["ready"], true);
            let upload = srv.create(&owner, data, "attachment.png").await;
            assert_eq!(upload.status(), StatusCode::CREATED);
            assert_eq!(header(&upload, "upload-offset"), "0");
            assert_eq!(srv.patch(&owner, &header(&upload, "location"), 0, data).await.status(), StatusCode::NO_CONTENT);
        }
        let (status, ready) = srv.rpc(&owner, endpoint, manifest).await;
        assert_eq!(status, StatusCode::OK, "{ready}");
        assert_eq!(ready["ready"], true);
        assert_eq!(ready["warnings"], json!([]));
        let response = srv.req(Method::POST, "/ndm/proxy/v1/read/object/open", &owner)
            .json(&json!({"obj_id":id.to_string()})).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.bytes().await.unwrap().as_ref(), data);
    }
}

#[tokio::test]
async fn termination_and_expiry_clean_up_only_their_own_staging() {
    let srv = start(1).await;
    let alice = user("alice");
    let data = vec![7u8; 50_000];
    let other = vec![8u8; 30_000];

    let location = header(&srv.create(&alice, &data, "d.bin").await, "location");
    assert_eq!(srv.patch(&alice, &location, 0, &data[..10_000]).await.status(), StatusCode::NO_CONTENT);
    let id = location.rsplit('/').next().unwrap().to_string();
    assert!(staging(&srv).join(format!("{id}.part")).exists());
    let gone = srv.req(Method::DELETE, &location, &alice).send().await.unwrap();
    assert_eq!(gone.status(), StatusCode::NO_CONTENT);
    assert!(!staging(&srv).join(format!("{id}.part")).exists());
    assert_eq!(srv.req(Method::HEAD, &location, &alice).send().await.unwrap().status(), StatusCode::NOT_FOUND);

    // A completed upload's termination releases its lease, never the chunk.
    let done = header(&srv.create(&alice, &other, "o.bin").await, "location");
    assert_eq!(srv.patch(&alice, &done, 0, &other).await.status(), StatusCode::NO_CONTENT);
    let again = srv.create(&alice, &other, "o2.bin").await;
    assert_eq!(header(&again, "ndm-chunk-status"), "skipped", "the chunk is already stored");
    let store = srv.state.share.store(&srv.state.config).await.unwrap();
    let chunk = chunk_of(&other).to_obj_id();
    let done_id = done.rsplit('/').next().unwrap();
    let again_id = header(&again, "location").rsplit('/').next().unwrap().to_string();
    assert!(store.anchor_state(&chunk, &format!("ndm-upload:{done_id}")).await.is_ok());
    assert_eq!(srv.req(Method::DELETE, &done, &alice).send().await.unwrap().status(), StatusCode::NO_CONTENT);
    assert!(store.anchor_state(&chunk, &format!("ndm-upload:{done_id}")).await.is_err());
    assert!(store.anchor_state(&chunk, &format!("ndm-upload:{again_id}")).await.is_ok(), "the other session keeps its lease");
    assert!(store.have_chunk(&chunk_of(&other)).await);

    // Idle unfinished uploads expire with their staged bytes.
    let idle = header(&srv.create(&alice, &data, "d.bin").await, "location");
    srv.patch(&alice, &idle, 0, &data[..5_000]).await;
    let idle_id = idle.rsplit('/').next().unwrap().to_string();
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert_eq!(srv.patch(&alice, &idle, 5_000, &data[5_000..]).await.status(), StatusCode::GONE);
    let stale = header(&srv.create(&alice, &data, "e.bin").await, "location");
    srv.patch(&alice, &stale, 0, &data[..5_000]).await;
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    nfs_server::ndm::uploads::sweep(&srv.state).await;
    assert!(!staging(&srv).join(format!("{idle_id}.part")).exists());
    let stale_id = stale.rsplit('/').next().unwrap();
    assert!(!staging(&srv).join(format!("{stale_id}.part")).exists());
    assert_eq!(count(&srv, "SELECT COUNT(*) FROM ndm_uploads WHERE status='uploading'"), 0);
}

#[tokio::test]
async fn proxy_permissions_separate_uploads_reads_retention_and_management() {
    let srv = start(3600).await;
    let app = Some(token("alice", TokenPrincipalKind::App, "app:testapp@alice"));
    let agent = Some(token(AGENT, TokenPrincipalKind::Agent, "app:helper@owner"));
    let frame = Some(token("aicc", TokenPrincipalKind::System, "system:aicc"));
    let (alice, owner) = (user("alice"), user("owner"));
    let data = vec![3u8; 20_000];
    let (file_id, file_body) = file_of("p.bin", &data);
    let put = json!({"obj_id": file_id.to_string(), "obj_data": file_body});
    let read = json!({"obj_id": file_id.to_string()});

    // An installed app may upload and register, then read back only what it holds.
    assert!(srv.write_chunk(&app, &data).await.is_success());
    assert_eq!(srv.proxy_call(&app, "put_object", put.clone()).await.0, StatusCode::NO_CONTENT);
    assert_eq!(srv.proxy_call(&app, "query_object_by_id", read.clone()).await.0, StatusCode::OK);
    let (other_id, other_body) = file_of("q.bin", &[4u8; 1000]);
    assert!(srv.write_chunk(&alice, &[4u8; 1000]).await.is_success());
    assert_eq!(srv.proxy_call(&alice, "put_object", json!({"obj_id": other_id.to_string(), "obj_data": other_body})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(srv.proxy_call(&app, "get_object", json!({"obj_id": other_id.to_string()})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(open(&srv, &app, &other_id).await, StatusCode::FORBIDDEN);
    assert_eq!(open(&srv, &None, &file_id).await, StatusCode::UNAUTHORIZED);
    // Management and other owners' retention are out of its reach.
    for (method, body) in [
        ("remove_object", read.clone()),
        ("outbox_count", json!({})),
        ("forced_gc_until", json!({"target_bytes": 0})),
        ("pin", json!({"obj_id": file_id.to_string(), "owner": "share:x", "scope": "recursive"})),
        ("unpin_owner", json!({"owner": "msg-center:x"})),
    ] {
        assert_eq!(srv.proxy_call(&app, method, body).await.0, StatusCode::FORBIDDEN, "{method}");
    }

    // An agent may read objects (AIGC artifacts) but not upload.
    assert_eq!(open(&srv, &agent, &other_id).await, StatusCode::OK);
    assert_eq!(srv.write_chunk(&agent, &[5u8; 10]).await, StatusCode::FORBIDDEN);

    // A frame service keeps its own retention, not others'.
    let own = format!("system:aicc/aicc/{}", "results");
    assert_eq!(srv.proxy_call(&frame, "pin", json!({"obj_id": file_id.to_string(), "owner": own, "scope": "recursive"})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(srv.proxy_call(&frame, "unpin", json!({"obj_id": file_id.to_string(), "owner": "share:x"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(srv.proxy_call(&frame, "remove_object", read.clone()).await.0, StatusCode::FORBIDDEN);

    // The zone owner manages the store.
    assert_eq!(srv.proxy_call(&owner, "outbox_count", json!({})).await.0, StatusCode::OK);
    assert_eq!(srv.proxy_call(&owner, "unpin", json!({"obj_id": file_id.to_string(), "owner": own})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(srv.proxy_call(&owner, "remove_object", json!({"obj_id": other_id.to_string()})).await.0, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn commit_named_objects_materializes_chunklists_and_directories_with_acl() {
    let srv = start(3600).await;
    let alice = user("alice");
    let bob = user("bob");
    let store = nfs_server::ndm::store(&srv.state).await.unwrap();
    let chunks = [b"first".as_slice(), b"second".as_slice()];
    for bytes in chunks { assert!(srv.write_chunk(&alice, bytes).await.is_success()); }
    let (list_id, list_json) = ndn_lib::ChunkList::from_chunk_list(chunks.iter().map(|data| chunk_of(data)).collect()).unwrap().gen_obj_id();
    assert!(srv.store_call(&alice, "put_object", json!({"obj_id": list_id.to_string(), "obj_data": list_json})).await.0.is_success());
    let file = ndn_lib::FileObject::new("data.txt".into(), 11, list_id.to_string());
    let (file_id, file_json) = file.gen_obj_id();
    assert!(srv.store_call(&alice, "put_object", json!({"obj_id":file_id.to_string(), "obj_data":file_json})).await.0.is_success());
    let empty = ndn_lib::DirObject::new(Some("empty".into()));
    let (empty_id, empty_json) = empty.gen_obj_id().unwrap();
    assert!(srv.store_call(&alice, "put_object", json!({"obj_id":empty_id.to_string(), "obj_data":empty_json})).await.0.is_success());
    let mut dir = ndn_lib::DirObject::new(Some("root".into()));
    dir.object_map.body.insert("data.txt".into(), ndn_lib::SimpleMapItem::ObjId(file_id.clone()));
    dir.object_map.body.insert("empty".into(), ndn_lib::SimpleMapItem::ObjId(empty_id));
    let (dir_id, dir_json) = dir.gen_obj_id().unwrap();
    assert!(srv.store_call(&alice, "put_object", json!({"obj_id":dir_id.to_string(), "obj_data":dir_json})).await.0.is_success());
    assert!(store.is_object_stored(&dir_id, None).await.unwrap());
    let (_, hello) = srv.rpc(&alice, "/nfs/v1/hello", json!({"args":{}})).await;
    let session = hello["result"]["session"].as_str().unwrap();
    let request = json!({"session":session,"seq":1,"at":{"path":"/home/alice"},"args":{"name":"folder","obj_id":dir_id.to_string()}});
    let (status, committed) = srv.rpc(&alice, "/nfs/v1/commit_object", request.clone()).await;
    assert_eq!(status, StatusCode::OK, "{committed}");
    assert_eq!(committed["result"]["kind"], "dir");
    let home = &srv.state.config.root("home").unwrap().path;
    assert_eq!(std::fs::read(home.join("alice/folder/data.txt")).unwrap(), b"firstsecond");
    assert!(home.join("alice/folder/empty").is_dir());
    let (_, replay) = srv.rpc(&alice, "/nfs/v1/commit_object", request).await;
    assert_eq!(replay["result"], committed["result"]);
    let (status, _) = srv.rpc(&alice, "/nfs/v1/commit_object", json!({"session":session,"seq":2,"at":{"path":"/home/alice"},"args":{"name":"folder","obj_id":dir_id.to_string()}})).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = srv.rpc(&alice, "/nfs/v1/commit_object", json!({"session":session,"seq":3,"at":{"path":"/home/alice"},"args":{"name":"other","obj_id":file_id.to_string(),"expected_revision":"stale"}})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    // A different existing tree is never replaced, even with overwrite=true.
    std::fs::write(home.join("alice/folder/data.txt"), b"edited locally").unwrap();
    let (status, _) = srv.rpc(&alice, "/nfs/v1/commit_object", json!({"session":session,"seq":4,"at":{"path":"/home/alice"},"args":{"name":"folder","obj_id":dir_id.to_string(),"overwrite":true}})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(std::fs::read(home.join("alice/folder/data.txt")).unwrap(), b"edited locally");
    assert!(std::fs::read_dir(home.join("alice")).unwrap().all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".nfs_import_")));
    #[cfg(unix)] {
        std::os::unix::fs::symlink(home.join("alice/folder"), home.join("alice/link")).unwrap();
        let (status, _) = srv.rpc(&alice, "/nfs/v1/commit_object", json!({"session":session,"seq":5,"at":{"path":"/home/alice/link/empty"},"args":{"name":"escaped","obj_id":dir_id.to_string()}})).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!home.join("alice/folder/empty/escaped").exists());
    }
    let (_, bob_hello) = srv.rpc(&bob, "/nfs/v1/hello", json!({"args":{}})).await;
    let (status, _) = srv.rpc(&bob, "/nfs/v1/commit_object", json!({"session":bob_hello["result"]["session"],"seq":1,"at":{"path":"/home/bob"},"args":{"name":"stolen","obj_id":dir_id.to_string()}})).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!home.join("bob/stolen").exists());
}

#[tokio::test]
async fn runtime_named_store_round_trip_preserves_app_and_user_authorization() {
    use tokio::io::AsyncReadExt;

    static GATEWAY_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct GatewayEnv(Option<std::ffi::OsString>);

    impl Drop for GatewayEnv {
        fn drop(&mut self) {
            match &self.0 {
                Some(value) => std::env::set_var("BUCKYOS_HOST_GATEWAY", value),
                None => std::env::remove_var("BUCKYOS_HOST_GATEWAY"),
            }
        }
    }

    async fn runtime(
        srv: &Server,
        root: &std::path::Path,
        app: &str,
        user: &str,
    ) -> BuckyOSRuntime {
        let mut runtime =
            BuckyOSRuntime::new(app, Some(user.into()), BuckyOSRuntimeType::AppClient);
        runtime.buckyos_root_dir = root.to_path_buf();
        runtime.node_gateway_port = reqwest::Url::parse(&srv.base).unwrap().port().unwrap();
        *runtime.session_token.write().await =
            token(user, TokenPrincipalKind::User, &format!("app:{app}@{user}"));
        runtime
    }

    let _env_lock = GATEWAY_ENV_LOCK.lock().unwrap();
    let _env = GatewayEnv(std::env::var_os("BUCKYOS_HOST_GATEWAY"));
    std::env::set_var("BUCKYOS_HOST_GATEWAY", "127.0.0.1");
    let srv = start(3600).await;
    let client_root = tempfile::tempdir().unwrap();
    let local_config = client_root.path().join("storage/named_store.json");
    assert!(!local_config.exists());
    let alice = runtime(&srv, client_root.path(), "testapp", "alice").await;
    assert!(alice.get_local_named_store().await.is_err());
    let store = alice.get_named_store().await.unwrap();
    let bytes = b"\x89PNG\r\n\x1a\nimage attachment bytes through the runtime SDK\x00\xff";
    let chunk = chunk_of(bytes);
    let (id, body) = file_of("attachment.png", bytes);
    store.put_chunk(&chunk, bytes).await.unwrap();
    store.put_object(&id, &body).await.unwrap();
    assert_eq!(store.get_object(&id).await.unwrap(), body);
    let (mut reader, size) = store.open_reader(&id, None).await.unwrap();
    let mut received = Vec::new();
    reader.read_to_end(&mut received).await.unwrap();
    assert_eq!(size, bytes.len() as u64);
    assert_eq!(received, bytes);

    for (app, user) in [("otherapp", "alice"), ("testapp", "bob")] {
        let other = runtime(&srv, client_root.path(), app, user).await;
        let other_store = other.get_named_store().await.unwrap();
        assert!(matches!(
            other_store.get_object(&id).await,
            Err(ndn_lib::NdnError::PermissionDenied(_))
        ));
        assert!(matches!(
            other_store.open_reader(&id, None).await,
            Err(ndn_lib::NdnError::PermissionDenied(_))
        ));
    }

    assert_eq!(store.get_object(&id).await.unwrap(), body);
    let (mut reader, size) = store.open_reader(&id, None).await.unwrap();
    received.clear();
    reader.read_to_end(&mut received).await.unwrap();
    assert_eq!(size, bytes.len() as u64);
    assert_eq!(received, bytes);
    assert!(!local_config.exists());
}
