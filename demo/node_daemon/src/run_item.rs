use async_trait::async_trait;
use log::{info, warn,debug};
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
    //InstllDeps,
    Deploying,
    //DeployFailed(String,u32), //error message,failed count
    NotExist,
    Started,
    Stopped(String), //version
}

pub enum RunItemTargetState{
    Running(String),//version
    Stopped(String),//version
}


pub struct RunItemParams {
    pub node_ip:String,
    pub services_cfg:Option<String>,
}

impl RunItemParams {
    pub fn new(node_ip: String) -> Self {
        RunItemParams {
            node_ip,
            None,
        }
    }
}

#[async_trait]
pub trait RunItemControl {
    fn get_item_name(&self)->String;
    async fn deploy(&self,params:Option<&RunItemParams>) -> Result<()>;
    async fn remove(&self,params:Option<&RunItemParams>) -> Result<()>;
    //return new version
    async fn update(&self,params:Option<&RunItemParams>) -> Result<String>;

    async fn start(&self,params:Option<&RunItemParams>) -> Result<()>;
    async fn stop(&self,params:Option<&RunItemParams>) -> Result<()>;

    async fn get_state(&self,params:Option<&RunItemParams>) -> Result<RunItemState>;
}

pub async fn control_run_item_to_target_state(item:&dyn RunItemControl,target_state:RunItemTargetState,params:Option<&RunItemParams>)->Result<()>{
    match target_state {
        RunItemTargetState::Running(version)=>{
            match item.get_state(params).await? {
                RunItemState::Started=>{
                    debug!("{} is already running, do nothing!",item.get_item_name());
                    Ok(())
                },
                RunItemState::NotExist=>{
                    warn!("{} not exist,deploy and start it!",item.get_item_name());
                    item.deploy(params).await?;
                    warn!("{} deploy success,start it!",item.get_item_name());    
                    item.start(params).await?;
                    Ok(())
                },
                RunItemState::Stopped(_)=>{
                    warn!("{} stopped,start it!",item.get_item_name());
                    item.start(params).await?;
                    Ok(())
                },
                RunItemState::Deploying=>{
                    warn!("{} is deploying,wait for it!",item.get_item_name());
                    Ok(())
                }
            }
        },
        RunItemTargetState::Stopped(version)=>{
            match item.get_state(params).await? {
                RunItemState::Started=>{
                    warn!("{} is running,stop it!",item.get_item_name());   
                    item.stop(params).await?;
                    Ok(())
                },
                RunItemState::NotExist=>{
                    warn!("{} not exist,deploy it!",item.get_item_name());
                    item.deploy(params).await?;
                    Ok(())
                },
                RunItemState::Stopped(_)=>{
                    debug!("{} already stopped, do nothing!",item.get_item_name());
                    Ok(())
                },
                RunItemState::Deploying=>{
                    warn!("{} is deploying,wait for it!",item.get_item_name());
                    Ok(())
                }
            }
        }
    }
}