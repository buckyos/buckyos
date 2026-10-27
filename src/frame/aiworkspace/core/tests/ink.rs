//! Pen ink (notepads/aiworkspace-handwriting-todo.md D0): ink segments as the Desktop writes them
//! (`fixtures/ink/strokes.json`, from `desktop/tests/datamodel/aiworkspace-ink.test.ts`) commit through the planner and
//! read back unchanged; the outline projects the logical stroke; malformed ink, oversized configs and data bindings are
//! refused; a long stroke is one commit and one undo.

use aiworkspace_core::access::Access;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

const FIXTURE: &str = include_str!("../../fixtures/ink/strokes.json");

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn commits() -> Vec<Value> {
    serde_json::from_str::<Value>(FIXTURE).unwrap()["commits"].as_array().unwrap().clone()
}

fn replayed() -> (MemWorkspace, Vec<u64>) {
    let mut ws = MemWorkspace::new();
    let mut results = Vec::new();
    for c in commits() {
        results.push(ws.ok(c["operations"].clone()));
    }
    (ws, results)
}

fn outline_of(ws: &MemWorkspace, id: &str) -> Value {
    let outline = aiworkspace_core::read::outline(&ws.store, &Access::full("alice")).unwrap();
    outline.into_iter().find(|e| e["entity_id"] == id).unwrap()
}

#[test]
fn fixture_strokes_commit_and_read_back() {
    let (ws, _) = replayed();
    for c in commits().iter().skip(1) {
        for op in c["operations"].as_array().unwrap() {
            let id = op["entity_id"].as_str().unwrap();
            assert_eq!(ws.store.entities[id].payload["config"], op["payload"]["config"], "{id}");
            assert_eq!(ws.store.edges[id].placement.as_ref(), Some(&op["placement"]), "{id}");
            let o = outline_of(&ws, id);
            assert_eq!(o["view_type"], "ink");
            assert_eq!(o["ink_stroke"], op["payload"]["config"]["stroke"], "{id}");
        }
    }
    // the long stroke: several segments of one logical stroke, in order, each within the config limit
    let long = &commits()[4]["operations"];
    let segs: Vec<&Value> = long.as_array().unwrap().iter().collect();
    assert!(segs.len() > 1);
    for (i, op) in segs.iter().enumerate() {
        assert_eq!(op["payload"]["config"]["seg"], json!(i));
        assert_eq!(op["payload"]["config"]["segs"], json!(segs.len()));
        assert!(serde_json::to_string(&op["payload"]["config"]).unwrap().len() <= 64 * 1024);
    }
    // a Block of another view carries no stroke
    assert!(outline_of(&ws, "ink").get("ink_stroke").is_none());
}

#[test]
fn a_long_stroke_is_one_commit_and_one_undo() {
    let (mut ws, results) = replayed();
    let long = *results.last().unwrap();
    let ids: Vec<String> = commits()[4]["operations"].as_array().unwrap().iter().map(|o| o["entity_id"].as_str().unwrap().to_string()).collect();
    ws.undo(long).ok().expect("undo the stroke");
    for id in &ids {
        assert!(!ws.store.entities[id.as_str()].alive(), "{id}");
    }
    // erasing it again is one commit of deletes, undone as one
    let mut ws2 = replayed().0;
    let deletes: Vec<Value> = ids.iter().map(|id| json!({ "op": "entity.delete", "entity_id": id, "expect": { "rev": ws2.store.entities[id.as_str()].life_rev } })).collect();
    let erased = ws2.ok(json!(deletes));
    ws2.undo(erased).ok().expect("undo the erase");
    for id in &ids {
        assert!(ws2.store.entities[id.as_str()].alive(), "{id}");
    }
}

#[test]
fn malformed_ink_is_refused() {
    let (mut ws, _) = replayed();
    let base = commits()[3]["operations"][0].clone();
    let with = |patch: &dyn Fn(&mut Value)| {
        let mut op = base.clone();
        op["entity_id"] = json!("c-bad");
        patch(&mut op["payload"]["config"]);
        json!([op])
    };
    let cases: Vec<(&str, Box<dyn Fn(&mut Value)>)> = vec![
        ("format", Box::new(|c| c["format"] = json!(2))),
        ("unknown key", Box::new(|c| c["brush"] = json!("pencil"))),
        ("stroke id", Box::new(|c| c["stroke"] = json!("Not An Id"))),
        ("seg", Box::new(|c| c["seg"] = json!(1))),
        ("segs", Box::new(|c| c["segs"] = json!(65))),
        ("color", Box::new(|c| c["color"] = json!("red"))),
        ("width", Box::new(|c| c["width"] = json!(0))),
        ("pressure flag", Box::new(|c| c["pressure"] = json!("yes"))),
        ("frame", Box::new(|c| c["frame"] = json!({ "w": 0, "h": 10 }))),
        ("lengths", Box::new(|c| { c["points"]["y"].as_array_mut().unwrap().pop(); })),
        ("empty", Box::new(|c| c["points"] = json!({ "x": [], "y": [], "t": [] }))),
        ("outside the frame", Box::new(|c| c["points"]["x"][0] = json!(1e6))),
        ("time order", Box::new(|c| { let n = c["points"]["t"].as_array().unwrap().len(); c["points"]["t"][n - 1] = json!(0); c["points"]["t"][0] = json!(5); })),
        ("whole ms", Box::new(|c| c["points"]["t"][0] = json!(0.5))),
        ("pressure range", Box::new(|c| c["points"]["p"][0] = json!(1.5))),
        ("tilt range", Box::new(|c| c["points"]["tilt_x"][0] = json!(120))),
        ("unknown channel", Box::new(|c| c["points"]["twist"] = json!([0]))),
    ];
    for (what, patch) in cases {
        assert_eq!(code(&ws.fail(with(&*patch))), "INVALID_SCHEMA", "{what}");
    }
    // no data binding on a stroke
    let mut bound = base.clone();
    bound["entity_id"] = json!("c-bound");
    bound["payload"]["source_ref"] = json!({ "entity_id": "ink-content" });
    assert_eq!(code(&ws.fail(json!([bound]))), "INVALID_SCHEMA");
    // the config limit is the core's, measured on the canonical encoding
    let mut big = base.clone();
    big["entity_id"] = json!("c-big");
    let n = 9000;
    big["payload"]["config"]["frame"] = json!({ "w": 1000, "h": 1000 });
    big["payload"]["config"]["points"] = json!({ "x": vec![123.45; n], "y": vec![678.91; n], "t": (0..n).collect::<Vec<_>>() });
    assert_eq!(code(&ws.fail(json!([big]))), "LIMIT_EXCEEDED");
    // an unchanged Block keeps working: a later key write does not re-judge a valid config
    let id = "c-pressure-0";
    let rev = ws.store.entities[id].key_rev("title");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": id, "keys": [{ "key": "title", "value": "a curve", "expect": { "rev": rev } }] }]));
    // a later version of the renderer is not judged by version 1's rules
    let mut later = base.clone();
    later["entity_id"] = json!("c-v2");
    later["payload"]["view"]["version"] = json!(2);
    later["payload"]["config"] = json!({ "format": 2 });
    ws.ok(json!([later]));
}
