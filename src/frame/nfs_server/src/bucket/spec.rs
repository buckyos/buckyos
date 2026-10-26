//! Buckets exist because an app declared them (D7): `install_config.buckets`
//! lands in `users/<user>/apps/<appid>/spec` (or `services/<appid>/spec`) as
//! `spec_config.buckets`; this module creates them lazily and keeps their
//! declared fields current. Standalone servers read `--bucket-specs`.

use super::CACHE_TTL;
use crate::config::AuthMode;
use crate::state::AppState;
use bucket_store::{BucketDecl, BucketError, BucketId, BucketInfo, BucketResult, BucketScope, ZONE_OWNER};
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

/// What an app declares (spec_config subset, or one entry of `--bucket-specs`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppBuckets {
    #[serde(default)]
    pub buckets: HashMap<String, BucketDecl>,
    #[serde(default)]
    pub storage_quota_mib: Option<u64>,
}

#[derive(Deserialize)]
struct SpecView {
    #[serde(default)]
    spec_config: AppBuckets,
}

async fn load_runtime(appid: &str, owner: &str) -> BucketResult<Option<AppBuckets>> {
    let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| BucketError::Internal(e.to_string()))?;
    let client = runtime.get_system_config_client().await.map_err(|e| BucketError::Internal(e.to_string()))?;
    let mut keys = Vec::new();
    if owner != ZONE_OWNER {
        keys.push(format!("users/{owner}/apps/{appid}/spec"));
    }
    keys.push(format!("services/{appid}/spec"));
    for key in keys {
        match client.get(&key).await {
            Ok(v) => {
                let view: SpecView = serde_json::from_str(&v.value).map_err(|e| BucketError::Internal(format!("{key}: {e}")))?;
                return Ok(Some(view.spec_config));
            }
            Err(buckyos_api::SystemConfigError::KeyNotFound(_)) => continue,
            Err(e) => return Err(BucketError::Internal(format!("{key}: {e}"))),
        }
    }
    Ok(None)
}

async fn load(state: &AppState, appid: &str, owner: &str) -> BucketResult<Option<AppBuckets>> {
    match state.config.auth {
        AuthMode::Runtime => load_runtime(appid, owner).await,
        _ => Ok(state.config.bucket_specs.get(appid).cloned()),
    }
}

/// Creates / updates the buckets `appid` declares for `owner` (`_` = zone).
pub async fn sync_app(state: &AppState, appid: &str, owner: &str, force: bool) -> BucketResult<()> {
    let key = format!("{appid}@{owner}");
    if !force && state.buckets.synced.lock().unwrap().get(&key).is_some_and(|at| at.elapsed() < CACHE_TTL) {
        return Ok(());
    }
    let store = state.buckets.store(state).await?;
    let declared = load(state, appid, owner).await?.unwrap_or_default();
    let scope = if owner == ZONE_OWNER { BucketScope::Zone } else { BucketScope::User };
    let mut names = Vec::new();
    for (name, decl) in &declared.buckets {
        if decl.scope != scope {
            continue;
        }
        let id = BucketId::new(scope, appid, owner, name)?;
        match store.ensure_bucket(&id, decl) {
            Ok(_) => names.push(name.clone()),
            Err(e) => log::warn!("declared bucket {id} rejected: {e}"),
        }
    }
    if declared.buckets.values().any(|d| d.needs_app_pool()) {
        store.set_declared_pool_quota(appid, owner, declared.storage_quota_mib.map(|m| m * 1024 * 1024))?;
    }
    for existing in store.list_buckets(Some(owner), Some(appid), Some(scope))? {
        if !names.contains(&existing.name) && existing.orphaned_at.is_none() {
            log::info!("bucket {} is no longer declared; kept as an orphan", existing.bucket_id);
            store.mark_orphaned(&existing.bucket_id)?;
        }
    }
    state.buckets.synced.lock().unwrap().insert(key, Instant::now());
    Ok(())
}

/// The bucket, creating it first when its app declares it.
pub async fn ensure(state: &AppState, id: &BucketId) -> BucketResult<BucketInfo> {
    let store = state.buckets.store(state).await?;
    match store.bucket(id) {
        Ok(b) => {
            sync_app(state, &id.appid, &id.owner, false).await?;
            Ok(store.bucket(id).unwrap_or(b))
        }
        Err(BucketError::BucketNotFound(_)) => {
            sync_app(state, &id.appid, &id.owner, true).await?;
            store.bucket(id)
        }
        Err(e) => Err(e),
    }
}

/// Declared buckets of every app a user has installed (and services with per-user buckets).
pub async fn sync_user(state: &AppState, user: &str) -> BucketResult<()> {
    let apps: Vec<String> = match state.config.auth {
        AuthMode::Runtime => {
            let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| BucketError::Internal(e.to_string()))?;
            let client = runtime.get_system_config_client().await.map_err(|e| BucketError::Internal(e.to_string()))?;
            let mut apps = client.list(&format!("users/{user}/apps")).await.unwrap_or_default();
            apps.extend(client.list("services").await.unwrap_or_default());
            apps
        }
        _ => state.config.bucket_specs.keys().cloned().collect(),
    };
    for app in apps {
        if let Err(e) = sync_app(state, &app, user, false).await {
            log::debug!("bucket spec of {app} for {user}: {e}");
        }
    }
    Ok(())
}

/// Zone buckets declared by services and standalone specs.
pub async fn sync_zone(state: &AppState) -> BucketResult<()> {
    let apps: Vec<String> = match state.config.auth {
        AuthMode::Runtime => {
            let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| BucketError::Internal(e.to_string()))?;
            let client = runtime.get_system_config_client().await.map_err(|e| BucketError::Internal(e.to_string()))?;
            client.list("services").await.unwrap_or_default()
        }
        _ => state.config.bucket_specs.keys().cloned().collect(),
    };
    for app in apps {
        if let Err(e) = sync_app(state, &app, ZONE_OWNER, false).await {
            log::debug!("zone bucket spec of {app}: {e}");
        }
    }
    Ok(())
}
