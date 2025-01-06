#![allow(unused, dead_code)]

mod crypto_utils;
mod def;
mod downloader;
mod index_publisher;
mod repo_server;
mod source_manager;
mod source_node;
mod task_manager;
use package_lib::PackageId;

use crate::def::*;
use crate::repo_server::RepoServer;
use std::fs::File;
use sys_config::{SystemConfigClient, SystemConfigError};

use log::*;
use serde_json::{json, Value};
use simplelog::*;

use buckyos_kit::*;
use cyfs_gateway_lib::WarpServerConfig;
use cyfs_warp::*;

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new()
        .set_time_format_custom(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
        ))
        .build();

    let log_path = get_buckyos_root_dir().join("logs").join("repo_service.log");
    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(LevelFilter::Info, config, File::create(log_path).unwrap()),
    ])
    .unwrap();
}

async fn service_main() {
    init_log_config();

    let repo_server = RepoServer::new().await.unwrap();
    register_inner_service_builder("repo_server", move || Box::new(repo_server.clone())).await;

    let repo_server_dir = get_buckyos_system_bin_dir().join("repo");
    let repo_server_config = json!({
      "tls_port":4010,
      "http_port":4000,
      "hosts": {
        "*": {
          "enable_cors":true,
          "routes": {
            "/": {
              "local_dir": repo_server_dir.to_str().unwrap()
            },
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
}

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(service_main());
}

#[cfg(test)]
mod test {}
