#![allow(dead_code)]
//! An nfs-server inside the test process, as a Zone has it for this service: the named store, the
//! `aiworkspace` buckets (`--bucket-specs`), the NDM gateway and the `/bucket/v1` data plane. No
//! authentication (standalone): every caller has full access.

use aiworkspace_server::buckets::ZoneBuckets;
use aiworkspace_store::objects::ObjectStore;
use nfs_server::config::{ExportRoot, NamedStoreSource, ServerConfig};
use nfs_server::state::{AppState, SharedState};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct Nfs {
    pub base: String,
    pub store_config: PathBuf,
    pub state: SharedState,
    pub http: reqwest::Client,
}

impl Nfs {
    pub async fn start(dir: &Path) -> Nfs {
        let root = dir.join("nfs");
        std::fs::create_dir_all(root.join("home")).unwrap();
        let mut config = ServerConfig::new(root.join("fsdb"), vec![ExportRoot { id: "home".into(), path: root.join("home") }]);
        config.named_store = NamedStoreSource::Dir(root.join("named-store"));
        config.bucket_specs = serde_json::from_value(json!({
            "aiworkspace": { "buckets": buckyos_api::aiworkspace_bucket_decls(), "storage_quota_mib": buckyos_api::AIWORKSPACE_STORAGE_QUOTA_MIB }
        }))
        .unwrap();
        let store_config = root.join("named-store.json");
        std::fs::write(&store_config, json!({ "stores": [{ "path": root.join("named-store") }] }).to_string()).unwrap();
        let state = AppState::new(config).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = nfs_server::server::build_router(state.clone());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Nfs { base, store_config, state, http: reqwest::Client::new() }
    }

    /// Arguments of a spawned `aiworkspace` that uses this server.
    pub fn args(&self) -> Vec<String> {
        vec!["--nfs-server".into(), self.base.clone(), "--named-store-config".into(), self.store_config.display().to_string()]
    }

    /// The object store of an in-process service: this server's named store and buckets.
    pub async fn objects(&self) -> Arc<ObjectStore> {
        let ndm = self.state.share.store(&self.state.config).await.unwrap().clone();
        let client = buckyos_api::BucketClient::new(kRPC::kRPC::new(&format!("{}/kapi/nfs-server", self.base), None));
        ObjectStore::new(ndm, Arc::new(ZoneBuckets::Fixed(client)))
    }

    /// `PUT /bucket/v1/user/aiworkspace/<user>/<name>/<key>`: what a browser upload with `bind` leaves behind.
    pub async fn put(&self, user: &str, name: &str, key: &str, bytes: Vec<u8>) -> Value {
        let bucket = format!("user/aiworkspace/{user}/{name}");
        let r = self.http.put(format!("{}/bucket/v1/{bucket}/{key}", self.base)).body(bytes).send().await.unwrap();
        assert!(r.status().is_success(), "{}", r.text().await.unwrap());
        let put: Value = r.json().await.unwrap();
        json!({ "bucket_id": bucket, "key": key, "obj_id": put["obj_id"] })
    }

    /// The bytes a download (`doc.export` / `replica.bootstrap` → `download`) names, read with its token.
    pub async fn fetch(&self, download: &Value) -> Vec<u8> {
        let url = format!("{}/bucket/v1/{}/{}?token={}", self.base, download["bucket_id"].as_str().unwrap(), download["key"].as_str().unwrap(), download["token"].as_str().unwrap());
        let r = self.http.get(url).send().await.unwrap();
        assert!(r.status().is_success(), "{}", r.status());
        r.bytes().await.unwrap().to_vec()
    }

    /// The object bound under `key` in a bucket of `user`, if any.
    pub async fn bound(&self, user: &str, name: &str, key: &str) -> Option<String> {
        let r: Value = self
            .http
            .post(format!("{}/kapi/nfs-server", self.base))
            .json(&json!({ "method": "bucket.get", "params": { "bucket_id": format!("user/aiworkspace/{user}/{name}"), "key": key }, "sys": [1] }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        r["result"]["obj_id"].as_str().map(str::to_string)
    }
}
