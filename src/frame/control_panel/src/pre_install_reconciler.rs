use crate::pikg::PikgReader;
use crate::ControlPanelServer;
use buckyos_api::{
    get_buckyos_api_runtime, validate_preinstall_pikg_path, AppId, AppInstanceId,
    InstallPlanExecutionKey, PreInstallAppConfig, SystemConfigClient, SystemInstallSettings,
};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_root_dir};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const INSTALL_SETTINGS_KEY: &str = "system/install_settings";
const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);
const DEPENDENCY_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const SCHEDULER_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

fn next_sweep_interval(stable_interval: Duration, needs_follow_up: bool) -> Duration {
    if needs_follow_up {
        DEPENDENCY_RETRY_INTERVAL
    } else {
        stable_interval
    }
}

#[derive(Debug)]
struct PreInstallError {
    code: &'static str,
    retryable: bool,
    message: String,
}

impl PreInstallError {
    fn new(code: &'static str, retryable: bool, message: impl Into<String>) -> Self {
        Self {
            code,
            retryable,
            message: message.into(),
        }
    }
}

pub(crate) struct PreInstallReconciler {
    server: Arc<ControlPanelServer>,
    sweep_interval: Duration,
}

impl PreInstallReconciler {
    pub(crate) fn new(server: Arc<ControlPanelServer>) -> Arc<Self> {
        Arc::new(Self {
            server,
            sweep_interval: SWEEP_INTERVAL,
        })
    }

    pub(crate) fn start(self: &Arc<Self>) {
        let reconciler = self.clone();
        tokio::spawn(async move {
            loop {
                let delay = match reconciler.reconcile_once().await {
                    Ok(needs_follow_up) => {
                        next_sweep_interval(reconciler.sweep_interval, needs_follow_up)
                    }
                    Err(error) => {
                        log::warn!("pre-install reconcile sweep deferred: {error}");
                        DEPENDENCY_RETRY_INTERVAL
                    }
                };
                tokio::time::sleep(delay).await;
            }
        });
    }

    async fn reconcile_once(&self) -> Result<bool, String> {
        let runtime = get_buckyos_api_runtime().map_err(|error| error.to_string())?;
        let system_config = runtime
            .get_system_config_client()
            .await
            .map_err(|error| format!("system-config unavailable: {error}"))?;
        let settings_value = system_config
            .get(INSTALL_SETTINGS_KEY)
            .await
            .map_err(|error| format!("read {INSTALL_SETTINGS_KEY} failed: {error}"))?;
        let settings: SystemInstallSettings = serde_json::from_str(&settings_value.value)
            .map_err(|error| format!("invalid {INSTALL_SETTINGS_KEY}: {error}"))?;
        let owner_user_id = system_config
            .get_zone_owner_user_id()
            .await
            .map_err(|error| format!("zone owner unavailable: {error}"))?;

        runtime
            .get_task_mgr_client()
            .await
            .map_err(|error| format!("TaskManager unavailable: {error}"))?;
        runtime
            .get_named_store()
            .await
            .map_err(|error| format!("NamedStore unavailable: {error}"))?;
        let scheduler = runtime
            .get_scheduler_client()
            .await
            .map_err(|error| format!("Scheduler client unavailable: {error}"))?;
        let probe_key = InstallPlanExecutionKey {
            app_instance_id: AppInstanceId::new(
                AppId::parse("preinstall-probe.buckyos.bns.did")?,
                owner_user_id.clone(),
            )?,
            task_id: "t-00000000000000000000000000000000".to_string(),
            plan_fingerprint: format!("planfp:{}", "0".repeat(64)),
        };
        match tokio::time::timeout(
            SCHEDULER_PROBE_TIMEOUT,
            scheduler.get_install_plan_status(probe_key),
        )
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(error)) if !scheduler_transport_unavailable(error.to_string().as_str()) => {}
            Ok(Err(error)) => return Err(format!("Scheduler endpoint unavailable: {error}")),
            Err(_) => return Err("Scheduler endpoint readiness probe timed out".to_string()),
        }

        let mut apps = settings.pre_install_apps.into_iter().collect::<Vec<_>>();
        apps.sort_by(|(left, _), (right, _)| left.cmp(right));
        let mut retry_needed = false;
        let mut follow_up_needed = false;
        for (raw_app_id, config) in apps {
            let app_id = match AppId::parse(&raw_app_id) {
                Ok(app_id) if app_id.as_str() == raw_app_id => app_id,
                Ok(_) | Err(_) => {
                    log::error!(
                        "pre-install app_id={} path={} error_code=INVALID_APP_ID",
                        raw_app_id,
                        config.pikg_path
                    );
                    continue;
                }
            };
            match self
                .reconcile_app(&system_config, owner_user_id.as_str(), &app_id, &config)
                .await
            {
                Ok(needs_follow_up) => follow_up_needed |= needs_follow_up,
                Err(error) => {
                    retry_needed |= error.retryable;
                    let state = json!({
                        "schema_version": 1,
                        "app_id": app_id,
                        "pikg_path": config.pikg_path,
                        "updated_at": buckyos_get_unix_timestamp(),
                        "error": {
                            "code": error.code,
                            "retryable": error.retryable,
                            "message": error.message,
                        }
                    });
                    if let Err(write_error) =
                        Self::write_state(&system_config, &app_id, state).await
                    {
                        log::warn!(
                            "pre-install app_id={} path={} error_code={} state_write_error={}",
                            app_id,
                            config.pikg_path,
                            error.code,
                            write_error
                        );
                    } else {
                        log::error!(
                            "pre-install app_id={} path={} error_code={} retryable={} error={}",
                            app_id,
                            config.pikg_path,
                            error.code,
                            error.retryable,
                            error.message
                        );
                    }
                }
            }
        }
        if retry_needed {
            Err("one or more pre-install apps need a dependency retry".to_string())
        } else {
            Ok(follow_up_needed)
        }
    }

    async fn reconcile_app(
        &self,
        system_config: &Arc<SystemConfigClient>,
        owner_user_id: &str,
        app_id: &AppId,
        config: &PreInstallAppConfig,
    ) -> Result<bool, PreInstallError> {
        config
            .validate()
            .map_err(|error| PreInstallError::new("INVALID_CONFIG", false, error))?;
        let root = get_buckyos_root_dir();
        let source_path = canonical_preinstall_path(&root, config.pikg_path.as_str()).await?;
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            PreInstallError::new("RUNTIME_UNAVAILABLE", true, error.to_string())
        })?;
        let metadata = self
            .server
            .staging_store
            .stage_preinstall_file(
                &source_path,
                owner_user_id,
                "system:control-panel",
                &runtime.zone_id,
            )
            .await
            .map_err(|error| PreInstallError::new("PIKG_STAGE_FAILED", false, error.to_string()))?;
        let (_, staged_path) = self
            .server
            .staging_store
            .resolve(
                metadata.handle.as_str(),
                owner_user_id,
                "system:control-panel",
                &runtime.zone_id,
                buckyos_api::PikgStagingPurpose::Install,
                None,
            )
            .await
            .map_err(|error| {
                PreInstallError::new("PIKG_STAGE_RESOLVE_FAILED", true, error.to_string())
            })?;
        let reader = PikgReader::open(&staged_path, Some(metadata.pikg_digest.as_str()))
            .await
            .map_err(|error| PreInstallError::new("INVALID_PIKG", false, error.to_string()))?;
        let inspection = reader.inspection();
        let canonical_app_id = AppId::from_app_did(inspection.app_doc.app_did())
            .map_err(|error| PreInstallError::new("INVALID_APP_ID", false, error))?;
        if &canonical_app_id != app_id {
            return Err(PreInstallError::new(
                "APP_ID_MISMATCH",
                false,
                format!("map key {app_id} != PIKG AppDID-derived AppId {canonical_app_id}"),
            ));
        }

        let outcome = self
            .server
            .submit_preinstall(
                owner_user_id,
                app_id,
                metadata.pikg_digest.as_str(),
                &inspection.app_doc_object_id,
                &inspection.app_doc,
                metadata.handle.as_str(),
                &config.install_plan,
            )
            .await
            .map_err(|error| {
                PreInstallError::new("INSTALL_SUBMIT_FAILED", true, error.to_string())
            })?;
        let needs_follow_up = outcome.task_id.is_some();
        let state = json!({
            "schema_version": 1,
            "app_id": app_id,
            "app_instance_id": outcome.app_instance_id,
            "pikg_path": config.pikg_path,
            "pikg_digest": metadata.pikg_digest,
            "task_id": outcome.task_id,
            "plan_fingerprint": outcome.plan_fingerprint,
            "action": outcome.action,
            "updated_at": buckyos_get_unix_timestamp(),
            "error": null,
        });
        Self::write_state(system_config, app_id, state)
            .await
            .map_err(|error| PreInstallError::new("STATE_WRITE_FAILED", true, error))?;
        log::info!(
            "pre-install app_id={} path={} digest={} task_id={} plan_fingerprint={} action={}",
            app_id,
            config.pikg_path,
            metadata.pikg_digest,
            outcome.task_id.as_deref().unwrap_or("none"),
            outcome.plan_fingerprint,
            outcome.action
        );
        Ok(needs_follow_up)
    }

    async fn write_state(
        client: &Arc<SystemConfigClient>,
        app_id: &AppId,
        value: Value,
    ) -> Result<(), String> {
        let key = format!("system/control_panel/pre_install_apps/{app_id}");
        let raw = serde_json::to_string(&value).map_err(|error| error.to_string())?;
        if client.set(&key, &raw).await.is_err() {
            client
                .create(&key, &raw)
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

fn scheduler_transport_unavailable(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "502 bad gateway",
        "503 service unavailable",
        "504 gateway timeout",
        "connection refused",
        "failed to connect",
        "connection reset",
        "transport error",
        "timed out",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

async fn canonical_preinstall_path(
    buckyos_root: &Path,
    raw: &str,
) -> Result<PathBuf, PreInstallError> {
    validate_preinstall_pikg_path(raw)
        .map_err(|error| PreInstallError::new("UNSAFE_PIKG_PATH", false, error))?;
    let cache_root = tokio::fs::canonicalize(buckyos_root.join("data").join("cache"))
        .await
        .map_err(|error| {
            PreInstallError::new(
                "CACHE_ROOT_UNAVAILABLE",
                true,
                format!("canonicalize rootfs cache failed: {error}"),
            )
        })?;
    let candidate = buckyos_root.join(raw);
    let canonical = tokio::fs::canonicalize(&candidate).await.map_err(|error| {
        PreInstallError::new(
            "PIKG_UNAVAILABLE",
            true,
            format!("canonicalize {} failed: {error}", candidate.display()),
        )
    })?;
    if !canonical.starts_with(&cache_root) {
        return Err(PreInstallError::new(
            "PIKG_PATH_ESCAPE",
            false,
            "pre-install PIKG escapes $BUCKYOS_ROOT/data/cache",
        ));
    }
    let metadata = tokio::fs::metadata(&canonical).await.map_err(|error| {
        PreInstallError::new(
            "PIKG_UNAVAILABLE",
            true,
            format!("stat {} failed: {error}", canonical.display()),
        )
    })?;
    if !metadata.is_file() {
        return Err(PreInstallError::new(
            "PIKG_NOT_REGULAR_FILE",
            false,
            "pre-install PIKG is not a regular file",
        ));
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_probe_distinguishes_transport_failure_from_reachable_rpc_error() {
        assert!(scheduler_transport_unavailable(
            "rpc call error: 502 Bad Gateway"
        ));
        assert!(scheduler_transport_unavailable("connection refused"));
        assert!(!scheduler_transport_unavailable(
            "system config key not found"
        ));
    }

    #[test]
    fn submitted_or_retried_install_gets_fast_follow_up() {
        let stable_interval = Duration::from_secs(600);
        assert_eq!(
            next_sweep_interval(stable_interval, true),
            DEPENDENCY_RETRY_INTERVAL
        );
        assert_eq!(next_sweep_interval(stable_interval, false), stable_interval);
    }

    #[tokio::test]
    async fn canonical_path_accepts_cache_file_and_rejects_escape() {
        let root = std::env::temp_dir().join(format!(
            "buckyos-preinstall-path-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let cache = root.join("data").join("cache");
        tokio::fs::create_dir_all(&cache).await.unwrap();
        tokio::fs::write(cache.join("demo.pikg"), b"demo")
            .await
            .unwrap();
        assert_eq!(
            canonical_preinstall_path(&root, "data/cache/demo.pikg")
                .await
                .unwrap(),
            cache.join("demo.pikg").canonicalize().unwrap()
        );
        assert!(canonical_preinstall_path(&root, "data/cache/../demo.pikg")
            .await
            .is_err());
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn canonical_path_rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!(
            "buckyos-preinstall-symlink-test-{}",
            uuid::Uuid::new_v4().simple()
        ));
        let cache = root.join("data").join("cache");
        tokio::fs::create_dir_all(&cache).await.unwrap();
        let outside = root.join("outside.pikg");
        tokio::fs::write(&outside, b"outside").await.unwrap();
        symlink(&outside, cache.join("escape.pikg")).unwrap();
        let error = canonical_preinstall_path(&root, "data/cache/escape.pikg")
            .await
            .unwrap_err();
        assert_eq!(error.code, "PIKG_PATH_ESCAPE");
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
