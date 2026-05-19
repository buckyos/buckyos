//! §2.2.2 of the Agent Message protocol — outbound attachment validation.
//!
//! LLM-emitted `<attachment>` markers are untrusted input. Before lowering
//! them into `MsgContent.refs` we filter through this validator:
//!
//! - **Object IDs** are content-addressed and unforgeable, but we still
//!   route them through `validate_obj_id` so a future ACL hook (e.g. "is
//!   this object readable by the recipient zone?") has a single entry
//!   point to extend.
//! - **Local paths** are checked against the agent runtime filesystem policy;
//!   workspace mode confines them to the workspace directory, while
//!   unrestricted mode permits host-readable absolute paths.
//! - **URLs** pass through today; the scheme check lives here for future
//!   tightening (e.g. block `file://` from being smuggled as a URL).
//!
//! Rejected references stay in the outbound text as inert markers — the
//! recipient still sees what the LLM tried to attach, and the audit log
//! captures the violation, but no structured ref is created.

use std::path::{Path, PathBuf};

use llm_context::{AttachmentValidation, AttachmentValidator};

use crate::agent_config::FilesystemPolicy;

/// Validator that bounds outbound `<attachment path=…>` according to the
/// agent's path policy:
///
/// - `Workspace` (default): path must resolve under `workspace_root` after
///   canonicalization — symlink-escape and `..` traversal are rejected.
/// - `Unrestricted`: workspace fence is lifted, so the agent can attach
///   host-readable files such as `/opt/buckyos/logs/...`. `..` traversal
///   is still rejected so audit logs surface the raw input verbatim.
pub struct WorkspaceAttachmentValidator {
    workspace_root: Option<PathBuf>,
    agent_id: String,
    path_policy: FilesystemPolicy,
}

impl WorkspaceAttachmentValidator {
    pub fn new(workspace_root: Option<PathBuf>, agent_id: impl Into<String>) -> Self {
        Self::with_policy(workspace_root, agent_id, FilesystemPolicy::default())
    }

    pub fn with_policy(
        workspace_root: Option<PathBuf>,
        agent_id: impl Into<String>,
        path_policy: FilesystemPolicy,
    ) -> Self {
        let canonical = workspace_root.and_then(|p| {
            std::fs::canonicalize(&p).ok().or_else(|| {
                // Workspace dir may not exist on disk yet for brand-new
                // sessions. Fall back to the lexical path; the per-path
                // check below still rejects `..` traversal.
                Some(p)
            })
        });
        Self {
            workspace_root: canonical,
            agent_id: agent_id.into(),
            path_policy,
        }
    }

    fn check_path(&self, raw: &str) -> AttachmentValidation {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("attachment path is empty".to_string());
        }
        let path = Path::new(trimmed);
        if path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::Prefix(_)
            )
        }) {
            return Err(format!(
                "attachment path `{trimmed}` rejected: contains `..` or volume prefix"
            ));
        }
        match self.path_policy {
            FilesystemPolicy::Unrestricted => {
                // Workspace fence lifted by config. `..` traversal is still
                // rejected above; everything else the host can read is
                // fair game (e.g. /opt/buckyos/logs/aicc/*.html). Relative
                // paths require a workspace_root to anchor against.
                if !path.is_absolute() && self.workspace_root.is_none() {
                    return Err(format!(
                        "attachment path `{trimmed}` rejected: relative path needs workspace anchor (agent `{}`)",
                        self.agent_id
                    ));
                }
                Ok(())
            }
            FilesystemPolicy::Workspace => {
                let Some(root) = self.workspace_root.as_deref() else {
                    return Err(format!(
                        "attachment path `{trimmed}` rejected: agent `{}` has no workspace bound",
                        self.agent_id
                    ));
                };
                let absolute = if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    root.join(path)
                };
                // canonicalize when the target exists — resolves any symlink
                // chain and lets us compare against the workspace root
                // deterministically.
                let resolved = std::fs::canonicalize(&absolute).unwrap_or(absolute);
                if !resolved.starts_with(root) {
                    return Err(format!(
                        "attachment path `{trimmed}` rejected: resolves outside workspace `{}`",
                        root.display()
                    ));
                }
                Ok(())
            }
        }
    }
}

impl AttachmentValidator for WorkspaceAttachmentValidator {
    fn validate_path(&self, path: &str) -> AttachmentValidation {
        self.check_path(path)
    }

    fn validate_obj_id(&self, _obj_id: &ndn_lib::ObjId) -> AttachmentValidation {
        // Content-addressed: no spoofing risk on the wire. ACL enforcement
        // belongs to msg-center's accept path, not the producer side —
        // the recipient zone is the authoritative judge of "can I read
        // this object." Hook stays here so a future agent-side ACL check
        // can be added without re-threading the validator.
        Ok(())
    }

    fn validate_url(&self, url: &str) -> AttachmentValidation {
        // Reject `file://` smuggling: workspace path checking would
        // otherwise be bypassed by re-encoding a local path as a URL.
        let lowered = url.trim().to_ascii_lowercase();
        if lowered.starts_with("file:") {
            return Err(format!(
                "attachment url `{url}` rejected: file:// scheme not allowed at egress"
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn allows_path_inside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let v = WorkspaceAttachmentValidator::new(Some(dir.path().to_path_buf()), "agent-a");
        let inner = dir.path().join("file.txt");
        fs::write(&inner, b"hi").unwrap();
        assert!(v.validate_path(inner.to_str().unwrap()).is_ok());
        assert!(v.validate_path("file.txt").is_ok());
    }

    #[test]
    fn rejects_dot_dot_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let v = WorkspaceAttachmentValidator::new(Some(dir.path().to_path_buf()), "agent-a");
        assert!(v.validate_path("../etc/passwd").is_err());
    }

    #[test]
    fn rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside_root = tempfile::tempdir().unwrap();
        let outside = outside_root.path().join("secret");
        fs::write(&outside, b"secret").unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let v = WorkspaceAttachmentValidator::new(Some(dir.path().to_path_buf()), "agent-a");
        #[cfg(unix)]
        assert!(v.validate_path(link.to_str().unwrap()).is_err());
        let _ = link;
    }

    #[test]
    fn rejects_file_url() {
        let v = WorkspaceAttachmentValidator::new(None, "agent-a");
        assert!(v.validate_url("file:///etc/passwd").is_err());
        assert!(v.validate_url("https://example.com/x.png").is_ok());
    }

    #[test]
    fn empty_workspace_rejects_paths() {
        let v = WorkspaceAttachmentValidator::new(None, "agent-a");
        assert!(v.validate_path("file.txt").is_err());
    }

    #[test]
    fn unrestricted_allows_paths_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let v = WorkspaceAttachmentValidator::with_policy(
            Some(dir.path().to_path_buf()),
            "agent-a",
            FilesystemPolicy::Unrestricted,
        );
        // /tmp or any host-readable absolute path passes when fence is lifted.
        assert!(v.validate_path("/opt/buckyos/logs/aicc.html").is_ok());
        // `..` traversal still rejected even under unrestricted.
        assert!(v.validate_path("../etc/passwd").is_err());
    }

    #[test]
    fn unrestricted_relative_needs_workspace_anchor() {
        let v = WorkspaceAttachmentValidator::with_policy(
            None,
            "agent-a",
            FilesystemPolicy::Unrestricted,
        );
        assert!(v.validate_path("file.txt").is_err());
        assert!(v.validate_path("/etc/hosts").is_ok());
    }
}
