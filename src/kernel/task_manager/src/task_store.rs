//! Task Core durable store: schema bootstrap plus the transactional command
//! layer that enforces the TaskMgr 2.0 protocol invariants (doc §2.4):
//! one-shot Result, Terminal absorption, runner-epoch fencing and revision
//! CAS. Immutable columns are physically excluded from every UPDATE — the
//! only statement that writes them is the INSERT at create time.
//!
//! Every mutation is one RDB transaction: load row → validate → rewrite the
//! mutable column set guarded by `WHERE revision = ?` → append exactly one
//! `task_event` row → commit. KEvent publishing happens after commit, in the
//! service layer.

use buckyos_api::*;
use kRPC::{RPCErrors, Result};
use log::*;
use serde_json::{json, Value};
use sqlx::any::{install_default_drivers, AnyPoolOptions, AnyRow};
use sqlx::{Any, AnyPool, Executor, Row, Transaction};
use std::str::FromStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Once;

static INSTALL_DRIVERS: Once = Once::new();
static EVENT_SEQ: AtomicU64 = AtomicU64::new(0);

fn ensure_any_drivers_installed() {
    INSTALL_DRIVERS.call_once(install_default_drivers);
}

pub fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis().max(0) as u64
}

pub fn new_task_id() -> TaskId {
    format!("t-{}", uuid::Uuid::new_v4().simple())
}

pub fn new_grant_id() -> String {
    format!("g-{}", uuid::Uuid::new_v4().simple())
}

/// Time-ordered opaque event id: ms timestamp prefix + process-local
/// sequence, so lexicographic order ≈ creation order and cursor queries can
/// use plain string comparison.
fn new_event_id(now: u64) -> String {
    let seq = EVENT_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("e{:013}-{:06}-{:04x}", now, seq % 1_000_000, rand_suffix())
}

fn rand_suffix() -> u16 {
    // Uniqueness back-stop across process restarts within the same ms.
    (uuid::Uuid::new_v4().as_u128() & 0xffff) as u16
}

fn db_err(err: sqlx::Error) -> RPCErrors {
    RPCErrors::ReasonError(format!("task store error: {}", err))
}

fn is_unique_violation(err: &sqlx::Error) -> bool {
    match err {
        sqlx::Error::Database(db) => {
            let message = db.message().to_ascii_lowercase();
            message.contains("unique") || message.contains("duplicate")
        }
        _ => false,
    }
}

/// A committed mutation: the fresh snapshot plus the durable event to fan
/// out through KEvent.
pub struct MutationOutcome {
    pub task: Task,
    pub event: TaskEvent,
}

/// Arguments shared by both create paths after service-level validation.
pub struct CreateTaskArgs {
    pub name: String,
    pub schema_id: String,
    pub schema_version: u32,
    pub input: Value,
    pub creator: ActorRef,
    pub idempotency_key: String,
    pub origin_ref: Option<TaskOriginRef>,
    pub parent_id: Option<TaskId>,
    pub child_control_policy: ChildControlPolicy,
    pub policy_preset: String,
    pub permission_boundary: bool,
    pub retry_of: Option<TaskId>,
    pub supersedes: Option<TaskId>,
    pub executor: TaskExecutor,
    pub assignees: Vec<String>,
    pub phase: TaskPhase,
    pub wait_reason: Option<TaskWaitReason>,
    pub message: Option<String>,
}

pub struct TaskStore {
    pool: AnyPool,
    backend: RdbBackend,
}

pub type DbResult<T> = std::result::Result<T, sqlx::Error>;

impl TaskStore {
    pub async fn open(
        connection: &str,
        backend: RdbBackend,
        schema: Option<&str>,
    ) -> std::result::Result<Self, String> {
        ensure_any_drivers_installed();
        let mut opts = AnyPoolOptions::new().max_connections(8);
        // Each pooled sqlite connection needs `foreign_keys = ON`; the pragma
        // is per-connection, so it must run in after_connect for every
        // connection the pool opens.
        if backend == RdbBackend::Sqlite {
            opts = opts.after_connect(|conn, _meta| {
                Box::pin(async move {
                    conn.execute("PRAGMA foreign_keys = ON;").await?;
                    Ok(())
                })
            });
        }
        let pool = opts
            .connect(connection)
            .await
            .map_err(|err| format!("open task-manager db at {}: {}", connection, err))?;
        let store = TaskStore { pool, backend };
        store
            .apply_schema(schema)
            .await
            .map_err(|err| format!("apply task-manager schema: {}", err))?;
        Ok(store)
    }

    pub async fn open_from_service_spec() -> std::result::Result<Self, String> {
        let instance = get_rdb_instance(
            TASK_MANAGER_SERVICE_NAME,
            None,
            TASK_MANAGER_RDB_INSTANCE_ID,
        )
        .await
        .map_err(|err| format!("resolve task-manager rdb instance failed: {}", err))?;
        info!("task_store.open {}", instance.connection);
        Self::open(
            &instance.connection,
            instance.backend,
            instance.schema.as_deref(),
        )
        .await
    }

    async fn apply_schema(&self, override_ddl: Option<&str>) -> DbResult<()> {
        let ddl: &str =
            override_ddl
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(match self.backend {
                    RdbBackend::Sqlite => TASK_MANAGER_RDB_SCHEMA_SQLITE,
                    RdbBackend::Postgres => TASK_MANAGER_RDB_SCHEMA_POSTGRES,
                });
        for statement in split_sql_statements(ddl) {
            self.pool.execute(statement.as_str()).await?;
        }
        // beta2.2 no-compat strategy (doc §15.6): a dev/DV database still on
        // the 1.x layout (integer task ids) is dropped and rebuilt as v7.
        self.rebuild_if_v1_layout().await?;
        Ok(())
    }

    async fn rebuild_if_v1_layout(&self) -> DbResult<()> {
        let v1 = match self.backend {
            RdbBackend::Sqlite => {
                sqlx::query("SELECT 1 FROM pragma_table_info('task') WHERE name = 'task_type'")
                    .fetch_optional(&self.pool)
                    .await?
                    .is_some()
            }
            RdbBackend::Postgres => sqlx::query(
                "SELECT 1 FROM information_schema.columns WHERE table_name = 'task' AND column_name = 'task_type'",
            )
            .fetch_optional(&self.pool)
            .await?
            .is_some(),
        };
        if !v1 {
            return Ok(());
        }
        warn!("task_store: 1.x schema detected; rebuilding as TaskMgr 2.0 schema v7 (no-compat)");
        for table in [
            "task_note",
            "task_event",
            "task_acl_grant",
            "task_assignee",
            "task_schema",
            "task",
        ] {
            let sql = format!("DROP TABLE IF EXISTS {}", table);
            self.pool.execute(sql.as_str()).await?;
        }
        let ddl = match self.backend {
            RdbBackend::Sqlite => TASK_MANAGER_RDB_SCHEMA_SQLITE,
            RdbBackend::Postgres => TASK_MANAGER_RDB_SCHEMA_POSTGRES,
        };
        for statement in split_sql_statements(ddl) {
            self.pool.execute(statement.as_str()).await?;
        }
        Ok(())
    }

    fn render_sql(&self, sql: &str) -> String {
        match self.backend {
            RdbBackend::Postgres => rewrite_placeholders_to_dollar(sql),
            RdbBackend::Sqlite => sql.to_string(),
        }
    }

    pub fn pool(&self) -> &AnyPool {
        &self.pool
    }

    // -----------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------

    pub async fn get_task(&self, task_id: &str) -> Result<Option<Task>> {
        let sql = self.render_sql("SELECT * FROM task WHERE task_id = ?");
        let row = sqlx::query(&sql)
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let mut task = task_from_row(row).map_err(db_err)?;
        self.attach_assignees(&mut task).await?;
        Ok(Some(task))
    }

    async fn attach_assignees(&self, task: &mut Task) -> Result<()> {
        if task.executor.kind() == TaskExecutorKind::HumanSet {
            task.assignees = Some(self.active_assignees(&task.task_id).await?);
        }
        Ok(())
    }

    pub async fn active_assignees(&self, task_id: &str) -> Result<Vec<String>> {
        let sql = self.render_sql(
            "SELECT user_id FROM task_assignee WHERE task_id = ? AND revoked_at IS NULL ORDER BY user_id",
        );
        let rows = sqlx::query(&sql)
            .bind(task_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|row| row.try_get::<String, _>("user_id").map_err(db_err))
            .collect()
    }

    /// Parent chain from the task itself up to the root:
    /// `[(task_id, parent_id, permission_boundary), ...]`.
    pub async fn get_task_chain(&self, task_id: &str) -> Result<Vec<(TaskId, Option<TaskId>, bool)>> {
        let mut chain = Vec::new();
        let mut cursor = Some(task_id.to_string());
        while let Some(current) = cursor {
            if chain.len() > 128 {
                return Err(RPCErrors::ReasonError(
                    "task parent chain too deep or cyclic".to_string(),
                ));
            }
            let sql = self.render_sql(
                "SELECT task_id, parent_id, permission_boundary FROM task WHERE task_id = ?",
            );
            let row = sqlx::query(&sql)
                .bind(&current)
                .fetch_optional(&self.pool)
                .await
                .map_err(db_err)?;
            let Some(row) = row else { break };
            let parent: Option<String> = row.try_get("parent_id").map_err(db_err)?;
            let boundary: i64 = row.try_get("permission_boundary").map_err(db_err)?;
            chain.push((current.clone(), parent.clone(), boundary != 0));
            cursor = parent;
        }
        Ok(chain)
    }

    /// Active explicit grants attached to any of `task_ids`.
    pub async fn active_grants_for(&self, task_ids: &[TaskId]) -> Result<Vec<TaskAclGrant>> {
        let mut grants = Vec::new();
        for task_id in task_ids {
            let sql = self
                .render_sql("SELECT * FROM task_acl_grant WHERE task_id = ? AND revoked_at IS NULL");
            let rows = sqlx::query(&sql)
                .bind(task_id)
                .fetch_all(&self.pool)
                .await
                .map_err(db_err)?;
            for row in rows {
                grants.push(grant_from_row(row).map_err(db_err)?);
            }
        }
        Ok(grants)
    }

    pub async fn all_grants_for_task(&self, task_id: &str) -> Result<Vec<TaskAclGrant>> {
        let sql =
            self.render_sql("SELECT * FROM task_acl_grant WHERE task_id = ? ORDER BY created_at");
        let rows = sqlx::query(&sql)
            .bind(task_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|row| grant_from_row(row).map_err(db_err))
            .collect()
    }

    /// Is the principal a participant (creator / runner app / active
    /// assignee) of any task in the tree? Powers the preset's tree-wide
    /// MetaOnly read row without scanning the tree in Rust.
    pub async fn is_tree_participant(
        &self,
        root_id: &str,
        user_id: &str,
        app_id: &str,
    ) -> Result<bool> {
        let sql = self.render_sql(
            "SELECT 1 FROM task WHERE root_id = ? AND ((creator_user_id = ? AND creator_app_id = ?) OR runner_app_id = ?) LIMIT 1",
        );
        let hit = sqlx::query(&sql)
            .bind(root_id)
            .bind(user_id)
            .bind(app_id)
            .bind(app_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        if hit.is_some() {
            return Ok(true);
        }
        let sql = self.render_sql(
            "SELECT 1 FROM task_assignee a JOIN task t ON a.task_id = t.task_id WHERE t.root_id = ? AND a.user_id = ? AND a.revoked_at IS NULL LIMIT 1",
        );
        let hit = sqlx::query(&sql)
            .bind(root_id)
            .bind(user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(hit.is_some())
    }

    pub async fn list_tasks(&self, req: &ListTasksReq) -> Result<(Vec<Task>, Option<String>)> {
        let mut sql = String::from("SELECT * FROM task");
        let mut conditions: Vec<String> = Vec::new();
        enum Param {
            Text(String),
            Int(i64),
        }
        let mut params: Vec<Param> = Vec::new();

        if let Some(user) = req.creator_user_id.as_deref() {
            conditions.push("creator_user_id = ?".into());
            params.push(Param::Text(user.to_string()));
        }
        if let Some(app) = req.creator_app_id.as_deref() {
            conditions.push("creator_app_id = ?".into());
            params.push(Param::Text(app.to_string()));
        }
        if let Some(schema) = req.schema_id.as_deref() {
            conditions.push("schema_id = ?".into());
            params.push(Param::Text(schema.to_string()));
        }
        if let Some(phase) = req.phase {
            conditions.push("phase = ?".into());
            params.push(Param::Text(phase.to_string()));
        }
        if let Some(root) = req.root_id.as_deref() {
            conditions.push("root_id = ?".into());
            params.push(Param::Text(root.to_string()));
        }
        if let Some(kind) = req.executor_kind {
            conditions.push("executor_kind = ?".into());
            params.push(Param::Text(kind.to_string()));
        }
        if let Some(after) = req.created_after {
            conditions.push("created_at >= ?".into());
            params.push(Param::Int(after as i64));
        }
        if let Some(before) = req.created_before {
            conditions.push("created_at < ?".into());
            params.push(Param::Int(before as i64));
        }
        if !req.include_archived {
            conditions.push("archived_at IS NULL".into());
        }
        if let Some(cursor) = req.cursor.as_deref() {
            let (created_at, task_id) = parse_cursor(cursor)?;
            conditions.push("(created_at > ? OR (created_at = ? AND task_id > ?))".into());
            params.push(Param::Int(created_at));
            params.push(Param::Int(created_at));
            params.push(Param::Text(task_id));
        }

        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        let limit = req.limit.unwrap_or(100).clamp(1, 500) as i64;
        sql.push_str(" ORDER BY created_at ASC, task_id ASC LIMIT ");
        sql.push_str(&(limit + 1).to_string());

        let sql = self.render_sql(&sql);
        let mut query = sqlx::query(&sql);
        for param in params {
            query = match param {
                Param::Text(v) => query.bind(v),
                Param::Int(v) => query.bind(v),
            };
        }
        let rows = query.fetch_all(&self.pool).await.map_err(db_err)?;
        let mut tasks: Vec<Task> = rows
            .into_iter()
            .map(|row| task_from_row(row).map_err(db_err))
            .collect::<Result<_>>()?;
        let next_cursor = if tasks.len() as i64 > limit {
            tasks.truncate(limit as usize);
            tasks
                .last()
                .map(|task| make_cursor(task.created_at, &task.task_id))
        } else {
            None
        };
        Ok((tasks, next_cursor))
    }

    /// All tasks of a tree in stable `(created_at, task_id)` order.
    pub async fn list_tree(
        &self,
        root_id: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<Task>, Option<String>)> {
        let req = ListTasksReq {
            root_id: Some(root_id.to_string()),
            include_archived: true,
            cursor: cursor.map(|s| s.to_string()),
            limit: Some(limit),
            ..Default::default()
        };
        self.list_tasks(&req).await
    }

    pub async fn list_children(
        &self,
        parent_id: &str,
        cursor: Option<&str>,
        limit: u32,
    ) -> Result<(Vec<Task>, Option<String>)> {
        let mut sql = String::from("SELECT * FROM task WHERE parent_id = ?");
        let mut binds: Vec<String> = vec![parent_id.to_string()];
        let mut int_binds: Vec<i64> = Vec::new();
        if let Some(cursor) = cursor {
            let (created_at, task_id) = parse_cursor(cursor)?;
            sql.push_str(" AND (created_at > ? OR (created_at = ? AND task_id > ?))");
            int_binds.push(created_at);
            int_binds.push(created_at);
            binds.push(task_id);
        }
        let limit = limit.clamp(1, 500) as i64;
        sql.push_str(" ORDER BY created_at ASC, task_id ASC LIMIT ");
        sql.push_str(&(limit + 1).to_string());
        let sql = self.render_sql(&sql);
        let mut query = sqlx::query(&sql).bind(binds[0].clone());
        if !int_binds.is_empty() {
            query = query
                .bind(int_binds[0])
                .bind(int_binds[1])
                .bind(binds[1].clone());
        }
        let rows = query.fetch_all(&self.pool).await.map_err(db_err)?;
        let mut tasks: Vec<Task> = rows
            .into_iter()
            .map(|row| task_from_row(row).map_err(db_err))
            .collect::<Result<_>>()?;
        let next_cursor = if tasks.len() as i64 > limit {
            tasks.truncate(limit as usize);
            tasks
                .last()
                .map(|task| make_cursor(task.created_at, &task.task_id))
        } else {
            None
        };
        Ok((tasks, next_cursor))
    }

    /// The subtree rooted at `task_id` (task itself included), bounded by
    /// `max_nodes`. Loads the whole tree by root_id and filters in memory —
    /// trees are bounded by the recursive-control node cap.
    pub async fn collect_subtree(&self, task: &Task, max_nodes: usize) -> Result<Vec<Task>> {
        let (tree, _) = self.list_tree(&task.root_id, None, max_nodes.max(1) as u32).await?;
        let mut result = Vec::new();
        let mut frontier: Vec<&str> = vec![task.task_id.as_str()];
        result.push(task.clone());
        while let Some(current) = frontier.pop() {
            for node in tree.iter() {
                if node.parent_id.as_deref() == Some(current) {
                    result.push(node.clone());
                    frontier.push(node.task_id.as_str());
                    if result.len() >= max_nodes {
                        return Ok(result);
                    }
                }
            }
        }
        Ok(result)
    }

    pub async fn find_by_idempotency(
        &self,
        creator_user_id: &str,
        creator_app_id: &str,
        idempotency_key: &str,
    ) -> Result<Option<Task>> {
        let sql = self.render_sql(
            "SELECT * FROM task WHERE creator_user_id = ? AND creator_app_id = ? AND idempotency_key = ?",
        );
        let row = sqlx::query(&sql)
            .bind(creator_user_id)
            .bind(creator_app_id)
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        match row {
            Some(row) => {
                let mut task = task_from_row(row).map_err(db_err)?;
                self.attach_assignees(&mut task).await?;
                Ok(Some(task))
            }
            None => Ok(None),
        }
    }

    pub async fn find_by_origin(&self, kind: &str, id: &str) -> Result<Option<Task>> {
        let sql = self.render_sql("SELECT * FROM task WHERE origin_kind = ? AND origin_id = ?");
        let row = sqlx::query(&sql)
            .bind(kind)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        match row {
            Some(row) => Ok(Some(task_from_row(row).map_err(db_err)?)),
            None => Ok(None),
        }
    }

    // -----------------------------------------------------------------
    // Create
    // -----------------------------------------------------------------

    /// Idempotent create. Replays (same creator principal + idempotency key)
    /// return the original task after verifying the immutable request digest
    /// still matches; mismatches fail with `idempotency_conflict`.
    pub async fn create_task(&self, args: CreateTaskArgs) -> Result<MutationOutcome> {
        if let Some(existing) = self
            .find_by_idempotency(
                &args.creator.user_id,
                &args.creator.app_id,
                &args.idempotency_key,
            )
            .await?
        {
            return self.replay_created(existing, &args);
        }

        let now = now_ms();
        let task_id = new_task_id();
        let (root_id, parent_root) = match args.parent_id.as_deref() {
            Some(parent_id) => {
                let parent = self.get_task(parent_id).await?.ok_or_else(|| {
                    task_mgr_error(TASK_ERR_NOT_FOUND, format!("parent task {}", parent_id))
                })?;
                (parent.root_id.clone(), Some(parent))
            }
            None => (task_id.clone(), None),
        };
        drop(parent_root);

        let input_digest = compute_task_input_digest(&args.input);
        let (executor_kind, target_id, runner_app_id, runner_instance_id) = match &args.executor {
            TaskExecutor::Unbound => (TaskExecutorKind::Unbound, None, None, None),
            TaskExecutor::App {
                target_id,
                app_id,
                app_instance_id,
            } => (
                TaskExecutorKind::App,
                target_id.clone(),
                Some(app_id.clone()),
                app_instance_id.clone(),
            ),
            TaskExecutor::HumanSet => (TaskExecutorKind::HumanSet, None, None, None),
        };
        let runner_epoch: u64 = if executor_kind == TaskExecutorKind::App {
            1
        } else {
            0
        };
        let control_profile = TaskControlProfile::baseline(now);

        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let sql = self.render_sql(
            "INSERT INTO task (
                task_id, schema_id, schema_version, name, input_json, input_digest,
                creator_user_id, creator_app_id, creator_instance_id, idempotency_key,
                origin_kind, origin_id, parent_id, root_id, child_control_policy_json,
                retry_of, supersedes, executor_kind, runner_target_id, runner_instance_id,
                runner_app_id, runner_epoch, phase, wait_reason_json, control_profile_json,
                message, policy_preset, permission_boundary, revision, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        let insert = sqlx::query(&sql)
            .bind(&task_id)
            .bind(&args.schema_id)
            .bind(args.schema_version as i64)
            .bind(&args.name)
            .bind(args.input.to_string())
            .bind(&input_digest)
            .bind(&args.creator.user_id)
            .bind(&args.creator.app_id)
            .bind(args.creator.app_instance_id.clone())
            .bind(&args.idempotency_key)
            .bind(args.origin_ref.as_ref().map(|o| o.kind.clone()))
            .bind(args.origin_ref.as_ref().map(|o| o.id.clone()))
            .bind(args.parent_id.clone())
            .bind(&root_id)
            .bind(serde_json::to_string(&args.child_control_policy).unwrap_or_default())
            .bind(args.retry_of.clone())
            .bind(args.supersedes.clone())
            .bind(executor_kind.to_string())
            .bind(target_id)
            .bind(runner_instance_id)
            .bind(runner_app_id)
            .bind(runner_epoch as i64)
            .bind(args.phase.to_string())
            .bind(
                args.wait_reason
                    .as_ref()
                    .map(|r| serde_json::to_string(r).unwrap_or_default()),
            )
            .bind(serde_json::to_string(&control_profile).unwrap_or_default())
            .bind(args.message.clone())
            .bind(&args.policy_preset)
            .bind(if args.permission_boundary { 1i64 } else { 0i64 })
            .bind(1i64)
            .bind(now as i64)
            .bind(now as i64)
            .execute(&mut *tx)
            .await;

        if let Err(err) = insert {
            drop(tx);
            if is_unique_violation(&err) {
                // Concurrent create with the same idempotency key (or origin
                // ref): re-read and replay-check.
                if let Some(existing) = self
                    .find_by_idempotency(
                        &args.creator.user_id,
                        &args.creator.app_id,
                        &args.idempotency_key,
                    )
                    .await?
                {
                    return self.replay_created(existing, &args);
                }
                if let Some(origin) = args.origin_ref.as_ref() {
                    if let Some(existing) = self.find_by_origin(&origin.kind, &origin.id).await? {
                        return self.replay_created(existing, &args);
                    }
                }
            }
            return Err(db_err(err));
        }

        for assignee in &args.assignees {
            let sql = self.render_sql(
                "INSERT INTO task_assignee (task_id, user_id, granted_by_user_id, granted_by_app_id, created_at) VALUES (?, ?, ?, ?, ?)",
            );
            sqlx::query(&sql)
                .bind(&task_id)
                .bind(assignee)
                .bind(&args.creator.user_id)
                .bind(&args.creator.app_id)
                .bind(now as i64)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }

        let event = self
            .insert_event_tx(
                &mut tx,
                &task_id,
                &root_id,
                1,
                TaskEventType::TaskCreated,
                Some(&args.creator),
                json!({
                    "schema_id": args.schema_id,
                    "schema_version": args.schema_version,
                    "phase": args.phase.to_string(),
                    "executor_kind": executor_kind.to_string(),
                    "parent_id": args.parent_id,
                }),
                now,
            )
            .await?;
        tx.commit().await.map_err(db_err)?;

        let task = self.get_task(&task_id).await?.ok_or_else(|| {
            RPCErrors::ReasonError("created task vanished before readback".to_string())
        })?;
        info!(
            "task_store.create_task: task_id={} schema={}/{} phase={} executor={} creator={}:{}",
            task_id,
            args.schema_id,
            args.schema_version,
            args.phase,
            executor_kind,
            args.creator.user_id,
            args.creator.app_id
        );
        Ok(MutationOutcome { task, event })
    }

    fn replay_created(&self, existing: Task, args: &CreateTaskArgs) -> Result<MutationOutcome> {
        let same = existing.schema_id == args.schema_id
            && existing.schema_version == args.schema_version
            && existing.input_digest == compute_task_input_digest(&args.input)
            && existing.parent_id == args.parent_id
            && existing.executor.kind() == args.executor.kind();
        if !same {
            return Err(task_mgr_error(
                TASK_ERR_IDEMPOTENCY_CONFLICT,
                format!(
                    "idempotency key {} maps to a different immutable request",
                    args.idempotency_key
                ),
            ));
        }
        let event = TaskEvent {
            event_id: String::new(),
            task_id: existing.task_id.clone(),
            root_id: existing.root_id.clone(),
            task_revision: existing.revision,
            event_type: TaskEventType::TaskCreated,
            actor: Some(args.creator.clone()),
            payload: json!({"replayed": true}),
            created_at: existing.created_at,
        };
        Ok(MutationOutcome {
            task: existing,
            event,
        })
    }

    // -----------------------------------------------------------------
    // Generic guarded mutation
    // -----------------------------------------------------------------

    /// Load-mutate-store under revision CAS, appending exactly one event.
    /// `mutate` sees the current snapshot and edits the mutable projection in
    /// place; immutable columns are never part of the UPDATE statement.
    pub async fn mutate_task<F>(
        &self,
        task_id: &str,
        actor: Option<&ActorRef>,
        event_type: TaskEventType,
        event_payload: Value,
        expected_revision: Option<u64>,
        mutate: F,
    ) -> Result<MutationOutcome>
    where
        F: FnOnce(&mut Task) -> Result<()>,
    {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let sql = self.render_sql("SELECT * FROM task WHERE task_id = ?");
        let row = sqlx::query(&sql)
            .bind(task_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Err(task_mgr_error(TASK_ERR_NOT_FOUND, task_id));
        };
        let mut task = task_from_row(row).map_err(db_err)?;
        if task.executor.kind() == TaskExecutorKind::HumanSet {
            let sql = self.render_sql(
                "SELECT user_id FROM task_assignee WHERE task_id = ? AND revoked_at IS NULL ORDER BY user_id",
            );
            let rows = sqlx::query(&sql)
                .bind(task_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(db_err)?;
            let assignees: Vec<String> = rows
                .into_iter()
                .map(|row| row.try_get::<String, _>("user_id").map_err(db_err))
                .collect::<Result<_>>()?;
            task.assignees = Some(assignees);
        }
        if let Some(expected) = expected_revision {
            if task.revision != expected {
                return Err(task_mgr_error(
                    TASK_ERR_REVISION_CONFLICT,
                    format!("expected revision {}, found {}", expected, task.revision),
                ));
            }
        }
        let before_revision = task.revision;
        mutate(&mut task)?;
        let now = now_ms();
        task.revision = before_revision + 1;
        task.updated_at = now;

        let affected = self
            .write_mutable_columns_tx(&mut tx, &task, before_revision)
            .await?;
        if affected == 0 {
            return Err(task_mgr_error(
                TASK_ERR_REVISION_CONFLICT,
                format!("task {} was modified concurrently", task_id),
            ));
        }
        let event = self
            .insert_event_tx(
                &mut tx,
                &task.task_id,
                &task.root_id,
                task.revision,
                event_type,
                actor,
                event_payload,
                now,
            )
            .await?;
        tx.commit().await.map_err(db_err)?;
        Ok(MutationOutcome { task, event })
    }

    async fn write_mutable_columns_tx(
        &self,
        tx: &mut Transaction<'_, Any>,
        task: &Task,
        expected_revision: u64,
    ) -> Result<u64> {
        let (executor_kind, target_id, runner_app_id, runner_instance_id) = match &task.executor {
            TaskExecutor::Unbound => (TaskExecutorKind::Unbound, None, None, None),
            TaskExecutor::App {
                target_id,
                app_id,
                app_instance_id,
            } => (
                TaskExecutorKind::App,
                target_id.clone(),
                Some(app_id.clone()),
                app_instance_id.clone(),
            ),
            TaskExecutor::HumanSet => (TaskExecutorKind::HumanSet, None, None, None),
        };
        let sql = self.render_sql(
            "UPDATE task SET
                executor_kind = ?, runner_target_id = ?, runner_instance_id = ?, runner_app_id = ?,
                runner_epoch = ?, phase = ?, wait_reason_json = ?, control_request_json = ?,
                control_profile_json = ?, progress_json = ?, message = ?, outcome = ?,
                result_json = ?, error_json = ?, completed_by_user_id = ?, completed_by_app_id = ?,
                revision = ?, updated_at = ?, completed_at = ?, archived_at = ?
            WHERE task_id = ? AND revision = ?",
        );
        let result = sqlx::query(&sql)
            .bind(executor_kind.to_string())
            .bind(target_id)
            .bind(runner_instance_id)
            .bind(runner_app_id)
            .bind(task.runner_epoch as i64)
            .bind(task.phase.to_string())
            .bind(
                task.wait_reason
                    .as_ref()
                    .map(|r| serde_json::to_string(r).unwrap_or_default()),
            )
            .bind(
                task.pending_control
                    .as_ref()
                    .map(|r| serde_json::to_string(r).unwrap_or_default()),
            )
            .bind(serde_json::to_string(&task.control_profile).unwrap_or_default())
            .bind(task.progress.as_ref().map(|p| p.to_string()))
            .bind(task.message.clone())
            .bind(task.outcome.map(|o| o.to_string()))
            .bind(task.result.as_ref().map(|r| r.to_string()))
            .bind(
                task.error
                    .as_ref()
                    .map(|e| serde_json::to_string(e).unwrap_or_default()),
            )
            .bind(task.completed_by.as_ref().map(|a| a.user_id.clone()))
            .bind(task.completed_by.as_ref().map(|a| a.app_id.clone()))
            .bind(task.revision as i64)
            .bind(task.updated_at as i64)
            .bind(task.completed_at.map(|v| v as i64))
            .bind(task.archived_at.map(|v| v as i64))
            .bind(&task.task_id)
            .bind(expected_revision as i64)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        Ok(result.rows_affected())
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_event_tx(
        &self,
        tx: &mut Transaction<'_, Any>,
        task_id: &str,
        root_id: &str,
        task_revision: u64,
        event_type: TaskEventType,
        actor: Option<&ActorRef>,
        payload: Value,
        now: u64,
    ) -> Result<TaskEvent> {
        let event_id = new_event_id(now);
        let sql = self.render_sql(
            "INSERT INTO task_event (event_id, task_id, root_id, task_revision, event_type, actor_user_id, actor_app_id, actor_instance_id, payload_json, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(&event_id)
            .bind(task_id)
            .bind(root_id)
            .bind(task_revision as i64)
            .bind(event_type.to_string())
            .bind(actor.map(|a| a.user_id.clone()))
            .bind(actor.map(|a| a.app_id.clone()))
            .bind(actor.and_then(|a| a.app_instance_id.clone()))
            .bind(payload.to_string())
            .bind(now as i64)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        Ok(TaskEvent {
            event_id,
            task_id: task_id.to_string(),
            root_id: root_id.to_string(),
            task_revision,
            event_type,
            actor: actor.cloned(),
            payload,
            created_at: now,
        })
    }

    // -----------------------------------------------------------------
    // Assignees & grants (revision-CAS'd through mutate_task callers)
    // -----------------------------------------------------------------

    /// Apply an assignee delta inside the same transaction pattern: the task
    /// row's revision CAS serializes assignee updates against result commits
    /// (doc §10.2).
    pub async fn update_assignees(
        &self,
        task_id: &str,
        actor: &ActorRef,
        add: &[String],
        remove: &[String],
        expected_revision: u64,
    ) -> Result<MutationOutcome> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let sql = self.render_sql("SELECT * FROM task WHERE task_id = ?");
        let row = sqlx::query(&sql)
            .bind(task_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Err(task_mgr_error(TASK_ERR_NOT_FOUND, task_id));
        };
        let mut task = task_from_row(row).map_err(db_err)?;
        if task.executor.kind() != TaskExecutorKind::HumanSet {
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                "update_assignees only applies to HumanSet tasks",
            ));
        }
        if task.phase.is_terminal() {
            return Err(task_mgr_error(
                TASK_ERR_ALREADY_COMPLETED,
                "task already terminal",
            ));
        }
        if task.revision != expected_revision {
            return Err(task_mgr_error(
                TASK_ERR_REVISION_CONFLICT,
                format!("expected revision {}, found {}", expected_revision, task.revision),
            ));
        }

        let sql = self.render_sql(
            "SELECT user_id FROM task_assignee WHERE task_id = ? AND revoked_at IS NULL",
        );
        let rows = sqlx::query(&sql)
            .bind(task_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(db_err)?;
        let mut current: Vec<String> = rows
            .into_iter()
            .map(|row| row.try_get::<String, _>("user_id").map_err(db_err))
            .collect::<Result<_>>()?;

        for user in remove {
            current.retain(|u| u != user);
        }
        for user in add {
            if !current.contains(user) {
                current.push(user.clone());
            }
        }
        if current.is_empty() {
            // Removing the last assignee is not expressible through a plain
            // update (doc §4.2) — executor-kind changes need Reassign.
            return Err(task_mgr_error(
                TASK_ERR_INVALID_PHASE,
                "a HumanSet task must keep at least one active assignee",
            ));
        }

        let now = now_ms();
        for user in remove {
            let sql = self.render_sql(
                "UPDATE task_assignee SET revoked_at = ? WHERE task_id = ? AND user_id = ? AND revoked_at IS NULL",
            );
            sqlx::query(&sql)
                .bind(now as i64)
                .bind(task_id)
                .bind(user)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
        for user in add {
            // Re-granting an existing row clears revoked_at (history stays in
            // task_event); fresh users get a new row.
            let sql = self.render_sql(
                "UPDATE task_assignee SET revoked_at = NULL, granted_by_user_id = ?, granted_by_app_id = ?, created_at = ? WHERE task_id = ? AND user_id = ?",
            );
            let updated = sqlx::query(&sql)
                .bind(&actor.user_id)
                .bind(&actor.app_id)
                .bind(now as i64)
                .bind(task_id)
                .bind(user)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            if updated.rows_affected() == 0 {
                let sql = self.render_sql(
                    "INSERT INTO task_assignee (task_id, user_id, granted_by_user_id, granted_by_app_id, created_at) VALUES (?, ?, ?, ?, ?)",
                );
                sqlx::query(&sql)
                    .bind(task_id)
                    .bind(user)
                    .bind(&actor.user_id)
                    .bind(&actor.app_id)
                    .bind(now as i64)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
        }

        task.revision += 1;
        task.updated_at = now;
        task.assignees = Some(current.clone());
        let affected = self
            .write_mutable_columns_tx(&mut tx, &task, expected_revision)
            .await?;
        if affected == 0 {
            return Err(task_mgr_error(
                TASK_ERR_REVISION_CONFLICT,
                format!("task {} was modified concurrently", task_id),
            ));
        }
        let event = self
            .insert_event_tx(
                &mut tx,
                task_id,
                &task.root_id.clone(),
                task.revision,
                TaskEventType::AssigneesChanged,
                Some(actor),
                json!({"add": add, "remove": remove, "active": current}),
                now,
            )
            .await?;
        tx.commit().await.map_err(db_err)?;
        Ok(MutationOutcome { task, event })
    }

    pub async fn insert_grant(
        &self,
        task_id: &str,
        actor: &ActorRef,
        spec: &TaskAclGrantSpec,
        expected_revision: u64,
    ) -> Result<(MutationOutcome, TaskAclGrant)> {
        let grant_id = new_grant_id();
        let now = now_ms();
        let grant = TaskAclGrant {
            grant_id: grant_id.clone(),
            task_id: task_id.to_string(),
            subject: spec.subject.clone(),
            actions: spec.actions.clone(),
            scope: spec.scope,
            data_scope: spec.data_scope,
            created_by: actor.clone(),
            created_at: now,
            revoked_at: None,
        };
        let grant_for_insert = grant.clone();
        let store = self;
        // Write the grant row inside the same transaction as the revision
        // bump: reuse mutate_task's guarded skeleton by doing the row insert
        // first in its own statement inside the transaction is not possible
        // through the closure, so grants use a bespoke transaction.
        let mut tx = store.pool.begin().await.map_err(db_err)?;
        let sql = store.render_sql("SELECT * FROM task WHERE task_id = ?");
        let row = sqlx::query(&sql)
            .bind(task_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Err(task_mgr_error(TASK_ERR_NOT_FOUND, task_id));
        };
        let mut task = task_from_row(row).map_err(db_err)?;
        if task.revision != expected_revision {
            return Err(task_mgr_error(
                TASK_ERR_REVISION_CONFLICT,
                format!("expected revision {}, found {}", expected_revision, task.revision),
            ));
        }
        let (subject_kind, relation, user_id, app_id, system_role) =
            grant_subject_columns(&grant_for_insert.subject);
        let sql = store.render_sql(
            "INSERT INTO task_acl_grant (grant_id, task_id, subject_kind, subject_relation, subject_user_id, subject_app_id, subject_system_role, actions_json, scope, data_scope, created_by_user_id, created_by_app_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(&grant_id)
            .bind(task_id)
            .bind(subject_kind)
            .bind(relation)
            .bind(user_id)
            .bind(app_id)
            .bind(system_role)
            .bind(serde_json::to_string(&grant_for_insert.actions).unwrap_or_default())
            .bind(grant_for_insert.scope.to_string())
            .bind(grant_for_insert.data_scope.to_string())
            .bind(&actor.user_id)
            .bind(&actor.app_id)
            .bind(now as i64)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;

        task.revision += 1;
        task.updated_at = now;
        let affected = store
            .write_mutable_columns_tx(&mut tx, &task, expected_revision)
            .await?;
        if affected == 0 {
            return Err(task_mgr_error(
                TASK_ERR_REVISION_CONFLICT,
                format!("task {} was modified concurrently", task_id),
            ));
        }
        let event = store
            .insert_event_tx(
                &mut tx,
                task_id,
                &task.root_id.clone(),
                task.revision,
                TaskEventType::AccessGranted,
                Some(actor),
                json!({
                    "grant_id": grant_id,
                    "subject": grant_for_insert.subject,
                    "actions": grant_for_insert.actions,
                    "scope": grant_for_insert.scope,
                    "data_scope": grant_for_insert.data_scope,
                }),
                now,
            )
            .await?;
        tx.commit().await.map_err(db_err)?;
        let mut task = task;
        self.attach_assignees(&mut task).await?;
        Ok((MutationOutcome { task, event }, grant))
    }

    pub async fn revoke_grant(
        &self,
        task_id: &str,
        actor: &ActorRef,
        grant_id: &str,
        expected_revision: u64,
    ) -> Result<MutationOutcome> {
        let mut tx = self.pool.begin().await.map_err(db_err)?;
        let sql = self.render_sql("SELECT * FROM task WHERE task_id = ?");
        let row = sqlx::query(&sql)
            .bind(task_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
        let Some(row) = row else {
            return Err(task_mgr_error(TASK_ERR_NOT_FOUND, task_id));
        };
        let mut task = task_from_row(row).map_err(db_err)?;
        if task.revision != expected_revision {
            return Err(task_mgr_error(
                TASK_ERR_REVISION_CONFLICT,
                format!("expected revision {}, found {}", expected_revision, task.revision),
            ));
        }
        let now = now_ms();
        let sql = self.render_sql(
            "UPDATE task_acl_grant SET revoked_at = ? WHERE grant_id = ? AND task_id = ? AND revoked_at IS NULL",
        );
        let updated = sqlx::query(&sql)
            .bind(now as i64)
            .bind(grant_id)
            .bind(task_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        if updated.rows_affected() == 0 {
            return Err(task_mgr_error(
                TASK_ERR_NOT_FOUND,
                format!("active grant {} on task {}", grant_id, task_id),
            ));
        }
        task.revision += 1;
        task.updated_at = now;
        let affected = self
            .write_mutable_columns_tx(&mut tx, &task, expected_revision)
            .await?;
        if affected == 0 {
            return Err(task_mgr_error(
                TASK_ERR_REVISION_CONFLICT,
                format!("task {} was modified concurrently", task_id),
            ));
        }
        let event = self
            .insert_event_tx(
                &mut tx,
                task_id,
                &task.root_id.clone(),
                task.revision,
                TaskEventType::AccessRevoked,
                Some(actor),
                json!({"grant_id": grant_id}),
                now,
            )
            .await?;
        tx.commit().await.map_err(db_err)?;
        let mut task = task;
        self.attach_assignees(&mut task).await?;
        Ok(MutationOutcome { task, event })
    }

    // -----------------------------------------------------------------
    // Schema registry
    // -----------------------------------------------------------------

    pub async fn register_schema(&self, def: &TaskSchemaDefinition) -> Result<TaskSchemaDefinition> {
        let now = now_ms();
        let sql = self.render_sql(
            "SELECT 1 FROM task_schema WHERE schema_id = ? AND schema_version = ?",
        );
        let exists = sqlx::query(&sql)
            .bind(&def.schema_id)
            .bind(def.schema_version as i64)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?
            .is_some();
        if exists {
            // Published revisions are immutable (doc §9.1): re-registering
            // the same (id, version) is only legal when nothing changed.
            let existing = self
                .get_schema(&def.schema_id, Some(def.schema_version))
                .await?;
            let same = existing.input_schema == def.input_schema
                && existing.output_schema == def.output_schema
                && existing.presentation_schema == def.presentation_schema
                && existing.allowed_executor_kinds == def.allowed_executor_kinds
                && existing.publisher_app_id == def.publisher_app_id;
            if !same {
                return Err(task_mgr_error(
                    TASK_ERR_IDEMPOTENCY_CONFLICT,
                    format!(
                        "task schema {}@{} already published with different content",
                        def.schema_id, def.schema_version
                    ),
                ));
            }
            return Ok(existing);
        }
        let sql = self.render_sql(
            "INSERT INTO task_schema (schema_id, schema_version, input_schema_json, output_schema_json, presentation_schema_json, executor_kinds_json, user_creatable, publisher_app_id, enabled, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        );
        sqlx::query(&sql)
            .bind(&def.schema_id)
            .bind(def.schema_version as i64)
            .bind(def.input_schema.to_string())
            .bind(def.output_schema.to_string())
            .bind(def.presentation_schema.as_ref().map(|p| p.to_string()))
            .bind(serde_json::to_string(&def.allowed_executor_kinds).unwrap_or_default())
            .bind(if def.user_creatable { 1i64 } else { 0i64 })
            .bind(&def.publisher_app_id)
            .bind(if def.enabled { 1i64 } else { 0i64 })
            .bind(now as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        let mut stored = def.clone();
        stored.created_at = now;
        Ok(stored)
    }

    /// `version = None` -> the highest enabled revision.
    pub async fn get_schema(
        &self,
        schema_id: &str,
        version: Option<u32>,
    ) -> Result<TaskSchemaDefinition> {
        let (sql, bind_version) = match version {
            Some(v) => (
                "SELECT * FROM task_schema WHERE schema_id = ? AND schema_version = ?",
                Some(v as i64),
            ),
            None => (
                "SELECT * FROM task_schema WHERE schema_id = ? AND enabled = 1 ORDER BY schema_version DESC LIMIT 1",
                None,
            ),
        };
        let sql = self.render_sql(sql);
        let mut query = sqlx::query(&sql).bind(schema_id);
        if let Some(v) = bind_version {
            query = query.bind(v);
        }
        let row = query.fetch_optional(&self.pool).await.map_err(db_err)?;
        let Some(row) = row else {
            return Err(task_mgr_error(
                TASK_ERR_SCHEMA_NOT_FOUND,
                format!("{}@{:?}", schema_id, version),
            ));
        };
        schema_from_row(row).map_err(db_err)
    }

    pub async fn list_schemas(
        &self,
        user_creatable_only: bool,
        include_disabled: bool,
    ) -> Result<Vec<TaskSchemaDefinition>> {
        let mut sql = String::from("SELECT * FROM task_schema");
        let mut conditions = Vec::new();
        if user_creatable_only {
            conditions.push("user_creatable = 1");
        }
        if !include_disabled {
            conditions.push("enabled = 1");
        }
        if !conditions.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&conditions.join(" AND "));
        }
        sql.push_str(" ORDER BY schema_id, schema_version");
        let sql = self.render_sql(&sql);
        let rows = sqlx::query(&sql).fetch_all(&self.pool).await.map_err(db_err)?;
        rows.into_iter()
            .map(|row| schema_from_row(row).map_err(db_err))
            .collect()
    }

    pub async fn set_schema_enabled(
        &self,
        schema_id: &str,
        schema_version: u32,
        enabled: bool,
    ) -> Result<TaskSchemaDefinition> {
        let sql = self.render_sql(
            "UPDATE task_schema SET enabled = ? WHERE schema_id = ? AND schema_version = ?",
        );
        let updated = sqlx::query(&sql)
            .bind(if enabled { 1i64 } else { 0i64 })
            .bind(schema_id)
            .bind(schema_version as i64)
            .execute(&self.pool)
            .await
            .map_err(db_err)?;
        if updated.rows_affected() == 0 {
            return Err(task_mgr_error(
                TASK_ERR_SCHEMA_NOT_FOUND,
                format!("{}@{}", schema_id, schema_version),
            ));
        }
        self.get_schema(schema_id, Some(schema_version)).await
    }

    // -----------------------------------------------------------------
    // Events & notes
    // -----------------------------------------------------------------

    pub async fn list_events(
        &self,
        task_id: Option<&str>,
        root_id: Option<&str>,
        after_event_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<TaskEvent>> {
        let mut sql = String::from("SELECT * FROM task_event WHERE ");
        let key_value = match (task_id, root_id) {
            (Some(task_id), None) => {
                sql.push_str("task_id = ?");
                task_id
            }
            (None, Some(root_id)) => {
                sql.push_str("root_id = ?");
                root_id
            }
            _ => {
                return Err(RPCErrors::ParseRequestError(
                    "exactly one of task_id / root_id is required".to_string(),
                ))
            }
        };
        if after_event_id.is_some() {
            sql.push_str(" AND event_id > ?");
        }
        let limit = limit.clamp(1, 500);
        sql.push_str(" ORDER BY event_id ASC LIMIT ");
        sql.push_str(&limit.to_string());
        let sql = self.render_sql(&sql);
        let mut query = sqlx::query(&sql).bind(key_value);
        if let Some(cursor) = after_event_id {
            query = query.bind(cursor);
        }
        let rows = query.fetch_all(&self.pool).await.map_err(db_err)?;
        rows.into_iter()
            .map(|row| event_from_row(row).map_err(db_err))
            .collect()
    }

    pub async fn add_task_note(&self, note: &TaskNote) -> Result<TaskNote> {
        let sql = self.render_sql(
            "INSERT INTO task_note (task_id, note_type, content, data, author_user_id, author_app_id, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        );
        let row = sqlx::query(&sql)
            .bind(&note.task_id)
            .bind(&note.note_type)
            .bind(&note.content)
            .bind(note.data.to_string())
            .bind(&note.author_user_id)
            .bind(&note.author_app_id)
            .bind(note.created_at as i64)
            .bind(note.updated_at as i64)
            .fetch_one(&self.pool)
            .await
            .map_err(db_err)?;
        let id: i64 = row.try_get("id").map_err(db_err)?;
        let mut stored = note.clone();
        stored.id = id;
        Ok(stored)
    }

    pub async fn list_task_notes(&self, task_id: &str) -> Result<Vec<TaskNote>> {
        let sql = self.render_sql(
            "SELECT * FROM task_note WHERE task_id = ? ORDER BY created_at ASC, id ASC",
        );
        let rows = sqlx::query(&sql)
            .bind(task_id)
            .fetch_all(&self.pool)
            .await
            .map_err(db_err)?;
        rows.into_iter()
            .map(|row| note_from_row(row).map_err(db_err))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Row decoding
// ---------------------------------------------------------------------------

fn parse_json_column(value: Option<String>) -> Value {
    value
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null)
}

pub fn task_from_row(row: AnyRow) -> DbResult<Task> {
    let task_id: String = row.try_get("task_id")?;
    let schema_id: String = row.try_get("schema_id")?;
    let schema_version: i64 = row.try_get("schema_version")?;
    let name: String = row.try_get("name")?;
    let input_json: String = row.try_get("input_json")?;
    let input_digest: String = row.try_get("input_digest")?;
    let result_json: Option<String> = row.try_get("result_json")?;
    let error_json: Option<String> = row.try_get("error_json")?;
    let creator_user_id: String = row.try_get("creator_user_id")?;
    let creator_app_id: String = row.try_get("creator_app_id")?;
    let creator_instance_id: Option<String> = row.try_get("creator_instance_id")?;
    let idempotency_key: String = row.try_get("idempotency_key")?;
    let origin_kind: Option<String> = row.try_get("origin_kind")?;
    let origin_id: Option<String> = row.try_get("origin_id")?;
    let parent_id: Option<String> = row.try_get("parent_id")?;
    let root_id: String = row.try_get("root_id")?;
    let child_policy_json: String = row.try_get("child_control_policy_json")?;
    let retry_of: Option<String> = row.try_get("retry_of")?;
    let supersedes: Option<String> = row.try_get("supersedes")?;
    let executor_kind: String = row.try_get("executor_kind")?;
    let runner_target_id: Option<String> = row.try_get("runner_target_id")?;
    let runner_instance_id: Option<String> = row.try_get("runner_instance_id")?;
    let runner_app_id: Option<String> = row.try_get("runner_app_id")?;
    let runner_epoch: i64 = row.try_get("runner_epoch")?;
    let phase: String = row.try_get("phase")?;
    let wait_reason_json: Option<String> = row.try_get("wait_reason_json")?;
    let control_request_json: Option<String> = row.try_get("control_request_json")?;
    let control_profile_json: String = row.try_get("control_profile_json")?;
    let progress_json: Option<String> = row.try_get("progress_json")?;
    let message: Option<String> = row.try_get("message")?;
    let outcome: Option<String> = row.try_get("outcome")?;
    let completed_by_user_id: Option<String> = row.try_get("completed_by_user_id")?;
    let completed_by_app_id: Option<String> = row.try_get("completed_by_app_id")?;
    let policy_preset: String = row.try_get("policy_preset")?;
    let permission_boundary: i64 = row.try_get("permission_boundary")?;
    let revision: i64 = row.try_get("revision")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;
    let completed_at: Option<i64> = row.try_get("completed_at")?;
    let archived_at: Option<i64> = row.try_get("archived_at")?;

    let executor = match executor_kind.as_str() {
        "App" => TaskExecutor::App {
            target_id: runner_target_id,
            app_id: runner_app_id.unwrap_or_default(),
            app_instance_id: runner_instance_id,
        },
        "HumanSet" => TaskExecutor::HumanSet,
        _ => TaskExecutor::Unbound,
    };
    let phase = TaskPhase::from_str(phase.as_str()).unwrap_or(TaskPhase::Promised);
    let outcome = outcome.and_then(|o| TaskOutcome::from_str(o.as_str()).ok());
    let control_profile = serde_json::from_str::<TaskControlProfile>(&control_profile_json)
        .unwrap_or_else(|_| TaskControlProfile::baseline(created_at.max(0) as u64));
    let completed_by = match (completed_by_user_id, completed_by_app_id) {
        (Some(user_id), Some(app_id)) => Some(ActorRef {
            user_id,
            app_id,
            app_instance_id: None,
        }),
        _ => None,
    };
    let origin_ref = match (origin_kind, origin_id) {
        (Some(kind), Some(id)) => Some(TaskOriginRef { kind, id }),
        _ => None,
    };

    Ok(Task {
        task_id,
        name,
        parent_id,
        root_id,
        child_control_policy: serde_json::from_str(&child_policy_json).unwrap_or_default(),
        schema_id,
        schema_version: schema_version.max(0) as u32,
        input: serde_json::from_str(&input_json).unwrap_or(Value::Null),
        input_digest,
        creator: ActorRef {
            user_id: creator_user_id,
            app_id: creator_app_id,
            app_instance_id: creator_instance_id,
        },
        idempotency_key,
        origin_ref,
        retry_of,
        supersedes,
        executor,
        runner_epoch: runner_epoch.max(0) as u64,
        assignees: None,
        phase,
        wait_reason: wait_reason_json.and_then(|s| serde_json::from_str(&s).ok()),
        pending_control: control_request_json.and_then(|s| serde_json::from_str(&s).ok()),
        control_profile,
        progress: progress_json.and_then(|s| serde_json::from_str(&s).ok()),
        message,
        outcome,
        result: result_json.and_then(|s| serde_json::from_str(&s).ok()),
        error: error_json.and_then(|s| serde_json::from_str(&s).ok()),
        completed_by,
        policy_preset,
        permission_boundary: permission_boundary != 0,
        revision: revision.max(1) as u64,
        data_scope: None,
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
        completed_at: completed_at.map(|v| v.max(0) as u64),
        archived_at: archived_at.map(|v| v.max(0) as u64),
    })
}

fn grant_subject_columns(
    subject: &TaskGrantSubject,
) -> (
    &'static str,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    match subject {
        TaskGrantSubject::RootCreator => ("relation", Some("RootCreator".into()), None, None, None),
        TaskGrantSubject::Creator => ("relation", Some("Creator".into()), None, None, None),
        TaskGrantSubject::Runner => ("relation", Some("Runner".into()), None, None, None),
        TaskGrantSubject::Assignees => ("relation", Some("Assignees".into()), None, None, None),
        TaskGrantSubject::User { user_id } => ("user", None, Some(user_id.clone()), None, None),
        TaskGrantSubject::App { app_id } => ("app", None, None, Some(app_id.clone()), None),
        TaskGrantSubject::Principal { user_id, app_id } => (
            "principal",
            None,
            Some(user_id.clone()),
            Some(app_id.clone()),
            None,
        ),
        TaskGrantSubject::SystemRole { role } => {
            ("system_role", None, None, None, Some(role.clone()))
        }
    }
}

fn grant_from_row(row: AnyRow) -> DbResult<TaskAclGrant> {
    let grant_id: String = row.try_get("grant_id")?;
    let task_id: String = row.try_get("task_id")?;
    let subject_kind: String = row.try_get("subject_kind")?;
    let subject_relation: Option<String> = row.try_get("subject_relation")?;
    let subject_user_id: Option<String> = row.try_get("subject_user_id")?;
    let subject_app_id: Option<String> = row.try_get("subject_app_id")?;
    let subject_system_role: Option<String> = row.try_get("subject_system_role")?;
    let actions_json: String = row.try_get("actions_json")?;
    let scope: String = row.try_get("scope")?;
    let data_scope: String = row.try_get("data_scope")?;
    let created_by_user_id: String = row.try_get("created_by_user_id")?;
    let created_by_app_id: String = row.try_get("created_by_app_id")?;
    let created_at: i64 = row.try_get("created_at")?;
    let revoked_at: Option<i64> = row.try_get("revoked_at")?;

    let subject = match subject_kind.as_str() {
        "relation" => match subject_relation.as_deref() {
            Some("RootCreator") => TaskGrantSubject::RootCreator,
            Some("Creator") => TaskGrantSubject::Creator,
            Some("Runner") => TaskGrantSubject::Runner,
            Some("Assignees") => TaskGrantSubject::Assignees,
            other => {
                warn!("task_acl_grant {}: unknown relation {:?}", grant_id, other);
                TaskGrantSubject::Creator
            }
        },
        "user" => TaskGrantSubject::User {
            user_id: subject_user_id.unwrap_or_default(),
        },
        "app" => TaskGrantSubject::App {
            app_id: subject_app_id.unwrap_or_default(),
        },
        "principal" => TaskGrantSubject::Principal {
            user_id: subject_user_id.unwrap_or_default(),
            app_id: subject_app_id.unwrap_or_default(),
        },
        _ => TaskGrantSubject::SystemRole {
            role: subject_system_role.unwrap_or_default(),
        },
    };

    Ok(TaskAclGrant {
        grant_id,
        task_id,
        subject,
        actions: serde_json::from_str(&actions_json).unwrap_or_default(),
        scope: scope.parse().unwrap_or(TaskGrantScope::SelfOnly),
        data_scope: data_scope.parse().unwrap_or(TaskDataScope::MetaOnly),
        created_by: ActorRef {
            user_id: created_by_user_id,
            app_id: created_by_app_id,
            app_instance_id: None,
        },
        created_at: created_at.max(0) as u64,
        revoked_at: revoked_at.map(|v| v.max(0) as u64),
    })
}

fn schema_from_row(row: AnyRow) -> DbResult<TaskSchemaDefinition> {
    let schema_id: String = row.try_get("schema_id")?;
    let schema_version: i64 = row.try_get("schema_version")?;
    let input_schema_json: String = row.try_get("input_schema_json")?;
    let output_schema_json: String = row.try_get("output_schema_json")?;
    let presentation_schema_json: Option<String> = row.try_get("presentation_schema_json")?;
    let executor_kinds_json: String = row.try_get("executor_kinds_json")?;
    let user_creatable: i64 = row.try_get("user_creatable")?;
    let publisher_app_id: String = row.try_get("publisher_app_id")?;
    let enabled: i64 = row.try_get("enabled")?;
    let created_at: i64 = row.try_get("created_at")?;

    Ok(TaskSchemaDefinition {
        schema_id,
        schema_version: schema_version.max(0) as u32,
        input_schema: serde_json::from_str(&input_schema_json).unwrap_or(json!({})),
        output_schema: serde_json::from_str(&output_schema_json).unwrap_or(json!({})),
        presentation_schema: presentation_schema_json.and_then(|s| serde_json::from_str(&s).ok()),
        allowed_executor_kinds: serde_json::from_str(&executor_kinds_json).unwrap_or_default(),
        user_creatable: user_creatable != 0,
        publisher_app_id,
        enabled: enabled != 0,
        created_at: created_at.max(0) as u64,
    })
}

fn event_from_row(row: AnyRow) -> DbResult<TaskEvent> {
    let event_id: String = row.try_get("event_id")?;
    let task_id: String = row.try_get("task_id")?;
    let root_id: String = row.try_get("root_id")?;
    let task_revision: i64 = row.try_get("task_revision")?;
    let event_type: String = row.try_get("event_type")?;
    let actor_user_id: Option<String> = row.try_get("actor_user_id")?;
    let actor_app_id: Option<String> = row.try_get("actor_app_id")?;
    let actor_instance_id: Option<String> = row.try_get("actor_instance_id")?;
    let payload_json: String = row.try_get("payload_json")?;
    let created_at: i64 = row.try_get("created_at")?;

    let actor = match (actor_user_id, actor_app_id) {
        (Some(user_id), Some(app_id)) => Some(ActorRef {
            user_id,
            app_id,
            app_instance_id: actor_instance_id,
        }),
        _ => None,
    };
    let event_type = serde_json::from_value::<TaskEventType>(Value::String(event_type.clone()))
        .unwrap_or(TaskEventType::PhaseChanged);

    Ok(TaskEvent {
        event_id,
        task_id,
        root_id,
        task_revision: task_revision.max(0) as u64,
        event_type,
        actor,
        payload: parse_json_column(Some(payload_json)),
        created_at: created_at.max(0) as u64,
    })
}

fn note_from_row(row: AnyRow) -> DbResult<TaskNote> {
    let id: i64 = row.try_get("id")?;
    let task_id: String = row.try_get("task_id")?;
    let note_type: String = row.try_get("note_type")?;
    let content: String = row.try_get("content")?;
    let data_str: Option<String> = row.try_get("data")?;
    let author_user_id: String = row.try_get("author_user_id")?;
    let author_app_id: String = row.try_get("author_app_id")?;
    let created_at: i64 = row.try_get("created_at")?;
    let updated_at: i64 = row.try_get("updated_at")?;

    Ok(TaskNote {
        id,
        task_id,
        note_type,
        content,
        data: data_str
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| json!({})),
        author_user_id,
        author_app_id,
        created_at: created_at.max(0) as u64,
        updated_at: updated_at.max(0) as u64,
    })
}

// ---------------------------------------------------------------------------
// Cursor & SQL helpers
// ---------------------------------------------------------------------------

fn make_cursor(created_at: u64, task_id: &str) -> String {
    format!("{}:{}", created_at, task_id)
}

fn parse_cursor(cursor: &str) -> Result<(i64, String)> {
    let (created_at, task_id) = cursor.split_once(':').ok_or_else(|| {
        RPCErrors::ParseRequestError(format!("invalid cursor: {}", cursor))
    })?;
    let created_at = created_at
        .parse::<i64>()
        .map_err(|_| RPCErrors::ParseRequestError(format!("invalid cursor: {}", cursor)))?;
    Ok((created_at, task_id.to_string()))
}

pub(crate) fn rewrite_placeholders_to_dollar(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut idx = 0u32;
    let mut in_single = false;
    let mut in_double = false;
    for ch in sql.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(ch);
            }
            '?' if !in_single && !in_double => {
                idx += 1;
                out.push('$');
                out.push_str(&idx.to_string());
            }
            _ => out.push(ch),
        }
    }
    out
}

pub(crate) fn split_sql_statements(ddl: &str) -> Vec<String> {
    let mut stmts = Vec::new();
    let mut buf = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for ch in ddl.chars() {
        match ch {
            '\'' if !in_double => {
                in_single = !in_single;
                buf.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                buf.push(ch);
            }
            ';' if !in_single && !in_double => {
                let trimmed = buf.trim();
                if !trimmed.is_empty() {
                    stmts.push(trimmed.to_string());
                }
                buf.clear();
            }
            _ => buf.push(ch),
        }
    }
    let trimmed = buf.trim();
    if !trimmed.is_empty() {
        stmts.push(trimmed.to_string());
    }
    stmts
}
