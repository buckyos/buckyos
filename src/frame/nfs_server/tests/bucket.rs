//! Application object buckets through nfs-server: buckets declared by app
//! specs and created lazily, kRPC `bucket.*`, the `/bucket/v1` data plane with
//! RBAC (real model + default policy), capability tokens, holding proof and
//! `auto_bind` on import finalize. Tokens use the test key of tests/ndm.rs.

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use ndn_lib::NamedObject;
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::{AppState, SharedState};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = "g, owner, admin\ng, alice, users\ng, bob, users\ng, app:photos, app\ng, app:editor, app\ng, system:control-panel, system\n";
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

/// The user in their own system UI.
fn ui(name: &str) -> String {
    token(name, TokenPrincipalKind::User, "system:control-panel")
}

/// The user signed in on an app's origin.
fn via(name: &str, app: &str) -> String {
    token(name, TokenPrincipalKind::User, &format!("app:{app}@{name}"))
}

struct Server {
    base: String,
    state: SharedState,
    http: reqwest::Client,
    _tmp: tempfile::TempDir,
}

async fn start() -> Server {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("exports");
    std::fs::create_dir_all(data.join("home/alice")).unwrap();
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: data.join("home") }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.bucket_specs = serde_json::from_value(json!({
        "photos": {
            "storage_quota_mib": 1,
            "buckets": {
                "library": {"kind": "data", "data_policy": "documents"},
                "index": {"kind": "data", "data_policy": "app_data", "quota_mib": 1},
                "drop": {"kind": "data", "data_policy": "app_data", "scope": "zone"}
            }
        },
        "editor": {"buckets": {"notes": {"kind": "object", "data_policy": "documents"}}}
    }))
    .unwrap();
    let state = AppState::new(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::server::build_router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Server { base, state, http: reqwest::Client::new(), _tmp: tmp }
}

impl Server {
    async fn call(&self, token: &str, method: &str, params: Value) -> Result<Value, (String, Value)> {
        let body = json!({"method": method, "params": params, "sys": [1, token]});
        let v: Value = self.http.post(format!("{}/kapi/nfs-server", self.base)).json(&body).send().await.unwrap().json().await.unwrap();
        match v.get("result") {
            Some(r) => Ok(r.clone()),
            None => Err((v["code"].as_str().unwrap_or_default().to_string(), v)),
        }
    }

    fn data(&self, method: Method, path: &str, auth: Option<&str>) -> reqwest::RequestBuilder {
        let r = self.http.request(method, format!("{}/bucket/v1/{}", self.base, path));
        match auth {
            Some(t) => r.bearer_auth(t),
            None => r,
        }
    }

    async fn write_chunk(&self, auth: &str, data: &[u8]) -> StatusCode {
        let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap();
        self.http
            .put(format!("{}/ndm/proxy/v1/write/chunk/{}", self.base, chunk.to_string()))
            .header("NDM-Chunk-Size", data.len().to_string())
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .status()
    }
}

fn file_of(name: &str, data: &[u8]) -> (ndn_lib::ObjId, String) {
    let chunk = ndn_lib::ChunkHasher::new(None).unwrap().calc_mix_chunk_id_from_bytes(data).unwrap();
    ndn_lib::FileObject::new(name.into(), data.len() as u64, chunk.to_string()).gen_obj_id()
}

const LIB: &str = "user/photos/alice/library";

#[tokio::test]
async fn declared_buckets_data_plane_and_rbac() {
    let srv = start().await;
    let alice_photos = via("alice", "photos");

    // Only declared buckets exist (D7).
    let resp = srv.data(Method::PUT, "user/photos/alice/nope/a.txt", Some(&alice_photos)).body("x").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    let put = srv
        .data(Method::PUT, &format!("{LIB}/2026/a.txt"), Some(&alice_photos))
        .header("content-type", "text/plain")
        .header("x-bucky-meta-album", "trip")
        .body("hello bucket")
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), StatusCode::CREATED);
    let put: Value = put.json().await.unwrap();
    let obj_id = put["obj_id"].as_str().unwrap().to_string();

    // `<img src>` style: the SSO cookie authenticates the read.
    let get = srv
        .http
        .get(format!("{}/bucket/v1/{LIB}/2026/a.txt", srv.base))
        .header("cookie", format!("buckyos_session_token={}", ui("alice")))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(get.headers()["etag"], format!("\"{obj_id}\""));
    assert_eq!(get.headers()["x-bucky-meta-album"], "trip");
    assert_eq!(get.text().await.unwrap(), "hello bucket");
    let range = srv.data(Method::GET, &format!("{LIB}/2026/a.txt"), Some(&alice_photos)).header("range", "bytes=6-").send().await.unwrap();
    assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(range.text().await.unwrap(), "bucket");

    // Other users, other apps, anonymous: nothing.
    for who in [Some(ui("bob")), Some(via("alice", "editor")), None] {
        let r = srv.data(Method::GET, &format!("{LIB}/2026/a.txt"), who.as_deref()).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN);
    }
    // The zone owner may list and read (D16) but not write.
    assert_eq!(srv.data(Method::GET, &format!("{LIB}/2026/a.txt"), Some(&ui("owner"))).send().await.unwrap().status(), StatusCode::OK);
    assert_eq!(
        srv.data(Method::PUT, &format!("{LIB}/admin.txt"), Some(&ui("owner"))).body("x").send().await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    // kRPC: list, stat, objects.
    let list = srv.call(&alice_photos, "bucket.list", json!({})).await.unwrap();
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|b| b["bucket_id"].as_str().unwrap()).collect();
    assert!(ids.contains(&LIB) && ids.contains(&"user/photos/alice/index"), "{ids:?}");
    assert!(!ids.contains(&"zone/photos/_/drop"), "zone buckets are not shared by default");
    let objs = srv.call(&alice_photos, "bucket.list_objects", json!({"bucket_id": LIB, "delimiter": "/"})).await.unwrap();
    assert_eq!(objs["common_prefixes"], json!(["2026/"]));
    let stat = srv.call(&alice_photos, "bucket.stat", json!({"bucket_id": LIB})).await.unwrap();
    assert_eq!((stat["key_count"].as_u64(), stat["versioning"].as_bool()), (Some(1), Some(true)));

    // Knowing alice's ObjId does not let bob bind it (§6.4).
    let err = srv.call(&via("bob", "photos"), "bucket.put", json!({"bucket_id": "user/photos/bob/index", "key": "stolen", "obj_id": obj_id})).await;
    assert_eq!(err.unwrap_err().0, "not_held");

    // An app does not manage user documents; the user does, from their own UI.
    let grant = json!({"bucket_id": LIB, "grantee": {"kind": "zone_users"}, "prefix": "2026/", "actions": ["list", "read"]});
    assert_eq!(srv.call(&alice_photos, "bucket.grant", grant.clone()).await.unwrap_err().0, "permission_denied");
    srv.call(&ui("alice"), "bucket.grant", grant).await.unwrap();
    let bobs_view = srv.call(&ui("bob"), "bucket.list_objects", json!({"bucket_id": LIB})).await.unwrap();
    assert_eq!(bobs_view["items"].as_array().unwrap().len(), 1, "the family album grant");
    // Holding read is enough to hand out a read link (§4.4.5).
    let req = json!({"bucket_id": LIB, "key": "2026/a.txt", "actions": ["read"], "expires_at": bucket_store::now_ms() + 600_000});
    let issued = srv.call(&alice_photos, "bucket.create_access_token", req).await.unwrap();
    let path = issued["path"].as_str().unwrap();
    let shared = srv.http.get(format!("{}{}", srv.base, path)).send().await.unwrap();
    assert_eq!(shared.status(), StatusCode::OK, "a capability token needs no Zone identity");
    let token = issued["token"].as_str().unwrap();
    let listing = srv.call("", "bucket.list_objects", json!({"bucket_id": LIB, "access_token": token})).await;
    assert_eq!(listing.unwrap_err().0, "permission_denied");

    // Copying within alice's own buckets is fine (read on the source).
    let copied = srv
        .call(&alice_photos, "bucket.copy", json!({"src_bucket_id": LIB, "src_key": "2026/a.txt", "dst_bucket_id": "user/photos/alice/index", "dst_key": "a"}))
        .await
        .unwrap();
    assert_eq!(copied["etag"], json!(obj_id));

    // Quota: the 1 MiB app bucket refuses new bytes beyond it.
    let big = vec![1u8; 1024 * 1024];
    let r = srv.data(Method::PUT, "user/photos/alice/index/big", Some(&alice_photos)).body(big).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 507);
    assert_eq!(r.json::<Value>().await.unwrap()["data"]["level"], "bucket");

    let report = srv.call(&ui("owner"), "bucket.usage_report", json!({"kind": "zone"})).await.unwrap();
    assert!(report["by_policy"]["documents"].as_u64().unwrap() > 0);
    assert_eq!(srv.call(&ui("bob"), "bucket.usage_report", json!({"kind": "zone"})).await.unwrap_err().0, "permission_denied");

    let del = srv.data(Method::DELETE, &format!("{LIB}/2026/a.txt"), Some(&alice_photos)).send().await.unwrap();
    assert_eq!(del.status(), StatusCode::OK);
    assert_eq!(srv.data(Method::GET, &format!("{LIB}/2026/a.txt"), Some(&alice_photos)).send().await.unwrap().status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn import_finalize_auto_binds_the_upload() {
    let srv = start().await;
    let alice_photos = via("alice", "photos");
    let data = b"uploaded straight into a bucket";
    let (id, body) = file_of("photo.jpg", data);
    let status = srv.write_chunk(&alice_photos, data).await;
    assert!(status.is_success(), "{status}");
    let manifest = json!({
        "objects": [{"obj_id": id.to_string(), "obj_data": body}],
        "roots": [id.to_string()],
        "auto_bind": {"bucket_id": LIB, "key": "camera/photo.jpg", "content_type": "image/jpeg", "if_none_match": "*"}
    });
    let resp = srv.http.post(format!("{}/ndm/v1/import/finalize", srv.base)).bearer_auth(&alice_photos).json(&manifest).send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(v["binding"]["obj_id"], json!(id.to_string()));
    assert_eq!(v["binding"]["created"], json!(true));
    let get = srv.data(Method::GET, &format!("{LIB}/camera/photo.jpg"), Some(&alice_photos)).send().await.unwrap();
    assert_eq!(get.headers()["content-type"], "image/jpeg");
    assert_eq!(get.bytes().await.unwrap().as_ref(), data);
    // The condition holds on a retry.
    let again = srv.http.post(format!("{}/ndm/v1/import/finalize", srv.base)).bearer_auth(&alice_photos).json(&manifest).send().await.unwrap();
    assert_eq!(again.status(), StatusCode::PRECONDITION_FAILED);

    // The binding is a retention promise in the named store.
    let store = srv.state.share.store(&srv.state.config).await.unwrap();
    let pins = store.list_pins_by_owner(&format!("bucket:{LIB}"), None, 10).await.unwrap();
    assert_eq!(pins.len(), 1);
    assert_eq!(pins[0].obj_id, id);
    // Reconcile keeps it; a full maintenance round (cascade delivery) works on the store.
    let buckets = srv.state.buckets.store(&srv.state).await.unwrap();
    assert_eq!(buckets.reconcile(true).await.unwrap(), bucket_store::ReconcileReport::default());
    store.maintenance_round(false).await.unwrap();
}

/// A request signed like an S3 client (SigV4 header form).
fn s3_request(srv: &Server, key: &Value, method: reqwest::Method, path: &str, body: Vec<u8>) -> reqwest::RequestBuilder {
    let authority = srv.base.trim_start_matches("http://").to_string();
    let date = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut headers = http::HeaderMap::new();
    headers.insert("host", authority.parse().unwrap());
    headers.insert("x-amz-date", date.parse().unwrap());
    headers.insert("x-amz-content-sha256", "UNSIGNED-PAYLOAD".parse().unwrap());
    let signed: Vec<String> = ["host", "x-amz-content-sha256", "x-amz-date"].iter().map(|s| s.to_string()).collect();
    let (id, secret) = (key["access_key_id"].as_str().unwrap(), key["secret"].as_str().unwrap());
    let sig = s3_gateway::sigv4::sign(secret, id, "us-east-1", &date, method.as_str(), path, &[], &headers, &signed, "UNSIGNED-PAYLOAD");
    srv.http
        .request(method, format!("{}{}", srv.base, path))
        .header("x-amz-date", date.clone())
        .header("x-amz-content-sha256", "UNSIGNED-PAYLOAD")
        .header(
            "authorization",
            format!("AWS4-HMAC-SHA256 Credential={id}/{}/us-east-1/s3/aws4_request, SignedHeaders={}, Signature={sig}", &date[..8], signed.join(";")),
        )
        .body(body)
}

#[tokio::test]
async fn s3_face_with_keys_issued_by_the_user() {
    let srv = start().await;
    // Keys are issued by the user from their own UI, never by an app session.
    assert_eq!(srv.call(&via("alice", "photos"), "bucket.create_s3_key", json!({"appid": "photos"})).await.unwrap_err().0, "permission_denied");
    let key = srv.call(&ui("alice"), "bucket.create_s3_key", json!({"appid": "photos"})).await.unwrap();
    assert!(key["access_key_id"].as_str().unwrap().starts_with("BK"));

    let list = s3_request(&srv, &key, reqwest::Method::GET, "/s3/", Vec::new()).send().await.unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let body = list.text().await.unwrap();
    assert!(body.contains("<Name>photos-library</Name>") && body.contains("<Name>photos-index</Name>"), "{body}");

    let put = s3_request(&srv, &key, reqwest::Method::PUT, "/s3/photos-library/s3/hello.txt", b"via s3".to_vec()).send().await.unwrap();
    assert_eq!(put.status(), StatusCode::OK, "{}", put.text().await.unwrap());
    let get = s3_request(&srv, &key, reqwest::Method::GET, "/s3/photos-library/s3/hello.txt", Vec::new()).send().await.unwrap();
    assert_eq!(get.text().await.unwrap(), "via s3");
    // The same binding through the native interfaces.
    let b = srv.call(&via("alice", "photos"), "bucket.get", json!({"bucket_id": LIB, "key": "s3/hello.txt"})).await.unwrap();
    assert!(b["etag_md5"].is_string());
    assert_eq!(srv.data(Method::GET, &format!("{LIB}/s3/hello.txt"), Some(&ui("alice"))).send().await.unwrap().text().await.unwrap(), "via s3");

    // RBAC of the key's (user, app): bob's key for the same app sees only bob's buckets.
    let bobs = srv.call(&ui("bob"), "bucket.create_s3_key", json!({"appid": "photos"})).await.unwrap();
    let denied = s3_request(&srv, &bobs, reqwest::Method::GET, "/s3/photos-library/s3/hello.txt", Vec::new()).send().await.unwrap();
    assert_eq!(denied.status(), StatusCode::NOT_FOUND, "photos-library of bob's key is bob's own, empty bucket");

    let keys = srv.call(&ui("alice"), "bucket.list_s3_keys", json!({})).await.unwrap();
    assert_eq!(keys.as_array().unwrap().len(), 1);
    assert!(keys[0].get("secret").is_none(), "secrets are shown once");
    srv.call(&ui("alice"), "bucket.revoke_s3_key", json!({"access_key_id": key["access_key_id"]})).await.unwrap();
    let revoked = s3_request(&srv, &key, reqwest::Method::GET, "/s3/", Vec::new()).send().await.unwrap();
    assert_eq!(revoked.status(), StatusCode::FORBIDDEN);
}
