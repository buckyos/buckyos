//! The domain of an activated node: `/kapi/node_daemon` for the settings page
//! (control_panel forwards, this node checks the caller again), the periodic
//! check, and the owner's notifications.

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ::kRPC::{kRPC, RPCErrors, RPCHandler, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use async_trait::async_trait;
use buckyos_api::{
    atomic_write, get_buckyos_api_runtime, validate_verify_hub_token_claims, AuthTarget,
    DomainCertificateSource, DomainHosting, DomainHostingMode, NotifySendReq, SystemConfigError,
    TokenPrincipalKind, TokenUse, CONTROL_PANEL_SERVICE_UNIQUE_ID, DOMAIN_HOSTING_KEY,
    START_CONFIG_DOMAIN_HOSTING,
};
use buckyos_http_server::{
    serve_http_by_rpc_handler_with_limit, server_err, HttpServer, ServerError, ServerErrorCode,
    ServerResult, StreamInfo,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use bytes::Bytes;
use cyfs_gateway_api::{SnClient, CONTROL_SERVER};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use log::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::active_server::parse_secret_params;

use super::check::{CheckItem, CheckReport, DomainBinding, ItemStatus};
use super::plan::{DomainSetup, NetId};
use super::remote::{HostnameBinding, SnChallenge};
use super::service::{
    certificate_hosts, credential_of, install_domain, remove_credential, remove_other_credentials,
    run_challenge, ApplyCredential, ApplyResponse, ApplyStatus, ChallengeWriter, CommitRequest,
    DomainService, Trial,
};
use super::{DomainError, DomainResult};

pub const NODE_DAEMON_RPC_PATH: &str = "/kapi/node_daemon";
const MAX_REQUEST_BODY: usize = 2 * 1024 * 1024;
const FIRST_ROUND_DELAY: Duration = Duration::from_secs(180);
const ROUND_INTERVAL: Duration = Duration::from_secs(6 * 3600);
/// While something is wrong.
const PROBLEM_ROUND_INTERVAL: Duration = Duration::from_secs(3600);
/// A first failed issuance is usually gone by the next attempt.
const RENEWAL_GRACE_SECS: u64 = 1800;
const EXPIRY_WARNING_SECS: u64 = 14 * 24 * 3600;
/// An attempt this close counts as under way.
const ORDER_LEAD_SECS: u64 = 30;
const CHECK_TRIAL_WAIT: Duration = Duration::from_secs(8);
const FULL_TRIAL_WAIT: Duration = Duration::from_secs(360);

/// What this node knows about itself.
#[derive(Clone, Debug)]
pub struct NodeFacts {
    pub hostname: String,
    pub net_id: String,
    pub public_ips: Vec<IpAddr>,
    pub owner_name: String,
    pub security_root: PathBuf,
    pub start_config: PathBuf,
    pub state_file: PathBuf,
}

/// The Zone around this node.
#[async_trait]
pub trait ZoneAccess: Send + Sync {
    async fn authorize(&self, token: Option<&str>, sudo: bool) -> Result<(), RPCErrors>;
    async fn hosting(&self) -> Result<Option<DomainHosting>, String>;
    async fn set_hosting(&self, hosting: &DomainHosting) -> Result<(), String>;
    async fn cert_status(&self) -> Result<Vec<Value>, String>;
    async fn notify_owner(&self, title: String, message: String, key: String)
        -> Result<(), String>;
    fn binding(&self) -> Box<dyn DomainBinding>;
    fn sn_challenge(&self) -> Result<Arc<dyn ChallengeWriter>, String>;
}

#[derive(Debug, Deserialize)]
struct CertStatus {
    host: String,
    #[serde(default)]
    loaded: bool,
    #[serde(default)]
    not_after: Option<u64>,
    #[serde(default)]
    acme: Option<AcmeStatus>,
}

#[derive(Debug, Deserialize)]
struct AcmeStatus {
    state: String,
    #[serde(default)]
    last_error: Option<Value>,
    #[serde(default)]
    next_retry_at: Option<u64>,
}

/// Since when a problem is there: one notification each time it begins.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
struct Episodes {
    #[serde(default)]
    credential_rejected: Option<(String, u64)>,
    #[serde(default)]
    renewal_failed: Option<u64>,
}

#[derive(Default)]
struct RuntimeState {
    check: Option<CheckReport>,
    episodes: Episodes,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyParams {}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectCertificateParams {
    files: Vec<String>,
    #[serde(default)]
    pfx_password: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PlanParams {
    mode: DomainHostingMode,
    certificate: DomainCertificateSource,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApplyParams {
    mode: DomainHostingMode,
    certificate: DomainCertificateSource,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    fields: Option<BTreeMap<String, String>>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    credential_id: Option<String>,
    confirm: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyParams {
    mode: DomainHostingMode,
    certificate: DomainCertificateSource,
    #[serde(default)]
    credential_id: Option<String>,
    #[serde(default)]
    certificate_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateParams {
    mode: DomainHostingMode,
    certificate: DomainCertificateSource,
    #[serde(default)]
    credential_id: Option<String>,
    #[serde(default)]
    certificate_id: Option<String>,
    accept_warnings: bool,
}

fn parse<T: DeserializeOwned>(params: Value) -> Result<T, RPCErrors> {
    parse_secret_params(params, "domain params")
}

fn domain_error(error: DomainError) -> RPCErrors {
    error.into_rpc_error("domain")
}

pub struct DomainRuntime {
    facts: NodeFacts,
    /// `None`: the Zone has no SN, so no custom domain of this kind either.
    service: Option<DomainService>,
    zone: Arc<dyn ZoneAccess>,
    state: Mutex<RuntimeState>,
    /// Updates and repairs do not interleave.
    writing: tokio::sync::Mutex<()>,
}

impl DomainRuntime {
    pub fn new(
        facts: NodeFacts,
        service: Option<DomainService>,
        zone: Arc<dyn ZoneAccess>,
    ) -> Self {
        let episodes = std::fs::read(&facts.state_file)
            .ok()
            .and_then(|content| serde_json::from_slice(&content).ok())
            .unwrap_or_default();
        Self {
            facts,
            service,
            zone,
            state: Mutex::new(RuntimeState {
                check: None,
                episodes,
            }),
            writing: tokio::sync::Mutex::new(()),
        }
    }

    pub fn spawn(self: &Arc<Self>) {
        let runtime = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(FIRST_ROUND_DELAY).await;
            loop {
                let next = match runtime.round().await {
                    Ok(true) => ROUND_INTERVAL,
                    Ok(false) => PROBLEM_ROUND_INTERVAL,
                    Err(error) => {
                        warn!("domain check failed: {error}");
                        PROBLEM_ROUND_INTERVAL
                    }
                };
                tokio::time::sleep(next).await;
            }
        });
    }

    fn service(&self) -> DomainResult<&DomainService> {
        self.service
            .as_ref()
            .ok_or_else(|| DomainError::new("no_sn", "this Zone has no SN"))
    }

    fn setup(
        &self,
        domain: &str,
        mode: DomainHostingMode,
        certificate: DomainCertificateSource,
    ) -> DomainResult<DomainSetup> {
        let net_id = NetId::parse(&self.facts.net_id).ok_or_else(|| {
            DomainError::new("invalid_setup", format!("net_id {}", self.facts.net_id))
        })?;
        Ok(DomainSetup {
            domain: domain.to_string(),
            mode,
            certificate,
            net_id,
            public_ips: if net_id == NetId::Wan {
                self.facts.public_ips.clone()
            } else {
                Vec::new()
            },
            owner_name: self.facts.owner_name.clone(),
        })
    }

    async fn hosting(&self) -> DomainResult<DomainHosting> {
        self.zone
            .hosting()
            .await
            .map_err(|error| DomainError::new("system_config_unavailable", error))?
            .ok_or_else(|| DomainError::new("default_hostname", self.facts.hostname.clone()))
    }

    async fn run_check(&self, hosting: &DomainHosting, trial: Trial) -> DomainResult<CheckReport> {
        let setup = self.setup(&hosting.domain, hosting.mode, hosting.certificate)?;
        let binding = self.zone.binding();
        let report = self
            .service()?
            .check(
                &setup,
                Some(binding.as_ref()),
                hosting.credential_id.as_deref(),
                None,
                trial,
            )
            .await?;
        self.state.lock().unwrap().check = Some(report.clone());
        Ok(report)
    }

    async fn certificates(&self, hostname: &str) -> Vec<Value> {
        let hosts = certificate_hosts(hostname);
        match self.zone.cert_status().await {
            Ok(all) => all
                .into_iter()
                .filter(|status| {
                    status["host"]
                        .as_str()
                        .is_some_and(|host| hosts.iter().any(|known| known == host))
                })
                .collect(),
            Err(error) => {
                debug!("certificate status not available: {error}");
                Vec::new()
            }
        }
    }

    async fn status(&self) -> DomainResult<Value> {
        let hosting = self
            .zone
            .hosting()
            .await
            .map_err(|error| DomainError::new("system_config_unavailable", error))?;
        let certificates = self.certificates(&self.facts.hostname).await;
        let check = self.state.lock().unwrap().check.clone();
        let (check, problems) = match &hosting {
            None => (None, BTreeSet::new()),
            Some(hosting) => (check, self.problems(hosting, &certificates)),
        };
        let credential = hosting
            .as_ref()
            .and_then(|hosting| {
                hosting
                    .credential_id
                    .as_ref()
                    .zip(hosting.provider.as_ref())
            })
            .map(|(id, provider)| json!({ "id": id, "provider": provider }));
        Ok(json!({
            "hosting": hosting,
            "hostname": self.facts.hostname,
            "net_id": self.facts.net_id,
            "certificates": certificates,
            "credential": credential,
            "check": check,
            "problems": problems,
        }))
    }

    fn problems(&self, hosting: &DomainHosting, certificates: &[Value]) -> BTreeSet<&'static str> {
        let mut problems = BTreeSet::new();
        if let Some(id) = hosting.credential_id.as_deref() {
            let missing = self
                .service
                .as_ref()
                .is_some_and(|service| service.credential(id).is_err());
            if missing {
                problems.insert("credential_missing");
            }
            let rejected = matches!(
                &self.state.lock().unwrap().episodes.credential_rejected,
                Some((rejected, _)) if rejected == id
            );
            if rejected {
                problems.insert("credential_rejected");
            }
        }
        if hosting.uses_acme() && renewal_failure(hosting, certificates).is_some() {
            problems.insert("renewal_failed");
        }
        problems
    }

    /// `Ok(true)`: nothing wrong.
    async fn round(&self) -> Result<bool, String> {
        let Some(hosting) = self.zone.hosting().await? else {
            return Ok(true);
        };
        let text = |error: DomainError| format!("[{}] {}", error.code, error.detail);
        let certificates = self.certificates(&hosting.domain).await;
        // Some services are written as a whole: not while the gateway writes.
        let busy = order_under_way(&hosting.domain, &certificates, buckyos_get_unix_timestamp());
        let repaired = if busy {
            Err(DomainError::new(
                "challenge_busy",
                "a certificate is being ordered",
            ))
        } else {
            let _writing = self.writing.lock().await;
            self.repair(&hosting).await
        };
        let rejected = match &repaired {
            Ok(Some(response)) => Some(
                response
                    .results
                    .iter()
                    .any(|result| result.code.as_deref() == Some("credential_rejected")),
            ),
            Ok(None) => Some(false),
            Err(error) => {
                warn!(
                    "repair DNS records of {} failed: {}",
                    hosting.domain,
                    text(error.clone())
                );
                None
            }
        };
        let challenge = Trial::Known(challenge_from_certificates(&hosting, &certificates));
        let report = self.run_check(&hosting, challenge).await.map_err(text)?;
        self.follow_episodes(&hosting, rejected, &certificates)
            .await;
        Ok(report.ready
            && rejected == Some(false)
            && self.problems(&hosting, &certificates).is_empty())
    }

    /// DNS API only: writes what is missing. A name that holds something else
    /// is the user's to decide.
    async fn repair(&self, hosting: &DomainHosting) -> DomainResult<Option<ApplyResponse>> {
        let (DomainHostingMode::DnsApi, Some(id)) = (hosting.mode, hosting.credential_id.clone())
        else {
            return Ok(None);
        };
        let setup = self.setup(&hosting.domain, hosting.mode, hosting.certificate)?;
        let response = self
            .service()?
            .apply(&setup, ApplyCredential::Known(id), &[])
            .await?;
        for result in &response.results {
            match result.status {
                ApplyStatus::Written => {
                    info!(
                        "DNS record {} {:?} was missing and is written again",
                        result.name, result.ty
                    )
                }
                ApplyStatus::Failed => warn!(
                    "DNS record {} {:?} cannot be written: {}",
                    result.name,
                    result.ty,
                    result.code.as_deref().unwrap_or_default()
                ),
                _ => {}
            }
        }
        Ok(Some(response))
    }

    async fn follow_episodes(
        &self,
        hosting: &DomainHosting,
        rejected: Option<bool>,
        certificates: &[Value],
    ) {
        let now = buckyos_get_unix_timestamp();
        let failure = renewal_failure(hosting, certificates);
        let episodes = {
            let mut state = self.state.lock().unwrap();
            let episodes = &mut state.episodes;
            let id = hosting.credential_id.clone().unwrap_or_default();
            match rejected {
                Some(true) => {
                    if !matches!(&episodes.credential_rejected, Some((known, _)) if *known == id) {
                        episodes.credential_rejected = Some((id, now));
                    }
                }
                Some(false) => episodes.credential_rejected = None,
                None => {}
            }
            episodes.renewal_failed = match failure {
                Some(_) => episodes.renewal_failed.or(Some(now)),
                None => None,
            };
            episodes.clone()
        };
        self.save_episodes(&episodes);
        let domain = &hosting.domain;
        let mut notices = Vec::new();
        if let Some((id, since)) = &episodes.credential_rejected {
            notices.push((
                format!("The DNS token of {domain} no longer works"),
                "The DNS service refused it, so the certificate cannot be renewed and the records cannot be repaired. Open Settings > Zone Config and enter a new token.".to_string(),
                format!("domain-credential-rejected:{id}:{since}"),
            ));
        }
        if let (Some(since), Some(failure)) = (episodes.renewal_failed, failure) {
            let expiring = failure
                .not_after
                .is_some_and(|not_after| not_after < now + EXPIRY_WARNING_SECS);
            if now >= since + RENEWAL_GRACE_SECS {
                notices.push((
                    format!("The certificate of {domain} could not be renewed"),
                    format!(
                        "{} Open Settings > Zone Config to see the cause.",
                        failure.message
                    ),
                    format!("domain-renewal-failed:{domain}:{since}"),
                ));
                if expiring {
                    let not_after = failure.not_after.unwrap_or_default();
                    notices.push((
                        format!("The certificate of {domain} is about to expire"),
                        format!(
                            "Renewing it still fails, and browsers stop opening the Zone when it expires. {} Open Settings > Zone Config to see the cause.",
                            failure.message
                        ),
                        format!("domain-certificate-expiring:{domain}:{not_after}"),
                    ));
                }
            }
        }
        for (title, message, key) in notices {
            if let Err(error) = self.zone.notify_owner(title, message, key).await {
                warn!("owner not told about the domain: {error}");
            }
        }
    }

    fn save_episodes(&self, episodes: &Episodes) {
        let Ok(content) = serde_json::to_vec(episodes) else {
            return;
        };
        if let Err(error) = atomic_write(&self.facts.state_file, &content) {
            warn!("save domain state failed: {error}");
        }
    }

    async fn apply(&self, params: ApplyParams) -> DomainResult<ApplyResponse> {
        let hosting = self.hosting().await?;
        let setup = self.setup(&hosting.domain, params.mode, params.certificate)?;
        let credential = match (params.provider, params.fields, params.credential_id) {
            (Some(provider), Some(fields), None) => {
                ApplyCredential::New(credential_of(provider, fields, params.endpoint))
            }
            (None, None, Some(id)) if params.endpoint.is_none() => ApplyCredential::Known(id),
            _ => {
                return Err(DomainError::new(
                    "invalid_setup",
                    "apply needs either provider and fields or credential_id",
                ))
            }
        };
        let _writing = self.writing.lock().await;
        self.service()?
            .apply(&setup, credential, &params.confirm)
            .await
    }

    async fn verify(&self, params: VerifyParams) -> DomainResult<CheckReport> {
        let hosting = self.hosting().await?;
        let setup = self.setup(&hosting.domain, params.mode, params.certificate)?;
        let credential_id = credential_in_use(&hosting, params.mode, params.credential_id);
        let binding = self.zone.binding();
        let trial = self.trial(&hosting, CHECK_TRIAL_WAIT).await;
        self.service()?
            .check(
                &setup,
                Some(binding.as_ref()),
                credential_id.as_deref(),
                params.certificate_id.as_deref(),
                trial,
            )
            .await
    }

    /// The gateway writes at the same name for its orders, in another process.
    async fn trial(&self, hosting: &DomainHosting, wait: Duration) -> Trial {
        let certificates = self.certificates(&hosting.domain).await;
        let now = buckyos_get_unix_timestamp();
        if order_under_way(&hosting.domain, &certificates, now) {
            Trial::Busy
        } else {
            Trial::Run(wait)
        }
    }

    async fn update(&self, params: UpdateParams) -> DomainResult<()> {
        let _writing = self.writing.lock().await;
        let current = self.hosting().await?;
        let service = self.service()?;
        let setup = self.setup(&current.domain, params.mode, params.certificate)?;
        let new_credential = params
            .credential_id
            .as_ref()
            .is_some_and(|id| current.credential_id.as_ref() != Some(id));
        let credential_id = credential_in_use(&current, params.mode, params.credential_id);
        let request = CommitRequest {
            setup: &setup,
            credential_id: credential_id.as_deref(),
            certificate_id: params.certificate_id.as_deref(),
            accept_warnings: params.accept_warnings,
        };
        let issuer = current.acme_issuer.clone();
        let refresh = current.mode == params.mode && current.certificate == params.certificate;
        let commit = if refresh {
            service
                .prepare_refresh(&request, new_credential, issuer)
                .await?
        } else {
            let binding = self.zone.binding();
            let trial = self.trial(&current, FULL_TRIAL_WAIT).await;
            service
                .prepare_commit(&request, Some(binding.as_ref()), issuer, trial)
                .await?
        };
        let starts_sn_acme = commit.hosting.mode == DomainHostingMode::Sn
            && commit.hosting.uses_acme()
            && !(current.mode == DomainHostingMode::Sn && current.uses_acme());
        if starts_sn_acme {
            let writer = self
                .zone
                .sn_challenge()
                .map_err(|error| DomainError::new("sn_challenge_failed", error))?;
            let trial = tokio::spawn(run_challenge(
                service.lookup.clone(),
                writer,
                current.domain.clone(),
                vec![service.names.name_server()],
                service.timing(),
            ));
            let failure = match trial.await {
                Ok(outcome) => outcome.failure.map(|f| format!("{}: {}", f.code, f.detail)),
                Err(error) => Some(error.to_string()),
            };
            if let Some(failure) = failure {
                return Err(DomainError::new("sn_challenge_failed", failure));
            }
        }

        let install_failed = |error: String| DomainError::new("install_failed", error);
        install_domain(&self.facts.security_root, service.certificates(), &commit)
            .map_err(install_failed)?;
        let saved = self.save_hosting(&commit.hosting).await;
        if let Err(error) = saved {
            let new_id = commit.hosting.credential_id.as_deref();
            if let Some(id) = new_id.filter(|id| current.credential_id.as_deref() != Some(*id)) {
                remove_credential(&self.facts.security_root, id);
            }
            return Err(install_failed(error));
        }
        remove_other_credentials(
            &self.facts.security_root,
            commit.hosting.credential_id.as_deref(),
        );
        let episodes = {
            let mut state = self.state.lock().unwrap();
            state.episodes = Episodes::default();
            state.check = None;
            state.episodes.clone()
        };
        self.save_episodes(&episodes);
        info!(
            "domain {} is now hosted by {:?} with {:?} certificates",
            commit.hosting.domain, commit.hosting.mode, commit.hosting.certificate
        );
        Ok(())
    }

    /// `start_config.json` follows: a Zone that boots from it again starts
    /// with the same hosting.
    async fn save_hosting(&self, hosting: &DomainHosting) -> Result<(), String> {
        let previous = write_start_config_hosting(&self.facts.start_config, hosting)?;
        if let Err(error) = self.zone.set_hosting(hosting).await {
            if let Some(previous) = previous {
                if let Err(restore) = replace_file(&self.facts.start_config, &previous) {
                    warn!(
                        "restore {} failed: {restore}",
                        self.facts.start_config.display()
                    );
                }
            }
            return Err(error);
        }
        Ok(())
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, RPCErrors> {
        fn value<T: Serialize>(result: DomainResult<T>) -> Result<Value, RPCErrors> {
            let result = result.map_err(domain_error)?;
            serde_json::to_value(result).map_err(|error| RPCErrors::ReasonError(error.to_string()))
        }
        match method {
            "domain.status" => {
                parse::<EmptyParams>(params)?;
                value(self.status().await)
            }
            "domain.check" => {
                parse::<EmptyParams>(params)?;
                let hosting = self.hosting().await.map_err(domain_error)?;
                let trial = self.trial(&hosting, CHECK_TRIAL_WAIT).await;
                value(self.run_check(&hosting, trial).await)
            }
            "domain.providers" => {
                parse::<EmptyParams>(params)?;
                self.hosting().await.map_err(domain_error)?;
                Ok(json!({ "providers": DomainService::list_providers() }))
            }
            "domain.inspect_certificate" => {
                let params: InspectCertificateParams = parse(params)?;
                let hosting = self.hosting().await.map_err(domain_error)?;
                let service = self.service().map_err(domain_error)?;
                value(
                    service
                        .inspect_certificate(&hosting.domain, &params.files, params.pfx_password)
                        .await,
                )
            }
            "domain.plan" => {
                let params: PlanParams = parse(params)?;
                let hosting = self.hosting().await.map_err(domain_error)?;
                let setup = self
                    .setup(&hosting.domain, params.mode, params.certificate)
                    .map_err(domain_error)?;
                value(self.service().map_err(domain_error)?.plan(&setup).await)
            }
            "domain.apply" => value(self.apply(parse(params)?).await),
            "domain.verify" => value(self.verify(parse(params)?).await),
            "domain.update" => {
                self.update(parse(params)?).await.map_err(domain_error)?;
                value(self.status().await)
            }
            _ => Err(RPCErrors::UnknownMethod(method.to_string())),
        }
    }
}

/// The stored token stays in use while the mode stays DNS API.
fn credential_in_use(
    current: &DomainHosting,
    mode: DomainHostingMode,
    given: Option<String>,
) -> Option<String> {
    given.or_else(|| {
        (mode == DomainHostingMode::DnsApi)
            .then(|| current.credential_id.clone())
            .flatten()
    })
}

fn cert_statuses(domain: &str, certificates: &[Value]) -> Vec<CertStatus> {
    let hosts = certificate_hosts(domain);
    certificates
        .iter()
        .filter_map(|status| serde_json::from_value::<CertStatus>(status.clone()).ok())
        .filter(|status| hosts.contains(&status.host))
        .collect()
}

/// The gateway is working on an order, so it may be writing at the challenge
/// name. Between two attempts it is not.
fn order_under_way(domain: &str, certificates: &[Value], now: u64) -> bool {
    cert_statuses(domain, certificates).iter().any(|status| {
        status.acme.as_ref().is_some_and(|acme| {
            matches!(acme.state.as_str(), "ordering" | "renewing")
                && acme
                    .next_retry_at
                    .is_none_or(|at| at <= now + ORDER_LEAD_SECS)
        })
    })
}

/// What the certificates tell about the challenge, without writing one.
fn challenge_from_certificates(hosting: &DomainHosting, certificates: &[Value]) -> CheckItem {
    const ID: &str = "challenge";
    if let Some(failure) = renewal_failure(hosting, certificates) {
        return CheckItem::fail(ID, "renewal_failed").detail(failure.message);
    }
    let statuses = cert_statuses(&hosting.domain, certificates);
    if order_under_way(&hosting.domain, certificates, buckyos_get_unix_timestamp()) {
        return CheckItem::waiting(ID, "challenge_busy");
    }
    if statuses.iter().any(|status| status.loaded) {
        return CheckItem::pass(ID);
    }
    CheckItem::new(ID, ItemStatus::Skipped, None)
}

struct RenewalFailure {
    message: String,
    not_after: Option<u64>,
}

/// An uploaded certificate is never a failure (Q13).
fn renewal_failure(hosting: &DomainHosting, certificates: &[Value]) -> Option<RenewalFailure> {
    if !hosting.uses_acme() {
        return None;
    }
    let failing = cert_statuses(&hosting.domain, certificates)
        .into_iter()
        .filter_map(|status| {
            let acme = status.acme.filter(|acme| acme.state != "manual")?;
            let error = acme.last_error?;
            let message = error["message"].as_str().unwrap_or_default().to_string();
            Some((status.host, message, status.not_after))
        })
        .collect::<Vec<_>>();
    let (host, message, _) = failing.first()?;
    Some(RenewalFailure {
        message: format!("{host}: {}.", message.trim_end_matches('.')),
        not_after: failing
            .iter()
            .filter_map(|(_, _, not_after)| *not_after)
            .min(),
    })
}

/// Returns the previous content. Nothing to do on a node without the file.
fn write_start_config_hosting(
    path: &Path,
    hosting: &DomainHosting,
) -> Result<Option<Vec<u8>>, String> {
    let previous = match std::fs::read(path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("read {} failed: {error}", path.display())),
    };
    let mut config: Value = serde_json::from_slice(&previous)
        .map_err(|error| format!("parse {} failed: {error}", path.display()))?;
    config[START_CONFIG_DOMAIN_HOSTING] = json!(hosting);
    let content = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    replace_file(path, &content)?;
    Ok(Some(previous))
}

/// Atomic; owner and permissions of the replaced file stay.
fn replace_file(path: &Path, content: &[u8]) -> Result<(), String> {
    let failed = |error: std::io::Error| format!("replace {} failed: {error}", path.display());
    let metadata = std::fs::metadata(path).map_err(failed)?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    let written = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        std::io::Write::write_all(&mut file, content)?;
        file.sync_all()?;
        std::fs::set_permissions(&temp, metadata.permissions())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let own = std::fs::metadata(&temp)?;
            if (own.uid(), own.gid()) != (metadata.uid(), metadata.gid()) {
                std::os::unix::fs::chown(&temp, Some(metadata.uid()), Some(metadata.gid()))?;
            }
        }
        std::fs::rename(&temp, path)
    })();
    written.map_err(|error| {
        let _ = std::fs::remove_file(&temp);
        failed(error)
    })
}

/// A verify-hub session of the Zone owner on the control panel; changes also
/// need sudo.
pub fn check_caller(
    token: &RPCSessionToken,
    zone_owner: &str,
    sudo: bool,
) -> Result<(), RPCErrors> {
    let claims = validate_verify_hub_token_claims(token, TokenUse::Session)?;
    let from_control_panel = claims.principal_kind == TokenPrincipalKind::User
        && matches!(
            &claims.target,
            AuthTarget::System { service_id } if service_id.as_str() == CONTROL_PANEL_SERVICE_UNIQUE_ID
        );
    if !from_control_panel {
        return Err(RPCErrors::NoPermission(
            "the Zone domain needs a user's Control Panel session".to_string(),
        ));
    }
    if token.sub.as_deref().map(str::trim) != Some(zone_owner) {
        return Err(RPCErrors::NoPermission(
            "only the Zone owner manages the Zone domain".to_string(),
        ));
    }
    if sudo && !token.sudo {
        return Err(RPCErrors::NoPermission(
            "sudo permission is required to manage the Zone domain".to_string(),
        ));
    }
    Ok(())
}

fn needs_sudo(method: &str) -> bool {
    !matches!(method, "domain.status" | "domain.providers")
}

#[async_trait]
impl RPCHandler for DomainRuntime {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        if !ip_from.to_canonical().is_loopback() {
            return Err(RPCErrors::NoPermission(
                "this service is for the local node".to_string(),
            ));
        }
        self.zone
            .authorize(req.token.as_deref(), needs_sudo(&req.method))
            .await?;
        let result = self.call(&req.method, req.params).await?;
        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }
}

#[async_trait]
impl HttpServer for DomainRuntime {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler_with_limit(req, info, self, MAX_REQUEST_BODY).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        "node-daemon-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

/// [`ZoneAccess`] through the node's BuckyOS runtime.
pub struct RuntimeZone {
    pub owner_name: String,
    pub sn_url: Option<String>,
    /// Key DID and a fresh device token for the SN.
    pub sn_device: Arc<dyn Fn() -> Result<(String, String), String> + Send + Sync>,
}

const GATEWAY_RPC_TIMEOUT: Duration = Duration::from_secs(5);

impl RuntimeZone {
    fn sn_client(&self, token: Option<String>) -> Result<Arc<SnClient>, String> {
        let url = self.sn_url.as_deref().ok_or("this Zone has no SN")?;
        Ok(Arc::new(SnClient::new_krpc(url, token)))
    }
}

struct NoBinding;

#[async_trait]
impl DomainBinding for NoBinding {
    async fn bound(&self, _domain: &str) -> Result<bool, RPCErrors> {
        Err(RPCErrors::ReasonError("this Zone has no SN".to_string()))
    }

    async fn bind(&self, _domain: &str) -> Result<(), RPCErrors> {
        Err(RPCErrors::ReasonError("this Zone has no SN".to_string()))
    }
}

#[async_trait]
impl ZoneAccess for RuntimeZone {
    async fn authorize(&self, token: Option<&str>, sudo: bool) -> Result<(), RPCErrors> {
        let token = token
            .ok_or_else(|| RPCErrors::InvalidToken("missing caller session token".to_string()))?;
        let runtime = get_buckyos_api_runtime()?;
        let verified = runtime.verify_trusted_session_token(token).await?;
        let owner = runtime
            .get_system_config_client()
            .await?
            .get_zone_owner_user_id()
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        check_caller(&verified, owner.as_str(), sudo)
    }

    async fn hosting(&self) -> Result<Option<DomainHosting>, String> {
        let client = get_buckyos_api_runtime()
            .map_err(|error| error.to_string())?
            .get_system_config_client()
            .await
            .map_err(|error| error.to_string())?;
        match client.get(DOMAIN_HOSTING_KEY).await {
            Ok(value) => DomainHosting::parse(&value.value)
                .map(Some)
                .map_err(|error| format!("invalid {DOMAIN_HOSTING_KEY}: {error}")),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn set_hosting(&self, hosting: &DomainHosting) -> Result<(), String> {
        let content = serde_json::to_string(hosting).map_err(|error| error.to_string())?;
        get_buckyos_api_runtime()
            .map_err(|error| error.to_string())?
            .get_system_config_client()
            .await
            .map_err(|error| error.to_string())?
            .set(DOMAIN_HOSTING_KEY, &content)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    async fn cert_status(&self) -> Result<Vec<Value>, String> {
        let token = crate::gateway_tunnel_probe::build_local_gateway_token()
            .ok_or("no gateway control token")?;
        let client = kRPC::new(CONTROL_SERVER, Some(token));
        let result = tokio::time::timeout(
            GATEWAY_RPC_TIMEOUT,
            client.call("get_cert_status", json!({})),
        )
        .await
        .map_err(|_| "get_cert_status timed out".to_string())?
        .map_err(|error| error.to_string())?;
        result["hosts"]
            .as_array()
            .cloned()
            .ok_or_else(|| "get_cert_status returned no hosts".to_string())
    }

    async fn notify_owner(
        &self,
        title: String,
        message: String,
        key: String,
    ) -> Result<(), String> {
        let runtime = get_buckyos_api_runtime().map_err(|error| error.to_string())?;
        let owner = runtime
            .get_system_config_client()
            .await
            .map_err(|error| error.to_string())?
            .get_zone_owner_user_id()
            .await
            .map_err(|error| error.to_string())?;
        runtime
            .get_msg_center_client()
            .await
            .map_err(|error| error.to_string())?
            .notify_send(NotifySendReq::notice(owner, title, message, key))
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn binding(&self) -> Box<dyn DomainBinding> {
        match self.sn_client(None) {
            Ok(sn) => Box::new(HostnameBinding {
                sn,
                owner_name: self.owner_name.clone(),
            }),
            Err(_) => Box::new(NoBinding),
        }
    }

    fn sn_challenge(&self) -> Result<Arc<dyn ChallengeWriter>, String> {
        let (device_did, token) = (self.sn_device)()?;
        Ok(Arc::new(SnChallenge {
            sn: self.sn_client(Some(token))?,
            device_did,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::super::testing::*;
    use super::*;
    use buckyos_api::{
        bind_token_principal_kind, SystemServiceId, TOKEN_TARGET_KIND_CLAIM, TOKEN_USE_CLAIM,
    };
    use cyfs_dns_provider::RecordType;
    use cyfs_identity_cert::CertManager;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    const DOMAIN: &str = "home.example.com";

    #[derive(Default)]
    struct FakeZone {
        hosting: Mutex<Option<DomainHosting>>,
        certs: Mutex<Vec<Value>>,
        notices: Mutex<BTreeMap<String, String>>,
        read_only: AtomicBool,
        authorized: Mutex<Vec<bool>>,
    }

    #[async_trait]
    impl ZoneAccess for FakeZone {
        async fn authorize(&self, token: Option<&str>, sudo: bool) -> Result<(), RPCErrors> {
            self.authorized.lock().unwrap().push(sudo);
            token
                .map(|_| ())
                .ok_or_else(|| RPCErrors::InvalidToken("missing".to_string()))
        }

        async fn hosting(&self) -> Result<Option<DomainHosting>, String> {
            Ok(self.hosting.lock().unwrap().clone())
        }

        async fn set_hosting(&self, hosting: &DomainHosting) -> Result<(), String> {
            if self.read_only.load(Ordering::Relaxed) {
                return Err("no permission".to_string());
            }
            *self.hosting.lock().unwrap() = Some(hosting.clone());
            Ok(())
        }

        async fn cert_status(&self) -> Result<Vec<Value>, String> {
            Ok(self.certs.lock().unwrap().clone())
        }

        async fn notify_owner(
            &self,
            title: String,
            _message: String,
            key: String,
        ) -> Result<(), String> {
            self.notices.lock().unwrap().insert(key, title);
            Ok(())
        }

        fn binding(&self) -> Box<dyn DomainBinding> {
            let binding = FakeBinding::default();
            *binding.bound.lock().unwrap() = Some(DOMAIN.to_string());
            Box::new(binding)
        }

        fn sn_challenge(&self) -> Result<Arc<dyn ChallengeWriter>, String> {
            Err("no device".to_string())
        }
    }

    struct Node {
        _root: tempfile::TempDir,
        dns: Arc<FakeDns>,
        provider: Arc<FakeProvider>,
        certificates: Arc<FakeCertificates>,
        zone: Arc<FakeZone>,
        runtime: DomainRuntime,
        facts: NodeFacts,
    }

    impl Node {
        fn start_config(&self) -> Value {
            serde_json::from_slice(&std::fs::read(&self.facts.start_config).unwrap()).unwrap()
        }

        fn credentials(&self) -> Vec<String> {
            let mut names = std::fs::read_dir(self.facts.security_root.join("dns_credentials"))
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|entry| entry.file_name().to_string_lossy().to_string())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            names.sort();
            names
        }

        fn notices(&self) -> Vec<String> {
            self.zone
                .notices
                .lock()
                .unwrap()
                .values()
                .cloned()
                .collect()
        }

        async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
            self.runtime
                .call(method, params)
                .await
                .map_err(|error| error.to_string())
        }
    }

    fn node(hosting: Option<Value>) -> Node {
        let root = tempfile::tempdir().unwrap();
        let facts = NodeFacts {
            hostname: DOMAIN.to_string(),
            net_id: "nat".to_string(),
            public_ips: vec!["192.168.1.2".parse().unwrap()],
            owner_name: "alice".to_string(),
            security_root: root.path().join("security"),
            start_config: root.path().join("security/start_config.json"),
            state_file: root.path().join("local/domain_state.json"),
        };
        std::fs::create_dir_all(&facts.security_root).unwrap();
        std::fs::write(
            &facts.start_config,
            json!({"schema": "x", "domain_hosting": hosting}).to_string(),
        )
        .unwrap();
        let world = world();
        let zone = Arc::new(FakeZone::default());
        *zone.hosting.lock().unwrap() =
            hosting.map(|hosting| serde_json::from_value(hosting).unwrap());
        let service = world.service.with_stored(facts.security_root.clone());
        Node {
            runtime: DomainRuntime::new(facts.clone(), Some(service), zone.clone()),
            _root: root,
            dns: world.dns,
            provider: world.provider,
            certificates: world.certificates,
            zone,
            facts,
        }
    }

    fn dns_api_node() -> Node {
        let node = node(Some(json!({
            "schema_version": 1, "mode": "dns_api", "certificate": "acme", "domain": DOMAIN,
            "provider": "cloudflare", "credential_id": "old-1",
            "acme_issuer": "https://acme-staging-v02.api.letsencrypt.org/directory"
        })));
        let credential = credential_of(
            "cloudflare".to_string(),
            BTreeMap::from([("api_token".to_string(), TEST_TOKEN.to_string())]),
            None,
        );
        let path = cyfs_dns_provider::credential_path(&node.facts.security_root, "old-1").unwrap();
        cyfs_dns_provider::save_credential(&path, &credential).unwrap();
        for (name, ty, value) in [
            (format!("_pkx.{DOMAIN}"), RecordType::TXT, PKX),
            (DOMAIN.to_string(), RecordType::CNAME, ENTRY),
            (format!("*.{DOMAIN}"), RecordType::CNAME, ENTRY),
        ] {
            node.dns.add("example.com", &name, ty, value);
        }
        node
    }

    fn cert(host: &str, error: Option<&str>, not_after: u64) -> Value {
        json!({
            "host": host, "loaded": true, "not_after": not_after, "issuer": "x",
            "fingerprint": "y", "manager": "acme",
            "acme": { "state": if error.is_some() { "renewing" } else { "ready" },
                      "last_error": error.map(|message| json!({"message": message, "at": 1})),
                      "next_retry_at": null }
        })
    }

    #[tokio::test]
    async fn a_default_hostname_has_a_status_and_nothing_else() {
        let node = node(None);
        *node.zone.certs.lock().unwrap() =
            vec![cert(DOMAIN, None, 10), cert("other.example", None, 10)];
        let status = node.call("domain.status", json!({})).await.unwrap();
        assert_eq!(status["hosting"], Value::Null);
        assert_eq!(status["hostname"], DOMAIN);
        assert_eq!(status["certificates"].as_array().unwrap().len(), 1);
        assert_eq!(status["credential"], Value::Null);
        assert_eq!(status["check"], Value::Null);
        assert_eq!(status["problems"], json!([]));
        for (method, params) in [
            ("domain.check", json!({})),
            ("domain.providers", json!({})),
            ("domain.inspect_certificate", json!({"files": []})),
            ("domain.plan", json!({"mode": "sn", "certificate": "acme"})),
            (
                "domain.apply",
                json!({"mode": "dns_api", "certificate": "acme", "credential_id": "x", "confirm": []}),
            ),
            (
                "domain.verify",
                json!({"mode": "sn", "certificate": "acme"}),
            ),
            (
                "domain.update",
                json!({"mode": "sn", "certificate": "acme", "accept_warnings": false}),
            ),
        ] {
            let error = node.call(method, params).await.unwrap_err();
            assert!(
                error.contains("[domain:default_hostname]"),
                "{method} {error}"
            );
        }
        assert!(node.call("domain.status", json!({"x": 1})).await.is_err());
        assert!(node.call("domain.unknown", json!({})).await.is_err());
        assert!(node.runtime.round().await.unwrap());
    }

    #[tokio::test]
    async fn the_page_can_check_and_read_the_status() {
        let node = dns_api_node();
        let check = node.call("domain.check", json!({})).await.unwrap();
        assert_eq!(check["ready"], true, "{check:#}");
        let status = node.call("domain.status", json!({})).await.unwrap();
        assert_eq!(status["check"], check);
        assert_eq!(
            status["credential"],
            json!({"id": "old-1", "provider": "cloudflare"})
        );
        assert_eq!(status["net_id"], "nat");
        assert!(!status.to_string().contains(TEST_TOKEN));
        let providers = node.call("domain.providers", json!({})).await.unwrap();
        assert!(!providers["providers"].as_array().unwrap().is_empty());
        let plan = node
            .call(
                "domain.plan",
                json!({"mode": "dns_api", "certificate": "acme"}),
            )
            .await
            .unwrap();
        assert_eq!(plan["records"].as_array().unwrap().len(), 3);
        assert_eq!(plan["conflicts"], json!([]));
    }

    #[tokio::test]
    async fn a_new_token_replaces_the_old_one() {
        let node = dns_api_node();
        let applied = node
            .call(
                "domain.apply",
                json!({"mode": "dns_api", "certificate": "acme", "provider": "cloudflare",
                       "fields": {"api_token": "another-secret-token"}, "confirm": []}),
            )
            .await
            .unwrap();
        let id = applied["credential_id"].as_str().unwrap().to_string();
        let update = json!({"mode": "dns_api", "certificate": "acme",
                            "credential_id": id, "accept_warnings": false});

        node.zone.read_only.store(true, Ordering::Relaxed);
        let error = node
            .call("domain.update", update.clone())
            .await
            .unwrap_err();
        assert!(error.contains("[domain:install_failed]"), "{error}");
        assert_eq!(node.credentials(), vec!["old-1.json"]);
        assert_eq!(
            node.start_config()["domain_hosting"]["credential_id"],
            "old-1"
        );
        node.zone.read_only.store(false, Ordering::Relaxed);

        let status = node.call("domain.update", update).await.unwrap();
        assert_eq!(status["credential"]["id"], id);
        assert_eq!(node.credentials(), vec![format!("{id}.json")]);
        assert_eq!(node.start_config()["domain_hosting"]["credential_id"], id);
        assert_eq!(
            node.start_config()["domain_hosting"]["acme_issuer"],
            "https://acme-staging-v02.api.letsencrypt.org/directory"
        );
        assert!(!node
            .start_config()
            .to_string()
            .contains("another-secret-token"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_start_config_keeps_its_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let node = dns_api_node();
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        for expected in [0o600, 0o640] {
            std::fs::set_permissions(
                &node.facts.start_config,
                std::fs::Permissions::from_mode(expected),
            )
            .unwrap();
            let hosting = node.zone.hosting().await.unwrap().unwrap();
            write_start_config_hosting(&node.facts.start_config, &hosting).unwrap();
            assert_eq!(mode(&node.facts.start_config), expected);
        }
        assert_eq!(node.start_config()["schema"], "x");
        assert_eq!(
            write_start_config_hosting(
                &node.facts.security_root.join("none.json"),
                &node.zone.hosting().await.unwrap().unwrap()
            )
            .unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn certificate_sources_are_switched_by_the_node() {
        let node = dns_api_node();
        let mut params =
            json!({"mode": "dns_api", "certificate": "upload", "accept_warnings": false});
        // The issued certificate is no uploaded one.
        node.certificates.put(DOMAIN, CertManager::Acme, u64::MAX);
        node.certificates
            .put(&format!("*.{DOMAIN}"), CertManager::Acme, u64::MAX);
        let error = node
            .call("domain.update", params.clone())
            .await
            .unwrap_err();
        assert!(error.contains("certificate_missing"), "{error}");

        let inspected = node
            .call("domain.inspect_certificate", json!({"files": ["cGVt"]}))
            .await
            .unwrap();
        params["certificate_id"] = inspected["certificate_id"].clone();
        let status = node.call("domain.update", params.clone()).await.unwrap();
        assert_eq!(status["hosting"]["certificate"], "upload");
        assert_eq!(node.certificates.manager(DOMAIN), Some(CertManager::Manual));
        // The token stays: it keeps the records.
        assert_eq!(status["credential"]["id"], "old-1");
        assert_eq!(node.credentials(), vec!["old-1.json"]);

        // Same mode without a new certificate keeps the installed one.
        params.as_object_mut().unwrap().remove("certificate_id");
        node.call("domain.update", params).await.unwrap();

        let status = node
            .call(
                "domain.update",
                json!({"mode": "dns_api", "certificate": "acme", "accept_warnings": false}),
            )
            .await
            .unwrap();
        assert_eq!(status["hosting"]["certificate"], "acme");
        assert_eq!(node.certificates.manager(DOMAIN), Some(CertManager::Acme));
    }

    async fn new_token(node: &Node, certificate: &str, token: &str) -> String {
        node.call(
            "domain.apply",
            json!({"mode": "dns_api", "certificate": certificate, "provider": "cloudflare",
                   "fields": {"api_token": token}, "confirm": []}),
        )
        .await
        .unwrap()["credential_id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[tokio::test]
    async fn an_unrelated_finding_does_not_keep_a_dead_token_or_certificate() {
        let node = dns_api_node();
        node.dns.add(
            "example.com",
            &format!("ood1.{DOMAIN}"),
            RecordType::A,
            "192.0.2.1",
        );
        let id = new_token(&node, "acme", "another-secret-token").await;
        let update = json!({"mode": "dns_api", "certificate": "acme",
                            "credential_id": id, "accept_warnings": false});

        node.provider.rejected.store(true, Ordering::Relaxed);
        let error = node
            .call("domain.update", update.clone())
            .await
            .unwrap_err();
        assert!(error.contains("[domain:credential_rejected]"), "{error}");
        assert_eq!(node.credentials(), vec!["old-1.json"]);
        node.provider.rejected.store(false, Ordering::Relaxed);
        tokio::time::sleep(Duration::from_millis(20)).await;

        // A source that changes is still checked in full.
        let inspected = node
            .call("domain.inspect_certificate", json!({"files": ["cGVt"]}))
            .await
            .unwrap();
        let switch = json!({"mode": "dns_api", "certificate": "upload",
                            "certificate_id": inspected["certificate_id"], "accept_warnings": false});
        let error = node
            .call("domain.update", switch.clone())
            .await
            .unwrap_err();
        assert!(error.contains("stale_record"), "{error}");

        let other = new_token(&node, "acme", "a-third-secret-token").await;
        let status = node
            .call(
                "domain.update",
                json!({"mode": "dns_api", "certificate": "acme",
                       "credential_id": other, "accept_warnings": false}),
            )
            .await
            .unwrap();
        assert_eq!(status["credential"]["id"], other);
        assert_eq!(node.credentials(), vec![format!("{other}.json")]);
    }

    #[tokio::test]
    async fn an_uploaded_certificate_is_replaced_whatever_the_records_say() {
        use cyfs_identity_cert::CheckLevel;
        let node = node(Some(
            json!({"schema_version": 1, "mode": "manual", "certificate": "upload", "domain": DOMAIN}),
        ));
        let inspected = node
            .call("domain.inspect_certificate", json!({"files": ["cGVt"]}))
            .await
            .unwrap();
        let mut update = json!({"mode": "manual", "certificate": "upload",
                                "certificate_id": inspected["certificate_id"], "accept_warnings": false});
        *node.certificates.inspection.lock().unwrap() = Some(inspection(DOMAIN, CheckLevel::Warn));
        let error = node
            .call("domain.update", update.clone())
            .await
            .unwrap_err();
        assert!(
            error.contains("[domain:certificate_warning_unconfirmed]"),
            "{error}"
        );
        assert_eq!(node.certificates.manager(DOMAIN), None);
        update["accept_warnings"] = json!(true);
        node.call("domain.update", update).await.unwrap();
        assert_eq!(node.certificates.manager(DOMAIN), Some(CertManager::Manual));
    }

    #[tokio::test]
    async fn a_token_that_only_keeps_records_is_tried_on_the_proof_record() {
        let node = node(Some(json!({
            "schema_version": 1, "mode": "dns_api", "certificate": "upload", "domain": DOMAIN,
            "provider": "cloudflare", "credential_id": "old-1"
        })));
        let id = new_token(&node, "upload", "another-secret-token").await;
        let update = json!({"mode": "dns_api", "certificate": "upload",
                            "credential_id": id, "accept_warnings": false});
        node.provider.rejected.store(true, Ordering::Relaxed);
        let error = node
            .call("domain.update", update.clone())
            .await
            .unwrap_err();
        assert!(error.contains("[domain:credential_rejected]"), "{error}");
        node.provider.rejected.store(false, Ordering::Relaxed);
        let status = node.call("domain.update", update).await.unwrap();
        assert_eq!(status["credential"]["id"], id);
        let challenges = node
            .provider
            .writes
            .lock()
            .unwrap()
            .iter()
            .filter(|write| write.contains("_acme-challenge"))
            .count();
        assert_eq!(challenges, 0);
    }

    #[tokio::test]
    async fn leaving_dns_api_removes_the_token() {
        let node = dns_api_node();
        let inspected = node
            .call("domain.inspect_certificate", json!({"files": ["cGVt"]}))
            .await
            .unwrap();
        let status = node
            .call(
                "domain.update",
                json!({"mode": "manual", "certificate": "upload",
                       "certificate_id": inspected["certificate_id"], "accept_warnings": false}),
            )
            .await
            .unwrap();
        assert_eq!(status["credential"], Value::Null);
        assert!(node.credentials().is_empty());
        assert!(node.start_config()["domain_hosting"]
            .get("credential_id")
            .is_none());
    }

    #[tokio::test]
    async fn an_update_that_is_not_ready_changes_nothing() {
        let node = dns_api_node();
        let verify = node
            .call(
                "domain.verify",
                json!({"mode": "sn", "certificate": "acme"}),
            )
            .await
            .unwrap();
        assert_eq!(verify["ready"], false);
        let error = node
            .call(
                "domain.update",
                json!({"mode": "sn", "certificate": "acme", "accept_warnings": false}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("[domain:domain_not_ready]"), "{error}");
        assert_eq!(node.credentials(), vec!["old-1.json"]);
        assert_eq!(node.start_config()["domain_hosting"]["mode"], "dns_api");
    }

    #[tokio::test]
    async fn missing_records_are_written_again_and_changed_ones_are_not() {
        let node = dns_api_node();
        let wildcard = format!("*.{DOMAIN}");
        node.dns
            .remove("example.com", &wildcard, RecordType::CNAME, None);
        assert!(node.runtime.round().await.unwrap());
        assert_eq!(
            node.dns.values("example.com", &wildcard, RecordType::CNAME),
            vec![ENTRY]
        );

        node.dns
            .remove("example.com", DOMAIN, RecordType::CNAME, None);
        node.dns
            .add("example.com", DOMAIN, RecordType::A, "192.0.2.1");
        assert!(!node.runtime.round().await.unwrap());
        assert_eq!(
            node.dns.values("example.com", DOMAIN, RecordType::A),
            vec!["192.0.2.1"]
        );
        assert!(node
            .dns
            .values("example.com", DOMAIN, RecordType::CNAME)
            .is_empty());
        assert!(node.notices().is_empty());
    }

    #[tokio::test]
    async fn a_revoked_token_is_told_once_per_episode() {
        let node = dns_api_node();
        assert!(node.runtime.round().await.unwrap());
        assert!(node.notices().is_empty());
        node.provider.rejected.store(true, Ordering::Relaxed);
        assert!(!node.runtime.round().await.unwrap());
        assert!(!node.runtime.round().await.unwrap());
        assert_eq!(node.notices().len(), 1);
        assert!(node.notices()[0].contains("token"));
        let status = node.call("domain.status", json!({})).await.unwrap();
        assert_eq!(status["problems"], json!(["credential_rejected"]));

        // A restart continues the episode.
        let saved = std::fs::read_to_string(&node.facts.state_file).unwrap();
        assert!(saved.contains("old-1"));
        let restarted = DomainRuntime::new(node.facts.clone(), None, node.zone.clone());
        assert!(restarted
            .state
            .lock()
            .unwrap()
            .episodes
            .credential_rejected
            .is_some());

        node.provider.rejected.store(false, Ordering::Relaxed);
        node.runtime.round().await.unwrap();
        let status = node.call("domain.status", json!({})).await.unwrap();
        assert_eq!(status["problems"], json!([]));
    }

    #[tokio::test]
    async fn a_failing_renewal_is_told_after_a_while_and_again_before_expiry() {
        let node = dns_api_node();
        let now = buckyos_get_unix_timestamp();
        let wildcard = format!("*.{DOMAIN}");
        *node.zone.certs.lock().unwrap() = vec![
            cert(DOMAIN, None, now),
            cert(&wildcard, Some("rate limited"), now + 40 * 86400),
        ];
        assert!(!node.runtime.round().await.unwrap());
        assert!(
            node.notices().is_empty(),
            "the first attempt may just be unlucky"
        );
        let status = node.call("domain.status", json!({})).await.unwrap();
        assert_eq!(status["problems"], json!(["renewal_failed"]));

        let since = now - RENEWAL_GRACE_SECS - 1;
        node.runtime.state.lock().unwrap().episodes.renewal_failed = Some(since);
        node.runtime.round().await.unwrap();
        node.runtime.round().await.unwrap();
        assert_eq!(node.notices().len(), 1);

        *node.zone.certs.lock().unwrap() =
            vec![cert(&wildcard, Some("rate limited"), now + 13 * 86400)];
        node.runtime.round().await.unwrap();
        node.runtime.round().await.unwrap();
        assert_eq!(node.notices().len(), 2);

        *node.zone.certs.lock().unwrap() = vec![cert(&wildcard, None, now + 90 * 86400)];
        assert!(node.runtime.round().await.unwrap());
        assert_eq!(
            node.runtime.state.lock().unwrap().episodes,
            Episodes::default()
        );
    }

    fn ordering(host: &str, state: &str, next_retry_at: Option<u64>) -> Value {
        let mut status = cert(host, None, 10);
        status["acme"]["state"] = json!(state);
        status["acme"]["next_retry_at"] = json!(next_retry_at);
        status
    }

    #[test]
    fn an_order_is_under_way_while_it_runs_and_not_between_attempts() {
        let now = 1000;
        let busy = |status: Value| order_under_way(DOMAIN, &[status], now);
        for state in ["ordering", "renewing"] {
            assert!(busy(ordering(DOMAIN, state, None)), "{state}");
            assert!(busy(ordering(&format!("*.{DOMAIN}"), state, Some(now - 5))));
            assert!(busy(ordering(DOMAIN, state, Some(now + 30))));
            assert!(!busy(ordering(DOMAIN, state, Some(now + 31))));
            assert!(!busy(ordering("other.example", state, None)));
        }
        for state in ["none", "ready", "manual"] {
            assert!(!busy(ordering(DOMAIN, state, None)), "{state}");
        }
    }

    #[tokio::test]
    async fn no_test_record_is_written_beside_a_real_order() {
        let node = dns_api_node();
        *node.zone.certs.lock().unwrap() = vec![ordering(DOMAIN, "renewing", None)];
        let check = node.call("domain.check", json!({})).await.unwrap();
        let challenge = check["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "challenge")
            .unwrap();
        assert_eq!(
            (&challenge["status"], &challenge["code"]),
            (&json!("waiting"), &json!("challenge_busy"))
        );
        assert!(!node.runtime.round().await.unwrap());
        assert!(node.provider.writes.lock().unwrap().is_empty());

        // A new token does not wait for it.
        let id = new_token(&node, "acme", "another-secret-token").await;
        let update = json!({"mode": "dns_api", "certificate": "acme",
                            "credential_id": id, "accept_warnings": false});
        let status = node.call("domain.update", update).await.unwrap();
        assert_eq!(status["credential"]["id"], id);

        *node.zone.certs.lock().unwrap() = vec![];
        let check = node.call("domain.check", json!({})).await.unwrap();
        assert_eq!(check["ready"], true, "{check:#}");
        // A trial that passed a moment ago still counts.
        *node.zone.certs.lock().unwrap() = vec![ordering(DOMAIN, "ordering", None)];
        let check = node.call("domain.check", json!({})).await.unwrap();
        assert_eq!(check["ready"], true, "{check:#}");
    }

    #[tokio::test]
    async fn the_periodic_check_writes_no_test_record() {
        let node = dns_api_node();
        for status in [vec![], vec![cert(DOMAIN, None, u64::MAX)]] {
            *node.zone.certs.lock().unwrap() = status;
            assert!(node.runtime.round().await.unwrap());
        }
        *node.zone.certs.lock().unwrap() = vec![cert(DOMAIN, Some("rate limited"), u64::MAX)];
        assert!(!node.runtime.round().await.unwrap());
        let status = node.call("domain.status", json!({})).await.unwrap();
        let challenge = status["check"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["id"] == "challenge")
            .unwrap()
            .clone();
        assert_eq!(
            (&challenge["status"], &challenge["code"]),
            (&json!("fail"), &json!("renewal_failed"))
        );
        let writes = node.provider.writes.lock().unwrap().clone();
        assert!(
            writes
                .iter()
                .all(|write| !write.contains("_acme-challenge")),
            "{writes:?}"
        );
    }

    #[tokio::test]
    async fn an_uploaded_certificate_is_never_told_about() {
        let node = node(Some(
            json!({"schema_version": 1, "mode": "manual", "certificate": "upload", "domain": DOMAIN}),
        ));
        let mut status = cert(DOMAIN, Some("x"), 1);
        status["acme"]["state"] = json!("manual");
        *node.zone.certs.lock().unwrap() = vec![status];
        node.runtime.state.lock().unwrap().episodes.renewal_failed = Some(1);
        node.runtime.round().await.unwrap();
        assert!(node.notices().is_empty());
        let status = node.call("domain.status", json!({})).await.unwrap();
        assert_eq!(status["problems"], json!([]));
    }

    #[tokio::test]
    async fn only_a_local_authorized_caller_is_served() {
        let node = node(None);
        let call = |method: &str, token: Option<&str>, ip: &str| {
            let mut request = RPCRequest::new(method, json!({}));
            request.token = token.map(str::to_string);
            node.runtime.handle_rpc_call(request, ip.parse().unwrap())
        };
        assert!(call("domain.status", Some("t"), "127.0.0.1").await.is_ok());
        assert!(call("domain.status", Some("t"), "::ffff:127.0.0.1")
            .await
            .is_ok());
        assert!(call("domain.status", None, "127.0.0.1").await.is_err());
        assert!(call("domain.status", Some("t"), "192.168.1.9")
            .await
            .is_err());
        let _ = call("domain.check", Some("t"), "127.0.0.1").await;
        let _ = call("domain.providers", Some("t"), "127.0.0.1").await;
        assert_eq!(
            *node.zone.authorized.lock().unwrap(),
            vec![false, false, false, true, false]
        );
        for method in [
            "check",
            "inspect_certificate",
            "plan",
            "apply",
            "verify",
            "update",
        ] {
            assert!(needs_sudo(&format!("domain.{method}")), "{method}");
        }
    }

    fn session(user: &str, target: &str, sudo: bool) -> RPCSessionToken {
        let mut token = RPCSessionToken {
            token_type: ::kRPC::RPCSessionTokenType::Normal,
            token: None,
            aud: None,
            exp: Some(u64::MAX),
            iss: Some("verify-hub".to_string()),
            jti: None,
            sub: Some(user.to_string()),
            appid: Some(target.to_string()),
            sudo,
            extra: HashMap::new(),
        };
        token
            .extra
            .insert(TOKEN_USE_CLAIM.to_string(), json!("session"));
        token
            .extra
            .insert(TOKEN_TARGET_KIND_CLAIM.to_string(), json!("system"));
        bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
        token
    }

    #[test]
    fn the_caller_is_the_owner_on_the_control_panel_with_sudo() {
        SystemServiceId::parse(CONTROL_PANEL_SERVICE_UNIQUE_ID).unwrap();
        let owner = session("alice", CONTROL_PANEL_SERVICE_UNIQUE_ID, true);
        check_caller(&owner, "alice", true).unwrap();
        let reader = session("alice", CONTROL_PANEL_SERVICE_UNIQUE_ID, false);
        check_caller(&reader, "alice", false).unwrap();
        assert!(check_caller(&reader, "alice", true).is_err());
        let other = session("bob", CONTROL_PANEL_SERVICE_UNIQUE_ID, true);
        assert!(check_caller(&other, "alice", false).is_err());
        let elsewhere = session("alice", "nfs-server", true);
        assert!(check_caller(&elsewhere, "alice", false).is_err());
        let mut device = owner.clone();
        bind_token_principal_kind(&mut device, TokenPrincipalKind::Device);
        assert!(check_caller(&device, "alice", false).is_err());
        let mut refresh = owner.clone();
        refresh.sudo = false;
        refresh
            .extra
            .insert(TOKEN_USE_CLAIM.to_string(), json!("refresh"));
        assert!(check_caller(&refresh, "alice", false).is_err());
        let mut foreign = owner;
        foreign.iss = Some("ood1".to_string());
        assert!(check_caller(&foreign, "alice", false).is_err());
    }
}
