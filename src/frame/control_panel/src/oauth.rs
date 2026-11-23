use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::exclusive_invite::random_token;
use crate::federation::{callback_error_location, did_error_location};
use crate::sso_settings::load_sso_settings;
use crate::ControlPanelServer;
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
pub(crate) use buckyos_api::external_source_for_did;
use buckyos_api::{
    get_buckyos_api_runtime, oauth_account_label, oauth_binding_key, oauth_subject_key,
    user_did_from_doc_value, valid_oauth_id, AuthTarget, OAuthAccountLogin, OAuthSource,
    OAuthSourceKind, SsoSettings, CONTROL_PANEL_SERVICE_UNIQUE_ID,
};
use buckyos_http_server::{ServerError, ServerResult};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use bytes::Bytes;
use http_body_util::combinators::BoxBody;
use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, time::Duration};

type Result<T> = std::result::Result<T, RPCErrors>;
type HttpReq = http::Request<BoxBody<Bytes, ServerError>>;
type HttpRes = http::Response<BoxBody<Bytes, ServerError>>;
const PREFIX: &str = "services/control_panel/oauth";
const TTL: u64 = 600;

fn invalid(msg: &str) -> RPCErrors {
    RPCErrors::ParseRequestError(msg.into())
}
fn refused(msg: &str) -> RPCErrors {
    RPCErrors::NoPermission(msg.into())
}
fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|e| RPCErrors::ReasonError(e.to_string()))
}
fn parse<T: DeserializeOwned>(value: &str) -> Result<T> {
    serde_json::from_str(value).map_err(|_| RPCErrors::ReasonError("invalid OAuth record".into()))
}
fn enabled_source<'a>(settings: &'a SsoSettings, id: &str) -> Result<&'a OAuthSource> {
    settings
        .oauth_sources
        .iter()
        .find(|s| s.id == id && s.enabled)
        .ok_or_else(|| refused("oauth_source_disabled"))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct VerifiedOAuth {
    pub source_id: String,
    pub issuer: String,
    pub sub: String,
    pub email: String,
    pub label: String,
    pub did: String,
    pub webhost: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct OAuthSubject {
    pub identity: VerifiedOAuth,
    pub did: String,
    pub email: String,
    #[serde(skip)]
    pub bound_user: Option<String>,
}
fn subject_key(key: &str) -> String {
    format!("{PREFIX}/subjects/{key}")
}
async fn read_subject(kv: &dyn AgentKv, key: &str) -> Result<Option<OAuthSubject>> {
    let Some((raw, _)) = kv.get(&subject_key(key)).await? else {
        return Ok(None);
    };
    let mut record = parse::<OAuthSubject>(&raw)?;
    record.bound_user = kv
        .get(&oauth_binding_key(
            &record.identity.issuer,
            &record.identity.sub,
        ))
        .await?
        .map(|(u, _)| u);
    Ok(Some(record))
}

pub(crate) async fn subject_actions(
    kv: &dyn AgentKv,
    identity: &VerifiedOAuth,
    bind_user: Option<&str>,
) -> Result<(OAuthSubject, HashMap<String, KVAction>)> {
    let key = oauth_subject_key(&identity.issuer, &identity.sub);
    let mut actions = HashMap::new();
    let mut record = match read_subject(kv, &key).await? {
        Some(record) => record,
        None => {
            let record = OAuthSubject {
                identity: identity.clone(),
                did: identity.did.clone(),
                email: identity.email.clone(),
                bound_user: None,
            };
            if kv
                .get(&buckyos_api::oauth_label_key(&record.did))
                .await?
                .is_some()
            {
                return Err(refused("OAuth identity label belongs to another account"));
            }
            actions.insert(subject_key(&key), KVAction::Create(encode(&record)?));
            actions.insert(
                buckyos_api::oauth_label_key(&record.did),
                KVAction::Create(key),
            );
            record
        }
    };
    if let Some(user) = bind_user {
        match record.bound_user.as_deref() {
            Some(existing) if existing != user => {
                return Err(refused("OAuth subject is already bound"))
            }
            Some(_) => {}
            None => {
                actions.insert(
                    oauth_binding_key(&identity.issuer, &identity.sub),
                    KVAction::Create(user.into()),
                );
                record.bound_user = Some(user.to_owned());
            }
        }
    }
    Ok((record, actions))
}

pub(crate) async fn bound_account_did(
    kv: &dyn AgentKv,
    external_did: &str,
) -> Result<Option<String>> {
    let Some((key, _)) = kv.get(&buckyos_api::oauth_label_key(external_did)).await? else {
        return Ok(None);
    };
    let record = read_subject(kv, &key)
        .await?
        .ok_or_else(|| refused("missing OAuth subject"))?;
    if record.did != external_did {
        return Err(refused("inconsistent OAuth identity index"));
    }
    let Some(user) = record.bound_user else {
        return Ok(None);
    };
    let path = format!("users/{user}/doc");
    let (doc, _) = kv
        .get(&path)
        .await?
        .ok_or_else(|| refused("bound account is unavailable"))?;
    Ok(Some(user_did_from_doc_value(&path, &doc)?.to_string()))
}
pub(crate) async fn share_subject_did(
    kv: &dyn AgentKv,
    invite_id: &str,
    identity: &VerifiedOAuth,
) -> Result<String> {
    let (record, _) = subject_actions(kv, identity, None).await?;
    let acceptance = buckyos_api::resource_share_acceptance_key(invite_id, &record.did);
    if kv.get(&acceptance).await?.is_some() {
        return Ok(record.did);
    }
    Ok(bound_account_did(kv, &record.did)
        .await?
        .unwrap_or(record.did))
}

pub(crate) async fn resolve_email(
    kv: &dyn AgentKv,
    source_id: &str,
    email: &str,
) -> Result<String> {
    let settings = load_sso_settings(kv).await?;
    let source = enabled_source(&settings, source_id)
        .map_err(|_| refused("本区未启用该登录来源，请改邀对方的 BuckyOS 账号 DID"))?;
    let (_, label) = oauth_account_label(source.kind == OAuthSourceKind::Google, email)
        .map_err(RPCErrors::ParseRequestError)?;
    let did = format!("did:web:{label}.{}", source.webhost);
    Ok(bound_account_did(kv, &did).await?.unwrap_or(did))
}

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Purpose {
    Login,
    Share,
    UserInvite,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Begin {
    source_id: String,
    purpose: Purpose,
    invite_id: Option<String>,
    redirect_url: String,
}
impl Begin {
    fn invitation_path(&self) -> Option<String> {
        let prefix = match self.purpose {
            Purpose::Login => return None,
            Purpose::Share => "/sso/share/",
            Purpose::UserInvite => "/users/invite/",
        };
        self.invite_id.as_ref().map(|id| format!("{prefix}{id}"))
    }

    fn validate_return_page(&self) -> Result<()> {
        if let Some(path) = self.invitation_path() {
            if url::Url::parse(&self.redirect_url)
                .ok()
                .is_none_or(|url| url.path() != path)
            {
                return Err(invalid("invitation purpose and return page must match"));
            }
        }
        Ok(())
    }

    fn error_location(&self, reason: &str) -> String {
        if self.validate_return_page().is_err() {
            return did_error_location(reason, None);
        }
        callback_error_location(reason, &self.redirect_url, self.invite_id.is_some())
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Attempt {
    begin: Begin,
    source: OAuthSource,
    redirect_uri: String,
    verifier: String,
    nonce: String,
    browser: String,
}
#[derive(Serialize, Deserialize)]
struct ShareProof {
    invite_id: String,
    identity: VerifiedOAuth,
}

fn database() -> Result<Connection> {
    let dir = buckyos_kit::get_buckyos_service_data_dir("control-panel");
    std::fs::create_dir_all(&dir).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
    let path = dir.join("oauth.sqlite");
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|_| refused("OAuth attempt storage unavailable"))?;
    }
    let conn = Connection::open(path).map_err(sql_error)?;
    conn.busy_timeout(Duration::from_secs(3))
        .map_err(sql_error)?;
    conn.execute_batch("CREATE TABLE IF NOT EXISTS oauth_attempts (key TEXT PRIMARY KEY,payload TEXT NOT NULL,expires INTEGER NOT NULL);CREATE INDEX IF NOT EXISTS oauth_attempt_exp ON oauth_attempts(expires)").map_err(sql_error)?;
    Ok(conn)
}
fn sql_error(_: rusqlite::Error) -> RPCErrors {
    RPCErrors::ReasonError("OAuth attempt storage unavailable".into())
}
fn attempt_key(kind: &str, token: &str) -> String {
    format!("{kind}/{:x}", Sha256::digest(token.as_bytes()))
}
async fn save<T: Serialize>(kind: &str, value: &T) -> Result<String> {
    let token = random_token(32);
    let key = attempt_key(kind, &token);
    let payload = encode(value)?;
    tokio::task::spawn_blocking(move || {
        let mut conn = database()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(sql_error)?;
        let now = buckyos_get_unix_timestamp();
        tx.execute("DELETE FROM oauth_attempts WHERE expires<=?1", [now])
            .map_err(sql_error)?;
        let count: u64 = tx
            .query_row("SELECT COUNT(*) FROM oauth_attempts", [], |r| r.get(0))
            .map_err(sql_error)?;
        if count >= 8192 {
            return Err(refused("too many OAuth attempts"));
        }
        tx.execute(
            "INSERT INTO oauth_attempts VALUES(?1,?2,?3)",
            params![key, payload, now + TTL],
        )
        .map_err(sql_error)?;
        tx.commit().map_err(sql_error)
    })
    .await
    .map_err(|_| refused("OAuth attempt storage unavailable"))??;
    Ok(token)
}
async fn load<T: DeserializeOwned>(kind: &str, token: &str, take: bool) -> Result<T> {
    if token.len() != 43 {
        return Err(refused("invalid OAuth attempt"));
    }
    let key = attempt_key(kind, token);
    let raw: Option<String> = tokio::task::spawn_blocking(move || {
        let conn = database()?;
        let sql = if take {
            "DELETE FROM oauth_attempts WHERE key=?1 AND expires>?2 RETURNING payload"
        } else {
            "SELECT payload FROM oauth_attempts WHERE key=?1 AND expires>?2"
        };
        conn.query_row(sql, params![key, buckyos_get_unix_timestamp()], |r| {
            r.get(0)
        })
        .optional()
        .map_err(sql_error)
    })
    .await
    .map_err(|_| refused("OAuth attempt storage unavailable"))??;
    parse(&raw.ok_or_else(|| refused("expired OAuth attempt"))?)
}

pub(crate) async fn share_identity(proof: &str, invite_id: &str) -> Result<VerifiedOAuth> {
    let proof = load::<ShareProof>("proof", proof, false).await?;
    if proof.invite_id != invite_id {
        return Err(refused("OAuth proof is for another invite"));
    }
    let settings = load_sso_settings(&ZoneKv::connect().await?).await?;
    let source = enabled_source(&settings, &proof.identity.source_id)?;
    if source.issuer != proof.identity.issuer {
        return Err(refused("OAuth source changed"));
    }
    Ok(proof.identity)
}

fn verified(source: &OAuthSource, sub: &str, email: &str) -> Result<VerifiedOAuth> {
    if sub.is_empty() || sub.len() > 512 || sub.chars().any(char::is_control) {
        return Err(refused("invalid provider account ID"));
    }
    let (email, label) = oauth_account_label(source.kind == OAuthSourceKind::Google, email)
        .map_err(RPCErrors::ParseRequestError)?;
    Ok(VerifiedOAuth {
        source_id: source.id.clone(),
        issuer: source.issuer.clone(),
        sub: sub.into(),
        email,
        did: format!("did:web:{label}.{}", source.webhost),
        label,
        webhost: source.webhost.clone(),
    })
}

fn issuer_matches(source: &OAuthSource, issuer: &str) -> bool {
    source.issuer == issuer
        || (source.kind == OAuthSourceKind::Google && issuer == "accounts.google.com")
}

fn verify_oidc(
    source: &OAuthSource,
    nonce: &str,
    jwt: &str,
    keys: &JwkSet,
) -> Result<VerifiedOAuth> {
    let bad = || refused("invalid OIDC identity token");
    let header = decode_header(jwt).map_err(|_| bad())?;
    if !matches!(
        header.alg,
        Algorithm::RS256 | Algorithm::ES256 | Algorithm::EdDSA
    ) || (source.kind == OAuthSourceKind::Google && header.alg != Algorithm::RS256)
    {
        return Err(bad());
    }
    let key = keys
        .find(header.kid.as_deref().ok_or_else(bad)?)
        .ok_or_else(bad)?;
    if key
        .common
        .public_key_use
        .as_ref()
        .is_some_and(|u| *u != jsonwebtoken::jwk::PublicKeyUse::Signature)
        || key
            .common
            .key_operations
            .as_ref()
            .is_some_and(|ops| !ops.contains(&jsonwebtoken::jwk::KeyOperations::Verify))
        || key
            .common
            .key_algorithm
            .as_ref()
            .is_some_and(|alg| alg.to_string() != format!("{:?}", header.alg))
    {
        return Err(bad());
    }
    let mut validation = Validation::new(header.alg);
    validation.leeway = 0;
    validation.validate_nbf = true;
    validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
    validation.set_audience(&[&source.client_id]);
    let issuers = [source.issuer.as_str(), "accounts.google.com"];
    validation.set_issuer(
        &issuers
            .iter()
            .filter(|issuer| issuer_matches(source, issuer))
            .copied()
            .collect::<Vec<_>>(),
    );
    let claims = decode::<Value>(
        jwt,
        &DecodingKey::from_jwk(key).map_err(|_| bad())?,
        &validation,
    )
    .map_err(|_| bad())?
    .claims;
    if claims["nonce"].as_str() != Some(nonce)
        || claims["email_verified"].as_bool() != Some(true)
        || (claims.get("azp").is_some()
            || claims["aud"].as_array().is_some_and(|aud| aud.len() > 1))
            && claims["azp"].as_str() != Some(&source.client_id)
    {
        return Err(bad());
    }
    verified(
        source,
        claims["sub"].as_str().ok_or_else(bad)?,
        claims["email"].as_str().ok_or_else(bad)?,
    )
}
async fn fetch(request: reqwest::RequestBuilder) -> Result<Value> {
    let error = || refused("OAuth provider request failed");
    let mut response = request
        .send()
        .await
        .map_err(|_| error())?
        .error_for_status()
        .map_err(|_| error())?;
    let mut body = Vec::new();
    while let Some(part) = response.chunk().await.map_err(|_| error())? {
        if body.len() + part.len() > 1024 * 1024 {
            return Err(error());
        }
        body.extend_from_slice(&part);
    }
    serde_json::from_slice(&body).map_err(|_| error())
}
async fn exchange(attempt: &Attempt, code: &str) -> Result<VerifiedOAuth> {
    let source = &attempt.source;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| refused("OAuth HTTP unavailable"))?;
    let secret = tokio::fs::read_to_string(
        buckyos_kit::get_buckyos_root_dir()
            .join("security/oauth")
            .join(&source.secret_ref),
    )
    .await
    .map_err(|_| refused("OAuth client credential unavailable"))?;
    exchange_with_client(attempt, code, &client, secret.trim_end()).await
}

async fn exchange_with_client(
    attempt: &Attempt,
    code: &str,
    client: &reqwest::Client,
    secret: &str,
) -> Result<VerifiedOAuth> {
    let source = &attempt.source;
    let tokens = fetch(client.post(&source.token_endpoint).form(&[
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", &attempt.redirect_uri),
        ("client_id", &source.client_id),
        ("client_secret", secret),
        ("code_verifier", &attempt.verifier),
    ]))
    .await?;
    if source.kind != OAuthSourceKind::OAuth2 {
        let jwt = tokens["id_token"]
            .as_str()
            .ok_or_else(|| refused("missing ID token"))?;
        let keys = serde_json::from_value(fetch(client.get(&source.jwks_uri)).await?)
            .map_err(|_| refused("invalid OIDC keys"))?;
        return verify_oidc(source, &attempt.nonce, jwt, &keys);
    }
    if !tokens["token_type"]
        .as_str()
        .is_some_and(|s| s.eq_ignore_ascii_case("bearer"))
    {
        return Err(refused("account endpoint needs a bearer access token"));
    }
    let access = tokens["access_token"]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| refused("missing access token"))?;
    let account = fetch(client.get(&source.account_endpoint).bearer_auth(access)).await?;
    if account[&source.email_verified_field].as_bool() != Some(true) {
        return Err(refused("provider did not verify the email address"));
    }
    let id = &account[&source.id_field];
    let sub = if let Some(s) = id.as_str() {
        s.to_owned()
    } else if let Some(n) = id.as_u64() {
        n.to_string()
    } else {
        return Err(refused("missing stable account ID"));
    };
    verified(
        source,
        &sub,
        account[&source.email_field]
            .as_str()
            .ok_or_else(|| refused("missing email"))?,
    )
}

impl ControlPanelServer {
    pub(crate) async fn handle_oauth_begin(&self, req: RPCRequest) -> Result<RPCResponse> {
        let mut begin: Begin = Self::typed_params(&req)?;
        if !valid_oauth_id(&begin.source_id) || begin.redirect_url.len() > 4096 {
            return Err(invalid("invalid OAuth request"));
        }
        match (begin.purpose, begin.invite_id.as_deref()) {
            (Purpose::Login, None) => {}
            (Purpose::Share | Purpose::UserInvite, Some(id))
                if !id.is_empty()
                    && id.len() <= 256
                    && id
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_')) => {}
            _ => return Err(invalid("OAuth invite purpose requires its invite ID")),
        }
        let runtime = get_buckyos_api_runtime()?;
        let zone = Self::sso_zone_host(runtime)?;
        let target = Self::resolve_sso_auth_target(
            &begin.redirect_url,
            &zone,
            false,
            runtime.node_gateway_port,
        )?;
        let origin = format!("https://{zone}");
        if target.canonical_origin != origin
            || !matches!(target.auth_target, AuthTarget::System{service_id} if service_id.as_str()==CONTROL_PANEL_SERVICE_UNIQUE_ID)
        {
            return Err(invalid("OAuth returns to the Zone root"));
        }
        begin.redirect_url = target.canonical_redirect_url;
        begin.validate_return_page()?;
        let mut settings = load_sso_settings(&ZoneKv::connect().await?).await?;
        buckyos_api::validate_oauth_sources(&mut settings.oauth_sources, &zone)
            .map_err(RPCErrors::ParseRequestError)?;
        let source = enabled_source(&settings, &begin.source_id)?.clone();
        let id = source.id.clone();
        let attempt = Attempt {
            begin,
            redirect_uri: source.redirect_uri.clone(),
            source,
            nonce: random_token(32),
            verifier: random_token(32),
            browser: random_token(32),
        };
        let start = save("start", &attempt).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(
                json!({"status":"redirect","url":format!("{origin}/sso/oauth/{id}/authorize?request={start}")}),
            ),
            req.seq,
        ))
    }

    pub(crate) async fn serve_oauth(&self, req: HttpReq) -> ServerResult<HttpRes> {
        let path = req.uri().path().trim_start_matches("/sso/oauth/");
        let fail = |reason: &str| Self::did_redirect(&did_error_location(reason, None), vec![]);
        let Some((source_id, action)) = path.split_once('/') else {
            return fail("authorization_unavailable");
        };
        if !valid_oauth_id(source_id) {
            return fail("authorization_unavailable");
        }
        let runtime = get_buckyos_api_runtime().map_err(Self::rpc_to_server_error)?;
        let origin = Self::request_origin(&req, false, runtime.node_gateway_port)
            .map_err(Self::rpc_to_server_error)?;
        let zone = Self::sso_zone_host(runtime).map_err(Self::rpc_to_server_error)?;
        if origin != format!("https://{zone}") {
            return fail("authorization_unavailable");
        }
        if action == "authorize" {
            let start = Self::http_query_param(&req, "request").unwrap_or_default();
            let Ok(attempt) = load::<Attempt>("start", &start, true).await else {
                return fail("authorization_unavailable");
            };
            if attempt.source.id != source_id {
                return fail("authorization_unavailable");
            }
            let reject =
                |reason: &str| Self::did_redirect(&attempt.begin.error_location(reason), vec![]);
            let Ok(kv) = ZoneKv::connect().await else {
                return reject("authorization_unavailable");
            };
            let Ok(settings) = load_sso_settings(&kv).await else {
                return reject("authorization_unavailable");
            };
            if enabled_source(&settings, source_id).ok() != Some(&attempt.source) {
                return reject("oauth_source_disabled");
            }
            let Ok(state) = save("state", &attempt).await else {
                return reject("authorization_unavailable");
            };
            let Ok(mut url) = url::Url::parse(&attempt.source.authorization_endpoint) else {
                return reject("oauth_source_disabled");
            };
            let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(attempt.verifier.as_bytes()));
            url.query_pairs_mut()
                .append_pair("response_type", "code")
                .append_pair("client_id", &attempt.source.client_id)
                .append_pair("redirect_uri", &attempt.redirect_uri)
                .append_pair("scope", &attempt.source.scopes.join(" "))
                .append_pair("state", &state)
                .append_pair("code_challenge", &challenge)
                .append_pair("code_challenge_method", "S256");
            if attempt.source.kind != OAuthSourceKind::OAuth2 {
                url.query_pairs_mut().append_pair("nonce", &attempt.nonce);
            }
            let cookie=format!("__Host-buckyos_oauth_{state}={}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={TTL}",attempt.browser);
            return Self::did_redirect(url.as_str(), vec![cookie]);
        }
        if action != "callback" {
            return fail("authorization_unavailable");
        }
        let state = Self::http_query_param(&req, "state").unwrap_or_default();
        let Ok(attempt) = load::<Attempt>("state", &state, false).await else {
            return fail("authorization_unavailable");
        };
        if attempt.source.id != source_id
            || attempt.redirect_uri != format!("{origin}/sso/oauth/{source_id}/callback")
            || Self::extract_http_cookie(&req, &format!("__Host-buckyos_oauth_{state}")).as_deref()
                != Some(&attempt.browser)
        {
            return fail("authorization_unavailable");
        }
        if load::<Attempt>("state", &state, true).await.is_err() {
            return fail("authorization_unavailable");
        }
        let return_to = &attempt.begin.redirect_url;
        let cleared = format!(
            "__Host-buckyos_oauth_{state}=; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age=0"
        );
        let decline = |reason: &str| {
            Self::did_redirect(&attempt.begin.error_location(reason), vec![cleared.clone()])
        };
        if Self::http_query_param(&req, "error").is_some() {
            return decline("access_denied");
        }
        if let Some(issuer) = Self::http_query_param(&req, "iss") {
            if !issuer_matches(&attempt.source, &issuer) {
                return decline("oauth_validation_failed");
            }
        }
        let Some(code) =
            Self::http_query_param(&req, "code").filter(|s| !s.is_empty() && s.len() <= 8192)
        else {
            return decline("authorization_unavailable");
        };
        let Ok(kv) = ZoneKv::connect().await else {
            return decline("authorization_unavailable");
        };
        let active = || async {
            load_sso_settings(&kv).await.is_ok_and(|settings| {
                enabled_source(&settings, source_id).ok() == Some(&attempt.source)
            })
        };
        if !active().await {
            return decline("oauth_source_disabled");
        }
        let Ok(identity) = exchange(&attempt, &code).await else {
            return decline("oauth_validation_failed");
        };
        if !active().await {
            return decline("oauth_source_disabled");
        }
        let (record, _) = match subject_actions(&kv, &identity, None).await {
            Ok(v) => v,
            Err(_) => return decline("oauth_identity_conflict"),
        };
        let held = Self::sso_cookie_subjects(&req);
        let expected = match attempt.begin.purpose {
            Purpose::UserInvite => match crate::user_mgr::oauth_invite_user(
                &kv,
                attempt.begin.invite_id.as_deref().unwrap_or_default(),
            )
            .await
            {
                Ok(user) => Some(user),
                Err(_) => return decline("oauth_invite_unavailable"),
            },
            _ => record.bound_user.clone(),
        };
        if held.iter().any(|s| {
            Some(s) != expected.as_ref()
                && !(attempt.begin.purpose == Purpose::Share && s == &record.did)
        }) {
            return decline("identity_conflict");
        }
        if attempt.begin.purpose == Purpose::Share {
            let invite_id = attempt.begin.invite_id.clone().unwrap_or_default();
            let cookie_name = crate::share_session::proof_cookie("oauth_share", &invite_id, true);
            let Ok(proof) = save(
                "proof",
                &ShareProof {
                    invite_id,
                    identity,
                },
            )
            .await
            else {
                return decline("authorization_unavailable");
            };
            let cookie = format!(
                "{cookie_name}={proof}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={TTL}"
            );
            return Self::did_redirect(return_to, vec![cleared, cookie]);
        }
        let user = if attempt.begin.purpose == Purpose::UserInvite {
            match crate::user_mgr::accept_oauth_invite(
                &kv,
                attempt.begin.invite_id.as_deref().unwrap_or_default(),
                &identity,
            )
            .await
            {
                Ok(user) => user,
                Err(_) => return decline("oauth_invite_unavailable"),
            }
        } else {
            let Some(user) = record.bound_user else {
                return decline("oauth_account_unbound");
            };
            user
        };
        if held.iter().any(|s| s != &user) {
            return decline("identity_conflict");
        }
        let Ok(hub) = runtime.get_verify_hub_client().await else {
            return decline("authorization_unavailable");
        };
        let Ok(pair) = hub
            .login_by_oauth_account(OAuthAccountLogin {
                source_id: identity.source_id,
                issuer: identity.issuer,
                sub: identity.sub,
                user_id: user.clone(),
            })
            .await
        else {
            return decline("oauth_account_unavailable");
        };
        let target = AuthTarget::system(
            CONTROL_PANEL_SERVICE_UNIQUE_ID
                .parse()
                .expect("control-panel service ID"),
        );
        if Self::validate_sso_token_pair(&pair.session_token, &pair.refresh_token, &target).is_err()
            || !::kRPC::RPCSessionToken::from_string(&pair.session_token)
                .is_ok_and(|token| token.sub.as_ref() == Some(&user))
        {
            let _ = hub.logout(&pair.refresh_token).await;
            return decline("authorization_unavailable");
        }
        let Ok(mut cookies) =
            Self::build_sso_cookie_headers(&req, &pair.session_token, &pair.refresh_token)
        else {
            let _ = hub.logout(&pair.refresh_token).await;
            return decline("authorization_unavailable");
        };
        cookies.push(cleared);
        Self::did_redirect(return_to, cookies)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclusive_invite::tests::MemKv;

    fn source(kind: &str) -> OAuthSource {
        let id = if kind == "google" { "google" } else { "acme" };
        let mut source:OAuthSource=serde_json::from_value(json!({
            "id":id,"display_name":"Provider","enabled":true,"kind":kind,"client_id":"desktop","secret_ref":"provider",
            "webhost":"id.example","issuer":"https://issuer.id.example","authorization_endpoint":"https://issuer.id.example/authorize",
            "token_endpoint":"https://issuer.id.example/token","account_endpoint":"https://issuer.id.example/me","jwks_uri":"https://issuer.id.example/keys",
            "id_field":"number","email_field":"email","email_verified_field":"confirmed",
        })).unwrap();
        buckyos_api::validate_oauth_sources(std::slice::from_mut(&mut source), "zone.example")
            .unwrap();
        source
    }

    #[test]
    fn verified_invitation_failures_return_to_the_original_purpose_for_retry() {
        for (purpose, path) in [
            (Purpose::Share, "/sso/share/inv-42"),
            (Purpose::UserInvite, "/users/invite/inv-42"),
        ] {
            let mut begin = Begin {
                source_id: "google".into(),
                purpose,
                invite_id: Some("inv-42".into()),
                redirect_url: format!("https://zone.example{path}?lang=en&note=a%20b&flag&did_error=older#details"),
            };
            for reason in [
                "access_denied",
                "oauth_source_disabled",
                "oauth_validation_failed",
            ] {
                assert_eq!(
                    begin.error_location(reason),
                    format!("https://zone.example{path}?lang=en&note=a%20b&flag&did_error={reason}#details")
                );
                begin.redirect_url = begin.error_location(reason);
                begin.validate_return_page().unwrap();
            }
            begin.redirect_url = "https://zone.example/users/invite/someone-else".into();
            assert!(begin.validate_return_page().is_err());
            assert_eq!(
                begin.error_location("authorization_unavailable"),
                "/login?did_error=authorization_unavailable"
            );
        }
        let ordinary = Begin {
            source_id: "google".into(),
            purpose: Purpose::Login,
            invite_id: None,
            redirect_url: "https://zone.example/".into(),
        };
        assert_eq!(
            ordinary.error_location("access_denied"),
            "/login?did_error=access_denied&redirect_url=https%3A%2F%2Fzone.example%2F"
        );
    }

    #[test]
    fn custom_domain_and_account_fields_checked_before_save() {
        let base = source("oauth2");
        for domain in [
            "zone.example",
            "sub.zone.example",
            "other.example",
            "google.com",
        ] {
            let mut s = base.clone();
            s.webhost = domain.into();
            assert!(buckyos_api::validate_oauth_sources(&mut [s], "zone.example").is_err());
        }
        let mut s = base.clone();
        s.email_verified_field.clear();
        assert!(buckyos_api::validate_oauth_sources(&mut [s], "zone.example").is_err());
        let mut other = base.clone();
        other.id = "different".into();
        assert!(buckyos_api::validate_oauth_sources(&mut [base, other], "zone.example").is_err());
        let mut preset = source("google");
        preset.token_endpoint = "https://wrong.example/token".into();
        buckyos_api::validate_oauth_sources(std::slice::from_mut(&mut preset), "zone.example")
            .unwrap();
        assert_eq!(preset.token_endpoint, "https://oauth2.googleapis.com/token");
    }

    #[test]
    fn issuer_alias_is_scoped_to_google() {
        let google = source("google");
        assert!(issuer_matches(&google, "https://accounts.google.com"));
        assert!(issuer_matches(&google, "accounts.google.com"));
        assert!(!issuer_matches(&google, "https://different.example"));
        assert!(!issuer_matches(&source("oidc"), "accounts.google.com"));
        assert_eq!(
            verified(&google, "123", "alice@gmail.com").unwrap().issuer,
            "https://accounts.google.com"
        );
    }

    #[test]
    fn signed_oidc_requires_matching_identity_and_verified_email() {
        let (pem, mut jwk) = name_lib::generate_ed25519_key_pair();
        jwk["kid"] = json!("signing");
        let keys = serde_json::from_value(json!({"keys":[jwk]})).unwrap();
        let key = jsonwebtoken::EncodingKey::from_ed_pem(pem.as_bytes()).unwrap();
        let mut header = jsonwebtoken::Header::new(Algorithm::EdDSA);
        header.kid = Some("signing".into());
        let s = source("oidc");
        let base = json!({"iss":s.issuer,"aud":"desktop","exp":buckyos_get_unix_timestamp()+300,"sub":"stable","email":"Alice@company.com","email_verified":true,"nonce":"attempt"});
        let verify = |claims: &Value| {
            verify_oidc(
                &s,
                "attempt",
                &jsonwebtoken::encode(&header, claims, &key).unwrap(),
                &keys,
            )
        };
        let who = verify(&base).unwrap();
        assert_eq!(who.email, "alice@company.com");
        for (field, value) in [
            ("email_verified", json!(false)),
            ("email", Value::Null),
            ("nonce", json!("another")),
            ("iss", json!("https://other.example")),
            ("aud", json!("another")),
            ("azp", json!("another")),
            ("exp", json!(1)),
        ] {
            let mut claims = base.clone();
            claims[field] = value;
            assert!(verify(&claims).is_err(), "{field}");
        }
    }

    async fn provider_reply(
        req: http::Request<hyper::body::Incoming>,
        token: Value,
        jwks: Value,
        redirect_uri: String,
    ) -> std::result::Result<http::Response<http_body_util::Full<Bytes>>, std::convert::Infallible>
    {
        use http_body_util::{BodyExt, Full};
        let response = match req.uri().path() {
            "/token" => {
                assert_eq!(req.method(), http::Method::POST);
                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                let form = url::form_urlencoded::parse(&bytes)
                    .collect::<std::collections::BTreeMap<_, _>>();
                assert_eq!(form["grant_type"], "authorization_code");
                assert_eq!(form["code"], "confirmed");
                assert_eq!(form["redirect_uri"], redirect_uri);
                assert_eq!(form["code_verifier"], "local-attempt-verifier");
                assert_eq!(form["client_secret"], "local-test-credential");
                token
            }
            "/me" => {
                assert_eq!(req.method(), http::Method::GET);
                assert_eq!(
                    req.headers()[http::header::AUTHORIZATION],
                    "Bearer test-access"
                );
                json!({"number":42,"email":"person@company.com","confirmed":true})
            }
            "/keys" => {
                assert_eq!(req.method(), http::Method::GET);
                jwks
            }
            _ => panic!("unregistered provider endpoint"),
        };
        Ok(http::Response::new(Full::new(Bytes::from(
            response.to_string(),
        ))))
    }

    async fn exchange_with_provider(kind: &str) -> VerifiedOAuth {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let mut source = source(kind);
        source.token_endpoint = format!("http://{addr}/token");
        source.account_endpoint = format!("http://{addr}/me");
        source.jwks_uri = format!("http://{addr}/keys");

        let email = if source.kind == OAuthSourceKind::Google {
            "alice@gmail.com"
        } else {
            "person@company.com"
        };
        let (token, jwks) = if source.kind == OAuthSourceKind::OAuth2 {
            (
                json!({"access_token":"test-access","token_type":"Bearer"}),
                Value::Null,
            )
        } else {
            let rsa = openssl::rsa::Rsa::generate(2048).unwrap();
            let key = jsonwebtoken::EncodingKey::from_rsa_pem(&rsa.private_key_to_pem().unwrap())
                .unwrap();
            let jwk = json!({"kid":"test-oidc", "kty":"RSA", "alg":"RS256", "use":"sig",
                "n":URL_SAFE_NO_PAD.encode(rsa.n().to_vec()), "e":URL_SAFE_NO_PAD.encode(rsa.e().to_vec())});
            let mut header = jsonwebtoken::Header::new(Algorithm::RS256);
            header.kid = Some("test-oidc".into());
            let claims = json!({"iss":source.issuer,"aud":source.client_id,"sub":"42","nonce":"nonce","email":email,"email_verified":true,"exp":buckyos_get_unix_timestamp()+60});
            (
                json!({"id_token":jsonwebtoken::encode(&header,&claims,&key).unwrap()}),
                json!({"keys":[jwk]}),
            )
        };
        let callback = source.redirect_uri.clone();
        let id = source.id.clone();
        let provider = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let server = hyper::service::service_fn(move |req| {
                provider_reply(req, token.clone(), jwks.clone(), callback.clone())
            });
            hyper::server::conn::http1::Builder::new()
                .serve_connection(hyper_util::rt::TokioIo::new(stream), server)
                .await
                .unwrap();
        });
        let client = reqwest::Client::new();
        let attempt = Attempt {
            redirect_uri: source.redirect_uri.clone(),
            source,
            begin: Begin {
                source_id: id,
                purpose: Purpose::Share,
                invite_id: Some("share".into()),
                redirect_url: "https://zone.example/sso/share/share".into(),
            },
            verifier: "local-attempt-verifier".into(),
            nonce: "nonce".into(),
            browser: "browser".into(),
        };
        let who = exchange_with_client(&attempt, "confirmed", &client, "local-test-credential")
            .await
            .unwrap();
        assert_eq!(who.sub, "42");
        assert_eq!(who.email, email);
        drop(client);
        tokio::time::timeout(Duration::from_secs(3), provider)
            .await
            .unwrap()
            .unwrap();
        who
    }

    #[tokio::test]
    async fn oauth2_provider_exchanges_code_and_verifies_email() {
        exchange_with_provider("oauth2").await;
    }
    #[tokio::test]
    async fn oidc_provider_exchanges_code_and_verifies_signature() {
        exchange_with_provider("oidc").await;
    }

    #[tokio::test]
    async fn binding_keeps_accepted_grants_and_routes_pending_shares_to_the_home_account() {
        let kv = MemKv::default();
        let google = source("google");
        let who = exchange_with_provider("google").await;
        let external = "did:web:alice.google.com";
        let local = "did:web:alice.zone.example";
        let mut settings = SsoSettings {
            oauth_sources: vec![google],
            ..Default::default()
        };
        kv.put(buckyos_api::SSO_SETTINGS_KEY, &settings);
        assert_eq!(
            resolve_email(&kv, "google", "Alice@GMAIL.COM")
                .await
                .unwrap(),
            external
        );
        assert_eq!(
            share_subject_did(&kv, "pending", &who).await.unwrap(),
            external
        );
        assert!(!kv.has(&buckyos_api::oauth_label_key(external)));
        let (_, mut accepted) = subject_actions(&kv, &who, None).await.unwrap();
        accepted.insert(
            buckyos_api::resource_share_acceptance_key("used", external),
            KVAction::Create("true".into()),
        );
        kv.exec_tx(accepted, None).await.unwrap();
        kv.put("users/alice/doc", &json!({"id":local}));
        let (_, binding) = subject_actions(&kv, &who, Some("alice")).await.unwrap();
        kv.exec_tx(binding, None).await.unwrap();
        assert_eq!(
            resolve_email(&kv, "google", "Alice@GMAIL.COM")
                .await
                .unwrap(),
            local
        );
        assert_eq!(
            share_subject_did(&kv, "pending", &who).await.unwrap(),
            local
        );
        assert_eq!(
            share_subject_did(&kv, "used", &who).await.unwrap(),
            external
        );
        settings.oauth_sources[0].enabled = false;
        kv.put(buckyos_api::SSO_SETTINGS_KEY, &settings);
        assert!(resolve_email(&kv, "google", "Alice@GMAIL.COM")
            .await
            .is_err());
        assert_eq!(
            bound_account_did(&kv, external).await.unwrap().as_deref(),
            Some(local)
        );
    }

    #[tokio::test]
    async fn accepting_and_binding_pin_the_subject_and_label_atomically() {
        let kv = MemKv::default();
        let s = source("google");
        let alice = verified(&s, "1", "alice@gmail.com").unwrap();
        let (first, actions) = subject_actions(&kv, &alice, None).await.unwrap();
        let (_, concurrent) = subject_actions(&kv, &alice, None).await.unwrap();
        assert!(!kv.has(&buckyos_api::oauth_label_key(&alice.did)));
        kv.exec_tx(actions, None).await.unwrap();
        assert!(kv.exec_tx(concurrent, None).await.is_err());
        let renamed = verified(&s, "1", "carol@gmail.com").unwrap();
        let (existing, actions) = subject_actions(&kv, &renamed, None).await.unwrap();
        assert_eq!(first.did, existing.did);
        assert_eq!(existing.email, "alice@gmail.com");
        assert!(actions.is_empty());
        let replacement = verified(&s, "2", "alice@gmail.com").unwrap();
        assert!(subject_actions(&kv, &replacement, None).await.is_err());
        let (_, binding) = subject_actions(&kv, &alice, Some("alice")).await.unwrap();
        let (_, other) = subject_actions(&kv, &alice, Some("other")).await.unwrap();
        kv.exec_tx(binding, None).await.unwrap();
        assert!(kv.exec_tx(other, None).await.is_err());
        assert!(subject_actions(&kv, &alice, Some("other")).await.is_err());
    }
}
