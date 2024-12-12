use async_trait::async_trait;
use buckyos_kit::ServiceState;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::{debug, info, warn};
use serde_json::Value;
use serde::{Deserialize, Serialize};


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
}

pub type Result<T> = std::result::Result<T, ControlRuntItemErrors>;

#[derive(Serialize, Deserialize, Debug,Clone)]
pub enum RunItemTargetState {
    Running, 
    Stopped, 
}

impl RunItemTargetState {
    pub fn from_str(state: &str) -> Result<Self> {
        match state {
            "Running" => Ok(RunItemTargetState::Running),
            "Stopped" => Ok(RunItemTargetState::Stopped),
            _ => Err(ControlRuntItemErrors::ParserConfigError(format!("invalid target state: {}", state))),
        }
    }
}


#[derive(Serialize, Deserialize, Debug,Clone)]
pub struct RunItemControlOperation {
    pub command : String,
    pub params : Option<Vec<String>>,
}
#[async_trait]
pub trait RunItemControl {
    fn get_item_name(&self) -> Result<String>;
    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()>;
    //async fn remove(&self, params: Option<&RunItemParams>) -> Result<()>;
    //return new version
    //async fn update(&self, params: &Option<RunItemParams>) -> Result<String>;

    async fn start(&self, control_key:&EncodingKey,params:Option<&Vec<String>>) -> Result<()>;
    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()>;

    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceState>;
}

pub async fn control_run_item_to_target_state(
    item: &dyn RunItemControl,
    target_state: RunItemTargetState,
    device_private_key: &EncodingKey
) -> Result<()> {
    let item_name = item.get_item_name()?;
    match target_state {
        RunItemTargetState::Running => match item.get_state(None).await? {
            ServiceState::Started => {
                debug!("{} is already running, do nothing!", item_name);
                Ok(())
            }
            ServiceState::NotExist => {
                warn!("{} not exist,deploy and start it!", item_name);
                item.deploy(None).await?;
                warn!("{} deploy success,start it!", item_name);
                item.start(device_private_key,None).await?;
                Ok(())
            }
            ServiceState::Stopped => {
                warn!("{} stopped,start it!", item_name);
                item.start(device_private_key,None).await?;
                Ok(())
            }
            ServiceState::Deploying => {
                warn!("{} is deploying,wait for it!", item_name);
                Ok(())
            }
        },
        RunItemTargetState::Stopped => match item.get_state(None).await? {
            ServiceState::Started => {
                warn!("{} is running,stop it!", item_name);
                item.stop(None).await?;
                Ok(())
            }
            ServiceState::NotExist => {
                warn!("{} not exist,deploy it!", item_name);
                item.deploy(None).await?;
                Ok(())
            }
            ServiceState::Stopped => {
                debug!("{} already stopped, do nothing!", item_name);
                Ok(())
            }
            ServiceState::Deploying => {
                warn!("{} is deploying,wait for it!", item_name);
                Ok(())
            }
        },
    }
}
