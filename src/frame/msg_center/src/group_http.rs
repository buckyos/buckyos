use crate::group_types::*;
use crate::msg_center::MessageCenter;
use buckyos_api::MailboxAddress;
use buckyos_http_server::ServerError;
use bytes::Bytes;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use kRPC::RPCContext;
use name_lib::DID;
use ndn_lib::*;
use serde_json::{json, Value};

type Body = BoxBody<Bytes, ServerError>;
pub(crate) fn group_target(host: &str, path: &str) -> Result<String> {
    let base = normalize_cyfs_dispatch_target(host, "/inbox").map_err(invalid)?;
    if !path.starts_with('/')
        || path.contains('?')
        || path.contains('#')
        || path.contains("//")
        || path.split('/').any(|s| matches!(s, "." | ".." | "@"))
    {
        return Err(invalid("invalid-group-path"));
    }
    let mut url = reqwest::Url::parse(&base).map_err(invalid)?;
    url.set_path(path);
    Ok(url.to_string())
}
fn reply(status: StatusCode, value: Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("content-type", "application/json")
        .header("cache-control", "no-store")
        .body(
            Full::new(Bytes::from(value.to_string()))
                .map_err(|e| match e {})
                .boxed(),
        )
        .unwrap()
}
fn error_details(e: kRPC::RPCErrors) -> (StatusCode, String, bool) {
    match e {
        kRPC::RPCErrors::NoPermission(r) => (
            if r == "not-found" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::FORBIDDEN
            },
            r,
            false,
        ),
        kRPC::RPCErrors::ParseRequestError(r) => (StatusCode::BAD_REQUEST, r, false),
        e => (StatusCode::SERVICE_UNAVAILABLE, e.to_string(), true),
    }
}
fn error(e: kRPC::RPCErrors) -> Response<Body> {
    let (status, reason, retryable) = error_details(e);
    reply(status, json!({"error":reason,"retryable":retryable}))
}
fn request_error(e: kRPC::RPCErrors, req: &Request<Body>, target: &str) -> Response<Body> {
    if req.method() == Method::PUT && req.uri().path().ends_with("/inbox") {
        let (status, reason, retryable) = error_details(e);
        let id = req
            .headers()
            .get(CYFS_HEADER_OBJ_ID)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| ObjId::new(s).ok());
        crate::cyfs_dispatch::response(
            status,
            &CyfsDispatchResult::rejected(id, target.into(), reason, retryable),
        )
    } else {
        error(e)
    }
}
fn query(req: &Request<Body>) -> std::collections::BTreeMap<String, String> {
    let url = reqwest::Url::parse(&format!("http://local{}", req.uri())).unwrap();
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}
async fn body(req: &mut Request<Body>, max: usize) -> Result<Vec<u8>> {
    let mut data = vec![];
    while let Some(frame) = req.body_mut().frame().await {
        let frame = frame.map_err(|_| invalid("body-read-failed"))?;
        if let Some(b) = frame.data_ref() {
            if b.len() > max.saturating_sub(data.len()) {
                return Err(invalid("object-too-large"));
            }
            data.extend_from_slice(b);
        }
    }
    Ok(data)
}
async fn principal(
    center: &MessageCenter,
    req: &Request<Body>,
) -> Result<(GroupActor, RPCContext)> {
    let raw = req
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or_else(|| denied("unauthenticated"))?;
    let ctx = RPCContext {
        token: Some(raw.to_owned()),
        from_ip: Some("127.0.0.1".parse().unwrap()),
        ..Default::default()
    };
    let mut actor = center.group_actor(&ctx).await?;
    let original = req
        .headers()
        .get(CYFS_HEADER_ORIGINAL_USER)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| DID::from_str(s).ok())
        .ok_or_else(|| denied("original-user-required"))?;
    if actor.did != original {
        return Err(denied("original-user-mismatch"));
    }
    if original.method == "msgtunnel" {
        return Err(denied("shadow-endpoint-local-only"));
    }
    actor.remote = true;
    actor.client = None;
    Ok((actor, ctx))
}
pub(crate) async fn serve(
    center: &MessageCenter,
    req: &mut Request<Body>,
) -> Option<Response<Body>> {
    let path = req.uri().path().to_owned();
    let parts: Vec<_> = path.trim_start_matches('/').split('/').collect();
    let group = DID::from_str(parts.first().copied().unwrap_or("")).ok()?;
    let state = match center.groups.load(&group).await {
        Ok(Some(g)) => g,
        Ok(None) => return None,
        Err(e) => return Some(error(e)),
    };
    let config = center.cyfs_dispatch.read().unwrap().clone();
    let host = req
        .headers()
        .get("host")
        .and_then(|h| h.to_str().ok())
        .or_else(|| req.uri().host())
        .unwrap_or("");
    let expected = config.target_zone.as_deref().unwrap_or_else(|| "localhost");
    let target = match group_target(host, &path) {
        Ok(target) => target,
        Err(e) => return Some(error(e)),
    };
    if group_target(expected, &path).ok().as_ref() != Some(&target) {
        return Some(reply(StatusCode::NOT_FOUND, json!({"error":"not-found"})));
    }
    if parts.len() == 2 && matches!(parts[1], "doc" | "did.json") && req.method() == Method::GET {
        return Some(if state.lifecycle == "deleted" {
            reply(StatusCode::NOT_FOUND, json!({"error":"not-found"}))
        } else {
            match center.group_doc(&state) {
                Ok(doc) => reply(
                    StatusCode::OK,
                    if parts[1] == "did.json" {
                        doc["doc"].clone()
                    } else {
                        doc
                    },
                ),
                Err(e) => error(e),
            }
        });
    }
    let (actor, ctx) = match principal(center, req).await {
        Ok(a) => a,
        Err(e) => return Some(request_error(e, req, &target)),
    };
    let mut sid = None;
    let operation;
    if parts.get(1) == Some(&"sessions") && parts.len() >= 4 {
        let address = match MailboxAddress::try_from(format!("{}/{}", group.to_string(), parts[2]))
        {
            Ok(a) => a,
            Err(e) => return Some(error(invalid(e))),
        };
        if address.to_string() != format!("{}/{}", group.to_string(), parts[2]) {
            return Some(error(invalid("non-canonical-session-address")));
        }
        sid = address.session_id().map(str::to_owned);
        operation = parts[3..].join("/");
    } else {
        operation = parts[1..].join("/");
    }
    let params = query(req);
    let result: Result<Response<Body>> = async {
        let base = json!({"group_did":group,"session_id":sid});
        if matches!(
            operation.as_str(),
            "inbox" | "sessions" | "changes" | "objects" | "read_markers"
        ) || operation.starts_with("objects/")
        {
            // Readers are identified by the authenticated principal alone;
            // membership, guest records and the deletion tombstone decide.
            let known = state.members.contains_key(&actor.did.to_string())
                || state
                    .participants
                    .values()
                    .any(|p| p.contains_key(&actor.did.to_string()))
                || (operation == "changes"
                    && state.tombstone_readers.contains(&actor.did.to_string()));
            if !known {
                return Err(missing());
            }
        }
        match (req.method(), operation.as_str()) {
            (&Method::GET, "sessions") => Ok(reply(
                StatusCode::OK,
                center.group_sessions(&actor, &group).await?,
            )),
            (&Method::GET, "changes") => Ok(reply(
                StatusCode::OK,
                center
                    .group_changes(
                        &actor,
                        &group,
                        params.get("since").map(String::as_str),
                        params
                            .get("limit")
                            .map(|s| s.parse().map_err(|_| invalid("invalid-limit")))
                            .transpose()?
                            .unwrap_or(100),
                    )
                    .await?,
            )),
            (&Method::GET, "inbox") => {
                if params.contains_key("dispatch-status") {
                    let id = ObjId::new(&params["dispatch-status"])
                        .map_err(|_| invalid("invalid-object-id"))?;
                    let saved = state
                        .operations
                        .get(&format!("message:{}", id.to_string()))
                        .filter(|s| s.request["from"] == json!(actor.did))
                        .ok_or_else(missing)?;
                    let dispatch: buckyos_api::DispatchResult =
                        serde_json::from_value(saved.result.clone()).map_err(invalid)?;
                    let target = group_target(expected, &path)?;
                    let result = if dispatch.ok {
                        CyfsDispatchResult::new(Some(id), target, CyfsDispatchStatus::Accepted)
                    } else {
                        CyfsDispatchResult::rejected(
                            Some(id),
                            target,
                            dispatch.reason.unwrap_or_else(|| "rejected".into()),
                            false,
                        )
                    };
                    return Ok(crate::cyfs_dispatch::response(StatusCode::OK, &result));
                }
                Ok(reply(
                    StatusCode::OK,
                    center
                        .group_inbox(
                            &actor,
                            &group,
                            sid.as_deref(),
                            params
                                .get("after_seq")
                                .map(|s| s.parse().map_err(|_| invalid("invalid-after-seq")))
                                .transpose()?
                                .unwrap_or(0),
                            params
                                .get("limit")
                                .map(|s| s.parse().map_err(|_| invalid("invalid-limit")))
                                .transpose()?
                                .unwrap_or(100),
                        )
                        .await?,
                ))
            }
            (&Method::PUT, "inbox") => {
                let encoding = req
                    .headers()
                    .get("content-type")
                    .and_then(|h| h.to_str().ok())
                    .and_then(CyfsNamedObjectEncoding::from_content_type)
                    .ok_or_else(|| invalid("unsupported-content-type"))?;
                if req.headers().contains_key("content-encoding") {
                    return Err(invalid("unsupported-content-encoding"));
                }
                let claimed = req
                    .headers()
                    .get(CYFS_HEADER_OBJ_ID)
                    .and_then(|h| h.to_str().ok())
                    .map(str::to_owned);
                let bytes = body(req, config.max_object_bytes).await?;
                let id = validate_cyfs_dispatch_body(encoding, &bytes, claimed.as_deref())
                    .map_err(invalid)?;
                let (msg, jwt) = match encoding {
                    CyfsNamedObjectEncoding::Json => (
                        MsgObject::from_json_value_checked(
                            serde_json::from_slice(&bytes).map_err(invalid)?,
                        )
                        .map_err(invalid)?
                        .0,
                        None,
                    ),
                    CyfsNamedObjectEncoding::Jwt => {
                        let jwt = String::from_utf8(bytes).map_err(invalid)?;
                        let signed = crate::cyfs_dispatch::verify_signed_message(&jwt)
                            .await
                            .map_err(|(_, r)| denied(r))?;
                        (signed.msg, Some(jwt))
                    }
                };
                if msg.to_session != sid || msg.to != [group.clone()] || msg.from != actor.did {
                    return Err(denied("sender-or-session-mismatch"));
                }
                let dispatch = center.accept_group_message(&actor, msg, jwt).await?;
                let target = group_target(expected, &path)?;
                let mut result = if dispatch.ok {
                    CyfsDispatchResult::new(Some(id.clone()), target, CyfsDispatchStatus::Accepted)
                } else {
                    CyfsDispatchResult::rejected(
                        Some(id.clone()),
                        target,
                        dispatch.reason.clone().unwrap_or_else(|| "rejected".into()),
                        false,
                    )
                };
                result.source = Some(CyfsDispatchSource::Upstream);
                let mut response = crate::cyfs_dispatch::response(
                    if dispatch.ok {
                        StatusCode::OK
                    } else {
                        StatusCode::FORBIDDEN
                    },
                    &result,
                );
                if let Some(seq) = center
                    .groups
                    .load(&group)
                    .await?
                    .and_then(|g| g.messages.get(&id.to_string()).map(|m| m.session_seq))
                {
                    response
                        .headers_mut()
                        .insert("cyfs-session-seq", seq.to_string().parse().unwrap());
                }
                Ok(response)
            }
            (&Method::PUT, "join")
            | (&Method::PUT, "guest_requests")
            | (&Method::PUT, "read_markers") => {
                let bytes = body(req, config.max_object_bytes).await?;
                let mut p = base;
                if operation == "read_markers" {
                    let v: Value = serde_json::from_slice(&bytes).map_err(invalid)?;
                    p["session_id"] = if let Some(session) = v.get("session") {
                        let address = MailboxAddress::try_from(
                            session
                                .as_str()
                                .ok_or_else(|| invalid("invalid-session-address"))?
                                .to_owned(),
                        )
                        .map_err(invalid)?;
                        if address.owner() != &group {
                            return Err(missing());
                        }
                        json!(address.session_id())
                    } else {
                        v.get("session_id").cloned().unwrap_or(Value::Null)
                    };
                    p["last_read_seq"] = v["last_read_seq"].clone();
                } else {
                    let v: Value = if bytes.iter().all(u8::is_ascii_whitespace) {
                        json!({})
                    } else {
                        serde_json::from_slice(&bytes).map_err(|_| invalid("invalid-join-body"))?
                    };
                    if !v.is_object() {
                        return Err(invalid("invalid-join-body"));
                    }
                    if let Some(invite) = params.get("invite") {
                        p["invite"] = json!(invite);
                    }
                    if operation == "guest_requests" {
                        p["request_id"] = v.get("request_id").cloned().unwrap_or(Value::Null);
                    } else if sid.is_none() {
                        // An explicit invitation id accepts that invitation; a
                        // bare join accepts the caller's pending invitation when
                        // one exists and no invite link was given, otherwise it
                        // is a join request.
                        let explicit = v.get("invitation_id").and_then(Value::as_str);
                        let pending = state
                            .members
                            .get(&actor.did.to_string())
                            .filter(|m| m.state == MemberStatus::Invited)
                            .and_then(|m| m.invitation_id.as_deref());
                        if let Some(id) = explicit.or(if params.contains_key("invite") {
                            None
                        } else {
                            pending
                        }) {
                            p["invitation_id"] = json!(id);
                        }
                    }
                }
                let method = match operation.as_str() {
                    "read_markers" => "group.update_read_marker",
                    "guest_requests" => "group.submit_guest_request",
                    _ if sid.is_some() => "group.accept_session_invitation",
                    _ if p.get("invitation_id").is_some() => "group.accept_invitation",
                    _ => "group.request_join",
                };
                Ok(reply(
                    StatusCode::OK,
                    center
                        .group_rpc_authenticated(method, p, ctx, actor.clone())
                        .await?,
                ))
            }
            (&Method::GET, op) if op.starts_with("objects/") => {
                let id = ObjId::new(op.trim_start_matches("objects/")).map_err(invalid)?;
                if id.obj_type != OBJ_TYPE_MSG {
                    let context = params.get("context_path").ok_or_else(missing)?;
                    let (mailbox, _) = context.rsplit_once('/').ok_or_else(missing)?;
                    let address = MailboxAddress::try_from(mailbox.to_string()).map_err(invalid)?;
                    if address.owner() != &group {
                        return Err(missing());
                    }
                    center
                        .authorize_group_attachment(&actor, &id, context)
                        .await?;
                    return Ok(crate::object_access::fetch_authorized_object(&id, false).await);
                }
                center.authorize_group_message(&actor, &id).await?;
                let (object_group, body, jwt) =
                    center.groups.object(&id).await?.ok_or_else(missing)?;
                if object_group != group {
                    return Err(missing());
                }
                let (mime, body) = match jwt {
                    Some(jwt) => (CYFS_CONTENT_TYPE_NAMED_OBJECT_JWT, jwt),
                    None => (
                        CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON,
                        body.ok_or_else(missing)?,
                    ),
                };
                Ok(Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", mime)
                    .header("cache-control", "no-store")
                    .body(Full::new(Bytes::from(body)).map_err(|e| match e {}).boxed())
                    .unwrap())
            }
            _ => Err(invalid("unsupported-group-operation")),
        }
    }
    .await;
    Some(match result {
        Ok(r) => r,
        Err(e) => request_error(e, req, &target),
    })
}
