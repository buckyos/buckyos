/*
功能：
username + ui_session_id + state_key => ui_state_value (可以读写)
也可以进一步通过json_path来读写ui_state中的部分数据


保存在system_config中
/config/users/$userid/desktop/$ui_session_id/$state_key => $state_value

实际使用的$state_key 是：
- appearance
- window_layout
- app_items_layout
- widgets_layout

桌面上有哪些项目不在这里：它们是用户 home 下 Desktop 文件夹里的文件（见 desktop_folder.rs，
items.* 动作），各 session 只记录项目的位置。

核心接口
1） uisession的管理，包括创建uisession, 删除uisession, 获取uisession列表，重命名uisession
2）针对state_key的读写操作,通过json_path来支持部分写

客户端使用逻辑
0）判断localStorage中是否存在配置
1）不存在尝试选择一个uisession同步配置
2）存在则判断远端的配置是否被本地配置更新，远端更新则同步更新本地配置
3）定期把localStorage中的配置和远端的配置进行对比，判断和上一次同步点，是否有变化。并定期更新（2次更新之间的间隔不会小于10秒）

*/

use super::desktop_folder;
use super::desktop_wallpaper::WALLPAPER_ROUTE;
use super::ControlPanelServer;
use buckyos_api::{get_buckyos_api_runtime, SystemConfigError};
use buckyos_http_server::*;
use bytes::Bytes;
use http::{Method, StatusCode};
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use uuid::Uuid;

/// Config key prefix for desktop UI sessions.
/// Full path: /config/users/{userid}/desktop/{session_id}/{state_key}
fn desktop_session_prefix(userid: &str) -> String {
    format!("users/{}/desktop", userid)
}

fn desktop_session_key(userid: &str, session_id: &str) -> String {
    format!("users/{}/desktop/{}", userid, session_id)
}

fn desktop_state_key(userid: &str, session_id: &str, state_key: &str) -> String {
    format!("users/{}/desktop/{}/{}", userid, session_id, state_key)
}

/// Session metadata is stored under: users/{userid}/desktop/{session_id}/_meta
fn desktop_session_meta_key(userid: &str, session_id: &str) -> String {
    format!("users/{}/desktop/{}/_meta", userid, session_id)
}

/// Session ids and state keys are single config path segments: `[A-Za-z0-9_-]`, at most 64, and a state key
/// never starts with `_` (that is where `_meta` lives).
fn is_path_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn is_state_key(value: &str) -> bool {
    is_path_segment(value) && !value.starts_with('_')
}

/// Read a value at a json_path from a JSON value.
/// json_path uses dot-separated keys, e.g. "theme.colors.primary"
fn json_path_get(value: &Value, json_path: &str) -> Option<Value> {
    let parts: Vec<&str> = json_path.split('.').filter(|p| !p.is_empty()).collect();
    let mut current = value;
    for part in parts {
        match current {
            Value::Object(map) => {
                current = map.get(part)?;
            }
            Value::Array(arr) => {
                let idx: usize = part.parse().ok()?;
                current = arr.get(idx)?;
            }
            _ => return None,
        }
    }
    Some(current.clone())
}

/// Set a value at a json_path in a JSON value, creating intermediate objects as needed.
/// Returns the modified root value.
fn json_path_set(root: &mut Value, json_path: &str, new_value: Value) -> Result<(), String> {
    let parts: Vec<&str> = json_path.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        *root = new_value;
        return Ok(());
    }

    let mut current = root;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;
        if is_last {
            match current {
                Value::Object(map) => {
                    map.insert(part.to_string(), new_value);
                    return Ok(());
                }
                Value::Array(arr) => {
                    let idx: usize = part
                        .parse()
                        .map_err(|_| format!("invalid array index: {}", part))?;
                    if idx < arr.len() {
                        arr[idx] = new_value;
                        return Ok(());
                    } else {
                        return Err(format!("array index out of bounds: {}", idx));
                    }
                }
                _ => return Err(format!("cannot index into non-object/array at '{}'", part)),
            }
        } else {
            // Navigate or create intermediate object
            match current {
                Value::Object(map) => {
                    current = map.entry(part.to_string()).or_insert_with(|| json!({}));
                }
                Value::Array(arr) => {
                    let idx: usize = part
                        .parse()
                        .map_err(|_| format!("invalid array index: {}", part))?;
                    if idx < arr.len() {
                        current = &mut arr[idx];
                    } else {
                        return Err(format!("array index out of bounds: {}", idx));
                    }
                }
                _ => {
                    return Err(format!(
                        "cannot navigate through non-object/array at '{}'",
                        part
                    ))
                }
            }
        }
    }
    Ok(())
}

/// Delete a key at a json_path in a JSON value.
fn json_path_delete(root: &mut Value, json_path: &str) -> Result<Option<Value>, String> {
    let parts: Vec<&str> = json_path.split('.').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Err("empty json_path for delete".to_string());
    }

    let mut current = root;
    for (i, part) in parts.iter().enumerate() {
        let is_last = i == parts.len() - 1;
        if is_last {
            match current {
                Value::Object(map) => return Ok(map.remove(*part)),
                Value::Array(arr) => {
                    let idx: usize = part
                        .parse()
                        .map_err(|_| format!("invalid array index: {}", part))?;
                    if idx < arr.len() {
                        return Ok(Some(arr.remove(idx)));
                    }
                    return Ok(None);
                }
                _ => return Err(format!("cannot delete from non-object/array at '{}'", part)),
            }
        } else {
            match current {
                Value::Object(map) => {
                    current = map
                        .get_mut(*part)
                        .ok_or_else(|| format!("path not found at '{}'", part))?;
                }
                Value::Array(arr) => {
                    let idx: usize = part
                        .parse()
                        .map_err(|_| format!("invalid array index: {}", part))?;
                    current = arr
                        .get_mut(idx)
                        .ok_or_else(|| format!("array index out of bounds: {}", idx))?;
                }
                _ => {
                    return Err(format!(
                        "cannot navigate through non-object/array at '{}'",
                        part
                    ))
                }
            }
        }
    }
    Ok(None)
}

impl ControlPanelServer {
    /// Handle HTTP requests to /api/desktop
    pub(super) async fn handle_desktop_api(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let path = req.uri().path();
        if path == WALLPAPER_ROUTE || path.starts_with(&format!("{}/", WALLPAPER_ROUTE)) {
            return self.handle_desktop_wallpaper(req).await;
        }
        if req.method() != Method::POST {
            return self.desktop_json_response(
                StatusCode::METHOD_NOT_ALLOWED,
                json!({"ok": false, "error": "Method not allowed, use POST"}),
            );
        }

        // Extract auth token
        let token = Self::extract_http_session_token(&req);
        let principal = match self
            .authenticate_session_token_for_method("desktop.api", token)
            .await
        {
            Ok(Some(p)) => p,
            Ok(None) => {
                return self.desktop_json_response(
                    StatusCode::UNAUTHORIZED,
                    json!({"ok": false, "error": "authentication required"}),
                );
            }
            Err(e) => {
                return self.desktop_json_response(
                    StatusCode::UNAUTHORIZED,
                    json!({"ok": false, "error": format!("{}", e)}),
                );
            }
        };

        // Read body
        let body_bytes = req
            .into_body()
            .collect()
            .await
            .map_err(|e| server_err!(ServerErrorCode::BadRequest, "read body: {:?}", e))?
            .to_bytes();

        let body: Value = serde_json::from_slice(&body_bytes)
            .map_err(|e| server_err!(ServerErrorCode::BadRequest, "invalid JSON body: {}", e))?;

        let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
        if let Err(error) = principal.require_agent_permission(&format!("desktop.{action}")) {
            return self.desktop_json_response(
                StatusCode::FORBIDDEN,
                json!({"ok": false, "error": error.to_string()}),
            );
        }

        let userid = &principal.username;

        let session_id = body.get("session_id").and_then(|v| v.as_str());
        let state_key = body.get("state_key").and_then(|v| v.as_str());
        if session_id.is_some_and(|id| !is_path_segment(id))
            || state_key.is_some_and(|key| !is_state_key(key))
        {
            return self.desktop_json_response(
                StatusCode::BAD_REQUEST,
                json!({"ok": false, "error": "session_id and state_key must be [A-Za-z0-9_-]{1,64}; a state_key cannot start with _"}),
            );
        }

        let result = match action {
            "session.list" => self.desktop_session_list(userid).await,
            "session.create" => {
                let name = body
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("default");
                self.desktop_session_create(userid, name).await
            }
            "session.delete" => {
                let session_id = body
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing session_id".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                self.desktop_session_delete(userid, session_id).await
            }
            "session.rename" => {
                let session_id = body
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing session_id".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                let name = body
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing name".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                self.desktop_session_rename(userid, session_id, name).await
            }
            "state.get" => {
                let session_id = body
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing session_id".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                let state_key = body
                    .get("state_key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing state_key".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                let json_path = body.get("json_path").and_then(|v| v.as_str());
                self.desktop_state_get(userid, session_id, state_key, json_path)
                    .await
            }
            "state.set" => {
                let session_id = body
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing session_id".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                let state_key = body
                    .get("state_key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing state_key".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                let json_path = body.get("json_path").and_then(|v| v.as_str());
                let value = body
                    .get("value")
                    .cloned()
                    .ok_or_else(|| server_err!(ServerErrorCode::BadRequest, "missing value"))?;
                self.desktop_state_set(userid, session_id, state_key, json_path, value)
                    .await
            }
            "state.delete" => {
                let session_id = body
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing session_id".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                let state_key = body
                    .get("state_key")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| "missing state_key".to_string())
                    .map_err(|e| server_err!(ServerErrorCode::BadRequest, "{}", e))?;
                let json_path = body.get("json_path").and_then(|v| v.as_str());
                self.desktop_state_delete(userid, session_id, state_key, json_path)
                    .await
            }
            "items.list" | "items.write" | "items.delete" => {
                return self.desktop_items_action(userid, action, &body).await;
            }
            _ => {
                return self.desktop_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({"ok": false, "error": format!("unknown action: {}", action)}),
                );
            }
        };

        match result {
            Ok(data) => {
                self.desktop_json_response(StatusCode::OK, json!({"ok": true, "data": data}))
            }
            Err(e) => self.desktop_json_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                json!({"ok": false, "error": e}),
            ),
        }
    }

    // ── Desktop folder (desktop_folder.rs) ──────────────────────────────

    /// The items of the user's Desktop folder; a refused request answers 400, a failed one 500.
    async fn desktop_items_action(
        &self,
        userid: &str,
        action: &str,
        body: &Value,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let bad_request = |error: String| {
            self.desktop_json_response(
                StatusCode::BAD_REQUEST,
                json!({"ok": false, "error": error}),
            )
        };
        let Some(dir) = desktop_folder::desktop_dir(userid) else {
            return bad_request("this account has no home folder".to_string());
        };
        let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let result = match action {
            "items.list" => {
                let mut seed = match desktop_folder::parse_seed(body.get("seed")) {
                    Ok(seed) => seed,
                    Err(error) => return bad_request(error),
                };
                if !desktop_folder::folder_exists(&dir).await {
                    let declared = self.declared_desktop_shortcuts(&seed).await;
                    seed.extend(declared);
                }
                desktop_folder::list_items(&dir, &seed)
                    .await
                    .map(|(created, items)| {
                        json!({
                            "path": desktop_folder::desktop_dfs_path(userid),
                            "created": created,
                            "items": items,
                        })
                    })
            }
            "items.write" => {
                let Some(text) = body.get("text").and_then(|v| v.as_str()) else {
                    return bad_request("missing text".to_string());
                };
                if !desktop_folder::is_desktop_object_name(name) {
                    return bad_request(format!("not a .lnk or .widget name: {}", name));
                }
                let create = body
                    .get("create")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match desktop_folder::write_item(&dir, name, text, create).await {
                    Err(error) if error.starts_with("exists:") => return bad_request(error),
                    other => other.map(|()| json!({"name": name})),
                }
            }
            _ => {
                if !desktop_folder::is_desktop_object_name(name) {
                    return bad_request(format!("not a .lnk or .widget name: {}", name));
                }
                desktop_folder::delete_item(&dir, name)
                    .await
                    .map(|()| json!({"name": name}))
            }
        };
        match result {
            Ok(data) => {
                self.desktop_json_response(StatusCode::OK, json!({"ok": true, "data": data}))
            }
            Err(error) => {
                log::error!("desktop: {} for {}: {}", action, userid, error);
                self.desktop_json_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    json!({"ok": false, "error": error}),
                )
            }
        }
    }

    /// Shortcuts `system/install_settings` declares for pre-installed Apps; none when it cannot be read.
    async fn declared_desktop_shortcuts(
        &self,
        seed: &[desktop_folder::DesktopSeedItem],
    ) -> Vec<desktop_folder::DesktopSeedItem> {
        let settings = async {
            let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
            let client = runtime
                .get_system_config_client()
                .await
                .map_err(|e| e.to_string())?;
            let value = client
                .get(super::pre_install_reconciler::INSTALL_SETTINGS_KEY)
                .await
                .map_err(|e| e.to_string())?;
            serde_json::from_str::<buckyos_api::SystemInstallSettings>(&value.value)
                .map_err(|e| e.to_string())
        }
        .await;
        match settings {
            Ok(settings) => desktop_folder::declared_shortcuts(&settings, seed),
            Err(error) => {
                log::warn!(
                    "desktop: install_settings unavailable for desktop shortcuts: {}",
                    error
                );
                Vec::new()
            }
        }
    }

    // ── Session management ──────────────────────────────────────────────

    /// List all UI sessions for a user.
    async fn desktop_session_list(&self, userid: &str) -> Result<Value, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("get runtime: {}", e))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|e| format!("get config client: {}", e))?;

        let prefix = desktop_session_prefix(userid);
        let items = client
            .list(&prefix)
            .await
            .map_err(|e| format!("list sessions: {}", e))?;

        // items are direct children under the prefix — each is a session_id
        let mut sessions: Vec<Value> = Vec::new();
        for item in &items {
            let session_id = item
                .strip_prefix(&format!("{}/", prefix))
                .unwrap_or(item)
                .to_string();
            // Skip sub-keys (those containing '/') — we only want session-level entries
            if session_id.contains('/') || session_id.starts_with('_') {
                continue;
            }

            // Try to load metadata
            let meta_key = desktop_session_meta_key(userid, &session_id);
            let meta = match client.get(&meta_key).await {
                Ok(v) => serde_json::from_str::<Value>(&v.value).unwrap_or(json!({})),
                Err(_) => json!({}),
            };

            sessions.push(json!({
                "session_id": session_id,
                "name": meta.get("name").and_then(|v| v.as_str()).unwrap_or(&session_id),
                "created_at": meta.get("created_at"),
                "updated_at": meta.get("updated_at"),
            }));
        }

        Ok(json!({ "sessions": sessions }))
    }

    /// Create a new UI session.
    async fn desktop_session_create(&self, userid: &str, name: &str) -> Result<Value, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("get runtime: {}", e))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|e| format!("get config client: {}", e))?;

        let session_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();

        let meta = json!({
            "name": name,
            "created_at": now,
            "updated_at": now,
        });

        let meta_key = desktop_session_meta_key(userid, &session_id);
        let serialized =
            serde_json::to_string(&meta).map_err(|e| format!("serialize meta: {}", e))?;
        client
            .set(&meta_key, &serialized)
            .await
            .map_err(|e| format!("save session meta: {}", e))?;

        log::info!(
            "desktop: created session {} for user {}",
            session_id,
            userid
        );

        Ok(json!({
            "session_id": session_id,
            "name": name,
            "created_at": now,
        }))
    }

    /// Delete a UI session and all its state.
    async fn desktop_session_delete(
        &self,
        userid: &str,
        session_id: &str,
    ) -> Result<Value, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("get runtime: {}", e))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|e| format!("get config client: {}", e))?;

        let session_prefix = desktop_session_key(userid, session_id);

        // List all keys under this session and delete them
        let items = client
            .list(&session_prefix)
            .await
            .map_err(|e| format!("list session keys: {}", e))?;

        for item in &items {
            let _ = client.delete(item).await;
        }

        // Also delete the meta key explicitly
        let meta_key = desktop_session_meta_key(userid, session_id);
        let _ = client.delete(&meta_key).await;

        log::info!(
            "desktop: deleted session {} for user {} ({} keys removed)",
            session_id,
            userid,
            items.len()
        );

        Ok(json!({
            "session_id": session_id,
            "deleted_keys": items.len(),
        }))
    }

    /// Rename a UI session.
    async fn desktop_session_rename(
        &self,
        userid: &str,
        session_id: &str,
        new_name: &str,
    ) -> Result<Value, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("get runtime: {}", e))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|e| format!("get config client: {}", e))?;

        let meta_key = desktop_session_meta_key(userid, session_id);
        let now = chrono::Utc::now().to_rfc3339();

        // Load existing meta or create new
        let mut meta = match client.get(&meta_key).await {
            Ok(v) => serde_json::from_str::<Value>(&v.value).unwrap_or(json!({})),
            Err(_) => json!({}),
        };

        if let Some(obj) = meta.as_object_mut() {
            obj.insert("name".to_string(), json!(new_name));
            obj.insert("updated_at".to_string(), json!(now));
        }

        let serialized =
            serde_json::to_string(&meta).map_err(|e| format!("serialize meta: {}", e))?;
        client
            .set(&meta_key, &serialized)
            .await
            .map_err(|e| format!("save session meta: {}", e))?;

        Ok(json!({
            "session_id": session_id,
            "name": new_name,
        }))
    }

    // ── State read/write ────────────────────────────────────────────────

    /// Get a state value, optionally at a json_path.
    async fn desktop_state_get(
        &self,
        userid: &str,
        session_id: &str,
        state_key: &str,
        json_path: Option<&str>,
    ) -> Result<Value, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("get runtime: {}", e))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|e| format!("get config client: {}", e))?;

        let key = desktop_state_key(userid, session_id, state_key);
        let config_value = match client.get(&key).await {
            Ok(value) => value,
            // Never written is an answer, not a failure: the desktop then seeds the state from what it has.
            Err(SystemConfigError::KeyNotFound(_)) => {
                return Ok(json!({
                    "session_id": session_id,
                    "state_key": state_key,
                    "value": Value::Null,
                    "version": Value::Null,
                    "exists": false,
                }))
            }
            Err(e) => return Err(format!("get state: {}", e)),
        };

        let parsed: Value = serde_json::from_str(&config_value.value)
            .unwrap_or(Value::String(config_value.value.clone()));

        let result = if let Some(path) = json_path {
            json_path_get(&parsed, path).unwrap_or(Value::Null)
        } else {
            parsed
        };

        Ok(json!({
            "session_id": session_id,
            "state_key": state_key,
            "value": result,
            "version": config_value.version,
            "exists": true,
        }))
    }

    /// Set a state value, optionally at a json_path for partial update.
    async fn desktop_state_set(
        &self,
        userid: &str,
        session_id: &str,
        state_key: &str,
        json_path: Option<&str>,
        value: Value,
    ) -> Result<Value, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("get runtime: {}", e))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|e| format!("get config client: {}", e))?;

        let key = desktop_state_key(userid, session_id, state_key);

        let final_value = if let Some(path) = json_path {
            // Partial update: load existing value, merge at path
            let existing = match client.get(&key).await {
                Ok(v) => serde_json::from_str::<Value>(&v.value).unwrap_or(json!({})),
                Err(_) => json!({}),
            };
            let mut root = existing;
            json_path_set(&mut root, path, value)?;
            root
        } else {
            value
        };

        let serialized =
            serde_json::to_string(&final_value).map_err(|e| format!("serialize state: {}", e))?;
        client
            .set(&key, &serialized)
            .await
            .map_err(|e| format!("set state: {}", e))?;

        // Update session's updated_at timestamp. A session first written without `session.create` (the
        // desktop's well-known `default` one, shared by all of the user's devices) gets its metadata here.
        let meta_key = desktop_session_meta_key(userid, session_id);
        let now = chrono::Utc::now().to_rfc3339();
        match client.get(&meta_key).await {
            Ok(meta_val) => {
                if let Ok(mut meta) = serde_json::from_str::<Value>(&meta_val.value) {
                    if let Some(obj) = meta.as_object_mut() {
                        obj.insert("updated_at".to_string(), json!(now));
                        if let Ok(s) = serde_json::to_string(&meta) {
                            let _ = client.set(&meta_key, &s).await;
                        }
                    }
                }
            }
            Err(SystemConfigError::KeyNotFound(_)) => {
                let meta = json!({"name": session_id, "created_at": now, "updated_at": now});
                let _ = client.set(&meta_key, &meta.to_string()).await;
            }
            Err(_) => {}
        }

        Ok(json!({
            "session_id": session_id,
            "state_key": state_key,
        }))
    }

    /// Delete a state key entirely, or delete a sub-path within a state value.
    async fn desktop_state_delete(
        &self,
        userid: &str,
        session_id: &str,
        state_key: &str,
        json_path: Option<&str>,
    ) -> Result<Value, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| format!("get runtime: {}", e))?;
        let client = runtime
            .get_system_config_client()
            .await
            .map_err(|e| format!("get config client: {}", e))?;

        let key = desktop_state_key(userid, session_id, state_key);

        if let Some(path) = json_path {
            // Partial delete: load existing value, remove at path
            let existing = match client.get(&key).await {
                Ok(v) => serde_json::from_str::<Value>(&v.value).unwrap_or(json!({})),
                Err(_) => return Err("state key not found".to_string()),
            };
            let mut root = existing;
            json_path_delete(&mut root, path)?;

            let serialized =
                serde_json::to_string(&root).map_err(|e| format!("serialize state: {}", e))?;
            client
                .set(&key, &serialized)
                .await
                .map_err(|e| format!("set state: {}", e))?;
        } else {
            // Delete the entire state key
            client
                .delete(&key)
                .await
                .map_err(|e| format!("delete state: {}", e))?;
        }

        Ok(json!({
            "session_id": session_id,
            "state_key": state_key,
        }))
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    fn desktop_json_response(
        &self,
        status: StatusCode,
        body: Value,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let json_bytes = serde_json::to_vec(&body).unwrap_or_default();
        http::Response::builder()
            .status(status)
            .header("Content-Type", "application/json")
            .body(Self::boxed_http_body(json_bytes))
            .map_err(|e| server_err!(ServerErrorCode::BadRequest, "build response: {}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_and_state_keys_are_single_segments() {
        assert!(is_path_segment("default"));
        assert!(is_path_segment("3f2b9c1e-7d1a-4c55-9a0e-1b2c3d4e5f60"));
        assert!(is_state_key("appearance"));
        assert!(is_state_key("window_layout"));
        assert!(!is_path_segment(""));
        assert!(!is_path_segment("a/b"));
        assert!(!is_path_segment(".."));
        assert!(!is_path_segment(&"a".repeat(65)));
        assert!(!is_state_key("_meta"));
    }

    #[test]
    fn json_path_set_merges_one_field() {
        let mut root = json!({"theme": "light", "wallpaper": {"fit": "fill"}});
        json_path_set(&mut root, "wallpaper.fit", json!("tile")).unwrap();
        json_path_set(&mut root, "locale", json!("zh-CN")).unwrap();
        assert_eq!(
            root,
            json!({"theme": "light", "locale": "zh-CN", "wallpaper": {"fit": "tile"}})
        );
    }
}
