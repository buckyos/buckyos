use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::ObjectUrl;
use name_client::{DIDObjectClient, ResolvedDIDObject};
use name_lib::{ActionResponse, EventSubscribeRequest, DEFAULT_EXPIRE_TIME};
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
        if parsed.scheme() != "https" || !parsed.host_str()?.eq_ignore_ascii_case(&zone_host) {
            return None;
        }
        let origin = self.zone.gateway_origin()?;
        let query = parsed.query().map(|q| format!("?{q}")).unwrap_or_default();
        Some(format!("{}{}{query}", origin.trim_end_matches('/'), parsed.path()))
    }

    /// The client for `endpoint` (with the caller's session on the Zone's own origins) and the object it resolves.
    async fn resolve(
        &self,
        url: &ObjectUrl,
        endpoint: &str,
    ) -> Result<(DIDObjectClient, reqwest::Client, ResolvedDIDObject), (reqwest::Client, AgentDIDObjectError)> {
        let http = client_for(self.zone.as_ref(), endpoint).await;
        let client = DIDObjectClient::with_http_client(http.clone());
        match client.resolve(endpoint).await {
            Ok(resolved) => Ok((client, http, resolved)),
            Err(err) => Err((http, AgentDIDObjectError::ResolveError(format!("{url}: {err}")))),
        }
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
        let (client, resolved) = match self.resolve(&url, &endpoint).await {
            Ok((client, _, resolved)) => (client, resolved),
            // An https page that is no DID Object: read it as web content.
            Err((http, _)) if matches!(url, ObjectUrl::Web { .. }) => {
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
            property_values
                .values()
                .map(|value| match value {
                    Value::String(text) => text.clone(),
                    other => serde_json::to_string_pretty(other).unwrap_or_default(),
                })
                .collect::<Vec<_>>()
                .join("\n")
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
            version: card
                .iat
                .or_else(|| {
                    card.exp
                        .map(|exp| exp.saturating_sub(DEFAULT_EXPIRE_TIME))
                })
                .map(|value| value.to_string()),
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
        let (client, _, resolved) = self.resolve(&url, &endpoint).await.map_err(|(_, err)| err)?;
        if !DIDObjectClient::has_action(&resolved.object_profile, &req.input.action) {
            return Err(AgentDIDObjectError::DeclaredCapabilityNotFound(format!(
                "action {} is not declared by {}",
                req.input.action, resolved.object_url
            )));
        }
        // The side effect the Profile declares (`x-buckyos:risk`) goes with the result: the Provider authorizes the
        // caller, the record shows what kind of call it was (TODO A7 / G2).
        let risk = serde_json::to_value(&resolved.object_profile)
            .ok()
            .and_then(|p| p["actions"][&req.input.action]["x-buckyos:risk"].as_str().map(str::to_string));
        let response = client
            .invoke_action_from_resolved(&resolved, &req.input.action, req.input.params.clone())
            .await
            .map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string()))?;
        let mut response = action_response_to_adapter_response(&req.input.action, response, req.route_trace);
        if let Value::Object(detail) = &mut response.detail {
            detail.insert("object".into(), json!(url.to_string()));
            detail.insert("risk".into(), json!(risk));
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
            output: None,
            detail: json!({
                "error": error,
                "meta": response.meta,
            }),
            title: format!("xcall `{action}` failed").into(),
            summary: error_summary(action),
            route,
        };
    }

    AdapterXCallResponse {
        status: AdapterCallStatus::Success,
        output: None,
        detail: json!({
            "result": response.result.unwrap_or(Value::Null),
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
            let risk = text(entry, "x-buckyos:risk");
            if !risk.is_empty() {
                line.push_str(&format!(" (risk: {risk})"));
            }
            lines.push(line);
        }
    }
    if !summary.is_empty() {
        lines.push("Summary:".to_string());
        for (name, value) in summary {
            lines.push(format!("- {name}:"));
            lines.push(serde_json::to_string_pretty(value).unwrap_or_default());
        }
    }
    lines.join("\n")
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
