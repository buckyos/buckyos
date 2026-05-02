//! Workflow Service 注册元数据。
//!
//! 与 task-manager / msg-center / aicc 等其他 kernel service 保持一致：
//! unique id / 端口 / [`AppDoc`] 生成集中放在 buckyos-api，
//! workflow service 自身和 scheduler 都从这里取，避免常量分裂。

use crate::{AppDoc, AppType, SelectorType};
use name_lib::DID;

pub const WORKFLOW_SERVICE_UNIQUE_ID: &str = "workflow";
pub const WORKFLOW_SERVICE_NAME: &str = "workflow";
pub const WORKFLOW_SERVICE_PORT: u16 = 4070;
pub const WORKFLOW_SERVICE_HTTP_PATH: &str = "/kapi/workflow";

pub fn generate_workflow_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        WORKFLOW_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Workflow Service")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}
