use ::kRPC::*;
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    get_session_token_env_key, AffinityType, FunctionObject, FunctionParamType, FunctionType,
    SchedulerDispatchReceipt, SchedulerRunThunkResponse, SchedulerRunThunkStatus,
    SystemConfigClient, ThunkObject,
    RESOURCE_TYPE_CPU, RESOURCE_TYPE_DISK_CACHE, RESOURCE_TYPE_DOWNLOAD, RESOURCE_TYPE_GPU,
    RESOURCE_TYPE_GPU_CORES, RESOURCE_TYPE_GPU_MEMORY, RESOURCE_TYPE_MEMORY,
    RESOURCE_TYPE_UPLOAD, SCHEDULER_SERVICE_SERVICE_NAME,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::Arc;

use crate::scheduler::{NodeItem, NodeState};
use crate::system_config_agent::create_scheduler_by_system_config;

const SYSTEM_CONFIG_URL: &str = "http://127.0.0.1:3200/kapi/system_config";

#[derive(Debug, Clone, Default)]
pub struct FunctionObjectSchedulingHint {
    pub source: String,
    pub preferred_runner: Option<String>,
    pub affinity_selectors: Vec<String>,
    pub affinity_type: String,
    pub requires_gpu: bool,
}

#[derive(Debug, Clone)]
pub struct ThunkNodeCandidate {
    pub node_id: String,
    pub runner: String,
    pub labels: HashMap<String, String>,
    pub label_set: HashSet<String>,
    pub available_resources: HashMap<String, u64>,
    pub total_resources: HashMap<String, u64>,
    pub network_zone: String,
    pub has_gpu: bool,
    pub score: i64,
}

#[async_trait]
pub trait NamedObjectInspector: Send + Sync {
    async fn exists(&self, obj_id: &str) -> Result<bool>;
}

#[async_trait]
pub trait ThunkNodeCatalog: Send + Sync {
    async fn list_candidates(
        &self,
        thunk: &ThunkObject,
        hint: &FunctionObjectSchedulingHint,
    ) -> Result<Vec<ThunkNodeCandidate>>;
}

#[async_trait]
pub trait ThunkDispatchBackend: Send + Sync {
    async fn dispatch(
        &self,
        node: &ThunkNodeCandidate,
        task_id: i64,
        thunk_obj_id: &str,
        thunk: &ThunkObject,
        function_object: &FunctionObject,
        hint: &FunctionObjectSchedulingHint,
    ) -> Result<SchedulerDispatchReceipt>;
}

#[derive(Default)]
pub struct PlaceholderNamedObjectInspector;

#[async_trait]
impl NamedObjectInspector for PlaceholderNamedObjectInspector {
    async fn exists(&self, obj_id: &str) -> Result<bool> {
        Ok(!obj_id.trim().is_empty())
    }
}

pub struct SystemConfigBackedNodeCatalog {
    system_config_url: String,
    session_token: Option<String>,
}

impl Default for SystemConfigBackedNodeCatalog {
    fn default() -> Self {
        Self {
            system_config_url: env::var("SYSTEM_CONFIG_URL")
                .unwrap_or_else(|_| SYSTEM_CONFIG_URL.to_string()),
            session_token: load_scheduler_session_token(),
        }
    }
}

#[async_trait]
impl ThunkNodeCatalog for SystemConfigBackedNodeCatalog {
    async fn list_candidates(
        &self,
        _thunk: &ThunkObject,
        hint: &FunctionObjectSchedulingHint,
    ) -> Result<Vec<ThunkNodeCandidate>> {
        let client = SystemConfigClient::new(
            Some(self.system_config_url.as_str()),
            self.session_token.as_deref(),
        );
        let dumped = client
            .dump_configs_for_scheduler()
            .await
            .context("dump_configs_for_scheduler failed")?;
        let input = parse_scheduler_config_dump(dumped)?;
        let (scheduler, _) = create_scheduler_by_system_config(&input)
            .context("create_scheduler_by_system_config failed")?;

        Ok(scheduler
            .nodes
            .values()
            .filter(|node| matches!(&node.state, NodeState::Ready))
            .map(|node| build_candidate_from_node(node, hint))
            .collect())
    }
}

pub struct ExternalTaskThunkDispatchBackend;

#[async_trait]
impl ThunkDispatchBackend for ExternalTaskThunkDispatchBackend {
    async fn dispatch(
        &self,
        node: &ThunkNodeCandidate,
        task_id: i64,
        thunk_obj_id: &str,
        thunk: &ThunkObject,
        function_object: &FunctionObject,
        hint: &FunctionObjectSchedulingHint,
    ) -> Result<SchedulerDispatchReceipt> {
        Ok(SchedulerDispatchReceipt {
            node_id: node.node_id.clone(),
            dispatch_type: "external_task".to_string(),
            task_id: Some(task_id),
            runner: Some(node.runner.clone()),
            function_hint_source: Some(hint.source.clone()),
            details: json!({
                "task_id": task_id,
                "thunk_obj_id": thunk_obj_id,
                "fun_id": thunk.fun_id.to_string(),
                "function_type": function_type_name(&function_object.func_type),
                "node_id": node.node_id.clone(),
                "runner": node.runner.clone(),
                "network_zone": node.network_zone.clone(),
                "affinity_type": hint.affinity_type.clone(),
                "affinity_selectors": hint.affinity_selectors.clone(),
                "available_resources": node.available_resources.clone(),
                "next_step": "external dispatcher should bind task to runner/node",
            }),
        })
    }
}

pub struct DefaultThunkRunner {
    named_object_inspector: Arc<dyn NamedObjectInspector>,
    node_catalog: Arc<dyn ThunkNodeCatalog>,
    dispatch_backend: Arc<dyn ThunkDispatchBackend>,
}

impl Default for DefaultThunkRunner {
    fn default() -> Self {
        Self {
            named_object_inspector: Arc::new(PlaceholderNamedObjectInspector),
            node_catalog: Arc::new(SystemConfigBackedNodeCatalog::default()),
            dispatch_backend: Arc::new(ExternalTaskThunkDispatchBackend),
        }
    }
}

impl DefaultThunkRunner {
    pub fn new(
        named_object_inspector: Arc<dyn NamedObjectInspector>,
        node_catalog: Arc<dyn ThunkNodeCatalog>,
        dispatch_backend: Arc<dyn ThunkDispatchBackend>,
    ) -> Self {
        Self {
            named_object_inspector,
            node_catalog,
            dispatch_backend,
        }
    }

    pub async fn run_thunk(
        &self,
        task_id: i64,
        thunk: ThunkObject,
        function_object: FunctionObject,
    ) -> Result<SchedulerRunThunkResponse> {
        let thunk_obj_id = calc_thunk_obj_id(&thunk)?;
        if !self
            .check_param_is_ready(&thunk, &function_object)
            .await?
        {
            return Ok(SchedulerRunThunkResponse {
                thunk_obj_id,
                status: SchedulerRunThunkStatus::Rejected,
                reason: Some("params_not_ready".to_string()),
                dispatch: None,
            });
        }

        let function_hint = build_scheduling_hint(&function_object);
        let candidates = self.node_catalog.list_candidates(&thunk, &function_hint).await?;
        let Some(node) = select_best_node(&function_object, &function_hint, &candidates) else {
            return Ok(SchedulerRunThunkResponse {
                thunk_obj_id,
                status: SchedulerRunThunkStatus::Rejected,
                reason: Some("no_executor_node_available".to_string()),
                dispatch: None,
            });
        };

        let dispatch = self
            .dispatch_backend
            .dispatch(
                node,
                task_id,
                &thunk_obj_id,
                &thunk,
                &function_object,
                &function_hint,
            )
            .await?;

        Ok(SchedulerRunThunkResponse {
            thunk_obj_id,
            status: SchedulerRunThunkStatus::Dispatched,
            reason: None,
            dispatch: Some(dispatch),
        })
    }

    pub async fn check_param_is_ready(
        &self,
        thunk: &ThunkObject,
        function_object: &FunctionObject,
    ) -> Result<bool> {
        for (param_name, param_type) in &function_object.params_type {
            let Some(param_value) = thunk.params.get(param_name) else {
                return Ok(false);
            };

            match param_type {
                FunctionParamType::Fixed(_) | FunctionParamType::CheckByRunner(_) => {}
                FunctionParamType::ObjId(_) => {
                    let Some(obj_id) = param_value.as_str() else {
                        return Ok(false);
                    };
                    if !self.named_object_inspector.exists(obj_id).await? {
                        return Ok(false);
                    }
                }
            }
        }

        Ok(true)
    }
}

fn build_scheduling_hint(function_object: &FunctionObject) -> FunctionObjectSchedulingHint {
    let preferred_runner = match &function_object.func_type {
        FunctionType::ExecPkg => Some("package-runner".to_string()),
        FunctionType::Script(language) => Some(format!("script-runner:{language}")),
        FunctionType::Operator => Some("operator-runner".to_string()),
    };

    let affinity_selectors = match &function_object.affinity_type {
        AffinityType::Custom(selectors) => selectors.clone(),
        _ => Vec::new(),
    };

    FunctionObjectSchedulingHint {
        source: "provided_function_object".to_string(),
        preferred_runner,
        affinity_selectors,
        affinity_type: affinity_type_name(&function_object.affinity_type).to_string(),
        requires_gpu: function_requires_gpu(function_object),
    }
}

fn select_best_node<'a>(
    function_object: &FunctionObject,
    hint: &FunctionObjectSchedulingHint,
    candidates: &'a [ThunkNodeCandidate],
) -> Option<&'a ThunkNodeCandidate> {
    candidates
        .iter()
        .filter(|node| node_satisfies_requirements(node, function_object, hint))
        .max_by_key(|node| score_node(node, function_object, hint))
}

fn node_satisfies_requirements(
    node: &ThunkNodeCandidate,
    function_object: &FunctionObject,
    hint: &FunctionObjectSchedulingHint,
) -> bool {
    if hint.requires_gpu && !node.has_gpu {
        return false;
    }

    if !hint
        .affinity_selectors
        .iter()
        .all(|selector| node_matches_selector(node, selector))
    {
        return false;
    }

    for (resource_type, min_value) in &function_object.requirements {
        if !resource_is_present(node, resource_type) {
            return false;
        }

        if *min_value > 0 {
            let available = available_resource(node, resource_type).unwrap_or_default();
            if available < *min_value {
                return false;
            }
        }
    }

    true
}

fn score_node(
    node: &ThunkNodeCandidate,
    function_object: &FunctionObject,
    hint: &FunctionObjectSchedulingHint,
) -> i64 {
    let mut score = node.score;

    if function_object.best_run_weight.is_empty() {
        return score;
    }

    for selector in &hint.affinity_selectors {
        if node_matches_selector(node, selector) {
            score += 500;
        }
    }

    for (resource_type, weight) in &function_object.best_run_weight {
        score += score_weighted_resource(
            node,
            resource_type,
            *weight,
            function_object.requirements.get(resource_type).copied(),
        );
    }

    score
}

fn score_weighted_resource(
    node: &ThunkNodeCandidate,
    resource_type: &str,
    weight: u64,
    required_value: Option<u64>,
) -> i64 {
    if !resource_is_present(node, resource_type) {
        return 0;
    }

    let weight = weight as i64;
    if is_presence_selector(resource_type) {
        return 1000 * weight;
    }

    let available = available_resource(node, resource_type).unwrap_or_default();
    let total = total_resource(node, resource_type).unwrap_or(available).max(1);
    let required = required_value.unwrap_or_default();

    let normalized = if required > 0 {
        let headroom = available.saturating_sub(required);
        headroom.saturating_mul(1000) / total
    } else {
        available.saturating_mul(1000) / total
    };

    normalized as i64 * weight
}

fn function_requires_gpu(function_object: &FunctionObject) -> bool {
    function_object.requirements.contains_key(RESOURCE_TYPE_GPU)
        || function_object
            .requirements
            .contains_key(RESOURCE_TYPE_GPU_MEMORY)
        || function_object
            .requirements
            .contains_key(RESOURCE_TYPE_GPU_CORES)
}

fn build_candidate_from_node(
    node: &NodeItem,
    hint: &FunctionObjectSchedulingHint,
) -> ThunkNodeCandidate {
    let (labels, label_set) = normalize_node_labels(&node.labels);
    let (available_resources, total_resources) = build_resource_maps(node);
    let has_gpu = total_resources
        .get(RESOURCE_TYPE_GPU)
        .copied()
        .unwrap_or_default()
        > 0
        || total_resources
            .get(RESOURCE_TYPE_GPU_MEMORY)
            .copied()
            .unwrap_or_default()
            > 0
        || total_resources
            .get(RESOURCE_TYPE_GPU_CORES)
            .copied()
            .unwrap_or_default()
            > 0;

    ThunkNodeCandidate {
        node_id: node.id.clone(),
        runner: hint
            .preferred_runner
            .clone()
            .unwrap_or_else(|| "function-runner".to_string()),
        labels,
        label_set,
        score: default_capacity_score(&available_resources, &total_resources),
        available_resources,
        total_resources,
        network_zone: node.network_zone.clone(),
        has_gpu,
    }
}

fn build_resource_maps(node: &NodeItem) -> (HashMap<String, u64>, HashMap<String, u64>) {
    let mut available = HashMap::new();
    let mut total = HashMap::new();

    available.insert(RESOURCE_TYPE_CPU.to_string(), node.available_cpu_mhz as u64);
    total.insert(RESOURCE_TYPE_CPU.to_string(), node.total_cpu_mhz as u64);

    available.insert(RESOURCE_TYPE_MEMORY.to_string(), node.available_memory);
    total.insert(RESOURCE_TYPE_MEMORY.to_string(), node.total_memory);

    available.insert(
        RESOURCE_TYPE_GPU_MEMORY.to_string(),
        node.available_gpu_memory,
    );
    total.insert(RESOURCE_TYPE_GPU_MEMORY.to_string(), node.total_gpu_memory);

    let gpu_tflops = node.gpu_tflops.max(0.0).round() as u64;
    available.insert(RESOURCE_TYPE_GPU.to_string(), gpu_tflops);
    total.insert(RESOURCE_TYPE_GPU.to_string(), gpu_tflops);

    for (resource_name, resource) in &node.resources {
        available.insert(
            resource_name.clone(),
            resource.total_capacity.saturating_sub(resource.used_capacity),
        );
        total.insert(resource_name.clone(), resource.total_capacity);
    }

    (available, total)
}

fn default_capacity_score(
    available_resources: &HashMap<String, u64>,
    total_resources: &HashMap<String, u64>,
) -> i64 {
    let mut score = 0;
    for key in [
        RESOURCE_TYPE_CPU,
        RESOURCE_TYPE_MEMORY,
        RESOURCE_TYPE_GPU_MEMORY,
        RESOURCE_TYPE_GPU,
    ] {
        let total = total_resources.get(key).copied().unwrap_or_default();
        if total == 0 {
            continue;
        }
        let available = available_resources.get(key).copied().unwrap_or_default();
        score += available.min(total).saturating_mul(100) as i64 / total as i64;
    }
    score.max(1)
}

fn normalize_node_labels(labels: &[String]) -> (HashMap<String, String>, HashSet<String>) {
    let mut map = HashMap::new();
    let mut set = HashSet::new();

    for raw in labels {
        set.insert(raw.clone());
        if let Some((key, value)) = raw.split_once('=') {
            map.insert(key.to_string(), value.to_string());
            set.insert(key.to_string());
            continue;
        }
        if !raw.starts_with('/') {
            if let Some((key, value)) = raw.split_once(':') {
                map.insert(key.to_string(), value.to_string());
                set.insert(key.to_string());
                continue;
            }
        }
        map.entry(raw.clone()).or_insert_with(|| "true".to_string());
    }

    (map, set)
}

fn resource_is_present(node: &ThunkNodeCandidate, resource_type: &str) -> bool {
    match resource_type {
        RESOURCE_TYPE_CPU | RESOURCE_TYPE_MEMORY => true,
        _ => total_resource(node, resource_type).unwrap_or_default() > 0
            || available_resource(node, resource_type).unwrap_or_default() > 0
            || node_matches_selector(node, resource_type),
    }
}

fn available_resource(node: &ThunkNodeCandidate, resource_type: &str) -> Option<u64> {
    node.available_resources
        .get(resource_type)
        .copied()
        .or_else(|| node_matches_selector(node, resource_type).then_some(1))
}

fn total_resource(node: &ThunkNodeCandidate, resource_type: &str) -> Option<u64> {
    node.total_resources
        .get(resource_type)
        .copied()
        .or_else(|| node_matches_selector(node, resource_type).then_some(1))
}

fn node_matches_selector(node: &ThunkNodeCandidate, selector: &str) -> bool {
    if let Some((key, expected)) = selector.split_once('=') {
        return node.labels.get(key).map(|value| value.as_str()) == Some(expected);
    }

    if !selector.starts_with('/') {
        if let Some((key, expected)) = selector.split_once(':') {
            return node.labels.get(key).map(|value| value.as_str()) == Some(expected);
        }
    }

    node.label_set.contains(selector)
        || node.labels.contains_key(selector)
        || node.available_resources.contains_key(selector)
}

fn is_presence_selector(resource_type: &str) -> bool {
    !matches!(
        resource_type,
        RESOURCE_TYPE_CPU
            | RESOURCE_TYPE_MEMORY
            | RESOURCE_TYPE_GPU
            | RESOURCE_TYPE_GPU_MEMORY
            | RESOURCE_TYPE_GPU_CORES
            | RESOURCE_TYPE_DISK_CACHE
            | RESOURCE_TYPE_UPLOAD
            | RESOURCE_TYPE_DOWNLOAD
    )
}

fn parse_scheduler_config_dump(value: Value) -> Result<HashMap<String, String>> {
    let Value::Object(map) = value else {
        bail!("dump_configs_for_scheduler returned non-object payload");
    };

    let mut result = HashMap::new();
    for (key, value) in map {
        let Some(value) = value.as_str() else {
            bail!("scheduler dump value for key {} is not a string", key);
        };
        result.insert(key, value.to_string());
    }
    Ok(result)
}

fn load_scheduler_session_token() -> Option<String> {
    let key = get_session_token_env_key(SCHEDULER_SERVICE_SERVICE_NAME, false);
    env::var(&key)
        .ok()
        .or_else(|| env::var("SCHEDULER_SESSION_TOKEN").ok())
}

fn calc_thunk_obj_id(thunk: &ThunkObject) -> Result<String> {
    let bytes = serde_json::to_vec(thunk)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    Ok(format!("thunk:{}", hex::encode(hasher.finalize())))
}

fn function_type_name(func_type: &FunctionType) -> &'static str {
    match func_type {
        FunctionType::ExecPkg => "exec_pkg",
        FunctionType::Script(_) => "script",
        FunctionType::Operator => "operator",
    }
}

fn affinity_type_name(affinity_type: &AffinityType) -> &'static str {
    match affinity_type {
        AffinityType::Input => "input",
        AffinityType::Result => "result",
        AffinityType::Custom(_) => "custom",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AffinityType, FunctionObject, FunctionParamType, FunctionResultType, FunctionType,
        ThunkObject, RESOURCE_TYPE_CPU, RESOURCE_TYPE_MEMORY,
    };
    use ndn_lib::ObjId;
    use serde_json::json;

    struct MockNodeCatalog {
        nodes: Vec<ThunkNodeCandidate>,
    }

    #[async_trait]
    impl ThunkNodeCatalog for MockNodeCatalog {
        async fn list_candidates(
            &self,
            _thunk: &ThunkObject,
            _hint: &FunctionObjectSchedulingHint,
        ) -> Result<Vec<ThunkNodeCandidate>> {
            Ok(self.nodes.clone())
        }
    }

    #[derive(Default)]
    struct MockDispatchBackend;

    #[async_trait]
    impl ThunkDispatchBackend for MockDispatchBackend {
        async fn dispatch(
            &self,
            node: &ThunkNodeCandidate,
            task_id: i64,
            thunk_obj_id: &str,
            _thunk: &ThunkObject,
            _function_object: &FunctionObject,
            hint: &FunctionObjectSchedulingHint,
        ) -> Result<SchedulerDispatchReceipt> {
            Ok(SchedulerDispatchReceipt {
                node_id: node.node_id.clone(),
                dispatch_type: "mock".to_string(),
                task_id: Some(task_id),
                runner: Some(node.runner.clone()),
                function_hint_source: Some(hint.source.clone()),
                details: json!({
                    "thunk_obj_id": thunk_obj_id,
                    "node_id": node.node_id.clone(),
                }),
            })
        }
    }

    fn sample_thunk() -> ThunkObject {
        ThunkObject {
            fun_id: ObjId::new("func:1234567890").unwrap(),
            params: HashMap::from([("input".to_string(), json!("obj:1234567890"))]),
            metadata: json!({
                "run_id": "run-1",
                "attempt": 1
            }),
        }
    }

    fn sample_function_object() -> FunctionObject {
        FunctionObject {
            func_type: FunctionType::ExecPkg,
            content: "pkg:test".to_string(),
            is_pure: true,
            timeout: None,
            requirements: HashMap::from([
                (RESOURCE_TYPE_CPU.to_string(), 100),
                (RESOURCE_TYPE_MEMORY.to_string(), 256),
            ]),
            best_run_weight: HashMap::from([(RESOURCE_TYPE_CPU.to_string(), 2)]),
            affinity_type: AffinityType::Input,
            params_type: HashMap::from([(
                "input".to_string(),
                FunctionParamType::ObjId("named_object".to_string()),
            )]),
            result_type: FunctionResultType::Object("named_object".to_string()),
        }
    }

    fn sample_node(node_id: &str, available_cpu: u64, available_memory: u64) -> ThunkNodeCandidate {
        ThunkNodeCandidate {
            node_id: node_id.to_string(),
            runner: "package-runner".to_string(),
            labels: HashMap::new(),
            label_set: HashSet::new(),
            available_resources: HashMap::from([
                (RESOURCE_TYPE_CPU.to_string(), available_cpu),
                (RESOURCE_TYPE_MEMORY.to_string(), available_memory),
            ]),
            total_resources: HashMap::from([
                (RESOURCE_TYPE_CPU.to_string(), 1000),
                (RESOURCE_TYPE_MEMORY.to_string(), 2048),
            ]),
            network_zone: "zone-a".to_string(),
            has_gpu: false,
            score: 100,
        }
    }

    fn mock_runner(nodes: Vec<ThunkNodeCandidate>) -> DefaultThunkRunner {
        DefaultThunkRunner::new(
            Arc::new(PlaceholderNamedObjectInspector),
            Arc::new(MockNodeCatalog { nodes }),
            Arc::new(MockDispatchBackend),
        )
    }

    #[tokio::test]
    async fn fixed_params_are_ready() {
        let runner = mock_runner(vec![sample_node("node-a", 600, 1024)]);
        assert!(runner
            .check_param_is_ready(&sample_thunk(), &sample_function_object())
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn run_thunk_returns_dispatch_receipt() {
        let runner = mock_runner(vec![sample_node("node-a", 600, 1024)]);
        let response = runner
            .run_thunk(42, sample_thunk(), sample_function_object())
            .await
            .unwrap();

        assert_eq!(response.status, SchedulerRunThunkStatus::Dispatched);
        assert_eq!(response.dispatch.unwrap().task_id, Some(42));
    }

    #[tokio::test]
    async fn run_thunk_rejects_when_no_node_meets_requirements() {
        let runner = mock_runner(vec![sample_node("node-a", 50, 128)]);
        let response = runner
            .run_thunk(7, sample_thunk(), sample_function_object())
            .await
            .unwrap();

        assert_eq!(response.status, SchedulerRunThunkStatus::Rejected);
        assert_eq!(response.reason.as_deref(), Some("no_executor_node_available"));
    }

    #[test]
    fn custom_affinity_selector_matches_node_labels() {
        let hint = FunctionObjectSchedulingHint {
            affinity_selectors: vec!["/data/model-a".to_string(), "zone=public".to_string()],
            ..Default::default()
        };
        let mut node = sample_node("node-a", 600, 1024);
        node.label_set.insert("/data/model-a".to_string());
        node.labels
            .insert("zone".to_string(), "public".to_string());

        assert!(hint
            .affinity_selectors
            .iter()
            .all(|selector| node_matches_selector(&node, selector)));
    }
}
