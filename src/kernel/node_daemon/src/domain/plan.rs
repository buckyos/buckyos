use std::net::IpAddr;

use buckyos_api::{DomainCertificateSource, DomainHostingMode};
use cyfs_dns_provider::RecordType;
use jsonwebtoken::jwk::Jwk;
use name_lib::get_x_from_jwk;
use serde::{Deserialize, Serialize};

use super::{DomainError, DomainResult};

/// How traffic reaches the node.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NetId {
    Nat,
    Portmap,
    WanDyn,
    Wan,
}

impl NetId {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "nat" => Some(Self::Nat),
            "portmap" => Some(Self::Portmap),
            "wan_dyn" => Some(Self::WanDyn),
            "wan" => Some(Self::Wan),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nat => "nat",
            Self::Portmap => "portmap",
            Self::WanDyn => "wan_dyn",
            Self::Wan => "wan",
        }
    }
}

/// One IPv4 and at most one IPv6 for `wan`, nothing for the others.
pub fn validate_public_ips(net_id: &str, ips: &[IpAddr]) -> Result<(), String> {
    if net_id != "wan" {
        return if ips.is_empty() {
            Ok(())
        } else {
            Err(format!("public_ips belong to wan, not {net_id}"))
        };
    }
    let v4 = ips.iter().filter(|ip| ip.is_ipv4()).count();
    let v6 = ips.len() - v4;
    if v4 != 1 || v6 > 1 {
        return Err("wan needs one IPv4 public address and at most one IPv6".to_string());
    }
    for ip in ips {
        let unusable = ip.is_loopback()
            || ip.is_unspecified()
            || ip.is_multicast()
            || match ip {
                IpAddr::V4(v4) => v4.is_link_local() || v4.is_broadcast(),
                IpAddr::V6(v6) => (v6.segments()[0] & 0xffc0) == 0xfe80,
            };
        if unusable {
            return Err(format!("{ip} cannot be a public address"));
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DomainSetup {
    pub domain: String,
    pub mode: DomainHostingMode,
    pub certificate: DomainCertificateSource,
    pub net_id: NetId,
    pub public_ips: Vec<IpAddr>,
    pub owner_name: String,
}

impl DomainSetup {
    pub fn validate(&self, names: &SnNames) -> DomainResult<()> {
        let invalid = |detail: String| Err(DomainError::new("invalid_setup", detail));
        if canonical_domain(&self.domain).as_deref() != Some(self.domain.as_str()) {
            return invalid(format!("invalid domain `{}`", self.domain));
        }
        if self.domain == names.base_host || self.domain.ends_with(&format!(".{}", names.base_host))
        {
            return invalid(format!("`{}` belongs to the SN", self.domain));
        }
        if self.mode == DomainHostingMode::Manual
            && self.certificate != DomainCertificateSource::Upload
        {
            return invalid("manual DNS needs an uploaded certificate".to_string());
        }
        if canonical_domain(&format!("{}.x", self.owner_name)).is_none() {
            return invalid(format!("invalid owner name `{}`", self.owner_name));
        }
        validate_public_ips(self.net_id.as_str(), &self.public_ips).or_else(invalid)
    }

    pub fn uses_acme(&self) -> bool {
        self.certificate == DomainCertificateSource::Acme
    }
}

/// Lower case, no trailing dot, at least two labels, ASCII host name labels.
/// For such a name it is what the SN's `canonical_user_domain` returns.
pub fn canonical_domain(value: &str) -> Option<String> {
    let name = value.trim().trim_end_matches('.').to_ascii_lowercase();
    let labels = name.split('.').collect::<Vec<_>>();
    let valid = name.len() <= 200
        && labels.len() >= 2
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
        && labels
            .last()
            .is_some_and(|label| !label.bytes().all(|b| b.is_ascii_digit()));
    valid.then_some(name)
}

/// The host names the SN serves this account under.
#[derive(Clone, Debug, PartialEq)]
pub struct SnNames {
    pub base_host: String,
}

impl SnNames {
    pub fn new(base_host: &str) -> Self {
        Self {
            base_host: base_host.trim().to_ascii_lowercase(),
        }
    }

    /// `sn.<base>` -> `<base>`.
    pub fn from_sn_host(sn_host: &str) -> Option<Self> {
        sn_host
            .trim()
            .strip_prefix("sn.")
            .filter(|base| !base.is_empty())
            .map(Self::new)
    }

    pub fn name_server(&self) -> String {
        format!("dns.{}", self.base_host)
    }

    pub fn entry(&self, owner_name: &str) -> String {
        format!("{owner_name}.web3.{}", self.base_host)
    }
}

/// Same name and value as the SN's `pkx_record_name` and `pkx_value`.
pub fn pkx_record_name(domain: &str) -> String {
    format!("_pkx.{domain}")
}

pub fn pkx_value(owner_key: &Jwk) -> DomainResult<String> {
    let x = get_x_from_jwk(owner_key)
        .map_err(|error| DomainError::new("owner_key_invalid", error.to_string()))?;
    let x = x.trim();
    if x.is_empty() {
        return Err(DomainError::new("owner_key_invalid", "owner key has no x"));
    }
    Ok(format!("PKX({x})"))
}

/// Same comparison as the SN's `txt_matches_pkx`.
pub fn txt_matches(txt: &str, expected: &str) -> bool {
    txt.trim().trim_matches('"').trim() == expected
}

pub fn challenge_name(domain: &str) -> String {
    format!("_acme-challenge.{domain}")
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordId {
    Pkx,
    Entry,
    Wildcard,
    Ns,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecordPhase {
    Ownership,
    Routing,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct PlannedRecord {
    pub id: RecordId,
    pub phase: RecordPhase,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: RecordType,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RecordValues {
    #[serde(rename = "type")]
    pub ty: RecordType,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ExistingRecord {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: RecordType,
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct RecordConflict {
    pub name: String,
    pub existing: Vec<RecordValues>,
    pub planned: RecordValues,
}

/// The records of `setup` (TODO §3.3).
pub fn record_plan(setup: &DomainSetup, names: &SnNames, pkx: &str) -> Vec<PlannedRecord> {
    let mut records = vec![PlannedRecord {
        id: RecordId::Pkx,
        phase: RecordPhase::Ownership,
        name: pkx_record_name(&setup.domain),
        ty: RecordType::TXT,
        values: vec![pkx.to_string()],
    }];
    if setup.mode == DomainHostingMode::Sn {
        records.push(PlannedRecord {
            id: RecordId::Ns,
            phase: RecordPhase::Routing,
            name: setup.domain.clone(),
            ty: RecordType::NS,
            values: vec![names.name_server()],
        });
        return records;
    }
    for (id, name) in [
        (RecordId::Entry, setup.domain.clone()),
        (RecordId::Wildcard, format!("*.{}", setup.domain)),
    ] {
        for (ty, values) in entry_values(setup, names) {
            records.push(PlannedRecord {
                id,
                phase: RecordPhase::Routing,
                name: name.clone(),
                ty,
                values,
            });
        }
    }
    records
}

/// What `<domain>` and every name under it answer: a CNAME to the SN's name
/// of this account, or the fixed addresses of `wan`.
pub fn entry_values(setup: &DomainSetup, names: &SnNames) -> Vec<(RecordType, Vec<String>)> {
    if setup.net_id != NetId::Wan {
        return vec![(RecordType::CNAME, vec![names.entry(&setup.owner_name)])];
    }
    let of = |v4: bool| {
        setup
            .public_ips
            .iter()
            .filter(|ip| ip.is_ipv4() == v4)
            .map(IpAddr::to_string)
            .collect::<Vec<_>>()
    };
    [(RecordType::A, of(true)), (RecordType::AAAA, of(false))]
        .into_iter()
        .filter(|(_, values)| !values.is_empty())
        .collect()
}

/// Why `mode` cannot serve this domain (Q7), or `None`.
pub fn apex_block(
    mode: DomainHostingMode,
    net_id: NetId,
    is_apex: bool,
    apex_cname: bool,
) -> Option<&'static str> {
    if !is_apex {
        return None;
    }
    match mode {
        DomainHostingMode::Sn => Some("apex"),
        _ if net_id == NetId::Wan || apex_cname => None,
        _ => Some("apex_needs_subdomain"),
    }
}

pub fn same_values(ty: RecordType, left: &[String], right: &[String]) -> bool {
    normalized(ty, left) == normalized(ty, right)
}

fn normalized(ty: RecordType, values: &[String]) -> Vec<String> {
    let mut values = values
        .iter()
        .map(|value| match ty {
            RecordType::TXT => value.clone(),
            RecordType::A | RecordType::AAAA => value
                .parse::<IpAddr>()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|_| value.clone()),
            _ => value.trim_end_matches('.').to_ascii_lowercase(),
        })
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

/// Records already at the planned names that the plan would change or hide.
/// Other TXT values next to `_pkx` stay, so they are no conflict.
pub fn find_conflicts(
    planned: &[PlannedRecord],
    existing: &[ExistingRecord],
    is_apex: bool,
) -> Vec<RecordConflict> {
    let mut conflicts = Vec::new();
    for record in planned {
        let planned_types = planned
            .iter()
            .filter(|other| other.name == record.name)
            .map(|other| other.ty)
            .collect::<Vec<_>>();
        let in_the_way = existing
            .iter()
            .filter(|found| found.name == record.name)
            .filter(|found| match (record.ty, found.ty) {
                (planned, found_ty) if planned == found_ty => {
                    record.ty != RecordType::TXT
                        && !same_values(record.ty, &record.values, &found.values)
                }
                (RecordType::TXT, other) => other == RecordType::CNAME,
                (RecordType::CNAME, RecordType::A | RecordType::AAAA) => true,
                (RecordType::CNAME, RecordType::TXT | RecordType::NS) => !is_apex,
                (RecordType::A | RecordType::AAAA, RecordType::CNAME) => true,
                (RecordType::A, RecordType::AAAA) => !planned_types.contains(&RecordType::AAAA),
                (RecordType::A | RecordType::AAAA, RecordType::NS) => !is_apex,
                (RecordType::NS, RecordType::A | RecordType::AAAA | RecordType::CNAME) => true,
                _ => false,
            })
            .map(|found| RecordValues {
                ty: found.ty,
                values: found.values.clone(),
            })
            .collect::<Vec<_>>();
        if !in_the_way.is_empty() {
            conflicts.push(RecordConflict {
                name: record.name.clone(),
                existing: in_the_way,
                planned: RecordValues {
                    ty: record.ty,
                    values: record.values.clone(),
                },
            });
        }
    }
    conflicts
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn names() -> SnNames {
        SnNames::new("buckyos.ai")
    }

    fn setup(domain: &str, mode: DomainHostingMode, net_id: NetId, ips: &[&str]) -> DomainSetup {
        DomainSetup {
            domain: domain.to_string(),
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

    fn existing(name: &str, ty: RecordType, values: &[&str]) -> ExistingRecord {
        ExistingRecord {
            name: name.to_string(),
            ty,
            values: values.iter().map(ToString::to_string).collect(),
        }
    }

    const MODES: [DomainHostingMode; 3] = [
        DomainHostingMode::Sn,
        DomainHostingMode::Manual,
        DomainHostingMode::DnsApi,
    ];
    const NET_IDS: [NetId; 4] = [NetId::Nat, NetId::Portmap, NetId::WanDyn, NetId::Wan];

    #[test]
    fn every_mode_and_access_gets_its_records() {
        for domain in ["home.example.com", "example.com"] {
            for mode in MODES {
                for net_id in NET_IDS {
                    let ips: &[&str] = if net_id == NetId::Wan {
                        &["203.0.113.7"]
                    } else {
                        &[]
                    };
                    let setup = setup(domain, mode, net_id, ips);
                    setup.validate(&names()).unwrap();
                    let plan = record_plan(&setup, &names(), "PKX(x)");
                    assert_eq!(
                        plan[0],
                        PlannedRecord {
                            id: RecordId::Pkx,
                            phase: RecordPhase::Ownership,
                            name: format!("_pkx.{domain}"),
                            ty: RecordType::TXT,
                            values: vec!["PKX(x)".to_string()],
                        }
                    );
                    let rest = plan[1..]
                        .iter()
                        .map(|r| (r.id, r.name.clone(), r.ty, r.values.join(",")))
                        .collect::<Vec<_>>();
                    let expected = if mode == DomainHostingMode::Sn {
                        vec![(
                            RecordId::Ns,
                            domain.to_string(),
                            RecordType::NS,
                            "dns.buckyos.ai".to_string(),
                        )]
                    } else {
                        let (ty, value) = if net_id == NetId::Wan {
                            (RecordType::A, "203.0.113.7")
                        } else {
                            (RecordType::CNAME, "alice.web3.buckyos.ai")
                        };
                        vec![
                            (RecordId::Entry, domain.to_string(), ty, value.to_string()),
                            (
                                RecordId::Wildcard,
                                format!("*.{domain}"),
                                ty,
                                value.to_string(),
                            ),
                        ]
                    };
                    assert_eq!(rest, expected, "{domain} {mode:?} {net_id:?}");
                    assert!(plan[1..].iter().all(|r| r.phase == RecordPhase::Routing));
                }
            }
        }
    }

    #[test]
    fn wan_with_both_families_gets_two_records_per_name() {
        let setup = setup(
            "home.example.com",
            DomainHostingMode::DnsApi,
            NetId::Wan,
            &["203.0.113.7", "2001:db8::7"],
        );
        let plan = record_plan(&setup, &names(), "PKX(x)");
        assert_eq!(
            plan[1..]
                .iter()
                .map(|r| (r.name.as_str(), r.ty, r.values[0].as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("home.example.com", RecordType::A, "203.0.113.7"),
                ("home.example.com", RecordType::AAAA, "2001:db8::7"),
                ("*.home.example.com", RecordType::A, "203.0.113.7"),
                ("*.home.example.com", RecordType::AAAA, "2001:db8::7"),
            ]
        );
        assert_eq!(
            serde_json::to_value(&plan[2]).unwrap(),
            json!({"id":"entry","phase":"routing","name":"home.example.com","type":"AAAA","values":["2001:db8::7"]})
        );
    }

    #[test]
    fn apex_rules_follow_the_mode_the_access_and_the_provider() {
        for net_id in NET_IDS {
            for apex_cname in [false, true] {
                for mode in MODES {
                    assert_eq!(apex_block(mode, net_id, false, apex_cname), None);
                }
                assert_eq!(
                    apex_block(DomainHostingMode::Sn, net_id, true, apex_cname),
                    Some("apex")
                );
                for mode in [DomainHostingMode::Manual, DomainHostingMode::DnsApi] {
                    let expected =
                        (net_id != NetId::Wan && !apex_cname).then_some("apex_needs_subdomain");
                    assert_eq!(apex_block(mode, net_id, true, apex_cname), expected);
                }
            }
        }
    }

    #[test]
    fn inconsistent_setups_are_refused() {
        let ok = setup("home.example.com", DomainHostingMode::Sn, NetId::Nat, &[]);
        ok.validate(&names()).unwrap();
        let mut cases = Vec::new();
        for domain in [
            "Home.example.com",
            "example",
            "*.example.com",
            "a..com",
            "1.2.3.4",
            "home.example.com.",
            "alice.web3.buckyos.ai",
            "buckyos.ai",
            "a_b.example.com",
        ] {
            let mut bad = ok.clone();
            bad.domain = domain.to_string();
            cases.push(bad);
        }
        let mut bad = ok.clone();
        bad.mode = DomainHostingMode::Manual;
        cases.push(bad);
        let mut bad = ok.clone();
        bad.public_ips = vec!["203.0.113.7".parse().unwrap()];
        cases.push(bad);
        for ips in [
            &[][..],
            &["2001:db8::7"],
            &["203.0.113.7", "203.0.113.8"],
            &["127.0.0.1"],
            &["0.0.0.0"],
        ] {
            let mut bad = ok.clone();
            bad.net_id = NetId::Wan;
            bad.public_ips = ips.iter().map(|ip| ip.parse().unwrap()).collect();
            cases.push(bad);
        }
        let mut bad = ok.clone();
        bad.owner_name = "a/b".to_string();
        cases.push(bad);
        for bad in cases {
            assert_eq!(
                bad.validate(&names()).unwrap_err().code,
                "invalid_setup",
                "{bad:?}"
            );
        }
        assert!(serde_json::from_value::<DomainSetup>(json!({
            "domain":"home.example.com","mode":"sn","certificate":"acme","net_id":"nat",
            "public_ips":[],"owner_name":"alice","extra":1
        }))
        .is_err());
    }

    /// Vectors of cyfs-sn `sn_auth.rs` (`pkx_record_name`, `pkx_value`,
    /// `txt_matches_pkx`, `canonical_user_domain`).
    #[test]
    fn pkx_is_what_the_sn_computes() {
        assert_eq!(pkx_record_name("example.com"), "_pkx.example.com");
        let key: Jwk = serde_json::from_value(
            json!({"crv":"Ed25519","kty":"OKP","x":"T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"}),
        )
        .unwrap();
        assert_eq!(
            pkx_value(&key).unwrap(),
            "PKX(T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8)"
        );
        assert!(txt_matches("  \"PKX(owner-key)\"  ", "PKX(owner-key)"));
        assert!(!txt_matches("PKX(other)", "PKX(owner-key)"));
        assert_eq!(
            canonical_domain("Example.COM.").as_deref(),
            Some("example.com")
        );
        assert_eq!(canonical_domain("   "), None);
        assert_eq!(canonical_domain("."), None);
    }

    #[test]
    fn sn_names_come_from_the_sn_host() {
        let names = SnNames::from_sn_host("sn.buckyos.ai").unwrap();
        assert_eq!(names.name_server(), "dns.buckyos.ai");
        assert_eq!(names.entry("alice"), "alice.web3.buckyos.ai");
        assert_eq!(SnNames::from_sn_host("relay.buckyos.ai"), None);
    }

    #[test]
    fn conflicts_are_found_and_matching_records_are_not() {
        let plan = record_plan(
            &setup(
                "home.example.com",
                DomainHostingMode::Manual,
                NetId::Nat,
                &[],
            ),
            &names(),
            "PKX(x)",
        );
        let entry = "alice.web3.buckyos.ai";
        assert!(find_conflicts(&plan, &[], false).is_empty());
        assert!(find_conflicts(
            &plan,
            &[
                existing(
                    "home.example.com",
                    RecordType::CNAME,
                    &["Alice.web3.buckyos.ai."]
                ),
                existing(
                    "_pkx.home.example.com",
                    RecordType::TXT,
                    &["PKX(someone-else)"]
                ),
                existing("ood1.home.example.com", RecordType::A, &["1.2.3.4"]),
            ],
            false,
        )
        .is_empty());
        let conflicts = find_conflicts(
            &plan,
            &[
                existing("home.example.com", RecordType::A, &["1.2.3.4"]),
                existing("home.example.com", RecordType::TXT, &["v=spf1"]),
                existing(
                    "*.home.example.com",
                    RecordType::CNAME,
                    &["other.example.net"],
                ),
                existing(
                    "_pkx.home.example.com",
                    RecordType::CNAME,
                    &["x.example.net"],
                ),
            ],
            false,
        );
        assert_eq!(
            serde_json::to_value(&conflicts).unwrap(),
            json!([
                {"name":"_pkx.home.example.com","existing":[{"type":"CNAME","values":["x.example.net"]}],
                 "planned":{"type":"TXT","values":["PKX(x)"]}},
                {"name":"home.example.com","existing":[{"type":"A","values":["1.2.3.4"]},{"type":"TXT","values":["v=spf1"]}],
                 "planned":{"type":"CNAME","values":[entry]}},
                {"name":"*.home.example.com","existing":[{"type":"CNAME","values":["other.example.net"]}],
                 "planned":{"type":"CNAME","values":[entry]}},
            ])
        );
    }

    #[test]
    fn address_and_delegation_conflicts() {
        let wan = record_plan(
            &setup(
                "example.com",
                DomainHostingMode::DnsApi,
                NetId::Wan,
                &["203.0.113.7"],
            ),
            &names(),
            "PKX(x)",
        );
        assert!(find_conflicts(
            &wan,
            &[
                existing("example.com", RecordType::A, &["203.0.113.7"]),
                existing("example.com", RecordType::NS, &["ns1.example.net"]),
                existing("example.com", RecordType::TXT, &["v=spf1"]),
            ],
            true,
        )
        .is_empty());
        let conflicts = find_conflicts(
            &wan,
            &[
                existing(
                    "example.com",
                    RecordType::A,
                    &["203.0.113.7", "203.0.113.8"],
                ),
                existing("example.com", RecordType::AAAA, &["2001:db8::7"]),
            ],
            true,
        );
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].existing.len(), 2);

        let ns = record_plan(
            &setup("home.example.com", DomainHostingMode::Sn, NetId::Nat, &[]),
            &names(),
            "PKX(x)",
        );
        let conflicts = find_conflicts(
            &ns,
            &[
                existing("home.example.com", RecordType::A, &["1.2.3.4"]),
                existing("home.example.com", RecordType::NS, &["ns1.example.net"]),
            ],
            false,
        );
        assert_eq!(conflicts[0].existing.len(), 2);
    }
}
