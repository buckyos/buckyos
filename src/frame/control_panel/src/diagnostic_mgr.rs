use crate::redaction::{redact_text, REDACTION_VERSION};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, ActorRef, CreateDelegatedTaskReq, StorageDomain, TaskPhase,
    DIAGNOSTIC_COLLECT_TASK_SCHEMA_ID,
};
use buckyos_http_server::*;
use bytes::Bytes;
use chrono::{DateTime, Utc};
use http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task;
use uuid::Uuid;
use zip::write::FileOptions;
use zip::CompressionMethod;

const DIAGNOSTIC_TTL_SECS: u64 = 3600;
const DIAGNOSTIC_MAX_CONTENT_BYTES: usize = 50 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct DiagnosticBundleEntry {
    pub(crate) path: PathBuf,
    pub(crate) filename: String,
    pub(crate) expires_at: SystemTime,
    pub(crate) metadata: Value,
    pub(crate) artifact_sha256: String,
    pub(crate) download_token: String,
}

#[derive(Clone)]
struct DiagnosticScope {
    services: Vec<String>,
    since: Option<String>,
    until: Option<String>,
    since_time: Option<DateTime<Utc>>,
    until_time: Option<DateTime<Utc>>,
}

impl ControlPanelServer {
    pub(crate) async fn handle_diagnostic_collect(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        if !Self::is_privileged(principal) {
            return Err(RPCErrors::NoPermission(
                "diagnostic collection requires administrator privileges".to_string(),
            ));
        }
        let mut services = req
            .params
            .get("services")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        services.retain(|service| seen.insert(service.clone()));
        if services.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "diagnostic services must not be empty".to_string(),
            ));
        }
        let available = self.list_log_service_ids()?;
        if let Some(service) = services.iter().find(|service| !available.contains(service)) {
            return Err(RPCErrors::ReasonError(format!(
                "Unknown log service: {}",
                service
            )));
        }
        let since = Self::param_str(&req, "since");
        let until = Self::param_str(&req, "until");
        let since_time = Self::parse_diagnostic_time("since", since.as_deref())?;
        let until_time = Self::parse_diagnostic_time("until", until.as_deref())?;
        if since_time
            .zip(until_time)
            .is_some_and(|(start, end)| start > end)
        {
            return Err(RPCErrors::ParseRequestError(
                "diagnostic since must not be later than until".to_string(),
            ));
        }
        let idempotency_key = Self::require_param_str(&req, "idempotency_key")?;
        let scope = DiagnosticScope {
            services,
            since,
            until,
            since_time,
            until_time,
        };
        let task = get_buckyos_api_runtime()?
            .get_task_mgr_client()
            .await?
            .create_delegated_task(CreateDelegatedTaskReq {
                task_id: None,
                name: "Collect diagnostic bundle".to_string(),
                schema_id: DIAGNOSTIC_COLLECT_TASK_SCHEMA_ID.to_string(),
                schema_version: None,
                input: json!({ "scope": scope.to_json() }),
                creator: ActorRef::new(
                    principal.username.clone(),
                    principal.authenticated_app_id.clone(),
                ),
                runner_app_instance_id: None,
                parent_id: None,
                child_control_policy: None,
                policy_preset: None,
                permission_boundary: false,
                storage_domain: Some(StorageDomain::System),
                idempotency_key,
                retry_of: None,
                supersedes: None,
                message: None,
            })
            .await?;
        if task.phase != TaskPhase::Terminal {
            let should_spawn = self
                .running_diagnostic_tasks
                .lock()
                .await
                .insert(task.task_id.clone());
            if should_spawn {
                let server = self.clone();
                let task_id = task.task_id.clone();
                tokio::spawn(async move {
                    server.run_diagnostic_task(task_id, scope).await;
                });
            }
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "task_id": task.task_id })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_diagnostic_export(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        if !Self::is_privileged(principal) {
            return Err(RPCErrors::NoPermission(
                "diagnostic export requires administrator privileges".to_string(),
            ));
        }
        let bundle_id = Self::require_param_str(&req, "bundle_id")?;
        self.cleanup_diagnostic_bundles().await;
        let entry = self
            .diagnostic_bundles
            .lock()
            .await
            .get(&bundle_id)
            .cloned()
            .ok_or_else(|| {
                RPCErrors::ReasonError("diagnostic bundle not found or expired".into())
            })?;
        let mut response = entry.metadata.as_object().cloned().unwrap_or_default();
        response.insert(
            "url".into(),
            json!(format!(
                "/kapi/control-panel/diagnostics/download/{}",
                entry.download_token
            )),
        );
        response.insert("filename".into(), json!(entry.filename));
        response.insert("artifact_sha256".into(), json!(entry.artifact_sha256));
        Ok(RPCResponse::new(
            RPCResult::Success(Value::Object(response)),
            req.seq,
        ))
    }

    pub(crate) async fn handle_diagnostic_download_http(
        &self,
        download_token: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        self.cleanup_diagnostic_bundles().await;
        let entry = self
            .diagnostic_bundles
            .lock()
            .await
            .values()
            .find(|entry| entry.download_token == download_token)
            .cloned()
            .ok_or_else(|| server_err!(ServerErrorCode::BadRequest, "Invalid bundle id"))?;
        let content = tokio::fs::read(&entry.path).await.map_err(|error| {
            server_err!(ServerErrorCode::InvalidData, "Read zip error: {}", error)
        })?;
        let body = BoxBody::new(
            Full::new(Bytes::from(content))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        );
        http::Response::builder()
            .header(CONTENT_TYPE, "application/zip")
            .header(
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", entry.filename),
            )
            .header(CACHE_CONTROL, "no-store")
            .body(body)
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build download response: {}",
                    error
                )
            })
    }

    async fn run_diagnostic_task(&self, task_id: String, scope: DiagnosticScope) {
        let result = self.build_diagnostic_bundle(&task_id, scope).await;
        let client = match get_buckyos_api_runtime() {
            Ok(runtime) => match runtime.get_task_mgr_client().await {
                Ok(client) => client,
                Err(error) => {
                    log::error!(
                        "diagnostic task {} cannot get TaskManager: {}",
                        task_id,
                        error
                    );
                    self.running_diagnostic_tasks.lock().await.remove(&task_id);
                    return;
                }
            },
            Err(error) => {
                log::error!("diagnostic task {} cannot get runtime: {}", task_id, error);
                self.running_diagnostic_tasks.lock().await.remove(&task_id);
                return;
            }
        };
        match result {
            Ok((entry, metadata)) => {
                let bundle_id = metadata
                    .get("bundle_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.diagnostic_bundles
                    .lock()
                    .await
                    .insert(bundle_id, entry);
                let cleanup = self.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_secs(DIAGNOSTIC_TTL_SECS)).await;
                    cleanup.cleanup_diagnostic_bundles().await;
                });
                if let Err(error) = client.runner_complete(&task_id, metadata).await {
                    log::error!("diagnostic task {} completion failed: {}", task_id, error);
                }
            }
            Err(error) => {
                if let Err(report_error) = client
                    .runner_fail(
                        &task_id,
                        "diagnostic_collect_failed",
                        error.to_string(),
                        None,
                    )
                    .await
                {
                    log::error!(
                        "diagnostic task {} failure report failed: {}",
                        task_id,
                        report_error
                    );
                }
            }
        }
        self.running_diagnostic_tasks.lock().await.remove(&task_id);
    }

    async fn build_diagnostic_bundle(
        &self,
        task_id: &str,
        scope: DiagnosticScope,
    ) -> Result<(DiagnosticBundleEntry, Value), RPCErrors> {
        let client = get_buckyos_api_runtime()?.get_task_mgr_client().await?;
        client.runner_start(task_id).await?;
        let bundle_id = format!("diag-{}", Uuid::new_v4().simple());
        let filename = format!("buckyos-diagnostic-{}.zip", bundle_id);
        let dir = buckyos_kit::get_buckyos_root_dir()
            .join("cache")
            .join("control_panel")
            .join("diagnostics");
        std::fs::create_dir_all(&dir).map_err(|error| {
            RPCErrors::ReasonError(format!("create diagnostic cache failed: {}", error))
        })?;
        let path = dir.join(&filename);
        let created_at = unix_millis(SystemTime::now());
        let expires_at = created_at + DIAGNOSTIC_TTL_SECS * 1000;
        let server = self.clone();
        let scope_for_bundle = scope.clone();
        let path_for_bundle = path.clone();
        let bundle_id_for_bundle = bundle_id.clone();
        let generated = task::spawn_blocking(move || {
            server.write_diagnostic_zip(
                &path_for_bundle,
                &bundle_id_for_bundle,
                &scope_for_bundle,
                created_at,
                expires_at,
            )
        })
        .await
        .map_err(|error| RPCErrors::ReasonError(format!("diagnostic task failed: {}", error)))?;
        let (metadata, artifact_sha256) = match generated {
            Ok(generated) => generated,
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                return Err(error);
            }
        };
        let entry = DiagnosticBundleEntry {
            path,
            filename,
            expires_at: UNIX_EPOCH + Duration::from_millis(expires_at),
            metadata: metadata.clone(),
            artifact_sha256,
            download_token: Uuid::new_v4().to_string(),
        };
        Ok((entry, metadata))
    }

    fn write_diagnostic_zip(
        &self,
        path: &PathBuf,
        bundle_id: &str,
        scope: &DiagnosticScope,
        created_at: u64,
        expires_at: u64,
    ) -> Result<(Value, String), RPCErrors> {
        let mut content = Vec::new();
        let mut content_size = 0usize;
        for service in &scope.services {
            let mut service_log = String::new();
            for file in self.collect_log_files(service, None)? {
                let reader = BufReader::new(std::fs::File::open(&file.path).map_err(|error| {
                    RPCErrors::ReasonError(format!("read {} failed: {}", file.name, error))
                })?);
                for line in reader.lines().map_while(Result::ok) {
                    let (timestamp, _, _) = Self::split_log_line(&line);
                    if scope.since_time.is_some() || scope.until_time.is_some() {
                        let timestamp = match Self::parse_log_timestamp(&timestamp) {
                            Some(timestamp) => timestamp,
                            None => continue,
                        };
                        if scope.since_time.is_some_and(|start| timestamp < start)
                            || scope.until_time.is_some_and(|end| timestamp > end)
                        {
                            continue;
                        }
                    }
                    let redacted = redact_text(&line);
                    content_size = content_size.saturating_add(redacted.len() + 1);
                    if content_size > DIAGNOSTIC_MAX_CONTENT_BYTES {
                        return Err(RPCErrors::ReasonError(
                            "diagnostic content exceeds 50 MiB limit".to_string(),
                        ));
                    }
                    service_log.push_str(&redacted);
                    service_log.push('\n');
                }
            }
            content.push((format!("logs/{}/diagnostic.log", service), service_log));
        }
        write_diagnostic_content_zip(
            path,
            bundle_id,
            scope,
            content,
            content_size,
            created_at,
            expires_at,
        )
    }

    async fn cleanup_diagnostic_bundles(&self) {
        let now = SystemTime::now();
        let mut expired = Vec::new();
        self.diagnostic_bundles.lock().await.retain(|_, entry| {
            if entry.expires_at <= now {
                expired.push(entry.path.clone());
                false
            } else {
                true
            }
        });
        for path in expired {
            let _ = std::fs::remove_file(path);
        }
    }

    fn parse_diagnostic_time(
        name: &str,
        value: Option<&str>,
    ) -> Result<Option<DateTime<Utc>>, RPCErrors> {
        value
            .map(|value| {
                Self::parse_filter_time(value).ok_or_else(|| {
                    RPCErrors::ParseRequestError(format!("diagnostic {} must be RFC 3339", name))
                })
            })
            .transpose()
    }
}

impl DiagnosticScope {
    fn to_json(&self) -> Value {
        json!({
            "services": self.services,
            "since": self.since,
            "until": self.until,
        })
    }
}

fn unix_millis(value: SystemTime) -> u64 {
    value
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn write_diagnostic_content_zip(
    path: &PathBuf,
    bundle_id: &str,
    scope: &DiagnosticScope,
    content: Vec<(String, String)>,
    content_size: usize,
    created_at: u64,
    expires_at: u64,
) -> Result<(Value, String), RPCErrors> {
    let mut hasher = Sha256::new();
    for (name, value) in &content {
        hasher.update(name.as_bytes());
        hasher.update([0]);
        hasher.update(value.as_bytes());
        hasher.update([0]);
    }
    let metadata = json!({
        "schema_version": 1,
        "bundle_id": bundle_id,
        "scope": scope.to_json(),
        "redaction_version": REDACTION_VERSION,
        "sha256": hex::encode(hasher.finalize()),
        "size": content_size,
        "created_at": created_at,
        "expires_at": expires_at,
    });
    let file = std::fs::File::create(path).map_err(|error| {
        RPCErrors::ReasonError(format!("create diagnostic zip failed: {}", error))
    })?;
    let mut zip = zip::ZipWriter::new(file);
    let options = FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);
    zip.start_file("manifest.json", options)
        .map_err(|error| RPCErrors::ReasonError(format!("zip manifest failed: {}", error)))?;
    zip.write_all(
        serde_json::to_string_pretty(&metadata)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
            .as_bytes(),
    )
    .map_err(|error| RPCErrors::ReasonError(format!("write manifest failed: {}", error)))?;
    for (name, value) in content {
        zip.start_file(name, options)
            .map_err(|error| RPCErrors::ReasonError(format!("zip log failed: {}", error)))?;
        zip.write_all(value.as_bytes())
            .map_err(|error| RPCErrors::ReasonError(format!("write log failed: {}", error)))?;
    }
    zip.finish()
        .map_err(|error| RPCErrors::ReasonError(format!("finish zip failed: {}", error)))?;
    let mut artifact = Vec::new();
    std::fs::File::open(path)
        .and_then(|mut file| file.read_to_end(&mut artifact))
        .map_err(|error| RPCErrors::ReasonError(format!("hash zip failed: {}", error)))?;
    Ok((metadata, hex::encode(Sha256::digest(&artifact))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostic_manifest_matches_task_result_and_archive_hash() {
        let path = std::env::temp_dir().join(format!("diagnostic-test-{}.zip", Uuid::new_v4()));
        let scope = DiagnosticScope {
            services: vec!["scheduler".to_string()],
            since: Some("2026-08-25T00:00:00Z".to_string()),
            until: None,
            since_time: None,
            until_time: None,
        };
        let content = vec![(
            "logs/scheduler/diagnostic.log".to_string(),
            "safe\n".to_string(),
        )];
        let (metadata, artifact_hash) =
            write_diagnostic_content_zip(&path, "diag-test", &scope, content, 5, 10, 20).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let manifest: Value =
            serde_json::from_reader(archive.by_name("manifest.json").unwrap()).unwrap();
        assert_eq!(manifest, metadata);
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(artifact_hash, hex::encode(Sha256::digest(bytes)));
        assert_eq!(metadata["sha256"].as_str().unwrap().len(), 64);
        let _ = std::fs::remove_file(path);
    }
}
