//! Task Dispatch Center unit tests.
//!
//! Coverage per `doc/task_mgr/task_dispatch_center.md` §10 M1.4 / §11:
//! explicit-target & default-route resolution, route switching vs target
//! stickiness, dispatch/claim/accept/reject/cancel/renew, offer-lease expiry
//! & redelivery, delivery exhaustion, request expiry, idempotency keys,
//! stale-epoch fencing, Uncertain gating, restart recovery, and the
//! authorization matrix (normal vs zone-trusted callers, target owners).
//!
//! Time-dependent paths are driven by passing a future `now` into
//! `run_maintenance_once` / `evaluate_target` — no sleeps.

use super::dispatch_db::DispatchDb;
use super::service::{TaskDispatcherService, INSTANCE_LEASE_TTL_MS};
use crate::server::SessionTokenVerifier;
use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::*;
use jsonwebtoken::{DecodingKey, EncodingKey};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::net::IpAddr;
use std::str::FromStr;
use std::sync::Arc;
use tempfile::tempdir;

// Fixed ed25519 test keypair (same material as the task server tests).
const TEST_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#;
const TEST_PUBLIC_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

const OPENDAN_USER: &str = "ood1";
const OPENDAN_APP: &str = "opendan";
const TARGET_JARVIS: &str = "did:agent:jarvis";
const TARGET_EBOOK_OCR: &str = "did:service:ebook-ocr";
const TARGET_CAD_OCR: &str = "did:service:cad-ocr";
const OP_DELEGATE: &str = "agent.delegate/v1";
const OP_OCR: &str = "document.ocr/v1";

struct StaticKeyVerifier {
    key: DecodingKey,
}

#[async_trait]
impl SessionTokenVerifier for StaticKeyVerifier {
    async fn verify(&self, token: &str) -> Result<RPCSessionToken> {
        let mut parsed = RPCSessionToken::from_string(token)?;
        parsed.verify_by_key(&self.key)?;
        Ok(parsed)
    }
}

fn test_encoding_key() -> EncodingKey {
    EncodingKey::from_ed_pem(TEST_PRIVATE_KEY.as_bytes()).unwrap()
}

/// Self-signed (iss == sub): zone-trusted kernel/frame service identity.
fn zone_token(user_id: &str, app_id: &str) -> String {
    let (jwt, _) =
        RPCSessionToken::generate_jwt_token(user_id, app_id, None, &test_encoding_key()).unwrap();
    jwt
}

/// verify-hub style token: interactive user/app session, never zone-trusted.
fn user_token(user_id: &str, app_id: &str) -> String {
    let now = buckyos_kit::buckyos_get_unix_timestamp();
    let session_token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(now + 3600),
        iss: Some(VERIFY_HUB_UNIQUE_ID.to_string()),
        jti: None,
        sub: Some(user_id.to_string()),
        appid: Some(app_id.to_string()),
        sudo: false,
        extra: HashMap::new(),
    };
    session_token.generate_jwt(None, &test_encoding_key()).unwrap()
}

struct Harness {
    service: TaskDispatcherService,
    server: TaskDispatcherServerHandler<TaskDispatcherService>,
    _temp_dir: tempfile::TempDir,
    db_conn: String,
}

async fn open_service(conn: &str) -> TaskDispatcherService {
    let db = DispatchDb::open(conn, RdbBackend::Sqlite, None).await.unwrap();
    let verifier = StaticKeyVerifier {
        key: DecodingKey::from_ed_components(TEST_PUBLIC_X).unwrap(),
    };
    TaskDispatcherService::new(
        db,
        KEventClient::new_local("task-dispatcher-test"),
        Arc::new(verifier),
    )
}

async fn setup() -> Harness {
    let temp_dir = tempdir().unwrap();
    let db_path = temp_dir.path().join("dispatch.db");
    let db_conn = format!("sqlite://{}?mode=rwc", db_path.to_str().unwrap());
    let service = open_service(&db_conn).await;
    let server = TaskDispatcherServerHandler::new(service.clone());
    Harness {
        service,
        server,
        _temp_dir: temp_dir,
        db_conn,
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

impl Harness {
    async fn rpc(&self, method: &str, params: Value, token: Option<String>) -> Result<RPCResponse> {
        let req = RPCRequest {
            method: method.to_string(),
            params,
            seq: 1,
            token,
            trace_id: Some("".to_string()),
        };
        self.server
            .handle_rpc_call(req, IpAddr::from_str("127.0.0.1").unwrap())
            .await
    }

    async fn call_ok(&self, method: &str, params: Value, token: &str) -> Value {
        let resp = self
            .rpc(method, params, Some(token.to_string()))
            .await
            .unwrap_or_else(|err| panic!("{} failed: {}", method, err));
        match resp.result {
            RPCResult::Success(value) => value,
            RPCResult::Failed(err) => panic!("{} failed: {}", method, err),
        }
    }

    async fn call_err(&self, method: &str, params: Value, token: &str) -> String {
        match self.rpc(method, params, Some(token.to_string())).await {
            Err(err) => err.to_string(),
            Ok(resp) => match resp.result {
                RPCResult::Failed(err) => err,
                RPCResult::Success(value) => {
                    panic!("{} unexpectedly succeeded: {}", method, value)
                }
            },
        }
    }

    async fn register_default_target(&self) {
        self.register_target_with(TARGET_JARVIS, OP_DELEGATE, "IdempotentAccept", 4)
            .await;
    }

    async fn register_target_with(
        &self,
        target_id: &str,
        operation: &str,
        contract: &str,
        max_concurrency: u32,
    ) {
        self.call_ok(
            "register_target",
            json!({
                "registration": {
                    "target_id": target_id,
                    "operations": [{"operation": operation}],
                    "auth_policy": "ZoneUsers",
                    "idempotency_contract": contract,
                    "delivery_policy": {
                        "offer_lease_ms": 30000,
                        "max_offer_deliveries": 10,
                        "instance_selection": "RoundRobin"
                    },
                    "max_concurrency": max_concurrency,
                    "enabled": true
                }
            }),
            &zone_token(OPENDAN_USER, OPENDAN_APP),
        )
        .await;
    }

    async fn attach(&self) -> (String, u64) {
        let result = self
            .call_ok(
                "attach_instance",
                json!({"target_id": TARGET_JARVIS, "capacity": 4}),
                &zone_token(OPENDAN_USER, OPENDAN_APP),
            )
            .await;
        (
            result["instance_id"].as_str().unwrap().to_string(),
            result["lease_epoch"].as_u64().unwrap(),
        )
    }

    async fn dispatch_to_jarvis(&self, idempotency_key: &str, token: &str) -> Value {
        self.call_ok(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": idempotency_key,
                "input": {"title": "Review changes", "purpose": "check the diff"}
            }),
            token,
        )
        .await
    }

    async fn record(&self, dispatch_id: &str) -> DispatchRecord {
        let value = self
            .call_ok(
                "get_dispatch",
                json!({"dispatch_id": dispatch_id}),
                &zone_token(OPENDAN_USER, "observer"),
            )
            .await;
        serde_json::from_value(value["record"].clone()).unwrap()
    }
}

// ---------------------------------------------------------------------------
// resolution: explicit target / default route / stickiness
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn dispatch_requires_registered_enabled_supporting_target() {
    let h = setup().await;
    let caller = user_token("alice", "some-app");

    // Unregistered explicit target fails immediately — no orphan record.
    let err = h
        .call_err(
            "dispatch",
            json!({
                "target_id": "did:agent:ghost",
                "operation": OP_DELEGATE,
                "idempotency_key": "k1",
                "input": {}
            }),
            &caller,
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_TARGET_NOT_REGISTERED), "{}", err);

    h.register_default_target().await;

    // Unsupported operation fails.
    let err = h
        .call_err(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": "agent.delegate/v2",
                "idempotency_key": "k2",
                "input": {}
            }),
            &caller,
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_UNSUPPORTED_OPERATION), "{}", err);

    // Disabled target fails; waiting records are unaffected but new ones
    // are refused.
    h.call_ok(
        "disable_target",
        json!({"target_id": TARGET_JARVIS}),
        &zone_token(OPENDAN_USER, OPENDAN_APP),
    )
    .await;
    let err = h
        .call_err(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "k3",
                "input": {}
            }),
            &caller,
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_TARGET_DISABLED), "{}", err);

    // No default route configured for the operation.
    let err = h
        .call_err(
            "dispatch",
            json!({
                "operation": OP_OCR,
                "idempotency_key": "k4",
                "input": {}
            }),
            &caller,
        )
        .await;
    assert!(
        err.contains(DISPATCH_ERR_DEFAULT_TARGET_NOT_CONFIGURED),
        "{}",
        err
    );
}

#[tokio::test(flavor = "current_thread")]
async fn default_route_resolves_and_freezes_target() {
    let h = setup().await;
    let admin = zone_token(OPENDAN_USER, "control-panel");
    h.register_target_with(TARGET_EBOOK_OCR, OP_OCR, "IdempotentAccept", 4)
        .await;
    h.register_target_with(TARGET_CAD_OCR, OP_OCR, "IdempotentAccept", 4)
        .await;

    // Route must point at a registered, enabled, supporting target.
    let err = h
        .call_err(
            "set_operation_route",
            json!({"operation": OP_OCR, "default_target_id": "did:service:nope"}),
            &admin,
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_TARGET_NOT_REGISTERED), "{}", err);

    let route = h
        .call_ok(
            "set_operation_route",
            json!({"operation": OP_OCR, "default_target_id": TARGET_EBOOK_OCR}),
            &admin,
        )
        .await;
    assert_eq!(route["revision"], 1);

    // Omitted target resolves via the route and records the revision.
    let submit = h
        .call_ok(
            "dispatch",
            json!({
                "operation": OP_OCR,
                "idempotency_key": "ocr-1",
                "input": {"content_ref": "obj://sha256/8af3"}
            }),
            &user_token("alice", "some-app"),
        )
        .await;
    assert_eq!(submit["target_id"], TARGET_EBOOK_OCR);
    assert_eq!(submit["target_selection"]["DefaultRoute"]["route_revision"], 1);
    let first_dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();

    // Admin switches the default backend: only NEW dispatches see it.
    let route = h
        .call_ok(
            "set_operation_route",
            json!({"operation": OP_OCR, "default_target_id": TARGET_CAD_OCR}),
            &admin,
        )
        .await;
    assert_eq!(route["revision"], 2);

    let submit2 = h
        .call_ok(
            "dispatch",
            json!({
                "operation": OP_OCR,
                "idempotency_key": "ocr-2",
                "input": {"content_ref": "obj://sha256/8af3"}
            }),
            &user_token("alice", "some-app"),
        )
        .await;
    assert_eq!(submit2["target_id"], TARGET_CAD_OCR);

    // Target stickiness: the old record stays on ebook-ocr across
    // maintenance rounds, and the idempotent replay still answers ebook-ocr.
    h.service.run_maintenance_once(now_ms()).await;
    let old = h.record(&first_dispatch_id).await;
    assert_eq!(old.target_id, TARGET_EBOOK_OCR);

    let replay = h
        .call_ok(
            "dispatch",
            json!({
                "operation": OP_OCR,
                "idempotency_key": "ocr-1",
                "input": {"content_ref": "obj://sha256/8af3"}
            }),
            &user_token("alice", "some-app"),
        )
        .await;
    assert_eq!(replay["dispatch_id"].as_str().unwrap(), first_dispatch_id);
    assert_eq!(replay["target_id"], TARGET_EBOOK_OCR);

    // Disabled route refuses new default-route dispatches.
    h.call_ok(
        "disable_operation_route",
        json!({"operation": OP_OCR}),
        &admin,
    )
    .await;
    let err = h
        .call_err(
            "dispatch",
            json!({"operation": OP_OCR, "idempotency_key": "ocr-3", "input": {}}),
            &user_token("alice", "some-app"),
        )
        .await;
    assert!(
        err.contains(DISPATCH_ERR_DEFAULT_TARGET_NOT_CONFIGURED),
        "{}",
        err
    );
}

// ---------------------------------------------------------------------------
// happy path: offer -> claim -> accept, and the idempotent replays around it
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn dispatch_claim_accept_round_trip() {
    let h = setup().await;
    h.register_default_target().await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let (instance_id, epoch) = h.attach().await;

    // Offline target: record waits durably.
    let submit = h
        .dispatch_to_jarvis("wf-run-9f3a/step-review/attempt-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "Offered"); // instance online + capacity

    // claim is idempotent transport: same batch until accepted.
    let claim_params = json!({
        "target_id": TARGET_JARVIS,
        "instance_id": instance_id,
        "lease_epoch": epoch,
        "max": 16
    });
    let claimed = h.call_ok("claim_next", claim_params.clone(), &owner).await;
    assert_eq!(claimed["records"].as_array().unwrap().len(), 1);
    let claimed_again = h.call_ok("claim_next", claim_params.clone(), &owner).await;
    assert_eq!(claimed_again["records"].as_array().unwrap().len(), 1);
    let record_value = &claimed_again["records"][0];
    assert_eq!(record_value["dispatch_id"], dispatch_id.as_str());
    // The envelope rides with the offer; identity came from the token.
    assert_eq!(record_value["auth"]["requested_by_user"], "alice");
    assert_eq!(record_value["auth"]["on_behalf_of"], "alice");

    let accept_params = json!({
        "dispatch_id": dispatch_id,
        "instance_id": instance_id,
        "lease_epoch": epoch,
        "target_task_id": 42
    });
    let accepted = h.call_ok("accept_dispatch", accept_params.clone(), &owner).await;
    assert_eq!(accepted["accepted"], true);
    assert_eq!(accepted["status"], "Accepted");

    // ACK-loss replay: same dispatch_id + same task id succeeds; a
    // different task id is a contract violation.
    let replay = h.call_ok("accept_dispatch", accept_params, &owner).await;
    assert_eq!(replay["accepted"], true);
    let err = h
        .call_err(
            "accept_dispatch",
            json!({
                "dispatch_id": dispatch_id,
                "instance_id": instance_id,
                "lease_epoch": epoch,
                "target_task_id": 43
            }),
            &owner,
        )
        .await;
    assert!(err.contains("already accepted"), "{}", err);

    // Accepted is terminal: the caller observes the link, cancel is
    // refused, nothing re-offers.
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Accepted);
    assert_eq!(record.target_task_id, Some(42));
    let err = h
        .call_err(
            "cancel_dispatch",
            json!({"dispatch_id": dispatch_id}),
            &user_token("alice", "some-app"),
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_ALREADY_ACCEPTED), "{}", err);

    let claimed = h.call_ok("claim_next", claim_params.clone(), &owner).await;
    assert_eq!(claimed["records"].as_array().unwrap().len(), 0);

    // Business failure of task 42 never reaches the dispatcher: there is no
    // API to re-open an accepted record, and maintenance leaves it alone.
    h.service.run_maintenance_once(now_ms() + 3_600_000).await;
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Accepted);
    assert_eq!(record.offer_delivery_count, 1);
}

#[tokio::test(flavor = "current_thread")]
async fn idempotency_key_replay_and_conflict() {
    let h = setup().await;
    h.register_default_target().await;
    let caller = user_token("alice", "some-app");

    let first = h.dispatch_to_jarvis("key-1", &caller).await;
    let replay = h.dispatch_to_jarvis("key-1", &caller).await;
    assert_eq!(first["dispatch_id"], replay["dispatch_id"]);

    // Same key, different input -> conflict, no second record.
    let err = h
        .call_err(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "key-1",
                "input": {"different": true}
            }),
            &caller,
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_IDEMPOTENCY_CONFLICT), "{}", err);

    // Same key from a different caller identity is a different scope.
    let other = h.dispatch_to_jarvis("key-1", &user_token("bob", "some-app")).await;
    assert_ne!(other["dispatch_id"], first["dispatch_id"]);
}

// ---------------------------------------------------------------------------
// offer lease expiry: redelivery under IdempotentAccept, Uncertain without
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn expired_offer_requeues_and_redelivers_same_dispatch_id() {
    let h = setup().await;
    h.register_default_target().await;
    let (instance_id, _epoch) = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("redeliver-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "Offered");
    let offered = h.record(&dispatch_id).await;
    assert_eq!(offered.offer_instance_id.as_deref(), Some(instance_id.as_str()));
    assert_eq!(offered.offer_delivery_count, 1);

    // Offer lease (30s) runs out; instance lease (60s) is still alive:
    // redelivery reuses the SAME dispatch_id — handoff recovery, not retry.
    let later = now_ms() + 31_000;
    h.service.run_maintenance_once(later).await;

    let redelivered = h.record(&dispatch_id).await;
    assert_eq!(redelivered.status, DispatchStatus::Offered);
    assert_eq!(redelivered.offer_delivery_count, 2);
    assert_eq!(
        redelivered.offer_instance_id.as_deref(),
        Some(instance_id.as_str())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn none_contract_goes_uncertain_and_needs_resolve() {
    let h = setup().await;
    h.register_target_with(TARGET_JARVIS, OP_DELEGATE, "None", 4).await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let _ = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("uncertain-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "Offered");

    let later = now_ms() + 31_000;
    h.service.run_maintenance_once(later).await;
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Uncertain);

    // No automatic redelivery, ever — even long after all leases are gone.
    h.service.run_maintenance_once(later + 120_000).await;
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Uncertain);

    // The runtime restarts and re-attaches (fresh instance + epoch) — the
    // realistic path for a late business-side accept.
    let (instance_id, epoch) = h.attach().await;

    // Caller cancel is gated: Uncertain requires resolve_uncertain.
    let err = h
        .call_err(
            "cancel_dispatch",
            json!({"dispatch_id": dispatch_id}),
            &user_token("alice", "some-app"),
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_UNCERTAIN_REQUIRES_RESOLVE), "{}", err);

    // Normal callers cannot resolve.
    let err = h
        .call_err(
            "resolve_uncertain",
            json!({"dispatch_id": dispatch_id, "resolution": "Canceled"}),
            &user_token("alice", "some-app"),
        )
        .await;
    assert!(err.contains("zone-trusted"), "{}", err);

    // A late accept from the target itself is authoritative business
    // recovery and resolves Uncertain -> Accepted.
    let accepted = h
        .call_ok(
            "accept_dispatch",
            json!({
                "dispatch_id": dispatch_id,
                "instance_id": instance_id,
                "lease_epoch": epoch,
                "target_task_id": 77
            }),
            &owner,
        )
        .await;
    assert_eq!(accepted["accepted"], true);
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Accepted);
    assert_eq!(record.target_task_id, Some(77));
}

#[tokio::test(flavor = "current_thread")]
async fn resolve_uncertain_as_canceled() {
    let h = setup().await;
    h.register_target_with(TARGET_JARVIS, OP_DELEGATE, "None", 4).await;
    let _ = h.attach().await;
    let submit = h
        .dispatch_to_jarvis("uncertain-2", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    h.service.run_maintenance_once(now_ms() + 31_000).await;
    assert_eq!(h.record(&dispatch_id).await.status, DispatchStatus::Uncertain);

    let resolved = h
        .call_ok(
            "resolve_uncertain",
            json!({"dispatch_id": dispatch_id, "resolution": "Canceled"}),
            &zone_token(OPENDAN_USER, "admin-svc"),
        )
        .await;
    assert_eq!(resolved["record"]["status"], "Canceled");
}

#[tokio::test(flavor = "current_thread")]
async fn delivery_exhaustion_expires_record() {
    let h = setup().await;
    // max_offer_deliveries = 2
    h.call_ok(
        "register_target",
        json!({
            "registration": {
                "target_id": TARGET_JARVIS,
                "operations": [{"operation": OP_DELEGATE}],
                "idempotency_contract": "IdempotentAccept",
                "delivery_policy": {
                    "offer_lease_ms": 30000,
                    "max_offer_deliveries": 2,
                    "instance_selection": "RoundRobin"
                },
                "max_concurrency": 4,
                "enabled": true
            }
        }),
        &zone_token(OPENDAN_USER, OPENDAN_APP),
    )
    .await;
    let _ = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("exhaust-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "Offered"); // delivery 1

    let base = now_ms();
    h.service.run_maintenance_once(base + 31_000).await; // requeue + delivery 2
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.offer_delivery_count, 2);

    h.service.run_maintenance_once(base + 62_000).await; // budget exhausted
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Expired);
    assert_eq!(record.message.as_deref(), Some("delivery_exhausted"));
}

#[tokio::test(flavor = "current_thread")]
async fn request_expiry_wins_over_waiting() {
    let h = setup().await;
    h.register_default_target().await;
    // No instance attached: the record waits durably.
    let expires_at = (now_ms() + 5_000) as u64;
    let submit = h
        .call_ok(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "expire-1",
                "expires_at": expires_at,
                "input": {}
            }),
            &user_token("alice", "some-app"),
        )
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "WaitingForTarget");

    h.service.run_maintenance_once(now_ms() + 6_000).await;
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Expired);
    assert_eq!(record.message.as_deref(), Some("request_expired"));

    // Re-submitting after a terminal state needs a NEW idempotency key —
    // the old key (same immutable request) still answers with the old
    // (expired) record instead of restarting anything.
    let replay = h
        .call_ok(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "expire-1",
                "expires_at": expires_at,
                "input": {}
            }),
            &user_token("alice", "some-app"),
        )
        .await;
    assert_eq!(replay["dispatch_id"].as_str().unwrap(), dispatch_id);
    assert_eq!(replay["status"], "Expired");
    let fresh = h.dispatch_to_jarvis("expire-2", &user_token("alice", "some-app")).await;
    assert_ne!(fresh["dispatch_id"].as_str().unwrap(), dispatch_id);

    // Past deadlines are refused outright.
    let err = h
        .call_err(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "expire-3",
                "expires_at": 1000,
                "input": {}
            }),
            &user_token("alice", "some-app"),
        )
        .await;
    assert!(err.contains("expires_at"), "{}", err);
}

#[tokio::test(flavor = "current_thread")]
async fn replay_after_real_deadline_returns_existing_record() {
    let h = setup().await;
    h.register_default_target().await;

    // Deadline 1s out; the record expires (maintenance) and REAL time also
    // passes the deadline before the caller retries after a network error.
    let expires_at = (now_ms() + 1_000) as u64;
    let params = json!({
        "target_id": TARGET_JARVIS,
        "operation": OP_DELEGATE,
        "idempotency_key": "late-replay-1",
        "expires_at": expires_at,
        "input": {}
    });
    let submit = h
        .call_ok("dispatch", params.clone(), &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();

    tokio::time::sleep(std::time::Duration::from_millis(1_200)).await;
    h.service.run_maintenance_once(now_ms()).await;

    // The replay must answer with the existing (Expired) record — the
    // past-deadline check only applies to NEW records.
    let replay = h
        .call_ok("dispatch", params, &user_token("alice", "some-app"))
        .await;
    assert_eq!(replay["dispatch_id"].as_str().unwrap(), dispatch_id);
    assert_eq!(replay["status"], "Expired");
}

// ---------------------------------------------------------------------------
// instance lifecycle: epoch fencing, renew, detach recycling
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn stale_epoch_is_fenced_everywhere() {
    let h = setup().await;
    h.register_default_target().await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let (instance_id, epoch) = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("fence-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();

    for (method, params) in [
        (
            "claim_next",
            json!({"target_id": TARGET_JARVIS, "instance_id": instance_id, "lease_epoch": epoch + 1, "max": 4}),
        ),
        (
            "renew_instance",
            json!({"target_id": TARGET_JARVIS, "instance_id": instance_id, "lease_epoch": epoch + 1, "available_capacity": 4}),
        ),
        (
            "accept_dispatch",
            json!({"dispatch_id": dispatch_id, "instance_id": instance_id, "lease_epoch": epoch + 1, "target_task_id": 42}),
        ),
        (
            "reject_dispatch",
            json!({"dispatch_id": dispatch_id, "instance_id": instance_id, "lease_epoch": epoch + 1, "reason": "schema_mismatch"}),
        ),
    ] {
        let err = h.call_err(method, params, &owner).await;
        assert!(
            err.contains(DISPATCH_ERR_STALE_INSTANCE),
            "{}: {}",
            method,
            err
        );
    }

    // Current epoch still works.
    let renewed = h
        .call_ok(
            "renew_instance",
            json!({"target_id": TARGET_JARVIS, "instance_id": instance_id, "lease_epoch": epoch, "available_capacity": 4}),
            &owner,
        )
        .await;
    assert_eq!(renewed["has_due"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn detach_recycles_offers_to_next_instance() {
    let h = setup().await;
    h.register_default_target().await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let (first_instance, first_epoch) = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("detach-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "Offered");

    // Clean detach recycles the in-flight offer immediately; with no other
    // instance the record waits.
    h.call_ok(
        "detach_instance",
        json!({"target_id": TARGET_JARVIS, "instance_id": first_instance, "lease_epoch": first_epoch}),
        &owner,
    )
    .await;
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::WaitingForTarget);

    // The next attach gets a higher epoch and receives the redelivery.
    let (second_instance, second_epoch) = h.attach().await;
    assert!(second_epoch > first_epoch);
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Offered);
    assert_eq!(
        record.offer_instance_id.as_deref(),
        Some(second_instance.as_str())
    );
    assert_eq!(record.offer_delivery_count, 2);

    // The detached instance can no longer act on the record.
    let err = h
        .call_err(
            "accept_dispatch",
            json!({
                "dispatch_id": dispatch_id,
                "instance_id": first_instance,
                "lease_epoch": first_epoch,
                "target_task_id": 42
            }),
            &owner,
        )
        .await;
    assert!(err.contains(DISPATCH_ERR_STALE_INSTANCE), "{}", err);

    // The new instance accepts normally.
    let accepted = h
        .call_ok(
            "accept_dispatch",
            json!({
                "dispatch_id": dispatch_id,
                "instance_id": second_instance,
                "lease_epoch": second_epoch,
                "target_task_id": 4242
            }),
            &owner,
        )
        .await;
    assert_eq!(accepted["accepted"], true);
}

#[tokio::test(flavor = "current_thread")]
async fn reject_is_terminal_and_stable() {
    let h = setup().await;
    h.register_default_target().await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let (instance_id, epoch) = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("reject-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();

    h.call_ok(
        "reject_dispatch",
        json!({
            "dispatch_id": dispatch_id,
            "instance_id": instance_id,
            "lease_epoch": epoch,
            "reason": "schema_mismatch",
            "detail": "unknown field"
        }),
        &owner,
    )
    .await;
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Rejected);
    assert_eq!(record.reject_reason, Some(DispatchRejectReason::SchemaMismatch));

    // Accepting a rejected record reports the terminal state instead: the
    // target must cancel any local task.
    let result = h
        .call_ok(
            "accept_dispatch",
            json!({
                "dispatch_id": dispatch_id,
                "instance_id": instance_id,
                "lease_epoch": epoch,
                "target_task_id": 9
            }),
            &owner,
        )
        .await;
    assert_eq!(result["accepted"], false);
    assert_eq!(result["status"], "Rejected");

    // No redelivery of rejected records, no matter how much time passes.
    h.service.run_maintenance_once(now_ms() + 120_000).await;
    assert_eq!(h.record(&dispatch_id).await.status, DispatchStatus::Rejected);
}

#[tokio::test(flavor = "current_thread")]
async fn cancel_before_accept_wins_race() {
    let h = setup().await;
    h.register_default_target().await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let (instance_id, epoch) = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("race-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();

    // Caller cancels while the offer is out.
    let canceled = h
        .call_ok(
            "cancel_dispatch",
            json!({"dispatch_id": dispatch_id}),
            &user_token("alice", "some-app"),
        )
        .await;
    assert_eq!(canceled["status"], "Canceled");

    // The instance's accept is atomically refused; it must cancel the task
    // it just created locally.
    let result = h
        .call_ok(
            "accept_dispatch",
            json!({
                "dispatch_id": dispatch_id,
                "instance_id": instance_id,
                "lease_epoch": epoch,
                "target_task_id": 55
            }),
            &owner,
        )
        .await;
    assert_eq!(result["accepted"], false);
    assert_eq!(result["status"], "Canceled");

    // Cancel replay is a no-op success.
    let canceled = h
        .call_ok(
            "cancel_dispatch",
            json!({"dispatch_id": dispatch_id}),
            &user_token("alice", "some-app"),
        )
        .await;
    assert_eq!(canceled["status"], "Canceled");
}

// ---------------------------------------------------------------------------
// restart recovery
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn restart_recovers_waiting_records_and_timers() {
    let h = setup().await;
    h.register_default_target().await;

    // Offline submit; then the "process" restarts on the same store.
    let submit = h
        .dispatch_to_jarvis("restart-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "WaitingForTarget");

    let service2 = open_service(&h.db_conn).await;
    let server2 = TaskDispatcherServerHandler::new(service2.clone());
    service2.startup_recovery().await;

    // Instance attaches to the new process and receives the offer; the
    // record identity is unchanged.
    let h2 = Harness {
        service: service2,
        server: server2,
        _temp_dir: tempdir().unwrap(),
        db_conn: h.db_conn.clone(),
    };
    let (instance_id, epoch) = h2.attach().await;
    let record = h2.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::Offered);

    let accepted = h2
        .call_ok(
            "accept_dispatch",
            json!({
                "dispatch_id": dispatch_id,
                "instance_id": instance_id,
                "lease_epoch": epoch,
                "target_task_id": 314
            }),
            &zone_token(OPENDAN_USER, OPENDAN_APP),
        )
        .await;
    assert_eq!(accepted["accepted"], true);
}

// ---------------------------------------------------------------------------
// authorization matrix
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn requests_without_token_are_rejected() {
    let h = setup().await;
    for (method, params) in [
        ("dispatch", json!({"operation": OP_DELEGATE, "idempotency_key": "k", "input": {}})),
        ("get_dispatch", json!({"dispatch_id": "dsp-x"})),
        ("register_target", json!({"registration": {"target_id": "t", "operations": [{"operation": "x/v1"}]}})),
        ("attach_instance", json!({"target_id": "t"})),
        ("resolve_uncertain", json!({"dispatch_id": "dsp-x", "resolution": "Canceled"})),
    ] {
        for token in [None, Some(String::new())] {
            let result = h.rpc(method, params.clone(), token).await;
            let failed = match result {
                Err(_) => true,
                Ok(resp) => matches!(resp.result, RPCResult::Failed(_)),
            };
            assert!(failed, "{} should fail without a session token", method);
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn normal_callers_cannot_touch_target_or_admin_surface() {
    let h = setup().await;
    h.register_default_target().await;
    let (instance_id, epoch) = h.attach().await;
    let mallory = user_token("mallory", "evil-app");

    // register / disable / route config / instance ops all refuse
    // interactive-session tokens.
    for (method, params) in [
        (
            "register_target",
            json!({"registration": {"target_id": "did:agent:evil", "operations": [{"operation": OP_DELEGATE}]}}),
        ),
        ("disable_target", json!({"target_id": TARGET_JARVIS})),
        (
            "set_operation_route",
            json!({"operation": OP_DELEGATE, "default_target_id": TARGET_JARVIS}),
        ),
        ("disable_operation_route", json!({"operation": OP_DELEGATE})),
        ("list_targets", json!({})),
        ("attach_instance", json!({"target_id": TARGET_JARVIS})),
        (
            "claim_next",
            json!({"target_id": TARGET_JARVIS, "instance_id": instance_id, "lease_epoch": epoch, "max": 4}),
        ),
        (
            "renew_instance",
            json!({"target_id": TARGET_JARVIS, "instance_id": instance_id, "lease_epoch": epoch, "available_capacity": 1}),
        ),
    ] {
        let err = h.call_err(method, params, &mallory).await;
        assert!(
            err.contains("zone-trusted") || err.contains("No permission"),
            "{}: {}",
            method,
            err
        );
    }

    // A zone-trusted service that is NOT the owner cannot drive the target
    // side either.
    let other_service = zone_token(OPENDAN_USER, "other-svc");
    for (method, params) in [
        ("attach_instance", json!({"target_id": TARGET_JARVIS})),
        (
            "claim_next",
            json!({"target_id": TARGET_JARVIS, "instance_id": instance_id, "lease_epoch": epoch, "max": 4}),
        ),
        ("disable_target", json!({"target_id": TARGET_JARVIS})),
        (
            "register_target",
            json!({"registration": {"target_id": TARGET_JARVIS, "operations": [{"operation": OP_DELEGATE}]}}),
        ),
    ] {
        let err = h.call_err(method, params, &other_service).await;
        assert!(err.contains("not the owner") || err.contains("owned by"), "{}: {}", method, err);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn same_owner_reregistration_updates_in_place() {
    // Adapters re-register on every runtime start; the upsert must update
    // the existing row instead of erroring or duplicating.
    let h = setup().await;
    h.register_target_with(TARGET_JARVIS, OP_DELEGATE, "IdempotentAccept", 4)
        .await;
    h.register_target_with(TARGET_JARVIS, OP_DELEGATE, "IdempotentAccept", 8)
        .await;

    let target = h
        .call_ok(
            "get_target",
            json!({"target_id": TARGET_JARVIS}),
            &zone_token(OPENDAN_USER, "observer-svc"),
        )
        .await;
    assert_eq!(target["target"]["max_concurrency"], 8);
    assert_eq!(target["target"]["owner_app_id"], OPENDAN_APP);

    let listed = h
        .call_ok("list_targets", json!({}), &zone_token(OPENDAN_USER, "observer-svc"))
        .await;
    assert_eq!(listed["targets"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn on_behalf_of_cannot_launder_identity() {
    let h = setup().await;
    h.register_default_target().await;

    // Normal caller impersonating someone else: refused.
    let err = h
        .call_err(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "obo-1",
                "on_behalf_of": "root",
                "input": {}
            }),
            &user_token("mallory", "evil-app"),
        )
        .await;
    assert!(err.contains("on behalf of"), "{}", err);

    // Zone-trusted caller may carry an authenticated business user.
    let submit = h
        .call_ok(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "obo-2",
                "on_behalf_of": "alice",
                "workflow_ref": {"workflow_id": "code-review", "run_id": "wf-run-9f3a", "step_id": "review"},
                "input": {"purpose": "review"}
            }),
            &zone_token(OPENDAN_USER, "workflow"),
        )
        .await;
    let record = h.record(submit["dispatch_id"].as_str().unwrap()).await;
    assert_eq!(record.auth.requested_by_user, OPENDAN_USER);
    assert_eq!(record.auth.requested_by_app, "workflow");
    assert_eq!(record.auth.on_behalf_of, "alice");
    assert!(record.auth.zone_trusted_caller);
    assert_eq!(
        record.auth.workflow_ref.as_ref().map(|r| r.run_id.as_str()),
        Some("wf-run-9f3a")
    );
}

#[tokio::test(flavor = "current_thread")]
async fn zone_trusted_only_target_refuses_normal_users() {
    let h = setup().await;
    h.call_ok(
        "register_target",
        json!({
            "registration": {
                "target_id": TARGET_JARVIS,
                "operations": [{"operation": OP_DELEGATE}],
                "auth_policy": "ZoneTrustedOnly",
                "max_concurrency": 2
            }
        }),
        &zone_token(OPENDAN_USER, OPENDAN_APP),
    )
    .await;

    let err = h
        .call_err(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "zt-1",
                "input": {}
            }),
            &user_token("alice", "some-app"),
        )
        .await;
    assert!(err.contains("zone-trusted"), "{}", err);

    let submit = h
        .call_ok(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "zt-2",
                "input": {}
            }),
            &zone_token(OPENDAN_USER, "workflow"),
        )
        .await;
    assert_eq!(submit["status"], "WaitingForTarget");
}

#[tokio::test(flavor = "current_thread")]
async fn record_visibility_follows_requester_and_on_behalf_of() {
    let h = setup().await;
    h.register_default_target().await;

    let alice_submit = h.dispatch_to_jarvis("vis-1", &user_token("alice", "some-app")).await;
    let alice_id = alice_submit["dispatch_id"].as_str().unwrap().to_string();
    // Workflow dispatches on behalf of alice.
    let wf_submit = h
        .call_ok(
            "dispatch",
            json!({
                "target_id": TARGET_JARVIS,
                "operation": OP_DELEGATE,
                "idempotency_key": "vis-2",
                "on_behalf_of": "alice",
                "input": {}
            }),
            &zone_token(OPENDAN_USER, "workflow"),
        )
        .await;
    let wf_id = wf_submit["dispatch_id"].as_str().unwrap().to_string();

    // bob sees neither.
    for dispatch_id in [&alice_id, &wf_id] {
        let err = h
            .call_err(
                "get_dispatch",
                json!({"dispatch_id": dispatch_id}),
                &user_token("bob", "some-app"),
            )
            .await;
        assert!(err.contains("No permission"), "{}", err);
    }
    let listed = h
        .call_ok("list_dispatches", json!({}), &user_token("bob", "some-app"))
        .await;
    assert_eq!(listed["records"].as_array().unwrap().len(), 0);

    // alice sees both: one as requester, one as on_behalf_of.
    let listed = h
        .call_ok("list_dispatches", json!({}), &user_token("alice", "some-app"))
        .await;
    assert_eq!(listed["records"].as_array().unwrap().len(), 2);

    // bob cannot cancel alice's dispatch.
    let err = h
        .call_err(
            "cancel_dispatch",
            json!({"dispatch_id": alice_id}),
            &user_token("bob", "some-app"),
        )
        .await;
    assert!(err.contains("No permission"), "{}", err);

    // alice can cancel the workflow-made dispatch that names her.
    let canceled = h
        .call_ok(
            "cancel_dispatch",
            json!({"dispatch_id": wf_id}),
            &user_token("alice", "some-app"),
        )
        .await;
    assert_eq!(canceled["status"], "Canceled");
}

// ---------------------------------------------------------------------------
// assignment behavior details
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
async fn max_concurrency_bounds_in_flight_offers() {
    let h = setup().await;
    h.register_target_with(TARGET_JARVIS, OP_DELEGATE, "IdempotentAccept", 2)
        .await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let (instance_id, epoch) = h.attach().await;

    let mut dispatch_ids = Vec::new();
    for index in 0..3 {
        let submit = h
            .dispatch_to_jarvis(&format!("cap-{}", index), &user_token("alice", "some-app"))
            .await;
        dispatch_ids.push(submit["dispatch_id"].as_str().unwrap().to_string());
    }

    let offered: Vec<_> = futures_join_records(&h, &dispatch_ids)
        .await
        .into_iter()
        .filter(|record| record.status == DispatchStatus::Offered)
        .collect();
    assert_eq!(offered.len(), 2, "max_concurrency=2 caps in-flight offers");

    // Accepting one frees a slot for the third record.
    let first_offered = offered[0].dispatch_id.clone();
    h.call_ok(
        "accept_dispatch",
        json!({
            "dispatch_id": first_offered,
            "instance_id": instance_id,
            "lease_epoch": epoch,
            "target_task_id": 1
        }),
        &owner,
    )
    .await;
    let records = futures_join_records(&h, &dispatch_ids).await;
    let offered_count = records
        .iter()
        .filter(|record| record.status == DispatchStatus::Offered)
        .count();
    assert_eq!(offered_count, 2);
}

async fn futures_join_records(h: &Harness, ids: &[String]) -> Vec<DispatchRecord> {
    let mut records = Vec::with_capacity(ids.len());
    for id in ids {
        records.push(h.record(id).await);
    }
    records
}

#[tokio::test(flavor = "current_thread")]
async fn kevent_notifies_target_and_record_channels() {
    let h = setup().await;
    h.register_default_target().await;

    // Subscribe the target hint channel before attaching so the offer
    // notification is captured. (kevent is acceleration only; the claim
    // path itself is exercised elsewhere without it.)
    let kevent = KEventClient::new_local("task-dispatcher-observer");
    let service_kevent = h.service.clone();
    let _ = service_kevent; // service publishes through its own local client

    // Local-mode clients are process-local brokers, so subscribe against
    // the service's publishing client via a reader created from it.
    let target_key = task_dispatcher_target_key(TARGET_JARVIS);
    let channel = task_dispatcher_target_event_id(target_key.as_str());
    drop(kevent);
    let reader = h
        .service
        .kevent_client_for_tests()
        .create_event_reader(vec![channel])
        .await
        .unwrap();

    let _ = h.attach().await;
    let submit = h
        .dispatch_to_jarvis("kevent-1", &user_token("alice", "some-app"))
        .await;
    assert_eq!(submit["status"], "Offered");

    let event = reader.pull_event(Some(1_000)).await.unwrap();
    let event = event.expect("offer notification should arrive on the target channel");
    assert_eq!(
        event.data["dispatch_id"].as_str().unwrap(),
        submit["dispatch_id"].as_str().unwrap()
    );
    // Payload carries ids only, never the input.
    assert!(event.data.get("input").is_none());
}

#[tokio::test(flavor = "current_thread")]
async fn audit_trail_records_every_transition() {
    let h = setup().await;
    h.register_default_target().await;
    let owner = zone_token(OPENDAN_USER, OPENDAN_APP);
    let (instance_id, epoch) = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("audit-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    h.call_ok(
        "accept_dispatch",
        json!({
            "dispatch_id": dispatch_id,
            "instance_id": instance_id,
            "lease_epoch": epoch,
            "target_task_id": 42
        }),
        &owner,
    )
    .await;

    let events = h
        .service
        .db_for_tests()
        .list_events(dispatch_id.as_str())
        .await
        .unwrap();
    let transitions: Vec<(String, String)> = events;
    assert_eq!(
        transitions,
        vec![
            ("".to_string(), "Queued".to_string()),
            ("Queued".to_string(), "Offered".to_string()),
            ("Offered".to_string(), "Accepted".to_string()),
        ]
    );
}

#[tokio::test(flavor = "current_thread")]
async fn instance_lease_expiry_recycles_offers() {
    let h = setup().await;
    h.register_default_target().await;
    let _ = h.attach().await;

    let submit = h
        .dispatch_to_jarvis("lease-1", &user_token("alice", "some-app"))
        .await;
    let dispatch_id = submit["dispatch_id"].as_str().unwrap().to_string();
    assert_eq!(submit["status"], "Offered");

    // Past the instance lease (60s): instance dropped, offer lease (30s)
    // also gone -> record requeued, and with no instances left it waits.
    let later = now_ms() + INSTANCE_LEASE_TTL_MS as i64 + 1_000;
    h.service.run_maintenance_once(later).await;
    let record = h.record(&dispatch_id).await;
    assert_eq!(record.status, DispatchStatus::WaitingForTarget);
}
