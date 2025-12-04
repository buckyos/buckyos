
use ::kRPC::*;
use crate::{KernelServiceDoc, SelectorType};
use name_lib::DID;
use package_lib::PackageMeta;
use serde_json::json;

pub const SCHEDULER_SERVICE_UNIQUE_ID: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_NAME: &str = "scheduler";
pub const SCHEDULER_SERVICE_SERVICE_PORT: u16 = 3220;
pub struct SchedulerClient {
    rpc_client: kRPC,
}

impl SchedulerClient {
    pub fn new(rpc_client: kRPC) -> Self {
        Self { rpc_client }
    }
}


pub fn generate_scheduler_service_doc() -> KernelServiceDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    let mut pkg_meta = PackageMeta::new(SCHEDULER_SERVICE_UNIQUE_ID, VERSION, "did:bns:buckyos",&owner_did, None);
    pkg_meta.description = json!("Scheduler is the core service of buckyos, controlling the scheduling of tasks and services");
    let doc = KernelServiceDoc {
        meta: pkg_meta,
        show_name: "Scheduler".to_string(),
        selector_type: SelectorType::Single,
    };
    return doc;
}

mod tests {
    use super::*;

    #[test]
    fn test_generate_scheduler_service_doc() {
        let doc = generate_scheduler_service_doc();
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }
}