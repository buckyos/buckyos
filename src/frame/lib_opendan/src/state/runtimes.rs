//! Runtime registrations of an agent (`state/runtimes/<runtime_id>.json`):
//! the runtimes its sessions and workspaces may use, whether this host can
//! use them now, and where workspaces are best placed in them. Writes hold
//! the `runtimes` lock; updates are compare-and-set on `revision`.

use async_trait::async_trait;

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::FileLock;
use crate::protocol::*;

use super::{AgentLayout, RuntimeRegistry};

pub(super) struct FsRuntimes {
    layout: AgentLayout,
}

fn invalid(code: &str, message: impl std::fmt::Display) -> OpenDanError {
    OpenDanError::InvalidArgument(format!("runtime {code}: {message}"))
}

fn validate_id(id: &str) -> Result<()> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && id != LEGACY_LOCAL_RUNTIME;
    if !ok {
        return Err(invalid(
            "invalid_id",
            format!("`{id}` (lowercase letters, digits, `-`, `_`; `{LEGACY_LOCAL_RUNTIME}` is reserved)"),
        ));
    }
    Ok(())
}

impl FsRuntimes {
    pub(super) fn new(layout: AgentLayout) -> Self {
        Self { layout }
    }

    async fn lock(&self) -> Result<FileLock> {
        let path = self.layout.lock_path("runtimes")?;
        for _ in 0..100 {
            if let Some(l) = FileLock::try_acquire(&path)? {
                return Ok(l);
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        Err(OpenDanError::Busy {
            resource: "runtimes".into(),
            holder: None,
        })
    }

    fn read(&self, id: &str) -> Result<Option<RuntimeRecord>> {
        validate_id(id)?;
        let record = fsutil::read_json_opt::<RuntimeRecord>(&self.layout.runtime_entry(id))?;
        if record
            .as_ref()
            .is_some_and(|r| r.runtime_id != id || r.version != RUNTIME_RECORD_VERSION)
        {
            return Err(invalid("invalid_registration", id));
        }
        Ok(record)
    }

    fn require(&self, id: &str) -> Result<RuntimeRecord> {
        self.read(id)?
            .ok_or_else(|| OpenDanError::NotFound(format!("runtime {id}")))
    }

    fn write(&self, record: &RuntimeRecord) -> Result<()> {
        let dir = self.layout.runtimes_dir();
        std::fs::create_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
        fsutil::atomic_replace_json(&self.layout.runtime_entry(&record.runtime_id), record)
    }

    fn all(&self) -> Result<Vec<RuntimeRecord>> {
        let dir = self.layout.runtimes_dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        let mut out = Vec::new();
        for entry in rd {
            let entry = entry.map_err(|e| OpenDanError::io(&dir, e))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_suffix(".json").filter(|n| !n.starts_with('.')) {
                match self.read(id) {
                    Ok(Some(r)) => out.push(r),
                    Ok(None) => {}
                    Err(e) => log::warn!("runtime registration {name}: {e}"),
                }
            }
        }
        out.sort_by(|a, b| a.runtime_id.cmp(&b.runtime_id));
        Ok(out)
    }

    /// Whether this host can use the runtime now. Only the agent's own
    /// environment is executed directly; any other kind needs an executor
    /// this runner does not have yet.
    fn observe(&self, record: &mut RuntimeRecord) {
        let (availability, error) = match (record.kind, &record.exec) {
            (_, None) => (
                RuntimeAvailability::RuntimeUnavailable,
                Some(format!("no executor for the `{}` runtime yet", record.runtime_id)),
            ),
            (RuntimeKind::Container, Some(_)) if record.runtime_host != crate::runtime::native_host_id() => (
                RuntimeAvailability::RuntimeUnavailable,
                Some("registered by another host".to_string()),
            ),
            (RuntimeKind::Container, Some(_)) => (RuntimeAvailability::Available, None),
            (kind, Some(_)) => (
                RuntimeAvailability::RuntimeUnavailable,
                Some(format!("this runner cannot use {kind:?} runtimes yet")),
            ),
        };
        record.availability = availability;
        record.last_error = error;
        record.checked_at_ms = Some(crate::now_ms());
    }

    fn new_record(&self, req: &RuntimeRegister, who: &str, builtin: bool) -> RuntimeRecord {
        let now = crate::now_ms();
        let mut record = RuntimeRecord {
            version: RUNTIME_RECORD_VERSION,
            runtime_id: req.runtime_id.clone(),
            kind: req.kind,
            name: req.name.clone(),
            description: req.description.clone(),
            runtime_host: crate::runtime::native_host_id(),
            exec: req.exec.clone(),
            roots: req.roots.clone(),
            capabilities: req.capabilities.clone(),
            availability: RuntimeAvailability::RuntimeUnavailable,
            builtin,
            revision: 1,
            registered_by: who.to_string(),
            registered_at_ms: now,
            updated_at_ms: now,
            checked_at_ms: None,
            last_error: None,
        };
        self.observe(&mut record);
        record
    }
}

fn same_registration(r: &RuntimeRecord, req: &RuntimeRegister) -> bool {
    r.kind == req.kind
        && r.name == req.name
        && r.description == req.description
        && r.exec == req.exec
        && r.roots == req.roots
}

#[async_trait]
impl RuntimeRegistry for FsRuntimes {
    async fn list(&self) -> Result<Vec<RuntimeRecord>> {
        self.all()
    }

    async fn lookup(&self, runtime_id: &str) -> Result<Option<RuntimeRecord>> {
        self.read(canonical_runtime(runtime_id))
    }

    async fn register(&self, req: &RuntimeRegister, who: &str) -> Result<RuntimeRecord> {
        validate_id(&req.runtime_id)?;
        if who.trim().is_empty() || req.name.trim().is_empty() {
            return Err(invalid("invalid_registration", "a name and the caller are required"));
        }
        let _lock = self.lock().await?;
        if let Some(existing) = self.read(&req.runtime_id)? {
            if !existing.builtin && same_registration(&existing, req) {
                return Ok(existing);
            }
            return Err(invalid(
                "already_registered",
                format!("{} (update it, or unregister it first)", req.runtime_id),
            ));
        }
        let record = self.new_record(req, who, false);
        self.write(&record)?;
        Ok(record)
    }

    async fn update(&self, runtime_id: &str, req: &RuntimeUpdate, _who: &str) -> Result<RuntimeRecord> {
        let _lock = self.lock().await?;
        let mut record = self.require(canonical_runtime(runtime_id))?;
        if record.revision != req.expected_revision {
            return Err(invalid(
                "revision_conflict",
                format!("expected {}, current {}", req.expected_revision, record.revision),
            ));
        }
        if let Some(n) = req.name.as_ref().filter(|n| !n.trim().is_empty()) {
            record.name = n.clone();
        }
        if let Some(d) = &req.description {
            record.description = d.clone();
        }
        if let Some(r) = &req.roots {
            if let Some(bad) = r.iter().find(|p| !p.is_absolute()) {
                return Err(invalid("invalid_root", format!("{} is not absolute", bad.display())));
            }
            record.roots = r.clone();
        }
        record.revision += 1;
        record.updated_at_ms = crate::now_ms();
        self.write(&record)?;
        Ok(record)
    }

    async fn check(&self, runtime_id: &str) -> Result<RuntimeRecord> {
        let _lock = self.lock().await?;
        let mut record = self.require(canonical_runtime(runtime_id))?;
        let before = (record.availability, record.last_error.clone());
        self.observe(&mut record);
        if before != (record.availability, record.last_error.clone()) {
            record.revision += 1;
            record.updated_at_ms = crate::now_ms();
        }
        self.write(&record)?;
        Ok(record)
    }

    async fn unregister(&self, runtime_id: &str, expected_revision: u64, _who: &str) -> Result<()> {
        let _lock = self.lock().await?;
        let record = self.require(canonical_runtime(runtime_id))?;
        if record.builtin {
            return Err(invalid("builtin", format!("{runtime_id} is built in and cannot be unregistered")));
        }
        if record.revision != expected_revision {
            return Err(invalid(
                "revision_conflict",
                format!("expected {expected_revision}, current {}", record.revision),
            ));
        }
        let path = self.layout.runtime_entry(&record.runtime_id);
        std::fs::remove_file(&path).map_err(|e| OpenDanError::io(&path, e))?;
        fsutil::fsync_dir(&self.layout.runtimes_dir())
    }

    async fn ensure_builtin(&self, builtin: &[RuntimeRegister], who: &str) -> Result<Vec<RuntimeRecord>> {
        let _lock = self.lock().await?;
        let mut out = Vec::new();
        for req in builtin {
            validate_id(&req.runtime_id)?;
            let record = match self.read(&req.runtime_id)? {
                // What the owner changed (name, description, roots) stays;
                // how the host reaches it is the host's.
                Some(mut r) => {
                    let before = r.clone();
                    r.kind = req.kind;
                    r.exec = req.exec.clone();
                    r.builtin = true;
                    if r.runtime_host != crate::runtime::native_host_id() && r.kind == RuntimeKind::Container {
                        r.runtime_host = crate::runtime::native_host_id();
                    }
                    self.observe(&mut r);
                    if (r.kind, &r.exec, r.builtin, &r.runtime_host, r.availability, &r.last_error)
                        != (before.kind, &before.exec, before.builtin, &before.runtime_host, before.availability, &before.last_error)
                    {
                        r.revision += 1;
                        r.updated_at_ms = crate::now_ms();
                    }
                    r
                }
                None => self.new_record(req, who, true),
            };
            self.write(&record)?;
            out.push(record);
        }
        Ok(out)
    }
}
