//! DID Object Provider of msg-center (notepads/agent-object-url-todo.md §4.2 / §4.3):
//! `obj://msg-center/owners/<owner_did>/{sessions[/<sid>],contacts[/<did>]}` and
//! `obj://msg-center/messages/<msg_id>`, served at `/kapi/msg-center/obj/...`. Every
//! property and query is the kRPC method with the caller's own token, so the owner
//! binding and RBAC are those of `msg.*` / `contact.*`.

use std::sync::{Arc, Weak};

use buckyos_api::{obj_service_url, Captures, KrpcObjectProvider, ObjectKind, RpcBinding, RpcHandlerBackend};
use buckyos_http_server::DIDObjectError;
use kRPC::RPCHandler;
use name_lib::DID;
use serde_json::{json, Value};

const SERVICE: &str = "msg-center";
const BOX_EVENTS: [(&str, &str); 4] = [
    ("inbox", "A record of the INBOX box changed"),
    ("sent", "A record of the SENT box changed"),
    ("group_inbox", "A record of the GROUP_INBOX box changed"),
    ("request_box", "A record of the REQUEST_BOX box changed"),
];

fn cap(captures: &Captures, name: &str) -> String {
    captures.get(name).cloned().unwrap_or_default()
}

fn page_size(input: &Value, default: u64) -> u64 {
    input
        .get("page_size")
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(1, 200)
}

fn invalid_cursor() -> DIDObjectError {
    DIDObjectError::new("invalid_params", "invalid cursor")
}

fn session_url(owner: &str, session: &str) -> String {
    obj_service_url(SERVICE, &["owners", owner, "sessions", session])
}

fn session_rows(captures: &Captures, page: Value) -> Value {
    let owner = cap(captures, "owner");
    let rows: Vec<Value> = page["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let sid = item["session_id"].as_str().unwrap_or_default().to_string();
            json!({
                "row_id": sid,
                "object": session_url(&owner, &sid),
                "version": item["updated_at_ms"],
                "cells": {
                    "unread_count": item["unread_count"],
                    "updated_at_ms": item["updated_at_ms"],
                    "last_activity_ms": item["last_activity_ms"],
                },
            })
        })
        .collect();
    let next = match (page["next_cursor_updated_at_ms"].as_u64(), page["next_cursor_session_id"].as_str()) {
        (Some(ms), Some(sid)) => json!(format!("{ms}:{sid}")),
        _ => Value::Null,
    };
    json!({ "rows": rows, "next_cursor": next })
}

fn message_rows(page: Value) -> Value {
    let rows: Vec<Value> = page["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|item| {
            let msg_id = item["msg_id"]
                .as_str()
                .or_else(|| item["record"]["msg_id"].as_str())
                .unwrap_or_default()
                .to_string();
            json!({
                "row_id": item["record_id"].as_str().unwrap_or(&msg_id),
                "object": obj_service_url(SERVICE, &["messages", &msg_id]),
                "cells": item,
            })
        })
        .collect();
    let next = match (page["next_cursor_sort_key"].as_u64(), page["next_cursor_record_id"].as_str()) {
        (Some(key), Some(record)) => json!(format!("{key}:{record}")),
        _ => Value::Null,
    };
    json!({ "rows": rows, "next_cursor": next })
}

fn split_cursor(input: &Value) -> Result<Option<(u64, String)>, DIDObjectError> {
    let Some(cursor) = input.get("cursor").and_then(Value::as_str) else {
        return Ok(None);
    };
    let (key, rest) = cursor.split_once(':').ok_or_else(invalid_cursor)?;
    Ok(Some((key.parse().map_err(|_| invalid_cursor())?, rest.to_string())))
}

fn contact_rows(captures: &Captures, contacts: Value) -> Value {
    let owner = cap(captures, "owner");
    let rows: Vec<Value> = contacts
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|contact| {
            let did = contact["did"].as_str().unwrap_or_default().to_string();
            json!({
                "row_id": did,
                "object": obj_service_url(SERVICE, &["owners", &owner, "contacts", &did]),
                "cells": { "name": contact["name"], "tags": contact["tags"], "friend_state": contact["friend_state"] },
            })
        })
        .collect();
    json!({ "rows": rows })
}

fn with_box_events(mut kind: ObjectKind) -> ObjectKind {
    for (name, description) in BOX_EVENTS {
        kind = kind.event(name, description);
    }
    kind
}

pub(crate) fn object_kinds() -> Vec<ObjectKind> {
    let object = json!({ "type": "object" });
    let sessions = ObjectKind::new(
        "owners/{owner}/sessions",
        "msg-center.sessions",
        "Message sessions",
        "Every session of one mailbox owner, newest activity first. Rows link to the session objects.",
    )
    .summary_property(
        "recent",
        "The 20 most recently updated sessions",
        object.clone(),
        RpcBinding::new("msg.list_sessions", |c, _| Ok(json!({ "owner": cap(c, "owner"), "limit": 20 })))
            .map(session_rows),
    )
    .index(
        "List the owner's sessions",
        json!({ "unread_count": "integer", "updated_at_ms": "integer", "last_activity_ms": "integer" }),
        RpcBinding::new("msg.list_sessions", |c, input| {
            let cursor = split_cursor(input)?;
            Ok(json!({
                "owner": cap(c, "owner"),
                "limit": page_size(input, 50),
                "cursor_updated_at_ms": cursor.as_ref().map(|c| c.0),
                "cursor_session_id": cursor.map(|c| c.1),
            }))
        })
        .map(session_rows),
    );
    let session = ObjectKind::new(
        "owners/{owner}/sessions/{session}",
        "msg-center.session",
        "Message session",
        "One conversation of a mailbox owner: its state and merged timeline (inbound, outbound, delivery state).",
    )
    .summary_property(
        "state",
        "Session state (title, members, lifecycle, read position)",
        object.clone(),
        RpcBinding::new("msg.get_session_state", |c, _| {
            Ok(json!({ "owner": cap(c, "owner"), "session_id": cap(c, "session") }))
        }),
    )
    .summary_property(
        "recent_messages",
        "The 20 latest messages of the session, newest first",
        object.clone(),
        RpcBinding::new("msg.list_session", |c, _| {
            Ok(json!({ "owner": cap(c, "owner"), "session_id": cap(c, "session"), "limit": 20, "descending": true, "with_object": true }))
        })
        .map(|_, page| message_rows(page)),
    )
    .index(
        "Page through the session timeline, newest first",
        json!({ "record": "object" }),
        RpcBinding::new("msg.list_session", |c, input| {
            let cursor = split_cursor(input)?;
            Ok(json!({
                "owner": cap(c, "owner"),
                "session_id": cap(c, "session"),
                "limit": page_size(input, 50),
                "cursor_sort_key": cursor.as_ref().map(|c| c.0),
                "cursor_record_id": cursor.map(|c| c.1),
                "descending": true,
                "with_object": true,
            }))
        })
        .map(|_, page| message_rows(page)),
    );
    let contacts = ObjectKind::new(
        "owners/{owner}/contacts",
        "msg-center.contacts",
        "Contacts",
        "The address book of one owner. Rows link to the contact records.",
    )
    .summary_property(
        "contacts",
        "Every contact of the owner",
        object.clone(),
        RpcBinding::new("contact.list_contacts", |c, _| {
            Ok(json!({ "query": {}, "contact_mgr_owner": cap(c, "owner") }))
        })
        .map(contact_rows),
    )
    .index(
        "Search the owner's contacts",
        json!({ "name": "string", "tags": "array", "friend_state": "string" }),
        RpcBinding::new("contact.list_contacts", |c, input| {
            let query = input.get("query").cloned().unwrap_or_else(|| json!({}));
            Ok(json!({ "query": query, "contact_mgr_owner": cap(c, "owner") }))
        })
        .map(contact_rows),
    )
    .event("changed", "The address book changed");
    let contact = ObjectKind::new(
        "owners/{owner}/contacts/{contact}",
        "msg-center.contact",
        "Contact record",
        "The owner's record of one contact. It is not the contact's own identity object.",
    )
    .summary_property(
        "record",
        "The contact record (name, tags, bindings, friend state)",
        object.clone(),
        RpcBinding::new("contact.get_contact", |c, _| {
            Ok(json!({ "did": cap(c, "contact"), "contact_mgr_owner": cap(c, "owner") }))
        }),
    )
    .property(
        "preferred_binding",
        "The account binding messages to this contact go through",
        object.clone(),
        RpcBinding::new("contact.get_preferred_binding", |c, _| {
            Ok(json!({ "did": cap(c, "contact"), "contact_mgr_owner": cap(c, "owner") }))
        }),
    );
    let message = ObjectKind::new(
        "messages/{msg_id}",
        "msg-center.message",
        "Message",
        "One message object. Attachments are content objects: read them by their cyfs:// ObjId references.",
    )
    .summary_property(
        "message",
        "The message object",
        object,
        RpcBinding::new("msg.get_message", |c, _| Ok(json!({ "msg_id": cap(c, "msg_id") }))),
    );
    vec![
        with_box_events(sessions),
        with_box_events(session),
        contacts,
        contact,
        message,
    ]
}

pub(crate) fn provider<H: RPCHandler + Send + Sync + 'static>(
    handler: Weak<H>,
    zone_did: DID,
) -> KrpcObjectProvider {
    KrpcObjectProvider::new(SERVICE, zone_did, object_kinds(), Arc::new(RpcHandlerBackend(handler)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_http_server::{HttpServer, StreamInfo};
    use bytes::Bytes;
    use http_body_util::{BodyExt, Full};
    use kRPC::{RPCErrors, RPCRequest, RPCResponse};
    use std::net::IpAddr;

    struct NoRpc;

    #[async_trait::async_trait]
    impl RPCHandler for NoRpc {
        async fn handle_rpc_call(&self, _req: RPCRequest, _ip: IpAddr) -> Result<RPCResponse, RPCErrors> {
            Err(RPCErrors::NoPermission("test".into()))
        }
    }

    #[tokio::test]
    async fn every_object_kind_has_a_valid_profile_and_guards_its_card() {
        let rpc = Arc::new(NoRpc);
        let server = provider(Arc::downgrade(&rpc), DID::new("web", "test.buckyos.io")).into_http_server();
        for path in [
            "owners/did:bns:alice/sessions",
            "owners/did:bns:alice/sessions/chat-1",
            "owners/did:bns:alice/contacts",
            "owners/did:bns:alice/contacts/did:bns:bob",
            "messages/cymsg:00ff",
        ] {
            let get = |query: &str| {
                http::Request::builder()
                    .uri(format!("/kapi/msg-center/obj/{path}{query}"))
                    .header("host", "test.buckyos.io")
                    .body(Full::new(Bytes::new()).map_err(|never| match never {}).boxed())
                    .unwrap()
            };
            let resp = server.serve_request(get("?type=profile"), StreamInfo::default()).await.unwrap();
            assert_eq!(resp.status(), 200, "{path}");
            let body = resp.into_body().collect().await.unwrap().to_bytes();
            let profile = name_lib::ObjectProfile::from_value(serde_json::from_slice(&body).unwrap()).unwrap();
            profile.validate().unwrap();
            let card = server.serve_request(get(""), StreamInfo::default()).await.unwrap();
            assert_eq!(card.status(), 403, "{path}: the Card needs a readable object");
        }
    }
}
