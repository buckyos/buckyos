use async_trait::async_trait;
use serde_json::Value;

use thiserror::Error;
#[derive(Error, Debug)]
pub enum ControlRuntItemErrors {
    #[error("Download {0} error!")]
    DownloadError(String),
    #[error("Execute cmd {0} error:{1}")]
    ExecuteError(String,String),
    #[error("Config parser error: {0}")]
    ParserConfigError(String),
    #[error("Network Error: {0}")]
    NetworkError(String),
}

pub type Result<T> = std::result::Result<T, ControlRuntItemErrors>;

pub enum RunItemState {
    Deploying,
    NotExist,
    Started,
    Stopped(String), //version
}

#[async_trait]
pub trait RunItemControl {
    async fn deploy(&self,params:Option<&Value>) -> Result<()>;
    async fn remove(&self,params:Option<&Value>) -> Result<()>;
    //return new version
    async fn update(&self,params:Option<&Value>) -> Result<String>;

    async fn start(&self,params:Option<&Value>) -> Result<()>;
    async fn stop(&self,params:Option<&Value>) -> Result<()>;

    async fn get_state(&self) -> Result<RunItemState>;
}