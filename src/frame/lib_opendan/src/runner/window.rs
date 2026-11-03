//! The context window a run is given (work context budget TODO §3.1): the
//! session's explicit `context_window`, else the window of the model its
//! alias routes to (AICC `capabilities.max_context_tokens`), else
//! `RunnerOptions.default_context_window`. xllm derives the compaction
//! trigger from it; the mid-run rewrite derives its target line.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde_json::Value;

/// How long a looked-up window is reused for an alias.
pub const WINDOW_CACHE_TTL: Duration = Duration::from_secs(600);

/// Where the window of a model alias comes from.
#[async_trait]
pub trait ContextWindowSource: Send + Sync {
    /// Window (tokens) of the model `model` routes to now; `Ok(None)`:
    /// the model states none.
    async fn context_window(&self, model: &str) -> std::result::Result<Option<u32>, String>;
}

/// A source with a per-alias cache ([`WINDOW_CACHE_TTL`]). Failures are not
/// cached: the next run asks again.
pub struct CachedContextWindows {
    source: Arc<dyn ContextWindowSource>,
    ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, Option<u32>)>>,
}

impl CachedContextWindows {
    pub fn new(source: Arc<dyn ContextWindowSource>) -> Self {
        Self::with_ttl(source, WINDOW_CACHE_TTL)
    }

    pub fn with_ttl(source: Arc<dyn ContextWindowSource>, ttl: Duration) -> Self {
        Self {
            source,
            ttl,
            entries: Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl ContextWindowSource for CachedContextWindows {
    async fn context_window(&self, model: &str) -> std::result::Result<Option<u32>, String> {
        if let Some((at, window)) = self.entries.lock().expect("windows").get(model) {
            if at.elapsed() < self.ttl {
                return Ok(*window);
            }
        }
        let window = self.source.context_window(model).await?;
        self.entries
            .lock()
            .expect("windows")
            .insert(model.to_string(), (Instant::now(), window));
        Ok(window)
    }
}

/// The zone's AICC, as this process's app: the exact model a logical alias
/// routes to (`route.resolve`), then its `capabilities.max_context_tokens`
/// in `models.list`. An exact model (`name@provider`) is looked up directly.
pub struct AiccContextWindows;

#[async_trait]
impl ContextWindowSource for AiccContextWindows {
    async fn context_window(&self, model: &str) -> std::result::Result<Option<u32>, String> {
        let client = buckyos_api::get_buckyos_api_runtime()
            .map_err(|e| e.to_string())?
            .get_aicc_client()
            .await
            .map_err(|e| e.to_string())?;
        let exact = if model.contains('@') {
            model.to_string()
        } else {
            client
                .route_resolve(buckyos_api::RouteResolveRequest::new(
                    buckyos_api::ApiType::Llm,
                    model,
                ))
                .await
                .map_err(|e| format!("route.resolve {model}: {e}"))?
                .selected_exact_model
        };
        let models = client
            .list_models()
            .await
            .map_err(|e| format!("models.list: {e}"))?;
        Ok(max_context_tokens(&models, &exact))
    }
}

/// `capabilities.max_context_tokens` of `exact_model` in a `models.list`
/// value.
pub fn max_context_tokens(models: &Value, exact_model: &str) -> Option<u32> {
    models
        .get("models")
        .and_then(Value::as_array)?
        .iter()
        .find(|m| m.get("exact_model").and_then(Value::as_str) == Some(exact_model))?
        .pointer("/capabilities/max_context_tokens")
        .and_then(Value::as_u64)
        .filter(|n| *n > 0)
        .map(|n| n.min(u64::from(u32::MAX)) as u32)
}
