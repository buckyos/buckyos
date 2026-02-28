use crate::config::KLogRuntimeConfig;
use klog::network::KLogAdminRequestType;
use klog::{KNode, KRaftRef};
use log::{error, info, warn};
use std::collections::BTreeMap;
use std::time::Duration;
use tokio::task::JoinHandle;

pub async fn initialize_cluster_if_needed(cfg: &KLogRuntimeConfig, raft: &KRaftRef) {
    if cfg.auto_bootstrap {
        let mut members = BTreeMap::new();
        members.insert(
            cfg.node_id,
            KNode {
                id: cfg.node_id,
                addr: cfg.advertise_addr.clone(),
                port: cfg.advertise_port,
            },
        );
        match raft.initialize(members).await {
            Ok(()) => {
                info!(
                    "Raft cluster initialized: node_id={}, cluster_name={}",
                    cfg.node_id, cfg.cluster_name
                );
            }
            Err(e) => {
                warn!(
                    "Raft initialize skipped/failed (possibly already initialized): {}",
                    e
                );
            }
        }
    } else {
        info!("Skip raft initialize because KLOG_AUTO_BOOTSTRAP=false");
    }
}

pub fn spawn_auto_join_task(cfg: &KLogRuntimeConfig) -> Option<JoinHandle<()>> {
    if !cfg.auto_bootstrap && !cfg.join_targets.is_empty() {
        let join_cfg = cfg.clone();
        Some(tokio::spawn(async move {
            run_auto_join_loop(join_cfg).await;
        }))
    } else {
        if !cfg.auto_bootstrap {
            warn!(
                "KLOG_AUTO_BOOTSTRAP=false but no join targets configured; daemon will run without auto-join"
            );
        }
        None
    }
}

pub async fn stop_auto_join_task(join_task: Option<JoinHandle<()>>) {
    if let Some(handle) = join_task {
        handle.abort();
        let _ = handle.await;
        info!("Auto-join task stopped because network server exited");
    }
}

async fn run_auto_join_loop(cfg: KLogRuntimeConfig) {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build();
    let client = match client {
        Ok(client) => client,
        Err(e) => {
            error!("Failed to create join http client: {}", e);
            return;
        }
    };

    let mut attempts: u32 = 0;
    loop {
        if cfg.join_max_attempts > 0 && attempts >= cfg.join_max_attempts {
            warn!(
                "Auto-join reached max attempts without success: attempts={}, node_id={}, targets={:?}",
                cfg.join_max_attempts, cfg.node_id, cfg.join_targets
            );
            return;
        }
        attempts += 1;

        match try_join_once(&client, &cfg).await {
            Ok(msg) => {
                info!(
                    "Auto-join succeeded: node_id={}, attempt={}, {}",
                    cfg.node_id, attempts, msg
                );
                return;
            }
            Err(e) => {
                warn!(
                    "Auto-join attempt failed: node_id={}, attempt={}, err={}",
                    cfg.node_id, attempts, e
                );
            }
        }

        tokio::time::sleep(Duration::from_millis(cfg.join_retry_interval_ms)).await;
    }
}

async fn try_join_once(
    client: &reqwest::Client,
    cfg: &KLogRuntimeConfig,
) -> Result<String, String> {
    let add_learner_path = KLogAdminRequestType::AddLearner.klog_path();
    let mut errors = Vec::new();

    for target in &cfg.join_targets {
        let mut url = match build_admin_url(target, &add_learner_path) {
            Ok(url) => url,
            Err(err) => {
                errors.push(format!("target='{}': {}", target, err));
                continue;
            }
        };

        {
            let mut q = url.query_pairs_mut();
            q.append_pair("node_id", &cfg.node_id.to_string());
            q.append_pair("addr", &cfg.advertise_addr);
            q.append_pair("port", &cfg.advertise_port.to_string());
            q.append_pair("blocking", if cfg.join_blocking { "true" } else { "false" });
        }

        let response = client.post(url.clone()).send().await;
        let response = match response {
            Ok(resp) => resp,
            Err(e) => {
                errors.push(format!("target='{}': http send failed: {}", target, e));
                continue;
            }
        };

        let status = response.status();
        let body = match response.text().await {
            Ok(text) => text,
            Err(e) => format!("<failed to read body: {}>", e),
        };

        if status.is_success() {
            return Ok(format!(
                "target='{}', status={}, body={}",
                target, status, body
            ));
        }

        errors.push(format!(
            "target='{}': status={}, body={}",
            target, status, body
        ));
    }

    Err(errors.join(" | "))
}

fn build_admin_url(target: &str, path: &str) -> Result<reqwest::Url, String> {
    let trimmed = target.trim();
    if trimmed.is_empty() {
        return Err("empty join target".to_string());
    }

    let with_scheme = if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        trimmed.to_string()
    } else {
        format!("http://{}", trimmed)
    };

    let mut url = reqwest::Url::parse(&with_scheme)
        .map_err(|e| format!("invalid join target url '{}': {}", trimmed, e))?;
    url.set_path(path);
    url.set_query(None);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::build_admin_url;

    #[test]
    fn test_build_admin_url_adds_scheme_and_path() {
        let url =
            build_admin_url("127.0.0.1:21001", "/klog/admin/add-learner").expect("build admin url");
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:21001/klog/admin/add-learner"
        );
    }

    #[test]
    fn test_build_admin_url_rejects_empty() {
        let err = build_admin_url("  ", "/klog/admin/add-learner").expect_err("should fail");
        assert!(err.contains("empty join target"));
    }
}
