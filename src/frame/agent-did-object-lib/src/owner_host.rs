use crate::zone::{RuntimeZoneAccess, ZoneAccess};
use crate::{AgentDIDObjectError as Error, XCallInput};
use buckyos_api::{obj_service_url, NotifySendReq, NotifySource};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type Result<T> = std::result::Result<T, Error>;

fn failure(code: &str, message: &str, detail: Value) -> Error {
    Error::Business {
        code: code.to_owned(),
        message: message.to_owned(),
        detail,
    }
}
fn field<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p[key]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| failure("INVALID_PARAMS", &format!("missing {key}"), Value::Null))
}
fn service_error(err: &Value, fallback: &str) -> Error {
    let message = err["message"]
        .as_str()
        .or_else(|| err["detail"].as_str())
        .or_else(|| err.as_str())
        .unwrap_or(fallback);
    let lower = message.to_ascii_lowercase();
    let code = err["code"].as_str().unwrap_or_else(|| {
        if lower.contains("idempotency_conflict") {
            "IDEMPOTENCY_CONFLICT"
        } else if lower.contains("denied") || lower.contains("permission") {
            "PERMISSION_DENIED"
        } else if lower.contains("invalid token") || lower.contains("session token") {
            "IDENTITY_UNAVAILABLE"
        } else if lower.contains("resource_binding_cycle") {
            "RESOURCE_BINDING_CYCLE"
        } else if lower.contains("provider_unavailable") {
            "PROVIDER_UNAVAILABLE"
        } else if lower.contains("provider_type_mismatch") {
            "PROVIDER_TYPE_MISMATCH"
        } else {
            fallback
        }
    });
    failure(code, message, err.clone())
}

async fn rpc(service: &str, method: &str, params: Value, write: bool) -> Result<Value> {
    let zone = RuntimeZoneAccess;
    let origin = zone.gateway_origin().ok_or_else(|| {
        failure(
            "IDENTITY_UNAVAILABLE",
            "Zone gateway unavailable",
            Value::Null,
        )
    })?;
    let token = zone
        .session_token()
        .await
        .ok_or_else(|| failure("IDENTITY_UNAVAILABLE", "Zone session required", Value::Null))?;
    let url = format!("{}/kapi/{service}", origin.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()?;
    let transport = || {
        failure(
            if write {
                "WRITE_OUTCOME_UNKNOWN"
            } else {
                "PROVIDER_UNAVAILABLE"
            },
            "Zone service did not confirm the request",
            json!({"method":method,"retryable":!write}),
        )
    };
    let response = client
        .post(url)
        .json(&json!({"method":method,"params":params,"sys":[0,token]}))
        .send()
        .await
        .map_err(|_| transport())?;
    match response.status().as_u16() {
        401 => {
            return Err(failure(
                "IDENTITY_UNAVAILABLE",
                "Zone session rejected",
                Value::Null,
            ))
        }
        403 => return Err(failure("PERMISSION_DENIED", "access denied", Value::Null)),
        n if !(200..300).contains(&n) => return Err(transport()),
        _ => {}
    }
    let reply: Value = response.json().await.map_err(|_| transport())?;
    if let Some(error) = reply.get("error").filter(|e| !e.is_null()) {
        let fallback = reply["code"].as_str().unwrap_or("SERVICE_ERROR");
        return Err(service_error(error, fallback));
    }
    let result = reply.get("result").cloned().ok_or_else(transport)?;
    if result["ok"] == false {
        return Err(service_error(&result["error"], "SERVICE_ERROR"));
    }
    Ok(result)
}

pub async fn context() -> Result<Value> {
    rpc("control-panel", "owner.context", json!({}), false).await
}
pub async fn alias_context(name: &str) -> Result<Value> {
    rpc(
        "control-panel",
        "owner.context",
        json!({"alias":name}),
        false,
    )
    .await
}

fn operation(ctx: &Value, input: &XCallInput, params: &Value) -> Result<Value> {
    let key = input
        .idempotency_key
        .as_deref()
        .or_else(|| ctx["invocation_id"].as_str())
        .unwrap_or("");
    if input.action != "list" && key.is_empty() {
        return Err(failure(
            "INVALID_PARAMS",
            "missing invocation identity",
            Value::Null,
        ));
    }
    let mut p = json!({"slot":field(params,"slot")?,"action":input.action,"object":input.object,"alias_revision":ctx["alias_revision"],
        "idempotency_key":key,"input_digest":format!("{:x}",Sha256::digest(buckyos_api::canonical_json(&input.params).as_bytes()))});
    for field in ["record_ref", "record_refs", "before", "after"] {
        if let Some(v) = input.params.get(field) {
            p[field] = v.clone()
        }
    }
    Ok(p)
}

fn qualify_record(record: &mut Value, binding: &str) -> Result<()> {
    if let Some(value) = record.get("record_ref") {
        let raw = value
            .as_str()
            .ok_or_else(|| failure("INVALID_RECEIPT", "record_ref must be a URL", Value::Null))?
            .to_owned();
        let reference = buckyos_api::owner_record_ref::scoped_owner_record_ref(&raw, binding)
            .ok_or_else(|| {
                failure(
                    "INVALID_RECEIPT",
                    "invalid provider record URL or binding",
                    Value::Null,
                )
            })?;
        record["provider_record_ref"] = json!(raw);
        record["record_ref"] = json!(reference);
    }
    Ok(())
}

async fn resource(ctx: &Value, input: &XCallInput, method: &str, params: &Value) -> Result<Value> {
    let mut p = operation(ctx, input, params)?;
    if method == "owner.pin" {
        p["workspace_id"] = json!(field(params, "workspace_id")?);
        p["epoch"] = json!(field(params, "epoch")?);
        return rpc("control-panel", method, p, true).await;
    }
    let mut target = rpc(
        "control-panel",
        "owner.prepare",
        p.clone(),
        input.action != "list",
    )
    .await?;
    if target["mode"] != "object" {
        return Ok(target);
    }
    let idempotency = p["idempotency_key"].as_str();
    let mut invocation = input.clone();
    if let Some(params) = target["provider_params"].as_object() {
        for field in ["record_ref", "record_refs", "before", "after"] {
            if let Some(value) = params.get(field) {
                invocation.params[field] = value.clone();
            }
        }
    }
    invocation.object = field(&target, "provider_ref")?.to_string();
    invocation.idempotency_key = idempotency.filter(|s| !s.is_empty()).map(str::to_owned);
    let adapter = crate::adapters::DidObjectProtocolAdapter::new("did-object".into());
    let slot = field(params, "slot")?;
    let mut result = if target["replay"] == true {
        adapter.recover_write(slot, &invocation).await?
    } else {
        let response = adapter.invoke_resource(slot, &invocation).await?;
        if let Some(err) = response.error {
            return Err(service_error(&json!(err), "PROVIDER_ERROR"));
        }
        response
            .result
            .ok_or_else(|| failure("INVALID_RECEIPT", "missing provider result", Value::Null))?
    };
    if !result.is_object() {
        return Err(failure(
            "INVALID_RECEIPT",
            "expected a personal resource receipt",
            Value::Null,
        ));
    }
    let marker = field(&target, "record_binding")?;
    if input.action == "list" {
        let key = if p["slot"] == "calendar" {
            "events"
        } else {
            "items"
        };
        let rows = result
            .get_mut(key)
            .and_then(Value::as_array_mut)
            .ok_or_else(|| {
                failure(
                    "INVALID_RECEIPT",
                    "provider did not return a list",
                    Value::Null,
                )
            })?;
        for row in rows {
            if row.get("record_ref").is_none() {
                return Err(failure(
                    "INVALID_RECEIPT",
                    "provider omitted the record reference",
                    Value::Null,
                ));
            }
            qualify_record(row, marker)?;
        }
    } else {
        qualify_record(&mut result, marker)?;
    }
    result["binding_revision"] = target["binding_revision"].take();
    result["owner_did"] = ctx["owner_did"].clone();
    Ok(json!({"result":result}))
}

async fn notify(ctx: &Value, input: &XCallInput, params: &Value) -> Result<Value> {
    if params
        .as_object()
        .is_none_or(|m| m.keys().any(|k| k != "text"))
    {
        return Err(failure(
            "INVALID_PARAMS",
            "only text is accepted",
            Value::Null,
        ));
    }
    let user = field(ctx, "owner_user_id")?;
    let text = field(params, "text")?;
    let key = input.idempotency_key.clone().unwrap_or_else(|| {
        let hour = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / 3600;
        format!("{hour}-{:x}", Sha256::digest(text.as_bytes()))
    });
    let transport_key = format!("xcall:owner:{}:{key}", input.action);
    let mut req = match input.action.as_str() {
        "ask" => NotifySendReq::input(user, text, text, Vec::new(), true, transport_key),
        "notify" => NotifySendReq::notice(user, text, text, transport_key),
        _ => {
            return Err(failure(
                "UNSUPPORTED_ACTION",
                "unexpected notification action",
                Value::Null,
            ))
        }
    };
    req.source = Some(NotifySource {
        session_id: input.session_id.clone(),
        call_id: input.trace_id.clone(),
        ..Default::default()
    });
    let mut result = rpc("msg-center", "notify.send", json!(req), true).await?;
    result["idempotency_key"] = json!(key);
    Ok(result)
}

async fn contact(ctx: &Value, method: &str, params: &Value) -> Result<Value> {
    let owner = field(ctx, "owner_did")?;
    if method == "contact.find" {
        let mut result = rpc(
            "msg-center",
            "contact.lookup",
            json!({"input":field(params,"name")?,"kinds":["user"],"contact_mgr_owner":owner}),
            false,
        )
        .await?;
        if let Some(list) = result["candidates"].as_array_mut() {
            for item in list {
                if let Some(did) = item["did"].as_str().map(str::to_owned) {
                    if item["relation"]["is_contact"] == true {
                        item["contact_ref"] = json!(obj_service_url(
                            "msg-center",
                            &["owners", owner, "contacts", &did]
                        ));
                    }
                    let visible = rpc(
                        "control-panel",
                        "owner.sharing.discover",
                        json!({"did":did}),
                        false,
                    )
                    .await?;
                    if let Some(r) = visible.get("availability_ref") {
                        item["availability_ref"] = r.clone();
                        item["zone_did"] = ctx["zone_did"].clone()
                    }
                }
            }
        }
        return Ok(result);
    }
    let did = field(params, "did")?;
    let mut p = json!({"did":did,"contact_mgr_owner":owner});
    let update = method == "contact.update";
    if update {
        p["patch"] = params["patch"].clone()
    }
    let v = rpc(
        "msg-center",
        if update {
            "contact.update_contact"
        } else {
            "contact.get_contact"
        },
        p,
        update,
    )
    .await?;
    if v.is_null() {
        return Err(failure(
            "CONTACT_NOT_FOUND",
            "this contact is absent from the owner's address book",
            Value::Null,
        ));
    }
    let mut out = json!({"record":v,"did":did,"contact_ref":obj_service_url("msg-center",&["owners",owner,"contacts",did])});
    if update {
        return Ok(out);
    }
    let visible = rpc(
        "control-panel",
        "owner.sharing.discover",
        json!({"did":did}),
        false,
    )
    .await?;
    if let Some(r) = visible.get("availability_ref") {
        out["availability_ref"] = r.clone();
        out["zone_did"] = ctx["zone_did"].clone()
    }
    Ok(out)
}

pub(crate) fn writes(input: &XCallInput, method: &str) -> bool {
    matches!(
        method,
        "notify.send"
            | "contact.update"
            | "binding.set"
            | "binding.reset"
            | "sharing.grant"
            | "sharing.revoke"
            | "aiws.doc.commit"
            | "aiws.ws.ensure"
            | "owner.pin"
    ) || (method == "owner.prepare" && input.action != "list")
}

pub async fn call(ctx: &Value, input: &XCallInput, method: &str, params: Value) -> Result<Value> {
    let write = writes(input, method);
    if write && ctx["read_only"].as_bool() != Some(false) {
        return Err(failure(
            "PERMISSION_DENIED",
            "this host call requires a writable session",
            Value::Null,
        ));
    }
    match method {
        "owner.who" => Ok(
            json!({"owner_did":ctx["owner_did"],"caller_did":ctx["caller_did"],"zone_did":ctx["zone_did"]}),
        ),
        "owner.resolve" | "binding.get" | "binding.set" | "binding.reset" | "sharing.grant"
        | "sharing.revoke" => {
            let m = if method == "owner.resolve" {
                method.to_string()
            } else {
                format!("owner.{method}")
            };
            rpc(
                "control-panel",
                &m,
                params,
                !matches!(method, "owner.resolve" | "binding.get"),
            )
            .await
        }
        "notify.send" => notify(ctx, input, &params).await,
        "notify.get" => {
            rpc(
                "msg-center",
                "notify.get",
                json!({"notify_id":field(&params,"notify_id")?}),
                false,
            )
            .await
        }
        "contact.find" | "contact.get" | "contact.update" => contact(ctx, method, &params).await,
        "owner.prepare" | "owner.pin" => resource(ctx, input, method, &params).await,
        "aiws.ws.lookup"
        | "aiws.ws.ensure"
        | "aiws.ws.get_info"
        | "aiws.doc.query"
        | "aiws.doc.read"
        | "aiws.doc.commit"
        | "aiws.doc.get_submission" => {
            rpc(
                "aiworkspace",
                &method[5..],
                params,
                matches!(method, "aiws.ws.ensure" | "aiws.doc.commit"),
            )
            .await
        }
        _ => Err(failure(
            "UNSUPPORTED_HOST_CALL",
            "unrecognized personal resource host call",
            Value::Null,
        )),
    }
}
