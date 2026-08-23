#![allow(dead_code)]
use std::env;

use log::{info, warn};

mod content_mgr_client;
mod control_panel;
mod device_identity;
mod group_mgr;
mod msg_center_client;
pub mod msg_queue;
mod scheduler_client;
mod system_config;
mod task_dispatcher;
mod task_mgr;
mod taskdata;
mod thunk_object;
mod verify_hub_client;
pub mod workflow_dsl;
pub mod workflow_runtime;
mod workflow_service;
pub mod workflow_types;
mod zone_gateway;

mod aicc_client;
mod aicc_usage_log;
mod app_availability;
mod app_doc;
mod app_identity;
pub mod app_install;
mod app_mgr;
mod app_schema;
mod gateway_control;
mod kevent_bridge;
mod kevent_client;
mod kevent_ringbuffer;
pub mod network_observation;
pub mod node_control;
mod opendan_client;
mod permission;
mod rbac_config;
mod rdb_mgr;
mod repo_client;
mod runtime;
pub mod test_config;

pub use aicc_client::*;
pub use aicc_usage_log::*;
pub use app_availability::*;
pub use app_doc::*;
pub use app_identity::*;
pub use app_install::*;
pub use app_schema::*;
pub use content_mgr_client::*;
pub use control_panel::*;
pub use cyfs_gateway_api::{
    generate_sn_device_token, get_real_sn_host_name, sn_auth_login, sn_auth_register,
    sn_register_device_online, sn_resolve_ood_by_did, sn_resolve_ood_by_hostname,
    sn_update_device_online, SnAuthLoginReq, SnAuthRegisterReq, SnClient, SnDeviceOnlineReportReq,
    SnDnsRecordReq, SnHandler, SnServerHandler, SN_DEVICE_TOKEN_AUD,
    SN_DEVICE_TOKEN_DEFAULT_TTL_SECS,
};
pub use device_identity::*;
pub use group_mgr::*;
pub use msg_center_client::*;
pub use repo_client::*;
pub use scheduler_client::*;
pub use system_config::*;
pub use task_dispatcher::*;
pub use task_mgr::*;
pub use taskdata::*;
pub use thunk_object::*;
pub use verify_hub_client::*;
pub use workflow_dsl::*;
pub use workflow_runtime::*;
pub use workflow_service::*;
pub use workflow_types::*;
pub use zone_gateway::*;

pub use app_mgr::*;
pub use gateway_control::*;
pub use kevent_bridge::*;
pub use kevent_client::*;
pub use kevent_ringbuffer::*;
pub use network_observation::*;
pub use opendan_client::*;
pub use permission::*;
pub use rbac_config::*;
pub use rdb_mgr::*;
pub use runtime::*;

use ::kRPC::*;
use buckyos_kit::*;
use name_lib::DID;
use once_cell::sync::OnceCell;

pub const SMB_SERVICE_UNIQUE_ID: &str = "smb-service";
pub const SMB_SERVICE_SERVICE_NAME: &str = "smb-service";
pub const OPENDAN_SERVICE_UNIQUE_ID: &str = "opendan";
pub const OPENDAN_SERVICE_NAME: &str = "opendan";
pub const OPENDAN_SERVICE_PORT: u16 = 4060;

pub const BASE_APP_PORT: u16 = 10000;
pub const MAX_APP_INDEX: u16 = MAX_ALLOCATABLE_APP_INDEX;
pub const BUCKYOS_APP_DID_ENV: &str = "BUCKYOS_APP_DID";
pub const BUCKYOS_APP_ID_ENV: &str = "BUCKYOS_APP_ID";
pub const BUCKYOS_APP_INSTANCE_ID_ENV: &str = "BUCKYOS_APP_INSTANCE_ID";
pub const BUCKYOS_OWNER_USER_ID_ENV: &str = "BUCKYOS_OWNER_USER_ID";
pub const BUCKYOS_DATA_DIR_ENV: &str = "BUCKYOS_DATA_DIR";
pub const BUCKYOS_APP_TOKEN_ENV: &str = "BUCKYOS_APP_TOKEN";

static CURRENT_BUCKYOS_RUNTIME: OnceCell<BuckyOSRuntime> = OnceCell::new();
pub fn get_buckyos_api_runtime() -> Result<&'static BuckyOSRuntime> {
    CURRENT_BUCKYOS_RUNTIME.get().ok_or(RPCErrors::ReasonError(
        "BuckyOSRuntime is not initialized".to_string(),
    ))
}

pub fn set_buckyos_api_runtime(runtime: BuckyOSRuntime) -> Result<()> {
    CURRENT_BUCKYOS_RUNTIME
        .set(runtime)
        .map_err(|_| RPCErrors::ReasonError("BuckyOSRuntime is already registered".to_string()))?;
    if let Some(runtime) = CURRENT_BUCKYOS_RUNTIME.get() {
        runtime.start_registered_tasks_if_needed();
    }
    Ok(())
}

pub fn is_buckyos_api_runtime_set() -> bool {
    CURRENT_BUCKYOS_RUNTIME.get().is_some()
}

pub fn get_local_app_runtime_key(app_id: &str, owner_user_id: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{app_id}@{owner_user_id}").as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn get_service_session_token_env_key(service_id: &str) -> String {
    format!(
        "{}_SESSION_TOKEN",
        service_id.to_uppercase().replace('-', "_")
    )
}

pub fn parse_app_identity_from_instance_config(
    app_instance_config: &str,
) -> Result<(String, String)> {
    let config: AppServiceInstanceConfig =
        serde_json::from_str(app_instance_config).map_err(|err| {
            warn!(
                "parse app_instance_config failed: err={} bytes={}",
                err,
                app_instance_config.len()
            );
            RPCErrors::ReasonError(format!("parse app_instance_config failed: {}", err))
        })?;
    let app_id = config
        .node_execution_spec
        .app_instance_id
        .app_id()
        .to_string();
    let owner_user_id = config
        .node_execution_spec
        .app_instance_id
        .owner_user_id()
        .to_string();
    if app_id.is_empty() {
        warn!("app_instance_config parsed but node_execution_spec app_id is empty");
        return Err(RPCErrors::ReasonError(
            "app_instance_config.node_execution_spec.app_id is empty".to_string(),
        ));
    }
    if owner_user_id.is_empty() {
        warn!(
            "app_instance_config parsed for app_id={} but owner_user_id is empty",
            app_id
        );
        return Err(RPCErrors::ReasonError(
            "app_instance_config.owner_user_id is empty".to_string(),
        ));
    }
    info!(
        "resolved app identity from app_instance_config: app_id={} owner_user_id={}",
        app_id, owner_user_id
    );
    Ok((app_id, owner_user_id))
}

pub fn load_app_identity_from_env() -> Result<Option<(String, String)>> {
    let read = |key: &str| -> Result<Option<String>> {
        match env::var(key) {
            Ok(value) if value.trim().is_empty() => {
                Err(RPCErrors::ReasonError(format!("{key} is set but empty")))
            }
            Ok(value) => Ok(Some(value)),
            Err(env::VarError::NotPresent) => Ok(None),
            Err(error) => Err(RPCErrors::ReasonError(format!(
                "read {key} from env failed: {error}"
            ))),
        }
    };
    let app_did = read(BUCKYOS_APP_DID_ENV)?;
    let app_id = read(BUCKYOS_APP_ID_ENV)?;
    let app_instance_id = read(BUCKYOS_APP_INSTANCE_ID_ENV)?;
    let owner_user_id = read(BUCKYOS_OWNER_USER_ID_ENV)?;
    let data_dir = read(BUCKYOS_DATA_DIR_ENV)?;
    if app_did.is_none()
        && app_id.is_none()
        && app_instance_id.is_none()
        && owner_user_id.is_none()
        && data_dir.is_none()
    {
        return Ok(None);
    }
    let app_did = app_did
        .ok_or_else(|| RPCErrors::ReasonError(format!("{BUCKYOS_APP_DID_ENV} is required")))?;
    let app_id = app_id
        .ok_or_else(|| RPCErrors::ReasonError(format!("{BUCKYOS_APP_ID_ENV} is required")))?;
    let app_instance_id = app_instance_id.ok_or_else(|| {
        RPCErrors::ReasonError(format!("{BUCKYOS_APP_INSTANCE_ID_ENV} is required"))
    })?;
    let owner_user_id = owner_user_id.ok_or_else(|| {
        RPCErrors::ReasonError(format!("{BUCKYOS_OWNER_USER_ID_ENV} is required"))
    })?;
    let data_dir = data_dir
        .ok_or_else(|| RPCErrors::ReasonError(format!("{BUCKYOS_DATA_DIR_ENV} is required")))?;
    if !std::path::Path::new(&data_dir).is_absolute() {
        return Err(RPCErrors::ReasonError(format!(
            "{BUCKYOS_DATA_DIR_ENV} must be an absolute path"
        )));
    }

    let parsed_app_id = AppId::parse(&app_id).map_err(|error| {
        RPCErrors::ReasonError(format!("invalid {BUCKYOS_APP_ID_ENV}: {error}"))
    })?;
    let parsed_instance = app_instance_id.parse::<AppInstanceId>().map_err(|error| {
        RPCErrors::ReasonError(format!("invalid {BUCKYOS_APP_INSTANCE_ID_ENV}: {error}"))
    })?;
    let parsed_did = DID::from_str(&app_did).map_err(|error| {
        RPCErrors::ReasonError(format!("invalid {BUCKYOS_APP_DID_ENV}: {error}"))
    })?;
    if parsed_instance.app_id() != &parsed_app_id
        || parsed_instance.owner_user_id() != owner_user_id
        || parsed_app_id.app_did() != parsed_did
    {
        return Err(RPCErrors::ReasonError(
            "app identity environment variables are inconsistent".to_string(),
        ));
    }
    Ok(Some((app_id, owner_user_id)))
}

pub async fn init_buckyos_api_runtime(
    app_id: &str,
    app_owner_id: Option<String>,
    runtime_type: BuckyOSRuntimeType,
) -> Result<BuckyOSRuntime> {
    if CURRENT_BUCKYOS_RUNTIME.get().is_some() {
        return Err(RPCErrors::ReasonError(
            "BuckyOSRuntime already initialized".to_string(),
        ));
    }

    let mut resolved_app_id = app_id.trim().to_string();
    let mut resolved_owner_id = app_owner_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    info!(
        "init_buckyos_api_runtime: runtime_type={:?} input_app_id={} input_owner_user_id={}",
        runtime_type,
        if resolved_app_id.is_empty() {
            "<empty>"
        } else {
            resolved_app_id.as_str()
        },
        resolved_owner_id.as_deref().unwrap_or("<none>")
    );

    if runtime_type == BuckyOSRuntimeType::AppService {
        let (env_app_id, env_owner_id) = load_app_identity_from_env()?.ok_or_else(|| {
            RPCErrors::ReasonError(
                "fixed BuckyOS AppService identity environment is required".into(),
            )
        })?;
        if !resolved_app_id.is_empty() && resolved_app_id != env_app_id {
            return Err(RPCErrors::ReasonError(format!(
                "runtime app_id {resolved_app_id} does not match {BUCKYOS_APP_ID_ENV} {env_app_id}"
            )));
        }
        if resolved_owner_id
            .as_deref()
            .is_some_and(|owner| owner != env_owner_id)
        {
            return Err(RPCErrors::ReasonError(format!(
                "runtime owner_user_id {} does not match {BUCKYOS_OWNER_USER_ID_ENV} {env_owner_id}",
                resolved_owner_id.as_deref().unwrap_or_default()
            )));
        }
        resolved_app_id = env_app_id;
        resolved_owner_id = Some(env_owner_id);
    }

    if resolved_app_id.is_empty() {
        warn!(
            "init_buckyos_api_runtime failed: runtime_type={:?} resolved app_id is empty",
            runtime_type
        );
        return Err(RPCErrors::ReasonError(
            "app_id is required for runtime init".to_string(),
        ));
    }

    if runtime_type == BuckyOSRuntimeType::AppService && resolved_owner_id.is_none() {
        warn!(
            "init_buckyos_api_runtime failed: runtime_type={:?} app_id={} owner_user_id is missing",
            runtime_type, resolved_app_id
        );
        return Err(RPCErrors::ReasonError(
            "owner_user_id is required for AppService".to_string(),
        ));
    }
    info!(
        "init_buckyos_api_runtime resolved identity: runtime_type={:?} app_id={} owner_user_id={}",
        runtime_type,
        resolved_app_id,
        resolved_owner_id.as_deref().unwrap_or("<none>")
    );

    let mut runtime = BuckyOSRuntime::new(
        resolved_app_id.as_str(),
        resolved_owner_id,
        runtime_type.clone(),
    );
    let token_authenticated_appclient = runtime_type == BuckyOSRuntimeType::AppClient
        && env::var(BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV)
            .ok()
            .is_some_and(|token| !token.trim().is_empty());
    if token_authenticated_appclient {
        if let Err(error) = runtime.fill_policy_by_load_config().await {
            info!(
                "token-authenticated AppClient has no local runtime policy config; using environment/default policy: {}",
                error
            );
        }
    } else {
        runtime.fill_policy_by_load_config().await?;

        if runtime_type == BuckyOSRuntimeType::Kernel
            || runtime_type == BuckyOSRuntimeType::AppClient
            || runtime_type == BuckyOSRuntimeType::KernelService
            || runtime_type == BuckyOSRuntimeType::FrameService
        {
            runtime.fill_by_load_config().await?;
        }
    }
    runtime.fill_by_env_var().await?;

    Ok(runtime)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::env;
    use std::fs;
    use std::sync::{Mutex, OnceLock};
    use std::time::{SystemTime, UNIX_EPOCH};

    use name_lib::DID;

    use super::{
        init_buckyos_api_runtime, parse_app_identity_from_instance_config, AppDoc, AppId,
        AppInstanceId, AppServiceInstanceConfig, AppType, BuckyOSRuntimeType, DeploymentIdentity,
        NodeExecutionSpec, ServiceInstanceState, ServiceSpecConfig, SubPkgDesc,
        BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV, BUCKYOS_APP_DID_ENV, BUCKYOS_APP_ID_ENV,
        BUCKYOS_APP_INSTANCE_ID_ENV, BUCKYOS_APP_TOKEN_ENV, BUCKYOS_DATA_DIR_ENV,
        BUCKYOS_OWNER_USER_ID_ENV, NODE_EXECUTION_SPEC_SCHEMA_VERSION, OBJ_TYPE_APP_DOC,
    };

    fn test_env_lock() -> &'static Mutex<()> {
        static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    fn set_env_var(key: &str, value: &str) -> Option<String> {
        let previous = env::var(key).ok();
        env::set_var(key, value);
        previous
    }

    fn restore_env_var(key: &str, previous: Option<String>) {
        if let Some(value) = previous {
            env::set_var(key, value);
        } else {
            env::remove_var(key);
        }
    }

    #[test]
    fn parse_app_identity_from_instance_config_extracts_app_and_owner() {
        let owner_did = DID::from_str("did:bns:devtest").expect("parse owner did");
        let app_doc = AppDoc::builder(
            AppType::Agent,
            "buckyos-jarvis",
            "0.1.0",
            "did:bns:devtest",
            &owner_did,
        )
        .show_name("Jarvis")
        .agent_pkg(
            SubPkgDesc::new("agent.buckyos-jarvis.devtest.bns.did#0.1.0")
                .package_meta_object_id(ndn_lib::ObjId::new_by_raw("pkg".to_string(), vec![1; 32])),
        )
        .build()
        .expect("build app doc");
        let app_instance_id = AppInstanceId::from_app_did(app_doc.app_did(), "devtest").unwrap();
        let app_doc_value = serde_json::to_value(&app_doc).unwrap();
        let (app_doc_object_id, _) =
            ndn_lib::build_named_object_by_json(OBJ_TYPE_APP_DOC, &app_doc_value);
        let config = AppServiceInstanceConfig {
            target_state: ServiceInstanceState::Started,
            node_id: "ood1".to_string(),
            node_execution_spec: NodeExecutionSpec {
                schema_version: NODE_EXECUTION_SPEC_SCHEMA_VERSION,
                app_instance_id: app_instance_id.clone(),
                app_did: app_doc.app_did().clone(),
                app_doc_object_id: app_doc_object_id.clone(),
                spec_generation: 1,
                app_type: app_doc.app_type,
                packages: std::collections::BTreeMap::new(),
                permission: app_doc.permissions.clone(),
                service_spec_config: ServiceSpecConfig::default(),
                app_name: "buckyos-jarvis".to_string(),
                app_host_name: "buckyos-jarvis".to_string(),
                app_index: 1,
            },
            service_ports_config: HashMap::from([("www".to_string(), 10016)]),
            deployment: DeploymentIdentity {
                app_instance_id,
                task_id: "test:install".to_string(),
                app_doc_object_id,
                spec_generation: 1,
                pikg_digest: None,
            },
        };
        let raw = serde_json::to_string(&config).expect("serialize app_instance_config");

        let (app_id, owner_user_id) =
            parse_app_identity_from_instance_config(&raw).expect("parse app_instance_config");
        assert_eq!(app_id, "buckyos-jarvis.devtest.bns.did");
        assert_eq!(owner_user_id, "devtest");
    }

    #[test]
    fn runtime_key_uses_full_canonical_app_instance_sha256() {
        let app_instance_id =
            AppInstanceId::new(AppId::parse("filebrowser.buckyos.ai").unwrap(), "alice").unwrap();
        assert_eq!(
            app_instance_id.runtime_key(),
            "0f77133700c08ac0aff571f1b710c5ade021d76b9a4a86477887f7d319c90768"
        );
    }

    #[tokio::test]
    async fn init_app_service_runtime_skips_system_etc_and_uses_env_bootstrap() {
        let _lock = test_env_lock().lock().expect("lock env");
        let app_id = "buckyos-jarvis.devtest.bns.did";
        let app_instance_id = format!("{app_id}@devtest");
        let missing_root = env::temp_dir().join(format!(
            "buckyos-appservice-runtime-missing-root-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("unix epoch")
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&missing_root);
        assert!(!missing_root.exists(), "test root should not exist");

        let prev_root = set_env_var("BUCKYOS_ROOT", missing_root.to_string_lossy().as_ref());
        let prev_app_did = set_env_var(BUCKYOS_APP_DID_ENV, "did:bns:buckyos-jarvis.devtest");
        let prev_app_id = set_env_var(BUCKYOS_APP_ID_ENV, app_id);
        let prev_instance_id = set_env_var(BUCKYOS_APP_INSTANCE_ID_ENV, &app_instance_id);
        let prev_owner = set_env_var(BUCKYOS_OWNER_USER_ID_ENV, "devtest");
        let prev_data_dir = set_env_var(
            BUCKYOS_DATA_DIR_ENV,
            missing_root.join("data").to_string_lossy().as_ref(),
        );
        let prev_token = set_env_var(BUCKYOS_APP_TOKEN_ENV, "dummy-session-token");

        let result = init_buckyos_api_runtime("", None, BuckyOSRuntimeType::AppService).await;

        restore_env_var(BUCKYOS_APP_TOKEN_ENV, prev_token);
        restore_env_var(BUCKYOS_DATA_DIR_ENV, prev_data_dir);
        restore_env_var(BUCKYOS_OWNER_USER_ID_ENV, prev_owner);
        restore_env_var(BUCKYOS_APP_INSTANCE_ID_ENV, prev_instance_id);
        restore_env_var(BUCKYOS_APP_ID_ENV, prev_app_id);
        restore_env_var(BUCKYOS_APP_DID_ENV, prev_app_did);
        restore_env_var("BUCKYOS_ROOT", prev_root);

        let runtime = result.expect("init app service runtime should succeed without system etc");
        assert_eq!(runtime.get_app_id(), app_id);
        assert_eq!(runtime.get_owner_user_id().as_deref(), Some("devtest"));
        assert_eq!(runtime.user_id.as_deref(), Some("devtest"));
        assert_eq!(runtime.get_authenticated_user_id().as_deref(), None);
        assert_eq!(
            runtime.session_token.read().await.as_str(),
            "dummy-session-token"
        );
    }

    #[tokio::test]
    async fn init_appclient_runtime_uses_appclient_session_token_env() {
        let _lock = test_env_lock().lock().expect("lock env");
        let temp_root = env::temp_dir().join(format!(
            "buckyos-appclient-runtime-root-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("unix epoch")
                .as_nanos()
        ));
        let dev_home = temp_root.join("missing_dev_home");

        let prev_root = set_env_var("BUCKYOS_ROOT", temp_root.to_string_lossy().as_ref());
        let prev_dev_home = set_env_var("BUCKYOS_DEV_HOME", dev_home.to_string_lossy().as_ref());
        let prev_token = set_env_var(BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV, "dummy-appclient-token");

        let result =
            init_buckyos_api_runtime("buckycli", None, BuckyOSRuntimeType::AppClient).await;

        restore_env_var(BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV, prev_token);
        restore_env_var("BUCKYOS_DEV_HOME", prev_dev_home);
        restore_env_var("BUCKYOS_ROOT", prev_root);
        let _ = fs::remove_dir_all(&temp_root);

        let runtime = result.expect("init appclient runtime should load env session token");
        assert_eq!(runtime.get_app_id(), "buckycli");
        assert!(runtime.device_private_key.is_none());
        assert!(runtime.user_private_key.is_none());
        assert_eq!(
            runtime.session_token.read().await.as_str(),
            "dummy-appclient-token"
        );
    }
}

pub fn generate_smb_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        SMB_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Samba Service")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

pub fn generate_opendan_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        OPENDAN_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("OpenDAN Runtime")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}
