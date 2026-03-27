use anyhow::Result;
use async_trait::async_trait;
use buckyos_api::{
    SchedulerDispatchReceipt, SchedulerRunThunkResponse, SchedulerRunThunkStatus, ThunkObject,
    ThunkParamType,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::env;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FunctionObjectSchedulingHint {
    pub source: String,
    #[serde(default)]
    pub preferred_runner: Option<String>,
    #[serde(default)]
    pub required_labels: HashMap<String, String>,
    #[serde(default)]
    pub requires_gpu: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkNodeCandidate {
    pub node_id: String,
    pub runner: String,
    #[serde(default)]
    pub labels: HashMap<String, String>,
    #[serde(default)]
    pub has_gpu: bool,
    #[serde(default)]
    pub score: i64,
}

#[async_trait]
pub trait NamedObjectInspector: Send + Sync {
    async fn exists(&self, obj_id: &str) -> Result<bool>;
}

#[async_trait]
pub trait FunctionObjectResolver: Send + Sync {
    async fn resolve(&self, thunk: &ThunkObject) -> Result<FunctionObjectSchedulingHint>;
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
        thunk: &ThunkObject,
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

#[derive(Default)]
pub struct PlaceholderFunctionObjectResolver;

#[async_trait]
impl FunctionObjectResolver for PlaceholderFunctionObjectResolver {
    async fn resolve(&self, thunk: &ThunkObject) -> Result<FunctionObjectSchedulingHint> {
        Ok(FunctionObjectSchedulingHint {
            source: "placeholder_from_fun_id".to_string(),
            preferred_runner: Some("local-placeholder-runner".to_string()),
            required_labels: HashMap::new(),
            requires_gpu: Some(thunk.resource_requirements.gpu_required),
        })
    }
}

#[derive(Default)]
pub struct LocalThunkNodeCatalog;

#[async_trait]
impl ThunkNodeCatalog for LocalThunkNodeCatalog {
    async fn list_candidates(
        &self,
        _thunk: &ThunkObject,
        hint: &FunctionObjectSchedulingHint,
    ) -> Result<Vec<ThunkNodeCandidate>> {
        let node_id = env::var("SCHEDULER_LOCAL_NODE_ID")
            .or_else(|_| env::var("HOSTNAME"))
            .unwrap_or_else(|_| "scheduler-local".to_string());
        let runner = hint
            .preferred_runner
            .clone()
            .unwrap_or_else(|| "local-placeholder-runner".to_string());

        Ok(vec![ThunkNodeCandidate {
            node_id,
            runner,
            labels: HashMap::new(),
            has_gpu: false,
            score: 100,
        }])
    }
}

#[derive(Default)]
pub struct PlaceholderThunkDispatchBackend;

#[async_trait]
impl ThunkDispatchBackend for PlaceholderThunkDispatchBackend {
    async fn dispatch(
        &self,
        node: &ThunkNodeCandidate,
        thunk: &ThunkObject,
        hint: &FunctionObjectSchedulingHint,
    ) -> Result<SchedulerDispatchReceipt> {
        Ok(SchedulerDispatchReceipt {
            node_id: node.node_id.clone(),
            dispatch_type: "placeholder".to_string(),
            runner: Some(node.runner.clone()),
            function_hint_source: Some(hint.source.clone()),
            details: json!({
                "thunk_obj_id": thunk.thunk_obj_id,
                "fun_id": thunk.fun_id,
                "next_step": "wire real function_object resolution and executor dispatch"
            }),
        })
    }
}

pub struct DefaultThunkRunner {
    named_object_inspector: Arc<dyn NamedObjectInspector>,
    function_resolver: Arc<dyn FunctionObjectResolver>,
    node_catalog: Arc<dyn ThunkNodeCatalog>,
    dispatch_backend: Arc<dyn ThunkDispatchBackend>,
}

impl Default for DefaultThunkRunner {
    fn default() -> Self {
        Self {
            named_object_inspector: Arc::new(PlaceholderNamedObjectInspector),
            function_resolver: Arc::new(PlaceholderFunctionObjectResolver),
            node_catalog: Arc::new(LocalThunkNodeCatalog),
            dispatch_backend: Arc::new(PlaceholderThunkDispatchBackend),
        }
    }
}

impl DefaultThunkRunner {
    pub async fn run_thunk(&self, thunk: ThunkObject) -> Result<SchedulerRunThunkResponse> {
        if !self.check_param_is_ready(&thunk).await? {
            return Ok(SchedulerRunThunkResponse {
                thunk_obj_id: thunk.thunk_obj_id,
                status: SchedulerRunThunkStatus::Rejected,
                reason: Some("params_not_ready".to_string()),
                dispatch: None,
            });
        }

        let function_hint = self.function_resolver.resolve(&thunk).await?;
        let candidates = self.node_catalog.list_candidates(&thunk, &function_hint).await?;
        let Some(node) = select_best_node(&thunk, &function_hint, &candidates) else {
            return Ok(SchedulerRunThunkResponse {
                thunk_obj_id: thunk.thunk_obj_id,
                status: SchedulerRunThunkStatus::Rejected,
                reason: Some("no_executor_node_available".to_string()),
                dispatch: None,
            });
        };

        let dispatch = self
            .dispatch_backend
            .dispatch(node, &thunk, &function_hint)
            .await?;

        Ok(SchedulerRunThunkResponse {
            thunk_obj_id: thunk.thunk_obj_id,
            status: SchedulerRunThunkStatus::Dispatched,
            reason: None,
            dispatch: Some(dispatch),
        })
    }

    pub async fn check_param_is_ready(&self, thunk: &ThunkObject) -> Result<bool> {
        match thunk.params.param_type {
            ThunkParamType::CheckByRunner | ThunkParamType::Fixed => Ok(true),
            ThunkParamType::Normal => {
                for obj_ref in &thunk.params.obj_refs {
                    if !self.named_object_inspector.exists(obj_ref).await? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
        }
    }
}

fn select_best_node<'a>(
    thunk: &ThunkObject,
    hint: &FunctionObjectSchedulingHint,
    candidates: &'a [ThunkNodeCandidate],
) -> Option<&'a ThunkNodeCandidate> {
    candidates
        .iter()
        .filter(|node| node_satisfies_requirements(node, thunk, hint))
        .max_by_key(|node| node.score)
}

fn node_satisfies_requirements(
    node: &ThunkNodeCandidate,
    thunk: &ThunkObject,
    hint: &FunctionObjectSchedulingHint,
) -> bool {
    let requires_gpu = hint
        .requires_gpu
        .unwrap_or(thunk.resource_requirements.gpu_required);
    if requires_gpu && !node.has_gpu {
        return false;
    }

    hint.required_labels
        .iter()
        .all(|(key, expected)| node.labels.get(key) == Some(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{ResourceRequirements, ThunkMetadata, ThunkObject, ThunkParams};
    use serde_json::json;

    fn sample_thunk(param_type: ThunkParamType) -> ThunkObject {
        ThunkObject {
            thunk_obj_id: "thunk-1".to_string(),
            fun_id: "fun-1".to_string(),
            params: ThunkParams {
                param_type,
                values: json!({"hello": "world"}),
                obj_refs: vec!["obj-1".to_string()],
            },
            idempotent: true,
            resource_requirements: ResourceRequirements::default(),
            metadata: ThunkMetadata {
                run_id: "run-1".to_string(),
                node_id: "node-1".to_string(),
                attempt: 1,
                shard: None,
            },
        }
    }

    #[tokio::test]
    async fn fixed_params_are_ready() {
        let runner = DefaultThunkRunner::default();
        assert!(runner
            .check_param_is_ready(&sample_thunk(ThunkParamType::Fixed))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn run_thunk_returns_dispatch_receipt() {
        let runner = DefaultThunkRunner::default();
        let response = runner
            .run_thunk(sample_thunk(ThunkParamType::Fixed))
            .await
            .unwrap();

        assert_eq!(response.status, SchedulerRunThunkStatus::Dispatched);
        assert!(response.dispatch.is_some());
    }
}
