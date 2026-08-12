use crate::run_plist::{update_run_item, RunPlistItemState};
use crate::service_pkg::*;
use async_trait::async_trait;
use buckyos_api::ServiceInstanceState;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
    #[error("Not support: {0}")]
    NotSupport(String),
}

pub type Result<T> = std::result::Result<T, ControlRuntItemErrors>;

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RunItemControlOperation {
    pub command: String,
    pub params: Option<Vec<String>>,
}

#[async_trait]
pub trait RunItemControl: Send + Sync {
    fn get_item_name(&self) -> Result<String>;
    fn get_item_kind(&self) -> &'static str {
        "run_item"
    }
    async fn deploy(&self, params: Option<&Vec<String>>) -> Result<()>;
    //async fn remove(&self, params: Option<&RunItemParams>) -> Result<()>;
    //return new version
    //async fn update(&self, params: &Option<RunItemParams>) -> Result<String>;

    async fn start(&self, params: Option<&Vec<String>>) -> Result<()>;
    async fn stop(&self, params: Option<&Vec<String>>) -> Result<()>;
    async fn get_state(&self, params: Option<&Vec<String>>) -> Result<ServiceInstanceState>;
}

pub async fn ensure_run_item_state(
    item: &dyn RunItemControl,
    target_state: RunItemTargetState,
) -> Result<()> {
    let item_name = item.get_item_name()?;
    let item_kind = item.get_item_kind();
    match target_state {
        RunItemTargetState::Running => {
            record_run_item_state(
                item_name.as_str(),
                item_kind,
                &target_state,
                None,
                RunPlistItemState::PlannedStart,
                None,
            );
            let current_state =
                observe_run_item_state(item, item_name.as_str(), item_kind, &target_state).await?;
            match &current_state {
                ServiceInstanceState::Started => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::AlreadyRunning,
                        None,
                    );
                    debug!("{} is already running, do nothing!", item_name);
                    Ok(())
                }
                ServiceInstanceState::NotExist => {
                    warn!("{} not exist,deploy and start it!", item_name);
                    deploy_run_item(
                        item,
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        &current_state,
                    )
                    .await?;
                    warn!("{} deploy success,start it!", item_name);
                    start_run_item(
                        item,
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        &current_state,
                    )
                    .await?;
                    Ok(())
                }
                ServiceInstanceState::Stopped => {
                    warn!("{} stopped,start it!", item_name);
                    start_run_item(
                        item,
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        &current_state,
                    )
                    .await?;
                    Ok(())
                }
                ServiceInstanceState::Exited => {
                    warn!("{} stopped,start it!", item_name);
                    start_run_item(
                        item,
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        &current_state,
                    )
                    .await?;
                    Ok(())
                }
                ServiceInstanceState::Deploying => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::WaitingDeploy,
                        None,
                    );
                    warn!("{} is deploying,wait for it!", item_name);
                    Ok(())
                }
            }
        }
        RunItemTargetState::Exited => {
            let current_state =
                observe_run_item_state(item, item_name.as_str(), item_kind, &target_state).await?;
            match &current_state {
                ServiceInstanceState::NotExist => {
                    warn!("{} not exist,deploy a it!", item_name);
                    deploy_run_item(
                        item,
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        &current_state,
                    )
                    .await?;
                    Ok(())
                }
                ServiceInstanceState::Exited => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::Exited,
                        None,
                    );
                    Ok(())
                }
                ServiceInstanceState::Started => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::Started,
                        None,
                    );
                    Ok(())
                }
                ServiceInstanceState::Stopped => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::Stopped,
                        None,
                    );
                    Ok(())
                }
                ServiceInstanceState::Deploying => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::WaitingDeploy,
                        None,
                    );
                    Ok(())
                }
            }
        }
        RunItemTargetState::Stopped => {
            record_run_item_state(
                item_name.as_str(),
                item_kind,
                &target_state,
                None,
                RunPlistItemState::PlannedStop,
                None,
            );
            let current_state =
                observe_run_item_state(item, item_name.as_str(), item_kind, &target_state).await?;
            match &current_state {
                ServiceInstanceState::Started => {
                    warn!("{} is running,stop it!", item_name);
                    stop_run_item(
                        item,
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        &current_state,
                    )
                    .await?;
                    Ok(())
                }
                ServiceInstanceState::NotExist => {
                    //warn!("{} not exist,deploy it!", item_name);
                    //item.deploy(None).await?;
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::NotExist,
                        None,
                    );
                    debug!("{} not exist,do nothing!", item_name);
                    Ok(())
                }
                ServiceInstanceState::Exited => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::Exited,
                        None,
                    );
                    warn!("{} exited,do nothing!", item_name);
                    Ok(())
                }
                ServiceInstanceState::Stopped => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::Stopped,
                        None,
                    );
                    debug!("{} already stopped, do nothing!", item_name);
                    Ok(())
                }
                ServiceInstanceState::Deploying => {
                    record_run_item_state(
                        item_name.as_str(),
                        item_kind,
                        &target_state,
                        Some(&current_state),
                        RunPlistItemState::WaitingDeploy,
                        None,
                    );
                    warn!("{} is deploying,wait for it!", item_name);
                    Ok(())
                }
            }
        }
    }
}

fn record_run_item_state(
    item_name: &str,
    item_kind: &str,
    target_state: &RunItemTargetState,
    observed_state: Option<&ServiceInstanceState>,
    run_state: RunPlistItemState,
    last_error: Option<String>,
) {
    update_run_item(
        item_name,
        item_kind,
        target_state,
        observed_state,
        run_state,
        last_error,
    );
}

async fn observe_run_item_state(
    item: &dyn RunItemControl,
    item_name: &str,
    item_kind: &str,
    target_state: &RunItemTargetState,
) -> Result<ServiceInstanceState> {
    match item.get_state(None).await {
        Ok(state) => Ok(state),
        Err(err) => {
            record_run_item_state(
                item_name,
                item_kind,
                target_state,
                None,
                RunPlistItemState::ObserveFailed,
                Some(err.to_string()),
            );
            Err(err)
        }
    }
}

async fn deploy_run_item(
    item: &dyn RunItemControl,
    item_name: &str,
    item_kind: &str,
    target_state: &RunItemTargetState,
    observed_state: &ServiceInstanceState,
) -> Result<()> {
    record_run_item_state(
        item_name,
        item_kind,
        target_state,
        Some(observed_state),
        RunPlistItemState::Deploying,
        None,
    );
    if let Err(err) = item.deploy(None).await {
        record_run_item_state(
            item_name,
            item_kind,
            target_state,
            Some(observed_state),
            RunPlistItemState::DeployFailed,
            Some(err.to_string()),
        );
        return Err(err);
    }
    record_run_item_state(
        item_name,
        item_kind,
        target_state,
        Some(observed_state),
        RunPlistItemState::Deployed,
        None,
    );
    Ok(())
}

async fn start_run_item(
    item: &dyn RunItemControl,
    item_name: &str,
    item_kind: &str,
    target_state: &RunItemTargetState,
    observed_state: &ServiceInstanceState,
) -> Result<()> {
    record_run_item_state(
        item_name,
        item_kind,
        target_state,
        Some(observed_state),
        RunPlistItemState::Starting,
        None,
    );
    if let Err(err) = item.start(None).await {
        record_run_item_state(
            item_name,
            item_kind,
            target_state,
            Some(observed_state),
            RunPlistItemState::StartFailed,
            Some(err.to_string()),
        );
        return Err(err);
    }
    record_run_item_state(
        item_name,
        item_kind,
        target_state,
        Some(&ServiceInstanceState::Started),
        RunPlistItemState::Started,
        None,
    );
    Ok(())
}

async fn stop_run_item(
    item: &dyn RunItemControl,
    item_name: &str,
    item_kind: &str,
    target_state: &RunItemTargetState,
    observed_state: &ServiceInstanceState,
) -> Result<()> {
    record_run_item_state(
        item_name,
        item_kind,
        target_state,
        Some(observed_state),
        RunPlistItemState::Stopping,
        None,
    );
    if let Err(err) = item.stop(None).await {
        record_run_item_state(
            item_name,
            item_kind,
            target_state,
            Some(observed_state),
            RunPlistItemState::StopFailed,
            Some(err.to_string()),
        );
        return Err(err);
    }
    record_run_item_state(
        item_name,
        item_kind,
        target_state,
        Some(&ServiceInstanceState::Stopped),
        RunPlistItemState::Stopped,
        None,
    );
    Ok(())
}
