use crate::group_types::{denied, invalid, GroupActor, Result};
use crate::msg_center::MessageCenter;
use http::StatusCode;
use name_lib::DID;
use ndn_lib::{MachineContent, MsgContent, MsgObjKind, MsgObject};
use serde_json::json;
use sha2::{Digest, Sha256};

pub(crate) const CYFS_HEADER_GROUP_REQUEST: &str = "cyfs-group-request";
const GROUP_REQUEST_INTENT: &str = "buckyos.group_request";
const REQUEST_MAX_AGE_MS: u64 = 60_000;
const REQUEST_CLOCK_SKEW_MS: u64 = 30_000;
const MAX_PROOF_BYTES: usize = 16_384;

fn request_binding(
    host: &str,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Result<serde_json::Value> {
    let audience = ndn_lib::normalize_cyfs_dispatch_target(host, "/inbox").map_err(invalid)?;
    let uri: http::uri::PathAndQuery = path_and_query.parse().map_err(invalid)?;
    if !uri.path().starts_with('/') || !matches!(method, "GET" | "PUT") {
        return Err(invalid("invalid-group-request"));
    }
    Ok(json!({
        "audience": audience,
        "method": method,
        "path_and_query": path_and_query,
        "body_sha256": hex::encode(Sha256::digest(body)),
    }))
}

pub(crate) fn signed_group_request(
    center: &MessageCenter,
    owner: &DID,
    group: &DID,
    host: &str,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Result<String> {
    if owner.method == "msgtunnel" || !center.is_hosted_identity(owner) {
        return Err(denied("remote-group-auth-failed"));
    }
    let signer = center
        .zone_signer
        .get()
        .ok_or_else(|| denied("remote-group-auth-failed"))?;
    let msg = MsgObject {
        from: owner.clone(),
        to: vec![group.clone()],
        kind: MsgObjKind::Operation,
        created_at_ms: MessageCenter::now_ms(),
        nonce: Some(uuid::Uuid::new_v4().as_u128() as u64),
        content: MsgContent {
            machine: Some(MachineContent {
                intent: Some(GROUP_REQUEST_INTENT.into()),
                data: serde_json::from_value(request_binding(host, method, path_and_query, body)?)
                    .map_err(invalid)?,
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    msg.to_jwt(&signer.key, &signer.kid)
        .map_err(|_| denied("remote-group-auth-failed"))
}

fn check_request(
    msg: &MsgObject,
    original_user: &DID,
    group: &DID,
    expected: &serde_json::Value,
    now: u64,
) -> Result<()> {
    let machine = msg.content.machine.as_ref();
    if &msg.from != original_user
        || msg.from.method == "msgtunnel"
        || msg.to != [group.clone()]
        || msg.kind != MsgObjKind::Operation
        || msg.to_session.is_some()
        || msg.created_at_ms > now.saturating_add(REQUEST_CLOCK_SKEW_MS)
        || now.saturating_sub(msg.created_at_ms) > REQUEST_MAX_AGE_MS
        || machine.and_then(|machine| machine.intent.as_deref()) != Some(GROUP_REQUEST_INTENT)
        || machine.map(|machine| json!(machine.data)).as_ref() != Some(expected)
    {
        return Err(denied("remote-group-auth-failed"));
    }
    Ok(())
}

pub(crate) async fn verify_group_request(
    center: &MessageCenter,
    proof: &str,
    original_user: &DID,
    group: &DID,
    host: &str,
    method: &str,
    path_and_query: &str,
    body: &[u8],
) -> Result<GroupActor> {
    if proof.len() > MAX_PROOF_BYTES {
        return Err(denied("remote-group-auth-failed"));
    }
    let expected = request_binding(host, method, path_and_query, body)
        .map_err(|_| denied("remote-group-auth-failed"))?;
    let decoded =
        ndn_lib::decode_msg_object_jwt(proof).map_err(|_| denied("remote-group-auth-failed"))?;
    check_request(
        &decoded.msg,
        original_user,
        group,
        &expected,
        MessageCenter::now_ms(),
    )?;
    let verified = crate::cyfs_dispatch::verify_signed_message_with(proof, Some(center))
        .await
        .map_err(|(status, _)| {
            if status == StatusCode::SERVICE_UNAVAILABLE {
                kRPC::RPCErrors::ReasonError("remote-group-unavailable".into())
            } else {
                denied("remote-group-auth-failed")
            }
        })?;
    check_request(
        &verified.msg,
        original_user,
        group,
        &expected,
        MessageCenter::now_ms(),
    )?;
    Ok(GroupActor {
        did: verified.msg.from,
        client: None,
        remote: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg_box_db::MsgBoxDbMgr;
    use crate::network_directory::{test_support::StaticDirectory, ZoneSigner};
    use buckyos_api::LookupKind;
    use jsonwebtoken::EncodingKey;

    const KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
    const OTHER_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----";
    const DEVICE: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";

    async fn center() -> (MessageCenter, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db = MsgBoxDbMgr::open_default_sqlite(&format!(
            "sqlite://{}?mode=rwc",
            tmp.path().join("group-auth.db").display()
        ))
        .await
        .unwrap();
        (MessageCenter::open_with_db(db).await.unwrap(), tmp)
    }

    fn proof_with(msg: &MsgObject, key: &str) -> String {
        msg.to_jwt(
            &EncodingKey::from_ed_pem(key.as_bytes()).unwrap(),
            &format!("did:dev:{DEVICE}#main_key"),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn signed_group_requests_bind_identity_route_payload_and_lifetime() {
        let (source, _s) = center().await;
        let (host, _h) = center().await;
        let owner = DID::new("web", "alice.source.example");
        let group = DID::new("web", "chat.host.example");
        source.register_local_recipients([owner.clone()]);
        source.zone_signer.set(ZoneSigner {
            key: EncodingKey::from_ed_pem(KEY.as_bytes()).unwrap(),
            kid: format!("did:dev:{DEVICE}#main_key"),
        });
        host.directory.set(StaticDirectory::with(vec![
            (owner.clone(), LookupKind::User, "Alice", "source.example"),
            (
                DID::new("dev", DEVICE),
                LookupKind::Device,
                "ood",
                "source.example",
            ),
        ]));
        let path = format!("/kapi/msg-center/groups/{}/join", group.to_string());
        let body = br#"{"invitation_id":"invite-1"}"#;
        let proof =
            signed_group_request(&source, &owner, &group, "host.example", "PUT", &path, body)
                .unwrap();
        let actor = verify_group_request(
            &host,
            &proof,
            &owner,
            &group,
            "host.example",
            "PUT",
            &path,
            body,
        )
        .await
        .unwrap();
        assert_eq!(actor.did, owner);
        assert!(actor.remote);
        assert!(actor.client.is_none());
        for (who, target, hostname, method, requested_path, payload) in [
            (
                DID::new("web", "bob.source.example"),
                group.clone(),
                "host.example",
                "PUT",
                path.clone(),
                body.as_slice(),
            ),
            (
                owner.clone(),
                DID::new("web", "other.host.example"),
                "host.example",
                "PUT",
                path.clone(),
                body.as_slice(),
            ),
            (
                owner.clone(),
                group.clone(),
                "evil.example",
                "PUT",
                path.clone(),
                body.as_slice(),
            ),
            (
                owner.clone(),
                group.clone(),
                "host.example",
                "GET",
                path.clone(),
                body.as_slice(),
            ),
            (
                owner.clone(),
                group.clone(),
                "host.example",
                "PUT",
                format!("{path}?invite=other"),
                body.as_slice(),
            ),
            (
                owner.clone(),
                group.clone(),
                "host.example",
                "PUT",
                path.clone(),
                b"{}".as_slice(),
            ),
        ] {
            assert!(verify_group_request(
                &host,
                &proof,
                &who,
                &target,
                hostname,
                method,
                &requested_path,
                payload
            )
            .await
            .is_err());
        }
        let mut msg = ndn_lib::decode_msg_object_jwt(&proof).unwrap().msg;
        for created_at in [
            MessageCenter::now_ms().saturating_sub(REQUEST_MAX_AGE_MS + 1_000),
            MessageCenter::now_ms() + REQUEST_CLOCK_SKEW_MS + 1_000,
        ] {
            msg.created_at_ms = created_at;
            let stale = proof_with(&msg, KEY);
            assert!(verify_group_request(
                &host,
                &stale,
                &owner,
                &group,
                "host.example",
                "PUT",
                &path,
                body
            )
            .await
            .is_err());
        }
        msg.created_at_ms = MessageCenter::now_ms();
        let forged = proof_with(&msg, OTHER_KEY);
        assert!(verify_group_request(
            &host,
            &forged,
            &owner,
            &group,
            "host.example",
            "PUT",
            &path,
            body
        )
        .await
        .is_err());
        let other = DID::new("dev", "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0");
        host.directory.set(StaticDirectory::with(vec![
            (other.clone(), LookupKind::User, "Other", "another.example"),
            (
                DID::new("dev", DEVICE),
                LookupKind::Device,
                "ood",
                "source.example",
            ),
        ]));
        msg.from = other.clone();
        let wrong_zone = proof_with(&msg, KEY);
        assert!(verify_group_request(
            &host,
            &wrong_zone,
            &other,
            &group,
            "host.example",
            "PUT",
            &path,
            body,
        )
        .await
        .is_err());
        let unknown = signed_group_request(
            &source,
            &DID::new("web", "foreign.example"),
            &group,
            "host.example",
            "PUT",
            &path,
            body,
        );
        assert!(unknown.is_err());
    }

    #[tokio::test]
    async fn signed_prefixed_join_restores_body_and_preserves_group_permissions() {
        use crate::test_group_service::http_body;
        use http::Request;
        use http_body_util::BodyExt;
        use kRPC::RPCContext;

        let (source, _s) = center().await;
        let (host, _h) = center().await;
        let member = DID::new("dev", DEVICE);
        let owner = DID::new("dev", "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0");
        let group = DID::new("web", "chat.host.example");
        host.cyfs_dispatch.write().unwrap().target_zone = Some("host.example".into());
        host.set_message_hub_did(DID::new("web", "hub.host.example"));
        source.register_local_recipients([member.clone()]);
        source.zone_signer.set(ZoneSigner {
            key: EncodingKey::from_ed_pem(KEY.as_bytes()).unwrap(),
            kid: format!("did:dev:{DEVICE}#main_key"),
        });
        let actor = GroupActor {
            did: owner,
            client: None,
            remote: false,
        };
        host.group_rpc_authenticated(
            "group.create",
            json!({"group_did":group,"idempotency_key":"auth-test"}),
            RPCContext::default(),
            actor.clone(),
        )
        .await
        .unwrap();
        let invite = host
            .group_rpc_authenticated(
                "group.invite_member",
                json!({"group_did":group,"member_did":member}),
                RPCContext::default(),
                actor,
            )
            .await
            .unwrap();
        let request = |operation: &str, payload: Vec<u8>| {
            let path = format!("/kapi/msg-center/groups/{}/{operation}", group.to_string());
            let method = if operation == "join" { "PUT" } else { "GET" };
            let proof = signed_group_request(
                &source,
                &member,
                &group,
                "host.example",
                method,
                &path,
                &payload,
            )
            .unwrap();
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "host.example")
                .header(ndn_lib::CYFS_HEADER_ORIGINAL_USER, member.to_string())
                .header(CYFS_HEADER_GROUP_REQUEST, proof)
                .body(http_body(payload))
                .unwrap()
        };
        let mut before = request("sessions", vec![]);
        assert_eq!(
            crate::group_http::serve(&host, &mut before)
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
        let mut join = request(
            "join",
            serde_json::to_vec(&json!({"invitation_id":invite["invite_id"]})).unwrap(),
        );
        let response = crate::group_http::serve(&host, &mut join).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(result["state"], "active");
        let mut after = request("sessions", vec![]);
        assert_eq!(
            crate::group_http::serve(&host, &mut after)
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
        let mut no_proof = request("sessions", vec![]);
        no_proof.headers_mut().remove(CYFS_HEADER_GROUP_REQUEST);
        assert_eq!(
            crate::group_http::serve(&host, &mut no_proof)
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let mut not_found = Request::builder()
            .uri("/kapi/msg-center/groups/did:web:missing.host.example/sessions")
            .body(http_body(""))
            .unwrap();
        let response = crate::group_http::serve(&host, &mut not_found)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let error: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(error["error"], "remote-group-not-found");
        let mut other_handler = Request::builder()
            .uri("/did:web:missing.host.example/sessions")
            .body(http_body(""))
            .unwrap();
        assert!(crate::group_http::serve(&host, &mut other_handler)
            .await
            .is_none());
        let mut personal_inbox = Request::builder()
            .method("PUT")
            .uri("/kapi/msg-center/did:web:alice.host.example/inbox")
            .body(http_body("{}"))
            .unwrap();
        assert!(crate::group_http::serve(&host, &mut personal_inbox)
            .await
            .is_none());
    }
}
