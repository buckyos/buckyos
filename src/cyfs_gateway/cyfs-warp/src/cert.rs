use crate::router::Router;
use anyhow::Result;
use cyfs_gateway_lib::{
    AcmeChallengeEntry, AcmeChallengeResponder, ResponseRouteConfig, RouteConfig,
};
use std::collections::HashMap;

pub(crate) struct ChallengeEntry {
    router: Router,
}

impl ChallengeEntry {
    pub fn new(router: Router) -> Self {
        ChallengeEntry { router }
    }
}

impl AcmeChallengeEntry for ChallengeEntry {
    type Responder = ChallengeResponder;
    fn create_challenge_responder(&self) -> Self::Responder {
        ChallengeResponder {
            router: self.router.clone(),
        }
    }
}

pub(crate) struct ChallengeResponder {
    router: Router,
}

#[async_trait::async_trait]
impl AcmeChallengeResponder for ChallengeResponder {
    async fn respond_http(&self, domain: &str, token: &str, key_auth: &str) -> Result<()> {
        let path = format!("/.well-known/acme-challenge/{}", token);
        let config = RouteConfig {
            enable_cors: false,
            response: Some(ResponseRouteConfig {
                status: Some(200),
                headers: Some(HashMap::from_iter(vec![(
                    "Content-Type".to_string(),
                    "text/plain".to_string(),
                )])),
                body: Some(key_auth.to_string()),
            }),
            upstream: None,
            local_dir: None,
            inner_service: None,
            tunnel_selector: None,
            bucky_service: None,
            named_mgr: None,
        };
        self.router
            .insert_route_config(domain, path.as_str(), config);
        Ok(())
    }
    fn revert_http(&self, domain: &str, token: &str) {
        let path = format!("/.well-known/acme-challenge/{}", token);
        self.router.remove_route_config(domain, path.as_str());
    }

    async fn respond_dns(&self, _domain: &str, _digest: &str) -> Result<()> {
        Ok(())
    }
    fn revert_dns(&self, _domain: &str, _digest: &str) {}

    async fn respond_tls_alpn(&self, _domain: &str, _key_auth: &str) -> Result<()> {
        Ok(())
    }
    fn revert_tls_alpn(&self, _domain: &str, _key_auth: &str) {}
}
