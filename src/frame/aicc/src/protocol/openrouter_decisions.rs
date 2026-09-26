use super::{
    CodecCall, ExecutionMode, HttpBody, HttpRequest, HttpResponse, OperationBinding,
    OperationCodec, OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolExecution,
    ProtocolOutput, ProtocolResultValue,
};
use async_trait::async_trait;
use buckyos_api::{AiUsage, AiccCall, ApiType, DecisionQuestion};
use reqwest::{header::CONTENT_TYPE, Method, Url};
use serde::Deserialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const OPENROUTER_DECISIONS_OPERATION_ID: &str = "decisions.create";
pub(crate) const JEV_CHANNEL_ID: &str = "typesafe/jev-1.13";
pub(crate) const JEV_ALIAS_ID: &str = "~typesafe/jev-latest";
pub(crate) const JEV_BUILD_ID: &str = "typesafe/jev-1.13-20260917";
pub(crate) const JEV_ORIGIN_ID: &str = "jev-1.13.0";
pub(crate) const DECISION_FEATURES: [&str; 6] = [
    "decision.choice",
    "decision.score",
    "decision.boolean",
    "decision.probabilities",
    "decision.structured_state",
    "decision.structured_rules",
];

pub(super) struct OpenRouterDecisionsCodec {
    pub descriptor: OperationDescriptor,
}

pub(super) fn descriptor() -> OperationDescriptor {
    let mut binding = OperationBinding::new(ApiType::Decision, [ExecutionMode::Immediate]);
    binding.supported_features = DECISION_FEATURES.into_iter().map(str::to_owned).collect();
    OperationDescriptor {
        operation_id: OPENROUTER_DECISIONS_OPERATION_ID.into(),
        bindings: vec![binding],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: 2 * 1024 * 1024,
        max_response_bytes: 8 * 1024 * 1024,
    }
}

fn endpoint(base_url: &str) -> ProtocolResultValue<String> {
    let mut url = Url::parse(base_url)
        .map_err(|_| ProtocolError::invalid_configuration("invalid OpenRouter base URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ProtocolError::invalid_configuration(
            "OpenRouter requires an HTTP origin without URL credentials",
        ));
    }
    let path = url.path().trim_end_matches('/');
    let prefix = path.strip_suffix("/api/v1").unwrap_or(path);
    url.set_path(&format!("{prefix}/api/alpha/decisions"));
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.into())
}

#[async_trait]
impl OperationCodec for OpenRouterDecisionsCodec {
    fn descriptor(&self) -> &OperationDescriptor {
        &self.descriptor
    }
    fn api_type(&self) -> ApiType {
        ApiType::Decision
    }
    fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
        BTreeSet::from([ExecutionMode::Immediate])
    }

    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
        call.context.validate()?;
        call.input
            .validate_for(self.descriptor.binding(call.api_type)?)?;
        let AiccCall::DecisionEvaluate(request) = &call.input.canonical_request else {
            return Err(ProtocolError::invalid_request(
                "OpenRouter Decisions requires decision.evaluate",
            ));
        };
        request
            .validate()
            .map_err(|e| ProtocolError::invalid_request(e.message))?;
        if request.execution_mode != buckyos_api::AiccExecutionMode::Immediate {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "OpenRouter Decisions only supports immediate execution",
            ));
        }
        if call
            .input
            .resolved_parameters
            .keys()
            .any(|key| key != "provider_model_id")
        {
            return Err(ProtocolError::invalid_request(
                "unsupported OpenRouter Decisions parameter",
            ));
        }
        let model = call
            .input
            .resolved_parameters
            .get("provider_model_id")
            .and_then(|v| v.as_str());
        if !matches!(model, Some(JEV_CHANNEL_ID | JEV_ALIAS_ID))
            || call.context.state_coordinate.origin_provider != "typesafe"
            || call.context.state_coordinate.origin_model != JEV_ORIGIN_ID
        {
            return Err(ProtocolError::invalid_request(
                "unverified OpenRouter decision identity",
            ));
        }
        for question in &request.questions {
            if let DecisionQuestion::Boolean {
                criteria: Some(criteria),
                ..
            } = question
            {
                let value = serde_json::to_value(criteria)
                    .map_err(|_| ProtocolError::invalid_request("invalid boolean criteria"))?;
                if value.get("true").is_none() || value.get("false").is_none() {
                    return Err(ProtocolError::invalid_request(
                        "OpenRouter noul criteria require both true and false when supplied",
                    ));
                }
            }
        }
        let mut wire = HttpRequest::new(Method::POST, endpoint(&call.context.base_url)?);
        wire.headers
            .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        call.context
            .credential
            .as_ref()
            .ok_or_else(|| {
                ProtocolError::new(
                    ProtocolErrorKind::Authentication,
                    "OpenRouter credential is missing",
                )
            })?
            .apply(&mut wire.headers)?;
        wire.body = HttpBody::Json(json!({"model": model, "state": request.state,
            "questions": super::typesafe::encode_questions(&request.questions)}));
        wire.timeout = Some(call.context.limits.request_timeout);
        wire.max_request_bytes = Some(
            call.context
                .limits
                .max_request_bytes
                .min(self.descriptor.max_request_bytes),
        );
        wire.max_response_bytes = Some(
            call.context
                .limits
                .max_response_bytes
                .min(self.descriptor.max_response_bytes),
        );
        Ok(wire)
    }

    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
        if !response.status.is_success() {
            let kind = match response.status.as_u16() {
                402 => ProtocolErrorKind::ProviderRejected,
                413 => ProtocolErrorKind::InvalidRequest,
                429 | 529 => ProtocolErrorKind::Transport,
                524 => ProtocolErrorKind::Timeout,
                _ => super::protocol_error_kind_from_http_status(response.status),
            };
            return Err(
                ProtocolError::new(kind, "OpenRouter Decisions request failed")
                    .with_http_status(response.status.as_u16())
                    .with_provider_code(Some(response.status.as_u16().to_string()))
                    .with_request_id(Some(response.request_id))
                    .with_retry_after(response.retry_after),
            );
        }
        let body: WireResponse = response.json(self.descriptor.max_response_bytes)?;
        if body.model != JEV_BUILD_ID {
            return Err(ProtocolError::invalid_response(
                "unverified OpenRouter decision response build; refresh verified metadata",
            )
            .with_request_id(Some(response.request_id)));
        }
        if body
            .usage
            .cost
            .is_some_and(|cost| !cost.is_finite() || cost < 0.0)
        {
            return Err(ProtocolError::invalid_response(
                "invalid OpenRouter decision cost",
            ));
        }
        let total = body
            .usage
            .input_tokens
            .checked_add(body.usage.output_tokens)
            .ok_or_else(|| {
                ProtocolError::invalid_response("OpenRouter decision token usage overflow")
            })?;
        let answers = super::typesafe::decode_answers(body.answers)?;
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({"answers": answers, "model": JEV_ORIGIN_ID,
                "provider_metadata": {"model": body.model, "id": body.id, "provider": body.provider}}),
            usage: Some(AiUsage {
                input_tokens: Some(body.usage.input_tokens),
                output_tokens: Some(body.usage.output_tokens),
                total_tokens: Some(total),
                reported_cost: body.usage.cost,
                ..Default::default()
            }),
            artifacts: Vec::new(),
        }))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    model: String,
    answers: BTreeMap<String, super::typesafe::WireAnswer>,
    usage: WireUsage,
    id: Option<String>,
    provider: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireUsage {
    input_tokens: u64,
    output_tokens: u64,
    cost: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, GoldenBody, ProtocolContractHarness,
        ResolvedCredential,
    };
    use buckyos_api::{DecisionAnswer, DecisionEvaluateRequest, ProviderStateCoordinate};
    use serde_json::Value;
    use std::time::{Duration, UNIX_EPOCH};

    fn fixture() -> Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../test/aicc_test/acceptance/fixtures/openrouter-decisions.json"
        )))
        .unwrap()
    }
    fn context(base: &str) -> CodecContext {
        CodecContext {
            base_url: base.into(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "openrouter".into(),
                adapter_type: "openrouter-responses".into(),
                origin_provider: "typesafe".into(),
                origin_model: JEV_ORIGIN_ID.into(),
            },
            credential: Some(ResolvedCredential::bearer("secret://test", "test-secret").unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(3),
                max_request_bytes: 1024 * 1024,
                max_response_bytes: 1024 * 1024,
            },
        }
    }
    fn input(model: &str) -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::DecisionEvaluate(
                DecisionEvaluateRequest::from_json(fixture()["canonical_request"].clone()).unwrap(),
            ),
            resolved_parameters: BTreeMap::from([("provider_model_id".into(), json!(model))]),
        }
    }
    fn registry() -> CodecRegistry {
        let mut r = CodecRegistry::default();
        let (d, c) = super::super::openai_responses_adapter();
        r.register_codecs(d, c).unwrap();
        let (d, c) = super::super::openrouter_responses_adapter();
        r.register_derived(d, c).unwrap();
        r
    }
    async fn decode(value: Value) -> ProtocolResultValue<ProtocolOutput> {
        let response = ProtocolContractHarness::default()
            .response(
                reqwest::StatusCode::OK,
                &[],
                serde_json::to_vec(&value).unwrap(),
                "req-test",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = registry()
            .decode(
                "openrouter-responses",
                OPENROUTER_DECISIONS_OPERATION_ID,
                ApiType::Decision,
                response,
            )
            .await?
        else {
            panic!()
        };
        Ok(output)
    }
    #[tokio::test]
    async fn openrouter_decisions_official_mixed_fixture_and_same_origin_endpoints() {
        for (base, expected) in [
            (
                "https://openrouter.ai/api/v1",
                "https://openrouter.ai/api/alpha/decisions",
            ),
            (
                "https://proxy.test/tenant/api/v1/",
                "https://proxy.test/tenant/api/alpha/decisions",
            ),
            (
                "https://proxy.test/tenant",
                "https://proxy.test/tenant/api/alpha/decisions",
            ),
            (
                "https://proxy.test/",
                "https://proxy.test/api/alpha/decisions",
            ),
        ] {
            let wire = registry()
                .encode(
                    "openrouter-responses",
                    OPENROUTER_DECISIONS_OPERATION_ID,
                    ApiType::Decision,
                    &input(JEV_CHANNEL_ID),
                    &context(base),
                )
                .unwrap();
            assert_eq!(wire.url, expected);
            assert_eq!(
                wire.headers[reqwest::header::AUTHORIZATION],
                "Bearer test-secret"
            );
            assert_eq!(
                ProtocolContractHarness::default()
                    .request(&wire)
                    .unwrap()
                    .body,
                GoldenBody::Json(fixture()["wire_request"].clone())
            );
        }
        for base in ["file:///tmp/test", "https://user:pass@proxy.test/api/v1"] {
            assert!(endpoint(base).is_err());
        }
        for model in [JEV_CHANNEL_ID, JEV_ALIAS_ID] {
            assert!(registry()
                .encode(
                    "openrouter-responses",
                    OPENROUTER_DECISIONS_OPERATION_ID,
                    ApiType::Decision,
                    &input(model),
                    &context("https://proxy.test/api/v1")
                )
                .is_ok());
        }
        for model in ["typesafe/jev-1.14", JEV_BUILD_ID, "typesafe/jev-router"] {
            assert!(registry()
                .encode(
                    "openrouter-responses",
                    OPENROUTER_DECISIONS_OPERATION_ID,
                    ApiType::Decision,
                    &input(model),
                    &context("https://proxy.test/api/v1")
                )
                .is_err());
        }
        let output = decode(fixture()["wire_response"].clone()).await.unwrap();
        let answers: Vec<DecisionAnswer> =
            serde_json::from_value(output.value["answers"].clone()).unwrap();
        let AiccCall::DecisionEvaluate(request) = input(JEV_CHANNEL_ID).canonical_request else {
            panic!()
        };
        request.validate_answers(&answers).unwrap();
        assert_eq!(output.value["model"], JEV_ORIGIN_ID);
        assert_eq!(output.value["provider_metadata"]["model"], JEV_BUILD_ID);
        assert_eq!(output.value["provider_metadata"]["id"], "gen-dec-fixture");
        assert_eq!(output.usage.as_ref().unwrap().output_tokens, Some(72));
        assert_eq!(output.usage.unwrap().reported_cost, Some(0.000013356));
        assert_eq!(
            answers
                .iter()
                .find(|a| a.id() == "contains")
                .unwrap()
                .confidence(),
            None
        );
    }
    #[tokio::test]
    async fn openrouter_decisions_rejects_drift_invalid_usage_and_answers() {
        for (pointer, value) in [
            ("/model", json!("typesafe/jev-1.13-20260918")),
            ("/model", json!(JEV_ALIAS_ID)),
            ("/usage/cost", json!(-0.01)),
            ("/usage/input_tokens", json!(-1)),
            ("/usage/output_tokens", json!(1.5)),
            ("/usage", Value::Null),
        ] {
            let mut body = fixture()["wire_response"].clone();
            *body.pointer_mut(pointer).unwrap() = value;
            assert!(decode(body).await.is_err(), "{pointer}");
        }
        let AiccCall::DecisionEvaluate(request) = input(JEV_CHANNEL_ID).canonical_request else {
            panic!()
        };
        for (pointer, value) in [
            ("/answers/marker/probabilities/present", json!(0.2)),
            ("/answers/priority/score", json!(0.0)),
            ("/answers/contains/noul", json!(2)),
            ("/answers/marker/confidence", json!(-0.1)),
            ("/answers/marker/choice", json!("other")),
        ] {
            let mut body = fixture()["wire_response"].clone();
            *body.pointer_mut(pointer).unwrap() = value;
            let out = decode(body).await.unwrap();
            let answers: Vec<DecisionAnswer> =
                serde_json::from_value(out.value["answers"].clone()).unwrap();
            assert!(request.validate_answers(&answers).is_err(), "{pointer}");
        }
        for missing in [true, false] {
            let mut body = fixture()["wire_response"].clone();
            if missing {
                body["answers"].as_object_mut().unwrap().remove("contains");
            } else {
                body["answers"]["extra"] = json!({"type":"noul","noul":0.5});
            }
            let out = decode(body).await.unwrap();
            let answers: Vec<DecisionAnswer> =
                serde_json::from_value(out.value["answers"].clone()).unwrap();
            assert!(request.validate_answers(&answers).is_err());
        }
        for cost in [None, Some(0.0)] {
            let mut body = fixture()["wire_response"].clone();
            body["usage"].as_object_mut().unwrap().remove("cost");
            if let Some(cost) = cost {
                body["usage"]["cost"] = json!(cost);
            }
            assert_eq!(
                decode(body).await.unwrap().usage.unwrap().reported_cost,
                cost
            );
        }
    }
    #[tokio::test]
    async fn openrouter_decisions_http_errors_preserve_audit_without_secrets() {
        for (status, kind) in [
            (400, ProtocolErrorKind::InvalidRequest),
            (401, ProtocolErrorKind::Authentication),
            (403, ProtocolErrorKind::Authentication),
            (402, ProtocolErrorKind::ProviderRejected),
            (413, ProtocolErrorKind::InvalidRequest),
            (429, ProtocolErrorKind::Transport),
            (500, ProtocolErrorKind::Transport),
            (524, ProtocolErrorKind::Timeout),
            (529, ProtocolErrorKind::Transport),
        ] {
            let response = ProtocolContractHarness::default()
                .response(
                    reqwest::StatusCode::from_u16(status).unwrap(),
                    &[("retry-after", "2")],
                    br#"{"error":{"code":401,"message":"test-secret"}}"#.as_slice(),
                    "req-error",
                    UNIX_EPOCH,
                )
                .unwrap();
            let error = registry()
                .decode(
                    "openrouter-responses",
                    OPENROUTER_DECISIONS_OPERATION_ID,
                    ApiType::Decision,
                    response,
                )
                .await
                .unwrap_err();
            assert_eq!(error.kind, kind);
            assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
            assert_eq!(error.request_id.as_deref(), Some("req-error"));
            assert!(!format!("{error:?}").contains("test-secret"));
        }
    }
}
