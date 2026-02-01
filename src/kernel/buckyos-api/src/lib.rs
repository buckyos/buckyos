#![allow(dead_code)]
mod control_panel;
mod scheduler_client;
mod sn_client;
mod system_config;
mod task_mgr;
mod verify_hub_client;
mod zone_gateway;

mod app_doc;
mod app_mgr;
mod gateway_control;
mod permission;
mod repo_client;
mod runtime;
pub mod test_config;

pub use app_doc::*;
pub use control_panel::*;
pub use repo_client::*;
pub use scheduler_client::*;
pub use sn_client::*;
pub use system_config::*;
pub use task_mgr::*;
pub use verify_hub_client::*;
pub use zone_gateway::*;

pub use app_mgr::*;
pub use gateway_control::*;
pub use permission::*;
pub use runtime::*;

use ::kRPC::*;
use buckyos_kit::*;
use name_lib::DID;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub const SMB_SERVICE_UNIQUE_ID: &str = "smb-service";
pub const SMB_SERVICE_SERVICE_NAME: &str = "smb-service";

pub const BASE_APP_PORT: u16 = 10000;
pub const MAX_APP_INDEX: u16 = 2048;

static CURRENT_BUCKYOS_RUNTIME: OnceCell<BuckyOSRuntime> = OnceCell::new();
pub fn get_buckyos_api_runtime() -> Result<&'static BuckyOSRuntime> {
    let runtime = CURRENT_BUCKYOS_RUNTIME.get().unwrap();
    Ok(runtime)
}

pub fn set_buckyos_api_runtime(runtime: BuckyOSRuntime) {
    let _ = CURRENT_BUCKYOS_RUNTIME.set(runtime);
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

    match runtime_type {
        BuckyOSRuntimeType::AppService => {
            if app_owner_id.is_none() {
                return Err(RPCErrors::ReasonError(
                    "owner_user_id is required for AppClient or AppService".to_string(),
                ));
            }
        }
        BuckyOSRuntimeType::FrameService => {
            return Err(RPCErrors::ReasonError(
                "FrameService is not supported yet".to_string(),
            ));
        }
        _ => {
            //do nothing
        }
    }

    let mut runtime = BuckyOSRuntime::new(app_id, app_owner_id, runtime_type.clone());
    runtime.fill_policy_by_load_config().await?;

    if runtime_type == BuckyOSRuntimeType::Kernel
        || runtime_type == BuckyOSRuntimeType::AppClient
        || runtime_type == BuckyOSRuntimeType::KernelService
    {
        runtime.fill_by_load_config().await?;
    }
    runtime.fill_by_env_var().await?;

    Ok(runtime)
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
