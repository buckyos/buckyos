//! URL downloads into the user's home (`download.create`).
//!
//! Each download is a `download/v1` task (the revision that asks for system
//! notifications) created on behalf of the calling user and run by
//! control-panel itself, the way `diagnostic.collect` is. The file goes to
//! `/home/<user>/Downloads/`: it is written as a hidden `.part` file next to
//! the destination and renamed when complete, so a half-written download is
//! never mistaken for a finished one.
//!
//! A download interrupted by a control-panel restart is not resumed: the
//! startup sweep fails it as interrupted and removes its partial file.

use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    builtin_task_schemas, get_buckyos_api_runtime, task_mgr_error_code, AckControlReq, ActorRef,
    CancelCapability, CreateDelegatedTaskReq, DownloadTaskData, DownloadTaskOptions,
    DownloadTaskRequest, DownloadTaskResult, ListTasksReq, ReportProgressReq, RunnerWriteEnvelope,
    StorageDomain, Task, TaskControlAction, TaskControlProfile, TaskDataProgress,
    TaskManagerClient, TaskPhase, UpdateControlProfileReq, DOWNLOAD_TASK_SCHEMA_ID,
    DOWNLOAD_TASK_SCHEMA_NOTIFY_VERSION, TASK_ERR_REVISION_CONFLICT, TASK_ERR_SCHEMA_NOT_FOUND,
};
use buckyos_kit::get_buckyos_root_dir;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;

const DOWNLOAD_DIR: &str = "Downloads";
const CONTROL_PANEL_APP_ID: &str = "control-panel";
/// Progress is written at most this often; every write is a TaskMgr event.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(500);
/// How often a download with no new bytes still checks for a cancel request.
const IDLE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// A response that sends nothing for this long fails the download.
const STALL_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FILENAME_BYTES: usize = 200;

#[derive(Clone, Debug)]
struct DownloadJob {
    task_id: String,
    url: String,
    username: String,
    /// The name the user asked for; otherwise derived from the response.
    filename: Option<String>,
}

enum DownloadEnd {
    Completed(Value),
    Canceled(Task),
}

impl ControlPanelServer {
    pub(crate) async fn handle_download_create(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let username = home_segment(&principal.username).ok_or_else(|| {
            RPCErrors::NoPermission("downloads need a signed-in Zone user".to_string())
        })?;
        let url = validate_url(&Self::require_param_str(&req, "url")?)?;
        let filename = match Self::param_str(&req, "filename") {
            Some(value) if !value.trim().is_empty() => Some(
                sanitize_filename(&value)
                    .ok_or_else(|| RPCErrors::ParseRequestError("invalid filename".to_string()))?,
            ),
            _ => None,
        };
        let idempotency_key = Self::require_param_str(&req, "idempotency_key")?;
        let input = DownloadTaskData {
            request: DownloadTaskRequest {
                download_url: Some(url.clone()),
                urls: vec![url.clone()],
                objid: None,
                resolved_objid: None,
                options: Some(DownloadTaskOptions {
                    local_path: Some(format!("/home/{}/{}", username, DOWNLOAD_DIR)),
                    filename: filename.clone(),
                    ..Default::default()
                }),
            },
            progress: None,
            result: None,
            extra: Default::default(),
        };
        let display_name = filename
            .clone()
            .or_else(|| filename_from_url(&url))
            .unwrap_or_else(|| url.clone());
        let client = get_buckyos_api_runtime()?.get_task_mgr_client().await?;
        let request = CreateDelegatedTaskReq {
            task_id: None,
            name: format!("Download {}", display_name),
            schema_id: DOWNLOAD_TASK_SCHEMA_ID.to_string(),
            schema_version: Some(DOWNLOAD_TASK_SCHEMA_NOTIFY_VERSION),
            input: serde_json::to_value(&input)
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
            creator: ActorRef::new(
                principal.username.clone(),
                principal.authenticated_app_id.clone(),
            ),
            runner_app_instance_id: None,
            parent_id: None,
            child_control_policy: None,
            policy_preset: None,
            permission_boundary: false,
            storage_domain: Some(StorageDomain::User),
            idempotency_key,
            retry_of: None,
            supersedes: None,
            message: None,
        };
        let task = match client.create_delegated_task(request.clone()).await {
            // A task-manager older than this control-panel has not seeded the
            // revision yet; control-panel publishes it, so it can do that itself.
            Err(error) if task_mgr_error_code(&error) == Some(TASK_ERR_SCHEMA_NOT_FOUND) => {
                let definition = builtin_task_schemas()
                    .into_iter()
                    .find(|schema| {
                        schema.schema_id == DOWNLOAD_TASK_SCHEMA_ID
                            && schema.schema_version == DOWNLOAD_TASK_SCHEMA_NOTIFY_VERSION
                    })
                    .ok_or(error)?;
                client.register_task_schema(definition).await?;
                client.create_delegated_task(request).await?
            }
            result => result?,
        };
        if task.phase != TaskPhase::Terminal {
            let should_spawn = self
                .running_download_tasks
                .lock()
                .await
                .insert(task.task_id.clone());
            if should_spawn {
                let server = self.clone();
                let job = DownloadJob {
                    task_id: task.task_id.clone(),
                    url,
                    username: username.to_string(),
                    filename,
                };
                tokio::spawn(async move {
                    server.run_download_task(job).await;
                });
            }
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "task_id": task.task_id })),
            req.seq,
        ))
    }

    /// Fails the downloads a previous control-panel process left running.
    pub(crate) fn start_download_recovery(&self) {
        let server = self.clone();
        tokio::spawn(async move {
            // TaskMgr may still be starting with the rest of the Zone.
            for attempt in 0..30u32 {
                match server.recover_interrupted_downloads().await {
                    Ok(count) => {
                        if count > 0 {
                            log::info!("download recovery: {} interrupted downloads failed", count);
                        }
                        return;
                    }
                    Err(error) => {
                        log::warn!(
                            "download recovery attempt {} failed: {}",
                            attempt + 1,
                            error
                        );
                        tokio::time::sleep(Duration::from_secs(10)).await;
                    }
                }
            }
        });
    }

    async fn recover_interrupted_downloads(&self) -> Result<usize, RPCErrors> {
        let client = get_buckyos_api_runtime()?.get_task_mgr_client().await?;
        let mut cursor = None;
        let mut interrupted = Vec::new();
        loop {
            let page = client
                .list_tasks(ListTasksReq {
                    schema_id: Some(DOWNLOAD_TASK_SCHEMA_ID.to_string()),
                    runner_app_id: Some(CONTROL_PANEL_APP_ID.to_string()),
                    cursor,
                    limit: Some(100),
                    ..Default::default()
                })
                .await?;
            interrupted.extend(
                page.tasks
                    .into_iter()
                    .filter(|task| !task.phase.is_terminal())
                    .map(|task| (task.task_id, task.creator.user_id)),
            );
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => break,
            }
        }
        let mut count = 0;
        for (task_id, user_id) in interrupted {
            // A download this process already runs is not interrupted.
            if self.running_download_tasks.lock().await.contains(&task_id) {
                continue;
            }
            if let Some(user) = home_segment(&user_id) {
                remove_partial(&partial_path(&downloads_dir(user), &task_id)).await;
            }
            let failed = client
                .runner_fail(
                    &task_id,
                    "download_interrupted",
                    "The download was interrupted when control-panel restarted.",
                    None,
                )
                .await;
            match failed {
                Ok(_) => count += 1,
                Err(error) => log::warn!("download {} recovery failed: {}", task_id, error),
            }
        }
        Ok(count)
    }

    async fn run_download_task(&self, job: DownloadJob) {
        let task_id = job.task_id.clone();
        let client = match get_buckyos_api_runtime() {
            Ok(runtime) => match runtime.get_task_mgr_client().await {
                Ok(client) => Some(client),
                Err(error) => {
                    log::error!("download {} cannot get TaskManager: {}", task_id, error);
                    None
                }
            },
            Err(error) => {
                log::error!("download {} cannot get runtime: {}", task_id, error);
                None
            }
        };
        if let Some(client) = client {
            let partial = partial_path(&downloads_dir(&job.username), &task_id);
            match download(&client, &job, &partial).await {
                Ok(DownloadEnd::Completed(result)) => {
                    if let Err(error) = client.runner_complete(&task_id, result).await {
                        log::error!("download {} completion failed: {}", task_id, error);
                    }
                }
                Ok(DownloadEnd::Canceled(task)) => {
                    remove_partial(&partial).await;
                    if let Some(control) = &task.pending_control {
                        let acked = client
                            .ack_control(AckControlReq {
                                envelope: envelope(&task),
                                request_id: control.request_id.clone(),
                                applied: true,
                                reject_reason: None,
                            })
                            .await;
                        if let Err(error) = acked {
                            log::error!("download {} cancel ack failed: {}", task_id, error);
                        }
                    }
                }
                Err(message) => {
                    remove_partial(&partial).await;
                    log::warn!("download {} failed: {}", task_id, message);
                    if let Err(error) = client
                        .runner_fail(&task_id, "download_failed", message, None)
                        .await
                    {
                        log::error!("download {} failure report failed: {}", task_id, error);
                    }
                }
            }
        }
        self.running_download_tasks.lock().await.remove(&task_id);
    }
}

/// Streams the URL into `partial`, then moves it to its final name.
async fn download(
    client: &TaskManagerClient,
    job: &DownloadJob,
    partial: &Path,
) -> Result<DownloadEnd, String> {
    let task = client
        .runner_start(&job.task_id)
        .await
        .map_err(|error| format!("start task: {}", error))?;
    if task.phase.is_terminal() {
        return Err("the task already ended".to_string());
    }
    let task = client
        .update_control_profile(UpdateControlProfileReq {
            envelope: envelope(&task),
            profile: TaskControlProfile {
                cancel: CancelCapability::Safe,
                ..TaskControlProfile::baseline(0)
            },
        })
        .await
        .map_err(|error| format!("declare cancel: {}", error))?;
    if cancel_requested(&task) {
        return Ok(DownloadEnd::Canceled(task));
    }

    let http = reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .map_err(|error| format!("http client: {}", error))?;
    let mut response = http
        .get(&job.url)
        .send()
        .await
        .map_err(|error| format!("request failed: {}", error))?;
    if !response.status().is_success() {
        return Err(format!("the server answered {}", response.status()));
    }
    let total = response.content_length();
    let filename = job
        .filename
        .clone()
        .or_else(|| filename_from_content_disposition(response.headers()))
        .or_else(|| filename_from_url(response.url().as_str()))
        .or_else(|| filename_from_url(&job.url))
        .unwrap_or_else(|| "download".to_string());

    let dir = downloads_dir(&job.username);
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|error| format!("create {}: {}", dir.display(), error))?;
    let mut file = tokio::fs::File::create(partial)
        .await
        .map_err(|error| format!("create {}: {}", partial.display(), error))?;

    let mut downloaded: u64 = 0;
    let mut reported = Instant::now();
    let mut last_data = Instant::now();
    loop {
        match tokio::time::timeout(IDLE_CHECK_INTERVAL, response.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                file.write_all(&chunk)
                    .await
                    .map_err(|error| format!("write: {}", error))?;
                downloaded += chunk.len() as u64;
                last_data = Instant::now();
                if reported.elapsed() >= PROGRESS_INTERVAL {
                    reported = Instant::now();
                    let task = report_progress(client, &job.task_id, downloaded, total).await?;
                    if cancel_requested(&task) {
                        return Ok(DownloadEnd::Canceled(task));
                    }
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(error)) => return Err(format!("download interrupted: {}", error)),
            Err(_) => {
                if last_data.elapsed() >= STALL_TIMEOUT {
                    return Err(format!(
                        "no data received for {} seconds",
                        STALL_TIMEOUT.as_secs()
                    ));
                }
                let task = client
                    .get_task(&job.task_id)
                    .await
                    .map_err(|error| format!("read task: {}", error))?;
                if task.phase.is_terminal() {
                    return Err("the task ended while downloading".to_string());
                }
                if cancel_requested(&task) {
                    return Ok(DownloadEnd::Canceled(task));
                }
            }
        }
    }
    file.flush()
        .await
        .map_err(|error| format!("flush: {}", error))?;
    file.sync_all()
        .await
        .map_err(|error| format!("sync: {}", error))?;
    drop(file);
    if let Some(expected) = total {
        if downloaded != expected {
            return Err(format!("received {} of {} bytes", downloaded, expected));
        }
    }

    // A cancel that arrived after the last progress write still wins.
    let task = report_progress(client, &job.task_id, downloaded, Some(downloaded)).await?;
    if cancel_requested(&task) {
        return Ok(DownloadEnd::Canceled(task));
    }
    let target = unique_target(&dir, &filename).await;
    tokio::fs::rename(partial, &target)
        .await
        .map_err(|error| format!("move into place: {}", error))?;
    let final_name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or(filename);
    let result = DownloadTaskResult {
        state: Some("completed".to_string()),
        local_path: Some(format!(
            "/home/{}/{}/{}",
            job.username, DOWNLOAD_DIR, final_name
        )),
        downloaded_bytes: Some(downloaded),
        total_bytes: Some(downloaded),
        completed_at: Some(now_ms()),
        ..Default::default()
    };
    serde_json::to_value(result)
        .map(DownloadEnd::Completed)
        .map_err(|error| error.to_string())
}

async fn report_progress(
    client: &TaskManagerClient,
    task_id: &str,
    downloaded: u64,
    total: Option<u64>,
) -> Result<Task, String> {
    let mut progress = TaskDataProgress::with_bytes(downloaded, total);
    progress.updated_at = Some(now_ms() as i64);
    let progress = serde_json::to_value(progress).map_err(|error| error.to_string())?;
    let message = match total {
        Some(total) => format!("{} / {}", format_bytes(downloaded), format_bytes(total)),
        None => format_bytes(downloaded),
    };
    // A cancel request bumps the revision; re-read and try again.
    for _ in 0..5 {
        let task = client
            .get_task(task_id)
            .await
            .map_err(|error| format!("read task: {}", error))?;
        if task.phase.is_terminal() {
            return Err("the task ended while downloading".to_string());
        }
        if cancel_requested(&task) {
            return Ok(task);
        }
        match client
            .report_progress(ReportProgressReq {
                envelope: envelope(&task),
                progress: Some(progress.clone()),
                message: Some(message.clone()),
            })
            .await
        {
            Ok(task) => return Ok(task),
            Err(error) if task_mgr_error_code(&error) == Some(TASK_ERR_REVISION_CONFLICT) => {
                continue
            }
            Err(error) => return Err(format!("report progress: {}", error)),
        }
    }
    Err("task progress is changing too quickly".to_string())
}

fn envelope(task: &Task) -> RunnerWriteEnvelope {
    RunnerWriteEnvelope {
        task_id: task.task_id.clone(),
        app_instance_id: None,
        runner_epoch: task.runner_epoch,
        expected_revision: task.revision,
    }
}

fn cancel_requested(task: &Task) -> bool {
    task.pending_control
        .as_ref()
        .is_some_and(|control| control.action == TaskControlAction::Cancel)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or(0)
}

fn downloads_dir(username: &str) -> PathBuf {
    get_buckyos_root_dir()
        .join("data")
        .join("home")
        .join(username)
        .join(DOWNLOAD_DIR)
}

fn partial_path(dir: &Path, task_id: &str) -> PathBuf {
    dir.join(format!(".download-{}.part", task_id))
}

async fn remove_partial(path: &Path) {
    if let Err(error) = tokio::fs::remove_file(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            log::warn!("remove partial download {}: {}", path.display(), error);
        }
    }
}

/// `name.ext`, or `name (1).ext`, `name (2).ext`, … when taken.
async fn unique_target(dir: &Path, filename: &str) -> PathBuf {
    let first = dir.join(filename);
    if !path_exists(&first).await {
        return first;
    }
    let (stem, ext) = split_extension(filename);
    for index in 1..10_000 {
        let candidate = dir.join(format!("{} ({}){}", stem, index, ext));
        if !path_exists(&candidate).await {
            return candidate;
        }
    }
    dir.join(format!("{} ({}){}", stem, now_ms(), ext))
}

async fn path_exists(path: &Path) -> bool {
    tokio::fs::symlink_metadata(path).await.is_ok()
}

/// (`report`, `.tar.gz`) keeps compound extensions together; a leading dot
/// (`.bashrc`) is part of the name.
fn split_extension(filename: &str) -> (&str, &str) {
    let search_from = filename.strip_prefix('.').map(|_| 1).unwrap_or(0);
    let lower = filename.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tar.xz") || lower.ends_with(".tar.bz2") {
        let index = lower[..lower.len() - 3]
            .rfind('.')
            .unwrap_or(filename.len());
        if index > search_from {
            return filename.split_at(index);
        }
    }
    match filename[search_from..].rfind('.') {
        Some(index) if index > 0 => filename.split_at(search_from + index),
        _ => (filename, ""),
    }
}

/// The user's directory name under `data/home`, when the name is safe as one.
fn home_segment(username: &str) -> Option<&str> {
    let name = username.trim();
    let safe = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', '\0'])
        && !name.starts_with('.');
    safe.then_some(name)
}

fn validate_url(value: &str) -> Result<String, RPCErrors> {
    let parsed = url::Url::parse(value.trim())
        .map_err(|error| RPCErrors::ParseRequestError(format!("invalid url: {}", error)))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(RPCErrors::ParseRequestError(
            "only http and https URLs can be downloaded".to_string(),
        ));
    }
    Ok(parsed.to_string())
}

/// A single path segment: no separators, control characters or dot names,
/// at most `MAX_FILENAME_BYTES`.
fn sanitize_filename(value: &str) -> Option<String> {
    let cleaned: String = value
        .trim()
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            ch if ch.is_control() => '_',
            ch => ch,
        })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim().to_string();
    if cleaned.is_empty() {
        return None;
    }
    if cleaned.len() <= MAX_FILENAME_BYTES {
        return Some(cleaned);
    }
    let (stem, ext) = split_extension(&cleaned);
    let ext = if ext.len() < 32 { ext } else { "" };
    let mut end = MAX_FILENAME_BYTES.saturating_sub(ext.len()).min(stem.len());
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!("{}{}", &stem[..end], ext))
}

fn filename_from_url(value: &str) -> Option<String> {
    let parsed = url::Url::parse(value).ok()?;
    let segment = parsed
        .path_segments()?
        .rev()
        .find(|segment| !segment.is_empty())?;
    let decoded = percent_decode(segment);
    sanitize_filename(&decoded)
}

/// `filename*=UTF-8''…` wins over `filename="…"` (RFC 6266).
fn filename_from_content_disposition(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers
        .get(reqwest::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    let mut plain = None;
    for part in value.split(';').map(str::trim) {
        let Some((key, raw)) = part.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let raw = raw.trim();
        if key == "filename*" {
            let encoded = raw.splitn(3, '\'').nth(2).unwrap_or(raw);
            if let Some(name) = sanitize_filename(&percent_decode(encoded)) {
                return Some(name);
            }
        } else if key == "filename" {
            plain = sanitize_filename(raw.trim_matches('"'));
        }
    }
    plain
}

fn percent_decode(value: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", value.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, decoded)| decoded.into_owned())
        .unwrap_or_else(|| value.to_string())
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} B", bytes)
    } else {
        format!("{:.1} {}", value, UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_are_single_safe_segments() {
        assert_eq!(
            sanitize_filename("report.pdf").as_deref(),
            Some("report.pdf")
        );
        assert_eq!(
            sanitize_filename("../etc/passwd").as_deref(),
            Some("_etc_passwd")
        );
        assert_eq!(sanitize_filename("  .hidden ").as_deref(), Some("hidden"));
        assert_eq!(sanitize_filename("a:b?c").as_deref(), Some("a_b_c"));
        assert_eq!(sanitize_filename(".."), None);
        assert_eq!(sanitize_filename("   "), None);
        let long = format!("{}.zip", "x".repeat(400));
        let cut = sanitize_filename(&long).unwrap();
        assert!(cut.len() <= MAX_FILENAME_BYTES && cut.ends_with(".zip"));
    }

    #[test]
    fn names_come_from_the_url_and_content_disposition() {
        assert_eq!(
            filename_from_url("https://example.com/files/My%20Report.pdf?x=1").as_deref(),
            Some("My Report.pdf")
        );
        assert_eq!(filename_from_url("https://example.com/"), None);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_DISPOSITION,
            "attachment; filename=\"plain.txt\"; filename*=UTF-8''%E6%8A%A5%E5%91%8A.txt"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            filename_from_content_disposition(&headers).as_deref(),
            Some("报告.txt")
        );
        headers.insert(
            reqwest::header::CONTENT_DISPOSITION,
            "attachment; filename=\"../plain.txt\"".parse().unwrap(),
        );
        assert_eq!(
            filename_from_content_disposition(&headers).as_deref(),
            Some("_plain.txt")
        );
    }

    #[test]
    fn extensions_split_for_numbered_copies() {
        assert_eq!(split_extension("a.txt"), ("a", ".txt"));
        assert_eq!(split_extension("backup.tar.gz"), ("backup", ".tar.gz"));
        assert_eq!(split_extension("README"), ("README", ""));
        assert_eq!(split_extension(".bashrc"), (".bashrc", ""));
    }

    #[test]
    fn only_web_urls_and_plain_user_names_are_accepted() {
        assert!(validate_url("https://example.com/a.zip").is_ok());
        assert!(validate_url("ftp://example.com/a.zip").is_err());
        assert!(validate_url("file:///etc/passwd").is_err());
        assert_eq!(home_segment("alice"), Some("alice"));
        assert_eq!(home_segment("../root"), None);
        assert_eq!(home_segment(".alice"), None);
        assert_eq!(home_segment(""), None);
    }

    #[tokio::test]
    async fn taken_names_get_a_number() {
        let dir = std::env::temp_dir().join(format!("cp-download-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(unique_target(&dir, "a.txt").await, dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"1").unwrap();
        std::fs::write(dir.join("a (1).txt"), b"2").unwrap();
        assert_eq!(unique_target(&dir, "a.txt").await, dir.join("a (2).txt"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
