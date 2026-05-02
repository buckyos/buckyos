#![allow(dead_code)]
use std::env;

use log::{info, warn};

mod content_mgr_client;
mod control_panel;
mod group_mgr;
mod msg_center_client;
pub mod msg_queue;
mod scheduler_client;
mod sn_client;
mod system_config;
mod task_mgr;
mod thunk_object;
mod verify_hub_client;
pub mod workflow_dsl;
pub mod workflow_runtime;
mod workflow_service;
pub mod workflow_types;
mod zone_gateway;

mod aicc_client;
mod aicc_usage_log;
mod app_doc;
mod app_mgr;
mod gateway_control;
mod kevent_client;
mod kevent_ringbuffer;
pub mod network_observation;
mod opendan_client;
mod permission;
mod rdb_mgr;
mod repo_client;
mod runtime;
pub mod test_config;

pub use aicc_client::*;
pub use aicc_usage_log::*;
pub use app_doc::*;
pub use content_mgr_client::*;
pub use control_panel::*;
pub use group_mgr::*;
pub use msg_center_client::*;
pub use repo_client::*;
pub use scheduler_client::*;
pub use sn_client::*;
pub use system_config::*;
pub use task_mgr::*;
pub use thunk_object::*;
pub use verify_hub_client::*;
pub use workflow_dsl::*;
pub use workflow_runtime::*;
pub use workflow_service::*;
pub use workflow_types::*;
pub use zone_gateway::*;

pub use app_mgr::*;
pub use gateway_control::*;
pub use kevent_client::*;
pub use kevent_ringbuffer::*;
pub use network_observation::*;
pub use opendan_client::*;
pub use permission::*;
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
pub const MAX_APP_INDEX: u16 = 2048;

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

pub fn get_full_appid(app_id: &str, owner_user_id: &str) -> String {
    format!("{}-{}", owner_user_id, app_id)
}

pub fn get_session_token_env_key(app_full_id: &str, is_app_service: bool) -> String {
    let app_id = app_full_id.to_uppercase();
    let app_id = app_id.replace("-", "_");
    if !is_app_service {
        format!("{}_SESSION_TOKEN", app_id)
    } else {
        format!("{}_TOKEN", app_id)
    }
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
    let app_id = config.app_spec.app_id().trim().to_string();
    let owner_user_id = config.app_spec.user_id.trim().to_string();
    if app_id.is_empty() {
        warn!("app_instance_config parsed but app_spec.app_id is empty");
        return Err(RPCErrors::ReasonError(
            "app_instance_config.app_spec.app_id is empty".to_string(),
        ));
    }
    if owner_user_id.is_empty() {
        warn!(
            "app_instance_config parsed for app_id={} but app_spec.user_id is empty",
            app_id
        );
        return Err(RPCErrors::ReasonError(
            "app_instance_config.app_spec.user_id is empty".to_string(),
        ));
    }
    info!(
        "resolved app identity from app_instance_config: app_id={} owner_user_id={}",
        app_id, owner_user_id
    );
    Ok((app_id, owner_user_id))
}

pub fn load_app_identity_from_env() -> Result<Option<(String, String)>> {
    let app_instance_config = match env::var("app_instance_config") {
        Ok(value) => {
            info!("found app_instance_config in env, bytes={}", value.len());
            value
        }
        Err(env::VarError::NotPresent) => {
            info!("app_instance_config not found in env");
            return Ok(None);
        }
        Err(err) => {
            warn!("read app_instance_config from env failed: {}", err);
            return Err(RPCErrors::ReasonError(format!(
                "read app_instance_config from env failed: {}",
                err
            )));
        }
    };
    parse_app_identity_from_instance_config(&app_instance_config).map(Some)
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

    if (resolved_app_id.is_empty() || resolved_owner_id.is_none())
        && matches!(
            runtime_type,
            BuckyOSRuntimeType::AppService | BuckyOSRuntimeType::FrameService
        )
    {
        if let Some((env_app_id, env_owner_id)) = load_app_identity_from_env()? {
            if resolved_app_id.is_empty() {
                info!(
                    "init_buckyos_api_runtime: app_id missing, using app_instance_config value={}",
                    env_app_id
                );
                resolved_app_id = env_app_id;
            }
            if resolved_owner_id.is_none() {
                info!(
                    "init_buckyos_api_runtime: owner_user_id missing, using app_instance_config value={}",
                    env_owner_id
                );
                resolved_owner_id = Some(env_owner_id);
            }
        }
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
            "owner_user_id is required for AppClient or AppService".to_string(),
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
    runtime.fill_policy_by_load_config().await?;

    if runtime_type == BuckyOSRuntimeType::Kernel
        || runtime_type == BuckyOSRuntimeType::AppClient
        || runtime_type == BuckyOSRuntimeType::KernelService
        || runtime_type == BuckyOSRuntimeType::FrameService
    {
        runtime.fill_by_load_config().await?;
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
        get_full_appid, get_session_token_env_key, init_buckyos_api_runtime,
        parse_app_identity_from_instance_config, AppDoc, AppServiceInstanceConfig, AppServiceSpec,
        AppType, BuckyOSRuntimeType, ServiceInstallConfig, ServiceInstanceState, ServiceState,
        SubPkgDesc,
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
            "jarvis",
            "0.1.0",
            "did:bns:devtest",
            &owner_did,
        )
        .show_name("Jarvis")
        .agent_pkg(SubPkgDesc::new("jarvis-agent#0.1.0"))
        .build()
        .expect("build app doc");
        let config = AppServiceInstanceConfig {
            target_state: ServiceInstanceState::Started,
            node_id: "ood1".to_string(),
            app_spec: AppServiceSpec {
                app_doc,
                app_index: 1,
                user_id: "devtest".to_string(),
                enable: true,
                expected_instance_count: 1,
                state: ServiceState::Running,
                install_config: ServiceInstallConfig::default(),
            },
            service_ports_config: HashMap::from([("www".to_string(), 10016)]),
        };
        let raw = serde_json::to_string(&config).expect("serialize app_instance_config");

        let (app_id, owner_user_id) =
            parse_app_identity_from_instance_config(&raw).expect("parse app_instance_config");
        assert_eq!(app_id, "jarvis");
        assert_eq!(owner_user_id, "devtest");
    }

    #[tokio::test]
    async fn init_app_service_runtime_skips_system_etc_and_uses_env_bootstrap() {
        let _lock = test_env_lock().lock().expect("lock env");
        let token_key = get_session_token_env_key(&get_full_appid("jarvis", "devtest"), true);
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
        let prev_token = set_env_var(&token_key, "dummy-session-token");

        let result = init_buckyos_api_runtime(
            "jarvis",
            Some("devtest".to_string()),
            BuckyOSRuntimeType::AppService,
        )
        .await;

        restore_env_var(&token_key, prev_token);
        restore_env_var("BUCKYOS_ROOT", prev_root);

        let runtime = result.expect("init app service runtime should succeed without system etc");
        assert_eq!(runtime.get_app_id(), "jarvis");
        assert_eq!(runtime.get_owner_user_id().as_deref(), Some("devtest"));
        assert_eq!(runtime.user_id.as_deref(), Some("devtest"));
        assert_eq!(runtime.get_authenticated_user_id().as_deref(), None);
        assert_eq!(
            runtime.session_token.read().await.as_str(),
            "dummy-session-token"
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
