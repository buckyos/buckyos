//! Runtime registrations of an agent (`state/runtimes/<runtime_id>.json`):
//! the runtimes its sessions and workspaces may use, whether this host can
//! use them now, and what the agent may do there. Writes hold the
//! `runtimes` lock; updates are compare-and-set on `revision`.

use std::path::PathBuf;

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
        && id != LEGACY_LOCAL_RUNTIME
        && id != LEGACY_CONTAINER_RUNTIME;
    if !ok {
        return Err(invalid(
            "invalid_id",
            format!("`{id}` (lowercase letters, digits, `-`, `_`; `{LEGACY_CONTAINER_RUNTIME}` and `{LEGACY_LOCAL_RUNTIME}` are the private runtime's former names)"),
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
        // The private runtime first, then by id.
        out.sort_by(|a, b| b.is_private().cmp(&a.is_private()).then(a.runtime_id.cmp(&b.runtime_id)));
        Ok(out)
    }

    /// Whether this host can use the runtime now, from its record: the
    /// private runtime is executed directly and is always usable; a node
    /// runtime needs to be switched on and its grant in term — whether the
    /// node answers is learned by [`RuntimeRegistry::check`], which probes it.
    fn observe(&self, record: &mut RuntimeRecord) {
        let now = crate::now_ms();
        let (availability, error) = if record.lifecycle == RuntimeLifecycle::Retired {
            (RuntimeAvailability::RuntimeUnavailable, Some("retired: withdrawn, expired or unregistered; kept for its history".to_string()))
        } else if record.is_private() {
            if record.runtime_host != crate::runtime::native_host_id() {
                (RuntimeAvailability::RuntimeUnavailable, Some("registered by another host".to_string()))
            } else {
                (RuntimeAvailability::Available, None)
            }
        } else if !record.enabled {
            (
                RuntimeAvailability::RuntimeUnavailable,
                Some(format!("the `{}` runtime is not enabled for this agent", record.runtime_id)),
            )
        } else if record.grant.expired_at(now) {
            (
                RuntimeAvailability::RuntimeUnavailable,
                Some(format!(
                    "the grant on `{}` expired at {}; a renewed daemon URL or grant makes it usable again",
                    record.runtime_id,
                    record.grant.expires_at_ms.unwrap_or_default()
                )),
            )
        } else {
            match &record.connector {
                RuntimeConnector::Native => unreachable!("private handled above"),
                RuntimeConnector::NodeExecutor { node_did, executor_id, profile_id } => {
                    if node_did.as_deref().is_none_or(str::is_empty) && executor_id.as_deref().is_none_or(str::is_empty) {
                        (RuntimeAvailability::RuntimeUnavailable, Some(format!("no executor for the `{}` runtime yet", record.runtime_id)))
                    } else if node_did.is_some() && profile_id.is_empty() {
                        (RuntimeAvailability::RuntimeUnavailable, Some("a node runtime needs the agent's profile on the node".to_string()))
                    } else {
                        (RuntimeAvailability::Available, None)
                    }
                }
                RuntimeConnector::Ssh { .. } => (
                    RuntimeAvailability::RuntimeUnavailable,
                    Some("this runner cannot host sessions on ssh runtimes yet".to_string()),
                ),
            }
        };
        record.availability = availability;
        record.last_error = error;
        record.checked_at_ms = Some(now);
    }

    /// Ask a node runtime's executor whether it accepts this agent now:
    /// login + describe through the host's connector. The executor's roots
    /// (unless the owner narrowed them), ops and platform become the
    /// record's.
    async fn probe(&self, record: &mut RuntimeRecord) {
        if !record.connector.is_node() || !record.enabled {
            return;
        }
        let ops_dir = self.layout.runtimes_dir().join(".probe");
        match crate::runtime::open_node_target(record, &ops_dir).await {
            Ok(target) => {
                let roots: Vec<PathBuf> = target.workdir_roots().iter().map(PathBuf::from).collect();
                if record.grant.roots.is_empty() || record.grant.roots.iter().any(|r| !roots.contains(r)) {
                    record.grant.roots = roots;
                }
                record.grant.ops = target.caps().to_vec();
                let info = &target.described().info;
                record.caps.platform = RuntimePlatform::reported(&info.os, &info.arch, &info.shell);
                record.caps.identity_mode = info.identity_mode.clone();
                record.caps.buckyos_root = info.buckyos_root.clone();
                record.caps.desktop = info
                    .desktop
                    .as_ref()
                    .and_then(|d| d.pointer("/ui/supported"))
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if let Some(expires) = target.expires_at_ms() {
                    record.grant.expires_at_ms = Some(expires);
                }
                record.availability = RuntimeAvailability::Available;
                record.last_error = None;
            }
            Err(e) => {
                record.availability = RuntimeAvailability::RuntimeUnavailable;
                record.last_error = Some(e.to_string());
            }
        }
        record.checked_at_ms = Some(crate::now_ms());
    }

    fn new_record(&self, req: &RuntimeRegister, who: &str, builtin: bool) -> RuntimeRecord {
        let now = crate::now_ms();
        let mut record = RuntimeRecord {
            version: RUNTIME_RECORD_VERSION,
            runtime_id: req.runtime_id.clone(),
            name: req.name.clone(),
            desc: req.desc.clone(),
            runtime_host: crate::runtime::native_host_id(),
            connector: req.connector.clone(),
            enabled: req.enabled,
            caps: RuntimeCaps {
                platform: if req.connector.is_native() { RuntimePlatform::local() } else { RuntimePlatform::default() },
                baseline: req.baseline,
                ..Default::default()
            },
            grant: req.grant.clone(),
            availability: RuntimeAvailability::RuntimeUnavailable,
            builtin,
            default: req.default,
            lifecycle: RuntimeLifecycle::Active,
            usage: RuntimeUsage::default(),
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

    /// Reachability, the declared baseline's tools and `commands` of one
    /// record (`full`: also their versions); writes it when anything changed.
    async fn check_record(&self, mut record: RuntimeRecord, commands: &[String], full: bool) -> Result<RuntimeRecord> {
        let identity = |r: &RuntimeRecord| {
            (r.availability, r.last_error.clone(), r.grant.clone(), r.caps.clone())
        };
        let before = identity(&record);
        self.observe(&mut record);
        if record.availability == RuntimeAvailability::Available {
            self.probe(&mut record).await;
        }
        if record.availability == RuntimeAvailability::Available {
            let ops_dir = self.layout.runtimes_dir().join(".probe");
            let agent_root = self.layout.root.as_path();
            record.caps.baseline_check = Some(crate::runtime::check_baseline(&record, &ops_dir, Some(agent_root), full).await);
            if !commands.is_empty() {
                record.caps.tools_check = Some(crate::runtime::check_commands(&record, &ops_dir, commands, Some(agent_root), full).await);
            }
        }
        if before != identity(&record) {
            record.revision += 1;
            record.updated_at_ms = crate::now_ms();
        }
        self.write(&record)?;
        Ok(record)
    }
}

fn same_registration(r: &RuntimeRecord, req: &RuntimeRegister) -> bool {
    r.connector == req.connector
        && r.name == req.name
        && r.desc == req.desc
        && r.enabled == req.enabled
        && r.default == req.default
        && r.caps.baseline == req.baseline
        && r.grant == req.grant
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
        if req.connector.is_native() {
            return Err(invalid("invalid_registration", "the private runtime is registered by the host"));
        }
        if req.grant.roots.is_empty() {
            return Err(invalid("invalid_registration", "a registered runtime needs at least one --root: the directories the agent may use there"));
        }
        if let Some(bad) = req.grant.roots.iter().find(|p| !buckyos_api::is_absolute_node_path(&p.display().to_string())) {
            return Err(invalid("invalid_root", format!("{} is not absolute", bad.display())));
        }
        let record = {
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
            record
        };
        // Checked at once (R-C2): the record says whether it answers.
        let _lock = self.lock().await?;
        self.check_record(record, &[], false).await
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
        if let Some(d) = &req.desc {
            record.desc = d.clone();
        }
        if let Some(r) = &req.roots {
            if let Some(bad) = r.iter().find(|p| !buckyos_api::is_absolute_node_path(&p.display().to_string())) {
                return Err(invalid("invalid_root", format!("{} is not absolute", bad.display())));
            }
            record.grant.roots = r.clone();
        }
        record.revision += 1;
        record.updated_at_ms = crate::now_ms();
        self.write(&record)?;
        Ok(record)
    }

    async fn check(&self, runtime_id: &str, commands: &[String], full: bool) -> Result<RuntimeRecord> {
        let _lock = self.lock().await?;
        let record = self.require(canonical_runtime(runtime_id))?;
        self.check_record(record, commands, full).await
    }

    async fn unregister(&self, runtime_id: &str, expected_revision: u64, who: &str) -> Result<RuntimeRecord> {
        let _lock = self.lock().await?;
        let mut record = self.require(canonical_runtime(runtime_id))?;
        if record.builtin {
            return Err(invalid("builtin", format!("{runtime_id} is built in and cannot be unregistered; disable it in the agent's settings")));
        }
        if record.revision != expected_revision {
            return Err(invalid(
                "revision_conflict",
                format!("expected {expected_revision}, current {}", record.revision),
            ));
        }
        if record.lifecycle == RuntimeLifecycle::Retired {
            return Ok(record);
        }
        record.lifecycle = RuntimeLifecycle::Retired;
        record.default = false;
        record.last_error = Some(format!("retired by {who}"));
        self.observe(&mut record);
        record.revision += 1;
        record.updated_at_ms = crate::now_ms();
        self.write(&record)?;
        Ok(record)
    }

    async fn note_usage(&self, runtime_id: &str, event: &RuntimeUsageEvent) -> Result<()> {
        let _lock = self.lock().await?;
        let Some(mut record) = self.read(canonical_runtime(runtime_id))? else {
            return Ok(());
        };
        let now = crate::now_ms();
        match event {
            RuntimeUsageEvent::Session { session_id } => {
                record.usage.sessions_total += 1;
                record.usage.last_used_at_ms = Some(now);
                record.usage.last_session_id = Some(session_id.clone());
            }
            RuntimeUsageEvent::Workspace => {
                record.usage.workspaces += 1;
                record.usage.last_used_at_ms = Some(now);
            }
            RuntimeUsageEvent::SkillCommand { name } => {
                if !record.usage.skill_commands.contains(name) {
                    record.usage.skill_commands.push(name.clone());
                }
                record.usage.last_used_at_ms = Some(now);
            }
        }
        self.write(&record)
    }

    async fn ensure_builtin(&self, builtin: &[RuntimeRegister], who: &str) -> Result<Vec<RuntimeRecord>> {
        let _lock = self.lock().await?;
        let mut out = Vec::new();
        for req in builtin {
            validate_id(&req.runtime_id)?;
            // Built-in records are the host's projection: one this version
            // cannot read (an older record version) is written anew.
            let existing = match self.read(&req.runtime_id) {
                Ok(r) => r,
                Err(e) => {
                    log::warn!("runtime registration {}: {e}; rewritten by the host", req.runtime_id);
                    None
                }
            };
            let record = match existing {
                // What the owner changed of the private runtime (name, desc)
                // stays; how the host reaches a runtime, whether it is on,
                // the default and what the agent may do there are the
                // system's. Roots, ops and platform a node reported stay
                // unless its connector changed.
                Some(mut r) => {
                    let before = r.clone();
                    let reconnected = r.connector != req.connector;
                    r.connector = req.connector.clone();
                    r.enabled = req.enabled;
                    r.builtin = true;
                    r.default = req.default;
                    r.lifecycle = RuntimeLifecycle::Active;
                    r.caps.baseline = req.baseline;
                    let learned_roots = std::mem::take(&mut r.grant.roots);
                    let learned_ops = std::mem::take(&mut r.grant.ops);
                    r.grant = req.grant.clone();
                    if !reconnected && !r.is_private() {
                        if r.grant.roots.is_empty() {
                            r.grant.roots = learned_roots;
                        }
                        r.grant.ops = learned_ops;
                    }
                    if reconnected {
                        r.caps = RuntimeCaps { baseline: req.baseline, ..Default::default() };
                    }
                    if r.is_private() {
                        r.runtime_host = crate::runtime::native_host_id();
                        r.caps.platform = RuntimePlatform::local();
                        r.desc = req.desc.clone();
                    } else {
                        r.name = req.name.clone();
                        r.desc = req.desc.clone();
                    }
                    self.observe(&mut r);
                    let changed = {
                        let mut probe = r.clone();
                        probe.checked_at_ms = before.checked_at_ms;
                        probe != before
                    };
                    if changed {
                        r.revision = before.revision + 1;
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
