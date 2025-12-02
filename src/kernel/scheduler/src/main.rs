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
use system_config_agent::schedule_loop;
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
    let mut zone_config: ZoneConfig = serde_json::from_str(boot_config_str.as_str()).map_err(|e| {
        error!("serde_json::from_str failed: {:?}", e);
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
    use jsonwebtoken::jwk::Jwk;
    use name_client::{
        NameClient, NameClientConfig, NameInfo, NsProvider, RecordType, GLOBAL_NAME_CLIENT,
    };
    use name_lib::{EncodedDocument, NSError, OODDescriptionString, DEFAULT_EXPIRE_TIME};
    use package_lib::PackageId;
    use serde_json::{json, Value};
    use std::collections::HashMap;
    use std::fs;
    use std::net::IpAddr;
    use std::path::Path;
    use std::sync::Arc;
    use tempfile::TempDir;

    const TEST_PUBLIC_KEY_X: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    #[tokio::test]
    async fn create_init_list_by_template_generates_service_specs() {
        let temp_root = TempDir::new().unwrap();
        unsafe {
            std::env::set_var("BUCKYOS_ROOT", temp_root.path().to_string_lossy().to_string());
        }

        write_start_config(temp_root.path());
        write_boot_template(temp_root.path());
        init_static_name_client().await;

        let zone_boot_config = build_zone_boot_config();
        let init_map = create_init_list_by_template(&zone_boot_config)
            .await
            .expect("init list generation should succeed");

        assert!(init_map.contains_key("boot/config"));
        assert!(init_map.contains_key("services/verify-hub/spec"));
        assert!(init_map.contains_key("services/scheduler/spec"));
        assert!(init_map.contains_key("services/repo-service/spec"));
        assert!(init_map.contains_key("services/smb-service/spec"));

        for (key, value) in init_map.iter() {
            println!("#{} ==> {}", key, value);
        }

        unsafe {
            std::env::remove_var("BUCKYOS_ROOT");
        }
        drop(temp_root);
    }

    fn write_start_config(root: &Path) {
        let etc_dir = root.join("etc");
        fs::create_dir_all(&etc_dir).unwrap();
        let start_config = json!({
            "user_name": "tester",
            "admin_password_hash": "hash",
            "public_key": test_public_key_value(),
            "ood_jwt": "dummy-jwt"
        });
        let config_path = etc_dir.join("start_config.json");
        fs::write(config_path, serde_json::to_string_pretty(&start_config).unwrap()).unwrap();
    }

    fn write_boot_template(root: &Path) {
        let scheduler_dir = root.join("etc").join("scheduler");
        fs::create_dir_all(&scheduler_dir).unwrap();
        let template = r#"
"system/install_settings" = """
{
    "pre_install_apps": {}
}
"""
"#;
        fs::write(scheduler_dir.join("boot.template.toml"), template).unwrap();
    }

    fn build_zone_boot_config() -> ZoneBootConfig {
        ZoneBootConfig {
            id: Some(DID::new("bns", "test-zone")),
            oods: vec!["ood1".parse::<OODDescriptionString>().unwrap()],
            sn: None,
            exp: DEFAULT_EXPIRE_TIME + 10,
            owner: Some(DID::new("bns", "tester")),
            extra_info: HashMap::new(),
            owner_key: Some(test_public_key_jwk()),
            gateway_devs: vec![],
            devices: HashMap::new(),
        }
    }

    fn test_public_key_value() -> Value {
        json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": TEST_PUBLIC_KEY_X
        })
    }

    fn test_public_key_jwk() -> Jwk {
        serde_json::from_value(test_public_key_value()).unwrap()
    }

    async fn init_static_name_client() {
        if GLOBAL_NAME_CLIENT.get().is_some() {
            return;
        }
        let client = NameClient::new(NameClientConfig::default());
        client
            .add_provider(Box::new(StaticProvider::new(kernel_service_docs())))
            .await;
        let _ = GLOBAL_NAME_CLIENT.set(client);
    }

    fn kernel_service_docs() -> HashMap<String, EncodedDocument> {
        let mut docs = HashMap::new();
        for pkg in [
            VERIFY_HUB_UNIQUE_ID,
            SCHEDULER_SERVICE_UNIQUE_ID,
            REPO_SERVICE_UNIQUE_ID,
            SMB_SERVICE_UNIQUE_ID,
        ] {
            let did = PackageId::unique_name_to_did(pkg);
            docs.insert(did.to_host_name(), kernel_service_doc(pkg));
        }
        docs
    }

    fn kernel_service_doc(pkg_name: &str) -> EncodedDocument {
        EncodedDocument::JsonLd(json!({
            "pkg_name": pkg_name,
            "version": "0.1.0",
            "description": {},
            "pub_time": 0,
            "exp": DEFAULT_EXPIRE_TIME * 2,
            "deps": {},
            "author": "tester",
            "owner": "did:bns:tester",
            "show_name": pkg_name,
            "selector_type": "random"
        }))
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
