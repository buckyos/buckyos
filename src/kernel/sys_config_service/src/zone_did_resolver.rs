// Zone DID Resolver —— "伪装成 resolver 的 zone 内权威 cache"
// （buckyos-base doc/简单介绍resolve-did.md §2.2/§3/§5/§7，速查规则 8；
//   需求整理 notepads/zone-did-resolver需求.md）。
//
// 对外两个入口，语义不同（buckyos-base doc/http_did_resolver_api.md §1.1）：
//   GET /1.0/identifiers/{did}?type={doc_type}   resolver API，回答一律是
//       W3C DID Resolution Result 信封（JWT 文档也在信封里，didDocument 为
//       JWT 字符串），didDocumentMetadata.buckyos.documentStatus 是语义来源；
//   GET /.well-known/{doc_type}[.json|.jwt]      静态发布面，返回裸文档 body
//       （JSON/JSON-LD 或 JWT 原文），不返回信封。did.json 是 W3C did:web
//       兼容入口。需要状态语义（missing/revoked/...）时走 resolver API。
//       注：部署中的 name-client 对 JWT 文档也按 {doc_type}.json 请求并对
//       body 自动识别，所以 .json 与无后缀同为自动识别入口，.jwt 才是强类型。
//
// resolver API 的状态码纪律（客户端按它区分"回答"与"可回退"，是本文件最重要的约束）：
//   200 + documentStatus=active    zone 持有该 (did, doc_type)，独占本次解析；
//   200 + expired/migrated         zone 控制面（resolver/cache）登记的明确回答；
//   404 + documentStatus=missing   zone 对该名字有权威，明确回答"从未发布"——
//                                  是回答不是查不到，客户端会当强负证据；
//   410 + revoked/tombstoned       zone 控制面登记的强负状态，deactivated=true；
//   503                            zone 对该名字没有意见（zone 外名字 / zone 身份
//                                  尚不可用），客户端据此回退本机 cache 与 provider 链；
//   400                            非法入参（key 类 DID 等，规则 10）；
//   501 + historicalQuerySupported=false   带 iat 的历史查询：本 resolver 无历史
//                                  索引，按协议 §7 声明能力缺失，绝不能把当前
//                                  状态伪装成历史快照；
//   500                            zone 内条目损坏。注意：当前 ZoneResolverClient
//                                  把 500 当 zone L1 unknown 回退本机解析（对 zone
//                                  内名字外查通常也解不出来，不会投毒，但也堵不住）。
//                                  若要把"zone 内损坏"升级为独占坏回答，必须服务端、
//                                  客户端、协议文档三方同步改，不能只改一侧。
// 绝不能把"zone 外名字"落进 404/500：404 会被缓存成强负状态。
//
// zone 级 cache / override（需求 §5.4）：SystemConfig 下的控制面登记，
// 优先于结构化查询（devices/* 等），对 zone 外名字构成独占回答：
//   resolver/cache/{escaped_did}/{doc_type}/state   状态 JSON（见 ResolverCacheState 注释）
//   resolver/cache/{escaped_did}/{doc_type}/doc     EncodedDocument 字符串（JWT 或 JSON）
//   resolver/cache/{escaped_did}/{doc_type}/metadata  管理侧自由字段，resolver 不消费
// escaped_did/doc_type 段的转义规则：'%' -> "%25"，'/' -> "%2F"（合法 DID 本身
// 不含 '/'，did:web 的端口写法含 '%'，如 did:web:example.com%3A3000）。
// zone 内短名的 cache 键用规范 DID：did:web:{short}.{zone_host}。
// 写入权限由 sys_config RBAC 承担（policy 是 allow-list：kernel/system 角色与
// root/su_admin 可写，普通 app/users 默认无权限）；写入在 main.rs 记审计日志。
// §6.3：zone 自身的 zone/boot 文档不经过 cache——不能绕过 Boot 状态机
// （LKGS / smooth upgrade）替换 boot/config.zone_document。
//
// Root Trust 保护（需求 §6.1/§6.2）：对"当前 zone owner"的 OwnerDocument 回答，
// key material 永远锚定本地 Root Trust（boot 已接受的 zone_document 默认 key，
// 不直接读 node_identity 文件），权威源只能贡献非密钥的 profile 字段；
// owner key 变更必须走维护/recovery 流程，不能经普通 resolve/写库完成。
//
// 尚未实现（有真实需求时再补）：Cache-Control 负状态缓存策略、按请求来源的
// doc_type 可见性分级（需求 §7，需 gateway 注入来源信息后一起设计）。

#![allow(unused)]

use log::*;
use name_lib::*;

use async_trait::async_trait;
use buckyos_api::ZoneConfig;
use buckyos_http_server::{
    server_err, HttpServer, ServerError, ServerErrorCode, ServerResult, StreamInfo,
};
use bytes::Bytes;
use http::{Method, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use jsonwebtoken::jwk::Jwk;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use url::form_urlencoded;

use crate::SYS_STORE;

const AGENT_DOC_TYPE: &str = "agent";
const CONTENT_TYPE_RESOLUTION: &str = "application/did-resolution+json";
const CONTENT_TYPE_JSON: &str = "application/json";
const CONTENT_TYPE_DID_JWT: &str = "application/did+jwt";
const CONTENT_TYPE_DID_JSON: &str = "application/did+ld+json";
const RESOLVER_CACHE_PREFIX: &str = "resolver/cache";

#[derive(Clone)]
pub struct ZoneDidResolver {}

// zone 身份快照：boot/config（ZoneConfig）里 zone_document 解出的权威事实。
struct ZoneIdentity {
    zone_doc: ZoneDocument,
    // 原始 zone_document。能整体解码为 ZoneDocument 时优先原样返回（保留签名）；
    // 只存了 ZoneBootDocument jwt 时返回重建的 ZoneDocument JSON。
    raw: EncodedDocument,
    raw_is_zone_doc: bool,
}

impl ZoneIdentity {
    fn hostname(&self) -> String {
        if !self.zone_doc.hostname.is_empty() {
            return self.zone_doc.hostname.clone();
        }
        self.zone_doc.id.to_host_name()
    }

    // 本地 Root Trust 的 key material：boot 状态机已接受并写入 boot/config 的
    // zone_document 默认 key（激活时由 node_identity.owner_public_key 建立）。
    // sys_config 不直接读 node_identity 文件——避免把 Root Trust 文件路径
    // 当普通运行时依赖暴露（需求 §11）。
    fn local_root_trust_key(&self) -> Option<Jwk> {
        self.zone_doc.get_default_key()
    }

    fn is_zone_owner(&self, did: &DID) -> bool {
        self.zone_doc.owner.is_valid() && *did == self.zone_doc.owner
    }

    // zone owner 在 zone 内的约定短名（users/{owner.id}/doc）。
    fn is_zone_owner_short(&self, short: &str) -> bool {
        self.zone_doc.owner.is_valid() && short == self.zone_doc.owner.id
    }
}

// 解析目标三分。Foreign 不代表拒答：cache override 或 store 里恰好持有该主体的
// 文档且文档自述 id 与查询一致时仍可回答，否则 503。
enum Target {
    ZoneItself,
    InZone(String),
    Foreign(DID),
}

// 已发布状态（missing 单独用 ZoneAnswer::Missing 表达）。
// HTTP 映射与 deactivated 语义见协议 §4。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PublishedStatus {
    Active,
    Expired,
    Revoked,
    Tombstoned,
    Migrated,
}

impl PublishedStatus {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Expired => "expired",
            Self::Revoked => "revoked",
            Self::Tombstoned => "tombstoned",
            Self::Migrated => "migrated",
        }
    }

    fn parse(raw: &str) -> Option<Self> {
        match raw {
            "active" => Some(Self::Active),
            "expired" => Some(Self::Expired),
            "revoked" => Some(Self::Revoked),
            "tombstoned" => Some(Self::Tombstoned),
            "migrated" => Some(Self::Migrated),
            _ => None,
        }
    }

    fn http_status(&self) -> StatusCode {
        match self {
            Self::Active | Self::Expired | Self::Migrated => StatusCode::OK,
            Self::Revoked | Self::Tombstoned => StatusCode::GONE,
        }
    }

    fn deactivated(&self) -> bool {
        matches!(self, Self::Revoked | Self::Tombstoned)
    }
}

// 一次"已发布"回答的全部信封素材。
struct PublishedAnswer {
    status: PublishedStatus,
    // Active 必须内联文档（zone 内 ZoneResolverClient 不支持 anchor-only 回答），
    // 其余状态可以只有状态没有 body。
    document: Option<EncodedDocument>,
    doc_type: DidDocType,
    version: Option<u64>,
    effective_owner: Option<DID>,
    authority_seq: Option<u64>,
    // Some = 已钉死的锚点（cache state 提供）；None 且 with_hash 时对返回 body 现算。
    doc_hash: Option<String>,
    // Info 这类实时数据没有"已发布 body"语义，不带 docHash 锚点
    with_hash: bool,
    migration_target: Option<DID>,
}

// 一次解析的回答，与文件头的状态码纪律一一对应。
enum ZoneAnswer {
    Published(PublishedAnswer),
    // legacy 别名 `self` 专用：裸 ZoneDocument JSON（websdk 等旧消费者读顶层
    // hostname/id），resolve_did 客户端不会用非 DID 入参查询
    Bare(Value),
    Missing(DidDocType),
    NoOpinion(String),
    BadRequest(String),
    // 带 iat 的历史查询：本 resolver 无历史索引（协议 §7 的 501 分支）
    HistoricalNotSupported(String),
    Internal(String),
}

fn is_key_class_method(method: &str) -> bool {
    method == "dev" || method == "key"
}

impl ZoneDidResolver {
    pub fn new() -> Self {
        Self {}
    }

    async fn store_get(&self, key: &str) -> Result<Option<String>, String> {
        let store = SYS_STORE.lock().await;
        store
            .get(key.to_string())
            .await
            .map_err(|e| format!("sys_store get {} failed: {}", key, e))
    }

    async fn load_zone_identity(&self) -> Result<Option<ZoneIdentity>, String> {
        let Some(cfg_str) = self.store_get("boot/config").await? else {
            return Ok(None);
        };
        let zone_config: ZoneConfig = serde_json::from_str(&cfg_str)
            .map_err(|e| format!("parse boot/config as ZoneConfig failed: {}", e))?;
        let raw = EncodedDocument::from_str(zone_config.zone_document.clone())
            .map_err(|e| format!("parse boot/config zone_document failed: {}", e))?;
        let raw_is_zone_doc = ZoneDocument::decode(&raw, None).is_ok();
        let zone_doc = zone_config
            .zone_document()
            .map_err(|e| format!("decode boot/config zone_document failed: {}", e))?;
        Ok(Some(ZoneIdentity {
            zone_doc,
            raw,
            raw_is_zone_doc,
        }))
    }

    // did:web:<short>.<zone_host> / 裸短名 → zone 命名空间内；zone 自身两种写法
    //（zone did / zone hostname）→ ZoneItself；其余是 zone 外名字。
    fn classify(&self, zone: &ZoneIdentity, input: &str) -> Result<Target, String> {
        if !DID::is_did(input) {
            return Ok(Target::InZone(input.to_string()));
        }
        let did = DID::from_str(input).map_err(|e| format!("invalid did {}: {}", input, e))?;
        // 规则 10：key 类 DID 不是 resolve_did 的合法入参。设备/身份一律走逻辑名，
        // key 只出现在文档内容里。
        if is_key_class_method(did.method.as_str()) {
            return Err(format!(
                "key-class DID {} is not a legal resolve input; query the logical name instead",
                input
            ));
        }
        if did == zone.zone_doc.id {
            return Ok(Target::ZoneItself);
        }
        if did.method == "web" {
            let zone_host = zone.hostname();
            if did.id == zone_host {
                return Ok(Target::ZoneItself);
            }
            let suffix = format!(".{}", zone_host);
            if let Some(short) = did.id.strip_suffix(suffix.as_str()) {
                if !short.is_empty() {
                    return Ok(Target::InZone(short.to_string()));
                }
            }
            // 无点的 did:web 简写按 zone 内短名处理（历史行为）
            if !did.id.contains('.') {
                return Ok(Target::InZone(did.id.clone()));
            }
        }
        Ok(Target::Foreign(did))
    }

    fn doc_version(encoded: &EncodedDocument) -> Option<u64> {
        let value = encoded.clone().to_json_value().ok()?;
        value.get("iat").and_then(Value::as_u64).or_else(|| {
            value
                .get("exp")
                .and_then(Value::as_u64)
                .map(|exp| exp.saturating_sub(DEFAULT_EXPIRE_TIME))
        })
    }

    async fn load_device_doc(&self, key: &str) -> Result<Option<EncodedDocument>, String> {
        let path = format!("devices/{}/doc", key);
        let Some(doc_str) = self.store_get(path.as_str()).await? else {
            return Ok(None);
        };
        let encoded = EncodedDocument::from_str(doc_str)
            .map_err(|e| format!("parse {} failed: {}", path, e))?;
        DeviceDocument::decode(&encoded, None).map_err(|e| format!("{} corrupt: {}", path, e))?;
        Ok(Some(encoded))
    }

    async fn load_owner_doc(
        &self,
        key: &str,
    ) -> Result<Option<(OwnerDocument, EncodedDocument)>, String> {
        let path = format!("users/{}/doc", key);
        let Some(doc_str) = self.store_get(path.as_str()).await? else {
            return Ok(None);
        };
        let encoded = EncodedDocument::from_str(doc_str)
            .map_err(|e| format!("parse {} failed: {}", path, e))?;
        let owner_doc = OwnerDocument::decode(&encoded, None)
            .map_err(|e| format!("{} corrupt: {}", path, e))?;
        Ok(Some((owner_doc, encoded)))
    }

    async fn load_agent_doc(
        &self,
        key: &str,
    ) -> Result<Option<(AgentDocument, EncodedDocument)>, String> {
        let path = format!("agents/{}/doc", key);
        let Some(doc_str) = self.store_get(path.as_str()).await? else {
            return Ok(None);
        };
        let encoded = EncodedDocument::from_str(doc_str)
            .map_err(|e| format!("parse {} failed: {}", path, e))?;
        let agent_doc = AgentDocument::decode(&encoded, None)
            .map_err(|e| format!("{} corrupt: {}", path, e))?;
        Ok(Some((agent_doc, encoded)))
    }

    // agent 的真实 DID 与 store 短名键无关，按 id 扫描（zone 内 agent 数量很小）。
    async fn find_agent_doc_by_did(
        &self,
        did: &DID,
    ) -> Result<Option<(AgentDocument, EncodedDocument)>, String> {
        let agent_ids = {
            let store = SYS_STORE.lock().await;
            store
                .list_direct_children("agents".to_string())
                .await
                .map_err(|e| format!("list agents failed: {}", e))?
        };
        for agent_id in agent_ids {
            if let Ok(Some((agent_doc, encoded))) = self.load_agent_doc(agent_id.as_str()).await {
                if agent_doc.id == *did {
                    return Ok(Some((agent_doc, encoded)));
                }
            }
        }
        Ok(None)
    }

    async fn load_device_info(&self, key: &str) -> Result<Option<Value>, String> {
        let path = format!("devices/{}/info", key);
        let Some(info_str) = self.store_get(path.as_str()).await? else {
            return Ok(None);
        };
        let device_info: DeviceInfo = serde_json::from_str(info_str.as_str())
            .map_err(|e| format!("{} corrupt: {}", path, e))?;
        serde_json::to_value(&device_info)
            .map(Some)
            .map_err(|e| format!("serialize device info {} failed: {}", path, e))
    }

    fn active(
        document: EncodedDocument,
        doc_type: &DidDocType,
        effective_owner: Option<DID>,
        with_hash: bool,
    ) -> ZoneAnswer {
        let version = Self::doc_version(&document);
        ZoneAnswer::Published(PublishedAnswer {
            status: PublishedStatus::Active,
            document: Some(document),
            doc_type: doc_type.clone(),
            version,
            effective_owner,
            authority_seq: None,
            doc_hash: None,
            with_hash,
            migration_target: None,
        })
    }

    // ---------------- zone 级 cache / override ----------------

    // store 键段转义：合法 DID 不含 '/'，did:web 端口写法含 '%'；转义保证
    // 键段无歧义且通常保持人类可读（did:bns:alice 原样）。写入方必须用同一规则。
    fn escape_cache_segment(raw: &str) -> String {
        raw.replace('%', "%25").replace('/', "%2F")
    }

    fn cache_key_base(did: &DID, doc_type: &DidDocType) -> String {
        format!(
            "{}/{}/{}",
            RESOLVER_CACHE_PREFIX,
            Self::escape_cache_segment(did.to_string().as_str()),
            Self::escape_cache_segment(doc_type.as_str())
        )
    }

    // 把一条 cache 登记（state + 可选 doc）翻译成回答。纯函数便于单测。
    // state JSON 字段（需求 §5.4）：
    //   document_status: active|missing|revoked|tombstoned|migrated|expired（必填）
    //   document_version（当前发布文档的 iat）/ effective_owner / authority_seq /
    //   doc_hash / migration_target（可选）
    //   updated_at / updated_by：审计字段，resolver 不消费
    fn cache_answer(
        state_str: &str,
        doc_str: Option<String>,
        doc_type: &DidDocType,
    ) -> Result<ZoneAnswer, String> {
        let state: Value = serde_json::from_str(state_str)
            .map_err(|e| format!("state is not valid json: {}", e))?;
        let status_str = state
            .get("document_status")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "state missing document_status".to_string())?;
        if status_str == "missing" {
            return Ok(ZoneAnswer::Missing(doc_type.clone()));
        }
        let status = PublishedStatus::parse(status_str)
            .ok_or_else(|| format!("unknown document_status {}", status_str))?;

        let document = doc_str
            .map(EncodedDocument::from_str)
            .transpose()
            .map_err(|e| format!("doc is not a valid encoded document: {}", e))?;
        if status == PublishedStatus::Active && document.is_none() {
            // zone 内 ZoneResolverClient 把"200 active 但没有文档"当坏回答，
            // anchor-only 的 active 登记在 zone L1 上不成立。
            return Err("active entry requires an inline doc".to_string());
        }

        let parse_did = |field: &str| -> Result<Option<DID>, String> {
            state
                .get(field)
                .and_then(|v| v.as_str())
                .map(|s| {
                    DID::from_str(s).map_err(|e| format!("{} is not a valid did: {}", field, e))
                })
                .transpose()
        };
        let effective_owner = parse_did("effective_owner")?;
        let migration_target = parse_did("migration_target")?;
        if status == PublishedStatus::Migrated && migration_target.is_none() {
            return Err("migrated entry requires migration_target".to_string());
        }

        let declared_version = state.get("document_version").and_then(|v| v.as_u64());
        let document_version = document.as_ref().and_then(Self::doc_version);
        if status == PublishedStatus::Active && *doc_type != DidDocType::Info {
            let document_version = document_version.ok_or_else(|| {
                "active document must carry iat or exp so document_version can be derived"
                    .to_string()
            })?;
            if let Some(declared_version) = declared_version {
                if declared_version != document_version {
                    return Err(format!(
                        "document_version {} does not match document iat {}",
                        declared_version, document_version
                    ));
                }
            }
        }
        let version = document_version.or(declared_version);
        let authority_seq = state.get("authority_seq").and_then(|v| v.as_u64());
        let doc_hash = state
            .get("doc_hash")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(ZoneAnswer::Published(PublishedAnswer {
            status,
            document,
            doc_type: doc_type.clone(),
            version,
            effective_owner,
            authority_seq,
            doc_hash,
            with_hash: true,
            migration_target,
        }))
    }

    // 查 cache 登记。Ok(None) = 没有登记，继续结构化查询；Err = 登记存在但坏了
    //（显式 override 损坏必须大声失败成 500，而不是静默绕过——那可能是一条吊销）。
    async fn cache_lookup(
        &self,
        did: &DID,
        doc_type: &DidDocType,
    ) -> Result<Option<ZoneAnswer>, String> {
        let base = Self::cache_key_base(did, doc_type);
        let Some(state_str) = self.store_get(format!("{}/state", base).as_str()).await? else {
            return Ok(None);
        };
        let doc_str = self.store_get(format!("{}/doc", base).as_str()).await?;
        Self::cache_answer(state_str.as_str(), doc_str, doc_type)
            .map(Some)
            .map_err(|e| format!("resolver cache {} corrupt: {}", base, e))
    }

    // ---------------- zone owner 的 Root Trust 保护（需求 §6.2） ----------------

    // 解析结果 = 权威源 OwnerDocument 的非公钥部分 + 本地 Root Trust 的公钥部分。
    // key material 字段（verificationMethod / authentication / assertionMethod /
    // capabilityInvocation）整体取自以本地 key 构造的最小文档；keyScope 属于
    // key material 相关扩展，回落到本地基线（空）——扩展 key scope 必须走维护
    // 流程或受控系统配置。合并产物是未签名 JSON（sys_config 不持有 owner 私钥，
    // 也绝不该持有）；zone 内消费者信任控制面传输通道。
    fn merge_owner_profile_with_local_key(
        stored: &OwnerDocument,
        local_key: &Jwk,
    ) -> Result<Value, String> {
        let mut merged = serde_json::to_value(stored)
            .map_err(|e| format!("serialize stored owner doc failed: {}", e))?;
        let minimal = OwnerDocument::new(
            stored.id.clone(),
            stored.name.clone(),
            stored.display_name.clone(),
            local_key.clone(),
        );
        let minimal = serde_json::to_value(&minimal)
            .map_err(|e| format!("serialize minimal owner doc failed: {}", e))?;
        for field in [
            "verificationMethod",
            "authentication",
            "assertionMethod",
            "capabilityInvocation",
        ] {
            merged[field] = minimal.get(field).cloned().unwrap_or(Value::Null);
        }
        if let Some(obj) = merged.as_object_mut() {
            obj.remove("keyScope");
            obj.remove("buckyos:scopes");
        }
        Ok(merged)
    }

    // 激活建立的信任在 runtime 的最小投影：权威源 owner doc 丢失/损坏时，
    // profile 展示退化到它，而不是让 zone 对自己的 owner 回答 missing 或
    // 放任外查（Boot 侧的对应物是 node_daemon 的 install_local_owner_trust）。
    fn minimal_zone_owner_doc(zone: &ZoneIdentity) -> Option<Value> {
        if !zone.zone_doc.owner.is_valid() {
            return None;
        }
        let local_key = zone.local_root_trust_key()?;
        let owner = zone.zone_doc.owner.clone();
        let mut doc =
            OwnerDocument::new(owner.clone(), owner.id.clone(), owner.id.clone(), local_key);
        doc.set_default_zone_did(zone.zone_doc.id.clone());
        serde_json::to_value(&doc).ok()
    }

    // 对"当前 zone owner"的 owner doc 回答做 Root Trust 锚定：
    // 权威源 key 与本地一致 → 原样返回（保留签名）；不一致 → 合并（本地 key 优先）。
    // 非 zone owner 的 owner doc 不经过本函数。
    fn guarded_owner_active(
        zone: &ZoneIdentity,
        owner_doc: OwnerDocument,
        encoded: EncodedDocument,
        doc_type: &DidDocType,
    ) -> ZoneAnswer {
        let owner_did = owner_doc.id.clone();
        let Some(local_key) = zone.local_root_trust_key() else {
            // boot/config 里 zone_document 没有默认 key：无从校验，只能原样返回。
            error!(
                "zone document has no default key; cannot anchor owner doc {} to local root trust",
                owner_did.to_string()
            );
            return Self::active(encoded, doc_type, Some(owner_did), true);
        };
        match owner_doc.get_default_key() {
            Some(stored_key) if stored_key == local_key => {
                Self::active(encoded, doc_type, Some(owner_did), true)
            }
            _ => {
                warn!(
                    "authoritative owner doc {} key material differs from local root trust; \
                     serving merged doc with local key (owner key rotation must go through \
                     maintenance/recovery flow)",
                    owner_did.to_string()
                );
                match Self::merge_owner_profile_with_local_key(&owner_doc, &local_key) {
                    Ok(merged) => Self::active(
                        EncodedDocument::JsonLd(merged),
                        doc_type,
                        Some(owner_did),
                        true,
                    ),
                    Err(e) => ZoneAnswer::Internal(e),
                }
            }
        }
    }

    // cache 命中同样不能成为替换 Root Trust 的通道：owner/user 类 active 命中若
    // 载荷是当前 zone owner 的 OwnerDocument，key material 一致才原样放行，
    // 否则换成合并文档（state 钉的 doc_hash 随 body 一起失效，改为现算）。
    fn guard_cache_hit(
        zone: &ZoneIdentity,
        answer: ZoneAnswer,
        doc_type: &DidDocType,
    ) -> ZoneAnswer {
        if !matches!(doc_type, DidDocType::Owner | DidDocType::User) {
            return answer;
        }
        let ZoneAnswer::Published(mut published) = answer else {
            return answer;
        };
        if published.status != PublishedStatus::Active {
            return ZoneAnswer::Published(published);
        }
        let Some(document) = published.document.as_ref() else {
            return ZoneAnswer::Published(published);
        };
        let owner_doc = match OwnerDocument::decode(document, None) {
            Ok(owner_doc) => owner_doc,
            Err(e) => {
                // owner/user 类 active 登记必须能被审视，否则无法执行 Root Trust 锚定
                return ZoneAnswer::Internal(format!(
                    "resolver cache active owner doc is not a valid OwnerDocument: {}",
                    e
                ));
            }
        };
        if !zone.is_zone_owner(&owner_doc.id) {
            return ZoneAnswer::Published(published);
        }
        let Some(local_key) = zone.local_root_trust_key() else {
            error!(
                "zone document has no default key; cannot anchor cached owner doc {}",
                owner_doc.id.to_string()
            );
            return ZoneAnswer::Published(published);
        };
        if owner_doc.get_default_key() == Some(local_key.clone()) {
            return ZoneAnswer::Published(published);
        }
        warn!(
            "resolver cache owner doc {} key material differs from local root trust; \
             serving merged doc with local key",
            owner_doc.id.to_string()
        );
        match Self::merge_owner_profile_with_local_key(&owner_doc, &local_key) {
            Ok(merged) => {
                published.document = Some(EncodedDocument::JsonLd(merged));
                published.doc_hash = None;
                published.with_hash = true;
                ZoneAnswer::Published(published)
            }
            Err(e) => ZoneAnswer::Internal(e),
        }
    }

    // ---------------- 解析主流程 ----------------

    async fn resolve(&self, name: &str, doc_type: &DidDocType) -> ZoneAnswer {
        // zone 身份不可用（未初始化 / store 故障 / 配置损坏）时这个 cache 语义上
        // 就是不可用：503 让客户端回退自己的解析管线，而不是 500 堵死一切。
        let zone = match self.load_zone_identity().await {
            Ok(Some(zone)) => zone,
            Ok(None) => {
                return ZoneAnswer::NoOpinion(
                    "zone is not initialized (boot/config not set)".to_string(),
                )
            }
            Err(e) => {
                warn!("ZoneDidResolver load zone identity failed: {}", e);
                return ZoneAnswer::NoOpinion(e);
            }
        };

        if name == "self" {
            return match serde_json::to_value(&zone.zone_doc) {
                Ok(v) => ZoneAnswer::Bare(v),
                Err(e) => ZoneAnswer::Internal(format!("serialize zone document failed: {}", e)),
            };
        }

        match self.classify(&zone, name) {
            Err(reason) => ZoneAnswer::BadRequest(reason),
            Ok(Target::ZoneItself) => match doc_type {
                // §6.3：zone/boot 只来自 boot 状态机已接受的 boot/config，
                // cache override 不得绕过 LKGS / smooth upgrade。
                DidDocType::Zone | DidDocType::Boot => self.resolve_zone_itself(&zone, doc_type),
                _ => match self.cache_lookup(&zone.zone_doc.id, doc_type).await {
                    Ok(Some(answer)) => Self::guard_cache_hit(&zone, answer, doc_type),
                    // zone 对自己的命名空间是权威的，没有登记就是 Missing
                    Ok(None) => ZoneAnswer::Missing(doc_type.clone()),
                    Err(e) => {
                        error!("ZoneDidResolver cache lookup for zone itself failed: {}", e);
                        ZoneAnswer::Internal(e)
                    }
                },
            },
            Ok(Target::InZone(short)) => {
                let canonical = DID::new("web", format!("{}.{}", short, zone.hostname()).as_str());
                match self.cache_lookup(&canonical, doc_type).await {
                    Ok(Some(answer)) => Self::guard_cache_hit(&zone, answer, doc_type),
                    Ok(None) => self.resolve_in_zone(&zone, short.as_str(), doc_type).await,
                    Err(e) => {
                        error!("ZoneDidResolver cache lookup for {} failed: {}", short, e);
                        ZoneAnswer::Internal(e)
                    }
                }
            }
            Ok(Target::Foreign(did)) => match self.cache_lookup(&did, doc_type).await {
                Ok(Some(answer)) => Self::guard_cache_hit(&zone, answer, doc_type),
                Ok(None) => self.resolve_foreign(&zone, &did, doc_type).await,
                Err(e) => {
                    error!(
                        "ZoneDidResolver cache lookup for {} failed: {}",
                        did.to_string(),
                        e
                    );
                    ZoneAnswer::Internal(e)
                }
            },
        }
    }

    fn resolve_zone_itself(&self, zone: &ZoneIdentity, doc_type: &DidDocType) -> ZoneAnswer {
        let owner = zone
            .zone_doc
            .owner
            .is_valid()
            .then(|| zone.zone_doc.owner.clone());
        match doc_type {
            DidDocType::Zone => {
                let document = if zone.raw_is_zone_doc {
                    zone.raw.clone()
                } else {
                    match serde_json::to_value(&zone.zone_doc) {
                        Ok(v) => EncodedDocument::JsonLd(v),
                        Err(e) => {
                            return ZoneAnswer::Internal(format!(
                                "serialize zone document failed: {}",
                                e
                            ))
                        }
                    }
                };
                Self::active(document, doc_type, owner, true)
            }
            DidDocType::Boot => {
                if zone.zone_doc.boot_jwt.is_empty() {
                    ZoneAnswer::Missing(doc_type.clone())
                } else {
                    Self::active(
                        EncodedDocument::Jwt(zone.zone_doc.boot_jwt.clone()),
                        doc_type,
                        owner,
                        true,
                    )
                }
            }
            _ => ZoneAnswer::Missing(doc_type.clone()),
        }
    }

    async fn resolve_in_zone(
        &self,
        zone: &ZoneIdentity,
        short: &str,
        doc_type: &DidDocType,
    ) -> ZoneAnswer {
        // zone 命名空间内（*.zone_host / 裸短名）的绑定是结构性的：miss 是权威 Missing；
        // store 读失败/条目损坏是 Internal。例外是 zone owner 自己的 owner doc：
        // owner 由 boot/config 锚定、永远存在，miss/corrupt 都退化为本地最小文档（§6.2）。
        let result: Result<Option<ZoneAnswer>, String> = match doc_type {
            DidDocType::Info => self.load_device_info(short).await.map(|info| {
                info.map(|v| Self::active(EncodedDocument::JsonLd(v), doc_type, None, false))
            }),
            DidDocType::Device => self
                .load_device_doc(short)
                .await
                .map(|doc| doc.map(|encoded| Self::active(encoded, doc_type, None, true))),
            DidDocType::Owner | DidDocType::User => {
                return self.resolve_in_zone_owner(zone, short, doc_type).await
            }
            DidDocType::Custom(ref t) if t == AGENT_DOC_TYPE => self
                .load_agent_doc(short)
                .await
                .map(|doc| doc.map(|(_, encoded)| Self::active(encoded, doc_type, None, true))),
            // 默认 doc_type（zone）：typeless 老客户端把设备逻辑名当默认类型查
            //（如 resolve_did(device_did, None) 取 device doc 提 ips），保留
            // device → agent → owner 的 any-doc 兜底。
            DidDocType::Zone => self.load_any_doc(zone, short).await,
            _ => Ok(None),
        };
        match result {
            Ok(Some(answer)) => answer,
            Ok(None) => ZoneAnswer::Missing(doc_type.clone()),
            Err(e) => {
                warn!("ZoneDidResolver in-zone lookup {} failed: {}", short, e);
                ZoneAnswer::Internal(e)
            }
        }
    }

    async fn resolve_in_zone_owner(
        &self,
        zone: &ZoneIdentity,
        short: &str,
        doc_type: &DidDocType,
    ) -> ZoneAnswer {
        match self.load_owner_doc(short).await {
            Ok(Some((owner_doc, encoded))) => {
                if zone.is_zone_owner(&owner_doc.id) {
                    Self::guarded_owner_active(zone, owner_doc, encoded, doc_type)
                } else {
                    let owner_did = owner_doc.id.clone();
                    Self::active(encoded, doc_type, Some(owner_did), true)
                }
            }
            Ok(None) => {
                if zone.is_zone_owner_short(short) {
                    if let Some(minimal) = Self::minimal_zone_owner_doc(zone) {
                        info!(
                            "users/{}/doc not set; serving minimal owner doc from local root trust",
                            short
                        );
                        let owner_did = zone.zone_doc.owner.clone();
                        return Self::active(
                            EncodedDocument::JsonLd(minimal),
                            doc_type,
                            Some(owner_did),
                            true,
                        );
                    }
                }
                ZoneAnswer::Missing(doc_type.clone())
            }
            Err(e) => {
                if zone.is_zone_owner_short(short) {
                    if let Some(minimal) = Self::minimal_zone_owner_doc(zone) {
                        error!(
                            "users/{}/doc corrupt ({}); degrading to minimal owner doc from local root trust",
                            short, e
                        );
                        let owner_did = zone.zone_doc.owner.clone();
                        return Self::active(
                            EncodedDocument::JsonLd(minimal),
                            doc_type,
                            Some(owner_did),
                            true,
                        );
                    }
                }
                warn!("ZoneDidResolver in-zone lookup {} failed: {}", short, e);
                ZoneAnswer::Internal(e)
            }
        }
    }

    async fn load_any_doc(
        &self,
        zone: &ZoneIdentity,
        short: &str,
    ) -> Result<Option<ZoneAnswer>, String> {
        if let Some(encoded) = self.load_device_doc(short).await? {
            return Ok(Some(Self::active(encoded, &DidDocType::Device, None, true)));
        }
        if let Some((_, encoded)) = self.load_agent_doc(short).await? {
            return Ok(Some(Self::active(
                encoded,
                &DidDocType::Custom(AGENT_DOC_TYPE.to_string()),
                None,
                true,
            )));
        }
        if let Some((owner_doc, encoded)) = self.load_owner_doc(short).await? {
            if zone.is_zone_owner(&owner_doc.id) {
                return Ok(Some(Self::guarded_owner_active(
                    zone,
                    owner_doc,
                    encoded,
                    &DidDocType::Owner,
                )));
            }
            let owner_did = owner_doc.id.clone();
            return Ok(Some(Self::active(
                encoded,
                &DidDocType::Owner,
                Some(owner_did),
                true,
            )));
        }
        Ok(None)
    }

    async fn resolve_foreign(
        &self,
        zone: &ZoneIdentity,
        did: &DID,
        doc_type: &DidDocType,
    ) -> ZoneAnswer {
        // zone 对 zone 外名字唯一能做的贡献：cache override（在 resolve 里已查过）
        // 或 store 里恰好持有该主体的文档（zone 成员的 owner doc、按真实 DID 注册
        // 的 agent doc）。store 键是 zone 内短名，与全局名同名不同主体时
        //（users/alice vs did:bns:alice）绝不能张冠李戴——回答前必须核对文档自述
        // id 与查询 DID 一致，不一致按没有意见处理。这里的 store/解码失败也降级
        // 为没有意见：zone 外名字永远有外部解析可走。
        // 例外：当前 zone owner 的 owner doc 查询（§6.2）——zone 对"谁是我的
        // owner"有权威（boot/config 锚定），一律给出锚定本地 Root Trust 的回答，
        // 不放任外查替换信任根。
        let hit: Option<ZoneAnswer> = match doc_type {
            DidDocType::Owner | DidDocType::User => {
                if zone.is_zone_owner(did) {
                    return self.resolve_foreign_zone_owner(zone, did, doc_type).await;
                }
                self.load_owner_doc(did.id.as_str())
                    .await
                    .ok()
                    .flatten()
                    .filter(|(owner_doc, _)| owner_doc.id == *did)
                    .map(|(owner_doc, encoded)| {
                        Self::active(encoded, doc_type, Some(owner_doc.id), true)
                    })
            }
            DidDocType::Custom(ref t) if t == AGENT_DOC_TYPE => {
                let by_key = self
                    .load_agent_doc(did.id.as_str())
                    .await
                    .ok()
                    .flatten()
                    .filter(|(agent_doc, _)| agent_doc.id == *did);
                let found = match by_key {
                    Some(found) => Some(found),
                    None => self.find_agent_doc_by_did(did).await.ok().flatten(),
                };
                found.map(|(_, encoded)| Self::active(encoded, doc_type, None, true))
            }
            // 默认 doc_type：只保留 typeless 老客户端按真实 DID 找 agent doc 的行为。
            // 不做 owner doc 兜底——把 (did, zone) 回答成 owner 文档是错误类型的独占
            // 回答，会堵死客户端对该名字真正 zone 文档的外部解析；owner 文档走显式
            // type=owner 查询。
            DidDocType::Zone => {
                self.find_agent_doc_by_did(did)
                    .await
                    .ok()
                    .flatten()
                    .map(|(_, encoded)| {
                        Self::active(
                            encoded,
                            &DidDocType::Custom(AGENT_DOC_TYPE.to_string()),
                            None,
                            true,
                        )
                    })
            }
            _ => None,
        };
        match hit {
            Some(answer) => answer,
            None => ZoneAnswer::NoOpinion(format!(
                "zone resolver has no answer for {}#{}",
                did.to_string(),
                doc_type
            )),
        }
    }

    // 当前 zone owner 的 owner doc（按真实 DID 查询，如 did:bns:alice?type=owner）：
    // store 有且 id 一致 → 锚定后的回答；miss / corrupt / id 不一致 → 本地最小文档。
    async fn resolve_foreign_zone_owner(
        &self,
        zone: &ZoneIdentity,
        did: &DID,
        doc_type: &DidDocType,
    ) -> ZoneAnswer {
        match self.load_owner_doc(did.id.as_str()).await {
            Ok(Some((owner_doc, encoded))) if owner_doc.id == *did => {
                Self::guarded_owner_active(zone, owner_doc, encoded, doc_type)
            }
            other => {
                if let Err(e) = &other {
                    error!(
                        "zone owner doc users/{}/doc unusable ({}); degrading to minimal owner doc",
                        did.id, e
                    );
                }
                match Self::minimal_zone_owner_doc(zone) {
                    Some(minimal) => {
                        info!(
                            "serving minimal owner doc for zone owner {} from local root trust",
                            did.to_string()
                        );
                        Self::active(
                            EncodedDocument::JsonLd(minimal),
                            doc_type,
                            Some(did.clone()),
                            true,
                        )
                    }
                    None => ZoneAnswer::NoOpinion(format!(
                        "zone resolver has no answer for {}#{}",
                        did.to_string(),
                        doc_type
                    )),
                }
            }
        }
    }

    // ---------------- 响应信封 ----------------

    fn sha256_hex(data: &str) -> String {
        let digest = Sha256::digest(data.as_bytes());
        digest.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn published_envelope(answer: &PublishedAnswer) -> Value {
        let mut resolution_metadata = json!({});
        let (did_document, doc_string) = match &answer.document {
            Some(EncodedDocument::Jwt(jwt)) => {
                resolution_metadata["contentType"] = json!(CONTENT_TYPE_DID_JWT);
                (Value::String(jwt.clone()), Some(jwt.clone()))
            }
            Some(EncodedDocument::JsonLd(v)) => {
                resolution_metadata["contentType"] = json!(CONTENT_TYPE_DID_JSON);
                (
                    v.clone(),
                    Some(serde_json::to_string(v).unwrap_or_default()),
                )
            }
            None => (Value::Null, None),
        };
        let mut buckyos = json!({
            "docType": answer.doc_type.as_str(),
            "documentStatus": answer.status.as_str(),
        });
        if let Some(version) = answer.version {
            buckyos["documentVersion"] = json!(version);
        }
        if let Some(owner) = &answer.effective_owner {
            buckyos["effectiveOwner"] = json!(owner.to_string());
        }
        if let Some(seq) = answer.authority_seq {
            buckyos["authoritySeq"] = json!(seq);
        }
        if let Some(target) = &answer.migration_target {
            buckyos["migrationTarget"] = json!(target.to_string());
        }
        let doc_hash = answer.doc_hash.clone().or_else(|| {
            if answer.with_hash {
                doc_string
                    .as_ref()
                    .map(|s| format!("sha256:{}", Self::sha256_hex(s.as_str())))
            } else {
                None
            }
        });
        if let Some(hash) = doc_hash {
            buckyos["docHash"] = json!(hash);
        }
        let mut document_metadata = json!({
            "deactivated": answer.status.deactivated(),
            "buckyos": buckyos,
        });
        if let Some(version) = answer.version {
            document_metadata["versionId"] = json!(version.to_string());
        }
        json!({
            "didResolutionMetadata": resolution_metadata,
            "didDocument": did_document,
            "didDocumentMetadata": document_metadata,
        })
    }

    fn missing_envelope(doc_type: &DidDocType) -> Value {
        json!({
            "didResolutionMetadata": {"error": "notFound"},
            "didDocument": Value::Null,
            "didDocumentMetadata": {
                "deactivated": false,
                "buckyos": {
                    "docType": doc_type.as_str(),
                    "documentStatus": "missing",
                }
            }
        })
    }

    // resolver API（/1.0/identifiers）的渲染：一律信封。
    fn answer_to_response(name: &str, answer: ZoneAnswer) -> (StatusCode, &'static str, String) {
        match answer {
            ZoneAnswer::Published(published) => {
                info!(
                    "ZoneDidResolver resolve {}#{} => {}",
                    name,
                    published.doc_type.as_str(),
                    published.status.as_str()
                );
                let body = Self::published_envelope(&published);
                (
                    published.status.http_status(),
                    CONTENT_TYPE_RESOLUTION,
                    body.to_string(),
                )
            }
            ZoneAnswer::Bare(value) => (StatusCode::OK, CONTENT_TYPE_JSON, value.to_string()),
            ZoneAnswer::Missing(doc_type) => {
                info!(
                    "ZoneDidResolver resolve {}#{} => missing",
                    name,
                    doc_type.as_str()
                );
                (
                    StatusCode::NOT_FOUND,
                    CONTENT_TYPE_RESOLUTION,
                    Self::missing_envelope(&doc_type).to_string(),
                )
            }
            ZoneAnswer::NoOpinion(reason) => {
                debug!("ZoneDidResolver no opinion for {}: {}", name, reason);
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    CONTENT_TYPE_JSON,
                    json!({"error": reason}).to_string(),
                )
            }
            ZoneAnswer::BadRequest(reason) => {
                warn!("ZoneDidResolver bad request {}: {}", name, reason);
                (
                    StatusCode::BAD_REQUEST,
                    CONTENT_TYPE_JSON,
                    json!({"error": reason}).to_string(),
                )
            }
            ZoneAnswer::HistoricalNotSupported(reason) => {
                info!(
                    "ZoneDidResolver historical query rejected for {}: {}",
                    name, reason
                );
                (
                    StatusCode::NOT_IMPLEMENTED,
                    CONTENT_TYPE_JSON,
                    json!({
                        "error": reason,
                        "buckyos": {"historicalQuerySupported": false},
                    })
                    .to_string(),
                )
            }
            ZoneAnswer::Internal(reason) => {
                error!("ZoneDidResolver internal error for {}: {}", name, reason);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CONTENT_TYPE_JSON,
                    json!({"error": reason}).to_string(),
                )
            }
        }
    }

    // 静态发布面（/.well-known/*）的渲染：Active 返回裸文档 body，绝不带信封；
    // 状态语义不属于静态面——miss 是裸 404（客户端语义 unknown，安全），
    // 吊销登记映射为裸 410（旧客户端也按 Disabled 解释）。
    fn answer_to_static_response(
        name: &str,
        answer: ZoneAnswer,
        jwt_only: bool,
    ) -> (StatusCode, &'static str, String) {
        match answer {
            ZoneAnswer::Published(published) => {
                info!(
                    "ZoneDidResolver well-known {}#{} => {}",
                    name,
                    published.doc_type.as_str(),
                    published.status.as_str()
                );
                match (published.status, published.document) {
                    (PublishedStatus::Active, Some(EncodedDocument::Jwt(jwt))) => {
                        (StatusCode::OK, CONTENT_TYPE_DID_JWT, jwt)
                    }
                    (PublishedStatus::Active, Some(EncodedDocument::JsonLd(v))) => {
                        if jwt_only {
                            (
                                StatusCode::NOT_FOUND,
                                CONTENT_TYPE_JSON,
                                json!({"error": "no jwt representation published"}).to_string(),
                            )
                        } else {
                            (StatusCode::OK, CONTENT_TYPE_DID_JSON, v.to_string())
                        }
                    }
                    (PublishedStatus::Revoked | PublishedStatus::Tombstoned, _) => (
                        StatusCode::GONE,
                        CONTENT_TYPE_JSON,
                        json!({"error": "document revoked"}).to_string(),
                    ),
                    // active 无文档不可达；expired/migrated 在静态面没有可发布内容
                    _ => (
                        StatusCode::NOT_FOUND,
                        CONTENT_TYPE_JSON,
                        json!({"error": "not published"}).to_string(),
                    ),
                }
            }
            ZoneAnswer::Bare(value) => (StatusCode::OK, CONTENT_TYPE_JSON, value.to_string()),
            ZoneAnswer::Missing(doc_type) => {
                info!(
                    "ZoneDidResolver well-known {}#{} => not published",
                    name,
                    doc_type.as_str()
                );
                (
                    StatusCode::NOT_FOUND,
                    CONTENT_TYPE_JSON,
                    json!({"error": "not published"}).to_string(),
                )
            }
            ZoneAnswer::NoOpinion(reason) => (
                StatusCode::SERVICE_UNAVAILABLE,
                CONTENT_TYPE_JSON,
                json!({"error": reason}).to_string(),
            ),
            ZoneAnswer::BadRequest(reason) => (
                StatusCode::BAD_REQUEST,
                CONTENT_TYPE_JSON,
                json!({"error": reason}).to_string(),
            ),
            ZoneAnswer::HistoricalNotSupported(reason) => (
                StatusCode::NOT_IMPLEMENTED,
                CONTENT_TYPE_JSON,
                json!({"error": reason}).to_string(),
            ),
            ZoneAnswer::Internal(reason) => {
                error!("ZoneDidResolver internal error for {}: {}", name, reason);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    CONTENT_TYPE_JSON,
                    json!({"error": reason}).to_string(),
                )
            }
        }
    }
}

fn parse_doc_type(raw: Option<String>) -> DidDocType {
    match raw {
        None => DEFAULT_DID_DOC_TYPE,
        // well-known 的 did.json 与显式 type=did 都表示默认 doc_type
        Some(t) if t.is_empty() || t == "did" => DEFAULT_DID_DOC_TYPE,
        Some(t) => DidDocType::from(t),
    }
}

// /.well-known/{doc_type}[.json|.jwt] 的文件名解析：返回 (doc_type_token, jwt_only)。
// doc_type token 只允许 [A-Za-z0-9_-]（协议 §1.1：不能含 '.' 或 '/'，后缀解析
// 才没有歧义）。.json 与无后缀是自动识别入口（body 按存储编码返回），.jwt 是
// 强类型 JWT 入口。
fn parse_well_known_file(file: &str) -> Result<(String, bool), String> {
    let (token, jwt_only) = match file.rsplit_once('.') {
        Some((token, "json")) => (token, false),
        Some((token, "jwt")) => (token, true),
        Some((_, suffix)) => return Err(format!("unsupported representation suffix .{}", suffix)),
        None => (file, false),
    };
    if token.is_empty()
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(format!("invalid doc_type token: {}", token));
    }
    Ok((token.to_string(), jwt_only))
}

#[async_trait]
impl HttpServer for ZoneDidResolver {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let build_resp = |status: StatusCode,
                          content_type: &str,
                          body: String|
         -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
            Ok(http::Response::builder()
                .status(status)
                .header(http::header::CONTENT_TYPE, content_type)
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(http::header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
                .header(http::header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(BoxBody::new(
                    Full::new(Bytes::from(body))
                        .map_err(|never: std::convert::Infallible| -> ServerError {
                            match never {}
                        })
                        .boxed(),
                ))
                .map_err(|e| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "Failed to build response: {}",
                        e
                    )
                })?)
        };

        // CORS 预检
        if *req.method() == Method::OPTIONS {
            return Ok(http::Response::builder()
                .status(StatusCode::NO_CONTENT)
                .header(http::header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(http::header::ACCESS_CONTROL_ALLOW_METHODS, "GET, OPTIONS")
                .header(http::header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(BoxBody::new(
                    Full::new(Bytes::from_static(b""))
                        .map_err(|never: std::convert::Infallible| -> ServerError {
                            match never {}
                        })
                        .boxed(),
                ))
                .map_err(|e| {
                    server_err!(
                        ServerErrorCode::InvalidData,
                        "Failed to build response: {}",
                        e
                    )
                })?);
        }

        if *req.method() != Method::GET {
            return build_resp(
                StatusCode::METHOD_NOT_ALLOWED,
                CONTENT_TYPE_JSON,
                json!({"error": "method not allowed"}).to_string(),
            );
        }

        // GET /1.0/identifiers/{did}?type={doc_type}[&iat={ts}]
        let path = req.uri().path().to_string();
        if path.starts_with("/1.0/identifiers/") {
            let did_str = path.trim_start_matches("/1.0/identifiers/").to_string();
            if did_str.is_empty() {
                return build_resp(
                    StatusCode::BAD_REQUEST,
                    CONTENT_TYPE_JSON,
                    json!({"error": "invalid did in path"}).to_string(),
                );
            }

            let mut typ: Option<String> = None;
            let mut iat: Option<String> = None;
            if let Some(query) = req.uri().query() {
                for (key, value) in form_urlencoded::parse(query.as_bytes()) {
                    match key.as_ref() {
                        "type" => typ = Some(value.into_owned()),
                        "iat" => iat = Some(value.into_owned()),
                        _ => {}
                    }
                }
            }
            let doc_type = parse_doc_type(typ);

            // 协议 §7：无历史索引时必须区分"能力缺失"（501）与"历史 missing"
            //（404）。本 resolver 只有当前状态，任何带 iat 的查询都不能用当前
            // 结果冒充历史快照。
            let answer = if let Some(iat) = iat {
                ZoneAnswer::HistoricalNotSupported(format!(
                    "zone resolver keeps no historical index (iat={} ignored is unsafe)",
                    iat
                ))
            } else {
                self.resolve(did_str.as_str(), &doc_type).await
            };
            let (status, content_type, body) = Self::answer_to_response(did_str.as_str(), answer);
            return build_resp(status, content_type, body);
        }

        // GET http://{did_host_name}/.well-known/{doc_type}[.json|.jwt]
        // 静态发布面：返回裸文档 body（协议 §1.1），did.json 是 W3C did:web 兼容入口。
        if path.starts_with("/.well-known/") {
            let host = req.uri().host().map(|v| v.to_string()).or_else(|| {
                req.headers()
                    .get(http::header::HOST)
                    .and_then(|v| v.to_str().ok())
                    .map(|v| v.split(':').next().unwrap_or(v).to_string())
            });

            let Some(host) = host else {
                return build_resp(
                    StatusCode::BAD_REQUEST,
                    CONTENT_TYPE_JSON,
                    json!({"error": "host not found"}).to_string(),
                );
            };

            let file = path.trim_start_matches("/.well-known/").to_string();
            let (doc_type_token, jwt_only) = match parse_well_known_file(file.as_str()) {
                Ok(parsed) => parsed,
                Err(reason) => {
                    return build_resp(
                        StatusCode::BAD_REQUEST,
                        CONTENT_TYPE_JSON,
                        json!({"error": reason}).to_string(),
                    );
                }
            };
            let doc_type = parse_doc_type(Some(doc_type_token));

            let did_str = if DID::is_did(host.as_str()) {
                host.clone()
            } else {
                format!("did:web:{}", host)
            };

            let answer = self.resolve(did_str.as_str(), &doc_type).await;
            let (status, content_type, body) =
                Self::answer_to_static_response(did_str.as_str(), answer, jwt_only);
            return build_resp(status, content_type, body);
        }

        build_resp(
            StatusCode::NOT_FOUND,
            CONTENT_TYPE_JSON,
            json!({"error": "unknown path"}).to_string(),
        )
    }

    fn id(&self) -> String {
        "zone-did-resolver".to_string()
    }

    fn http_version(&self) -> http::Version {
        http::Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_JWK: &str =
        r#"{"kty":"OKP","crv":"Ed25519","x":"qJdNEtscIYwTo-I0K7iPEt_UZdBDRd4r16jdBfNR0tM"}"#;
    // 与 TEST_JWK 不同的合法形状 Ed25519 公钥（32 字节全零），模拟权威源 key rotation
    const OTHER_JWK: &str =
        r#"{"kty":"OKP","crv":"Ed25519","x":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#;

    fn test_zone_identity() -> ZoneIdentity {
        let jwk = serde_json::from_str(TEST_JWK).unwrap();
        let zone_doc = ZoneDocument::new(
            DID::new("web", "test.buckyos.io"),
            DID::new("bns", "testowner"),
            jwk,
        );
        let raw_value = serde_json::to_value(&zone_doc).unwrap();
        ZoneIdentity {
            zone_doc,
            raw: EncodedDocument::JsonLd(raw_value),
            raw_is_zone_doc: true,
        }
    }

    fn owner_doc_with_key(jwk_str: &str) -> OwnerDocument {
        let jwk: Jwk = serde_json::from_str(jwk_str).unwrap();
        let mut doc = OwnerDocument::new(
            DID::new("bns", "testowner"),
            "testowner".to_string(),
            "Test Owner".to_string(),
            jwk,
        );
        doc.avatar = Some("https://example.com/avatar.png".to_string());
        doc.key_scope
            .insert("#main_key".to_string(), vec!["*".to_string()]);
        doc
    }

    #[test]
    fn classify_routes_zone_and_namespace_names() {
        let resolver = ZoneDidResolver::new();
        let zone = test_zone_identity();

        assert!(matches!(
            resolver.classify(&zone, "did:web:test.buckyos.io"),
            Ok(Target::ZoneItself)
        ));
        let Ok(Target::InZone(short)) = resolver.classify(&zone, "did:web:ood1.test.buckyos.io")
        else {
            panic!("expected in-zone short name");
        };
        assert_eq!(short, "ood1");
        assert!(matches!(
            resolver.classify(&zone, "ood1"),
            Ok(Target::InZone(_))
        ));
        assert!(matches!(
            resolver.classify(&zone, "did:web:other.example.com"),
            Ok(Target::Foreign(_))
        ));
        assert!(matches!(
            resolver.classify(&zone, "did:bns:alice"),
            Ok(Target::Foreign(_))
        ));
    }

    #[test]
    fn classify_rejects_key_class_dids() {
        let resolver = ZoneDidResolver::new();
        let zone = test_zone_identity();
        assert!(resolver.classify(&zone, "did:dev:abcdefg").is_err());
        assert!(resolver.classify(&zone, "did:key:z6Mk").is_err());
    }

    #[test]
    fn parse_doc_type_maps_default_aliases() {
        assert!(matches!(parse_doc_type(None), DidDocType::Zone));
        assert!(matches!(
            parse_doc_type(Some("did".to_string())),
            DidDocType::Zone
        ));
        assert!(matches!(
            parse_doc_type(Some("".to_string())),
            DidDocType::Zone
        ));
        assert!(matches!(
            parse_doc_type(Some("device".to_string())),
            DidDocType::Device
        ));
        match parse_doc_type(Some("agent".to_string())) {
            DidDocType::Custom(t) => assert_eq!(t, "agent"),
            other => panic!("unexpected doc type {:?}", other.as_str()),
        }
    }

    #[test]
    fn escape_cache_segment_is_unambiguous_and_readable() {
        // 常规 DID 原样保持人类可读
        assert_eq!(
            ZoneDidResolver::escape_cache_segment("did:bns:alice"),
            "did:bns:alice"
        );
        // did:web 端口写法里的 '%' 必须转义，避免与转义序列混淆
        assert_eq!(
            ZoneDidResolver::escape_cache_segment("did:web:example.com%3A3000"),
            "did:web:example.com%253A3000"
        );
        // '/' 不能穿透 store 键层级
        assert_eq!(ZoneDidResolver::escape_cache_segment("a/b"), "a%2Fb");
        let base = ZoneDidResolver::cache_key_base(&DID::new("bns", "alice"), &DidDocType::Owner);
        assert_eq!(base, "resolver/cache/did:bns:alice/owner");
    }

    #[test]
    fn published_envelope_carries_status_and_anchor() {
        let jwt = "header.payload.sig".to_string();
        let answer = PublishedAnswer {
            status: PublishedStatus::Active,
            document: Some(EncodedDocument::Jwt(jwt.clone())),
            doc_type: DidDocType::Device,
            version: Some(7),
            effective_owner: Some(DID::new("bns", "testowner")),
            authority_seq: Some(9),
            doc_hash: None,
            with_hash: true,
            migration_target: None,
        };
        let envelope = ZoneDidResolver::published_envelope(&answer);
        assert_eq!(
            envelope["didResolutionMetadata"]["contentType"],
            "application/did+jwt"
        );
        assert_eq!(envelope["didDocument"], Value::String(jwt));
        let buckyos = &envelope["didDocumentMetadata"]["buckyos"];
        assert_eq!(buckyos["documentStatus"], "active");
        assert_eq!(buckyos["docType"], "device");
        assert_eq!(buckyos["documentVersion"], 7);
        assert_eq!(buckyos["effectiveOwner"], "did:bns:testowner");
        assert_eq!(buckyos["authoritySeq"], 9);
        assert!(buckyos["docHash"].as_str().unwrap().starts_with("sha256:"));
        assert_eq!(envelope["didDocumentMetadata"]["versionId"], "7");
        assert_eq!(envelope["didDocumentMetadata"]["deactivated"], false);
    }

    #[test]
    fn published_envelope_pins_cache_doc_hash_over_computed() {
        let answer = PublishedAnswer {
            status: PublishedStatus::Active,
            document: Some(EncodedDocument::JsonLd(json!({"marker": "doc"}))),
            doc_type: DidDocType::Owner,
            version: None,
            effective_owner: None,
            authority_seq: None,
            doc_hash: Some("sha256:pinned".to_string()),
            with_hash: true,
            migration_target: None,
        };
        let envelope = ZoneDidResolver::published_envelope(&answer);
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["docHash"],
            "sha256:pinned"
        );
    }

    #[test]
    fn missing_envelope_is_answer_not_transport_error() {
        let envelope = ZoneDidResolver::missing_envelope(&DidDocType::Owner);
        assert_eq!(envelope["didResolutionMetadata"]["error"], "notFound");
        assert_eq!(envelope["didDocument"], Value::Null);
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["documentStatus"],
            "missing"
        );
    }

    #[test]
    fn answer_status_codes_follow_contract() {
        // JWT 文档：进 DID Resolution Result 信封（didDocument 为 JWT 字符串）。
        // 旧的裸 JWT 兼容形态已随 name-client（buckyos / cyfs-gateway 均已
        // 升级到含 ZoneResolverClient 的版本）移除。
        let active_jwt = ZoneDidResolver::active(
            EncodedDocument::Jwt("a.b.c".to_string()),
            &DidDocType::Device,
            None,
            false,
        );
        let (status, content_type, body) = ZoneDidResolver::answer_to_response("x", active_jwt);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, CONTENT_TYPE_RESOLUTION);
        let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(envelope["didDocument"], "a.b.c");
        assert_eq!(
            envelope["didResolutionMetadata"]["contentType"],
            "application/did+jwt"
        );
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["documentStatus"],
            "active"
        );

        // JSON 文档：完整 DID Resolution Result 信封
        let active_json = ZoneDidResolver::active(
            EncodedDocument::JsonLd(json!({"marker": "doc"})),
            &DidDocType::Info,
            None,
            false,
        );
        let (status, content_type, body) = ZoneDidResolver::answer_to_response("x", active_json);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, CONTENT_TYPE_RESOLUTION);
        let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(envelope["didDocument"]["marker"], "doc");
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["documentStatus"],
            "active"
        );
        // Info 类没有"已发布 body"语义，不带 docHash
        assert!(envelope["didDocumentMetadata"]["buckyos"]
            .get("docHash")
            .is_none());

        let (status, _, _) =
            ZoneDidResolver::answer_to_response("x", ZoneAnswer::Missing(DidDocType::Device));
        assert_eq!(status, StatusCode::NOT_FOUND);

        // zone 外名字必须是 503：客户端只对 502/503/504 回退本机解析
        let (status, _, _) =
            ZoneDidResolver::answer_to_response("x", ZoneAnswer::NoOpinion("not ours".to_string()));
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, _, _) =
            ZoneDidResolver::answer_to_response("x", ZoneAnswer::BadRequest("bad".to_string()));
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // iat 历史查询：501 + historicalQuerySupported=false（协议 §7）
        let (status, _, body) = ZoneDidResolver::answer_to_response(
            "x",
            ZoneAnswer::HistoricalNotSupported("no history".to_string()),
        );
        assert_eq!(status, StatusCode::NOT_IMPLEMENTED);
        let value: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(value["buckyos"]["historicalQuerySupported"], false);

        let (status, _, _) =
            ZoneDidResolver::answer_to_response("x", ZoneAnswer::Internal("boom".to_string()));
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let (status, _, body) =
            ZoneDidResolver::answer_to_response("self", ZoneAnswer::Bare(json!({"hostname": "h"})));
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("hostname"));
    }

    // ---------------- resolver/cache ----------------

    #[test]
    fn cache_active_entry_requires_inline_doc() {
        let state = json!({"document_status": "active"}).to_string();
        assert!(ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Owner).is_err());

        let doc = json!({"marker": "cached", "iat": 3}).to_string();
        let answer =
            ZoneDidResolver::cache_answer(state.as_str(), Some(doc), &DidDocType::Owner).unwrap();
        let ZoneAnswer::Published(published) = answer else {
            panic!("expected published answer");
        };
        assert_eq!(published.status, PublishedStatus::Active);
        // state 未给 document_version 时回落到文档 revision iat
        assert_eq!(published.version, Some(3));
    }

    #[test]
    fn document_version_falls_back_to_exp_derived_iat() {
        let document = EncodedDocument::JsonLd(json!({
            "exp": DEFAULT_EXPIRE_TIME + 7,
        }));
        assert_eq!(ZoneDidResolver::doc_version(&document), Some(7));
    }

    #[test]
    fn cache_state_fields_flow_into_envelope() {
        let state = json!({
            "document_status": "active",
            "document_version": 5,
            "effective_owner": "did:bns:alice",
            "authority_seq": 2,
            "doc_hash": "sha256:pinned",
            "updated_at": 0,
            "updated_by": "admin",
        })
        .to_string();
        let doc = json!({"marker": "cached", "iat": 5}).to_string();
        let answer =
            ZoneDidResolver::cache_answer(state.as_str(), Some(doc), &DidDocType::Owner).unwrap();
        let (status, content_type, body) = ZoneDidResolver::answer_to_response("x", answer);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, CONTENT_TYPE_RESOLUTION);
        let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
        let buckyos = &envelope["didDocumentMetadata"]["buckyos"];
        assert_eq!(buckyos["documentVersion"], 5);
        assert_eq!(buckyos["effectiveOwner"], "did:bns:alice");
        assert_eq!(buckyos["authoritySeq"], 2);
        assert_eq!(buckyos["docHash"], "sha256:pinned");
    }

    #[test]
    fn cache_rejects_document_version_that_differs_from_iat() {
        let state = json!({
            "document_status": "active",
            "document_version": 6,
        })
        .to_string();
        let doc = json!({"marker": "cached", "iat": 5}).to_string();
        let error = ZoneDidResolver::cache_answer(state.as_str(), Some(doc), &DidDocType::Owner)
            .err()
            .unwrap();
        assert!(error.contains("does not match document iat"));
    }

    #[test]
    fn cache_missing_is_strong_negative_answer() {
        let state = json!({"document_status": "missing"}).to_string();
        let answer =
            ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).unwrap();
        let (status, _, body) = ZoneDidResolver::answer_to_response("x", answer);
        assert_eq!(status, StatusCode::NOT_FOUND);
        let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["documentStatus"],
            "missing"
        );
    }

    #[test]
    fn cache_revocation_maps_to_410_with_deactivated() {
        for negative in ["revoked", "tombstoned"] {
            let state = json!({"document_status": negative}).to_string();
            let answer =
                ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).unwrap();
            let (status, _, body) = ZoneDidResolver::answer_to_response("x", answer);
            assert_eq!(status, StatusCode::GONE);
            let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
            assert_eq!(envelope["didDocumentMetadata"]["deactivated"], true);
            assert_eq!(
                envelope["didDocumentMetadata"]["buckyos"]["documentStatus"],
                negative
            );
        }
    }

    #[test]
    fn cache_migrated_requires_target_and_expired_allows_missing_doc() {
        let state = json!({"document_status": "migrated"}).to_string();
        assert!(ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).is_err());

        let state = json!({
            "document_status": "migrated",
            "migration_target": "did:bns:new-home",
        })
        .to_string();
        let answer =
            ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).unwrap();
        let (status, _, body) = ZoneDidResolver::answer_to_response("x", answer);
        assert_eq!(status, StatusCode::OK);
        let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["migrationTarget"],
            "did:bns:new-home"
        );

        let state = json!({"document_status": "expired"}).to_string();
        let answer =
            ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).unwrap();
        let (status, _, body) = ZoneDidResolver::answer_to_response("x", answer);
        assert_eq!(status, StatusCode::OK);
        let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["documentStatus"],
            "expired"
        );
        assert_eq!(envelope["didDocument"], Value::Null);
    }

    #[test]
    fn cache_answer_rejects_corrupt_state() {
        assert!(ZoneDidResolver::cache_answer("not json", None, &DidDocType::Zone).is_err());
        let state = json!({"document_status": "banana"}).to_string();
        assert!(ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).is_err());
        let state = json!({"other": 1}).to_string();
        assert!(ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).is_err());
    }

    // ---------------- zone owner 的 Root Trust 锚定 ----------------

    #[test]
    fn owner_doc_with_matching_key_is_served_verbatim() {
        let zone = test_zone_identity();
        let owner_doc = owner_doc_with_key(TEST_JWK);
        let encoded = EncodedDocument::Jwt("stored.owner.jwt".to_string());
        let answer =
            ZoneDidResolver::guarded_owner_active(&zone, owner_doc, encoded, &DidDocType::Owner);
        let ZoneAnswer::Published(published) = answer else {
            panic!("expected published answer");
        };
        // key material 一致：原样返回（签名保留）
        assert_eq!(
            published.document,
            Some(EncodedDocument::Jwt("stored.owner.jwt".to_string()))
        );
        assert_eq!(
            published.effective_owner,
            Some(DID::new("bns", "testowner"))
        );
    }

    #[test]
    fn owner_doc_with_rotated_key_is_merged_onto_local_root_trust() {
        let zone = test_zone_identity();
        let owner_doc = owner_doc_with_key(OTHER_JWK);
        let encoded_value = serde_json::to_value(&owner_doc).unwrap();
        let answer = ZoneDidResolver::guarded_owner_active(
            &zone,
            owner_doc,
            EncodedDocument::JsonLd(encoded_value),
            &DidDocType::Owner,
        );
        let ZoneAnswer::Published(published) = answer else {
            panic!("expected published answer");
        };
        let Some(EncodedDocument::JsonLd(merged)) = published.document else {
            panic!("expected merged json document");
        };
        // key material 锚定本地 Root Trust（zone doc 的默认 key）
        let expected_jwk: Value = serde_json::from_str(TEST_JWK).unwrap();
        assert_eq!(
            merged["verificationMethod"][0]["publicKeyJwk"],
            expected_jwk
        );
        assert_eq!(merged["authentication"], json!(["#main_key"]));
        // 非密钥 profile 字段来自权威源
        assert_eq!(merged["id"], "did:bns:testowner");
        assert_eq!(merged["avatar"], "https://example.com/avatar.png");
        // keyScope 回落到本地基线（清空）：扩 key scope 必须走维护流程
        assert!(merged.get("keyScope").is_none());
        assert!(merged.get("buckyos:scopes").is_none());
    }

    #[test]
    fn minimal_zone_owner_doc_projects_local_root_trust() {
        let zone = test_zone_identity();
        let minimal = ZoneDidResolver::minimal_zone_owner_doc(&zone).unwrap();
        assert_eq!(minimal["id"], "did:bns:testowner");
        let expected_jwk: Value = serde_json::from_str(TEST_JWK).unwrap();
        assert_eq!(
            minimal["verificationMethod"][0]["publicKeyJwk"],
            expected_jwk
        );
        let binded = minimal["binded_zone_list"].as_array().unwrap();
        assert_eq!(binded[0], "did:web:test.buckyos.io");
    }

    #[test]
    fn cache_hit_for_zone_owner_doc_is_also_guarded() {
        let zone = test_zone_identity();
        let rotated = owner_doc_with_key(OTHER_JWK);
        let doc_str = serde_json::to_string(&serde_json::to_value(&rotated).unwrap()).unwrap();
        let state = json!({
            "document_status": "active",
            "doc_hash": "sha256:pinned-by-admin",
        })
        .to_string();
        let answer =
            ZoneDidResolver::cache_answer(state.as_str(), Some(doc_str), &DidDocType::Owner)
                .unwrap();
        let guarded = ZoneDidResolver::guard_cache_hit(&zone, answer, &DidDocType::Owner);
        let ZoneAnswer::Published(published) = guarded else {
            panic!("expected published answer");
        };
        let Some(EncodedDocument::JsonLd(merged)) = &published.document else {
            panic!("expected merged json document");
        };
        let expected_jwk: Value = serde_json::from_str(TEST_JWK).unwrap();
        assert_eq!(
            merged["verificationMethod"][0]["publicKeyJwk"],
            expected_jwk
        );
        // body 被替换后，管理员钉的 doc_hash 不再成立，必须改为对返回 body 现算
        assert!(published.doc_hash.is_none());
        assert!(published.with_hash);

        // 非 zone owner 的 cache 命中不受影响
        let other_state = json!({"document_status": "active"}).to_string();
        let other_doc = serde_json::to_string(
            &serde_json::to_value(&OwnerDocument::new(
                DID::new("bns", "alice"),
                "alice".to_string(),
                "Alice".to_string(),
                serde_json::from_str(OTHER_JWK).unwrap(),
            ))
            .unwrap(),
        )
        .unwrap();
        let answer = ZoneDidResolver::cache_answer(
            other_state.as_str(),
            Some(other_doc),
            &DidDocType::Owner,
        )
        .unwrap();
        let untouched = ZoneDidResolver::guard_cache_hit(&zone, answer, &DidDocType::Owner);
        let ZoneAnswer::Published(published) = untouched else {
            panic!("expected published answer");
        };
        let Some(EncodedDocument::JsonLd(doc)) = &published.document else {
            panic!("expected json document");
        };
        let other_jwk: Value = serde_json::from_str(OTHER_JWK).unwrap();
        assert_eq!(doc["verificationMethod"][0]["publicKeyJwk"], other_jwk);
    }

    // ---------------- 静态发布面 ----------------

    #[test]
    fn parse_well_known_file_maps_suffixes() {
        assert_eq!(
            parse_well_known_file("did.json").unwrap(),
            ("did".to_string(), false)
        );
        assert_eq!(
            parse_well_known_file("boot.jwt").unwrap(),
            ("boot".to_string(), true)
        );
        assert_eq!(
            parse_well_known_file("device").unwrap(),
            ("device".to_string(), false)
        );
        assert!(parse_well_known_file("foo.bar.json").is_err());
        assert!(parse_well_known_file("foo.png").is_err());
        assert!(parse_well_known_file(".json").is_err());
        assert!(parse_well_known_file("a/b").is_err());
    }

    #[test]
    fn static_surface_serves_bare_documents_without_envelope() {
        // JSON 文档 → 裸 JSON body
        let active_json = ZoneDidResolver::active(
            EncodedDocument::JsonLd(json!({"marker": "zone-doc"})),
            &DidDocType::Zone,
            None,
            true,
        );
        let (status, content_type, body) =
            ZoneDidResolver::answer_to_static_response("x", active_json, false);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, CONTENT_TYPE_DID_JSON);
        let value: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(value["marker"], "zone-doc");
        assert!(value.get("didDocumentMetadata").is_none());

        // JWT 文档 → JWT 原文
        let active_jwt = ZoneDidResolver::active(
            EncodedDocument::Jwt("a.b.c".to_string()),
            &DidDocType::Boot,
            None,
            true,
        );
        let (status, content_type, body) =
            ZoneDidResolver::answer_to_static_response("x", active_jwt, false);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, CONTENT_TYPE_DID_JWT);
        assert_eq!(body, "a.b.c");

        // .jwt 强类型入口对 JSON 文档 404
        let active_json = ZoneDidResolver::active(
            EncodedDocument::JsonLd(json!({"marker": "zone-doc"})),
            &DidDocType::Zone,
            None,
            true,
        );
        let (status, _, _) = ZoneDidResolver::answer_to_static_response("x", active_json, true);
        assert_eq!(status, StatusCode::NOT_FOUND);

        // miss 是裸 404，不带 resolution 信封（静态面没有状态语义）
        let (status, _, body) = ZoneDidResolver::answer_to_static_response(
            "x",
            ZoneAnswer::Missing(DidDocType::Device),
            false,
        );
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(!body.contains("didDocumentMetadata"));

        // 吊销登记在静态面映射为裸 410
        let state = json!({"document_status": "revoked"}).to_string();
        let answer =
            ZoneDidResolver::cache_answer(state.as_str(), None, &DidDocType::Zone).unwrap();
        let (status, _, _) = ZoneDidResolver::answer_to_static_response("x", answer, false);
        assert_eq!(status, StatusCode::GONE);
    }
}
