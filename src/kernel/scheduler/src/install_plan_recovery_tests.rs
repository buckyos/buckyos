use super::*;
use serde_json::{json, Value};
use std::sync::Mutex;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;

#[path = "../../../test/install_plan_fixture.rs"]
mod fixture;

#[derive(Default)]
struct Database {
    values: HashMap<String, (String, u64)>,
    before_tx: Vec<(String, String)>,
    after_get: Option<(String, usize, Vec<(String, String)>)>,
}

impl Database {
    fn put(&mut self, key: String, value: String) {
        let revision = self
            .values
            .get(&key)
            .map(|(_, revision)| revision + 1)
            .unwrap_or(1);
        self.values.insert(key, (value, revision));
    }
    fn handle(&mut self, method: &str, params: &Value) -> std::result::Result<Value, String> {
        let key = params["key"].as_str().unwrap_or_default();
        match method {
            "sys_config_get" => {
                let result = self
                    .values
                    .get(key)
                    .map(|(value, revision)| json!({"value": value, "version": revision}))
                    .unwrap_or(Value::Null);
                if let Some((target, remaining, _)) = self.after_get.as_mut() {
                    if target == key {
                        *remaining -= 1;
                    }
                    if *remaining == 0 {
                        let (_, _, updates) = self.after_get.take().unwrap();
                        for (key, value) in updates {
                            self.put(key, value);
                        }
                    }
                }
                Ok(result)
            }
            "sys_config_create" => {
                if self.values.contains_key(key) {
                    return Err("key exists".into());
                }
                self.put(key.into(), params["value"].as_str().unwrap().into());
                Ok(json!(0))
            }
            "sys_config_list" => {
                let prefix = format!("{}/", key.trim_end_matches('/'));
                let children: BTreeSet<_> = self
                    .values
                    .keys()
                    .filter_map(|key| {
                        key.strip_prefix(&prefix)
                            .map(|suffix| suffix.split('/').next().unwrap())
                    })
                    .collect();
                Ok(json!(children))
            }
            "sys_config_exec_tx" => {
                for (key, value) in std::mem::take(&mut self.before_tx) {
                    self.put(key, value);
                }
                let main = &params["main_key"];
                if let Some(key) = main["key"].as_str() {
                    if self
                        .values
                        .get(key)
                        .map(|(_, version)| *version)
                        .unwrap_or(0)
                        != main["revision"].as_u64().unwrap()
                    {
                        return Err("revision mismatch".into());
                    }
                }
                let actions = params["actions"].as_object().unwrap();
                for (key, action) in actions {
                    if action["action"] == "create" && self.values.contains_key(key) {
                        return Err("key exists".into());
                    }
                }
                for (key, action) in actions {
                    match action["action"].as_str().unwrap() {
                        "create" | "update" => {
                            self.put(key.clone(), action["value"].as_str().unwrap().into())
                        }
                        "remove" => {
                            self.values.remove(key);
                        }
                        other => panic!("unexpected transaction action: {other}"),
                    }
                }
                if let Some(key) = main["key"]
                    .as_str()
                    .filter(|key| !actions.contains_key(*key))
                {
                    self.values.get_mut(key).unwrap().1 += 1;
                }
                Ok(json!(0))
            }
            other => panic!("unexpected RPC: {other}"),
        }
    }
}

struct TestConfig {
    db: Arc<Mutex<Database>>,
    server: SchedulerServer,
    listener: tokio::task::JoinHandle<()>,
}

impl Drop for TestConfig {
    fn drop(&mut self) {
        self.listener.abort();
    }
}

impl TestConfig {
    async fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/kapi/system_config",
            listener.local_addr().unwrap()
        );
        let db = Arc::new(Mutex::new(Database::default()));
        let server_db = db.clone();
        let listener = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let db = server_db.clone();
                tokio::spawn(async move {
                    let mut stream = BufReader::new(stream);
                    let mut length = 0;
                    loop {
                        let mut line = String::new();
                        if stream.read_line(&mut line).await.unwrap() == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((name, value)) = line.split_once(':') {
                            if name.eq_ignore_ascii_case("content-length") {
                                length = value.trim().parse::<usize>().unwrap();
                            }
                        }
                    }
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).await.unwrap();
                    let request: Value = serde_json::from_slice(&body).unwrap();
                    let result = db
                        .lock()
                        .unwrap()
                        .handle(request["method"].as_str().unwrap(), &request["params"]);
                    let response = match result {
                        Ok(result) => json!({"sys": [request["sys"][0]], "result": result}),
                        Err(error) => json!({"sys": [request["sys"][0]], "error": error}),
                    }
                    .to_string();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).as_bytes()).await.unwrap();
                });
            }
        });
        let server = SchedulerServer::new(Arc::new(SystemConfigClient::new(Some(&url), None)));
        Self {
            db,
            server,
            listener,
        }
    }
    fn put<T: serde::Serialize>(&self, key: &str, value: &T) {
        self.db
            .lock()
            .unwrap()
            .put(key.into(), serde_json::to_string(value).unwrap());
    }
    fn read<T: serde::de::DeserializeOwned>(&self, key: &str) -> T {
        serde_json::from_str(&self.db.lock().unwrap().values[key].0).unwrap()
    }
    fn seed(&self) -> (InstallPlanExecutionRecord, AppServiceSpec, InstallRecord) {
        let plan = fixture::plan();
        let (spec, installed) = fixture::installation(&plan, 1);
        let mut execution = InstallPlanExecutionRecord::new(plan);
        execution.state = InstallPlanExecutionState::Failed;
        execution.commit_point = InstallPlanCommitPoint::DesiredStateCommitted;
        execution.error = Some(install_error(
            InstallErrorCode::ActivationFailed,
            true,
            "schedule failed",
        ));
        self.put(&execution.key.storage_key(), &execution);
        self.put(&user_app_spec_key("alice", spec.app_id()), &spec);
        self.put(&install_record_key("alice", spec.app_id()), &installed);
        self.put(APP_REGISTRY_KEY, &AppRegistry::default());
        (execution, spec, installed)
    }
}

#[tokio::test]
async fn cancellation_before_submit_blocks_delayed_submission_and_retry() {
    let test = TestConfig::new().await;
    let plan = fixture::plan();
    let canceled = test
        .server
        .handle_cancel_install_plan(plan.clone(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(canceled.state, InstallPlanExecutionState::Canceled);
    let replay = test
        .server
        .handle_cancel_install_plan(plan.clone(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(replay, canceled);
    let delayed = test
        .server
        .handle_submit_install_plan(plan.clone(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(delayed.state, InstallPlanExecutionState::Canceled);
    let retried = test
        .server
        .handle_retry_install_plan(canceled.key, RPCContext::default())
        .await
        .unwrap();
    assert_eq!(retried.state, InstallPlanExecutionState::Canceled);
    assert_eq!(test.db.lock().unwrap().values.len(), 1);
}

#[tokio::test]
async fn claimed_cancel_invalidates_in_flight_registry_commit() {
    let test = TestConfig::new().await;
    let mut record = InstallPlanExecutionRecord::new(fixture::plan());
    record.state = InstallPlanExecutionState::Claimed;
    record.commit_point = InstallPlanCommitPoint::Claimed;
    test.put(&record.key.storage_key(), &record);
    test.put(APP_REGISTRY_KEY, &AppRegistry::default());
    let registry = test
        .server
        .system_config_client
        .get(APP_REGISTRY_KEY)
        .await
        .unwrap();
    let canceled = test
        .server
        .handle_cancel_install_plan(record.plan.clone(), RPCContext::default())
        .await
        .unwrap();
    assert_eq!(canceled.state, InstallPlanExecutionState::Canceled);
    let (spec, installed) = fixture::installation(&record.plan, 1);
    let spec_key = user_app_spec_key("alice", spec.app_id());
    let actions = HashMap::from([
        (
            spec_key.clone(),
            KVAction::Create(serialize(&spec).unwrap()),
        ),
        (
            install_record_key("alice", spec.app_id()),
            KVAction::Create(serialize(&installed).unwrap()),
        ),
        (
            record.key.storage_key(),
            KVAction::Update(serialize(&record).unwrap()),
        ),
    ]);
    assert!(test
        .server
        .system_config_client
        .exec_tx(actions, Some((APP_REGISTRY_KEY.into(), registry.version)))
        .await
        .is_err());
    assert!(!test.db.lock().unwrap().values.contains_key(&spec_key));
    assert_eq!(
        test.read::<InstallPlanExecutionRecord>(&record.key.storage_key())
            .state,
        InstallPlanExecutionState::Canceled
    );
}

#[tokio::test]
async fn recovery_after_uninstall_or_reinstall_leaves_current_installation_untouched() {
    for reinstall in [false, true] {
        let test = TestConfig::new().await;
        let (execution, mut spec, mut installed) = test.seed();
        if reinstall {
            let mut plan = execution.plan.clone();
            plan.task_id = "install-2".into();
            plan.plan_fingerprint = plan.expected_fingerprint();
            (spec, installed) = fixture::installation(&plan, 2);
        } else {
            spec.state = ServiceState::Deleted;
        }
        let spec_key = user_app_spec_key("alice", spec.app_id());
        let record_key = install_record_key("alice", spec.app_id());
        test.put(&spec_key, &spec);
        test.put(&record_key, &installed);
        test.server.recover_install_plan_executions().await.unwrap();
        assert_eq!(test.read::<AppServiceSpec>(&spec_key), spec);
        assert_eq!(test.read::<InstallRecord>(&record_key), installed);
        let failed = test.read::<InstallPlanExecutionRecord>(&execution.key.storage_key());
        assert!(!failed.error.unwrap().retryable);
        test.server.recover_install_plan_executions().await.unwrap();
        assert_eq!(test.read::<InstallRecord>(&record_key), installed);
    }
}

#[tokio::test]
async fn committed_failure_and_success_update_matching_install_status() {
    let test = TestConfig::new().await;
    let (execution, spec, _) = test.seed();
    let (record, revision) = test.server.load_execution(&execution.key).await.unwrap();
    let error = install_error(
        InstallErrorCode::ActivationFailed,
        true,
        "node config unavailable",
    );
    test.server
        .fail_execution(
            &execution.key.storage_key(),
            record,
            revision,
            error.clone(),
        )
        .await
        .unwrap();
    let record_key = install_record_key("alice", spec.app_id());
    let failed: InstallRecord = test.read(&record_key);
    assert_eq!(
        failed.state,
        InstallRecordState::DeployedButActivationFailed
    );
    assert_eq!(failed.last_error, Some(error));
    test.server
        .complete_install_execution(&execution.key)
        .await
        .unwrap();
    let installed: InstallRecord = test.read(&record_key);
    assert_eq!(installed.state, InstallRecordState::Installed);
    assert!(installed.last_error.is_none());
    assert!(test
        .read::<InstallPlanExecutionRecord>(&execution.key.storage_key())
        .error
        .is_none());
}

#[tokio::test]
async fn reinstall_between_completion_read_and_write_fails_cas() {
    let test = TestConfig::new().await;
    let (execution, _, _) = test.seed();
    let mut new_plan = execution.plan.clone();
    new_plan.task_id = "install-2".into();
    new_plan.plan_fingerprint = new_plan.expected_fingerprint();
    let (spec, installed) = fixture::installation(&new_plan, 2);
    let spec_key = user_app_spec_key("alice", spec.app_id());
    let record_key = install_record_key("alice", spec.app_id());
    test.db.lock().unwrap().before_tx = vec![
        (spec_key.clone(), serialize(&spec).unwrap()),
        (record_key.clone(), serialize(&installed).unwrap()),
    ];
    assert!(test
        .server
        .complete_install_execution(&execution.key)
        .await
        .is_err());
    assert_eq!(test.read::<AppServiceSpec>(&spec_key), spec);
    assert_eq!(test.read::<InstallRecord>(&record_key), installed);
}

#[tokio::test]
async fn cancel_after_commit_is_rejected_without_mutating_metadata() {
    let test = TestConfig::new().await;
    let (execution, spec, installed) = test.seed();
    assert!(test
        .server
        .handle_cancel_install_plan(execution.plan.clone(), RPCContext::default())
        .await
        .is_err());
    assert_eq!(
        test.read::<AppServiceSpec>(&user_app_spec_key("alice", spec.app_id())),
        spec
    );
    assert_eq!(
        test.read::<InstallRecord>(&install_record_key("alice", spec.app_id())),
        installed
    );
    assert_eq!(
        test.read::<InstallPlanExecutionRecord>(&execution.key.storage_key()),
        execution
    );
}

#[tokio::test]
async fn late_failure_does_not_downgrade_completed_execution() {
    let test = TestConfig::new().await;
    let (execution, spec, _) = test.seed();
    let (stale, revision) = test.server.load_execution(&execution.key).await.unwrap();
    test.server
        .complete_install_execution(&execution.key)
        .await
        .unwrap();
    let result = test
        .server
        .fail_execution(
            &execution.key.storage_key(),
            stale,
            revision,
            install_error(
                InstallErrorCode::ActivationFailed,
                true,
                "late scheduling error",
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.state, InstallPlanExecutionState::Completed);
    let installed: InstallRecord = test.read(&install_record_key("alice", spec.app_id()));
    assert_eq!(installed.state, InstallRecordState::Installed);
    assert!(installed.last_error.is_none());
}

#[tokio::test]
async fn cancel_between_execution_snapshot_and_commit_prevents_spec_creation() {
    let test = TestConfig::new().await;
    let plan = fixture::plan();
    let mut record = InstallPlanExecutionRecord::new(plan.clone());
    record.state = InstallPlanExecutionState::Claimed;
    record.commit_point = InstallPlanCommitPoint::Claimed;
    let path = record.key.storage_key();
    test.put(&path, &record);
    test.put(APP_REGISTRY_KEY, &AppRegistry::default());
    test.put(ZONE_OWNER_USER_ID_KEY, &"alice");
    let mut canceled = record.clone();
    canceled.state = InstallPlanExecutionState::Canceled;
    test.db.lock().unwrap().after_get = Some((
        path.clone(),
        3,
        vec![
            (path, serialize(&canceled).unwrap()),
            (
                APP_REGISTRY_KEY.into(),
                serialize(&AppRegistry::default()).unwrap(),
            ),
        ],
    ));
    let result = test
        .server
        .execute_install_plan(&record.key, true)
        .await
        .unwrap();
    assert_eq!(result.state, InstallPlanExecutionState::Canceled);
    let db = test.db.lock().unwrap();
    assert!(!db
        .values
        .contains_key(&user_app_spec_key("alice", plan.app_instance_id.app_id())));
    assert!(!db
        .values
        .contains_key(&install_record_key("alice", plan.app_instance_id.app_id())));
}
