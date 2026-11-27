use std::path::Path;
use crate::extensions::{self, Destination, Resolved};
use std::sync::Arc;

use agent_tool::{AgentToolResult, AgentToolStatus, AGENT_TOOL_PROTOCOL_VERSION};
use buckyos_api::KEventClient;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::adapters::{
    AdapterCallStatus, AdapterReadRequest, AdapterReadResponse, AdapterRegistry,
    AdapterSubscribeEventRequest, AdapterXCallRequest,
};
use crate::config::ObjectRouteConfig;
use crate::error::AgentDIDObjectError;
use crate::event_bridge::EventBridgeManager;
use crate::router::{ObjectRouter, RouteMethod};
use crate::types::{
    apply_line_range, cmd_args_value, EventBridgeSubscription, ObjectRef, ReadInput,
    SubscribeEventInput, UnsubscribeEventInput, XCallInput,
};

pub struct AgentDIDObjectRuntime {
    router: ObjectRouter,
    registry: AdapterRegistry,
    kevent_client: Option<Arc<KEventClient>>,
    event_bridge: EventBridgeManager,
}

impl AgentDIDObjectRuntime {
    pub fn new(config: ObjectRouteConfig) -> Result<Self, AgentDIDObjectError> {
        let registry = AdapterRegistry::with_builtin(&config.adapters)?;
        Ok(Self {
            router: ObjectRouter::new(config),
            registry,
            kevent_client: None,
            event_bridge: EventBridgeManager::new(None),
        })
    }

    pub fn with_registry(
        config: ObjectRouteConfig,
        registry: AdapterRegistry,
    ) -> Result<Self, AgentDIDObjectError> {
        config.validate()?;
        Ok(Self {
            router: ObjectRouter::new(config),
            registry,
            kevent_client: None,
            event_bridge: EventBridgeManager::new(None),
        })
    }

    pub fn with_kevent_client(mut self, client: KEventClient) -> Self {
        let client = Arc::new(client);
        self.kevent_client = Some(client.clone());
        self.event_bridge = EventBridgeManager::new(Some(client));
        self
    }

    pub fn register_adapter(&mut self, adapter: Arc<dyn crate::adapters::AgentObjectAdapter>) {
        self.registry.register(adapter);
    }

    pub async fn read(&self, input: ReadInput) -> Result<AgentToolResult, AgentDIDObjectError> {
        let response = self.read_response(&input).await?;
        Ok(read_response_to_tool_result(&input, response))
    }

    /// What the routed adapter read, before it is rendered as a tool result.
    pub async fn read_response(
        &self,
        input: &ReadInput,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        if let Some(resolved) = extensions::resolve(&input.object, Path::new(".")).await? {
            return self.read_resolved(input, resolved).await;
        }
        self.read_route(input, ObjectRef::parse(&input.object)?)
            .await
    }

    pub(crate) async fn read_resolved(
        &self,
        input: &ReadInput,
        resolved: Resolved,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let mut response = match resolved.target {
            Destination::Local(object) => crate::local_script::read(object, input)?,
            Destination::Route(target) => self.read_route(input, target).await?,
        };
        if let Some(revision) = resolved.revision {
            response.adapt_meta["alias_revision"] = revision.clone();
            response.version = Some(format!(
                "{}@{}",
                response.version.unwrap_or_default(),
                revision
            ));
            response.cache_key = Some(input.object.clone());
            response.object = input.object.clone();
        }
        Ok(response)
    }

    async fn read_route(
        &self,
        input: &ReadInput,
        object_ref: ObjectRef,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let route_match = self.router.route(RouteMethod::Read, &object_ref)?;
        let adapter_config = self
            .router
            .config()
            .adapter(&route_match.route.adapter)
            .ok_or_else(|| AgentDIDObjectError::AdapterNotFound(route_match.route.adapter.clone()))?
            .clone();
        let adapter = self
            .registry
            .get(&route_match.route.adapter)
            .ok_or_else(|| {
                AgentDIDObjectError::AdapterNotFound(route_match.route.adapter.clone())
            })?;

        adapter
            .read(AdapterReadRequest {
                object_ref,
                input: input.clone(),
                route: route_match.route,
                route_trace: route_match.trace,
                adapter_config,
            })
            .await
    }

    pub async fn x_call(&self, input: XCallInput) -> Result<AgentToolResult, AgentDIDObjectError> {
        self.x_call_in(input, Path::new(".")).await
    }

    pub async fn x_call_in(
        &self,
        input: XCallInput,
        cwd: &Path,
    ) -> Result<AgentToolResult, AgentDIDObjectError> {
        let resolved = extensions::resolve(&input.object, cwd).await?;
        let response = match resolved {
            Some(Resolved {
                target: Destination::Local(object),
                context,
                revision,
            }) => {
                let mut response = crate::local_script::invoke(object, &input, context).await?;
                if let Some(revision) = revision {
                    response.detail["alias_revision"] = revision;
                }
                response
            }
            Some(Resolved {
                target: Destination::Route(target),
                revision,
                context,
            }) => {
                let mut response = self.call_route(&input, target, Some(context)).await?;
                response.detail["object"] = json!(input.object);
                response.detail["alias_revision"] = json!(revision);
                response
            }
            None => {
                let target = if buckyos_api::object_extensions::object_alias_name(&input.object) {
                    let path = cwd.join(&input.object);
                    if !path.exists() {
                        return Err(extensions::failure(
                            "UNKNOWN_ALIAS",
                            format!("unknown alias {}", input.object),
                        ));
                    }
                    let url = url::Url::from_file_path(path.canonicalize()?)
                        .map_err(|_| extensions::failure("INVALID_OBJECT_REF", "invalid path"))?;
                    ObjectRef::parse(url.as_str())?
                } else {
                    ObjectRef::parse(&input.object)?
                };
                self.call_route(&input, target, None).await?
            }
        };
        Ok(AgentToolResult {
            agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
            hints: Vec::new(),
            dedup: None,
            tool: None,
            cmd_name: Some("xcall".to_string()),
            status: match response.status {
                AdapterCallStatus::Success => AgentToolStatus::Success,
                AdapterCallStatus::Error => AgentToolStatus::Error,
                AdapterCallStatus::Pending => AgentToolStatus::Pending,
            },
            task_id: None,
            pending_reason: None,
            check_after: None,
            estimated_wait: None,
            title: response
                .title
                .unwrap_or_else(|| format!("xcall `{}`", input.action)),
            summary: response.summary.unwrap_or_else(|| {
                format!(
                    "xcall `{}` on `{}` returned {:?}.",
                    input.action, input.object, response.status
                )
            }),
            details: normalize_detail(response.detail),
            cmd_args: Some(format!(
                "{} {} {}",
                input.object,
                input.action,
                cmd_args_value(&input.params)
            )),
            return_code: Some(if response.status == AdapterCallStatus::Error {
                1
            } else {
                0
            }),
            partial_output: None,
            output: response.output,
            images: Vec::new(),
        })
    }

    async fn call_route(
        &self,
        input: &XCallInput,
        object_ref: ObjectRef,
        alias_context: Option<Value>,
    ) -> Result<crate::adapters::AdapterXCallResponse, AgentDIDObjectError> {
        let route_match = self.router.route(RouteMethod::XCall, &object_ref)?;
        let adapter_config = self
            .router
            .config()
            .adapter(&route_match.route.adapter)
            .ok_or_else(|| AgentDIDObjectError::AdapterNotFound(route_match.route.adapter.clone()))?
            .clone();
        let adapter = self
            .registry
            .get(&route_match.route.adapter)
            .ok_or_else(|| {
                AgentDIDObjectError::AdapterNotFound(route_match.route.adapter.clone())
            })?;

        if alias_context.is_some()
            && input.object == "owner"
            && adapter_config.adapter_type != crate::AdapterType::DidObject
        {
            return Err(extensions::failure(
                "OWNER_IDENTITY_UNVERIFIED",
                "the linked owner requires a verified provider",
            ));
        }
        adapter
            .x_call(AdapterXCallRequest {
                object_ref,
                input: input.clone(),
                alias_context,
                route: route_match.route,
                route_trace: route_match.trace,
                adapter_config,
            })
            .await
    }

    pub async fn subscribe_event(
        &self,
        mut input: SubscribeEventInput,
    ) -> Result<EventBridgeSubscription, AgentDIDObjectError> {
        let (object_ref,revision) = match extensions::resolve(&input.object, Path::new(".")).await? {
            Some(Resolved {
                target: Destination::Route(target),
                revision,..
            }) => (target,revision),
            Some(_) => {
                return Err(extensions::failure(
                    "UNSUPPORTED_EVENT",
                    "local objects do not publish events",
                ))
            }
            None => (ObjectRef::parse(&input.object)?,None),
        };
        let route_match = self
            .router
            .route(RouteMethod::SubscribeEvent, &object_ref)?;
        let adapter_config = self
            .router
            .config()
            .adapter(&route_match.route.adapter)
            .ok_or_else(|| AgentDIDObjectError::AdapterNotFound(route_match.route.adapter.clone()))?
            .clone();
        let adapter = self
            .registry
            .get(&route_match.route.adapter)
            .ok_or_else(|| {
                AgentDIDObjectError::AdapterNotFound(route_match.route.adapter.clone())
            })?;
        input.object = object_ref.normalized.clone();
        let mut receipt = self.event_bridge
            .subscribe(
                adapter,
                AdapterSubscribeEventRequest {
                    object_ref,
                    input,
                    route: route_match.route,
                    route_trace: route_match.trace,
                    adapter_config,
                },
            )
            .await?;
        receipt.alias_revision = revision;
        Ok(receipt)
    }

    pub async fn unsubscribe_event(
        &self,
        input: UnsubscribeEventInput,
    ) -> Result<(), AgentDIDObjectError> {
        self.event_bridge.unsubscribe(&self.registry, input).await
    }
}

pub(crate) fn read_response_to_tool_result(
    input: &ReadInput,
    response: AdapterReadResponse,
) -> AgentToolResult {
    let content_hash = hash_text(response.content.as_deref().unwrap_or_default());
    let mut content = response.content.clone().unwrap_or_default();
    if let Some(range) = &input.range {
        content = apply_line_range(&content, range);
    }
    if let Some(max) = input.max_tokens.map(|tokens| tokens.saturating_mul(4)) {
        let first_line = input.range.as_ref().map_or(1, |r| r.offset.max(1));
        content = limit_read_text(content, max, first_line);
    }

    let output = if input.content_only {
        content
    } else {
        render_read_sections(&content, &response)
    };
    let dedup_key = read_dedup_key(input, &response, &output);

    AgentToolResult {
        agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
        hints: Vec::new(),
        dedup: None,
        tool: None,
        cmd_name: Some("read".to_string()),
        status: AgentToolStatus::Success,
        task_id: None,
        pending_reason: None,
        check_after: None,
        estimated_wait: None,
        title: response
            .meta
            .title
            .clone()
            .unwrap_or_else(|| format!("Read {}", input.object)),
        summary: render_read_summary(&response),
        details: read_details(input, &response, &content_hash),
        cmd_args: Some(format!(
            "{}{}",
            input.object,
            if input.content_only {
                " --content-only"
            } else {
                ""
            }
        )),
        return_code: Some(0),
        partial_output: None,
        output: Some(output),
        images: Vec::new(),
    }
    .with_dedup(dedup_key, format!("`{}` is unchanged", input.object))
}

/// `text` (lines from `first_line` on) within `max` characters, cut at a
/// line boundary when one is near: the marker says how much is left and
/// where the next read starts (prompt mechanics TODO B2).
fn limit_read_text(text: String, max: usize, first_line: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text;
    }
    let mut end = text.char_indices().nth(max).map_or(text.len(), |(i, _)| i);
    let at_line_end = match text[..end].rfind('\n').filter(|nl| *nl > end / 2) {
        Some(nl) => {
            end = nl;
            true
        }
        None => false,
    };
    let shown = &text[..end];
    // A line cut in the middle is read again from its start.
    let next = first_line + shown.matches('\n').count() + usize::from(at_line_end);
    format!(
        "{shown}\n[truncated: {} more characters; continue with offset={next}]",
        total - shown.chars().count()
    )
}

/// What a read showed (prompt mechanics TODO D5): the canonical object
/// (the adapter's `cache_key`), its version, the window (range, properties,
/// content only) and a digest of the text shown. The runtime that
/// dispatches the result answers a repeat with one line.
fn read_dedup_key(input: &ReadInput, response: &AdapterReadResponse, shown: &str) -> String {
    let what = response
        .cache_key
        .as_deref()
        .filter(|k| !k.is_empty())
        .unwrap_or(&response.object);
    let window = json!({
        "range": input.range,
        "properties": input.options.get("properties"),
        "content_only": input.content_only,
    });
    format!(
        "read:{what}#{}:{}:{}",
        hash_text(&window.to_string()).get(..16).unwrap_or_default(),
        response.version.as_deref().unwrap_or(""),
        hash_text(shown).get(..16).unwrap_or_default()
    )
}

fn read_details(input: &ReadInput, response: &AdapterReadResponse, content_hash: &str) -> Value {
    json!({
        "object": input.object,
        "content_only": input.content_only,
        "range": input.range,
        "cache_key": response.cache_key,
        "version": response.version,
        "content_hash": content_hash,
        "route": response.route,
        "meta": response.meta,
    })
}

fn hash_text(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn render_read_sections(content: &str, response: &AdapterReadResponse) -> String {
    let mut sections = Vec::new();
    if !content.is_empty() {
        sections.push(content.to_string());
    }

    let mut meta = Vec::new();
    if let Some(content_type) = &response.meta.content_type {
        meta.push(format!("- content_type: {content_type}"));
    }
    if let Some(size) = response.meta.size {
        meta.push(format!("- size: {size}"));
    }
    if let Some(source) = &response.meta.source {
        meta.push(format!("- source: {source}"));
    }
    if !meta.is_empty() {
        sections.push(format!("Meta:\n{}", meta.join("\n")));
    }

    if !response.prompt_guidance.is_empty() {
        sections.push(format!(
            "Guidance:\n{}",
            response
                .prompt_guidance
                .iter()
                .map(|item| format!("- {}", item.message))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !response.trust_guidance.is_empty() {
        sections.push(format!(
            "Trust:\n{}",
            response
                .trust_guidance
                .iter()
                .map(|item| format!("- {}", item.message))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !response.errors.is_empty() {
        sections.push(format!(
            "Errors:\n{}",
            response
                .errors
                .iter()
                .map(|item| match &item.adapter {
                    Some(adapter) => format!("- {adapter}: {}", item.message),
                    None => format!("- {}", item.message),
                })
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    sections.join("\n\n")
}

fn render_read_summary(response: &AdapterReadResponse) -> String {
    let mut parts = vec![format!(
        "Read `{}` via `{}`.",
        response.object, response.route.adapter
    )];
    if let Some(content_type) = &response.meta.content_type {
        parts.push(format!("content_type={content_type}."));
    }
    if !response.errors.is_empty() {
        parts.push(format!("{} attached error(s).", response.errors.len()));
    }
    parts.join(" ")
}

fn normalize_detail(value: Value) -> Value {
    if value.is_null() {
        json!({})
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde_json::json;

    use crate::adapters::{
        AdapterEventSubscription, AdapterReadRequest, AdapterSubscribeEventRequest,
        AdapterUnsubscribeEventRequest, AdapterXCallRequest, AgentObjectAdapter,
    };
    use crate::config::{AdapterConfig, AdapterType, ObjectRoute};
    use crate::router::RouteMatchType;
    use crate::types::ReadMeta;

    use super::*;

    fn read_input(object: &str, property: Option<&str>) -> ReadInput {
        ReadInput {
            object: object.into(),
            purpose: None,
            session_id: Some("s1".into()),
            content_only: true,
            range: None,
            max_tokens: None,
            options: property.map_or(json!({}), |p| json!({"properties": [p]})),
        }
    }

    fn response(content: &str, version: Option<&str>) -> AdapterReadResponse {
        let mut r = AdapterReadResponse::text(
            "obj://svc/x".into(),
            content.into(),
            crate::router::RouteTrace {
                route_id: "r".into(),
                adapter: "fake".into(),
                method: crate::router::RouteMethod::Read,
            },
        );
        r.version = version.map(str::to_string);
        r
    }

    #[test]
    fn a_cut_read_says_where_the_next_one_starts() {
        let text = (1..=100).map(|i| format!("row {i}")).collect::<Vec<_>>().join("\n");
        let out = limit_read_text(text, 60, 11);
        assert!(out.ends_with("more characters; continue with offset=20]"), "{out}");
        assert!(out.contains("row 9\n[truncated"), "{out}");
    }

    #[test]
    fn a_read_names_what_it_showed_and_the_runtime_answers_a_repeat_in_one_line() {
        let key = |input: &ReadInput, r: AdapterReadResponse| {
            read_response_to_tool_result(input, r).dedup.expect("dedup").key
        };
        let a = key(&read_input("obj://svc/x", None), response("body", Some("1")));
        assert_eq!(a, key(&read_input("obj://svc/x", None), response("body", Some("1"))));
        assert_ne!(a, key(&read_input("obj://svc/x", Some("p")), response("body", Some("1"))));
        assert_ne!(a, key(&read_input("obj://svc/x", None), response("body", Some("2"))));
        assert_ne!(a, key(&read_input("obj://svc/x", None), response("changed", Some("1"))));

        let ledger = agent_tool::MemoryHintLedger::new();
        let mut first = read_response_to_tool_result(&read_input("obj://svc/x", None), response("body", None));
        agent_tool::apply_hints(&mut first, ledger.as_ref(), "c1");
        assert_eq!(first.output.as_deref(), Some("body"));
        let mut again = read_response_to_tool_result(&read_input("obj://svc/x", None), response("body", None));
        agent_tool::apply_hints(&mut again, ledger.as_ref(), "c2");
        assert_eq!(
            again.output.as_deref(),
            Some("`obj://svc/x` is unchanged; same as the result of call c1 above.")
        );
    }

    struct FakeAdapter;

    #[async_trait]
    impl AgentObjectAdapter for FakeAdapter {
        fn id(&self) -> &str {
            "fake"
        }

        async fn read(
            &self,
            req: AdapterReadRequest,
        ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
            Ok(AdapterReadResponse {
                object: req.object_ref.raw,
                object_did: None,
                content: Some("line1\nline2\nline3".to_string()),
                meta: ReadMeta {
                    title: Some("Fake".to_string()),
                    content_type: Some("text/plain".to_string()),
                    size: Some(17),
                    updated_at: None,
                    source: None,
                    extra: json!({}),
                },
                prompt_guidance: vec![],
                trust_guidance: vec![],
                errors: vec![],
                cache_key: None,
                version: None,
                route: req.route_trace,
                adapt_meta: json!({}),
            })
        }

        async fn x_call(
            &self,
            req: AdapterXCallRequest,
        ) -> Result<crate::adapters::AdapterXCallResponse, AgentDIDObjectError> {
            Ok(crate::adapters::AdapterXCallResponse {
                status: AdapterCallStatus::Success,
                output: None,
                detail: json!({"action": req.input.action, "ok": true}),
                title: None,
                summary: None,
                route: req.route_trace,
            })
        }

        async fn subscribe_event(
            &self,
            req: AdapterSubscribeEventRequest,
        ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
            Ok(AdapterEventSubscription {
                subscription: EventBridgeSubscription {
                    alias_revision: None,
                    subscription_id: "remote-sub".to_string(),
                    object: req.object_ref.normalized,
                    object_did: None,
                    event: req.input.event,
                    kevent_pattern: "remote-pattern".to_string(),
                    expires_at: Some("2026-06-07T13:00:00Z".to_string()),
                    cursor: None,
                    route: req.route_trace,
                },
                transport: None,
                unsubscribe_via_adapter: true,
            })
        }

        async fn unsubscribe_event(
            &self,
            _req: AdapterUnsubscribeEventRequest,
        ) -> Result<(), AgentDIDObjectError> {
            Ok(())
        }
    }

    fn runtime() -> AgentDIDObjectRuntime {
        let config = ObjectRouteConfig {
            version: 1,
            adapters: vec![AdapterConfig {
                id: "fake".to_string(),
                adapter_type: AdapterType::Web,
                endpoint: None,
                auth_token_env: None,
                options: json!({}),
            }],
            routes: vec![ObjectRoute {
                id: "obj".to_string(),
                priority: 0,
                match_type: RouteMatchType::Scheme,
                pattern: "obj".to_string(),
                adapter: "fake".to_string(),
                methods: vec![],
                options: json!({}),
            }],
        };
        let mut registry = AdapterRegistry::new();
        registry.register(Arc::new(FakeAdapter));
        AgentDIDObjectRuntime::with_registry(config, registry).unwrap()
    }

    #[tokio::test]
    async fn read_uses_router_and_returns_agent_tool_result() {
        let result = runtime()
            .read(ReadInput {
                object: "obj://example/1".to_string(),
                purpose: None,
                session_id: None,
                content_only: false,
                range: Some(crate::types::ReadLineRange {
                    offset: 2,
                    limit: Some(1),
                }),
                max_tokens: None,
                options: json!({}),
            })
            .await
            .unwrap();
        assert_eq!(result.agent_tool_protocol, "1");
        assert_eq!(result.status, AgentToolStatus::Success);
        assert!(result.output.unwrap().starts_with("line2"));
    }

    #[tokio::test]
    async fn x_call_maps_adapter_response() {
        let result = runtime()
            .x_call(XCallInput {
                object: "obj://example/1".to_string(),
                action: "do".to_string(),
                params: json!({"x": 1}),
                session_id: None,
                idempotency_key: None,
                confirm_token: None,
                trace_id: None,
            })
            .await
            .unwrap();
        assert_eq!(result.details["ok"], true);
        assert_eq!(result.cmd_name.as_deref(), Some("xcall"));
    }

    #[tokio::test]
    async fn subscribe_event_uses_bridge_manager_and_local_pattern() {
        let runtime = runtime();
        let first = runtime
            .subscribe_event(SubscribeEventInput {
                object: "obj://example/1".to_string(),
                event: "changed".to_string(),
                filter: json!({}),
                session_id: None,
                ttl_ms: None,
                cursor: None,
                trace_id: None,
            })
            .await
            .unwrap();
        let second = runtime
            .subscribe_event(SubscribeEventInput {
                object: "obj://example/1".to_string(),
                event: "changed".to_string(),
                filter: json!({}),
                session_id: None,
                ttl_ms: None,
                cursor: None,
                trace_id: None,
            })
            .await
            .unwrap();

        assert_eq!(first.subscription_id, second.subscription_id);
        assert_eq!(first.kevent_pattern, "/obj/example/1/changed");
        assert_ne!(first.kevent_pattern, "remote-pattern");

        runtime
            .unsubscribe_event(UnsubscribeEventInput {
                subscription_id: first.subscription_id.clone(),
            })
            .await
            .unwrap();
        runtime
            .unsubscribe_event(UnsubscribeEventInput {
                subscription_id: first.subscription_id,
            })
            .await
            .unwrap();
    }
}
