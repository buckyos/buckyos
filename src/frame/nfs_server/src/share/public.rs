//! Public projection (design §3.9, D4): `srv/publish` is what the gateway
//! already serves anonymously as `public.<zone>`. nfs-server only observes it
//! — first seen, gone, and whether a symlink still points inside the zone's
//! shareable areas — and reports "path reachable", never "NDN object ready".

use super::db::PublicObservation;
use crate::fsutil::unix_now;
use crate::state::{AppState, SharedState};
use std::path::{Path, PathBuf};

const PUBLISH: &str = "publish";
/// RFC 3986 path segment: everything but unreserved characters is escaped.
const SEGMENT: &percent_encoding::AsciiSet =
    &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');
const MAX_ENTRIES: usize = 20_000;

/// `https://public.<zone>/<rel>` for a path relative to `srv/publish`.
pub fn public_url(state: &AppState, rel: &str) -> Option<String> {
    let base = state.config.share.public_base_url.as_deref()?;
    let encoded = rel
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| percent_encoding::utf8_percent_encode(s, SEGMENT).to_string())
        .collect::<Vec<_>>()
        .join("/");
    Some(format!("{}/{}", base.trim_end_matches('/'), encoded))
}

/// True when `rel` (relative to the `srv` root) lies in the publish dir.
pub fn in_publish(root: &str, rel: &str) -> Option<String> {
    if root != "srv" {
        return None;
    }
    if rel == PUBLISH {
        return Some(String::new());
    }
    rel.strip_prefix("publish/").map(String::from)
}

impl AppState {
    fn publish_dir(&self) -> Option<PathBuf> {
        self.config.root("srv").map(|r| r.path.join(PUBLISH)).filter(|p| p.is_dir())
    }

    /// A symlink in Public may only lead into `srv/publish` or `srv/library`.
    fn link_allowed(&self, target: &Path) -> bool {
        let Some(srv) = self.config.root("srv") else { return false };
        [PUBLISH, "library"].iter().any(|area| {
            std::fs::canonicalize(srv.path.join(area)).is_ok_and(|base| target.starts_with(base))
        })
    }

    /// One scan of `srv/publish` into `public_observations`.
    pub fn share_public_scan(self: &SharedState) {
        let now = unix_now();
        let Some(dir) = self.publish_dir() else {
            let _ = self.db.public_observations_apply(&[], now);
            self.share.set_public_scan_state(now, false);
            return;
        };
        let mut seen = Vec::new();
        let mut stack = vec![(dir.clone(), String::new())];
        let mut ok = true;
        while let Some((disk, rel)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&disk) else {
                ok = false;
                continue;
            };
            for entry in entries.flatten() {
                if seen.len() >= MAX_ENTRIES {
                    ok = false;
                    break;
                }
                let Ok(name) = entry.file_name().into_string() else { continue };
                if name.starts_with(".nfs_tmp_") {
                    continue;
                }
                let child_rel = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                let path = entry.path();
                let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64);
                let (kind, link_target, link_ok, size) = if meta.file_type().is_symlink() {
                    let target = std::fs::read_link(&path).ok().map(|t| t.to_string_lossy().into_owned());
                    let ok = std::fs::canonicalize(&path).is_ok_and(|real| self.link_allowed(&real));
                    let size = ok.then(|| std::fs::metadata(&path).ok().map(|m| m.len() as i64)).flatten();
                    ("symlink", target, Some(ok), size)
                } else if meta.is_dir() {
                    stack.push((path.clone(), child_rel.clone()));
                    ("dir", None, None, None)
                } else {
                    ("file", None, None, Some(meta.len() as i64))
                };
                seen.push(PublicObservation {
                    rel_path: child_rel,
                    entry_kind: kind.into(),
                    link_target,
                    link_ok,
                    size,
                    mtime,
                    first_seen_at: now,
                    last_seen_at: now,
                    missing_since: None,
                });
            }
        }
        if let Err(e) = self.db.public_observations_apply(&seen, now) {
            log::warn!("public scan store failed: {}", e);
            ok = false;
        }
        self.share.set_public_scan_state(now, ok);
    }
}
