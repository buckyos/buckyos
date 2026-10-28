//! Wish `control` results (脑图 TODO M3, R11) end to end on the service with a scripted model: a mind map written
//! as a Markdown outline lands as a container with one node per item, an invalid kind or an empty tree is refused
//! in the run, an overwrite re-run (here from a program) replaces the container in place, and a wish whose input is
//! a mind map node reads its branch as an outline and expands it below that node.

#[path = "common/wishkit.rs"]
mod wishkit;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use wishkit::*;

const PLAN_MD: &str = "# Launch plan\n\n## Market\n- interviews\n- **competitors**\n\n## Product\n1. MVP\n2. Pricing\n";

async fn setup(srv: &mut Srv) {
    let ws = srv.rpc("ws.create", json!({ "title": "脑图" })).await;
    srv.ws = ws["workspace_id"].as_str().unwrap().to_string();
    let auto = json!({ "x": "auto", "y": "auto", "w": "auto", "h": "auto" });
    let node = |id: &str, parent: &str, key: &str, title: &str| {
        json!({ "op": "entity.create", "entity_id": id, "type_id": "buckyos.cell", "parent_id": parent, "order_key": key, "placement": auto,
                "payload": { "view": { "type": "mindnode" }, "title": title } })
    };
    srv.commit(json!([
        { "op": "entity.create", "entity_id": "sf-content", "type_id": "buckyos.container", "parent_id": "canvas-content", "order_key": "s",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "sf", "title": "Board" } },
        { "op": "entity.create", "entity_id": "sf", "type_id": "buckyos.container", "parent_id": "surfaces", "order_key": "s", "name": "Board",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "title": "Board", "content_folder_id": "sf-content" } },
        { "op": "entity.create", "entity_id": "wish-mm", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "w",
          "payload": { "title": "Plan", "prompt": "Brainstorm a launch plan as a mind map", "executor": "xllm", "output_mode": "overwrite",
                       "output": { "container_id": "sf-content", "surface_id": "sf", "name": "Plan" } } },
        { "op": "entity.create", "entity_id": "blk-wish", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "d", "placement": { "x": 40, "y": 40, "w": 360, "h": 240 },
          "payload": { "view": { "type": "wish" }, "source_ref": { "entity_id": "wish-mm" } } },
        // a mind map someone drew, and a wish that expands one of its branches
        { "op": "entity.create", "entity_id": "mm", "type_id": "buckyos.container", "parent_id": "sf", "order_key": "m",
          "placement": { "x": 40, "y": 800, "w": "auto", "h": "auto" }, "payload": { "kind": "mindmap", "title": "Launch" } },
        node("n-root", "mm", "a", "Launch"),
        node("n-market", "n-root", "a", "Market"),
        node("n-int", "n-market", "a", "interviews"),
        node("n-product", "n-root", "b", "Product"),
        { "op": "entity.create", "entity_id": "wish-ex", "type_id": "buckyos.wish", "parent_id": "sf-content", "order_key": "x",
          "payload": { "title": "Expand", "prompt": "Expand the Market branch with more ideas", "executor": "xllm", "output_mode": "overwrite",
                       "output": { "container_id": "sf-content", "surface_id": "sf", "name": "Expand" } } },
        { "op": "entity.create", "entity_id": "blk-ex", "type_id": "buckyos.cell", "parent_id": "sf", "order_key": "e", "placement": { "x": 600, "y": 800, "w": 360, "h": 240 },
          "payload": { "view": { "type": "wish" }, "source_ref": { "entity_id": "wish-ex" } } },
    ]))
    .await;
}

/// The live Blocks under `id` as an outline tree (`[{ text, children }]`, sibling order).
async fn tree(srv: &Srv, id: &str) -> Value {
    let all = srv.rpc("doc.outline", json!({})).await["entities"].as_array().cloned().unwrap();
    fn under(all: &[Value], id: &str) -> Value {
        let mut kids: Vec<&Value> = all.iter().filter(|e| e["parent_id"] == json!(id) && e["deleted"] != json!(true)).collect();
        kids.sort_by_key(|e| (e["order_key"].as_str().unwrap_or("").to_string(), e["entity_id"].as_str().unwrap_or("").to_string()));
        json!(kids.iter().map(|e| json!({ "text": e["title"], "children": under(all, e["entity_id"].as_str().unwrap()) })).collect::<Vec<_>>())
    }
    under(&all, id)
}

async fn entity(srv: &Srv, id: &str) -> Option<Value> {
    srv.rpc("doc.outline", json!({})).await["entities"].as_array().unwrap().iter().find(|e| e["entity_id"] == json!(id) && e["deleted"] != json!(true)).cloned()
}

fn analysis(inputs: Value, result: Value, prompt: &str) -> Value {
    json!({ "analysis": { "schema_version": "wish.analysis.v2", "status": "ready", "context_prompt": prompt, "inputs": inputs,
                          "output_contract": { "results": [result] }, "checks": [], "blockers": [], "warnings": [] } })
}

fn loc(cell: &str) -> Value {
    json!({ "cell_id": cell, "selection": [] })
}

/// Records every tool answer the model saw.
fn recording(seen: &Arc<Mutex<Vec<String>>>, steps: Vec<Value>) -> Script {
    let seen = seen.clone();
    Box::new(move |t: &Turn| {
        *seen.lock().unwrap() = t.results.clone();
        match steps.get(t.results.len()) {
            Some(s) => call(s["tool"].as_str().unwrap(), s["args"].clone()),
            None => text("done"),
        }
    })
}

fn step(tool: &str, args: Value) -> Value {
    json!({ "tool": tool, "args": args })
}

#[test]
fn a_control_result_lands_as_a_tree_and_an_overwrite_replaces_it() {
    run_test(|| async move {
    let m = model();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    *m.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("submit_analysis", analysis(json!([]), json!({ "name": "plan", "type": "control", "kind": "mindmap", "title": "Launch plan", "approach": "direct" }), "Write a mind map of a product launch plan.")),
        _ => text("ok"),
    });
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-mm", "stage": "analyze", "location": loc("blk-wish") })).await;
    assert_eq!(a["state"], "waiting_confirmation", "{a}");
    assert_eq!(a["candidate"]["analysis"]["output_contract"]["results"][0]["kind"], "mindmap");
    srv.apply(&a, json!({})).await;

    // the model: a wrong kind and an empty tree are answered in the run; then the outline
    let seen = Arc::new(Mutex::new(Vec::new()));
    *m.execute.lock().unwrap() = recording(&seen, vec![
        step("put_result", json!({ "name": "plan", "type": "control", "kind": "flowchart", "children": PLAN_MD })),
        step("put_result", json!({ "name": "plan", "type": "control", "kind": "mindmap", "children": "\n```\n- fenced\n```\n" })),
        step("put_result", json!({ "name": "plan", "type": "control", "kind": "mindmap", "children": PLAN_MD, "config": { "direction": "both", "x": 10 } })),
        step("finish", json!({ "summary": "a launch plan" })),
    ]);
    let e = srv.run("wish.xllm@1", json!({ "wish_id": "wish-mm", "stage": "execute", "location": loc("blk-wish") })).await;
    assert_eq!(e["state"], "waiting_confirmation", "{e}");
    let answers = seen.lock().unwrap().clone();
    assert!(answers[0].contains("needs kind, a control of catalog/controls.json"), "{answers:?}");
    assert!(answers[1].contains("has no nodes"), "{answers:?}");
    assert!(answers[2].contains("Received result \"plan\"") && answers[2].contains("- Launch plan\\n  - Market"), "{answers:?}");
    let r = &e["candidate"]["results"][0];
    assert_eq!((r["type"].as_str(), r["control"]["kind"].as_str()), (Some("control"), Some("mindmap")));
    assert_eq!(r["control"]["config"], json!({ "direction": "both" }), "only the control's attributes are kept");
    srv.apply(&e, json!({})).await;

    let w = srv.wish_of("wish-mm").await;
    let b = &w["last_run"]["result_bindings"];
    let first = b["results"]["plan"]["entity_id"].as_str().unwrap().to_string();
    let c = entity(&srv, &first).await.expect("the control");
    assert_eq!((c["kind"].as_str(), c["title"].as_str(), c["parent_id"].as_str()), (Some("mindmap"), Some("Launch plan"), b["group"].as_str()));
    assert_eq!(c["layout"], json!({ "mode": "tree" }));
    assert_eq!(c["config"]["direction"], "both");
    assert_eq!((c["placement"]["w"].as_str(), c["placement"]["h"].as_str()), (Some("auto"), Some("auto")), "the control's box is its layout's");
    let expected = json!([{ "text": "Launch plan", "children": [
        { "text": "Market", "children": [{ "text": "interviews", "children": [] }, { "text": "**competitors**", "children": [] }] },
        { "text": "Product", "children": [{ "text": "MVP", "children": [] }, { "text": "Pricing", "children": [] }] }] }]);
    assert_eq!(tree(&srv, &first).await, expected, "the tree is the outline");
    let root = srv.rpc("doc.outline", json!({})).await["entities"].as_array().unwrap().iter().find(|e| e["parent_id"] == json!(first)).cloned().unwrap();
    assert_eq!(root["view_type"], "mindnode");
    assert_eq!(root["placement"], json!({ "x": "auto", "y": "auto", "w": "auto", "h": "auto" }), "no coordinates");
    assert_eq!(srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-mm"] })).await["items"][0]["status"], "current");

    // an overwrite re-run, this time from a program: the old control and its nodes go, the new one takes its place
    let moved = json!({ "x": 64, "y": 96, "w": "auto", "h": "auto" });
    srv.commit(json!([{ "op": "tree.move", "entity_id": first, "new_parent_id": b["group"], "order_key": c["order_key"], "placement": moved }])).await;
    *m.execute.lock().unwrap() = recording(&seen, vec![
        step("write_file", json!({ "path": "program/main.js", "content": "export default async function main(aiws) {\n  aiws.result.control('plan', { kind: 'mindmap', children: [{ text: 'Launch v2', children: [{ text: 'Sales', children: [] }] }] })\n}\n" })),
        step("run_program", json!({})),
        step("finish", json!({ "summary": "v2" })),
    ]);
    let e2 = srv.run("wish.xllm@1", json!({ "wish_id": "wish-mm", "stage": "execute", "location": loc("blk-wish") })).await;
    assert_eq!(e2["state"], "waiting_confirmation", "{e2}");
    srv.apply(&e2, json!({})).await;
    let w = srv.wish_of("wish-mm").await;
    let second = w["last_run"]["result_bindings"]["results"]["plan"]["entity_id"].as_str().unwrap().to_string();
    assert_ne!(second, first);
    assert!(entity(&srv, &first).await.is_none(), "the last control is deleted");
    let c2 = entity(&srv, &second).await.unwrap();
    assert_eq!((c2["parent_id"].clone(), c2["placement"].clone()), (b["group"].clone(), moved), "the new control takes the old one's place");
    assert_eq!(tree(&srv, &second).await, json!([{ "text": "Launch v2", "children": [{ "text": "Sales", "children": [] }] }]));
    assert_eq!(w["last_run"]["result_bindings"]["results"]["plan"]["approach"], "program");
    });
}

#[test]
fn a_mind_map_node_input_is_read_as_its_branch_and_expanded_below_it() {
    run_test(|| async move {
    let m = model();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    *m.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => {
            let market = handle(&t.user, "mind map node \"Market\"");
            call("submit_analysis", analysis(json!([{ "name": "branch", "ref": market }]), json!({ "name": "ideas", "type": "control", "kind": "mindmap", "approach": "direct" }),
                                             &format!("Read {market} and add ideas below it.")))
        }
        _ => text("ok"),
    });
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-ex", "stage": "analyze", "location": loc("blk-ex") })).await;
    assert_eq!(a["state"], "waiting_confirmation", "{a}");
    assert_eq!(a["candidate"]["inputs"][0]["entity_id"], "n-market", "a node is kept as a Block input: {}", a["candidate"]);
    srv.apply(&a, json!({})).await;

    let seen = Arc::new(Mutex::new(Vec::new()));
    let script = |children: &str| vec![
        step("ws_read", json!({ "target": "n-market" })),
        step("put_result", json!({ "name": "ideas", "type": "control", "kind": "mindmap", "children": "- x", "under": "nope" })),
        step("put_result", json!({ "name": "ideas", "type": "control", "kind": "mindmap", "children": children, "under": "branch" })),
        step("finish", json!({ "summary": "more ideas" })),
    ];
    *m.execute.lock().unwrap() = recording(&seen, script("- Pricing\n- Channels\n  - Partners\n"));
    let e = srv.run("wish.xllm@1", json!({ "wish_id": "wish-ex", "stage": "execute", "location": loc("blk-ex") })).await;
    assert_eq!(e["state"], "waiting_confirmation", "{e}");
    let answers = seen.lock().unwrap().clone();
    assert!(answers[0].contains("is a node of a mindmap") && answers[0].contains("- Market\n  - interviews\n"), "{answers:?}");
    assert!(answers[1].contains("is not an input name"), "{answers:?}");
    // the program and the model read the branch as a file of the input
    let outline = find(srv._dir.path(), "outline.md").expect("the branch is materialized");
    assert_eq!(std::fs::read_to_string(outline).unwrap(), "- Market\n  - interviews\n");
    srv.apply(&e, json!({})).await;
    let market = |extra: Value| {
        let mut kids = vec![json!({ "text": "interviews", "children": [] })];
        kids.extend(extra.as_array().cloned().unwrap());
        json!(kids)
    };
    assert_eq!(tree(&srv, "n-market").await, market(json!([{ "text": "Pricing", "children": [] }, { "text": "Channels", "children": [{ "text": "Partners", "children": [] }] }])));
    assert_eq!(tree(&srv, "mm").await[0]["children"][1]["text"], "Product", "the rest of the map is untouched");
    let f = srv.rpc("doc.freshness", json!({ "entity_ids": ["wish-ex"] })).await;
    assert_eq!(f["items"][0]["status"], "current", "what the wish added below its input does not make it stale: {f}");

    // an overwrite re-run replaces what the last run added; its input no longer shows them
    *m.execute.lock().unwrap() = recording(&seen, script("- Pricing v2\n"));
    let e2 = srv.run("wish.xllm@1", json!({ "wish_id": "wish-ex", "stage": "execute", "location": loc("blk-ex") })).await;
    assert_eq!(e2["state"], "waiting_confirmation", "{e2}");
    let answers = seen.lock().unwrap().clone();
    assert!(answers[0].contains("- Market\n  - interviews\n") && !answers[0].contains("Pricing"), "{answers:?}");
    srv.apply(&e2, json!({})).await;
    assert_eq!(tree(&srv, "n-market").await, market(json!([{ "text": "Pricing v2", "children": [] }])));
    });
}

#[test]
fn an_executor_outside_the_service_reports_a_control() {
    run_test(|| async move {
    let m = model();
    let mut srv = Srv::start(tempfile::tempdir().unwrap(), m.clone()).await;
    setup(&mut srv).await;
    *m.analyze.lock().unwrap() = Box::new(|t: &Turn| match t.results.len() {
        0 => call("submit_analysis", analysis(json!([]), json!({ "name": "plan", "type": "control", "kind": "mindmap", "approach": "direct" }), "Write a mind map of a product launch plan.")),
        _ => text("ok"),
    });
    let a = srv.run("wish.xllm@1", json!({ "wish_id": "wish-mm", "stage": "analyze", "location": loc("blk-wish") })).await;
    srv.apply(&a, json!({})).await;
    let report = |key: &'static str, content: Value| {
        let srv = &srv;
        async move {
            let start = srv.rpc("proc.start", json!({ "program": "wish.xllm@1", "idempotency_key": key, "params": { "wish_id": "wish-mm", "stage": "execute", "location": loc("blk-wish"), "execution": { "mode": "local" } } })).await;
            let run_id = start["run_id"].as_str().unwrap().to_string();
            let claimed = srv.rpc("proc.claim", json!({ "run_id": run_id, "claim_token": start["claim_token"] })).await;
            let results = json!([{ "name": "plan", "type": "control", "title": "Launch", "content": content }]);
            let r = srv.call_as(claimed["run_token"].as_str().unwrap(), "proc.report", json!({ "workspace_id": srv.ws, "run_id": run_id, "attempt": 1, "results": results, "summary": "done", "read_set": [] })).await;
            assert_eq!(r["result"]["ok"], true, "{r}");
            srv.wait(&run_id).await
        }
    };
    let bad = report("loc-bad", json!({ "kind": "mindmap", "children": [] })).await;
    assert_eq!(bad["state"], "failed", "{bad}");
    assert!(bad["error"]["detail"].as_str().unwrap_or("").contains("has no nodes"), "{bad}");
    let done = report("loc-ok", json!({ "kind": "mindmap", "children": "- A\n  - B\n", "config": { "edge": "elbow" } })).await;
    assert_eq!(done["state"], "waiting_confirmation", "{done}");
    srv.apply(&done, json!({})).await;
    let id = srv.wish_of("wish-mm").await["last_run"]["result_bindings"]["results"]["plan"]["entity_id"].as_str().unwrap().to_string();
    assert_eq!(tree(&srv, &id).await, json!([{ "text": "A", "children": [{ "text": "B", "children": [] }] }]));
    assert_eq!(entity(&srv, &id).await.unwrap()["config"]["edge"], "elbow");
    });
}

fn find(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find(&p, name) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}
