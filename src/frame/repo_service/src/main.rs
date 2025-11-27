#![allow(unused, dead_code)]


mod repo_server;
mod pkg_task_data;

#[cfg(test)]
mod test;

use crate::repo_server::*;
use std::{fs::File, sync::Arc};
use buckyos_api::*;

use log::*;
use serde_json::*;

use buckyos_kit::*;
use name_client::*;
use cyfs_gateway_lib::WarpServerConfig;
use server_runner::*;

use anyhow::Result;

const REPO_SERVICE_MAIN_PORT: u16 = 4000;

async fn service_main() -> Result<()> {
    init_logging("repo_service",true);
    let mut runtime = init_buckyos_api_runtime("repo-service",None,BuckyOSRuntimeType::KernelService).await?;
    let login_result = runtime.login().await;
    if  login_result.is_err() {
        error!("repo service login to system failed! err:{:?}", login_result);
        return Err(anyhow::anyhow!("repo service login to system failed! err:{:?}", login_result));
    }
    runtime.set_main_service_port(REPO_SERVICE_MAIN_PORT).await;
    set_buckyos_api_runtime(runtime);
    let runtime = get_buckyos_api_runtime()?;

    let repo_service_settings = runtime.get_my_settings().await
      .map_err(|e| {
        error!("repo service settings not found! err:{}", e);
        anyhow::anyhow!("repo service settings not found! err:{}", e)
      })?;
    let repo_service_settings: RepoServerSetting = serde_json::from_value(repo_service_settings).map_err(|e| {
      error!("repo service settings parse error! err:{}", e);
      anyhow::anyhow!("repo service settings parse error! err:{}", e)
    })?;

    let repo_server_data_folder = runtime.get_data_folder();
    // 确保repo_server_data_folder目录存在
    if !repo_server_data_folder.exists() {
        std::fs::create_dir_all(&repo_server_data_folder).map_err(|e| {
            error!("Failed to create repo_server_data_folder: {}, err: {}", repo_server_data_folder.display(), e);
            anyhow::anyhow!("Failed to create repo_server_data_folder: {}, err: {}", repo_server_data_folder.display(), e)
        })?;
        info!("Created repo_server_data_folder: {}", repo_server_data_folder.display());
    }

    let repo_server = RepoServer::new(repo_service_settings).await
      .map_err(|e| {
        error!("repo service init error! err:{}", e);
        anyhow::anyhow!("repo service init error! err:{}", e)
      })?;
    
    repo_server.init().await?;
    info!("repo service init check OK, start server...");

    //start!
    let runner = Runner::new(REPO_SERVICE_MAIN_PORT);
    runner.add_http_server("/kapi/repo-service".to_string(),Arc::new(repo_server));
    runner.run().await;


    Ok(())
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}

