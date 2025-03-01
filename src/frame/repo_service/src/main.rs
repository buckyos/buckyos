#![allow(unused, dead_code)]


mod repo_server;

use crate::repo_server::*;
use std::fs::File;
use sys_config::{SystemConfigClient, SystemConfigError};

use log::*;
use serde_json::*;

use buckyos_kit::*;
use name_client::*;
use cyfs_gateway_lib::WarpServerConfig;
use cyfs_warp::*;

use anyhow::Result;

async fn service_main() -> Result<()> {
    init_logging("repo_service");

    init_global_buckyos_value_by_env("REPO_SERVICE");
    let rpc_session_token = std::env::var("REPO_SERVICE_SESSION_TOKEN").map_err(|e| {
      error!("repo service session token not found! err:{}", e);
      anyhow::anyhow!("repo service session token not found! err:{}", e)
    })?;
    //TODO: 到verify-hub login获得更统一的session_token?

    let sys_config_client = SystemConfigClient::new(None, Some(rpc_session_token.as_str()));
    let (repo_service_settings, _) = sys_config_client.get("services/repo_service/settings").await
      .map_err(|e| {
        error!("repo service settings not found! err:{}", e);
        anyhow::anyhow!("repo service settings not found! err:{}", e)
      })?;
    let repo_service_settings: RepoServerSetting = serde_json::from_str(repo_service_settings.as_str()).map_err(|e| {
      error!("repo service settings parse error! err:{}", e);
      anyhow::anyhow!("repo service settings parse error! err:{}", e)
    })?;

    let repo_server = RepoServer::new(repo_service_settings,Some(rpc_session_token)).await
      .map_err(|e| {
        error!("repo service init error! err:{}", e);
        anyhow::anyhow!("repo service init error! err:{}", e)
      })?;
    repo_server.init().await.map_err(|e| {
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
