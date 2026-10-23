//! The executor lease (内核架构改进 D15, M3): a claimed run whose executor stops heartbeating is interrupted
//! when it is next read, and only an explicit new claim (attempt) continues it.

mod common;

use common::*;
use serde_json::json;

#[test]
fn a_lapsed_lease_interrupts_the_run_and_a_new_claim_resumes_it() {
    let env = env();
    let id = project(&env);
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    let claim = "ct_test";
    let (run_id, _) = ws
        .wish_run_create_with(&alice(), "wish.xllm@1", &json!({ "wish_id": "w", "stage": "execute" }), "k1", &json!({ "mode": "workspace_owner", "user_id": "alice" }),
            &json!({ "exec": { "mode": "local", "attempt": 0, "claim_hash": aiworkspace_core::canonical::sha256_hex(claim.as_bytes()) } }))
        .unwrap();
    assert_eq!(ws.wish_run_claim(&run_id, "ct_wrong", &json!({})).unwrap_err().code.as_str(), "PERMISSION_DENIED");
    let first = ws.wish_run_claim(&run_id, claim, &json!({ "principal": "alice" })).unwrap();
    assert_eq!(first["attempt"], 1);
    let run = ws.wish_run(&alice(), &run_id).unwrap();
    assert_eq!(run["state"], "running");
    assert!(!ws.wish_run_expire_lease(&run).unwrap(), "inside the lease");
    env.advance(60_000);
    ws.wish_run_heartbeat(&run_id, 1, None).unwrap();
    env.advance(60_000);
    let run = ws.wish_run(&alice(), &run_id).unwrap();
    assert!(!ws.wish_run_expire_lease(&run).unwrap(), "the heartbeat moved the lease");
    env.advance(120_000);
    let run = ws.wish_run(&alice(), &run_id).unwrap();
    assert!(ws.wish_run_expire_lease(&run).unwrap(), "no heartbeat: interrupted");
    let run = ws.wish_run(&alice(), &run_id).unwrap();
    assert_eq!((run["state"].as_str(), run["detail"]["error"]["code"].as_str()), (Some("interrupted"), Some("INTERRUPTED")));
    // the old attempt may not report any more; a new claim continues
    assert_eq!(ws.wish_run_heartbeat(&run_id, 1, None).unwrap_err().code.as_str(), "RUN_REVOKED");
    let second = ws.wish_run_claim(&run_id, claim, &json!({ "principal": "alice" })).unwrap();
    assert_eq!(second["attempt"], 2);
    assert!(ws.wish_run_heartbeat(&run_id, 2, None).is_ok());
    let (by_token, attempt) = ws.wish_run_by_token(first["run_token"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!((by_token["run_id"].as_str(), attempt), (Some(run_id.as_str()), 1));
    assert_eq!(ws.wish_run_check_attempt(&by_token, 1).unwrap_err().code.as_str(), "RUN_REVOKED");
}
