//! Files leaving a session (report artifacts, attached local files) become
//! named objects of the zone's NamedStore, so a reply carries object ids
//! and the message tunnels deliver them like any other attachment.

use std::path::Path;

use async_trait::async_trait;
use buckyos_api::get_buckyos_api_runtime;
use libopendan::runner::ObjectStore;
use ndn_lib::{load_named_obj, FileObject, NamedObject, ObjId, StoreMode};
use ndn_toolkit::{cacl_file_object, CheckMode};

/// LocalLink needs a qcid, which files smaller than this do not have: they
/// are copied into the store instead.
const LOCAL_LINK_MIN_FILE_SIZE: u64 = 3 * 4096;

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
