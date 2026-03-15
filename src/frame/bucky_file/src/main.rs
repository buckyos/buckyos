use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType,
    ControlPanelClient,
};
use buckyos_kit::{get_buckyos_root_dir, init_logging};
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use http::{Method, StatusCode, Version};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use kRPC::{RPCHandler, RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use log::{error, info, warn};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use server_runner::Runner;
use server_runner::DirHandlerOptions;
use std::io::SeekFrom;
use std::net::IpAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use uuid::Uuid;

const BUCKY_FILE_SERVICE_NAME: &str = "bucky-file";
const BUCKY_FILE_SERVICE_PORT: u16 = 4070;
const TOKEN_EXPIRE_SECONDS: u64 = 2 * 60 * 60;

#[derive(Debug, Clone)]
struct BuckyFileServer {
    jwt_secret: String,
    standalone_mode: bool,
    data_folder: PathBuf,
    db_path: PathBuf,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Claims {
    sub: String,
    iat: u64,
    exp: u64,
}

#[derive(Debug, Serialize)]
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

#[derive(Debug, Deserialize)]
struct PutFileRequest {
    content: String,
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

impl BuckyFileServer {
    fn new(data_folder: PathBuf, standalone_mode: bool) -> Self {
        let jwt_secret = std::env::var("BUCKY_FILE_JWT_SECRET")
            .unwrap_or_else(|_| "bucky-file-dev-secret-change-me".to_string());
        let db_path = data_folder.join("bucky_file.db");
        Self {
            jwt_secret,
            standalone_mode,
            data_folder,
            db_path,
        }
    }

    fn token_validation() -> Validation {
        Validation::new(Algorithm::HS256)
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

    async fn init_share_db(&self) -> Result<(), RPCErrors> {
        let db_path = self.db_path();
        let upload_tmp_dir = self.upload_tmp_dir();
        tokio::task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open share database failed: {}", err)))?;
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
                ",
            )
            .map_err(|err| RPCErrors::ReasonError(format!("init share database failed: {}", err)))?;

            std::fs::create_dir_all(&upload_tmp_dir).map_err(|err| {
                RPCErrors::ReasonError(format!("prepare upload tmp dir failed: {}", err))
            })?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("init share database join error: {}", err)))?
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
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open share database failed: {}", err)))?;
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

    async fn get_share_record(&self, share_id: &str) -> Result<Option<(ShareItem, Option<String>)>, RPCErrors> {
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
            let conn = Connection::open(db_path)
                .map_err(|err| RPCErrors::ReasonError(format!("open upload database failed: {}", err)))?;
            let deleted = conn
                .execute(
                    "DELETE FROM upload_sessions WHERE id = ?1 AND owner = ?2",
                    params![session_id, owner],
                )
                .map_err(|err| RPCErrors::ReasonError(format!("delete upload session failed: {}", err)))?;
            Ok(deleted > 0)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("delete upload session join error: {}", err)))?
    }

    fn boxed_body(bytes: Vec<u8>) -> BoxBody<Bytes, ServerError> {
        BoxBody::new(
            Full::new(Bytes::from(bytes))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        )
    }

    fn text_response(
        status: StatusCode,
        text: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        http::Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Self::boxed_body(text.as_bytes().to_vec()))
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "build text response failed: {}",
                    err
                )
            })
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

    fn issue_token(&self, username: &str) -> Result<String, RPCErrors> {
        let now = Self::now_unix();
        let claims = Claims {
            sub: username.to_string(),
            iat: now,
            exp: now + TOKEN_EXPIRE_SECONDS,
        };
        encode(
            &Header::new(Algorithm::HS256),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )
        .map_err(|err| RPCErrors::ReasonError(format!("issue token failed: {}", err)))
    }

    fn decode_token(&self, token: &str) -> Result<Claims, RPCErrors> {
        decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Self::token_validation(),
        )
        .map(|v| v.claims)
        .map_err(|err| RPCErrors::ReasonError(format!("invalid token: {}", err)))
    }

    async fn get_user_settings(&self, username: &str) -> Result<buckyos_api::UserSettings, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let system_config_client = runtime.get_system_config_client().await?;
        let control_panel_client = ControlPanelClient::new(system_config_client);
        control_panel_client
            .get_user_settings_by_username(username)
            .await
    }

    fn calc_password_hash(username: &str, password: &str) -> String {
        let source = format!("{}{}.buckyos", password, username);
        let digest = Sha256::digest(source.as_bytes());
        STANDARD.encode(digest)
    }

    async fn validate_user_credentials(&self, username: &str, password: &str) -> Result<(), RPCErrors> {
        if self.standalone_mode {
            if username.trim().is_empty() || password.trim().is_empty() {
                return Err(RPCErrors::InvalidPassword);
            }
            return Ok(());
        }

        let user_settings = self.get_user_settings(username).await?;
        if !matches!(user_settings.state, buckyos_api::UserState::Active) {
            return Err(RPCErrors::NoPermission("user is not active".to_string()));
        }

        let password_from_plain = Self::calc_password_hash(username, password);
        let stored_password = user_settings.password.trim();
        let provided_password = password.trim();

        let matched = password_from_plain == stored_password || provided_password == stored_password;
        if !matched {
            return Err(RPCErrors::InvalidPassword);
        }

        Ok(())
    }

    async fn verify_verify_hub_token(&self, token: &str) -> Result<String, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let verify_hub_client = runtime.get_verify_hub_client().await?;
        let verified = verify_hub_client.verify_token(token, None).await?;
        if !verified {
            return Err(RPCErrors::InvalidToken("verify-hub token is invalid".to_string()));
        }

        let parsed = RPCSessionToken::from_string(token)?;
        let username = parsed
            .sub
            .ok_or_else(|| RPCErrors::InvalidToken("verify-hub token missing subject".to_string()))?;

        let user_settings = self.get_user_settings(&username).await?;
        if !matches!(user_settings.state, buckyos_api::UserState::Active) {
            return Err(RPCErrors::NoPermission("user is not active".to_string()));
        }

        Ok(username)
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
                    if let Some(token) = segment.strip_prefix("auth=") {
                        if !token.trim().is_empty() {
                            return Some(token.trim().to_string());
                        }
                    }
                }
            }
        }

        None
    }

    async fn auth_user(
        &self,
        req: &http::Request<BoxBody<Bytes, ServerError>>,
    ) -> Result<String, http::Response<BoxBody<Bytes, ServerError>>> {
        let token = match Self::extract_auth_token(req) {
            Some(v) => v,
            None => {
                return Err(
                    Self::json_response(
                        StatusCode::UNAUTHORIZED,
                        json!({"error": "missing authentication token"}),
                    )
                    .unwrap_or_else(|_| {
                        http::Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .body(Self::boxed_body(Vec::new()))
                            .unwrap_or_else(|_| unreachable!())
                    }),
                )
            }
        };

        if let Ok(claims) = self.decode_token(&token) {
            if self.standalone_mode {
                return Ok(claims.sub);
            }

            if let Ok(user_settings) = self.get_user_settings(&claims.sub).await {
                if matches!(user_settings.state, buckyos_api::UserState::Active) {
                    return Ok(claims.sub);
                }
            }
        }

        if !self.standalone_mode {
            if let Ok(username) = self.verify_verify_hub_token(&token).await {
                return Ok(username);
            }
        }

        Err(
            Self::json_response(
                StatusCode::UNAUTHORIZED,
                json!({"error": "invalid authentication token"}),
            )
            .unwrap_or_else(|_| {
                http::Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Self::boxed_body(Vec::new()))
                    .unwrap_or_else(|_| unreachable!())
            }),
        )
    }

    fn user_root(&self, username: &str) -> PathBuf {
        if let Ok(root) = std::env::var("BUCKY_FILE_ROOT") {
            return PathBuf::from(root).join(username);
        }
        get_buckyos_root_dir().join("data").join("home").join(username)
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

    async fn handle_api_login(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let body = Self::read_body_bytes(req).await?;
        let login_req: LoginRequest = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => {
                return Self::json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"error": "invalid login payload"}),
                )
            }
        };

        let username = login_req.username.trim();
        let password = login_req.password.trim();
        if username.is_empty() || password.is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "username and password are required"}),
            );
        }

        if let Err(err) = self.validate_user_credentials(username, password).await {
            warn!("bucky-file login failed for {}: {}", username, err);
            return Self::json_response(
                StatusCode::UNAUTHORIZED,
                json!({"error": "invalid username or password"}),
            );
        }

        tokio::fs::create_dir_all(self.user_root(username))
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "prepare user root directory failed: {}",
                    err
                )
            })?;

        let token = self.issue_token(username).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "issue login token failed: {}",
                err
            )
        })?;

        Self::text_response(StatusCode::OK, &token)
    }

    async fn handle_api_renew(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let username = match self.auth_user(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let token = self.issue_token(&username).map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "issue renew token failed: {}",
                err
            )
        })?;
        Self::text_response(StatusCode::OK, &token)
    }

    fn get_query_param(req: &http::Request<BoxBody<Bytes, ServerError>>, key: &str) -> Option<String> {
        req.uri().query().and_then(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.to_string())
        })
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
            return Err(RPCErrors::ReasonError("upload session id is required".to_string()));
        }
        if !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
        {
            return Err(RPCErrors::ReasonError("invalid upload session id".to_string()));
        }
        Ok(value.to_string())
    }

    async fn handle_api_upload_session_create(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let username = match self.auth_user(&req).await {
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

        let chunk_size = Self::parse_upload_chunk_size(payload.chunk_size);
        let override_existing = payload.override_existing.unwrap_or(false);
        let target = self.user_root(&username).join(&rel_path);
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
                &username,
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
        let username = match self.auth_user(&req).await {
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
            .get_upload_session_record(&username, &session_id)
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
                    .update_upload_session_progress(&username, &session.id, actual_uploaded)
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
        let username = match self.auth_user(&req).await {
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
            .get_upload_session_record(&username, &session_id)
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
            server_err!(ServerErrorCode::InvalidData, "flush upload chunk failed: {}", err)
        })?;

        let next_uploaded_size = session.uploaded_size + chunk_size;
        self.update_upload_session_progress(&username, &session.id, next_uploaded_size)
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
        let username = match self.auth_user(&req).await {
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
            .get_upload_session_record(&username, &session_id)
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
                .update_upload_session_progress(&username, &session.id, actual_uploaded)
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

        let target = self.user_root(&username).join(&rel_path);
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
            tokio::fs::copy(&tmp_path, &target).await.map_err(|copy_err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "copy upload temp file failed: {}",
                    copy_err
                )
            })?;
            let _ = tokio::fs::remove_file(&tmp_path).await;
        }

        let _ = self
            .delete_upload_session_record(&username, &session.id)
            .await
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "delete upload session record failed: {}",
                    err
                )
            })?;

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
        let username = match self.auth_user(&req).await {
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
            .delete_upload_session_record(&username, &session_id)
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
        let username = match self.auth_user(&req).await {
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
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "share path is required"}),
            );
        }

        let target = self.user_root(&username).join(&rel_path);
        if !target.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "path not found"}));
        }

        let now = Self::now_unix();
        let expires_at = payload.expires_in_seconds.map(|seconds| now.saturating_add(seconds));
        let password_hash = Self::hash_optional_password(payload.password.as_deref());
        let share_item = self
            .create_share_record(
                &username,
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
        let username = match self.auth_user(&req).await {
            Ok(v) => v,
            Err(resp) => return Ok(resp),
        };

        let items = self.list_share_records(&username).await.map_err(|err| {
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
        let username = match self.auth_user(&req).await {
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
            .delete_share_record(&username, share_id)
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
        let Some((share_item, password_hash)) = self.get_share_record(share_id).await.map_err(|err| {
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
        })? else {
            return Err(
                Self::json_response(StatusCode::NOT_FOUND, json!({"error": "share not found"}))
                    .unwrap_or_else(|_| {
                        http::Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Self::boxed_body(Vec::new()))
                            .unwrap_or_else(|_| unreachable!())
                    }),
            );
        };

        if let Some(expires_at) = share_item.expires_at {
            if expires_at <= Self::now_unix() {
                return Err(
                    Self::json_response(StatusCode::GONE, json!({"error": "share expired"}))
                        .unwrap_or_else(|_| {
                            http::Response::builder()
                                .status(StatusCode::GONE)
                                .body(Self::boxed_body(Vec::new()))
                                .unwrap_or_else(|_| unreachable!())
                        }),
                );
            }
        }

        if let Some(stored_hash) = password_hash {
            let provided_hash = Self::hash_optional_password(Self::get_share_password(req).as_deref());
            if provided_hash.as_deref() != Some(stored_hash.as_str()) {
                return Err(
                    Self::json_response(
                        StatusCode::UNAUTHORIZED,
                        json!({"error": "share password required or invalid"}),
                    )
                    .unwrap_or_else(|_| {
                        http::Response::builder()
                            .status(StatusCode::UNAUTHORIZED)
                            .body(Self::boxed_body(Vec::new()))
                            .unwrap_or_else(|_| unreachable!())
                    }),
                );
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
            return Err(
                Self::json_response(StatusCode::NOT_FOUND, json!({"error": "shared path not found"}))
                    .unwrap_or_else(|_| {
                        http::Response::builder()
                            .status(StatusCode::NOT_FOUND)
                            .body(Self::boxed_body(Vec::new()))
                            .unwrap_or_else(|_| unreachable!())
                    }),
            );
        }

        Ok((share_item, share_root, target))
    }

    async fn handle_api_public_share_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        share_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let (share_item, share_root, target) = match self.resolve_public_share(&req, share_id).await {
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
                server_err!(ServerErrorCode::InvalidData, "read shared directory failed: {}", err)
            })?;

            while let Some(entry) = reader.next_entry().await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "read shared dir entry failed: {}",
                    err
                )
            })? {
                let entry_meta = entry.metadata().await.map_err(|err| {
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
                    size: if entry_meta.is_file() { entry_meta.len() } else { 0 },
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

        let file_data = tokio::fs::read(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read shared file failed: {}",
                err
            )
        })?;
        let content = String::from_utf8(file_data).ok();
        Self::json_response(
            StatusCode::OK,
            json!({
                "share": share_item,
                "is_dir": false,
                "path": display_path,
                "parent_path": parent_display_path,
                "size": metadata.len(),
                "modified": Self::unix_mtime(&metadata),
                "content": content,
            }),
        )
    }

    async fn handle_api_public_download_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        share_id: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let (_share_item, _share_root, target) = match self.resolve_public_share(&req, share_id).await {
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
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "directory download is not supported yet"}),
            );
        }

        let content = tokio::fs::read(&target).await.map_err(|err| {
            server_err!(
                ServerErrorCode::InvalidData,
                "read shared file failed: {}",
                err
            )
        })?;
        let filename = target
            .file_name()
            .map(|v| v.to_string_lossy().to_string())
            .unwrap_or_else(|| "download.bin".to_string());

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/octet-stream")
            .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
            .header(CACHE_CONTROL, "no-store")
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
        let username = match self.auth_user(&req).await {
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

        let target = self.user_root(&username).join(&rel_path);
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
            let mut items: Vec<FileEntry> = Vec::new();
            let mut reader = tokio::fs::read_dir(&target).await.map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "read dir failed: {}", err)
            })?;

            while let Some(entry) = reader.next_entry().await.map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "read dir entry failed: {}", err)
            })? {
                let file_name = entry.file_name().to_string_lossy().to_string();
                let entry_meta = entry.metadata().await.map_err(|err| {
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
                items.push(FileEntry {
                    name: file_name,
                    path: Self::to_display_path(&item_rel_path),
                    is_dir: entry_meta.is_dir(),
                    size: if entry_meta.is_file() { entry_meta.len() } else { 0 },
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

        let file_data = tokio::fs::read(&target).await.map_err(|err| {
            server_err!(ServerErrorCode::InvalidData, "read file failed: {}", err)
        })?;
        let content = String::from_utf8(file_data).ok();

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
        let username = match self.auth_user(&req).await {
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
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is required"}),
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

        let target = self.user_root(&username).join(&rel_path);

        if create_dir {
            tokio::fs::create_dir_all(&target).await.map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "create directory failed: {}",
                    err
                )
            })?;
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
        let username = match self.auth_user(&req).await {
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
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is required"}),
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

        let target = self.user_root(&username).join(&rel_path);
        if target.exists() {
            let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
                server_err!(ServerErrorCode::InvalidData, "read metadata failed: {}", err)
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

        tokio::fs::write(&target, &content_bytes).await.map_err(|err| {
            server_err!(ServerErrorCode::InvalidData, "write file failed: {}", err)
        })?;

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
                    let matched = normalized_name.contains(keyword) || normalized_path.contains(keyword);
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
                            .unwrap_or_else(|| "".to_string()),
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
        let username = match self.auth_user(&req).await {
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
        let user_root = self.user_root(&username);
        let search_root = user_root.join(&rel_path);
        if !search_root.exists() {
            return Self::json_response(
                StatusCode::NOT_FOUND,
                json!({"error": "search path not found"}),
            );
        }

        let (items, truncated) = self
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
                    server_err!(ServerErrorCode::InvalidData, "copy directory failed: {}", err)
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
        let username = match self.auth_user(&req).await {
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

        let source_abs_path = self.user_root(&username).join(&source_rel_path);
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

        if source_rel_path == target_rel_path {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "source and target paths are identical"}),
            );
        }

        let target_abs_path = self.user_root(&username).join(&target_rel_path);

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
                            tokio::fs::remove_dir_all(&source_abs_path).await.map_err(|remove_err| {
                                server_err!(
                                    ServerErrorCode::InvalidData,
                                    "remove source directory after move failed: {}",
                                    remove_err
                                )
                            })?;
                        } else {
                            tokio::fs::remove_file(&source_abs_path).await.map_err(|remove_err| {
                                server_err!(
                                    ServerErrorCode::InvalidData,
                                    "remove source file after move failed: {}",
                                    remove_err
                                )
                            })?;
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
        let username = match self.auth_user(&req).await {
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

        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::FORBIDDEN,
                json!({"error": "root path cannot be deleted"}),
            );
        }

        let target = self.user_root(&username).join(&rel_path);
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

        Self::json_response(
            StatusCode::OK,
            json!({"ok": true, "path": Self::to_display_path(&rel_path)}),
        )
    }

    async fn handle_api_raw_get(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        raw_path: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let username = match self.auth_user(&req).await {
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
        if rel_path.as_os_str().is_empty() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "file path is required"}),
            );
        }

        let target = self.user_root(&username).join(&rel_path);
        if !target.exists() {
            return Self::json_response(StatusCode::NOT_FOUND, json!({"error": "file not found"}));
        }
        let metadata = tokio::fs::metadata(&target).await.map_err(|err| {
            server_err!(ServerErrorCode::InvalidData, "read metadata failed: {}", err)
        })?;
        if metadata.is_dir() {
            return Self::json_response(
                StatusCode::BAD_REQUEST,
                json!({"error": "target path is a directory"}),
            );
        }

        let content = tokio::fs::read(&target).await.map_err(|err| {
            server_err!(ServerErrorCode::InvalidData, "read file failed: {}", err)
        })?;
        let filename = rel_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "download.bin".to_string());

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/octet-stream")
            .header(CONTENT_DISPOSITION, format!("attachment; filename=\"{}\"", filename))
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

        if method == Method::POST && path == "/api/login" {
            return self.handle_api_login(req).await;
        }

        if method == Method::POST && path == "/api/renew" {
            return self.handle_api_renew(req).await;
        }

        if method == Method::GET && path == "/api/search" {
            return self.handle_api_search_get(req).await;
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
                    Method::POST => self.handle_api_upload_session_complete(req, session_id).await,
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
                Method::PATCH => self.handle_api_resources_patch(req, raw_resource_path).await,
                Method::DELETE => self.handle_api_resources_delete(req, raw_resource_path).await,
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

async fn start_bucky_file_service() -> anyhow::Result<()> {
    let standalone_requested = std::env::var("BUCKY_FILE_STANDALONE")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let require_runtime = std::env::var("BUCKY_FILE_REQUIRE_RUNTIME")
        .or_else(|_| std::env::var("BUCKY_FILE_FORCE_RUNTIME"))
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let mut standalone_mode = standalone_requested;
    if standalone_requested {
        warn!("bucky-file standalone mode explicitly requested by env");
    } else {
        let runtime_result = async {
            let mut runtime =
                init_buckyos_api_runtime(BUCKY_FILE_SERVICE_NAME, None, BuckyOSRuntimeType::KernelService)
                    .await?;
            runtime.login().await?;
            runtime.set_main_service_port(BUCKY_FILE_SERVICE_PORT).await;
            set_buckyos_api_runtime(runtime);
            Ok::<(), RPCErrors>(())
        }
        .await;

        if let Err(err) = runtime_result {
            if require_runtime {
                return Err(anyhow::anyhow!(
                    "bucky-file runtime login required but failed: {:?}",
                    err
                ));
            }
            warn!(
                "bucky-file runtime login failed: {:?}; falling back to standalone mode",
                err
            );
            standalone_mode = true;
        }
    }

    let data_folder = if standalone_mode {
        std::env::temp_dir().join("bucky-file-data")
    } else {
        get_buckyos_root_dir().join("data").join("var").join(BUCKY_FILE_SERVICE_NAME)
    };
    if !data_folder.exists() {
        tokio::fs::create_dir_all(&data_folder).await?;
    }

    let server = Arc::new(BuckyFileServer::new(data_folder.clone(), standalone_mode));
    if standalone_mode {
        warn!("bucky-file standalone mode enabled; runtime login is skipped");
    } else {
        info!("bucky-file runtime login succeeded");
    }

    server
        .init_share_db()
        .await
        .map_err(|err| anyhow::anyhow!("init share db failed: {}", err))?;

    let runner = Runner::new(BUCKY_FILE_SERVICE_PORT);
    let _ = runner.add_http_server("/kapi/bucky-file".to_string(), server.clone());
    let _ = runner.add_http_server("/api".to_string(), server.clone());

    let web_dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join("web")));
    if let Some(web_dir) = web_dir {
        let _ = runner
            .add_dir_handler_with_options(
                "/".to_string(),
                web_dir,
                DirHandlerOptions {
                    fallback_file: Some("index.html".to_string()),
                    ..Default::default()
                },
            )
            .await;
    } else {
        warn!("bucky-file web directory not found, static UI disabled");
    }

    let _ = runner.start();
    info!(
        "bucky-file service started at port {}",
        BUCKY_FILE_SERVICE_PORT
    );
    Ok(())
}

async fn service_main() {
    init_logging("bucky-file", true);
    if let Err(err) = start_bucky_file_service().await {
        error!("bucky-file service start failed: {:?}", err);
        return;
    }

    let _ = tokio::signal::ctrl_c().await;
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}
