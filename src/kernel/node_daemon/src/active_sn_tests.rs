//! T2-5: this node against an unchanged SN (cyfs-gateway
//! `components/cyfs-sn/tests/node_active_commit.py` runs both sides).

use super::*;
use crate::domain::check::ItemStatus;
use crate::domain::testing::*;
use cyfs_dns_provider::{DnsAnswer, DnsLookup, DnsLookupResult, RecordType};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The SN's servers are real, everybody else's are `domain::testing`.
struct SplitDns {
    fake: Arc<FakeDns>,
    sn: String,
    system: Arc<SystemDnsLookup>,
}

#[async_trait]
impl DnsLookup for SplitDns {
    async fn public(&self, name: &str, ty: RecordType) -> DnsLookupResult<DnsAnswer> {
        recurse(self, name, ty).await
    }

    async fn direct(&self, server: &str, name: &str, ty: RecordType) -> DnsLookupResult<DnsAnswer> {
        if server == SN_NS {
            return self.system.direct(&self.sn, name, ty).await;
        }
        self.fake.direct(server, name, ty).await
    }
}

struct World {
    sn_url: String,
    bns_url: String,
    relay_ip: String,
    codes: std::sync::Mutex<Vec<String>>,
    dns: Arc<SplitDns>,
    /// Questions the SN asked its public resolver.
    proofs: Arc<AtomicUsize>,
}

/// dns.google's JSON API over `lookup`, for the SN's proof of `_pkx`.
async fn serve_doh(listener: TcpListener, lookup: Arc<SplitDns>, proofs: Arc<AtomicUsize>) {
    while let Ok((mut stream, _)) = listener.accept().await {
        let (lookup, proofs) = (lookup.clone(), proofs.clone());
        tokio::spawn(async move {
            let mut buffer = vec![0u8; 8192];
            let length = stream.read(&mut buffer).await.unwrap_or(0);
            let head = String::from_utf8_lossy(&buffer[..length]).to_string();
            let target = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            let url = url::Url::parse(&format!("http://doh{target}")).unwrap();
            let name = url
                .query_pairs()
                .find(|(key, _)| key == "name")
                .map(|(_, value)| value.to_string())
                .unwrap_or_default();
            proofs.fetch_add(1, Ordering::Relaxed);
            let body = match lookup.public(&name, RecordType::TXT).await {
                Ok(answer) => json!({
                    "Status": if answer.answers.is_empty() { 3 } else { 0 },
                    "Answer": answer.values(RecordType::TXT).iter()
                        .map(|value| json!({"name": name, "type": 16, "TTL": 60, "data": format!("\"{value}\"")}))
                        .collect::<Vec<_>>()
                }),
                Err(_) => json!({"Status": 2}),
            }
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}

impl World {
    async fn connect() -> Arc<Self> {
        let path = std::env::var("NODE_ACTIVE_SN_FIXTURE").expect("fixture path");
        let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let text = |key: &str| fixture[key].as_str().unwrap().to_string();
        let dns = Arc::new(SplitDns {
            fake: FakeDns::example(),
            sn: text("dns"),
            system: SystemDnsLookup::new(),
        });
        let proofs = Arc::new(AtomicUsize::new(0));
        let doh = url::Url::parse(&text("doh")).unwrap();
        let listener = TcpListener::bind(("127.0.0.1", doh.port().unwrap()))
            .await
            .unwrap();
        tokio::spawn(serve_doh(listener, dns.clone(), proofs.clone()));
        Arc::new(Self {
            sn_url: text("sn_url"),
            bns_url: text("bns_url"),
            relay_ip: text("relay_ip"),
            codes: std::sync::Mutex::new(
                serde_json::from_value(fixture["active_codes"].clone()).unwrap(),
            ),
            dns,
            proofs,
        })
    }

    async fn addresses(&self, name: &str) -> BTreeSet<String> {
        self.dns
            .public(name, RecordType::A)
            .await
            .map(|answer| answer.values(RecordType::A).into_iter().collect())
            .unwrap_or_default()
    }

    /// Every one of `names` answers exactly `ip`, once the SN's caches let it.
    async fn expect_address(&self, case: &str, names: &[String], ip: &str) {
        let started = Instant::now();
        loop {
            let mut wrong = Vec::new();
            for name in names {
                let found = self.addresses(name).await;
                if found != BTreeSet::from([ip.to_string()]) {
                    wrong.push((name.clone(), found));
                }
            }
            if wrong.is_empty() {
                println!("{case}: {names:?} -> {ip} ({:?})", started.elapsed());
                return;
            }
            assert!(
                started.elapsed() < Duration::from_secs(90),
                "{case}: expected {ip}, got {wrong:?}"
            );
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}

/// The device documents of BNS, as the name client asks for them.
struct BnsDevices {
    bns_url: String,
    name: String,
}

#[async_trait]
impl name_client::Source for BnsDevices {
    fn id(&self) -> String {
        "bns-of-the-fixture".to_string()
    }

    async fn get_document(
        &self,
        did: &DID,
        _doc_type: &DidDocType,
        _revision: &Revision,
        _opts: &ResolveOpts,
    ) -> name_client::SourceAnswer {
        let answer = name_client::SourceAnswer::new(SourceRef::provider(
            self.id(),
            name_client::SourceTier::Authority,
        ));
        assert_eq!(did.to_string(), format!("did:bns:ood1.{}", self.name));
        let client = BnsIndexerClient::new_bns_server_url(&self.bns_url, None);
        match client.resolve_document(&self.name, "ood1").await {
            Ok(found) => {
                let jwt = found.document_state.document.inline_document;
                answer.with_body(
                    EncodedDocument::Jwt(String::from_utf8(jwt).unwrap()),
                    name_client::Carrier::Body,
                )
            }
            Err(_) => answer,
        }
    }
}

/// One account with one node.
struct Account {
    world: Arc<World>,
    name: String,
    domain: String,
    mnemonic: String,
    owner: OwnerDocument,
    access_token: String,
    root: tempfile::TempDir,
    server: ActiveServer,
}

impl Account {
    async fn register(world: &Arc<World>, name: &str) -> Self {
        let mnemonic = generate_buckyos_mnemonic().unwrap();
        let key = derive_bucky_key_from_mnemonic(&mnemonic, None, 0).unwrap();
        let evm = derive_evm_key_from_mnemonic(&mnemonic, None, 0).unwrap();
        let mut owner = OwnerDocument::new(
            DID::new("bns", name),
            name.to_string(),
            name.to_string(),
            serde_json::from_value(key.public_jwk).unwrap(),
        );
        owner.wallets.insert(
            "main".to_string(),
            OwnerWallet {
                wallet_type: "eth".to_string(),
                address: evm.address.clone(),
            },
        );
        let code = world.codes.lock().unwrap().pop().unwrap();
        let session = SnClient::new_krpc(&world.sn_url, None)
            .register(cyfs_gateway_api::SnAuthRegisterReq {
                name: name.to_string(),
                email: format!("{name}@example.org"),
                pwd_hash: "12345678".to_string(),
                active_code: code,
                region: Some("test".to_string()),
                request_id: None,
                asset_owner: Some(evm.address),
                owner_config: Some(serde_json::to_value(&owner).unwrap()),
                initial_documents: None,
            })
            .await
            .unwrap();
        let registered = read_owner_document(&world.bns_url, name)
            .await
            .unwrap()
            .expect("registered OwnerDocument");

        let client = NameClient::new(name_client::NameClientConfig {
            cache_backend: name_client::CacheBackend::Memory,
            enable_zone_locker: false,
            ..Default::default()
        });
        client.set_method_authority(
            "bns",
            Arc::new(BnsDevices {
                bns_url: world.bns_url.clone(),
                name: name.to_string(),
            }),
        );
        client.lock_document(
            registered.id.clone(),
            DidDocType::Owner,
            EncodedDocument::JsonLd(serde_json::to_value(&registered).unwrap()),
            name_client::LockOptions {
                scope: "test".to_string(),
                simulate_history: true,
                ..Default::default()
            },
        );

        let root = tempfile::tempdir().unwrap();
        let paths = ActivationPaths {
            etc_dir: root.path().join("etc"),
            roots: identity_roots_for_buckyos_root(root.path()),
        };
        std::fs::create_dir_all(&paths.etc_dir).unwrap();
        let lookup = world.dns.clone();
        let service = DomainService::new(
            SnNames::new("buckyos.ai"),
            lookup.clone(),
            Arc::new(FakeProviders {
                provider: FakeProvider::new(&lookup.fake, "example.com"),
            }),
            Arc::new(BnsOwnerKeys {
                bns_url: world.bns_url.clone(),
            }),
            Arc::new(FakeCertificates::default()),
        )
        .with_timing(crate::domain::service::ChallengeTiming {
            poll: Duration::from_millis(200),
            authoritative: Duration::from_secs(60),
            public: Duration::from_secs(60),
        });
        let server = ActiveServer::new(
            ActiveServiceConfig {
                sn_base_host: "buckyos.ai".to_string(),
                http_schema: "https".to_string(),
                acme_issuer: None,
            },
            Arc::new(service),
            ActivationTarget {
                sn_url: world.sn_url.clone(),
                bns_url: world.bns_url.clone(),
                paths,
                name_client: Some(Arc::new(client)),
                exit_when_done: false,
            },
        );
        Self {
            world: world.clone(),
            name: name.to_string(),
            domain: format!("{name}.example.com"),
            mnemonic,
            owner: registered,
            access_token: session.access_token,
            root,
            server,
        }
    }

    fn setup(&self, mode: &str, certificate: &str, net_id: &str, ips: &[&str]) -> DomainSetup {
        serde_json::from_value(json!({
            "domain": self.domain, "mode": mode, "certificate": certificate, "net_id": net_id,
            "public_ips": ips, "owner_name": self.name
        }))
        .unwrap()
    }

    async fn check(&self, setup: &DomainSetup, credential_id: Option<&str>) -> CheckReport {
        self.server
            .check_domain_records(CheckDomainRecordsReq {
                setup: setup.clone(),
                access_token: Some(self.access_token.clone()),
                credential_id: credential_id.map(str::to_string),
                certificate_id: None,
            })
            .await
            .unwrap()
    }

    async fn request(
        &self,
        net_id: &str,
        ips: &[&str],
        reported: &str,
        domain: Option<DomainCommitConfig>,
    ) -> CommitActiveReq {
        let hostname = match &domain {
            Some(domain) => domain.setup.domain.clone(),
            None => format!("{}.web3.buckyos.ai", self.name),
        };
        let names = ActiveNameMapping::derive(&self.owner, &hostname, domain.is_some());
        let (device_private_key, device_public_key) = generate_ed25519_key_pair();
        let mut prepared = self
            .server
            .prepare_active_documents(PrepareActiveDocumentsReq {
                owner_document: self.owner.clone(),
                names,
                topology: GatewayTopology {
                    net_id: net_id.to_string(),
                    rtcp_port: DEFAULT_RTCP_PORT,
                    support_container: installed_product().unwrap().container_enabled(),
                    public_ips: ips.iter().map(|ip| ip.parse().unwrap()).collect(),
                    sn_url: "https://sn.buckyos.ai/kapi/sn".to_string(),
                },
                device_public_key,
            })
            .await
            .unwrap();
        // The address this machine would report if it were on the Internet.
        prepared.device_info.all_ip = vec![reported.parse().unwrap()];
        let signed_documents = self
            .server
            .sign_web_active_documents(SignWebActiveDocumentsReq {
                mnemonic_words: self.mnemonic.split(' ').map(ToString::to_string).collect(),
                prepared: prepared.clone(),
            })
            .unwrap();
        CommitActiveReq {
            owner_document: self.owner.clone(),
            prepared,
            signed_documents,
            device_private_key,
            system_settings: LocalSystemSettings {
                admin_password_hash: "admin-hash".to_string(),
                guest_access: false,
                friend_passcode: String::new(),
                enabled_features: json!({}),
                host_account: String::new(),
            },
            sn: SnCommitConfig {
                sn_url: "https://sn.buckyos.ai/kapi/sn".to_string(),
                bns_url: "https://bns.buckyos.ai/kapi/bns".to_string(),
                access_token: self.access_token.clone(),
            },
            domain,
            owner_mnemonic_words: None,
        }
    }

    async fn commit(&self, request: &CommitActiveReq) {
        let started = Instant::now();
        self.server
            .commit_active(request.clone())
            .await
            .unwrap_or_else(|error| panic!("{}: {error}", self.name));
        assert!(self.root.path().join("etc/node_identity.json").exists());
        println!("{}: committed in {:?}", self.name, started.elapsed());
    }

    fn device_client(&self, request: &CommitActiveReq) -> (String, SnClient) {
        let key = EncodingKey::from_ed_pem(request.device_private_key.as_bytes()).unwrap();
        let (did, token) = crate::node_daemon::generate_sn_device_credentials(
            &request.prepared.device_document,
            &key,
            &request.signed_documents.zone_document,
        )
        .unwrap();
        (did, SnClient::new_krpc(&self.world.sn_url, Some(token)))
    }

    /// The gateway's report that the node has its own certificate, and what
    /// the node reads back every hour.
    async fn report_self_cert(&self, request: &CommitActiveReq) {
        let (did, sn) = self.device_client(request);
        assert!(
            !sn.get_zone_info().await.unwrap().self_cert,
            "{}",
            self.name
        );
        sn.set_self_cert(true, Some(&did)).await.unwrap();
        let info = sn.get_zone_info().await.unwrap();
        assert!(info.self_cert, "{}", self.name);
        assert_eq!(info.relay_sn.as_deref(), Some("relay.buckyos.ai"));
    }

    async fn report_address(&self, request: &CommitActiveReq, ip: &str) {
        let (did, sn) = self.device_client(request);
        let mut info = request.prepared.device_info.clone();
        info.all_ip = vec![ip.parse().unwrap()];
        sn.update_device_online(build_sn_device_online_report("ood1", &did, &info).unwrap())
            .await
            .unwrap();
    }

    fn names(&self) -> Vec<String> {
        vec![
            self.domain.clone(),
            format!("ood1.{}", self.domain),
            format!("sys.{}", self.domain),
            format!("any-name.{}", self.domain),
            self.web3(),
        ]
    }

    fn web3(&self) -> String {
        format!("{}.web3.buckyos.ai", self.name)
    }

    fn fake(&self) -> &FakeDns {
        &self.world.dns.fake
    }

    fn add_pkx(&self) {
        let key = self.owner.get_default_key().unwrap();
        self.fake().add(
            "example.com",
            &format!("_pkx.{}", self.domain),
            RecordType::TXT,
            &crate::domain::plan::pkx_value(&key).unwrap(),
        );
    }
}

fn config(setup: DomainSetup) -> DomainCommitConfig {
    DomainCommitConfig {
        setup,
        credential_id: None,
        certificate_id: None,
        accept_warnings: false,
    }
}

const FIXED_IP: &str = "203.0.113.7";
const REPORTED_IP: &str = "198.51.100.77";
const MOVED_IP: &str = "198.51.100.78";

async fn sn_hosting(account: Account, net_id: &'static str) {
    let (world, name) = (account.world.clone(), account.name.clone());
    let ips: &[&str] = if net_id == "wan" { &[FIXED_IP] } else { &[] };
    let setup = account.setup("sn", "acme", net_id, ips);

    let found = account
        .server
        .inspect_domain(InspectDomainReq {
            domain: account.domain.clone(),
            net_id: setup.net_id,
            owner_name: name.clone(),
            access_token: Some(account.access_token.clone()),
        })
        .await
        .unwrap();
    assert_eq!(found.bound, Some(false));
    let report = account.check(&setup, None).await;
    assert_eq!(
        report.item("ownership").unwrap().code.as_deref(),
        Some("txt_missing")
    );

    // The SN finds the proof through its public resolver.
    account.add_pkx();
    let report = account.check(&setup, None).await;
    assert_eq!(
        report.item("ownership").unwrap().status,
        ItemStatus::Pass,
        "{report:#?}"
    );
    assert_eq!(
        report.item("delegation").unwrap().code.as_deref(),
        Some("ns_missing")
    );

    account
        .fake()
        .delegate("example.com", &account.domain, &[SN_NS]);
    let report = account.check(&setup, None).await;
    assert!(report.ready, "{name}: {report:#?}");
    // Before there is a device the SN answers the relay.
    world
        .expect_address(&format!("{name} before"), &account.names(), &world.relay_ip)
        .await;

    let request = account
        .request(net_id, ips, REPORTED_IP, Some(config(setup.clone())))
        .await;
    account.commit(&request).await;
    account.report_self_cert(&request).await;
    let expected = match net_id {
        "nat" | "portmap" => world.relay_ip.as_str(),
        "wan_dyn" => REPORTED_IP,
        _ => FIXED_IP,
    };
    world
        .expect_address(&name, &account.names(), expected)
        .await;
    account.report_address(&request, MOVED_IP).await;
    let expected = if net_id == "wan_dyn" {
        MOVED_IP
    } else {
        expected
    };
    world
        .expect_address(&format!("{name} moved"), &account.names(), expected)
        .await;
    let challenge = world
        .dns
        .public(
            &format!("_acme-challenge.{}", account.domain),
            RecordType::TXT,
        )
        .await
        .unwrap();
    assert!(challenge.values(RecordType::TXT).is_empty());

    if net_id != "nat" {
        return;
    }
    // Again with the delegation in place, where `_pkx` cannot be seen.
    account.fake().remove(
        "example.com",
        &format!("_pkx.{}", account.domain),
        RecordType::TXT,
        None,
    );
    let request = account
        .request(net_id, ips, REPORTED_IP, Some(config(setup)))
        .await;
    account.commit(&request).await;
}

async fn default_hostname(account: Account, net_id: &'static str) {
    let (world, name) = (account.world.clone(), account.name.clone());
    let ips: &[&str] = if net_id == "wan" { &[FIXED_IP] } else { &[] };
    let request = account.request(net_id, ips, REPORTED_IP, None).await;
    account.commit(&request).await;
    account.report_self_cert(&request).await;
    let names = [account.web3(), format!("public.{}", account.web3())];
    let expected = match net_id {
        "nat" | "portmap" => world.relay_ip.as_str(),
        "wan_dyn" => REPORTED_IP,
        _ => FIXED_IP,
    };
    world
        .expect_address(&format!("{name} default"), &names[..1], expected)
        .await;
    account.report_address(&request, MOVED_IP).await;
    let expected = if net_id == "wan_dyn" {
        MOVED_IP
    } else {
        expected
    };
    world
        .expect_address(&format!("{name} moved"), &names, expected)
        .await;
}

async fn cname_hosting(account: Account, mode: &'static str, net_id: &'static str) {
    let (world, name) = (account.world.clone(), account.name.clone());
    let ips: &[&str] = if net_id == "wan" { &[FIXED_IP] } else { &[] };
    let certificate = if mode == "manual" { "upload" } else { "acme" };
    let setup = account.setup(mode, certificate, net_id, ips);
    let mut domain = config(setup.clone());
    if mode == "manual" {
        let plan = account
            .server
            .plan_domain_records(PlanDomainRecordsReq {
                setup: setup.clone(),
            })
            .await
            .unwrap();
        for record in plan.records {
            for value in record.values {
                account
                    .fake()
                    .add("example.com", &record.name, record.ty, &value);
            }
        }
        let inspected = account
            .server
            .inspect_certificate(InspectCertificateReq {
                domain: account.domain.clone(),
                files: vec!["cGVt".to_string()],
                pfx_password: None,
            })
            .await
            .unwrap();
        domain.certificate_id = inspected.certificate_id;
    } else {
        let applied = account
            .server
            .apply_domain_records(ApplyDomainRecordsReq {
                setup: setup.clone(),
                provider: Some("cloudflare".to_string()),
                fields: Some(BTreeMap::from([(
                    "api_token".to_string(),
                    TEST_TOKEN.to_string(),
                )])),
                endpoint: None,
                credential_id: None,
                confirm: vec![],
            })
            .await
            .unwrap();
        domain.credential_id = applied.credential_id;
    }
    let request = account
        .request(net_id, ips, REPORTED_IP, Some(domain))
        .await;
    account.commit(&request).await;
    account.report_self_cert(&request).await;
    let (entry, web3) = match net_id {
        "nat" | "portmap" => (world.relay_ip.as_str(), world.relay_ip.as_str()),
        "wan_dyn" => (REPORTED_IP, REPORTED_IP),
        _ => (FIXED_IP, FIXED_IP),
    };
    world
        .expect_address(&name, &account.names()[..4], entry)
        .await;
    world
        .expect_address(&format!("{name} web3"), &[account.web3()], web3)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the SN of cyfs-gateway components/cyfs-sn/tests/node_active_commit.py"]
async fn commits_against_an_unchanged_sn() {
    let world = World::connect().await;
    let mut cases = tokio::task::JoinSet::new();
    for net_id in ["nat", "portmap", "wan_dyn", "wan"] {
        let name = format!("activesn{}", net_id.replace('_', ""));
        let account = Account::register(&world, &name).await;
        cases.spawn(sn_hosting(account, net_id));
    }
    for (mode, net_id) in [
        ("manual", "nat"),
        ("manual", "wan"),
        ("dns_api", "portmap"),
        ("dns_api", "wan_dyn"),
    ] {
        let name = format!("active{}{}", mode.replace('_', ""), net_id.replace('_', ""));
        let account = Account::register(&world, &name).await;
        cases.spawn(cname_hosting(account, mode, net_id));
    }
    for net_id in ["nat", "portmap", "wan_dyn", "wan"] {
        let name = format!("activeweb{}", net_id.replace('_', ""));
        let account = Account::register(&world, &name).await;
        cases.spawn(default_hostname(account, net_id));
    }
    while let Some(result) = cases.join_next().await {
        result.unwrap();
    }
    // One proof per account: re-activation asks for none.
    assert_eq!(world.proofs.load(Ordering::Relaxed), 8);
}
