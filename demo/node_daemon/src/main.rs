mod backup;
mod run_item;
mod service_mgr;
mod pkg_mgr;
mod system_config;

use etcd_client::*;
use log::*;
use serde::Deserialize;
use serde_json::error;
use simplelog::*;
use std::{collections::HashMap, fs::File};
use tokio::*;
use toml;

use name_client::NameClient;
use crate::run_item::*;
use crate::service_mgr::*;
use crate::system_config::*;

use thiserror::Error;

#[derive(Error, Debug)]
enum NodeDaemonErrors {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("Config file read error: {0}")]
    ReadConfigError(String),
    #[error("Config parser error: {0}")]
    ParserConfigError(String),
    #[error("SystemConfig Error: {0}")]
    SystemConfigError(String), //key
                               // 其他错误类型
}

type Result<T> = std::result::Result<T, NodeDaemonErrors>;

#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    owner_zone_id: String,
    node_id: String,
    //node_pubblic_key : String,
    //node_private_key : String,
}

struct ZoneConfig {
    zone_id: String,
    zone_public_key: String,
    etcd_servers: Vec<String>, //etcd server endpoints
    etcd_data_version: u64,    //last backup etcd data version, 0 is not backup
    backup_server_id: Option<String>,
}

enum EtcdState {
    Good(String),                 //string is best node_name have etcd for this node
    Error(String),                //string is error message
    NeedRunInThisMachine(String), //string is the endpoint info
}

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new().build();

    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(
            LevelFilter::Info,
            config.clone(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        // 同时将日志输出到文件
        WriteLogger::new(
            LevelFilter::Info,
            config,
            File::create("node_daemon.log").unwrap(),
        ),
    ])
    .unwrap();
}

fn load_identity_config() -> Result<NodeIdentityConfig> {
    // load from /etc/buckyos/node_identity.toml
    let file_path = "/etc/buckyos/node_identity.toml";
    let contents = std::fs::read_to_string(file_path).map_err(|err| {
        error!("read node identity config failed!");
        return NodeDaemonErrors::ReadConfigError(String::from(file_path));
    })?;

    let config: NodeIdentityConfig = toml::from_str(&contents).map_err(|err| {
        error!("parse node identity config failed!");
        return NodeDaemonErrors::ParserConfigError(format!(
            "Failed to parse NodeIdentityConfig TOML: {}",
            err
        ));
    })?;

    Ok(config)
}

async fn looking_zone_config(node_cfg: &NodeIdentityConfig) -> Result<ZoneConfig> {
    let name_client = NameClient::new();
    let name_info = name_client.query(node_cfg.owner_zone_id.as_str()).await.map_err(|err|{
        error!("query zone config failed! {}", err);
        return NodeDaemonErrors::ReasonError("query zone config failed!".to_string());
    })?;

    let zone_config: Option<name_client::ZoneConfig> = name_info.get_extra().map_err(|err|{
        error!("get zone config failed! {}", err);
        return NodeDaemonErrors::ReasonError("get zone config failed!".to_string());
    })?;

    if let Some(zone_cfg) = zone_config {
        Ok(ZoneConfig {
            zone_id: node_cfg.node_id.clone(),
            zone_public_key: "".to_string(),
            etcd_servers: zone_cfg.etcds.iter().map(|v| v.name.clone()).collect(),
            etcd_data_version: 0,
            backup_server_id: zone_cfg.backup_server,
        })
    } else {
        Err(NodeDaemonErrors::ReasonError("zone config not found!".to_string()))
    }
    //get name service client
    //config =  client.lookup($zone_id)
    //parser config
    //if have backup server, connect to backupserver and get backup info, get etcd_data_version
}

async fn check_etcd_by_zone_config(
    config: &ZoneConfig,
    node_config: &NodeIdentityConfig,
) -> Result<EtcdState> {
    let node_id = &node_config.node_id;
    let local_endpoint = config
        .etcd_servers
        .iter()
        .find(|&server| server.contains(node_id));

    if let Some(endpoint) = local_endpoint {
        match EtcdClient::connect(endpoint).await {
            Ok(_) => Ok(EtcdState::Good(node_id.clone())),
            Err(_) => Ok(EtcdState::NeedRunInThisMachine(node_id.clone())),
        }
    } else {
        for endpoint in &config.etcd_servers {
            if EtcdClient::connect(endpoint).await.is_ok() {
                return Ok(EtcdState::Good(endpoint.clone()));
            }
        }
        Ok(EtcdState::Error("No etcd servers available".to_string()))
    }
}

async fn check_etcd_data() -> Result<bool> {
    unimplemented!();
}

async fn get_etcd_data_version() -> Result<u64> {
    unimplemented!();
}

async fn try_start_etcd() -> Result<()> {
    unimplemented!();
}

async fn try_restore_etcd(node_cfg: &NodeIdentityConfig, zone_cfg: &ZoneConfig) -> Result<()> {
    //backup_server_client.open()
    //backup_info = backup_server_client.restore_meta("zone_backup")
    //backup_server_client.restore_chunk_list("etcd_data." + backup_info.etcd_data_version,local_dir)
    //unpack chunkdata to etcd data dir
    unimplemented!();
}

//fn execute_docker(docker_config)   -> Result<(), Box<dyn std::error::Error>>{
//    for docker_instance in docker_config {
//尝试启动/停止镜像
//启动镜像前，需要通知zone内的docker repo先更新必要的镜像。该过程和docekr repo的实现是解耦合的，后续可以用
//    }
//}

//fn execute_service(service_config)  -> Result<(), Box<dyn std::error::Error>>{
//    for service_instance in service_config {
//service一定不跑在docker里
//尝试启动/停止/更新服务

//    }
//}

async fn node_daemon_main_loop(node_cfg: &NodeIdentityConfig, config: &ZoneConfig) -> Result<()> {
    let mut loop_step = 0;
    let mut is_running = true;
    loop {
        if is_running == false {
            break;
        }
        loop_step += 1;
        info!("node daemon main loop step:{}", loop_step);
        //etcd_client = create_etcd_client()
        //system_config.init(etcd_client)
        let sys_cfg = SystemConfig::new(None);
        let node_ip = String::from("127.0.0.1");
        //try_backup_etcd_data()

        //try_report_node_status()

        //cmd_config = system_config.get("")
        //execute_cmd(cmd_config) //一般是执行运维命令，类似系统备份和恢复
        //system config是以node_id为单位配置的
        //以服务为单位的配置一般从name service那展开
        let service_cfg_key = format!("/{}/services", node_cfg.node_id);
        let all_service_item = sys_cfg.list(&service_cfg_key).await.map_err(|err| {
            error!("list service config failed!");
            return NodeDaemonErrors::SystemConfigError(service_cfg_key);
        })?;
       
        for (service_name, service_cfg) in all_service_item {
            //parse servce_cfg to json，get target state from service_cfg
            let (service_config,_) = sys_cfg.get(&service_name.as_str()).await.unwrap();
            let mut run_params = RunItemParams::new(node_ip.clone());
            run_params.services_cfg = Some(service_config);

            let target_state: RunItemTargetState = RunItemTargetState::Running(String::new());
            let service_item = create_service_item_from_config(&service_cfg).await.map_err(|err|{
                error!("create service item from config failed!");
                return NodeDaemonErrors::SystemConfigError(service_cfg_key);
            })?;
            

            control_run_item_to_target_state(&service_item, target_state, Some(&run_params)).await.map_err(|err|{
                error!("control service item to target state failed!");
                return NodeDaemonErrors::SystemConfigError(service_cfg_key);
            })?;
            //get service item from service_cfg
            //query service item state
            // if service item state is not equal to service_cfg state, do something
        }

        //service_config = system_config.get("")
        //execute_service(service_config)
        //vm_config = system_config.get("")
        //execute_vm(vm_config)
        //docker_config = system_config.get("")
        //execute_docker(docker_config)
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    init_log_config();
    info!("node_dameon start...");

    let node_identity = load_identity_config().map_err(|err| {
        error!("load node identity config failed!");
        String::from("load node identity config failed!")
    })?;

    info!(
        "zone_id : {}, node_id is:{}",
        node_identity.owner_zone_id, node_identity.node_id
    );

    let zone_config = looking_zone_config(&node_identity).await.map_err(|err| {
        error!("looking zone config failed!");
        String::from("looking zone config failed!")
    })?;

    //检查etcd状态
    let etcd_state = check_etcd_by_zone_config(&zone_config, &node_identity)
        .await
        .map_err(|err| {
            error!("check etcd by zone config failed!");
            return String::from("check etcd by zone config failed!");
        })?;

    match etcd_state {
        EtcdState::Good(node_name) => {
            info!("etcd service is good, node:{} is my server.", node_name);
        }
        EtcdState::Error(err_msg) => {
            error!("etcd is error, err_msg:{}", err_msg);
            return Err(String::from("etcd is error!"));
        }
        EtcdState::NeedRunInThisMachine(endpoint) => {
            info!("etcd need run in this machine, endpoint:{}", endpoint);
            let etcd_data_version = get_etcd_data_version().await.map_err(|err| {
                error!("get etcd data version failed!");
                return String::from("get etcd data version failed!");
            })?;

            if etcd_data_version < zone_config.etcd_data_version {
                info!("local etcd data version is old, wait for etcd restore!");
                try_restore_etcd(&node_identity, &zone_config)
                    .await
                    .map_err(|err| {
                        error!("try restore etcd failed!");
                        return String::from("try restore etcd failed!");
                    })?;
            }

            try_start_etcd().await.map_err(|err| {
                error!("try start etcd failed!");
                return String::from("try start etcd failed!");
            })?;
        }
    }

    info!("Ready, start node daemon main loop!");
    node_daemon_main_loop(&node_identity, &zone_config)
        .await
        .map_err(|err| {
            error!("node daemon main loop failed!");
            return String::from("node daemon main loop failed!");
        })?;

    Ok(())
}
