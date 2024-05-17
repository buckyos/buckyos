use crate::error::*;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct DownloadTask {
    progress: u8,
    speed: u64,
}

// 下载器的状态，包含所有下载任务的状态
struct DownloaderState {
    tasks: Mutex<Vec<DownloadTask>>,
}

pub type TaskId = usize;

#[async_trait]
pub trait Downloader {
    async fn download(&self, url: &str, target: &PathBuf) -> PkgSysResult<TaskId>;
    fn get_task_state(&self, task_id: TaskId) -> PkgSysResult<DownloadTask>;
}

// fake downloader
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

#[async_trait]
impl Downloader for FakeDownloader {
    async fn download(&self, url: &str, target: &PathBuf) -> PkgSysResult<TaskId> {
        /*let state = self.state.clone();
        let url = url.to_string();
        let target = PathBuf::from(target);

        // 创建新的下载任务状态
        let task_id = {
            let mut tasks = state.tasks.lock().unwrap();
            tasks.push(DownloadTask {
                progress: 0,
                speed: 0,
            });
            tasks.len() - 1
        };

        // 启动一个新的异步任务来执行下载
        tokio::spawn(async move {
            // 先试用普通的http下载
            let client = reqwest::Client::new();
            let resp = client.get(&url).send().await.unwrap();
            let body = resp.bytes().await.unwrap();

            // 写入文件
            std::fs::write(&target, &body).unwrap();

            // 更新下载任务状态
            let mut tasks = state.tasks.lock().unwrap();
            tasks[task_id].progress = 100;

            info!("download {} to {} success", url, target.display());
        });

        Ok(task_id)*/
        unimplemented!("fake downloader")
    }

    fn get_task_state(&self, task_id: TaskId) -> PkgSysResult<DownloadTask> {
        /*let tasks = self.state.tasks.lock().unwrap();
        if task_id < tasks.len() {
            Ok(tasks[task_id])
        } else {
            Err(PackageSystemErrors::DownloadError(format!(
                "task_id {} not found",
                task_id
            )))
        }*/
        unimplemented!("fake downloader")
    }
}
