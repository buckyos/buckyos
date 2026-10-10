use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::error::{OpenDanError, Result};
use crate::fsutil;
use crate::lock::FileLock;
use crate::protocol::*;

use super::{AgentLayout, WorkspaceManager};

pub(super) struct FsWorkspaces {
    layout: AgentLayout,
    agent_did: String,
}

fn invalid(code: &str, message: impl std::fmt::Display) -> OpenDanError {
    OpenDanError::InvalidArgument(format!("workspace {code}: {message}"))
}

pub(super) fn validate_workspace_id(id: &str) -> Result<()> {
    let suffix = id
        .strip_prefix("ws-")
        .ok_or_else(|| invalid("invalid_id", id))?;
    let uuid = uuid::Uuid::parse_str(suffix).map_err(|_| invalid("invalid_id", id))?;
    if uuid.to_string() != suffix {
        return Err(invalid("invalid_id", id));
    }
    Ok(())
}

fn validate_metadata(metadata: &WorkspaceMetadata) -> Result<()> {
    if metadata.version != WORKSPACE_METADATA_VERSION {
        return Err(invalid("unknown_metadata_version", metadata.version));
    }
    validate_workspace_id(&metadata.workspace_id)?;
    if metadata.name.trim().is_empty()
        || metadata.name.len() > 512
        || metadata.description.len() > 32768
    {
        return Err(invalid("invalid_metadata", "invalid name or description"));
    }
    Ok(())
}

fn read_metadata(directory: &Path) -> Result<Option<WorkspaceMetadata>> {
    let path = directory.join(WORKSPACE_METADATA_FILE);
    let meta = match std::fs::symlink_metadata(&path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(OpenDanError::io(&path, e)),
    };
    if !meta.is_file() || meta.file_type().is_symlink() || meta.len() > 65536 {
        return Err(invalid(
            "invalid_metadata",
            "identity metadata must be a regular file of at most 64 KiB",
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options
        .open(&path)
        .map_err(|e| OpenDanError::io(&path, e))?;
    if !file
        .metadata()
        .map_err(|e| OpenDanError::io(&path, e))?
        .is_file()
    {
        return Err(invalid(
            "invalid_metadata",
            "identity metadata is not a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take(65537)
        .read_to_end(&mut bytes)
        .map_err(|e| OpenDanError::io(&path, e))?;
    if bytes.len() > 65536 {
        return Err(invalid(
            "invalid_metadata",
            "identity metadata is too large",
        ));
    }
    let metadata: WorkspaceMetadata =
        serde_json::from_slice(&bytes).map_err(|e| invalid("invalid_metadata", e))?;
    validate_metadata(&metadata)?;
    Ok(Some(metadata))
}

fn resolve_directory(path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid("invalid_directory", "directory must be absolute"));
    }
    let resolved = std::fs::canonicalize(path).map_err(|e| OpenDanError::io(path, e))?;
    if !resolved.is_dir() {
        return Err(invalid(
            "invalid_directory",
            "workspace location is not a directory",
        ));
    }
    std::fs::read_dir(&resolved).map_err(|e| OpenDanError::io(&resolved, e))?;
    Ok(resolved)
}

impl FsWorkspaces {
    pub(super) fn new(layout: AgentLayout, agent_did: &str) -> Self {
        Self {
            layout,
            agent_did: agent_did.to_string(),
        }
    }

    fn lock(&self) -> Result<FileLock> {
        FileLock::try_acquire(&self.layout.lock_path("workspaces")?)?.ok_or_else(|| {
            OpenDanError::Busy {
                resource: "workspaces".into(),
                holder: None,
            }
        })
    }

    async fn lock_for_check(&self) -> Result<FileLock> {
        for _ in 0..50 {
            match self.lock() {
                Ok(lock) => return Ok(lock),
                Err(OpenDanError::Busy { .. }) => {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                Err(error) => return Err(error),
            }
        }
        self.lock()
    }

    fn runtime_available(&self, runtime_id: &str) -> Result<bool> {
        if runtime_id != LOCAL_WORKSPACE_RUNTIME {
            return Ok(false);
        }
        Ok(!self
            .layout
            .workspaces_dir()
            .join(".local-unavailable.json")
            .exists())
    }

    fn location(&self, location: &WorkspaceLocation) -> Result<WorkspaceLocation> {
        if !self.runtime_available(&location.runtime_id)? {
            return Err(invalid("runtime_unavailable", &location.runtime_id));
        }
        Ok(WorkspaceLocation {
            runtime_id: location.runtime_id.clone(),
            directory: resolve_directory(&location.directory)?,
        })
    }

    fn read(&self, id: &str) -> Result<Option<WorkspaceRecord>> {
        validate_workspace_id(id)?;
        let record = fsutil::read_json_opt::<WorkspaceRecord>(&self.layout.workspace_entry(id))?;
        if record
            .as_ref()
            .is_some_and(|r| r.workspace_id != id || r.version != WORKSPACE_RECORD_VERSION)
        {
            return Err(invalid("invalid_registration", id));
        }
        Ok(record)
    }

    fn require(&self, id: &str) -> Result<WorkspaceRecord> {
        self.read(id)?
            .ok_or_else(|| OpenDanError::NotFound(format!("workspace {id}")))
    }

    fn write(&self, record: &WorkspaceRecord) -> Result<()> {
        fsutil::atomic_replace_json(&self.layout.workspace_entry(&record.workspace_id), record)
    }

    fn all(&self) -> Result<Vec<WorkspaceRecord>> {
        let path = self.layout.workspaces_dir();
        let mut records = Vec::new();
        for entry in std::fs::read_dir(&path).map_err(|e| OpenDanError::io(&path, e))? {
            let entry = entry.map_err(|e| OpenDanError::io(&path, e))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if let Some(id) = name.strip_suffix(".json") {
                if let Some(record) = self.read(id)? {
                    records.push(record);
                }
            }
        }
        records.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));
        Ok(records)
    }

    fn check_revision(&self, record: &WorkspaceRecord, expected: u64) -> Result<()> {
        if record.revision != expected {
            return Err(invalid(
                "revision_conflict",
                format!("expected {expected}, current {}", record.revision),
            ));
        }
        Ok(())
    }

    fn touch(record: &mut WorkspaceRecord) {
        record.revision += 1;
        record.updated_at_ms = crate::now_ms();
    }

    fn operation_id(&self, operation_id: &str, who: &str) -> Result<String> {
        if operation_id.trim().is_empty() || operation_id.len() > 256 || who.trim().is_empty() {
            return Err(invalid(
                "invalid_operation",
                "operation_id and caller are required",
            ));
        }
        let digest =
            Sha256::digest(serde_json::to_vec(&(&self.agent_did, who, operation_id)).unwrap());
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&digest[..16]);
        bytes[6] = (bytes[6] & 0x0f) | 0x40;
        bytes[8] = (bytes[8] & 0x3f) | 0x80;
        Ok(format!("ws-{}", uuid::Uuid::from_bytes(bytes)))
    }

    fn no_overlap(&self, id: &str, location: &WorkspaceLocation) -> Result<()> {
        for existing in self.all()? {
            if existing.workspace_id != id
                && existing.location.runtime_id == location.runtime_id
                && (existing.location.directory.starts_with(&location.directory)
                    || location.directory.starts_with(&existing.location.directory))
            {
                return Err(invalid(
                    "overlapping_directory",
                    format!("overlaps workspace {}", existing.workspace_id),
                ));
            }
        }
        Ok(())
    }

    fn active_sessions(&self, id: &str) -> Result<Vec<String>> {
        let mut sessions = Vec::new();
        let path = self.layout.sessions_dir();
        for entry in std::fs::read_dir(&path).map_err(|e| OpenDanError::io(&path, e))? {
            let entry = entry.map_err(|e| OpenDanError::io(&path, e))?;
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let session: RegistryEntry = fsutil::read_json(&entry.path())?;
            if !session
                .workspace
                .as_ref()
                .is_some_and(|w| w.workspace_id == id)
            {
                continue;
            }
            if session.status.run_state != RunState::Finished {
                sessions.push(session.session_id);
                continue;
            }
            let directory = crate::session::SessionDir::open(&session.location).map_err(|e| {
                invalid("unverified_session", format!("{}: {e}", session.session_id))
            })?;
            let state = directory.state()?;
            let mut unresolved =
                !state.watched_tasks.is_empty() || state.run_state != RunState::Finished;
            let runs = directory.runs();
            for run_id in runs.list()? {
                let run = runs.record(&run_id)?;
                unresolved |= run.status == agent_tool::xllm::RunStatus::Running
                    || !run.inflight.is_empty()
                    || run
                        .host
                        .as_ref()
                        .and_then(|h| h.extra.get("tasks"))
                        .and_then(|tasks| tasks.as_array())
                        .is_some_and(|tasks| !tasks.is_empty());
            }
            if unresolved {
                sessions.push(session.session_id);
            }
        }
        Ok(sessions)
    }

    fn idle(&self, id: &str) -> Result<()> {
        let sessions = self.active_sessions(id)?;
        if !sessions.is_empty()
            || FileLock::try_acquire(&self.layout.lock_path(&format!("workspace:{id}"))?)?.is_none()
        {
            return Err(invalid(
                "active_sessions",
                format!("workspace {id} is occupied: {}", sessions.join(", ")),
            ));
        }
        Ok(())
    }

    fn conflict(
        &self,
        record: &mut WorkspaceRecord,
        candidate: &WorkspaceLocation,
        code: &str,
        message: impl Into<String>,
    ) -> Result<WorkspaceRecord> {
        let message = message.into();
        record.conflict = Some(WorkspaceConflict {
            code: code.into(),
            message: message.clone(),
            candidate: candidate.clone(),
            at_ms: crate::now_ms(),
        });
        record.availability = WorkspaceAvailability::Conflict;
        record.last_error = Some(message.clone());
        Self::touch(record);
        self.write(record)?;
        Err(invalid(code, message))
    }

    fn register_metadata(
        &self,
        metadata: &WorkspaceMetadata,
        location: WorkspaceLocation,
        expected_revision: Option<u64>,
        who: &str,
        source: &str,
        usage: WorkspaceUsage,
        source_session: Option<String>,
        policy_ref: Option<String>,
    ) -> Result<WorkspaceRecord> {
        self.no_overlap(&metadata.workspace_id, &location)?;
        if let Some(mut record) = self.read(&metadata.workspace_id)? {
            if let Some(expected) = expected_revision {
                self.check_revision(&record, expected)?;
            }
            if record.runtime_host != crate::runtime::native_host_id() {
                return self.conflict(&mut record, &location, "old_runtime_unavailable", "the original local runtime belongs to another host and cannot be verified here");
            }
            if record.location == location {
                return Ok(record);
            }
            let expected = expected_revision.ok_or_else(|| {
                invalid("revision_required", "relocation requires expected_revision")
            })?;
            self.check_revision(&record, expected)?;
            if !self.runtime_available(&record.location.runtime_id)? {
                return self.conflict(
                    &mut record,
                    &location,
                    "old_runtime_unavailable",
                    "cannot prove relocation while the original runtime is unavailable",
                );
            }
            if let Err(e) = self.idle(&record.workspace_id) {
                return self.conflict(&mut record, &location, "active_sessions", e.to_string());
            }
            match std::fs::symlink_metadata(&record.location.directory) {
                Ok(_) => return self.conflict(
                    &mut record,
                    &location,
                    "duplicate_identity",
                    "the original directory still exists; copies require a new workspace identity",
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => {
                    return self.conflict(
                        &mut record,
                        &location,
                        "old_location_unverified",
                        e.to_string(),
                    )
                }
            }
            record.location = location;
            record.location_revision += 1;
            record.name = metadata.name.clone();
            record.description = metadata.description.clone();
            record.availability = WorkspaceAvailability::Available;
            record.conflict = None;
            record.last_error = None;
            record.checked_at_ms = Some(crate::now_ms());
            Self::touch(&mut record);
            self.write(&record)?;
            return Ok(record);
        }
        let now = crate::now_ms();
        let record = WorkspaceRecord {
            version: WORKSPACE_RECORD_VERSION,
            runtime_host: crate::runtime::native_host_id(),
            workspace_id: metadata.workspace_id.clone(),
            name: metadata.name.clone(),
            description: metadata.description.clone(),
            location,
            usage,
            lifecycle: WorkspaceLifecycle::Active,
            availability: WorkspaceAvailability::Available,
            revision: 1,
            location_revision: 1,
            created_at_ms: metadata.created_at_ms,
            registered_at_ms: now,
            updated_at_ms: now,
            checked_at_ms: Some(now),
            registered_by: who.into(),
            source: source.into(),
            private_notes: String::new(),
            source_session,
            policy_ref,
            conflict: None,
            last_error: None,
        };
        self.write(&record)?;
        Ok(record)
    }

    fn initialize_metadata(
        &self,
        directory: &Path,
        metadata: &WorkspaceMetadata,
    ) -> Result<WorkspaceMetadata> {
        validate_metadata(metadata)?;
        let path = directory.join(WORKSPACE_METADATA_FILE);
        if fsutil::publish_noreplace_json(&path, metadata)? {
            return Ok(metadata.clone());
        }
        read_metadata(directory)?.ok_or_else(|| {
            invalid(
                "metadata_changed",
                "metadata disappeared during initialization",
            )
        })
    }

    fn conflict_resolved(&self, record: &WorkspaceRecord) -> bool {
        let Some(conflict) = &record.conflict else {
            return false;
        };
        if conflict.candidate.runtime_id != LOCAL_WORKSPACE_RUNTIME
            || conflict.candidate == record.location
        {
            return false;
        }
        match std::fs::symlink_metadata(&conflict.candidate.directory) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
            Ok(_) => read_metadata(&conflict.candidate.directory)
                .ok()
                .flatten()
                .is_some_and(|metadata| metadata.workspace_id != record.workspace_id),
            Err(_) => false,
        }
    }

    fn observe(&self, record: &mut WorkspaceRecord) -> Result<()> {
        let observed = if record.runtime_host != crate::runtime::native_host_id()
            || !self.runtime_available(&record.location.runtime_id)?
        {
            Err(invalid("runtime_unavailable", &record.location.runtime_id))
        } else {
            self.location(&record.location).and_then(|actual| {
                if actual != record.location {
                    return Err(invalid(
                        "location_changed",
                        "directory resolves to a different location",
                    ));
                }
                let metadata = read_metadata(&actual.directory)?
                    .ok_or_else(|| invalid("missing_metadata", "identity metadata is missing"))?;
                if metadata.workspace_id != record.workspace_id {
                    return Err(invalid(
                        "identity_changed",
                        "directory identity differs from registration",
                    ));
                }
                Ok(metadata)
            })
        };
        match observed {
            Ok(metadata) => {
                if self.conflict_resolved(record) {
                    record.conflict = None;
                }
                record.name = metadata.name;
                record.description = metadata.description;
                record.availability = if record.conflict.is_some() {
                    WorkspaceAvailability::Conflict
                } else {
                    WorkspaceAvailability::Available
                };
                if record.conflict.is_none() {
                    record.last_error = None;
                }
            }
            Err(e) => {
                record.availability = match &e {
                    OpenDanError::Io { source, .. }
                        if source.kind() == std::io::ErrorKind::NotFound =>
                    {
                        WorkspaceAvailability::Missing
                    }
                    OpenDanError::Io { source, .. }
                        if source.kind() == std::io::ErrorKind::PermissionDenied =>
                    {
                        WorkspaceAvailability::PermissionDenied
                    }
                    _ if record.runtime_host != crate::runtime::native_host_id()
                        || !self.runtime_available(&record.location.runtime_id)? =>
                    {
                        WorkspaceAvailability::RuntimeUnavailable
                    }
                    _ => WorkspaceAvailability::InvalidMetadata,
                };
                record.last_error = Some(e.to_string());
            }
        }
        record.checked_at_ms = Some(crate::now_ms());
        Self::touch(record);
        self.write(record)
    }
}

#[async_trait]
impl WorkspaceManager for FsWorkspaces {
    async fn create(&self, request: &WorkspaceCreate, who: &str) -> Result<WorkspaceRecord> {
        let _lock = self.lock()?;
        let id = self.operation_id(&request.operation_id, who)?;
        if !self.runtime_available(&request.location.runtime_id)? {
            return Err(invalid("runtime_unavailable", &request.location.runtime_id));
        }
        if !request.location.directory.is_absolute() {
            return Err(invalid("invalid_directory", "directory must be absolute"));
        }
        let draft = WorkspaceMetadata {
            version: WORKSPACE_METADATA_VERSION,
            workspace_id: id.clone(),
            name: request.name.clone(),
            description: request.description.clone(),
            created_at_ms: crate::now_ms(),
            operation_id: Some(request.operation_id.clone()),
        };
        validate_metadata(&draft)?;
        if let Some(existing) = self.read(&id)? {
            if existing.runtime_host != crate::runtime::native_host_id() {
                return Err(invalid(
                    "old_runtime_unavailable",
                    "the existing operation belongs to another local runtime host",
                ));
            }
            let directory = std::fs::canonicalize(&request.location.directory)
                .unwrap_or_else(|_| request.location.directory.clone());
            if existing.location.runtime_id != request.location.runtime_id
                || existing.location.directory != directory
            {
                return Err(invalid(
                    "operation_reused",
                    "this operation already created a workspace at another location",
                ));
            }
        }
        if let Some(existing) = self.read(&id)? {
            if existing.name != request.name
                || existing.description != request.description
                || existing.usage != request.usage
                || existing.source_session != request.source_session
                || existing.policy_ref != request.policy_ref
            {
                return Err(invalid(
                    "operation_conflict",
                    "operation parameters differ from the existing workspace",
                ));
            }
            let location = self.location(&request.location)?;
            let metadata = read_metadata(&location.directory)?.ok_or_else(|| {
                invalid(
                    "missing_metadata",
                    "an existing registration cannot be reinitialized",
                )
            })?;
            if metadata.workspace_id != id
                || metadata.operation_id.as_deref() != Some(&request.operation_id)
            {
                return Err(invalid(
                    "identity_changed",
                    "an existing registration has different directory metadata",
                ));
            }
            if metadata.name != request.name || metadata.description != request.description {
                return Err(invalid(
                    "operation_conflict",
                    "operation parameters differ from the directory metadata",
                ));
            }
            return Ok(existing);
        }
        if !request.location.directory.exists() {
            let parent = request
                .location
                .directory
                .parent()
                .ok_or_else(|| invalid("invalid_directory", "missing parent directory"))?;
            let parent = resolve_directory(parent)?;
            let target = parent.join(
                request
                    .location
                    .directory
                    .file_name()
                    .ok_or_else(|| invalid("invalid_directory", "missing directory name"))?,
            );
            self.no_overlap(
                &id,
                &WorkspaceLocation {
                    runtime_id: request.location.runtime_id.clone(),
                    directory: target.clone(),
                },
            )?;
            std::fs::create_dir(&target).map_err(|e| OpenDanError::io(&target, e))?;
        }
        let location = self.location(&request.location)?;
        self.no_overlap(&id, &location)?;
        let metadata = match read_metadata(&location.directory)? {
            Some(metadata) => {
                if metadata.workspace_id != id
                    || metadata.operation_id.as_deref() != Some(&request.operation_id)
                {
                    return Err(invalid(
                        "already_initialized",
                        "directory already has another workspace; import it explicitly",
                    ));
                }
                metadata
            }
            None => {
                if std::fs::read_dir(&location.directory)
                    .map_err(|e| OpenDanError::io(&location.directory, e))?
                    .next()
                    .is_some()
                {
                    return Err(invalid(
                        "directory_not_empty",
                        "create requires an empty directory; use import for existing content",
                    ));
                }
                self.initialize_metadata(&location.directory, &draft)?
            }
        };
        if metadata.name != request.name || metadata.description != request.description {
            return Err(invalid(
                "operation_conflict",
                "operation parameters differ from the directory metadata",
            ));
        }
        if metadata.workspace_id != id {
            return Err(invalid(
                "identity_conflict",
                "another operation initialized this directory",
            ));
        }
        self.register_metadata(
            &metadata,
            location,
            None,
            who,
            "created",
            request.usage,
            request.source_session.clone(),
            request.policy_ref.clone(),
        )
    }

    async fn import(&self, request: &WorkspaceImport, who: &str) -> Result<WorkspaceRecord> {
        let _lock = self.lock()?;
        let id = self.operation_id(&request.operation_id, who)?;
        let location = self.location(&request.location)?;
        let metadata = match read_metadata(&location.directory)? {
            Some(metadata) => metadata,
            None => {
                if self.read(&id)?.is_some() {
                    return Err(invalid(
                        "operation_reused",
                        "this operation already initialized a different directory",
                    ));
                }
                self.no_overlap(&id, &location)?;
                let name = request.name.clone().unwrap_or_else(|| {
                    location
                        .directory
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string()
                });
                self.initialize_metadata(
                    &location.directory,
                    &WorkspaceMetadata {
                        version: WORKSPACE_METADATA_VERSION,
                        workspace_id: id,
                        name,
                        description: request.description.clone(),
                        created_at_ms: crate::now_ms(),
                        operation_id: Some(request.operation_id.clone()),
                    },
                )?
            }
        };
        self.register_metadata(
            &metadata,
            location,
            request.expected_revision,
            who,
            "imported",
            request.usage,
            request.source_session.clone(),
            request.policy_ref.clone(),
        )
    }

    async fn discover(&self, request: &WorkspaceDiscover, who: &str) -> Result<WorkspaceRecord> {
        let _lock = self.lock()?;
        let location = self.location(&request.location)?;
        let metadata = read_metadata(&location.directory)?.ok_or_else(|| {
            invalid(
                "missing_metadata",
                "ordinary directory; explicit import is required",
            )
        })?;
        if request
            .expected_workspace_id
            .as_ref()
            .is_some_and(|id| id != &metadata.workspace_id)
        {
            return Err(invalid(
                "identity_mismatch",
                "directory does not contain the expected workspace",
            ));
        }
        self.register_metadata(
            &metadata,
            location,
            request.expected_revision,
            who,
            "discovered",
            WorkspaceUsage::Collaborative,
            None,
            None,
        )
    }

    async fn lookup(&self, workspace_id: &str) -> Result<Option<WorkspaceRecord>> {
        self.read(workspace_id)
    }

    async fn query(&self, query: &WorkspaceQuery) -> Result<Vec<WorkspaceRecord>> {
        Ok(self
            .all()?
            .into_iter()
            .filter(|r| {
                query
                    .runtime_id
                    .as_ref()
                    .is_none_or(|v| v == &r.location.runtime_id)
                    && query.lifecycle.is_none_or(|v| v == r.lifecycle)
                    && query.availability.is_none_or(|v| v == r.availability)
                    && query.text.as_ref().is_none_or(|v| {
                        let v = v.to_lowercase();
                        r.name.to_lowercase().contains(&v)
                            || r.description.to_lowercase().contains(&v)
                            || r.workspace_id.contains(&v)
                    })
            })
            .collect())
    }

    async fn check(&self, workspace_id: &str) -> Result<WorkspaceRecord> {
        let _lock = self.lock_for_check().await?;
        let mut record = self.require(workspace_id)?;
        self.observe(&mut record)?;
        Ok(record)
    }

    async fn update(
        &self,
        workspace_id: &str,
        request: &WorkspaceUpdate,
        _who: &str,
    ) -> Result<WorkspaceRecord> {
        let _lock = self.lock()?;
        let mut record = self.require(workspace_id)?;
        self.check_revision(&record, request.expected_revision)?;
        if let Some(lifecycle) = request.lifecycle {
            if lifecycle == WorkspaceLifecycle::Archived && record.lifecycle != lifecycle {
                self.idle(workspace_id)?;
            }
            record.lifecycle = lifecycle;
        }
        if let Some(notes) = &request.private_notes {
            record.private_notes = notes.clone();
        }
        Self::touch(&mut record);
        self.write(&record)?;
        Ok(record)
    }

    async fn unregister(
        &self,
        workspace_id: &str,
        expected_revision: u64,
        _who: &str,
    ) -> Result<()> {
        let _lock = self.lock()?;
        let record = self.require(workspace_id)?;
        self.check_revision(&record, expected_revision)?;
        self.idle(workspace_id)?;
        let path = self.layout.workspace_entry(workspace_id);
        std::fs::remove_file(&path).map_err(|e| OpenDanError::io(&path, e))?;
        fsutil::fsync_dir(&self.layout.workspaces_dir())
    }

    async fn runtime_impact(&self, runtime_id: &str) -> Result<Vec<WorkspaceRecord>> {
        self.query(&WorkspaceQuery {
            runtime_id: Some(runtime_id.into()),
            ..Default::default()
        })
        .await
    }

    async fn set_runtime_available(
        &self,
        runtime_id: &str,
        available: bool,
        _who: &str,
    ) -> Result<Vec<WorkspaceRecord>> {
        let _lock = self.lock()?;
        if runtime_id != LOCAL_WORKSPACE_RUNTIME {
            return Err(invalid("unsupported_runtime", runtime_id));
        }
        let marker = self.layout.workspaces_dir().join(".local-unavailable.json");
        if available {
            match std::fs::remove_file(&marker) {
                Ok(()) => {
                    fsutil::fsync_dir(&self.layout.workspaces_dir())?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(OpenDanError::io(&marker, e)),
            }
        } else {
            fsutil::atomic_replace_json(
                &marker,
                &serde_json::json!({ "version": 1, "at_ms": crate::now_ms() }),
            )?;
        }
        let mut records = self.all()?;
        for record in &mut records {
            if record.location.runtime_id == runtime_id {
                self.observe(record)?;
            }
        }
        Ok(records
            .into_iter()
            .filter(|r| r.location.runtime_id == runtime_id)
            .collect())
    }
}
