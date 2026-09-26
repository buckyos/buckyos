use async_trait::async_trait;
use buckyos_api::*;
use kRPC::{RPCContext, RPCErrors, RPCHandler, RPCRequest, RPCResult};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn request() -> DecisionEvaluateRequest {
    DecisionEvaluateRequest::from_json(json!({
        "exact_model":"jev-1.13.0@typesafe", "state":{"text":"Urgent payment problem"},
        "questions":[
            {"id":"team","type":"choice","instructions":{"task":"Choose team"},"options":[{"id":"billing","description":"Payments"},{"id":"support","description":null}]},
            {"id":"urgency","type":"score","instructions":"Rate urgency","levels":["Routine","Urgent","Critical"]},
            {"id":"retry","type":"boolean","instructions":"Already retried?","criteria":{"true":{"rule":"Retry mentioned"},"false":"No retry"}}
        ]
    })).unwrap()
}

fn answers() -> Value {
    json!([
        {"id":"team","type":"choice","selected":"billing","probabilities":{"billing":0.8,"support":0.2},"confidence":0.7},
        {"id":"urgency","type":"score","score":1.3,"levels":["Routine","Urgent","Critical"],"probabilities":{"0":0.1,"1":0.5,"2":0.4},"confidence":0.25},
        {"id":"retry","type":"boolean","probability_true":0.65}
    ])
}

#[test]
fn decision_mixed_and_single_questions_round_trip_with_structured_rules() {
    let request = request();
    let answers: Vec<DecisionAnswer> = serde_json::from_value(answers()).unwrap();
    request.validate_answers(&answers).unwrap();
    assert_eq!(answers[2].confidence(), None);
    assert_eq!(serde_json::to_value(&answers).unwrap(), self::answers());
    for (question, answer) in request.questions.iter().zip(&answers) {
        for state in [
            json!("text"),
            json!({"nested":[1,true,null]}),
            json!(["text",{"record":1}]),
        ] {
            let single =
                DecisionEvaluateRequest::new("jev-1.13.0@typesafe", state, vec![question.clone()]);
            single.validate().unwrap();
            single.validate_answers(&[answer.clone()]).unwrap();
            assert_eq!(
                DecisionEvaluateRequest::from_json(serde_json::to_value(&single).unwrap()).unwrap(),
                single
            );
        }
    }
    let requirements = request.requirements().decision.unwrap();
    assert_eq!(requirements.question_types.len(), 3);
    assert!(requirements.structured_state && requirements.structured_rules);
    assert_eq!(requirements.max_options, 2);
    assert_eq!(requirements.max_levels, 3);
    assert_eq!(ApiType::Decision.typed_method(), "decision.evaluate");
    assert_eq!(ApiType::Decision.capability(), Capability::Decision);
    assert_eq!(serde_json::to_value(ApiType::Decision).unwrap(), "decision");
    assert!(ai_methods::is_ai_method("decision.evaluate"));
}

#[test]
fn decision_rejects_invalid_requests_in_both_json_entrypoints() {
    let original = serde_json::to_value(request()).unwrap();
    let mut cases = Vec::new();
    for state in [Value::Null, json!(false), json!(1)] {
        let mut r = original.clone();
        r["state"] = state;
        cases.push(r);
    }
    for (pointer, value) in [
        ("/questions", json!([])),
        ("/questions/0/id", json!("bad id")),
        ("/questions/1/id", json!("team")),
        ("/questions/0/options/1/id", json!("billing")),
        ("/questions/1/levels", json!(["one"])),
        ("/questions/1/levels", json!(["one", "one"])),
        ("/questions/0/instructions", json!(42)),
        ("/exact_model", json!("decision")),
    ] {
        let mut r = original.clone();
        *r.pointer_mut(pointer).unwrap() = value;
        cases.push(r);
    }
    let mut r = original.clone();
    r["questions"][0]["unexpected"] = json!(true);
    cases.push(r);
    let mut r = original.clone();
    r["unexpected"] = json!(true);
    cases.push(r);
    let mut r = original.clone();
    r["state"] = json!("x".repeat(DECISION_MAX_INPUT_BYTES));
    cases.push(r);
    for value in cases {
        assert!(DecisionEvaluateRequest::from_json(value.clone()).is_err());
        assert!(AiccCall::from_method_and_params(ai_methods::DECISION_EVALUATE, value).is_err());
    }
}

#[test]
fn decision_rejects_incomplete_wrong_type_and_invalid_distributions_without_repair() {
    let request = request();
    let original = answers();
    for (pointer, value) in [
        ("/0/id", json!("other")),
        ("/2/id", json!("team")),
        ("/0/selected", json!("outside")),
        ("/0/selected", json!("support")),
        ("/0/probabilities/billing", json!(1.1)),
        ("/0/probabilities/billing", json!(0.7)),
        ("/1/score", json!(2.1)),
        ("/1/score", json!(0.9)),
        ("/1/levels/0", json!("wrong")),
        ("/2/probability_true", json!(-0.1)),
        ("/0/confidence", json!(1.5)),
        ("/0/probabilities", json!({"billing":1.0})),
        (
            "/0/probabilities",
            json!({"billing":0.8,"support":0.2,"extra":0.0}),
        ),
    ] {
        let mut mutated = original.clone();
        *mutated.pointer_mut(pointer).unwrap() = value;
        let typed: Vec<DecisionAnswer> = serde_json::from_value(mutated.clone()).unwrap();
        assert!(
            request.validate_answers(&typed).is_err(),
            "accepted {pointer}"
        );
        assert_eq!(serde_json::to_value(typed).unwrap(), mutated);
    }
    let mut typed: Vec<DecisionAnswer> = serde_json::from_value(original).unwrap();
    typed[0] = DecisionAnswer::Boolean {
        id: "team".into(),
        probability_true: 0.5,
        confidence: None,
    };
    assert!(request.validate_answers(&typed).is_err());
    assert!(request.validate_answers(&typed[..2]).is_err());
    typed.push(typed[2].clone());
    assert!(request.validate_answers(&typed).is_err());
    let single = DecisionEvaluateRequest::new(
        "m@p",
        json!("s"),
        vec![DecisionQuestion::Boolean {
            id: "q".into(),
            instructions: json!("q"),
            criteria: None,
        }],
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(single
            .validate_answers(&[DecisionAnswer::Boolean {
                id: "q".into(),
                probability_true: value,
                confidence: None
            }])
            .is_err());
    }
    let mut value = answers();
    value[0]["unknown"] = json!(1);
    assert!(serde_json::from_value::<Vec<DecisionAnswer>>(value).is_err());
}

struct Handler(Arc<AtomicUsize>);
#[async_trait]
impl AiccHandler for Handler {
    async fn handle_cancel(
        &self,
        task_id: &str,
        _ctx: RPCContext,
    ) -> Result<CancelResponse, RPCErrors> {
        Ok(CancelResponse::new(task_id.to_owned(), false))
    }
    async fn handle_decision_evaluate(
        &self,
        request: DecisionEvaluateRequest,
        _ctx: RPCContext,
    ) -> Result<DecisionEvaluateResponse, RPCErrors> {
        self.0.fetch_add(1, Ordering::SeqCst);
        request.validate().unwrap();
        let mut response =
            DecisionEvaluateResponse::new("task-decision", AiMethodStatus::Succeeded);
        response.answers = serde_json::from_value(answers()).unwrap();
        Ok(response)
    }
}

#[tokio::test]
async fn decision_client_and_krpc_dispatch_validate_before_handler() {
    let count = Arc::new(AtomicUsize::new(0));
    let client = AiccClient::new_in_process(Box::new(Handler(count.clone())));
    assert_eq!(
        client
            .decision_evaluate(request())
            .await
            .unwrap()
            .answers
            .len(),
        3
    );
    let mut invalid = request();
    invalid.questions.clear();
    assert!(client.decision_evaluate(invalid.clone()).await.is_err());
    let server = AiccServerHandler::new(Handler(count.clone()));
    let response = server
        .handle_rpc_call(
            RPCRequest::new(
                "decision.evaluate",
                serde_json::to_value(request()).unwrap(),
            ),
            "127.0.0.1".parse().unwrap(),
        )
        .await
        .unwrap();
    assert!(matches!(response.result, RPCResult::Success(_)));
    assert!(server
        .handle_rpc_call(
            RPCRequest::new("decision.evaluate", serde_json::to_value(invalid).unwrap()),
            "127.0.0.1".parse().unwrap()
        )
        .await
        .is_err());
    assert_eq!(count.load(Ordering::SeqCst), 2);
}

#[test]
fn decision_directory_requirements_union_features_and_take_stricter_limits() {
    let mut requirement = request().requirements();
    requirement.set_feature_required("decision.structured_state");
    assert!(requirement.requires_feature("decision.structured_state"));
    let other = DecisionRequirements {
        max_options: 300,
        max_levels: 2,
        question_types: std::collections::BTreeSet::from([DecisionQuestionType::Boolean]),
        ..Default::default()
    };
    requirement.decision.as_mut().unwrap().merge(&other);
    assert_eq!(requirement.decision.as_ref().unwrap().max_options, 300);
    assert_eq!(
        requirement.decision.as_ref().unwrap().question_types.len(),
        3
    );
    assert_eq!(
        RoutingPreviewRequest::from_json(json!({"paths":["decision"],"requirements":requirement}))
            .unwrap()
            .requirements,
        Some(requirement)
    );
}
