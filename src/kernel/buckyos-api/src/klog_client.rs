use crate::{AppDoc, AppType, SelectorType};
use name_lib::DID;

pub const KLOG_SERVICE_UNIQUE_ID: &str = "klog-service";
pub const KLOG_SERVICE_NAME: &str = "klog-service";
pub const KLOG_SERVICE_PORT: u16 = 4070;

pub fn generate_klog_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        KLOG_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Kernel Log Service")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_klog_service_doc() {
        let doc = generate_klog_service_doc();
        assert_eq!(doc.name, KLOG_SERVICE_UNIQUE_ID);
        assert_eq!(doc.selector_type, SelectorType::Random);
    }
}
