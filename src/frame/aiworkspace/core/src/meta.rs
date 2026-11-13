//! The common `meta` of an entity (核心 API 调整 §2.2; design draft §3): identity, place, versions, what reading its
//! body gives, where its data comes from, what it shows, what the caller may do and which operations its type takes.
//! Reading it never reads a table's records or a body's text (the body `etag` of a table comes from its versions).
//! The service adds the Workspace-level parts (`ref`, `url`, `epoch`, `head_seq`).

use crate::access::Access;
use crate::body::{describe, AssetLoader, BODY_FORMAT_VERSION};
use crate::error::WsResult;
use crate::model::*;
use crate::path::{path_of, short_type};
use serde_json::{json, Value};

fn shown(ctx: &dyn ReadCtx, access: &Access, id: &str) -> WsResult<Value> {
    Ok(match ctx.entity(id)? {
        Some(x) if x.alive() && access.can_read(ctx, &x)? => json!({ "entity_id": id, "path": path_of(ctx, access, id)?, "type": short_type(&x.type_id) }),
        _ => json!({ "entity_id": id, "available": false }),
    })
}

pub fn meta(ctx: &dyn ReadCtx, access: &Access, e: &EntityRow, assets: Option<AssetLoader>) -> WsResult<Value> {
    let edge = ctx.edge(&e.entity_id)?;
    let parent = match &edge {
        Some(edge) => match ctx.entity(&edge.parent_id)? {
            Some(p) if p.alive() && access.can_read(ctx, &p)? => json!({ "entity_id": p.entity_id, "path": path_of(ctx, access, &p.entity_id)? }),
            _ => Value::Null,
        },
        None => Value::Null,
    };
    let mut children = 0u64;
    for k in ctx.children(&e.entity_id)? {
        if let Some(x) = ctx.entity(&k.child_id)? {
            if x.alive() && access.can_read(ctx, &x)? {
                children += 1;
            }
        }
    }
    let d = describe(ctx, e)?;
    let mut body = json!({ "state": d.state, "format": d.format, "format_version": BODY_FORMAT_VERSION,
                           "etag": crate::body::etag(ctx, access, e, assets)?, "records": Value::Null, "bytes": Value::Null });
    if let Some(r) = d.reason {
        body["reason"] = json!(r);
    }
    if e.type_id == TYPE_ASSET {
        body["bytes"] = e.payload.get("size").cloned().unwrap_or(Value::Null);
    }
    let kind = match e.type_id.as_str() {
        TYPE_CONTAINER => e.payload.get("kind").cloned().unwrap_or(Value::Null),
        TYPE_CELL => e.payload.get("view").and_then(|v| v.get("type")).cloned().unwrap_or(Value::Null),
        TYPE_ANNOTATION => e.payload.get("kind").cloned().unwrap_or(Value::Null),
        _ => Value::Null,
    };
    let mut v = json!({
        "entity_id": e.entity_id,
        "path": path_of(ctx, access, &e.entity_id)?,
        "name": e.name,
        "name_follows": crate::naming::entity_follows(e),
        "title": crate::naming::title_source(&e.type_id, &e.payload),
        "type": short_type(&e.type_id), "type_id": e.type_id, "kind": kind,
        "schema_id": e.schema_id, "schema_version": e.schema_version,
        "scope": if e.owner().is_some() { "personal" } else if e.scope == SCOPE_PRESENTER { "presenter" } else if e.participant().is_some() { "participant" } else { "shared" },
        "parent": parent,
        "version": { "content_rev": e.content_rev, "meta_rev": e.meta_rev, "life_rev": e.life_rev, "struct_rev": edge.as_ref().map(|x| x.struct_rev) },
        "body": body,
        "children": children,
        "capabilities": access.entity_caps(ctx, e)?.names(),
        "ops": crate::plan::operations_for(&e.type_id),
    });
    if let Some(d) = e.degraded() {
        v["degraded"] = json!(d);
    }
    // where the data comes from: only states that exist (D12 sources, URL tables)
    if crate::model::is_data_type(&e.type_id) {
        let mode = if crate::types::is_url_table(e) {
            "url_query"
        } else if e.payload.get("source").is_some_and(Value::is_object) {
            "synced"
        } else {
            "local"
        };
        let mut data = json!({ "mode": mode });
        if let Some(s) = e.payload.get("source").filter(|s| s.is_object()) {
            let mut src = serde_json::Map::new();
            for k in ["kind", "mode", "url", "workspace_id", "entity_id", "refresh", "revision", "fetched_at", "refreshed_at", "error"] {
                if let Some(x) = s.get(k) {
                    src.insert(k.into(), x.clone());
                }
            }
            data["source"] = Value::Object(src);
        }
        if let Some(d) = &e.derived {
            data["derived"] = json!({ "wish_id": d.get("wish_id"), "run_id": d.get("run_id"), "at": d.get("at") });
        }
        v["data"] = data;
    }
    // what a Block or a data control shows
    if e.type_id == TYPE_CELL {
        if let Some(id) = e.payload.get("source_ref").and_then(crate::value::reference_entity_id) {
            let mut s = shown(ctx, access, id)?;
            crate::sqlview::describe_cell_view(ctx, access, e, &mut s)?;
            v["shows"] = s;
        }
    } else if let Some(b) = e.payload.get("bindings").and_then(Value::as_object) {
        let mut m = serde_json::Map::new();
        for (k, x) in b {
            m.insert(
                k.clone(),
                match x {
                    Value::Array(list) => Value::Array(list.iter().filter_map(|i| i.get("entity_id").and_then(Value::as_str)).map(|id| shown(ctx, access, id)).collect::<WsResult<_>>()?),
                    _ => match x.get("entity_id").and_then(Value::as_str) {
                        Some(id) => shown(ctx, access, id)?,
                        None => Value::Null,
                    },
                },
            );
        }
        v["shows"] = Value::Object(m);
    }
    if let Some(d) = e.payload.get("def_ref").and_then(crate::value::reference_entity_id) {
        v["definition"] = shown(ctx, access, d)?;
    }
    Ok(v)
}
