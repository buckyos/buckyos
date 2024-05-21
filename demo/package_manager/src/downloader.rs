use async_trait::async_trait;
use futures_util::StreamExt;
use log::*;
use reqwest::Client;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::io::AsyncWriteExt;

use crate::error::*;

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub total_size: u64,
    pub downloaded_size: u64,
    pub speed: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub task_id: TaskId,
    pub url: String,
    pub target: PathBuf,
    pub result: Result<(), String>,
}

pub type TaskId = usize;

#[async_trait]
pub trait Downloader {
    async fn download(
        &self,
        url: &str,
        target: &PathBuf,
        callback: Option<Box<dyn FnOnce(DownloadResult) + Send>>,
    ) -> PkgSysResult<TaskId>;
    fn get_task_state(&self, task_id: TaskId) -> PkgSysResult<DownloadTask>;
}

#[derive(Clone)]
pub struct FakeDownloader {
    state: Arc<DownloaderState>,
}

impl FakeDownloader {
    pub fn new() -> Self {
        FakeDownloader {
            state: Arc::new(DownloaderState {
                tasks: Mutex::new(Vec::new()),
            }),
        }
    }
}

struct DownloaderState {
    tasks: Mutex<Vec<DownloadTask>>,
}

#[async_trait]
impl Downloader for FakeDownloader {
    async fn download(
        &self,
        url: &str,
        target: &PathBuf,
        callback: Option<Box<dyn FnOnce(DownloadResult) + Send>>,
    ) -> PkgSysResult<TaskId> {
        let state = self.state.clone();
        let url = url.to_string();
        let target = target.clone();

        let task_id = {
            let mut tasks = state.tasks.lock().unwrap();
            tasks.push(DownloadTask {
                total_size: 0,
                downloaded_size: 0,
                speed: 0,
                error: None,
            });
            tasks.len() - 1
        };

        tokio::spawn(async move {
            let client = Client::new();
            let start_time = Instant::now();

            let result = match client.get(&url).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let total_size = resp.content_length().unwrap_or(0);
                        let mut downloaded_size = 0;
                        let mut file = match tokio::fs::File::create(&target).await {
                            Ok(file) => file,
                            Err(e) => {
                                let error_msg =
                                    format!("Failed to create file {}: {}", target.display(), e);
                                error!("{}", error_msg);
                                let mut tasks = state.tasks.lock().unwrap();
                                if let Some(task) = tasks.get_mut(task_id) {
                                    task.error = Some(error_msg.clone());
                                }
                                if let Some(cb) = callback {
                                    cb(DownloadResult {
                                        task_id,
                                        url: url.clone(),
                                        target: target.clone(),
                                        result: Err(error_msg),
                                    });
                                }
                                return;
                            }
                        };

                        let mut stream = resp.bytes_stream();
                        while let Some(chunk) = stream.next().await {
                            match chunk {
                                Ok(bytes) => {
                                    if let Err(e) = file.write_all(&bytes).await {
                                        let error_msg = format!(
                                            "Failed to write to file {}: {}",
                                            target.display(),
                                            e
                                        );
                                        error!("{}", error_msg);
                                        let mut tasks = state.tasks.lock().unwrap();
                                        if let Some(task) = tasks.get_mut(task_id) {
                                            task.error = Some(error_msg.clone());
                                        }
                                        if let Some(cb) = callback {
                                            cb(DownloadResult {
                                                task_id,
                                                url: url.clone(),
                                                target: target.clone(),
                                                result: Err(error_msg),
                                            });
                                        }
                                        return;
                                    }

                                    downloaded_size += bytes.len() as u64;
                                    let elapsed = start_time.elapsed().as_secs();
                                    let speed = if elapsed > 0 {
                                        downloaded_size / elapsed
                                    } else {
                                        0
                                    };

                                    let mut tasks = state.tasks.lock().unwrap();
                                    if let Some(task) = tasks.get_mut(task_id) {
                                        task.total_size = total_size;
                                        task.downloaded_size = downloaded_size;
                                        task.speed = speed;
                                    }

                                    info!(
                                        "Downloading {}: {} bytes of {} bytes, {} bytes/sec",
                                        url, downloaded_size, total_size, speed
                                    );
                                }
                                Err(e) => {
                                    let error_msg =
                                        format!("Failed to read chunk from {}: {}", url, e);
                                    error!("{}", error_msg);
                                    let mut tasks = state.tasks.lock().unwrap();
                                    if let Some(task) = tasks.get_mut(task_id) {
                                        task.error = Some(error_msg.clone());
                                    }
                                    if let Some(cb) = callback {
                                        cb(DownloadResult {
                                            task_id,
                                            url: url.clone(),
                                            target: target.clone(),
                                            result: Err(error_msg),
                                        });
                                    }
                                    return;
                                }
                            }
                        }

                        info!("Download {} to {} success", url, target.display());
                        if let Some(cb) = callback {
                            cb(DownloadResult {
                                task_id,
                                url: url.clone(),
                                target: target.clone(),
                                result: Ok(()),
                            });
                        }
                    } else {
                        let error_msg =
                            format!("Failed to download {}: HTTP {}", url, resp.status());
                        error!("{}", error_msg);
                        let mut tasks = state.tasks.lock().unwrap();
                        if let Some(task) = tasks.get_mut(task_id) {
                            task.error = Some(error_msg.clone());
                        }
                        if let Some(cb) = callback {
                            cb(DownloadResult {
                                task_id,
                                url: url.clone(),
                                target: target.clone(),
                                result: Err(error_msg),
                            });
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!("Failed to send request to {}: {}", url, e);
                    error!("{}", error_msg);
                    let mut tasks = state.tasks.lock().unwrap();
                    if let Some(task) = tasks.get_mut(task_id) {
                        task.error = Some(error_msg.clone());
                    }
                    if let Some(cb) = callback {
                        cb(DownloadResult {
                            task_id,
                            url: url.clone(),
                            target: target.clone(),
                            result: Err(error_msg),
                        });
                    }
                }
            };
        });

        Ok(task_id)
    }

    fn get_task_state(&self, task_id: TaskId) -> PkgSysResult<DownloadTask> {
        let tasks = self.state.tasks.lock().unwrap();
        if let Some(task) = tasks.get(task_id) {
            Ok(task.clone())
        } else {
            Err(PackageSystemErrors::DownloadError(
                task_id.to_string(),
                format!("task_id {} not found", task_id),
            ))
        }
    }
}
