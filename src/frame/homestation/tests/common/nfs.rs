//! A zone's nfs-server for standalone HomeStations: the named store and the buckets HomeStation
//! declares (what `nfs_server --named-store <dir> --bucket-specs <json>` serves standalone),
//! in process and without authentication.

use nfs_server::bucket::spec::AppBuckets;
use nfs_server::config::{NamedStoreSource, ServerConfig};
use std::path::Path;

/// Starts it on a free loopback port; returns its base URL.
pub async fn start_nfs_server(dir: &Path) -> String {
    let mut config = ServerConfig::new(dir.join("fsdb"), vec![]);
    config.named_store = NamedStoreSource::Dir(dir.join("store"));
    config.bucket_specs = std::collections::HashMap::from([(
        buckyos_api::HOMESTATION_SERVICE_NAME.to_string(),
        AppBuckets {
            buckets: buckyos_api::homestation_bucket_decls(),
            storage_quota_mib: Some(buckyos_api::HOMESTATION_STORAGE_QUOTA_MIB),
            ..Default::default()
        },
    )]);
    let state = nfs_server::AppState::new(config).expect("nfs-server state");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("nfs-server listener");
    let base = format!("http://{}", listener.local_addr().unwrap());
    let router = nfs_server::build_router(state);
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    base
}
