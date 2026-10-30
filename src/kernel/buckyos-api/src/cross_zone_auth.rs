//! Cross-Zone authentication (doc/arch/verify-hub/跨Zone登录断言契约.md): the signed login
//! assertion a device presents to another Zone's verify-hub, and the one custody rule every
//! service uses to decide whether a signing device may speak for a user or Agent DID.

use crate::AuthTarget;
use ::kRPC::{RPCErrorDetail, RPCErrors};
use async_trait::async_trait;
use buckyos_kit::buckyos_get_unix_timestamp;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use name_lib::{decode_jwt_claim_without_verify, DIDDocumentTrait, DidDocType, EncodedDocument, ZoneDocument, DID};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// Written by the gateway on HTTP requests that arrived over RTCP; removed on every other entry.
pub const RTCP_DEVICE_HEADER: &str = "x-cyfs-rtcp-device";
pub const RTCP_TRUST_HEADER: &str = "x-cyfs-rtcp-trust";
pub const CROSS_ZONE_LOGIN_METHOD: &str = "login_by_cross_zone_assertion";
pub const CROSS_ZONE_ASSERTION_MAX_TTL_SECS: u64 = 60;
const ASSERTION_CLOCK_SKEW_SECS: u64 = 30;
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(5);

/// Target services a cross-Zone login may ask for, with the scopes each may carry (contract §4.2).
pub fn cross_zone_target_scopes(target: &AuthTarget) -> Option<&'static [crate::SessionScope]> {
    use crate::SessionScope::*;
    match target {
        AuthTarget::System { service_id } => match service_id.as_str() {
            // Group sync acts as a federated External member would (`msg_group`).
            "msg-center" => Some(&[MsgInbox, MsgGroup]),
            "homestation" => Some(&[HomeStation]),
            _ => None,
        },
        AuthTarget::App { .. } => None,
    }
}

// ─── resolution ─────────────────────────────────────────────────────────────

/// Outcome of a name-service lookup. Only `NotFound` is an answer; the other two say nothing
/// about the DID and callers must treat them as retryable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveFailure {
    NotFound(String),
    Unavailable(String),
    Timeout,
}

impl std::fmt::Display for ResolveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(reason) => write!(f, "not found: {reason}"),
            Self::Unavailable(reason) => write!(f, "unavailable: {reason}"),
            Self::Timeout => f.write_str("timeout"),
        }
    }
}

#[async_trait]
pub trait CustodyResolver: Send + Sync {
    async fn resolve(&self, did: &DID, doc_type: Option<DidDocType>) -> Result<EncodedDocument, ResolveFailure>;
}

/// DID documents through name-client.
#[derive(Default, Clone, Copy)]
pub struct NameServiceResolver;

#[async_trait]
impl CustodyResolver for NameServiceResolver {
    async fn resolve(&self, did: &DID, doc_type: Option<DidDocType>) -> Result<EncodedDocument, ResolveFailure> {
        match tokio::time::timeout(RESOLVE_TIMEOUT, name_client::resolve_did(did, doc_type)).await {
            Ok(Ok(doc)) => Ok(doc),
            // Key-class DIDs (`did:dev`) and malformed ones have no document to wait for.
            Ok(Err(name_lib::NSError::NotFound(reason) | name_lib::NSError::InvalidDID(reason))) => {
                Err(ResolveFailure::NotFound(reason))
            }
            Ok(Err(name_lib::NSError::DeadlineExceeded(_))) | Err(_) => Err(ResolveFailure::Timeout),
            Ok(Err(error)) => Err(ResolveFailure::Unavailable(error.to_string())),
        }
    }
}

/// Documents declared up front, for tests and standalone setups: `(did, doc type)` → document;
/// a `None` type entry answers any type that has no entry of its own.
#[derive(Default, Clone)]
pub struct StaticCustodyResolver {
    docs: HashMap<(String, Option<String>), Value>,
    unavailable: Vec<String>,
}

impl StaticCustodyResolver {
    pub fn with_doc(mut self, did: &DID, doc_type: Option<DidDocType>, doc: Value) -> Self {
        self.docs.insert((did.to_string(), doc_type.map(|t| t.to_string())), doc);
        self
    }

    /// Lookups of `did` fail as `Unavailable`.
    pub fn with_unavailable(mut self, did: &DID) -> Self {
        self.unavailable.push(did.to_string());
        self
    }
}

#[async_trait]
impl CustodyResolver for StaticCustodyResolver {
    async fn resolve(&self, did: &DID, doc_type: Option<DidDocType>) -> Result<EncodedDocument, ResolveFailure> {
        let key = did.to_string();
        if self.unavailable.contains(&key) {
            return Err(ResolveFailure::Unavailable(key));
        }
        let typed = (key.clone(), doc_type.as_ref().map(|t| t.to_string()));
        self.docs
            .get(&typed)
            .or_else(|| doc_type.is_some().then(|| self.docs.get(&(key.clone(), None))).flatten())
            .cloned()
            .map(EncodedDocument::JsonLd)
            .ok_or(ResolveFailure::NotFound(key))
    }
}

/// Zone identity for comparisons: `did:bns:alice` and `did:web:alice.bns.did` are one Zone.
pub fn zone_host(did: &DID) -> String {
    did.to_host_name().trim_end_matches('.').to_ascii_lowercase()
}

pub fn same_zone(a: &DID, b: &DID) -> bool {
    zone_host(a) == zone_host(b)
}

fn doc_context(doc: &Value) -> String {
    doc.get("@context").map(Value::to_string).unwrap_or_default()
}

fn did_field(doc: &Value, key: &str) -> Option<DID> {
    doc.get(key).and_then(Value::as_str).and_then(|raw| DID::from_str(raw).ok())
}

/// The Zone a device document places its device in.
pub fn device_zone(doc: &Value) -> Option<DID> {
    did_field(doc, "zone_did")
}

/// Canonical Zone of a principal (contract §3 step 4). `local` is what the caller's own records
/// say (a user or Agent this Zone hosts) and wins over the name service. `Ok(None)`: the
/// principal has no Zone (no document, or a document that names none).
pub async fn canonical_zone_of(
    resolver: &dyn CustodyResolver,
    did: &DID,
    local: Option<DID>,
) -> Result<Option<DID>, ResolveFailure> {
    if local.is_some() {
        return Ok(local);
    }
    match resolve_principal_document(resolver, did).await? {
        Some(doc) => canonical_zone_from_doc(resolver, did, &doc).await,
        None => Ok(None),
    }
}

/// The document a user or Agent DID publishes: its OwnerDocument, else its default document
/// (an Agent's). `Ok(None)` when there is neither.
async fn resolve_principal_document(resolver: &dyn CustodyResolver, did: &DID) -> Result<Option<Value>, ResolveFailure> {
    let encoded = match resolver.resolve(did, Some(DidDocType::Owner)).await {
        Err(ResolveFailure::NotFound(_)) => resolver.resolve(did, None).await,
        result => result,
    };
    match encoded {
        Ok(doc) => Ok(doc.to_json_value().ok()),
        Err(ResolveFailure::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

async fn canonical_zone_from_doc(
    resolver: &dyn CustodyResolver,
    did: &DID,
    doc: &Value,
) -> Result<Option<DID>, ResolveFailure> {
    let context = doc_context(doc);
    if let Some(zones) = doc.get("binded_zone_list").and_then(Value::as_array) {
        return Ok(Some(match zones.first() {
            Some(zone) => match zone.as_str().and_then(|raw| DID::from_str(raw).ok()) {
                Some(zone) => zone,
                None => return Ok(None),
            },
            None => did.clone(),
        }));
    }
    if context.contains("ns/owner") {
        // A self-hosted Zone: its owner DID is the Zone DID and binds no other Zone.
        return Ok(Some(did.clone()));
    }
    if context.contains("ns/agent") || doc.get("httpServicePorts").is_some() {
        let Some(owner) = did_field(doc, "owner").filter(|owner| owner != did) else {
            return Ok(None);
        };
        let Some(owner_doc) = resolve_principal_document(resolver, &owner).await? else {
            return Ok(None);
        };
        if owner_doc.get("binded_zone_list").is_none() && !doc_context(&owner_doc).contains("ns/owner") {
            return Ok(None);
        }
        return Box::pin(canonical_zone_from_doc(resolver, &owner, &owner_doc)).await;
    }
    Ok(did_field(doc, "zone_did"))
}

/// Hostname a Zone serves under: its ZoneDocument's `hostname`.
pub async fn zone_hostname(resolver: &dyn CustodyResolver, zone: &DID) -> Result<String, ResolveFailure> {
    let encoded = resolver.resolve(zone, Some(DidDocType::Zone)).await?;
    let doc = ZoneDocument::decode(&encoded, None).map_err(|error| ResolveFailure::NotFound(error.to_string()))?;
    let host = crate::zone_document_hostname(&doc).to_ascii_lowercase();
    if host.is_empty() || host.chars().any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '.')) {
        return Err(ResolveFailure::NotFound(format!("ZoneDocument of {} has no usable hostname", zone.to_string())));
    }
    Ok(host)
}

// ─── custody ────────────────────────────────────────────────────────────────

/// Why a signer speaks for a principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Custody {
    /// The principal signed with its own key.
    Signer,
    /// The principal's document lists the signing key.
    ListedInDoc,
    /// A device of the Zone that hosts the principal (known risk: a Zone signs for its users).
    SameZone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CustodyError {
    /// No key behind the kid: no document, or the document does not list it.
    UnknownSigner,
    /// The key is real but may not speak for the principal.
    NotAuthorized,
    /// A document could not be resolved right now; retry later.
    Unavailable(ResolveFailure),
}

impl std::fmt::Display for CustodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSigner => f.write_str("unknown signing key"),
            Self::NotAuthorized => f.write_str("signer may not speak for the principal"),
            Self::Unavailable(failure) => write!(f, "signer or principal unresolvable: {failure}"),
        }
    }
}

impl From<ResolveFailure> for CustodyError {
    fn from(failure: ResolveFailure) -> Self {
        Self::Unavailable(failure)
    }
}

/// A resolved signing key.
#[derive(Clone)]
pub struct SignerKey {
    pub did: DID,
    pub kid: String,
    pub key: DecodingKey,
    /// The signer's document; none for key-class DIDs (`did:dev`).
    pub doc: Option<Value>,
}

impl SignerKey {
    pub fn zone(&self) -> Option<DID> {
        self.doc.as_ref().and_then(device_zone)
    }
}

pub fn kid_did(kid: &str) -> Option<DID> {
    DID::from_str(kid.split('#').next().unwrap_or(kid)).ok()
}

fn kid_fragment(kid: &str) -> Option<String> {
    kid.split_once('#').map(|(_, fragment)| format!("#{fragment}"))
}

/// The key behind `kid` (contract §3 step 1): a `did:dev` carries it; otherwise the device
/// document, else the default document, of the kid's DID.
pub async fn resolve_signer_key(resolver: &dyn CustodyResolver, kid: &str) -> Result<SignerKey, CustodyError> {
    let did = kid_did(kid).ok_or(CustodyError::UnknownSigner)?;
    if did.method == "dev" {
        let key = DecodingKey::from_ed_components(&did.id).map_err(|_| CustodyError::UnknownSigner)?;
        return Ok(SignerKey { did, kid: kid.to_string(), key, doc: None });
    }
    let encoded = match resolve_device_document(resolver, &did).await {
        Ok(doc) => doc,
        Err(ResolveFailure::NotFound(_)) => return Err(CustodyError::UnknownSigner),
        Err(error) => return Err(error.into()),
    };
    let doc = encoded.clone().to_json_value().map_err(|_| CustodyError::UnknownSigner)?;
    let key = name_lib::parse_did_doc(encoded)
        .ok()
        .and_then(|parsed| parsed.get_auth_key(kid_fragment(kid).as_deref()))
        .map(|(key, _)| key)
        .ok_or(CustodyError::UnknownSigner)?;
    Ok(SignerKey { did, kid: kid.to_string(), key, doc: Some(doc) })
}

/// The device document of `did`, else its default document.
async fn resolve_device_document(resolver: &dyn CustodyResolver, did: &DID) -> Result<EncodedDocument, ResolveFailure> {
    match resolver.resolve(did, Some(DidDocType::Device)).await {
        Err(ResolveFailure::NotFound(_)) => resolver.resolve(did, None).await,
        result => result,
    }
}

/// The Zone a device is in, by its document; `Ok(None)` when it has no document or names none.
pub async fn device_zone_of(resolver: &dyn CustodyResolver, did: &DID) -> Result<Option<DID>, ResolveFailure> {
    match resolve_device_document(resolver, did).await {
        Ok(doc) => Ok(doc.to_json_value().ok().as_ref().and_then(device_zone)),
        Err(ResolveFailure::NotFound(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Whether `doc` lists `signer` / `kid` among its authentication keys. A relative id (`#key`)
/// is a key of the document's own DID, never of another signer using the same fragment.
pub fn lists_signer(doc: &Value, signer: &DID, kid: &str) -> bool {
    let doc_id = doc.get("id").and_then(Value::as_str).unwrap_or_default();
    let signer = signer.to_string();
    doc.get("authentication").and_then(Value::as_array).is_some_and(|items| {
        items.iter().any(|item| {
            let id = item.as_str().or_else(|| item.get("id").and_then(Value::as_str)).unwrap_or("");
            let id = if id.starts_with('#') { format!("{doc_id}{id}") } else { id.to_string() };
            id == kid || id == signer
        })
    })
}

/// May `signer` speak for `principal` (contract §3)? `local_principal_zone`: the Zone this
/// service's own records say hosts `principal` (its users and Agents), if any; it is one more
/// way in, never a reason to refuse what the documents allow.
pub async fn check_custody(
    resolver: &dyn CustodyResolver,
    signer: &SignerKey,
    principal: &DID,
    local_principal_zone: Option<DID>,
) -> Result<Custody, CustodyError> {
    if *principal == signer.did {
        return Ok(Custody::Signer);
    }
    let device_zone = signer.zone();
    if let (Some(device_zone), Some(local)) = (&device_zone, &local_principal_zone) {
        if same_zone(device_zone, local) {
            return Ok(Custody::SameZone);
        }
    }
    let doc = resolve_principal_document(resolver, principal).await?;
    if doc.as_ref().is_some_and(|doc| lists_signer(doc, &signer.did, &signer.kid)) {
        return Ok(Custody::ListedInDoc);
    }
    let Some(device_zone) = device_zone else {
        return Err(CustodyError::NotAuthorized);
    };
    // The local hint only adds a way in; the principal's own documents still count.
    let principal_zone = match doc {
        Some(doc) => canonical_zone_from_doc(resolver, principal, &doc).await?,
        None => None,
    };
    match principal_zone {
        Some(zone) if same_zone(&zone, &device_zone) => Ok(Custody::SameZone),
        _ => Err(CustodyError::NotAuthorized),
    }
}

// ─── signing ────────────────────────────────────────────────────────────────

/// This node's device key, the key Zone services sign with (`<device did>#main_key`).
#[derive(Clone)]
pub struct DeviceSigner {
    pub device_did: DID,
    pub kid: String,
    key: EncodingKey,
}

impl DeviceSigner {
    pub fn new(device_did: DID, key: EncodingKey) -> Self {
        let kid = format!("{}#main_key", device_did.to_string());
        Self { device_did, kid, key }
    }

    /// A key its document lists under another id than `#main_key`.
    pub fn with_kid(device_did: DID, kid: String, key: EncodingKey) -> Self {
        Self { device_did, kid, key }
    }

    pub fn key(&self) -> &EncodingKey {
        &self.key
    }

    pub fn sign<T: Serialize>(&self, claims: &T) -> Result<String, RPCErrors> {
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.kid.clone());
        jsonwebtoken::encode(&header, claims, &self.key)
            .map_err(|error| RPCErrors::ReasonError(format!("sign with device key failed: {error}")))
    }
}

impl std::fmt::Debug for DeviceSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceSigner").field("kid", &self.kid).finish()
    }
}

/// Claims of a cross-Zone login assertion (contract §2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CrossZoneAssertionClaims {
    pub iss: String,
    pub sub: String,
    pub dev: String,
    pub aud: String,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub jti: String,
    pub iat: u64,
    pub exp: u64,
}

pub fn sign_cross_zone_assertion(
    signer: &DeviceSigner,
    principal: &DID,
    target_zone: &DID,
    target: &AuthTarget,
    app: Option<&str>,
) -> Result<String, RPCErrors> {
    let iat = buckyos_get_unix_timestamp();
    signer.sign(&CrossZoneAssertionClaims {
        iss: principal.to_string(),
        sub: principal.to_string(),
        dev: signer.device_did.to_string(),
        aud: target_zone.to_string(),
        target: target.canonical_key(),
        app: app.map(str::to_string),
        jti: uuid::Uuid::new_v4().to_string(),
        iat,
        exp: iat + CROSS_ZONE_ASSERTION_MAX_TTL_SECS,
    })
}

// ─── verification ───────────────────────────────────────────────────────────

/// Wire error codes of `login_by_cross_zone_assertion` (contract §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossZoneLoginError {
    AssertionInvalid,
    CustodyRejected,
    ZoneUnavailable,
    RtcpIdentityMismatch,
    TargetNotAllowed,
}

impl CrossZoneLoginError {
    pub const ALL: [Self; 5] = [
        Self::AssertionInvalid,
        Self::CustodyRejected,
        Self::ZoneUnavailable,
        Self::RtcpIdentityMismatch,
        Self::TargetNotAllowed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AssertionInvalid => "assertion_invalid",
            Self::CustodyRejected => "custody_rejected",
            Self::ZoneUnavailable => "zone_unavailable",
            Self::RtcpIdentityMismatch => "rtcp_identity_mismatch",
            Self::TargetNotAllowed => "target_not_allowed",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|error| error.as_str() == code)
    }

    pub fn is_retryable(self) -> bool {
        self == Self::ZoneUnavailable
    }

    pub fn into_rpc_error(self, detail: impl std::fmt::Display) -> RPCErrors {
        RPCErrors::Detailed(RPCErrorDetail::new(self.as_str(), format!("{}: {detail}", self.as_str()), None))
    }

    pub fn from_rpc_error(error: &RPCErrors) -> Option<Self> {
        Self::parse(&error.detail()?.code)
    }
}

/// A rejected assertion: the wire code and what to log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrossZoneLoginFailure {
    pub code: CrossZoneLoginError,
    pub detail: String,
}

impl CrossZoneLoginFailure {
    pub fn new(code: CrossZoneLoginError, detail: impl Into<String>) -> Self {
        Self { code, detail: detail.into() }
    }

    pub fn into_rpc_error(self) -> RPCErrors {
        self.code.into_rpc_error(self.detail)
    }
}

impl From<CustodyError> for CrossZoneLoginFailure {
    fn from(error: CustodyError) -> Self {
        let code = match &error {
            CustodyError::UnknownSigner => CrossZoneLoginError::AssertionInvalid,
            CustodyError::NotAuthorized => CrossZoneLoginError::CustodyRejected,
            CustodyError::Unavailable(_) => CrossZoneLoginError::ZoneUnavailable,
        };
        Self::new(code, error.to_string())
    }
}

/// Whom a verified assertion speaks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRequester {
    pub requester: DID,
    pub device: DID,
    /// The signing device's Zone; none when the principal signed with its own key.
    pub device_zone: Option<DID>,
    pub custody: Custody,
    pub target: AuthTarget,
    pub app: Option<String>,
    pub jti: String,
    pub exp: u64,
}

/// Verify a login assertion addressed to this Zone (contract §2, §3): `aud` names `own_zone`,
/// or `did:web:<own_hostname>` (a caller that only knows the Zone's hostname). Replay of `jti`
/// is the caller's to check.
pub async fn verify_signed_assertion(
    resolver: &dyn CustodyResolver,
    jwt: &str,
    own_zone: &DID,
    own_hostname: &str,
    now: u64,
) -> Result<VerifiedRequester, CrossZoneLoginFailure> {
    use CrossZoneLoginError::*;
    let invalid = |detail: &str| CrossZoneLoginFailure::new(AssertionInvalid, detail);
    let header = jsonwebtoken::decode_header(jwt).map_err(|_| invalid("malformed assertion"))?;
    if header.alg != Algorithm::EdDSA {
        return Err(invalid("assertion must be EdDSA"));
    }
    let kid = header.kid.filter(|kid| kid.contains('#')).ok_or_else(|| invalid("assertion has no key id"))?;
    // Reject malformed claims before resolving anything they name.
    let unverified: CrossZoneAssertionClaims = decode_jwt_claim_without_verify(jwt)
        .ok()
        .and_then(|value| serde_json::from_value(value).ok())
        .ok_or_else(|| invalid("malformed assertion claims"))?;
    let device = kid_did(&kid).ok_or_else(|| invalid("assertion key id is not a DID"))?;
    if DID::from_str(&unverified.dev).ok().as_ref() != Some(&device) {
        return Err(invalid("dev does not match the key id"));
    }
    let aud = DID::from_str(&unverified.aud).map_err(|_| invalid("aud is not a DID"))?;
    let own_hostname = own_hostname.trim_end_matches('.').to_ascii_lowercase();
    if !same_zone(&aud, own_zone) && zone_host(&aud) != own_hostname {
        return Err(invalid("assertion is for another Zone"));
    }
    if unverified.exp <= now
        || unverified.iat > now + ASSERTION_CLOCK_SKEW_SECS
        || unverified.exp <= unverified.iat
        || unverified.exp - unverified.iat > CROSS_ZONE_ASSERTION_MAX_TTL_SECS
    {
        return Err(invalid("assertion expired or too long-lived"));
    }
    let signer = resolve_signer_key(resolver, &kid).await?;
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.required_spec_claims.clear();
    validation.validate_exp = false;
    validation.validate_aud = false;
    let claims = jsonwebtoken::decode::<CrossZoneAssertionClaims>(jwt, &signer.key, &validation)
        .map_err(|_| invalid("assertion signature does not verify"))?
        .claims;
    if claims != unverified || claims.iss != claims.sub || claims.jti.is_empty() {
        return Err(invalid("inconsistent assertion claims"));
    }
    let requester = DID::from_str(&claims.iss).map_err(|_| invalid("iss is not a DID"))?;
    let target = AuthTarget::from_canonical_key(&claims.target)
        .map_err(|_| CrossZoneLoginFailure::new(TargetNotAllowed, "target is not a service"))?;
    let custody = check_custody(resolver, &signer, &requester, None).await?;
    Ok(VerifiedRequester {
        requester,
        device_zone: signer.zone(),
        device,
        custody,
        target,
        app: claims.app,
        jti: claims.jti,
        exp: claims.exp,
    })
}

/// One table of custody cases every caller of [`check_custody`] runs in its own tests
/// (msg-center message signatures, HomeStation reader and object signers, verify-hub logins):
/// the same `(signing key, principal)` gets the same answer everywhere, and a Zone that cannot be
/// resolved is retryable everywhere.
pub mod custody_cases {
    use super::StaticCustodyResolver;
    use name_lib::{DidDocType, DID};
    use serde_json::{json, Value};

    pub const ALICE_OOD_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
    pub const MALLORY_OOD_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----";
    const ALICE_OOD_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
    const MALLORY_OOD_X: &str = "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0";
    /// The Zone every case's verifier belongs to.
    pub const TARGET_ZONE: &str = "did:web:target.example";

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Outcome {
        Allowed,
        Refused,
        Retry,
    }

    pub struct Case {
        /// Signing device DID; its key id is `#main_key`.
        pub device: &'static str,
        pub key_pem: &'static str,
        pub principal: &'static str,
        pub outcome: Outcome,
    }

    fn device(id: &str, x: &str, zone: &str) -> Value {
        json!({
            "id": id,
            "verificationMethod": [{"type": "Ed25519VerificationKey2020", "id": "#main_key", "controller": id,
                "publicKeyJwk": {"kty": "OKP", "crv": "Ed25519", "x": x}}],
            "authentication": ["#main_key"],
            "exp": 4102444800u64, "iat": 1,
            "zone_did": zone, "owner": zone, "device_type": "ood", "name": "ood1"
        })
    }

    pub fn resolver() -> StaticCustodyResolver {
        let did = |value: &str| DID::from_str(value).unwrap();
        StaticCustodyResolver::default()
            .with_doc(&did("did:web:ood1.alice.example"), Some(DidDocType::Device),
                device("did:web:ood1.alice.example", ALICE_OOD_X, "did:web:alice.example"))
            .with_doc(&did("did:web:ood1.mallory.example"), Some(DidDocType::Device),
                device("did:web:ood1.mallory.example", MALLORY_OOD_X, "did:web:mallory.example"))
            .with_doc(&did("did:web:alice.example"), None,
                json!({"@context": "https://www.buckyos.org/ns/owner", "id": "did:web:alice.example"}))
            .with_doc(&did("did:web:bob.example"), None,
                json!({"@context": "https://www.buckyos.org/ns/owner", "id": "did:web:bob.example",
                    "binded_zone_list": ["did:web:alice.example"]}))
            .with_unavailable(&did("did:web:down.example"))
    }

    pub fn cases() -> Vec<Case> {
        use Outcome::*;
        let case = |device, key_pem, principal, outcome| Case { device, key_pem, principal, outcome };
        vec![
            case("did:web:ood1.alice.example", ALICE_OOD_KEY, "did:web:alice.example", Allowed),
            case("did:web:ood1.alice.example", ALICE_OOD_KEY, "did:web:bob.example", Allowed),
            case("did:web:ood1.mallory.example", MALLORY_OOD_KEY, "did:web:alice.example", Refused),
            case("did:dev:T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8", ALICE_OOD_KEY, "did:web:alice.example", Refused),
            case("did:web:ood1.alice.example", ALICE_OOD_KEY, "did:web:nobody.example", Refused),
            case("did:web:ood1.alice.example", ALICE_OOD_KEY, "did:web:down.example", Retry),
        ]
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub const DEVICE_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr\n-----END PRIVATE KEY-----";
    pub const DEVICE_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
    const OTHER_KEY: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIF5zplRSY5MsYbBMdbzItH90daUlrx/OflPm8Kp9m2j1\n-----END PRIVATE KEY-----";
    const OTHER_X: &str = "mL_tm5diVGGkpt_6WmCZQxUsee4rHacyz7mx7TkKb-0";

    fn did(value: &str) -> DID {
        DID::from_str(value).unwrap()
    }

    fn device_doc(id: &str, x: &str, zone: &str) -> Value {
        json!({
            "@context": "https://www.w3.org/ns/did/v1",
            "id": id,
            "verificationMethod": [{"type": "Ed25519VerificationKey2020", "id": "#main_key", "controller": id,
                "publicKeyJwk": {"kty": "OKP", "crv": "Ed25519", "x": x}}],
            "authentication": ["#main_key"],
            "exp": 4102444800u64, "iat": 1,
            "zone_did": zone, "owner": "did:web:alice.example", "device_type": "ood", "name": "ood1"
        })
    }

    fn owner_doc(id: &str, zones: &[&str]) -> Value {
        json!({"@context": "https://www.buckyos.org/ns/owner", "id": id, "binded_zone_list": zones,
            "authentication": ["#main_key"]})
    }

    pub fn fixture() -> StaticCustodyResolver {
        StaticCustodyResolver::default()
            .with_doc(&did("did:web:ood1.alice.example"), Some(DidDocType::Device),
                device_doc("did:web:ood1.alice.example", DEVICE_X, "did:web:alice.example"))
            .with_doc(&did("did:web:ood1.mallory.example"), Some(DidDocType::Device),
                device_doc("did:web:ood1.mallory.example", OTHER_X, "did:web:mallory.example"))
            .with_doc(&did("did:web:alice.example"), None, owner_doc("did:web:alice.example", &[]))
            .with_doc(&did("did:web:bob.example"), None, owner_doc("did:web:bob.example", &["did:web:alice.example"]))
            .with_doc(&did("did:web:jarvis.alice.example"), None, json!({
                "@context": "https://www.buckyos.org/ns/agent", "id": "did:web:jarvis.alice.example",
                "owner": "did:web:alice.example", "authentication": ["#main_key"]}))
    }

    pub fn signer(device: &str, key: &str) -> DeviceSigner {
        DeviceSigner::new(did(device), EncodingKey::from_ed_pem(key.as_bytes()).unwrap())
    }

    fn msg_center() -> AuthTarget {
        AuthTarget::from_canonical_key("system:msg-center").unwrap()
    }

    #[tokio::test]
    async fn custody_follows_the_one_rule() {
        let resolver = fixture();
        let alice_ood = resolve_signer_key(&resolver, "did:web:ood1.alice.example#main_key").await.unwrap();
        let mallory_ood = resolve_signer_key(&resolver, "did:web:ood1.mallory.example#main_key").await.unwrap();
        let dev = resolve_signer_key(&resolver, &format!("did:dev:{DEVICE_X}#main_key")).await.unwrap();
        let cases = [
            (&alice_ood, "did:web:ood1.alice.example", Ok(Custody::Signer)),
            (&alice_ood, "did:web:alice.example", Ok(Custody::SameZone)),
            // bob binds alice.example as his canonical Zone.
            (&alice_ood, "did:web:bob.example", Ok(Custody::SameZone)),
            // An Agent lives in its owner's canonical Zone.
            (&alice_ood, "did:web:jarvis.alice.example", Ok(Custody::SameZone)),
            (&mallory_ood, "did:web:alice.example", Err(CustodyError::NotAuthorized)),
            (&alice_ood, "did:web:nobody.example", Err(CustodyError::NotAuthorized)),
            // A key-class DID has no Zone: it speaks only for itself.
            (&dev, "did:web:alice.example", Err(CustodyError::NotAuthorized)),
        ];
        for (signer, principal, expected) in cases {
            assert_eq!(check_custody(&resolver, signer, &did(principal), None).await, expected, "{principal}");
        }
        // The caller's own records answer first.
        assert_eq!(
            check_custody(&resolver, &alice_ood, &did("did:web:nobody.example"), Some(did("did:web:alice.example"))).await,
            Ok(Custody::SameZone)
        );
        let listed = fixture().with_doc(&did("did:web:carol.example"), None, json!({
            "id": "did:web:carol.example", "authentication": ["did:web:ood1.mallory.example#main_key"]}));
        assert_eq!(check_custody(&listed, &mallory_ood, &did("did:web:carol.example"), None).await, Ok(Custody::ListedInDoc));
        // A BNS name answers its Zone document by default: custody reads its OwnerDocument.
        let bns = fixture()
            .with_doc(&did("did:bns:carol"), Some(DidDocType::Owner), owner_doc("did:bns:carol", &["did:web:alice.example"]))
            .with_doc(&did("did:bns:carol"), None, json!({"@context": ["https://www.w3.org/ns/did/v1", "https://buckyos.org/ns/zone/v1"],
                "id": "did:bns:carol", "hostname": "carol.example", "owner": "did:bns:carol"}));
        assert_eq!(check_custody(&bns, &alice_ood, &did("did:bns:carol"), None).await, Ok(Custody::SameZone));
        assert_eq!(canonical_zone_of(&bns, &did("did:bns:carol"), None).await, Ok(Some(did("did:web:alice.example"))));
        // A relative id belongs to the document's own DID.
        assert!(!lists_signer(&owner_doc("did:web:alice.example", &[]), &mallory_ood.did, &mallory_ood.kid));
    }

    #[tokio::test]
    async fn unresolvable_documents_are_retryable_not_rejections() {
        let resolver = fixture().with_unavailable(&did("did:web:bob.example"));
        let alice_ood = resolve_signer_key(&resolver, "did:web:ood1.alice.example#main_key").await.unwrap();
        assert!(matches!(
            check_custody(&resolver, &alice_ood, &did("did:web:bob.example"), None).await,
            Err(CustodyError::Unavailable(_))
        ));
        let down = fixture().with_unavailable(&did("did:web:ood1.alice.example"));
        assert!(matches!(
            resolve_signer_key(&down, "did:web:ood1.alice.example#main_key").await,
            Err(CustodyError::Unavailable(_))
        ));
        assert!(matches!(
            resolve_signer_key(&resolver, "did:web:ood9.alice.example#main_key").await,
            Err(CustodyError::UnknownSigner)
        ));
    }

    #[tokio::test]
    async fn assertions_verify_only_for_their_zone_signer_and_window() {
        let resolver = fixture();
        let zone = did("did:web:target.example");
        let alice = signer("did:web:ood1.alice.example", DEVICE_KEY);
        let jwt = sign_cross_zone_assertion(&alice, &did("did:web:bob.example"), &zone, &msg_center(), Some("app.example")).unwrap();
        let now = buckyos_get_unix_timestamp();
        let verified = verify_signed_assertion(&resolver, &jwt, &zone, "target.example", now).await.unwrap();
        assert_eq!(verified.requester, did("did:web:bob.example"));
        assert_eq!(verified.device, did("did:web:ood1.alice.example"));
        assert_eq!(verified.device_zone, Some(did("did:web:alice.example")));
        assert_eq!(verified.custody, Custody::SameZone);
        assert_eq!(verified.target, msg_center());
        assert_eq!(verified.app.as_deref(), Some("app.example"));

        // A caller that knows only the hostname addresses `did:web:<hostname>`.
        let by_host = sign_cross_zone_assertion(&alice, &did("did:web:target.example"), &did("did:web:serve.example"), &msg_center(), None).unwrap();
        assert!(verify_signed_assertion(&resolver, &by_host, &did("did:bns:target"), "serve.example", now).await.is_err());
        let by_host = sign_cross_zone_assertion(&alice, &did("did:web:bob.example"), &did("did:web:serve.example"), &msg_center(), None).unwrap();
        assert!(verify_signed_assertion(&resolver, &by_host, &did("did:bns:target"), "serve.example", now).await.is_ok());
        let code = |result: Result<VerifiedRequester, CrossZoneLoginFailure>| result.unwrap_err().code;
        assert_eq!(code(verify_signed_assertion(&resolver, &jwt, &did("did:web:other.example"), "other.example", now).await), CrossZoneLoginError::AssertionInvalid);
        assert_eq!(code(verify_signed_assertion(&resolver, &jwt, &zone, "target.example", now + 61).await), CrossZoneLoginError::AssertionInvalid);
        // Another Zone's device cannot log in as bob.
        let mallory = signer("did:web:ood1.mallory.example", OTHER_KEY);
        let forged = sign_cross_zone_assertion(&mallory, &did("did:web:bob.example"), &zone, &msg_center(), None).unwrap();
        assert_eq!(code(verify_signed_assertion(&resolver, &forged, &zone, "target.example", now).await), CrossZoneLoginError::CustodyRejected);
        // A bare did:dev key signs for nobody but itself.
        let bare = signer(&format!("did:dev:{DEVICE_X}"), DEVICE_KEY);
        let bare = sign_cross_zone_assertion(&bare, &did("did:web:bob.example"), &zone, &msg_center(), None).unwrap();
        assert_eq!(code(verify_signed_assertion(&resolver, &bare, &zone, "target.example", now).await), CrossZoneLoginError::CustodyRejected);
        // Signed by a key the device document does not hold.
        let wrong_key = signer("did:web:ood1.alice.example", OTHER_KEY);
        let wrong_key = sign_cross_zone_assertion(&wrong_key, &did("did:web:bob.example"), &zone, &msg_center(), None).unwrap();
        assert_eq!(code(verify_signed_assertion(&resolver, &wrong_key, &zone, "target.example", now).await), CrossZoneLoginError::AssertionInvalid);
        let unknown_target = sign_cross_zone_assertion(&alice, &did("did:web:bob.example"), &zone,
            &AuthTarget::from_canonical_key("system:control-panel").unwrap(), None).unwrap();
        // A target outside the whitelist still verifies; verify-hub refuses it.
        let verified = verify_signed_assertion(&resolver, &unknown_target, &zone, "target.example", now).await.unwrap();
        assert!(cross_zone_target_scopes(&verified.target).is_none());
        let unavailable = fixture().with_unavailable(&did("did:web:bob.example"));
        assert_eq!(code(verify_signed_assertion(&unavailable, &jwt, &zone, "target.example", now).await), CrossZoneLoginError::ZoneUnavailable);
    }

    #[tokio::test]
    async fn the_shared_custody_table_holds_here() {
        use super::custody_cases::*;
        let resolver = resolver();
        for case in cases() {
            let kid = format!("{}#main_key", case.device);
            let outcome = match resolve_signer_key(&resolver, &kid).await {
                Ok(signer) => match check_custody(&resolver, &signer, &did(case.principal), None).await {
                    Ok(_) => Outcome::Allowed,
                    Err(CustodyError::Unavailable(_)) => Outcome::Retry,
                    Err(_) => Outcome::Refused,
                },
                Err(CustodyError::Unavailable(_)) => Outcome::Retry,
                Err(_) => Outcome::Refused,
            };
            assert_eq!(outcome, case.outcome, "{} for {}", case.device, case.principal);
        }
    }

    #[test]
    fn login_error_codes_round_trip_on_the_wire() {
        for code in CrossZoneLoginError::ALL {
            assert_eq!(CrossZoneLoginError::from_rpc_error(&code.into_rpc_error("x")), Some(code));
        }
        assert_eq!(CrossZoneLoginError::from_rpc_error(&RPCErrors::ReasonError("assertion_invalid".into())), None);
    }
}
