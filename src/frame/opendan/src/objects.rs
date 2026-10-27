//! Files leaving a session (report artifacts, attached local files) become
//! named objects of the zone's NamedStore bound into the Agent app's
//! `artifacts` bucket (`documents`: the owner's, kept and backed up), so a
//! reply carries object ids and their `obj://bucket/...` bindings and the
//! message tunnels deliver them like any other attachment. An object bound
//! elsewhere (an AICC result) is copied into `artifacts`, not stored again.
//! `Ok` means the bucket retains it.

use std::path::Path;

use async_trait::async_trait;
use buckyos_api::bucket_client::{BucketId, Conditions, CopyRequest, PutRequest};
use buckyos_api::{
    bucket_obj_url, get_buckyos_api_runtime, is_precondition_failed, parse_bucket_obj_url,
    BucketClient,
};
use libopendan::runner::{ObjectStore, StoredObject};
use ndn_lib::{load_named_obj, FileObject, NamedObject, ObjId, StoreMode};
use ndn_toolkit::{cacl_file_object, CheckMode, NamedObjectStore};

/// Declared in the Jarvis template's `service_config_tips.buckets`.
pub const ARTIFACTS_BUCKET: &str = "artifacts";

pub fn artifact_key(obj_id: &ObjId, name: &str) -> String {
    let id = obj_id.to_string();
    let hash = id.split_once(':').map(|(_, h)| h).unwrap_or(&id);
    let name: String = name
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    format!(
        "{}/{}",
        &hash[..hash.len().min(16)],
        if name.is_empty() { "artifact" } else { &name }
    )
}

/// Stores the file at `path` as a FileObject named `name` and binds it in `bucket`.
pub async fn put_artifact(
    store: &dyn NamedObjectStore,
    client: &BucketClient,
    bucket: &BucketId,
    path: &Path,
    name: &str,
) -> Result<StoredObject, String> {
    let meta = tokio::fs::metadata(path)
        .await
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("{} is not a file", path.display()));
    }
    let (mut file, mut obj_id, _) = cacl_file_object(
        Some(store),
        path,
        &FileObject::default(),
        true,
        &CheckMode::ByFullHash,
        StoreMode::StoreInNamedMgr,
        None,
    )
    .await
    .map_err(|e| format!("store {}: {e}", path.display()))?;
    // The object is named as it is delivered, not as the file on disk
    // (a report's stable copy is named by its digest).
    if !name.is_empty() && file.content_obj.name != name {
        file.content_obj.name = name.to_string();
        let (id, obj_str) = file.gen_obj_id();
        store
            .put_object(&id, &obj_str)
            .await
            .map_err(|e| format!("store {}: {e}", path.display()))?;
        obj_id = id;
    }
    let key = artifact_key(&obj_id, &file.content_obj.name);
    let req = PutRequest {
        key: key.clone(),
        obj_id: obj_id.to_string(),
        conditions: Conditions::create_only(),
        ..Default::default()
    };
    match client.put(bucket, &req).await {
        Ok(_) => {}
        Err(e) if is_precondition_failed(&e) => same_object(client, bucket, &key, &obj_id).await?,
        Err(e) => return Err(format!("{bucket}/{key}: {e}")),
    }
    Ok(StoredObject {
        obj_id,
        url: Some(bucket_obj_url(bucket, &key)),
    })
}

/// Retains what `url` (`obj://bucket/...`) binds in `bucket`: as it is when
/// it is already there, copied (same object, a second binding) otherwise.
pub async fn retain_bucket_object(
    client: &BucketClient,
    bucket: &BucketId,
    url: &str,
) -> Result<StoredObject, String> {
    let (src, key) = parse_bucket_obj_url(url)
        .ok_or_else(|| format!("`{url}` is not an obj://bucket/ reference"))?;
    let binding = client
        .get(&src, &key, None)
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    let obj_id = binding
        .obj_id
        .as_deref()
        .and_then(|id| ObjId::new(id).ok())
        .ok_or_else(|| format!("{url} binds no object"))?;
    if &src == bucket {
        return Ok(StoredObject {
            obj_id,
            url: Some(url.to_string()),
        });
    }
    let dst_key = artifact_key(&obj_id, key.rsplit('/').next().unwrap_or(&key));
    let req = CopyRequest {
        src_bucket_id: src.to_string(),
        src_key: key.clone(),
        src_version: Some(binding.version),
        dst_bucket_id: bucket.to_string(),
        dst_key: dst_key.clone(),
        conditions: Conditions::create_only(),
        ..Default::default()
    };
    match client.copy(&req).await {
        Ok(_) => {}
        Err(e) if is_precondition_failed(&e) => {
            same_object(client, bucket, &dst_key, &obj_id).await?
        }
        Err(e) => return Err(format!("copy {url} to {bucket}: {e}")),
    }
    Ok(StoredObject {
        obj_id,
        url: Some(bucket_obj_url(bucket, &dst_key)),
    })
}

/// A key named after its content already exists: the same artifact delivered again.
async fn same_object(
    client: &BucketClient,
    bucket: &BucketId,
    key: &str,
    obj_id: &ObjId,
) -> Result<(), String> {
    let existing = client
        .get(bucket, key, None)
        .await
        .map_err(|e| format!("{bucket}/{key}: {e}"))?;
    if existing.obj_id.as_deref() == Some(obj_id.to_string().as_str()) {
        Ok(())
    } else {
        Err(format!("{bucket}/{key} already binds another object"))
    }
}

/// This Agent's `artifacts` bucket and a bucket client of the runtime.
async fn artifacts_bucket() -> Result<(BucketClient, BucketId), String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| format!("buckyos runtime: {e}"))?;
    let owner = runtime
        .get_owner_user_id()
        .ok_or("the runtime has no owner user")?;
    let bucket = BucketId::user(&runtime.get_app_id(), &owner, ARTIFACTS_BUCKET)
        .map_err(|e| e.to_string())?;
    let client = runtime
        .get_bucket_client()
        .await
        .map_err(|e| format!("bucket client: {e}"))?;
    Ok((client, bucket))
}

pub struct NamedStoreObjects;

#[async_trait]
impl ObjectStore for NamedStoreObjects {
    async fn put_file(&self, path: &Path, name: &str) -> Result<StoredObject, String> {
        let store = get_buckyos_api_runtime()
            .map_err(|e| format!("buckyos runtime: {e}"))?
            .get_named_store()
            .await
            .map_err(|e| format!("named store: {e}"))?;
        let (client, bucket) = artifacts_bucket().await?;
        put_artifact(&store, &client, &bucket, path, name).await
    }

    async fn retain_url(&self, url: &str) -> Result<StoredObject, String> {
        let (client, bucket) = artifacts_bucket().await?;
        retain_bucket_object(&client, &bucket, url).await
    }

    async fn name_of(&self, obj_id: &ObjId) -> Option<String> {
        if !obj_id.is_file_object() {
            return None;
        }
        let store = get_buckyos_api_runtime()
            .ok()?
            .get_named_store()
            .await
            .ok()?;
        let file: FileObject = load_named_obj(&store.get_object(obj_id).await.ok()?).ok()?;
        Some(file.content_obj.name).filter(|n| !n.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn artifact_keys_are_short_and_flat() {
        let id = ObjId::new("cyfile:0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(artifact_key(&id, "report.md"), "0123456789abcdef/report.md");
        assert_eq!(
            artifact_key(&id, "a/b\\c.txt"),
            "0123456789abcdef/a_b_c.txt"
        );
        assert_eq!(artifact_key(&id, ""), "0123456789abcdef/artifact");
        assert!(buckyos_api::bucket_client::validate_key(&artifact_key(&id, "x")).is_ok());
    }

    #[test]
    fn the_jarvis_template_declares_valid_buckets() {
        use buckyos_api::bucket_client::{BucketDecl, DataPolicy};
        let doc: serde_json::Value = serde_json::from_str(include_str!(
            "../../../apps/jarvis_agent/dapp_meta/app.json"
        ))
        .unwrap();
        let tips = &doc["service_config_tips"];
        let buckets: std::collections::HashMap<String, BucketDecl> =
            serde_json::from_value(tips["buckets"].clone()).unwrap();
        assert_eq!(
            buckyos_api::validate_app_buckets(&buckets, tips["storage_quota_mib"].as_u64()),
            Vec::<String>::new()
        );
        assert_eq!(buckets[ARTIFACTS_BUCKET].data_policy, DataPolicy::Documents);
        assert_eq!(buckets["scratch"].data_policy, DataPolicy::Temp);
    }

    /// An nfs-server of its own (no auth) declaring the Agent's buckets and
    /// AICC's `outputs`, and its named store.
    async fn zone(dir: &Path) -> (BucketClient, named_store::NamedDataMgr) {
        let mut config = nfs_server::ServerConfig::new(dir.join("nfs"), vec![]);
        config.named_store = nfs_server::config::NamedStoreSource::Dir(dir.join("store"));
        config.bucket_specs = serde_json::from_value(json!({
            "agentx": {"buckets": {ARTIFACTS_BUCKET: {"kind": "data", "data_policy": "documents"}}},
            "aicc": {"buckets": buckyos_api::aicc_bucket_decls(), "storage_quota_mib": buckyos_api::AICC_STORAGE_QUOTA_MIB},
        }))
        .unwrap();
        let state = nfs_server::AppState::new(config).unwrap();
        let store = state.share.store(&state.config).await.unwrap().clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/kapi/nfs-server", listener.local_addr().unwrap());
        let app = nfs_server::build_router(state);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (BucketClient::new(kRPC::kRPC::new(&url, None)), store)
    }

    #[tokio::test]
    async fn artifacts_are_bound_under_their_delivered_name_and_bucket_objects_are_copied() {
        let tmp = tempfile::tempdir().unwrap();
        let (client, store) = zone(tmp.path()).await;
        let artifacts = BucketId::user("agentx", "alice", ARTIFACTS_BUCKET).unwrap();
        let file = tmp.path().join("0-deadbeef");
        std::fs::write(&file, b"x".repeat(64 * 1024)).unwrap();
        let mtime = std::fs::metadata(&file).unwrap().modified().unwrap();
        let stored = put_artifact(&store, &client, &artifacts, &file, "report.md")
            .await
            .unwrap();
        let key = artifact_key(&stored.obj_id, "report.md");
        assert_eq!(stored.url, Some(bucket_obj_url(&artifacts, &key)));
        let binding = client.get(&artifacts, &key, None).await.unwrap();
        assert_eq!(binding.obj_id, Some(stored.obj_id.to_string()));
        // Retained in the store, not linked to the file: the file may go.
        std::fs::remove_file(&file).unwrap();
        let file_obj: FileObject =
            load_named_obj(&store.get_object(&stored.obj_id).await.unwrap()).unwrap();
        assert_eq!(file_obj.content_obj.name, "report.md");
        let (mut reader, size) = store.open_reader(&stored.obj_id, None).await.unwrap();
        let mut back = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut reader, &mut back)
            .await
            .unwrap();
        assert_eq!((size, back.len()), (64 * 1024, 64 * 1024));

        // Delivered again (a retried report): the same binding.
        let again = tmp.path().join("again");
        std::fs::write(&again, b"x".repeat(64 * 1024)).unwrap();
        // A FileObject carries the file's mtime: a retry re-delivers the same stable copy.
        std::fs::File::options().write(true).open(&again).unwrap().set_modified(mtime).unwrap();
        assert_eq!(
            put_artifact(&store, &client, &artifacts, &again, "report.md")
                .await
                .unwrap(),
            stored
        );
        assert_eq!(
            retain_bucket_object(&client, &artifacts, stored.url.as_deref().unwrap())
                .await
                .unwrap(),
            stored
        );

        // An AICC result: one more binding of the same object.
        let outputs = BucketId::user("aicc", "alice", buckyos_api::AICC_BUCKET_OUTPUTS).unwrap();
        let (generated, _) = client
            .put_bytes(
                &store,
                &outputs,
                PutRequest {
                    key: "agentx/0123/image_1.png".into(),
                    ..Default::default()
                },
                b"png bytes",
            )
            .await
            .unwrap();
        let url = bucket_obj_url(&outputs, "agentx/0123/image_1.png");
        let kept = retain_bucket_object(&client, &artifacts, &url)
            .await
            .unwrap();
        assert_eq!(kept.obj_id, generated);
        let copied_key = artifact_key(&generated, "image_1.png");
        assert_eq!(kept.url, Some(bucket_obj_url(&artifacts, &copied_key)));
        assert_eq!(
            client
                .get(&artifacts, &copied_key, None)
                .await
                .unwrap()
                .obj_id,
            Some(generated.to_string())
        );
        assert_eq!(
            retain_bucket_object(&client, &artifacts, &url)
                .await
                .unwrap(),
            kept
        );
    }

    #[tokio::test]
    async fn an_unretained_artifact_is_an_error() {
        let tmp = tempfile::tempdir().unwrap();
        let (client, store) = zone(tmp.path()).await;
        let file = tmp.path().join("a.txt");
        std::fs::write(&file, b"hello").unwrap();
        let undeclared = BucketId::user("agenty", "alice", ARTIFACTS_BUCKET).unwrap();
        assert!(put_artifact(&store, &client, &undeclared, &file, "a.txt")
            .await
            .is_err());
        let artifacts = BucketId::user("agentx", "alice", ARTIFACTS_BUCKET).unwrap();
        let missing = bucket_obj_url(
            &BucketId::user("aicc", "alice", "outputs").unwrap(),
            "gone.png",
        );
        assert!(retain_bucket_object(&client, &artifacts, &missing)
            .await
            .is_err());
        assert!(retain_bucket_object(&client, &artifacts, "cyfile:abc")
            .await
            .is_err());
    }
}
