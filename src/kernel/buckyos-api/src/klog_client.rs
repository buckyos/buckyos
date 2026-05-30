use crate::{AppDoc, AppType, SelectorType};
pub use ::klog::error::{KLogErrorCode, KLogErrorEnvelope};
pub use ::klog::network::{
    KLogAppendRequest, KLogAppendResponse, KLogMetaDeleteRequest, KLogMetaDeleteResponse,
    KLogMetaPutRequest, KLogMetaPutResponse, KLogMetaQueryRequest, KLogMetaQueryResponse,
    KLogQueryRequest, KLogQueryResponse,
};
use ::klog::rpc::KLogClient as KLogRpcClient;
pub use ::klog::rpc::{KLogCallTrace, KLogClientError};
pub use ::klog::{
    KLogEntry, KLogLevel, KLogMetaEntry, KLogMetaTxAction, KLogMetaTxGuard, KLogMetaTxRequest,
    KLogMetaTxResponse, KNode, KNodeId,
};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const KLOG_SERVICE_UNIQUE_ID: &str = "klog-service";
pub const KLOG_SERVICE_NAME: &str = "klog-service";
pub const KLOG_SERVICE_PORT: u16 = 4080;
pub const KLOG_CLUSTER_RAFT_SERVICE_NAME: &str = "raft";
pub const KLOG_CLUSTER_INTER_SERVICE_NAME: &str = "inter";
pub const KLOG_CLUSTER_ADMIN_SERVICE_NAME: &str = "admin";
pub const KLOG_CLUSTER_RAFT_PORT: u16 = 21001;
pub const KLOG_CLUSTER_INTER_PORT: u16 = 21002;
pub const KLOG_CLUSTER_ADMIN_PORT: u16 = 21003;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KLogDeploymentMode {
    #[serde(rename = "ood_voters")]
    OODVoters,
}

impl Default for KLogDeploymentMode {
    fn default() -> Self {
        Self::OODVoters
    }
}

impl KLogDeploymentMode {
    pub fn is_ood_voters(self) -> bool {
        matches!(self, Self::OODVoters)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KLogDeploymentSettings {
    #[serde(default)]
    pub mode: KLogDeploymentMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct KLogBuckyosSettings {
    #[serde(default)]
    pub deployment: KLogDeploymentSettings,
}

pub fn default_klog_buckyos_settings() -> KLogBuckyosSettings {
    KLogBuckyosSettings::default()
}

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
    .service_port("www", KLOG_SERVICE_PORT)
    .service_port(KLOG_CLUSTER_RAFT_SERVICE_NAME, KLOG_CLUSTER_RAFT_PORT)
    .service_port(KLOG_CLUSTER_INTER_SERVICE_NAME, KLOG_CLUSTER_INTER_PORT)
    .service_port(KLOG_CLUSTER_ADMIN_SERVICE_NAME, KLOG_CLUSTER_ADMIN_PORT)
    .build()
    .unwrap()
}

pub struct KLogClient {
    inner: KLogRpcClient,
}

impl KLogClient {
    pub fn new(endpoint: impl Into<String>, request_node_name: impl Into<String>) -> Self {
        Self {
            inner: KLogRpcClient::new(endpoint, request_node_name),
        }
    }

    pub fn from_daemon_addr(addr: &str, request_node_name: impl Into<String>) -> Self {
        Self {
            inner: KLogRpcClient::from_daemon_addr(addr, request_node_name),
        }
    }

    pub fn from_buckyos_service_addr(addr: &str, request_node_name: impl Into<String>) -> Self {
        Self {
            inner: KLogRpcClient::from_buckyos_service_addr(addr, request_node_name),
        }
    }

    pub fn from_buckyos_service_url(
        url: impl Into<String>,
        request_node_name: impl Into<String>,
    ) -> Self {
        Self::new(url, request_node_name)
    }

    pub fn local_default(request_node_name: impl Into<String>) -> Self {
        Self {
            inner: KLogRpcClient::local_default(request_node_name),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.inner = self.inner.with_timeout(timeout);
        self
    }

    pub fn generate_request_id(node_name: &str) -> String {
        KLogRpcClient::generate_request_id(node_name)
    }

    pub fn inner(&self) -> &KLogRpcClient {
        &self.inner
    }

    pub fn into_inner(self) -> KLogRpcClient {
        self.inner
    }

    pub async fn append_log(
        &self,
        req: KLogAppendRequest,
    ) -> Result<KLogAppendResponse, KLogClientError> {
        self.inner.append_log(req).await
    }

    pub async fn append_log_with_trace(
        &self,
        req: KLogAppendRequest,
    ) -> Result<(KLogAppendResponse, KLogCallTrace), KLogClientError> {
        self.inner.append_log_with_trace(req).await
    }

    pub async fn append_log_message(
        &self,
        message: impl Into<String>,
    ) -> Result<u64, KLogClientError> {
        self.inner.append_log_message(message).await
    }

    pub async fn query_log(
        &self,
        req: KLogQueryRequest,
    ) -> Result<KLogQueryResponse, KLogClientError> {
        self.inner.query_log(req).await
    }

    pub async fn query_log_with_trace(
        &self,
        req: KLogQueryRequest,
    ) -> Result<(KLogQueryResponse, KLogCallTrace), KLogClientError> {
        self.inner.query_log_with_trace(req).await
    }

    pub async fn put_meta(
        &self,
        req: KLogMetaPutRequest,
    ) -> Result<KLogMetaPutResponse, KLogClientError> {
        self.inner.put_meta(req).await
    }

    pub async fn put_meta_with_trace(
        &self,
        req: KLogMetaPutRequest,
    ) -> Result<(KLogMetaPutResponse, KLogCallTrace), KLogClientError> {
        self.inner.put_meta_with_trace(req).await
    }

    pub async fn delete_meta(
        &self,
        req: KLogMetaDeleteRequest,
    ) -> Result<KLogMetaDeleteResponse, KLogClientError> {
        self.inner.delete_meta(req).await
    }

    pub async fn delete_meta_with_trace(
        &self,
        req: KLogMetaDeleteRequest,
    ) -> Result<(KLogMetaDeleteResponse, KLogCallTrace), KLogClientError> {
        self.inner.delete_meta_with_trace(req).await
    }

    pub async fn exec_meta_tx(
        &self,
        req: KLogMetaTxRequest,
    ) -> Result<KLogMetaTxResponse, KLogClientError> {
        self.inner.exec_meta_tx(req).await
    }

    pub async fn exec_meta_tx_with_trace(
        &self,
        req: KLogMetaTxRequest,
    ) -> Result<(KLogMetaTxResponse, KLogCallTrace), KLogClientError> {
        self.inner.exec_meta_tx_with_trace(req).await
    }

    pub async fn query_meta(
        &self,
        req: KLogMetaQueryRequest,
    ) -> Result<KLogMetaQueryResponse, KLogClientError> {
        self.inner.query_meta(req).await
    }

    pub async fn query_meta_with_trace(
        &self,
        req: KLogMetaQueryRequest,
    ) -> Result<(KLogMetaQueryResponse, KLogCallTrace), KLogClientError> {
        self.inner.query_meta_with_trace(req).await
    }
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

    #[test]
    fn test_klog_service_constants() {
        assert_eq!(::klog::rpc::KLOG_JSON_RPC_PATH, "/klog/rpc");
        assert_eq!(
            ::klog::rpc::KLOG_JSON_RPC_SERVICE_PATH,
            "/kapi/klog-service"
        );
        assert_eq!(KLOG_CLUSTER_RAFT_SERVICE_NAME, "raft");
        assert_eq!(KLOG_CLUSTER_INTER_SERVICE_NAME, "inter");
        assert_eq!(KLOG_CLUSTER_ADMIN_SERVICE_NAME, "admin");
        assert_eq!(KLOG_SERVICE_PORT, 4080);
        assert_eq!(KLOG_CLUSTER_RAFT_PORT, 21001);
        assert_eq!(KLOG_CLUSTER_INTER_PORT, 21002);
        assert_eq!(KLOG_CLUSTER_ADMIN_PORT, 21003);
    }

    #[test]
    fn test_default_klog_buckyos_settings() {
        let settings = default_klog_buckyos_settings();
        assert!(settings.deployment.mode.is_ood_voters());

        let value = serde_json::to_value(&settings).unwrap();
        assert_eq!(value["deployment"]["mode"], "ood_voters");
    }
}
