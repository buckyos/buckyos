use crate::{task_mgr, CheckPointVersion, CheckPointVersionStrategy, ChunkId, ChunkInfo, ChunkMgr, ChunkMgrSelector, ChunkMgrServer, ChunkMgrServerSelector, ChunkServerType, FileId, FileInfo, FileMgr, FileMgrSelector, FileMgrServer, FileMgrServerSelector, FileServerType, ListOffset, TaskId, TaskInfo, TaskKey, TaskMgr, TaskMgrSelector, TaskMgrServer, TaskServerType};
use std::sync::Arc;
use warp::{Filter, Reply};
use serde::{Serialize, Deserialize};
use reqwest::Client;
use serde_json::json;
use std::error::Error;
use std::str::FromStr;

#[derive(Debug)]
pub struct SimpleServerError {
    state_code: u16,
    message: String,
}

impl SimpleServerError {
    pub fn new(state_code: u16, message: String) -> Self {
        SimpleServerError { state_code, message }
    }
}

impl warp::reject::Reject for SimpleServerError {}

impl std::error::Error for SimpleServerError {}

impl std::fmt::Display for SimpleServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCheckPointStrategyRequest {
    zone_id: String,
    task_key: TaskKey,
    strategy: CheckPointVersionStrategy,
}

#[derive(Debug, Serialize, Deserialize)]
struct UpdateCheckPointStrategyResponse;

#[derive(Debug, Serialize, Deserialize)]
struct GetCheckPointStrategyRequest {
    zone_id: String,
    task_key: TaskKey,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetCheckPointStrategyResponse {
    strategy: CheckPointVersionStrategy,
}

#[derive(Debug, Serialize, Deserialize)]
struct PushTaskInfoRequest {
    zone_id: String,
    task_key: TaskKey,
    check_point_version: CheckPointVersion,
    prev_check_point_version: Option<CheckPointVersion>,
    meta: Option<String>,
    dir_path: std::path::PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct PushTaskInfoResponse {
    task_id: TaskId,
}

#[derive(Debug, Serialize, Deserialize)]
struct AddFileInTaskRequest {
    task_id: TaskId,
    file_seq: u64,
    file_path: std::path::PathBuf,
    hash: String,
    file_size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct AddFileInTaskResponse {
    file_server_type: FileServerType,
    file_server_name: String,
    file_id: FileId,
    chunk_size: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct SetFilesPrepareReadyRequest {
    task_id: TaskId,
}

#[derive(Debug, Serialize, Deserialize)]
struct SetFilesPrepareReadyResponse;

#[derive(Debug, Serialize, Deserialize)]
struct SetFileUploadedRequest {
    task_id: TaskId,
    file_path: std::path::PathBuf,
}

#[derive(Debug, Serialize)]
struct SetFileUploadedResponse;

#[derive(Debug, Serialize, Deserialize)]
struct GetCheckPointVersionListRequest {
    zone_id: String,
    task_key: TaskKey,
    offset: ListOffset,
    limit: u32,
    is_restorable_only: bool,
}

#[derive(Serialize, Deserialize)]
struct GetCheckPointVersionListResponse {
    task_info_list: Vec<TaskInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetCheckPointVersionRequest {
    zone_id: String,
    task_key: TaskKey,
    check_point_version: CheckPointVersion,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetCheckPointVersionResponse {
    task_info: Option<TaskInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetFileInfoRequest {
    zone_id: String,
    task_id: TaskId,
    file_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetFileInfoResponse {
    file_info: Option<FileInfo>,
}

#[derive(Clone)]
pub struct TaskMgrHttpServer {
    task_mgr: Arc<Box<dyn TaskMgrServer>>,
}

impl TaskMgrHttpServer {
    pub fn new(task_mgr: Box<dyn TaskMgrServer>) -> Self {
        TaskMgrHttpServer { task_mgr: Arc::new(task_mgr) }
    }

    async fn update_check_point_strategy_handler(
        request: UpdateCheckPointStrategyRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let zone_id = request.zone_id;
        let task_key = request.task_key;
        let strategy = request.strategy;

        task_mgr
            .update_check_point_strategy(&zone_id, &task_key, strategy)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        Ok(warp::reply::json(&UpdateCheckPointStrategyResponse))
    }

    async fn get_check_point_strategy_handler(
        request: GetCheckPointStrategyRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let zone_id = request.zone_id;
        let task_key = request.task_key;

        let strategy = task_mgr
            .get_check_point_strategy(&zone_id, &task_key)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        let response = GetCheckPointStrategyResponse { strategy };

        Ok(warp::reply::json(&response))
    }

    async fn push_task_info_handler(
        request: PushTaskInfoRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let zone_id = request.zone_id;
        let task_key = request.task_key;
        let check_point_version = request.check_point_version;
        let prev_check_point_version = request.prev_check_point_version;
        let meta = request.meta;

        let task_id = task_mgr
            .push_task_info(
                &zone_id,
                &task_key,
                check_point_version,
                prev_check_point_version,
                meta.as_deref(),
                request.dir_path.as_path(),
            )
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        let response = PushTaskInfoResponse { task_id };

        Ok(warp::reply::json(&response))
    }

    async fn add_file_handler(
        request: AddFileInTaskRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let task_id = request.task_id;
        let file_seq = request.file_seq;
        let hash = request.hash;
        let file_size = request.file_size;

        let response = task_mgr
            .add_file(task_id, file_seq, request.file_path.as_path(), hash.as_str(), file_size)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        let add_file_response = AddFileInTaskResponse {
            file_server_type: response.0,
            file_server_name: response.1,
            file_id: response.2,
            chunk_size: response.3,
        };

        Ok(warp::reply::json(&add_file_response))
    }

    async fn set_files_prepare_ready_handler(
        request: SetFilesPrepareReadyRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let task_id = request.task_id;
        task_mgr
            .set_files_prepare_ready(task_id)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        Ok(warp::reply::json(&SetFilesPrepareReadyResponse))
    }

    async fn set_file_uploaded_handler(
        request: SetFileUploadedRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let task_id = request.task_id;
        task_mgr
            .set_file_uploaded(task_id, request.file_path.as_path())
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        Ok(warp::reply::json(&SetFileUploadedResponse))
    }

    async fn get_check_point_version_list_handler(
        request: GetCheckPointVersionListRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let zone_id = request.zone_id;
        let task_key = request.task_key;
        let offset = request.offset;
        let limit = request.limit;
        let is_restorable_only = request.is_restorable_only;
        let task_info_list = task_mgr
            .get_check_point_version_list(&zone_id, &task_key, offset, limit, is_restorable_only)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;
        let response = GetCheckPointVersionListResponse { task_info_list };
        Ok(warp::reply::json(&response))
    }

    async fn get_check_point_version_handler(
        request: GetCheckPointVersionRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let zone_id = request.zone_id;
        let task_key = request.task_key;
        let check_point_version = request.check_point_version;
        let task_info = task_mgr
            .get_check_point_version(&zone_id, &task_key, check_point_version)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;
        let response = GetCheckPointVersionResponse { task_info };
        Ok(warp::reply::json(&response))
    }

    async fn get_file_info_handler(
        request: GetFileInfoRequest,
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> Result<impl Reply, warp::Rejection> {
        let zone_id = request.zone_id;
        let task_id = request.task_id;
        let file_seq = request.file_seq;
        let file_info = task_mgr
            .get_file_info(&zone_id, task_id, file_seq)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;
        let response = GetFileInfoResponse { file_info };
        Ok(warp::reply::json(&response))
    }

    // Implement the remaining methods in a similar fashion
    // ...

        
    pub fn routes(
        task_mgr: Arc<Box<dyn TaskMgrServer>>,
    ) -> impl Filter<Extract = impl Reply, Error = warp::Rejection> + Clone {
        let update_check_point_strategy_route = {
            let task_mgr = task_mgr.clone();
            warp::path!("update_check_point_strategy")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: UpdateCheckPointStrategyRequest| {
                Self::update_check_point_strategy_handler(request, task_mgr.clone())
            })
        };

        let get_check_point_strategy_route = {
            let task_mgr = task_mgr.clone();warp::path!("get_check_point_strategy")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: GetCheckPointStrategyRequest| {
                Self::get_check_point_strategy_handler(request, task_mgr.clone())
            })
        };

        let push_task_info_route = {
            let task_mgr = task_mgr.clone();warp::path!("push_task_info")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: PushTaskInfoRequest| {
                Self::push_task_info_handler(request, task_mgr.clone())
            })
        };
        let add_file_route = {
            let task_mgr = task_mgr.clone();
            warp::path!("add_file")
                .and(warp::post())
                .and(warp::body::json())
                .and_then(move |request: AddFileInTaskRequest| {
                    Self::add_file_handler(request, task_mgr.clone())
                })
        };

        let set_files_prepare_ready_route = {
            let task_mgr = task_mgr.clone();
            warp::path!("set_files_prepare_ready")
                .and(warp::post())
                .and(warp::body::json())
                .and_then(move |request: SetFilesPrepareReadyRequest| {
                    Self::set_files_prepare_ready_handler(request, task_mgr.clone())
                })
        };

        let set_file_uploaded_route = {
            let task_mgr = task_mgr.clone();
            warp::path!("set_file_uploaded")
                .and(warp::post())
                .and(warp::body::json())
                .and_then(move |request: SetFileUploadedRequest| {
                    Self::set_file_uploaded_handler(request, task_mgr.clone())
                })
        };

        let get_check_point_version_list_route = {
            let task_mgr = task_mgr.clone();
            warp::path!("get_check_point_version_list")
                .and(warp::post())
                .and(warp::body::json())
                .and_then(move |request: GetCheckPointVersionListRequest| {
                    Self::get_check_point_version_list_handler(request, task_mgr.clone())
                })
        };

        let get_check_point_version_route = {
            let task_mgr = task_mgr.clone();
            warp::path!("get_check_point_version")
                .and(warp::post())
                .and(warp::body::json())
                .and_then(move |request: GetCheckPointVersionRequest| {
                    Self::get_check_point_version_handler(request, task_mgr.clone())
                })
        };

        let get_file_info_route = {
            let task_mgr = task_mgr.clone();
            warp::path!("get_file_info")
                .and(warp::post())
                .and(warp::body::json())
                .and_then(move |request: GetFileInfoRequest| {
                    Self::get_file_info_handler(request, task_mgr.clone())
                })
        };

        warp::path!("task-mgr").and(update_check_point_strategy_route
            .or(get_check_point_strategy_route)
            .or(push_task_info_route)
            .or(add_file_route)
            .or(set_files_prepare_ready_route)
            .or(set_file_uploaded_route)
            .or(get_check_point_version_list_route)
            .or(get_check_point_version_route)
            .or(get_file_info_route)
            // Combine with routes for the remaining methods
            // ...
        )
    }

}

pub struct TaskMgrHttpClient {
    base_url: String,
}

impl TaskMgrHttpClient {
    pub fn new(base_url: &str) -> Self {
        TaskMgrHttpClient {
            base_url: base_url.to_string(),
        }
    }
}


#[async_trait::async_trait]
impl TaskMgr for TaskMgrHttpClient {
    fn server_type(&self) -> TaskServerType {
        TaskServerType::Http
    }

    fn server_name(&self) -> &str {
        self.base_url.as_str()
    }

    async fn update_check_point_strategy(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        strategy: CheckPointVersionStrategy,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let request = UpdateCheckPointStrategyRequest {
            zone_id: zone_id.to_string(),
            task_key: task_key.clone(),
            strategy,
        };

        let url = format!("{}/task-mgr/update-check-point-strategy", self.base_url);
        let client = Client::new();
        let response = client
            .post(url.as_str())
            .json(&request)
            .send()
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await?;
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn get_check_point_strategy(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
    ) -> Result<CheckPointVersionStrategy, Box<dyn Error + Send + Sync>> {
        let request = GetCheckPointStrategyRequest {
            zone_id: zone_id.to_string(),
            task_key: task_key.clone(),
        };
        let url = format!("{}/task-mgr/get-check-point-strategy", self.base_url);
        let client = Client::new();
        let response = client
            .post(url.as_str())
            .json(&request)
            .send()
            .await?;
        if response.status().is_success() {
            let response_body: GetCheckPointStrategyResponse = response.json().await?;
            Ok(response_body.strategy)
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await?;
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn push_task_info(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        prev_check_point_version: Option<CheckPointVersion>,
        meta: Option<&str>,
        dir_path: &std::path::Path,
    ) -> Result<TaskId, Box<dyn Error + Send + Sync>> {
        let request = PushTaskInfoRequest {
            zone_id: zone_id.to_string(),
            task_key: task_key.clone(),
            check_point_version,
            prev_check_point_version,
            meta: meta.map(|s| s.to_string()),
            dir_path: std::path::PathBuf::from(dir_path),
        };
        let url = format!("{}/task-mgr/push-task-info", self.base_url);
        let client = Client::new();
        let response = client
            .post(url.as_str())
            .json(&request)
            .send()
            .await?;
        if response.status().is_success() {
            let response_body: PushTaskInfoResponse = response.json().await?;
            Ok(response_body.task_id)
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await?;
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn add_file(
        &self,
        task_id: TaskId,
        file_seq: u64,
        file_path: &std::path::Path,
        hash: &str,
        file_size: u64,
    ) -> Result<(FileServerType, String, FileId, u32), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/task-mgr/add-file", self.base_url);
        let request_body = AddFileInTaskRequest {
            task_id,
            file_seq,
            file_path: std::path::PathBuf::from(file_path),
            hash: hash.to_string(),
            file_size,
        };
        let client = Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;

        if response.status().is_success() {
            let add_file_response = response
                .json::<AddFileInTaskResponse>()
                .await
                .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;
            Ok((add_file_response.file_server_type, add_file_response.file_server_name, add_file_response.file_id, add_file_response.chunk_size))
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await.unwrap_or_else(|_| "".to_string());
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn set_files_prepare_ready(
        &self,
        task_id: TaskId,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/task-mgr/set-files-prepare-ready", self.base_url);
        let request_body = SetFilesPrepareReadyRequest { task_id };
        let client = Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;

        if response.status().is_success() {
            Ok(())
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await.unwrap_or_else(|_| "".to_string());
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn set_file_uploaded(
        &self,
        task_id: TaskId,
        file_path: &std::path::Path,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let url = format!("{}/task-mgr/set-file-uploaded", self.base_url);
        let request_body = SetFileUploadedRequest {
            task_id,
            file_path: std::path::PathBuf::from(file_path),
        };
        let client = Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;

        if response.status().is_success() {
            Ok(())
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await.unwrap_or_else(|_| "".to_string());
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn get_check_point_version_list(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        offset: ListOffset,
        limit: u32,
        is_restorable_only: bool,
    ) -> Result<Vec<TaskInfo>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/task-mgr/get-check-point-version-list", self.base_url);
        let request_body = GetCheckPointVersionListRequest {
            zone_id: zone_id.to_string(),
            task_key: task_key.clone(),
            offset,
            limit,
            is_restorable_only,
        };
        let client = Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;

        if response.status().is_success() {
            let response = response
                .json::<GetCheckPointVersionListResponse>()
                .await
                .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;
            Ok(response.task_info_list)
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await.unwrap_or_else(|_| "".to_string());
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn get_check_point_version(
        &self,
        zone_id: &str,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
    ) -> Result<Option<TaskInfo>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/task-mgr/get-check-point-version", self.base_url);
        let request_body = GetCheckPointVersionRequest {
            zone_id: zone_id.to_string(),
            task_key: task_key.clone(),
            check_point_version,
        };
        let client = Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;

        if response.status().is_success() {
            let response = response
                .json::<GetCheckPointVersionResponse>()
                .await
                .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;
            Ok(response.task_info)
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await.unwrap_or_else(|_| "".to_string());
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }

    async fn get_file_info(
        &self,
        zone_id: &str,
        task_id: TaskId,
        file_seq: u64,
    ) -> Result<Option<FileInfo>, Box<dyn Error + Send + Sync>> {
        let url = format!("{}/task-mgr/get-file-info", self.base_url);
        let request_body = GetFileInfoRequest {
            zone_id: zone_id.to_string(),
            task_id,
            file_seq,
        };
        let client = Client::new();
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;

        if response.status().is_success() {
            let response = response
                .json::<GetFileInfoResponse>()
                .await
                .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)?;
            Ok(response.file_info)
        } else {
            let state_code = response.status().into();
            let error_message = response.text().await.unwrap_or_else(|_| "".to_string());
            Err(Box::new(SimpleServerError::new(
                state_code,
                error_message,
            )))
        }
    }
}

impl TaskMgrServer for TaskMgrHttpClient {

}

pub struct SimpleTaskMgrSelector {
    server_name: String,
}

impl SimpleTaskMgrSelector {
    pub fn new(server_name: &str) -> Self {
        SimpleTaskMgrSelector {
            server_name: server_name.to_string(),
        }
    }
}

// #[async_trait::async_trait]
// impl TaskMgrServerSelector for SimpleTaskMgrSelector {
//     async fn select(
//         &self,
//         task_key: &TaskKey,
//         check_point_version: CheckPointVersion,
//         file_hash: &str,
//     ) -> Result<Box<dyn TaskMgrServer>, Box<dyn std::error::Error + Send + Sync>> {
//         Ok(Box::new(ChunkMgrHttpClient::new(self.server_name.clone())))
//     }

//     async fn select_by_name(
//         &self,
//         file_server_type: TaskServerType,
//         server_name: &str,
//     ) -> Result<Box<dyn TaskMgrServer>, Box<dyn std::error::Error + Send + Sync>> {
//         Ok(Box::new(TaskMgrHttpClient::new(server_name.to_string())))
//     }
// }

#[async_trait::async_trait]
impl TaskMgrSelector for SimpleTaskMgrSelector {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: Option<CheckPointVersion>,
    ) -> Result<Box<dyn TaskMgr>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(TaskMgrHttpClient::new(self.server_name.as_str())))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct AddFileRequest {
    task_server_type: TaskServerType,
    task_server_name: String,
    file_hash: String,
    file_size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct AddFileResponse {
    file_id: FileId,
    chunk_size: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct AddChunkInFileRequest {
    file_id: FileId,
    chunk_seq: u64,
    chunk_hash: String,
    chunk_size: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct AddChunkInFileResponse {
    chunk_server_type: ChunkServerType,
    chunk_server_name: String,
    chunk_id: ChunkId,
}

#[derive(Debug, Serialize, Deserialize)]
struct SetChunkUploadedRequest {
    file_id: FileId,
    chunk_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetChunkInfoRequest {
    file_id: FileId,
    chunk_seq: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetChunkInfoResponse {
    chunk_info: Option<ChunkInfo>,
}

pub struct FileMgrHttpServer {
    file_mgr: Arc<Box<dyn FileMgrServer>>,
}

impl FileMgrHttpServer {
    pub fn new(file_mgr: Box<dyn FileMgrServer>) -> Self {
        FileMgrHttpServer { file_mgr: Arc::new(file_mgr) }
    }

    async fn add_file_handler(request: AddFileRequest, file_mgr: Arc<Box<dyn FileMgrServer>>) -> Result<impl Reply, warp::Rejection> {
        let (file_id, chunk_size) = file_mgr.add_file(
            request.task_server_type,
            &request.task_server_name,
            &request.file_hash,
            request.file_size,
        ).await.map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        let response = AddFileResponse {
            file_id,
            chunk_size,
        };

        Ok(warp::reply::json(&response))
    }

    async fn add_chunk_handler(request: AddChunkInFileRequest, file_mgr: Arc<Box<dyn FileMgrServer>>) -> Result<impl Reply, warp::Rejection> {
        let (chunk_server_type, chunk_server_name, chunk_id) = file_mgr.add_chunk(
            request.file_id,
            request.chunk_seq,
            &request.chunk_hash,
            request.chunk_size,
        ).await.map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        let response = AddChunkInFileResponse {
            chunk_server_type,
            chunk_server_name,
            chunk_id,
        };

        Ok(warp::reply::json(&response))
    }

    async fn set_chunk_uploaded_handler(request: SetChunkUploadedRequest, file_mgr: Arc<Box<dyn FileMgrServer>>) -> Result<impl Reply, warp::Rejection> {
        file_mgr.set_chunk_uploaded(request.file_id, request.chunk_seq)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        Ok(warp::reply())
    }

    async fn get_chunk_info_handler(request: GetChunkInfoRequest, file_mgr: Arc<Box<dyn FileMgrServer>>) -> Result<impl Reply, warp::Rejection> {
        let chunk_info = file_mgr.get_chunk_info(request.file_id, request.chunk_seq)
            .await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        let response = GetChunkInfoResponse {
            chunk_info,
        };

        Ok(warp::reply::json(&response))
    }

    pub fn routes(file_mgr: Arc<Box<dyn FileMgrServer>>) -> impl Filter<Extract = impl Reply, Error = warp::Rejection> + Clone {
        let add_file = {
            let file_mgr = file_mgr.clone();
            warp::path("add_file")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: AddFileRequest| Self::add_file_handler(request, file_mgr.clone()))
        };

        let add_chunk = {
            let file_mgr = file_mgr.clone();
            warp::path("add_chunk")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: AddChunkInFileRequest| Self::add_chunk_handler(request, file_mgr.clone()))
        };

        let set_chunk_uploaded = {
            let file_mgr = file_mgr.clone();
            warp::path("set_chunk_uploaded")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: SetChunkUploadedRequest| Self::set_chunk_uploaded_handler(request, file_mgr.clone()))
        };

        let get_chunk_info = {
            let file_mgr = file_mgr.clone();
            warp::path("get_chunk_info")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: GetChunkInfoRequest| Self::get_chunk_info_handler(request, file_mgr.clone()))
        };

        warp::path("file-mgr").and(add_file.or(add_chunk).or(set_chunk_uploaded).or(get_chunk_info))
    }
}

struct FileMgrHttpClient {
    base_url: String,
    client: Client,
}

impl FileMgrHttpClient {
    fn new(base_url: String) -> Self {
        FileMgrHttpClient {
            base_url,
            client: Client::new(),
        }
    }

    async fn send_request<T>(&self, path: &str, body: serde_json::Value) -> Result<T, Box<dyn Error + Send + Sync>>
    where
        T: serde::de::DeserializeOwned,
    {
        let url = format!("{}//file-mgr{}", self.base_url, path);
        let response = self.client.post(&url).json(&body).send().await?;
        let response_body: String = response.text().await?;
        let result = serde_json::from_str(&response_body)?;
        Ok(result)
    }
}

#[async_trait::async_trait]
impl FileMgrServer for FileMgrHttpClient {
    async fn add_file(
        &self,
        task_server_type: TaskServerType,
        task_server_name: &str,
        file_hash: &str,
        file_size: u64,
    ) -> Result<(FileId, u32), Box<dyn Error + Send + Sync>> {
        let request_body = json!({
            "task_server_type": task_server_type,
            "task_server_name": task_server_name,
            "file_hash": file_hash,
            "file_size": file_size,
        });
        let response: serde_json::Value = self.send_request("/add_file", request_body).await?;
        let file_id = FileId::from(response["file_id"].as_u64().ok_or("Invalid file_id")? as u128);
        let chunk_size = response["chunk_size"].as_u64().ok_or("Invalid chunk_size")? as u32;
        Ok((file_id, chunk_size))
    }
}

#[async_trait::async_trait]
impl FileMgr for FileMgrHttpClient {
    async fn add_chunk(
        &self,
        file_id: FileId,
        chunk_seq: u64,
        chunk_hash: &str,
        chunk_size: u32,
    ) -> Result<(ChunkServerType, String, ChunkId), Box<dyn Error + Send + Sync>> {
        let request_body = json!({
            "file_id": file_id,
            "chunk_seq": chunk_seq,
            "chunk_hash": chunk_hash,
            "chunk_size": chunk_size,
        });
        let response: serde_json::Value = self.send_request("/add_chunk", request_body).await?;
        let chunk_server_type = ChunkServerType::try_from(response["chunk_server_type"].as_u64().ok_or("Invalid chunk_server_type")? as u32).expect("Invalid chunk_server_type");
        let chunk_server_name = response["chunk_server_name"].as_str().ok_or("Invalid chunk_server_name")?;
        let chunk_id = ChunkId::from(response["chunk_id"].as_u64().ok_or("Invalid chunk_id")? as u128);
        Ok((chunk_server_type, chunk_server_name.to_string(), chunk_id))
    }

    async fn set_chunk_uploaded(
        &self,
        file_id: FileId,
        chunk_seq: u64,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let request_body = json!({
            "file_id": file_id,
            "chunk_seq": chunk_seq,
        });
        self.send_request::<()>("/set_chunk_uploaded", request_body).await?;
        Ok(())
    }

    async fn get_chunk_info(
        &self,
        file_id: FileId,
        chunk_seq: u64,
    ) -> Result<Option<ChunkInfo>, Box<dyn Error + Send + Sync>> {
        let request_body = json!({
            "file_id": file_id,
            "chunk_seq": chunk_seq,
        });
        let response: serde_json::Value = self.send_request("/get_chunk_info", request_body).await?;
        let chunk_info = response["chunk_info"].as_object().map(|obj| serde_json::from_value(serde_json::Value::Object(obj.clone()))).transpose()?;
        Ok(chunk_info)
    }

    fn server_type(&self) -> FileServerType {
        FileServerType::Http
    }

    fn server_name(&self) -> &str {
        &self.base_url
    }
}

pub struct SimpleFileMgrSelector {
    server_name: String,
}

impl SimpleFileMgrSelector {
    pub fn new(server_name: &str) -> Self {
        SimpleFileMgrSelector {
            server_name: server_name.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl FileMgrServerSelector for SimpleFileMgrSelector {
    async fn select(
        &self,
        task_key: &TaskKey,
        check_point_version: CheckPointVersion,
        file_hash: &str,
    ) -> Result<Box<dyn FileMgrServer>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(FileMgrHttpClient::new(self.server_name.clone())))
    }

    async fn select_by_name(
        &self,
        file_server_type: FileServerType,
        server_name: &str,
    ) -> Result<Box<dyn FileMgrServer>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(FileMgrHttpClient::new(server_name.to_string())))
    }
}

#[async_trait::async_trait]
impl FileMgrSelector for SimpleFileMgrSelector {
    async fn select_by_name(
        &self,
        file_server_type: FileServerType,
        server_name: &str,
    ) -> Result<Box<dyn FileMgr>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(FileMgrHttpClient::new(server_name.to_string())))
    }
}

#[derive(Serialize, Deserialize)]
struct AddChunkRequest {
    file_server_type: FileServerType,
    file_server_name: String,
    chunk_hash: String,
    chunk_size: u32,
}

#[derive(Serialize, Deserialize)]
struct AddChunkResponse {
    chunk_id: ChunkId,
}

#[derive(Serialize, Deserialize)]
struct UploadRequest {
    chunk_hash: String,
    chunk: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct DownloadRequest {
    chunk_id: ChunkId,
}

pub struct ChunkMgrHttpServer {
    // Implement the ChunkMgrServer trait here
}

impl ChunkMgrHttpServer {
    async fn add_chunk_handler(request: AddChunkRequest, chunk_mgr: Arc<Box<dyn ChunkMgrServer>>) -> Result<impl Reply, warp::Rejection> {
        // Convert request to ChunkMgrServer parameters
        let file_server_type = request.file_server_type;
        let file_server_name = request.file_server_name;
        let chunk_hash = request.chunk_hash;
        let chunk_size = request.chunk_size;

        // Call ChunkMgrServer interface
        let chunk_id = chunk_mgr.add_chunk(file_server_type, &file_server_name, &chunk_hash, chunk_size).await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        // Convert ChunkMgrServer response to HTTP response
        let response = AddChunkResponse {
            chunk_id,
        };

        Ok(warp::reply::json(&response))
    }

    async fn upload_handler(chunk_hash: String, chunk: warp::hyper::body::Bytes, chunk_mgr: Arc<Box<dyn ChunkMgrServer>>) -> Result<impl Reply, warp::Rejection> {
        // Call ChunkMgr interface
        chunk_mgr.upload(chunk_hash.as_str(), chunk.as_ref()).await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        // Convert ChunkMgr response to HTTP response

        Ok(warp::reply::json(&()))
    }

    async fn download_handler(request: DownloadRequest, chunk_mgr: Arc<Box<dyn ChunkMgrServer>>) -> Result<impl Reply, warp::Rejection> {
        // Convert request to ChunkMgr parameters
        let chunk_id = request.chunk_id;

        // Call ChunkMgr interface
        let chunk_data = chunk_mgr.download(chunk_id).await
            .map_err(|err| warp::reject::custom(SimpleServerError::new(warp::http::StatusCode::INTERNAL_SERVER_ERROR.into(), err.to_string())))?;

        // Convert ChunkMgr response to HTTP response

        Ok(chunk_data)
    }

    pub fn routes(chunk_mgr: Arc<Box<dyn ChunkMgrServer>>) -> impl Filter<Extract = impl Reply, Error = warp::Rejection> + Clone {
        let add_chunk = {
            let chunk_mgr = chunk_mgr.clone();
            warp::path("add_chunk")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: AddChunkRequest| Self::add_chunk_handler(request, chunk_mgr.clone()))
        };
        let upload = {
            let chunk_mgr = chunk_mgr.clone();
            warp::path("upload")
                .and(warp::path::param::<String>())
                .and(warp::post())
                .and(warp::body::bytes())
                .and_then(move |chunk_hash, chunk| Self::upload_handler(chunk_hash, chunk, chunk_mgr.clone()))
        };

        let download = {
            let chunk_mgr = chunk_mgr.clone();
            warp::path("download")
            .and(warp::post())
            .and(warp::body::json())
            .and_then(move |request: DownloadRequest| Self::download_handler(request, chunk_mgr.clone()))
        };

        warp::path("chunk-mgr").and(add_chunk.or(upload).or(download))
    }
}

struct ChunkMgrHttpClient {
    base_url: String,
    client: Client,
}

impl ChunkMgrHttpClient {
    fn new(base_url: String) -> Self {
        ChunkMgrHttpClient {
            base_url,
            client: Client::new(),
        }
    }

    async fn send_request<T>(&self, path: &str, body: serde_json::Value) -> Result<T, Box<dyn Error + Send + Sync>>
    where
        T: serde::de::DeserializeOwned,
    {
        let url = format!("{}/chunk-mgr{}", self.base_url, path);
        let response = self.client.post(&url).json(&body).send().await?;
        let response_body: String = response.text().await?;
        let result = serde_json::from_str(&response_body)?;
        Ok(result)
    }
}

#[async_trait::async_trait]
impl ChunkMgrServer for ChunkMgrHttpClient {
    async fn add_chunk(
        &self,
        file_server_type: FileServerType,
        file_server_name: &str,
        chunk_hash: &str,
        chunk_size: u32,
    ) -> Result<ChunkId, Box<dyn std::error::Error + Send + Sync>> {
        let request = AddChunkRequest {
            file_server_type,
            file_server_name: file_server_name.to_string(),
            chunk_hash: chunk_hash.to_string(),
            chunk_size,
        };

        let body = serde_json::to_value(request)?;
        let response: AddChunkResponse = self.send_request("/add_chunk", body).await?;

        Ok(response.chunk_id)
    }
}

// 为ChunkMgrHttpClient实现所有ChunkMgr trait接口，其实现为远程调用ChunkMgrHttpServer提供的远程接口
#[async_trait::async_trait]
impl ChunkMgr for ChunkMgrHttpClient {
    fn server_type(&self) -> ChunkServerType {
        ChunkServerType::Http
    }

    fn server_name(&self) -> &str {
        self.base_url.as_str()
    }

    async fn upload(
        &self,
        chunk_hash: &str,
        chunk: &[u8],
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/chunk-mgr/upload/{}", self.base_url, chunk_hash);
        let response = self.client.post(&url).body(chunk.to_vec()).send().await?;
        let response_body: String = response.text().await?;
        let result = serde_json::from_str(&response_body)?;
        Ok(result)
    }

    async fn download(
        &self,
        chunk_id: ChunkId,
    ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let request = DownloadRequest {
            chunk_id,
        };

        let url = format!("{}/chunk-mgr/download", self.base_url);
        let response = self.client.post(&url).json(&request).send().await?;
        let chunk = response.bytes().await?.to_vec();
        Ok(chunk)
    }
}

pub struct SimpleChunkMgrSelector {
    server_name: String,
}

impl SimpleChunkMgrSelector {
    pub fn new(server_name: &str) -> Self {
        SimpleChunkMgrSelector {
            server_name: server_name.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl ChunkMgrServerSelector for SimpleChunkMgrSelector {
    async fn select(
        &self,
        file_hash: &str,
        chunk_seq: u64,
        chunk_hash: &str,
    ) -> Result<Box<dyn ChunkMgrServer>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(ChunkMgrHttpClient::new(self.server_name.clone())))
    }

    async fn select_by_name(
        &self,
        chunk_server_type: ChunkServerType,
        server_name: &str,
    ) -> Result<Box<dyn ChunkMgrServer>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(ChunkMgrHttpClient::new(server_name.to_string())))
    }
}

#[async_trait::async_trait]
impl ChunkMgrSelector for SimpleChunkMgrSelector {
    async fn select_by_name(
        &self,
        chunk_server_type: ChunkServerType,
        server_name: &str,
    ) -> Result<Box<dyn ChunkMgr>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(ChunkMgrHttpClient::new(server_name.to_string())))
    }
}