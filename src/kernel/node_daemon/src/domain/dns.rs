use std::collections::BTreeSet;
use std::net::IpAddr;

use cyfs_dns_provider::{
    find_parent_zone, DnsAnswer, DnsLookup, DnsLookupError, DnsLookupResult, DnsRcode, RecordType,
    ZoneApex,
};
use futures::future::join_all;
use serde::Serialize;

use super::plan::{same_values, ExistingRecord};

/// Servers asked of one zone; enough to see a server that lags behind.
const MAX_SERVERS: usize = 6;

pub fn same_host(left: &str, right: &str) -> bool {
    left.trim_end_matches('.')
        .eq_ignore_ascii_case(right.trim_end_matches('.'))
}

/// Who answers for a domain today.
#[derive(Clone, Debug, PartialEq)]
pub struct Hosting {
    /// The zone the domain's records go into: its own when it is a zone apex,
    /// the enclosing one otherwise (also when it is delegated to the SN).
    pub zone: ZoneApex,
    pub is_apex: bool,
    pub delegated_to_sn: bool,
    /// What each server of the parent zone says the domain is delegated to.
    pub delegation: Vec<(String, BTreeSet<String>)>,
}

/// `Ok(None)`: the domain is not registered.
pub async fn find_hosting(
    lookup: &dyn DnsLookup,
    domain: &str,
    sn_name_server: &str,
) -> DnsLookupResult<Option<Hosting>> {
    let parent = find_parent_zone(lookup, domain).await?;
    let answers = ask_all(lookup, &parent.ns, domain, RecordType::NS).await;
    let mut delegation = Vec::new();
    let mut failures = Vec::new();
    for (server, answer) in answers {
        match answer {
            Ok(answer) => delegation.push((server, name_servers_of(&answer, domain))),
            Err(error) => failures.push(format!("{server}: {error}")),
        }
    }
    if delegation.is_empty() {
        return Err(DnsLookupError::Failed(format!(
            "no name server of {} answered: {}",
            parent.apex,
            failures.join("; ")
        )));
    }
    let delegated: BTreeSet<String> = delegation
        .iter()
        .flat_map(|(_, servers)| servers.iter().cloned())
        .collect();
    let delegated_to_sn = delegated.iter().any(|ns| same_host(ns, sn_name_server));
    if delegated.is_empty() || delegated_to_sn {
        if delegated.is_empty() && !parent.apex.contains('.') {
            return Ok(None);
        }
        return Ok(Some(Hosting {
            zone: parent,
            is_apex: false,
            delegated_to_sn,
            delegation,
        }));
    }
    Ok(Some(Hosting {
        zone: ZoneApex {
            apex: domain.to_string(),
            ns: delegated.into_iter().collect(),
        },
        is_apex: true,
        delegated_to_sn: false,
        delegation,
    }))
}

fn name_servers_of(answer: &DnsAnswer, domain: &str) -> BTreeSet<String> {
    answer
        .answers
        .iter()
        .chain(&answer.authority)
        .filter(|record| record.ty == RecordType::NS && same_host(&record.name, domain))
        .map(|record| record.value.trim_end_matches('.').to_ascii_lowercase())
        .collect()
}

pub async fn ask_all(
    lookup: &dyn DnsLookup,
    servers: &[String],
    name: &str,
    ty: RecordType,
) -> Vec<(String, Result<DnsAnswer, String>)> {
    join_all(servers.iter().take(MAX_SERVERS).map(|server| async move {
        let answer = lookup
            .direct(server, name, ty)
            .await
            .map_err(|error| error.to_string())
            .and_then(|answer| match answer.rcode {
                DnsRcode::NoError | DnsRcode::NxDomain => Ok(answer),
                other => Err(format!("{other:?}")),
            });
        (server.clone(), answer)
    }))
    .await
}

pub async fn ask_first(
    lookup: &dyn DnsLookup,
    servers: &[String],
    name: &str,
    ty: RecordType,
) -> DnsLookupResult<DnsAnswer> {
    let mut last = DnsLookupError::Failed("no name server".to_string());
    for server in servers.iter().take(MAX_SERVERS) {
        match lookup.direct(server, name, ty).await {
            Ok(answer) if matches!(answer.rcode, DnsRcode::NoError | DnsRcode::NxDomain) => {
                return Ok(answer)
            }
            Ok(answer) => last = DnsLookupError::Failed(format!("{server}: {:?}", answer.rcode)),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// Values of `ty` the answer carries for exactly `name`.
pub fn own_values(answer: &DnsAnswer, name: &str, ty: RecordType) -> Vec<String> {
    answer
        .answers
        .iter()
        .chain(
            answer
                .authority
                .iter()
                .filter(|_| ty == RecordType::NS && !answer.authoritative),
        )
        .filter(|record| record.ty == ty && same_host(&record.name, name))
        .map(|record| record.value.clone())
        .collect()
}

pub fn addresses(answer: &DnsAnswer) -> BTreeSet<IpAddr> {
    answer
        .answers
        .iter()
        .filter(|record| matches!(record.ty, RecordType::A | RecordType::AAAA))
        .filter_map(|record| record.value.parse().ok())
        .collect()
}

/// The records written at exactly these names. What a wildcard above a name
/// answers for it is not one of them.
pub async fn existing_records(
    lookup: &dyn DnsLookup,
    hosting: &Hosting,
    names: &[String],
) -> DnsLookupResult<Vec<ExistingRecord>> {
    let types = [
        RecordType::A,
        RecordType::AAAA,
        RecordType::CNAME,
        RecordType::TXT,
        RecordType::NS,
    ];
    let lookups = names
        .iter()
        .flat_map(|name| types.iter().map(move |ty| (name, *ty)))
        .filter(|(name, ty)| !(*ty == RecordType::NS && same_host(name, &hosting.zone.apex)))
        .map(|(name, ty)| async move {
            let answer = ask_first(lookup, &hosting.zone.ns, name, ty).await?;
            let values = own_values(&answer, name, ty);
            if values.is_empty() || from_wildcard(lookup, hosting, name, ty, &values).await? {
                return Ok(None);
            }
            Ok(Some(ExistingRecord {
                name: name.clone(),
                ty,
                values,
            }))
        });
    let mut records = Vec::new();
    for record in join_all(lookups).await {
        if let Some(record) = record? {
            records.push(record);
        }
    }
    Ok(records)
}

async fn from_wildcard(
    lookup: &dyn DnsLookup,
    hosting: &Hosting,
    name: &str,
    ty: RecordType,
    values: &[String],
) -> DnsLookupResult<bool> {
    let mut ancestor = name;
    while !same_host(ancestor, &hosting.zone.apex) {
        let Some((_, parent)) = ancestor.split_once('.') else {
            break;
        };
        ancestor = parent;
        let wildcard = format!("*.{ancestor}");
        if same_host(&wildcard, name) {
            continue;
        }
        let answer = ask_first(lookup, &hosting.zone.ns, &wildcard, ty).await?;
        let found = own_values(&answer, &wildcard, ty);
        if !found.is_empty() && same_values(ty, &found, values) {
            return Ok(true);
        }
    }
    Ok(false)
}

const CLOUDFLARE_RANGES: &[(&str, u8)] = &[
    ("173.245.48.0", 20),
    ("103.21.244.0", 22),
    ("103.22.200.0", 22),
    ("103.31.4.0", 22),
    ("141.101.64.0", 18),
    ("108.162.192.0", 18),
    ("190.93.240.0", 20),
    ("188.114.96.0", 20),
    ("197.234.240.0", 22),
    ("198.41.128.0", 17),
    ("162.158.0.0", 15),
    ("104.16.0.0", 13),
    ("104.24.0.0", 14),
    ("172.64.0.0", 13),
    ("131.0.72.0", 22),
    ("2400:cb00::", 32),
    ("2606:4700::", 32),
    ("2803:f800::", 32),
    ("2405:b500::", 32),
    ("2405:8100::", 32),
    ("2a06:98c0::", 29),
    ("2c0f:f248::", 32),
];

/// An address Cloudflare answers for a record whose proxy is on.
pub fn is_cloudflare_address(ip: IpAddr) -> bool {
    CLOUDFLARE_RANGES.iter().any(|(base, prefix)| {
        let Ok(base) = base.parse::<IpAddr>() else {
            return false;
        };
        match (ip, base) {
            (IpAddr::V4(ip), IpAddr::V4(base)) => {
                let shift = 32 - u32::from(*prefix);
                u32::from(ip) >> shift == u32::from(base) >> shift
            }
            (IpAddr::V6(ip), IpAddr::V6(base)) => {
                let shift = 128 - u32::from(*prefix);
                u128::from(ip) >> shift == u128::from(base) >> shift
            }
            _ => false,
        }
    })
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CaaRecord {
    pub name: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct CaaReport {
    pub allowed: bool,
    pub records: Vec<CaaRecord>,
}

const CA_NAME: &str = "letsencrypt.org";

/// Whether Let's Encrypt may issue `<domain>` and `*.<domain>` (RFC 8659: the
/// closest name upwards that has CAA decides).
pub async fn caa_report(lookup: &dyn DnsLookup, domain: &str) -> DnsLookupResult<CaaReport> {
    let mut name = domain;
    loop {
        let answer = lookup.public(name, RecordType::CAA).await?;
        if !matches!(answer.rcode, DnsRcode::NoError | DnsRcode::NxDomain) {
            return Err(DnsLookupError::Failed(format!(
                "CAA of {name}: {:?}",
                answer.rcode
            )));
        }
        let values = answer.values(RecordType::CAA);
        if !values.is_empty() {
            return Ok(CaaReport {
                allowed: caa_allows(&values),
                records: values
                    .into_iter()
                    .map(|value| CaaRecord {
                        name: name.to_string(),
                        value,
                    })
                    .collect(),
            });
        }
        match name.split_once('.') {
            Some((_, parent)) if parent.contains('.') => name = parent,
            _ => {
                return Ok(CaaReport {
                    allowed: true,
                    records: Vec::new(),
                })
            }
        }
    }
}

fn caa_allows(values: &[String]) -> bool {
    let mut issue = Vec::new();
    let mut issuewild = Vec::new();
    for value in values {
        let mut parts = value.trim().splitn(3, char::is_whitespace);
        let flags = parts.next().unwrap_or_default().parse::<u8>().unwrap_or(0);
        let tag = parts.next().unwrap_or_default().to_ascii_lowercase();
        let value = parts.next().unwrap_or_default().trim().trim_matches('"');
        match tag.as_str() {
            "issue" => issue.push(caa_value_allows(value)),
            "issuewild" => issuewild.push(caa_value_allows(value)),
            "iodef" | "issuemail" | "contactemail" | "contactphone" => {}
            _ if flags & 128 != 0 => return false,
            _ => {}
        }
    }
    let allows = |list: &[bool]| list.is_empty() || list.contains(&true);
    let wildcard = if issuewild.is_empty() {
        &issue
    } else {
        &issuewild
    };
    allows(&issue) && allows(wildcard)
}

fn caa_value_allows(value: &str) -> bool {
    let mut parts = value.split(';').map(str::trim);
    if !parts
        .next()
        .unwrap_or_default()
        .eq_ignore_ascii_case(CA_NAME)
    {
        return false;
    }
    parts.filter(|part| !part.is_empty()).all(|part| {
        let (key, value) = part.split_once('=').unwrap_or((part, ""));
        match key.trim().to_ascii_lowercase().as_str() {
            "validationmethods" => value
                .split(',')
                .any(|method| method.trim().eq_ignore_ascii_case("dns-01")),
            "accounturi" => false,
            _ => true,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::super::testing::FakeDns;
    use super::*;

    #[test]
    fn cloudflare_ranges() {
        for ip in ["104.16.1.2", "172.67.10.1", "2606:4700::6810:1"] {
            assert!(is_cloudflare_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["203.0.113.7", "8.8.8.8", "2001:db8::1"] {
            assert!(!is_cloudflare_address(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn caa_rules() {
        let allows = |values: &[&str]| {
            caa_allows(&values.iter().map(ToString::to_string).collect::<Vec<_>>())
        };
        assert!(allows(&["0 issue \"letsencrypt.org\""]));
        assert!(allows(&[
            "0 issue letsencrypt.org; validationmethods=dns-01"
        ]));
        assert!(allows(&[
            "0 issue \"digicert.com\"",
            "0 issue \"letsencrypt.org\""
        ]));
        assert!(allows(&["0 iodef \"mailto:a@example.com\""]));
        assert!(allows(&[
            "0 issue \"letsencrypt.org\"",
            "0 issuewild \"letsencrypt.org\""
        ]));
        assert!(!allows(&["0 issue \"digicert.com\""]));
        assert!(!allows(&["0 issue \";\""]));
        assert!(!allows(&[
            "0 issue \"letsencrypt.org\"",
            "0 issuewild \"digicert.com\""
        ]));
        assert!(!allows(&["0 issuewild \";\""]));
        assert!(!allows(&[
            "0 issue \"letsencrypt.org; validationmethods=http-01\""
        ]));
        assert!(!allows(&[
            "0 issue \"letsencrypt.org; accounturi=https://x/acct/1\""
        ]));
        assert!(!allows(&["128 tbs \"x\"", "0 issue \"letsencrypt.org\""]));
    }

    #[tokio::test]
    async fn caa_is_read_from_the_closest_name_upwards() {
        let dns = FakeDns::example();
        let report = caa_report(dns.as_ref(), "home.example.com").await.unwrap();
        assert_eq!(
            report,
            CaaReport {
                allowed: true,
                records: vec![]
            }
        );

        dns.add(
            "example.com",
            "example.com",
            RecordType::CAA,
            "0 issue \"digicert.com\"",
        );
        let report = caa_report(dns.as_ref(), "home.example.com").await.unwrap();
        assert!(!report.allowed);
        assert_eq!(report.records[0].name, "example.com");

        dns.add(
            "example.com",
            "home.example.com",
            RecordType::CAA,
            "0 issue \"letsencrypt.org\"",
        );
        let report = caa_report(dns.as_ref(), "home.example.com").await.unwrap();
        assert!(report.allowed);
        assert_eq!(report.records[0].name, "home.example.com");
    }

    #[tokio::test]
    async fn hosting_is_read_at_the_parent() {
        let dns = FakeDns::example();
        let sub = find_hosting(dns.as_ref(), "home.example.com", "dns.buckyos.ai")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sub.zone.apex, "example.com");
        assert!(!sub.is_apex && !sub.delegated_to_sn);

        let apex = find_hosting(dns.as_ref(), "example.com", "dns.buckyos.ai")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(apex.zone.apex, "example.com");
        assert!(apex.is_apex);
        assert_eq!(apex.zone.ns, vec!["ns1.dnshost.net", "ns2.dnshost.net"]);

        dns.delegate("example.com", "home.example.com", &["dns.buckyos.ai"]);
        let delegated = find_hosting(dns.as_ref(), "home.example.com", "dns.buckyos.ai")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delegated.zone.apex, "example.com");
        assert!(delegated.delegated_to_sn && !delegated.is_apex);

        assert_eq!(
            find_hosting(dns.as_ref(), "unregistered.com", "dns.buckyos.ai")
                .await
                .unwrap(),
            None
        );
    }

    #[tokio::test]
    #[ignore = "asks the real DNS"]
    async fn real_dns_smoke() {
        use cyfs_dns_provider::{detect_provider, SystemDnsLookup};
        let lookup = SystemDnsLookup::new();
        let apex = find_hosting(lookup.as_ref(), "cloudflare.com", "dns.buckyos.ai")
            .await
            .unwrap()
            .unwrap();
        println!("{apex:#?}");
        assert!(apex.is_apex);
        let sub = find_hosting(
            lookup.as_ref(),
            "nothing-here.cloudflare.com",
            "dns.buckyos.ai",
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(sub.zone.apex, "cloudflare.com");
        assert!(!sub.is_apex);
        println!(
            "{:?}",
            detect_provider(&sub.zone.ns).map(|provider| provider.id)
        );
        assert_eq!(
            find_hosting(
                lookup.as_ref(),
                "no-such-domain-3f9a1c77b2.com",
                "dns.buckyos.ai"
            )
            .await
            .unwrap(),
            None
        );
        println!(
            "{:?}",
            caa_report(lookup.as_ref(), "www.google.com").await.unwrap()
        );
        let found = existing_records(
            lookup.as_ref(),
            &apex,
            &[
                "cloudflare.com".to_string(),
                "www.cloudflare.com".to_string(),
            ],
        )
        .await
        .unwrap();
        println!("{found:#?}");
    }

    #[tokio::test]
    async fn wildcard_answers_are_not_existing_records() {
        let dns = FakeDns::example();
        dns.add("example.com", "*.example.com", RecordType::A, "1.2.3.4");
        let hosting = find_hosting(dns.as_ref(), "home.example.com", "dns.buckyos.ai")
            .await
            .unwrap()
            .unwrap();
        let names = [
            "home.example.com",
            "*.home.example.com",
            "_pkx.home.example.com",
        ]
        .map(str::to_string);
        assert_eq!(
            existing_records(dns.as_ref(), &hosting, &names)
                .await
                .unwrap(),
            vec![]
        );

        dns.add("example.com", "home.example.com", RecordType::A, "5.6.7.8");
        dns.add(
            "example.com",
            "*.home.example.com",
            RecordType::CNAME,
            "alice.web3.buckyos.ai",
        );
        let found = existing_records(dns.as_ref(), &hosting, &names)
            .await
            .unwrap();
        assert_eq!(
            found
                .iter()
                .map(|r| (r.name.as_str(), r.ty))
                .collect::<Vec<_>>(),
            vec![
                ("home.example.com", RecordType::A),
                ("*.home.example.com", RecordType::CNAME)
            ]
        );
    }
}
