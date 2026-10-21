//! Form A of the Node Executor (doc/arch/node_exector.md): node_daemon hands
//! the scheduler snapshot to the embedded executor and keeps the gateway
//! device ACL in step with the same snapshot.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, load_current_rbac_config, node_config_key,
    validate_verify_hub_token_claims, AuthTarget, ExecutorConfig, TokenUse,
    NODE_EXECUTOR_SERVICE_PORT,
};
use cyfs_gateway_api::CONTROL_SERVER;
use log::*;
use node_executor::{
    default_shell, ExecutorHost, NodeExecutor, RbacChecker, TokenVerifier, VerifiedSession,
    WorkerLauncher,
};
use serde_json::{json, Value};

use crate::gateway_tunnel_probe::build_local_gateway_token;

pub const EXECUTOR_ACL_FILE: &str = "node_executor_acl.json";
const TICK: Duration = Duration::from_secs(1);
const CONFIG_REFRESH: Duration = Duration::from_secs(5);

struct RuntimeVerifier;

#[async_trait]
impl TokenVerifier for RuntimeVerifier {
    async fn verify(&self, token: &str) -> Result<VerifiedSession, String> {
        let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        let parsed = runtime
            .verify_trusted_session_token(token)
            .await
            .map_err(|e| e.to_string())?;
        let claims =
            validate_verify_hub_token_claims(&parsed, TokenUse::Session).map_err(|e| e.to_string())?;
        let subject = parsed.sub.clone().ok_or("token has no subject")?;
        let expires_at_ms = parsed.exp.ok_or("token has no expiry")?.saturating_mul(1000);
        let app_instance_id = match &claims.target {
            AuthTarget::App { app_instance_id } => Some(app_instance_id.clone()),
            _ => None,
        };
        Ok(VerifiedSession {
            subject,
            principal_kind: claims.principal_kind,
            token_use: claims.token_use,
            authorization_key: claims.target.authorization_key(),
            app_instance_id,
            expires_at_ms,
        })
    }
}

struct RuntimeRbac;

#[async_trait]
impl RbacChecker for RuntimeRbac {
    async fn allowed(&self, user_id: &str, app_key: &str, resource: &str, action: &str) -> bool {
        let Ok(runtime) = get_buckyos_api_runtime() else {
            return false;
        };
        let Ok(client) = runtime.get_system_config_client().await else {
            return false;
        };
        let Ok(config) = load_current_rbac_config(&client).await else {
            return false;
        };
        if rbac::apply_enforcer_policy(&config.model, &config.policy)
            .await
            .is_err()
        {
            return false;
        }
        rbac::enforce(user_id, app_key, resource, action, None).await
    }
}

/// Written before cyfs-gateway starts: the `EXECUTOR_ACL` collection must
/// exist, and without a snapshot it admits no device.
pub fn write_boot_executor_acl() -> std::io::Result<()> {
    write_acl(&None)
}

fn write_acl(acl: &Option<(String, Vec<String>)>) -> std::io::Result<()> {
    let (revision, devices) = acl.clone().unwrap_or_default();
    let allowed: serde_json::Map<String, Value> = devices
        .into_iter()
        .map(|d| (d, Value::String("allow".into())))
        .collect();
    let body = json!({ "revision": revision, "allowed": allowed });
    let path = buckyos_kit::get_buckyos_system_etc_dir().join(EXECUTOR_ACL_FILE);
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&body).unwrap_or_default())?;
    std::fs::rename(&tmp, &path)
}

/// Ask the gateway to drop executor forwards from devices outside `keep`.
async fn close_gateway_forwards(keep: &BTreeSet<String>) {
    let Some(token) = build_local_gateway_token() else {
        warn!("node executor: no gateway control token, existing forwards stay open");
        return;
    };
    let client = kRPC::kRPC::new(CONTROL_SERVER, Some(token));
    let params = json!({
        "stack": "node_rtcp",
        "dest_port": NODE_EXECUTOR_SERVICE_PORT,
        "keep_source_devices": keep.iter().collect::<Vec<_>>(),
    });
    match client.call("close_connections", params).await {
        Ok(result) => info!("node executor: gateway closed revoked forwards: {result}"),
        Err(err) => warn!("node executor: gateway close_connections failed: {err:?}"),
    }
}

async fn load_executor_config(node_host_name: &str) -> Result<Option<ExecutorConfig>, String> {
    let runtime = get_buckyos_api_runtime().map_err(|e| e.to_string())?;
    let client = runtime
        .get_system_config_client()
        .await
        .map_err(|e| e.to_string())?;
    let value = client
        .get(&node_config_key(node_host_name))
        .await
        .map_err(|e| e.to_string())?;
    let config: Value = serde_json::from_str(&value.value).map_err(|e| e.to_string())?;
    match config.get("executor") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("invalid executor config: {e}")),
    }
}

fn open_executor(node_host_name: &str, node_did: &str) -> Result<NodeExecutor, String> {
    let program = std::env::current_exe().map_err(|e| e.to_string())?;
    NodeExecutor::open(ExecutorHost {
        node_id: node_host_name.to_string(),
        node_did: node_did.to_string(),
        store_dir: buckyos_kit::get_buckyos_service_local_data_dir("node_daemon").join("executor"),
        worker: WorkerLauncher {
            program,
            args: vec![node_executor::worker::WORKER_ARG.to_string()],
        },
        shell: default_shell(),
        verifier: Arc::new(RuntimeVerifier),
        rbac: Arc::new(RuntimeRbac),
    })
    .map_err(|e| e.to_string())
}

/// Opens the executor on the first snapshot, delivers snapshots every 5 s,
/// and keeps the gateway ACL equal to what the executor currently accepts.
pub fn spawn_node_executor(node_host_name: String, node_did: String) {
    tokio::spawn(async move {
        let mut executor: Option<NodeExecutor> = None;
        let mut last_fetch: Option<Instant> = None;
        let mut written: Option<(String, Vec<String>)> = None;
        let mut acl_written_once = false;
        let mut tick = tokio::time::interval(TICK);
        loop {
            tick.tick().await;
            if last_fetch.map(|t| t.elapsed() >= CONFIG_REFRESH).unwrap_or(true) {
                last_fetch = Some(Instant::now());
                match load_executor_config(&node_host_name).await {
                    Ok(config) => {
                        if executor.is_none() && config.as_ref().map(|c| c.enabled).unwrap_or(false) {
                            match open_executor(&node_host_name, &node_did) {
                                Ok(exec) => {
                                    let listener = exec.clone();
                                    let addr = SocketAddr::from(([127, 0, 0, 1], NODE_EXECUTOR_SERVICE_PORT));
                                    tokio::spawn(async move {
                                        if let Err(err) = listener.listen(addr).await {
                                            error!("node executor listener on {addr} stopped: {err}");
                                        }
                                    });
                                    info!(
                                        "node executor opened, store_id={} caps={:?}",
                                        exec.store_id(),
                                        exec.host_capabilities()
                                    );
                                    executor = Some(exec);
                                }
                                Err(err) => error!("node executor cannot open its ledger: {err}"),
                            }
                        }
                        if let Some(exec) = executor.as_ref() {
                            exec.apply_config(config).await;
                        }
                    }
                    Err(err) => debug!("node executor: reload node config failed: {err}"),
                }
            }
            let acl = executor.as_ref().and_then(|e| e.gateway_acl());
            if acl != written || !acl_written_once {
                if let Err(err) = write_acl(&acl) {
                    warn!("node executor: write gateway ACL failed: {err}");
                    continue;
                }
                acl_written_once = true;
                let before: BTreeSet<String> =
                    written.as_ref().map(|(_, d)| d.iter().cloned().collect()).unwrap_or_default();
                let after: BTreeSet<String> =
                    acl.as_ref().map(|(_, d)| d.iter().cloned().collect()).unwrap_or_default();
                if !before.is_subset(&after) {
                    close_gateway_forwards(&after).await;
                }
                written = acl;
            }
        }
    });
}
