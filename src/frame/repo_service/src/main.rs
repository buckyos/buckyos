#![allow(unused, dead_code)]


mod repo_server;
mod pub_task_mgr;
use crate::repo_server::*;
use std::fs::File;
use buckyos_api::*;

use log::*;
use serde_json::*;

use buckyos_kit::*;
use name_client::*;
use cyfs_gateway_lib::WarpServerConfig;
use cyfs_warp::*;

use anyhow::Result;

async fn service_main() -> Result<()> {
    init_logging("repo_service");
    init_buckyos_api_runtime("repo_service",None,BuckyOSRuntimeType::FrameService).await?;
    let mut runtime = get_buckyos_api_runtime()?;
    let login_result = runtime.login(None,None).await;
    if  login_result.is_err() {
        error!("repo service login to system failed! err:{:?}", login_result);
        return Err(anyhow::anyhow!("repo service login to system failed! err:{:?}", login_result));
    }
    let repo_service_settings = runtime.get_my_settings().await
      .map_err(|e| {
        error!("repo service settings not found! err:{}", e);
        anyhow::anyhow!("repo service settings not found! err:{}", e)
      })?;
    let repo_service_settings: RepoServerSetting = serde_json::from_value(repo_service_settings).map_err(|e| {
      error!("repo service settings parse error! err:{}", e);
      anyhow::anyhow!("repo service settings parse error! err:{}", e)
    })?;

    let repo_server = RepoServer::new(repo_service_settings).await
      .map_err(|e| {
        error!("repo service init error! err:{}", e);
        anyhow::anyhow!("repo service init error! err:{}", e)
      })?;


    register_inner_service_builder("repo_server", move || Box::new(repo_server.clone())).await;
    //let repo_server_dir = get_buckyos_system_bin_dir().join("repo");
    let repo_server_config = json!({
      "http_port":4000,//TODO：服务的端口分配和管理
      "hosts": {
        "*": {
          "enable_cors":true,
          "routes": {
            "/kapi/repo" : {
                "inner_service":"repo_server"
            }
          }
        }
      }
    });

    let repo_server_config: WarpServerConfig = serde_json::from_value(repo_server_config).unwrap();
    //start!
    info!("start repo service...");
    start_cyfs_warp_server(repo_server_config).await;

    let _ = tokio::signal::ctrl_c().await;
    Ok(())
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}

#[cfg(test)]
mod test {}
