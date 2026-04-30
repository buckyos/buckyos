use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_service_local_data_dir};
use log::*;
use name_lib::{
    decode_jwt_claim_without_verify, DIDDocumentTrait, DeviceConfig, EncodedDocument,
    ZoneBootConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::sync::{mpsc, Notify, RwLock};

use anyhow::{anyhow, Result};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};

const FINDER_PROTOCOL_VERSION: u32 = 1;
const FINDER_SERVER_UDP_PORT: u16 = 2980;
const DEFAULT_RTCP_PORT: u16 = 2980;
const FINDER_CACHE_VERSION: u32 = 1;
const FINDER_CACHE_FILE: &str = "finder_cache.json";
const FINDER_CACHE_TTL_SECS: u64 = 3600 * 24 * 7;
const FINDER_MESSAGE_TTL_SECS: u64 = 60;
const FINDER_BROADCAST_INTERVAL_SECS: u64 = 2;
const FINDER_TARGET_ALL: &str = "*";
const FINDER_REQ_TYPE: &str = "finder_req";
const FINDER_RESP_TYPE: &str = "finder_resp";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookingForReq {
    pub node_id: String,
    pub seq: u64,
    pub iam: String,
}

impl LookingForReq {
    pub fn new(node_id: String, seq: u64, iam: String) -> Self {
        Self { node_id, seq, iam }
    }

    pub fn decode_from_bytes(bytes: &[u8]) -> Result<Self> {
        let req = serde_json::from_slice(bytes)?;
        Ok(req)
    }

    pub fn encode_to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        Ok(bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LookingForResp {
    pub seq: u64,
    pub resp: String,
}

impl LookingForResp {
    pub fn new(seq: u64, resp: String) -> Self {
        Self { seq, resp }
    }

    pub fn decode_from_bytes(bytes: &[u8]) -> Result<Self> {
        let resp = serde_json::from_slice(bytes)?;
        Ok(resp)
    }

    pub fn encode_to_bytes(&self) -> Result<Vec<u8>> {
        let bytes = serde_json::to_vec(self)?;
        Ok(bytes)
    }
}

#[derive(Clone, Debug)]
pub struct DiscoveredNode {
    pub node_id: String,
    pub device_doc: DeviceConfig,
    pub device_doc_jwt: String,
    pub addr: SocketAddr,
    pub rtcp_port: u16,
    pub last_seen: u64,
    pub from_cache: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FinderIdentityClaims {
    version: u32,
    msg_type: String,
    zone_did: String,
    node_id: String,
    target_node_id: Option<String>,
    seq: u64,
    iat: u64,
    exp: u64,
    device_doc_jwt: String,
}

#[derive(Clone, Debug)]
struct VerifiedFinderIdentity {
    claims: FinderIdentityClaims,
    device_doc: DeviceConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinderEndpoint {
    pub ip: IpAddr,
    pub rtcp_port: u16,
    pub seen_at: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinderCacheEntry {
    pub node_id: String,
    pub device_did: String,
    pub device_doc_jwt: String,
    pub net_id: Option<String>,
    pub rtcp_port: Option<u32>,
    pub endpoints: Vec<FinderEndpoint>,
    pub last_seen: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FinderCache {
    pub version: u32,
    pub zone_did: String,
    pub updated_at: u64,
    pub entries: HashMap<String, FinderCacheEntry>,
}

impl FinderCache {
    fn new(zone_did: String) -> Self {
        Self {
            version: FINDER_CACHE_VERSION,
            zone_did,
            updated_at: buckyos_get_unix_timestamp(),
            entries: HashMap::new(),
        }
    }
}

#[derive(Clone)]
struct FinderContext {
    zone_boot_config: ZoneBootConfig,
    owner_public_key: Arc<DecodingKey>,
    this_device_doc: DeviceConfig,
}

pub struct NodeFinder {
    this_device_jwt: String,
    this_device_private_key: EncodingKey,
    context: Option<FinderContext>,
    running: Arc<RwLock<bool>>,
}

impl NodeFinder {
    pub fn new(this_device_jwt: String, device_private_key: EncodingKey) -> Self {
        Self {
            this_device_jwt,
            this_device_private_key: device_private_key,
            context: None,
            running: Arc::new(RwLock::new(false)),
        }
    }

    pub fn new_for_zone(
        this_device_jwt: String,
        device_private_key: EncodingKey,
        zone_boot_config: ZoneBootConfig,
        owner_public_key: DecodingKey,
    ) -> Result<Self> {
        let owner_public_key = Arc::new(owner_public_key);
        let this_device_doc = decode_device_doc(&this_device_jwt, owner_public_key.as_ref())
            .map_err(|err| anyhow!("decode local device doc for finder failed: {}", err))?;
        validate_ood_device(&this_device_doc, &zone_boot_config)?;
        Ok(Self {
            this_device_jwt,
            this_device_private_key: device_private_key,
            context: Some(FinderContext {
                zone_boot_config,
                owner_public_key,
                this_device_doc,
            }),
            running: Arc::new(RwLock::new(false)),
        })
    }

    pub async fn run_udp_server(&self) -> Result<()> {
        let context = self
            .context
            .clone()
            .ok_or_else(|| anyhow!("NodeFinder requires new_for_zone before run_udp_server"))?;

        let socket = UdpSocket::bind(format!("0.0.0.0:{}", FINDER_SERVER_UDP_PORT))
            .await
            .map_err(|e| {
                warn!("bind NodeFinder server error: {}", e);
                anyhow!("bind udp server error: {}", e)
            })?;

        {
            let mut running = self.running.write().await;
            *running = true;
        }

        let this_device_jwt = self.this_device_jwt.clone();
        let this_device_private_key = self.this_device_private_key.clone();
        let running = self.running.clone();
        tokio::spawn(async move {
            info!("NodeFinder server start.");
            let mut buf = [0; 65535];
            loop {
                let running_guard = running.read().await;
                if !*running_guard {
                    info!("Running is false, NodeFinder server will stop.");
                    break;
                }
                drop(running_guard);

                let res = socket.recv_from(&mut buf).await;
                let (size, addr) = match res {
                    Ok(value) => value,
                    Err(err) => {
                        warn!("recv from NodeFinder server error: {}", err);
                        continue;
                    }
                };

                let req = match LookingForReq::decode_from_bytes(&buf[..size]) {
                    Ok(req) => req,
                    Err(err) => {
                        warn!("decode NodeFinder req error: {}", err);
                        continue;
                    }
                };

                let identity = match verify_finder_identity(
                    req.iam.as_str(),
                    FINDER_REQ_TYPE,
                    req.seq,
                    &context.zone_boot_config,
                    context.owner_public_key.as_ref(),
                ) {
                    Ok(identity) => identity,
                    Err(err) => {
                        warn!("reject invalid NodeFinder req from {}: {}", addr, err);
                        continue;
                    }
                };

                if identity.device_doc.name == context.this_device_doc.name {
                    continue;
                }
                if req.node_id != FINDER_TARGET_ALL && req.node_id != context.this_device_doc.name {
                    continue;
                }

                let zone_id = zone_id_string(&context.zone_boot_config);
                let resp_jwt = match build_finder_identity_jwt(
                    FINDER_RESP_TYPE,
                    zone_id.as_str(),
                    context.this_device_doc.name.as_str(),
                    Some(identity.device_doc.name.as_str()),
                    req.seq,
                    this_device_jwt.as_str(),
                    &this_device_private_key,
                ) {
                    Ok(jwt) => jwt,
                    Err(err) => {
                        warn!("build NodeFinder resp failed: {}", err);
                        continue;
                    }
                };

                let resp = LookingForResp::new(req.seq, resp_jwt);
                let resp_bytes = match resp.encode_to_bytes() {
                    Ok(bytes) => bytes,
                    Err(err) => {
                        warn!("encode NodeFinder resp failed: {}", err);
                        continue;
                    }
                };
                if let Err(err) = socket.send_to(&resp_bytes, addr).await {
                    warn!("send NodeFinder resp to {} failed: {}", addr, err);
                }
            }
            info!("NodeFinder server stop.");
        });

        Ok(())
    }

    pub async fn stop_udp_server(&self) {
        let mut running = self.running.write().await;
        *running = false;
        info!("NodeFinder server stop...");
    }
}

pub struct NodeFinderClient {
    this_device_jwt: String,
    this_device_private_key: EncodingKey,
    context: Option<FinderContext>,
    cache_path: PathBuf,
    cache_ttl_secs: u64,
}

impl NodeFinderClient {
    pub fn new(this_device_jwt: String, this_device_private_key: EncodingKey) -> Self {
        Self {
            this_device_jwt,
            this_device_private_key,
            context: None,
            cache_path: default_cache_path(),
            cache_ttl_secs: FINDER_CACHE_TTL_SECS,
        }
    }

    pub fn new_for_zone(
        this_device_jwt: String,
        this_device_private_key: EncodingKey,
        zone_boot_config: ZoneBootConfig,
        owner_public_key: DecodingKey,
    ) -> Result<Self> {
        Self::new_for_zone_inner(
            this_device_jwt,
            this_device_private_key,
            zone_boot_config,
            owner_public_key,
            true,
        )
    }

    // 非 OOD 角色（ZoneGateway / 普通 Node）用来发现局域网内的 OOD。
    // 自己不需要是 OOD，但仍只接受来自 OOD 的应答（cache/响应验证保持严格）。
    pub fn new_as_lan_client(
        this_device_jwt: String,
        this_device_private_key: EncodingKey,
        zone_boot_config: ZoneBootConfig,
        owner_public_key: DecodingKey,
    ) -> Result<Self> {
        Self::new_for_zone_inner(
            this_device_jwt,
            this_device_private_key,
            zone_boot_config,
            owner_public_key,
            false,
        )
    }

    fn new_for_zone_inner(
        this_device_jwt: String,
        this_device_private_key: EncodingKey,
        zone_boot_config: ZoneBootConfig,
        owner_public_key: DecodingKey,
        require_self_ood: bool,
    ) -> Result<Self> {
        let owner_public_key = Arc::new(owner_public_key);
        let this_device_doc = decode_device_doc(&this_device_jwt, owner_public_key.as_ref())
            .map_err(|err| anyhow!("decode local device doc for finder client failed: {}", err))?;
        if require_self_ood {
            validate_ood_device(&this_device_doc, &zone_boot_config)?;
        }
        Ok(Self {
            this_device_jwt,
            this_device_private_key,
            context: Some(FinderContext {
                zone_boot_config,
                owner_public_key,
                this_device_doc,
            }),
            cache_path: default_cache_path(),
            cache_ttl_secs: FINDER_CACHE_TTL_SECS,
        })
    }

    pub fn with_cache_path(mut self, cache_path: PathBuf) -> Self {
        self.cache_path = cache_path;
        self
    }

    pub fn with_cache_ttl_secs(mut self, cache_ttl_secs: u64) -> Self {
        self.cache_ttl_secs = cache_ttl_secs;
        self
    }

    pub fn load_cached_oods(&self) -> Result<HashMap<String, DiscoveredNode>> {
        let context = self.context.as_ref().ok_or_else(|| {
            anyhow!("NodeFinderClient requires new_for_zone before loading cache")
        })?;
        load_valid_cache_entries(
            self.cache_path.as_path(),
            &context.zone_boot_config,
            context.owner_public_key.as_ref(),
            self.cache_ttl_secs,
        )
    }

    pub async fn looking_oods_by_udpv4(
        &self,
        timeout_secs: u64,
    ) -> Result<HashMap<String, DiscoveredNode>> {
        let context = self
            .context
            .as_ref()
            .ok_or_else(|| anyhow!("NodeFinderClient requires new_for_zone before discovery"))?;

        let mut discovered = self.load_cached_oods().unwrap_or_else(|err| {
            warn!("load finder cache failed: {}", err);
            HashMap::new()
        });

        let expected_nodes = expected_ood_names(&context.zone_boot_config)
            .into_iter()
            .filter(|node_id| node_id != &context.this_device_doc.name)
            .collect::<HashSet<_>>();

        if !expected_nodes.is_empty()
            && expected_nodes
                .iter()
                .all(|node_id| discovered.contains_key(node_id))
        {
            return Ok(discovered);
        }

        let broadcast_addrs = Self::get_ipv4_broadcast_addr().await?;
        if broadcast_addrs.is_empty() {
            return Ok(discovered);
        }

        let seq = buckyos_get_unix_timestamp();
        let zone_id = zone_id_string(&context.zone_boot_config);
        let req_jwt = build_finder_identity_jwt(
            FINDER_REQ_TYPE,
            zone_id.as_str(),
            context.this_device_doc.name.as_str(),
            Some(FINDER_TARGET_ALL),
            seq,
            self.this_device_jwt.as_str(),
            &self.this_device_private_key,
        )?;
        let req = LookingForReq::new(FINDER_TARGET_ALL.to_string(), seq, req_jwt);
        let req_bytes = Arc::new(req.encode_to_bytes()?);
        let notify = Arc::new(Notify::new());
        let (tx, mut rx) = mpsc::channel::<(Vec<u8>, SocketAddr)>(32);

        for (ip, broadcast) in broadcast_addrs {
            let socket = UdpSocket::bind(format!("{}:0", ip)).await?;
            socket.set_broadcast(true)?;
            let to_address = format!("{}:{}", broadcast, FINDER_SERVER_UDP_PORT);
            let tx = tx.clone();
            let notify = notify.clone();
            let req_bytes = req_bytes.clone();
            tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(Duration::from_secs(FINDER_BROADCAST_INTERVAL_SECS));
                loop {
                    tokio::select! {
                        _ = notify.notified() => {
                            break;
                        }
                        _ = interval.tick() => {
                            if let Err(err) = socket.send_to(req_bytes.as_slice(), to_address.as_str()).await {
                                warn!("send NodeFinder req to {} failed: {}", to_address, err);
                                break;
                            }
                        }
                        res = recv_udp_packet(&socket) => {
                            match res {
                                Ok((bytes, addr)) => {
                                    if tx.send((bytes, addr)).await.is_err() {
                                        break;
                                    }
                                }
                                Err(err) => {
                                    warn!("recv NodeFinder resp from {} failed: {}", to_address, err);
                                }
                            }
                        }
                    }
                }
            });
        }
        drop(tx);

        let deadline = tokio::time::sleep(Duration::from_secs(timeout_secs));
        tokio::pin!(deadline);

        loop {
            tokio::select! {
                _ = &mut deadline => {
                    notify.notify_waiters();
                    break;
                }
                maybe_packet = rx.recv() => {
                    let Some((bytes, addr)) = maybe_packet else {
                        break;
                    };
                    match self.handle_response_packet(bytes.as_slice(), addr, seq) {
                        Ok(node) => {
                            if node.node_id != context.this_device_doc.name {
                                discovered.insert(node.node_id.clone(), node);
                            }
                            if !expected_nodes.is_empty() && expected_nodes.iter().all(|node_id| discovered.contains_key(node_id)) {
                                notify.notify_waiters();
                                break;
                            }
                        }
                        Err(err) => {
                            debug!("ignore invalid NodeFinder resp from {}: {}", addr, err);
                        }
                    }
                }
            }
        }

        save_finder_cache(
            self.cache_path.as_path(),
            zone_id.as_str(),
            discovered.values(),
        )?;
        Ok(discovered)
    }

    pub async fn looking_by_udpv4(&self, node_id: String, timeout_secs: u64) -> Result<IpAddr> {
        let nodes = self.looking_oods_by_udpv4(timeout_secs).await?;
        nodes
            .get(node_id.as_str())
            .map(|node| node.addr.ip())
            .ok_or_else(|| anyhow!("node {} not found by finder", node_id))
    }

    fn handle_response_packet(
        &self,
        bytes: &[u8],
        addr: SocketAddr,
        seq: u64,
    ) -> Result<DiscoveredNode> {
        let context = self.context.as_ref().ok_or_else(|| {
            anyhow!("NodeFinderClient requires new_for_zone before response verify")
        })?;
        let resp = LookingForResp::decode_from_bytes(bytes)?;
        if resp.seq != seq {
            return Err(anyhow!("response seq mismatch"));
        }
        let identity = verify_finder_identity(
            resp.resp.as_str(),
            FINDER_RESP_TYPE,
            seq,
            &context.zone_boot_config,
            context.owner_public_key.as_ref(),
        )?;
        if identity.claims.target_node_id.as_deref() != Some(context.this_device_doc.name.as_str())
        {
            return Err(anyhow!("response target is not local node"));
        }

        let rtcp_port = identity
            .device_doc
            .rtcp_port
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(DEFAULT_RTCP_PORT);
        Ok(DiscoveredNode {
            node_id: identity.device_doc.name.clone(),
            device_doc_jwt: identity.claims.device_doc_jwt,
            device_doc: identity.device_doc,
            addr: SocketAddr::new(addr.ip(), rtcp_port),
            rtcp_port,
            last_seen: buckyos_get_unix_timestamp(),
            from_cache: false,
        })
    }

    async fn get_ipv4_broadcast_addr() -> Result<Vec<(Ipv4Addr, Ipv4Addr)>> {
        let interfaces = if_addrs::get_if_addrs()?;
        let mut broadcast_addrs = Vec::new();
        for interface in interfaces {
            if interface.is_loopback() {
                continue;
            }

            if let if_addrs::IfAddr::V4(ifv4addr) = interface.addr {
                if let Some(broadcast) = ifv4addr.broadcast {
                    broadcast_addrs.push((ifv4addr.ip, broadcast));
                }
            }
        }
        Ok(broadcast_addrs)
    }
}

async fn recv_udp_packet(socket: &UdpSocket) -> Result<(Vec<u8>, SocketAddr)> {
    let mut buf = vec![0u8; 65535];
    let (size, addr) = socket.recv_from(&mut buf).await?;
    buf.truncate(size);
    Ok((buf, addr))
}

fn default_cache_path() -> PathBuf {
    get_buckyos_service_local_data_dir("node_daemon").join(FINDER_CACHE_FILE)
}

fn build_finder_identity_jwt(
    msg_type: &str,
    zone_did: &str,
    node_id: &str,
    target_node_id: Option<&str>,
    seq: u64,
    device_doc_jwt: &str,
    device_private_key: &EncodingKey,
) -> Result<String> {
    let now = buckyos_get_unix_timestamp();
    let claims = FinderIdentityClaims {
        version: FINDER_PROTOCOL_VERSION,
        msg_type: msg_type.to_string(),
        zone_did: zone_did.to_string(),
        node_id: node_id.to_string(),
        target_node_id: target_node_id.map(ToString::to_string),
        seq,
        iat: now,
        exp: now + FINDER_MESSAGE_TTL_SECS,
        device_doc_jwt: device_doc_jwt.to_string(),
    };
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = None;
    encode(&header, &claims, device_private_key)
        .map_err(|err| anyhow!("encode finder identity jwt failed: {}", err))
}

fn verify_finder_identity(
    jwt: &str,
    expected_msg_type: &str,
    expected_seq: u64,
    zone_boot_config: &ZoneBootConfig,
    owner_public_key: &DecodingKey,
) -> Result<VerifiedFinderIdentity> {
    let claims_value = decode_jwt_claim_without_verify(jwt)?;
    let claims: FinderIdentityClaims = serde_json::from_value(claims_value)?;
    if claims.version != FINDER_PROTOCOL_VERSION {
        return Err(anyhow!(
            "unsupported finder protocol version {}",
            claims.version
        ));
    }
    if claims.msg_type != expected_msg_type {
        return Err(anyhow!("unexpected finder msg_type {}", claims.msg_type));
    }
    if claims.seq != expected_seq {
        return Err(anyhow!("finder seq mismatch"));
    }
    if claims.zone_did != zone_id_string(zone_boot_config) {
        return Err(anyhow!("finder zone mismatch"));
    }
    let now = buckyos_get_unix_timestamp();
    if claims.iat > now + FINDER_MESSAGE_TTL_SECS || claims.exp < now {
        return Err(anyhow!("finder identity jwt expired"));
    }

    let device_doc = decode_device_doc(claims.device_doc_jwt.as_str(), owner_public_key)?;
    if device_doc.name != claims.node_id {
        return Err(anyhow!("finder node_id does not match device doc"));
    }
    validate_ood_device(&device_doc, zone_boot_config)?;

    let (device_public_key, _) = device_doc
        .get_auth_key(None)
        .ok_or_else(|| anyhow!("device doc has no auth key"))?;
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.validate_aud = false;
    decode::<FinderIdentityClaims>(jwt, &device_public_key, &validation)
        .map_err(|err| anyhow!("verify finder identity jwt failed: {}", err))?;

    Ok(VerifiedFinderIdentity { claims, device_doc })
}

fn decode_device_doc(device_doc_jwt: &str, owner_public_key: &DecodingKey) -> Result<DeviceConfig> {
    let encoded_doc = EncodedDocument::from_str(device_doc_jwt.to_string())?;
    let device_doc = DeviceConfig::decode(&encoded_doc, Some(owner_public_key))?;
    Ok(device_doc)
}

fn validate_ood_device(device_doc: &DeviceConfig, zone_boot_config: &ZoneBootConfig) -> Result<()> {
    if !zone_boot_config.device_is_ood(device_doc.name.as_str()) {
        return Err(anyhow!(
            "device {} is not an OOD in ZoneBootConfig",
            device_doc.name
        ));
    }
    if let Some(zone_did) = zone_boot_config.id.as_ref() {
        if device_doc.zone_did.as_ref() != Some(zone_did) {
            return Err(anyhow!("device {} zone_did mismatch", device_doc.name));
        }
    }
    if let Some(owner) = zone_boot_config.owner.as_ref() {
        if &device_doc.owner != owner {
            return Err(anyhow!("device {} owner mismatch", device_doc.name));
        }
    }
    Ok(())
}

fn zone_id_string(zone_boot_config: &ZoneBootConfig) -> String {
    zone_boot_config
        .id
        .as_ref()
        .map(|did| did.to_string())
        .unwrap_or_default()
}

fn expected_ood_names(zone_boot_config: &ZoneBootConfig) -> HashSet<String> {
    zone_boot_config
        .oods
        .iter()
        .filter(|ood| ood.node_type.is_ood())
        .map(|ood| ood.name.clone())
        .collect()
}

fn load_valid_cache_entries(
    cache_path: &Path,
    zone_boot_config: &ZoneBootConfig,
    owner_public_key: &DecodingKey,
    cache_ttl_secs: u64,
) -> Result<HashMap<String, DiscoveredNode>> {
    if !cache_path.exists() {
        return Ok(HashMap::new());
    }
    let cache_str = std::fs::read_to_string(cache_path)?;
    let cache: FinderCache = serde_json::from_str(cache_str.as_str())?;
    if cache.version != FINDER_CACHE_VERSION || cache.zone_did != zone_id_string(zone_boot_config) {
        return Ok(HashMap::new());
    }

    let now = buckyos_get_unix_timestamp();
    let mut result = HashMap::new();
    for entry in cache.entries.values() {
        if entry.last_seen + cache_ttl_secs < now {
            continue;
        }
        let device_doc = match decode_device_doc(entry.device_doc_jwt.as_str(), owner_public_key) {
            Ok(device_doc) => device_doc,
            Err(err) => {
                warn!(
                    "ignore invalid finder cache entry {}: {}",
                    entry.node_id, err
                );
                continue;
            }
        };
        if validate_ood_device(&device_doc, zone_boot_config).is_err() {
            continue;
        }
        let Some(endpoint) = entry
            .endpoints
            .iter()
            .max_by_key(|endpoint| endpoint.seen_at)
        else {
            continue;
        };
        let rtcp_port = entry
            .rtcp_port
            .and_then(|port| u16::try_from(port).ok())
            .unwrap_or(endpoint.rtcp_port);
        result.insert(
            entry.node_id.clone(),
            DiscoveredNode {
                node_id: entry.node_id.clone(),
                device_doc,
                device_doc_jwt: entry.device_doc_jwt.clone(),
                addr: SocketAddr::new(endpoint.ip, rtcp_port),
                rtcp_port,
                last_seen: entry.last_seen,
                from_cache: true,
            },
        );
    }
    Ok(result)
}

fn save_finder_cache<'a>(
    cache_path: &Path,
    zone_did: &str,
    discovered: impl IntoIterator<Item = &'a DiscoveredNode>,
) -> Result<()> {
    let mut cache = FinderCache::new(zone_did.to_string());
    for node in discovered {
        let endpoint = FinderEndpoint {
            ip: node.addr.ip(),
            rtcp_port: node.rtcp_port,
            seen_at: node.last_seen,
        };
        cache.entries.insert(
            node.node_id.clone(),
            FinderCacheEntry {
                node_id: node.node_id.clone(),
                device_did: node.device_doc.id.to_string(),
                device_doc_jwt: node.device_doc_jwt.clone(),
                net_id: node.device_doc.net_id.clone(),
                rtcp_port: node.device_doc.rtcp_port,
                endpoints: vec![endpoint],
                last_seen: node.last_seen,
            },
        );
    }

    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let cache_str = serde_json::to_string_pretty(&cache)?;
    std::fs::write(cache_path, cache_str)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::jwk::Jwk;
    use name_lib::{DeviceNodeType, OODDescriptionString, DID};
    use serde_json::json;

    const OWNER_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIJBRONAzbwpIOwm0ugIQNyZJrDXxZF7HoPWAZesMedOr
-----END PRIVATE KEY-----"#;
    const OWNER_PUBLIC_X: &str = "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8";
    const OOD1_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEIMDp9endjUnT2o4ImedpgvhVFyZEunZqG+ca0mka8oRp
-----END PRIVATE KEY-----"#;
    const OOD1_PUBLIC_X: &str = "gubVIszw-u_d5PVTh-oc8CKAhM9C-ne5G_yUK5BDaXc";
    const OOD2_PRIVATE_KEY: &str = r#"-----BEGIN PRIVATE KEY-----
MC4CAQAwBQYDK2VwBCIEICwMZt1W7P/9v3Iw/rS2RdziVkF7L+o5mIt/WL6ef/0w
-----END PRIVATE KEY-----"#;
    const OOD2_PUBLIC_X: &str = "Bb325f2ed0XSxrPS5sKQaX7ylY9Jh9rfevXiidKA1zc";

    fn jwk(public_x: &str) -> Jwk {
        serde_json::from_value(json!({
            "kty": "OKP",
            "crv": "Ed25519",
            "x": public_x
        }))
        .unwrap()
    }

    fn encoding_key(pem: &str) -> EncodingKey {
        EncodingKey::from_ed_pem(pem.as_bytes()).unwrap()
    }

    fn decoding_key(public_x: &str) -> DecodingKey {
        DecodingKey::from_jwk(&jwk(public_x)).unwrap()
    }

    fn test_zone_boot_config() -> ZoneBootConfig {
        ZoneBootConfig {
            id: Some(DID::new("bns", "alice")),
            owner: Some(DID::new("bns", "alice")),
            owner_key: Some(jwk(OWNER_PUBLIC_X)),
            oods: vec![
                OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, None, None),
                OODDescriptionString::new("ood2".to_string(), DeviceNodeType::OOD, None, None),
            ],
            sn: None,
            exp: buckyos_get_unix_timestamp() + 3600,
            extra_info: HashMap::new(),
        }
    }

    fn signed_device_doc(name: &str, public_x: &str) -> String {
        let mut device_doc = DeviceConfig::new(name, public_x.to_string());
        device_doc.zone_did = Some(DID::new("bns", "alice"));
        device_doc.owner = DID::new("bns", "alice");
        device_doc
            .encode(Some(&encoding_key(OWNER_PRIVATE_KEY)))
            .unwrap()
            .to_string()
    }

    #[test]
    fn finder_identity_requires_owner_signed_ood_device_doc() {
        let zone_boot_config = test_zone_boot_config();
        let ood2_doc_jwt = signed_device_doc("ood2", OOD2_PUBLIC_X);
        let jwt = build_finder_identity_jwt(
            FINDER_RESP_TYPE,
            zone_id_string(&zone_boot_config).as_str(),
            "ood2",
            Some("ood1"),
            42,
            ood2_doc_jwt.as_str(),
            &encoding_key(OOD2_PRIVATE_KEY),
        )
        .unwrap();

        let verified = verify_finder_identity(
            jwt.as_str(),
            FINDER_RESP_TYPE,
            42,
            &zone_boot_config,
            &decoding_key(OWNER_PUBLIC_X),
        )
        .unwrap();
        assert_eq!(verified.device_doc.name, "ood2");
    }

    #[test]
    fn finder_identity_rejects_non_ood_device() {
        let zone_boot_config = test_zone_boot_config();
        let node_doc_jwt = signed_device_doc("node1", OOD2_PUBLIC_X);
        let jwt = build_finder_identity_jwt(
            FINDER_RESP_TYPE,
            zone_id_string(&zone_boot_config).as_str(),
            "node1",
            Some("ood1"),
            42,
            node_doc_jwt.as_str(),
            &encoding_key(OOD2_PRIVATE_KEY),
        )
        .unwrap();

        let err = verify_finder_identity(
            jwt.as_str(),
            FINDER_RESP_TYPE,
            42,
            &zone_boot_config,
            &decoding_key(OWNER_PUBLIC_X),
        )
        .unwrap_err();
        assert!(err.to_string().contains("not an OOD"));
    }

    #[test]
    fn finder_cache_round_trip_keeps_verified_nodes() {
        let temp_dir = std::env::temp_dir().join(format!(
            "buckyos-finder-test-{}",
            buckyos_get_unix_timestamp()
        ));
        let cache_path = temp_dir.join(FINDER_CACHE_FILE);
        let zone_boot_config = test_zone_boot_config();
        let ood2_doc_jwt = signed_device_doc("ood2", OOD2_PUBLIC_X);
        let ood2_doc =
            decode_device_doc(ood2_doc_jwt.as_str(), &decoding_key(OWNER_PUBLIC_X)).unwrap();
        let node = DiscoveredNode {
            node_id: "ood2".to_string(),
            device_doc: ood2_doc,
            device_doc_jwt: ood2_doc_jwt,
            addr: "192.168.1.20:2980".parse().unwrap(),
            rtcp_port: DEFAULT_RTCP_PORT,
            last_seen: buckyos_get_unix_timestamp(),
            from_cache: false,
        };

        save_finder_cache(
            cache_path.as_path(),
            zone_id_string(&zone_boot_config).as_str(),
            [&node],
        )
        .unwrap();
        let cached = load_valid_cache_entries(
            cache_path.as_path(),
            &zone_boot_config,
            &decoding_key(OWNER_PUBLIC_X),
            FINDER_CACHE_TTL_SECS,
        )
        .unwrap();
        let cached_node = cached.get("ood2").unwrap();
        assert!(cached_node.from_cache);
        assert_eq!(
            cached_node.addr.ip(),
            "192.168.1.20".parse::<IpAddr>().unwrap()
        );
        let _ = std::fs::remove_dir_all(temp_dir);
    }

    #[tokio::test]
    async fn test_get_all_local_ipv4_addresses() {
        let ips = NodeFinderClient::get_ipv4_broadcast_addr().await.unwrap();
        for ip in ips {
            println!("ip: {:?}, broadcast: {:?}", ip.0, ip.1);
        }
    }
}
