// Zone DID Resolver —— "伪装成 resolver 的 zone 内权威 cache"
// （buckyos-base doc/简单介绍resolve-did.md §2.2/§3/§5/§7，速查规则 8）。
//
// 它对外提供标准 resolver HTTP 接口（doc/http_did_resolver_api.md）：
//   GET /1.0/identifiers/{did}?type={doc_type}
//   GET /.well-known/{doc_type}.json   （Host 决定 did；did.json = 默认 doc_type）
// 但语义上不是 resolver-provider：对 zone 内客户端（name-client 的
// ZoneResolverClient / BaseHttpProvider）来说它代表 cluster 的解析控制面，
// 回答一律用 W3C DID Resolution Result 信封携带 documentStatus。
//
// 状态码纪律（客户端按它区分"回答"与"可回退"，是本文件最重要的约束）：
//   200 + documentStatus=active   zone 持有该 (did, doc_type)，独占本次解析；
//   404 + documentStatus=missing  zone 对该名字有权威，明确回答"从未发布"——
//                                 是回答不是查不到，客户端会当强负证据；
//   503                           zone 对该名字没有意见（zone 外名字 / zone 身份
//                                 尚不可用），客户端据此回退本机 cache 与 provider 链；
//   400                           非法入参（key 类 DID 等，规则 10）；
//   500                           zone 内部错误（zone 内条目损坏）——对客户端是
//                                 "zone 回答了 unknown"，仍独占解析，不允许绕过控制面外查。
// 绝不能把"zone 外名字"落进 404/500：404 会被缓存成强负状态，500 会堵死外部解析。

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
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::str::FromStr;
use url::form_urlencoded;

use crate::SYS_STORE;

const AGENT_DOC_TYPE: &str = "agent";
const CONTENT_TYPE_RESOLUTION: &str = "application/did-resolution+json";
const CONTENT_TYPE_JSON: &str = "application/json";

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
}

// 解析目标三分。Foreign 不代表拒答：store 里恰好持有该主体的文档且文档自述 id
// 与查询一致时仍可回答（cluster cache 能看到不对外发布的文档），否则 503。
enum Target {
    ZoneItself,
    InZone(String),
    Foreign(DID),
}

// 一次解析的回答，与文件头的状态码纪律一一对应。
enum ZoneAnswer {
    Active {
        document: EncodedDocument,
        doc_type: DidDocType,
        version: Option<u64>,
        effective_owner: Option<DID>,
        // Info 这类实时数据没有"已发布 body"语义，不带 docHash 锚点
        with_hash: bool,
    },
    // legacy 别名 `self` 专用：裸 ZoneDocument JSON（websdk 等旧消费者读顶层
    // hostname/id），resolve_did 客户端不会用非 DID 入参查询
    Bare(Value),
    Missing(DidDocType),
    NoOpinion(String),
    BadRequest(String),
    Internal(String),
}

fn is_key_class_method(method: &str) -> bool {
    method == "dev" || method == "key"
}

// 目前 sys_config 没有 zone 内吊销登记，Revoked/Tombstoned（410）尚不产生；
// 引入吊销控制面后在 resolve 分派处补 410 信封即可，客户端两代都已按协议支持。

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
        encoded
            .clone()
            .to_json_value()
            .ok()?
            .get("version_seq")?
            .as_u64()
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
        serde_json::to_value(&device_info).map(Some).map_err(|e| {
            format!("serialize device info {} failed: {}", path, e)
        })
    }

    fn active(
        document: EncodedDocument,
        doc_type: &DidDocType,
        effective_owner: Option<DID>,
        with_hash: bool,
    ) -> ZoneAnswer {
        let version = Self::doc_version(&document);
        ZoneAnswer::Active {
            document,
            doc_type: doc_type.clone(),
            version,
            effective_owner,
            with_hash,
        }
    }

    async fn resolve(&self, name: &str, doc_type: &DidDocType) -> ZoneAnswer {
        // zone 身份不可用（未初始化 / store 故障 / 配置损坏）时这个 cache 语义上
        // 就是不可用：503 让客户端回退自己的解析管线，而不是 500 堵死一切。
        let zone = match self.load_zone_identity().await {
            Ok(Some(zone)) => zone,
            Ok(None) => {
                return ZoneAnswer::NoOpinion("zone is not initialized (boot/config not set)".to_string())
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
            Ok(Target::ZoneItself) => self.resolve_zone_itself(&zone, doc_type),
            Ok(Target::InZone(short)) => self.resolve_in_zone(short.as_str(), doc_type).await,
            Ok(Target::Foreign(did)) => self.resolve_foreign(&did, doc_type).await,
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
            // zone 自身没有这种文档；zone 对自己的命名空间是权威的，miss 就是 Missing
            _ => ZoneAnswer::Missing(doc_type.clone()),
        }
    }

    async fn resolve_in_zone(&self, short: &str, doc_type: &DidDocType) -> ZoneAnswer {
        // zone 命名空间内（*.zone_host / 裸短名）的绑定是结构性的：miss 是权威 Missing；
        // store 读失败/条目损坏是 Internal（zone unknown，不允许外查顶替 zone 内名字）。
        let result: Result<Option<ZoneAnswer>, String> = match doc_type {
            DidDocType::Info => self.load_device_info(short).await.map(|info| {
                info.map(|v| Self::active(EncodedDocument::JsonLd(v), doc_type, None, false))
            }),
            DidDocType::Device => self.load_device_doc(short).await.map(|doc| {
                doc.map(|encoded| Self::active(encoded, doc_type, None, true))
            }),
            DidDocType::Owner | DidDocType::User => self.load_owner_doc(short).await.map(|doc| {
                doc.map(|(owner_doc, encoded)| {
                    Self::active(encoded, doc_type, Some(owner_doc.id), true)
                })
            }),
            DidDocType::Custom(ref t) if t == AGENT_DOC_TYPE => {
                self.load_agent_doc(short).await.map(|doc| {
                    doc.map(|(_, encoded)| Self::active(encoded, doc_type, None, true))
                })
            }
            // 默认 doc_type（zone）：typeless 老客户端把设备逻辑名当默认类型查
            //（如 resolve_did(device_did, None) 取 device doc 提 ips），保留
            // device → agent → owner 的 any-doc 兜底。
            DidDocType::Zone => self.load_any_doc(short).await,
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

    async fn load_any_doc(&self, short: &str) -> Result<Option<ZoneAnswer>, String> {
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
            return Ok(Some(Self::active(
                encoded,
                &DidDocType::Owner,
                Some(owner_doc.id),
                true,
            )));
        }
        Ok(None)
    }

    async fn resolve_foreign(&self, did: &DID, doc_type: &DidDocType) -> ZoneAnswer {
        // zone 对 zone 外名字唯一能做的贡献：store 里恰好持有该主体的文档
        //（zone 成员的 owner doc、按真实 DID 注册的 agent doc）。store 键是 zone 内
        // 短名，与全局名同名不同主体时（users/alice vs did:bns:alice）绝不能张冠
        // 李戴——回答前必须核对文档自述 id 与查询 DID 一致，不一致按没有意见处理。
        // 这里的 store/解码失败也降级为没有意见：zone 外名字永远有外部解析可走。
        let hit: Option<ZoneAnswer> = match doc_type {
            DidDocType::Owner | DidDocType::User => self
                .load_owner_doc(did.id.as_str())
                .await
                .ok()
                .flatten()
                .filter(|(owner_doc, _)| owner_doc.id == *did)
                .map(|(owner_doc, encoded)| {
                    Self::active(encoded, doc_type, Some(owner_doc.id), true)
                }),
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
            DidDocType::Zone => self
                .find_agent_doc_by_did(did)
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
                }),
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

    // ---------------- 响应信封 ----------------

    fn sha256_hex(data: &str) -> String {
        let digest = Sha256::digest(data.as_bytes());
        digest.iter().map(|b| format!("{:02x}", b)).collect()
    }

    fn active_envelope(
        document: &EncodedDocument,
        doc_type: &DidDocType,
        version: Option<u64>,
        effective_owner: &Option<DID>,
        with_hash: bool,
    ) -> Value {
        let (content_type, did_document, doc_string) = match document {
            EncodedDocument::Jwt(jwt) => (
                "application/did+jwt",
                Value::String(jwt.clone()),
                jwt.clone(),
            ),
            EncodedDocument::JsonLd(v) => (
                "application/did+ld+json",
                v.clone(),
                serde_json::to_string(v).unwrap_or_default(),
            ),
        };
        let mut buckyos = json!({
            "docType": doc_type.as_str(),
            "documentStatus": "active",
        });
        if let Some(version) = version {
            buckyos["documentVersion"] = json!(version);
        }
        if let Some(owner) = effective_owner {
            buckyos["effectiveOwner"] = json!(owner.to_string());
        }
        if with_hash {
            buckyos["docHash"] = json!(format!("sha256:{}", Self::sha256_hex(doc_string.as_str())));
        }
        let mut document_metadata = json!({
            "deactivated": false,
            "buckyos": buckyos,
        });
        if let Some(version) = version {
            document_metadata["versionId"] = json!(version.to_string());
        }
        json!({
            "didResolutionMetadata": {"contentType": content_type},
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

    fn answer_to_response(name: &str, answer: ZoneAnswer) -> (StatusCode, &'static str, String) {
        match answer {
            ZoneAnswer::Active {
                ref document,
                ref doc_type,
                version,
                ref effective_owner,
                with_hash,
            } => {
                info!(
                    "ZoneDidResolver resolve {}#{} => active",
                    name,
                    doc_type.as_str()
                );
                match document {
                    // JWT 文档返回裸 body：当前仓库同一 build 里的旧 name-client
                    //（BaseHttpProvider::parse_response）只认"信封里的 object"，
                    // 会把信封里的 JWT string 包成 JsonLd(Value::String) 导致解码
                    // 失败；裸 JWT 两代客户端都正确处理（新 ZoneResolverClient 按
                    // 三段式判定并仍作为 ZoneHit）。等 zone 内客户端全部升级后，
                    // JWT 也可以进信封携带 docHash/version 元数据。
                    EncodedDocument::Jwt(jwt) => {
                        (StatusCode::OK, "application/did+jwt", jwt.clone())
                    }
                    EncodedDocument::JsonLd(_) => {
                        let body = Self::active_envelope(
                            document,
                            doc_type,
                            version,
                            effective_owner,
                            with_hash,
                        );
                        (StatusCode::OK, CONTENT_TYPE_RESOLUTION, body.to_string())
                    }
                }
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

        // GET /1.0/identifiers/{did}?type={doc_type}
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

            let typ = req.uri().query().and_then(|q| {
                form_urlencoded::parse(q.as_bytes())
                    .find(|(k, _)| k == "type")
                    .map(|(_, v)| v.into_owned())
            });
            let doc_type = parse_doc_type(typ);

            let answer = self.resolve(did_str.as_str(), &doc_type).await;
            let (status, content_type, body) =
                Self::answer_to_response(did_str.as_str(), answer);
            return build_resp(status, content_type, body);
        }

        // GET http://{did_host_name}/.well-known/{doc_type}.json （did.json = 默认）
        if path.starts_with("/.well-known/") && path.ends_with(".json") {
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

            let doc_type_str = path
                .trim_start_matches("/.well-known/")
                .trim_end_matches(".json")
                .to_string();
            let doc_type = parse_doc_type(Some(doc_type_str));

            let did_str = if DID::is_did(host.as_str()) {
                host.clone()
            } else {
                format!("did:web:{}", host)
            };

            let answer = self.resolve(did_str.as_str(), &doc_type).await;
            let (status, content_type, body) =
                Self::answer_to_response(did_str.as_str(), answer);
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

    const TEST_JWK: &str = r#"{"kty":"OKP","crv":"Ed25519","x":"qJdNEtscIYwTo-I0K7iPEt_UZdBDRd4r16jdBfNR0tM"}"#;

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
    fn active_envelope_carries_status_and_anchor() {
        let jwt = "header.payload.sig".to_string();
        let envelope = ZoneDidResolver::active_envelope(
            &EncodedDocument::Jwt(jwt.clone()),
            &DidDocType::Device,
            Some(7),
            &Some(DID::new("bns", "testowner")),
            true,
        );
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
        assert!(buckyos["docHash"]
            .as_str()
            .unwrap()
            .starts_with("sha256:"));
        assert_eq!(envelope["didDocumentMetadata"]["versionId"], "7");
        assert_eq!(envelope["didDocumentMetadata"]["deactivated"], false);
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
        // JWT 文档：裸 body（新旧两代 name-client 唯一都能正确解码的形态）
        let active_jwt = ZoneAnswer::Active {
            document: EncodedDocument::Jwt("a.b.c".to_string()),
            doc_type: DidDocType::Device,
            version: None,
            effective_owner: None,
            with_hash: false,
        };
        let (status, content_type, body) = ZoneDidResolver::answer_to_response("x", active_jwt);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, "application/did+jwt");
        assert_eq!(body, "a.b.c");

        // JSON 文档：完整 DID Resolution Result 信封
        let active_json = ZoneAnswer::Active {
            document: EncodedDocument::JsonLd(json!({"marker": "doc"})),
            doc_type: DidDocType::Info,
            version: None,
            effective_owner: None,
            with_hash: false,
        };
        let (status, content_type, body) = ZoneDidResolver::answer_to_response("x", active_json);
        assert_eq!(status, StatusCode::OK);
        assert_eq!(content_type, CONTENT_TYPE_RESOLUTION);
        let envelope: Value = serde_json::from_str(body.as_str()).unwrap();
        assert_eq!(envelope["didDocument"]["marker"], "doc");
        assert_eq!(
            envelope["didDocumentMetadata"]["buckyos"]["documentStatus"],
            "active"
        );

        let (status, _, _) =
            ZoneDidResolver::answer_to_response("x", ZoneAnswer::Missing(DidDocType::Device));
        assert_eq!(status, StatusCode::NOT_FOUND);

        // zone 外名字必须是 503：客户端只对 502/503/504 回退本机解析
        let (status, _, _) = ZoneDidResolver::answer_to_response(
            "x",
            ZoneAnswer::NoOpinion("not ours".to_string()),
        );
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

        let (status, _, _) = ZoneDidResolver::answer_to_response(
            "x",
            ZoneAnswer::BadRequest("bad".to_string()),
        );
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (status, _, _) = ZoneDidResolver::answer_to_response(
            "x",
            ZoneAnswer::Internal("boom".to_string()),
        );
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);

        let (status, _, body) =
            ZoneDidResolver::answer_to_response("self", ZoneAnswer::Bare(json!({"hostname": "h"})));
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("hostname"));
    }
}
