use log::*;
use simplelog::*;
use std::fs::File;
use tokio::*;
use thiserror::Error;

#[derive(Error, Debug)]
enum NodeDaemonErrors {
    #[error("Failed due to reason: {0}")]
    ReasonError(String),
    #[error("File not found: {0}")]
    FileNotFoundError(String),
    // 其他错误类型
}

type Result<T> = std::result::Result<T, NodeDaemonErrors>;

struct NodeIdentityConfig {
    node_id : String,
    node_pubblic_key : String,
}

struct ZoneConfig {
    etcd_version : u64,
}

enum EtcdState {
    Good(String),//string is best node_name have etcd for this node 
    Error(String),//string is error message
    NeedRunInThisMachine(String),//string is the endpoint info

}

fn init_log_config() {
    // 创建一个日志配置对象
    let config = ConfigBuilder::new().build();

    // 初始化日志器
    CombinedLogger::init(vec![
        // 将日志输出到标准输出，例如终端
        TermLogger::new(LevelFilter::Info, config.clone(), TerminalMode::Mixed, ColorChoice::Auto),
        // 同时将日志输出到文件
        WriteLogger::new(LevelFilter::Info, config, File::create("node_daemon.log").unwrap())
    ]).unwrap();
}

fn load_identity_config() -> Result<NodeIdentityConfig> {
    // load from /etc/buckyos/node_identity.toml
    unimplemented!();
} 

async fn looking_zone_config() -> Result<ZoneConfig> {
    
    unimplemented!();
}


async fn check_etcd_by_zone_config(config:&ZoneConfig) -> Result<EtcdState> {
    unimplemented!();
}


async fn check_etcd_data()->Result<bool> {
    unimplemented!();
}

async fn get_etcd_data_version()->Result<u64> {
    unimplemented!();
}

async fn try_start_etcd()->Result<()> {
    unimplemented!();
}

async fn try_restore_etcd()->Result<()> {
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


async fn node_daemon_main_loop(config:ZoneConfig) -> Result<()> {
    //etcd_client = create_etcd_client()
    //etcd_client.refresh_config()
    //system_config.init()
    //cmd_config = system_config.get("")
    //execute_cmd(cmd_config) //一般是执行运维命令，类似系统备份和恢复
    //service_config = system_config.get("")
    //execute_service(service_config)
    //vm_config = system_config.get("")
    //execute_vm(vm_config)
    //docker_config = system_config.get("")
    //execute_docker(docker_config)
    unimplemented!();
}


#[tokio::main]
async fn main() -> std::result::Result<(), String> {
    init_log_config();
    info!("node_dameon start!");

    let node_identity = load_identity_config().map_err(|err|{
        error!("load node identity config failed!");
        String::from("load node identity config failed!")
    })?;

    let zone_config = looking_zone_config().await.map_err(|err|{
        error!("looking zone config failed!");
        String::from("looking zone config failed!")
    })?;
    
    //检查etcd状态
    let etcd_state = check_etcd_by_zone_config(&zone_config).await.map_err(|err|{
        error!("check etcd by zone config failed!");
        return String::from("check etcd by zone config failed!");
    })?;

    match etcd_state {
        EtcdState::Good(node_name) => {
            info!("etcd service is good, node:{} is my server.", node_name);
        },
        EtcdState::Error(err_msg) => {
            error!("etcd is error, err_msg:{}", err_msg);
            return Err(String::from("etcd is error!"));
        },
        EtcdState::NeedRunInThisMachine(endpoint) => {
            info!("etcd need run in this machine, endpoint:{}", endpoint);
            let etcd_data_version = get_etcd_data_version().await.map_err(|err|{
                error!("get etcd data version failed!");
                return String::from("get etcd data version failed!");
            })?;

            if etcd_data_version < zone_config.etcd_version {
                info!("local etcd data version is old, wait for etcd restore!");
                try_restore_etcd().await.map_err(|err|{
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


    node_daemon_main_loop(zone_config).await.map_err(|err|{
        error!("node daemon main loop failed!");
        return String::from("node daemon main loop failed!");
    })?;

    Ok(())
}
