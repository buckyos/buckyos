#![allow(unused_mut, unused, dead_code)]
mod app;
mod scheduler;
mod scheduler_server;
mod service;
mod system_config_agent;
mod system_config_builder;
mod thunk_runner;
mod zone_route_builder;

#[cfg(test)]
mod scheduler_test;

use jsonwebtoken::jwk::Jwk;
use log::*;
use serde_json::json;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::process::exit;
//use upon::Engine;

use anyhow::Result;
use app::*;
use buckyos_api::*;
use buckyos_api::*;
use buckyos_http_server::*;
use buckyos_kit::*;
use name_client::*;
use name_lib::*;
use scheduler_server::*;
use service::*;
use std::sync::Arc;
use system_config_agent::schedule_loop;
use system_config_builder::{StartConfigSummary, SystemConfigBuilder};

const BUCKYOS_ZONE_DOC_ENV: &str = "BUCKYOS_ZONE_DOC";

fn boot_ood_names(zone_document: &ZoneDocument) -> Vec<String> {
    zone_document
        .oods
        .iter()
        .filter(|ood| ood.node_type.is_ood())
        .map(|ood| ood.name.clone())
        .collect()
}

// buckyos_root 显式传入：get_buckyos_root_dir() 的进程级缓存会被并行/先行调用者
// 固定住，测试无法用 env 覆盖，导致读到宿主机安装目录下的旧模板。
async fn create_init_list_by_template(
    zone_document: &ZoneDocument,
    zone_document_str: &str,
    buckyos_root_dir: &Path,
) -> Result<HashMap<String, String>> {
    //load start_parms from active_service.
    let etc_dir = buckyos_root_dir.join("etc");
    let start_params_file_path = etc_dir.join("start_config.json");
    info!(
        "load start_params from :{}",
        start_params_file_path.to_string_lossy()
    );
    let start_params_str = tokio::fs::read_to_string(start_params_file_path).await?;
    let mut start_params: serde_json::Value = serde_json::from_str(&start_params_str)?;
    // 将Windows路径中的反斜杠转换为正斜杠，避免TOML转义问题
    let buckyos_root = buckyos_root_dir
        .to_string_lossy()
        .to_string()
        .replace('\\', "/");
    start_params["BUCKYOS_ROOT"] = json!(buckyos_root);
    let start_config = StartConfigSummary::from_value(&start_params)?;

    //generate dynamic params
    let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
    let verify_hub_public_key: Jwk = serde_json::from_value(public_key_jwk)
        .map_err(|e| anyhow::anyhow!("invalid jwk: {}", e))?;

    //load boot.template
    let template_file_path = etc_dir.join("scheduler").join("boot.template.toml");
    let template_str = match tokio::fs::read_to_string(&template_file_path).await {
        Ok(content) => content,
        Err(err) => {
            error!(
                "read template failed: path={}, err={}",
                template_file_path.to_string_lossy(),
                err
            );
            exit(1);
        }
    };

    let mut engine = upon::Engine::new();
    engine.add_template("config", &template_str)?;
    let result = engine
        .template("config")
        .render(&start_params)
        .to_string()?;

    if result.find("{{").is_some() {
        return Err(anyhow::anyhow!(
            "template contains unescaped double curly braces"
        ));
    }
    let mut boot_config: HashMap<String, String> = toml::from_str(&result)?;

    let ood_names = boot_ood_names(zone_document);
    if ood_names.is_empty() {
        return Err(anyhow::anyhow!("zone document has no OOD nodes"));
    }

    let mut builder = SystemConfigBuilder::new(boot_config);
    builder
        .add_boot_config(&start_config, &verify_hub_public_key, zone_document_str)?
        .add_user_doc(&start_config)?
        .add_default_accounts(&start_config)?
        .add_system_defaults()?
        .add_verify_hub(&private_key_pem)
        .await?
        .add_scheduler()
        .await?
        .add_task_mgr()
        .await?
        .add_kmsg()
        .await?
        .add_repo_service()
        .await?
        .add_aicc(&start_config)
        .await?
        .add_msg_center(&start_config)
        .await?
        .add_workflow()
        .await?
        .add_control_panel()
        .await?;

    info!("add_kernel_services success, add default apps and gateway settings...");
    builder
        //.add_smb_service().await?
        .add_default_apps(&start_config)
        .await?
        .add_default_agents(&start_config)
        .await?
        .add_gateway_settings(&start_config)?;
    for ood_name in ood_names.iter() {
        builder.add_node(ood_name.as_str())?;
    }
    let mut config = builder.build();

    Ok(config)
}

async fn do_boot_scheduler() -> Result<()> {
    let mut init_list: HashMap<String, String> = HashMap::new();
    let zone_document_str = std::env::var(BUCKYOS_ZONE_DOC_ENV);

    if zone_document_str.is_err() {
        warn!("{} is not set", BUCKYOS_ZONE_DOC_ENV);
        return Err(anyhow::anyhow!("{} is not set", BUCKYOS_ZONE_DOC_ENV));
    }

    info!(
        "{}:{}",
        BUCKYOS_ZONE_DOC_ENV,
        zone_document_str.as_ref().unwrap()
    );
    let zone_document_str = zone_document_str.unwrap();
    let zone_document: ZoneDocument = serde_json::from_str(&zone_document_str).unwrap();
    let rpc_session_token_str = std::env::var("SCHEDULER_SESSION_TOKEN");

    if rpc_session_token_str.is_err() {
        return Err(anyhow::anyhow!("SCHEDULER_SESSION_TOKEN is not set"));
    }

    let rpc_session_token = rpc_session_token_str.unwrap();
    let system_config_client = SystemConfigClient::new(None, Some(rpc_session_token.as_str()));
    let boot_config = system_config_client.get("boot/config").await;
    if boot_config.is_ok() {
        return Err(anyhow::anyhow!(
            "boot/config already exists, boot scheduler failed"
        ));
    }

    let mut init_list = create_init_list_by_template(
        &zone_document,
        &zone_document_str,
        get_buckyos_root_dir().as_path(),
    )
    .await
    .map_err(|e| {
        error!("create_init_list_by_template failed: {:?}", e);
        e
    })?;

    let boot_config_str = init_list.get("boot/config");
    if boot_config_str.is_none() {
        return Err(anyhow::anyhow!("boot/config not found in init list"));
    }
    let boot_config_str = boot_config_str.unwrap();
    info!("after boot_config_str: {}", boot_config_str);
    let _zone_config: ZoneConfig = serde_json::from_str(boot_config_str.as_str()).map_err(|e| {
        error!("load ZoneConfig from boot/config failed: {:?}", e);
        e
    })?;
    //info!("use init list from template {} to do boot scheduler",template_type_str);
    //write to system_config
    for (key, value) in init_list.iter() {
        system_config_client.create(key, value).await?;
    }

    info!("start boot schedule...");
    let boot_result = schedule_loop(true).await;
    if boot_result.is_err() {
        error!(
            "boot schedule_loop failed: {:?}",
            boot_result.err().unwrap()
        );
        return Err(anyhow::anyhow!("schedule_loop failed"));
    }
    system_config_client.refresh_trust_keys().await?;
    info!("system_config_service refresh trust keys success.");

    info!("do boot scheduler success!");
    return Ok(());
}

async fn service_main(is_boot: bool) -> Result<i32> {
    init_logging("scheduler", true);
    info!("Starting scheduler service............................");

    if is_boot {
        info!("do_boot_scheduler,scheduler run once");
        let runtime =
            init_buckyos_api_runtime("scheduler", None, BuckyOSRuntimeType::KernelService)
                .await
                .map_err(|e| {
                    error!("init_buckyos_api_runtime failed: {:?}", e);
                    e
                })?;
        runtime.set_all_background_tasks_enabled(false).await;
        let mut real_machine_config = BuckyOSMachineConfig::default();
        let machine_config = BuckyOSMachineConfig::load_machine_config();
        if machine_config.is_some() {
            real_machine_config = machine_config.unwrap();
        }
        info!("machine_config: {:?}", &real_machine_config);

        init_name_lib(&real_machine_config.web3_bridge)
            .await
            .map_err(|err| {
                error!("init default name client failed! {}", err);
                return String::from("init default name client failed!");
            })
            .unwrap();
        info!("init default name client OK!");
        set_buckyos_api_runtime(runtime).map_err(|err| {
            error!("register global runtime failed: {}", err);
            anyhow::anyhow!("register global runtime failed: {}", err)
        })?;
        do_boot_scheduler().await.map_err(|e| {
            error!("do_boot_scheduler failed: {:?}", e);
            e
        })?;
        return Ok(0);
    } else {
        info!("Enter schedule loop.");
        let mut runtime =
            init_buckyos_api_runtime("scheduler", None, BuckyOSRuntimeType::KernelService)
                .await
                .map_err(|e| {
                    error!("init_buckyos_api_runtime failed: {:?}", e);
                    e
                })?;

        runtime.login().await.map_err(|e| {
            error!("buckyos-api-runtime::login failed: {:?}", e);
            e
        })?;
        set_buckyos_api_runtime(runtime).map_err(|err| {
            error!("register global runtime failed: {}", err);
            anyhow::anyhow!("register global runtime failed: {}", err)
        })?;

        let scheduler_server = SchedulerServer::new();

        //start!
        info!("Start Scheduler Server...");
        let runner = Runner::new(SCHEDULER_SERVICE_MAIN_PORT);
        runner.add_http_server("/kapi/scheduler".to_string(), Arc::new(scheduler_server));
        info!("Start scheduler loop task...");
        tokio::spawn(async move {
            if let Err(err) = schedule_loop(false).await {
                error!("schedule_loop failed: {:?}", err);
            }
        });
        if let Err(err) = runner.run().await {
            error!("scheduler runner exited with error: {:?}", err);
            return Err(anyhow::anyhow!("scheduler runner exited: {:?}", err));
        }
        return Ok(0);
    }
}

#[tokio::main]
async fn main() {
    let args = std::env::args().collect::<Vec<String>>();
    let mut is_boot = false;
    if args.len() > 1 {
        if args[1] == "--boot" {
            is_boot = true;
        }
    }

    unsafe {
        //std::env::set_var("BUCKY_LOG", "debug");
    }

    let ret = service_main(is_boot).await;
    if ret.is_err() {
        println!("service_main failed: {:?}", ret);
        exit(-1);
    }
    exit(ret.unwrap());
}

#[cfg(test)]
mod test {
    use super::*;
    use async_trait::async_trait;
    use buckyos_api::test_config;
    use jsonwebtoken::{jwk::Jwk, DecodingKey};
    use name_client::{
        NameClient, NameClientConfig, NameInfo, NsProvider, RecordType, GLOBAL_NAME_CLIENT,
    };
    use name_lib::{
        DeviceDocument, DeviceInfo, EncodedDocument, NSError, OODDescriptionString,
        DEFAULT_EXPIRE_TIME,
    };
    use package_lib::PackageId;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fs;
    use std::net::IpAddr;
    use std::path::Path;
    use std::sync::Arc;
    use system_config_agent::*;
    use tempfile::TempDir;

    const TEST_USERNAME: &str = "devtest";
    const TEST_ZONE_NAME: &str = "devtest";
    const TEST_HOSTNAME: &str = "devtest.buckyos.io";
    const TEST_DEVICE_NAME: &str = "ood1";
    const TEST_NET_ID: &str = "lan1";

    #[tokio::test]
    async fn test_gen_service_doc() -> Result<()> {
        let mut docs = test_config::gen_kernel_service_docs();
        for (did, doc) in docs.iter() {
            let doc_path = format!("/tmp/{}.doc.json", did.to_raw_host_name());
            fs::write(doc_path.clone(), doc.to_string()).unwrap();
            println!("path: {}, doc: {}", doc_path, doc.to_string());
        }
        Ok(())
    }

    #[tokio::test]
    async fn test_boot_schedule() {
        let temp_root = TempDir::new().unwrap();
        unsafe {
            //std::env::set_var("BUCKY_LOG", "debug");
            std::env::set_var(
                "BUCKYOS_ROOT",
                temp_root.path().to_string_lossy().to_string(),
            );
        }

        buckyos_kit::init_logging("scheduler-test", false);

        write_boot_template(temp_root.path());
        init_static_name_client().await;

        let zone_document = prepare_scheduler_test_configs(temp_root.path()).await;
        let zone_document_str = serde_json::to_string(&zone_document).unwrap();
        let mut init_map =
            create_init_list_by_template(&zone_document, &zone_document_str, temp_root.path())
                .await
                .expect("init list generation should succeed");
        let start_config_value: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(temp_root.path().join("etc").join("start_config.json"))
                .expect("failed to read start_config"),
        )
        .expect("start_config should be valid json");
        let start_config = StartConfigSummary::from_value(&start_config_value)
            .expect("start_config summary should parse");
        ensure_device_info_entry(
            &mut init_map,
            start_config
                .ood_jwt
                .as_deref()
                .expect("start_config ood_jwt missing"),
            zone_document
                .get_default_key()
                .as_ref()
                .expect("owner key missing"),
        )
        .expect("device info generation failed");

        assert!(init_map.contains_key("boot/config"));
        assert!(init_map.contains_key("security/verify-hub/key"));
        assert!(!init_map.contains_key("system/verify-hub/key"));
        assert!(!init_map.contains_key(&format!("devices/{}/doc", TEST_DEVICE_NAME)));
        assert!(init_map.contains_key("services/verify-hub/spec"));
        assert!(init_map.contains_key("services/scheduler/spec"));
        assert!(init_map.contains_key("services/task-manager/spec"));
        assert!(init_map.contains_key("services/kmsg/spec"));
        assert!(init_map.contains_key("services/repo-service/spec"));
        assert!(init_map.contains_key("services/repo-service/settings"));
        assert!(init_map.contains_key("services/repo-service/pkg_list"));
        assert!(init_map.contains_key("services/aicc/spec"));
        assert!(init_map.contains_key("services/msg-center/spec"));
        //assert!(init_map.contains_key("services/smb-service/spec"));
        assert!(init_map.contains_key(&format!("users/{}/profile", TEST_USERNAME)));
        let user_settings: serde_json::Value = serde_json::from_str(
            init_map
                .get(&format!("users/{}/settings", TEST_USERNAME))
                .expect("user settings should exist"),
        )
        .expect("user settings should be valid json");
        assert_eq!(user_settings["is_local"], true);
        assert!(user_settings.get("show_name").is_none());
        assert!(user_settings.get("contact").is_none());

        for (key, value) in init_map.iter() {
            println!("#{} ==> {}", key, value);
        }

        println!("start test boot scheduler...");
        let schedule_plan = build_schedule_plan(&init_map, true)
            .await
            .expect("boot schedule should succeed");

        let this_snapshot = serde_json::to_string_pretty(&schedule_plan.schedule_snapshot).unwrap();
        println!("this_snapshot: {}", this_snapshot);

        assert!(!schedule_plan.tx_actions.is_empty());
        assert_eq!(schedule_plan.schedule_snapshot.nodes.len(), 1);
        assert!(schedule_plan
            .schedule_snapshot
            .service_infos
            .contains_key("scheduler"));

        let mut multi_ood_zone_document = zone_document.clone();
        multi_ood_zone_document.oods = vec![
            OODDescriptionString::new(
                "zone-gateway".to_string(),
                DeviceNodeType::Gateway,
                None,
                None,
            ),
            OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, None, None),
            OODDescriptionString::new("ood2".to_string(), DeviceNodeType::OODOnly, None, None),
        ];
        let multi_ood_zone_document_str = serde_json::to_string(&multi_ood_zone_document).unwrap();
        let multi_ood_init_map = create_init_list_by_template(
            &multi_ood_zone_document,
            &multi_ood_zone_document_str,
            temp_root.path(),
        )
        .await
        .expect("multi OOD init list generation should succeed");
        assert!(multi_ood_init_map.contains_key("nodes/ood1/config"));
        assert!(multi_ood_init_map.contains_key("nodes/ood2/config"));
        assert!(!multi_ood_init_map.contains_key("nodes/zone-gateway/config"));
        assert!(!multi_ood_init_map.contains_key("devices/ood1/doc"));
        assert!(!multi_ood_init_map.contains_key("devices/ood2/doc"));

        let gateway_only_zone_document = {
            let mut document = zone_document.clone();
            document.oods = vec![OODDescriptionString::new(
                "zone-gateway".to_string(),
                DeviceNodeType::Gateway,
                None,
                None,
            )];
            document
        };
        let gateway_only_zone_document_str =
            serde_json::to_string(&gateway_only_zone_document).unwrap();
        let gateway_only_init_result = create_init_list_by_template(
            &gateway_only_zone_document,
            &gateway_only_zone_document_str,
            temp_root.path(),
        )
        .await
        .err()
        .expect("gateway-only init list should fail");
        assert!(gateway_only_init_result
            .to_string()
            .contains("zone document has no OOD nodes"));

        unsafe {
            std::env::remove_var("BUCKYOS_ROOT");
        }
        drop(temp_root);
    }

    fn write_boot_template(root: &Path) {
        let scheduler_dir = root.join("etc").join("scheduler");
        fs::create_dir_all(&scheduler_dir).unwrap();
        let template = r#"
"system/install_settings" = """
{
    "pre_install_apps": {
        "buckyos_filebrowser": {
            "app_doc": {
                "id": "did:bns:buckyos_filebrowser.buckyos.ai",
                "doc_type": "app",
                "name": "buckyos_filebrowser",
                "version": "0.5.1",
                "meta": {
                    "detail": "BuckyOS File Browser"
                },
                "create_time": 1743008063,
                "last_update_time": 1743008063,
                "exp": 1837616063,
                "tag": "latest",
                "author": "did:web:buckyos.ai",
                "owner": "did:web:buckyos.ai",
                "show_name": "BuckyOS File Browser",
                "selector_type": "single",
                "install_config_tips": {
                    "data_mount_point": ["/srv/", "/database/", "/config/"],
                    "local_cache_mount_point": [],
                    "service_ports": {
                        "www": 80
                    }
                },
                "pkg_list": {
                    "amd64_docker_image": {
                        "pkg_id": "nightly-linux-amd64.buckyos_filebrowser-img#0.5.1",
                        "docker_image_name": "buckyos/nightly-buckyos_filebrowser:0.5.1-amd64"
                    },
                    "aarch64_docker_image": {
                        "pkg_id": "nightly-linux-aarch64.buckyos_filebrowser-img#0.5.1",
                        "docker_image_name": "buckyos/nightly-buckyos_filebrowser:0.5.1-aarch64"
                    },
                    "amd64_win_app": {
                        "pkg_id": "nightly-windows-amd64.buckyos_filebrowser-bin#0.5.1"
                    },
                    "aarch64_apple_app": {
                        "pkg_id": "nightly-apple-aarch64.buckyos_filebrowser-bin#0.5.1"
                    },
                    "amd64_apple_app": {
                        "pkg_id": "nightly-apple-amd64.buckyos_filebrowser-bin#0.5.1"
                    }
                }
            },
            "data_mount_point": {
                "root": "/root"
            },
            "cache_mount_point": [
            ],
            "local_cache_mount_point": [
            ],
            "bind_address": "0.0.0.0",
            "service_ports": {
                "http": 80
            },
            "res_pool_id": "default"
        },
        "buckyos_systest": {
            "app_doc": {
                "id": "did:bns:buckyos_systest.buckyos.ai",
                "doc_type": "app",
                "name": "buckyos_systest",
                "version": "0.5.1",
                "meta": {
                    "detail": "BuckyOS System Test App"
                },
                "create_time": 1743008063,
                "last_update_time": 1743008063,
                "exp": 1837616063,
                "tag": "latest",
                "author": "did:web:buckyos.ai",
                "owner": "did:web:buckyos.ai",
                "show_name": "BuckyOS System Test",
                "selector_type": "static",
                "install_config_tips": {},
                "pkg_list": {
                    "web": {
                        "pkg_id": "nightly-linux-amd64.buckyos_systest#0.5.1"
                    }
                }
            },
            "data_mount_point": {},
            "cache_mount_point": [],
            "local_cache_mount_point": [],
            "res_pool_id": "default"
        }
    }
}
"""
"#;
        fs::write(scheduler_dir.join("boot.template.toml"), template).unwrap();
    }

    async fn prepare_scheduler_test_configs(root: &Path) -> ZoneDocument {
        let output_dir = root.join("dev_env");
        fs::create_dir_all(&output_dir).unwrap();
        let output_dir_str = output_dir.to_string_lossy().to_string();

        test_config::cmd_create_user_env(
            TEST_USERNAME,
            TEST_HOSTNAME,
            "ood1",
            "",
            2980,
            Some(output_dir_str.as_str()),
        )
        .await
        .expect("failed to create user env");

        test_config::cmd_create_node_configs(
            TEST_DEVICE_NAME,
            output_dir.as_path(),
            None,
            Some(TEST_NET_ID),
        )
        .await
        .expect("failed to create node config");

        let etc_dir = root.join("etc");
        fs::create_dir_all(&etc_dir).unwrap();
        let start_config_src: std::path::PathBuf =
            output_dir.join(TEST_DEVICE_NAME).join("start_config.json");
        fs::copy(start_config_src, etc_dir.join("start_config.json"))
            .expect("failed to copy start_config");

        let owner_config_path = output_dir.join("user_config.json");
        let owner_config_value: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(owner_config_path).expect("failed to read owner config"),
        )
        .expect("failed to parse owner config");
        let owner_key_value = owner_config_value["verificationMethod"]
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|vm| vm.get("publicKeyJwk"))
            .cloned()
            .expect("owner public key not found");
        let owner_key: Jwk = serde_json::from_value(owner_key_value).expect("invalid owner jwk");

        let mut zone_document = ZoneDocument::new(
            DID::new("web", TEST_HOSTNAME),
            DID::new("bns", TEST_USERNAME),
            owner_key,
        );
        zone_document.oods = vec![OODDescriptionString::new(
            TEST_DEVICE_NAME.to_string(),
            DeviceNodeType::OOD,
            None,
            None,
        )];
        zone_document
    }

    fn ensure_device_info_entry(
        init_map: &mut HashMap<String, String>,
        device_doc_jwt: &str,
        owner_key: &Jwk,
    ) -> Result<(), String> {
        let encoded_doc = EncodedDocument::from_str(device_doc_jwt.to_string())
            .map_err(|e| format!("invalid encoded doc: {:?}", e))?;
        let decoding_key =
            DecodingKey::from_jwk(owner_key).map_err(|e| format!("invalid owner jwk: {}", e))?;
        let device_config = DeviceDocument::decode(&encoded_doc, Some(&decoding_key))
            .map_err(|e| format!("failed to decode device document: {}", e))?;
        let mut device_info = serde_json::to_value(DeviceInfo::from_device_doc(&device_config))
            .map_err(|e| format!("serialize device info: {}", e))?;
        let device_info_obj = device_info
            .as_object_mut()
            .ok_or_else(|| "device info is not a json object".to_string())?;
        device_info_obj.insert("support_container".to_string(), json!(true));
        device_info_obj.insert("cpu_mhz".to_string(), json!(16000));
        device_info_obj.insert("total_mem".to_string(), json!(8_u64 * 1024 * 1024 * 1024));
        device_info_obj.insert("mem_usage".to_string(), json!(0));
        device_info_obj.insert("net_id".to_string(), json!(TEST_NET_ID));
        let device_info_json = serde_json::to_string(&device_info)
            .map_err(|e| format!("serialize device info: {}", e))?;

        init_map.insert(
            format!("devices/{}/info", TEST_DEVICE_NAME),
            device_info_json,
        );
        Ok(())
    }

    async fn init_static_name_client() {
        if GLOBAL_NAME_CLIENT.get().is_some() {
            return;
        }
        let client = NameClient::new(NameClientConfig::default());

        let mut docs = buckyos_api::test_config::gen_kernel_service_docs();
        docs.insert(
            PackageId::unique_name_to_did("buckyos_filebrowser"),
            get_filebrowser_doc(),
        );
        let docs = docs
            .into_iter()
            .map(|(did, doc)| (did.to_raw_host_name(), doc))
            .collect();
        client
            .add_provider(Box::new(StaticProvider::new(docs)), None)
            .await;
        let _ = GLOBAL_NAME_CLIENT.set(client);
    }

    fn get_filebrowser_doc() -> EncodedDocument {
        let doc_str = r#"{
  "name": "buckyos_filebrowser",
    "version": "0.5.1",
    "meta": {
      "detail": "BuckyOS File Browser"
    },
    "create_time": 1743008063,
    "last_update_time": 1743008063,
    "exp": 1837616063,
    "tag": "latest",
    "author": "did:web:buckyos.ai",
    "owner": "did:web:buckyos.ai",
    "show_name": "BuckyOS File Browser",
    "selector_type": "single",
    "install_config_tips": {
      "data_mount_point": [
        "/srv/",
        "/database/",
        "/config/"
      ],
      "local_cache_mount_point": [],
      "service_ports": {
        "www": 80
      }
    },
    "pkg_list": {
      "amd64_docker_image": {
        "pkg_id": "nightly-linux-amd64.buckyos_filebrowser-img#0.5.1",
        "docker_image_name": "buckyos/nightly-buckyos_filebrowser:0.5.1-amd64"
      },
      "aarch64_docker_image": {
        "pkg_id": "nightly-linux-aarch64.buckyos_filebrowser-img#0.5.1",
        "docker_image_name": "buckyos/nightly-buckyos_filebrowser:0.5.1-aarch64"
      },
      "amd64_win_app": {
        "pkg_id": "nightly-windows-amd64.buckyos_filebrowser-bin#0.5.1"
      },
      "aarch64_apple_app": {
        "pkg_id": "nightly-apple-aarch64.buckyos_filebrowser-bin#0.5.1"
      },
      "web": null,
      "amd64_apple_app": {
        "pkg_id": "nightly-apple-amd64.buckyos_filebrowser-bin#0.5.1"
      }
    }
  }             
        "#;
        let doc: EncodedDocument = EncodedDocument::from_str(doc_str.to_string()).unwrap();
        doc
    }

    #[derive(Clone)]
    struct StaticProvider {
        docs: Arc<HashMap<String, EncodedDocument>>,
    }

    impl StaticProvider {
        fn new(docs: HashMap<String, EncodedDocument>) -> Self {
            Self {
                docs: Arc::new(docs),
            }
        }
    }

    #[async_trait]
    impl NsProvider for StaticProvider {
        fn get_id(&self) -> String {
            "static-provider".to_string()
        }

        async fn query(
            &self,
            name: &str,
            _record_type: Option<RecordType>,
            _from_ip: Option<IpAddr>,
        ) -> name_lib::NSResult<NameInfo> {
            Err(NSError::NotFound(name.to_string()))
        }

        async fn query_did(
            &self,
            did: &DID,
            _doc_type: Option<name_client::DidDocType>,
            _from_ip: Option<IpAddr>,
        ) -> name_lib::NSResult<EncodedDocument> {
            let host = did.to_host_name();
            self.docs
                .get(&host)
                .cloned()
                .ok_or_else(|| NSError::NotFound(host))
        }
    }
}
