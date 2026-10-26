//! Files leaving a session (report artifacts, attached local files) become
//! named objects of the zone's NamedStore, so a reply carries object ids
//! and the message tunnels deliver them like any other attachment. Each one
//! is also bound into the Agent app's `artifacts` bucket, which retains it.

use std::path::Path;

use async_trait::async_trait;
use buckyos_api::get_buckyos_api_runtime;
use libopendan::runner::ObjectStore;
use ndn_lib::{load_named_obj, FileObject, NamedObject, ObjId, StoreMode};
use ndn_toolkit::{cacl_file_object, CheckMode};

/// LocalLink needs a qcid, which files smaller than this do not have: they
/// are copied into the store instead.
const LOCAL_LINK_MIN_FILE_SIZE: u64 = 3 * 4096;

/// Declared in the Jarvis template's `service_config_tips.buckets`.
pub const ARTIFACTS_BUCKET: &str = "artifacts";

pub fn artifact_key(obj_id: &ObjId, name: &str) -> String {
    let id = obj_id.to_string();
    let hash = id.split_once(':').map(|(_, h)| h).unwrap_or(&id);
    let name: String = name.chars().map(|c| if c == '/' || c == '\\' { '_' } else { c }).collect();
    format!("{}/{}", &hash[..hash.len().min(16)], if name.is_empty() { "artifact" } else { &name })
}

/// Binds an artifact into this Agent's `artifacts` bucket. Agents whose app
/// does not declare it (built from an older template) keep the old behavior.
async fn retain(obj_id: &ObjId, name: &str) {
    let Ok(runtime) = get_buckyos_api_runtime() else { return };
    let Some(owner) = runtime.get_owner_user_id() else { return };
    let Ok(bucket) = buckyos_api::bucket_client::BucketId::user(&runtime.get_app_id(), &owner, ARTIFACTS_BUCKET) else { return };
    let client = match runtime.get_bucket_client().await {
        Ok(c) => c,
        Err(e) => {
            log::warn!("artifact {} not retained: {e}", obj_id.to_string());
            return;
        }
    };
    let req = buckyos_api::bucket_client::PutRequest {
        key: artifact_key(obj_id, name),
        obj_id: obj_id.to_string(),
        conditions: buckyos_api::bucket_client::Conditions::create_only(),
        ..Default::default()
    };
    match client.put(&bucket, &req).await {
        Ok(r) => log::info!("artifact {} retained in {bucket}/{} (v{})", obj_id.to_string(), req.key, r.version),
        Err(e) => {
            let message = e.to_string();
            if !message.contains("precondition_failed") {
                log::warn!("artifact {} not retained in {bucket}: {message}", obj_id.to_string());
            }
        }
    }
}

pub struct NamedStoreObjects;

#[async_trait]
impl ObjectStore for NamedStoreObjects {
    async fn put_file(&self, path: &Path, name: &str) -> Result<ObjId, String> {
        let meta = tokio::fs::metadata(path)
            .await
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if !meta.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        let store = get_buckyos_api_runtime()
            .map_err(|e| format!("buckyos runtime: {e}"))?
            .get_named_store()
            .await
            .map_err(|e| format!("named store: {e}"))?;
        let mode = if meta.len() < LOCAL_LINK_MIN_FILE_SIZE {
            StoreMode::StoreInNamedMgr
        } else {
            StoreMode::LocalFile(path.to_path_buf(), 0..meta.len(), false)
        };
        let (mut file, obj_id, _) = cacl_file_object(
            Some(&store),
            path,
            &FileObject::default(),
            true,
            &CheckMode::ByFullHash,
            mode,
            None,
        )
        .await
        .map_err(|e| format!("store {}: {e}", path.display()))?;
        if name.is_empty() || file.content_obj.name == name {
            retain(&obj_id, &file.content_obj.name).await;
            return Ok(obj_id);
        }
        // The object is named as it is delivered, not as the file on disk
        // (a report's stable copy is named by its digest).
        file.content_obj.name = name.to_string();
        let (obj_id, obj_str) = file.gen_obj_id();
        store
            .put_object(&obj_id, &obj_str)
            .await
            .map_err(|e| format!("store {}: {e}", path.display()))?;
        retain(&obj_id, name).await;
        Ok(obj_id)
    }

    async fn name_of(&self, obj_id: &ObjId) -> Option<String> {
        if !obj_id.is_file_object() {
            return None;
        }
        let store = get_buckyos_api_runtime().ok()?.get_named_store().await.ok()?;
        let file: FileObject = load_named_obj(&store.get_object(obj_id).await.ok()?).ok()?;
        Some(file.content_obj.name).filter(|n| !n.trim().is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_keys_are_short_and_flat() {
        let id = ObjId::new("cyfile:0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(artifact_key(&id, "report.md"), "0123456789abcdef/report.md");
        assert_eq!(artifact_key(&id, "a/b\\c.txt"), "0123456789abcdef/a_b_c.txt");
        assert_eq!(artifact_key(&id, ""), "0123456789abcdef/artifact");
        assert!(buckyos_api::bucket_client::validate_key(&artifact_key(&id, "x")).is_ok());
    }
}
