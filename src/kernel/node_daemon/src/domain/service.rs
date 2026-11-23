use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use base64::Engine;
use buckyos_api::{DomainCertificateSource, DomainHosting, DomainHostingMode};
use buckyos_kit::buckyos_get_unix_timestamp;
use cyfs_dns_provider::{
    create_provider, credential_path, detect_provider, load_credential, provider_table,
    save_credential, DnsCredential, DnsLookupRef, DnsProviderError, DnsProviderResult,
    DnsRecordProviderRef, ProviderInfo, RecordType,
};
use cyfs_identity_cert::{
    clear_manual_mark, inspect_certificate, install_manual_certificates, installed_certificate,
    CertManager, CertificateInput, CertificateInspection, InstalledCertificate,
    PreparedCertificate,
};
use futures::future::join;
use jsonwebtoken::jwk::Jwk;
use log::*;
use name_client::IdentityRoots;
use serde::Serialize;
use tokio::sync::watch;

use super::check::{
    caa, certificate, delegation, entry, ownership, CheckContext, CheckItem, CheckReport,
    DomainBinding, ItemStatus,
};
use super::dns::{
    addresses, ask_all, caa_report, existing_records, find_hosting, own_values, CaaReport, Hosting,
};
use super::plan::{
    apex_block, canonical_domain, challenge_name, find_conflicts, pkx_record_name, pkx_value,
    record_plan, same_values, txt_matches, DomainSetup, ExistingRecord, NetId, PlannedRecord,
    RecordConflict, RecordId, SnNames,
};
use super::{DomainError, DomainResult};

/// Long enough for every service's minimum.
const RECORD_TTL: u32 = 600;
const MAX_CERTIFICATE_BYTES: usize = 1024 * 1024;
const MAX_STAGED: usize = 8;
const ITEM_TIMEOUT: Duration = Duration::from_secs(30);

pub trait ProviderFactory: Send + Sync {
    fn validate(&self, credential: &DnsCredential) -> DnsProviderResult<()>;
    fn create(
        &self,
        credential: &DnsCredential,
        origin: &str,
    ) -> DnsProviderResult<DnsRecordProviderRef>;
}

pub struct DnsServices;

impl ProviderFactory for DnsServices {
    fn validate(&self, credential: &DnsCredential) -> DnsProviderResult<()> {
        credential.validate()
    }

    fn create(
        &self,
        credential: &DnsCredential,
        origin: &str,
    ) -> DnsProviderResult<DnsRecordProviderRef> {
        create_provider(credential, origin)
    }
}

#[async_trait]
pub trait OwnerKeys: Send + Sync {
    /// The key the SN computes `_pkx` from.
    async fn owner_key(&self, owner_name: &str) -> DomainResult<Jwk>;
}

pub trait CertificateTool: Send + Sync {
    fn inspect(
        &self,
        input: &CertificateInput,
        domain: &str,
    ) -> (CertificateInspection, Option<Vec<PreparedCertificate>>);
    fn install(&self, certificates: &[PreparedCertificate]) -> Result<(), String>;
    fn installed(&self, host: &str) -> Result<Option<InstalledCertificate>, String>;
    fn clear_manual_mark(&self, host: &str) -> Result<(), String>;
}

/// The certificates at this node's identity paths.
pub struct IdentityCertificates {
    pub roots: IdentityRoots,
}

impl CertificateTool for IdentityCertificates {
    fn inspect(
        &self,
        input: &CertificateInput,
        domain: &str,
    ) -> (CertificateInspection, Option<Vec<PreparedCertificate>>) {
        inspect_certificate(input, domain)
    }

    fn install(&self, certificates: &[PreparedCertificate]) -> Result<(), String> {
        install_manual_certificates(&self.roots, certificates).map_err(|error| error.to_string())
    }

    fn installed(&self, host: &str) -> Result<Option<InstalledCertificate>, String> {
        installed_certificate(&self.roots, host).map_err(|error| error.to_string())
    }

    fn clear_manual_mark(&self, host: &str) -> Result<(), String> {
        clear_manual_mark(&self.roots, host).map_err(|error| error.to_string())
    }
}

/// Puts a TXT at the challenge name and takes it away again.
#[async_trait]
pub trait ChallengeWriter: Send + Sync {
    async fn add(&self, name: &str, value: &str) -> Result<(), ChallengeFailure>;
    async fn remove(&self, name: &str, value: &str) -> Result<(), ChallengeFailure>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChallengeFailure {
    pub code: &'static str,
    pub detail: String,
}

struct ProviderChallenge {
    provider: DnsRecordProviderRef,
    credential: DnsCredential,
}

impl ProviderChallenge {
    fn failure(&self, error: DnsProviderError) -> ChallengeFailure {
        ChallengeFailure {
            code: match error {
                DnsProviderError::Unauthorized(_) | DnsProviderError::InvalidCredential(_) => {
                    "credential_rejected"
                }
                _ => "challenge_write_failed",
            },
            detail: redact(&error.to_string(), &self.credential),
        }
    }
}

#[async_trait]
impl ChallengeWriter for ProviderChallenge {
    async fn add(&self, name: &str, value: &str) -> Result<(), ChallengeFailure> {
        self.provider
            .add_values(name, RecordType::TXT, RECORD_TTL, vec![value.to_string()])
            .await
            .map_err(|error| self.failure(error))
    }

    async fn remove(&self, name: &str, value: &str) -> Result<(), ChallengeFailure> {
        self.provider
            .remove_values(name, RecordType::TXT, vec![value.to_string()])
            .await
            .map_err(|error| self.failure(error))
    }
}

#[derive(Clone, Debug)]
pub struct ChallengeTiming {
    pub poll: Duration,
    pub authoritative: Duration,
    pub public: Duration,
}

impl Default for ChallengeTiming {
    fn default() -> Self {
        Self {
            poll: Duration::from_secs(3),
            authoritative: Duration::from_secs(180),
            public: Duration::from_secs(150),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChallengeOutcome {
    pub failure: Option<ChallengeFailure>,
}

impl ChallengeOutcome {
    fn pass() -> Self {
        Self { failure: None }
    }

    fn not_visible(detail: String) -> Self {
        Self {
            failure: Some(ChallengeFailure {
                code: "challenge_not_visible",
                detail,
            }),
        }
    }

    fn item(&self) -> CheckItem {
        const ID: &str = "challenge";
        match &self.failure {
            None => CheckItem::pass(ID),
            Some(failure) if failure.code == "challenge_not_visible" => {
                CheckItem::waiting(ID, failure.code).detail(failure.detail.clone())
            }
            Some(failure) => CheckItem::fail(ID, failure.code).detail(failure.detail.clone()),
        }
    }
}

/// Goes the way the CA goes: write a value, see it on the authoritative
/// servers, then through a public resolver. The value is always removed.
pub async fn run_challenge(
    lookup: DnsLookupRef,
    writer: Arc<dyn ChallengeWriter>,
    domain: String,
    servers: Vec<String>,
    timing: ChallengeTiming,
) -> ChallengeOutcome {
    let name = challenge_name(&domain);
    let value = format!("buckyos-{}", hex::encode(rand::random::<[u8; 16]>()));
    let outcome = match writer.add(&name, &value).await {
        Err(failure) => ChallengeOutcome {
            failure: Some(failure),
        },
        Ok(()) => wait_challenge(&lookup, &name, &value, &servers, &timing).await,
    };
    let written =
        !matches!(&outcome.failure, Some(failure) if failure.code == "credential_rejected");
    if written {
        for attempt in 1..=3 {
            match writer.remove(&name, &value).await {
                Ok(()) => break,
                Err(failure) => {
                    warn!(
                        "remove test record at {name} failed ({attempt}/3): {}",
                        failure.detail
                    );
                    tokio::time::sleep(timing.poll).await;
                }
            }
        }
    }
    outcome
}

async fn wait_challenge(
    lookup: &DnsLookupRef,
    name: &str,
    value: &str,
    servers: &[String],
    timing: &ChallengeTiming,
) -> ChallengeOutcome {
    let started = Instant::now();
    loop {
        let answers = ask_all(lookup.as_ref(), servers, name, RecordType::TXT).await;
        let reachable = answers
            .into_iter()
            .filter_map(|(_, answer)| answer.ok())
            .collect::<Vec<_>>();
        let visible = !reachable.is_empty()
            && reachable.iter().all(|answer| {
                own_values(answer, name, RecordType::TXT)
                    .iter()
                    .any(|found| txt_matches(found, value))
            });
        if visible {
            break;
        }
        if started.elapsed() >= timing.authoritative {
            return ChallengeOutcome::not_visible(format!("not on {}", servers.join(", ")));
        }
        tokio::time::sleep(timing.poll).await;
    }
    let started = Instant::now();
    loop {
        if let Ok(answer) = lookup.public(name, RecordType::TXT).await {
            if answer
                .values(RecordType::TXT)
                .iter()
                .any(|found| txt_matches(found, value))
            {
                return ChallengeOutcome::pass();
            }
        }
        if started.elapsed() >= timing.public {
            return ChallengeOutcome::not_visible("not through a public resolver".to_string());
        }
        tokio::time::sleep(timing.poll).await;
    }
}

const TRIAL_PASS_TTL: Duration = Duration::from_secs(600);
const TRIAL_FAIL_TTL: Duration = Duration::from_secs(20);

type TrialResult = Option<(ChallengeOutcome, Instant)>;

/// Trials by setup: a pass is good for a while, so polling does not write a
/// record every time, and a running one is joined.
#[derive(Default)]
struct Trials {
    slots: Mutex<HashMap<String, watch::Receiver<TrialResult>>>,
}

impl Trials {
    fn passed(&self, key: &str) -> bool {
        self.slots.lock().unwrap().get(key).is_some_and(|slot| {
            matches!(&*slot.borrow(), Some((outcome, at))
                if outcome.failure.is_none() && at.elapsed() < TRIAL_PASS_TTL)
        })
    }

    /// `None`: still running after `wait`.
    async fn run<F>(&self, key: String, wait: Duration, start: F) -> Option<ChallengeOutcome>
    where
        F: FnOnce() -> futures::future::BoxFuture<'static, ChallengeOutcome>,
    {
        let mut receiver = {
            let mut slots = self.slots.lock().unwrap();
            slots.retain(|_, slot| match &*slot.borrow() {
                Some((outcome, at)) => {
                    let ttl = if outcome.failure.is_none() {
                        TRIAL_PASS_TTL
                    } else {
                        TRIAL_FAIL_TTL
                    };
                    at.elapsed() < ttl
                }
                None => slot.has_changed().is_ok(),
            });
            match slots.get(&key) {
                Some(slot) => slot.clone(),
                None => {
                    let (sender, receiver) = watch::channel(None);
                    let trial = start();
                    tokio::spawn(async move {
                        let outcome = trial.await;
                        let _ = sender.send(Some((outcome, Instant::now())));
                    });
                    slots.insert(key, receiver.clone());
                    receiver
                }
            }
        };
        let finished =
            tokio::time::timeout(wait, receiver.wait_for(|result| result.is_some())).await;
        match finished {
            Ok(Ok(result)) => result.as_ref().map(|(outcome, _)| outcome.clone()),
            _ => None,
        }
    }
}

struct StagedCredential {
    id: String,
    domain: String,
    credential: DnsCredential,
}

struct StagedCertificate {
    id: String,
    domain: String,
    input: CertificateInput,
    inspection: CertificateInspection,
}

pub enum ApplyCredential {
    New(DnsCredential),
    Known(String),
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PlanResponse {
    pub records: Vec<PlannedRecord>,
    pub conflicts: Vec<RecordConflict>,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApplyStatus {
    Written,
    Unchanged,
    NeedsConfirm,
    Failed,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ApplyResult {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: RecordType,
    pub status: ApplyStatus,
    pub code: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ApplyResponse {
    pub credential_id: Option<String>,
    pub results: Vec<ApplyResult>,
    pub conflicts: Vec<RecordConflict>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct BlockedModes {
    pub sn: Option<&'static str>,
    pub manual: Option<&'static str>,
    pub dns_api: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct InspectDomainResponse {
    pub domain: String,
    pub zone_apex: String,
    pub is_apex: bool,
    pub ns: Vec<String>,
    pub provider: Option<&'static str>,
    pub provider_apex_cname: bool,
    pub delegated_to_sn: bool,
    pub bound: Option<bool>,
    pub caa: CaaReport,
    pub existing: Vec<ExistingRecord>,
    pub recommended: DomainHostingMode,
    pub blocked: BlockedModes,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct InspectCertificateResponse {
    pub certificate_id: Option<String>,
    pub inspection: CertificateInspection,
}

/// What a check does about the challenge of a domain the node writes to.
pub enum Trial {
    /// Write a value and look for it, waiting this long for the result.
    Run(Duration),
    /// The gateway is writing at the same name for a real order.
    Busy,
    /// Known without writing anything.
    Known(CheckItem),
}

/// What a commit installs, checked by the node itself.
pub struct DomainCommit {
    pub hosting: DomainHosting,
    pub credential: Option<DnsCredential>,
    pub certificates: Option<Vec<PreparedCertificate>>,
}

pub struct CommitRequest<'a> {
    pub setup: &'a DomainSetup,
    pub credential_id: Option<&'a str>,
    pub certificate_id: Option<&'a str>,
    pub accept_warnings: bool,
}

pub struct DomainService {
    pub names: SnNames,
    pub lookup: DnsLookupRef,
    providers: Arc<dyn ProviderFactory>,
    owner_keys: Arc<dyn OwnerKeys>,
    certificates: Arc<dyn CertificateTool>,
    /// An activated node also uses what is already on its disk.
    stored: Option<PathBuf>,
    timing: ChallengeTiming,
    staged_credentials: Mutex<Vec<StagedCredential>>,
    staged_certificates: Mutex<Vec<StagedCertificate>>,
    trials: Trials,
}

impl DomainService {
    pub fn new(
        names: SnNames,
        lookup: DnsLookupRef,
        providers: Arc<dyn ProviderFactory>,
        owner_keys: Arc<dyn OwnerKeys>,
        certificates: Arc<dyn CertificateTool>,
    ) -> Self {
        Self {
            names,
            lookup,
            providers,
            owner_keys,
            certificates,
            stored: None,
            timing: ChallengeTiming::default(),
            staged_credentials: Mutex::new(Vec::new()),
            staged_certificates: Mutex::new(Vec::new()),
            trials: Trials::default(),
        }
    }

    pub fn with_stored(mut self, security_root: PathBuf) -> Self {
        self.stored = Some(security_root);
        self
    }

    #[cfg(test)]
    pub fn with_timing(mut self, timing: ChallengeTiming) -> Self {
        self.timing = timing;
        self
    }

    pub fn timing(&self) -> ChallengeTiming {
        self.timing.clone()
    }

    pub fn certificates(&self) -> &dyn CertificateTool {
        self.certificates.as_ref()
    }

    async fn hosting(&self, domain: &str) -> DomainResult<Hosting> {
        find_hosting(self.lookup.as_ref(), domain, &self.names.name_server())
            .await
            .map_err(lookup_failed)?
            .ok_or_else(|| {
                DomainError::new("domain_not_found", format!("{domain} is not registered"))
            })
    }

    async fn pkx(&self, owner_name: &str) -> DomainResult<String> {
        pkx_value(&self.owner_keys.owner_key(owner_name).await?)
    }

    /// Records at `names`; an apex CNAME its service flattened counts as that CNAME.
    async fn existing(
        &self,
        hosting: &Hosting,
        setup: Option<&DomainSetup>,
        names: &[String],
    ) -> DomainResult<Vec<ExistingRecord>> {
        let mut found = existing_records(self.lookup.as_ref(), hosting, names)
            .await
            .map_err(lookup_failed)?;
        let flattens =
            detect_provider(&hosting.zone.ns).is_some_and(|provider| provider.apex_cname);
        if let (true, true, Some(setup)) = (hosting.is_apex, flattens, setup) {
            let target = self.names.entry(&setup.owner_name);
            let ips = self
                .lookup
                .public(&target, RecordType::A)
                .await
                .map(|answer| addresses(&answer))
                .unwrap_or_default()
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            for record in &mut found {
                if record.name == setup.domain
                    && record.ty == RecordType::A
                    && !ips.is_empty()
                    && same_values(RecordType::A, &record.values, &ips)
                    && setup.net_id != NetId::Wan
                {
                    record.ty = RecordType::CNAME;
                    record.values = vec![target.clone()];
                }
            }
        }
        Ok(found)
    }

    async fn plan_and_existing(
        &self,
        setup: &DomainSetup,
        hosting: &Hosting,
    ) -> DomainResult<(Vec<PlannedRecord>, Vec<ExistingRecord>, Vec<RecordConflict>)> {
        let plan = record_plan(setup, &self.names, &self.pkx(&setup.owner_name).await?);
        let mut names = plan
            .iter()
            .map(|record| record.name.clone())
            .collect::<Vec<_>>();
        names.dedup();
        let existing = self.existing(hosting, Some(setup), &names).await?;
        let conflicts = find_conflicts(&plan, &existing, hosting.is_apex);
        Ok((plan, existing, conflicts))
    }

    pub async fn plan(&self, setup: &DomainSetup) -> DomainResult<PlanResponse> {
        setup.validate(&self.names)?;
        let hosting = self.hosting(&setup.domain).await?;
        let (records, _, conflicts) = self.plan_and_existing(setup, &hosting).await?;
        Ok(PlanResponse { records, conflicts })
    }

    pub async fn inspect_domain(
        &self,
        domain: &str,
        net_id: NetId,
        owner_name: &str,
        binding: Option<&dyn DomainBinding>,
    ) -> DomainResult<InspectDomainResponse> {
        let domain = canonical_domain(domain).ok_or_else(|| {
            DomainError::new("invalid_setup", format!("invalid domain `{domain}`"))
        })?;
        DomainSetup {
            domain: domain.clone(),
            mode: DomainHostingMode::Sn,
            certificate: DomainCertificateSource::Acme,
            net_id: NetId::Nat,
            public_ips: Vec::new(),
            owner_name: owner_name.to_string(),
        }
        .validate(&self.names)?;
        let hosting = self.hosting(&domain).await?;
        let provider = detect_provider(&hosting.zone.ns);
        let names = [
            domain.clone(),
            format!("*.{domain}"),
            pkx_record_name(&domain),
            format!("ood1.{domain}"),
            format!("sys.{domain}"),
            format!("public.{domain}"),
        ];
        let bound = async {
            match binding {
                Some(binding) => binding.bound(&domain).await.ok(),
                None => None,
            }
        };
        let (existing, (caa, bound)) = join(
            self.existing(&hosting, None, &names),
            join(caa_report(self.lookup.as_ref(), &domain), bound),
        )
        .await;
        let apex_cname = provider.is_some_and(|provider| provider.apex_cname);
        let block = |mode| apex_block(mode, net_id, hosting.is_apex, apex_cname);
        let recommended = if hosting.delegated_to_sn {
            DomainHostingMode::Sn
        } else if provider.is_some() {
            DomainHostingMode::DnsApi
        } else if !hosting.is_apex {
            DomainHostingMode::Sn
        } else {
            DomainHostingMode::Manual
        };
        Ok(InspectDomainResponse {
            zone_apex: hosting.zone.apex.clone(),
            is_apex: hosting.is_apex,
            ns: hosting.zone.ns.clone(),
            provider: provider.map(|provider| provider.id),
            provider_apex_cname: apex_cname,
            delegated_to_sn: hosting.delegated_to_sn,
            bound,
            // Not answered while the domain is delegated but not bound; checked again later.
            caa: caa.unwrap_or(CaaReport {
                allowed: true,
                records: Vec::new(),
            }),
            existing: existing?,
            recommended,
            blocked: BlockedModes {
                sn: block(DomainHostingMode::Sn),
                manual: block(DomainHostingMode::Manual),
                dns_api: block(DomainHostingMode::DnsApi),
            },
            domain,
        })
    }

    pub fn list_providers() -> &'static [ProviderInfo] {
        provider_table()
    }

    pub fn credential(&self, id: &str) -> DomainResult<DnsCredential> {
        let missing = || DomainError::new("credential_missing", id.to_string());
        if let Some(staged) = self
            .staged_credentials
            .lock()
            .unwrap()
            .iter()
            .find(|staged| staged.id == id)
        {
            return Ok(staged.credential.clone());
        }
        let root = self.stored.as_ref().ok_or_else(missing)?;
        let path = credential_path(root, id).map_err(|_| missing())?;
        load_credential(&path).map_err(|_| missing())
    }

    fn stage_credential(&self, domain: &str, credential: DnsCredential) -> String {
        let mut staged = self.staged_credentials.lock().unwrap();
        if let Some(known) = staged
            .iter()
            .find(|known| known.domain == domain && known.credential == credential)
        {
            return known.id.clone();
        }
        let id = new_credential_id(domain);
        staged.push(StagedCredential {
            id: id.clone(),
            domain: domain.to_string(),
            credential,
        });
        if staged.len() > MAX_STAGED {
            staged.remove(0);
        }
        id
    }

    /// Writes the plan. Only planned names are touched, and one that already
    /// holds something else only when it is in `confirm`.
    pub async fn apply(
        &self,
        setup: &DomainSetup,
        credential: ApplyCredential,
        confirm: &[String],
    ) -> DomainResult<ApplyResponse> {
        setup.validate(&self.names)?;
        if setup.mode != DomainHostingMode::DnsApi {
            return Err(DomainError::new(
                "invalid_setup",
                "records are written in dns_api only",
            ));
        }
        let (credential, known_id) = match credential {
            ApplyCredential::New(credential) => (credential, None),
            ApplyCredential::Known(id) => (self.credential(&id)?, Some(id)),
        };
        let hosting = self.hosting(&setup.domain).await?;
        let provider_info = detect_provider(&hosting.zone.ns);
        if let Some(code) = apex_block(
            setup.mode,
            setup.net_id,
            hosting.is_apex,
            provider_info.is_some_and(|provider| provider.apex_cname),
        ) {
            return Err(DomainError::new(code, setup.domain.clone()));
        }
        let (plan, existing, conflicts) = self.plan_and_existing(setup, &hosting).await?;
        let provider = self
            .providers
            .validate(&credential)
            .and_then(|_| self.providers.create(&credential, &hosting.zone.apex));
        let provider = match provider {
            Ok(provider) => provider,
            Err(error) => {
                return Ok(ApplyResponse {
                    credential_id: None,
                    results: plan
                        .iter()
                        .map(|record| failed_result(record, &error, &credential))
                        .collect(),
                    conflicts,
                })
            }
        };

        let mut results = Vec::new();
        let mut usable = false;
        for record in &plan {
            let in_conflict = conflicts
                .iter()
                .any(|conflict| conflict.name == record.name);
            if in_conflict && !confirm.contains(&record.name) {
                results.push(ApplyResult {
                    name: record.name.clone(),
                    ty: record.ty,
                    status: ApplyStatus::NeedsConfirm,
                    code: None,
                    detail: None,
                });
                continue;
            }
            let here = existing
                .iter()
                .filter(|found| found.name == record.name)
                .collect::<Vec<_>>();
            let planned_types = plan
                .iter()
                .filter(|other| other.name == record.name)
                .map(|other| other.ty)
                .collect::<Vec<_>>();
            let mut remove = conflicts
                .iter()
                .filter(|conflict| conflict.name == record.name)
                .flat_map(|conflict| conflict.existing.iter().map(|found| found.ty))
                .filter(|ty| !planned_types.contains(ty))
                .collect::<Vec<_>>();
            remove.sort();
            remove.dedup();
            let hidden_cname = in_conflict
                && matches!(record.ty, RecordType::A | RecordType::AAAA)
                && provider_info.is_some_and(|provider| provider.id == "cloudflare");
            if hidden_cname && !remove.contains(&RecordType::CNAME) {
                // A proxied CNAME is answered as addresses.
                let _ = provider
                    .set_rrset(&record.name, RecordType::CNAME, RECORD_TTL, vec![])
                    .await;
            }
            match write_record(provider.as_ref(), record, &here, &remove).await {
                Ok(status) => {
                    usable = true;
                    results.push(ApplyResult {
                        name: record.name.clone(),
                        ty: record.ty,
                        status,
                        code: None,
                        detail: None,
                    });
                }
                Err(error) => {
                    let result = failed_result(record, &error, &credential);
                    let hopeless = matches!(
                        error,
                        DnsProviderError::Unauthorized(_)
                            | DnsProviderError::InvalidCredential(_)
                            | DnsProviderError::ZoneNotFound(_)
                    );
                    if hopeless {
                        return Ok(ApplyResponse {
                            credential_id: None,
                            results: plan
                                .iter()
                                .map(|record| failed_result(record, &error, &credential))
                                .collect(),
                            conflicts,
                        });
                    }
                    results.push(result);
                }
            }
        }
        let credential_id = match known_id {
            Some(id) => Some(id),
            None if usable => Some(self.stage_credential(&setup.domain, credential)),
            None => None,
        };
        Ok(ApplyResponse {
            credential_id,
            results,
            conflicts: conflicts
                .into_iter()
                .filter(|conflict| !confirm.contains(&conflict.name))
                .collect(),
        })
    }

    pub async fn check(
        &self,
        setup: &DomainSetup,
        binding: Option<&dyn DomainBinding>,
        credential_id: Option<&str>,
        certificate_id: Option<&str>,
        trial: Trial,
    ) -> DomainResult<CheckReport> {
        setup.validate(&self.names)?;
        let pkx = match self.pkx(&setup.owner_name).await {
            Ok(pkx) => pkx,
            Err(error) if error.code == "bns_unavailable" => {
                return Ok(unreachable_report(setup, &error.detail))
            }
            Err(error) => return Err(error),
        };
        let hosting = match self.hosting(&setup.domain).await {
            Ok(hosting) => hosting,
            Err(error) if error.code == "dns_lookup_failed" => {
                return Ok(unreachable_report(setup, &error.detail))
            }
            Err(error) => return Err(error),
        };
        let ctx = CheckContext {
            lookup: self.lookup.as_ref(),
            setup,
            names: &self.names,
            hosting: &hosting,
            provider: detect_provider(&hosting.zone.ns),
            pkx: &pkx,
        };
        let mut items = Vec::new();
        let (owned, bound) = tokio::time::timeout(ITEM_TIMEOUT, ownership(&ctx, binding))
            .await
            .unwrap_or_else(|_| (CheckItem::waiting("ownership", "lookup_failed"), None));
        items.push(owned);

        let sn_mode = setup.mode == DomainHostingMode::Sn;
        let routing = async {
            let delegated = if sn_mode {
                Some(limited("delegation", delegation(&ctx)).await)
            } else {
                None
            };
            let entry = limited("entry", entry(&ctx, delegated.as_ref())).await;
            delegated.into_iter().chain([entry]).collect::<Vec<_>>()
        };
        let challenge = async {
            match (setup.uses_acme(), setup.mode) {
                (false, _) => None,
                (true, DomainHostingMode::Sn) => Some(CheckItem::new(
                    "challenge",
                    ItemStatus::Skipped,
                    Some("commit_time"),
                )),
                (true, _) => Some(self.challenge(setup, &hosting, credential_id, trial).await),
            }
        };
        let caa = async {
            match setup.uses_acme() {
                true => Some(limited("caa", caa(self.lookup.as_ref(), &setup.domain)).await),
                false => None,
            }
        };
        let (routing, (challenge, caa)) = join(routing, join(challenge, caa)).await;
        items.extend(routing);
        items.extend(challenge);
        items.extend(caa);
        if !setup.uses_acme() {
            items.push(self.certificate_item(setup, certificate_id));
        }
        Ok(CheckReport::new(items, bound, buckyos_get_unix_timestamp()))
    }

    async fn challenge(
        &self,
        setup: &DomainSetup,
        hosting: &Hosting,
        credential_id: Option<&str>,
        trial: Trial,
    ) -> CheckItem {
        const ID: &str = "challenge";
        let Some(id) = credential_id else {
            return CheckItem::fail(ID, "credential_missing");
        };
        let credential = match self.credential(id) {
            Ok(credential) => credential,
            Err(error) => return CheckItem::fail(ID, "credential_missing").detail(error.detail),
        };
        let provider = match self.providers.create(&credential, &hosting.zone.apex) {
            Ok(provider) => provider,
            Err(error) => {
                return CheckItem::fail(ID, "challenge_write_failed")
                    .detail(redact(&error.to_string(), &credential))
            }
        };
        let key = format!(
            "{id} {}",
            serde_json::to_string(setup).unwrap_or_else(|_| setup.domain.clone())
        );
        let wait = match trial {
            Trial::Run(wait) => wait,
            Trial::Known(item) => return item,
            Trial::Busy if self.trials.passed(&key) => return CheckItem::pass(ID),
            Trial::Busy => return CheckItem::waiting(ID, "challenge_busy"),
        };
        let lookup = self.lookup.clone();
        let domain = setup.domain.clone();
        let servers = hosting.zone.ns.clone();
        let timing = self.timing.clone();
        let writer = Arc::new(ProviderChallenge {
            provider,
            credential,
        });
        let outcome = self
            .trials
            .run(key, wait, move || {
                Box::pin(run_challenge(lookup, writer, domain, servers, timing))
            })
            .await;
        match outcome {
            Some(outcome) => outcome.item(),
            None => CheckItem::waiting(ID, "challenge_not_visible"),
        }
    }

    fn certificate_item(&self, setup: &DomainSetup, certificate_id: Option<&str>) -> CheckItem {
        if let Some(id) = certificate_id {
            let staged = self.staged_certificates.lock().unwrap();
            let inspection = staged
                .iter()
                .find(|staged| staged.id == id && staged.domain == setup.domain)
                .map(|staged| &staged.inspection);
            return certificate(inspection);
        }
        if self.stored.is_none() {
            return certificate(None);
        }
        let now = buckyos_get_unix_timestamp();
        for host in certificate_hosts(&setup.domain) {
            match self.certificates.installed(&host) {
                Ok(Some(installed)) if installed.manager != Some(CertManager::Manual) => {
                    return certificate(None)
                }
                Ok(Some(installed)) if installed.not_after > now => {}
                Ok(Some(_)) => {
                    return CheckItem::fail("certificate", "certificate_rejected")
                        .detail(format!("the certificate of {host} has expired"))
                }
                Ok(None) => return certificate(None),
                Err(error) => {
                    return CheckItem::fail("certificate", "certificate_missing").detail(error)
                }
            }
        }
        CheckItem::pass("certificate")
    }

    pub async fn inspect_certificate(
        &self,
        domain: &str,
        files: &[String],
        pfx_password: Option<String>,
    ) -> DomainResult<InspectCertificateResponse> {
        let domain = canonical_domain(domain).ok_or_else(|| {
            DomainError::new("invalid_setup", format!("invalid domain `{domain}`"))
        })?;
        let mut total = 0;
        let mut decoded = Vec::new();
        for file in files {
            let text = file.trim();
            if text.len() / 4 * 3 > MAX_CERTIFICATE_BYTES + 3 {
                return Err(certificate_too_large());
            }
            let bytes = STANDARD
                .decode(text)
                .or_else(|_| STANDARD_NO_PAD.decode(text))
                .map_err(|_| DomainError::new("certificate_unreadable", "files are base64"))?;
            total += bytes.len();
            if total > MAX_CERTIFICATE_BYTES {
                return Err(certificate_too_large());
            }
            decoded.push(bytes);
        }
        let input = CertificateInput {
            files: decoded,
            pfx_password,
        };
        let (inspection, prepared, input) = self.run_inspection(input, &domain).await?;
        let certificate_id = (prepared.is_some() && !inspection.rejected()).then(|| {
            let id = hex::encode(rand::random::<[u8; 12]>());
            let mut staged = self.staged_certificates.lock().unwrap();
            staged.push(StagedCertificate {
                id: id.clone(),
                domain,
                input,
                inspection: inspection.clone(),
            });
            if staged.len() > MAX_STAGED {
                staged.remove(0);
            }
            id
        });
        Ok(InspectCertificateResponse {
            certificate_id,
            inspection,
        })
    }

    async fn run_inspection(
        &self,
        input: CertificateInput,
        domain: &str,
    ) -> DomainResult<(
        CertificateInspection,
        Option<Vec<PreparedCertificate>>,
        CertificateInput,
    )> {
        let tool = self.certificates.clone();
        let domain = domain.to_string();
        tokio::task::spawn_blocking(move || {
            let (inspection, prepared) = tool.inspect(&input, &domain);
            (inspection, prepared, input)
        })
        .await
        .map_err(|_| DomainError::new("certificate_unreadable", "inspection failed"))
    }

    /// Everything a commit needs, checked again by the node: the page's state
    /// is not trusted.
    pub async fn prepare_commit(
        &self,
        req: &CommitRequest<'_>,
        binding: Option<&dyn DomainBinding>,
        acme_issuer: Option<String>,
        trial: Trial,
    ) -> DomainResult<DomainCommit> {
        let (credential, certificates) = self.commit_parts(req).await?;
        let report = self
            .check(
                req.setup,
                binding,
                req.credential_id,
                req.certificate_id,
                trial,
            )
            .await?;
        if !report.ready {
            if let Some(code) = report
                .item("ownership")
                .and_then(|item| item.code.as_deref())
                .filter(|code| code.starts_with("bind_"))
            {
                return Err(DomainError::new(code, report_text(&report)));
            }
            return Err(DomainError::new("domain_not_ready", report_text(&report)));
        }
        finish_commit(req, credential, certificates, acme_issuer)
    }

    /// A new token or certificate where neither the hosting nor the source of
    /// the certificate changes: only what is new is checked, so an unrelated
    /// finding does not keep a dead token or an expired certificate in place.
    pub async fn prepare_refresh(
        &self,
        req: &CommitRequest<'_>,
        new_credential: bool,
        acme_issuer: Option<String>,
    ) -> DomainResult<DomainCommit> {
        let (credential, certificates) = self.commit_parts(req).await?;
        if let (true, Some(credential)) = (new_credential, &credential) {
            self.check_credential(req, credential).await?;
        }
        finish_commit(req, credential, certificates, acme_issuer)
    }

    /// Writes `_pkx` again: idempotent, and beside whatever ACME is doing.
    async fn check_credential(
        &self,
        req: &CommitRequest<'_>,
        credential: &DnsCredential,
    ) -> DomainResult<()> {
        let hosting = self.hosting(&req.setup.domain).await?;
        let pkx = self.pkx(&req.setup.owner_name).await?;
        let written = async {
            self.providers
                .create(credential, &hosting.zone.apex)?
                .add_values(
                    &pkx_record_name(&req.setup.domain),
                    RecordType::TXT,
                    RECORD_TTL,
                    vec![pkx],
                )
                .await
        };
        written.await.map_err(|error| {
            DomainError::new(
                provider_error_code(&error),
                redact(&error.to_string(), credential),
            )
        })
    }

    async fn commit_parts(
        &self,
        req: &CommitRequest<'_>,
    ) -> DomainResult<(Option<DnsCredential>, Option<Vec<PreparedCertificate>>)> {
        req.setup.validate(&self.names)?;
        let dns_api = req.setup.mode == DomainHostingMode::DnsApi;
        let credential = match (dns_api, req.credential_id) {
            (true, Some(id)) => Some(self.credential(id)?),
            (true, None) => return Err(DomainError::new("credential_missing", "no credential")),
            (false, Some(_)) => {
                return Err(DomainError::new(
                    "invalid_setup",
                    "credential_id belongs to dns_api",
                ))
            }
            (false, None) => None,
        };
        Ok((credential, self.commit_certificates(req).await?))
    }

    /// `None`: issued by ACME, or the installed one stays.
    async fn commit_certificates(
        &self,
        req: &CommitRequest<'_>,
    ) -> DomainResult<Option<Vec<PreparedCertificate>>> {
        if req.setup.uses_acme() {
            return match req.certificate_id {
                None => Ok(None),
                Some(_) => Err(DomainError::new(
                    "invalid_setup",
                    "certificate_id belongs to upload",
                )),
            };
        }
        let missing = || DomainError::new("certificate_missing", "no uploaded certificate");
        let Some(id) = req.certificate_id else {
            return if self.stored.is_some() {
                Ok(None)
            } else {
                Err(missing())
            };
        };
        let input = {
            let staged = self.staged_certificates.lock().unwrap();
            let staged = staged
                .iter()
                .find(|staged| staged.id == id && staged.domain == req.setup.domain)
                .ok_or_else(missing)?;
            CertificateInput {
                files: staged.input.files.clone(),
                pfx_password: staged.input.pfx_password.clone(),
            }
        };
        let (inspection, prepared, _) = self.run_inspection(input, &req.setup.domain).await?;
        match prepared {
            Some(prepared) if !inspection.rejected() => {
                if inspection.has_warning() && !req.accept_warnings {
                    return Err(DomainError::new(
                        "certificate_warning_unconfirmed",
                        serde_json::to_string(&inspection.items).unwrap_or_default(),
                    ));
                }
                Ok(Some(prepared))
            }
            _ => Err(DomainError::new(
                "certificate_rejected",
                serde_json::to_string(&inspection.items).unwrap_or_default(),
            )),
        }
    }
}

fn finish_commit(
    req: &CommitRequest<'_>,
    credential: Option<DnsCredential>,
    certificates: Option<Vec<PreparedCertificate>>,
    acme_issuer: Option<String>,
) -> DomainResult<DomainCommit> {
    let hosting = DomainHosting {
        mode: req.setup.mode,
        certificate: req.setup.certificate,
        domain: req.setup.domain.clone(),
        provider: credential
            .as_ref()
            .map(|credential| credential.provider.clone()),
        credential_id: credential
            .as_ref()
            .and(req.credential_id)
            .map(str::to_string),
        acme_issuer,
    };
    hosting
        .validate()
        .map_err(|error| DomainError::new("invalid_setup", error))?;
    Ok(DomainCommit {
        hosting,
        credential,
        certificates,
    })
}

pub fn certificate_hosts(domain: &str) -> [String; 2] {
    [domain.to_string(), format!("*.{domain}")]
}

/// The credential file and the certificates of `commit`. A failure removes a
/// new credential again; certificates are installed all or not at all.
pub fn install_domain(
    security_root: &Path,
    certificates: &dyn CertificateTool,
    commit: &DomainCommit,
) -> Result<(), String> {
    let new_credential = commit
        .hosting
        .credential_id
        .as_deref()
        .filter(|id| credential_path(security_root, id).is_ok_and(|path| !path.exists()));
    let installed = store_credential(security_root, commit).and_then(|_| {
        if let Some(prepared) = &commit.certificates {
            return certificates.install(prepared);
        }
        if !commit.hosting.uses_acme() {
            return Ok(());
        }
        for host in certificate_hosts(&commit.hosting.domain) {
            certificates.clear_manual_mark(&host)?;
        }
        Ok(())
    });
    if installed.is_err() {
        if let Some(id) = new_credential {
            remove_credential(security_root, id);
        }
    }
    installed
}

fn store_credential(security_root: &Path, commit: &DomainCommit) -> Result<(), String> {
    let (Some(credential), Some(id)) = (&commit.credential, &commit.hosting.credential_id) else {
        return Ok(());
    };
    let path = credential_path(security_root, id).map_err(|error| error.to_string())?;
    save_credential(&path, credential).map_err(|error| error.to_string())
}

pub fn remove_credential(security_root: &Path, id: &str) {
    let Ok(path) = credential_path(security_root, id) else {
        return;
    };
    if let Err(error) = std::fs::remove_file(&path) {
        if error.kind() != std::io::ErrorKind::NotFound {
            warn!("remove DNS credential {} failed: {error}", path.display());
        }
    }
}

/// Leaves only the credential in use.
pub fn remove_other_credentials(security_root: &Path, keep: Option<&str>) {
    let Ok(entries) = std::fs::read_dir(security_root.join("dns_credentials")) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let id = path.file_stem().and_then(|stem| stem.to_str());
        let is_credential = path
            .extension()
            .is_some_and(|extension| extension == "json");
        if is_credential && id != keep {
            if let Err(error) = std::fs::remove_file(&path) {
                warn!(
                    "remove unused DNS credential {} failed: {error}",
                    path.display()
                );
            }
        }
    }
}

fn unreachable_report(setup: &DomainSetup, detail: &str) -> CheckReport {
    let sn_mode = setup.mode == DomainHostingMode::Sn;
    let items = [
        ("ownership", true),
        ("delegation", sn_mode),
        ("entry", true),
        ("challenge", setup.uses_acme()),
        ("caa", setup.uses_acme()),
        ("certificate", !setup.uses_acme()),
    ]
    .into_iter()
    .filter(|(_, applies)| *applies)
    .map(|(id, _)| CheckItem::waiting(id, "lookup_failed").detail(detail))
    .collect();
    CheckReport::new(items, None, buckyos_get_unix_timestamp())
}

fn report_text(report: &CheckReport) -> String {
    serde_json::to_string(&report.items).unwrap_or_default()
}

fn certificate_too_large() -> DomainError {
    DomainError::new(
        "certificate_too_large",
        format!("the files may have {MAX_CERTIFICATE_BYTES} bytes together"),
    )
}

fn lookup_failed(error: cyfs_dns_provider::DnsLookupError) -> DomainError {
    DomainError::new("dns_lookup_failed", error.to_string())
}

async fn limited(
    id: &'static str,
    check: impl std::future::Future<Output = CheckItem>,
) -> CheckItem {
    tokio::time::timeout(ITEM_TIMEOUT, check)
        .await
        .unwrap_or_else(|_| CheckItem::waiting(id, "lookup_failed"))
}

/// `[a-z0-9-]{1,64}`
fn new_credential_id(domain: &str) -> String {
    let mut name = domain.replace('.', "-");
    name.truncate(55);
    format!("{name}-{:08x}", rand::random::<u32>())
}

/// A provider's message may quote the request it was given.
fn redact(text: &str, credential: &DnsCredential) -> String {
    let mut text = text.to_string();
    for value in credential.fields.values() {
        for part in std::iter::once(value.as_str()).chain(value.lines()) {
            let part = part.trim();
            if part.len() >= 6 {
                text = text.replace(part, "***");
            }
        }
    }
    text
}

fn failed_result(
    record: &PlannedRecord,
    error: &DnsProviderError,
    credential: &DnsCredential,
) -> ApplyResult {
    ApplyResult {
        name: record.name.clone(),
        ty: record.ty,
        status: ApplyStatus::Failed,
        code: Some(provider_error_code(error).to_string()),
        detail: Some(redact(&error.to_string(), credential)),
    }
}

fn provider_error_code(error: &DnsProviderError) -> &'static str {
    match error {
        DnsProviderError::Unauthorized(_) => "credential_rejected",
        DnsProviderError::InvalidCredential(_) => "credential_invalid",
        DnsProviderError::ZoneNotFound(_) => "zone_not_found",
        DnsProviderError::RateLimited { .. } => "rate_limited",
        DnsProviderError::Network(_) => "network",
        DnsProviderError::Unsupported(_) | DnsProviderError::Api(_) => "provider_error",
    }
}

async fn write_record(
    provider: &dyn cyfs_dns_provider::DnsRecordProvider,
    record: &PlannedRecord,
    here: &[&ExistingRecord],
    remove: &[RecordType],
) -> DnsProviderResult<ApplyStatus> {
    let unchanged = here.iter().any(|found| {
        found.ty == record.ty
            && match record.ty {
                RecordType::TXT => record
                    .values
                    .iter()
                    .all(|value| found.values.iter().any(|found| txt_matches(found, value))),
                ty => same_values(ty, &found.values, &record.values),
            }
    });
    for ty in remove {
        provider
            .set_rrset(&record.name, *ty, RECORD_TTL, vec![])
            .await?;
    }
    if record.id == RecordId::Pkx {
        // Also proves the token when the value is already there.
        provider
            .add_values(&record.name, record.ty, RECORD_TTL, record.values.clone())
            .await?;
    } else if !unchanged {
        provider
            .set_rrset(&record.name, record.ty, RECORD_TTL, record.values.clone())
            .await?;
    }
    Ok(if unchanged {
        ApplyStatus::Unchanged
    } else {
        ApplyStatus::Written
    })
}

pub fn credential_of(
    provider: String,
    fields: BTreeMap<String, String>,
    endpoint: Option<String>,
) -> DnsCredential {
    DnsCredential {
        provider,
        fields,
        endpoint,
    }
}

#[cfg(test)]
mod tests {
    use super::super::check::{CheckTarget, ItemStatus};
    use super::super::testing::*;
    use super::*;
    use ::kRPC::RPCErrors;
    use cyfs_identity_cert::CheckLevel;
    use std::sync::atomic::Ordering;

    const DOMAIN: &str = "home.example.com";

    fn setup(mode: DomainHostingMode, net_id: NetId, ips: &[&str]) -> DomainSetup {
        DomainSetup {
            domain: DOMAIN.to_string(),
            mode,
            certificate: if mode == DomainHostingMode::Manual {
                DomainCertificateSource::Upload
            } else {
                DomainCertificateSource::Acme
            },
            net_id,
            public_ips: ips.iter().map(|ip| ip.parse().unwrap()).collect(),
            owner_name: "alice".to_string(),
        }
    }

    fn token() -> ApplyCredential {
        ApplyCredential::New(credential_of(
            "cloudflare".to_string(),
            BTreeMap::from([("api_token".to_string(), TEST_TOKEN.to_string())]),
            None,
        ))
    }

    fn write_plan(world: &World, setup: &DomainSetup) {
        for record in record_plan(setup, &world.service.names, PKX) {
            for value in record.values {
                world
                    .dns
                    .add("example.com", &record.name, record.ty, &value);
            }
        }
    }

    async fn check(
        world: &World,
        setup: &DomainSetup,
        binding: Option<&FakeBinding>,
        credential_id: Option<&str>,
        certificate_id: Option<&str>,
    ) -> CheckReport {
        world
            .service
            .check(
                setup,
                binding.map(|binding| binding as &dyn DomainBinding),
                credential_id,
                certificate_id,
                Trial::Run(Duration::from_secs(5)),
            )
            .await
            .unwrap()
    }

    fn state(report: &CheckReport, id: &str) -> (ItemStatus, Option<String>) {
        let item = report.item(id).unwrap_or_else(|| panic!("no {id}"));
        (item.status, item.code.clone())
    }

    fn target<'a>(report: &'a CheckReport, label: &str) -> &'a CheckTarget {
        report
            .item("entry")
            .unwrap()
            .targets
            .iter()
            .find(|target| target.host == format!("{label}{DOMAIN}"))
            .unwrap()
    }

    const PASS: (ItemStatus, Option<String>) = (ItemStatus::Pass, None);

    fn fails(code: &str) -> (ItemStatus, Option<String>) {
        (ItemStatus::Fail, Some(code.to_string()))
    }

    fn waits(code: &str) -> (ItemStatus, Option<String>) {
        (ItemStatus::Waiting, Some(code.to_string()))
    }

    async fn certificate_id(world: &World) -> String {
        world
            .service
            .inspect_certificate(DOMAIN, &[STANDARD.encode("pem")], None)
            .await
            .unwrap()
            .certificate_id
            .unwrap()
    }

    #[tokio::test]
    async fn ownership_is_bound_only_after_the_authoritative_servers_show_it() {
        let world = world();
        let setup = setup(DomainHostingMode::Manual, NetId::Nat, &[]);
        let binding = FakeBinding::default();
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "ownership"), waits("txt_missing"));
        assert_eq!(
            report.item("ownership").unwrap().expected,
            vec![format!("TXT {PKX}")]
        );
        assert_eq!(report.bound, Some(false));
        assert_eq!(binding.binds.load(Ordering::Relaxed), 0);

        let pkx = format!("_pkx.{DOMAIN}");
        world
            .dns
            .add("example.com", &pkx, RecordType::TXT, "PKX(other)");
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "ownership"), fails("txt_mismatch"));
        assert_eq!(
            report.item("ownership").unwrap().actual,
            vec!["TXT PKX(other)"]
        );

        world.dns.add("example.com", &pkx, RecordType::TXT, PKX);
        let report = check(&world, &setup, None, None, None).await;
        assert_eq!(state(&report, "ownership"), fails("no_session"));

        *binding.bind_error.lock().unwrap() = Some(RPCErrors::ParseRequestError(
            "[SN:1016:domain_proof_failed] {}".to_string(),
        ));
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(
            state(&report, "ownership"),
            waits("bind_domain_proof_failed")
        );
        *binding.bind_error.lock().unwrap() = Some(RPCErrors::ParseRequestError(
            "[SN:1015:invalid_domain] x".to_string(),
        ));
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "ownership"), fails("bind_invalid_domain"));
        *binding.bind_error.lock().unwrap() = Some(RPCErrors::TokenExpired("x".to_string()));
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "ownership"), fails("no_session"));

        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "ownership"), PASS);
        assert_eq!(report.bound, Some(true));
        let binds = binding.binds.load(Ordering::Relaxed);
        world.dns.remove("example.com", &pkx, RecordType::TXT, None);
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "ownership"), PASS);
        assert_eq!(binding.binds.load(Ordering::Relaxed), binds);
    }

    #[tokio::test]
    async fn sn_hosting_waits_for_the_delegation() {
        let world = world();
        let setup = setup(DomainHostingMode::Sn, NetId::Nat, &[]);
        let binding = FakeBinding::default();
        world.dns.add(
            "example.com",
            &format!("_pkx.{DOMAIN}"),
            RecordType::TXT,
            PKX,
        );
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "ownership"), PASS);
        assert_eq!(state(&report, "delegation"), waits("ns_missing"));
        assert_eq!(state(&report, "entry"), waits("ns_missing"));
        assert_eq!(
            state(&report, "challenge"),
            (ItemStatus::Skipped, Some("commit_time".to_string()))
        );
        assert_eq!(state(&report, "caa"), PASS);
        assert!(report.item("certificate").is_none());
        assert!(!report.ready);

        world
            .dns
            .delegate("example.com", DOMAIN, &[SN_NS, "ns9.dnshost.net"]);
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "delegation"), fails("ns_extra"));

        world.dns.delegate("example.com", DOMAIN, &[SN_NS]);
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "delegation"), waits("ns_not_answering"));

        world.dns.sn_serves(DOMAIN, RELAY_IP);
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert!(report.ready, "{report:#?}");
        assert_eq!(report.item("entry").unwrap().targets.len(), 5);

        // Re-activation: the delegation hides `_pkx`, the SN still knows the domain.
        let report = check(&world, &setup, Some(&FakeBinding::default()), None, None).await;
        assert_eq!(state(&report, "ownership"), waits("txt_missing"));
        assert_eq!(
            report.item("ownership").unwrap().detail.as_deref(),
            Some("delegated_to_sn")
        );

        world
            .dns
            .add("buckyos.ai", "other", RecordType::A, "1.1.1.1");
        world.dns.add(
            DOMAIN,
            &format!("ood1.{DOMAIN}"),
            RecordType::A,
            "192.0.2.1",
        );
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "entry"), fails("stale_record"));
    }

    #[tokio::test]
    async fn manual_records_are_checked_one_by_one() {
        let world = world();
        let setup = setup(DomainHostingMode::Manual, NetId::Nat, &[]);
        let binding = FakeBinding::default();
        let certificate = certificate_id(&world).await;
        write_plan(&world, &setup);
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert!(report.ready, "{report:#?}");
        assert_eq!(
            report.items.iter().map(|item| item.id).collect::<Vec<_>>(),
            vec!["ownership", "entry", "certificate"]
        );
        assert_eq!(
            target(&report, "ood1.").actual,
            vec![format!("CNAME {ENTRY}")]
        );

        // A public resolver that still has the old answer.
        world
            .dns
            .cache_publicly(DOMAIN, RecordType::A, &["192.0.2.1"]);
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(state(&report, "entry"), waits("record_mismatch"));
        world.dns.forget_public_cache();

        // Forgotten records of an older setup shadow the wildcard.
        world.dns.add(
            "example.com",
            &format!("ood1.{DOMAIN}"),
            RecordType::A,
            "192.0.2.1",
        );
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(state(&report, "entry"), fails("stale_record"));
        assert_eq!(target(&report, "ood1.").actual, vec!["A 192.0.2.1"]);
        assert_eq!(target(&report, "sys.").status, ItemStatus::Pass);
        world.dns.remove(
            "example.com",
            &format!("ood1.{DOMAIN}"),
            RecordType::A,
            None,
        );
        world.dns.add(
            "example.com",
            &format!("public.{DOMAIN}"),
            RecordType::TXT,
            "x",
        );
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(
            target(&report, "public.").code.as_deref(),
            Some("stale_record")
        );
        world.dns.remove(
            "example.com",
            &format!("public.{DOMAIN}"),
            RecordType::TXT,
            None,
        );

        world.dns.remove(
            "example.com",
            &format!("*.{DOMAIN}"),
            RecordType::CNAME,
            None,
        );
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(state(&report, "entry"), waits("record_missing"));
        assert_eq!(target(&report, "").status, ItemStatus::Pass);

        world
            .dns
            .remove("example.com", DOMAIN, RecordType::CNAME, None);
        world
            .dns
            .add("example.com", DOMAIN, RecordType::A, "192.0.2.1");
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(state(&report, "entry"), fails("record_conflict"));
        world.dns.remove("example.com", DOMAIN, RecordType::A, None);
        world.dns.add(
            "example.com",
            DOMAIN,
            RecordType::CNAME,
            "other.example.net",
        );
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(state(&report, "entry"), fails("record_mismatch"));

        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "certificate"), fails("certificate_missing"));
        let report = check(&world, &setup, Some(&binding), None, Some("unknown")).await;
        assert_eq!(state(&report, "certificate"), fails("certificate_missing"));
    }

    #[tokio::test]
    async fn a_lagging_name_server_keeps_it_waiting() {
        let world = world();
        let setup = setup(DomainHostingMode::Manual, NetId::Wan, &["203.0.113.7"]);
        write_plan(&world, &setup);
        let binding = FakeBinding::default();
        let certificate = certificate_id(&world).await;
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert!(report.ready, "{report:#?}");
        world
            .dns
            .add("example.com", DOMAIN, RecordType::AAAA, "2001:db8::9");
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(state(&report, "entry"), fails("record_mismatch"));
        world
            .dns
            .remove("example.com", DOMAIN, RecordType::AAAA, None);
        world.dns.kill_server("ns2.dnshost.net");
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert!(report.ready, "one reachable server is enough: {report:#?}");
        world.dns.kill_server("ns1.dnshost.net");
        world.dns.kill_server("a.gtld-servers.net");
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert!(!report.ready);
        assert!(report
            .items
            .iter()
            .filter(|item| item.id != "certificate")
            .all(|item| item.code.as_deref() == Some("lookup_failed")));
    }

    #[tokio::test]
    async fn a_proxied_cloudflare_record_is_refused() {
        let world = world();
        let cloudflare = ["amy.ns.cloudflare.com", "bob.ns.cloudflare.com"];
        world.dns.zone("example.com", &cloudflare);
        world.dns.delegate("com", "example.com", &cloudflare);
        let setup = setup(DomainHostingMode::Manual, NetId::Nat, &[]);
        let binding = FakeBinding::default();
        let certificate = certificate_id(&world).await;
        write_plan(&world, &setup);
        world
            .dns
            .remove("example.com", DOMAIN, RecordType::CNAME, None);
        world
            .dns
            .add("example.com", DOMAIN, RecordType::A, "104.16.1.2");
        let report = check(&world, &setup, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(state(&report, "entry"), fails("proxied"));

        // The same for fixed addresses.
        let wan = super::DomainSetup {
            net_id: NetId::Wan,
            public_ips: vec!["203.0.113.7".parse().unwrap()],
            ..setup.clone()
        };
        let report = check(&world, &wan, Some(&binding), None, Some(&certificate)).await;
        assert_eq!(target(&report, "").code.as_deref(), Some("proxied"));
    }

    #[tokio::test]
    async fn a_flattened_apex_cname_is_compared_by_address() {
        let world = world();
        let cloudflare = ["amy.ns.cloudflare.com", "bob.ns.cloudflare.com"];
        world.dns.zone("example.com", &cloudflare);
        world.dns.delegate("com", "example.com", &cloudflare);
        let mut setup = setup(DomainHostingMode::Manual, NetId::Nat, &[]);
        setup.domain = "example.com".to_string();
        let binding = FakeBinding::default();
        world
            .dns
            .add("example.com", "_pkx.example.com", RecordType::TXT, PKX);
        world
            .dns
            .add("example.com", "example.com", RecordType::A, RELAY_IP);
        world
            .dns
            .add("example.com", "*.example.com", RecordType::CNAME, ENTRY);
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "entry"), PASS, "{report:#?}");
        assert!(world
            .service
            .plan(&setup)
            .await
            .unwrap()
            .conflicts
            .is_empty());
        world
            .dns
            .remove("example.com", "example.com", RecordType::A, None);
        world
            .dns
            .add("example.com", "example.com", RecordType::A, "104.16.1.2");
        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "entry"), fails("proxied"));
    }

    #[tokio::test]
    async fn an_apex_needs_a_subdomain_where_it_cannot_hold_a_cname() {
        let world = world();
        let mut setup = setup(DomainHostingMode::Manual, NetId::Nat, &[]);
        setup.domain = "example.com".to_string();
        let report = check(&world, &setup, None, None, None).await;
        assert_eq!(state(&report, "entry"), fails("apex_needs_subdomain"));
        setup.mode = DomainHostingMode::Sn;
        setup.certificate = DomainCertificateSource::Acme;
        let report = check(&world, &setup, None, None, None).await;
        assert_eq!(state(&report, "delegation"), fails("apex_needs_subdomain"));
        assert!(!report.ready);
        setup.mode = DomainHostingMode::DnsApi;
        let error = world.service.apply(&setup, token(), &[]).await.unwrap_err();
        assert_eq!(error.code, "apex_needs_subdomain");
    }

    #[tokio::test]
    async fn caa_matters_only_for_automatic_certificates() {
        let world = world();
        world.dns.add(
            "example.com",
            "example.com",
            RecordType::CAA,
            "0 issue \"digicert.com\"",
        );
        let mut setup = setup(DomainHostingMode::Sn, NetId::Nat, &[]);
        let report = check(&world, &setup, None, None, None).await;
        assert_eq!(state(&report, "caa"), fails("caa_denied"));
        setup.certificate = DomainCertificateSource::Upload;
        let report = check(&world, &setup, None, None, None).await;
        assert!(report.item("caa").is_none());
    }

    #[tokio::test]
    async fn records_are_written_once_and_only_the_planned_ones() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        world
            .dns
            .add("example.com", "example.com", RecordType::TXT, "v=spf1");
        world.dns.add(
            "example.com",
            &format!("ood1.{DOMAIN}"),
            RecordType::A,
            "192.0.2.1",
        );
        let first = world.service.apply(&setup, token(), &[]).await.unwrap();
        let id = first.credential_id.clone().unwrap();
        assert!(id.starts_with("home-example-com-"), "{id}");
        assert!(id.len() <= 64 && !id.contains(TEST_TOKEN));
        assert!(first
            .results
            .iter()
            .all(|r| r.status == ApplyStatus::Written));
        assert!(first.conflicts.is_empty());
        assert_eq!(
            world.dns.values("example.com", DOMAIN, RecordType::CNAME),
            vec![ENTRY]
        );

        let again = world.service.apply(&setup, token(), &[]).await.unwrap();
        assert_eq!(again.credential_id.as_deref(), Some(id.as_str()));
        let by_id = world
            .service
            .apply(&setup, ApplyCredential::Known(id.clone()), &[])
            .await
            .unwrap();
        for response in [again, by_id] {
            assert!(response
                .results
                .iter()
                .all(|r| r.status == ApplyStatus::Unchanged));
        }
        let pkx = format!("_pkx.{DOMAIN}");
        assert_eq!(
            world.dns.values("example.com", &pkx, RecordType::TXT),
            vec![PKX]
        );
        assert_eq!(
            world
                .dns
                .values("example.com", "example.com", RecordType::TXT),
            vec!["v=spf1"]
        );
        let touched = world.provider.writes.lock().unwrap().clone();
        assert!(
            touched
                .iter()
                .all(|write| write.contains(DOMAIN) && !write.contains("ood1")),
            "{touched:?}"
        );
        assert!(world
            .service
            .apply(&setup, ApplyCredential::Known("unknown".to_string()), &[])
            .await
            .is_err_and(|error| error.code == "credential_missing"));
    }

    #[tokio::test]
    async fn a_half_written_plan_is_completed_by_a_retry() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        world
            .dns
            .add("example.com", DOMAIN, RecordType::CNAME, ENTRY);
        let response = world.service.apply(&setup, token(), &[]).await.unwrap();
        assert_eq!(
            response
                .results
                .iter()
                .map(|r| r.status)
                .collect::<Vec<_>>(),
            vec![
                ApplyStatus::Written,
                ApplyStatus::Unchanged,
                ApplyStatus::Written
            ]
        );
        assert_eq!(
            world.dns.values("example.com", DOMAIN, RecordType::CNAME),
            vec![ENTRY]
        );
    }

    #[tokio::test]
    async fn conflicts_are_changed_only_when_confirmed() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        world
            .dns
            .add("example.com", DOMAIN, RecordType::A, "192.0.2.1");
        world.dns.add(
            "example.com",
            &format!("_pkx.{DOMAIN}"),
            RecordType::TXT,
            "other",
        );
        let plan = world.service.plan(&setup).await.unwrap();
        assert_eq!(plan.records.len(), 3);
        assert_eq!(plan.conflicts.len(), 1);

        let response = world.service.apply(&setup, token(), &[]).await.unwrap();
        assert_eq!(response.results[1].status, ApplyStatus::NeedsConfirm);
        assert_eq!(response.results[2].status, ApplyStatus::Written);
        assert_eq!(response.conflicts.len(), 1);
        assert_eq!(
            world.dns.values("example.com", DOMAIN, RecordType::A),
            vec!["192.0.2.1"]
        );

        let response = world
            .service
            .apply(&setup, token(), &[DOMAIN.to_string()])
            .await
            .unwrap();
        assert_eq!(response.results[1].status, ApplyStatus::Written);
        assert!(response.conflicts.is_empty());
        assert!(world
            .dns
            .values("example.com", DOMAIN, RecordType::A)
            .is_empty());
        assert_eq!(
            world.dns.values("example.com", DOMAIN, RecordType::CNAME),
            vec![ENTRY]
        );
        assert_eq!(
            world
                .dns
                .values("example.com", &format!("_pkx.{DOMAIN}"), RecordType::TXT),
            vec!["other", PKX]
        );
    }

    #[tokio::test]
    async fn a_refused_token_is_not_kept_and_never_quoted() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        world.provider.rejected.store(true, Ordering::Relaxed);
        let response = world.service.apply(&setup, token(), &[]).await.unwrap();
        assert_eq!(response.credential_id, None);
        assert_eq!(response.results.len(), 3);
        assert!(response
            .results
            .iter()
            .all(|r| r.code.as_deref() == Some("credential_rejected")));
        world.provider.rejected.store(false, Ordering::Relaxed);

        let empty = ApplyCredential::New(credential_of(
            "cloudflare".to_string(),
            BTreeMap::new(),
            None,
        ));
        let response = world.service.apply(&setup, empty, &[]).await.unwrap();
        assert_eq!(
            response.results[0].code.as_deref(),
            Some("credential_invalid")
        );

        world.provider.broken.store(true, Ordering::Relaxed);
        let response = world.service.apply(&setup, token(), &[]).await.unwrap();
        assert_eq!(response.credential_id, None);
        assert_eq!(response.results[0].code.as_deref(), Some("provider_error"));

        let credential = credential_of(
            "cloudflare".to_string(),
            BTreeMap::from([("api_token".to_string(), TEST_TOKEN.to_string())]),
            None,
        );
        assert_eq!(
            redact(&format!("GET /x?key={TEST_TOKEN}"), &credential),
            "GET /x?key=***"
        );
        let debug = format!("{credential:?}");
        assert!(!debug.contains(TEST_TOKEN), "{debug}");
    }

    #[tokio::test]
    async fn the_trial_challenge_is_written_seen_and_removed() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        let binding = FakeBinding::default();
        let id = world
            .service
            .apply(&setup, token(), &[])
            .await
            .unwrap()
            .credential_id
            .unwrap();
        let report = check(&world, &setup, Some(&binding), Some(&id), None).await;
        assert!(report.ready, "{report:#?}");
        let challenge = format!("_acme-challenge.{DOMAIN}");
        assert!(world
            .dns
            .values("example.com", &challenge, RecordType::TXT)
            .is_empty());

        // A pass is good for a while; another setup starts over.
        let calls = world.provider.calls.load(Ordering::Relaxed);
        check(&world, &setup, Some(&binding), Some(&id), None).await;
        assert_eq!(world.provider.calls.load(Ordering::Relaxed), calls);
        let mut other = setup.clone();
        other.net_id = NetId::Portmap;
        check(&world, &other, Some(&binding), Some(&id), None).await;
        assert!(world.provider.calls.load(Ordering::Relaxed) > calls);

        let report = check(&world, &setup, Some(&binding), None, None).await;
        assert_eq!(state(&report, "challenge"), fails("credential_missing"));
        let report = check(&world, &setup, Some(&binding), Some("unknown"), None).await;
        assert_eq!(state(&report, "challenge"), fails("credential_missing"));
    }

    #[tokio::test]
    async fn a_failing_trial_also_cleans_up() {
        for public_only in [false, true] {
            let world = world();
            let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
            let binding = FakeBinding::default();
            let id = world
                .service
                .apply(&setup, token(), &[])
                .await
                .unwrap()
                .credential_id
                .unwrap();
            let challenge = format!("_acme-challenge.{DOMAIN}");
            if public_only {
                world
                    .dns
                    .cache_publicly(&challenge, RecordType::TXT, &["old"]);
            } else {
                world.provider.lost.store(true, Ordering::Relaxed);
            }
            let report = check(&world, &setup, Some(&binding), Some(&id), None).await;
            assert_eq!(state(&report, "challenge"), waits("challenge_not_visible"));
            assert!(!report.ready);
            assert!(world
                .dns
                .values("example.com", &challenge, RecordType::TXT)
                .is_empty());
            let writes = world.provider.writes.lock().unwrap().clone();
            let count = |what: &str| writes.iter().filter(|w| w.starts_with(what)).count();
            assert_eq!(count(&format!("add {challenge}")), 1);
            assert_eq!(count(&format!("remove {challenge}")), 1);
        }
    }

    #[tokio::test]
    async fn a_running_trial_is_joined_and_survives_a_dropped_caller() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        let binding = FakeBinding::default();
        let id = world
            .service
            .apply(&setup, token(), &[])
            .await
            .unwrap()
            .credential_id
            .unwrap();
        world.provider.lost.store(true, Ordering::Relaxed);
        let challenge = format!("_acme-challenge.{DOMAIN}");
        for _ in 0..3 {
            let report = world
                .service
                .check(
                    &setup,
                    Some(&binding),
                    Some(&id),
                    None,
                    Trial::Run(Duration::from_millis(1)),
                )
                .await
                .unwrap();
            assert_eq!(state(&report, "challenge"), waits("challenge_not_visible"));
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
        let writes = world.provider.writes.lock().unwrap().clone();
        assert_eq!(
            writes
                .iter()
                .filter(|w| **w == format!("add {challenge} TXT"))
                .count(),
            1
        );
        assert_eq!(
            writes
                .iter()
                .filter(|w| **w == format!("remove {challenge} TXT"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn a_token_revoked_after_writing_is_reported_by_the_trial() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        let id = world
            .service
            .apply(&setup, token(), &[])
            .await
            .unwrap()
            .credential_id
            .unwrap();
        world.provider.rejected.store(true, Ordering::Relaxed);
        let report = check(
            &world,
            &setup,
            Some(&FakeBinding::default()),
            Some(&id),
            None,
        )
        .await;
        assert_eq!(state(&report, "challenge"), fails("credential_rejected"));
    }

    struct SnWriter {
        dns: Arc<FakeDns>,
        refused: bool,
        removed: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl ChallengeWriter for SnWriter {
        async fn add(&self, name: &str, value: &str) -> Result<(), ChallengeFailure> {
            if self.refused {
                return Err(ChallengeFailure {
                    code: "challenge_write_failed",
                    detail: "[SN:1013:device_permission_denied]".to_string(),
                });
            }
            self.dns.add(DOMAIN, name, RecordType::TXT, value);
            Ok(())
        }

        async fn remove(&self, name: &str, value: &str) -> Result<(), ChallengeFailure> {
            self.removed.fetch_add(1, Ordering::Relaxed);
            self.dns.remove(DOMAIN, name, RecordType::TXT, Some(value));
            Ok(())
        }
    }

    #[tokio::test]
    async fn the_sn_challenge_of_a_commit_passes_only_through_the_delegation() {
        let dns = FakeDns::example();
        dns.sn_serves(DOMAIN, RELAY_IP);
        let run = |refused: bool| {
            let writer = Arc::new(SnWriter {
                dns: dns.clone(),
                refused,
                removed: Default::default(),
            });
            let trial = run_challenge(
                dns.clone(),
                writer.clone(),
                DOMAIN.to_string(),
                vec![SN_NS.to_string()],
                fast(),
            );
            async move { (trial.await, writer.removed.load(Ordering::Relaxed)) }
        };
        // Not delegated: the SN has it, the world does not.
        let (outcome, removed) = run(false).await;
        assert_eq!(outcome.failure.unwrap().code, "challenge_not_visible");
        assert_eq!(removed, 1);
        dns.delegate("example.com", DOMAIN, &[SN_NS]);
        let (outcome, removed) = run(false).await;
        assert_eq!(outcome, ChallengeOutcome::pass());
        assert_eq!(removed, 1);
        let (outcome, _) = run(true).await;
        assert_eq!(outcome.failure.unwrap().code, "challenge_write_failed");
        assert!(dns
            .values(
                DOMAIN,
                &format!("_acme-challenge.{DOMAIN}"),
                RecordType::TXT
            )
            .is_empty());
    }

    #[tokio::test]
    async fn a_domain_is_inspected_where_it_is_hosted() {
        let world = world();
        let binding = FakeBinding::default();
        world
            .dns
            .add("example.com", DOMAIN, RecordType::A, "192.0.2.1");
        let found = world
            .service
            .inspect_domain("Home.Example.com.", NetId::Nat, "alice", Some(&binding))
            .await
            .unwrap();
        assert_eq!(found.domain, DOMAIN);
        assert_eq!(found.zone_apex, "example.com");
        assert_eq!(
            (found.is_apex, found.delegated_to_sn, found.bound),
            (false, false, Some(false))
        );
        assert_eq!(found.recommended, DomainHostingMode::Sn);
        assert_eq!(found.existing.len(), 1);
        assert_eq!(
            found.blocked,
            BlockedModes {
                sn: None,
                manual: None,
                dns_api: None
            }
        );

        let apex = world
            .service
            .inspect_domain("example.com", NetId::Nat, "alice", None)
            .await
            .unwrap();
        assert_eq!((apex.is_apex, apex.bound), (true, None));
        assert_eq!(apex.recommended, DomainHostingMode::Manual);
        assert_eq!(
            apex.blocked,
            BlockedModes {
                sn: Some("apex"),
                manual: Some("apex_needs_subdomain"),
                dns_api: Some("apex_needs_subdomain"),
            }
        );
        let wan = world
            .service
            .inspect_domain("example.com", NetId::Wan, "alice", None)
            .await
            .unwrap();
        assert_eq!((wan.blocked.manual, wan.blocked.dns_api), (None, None));

        let cloudflare = ["amy.ns.cloudflare.com", "bob.ns.cloudflare.com"];
        world.dns.zone("example.com", &cloudflare);
        world.dns.delegate("com", "example.com", &cloudflare);
        let apex = world
            .service
            .inspect_domain("example.com", NetId::Nat, "alice", None)
            .await
            .unwrap();
        assert_eq!(
            (apex.provider, apex.provider_apex_cname),
            (Some("cloudflare"), true)
        );
        assert_eq!(apex.recommended, DomainHostingMode::DnsApi);
        assert_eq!((apex.blocked.manual, apex.blocked.dns_api), (None, None));

        world.dns.delegate("example.com", DOMAIN, &[SN_NS]);
        let sn = world
            .service
            .inspect_domain(DOMAIN, NetId::Nat, "alice", None)
            .await
            .unwrap();
        assert!(sn.delegated_to_sn);
        assert_eq!(sn.zone_apex, "example.com");
        assert_eq!(sn.provider, Some("cloudflare"));
        assert_eq!(sn.recommended, DomainHostingMode::Sn);

        for (domain, code) in [
            ("nope.com", "domain_not_found"),
            ("*.example.com", "invalid_setup"),
            ("bob.web3.buckyos.ai", "invalid_setup"),
        ] {
            let error = world
                .service
                .inspect_domain(domain, NetId::Nat, "alice", None)
                .await
                .unwrap_err();
            assert_eq!(error.code, code, "{domain}");
        }
        world.dns.offline.store(true, Ordering::Relaxed);
        let error = world
            .service
            .inspect_domain(DOMAIN, NetId::Nat, "alice", None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "dns_lookup_failed");
    }

    #[tokio::test]
    async fn uploads_are_limited_and_rejected_ones_are_not_kept() {
        let world = world();
        let too_large = STANDARD.encode(vec![0u8; MAX_CERTIFICATE_BYTES + 1]);
        let error = world
            .service
            .inspect_certificate(DOMAIN, &[too_large], None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "certificate_too_large");
        let half = STANDARD.encode(vec![0u8; MAX_CERTIFICATE_BYTES / 2 + 1]);
        let error = world
            .service
            .inspect_certificate(DOMAIN, &[half.clone(), half], None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "certificate_too_large");
        let error = world
            .service
            .inspect_certificate(DOMAIN, &["not base64!".to_string()], None)
            .await
            .unwrap_err();
        assert_eq!(error.code, "certificate_unreadable");

        *world.certificates.inspection.lock().unwrap() =
            Some(inspection(DOMAIN, CheckLevel::Reject));
        let response = world
            .service
            .inspect_certificate(DOMAIN, &[STANDARD.encode("x")], None)
            .await
            .unwrap();
        assert_eq!(response.certificate_id, None);
    }

    async fn commit(
        world: &World,
        setup: &DomainSetup,
        credential_id: Option<&str>,
        certificate_id: Option<&str>,
        accept_warnings: bool,
    ) -> DomainResult<DomainCommit> {
        world
            .service
            .prepare_commit(
                &CommitRequest {
                    setup,
                    credential_id,
                    certificate_id,
                    accept_warnings,
                },
                Some(&FakeBinding::default()),
                None,
                Trial::Run(Duration::from_secs(5)),
            )
            .await
    }

    #[tokio::test]
    async fn a_commit_is_checked_by_the_node_and_not_by_the_page() {
        let world = world();
        let manual = setup(DomainHostingMode::Manual, NetId::Nat, &[]);
        let error = commit(&world, &manual, None, None, true)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "certificate_missing");
        let certificate = certificate_id(&world).await;
        let error = commit(&world, &manual, None, Some(&certificate), true)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "domain_not_ready");
        assert!(serde_json::from_str::<serde_json::Value>(&error.detail)
            .unwrap()
            .is_array());

        write_plan(&world, &manual);
        let ready = commit(&world, &manual, None, Some(&certificate), false)
            .await
            .unwrap();
        ready.hosting.validate().unwrap();
        assert_eq!(ready.certificates.unwrap().len(), 2);

        // The certificate is inspected again, and its warnings need the user.
        *world.certificates.inspection.lock().unwrap() = Some(inspection(DOMAIN, CheckLevel::Warn));
        let error = commit(&world, &manual, None, Some(&certificate), false)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "certificate_warning_unconfirmed");
        assert!(commit(&world, &manual, None, Some(&certificate), true)
            .await
            .is_ok());
        *world.certificates.inspection.lock().unwrap() =
            Some(inspection(DOMAIN, CheckLevel::Reject));
        let error = commit(&world, &manual, None, Some(&certificate), true)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "certificate_rejected");

        world.dns.add(
            "example.com",
            &format!("ood1.{DOMAIN}"),
            RecordType::A,
            "192.0.2.1",
        );
        *world.certificates.inspection.lock().unwrap() = None;
        let error = commit(&world, &manual, None, Some(&certificate), true)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "domain_not_ready");
        assert!(error.detail.contains("stale_record"));

        let binding = FakeBinding::default();
        *binding.bind_error.lock().unwrap() = Some(RPCErrors::ParseRequestError(
            "[SN:1016:domain_proof_failed] {}".to_string(),
        ));
        let error = world
            .service
            .prepare_commit(
                &CommitRequest {
                    setup: &manual,
                    credential_id: None,
                    certificate_id: Some(&certificate),
                    accept_warnings: true,
                },
                Some(&binding),
                None,
                Trial::Run(Duration::from_secs(5)),
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "bind_domain_proof_failed");
    }

    #[tokio::test]
    async fn a_dns_api_commit_needs_its_credential_and_a_passed_trial() {
        let world = world();
        let setup = setup(DomainHostingMode::DnsApi, NetId::Nat, &[]);
        assert_eq!(
            commit(&world, &setup, None, None, false)
                .await
                .err()
                .unwrap()
                .code,
            "credential_missing"
        );
        let id = world
            .service
            .apply(&setup, token(), &[])
            .await
            .unwrap()
            .credential_id
            .unwrap();
        let ready = commit(&world, &setup, Some(&id), None, false)
            .await
            .unwrap();
        assert_eq!(ready.hosting.provider.as_deref(), Some("cloudflare"));
        assert_eq!(ready.hosting.credential_id.as_deref(), Some(id.as_str()));
        assert!(ready.certificates.is_none());
        let error = commit(&world, &setup, Some(&id), Some("x"), false)
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, "invalid_setup");
    }

    #[test]
    fn back_to_automatic_certificates_drops_the_manual_mark() {
        use cyfs_identity_cert::CertManager;
        let root = tempfile::tempdir().unwrap();
        let certificates = FakeCertificates::default();
        for host in certificate_hosts(DOMAIN) {
            certificates.put(&host, CertManager::Manual, u64::MAX);
        }
        let commit = DomainCommit {
            hosting: DomainHosting {
                mode: DomainHostingMode::Sn,
                certificate: DomainCertificateSource::Acme,
                domain: DOMAIN.to_string(),
                provider: None,
                credential_id: None,
                acme_issuer: None,
            },
            credential: None,
            certificates: None,
        };
        install_domain(root.path(), &certificates, &commit).unwrap();
        for host in certificate_hosts(DOMAIN) {
            assert_eq!(certificates.manager(&host), Some(CertManager::Acme));
        }
    }

    #[tokio::test]
    async fn the_real_inspection_refuses_what_is_no_certificate() {
        let root = tempfile::tempdir().unwrap();
        let world = world();
        let service = DomainService::new(
            SnNames::new("buckyos.ai"),
            world.dns.clone(),
            Arc::new(DnsServices),
            Arc::new(FakeOwnerKeys),
            Arc::new(IdentityCertificates {
                roots: buckyos_api::identity_roots_for_buckyos_root(root.path()),
            }),
        );
        let response = service
            .inspect_certificate(DOMAIN, &[STANDARD.encode("nothing")], None)
            .await
            .unwrap();
        assert!(response.inspection.rejected());
        assert_eq!(response.certificate_id, None);
        assert!(matches!(service.certificates().installed(DOMAIN), Ok(None)));
        service.certificates().clear_manual_mark(DOMAIN).unwrap();
    }

    #[test]
    fn credential_ids_fit_a_file_name() {
        let id = new_credential_id(&format!("{}.example.com", "a".repeat(63)));
        assert!(id.len() <= 64);
        assert!(id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'));
    }
}
