//! The current Zone as object reads see it: the gateway origin `obj://` endpoints are built on, the Zone host, the
//! data root behind `cyfs:///` paths, and the caller's session token, sent only to the Zone's own origins
//! (agent-object-url TODO §6 第 3 段).

use std::path::PathBuf;

use async_trait::async_trait;
use url::Url;

#[async_trait]
pub trait ZoneAccess: Send + Sync {
    /// The gateway origin of this process (`http://<node gateway>:3180`), `None` outside a Zone.
    fn gateway_origin(&self) -> Option<String>;
    /// The Zone hostname (`cyfs://<zone>/` of this Zone, the Zone's https origins).
    fn zone_host(&self) -> Option<String>;
    /// `$BUCKYOS_ROOT/data`: DFS paths are files below it.
    fn data_root(&self) -> PathBuf;
    async fn session_token(&self) -> Option<String>;
}

/// The process's BuckyOS runtime: an Agent's container, a zone service, the dev CLI run inside one.
pub struct RuntimeZoneAccess;

#[async_trait]
impl ZoneAccess for RuntimeZoneAccess {
    fn gateway_origin(&self) -> Option<String> {
        let runtime = buckyos_api::get_buckyos_api_runtime().ok()?;
        Some(format!(
            "http://{}:{}",
            runtime.get_local_gateway_host(),
            runtime.node_gateway_port
        ))
    }

    fn zone_host(&self) -> Option<String> {
        let runtime = buckyos_api::get_buckyos_api_runtime().ok()?;
        let document = runtime.get_zone_config()?.zone_document().ok()?;
        Some(buckyos_api::zone_document_hostname(&document).to_ascii_lowercase())
    }

    fn data_root(&self) -> PathBuf {
        buckyos_kit::get_buckyos_root_dir().join("data")
    }

    async fn session_token(&self) -> Option<String> {
        let runtime = buckyos_api::get_buckyos_api_runtime().ok()?;
        let _ = runtime.renew_token_from_verify_hub().await;
        Some(runtime.get_session_token().await).filter(|token| !token.is_empty())
    }
}

/// An origin of this Zone: the gateway origin, the Zone host or one of its subdomains.
pub fn is_zone_url(zone: &dyn ZoneAccess, url: &str) -> bool {
    let Ok(parsed) = Url::parse(url) else {
        return false;
    };
    if let Some(origin) = zone.gateway_origin() {
        if Url::parse(&origin).is_ok_and(|o| o.origin() == parsed.origin()) {
            return true;
        }
    }
    let (Some(host), Some(zone_host)) = (parsed.host_str(), zone.zone_host()) else {
        return false;
    };
    let host = host.to_ascii_lowercase();
    parsed.scheme() == "https" && (host == zone_host || host.ends_with(&format!(".{zone_host}")))
}

/// An HTTP client that presents the caller's session to `url` when it is one of the Zone's origins.
pub async fn client_for(zone: &dyn ZoneAccess, url: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if is_zone_url(zone, url) {
        if let Some(token) = zone.session_token().await {
            if let Ok(value) = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(reqwest::header::AUTHORIZATION, value);
            }
        }
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub struct TestZone {
        pub origin: Option<String>,
        pub data_root: PathBuf,
    }

    #[async_trait]
    impl ZoneAccess for TestZone {
        fn gateway_origin(&self) -> Option<String> {
            self.origin.clone()
        }
        fn zone_host(&self) -> Option<String> {
            Some("test.buckyos.io".to_string())
        }
        fn data_root(&self) -> PathBuf {
            self.data_root.clone()
        }
        async fn session_token(&self) -> Option<String> {
            Some("token-1".to_string())
        }
    }

    #[test]
    fn only_zone_origins_get_the_session() {
        let zone = TestZone {
            origin: Some("http://host.docker.internal:3180".to_string()),
            data_root: PathBuf::from("/opt/buckyos/data"),
        };
        assert!(is_zone_url(&zone, "http://host.docker.internal:3180/kapi/msg-center/obj"));
        assert!(is_zone_url(&zone, "https://test.buckyos.io/home/alice/feed"));
        assert!(is_zone_url(&zone, "https://aiws-1.test.buckyos.io/x"));
        assert!(!is_zone_url(&zone, "http://test.buckyos.io/x"));
        assert!(!is_zone_url(&zone, "https://eviltest.buckyos.io/x"));
        assert!(!is_zone_url(&zone, "https://example.com/x"));
        assert!(!is_zone_url(&zone, "http://host.docker.internal:8080/x"));
    }
}
