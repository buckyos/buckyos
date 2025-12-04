#![allow(unused_mut, unused, dead_code)]
mod app;
mod scheduler;
mod service;
mod scheduler_server;
mod system_config_agent;
mod system_config_builder;

#[cfg(test)]
mod scheduler_test;

use jsonwebtoken::jwk::Jwk;
use log::*;
use serde_json::Value;
use serde_json::json;
use std::collections::HashMap;
use std::process::exit;
//use upon::Engine;

use app::*;
use buckyos_api::*;
use buckyos_api::*;
use buckyos_kit::*;
use name_client::*;
use name_lib::*;
use scheduler_server::*;
use service::*;
use system_config_agent::{
    schedule_action_to_tx_actions, schedule_loop, update_gateway_node_list,
};
use system_config_builder::{StartConfigSummary, SystemConfigBuilder};
use server_runner::*;
use std::sync::Arc;
use anyhow::Result;


async fn create_init_list_by_template(zone_boot_config: &ZoneBootConfig) -> Result<HashMap<String, String>> {
    //load start_parms from active_service.
    let start_params_file_path = get_buckyos_system_etc_dir().join("start_config.json");
    info!(
        "load start_params from :{}",
        start_params_file_path.to_string_lossy()
    );
    let start_params_str = tokio::fs::read_to_string(start_params_file_path).await?;
    let mut start_params: serde_json::Value = serde_json::from_str(&start_params_str)?;
    // 将Windows路径中的反斜杠转换为正斜杠，避免TOML转义问题
    let buckyos_root = get_buckyos_root_dir().to_string_lossy().to_string().replace('\\', "/");
    start_params["BUCKYOS_ROOT"] = json!(buckyos_root);
    let start_config = StartConfigSummary::from_value(&start_params)?;

    //generate dynamic params
    let (private_key_pem, public_key_jwk) = generate_ed25519_key_pair();
    let verify_hub_public_key: Jwk = serde_json::from_value(public_key_jwk).map_err(|e| anyhow::anyhow!("invalid jwk: {}", e))?;

    //load boot.template
    let template_type_str = "boot".to_string();
    let template_file_path = get_buckyos_system_etc_dir()
        .join("scheduler")
        .join(format!("{}.template.toml", template_type_str));
    let template_str = tokio::fs::read_to_string(template_file_path).await?;

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
    if !boot_config.contains_key("system/rbac/base_policy") {
        boot_config.insert("system/rbac/base_policy".to_string(), rbac::DEFAULT_POLICY.to_string());
    }
    if !boot_config.contains_key("system/rbac/model") {
        boot_config.insert("system/rbac/model".to_string(), rbac::DEFAULT_MODEL.to_string());
    }

    let ood_name = zone_boot_config.oods.first().unwrap().name.as_str();
    let mut builder = SystemConfigBuilder::new(boot_config);
    builder
        .add_boot_config(&start_config, &verify_hub_public_key, zone_boot_config)?
        .add_user_doc(&start_config)?
        .add_default_accounts(&start_config)?
        .add_device_doc(ood_name,&start_config)?
        .add_system_defaults()?
        .add_verify_hub(&private_key_pem).await?
        .add_scheduler().await?
        .add_repo_service().await?
        .add_smb_service().await?
        .add_default_apps(&start_config).await?
        .add_gateway_settings(&start_config)?
        .add_node(ood_name)?;
    let mut config = builder.build();

    Ok(config)
}

async fn do_boot_scheduler() -> Result<()> {
    let mut init_list: HashMap<String, String> = HashMap::new();
    let zone_boot_config_str = std::env::var("BUCKYOS_ZONE_BOOT_CONFIG");

    if zone_boot_config_str.is_err() {
        warn!("BUCKYOS_ZONE_BOOT_CONFIG is not set, use default zone config");
        return Err(anyhow::anyhow!("BUCKYOS_ZONE_BOOT_CONFIG is not set"));
    }

    info!(
        "zone_boot_config_str:{}",
        zone_boot_config_str.as_ref().unwrap()
    );
    let zone_boot_config: ZoneBootConfig =
        serde_json::from_str(&zone_boot_config_str.unwrap()).unwrap();
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

    let mut init_list = create_init_list_by_template(&zone_boot_config).await.map_err(|e| {
        error!("create_init_list_by_template failed: {:?}", e);
        e
    })?;

    let boot_config_str = init_list.get("boot/config");
    if boot_config_str.is_none() {
        return Err(anyhow::anyhow!("boot/config not found in init list"));
    }
    let boot_config_str = boot_config_str.unwrap();
    debug!("boot_config_str: {}", boot_config_str);
    let mut zone_config: ZoneConfig = serde_json::from_str(boot_config_str.as_str()).map_err(|e| {
        error!("load ZoneConfig from boot/config failed: {:?}", e);
        e
    })?;
    zone_config.init_by_boot_config(&zone_boot_config);
    init_list.insert(
        "boot/config".to_string(),
        serde_json::to_string_pretty(&zone_config).unwrap(),
    );
    //info!("use init list from template {} to do boot scheduler",template_type_str);
    //write to system_config
    for (key, value) in init_list.iter() {
        system_config_client.create(key, value).await?;
    }

    info!("start boot schedule...");
    let boot_result = schedule_loop(true).await;
    if boot_result.is_err() {
        error!("boot schedule_loop failed: {:?}", boot_result.err().unwrap());
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
        let mut real_machine_config = BuckyOSMachineConfig::default();
        let machine_config = BuckyOSMachineConfig::load_machine_config();
        if machine_config.is_some() {
            real_machine_config = machine_config.unwrap();
        }
        info!("machine_config: {:?}", &real_machine_config);
    
        init_name_lib(&real_machine_config.web3_bridge).await.map_err(|err| {
            error!("init default name client failed! {}", err);
            return String::from("init default name client failed!");
        }).unwrap();
        info!("init default name client OK!");
        set_buckyos_api_runtime(runtime);
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
        set_buckyos_api_runtime(runtime);

        let scheduler_server = SchedulerServer::new();
        
        //start!
        info!("Start Scheduler Server...");
        let runner = Runner::new(SCHEDULER_SERVICE_MAIN_PORT);
        runner.add_http_server("/kapi/scheduler".to_string(), Arc::new(scheduler_server));
        runner.run().await;

        schedule_loop(false).await.map_err(|e| {
            error!("schedule_loop failed: {:?}", e);
            e
        });
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
    use system_config_agent::*;
    use async_trait::async_trait;
    use jsonwebtoken::{jwk::Jwk, DecodingKey};
    use name_client::{
        NameClient, NameClientConfig, NameInfo, NsProvider, RecordType, GLOBAL_NAME_CLIENT,
    };
    use name_lib::{
        DeviceConfig, DeviceInfo, EncodedDocument, NSError, OODDescriptionString, DEFAULT_EXPIRE_TIME,
    };
    use package_lib::PackageId;
    use serde_json::json;
    use std::collections::{HashMap, HashSet};
    use std::fs;
    use std::net::IpAddr;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::TempDir;
    use buckyos_api::test_config;

    const TEST_USERNAME: &str = "devtest";
    const TEST_ZONE_NAME: &str = "devtest";
    const TEST_HOSTNAME: &str = "devtest.buckyos.io";
    const TEST_DEVICE_NAME: &str = "ood1";
    const TEST_NET_ID: &str = "lan1";

    #[tokio::test]
    async fn test_gen_service_doc() -> Result<()> {
        let mut docs = kernel_service_docs();
        for (did, doc) in docs.iter() {
            
            let doc_path = format!("/tmp/{}.doc.json", did.as_str());
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
            std::env::set_var("BUCKYOS_ROOT", temp_root.path().to_string_lossy().to_string());
        }

        buckyos_kit::init_logging("scheduler-test", false);

        write_boot_template(temp_root.path());
        init_static_name_client().await;

        let zone_boot_config = prepare_scheduler_test_configs(temp_root.path()).await;
        let mut init_map = create_init_list_by_template(&zone_boot_config)
            .await
            .expect("init list generation should succeed");
        ensure_device_info_entry(
            &mut init_map,
            zone_boot_config.owner_key.as_ref().expect("owner key missing"),
        )
        .expect("device info generation failed");

        assert!(init_map.contains_key("boot/config"));
        assert!(init_map.contains_key("services/verify-hub/spec"));
        assert!(init_map.contains_key("services/scheduler/spec"));
        assert!(init_map.contains_key("services/repo-service/spec"));
        assert!(init_map.contains_key("services/smb-service/spec"));

        for (key, value) in init_map.iter() {
            println!("#{} ==> {}", key, value);
        }

        println!("start test boot scheduler...");
        let (mut scheduler_ctx, device_list) = create_scheduler_by_system_config(&init_map).unwrap();
        let action_list = scheduler_ctx
            .schedule(None)
            .expect("schedule should succeed");

        let this_snapshot = serde_json::to_string_pretty(&scheduler_ctx).unwrap();
        println!("this_snapshot: {}", this_snapshot);
    
        let mut tx_actions = HashMap::new();
        let mut need_update_gateway_node_list:HashSet<String> = HashSet::new();
        let mut need_update_rbac = false;
        for action in action_list {
            let new_tx_actions = schedule_action_to_tx_actions(
                &action,
                &scheduler_ctx,
                &device_list,
                &init_map,
                &mut need_update_gateway_node_list,
                &mut need_update_rbac,
            ).unwrap();
            extend_kv_action_map(&mut tx_actions, &new_tx_actions);
        }
        
 
        need_update_rbac = true;
        need_update_gateway_node_list = scheduler_ctx.nodes.keys().cloned().collect();
       

        if need_update_gateway_node_list.len() > 0 {
            // 重新生成node_gateway_config
            let update_gateway_node_list_actions = update_gateway_node_list(&need_update_gateway_node_list, &scheduler_ctx).await.unwrap();
            extend_kv_action_map(&mut tx_actions, &update_gateway_node_list_actions);
        }
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
        }
    }
}
"""
"system/rbac/base_policy" = """
p, kernel, kv://*, read|write,allow
p, kernel, dfs://*, read|write,allow
p, kernel, ndn://*, read|write,allow

p, root, kv://*, read|write,allow
p, root, dfs://*, read|write,allow
p, root, ndn://*, read|write,allow

p, ood,kv://*,read,allow
p, ood,kv://users/*/apps/*,read|write,allow
p, ood,kv://nodes/{device}/*,read|write,allow
p, ood,kv://services/*,read|write,allow
p, ood,kv://system/rbac/policy,read|write,allow

p, client, kv://boot/*, read,allow
p, client,kv://devices/{device}/*,read,allow
p, client,kv://devices/{device}/info,read|write,allow

p, service, kv://boot/*, read,allow
p, service,kv://services/{service}/*,read|write,allow
p, service,kv://services/*/info,read,allow
p, service,kv://users*,read,allow
p, service,kv://users/*/*,read,allow
p, service,kv://system/*,read,allow
p, service,dfs://system/data/{service}/*,read|write,allow
p, service,dfs://system/cache/{service}/*,read|write,allow

p, app, kv://boot/*, read,allow
p, app, kv://users/*/apps/{app}/settings,read|write,allow
p, app, kv://users/*/apps/{app}/config,read,allow
p, app, kv://users/*/apps/{app}/info,read,allow
p, app, dfs://users/*/appdata/{app}/*, read|write,allow
p, app, dfs://users/*/cache/{app}/*, read|write,allow
p, admin, kv://boot/*, read,allow
p, admin,kv://users/{user}/*,read|write,allow
p, admin,dfs://users/{user}/*,read|write,allow
p, admin,kv://services/*,read|write,allow
p, admin,dfs://library/*,read|write,allow
p, user, kv://boot/*, read,allow
p, user,kv://users/{user}/*,read,allow
p, user,kv://users/{user}/apps/*/*,read|write,allow
p, user,dfs://users/{user}/*,read|write,allow
p, user,dfs://users/{user}/home/*,read|write,allow
p, user,dfs://library/*,read,allow

g, node-daemon, kernel
g, scheduler, kernel
g, system-config, kernel
g, verify-hub, kernel
g, control-panel, kernel
g, buckycli, kernel
g, cyfs-gateway, kernel
"""
"#;
        fs::write(scheduler_dir.join("boot.template.toml"), template).unwrap();
    }

    async fn prepare_scheduler_test_configs(root: &Path) -> ZoneBootConfig {
        let output_dir = root.join("dev_env");
        fs::create_dir_all(&output_dir).unwrap();
        let output_dir_str = output_dir.to_string_lossy().to_string();

        test_config::cmd_create_user_env(
            TEST_USERNAME,
            TEST_HOSTNAME,
            TEST_NET_ID,
            Some(output_dir_str.as_str()),
        )
        .await
        .expect("failed to create user env");

        test_config::cmd_create_node_configs(
            TEST_USERNAME,
            TEST_DEVICE_NAME,
            TEST_ZONE_NAME,
            Some(output_dir_str.as_str()),
            Some(TEST_NET_ID),
        )
        .await
        .expect("failed to create node config");

        let etc_dir = root.join("etc");
        fs::create_dir_all(&etc_dir).unwrap();
        let start_config_src = output_dir
            .join(TEST_USERNAME)
            .join(TEST_DEVICE_NAME)
            .join("start_config.json");
        fs::copy(
            start_config_src,
            etc_dir.join("start_config.json"),
        )
        .expect("failed to copy start_config");

        let zone_config_file = format!("{}.zone.json", TEST_HOSTNAME);
        let zone_boot_path = output_dir
            .join(TEST_USERNAME)
            .join(zone_config_file);
        let mut zone_boot_config: ZoneBootConfig = serde_json::from_str(
            &fs::read_to_string(zone_boot_path).expect("failed to read zone boot config"),
        )
        .expect("failed to parse zone boot config");

        let owner_config_path = output_dir
            .join(TEST_USERNAME)
            .join("user_config.json");
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

        zone_boot_config.owner_key = Some(owner_key);
        zone_boot_config.owner = Some(DID::new("bns", TEST_USERNAME));
        zone_boot_config.id = Some(DID::new("web", TEST_HOSTNAME));

        zone_boot_config
    }

    fn ensure_device_info_entry(
        init_map: &mut HashMap<String, String>,
        owner_key: &Jwk,
    ) -> Result<(), String> {
        let doc_key = format!("devices/{}/doc", TEST_DEVICE_NAME);
        let doc_value = init_map
            .get(&doc_key)
            .ok_or_else(|| format!("{} not found in init map", doc_key))?
            .clone();

        let encoded_doc =
            EncodedDocument::from_str(doc_value).map_err(|e| format!("invalid encoded doc: {:?}", e))?;
        let decoding_key =
            DecodingKey::from_jwk(owner_key).map_err(|e| format!("invalid owner jwk: {}", e))?;
        let device_config =
            DeviceConfig::decode(&encoded_doc, Some(&decoding_key)).map_err(|e| {
                format!("failed to decode device document: {}", e)
            })?;
        let device_info = DeviceInfo::from_device_doc(&device_config);
        let device_info_json =
            serde_json::to_string(&device_info).map_err(|e| format!("serialize device info: {}", e))?;

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

        let mut docs = kernel_service_docs();
        docs.insert(PackageId::unique_name_to_did("buckyos_filebrowser").to_raw_host_name(), get_filebrowser_doc());
        client
            .add_provider(Box::new(StaticProvider::new(docs)))
            .await;
        let _ = GLOBAL_NAME_CLIENT.set(client);
    }

    fn get_filebrowser_doc() -> EncodedDocument {
        let doc_str = r#"{
  "pkg_name": "buckyos_filebrowser",
  "version": "0.4.1",
  "description": {
    "detail": "BuckyOS File Browser"
  },
  "pub_time": 1743008063,
  "exp": 1837616063,
  "deps": {
    "nightly-apple-amd64.buckyos_filebrowser-bin": "0.4.1",
    "nightly-linux-aarch64.buckyos_filebrowser-img": "0.4.1",
    "nightly-linux-amd64.buckyos_filebrowser-img": "0.4.1",
    "nightly-windows-amd64.buckyos_filebrowser-bin": "0.4.1",
    "nightly-apple-aarch64.buckyos_filebrowser-bin": "0.4.1"
  },
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
      "pkg_id": "nightly-linux-amd64.buckyos_filebrowser-img#0.4.1",
      "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-amd64"
    },
    "aarch64_docker_image": {
      "pkg_id": "nightly-linux-aarch64.buckyos_filebrowser-img#0.4.1",
      "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-aarch64"
    },
    "amd64_win_app": {
      "pkg_id": "nightly-windows-amd64.buckyos_filebrowser-bin#0.4.1"
    },
    "aarch64_apple_app": {
      "pkg_id": "nightly-apple-aarch64.buckyos_filebrowser-bin#0.4.1"
    },
    "web": null,
    "amd64_apple_app": {
      "pkg_id": "nightly-apple-amd64.buckyos_filebrowser-bin#0.4.1"
    }
  }
}        
        "#;
        let doc: EncodedDocument = EncodedDocument::from_str(doc_str.to_string()).unwrap();
        doc
    }

    fn kernel_service_docs() -> HashMap<String, EncodedDocument> {
        let mut docs = HashMap::new();
        let verify_hub_doc = buckyos_api::generate_verify_hub_service_doc();
        let verify_hub_json = serde_json::to_string(&verify_hub_doc).unwrap();
        let verify_hub_did = PackageId::unique_name_to_did(VERIFY_HUB_UNIQUE_ID);

        let scheduler_doc = buckyos_api::generate_scheduler_service_doc();
        let scheduler_json = serde_json::to_string(&scheduler_doc).unwrap();
        let scheduler_did = PackageId::unique_name_to_did(SCHEDULER_SERVICE_UNIQUE_ID);

        let repo_doc = buckyos_api::generate_repo_service_doc();
        let repo_did = PackageId::unique_name_to_did(REPO_SERVICE_UNIQUE_ID);
        let repo_json = serde_json::to_string(&repo_doc).unwrap();

        let smb_doc = buckyos_api::generate_smb_service_doc();
        let smb_json = serde_json::to_string(&smb_doc).unwrap();
        let smb_did = PackageId::unique_name_to_did(SMB_SERVICE_UNIQUE_ID);
        docs.insert(verify_hub_did.to_raw_host_name(), EncodedDocument::from_str(verify_hub_json).unwrap());
        docs.insert(scheduler_did.to_raw_host_name(), EncodedDocument::from_str(scheduler_json).unwrap());
        docs.insert(repo_did.to_raw_host_name(), EncodedDocument::from_str(repo_json).unwrap());
        docs.insert(smb_did.to_raw_host_name(), EncodedDocument::from_str(smb_json).unwrap());
        docs
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
            _fragment: Option<&str>,
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
