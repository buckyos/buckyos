//! Annotation anchors (design doc §3.7): graded resolution, rich text ranges that follow edits
//! and fall back to their quote, application ranges, replay of newer anchors, re-anchoring.

use aiworkspace_core::access::{Access, Cap, CapSet};
use aiworkspace_core::model::*;
use aiworkspace_core::read;
use aiworkspace_core::richtext;
use aiworkspace_core::testkit::MemWorkspace;
use base64::Engine;
use loro::cursor::Side;
use loro::{Container, LoroDoc, LoroMap, LoroText, LoroValue, ValueOrContainer, VersionVector};
use serde_json::{json, Value};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

fn para(id: &str, text: &str) -> Value {
    json!({ "type": "paragraph", "attrs": { "block_id": id }, "content": [{ "type": "text", "text": text }] })
}

fn setup(blocks: Value) -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "page-main", "type_id": TYPE_CONTAINER, "parent_id": "root", "order_key": "a",
          "payload": { "kind": "page" } },
        { "op": "entity.create", "entity_id": "notes", "type_id": TYPE_RICHTEXT, "parent_id": "page-main", "order_key": "b",
          "payload": { "content": { "type": "doc", "content": blocks } } }
    ]));
    ws
}

fn tasks(ws: &mut MemWorkspace) {
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "tasks", "type_id": TYPE_TABLE, "parent_id": "page-main", "order_key": "c",
          "payload": { "fields": [{ "field_id": "title", "name": "任务", "type": "text" }] } },
        { "op": "table.insert_records", "source_id": "tasks", "records": [{ "record_id": "task-1", "values": { "title": "盘点" } }] }
    ]));
}

fn note(id: &str, payload: Value) -> Value {
    let mut payload = payload;
    payload["kind"] = json!("note");
    payload["body"] = json!(format!("body of {id}"));
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_ANNOTATION, "parent_id": "page-main", "order_key": "n", "payload": payload })
}

fn block(id: &str) -> Value {
    json!({ "entity_id": "notes", "selector": { "kind": "richtext_block", "block_id": id } })
}

fn anchor(ws: &MemWorkspace, id: &str) -> Value {
    read::read(&ws.store, &Access::full("alice"), id, None).unwrap()["content"]["anchor"].clone()
}

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn find_text(map: &LoroMap, block_id: &str) -> Option<LoroText> {
    let Some(ValueOrContainer::Container(Container::List(children))) = map.get("children") else { return None };
    for i in 0..children.len() {
        let Some(ValueOrContainer::Container(Container::Map(m))) = children.get(i) else { continue };
        let id = match m.get("attributes") {
            Some(ValueOrContainer::Container(Container::Map(a))) => match a.get("block_id") {
                Some(ValueOrContainer::Value(LoroValue::String(s))) => Some(s.to_string()),
                _ => None,
            },
            _ => None,
        };
        if id.as_deref() == Some(block_id) {
            let Some(ValueOrContainer::Container(Container::List(inline))) = m.get("children") else { return None };
            return match inline.get(0) {
                Some(ValueOrContainer::Container(Container::Text(t))) => Some(t),
                _ => None,
            };
        }
        if let Some(t) = find_text(&m, block_id) {
            return Some(t);
        }
    }
    None
}

/// An editor's working copy of `notes`.
struct Client {
    doc: LoroDoc,
    since: VersionVector,
}

impl Client {
    fn open(ws: &MemWorkspace) -> Client {
        let doc = richtext::fork(&ws.store.richtexts["notes"].doc);
        doc.set_peer_id(9).unwrap();
        let since = doc.oplog_vv();
        Client { doc, since }
    }
    fn text(&self, block_id: &str) -> LoroText {
        find_text(&self.doc.get_map("doc"), block_id).expect("text block")
    }
    fn range(&self, (b1, p1): (&str, usize), (b2, p2): (&str, usize)) -> Value {
        let cursor = |b: &str, p: usize| B64.encode(self.text(b).get_cursor(p, Side::Middle).unwrap().encode());
        json!({ "kind": "richtext_text", "start": { "block_id": b1, "cursor": cursor(b1, p1) }, "end": { "block_id": b2, "cursor": cursor(b2, p2) } })
    }
    fn push(&mut self, ws: &mut MemWorkspace) {
        let update = richtext::export_updates(&self.doc, &self.since).unwrap();
        self.since = self.doc.oplog_vv();
        let lineage = ws.store.richtexts["notes"].meta.lineage_id.clone();
        ws.ok(json!([{ "op": "richtext.apply_update", "entity_id": "notes", "lineage_id": lineage, "update": B64.encode(update) }]));
    }
}

fn info(ws: &MemWorkspace, block_id: &str) -> BlockInfo {
    ws.store.richtexts["notes"].meta.block_index[block_id].clone()
}

#[test]
fn text_range_follows_edits_then_its_quote() {
    let mut ws = setup(json!([para("p1", "第一期围绕文档格式展开验证"), para("p2", "其他内容")]));
    let mut c = Client::open(&ws);
    ws.ok(json!([note("n1", json!({ "target": block("p1"), "range": c.range(("p1", 5), ("p1", 9)),
        "context": { "quote": { "exact": "文档格式", "prefix": "第一期围绕", "suffix": "展开验证" }, "label": "第 1 段" } }))]));
    let a = anchor(&ws, "n1");
    assert_eq!((a["state"].as_str(), a["level"].as_str(), a["range_status"].as_str()), (Some("resolved"), Some("range"), Some("exact")));
    assert_eq!(a["position"], json!({ "block_id": "p1", "text": "文档格式" }));

    // concurrent typing before the range: the cursors follow
    c.text("p1").insert(0, "【新】").unwrap();
    c.push(&mut ws);
    assert_eq!(anchor(&ws, "n1")["range_status"], "exact");
    assert_eq!(anchor(&ws, "n1")["position"]["text"], "文档格式");

    // a block operation rebuilds the block: the cursors are gone, the quote finds the text again
    ws.ok(json!([{ "op": "richtext.move_block", "entity_id": "notes", "block_id": "p1",
        "expect": { "struct_rev": info(&ws, "p1").struct_rev }, "position": { "after": "p2" } }]));
    let a = anchor(&ws, "n1");
    assert_eq!((a["state"].as_str(), a["level"].as_str(), a["range_status"].as_str()), (Some("resolved"), Some("range"), Some("relocated")));
    assert_eq!(a["position"]["text"], "文档格式");

    // the text itself is rewritten: shown on its block
    ws.ok(json!([{ "op": "richtext.replace_block", "entity_id": "notes", "block_id": "p1",
        "expect": { "hash": info(&ws, "p1").hash }, "node": para("p1", "全部重写") }]));
    let a = anchor(&ws, "n1");
    assert_eq!((a["state"].as_str(), a["level"].as_str(), a["range_status"].as_str()), (Some("degraded"), Some("target"), Some("lost")));
    assert!(a.get("position").is_none());

    // its block is gone but the quoted text exists once elsewhere (cut and paste gives new ids)
    ws.ok(json!([{ "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "end": true }, "blocks": [para("p3", "再谈文档格式")] }]));
    ws.ok(json!([{ "op": "richtext.delete_blocks", "entity_id": "notes",
        "blocks": [{ "block_id": "p1", "expect": { "hash": info(&ws, "p1").hash, "struct_rev": info(&ws, "p1").struct_rev } }] }]));
    let a = anchor(&ws, "n1");
    assert_eq!((a["state"].as_str(), a["level"].as_str(), a["range_status"].as_str()), (Some("degraded"), Some("range"), Some("relocated")));
    assert_eq!(a["position"], json!({ "block_id": "p3", "text": "文档格式" }));

    // the entity is gone: nothing left to show it on; the recorded anchor is never rewritten
    let life = ws.store.entities["notes"].life_rev;
    ws.ok(json!([{ "op": "entity.delete", "entity_id": "notes", "expect": { "rev": life } }]));
    assert_eq!(anchor(&ws, "n1"), json!({ "state": "target_deleted", "level": "none" }));
    assert_eq!(ws.store.entities["n1"].payload["target"], block("p1"));
}

#[test]
fn cursors_count_chars_and_span_blocks() {
    let mut ws = setup(json!([para("p1", "😀开头：甲乙丙"), para("p2", "丁戊己庚")]));
    let mut c = Client::open(&ws);
    ws.ok(json!([note("n1", json!({ "target": block("p1"), "range": c.range(("p1", 5), ("p2", 2)),
        "context": { "quote": { "exact": "乙丙\n丁戊" } } }))]));
    assert_eq!(anchor(&ws, "n1")["position"], json!({ "block_id": "p1", "end_block_id": "p2", "text": "乙丙\n丁戊" }));
    // typing at the start boundary stays outside, typing inside widens the range
    c.text("p1").insert(5, "X").unwrap();
    c.text("p2").insert(1, "Y").unwrap();
    c.push(&mut ws);
    let a = anchor(&ws, "n1");
    assert_eq!((a["range_status"].as_str(), a["position"]["text"].as_str()), (Some("exact"), Some("乙丙\n丁Y戊")));
    // a range must start in the selected block, and its cursors must be cursors
    let mut bad = c.range(("p1", 5), ("p2", 2));
    bad["start"]["block_id"] = json!("p2");
    assert_eq!(code(&ws.fail(json!([note("n2", json!({ "target": block("p1"), "range": bad }))]))["errors"][0]), "INVALID_SCHEMA");
    let mut bad = c.range(("p1", 5), ("p2", 2));
    bad["end"]["cursor"] = json!("bm90IGEgY3Vyc29y");
    assert_eq!(code(&ws.fail(json!([note("n2", json!({ "target": block("p1"), "range": bad }))]))["errors"][0]), "INVALID_SCHEMA");
}

#[test]
fn block_annotations_fall_back_by_grade() {
    let mut ws = setup(json!([para("p1", "季度目标与预算"), para("p2", "风险"), para("p4", "附录")]));
    ws.ok(json!([note("n1", json!({ "target": block("p1"), "context": { "quote": { "exact": "季度目标与预算" } } })),
                 note("n2", json!({ "target": block("p2") }))]));
    assert_eq!(anchor(&ws, "n1"), json!({ "state": "resolved", "level": "target" }));
    // cut and paste: the block comes back under a new id and the quote finds it
    ws.ok(json!([
        { "op": "richtext.delete_blocks", "entity_id": "notes",
          "blocks": [{ "block_id": "p1", "expect": { "hash": info(&ws, "p1").hash, "struct_rev": info(&ws, "p1").struct_rev } }] },
        { "op": "richtext.insert_blocks", "entity_id": "notes", "position": { "after": "p4" }, "blocks": [para("p9", "季度目标与预算")] }
    ]));
    assert_eq!(anchor(&ws, "n1"), json!({ "state": "degraded", "level": "target", "position": { "block_id": "p9" } }));
    // without a quote the annotation falls back to the entity, and comes back with its block
    let s = ws.ok(json!([{ "op": "richtext.delete_blocks", "entity_id": "notes",
        "blocks": [{ "block_id": "p2", "expect": { "hash": info(&ws, "p2").hash, "struct_rev": info(&ws, "p2").struct_rev } }] }]));
    assert_eq!(anchor(&ws, "n2"), json!({ "state": "degraded", "level": "entity" }));
    ws.undo(s).ok().expect("undo delete");
    assert_eq!(anchor(&ws, "n2"), json!({ "state": "resolved", "level": "target" }));
}

#[test]
fn application_ranges_are_kept_verbatim() {
    let mut ws = setup(json!([para("p1", "甲")]));
    tasks(&mut ws);
    let cell = json!({ "entity_id": "tasks", "selector": { "kind": "table_cell", "record_id": "task-1", "field_id": "title" } });
    let app = json!({ "kind": "com.example.sheet/chars", "from": 0, "to": 2, "any": { "shape": [1, 2] } });
    ws.ok(json!([note("n1", json!({ "target": cell, "range": app, "context": { "quote": { "exact": "盘点" }, "label": "任务 / 盘点" } })),
                 note("n2", json!({ "target": block("p1"), "range": { "kind": "com.example.chart/point", "series": "s1", "index": 3 } })),
                 note("n3", json!({ "target": { "entity_id": "tasks" }, "range": { "kind": "com.example.sheet/rows", "rows": [1] } }))]));
    for id in ["n1", "n2", "n3"] {
        assert_eq!(anchor(&ws, id), json!({ "state": "resolved", "level": "target", "range_status": "unchecked" }), "{id}");
    }
    assert_eq!(ws.store.entities["n1"].payload["range"], app);
    // writes are strict: only known anchors, well-formed context, bounded sizes
    let fails = [
        json!({ "target": cell, "range": { "kind": "Sheet/x" } }),
        json!({ "target": cell, "range": { "kind": "richtext_text", "start": { "block_id": "p1" }, "end": { "block_id": "p1" } } }),
        json!({ "target": block("p1"), "range": { "kind": "richtext_word" } }),
        json!({ "target": block("p1"), "range": { "from": 1 } }),
        json!({ "target": { "entity_id": "notes", "selector": { "kind": "richtext_line", "line": 1 } } }),
        json!({ "target": { "entity_id": "tasks", "selector": { "kind": "table_cell", "record_id": "task-1" } } }),
        json!({ "target": block("p1"), "context": { "quote": { "prefix": "x" } } }),
        json!({ "target": block("p1"), "context": { "color": "red" } }),
        json!({ "target": { "entity_id": "notes", "workspace_id": "ws_other" } }),
    ];
    for payload in fails {
        assert_eq!(code(&ws.fail(json!([note("bad", payload.clone())]))["errors"][0]), "INVALID_SCHEMA", "{payload}");
    }
    let big = json!({ "target": cell, "range": { "kind": "com.example.sheet/blob", "data": "x".repeat(5000) } });
    assert_eq!(code(&ws.fail(json!([note("bad", big)]))["errors"][0]), "LIMIT_EXCEEDED");
    assert_eq!(code(&ws.fail(json!([note("bad", json!({ "target": block("ghost") }))]))["errors"][0]), "REFERENCE_BROKEN");
}

#[test]
fn replay_keeps_anchors_of_newer_backends() {
    let mut ws = setup(json!([para("p1", "甲")]));
    // what a newer backend accepted arrives through replay/import: kept, never refused
    let mut env = ws.env("alice", "human", None, true);
    env.import = true;
    env.replay = true;
    let req = MemWorkspace::request(json!([
        note("n1", json!({ "target": { "entity_id": "notes", "selector": { "kind": "richtext_line", "line": 3 } },
                           "range": { "kind": "richtext_columns", "from": 1 }, "context": { "quote": { "exact": "甲" }, "color": "red" },
                           "thread": { "replies": 2 }, "author": "dave" })),
        note("n2", json!({ "target": block("p1"), "range": { "kind": "richtext_columns", "from": 1 } }))
    ]));
    ws.commit_with(&Access::full("alice"), &env, &req).ok().expect("replay");
    assert_eq!(ws.store.entities["n1"].payload["author"], "dave");
    assert_eq!(ws.store.entities["n1"].payload["thread"], json!({ "replies": 2 }));
    // shown where this backend can: on the entity, or on the block with the range left to the application
    assert_eq!(anchor(&ws, "n1"), json!({ "state": "unsupported", "level": "entity" }));
    assert_eq!(anchor(&ws, "n2"), json!({ "state": "resolved", "level": "target", "range_status": "unchecked" }));
}

#[test]
fn only_the_author_reanchors() {
    let mut ws = setup(json!([para("p1", "甲乙"), para("p2", "丙丁")]));
    ws.ok(json!([note("n1", json!({ "target": block("p1"), "context": { "quote": { "exact": "甲乙" } } }))]));
    let rev = |ws: &MemWorkspace, k: &str| ws.store.entities["n1"].key_rev(k);
    let set = |keys: Value| json!([{ "op": "entity.set_keys", "entity_id": "n1", "keys": keys }]);
    // another principal (even with manage) can edit the body, never move the anchor
    let bob = Access::full("bob");
    let env = ws.env("bob", "human", None, false);
    let req = MemWorkspace::request(set(json!([{ "key": "target", "value": block("p2"), "expect": { "rev": rev(&ws, "target") } }])));
    assert_eq!(code(&ws.commit_with(&bob, &env, &req).err().unwrap().to_json()["errors"][0]), "PERMISSION_DENIED");
    let req = MemWorkspace::request(set(json!([{ "key": "body", "value": "补充", "expect": { "rev": rev(&ws, "body") } }])));
    ws.commit_with(&bob, &env, &req).ok().expect("bob edits the body");
    // the author: a new target must restate the old quote (null clears it)
    let f = ws.fail(set(json!([{ "key": "target", "value": block("p2"), "expect": { "rev": rev(&ws, "target") } }])));
    assert_eq!(code(&f["errors"][0]), "INVALID_OPERATION");
    let s = ws.ok(set(json!([{ "key": "target", "value": block("p2"), "expect": { "rev": rev(&ws, "target") } },
                             { "key": "context", "value": null, "expect": { "rev": rev(&ws, "context") } }])));
    assert!(ws.store.entities["n1"].payload.get("context").is_none());
    assert_eq!(ws.store.entities["n1"].payload["target"]["selector"]["block_id"], "p2");
    // a target that does not exist is refused like on creation; undo brings the old anchor back
    let f = ws.fail(set(json!([{ "key": "target", "value": block("ghost"), "expect": { "rev": rev(&ws, "target") } }])));
    assert_eq!(code(&f["errors"][0]), "REFERENCE_BROKEN");
    ws.undo(s).ok().expect("undo re-anchor");
    assert_eq!(ws.store.entities["n1"].payload["target"]["selector"]["block_id"], "p1");
    assert_eq!(ws.store.entities["n1"].payload["context"], json!({ "quote": { "exact": "甲乙" } }));
}

#[test]
fn list_annotations_by_target_and_page() {
    let mut ws = setup(json!([para("p1", "甲")]));
    tasks(&mut ws);
    let carol = Access { principal: "carol".into(), ws_caps: CapSet(Cap::Read as u16 | Cap::Comment as u16), scoped: Default::default() };
    ws.ok(json!([note("n1", json!({ "target": block("p1") })), note("n2", json!({ "target": { "entity_id": "tasks" } }))]));
    let env = ws.env("carol", "human", None, false);
    let mut mine = note("n3", json!({ "target": block("p1") }));
    mine["scope"] = json!("personal");
    ws.commit_with(&carol, &env, &MemWorkspace::request(json!([mine]))).ok().expect("personal note");
    let ids = |v: Vec<Value>| v.iter().map(|a| a["entity_id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let alice = Access::full("alice");
    assert_eq!(ids(read::list_annotations(&ws.store, &alice, &["notes".into()], None).unwrap()), ["n1"]);
    assert_eq!(ids(read::list_annotations(&ws.store, &carol, &["notes".into()], None).unwrap()), ["n1", "n3"]);
    let all = read::list_annotations(&ws.store, &carol, &[], Some("page-main")).unwrap();
    assert_eq!(ids(all.clone()), ["n1", "n2", "n3"]);
    assert_eq!(all[2]["scope"], "personal");
    assert_eq!(all[0]["content"]["anchor"]["state"], "resolved");
    assert_eq!(code(&read::list_annotations(&ws.store, &carol, &[], Some("ghost")).unwrap_err().to_json()), "NOT_FOUND");
}
