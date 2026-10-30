// aiworkspace (AI Workspace artifact management service) constants, settings
// schema and a thin kRPC client.
//
// Business results always travel in `result` (`{ ok, ... }` or the three-state
// commit result); the kRPC error channel only carries protocol-level failures.
// See doc/workspace and src/frame/aiworkspace/README.md.

use ::kRPC::kRPC;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const AIWORKSPACE_UNIQUE_ID: &str = "aiworkspace";
pub const AIWORKSPACE_SERVICE_NAME: &str = "aiworkspace";
pub const AIWORKSPACE_SERVICE_PORT: u16 = 4120;
pub const AIWORKSPACE_HTTP_PATH: &str = "/kapi/aiworkspace";
/// The gateway `app_info` key every Workspace host folds onto (AI Workspace 内核架构改进 D21): the scheduler
/// registers it, `boot_gateway.yaml` folds `aiws-<label>.<zone>` onto it, the Control Panel resolves SSO
/// redirects to it. The label is the workspace id without its `ws_` prefix (a DNS label carries no `_`).
pub const AIWORKSPACE_GATEWAY_APP_KEY: &str = "aiws";
pub const AIWORKSPACE_HOST_PREFIX: &str = "aiws-";

/// Whether a host label (`<label>.<zone>`) is a Workspace's own origin: `aiws-` followed by `[a-z0-9]+`, the
/// same rule the gateway applies.
pub fn is_aiworkspace_host_label(label: &str) -> bool {
    label
        .strip_prefix(AIWORKSPACE_HOST_PREFIX)
        .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()))
}
/// App pool of the service's `app_data` and `temp` buckets per user (D17).
pub const AIWORKSPACE_STORAGE_QUOTA_MIB: u64 = 16_384;

/// The per-user buckets the service declares in `services/aiworkspace/spec` (doc/workspace
/// `BuckyOS AiWorkspace Durable Data Schema.md`): `assets` (uploaded and generated files),
/// `objects` (one retention DirObject per snapshot / generated version), `exports` (packages,
/// replicas, fixed publications and uploaded packages; 7 days).
/// The Object URL of an AI Workspace Reference (D10 → doc/key url.md §6): the
/// default `obj://aiworkspace/<ws>/entities/<entity_id>` for a live reference
/// without selector; `mode=fixed`, `selector` and `version` go into the query as
/// canonical JSON, so one Reference has exactly one URL. `zone` stays out of the
/// current-zone form.
pub fn aiws_reference_url(reference: &Value, workspace_id: Option<&str>) -> Result<String, String> {
    let ws = reference
        .get("workspace_id")
        .and_then(Value::as_str)
        .or(workspace_id)
        .ok_or("reference names no workspace")?;
    let entity = reference
        .get("entity_id")
        .and_then(Value::as_str)
        .ok_or("reference names no entity_id")?;
    let mut url = crate::obj_service_url("aiworkspace", &[ws, "entities", entity]);
    let mut query = Vec::new();
    if reference.get("mode").and_then(Value::as_str) == Some("fixed") {
        query.push(("mode", "fixed".to_string()));
    }
    for key in ["selector", "version"] {
        if let Some(value) = reference.get(key).filter(|v| !v.is_null()) {
            query.push((key, crate::canonical_json(value)));
        }
    }
    if !query.is_empty() {
        url.push('?');
        url.push_str(
            &query
                .iter()
                .map(|(k, v)| format!("{k}={}", crate::encode_obj_query_component(v)))
                .collect::<Vec<_>>()
                .join("&"),
        );
    }
    crate::parse_object_url(&url)
        .map(|u| u.to_string())
        .map_err(|e| e.to_string())
}

/// The Reference an `obj://aiworkspace/<ws>/entities/<entity_id>[?...]` URL projects.
pub fn parse_aiws_reference_url(url: &str) -> Result<Value, String> {
    let parsed = crate::parse_object_url(url).map_err(|e| e.to_string())?;
    let segments = parsed.obj_segments();
    let (crate::ObjectUrl::Obj { host, query, .. }, [ws, entities, entity]) = (&parsed, segments.as_slice()) else {
        return Err(format!("{url} is not an AI Workspace entity url"));
    };
    if host.as_str() != "aiworkspace" || entities != "entities" {
        return Err(format!("{url} is not an AI Workspace entity url"));
    }
    let mut reference = json!({
        "workspace_id": ws,
        "entity_id": entity,
        "mode": query.get("mode").map(String::as_str).unwrap_or("live"),
    });
    for key in ["selector", "version"] {
        if let Some(value) = query.get(key) {
            reference[key] = serde_json::from_str(value).map_err(|e| format!("{key}: {e}"))?;
        }
    }
    Ok(reference)
}

pub fn aiworkspace_bucket_decls() -> HashMap<String, bucket_store::BucketDecl> {
    serde_json::from_value(json!({
        "assets": { "kind": "data", "data_policy": "app_data", "scope": "user" },
        "objects": { "kind": "data", "data_policy": "app_data", "scope": "user" },
        "exports": { "kind": "data", "data_policy": "temp", "scope": "user", "default_ttl_days": 7 },
        "library": { "kind": "data", "data_policy": "app_data", "scope": "user" }
    }))
    .expect("aiworkspace bucket declarations")
}

/// `services/aiworkspace/settings`. Seeded by the scheduler boot builder with
/// defaults (insert_json_if_absent, so user edits survive re-boot).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiWorkspaceSettings {
    /// Upper bound of one Commit request body, in bytes.
    pub max_commit_bytes: u64,
    /// Upper bound of operations in one Commit.
    pub max_ops: u64,
    /// Upper bound of `doc.wait_changes` long polling, in milliseconds.
    pub max_wait_ms: u64,
    /// Upper bound of one uploaded asset, in bytes (single chunk in this phase).
    pub max_asset_bytes: u64,
    /// Wish runs (xllm executor, program runner).
    pub wish: AiWorkspaceWishSettings,
    /// Logical bytes one Workspace may occupy (内核架构改进 D19); `None` = no quota.
    pub quota_bytes: Option<u64>,
    /// Days a resource only the history references is kept before the collector may reclaim it (D06).
    pub history_retention_days: u64,
    /// Bytes a Workspace's Sources may fetch over HTTP (内核架构改进 D24); `None` = unlimited.
    pub traffic_quota_bytes: Option<u64>,
}

/// Wish execution settings (doc/workspace 许愿格详细设计 §13.3). Models are AICC selectors (logical
/// names such as `llm.code`, or exact `model@provider`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiWorkspaceWishSettings {
    pub analyze_model: String,
    pub execute_model: String,
    /// Model of `aiws.llm.map` (per-item judgements inside programs).
    pub map_model: String,
    /// Deno used to run wish programs; empty = `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno`, then `PATH`.
    pub deno: String,
    pub analyze_tool_iterations: u32,
    pub execute_tool_iterations: u32,
    /// Output limit of one model turn (thinking included). Some providers (Claude) require one; AICC
    /// routes only to models whose output limit reaches it.
    pub max_output_tokens: u32,
    /// Wall clock of one stage, seconds.
    pub stage_timeout_secs: u64,
    pub program_timeout_secs: u64,
    pub program_memory_mb: u32,
    pub llm_map_max_items: u32,
    /// Where a wish runs when the request names no mode (内核架构改进 D14): `service` (the transitional
    /// in-service executor, until T3-9), `local` (the browser) or `hosted` (an Agent Work Session).
    pub default_execution: String,
    /// The transitional in-service executor may still be chosen (`false` once hosted execution passed DV, T3-9).
    pub allow_service_execution: bool,
}

impl Default for AiWorkspaceWishSettings {
    fn default() -> Self {
        AiWorkspaceWishSettings {
            analyze_model: "llm.plan".into(),
            execute_model: "llm.code".into(),
            map_model: "llm.chat".into(),
            deno: String::new(),
            analyze_tool_iterations: 30,
            execute_tool_iterations: 60,
            max_output_tokens: 32_000,
            stage_timeout_secs: 1800,
            program_timeout_secs: 120,
            program_memory_mb: 1024,
            llm_map_max_items: 2000,
            default_execution: "service".into(),
            allow_service_execution: true,
        }
    }
}

impl Default for AiWorkspaceSettings {
    fn default() -> Self {
        AiWorkspaceSettings {
            max_commit_bytes: 8 * 1024 * 1024,
            max_ops: 10_000,
            max_wait_ms: 30_000,
            max_asset_bytes: 32 * 1024 * 1024,
            wish: AiWorkspaceWishSettings::default(),
            quota_bytes: None,
            history_retention_days: 30,
            traffic_quota_bytes: None,
        }
    }
}

/// kRPC client for callers without a UI (agents, other services, tests).
pub struct AiWorkspaceClient {
    client: kRPC,
}

impl AiWorkspaceClient {
    pub fn new(client: kRPC) -> Self {
        Self { client }
    }

    /// Call `method` and return the `result` object as is. Callers branch on
    /// `ok` / `status` / `error.code`, never on error text.
    pub async fn call(
        &self,
        method: &str,
        params: Value,
    ) -> std::result::Result<Value, ::kRPC::RPCErrors> {
        self.client.call(method, params).await
    }
}

#[cfg(test)]
mod reference_url_tests {
    use super::*;

    #[test]
    fn a_reference_has_one_url() {
        let live = json!({ "workspace_id": "ws_1", "entity_id": "e_1", "mode": "live" });
        assert_eq!(aiws_reference_url(&live, None).unwrap(), "obj://aiworkspace/ws_1/entities/e_1");
        let fixed = json!({ "entity_id": "e_1", "mode": "fixed", "selector": { "b": [1, 2], "a": "x" }, "version": { "seq": 3, "epoch": "ep" } });
        let url = aiws_reference_url(&fixed, Some("ws_1")).unwrap();
        assert_eq!(url, "obj://aiworkspace/ws_1/entities/e_1?mode=fixed&selector=%7B%22a%22:%22x%22,%22b%22:%5B1,2%5D%7D&version=%7B%22epoch%22:%22ep%22,%22seq%22:3%7D");
        let back = parse_aiws_reference_url(&url).unwrap();
        assert_eq!(back["selector"], fixed["selector"]);
        assert_eq!(back["version"], fixed["version"]);
        assert_eq!(aiws_reference_url(&back, None).unwrap(), url);
        assert!(parse_aiws_reference_url("obj://aiworkspace/ws_1/path/a").is_err());
        let tricky = json!({ "entity_id": "e_1", "mode": "live", "selector": { "q": "a&b=c" } });
        let url = aiws_reference_url(&tricky, Some("ws_1")).unwrap();
        assert_eq!(parse_aiws_reference_url(&url).unwrap()["selector"], tricky["selector"]);
    }
}
