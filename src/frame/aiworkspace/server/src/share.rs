use crate::{blocking, entry_admits, prepare_caller, s, AppState};
use aiworkspace_core::{access::Cap, WsError, WsResult};
use aiworkspace_store::entries::{parse_entry_methods, split_entry_id, ENTRY_GUEST};
use aiworkspace_store::{Caller, CallerClass};
use buckyos_api::{get_buckyos_api_runtime, AuthMethod};
use serde_json::{json, Value};
use std::sync::Arc;

pub(crate) async fn issue(state: &Arc<AppState>, p: &Value, caller: &Caller) -> WsResult<Value> {
    if !matches!(
        caller.class,
        CallerClass::Account { .. } | CallerClass::External { .. }
    ) {
        return Err(WsError::denied(
            "a signed-in person must create the invitation",
        ));
    }
    let ws_id = s(p, "workspace_id")?.to_owned();
    let request = s(p, "request_id")?;
    if request.is_empty() || request.len() > 128 {
        return Err(WsError::invalid_op("invalid request_id"));
    }
    let audience = &p["audience"];
    if !matches!(audience["kind"].as_str(), Some("directed" | "open")) {
        return Err(WsError::invalid_op("choose an invitation audience"));
    }
    let methods = parse_entry_methods(&p["methods"])?;
    if audience["kind"] == "directed" && methods & ENTRY_GUEST != 0 {
        return Err(WsError::invalid_op(
            "a directed invitation cannot admit anonymous guests",
        ));
    }
    let integer = |key: &str| match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| WsError::invalid_op(format!("invalid {key}"))),
    };
    let expiry = integer("expires_at")?;
    let max = integer("max_joins")?;
    let group = s(p, "group_id")?;
    let path = match p.get("return_path").and_then(Value::as_str) {
        Some(path) => path.to_owned(),
        None => state.with_ws(&ws_id, |ws| Ok(format!("/{}", ws.workspace_slug())))?,
    };
    let (id, secret, url) = state.with_ws(&ws_id, |ws| {
        ws.share_request(caller, request, p, group, methods, expiry, max)
    })?;
    let local = split_entry_id(&id)
        .ok_or_else(|| WsError::invalid_op("invalid entry"))?
        .1;
    let url = match url {
        Some(url) => url,
        None => {
            let client = get_buckyos_api_runtime()
                .map_err(|_| WsError::io("Zone unavailable"))?
                .get_zone_service_krpc_client("control-panel")
                .await
                .map_err(|_| WsError::io("Control Panel unavailable"))?;
            let reply = client.call("resource.share.issue",json!({"request_id":id,"resource_id":id,"audience":audience,"callback_secret":secret,"return_path":path,"recipient_email":p.get("recipient_email")})).await
                .map_err(|e| WsError::invalid_op(format!("share registration: {e}")))?;
            let url = s(&reply, "url")?.to_owned();
            state.with_ws(&ws_id, |ws| ws.save_share_url(local, &url))?;
            url
        }
    };
    let mut entry = state.with_ws(&ws_id, |ws| {
        ws.entry(local)?
            .map(|e| e.to_json())
            .ok_or_else(|| WsError::not_found("entry unavailable"))
    })?;
    entry["share_url"] = json!(url);
    Ok(json!({"ok":true,"entry":entry}))
}

pub(crate) async fn callback(state: &Arc<AppState>, p: &Value) -> WsResult<Value> {
    let (ws_id, local) = split_entry_id(s(p, "resource_id")?)
        .ok_or_else(|| WsError::not_found("entry unavailable"))?;
    let secret = s(p, "callback_secret")?;
    if p["operation"] == "resume" {
        return state.with_ws(ws_id, |ws| {
            let entry = ws.check_share_callback(local, secret, false)?;
            Ok(json!({"ok":ws.has_share_grant(local,&entry,s(p,"did")?)?}))
        });
    }
    let entry = state.with_ws(ws_id, |ws| ws.check_share_callback(local, secret, true))?;
    let mut sharer = state
        .auth
        .share_author(&entry.created_by)
        .await
        .map_err(|_| WsError::denied("share author unavailable"))?;
    if !matches!(
        sharer.class,
        CallerClass::Account { .. } | CallerClass::External { .. }
    ) {
        return Err(WsError::denied("share author unavailable"));
    }
    prepare_caller(state, &mut sharer, "ws.create_entry", Some(ws_id)).await;
    let settings = state
        .auth
        .sso_settings()
        .await
        .map_err(|_| WsError::denied("Zone sign-in settings unavailable"))?;
    let (st, id, local, secret, params) = (
        state.clone(),
        ws_id.to_owned(),
        local.to_owned(),
        secret.to_owned(),
        p.clone(),
    );
    blocking(move || {
        st.with_ws(&id, |ws| {
            let entry = ws.check_share_callback(&local, &secret, true)?;
            ws.require_ws(&sharer, Cap::Manage)?;
            let acl = ws.acl_data()?;
            let caps = acl.allowed_anywhere(&format!("@{}", entry.group_id));
            if !acl.groups.contains_key(&entry.group_id)
                || caps.is_empty()
                || caps.has(Cap::Manage)
                || caps.has(Cap::Run)
            {
                return Err(WsError::denied("share group unavailable"));
            }
            match s(&params, "operation")? {
                "inspect" => Ok(json!({
                    "ok": true,
                    "sharer": entry.created_by,
                    "title": ws.title(),
                    "operations": caps.names(),
                    "methods": crate::entry_methods(entry.methods),
                    "expires_at": entry.expires_at
                })),
                operation @ ("prepare" | "commit") => {
                    let did = s(&params, "did")?;
                    let method = AuthMethod::parse(s(&params, "auth_method")?)
                        .ok_or_else(|| WsError::denied("unsupported sign-in method"))?;
                    entry_admits(entry.methods, &settings, Some(method))?;
                    ws.share_grant(&local, &entry, did, operation == "commit")?;
                    Ok(json!({"ok": true}))
                }
                _ => Err(WsError::invalid_op("unknown share operation")),
            }
        })
    })
    .await
}
