use crate::catalog::{CatalogSnapshot, ModelIdentity, ProviderModelMatch};
use crate::provider::{
    DiscoveryContext, ProviderDiscovery, ProviderDiscoverySnapshot, ProviderResult,
};
use async_trait::async_trait;
use std::sync::Arc;

pub(super) struct TypeSafeCatalogDiscovery(pub Arc<dyn ProviderDiscovery>);

#[async_trait]
impl ProviderDiscovery for TypeSafeCatalogDiscovery {
    fn match_model_driver(&self, id: &str, _catalog: &CatalogSnapshot) -> ProviderModelMatch {
        if matches!(id, "jev-latest" | "jev-preview") {
            ProviderModelMatch::Matched(ModelIdentity {
                model_driver_id: "typesafe".into(),
                model_id: "jev-1.13.0".into(),
            })
        } else {
            ProviderModelMatch::NotHandled
        }
    }

    async fn refresh_catalog(
        &self,
        catalog: &CatalogSnapshot,
        profile: &str,
    ) -> ProviderResult<()> {
        self.0.refresh_catalog(catalog, profile).await
    }

    async fn discover(
        &self,
        context: &DiscoveryContext<'_>,
    ) -> ProviderResult<ProviderDiscoverySnapshot> {
        self.0.discover(context).await
    }
}
