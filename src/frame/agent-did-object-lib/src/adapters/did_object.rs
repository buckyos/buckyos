use std::sync::Arc;

use agent_tool::prompt_engine::value_text;
use agent_tool::text_budget::clip_to_default_budget;
use async_trait::async_trait;
use buckyos_api::ObjectUrl;
use name_client::{DIDObjectClient, ResolvedDIDObject};
use name_lib::{ActionInvocation, ActionObserved, ActionResponse, EventSubscribeRequest};
use serde_json::{json, Value};

use super::{
    AdapterCallStatus, AdapterEventSubscription, AdapterEventTransport, AdapterReadRequest,
    AdapterReadResponse, AdapterSubscribeEventRequest, AdapterUnsubscribeEventRequest,
    AdapterXCallRequest, AdapterXCallResponse, AgentObjectAdapter,
};
use super::web::WebAdapter;
use crate::error::AgentDIDObjectError;
use crate::zone::{client_for, RuntimeZoneAccess, ZoneAccess};
use crate::types::{
    PromptGuidance, ReadAttachedError, ReadMeta,
    TrustGuidance,
};

pub struct DidObjectProtocolAdapter {
    id: String,
    zone: Arc<dyn ZoneAccess>,
}

impl DidObjectProtocolAdapter {
    pub fn new(id: String) -> Self {
        Self::with_zone(id, Arc::new(RuntimeZoneAccess))
    }

    pub fn with_zone(id: String, zone: Arc<dyn ZoneAccess>) -> Self {
        Self { id, zone }
    }

    /// The canonical URL of a reference and the endpoint its Card is found at: an `obj://` or `cyfs:///` object
    /// through this process's gateway (TODO §1.4), an https / http URL as it is.
    fn locate(&self, object_ref: &crate::types::ObjectRef) -> Result<(ObjectUrl, String), AgentDIDObjectError> {
        let url = buckyos_api::parse_object_url(&object_ref.normalized)
            .map_err(|err| AgentDIDObjectError::UnsupportedObjectRef(err.to_string()))?;
        if let ObjectUrl::Web { url: web } = &url {
            return Ok((url.clone(), self.zone_web_endpoint(web).unwrap_or_else(|| web.clone())));
        }
        let origin = self.zone.gateway_origin().ok_or_else(|| {
            AgentDIDObjectError::AdapterUnavailable(format!(
                "{url}: this process has no BuckyOS runtime (it is not in a Zone, or its session could not be established), so there is no gateway to its Provider"
            ))
        })?;
        let endpoint = buckyos_api::object_endpoint(&url, &origin, self.zone.zone_host().as_deref())
            .ok_or_else(|| AgentDIDObjectError::UnsupportedObjectRef(format!("{url} has no DID Object endpoint")))?;
        Ok((url, endpoint))
    }

    /// An https URL of this Zone's host, reached through this process's gateway (no TLS hairpin to the Zone's
    /// public address).
    fn zone_web_endpoint(&self, web: &str) -> Option<String> {
        let parsed = url::Url::parse(web).ok()?;
        let zone_host = self.zone.zone_host()?;
        if parsed.scheme() != "https"
            || !parsed.host_str()?.eq_ignore_ascii_case(&zone_host)
            || !crate::zone::is_zone_url(self.zone.as_ref(), web)
        {
            return None;
        }
        let origin = self.zone.gateway_origin()?;
        let query = parsed.query().map(|q| format!("?{q}")).unwrap_or_default();
        Some(format!(
            "{}{}{query}",
            origin.trim_end_matches('/'),
            parsed.path()
        ))
    }

    /// The client for `endpoint` (with the caller's session on the Zone's own origins) and the object it resolves.
    async fn resolve(
        &self,
        url: &ObjectUrl,
        endpoint: &str,
    ) -> Result<
        (DIDObjectClient, reqwest::Client, ResolvedDIDObject),
        (reqwest::Client, AgentDIDObjectError),
    > {
        let http = client_for(self.zone.as_ref(), endpoint).await;
        let client = DIDObjectClient::with_http_client(http.clone());
        let discovery_error = |err: name_lib::NSError| match err {
            name_lib::NSError::Forbid => {
                crate::extensions::failure("PERMISSION_DENIED", "provider denied discovery")
            }
            name_lib::NSError::Failed(message) if message.contains("request returned HTTP 401") => {
                crate::extensions::failure("PERMISSION_DENIED", "provider rejected the session")
            }
            name_lib::NSError::Disabled(_) => {
                crate::extensions::failure("RESOURCE_DISABLED", "provider object is disabled")
            }
            _ => AgentDIDObjectError::ResolveError(format!("{url}: provider discovery failed")),
        };
        let result = async {
            let card = client
                .resolve_card(endpoint)
                .await
                .map_err(|err| match err {
                    name_lib::NSError::NotFound(_) => crate::extensions::failure(
                        "OBJECT_NOT_FOUND",
                        format!("{url}: no DID Object Card"),
                    ),
                    other => discovery_error(other),
                })?;
            let profile_endpoint = name_lib::resolve_href(
                card.service_endpoint().map_err(discovery_error)?,
                card.profile().map_err(discovery_error)?,
            );
            let profile_http = self.form_client(endpoint, &profile_endpoint).await?;
            let profile = DIDObjectClient::with_http_client(profile_http)
                .resolve_profile_from_card(&card)
                .await
                .map_err(discovery_error)?;
            Ok(ResolvedDIDObject {
                object_url: endpoint.into(),
                object_card: card,
                object_profile: profile,
            })
        }
        .await;
        match result {
            Ok(resolved) => Ok((client, http, resolved)),
            Err(err) => Err((http, err)),
        }
    }

    async fn form_client(
        &self,
        source: &str,
        target: &str,
    ) -> Result<reqwest::Client, AgentDIDObjectError> {
        let parse = |value: &str| {
            url::Url::parse(value)
                .map_err(|_| AgentDIDObjectError::ProtocolError("invalid object form URL".into()))
        };
        let source_url = parse(source)?;
        let target_url = parse(target)?;
        let zone = self.zone.as_ref();
        let target_in_zone = crate::zone::is_zone_url(zone, target);
        if !crate::zone::is_zone_url(zone, source) && target_in_zone {
            return Err(crate::extensions::failure(
                "PERMISSION_DENIED",
                "external profile cannot delegate to the caller's Zone",
            ));
        }
        let same_origin = source_url.origin() == target_url.origin();
        if !target_url.username().is_empty()
            || target_url.password().is_some()
            || !(target_url.scheme() == "https"
                || (target_url.scheme() == "http" && (same_origin || target_in_zone)))
        {
            return Err(crate::extensions::failure(
                "PERMISSION_DENIED",
                "object form requires a secure origin",
            ));
        }
        Ok(client_for(zone, target).await)
    }

    async fn invoke_declared(
        &self,
        endpoint: &str,
        resolved: &ResolvedDIDObject,
        input: &crate::XCallInput,
        effect: Option<&str>,
    ) -> Result<ActionResponse, AgentDIDObjectError> {
        let form = DIDObjectClient::action_endpoint(
            &resolved.object_card,
            &resolved.object_profile,
            &input.action,
        )
        .map_err(|e| AgentDIDObjectError::ProtocolError(e.to_string()))?;
        let invocation = ActionInvocation {
            method: input.action.clone(),
            params: input.params.clone(),
            obj: resolved.object_card.object_url().map(str::to_owned),
            obj_did: Some(resolved.object_card.id.clone()),
            observed: Some(ActionObserved {
                profile: resolved.object_card.profile().ok().map(str::to_owned),
                ..Default::default()
            }),
            idempotency_key: input.idempotency_key.clone(),
            confirm_token: input.confirm_token.clone(),
            trace_id: input.trace_id.clone(),
        };
        let http = self.form_client(endpoint, &form).await?;
        let unknown = || crate::AgentDIDObjectError::Business {
            code: "WRITE_OUTCOME_UNKNOWN".into(),
            message: "the provider did not confirm this action".into(),
            detail: json!({"idempotency_key":input.idempotency_key,"trace_id":input.trace_id}),
        };
        let response = http
            .post(&form)
            .json(&invocation)
            .send()
            .await
            .map_err(|_| {
                if matches!(effect, Some("read" | "pure")) {
                    crate::extensions::failure("PROVIDER_UNAVAILABLE", "cannot reach provider")
                } else {
                    unknown()
                }
            })?;
        let status = response.status();
        let response: ActionResponse = match response.json().await {
            Ok(response) => response,
            Err(_) if status.as_u16() == 403 || status.as_u16() == 401 => {
                return Err(crate::extensions::failure(
                    "PERMISSION_DENIED",
                    "provider rejected the caller",
                ))
            }
            Err(_) if matches!(effect, Some("read" | "pure")) => {
                return Err(crate::extensions::failure(
                    "PROVIDER_UNAVAILABLE",
                    "invalid provider response",
                ))
            }
            Err(_) => return Err(unknown()),
        };
        if !status.is_success() && response.error.is_none() || response.validate().is_err() {
            return Err(if matches!(effect, Some("read" | "pure")) {
                crate::extensions::failure("PROTOCOL_ERROR", "invalid provider receipt")
            } else {
                unknown()
            });
        }
        Ok(response)
    }

    /// Discover once under the caller's credentials, then use that exact profile's action form.
    pub(crate) async fn invoke_resource(
        &self,
        slot: &str,
        input: &crate::XCallInput,
    ) -> Result<ActionResponse, AgentDIDObjectError> {
        let target = crate::ObjectRef::parse(&input.object)?;
        let (url, endpoint) = self.locate(&target)?;
        let (_, _, resolved) = self.resolve(&url, &endpoint).await.map_err(|(_, e)| e)?;
        if !buckyos_api::owner_resource::matches_slot(&resolved.object_profile, slot) {
            return Err(crate::extensions::failure("PROVIDER_TYPE_MISMATCH", "current provider profile does not declare this personal resource with a read-only list"));
        }
        let effect = buckyos_api::owner_resource::action_effect(
            &resolved.object_profile,
            &input.action,
        )
        .ok_or_else(|| {
            crate::extensions::failure(
                "UNSUPPORTED_ACTION",
                "current provider profile does not declare this action with the required effect",
            )
        })?;
        self.invoke_declared(&endpoint, &resolved, input, Some(effect))
            .await
    }

    pub(crate) async fn recover_write(
        &self,
        slot: &str,
        input: &crate::XCallInput,
    ) -> Result<Value, AgentDIDObjectError> {
        let unknown = || AgentDIDObjectError::Business {
            code: "WRITE_OUTCOME_UNKNOWN".into(),
            message: "the original provider could not confirm this operation".into(),
            detail: json!({"idempotency_key":input.idempotency_key}),
        };
        let target = crate::ObjectRef::parse(&input.object)?;
        let (url, endpoint) = self.locate(&target)?;
        let (_, _, resolved) =
            self.resolve(&url, &endpoint)
                .await
                .map_err(|(_, err)| match err {
                    AgentDIDObjectError::Business { ref code, .. }
                        if code == "PERMISSION_DENIED" || code == "IDENTITY_UNAVAILABLE" => err,
                    _ => unknown(),
                })?;
        if !buckyos_api::owner_resource::matches_slot(&resolved.object_profile, slot)
            || buckyos_api::owner_resource::action_effect(
                &resolved.object_profile,
                "get_submission",
            )
            .is_none()
        {
            return Err(unknown());
        }
        let mut query = input.clone();
        query.action = "get_submission".into();
        query.params = json!({"action":input.action,"idempotency_key":input.idempotency_key});
        let receipt = self
            .invoke_declared(&endpoint, &resolved, &query, Some("read"))
            .await
            .map_err(|e| match e {
                AgentDIDObjectError::Business {
                    code,
                    message,
                    detail,
                } if code == "PERMISSION_DENIED" => AgentDIDObjectError::Business {
                    code,
                    message,
                    detail,
                },
                _ => unknown(),
            })?;
        if let Some(e) = receipt.error {
            if matches!(
                e.code.to_ascii_lowercase().as_str(),
                "permission_denied" | "access_denied" | "unauthorized"
            ) {
                return Err(crate::extensions::failure(
                    "PERMISSION_DENIED",
                    "provider refused the submission query",
                ));
            }
            return Err(unknown());
        }
        let status = receipt.result.ok_or_else(unknown)?;
        if status["status"] != "accepted" {
            return Err(unknown());
        }
        status
            .get("result")
            .filter(|v| v.is_object())
            .cloned()
            .ok_or_else(unknown)
    }

}

#[async_trait]
impl AgentObjectAdapter for DidObjectProtocolAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let (url, endpoint) = self.locate(&req.object_ref)?;
        let (_client, resolved) = match self.resolve(&url, &endpoint).await {
            Ok((client, _, resolved)) => (client, resolved),
            // An https page that is no DID Object: read it as web content.
            Err((http, AgentDIDObjectError::Business {code,..})) if code=="OBJECT_NOT_FOUND" && matches!(url, ObjectUrl::Web { .. }) && !buckyos_api::object_extensions::object_alias_name(&req.input.object) => {
                let canonical = url.to_string();
                let mut web_req = req;
                web_req.object_ref = crate::types::ObjectRef::parse(&endpoint)?;
                let mut response = WebAdapter::with_client(self.id.clone(), http).read(web_req).await?;
                response.object = canonical.clone();
                response.cache_key = Some(canonical);
                return Ok(response);
            }
            Err((_, err)) => return Err(err),
        };
        let profile = &resolved.object_profile;
        let card = &resolved.object_card;
        let profile_doc = serde_json::to_value(profile).unwrap_or(Value::Null);
        let canonical = url.to_string();

        let requested: Vec<String> = req
            .input
            .options
            .get("properties")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let explicit = !requested.is_empty();
        let names = if explicit { requested } else { summary_props(&profile_doc) };

        let mut errors = Vec::new();
        let mut property_values = serde_json::Map::new();
        for property in &names {
            if !DIDObjectClient::has_property(profile, property) {
                errors.push(ReadAttachedError {
                    adapter: Some(self.id.clone()),
                    message: format!("property {property} is not declared by {canonical}"),
                });
                continue;
            }
            let property_endpoint = DIDObjectClient::property_endpoint(card, profile, property)
                .map_err(|e| AgentDIDObjectError::ProtocolError(e.to_string()))?;
            let client = DIDObjectClient::with_http_client(
                self.form_client(&endpoint, &property_endpoint).await?,
            );
            match client.read_property_from_resolved(&resolved, property).await {
                Ok(value) => {
                    property_values.insert(property.clone(), value);
                }
                Err(err) => errors.push(ReadAttachedError {
                    adapter: Some(self.id.clone()),
                    message: format!("property {property} read failed: {err}"),
                }),
            }
        }
        if explicit && property_values.is_empty() {
            let message = errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>().join("; ");
            return Err(AgentDIDObjectError::DeclaredCapabilityNotFound(message));
        }

        let content = if explicit {
            let text = property_values
                .values()
                .map(|value| match value {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                })
                .collect::<Vec<_>>()
                .join("\n");
            clip_to_default_budget(&text, None).0
        } else {
            render_object_document(&canonical, &endpoint, card, &profile_doc, &property_values)
        };

        Ok(AdapterReadResponse {
            object: canonical.clone(),
            object_did: Some(did_to_string(&card.id)),
            content: Some(content),
            meta: ReadMeta {
                title: profile.title.clone(),
                content_type: Some(if explicit { "application/json" } else { "application/did-object-profile+json" }.to_string()),
                size: None,
                updated_at: None,
                source: Some(resolved.object_url.clone()),
                extra: json!({
                    "profile_id": profile.id,
                    "traits": profile.traits,
                }),
            },
            prompt_guidance: vec![PromptGuidance {
                message: "Only declared DID Object properties/actions/events are callable. Read one property with read uri=<object> property=<name>.".to_string(),
            }],
            trust_guidance: vec![TrustGuidance {
                message: "DID Object card and profile passed client-side structural validation; provider must still enforce auth and policy.".to_string(),
            }],
            errors,
            cache_key: Some(canonical.clone()),
            // A card is issued per request: its `iat` is no version of the content.
            version: None,
            route: req.route_trace,
            adapt_meta: json!({
                "object_url": canonical,
                "endpoint": resolved.object_url,
                "object_did": did_to_string(&card.id),
                "profile_id": profile.id,
                "property_values": property_values,
            }),
        })
    }

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
        let (url, endpoint) = self.locate(&req.object_ref)?;
        let (_, _, resolved) = self
            .resolve(&url, &endpoint)
            .await
            .map_err(|(_, err)| err)?;
        if !DIDObjectClient::has_action(&resolved.object_profile, &req.input.action) {
            return Err(AgentDIDObjectError::DeclaredCapabilityNotFound(format!(
                "action {} is not declared by {}",
                req.input.action, resolved.object_url
            )));
        }
        let declaration = &resolved.object_profile.actions[&req.input.action];
        let policy = declaration.buckyos_action.as_ref();
        let effect = policy.and_then(|p| p.effect.as_deref());
        if let Some(context) = &req.alias_context {
            if context["read_only"] == true && !matches!(effect, Some("read" | "pure")) {
                return Err(crate::extensions::failure(
                    "PERMISSION_DENIED",
                    "this action requires a writable session",
                ));
            }
            if req.input.object == "owner" {
                let owner = context["owner_did"].as_str().unwrap_or("");
                let published =
                    context["person_ref"].as_str() == Some(req.object_ref.normalized.as_str());
                let card_did = &resolved.object_card.id;
                let verified = card_did.to_string() == owner
                    && url::Url::parse(&req.object_ref.normalized).is_ok_and(|url| {
                        (url.scheme() == "obj" || (url.scheme() == "https" && url.port().is_none()))
                            && url.host_str().is_some_and(|host| {
                                host.eq_ignore_ascii_case(&card_did.to_raw_host_name())
                                    || host.eq_ignore_ascii_case(&card_did.to_host_name())
                            })
                    });
                if owner.is_empty() || !(published || verified) {
                    return Err(crate::extensions::failure(
                        "OWNER_IDENTITY_UNVERIFIED",
                        "the linked personal object does not verify the current owner",
                    ));
                }
            }
        }
        let response = self.invoke_declared(&endpoint, &resolved, &req.input, effect).await?;
        let mut response =
            action_response_to_adapter_response(&req.input.action, response, req.route_trace);
        if let Value::Object(detail) = &mut response.detail {
            detail.insert("object".into(), json!(url.to_string()));
            detail.insert("effect".into(), json!(effect));
        }
        Ok(response)
    }

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
        let (url, endpoint) = self.locate(&req.object_ref)?;
        let (_, _, resolved) = self.resolve(&url, &endpoint).await.map_err(|(_, err)| err)?;
        if !DIDObjectClient::has_event(&resolved.object_profile, &req.input.event) {
            return Err(AgentDIDObjectError::DeclaredCapabilityNotFound(format!(
                "event {} is not declared by {}",
                req.input.event, resolved.object_url
            )));
        }
        let endpoint = DIDObjectClient::event_endpoint(
            &resolved.object_card,
            &resolved.object_profile,
            &req.input.event,
        )
        .map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string()))?;
        let subscribe = EventSubscribeRequest {
            op: "subscribe".to_string(),
            object: resolved.object_url.clone(),
            object_did: Some(resolved.object_card.id.clone()),
            event: req.input.event.clone(),
            filter: req.input.filter.clone(),
            ttl_ms: req.input.ttl_ms,
            cursor: req.input.cursor.clone(),
            trace_id: req.input.trace_id.clone(),
        };
        Ok(AdapterEventSubscription {
            subscription: crate::types::EventBridgeSubscription {
                alias_revision: None,
                subscription_id: "pending_remote_subscription".to_string(),
                object: resolved.object_url.clone(),
                object_did: Some(did_to_string(&resolved.object_card.id)),
                event: req.input.event.clone(),
                kevent_pattern: crate::event_bridge::encode_object_event_id(
                    &resolved.object_url,
                    &req.input.event,
                ),
                expires_at: None,
                cursor: req.input.cursor.clone(),
                route: req.route_trace,
            },
            transport: Some(AdapterEventTransport::WebSocket {
                endpoint,
                subscribe: serde_json::to_value(subscribe)
                    .map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string()))?,
                unsubscribe: None,
            }),
            unsubscribe_via_adapter: false,
        })
    }

    async fn unsubscribe_event(
        &self,
        _req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError> {
        Ok(())
    }
}

pub fn action_response_to_adapter_response(
    action: &str,
    response: ActionResponse,
    route: crate::router::RouteTrace,
) -> AdapterXCallResponse {
    if let Some(error) = response.error {
        return AdapterXCallResponse {
            status: AdapterCallStatus::Error,
            output: Some(json!(&error).to_string()),
            detail: json!({
                "error": error,
                "meta": response.meta,
            }),
            title: format!("xcall `{action}` failed").into(),
            summary: error_summary(action),
            route,
        };
    }

    let result = response.result.unwrap_or(Value::Null);
    AdapterXCallResponse {
        status: AdapterCallStatus::Success,
        output: Some(result.to_string()),
        detail: json!({
            "result": result,
            "meta": response.meta,
        }),
        title: Some(format!("xcall `{action}` succeeded")),
        summary: Some(format!("Action `{action}` completed successfully.")),
        route,
    }
}

fn error_summary(action: &str) -> Option<String> {
    Some(format!("Action `{action}` returned a provider error."))
}

/// `x-buckyos:summary_props` of a Profile: the properties a description reads.
fn summary_props(profile: &Value) -> Vec<String> {
    profile
        .get("x-buckyos:summary_props")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

/// The object document `read` returns: identity, what the Profile declares, and the summary properties' values.
fn render_object_document(
    canonical: &str,
    endpoint: &str,
    card: &name_lib::DIDObjectCard,
    profile: &Value,
    summary: &serde_json::Map<String, Value>,
) -> String {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut lines = vec![format!("Object: {canonical}"), format!("Object DID: {}", did_to_string(&card.id))];
    // Other names of the object, not the transport endpoints the Card was reached at.
    let service = card.service_endpoint().ok().map(str::to_string);
    let aliases: Vec<String> = card
        .also_known_as
        .iter()
        .filter(|aka| aka.as_str() != canonical && aka.as_str() != endpoint && Some(aka.as_str()) != service.as_deref())
        .cloned()
        .collect();
    if !aliases.is_empty() {
        lines.push(format!("Also known as: {}", aliases.join(", ")));
    }
    for (label, key) in [("Title", "title"), ("Description", "description")] {
        let value = text(profile, key);
        if !value.is_empty() {
            lines.push(format!("{label}: {value}"));
        }
    }
    for (section, key) in [("Properties", "properties"), ("Actions", "actions"), ("Events", "events")] {
        let Some(entries) = profile.get(key).and_then(Value::as_object).filter(|e| !e.is_empty()) else {
            continue;
        };
        lines.push(format!("{section}:"));
        let mut names: Vec<&String> = entries.keys().collect();
        names.sort();
        for name in names {
            let entry = &entries[name];
            let mut line = format!("- {name}");
            let description = text(entry, "description");
            if !description.is_empty() {
                line.push_str(&format!(": {description}"));
            }
            let risk = text(&entry["x-buckyos:action"], "effect");
            if !risk.is_empty() {
                line.push_str(&format!(" (effect: {risk})"));
            }
            lines.push(line);
        }
    }
    if !summary.is_empty() {
        lines.push("Summary:".to_string());
        for (name, value) in summary {
            let text = summary_text(value);
            if text.contains('\n') {
                lines.push(format!("- {name}:"));
                lines.extend(text.lines().map(|l| format!("  {l}")));
            } else {
                lines.push(format!("- {name}: {text}"));
            }
        }
    }
    lines.join("\n")
}

/// A summary property's value as the document shows it: a collection page
/// one line per row (`- <object> — <cell>: <value> · …`), anything else by
/// the rule of template values (its readable fields, else compact JSON).
fn summary_text(value: &Value) -> String {
    let Some(rows) = value.get("rows").and_then(Value::as_array) else {
        return value_text(value).0;
    };
    let mut lines: Vec<String> = rows.iter().map(row_line).collect();
    if let Some(next) = value.get("next_cursor").filter(|c| !c.is_null()) {
        lines.push(format!("next_cursor: {}", cell_text(next).unwrap_or_default()));
    }
    if lines.is_empty() {
        return "(none)".to_string();
    }
    clip_to_default_budget(&lines.join("\n"), None).0
}

/// One row of a collection: the object and the cells its Provider chose.
fn row_line(row: &Value) -> String {
    let object = row
        .get("object")
        .or_else(|| row.get("row_id"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let cells: Vec<String> = row
        .get("cells")
        .and_then(Value::as_object)
        .map(|cells| {
            cells
                .iter()
                .filter_map(|(k, v)| cell_text(v).map(|t| format!("{k}: {t}")))
                .collect()
        })
        .unwrap_or_default();
    if cells.is_empty() {
        format!("- {object}")
    } else {
        format!("- {object} — {}", cells.join(" · "))
    }
}

const CELL_CHARS: usize = 160;

/// A cell on one line, at most [`CELL_CHARS`] characters.
fn cell_text(v: &Value) -> Option<String> {
    let text = match v {
        Value::Null => return None,
        Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        other => other.to_string(),
    };
    if text.is_empty() {
        return None;
    }
    if text.chars().count() <= CELL_CHARS {
        return Some(text);
    }
    let mut out: String = text.chars().take(CELL_CHARS - 1).collect();
    out.push('…');
    Some(out)
}

fn did_to_string(did: &name_lib::DID) -> String {
    format!("did:{}:{}", did.method, did.id)
}

#[cfg(test)]
mod tests {
    use name_lib::ActionResponse;
    use serde_json::json;

    use crate::router::{RouteMethod, RouteTrace};

    use super::*;

    fn trace() -> RouteTrace {
        RouteTrace {
            route_id: "r".to_string(),
            adapter: "did-object".to_string(),
            method: RouteMethod::XCall,
        }
    }

    #[tokio::test]
    async fn forms_and_zone_rewrites_check_the_full_origin() {
        let zone = crate::zone::tests::TestZone {
            origin: Some("http://local-gateway:3180".into()),
            data_root: Default::default(),
        };
        let adapter = DidObjectProtocolAdapter::with_zone("object".into(), Arc::new(zone));
        assert_eq!(
            adapter
                .zone_web_endpoint("https://test.buckyos.io:443/card")
                .as_deref(),
            Some("http://local-gateway:3180/card")
        );
        assert!(adapter
            .zone_web_endpoint("https://test.buckyos.io:8443/card")
            .is_none());
        for (source, target, allowed) in [
            (
                "https://provider.test/card",
                "https://forms.test/call",
                true,
            ),
            (
                "https://provider.test/card",
                "http://forms.test/call",
                false,
            ),
            (
                "https://provider.test/card",
                "http://local-gateway:3180/secret",
                false,
            ),
            (
                "http://local-gateway:3180/card",
                "http://local-gateway:3180/call",
                true,
            ),
            (
                "http://localhost:7000/card",
                "http://localhost:7000/call",
                true,
            ),
            (
                "http://localhost:7000/card",
                "http://localhost:7001/call",
                false,
            ),
        ] {
            assert_eq!(adapter.form_client(source, target).await.is_ok(), allowed);
        }
    }

    #[test]
    fn maps_action_success_to_detail() {
        let res = action_response_to_adapter_response(
            "reserve",
            ActionResponse::success(json!({"ok": true})),
            trace(),
        );
        assert_eq!(res.status, AdapterCallStatus::Success);
        assert_eq!(res.detail["result"]["ok"], true);
    }

    #[test]
    fn maps_action_error_to_error_status() {
        let res = action_response_to_adapter_response(
            "reserve",
            ActionResponse::failure("denied", "no"),
            trace(),
        );
        assert_eq!(res.status, AdapterCallStatus::Error);
        assert_eq!(res.detail["error"]["code"], "denied");
    }
}
