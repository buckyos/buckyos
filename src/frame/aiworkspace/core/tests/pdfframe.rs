//! The PDF Frame control (打印 TODO §3.3, T3): `buckyos.container { kind: "pdf-frame", config }` holds its two bands —
//! groups slotted `header` and `footer`, once each — with the Blocks of the header and the footer inside them; its
//! config is checked here, its size is numeric, and it is projected into the outline like any control.

use aiworkspace_core::model::*;
use aiworkspace_core::testkit::MemWorkspace;
use serde_json::{json, Value};

fn code(v: &Value) -> &str {
    v["code"].as_str().unwrap_or("")
}

fn sub(v: &Value) -> &str {
    v["sub_code"].as_str().unwrap_or("")
}

fn surface() -> MemWorkspace {
    let mut ws = MemWorkspace::new();
    ws.ok(json!([
        { "op": "entity.create", "entity_id": "s-content", "type_id": TYPE_CONTAINER, "parent_id": CANVAS_CONTENT_ID, "order_key": "a",
          "payload": { "kind": "folder", "system": "surface_content", "surface_id": "s" } },
        { "op": "entity.create", "entity_id": "s", "type_id": TYPE_CONTAINER, "parent_id": SURFACES_ID, "order_key": "a",
          "payload": { "kind": "surface", "layout": { "mode": "free" }, "content_folder_id": "s-content" } },
    ]));
    ws
}

fn frame(config: Value, placement: Value) -> Value {
    json!({ "op": "entity.create", "entity_id": "pf", "type_id": TYPE_CONTAINER, "parent_id": "s", "order_key": "m", "placement": placement,
            "payload": { "kind": "pdf-frame", "title": "Report", "config": config } })
}

fn band(id: &str, key: &str, slot: Option<&str>) -> Value {
    let mut payload = json!({ "kind": "group" });
    if let Some(s) = slot {
        payload["slot"] = json!(s);
    }
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CONTAINER, "parent_id": "pf", "order_key": key,
            "placement": { "x": 0, "y": 0, "w": 595, "h": 56 }, "payload": payload })
}

fn label(id: &str, parent: &str, title: &str) -> Value {
    json!({ "op": "entity.create", "entity_id": id, "type_id": TYPE_CELL, "parent_id": parent, "order_key": "a",
            "placement": { "x": 248, "y": 16, "w": 100, "h": 24 }, "payload": { "view": { "type": "label" }, "title": title, "config": { "align": "center" } } })
}

fn numeric() -> Value {
    json!({ "x": 40, "y": 40, "w": 595, "h": 842 })
}

fn config() -> Value {
    json!({ "page": "A", "orientation": "portrait", "header": 56, "footer": 56, "margin": 56 })
}

#[test]
fn a_frame_holds_its_two_bands_and_their_blocks() {
    let mut ws = surface();
    ws.ok(json!([frame(config(), numeric()), band("pf-h", "a", Some("header")), band("pf-f", "b", Some("footer")), label("pn", "pf-f", "{page} / {pages}")]));
    let pf = &ws.store.entities["pf"];
    assert_eq!(pf.payload["layout"], json!({ "mode": "free" }), "the layout is implied by the kind");
    let mut env = json!({ "kind": "pdf-frame" });
    aiworkspace_core::read::outline_extras(&mut env, pf);
    assert_eq!(env["config"]["page"], "A", "the outline carries the config (the clients lay the frame out from it)");
    let mut env = json!({ "kind": "group" });
    aiworkspace_core::read::outline_extras(&mut env, &ws.store.entities["pf-h"]);
    assert_eq!(env["slot"], "header");
    // a custom shape, bands of 0
    let rev = ws.store.entities["pf"].key_rev("config");
    ws.ok(json!([{ "op": "entity.set_keys", "entity_id": "pf", "keys": [{ "key": "config", "value": { "page": "custom", "custom_w": 100, "custom_h": 50.5, "header": 0, "footer": 0, "margin": 0 }, "expect": { "rev": rev } }] }]));
}

#[test]
fn only_bands_under_a_frame_each_once() {
    let mut ws = surface();
    ws.ok(json!([frame(config(), numeric()), band("pf-h", "a", Some("header"))]));
    // a Block directly under the frame, or a control: not a band
    let f = ws.fail(json!([label("x", "pf", "loose")]));
    assert_eq!(sub(&f), "CHILD_NOT_ALLOWED", "{f}");
    let f = ws.fail(json!([{ "op": "entity.create", "entity_id": "mm", "type_id": TYPE_CONTAINER, "parent_id": "pf", "order_key": "c",
        "placement": { "x": 0, "y": 0, "w": "auto", "h": "auto" }, "payload": { "kind": "mindmap", "slot": "footer" } }]));
    assert_eq!(sub(&f), "CHILD_NOT_ALLOWED", "{f}");
    // a group without a band's slot, with another slot, or a second header
    assert_eq!(code(&ws.fail(json!([band("g", "c", None)]))), "INVALID_SCHEMA");
    assert_eq!(code(&ws.fail(json!([band("g", "c", Some("body"))]))), "INVALID_SCHEMA");
    let f = ws.fail(json!([band("g", "c", Some("header"))]));
    assert_eq!(sub(&f), "SLOT_CONFLICT", "{f}");
    ws.ok(json!([band("pf-f", "b", Some("footer"))]));
    // a band stays one: its slot cannot go
    let rev = ws.store.entities["pf-f"].key_rev("slot");
    assert_eq!(code(&ws.fail(json!([{ "op": "entity.unset_keys", "entity_id": "pf-f", "keys": [{ "key": "slot", "expect": { "rev": rev } }] }]))), "INVALID_SCHEMA");
    // inside a band, Blocks are free (no slot needed) and groups may nest
    ws.ok(json!([label("t", "pf-h", "{title}"), { "op": "entity.create", "entity_id": "g2", "type_id": TYPE_CONTAINER, "parent_id": "pf-h", "order_key": "b",
        "placement": { "x": 400, "y": 8, "w": 120, "h": 40 }, "payload": { "kind": "group" } }]));
}

#[test]
fn a_frames_config_and_size_are_checked() {
    let mut ws = surface();
    for bad in [
        json!({ "page": "B5" }),
        json!({ "orientation": "upright" }),
        json!({ "header": -1 }),
        json!({ "margin": 20001 }),
        json!({ "custom_w": 0 }),
        json!({ "footer": "56" }),
        json!({ "paper": "A" }),
    ] {
        let f = ws.fail(json!([frame(bad.clone(), numeric())]));
        assert_eq!(code(&f), "INVALID_SCHEMA", "{bad}: {f}");
    }
    // its size is the user's (never computed): "auto" is refused; its bands' x / y are the frame's to place
    let f = ws.fail(json!([frame(config(), json!({ "x": 0, "y": 0, "w": "auto", "h": "auto" }))]));
    assert_eq!(code(&f), "INVALID_SCHEMA", "{f}");
    ws.ok(json!([frame(config(), numeric()), { "op": "entity.create", "entity_id": "pf-h", "type_id": TYPE_CONTAINER, "parent_id": "pf", "order_key": "a",
        "placement": { "x": "auto", "y": "auto", "w": 595, "h": 56 }, "payload": { "kind": "group", "slot": "header" } }]));
    // Blocks inside a band are placed by hand
    let mut auto = label("t", "pf-h", "x");
    auto["placement"] = json!({ "x": "auto", "y": "auto", "w": 100, "h": 24 });
    assert_eq!(code(&ws.fail(json!([auto]))), "INVALID_SCHEMA");
}
