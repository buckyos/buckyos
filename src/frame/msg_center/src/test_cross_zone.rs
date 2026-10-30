//! Two test Zones talking the cross-Zone protocol (跨Zone登录断言契约): device and owner
//! documents for the custody rule, a fake verify-hub that signs Externals in by assertion,
//! and a token verifier per Zone.

use crate::msg_center::MessageCenter;
use crate::owner_session::{SessionTokenVerifier, ZoneAgentInfo};
use crate::test_group_service::{http_body, TestBody};
use buckyos_api::{
    bind_token_auth_method, bind_token_ceiling, bind_token_external, bind_token_target,
    cross_zone_target_scopes, verify_signed_assertion, AuthMethod, AuthTarget, CrossZoneClient,
    CustodyResolver, DeviceSigner, SessionCeiling, StaticCustodyResolver, SystemServiceId, TokenUse,
    CROSS_ZONE_LOGIN_METHOD,
};
use http::{Request, Response};
use http_body_util::BodyExt;
use jsonwebtoken::{DecodingKey, EncodingKey};
use kRPC::{RPCErrors, RPCSessionToken, RPCSessionTokenType};
use name_lib::{DidDocType, DID};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

pub(crate) struct TestZone {
    pub host: &'static str,
    pub key: &'static str,
    pub x: &'static str,
}

pub(crate) const ZONE_A: TestZone = TestZone {
    host: "a.example",
    key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----",
    x: "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8",
};

pub(crate) const ZONE_B: TestZone = TestZone {
    host: "b.example",
    key: "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----",
    x: "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0",
};

impl TestZone {
    pub fn did(&self) -> DID {
        DID::new("web", self.host)
    }

    /// The Zone's OOD: its key signs messages and login assertions.
    pub fn device(&self) -> DID {
        DID::new("web", &format!("ood1.{}", self.host))
    }

    pub fn signer(&self) -> DeviceSigner {
        DeviceSigner::new(self.device(), EncodingKey::from_ed_pem(self.key.as_bytes()).unwrap())
    }
}

/// Documents of the Zones and of the users each hosts.
pub(crate) fn documents(zones: &[(&TestZone, Vec<DID>)]) -> StaticCustodyResolver {
    let mut resolver = StaticCustodyResolver::default();
    for (zone, users) in zones {
        let device = zone.device().to_string();
        resolver = resolver
            .with_doc(&zone.device(), Some(DidDocType::Device), json!({
                "id": device,
                "verificationMethod": [{"type": "Ed25519VerificationKey2020", "id": "#main_key", "controller": device,
                    "publicKeyJwk": {"kty": "OKP", "crv": "Ed25519", "x": zone.x}}],
                "authentication": ["#main_key"],
                "exp": 4102444800u64, "iat": 1,
                "zone_did": zone.did(), "owner": zone.did(), "device_type": "ood", "name": "ood1"
            }))
            .with_doc(&zone.did(), Some(DidDocType::Zone), json!({
                "id": zone.did(), "hostname": zone.host, "owner": zone.did(), "oods": ["ood1"],
                "boot_jwt": "", "exp": 4102444800u64, "iat": 1,
                "verificationMethod": [], "authentication": []
            }));
        for user in users {
            resolver = resolver.with_doc(user, None, json!({
                "@context": "https://www.buckyos.org/ns/owner", "id": user,
                "binded_zone_list": [zone.did()], "authentication": []
            }));
        }
    }
    resolver
}

/// Sessions of one Zone: any token its key signed.
pub(crate) struct ZoneVerifier(pub &'static str);

#[async_trait::async_trait]
impl SessionTokenVerifier for ZoneVerifier {
    async fn authorize(&self, _: &str, _: &str, _: &str) -> Result<(), RPCErrors> {
        Ok(())
    }
    async fn verify(&self, raw: &str) -> Result<RPCSessionToken, RPCErrors> {
        let mut token = RPCSessionToken::from_string(raw)?;
        token.verify_by_key(&DecodingKey::from_ed_components(self.0).unwrap())?;
        Ok(token)
    }
    async fn resolve_user_did(&self, id: &str) -> Result<DID, RPCErrors> {
        DID::from_str(id).map_err(|error| RPCErrors::ParseRequestError(error.to_string()))
    }
    async fn zone_agent(&self, _: &DID) -> Result<Option<ZoneAgentInfo>, RPCErrors> {
        Ok(None)
    }
    async fn sso_settings(&self) -> Result<buckyos_api::SsoSettings, RPCErrors> {
        Ok(Default::default())
    }
}

/// `center` speaks as `zone`: its device signs, documents come from `resolver`.
pub(crate) fn sign_as(center: &MessageCenter, zone: &TestZone, resolver: Arc<dyn CustodyResolver>) {
    center.zone_signer.set(zone.signer());
    center.custody.set(resolver);
    center.set_token_verifier(Arc::new(ZoneVerifier(zone.x)));
}

/// `center` reaches other Zones at `origins` (host → test listener).
pub(crate) fn connect(
    center: &MessageCenter,
    zone: &TestZone,
    resolver: &Arc<StaticCustodyResolver>,
    origins: HashMap<String, String>,
) {
    let client = CrossZoneClient::new(zone.signer(), None)
        .unwrap()
        .with_resolver(resolver.clone())
        .with_origins(origins);
    center.cross_zone.set(Arc::new(client));
}

pub(crate) fn mint(zone: &TestZone, sub: &str, ceiling: &SessionCeiling, token_use: TokenUse) -> String {
    let mut token = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(buckyos_kit::buckyos_get_unix_timestamp() + 3600),
        iss: Some(buckyos_api::VERIFY_HUB_TOKEN_ISSUER.into()),
        jti: Some(uuid::Uuid::new_v4().to_string()),
        sub: Some(sub.to_string()),
        appid: None,
        sudo: false,
        extra: HashMap::new(),
    };
    bind_token_external(&mut token);
    bind_token_ceiling(&mut token, ceiling);
    bind_token_auth_method(&mut token, AuthMethod::CrossZoneAssertion);
    bind_token_target(
        &mut token,
        &AuthTarget::system(SystemServiceId::parse("msg-center").unwrap()),
        token_use,
    )
    .unwrap();
    token
        .generate_jwt(None, &EncodingKey::from_ed_pem(zone.key.as_bytes()).unwrap())
        .unwrap()
}

/// `zone`'s verify-hub for `/kapi/verify-hub`: assertion logins and refreshes; any other
/// request comes back for the caller to serve.
pub(crate) async fn verify_hub(
    zone: &TestZone,
    resolver: &dyn CustodyResolver,
    request: Request<TestBody>,
) -> Result<Response<TestBody>, Request<TestBody>> {
    if request.uri().path() != "/kapi/verify-hub" {
        return Err(request);
    }
    let body = request.into_body().collect().await.unwrap().to_bytes();
    let call: Value = serde_json::from_slice(&body).unwrap();
    let pair = |sub: &str, ceiling: &SessionCeiling| {
        json!({"result": {
            "session_token": mint(zone, sub, ceiling, TokenUse::Session),
            "refresh_token": mint(zone, sub, ceiling, TokenUse::Refresh),
        }})
    };
    let mut answer = match call["method"].as_str() {
        Some(CROSS_ZONE_LOGIN_METHOD) => {
            let assertion = call["params"]["assertion"].as_str().unwrap_or_default();
            let now = buckyos_kit::buckyos_get_unix_timestamp();
            match verify_signed_assertion(resolver, assertion, &zone.did(), zone.host, now).await {
                Ok(verified) => {
                    let scopes = cross_zone_target_scopes(&verified.target).unwrap();
                    pair(&verified.requester.to_string(), &SessionCeiling::restricted(scopes.iter().copied()))
                }
                Err(failure) => json!({"error": failure.detail, "code": failure.code.as_str()}),
            }
        }
        Some("refresh_token") => {
            let refresh = call["params"]["refresh_token"].as_str().unwrap_or_default();
            let token = RPCSessionToken::from_string(refresh).unwrap();
            let ceiling = buckyos_api::token_session_ceiling(&token).unwrap();
            pair(token.sub.as_deref().unwrap(), &ceiling)
        }
        _ => json!({"error": "unknown method"}),
    };
    answer["sys"] = json!([call["sys"][0]]);
    Ok(Response::builder()
        .header("content-type", "application/json")
        .body(http_body(answer.to_string()))
        .unwrap())
}

/// A test listener serving `center` as `zone`: its verify-hub, then `serve`.
pub(crate) async fn zone_server<F, Fut>(
    center: MessageCenter,
    zone: &'static TestZone,
    serve: F,
) -> (String, tokio::task::JoinHandle<()>)
where
    F: Fn(MessageCenter, Request<TestBody>) -> Fut + Send + Sync + Clone + 'static,
    Fut: std::future::Future<Output = Response<TestBody>> + Send,
{
    crate::test_group_service::http_fixture(move |request| {
        let center = center.clone();
        let serve = serve.clone();
        async move {
            let resolver = center.custody.get();
            match verify_hub(zone, resolver.as_ref(), request).await {
                Ok(response) => response,
                Err(request) => serve(center, request).await,
            }
        }
    })
    .await
}
