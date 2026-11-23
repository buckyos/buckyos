//! A small DNS world for tests: zones, wildcards, delegations, referrals, and
//! a provider that writes into it.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ::kRPC::RPCErrors;
use async_trait::async_trait;
use cyfs_identity_cert::{
    CertManager, CertificateInput, CertificateInspection, CheckCode, CheckItem, CheckLevel,
    InstalledCertificate, PreparedCertificate,
};
use jsonwebtoken::jwk::Jwk;

use super::check::DomainBinding;
use super::plan::SnNames;
use super::service::{
    certificate_hosts, CertificateTool, ChallengeTiming, DomainService, OwnerKeys,
};
use super::DomainResult;
use cyfs_dns_provider::{
    DnsAnswer, DnsCredential, DnsLookup, DnsLookupError, DnsLookupResult, DnsProviderError,
    DnsProviderResult, DnsRcode, DnsRecordData, DnsRecordProvider, DnsRecordProviderRef,
    RecordType,
};

#[derive(Clone, Default)]
struct Zone {
    ns: Vec<String>,
    records: Vec<(String, RecordType, String)>,
    delegations: BTreeMap<String, Vec<String>>,
}

#[derive(Default)]
struct Records {
    zones: BTreeMap<String, Zone>,
    /// A public resolver that still has an old answer.
    public_cache: HashMap<(String, RecordType), Vec<String>>,
    dead_servers: Vec<String>,
}

#[derive(Default)]
pub struct FakeDns {
    world: Mutex<Records>,
    /// Every lookup fails.
    pub offline: AtomicBool,
}

pub const ENTRY: &str = "alice.web3.buckyos.ai";
pub const RELAY_IP: &str = "198.51.100.9";
pub const SN_NS: &str = "dns.buckyos.ai";

impl FakeDns {
    /// `example.com` at an ordinary DNS service, `buckyos.ai` at the SN.
    pub fn example() -> Arc<Self> {
        let dns = Arc::new(Self::default());
        dns.zone("com", &["a.gtld-servers.net"]);
        dns.zone("ai", &["a.nic.ai"]);
        dns.zone("net", &["a.gtld-servers.net"]);
        dns.zone("example.com", &["ns1.dnshost.net", "ns2.dnshost.net"]);
        dns.delegate(
            "com",
            "example.com",
            &["ns1.dnshost.net", "ns2.dnshost.net"],
        );
        dns.zone("buckyos.ai", &[SN_NS]);
        dns.delegate("ai", "buckyos.ai", &[SN_NS]);
        dns.add("buckyos.ai", ENTRY, RecordType::A, RELAY_IP);
        dns
    }

    pub fn zone(&self, apex: &str, ns: &[&str]) {
        self.world
            .lock()
            .unwrap()
            .zones
            .entry(apex.to_string())
            .or_default()
            .ns = ns.iter().map(ToString::to_string).collect();
    }

    pub fn delegate(&self, zone: &str, name: &str, ns: &[&str]) {
        self.world
            .lock()
            .unwrap()
            .zones
            .get_mut(zone)
            .unwrap()
            .delegations
            .insert(
                name.to_string(),
                ns.iter().map(ToString::to_string).collect(),
            );
    }

    /// The SN takes over `domain` and answers `ip` for it and under it.
    pub fn sn_serves(&self, domain: &str, ip: &str) {
        self.zone(domain, &[SN_NS]);
        self.add(domain, domain, RecordType::A, ip);
        self.add(domain, &format!("*.{domain}"), RecordType::A, ip);
    }

    pub fn add(&self, zone: &str, name: &str, ty: RecordType, value: &str) {
        let mut world = self.world.lock().unwrap();
        let records = &mut world.zones.get_mut(zone).unwrap().records;
        let record = (name.to_string(), ty, value.to_string());
        if !records.contains(&record) {
            records.push(record);
        }
    }

    pub fn remove(&self, zone: &str, name: &str, ty: RecordType, value: Option<&str>) {
        let mut world = self.world.lock().unwrap();
        world
            .zones
            .get_mut(zone)
            .unwrap()
            .records
            .retain(|(n, t, v)| !(n == name && *t == ty && value.is_none_or(|value| value == v)));
    }

    pub fn values(&self, zone: &str, name: &str, ty: RecordType) -> Vec<String> {
        self.world.lock().unwrap().zones[zone]
            .records
            .iter()
            .filter(|(n, t, _)| n == name && *t == ty)
            .map(|(_, _, v)| v.clone())
            .collect()
    }

    pub fn cache_publicly(&self, name: &str, ty: RecordType, values: &[&str]) {
        self.world.lock().unwrap().public_cache.insert(
            (name.to_string(), ty),
            values.iter().map(ToString::to_string).collect(),
        );
    }

    pub fn forget_public_cache(&self) {
        self.world.lock().unwrap().public_cache.clear();
    }

    pub fn kill_server(&self, server: &str) {
        self.world
            .lock()
            .unwrap()
            .dead_servers
            .push(server.to_string());
    }

    fn answer(&self, server: &str, name: &str, ty: RecordType) -> DnsLookupResult<DnsAnswer> {
        let world = self.world.lock().unwrap();
        if self.offline.load(Ordering::Relaxed) || world.dead_servers.iter().any(|s| s == server) {
            return Err(DnsLookupError::Timeout(server.to_string()));
        }
        let Some((apex, zone)) = world
            .zones
            .iter()
            .filter(|(apex, zone)| zone.ns.iter().any(|ns| ns == server) && is_under(name, apex))
            .max_by_key(|(apex, _)| apex.len())
        else {
            return Ok(reply(DnsRcode::Refused, vec![], vec![], false));
        };
        let soa = data(
            apex,
            RecordType::SOA,
            &format!("{} hostmaster.{apex} 1 300 60 86400 60", zone.ns[0]),
        );
        if let Some((cut, ns)) = zone.delegations.iter().find(|(cut, _)| is_under(name, cut)) {
            let authority = ns.iter().map(|ns| data(cut, RecordType::NS, ns)).collect();
            return Ok(reply(DnsRcode::NoError, vec![], authority, false));
        }
        if name == apex && ty == RecordType::SOA {
            return Ok(reply(DnsRcode::NoError, vec![soa], vec![], true));
        }
        if name == apex && ty == RecordType::NS {
            let answers = zone
                .ns
                .iter()
                .map(|ns| data(apex, RecordType::NS, ns))
                .collect();
            return Ok(reply(DnsRcode::NoError, answers, vec![], true));
        }
        let exists = |n: &str| {
            n == apex
                || zone
                    .records
                    .iter()
                    .any(|(record, _, _)| is_under(record, n))
        };
        let mut owner = name.to_string();
        if !exists(name) {
            let mut ancestor = name;
            loop {
                let Some((_, parent)) = ancestor.split_once('.') else {
                    break;
                };
                ancestor = parent;
                let wildcard = format!("*.{ancestor}");
                if zone
                    .records
                    .iter()
                    .any(|(record, _, _)| *record == wildcard)
                {
                    owner = wildcard;
                    break;
                }
                if exists(ancestor) {
                    break;
                }
            }
            if owner == name {
                return Ok(reply(DnsRcode::NxDomain, vec![], vec![soa], true));
            }
        }
        let answers = zone
            .records
            .iter()
            .filter(|(record, t, _)| *record == owner && (*t == ty || *t == RecordType::CNAME))
            .map(|(_, t, value)| data(name, *t, value))
            .collect::<Vec<_>>();
        let authority = if answers.is_empty() {
            vec![soa]
        } else {
            vec![]
        };
        Ok(reply(DnsRcode::NoError, answers, authority, true))
    }
}

fn is_under(name: &str, zone: &str) -> bool {
    name == zone || name.ends_with(&format!(".{zone}"))
}

fn data(name: &str, ty: RecordType, value: &str) -> DnsRecordData {
    DnsRecordData {
        name: name.to_string(),
        ty,
        ttl: 60,
        value: value.to_string(),
    }
}

fn reply(
    rcode: DnsRcode,
    answers: Vec<DnsRecordData>,
    authority: Vec<DnsRecordData>,
    authoritative: bool,
) -> DnsAnswer {
    DnsAnswer {
        rcode,
        answers,
        authority,
        authoritative,
    }
}

#[async_trait]
impl DnsLookup for FakeDns {
    async fn public(&self, name: &str, ty: RecordType) -> DnsLookupResult<DnsAnswer> {
        if self.offline.load(Ordering::Relaxed) {
            return Err(DnsLookupError::Timeout(name.to_string()));
        }
        let cached = self
            .world
            .lock()
            .unwrap()
            .public_cache
            .get(&(name.to_string(), ty))
            .cloned();
        if let Some(values) = cached {
            let answers = values.iter().map(|value| data(name, ty, value)).collect();
            return Ok(reply(DnsRcode::NoError, answers, vec![], false));
        }
        recurse(self, name, ty).await
    }

    async fn direct(&self, server: &str, name: &str, ty: RecordType) -> DnsLookupResult<DnsAnswer> {
        self.answer(server, name, ty)
    }
}

/// What a public resolver does: from the top level down the delegations and
/// along CNAMEs, asking `lookup` directly.
pub async fn recurse(
    lookup: &dyn DnsLookup,
    name: &str,
    ty: RecordType,
) -> DnsLookupResult<DnsAnswer> {
    let failed = || reply(DnsRcode::ServFail, vec![], vec![], false);
    let mut target = name.to_string();
    let mut chain = Vec::new();
    for _ in 0..8 {
        let mut server = match target.rsplit('.').next().unwrap_or_default() {
            "com" | "net" => "a.gtld-servers.net".to_string(),
            "ai" => "a.nic.ai".to_string(),
            _ => return Ok(reply(DnsRcode::NxDomain, chain, vec![], false)),
        };
        let mut answer = lookup.direct(&server, &target, ty).await?;
        while answer.answers.is_empty() && !answer.authoritative {
            let Some(next) = answer.authority.iter().find(|r| r.ty == RecordType::NS) else {
                return Ok(failed());
            };
            server = next.value.clone();
            answer = match lookup.direct(&server, &target, ty).await {
                Ok(answer) if answer.rcode != DnsRcode::Refused => answer,
                _ => return Ok(failed()),
            };
        }
        let cname = answer
            .answers
            .iter()
            .find(|r| r.ty == RecordType::CNAME && ty != RecordType::CNAME)
            .map(|r| r.value.clone());
        chain.extend(answer.answers);
        match cname {
            Some(next) => target = next,
            None => return Ok(reply(answer.rcode, chain, answer.authority, false)),
        }
    }
    Ok(failed())
}

/// Writes into a zone of [`FakeDns`].
pub struct FakeProvider {
    pub dns: Arc<FakeDns>,
    pub zone: String,
    pub rejected: AtomicBool,
    pub broken: AtomicBool,
    /// Accepts writes that never show up.
    pub lost: AtomicBool,
    pub writes: Mutex<Vec<String>>,
    pub calls: AtomicUsize,
}

impl FakeProvider {
    pub fn new(dns: &Arc<FakeDns>, zone: &str) -> Arc<Self> {
        Arc::new(Self {
            dns: dns.clone(),
            zone: zone.to_string(),
            rejected: AtomicBool::new(false),
            broken: AtomicBool::new(false),
            lost: AtomicBool::new(false),
            writes: Mutex::new(Vec::new()),
            calls: AtomicUsize::new(0),
        })
    }

    fn begin(&self, what: String) -> DnsProviderResult<bool> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.rejected.load(Ordering::Relaxed) {
            return Err(DnsProviderError::Unauthorized("revoked".to_string()));
        }
        if self.broken.load(Ordering::Relaxed) {
            return Err(DnsProviderError::Api("internal".to_string()));
        }
        self.writes.lock().unwrap().push(what);
        Ok(!self.lost.load(Ordering::Relaxed))
    }
}

#[async_trait]
impl DnsRecordProvider for FakeProvider {
    async fn set_rrset(
        &self,
        name: &str,
        ty: RecordType,
        _ttl: u32,
        values: Vec<String>,
    ) -> DnsProviderResult<()> {
        if self.begin(format!("set {name} {ty:?} {values:?}"))? {
            self.dns.remove(&self.zone, name, ty, None);
            for value in values {
                self.dns.add(&self.zone, name, ty, &value);
            }
        }
        Ok(())
    }

    async fn add_values(
        &self,
        name: &str,
        ty: RecordType,
        _ttl: u32,
        values: Vec<String>,
    ) -> DnsProviderResult<()> {
        if self.begin(format!("add {name} {ty:?}"))? {
            for value in values {
                self.dns.add(&self.zone, name, ty, &value);
            }
        }
        Ok(())
    }

    async fn remove_values(
        &self,
        name: &str,
        ty: RecordType,
        values: Vec<String>,
    ) -> DnsProviderResult<()> {
        if self.begin(format!("remove {name} {ty:?}"))? {
            for value in values {
                self.dns.remove(&self.zone, name, ty, Some(&value));
            }
        }
        Ok(())
    }
}

pub struct FakeProviders {
    pub provider: Arc<FakeProvider>,
}

impl super::service::ProviderFactory for FakeProviders {
    fn validate(&self, credential: &DnsCredential) -> DnsProviderResult<()> {
        match credential.fields.get("api_token") {
            Some(token) if !token.is_empty() => Ok(()),
            _ => Err(DnsProviderError::InvalidCredential(
                "api_token is missing".to_string(),
            )),
        }
    }

    fn create(
        &self,
        _credential: &DnsCredential,
        origin: &str,
    ) -> DnsProviderResult<DnsRecordProviderRef> {
        if origin != self.provider.zone {
            return Err(DnsProviderError::ZoneNotFound(origin.to_string()));
        }
        Ok(self.provider.clone())
    }
}

pub const TEST_TOKEN: &str = "cf-secret-token-1234567890";

/// Accepts every certificate, or refuses to install any.
#[derive(Default)]
pub struct FakeCertificates {
    pub inspection: Mutex<Option<CertificateInspection>>,
    pub installed: Mutex<HashMap<String, InstalledCertificate>>,
    pub fails: bool,
}

impl FakeCertificates {
    pub fn failing() -> Self {
        Self {
            fails: true,
            ..Default::default()
        }
    }

    pub fn put(&self, host: &str, manager: CertManager, not_after: u64) {
        self.installed.lock().unwrap().insert(
            host.to_string(),
            InstalledCertificate {
                host: host.to_string(),
                manager: Some(manager),
                not_before: 0,
                not_after,
                issuer: None,
                dns_names: vec![host.to_string()],
                fingerprint: "sha256:00".to_string(),
            },
        );
    }

    pub fn manager(&self, host: &str) -> Option<CertManager> {
        self.installed
            .lock()
            .unwrap()
            .get(host)
            .and_then(|c| c.manager)
    }
}

pub fn inspection(domain: &str, level: CheckLevel) -> CertificateInspection {
    CertificateInspection {
        domain: domain.to_string(),
        items: vec![CheckItem {
            code: CheckCode::PublicTrust,
            level,
            reason: None,
            detail: None,
        }],
        coverage: vec![],
        dns_names: vec![],
        issuer: None,
        subject: None,
        not_before: None,
        not_after: None,
        publicly_trusted: level == CheckLevel::Pass,
        node_time: 0,
        fingerprint: None,
    }
}

impl CertificateTool for FakeCertificates {
    fn inspect(
        &self,
        _input: &CertificateInput,
        domain: &str,
    ) -> (CertificateInspection, Option<Vec<PreparedCertificate>>) {
        let inspection = self
            .inspection
            .lock()
            .unwrap()
            .clone()
            .unwrap_or_else(|| inspection(domain, CheckLevel::Pass));
        let prepared = (!inspection.rejected()).then(|| {
            certificate_hosts(domain)
                .into_iter()
                .map(|host| PreparedCertificate {
                    host,
                    fullchain_pem: b"chain".to_vec(),
                    key_pem: b"key".to_vec(),
                })
                .collect()
        });
        (inspection, prepared)
    }

    fn install(&self, certificates: &[PreparedCertificate]) -> Result<(), String> {
        if self.fails {
            return Err("disk full".to_string());
        }
        for certificate in certificates {
            self.put(&certificate.host, CertManager::Manual, u64::MAX);
        }
        Ok(())
    }

    fn installed(&self, host: &str) -> Result<Option<InstalledCertificate>, String> {
        Ok(self.installed.lock().unwrap().get(host).cloned())
    }

    fn clear_manual_mark(&self, host: &str) -> Result<(), String> {
        if let Some(installed) = self.installed.lock().unwrap().get_mut(host) {
            installed.manager = Some(CertManager::Acme);
        }
        Ok(())
    }
}

/// The owner key every test uses.
pub struct FakeOwnerKeys;

pub const OWNER_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
pub const PKX: &str = "PKX(T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8)";

#[async_trait]
impl OwnerKeys for FakeOwnerKeys {
    async fn owner_key(&self, _owner_name: &str) -> DomainResult<Jwk> {
        Ok(serde_json::from_value(
            serde_json::json!({"kty": "OKP", "crv": "Ed25519", "x": OWNER_X}),
        )
        .unwrap())
    }
}

pub struct World {
    pub dns: Arc<FakeDns>,
    pub provider: Arc<FakeProvider>,
    pub certificates: Arc<FakeCertificates>,
    pub service: DomainService,
}

pub fn fast() -> ChallengeTiming {
    ChallengeTiming {
        poll: Duration::from_millis(5),
        authoritative: Duration::from_millis(60),
        public: Duration::from_millis(60),
    }
}

pub fn world() -> World {
    let dns = FakeDns::example();
    let provider = FakeProvider::new(&dns, "example.com");
    let certificates = Arc::new(FakeCertificates::default());
    let service = DomainService::new(
        SnNames::new("buckyos.ai"),
        dns.clone(),
        Arc::new(FakeProviders {
            provider: provider.clone(),
        }),
        Arc::new(FakeOwnerKeys),
        certificates.clone(),
    )
    .with_timing(fast());
    World {
        dns,
        provider,
        certificates,
        service,
    }
}

pub fn fake_service() -> DomainService {
    world().service
}

/// An account at the SN.
#[derive(Default)]
pub struct FakeBinding {
    pub bound: Mutex<Option<String>>,
    pub binds: AtomicUsize,
    /// `domain.bind` answers this.
    pub bind_error: Mutex<Option<RPCErrors>>,
    /// Called with the domain when it is bound.
    pub on_bind: Mutex<Option<Box<dyn Fn(&str) + Send>>>,
}

#[async_trait]
impl DomainBinding for FakeBinding {
    async fn bound(&self, domain: &str) -> Result<bool, RPCErrors> {
        Ok(self.bound.lock().unwrap().as_deref() == Some(domain))
    }

    async fn bind(&self, domain: &str) -> Result<(), RPCErrors> {
        self.binds.fetch_add(1, Ordering::Relaxed);
        if let Some(error) = self.bind_error.lock().unwrap().take() {
            return Err(error);
        }
        *self.bound.lock().unwrap() = Some(domain.to_string());
        if let Some(on_bind) = self.on_bind.lock().unwrap().as_ref() {
            on_bind(domain);
        }
        Ok(())
    }
}
