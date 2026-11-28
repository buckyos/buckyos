pub(crate) mod api;
pub(crate) mod call;
pub(crate) mod canonical;
pub(crate) mod catalog;
pub(crate) mod error;
pub(crate) mod execution;
pub(crate) mod health;
pub(crate) mod matching;
pub(crate) mod media;
pub(crate) mod model;
pub(crate) mod observability;
pub(crate) mod protocol;
pub(crate) mod policy;
pub(crate) mod provider;
pub(crate) mod resource;
pub(crate) mod routing;
pub(crate) mod runtime;
pub(crate) mod service;
pub(crate) mod settings;
pub(crate) mod storage;
pub(crate) mod video;

pub async fn run_service() -> anyhow::Result<()> {
    service::run_service().await
}

/// Validate just the owned System Config value, without booting AICC or making RPCs.
pub fn validate_persisted_settings(key: &str, contents: &str) -> Result<(), String> {
    settings::validate_persisted(key, contents).map_err(|e| e.to_string())
}
