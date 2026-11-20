//! SQLite is never opened at its production path: even mode=ro may create -shm.
//! Copy the stopped main file and journal/WAL, then let SQLite recover the private copy.
use super::*;
use buckyos_api::*;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::Deserialize;

macro_rules! source_sql {
    ($path:literal) => {
        include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../", $path))
    };
}

#[derive(Deserialize)]
struct Spec {
    spec_config: Config,
}
#[derive(Deserialize)]
struct Config {
    #[serde(default)]
    rdb_instances: BTreeMap<String, RdbInstanceConfig>,
}

type InstanceKey = (String, String);
fn key(service: &str, id: &str) -> InstanceKey {
    (service.into(), id.into())
}

pub(super) fn discover(ctx: &mut Context) -> Result<(), Failure> {
    let defaults = [
        (
            TASK_MANAGER_SERVICE_NAME,
            TASK_MANAGER_RDB_INSTANCE_ID,
            task_manager_default_rdb_instance_config(),
        ),
        (
            TASK_MANAGER_SERVICE_NAME,
            TASK_DISPATCHER_RDB_INSTANCE_ID,
            task_dispatcher_default_rdb_instance_config(),
        ),
        (
            MSG_CENTER_SERVICE_NAME,
            MSG_CENTER_RDB_INSTANCE_ID,
            msg_center_default_rdb_instance_config(),
        ),
        (
            REPO_SERVICE_SERVICE_NAME,
            REPO_SERVICE_RDB_INSTANCE_ID,
            repo_service_default_rdb_instance_config(),
        ),
        (
            "aicc",
            AICC_USAGE_LOG_RDB_INSTANCE_ID,
            aicc_usage_log_default_rdb_instance_config(),
        ),
        (
            HOMESTATION_SERVICE_NAME,
            HOMESTATION_RDB_INSTANCE_ID,
            homestation_default_rdb_instance_config(),
        ),
        (
            NFS_SERVER_SERVICE_NAME,
            BUCKET_INDEX_RDB_INSTANCE_ID,
            bucket_index_rdb_instance_config(),
        ),
    ];
    let mut claimed = BTreeSet::new();
    for (k, v) in &ctx.snapshot {
        let Ok(name) = std::str::from_utf8(k) else {
            continue;
        };
        let parts: Vec<_> = name.split('/').collect();
        let (owner, app) = match parts.as_slice() {
            ["services", id, "spec"] => (None, *id),
            ["users", u, "apps", id, "spec"] => (Some(*u), *id),
            _ => continue,
        };
        let config: Spec = serde_json::from_slice(v).map_err(|e| ctx.unsupported(name, e))?;
        for (id, cfg) in &config.spec_config.rdb_instances {
            let paths = offline_rdb_paths(ctx.root, app, owner, id, cfg)
                .map_err(|e| ctx.unsupported(format!("{name}/{id}"), e))?;
            let known = defaults.iter().find(|(service, instance, _)| {
                owner.is_none() && *service == app && *instance == id
            });
            if let Some((_, _, expected)) = known {
                // These owners open named partitions with get_rdb_instance_in.
                // Others use get_rdb_instance, which accepts any single partition.
                if matches!(
                    id.as_str(),
                    TASK_MANAGER_RDB_INSTANCE_ID
                        | MSG_CENTER_RDB_INSTANCE_ID
                        | REPO_SERVICE_RDB_INSTANCE_ID
                ) {
                    if expected
                        .partitions
                        .iter()
                        .any(|p| !cfg.partitions.contains(p))
                    {
                        return Err(
                            ctx.unsupported(name, format!("missing required partition for {id}"))
                        );
                    }
                } else if paths.len() != 1 {
                    return Err(ctx.unsupported(name, format!("ambiguous partition for {id}")));
                }
            }
            for (_, path) in paths {
                if !claimed.insert(path.clone()) {
                    return Err(ctx.unsupported(path.display(), "overlapping instance paths"));
                }
                if known.is_some() {
                    ctx.rdb.entry(key(app, id)).or_default().push(path);
                } else if ctx.exists(&path)? {
                    return Err(
                        ctx.unsupported(path.display(), format!("no data reader for {name}/{id}"))
                    );
                }
            }
        }
    }
    for (service, id, cfg) in defaults {
        let k = key(service, id);
        if ctx.rdb.contains_key(&k) {
            continue;
        }
        // An interrupted install can leave the owned DB with missing binary/spec.
        let paths =
            offline_rdb_paths(ctx.root, service, None, id, &cfg).map_err(|e| ctx.error(id, e))?;
        ctx.rdb
            .insert(k, paths.into_iter().map(|(_, p)| p).collect());
    }
    Ok(())
}

struct Database {
    src: PathBuf,
    staged: PathBuf,
    conn: Connection,
}

fn suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}

impl Database {
    fn copy(ctx: &mut Context, src: &Path) -> Result<Option<Self>, Failure> {
        ctx.own(src)?;
        let exists = ctx.exists(src)?;
        let staged = ctx.copy_path();
        for ext in ["-journal", "-wal", "-shm"] {
            let p = suffix(src, ext);
            if ctx.exists(&p)? {
                if !exists {
                    return Err(
                        ctx.error(p.display(), "SQLite sidecar exists without main database")
                    );
                }
                if ext != "-shm" {
                    copy_tree(&p, &suffix(&staged, ext)).map_err(|e| ctx.error(p.display(), e))?;
                }
            }
        }
        if !exists {
            if !src.starts_with(ctx.root) {
                return Err(ctx.error(
                    src.display(),
                    "configured external database unavailable; mount its volume before updating",
                ));
            }
            return Ok(None);
        }
        let meta = fs::metadata(src).map_err(|e| ctx.error(src.display(), e))?;
        if !meta.is_file() || meta.len() < 100 {
            return Err(
                ctx.unsupported(src.display(), "existing file has no complete SQLite header")
            );
        }
        copy_tree(src, &staged).map_err(|e| ctx.error(src.display(), e))?;
        let conn = Connection::open_with_flags(&staged, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .map_err(|e| ctx.error(src.display(), e))?;
        conn.execute_batch("PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON")
            .map_err(|e| ctx.error(src.display(), e))?;
        let db = Self {
            src: src.to_owned(),
            staged,
            conn,
        };
        db.verify(ctx)?;
        Ok(Some(db))
    }

    fn verify(&self, ctx: &Context) -> Result<(), Failure> {
        let result: String = self
            .conn
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))
            .map_err(|e| ctx.error(self.src.display(), e))?;
        if result != "ok" {
            return Err(ctx.error(self.src.display(), result));
        }
        let violation = self
            .conn
            .query_row("PRAGMA foreign_key_check", [], |r| r.get::<_, String>(0))
            .optional()
            .map_err(|e| ctx.error(self.src.display(), e))?;
        if let Some(table) = violation {
            return Err(ctx.error(self.src.display(), format!("broken foreign key in {table}")));
        }
        Ok(())
    }
}

type Layout = BTreeMap<(String, String), String>;
fn layout(conn: &Connection) -> rusqlite::Result<Layout> {
    let mut stmt = conn.prepare(
        "SELECT type,name,sql FROM sqlite_schema WHERE sql IS NOT NULL AND name NOT GLOB 'sqlite_*'",
    )?;
    let mut layout = Layout::new();
    for row in stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (kind, name, sql) = row?;
        // SQLite retains token spelling and whitespace in sqlite_schema. The
        // service drivers differ in indentation and whether SQL comments survive.
        layout.insert((kind, name), canonical_sql(&sql));
    }
    Ok(layout)
}

fn canonical_sql(sql: &str) -> String {
    let mut out = String::new();
    let mut chars = sql.chars().peekable();
    let mut quote = None;
    let mut space = false;
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == q {
                quote = None;
            }
        } else if c == '-' && chars.peek() == Some(&'-') {
            for next in chars.by_ref() {
                if next == '\n' {
                    break;
                }
            }
            space = true;
        } else if c.is_whitespace() {
            space = true;
        } else {
            if space
                && (c.is_alphanumeric() || c == '_')
                && out.ends_with(|last: char| last.is_alphanumeric() || last == '_')
            {
                out.push(' ');
            }
            space = false;
            if matches!(c, '\'' | '"' | '`') {
                quote = Some(c);
            }
            out.push(c);
        }
    }
    out
}

fn reference(ctx: &Context, ddl: &[&str]) -> Result<Layout, Failure> {
    let conn = Connection::open_in_memory().map_err(|e| ctx.error("compiled schema", e))?;
    for sql in ddl {
        conn.execute_batch(sql)
            .map_err(|e| ctx.error("compiled schema", e))?;
    }
    layout(&conn).map_err(|e| ctx.error("compiled schema", e))
}
fn check_layout(ctx: &Context, db: &Database, expected: &Layout) -> Result<(), Failure> {
    let actual = layout(&db.conn).map_err(|e| ctx.error(db.src.display(), e))?;
    if let Some((kind, name)) = expected
        .keys()
        .chain(actual.keys())
        .find(|k| expected.get(*k) != actual.get(*k))
    {
        return Err(ctx.unsupported(
            db.src.display(),
            format!("unrecognized layout: {kind} {name} differs from the target schema"),
        ));
    }
    Ok(())
}
fn verify_schema(
    ctx: &Context,
    db: &Database,
    target: i64,
    query: Option<&str>,
    expected: &Layout,
) -> Result<(), Failure> {
    let actual: i64 = if let Some(sql) = query {
        let marker = db
            .conn
            .query_row(sql, [], |r| r.get::<_, rusqlite::types::Value>(0))
            .map_err(|e| {
                ctx.unsupported(
                    db.src.display(),
                    format!("cannot read required schema marker, target {target}: {e}"),
                )
            })?;
        match marker {
            rusqlite::types::Value::Integer(n) => n,
            rusqlite::types::Value::Text(s) if s == target.to_string() => target,
            rusqlite::types::Value::Text(s) => {
                return Err(ctx.unsupported(
                    db.src.display(),
                    format!("actual schema {s:?}, target {target}"),
                ));
            }
            _ => {
                return Err(ctx.unsupported(
                    db.src.display(),
                    format!("invalid schema marker type, target {target}"),
                ))
            }
        }
    } else {
        db.conn
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .map_err(|e| ctx.error(db.src.display(), e))?
    };
    if actual != target {
        return Err(ctx.unsupported(
            db.src.display(),
            format!("actual schema {actual}, target {target}"),
        ));
    }
    check_layout(ctx, db, expected)
}

fn stores(
    ctx: &mut Context,
    paths: Vec<PathBuf>,
    target: u64,
    query: Option<&str>,
    ddl: &[&str],
) -> Result<(), Failure> {
    let mut expected = None;
    for path in paths {
        if let Some(db) = Database::copy(ctx, &path)? {
            let schema = match expected.as_ref() {
                Some(s) => s,
                None => expected.insert(reference(ctx, ddl)?),
            };
            verify_schema(ctx, &db, target as i64, query, schema)?;
        }
    }
    Ok(())
}
fn rdb(
    ctx: &mut Context,
    service: &str,
    id: &str,
    target: u64,
    query: Option<&str>,
    ddl: &[&str],
) -> Result<(), Failure> {
    let paths = ctx
        .rdb
        .remove(&key(service, id))
        .ok_or_else(|| ctx.error(id, "unresolved registered instance"))?;
    stores(ctx, paths, target, query, ddl)
}

pub(super) fn tasks(ctx: &mut Context) -> Result<(), Failure> {
    rdb(
        ctx,
        TASK_MANAGER_SERVICE_NAME,
        TASK_MANAGER_RDB_INSTANCE_ID,
        TASK_MANAGER_RDB_SCHEMA_VERSION,
        Some(TASK_MANAGER_READ_PHYSICAL_SCHEMA),
        &[TASK_MANAGER_RDB_SCHEMA_SQLITE],
    )
}
pub(super) fn messages(ctx: &mut Context) -> Result<(), Failure> {
    rdb(
        ctx,
        MSG_CENTER_SERVICE_NAME,
        MSG_CENTER_RDB_INSTANCE_ID,
        MSG_CENTER_RDB_SCHEMA_VERSION,
        None,
        &[
            MSG_CENTER_RDB_SCHEMA_SQLITE,
            source_sql!("frame/msg_center/src/groups.sql"),
            source_sql!("frame/msg_center/src/notify.sql"),
        ],
    )
}
pub(super) fn dispatcher(ctx: &mut Context) -> Result<(), Failure> {
    rdb(
        ctx,
        TASK_MANAGER_SERVICE_NAME,
        TASK_DISPATCHER_RDB_INSTANCE_ID,
        TASK_DISPATCHER_RDB_SCHEMA_VERSION,
        None,
        &[TASK_DISPATCHER_RDB_SCHEMA_SQLITE],
    )
}
pub(super) fn repo(ctx: &mut Context) -> Result<(), Failure> {
    rdb(
        ctx,
        REPO_SERVICE_SERVICE_NAME,
        REPO_SERVICE_RDB_INSTANCE_ID,
        REPO_SERVICE_RDB_SCHEMA_VERSION,
        None,
        &[REPO_SERVICE_RDB_SCHEMA_SQLITE],
    )
}
pub(super) fn aicc(ctx: &mut Context) -> Result<(), Failure> {
    rdb(
        ctx,
        "aicc",
        AICC_USAGE_LOG_RDB_INSTANCE_ID,
        3,
        Some("SELECT schema_version FROM aicc_schema_meta WHERE schema_key='aicc'"),
        &[
            source_sql!("frame/aicc/src/storage/schema.sql"),
            source_sql!("frame/aicc/src/storage/quota.sql"),
        ],
    )
}
pub(super) fn home(ctx: &mut Context) -> Result<(), Failure> {
    rdb(
        ctx,
        HOMESTATION_SERVICE_NAME,
        HOMESTATION_RDB_INSTANCE_ID,
        HOMESTATION_RDB_SCHEMA_VERSION,
        Some("SELECT value FROM meta WHERE key='schema_version'"),
        &[HOMESTATION_RDB_SCHEMA_SQLITE],
    )
}
pub(super) fn bucket(ctx: &mut Context) -> Result<(), Failure> {
    let paths = ctx
        .rdb
        .remove(&key(NFS_SERVER_SERVICE_NAME, BUCKET_INDEX_RDB_INSTANCE_ID))
        .ok_or_else(|| ctx.error(BUCKET_INDEX_RDB_INSTANCE_ID, "unresolved bucket index"))?;
    for path in paths {
        if let Some(db) = Database::copy(ctx, &path)? {
            let expected = reference(ctx, &[bucket_store::db::SCHEMA])?;
            verify_schema(ctx, &db, 0, None, &expected)?;
            ctx.editor
                .discover(&db.conn)
                .map_err(|e| ctx.error(path.display(), e))?;
        }
    }
    Ok(())
}
pub(super) fn families(ctx: &mut Context) -> Result<(), Failure> {
    stores(
        ctx,
        vec![ctx.root.join("data/var/verify-hub/sessions.db")],
        1,
        None,
        &[source_sql!("kernel/verify_hub/src/families.sql")],
    )
}
pub(super) fn workflow(ctx: &mut Context) -> Result<(), Failure> {
    stores(
        ctx,
        vec![ctx.root.join("data/srv/workflow/workflow.db")],
        1,
        None,
        &[source_sql!("kernel/workflow/src/workflow.sql")],
    )
}
pub(super) fn filedb(ctx: &mut Context) -> Result<(), Failure> {
    stores(
        ctx,
        vec![ctx.root.join("data/.fsdb/filedb.sqlite")],
        1,
        None,
        &[
            source_sql!("frame/nfs_server/src/filedb.sql"),
            source_sql!("frame/nfs_server/src/share/share.sql"),
            source_sql!("frame/nfs_server/src/ndm/ndm.sql"),
            source_sql!("frame/nfs_server/src/preview/preview.sql"),
            source_sql!("frame/nfs_server/src/archive/password.sql"),
        ],
    )
}
pub(super) fn copy_jobs(ctx: &mut Context) -> Result<(), Failure> {
    stores(
        ctx,
        vec![ctx.root.join("data/.fsdb/copy.sqlite")],
        1,
        None,
        &[source_sql!("frame/nfs_server/src/copy.sql")],
    )
}

pub(super) fn index(ctx: &mut Context) -> Result<(), Failure> {
    let path = ctx.root.join("bin/pkgs/meta_index.db");
    let Some(db) = Database::copy(ctx, &path)? else {
        return Ok(());
    };
    package_lib::MetaIndexDb::open_existing_readonly(&db.staged)
        .map_err(|e| ctx.unsupported(path.display(), e))?;
    let refpath = ctx.copy_path();
    package_lib::MetaIndexDb::create_or_open(&refpath)
        .map_err(|e| ctx.error("PIKG target schema", e))?;
    let expected =
        layout(&Connection::open(refpath).map_err(|e| ctx.error("PIKG target schema", e))?)
            .map_err(|e| ctx.error("PIKG target schema", e))?;
    verify_schema(ctx, &db, 0, None, &expected)
}

#[derive(Deserialize)]
struct NamedConfig {
    #[serde(default = "one")]
    epoch: u64,
    #[serde(alias = "targets")]
    stores: Vec<Store>,
}
#[derive(Deserialize)]
struct Store {
    #[serde(default)]
    store_id: Option<String>,
    #[serde(default)]
    device_did: String,
    #[serde(default = "weight")]
    weight: u32,
    #[serde(default = "enabled")]
    enabled: bool,
    #[serde(default)]
    readonly: bool,
    #[serde(alias = "base_dir", alias = "root_path", alias = "store_path")]
    path: PathBuf,
}
fn one() -> u64 {
    1
}
fn weight() -> u32 {
    1
}
fn enabled() -> bool {
    true
}
#[derive(Deserialize)]
struct FsDaemonConfig {
    #[serde(alias = "meta_db_path", alias = "fs_meta_path")]
    fs_meta_db_path: Option<PathBuf>,
}

// Called both before prepare and when checking a recovered transaction.
// Never enumerate an external directory or infer authorization from a suffix.
fn read_config<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    crate::transaction::validate_path(path)?;
    let file = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    Ok(Some(serde_json::from_reader(file)?))
}

// Match native filesystem identities even when a persisted Windows config
// uses an ordinary drive path and the installer passes a canonical verbatim root.
// Validate before canonicalizing so a symlink, junction or ".." never grants access.
fn storage_db_path(root: &Path, path: &Path) -> anyhow::Result<PathBuf> {
    crate::transaction::validate_path(path)?;
    #[cfg(windows)]
    {
        use std::path::Prefix;
        anyhow::ensure!(
            matches!(path.components().next(), Some(Component::Prefix(p)) if matches!(p.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))),
            "expected a local drive path: {}",
            path.display()
        );
        let canonical = crate::absolute_root(path)?;
        let canonical_root = crate::absolute_root(root)?;
        return Ok(match canonical.strip_prefix(&canonical_root) {
            Ok(relative) => root.join(relative),
            Err(_) => canonical,
        });
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(path.to_owned())
    }
}

fn fs_meta_path(root: &Path) -> anyhow::Result<PathBuf> {
    let file = root.join("etc/fs_daemon.json");
    let cfg: Option<FsDaemonConfig> = read_config(&file)?;
    let path = cfg
        .and_then(|c| c.fs_meta_db_path)
        .unwrap_or_else(|| root.join("var/fs_meta/fs_meta.db"));
    storage_db_path(root, &path)
}

fn named_db_path(root: &Path, store: &Store) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(!store.path.as_os_str().is_empty(), "empty Named Store path");
    anyhow::ensure!(
        !store
            .path
            .components()
            .any(|c| matches!(c, Component::ParentDir)),
        "unexpected '..' in Named Store path"
    );
    anyhow::ensure!(
        store.path.is_absolute()
            || store
                .path
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "expected a relative store directory or an absolute local path"
    );
    let path = root
        .join("storage")
        .join(&store.path)
        .join("named_store.db");
    storage_db_path(root, &path)
}

pub fn configured_external_db(root: &Path, path: &Path) -> anyhow::Result<bool> {
    let path = storage_db_path(root, path)?;
    let allowed = |main: &Path| {
        !main.starts_with(root)
            && (main == path
                || ["-wal", "-shm", "-journal"]
                    .iter()
                    .any(|ext| suffix(main, ext) == path))
    };
    if allowed(&fs_meta_path(root)?) {
        return Ok(true);
    }
    let config: Option<NamedConfig> = read_config(&root.join("storage/named_store.json"))?;
    if let Some(cfg) = config {
        for store in &cfg.stores {
            if allowed(&named_db_path(root, store)?) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

pub(super) fn named(ctx: &mut Context) -> Result<(), Failure> {
    let dir = ctx.root.join("storage");
    let file = dir.join("named_store.json");
    let Some(config) =
        read_config::<NamedConfig>(&file).map_err(|e| ctx.error(file.display(), e))?
    else {
        return Ok(());
    };
    if config.stores.is_empty() {
        return Err(ctx.unsupported(file.display(), "store list is empty"));
    }
    let mut expected = None;
    ctx.editor.epoch = config.epoch.max(1);
    for (index, store) in config.stores.into_iter().enumerate() {
        let path = named_db_path(ctx.root, &store).map_err(|e| ctx.error(file.display(), e))?;
        let base = path.parent().unwrap().to_owned();
        let id = store.store_id.filter(|s| !s.is_empty()).unwrap_or_else(|| {
            store
                .path
                .file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("store-{}", index + 1))
        });
        let target = named_store::StoreTarget {
            store_id: id,
            device_did: store.device_did,
            capacity: None,
            used: None,
            readonly: store.readonly,
            enabled: store.enabled,
            weight: store.weight,
        };
        if ctx
            .editor
            .targets
            .iter()
            .any(|t| t.store_id == target.store_id)
        {
            return Err(ctx.unsupported(file.display(), "duplicate named store ID"));
        }
        ctx.editor.targets.push(target.clone());
        if let Some(db) = Database::copy(ctx, &path)? {
            if expected.is_none() {
                let template = ctx.copy_path();
                drop(
                    named_store::NamedLocalStoreDB::new(template.to_string_lossy().into_owned())
                        .map_err(|e| ctx.error("Named Store target schema", e))?,
                );
                let conn = Connection::open(template)
                    .map_err(|e| ctx.error("Named Store target schema", e))?;
                expected =
                    Some(layout(&conn).map_err(|e| ctx.error("Named Store target schema", e))?);
            }
            if !ctx.editor.records.is_empty() {
                // Reading a NamedObject can update LRU state. Give the content
                // reader a disposable metadata DB distinct from the apply plan.
                let private = ctx.copy_path();
                db.conn
                    .backup(rusqlite::DatabaseName::Main, &private, None)
                    .map_err(|e| ctx.error(path.display(), e))?;
                ctx.editor.stores.push((target.store_id, base, private));
            }
        }
    }
    Ok(())
}

pub(super) fn workspaces(ctx: &mut Context) -> Result<(), Failure> {
    use aiworkspace_store::schema::{DOC_DDL, LOCAL_ADDED_DDL, LOCAL_DDL};
    let data = ctx.root.join("data/aiworkspace");
    stores(
        ctx,
        vec![data.join("sessions.sqlite")],
        1,
        None,
        &[source_sql!(
            "frame/aiworkspace/store/src/session/sessions.sql"
        )],
    )?;
    stores(
        ctx,
        vec![data.join("users.sqlite")],
        1,
        None,
        &[source_sql!("frame/aiworkspace/store/src/users.sql")],
    )?;
    stores(
        ctx,
        vec![data.join("library.sqlite")],
        1,
        None,
        &[aiworkspace_store::library::DDL],
    )?;
    for dir in ctx.entries(&data.join("workspaces"))? {
        let id = dir.file_name().and_then(|s| s.to_str()).unwrap_or("");
        if id.starts_with(".tmp-") {
            return Err(ctx.error(
                dir.display(),
                "incomplete workspace operation requires recovery",
            ));
        }
        if !id.starts_with("ws_") {
            continue;
        }
        let doc = Database::copy(ctx, &dir.join("doc.sqlite"))?
            .ok_or_else(|| ctx.unsupported(dir.display(), "workspace document missing"))?;
        let local = Database::copy(ctx, &dir.join("local.sqlite"))?
            .ok_or_else(|| ctx.unsupported(dir.display(), "workspace local DB missing"))?;
        aiworkspace_store::workspace::validate_workspace_storage(&doc.conn, &local.conn)
            .map_err(|e| ctx.unsupported(dir.display(), e))?;
        let stored_id = doc
            .conn
            .query_row(
                "SELECT value FROM workspace_meta WHERE key='workspace_id'",
                [],
                |r| r.get::<_, String>(0),
            )
            .map_err(|e| ctx.unsupported(dir.display(), e))?;
        if stored_id != id {
            return Err(ctx.unsupported(dir.display(), "workspace identity mismatch"));
        }
        let expected_local = reference(ctx, &[LOCAL_DDL, LOCAL_ADDED_DDL])?;
        let mut doc_ddl = vec![DOC_DDL];
        if doc
            .conn
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name='session_events'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map_err(|e| ctx.error(dir.display(), e))?
            != 0
        {
            doc_ddl.push(aiworkspace_store::session::EVENTS_DDL);
        }
        check_layout(ctx, &doc, &reference(ctx, &doc_ddl)?)?;
        check_layout(ctx, &local, &expected_local)?;
    }
    Ok(())
}

/// fs-daemon is optional. Its deployed config, when present, selects the metadata
/// file, including an explicitly configured file on another local volume.
pub(super) fn fsmeta(ctx: &mut Context) -> Result<(), Failure> {
    let path = fs_meta_path(ctx.root).map_err(|e| ctx.error("etc/fs_daemon.json", e))?;
    let Some(db) = Database::copy(ctx, &path)? else {
        return Ok(());
    };
    let reference = ctx.copy_path();
    // The owner starts temporary cleanup tasks even when creating an empty
    // reference store. Drop its entire runtime before verifying the template.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| ctx.error("fs_meta target schema", e))?;
    runtime.block_on(async {
        drop(
            fs_meta::fs_meta_service::FSMetaService::new(reference.to_string_lossy().into_owned())
                .map_err(|e| ctx.error("fs_meta target schema", e))?,
        );
        Ok::<_, Failure>(())
    })?;
    drop(runtime);
    let conn = Connection::open(reference).map_err(|e| ctx.error("fs_meta target schema", e))?;
    let expected = layout(&conn).map_err(|e| ctx.error("fs_meta target schema", e))?;
    verify_schema(ctx, &db, 0, None, &expected)
}
