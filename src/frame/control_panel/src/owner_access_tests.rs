use super::*;
use crate::agent_mgr::tests::MemoryKv;
use buckyos_api::UserType;

fn principal(user: &str) -> RpcAuthPrincipal {
    RpcAuthPrincipal {
        username: user.into(),
        owner_user_id: user.into(),
        owner_did: format!("did:bns:{user}"),
        is_user_session: true,
        is_control_panel_session: true,
        authenticated_app_id: "system:control-panel".into(),
        principal_kind: TokenPrincipalKind::User,
        user_type: UserType::User,
        agent_permission_mode: None,
        session: Default::default(),
    }
}

#[tokio::test]
async fn availability_discovers_only_active_local_grants_and_revokes_immediately() {
    let kv = MemoryKv::default();
    let alice = principal("alice");
    let bob = principal("bob");
    let carol = principal("carol");
    for p in [&alice, &bob, &carol] {
        let user = &p.owner_user_id;
        let settings = json!({"user_id":user,"type":"user","state":"active","password":"","res_pool_id":"default"});
        assert!(
            save(&kv, &format!("users/{user}/settings"), None, &settings)
                .await
                .unwrap()
        );
        assert!(save(
            &kv,
            &format!("users/{user}/doc"),
            None,
            &json!({"id":p.owner_did})
        )
        .await
        .unwrap());
    }
    let query = json!({"did":alice.owner_did});
    let grant = json!({"slot":"availability","to":bob.owner_did});
    assert_eq!(
        sharing(&kv, &bob, "owner.sharing.discover", &query)
            .await
            .unwrap(),
        json!({})
    );
    assert!(sharing(&kv, &bob, "owner.sharing.check", &query)
        .await
        .is_err());
    let foreign = json!({"slot":"availability","to":"did:bns:outside"});
    assert!(sharing(&kv, &alice, "owner.sharing.grant", &foreign)
        .await
        .is_err());
    let mut agent = alice.clone();
    agent.principal_kind = TokenPrincipalKind::Agent;
    agent.is_user_session = false;
    assert!(sharing(&kv, &agent, "owner.sharing.grant", &grant)
        .await
        .is_err());
    assert_eq!(
        sharing(&kv, &alice, "owner.sharing.grant", &grant)
            .await
            .unwrap()["granted"],
        true
    );
    let visible = sharing(&kv, &bob, "owner.sharing.discover", &query)
        .await
        .unwrap();
    assert_eq!(
        visible,
        json!({"availability_ref":obj_service_url("aiworkspace", &["people", &alice.owner_did,"availability"])})
    );
    assert_eq!(
        sharing(&kv, &bob, "owner.sharing.check", &query)
            .await
            .unwrap()["owner_user_id"],
        "alice"
    );
    assert_eq!(
        sharing(&kv, &carol, "owner.sharing.discover", &query)
            .await
            .unwrap(),
        json!({})
    );
    assert_eq!(
        sharing(&kv, &alice, "owner.sharing.revoke", &grant)
            .await
            .unwrap()["granted"],
        false
    );
    assert!(sharing(&kv, &bob, "owner.sharing.check", &query)
        .await
        .is_err());
    assert_eq!(
        sharing(&kv, &bob, "owner.sharing.discover", &query)
            .await
            .unwrap(),
        json!({})
    );
}

#[tokio::test]
async fn identity_rejects_external_limited_device_and_unbound_app_principals() {
    let kv = MemoryKv::default();
    let human = principal("alice");
    assert_eq!(identity(&kv, &human).await.unwrap(), "did:bns:alice");
    let mut limited = human.clone();
    limited.user_type = UserType::Limited;
    assert!(identity(&kv, &limited).await.is_err());
    let mut external = human.clone();
    external.session.is_external = true;
    assert!(identity(&kv, &external).await.is_err());
    for kind in [
        TokenPrincipalKind::App,
        TokenPrincipalKind::Device,
        TokenPrincipalKind::System,
    ] {
        let mut unbound = human.clone();
        unbound.principal_kind = kind;
        unbound.authenticated_app_id = "app:weather@alice".into();
        assert!(identity(&kv, &unbound).await.is_err());
    }
}

#[tokio::test]
async fn concurrent_delegations_cannot_form_a_cycle_or_leave_history() {
    let kv = MemoryKv::default();
    let ak = binding_key("alice", "calendar");
    let bk = binding_key("bob", "calendar");
    let delegation = |to: &str| json!({"slot":"calendar","mode":"object","upstream":{"owner_did":to,"slot":"calendar"}});
    let to_a = delegation("did:bns:alice");
    let to_b = delegation("did:bns:bob");
    assert!(
        commit_binding(&kv, &ak, None, &to_a, "did:bns:alice", HashMap::new())
            .await
            .unwrap_err()
            .to_string()
            .contains("RESOURCE_BINDING_CYCLE")
    );
    let hist = |id: &str| HashMap::from([(format!("history/{id}"), KVAction::Create("{}".into()))]);
    let (a, b) = tokio::join!(
        commit_binding(&kv, &ak, None, &to_b, "did:bns:alice", hist("a")),
        commit_binding(&kv, &bk, None, &to_a, "did:bns:bob", hist("b")),
    );
    let mut results = [a, b];
    results.sort_by_key(|result| result.is_err());
    assert!(matches!(&results[0], Ok(true)));
    assert!(results[1]
        .as_ref()
        .unwrap_err()
        .to_string()
        .contains("RESOURCE_BINDING_CYCLE"));
    assert_eq!(kv.list("history").await.unwrap().len(), 1);
    let (existing, from, other, other_did, target) = if kv.get(&ak).await.unwrap().is_some() {
        (&ak, "did:bns:alice", &bk, "did:bns:bob", &to_a)
    } else {
        (&bk, "did:bns:bob", &ak, "did:bns:alice", &to_b)
    };
    let version = kv.get(existing).await.unwrap().unwrap().1;
    assert!(commit_binding(
        &kv,
        existing,
        Some(version),
        &default_binding("calendar"),
        from,
        HashMap::new()
    )
    .await
    .unwrap());
    assert!(
        commit_binding(&kv, other, None, target, other_did, HashMap::new())
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn resolve_exposes_only_the_personal_slot_contract() {
    let kv = MemoryKv::default();
    let target = json!({
        "slot":"calendar", "mode":"object", "binding_revision":3, "actions":["list"],
        "provider_ref":"https://example.test/private",
        "upstream":{"owner_did":"did:bns:bob","slot":"calendar"},
        "record_binding":"private-history-key"
    });
    assert!(save(&kv, &binding_key("alice", "calendar"), None, &target)
        .await
        .unwrap());
    let visible = binding(
        &kv,
        "alice",
        "zone/alice",
        "did:bns:alice",
        "owner.resolve",
        &json!({"slot":"calendar"}),
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        visible,
        json!({"slot":"calendar","mode":"object","binding_revision":3,"actions":["list"]})
    );
}
