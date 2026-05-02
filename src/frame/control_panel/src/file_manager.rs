use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::get_buckyos_api_runtime;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::get_buckyos_root_dir;
use bytes::Bytes;
use http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    RANGE,
};
use http::{Method, StatusCode, Version};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use kRPC::{RPCErrors, RPCHandler, RPCRequest, RPCResponse, RPCResult};
use log::warn;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::io::{Read, SeekFrom, Write};
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;
use zip::write::FileOptions;
use zip::CompressionMethod;

const BUCKY_FILE_SERVICE_NAME: &str = "bucky-file";
const INTERNAL_RECYCLE_BIN_DIR: &str = ".bucky_recycle_bin";
const USER_PUBLIC_LINK_NAME: &str = "public";
const INLINE_TEXT_CONTENT_MAX_BYTES: u64 = 2 * 1024 * 1024;
const PDF_PREVIEW_SUPPORTED_EXTENSIONS: &[&str] = &["doc", "docx", "odt", "rtf"];
const FILE_META_STATE_KEY_ROOT_SEEDED: &str = "root_seeded";
const THUMBNAIL_VARIANT_DEFAULT: &str = "s160";
const THUMBNAIL_SIZE_DEFAULT: u32 = 160;
const THUMBNAIL_SIZE_MIN: u32 = 48;
const THUMBNAIL_SIZE_MAX: u32 = 512;

static THUMBNAIL_WORKER_ONCE: Once = Once::new();

#[cfg(not(target_os = "windows"))]
fn external_command(program: impl AsRef<OsStr>) -> Command {
    Command::new(program)
}

#[cfg(target_os = "windows")]
fn external_command(program: impl AsRef<OsStr>) -> Command {
    let mut command = Command::new(program);
    use std::os::windows::process::CommandExt;
    command.creation_flags(windows_hidden_process_creation_flags());
    command
}

#[cfg(target_os = "windows")]
fn windows_hidden_process_creation_flags() -> u32 {
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW
}

#[derive(Debug, Clone)]
pub(crate) struct BuckyFileServer {
    data_folder: PathBuf,
    db_path: PathBuf,
}

#[derive(Debug, Clone)]
struct FileAuthPrincipal {
    username: String,
}

#[derive(Debug, Serialize, Clone)]
struct FileEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: u64,
}

#[derive(Debug, Serialize)]
struct DirectoryResponse {
    path: String,
    is_dir: bool,
    items: Vec<FileEntry>,
}

#[derive(Debug, Serialize)]
struct FileResponse {
    path: String,
    is_dir: bool,
    size: u64,
    modified: u64,
    content: Option<String>,
}

#[derive(Debug, Serialize)]
struct SearchResponse {
    query: String,
    path: String,
    kind: String,
    limit: usize,
    truncated: bool,
    items: Vec<FileEntry>,
}

#[derive(Debug, Serialize, Default, Clone)]
struct FileMetadataSyncStats {
    scanned: u64,
    upserted: u64,
    removed: u64,
}

#[derive(Debug, Serialize)]
struct FileNavResponse {
    path: String,
    previous: Option<FileEntry>,
    next: Option<FileEntry>,
}

#[derive(Debug, Serialize)]
struct FavoriteListResponse {
    items: Vec<FileEntry>,
}

#[derive(Debug, Serialize, Clone)]
struct RecentFileEntry {
    #[serde(flatten)]
    entry: FileEntry,
    last_accessed_at: u64,
    access_count: u64,
}

#[derive(Debug, Serialize)]
struct RecentListResponse {
    items: Vec<RecentFileEntry>,
}

#[derive(Debug, Serialize, Clone)]
struct RecycleBinItem {
    item_id: String,
    #[serde(flatten)]
    entry: FileEntry,
    original_path: String,
    deleted_at: u64,
}

#[derive(Debug, Serialize)]
struct RecycleBinListResponse {
    items: Vec<RecycleBinItem>,
}

#[derive(Debug, Clone)]
struct RecycleBinItemRecordInput {
    owner: String,
    item_id: String,
    original_rel_path: String,
    trashed_rel_path: String,
    name: String,
    is_dir: bool,
    size: u64,
    modified: u64,
    deleted_at: u64,
}

#[derive(Debug, Clone)]
struct ThumbnailTask {
    owner: String,
    rel_path: String,
    variant: String,
    source_size: i64,
    source_modified: i64,
    attempts: i64,
}

#[derive(Debug, Deserialize)]
struct PutFileRequest {
    content: String,
}

#[derive(Debug, Deserialize)]
struct FavoriteRequest {
    path: String,
}

#[derive(Debug, Deserialize)]
struct RecycleRestoreRequest {
    item_id: String,
    override_existing: Option<bool>,
}

#[derive(Debug, Clone)]
struct FileListFilters {
    kind: String,
    exts: Vec<String>,
    modified_from: Option<u64>,
    modified_to: Option<u64>,
    size_min: Option<u64>,
    size_max: Option<u64>,
    sort_by: Option<String>,
    order: String,
    limit: usize,
}

#[derive(Debug, Deserialize)]
struct PatchResourceRequest {
    action: String,
    destination: Option<String>,
    new_name: Option<String>,
    override_existing: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CreateShareRequest {
    path: String,
    password: Option<String>,
    expires_in_seconds: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct CreateUploadSessionRequest {
    path: String,
    size: u64,
    chunk_size: Option<u64>,
    override_existing: Option<bool>,
}

#[derive(Debug, Serialize, Clone)]
struct UploadSessionRecord {
    id: String,
    owner: String,
    path: String,
    size: u64,
    chunk_size: u64,
    uploaded_size: u64,
    override_existing: bool,
    created_at: u64,
    updated_at: u64,
}

#[derive(Debug, Serialize, Clone)]
struct ShareItem {
    id: String,
    owner: String,
    path: String,
    created_at: u64,
    expires_at: Option<u64>,
    password_required: bool,
}

enum RawByteRange {
    Full,
    Partial { start: u64, end: u64 },
    Unsatisfiable,
}

impl BuckyFileServer {
    pub(crate) fn new(data_folder: PathBuf, _standalone_mode: bool) -> Self {
        let db_path = data_folder.join("bucky_file.db");

        Self {
            data_folder,
            db_path,
        }
    }

    fn now_unix() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs()
    }

    fn hash_optional_password(password: Option<&str>) -> Option<String> {
        password
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(|value| STANDARD.encode(Sha256::digest(value.as_bytes())))
    }

    fn db_path(&self) -> PathBuf {
        self.db_path.clone()
    }

    fn upload_tmp_dir(&self) -> PathBuf {
        self.data_folder.join("upload_sessions")
    }

    fn upload_tmp_path(&self, session_id: &str) -> PathBuf {
        self.upload_tmp_dir().join(format!("{}.part", session_id))
    }

    pub(crate) async fn init_share_db(&self) -> Result<(), RPCErrors> {
        let db_path = self.db_path();
        let upload_tmp_dir = self.upload_tmp_dir();
        let thumbnail_cache_dir = self.thumbnail_cache_dir();
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open share database failed: {}", err))
            })?;
            conn.execute_batch(
                "
                CREATE TABLE IF NOT EXISTS shares (
                    id TEXT PRIMARY KEY,
                    owner TEXT NOT NULL,
                    path TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    expires_at INTEGER,
                    password_hash TEXT
                );
                CREATE INDEX IF NOT EXISTS idx_shares_owner ON shares(owner);

                CREATE TABLE IF NOT EXISTS upload_sessions (
                    id TEXT PRIMARY KEY,
                    owner TEXT NOT NULL,
                    path TEXT NOT NULL,
                    size INTEGER NOT NULL,
                    chunk_size INTEGER NOT NULL,
                    uploaded_size INTEGER NOT NULL,
                    override_existing INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_upload_sessions_owner ON upload_sessions(owner);

                CREATE TABLE IF NOT EXISTS file_entries (
                    owner TEXT NOT NULL,
                    rel_path TEXT NOT NULL,
                    parent_rel_path TEXT NOT NULL,
                    name TEXT NOT NULL,
                    lower_name TEXT NOT NULL,
                    ext TEXT NOT NULL,
                    is_dir INTEGER NOT NULL,
                    size INTEGER NOT NULL,
                    modified INTEGER NOT NULL,
                    mime TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    deleted_at INTEGER,
                    PRIMARY KEY (owner, rel_path)
                );
                CREATE INDEX IF NOT EXISTS idx_file_entries_parent ON file_entries(owner, parent_rel_path, is_dir, lower_name, name);
                CREATE INDEX IF NOT EXISTS idx_file_entries_modified ON file_entries(owner, modified DESC);
                CREATE INDEX IF NOT EXISTS idx_file_entries_ext ON file_entries(owner, ext);

                CREATE TABLE IF NOT EXISTS file_meta_state (
                    owner TEXT NOT NULL,
                    state_key TEXT NOT NULL,
                    state_value TEXT,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (owner, state_key)
                );

                CREATE TABLE IF NOT EXISTS file_thumbnails (
                    owner TEXT NOT NULL,
                    rel_path TEXT NOT NULL,
                    variant TEXT NOT NULL,
                    source_size INTEGER NOT NULL,
                    source_modified INTEGER NOT NULL,
                    status TEXT NOT NULL,
                    thumb_rel_path TEXT,
                    width INTEGER,
                    height INTEGER,
                    attempts INTEGER NOT NULL,
                    last_error TEXT,
                    updated_at INTEGER NOT NULL,
                    next_retry_at INTEGER NOT NULL,
                    PRIMARY KEY (owner, rel_path, variant)
                );
                CREATE INDEX IF NOT EXISTS idx_file_thumbnails_status_retry ON file_thumbnails(status, next_retry_at, updated_at);
                CREATE INDEX IF NOT EXISTS idx_file_thumbnails_owner_path ON file_thumbnails(owner, rel_path, variant);

                CREATE TABLE IF NOT EXISTS file_favorites (
                    owner TEXT NOT NULL,
                    rel_path TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    PRIMARY KEY (owner, rel_path)
                );
                CREATE INDEX IF NOT EXISTS idx_file_favorites_owner_updated ON file_favorites(owner, updated_at DESC);

                CREATE TABLE IF NOT EXISTS file_recent (
                    owner TEXT NOT NULL,
                    rel_path TEXT NOT NULL,
                    last_accessed_at INTEGER NOT NULL,
                    access_count INTEGER NOT NULL,
                    PRIMARY KEY (owner, rel_path)
                );
                CREATE INDEX IF NOT EXISTS idx_file_recent_owner_accessed ON file_recent(owner, last_accessed_at DESC);

                CREATE TABLE IF NOT EXISTS recycle_bin_items (
                    item_id TEXT PRIMARY KEY,
                    owner TEXT NOT NULL,
                    original_rel_path TEXT NOT NULL,
                    trashed_rel_path TEXT NOT NULL,
                    name TEXT NOT NULL,
                    is_dir INTEGER NOT NULL,
                    size INTEGER NOT NULL,
                    modified INTEGER NOT NULL,
                    deleted_at INTEGER NOT NULL
                );
                CREATE INDEX IF NOT EXISTS idx_recycle_bin_owner_deleted ON recycle_bin_items(owner, deleted_at DESC);
                ",
            )
            .map_err(|err| {
                RPCErrors::ReasonError(format!("init share database failed: {}", err))
            })?;

            std::fs::create_dir_all(&upload_tmp_dir).map_err(|err| {
                RPCErrors::ReasonError(format!("prepare upload tmp dir failed: {}", err))
            })?;
            std::fs::create_dir_all(&thumbnail_cache_dir).map_err(|err| {
                RPCErrors::ReasonError(format!("prepare thumbnail cache dir failed: {}", err))
            })?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("init share database join error: {}", err)))??;

        THUMBNAIL_WORKER_ONCE.call_once(|| {
            let worker = self.clone();
            tokio::spawn(async move {
                worker.run_thumbnail_worker().await;
            });
        });

        Ok(())
    }

    fn normalize_file_ext(name: &str) -> String {
        Path::new(name)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default()
    }

    fn normalize_file_mime(path: &Path, is_dir: bool) -> String {
        if is_dir {
            return "inode/directory".to_string();
        }
        Self::content_type_for_path(path)
            .split(';')
            .next()
            .unwrap_or("application/octet-stream")
            .trim()
            .to_string()
    }

    fn rel_path_scope_like(scope_display_path: &str) -> Option<String> {
        if scope_display_path == "/" {
            return None;
        }
        Some(format!("{}/%", scope_display_path.trim_end_matches('/')))
    }

    async fn mark_owner_root_seeded(&self, owner: &str) -> Result<(), RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open metadata database failed: {}", err))
            })?;
            let now = BuckyFileServer::now_unix() as i64;
            conn.execute(
                "INSERT INTO file_meta_state(owner, state_key, state_value, updated_at)
                 VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(owner, state_key) DO UPDATE SET
                   state_value=excluded.state_value,
                   updated_at=excluded.updated_at",
                params![owner, FILE_META_STATE_KEY_ROOT_SEEDED, "1", now],
            )
            .map_err(|err| {
                RPCErrors::ReasonError(format!("update metadata state failed: {}", err))
            })?;
            Ok(())
        })
        .await
        .map_err(|err| {
            RPCErrors::ReasonError(format!("update metadata state join error: {}", err))
        })?
    }

    async fn owner_root_seeded(&self, owner: &str) -> Result<bool, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open metadata database failed: {}", err))
            })?;
            let seeded = conn
                .query_row(
                    "SELECT 1 FROM file_meta_state WHERE owner = ?1 AND state_key = ?2 LIMIT 1",
                    params![owner, FILE_META_STATE_KEY_ROOT_SEEDED],
                    |_| Ok(()),
                )
                .optional()
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("query metadata state failed: {}", err))
                })?
                .is_some();
            Ok(seeded)
        })
        .await
        .map_err(|err| {
            RPCErrors::ReasonError(format!("query metadata state join error: {}", err))
        })?
    }

    async fn sync_file_metadata_subtree(
        &self,
        owner: &str,
        rel_scope_path: &Path,
    ) -> Result<FileMetadataSyncStats, RPCErrors> {
        #[derive(Clone)]
        struct IndexedFsEntry {
            rel_path: String,
            parent_rel_path: String,
            name: String,
            lower_name: String,
            ext: String,
            is_dir: i64,
            size: i64,
            modified: i64,
            mime: String,
        }

        fn collect_entries_under_scope(
            root_abs_path: &Path,
            rel_scope_path: &Path,
        ) -> Result<Vec<IndexedFsEntry>, RPCErrors> {
            let mut entries: Vec<IndexedFsEntry> = Vec::new();
            if !root_abs_path.exists() {
                return Ok(entries);
            }

            let mut stack: Vec<(PathBuf, PathBuf)> = Vec::new();

            if rel_scope_path.as_os_str().is_empty() {
                let root_reader = std::fs::read_dir(root_abs_path).map_err(|err| {
                    RPCErrors::ReasonError(format!(
                        "read root directory for metadata sync failed ({}): {}",
                        root_abs_path.display(),
                        err
                    ))
                })?;
                for child in root_reader {
                    let child = child.map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "read root directory entry for metadata sync failed: {}",
                            err
                        ))
                    })?;
                    let child_path = child.path();
                    let file_name = child.file_name().to_string_lossy().to_string();
                    if file_name.is_empty() {
                        continue;
                    }
                    if file_name == INTERNAL_RECYCLE_BIN_DIR {
                        continue;
                    }
                    let child_rel_path = PathBuf::from(&file_name);
                    stack.push((child_path, child_rel_path));
                }
            } else {
                if BuckyFileServer::is_internal_rel_path(rel_scope_path) {
                    return Ok(entries);
                }
                stack.push((root_abs_path.to_path_buf(), rel_scope_path.to_path_buf()));
            }

            while let Some((entry_abs_path, entry_rel_path)) = stack.pop() {
                if BuckyFileServer::is_internal_rel_path(&entry_rel_path) {
                    continue;
                }
                let metadata = std::fs::metadata(&entry_abs_path).map_err(|err| {
                    RPCErrors::ReasonError(format!(
                        "read metadata during metadata sync failed ({}): {}",
                        entry_abs_path.display(),
                        err
                    ))
                })?;
                let is_dir = metadata.is_dir();
                let name = entry_rel_path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_default();
                if name.is_empty() {
                    continue;
                }

                let rel_path_display = BuckyFileServer::to_display_path(&entry_rel_path);
                let parent_display = entry_rel_path
                    .parent()
                    .map(BuckyFileServer::to_display_path)
                    .unwrap_or_else(|| "/".to_string());

                entries.push(IndexedFsEntry {
                    rel_path: rel_path_display,
                    parent_rel_path: parent_display,
                    name: name.clone(),
                    lower_name: name.to_ascii_lowercase(),
                    ext: BuckyFileServer::normalize_file_ext(&name),
                    is_dir: if is_dir { 1 } else { 0 },
                    size: if metadata.is_file() {
                        metadata.len().min(i64::MAX as u64) as i64
                    } else {
                        0
                    },
                    modified: BuckyFileServer::unix_mtime(&metadata).min(i64::MAX as u64) as i64,
                    mime: BuckyFileServer::normalize_file_mime(&entry_abs_path, is_dir),
                });

                if !is_dir {
                    continue;
                }

                let reader = std::fs::read_dir(&entry_abs_path).map_err(|err| {
                    RPCErrors::ReasonError(format!(
                        "read directory during metadata sync failed ({}): {}",
                        entry_abs_path.display(),
                        err
                    ))
                })?;

                for child in reader {
                    let child = child.map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "read directory entry during metadata sync failed: {}",
                            err
                        ))
                    })?;
                    let file_type = child.file_type().map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "read file type during metadata sync failed: {}",
                            err
                        ))
                    })?;
                    if file_type.is_symlink() {
                        continue;
                    }
                    let child_name = child.file_name().to_string_lossy().to_string();
                    if child_name.is_empty() {
                        continue;
                    }
                    if child_name == INTERNAL_RECYCLE_BIN_DIR {
                        continue;
                    }

                    let child_rel_path = entry_rel_path.join(&child_name);
                    stack.push((child.path(), child_rel_path));
                }
            }

            Ok(entries)
        }

        let db_path = self.db_path();
        let owner = owner.to_string();
        let rel_scope_path = rel_scope_path.to_path_buf();
        let root_abs_path = self.user_root(&owner).join(&rel_scope_path);
        let scope_display_path = Self::to_display_path(&rel_scope_path);

        tokio::task::spawn_blocking(move || -> Result<FileMetadataSyncStats, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open metadata database failed: {}", err))
            })?;
            let tx = conn.unchecked_transaction().map_err(|err| {
                RPCErrors::ReasonError(format!("start metadata transaction failed: {}", err))
            })?;

            let entries = collect_entries_under_scope(&root_abs_path, &rel_scope_path)?;
            let mut stats = FileMetadataSyncStats {
                scanned: entries.len() as u64,
                upserted: 0,
                removed: 0,
            };

            tx.execute(
                "CREATE TEMP TABLE IF NOT EXISTS temp_file_meta_paths(rel_path TEXT PRIMARY KEY)",
                [],
            )
            .map_err(|err| {
                RPCErrors::ReasonError(format!("prepare metadata temp table failed: {}", err))
            })?;
            tx.execute("DELETE FROM temp_file_meta_paths", [])
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("clear metadata temp table failed: {}", err))
                })?;

            let now = BuckyFileServer::now_unix().min(i64::MAX as u64) as i64;
            for entry in entries {
                tx.execute(
                    "INSERT INTO temp_file_meta_paths(rel_path) VALUES(?1)",
                    params![entry.rel_path],
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("insert metadata temp path failed: {}", err))
                })?;

                tx.execute(
                    "INSERT INTO file_entries(
                        owner, rel_path, parent_rel_path, name, lower_name, ext,
                        is_dir, size, modified, mime, created_at, updated_at, deleted_at
                    ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL)
                     ON CONFLICT(owner, rel_path) DO UPDATE SET
                        parent_rel_path=excluded.parent_rel_path,
                        name=excluded.name,
                        lower_name=excluded.lower_name,
                        ext=excluded.ext,
                        is_dir=excluded.is_dir,
                        size=excluded.size,
                        modified=excluded.modified,
                        mime=excluded.mime,
                        updated_at=excluded.updated_at,
                        deleted_at=NULL",
                    params![
                        owner.as_str(),
                        entry.rel_path,
                        entry.parent_rel_path,
                        entry.name,
                        entry.lower_name,
                        entry.ext,
                        entry.is_dir,
                        entry.size,
                        entry.modified,
                        entry.mime,
                        now,
                        now,
                    ],
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("upsert file metadata failed: {}", err))
                })?;
                stats.upserted += 1;
            }

            let removed_rows = if scope_display_path == "/" {
                tx.execute(
                    "DELETE FROM file_entries
                     WHERE owner = ?1
                       AND rel_path NOT IN (SELECT rel_path FROM temp_file_meta_paths)",
                    params![owner.as_str()],
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("remove stale metadata failed: {}", err))
                })?
            } else {
                let like_scope =
                    BuckyFileServer::rel_path_scope_like(&scope_display_path).unwrap_or_default();
                tx.execute(
                    "DELETE FROM file_entries
                     WHERE owner = ?1
                       AND (rel_path = ?2 OR rel_path LIKE ?3)
                       AND rel_path NOT IN (SELECT rel_path FROM temp_file_meta_paths)",
                    params![owner.as_str(), scope_display_path, like_scope],
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("remove stale metadata failed: {}", err))
                })?
            };
            stats.removed = removed_rows.max(0) as u64;

            tx.execute("DELETE FROM temp_file_meta_paths", [])
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("clear metadata temp table failed: {}", err))
                })?;

            tx.commit().map_err(|err| {
                RPCErrors::ReasonError(format!("commit metadata transaction failed: {}", err))
            })?;

            Ok(stats)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("sync metadata join error: {}", err)))?
    }

    async fn ensure_owner_metadata_seeded(&self, owner: &str) -> Result<(), RPCErrors> {
        if self.owner_root_seeded(owner).await? {
            return Ok(());
        }

        let _ = self
            .sync_file_metadata_subtree(owner, Path::new(""))
            .await?;
        self.sync_thumbnail_tasks_for_scope(owner, Path::new(""))
            .await?;
        self.mark_owner_root_seeded(owner).await
    }

    fn recycle_bin_item_rel_path(item_id: &str, name: &str) -> PathBuf {
        PathBuf::from(INTERNAL_RECYCLE_BIN_DIR)
            .join(item_id)
            .join(name)
    }

    fn is_internal_rel_path(rel_path: &Path) -> bool {
        rel_path.components().any(|component| match component {
            Component::Normal(part) => part.to_string_lossy() == INTERNAL_RECYCLE_BIN_DIR,
            _ => false,
        })
    }

    fn thumbnail_cache_dir(&self) -> PathBuf {
        self.data_folder.join("thumb_cache")
    }

    fn thumbnail_variant_for_size(size: u32) -> String {
        format!("s{}", size)
    }

    fn parse_thumbnail_size(raw_size: Option<String>) -> u32 {
        raw_size
            .and_then(|value| value.trim().parse::<u32>().ok())
            .unwrap_or(THUMBNAIL_SIZE_DEFAULT)
            .clamp(THUMBNAIL_SIZE_MIN, THUMBNAIL_SIZE_MAX)
    }

    fn is_thumbnail_supported_ext(ext: &str) -> bool {
        matches!(ext, "jpg" | "jpeg" | "png" | "webp")
    }

    fn build_thumbnail_rel_path(
        owner: &str,
        rel_path: &str,
        variant: &str,
        source_size: i64,
        source_modified: i64,
    ) -> String {
        let payload = format!(
            "{}|{}|{}|{}|{}",
            owner, rel_path, variant, source_size, source_modified
        );
        let digest = Sha256::digest(payload.as_bytes());
        let digest_hex = digest
            .iter()
            .map(|value| format!("{:02x}", value))
            .collect::<String>();
        format!("{}/{}.jpg", &digest_hex[..2], digest_hex)
    }

    async fn sync_thumbnail_tasks_for_scope(
        &self,
        owner: &str,
        rel_scope_path: &Path,
    ) -> Result<(), RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let scope_display_path = Self::to_display_path(rel_scope_path);
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open thumbnail database failed: {}", err))
            })?;
            let tx = conn.unchecked_transaction().map_err(|err| {
                RPCErrors::ReasonError(format!("start thumbnail transaction failed: {}", err))
            })?;

            let now = BuckyFileServer::now_unix().min(i64::MAX as u64) as i64;
            let scope_like =
                BuckyFileServer::rel_path_scope_like(&scope_display_path).unwrap_or_default();

            let select_sql = if scope_display_path == "/" {
                "SELECT rel_path, ext, size, modified FROM file_entries WHERE owner = ?1 AND is_dir = 0"
            } else {
                "SELECT rel_path, ext, size, modified FROM file_entries
                 WHERE owner = ?1 AND is_dir = 0 AND (rel_path = ?2 OR rel_path LIKE ?3)"
            };

            {
                let mut stmt = tx.prepare(select_sql).map_err(|err| {
                    RPCErrors::ReasonError(format!("prepare thumbnail source query failed: {}", err))
                })?;
                let mut rows = if scope_display_path == "/" {
                    stmt.query(params![owner.as_str()])
                } else {
                    stmt.query(params![owner.as_str(), scope_display_path, scope_like])
                }
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("query thumbnail source failed: {}", err))
                })?;

                while let Some(row) = rows.next().map_err(|err| {
                    RPCErrors::ReasonError(format!("iterate thumbnail source failed: {}", err))
                })? {
                    let rel_path: String = row.get(0).map_err(|err| {
                        RPCErrors::ReasonError(format!("read thumbnail source rel_path failed: {}", err))
                    })?;
                    let ext: String = row.get(1).map_err(|err| {
                        RPCErrors::ReasonError(format!("read thumbnail source ext failed: {}", err))
                    })?;
                    let size: i64 = row.get(2).map_err(|err| {
                        RPCErrors::ReasonError(format!("read thumbnail source size failed: {}", err))
                    })?;
                    let modified: i64 = row.get(3).map_err(|err| {
                        RPCErrors::ReasonError(format!("read thumbnail source modified failed: {}", err))
                    })?;

                    if !BuckyFileServer::is_thumbnail_supported_ext(ext.as_str()) {
                        continue;
                    }

                    let variant = THUMBNAIL_VARIANT_DEFAULT;
                    tx.execute(
                    "INSERT INTO file_thumbnails(
                        owner, rel_path, variant, source_size, source_modified,
                        status, thumb_rel_path, width, height, attempts,
                        last_error, updated_at, next_retry_at
                    ) VALUES(?1, ?2, ?3, ?4, ?5, 'pending', NULL, NULL, NULL, 0, NULL, ?6, 0)
                     ON CONFLICT(owner, rel_path, variant) DO UPDATE SET
                        source_size=excluded.source_size,
                        source_modified=excluded.source_modified,
                        updated_at=excluded.updated_at,
                        status=CASE
                            WHEN file_thumbnails.source_size != excluded.source_size
                              OR file_thumbnails.source_modified != excluded.source_modified
                            THEN 'pending'
                            ELSE file_thumbnails.status
                        END,
                        thumb_rel_path=CASE
                            WHEN file_thumbnails.source_size != excluded.source_size
                              OR file_thumbnails.source_modified != excluded.source_modified
                            THEN NULL
                            ELSE file_thumbnails.thumb_rel_path
                        END,
                        width=CASE
                            WHEN file_thumbnails.source_size != excluded.source_size
                              OR file_thumbnails.source_modified != excluded.source_modified
                            THEN NULL
                            ELSE file_thumbnails.width
                        END,
                        height=CASE
                            WHEN file_thumbnails.source_size != excluded.source_size
                              OR file_thumbnails.source_modified != excluded.source_modified
                            THEN NULL
                            ELSE file_thumbnails.height
                        END,
                        attempts=CASE
                            WHEN file_thumbnails.source_size != excluded.source_size
                              OR file_thumbnails.source_modified != excluded.source_modified
                            THEN 0
                            ELSE file_thumbnails.attempts
                        END,
                        last_error=CASE
                            WHEN file_thumbnails.source_size != excluded.source_size
                              OR file_thumbnails.source_modified != excluded.source_modified
                            THEN NULL
                            ELSE file_thumbnails.last_error
                        END,
                        next_retry_at=CASE
                            WHEN file_thumbnails.source_size != excluded.source_size
                              OR file_thumbnails.source_modified != excluded.source_modified
                            THEN 0
                            ELSE file_thumbnails.next_retry_at
                        END",
                        params![owner.as_str(), rel_path, variant, size, modified, now],
                    )
                    .map_err(|err| {
                        RPCErrors::ReasonError(format!("upsert thumbnail task failed: {}", err))
                    })?;
                }
            }

            let cleanup_sql = if scope_display_path == "/" {
                "DELETE FROM file_thumbnails
                 WHERE owner = ?1 AND variant = ?2
                   AND NOT EXISTS(
                       SELECT 1 FROM file_entries
                       WHERE file_entries.owner = file_thumbnails.owner
                         AND file_entries.rel_path = file_thumbnails.rel_path
                         AND file_entries.is_dir = 0
                         AND file_entries.ext IN ('jpg', 'jpeg', 'png', 'webp')
                   )"
            } else {
                "DELETE FROM file_thumbnails
                 WHERE owner = ?1 AND variant = ?2
                   AND (rel_path = ?3 OR rel_path LIKE ?4)
                   AND NOT EXISTS(
                       SELECT 1 FROM file_entries
                       WHERE file_entries.owner = file_thumbnails.owner
                         AND file_entries.rel_path = file_thumbnails.rel_path
                         AND file_entries.is_dir = 0
                         AND file_entries.ext IN ('jpg', 'jpeg', 'png', 'webp')
                   )"
            };
            if scope_display_path == "/" {
                tx.execute(cleanup_sql, params![owner.as_str(), THUMBNAIL_VARIANT_DEFAULT])
            } else {
                tx.execute(
                    cleanup_sql,
                    params![
                        owner.as_str(),
                        THUMBNAIL_VARIANT_DEFAULT,
                        scope_display_path,
                        BuckyFileServer::rel_path_scope_like(&scope_display_path)
                            .unwrap_or_default(),
                    ],
                )
            }
            .map_err(|err| {
                RPCErrors::ReasonError(format!("cleanup thumbnail task failed: {}", err))
            })?;

            tx.commit().map_err(|err| {
                RPCErrors::ReasonError(format!("commit thumbnail transaction failed: {}", err))
            })?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("sync thumbnail join error: {}", err)))?
    }

    fn generate_thumbnail_bytes(
        source_abs_path: &Path,
        size: u32,
    ) -> Result<(Vec<u8>, i64, i64), RPCErrors> {
        let image = image::open(source_abs_path).map_err(|err| {
            RPCErrors::ReasonError(format!(
                "decode image for thumbnail failed ({}): {}",
                source_abs_path.display(),
                err
            ))
        })?;
        let resized = image.resize(size, size, FilterType::Lanczos3);
        let width = resized.width().min(i64::MAX as u32) as i64;
        let height = resized.height().min(i64::MAX as u32) as i64;
        let mut output = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut output, 82);
        encoder.encode_image(&resized).map_err(|err| {
            RPCErrors::ReasonError(format!("encode thumbnail jpeg failed: {}", err))
        })?;
        Ok((output, width, height))
    }

    async fn process_thumbnail_task(
        &self,
        owner: &str,
        rel_path: &str,
        variant: &str,
        source_size: i64,
        source_modified: i64,
        attempts: i64,
    ) -> Result<(), RPCErrors> {
        let rel_path_parsed = Self::parse_relative_path(rel_path)?;
        let source_abs_path = self.user_root(owner).join(&rel_path_parsed);
        let metadata = tokio::fs::metadata(&source_abs_path).await.map_err(|err| {
            RPCErrors::ReasonError(format!(
                "read source metadata for thumbnail failed ({}): {}",
                source_abs_path.display(),
                err
            ))
        })?;
        if !metadata.is_file() {
            return Err(RPCErrors::ReasonError(
                "thumbnail source is not a regular file".to_string(),
            ));
        }

        let current_size = metadata.len().min(i64::MAX as u64) as i64;
        let current_modified = Self::unix_mtime(&metadata).min(i64::MAX as u64) as i64;
        let thumb_size = variant
            .trim_start_matches('s')
            .parse::<u32>()
            .ok()
            .unwrap_or(THUMBNAIL_SIZE_DEFAULT)
            .clamp(THUMBNAIL_SIZE_MIN, THUMBNAIL_SIZE_MAX);

        let (bytes, width, height) = tokio::task::spawn_blocking({
            let source_abs_path = source_abs_path.clone();
            move || Self::generate_thumbnail_bytes(&source_abs_path, thumb_size)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("thumbnail task join failed: {}", err)))??;

        let thumb_rel_path = Self::build_thumbnail_rel_path(
            owner,
            rel_path,
            variant,
            current_size,
            current_modified,
        );
        let thumb_abs_path = self.thumbnail_cache_dir().join(&thumb_rel_path);
        if let Some(parent) = thumb_abs_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "create thumbnail cache directory failed ({}): {}",
                    parent.display(),
                    err
                ))
            })?;
        }
        tokio::fs::write(&thumb_abs_path, bytes)
            .await
            .map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "write thumbnail file failed ({}): {}",
                    thumb_abs_path.display(),
                    err
                ))
            })?;

        let db_path = self.db_path();
        let owner_db = owner.to_string();
        let rel_path_db = rel_path.to_string();
        let variant_db = variant.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open thumbnail database failed: {}", err))
            })?;
            let now = BuckyFileServer::now_unix().min(i64::MAX as u64) as i64;
            conn.execute(
                "UPDATE file_thumbnails SET
                    source_size = ?1,
                    source_modified = ?2,
                    status = 'ready',
                    thumb_rel_path = ?3,
                    width = ?4,
                    height = ?5,
                    attempts = ?6,
                    last_error = NULL,
                    next_retry_at = 0,
                    updated_at = ?7
                 WHERE owner = ?8 AND rel_path = ?9 AND variant = ?10",
                params![
                    current_size,
                    current_modified,
                    thumb_rel_path,
                    width,
                    height,
                    attempts,
                    now,
                    owner_db,
                    rel_path_db,
                    variant_db,
                ],
            )
            .map_err(|err| {
                RPCErrors::ReasonError(format!("update thumbnail ready status failed: {}", err))
            })?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("thumbnail update join error: {}", err)))??;

        if current_size != source_size || current_modified != source_modified {
            warn!(
                "thumbnail source changed during processing owner={}, path={}, variant={}",
                owner, rel_path, variant
            );
        }

        Ok(())
    }

    async fn mark_thumbnail_task_failed(
        &self,
        owner: &str,
        rel_path: &str,
        variant: &str,
        attempts: i64,
        error_text: &str,
    ) {
        let next_attempts = attempts.saturating_add(1);
        let backoff_secs = (15_i64 * (1_i64 << next_attempts.min(6))).clamp(15, 1800);
        let next_retry_at = Self::now_unix().saturating_add(backoff_secs as u64) as i64;
        let db_path = self.db_path();
        let owner = owner.to_string();
        let rel_path = rel_path.to_string();
        let variant = variant.to_string();
        let error_text = error_text.to_string();
        let _ = tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open thumbnail database failed: {}", err))
            })?;
            let now = BuckyFileServer::now_unix().min(i64::MAX as u64) as i64;
            conn.execute(
                "UPDATE file_thumbnails SET
                    status = 'failed',
                    attempts = ?1,
                    last_error = ?2,
                    next_retry_at = ?3,
                    updated_at = ?4
                 WHERE owner = ?5 AND rel_path = ?6 AND variant = ?7",
                params![
                    next_attempts,
                    error_text,
                    next_retry_at,
                    now,
                    owner,
                    rel_path,
                    variant,
                ],
            )
            .map_err(|err| {
                RPCErrors::ReasonError(format!("update thumbnail failed status failed: {}", err))
            })?;
            Ok(())
        })
        .await;
    }

    async fn take_next_thumbnail_task(&self) -> Result<Option<ThumbnailTask>, RPCErrors> {
        let db_path = self.db_path();
        tokio::task::spawn_blocking(move || -> Result<Option<ThumbnailTask>, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open thumbnail database failed: {}", err))
            })?;
            let tx = conn.unchecked_transaction().map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "start thumbnail dequeue transaction failed: {}",
                    err
                ))
            })?;
            let now = BuckyFileServer::now_unix().min(i64::MAX as u64) as i64;

            let task = tx
                .query_row(
                    "SELECT owner, rel_path, variant, source_size, source_modified, attempts
                     FROM file_thumbnails
                     WHERE (status = 'pending' OR status = 'failed')
                       AND next_retry_at <= ?1
                     ORDER BY updated_at ASC
                     LIMIT 1",
                    params![now],
                    |row| {
                        Ok(ThumbnailTask {
                            owner: row.get(0)?,
                            rel_path: row.get(1)?,
                            variant: row.get(2)?,
                            source_size: row.get(3)?,
                            source_modified: row.get(4)?,
                            attempts: row.get(5)?,
                        })
                    },
                )
                .optional()
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("query thumbnail dequeue failed: {}", err))
                })?;

            if let Some(task) = task.clone() {
                tx.execute(
                    "UPDATE file_thumbnails SET status='running', updated_at=?1
                     WHERE owner=?2 AND rel_path=?3 AND variant=?4",
                    params![now, task.owner, task.rel_path, task.variant],
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("mark thumbnail running failed: {}", err))
                })?;
            }

            tx.commit().map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "commit thumbnail dequeue transaction failed: {}",
                    err
                ))
            })?;
            Ok(task)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("dequeue thumbnail join error: {}", err)))?
    }

    async fn run_thumbnail_worker(&self) {
        loop {
            let task = match self.take_next_thumbnail_task().await {
                Ok(value) => value,
                Err(err) => {
                    warn!("thumbnail worker dequeue failed: {}", err);
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    continue;
                }
            };

            let Some(task) = task else {
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            };

            if let Err(err) = self
                .process_thumbnail_task(
                    &task.owner,
                    &task.rel_path,
                    &task.variant,
                    task.source_size,
                    task.source_modified,
                    task.attempts,
                )
                .await
            {
                warn!(
                    "thumbnail worker task failed owner={}, path={}, variant={}: {}",
                    task.owner, task.rel_path, task.variant, err
                );
                self.mark_thumbnail_task_failed(
                    &task.owner,
                    &task.rel_path,
                    &task.variant,
                    task.attempts,
                    err.to_string().as_str(),
                )
                .await;
            }
        }
    }

    async fn get_ready_thumbnail_rel_path(
        &self,
        owner: &str,
        rel_path_display: &str,
        variant: &str,
        source_size: i64,
        source_modified: i64,
    ) -> Result<Option<String>, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let rel_path_display = rel_path_display.to_string();
        let variant = variant.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<String>, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open thumbnail database failed: {}", err))
            })?;
            conn.query_row(
                "SELECT thumb_rel_path
                 FROM file_thumbnails
                 WHERE owner = ?1 AND rel_path = ?2 AND variant = ?3
                   AND status = 'ready'
                   AND source_size = ?4
                   AND source_modified = ?5
                   AND thumb_rel_path IS NOT NULL
                 LIMIT 1",
                params![
                    owner.as_str(),
                    rel_path_display,
                    variant.as_str(),
                    source_size,
                    source_modified,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|err| RPCErrors::ReasonError(format!("query ready thumbnail failed: {}", err)))
        })
        .await
        .map_err(|err| {
            RPCErrors::ReasonError(format!("query ready thumbnail join error: {}", err))
        })?
    }

    async fn sync_metadata_after_write_best_effort(&self, owner: &str, rel_path: &Path) {
        if Self::is_internal_rel_path(rel_path) {
            return;
        }

        if let Err(err) = self.sync_file_metadata_subtree(owner, rel_path).await {
            warn!(
                "sync file metadata failed for owner={}, path={}: {}",
                owner,
                Self::to_display_path(rel_path),
                err
            );
            return;
        }

        if let Err(err) = self.sync_thumbnail_tasks_for_scope(owner, rel_path).await {
            warn!(
                "sync thumbnail tasks failed for owner={}, path={}: {}",
                owner,
                Self::to_display_path(rel_path),
                err
            );
        }
    }

    async fn get_file_nav_neighbors(
        &self,
        owner: &str,
        rel_path: &Path,
    ) -> Result<(Option<FileEntry>, Option<FileEntry>), RPCErrors> {
        fn parse_row_to_file_entry(row: &rusqlite::Row<'_>) -> Result<FileEntry, rusqlite::Error> {
            let is_dir: i64 = row.get(2)?;
            let size: i64 = row.get(3)?;
            let modified: i64 = row.get(4)?;
            Ok(FileEntry {
                name: row.get(0)?,
                path: row.get(1)?,
                is_dir: is_dir != 0,
                size: size.max(0) as u64,
                modified: modified.max(0) as u64,
            })
        }

        let db_path = self.db_path();
        let owner = owner.to_string();
        let rel_display_path = Self::to_display_path(rel_path);
        tokio::task::spawn_blocking(
            move || -> Result<(Option<FileEntry>, Option<FileEntry>), RPCErrors> {
                let conn = Connection::open(db_path).map_err(|err| {
                    RPCErrors::ReasonError(format!("open metadata database failed: {}", err))
                })?;

                let current = conn
                    .query_row(
                        "SELECT parent_rel_path, lower_name, name, rel_path
                     FROM file_entries
                     WHERE owner = ?1 AND rel_path = ?2 AND is_dir = 0
                     LIMIT 1",
                        params![owner.as_str(), rel_display_path],
                        |row| {
                            let parent_rel_path: String = row.get(0)?;
                            let lower_name: String = row.get(1)?;
                            let name: String = row.get(2)?;
                            let rel_path: String = row.get(3)?;
                            Ok((parent_rel_path, lower_name, name, rel_path))
                        },
                    )
                    .optional()
                    .map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "query current metadata record failed: {}",
                            err
                        ))
                    })?;

                let Some((parent_rel_path, lower_name, name, rel_path)) = current else {
                    return Ok((None, None));
                };

                let previous = conn
                    .query_row(
                        "SELECT name, rel_path, is_dir, size, modified
                     FROM file_entries
                     WHERE owner = ?1
                       AND parent_rel_path = ?2
                       AND is_dir = 0
                       AND (
                         lower_name < ?3
                         OR (lower_name = ?3 AND name < ?4)
                         OR (lower_name = ?3 AND name = ?4 AND rel_path < ?5)
                       )
                     ORDER BY lower_name DESC, name DESC, rel_path DESC
                     LIMIT 1",
                        params![owner.as_str(), parent_rel_path, lower_name, name, rel_path],
                        parse_row_to_file_entry,
                    )
                    .optional()
                    .map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "query previous metadata record failed: {}",
                            err
                        ))
                    })?;

                let next = conn
                    .query_row(
                        "SELECT name, rel_path, is_dir, size, modified
                     FROM file_entries
                     WHERE owner = ?1
                       AND parent_rel_path = ?2
                       AND is_dir = 0
                       AND (
                         lower_name > ?3
                         OR (lower_name = ?3 AND name > ?4)
                         OR (lower_name = ?3 AND name = ?4 AND rel_path > ?5)
                       )
                     ORDER BY lower_name ASC, name ASC, rel_path ASC
                     LIMIT 1",
                        params![owner.as_str(), parent_rel_path, lower_name, name, rel_path],
                        parse_row_to_file_entry,
                    )
                    .optional()
                    .map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "query next metadata record failed: {}",
                            err
                        ))
                    })?;

                Ok((previous, next))
            },
        )
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("query metadata nav join error: {}", err)))?
    }

    async fn upsert_favorite_record(&self, owner: &str, rel_path: &str) -> Result<(), RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let rel_path = rel_path.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open favorites database failed: {}", err))
            })?;
            let now = BuckyFileServer::now_unix().min(i64::MAX as u64) as i64;
            conn.execute(
                "INSERT INTO file_favorites(owner, rel_path, created_at, updated_at)
                 VALUES(?1, ?2, ?3, ?3)
                 ON CONFLICT(owner, rel_path) DO UPDATE SET updated_at = excluded.updated_at",
                params![owner, rel_path, now],
            )
            .map_err(|err| RPCErrors::ReasonError(format!("upsert favorite failed: {}", err)))?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("upsert favorite join error: {}", err)))?
    }

    async fn delete_favorite_record(&self, owner: &str, rel_path: &str) -> Result<(), RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let rel_path = rel_path.to_string();
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open favorites database failed: {}", err))
            })?;
            conn.execute(
                "DELETE FROM file_favorites WHERE owner = ?1 AND rel_path = ?2",
                params![owner, rel_path],
            )
            .map_err(|err| RPCErrors::ReasonError(format!("delete favorite failed: {}", err)))?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("delete favorite join error: {}", err)))?
    }

    async fn list_favorite_entries(
        &self,
        owner: &str,
        filters: &FileListFilters,
    ) -> Result<Vec<FileEntry>, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let mut items =
            tokio::task::spawn_blocking(move || -> Result<Vec<FileEntry>, RPCErrors> {
                let conn = Connection::open(db_path).map_err(|err| {
                    RPCErrors::ReasonError(format!("open favorites database failed: {}", err))
                })?;

                let mut stmt = conn
                    .prepare(
                        "SELECT fe.name, fe.rel_path, fe.is_dir, fe.size, fe.modified
                     FROM file_favorites ff
                     JOIN file_entries fe
                       ON fe.owner = ff.owner
                      AND fe.rel_path = ff.rel_path
                     WHERE ff.owner = ?1
                     ORDER BY ff.updated_at DESC",
                    )
                    .map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "prepare favorites list query failed: {}",
                            err
                        ))
                    })?;

                let mut rows = stmt.query(params![owner.as_str()]).map_err(|err| {
                    RPCErrors::ReasonError(format!("query favorites list failed: {}", err))
                })?;

                let mut result = Vec::new();
                while let Some(row) = rows.next().map_err(|err| {
                    RPCErrors::ReasonError(format!("iterate favorites list failed: {}", err))
                })? {
                    let is_dir: i64 = row.get(2).map_err(|err| {
                        RPCErrors::ReasonError(format!("read favorites is_dir failed: {}", err))
                    })?;
                    let size: i64 = row.get(3).map_err(|err| {
                        RPCErrors::ReasonError(format!("read favorites size failed: {}", err))
                    })?;
                    let modified: i64 = row.get(4).map_err(|err| {
                        RPCErrors::ReasonError(format!("read favorites modified failed: {}", err))
                    })?;
                    result.push(FileEntry {
                        name: row.get(0).map_err(|err| {
                            RPCErrors::ReasonError(format!("read favorites name failed: {}", err))
                        })?,
                        path: row.get(1).map_err(|err| {
                            RPCErrors::ReasonError(format!("read favorites path failed: {}", err))
                        })?,
                        is_dir: is_dir != 0,
                        size: size.max(0) as u64,
                        modified: modified.max(0) as u64,
                    });
                }

                Ok(result)
            })
            .await
            .map_err(|err| {
                RPCErrors::ReasonError(format!("list favorites join error: {}", err))
            })??;

        items = Self::apply_file_list_filters(items, filters);
        Ok(items)
    }

    async fn touch_recent_record(&self, owner: &str, rel_path: &Path) -> Result<(), RPCErrors> {
        if rel_path.as_os_str().is_empty() || Self::is_internal_rel_path(rel_path) {
            return Ok(());
        }

        let db_path = self.db_path();
        let owner = owner.to_string();
        let rel_path = Self::to_display_path(rel_path);
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open recent database failed: {}", err))
            })?;
            let now = BuckyFileServer::now_unix().min(i64::MAX as u64) as i64;
            conn.execute(
                "INSERT INTO file_recent(owner, rel_path, last_accessed_at, access_count)
                 VALUES(?1, ?2, ?3, 1)
                 ON CONFLICT(owner, rel_path) DO UPDATE SET
                    last_accessed_at = excluded.last_accessed_at,
                    access_count = file_recent.access_count + 1",
                params![owner, rel_path, now],
            )
            .map_err(|err| RPCErrors::ReasonError(format!("touch recent failed: {}", err)))?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("touch recent join error: {}", err)))?
    }

    async fn list_recent_entries(
        &self,
        owner: &str,
        filters: &FileListFilters,
    ) -> Result<Vec<RecentFileEntry>, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let mut items =
            tokio::task::spawn_blocking(move || -> Result<Vec<RecentFileEntry>, RPCErrors> {
                let conn = Connection::open(db_path).map_err(|err| {
                    RPCErrors::ReasonError(format!("open recent database failed: {}", err))
                })?;

                let mut stmt = conn
                    .prepare(
                        "SELECT fe.name, fe.rel_path, fe.is_dir, fe.size, fe.modified,
                            fr.last_accessed_at, fr.access_count
                     FROM file_recent fr
                     JOIN file_entries fe
                       ON fe.owner = fr.owner
                      AND fe.rel_path = fr.rel_path
                     WHERE fr.owner = ?1
                     ORDER BY fr.last_accessed_at DESC",
                    )
                    .map_err(|err| {
                        RPCErrors::ReasonError(format!("prepare recent list query failed: {}", err))
                    })?;

                let mut rows = stmt.query(params![owner.as_str()]).map_err(|err| {
                    RPCErrors::ReasonError(format!("query recent list failed: {}", err))
                })?;

                let mut result = Vec::new();
                while let Some(row) = rows.next().map_err(|err| {
                    RPCErrors::ReasonError(format!("iterate recent list failed: {}", err))
                })? {
                    let is_dir: i64 = row.get(2).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recent is_dir failed: {}", err))
                    })?;
                    let size: i64 = row.get(3).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recent size failed: {}", err))
                    })?;
                    let modified: i64 = row.get(4).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recent modified failed: {}", err))
                    })?;
                    let last_accessed_at: i64 = row.get(5).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "read recent last_accessed_at failed: {}",
                            err
                        ))
                    })?;
                    let access_count: i64 = row.get(6).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recent access_count failed: {}", err))
                    })?;

                    result.push(RecentFileEntry {
                        entry: FileEntry {
                            name: row.get(0).map_err(|err| {
                                RPCErrors::ReasonError(format!("read recent name failed: {}", err))
                            })?,
                            path: row.get(1).map_err(|err| {
                                RPCErrors::ReasonError(format!("read recent path failed: {}", err))
                            })?,
                            is_dir: is_dir != 0,
                            size: size.max(0) as u64,
                            modified: modified.max(0) as u64,
                        },
                        last_accessed_at: last_accessed_at.max(0) as u64,
                        access_count: access_count.max(0) as u64,
                    });
                }

                Ok(result)
            })
            .await
            .map_err(|err| RPCErrors::ReasonError(format!("list recent join error: {}", err)))??;

        let filtered = Self::apply_file_list_filters(
            items.iter().map(|entry| entry.entry.clone()).collect(),
            filters,
        );
        let keep_paths = filtered
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<std::collections::HashSet<_>>();
        items.retain(|entry| keep_paths.contains(&entry.entry.path));

        if items.len() > filters.limit {
            items.truncate(filters.limit);
        }
        Ok(items)
    }

    async fn create_recycle_bin_item_record(
        &self,
        record: RecycleBinItemRecordInput,
    ) -> Result<(), RPCErrors> {
        let db_path = self.db_path();
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open recycle database failed: {}", err))
            })?;
            conn.execute(
                "INSERT INTO recycle_bin_items(
                    item_id, owner, original_rel_path, trashed_rel_path,
                    name, is_dir, size, modified, deleted_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    record.item_id,
                    record.owner,
                    record.original_rel_path,
                    record.trashed_rel_path,
                    record.name,
                    if record.is_dir { 1i64 } else { 0i64 },
                    record.size.min(i64::MAX as u64) as i64,
                    record.modified.min(i64::MAX as u64) as i64,
                    record.deleted_at.min(i64::MAX as u64) as i64,
                ],
            )
            .map_err(|err| {
                RPCErrors::ReasonError(format!("insert recycle bin item failed: {}", err))
            })?;
            Ok(())
        })
        .await
        .map_err(|err| {
            RPCErrors::ReasonError(format!("insert recycle bin item join error: {}", err))
        })?
    }

    async fn list_recycle_bin_items(
        &self,
        owner: &str,
        filters: &FileListFilters,
    ) -> Result<Vec<RecycleBinItem>, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let mut items =
            tokio::task::spawn_blocking(move || -> Result<Vec<RecycleBinItem>, RPCErrors> {
                let conn = Connection::open(db_path).map_err(|err| {
                    RPCErrors::ReasonError(format!("open recycle database failed: {}", err))
                })?;

                let mut stmt = conn
                .prepare(
                    "SELECT item_id, original_rel_path, name, is_dir, size, modified, deleted_at
                     FROM recycle_bin_items
                     WHERE owner = ?1
                     ORDER BY deleted_at DESC",
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("prepare recycle list query failed: {}", err))
                })?;

                let mut rows = stmt.query(params![owner.as_str()]).map_err(|err| {
                    RPCErrors::ReasonError(format!("query recycle list failed: {}", err))
                })?;

                let mut result = Vec::new();
                while let Some(row) = rows.next().map_err(|err| {
                    RPCErrors::ReasonError(format!("iterate recycle list failed: {}", err))
                })? {
                    let is_dir: i64 = row.get(3).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recycle is_dir failed: {}", err))
                    })?;
                    let size: i64 = row.get(4).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recycle size failed: {}", err))
                    })?;
                    let modified: i64 = row.get(5).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recycle modified failed: {}", err))
                    })?;
                    let deleted_at: i64 = row.get(6).map_err(|err| {
                        RPCErrors::ReasonError(format!("read recycle deleted_at failed: {}", err))
                    })?;
                    let original_path: String = row.get(1).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "read recycle original path failed: {}",
                            err
                        ))
                    })?;

                    result.push(RecycleBinItem {
                        item_id: row.get(0).map_err(|err| {
                            RPCErrors::ReasonError(format!("read recycle item id failed: {}", err))
                        })?,
                        entry: FileEntry {
                            name: row.get(2).map_err(|err| {
                                RPCErrors::ReasonError(format!("read recycle name failed: {}", err))
                            })?,
                            path: original_path.clone(),
                            is_dir: is_dir != 0,
                            size: size.max(0) as u64,
                            modified: modified.max(0) as u64,
                        },
                        original_path,
                        deleted_at: deleted_at.max(0) as u64,
                    });
                }

                Ok(result)
            })
            .await
            .map_err(|err| RPCErrors::ReasonError(format!("list recycle join error: {}", err)))??;

        let filtered = Self::apply_file_list_filters(
            items.iter().map(|entry| entry.entry.clone()).collect(),
            filters,
        );
        let keep_paths = filtered
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<std::collections::HashSet<_>>();
        items.retain(|entry| keep_paths.contains(&entry.entry.path));
        if items.len() > filters.limit {
            items.truncate(filters.limit);
        }
        Ok(items)
    }

    async fn get_recycle_bin_item(
        &self,
        owner: &str,
        item_id: &str,
    ) -> Result<Option<(String, String)>, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let item_id = item_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<(String, String)>, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open recycle database failed: {}", err))
            })?;

            conn.query_row(
                "SELECT original_rel_path, trashed_rel_path
                 FROM recycle_bin_items
                 WHERE owner = ?1 AND item_id = ?2
                 LIMIT 1",
                params![owner, item_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|err| RPCErrors::ReasonError(format!("query recycle item failed: {}", err)))
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("query recycle item join error: {}", err)))?
    }

    async fn delete_recycle_bin_item_record(
        &self,
        owner: &str,
        item_id: &str,
    ) -> Result<bool, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let item_id = item_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open recycle database failed: {}", err))
            })?;
            let rows = conn
                .execute(
                    "DELETE FROM recycle_bin_items WHERE owner = ?1 AND item_id = ?2",
                    params![owner, item_id],
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("delete recycle item record failed: {}", err))
                })?;
            Ok(rows > 0)
        })
        .await
        .map_err(|err| {
            RPCErrors::ReasonError(format!("delete recycle item record join error: {}", err))
        })?
    }

    async fn create_share_record(
        &self,
        owner: &str,
        path: &str,
        expires_at: Option<u64>,
        password_hash: Option<String>,
    ) -> Result<ShareItem, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> Result<ShareItem, RPCErrors> {
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open share database failed: {}", err)))?;

            let id = Uuid::new_v4().simple().to_string();
            let created_at = BuckyFileServer::now_unix();
            conn.execute(
                "INSERT INTO shares (id, owner, path, created_at, expires_at, password_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, owner, path, created_at as i64, expires_at.map(|v| v as i64), password_hash],
            )
            .map_err(|err| RPCErrors::ReasonError(format!("create share failed: {}", err)))?;

            Ok(ShareItem {
                id,
                owner,
                path,
                created_at,
                expires_at,
                password_required: password_hash.is_some(),
            })
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("create share join error: {}", err)))?
    }

    async fn list_share_records(&self, owner: &str) -> Result<Vec<ShareItem>, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        tokio::task::spawn_blocking(move || -> Result<Vec<ShareItem>, RPCErrors> {
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open share database failed: {}", err)))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, owner, path, created_at, expires_at, password_hash FROM shares WHERE owner = ?1 ORDER BY created_at DESC",
                )
                .map_err(|err| RPCErrors::ReasonError(format!("prepare share list query failed: {}", err)))?;

            let mut rows = stmt
                .query(params![owner])
                .map_err(|err| RPCErrors::ReasonError(format!("query share list failed: {}", err)))?;

            let mut result = Vec::new();
            while let Some(row) = rows
                .next()
                .map_err(|err| RPCErrors::ReasonError(format!("iterate share list failed: {}", err)))?
            {
                let id: String = row
                    .get(0)
                    .map_err(|err| RPCErrors::ReasonError(format!("read share id failed: {}", err)))?;
                let owner: String = row
                    .get(1)
                    .map_err(|err| RPCErrors::ReasonError(format!("read share owner failed: {}", err)))?;
                let path: String = row
                    .get(2)
                    .map_err(|err| RPCErrors::ReasonError(format!("read share path failed: {}", err)))?;
                let created_at: i64 = row
                    .get(3)
                    .map_err(|err| RPCErrors::ReasonError(format!("read share created_at failed: {}", err)))?;
                let expires_at: Option<i64> = row
                    .get(4)
                    .map_err(|err| RPCErrors::ReasonError(format!("read share expires_at failed: {}", err)))?;
                let password_hash: Option<String> = row
                    .get(5)
                    .map_err(|err| RPCErrors::ReasonError(format!("read share password hash failed: {}", err)))?;

                result.push(ShareItem {
                    id,
                    owner,
                    path,
                    created_at: created_at.max(0) as u64,
                    expires_at: expires_at.map(|v| v.max(0) as u64),
                    password_required: password_hash.is_some(),
                });
            }
            Ok(result)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("list share join error: {}", err)))?
    }

    async fn delete_share_record(&self, owner: &str, share_id: &str) -> Result<bool, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let share_id = share_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open share database failed: {}", err))
            })?;
            let rows = conn
                .execute(
                    "DELETE FROM shares WHERE id = ?1 AND owner = ?2",
                    params![share_id, owner],
                )
                .map_err(|err| RPCErrors::ReasonError(format!("delete share failed: {}", err)))?;
            Ok(rows > 0)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("delete share join error: {}", err)))?
    }

    async fn get_share_record(
        &self,
        share_id: &str,
    ) -> Result<Option<(ShareItem, Option<String>)>, RPCErrors> {
        let db_path = self.db_path();
        let share_id = share_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<(ShareItem, Option<String>)>, RPCErrors> {
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open share database failed: {}", err)))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, owner, path, created_at, expires_at, password_hash FROM shares WHERE id = ?1 LIMIT 1",
                )
                .map_err(|err| RPCErrors::ReasonError(format!("prepare share get query failed: {}", err)))?;

            let mut rows = stmt
                .query(params![share_id])
                .map_err(|err| RPCErrors::ReasonError(format!("query share failed: {}", err)))?;

            let Some(row) = rows
                .next()
                .map_err(|err| RPCErrors::ReasonError(format!("iterate share get failed: {}", err)))?
            else {
                return Ok(None);
            };

            let id: String = row
                .get(0)
                .map_err(|err| RPCErrors::ReasonError(format!("read share id failed: {}", err)))?;
            let owner: String = row
                .get(1)
                .map_err(|err| RPCErrors::ReasonError(format!("read share owner failed: {}", err)))?;
            let path: String = row
                .get(2)
                .map_err(|err| RPCErrors::ReasonError(format!("read share path failed: {}", err)))?;
            let created_at: i64 = row
                .get(3)
                .map_err(|err| RPCErrors::ReasonError(format!("read share created_at failed: {}", err)))?;
            let expires_at: Option<i64> = row
                .get(4)
                .map_err(|err| RPCErrors::ReasonError(format!("read share expires_at failed: {}", err)))?;
            let password_hash: Option<String> = row
                .get(5)
                .map_err(|err| RPCErrors::ReasonError(format!("read share password hash failed: {}", err)))?;

            Ok(Some((
                ShareItem {
                    id,
                    owner,
                    path,
                    created_at: created_at.max(0) as u64,
                    expires_at: expires_at.map(|v| v.max(0) as u64),
                    password_required: password_hash.is_some(),
                },
                password_hash,
            )))
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("get share join error: {}", err)))?
    }

    async fn create_upload_session_record(
        &self,
        owner: &str,
        path: &str,
        size: u64,
        chunk_size: u64,
        override_existing: bool,
    ) -> Result<UploadSessionRecord, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let path = path.to_string();
        tokio::task::spawn_blocking(move || -> Result<UploadSessionRecord, RPCErrors> {
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open upload database failed: {}", err)))?;

            let id = Uuid::new_v4().simple().to_string();
            let now = BuckyFileServer::now_unix();
            conn.execute(
                "INSERT INTO upload_sessions (id, owner, path, size, chunk_size, uploaded_size, override_existing, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    owner,
                    path,
                    size as i64,
                    chunk_size as i64,
                    0i64,
                    if override_existing { 1i64 } else { 0i64 },
                    now as i64,
                    now as i64,
                ],
            )
            .map_err(|err| RPCErrors::ReasonError(format!("create upload session failed: {}", err)))?;

            Ok(UploadSessionRecord {
                id,
                owner,
                path,
                size,
                chunk_size,
                uploaded_size: 0,
                override_existing,
                created_at: now,
                updated_at: now,
            })
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("create upload session join error: {}", err)))?
    }

    async fn get_upload_session_record(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<Option<UploadSessionRecord>, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<Option<UploadSessionRecord>, RPCErrors> {
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open upload database failed: {}", err)))?;

            let mut stmt = conn
                .prepare(
                    "SELECT id, owner, path, size, chunk_size, uploaded_size, override_existing, created_at, updated_at FROM upload_sessions WHERE id = ?1 AND owner = ?2 LIMIT 1",
                )
                .map_err(|err| RPCErrors::ReasonError(format!("prepare upload session query failed: {}", err)))?;

            let mut rows = stmt
                .query(params![session_id, owner])
                .map_err(|err| RPCErrors::ReasonError(format!("query upload session failed: {}", err)))?;

            let Some(row) = rows
                .next()
                .map_err(|err| RPCErrors::ReasonError(format!("iterate upload session failed: {}", err)))?
            else {
                return Ok(None);
            };

            let id: String = row
                .get(0)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload id failed: {}", err)))?;
            let owner: String = row
                .get(1)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload owner failed: {}", err)))?;
            let path: String = row
                .get(2)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload path failed: {}", err)))?;
            let size: i64 = row
                .get(3)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload size failed: {}", err)))?;
            let chunk_size: i64 = row
                .get(4)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload chunk_size failed: {}", err)))?;
            let uploaded_size: i64 = row
                .get(5)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload uploaded_size failed: {}", err)))?;
            let override_existing: i64 = row
                .get(6)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload override flag failed: {}", err)))?;
            let created_at: i64 = row
                .get(7)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload created_at failed: {}", err)))?;
            let updated_at: i64 = row
                .get(8)
                .map_err(|err| RPCErrors::ReasonError(format!("read upload updated_at failed: {}", err)))?;

            Ok(Some(UploadSessionRecord {
                id,
                owner,
                path,
                size: size.max(0) as u64,
                chunk_size: chunk_size.max(1) as u64,
                uploaded_size: uploaded_size.max(0) as u64,
                override_existing: override_existing != 0,
                created_at: created_at.max(0) as u64,
                updated_at: updated_at.max(0) as u64,
            }))
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("get upload session join error: {}", err)))?
    }

    async fn update_upload_session_progress(
        &self,
        owner: &str,
        session_id: &str,
        uploaded_size: u64,
    ) -> Result<bool, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool, RPCErrors> {
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open upload database failed: {}", err)))?;
            let now = BuckyFileServer::now_unix();
            let updated = conn
                .execute(
                    "UPDATE upload_sessions SET uploaded_size = ?1, updated_at = ?2 WHERE id = ?3 AND owner = ?4",
                    params![uploaded_size as i64, now as i64, session_id, owner],
                )
                .map_err(|err| RPCErrors::ReasonError(format!("update upload session failed: {}", err)))?;
            Ok(updated > 0)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("update upload session join error: {}", err)))?
    }

    async fn delete_upload_session_record(
        &self,
        owner: &str,
        session_id: &str,
    ) -> Result<bool, RPCErrors> {
        let db_path = self.db_path();
        let owner = owner.to_string();
        let session_id = session_id.to_string();
        tokio::task::spawn_blocking(move || -> Result<bool, RPCErrors> {
            let conn = Connection::open(db_path).map_err(|err| {
                RPCErrors::ReasonError(format!("open upload database failed: {}", err))
            })?;
            let deleted = conn
                .execute(
                    "DELETE FROM upload_sessions WHERE id = ?1 AND owner = ?2",
                    params![session_id, owner],
                )
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("delete upload session failed: {}", err))
                })?;
            Ok(deleted > 0)
        })
        .await
        .map_err(|err| {
            RPCErrors::ReasonError(format!("delete upload session join error: {}", err))
        })?
    }

    fn boxed_body(bytes: Vec<u8>) -> BoxBody<Bytes, ServerError> {
        BoxBody::new(
            Full::new(Bytes::from(bytes))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        )
    }

    fn json_response(
        status: StatusCode,
        value: Value,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let payload = serde_json::to_vec(&value).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "serialize json response failed: {}",
                err
            )
        })?;

        http::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "application/json")
            .body(Self::boxed_body(payload))
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build json response failed: {}",
                    err
                )
            })
    }

    fn unix_mtime(meta: &std::fs::Metadata) -> u64 {
        meta.modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    async fn read_body_bytes(
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<Vec<u8>> {
        let collected = req.into_body().collect().await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read request body failed: {}",
                err
            )
        })?;
        Ok(collected.to_bytes().to_vec())
    }

    async fn verify_trusted_token(&self, token: &str) -> Result<FileAuthPrincipal, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let parsed = runtime.verify_trusted_session_token(token).await?;
        let username = parsed
            .sub
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("trusted token missing subject".to_string()))?;

        Ok(FileAuthPrincipal { username })
    }

    fn extract_auth_token(req: &http::Request<BoxBody<Bytes, ServerError>>) -> Option<String> {
        if let Some(value) = req.headers().get("X-Auth") {
            if let Ok(token) = value.to_str() {
                if !token.trim().is_empty() {
                    return Some(token.trim().to_string());
                }
            }
        }

        if let Some(query) = req.uri().query() {
            for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                if key == "auth" && !value.trim().is_empty() {
                    return Some(value.to_string());
                }
            }
        }

        if let Some(cookie_header) = req.headers().get("Cookie") {
            if let Ok(raw_cookie) = cookie_header.to_str() {
                for piece in raw_cookie.split(';') {
                    let segment = piece.trim();
                    for key in ["auth=", "control-panel_token=", "control_panel_token="] {
                        if let Some(token) = segment.strip_prefix(key) {
                            if !token.trim().is_empty() {
                                return Some(token.trim().to_string());
                            }
                        }
                    }
                }
            }
        }

        None
    }

    fn can_read_rel_path(_principal: &FileAuthPrincipal, _rel_path: &Path) -> bool {
        true
    }

    fn can_write_rel_path(_principal: &FileAuthPrincipal, _rel_path: &Path) -> bool {
        true
    }

    fn user_root(&self, username: &str) -> PathBuf {
        if let Ok(root) = std::env::var("BUCKY_FILE_ROOT") {
            let trimmed = root.trim();
            if !trimmed.is_empty() {
                return PathBuf::from(trimmed).join(username);
            }
        }

        get_buckyos_root_dir()
            .join("data")
            .join("home")
            .join(username)
    }

    fn publish_root() -> PathBuf {
        get_buckyos_root_dir()
            .join("data")
            .join("srv")
            .join("publish")
    }

    async fn ensure_user_home(&self, principal: &FileAuthPrincipal) {
        let root = self.user_root(&principal.username);
        if let Err(err) = tokio::fs::create_dir_all(&root).await {
            warn!(
                "prepare user file root failed at {}: {}",
                root.display(),
                err
            );
            return;
        }

        let recycle_root = root.join(INTERNAL_RECYCLE_BIN_DIR);
        if let Err(err) = tokio::fs::create_dir_all(&recycle_root).await {
            warn!(
                "prepare recycle bin root failed for {} at {}: {}",
                principal.username,
                recycle_root.display(),
                err
            );
        }

        let public_link = root.join(USER_PUBLIC_LINK_NAME);
        let publish_root = Self::publish_root();
        let link_metadata = tokio::fs::symlink_metadata(&public_link).await;
        match link_metadata {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    match tokio::fs::read_link(&public_link).await {
                        Ok(target) if target == publish_root => {}
                        Ok(target) => {
                            warn!(
                                "user public link points to unexpected target for {}: {} -> {}",
                                principal.username,
                                public_link.display(),
                                target.display()
                            );
                        }
                        Err(err) => {
                            warn!(
                                "read user public link failed for {} at {}: {}",
                                principal.username,
                                public_link.display(),
                                err
                            );
                        }
                    }
                } else {
                    warn!(
                        "user public path exists but is not a symlink for {} at {}",
                        principal.username,
                        public_link.display()
                    );
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                #[cfg(unix)]
                {
                    if let Err(err) = std::os::unix::fs::symlink(&publish_root, &public_link) {
                        warn!(
                            "create user public symlink failed for {} at {} -> {}: {}",
                            principal.username,
                            public_link.display(),
                            publish_root.display(),
                            err
                        );
                    }
                }
                #[cfg(windows)]
                {
                    if let Err(err) = std::os::windows::fs::symlink_dir(&publish_root, &public_link)
                    {
                        warn!(
                            "create user public symlink failed for {} at {} -> {}: {}",
                            principal.username,
                            public_link.display(),
                            publish_root.display(),
                            err
                        );
                    }
                }
            }
            Err(err) => {
                warn!(
                    "inspect user public link failed for {} at {}: {}",
                    principal.username,
                    public_link.display(),
                    err
                );
            }
        }
    }

    fn forbidden_response(
        message: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        Self::json_response(StatusCode::FORBIDDEN, json!({"error": message}))
    }

    async fn auth_principal(
        &self,
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Result<FileAuthPrincipal, http::Response<BoxBody<Bytes, ServerError>>> {
        let token = match Self::extract_auth_token(req) {
            Some(v) => v,
            None => {
                return Err(Self::json_response(
                    StatusCode::UNAUTHORIZED,
                    json!({"error": "missing authentication token"}),
                )
                .unwrap_or_else(|_| {
                    http::Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Self::boxed_body(Vec::new()))
                        .unwrap_or_else(|_| unreachable!())
                }))
            }
        };

        if let Ok(principal) = self.verify_trusted_token(&token).await {
            self.ensure_user_home(&principal).await;
            return Ok(principal);
        }

        Err(Self::json_response(
            StatusCode::UNAUTHORIZED,
            json!({"error": "invalid authentication token"}),
        )
        .unwrap_or_else(|_| {
            http::Response::builder()
                .status(StatusCode::UNAUTHORIZED)
                .body(Self::boxed_body(Vec::new()))
                .unwrap_or_else(|_| unreachable!())
        }))
    }

    fn parse_relative_path(raw: &str) -> Result<PathBuf, RPCErrors> {
        fn hex_value(ch: u8) -> Option<u8> {
            match ch {
                b'0'..=b'9' => Some(ch - b'0'),
                b'a'..=b'f' => Some(ch - b'a' + 10),
                b'A'..=b'F' => Some(ch - b'A' + 10),
                _ => None,
            }
        }

        let mut decoded_bytes = Vec::with_capacity(raw.len());
        let raw_bytes = raw.as_bytes();
        let mut index = 0usize;
        while index < raw_bytes.len() {
            let current = raw_bytes[index];
            if current == b'%' {
                if index + 2 >= raw_bytes.len() {
                    return Err(RPCErrors::ReasonError(
                        "invalid percent-encoded path".to_string(),
                    ));
                }
                let hi = hex_value(raw_bytes[index + 1]).ok_or_else(|| {
                    RPCErrors::ReasonError("invalid percent-encoded path".to_string())
                })?;
                let lo = hex_value(raw_bytes[index + 2]).ok_or_else(|| {
                    RPCErrors::ReasonError("invalid percent-encoded path".to_string())
                })?;
                decoded_bytes.push((hi << 4) | lo);
                index += 3;
                continue;
            }

            decoded_bytes.push(current);
            index += 1;
        }

        let decoded = String::from_utf8(decoded_bytes)
            .map_err(|_| RPCErrors::ReasonError("invalid utf-8 path".to_string()))?;
        let trimmed = decoded.trim_start_matches('/');
        if trimmed.is_empty() {
            return Ok(PathBuf::new());
        }

        let mut relative = PathBuf::new();
        for comp in Path::new(trimmed).components() {
            match comp {
                Component::Normal(part) => relative.push(part),
                _ => {
                    return Err(RPCErrors::ReasonError(
                        "invalid path component in request".to_string(),
                    ))
                }
            }
        }
        Ok(relative)
    }

    fn to_display_path(relative: &Path) -> String {
        if relative.as_os_str().is_empty() {
            "/".to_string()
        } else {
            format!("/{}", relative.to_string_lossy().replace('\\', "/"))
        }
    }

    fn get_query_param(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        key: &str,
    ) -> Option<String> {
        req.uri().query().and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.to_string())
        })
    }

    fn parse_query_bool(value: &str) -> bool {
        let normalized = value.trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    }

    fn content_type_for_path(path: &Path) -> &'static str {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();

        match extension.as_str() {
            "txt" | "md" | "markdown" | "csv" | "log" => "text/plain; charset=utf-8",
            "json" => "application/json; charset=utf-8",
            "xml" => "application/xml; charset=utf-8",
            "yaml" | "yml" => "application/yaml; charset=utf-8",
            "toml" | "ini" | "conf" => "text/plain; charset=utf-8",
            "html" | "htm" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "js" | "mjs" => "application/javascript; charset=utf-8",
            "pdf" => "application/pdf",
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "webp" => "image/webp",
            "bmp" => "image/bmp",
            "svg" => "image/svg+xml",
            "doc" => "application/msword",
            "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "xls" => "application/vnd.ms-excel",
            "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "ppt" => "application/vnd.ms-powerpoint",
            "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            "odt" => "application/vnd.oasis.opendocument.text",
            "ods" => "application/vnd.oasis.opendocument.spreadsheet",
            "odp" => "application/vnd.oasis.opendocument.presentation",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "ogg" => "audio/ogg",
            "m4a" => "audio/mp4",
            "aac" => "audio/aac",
            "flac" => "audio/flac",
            "mp4" | "m4v" => "video/mp4",
            "mov" => "video/quicktime",
            "webm" => "video/webm",
            "ogv" => "video/ogg",
            "mkv" => "video/x-matroska",
            _ => "application/octet-stream",
        }
    }

    fn is_inline_text_extension(path: &Path) -> bool {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();

        if extension.is_empty() {
            return true;
        }

        matches!(
            extension.as_str(),
            "txt"
                | "md"
                | "markdown"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "ini"
                | "conf"
                | "log"
                | "csv"
                | "xml"
                | "js"
                | "mjs"
                | "cjs"
                | "css"
                | "html"
                | "htm"
                | "rs"
                | "py"
                | "go"
                | "java"
                | "kt"
                | "swift"
                | "c"
                | "h"
                | "cpp"
                | "hpp"
                | "sh"
                | "bash"
                | "zsh"
                | "sql"
                | "rb"
                | "php"
                | "scala"
                | "lua"
                | "dart"
                | "vue"
                | "svelte"
                | "ts"
                | "tsx"
                | "jsx"
        )
    }

    fn should_include_inline_file_content(
        path: &Path,
        file_size: u64,
        force_content: bool,
    ) -> bool {
        if force_content {
            return true;
        }
        if file_size > INLINE_TEXT_CONTENT_MAX_BYTES {
            return false;
        }
        Self::is_inline_text_extension(path)
    }

    fn preview_cache_dir(&self) -> PathBuf {
        self.data_folder.join("preview_cache")
    }

    fn is_pdf_preview_supported(path: &Path) -> bool {
        let extension = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_ascii_lowercase())
            .unwrap_or_default();
        PDF_PREVIEW_SUPPORTED_EXTENSIONS
            .iter()
            .any(|item| *item == extension)
    }

    fn build_pdf_preview_cache_key(source_path: &Path, metadata: &std::fs::Metadata) -> String {
        let modified_secs = metadata
            .modified()
            .ok()
            .and_then(|ts| ts.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        let payload = format!(
            "{}:{}:{}",
            source_path.display(),
            metadata.len(),
            modified_secs
        );
        let digest = Sha256::digest(payload.as_bytes());
        digest
            .iter()
            .map(|value| format!("{:02x}", value))
            .collect()
    }

    fn locate_generated_pdf_path(
        output_dir: &Path,
        source_path: &Path,
    ) -> Result<PathBuf, RPCErrors> {
        if let Some(stem) = source_path.file_stem().and_then(|value| value.to_str()) {
            let expected = output_dir.join(format!("{}.pdf", stem));
            if expected.exists() {
                return Ok(expected);
            }
        }

        let mut first_pdf: Option<PathBuf> = None;
        for entry in std::fs::read_dir(output_dir)
            .map_err(|err| RPCErrors::ReasonError(format!("read preview output failed: {}", err)))?
        {
            let entry = entry.map_err(|err| {
                RPCErrors::ReasonError(format!("read preview output entry failed: {}", err))
            })?;
            let path = entry.path();
            let is_pdf = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("pdf"))
                .unwrap_or(false);
            if is_pdf {
                first_pdf = Some(path);
                break;
            }
        }

        first_pdf.ok_or_else(|| {
            RPCErrors::ReasonError(
                "document conversion succeeded but no PDF output found".to_string(),
            )
        })
    }

    fn run_libreoffice_convert_to_pdf(
        source_path: &Path,
        output_dir: &Path,
    ) -> Result<(), RPCErrors> {
        let mut last_error: Option<String> = None;
        for binary in ["libreoffice", "soffice"] {
            let output = external_command(binary)
                .arg("--headless")
                .arg("--nologo")
                .arg("--nodefault")
                .arg("--nolockcheck")
                .arg("--nofirststartwizard")
                .arg("--convert-to")
                .arg("pdf")
                .arg("--outdir")
                .arg(output_dir)
                .arg(source_path)
                .output();

            match output {
                Ok(result) => {
                    if result.status.success() {
                        return Ok(());
                    }
                    let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
                    let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
                    let detail = if !stderr.is_empty() {
                        stderr
                    } else if !stdout.is_empty() {
                        stdout
                    } else {
                        format!("exit code {:?}", result.status.code())
                    };
                    last_error = Some(format!("{}: {}", binary, detail));
                }
                Err(err) => {
                    last_error = Some(format!("{}: {}", binary, err));
                }
            }
        }

        Err(RPCErrors::ReasonError(format!(
            "failed to convert document to PDF: {}",
            last_error.unwrap_or_else(|| "unknown converter error".to_string())
        )))
    }

    async fn ensure_pdf_preview_cache(
        &self,
        source_path: &Path,
        metadata: &std::fs::Metadata,
    ) -> Result<PathBuf, RPCErrors> {
        let cache_dir = self.preview_cache_dir();
        tokio::fs::create_dir_all(&cache_dir).await.map_err(|err| {
            RPCErrors::ReasonError(format!("prepare preview cache dir failed: {}", err))
        })?;

        let cache_key = Self::build_pdf_preview_cache_key(source_path, metadata);
        let cache_pdf_path = cache_dir.join(format!("{}.pdf", cache_key));

        if tokio::fs::metadata(&cache_pdf_path).await.is_ok() {
            return Ok(cache_pdf_path);
        }

        let source = source_path.to_path_buf();
        let cache_pdf = cache_pdf_path.clone();
        let cache_dir_clone = cache_dir.clone();
        tokio::task::spawn_blocking(move || -> Result<PathBuf, RPCErrors> {
            let temp_dir = cache_dir_clone.join(format!("tmp-{}", Uuid::new_v4().simple()));
            std::fs::create_dir_all(&temp_dir).map_err(|err| {
                RPCErrors::ReasonError(format!("prepare temporary preview dir failed: {}", err))
            })?;

            let convert_result = Self::run_libreoffice_convert_to_pdf(&source, &temp_dir)
                .and_then(|_| Self::locate_generated_pdf_path(&temp_dir, &source))
                .and_then(|generated_pdf| {
                    std::fs::copy(&generated_pdf, &cache_pdf).map_err(|err| {
                        RPCErrors::ReasonError(format!("store converted PDF failed: {}", err))
                    })?;
                    Ok(())
                });

            let cleanup_result = std::fs::remove_dir_all(&temp_dir);
            if let Err(err) = cleanup_result {
                warn!(
                    "cleanup temporary preview dir failed at {}: {}",
                    temp_dir.display(),
                    err
                );
            }

            convert_result.map(|_| cache_pdf)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("preview conversion task failed: {}", err)))?
    }

    fn sanitize_archive_name(name: &str) -> String {
        let sanitized = name
            .trim()
            .chars()
            .map(|ch| {
                if matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                    '_'
                } else {
                    ch
                }
            })
            .collect::<String>();
        if sanitized.is_empty() {
            "download".to_string()
        } else {
            sanitized
        }
    }

    fn archive_name_from_path(path: &Path, fallback: &str) -> String {
        let raw = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or(fallback);
        Self::sanitize_archive_name(raw)
    }

    fn zip_file_options() -> FileOptions<'static, ()> {
        FileOptions::<()>::default().compression_method(CompressionMethod::Deflated)
    }

    fn normalize_archive_relative_path(relative_path: &Path) -> String {
        relative_path
            .components()
            .filter_map(|component| match component {
                Component::Normal(value) => Some(value.to_string_lossy().to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/")
    }

    fn zip_directory_recursive<W: Write + std::io::Seek>(
        zip: &mut zip::ZipWriter<W>,
        source_root: &Path,
        current_dir: &Path,
        archive_root_name: &str,
    ) -> Result<(), ServerError> {
        let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(current_dir)
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read directory for archive failed: {}",
                    err
                )
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read directory entry for archive failed: {}",
                    err
                )
            })?;

        entries.sort_by(|a, b| {
            a.file_name()
                .to_string_lossy()
                .to_lowercase()
                .cmp(&b.file_name().to_string_lossy().to_lowercase())
        });

        for entry in entries {
            let entry_path = entry.path();
            let metadata = entry.metadata().map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read archive entry metadata failed: {}",
                    err
                )
            })?;

            let relative_path = entry_path.strip_prefix(source_root).map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "strip archive source prefix failed: {}",
                    err
                )
            })?;
            let relative_text = Self::normalize_archive_relative_path(relative_path);
            if relative_text.is_empty() {
                continue;
            }
            let archive_path = format!("{}/{}", archive_root_name, relative_text);

            if metadata.is_dir() {
                let dir_name = format!("{}/", archive_path);
                zip.add_directory(dir_name, Self::zip_file_options())
                    .map_err(|err| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "write archive directory failed: {}",
                            err
                        )
                    })?;
                Self::zip_directory_recursive(zip, source_root, &entry_path, archive_root_name)?;
                continue;
            }

            if !metadata.is_file() {
                continue;
            }

            zip.start_file(archive_path, Self::zip_file_options())
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "start archive file failed: {}",
                        err
                    )
                })?;

            let mut file_reader = std::fs::File::open(&entry_path).map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "open archive source file failed: {}",
                    err
                )
            })?;
            let mut buffer = Vec::new();
            file_reader.read_to_end(&mut buffer).map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read archive source file failed: {}",
                    err
                )
            })?;
            zip.write_all(&buffer).map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "write archive content failed: {}",
                    err
                )
            })?;
        }

        Ok(())
    }

    async fn build_directory_archive(
        source_dir: PathBuf,
        archive_root_name: String,
    ) -> Result<Vec<u8>, ServerError> {
        tokio::task::spawn_blocking(move || -> Result<Vec<u8>, ServerError> {
            let cursor = std::io::Cursor::new(Vec::<u8>::new());
            let mut zip = zip::ZipWriter::new(cursor);

            let root_dir_name = format!("{}/", archive_root_name);
            zip.add_directory(root_dir_name, Self::zip_file_options())
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "initialize archive root failed: {}",
                        err
                    )
                })?;

            Self::zip_directory_recursive(&mut zip, &source_dir, &source_dir, &archive_root_name)?;

            let cursor = zip.finish().map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "finalize archive failed: {}",
                    err
                )
            })?;
            Ok(cursor.into_inner())
        })
        .await
        .map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "directory archive task join failed: {}",
                err
            )
        })?
    }

    fn parse_raw_range_header(range_header: &str, file_size: u64) -> RawByteRange {
        if !range_header.starts_with("bytes=") {
            return RawByteRange::Full;
        }

        let range_value = range_header[6..].trim();
        if range_value.is_empty() || range_value.contains(',') {
            return RawByteRange::Full;
        }

        let Some((start_raw, end_raw)) = range_value.split_once('-') else {
            return RawByteRange::Full;
        };

        if start_raw.is_empty() {
            let suffix_length = match end_raw.trim().parse::<u64>() {
                Ok(value) if value > 0 => value,
                _ => return RawByteRange::Unsatisfiable,
            };

            if file_size == 0 {
                return RawByteRange::Unsatisfiable;
            }

            let actual_length = suffix_length.min(file_size);
            return RawByteRange::Partial {
                start: file_size - actual_length,
                end: file_size - 1,
            };
        }

        let start = match start_raw.trim().parse::<u64>() {
            Ok(value) => value,
            Err(_) => return RawByteRange::Full,
        };

        if file_size == 0 || start >= file_size {
            return RawByteRange::Unsatisfiable;
        }

        if end_raw.trim().is_empty() {
            return RawByteRange::Partial {
                start,
                end: file_size - 1,
            };
        }

        let end = match end_raw.trim().parse::<u64>() {
            Ok(value) => value,
            Err(_) => return RawByteRange::Full,
        };

        if end < start {
            return RawByteRange::Unsatisfiable;
        }

        RawByteRange::Partial {
            start,
            end: end.min(file_size - 1),
        }
    }

    fn parse_upload_chunk_size(input: Option<u64>) -> u64 {
        const DEFAULT_CHUNK_SIZE: u64 = 2 * 1024 * 1024;
        const MIN_CHUNK_SIZE: u64 = 64 * 1024;
        const MAX_CHUNK_SIZE: u64 = 16 * 1024 * 1024;

        input
            .unwrap_or(DEFAULT_CHUNK_SIZE)
            .clamp(MIN_CHUNK_SIZE, MAX_CHUNK_SIZE)
    }

    fn parse_upload_session_id(raw: &str) -> Result<String, RPCErrors> {
        let value = raw.trim();
        if value.is_empty() {
            return Err(RPCErrors::ReasonError(
                "upload session id is required".to_string(),
            ));
        }
        if !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err(RPCErrors::ReasonError(
                "invalid upload session id".to_string(),
            ));
        }
        Ok(value.to_string())
    }

    async fn handle_api_upload_session_create(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let body = Self::read_body_bytes(req).await?;
        let payload: CreateUploadSessionRequest = serde_json::from_slice(&body).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "invalid upload session payload: {}",
                err
            )
        })?;

        let rel_path = match Self::parse_relative_path(&payload.path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is required"}),
            );
        }
        if !Self::can_write_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: write access to target path is required",
            );
        }

        let chunk_size = Self::parse_upload_chunk_size(payload.chunk_size);
        let override_existing = payload.override_existing.unwrap_or(false);
        let target = self.user_root(&principal.username).join(&rel_path);
        if target.exists() && !override_existing {
            return Self::json_response(
                StatusCode::CONFLICT,
                json!({"error": "target file exists"}),
            );
        }

        tokio::fs::create_dir_all(self.upload_tmp_dir())
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "prepare upload temp directory failed: {}",
                    err
                )
            })?;

        let session = self
            .create_upload_session_record(
                &principal.username,
                &Self::to_display_path(&rel_path),
                payload.size,
                chunk_size,
                override_existing,
            )
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "create upload session record failed: {}",
                    err
                )
            })?;

        let tmp_path = self.upload_tmp_path(&session.id);
        tokio::fs::write(&tmp_path, &[]).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "prepare upload session temp file failed: {}",
                err
            )
        })?;

        Self::json_response(
            StatusCode::CREATED,
            json!({
                "session": session,
                "completed": payload.size == 0,
            }),
        )
    }

    async fn handle_api_upload_session_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        session_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let session_id = match Self::parse_upload_session_id(session_id) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let Some(mut session) = self
            .get_upload_session_record(&principal.username, &session_id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "load upload session record failed: {}",
                    err
                )
            })?
        else {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "upload session not found"}),
            );
        };

        let tmp_path = self.upload_tmp_path(&session.id);
        if let Ok(meta) = tokio::fs::metadata(&tmp_path).await {
            let actual_uploaded = meta.len();
            if actual_uploaded != session.uploaded_size {
                let _ = self
                    .update_upload_session_progress(
                        &principal.username,
                        &session.id,
                        actual_uploaded,
                    )
                    .await;
                session.uploaded_size = actual_uploaded;
                session.updated_at = Self::now_unix();
            }
        }

        Self::json_response(
            StatusCode::OK,
            json!({
                "session": session,
                "completed": session.uploaded_size >= session.size,
            }),
        )
    }

    async fn handle_api_upload_session_put(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        session_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let session_id = match Self::parse_upload_session_id(session_id) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let Some(session) = self
            .get_upload_session_record(&principal.username, &session_id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "load upload session record failed: {}",
                    err
                )
            })?
        else {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "upload session not found"}),
            );
        };

        let offset = Self::get_query_param(&req, "offset")
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(session.uploaded_size);
        if offset != session.uploaded_size {
            return Self::json_response(
                StatusCode::CONFLICT,
                json!({
                    "error": "chunk offset mismatch",
                    "expected_offset": session.uploaded_size,
                }),
            );
        }

        let bytes = Self::read_body_bytes(req).await?;
        let chunk_size = bytes.len() as u64;
        if chunk_size == 0 {
            return Self::json_response(
                StatusCode::OK,
                json!({
                    "ok": true,
                    "session_id": session.id,
                    "uploaded_size": session.uploaded_size,
                    "size": session.size,
                    "completed": session.uploaded_size >= session.size,
                }),
            );
        }

        if session.uploaded_size.saturating_add(chunk_size) > session.size {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "chunk exceeds expected file size"}),
            );
        }

        let tmp_path = self.upload_tmp_path(&session.id);
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&tmp_path)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "open upload temp file failed: {}",
                    err
                )
            })?;
        file.seek(SeekFrom::Start(offset)).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "seek upload temp file failed: {}",
                err
            )
        })?;
        file.write_all(&bytes).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "write upload chunk failed: {}",
                err
            )
        })?;
        file.flush().await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "flush upload chunk failed: {}",
                err
            )
        })?;

        let next_uploaded_size = session.uploaded_size + chunk_size;
        self.update_upload_session_progress(&principal.username, &session.id, next_uploaded_size)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "update upload progress failed: {}",
                    err
                )
            })?;

        Self::json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "session_id": session.id,
                "uploaded_size": next_uploaded_size,
                "size": session.size,
                "completed": next_uploaded_size >= session.size,
            }),
        )
    }

    async fn handle_api_upload_session_complete(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        session_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let session_id = match Self::parse_upload_session_id(session_id) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let Some(mut session) = self
            .get_upload_session_record(&principal.username, &session_id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "load upload session record failed: {}",
                    err
                )
            })?
        else {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "upload session not found"}),
            );
        };

        let tmp_path = self.upload_tmp_path(&session.id);
        let actual_uploaded = tokio::fs::metadata(&tmp_path)
            .await
            .map(|meta| meta.len())
            .unwrap_or(session.uploaded_size);
        if actual_uploaded != session.uploaded_size {
            let _ = self
                .update_upload_session_progress(&principal.username, &session.id, actual_uploaded)
                .await;
            session.uploaded_size = actual_uploaded;
        }

        if session.uploaded_size < session.size {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({
                    "error": "upload is not complete",
                    "uploaded_size": session.uploaded_size,
                    "size": session.size,
                }),
            );
        }

        let rel_path = match Self::parse_relative_path(&session.path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is required"}),
            );
        }
        if !Self::can_write_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: write access to target path is required",
            );
        }

        let target = self.user_root(&principal.username).join(&rel_path);
        if target.exists() {
            if !session.override_existing {
                return Self::json_response(
                    StatusCode::CONFLICT,
                    json!({"error": "target file exists"}),
                );
            }
            let meta = tokio::fs::metadata(&target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read target metadata before overwrite failed: {}",
                    err
                )
            })?;
            if meta.is_dir() {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "target path is a directory"}),
                );
            }
            tokio::fs::remove_file(&target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "remove existing target file failed: {}",
                    err
                )
            })?;
        }

        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "create parent directory for upload complete failed: {}",
                    err
                )
            })?;
        }

        if let Err(err) = tokio::fs::rename(&tmp_path, &target).await {
            warn!(
                "rename upload temp file failed, fallback to copy+remove. tmp={}, target={}, err={}",
                tmp_path.display(),
                target.display(),
                err
            );
            tokio::fs::copy(&tmp_path, &target)
                .await
                .map_err(|copy_err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "copy upload temp file failed: {}",
                        copy_err
                    )
                })?;
            let _ = tokio::fs::remove_file(&tmp_path).await;
        }

        let _ = self
            .delete_upload_session_record(&principal.username, &session.id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "delete upload session record failed: {}",
                    err
                )
            })?;

        self.sync_metadata_after_write_best_effort(&principal.username, &rel_path)
            .await;

        Self::json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "path": Self::to_display_path(&rel_path),
                "size": session.size,
            }),
        )
    }

    async fn handle_api_upload_session_delete(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        session_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let session_id = match Self::parse_upload_session_id(session_id) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let deleted = self
            .delete_upload_session_record(&principal.username, &session_id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "delete upload session record failed: {}",
                    err
                )
            })?;

        let tmp_path = self.upload_tmp_path(&session_id);
        let _ = tokio::fs::remove_file(&tmp_path).await;

        if !deleted {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "upload session not found"}),
            );
        }

        Self::json_response(StatusCode::OK, json!({"ok": true}))
    }

    fn get_share_password(req: &http::Request<BoxBody<Bytes, ServerError>>) -> Option<String> {
        if let Some(value) = req.headers().get("X-Share-Password") {
            if let Ok(password) = value.to_str() {
                let password = password.trim();
                if !password.is_empty() {
                    return Some(password.to_string());
                }
            }
        }
        Self::get_query_param(req, "password")
    }

    async fn handle_api_share_create(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let body = Self::read_body_bytes(req).await?;
        let payload: CreateShareRequest = serde_json::from_slice(&body).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "invalid share create payload: {}",
                err
            )
        })?;

        let rel_path = match Self::parse_relative_path(&payload.path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "share path is required"}),
            );
        }
        if !Self::can_write_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: share can only be created for writable paths",
            );
        }

        let target = self.user_root(&principal.username).join(&rel_path);
        if !target.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "path not found"}));
        }

        let now = Self::now_unix();
        let expires_at = payload
            .expires_in_seconds
            .map(|seconds| now.saturating_add(seconds));
        let password_hash = Self::hash_optional_password(payload.password.as_deref());
        let share_item = self
            .create_share_record(
                &principal.username,
                &Self::to_display_path(&rel_path),
                expires_at,
                password_hash,
            )
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "create share record failed: {}",
                    err
                )
            })?;

        Self::json_response(
            StatusCode::CREATED,
            json!({
                "item": share_item,
                "public_view_url": format!("/share/{}", share_item.id),
                "public_download_url": format!("/api/public/dl/{}", share_item.id),
            }),
        )
    }

    async fn handle_api_share_list(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let items = self
            .list_share_records(&principal.username)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "list share records failed: {}",
                    err
                )
            })?;
        Self::json_response(StatusCode::OK, json!({"items": items}))
    }

    async fn handle_api_share_delete(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        share_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        if share_id.trim().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "share id is required"}),
            );
        }

        let deleted = self
            .delete_share_record(&principal.username, share_id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "delete share record failed: {}",
                    err
                )
            })?;

        if !deleted {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "share not found"}));
        }

        Self::json_response(StatusCode::OK, json!({"ok": true}))
    }

    async fn resolve_public_share(
        &self,
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        share_id: &str,
    ) -> Result<(ShareItem, PathBuf, PathBuf), http::Response<BoxBody<Bytes, ServerError>>> {
        let Some((share_item, password_hash)) =
            self.get_share_record(share_id).await.map_err(|err| {
                Self::json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": format!("load share failed: {}", err)}),
                )
                .unwrap_or_else(|_| {
                    http::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(Self::boxed_body(Vec::new()))
                        .unwrap_or_else(|_| unreachable!())
                })
            })?
        else {
            return Err(Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "share not found"}),
            )
            .unwrap_or_else(|_| {
                http::Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Self::boxed_body(Vec::new()))
                    .unwrap_or_else(|_| unreachable!())
            }));
        };

        if let Some(expires_at) = share_item.expires_at {
            if expires_at <= Self::now_unix() {
                return Err(Self::json_response(
                    StatusCode::GONE,
                    json!({"error": "share expired"}),
                )
                .unwrap_or_else(|_| {
                    http::Response::builder()
                        .status(StatusCode::GONE)
                        .body(Self::boxed_body(Vec::new()))
                        .unwrap_or_else(|_| unreachable!())
                }));
            }
        }

        if let Some(stored_hash) = password_hash {
            let provided_hash =
                Self::hash_optional_password(Self::get_share_password(req).as_deref());
            if provided_hash.as_deref() != Some(stored_hash.as_str()) {
                return Err(Self::json_response(
                    StatusCode::UNAUTHORIZED,
                    json!({"error": "share password required or invalid"}),
                )
                .unwrap_or_else(|_| {
                    http::Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Self::boxed_body(Vec::new()))
                        .unwrap_or_else(|_| unreachable!())
                }));
            }
        }

        let share_root_rel_path = Self::parse_relative_path(&share_item.path).map_err(|_| {
            Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "invalid shared path"}),
            )
            .unwrap_or_else(|_| {
                http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Self::boxed_body(Vec::new()))
                    .unwrap_or_else(|_| unreachable!())
            })
        })?;

        let sub_path = Self::get_query_param(req, "path").unwrap_or_else(|| "/".to_string());
        let sub_rel_path = Self::parse_relative_path(&sub_path).map_err(|_| {
            Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "invalid shared relative path"}),
            )
            .unwrap_or_else(|_| {
                http::Response::builder()
                    .status(StatusCode::BAD_REQUEST)
                    .body(Self::boxed_body(Vec::new()))
                    .unwrap_or_else(|_| unreachable!())
            })
        })?;

        let share_root = self.user_root(&share_item.owner).join(&share_root_rel_path);
        let target = share_root.join(&sub_rel_path);
        if !target.exists() {
            return Err(Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "shared path not found"}),
            )
            .unwrap_or_else(|_| {
                http::Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .body(Self::boxed_body(Vec::new()))
                    .unwrap_or_else(|_| unreachable!())
            }));
        }

        Ok((share_item, share_root, target))
    }

    async fn handle_api_public_share_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        share_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let (share_item, share_root, target) = match self.resolve_public_share(&req, share_id).await
        {
            Ok(value) => value,
            Err(resp) => return Ok(resp),
        };

        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read shared metadata failed: {}",
                err
            )
        })?;

        let target_rel_path = target
            .strip_prefix(&share_root)
            .map(|path| path.to_path_buf())
            .unwrap_or_else(|_| PathBuf::new());
        let display_path = Self::to_display_path(&target_rel_path);
        let parent_display_path = target_rel_path
            .parent()
            .map(Self::to_display_path)
            .unwrap_or_else(|| "/".to_string());

        if metadata.is_dir() {
            let mut items: Vec<FileEntry> = Vec::new();
            let mut reader = tokio::fs::read_dir(&target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read shared directory failed: {}",
                    err
                )
            })?;

            while let Some(entry) = reader.next_entry().await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read shared dir entry failed: {}",
                    err
                )
            })? {
                let entry_path = entry.path();
                let entry_meta = tokio::fs::metadata(&entry_path).await.map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "read shared entry metadata failed: {}",
                        err
                    )
                })?;
                let name = entry.file_name().to_string_lossy().to_string();
                let item_rel_path = if target_rel_path.as_os_str().is_empty() {
                    PathBuf::from(&name)
                } else {
                    target_rel_path.join(&name)
                };
                items.push(FileEntry {
                    path: Self::to_display_path(&item_rel_path),
                    name,
                    is_dir: entry_meta.is_dir(),
                    size: if entry_meta.is_file() {
                        entry_meta.len()
                    } else {
                        0
                    },
                    modified: Self::unix_mtime(&entry_meta),
                });
            }

            items.sort_by(|a, b| {
                if a.is_dir == b.is_dir {
                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                } else if a.is_dir {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            });

            return Self::json_response(
                StatusCode::OK,
                json!({
                    "share": share_item,
                    "is_dir": true,
                    "path": display_path,
                    "parent_path": parent_display_path,
                    "items": items,
                }),
            );
        }

        let content =
            if Self::should_include_inline_file_content(&target_rel_path, metadata.len(), false) {
                let file_data = tokio::fs::read(&target).await.map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "read shared file failed: {}",
                        err
                    )
                })?;
                String::from_utf8(file_data).ok()
            } else {
                None
            };
        Self::json_response(
            StatusCode::OK,
            json!({
                "share": share_item,
                "is_dir": false,
                "path": display_path,
                "parent_path": parent_display_path,
                "size": metadata.len(),
                "modified": Self::unix_mtime(&metadata),
                "mime": Self::content_type_for_path(&target),
                "content": content,
            }),
        )
    }

    async fn handle_api_public_download_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        share_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let (share_item, share_root, target) = match self.resolve_public_share(&req, share_id).await
        {
            Ok(value) => value,
            Err(resp) => return Ok(resp),
        };

        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read shared metadata failed: {}",
                err
            )
        })?;
        if metadata.is_dir() {
            let archive_base_name = if target == share_root {
                Self::archive_name_from_path(Path::new(&share_item.path), "shared-folder")
            } else {
                Self::archive_name_from_path(&target, "shared-folder")
            };
            let archive_bytes =
                Self::build_directory_archive(target.clone(), archive_base_name.clone()).await?;

            return http::Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/zip")
                .header(
                    CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}.zip\"", archive_base_name),
                )
                .header(CONTENT_LENGTH, archive_bytes.len().to_string())
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(archive_bytes))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build public directory download response failed: {}",
                        err
                    )
                });
        }

        let force_download = Self::get_query_param(&req, "download")
            .map(|value| Self::parse_query_bool(&value))
            .unwrap_or(true);
        let force_inline = Self::get_query_param(&req, "inline")
            .map(|value| Self::parse_query_bool(&value))
            .unwrap_or(false);

        let content = tokio::fs::read(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read shared file failed: {}",
                err
            )
        })?;
        let content_type = Self::content_type_for_path(&target);
        let filename = target
            .file_name()
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_else(|| "download.bin".to_string());
        let disposition_type = if force_inline || !force_download {
            "inline"
        } else {
            "attachment"
        };

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, content_type)
            .header(
                CONTENT_DISPOSITION,
                format!("{}; filename=\"{}\"", disposition_type, filename),
            )
            .header(CACHE_CONTROL, "no-store")
            .header(CONTENT_LENGTH, content.len().to_string())
            .body(Self::boxed_body(content))
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build public download response failed: {}",
                    err
                )
            })
    }

    async fn handle_api_resources_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        if let Err(err) = self.ensure_owner_metadata_seeded(&principal.username).await {
            warn!(
                "ensure metadata seeded failed for owner={}: {}",
                principal.username, err
            );
        }

        let rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response("permission denied: read access to path is required");
        }

        let target = self.user_root(&principal.username).join(&rel_path);
        if !target.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "path not found"}));
        }

        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read metadata failed: {}",
                err
            )
        })?;

        if metadata.is_dir() {
            self.sync_metadata_after_write_best_effort(&principal.username, &rel_path)
                .await;

            let mut items: Vec<FileEntry> = Vec::new();
            let mut reader = tokio::fs::read_dir(&target).await.map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "read dir failed: {}", err)
            })?;

            while let Some(entry) = reader.next_entry().await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read dir entry failed: {}",
                    err
                )
            })? {
                let file_name = entry.file_name().to_string_lossy().to_string();
                if file_name == INTERNAL_RECYCLE_BIN_DIR {
                    continue;
                }
                let entry_path = entry.path();
                let entry_meta = tokio::fs::metadata(&entry_path).await.map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "read entry metadata failed: {}",
                        err
                    )
                })?;

                let item_rel_path = if rel_path.as_os_str().is_empty() {
                    PathBuf::from(&file_name)
                } else {
                    rel_path.join(&file_name)
                };
                if Self::is_internal_rel_path(&item_rel_path) {
                    continue;
                }
                if !Self::can_read_rel_path(&principal, &item_rel_path) {
                    continue;
                }
                items.push(FileEntry {
                    name: file_name,
                    path: Self::to_display_path(&item_rel_path),
                    is_dir: entry_meta.is_dir(),
                    size: if entry_meta.is_file() {
                        entry_meta.len()
                    } else {
                        0
                    },
                    modified: Self::unix_mtime(&entry_meta),
                });
            }

            items.sort_by(|a, b| {
                if a.is_dir == b.is_dir {
                    a.name.to_lowercase().cmp(&b.name.to_lowercase())
                } else if a.is_dir {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            });

            return Self::json_response(
                StatusCode::OK,
                json!(DirectoryResponse {
                    path: Self::to_display_path(&rel_path),
                    is_dir: true,
                    items,
                }),
            );
        }

        let force_content = Self::get_query_param(&req, "content")
            .map(|value| Self::parse_query_bool(&value))
            .unwrap_or(false);
        let content =
            if Self::should_include_inline_file_content(&rel_path, metadata.len(), force_content) {
                let file_data = tokio::fs::read(&target).await.map_err(|err| {
                    server_err!(ServerErrorCode::InvalidData, "read file failed: {}", err)
                })?;
                String::from_utf8(file_data).ok()
            } else {
                None
            };

        if let Err(err) = self
            .touch_recent_record(&principal.username, &rel_path)
            .await
        {
            warn!(
                "touch recent for file detail failed owner={}, path={}: {}",
                principal.username,
                Self::to_display_path(&rel_path),
                err
            );
        }

        Self::json_response(
            StatusCode::OK,
            json!(FileResponse {
                path: Self::to_display_path(&rel_path),
                is_dir: false,
                size: metadata.len(),
                modified: Self::unix_mtime(&metadata),
                content,
            }),
        )
    }

    async fn handle_api_resources_post(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
        create_dir: bool,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is required"}),
            );
        }
        if !Self::can_write_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: write access to target path is required",
            );
        }

        let should_override = req
            .uri()
            .query()
            .map(|query| {
                url::form_urlencoded::parse(query.as_bytes())
                    .any(|(k, v)| k == "override" && v == "true")
            })
            .unwrap_or(false);

        let target = self.user_root(&principal.username).join(&rel_path);

        if create_dir {
            tokio::fs::create_dir_all(&target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "create directory failed: {}",
                    err
                )
            })?;
            self.sync_metadata_after_write_best_effort(&principal.username, &rel_path)
                .await;
            return Self::json_response(
                StatusCode::CREATED,
                json!({"ok": true, "path": Self::to_display_path(&rel_path)}),
            );
        }

        if target.exists() && !should_override {
            return Self::json_response(
                StatusCode::CONFLICT,
                json!({"error": "target file exists"}),
            );
        }

        let parent = match target.parent() {
            Some(v) => v,
            None => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "invalid target path"}),
                )
            }
        };
        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "create parent directory failed: {}",
                err
            )
        })?;

        let bytes = Self::read_body_bytes(req).await?;
        tokio::fs::write(&target, bytes).await.map_err(|err| {
            server_err!(ServerErrorCode::InvalidData, "write file failed: {}", err)
        })?;

        self.sync_metadata_after_write_best_effort(&principal.username, &rel_path)
            .await;

        Self::json_response(
            StatusCode::OK,
            json!({"ok": true, "path": Self::to_display_path(&rel_path)}),
        )
    }

    async fn handle_api_resources_put(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is required"}),
            );
        }
        if !Self::can_write_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: write access to target path is required",
            );
        }

        let content_type = req
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();

        let bytes = Self::read_body_bytes(req).await?;
        let content_bytes = if content_type.starts_with("application/json") {
            let payload: PutFileRequest = serde_json::from_slice(&bytes).map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "invalid put payload for file content: {}",
                    err
                )
            })?;
            payload.content.into_bytes()
        } else {
            bytes
        };

        let target = self.user_root(&principal.username).join(&rel_path);
        if target.exists() {
            let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read metadata failed: {}",
                    err
                )
            })?;
            if metadata.is_dir() {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "target path is a directory"}),
                );
            }
        }

        let parent = match target.parent() {
            Some(v) => v,
            None => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "invalid target path"}),
                )
            }
        };

        tokio::fs::create_dir_all(parent).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "create parent directory failed: {}",
                err
            )
        })?;

        tokio::fs::write(&target, &content_bytes)
            .await
            .map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "write file failed: {}", err)
            })?;

        self.sync_metadata_after_write_best_effort(&principal.username, &rel_path)
            .await;

        Self::json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "path": Self::to_display_path(&rel_path),
                "size": content_bytes.len(),
            }),
        )
    }

    fn parse_flat_name(input: &str) -> Result<String, RPCErrors> {
        let name = input.trim();
        if name.is_empty() {
            return Err(RPCErrors::ReasonError("new name is required".to_string()));
        }

        let path = Path::new(name);
        let mut count = 0usize;
        for comp in path.components() {
            match comp {
                Component::Normal(_) => {
                    count += 1;
                }
                _ => {
                    return Err(RPCErrors::ReasonError(
                        "new name contains invalid path component".to_string(),
                    ))
                }
            }
        }

        if count != 1 {
            return Err(RPCErrors::ReasonError(
                "new name must be a single path segment".to_string(),
            ));
        }
        Ok(name.to_string())
    }

    fn parse_search_limit(input: Option<String>) -> usize {
        const DEFAULT_LIMIT: usize = 200;
        const MAX_LIMIT: usize = 1000;

        let parsed = input
            .as_deref()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(DEFAULT_LIMIT);
        parsed.clamp(1, MAX_LIMIT)
    }

    fn parse_list_limit(input: Option<String>, default_limit: usize, max_limit: usize) -> usize {
        let parsed = input
            .as_deref()
            .and_then(|value| value.trim().parse::<usize>().ok())
            .unwrap_or(default_limit);
        parsed.clamp(1, max_limit)
    }

    fn parse_u64_param(input: Option<String>) -> Option<u64> {
        input.and_then(|value| value.trim().parse::<u64>().ok())
    }

    fn parse_file_list_filters(
        req: &http::Request<BoxBody<Bytes, ServerError>>,
        default_limit: usize,
        max_limit: usize,
    ) -> Result<FileListFilters, RPCErrors> {
        let kind = Self::get_query_param(req, "kind")
            .unwrap_or_else(|| "all".to_string())
            .trim()
            .to_ascii_lowercase();
        if kind != "all" && kind != "file" && kind != "dir" {
            return Err(RPCErrors::ReasonError(
                "kind must be one of: all, file, dir".to_string(),
            ));
        }

        let exts = Self::get_query_param(req, "ext")
            .unwrap_or_default()
            .split(',')
            .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
            .filter(|ext| !ext.is_empty())
            .collect::<Vec<_>>();

        let sort_by = Self::get_query_param(req, "sort")
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| !value.is_empty());
        if let Some(value) = sort_by.as_deref() {
            if value != "name" && value != "modified" && value != "size" {
                return Err(RPCErrors::ReasonError(
                    "sort must be one of: name, modified, size".to_string(),
                ));
            }
        }

        let order = Self::get_query_param(req, "order")
            .unwrap_or_else(|| "desc".to_string())
            .trim()
            .to_ascii_lowercase();
        if order != "asc" && order != "desc" {
            return Err(RPCErrors::ReasonError(
                "order must be one of: asc, desc".to_string(),
            ));
        }

        let limit = Self::parse_list_limit(
            Self::get_query_param(req, "limit"),
            default_limit,
            max_limit,
        );

        Ok(FileListFilters {
            kind,
            exts,
            modified_from: Self::parse_u64_param(Self::get_query_param(req, "from")),
            modified_to: Self::parse_u64_param(Self::get_query_param(req, "to")),
            size_min: Self::parse_u64_param(Self::get_query_param(req, "size_min")),
            size_max: Self::parse_u64_param(Self::get_query_param(req, "size_max")),
            sort_by,
            order,
            limit,
        })
    }

    fn apply_file_list_filters(
        mut items: Vec<FileEntry>,
        filters: &FileListFilters,
    ) -> Vec<FileEntry> {
        items.retain(|entry| {
            if filters.kind == "file" && entry.is_dir {
                return false;
            }
            if filters.kind == "dir" && !entry.is_dir {
                return false;
            }

            if !filters.exts.is_empty() {
                if entry.is_dir {
                    return false;
                }
                let ext = Self::normalize_file_ext(entry.name.as_str());
                if !filters.exts.iter().any(|value| value == &ext) {
                    return false;
                }
            }

            if let Some(from) = filters.modified_from {
                if entry.modified < from {
                    return false;
                }
            }
            if let Some(to) = filters.modified_to {
                if entry.modified > to {
                    return false;
                }
            }

            if !entry.is_dir {
                if let Some(min_size) = filters.size_min {
                    if entry.size < min_size {
                        return false;
                    }
                }
                if let Some(max_size) = filters.size_max {
                    if entry.size > max_size {
                        return false;
                    }
                }
            }

            true
        });

        if let Some(sort_by) = filters.sort_by.as_deref() {
            items.sort_by(|a, b| {
                let ordering = match sort_by {
                    "modified" => a.modified.cmp(&b.modified),
                    "size" => a.size.cmp(&b.size),
                    _ => a
                        .name
                        .to_ascii_lowercase()
                        .cmp(&b.name.to_ascii_lowercase()),
                };

                if ordering == std::cmp::Ordering::Equal {
                    a.path
                        .to_ascii_lowercase()
                        .cmp(&b.path.to_ascii_lowercase())
                } else {
                    ordering
                }
            });
            if filters.order == "desc" {
                items.reverse();
            }
        }

        if items.len() > filters.limit {
            items.truncate(filters.limit);
        }
        items
    }

    async fn search_resources(
        &self,
        user_root: PathBuf,
        base_rel_path: PathBuf,
        keyword: String,
        kind: String,
        limit: usize,
    ) -> Result<(Vec<FileEntry>, bool), RPCErrors> {
        let keyword = keyword.to_lowercase();
        tokio::task::spawn_blocking(move || -> Result<(Vec<FileEntry>, bool), RPCErrors> {
            fn walk_dir(
                user_root: &Path,
                current_rel_path: &Path,
                keyword: &str,
                kind: &str,
                limit: usize,
                results: &mut Vec<FileEntry>,
                truncated: &mut bool,
            ) -> Result<(), RPCErrors> {
                let current_abs_path = user_root.join(current_rel_path);
                let read_dir = std::fs::read_dir(&current_abs_path).map_err(|err| {
                    RPCErrors::ReasonError(format!(
                        "read directory failed ({}): {}",
                        current_abs_path.display(),
                        err
                    ))
                })?;

                for item in read_dir {
                    let item = match item {
                        Ok(v) => v,
                        Err(err) => {
                            warn!(
                                "skip unreadable directory entry under {}: {}",
                                current_abs_path.display(),
                                err
                            );
                            continue;
                        }
                    };

                    if results.len() >= limit {
                        *truncated = true;
                        break;
                    }

                    let file_type = match item.file_type() {
                        Ok(v) => v,
                        Err(err) => {
                            warn!(
                                "skip unreadable file type under {}: {}",
                                current_abs_path.display(),
                                err
                            );
                            continue;
                        }
                    };

                    if file_type.is_symlink() {
                        continue;
                    }

                    let file_name = item.file_name().to_string_lossy().to_string();
                    let item_rel_path = if current_rel_path.as_os_str().is_empty() {
                        PathBuf::from(&file_name)
                    } else {
                        current_rel_path.join(&file_name)
                    };
                    if BuckyFileServer::is_internal_rel_path(&item_rel_path) {
                        continue;
                    }

                    let item_meta = match item.metadata() {
                        Ok(v) => v,
                        Err(err) => {
                            warn!(
                                "skip unreadable metadata for {}: {}",
                                item.path().display(),
                                err
                            );
                            continue;
                        }
                    };

                    let is_dir = item_meta.is_dir();
                    let is_file = item_meta.is_file();
                    let allowed_by_kind = match kind {
                        "file" => is_file,
                        "dir" => is_dir,
                        _ => is_dir || is_file,
                    };

                    let normalized_name = file_name.to_lowercase();
                    let normalized_path = item_rel_path.to_string_lossy().to_lowercase();
                    let matched =
                        normalized_name.contains(keyword) || normalized_path.contains(keyword);
                    if allowed_by_kind && matched {
                        results.push(FileEntry {
                            name: file_name,
                            path: BuckyFileServer::to_display_path(&item_rel_path),
                            is_dir,
                            size: if is_file { item_meta.len() } else { 0 },
                            modified: BuckyFileServer::unix_mtime(&item_meta),
                        });
                    }

                    if is_dir {
                        walk_dir(
                            user_root,
                            &item_rel_path,
                            keyword,
                            kind,
                            limit,
                            results,
                            truncated,
                        )?;
                    }
                }

                Ok(())
            }

            let search_root_abs = user_root.join(&base_rel_path);
            if !search_root_abs.exists() {
                return Err(RPCErrors::ReasonError("search path not found".to_string()));
            }
            if BuckyFileServer::is_internal_rel_path(&base_rel_path) {
                return Err(RPCErrors::ReasonError(
                    "permission denied: internal path is restricted".to_string(),
                ));
            }

            let mut results = Vec::new();
            let mut truncated = false;
            let search_root_meta = std::fs::metadata(&search_root_abs).map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "read search root metadata failed ({}): {}",
                    search_root_abs.display(),
                    err
                ))
            })?;

            if search_root_meta.is_file() {
                let is_match = base_rel_path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_lowercase().contains(&keyword))
                    .unwrap_or(false);
                let allowed_by_kind = kind == "all" || kind == "file";
                if is_match && allowed_by_kind {
                    results.push(FileEntry {
                        name: base_rel_path
                            .file_name()
                            .map(|v| v.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        path: BuckyFileServer::to_display_path(&base_rel_path),
                        is_dir: false,
                        size: search_root_meta.len(),
                        modified: BuckyFileServer::unix_mtime(&search_root_meta),
                    });
                }
            } else {
                walk_dir(
                    &user_root,
                    &base_rel_path,
                    &keyword,
                    &kind,
                    limit,
                    &mut results,
                    &mut truncated,
                )?;
            }

            results.sort_by(|a, b| {
                if a.is_dir == b.is_dir {
                    a.path.to_lowercase().cmp(&b.path.to_lowercase())
                } else if a.is_dir {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            });

            Ok((results, truncated))
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("search task join error: {}", err)))?
    }

    async fn handle_api_search_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let keyword = Self::get_query_param(&req, "q").unwrap_or_default();
        let keyword = keyword.trim().to_string();
        if keyword.is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "search keyword is required"}),
            );
        }

        let raw_path = Self::get_query_param(&req, "path").unwrap_or_else(|| "/".to_string());
        let rel_path = match Self::parse_relative_path(&raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to search path is required",
            );
        }

        let kind = Self::get_query_param(&req, "kind")
            .unwrap_or_else(|| "all".to_string())
            .trim()
            .to_ascii_lowercase();
        if kind != "all" && kind != "file" && kind != "dir" {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "kind must be one of: all, file, dir"}),
            );
        }

        let limit = Self::parse_search_limit(Self::get_query_param(&req, "limit"));
        let user_root = self.user_root(&principal.username);
        let search_root = user_root.join(&rel_path);
        if !search_root.exists() {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "search path not found"}),
            );
        }

        let (mut items, truncated) = self
            .search_resources(
                user_root,
                rel_path.clone(),
                keyword.clone(),
                kind.clone(),
                limit,
            )
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "search resources failed: {}",
                    err
                )
            })?;
        items.retain(|entry| {
            let parsed = Self::parse_relative_path(&entry.path).ok();
            parsed
                .as_ref()
                .map(|path| Self::can_read_rel_path(&principal, path))
                .unwrap_or(false)
        });

        Self::json_response(
            StatusCode::OK,
            json!(SearchResponse {
                query: keyword,
                path: Self::to_display_path(&rel_path),
                kind,
                limit,
                truncated,
                items,
            }),
        )
    }

    async fn handle_api_resources_recent_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        self.ensure_owner_metadata_seeded(&principal.username)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "ensure metadata seed failed: {}",
                    err
                )
            })?;

        let filters = match Self::parse_file_list_filters(&req, 200, 1000) {
            Ok(value) => value,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let items = self
            .list_recent_entries(&principal.username, &filters)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "query recent entries failed: {}",
                    err
                )
            })?;

        Self::json_response(StatusCode::OK, json!(RecentListResponse { items }))
    }

    async fn handle_api_resources_favorites_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        self.ensure_owner_metadata_seeded(&principal.username)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "ensure metadata seed failed: {}",
                    err
                )
            })?;

        let filters = match Self::parse_file_list_filters(&req, 200, 1000) {
            Ok(value) => value,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let items = self
            .list_favorite_entries(&principal.username, &filters)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "query favorite entries failed: {}",
                    err
                )
            })?;

        Self::json_response(StatusCode::OK, json!(FavoriteListResponse { items }))
    }

    async fn handle_api_resources_favorites_post(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let body = Self::read_body_bytes(req).await?;
        let payload: FavoriteRequest = serde_json::from_slice(&body).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "invalid favorites payload: {}",
                err
            )
        })?;

        let rel_path = match Self::parse_relative_path(payload.path.as_str()) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "path is required"}),
            );
        }
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to target path is required",
            );
        }

        let abs_path = self.user_root(&principal.username).join(&rel_path);
        if !abs_path.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "path not found"}));
        }

        let display_path = Self::to_display_path(&rel_path);
        self.upsert_favorite_record(&principal.username, display_path.as_str())
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "upsert favorite failed: {}",
                    err
                )
            })?;

        Self::json_response(
            StatusCode::OK,
            json!({"ok": true, "path": display_path, "starred": true}),
        )
    }

    async fn handle_api_resources_favorites_delete(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let mut target_path = Self::get_query_param(&req, "path").unwrap_or_default();
        if target_path.trim().is_empty() {
            let body = Self::read_body_bytes(req).await?;
            if !body.is_empty() {
                if let Ok(payload) = serde_json::from_slice::<FavoriteRequest>(&body) {
                    target_path = payload.path;
                }
            }
        }

        let rel_path = match Self::parse_relative_path(target_path.as_str()) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "path is required"}),
            );
        }
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to target path is required",
            );
        }

        let display_path = Self::to_display_path(&rel_path);
        self.delete_favorite_record(&principal.username, display_path.as_str())
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "delete favorite failed: {}",
                    err
                )
            })?;

        Self::json_response(
            StatusCode::OK,
            json!({"ok": true, "path": display_path, "starred": false}),
        )
    }

    async fn handle_api_resources_trash_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let filters = match Self::parse_file_list_filters(&req, 200, 1000) {
            Ok(value) => value,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let items = self
            .list_recycle_bin_items(&principal.username, &filters)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "query recycle bin items failed: {}",
                    err
                )
            })?;

        Self::json_response(StatusCode::OK, json!(RecycleBinListResponse { items }))
    }

    async fn handle_api_resources_trash_restore_post(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let body = Self::read_body_bytes(req).await?;
        let payload: RecycleRestoreRequest = serde_json::from_slice(&body).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "invalid recycle restore payload: {}",
                err
            )
        })?;

        let Some((original_path, _)) = self
            .get_recycle_bin_item(&principal.username, payload.item_id.as_str())
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "query recycle bin item failed: {}",
                    err
                )
            })?
        else {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "recycle bin item not found"}),
            );
        };

        let original_rel_path = match Self::parse_relative_path(original_path.as_str()) {
            Ok(value) => value,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if !Self::can_write_rel_path(&principal, &original_rel_path) {
            return Self::forbidden_response(
                "permission denied: write access to restore target path is required",
            );
        }

        let restored_path = match self
            .restore_recycle_bin_item(
                &principal.username,
                payload.item_id.as_str(),
                payload.override_existing.unwrap_or(false),
            )
            .await
        {
            Ok(path) => path,
            Err(err) => {
                let text = err.to_string();
                if text.contains("not found") {
                    return Self::json_response(StatusCode::NOT_FOUND, json!({"error": text}));
                }
                if text.contains("already exists") {
                    return Self::json_response(StatusCode::CONFLICT, json!({"error": text}));
                }
                return Self::json_response(StatusCode::BAD_REQUEST, json!({"error": text}));
            }
        };

        Self::json_response(StatusCode::OK, json!({"ok": true, "path": restored_path}))
    }

    async fn handle_api_resources_trash_item_delete(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        item_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let Some((original_path, trashed_path)) = self
            .get_recycle_bin_item(&principal.username, item_id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "query recycle bin item failed: {}",
                    err
                )
            })?
        else {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "recycle bin item not found"}),
            );
        };

        let original_rel_path = match Self::parse_relative_path(original_path.as_str()) {
            Ok(value) => value,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if !Self::can_write_rel_path(&principal, &original_rel_path) {
            return Self::forbidden_response(
                "permission denied: write access to recycle bin item is required",
            );
        }

        let trashed_rel_path = match Self::parse_relative_path(trashed_path.as_str()) {
            Ok(value) => value,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if !Self::is_internal_rel_path(&trashed_rel_path) {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "invalid recycle bin item path"}),
            );
        }

        let trashed_abs_path = self.user_root(&principal.username).join(&trashed_rel_path);
        if trashed_abs_path.exists() {
            let metadata = tokio::fs::metadata(&trashed_abs_path)
                .await
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "read recycle item metadata failed: {}",
                        err
                    )
                })?;
            if metadata.is_dir() {
                tokio::fs::remove_dir_all(&trashed_abs_path)
                    .await
                    .map_err(|err| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "delete recycle directory failed: {}",
                            err
                        )
                    })?;
            } else {
                tokio::fs::remove_file(&trashed_abs_path)
                    .await
                    .map_err(|err| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "delete recycle file failed: {}",
                            err
                        )
                    })?;
            }
        }

        self.delete_recycle_bin_item_record(&principal.username, item_id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "delete recycle item record failed: {}",
                    err
                )
            })?;

        Self::json_response(StatusCode::OK, json!({"ok": true, "item_id": item_id}))
    }

    async fn handle_api_resources_reindex_post(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let raw_path = Self::get_query_param(&req, "path").unwrap_or_else(|| "/".to_string());
        let rel_path = match Self::parse_relative_path(&raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }

        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to reindex path is required",
            );
        }

        let stats = self
            .sync_file_metadata_subtree(&principal.username, &rel_path)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "reindex file metadata failed: {}",
                    err
                )
            })?;

        self.sync_thumbnail_tasks_for_scope(&principal.username, &rel_path)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "reindex thumbnail tasks failed: {}",
                    err
                )
            })?;

        if rel_path.as_os_str().is_empty() {
            let _ = self.mark_owner_root_seeded(&principal.username).await;
        }

        Self::json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "path": Self::to_display_path(&rel_path),
                "stats": stats,
            }),
        )
    }

    async fn handle_api_resources_nav_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let raw_path = Self::get_query_param(&req, "path").unwrap_or_default();
        if raw_path.trim().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "path query is required"}),
            );
        }

        let rel_path = match Self::parse_relative_path(&raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "file path is required"}),
            );
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to file path is required",
            );
        }

        self.ensure_owner_metadata_seeded(&principal.username)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "ensure metadata seed failed: {}",
                    err
                )
            })?;

        let (previous, next) = self
            .get_file_nav_neighbors(&principal.username, &rel_path)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "query metadata neighbors failed: {}",
                    err
                )
            })?;

        Self::json_response(
            StatusCode::OK,
            json!(FileNavResponse {
                path: Self::to_display_path(&rel_path),
                previous,
                next,
            }),
        )
    }

    async fn move_path_with_fallback(
        source_abs_path: &Path,
        target_abs_path: &Path,
        is_dir: bool,
    ) -> Result<(), ServerError> {
        if let Err(err) = tokio::fs::rename(source_abs_path, target_abs_path).await {
            warn!(
                "rename failed, fallback to copy+delete. source={}, target={}, err={}",
                source_abs_path.display(),
                target_abs_path.display(),
                err
            );
            Self::copy_path(source_abs_path, target_abs_path).await?;
            if is_dir {
                tokio::fs::remove_dir_all(source_abs_path)
                    .await
                    .map_err(|remove_err| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "remove source directory after move failed: {}",
                            remove_err
                        )
                    })?;
            } else {
                tokio::fs::remove_file(source_abs_path)
                    .await
                    .map_err(|remove_err| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "remove source file after move failed: {}",
                            remove_err
                        )
                    })?;
            }
        }
        Ok(())
    }

    async fn move_resource_to_recycle_bin(
        &self,
        owner: &str,
        rel_path: &Path,
        source_abs_path: &Path,
        source_metadata: &std::fs::Metadata,
    ) -> ServerResult<String> {
        let name = rel_path
            .file_name()
            .map(|value| value.to_string_lossy().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                server_err!(
                    ServerErrorCode::BadRequest,
                    "invalid source path for recycle bin move"
                )
            })?;

        let item_id = Uuid::new_v4().simple().to_string();
        let recycle_rel_path = Self::recycle_bin_item_rel_path(&item_id, &name);
        let recycle_abs_path = self.user_root(owner).join(&recycle_rel_path);
        if let Some(parent) = recycle_abs_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "create recycle bin parent failed: {}",
                    err
                )
            })?;
        }

        Self::move_path_with_fallback(source_abs_path, &recycle_abs_path, source_metadata.is_dir())
            .await?;

        let original_display_path = Self::to_display_path(rel_path);
        let recycle_display_path = Self::to_display_path(&recycle_rel_path);
        let deleted_at = Self::now_unix();
        self.create_recycle_bin_item_record(RecycleBinItemRecordInput {
            owner: owner.to_string(),
            item_id: item_id.clone(),
            original_rel_path: original_display_path,
            trashed_rel_path: recycle_display_path,
            name,
            is_dir: source_metadata.is_dir(),
            size: if source_metadata.is_file() {
                source_metadata.len()
            } else {
                0
            },
            modified: Self::unix_mtime(source_metadata),
            deleted_at,
        })
        .await
        .map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "create recycle bin record failed: {}",
                err
            )
        })?;

        Ok(item_id)
    }

    async fn restore_recycle_bin_item(
        &self,
        owner: &str,
        item_id: &str,
        override_existing: bool,
    ) -> Result<String, RPCErrors> {
        let Some((original_rel_path, trashed_rel_path)) =
            self.get_recycle_bin_item(owner, item_id).await?
        else {
            return Err(RPCErrors::ReasonError(
                "recycle bin item not found".to_string(),
            ));
        };

        let original_rel = Self::parse_relative_path(original_rel_path.as_str())?;
        let trashed_rel = Self::parse_relative_path(trashed_rel_path.as_str())?;
        if !Self::is_internal_rel_path(&trashed_rel) {
            return Err(RPCErrors::ReasonError(
                "invalid recycle bin source path".to_string(),
            ));
        }

        let source_abs_path = self.user_root(owner).join(&trashed_rel);
        if !source_abs_path.exists() {
            let _ = self.delete_recycle_bin_item_record(owner, item_id).await;
            return Err(RPCErrors::ReasonError(
                "recycle bin source is missing".to_string(),
            ));
        }

        let source_metadata = tokio::fs::metadata(&source_abs_path).await.map_err(|err| {
            RPCErrors::ReasonError(format!("read recycle source metadata failed: {}", err))
        })?;
        let target_abs_path = self.user_root(owner).join(&original_rel);

        if target_abs_path.exists() {
            if !override_existing {
                return Err(RPCErrors::ReasonError(
                    "restore target already exists".to_string(),
                ));
            }
            Self::remove_existing_target(&target_abs_path)
                .await
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("remove restore target failed: {}", err))
                })?;
        }

        if let Some(parent) = target_abs_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                RPCErrors::ReasonError(format!("create restore target parent failed: {}", err))
            })?;
        }

        Self::move_path_with_fallback(&source_abs_path, &target_abs_path, source_metadata.is_dir())
            .await
            .map_err(|err| {
                RPCErrors::ReasonError(format!("restore recycle item failed: {}", err))
            })?;

        self.delete_recycle_bin_item_record(owner, item_id).await?;
        self.sync_metadata_after_write_best_effort(owner, &original_rel)
            .await;

        Ok(Self::to_display_path(&original_rel))
    }

    async fn remove_existing_target(target: &Path) -> Result<(), ServerError> {
        let metadata = tokio::fs::metadata(target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read target metadata failed: {}",
                err
            )
        })?;

        if metadata.is_dir() {
            tokio::fs::remove_dir_all(target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "remove target directory failed: {}",
                    err
                )
            })?;
        } else {
            tokio::fs::remove_file(target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "remove target file failed: {}",
                    err
                )
            })?;
        }
        Ok(())
    }

    fn copy_dir_recursive(source: &Path, target: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(target)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            let source_path = entry.path();
            let target_path = target.join(entry.file_name());
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                Self::copy_dir_recursive(&source_path, &target_path)?;
            } else {
                if let Some(parent) = target_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&source_path, &target_path)?;
            }
        }
        Ok(())
    }

    async fn copy_path(source: &Path, target: &Path) -> Result<(), ServerError> {
        let source = source.to_path_buf();
        let target = target.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<(), ServerError> {
            let metadata = std::fs::metadata(&source).map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read source metadata failed: {}",
                    err
                )
            })?;
            if metadata.is_dir() {
                Self::copy_dir_recursive(&source, &target).map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "copy directory failed: {}",
                        err
                    )
                })?;
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent).map_err(|err| {
                        server_err!(
                            ServerErrorCode::InvalidData,
                            "create target parent failed: {}",
                            err
                        )
                    })?;
                }
                std::fs::copy(&source, &target).map_err(|err| {
                    server_err!(ServerErrorCode::InvalidData, "copy file failed: {}", err)
                })?;
            }
            Ok(())
        })
        .await
        .map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "copy task join failed: {}",
                err
            )
        })?
    }

    async fn handle_api_resources_patch(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let source_rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&source_rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if source_rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "source path is required"}),
            );
        }

        let body = Self::read_body_bytes(req).await?;
        let patch_req: PatchResourceRequest = serde_json::from_slice(&body).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "invalid patch payload: {}",
                err
            )
        })?;

        let source_abs_path = self.user_root(&principal.username).join(&source_rel_path);
        if !source_abs_path.exists() {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "source path not found"}),
            );
        }

        let source_metadata = tokio::fs::metadata(&source_abs_path).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read source metadata failed: {}",
                err
            )
        })?;

        let action = patch_req.action.trim().to_ascii_lowercase();
        let override_existing = patch_req.override_existing.unwrap_or(false);

        let target_rel_path = match action.as_str() {
            "rename" => {
                let new_name = match patch_req.new_name {
                    Some(name) => Self::parse_flat_name(&name).map_err(|err| {
                        server_err!(ServerErrorCode::BadRequest, "invalid new name: {}", err)
                    })?,
                    None => {
                        return Self::json_response(
                            StatusCode::BAD_REQUEST,
                            json!({"error": "new_name is required for rename"}),
                        )
                    }
                };

                match source_rel_path.parent() {
                    Some(parent) if !parent.as_os_str().is_empty() => parent.join(new_name),
                    _ => PathBuf::from(new_name),
                }
            }
            "move" | "copy" => {
                let destination = match patch_req.destination {
                    Some(destination) => destination,
                    None => {
                        return Self::json_response(
                            StatusCode::BAD_REQUEST,
                            json!({"error": "destination is required for move/copy"}),
                        )
                    }
                };
                Self::parse_relative_path(&destination).map_err(|err| {
                    server_err!(
                        ServerErrorCode::BadRequest,
                        "invalid destination path: {}",
                        err
                    )
                })?
            }
            _ => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "unsupported patch action"}),
                )
            }
        };

        if target_rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is required"}),
            );
        }
        if Self::is_internal_rel_path(&target_rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }

        if source_rel_path == target_rel_path {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "source and target paths are identical"}),
            );
        }

        let source_read_allowed = Self::can_read_rel_path(&principal, &source_rel_path);
        let source_write_allowed = Self::can_write_rel_path(&principal, &source_rel_path);
        let target_write_allowed = Self::can_write_rel_path(&principal, &target_rel_path);

        match action.as_str() {
            "copy" => {
                if !source_read_allowed || !target_write_allowed {
                    return Self::forbidden_response(
                        "permission denied: copy requires read source and write target permissions",
                    );
                }
            }
            "move" | "rename" => {
                if !source_write_allowed || !target_write_allowed {
                    return Self::forbidden_response(
                        "permission denied: move/rename requires write permissions on source and target",
                    );
                }
            }
            _ => {}
        }

        let target_abs_path = self.user_root(&principal.username).join(&target_rel_path);

        if source_metadata.is_dir() && target_abs_path.starts_with(&source_abs_path) {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path cannot be inside source directory"}),
            );
        }

        if target_abs_path.exists() {
            if !override_existing {
                return Self::json_response(
                    StatusCode::CONFLICT,
                    json!({"error": "target path already exists"}),
                );
            }
            Self::remove_existing_target(&target_abs_path).await?;
        }

        if let Some(parent) = target_abs_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "create target parent failed: {}",
                    err
                )
            })?;
        }

        match action.as_str() {
            "rename" | "move" => {
                if let Err(err) = tokio::fs::rename(&source_abs_path, &target_abs_path).await {
                    if action == "move" {
                        warn!(
                            "rename failed for move, fallback to copy+delete. source={}, target={}, err={}",
                            source_abs_path.display(),
                            target_abs_path.display(),
                            err
                        );
                        Self::copy_path(&source_abs_path, &target_abs_path).await?;
                        if source_metadata.is_dir() {
                            tokio::fs::remove_dir_all(&source_abs_path).await.map_err(
                                |remove_err| {
                                    server_err!(
                                        ServerErrorCode::InvalidData,
                                        "remove source directory after move failed: {}",
                                        remove_err
                                    )
                                },
                            )?;
                        } else {
                            tokio::fs::remove_file(&source_abs_path).await.map_err(
                                |remove_err| {
                                    server_err!(
                                        ServerErrorCode::InvalidData,
                                        "remove source file after move failed: {}",
                                        remove_err
                                    )
                                },
                            )?;
                        }
                    } else {
                        return Err(server_err!(
                            ServerErrorCode::InvalidData,
                            "rename failed: {}",
                            err
                        ));
                    }
                }
            }
            "copy" => {
                Self::copy_path(&source_abs_path, &target_abs_path).await?;
            }
            _ => unreachable!(),
        }

        match action.as_str() {
            "copy" => {
                self.sync_metadata_after_write_best_effort(&principal.username, &target_rel_path)
                    .await;
            }
            "move" | "rename" => {
                self.sync_metadata_after_write_best_effort(&principal.username, &target_rel_path)
                    .await;
                self.sync_metadata_after_write_best_effort(&principal.username, &source_rel_path)
                    .await;
            }
            _ => {}
        }

        Self::json_response(
            StatusCode::OK,
            json!({
                "ok": true,
                "action": action,
                "source": Self::to_display_path(&source_rel_path),
                "target": Self::to_display_path(&target_rel_path),
            }),
        )
    }

    async fn handle_api_resources_delete(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }

        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::FORBIDDEN,
                json!({"error": "root path cannot be deleted"}),
            );
        }
        if !Self::can_write_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: write access to target path is required",
            );
        }

        let target = self.user_root(&principal.username).join(&rel_path);
        if !target.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "path not found"}));
        }

        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read metadata before delete failed: {}",
                err
            )
        })?;

        let recycle_delete = Self::get_query_param(&req, "recycle")
            .map(|value| Self::parse_query_bool(&value))
            .unwrap_or(true);
        let force_permanent = Self::get_query_param(&req, "permanent")
            .map(|value| Self::parse_query_bool(&value))
            .unwrap_or(false);

        if recycle_delete && !force_permanent {
            let item_id = self
                .move_resource_to_recycle_bin(&principal.username, &rel_path, &target, &metadata)
                .await?;

            self.sync_metadata_after_write_best_effort(&principal.username, &rel_path)
                .await;

            return Self::json_response(
                StatusCode::OK,
                json!({
                    "ok": true,
                    "path": Self::to_display_path(&rel_path),
                    "recycled": true,
                    "item_id": item_id,
                }),
            );
        }

        if metadata.is_dir() {
            tokio::fs::remove_dir_all(&target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "delete directory failed: {}",
                    err
                )
            })?;
        } else {
            tokio::fs::remove_file(&target).await.map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "delete file failed: {}", err)
            })?;
        }

        self.sync_metadata_after_write_best_effort(&principal.username, &rel_path)
            .await;

        Self::json_response(
            StatusCode::OK,
            json!({"ok": true, "path": Self::to_display_path(&rel_path)}),
        )
    }

    async fn handle_api_preview_pdf_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "file path is required"}),
            );
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to file path is required",
            );
        }

        let target = self.user_root(&principal.username).join(&rel_path);
        if !target.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "file not found"}));
        }

        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read metadata failed: {}",
                err
            )
        })?;
        if metadata.is_dir() {
            let archive_base_name = Self::archive_name_from_path(&target, "folder");
            let archive_bytes =
                Self::build_directory_archive(target.clone(), archive_base_name.clone()).await?;

            return http::Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "application/zip")
                .header(
                    CONTENT_DISPOSITION,
                    format!("attachment; filename=\"{}.zip\"", archive_base_name),
                )
                .header(CONTENT_LENGTH, archive_bytes.len().to_string())
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(archive_bytes))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build directory raw download response failed: {}",
                        err
                    )
                });
        }

        if !Self::is_pdf_preview_supported(&target) {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "PDF preview conversion supports: .doc, .docx, .odt, .rtf"}),
            );
        }

        let pdf_path = match self.ensure_pdf_preview_cache(&target, &metadata).await {
            Ok(path) => path,
            Err(err) => {
                return Self::json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"error": err.to_string()}),
                )
            }
        };

        let content = tokio::fs::read(&pdf_path).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read converted PDF failed: {}",
                err
            )
        })?;

        let file_stem = rel_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_else(|| "preview".to_string());
        let content_disposition = format!("inline; filename=\"{}.pdf\"", file_stem);

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/pdf")
            .header(CONTENT_DISPOSITION, content_disposition)
            .header(CONTENT_LENGTH, content.len().to_string())
            .header(CACHE_CONTROL, "no-store")
            .body(Self::boxed_body(content))
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build PDF preview response failed: {}",
                    err
                )
            })
    }

    async fn handle_api_raw_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "file path is required"}),
            );
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to file path is required",
            );
        }

        let target = self.user_root(&principal.username).join(&rel_path);
        if !target.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "file not found"}));
        }
        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read metadata failed: {}",
                err
            )
        })?;
        if metadata.is_dir() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is a directory"}),
            );
        }

        let force_download = Self::get_query_param(&req, "download")
            .map(|value| Self::parse_query_bool(&value))
            .unwrap_or(false);
        let content_type = Self::content_type_for_path(&target);
        let filename = rel_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "download.bin".to_string());
        let disposition_type = if force_download {
            "attachment"
        } else {
            "inline"
        };
        let content_disposition = format!("{}; filename=\"{}\"", disposition_type, filename);
        let file_size = metadata.len();

        if let Err(err) = self
            .touch_recent_record(&principal.username, &rel_path)
            .await
        {
            warn!(
                "touch recent for raw file failed owner={}, path={}: {}",
                principal.username,
                Self::to_display_path(&rel_path),
                err
            );
        }

        let range = req
            .headers()
            .get(RANGE)
            .and_then(|value| value.to_str().ok())
            .map(|value| Self::parse_raw_range_header(value, file_size))
            .unwrap_or(RawByteRange::Full);

        if matches!(range, RawByteRange::Unsatisfiable) {
            return http::Response::builder()
                .status(StatusCode::RANGE_NOT_SATISFIABLE)
                .header(ACCEPT_RANGES, "bytes")
                .header(CONTENT_RANGE, format!("bytes */{}", file_size))
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(Vec::new()))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build range not satisfiable response failed: {}",
                        err
                    )
                });
        }

        if let RawByteRange::Partial { start, end } = range {
            let length = end.saturating_sub(start).saturating_add(1);
            if length > usize::MAX as u64 {
                return Err(server_err!(
                    ServerErrorCode::InvalidData,
                    "requested file range is too large: {}",
                    length
                ));
            }

            let mut file = tokio::fs::File::open(&target).await.map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "open file failed: {}", err)
            })?;
            file.seek(SeekFrom::Start(start)).await.map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "seek file failed: {}", err)
            })?;

            let mut content = vec![0u8; length as usize];
            file.read_exact(&mut content).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read file range failed: {}",
                    err
                )
            })?;

            return http::Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header(CONTENT_TYPE, content_type)
                .header(CONTENT_DISPOSITION, content_disposition)
                .header(ACCEPT_RANGES, "bytes")
                .header(CONTENT_LENGTH, length.to_string())
                .header(
                    CONTENT_RANGE,
                    format!("bytes {}-{}/{}", start, end, file_size),
                )
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(content))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build partial content response failed: {}",
                        err
                    )
                });
        }

        let content = tokio::fs::read(&target).await.map_err(|err| {
            server_err!(ServerErrorCode::InvalidData, "read file failed: {}", err)
        })?;

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, content_type)
            .header(CONTENT_DISPOSITION, content_disposition)
            .header(ACCEPT_RANGES, "bytes")
            .header(CONTENT_LENGTH, content.len().to_string())
            .header(CACHE_CONTROL, "no-store")
            .body(Self::boxed_body(content))
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build download response failed: {}",
                    err
                )
            })
    }

    async fn handle_api_thumb_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let principal = match self.auth_principal(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let rel_path = match Self::parse_relative_path(raw_path) {
            Ok(v) => v,
            Err(err) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": err.to_string()}),
                )
            }
        };
        if Self::is_internal_rel_path(&rel_path) {
            return Self::forbidden_response("permission denied: internal path is restricted");
        }
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "file path is required"}),
            );
        }
        if !Self::can_read_rel_path(&principal, &rel_path) {
            return Self::forbidden_response(
                "permission denied: read access to file path is required",
            );
        }

        if let Err(err) = self.ensure_owner_metadata_seeded(&principal.username).await {
            warn!(
                "ensure metadata seeded failed for thumbnail owner={}: {}",
                principal.username, err
            );
        }

        let target = self.user_root(&principal.username).join(&rel_path);
        if !target.exists() {
            return http::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(Vec::new()))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build thumbnail not found response failed: {}",
                        err
                    )
                });
        }

        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read thumbnail source metadata failed: {}",
                err
            )
        })?;
        if !metadata.is_file() {
            return http::Response::builder()
                .status(StatusCode::BAD_REQUEST)
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(Vec::new()))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build thumbnail bad request response failed: {}",
                        err
                    )
                });
        }

        let ext = target
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if !Self::is_thumbnail_supported_ext(ext.as_str()) {
            return http::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(Vec::new()))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build unsupported thumbnail response failed: {}",
                        err
                    )
                });
        }

        let thumb_size = Self::parse_thumbnail_size(Self::get_query_param(&req, "size"));
        let variant = Self::thumbnail_variant_for_size(thumb_size);
        if let Err(err) = self
            .sync_thumbnail_tasks_for_scope(&principal.username, &rel_path)
            .await
        {
            warn!(
                "sync thumbnail task before read failed owner={}, path={}: {}",
                principal.username,
                Self::to_display_path(&rel_path),
                err
            );
        }

        let source_size = metadata.len().min(i64::MAX as u64) as i64;
        let source_modified = Self::unix_mtime(&metadata).min(i64::MAX as u64) as i64;
        let rel_path_display = Self::to_display_path(&rel_path);

        let mut ready_thumb_rel_path = self
            .get_ready_thumbnail_rel_path(
                &principal.username,
                rel_path_display.as_str(),
                variant.as_str(),
                source_size,
                source_modified,
            )
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "query ready thumbnail failed: {}",
                    err
                )
            })?;

        if ready_thumb_rel_path.is_none() {
            if let Err(err) = self
                .process_thumbnail_task(
                    &principal.username,
                    rel_path_display.as_str(),
                    variant.as_str(),
                    source_size,
                    source_modified,
                    0,
                )
                .await
            {
                warn!(
                    "on-demand thumbnail generation failed owner={}, path={}: {}",
                    principal.username, rel_path_display, err
                );
            }

            ready_thumb_rel_path = self
                .get_ready_thumbnail_rel_path(
                    &principal.username,
                    rel_path_display.as_str(),
                    variant.as_str(),
                    source_size,
                    source_modified,
                )
                .await
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "query ready thumbnail failed: {}",
                        err
                    )
                })?;
        }

        let Some(thumb_rel_path) = ready_thumb_rel_path else {
            return http::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(CACHE_CONTROL, "no-store")
                .body(Self::boxed_body(Vec::new()))
                .map_err(|err| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "build thumbnail pending response failed: {}",
                        err
                    )
                });
        };

        let thumb_abs_path = self.thumbnail_cache_dir().join(thumb_rel_path);
        let bytes = tokio::fs::read(&thumb_abs_path).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read thumbnail cache file failed: {}",
                err
            )
        })?;

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "image/jpeg")
            .header(CONTENT_LENGTH, bytes.len().to_string())
            .header(CACHE_CONTROL, "no-store")
            .body(Self::boxed_body(bytes))
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build thumbnail response failed: {}",
                    err
                )
            })
    }
}

#[async_trait]
impl RPCHandler for BuckyFileServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        match req.method.as_str() {
            "system.ping" => Ok(RPCResponse::new(
                RPCResult::Success(json!({"service": BUCKY_FILE_SERVICE_NAME, "ok": true})),
                req.seq,
            )),
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}

#[async_trait]
impl HttpServer for BuckyFileServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let method = req.method().clone();
        let path = req.uri().path().to_string();

        if method == Method::POST && path.starts_with("/kapi/bucky-file") {
            return serve_http_by_rpc_handler(req, info, self).await;
        }

        if method == Method::GET && path == "/api/health" {
            return Self::json_response(StatusCode::OK, json!({"ok": true}));
        }

        if method == Method::GET && path == "/api/search" {
            return self.handle_api_search_get(req).await;
        }

        if path == "/api/resources/reindex" {
            return match method {
                Method::POST => self.handle_api_resources_reindex_post(req).await,
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if method == Method::GET && path == "/api/resources/nav" {
            return self.handle_api_resources_nav_get(req).await;
        }

        if method == Method::GET && path == "/api/recent" {
            return self.handle_api_resources_recent_get(req).await;
        }

        if path == "/api/favorites" {
            return match method {
                Method::GET => self.handle_api_resources_favorites_get(req).await,
                Method::POST => self.handle_api_resources_favorites_post(req).await,
                Method::DELETE => self.handle_api_resources_favorites_delete(req).await,
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if method == Method::GET && path == "/api/recycle-bin" {
            return self.handle_api_resources_trash_get(req).await;
        }

        if path == "/api/recycle-bin/restore" {
            return match method {
                Method::POST => self.handle_api_resources_trash_restore_post(req).await,
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if path.starts_with("/api/recycle-bin/item/") {
            let item_id = path.strip_prefix("/api/recycle-bin/item/").unwrap_or("");
            return match method {
                Method::DELETE => {
                    self.handle_api_resources_trash_item_delete(req, item_id)
                        .await
                }
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if path == "/api/upload/session" {
            return match method {
                Method::POST => self.handle_api_upload_session_create(req).await,
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if path.starts_with("/api/upload/session/") {
            let suffix = path.strip_prefix("/api/upload/session/").unwrap_or("");
            if let Some(session_id) = suffix.strip_suffix("/complete") {
                return match method {
                    Method::POST => {
                        self.handle_api_upload_session_complete(req, session_id)
                            .await
                    }
                    _ => Self::json_response(
                        StatusCode::METHOD_NOT_ALLOWED,
                        json!({"error": "method not allowed"}),
                    ),
                };
            }

            return match method {
                Method::GET => self.handle_api_upload_session_get(req, suffix).await,
                Method::PUT => self.handle_api_upload_session_put(req, suffix).await,
                Method::DELETE => self.handle_api_upload_session_delete(req, suffix).await,
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if path == "/api/share" {
            return match method {
                Method::GET => self.handle_api_share_list(req).await,
                Method::POST => self.handle_api_share_create(req).await,
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if path.starts_with("/api/share/") {
            let share_id = path.strip_prefix("/api/share/").unwrap_or("");
            return match method {
                Method::DELETE => self.handle_api_share_delete(req, share_id).await,
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if method == Method::GET && path.starts_with("/api/public/share/") {
            let share_id = path.strip_prefix("/api/public/share/").unwrap_or("");
            return self.handle_api_public_share_get(req, share_id).await;
        }

        if method == Method::GET && path.starts_with("/api/public/dl/") {
            let share_id = path.strip_prefix("/api/public/dl/").unwrap_or("");
            return self.handle_api_public_download_get(req, share_id).await;
        }

        if method == Method::GET && path.starts_with("/api/preview/pdf") {
            let raw_file_path = path.strip_prefix("/api/preview/pdf").unwrap_or("");
            return self.handle_api_preview_pdf_get(req, raw_file_path).await;
        }

        if method == Method::GET && path.starts_with("/api/thumb") {
            let raw_file_path = path.strip_prefix("/api/thumb").unwrap_or("");
            return self.handle_api_thumb_get(req, raw_file_path).await;
        }

        if path.starts_with("/api/resources") {
            let raw_resource_path = path.strip_prefix("/api/resources").unwrap_or("");
            return match method {
                Method::GET => self.handle_api_resources_get(req, raw_resource_path).await,
                Method::POST => {
                    let create_dir = path.ends_with('/');
                    self.handle_api_resources_post(req, raw_resource_path, create_dir)
                        .await
                }
                Method::PUT => self.handle_api_resources_put(req, raw_resource_path).await,
                Method::PATCH => {
                    self.handle_api_resources_patch(req, raw_resource_path)
                        .await
                }
                Method::DELETE => {
                    self.handle_api_resources_delete(req, raw_resource_path)
                        .await
                }
                _ => Self::json_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    json!({"error": "method not allowed"}),
                ),
            };
        }

        if method == Method::GET && path.starts_with("/api/raw") {
            let raw_file_path = path.strip_prefix("/api/raw").unwrap_or("");
            return self.handle_api_raw_get(req, raw_file_path).await;
        }

        Self::json_response(StatusCode::NOT_FOUND, json!({"error": "not found"}))
    }

    fn id(&self) -> String {
        BUCKY_FILE_SERVICE_NAME.to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}
