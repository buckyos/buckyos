#![allow(dead_code)]
#![allow(unused)]

// mod backup;
// mod backup_task_mgr;
// mod backup_task_storage;
mod etcd_mgr;
mod pkg_mgr;
mod run_item;
mod service_mgr;
mod system_config;

use etcd_client::*;
use futures::prelude::*;
use log::*;
use serde::{Deserialize, Serialize};
// use serde_json::error;
use simplelog::*;
use std::fmt::format;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::{collections::HashMap, fs::File};
// use tokio::*;
use toml;

// use crate::backup::*;
// use crate::backup_task_mgr::*;
use crate::etcd_mgr::*;
use crate::run_item::*;
use crate::service_mgr::*;
use crate::system_config::*;
use gateway_lib::DeviceEndPoint;
use name_client::NameClient;

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
}

type Result<T> = std::result::Result<T, NodeDaemonErrors>;

#[derive(Deserialize, Debug)]
struct NodeIdentityConfig {
    owner_zone_id: String,
    node_id: String,
    //node_pubblic_key : String,
    //node_private_key : String,
}

//load from SystemConfig,node的配置分为下面几个部分
// 固定的硬件配置，一般只有硬件改变或损坏才会修改
// 系统资源情况，（比如可用内存等），改变密度很大。这一块暂时不用etcd实现，而是用专门的监控服务保存
// RunItem的配置。这块由调度器改变，一旦改变,node_daemon就会产生相应的控制命令
// Task(Cmd)配置，暂时不实现
#[derive(Serialize, Deserialize, Debug)]
struct NodeConfig {
    revision: u64,
    services: HashMap<String, ServiceConfig>,
}

impl NodeConfig {
    fn from_json_str(jons_str: &str) -> Result<Self> {
        let node_config: std::result::Result<NodeConfig, serde_json::Error> =
            serde_json::from_str(jons_str);
        if node_config.is_ok() {
            return Ok(node_config.unwrap());
        }
        return Err(NodeDaemonErrors::ParserConfigError(
            "Failed to parse NodeConfig JSON".to_string(),
        ));
    }
}

#[derive(Serialize, Deserialize, Debug)]
struct ZoneConfig {
    zone_id: String,
    //zone_public_key: String,
    etcd_servers: Vec<String>, //etcd server endpoints
    etcd_data_version: i64,    //last backup etcd data version, 0 is not backup
    backup_server_id: Option<String>,
}

//load from SystemConfig
//struct ZoneInnerConfig {
//service configs
//}

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
    let file_path = "node_identity.toml";
    let contents = std::fs::read_to_string(file_path).map_err(|err| {
        error!("read node identity config failed! {}", err);
        return NodeDaemonErrors::ReadConfigError(String::from(file_path));
    })?;

    let config: NodeIdentityConfig = toml::from_str(&contents).map_err(|err| {
        error!("parse node identity config failed! {}", err);
        return NodeDaemonErrors::ParserConfigError(format!(
            "Failed to parse NodeIdentityConfig TOML: {}",
            err
        ));
    })?;

    Ok(config)
}

async fn looking_zone_config(node_cfg: &NodeIdentityConfig) -> Result<ZoneConfig> {
    //如果本地文件存在则优先加载本地文件
    let json_config_path = format!("{}_zone_config.json", node_cfg.owner_zone_id);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let zone_config = serde_json::from_str(&json_config.unwrap());
        if zone_config.is_ok() {
            warn!(
                "load zone config from ./{}_zone_config.json success!",
                node_cfg.owner_zone_id
            );
            return Ok(zone_config.unwrap());
        }
    }
    info!("no local zone_config file found, try query from name service");

    let name_client = NameClient::new();
    let name_info = name_client
        .query(node_cfg.owner_zone_id.as_str())
        .await
        .map_err(|err| {
            error!("query zone config failed! {}", err);
            return NodeDaemonErrors::ReasonError("query zone config failed!".to_string());
        })?;

    let zone_config: Option<name_client::ZoneConfig> = name_info.get_extra().map_err(|err| {
        error!("get zone config failed! {}", err);
        return NodeDaemonErrors::ReasonError("get zone config failed!".to_string());
    })?;

    if let Some(zone_cfg) = zone_config {
        Ok(ZoneConfig {
            zone_id: node_cfg.owner_zone_id.clone(),
            //zone_public_key: "".to_string(),
            etcd_servers: zone_cfg.etcds.iter().map(|v| v.name.clone()).collect(),
            etcd_data_version: 0,
            backup_server_id: zone_cfg.backup_server,
        })
    } else {
        Err(NodeDaemonErrors::ReasonError(
            "zone config not found!".to_string(),
        ))
    }
    //get name service client
    //config =  client.lookup($zone_id)
    //parser config
    //if have backup server, connect to backupserver and get backup info, get etcd_data_version
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

async fn get_node_config(
    node_identity: &NodeIdentityConfig,
    sys_cfg: Arc<&SystemConfig>,
) -> Result<NodeConfig> {
    //首先尝试加载本地文件，如果本地文件存在则返回
    let json_config_path = format!("{}_node_config.json", node_identity.node_id);
    let json_config = std::fs::read_to_string(json_config_path);
    if json_config.is_ok() {
        let node_config = NodeConfig::from_json_str(&json_config.unwrap());
        if node_config.is_ok() {
            warn!(
                "load node config from ./{}_node_config.json success!",
                node_identity.node_id
            );
            return node_config;
        }
    }

    // key: nodes/$node_id
    //尝试通过system_config加载，加载成功更新缓存，失败则尝试使用缓存中的数据
    let sys_node_key = format!("nodes/{}", node_identity.node_id);
    // 从etcd中读取
    let sys_cfg_result = sys_cfg.get(&sys_node_key).await;
    if sys_cfg_result.is_err() {
        return Err(NodeDaemonErrors::ReasonError(
            "get node config failed from etcd!".to_string(),
        ));
    }
    let result = sys_cfg_result.as_ref().unwrap();
    let revision = result.1 as u64;
    let config: HashMap<String, serde_json::Value> = serde_json::from_str(result.0.as_str())
        .map_err(|e| {
            NodeDaemonErrors::ReasonError("get node config from etcd and parse failed!".to_string())
        })?;
    let services = config.get("services");
    if services.is_none() {
        return Err(NodeDaemonErrors::ReasonError(
            "get node config from etcd and parse failed!".to_string(),
        ));
    }
    let services: std::result::Result<HashMap<String, ServiceConfig>, serde_json::Error> =
        serde_json::from_value(services.unwrap().clone());
    if services.is_err() {
        return Err(NodeDaemonErrors::ReasonError(
            "get node config from etcd and parse failed!".to_string(),
        ));
    }
    let node_config = NodeConfig {
        revision: revision,
        services: services.unwrap(),
    };
    warn!("load node config from system_config success!",);
    return Ok(node_config);

    // //无法的找到node_config,返回错误
    // return Err(NodeDaemonErrors::ReasonError(
    //     "get node config failed!".to_string(),
    // ));
}

//start gateway for etcd cluster
async fn start_gateway_by_zone_config(
    zone_config: &ZoneConfig,
    current_node_id: &str,
) -> Result<()> {
    //get gateway config from zone_config
    let mut have_wan_node = false;
    let mut have_lan_node = false;
    let mut this_node_is_lan = false;
    let mut this_node = None;
    let mut lan_nodes = vec![];
    let mut wan_nodes = vec![];
    for etcd_server in zone_config.etcd_servers.iter() {
        let server_ep = DeviceEndPoint::from_str(etcd_server)
            .map_err(|err| return NodeDaemonErrors::ParserConfigError(err))?;
        if server_ep.nat_id.is_some() {
            if server_ep.device_name == current_node_id {
                this_node_is_lan = true;

                assert!(this_node.is_none());
                this_node = Some(server_ep);
            } else {
                lan_nodes.push(server_ep);
            }
            have_lan_node = true;
        } else {
            if server_ep.device_name == current_node_id {
                this_node_is_lan = false;

                assert!(this_node.is_none());
                this_node = Some(server_ep);
            } else {
                wan_nodes.push(server_ep);
            }
            have_wan_node = true;
        }
    }

    // gen gateway config now
    assert!(this_node.is_some());
    let this_node = this_node.unwrap();

    // Add current node config at first
    let mut gateway_config = gateway_lib::ConfigGen::new(
        current_node_id,
        if this_node_is_lan {
            gateway_lib::PeerAddrType::LAN
        } else {
            gateway_lib::PeerAddrType::WAN
        },
        0,
    );

    // Add etcd service as upstream
    let etcd_port = this_node.port.unwrap();
    gateway_config.add_upstream_service("etcd", "tcp", "127.0.0.1", etcd_port);

    // Add other nodes etcd service via tcp forward proxy
    for lan_node in lan_nodes {
        let etcd_port = lan_node.port.unwrap();

        gateway_config.add_device(&lan_node.device_name, None, None, Some(gateway_lib::PeerAddrType::LAN));
        gateway_config.add_forward_proxy(
            format!("{}-etcd", lan_node.device_name),
            "tcp",
            "127.0.0.1",
            etcd_port,
            lan_node.device_name,
            etcd_port,
        );
    }
    for wan_node in wan_nodes {
        let etcd_port = wan_node.port.unwrap();

        gateway_config.add_device(&wan_node.device_name, None, None, Some(gateway_lib::PeerAddrType::WAN));
        gateway_config.add_forward_proxy(
            format!("{}-etcd", wan_node.device_name),
            "tcp",
            "127.0.0.1",
            etcd_port,
            wan_node.device_name,
            etcd_port,
        );
    }

    let cmd = gateway_config.gen();
    info!("Gateway config: {}", cmd);

    // Try start gateway
    if have_wan_node == have_lan_node {
        if this_node_is_lan {
            //TODO:start gateway, and register on WAN nodes and start passive port forward
            warn!("start gateway, and register on WAN nodes and start passive port forward")
        } else {
            //TODO:start gateway, and start port forward to LAN nodes
            warn!("start gateway, and start port forward to LAN nodes")
        }
    } else {
        warn!("all node in some NAT, no gateway needed!");
    }

    Ok(())
}

async fn node_main(node_identity: &NodeIdentityConfig, zone_config: &ZoneConfig) -> Result<()> {
    // node_id对应的上，就用nodeid作为访问方式，对应不上就直接连local的etcd
    let node_id = node_identity.node_id.clone();
    let local_endpoint = zone_config
        .etcd_servers
        .iter()
        .find(|&server| server.starts_with(&node_id))
        .map_or_else(
            || "http://127.0.0.1:2379".to_string(),
            |server| parse_etcd_url(server.to_string()).unwrap().0,
        );

    info!("node_main local_endpoint:{}", local_endpoint);

    let sys_cfg = SystemConfig::new(&vec![local_endpoint])
        .await
        .map_err(|_| {
            error!("SystemConfig init failed!");
            NodeDaemonErrors::SystemConfigError("".to_string())
        })?;
    let sys_cfg = Arc::new(&sys_cfg);
    let node_config = get_node_config(node_identity, Arc::clone(&sys_cfg)).await?;

    //try_backup_etcd_data()
    etcd_mgr::try_report_node_status(node_identity, Arc::clone(&sys_cfg));

    //cmd_config = load_node_cmd_config()
    //execute_cmd(cmd_config) //一般是执行运维命令，类似系统备份和恢复,由node_ctl负责执行

    let service_stream = stream::iter(node_config.services);
    service_stream
        .for_each_concurrent(1, |(service_name, service_cfg)| async move {
            let target_state = service_cfg.target_state.clone();
            let _ = control_run_item_to_target_state(&service_cfg, target_state, None)
                .await
                .map_err(|_err| {
                    error!("control service item to target state failed!");
                    return NodeDaemonErrors::SystemConfigError(service_name.clone());
                });
        })
        .await;

    //service_config = system_config.get("")
    //execute_service(service_config)
    //vm_config = system_config.get("")
    //execute_vm(vm_config)
    //docker_config = system_config.get("")
    //execute_docker(docker_config)
    Ok(())
}

async fn node_daemon_main_loop(
    node_identity: &NodeIdentityConfig,
    zone_config: Arc<ZoneConfig>,
) -> Result<()> {
    let zone_config = zone_config.as_ref();
    let mut loop_step = 0;
    let mut is_running = true;

    loop {
        if is_running == false {
            break;
        }
        loop_step += 1;
        info!("node daemon main loop step:{}", loop_step);

        let node_main_result = node_main(node_identity, zone_config).await;
        match node_main_result {
            Ok(_) => {
                info!("node_main success!");
            }
            Err(err) => {
                error!("node_main failed! {}", err);
            }
        }
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }
    Ok(())
}

#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    init_log_config();
    info!("node_dameon start...");

    let node_identity = load_identity_config().map_err(|err| {
        error!("load node identity config failed! {}", err);
        String::from("load node identity config failed!")
    })?;

    info!(
        "zone_id : {}, node_id is:{}",
        node_identity.owner_zone_id, node_identity.node_id
    );

    let zone_config = looking_zone_config(&node_identity).await.map_err(|err| {
        error!("looking zone config failed! {}", err);
        String::from("looking zone config failed!")
    })?;
    info!("zone config: {:?}", zone_config);

    start_gateway_by_zone_config(&zone_config, &node_identity.node_id.as_str())
        .await
        .map_err(|err| {
            error!("start gateway by zone config failed!,{}", err);
            return String::from("start gateway by zone config failed!");
        })?;

    //检查etcd状态
    let etcd_state = check_etcd_by_zone_config(&zone_config, &node_identity)
        .await
        .map_err(|_err| {
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
            let etcd_data_version = etcd_mgr::get_etcd_data_version(&node_identity, &zone_config)
                .await
                .map_err(|_err| {
                    error!("get etcd data version failed!");
                    return String::from("get etcd data version failed!");
                })?;

            if etcd_data_version < zone_config.etcd_data_version {
                info!("local etcd data version is old, wait for etcd restore!");
                etcd_mgr::try_restore_etcd(&node_identity, &zone_config)
                    .await
                    .map_err(|_err| {
                        error!("try restore etcd failed!");
                        return String::from("try restore etcd failed!");
                    })?;
            }

            try_start_etcd(&node_identity, &zone_config)
                .await
                .map_err(|_err| {
                    error!("try start etcd failed!");
                    return String::from("try start etcd failed!");
                })?;
        }
    }

    // let node_identity = Arc::new(node_identity);
    let zone_config = Arc::new(zone_config);
    let zc_clone = Arc::clone(&zone_config);
    // 创建一个新线程来处理备份逻辑
    // std::thread::spawn(move || system_config_backup(zc_clone));

    info!("Ready, start node daemon main loop!");
    node_daemon_main_loop(&node_identity, zone_config)
        .await
        .map_err(|err| {
            error!("node daemon main loop failed! {}", err);
            return String::from("node daemon main loop failed!");
        })?;

    Ok(())
}
