use super::{
    AdapterCredentialContract, AdapterDescriptor, AdapterStatus, CodecCall, CodecRegistration,
    ExecutionMode, HttpBody, HttpRequest, HttpResponse, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolError, ProtocolErrorKind, ProtocolExecution, ProtocolOutput,
    ProtocolResultValue,
};
use async_trait::async_trait;
use buckyos_api::{AiUsage, AiccCall, ApiType, DecisionAnswer, DecisionQuestion};
use reqwest::{header::CONTENT_TYPE, Method, Url};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub(crate) fn typesafe_adapter() -> (AdapterDescriptor, CodecRegistration) {
    let mut binding = OperationBinding::new(ApiType::Decision, [ExecutionMode::Immediate]);
    binding.supported_features = [
        "decision.choice",
        "decision.score",
        "decision.boolean",
        "decision.probabilities",
        "decision.structured_state",
        "decision.structured_rules",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let operation = OperationDescriptor {
        operation_id: "systemone.evaluate".into(),
        bindings: vec![binding],
        supports_cancel: false,
        supports_webhook: false,
        max_request_bytes: 2 * 1024 * 1024,
        max_response_bytes: 8 * 1024 * 1024,
    };
    (
        AdapterDescriptor {
            protocol_family_id: "typesafe-systemone".into(),
            protocol_adapter_id: "typesafe-systemone".into(),
            interface_generation: "v1".into(),
            base_adapter_id: None,
            status: AdapterStatus::Stable,
            probe_priority: 100,
            probe_path: None,
            credential: AdapterCredentialContract::bearer(),
            operations: BTreeMap::from([(operation.operation_id.clone(), operation.clone())]),
        },
        CodecRegistration {
            operation_codecs: vec![Arc::new(TypeSafeCodec {
                descriptor: operation,
            })],
            native_task_codecs: Vec::new(),
        },
    )
}

struct TypeSafeCodec {
    descriptor: OperationDescriptor,
}

#[async_trait]
impl OperationCodec for TypeSafeCodec {
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
                "System One requires decision.evaluate",
            ));
        };
        request
            .validate()
            .map_err(|e| ProtocolError::invalid_request(e.message))?;
        if request.execution_mode != buckyos_api::AiccExecutionMode::Immediate {
            return Err(ProtocolError::new(
                ProtocolErrorKind::UnsupportedOperation,
                "System One only supports immediate execution",
            ));
        }
        if call
            .input
            .resolved_parameters
            .keys()
            .any(|key| key != "provider_model_id")
        {
            return Err(ProtocolError::invalid_request(
                "System One received unsupported provider parameters",
            ));
        }
        let model = call
            .input
            .resolved_parameters
            .get("provider_model_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| ProtocolError::invalid_request("missing provider_model_id"))?;
        let questions = encode_questions(&request.questions);
        let mut url = Url::parse(&call.context.base_url)
            .map_err(|_| ProtocolError::invalid_configuration("invalid System One base URL"))?;
        let prefix = url.path().trim_end_matches('/');
        url.set_path(&format!("{prefix}/systemone"));
        let mut wire = HttpRequest::new(Method::POST, url.to_string());
        wire.headers
            .insert(CONTENT_TYPE, "application/json".parse().unwrap());
        call.context
            .credential
            .as_ref()
            .ok_or_else(|| {
                ProtocolError::new(
                    ProtocolErrorKind::Authentication,
                    "System One credential is missing",
                )
            })?
            .apply(&mut wire.headers)?;
        wire.body =
            HttpBody::Json(json!({"model": model, "state": request.state, "questions": questions}));
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
                429 => ProtocolErrorKind::Transport,
                529 => ProtocolErrorKind::Transport,
                _ => super::protocol_error_kind_from_http_status(response.status),
            };
            return Err(ProtocolError::new(kind, "System One request failed")
                .with_provider_code(Some(response.status.as_u16().to_string()))
                .with_request_id(Some(response.request_id))
                .with_retry_after(response.retry_after));
        }
        let body: WireResponse = response.json(self.descriptor.max_response_bytes)?;
        if body.model.trim().is_empty() {
            return Err(ProtocolError::invalid_response(
                "System One response model is empty",
            ));
        }
        let answers = decode_answers(body.answers)?;
        let total = body
            .usage
            .input_tokens
            .checked_add(body.usage.output_tokens)
            .ok_or_else(|| ProtocolError::invalid_response("System One token usage overflow"))?;
        Ok(ProtocolExecution::Immediate(ProtocolOutput {
            value: json!({"answers": answers, "model": body.model}),
            usage: Some(AiUsage {
                input_tokens: Some(body.usage.input_tokens),
                output_tokens: Some(body.usage.output_tokens),
                total_tokens: Some(total),
                ..Default::default()
            }),
            artifacts: Vec::new(),
        }))
    }
}

pub(super) fn encode_questions(input: &[DecisionQuestion]) -> Map<String, Value> {
    let mut questions = Map::new();
    for question in input {
        let mut wire = json!({"instructions": question.instructions()});
        match question {
            DecisionQuestion::Choice { options, .. } => {
                wire["type"] = json!("choice");
                wire["criteria"] = Value::Object(
                    options
                        .iter()
                        .map(|o| (o.id.clone(), o.description.clone()))
                        .collect(),
                );
            }
            DecisionQuestion::Score { levels, .. } => {
                wire["type"] = json!("score");
                wire["criteria"] = json!(levels);
            }
            DecisionQuestion::Boolean { criteria, .. } => {
                wire["type"] = json!("noul");
                if let Some(criteria) = criteria {
                    wire["criteria"] = json!(criteria);
                }
            }
        }
        questions.insert(question.id().to_owned(), wire);
    }
    questions
}

pub(super) fn decode_answers(
    input: BTreeMap<String, WireAnswer>,
) -> ProtocolResultValue<Vec<DecisionAnswer>> {
    let mut answers = Vec::new();
    for (id, answer) in input {
        answers.push(match answer {
            WireAnswer::Choice {
                choice,
                probabilities,
                confidence,
            } => DecisionAnswer::Choice {
                id,
                selected: choice,
                probabilities,
                confidence,
            },
            WireAnswer::Noul { noul, confidence } => DecisionAnswer::Boolean {
                id,
                probability_true: noul,
                confidence,
            },
            WireAnswer::Score {
                score,
                legend,
                probabilities,
                confidence,
            } => {
                let levels = (0..legend.len())
                    .map(|i| {
                        legend.get(&i.to_string()).cloned().ok_or_else(|| {
                            ProtocolError::invalid_response(
                                "System One score legend is not a complete ordered scale",
                            )
                        })
                    })
                    .collect::<ProtocolResultValue<Vec<_>>>()?;
                DecisionAnswer::Score {
                    id,
                    score,
                    levels,
                    probabilities,
                    confidence,
                }
            }
        });
    }
    Ok(answers)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireResponse {
    model: String,
    answers: BTreeMap<String, WireAnswer>,
    usage: WireUsage,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WireUsage {
    input_tokens: u64,
    output_tokens: u64,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum WireAnswer {
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f64>,
        confidence: Option<f64>,
    },
    Score {
        score: f64,
        legend: BTreeMap<String, Value>,
        probabilities: BTreeMap<String, f64>,
        confidence: Option<f64>,
    },
    Noul {
        noul: f64,
        confidence: Option<f64>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        CodecContext, CodecInput, CodecLimits, CodecRegistry, GoldenBody, ProtocolContractHarness,
        ResolvedCredential,
    };
    use buckyos_api::{DecisionEvaluateRequest, ProviderStateCoordinate};
    use std::time::{Duration, UNIX_EPOCH};

    fn fixture() -> Value {
        serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../test/aicc_test/acceptance/fixtures/typesafe-systemone.json"
        )))
        .unwrap()
    }

    fn context() -> CodecContext {
        CodecContext {
            base_url: "https://api.typesafe.ai/v1".into(),
            state_coordinate: ProviderStateCoordinate {
                provider_profile_id: "typesafe".into(),
                adapter_type: "typesafe-systemone".into(),
                origin_provider: "typesafe".into(),
                origin_model: "jev-1.13.0".into(),
            },
            credential: Some(
                ResolvedCredential::bearer("secret://typesafe", "test-only-secret").unwrap(),
            ),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(3),
                max_request_bytes: 2 * 1024 * 1024,
                max_response_bytes: 8 * 1024 * 1024,
            },
        }
    }

    fn registry() -> CodecRegistry {
        let mut registry = CodecRegistry::default();
        let (descriptor, codecs) = typesafe_adapter();
        registry.register_codecs(descriptor, codecs).unwrap();
        registry
    }

    fn input() -> CodecInput {
        CodecInput {
            canonical_request: AiccCall::DecisionEvaluate(
                DecisionEvaluateRequest::from_json(fixture()["canonical_request"].clone()).unwrap(),
            ),
            resolved_parameters: BTreeMap::from([(
                "provider_model_id".into(),
                json!("jev-1.13.0"),
            )]),
        }
    }

    #[tokio::test]
    async fn typesafe_official_mixed_fixture_preserves_request_probabilities_confidence_and_usage()
    {
        let registry = registry();
        let harness = ProtocolContractHarness::default();
        let wire = registry
            .encode(
                "typesafe-systemone",
                "systemone.evaluate",
                ApiType::Decision,
                &input(),
                &context(),
            )
            .unwrap();
        assert_eq!(
            wire.headers[reqwest::header::AUTHORIZATION],
            "Bearer test-only-secret"
        );
        let golden = harness.request(&wire).unwrap();
        assert_eq!(golden.url, "https://api.typesafe.ai/v1/systemone");
        assert_eq!(golden.method, "POST");
        assert_eq!(
            golden.body,
            GoldenBody::Json(fixture()["wire_request"].clone())
        );
        harness
            .assert_no_secrets(&format!("{golden:?}"), &["test-only-secret"])
            .unwrap();
        let response = harness
            .response(
                reqwest::StatusCode::OK,
                &[],
                serde_json::to_vec(&fixture()["wire_response"]).unwrap(),
                "req-1",
                UNIX_EPOCH,
            )
            .unwrap();
        let ProtocolExecution::Immediate(output) = registry
            .decode(
                "typesafe-systemone",
                "systemone.evaluate",
                ApiType::Decision,
                response,
            )
            .await
            .unwrap()
        else {
            panic!()
        };
        let answers: Vec<DecisionAnswer> =
            serde_json::from_value(output.value["answers"].clone()).unwrap();
        let AiccCall::DecisionEvaluate(request) = input().canonical_request else {
            panic!()
        };
        request.validate_answers(&answers).unwrap();
        assert_eq!(
            answers
                .iter()
                .find(|a| a.id() == "contains")
                .unwrap()
                .confidence(),
            None
        );
        assert_eq!(
            answers
                .iter()
                .find(|a| a.id() == "marker")
                .unwrap()
                .confidence(),
            Some(0.85)
        );
        assert_eq!(output.usage.unwrap().output_tokens, Some(72));
    }

    #[tokio::test]
    async fn typesafe_errors_and_malformed_response_are_explicit_and_redacted() {
        let registry = registry();
        let harness = ProtocolContractHarness::default();
        for (status, kind) in [
            (401, ProtocolErrorKind::Authentication),
            (422, ProtocolErrorKind::InvalidRequest),
            (429, ProtocolErrorKind::Transport),
            (504, ProtocolErrorKind::Timeout),
            (529, ProtocolErrorKind::Transport),
        ] {
            let response = harness
                .response(
                    reqwest::StatusCode::from_u16(status).unwrap(),
                    &[("retry-after", "2")],
                    b"{\"message\":\"test-only-secret\"}".as_slice(),
                    "req-error",
                    UNIX_EPOCH,
                )
                .unwrap();
            let error = registry
                .decode(
                    "typesafe-systemone",
                    "systemone.evaluate",
                    ApiType::Decision,
                    response,
                )
                .await
                .err()
                .unwrap();
            assert_eq!(error.kind, kind);
            assert_eq!(error.provider_code, Some(status.to_string()));
            assert_eq!(error.retry_after, Some(Duration::from_secs(2)));
            harness
                .assert_no_secrets(&format!("{error:?}"), &["test-only-secret"])
                .unwrap();
        }
        for value in [
            json!({}),
            json!({"model":"jev-1.13.0","answers":{},"usage":{"input_tokens":-1,"output_tokens":2}}),
            json!({"model":"jev-1.13.0","answers":{"q":{"type":"unknown"}},"usage":{"input_tokens":1,"output_tokens":2}}),
        ] {
            let response = harness
                .response(
                    reqwest::StatusCode::OK,
                    &[],
                    serde_json::to_vec(&value).unwrap(),
                    "req-invalid",
                    UNIX_EPOCH,
                )
                .unwrap();
            assert!(registry
                .decode(
                    "typesafe-systemone",
                    "systemone.evaluate",
                    ApiType::Decision,
                    response
                )
                .await
                .is_err());
        }
        let mut stream = input();
        let AiccCall::DecisionEvaluate(request) = &mut stream.canonical_request else {
            panic!()
        };
        request.execution_mode = buckyos_api::AiccExecutionMode::Stream;
        assert!(registry
            .encode(
                "typesafe-systemone",
                "systemone.evaluate",
                ApiType::Decision,
                &stream,
                &context()
            )
            .is_err());
    }
}
