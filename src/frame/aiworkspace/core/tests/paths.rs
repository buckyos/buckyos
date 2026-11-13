//! Names, paths, bodies, text queries and meta (核心 API 调整 §2, §2.3, §3.2; design draft §2–§6).

use aiworkspace_core::access::{Access, CapSet};
use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use aiworkspace_core::{body, meta, path, search};
use serde_json::{json, Value};

fn ws() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "sales-folder", "type_id": TYPE_CONTAINER, "parent_id": DATA_ID, "order_key": "a", "payload": { "kind": "folder", "title": "销售/2026" } },
        { "op": "entity.create", "entity_id": "orders", "type_id": TYPE_TABLE, "parent_id": "sales-folder", "order_key": "a", "name": "订单",
          "payload": { "fields": [
            { "field_id": "customer", "name": "客户", "type": "text" },
            { "field_id": "region", "name": "地区", "type": "select", "options": [{ "option_id": "east", "label": "华东" }, { "option_id": "south", "label": "华南" }] },
            { "field_id": "note", "name": "备注", "type": "text" } ] } },
        { "op": "entity.create", "entity_id": "doc", "type_id": TYPE_RICHTEXT, "parent_id": "sales-folder", "order_key": "b",
          "payload": { "content": { "type": "doc", "content": [
            { "type": "heading", "attrs": { "block_id": "h1", "level": 2 }, "content": [{ "type": "text", "text": "口径" }] },
            { "type": "paragraph", "attrs": { "block_id": "p1" }, "content": [{ "type": "text", "text": "销售额按含税金额统计。" }] }] } } },
    ]));
    let rows: Vec<Value> = (0..30)
        .map(|i| json!({ "record_id": format!("o{i:02}"), "values": { "customer": format!("客户{i}"), "region": if i % 3 == 0 { "south" } else { "east" }, "note": "中文备注，一段比较长的说明文字".repeat(1 + i % 3) } }))
        .collect();
    ws.ok(json!([{ "op": "table.insert_records", "source_id": "orders", "records": rows }]));
    ws
}

fn full() -> Access {
    Access::full("alice")
}

#[test]
fn names_follow_titles_or_count_and_never_collide() {
    let mut ws = ws();
    let name = |ws: &MemWorkspace, id: &str| ws.store.entities[id].name.clone().unwrap();
    assert_eq!(name(&ws, "sales-folder"), "销售-2026", "a title's name: / becomes -");
    assert_eq!(name(&ws, "doc"), "doc-1", "a rich text has no title: a counter");
    // a surface and Blocks: titles, counters per view type
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "s1-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s1" } },
        { "op": "entity.create", "entity_id": "s1", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s1-content", "title": "看板" } },
        { "op": "entity.create", "entity_id": "b1", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "a", "placement": { "x": 0, "y": 100, "w": 10, "h": 10 }, "payload": { "view": { "type": "label" } } },
        { "op": "entity.create", "entity_id": "b2", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "b", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "label" } } },
        { "op": "entity.create", "entity_id": "b3", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "c", "placement": { "x": 50, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "table" }, "source_ref": { "entity_id": "orders" }, "title": "华东订单",
                     "filter": { "op": "cmp", "field_id": "region", "operator": "eq", "value": "east" } } },
    ]));
    assert_eq!((name(&ws, "s1"), name(&ws, "b1"), name(&ws, "b2"), name(&ws, "b3")), ("看板".into(), "label-1".into(), "label-2".into(), "华东订单".into()));
    // the title changes: the name follows in the same Commit; an explicitly chosen name stops following
    let rev = ws.store.entities["b3"].key_rev("title");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "b3", "keys": [{ "key": "title", "value": "华南订单", "expect": { "rev": rev } }] }]));
    assert_eq!(name(&ws, "b3"), "华南订单");
    let rev = ws.store.entities["b1"].key_rev("title");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "b1", "keys": [{ "key": "title", "value": "华南订单", "expect": { "rev": rev } }] }]));
    assert_eq!(name(&ws, "b1"), "华南订单-2", "a taken title's name moves aside");
    let meta_rev = ws.store.entities["b2"].meta_rev;
    ws.ok(json!([{ "op": "entity.rename", "entity_id": "b2", "name": "固定", "expect": { "rev": meta_rev } }]));
    let rev = ws.store.entities["b2"].key_rev("title");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "b2", "keys": [{ "key": "title", "value": "别的", "expect": { "rev": rev } }] }]));
    assert_eq!(name(&ws, "b2"), "固定");
    let meta_rev = ws.store.entities["b2"].meta_rev;
    ws.ok(json!([{ "op": "entity.rename", "entity_id": "b2", "name": null, "expect": { "rev": meta_rev } }]));
    assert_eq!(name(&ws, "b2"), "别的", "null gives the name back to the rule");
    // an explicitly given name that is taken is a conflict; one that is just the title's name moves aside
    let f = ws.fail(json!([{ "op": "entity.create", "entity_id": "b4", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "d", "name": "华南订单", "placement": { "x": 0, "y": 0, "w": 1, "h": 1 }, "payload": { "view": { "type": "label" } } }]));
    assert!(f.to_string().contains("NAME_CONFLICT"), "{f}");
    ws.ok(json!([{ "op": "entity.create", "entity_id": "b4", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "d", "name": "看板外", "placement": { "x": 0, "y": 0, "w": 1, "h": 1 }, "payload": { "view": { "type": "label" }, "title": "别的" } }]));
    assert_eq!(name(&ws, "b4"), "看板外");
    ws.ok(json!([{ "op": "entity.create", "entity_id": "b5", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "e", "name": "别的", "placement": { "x": 0, "y": 0, "w": 1, "h": 1 }, "payload": { "view": { "type": "label" }, "title": "别的" } }]));
    assert_eq!(name(&ws, "b5"), "别的-2");
    // names that would make paths ambiguous are refused
    let meta_rev = ws.store.entities["b5"].meta_rev;
    for bad in ["~x", "..", "a/b"] {
        assert!(ws.fail(json!([{ "op": "entity.rename", "entity_id": "b5", "name": bad, "expect": { "rev": meta_rev } }])).to_string().contains("INVALID_OPERATION"), "{bad}");
    }
}

#[test]
fn one_path_syntax_judged_by_the_caller() {
    let ws = ws();
    let ctx = &ws.store;
    assert_eq!(path::resolve(ctx, &full(), "/data/销售-2026/订单").unwrap(), "orders");
    assert_eq!(path::resolve(ctx, &full(), "~sales-folder/订单").unwrap(), "orders");
    assert_eq!(path::resolve(ctx, &full(), "orders").unwrap(), "orders", "an entity id");
    assert_eq!(path::path_of(ctx, &full(), "orders").unwrap(), "/data/销售-2026/订单");
    assert_eq!(path::resolve(ctx, &full(), "/data/nope").unwrap_err().code.as_str(), "NOT_FOUND");
    // a grant on the folder only: paths start at the highest readable ancestor, never naming the others
    let mut scoped = Access { principal: "bob".into(), ws_caps: CapSet::NONE, scoped: Default::default() };
    scoped.scoped.insert("sales-folder".into(), CapSet::parse(&["read".into()]).unwrap());
    assert_eq!(path::path_of(ctx, &scoped, "orders").unwrap(), "~sales-folder/订单");
    assert_eq!(path::resolve(ctx, &scoped, "~sales-folder/订单").unwrap(), "orders");
    assert_eq!(path::resolve(ctx, &scoped, "/data/销售-2026/订单").unwrap_err().code.as_str(), "NOT_FOUND", "an unreadable step is not there");
    // listing and finding with type filters
    let l = path::list(ctx, &full(), &json!({ "at": "/data/销售-2026" }), 7).unwrap();
    let names: Vec<&str> = l["items"].as_array().unwrap().iter().map(|i| i["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["订单", "doc-1"]);
    let l = path::list(ctx, &full(), &json!({ "at": "/data/销售-2026", "type": "table", "limit": 1 }), 7).unwrap();
    assert_eq!(l["items"].as_array().unwrap().len(), 1);
    let f = path::find(ctx, &full(), &json!({ "name": "订" }), 7).unwrap();
    assert_eq!((f["items"][0]["path"].as_str(), f["complete"].as_bool()), (Some("/data/销售-2026/订单"), Some(true)), "{f}");
    let f = path::find(ctx, &full(), &json!({ "type": "richtext" }), 7).unwrap();
    assert_eq!(f["items"][0]["entity_id"], "doc");
    // a cursor of another head is refused
    let l = path::list(ctx, &full(), &json!({ "at": "/data/销售-2026", "limit": 1 }), 7).unwrap();
    let e = path::list(ctx, &full(), &json!({ "at": "/data/销售-2026", "limit": 1, "cursor": l["next_cursor"] }), 8).unwrap_err();
    assert_eq!(e.code.as_str(), "SNAPSHOT_EXPIRED");
    let l2 = path::list(ctx, &full(), &json!({ "at": "/data/销售-2026", "limit": 1, "cursor": l["next_cursor"] }), 7).unwrap();
    assert_eq!(l2["items"][0]["name"], "doc-1");
}

#[test]
fn bodies_page_by_parts_and_never_cut_a_character() {
    let ws = ws();
    let ctx = &ws.store;
    let orders = ctx.entity("orders").unwrap().unwrap();
    // the whole table, page by page: the pages concatenate to the body; selects read as labels
    let read_all = |e: &EntityRow, max: u64| -> (String, usize) {
        let (mut text, mut cursor, mut pages) = (String::new(), None::<String>, 0);
        loop {
            let mut p = json!({ "max_bytes": max });
            if let Some(c) = &cursor {
                p["cursor"] = json!(c);
            }
            let b = body::read_body(ctx, &full(), e, &p, None).unwrap();
            assert!(b["text"].as_str().unwrap().len() as u64 <= max || pages == 0, "{b}");
            text.push_str(b["text"].as_str().unwrap());
            pages += 1;
            match b["next_cursor"].as_str() {
                Some(c) => cursor = Some(c.to_string()),
                None => return (text, pages),
            }
        }
    };
    let (text, pages) = read_all(&orders, 300);
    let (whole, one_page) = read_all(&orders, 262_144);
    assert_eq!((text.as_str(), one_page), (whole.as_str(), 1), "{pages} pages");
    let (cut_small, _) = read_all(&orders, 64);
    assert_eq!(cut_small, whole, "parts longer than a page are cut on characters and continued");
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], "_id,客户,地区,备注");
    assert_eq!(lines.len(), 31, "{pages} pages");
    assert_eq!(lines[1], "o00,客户0,华南,中文备注，一段比较长的说明文字");
    // one part by its selector; the rich text by blocks
    let one = body::read_body(ctx, &full(), &orders, &json!({ "selector": { "kind": "table_record", "record_id": "o01" } }), None).unwrap();
    assert!(one["text"].as_str().unwrap().ends_with("o01,客户1,华东,中文备注，一段比较长的说明文字中文备注，一段比较长的说明文字"), "{one}");
    let d = ctx.entity("doc").unwrap().unwrap();
    let b = body::read_body(ctx, &full(), &d, &json!({ "parts": true }), None).unwrap();
    assert_eq!(b["text"], "## 口径\n销售额按含税金额统计。");
    assert_eq!(b["parts"][1]["selector"], json!({ "kind": "richtext_block", "block_id": "p1" }));
    // a long note read in small pages: nothing lost, no character cut
    let long = "长".repeat(200);
    let mut ws2 = MemWorkspace::new();
    ws2.ok(json!([{ "op": "entity.create", "entity_id": "n", "type_id": TYPE_ANNOTATION, "parent_id": DATA_ID, "order_key": "a", "payload": { "kind": "note", "body": long } }]));
    let n = ws2.store.entity("n").unwrap().unwrap();
    let (mut all, mut cursor) = (String::new(), None::<String>);
    loop {
        let mut p = json!({ "max_bytes": 100 });
        if let Some(c) = &cursor {
            p["cursor"] = json!(c);
        }
        let b = body::read_body(&ws2.store, &full(), &n, &p, None).unwrap();
        assert!(b["text"].as_str().unwrap().len() <= 100);
        all.push_str(b["text"].as_str().unwrap());
        match b["next_cursor"].as_str() {
            Some(c) => cursor = Some(c.to_string()),
            None => break,
        }
    }
    assert_eq!(all, format!("Note\n{long}"));
    // the etag moves when what the body shows moves: an option renamed
    let before = body::etag(ctx, &full(), &orders, None).unwrap();
    let mut ws3 = ws;
    let def_rev = ws3.store.fields[&("orders".to_string(), "region".to_string())].def_rev;
    ws3.ok(json!([{ "op": "table.update_option", "source_id": "orders", "field_id": "region", "option_id": "east", "label": "东部", "expect": { "rev": def_rev } }]));
    let orders = ws3.store.entity("orders").unwrap().unwrap();
    assert_ne!(body::etag(&ws3.store, &full(), &orders, None).unwrap(), before);
}

#[test]
fn text_queries_find_what_bodies_show() {
    let ws = ws();
    let ctx = &ws.store;
    let never = || false;
    let q = search::text_query(ctx, &full(), &json!({ "text": "华东" }), 7, None, &never).unwrap();
    let hit = &q["hits"][0];
    assert_eq!((hit["on"].as_str(), hit["selector"]["kind"].as_str(), hit["selector"]["field_id"].as_str()), (Some("body"), Some("table_cell"), Some("region")), "{q}");
    let of_orders: Vec<&Value> = q["hits"].as_array().unwrap().iter().filter(|h| h["entity_id"] == "orders").collect();
    assert_eq!(of_orders.len(), 5);
    assert!(of_orders[4]["more"].as_u64().unwrap() > 0, "one entity gives at most five hits, the rest counted");
    assert_eq!(q["coverage"]["complete"], true);
    let q = search::text_query(ctx, &full(), &json!({ "text": "含税" }), 7, None, &never).unwrap();
    assert_eq!(q["hits"][0]["selector"], json!({ "kind": "richtext_block", "block_id": "p1" }));
    let q = search::text_query(ctx, &full(), &json!({ "text": "销售" }), 7, None, &never).unwrap();
    assert!(q["hits"].as_array().unwrap().iter().any(|h| h["entity_id"] == "sales-folder" && h["on"] == "name"), "{q}");
    // nothing there, completely covered: "not there"; a stop mid-way says so and resumes
    let q = search::text_query(ctx, &full(), &json!({ "text": "不存在的词" }), 7, None, &never).unwrap();
    assert_eq!((q["hits"].as_array().unwrap().len(), q["coverage"]["complete"].as_bool()), (0, Some(true)));
    let always = || true;
    let q = search::text_query(ctx, &full(), &json!({ "text": "不存在的词" }), 7, None, &always).unwrap();
    assert_eq!(q["coverage"]["complete"], false);
    assert!(q["next_cursor"].is_string());
    assert_eq!(search::text_query(ctx, &full(), &json!({ "text": "x", "regex": true }), 7, None, &never).unwrap_err().code.as_str(), "INVALID_OPERATION");
}

#[test]
fn meta_says_what_an_entity_is_without_reading_it() {
    let ws = ws();
    let ctx = &ws.store;
    let orders = ctx.entity("orders").unwrap().unwrap();
    let m = meta::meta(ctx, &full(), &orders, None).unwrap();
    assert_eq!((m["type"].as_str(), m["path"].as_str(), m["body"]["format"].as_str(), m["data"]["mode"].as_str()), (Some("table"), Some("/data/销售-2026/订单"), Some("text/csv"), Some("local")));
    assert_eq!(m["name_follows"], false, "订单 was given");
    assert!(m["ops"].as_array().unwrap().contains(&json!("table.insert_records")) && !m["ops"].as_array().unwrap().contains(&json!("richtext.insert_blocks")));
    let d = meta::meta(ctx, &full(), &ctx.entity("doc").unwrap().unwrap(), None).unwrap();
    assert_eq!(d["name_follows"], true);
}

#[test]
fn grouping_and_ungrouping_keep_following_names_apart() {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "s1-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a", "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s1" } },
        { "op": "entity.create", "entity_id": "s1", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a", "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s1-content", "title": "看板" } },
        { "op": "entity.create", "entity_id": "a", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "x1", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "label" } } },
        { "op": "entity.create", "entity_id": "b", "type_id": TYPE_CELL, "parent_id": "s1", "order_key": "x2", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 }, "payload": { "view": { "type": "shape" }, "title": "流程" } },
    ]));
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "g", "type_id": TYPE_CONTAINER, "parent_id": "s1", "order_key": "x3", "placement": { "x": 0, "y": 0, "w": 50, "h": 50 }, "payload": { "kind": "group", "layout": { "mode": "free" }, "title": "分组" } },
        { "op": "tree.move", "entity_id": "a", "new_parent_id": "g", "order_key": "a", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 } },
        { "op": "tree.move", "entity_id": "b", "new_parent_id": "g", "order_key": "b", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 } },
    ]));
    let life = ws.store.entities["g"].life_rev;
    let r = ws.commit(json!([
        { "op": "tree.move", "entity_id": "a", "new_parent_id": "s1", "order_key": "x4", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 } },
        { "op": "tree.move", "entity_id": "b", "new_parent_id": "s1", "order_key": "x5", "placement": { "x": 0, "y": 0, "w": 10, "h": 10 } },
        { "op": "entity.delete", "entity_id": "g", "expect": { "rev": life } },
    ]));
    assert!(r.is_ok(), "{}", r.err().unwrap().to_json());
    assert_eq!(ws.store.edges["b"].parent_id, "s1");
}
