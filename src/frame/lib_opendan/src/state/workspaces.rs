//! Workspaces of an agent (`state/workspaces/<id>.json`): the control side
//! of a workspace — registration, identity, URL, revisions, the exclusive
//! lease and the relocation rules — lives here, in the Agent State, for
//! every runtime alike (integration TODO §3.4). The runtime only holds the
//! directory: whether it exists and what its canonical form is, the
//! identity metadata inside it, and creating it. In the agent's own
//! environment those are local file operations; on a node runtime they are
//! the executor's `describe`, `stat`, `read`, `write` and `mkdir`.

use std::fs::OpenOptions;
use std::io::Read;
use std::path::{Path, PathBuf};

use agent_tool::runtime::files::PathType;
use agent_tool::runtime::node::RemoteNodeTarget;
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
    if let Some(url) = &metadata.url {
        validate_workspace_url(url).map_err(|e| invalid("invalid_url", e))?;
    }
    Ok(())
}

/// A URL given with a request: trimmed, validated, empty means none.
fn request_url(url: &Option<String>) -> Result<Option<String>> {
    match url.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(url) => validate_workspace_url(url)
            .map(Some)
            .map_err(|e| invalid("invalid_url", e)),
    }
}

fn parse_metadata(bytes: &[u8]) -> Result<WorkspaceMetadata> {
    if bytes.len() > 65536 {
        return Err(invalid("invalid_metadata", "identity metadata is too large"));
    }
    let metadata: WorkspaceMetadata =
        serde_json::from_slice(bytes).map_err(|e| invalid("invalid_metadata", e))?;
    validate_metadata(&metadata)?;
    Ok(metadata)
}

fn read_local_metadata(directory: &Path) -> Result<Option<WorkspaceMetadata>> {
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
    parse_metadata(&bytes).map(Some)
}

fn resolve_local_directory(path: &Path) -> Result<PathBuf> {
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

fn is_drive_path(path: &str) -> bool {
    let b = path.as_bytes();
    b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/')
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Whether `child` is `parent` or below it, with the path rules of the
/// directory's own OS (a drive path compares case-insensitively on `\`).
fn dir_contains(parent: &Path, child: &Path) -> bool {
    let (p, c) = (path_text(parent), path_text(child));
    if is_drive_path(&p) || is_drive_path(&c) {
        let norm = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_ascii_lowercase();
        let (p, c) = (norm(&p), norm(&c));
        return c == p || c.starts_with(&format!("{p}\\"));
    }
    child.starts_with(parent)
}

/// The metadata file inside a workspace directory, joined with the
/// directory's own separator.
fn metadata_path(directory: &Path) -> String {
    crate::runtime::target_join(&path_text(directory), &[WORKSPACE_METADATA_FILE])
}

/// Where a workspace directory is: the agent's own environment, or a node
/// reached through its executor.
enum Target {
    Local,
    Node(RemoteNodeTarget),
}

fn target_error(what: &str, e: agent_tool::AgentToolError) -> OpenDanError {
    match e {
        agent_tool::AgentToolError::InvalidArgs(m) => invalid(what, m),
        other => OpenDanError::Other(format!("workspace {what}: {other}")),
    }
}

impl Target {
    async fn canonical_dir(&self, path: &Path) -> Result<PathBuf> {
        match self {
            Target::Local => resolve_local_directory(path),
            Target::Node(t) => {
                let text = path_text(path);
                if !buckyos_api::is_absolute_node_path(&text) {
                    return Err(invalid("invalid_directory", "directory must be absolute on the node"));
                }
                t.canonical_dir(&text)
                    .await
                    .map(PathBuf::from)
                    .map_err(|e| target_error("invalid_directory", e))
            }
        }
    }

    /// `Missing` only when nothing is at the path; a dangling symlink or a
    /// file counts as present (a relocation needs the old place gone).
    async fn path_type(&self, path: &Path) -> Result<PathType> {
        match self {
            Target::Local => match std::fs::symlink_metadata(path) {
                Ok(m) if m.is_dir() => Ok(PathType::Directory),
                Ok(m) if m.is_file() => Ok(PathType::File),
                Ok(_) => Ok(PathType::Other),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(PathType::Missing),
                Err(e) => Err(OpenDanError::io(path, e)),
            },
            Target::Node(t) => t
                .path_type(&path_text(path))
                .await
                .map(|(kind, _)| kind)
                .map_err(|e| target_error("old_location_unverified", e)),
        }
    }

    async fn read_metadata(&self, directory: &Path) -> Result<Option<WorkspaceMetadata>> {
        match self {
            Target::Local => read_local_metadata(directory),
            Target::Node(t) => {
                let path = metadata_path(directory);
                let (kind, _) = t
                    .path_type(&path)
                    .await
                    .map_err(|e| target_error("invalid_metadata", e))?;
                match kind {
                    PathType::Missing => Ok(None),
                    PathType::File => {
                        let bytes = t.read(&path).await.map_err(|e| target_error("invalid_metadata", e))?;
                        parse_metadata(&bytes).map(Some)
                    }
                    _ => Err(invalid("invalid_metadata", "identity metadata is not a regular file")),
                }
            }
        }
    }

    /// Write the identity metadata unless the file exists (`false`).
    async fn write_metadata_new(&self, directory: &Path, metadata: &WorkspaceMetadata) -> Result<bool> {
        match self {
            Target::Local => fsutil::publish_noreplace_json(&directory.join(WORKSPACE_METADATA_FILE), metadata),
            Target::Node(t) => {
                let bytes = serde_json::to_vec_pretty(metadata).map_err(|e| OpenDanError::Other(e.to_string()))?;
                t.write_new(&metadata_path(directory), &bytes)
                    .await
                    .map_err(|e| target_error("metadata_write_failed", e))
            }
        }
    }

    async fn write_metadata(&self, directory: &Path, metadata: &WorkspaceMetadata) -> Result<()> {
        match self {
            Target::Local => fsutil::atomic_replace_json(&directory.join(WORKSPACE_METADATA_FILE), metadata),
            Target::Node(t) => {
                let bytes = serde_json::to_vec_pretty(metadata).map_err(|e| OpenDanError::Other(e.to_string()))?;
                t.write(&metadata_path(directory), &bytes)
                    .await
                    .map_err(|e| target_error("metadata_write_failed", e))
            }
        }
    }

    async fn mkdir(&self, path: &Path) -> Result<PathBuf> {
        match self {
            Target::Local => {
                std::fs::create_dir(path).map_err(|e| OpenDanError::io(path, e))?;
                Ok(path.to_path_buf())
            }
            Target::Node(t) => t
                .mkdir(&path_text(path))
                .await
                .map(PathBuf::from)
                .map_err(|e| target_error("invalid_directory", e)),
        }
    }

    /// Whether the directory has content besides identity metadata. A node
    /// has no listing operation: its directories count as empty when they
    /// carry no identity metadata (the only thing `create` would clobber).
    async fn is_empty_dir(&self, directory: &Path) -> Result<bool> {
        match self {
            Target::Local => Ok(std::fs::read_dir(directory)
                .map_err(|e| OpenDanError::io(directory, e))?
                .next()
                .is_none()),
            Target::Node(_) => Ok(true),
        }
    }
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

    fn runtime_record(&self, runtime_id: &str) -> Result<Option<RuntimeRecord>> {
        fsutil::read_json_opt::<RuntimeRecord>(&self.layout.runtime_entry(canonical_runtime(runtime_id)))
    }

    /// Whether the runtime a location names is configured and usable now:
    /// the agent's own environment (its registration and the owner's
    /// switch), or an enabled node runtime.
    fn runtime_available(&self, runtime_id: &str) -> Result<bool> {
        let registered = self.runtime_record(runtime_id)?;
        if is_native_runtime(runtime_id) {
            if registered.is_some_and(|r| r.availability != RuntimeAvailability::Available) {
                return Ok(false);
            }
            return Ok(!self
                .layout
                .workspaces_dir()
                .join(".local-unavailable.json")
                .exists());
        }
        Ok(registered.is_some_and(|r| {
            r.kind.is_node() && r.exec.is_some() && r.availability == RuntimeAvailability::Available
        }))
    }

    fn unavailable(&self, runtime_id: &str) -> OpenDanError {
        match self.runtime_record(runtime_id).ok().flatten() {
            None if !is_native_runtime(runtime_id) => invalid(
                "runtime_unavailable",
                format!("{runtime_id}: not a runtime of this agent (see `runtime list`)"),
            ),
            Some(r) if r.availability != RuntimeAvailability::Available => invalid(
                "runtime_unavailable",
                format!(
                    "{runtime_id}: {}",
                    r.last_error.unwrap_or_else(|| format!("{:?}", r.availability))
                ),
            ),
            _ => invalid("runtime_unavailable", runtime_id),
        }
    }

    async fn target(&self, runtime_id: &str) -> Result<Target> {
        if !self.runtime_available(runtime_id)? {
            return Err(self.unavailable(runtime_id));
        }
        if is_native_runtime(runtime_id) {
            return Ok(Target::Local);
        }
        let record = self
            .runtime_record(runtime_id)?
            .ok_or_else(|| self.unavailable(runtime_id))?;
        let ops_dir = self.layout.workspaces_dir().join(".remote_ops");
        match crate::runtime::open_node_target(&record, &ops_dir).await {
            Ok(target) => Ok(Target::Node(target)),
            Err(e) => Err(invalid("runtime_unavailable", format!("{runtime_id}: {e}"))),
        }
    }

    async fn location(&self, location: &WorkspaceLocation) -> Result<(Target, WorkspaceLocation)> {
        let target = self.target(&location.runtime_id).await?;
        let directory = target.canonical_dir(&location.directory).await?;
        Ok((
            target,
            WorkspaceLocation {
                runtime_id: location.runtime_id.clone(),
                directory,
            },
        ))
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
                && canonical_runtime(&existing.location.runtime_id)
                    == canonical_runtime(&location.runtime_id)
                && (dir_contains(&location.directory, &existing.location.directory)
                    || dir_contains(&existing.location.directory, &location.directory))
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

    /// Whether the old directory of a record is gone (a relocation may then
    /// take the identity along).
    async fn old_location_gone(&self, record: &WorkspaceRecord) -> Result<bool> {
        let target = self.target(&record.location.runtime_id).await?;
        Ok(target.path_type(&record.location.directory).await? == PathType::Missing)
    }

    #[allow(clippy::too_many_arguments)]
    async fn register_metadata(
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
                return self.conflict(&mut record, &location, "old_runtime_unavailable", "the original registration belongs to another host and cannot be verified here");
            }
            if record.location == location {
                if record.url != metadata.url {
                    record.url = metadata.url.clone();
                    Self::touch(&mut record);
                    self.write(&record)?;
                }
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
            match self.old_location_gone(&record).await {
                Ok(true) => {}
                Ok(false) => return self.conflict(
                    &mut record,
                    &location,
                    "duplicate_identity",
                    "the original directory still exists; copies require a new workspace identity",
                ),
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
            record.url = metadata.url.clone();
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
            url: metadata.url.clone(),
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

    async fn initialize_metadata(
        &self,
        target: &Target,
        directory: &Path,
        metadata: &WorkspaceMetadata,
    ) -> Result<WorkspaceMetadata> {
        validate_metadata(metadata)?;
        if target.write_metadata_new(directory, metadata).await? {
            return Ok(metadata.clone());
        }
        target.read_metadata(directory).await?.ok_or_else(|| {
            invalid(
                "metadata_changed",
                "metadata disappeared during initialization",
            )
        })
    }

    async fn conflict_resolved(&self, record: &WorkspaceRecord) -> bool {
        let Some(conflict) = &record.conflict else {
            return false;
        };
        if conflict.candidate == record.location {
            return false;
        }
        let Ok(target) = self.target(&conflict.candidate.runtime_id).await else {
            return false;
        };
        match target.path_type(&conflict.candidate.directory).await {
            Ok(PathType::Missing) => true,
            Ok(_) => target
                .read_metadata(&conflict.candidate.directory)
                .await
                .ok()
                .flatten()
                .is_some_and(|metadata| metadata.workspace_id != record.workspace_id),
            Err(_) => false,
        }
    }

    async fn observe(&self, record: &mut WorkspaceRecord) -> Result<()> {
        let observed = if record.runtime_host != crate::runtime::native_host_id()
            || !self.runtime_available(&record.location.runtime_id)?
        {
            Err(invalid("runtime_unavailable", &record.location.runtime_id))
        } else {
            match self.location(&record.location).await {
                Ok((target, actual)) => {
                    if actual != record.location {
                        Err(invalid(
                            "location_changed",
                            "directory resolves to a different location",
                        ))
                    } else {
                        match target.read_metadata(&actual.directory).await {
                            Ok(Some(metadata)) if metadata.workspace_id != record.workspace_id => Err(invalid(
                                "identity_changed",
                                "directory identity differs from registration",
                            )),
                            Ok(Some(metadata)) => Ok(metadata),
                            Ok(None) => Err(invalid("missing_metadata", "identity metadata is missing")),
                            Err(e) => Err(e),
                        }
                    }
                }
                Err(e) => Err(e),
            }
        };
        match observed {
            Ok(metadata) => {
                if self.conflict_resolved(record).await {
                    record.conflict = None;
                }
                record.name = metadata.name;
                record.description = metadata.description;
                record.url = metadata.url;
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
                    OpenDanError::InvalidArgument(m) if m.starts_with("workspace invalid_directory") => {
                        WorkspaceAvailability::Missing
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
        let url = request_url(&request.url)?;
        let target = self.target(&request.location.runtime_id).await?;
        let requested = path_text(&request.location.directory);
        if !buckyos_api::is_absolute_node_path(&requested) {
            return Err(invalid("invalid_directory", "directory must be absolute"));
        }
        let draft = WorkspaceMetadata {
            version: WORKSPACE_METADATA_VERSION,
            workspace_id: id.clone(),
            name: request.name.clone(),
            description: request.description.clone(),
            url: url.clone(),
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
            let directory = target
                .canonical_dir(&request.location.directory)
                .await
                .unwrap_or_else(|_| request.location.directory.clone());
            if canonical_runtime(&existing.location.runtime_id) != canonical_runtime(&request.location.runtime_id)
                || existing.location.directory != directory
            {
                return Err(invalid(
                    "operation_reused",
                    "this operation already created a workspace at another location",
                ));
            }
            if existing.name != request.name
                || existing.description != request.description
                || existing.url != url
                || existing.usage != request.usage
                || existing.source_session != request.source_session
                || existing.policy_ref != request.policy_ref
            {
                return Err(invalid(
                    "operation_conflict",
                    "operation parameters differ from the existing workspace",
                ));
            }
            let metadata = target.read_metadata(&directory).await?.ok_or_else(|| {
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
        if target.path_type(&request.location.directory).await? == PathType::Missing {
            let parent = request
                .location
                .directory
                .parent()
                .filter(|p| !path_text(p).is_empty())
                .ok_or_else(|| invalid("invalid_directory", "missing parent directory"))?;
            let parent = target.canonical_dir(parent).await?;
            let name = request
                .location
                .directory
                .file_name()
                .ok_or_else(|| invalid("invalid_directory", "missing directory name"))?
                .to_string_lossy()
                .to_string();
            let candidate = PathBuf::from(crate::runtime::target_join(&path_text(&parent), &[&name]));
            self.no_overlap(
                &id,
                &WorkspaceLocation {
                    runtime_id: request.location.runtime_id.clone(),
                    directory: candidate.clone(),
                },
            )?;
            target.mkdir(&candidate).await?;
        }
        let directory = target.canonical_dir(&request.location.directory).await?;
        let location = WorkspaceLocation {
            runtime_id: request.location.runtime_id.clone(),
            directory,
        };
        self.no_overlap(&id, &location)?;
        let metadata = match target.read_metadata(&location.directory).await? {
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
                if !target.is_empty_dir(&location.directory).await? {
                    return Err(invalid(
                        "directory_not_empty",
                        "create requires an empty directory; use import for existing content",
                    ));
                }
                self.initialize_metadata(&target, &location.directory, &draft).await?
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
        .await
    }

    async fn import(&self, request: &WorkspaceImport, who: &str) -> Result<WorkspaceRecord> {
        let _lock = self.lock()?;
        let id = self.operation_id(&request.operation_id, who)?;
        let url = request_url(&request.url)?;
        let (target, location) = self.location(&request.location).await?;
        let metadata = match target.read_metadata(&location.directory).await? {
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
                    &target,
                    &location.directory,
                    &WorkspaceMetadata {
                        version: WORKSPACE_METADATA_VERSION,
                        workspace_id: id,
                        name,
                        description: request.description.clone(),
                        url,
                        created_at_ms: crate::now_ms(),
                        operation_id: Some(request.operation_id.clone()),
                    },
                )
                .await?
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
        .await
    }

    async fn discover(&self, request: &WorkspaceDiscover, who: &str) -> Result<WorkspaceRecord> {
        let _lock = self.lock()?;
        let (target, location) = self.location(&request.location).await?;
        let metadata = target.read_metadata(&location.directory).await?.ok_or_else(|| {
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
        .await
    }

    async fn lookup(&self, workspace_id: &str) -> Result<Option<WorkspaceRecord>> {
        self.read(workspace_id)
    }

    async fn query(&self, query: &WorkspaceQuery) -> Result<Vec<WorkspaceRecord>> {
        Ok(self
            .all()?
            .into_iter()
            .filter(|r| {
                query.runtime_id.as_ref().is_none_or(|v| {
                    canonical_runtime(v) == canonical_runtime(&r.location.runtime_id)
                })
                    && query.lifecycle.is_none_or(|v| v == r.lifecycle)
                    && query.availability.is_none_or(|v| v == r.availability)
                    && query.text.as_ref().is_none_or(|v| {
                        let v = v.to_lowercase();
                        r.name.to_lowercase().contains(&v)
                            || r.description.to_lowercase().contains(&v)
                            || r.workspace_id.contains(&v)
                            || r.url.as_deref().is_some_and(|u| u.to_lowercase().contains(&v))
                    })
            })
            .collect())
    }

    async fn check(&self, workspace_id: &str) -> Result<WorkspaceRecord> {
        let _lock = self.lock_for_check().await?;
        let mut record = self.require(workspace_id)?;
        self.observe(&mut record).await?;
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
        if let Some(url) = &request.url {
            let url = request_url(&Some(url.clone()))?;
            if url != record.url {
                // The URL belongs to the directory's identity: every agent
                // that discovers the directory sees the same project.
                let (target, actual) = self.location(&record.location).await?;
                if actual != record.location {
                    return Err(invalid("location_changed", "directory resolves to a different location"));
                }
                let mut metadata = target
                    .read_metadata(&actual.directory)
                    .await?
                    .ok_or_else(|| invalid("missing_metadata", "identity metadata is missing"))?;
                if metadata.workspace_id != record.workspace_id {
                    return Err(invalid("identity_changed", "directory identity differs from registration"));
                }
                metadata.url = url.clone();
                target.write_metadata(&actual.directory, &metadata).await?;
                record.url = url;
            }
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
        if !is_native_runtime(runtime_id) {
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
            if is_native_runtime(&record.location.runtime_id) {
                self.observe(record).await?;
            }
        }
        Ok(records
            .into_iter()
            .filter(|r| is_native_runtime(&r.location.runtime_id))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_containment_follows_the_directory_os() {
        assert!(dir_contains(Path::new("/a/b"), Path::new("/a/b/c")));
        assert!(!dir_contains(Path::new("/a/b"), Path::new("/a/bc")));
        assert!(dir_contains(Path::new(r"C:\Users\o"), Path::new(r"c:\users\o\ws")));
        assert!(!dir_contains(Path::new(r"C:\Users\o"), Path::new(r"C:\Users\other")));
        assert_eq!(metadata_path(Path::new(r"C:\Users\o\ws")), r"C:\Users\o\ws\.opendan-workspace.json");
        assert_eq!(metadata_path(Path::new("/home/o/ws")), "/home/o/ws/.opendan-workspace.json");
    }
}
