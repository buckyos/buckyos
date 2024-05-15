use crate::*;
use std::net::TcpStream;
use std::time::Duration;

const BACKUP_STORAGE_DIR: &'static str = "/tmp/backup";

pub(crate) async fn check_etcd_by_zone_config(
    config: &ZoneConfig,
    node_config: &NodeIdentityConfig,
) -> Result<EtcdState> {
    let node_id = &node_config.node_id;

    // ping local 2379
    let timeout = Duration::from_secs(1);
    if TcpStream::connect_timeout(&"127.0.0.1:2379".parse().unwrap(), timeout).is_ok() {
        info!("local etcd is running ");
        return Ok(EtcdState::Good(node_id.clone()));
    }

    let local_endpoint = config
        .etcd_servers
        .iter()
        .find(|&server| server.starts_with(node_id));
    info!(
        "check local etcd, local node id {:?}, local_endpoint:{:?}",
        node_id, local_endpoint
    );

    if let Some(endpoint) = local_endpoint {
        info!(
            "Found etcd server in this machine:{} ,try connect to local etcd.",
            endpoint
        );
        match EtcdClient::connect(endpoint).await {
            Ok(_) => Ok(EtcdState::Good(node_id.clone())),
            Err(_) => Ok(EtcdState::NeedRunInThisMachine(node_id.clone())),
        }
    } else {
        
        for endpoint in &config.etcd_servers {
            let endpoint = format!("http://{}:2379", endpoint);
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
    // 转换 etcd_servers为 initial_cluster 字符串
    let initial_cluster = zone_cfg
        .etcd_servers
        .iter()
        .map(|server| format!("{}=http://{}:2380", server.trim_end_matches(zone_cfg.zone_id.as_str()).trim_end_matches("."), server))
        .collect::<Vec<_>>()
        .join(",");
    let name = node_cfg.node_id.clone();
    let revision = etcd_client::get_etcd_data_version(&name, &initial_cluster, zone_cfg.zone_id.as_str())
        .await
        .map_err(|err| {
            let err_msg = format!("start_etcd! {}", err);
            error!("{}", err_msg);
            NodeDaemonErrors::ReasonError(err_msg.to_string())
        })
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
    let initial_cluster = zone_cfg
        .etcd_servers
        .iter()
        .map(|server| format!("{}=http://{}:2380", server.trim_end_matches(zone_cfg.zone_id.as_str()).trim_end_matches("."), server))
        .collect::<Vec<_>>()
        .join(",");
    let name = node_cfg.node_id.clone();
    etcd_client::start_etcd(&name, &initial_cluster, zone_cfg.zone_id.as_str()).map_err(|err| {
        let err_msg = format!("start_etcd! {}", err);
        error!("{}", err_msg);
        NodeDaemonErrors::ReasonError(err_msg.to_string())
    })?;

    Ok(())
}

pub(crate) async fn try_restore_etcd(
    _node_cfg: &NodeIdentityConfig,
    zone_cfg: &ZoneConfig,
) -> Result<()> {
    let backup_server_id = zone_cfg.backup_server_id.clone().unwrap();
    let backup = Backup::new(
        backup_server_id,
        zone_cfg.zone_id.clone(),
        BACKUP_STORAGE_DIR,
    );
    let restore = "/tmp/etcd_restore";
    let restore_path = std::path::PathBuf::from_str(&restore).unwrap();

    let key = "system_config/etcd";
    let latest = backup.query_last_versions(key, true).await.map_err(|err| {
        let err_msg = format!("query last backup version failed! {}", err);
        error!("{}", err_msg);
        return NodeDaemonErrors::ReasonError(err_msg.to_string());
    })?;
    let version = latest.version;
    backup
        .download_backup(key, version, Some(&restore_path))
        .await
        .unwrap();

    etcd_client::try_restore_etcd(&restore, "http://127.0.0.1:1280")
        .await
        .unwrap();

    Ok(())
}

pub(crate) async fn system_config_backup(zone_config: Arc<ZoneConfig>) {
    let zone_config = zone_config.as_ref();
    let mut last_backup = Instant::now() - Duration::from_secs(24 * 3600); // 假设初始状态是需要立即备份
    loop {
        if last_backup.elapsed() >= Duration::from_secs(24 * 3600) {
            let initial_cluster = zone_config
                .etcd_servers
                .iter()
                .enumerate()
                .map(|(idx, server)| format!("etcd{}={}", idx, server))
                .collect::<Vec<_>>()
                .join(",");
            // 执行备份操作
            let backup_file = etcd_client::backup_etcd(&initial_cluster).await.unwrap();
            let backup_server_id = zone_config.backup_server_id.clone().unwrap();
            let zone_id = zone_config.zone_id.clone();
            let backup = Backup::new(backup_server_id, zone_id, BACKUP_STORAGE_DIR);
            let key = "system_config/etcd";
            let backup_file_path = std::path::Path::new(&backup_file);
            let dir_path = backup_file_path.parent().expect("not full path");
            let file_name = backup_file_path
                .file_name()
                .expect("no file name")
                .to_str()
                .expect("not utf-8 file name");
            let file_name = std::path::Path::new(file_name);
            let file_list = vec![file_name];

            futures::executor::block_on({
                // 内部没有实现Send + Sync 用block 包一层
                backup
                    .post_backup(key, 0, None, &"", dir_path, &file_list)
                    .map(|_| ())
            });
            info!("备份已完成");

            // 更新上次备份时间
            last_backup = Instant::now();
        }

        // 每小时检查一次是否需要备份
        std::thread::sleep(Duration::from_secs(3600));
    }
}
