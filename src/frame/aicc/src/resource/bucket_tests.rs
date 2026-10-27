//! Artifacts against a real bucket service: an nfs-server of its own with the
//! verify-hub test key and the default RBAC model, declaring AICC's buckets and
//! an Agent app's `artifacts`.

use super::*;
use buckyos_api::bucket_client::CopyRequest;
use buckyos_api::{
    bind_token_principal_kind, bind_token_target, overlap_rbac_policy, AuthTarget,
    TokenPrincipalKind, TokenUse, DEFAULT_RBAC_MODEL, DEFAULT_RBAC_POLICY, VERIFY_HUB_UNIQUE_ID,
};
use jsonwebtoken::EncodingKey;
use kRPC::{RPCSessionToken, RPCSessionTokenType};

const PRIVATE: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
const PUBLIC: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
const AGENT: &str = "agentx";

fn token(sub: &str, kind: TokenPrincipalKind, target: &str) -> String {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(VERIFY_HUB_UNIQUE_ID.into()),
        jti: None,
        sub: Some(sub.into()),
        appid: None,
        sudo: false,
        extra: Default::default(),
    };
    bind_token_principal_kind(&mut token, kind);
    bind_token_target(
        &mut token,
        &AuthTarget::from_canonical_key(target).unwrap(),
        TokenUse::Session,
    )
    .unwrap();
    token
        .generate_jwt(None, &EncodingKey::from_ed_pem(PRIVATE.as_bytes()).unwrap())
        .unwrap()
}

struct Zone {
    url: String,
    store: NamedDataMgr,
    _tmp: tempfile::TempDir,
}

impl Zone {
    async fn start() -> Self {
        static RBAC: tokio::sync::OnceCell<()> = tokio::sync::OnceCell::const_new();
        RBAC.get_or_init(|| async {
            let groups =
                format!("g, ood1, ood\ng, alice, users\ng, app:{AGENT}, app\ng, app:other, app\n");
            let policy = overlap_rbac_policy(DEFAULT_RBAC_POLICY, &groups);
            rbac::create_enforcer(DEFAULT_RBAC_MODEL.trim(), policy.trim())
                .await
                .unwrap();
        })
        .await;
        let tmp = tempfile::tempdir().unwrap();
        let mut config = nfs_server::ServerConfig::new(tmp.path().join("nfs"), vec![]);
        config.auth = nfs_server::config::AuthMode::TestKey {
            public_key: PUBLIC.into(),
            zone_owner: "owner".into(),
        };
        config.named_store = nfs_server::config::NamedStoreSource::Dir(tmp.path().join("store"));
        config.bucket_specs = serde_json::from_value(json!({
            "aicc": {
                "buckets": buckyos_api::aicc_bucket_decls(),
                "storage_quota_mib": buckyos_api::AICC_STORAGE_QUOTA_MIB,
            },
            AGENT: {"buckets": {"artifacts": {"kind": "data", "data_policy": "documents"}}},
        }))
        .unwrap();
        let state = nfs_server::AppState::new(config).unwrap();
        let store = state.share.store(&state.config).await.unwrap().clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/kapi/nfs-server", listener.local_addr().unwrap());
        let app = nfs_server::build_router(state);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            store,
            _tmp: tmp,
        }
    }

    fn client(&self, token: String) -> BucketClient {
        BucketClient::new(kRPC::kRPC::new(&self.url, Some(token)))
    }
}

struct AllowAll;

#[async_trait]
impl ResourceAuthorizer for AllowAll {
    async fn authorize(
        &self,
        _: &ResourceAccessContext,
        _: &ResourceTarget,
        _: ResourceAccessOperation,
    ) -> Result<(), ResourceError> {
        Ok(())
    }
}

struct NoFetch;

#[async_trait]
impl UrlResourceFetcher for NoFetch {
    async fn fetch(&self, _: &reqwest::Url, _: u64) -> Result<FetchedResource, ResourceError> {
        Err(ResourceError::new(ResourceFailure::Unavailable, "no fetch"))
    }
}

fn png(seed: u8) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(4096, seed);
    bytes
}

fn spec(name: &str) -> ArtifactSpec {
    ArtifactSpec {
        name: name.to_string(),
        mime: "image/png".to_string(),
        attributes: Map::new(),
        embedding: None,
    }
}

#[tokio::test]
async fn outputs_are_retained_for_the_user_and_readable_by_the_calling_app_only() {
    let zone = Zone::start().await;
    let aicc = zone.client(token("ood1", TokenPrincipalKind::System, "system:aicc"));
    let resources = NamedDataMgrResourceStore::new(zone.store.clone()).with_bucket_client(aicc);
    let manager = ResourceManager::new(
        Arc::new(AllowAll),
        Arc::new(resources),
        Arc::new(NoFetch),
        ResourceLimits::default(),
    )
    .unwrap();
    let context = ResourceAccessContext::new("alice", "alice", "request-1")
        .unwrap()
        .with_app(Some(AGENT.to_string()));
    let artifact = manager
        .write_artifact(&context, &png(1), spec("image_1.png"))
        .await
        .unwrap();
    let ResourceRef::NamedObject { obj_id } = &artifact.resource else {
        panic!("artifact must be a named object");
    };
    let bucket_ref = artifact.metadata.as_ref().unwrap()["bucket_ref"]
        .as_str()
        .unwrap()
        .to_string();
    let (outputs, key) = buckyos_api::parse_bucket_obj_url(&bucket_ref).unwrap();
    assert_eq!(outputs.to_string(), "user/aicc/alice/outputs");
    assert!(key.starts_with(&format!("{AGENT}/")) && key.ends_with("/image_1.png"));

    let aicc = zone.client(token("ood1", TokenPrincipalKind::System, "system:aicc"));
    let binding = aicc.get(&outputs, &key, None).await.unwrap();
    assert_eq!(binding.obj_id.as_deref(), Some(obj_id.to_string().as_str()));
    assert!(binding.expires_at.is_some(), "a temp binding expires");

    // The Agent's runtime session copies its result into its own documents
    // bucket: a second commitment, no new bytes.
    let agent = zone.client(token(
        "alice",
        TokenPrincipalKind::App,
        &format!("app:{AGENT}@alice"),
    ));
    let artifacts = BucketId::user(AGENT, "alice", "artifacts").unwrap();
    agent
        .copy(&CopyRequest {
            src_bucket_id: outputs.to_string(),
            src_key: key.clone(),
            dst_bucket_id: artifacts.to_string(),
            dst_key: "0123/image_1.png".to_string(),
            ..Default::default()
        })
        .await
        .unwrap();
    let copied = agent
        .get(&artifacts, "0123/image_1.png", None)
        .await
        .unwrap();
    assert_eq!(copied.obj_id, binding.obj_id);

    // Another App of the same user does not see it.
    let other = zone.client(token("alice", TokenPrincipalKind::App, "app:other@alice"));
    assert!(other.get(&outputs, &key, None).await.is_err());

    // A second result of the same App reuses the grant.
    let second = manager
        .write_artifact(&context, &png(2), spec("image_2.png"))
        .await
        .unwrap();
    assert!(second.metadata.unwrap()["bucket_ref"]
        .as_str()
        .unwrap()
        .ends_with("/image_2.png"));
    let grants = aicc
        .call("bucket.list_grants", json!({"bucket_id": outputs}))
        .await
        .unwrap();
    assert_eq!(grants.as_array().unwrap().len(), 1, "{grants}");

    // Derived inputs land in the one-day bucket.
    let input = manager
        .write_input(&context, &png(3), spec("video-condition-first.png"))
        .await
        .unwrap();
    assert!(input.metadata.unwrap()["bucket_ref"]
        .as_str()
        .unwrap()
        .starts_with("obj://bucket/user/aicc/alice/inputs/"));
}

#[tokio::test]
async fn a_failed_binding_fails_the_write() {
    let zone = Zone::start().await;
    let stranger = zone.client(token("alice", TokenPrincipalKind::App, "app:other@alice"));
    let resources = NamedDataMgrResourceStore::new(zone.store.clone()).with_bucket_client(stranger);
    let context = ResourceAccessContext::new("alice", "alice", "request-2").unwrap();
    let target = ArtifactTarget::new(&context, AICC_BUCKET_OUTPUTS).unwrap();
    let error = resources
        .write_artifact(&png(4), &spec("lost.png"), &target)
        .await
        .err()
        .expect("an unbound artifact is not written");
    assert_eq!(error.failure, ResourceFailure::Unavailable);
}
