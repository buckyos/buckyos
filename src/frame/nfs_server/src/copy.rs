use crate::error::*;
use crate::fsutil::{file_id, join_root, normalize_rel, validate_name};
use crate::namespace::Node;
use crate::state::SharedState;
use crate::types::WireRef;
use buckyos_api::*;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs::{File, Metadata, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Identity {
    dev: u64,
    ino: u64,
    kind: String,
    birth: Option<SystemTime>,
}
impl Identity {
    fn of(m: &Metadata) -> NfsResult<Self> {
        let id = file_id(m);
        if id.ino == 0 {
            return Err(NfsError::new(
                ErrorCode::Unsupported,
                "copy needs stable native file identity",
            ));
        }
        Ok(Self {
            dev: id.dev,
            ino: id.ino,
            birth: m.created().ok(),
            kind: if m.is_dir() {
                "dir"
            } else if m.is_file() {
                "file"
            } else if m.is_symlink() {
                "symlink"
            } else {
                "special"
            }
            .into(),
        })
    }
    fn check(&self, path: &Path) -> NfsResult<Metadata> {
        let m = std::fs::symlink_metadata(path)?;
        if Self::of(&m)? != *self {
            return Err(stale("file identity changed"));
        }
        Ok(m)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Row {
    id: i64,
    source_index: usize,
    parent: Option<i64>,
    source: String,
    target: String,
    identity: Option<Identity>,
    target_parent: Option<Identity>,
    copy_identity: Option<Identity>,
    temporary: Option<String>,
    state: String,
    error: Option<Value>,
    choice: NfsCopyChoice,
    conflict_kind: Option<String>,
    expanded: bool,
    size: u64,
    written: u64,
    modified: Option<SystemTime>,
    mode: u32,
    ctime: Option<(i64, i64)>,
}
impl Row {
    fn result(&self) -> NfsCopyItemResult {
        NfsCopyItemResult {
            id: self.id,
            source_index: self.source_index,
            source_path: self.source.clone(),
            target_path: self.target.clone(),
            kind: self
                .identity
                .as_ref()
                .map(|i| i.kind.clone())
                .unwrap_or_else(|| "unknown".into()),
            status: self.state.clone(),
            size: self
                .identity
                .as_ref()
                .filter(|i| i.kind == "file")
                .map(|_| self.size),
            mtime: self
                .modified
                .and_then(|time| time.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map(|time| time.as_secs_f64()),
            error: self.error.clone(),
            identity: self.copy_identity.as_ref().map(|i| json!(i)),
        }
    }
}

pub struct CopyJournal {
    db: Mutex<Connection>,
    locks: PathBuf,
}
impl CopyJournal {
    pub fn open(data: &Path) -> NfsResult<Self> {
        let db = Connection::open(data.join("copy.sqlite"))?;
        db.busy_timeout(Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;
            CREATE TABLE IF NOT EXISTS copy_job(task TEXT PRIMARY KEY, input TEXT NOT NULL, rules TEXT NOT NULL DEFAULT '{}');
            CREATE TABLE IF NOT EXISTS copy_item(task TEXT NOT NULL, id INTEGER NOT NULL, parent INTEGER, state TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(task,id));
            CREATE INDEX IF NOT EXISTS copy_state ON copy_item(task,state,id);")?;
        let locks = data.join("copy-locks");
        std::fs::create_dir_all(&locks)?;
        Ok(Self {
            db: Mutex::new(db),
            locks,
        })
    }
    fn save(&self, task: &str, row: &Row) -> NfsResult<()> {
        self.db.lock().unwrap().execute("INSERT INTO copy_item(task,id,parent,state,payload) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(task,id) DO UPDATE SET state=excluded.state,payload=excluded.payload", params![task,row.id,row.parent,row.state,serde_json::to_string(row)?])?;
        Ok(())
    }
    fn row(&self, task: &str, id: i64) -> NfsResult<Row> {
        let text: String = self.db.lock().unwrap().query_row(
            "SELECT payload FROM copy_item WHERE task=?1 AND id=?2",
            params![task, id],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_str(&text)?)
    }
    fn page(&self, task: &str, after: i64, limit: usize) -> NfsResult<Vec<Row>> {
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare(
            "SELECT payload FROM copy_item WHERE task=?1 AND id>?2 ORDER BY id LIMIT ?3",
        )?;
        let values = stmt
            .query_map(params![task, after, limit], |r| r.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        values
            .iter()
            .map(|s| Ok(serde_json::from_str(s)?))
            .collect()
    }
    fn next(&self, task: &str) -> NfsResult<Option<Row>> {
        let db = self.db.lock().unwrap();
        let text: Option<String> = db.query_row("SELECT payload FROM copy_item WHERE task=?1 AND state IN ('pending','creating','prepared','committed','conflict') ORDER BY CASE WHEN state='conflict' THEN 0 ELSE 1 END,id LIMIT 1", [task], |r| r.get(0)).optional()?;
        text.map(|s| Ok(serde_json::from_str(&s)?)).transpose()
    }
    fn summary(&self, task: &str) -> NfsResult<NfsCopySummary> {
        let mut summary = NfsCopySummary::default();
        let db = self.db.lock().unwrap();
        let mut stmt = db.prepare("SELECT state,COUNT(*),COALESCE(SUM(json_extract(payload,'$.size')),0),COALESCE(SUM(json_extract(payload,'$.written')),0) FROM copy_item WHERE task=?1 GROUP BY state")?;
        for row in stmt.query_map([task], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, u64>(2)?,
                r.get::<_, u64>(3)?,
            ))
        })? {
            let (state, count, bytes, written) = row?;
            match state.as_str() {
                "success" => {
                    summary.success += count;
                    summary.bytes += bytes;
                }
                "failed" => summary.failed += count,
                "skipped" => summary.skipped += count,
                "cancelled" => summary.cancelled += count,
                _ => {
                    summary.pending += count;
                    summary.bytes += written;
                }
            }
        }
        Ok(summary)
    }
    fn lock(&self, task: &str) -> NfsResult<Option<File>> {
        if !valid_task_id(task) {
            return Err(invalid("bad task id"));
        }
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.locks.join(task))?;
        match lock.try_lock() {
            Ok(()) => Ok(Some(lock)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => Err(e.into()),
        }
    }
}
fn valid_task_id(id: &str) -> bool {
    id.strip_prefix("t-")
        .is_some_and(|s| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
fn rpc_error(e: impl std::fmt::Display) -> NfsError {
    internal(format!("task-manager: {e}"))
}
async fn client(state: &SharedState) -> NfsResult<TaskManagerClient> {
    if let Some(config) = &state.config.copy_test {
        return Ok(config.task_client());
    }
    get_buckyos_api_runtime()
        .map_err(rpc_error)?
        .get_task_mgr_client()
        .await
        .map_err(rpc_error)
}
fn envelope(task: &Task) -> RunnerWriteEnvelope {
    RunnerWriteEnvelope {
        task_id: task.task_id.clone(),
        app_instance_id: None,
        runner_epoch: task.runner_epoch,
        expected_revision: task.revision,
    }
}
fn actor_matches(task: &Task, actor: &ActorRef) -> bool {
    task.schema_id == NFS_COPY_SCHEMA_ID
        && task.creator.user_id == actor.user_id
        && task.creator.app_id == actor.app_id
}
async fn owned(state: &SharedState, actor: &ActorRef, id: &str) -> NfsResult<Task> {
    let task = client(state).await?.get_task(id).await.map_err(rpc_error)?;
    if !actor_matches(&task, actor) {
        return Err(NfsError::new(
            ErrorCode::PermissionDenied,
            "this copy task belongs to another user",
        ));
    }
    Ok(task)
}
fn native(state: &SharedState, value: &Value) -> NfsResult<(String, Identity)> {
    let reference: WireRef = serde_json::from_value(value.clone())?;
    if !matches!(&reference, WireRef::Live { node_id, .. } if node_id.starts_with("nh_") && node_id != "nh_root")
    {
        return Err(NfsError::new(
            ErrorCode::Unsupported,
            "copy requires a signed local copy_ref from stat or list",
        ));
    }
    let node = state.resolve_ref(&reference)?;
    match node {
        Node::Native { root, rel, .. } => {
            let path = format!("/{root}{}{}", if rel.is_empty() { "" } else { "/" }, rel);
            let full = local(state, &path)?;
            Ok((path, Identity::of(&std::fs::symlink_metadata(full)?)?))
        }
        _ => Err(NfsError::new(
            ErrorCode::Unsupported,
            "only local filesystem entities can be copied",
        )),
    }
}
fn local(state: &SharedState, path: &str) -> NfsResult<PathBuf> {
    let normalized = normalize_rel(path)?;
    let (root_id, rel) = normalized.split_once('/').unwrap_or((&normalized, ""));
    let root = state
        .config
        .root(root_id)
        .ok_or_else(|| stale("export no longer exists"))?;
    let root_path = std::fs::canonicalize(&root.path)?;
    let full = join_root(&root_path, rel);
    if !rel.is_empty() {
        let parent = full.parent().ok_or_else(|| invalid("missing parent"))?;
        let mut current = root_path.clone();
        for component in parent
            .strip_prefix(&root_path)
            .map_err(|_| invalid("outside export"))?
            .components()
        {
            current.push(component);
            let m = std::fs::symlink_metadata(&current)?;
            if !m.is_dir() || m.is_symlink() {
                return Err(stale(
                    "copy path contains a symlink or a non-directory ancestor",
                ));
            }
        }
    }
    Ok(full)
}
fn join(parent: &str, name: &str) -> String {
    format!("{}/{name}", parent.trim_end_matches('/'))
}
fn parent(path: &str) -> &str {
    path.rsplit_once('/').map(|(p, _)| p).unwrap_or("")
}
fn keep_name(name: &str, n: usize, dir: bool) -> NfsResult<String> {
    let (stem, ext) = if !dir {
        name.rsplit_once('.')
            .filter(|(s, _)| !s.is_empty())
            .map(|(s, e)| (s, format!(".{e}")))
            .unwrap_or((name, String::new()))
    } else {
        (name, String::new())
    };
    let suffix = format!(" ({n}){ext}");
    if suffix.len() >= 255 {
        return Err(invalid("extension leaves no space for a copy name"));
    }
    let mut end = stem.len().min(255 - suffix.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    let result = format!("{}{suffix}", &stem[..end]);
    validate_name(&result)?;
    Ok(result)
}

pub async fn dispatch(
    state: &SharedState,
    method: &str,
    args: &Value,
    token: Option<&str>,
) -> NfsResult<Value> {
    let token = token.ok_or_else(|| {
        NfsError::new(
            ErrorCode::PermissionDenied,
            "copy requires an authenticated user session",
        )
    })?;
    let actor = authenticate_nfs_copy_user(token, state.config.copy_test.as_ref())
        .await
        .map_err(|e| NfsError::new(ErrorCode::PermissionDenied, e.to_string()))?;
    let api = client(state).await?;
    match method {
        "copy_capabilities" => {
            let schema = api
                .get_task_schema(NFS_COPY_SCHEMA_ID, Some(1))
                .await
                .map_err(rpc_error)?;
            if !schema.enabled {
                return Err(NfsError::new(
                    ErrorCode::Unsupported,
                    "copy task schema is disabled",
                ));
            }
            Ok(json!({"supported": cfg!(unix), "schema_id": NFS_COPY_SCHEMA_ID}))
        }
        "copy_submit" => {
            let key = args["idempotency_key"]
                .as_str()
                .filter(|k| !k.is_empty() && k.len() <= 200)
                .ok_or_else(|| invalid("idempotency_key required (max 200 bytes)"))?;
            let input: NfsCopyInput = serde_json::from_value(args["input"].clone())?;
            if input.sources.is_empty() || input.sources.len() > 256 {
                return Err(invalid("select 1–256 source entities per task"));
            }
            for source in &input.sources {
                validate_name(&source.name)?;
            }
            let existing = api
                .list_tasks(ListTasksReq {
                    creator_user_id: Some(actor.user_id.clone()),
                    creator_app_id: Some(actor.app_id.clone()),
                    idempotency_key: Some(key.into()),
                    limit: Some(1),
                    ..Default::default()
                })
                .await
                .map_err(rpc_error)?;
            if let Some(summary) = existing.tasks.first() {
                let task = owned(state, &actor, &summary.task_id).await?;
                if task.input != json!(input) {
                    return Err(NfsError::new(
                        ErrorCode::NamespaceConflict,
                        "idempotency key already used with a different input",
                    ));
                }
                spawn(state.clone(), task.task_id.clone());
                return Ok(json!({"task_id":task.task_id}));
            }
            let (_, destination) = native(state, &input.destination_ref)?;
            if destination.kind != "dir" {
                return Err(invalid("destination must be a local directory"));
            }
            if let Some(retry) = &input.retry_of {
                let previous = owned(state, &actor, retry).await?;
                if !previous.phase.is_terminal() {
                    return Err(invalid("retry requires a terminal task"));
                }
                let old: NfsCopyInput = serde_json::from_value(previous.input)?;
                if old.sources != input.sources || old.destination_ref != input.destination_ref {
                    return Err(invalid("retry cannot change sources or destination"));
                }
            }
            let task = api
                .create_delegated_task(CreateDelegatedTaskReq {
                    task_id: None,
                    name: format!("Copy {} item(s)", input.sources.len()),
                    schema_id: NFS_COPY_SCHEMA_ID.into(),
                    schema_version: Some(1),
                    input: json!(input),
                    creator: actor,
                    runner_app_instance_id: None,
                    parent_id: None,
                    child_control_policy: None,
                    policy_preset: None,
                    permission_boundary: false,
                    storage_domain: Some(StorageDomain::System),
                    idempotency_key: key.into(),
                    retry_of: input.retry_of.clone(),
                    supersedes: None,
                    message: None,
                })
                .await
                .map_err(rpc_error)?;
            spawn(state.clone(), task.task_id.clone());
            Ok(json!({"task_id":task.task_id}))
        }
        "copy_list" => {
            let page = api
                .list_tasks(ListTasksReq {
                    schema_id: Some(NFS_COPY_SCHEMA_ID.into()),
                    creator_user_id: Some(actor.user_id),
                    creator_app_id: Some(actor.app_id),
                    cursor: args["cursor"].as_str().map(String::from),
                    limit: Some(50),
                    ..Default::default()
                })
                .await
                .map_err(rpc_error)?;
            Ok(json!(page))
        }
        "copy_get" | "copy_cancel" | "copy_decide" => {
            let id = args["task_id"]
                .as_str()
                .ok_or_else(|| invalid("task_id required"))?;
            let task = owned(state, &actor, id).await?;
            if method == "copy_cancel" {
                if !task.phase.is_terminal() && task.pending_control.is_none() {
                    api.request_delegated_control(RequestDelegatedControlReq {
                        controller: actor,
                        task_id: id.into(),
                        action: TaskControlAction::Cancel,
                        request_id: args["request_id"].as_str().unwrap_or(id).into(),
                        expected_revision: None,
                    })
                    .await
                    .map_err(rpc_error)?;
                }
                spawn(state.clone(), id.into());
                return Ok(json!({"requested":true}));
            }
            if method == "copy_decide" {
                if task.phase.is_terminal() {
                    return Err(invalid("task is terminal"));
                }
                let row_id = args["item_id"]
                    .as_i64()
                    .ok_or_else(|| invalid("item_id required"))?;
                let mut row = state.copies.row(id, row_id)?;
                let choice: NfsCopyChoice = serde_json::from_value(args["choice"].clone())?;
                if choice == NfsCopyChoice::Cancel {
                    api.request_delegated_control(RequestDelegatedControlReq {
                        controller: actor,
                        task_id: id.into(),
                        action: TaskControlAction::Cancel,
                        request_id: format!("copy-conflict-cancel:{row_id}"),
                        expected_revision: None,
                    })
                    .await
                    .map_err(rpc_error)?;
                } else if row.state == "conflict" {
                    row.choice = choice.clone();
                    row.state = "pending".into();
                    if args["apply"].as_bool() == Some(true) {
                        let db = state.copies.db.lock().unwrap();
                        let text: String =
                            db.query_row("SELECT rules FROM copy_job WHERE task=?1", [id], |r| {
                                r.get(0)
                            })?;
                        let mut rules: Value = serde_json::from_str(&text)?;
                        rules[row.conflict_kind.as_deref().unwrap_or("unknown")] = json!(choice);
                        db.execute(
                            "UPDATE copy_job SET rules=?2 WHERE task=?1",
                            params![id, rules.to_string()],
                        )?;
                    }
                    state.copies.save(id, &row)?;
                }
                spawn(state.clone(), id.into());
                return Ok(json!({"accepted":true}));
            }
            let after = args["after"].as_i64().unwrap_or(0).max(0);
            let rows = state.copies.page(id, after, 100)?;
            let next = if rows.len() == 100 {
                rows.last().map(|r| r.id)
            } else {
                None
            };
            let conflict = state
                .copies
                .next(id)?
                .filter(|r| r.state == "conflict")
                .map(|r| r.result());
            Ok(
                json!({"task":task,"summary":state.copies.summary(id)?,"items":rows.iter().map(Row::result).collect::<Vec<_>>(),"next":next,"conflict":conflict}),
            )
        }
        _ => Err(invalid("unknown copy method")),
    }
}

fn spawn(state: SharedState, id: String) {
    tokio::spawn(async move {
        if let Err(e) = execute(&state, &id).await {
            log::error!("copy task {id}: {e}");
        }
    });
}
pub async fn recovery_loop(state: SharedState) {
    loop {
        if let Ok(api) = client(&state).await {
            let mut cursor = None;
            loop {
                let page = api
                    .list_tasks(ListTasksReq {
                        schema_id: Some(NFS_COPY_SCHEMA_ID.into()),
                        runner_app_id: Some(NFS_SERVER_SERVICE_NAME.into()),
                        cursor,
                        limit: Some(100),
                        ..Default::default()
                    })
                    .await;
                let Ok(page) = page else {
                    break;
                };
                for task in page.tasks {
                    if !task.phase.is_terminal() {
                        spawn(state.clone(), task.task_id);
                    }
                }
                match page.next_cursor {
                    Some(next) => cursor = Some(next),
                    None => break,
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn initialize(state: &SharedState, task: &Task) -> NfsResult<()> {
    let input: NfsCopyInput = serde_json::from_value(task.input.clone())?;
    let mut db = state.copies.db.lock().unwrap();
    if db
        .query_row(
            "SELECT 1 FROM copy_job WHERE task=?1",
            [&task.task_id],
            |r| r.get::<_, i32>(0),
        )
        .optional()?
        .is_some()
    {
        return Ok(());
    }
    let tx = db.transaction()?;
    if let Some(retry) = &input.retry_of {
        let exists = tx
            .query_row("SELECT 1 FROM copy_job WHERE task=?1", [retry], |r| {
                r.get::<_, i32>(0)
            })
            .optional()?
            .is_some();
        if !exists {
            return Err(stale(
                "original task journal is missing; manual reconciliation required",
            ));
        }
        tx.execute(
            "INSERT INTO copy_job(task,input,rules) SELECT ?1,?2,rules FROM copy_job WHERE task=?3",
            params![task.task_id, task.input.to_string(), retry],
        )?;
        tx.execute("INSERT INTO copy_item(task,id,parent,state,payload) SELECT ?1,id,parent,state,payload FROM copy_item WHERE task=?2",params![task.task_id,retry])?;
        tx.execute("UPDATE copy_item SET state='pending',payload=json_set(payload,'$.state','pending','$.error',NULL,'$.choice','ask') WHERE task=?1 AND state IN ('failed','cancelled') AND json_extract(payload,'$.copy_identity') IS NULL AND json_extract(payload,'$.temporary') IS NULL",[&task.task_id])?;
        tx.execute("UPDATE copy_item SET state='committed',payload=json_set(payload,'$.state','committed','$.error',NULL) WHERE task=?1 AND json_extract(payload,'$.identity.kind')='dir' AND json_extract(payload,'$.copy_identity') IS NOT NULL AND state IN ('failed','cancelled','directory_wait','committed')",[&task.task_id])?;
        tx.commit()?;
        return Ok(());
    }
    let (destination, destination_identity) = native(state, &input.destination_ref)?;
    tx.execute(
        "INSERT INTO copy_job(task,input) VALUES(?1,?2)",
        params![task.task_id, task.input.to_string()],
    )?;
    for (index, source) in input.sources.iter().enumerate() {
        let mut row = Row {
            id: index as i64 + 1,
            source_index: index,
            parent: None,
            source: source.source_path.clone(),
            target: join(&destination, &source.name),
            identity: None,
            target_parent: Some(destination_identity.clone()),
            copy_identity: None,
            temporary: None,
            state: "pending".into(),
            error: None,
            choice: input.conflict.clone(),
            conflict_kind: None,
            expanded: false,
            size: 0,
            written: 0,
            modified: None,
            mode: 0,
            ctime: None,
        };
        let result = (|| -> NfsResult<()> {
            let (source_path, identity) = native(state, &source.source_ref)?;
            if source_path != source.source_path {
                return Err(stale("source moved since Copy"));
            }
            let source_full = local(state, &source_path)?;
            let target_full = local(state, &destination)?;
            if identity.kind == "dir"
                && (target_full == source_full || target_full.starts_with(&source_full))
            {
                return Err(invalid(
                    "a directory cannot be copied into itself or its descendants",
                ));
            }
            row.identity = Some(identity);
            Ok(())
        })();
        if let Err(e) = result {
            row.state = "failed".into();
            row.error = Some(e.to_json());
        }
        tx.execute(
            "INSERT INTO copy_item(task,id,parent,state,payload) VALUES(?1,?2,NULL,?3,?4)",
            params![
                task.task_id,
                row.id,
                row.state,
                serde_json::to_string(&row)?
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}
fn check_parent(state: &SharedState, id: &str, row: &Row) -> NfsResult<PathBuf> {
    let path = local(state, parent(&row.target))?;
    let identity = if let Some(parent) = row.parent {
        state.copies.row(id, parent)?.copy_identity
    } else {
        row.target_parent.clone()
    }
    .ok_or_else(|| stale("destination parent identity unavailable"))?;
    identity.check(&path)?;
    Ok(path)
}
fn check_baseline(baseline: &Metadata, current: &Metadata) -> NfsResult<()> {
    let changed = Identity::of(baseline)? != Identity::of(current)?
        || baseline.len() != current.len()
        || baseline.modified().ok() != current.modified().ok();
    #[cfg(unix)]
    let changed = {
        use std::os::unix::fs::MetadataExt;
        changed
            || baseline.ctime() != current.ctime()
            || baseline.ctime_nsec() != current.ctime_nsec()
    };
    if changed {
        return Err(stale("source changed while copying"));
    }
    Ok(())
}
fn metadata_fields(row: &mut Row, m: &Metadata) {
    row.modified = m.modified().ok();
    row.size = if m.is_file() { m.len() } else { 0 };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        row.mode = m.permissions().mode() & 0o777;
        use std::os::unix::fs::MetadataExt;
        row.ctime = Some((m.ctime(), m.ctime_nsec()));
    }
}
fn preserve(row: &Row, path: &Path) -> NfsResult<()> {
    if row.identity.as_ref().is_some_and(|i| i.kind == "symlink") {
        return Ok(());
    }
    let file = File::open(path)?;
    if let Some(modified) = row.modified {
        file.set_times(std::fs::FileTimes::new().set_modified(modified))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(row.mode))?;
    }
    file.sync_all()?;
    Ok(())
}
fn cleanup(state: &SharedState, row: &mut Row) -> NfsResult<()> {
    if let Some(temporary) = &row.temporary {
        let path = local(state, temporary)?;
        if path.symlink_metadata().is_ok() {
            if let Some(identity) = &row.copy_identity {
                identity.check(&path)?;
            } else {
                return Err(stale(
                    "unidentified temporary file; manual reconciliation required",
                ));
            }
            std::fs::remove_file(&path)?;
            File::open(path.parent().unwrap())?.sync_all()?;
        }
        row.temporary = None;
        if row.state == "creating" {
            row.copy_identity = None;
            row.written = 0;
        }
    }
    Ok(())
}
async fn checkpoint(state: &SharedState, api: &TaskManagerClient, id: &str) -> NfsResult<Task> {
    for _ in 0..5 {
        let task = api.get_task(id).await.map_err(rpc_error)?;
        if task.phase.is_terminal() {
            return Ok(task);
        }
        let summary = state.copies.summary(id)?;
        match api
            .report_progress(ReportProgressReq {
                envelope: envelope(&task),
                progress: Some(json!({"journal":{"version":1,"task_id":id},"summary":summary})),
                message: Some(format!(
                    "{} copied, {} failed, {} remaining",
                    summary.success, summary.failed, summary.pending
                )),
            })
            .await
        {
            Ok(task) => return Ok(task),
            Err(e) if task_mgr_error_code(&e) == Some(TASK_ERR_REVISION_CONFLICT) => continue,
            Err(e) => return Err(rpc_error(e)),
        }
    }
    Err(internal("task progress is changing too quickly"))
}
async fn cancelled(api: &TaskManagerClient, id: &str) -> NfsResult<bool> {
    let task = api.get_task(id).await.map_err(rpc_error)?;
    Ok(task.phase.is_terminal()
        || task
            .pending_control
            .as_ref()
            .is_some_and(|c| c.action == TaskControlAction::Cancel))
}
async fn execute(state: &SharedState, id: &str) -> NfsResult<()> {
    let Some(_lock) = state.copies.lock(id)? else {
        return Ok(());
    };
    let api = client(state).await?;
    let task = api.get_task(id).await.map_err(rpc_error)?;
    if task.phase.is_terminal() {
        return Ok(());
    }
    if let Err(e) = initialize(state, &task) {
        api.runner_fail(id, "COPY_INITIALIZATION_FAILED", e.message, None)
            .await
            .map_err(rpc_error)?;
        return Ok(());
    }
    api.runner_start(id).await.map_err(rpc_error)?;
    let task = api.get_task(id).await.map_err(rpc_error)?;
    api.update_control_profile(UpdateControlProfileReq {
        envelope: envelope(&task),
        profile: TaskControlProfile {
            cancel: CancelCapability::Safe,
            ..TaskControlProfile::baseline(0)
        },
    })
    .await
    .map_err(rpc_error)?;
    loop {
        if cancelled(&api, id).await? {
            let mut after = 0;
            loop {
                let rows = state.copies.page(id, after, 100)?;
                if rows.is_empty() {
                    break;
                }
                for mut row in rows {
                    after = row.id;
                    if !matches!(
                        row.state.as_str(),
                        "success" | "skipped" | "failed" | "cancelled"
                    ) {
                        let cleaned = (|| -> NfsResult<()> {
                            if row.state == "prepared" {
                                reconcile_prepared(state, id, &mut row)?;
                            }
                            if row.state != "success" {
                                cleanup(state, &mut row)?;
                            }
                            Ok(())
                        })();
                        if let Err(error) = cleaned {
                            row.state = "failed".into();
                            row.error = Some(error.to_json());
                            state.copies.save(id, &row)?;
                        } else if row.state != "success" {
                            row.state = "cancelled".into();
                            row.error = Some(
                                json!({"code":"CANCELLED","message":"Stopped; committed children remain in the destination"}),
                            );
                            state.copies.save(id, &row)?;
                        }
                    }
                }
            }
            let task = checkpoint(state, &api, id).await?;
            if let Some(control) = &task.pending_control {
                api.ack_control(AckControlReq {
                    envelope: envelope(&task),
                    request_id: control.request_id.clone(),
                    applied: true,
                    reject_reason: None,
                })
                .await
                .map_err(rpc_error)?;
            }
            return Ok(());
        }
        let Some(mut row) = state.copies.next(id)? else {
            break;
        };
        if row.state == "conflict" {
            checkpoint(state, &api, id).await?;
            api.runner_wait(
                id,
                TaskWaitReason::with_code(TaskWaitReasonKind::Other, "copy_conflict"),
            )
            .await
            .map_err(rpc_error)?;
            return Ok(());
        }
        match process(state, &api, id, &mut row).await {
            Ok(()) => {}
            Err(e) => {
                if cancelled(&api, id).await? {
                    continue;
                }
                if let Err(cleanup_error) = cleanup(state, &mut row) {
                    row.error = Some(cleanup_error.to_json());
                } else {
                    row.error = Some(e.to_json());
                }
                row.state = "failed".into();
                state.copies.save(id, &row)?;
            }
        }
        checkpoint(state, &api, id).await?;
    }
    finalize_directories(state, id)?;
    let task = checkpoint(state, &api, id).await?;
    if task.pending_control.is_some() {
        return Ok(());
    }
    let summary = state.copies.summary(id)?;
    if summary.failed > 0 {
        api.runner_fail(
            id,
            "COPY_PARTIAL_FAILURE",
            "Some items could not be copied",
            Some(json!({"summary":summary})),
        )
        .await
        .map_err(rpc_error)?;
    } else {
        api.runner_complete(id, json!({"summary":summary}))
            .await
            .map_err(rpc_error)?;
    }
    Ok(())
}

fn choose_target(state: &SharedState, id: &str, row: &mut Row) -> NfsResult<bool> {
    let full = local(state, &row.target)?;
    let existing = match std::fs::symlink_metadata(&full) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(e) => return Err(e.into()),
    };
    let kind = format!(
        "{}:{}",
        row.identity.as_ref().unwrap().kind,
        Identity::of(&existing)?.kind
    );
    row.conflict_kind = Some(kind.clone());
    if row.choice == NfsCopyChoice::Ask {
        let text: String = state.copies.db.lock().unwrap().query_row(
            "SELECT rules FROM copy_job WHERE task=?1",
            [id],
            |r| r.get(0),
        )?;
        let rules: Value = serde_json::from_str(&text)?;
        if let Some(choice) = rules.get(&kind) {
            row.choice = serde_json::from_value(choice.clone())?;
        }
    }
    match row.choice {
        NfsCopyChoice::Ask | NfsCopyChoice::Cancel => {
            row.state = "conflict".into();
            row.error = Some(
                json!({"code":"CONFLICT","message":"The destination name already exists","target_kind":Identity::of(&existing)?.kind,"target_size":existing.is_file().then_some(existing.len()),"target_mtime":existing.modified().ok().and_then(|time|time.duration_since(SystemTime::UNIX_EPOCH).ok()).map(|time|time.as_secs_f64())}),
            );
            state.copies.save(id, row)?;
            Ok(false)
        }
        NfsCopyChoice::Skip => {
            row.state = "skipped".into();
            row.error = None;
            state.copies.save(id, row)?;
            Ok(false)
        }
        NfsCopyChoice::KeepBoth => {
            let name = row.target.rsplit('/').next().unwrap().to_string();
            for n in 2..10000 {
                let target = join(
                    parent(&row.target),
                    &keep_name(&name, n, row.identity.as_ref().unwrap().kind == "dir")?,
                );
                match std::fs::symlink_metadata(local(state, &target)?) {
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        row.target = target;
                        row.error = None;
                        state.copies.save(id, row)?;
                        return Ok(true);
                    }
                    Err(e) => return Err(e.into()),
                    _ => {}
                }
            }
            Err(invalid("no available copy name"))
        }
    }
}
fn notify_destination(state: &SharedState, row: &Row) {
    if let Ok(node) = state.resolve_dfs_path(parent(&row.target)) {
        state.bump_dir(
            &node,
            "copy",
            Some(json!({"name":row.target.rsplit('/').next()})),
        );
    }
}
fn reconcile_prepared(state: &SharedState, id: &str, row: &mut Row) -> NfsResult<()> {
    check_parent(state, id, row)?;
    let target = local(state, &row.target)?;
    match std::fs::symlink_metadata(&target) {
        Ok(m) if row.copy_identity.as_ref() == Some(&Identity::of(&m)?) => {
            cleanup(state, row)?;
            row.state = "success".into();
            row.error = None;
            state.copies.save(id, row)?;
            notify_destination(state, row);
        }
        Ok(_) => {
            return Err(stale(
                "destination identity differs from prepared copy; manual reconciliation required",
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
async fn process(
    state: &SharedState,
    api: &TaskManagerClient,
    id: &str,
    row: &mut Row,
) -> NfsResult<()> {
    check_parent(state, id, row)?;
    let target = local(state, &row.target)?;
    if row.state == "prepared" {
        reconcile_prepared(state, id, row)?;
        if row.state == "success" {
            return Ok(());
        }
        return commit_file(state, id, row);
    }
    if row.state == "creating" {
        if row.temporary.is_some() && row.copy_identity.is_some() {
            check_recorded_source(state, row)?;
            cleanup(state, row)?;
            row.state = "pending".into();
            state.copies.save(id, row)?;
        } else {
            return Err(stale(
                "interrupted before copy identity was recorded; manual reconciliation required",
            ));
        }
    }
    if row.state == "committed" {
        row.copy_identity
            .as_ref()
            .ok_or_else(|| stale("missing directory copy identity"))?
            .check(&target)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(row.mode | 0o700))?;
        }
        if !row.expanded {
            expand_directory(state, id, row)?;
        }
        row.state = "directory_wait".into();
        state.copies.save(id, row)?;
        return Ok(());
    }
    let source = local(state, &row.source)?;
    let baseline = row
        .identity
        .as_ref()
        .ok_or_else(|| stale("source identity unavailable"))?
        .check(&source)?;
    let kind = row.identity.as_ref().unwrap().kind.clone();
    if kind == "special" {
        return Err(NfsError::new(
            ErrorCode::Unsupported,
            "device files, FIFO and sockets cannot be copied",
        ));
    }
    metadata_fields(row, &baseline);
    if !choose_target(state, id, row)? {
        return Ok(());
    }
    let target = local(state, &row.target)?;
    if kind == "dir" {
        row.state = "creating".into();
        state.copies.save(id, row)?;
        if let Err(error) = std::fs::create_dir(&target) {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                row.state = "pending".into();
                row.choice = NfsCopyChoice::Ask;
                state.copies.save(id, row)?;
                return Ok(());
            }
            return Err(error.into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700))?;
        }
        row.copy_identity = Some(Identity::of(&std::fs::symlink_metadata(&target)?)?);
        File::open(target.parent().unwrap())?.sync_all()?;
        row.state = "committed".into();
        state.copies.save(id, row)?;
        notify_destination(state, row);
        return Ok(());
    }
    let temporary = join(parent(&row.target), &format!(".bucky-copy-{id}-{}", row.id));
    row.temporary = Some(temporary.clone());
    row.state = "creating".into();
    state.copies.save(id, row)?;
    let temporary = local(state, &temporary)?;
    if kind == "symlink" {
        let link = std::fs::read_link(&source)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(link, &temporary)?;
        #[cfg(not(unix))]
        return Err(NfsError::new(
            ErrorCode::Unsupported,
            "symlink copying is unsupported on this platform",
        ));
        row.copy_identity = Some(Identity::of(&std::fs::symlink_metadata(&temporary)?)?);
        state.copies.save(id, row)?;
        check_baseline(&baseline, &std::fs::symlink_metadata(&source)?)?;
    } else {
        let mut source_options = OpenOptions::new();
        source_options.read(true);
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::OpenOptionsExt;
            source_options.custom_flags(0x20000 | 0x800);
        }
        let source_file = source_options.open(&source)?;
        check_baseline(&baseline, &source_file.metadata()?)?;
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&temporary)?;
        row.copy_identity = Some(Identity::of(&file.metadata()?)?);
        state.copies.save(id, row)?;
        let mut source_file = tokio::fs::File::from_std(source_file);
        let mut output = tokio::fs::File::from_std(file);
        let mut buffer = vec![0u8; 1024 * 1024];
        let mut check = std::time::Instant::now();
        let mut progress = std::time::Instant::now();
        loop {
            if check.elapsed() >= Duration::from_millis(100) {
                if cancelled(api, id).await? {
                    return Err(internal("copy cancellation requested"));
                }
                check = std::time::Instant::now();
            }
            let read = source_file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read]).await?;
            row.written += read as u64;
            if progress.elapsed() >= Duration::from_millis(500) {
                state.copies.save(id, row)?;
                checkpoint(state, api, id).await?;
                progress = std::time::Instant::now();
            }
            if state.config.copy_test.is_some()
                && std::env::var("NFS_COPY_TEST_SLOW").as_deref() == Ok("1")
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        output.flush().await?;
        output.sync_all().await?;
        check_baseline(&baseline, &source_file.metadata().await?)?;
        check_baseline(&baseline, &std::fs::symlink_metadata(&source)?)?;
        preserve(row, &temporary)?;
    }
    File::open(temporary.parent().unwrap())?.sync_all()?;
    check_parent(state, id, row)?;
    if cancelled(api, id).await? {
        return Err(internal("copy cancellation requested"));
    }
    row.state = "prepared".into();
    state.copies.save(id, row)?;
    commit_file(state, id, row)
}
fn check_recorded_source(state: &SharedState, row: &Row) -> NfsResult<()> {
    let source = local(state, &row.source)?;
    let current = row.identity.as_ref().unwrap().check(&source)?;
    if current.modified().ok() != row.modified || (current.is_file() && current.len() != row.size) {
        return Err(stale("source changed before copy commit"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if row.ctime != Some((current.ctime(), current.ctime_nsec())) {
            return Err(stale("source changed before copy commit"));
        }
    }
    Ok(())
}
fn commit_file(state: &SharedState, id: &str, row: &mut Row) -> NfsResult<()> {
    check_parent(state, id, row)?;
    let target = local(state, &row.target)?;
    let temporary = local(
        state,
        row.temporary
            .as_deref()
            .ok_or_else(|| stale("prepared temporary file missing"))?,
    )?;
    row.copy_identity
        .as_ref()
        .ok_or_else(|| stale("prepared copy identity missing"))?
        .check(&temporary)?;
    check_recorded_source(state, row)?;
    if let Err(error) = std::fs::hard_link(&temporary, &target) {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            row.state = "creating".into();
            cleanup(state, row)?;
            row.state = "pending".into();
            row.choice = NfsCopyChoice::Ask;
            state.copies.save(id, row)?;
            return Ok(());
        }
        return Err(error.into());
    }
    File::open(target.parent().unwrap())?.sync_all()?;
    if state.config.copy_test.is_some()
        && std::env::var("NFS_COPY_TEST_FAULT").as_deref() == Ok("after-commit")
    {
        if let Ok(marker) = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(state.config.data_dir.join("copy-fault-fired"))
        {
            marker.sync_all()?;
            std::process::exit(86);
        }
    }
    row.state = "success".into();
    row.error = None;
    cleanup(state, row)?;
    state.copies.save(id, row)?;
    notify_destination(state, row);
    Ok(())
}
fn expand_directory(state: &SharedState, id: &str, row: &mut Row) -> NfsResult<()> {
    let source = local(state, &row.source)?;
    row.identity.as_ref().unwrap().check(&source)?;
    let mut db = state.copies.db.lock().unwrap();
    let tx = db.transaction()?;
    let mut next: i64 = tx.query_row(
        "SELECT COALESCE(MAX(id),0)+1 FROM copy_item WHERE task=?1",
        [id],
        |r| r.get(0),
    )?;
    for entry in std::fs::read_dir(&source)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid("non-UTF-8 directory entry cannot be represented"))?;
        validate_name(&name)?;
        let mut child = Row {
            id: next,
            source_index: row.source_index,
            parent: Some(row.id),
            source: join(&row.source, &name),
            target: join(&row.target, &name),
            identity: None,
            target_parent: None,
            copy_identity: None,
            temporary: None,
            state: "pending".into(),
            error: None,
            choice: NfsCopyChoice::Ask,
            conflict_kind: None,
            expanded: false,
            size: 0,
            written: 0,
            modified: None,
            mode: 0,
            ctime: None,
        };
        match std::fs::symlink_metadata(entry.path())
            .map_err(NfsError::from)
            .and_then(|m| Identity::of(&m))
        {
            Ok(identity) => child.identity = Some(identity),
            Err(e) => {
                child.state = "failed".into();
                child.error = Some(e.to_json());
            }
        }
        tx.execute(
            "INSERT INTO copy_item(task,id,parent,state,payload) VALUES(?1,?2,?3,?4,?5)",
            params![
                id,
                next,
                row.id,
                child.state,
                serde_json::to_string(&child)?
            ],
        )?;
        next += 1;
    }
    if let Node::Native {
        anchored: Some(entity),
        ..
    } = state.resolve_dfs_path(&row.source)?
    {
        for binding in state.db.bindings_by_parent(entity.node_id)? {
            let child = Row {
                id: next,
                source_index: row.source_index,
                parent: Some(row.id),
                source: join(&row.source, &binding.name),
                target: join(&row.target, &binding.name),
                identity: None,
                target_parent: None,
                copy_identity: None,
                temporary: None,
                state: "skipped".into(),
                error: Some(
                    json!({"code":"VIRTUAL_BINDING","message":"Virtual binding skipped; referenced entity was not copied"}),
                ),
                choice: NfsCopyChoice::Skip,
                conflict_kind: None,
                expanded: false,
                size: 0,
                written: 0,
                modified: None,
                mode: 0,
                ctime: None,
            };
            tx.execute(
                "INSERT INTO copy_item(task,id,parent,state,payload) VALUES(?1,?2,?3,?4,?5)",
                params![
                    id,
                    next,
                    row.id,
                    child.state,
                    serde_json::to_string(&child)?
                ],
            )?;
            next += 1;
        }
    }
    row.expanded = true;
    tx.execute(
        "UPDATE copy_item SET payload=?3 WHERE task=?1 AND id=?2",
        params![id, row.id, serde_json::to_string(row)?],
    )?;
    tx.commit()?;
    Ok(())
}
fn finalize_directories(state: &SharedState, id: &str) -> NfsResult<()> {
    loop {
        let value: Option<String>=state.copies.db.lock().unwrap().query_row("SELECT payload FROM copy_item WHERE task=?1 AND state='directory_wait' ORDER BY id DESC LIMIT 1",[id],|r|r.get(0)).optional()?;
        let Some(value) = value else {
            break;
        };
        let mut row: Row = serde_json::from_str(&value)?;
        let failed: u64=state.copies.db.lock().unwrap().query_row("SELECT COUNT(*) FROM copy_item WHERE task=?1 AND parent=?2 AND state IN ('failed','cancelled')",params![id,row.id],|r|r.get(0))?;
        let result = (|| -> NfsResult<()> {
            check_parent(state, id, &row)?;
            let target = local(state, &row.target)?;
            row.copy_identity.as_ref().unwrap().check(&target)?;
            preserve(&row, &target)?;
            if failed > 0 {
                return Err(internal(
                    "directory is partially copied; see child item results",
                ));
            }
            Ok(())
        })();
        match result {
            Ok(()) => {
                row.state = "success".into();
                row.error = None;
            }
            Err(e) => {
                row.state = "failed".into();
                row.error = Some(e.to_json());
            }
        }
        state.copies.save(id, &row)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keep_both_preserves_extension_and_utf8_boundary() {
        let name = format!("{}.txt", "汉".repeat(83));
        let result = keep_name(&name, 12, false).unwrap();
        assert!(result.len() <= 255);
        assert!(result.ends_with(" (12).txt"));
        assert_eq!(keep_name("a.tar.gz", 2, false).unwrap(), "a.tar (2).gz");
        assert_eq!(keep_name(".config", 2, false).unwrap(), ".config (2)");
    }
    #[test]
    fn journal_lock_excludes_concurrent_runners() {
        let dir = tempfile::tempdir().unwrap();
        let journal = CopyJournal::open(dir.path()).unwrap();
        let id = "t-00000000000000000000000000000000";
        let guard = journal.lock(id).unwrap().unwrap();
        assert!(journal.lock(id).unwrap().is_none());
        drop(guard);
        assert!(journal.lock(id).unwrap().is_some());
    }
}
