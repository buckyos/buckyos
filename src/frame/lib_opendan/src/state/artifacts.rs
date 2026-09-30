//! Artifact list (§6.5): registration and pointers only. Workspace
//! versioning / rollback is not part of the session protocol (Q13).
//!
//! Every head move and every validity change of a version holds the
//! `artifact:<aid>` lock; head and versions are re-read inside the lock.
//! Lock order is always session → artifact.

use std::collections::HashSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::Lease;
use crate::protocol::*;

use super::fs_client::AgentLayout;
use super::Artifacts;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecideResult {
    pub version: ArtifactVersion,
    pub head_before: Option<String>,
    pub head_after: Option<String>,
}

pub(super) struct FsArtifacts {
    layout: AgentLayout,
}

impl FsArtifacts {
    pub(super) fn new(layout: AgentLayout) -> Self {
        Self { layout }
    }

    fn read_head(&self, aid: &str) -> Result<Option<ArtifactHead>> {
        fsutil::read_json_opt(&self.layout.artifact_head(aid))
    }

    fn read_version(&self, aid: &str, ver: &str) -> Result<Option<ArtifactVersion>> {
        fsutil::read_json_opt(&self.layout.artifact_version(aid, ver))
    }

    fn check_lock(lease: &Lease, aid: &str) -> Result<()> {
        if lease.resource() != format!("artifact:{aid}") {
            return Err(OpenDanError::Artifact(format!(
                "operation on artifact {aid} requires the artifact:{aid} lock (holding {})",
                lease.resource()
            )));
        }
        lease.check()
    }
}

/// Nearest ancestor of `start` that is still `accepted` (walking `base`
/// links). Missing links or cycles are explicit errors; no valid ancestor →
/// `Ok(None)`.
pub fn nearest_valid_base(
    lookup: &dyn Fn(&str) -> Result<Option<ArtifactVersion>>,
    start: Option<&str>,
) -> Result<Option<String>> {
    let mut seen = HashSet::new();
    let mut cur = start.map(str::to_string);
    while let Some(ver) = cur {
        if !seen.insert(ver.clone()) {
            return Err(OpenDanError::Artifact(format!(
                "cycle in artifact version ancestry at {ver}"
            )));
        }
        let v = lookup(&ver)?.ok_or_else(|| {
            OpenDanError::Artifact(format!("artifact version {ver} referenced as base is missing"))
        })?;
        if v.state == VersionState::Accepted {
            return Ok(Some(ver));
        }
        cur = v.base.clone();
    }
    Ok(None)
}

#[async_trait]
impl Artifacts for FsArtifacts {
    async fn head(&self, aid: &str) -> Result<Option<ArtifactHead>> {
        self.read_head(aid)
    }

    async fn version(&self, aid: &str, ver: &str) -> Result<Option<ArtifactVersion>> {
        self.read_version(aid, ver)
    }

    async fn versions(&self, aid: &str) -> Result<Vec<ArtifactVersion>> {
        let dir = self.layout.artifacts_dir().join(aid).join("versions");
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        for e in entries.flatten() {
            if let Some(v) = fsutil::read_json_opt::<ArtifactVersion>(&e.path())? {
                out.push(v);
            }
        }
        out.sort_by(|a, b| a.ver.cmp(&b.ver));
        Ok(out)
    }

    async fn list(&self) -> Result<Vec<ArtifactHead>> {
        let dir = self.layout.artifacts_dir();
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        for e in entries.flatten() {
            if let Some(h) = fsutil::read_json_opt::<ArtifactHead>(&e.path().join("artifact.json"))? {
                out.push(h);
            }
        }
        out.sort_by(|a, b| a.aid.cmp(&b.aid));
        Ok(out)
    }

    async fn register_version(
        &self,
        session_lease: &Lease,
        aid: &str,
        workspace: Option<WorkspaceRef>,
        mut version: ArtifactVersion,
    ) -> Result<()> {
        session_lease.check()?;
        crate::ids::validate_session_id(aid)?;
        let head = ArtifactHead {
            aid: aid.to_string(),
            workspace,
            head: None,
            rev: 0,
            updated_at_ms: crate::now_ms(),
        };
        // Created once; later head moves happen under the artifact lock.
        fsutil::publish_noreplace_json(&self.layout.artifact_head(aid), &head)?;
        // A version keeps its decided state: registering again only refreshes
        // outputs while it is still `produced`.
        if let Some(cur) = self.read_version(aid, &version.ver)? {
            if cur.state != VersionState::Produced {
                return Ok(());
            }
        }
        version.state = VersionState::Produced;
        version.updated_at_ms = crate::now_ms();
        session_lease.fenced(|| {
            fsutil::atomic_replace_json(&self.layout.artifact_version(aid, &version.ver), &version)
        })
    }

    async fn decide(
        &self,
        artifact_lease: &Lease,
        aid: &str,
        ver: &str,
        decision: &str,
    ) -> Result<DecideResult> {
        Self::check_lock(artifact_lease, aid)?;
        // Re-read inside the lock — never trust a cached head.
        let mut head = self
            .read_head(aid)?
            .ok_or_else(|| OpenDanError::Artifact(format!("artifact {aid} is not registered")))?;
        let mut v = self
            .read_version(aid, ver)?
            .ok_or_else(|| OpenDanError::Artifact(format!("artifact version {ver} is missing")))?;
        let head_before = head.head.clone();
        match decision {
            "accept" => {
                if v.state == VersionState::Discarded {
                    return Err(OpenDanError::Artifact(format!(
                        "version {ver} was discarded and cannot be accepted"
                    )));
                }
                v.state = VersionState::Accepted;
                head.head = Some(ver.to_string());
            }
            "discard" => {
                v.state = VersionState::Discarded;
                if head.head.as_deref() == Some(ver) {
                    // Only fall back to an ancestor that is still accepted.
                    let lookup = |x: &str| -> Result<Option<ArtifactVersion>> {
                        if x == ver {
                            Ok(Some(v.clone()))
                        } else {
                            self.read_version(aid, x)
                        }
                    };
                    head.head = nearest_valid_base(&lookup, v.base.as_deref())?;
                }
                // head pointing to another version stays untouched.
            }
            other => {
                return Err(OpenDanError::InvalidArgument(format!(
                    "unknown decision `{other}` (accept | discard)"
                )))
            }
        }
        v.updated_at_ms = crate::now_ms();
        artifact_lease.fenced(|| {
            fsutil::atomic_replace_json(&self.layout.artifact_version(aid, ver), &v)
        })?;
        if head.head != head_before {
            head.rev += 1;
            head.updated_at_ms = crate::now_ms();
            artifact_lease
                .fenced(|| fsutil::atomic_replace_json(&self.layout.artifact_head(aid), &head))?;
        }
        Ok(DecideResult {
            version: v,
            head_before,
            head_after: head.head,
        })
    }
}
