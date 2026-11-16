//! Data sources on a real Workspace (AI Native 余下任务 §7.2): a pending source answers DATA_PENDING to SQL; a refresh
//! writes its data version, the same content again writes nothing; the last attempt and the freshness follow the
//! clock; a wish's write is not guarded by the sources it read (B7-2).

mod common;
use aiworkspace_store::source::Fetched;
use aiworkspace_store::wish::readset::{guarded, ReadSet};
use common::*;
use serde_json::json;

#[test]
fn a_source_from_pending_to_valid_to_expired() {
    let env = env();
    let id = env.svc.create_workspace(&alice(), "q", None).unwrap()["workspace_id"].as_str().unwrap().to_string();
    let h = env.svc.workspace(&id).unwrap();
    let mut ws = h.lock().unwrap();
    ok(&mut ws, &alice(), json!([
        { "op": "entity.create", "entity_id": "quotes", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "a", "name": "quotes",
          "payload": { "fields": [{ "field_id": "symbol", "name": "Symbol", "type": "text" }],
                       "source": { "kind": "zone", "path": "/home/alice/q.json", "def": { "meaning": "symbols", "primary_key": ["symbol"], "freshness": { "max_age_s": 60 } } } } },
        { "op": "entity.create", "entity_id": "own", "type_id": "buckyos.table-source", "parent_id": "data", "order_key": "b", "name": "own",
          "payload": { "fields": [{ "field_id": "n", "name": "N", "type": "number" }] } },
    ]));
    assert_eq!(sql(&ws, &alice(), "SELECT * FROM quotes").unwrap_err().sub, Some("DATA_PENDING"));
    // B7-2: a wish's write is guarded by the inputs people edit, not by the sources
    let mut reads = ReadSet::default();
    ws.read_ctx(|ctx| {
        reads.table(ctx, "quotes", None).unwrap();
        reads.table(ctx, "own", None).unwrap();
    });
    let inputs = ws.read_ctx(|ctx| reads.to_inputs(ctx)).unwrap();
    let kept = ws.read_ctx(|ctx| guarded(ctx, &inputs));
    assert!(!kept.is_empty() && kept.iter().all(|i| i["entity_id"] == "own"), "{kept:?}");
    // a refresh: the data and its version in one Commit; the same again writes nothing
    let fetched = Fetched::Table { fields: vec![json!({ "field_id": "symbol", "name": "Symbol", "type": "text" })],
                                   records: vec![json!({ "record_id": "k1", "values": { "symbol": "AAPL" } })] };
    let run = ws.source_refresh_start(&alice(), "quotes", "r1").unwrap()["run_id"].as_str().unwrap().to_string();
    let done = ws.source_apply(&run, &fetched, 10, None, &json!({ "program": { "name": "q", "version": "1" } })).unwrap();
    assert_eq!(done["result"]["status"], "accepted", "{done}");
    let m = meta(&ws, &alice(), "quotes");
    assert_eq!((m["data"]["state"].as_str(), m["data"]["version"]["records"].as_u64(), m["data"]["attempt"]["status"].as_str(), m["data"]["expired"].as_bool()),
               (Some("valid"), Some(1), Some("succeeded"), Some(false)), "{m}");
    let head = ws.head_seq;
    env.advance(30_000);
    let run = ws.source_refresh_start(&alice(), "quotes", "r2").unwrap()["run_id"].as_str().unwrap().to_string();
    assert_eq!(ws.source_apply(&run, &fetched, 10, None, &json!({ "program": { "name": "q", "version": "1" } })).unwrap()["result"]["status"], "unchanged");
    assert_eq!(ws.head_seq, head);
    // the last successful check counts for freshness; a minute past it, the data is expired
    env.advance(45_000);
    assert_eq!(meta(&ws, &alice(), "quotes")["data"]["expired"], false);
    env.advance(30_000);
    assert_eq!(meta(&ws, &alice(), "quotes")["data"]["expired"], true);
    let run = ws.source_refresh_start(&alice(), "quotes", "r3").unwrap()["run_id"].as_str().unwrap().to_string();
    ws.source_fail(&run, &aiworkspace_core::WsError::new(aiworkspace_core::Code::DependencyUnavailable, "gone")).unwrap();
    let m = meta(&ws, &alice(), "quotes");
    assert_eq!((m["data"]["attempt"]["status"].as_str(), m["data"]["attempt"]["error"]["detail"].as_str(), m["data"]["state"].as_str()), (Some("failed"), Some("gone"), Some("valid")), "{m}");
}
