//! `commit_active` from end to end: the SN and BNS are HTTP servers of this
//! process, DNS is `domain::testing`, files go to a temporary directory.

use super::*;
use crate::domain::check::ItemStatus;
use crate::domain::testing::*;
use bns_client::{
    AliasKind, BnsClientError, BnsRpcEnvelope, BnsRpcErrorInfo, DocumentRef, DocumentState,
    DocumentStatus, NameState, NameStatus, OwnerResolution, OwnerSource, Principal, ResolveResult,
};
use cyfs_dns_provider::{DnsLookup, RecordType};
use cyfs_identity_cert::{CertManager, CheckLevel};
use std::sync::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const MNEMONIC: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const DOMAIN: &str = "home.example.com";
const TOKEN: &str = "account-session-token";
const PROVIDER_TOKEN: &str = "provider-secret-token-123";

#[derive(Default)]
struct SnState {
    events: Vec<String>,
    /// Whether the node had its identity when it called.
    identity_at: HashMap<String, bool>,
    user_domain: Option<String>,
    /// `<name>/<doc type>` -> inline document.
    documents: HashMap<String, Vec<u8>>,
    device_did: Option<String>,
    reported: Option<Value>,
    fail_publish: Option<String>,
    refuse_challenge: bool,
}

/// The SN and BNS of one test.
struct Backend {
    state: Mutex<SnState>,
    dns: Arc<FakeDns>,
    etc_dir: PathBuf,
}

impl Backend {
    fn saw(&self, event: &str) {
        let identity = self.etc_dir.join("node_identity.json").exists();
        let mut state = self.state.lock().unwrap();
        state.events.push(event.to_string());
        state.identity_at.insert(event.to_string(), identity);
    }

    fn events(&self) -> Vec<String> {
        self.state.lock().unwrap().events.clone()
    }

    fn count(&self, event: &str) -> usize {
        self.events().iter().filter(|seen| *seen == event).count()
    }

    fn document(&self, doc_type: &str) -> Option<Vec<u8>> {
        self.state
            .lock()
            .unwrap()
            .documents
            .get(&format!("alice/{doc_type}"))
            .cloned()
    }

    fn document_text(&self, doc_type: &str) -> String {
        String::from_utf8(self.document(doc_type).unwrap()).unwrap()
    }

    async fn sn_call(&self, method: &str, params: &Value) -> Result<Value, String> {
        match method {
            "user.get_profile" => {
                let user_domain = self.state.lock().unwrap().user_domain.clone();
                Ok(json!({"code": 0, "name": "alice", "owner_key_bound": true,
                    "user_domain": user_domain, "self_cert": false, "sn_ips": null, "zone_config": ""}))
            }
            "domain.bind" => {
                self.saw("domain.bind");
                let domain = params["domain"].as_str().unwrap().to_string();
                let pkx = format!("_pkx.{domain}");
                let proof = self.dns.public(&pkx, RecordType::TXT).await.unwrap();
                if !proof
                    .values(RecordType::TXT)
                    .iter()
                    .any(|value| *value == pkx_of_alice())
                {
                    return Err(format!(
                        "Parse Request Error: [SN:1016:domain_proof_failed] {}",
                        json!({"domain": domain, "retryable": true})
                    ));
                }
                self.dns.sn_serves(&domain, RELAY_IP);
                self.state.lock().unwrap().user_domain = Some(domain.clone());
                Ok(
                    json!({"code": 0, "domain": domain, "pkx": pkx_of_alice(), "pkx_record_name": pkx,
                    "pkx_source": "bns-owner-config", "verified_at": 1}),
                )
            }
            "bns.publish_document" => {
                let doc_type = params["doc_type"].as_str().unwrap().to_string();
                self.saw(&format!("publish:{doc_type}"));
                let mut state = self.state.lock().unwrap();
                if state.fail_publish.as_deref() == Some(doc_type.as_str()) {
                    state.fail_publish = None;
                    return Err("Failed due to reason: [SN:1025:bns_write_failed] down".to_string());
                }
                let document = match &params["document"] {
                    Value::String(jwt) => jwt.clone().into_bytes(),
                    object => serde_json::to_vec(object).unwrap(),
                };
                let name = params["name"].as_str().unwrap();
                state
                    .documents
                    .insert(format!("{name}/{doc_type}"), document);
                Ok(
                    json!({"code": 0, "request_id": params["request_id"], "operation": "publish_document",
                    "name": name, "controller_id": "c", "controller_address": "0x0",
                    "status": "submitted", "reused": false}),
                )
            }
            "device.register" => {
                self.saw("device.register");
                let mut state = self.state.lock().unwrap();
                state.device_did = params["device_did"].as_str().map(str::to_string);
                state.reported = Some(params.clone());
                Ok(
                    json!({"code": 0, "did": params["device_did"], "zone": "alice",
                    "device_name": params["device_name"], "device_role": "gateway", "state": "online",
                    "public_ips": [], "private_ips": [], "active_endpoints": [],
                    "preferred_endpoint": null, "nat_type": "unknown", "is_wan_device": false,
                    "last_seen_at": 1, "expires_at": 2}),
                )
            }
            "zone.get_info" => {
                self.saw("zone.get_info");
                Ok(
                    json!({"code": 0, "zone": "alice", "bns_name": "alice", "relay_sn": "relay.buckyos.ai",
                    "self_cert": false, "cert_checked_at": null, "cert_expires_at": null,
                    "source_version": "v2", "updated_at": 1}),
                )
            }
            "user.add_dns_record" | "user.remove_dns_record" => {
                self.saw(method);
                let state = self.state.lock().unwrap();
                let name = params["domain"].as_str().unwrap();
                let value = params["record"].as_str().unwrap();
                if state.device_did.is_none()
                    || state.device_did.as_deref() != params["device_did"].as_str()
                    || state.refuse_challenge
                {
                    return Err(
                        "Parse Request Error: [SN:1013:device_permission_denied] device has no permission"
                            .to_string(),
                    );
                }
                let zone = state.user_domain.clone().unwrap();
                if method == "user.add_dns_record" {
                    self.dns.add(&zone, name, RecordType::TXT, value);
                } else {
                    self.dns.remove(&zone, name, RecordType::TXT, Some(value));
                }
                Ok(json!({"code": 0, "device_name": "ood1", "revision": 1, "changed": true}))
            }
            other => panic!("unexpected SN call {other}"),
        }
    }

    fn bns_call(&self, method: &str, params: &Value) -> Value {
        assert_eq!(method, "document.resolve");
        let name = params["name"].as_str().unwrap();
        let doc_type = params["doc_type"].as_str().unwrap();
        let document = self
            .state
            .lock()
            .unwrap()
            .documents
            .get(&format!("{name}/{doc_type}"))
            .cloned();
        let envelope = match document {
            Some(document) => BnsRpcEnvelope::success(resolve_result(name, doc_type, document)),
            None => BnsRpcEnvelope::failure(BnsClientError::Registry(BnsRpcErrorInfo::new(
                "DOCUMENT_NOT_FOUND",
                "document not found",
            ))),
        };
        serde_json::to_value(envelope).unwrap()
    }

    async fn serve(self: Arc<Self>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let backend = self.clone();
                tokio::spawn(async move { backend.answer(stream).await });
            }
        });
        format!("http://{address}")
    }

    async fn answer(&self, mut stream: TcpStream) {
        let Some((path, request)) = read_rpc(&mut stream).await else {
            return;
        };
        let method = request["method"].as_str().unwrap();
        let params = &request["params"];
        let result = if path == "/kapi/bns" {
            Ok(self.bns_call(method, params))
        } else {
            assert!(path.starts_with("/kapi/sn/"), "{path}");
            assert_eq!(request["sys"][1], TOKEN, "{method}");
            self.sn_call(method, params).await
        };
        let sys = json!([request["sys"][0]]);
        let body = match result {
            Ok(value) => json!({"result": value, "sys": sys}),
            Err(error) => json!({"error": error, "sys": sys}),
        }
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes()).await;
    }
}

async fn read_rpc(stream: &mut TcpStream) -> Option<(String, Value)> {
    let mut received = Vec::new();
    loop {
        let mut buffer = [0u8; 8192];
        let count = stream.read(&mut buffer).await.ok()?;
        if count == 0 {
            return None;
        }
        received.extend_from_slice(&buffer[..count]);
        let Some(end) = received.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
            continue;
        };
        let head = String::from_utf8_lossy(&received[..end]).to_string();
        let length = head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))?
            .1
            .trim()
            .parse::<usize>()
            .ok()?;
        if received.len() >= end + 4 + length {
            let path = head.split_whitespace().nth(1)?.to_string();
            let body = serde_json::from_slice(&received[end + 4..end + 4 + length]).ok()?;
            return Some((path, body));
        }
    }
}

fn resolve_result(name: &str, doc_type: &str, document: Vec<u8>) -> ResolveResult {
    let owner = Principal::chain_account("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let zero = "0".to_string();
    ResolveResult {
        name_state: NameState {
            name: name.to_string(),
            asset_owner: owner.value.clone(),
            semantic_owner: Principal::unset(),
            effective_owner: owner.clone(),
            owner_source: OwnerSource::AssetOwnerFallback,
            standard_transfer_enabled: false,
            status: NameStatus::Active,
            registered_at: 1,
            expire_at: 0,
            grace_until: 0,
            updated_at: 1,
            name_seq: 1,
            owner_document_version: 1,
            min_document_iat: 0,
            owner_policy_seq: 0,
            lineage_epoch: 0,
            renewable: true,
            transferable: true,
            allow_delegated_subnames: false,
            namespace_policy_hash: zero.clone(),
            payment_policy_hash: zero.clone(),
            alias_state_hash: zero.clone(),
        },
        document_state: DocumentState {
            name: name.to_string(),
            doc_type: doc_type.to_string(),
            version: 1,
            previous_version: 0,
            status: DocumentStatus::Active,
            document: DocumentRef::inline(document),
            controller: Principal::unset(),
            beneficiary: Principal::unset(),
            payment_target: String::new(),
            valid_from: 0,
            expire_at: 0,
            revoked_at: 0,
            controller_policy_hash: zero.clone(),
            payment_policy_hash: zero.clone(),
            split_policy_hash: zero.clone(),
            price_policy_hash: zero.clone(),
            rights_policy_hash: zero.clone(),
            document_state_hash: zero.clone(),
        },
        owner: OwnerResolution {
            effective_owner: owner.clone(),
            source: OwnerSource::AssetOwnerFallback,
            authority_root: zero.clone(),
            authority_seq: 0,
        },
        effective_controller: owner,
        status: DocumentStatus::Active,
        alias_kind: AliasKind::None,
        alias_target_did: String::new(),
        proof_root: zero,
    }
}

fn pkx_of_alice() -> String {
    let key = owner_of_alice().get_default_key().unwrap();
    crate::domain::plan::pkx_value(&key).unwrap()
}

fn owner_of_alice() -> OwnerDocument {
    let key = derive_bucky_key_from_mnemonic(MNEMONIC, None, 0).unwrap();
    let evm = derive_evm_key_from_mnemonic(MNEMONIC, None, 0).unwrap();
    let mut owner = OwnerDocument::new(
        DID::new("bns", "alice"),
        "alice".to_string(),
        "alice".to_string(),
        serde_json::from_value(key.public_jwk).unwrap(),
    );
    owner.iat = 1_700_000_000;
    owner.wallets.insert(
        "main".to_string(),
        OwnerWallet {
            wallet_type: "eth".to_string(),
            address: evm.address,
        },
    );
    owner
}

/// BNS as the name client sees it.
struct PublishedDevices(Arc<Backend>);

#[async_trait]
impl name_client::Source for PublishedDevices {
    fn id(&self) -> String {
        "bns-of-the-test".to_string()
    }

    async fn get_document(
        &self,
        did: &DID,
        doc_type: &DidDocType,
        _revision: &Revision,
        _opts: &ResolveOpts,
    ) -> name_client::SourceAnswer {
        let answer = name_client::SourceAnswer::new(SourceRef::provider(
            self.id(),
            name_client::SourceTier::Authority,
        ));
        assert_eq!(did.to_string(), "did:bns:ood1.alice");
        match (doc_type, self.0.document("ood1")) {
            (DidDocType::Device, Some(jwt)) => answer.with_body(
                EncodedDocument::Jwt(String::from_utf8(jwt).unwrap()),
                name_client::Carrier::Body,
            ),
            _ => answer,
        }
    }
}

struct Rig {
    root: tempfile::TempDir,
    backend: Arc<Backend>,
    dns: Arc<FakeDns>,
    certificates: Arc<FakeCertificates>,
    server: ActiveServer,
}

impl Rig {
    async fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let paths = ActivationPaths {
            etc_dir: root.path().join("etc"),
            roots: identity_roots_for_buckyos_root(root.path()),
        };
        std::fs::create_dir_all(&paths.etc_dir).unwrap();
        let dns = FakeDns::example();
        let backend = Arc::new(Backend {
            state: Mutex::new(SnState::default()),
            dns: dns.clone(),
            etc_dir: paths.etc_dir.clone(),
        });
        backend.state.lock().unwrap().documents.insert(
            "alice/owner".to_string(),
            serde_json::to_vec(&owner_of_alice()).unwrap(),
        );
        let url = backend.clone().serve().await;
        let bns_url = format!("{url}/kapi/bns");
        let certificates = Arc::new(FakeCertificates::default());
        let domain = DomainService::new(
            SnNames::new("buckyos.ai"),
            dns.clone(),
            Arc::new(FakeProviders {
                provider: FakeProvider::new(&dns, "example.com"),
            }),
            Arc::new(BnsOwnerKeys {
                bns_url: bns_url.clone(),
            }),
            certificates.clone(),
        )
        .with_timing(fast());
        let client = NameClient::new(name_client::NameClientConfig {
            cache_backend: name_client::CacheBackend::Memory,
            enable_zone_locker: false,
            ..Default::default()
        });
        client.set_method_authority("bns", Arc::new(PublishedDevices(backend.clone())));
        client.lock_document(
            DID::new("bns", "alice"),
            DidDocType::Owner,
            EncodedDocument::JsonLd(serde_json::to_value(owner_of_alice()).unwrap()),
            name_client::LockOptions {
                scope: "test".to_string(),
                simulate_history: true,
                ..Default::default()
            },
        );
        let server = ActiveServer::new(
            ActiveServiceConfig {
                sn_base_host: "buckyos.ai".to_string(),
                http_schema: "https".to_string(),
                acme_issuer: None,
            },
            Arc::new(domain),
            ActivationTarget {
                sn_url: format!("{url}/kapi/sn"),
                bns_url,
                paths,
                name_client: Some(Arc::new(client)),
                exit_when_done: false,
            },
        );
        Self {
            root,
            backend,
            dns,
            certificates,
            server,
        }
    }

    /// Signs the documents and builds the request of the page.
    async fn request(
        &self,
        net_id: &str,
        public_ips: &[&str],
        domain: Option<DomainCommitConfig>,
    ) -> CommitActiveReq {
        let owner = owner_of_alice();
        let hostname = match &domain {
            Some(domain) => domain.setup.domain.clone(),
            None => ENTRY.to_string(),
        };
        let names = ActiveNameMapping::derive(&owner, &hostname, domain.is_some());
        let (device_private_key, device_public_key) = generate_ed25519_key_pair();
        let prepared = self
            .server
            .prepare_active_documents(PrepareActiveDocumentsReq {
                owner_document: owner.clone(),
                names,
                topology: GatewayTopology {
                    net_id: net_id.to_string(),
                    rtcp_port: DEFAULT_RTCP_PORT,
                    support_container: installed_product().unwrap().container_enabled(),
                    public_ips: public_ips.iter().map(|ip| ip.parse().unwrap()).collect(),
                    sn_url: "https://sn.buckyos.ai/kapi/sn".to_string(),
                },
                device_public_key,
            })
            .await
            .unwrap();
        let signed_documents = self
            .server
            .sign_web_active_documents(SignWebActiveDocumentsReq {
                mnemonic_words: MNEMONIC.split(' ').map(ToString::to_string).collect(),
                prepared: prepared.clone(),
            })
            .unwrap();
        CommitActiveReq {
            owner_document: owner,
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
                access_token: TOKEN.to_string(),
            },
            domain,
            owner_mnemonic_words: None,
        }
    }

    async fn commit(&self, request: &CommitActiveReq) -> Result<CommitActiveResp, String> {
        // As it arrives over the wire.
        let value = serde_json::to_value(request).unwrap();
        let request = CommitActiveReq::from_json(value).map_err(|error| error.to_string())?;
        self.server
            .commit_active(request)
            .await
            .map_err(|error| error.to_string())
    }

    fn activated(&self) -> bool {
        self.root.path().join("etc/node_identity.json").exists()
    }

    fn start_config(&self) -> String {
        std::fs::read_to_string(self.root.path().join("security/start_config.json"))
            .unwrap_or_default()
    }

    /// Nothing of an activation is on the disk.
    fn assert_untouched(&self, case: &str) {
        let files = walk(self.root.path());
        assert!(files.is_empty(), "{case}: {files:?}");
    }

    fn assert_activated(&self, case: &str, request: &CommitActiveReq, hosting: Option<Value>) {
        let root = self.root.path();
        assert!(self.activated(), "{case}");
        let identity =
            load_local_node_identity_config(&root.join("etc/node_identity.json")).unwrap();
        assert_eq!(identity.zone_did, request.prepared.names.zone_did, "{case}");
        let booted =
            crate::zone_boot_resolve::boot_without_network(&root.join("etc"), &identity).unwrap();
        assert_eq!(booted, request.signed_documents.zone_document, "{case}");
        let start_config = self.start_config();
        let parsed: Value = serde_json::from_str(&start_config).unwrap();
        assert_eq!(
            parsed.get(START_CONFIG_DOMAIN_HOSTING),
            hosting.as_ref(),
            "{case}"
        );
        let key_line = request.device_private_key.lines().nth(1).unwrap();
        for secret in [TOKEN, PROVIDER_TOKEN, key_line, MNEMONIC, "PRIVATE KEY"] {
            assert!(!start_config.contains(secret), "{case}: {secret}");
        }
        for (path, content) in walk(root) {
            let is_credential = path.contains("dns_credentials");
            let is_device_key = content.contains("PRIVATE KEY");
            assert_eq!(
                content.contains(PROVIDER_TOKEN),
                is_credential,
                "{case}: {path}"
            );
            assert!(!content.contains(TOKEN), "{case}: {path}");
            if content.contains(key_line) {
                assert!(
                    is_device_key && path.replace('\\', "/").contains("security/"),
                    "{case}: {path}"
                );
            }
        }
        assert_eq!(
            self.backend.document("zone").unwrap(),
            request.signed_documents.zone_document_jwt.as_bytes(),
            "{case}"
        );
        assert_eq!(
            self.backend.document("ood1").unwrap(),
            request.signed_documents.device_document_jwt.as_bytes(),
            "{case}"
        );
        let owner: OwnerDocument =
            serde_json::from_slice(&self.backend.document("owner").unwrap()).unwrap();
        assert_eq!(
            owner.binded_zone_list[0], request.prepared.names.zone_did,
            "{case}"
        );
    }
}

fn walk(root: &std::path::Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else {
                let content = std::fs::read(&path).unwrap_or_default();
                files.push((
                    path.to_string_lossy().to_string(),
                    String::from_utf8_lossy(&content).to_string(),
                ));
            }
        }
    }
    files
}

fn setup(mode: &str, certificate: &str, net_id: &str, public_ips: &[&str]) -> DomainSetup {
    serde_json::from_value(json!({
        "domain": DOMAIN, "mode": mode, "certificate": certificate, "net_id": net_id,
        "public_ips": public_ips, "owner_name": "alice"
    }))
    .unwrap()
}

fn config(setup: DomainSetup) -> DomainCommitConfig {
    DomainCommitConfig {
        setup,
        credential_id: None,
        certificate_id: None,
        accept_warnings: false,
    }
}

impl Rig {
    async fn check(&self, config: &DomainCommitConfig) -> CheckReport {
        self.server
            .check_domain_records(CheckDomainRecordsReq {
                setup: config.setup.clone(),
                access_token: Some(TOKEN.to_string()),
                credential_id: config.credential_id.clone(),
                certificate_id: config.certificate_id.clone(),
            })
            .await
            .unwrap()
    }

    /// What the user does at their DNS service for the plan.
    async fn write_records(&self, setup: &DomainSetup) {
        let plan = self
            .server
            .plan_domain_records(PlanDomainRecordsReq {
                setup: setup.clone(),
            })
            .await
            .unwrap();
        for record in plan.records {
            for value in record.values {
                if record.ty == RecordType::NS {
                    self.dns.delegate("example.com", &record.name, &[&value]);
                } else {
                    self.dns.add("example.com", &record.name, record.ty, &value);
                }
            }
        }
    }

    async fn upload_certificate(&self) -> String {
        self.server
            .inspect_certificate(InspectCertificateReq {
                domain: DOMAIN.to_string(),
                files: vec!["cGVt".to_string()],
                pfx_password: None,
            })
            .await
            .unwrap()
            .certificate_id
            .unwrap()
    }

    async fn apply_token(&self, setup: &DomainSetup) -> String {
        self.server
            .apply_domain_records(ApplyDomainRecordsReq {
                setup: setup.clone(),
                provider: Some("cloudflare".to_string()),
                fields: Some(BTreeMap::from([(
                    "api_token".to_string(),
                    PROVIDER_TOKEN.to_string(),
                )])),
                endpoint: None,
                credential_id: None,
                confirm: vec![],
            })
            .await
            .unwrap()
            .credential_id
            .unwrap()
    }

    /// TXT, the page binds, then NS, as the page asks for them.
    async fn prepare_sn_hosting(&self) -> DomainCommitConfig {
        let config = config(setup("sn", "acme", "nat", &[]));
        self.dns.add(
            "example.com",
            &format!("_pkx.{DOMAIN}"),
            RecordType::TXT,
            &pkx_of_alice(),
        );
        assert!(!self.check(&config).await.ready);
        self.dns.delegate("example.com", DOMAIN, &[SN_NS]);
        let report = self.check(&config).await;
        assert!(report.ready, "{report:#?}");
        config
    }

    fn challenges_left(&self) -> usize {
        let name = format!("_acme-challenge.{DOMAIN}");
        self.dns.values("example.com", &name, RecordType::TXT).len()
            + if self.backend.state.lock().unwrap().user_domain.is_some() {
                self.dns.values(DOMAIN, &name, RecordType::TXT).len()
            } else {
                0
            }
    }
}

const PUBLISHED: [&str; 3] = ["publish:owner", "publish:zone", "publish:ood1"];

async fn default_hostname() {
    let rig = Rig::new().await;
    let request = rig.request("nat", &[], None).await;
    let response = rig.commit(&request).await.unwrap();
    assert_eq!(response.access_hostname, ENTRY);
    rig.assert_activated("default hostname", &request, None);
    let events = rig.backend.events();
    assert_eq!(
        events,
        [&PUBLISHED[..], &["device.register", "zone.get_info"]].concat()
    );
}

async fn sn_hosting_with_acme() {
    let rig = Rig::new().await;
    let config = rig.prepare_sn_hosting().await;
    assert_eq!(rig.backend.events(), ["domain.bind"]);
    let request = rig.request("nat", &[], Some(config)).await;
    rig.commit(&request).await.unwrap();
    rig.assert_activated(
        "sn hosting",
        &request,
        Some(json!({"schema_version": 1, "mode": "sn", "certificate": "acme", "domain": DOMAIN})),
    );
    let events = rig.backend.events();
    assert_eq!(
        events,
        [
            &["domain.bind"],
            &PUBLISHED[..],
            &[
                "device.register",
                "zone.get_info",
                "user.add_dns_record",
                "user.remove_dns_record"
            ]
        ]
        .concat()
    );
    let seen = rig.backend.state.lock().unwrap().identity_at.clone();
    assert!(seen.values().all(|identity| !identity), "{seen:?}");
    assert_eq!(rig.challenges_left(), 0);
}

async fn sn_hosting_again() {
    let rig = Rig::new().await;
    let config = rig.prepare_sn_hosting().await;
    let first = rig.request("nat", &[], Some(config.clone())).await;
    rig.commit(&first).await.unwrap();
    rig.dns.remove(
        "example.com",
        &format!("_pkx.{DOMAIN}"),
        RecordType::TXT,
        None,
    );
    std::fs::remove_file(rig.root.path().join("etc/node_identity.json")).unwrap();
    let binds = rig.backend.count("domain.bind");
    let again = rig.request("nat", &[], Some(config)).await;
    assert_ne!(
        first.signed_documents.zone_document_jwt,
        again.signed_documents.zone_document_jwt
    );
    rig.commit(&again).await.unwrap();
    assert_eq!(rig.backend.count("domain.bind"), binds);
    rig.assert_activated(
        "sn hosting again",
        &again,
        Some(json!({"schema_version": 1, "mode": "sn", "certificate": "acme", "domain": DOMAIN})),
    );
}

async fn manual_with_an_uploaded_certificate() {
    let rig = Rig::new().await;
    let mut config = config(setup("manual", "upload", "nat", &[]));
    rig.write_records(&config.setup).await;
    config.certificate_id = Some(rig.upload_certificate().await);
    let request = rig.request("nat", &[], Some(config)).await;
    rig.commit(&request).await.unwrap();
    rig.assert_activated(
        "manual",
        &request,
        Some(json!({"schema_version": 1, "mode": "manual", "certificate": "upload", "domain": DOMAIN})),
    );
    for host in [DOMAIN.to_string(), format!("*.{DOMAIN}")] {
        assert_eq!(rig.certificates.manager(&host), Some(CertManager::Manual));
    }
    let events = rig.backend.events();
    assert_eq!(events[0], "domain.bind");
    assert!(!events.iter().any(|event| event.contains("dns_record")));
    assert_eq!(rig.challenges_left(), 0);
}

async fn dns_api_with_acme() {
    let rig = Rig::new().await;
    let mut config = config(setup("dns_api", "acme", "nat", &[]));
    let id = rig.apply_token(&config.setup).await;
    config.credential_id = Some(id.clone());
    let request = rig.request("nat", &[], Some(config)).await;
    rig.commit(&request).await.unwrap();
    rig.assert_activated(
        "dns api",
        &request,
        Some(
            json!({"schema_version": 1, "mode": "dns_api", "certificate": "acme", "domain": DOMAIN,
                    "provider": "cloudflare", "credential_id": id}),
        ),
    );
    let credentials = walk(&rig.root.path().join("security/dns_credentials"));
    assert_eq!(credentials.len(), 1);
    assert!(credentials[0].0.ends_with(&format!("{id}.json")));
    assert_eq!(rig.challenges_left(), 0);
}

/// What is published for the SN to answer from.
async fn access_method(net_id: &'static str, ips: &'static [&'static str]) {
    let rig = Rig::new().await;
    let mut config = config(setup("manual", "upload", net_id, ips));
    rig.write_records(&config.setup).await;
    config.certificate_id = Some(rig.upload_certificate().await);
    let request = rig.request(net_id, ips, Some(config)).await;
    rig.commit(&request).await.unwrap();
    rig.assert_activated(
        net_id,
        &request,
        Some(json!({"schema_version": 1, "mode": "manual", "certificate": "upload", "domain": DOMAIN})),
    );
    let payload = |jwt: String| EncodedDocument::Jwt(jwt).to_json_value().unwrap();
    let zone = payload(rig.backend.document_text("zone"));
    let mini = payload(
        zone["mini_device_jwts"]["ood1"]
            .as_str()
            .unwrap()
            .to_string(),
    );
    let device = payload(rig.backend.document_text("ood1"));
    assert_eq!(zone["sn"], "sn.buckyos.ai", "{net_id}");
    assert_eq!(
        device["ddns_sn_url"], "https://sn.buckyos.ai/kapi/sn",
        "{net_id}"
    );
    assert_eq!(device["net_id"], net_id);
    if net_id == "wan" {
        assert_eq!(zone["gateway_ips"], json!(ips));
        assert_eq!(device["ips"], json!(ips));
    } else {
        assert!(zone.get("gateway_ips").is_none(), "{net_id}");
        assert!(
            device.get("ips").is_none_or(|ips| ips == &json!([])),
            "{net_id}"
        );
    }
    let in_mini = (net_id == "wan_dyn").then(|| json!("wan_dyn"));
    assert_eq!(mini.get("net_id"), in_mini.as_ref(), "{net_id}");
    let reported = rig.backend.state.lock().unwrap().reported.clone().unwrap();
    assert_eq!(reported["device_info"]["net_id"], net_id);
}

async fn not_ready_is_refused() {
    let rig = Rig::new().await;
    let mut manual = config(setup("manual", "upload", "nat", &[]));
    manual.certificate_id = Some(rig.upload_certificate().await);
    let request = rig.request("nat", &[], Some(manual.clone())).await;
    let error = rig.commit(&request).await.unwrap_err();
    assert!(error.contains("[active:domain_not_ready]"), "{error}");
    rig.assert_untouched("no records");

    rig.write_records(&manual.setup).await;
    rig.dns.add(
        "example.com",
        &format!("ood1.{DOMAIN}"),
        RecordType::A,
        "192.0.2.1",
    );
    let error = rig.commit(&request).await.unwrap_err();
    assert!(error.contains("stale_record"), "{error}");
    rig.assert_untouched("stale record");

    let sn = config(setup("sn", "acme", "nat", &[]));
    let request = rig.request("nat", &[], Some(sn)).await;
    let error = rig.commit(&request).await.unwrap_err();
    assert!(error.contains("ns_missing"), "{error}");
    rig.assert_untouched("no delegation");
    assert!(!rig
        .backend
        .events()
        .iter()
        .any(|event| event.starts_with("publish") || event == "device.register"));
}

async fn certificate_warnings_need_the_user() {
    let rig = Rig::new().await;
    let mut manual = config(setup("manual", "upload", "nat", &[]));
    rig.write_records(&manual.setup).await;
    manual.certificate_id = Some(rig.upload_certificate().await);
    *rig.certificates.inspection.lock().unwrap() = Some(inspection(DOMAIN, CheckLevel::Warn));
    let mut request = rig.request("nat", &[], Some(manual)).await;
    let error = rig.commit(&request).await.unwrap_err();
    assert!(
        error.contains("[active:certificate_warning_unconfirmed]"),
        "{error}"
    );
    rig.assert_untouched("warning");
    assert!(rig.backend.events().is_empty());
    request.domain.as_mut().unwrap().accept_warnings = true;
    rig.commit(&request).await.unwrap();
    assert!(rig.activated());
}

async fn a_refused_sn_challenge_can_be_retried() {
    let rig = Rig::new().await;
    let config = rig.prepare_sn_hosting().await;
    let request = rig.request("nat", &[], Some(config)).await;
    rig.backend.state.lock().unwrap().refuse_challenge = true;
    let error = rig.commit(&request).await.unwrap_err();
    assert!(error.contains("[active:sn_challenge_failed]"), "{error}");
    rig.assert_untouched("challenge refused");
    assert_eq!(rig.backend.count("device.register"), 1);

    // Written, but no public resolver sees it.
    rig.backend.state.lock().unwrap().refuse_challenge = false;
    let challenge = format!("_acme-challenge.{DOMAIN}");
    rig.dns.cache_publicly(&challenge, RecordType::TXT, &[]);
    let error = rig.commit(&request).await.unwrap_err();
    assert!(error.contains("challenge_not_visible"), "{error}");
    rig.assert_untouched("challenge not visible");
    assert_eq!(rig.challenges_left(), 0);

    rig.dns.forget_public_cache();
    rig.commit(&request).await.unwrap();
    rig.assert_activated(
        "retried",
        &request,
        Some(json!({"schema_version": 1, "mode": "sn", "certificate": "acme", "domain": DOMAIN})),
    );
    for published in PUBLISHED {
        assert_eq!(rig.backend.count(published), 1, "{published}");
    }
}

async fn a_failed_publish_can_be_retried(failing: &'static str) {
    let rig = Rig::new().await;
    let request = rig.request("nat", &[], None).await;
    rig.backend.state.lock().unwrap().fail_publish = Some(failing.to_string());
    let error = rig.commit(&request).await.unwrap_err();
    assert!(error.contains("bns_write_failed"), "{error}");
    rig.assert_untouched(failing);
    assert_eq!(rig.backend.count("device.register"), 0);
    rig.commit(&request).await.unwrap();
    rig.assert_activated(failing, &request, None);
    for published in PUBLISHED {
        let expected = if published.ends_with(failing) { 2 } else { 1 };
        assert_eq!(
            rig.backend.count(published),
            expected,
            "{failing}: {published}"
        );
    }
}

async fn a_lost_credential_is_named() {
    let rig = Rig::new().await;
    let mut dns_api = config(setup("dns_api", "acme", "nat", &[]));
    rig.apply_token(&dns_api.setup).await;
    for id in [None, Some("home-example-com-00000000".to_string())] {
        dns_api.credential_id = id;
        let request = rig.request("nat", &[], Some(dns_api.clone())).await;
        let error = rig.commit(&request).await.unwrap_err();
        assert!(error.contains("[active:credential_missing]"), "{error}");
        rig.assert_untouched("credential");
    }
    let mut upload = config(setup("sn", "upload", "nat", &[]));
    upload.certificate_id = Some("gone".to_string());
    let request = rig.request("nat", &[], Some(upload)).await;
    let error = rig.commit(&request).await.unwrap_err();
    assert!(error.contains("[active:certificate_missing]"), "{error}");
}

/// A published document is seen again only after the BNS client's read cache
/// expires, so each case takes a while: they run side by side.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn commit_active_end_to_end() {
    let mut cases = tokio::task::JoinSet::new();
    cases.spawn(default_hostname());
    cases.spawn(sn_hosting_with_acme());
    cases.spawn(sn_hosting_again());
    cases.spawn(manual_with_an_uploaded_certificate());
    cases.spawn(dns_api_with_acme());
    for net_id in ["nat", "portmap", "wan_dyn"] {
        cases.spawn(access_method(net_id, &[]));
    }
    cases.spawn(access_method("wan", &["203.0.113.7", "2001:db8::7"]));
    cases.spawn(not_ready_is_refused());
    cases.spawn(certificate_warnings_need_the_user());
    cases.spawn(a_refused_sn_challenge_can_be_retried());
    for failing in ["owner", "zone", "ood1"] {
        cases.spawn(a_failed_publish_can_be_retried(failing));
    }
    cases.spawn(a_lost_credential_is_named());
    while let Some(result) = cases.join_next().await {
        result.unwrap();
    }
}

#[tokio::test]
async fn requests_hide_their_secrets_when_printed() {
    let rig = Rig::new().await;
    let request = rig.request("nat", &[], None).await;
    let key_line = request
        .device_private_key
        .lines()
        .nth(1)
        .unwrap()
        .to_string();
    let mut texts = vec![format!("{request:?}"), format!("{:?}", request.sn)];
    texts.push(format!(
        "{:?}",
        SignWebActiveDocumentsReq {
            mnemonic_words: MNEMONIC.split(' ').map(ToString::to_string).collect(),
            prepared: request.prepared.clone(),
        }
    ));
    texts.push(format!(
        "{:?}",
        DeviceKeyPair {
            public_key: json!({}),
            private_key: request.device_private_key.clone(),
        }
    ));
    texts.push(format!(
        "{:?}",
        rig.server
            .derive_web_owner_material(DeriveWebOwnerMaterialReq {
                mnemonic_words: MNEMONIC.split(' ').map(ToString::to_string).collect(),
            })
            .unwrap()
    ));
    for text in texts {
        for secret in [TOKEN, key_line.as_str(), "abandon", "admin-hash"] {
            assert!(!text.contains(secret), "{secret}: {text}");
        }
    }
}

#[tokio::test]
async fn the_page_sees_an_unreachable_sn_as_such() {
    let rig = Rig::new().await;
    let mut server = rig.server.clone();
    server.target.sn_url = "http://127.0.0.1:1/kapi/sn".to_string();
    let report = server
        .check_domain_records(CheckDomainRecordsReq {
            setup: setup("manual", "upload", "nat", &[]),
            access_token: Some(TOKEN.to_string()),
            credential_id: None,
            certificate_id: None,
        })
        .await
        .unwrap();
    let item = report.item("ownership").unwrap();
    assert_eq!(item.status, ItemStatus::Waiting);
    assert_eq!(item.code.as_deref(), Some("bind_unavailable"));
}

#[tokio::test]
async fn the_owner_document_is_loaded_by_name() {
    let rig = Rig::new().await;
    let loaded = rig
        .server
        .load_owner_document(LoadOwnerDocumentReq {
            owner_name: " Alice ".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(
        loaded["owner_document"],
        serde_json::to_value(owner_of_alice()).unwrap()
    );
    let error = rig
        .server
        .load_owner_document(LoadOwnerDocumentReq {
            owner_name: "nobody".to_string(),
        })
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("[active:owner_not_found]"), "{error}");
}
