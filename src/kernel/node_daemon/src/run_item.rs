use async_trait::async_trait;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::{debug, info, warn};
use serde_json::Value;
use serde::{Deserialize, Serialize};
use buckyos_api::ServiceInstanceState;
use crate::service_pkg::*;
use thiserror::Error;
#[derive(Error, Debug)]
pub enum ControlRuntItemErrors {
    #[error("Download {0} error!")]
    DownloadError(String),
    #[error("Execute cmd {0} error:{1}")]
    ExecuteError(String, String),
    #[error("Config parser error: {0}")]
    ParserConfigError(String),
    #[error("Network Error: {0}")]
    NetworkError(String),
    #[error("Pkg not exist: {0}")]
    PkgNotExist(String),
}

pub type Result<T> = std::result::Result<T, ControlRuntItemErrors>;

#[derive(Serialize, Deserialize, Debug,Clone)]
pub enum RunItemTargetState {
    Running, 
    Stopped, 
    Exited,
}

impl RunItemTargetState {
    pub fn from_instance_state(state: &ServiceInstanceState) -> Self {
        match state {
            ServiceInstanceState::Started => RunItemTargetState::Running,
            ServiceInstanceState::Exited => RunItemTargetState::Exited,
            _ => RunItemTargetState::Stopped,
        }
    }
}


#[derive(Serialize, Deserialize, Debug,Clone)]
pub struct RunItemControlOperation {
    pub command : String,
    pub params : Option<Vec<String>>,
}

#[async_trait]
pub trait RunItemControl: Send + Sync {
    fn get_item_name(&self) -> Result<String>;
    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()>;
    //async fn remove(&self, params: Option<&RunItemParams>) -> Result<()>;
    //return new version
    //async fn update(&self, params: &Option<RunItemParams>) -> Result<String>;

    async fn start(&self,params:Option<&Vec<String>>) -> Result<()>;
    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()>;
    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceInstanceState>;
}

pub async fn ensure_run_item_state(
    item: &dyn RunItemControl,
    target_state: RunItemTargetState
) -> Result<()> {
    let item_name = item.get_item_name()?;
    match target_state {
        RunItemTargetState::Running => match item.get_state(None).await? {
            ServiceInstanceState::Started => {
                debug!("{} is already running, do nothing!", item_name);
                Ok(())
            }
            ServiceInstanceState::NotExist => {
                warn!("{} not exist,deploy and start it!", item_name);
                item.deploy(None).await?;
                warn!("{} deploy success,start it!", item_name);
                item.start(None).await?;
                Ok(())
            }
            ServiceInstanceState::Stopped => {
                warn!("{} stopped,start it!", item_name);
                item.start(None).await?;
                Ok(())
            }
            ServiceInstanceState::Exited => {
                warn!("{} stopped,start it!", item_name);
                item.start(None).await?;
                Ok(())
            }
            ServiceInstanceState::Deploying => {
                warn!("{} is deploying,wait for it!", item_name);
                Ok(())
            }
        },
        RunItemTargetState::Exited => match item.get_state(None).await? {
            ServiceInstanceState::NotExist => {
                warn!("{} not exist,deploy a it!", item_name);
                item.deploy(None).await?;
                Ok(())
            }
            _ => {
                Ok(())
            }
        },
        RunItemTargetState::Stopped => match item.get_state(None).await? {
            ServiceInstanceState::Started => {
                warn!("{} is running,stop it!", item_name);
                item.stop(None).await?;
                Ok(())
            }
            ServiceInstanceState::NotExist => {
                //warn!("{} not exist,deploy it!", item_name);
                //item.deploy(None).await?;
                debug!("{} not exist,do nothing!", item_name);
                Ok(())
            }
            ServiceInstanceState::Exited => {
                warn!("{} exited,do nothing!", item_name);
                Ok(())
            }
            ServiceInstanceState::Stopped => {
                debug!("{} already stopped, do nothing!", item_name);
                Ok(())
            }
            ServiceInstanceState::Deploying => {
                warn!("{} is deploying,wait for it!", item_name);
                Ok(())
            }
        },
    }
}
