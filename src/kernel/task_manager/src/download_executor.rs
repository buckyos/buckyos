use async_trait::async_trait;
use buckyos_api::{get_buckyos_api_runtime, AppDoc, Task, TaskStatus, TASK_MANAGER_SERVICE_NAME};
use buckyos_kit::get_buckyos_service_data_dir;
use lazy_static::lazy_static;
use log::{error, info, warn};
use named_store::NamedDataMgr;
use ndn_lib::{
    cyfs_get_obj_id_from_url, FileObject, NdnAction, NdnError, ObjId, ProgressCallbackResult,
    OBJ_TYPE_PKG,
};
use ndn_toolkit::cyfs_ndn_client::{CyfsNdnClient, CyfsPullResult};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{
    mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    Mutex, Semaphore,
};

pub const DOWNLOAD_TASK_TYPE: &str = "download";
const MAX_CONCURRENT_DOWNLOADS: usize = 1024;
const PROGRESS_REPORT_INTERVAL_MS: u64 = 250;

lazy_static! {
    static ref SHARED_DOWNLOAD_EXECUTOR: Arc<DownloadExecutor> =
        Arc::new(DownloadExecutor::new(MAX_CONCURRENT_DOWNLOADS));
}

#[async_trait]
pub trait DownloadTaskStore: Send + Sync + 'static {
    async fn load_task(&self, task_id: i64) -> std::result::Result<Task, String>;

    async fn update_task(
        &self,
        task_id: i64,
        status: Option<TaskStatus>,
        progress: Option<f32>,
        message: Option<String>,
        data_patch: Option<Value>,
        source_method: &'static str,
    ) -> std::result::Result<Task, String>;

    async fn mark_failed(
        &self,
        task_id: i64,
        error_message: String,
        source_method: &'static str,
    ) -> std::result::Result<Task, String>;
}

#[derive(Clone, Debug)]
pub struct DownloadTaskSpec {
    pub task_id: i64,
    pub download_url: String,
    pub objid: Option<ObjId>,
    pub download_options: Value,
}

struct ProgressState {
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    last_report_at_ms: u64,
}

pub struct DownloadExecutor {
    core: Arc<DownloadExecutorCore>,
    sender: UnboundedSender<DownloadJob>,
}

struct DownloadExecutorCore {
    limiter: Arc<Semaphore>,
    queued_tasks: Arc<Mutex<HashSet<i64>>>,
}

struct DownloadJob {
    core: Arc<DownloadExecutorCore>,
    store: Arc<dyn DownloadTaskStore>,
    spec: DownloadTaskSpec,
}

impl DownloadExecutor {
    pub fn new(max_concurrent_downloads: usize) -> Self {
        let core = Arc::new(DownloadExecutorCore {
            limiter: Arc::new(Semaphore::new(max_concurrent_downloads)),
            queued_tasks: Arc::new(Mutex::new(HashSet::new())),
        });
        let (sender, receiver) = unbounded_channel();
        start_download_worker(receiver);
        Self { core, sender }
    }

    pub async fn enqueue(
        self: &Arc<Self>,
        store: Arc<dyn DownloadTaskStore>,
        spec: DownloadTaskSpec,
    ) -> bool {
        let mut queued_tasks = self.core.queued_tasks.lock().await;
        if !queued_tasks.insert(spec.task_id) {
            return false;
        }
        drop(queued_tasks);

        let task_id = spec.task_id;
        let send_result = self.sender.send(DownloadJob {
            core: self.core.clone(),
            store,
            spec,
        });
        if let Err(err) = send_result {
            let mut queued_tasks = self.core.queued_tasks.lock().await;
            queued_tasks.remove(&task_id);
            error!(
                "download executor failed to enqueue task {}: {}",
                task_id, err
            );
            return false;
        }
        true
    }
}

impl DownloadExecutorCore {
    async fn run(self: Arc<Self>, store: Arc<dyn DownloadTaskStore>, spec: DownloadTaskSpec) {
        let task_id = spec.task_id;
        let _permit = match self.limiter.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(err) => {
                error!(
                    "download executor failed to acquire slot for task {}: {}",
                    task_id, err
                );
                self.finish(task_id).await;
                return;
            }
        };

        let run_result = self.run_inner(store.clone(), spec.clone()).await;
        if let Err(err) = run_result {
            if let Err(mark_err) = handle_run_error(store.clone(), task_id, err.clone()).await {
                error!(
                    "download executor failed to persist error for task {}: {} (original error: {})",
                    task_id, mark_err, err
                );
            }
        }

        self.finish(task_id).await;
    }

    async fn finish(&self, task_id: i64) {
        let mut queued_tasks = self.queued_tasks.lock().await;
        queued_tasks.remove(&task_id);
    }

    async fn run_inner(
        &self,
        store: Arc<dyn DownloadTaskStore>,
        spec: DownloadTaskSpec,
    ) -> std::result::Result<(), String> {
        let task = store.load_task(spec.task_id).await?;
        if task.status.is_terminal() || task.status == TaskStatus::Paused {
            info!(
                "download task skipped: task_id={} status={} url={}",
                spec.task_id, task.status, spec.download_url
            );
            return Ok(());
        }

        if task.status == TaskStatus::Canceled {
            info!(
                "download task canceled before start: task_id={} url={}",
                spec.task_id, spec.download_url
            );
            let _ = store
                .update_task(
                    spec.task_id,
                    Some(TaskStatus::Canceled),
                    None,
                    Some("Download canceled".to_string()),
                    Some(json!({ "download": { "state": "canceled" } })),
                    "download_executor_canceled",
                )
                .await;
            return Ok(());
        }

        let local_output_path = if spec.objid.is_none() {
            Some(resolve_local_output_path(
                spec.task_id,
                &spec.download_url,
                &spec.download_options,
            )?)
        } else {
            None
        };

        if let Some(local_output_path) = local_output_path.as_ref() {
            if let Some(parent) = local_output_path.parent() {
                tokio::fs::create_dir_all(parent).await.map_err(|err| {
                    format!(
                        "create download output dir {} failed: {}",
                        parent.display(),
                        err
                    )
                })?;
            }
        }

        store
            .update_task(
                spec.task_id,
                Some(TaskStatus::Running),
                Some(0.0),
                Some("Download started".to_string()),
                Some(json!({
                    "download": {
                        "state": "running",
                        "mode": download_mode(spec.objid.as_ref()),
                        "downloaded_bytes": 0u64,
                    }
                })),
                "download_executor_start",
            )
            .await?;
        info!(
            "download task started: task_id={} mode={} objid={:?} url={}",
            spec.task_id,
            download_mode(spec.objid.as_ref()),
            spec.objid,
            spec.download_url
        );

        let runtime = get_buckyos_api_runtime().map_err(|err| err.to_string())?;
        let session_token = runtime.get_session_token().await;
        let named_store = if spec.objid.is_some() {
            Some(
                runtime
                    .get_named_store()
                    .await
                    .map_err(|err| err.to_string())?,
            )
        } else {
            None
        };

        let client = build_ndn_client(
            session_token.as_str(),
            named_store.clone(),
            &spec.download_options,
        )?;

        let progress_state = Arc::new(Mutex::new(ProgressState {
            downloaded_bytes: 0,
            total_bytes: None,
            last_report_at_ms: 0,
        }));
        let progress_callback =
            build_progress_callback(store.clone(), spec.task_id, progress_state.clone());

        let result = if let Some(store_mgr) = named_store.as_ref() {
            let objid = spec
                .objid
                .clone()
                .ok_or_else(|| "named store download requires objid".to_string())?;
            pull_named_store_download(
                &client,
                store.clone(),
                spec.task_id,
                spec.download_url.as_str(),
                &objid,
                store_mgr,
                &spec.download_options,
                Some(progress_callback),
            )
            .await?
        } else {
            let mut request = client.get(spec.download_url.clone());
            request = request.progress_callback(progress_callback);
            let output_path = local_output_path
                .clone()
                .ok_or_else(|| "local output path is missing".to_string())?;
            request
                .pull_to_local_file(output_path)
                .await
                .map_err(|err| err.to_string())?
        };

        let completed_patch =
            build_completed_patch(&spec, &result, local_output_path.as_ref().cloned());
        store
            .update_task(
                spec.task_id,
                Some(TaskStatus::Completed),
                Some(100.0),
                Some("Download completed".to_string()),
                Some(completed_patch),
                "download_executor_complete",
            )
            .await?;

        info!(
            "download task completed: task_id={} objid={:?} url={}",
            spec.task_id, spec.objid, spec.download_url
        );
        Ok(())
    }
}

fn start_download_worker(mut receiver: UnboundedReceiver<DownloadJob>) {
    let builder = std::thread::Builder::new().name("task-download-executor".to_string());
    builder
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("download executor runtime init must succeed");
            let local_set = tokio::task::LocalSet::new();

            local_set.block_on(&runtime, async move {
                while let Some(job) = receiver.recv().await {
                    tokio::task::spawn_local(async move {
                        job.core.run(job.store, job.spec).await;
                    });
                }
            });
        })
        .expect("download executor worker thread must start");
}

pub fn shared_download_executor() -> Arc<DownloadExecutor> {
    Arc::clone(&SHARED_DOWNLOAD_EXECUTOR)
}

pub fn infer_objid_from_url(download_url: &str) -> Option<ObjId> {
    cyfs_get_obj_id_from_url(download_url)
        .ok()
        .map(|(objid, _)| objid)
}

pub fn build_download_task_name(download_url: &str, objid: Option<&ObjId>) -> String {
    if let Some(objid) = objid {
        return format!("download:objid:{}", objid);
    }

    let mut hasher = DefaultHasher::new();
    download_url.hash(&mut hasher);
    format!("download:url:{:016x}", hasher.finish())
}

pub fn build_download_task_data(
    download_url: &str,
    objid: Option<&ObjId>,
    download_options: Option<Value>,
) -> Value {
    let mut data = json!({
        "download_url": download_url,
        "urls": [download_url],
        "download": {
            "state": "pending",
            "mode": download_mode(objid),
            "downloaded_bytes": 0u64,
        }
    });

    if let Some(objid) = objid {
        data["objid"] = json!(objid.to_string());
    }
    if let Some(download_options) = download_options {
        if !download_options.is_null() {
            data["download_options"] = download_options;
        }
    }
    data
}

pub fn merge_download_source_patch(
    task_data: &Value,
    download_url: &str,
    objid: Option<&ObjId>,
    download_options: Option<&Value>,
) -> Option<Value> {
    let mut patch = serde_json::Map::new();
    let mut urls = extract_download_urls(task_data);
    let mut changed = false;

    if !urls.iter().any(|url| url == download_url) {
        urls.push(download_url.to_string());
        changed = true;
    }

    if changed {
        patch.insert("urls".to_string(), json!(urls));
    }

    if task_data
        .get("download_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        patch.insert("download_url".to_string(), json!(download_url));
    }

    if extract_task_objid(task_data).is_none() {
        if let Some(objid) = objid {
            patch.insert("objid".to_string(), json!(objid.to_string()));
        }
    }

    if task_data.get("download_options").is_none() {
        if let Some(download_options) = download_options {
            if !download_options.is_null() {
                patch.insert("download_options".to_string(), download_options.clone());
            }
        }
    }

    if task_data.pointer("/download/mode").is_none() {
        let mode_objid = objid.cloned().or_else(|| extract_task_objid(task_data));
        patch.insert(
            "download".to_string(),
            json!({
                "mode": download_mode(mode_objid.as_ref())
            }),
        );
    }

    if patch.is_empty() {
        None
    } else {
        Some(Value::Object(patch))
    }
}

pub fn task_has_objid(task: &Task, objid: &ObjId) -> bool {
    extract_task_objid(&task.data).as_ref() == Some(objid)
}

pub fn task_has_download_url(task: &Task, download_url: &str) -> bool {
    extract_download_urls(&task.data)
        .into_iter()
        .any(|url| url == download_url)
}

pub fn spec_from_task(task: &Task) -> Option<DownloadTaskSpec> {
    let download_url = task
        .data
        .get("download_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| extract_download_urls(&task.data).into_iter().next())?;

    Some(DownloadTaskSpec {
        task_id: task.id,
        objid: extract_task_objid(&task.data)
            .or_else(|| infer_objid_from_url(download_url.as_str())),
        download_url,
        download_options: task
            .data
            .get("download_options")
            .cloned()
            .unwrap_or_else(|| json!({})),
    })
}

pub fn should_enqueue_download_task(task: &Task) -> bool {
    matches!(task.status, TaskStatus::Pending | TaskStatus::Running)
}

fn extract_download_urls(task_data: &Value) -> Vec<String> {
    let mut urls = Vec::new();

    if let Some(download_url) = task_data
        .get("download_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        urls.push(download_url.to_string());
    }

    if let Some(items) = task_data.get("urls").and_then(|value| value.as_array()) {
        for item in items {
            if let Some(url) = item
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                if !urls.iter().any(|existing| existing == url) {
                    urls.push(url.to_string());
                }
            }
        }
    }

    urls
}

fn extract_task_objid(task_data: &Value) -> Option<ObjId> {
    for pointer in ["/objid", "/resolved_objid"] {
        if let Some(objid) = task_data
            .pointer(pointer)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .and_then(|value| ObjId::new(value).ok())
        {
            return Some(objid);
        }
    }
    None
}

fn download_mode(objid: Option<&ObjId>) -> &'static str {
    if objid.is_some() {
        "named_store"
    } else {
        "local_file"
    }
}

fn build_ndn_client(
    session_token: &str,
    named_store: Option<NamedDataMgr>,
    download_options: &Value,
) -> std::result::Result<CyfsNdnClient, String> {
    let mut builder = CyfsNdnClient::builder();

    if !session_token.trim().is_empty() {
        builder = builder.session_token(session_token.to_string());
    }
    if let Some(named_store) = named_store {
        builder = builder.default_store_mgr(named_store);
    }
    if let Some(default_remote_url) = download_options
        .get("default_remote_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        builder = builder.default_remote_url(default_remote_url.to_string());
    }
    if let Some(timeout_ms) = resolve_timeout_ms(download_options) {
        builder = builder.timeout(std::time::Duration::from_millis(timeout_ms));
    }
    if download_options
        .get("obj_id_in_host")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
    {
        builder = builder.obj_id_in_host(true);
    }

    builder.build().map_err(|err| err.to_string())
}

fn resolve_timeout_ms(download_options: &Value) -> Option<u64> {
    if let Some(timeout_ms) = download_options
        .get("timeout_ms")
        .and_then(|value| value.as_u64())
    {
        return Some(timeout_ms);
    }

    download_options
        .get("timeout_secs")
        .and_then(|value| value.as_u64())
        .map(|timeout_secs| timeout_secs.saturating_mul(1000))
}

fn resolve_local_output_path(
    task_id: i64,
    download_url: &str,
    download_options: &Value,
) -> std::result::Result<PathBuf, String> {
    if let Some(local_path) = download_options
        .get("local_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(PathBuf::from(local_path));
    }

    let base_dir = get_buckyos_service_data_dir(TASK_MANAGER_SERVICE_NAME)
        .join("downloads")
        .join(task_id.to_string());
    let filename = download_options
        .get("filename")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(sanitize_filename)
        .unwrap_or_else(|| derive_filename(download_url));

    Ok(base_dir.join(filename))
}

fn derive_filename(download_url: &str) -> String {
    let trimmed = download_url
        .split('?')
        .next()
        .unwrap_or(download_url)
        .split('#')
        .next()
        .unwrap_or(download_url);
    let path = if let Some(index) = trimmed.find("://") {
        let after_scheme = &trimmed[index + 3..];
        match after_scheme.find('/') {
            Some(path_index) => &after_scheme[path_index + 1..],
            None => "",
        }
    } else {
        trimmed
    };
    let candidate = path.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let sanitized = sanitize_filename(candidate);
    if sanitized.is_empty() {
        "download.bin".to_string()
    } else {
        sanitized
    }
}

fn sanitize_filename(raw: &str) -> String {
    let mut output = String::new();
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            output.push(ch);
        } else {
            output.push('_');
        }
    }

    let trimmed = output.trim_matches('_').trim_matches('.').to_string();
    if trimmed.is_empty() {
        "download.bin".to_string()
    } else if trimmed.len() > 120 {
        trimmed[..120].to_string()
    } else {
        trimmed
    }
}

fn build_progress_callback(
    store: Arc<dyn DownloadTaskStore>,
    task_id: i64,
    progress_state: Arc<Mutex<ProgressState>>,
) -> Arc<Mutex<ndn_lib::NdnProgressCallback>> {
    Arc::new(Mutex::new(Box::new(move |_inner_path, action| {
        let store = store.clone();
        let progress_state = progress_state.clone();
        Box::pin(async move {
            let mut progress_state = progress_state.lock().await;
            update_progress_state(&mut progress_state, &action);
            let now_ms = now_ms();
            let should_report = matches!(action, NdnAction::FileOK(_, _) | NdnAction::DirOK(_, _))
                || now_ms.saturating_sub(progress_state.last_report_at_ms)
                    >= PROGRESS_REPORT_INTERVAL_MS;

            if !should_report {
                return Ok(ProgressCallbackResult::Continue);
            }

            progress_state.last_report_at_ms = now_ms;
            let downloaded_bytes = progress_state.downloaded_bytes;
            let total_bytes = progress_state.total_bytes;
            drop(progress_state);

            let task = store.load_task(task_id).await.map_err(NdnError::Internal)?;
            if task.status == TaskStatus::Canceled {
                return Ok(ProgressCallbackResult::Stop);
            }

            let progress = total_bytes.map(|total| {
                if total == 0 {
                    0.0
                } else {
                    ((downloaded_bytes as f64 / total as f64) * 100.0).min(99.0) as f32
                }
            });
            let mut download_patch = json!({
                "downloaded_bytes": downloaded_bytes,
            });
            if let Some(total_bytes) = total_bytes {
                download_patch["total_bytes"] = json!(total_bytes);
            }

            store
                .update_task(
                    task_id,
                    None,
                    progress,
                    Some(progress_message(downloaded_bytes, total_bytes)),
                    Some(json!({ "download": download_patch })),
                    "download_executor_progress",
                )
                .await
                .map_err(NdnError::Internal)?;
            Ok(ProgressCallbackResult::Continue)
        })
    })))
}

fn update_progress_state(progress_state: &mut ProgressState, action: &NdnAction) {
    match action {
        NdnAction::ChunkOK(_, size) | NdnAction::Skip(size) => {
            progress_state.downloaded_bytes = progress_state.downloaded_bytes.saturating_add(*size);
        }
        NdnAction::FileOK(_, size) | NdnAction::DirOK(_, size) => {
            progress_state.downloaded_bytes = progress_state.downloaded_bytes.max(*size);
            progress_state.total_bytes = Some(*size);
        }
        NdnAction::PreFile | NdnAction::PreDir => {}
    }
}

fn progress_message(downloaded_bytes: u64, total_bytes: Option<u64>) -> String {
    if let Some(total_bytes) = total_bytes {
        return format!("Downloading {downloaded_bytes}/{total_bytes} bytes");
    }

    format!("Downloading {} bytes", downloaded_bytes)
}

#[derive(Clone)]
struct VerifiedJsonObject {
    obj_id: ObjId,
    obj_json: Value,
    obj_str: String,
}

#[derive(Clone)]
struct SubPkgDownloadSpec {
    key: String,
    download_url: String,
    objid: ObjId,
}

async fn pull_named_store_download(
    client: &CyfsNdnClient,
    store: Arc<dyn DownloadTaskStore>,
    task_id: i64,
    download_url: &str,
    objid: &ObjId,
    store_mgr: &NamedDataMgr,
    download_options: &Value,
    progress_callback: Option<Arc<Mutex<ndn_lib::NdnProgressCallback>>>,
) -> std::result::Result<CyfsPullResult, String> {
    if objid.is_chunk() || objid.is_chunk_list() || objid.is_file_object() {
        info!(
            "download task {} using direct named object path: objid={} url={}",
            task_id, objid, download_url
        );
        return pull_direct_to_named_store(
            client,
            download_url,
            objid,
            store_mgr,
            progress_callback,
        )
        .await;
    }

    let verified = fetch_verified_json_object(client, download_url, objid).await?;

    if let Ok(app_doc) = serde_json::from_value::<AppDoc>(verified.obj_json.clone()) {
        info!(
            "download task {} resolved AppDoc: objid={} url={}",
            task_id, objid, download_url
        );
        return pull_app_doc_to_named_store(
            client,
            store,
            task_id,
            download_url,
            verified,
            app_doc,
            store_mgr,
            download_options,
        )
        .await;
    }

    // `pkg` objects are PackageMeta. They flatten FileObject fields and should
    // be handled as metadata wrappers whose `content` points to the real payload.
    if objid.obj_type == OBJ_TYPE_PKG {
        info!(
            "download task {} resolved pkg metadata object: objid={} url={}",
            task_id, objid, download_url
        );
        let file_obj = serde_json::from_value::<FileObject>(verified.obj_json.clone())
            .map_err(|err| format!("parse pkg object {} as FileObject failed: {}", objid, err))?;
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
            progress_callback,
        )
        .await;
    }

    if let Ok(file_obj) = serde_json::from_value::<FileObject>(verified.obj_json.clone()) {
        info!(
            "download task {} resolved wrapped file object: objid={} url={}",
            task_id, objid, download_url
        );
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
            progress_callback,
        )
        .await;
    }

    warn!(
        "download task {} failed to resolve supported object type: objid={} url={}",
        task_id, objid, download_url
    );
    Err(format!(
        "unsupported obj type for download: {} ({})",
        objid.obj_type, objid
    ))
}

async fn pull_direct_to_named_store(
    client: &CyfsNdnClient,
    download_url: &str,
    objid: &ObjId,
    store_mgr: &NamedDataMgr,
    progress_callback: Option<Arc<Mutex<ndn_lib::NdnProgressCallback>>>,
) -> std::result::Result<CyfsPullResult, String> {
    info!(
        "direct named object download started: objid={} url={}",
        objid, download_url
    );
    let mut request = client.get(download_url.to_string()).obj_id(objid.clone());
    if let Some(progress_callback) = progress_callback {
        request = request.progress_callback(progress_callback);
    }
    request
        .pull_to_named_store(store_mgr)
        .await
        .map_err(|err| err.to_string())
}

async fn fetch_verified_json_object(
    client: &CyfsNdnClient,
    download_url: &str,
    objid: &ObjId,
) -> std::result::Result<VerifiedJsonObject, String> {
    let (real_obj_id, obj_str) = client
        .get(download_url.to_string())
        .obj_id(objid.clone())
        .send()
        .await
        .map_err(|err| err.to_string())?
        .object_string()
        .await
        .map_err(|err| err.to_string())?;

    let obj_json = serde_json::from_str::<Value>(obj_str.as_str()).map_err(|err| {
        format!(
            "parse object {} from {} as json failed: {}",
            real_obj_id, download_url, err
        )
    })?;

    Ok(VerifiedJsonObject {
        obj_id: real_obj_id,
        obj_json,
        obj_str,
    })
}

async fn pull_wrapped_file_object_to_named_store(
    client: &CyfsNdnClient,
    download_url: &str,
    verified: VerifiedJsonObject,
    file_obj: FileObject,
    store_mgr: &NamedDataMgr,
    download_options: &Value,
    progress_callback: Option<Arc<Mutex<ndn_lib::NdnProgressCallback>>>,
) -> std::result::Result<CyfsPullResult, String> {
    let content_objid = ObjId::new(file_obj.content.trim()).map_err(|err| {
        format!(
            "invalid wrapped file content obj id for {}: {}",
            verified.obj_id, err
        )
    })?;
    let content_download_url =
        resolve_related_download_url(download_url, &content_objid, download_options)?;
    info!(
        "wrapped metadata object resolved content: meta_objid={} content_objid={} url={} content_url={}",
        verified.obj_id, content_objid, download_url, content_download_url
    );

    let mut result = pull_direct_to_named_store(
        client,
        content_download_url.as_str(),
        &content_objid,
        store_mgr,
        progress_callback,
    )
    .await?;

    store_mgr
        .put_object(&verified.obj_id, verified.obj_str.as_str())
        .await
        .map_err(|err| err.to_string())?;
    info!(
        "wrapped metadata object stored: meta_objid={} content_objid={}",
        verified.obj_id, content_objid
    );
    push_stored_object(&mut result.stored_objects, verified.obj_id.clone());
    result.obj_id = Some(verified.obj_id);
    if file_obj.size > 0 {
        result.total_size = file_obj.size.max(result.total_size);
    }

    Ok(result)
}

async fn pull_sub_pkg_to_named_store(
    client: &CyfsNdnClient,
    download_url: &str,
    objid: &ObjId,
    store_mgr: &NamedDataMgr,
    download_options: &Value,
) -> std::result::Result<CyfsPullResult, String> {
    if objid.is_chunk() || objid.is_chunk_list() || objid.is_file_object() {
        info!(
            "AppDoc sub package using direct path: objid={} url={}",
            objid, download_url
        );
        return pull_direct_to_named_store(client, download_url, objid, store_mgr, None).await;
    }

    let verified = fetch_verified_json_object(client, download_url, objid).await?;

    if serde_json::from_value::<AppDoc>(verified.obj_json.clone()).is_ok() {
        warn!(
            "nested AppDoc sub package is unsupported: objid={} url={}",
            objid, download_url
        );
        return Err(format!(
            "nested AppDoc sub package is not supported for {}",
            objid
        ));
    }

    if objid.obj_type == OBJ_TYPE_PKG {
        info!(
            "AppDoc sub package resolved pkg metadata: objid={} url={}",
            objid, download_url
        );
        let file_obj = serde_json::from_value::<FileObject>(verified.obj_json.clone())
            .map_err(|err| format!("parse pkg object {} as FileObject failed: {}", objid, err))?;
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
            None,
        )
        .await;
    }

    if let Ok(file_obj) = serde_json::from_value::<FileObject>(verified.obj_json.clone()) {
        info!(
            "AppDoc sub package resolved wrapped file object: objid={} url={}",
            objid, download_url
        );
        return pull_wrapped_file_object_to_named_store(
            client,
            download_url,
            verified,
            file_obj,
            store_mgr,
            download_options,
            None,
        )
        .await;
    }

    warn!(
        "unsupported AppDoc sub package object type: objid={} url={}",
        objid, download_url
    );
    Err(format!(
        "unsupported sub package obj type for download: {} ({})",
        objid.obj_type, objid
    ))
}

async fn pull_app_doc_to_named_store(
    client: &CyfsNdnClient,
    store: Arc<dyn DownloadTaskStore>,
    task_id: i64,
    download_url: &str,
    verified: VerifiedJsonObject,
    app_doc: AppDoc,
    store_mgr: &NamedDataMgr,
    download_options: &Value,
) -> std::result::Result<CyfsPullResult, String> {
    ensure_task_can_continue(store.clone(), task_id).await?;

    store_mgr
        .put_object(&verified.obj_id, verified.obj_str.as_str())
        .await
        .map_err(|err| err.to_string())?;

    let sub_pkgs = resolve_app_doc_sub_pkg_specs(&app_doc, download_url, download_options)?;
    let total_sub_pkgs = sub_pkgs.len();
    info!(
        "AppDoc download resolved: objid={} url={} sub_pkg_count={}",
        verified.obj_id, download_url, total_sub_pkgs
    );

    let mut result = CyfsPullResult {
        obj_id: Some(verified.obj_id.clone()),
        total_size: 0,
        chunk_count: 0,
        stored_objects: vec![verified.obj_id.clone()],
    };

    if total_sub_pkgs == 0 {
        info!(
            "AppDoc download completed without sub packages: objid={} url={}",
            verified.obj_id, download_url
        );
        return Ok(result);
    }

    for (index, sub_pkg) in sub_pkgs.iter().enumerate() {
        ensure_task_can_continue(store.clone(), task_id).await?;
        info!(
            "AppDoc sub package download started: task_id={} appdoc_objid={} index={}/{} key={} objid={} url={}",
            task_id,
            verified.obj_id,
            index + 1,
            total_sub_pkgs,
            sub_pkg.key,
            sub_pkg.objid,
            sub_pkg.download_url
        );
        update_app_doc_progress(
            store.clone(),
            task_id,
            sub_pkg.key.as_str(),
            index,
            total_sub_pkgs,
            result.total_size,
        )
        .await?;

        let sub_result = pull_sub_pkg_to_named_store(
            client,
            sub_pkg.download_url.as_str(),
            &sub_pkg.objid,
            store_mgr,
            download_options,
        )
        .await?;

        merge_pull_result(&mut result, sub_result);
        info!(
            "AppDoc sub package download completed: task_id={} appdoc_objid={} index={}/{} key={} total_downloaded_bytes={}",
            task_id,
            verified.obj_id,
            index + 1,
            total_sub_pkgs,
            sub_pkg.key,
            result.total_size
        );
        update_app_doc_progress(
            store.clone(),
            task_id,
            sub_pkg.key.as_str(),
            index + 1,
            total_sub_pkgs,
            result.total_size,
        )
        .await?;
    }

    result.obj_id = Some(verified.obj_id);
    Ok(result)
}

async fn ensure_task_can_continue(
    store: Arc<dyn DownloadTaskStore>,
    task_id: i64,
) -> std::result::Result<(), String> {
    let task = store.load_task(task_id).await?;
    if task.status == TaskStatus::Canceled {
        info!("download task observed canceled state: task_id={}", task_id);
        return Err("Download canceled".to_string());
    }
    if task.status == TaskStatus::Paused {
        info!("download task observed paused state: task_id={}", task_id);
        return Err("Download paused".to_string());
    }
    Ok(())
}

async fn update_app_doc_progress(
    store: Arc<dyn DownloadTaskStore>,
    task_id: i64,
    current_sub_pkg: &str,
    completed_sub_pkgs: usize,
    total_sub_pkgs: usize,
    downloaded_bytes: u64,
) -> std::result::Result<(), String> {
    let progress = if total_sub_pkgs == 0 {
        None
    } else {
        Some(((completed_sub_pkgs as f32 / total_sub_pkgs as f32) * 99.0).min(99.0))
    };

    store
        .update_task(
            task_id,
            None,
            progress,
            Some(format!(
                "Downloading sub package {}/{}: {}",
                completed_sub_pkgs.min(total_sub_pkgs),
                total_sub_pkgs,
                current_sub_pkg
            )),
            Some(json!({
                "download": {
                    "downloaded_bytes": downloaded_bytes,
                    "sub_pkg_total": total_sub_pkgs,
                    "sub_pkg_completed": completed_sub_pkgs,
                    "current_sub_pkg": current_sub_pkg,
                }
            })),
            "download_executor_app_doc_progress",
        )
        .await
        .map(|_| ())
}

fn resolve_app_doc_sub_pkg_specs(
    app_doc: &AppDoc,
    app_doc_url: &str,
    download_options: &Value,
) -> std::result::Result<Vec<SubPkgDownloadSpec>, String> {
    let mut sub_pkgs = Vec::new();

    for (key, sub_pkg) in app_doc.pkg_list.iter() {
        let download_url = sub_pkg
            .source_url
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .or_else(|| {
                sub_pkg.pkg_objid.as_ref().and_then(|objid| {
                    resolve_related_download_url(app_doc_url, objid, download_options).ok()
                })
            })
            .or_else(|| {
                sub_pkg
                    .pkg_objid
                    .as_ref()
                    .map(|objid| format!("cyfs://{}", objid))
            })
            .ok_or_else(|| {
                warn!(
                    "AppDoc sub package missing source_url and pkg_objid: key={}",
                    key
                );
                format!(
                    "AppDoc sub package `{}` missing source_url and pkg_objid",
                    key
                )
            })?;

        let objid = sub_pkg
            .pkg_objid
            .clone()
            .or_else(|| infer_objid_from_url(download_url.as_str()))
            .ok_or_else(|| {
                warn!(
                    "AppDoc sub package objid is not resolvable: key={} url={}",
                    key, download_url
                );
                format!(
                    "AppDoc sub package `{}` does not provide a resolvable objid",
                    key
                )
            })?;

        sub_pkgs.push(SubPkgDownloadSpec {
            key,
            download_url,
            objid,
        });
    }

    Ok(sub_pkgs)
}

fn resolve_related_download_url(
    base_url: &str,
    objid: &ObjId,
    download_options: &Value,
) -> std::result::Result<String, String> {
    replace_obj_id_in_url(base_url, objid).or_else(|_| {
        build_download_url_from_default_remote(objid, download_options).ok_or_else(|| {
            warn!(
                "failed to resolve related download url: base_url={} objid={}",
                base_url, objid
            );
            format!(
                "cannot resolve related download url for {} from {}",
                objid, base_url
            )
        })
    })
}

fn replace_obj_id_in_url(base_url: &str, objid: &ObjId) -> std::result::Result<String, String> {
    let parsed_url = url::Url::parse(base_url)
        .map_err(|err| format!("parse url {} failed: {}", base_url, err))?;
    let (base_objid, _) =
        cyfs_get_obj_id_from_url(base_url).map_err(|err| format!("parse objid failed: {}", err))?;
    let mut replaced_url = parsed_url.clone();

    if parsed_url
        .host_str()
        .and_then(|host| ObjId::from_hostname(host).ok())
        .as_ref()
        == Some(&base_objid)
    {
        let host = parsed_url
            .host_str()
            .ok_or_else(|| format!("missing host in {}", base_url))?;
        let mut host_parts = host.split('.').map(str::to_string).collect::<Vec<String>>();
        if host_parts.is_empty() {
            return Err(format!("invalid host {} in {}", host, base_url));
        }
        host_parts[0] = objid.to_base32();
        replaced_url
            .set_host(Some(host_parts.join(".").as_str()))
            .map_err(|_| format!("replace host failed for {}", base_url))?;
        return Ok(replaced_url.to_string());
    }

    let segments = parsed_url
        .path_segments()
        .map(|segments| segments.map(str::to_string).collect::<Vec<String>>())
        .unwrap_or_default();
    for (index, segment) in segments.iter().enumerate() {
        if ObjId::new(segment).ok().as_ref() == Some(&base_objid) {
            let mut new_segments = segments.clone();
            new_segments[index] = objid.to_string();
            replaced_url.set_path(format!("/{}", new_segments.join("/")).as_str());
            return Ok(replaced_url.to_string());
        }
    }

    Err(format!("cannot replace objid in {}", base_url))
}

fn build_download_url_from_default_remote(
    objid: &ObjId,
    download_options: &Value,
) -> Option<String> {
    let default_remote_url = download_options
        .get("default_remote_url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let obj_id_in_host = download_options
        .get("obj_id_in_host")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    let parsed = url::Url::parse(default_remote_url).ok()?;
    if obj_id_in_host {
        let host = parsed.host_str()?;
        let mut replaced = parsed.clone();
        replaced
            .set_host(Some(format!("{}.{}", objid.to_base32(), host).as_str()))
            .ok()?;
        return Some(replaced.to_string());
    }

    Some(format!(
        "{}/{}",
        default_remote_url.trim_end_matches('/'),
        objid
    ))
}

fn merge_pull_result(total: &mut CyfsPullResult, next: CyfsPullResult) {
    total.total_size = total.total_size.saturating_add(next.total_size);
    total.chunk_count = total.chunk_count.saturating_add(next.chunk_count);
    if total.obj_id.is_none() {
        total.obj_id = next.obj_id;
    }
    for stored_object in next.stored_objects {
        push_stored_object(&mut total.stored_objects, stored_object);
    }
}

fn push_stored_object(stored_objects: &mut Vec<ObjId>, objid: ObjId) {
    if !stored_objects.iter().any(|existing| existing == &objid) {
        stored_objects.push(objid);
    }
}

fn build_completed_patch(
    spec: &DownloadTaskSpec,
    result: &CyfsPullResult,
    local_output_path: Option<PathBuf>,
) -> Value {
    let resolved_objid = result.obj_id.clone().or_else(|| spec.objid.clone());
    let mut patch = json!({
        "download": {
            "state": "completed",
            "mode": download_mode(spec.objid.as_ref()),
            "downloaded_bytes": result.total_size,
            "total_bytes": result.total_size,
            "chunk_count": result.chunk_count,
            "stored_objects": result
                .stored_objects
                .iter()
                .map(|objid| objid.to_string())
                .collect::<Vec<_>>(),
            "completed_at": now_secs(),
        }
    });

    if let Some(resolved_objid) = resolved_objid {
        patch["resolved_objid"] = json!(resolved_objid.to_string());
    }
    if let Some(local_output_path) = local_output_path {
        patch["local_path"] = json!(local_output_path.to_string_lossy().to_string());
    }

    patch
}

async fn handle_run_error(
    store: Arc<dyn DownloadTaskStore>,
    task_id: i64,
    err: String,
) -> std::result::Result<(), String> {
    match store.load_task(task_id).await {
        Ok(task) if task.status == TaskStatus::Canceled => {
            let _ = store
                .update_task(
                    task_id,
                    Some(TaskStatus::Canceled),
                    None,
                    Some("Download canceled".to_string()),
                    Some(json!({ "download": { "state": "canceled" } })),
                    "download_executor_canceled",
                )
                .await?;
            Ok(())
        }
        Ok(_) => {
            store
                .mark_failed(task_id, err.clone(), "download_executor_failed")
                .await?;
            warn!("download task failed: task_id={} err={}", task_id, err);
            Ok(())
        }
        Err(load_err) => Err(format!(
            "load task {} after download failure failed: {}",
            task_id, load_err
        )),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_download_source_patch_appends_unique_url() {
        let task_data = json!({
            "download_url": "https://example.com/a",
            "urls": ["https://example.com/a"],
        });

        let patch = merge_download_source_patch(
            &task_data,
            "https://example.com/b",
            None,
            Some(&json!({"timeout_ms": 1000})),
        )
        .unwrap();

        assert_eq!(
            patch
                .get("urls")
                .and_then(|value| value.as_array())
                .unwrap()
                .len(),
            2
        );
        assert!(patch.get("download_options").is_some());
    }

    #[test]
    fn derive_filename_falls_back_to_default() {
        assert_eq!(derive_filename("https://example.com/"), "download.bin");
        assert_eq!(
            derive_filename("https://example.com/a%20b.txt?x=1"),
            "a_20b.txt"
        );
    }

    #[test]
    fn replace_obj_id_in_url_rewrites_path_segment() {
        let base_url = "https://example.com/pkg:00112233/meta.json";
        let target = ObjId::new("pkg:aabbccdd").unwrap();

        let resolved = replace_obj_id_in_url(base_url, &target).unwrap();

        assert_eq!(resolved, "https://example.com/pkg:aabbccdd/meta.json");
    }

    #[test]
    fn resolve_app_doc_sub_pkg_specs_prefers_source_url_and_falls_back_to_objid() {
        let app_doc: AppDoc = serde_json::from_value(json!({
            "name": "demo",
            "version": "1.0.0",
            "show_name": "demo",
            "author": "demo",
            "owner": "did:web:demo.example",
            "create_time": 1,
            "last_update_time": 1,
            "exp": 999999,
            "selector_type": "single",
            "install_config_tips": {},
            "pkg_list": {
                "web": {
                    "pkg_id": "demo-web#1.0.0",
                    "pkg_objid": "pkg:aabbccdd",
                    "source_url": "https://download.example.com/demo-web"
                },
                "agent": {
                    "pkg_id": "demo-agent#1.0.0",
                    "pkg_objid": "pkg:11223344"
                }
            }
        }))
        .unwrap();

        let specs = resolve_app_doc_sub_pkg_specs(
            &app_doc,
            "https://repo.example.com/pkg:55667788/appdoc.json",
            &json!({}),
        )
        .unwrap();

        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].key, "web");
        assert_eq!(
            specs[0].download_url,
            "https://download.example.com/demo-web"
        );
        assert_eq!(specs[1].key, "agent");
        assert_eq!(
            specs[1].download_url,
            "https://repo.example.com/pkg:11223344/appdoc.json"
        );
    }
}
