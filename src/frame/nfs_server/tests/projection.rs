//! Files projection of documents / collected buckets (design §4.5.4, D11):
//! `home/<user>/Apps/<App>/<bucket>/<key...>`, read-only, delete = unbind.

use buckyos_api::*;
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};
use nfs_server::config::{AuthMode, ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::AppState;
use reqwest::StatusCode;
use serde_json::{json, Value};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();

async fn init_rbac() {
    RBAC.get_or_init(|| async {
        let groups = "g, owner, admin\ng, alice, users\ng, bob, users\ng, system:control-panel, system\n";
        let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, groups);
        rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim()).await.unwrap();
    })
    .await;
}

fn ui(name: &str) -> String {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(name.into()),
        appid: None,
        sudo: false,
        extra: Default::default(),
    };
    bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
    bind_token_target(&mut token, &AuthTarget::from_canonical_key("system:control-panel").unwrap(), TokenUse::Session).unwrap();
    token.generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap()).unwrap()
}

struct Nfsp {
    base: String,
    http: reqwest::Client,
    token: String,
    session: String,
    seq: std::sync::atomic::AtomicU64,
}

impl Nfsp {
    async fn hello(base: &str, token: String) -> Nfsp {
        let http = reqwest::Client::new();
        let v: Value = http.post(format!("{base}/nfs/v1/hello")).bearer_auth(&token).json(&json!({"args": {}})).send().await.unwrap().json().await.unwrap();
        assert_eq!(v["ok"], true, "hello: {v}");
        let session = v["result"]["session"].as_str().unwrap().to_string();
        Nfsp { base: base.to_string(), http, token, session, seq: 1.into() }
    }

    async fn call(&self, method: &str, at: &str, args: Value) -> Value {
        let mut env = json!({"session": self.session, "at": {"realm": "dfs", "path": at}, "args": args, "want": ["base", "ident", "access"]});
        if method == "delete" || method == "mkdir" {
            env["seq"] = json!(self.seq.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        }
        self.http.post(format!("{}/nfs/v1/{method}", self.base)).bearer_auth(&self.token).json(&env).send().await.unwrap().json().await.unwrap()
    }

    async fn names(&self, at: &str) -> Vec<(String, String)> {
        let v = self.call("list", at, json!({})).await;
        assert_eq!(v["ok"], true, "list {at}: {v}");
        v["result"]["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| (e["name"].as_str().unwrap().to_string(), e["target"]["kind"].as_str().unwrap().to_string()))
            .collect()
    }
}

fn pair(name: &str, kind: &str) -> (String, String) {
    (name.to_string(), kind.to_string())
}

#[tokio::test]
async fn documents_buckets_are_projected_into_the_users_home() {
    init_rbac().await;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("exports");
    std::fs::create_dir_all(data.join("home/alice/notes")).unwrap();
    let mut config = ServerConfig::new(tmp.path().join("fsdb"), vec![ExportRoot { id: "home".into(), path: data.join("home") }]);
    config.auth = AuthMode::TestKey { public_key: PUBLIC.into(), zone_owner: "owner".into() };
    config.named_store = NamedStoreSource::Dir(tmp.path().join("store"));
    config.bucket_specs = serde_json::from_value(json!({
        "photos": {
            "show_name": "Photos",
            "storage_quota_mib": 8,
            "buckets": {
                "library": {"kind": "data", "data_policy": "documents"},
                "cache": {"kind": "data", "data_policy": "app_data"}
            }
        },
        "editor": {"buckets": {"notes": {"kind": "object", "data_policy": "documents"}}},
        "clips": {"buckets": {"saved": {"kind": "data", "data_policy": "collected"}}}
    }))
    .unwrap();
    let state = AppState::new(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let app = nfs_server::server::build_router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let http = reqwest::Client::new();
    let alice = ui("alice");
    for (key, body) in [("2026/a.jpg", "jpeg"), ("2026", "a key next to a prefix"), ("top.txt", "top")] {
        let r = http.put(format!("{base}/bucket/v1/user/photos/alice/library/{key}")).bearer_auth(&alice).body(body).send().await.unwrap();
        assert!(r.status().is_success(), "put {key}: {}", r.status());
    }
    let r = http.put(format!("{base}/bucket/v1/user/photos/alice/cache/c.bin")).bearer_auth(&alice).body("c").send().await.unwrap();
    assert!(r.status().is_success());
    let origin = json!({"author": "did:web:bob.example.com", "source": "cyfs://bob.example.com/post/1", "collected_at": 1});
    let r = http
        .put(format!("{base}/bucket/v1/user/clips/alice/saved/post.md"))
        .bearer_auth(&alice)
        .header("x-bucky-origin", origin.to_string())
        .body("# saved")
        .send()
        .await
        .unwrap();
    assert!(r.status().is_success(), "collected put: {}", r.status());

    let rpc: Value = http
        .post(format!("{base}/kapi/nfs-server/"))
        .json(&json!({"method": "bucket.list", "params": {}, "sys": [1, alice]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(rpc["result"].is_array(), "browser runtimes call /kapi/nfs-server/: {rpc}");

    let fs = Nfsp::hello(&base, alice.clone()).await;
    assert_eq!(
        fs.names("/home/alice").await,
        vec![pair("Apps", "dir"), pair("Collected", "dir"), pair("notes", "dir")],
        "app_data and object buckets are not projected"
    );
    assert_eq!(fs.names("/home/alice/Collected/clips/saved").await, vec![pair("post.md", "file")]);
    assert_eq!(fs.names("/home/alice/Apps").await, vec![pair("Photos", "dir")]);
    assert_eq!(fs.names("/home/alice/Apps/Photos").await, vec![pair("library", "dir")]);
    assert_eq!(
        fs.names("/home/alice/Apps/Photos/library").await,
        vec![pair("2026", "dir"), pair("2026~file", "file"), pair("top.txt", "file")]
    );
    assert_eq!(fs.names("/home/alice/Apps/Photos/library/2026").await, vec![pair("a.jpg", "file")]);

    let info = fs.call("stat", "/home/alice/Apps/Photos/library/top.txt", json!({})).await;
    assert_eq!(info["ok"], true, "{info}");
    assert_eq!(info["result"]["size"], 3);
    assert!(info["result"]["flags"].as_array().unwrap().contains(&json!("read_only")));
    let read = info["result"]["access_urls"].as_array().unwrap().iter().find(|u| u["kind"] == "read").unwrap()["url"].as_str().unwrap().to_string();
    assert_eq!(read, "/bucket/v1/user/photos/alice/library/top.txt");
    let body = http.get(format!("{base}{read}")).bearer_auth(&alice).send().await.unwrap().text().await.unwrap();
    assert_eq!(body, "top");

    let by_ref: Value = http
        .post(format!("{base}/nfs/v1/stat"))
        .bearer_auth(&alice)
        .json(&json!({"session": fs.session, "at": {"ref": info["result"]["ref"]}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(by_ref["result"]["name"], "top.txt", "{by_ref}");

    let mkdir = fs.call("mkdir", "/home/alice/Apps/Photos/library", json!({"name": "new"})).await;
    assert_eq!(mkdir["ok"], false, "the projection is read-only: {mkdir}");

    let bob = Nfsp::hello(&base, ui("bob")).await;
    assert_eq!(bob.call("list", "/home/alice/Apps/Photos/library", json!({})).await["ok"], false);
    assert_eq!(bob.call("delete", "/home/alice/Apps/Photos/library", json!({"name": "top.txt"})).await["ok"], false);

    let del = fs.call("delete", "/home/alice/Apps/Photos/library", json!({"name": "top.txt"})).await;
    assert_eq!(del["ok"], true, "{del}");
    let gone = http.get(format!("{base}{read}")).bearer_auth(&alice).send().await.unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND, "deleting from Files unbinds the key");

    let not_empty = fs.call("delete", "/home/alice/Apps/Photos/library", json!({"name": "2026"})).await;
    assert_eq!(not_empty["error"]["code"], "NOT_EMPTY", "{not_empty}");
    let del = fs.call("delete", "/home/alice/Apps/Photos/library", json!({"name": "2026", "recursive": true})).await;
    assert_eq!(del["ok"], true, "{del}");
    assert_eq!(fs.names("/home/alice/Apps/Photos/library").await, vec![pair("2026", "file")]);

    let bucket = fs.call("delete", "/home/alice/Apps/Photos", json!({"name": "library"})).await;
    assert_eq!(bucket["ok"], false, "buckets go with their app: {bucket}");
}
