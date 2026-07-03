// Boot 阶段 Zone Document 的 resolve 与 accept。
//
// 设计依据 doc/arch/BuckyOS Booting阶段 resolve-did-document的特殊处理说明.md：
//  - Resolve 是发现，Accept 是 Boot 状态机决策（§5 / §17.1）；
//  - First Boot：resolve 失败或结果不可接受 => Boot Failed（§8.1）；
//  - Warm Restart：失败 / 不可平滑演进 => 忽略新结果，
//    用 Last Known Good State 继续启动（§8.2 / §9 / §10）；
//  - Boot 永不在线 resolve Owner Document，Root Trust 只来自
//    node_identity（§12 / §13），更新 Owner Document 必须走维护流程（§15）。
//
// name-client 重构后的配套（buckyos-base doc/已有did-resolver介绍.md）：
//  - dns_resolver 已降级为补充源，TXT 候选须 expected_owner + owner 验签才可见
//    （介绍文档 §3 / §8）。Boot 场景 zone 自己的权威发布面还没启动，管线推不出
//    expected_owner（一级名字无结构默认值），因此 resolve_did 管线拿不到 DNS
//    引导文档——这不是缺陷，是介绍文档 §3 要求的安全语义；
//  - Boot 的兜底路径改为直查 DnsProvider，并用 node_identity 里的 owner key
//    在本地闭环验签（介绍文档 §3："本地已经有 Owner Document，验签在本地闭环"）。
//
// 信任纪律：来自网络的 candidate 只接受 Jwt 形态且必须用本地 owner key 验签通过；
// JsonLd 形态结构上不携带签名，在 Boot 阶段一律拒绝（Boot 的信任锚是激活时建立
// 的 Root Trust，不是 HTTPS/CA）。本地文件（debug override / LKGS）与
// node_identity 同级信任，允许 JsonLd，但同样要通过 key material 锚定检查。

use std::path::{Path, PathBuf};
use std::time::Duration;

use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::DecodingKey;
use log::*;
use serde::{Deserialize, Serialize};
use serde_json::json;

use buckyos_api::LocalNodeIdentityConfig;
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_system_etc_dir};
use name_client::{
    resolve_did, update_did_cache, BaseHttpProvider, DnsProvider, NsProvider, GLOBAL_NAME_CLIENT,
};
use name_lib::{
    DIDDocumentTrait, DidDocType, EncodedDocument, OwnerDocument, ZoneBootDocument, ZoneDocument,
    DID,
};

use crate::boot::NodeRole;

const ZONE_LKGS_SCHEMA: &str = "buckyos.zone_lkgs.v1";
// 单步发现超时：web well-known 在 zone 未启动时是 TCP 层黑洞，必须有上限，
// 否则 First Boot 会卡在权威源探测上。
const DISCOVER_STEP_TIMEOUT_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootMode {
    FirstBoot,
    WarmRestart,
}

impl BootMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            BootMode::FirstBoot => "first_boot",
            BootMode::WarmRestart => "warm_restart",
        }
    }
}

// Smooth Upgrade 检查结果（Boot 文档 §17.2）。Incompatible 不是错误，
// 而是"resolve 到了当前启动状态机不能接受的结果"（§11）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmoothUpgradeResult {
    Compatible,
    // candidate 比当前状态旧（回滚保护，§11 "Version / Sequence 不符合预期"）
    IncompatibleVersionRollback,
    // 单 OOD <-> 多 OOD 共识集群是运行模型质变，不能通过 Boot 自动完成（§7）
    IncompatibleConsensusModelChanged,
    // candidate 里 zone 不再有任何 OOD，系统无法运行
    IncompatibleMembershipChange,
    // 本节点在 zone 中的角色发生变化（OOD/Gateway/Node 互转），
    // 必须走受控流程，Boot 不自动接受（§11 "可能导致系统失去管理入口"）
    IncompatibleRoleChange,
}

impl SmoothUpgradeResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            SmoothUpgradeResult::Compatible => "compatible",
            SmoothUpgradeResult::IncompatibleVersionRollback => "version_rollback",
            SmoothUpgradeResult::IncompatibleConsensusModelChanged => "consensus_model_changed",
            SmoothUpgradeResult::IncompatibleMembershipChange => "membership_change",
            SmoothUpgradeResult::IncompatibleRoleChange => "self_role_changed",
        }
    }
}

// Last Known Good State：上一次被 Boot 状态机 Accept 的 Zone Document（§9）。
// 与 name-client 的 did cache 不同，LKGS 是显式 Accept 决策的产物，只增不淘汰，
// 是 Warm Restart 场景的安全堡垒；did cache 只是解析加速。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LastKnownZoneState {
    pub schema: String,
    pub accepted_at: u64,
    pub zone_document: ZoneDocument,
}

fn zone_lkgs_file_path(etc_dir: &Path, zone_did: &DID) -> PathBuf {
    etc_dir.join(format!("{}.zone_lkgs.json", zone_did.to_raw_host_name()))
}

// 比较 Jwk 的关键材料（kty/crv/x），忽略 kid/alg/use 等元数据字段：
// 两份指向同一把 ed25519 公钥的 Jwk 可能携带不同的元数据。
fn jwk_key_material(jwk: &Jwk) -> Option<(String, String, String)> {
    let value = serde_json::to_value(jwk).ok()?;
    let get = |k: &str| {
        value
            .get(k)
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    };
    Some((get("kty")?, get("crv").unwrap_or_default(), get("x")?))
}

// Boot 阶段对 candidate 的核心校验（Resolve 复用能力里的 Verify 部分，§5）：
//  - id 必须是本节点所在 zone；
//  - owner 与 node_identity 一致（缺省则填充）；
//  - 文档声明的 zone 签名 key 必须等于激活时写入 node_identity 的 owner key
//    （Root Trust 锚定；对 Jwt 形态这与验签互为印证，对本地 JsonLd 形态这是唯一锚）。
fn normalize_zone_document_with_local_trust(
    mut zone_document: ZoneDocument,
    node_identity: &LocalNodeIdentityConfig,
) -> Result<ZoneDocument, String> {
    if zone_document.id != node_identity.zone_did {
        return Err(format!(
            "zone document id {} does not match node_identity zone_did {}",
            zone_document.id.to_string(),
            node_identity.zone_did.to_string()
        ));
    }
    if zone_document.owner.is_valid() {
        if zone_document.owner != node_identity.owner_did {
            return Err(format!(
                "zone document owner {} does not match node_identity owner_did {}",
                zone_document.owner.to_string(),
                node_identity.owner_did.to_string()
            ));
        }
    } else {
        zone_document.owner = node_identity.owner_did.clone();
    }

    let default_key = zone_document
        .get_default_key()
        .ok_or_else(|| "zone document's owner key is missing".to_string())?;
    let candidate_material = jwk_key_material(&default_key)
        .ok_or_else(|| "zone document's owner key is not a valid jwk".to_string())?;
    let local_material = jwk_key_material(&node_identity.owner_public_key)
        .ok_or_else(|| "node_identity owner_public_key is not a valid jwk".to_string())?;
    if candidate_material != local_material {
        return Err(
            "zone document's key does not match local root trust (node_identity owner key)"
                .to_string(),
        );
    }
    Ok(zone_document)
}

// 网络来源的 Zone Document candidate：只接受 Jwt + 本地 owner key 验签。
fn decode_signed_zone_document(
    doc: &EncodedDocument,
    node_identity: &LocalNodeIdentityConfig,
    owner_public_key: &DecodingKey,
) -> Result<ZoneDocument, String> {
    match doc {
        EncodedDocument::Jwt(_) => {
            let zone_document = ZoneDocument::decode(doc, Some(owner_public_key))
                .map_err(|err| format!("verify zone document jwt failed: {}", err))?;
            normalize_zone_document_with_local_trust(zone_document, node_identity)
        }
        EncodedDocument::JsonLd(_) => Err(
            "boot resolve only accepts owner-signed jwt zone document from network".to_string(),
        ),
    }
}

// 网络来源的 Zone Boot Document candidate：只接受 Jwt + 本地 owner key 验签，
// 然后无损转换为 ZoneDocument。
fn decode_signed_zone_boot_document(
    doc: &EncodedDocument,
    node_identity: &LocalNodeIdentityConfig,
    owner_public_key: &DecodingKey,
) -> Result<ZoneDocument, String> {
    if !matches!(doc, EncodedDocument::Jwt(_)) {
        return Err(
            "boot resolve only accepts owner-signed jwt zone boot document from network"
                .to_string(),
        );
    }
    let boot_document = ZoneBootDocument::decode(doc, Some(owner_public_key))
        .map_err(|err| format!("verify zone boot document jwt failed: {}", err))?;
    zone_boot_document_to_zone_document(boot_document, &doc.to_string(), node_identity)
}

fn zone_boot_document_to_zone_document(
    mut boot_document: ZoneBootDocument,
    boot_jwt: &str,
    node_identity: &LocalNodeIdentityConfig,
) -> Result<ZoneDocument, String> {
    boot_document.id = Some(node_identity.zone_did.clone());
    if let Some(owner) = boot_document.owner.as_ref() {
        if owner != &node_identity.owner_did {
            return Err(format!(
                "zone boot document owner {} does not match node_identity owner_did {}",
                owner.to_string(),
                node_identity.owner_did.to_string()
            ));
        }
    } else {
        boot_document.owner = Some(node_identity.owner_did.clone());
    }
    boot_document.owner_key = Some(node_identity.owner_public_key.clone());
    let zone_document = boot_document.to_zone_document(&boot_jwt.to_string());
    normalize_zone_document_with_local_trust(zone_document, node_identity)
}

// -------------------- Last Known Good State --------------------

pub fn load_last_known_zone_state(
    node_identity: &LocalNodeIdentityConfig,
) -> Option<LastKnownZoneState> {
    load_last_known_zone_state_in(get_buckyos_system_etc_dir().as_path(), node_identity)
}

fn load_last_known_zone_state_in(
    etc_dir: &Path,
    node_identity: &LocalNodeIdentityConfig,
) -> Option<LastKnownZoneState> {
    let path = zone_lkgs_file_path(etc_dir, &node_identity.zone_did);
    if !path.exists() {
        return None;
    }
    let content = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            warn!(
                "read last known zone state {} failed, treat as first boot: {}",
                path.display(),
                err
            );
            return None;
        }
    };
    let state: LastKnownZoneState = match serde_json::from_str(&content) {
        Ok(state) => state,
        Err(err) => {
            warn!(
                "parse last known zone state {} failed, treat as first boot: {}",
                path.display(),
                err
            );
            return None;
        }
    };
    if state.schema != ZONE_LKGS_SCHEMA {
        warn!(
            "last known zone state {} has unsupported schema '{}', treat as first boot",
            path.display(),
            state.schema
        );
        return None;
    }
    // LKGS 与 node_identity 同级信任，但仍做锚定检查，防 zone 切换 / 重新激活后
    // 读到旧 zone 的残留状态。
    match normalize_zone_document_with_local_trust(state.zone_document.clone(), node_identity) {
        Ok(zone_document) => Some(LastKnownZoneState {
            schema: state.schema,
            accepted_at: state.accepted_at,
            zone_document,
        }),
        Err(err) => {
            warn!(
                "last known zone state {} does not match current node_identity, treat as first boot: {}",
                path.display(),
                err
            );
            None
        }
    }
}

pub fn persist_last_known_zone_state(zone_document: &ZoneDocument) -> std::io::Result<()> {
    persist_last_known_zone_state_in(get_buckyos_system_etc_dir().as_path(), zone_document)
}

fn persist_last_known_zone_state_in(
    etc_dir: &Path,
    zone_document: &ZoneDocument,
) -> std::io::Result<()> {
    let path = zone_lkgs_file_path(etc_dir, &zone_document.id);
    let state = LastKnownZoneState {
        schema: ZONE_LKGS_SCHEMA.to_string(),
        accepted_at: buckyos_get_unix_timestamp(),
        zone_document: zone_document.clone(),
    };
    let body = serde_json::to_string_pretty(&state)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    // 写 tmp 再 rename，避免写一半掉电产生损坏的 LKGS。
    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, body.as_bytes())?;
    std::fs::rename(&tmp_path, &path)?;
    info!("persist last known zone state -> {}", path.display());
    Ok(())
}

// -------------------- Smooth Upgrade 检查（§7 / §17.2） --------------------

pub fn check_zone_smooth_upgrade(
    current: &ZoneDocument,
    candidate: &ZoneDocument,
    device_name: &str,
) -> SmoothUpgradeResult {
    // 回滚保护：candidate 比当前接受的状态旧则忽略。
    if candidate.iat < current.iat {
        return SmoothUpgradeResult::IncompatibleVersionRollback;
    }
    if let (Some(current_seq), Some(candidate_seq)) = (current.version_seq, candidate.version_seq)
    {
        if candidate_seq < current_seq {
            return SmoothUpgradeResult::IncompatibleVersionRollback;
        }
    }

    let current_ood_count = current.oods.iter().filter(|o| o.node_type.is_ood()).count();
    let candidate_ood_count = candidate
        .oods
        .iter()
        .filter(|o| o.node_type.is_ood())
        .count();

    if candidate_ood_count == 0 {
        return SmoothUpgradeResult::IncompatibleMembershipChange;
    }
    // 1 OOD <-> N OOD（N>1）：单机模式与共识集群互转是运行模型质变（§7），
    // 不能通过普通 Boot Resolve 自动完成；多 OOD 之间的数量变化（如 3->5）
    // 可基于 membership change 平滑过渡，放行。
    if (current_ood_count == 1) != (candidate_ood_count == 1) {
        return SmoothUpgradeResult::IncompatibleConsensusModelChanged;
    }

    let current_role = NodeRole::from_zone_document(current, device_name);
    let candidate_role = NodeRole::from_zone_document(candidate, device_name);
    if current_role != candidate_role {
        return SmoothUpgradeResult::IncompatibleRoleChange;
    }

    SmoothUpgradeResult::Compatible
}

// -------------------- 发现层（Resolve，不做接受决策） --------------------

// debug override：`{zone_host}.zone.json` 存在时人工短路网络发现，
// 用于离线环境绕开 DNS（保留原有语义）。文件损坏视为配置错误，直接失败。
fn load_debug_zone_document_override(
    node_identity: &LocalNodeIdentityConfig,
    owner_public_key: &DecodingKey,
) -> Result<Option<ZoneDocument>, String> {
    let path = get_buckyos_system_etc_dir().join(format!(
        "{}.zone.json",
        node_identity.zone_did.to_raw_host_name()
    ));
    if !path.exists() {
        return Ok(None);
    }
    warn!(
        "debug zone document override {} exists, skip network discovery",
        path.display()
    );
    let content = std::fs::read_to_string(&path)
        .map_err(|err| format!("read debug zone document {} failed: {}", path.display(), err))?;
    let encoded_doc = EncodedDocument::from_str(content.trim().to_string())
        .map_err(|err| format!("parse debug zone document {} failed: {}", path.display(), err))?;

    // debug 文件与 node_identity 同级信任：JsonLd 也放行，但同样过锚定检查。
    if let Ok(zone_document) = ZoneDocument::decode(&encoded_doc, Some(owner_public_key)) {
        return normalize_zone_document_with_local_trust(zone_document, node_identity).map(Some);
    }
    let boot_document = ZoneBootDocument::decode(&encoded_doc, Some(owner_public_key))
        .map_err(|err| {
            format!(
                "parse debug zone document {} as zone/boot document failed: {}",
                path.display(),
                err
            )
        })?;
    zone_boot_document_to_zone_document(boot_document, &encoded_doc.to_string(), node_identity)
        .map(Some)
}

async fn discover_step<F>(step_name: &str, fut: F) -> Result<EncodedDocument, String>
where
    F: std::future::Future<Output = name_lib::NSResult<EncodedDocument>>,
{
    match tokio::time::timeout(Duration::from_secs(DISCOVER_STEP_TIMEOUT_SECS), fut).await {
        Ok(Ok(doc)) => Ok(doc),
        Ok(Err(err)) => Err(format!("{}: {}", step_name, err)),
        Err(_) => Err(format!(
            "{}: timeout after {}s",
            step_name, DISCOVER_STEP_TIMEOUT_SECS
        )),
    }
}

// 发现一个可验证的 Zone Document candidate。三条路径按顺序，first-win：
//  1. resolve_did 管线取 Zone Document（did:bns 合约、Warm Restart 时仍在线的
//     权威发布面、did cache 都可能给出回答）；
//  2. resolve_did 管线取 Zone Boot Document（activation 写入的 cache 等）；
//  3. Boot 特殊路径：直查 dns_resolver 取 TXT 引导文档（介绍文档 §3 的 Zone
//     自举场景）。管线的 need_proof 验证在 Boot 场景推不出 expected_owner，
//     这里用本地 owner key 直接闭环验签，不依赖任何额外网络查询。
// 所有路径的产物都必须通过"Jwt + 本地 owner key 验签 + Root Trust 锚定"。
async fn discover_zone_document(
    node_identity: &LocalNodeIdentityConfig,
    owner_public_key: &DecodingKey,
) -> Result<ZoneDocument, String> {
    let zone_did = &node_identity.zone_did;
    let mut failures: Vec<String> = Vec::new();

    match discover_step(
        "resolve_did(zone)",
        resolve_did(zone_did, Some(DidDocType::Zone)),
    )
    .await
    .and_then(|doc| decode_signed_zone_document(&doc, node_identity, owner_public_key))
    {
        Ok(zone_document) => {
            info!("boot discover: got zone document via resolve_did pipeline");
            return Ok(zone_document);
        }
        Err(err) => {
            info!("boot discover: {}", err);
            failures.push(err);
        }
    }

    match discover_step(
        "resolve_did(boot)",
        resolve_did(zone_did, Some(DidDocType::Boot)),
    )
    .await
    .and_then(|doc| decode_signed_zone_boot_document(&doc, node_identity, owner_public_key))
    {
        Ok(zone_document) => {
            info!("boot discover: got zone boot document via resolve_did pipeline");
            return Ok(zone_document);
        }
        Err(err) => {
            info!("boot discover: {}", err);
            failures.push(err);
        }
    }

    let dns_provider = DnsProvider::new(None);
    match discover_step(
        "dns_direct(boot)",
        dns_provider.query_did(zone_did, Some(DidDocType::Boot), None),
    )
    .await
    .and_then(|doc| decode_signed_zone_boot_document(&doc, node_identity, owner_public_key))
    {
        Ok(zone_document) => {
            info!("boot discover: got zone boot document via direct dns query (local-verified)");
            return Ok(zone_document);
        }
        Err(err) => {
            info!("boot discover: {}", err);
            failures.push(err);
        }
    }

    Err(failures.join("; "))
}

// -------------------- Accept 状态机（§16） --------------------

// Boot 阶段解析 Zone Document 的入口。返回 Ok 即"当前节点接受、可用于继续启动
// 的 Zone Document"；返回 Err 即 Boot Failed（只在 First Boot 或 debug 配置
// 损坏时发生）。
pub async fn boot_resolve_zone_document(
    node_identity: &LocalNodeIdentityConfig,
) -> Result<ZoneDocument, String> {
    let owner_public_key = DecodingKey::from_jwk(&node_identity.owner_public_key)
        .map_err(|err| format!("parse owner public key from node_identity failed: {}", err))?;

    // debug override：显式人工短路，不参与 LKGS 演进。
    if let Some(zone_document) =
        load_debug_zone_document_override(node_identity, &owner_public_key)?
    {
        update_zone_document_cache(&zone_document).await;
        return Ok(zone_document);
    }

    let last_known = load_last_known_zone_state(node_identity);
    let boot_mode = if last_known.is_some() {
        BootMode::WarmRestart
    } else {
        BootMode::FirstBoot
    };
    info!(
        "boot resolve zone document: mode={}, zone={}",
        boot_mode.as_str(),
        node_identity.zone_did.to_string()
    );

    let candidate = discover_zone_document(node_identity, &owner_public_key).await;

    let accepted = match (last_known, candidate) {
        (None, Ok(candidate)) => {
            // First Boot：Success + Acceptable => Persist As Known Good State（§16.1）
            info!(
                "first boot: accept zone document (iat={}, version_seq={:?}, oods={})",
                candidate.iat,
                candidate.version_seq,
                candidate.oods.len()
            );
            if let Err(err) = persist_last_known_zone_state(&candidate) {
                warn!("persist last known zone state failed: {}", err);
            }
            candidate
        }
        (None, Err(err)) => {
            // First Boot 失败是合理结果：系统尚未建立任何 Last Known Good State（§8.1）。
            return Err(format!(
                "first boot: resolve zone document failed, boot failed: {}",
                err
            ));
        }
        (Some(state), Ok(candidate)) => {
            if candidate == state.zone_document {
                info!(
                    "warm restart: resolved zone document is identical to last known good state (iat={})",
                    candidate.iat
                );
                candidate
            } else {
                match check_zone_smooth_upgrade(
                    &state.zone_document,
                    &candidate,
                    &node_identity.device_name,
                ) {
                    SmoothUpgradeResult::Compatible => {
                        info!(
                            "warm restart: accept new zone document (current iat={} seq={:?} -> candidate iat={} seq={:?})",
                            state.zone_document.iat,
                            state.zone_document.version_seq,
                            candidate.iat,
                            candidate.version_seq
                        );
                        if let Err(err) = persist_last_known_zone_state(&candidate) {
                            warn!("persist last known zone state failed: {}", err);
                        }
                        candidate
                    }
                    incompatible => {
                        // §17.3：忽略时保留明确日志（当前版本 / candidate 版本 /
                        // 忽略原因 / boot 模式 / 继续使用 LKGS）。
                        warn!(
                            "warm restart: resolved zone document (iat={}, seq={:?}) is NOT smoothly evolvable from last known good state (iat={}, seq={:?}), reason={}. Ignore it and continue booting with last known good state.",
                            candidate.iat,
                            candidate.version_seq,
                            state.zone_document.iat,
                            state.zone_document.version_seq,
                            incompatible.as_str()
                        );
                        state.zone_document
                    }
                }
            }
        }
        (Some(state), Err(err)) => {
            // Warm Restart：resolve 失败不应导致系统不可用（§8.2 / §10）。
            warn!(
                "warm restart: resolve zone document failed ({}). Continue booting with last known good state (accepted_at={}, iat={}, seq={:?}).",
                err,
                state.accepted_at,
                state.zone_document.iat,
                state.zone_document.version_seq
            );
            state.zone_document
        }
    };

    // Accept 之后写入 did cache：让本进程（及 runtime 各组件）后续 resolve
    // 当前 zone 的 did 时有确定回答。cache 只是投影，Accept 决策不依赖它。
    update_zone_document_cache(&accepted).await;
    Ok(accepted)
}

async fn update_zone_document_cache(zone_document: &ZoneDocument) {
    let doc_value = match serde_json::to_value(zone_document) {
        Ok(value) => value,
        Err(err) => {
            warn!("serialize accepted zone document for did cache failed: {}", err);
            return;
        }
    };
    if let Err(err) = update_did_cache(
        zone_document.id.clone(),
        Some(DidDocType::Zone),
        EncodedDocument::JsonLd(doc_value),
    )
    .await
    {
        warn!("update accepted zone document into did cache failed: {}", err);
    }

    // boot_jwt 原文是 owner 签名的 Jwt：写进 Boot doc_type 的 cache 后，下次
    // Warm Restart 断网时管线的 Boot 路径能命中一份可本地验签的回答，
    // 不必落到 LKGS 兜底。
    if !zone_document.boot_jwt.is_empty() {
        if let Err(err) = update_did_cache(
            zone_document.id.clone(),
            Some(DidDocType::Boot),
            EncodedDocument::Jwt(zone_document.boot_jwt.clone()),
        )
        .await
        {
            warn!("update accepted zone boot jwt into did cache failed: {}", err);
        }
    }
}

// -------------------- Root Trust 注入与 zone resolver 接线 --------------------

// 把激活时建立的 Owner（node_identity.owner_public_key）合成为最小 Owner
// Document，注入 name-client 的 local authority override（hosts 语义）。
// 效果：本进程内一切对 owner_did#owner 的解析（包括 need_proof 候选验证的
// owner 递归）都命中本地 Root Trust，Boot 与 Runtime 全程不在线 resolve
// Owner Document（Boot 文档 §12 / §13；Owner 变更须走维护流程，§15）。
pub fn install_local_owner_trust(node_identity: &LocalNodeIdentityConfig) -> Result<(), String> {
    let client = GLOBAL_NAME_CLIENT
        .get()
        .ok_or_else(|| "name lib is not initialized".to_string())?;
    let owner_did = node_identity.owner_did.clone();
    let owner_name = owner_did.id.clone();
    let mut owner_document = OwnerDocument::new(
        owner_did.clone(),
        owner_name.clone(),
        owner_name,
        node_identity.owner_public_key.clone(),
    );
    owner_document.set_default_zone_did(node_identity.zone_did.clone());
    let doc_value = serde_json::to_value(&owner_document)
        .map_err(|err| format!("serialize local owner document failed: {}", err))?;
    client.set_local_authority_override(
        owner_did.clone(),
        DidDocType::Owner,
        EncodedDocument::JsonLd(doc_value),
        "node-daemon-boot-trust",
        None,
    );
    info!(
        "installed local owner trust for {} (boot never resolves owner document online)",
        owner_did.to_string()
    );
    Ok(())
}

// zone_resolver 接线（介绍文档 §5 / §8）：boot 完成后把本机 cyfs-gateway 的
// 3180 端口注册为当前 zone 的权威读取端（zone 内 did 的解析在本地闭环，且能
// 看到不对外发布的文档）。服务端 resolver 接口未就绪时该读取端自然给不出回答，
// 管线按多读取端纪律回退 method 权威源，不产生误判。
pub async fn register_zone_authority_resolver(zone_did: &DID) {
    let Some(client) = GLOBAL_NAME_CLIENT.get() else {
        warn!("register zone authority resolver skipped: name lib is not initialized");
        return;
    };
    match BaseHttpProvider::new_with_config(json!({
        "resolver_host": "127.0.0.1:3180",
        "scheme": "http",
    })) {
        Ok(provider) => {
            client
                .set_zone_authority(zone_did.clone(), Box::new(provider))
                .await;
            info!(
                "registered zone authority resolver for {} -> http://127.0.0.1:3180",
                zone_did.to_string()
            );
        }
        Err(err) => {
            warn!("build zone authority resolver provider failed: {}", err);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use name_lib::OODDescriptionString;

    const TEST_JWK: &str = r#"{"kty":"OKP","crv":"Ed25519","x":"qJdNEtscIYwTo-I0K7iPEt_UZdBDRd4r16jdBfNR0tM"}"#;

    fn test_jwk() -> Jwk {
        serde_json::from_str(TEST_JWK).unwrap()
    }

    fn make_zone_document(iat: u64, seq: Option<u64>, oods: &[&str]) -> ZoneDocument {
        let mut doc = ZoneDocument::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "testowner"),
            test_jwk(),
        );
        doc.iat = iat;
        doc.version_seq = seq;
        doc.oods = oods
            .iter()
            .map(|s| s.parse::<OODDescriptionString>().unwrap())
            .collect();
        doc
    }

    #[test]
    fn smooth_upgrade_accepts_raft_membership_growth() {
        let current = make_zone_document(100, Some(1), &["ood1", "ood2", "ood3"]);
        let candidate = make_zone_document(
            200,
            Some(2),
            &["ood1", "ood2", "ood3", "ood4", "ood5"],
        );
        assert_eq!(
            check_zone_smooth_upgrade(&current, &candidate, "node7"),
            SmoothUpgradeResult::Compatible
        );
    }

    #[test]
    fn smooth_upgrade_rejects_single_to_cluster() {
        let current = make_zone_document(100, Some(1), &["ood1"]);
        let candidate = make_zone_document(200, Some(2), &["ood1", "ood2", "ood3"]);
        assert_eq!(
            check_zone_smooth_upgrade(&current, &candidate, "node7"),
            SmoothUpgradeResult::IncompatibleConsensusModelChanged
        );
        // 反向（集群缩回单机，且 candidate 更新）同样是质变
        let cluster = make_zone_document(100, Some(1), &["ood1", "ood2", "ood3"]);
        let shrink_to_single = make_zone_document(300, Some(3), &["ood1"]);
        assert_eq!(
            check_zone_smooth_upgrade(&cluster, &shrink_to_single, "node7"),
            SmoothUpgradeResult::IncompatibleConsensusModelChanged
        );
    }

    #[test]
    fn smooth_upgrade_rejects_version_rollback() {
        let current = make_zone_document(200, Some(5), &["ood1"]);
        let older_iat = make_zone_document(100, Some(6), &["ood1"]);
        assert_eq!(
            check_zone_smooth_upgrade(&current, &older_iat, "node7"),
            SmoothUpgradeResult::IncompatibleVersionRollback
        );
        let older_seq = make_zone_document(300, Some(4), &["ood1"]);
        assert_eq!(
            check_zone_smooth_upgrade(&current, &older_seq, "node7"),
            SmoothUpgradeResult::IncompatibleVersionRollback
        );
    }

    #[test]
    fn smooth_upgrade_rejects_empty_oods_and_self_role_change() {
        let current = make_zone_document(100, Some(1), &["ood1", "ood2", "ood3"]);
        let no_oods = make_zone_document(200, Some(2), &[]);
        assert_eq!(
            check_zone_smooth_upgrade(&current, &no_oods, "ood1"),
            SmoothUpgradeResult::IncompatibleMembershipChange
        );
        // 本节点 ood1 在 candidate 里被移出 OOD 列表 => 角色变化，拒绝
        let self_removed = make_zone_document(200, Some(2), &["ood2", "ood3", "ood4"]);
        assert_eq!(
            check_zone_smooth_upgrade(&current, &self_removed, "ood1"),
            SmoothUpgradeResult::IncompatibleRoleChange
        );
        // 与本节点无关的成员变化（同为普通 node 视角）=> 平滑
        assert_eq!(
            check_zone_smooth_upgrade(&current, &self_removed, "node9"),
            SmoothUpgradeResult::Compatible
        );
    }

    #[test]
    fn jwk_key_material_ignores_metadata() {
        let a: Jwk = serde_json::from_str(TEST_JWK).unwrap();
        let b: Jwk = serde_json::from_str(
            r##"{"kty":"OKP","crv":"Ed25519","x":"qJdNEtscIYwTo-I0K7iPEt_UZdBDRd4r16jdBfNR0tM","kid":"#main_key","alg":"EdDSA"}"##,
        )
        .unwrap();
        assert_eq!(jwk_key_material(&a), jwk_key_material(&b));
    }

    #[test]
    fn lkgs_roundtrip_and_mismatch_guard() {
        // 用显式 etc_dir 注入，不碰 BUCKYOS_ROOT 全局缓存（并行测试下不可靠，
        // 且会污染真机 etc 目录）。
        let tmp_dir = tempfile::tempdir().unwrap();
        let etc_dir = tmp_dir.path();

        let doc = make_zone_document(100, Some(1), &["ood1"]);
        persist_last_known_zone_state_in(etc_dir, &doc).unwrap();

        let node_identity = LocalNodeIdentityConfig::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "testowner"),
            test_jwk(),
            "ood1".to_string(),
            DID::new("web", "ood1.test.buckyos.io"),
            0,
        );
        let loaded = load_last_known_zone_state_in(etc_dir, &node_identity).unwrap();
        assert_eq!(loaded.zone_document, doc);

        // owner 不匹配的 LKGS 必须被拒绝（zone 重新激活后的残留防护）
        let other_identity = LocalNodeIdentityConfig::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "another-owner"),
            test_jwk(),
            "ood1".to_string(),
            DID::new("web", "ood1.test.buckyos.io"),
            0,
        );
        assert!(load_last_known_zone_state_in(etc_dir, &other_identity).is_none());
    }
}
