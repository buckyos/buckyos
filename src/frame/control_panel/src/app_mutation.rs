use buckyos_api::{
    task_mgr_error_code, SystemConfigClient, SystemConfigError, TaskManagerClient, TaskPhase,
    TASK_ERR_NOT_FOUND,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use kRPC::RPCErrors;
use serde_json::Value;
use std::collections::HashMap;

pub const MUTATION_PREFIX: &str = "services/control_panel/app_mutations";
const RESERVATION_TTL_SECS: u64 = 300;

pub struct AppMutationLease {
    pub key: String,
    pub owner: Value,
}

pub fn installation_target(
    data: &buckyos_api::AppInstallTaskData,
) -> Option<&buckyos_api::AppInstanceId> {
    data.state
        .plan
        .as_ref()
        .or(data.request.submitted_plan.as_ref())
        .map(|plan| &plan.app_instance_id)
}

pub fn same_owner(current: &Value, expected: &Value) -> bool {
    [
        "creator_user_id",
        "creator_app_id",
        "idempotency_key",
        "created_at",
    ]
    .iter()
    .all(|field| current.get(*field) == expected.get(*field))
}

pub(crate) fn task_is_finished(result: Result<TaskPhase, RPCErrors>) -> Result<bool, RPCErrors> {
    match result {
        Ok(phase) => Ok(phase.is_terminal()),
        Err(error) if task_mgr_error_code(&error) == Some(TASK_ERR_NOT_FOUND) => Ok(true),
        Err(error) => Err(error),
    }
}

pub async fn reclaimable(value: &Value, client: &TaskManagerClient) -> Result<bool, RPCErrors> {
    if let Some(task_id) = value.get("task_id").and_then(Value::as_str) {
        task_is_finished(client.get_task(task_id).await.map(|task| task.phase))
    } else {
        Ok(value
            .get("created_at")
            .and_then(Value::as_u64)
            .is_some_and(|created| {
                buckyos_get_unix_timestamp().saturating_sub(created) >= RESERVATION_TTL_SECS
            }))
    }
}

pub async fn release_owned(
    client: &SystemConfigClient,
    key: &str,
    task_id: &str,
) -> Result<(), RPCErrors> {
    for _ in 0..3 {
        client.invalidate_cache(key).await;
        let current = match client.get(key).await {
            Ok(current) => current,
            Err(SystemConfigError::KeyNotFound(_)) => return Ok(()),
            Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
        };
        let owner: Value = serde_json::from_str(&current.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid app mutation owner: {error}"))
        })?;
        if owner.get("task_id").and_then(Value::as_str) != Some(task_id) {
            return Ok(());
        }
        if client
            .exec_tx(
                HashMap::from([(key.to_string(), KVAction::Remove)]),
                Some((key.to_string(), current.version)),
            )
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(RPCErrors::ReasonError(
        "app mutation release failed after CAS retries".to_string(),
    ))
}

pub async fn reconcile(
    client: &SystemConfigClient,
    tasks: &TaskManagerClient,
) -> Result<(), RPCErrors> {
    let owners = client
        .list(MUTATION_PREFIX)
        .await
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    for app_instance_id in owners {
        let key = format!("{MUTATION_PREFIX}/{app_instance_id}");
        client.invalidate_cache(&key).await;
        let current = match client.get(&key).await {
            Ok(current) => current,
            Err(SystemConfigError::KeyNotFound(_)) => continue,
            Err(error) => return Err(RPCErrors::ReasonError(error.to_string())),
        };
        let value: Value = serde_json::from_str(&current.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid app mutation owner: {error}"))
        })?;
        if reclaimable(&value, tasks).await? {
            let _ = client
                .exec_tx(
                    HashMap::from([(key.clone(), KVAction::Remove)]),
                    Some((key, current.version)),
                )
                .await;
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use buckyos_api::{task_mgr_error, TASK_ERR_PERMISSION_DENIED};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    struct ConfigState {
        owner: Value,
        version: u64,
        removals: usize,
        replace_on_remove: bool,
    }

    async fn config_server(
        state: Arc<Mutex<ConfigState>>,
    ) -> (SystemConfigClient, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let request = loop {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]);
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|value| value.trim().parse().unwrap())
                            })
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break serde_json::from_slice::<kRPC::RPCRequest>(
                                &bytes[end + 4..end + 4 + length],
                            )
                            .unwrap();
                        }
                    }
                };
                let result = {
                    let mut state = state.lock().unwrap();
                    match request.method.as_str() {
                        "sys_config_get" => kRPC::RPCResult::Success(
                            json!({"value":state.owner.to_string(), "version":state.version}),
                        ),
                        "sys_config_exec_tx" => {
                            state.removals += 1;
                            let expected = request.params["main_key"]["revision"].as_u64().unwrap();
                            if state.replace_on_remove {
                                state.replace_on_remove = false;
                                state.owner["task_id"] = json!("new-task");
                                state.version += 1;
                            }
                            if expected != state.version {
                                kRPC::RPCResult::Failed("version conflict".to_string())
                            } else {
                                state.owner = Value::Null;
                                state.version += 1;
                                kRPC::RPCResult::Success(json!(0))
                            }
                        }
                        method => panic!("unexpected method {method}"),
                    }
                };
                let body =
                    serde_json::to_vec(&kRPC::RPCResponse::new(result, request.seq)).unwrap();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                stream.write_all(&body).await.unwrap();
            }
        });
        (
            SystemConfigClient::new(Some(&format!("http://{address}")), None),
            server,
        )
    }

    #[tokio::test]
    async fn late_release_cannot_delete_a_new_owner_after_cas_conflict() {
        let state = Arc::new(Mutex::new(ConfigState {
            owner: json!({"task_id":"old-task"}),
            version: 1,
            removals: 0,
            replace_on_remove: true,
        }));
        let (client, server) = config_server(state.clone()).await;
        release_owned(&client, "mutation", "old-task")
            .await
            .unwrap();
        assert_eq!(state.lock().unwrap().owner["task_id"], "new-task");
        assert_eq!(state.lock().unwrap().removals, 1);
        server.abort();
    }

    #[tokio::test]
    async fn release_reads_current_owner_instead_of_cached_owner() {
        let state = Arc::new(Mutex::new(ConfigState {
            owner: json!({"task_id":"old-task"}),
            version: 1,
            removals: 0,
            replace_on_remove: false,
        }));
        let (client, server) = config_server(state.clone()).await;
        client.get("mutation").await.unwrap();
        {
            let mut state = state.lock().unwrap();
            state.owner["task_id"] = json!("new-task");
            state.version += 1;
        }
        release_owned(&client, "mutation", "old-task")
            .await
            .unwrap();
        assert_eq!(state.lock().unwrap().owner["task_id"], "new-task");
        assert_eq!(state.lock().unwrap().removals, 0);
        server.abort();
    }

    #[derive(Default)]
    pub struct TestTasks {
        pub states: HashMap<String, TaskPhase>,
        pub fail_reads: bool,
    }

    #[async_trait::async_trait]
    impl buckyos_api::TaskManagerHandler for TestTasks {
        async fn handle_get_task(
            &self,
            req: buckyos_api::GetTaskReq,
            _: kRPC::RPCContext,
        ) -> kRPC::Result<buckyos_api::Task> {
            if self.fail_reads {
                return Err(RPCErrors::ReasonError(
                    "TaskManager connection failed".to_string(),
                ));
            }
            let Some(phase) = self.states.get(&req.task_id) else {
                return Err(task_mgr_error(TASK_ERR_NOT_FOUND, "missing task"));
            };
            Ok(serde_json::from_value(json!({
                "task_id": req.task_id, "name": "install", "root_id": req.task_id,
                "child_control_policy": buckyos_api::ChildControlPolicy::default(),
                "schema_id": "app.install/v1", "schema_version": 1,
                "input": {}, "input_digest": "digest", "creator": buckyos_api::ActorRef::new("alice", "desktop"),
                "storage_domain": buckyos_api::StorageDomain::System, "idempotency_key": "request",
                "executor": buckyos_api::TaskExecutor::Unbound, "runner_epoch": 1, "phase": phase,
                "control_profile": buckyos_api::TaskControlProfile::baseline(0),
                "policy_preset": "default", "permission_boundary": false, "revision": 1,
                "created_at": 1, "updated_at": 1
            })).unwrap())
        }
    }

    #[tokio::test]
    async fn expiration_does_not_reclaim_a_live_waiting_task() {
        let client = TaskManagerClient::new_in_process(Box::new(TestTasks {
            states: HashMap::from([
                ("waiting".to_string(), TaskPhase::Waiting),
                ("done".to_string(), TaskPhase::Terminal),
            ]),
            ..Default::default()
        }));
        assert!(
            !reclaimable(&json!({"task_id":"waiting", "expires_at":0}), &client)
                .await
                .unwrap()
        );
        assert!(
            reclaimable(&json!({"task_id":"done", "expires_at":u64::MAX}), &client)
                .await
                .unwrap()
        );
        assert!(reclaimable(&json!({"task_id":"missing"}), &client)
            .await
            .unwrap());
        assert!(!reclaimable(
            &json!({"task_id":null, "created_at":buckyos_get_unix_timestamp()}),
            &client
        )
        .await
        .unwrap());
        assert!(reclaimable(&json!({"task_id":null, "created_at":buckyos_get_unix_timestamp()-RESERVATION_TTL_SECS}), &client).await.unwrap());
        let unavailable = TaskManagerClient::new_in_process(Box::new(TestTasks {
            fail_reads: true,
            ..Default::default()
        }));
        assert!(
            reclaimable(&json!({"task_id":"waiting", "expires_at":0}), &unavailable)
                .await
                .is_err()
        );
    }

    #[test]
    fn only_terminal_and_explicit_not_found_are_reclaimable() {
        assert!(task_is_finished(Ok(TaskPhase::Terminal)).unwrap());
        for phase in [
            TaskPhase::Accepted,
            TaskPhase::Running,
            TaskPhase::Waiting,
            TaskPhase::Paused,
        ] {
            assert!(!task_is_finished(Ok(phase)).unwrap());
        }
        assert!(task_is_finished(Err(task_mgr_error(TASK_ERR_NOT_FOUND, "gone"))).unwrap());
        assert!(
            task_is_finished(Err(task_mgr_error(TASK_ERR_PERMISSION_DENIED, "denied"))).is_err()
        );
        assert!(task_is_finished(Err(RPCErrors::ReasonError("connection failed".into()))).is_err());
    }

    #[test]
    fn a_replaced_reservation_is_not_the_original_owner() {
        let original = json!({"creator_user_id":"alice", "creator_app_id":"desktop", "idempotency_key":"a", "created_at":1});
        let mut replacement = original.clone();
        replacement["idempotency_key"] = json!("b");
        assert!(!same_owner(&replacement, &original));
        replacement = original.clone();
        replacement["created_at"] = json!(2);
        assert!(!same_owner(&replacement, &original));
        replacement = original.clone();
        replacement["task_id"] = json!("task-a");
        assert!(same_owner(&replacement, &original));
    }
}
