//! The checks of TODO §3.3.1.

use std::collections::BTreeSet;
use std::net::IpAddr;

use ::kRPC::RPCErrors;
use async_trait::async_trait;
use buckyos_api::DomainHostingMode;
use cyfs_dns_provider::{DnsLookup, DnsRcode, ProviderInfo, RecordType};
use cyfs_identity_cert::{CertificateInspection, CheckLevel};
use futures::future::join_all;
use serde::Serialize;

use super::dns::{
    addresses, ask_all, ask_first, caa_report, is_cloudflare_address, own_values, same_host,
    Hosting,
};
use super::plan::{apex_block, pkx_record_name, txt_matches, DomainSetup, NetId, SnNames};

/// The code of every item while this node has no Internet access.
pub const OFFLINE: &str = "offline";

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Pass,
    /// Not seen yet; gets better by itself.
    Waiting,
    /// The user has to do something.
    Fail,
    /// Done at commit time.
    Skipped,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CheckTarget {
    pub host: String,
    pub status: ItemStatus,
    pub code: Option<String>,
    pub expected: Vec<String>,
    pub actual: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CheckItem {
    pub id: &'static str,
    pub status: ItemStatus,
    pub code: Option<String>,
    pub expected: Vec<String>,
    pub actual: Vec<String>,
    pub detail: Option<String>,
    pub targets: Vec<CheckTarget>,
}

impl CheckItem {
    pub fn new(id: &'static str, status: ItemStatus, code: Option<&str>) -> Self {
        Self {
            id,
            status,
            code: code.map(str::to_string),
            expected: Vec::new(),
            actual: Vec::new(),
            detail: None,
            targets: Vec::new(),
        }
    }

    pub fn pass(id: &'static str) -> Self {
        Self::new(id, ItemStatus::Pass, None)
    }

    pub fn waiting(id: &'static str, code: &str) -> Self {
        Self::new(id, ItemStatus::Waiting, Some(code))
    }

    pub fn fail(id: &'static str, code: &str) -> Self {
        Self::new(id, ItemStatus::Fail, Some(code))
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    fn values(mut self, expected: Vec<String>, actual: Vec<String>) -> Self {
        self.expected = expected;
        self.actual = actual;
        self
    }
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CheckReport {
    pub ready: bool,
    pub bound: Option<bool>,
    pub checked_at: u64,
    pub items: Vec<CheckItem>,
}

impl CheckReport {
    pub fn new(items: Vec<CheckItem>, bound: Option<bool>, checked_at: u64) -> Self {
        let ready = items
            .iter()
            .all(|item| matches!(item.status, ItemStatus::Pass | ItemStatus::Skipped));
        Self {
            ready,
            bound,
            checked_at,
            items,
        }
    }

    pub fn is_offline(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.code.as_deref() == Some(OFFLINE))
    }

    pub fn item(&self, id: &str) -> Option<&CheckItem> {
        self.items.iter().find(|item| item.id == id)
    }
}

/// Whether the SN has the domain on record as this account's.
#[async_trait]
pub trait DomainBinding: Send + Sync {
    async fn bound(&self, domain: &str) -> Result<bool, RPCErrors>;
    async fn bind(&self, domain: &str) -> Result<(), RPCErrors>;
}

pub struct CheckContext<'a> {
    pub lookup: &'a dyn DnsLookup,
    pub setup: &'a DomainSetup,
    pub names: &'a SnNames,
    pub hosting: &'a Hosting,
    pub provider: Option<&'static ProviderInfo>,
    pub pkx: &'a str,
}

impl CheckContext<'_> {
    fn apex_block(&self) -> Option<&'static str> {
        apex_block(
            self.setup.mode,
            self.setup.net_id,
            self.hosting.is_apex,
            self.provider.is_some_and(|provider| provider.apex_cname),
        )
    }
}

/// `[SN:1016:domain_proof_failed] …` -> `domain_proof_failed`.
pub fn sn_error_name(error: &RPCErrors) -> Option<String> {
    let text = error.to_string();
    let rest = &text[text.find("[SN:")? + 4..];
    let (_, name) = rest[..rest.find(']')?].split_once(':')?;
    Some(name.to_string())
}

pub fn is_session_error(error: &RPCErrors) -> bool {
    matches!(
        error,
        RPCErrors::InvalidToken(_) | RPCErrors::TokenExpired(_)
    ) || matches!(
        sn_error_name(error).as_deref(),
        Some("auth_required" | "invalid_token" | "user_not_found")
    )
}

/// Binds the domain once `_pkx` is seen on the authoritative servers: asking
/// the SN earlier leaves "no such record" in its resolver's cache.
pub async fn ownership(
    ctx: &CheckContext<'_>,
    binding: Option<&dyn DomainBinding>,
) -> (CheckItem, Option<bool>) {
    const ID: &str = "ownership";
    let domain = ctx.setup.domain.as_str();
    let mut bound = None;
    if let Some(binding) = binding {
        match binding.bound(domain).await {
            Ok(true) => return (CheckItem::pass(ID), Some(true)),
            Ok(false) => bound = Some(false),
            Err(error) if is_session_error(&error) => {
                return (
                    CheckItem::fail(ID, "no_session").detail(error.to_string()),
                    None,
                )
            }
            Err(error) => {
                return (
                    CheckItem::waiting(ID, "bind_unavailable").detail(error.to_string()),
                    None,
                )
            }
        }
    }
    let name = pkx_record_name(domain);
    let expected = vec![format!("TXT {}", ctx.pkx)];
    let mut matched = 0;
    let mut answered = 0;
    let mut other = BTreeSet::new();
    let mut failures = Vec::new();
    for (server, answer) in ask_all(ctx.lookup, &ctx.hosting.zone.ns, &name, RecordType::TXT).await
    {
        match answer {
            Ok(answer) => {
                answered += 1;
                let values = own_values(&answer, &name, RecordType::TXT);
                if values.iter().any(|value| txt_matches(value, ctx.pkx)) {
                    matched += 1;
                } else {
                    other.extend(values.into_iter().map(|value| format!("TXT {value}")));
                }
            }
            Err(error) => failures.push(format!("{server}: {error}")),
        }
    }
    if answered == 0 {
        let item = CheckItem::waiting(ID, "lookup_failed").detail(failures.join("; "));
        return (item.values(expected, vec![]), bound);
    }
    if matched < answered {
        let actual = other.into_iter().collect::<Vec<_>>();
        let item = if actual.is_empty() || matched > 0 {
            CheckItem::waiting(ID, "txt_missing")
        } else {
            CheckItem::fail(ID, "txt_mismatch")
        };
        let item = if ctx.hosting.delegated_to_sn {
            item.detail("delegated_to_sn")
        } else {
            item
        };
        return (item.values(expected, actual), bound);
    }
    let seen = expected.clone();
    let Some(binding) = binding else {
        return (
            CheckItem::fail(ID, "no_session").values(expected, seen),
            bound,
        );
    };
    match binding.bind(domain).await {
        Ok(()) => (CheckItem::pass(ID).values(expected, seen), Some(true)),
        Err(error) => {
            let item = if is_session_error(&error) {
                CheckItem::fail(ID, "no_session")
            } else {
                match sn_error_name(&error) {
                    Some(name)
                        if matches!(name.as_str(), "domain_proof_failed" | "internal_error") =>
                    {
                        CheckItem::waiting(ID, &format!("bind_{name}"))
                    }
                    Some(name) => CheckItem::fail(ID, &format!("bind_{name}")),
                    None => CheckItem::waiting(ID, "bind_unavailable"),
                }
            };
            (item.detail(error.to_string()).values(expected, seen), bound)
        }
    }
}

pub async fn delegation(ctx: &CheckContext<'_>) -> CheckItem {
    const ID: &str = "delegation";
    let sn = ctx.names.name_server();
    let expected = vec![format!("NS {sn}")];
    let all = ctx
        .hosting
        .delegation
        .iter()
        .flat_map(|(_, servers)| servers.iter().cloned())
        .collect::<BTreeSet<_>>();
    let actual = all.iter().map(|ns| format!("NS {ns}")).collect::<Vec<_>>();
    if ctx.apex_block().is_some() {
        return CheckItem::fail(ID, "apex_needs_subdomain").values(expected, actual);
    }
    if all.iter().any(|ns| !same_host(ns, &sn)) {
        return CheckItem::fail(ID, "ns_extra").values(expected, actual);
    }
    if ctx
        .hosting
        .delegation
        .iter()
        .any(|(_, servers)| servers.is_empty())
    {
        return CheckItem::waiting(ID, "ns_missing").values(expected, actual);
    }
    let domain = &ctx.setup.domain;
    match ctx.lookup.public(domain, RecordType::SOA).await {
        Ok(answer) => {
            let from_sn = answer.answers.iter().any(|record| {
                record.ty == RecordType::SOA
                    && same_host(&record.name, domain)
                    && record
                        .value
                        .split_whitespace()
                        .next()
                        .is_some_and(|mname| same_host(mname, &sn))
            });
            if from_sn {
                return CheckItem::pass(ID).values(expected, actual);
            }
            let mut actual = actual;
            actual.extend(
                answer
                    .values(RecordType::SOA)
                    .iter()
                    .map(|soa| format!("SOA {soa}")),
            );
            CheckItem::waiting(ID, "ns_not_answering")
                .detail(format!("{:?}", answer.rcode))
                .values(expected, actual)
        }
        Err(error) => CheckItem::waiting(ID, "lookup_failed")
            .detail(error.to_string())
            .values(expected, actual),
    }
}

enum Expect {
    Cname {
        target: String,
        /// What the service answers when it flattens the CNAME of an apex.
        flattened: BTreeSet<IpAddr>,
    },
    Addresses {
        ips: BTreeSet<IpAddr>,
        both_families: bool,
    },
}

impl Expect {
    fn texts(&self) -> Vec<String> {
        match self {
            Self::Cname { target, .. } => vec![format!("CNAME {target}")],
            Self::Addresses { ips, .. } => ip_texts(ips),
        }
    }

    fn both_families(&self) -> bool {
        matches!(
            self,
            Self::Addresses {
                both_families: true,
                ..
            }
        )
    }
}

fn ip_texts(ips: &BTreeSet<IpAddr>) -> Vec<String> {
    ips.iter()
        .map(|ip| format!("{} {ip}", if ip.is_ipv4() { "A" } else { "AAAA" }))
        .collect()
}

#[derive(Clone, Default)]
struct Seen {
    cname: Option<String>,
    ips: BTreeSet<IpAddr>,
}

impl Seen {
    fn texts(&self) -> Vec<String> {
        match &self.cname {
            Some(target) => vec![format!("CNAME {target}")],
            None => ip_texts(&self.ips),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Match,
    Missing,
    Mismatch(&'static str),
}

fn judge(seen: &Seen, expect: &Expect, cloudflare: bool) -> Verdict {
    let proxied =
        cloudflare && !seen.ips.is_empty() && seen.ips.iter().copied().all(is_cloudflare_address);
    match expect {
        Expect::Cname { target, flattened } => match &seen.cname {
            Some(cname) if same_host(cname, target) => Verdict::Match,
            Some(_) => Verdict::Mismatch("record_mismatch"),
            None if seen.ips.is_empty() => Verdict::Missing,
            None if !flattened.is_empty() && seen.ips.is_subset(flattened) => Verdict::Match,
            None if proxied => Verdict::Mismatch("proxied"),
            None if !flattened.is_empty() => Verdict::Mismatch("record_mismatch"),
            None => Verdict::Mismatch("record_conflict"),
        },
        Expect::Addresses { ips: expected, .. } => {
            if seen.cname.is_some() {
                Verdict::Mismatch("record_conflict")
            } else if seen.ips == *expected {
                Verdict::Match
            } else if proxied {
                Verdict::Mismatch("proxied")
            } else if seen.ips.is_subset(expected) {
                Verdict::Missing
            } else {
                Verdict::Mismatch("record_mismatch")
            }
        }
    }
}

async fn seen_at(
    lookup: &dyn DnsLookup,
    server: Option<&str>,
    host: &str,
    both_families: bool,
) -> Result<Seen, String> {
    let mut seen = Seen::default();
    let types: &[RecordType] = if both_families {
        &[RecordType::A, RecordType::AAAA]
    } else {
        &[RecordType::A]
    };
    for ty in types {
        let answer = match server {
            Some(server) => lookup.direct(server, host, *ty).await,
            None => lookup.public(host, *ty).await,
        }
        .map_err(|error| error.to_string())?;
        if !matches!(answer.rcode, DnsRcode::NoError | DnsRcode::NxDomain) {
            return Err(format!("{:?}", answer.rcode));
        }
        if let Some(cname) = own_values(&answer, host, RecordType::CNAME)
            .into_iter()
            .next()
        {
            seen.cname = Some(cname);
        }
        seen.ips.extend(addresses(&answer));
    }
    Ok(seen)
}

struct HostVerdict {
    target: CheckTarget,
    authoritative: Verdict,
}

async fn check_host(
    lookup: &dyn DnsLookup,
    servers: &[String],
    host: String,
    expect: &Expect,
    cloudflare: bool,
) -> HostVerdict {
    let target = |status, code: Option<&str>, actual| CheckTarget {
        host: host.clone(),
        status,
        code: code.map(str::to_string),
        expected: expect.texts(),
        actual,
    };
    let seen = join_all(
        servers
            .iter()
            .take(6)
            .map(|server| seen_at(lookup, Some(server.as_str()), &host, expect.both_families())),
    )
    .await;
    let mut missing = None;
    let mut matched = None;
    for seen in seen.into_iter().flatten() {
        match judge(&seen, expect, cloudflare) {
            Verdict::Mismatch(code) => {
                return HostVerdict {
                    target: target(ItemStatus::Fail, Some(code), seen.texts()),
                    authoritative: Verdict::Mismatch(code),
                }
            }
            Verdict::Missing => missing = Some(seen),
            Verdict::Match => matched = Some(seen),
        }
    }
    if let Some(seen) = missing {
        return HostVerdict {
            target: target(ItemStatus::Waiting, Some("record_missing"), seen.texts()),
            authoritative: Verdict::Missing,
        };
    }
    let Some(matched) = matched else {
        return HostVerdict {
            target: target(ItemStatus::Waiting, Some("lookup_failed"), vec![]),
            authoritative: Verdict::Missing,
        };
    };
    let target = match seen_at(lookup, None, &host, expect.both_families()).await {
        Err(_) => target(ItemStatus::Waiting, Some("lookup_failed"), vec![]),
        Ok(seen) => match judge(&seen, expect, false) {
            Verdict::Match => target(ItemStatus::Pass, None, matched.texts()),
            Verdict::Missing => target(ItemStatus::Waiting, Some("record_missing"), seen.texts()),
            Verdict::Mismatch(_) => {
                target(ItemStatus::Waiting, Some("record_mismatch"), seen.texts())
            }
        },
    };
    HostVerdict {
        target,
        authoritative: Verdict::Match,
    }
}

/// `<domain>`, any name under it, and the names a forgotten record of an older
/// setup would shadow.
pub async fn entry(ctx: &CheckContext<'_>, delegation: Option<&CheckItem>) -> CheckItem {
    const ID: &str = "entry";
    let setup = ctx.setup;
    if ctx.apex_block() == Some("apex_needs_subdomain") {
        return CheckItem::fail(ID, "apex_needs_subdomain");
    }
    if let Some(delegation) = delegation {
        if delegation.status != ItemStatus::Pass {
            let code = delegation.code.clone();
            let mut item = CheckItem::waiting(ID, "ns_missing");
            item.code = code;
            return item;
        }
    }
    let sn_server = ctx.names.name_server();
    let entry_host = ctx.names.entry(&setup.owner_name);
    let sn_mode = setup.mode == DomainHostingMode::Sn;
    let flattens = ctx.hosting.is_apex && ctx.provider.is_some_and(|provider| provider.apex_cname);
    let entry_ips = if sn_mode || (flattens && setup.net_id != NetId::Wan) {
        let servers = [sn_server.clone()];
        let answer = match ask_first(ctx.lookup, &servers, &entry_host, RecordType::A).await {
            Ok(answer) => Ok(answer),
            Err(_) => ctx.lookup.public(&entry_host, RecordType::A).await,
        };
        match answer.map(|answer| addresses(&answer)) {
            Ok(ips) if !ips.is_empty() => ips,
            Ok(_) => {
                return CheckItem::waiting(ID, "lookup_failed")
                    .detail(format!("{entry_host} has no address"))
            }
            Err(error) => return CheckItem::waiting(ID, "lookup_failed").detail(error.to_string()),
        }
    } else {
        BTreeSet::new()
    };
    let expect = |flattened: BTreeSet<IpAddr>| {
        if sn_mode {
            Expect::Addresses {
                ips: entry_ips.clone(),
                both_families: false,
            }
        } else if setup.net_id == NetId::Wan {
            Expect::Addresses {
                ips: setup.public_ips.iter().copied().collect(),
                both_families: true,
            }
        } else {
            Expect::Cname {
                target: entry_host.clone(),
                flattened,
            }
        }
    };
    let apex_expect = expect(entry_ips.clone());
    let expect = expect(BTreeSet::new());
    let servers = if sn_mode {
        vec![sn_server]
    } else {
        ctx.hosting.zone.ns.clone()
    };
    let cloudflare = !sn_mode
        && ctx
            .provider
            .is_some_and(|provider| provider.id == "cloudflare");
    let random = format!("check-{:08x}", rand::random::<u32>());
    let labels = [random.as_str(), "ood1", "sys", "public"];
    let apex = check_host(
        ctx.lookup,
        &servers,
        setup.domain.clone(),
        &apex_expect,
        cloudflare,
    );
    let under = join_all(labels.iter().map(|label| {
        check_host(
            ctx.lookup,
            &servers,
            format!("{label}.{}", setup.domain),
            &expect,
            cloudflare,
        )
    }));
    let (apex, under) = futures::join!(apex, under);
    let wildcard_matches = under[0].authoritative == Verdict::Match;
    let mut targets = vec![apex.target];
    for (index, host) in under.into_iter().enumerate() {
        let mut target = host.target;
        if index > 0 && wildcard_matches {
            let shadowed = match host.authoritative {
                Verdict::Match => false,
                Verdict::Missing => target.code.as_deref() != Some("lookup_failed"),
                Verdict::Mismatch(code) => code != "proxied",
            };
            if shadowed {
                target.status = ItemStatus::Fail;
                target.code = Some("stale_record".to_string());
            }
        }
        targets.push(target);
    }
    if sn_mode {
        // The SN decides these answers; there is nothing for the user to change.
        for target in &mut targets {
            if target.code.as_deref() == Some("record_mismatch") {
                target.status = ItemStatus::Waiting;
            }
        }
    }
    let worst = targets
        .iter()
        .find(|target| target.status == ItemStatus::Fail)
        .or_else(|| {
            targets
                .iter()
                .find(|target| target.status == ItemStatus::Waiting)
        });
    let mut item = match worst {
        None => CheckItem::pass(ID),
        Some(target) => CheckItem::new(ID, target.status, target.code.as_deref()),
    };
    item.expected = expect.texts();
    item.targets = targets;
    item
}

pub async fn caa(lookup: &dyn DnsLookup, domain: &str) -> CheckItem {
    const ID: &str = "caa";
    match caa_report(lookup, domain).await {
        Ok(report) => {
            let actual = report
                .records
                .iter()
                .map(|record| format!("{} CAA {}", record.name, record.value))
                .collect();
            let expected = vec!["issue letsencrypt.org".to_string()];
            if report.allowed {
                CheckItem::pass(ID).values(vec![], actual)
            } else {
                CheckItem::fail(ID, "caa_denied").values(expected, actual)
            }
        }
        Err(error) => CheckItem::waiting(ID, "lookup_failed").detail(error.to_string()),
    }
}

pub fn certificate(inspection: Option<&CertificateInspection>) -> CheckItem {
    const ID: &str = "certificate";
    let Some(inspection) = inspection else {
        return CheckItem::fail(ID, "certificate_missing");
    };
    if inspection.rejected() {
        let reasons = inspection
            .items
            .iter()
            .filter(|item| item.level == CheckLevel::Reject)
            .map(|item| format!("{:?}", item.code))
            .collect::<Vec<_>>();
        return CheckItem::fail(ID, "certificate_rejected").detail(reasons.join(", "));
    }
    CheckItem::pass(ID)
}
