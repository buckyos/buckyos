//! Upload staging: a minimal tus-style resumable upload area (nfs_server.md N1:
//! 内嵌最小实现,报文对齐 tus/NDM). Sessions are in-memory and staging files are
//! wiped at startup — uploads never enter the namespace before commit (NFSP D4).
//!
//! SHA-256 is computed incrementally as bytes arrive, so commit_file gets the
//! full hash for free (feeds probe/秒传 through filedb's content_index).

use crate::error::{invalid, not_found, ErrorCode, NfsError, NfsResult};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct UploadMgr {
    dir: PathBuf,
    sessions: Mutex<HashMap<String, UploadState>>,
}

enum UploadState {
    Idle(UploadSession),
    /// A PATCH is in flight; concurrent PATCHes are rejected (tus requires serial).
    Busy,
}

pub struct UploadSession {
    pub path: PathBuf,
    pub offset: u64,
    pub expected_len: Option<u64>,
    hasher: Sha256,
}

pub struct FinishedUpload {
    pub path: PathBuf,
    pub size: u64,
    pub sha256: String,
}

impl UploadMgr {
    /// Creates the manager, clearing any staging leftovers from a previous run.
    pub fn new(dir: PathBuf) -> NfsResult<Self> {
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(UploadMgr { dir, sessions: Mutex::new(HashMap::new()) })
    }

    pub fn create(&self, expected_len: Option<u64>) -> NfsResult<String> {
        let fb = format!("fb_{}", uuid::Uuid::new_v4().simple());
        let path = self.dir.join(&fb);
        std::fs::File::create(&path)?;
        self.sessions.lock().unwrap().insert(
            fb.clone(),
            UploadState::Idle(UploadSession {
                path,
                offset: 0,
                expected_len,
                hasher: Sha256::new(),
            }),
        );
        Ok(fb)
    }

    pub fn offset(&self, fb: &str) -> NfsResult<(u64, Option<u64>)> {
        match self.sessions.lock().unwrap().get(fb) {
            Some(UploadState::Idle(s)) => Ok((s.offset, s.expected_len)),
            Some(UploadState::Busy) => {
                Err(NfsError::new(ErrorCode::LeaseConflict, "upload busy"))
            }
            None => Err(not_found(format!("unknown upload '{}'", fb))),
        }
    }

    /// Appends a chunk at `offset` (must equal the current offset).
    pub async fn append(&self, fb: &str, offset: u64, data: &[u8]) -> NfsResult<u64> {
        let mut session = {
            let mut map = self.sessions.lock().unwrap();
            match map.get_mut(fb) {
                Some(state @ UploadState::Idle(_)) => {
                    match std::mem::replace(state, UploadState::Busy) {
                        UploadState::Idle(s) => s,
                        UploadState::Busy => unreachable!(),
                    }
                }
                Some(UploadState::Busy) => {
                    return Err(NfsError::new(
                        ErrorCode::LeaseConflict,
                        "concurrent PATCH on the same upload",
                    ))
                }
                None => return Err(not_found(format!("unknown upload '{}'", fb))),
            }
        };
        let put_back = |s: UploadState, map: &Mutex<HashMap<String, UploadState>>| {
            map.lock().unwrap().insert(fb.to_string(), s);
        };
        if offset != session.offset {
            let cur = session.offset;
            put_back(UploadState::Idle(session), &self.sessions);
            return Err(invalid(format!(
                "upload offset mismatch: expected {}, got {}",
                cur, offset
            ))
            .with("expected_offset", serde_json::json!(cur)));
        }
        if let Some(exp) = session.expected_len {
            if session.offset + data.len() as u64 > exp {
                put_back(UploadState::Idle(session), &self.sessions);
                return Err(invalid("upload exceeds declared Upload-Length"));
            }
        }
        let res = async {
            use tokio::io::AsyncWriteExt;
            let mut f = tokio::fs::OpenOptions::new()
                .append(true)
                .open(&session.path)
                .await?;
            f.write_all(data).await?;
            f.flush().await?;
            Ok::<(), std::io::Error>(())
        }
        .await;
        match res {
            Ok(()) => {
                session.hasher.update(data);
                session.offset += data.len() as u64;
                let new_offset = session.offset;
                put_back(UploadState::Idle(session), &self.sessions);
                Ok(new_offset)
            }
            Err(e) => {
                put_back(UploadState::Idle(session), &self.sessions);
                Err(e.into())
            }
        }
    }

    /// Completes the upload: removes the session, leaving the staged file for
    /// commit_file to move into place.
    pub fn finish(&self, fb: &str) -> NfsResult<FinishedUpload> {
        let state = self
            .sessions
            .lock()
            .unwrap()
            .remove(fb)
            .ok_or_else(|| not_found(format!("unknown upload '{}'", fb)))?;
        let s = match state {
            UploadState::Idle(s) => s,
            UploadState::Busy => {
                return Err(NfsError::new(ErrorCode::LeaseConflict, "upload busy"))
            }
        };
        if let Some(exp) = s.expected_len {
            if s.offset != exp {
                // Put it back: commit of an incomplete upload is a client bug.
                let cur = s.offset;
                self.sessions.lock().unwrap().insert(fb.to_string(), UploadState::Idle(s));
                return Err(invalid(format!(
                    "upload incomplete: {} of {} bytes",
                    cur, exp
                )));
            }
        }
        Ok(FinishedUpload {
            path: s.path,
            size: s.offset,
            sha256: hex::encode(s.hasher.finalize()),
        })
    }

    pub fn abort(&self, fb: &str) {
        if let Some(UploadState::Idle(s)) = self.sessions.lock().unwrap().remove(fb) {
            let _ = std::fs::remove_file(s.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn upload_flow_and_hash() {
        let dir = tempfile::tempdir().unwrap();
        let m = UploadMgr::new(dir.path().join("staging")).unwrap();
        let fb = m.create(Some(11)).unwrap();
        assert_eq!(m.offset(&fb).unwrap().0, 0);
        assert_eq!(m.append(&fb, 0, b"hello ").await.unwrap(), 6);
        // Wrong offset rejected, state intact.
        let err = m.append(&fb, 3, b"x").await.unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidArgument);
        assert_eq!(m.offset(&fb).unwrap().0, 6);
        assert_eq!(m.append(&fb, 6, b"world").await.unwrap(), 11);
        let fin = m.finish(&fb).unwrap();
        assert_eq!(fin.size, 11);
        let expect = hex::encode(Sha256::digest(b"hello world"));
        assert_eq!(fin.sha256, expect);
        assert_eq!(std::fs::read(&fin.path).unwrap(), b"hello world");
        // Session gone after finish.
        assert!(m.offset(&fb).is_err());
    }

    #[tokio::test]
    async fn incomplete_finish_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let m = UploadMgr::new(dir.path().join("st")).unwrap();
        let fb = m.create(Some(10)).unwrap();
        m.append(&fb, 0, b"abc").await.unwrap();
        assert!(m.finish(&fb).is_err());
        // Still resumable.
        assert_eq!(m.offset(&fb).unwrap().0, 3);
        m.append(&fb, 3, b"defghij").await.unwrap();
        assert!(m.finish(&fb).is_ok());
    }

    #[tokio::test]
    async fn length_overflow_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let m = UploadMgr::new(dir.path().join("st")).unwrap();
        let fb = m.create(Some(3)).unwrap();
        assert!(m.append(&fb, 0, b"abcd").await.is_err());
        m.abort(&fb);
        assert!(m.offset(&fb).is_err());
    }

    #[test]
    fn staging_wiped_on_start() {
        let dir = tempfile::tempdir().unwrap();
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("orphan"), b"x").unwrap();
        let _m = UploadMgr::new(staging.clone()).unwrap();
        assert!(!staging.join("orphan").exists());
    }
}
