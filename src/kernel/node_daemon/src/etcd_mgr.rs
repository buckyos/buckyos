use crate::*;
use backup_lib::{
    CheckPointVersion, SimpleChunkMgrSelector, SimpleFileMgrSelector, SimpleTaskMgrSelector,
    TaskKey,
};
use std::sync::Arc;
use std::{net::TcpStream, thread};
use tokio::time::{sleep, Duration};

const BACKUP_STORAGE_DIR: &'static str = "/tmp/backup";
const ETCD_BACKUP_TASK_KEY: &str = "backup.etcd";

macro_rules! handle_error {
    ($prefix:expr) => {{
        move |err| {
            let err_msg = format!("{} {}", $prefix, err);
            error!("{}", err_msg);
            NodeDaemonErrors::ReasonError(err_msg.to_string())
        }
    }};
}

pub(crate) fn parse_etcd_url(server: String) -> Result<(String, String, String)> {
    let machine = server;
    let client_port: i32 = 2379;
    let peer_port = 2380;

    let client_url = format!("http://127.0.0.1:{}", client_port);
    let peer_url = format!("http://{}:{}", machine, peer_port);

    // --initial-cluster 参数应该是：
    // m1=http://m1:2380,m2=http://m2:2380,m3=http://m3:2380
    let cluster_url = format!("{}=http://{}:{}", machine, machine, peer_port);

    Ok((client_url, peer_url, cluster_url))
}

pub(crate) fn parse_initial_cluster(etcd_servers: Vec<String>) -> String {
    etcd_servers
        .iter()
        .map(|server| {
            let result = parse_etcd_url(server.to_string()).unwrap();
            result.2
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub(crate) async fn check_etcd_by_zone_config(
    config: &ZoneConfig,
    node_config: &NodeIdentityConfig,
) -> Result<EtcdState> {
    let node_id = &node_config.node_id;

    // ping local 2379
    // let timeout = Duration::from_secs(1);
    // if TcpStream::connect_timeout(&"127.0.0.1:2379".parse().unwrap(), timeout).is_ok() {
    //     info!("local etcd is running ");
    //     return Ok(EtcdState::Good(node_id.clone()));
    // }

    let local_server = config
        .etcd_servers
        .iter()
        .find(|&server| server.starts_with(node_id));
    info!(
        "check local etcd, local node id {:?}, local_endpoint:{:?}",
        node_id, local_server
    );

    if let Some(local_server) = local_server {
        let result = parse_etcd_url(local_server.to_string()).unwrap();
        let endpoint = result.0;
        info!(
            "Found etcd server should run on this machine:{} ,try connect to local etcd.",
            endpoint
        );
        match EtcdClient::connect(&endpoint).await {
            Ok(_) => Ok(EtcdState::Good(node_id.clone())),
            Err(_) => Ok(EtcdState::NeedRunInThisMachine(node_id.clone())),
        }
    } else {
        for endpoint in &config.etcd_servers {
            info!("Try connect to etcd server:{}", endpoint);
            if EtcdClient::connect(&endpoint).await.is_ok() {
                return Ok(EtcdState::Good(endpoint.clone()));
            }
        }
        Ok(EtcdState::Error("No etcd servers available".to_string()))
    }
}

pub(crate) async fn get_etcd_data_version(
    node_cfg: &NodeIdentityConfig,
    zone_cfg: &ZoneConfig,
) -> Result<i64> {
    let name = node_cfg.node_id.clone();
    let zone = zone_cfg.zone_id.clone();
    let initial_cluster = parse_initial_cluster(zone_cfg.etcd_servers.clone());

    etcd_client::start_etcd(&name, &initial_cluster, &zone)
        .map_err(handle_error!("start_etcd failed!"))?;
    sleep(Duration::from_secs(1)).await;

    // etcd要在cluster配置的所有节点都就绪后才能正常工作
    if !etcd_client::check_etcd_health(&name).await {
        error!("etcd is not healthy");
        return Err(NodeDaemonErrors::ReasonError(
            "etcd is not healthy".to_string(),
        ));
    }

    let revision = etcd_client::get_etcd_data_version()
        .await
        .map_err(handle_error!("get_etcd_data_version failed!"))
        .unwrap();
    info!("get_etcd_data_version:{}", revision);
    Ok(revision)
}

pub(crate) async fn check_etcd_data() -> Result<bool> {
    unimplemented!();
}

pub(crate) async fn try_start_etcd(
    node_cfg: &NodeIdentityConfig,
    zone_cfg: &ZoneConfig,
) -> Result<()> {
    let name = node_cfg.node_id.clone();
    let initial_cluster = parse_initial_cluster(zone_cfg.etcd_servers.clone());

    etcd_client::start_etcd(&name, &initial_cluster, zone_cfg.zone_id.as_str())
        .map_err(handle_error!("try start etcd failed"))?;
    Ok(())
}

pub(crate) async fn try_restore_etcd(
    _node_cfg: &NodeIdentityConfig,
    zone_cfg: &ZoneConfig,
) -> Result<()> {
    let task_key = TaskKey::from(ETCD_BACKUP_TASK_KEY);
    let chunk_mgr_selector =
        SimpleChunkMgrSelector::new(zone_cfg.backup_server_id.as_ref().unwrap());
    let file_mgr_selector = SimpleFileMgrSelector::new(zone_cfg.backup_server_id.as_ref().unwrap());
    let task_mgr_selector = SimpleTaskMgrSelector::new(zone_cfg.backup_server_id.as_ref().unwrap());

    let restore_task_mgr = backup_service::RestoreTaskMgr::new(
        zone_cfg.zone_id.clone(),
        Box::new(task_mgr_selector),
        Box::new(file_mgr_selector),
        Box::new(chunk_mgr_selector),
    );

    let restore = "/tmp/etcd_restore";
    let restore_path = std::path::PathBuf::from_str(&restore).unwrap();
    tokio::fs::create_dir_all(restore_path.as_path())
        .await
        .expect("Failed to create directory for restore");

    let last_version_task = restore_task_mgr
        .get_last_check_point_version(&task_key)
        .await
        .map_err(handle_error!(
            "try_restore_etcd get_last_check_point_version failed"
        ))?;
    let last_version_task = last_version_task.map_or(
        Err(NodeDaemonErrors::ReasonError("no backup found".to_string())),
        |t| Ok(t),
    )?;

    let mut files = restore_task_mgr
        .restore(
            task_key,
            last_version_task.check_point_version,
            restore_path.as_path(),
        )
        .await
        .map_err(handle_error!("etcd restore failed"))?;

    if files.len() == 0 {
        return Err(NodeDaemonErrors::ReasonError(
            "no file restored".to_string(),
        ));
    }

    let file_path = restore_path.join(files.remove(0));
    etcd_client::try_restore_etcd(
        file_path.to_str().expect("need utf-8 path for restore"),
        "http://127.0.0.1:1280",
    )
    .await
    .unwrap();

    Ok(())
}

// try_report_node_status 上报node信息
// 1 读取 nodelist
//     没有 nodelist这个key，就插入nodelist值是<nodeid>
//     如果有nodelist这个key，就检查并更新nodelist
// 2读取 nodes[nodeid]
//    更新latest, 更新ip
//    更新 nodes[nodeid]
pub(crate) async fn try_report_node_status(
    node_cfg: &NodeIdentityConfig,
    system_config: Arc<&SystemConfig>,
) -> Result<()> {
    let key = "nodelist";
    let value = node_cfg.node_id.to_string().clone();
    let response = system_config
        .get(key)
        .await
        .map_err(handle_error!("system_config get nodelist failed!"))?;

    if response.0.is_empty() {
        system_config
            .put(key, &value)
            .await
            .map_err(handle_error!("system_config put nodelist failed!"))?;
    } else {
        let mut nodelist = response.0.split(",").collect::<Vec<_>>();
        if !nodelist.contains(&value.as_str()) {
            nodelist.push(&value);
            let new_value = nodelist.join(",");
            system_config
                .put(key, &new_value)
                .await
                .map_err(handle_error!("system_config put nodelist failed!"))?;
        }
    }

    let node_key = format!("nodes[{}]", node_cfg.node_id);
    let node_ip = "";
    let node_value = format!(
        r#"{{"latest":{},"ip":"{}"}}"#,
        chrono::Local::now().timestamp(),
        node_ip
    );
    system_config
        .put(&node_key, &node_value)
        .await
        .map_err(handle_error!("system_config put nodes/<nodeid> failed!"))?;

    info!("report node status success");
    Ok(())
}

// pub(crate) async fn system_config_backup(zone_config: Arc<ZoneConfig>) {
//     let zone_config = zone_config.as_ref();
//     let mut last_backup = Instant::now() - Duration::from_secs(24 * 3600); // 假设初始状态是需要立即备份
//     loop {
//         if last_backup.elapsed() >= Duration::from_secs(24 * 3600) {
//             let initial_cluster = zone_config
//                 .etcd_servers
//                 .iter()
//                 .enumerate()
//                 .map(|(idx, server)| format!("etcd{}={}", idx, server))
//                 .collect::<Vec<_>>()
//                 .join(",");
//             // 执行备份操作
//             let backup_file = etcd_client::backup_etcd(&initial_cluster).await.unwrap();
//             let backup_server_id = zone_config.backup_server_id.clone().unwrap();
//             let zone_id = zone_config.zone_id.clone();
//             let backup = Backup::new(backup_server_id, zone_id, BACKUP_STORAGE_DIR);
//             let key = "system_config/etcd";
//             let backup_file_path = std::path::Path::new(&backup_file);
//             let dir_path = backup_file_path.parent().expect("not full path");
//             let file_name = backup_file_path
//                 .file_name()
//                 .expect("no file name")
//                 .to_str()
//                 .expect("not utf-8 file name");
//             let file_name = std::path::Path::new(file_name);
//             let file_list = vec![file_name];

//             futures::executor::block_on({
//                 // 内部没有实现Send + Sync 用block 包一层
//                 backup
//                     .post_backup(key, 0, None, &"", dir_path, &file_list)
//                     .map(|_| ())
//             });
//             info!("备份已完成");

//             // 更新上次备份时间
//             last_backup = Instant::now();
//         }

//         // 每小时检查一次是否需要备份
//         std::thread::sleep(Duration::from_secs(3600));
//     }
// }
