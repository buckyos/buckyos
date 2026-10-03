//! Agent Session directory (§4): paths, reading, creation, and the loaded
//! lease-bound [`Session`] used by the runner to commit state.

mod live;
pub mod runs;
pub mod worklog;

use std::path::{Path, PathBuf};

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::{Acquire, Lease};
use crate::protocol::*;

pub use live::Session;
pub use runs::SessionRuns;
pub use worklog::Worklog;

/// A session directory on disk. Cheap to clone; every accessor reads files.
#[derive(Debug, Clone)]
pub struct SessionDir {
    root: PathBuf,
    sid: String,
}

/// Result of the directory publication step of session creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Publish {
    Created,
    AlreadyExists,
}

impl SessionDir {
    /// Open an existing session directory.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let root = path.as_ref().to_path_buf();
        let cfg_path = root.join(STATE_DIR).join(SESSION_CONFIG_FILE);
        if !cfg_path.is_file() {
            return Err(OpenDanError::NotFound(format!(
                "{} is not a session directory (missing {})",
                root.display(),
                cfg_path.display()
            )));
        }
        let sid = root
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        Ok(Self { root, sid })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn sid(&self) -> &str {
        &self.sid
    }

    pub fn state_dir(&self) -> PathBuf {
        self.root.join(STATE_DIR)
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join(RUNTIME_DIR)
    }

    pub fn runtime_bin_dir(&self) -> PathBuf {
        self.runtime_dir().join("bin")
    }

    pub fn runs_dir(&self) -> PathBuf {
        self.state_dir().join(RUNS_DIR)
    }

    pub fn lease_path(&self) -> PathBuf {
        self.state_dir().join(LEASE_FILE)
    }

    pub fn file(&self, name: &str) -> PathBuf {
        self.state_dir().join(name)
    }

    pub fn config(&self) -> Result<SessionConfig> {
        fsutil::read_json(&self.file(SESSION_CONFIG_FILE))
    }

    pub fn state(&self) -> Result<SessionState> {
        fsutil::read_json(&self.file(STATE_FILE))
    }

    pub fn summary_opt(&self) -> Result<Option<SessionSummary>> {
        fsutil::read_json_opt(&self.file(SUMMARY_FILE))
    }

    pub fn binding_opt(&self) -> Result<Option<Binding>> {
        let value: Option<serde_json::Value> = fsutil::read_json_opt(&self.file(BINDING_FILE))?;
        if value
            .as_ref()
            .is_some_and(|v| v["schema"] != "opendan.binding/3")
        {
            return Err(OpenDanError::blocked("unsupported binding format", None));
        }
        let binding: Option<Binding> = value
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| OpenDanError::json(&self.file(BINDING_FILE), e.to_string()))?;
        Ok(binding)
    }

    pub fn statistics(&self) -> Result<SessionStatic> {
        Ok(fsutil::read_json_opt(&self.file(STATIC_FILE))?.unwrap_or_default())
    }

    pub fn report(&self) -> Option<String> {
        std::fs::read_to_string(self.root.join(REPORT_FILE)).ok()
    }

    pub fn readme(&self) -> Option<String> {
        std::fs::read_to_string(self.root.join(README_FILE)).ok()
    }

    pub fn worklog(&self) -> Worklog {
        Worklog::new(self.file(WORKLOG_FILE))
    }

    pub fn runs(&self) -> SessionRuns {
        SessionRuns::new(self.runs_dir())
    }

    /// Current lock holder (display only).
    pub fn holder(&self) -> Option<LockInfo> {
        crate::lock::read_holder_info(&self.lease_path())
    }

    /// Acquire the session lease. Refuses principals other than the driver
    /// (Q9: the driving identity never changes; processes may).
    pub fn acquire(&self, holder: HolderInfo) -> Result<Acquire> {
        let cfg = self.config()?;
        if cfg.session.driver.principal != holder.principal {
            return Err(OpenDanError::NotDriver {
                session_id: self.sid.clone(),
                principal: holder.principal,
                driver: cfg.session.driver.principal,
            });
        }
        Lease::acquire(&format!("session:{}", self.sid), &self.lease_path(), holder)
    }

    /// Load the session under a held lease for committing.
    pub fn load(&self, lease: &Lease) -> Result<Session> {
        lease.check()?;
        Session::load(self.clone())
    }

    /// Publish a new session directory built in a temp dir next to it.
    ///
    /// Writes `session_config.json`, the first worklog entry, `state.json`
    /// (rev 1, `created`) and `readme.md`, then renames the temp directory to
    /// `<parent>/<sid>` without replacing an existing one.
    pub fn publish_new(
        parent: &Path,
        config: &SessionConfig,
        created: WorklogBody,
        readme: &str,
    ) -> Result<(SessionDir, Publish)> {
        let sid = config.session.session_id.clone();
        crate::ids::validate_session_id(&sid)?;
        std::fs::create_dir_all(parent).map_err(|e| OpenDanError::io(parent, e))?;
        let final_dir = parent.join(&sid);
        if final_dir
            .join(STATE_DIR)
            .join(SESSION_CONFIG_FILE)
            .is_file()
        {
            return Ok((SessionDir::open(&final_dir)?, Publish::AlreadyExists));
        }
        let tmp = parent.join(format!(".tmp-{}", uuid::Uuid::new_v4().simple()));
        let build = || -> Result<()> {
            let state_dir = tmp.join(STATE_DIR);
            std::fs::create_dir_all(&state_dir).map_err(|e| OpenDanError::io(&state_dir, e))?;
            fsutil::atomic_replace_json(&state_dir.join(SESSION_CONFIG_FILE), config)?;
            let entry = WorklogEntry {
                seq: 1,
                body: created,
            };
            let end = fsutil::append_batch(
                &state_dir.join(WORKLOG_FILE),
                &fsutil::to_json_lines(&[entry])?,
            )?;
            let state = SessionState::initial_for(
                config,
                WorklogBoundary {
                    committed_seq: 1,
                    committed_bytes: end,
                },
                crate::now_ms(),
            );
            fsutil::atomic_replace_json(&state_dir.join(STATE_FILE), &state)?;
            fsutil::atomic_replace(&tmp.join(README_FILE), readme.as_bytes())?;
            Ok(())
        };
        if let Err(e) = build() {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(e);
        }
        if fsutil::publish_dir(&tmp, &final_dir)? {
            Ok((SessionDir::open(&final_dir)?, Publish::Created))
        } else {
            let _ = std::fs::remove_dir_all(&tmp);
            Ok((SessionDir::open(&final_dir)?, Publish::AlreadyExists))
        }
    }
}

/// Render the human readable `readme.md` of a new session.
pub fn render_readme(cfg: &SessionConfig) -> String {
    let mut s = String::new();
    let title = if cfg.session.objective.trim().is_empty() {
        cfg.session.session_id.clone()
    } else {
        cfg.session
            .objective
            .lines()
            .next()
            .unwrap_or_default()
            .chars()
            .take(120)
            .collect()
    };
    s.push_str(&format!("# {title}\n\n"));
    s.push_str(&format!("- session: `{}`\n", cfg.session.session_id));
    s.push_str(&format!("- kind: {}\n", cfg.session.kind.as_str()));
    s.push_str(&format!("- agent: {}\n", cfg.session.agent_did));
    s.push_str(&format!(
        "- created by: {} (via {})\n",
        cfg.session.created_by.principal, cfg.session.created_by.via
    ));
    s.push_str(&format!("- driver: {}\n", cfg.session.driver.principal));
    if let Some(origin) = &cfg.session.origin {
        if let Some(p) = &origin.parent_session {
            s.push_str(&format!("- origin: {p}\n"));
        }
    }
    if !cfg.session.objective.trim().is_empty() {
        s.push_str("\n## Objective\n\n");
        s.push_str(cfg.session.objective.trim());
        s.push('\n');
    }
    s
}
