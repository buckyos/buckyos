pub(crate) mod api;
pub(crate) mod call;
pub(crate) mod catalog;
pub(crate) mod error;
pub(crate) mod execution;
pub(crate) mod matching;
pub(crate) mod model;
pub(crate) mod observability;
pub(crate) mod protocol;
pub(crate) mod provider;
pub(crate) mod resource;
pub(crate) mod routing;
pub(crate) mod runtime;
pub(crate) mod service;
pub(crate) mod settings;
pub(crate) mod storage;

pub async fn run_service() -> anyhow::Result<()> {
    service::run_service().await
}
