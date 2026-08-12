//! Materializes agent-emitted `<attachment>BODY</attachment>` local paths
//! into NamedStore-backed `ObjId`s so the standard `RefItem::DataObj`
//! egress lane (TG tunnel, MessageHub, …) can upload them like any
//! externally-sourced attachment.
//!
//! Strategy: register large files in **LocalLink** mode — no bytes are
//! copied, the store just records a content-addressed pointer to the
//! original path. NamedStore's LocalLink path requires a qcid, and qcid is
//! unavailable for tiny files, so small files are stored directly in
//! NamedStore.
//!
//! Path-policy enforcement (workspace fence, `..` traversal,
//! symlink-escape) lives in [`crate::attachment_policy`]; by the time
//! this resolver sees a path the validator has already passed it. We
//! only need to canonicalize relatives against the workspace and stat
//! the target before handing off to `cacl_file_object`.

use std::path::PathBuf;

use async_trait::async_trait;
use buckyos_api::get_buckyos_api_runtime;
use llm_context::LocalFileResolver;
use ndn_lib::{FileObject, ObjId, StoreMode};
use ndn_toolkit::{cacl_file_object, CheckMode};

const LOCAL_LINK_MIN_FILE_SIZE: u64 = 3 * 4096;

pub struct NamedStoreLocalLinkResolver {
    workspace_root: Option<PathBuf>,
    agent_id: String,
}

impl NamedStoreLocalLinkResolver {
    pub fn new(workspace_root: Option<PathBuf>, agent_id: impl Into<String>) -> Self {
        Self {
            workspace_root,
            agent_id: agent_id.into(),
        }
    }
}

#[async_trait]
impl LocalFileResolver for NamedStoreLocalLinkResolver {
    async fn resolve(&self, raw_path: &str) -> Result<ObjId, String> {
        let candidate = std::path::Path::new(raw_path);
        let absolute = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else if let Some(root) = self.workspace_root.as_ref() {
            root.join(candidate)
        } else {
            return Err(format!(
                "relative attachment path `{raw_path}` has no workspace anchor (agent `{}`)",
                self.agent_id
            ));
        };

        let metadata = tokio::fs::metadata(&absolute)
            .await
            .map_err(|e| format!("stat `{}`: {e}", absolute.display()))?;
        if !metadata.is_file() {
            return Err(format!(
                "attachment target `{}` is not a regular file",
                absolute.display()
            ));
        }
        let file_size = metadata.len();

        let runtime =
            get_buckyos_api_runtime().map_err(|e| format!("buckyos runtime unavailable: {e}"))?;
        let store_mgr = runtime
            .get_named_store()
            .await
            .map_err(|e| format!("named_store unavailable: {e}"))?;

        // `cacl_file_object` re-derives per-chunk ChunkLocalInfo from the
        // actual file. LocalLink requires qcid, which is only defined for
        // files at least `LOCAL_LINK_MIN_FILE_SIZE` bytes; tiny files are
        // copied into NamedStore instead of being rejected.
        let template = FileObject::default();
        let store_mode = store_mode_for_attachment_file(&absolute, file_size);
        let store_mode_label = store_mode_label(&store_mode);
        let (_file_obj, file_obj_id, _file_obj_str) = cacl_file_object(
            Some(&store_mgr),
            &absolute,
            &template,
            true,
            &CheckMode::ByFullHash,
            store_mode,
            None,
        )
        .await
        .map_err(|e| {
            format!(
                "register `{}` into NamedStore ({store_mode_label}) failed: {e}",
                absolute.display(),
            )
        })?;

        Ok(file_obj_id)
    }
}

fn store_mode_for_attachment_file(path: &std::path::Path, file_size: u64) -> StoreMode {
    if file_size < LOCAL_LINK_MIN_FILE_SIZE {
        StoreMode::StoreInNamedMgr
    } else {
        StoreMode::LocalFile(path.to_path_buf(), 0..file_size, false)
    }
}

fn store_mode_label(mode: &StoreMode) -> &'static str {
    match mode {
        StoreMode::LocalFile(_, _, _) => "LocalLink",
        StoreMode::StoreInNamedMgr => "StoreInNamedMgr",
        StoreMode::NoStore => "NoStore",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_attachment_files_store_inline() {
        let path = std::path::Path::new("/tmp/small.txt");
        assert!(matches!(
            store_mode_for_attachment_file(path, LOCAL_LINK_MIN_FILE_SIZE - 1),
            StoreMode::StoreInNamedMgr
        ));
    }

    #[test]
    fn large_attachment_files_use_local_link() {
        let path = std::path::Path::new("/tmp/large.bin");
        assert!(matches!(
            store_mode_for_attachment_file(path, LOCAL_LINK_MIN_FILE_SIZE),
            StoreMode::LocalFile(_, _, false)
        ));
    }
}
