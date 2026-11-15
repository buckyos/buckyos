//! Files between the Zone and the pages (Agent Browser Protocol §3.4, §4):
//! a file the user chose for a page's file input comes from the Zone's file
//! system (`browser.upload`) or from the device the App runs on (the `PUT`
//! entry); a file the browser downloaded becomes an object the user can
//! save into their files (`browser.download_export`).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{get_buckyos_api_runtime, validate_verify_hub_token_claims, TokenUse};
use buckyos_http_server::ServerError;
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use http_body_util::BodyExt;
use libopendan::runner::ObjectStore;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use super::rpc::BrowserError;
use super::BrowserModule;

/// What one uploaded file may be at most, unless the module is tuned otherwise.
pub const UPLOAD_LIMIT: u64 = 100 * 1024 * 1024;
/// The version of the Zone's file protocol the module speaks.
const NFSP_VERSION: &str = "nfsp/0";

pub fn too_large(limit: u64) -> BrowserError {
    BrowserError::new("upload_too_large", format!("a file to upload may have at most {} MiB", limit / (1024 * 1024)))
}

fn other(what: impl std::fmt::Display) -> BrowserError {
    BrowserError::new("other", what.to_string())
}

/// The Zone as the transfers use it, on the caller's behalf.
#[async_trait]
pub trait Zone: Send + Sync + std::fmt::Debug {
    /// Copy the Zone file at `zone_path` to `dest`, read with the caller's
    /// `token`. More than `limit` bytes is `upload_too_large`.
    async fn read(&self, token: Option<&str>, zone_path: &str, dest: &Path, limit: u64) -> Result<u64, BrowserError>;
    /// Make the file an object the caller can save into their files with
    /// their own session. Returns its object id.
    async fn export(&self, token: Option<&str>, path: &Path, name: &str) -> Result<String, BrowserError>;
}

/// The Zone this service runs in: files through nfs-server at the node's
/// gateway, objects in the named store and the agent's `artifacts` bucket.
#[derive(Debug, Default)]
pub struct RuntimeZone;

#[async_trait]
impl Zone for RuntimeZone {
    async fn read(&self, token: Option<&str>, zone_path: &str, dest: &Path, limit: u64) -> Result<u64, BrowserError> {
        let runtime = get_buckyos_api_runtime().map_err(|e| other(format!("this agent runs outside a Zone: {e}")))?;
        let origin = format!("http://{}:{}", runtime.get_local_gateway_host(), runtime.node_gateway_port);
        read_zone_file(&origin, token, zone_path, dest, limit).await
    }

    async fn export(&self, token: Option<&str>, path: &Path, name: &str) -> Result<String, BrowserError> {
        let stored = crate::objects::NamedStoreObjects.put_file(path, name).await.map_err(other)?;
        let obj_id = stored.obj_id.to_string();
        // Handed to the caller, as a service does that shows a user an object
        // (msg-center, HomeStation). Without it only an administrator's own
        // session reads the object.
        if let Some(token) = token {
            if let Err(e) = grant(token, &obj_id).await {
                log::warn!("browser: download {name} ({obj_id}) not handed to its user: {e}");
            }
        }
        Ok(obj_id)
    }
}

async fn grant(token: &str, obj_id: &str) -> Result<(), String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
    let verified = runtime.verify_trusted_session_token(token).await.map_err(|e| e.to_string())?;
    let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session).map_err(|e| e.to_string())?;
    let user = verified.sub.as_deref().ok_or("the token names no user")?;
    let buckets = runtime.get_bucket_client().await.map_err(|e| e.to_string())?;
    buckets.grant_holdings(user, &claims.target.canonical_key(), &[obj_id.to_string()]).await.map_err(|e| e.to_string())
}

/// Read a file of the Zone's file system the way the Desktop does: `stat`
/// its name in its folder, then the data plane (`/nfs/v1/read/<node>`), all
/// with the caller's token.
pub async fn read_zone_file(origin: &str, token: Option<&str>, zone_path: &str, dest: &Path, limit: u64) -> Result<u64, BrowserError> {
    let (parent, name) = zone_path.rsplit_once('/').filter(|(_, name)| !name.is_empty()).ok_or_else(|| BrowserError::invalid(format!("`{zone_path}` is not the path of a file")))?;
    if !zone_path.starts_with('/') || zone_path.split('/').any(|segment| segment == "." || segment == "..") {
        return Err(BrowserError::invalid(format!("`{zone_path}` is not a path of the Zone's file system")));
    }
    let http = reqwest::Client::new();
    let authorized = |request: reqwest::RequestBuilder| match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };
    let call = |method: &'static str, body: Value| {
        let request = authorized(http.post(format!("{origin}/nfs/v1/{method}")).json(&body));
        async move {
            let answer: Value = request.send().await.map_err(|e| other(format!("nfs-server {method}: {e}")))?.json().await.map_err(|e| other(format!("nfs-server {method}: {e}")))?;
            if answer["ok"] == true {
                return Ok(answer["result"].clone());
            }
            let (code, message) = (answer["error"]["code"].as_str().unwrap_or("INTERNAL"), answer["error"]["message"].as_str().unwrap_or_default());
            let why = format!("`{zone_path}`: {message} ({code})");
            Err(if matches!(code, "NOT_FOUND" | "PERMISSION_DENIED" | "INVALID_ARGUMENT") { BrowserError::invalid(why) } else { other(why) })
        }
    };
    let greeted = call("hello", json!({ "args": { "versions": [NFSP_VERSION] } })).await?;
    let located = json!({
        "session": greeted["session"],
        "at": { "realm": "dfs", "path": if parent.is_empty() { "/" } else { parent } },
        "args": { "name": name },
        "want": ["base"],
    });
    let node = call("stat", located).await?;
    let node_id = match (node["kind"].as_str(), node["ref"]["node_id"].as_str()) {
        (Some("dir"), _) | (_, None) => return Err(BrowserError::invalid(format!("`{zone_path}` is not a file"))),
        (_, Some(node_id)) => node_id,
    };
    if node["size"].as_u64().is_some_and(|size| size > limit) {
        return Err(too_large(limit));
    }
    let mut response = authorized(http.get(format!("{origin}/nfs/v1/read/{node_id}"))).send().await.map_err(|e| other(format!("nfs-server read: {e}")))?;
    if !response.status().is_success() {
        return Err(other(format!("`{zone_path}`: nfs-server read answered {}", response.status())));
    }
    let mut file = tokio::fs::File::create(dest).await.map_err(|e| other(format!("create {}: {e}", dest.display())))?;
    let mut size = 0u64;
    while let Some(chunk) = response.chunk().await.map_err(|e| other(format!("`{zone_path}`: {e}")))? {
        size += chunk.len() as u64;
        if size > limit {
            return Err(too_large(limit));
        }
        file.write_all(&chunk).await.map_err(|e| other(format!("write {}: {e}", dest.display())))?;
    }
    file.flush().await.map_err(|e| other(format!("write {}: {e}", dest.display())))?;
    Ok(size)
}

fn zone_of(module: &BrowserModule) -> Arc<dyn Zone> {
    module.tuning.zone.clone().unwrap_or_else(|| Arc::new(RuntimeZone))
}

/// `browser.upload`: a file of the Zone for a file chooser request.
pub async fn upload(module: &BrowserModule, token: Option<&str>, request_id: &str, zone_path: &str, done: bool) -> Result<(), BrowserError> {
    let name = zone_path.rsplit('/').next().unwrap_or_default();
    let path = module.outside.stage(request_id, name)?;
    match zone_of(module).read(token, zone_path, &path, module.tuning.upload_limit).await {
        Ok(_) => module.outside.staged(module, request_id, path, done).await,
        Err(e) => {
            module.outside.unstage(request_id, &path);
            Err(e)
        }
    }
}

/// The `PUT` entry: a file of the App's device for a file chooser request.
/// The body is written as it arrives.
pub async fn receive(module: &BrowserModule, request_id: &str, name: &str, done: bool, mut body: BoxBody<Bytes, ServerError>) -> Result<(), BrowserError> {
    let limit = module.tuning.upload_limit;
    let path = module.outside.stage(request_id, name)?;
    let written = async {
        let mut file = tokio::fs::File::create(&path).await.map_err(|e| other(format!("create {}: {e}", path.display())))?;
        let mut size = 0u64;
        while let Some(frame) = body.frame().await {
            let Ok(chunk) = frame.map_err(|e| other(format!("upload: {e}")))?.into_data() else { continue };
            size += chunk.len() as u64;
            if size > limit {
                return Err(too_large(limit));
            }
            file.write_all(&chunk).await.map_err(|e| other(format!("write {}: {e}", path.display())))?;
        }
        file.flush().await.map_err(|e| other(format!("write {}: {e}", path.display())))
    }
    .await;
    match written {
        Ok(()) => module.outside.staged(module, request_id, path, done).await,
        Err(e) => {
            module.outside.unstage(request_id, &path);
            Err(e)
        }
    }
}

/// `browser.download_export`: a finished download as an object.
pub async fn export(module: &BrowserModule, token: Option<&str>, download_id: &str) -> Result<Value, BrowserError> {
    let file = module.outside.download(download_id)?;
    let obj_id = zone_of(module).export(token, &file.path, &file.name).await?;
    Ok(json!({ "obj_id": obj_id, "name": file.name, "size": file.size }))
}
