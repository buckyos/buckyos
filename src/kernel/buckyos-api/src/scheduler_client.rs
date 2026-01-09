
use ::kRPC::*;
use crate::{AppDoc, AppType, SelectorType};
use name_lib::DID;

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


pub fn generate_scheduler_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        SCHEDULER_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Scheduler")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}

mod tests {

    #[test]
    fn test_generate_scheduler_service_doc() {
        use super::generate_scheduler_service_doc;
        let doc = generate_scheduler_service_doc();
        let json_str = serde_json::to_string_pretty(&doc).unwrap();
        println!("json: {}", json_str);
    }
}
