//! Materializes agent-emitted `<attachment>BODY</attachment>` local paths
//! into NamedStore-backed `ObjId`s so the standard `RefItem::DataObj`
//! egress lane (TG tunnel, MessageHub, …) can upload them like any
//! externally-sourced attachment.
//!
//! Strategy: copy every attachment into NamedStore. The egress consumer may
//! run in a different container or filesystem namespace, so a LocalLink to
//! the agent's source path is not portable across the delivery boundary.
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

pub struct NamedStoreAttachmentResolver {
    workspace_root: Option<PathBuf>,
    agent_id: String,
}

impl NamedStoreAttachmentResolver {
    pub fn new(workspace_root: Option<PathBuf>, agent_id: impl Into<String>) -> Self {
        Self {
            workspace_root,
            agent_id: agent_id.into(),
        }
    }
}

#[async_trait]
impl LocalFileResolver for NamedStoreAttachmentResolver {
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
        let runtime =
            get_buckyos_api_runtime().map_err(|e| format!("buckyos runtime unavailable: {e}"))?;
        let store_mgr = runtime
            .get_named_store()
            .await
            .map_err(|e| format!("named_store unavailable: {e}"))?;

        // The object must own its payload because downstream message
        // delivery may run outside this process's filesystem namespace.
        let template = FileObject::default();
        let (_file_obj, file_obj_id, _file_obj_str) = cacl_file_object(
            Some(&store_mgr),
            &absolute,
            &template,
            true,
            &CheckMode::ByFullHash,
            attachment_store_mode(),
            None,
        )
        .await
        .map_err(|e| {
            format!(
                "register `{}` into NamedStore (StoreInNamedMgr) failed: {e}",
                absolute.display(),
            )
        })?;

        Ok(file_obj_id)
    }
}

fn attachment_store_mode() -> StoreMode {
    StoreMode::StoreInNamedMgr
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_payload_is_stored_in_named_store() {
        assert!(matches!(
            attachment_store_mode(),
            StoreMode::StoreInNamedMgr
        ));
    }
}
