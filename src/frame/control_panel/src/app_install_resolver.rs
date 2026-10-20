//! App Installer 的 Resolve 层：identifier 归一化 + `(App DID, "app")` 解析
//! 适配与 candidate 绑定校验（doc/App 安装协议.md §2、§9）。
//!
//! 生产实现只是 name-client `resolve_did_ex` 的 adapter：验证算法（expected
//! owner、doc_hash、iat 时刻 key、valid_iat）全部由 name-client 完成，
//! 这里不复制第二套验签，只做协议状态映射与硬约束复查。

use async_trait::async_trait;
#[cfg(test)]
use buckyos_api::AppDoc;
use buckyos_api::{
    AppDocType, DidCacheStatus, DidEvidenceLevel, DidResolutionSnapshot, DidVerificationStatus,
    DocumentStatus, InstallError, InstallErrorCode, InstallPolicy, InstallStage, InstallUserAction,
    OBJ_TYPE_APP_DOC,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::warn;
use name_lib::DID;
use ndn_lib::{build_named_object_by_json, ObjId};
use serde_json::Value;

pub const APP_DID_DOC_TYPE: &str = "app";

// ---------------------------------------------------------------------------
// identifier 归一化（P2.1）
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum NormalizedIdentifier {
    /// 直接得到 App DID，可立即进入标准解析。
    AppDid(DID),
    /// App Document Object ID：先做无副作用的最小 Acquisition 取 candidate。
    ObjectId(ObjId),
    /// App Document / pikg URL：同上。
    Url(String),
    /// 权威域名别名。必须经 name-client 查询 TXT 后才能得到 App DID。
    DomainAlias(String),
}

fn is_key_class_did(did: &DID) -> bool {
    matches!(did.method.as_str(), "key" | "dev")
}

pub fn normalize_identifier(raw: &str) -> Result<NormalizedIdentifier, InstallError> {
    let raw = raw.trim();
    let invalid_request = |message: String| {
        InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::InvalidRequest,
            false,
            message,
        )
    };

    if raw.is_empty() {
        return Err(invalid_request("identifier is empty".to_string()));
    }

    if raw.starts_with("did:") {
        let did = DID::from_str(raw)
            .map_err(|err| invalid_request(format!("invalid did identifier `{raw}`: {err}")))?;
        if is_key_class_did(&did) {
            // key 类 DID 不能作为 App resolve 输入（协议 §1.3）。
            return Err(invalid_request(format!(
                "key-class did `{raw}` can not be used as an app identifier"
            )));
        }
        return Ok(NormalizedIdentifier::AppDid(did));
    }

    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("cyfs://") {
        return Ok(NormalizedIdentifier::Url(raw.to_string()));
    }

    if let Ok(obj_id) = ObjId::new(raw) {
        return Ok(NormalizedIdentifier::ObjectId(obj_id));
    }

    // 含点裸名是权威域名别名，不能直接拼成 did:bns:*。
    if raw.contains('/') || raw.contains('\\') || raw.contains(char::is_whitespace) {
        return Err(invalid_request(format!(
            "identifier `{raw}` is not a did/name/objid/url"
        )));
    }
    if raw.contains('.') {
        return Ok(NormalizedIdentifier::DomainAlias(raw.to_ascii_lowercase()));
    }
    // 无点短名才按 BNS short name 处理。
    Ok(NormalizedIdentifier::AppDid(DID::new("bns", raw)))
}

fn app_did_from_alias_txt(value: &str) -> Option<DID> {
    let value = value.trim().trim_matches('"');
    let raw = value
        .strip_prefix("buckyos-app-did=")
        .or_else(|| value.strip_prefix("app-did="))
        .unwrap_or(value)
        .trim();
    if !raw.starts_with("did:") {
        return None;
    }
    DID::from_str(raw).ok().filter(|did| !is_key_class_did(did))
}

pub async fn resolve_domain_alias(alias: &str) -> Result<DID, InstallError> {
    let invalid = |message: String| {
        InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::InvalidRequest,
            false,
            message,
        )
    };
    let answer = name_client::resolve(alias, Some(name_client::RecordType::TXT))
        .await
        .map_err(|error| {
            invalid(format!(
                "authoritative domain alias `{alias}` could not be resolved: {error}"
            ))
        })?;
    let mut dids = answer
        .txt
        .iter()
        .filter_map(|value| app_did_from_alias_txt(value))
        .collect::<Vec<_>>();
    dids.sort_by_key(DID::to_string);
    dids.dedup();
    match dids.len() {
        1 => Ok(dids.remove(0)),
        0 => Err(invalid(format!(
            "authoritative domain alias `{alias}` has no buckyos-app-did TXT record"
        ))),
        _ => Err(InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::AmbiguousAppTarget,
            false,
            format!("authoritative domain alias `{alias}` names multiple App DIDs"),
        )),
    }
}

// ---------------------------------------------------------------------------
// resolver 抽象
// ---------------------------------------------------------------------------

/// resolver 输出：协议快照 + resolver 已验证的 body（若有）。
#[derive(Debug, Clone)]
pub struct ResolvedApp {
    pub snapshot: DidResolutionSnapshot,
    /// resolver 验证管线给出的 body（Anchored 或验证通过的 NeedProof）。
    /// `Missing/Unknown/Revoked` 等无 body 场景为 None。
    pub document_value: Option<Value>,
}

/// 内部 resolver 抽象：生产实现包 name-client；单元测试使用 fake，
/// 不初始化全局真实 resolver（P2.2）。
#[async_trait]
pub trait AppDidResolver: Send + Sync {
    async fn resolve_app(
        &self,
        app_did: &DID,
        policy: InstallPolicy,
    ) -> Result<ResolvedApp, InstallError>;
}

// ---------------------------------------------------------------------------
// 快照硬约束复查与 candidate 绑定
// ---------------------------------------------------------------------------

/// resolver 结果的硬约束复查：`document.did == app_did` 与 Object ID 绑定。
/// `doc_type` 由 `AppDocType` 在反序列化时强制；其余约束在此做纵深复查。
pub fn enforce_resolution_invariants(
    app_did: &DID,
    resolved: &ResolvedApp,
) -> Result<(), InstallError> {
    let snapshot = &resolved.snapshot;
    if snapshot.app_did != *app_did {
        return Err(InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::Internal,
            false,
            format!(
                "resolver answered for `{}` but `{}` was asked",
                snapshot.app_did.to_string(),
                app_did.to_string()
            ),
        ));
    }
    if let Some(value) = resolved.document_value.as_ref() {
        let body_did = value
            .get("did")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if body_did != app_did.to_string() {
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::VerificationFailed,
                false,
                format!(
                    "resolved document.did `{body_did}` != app did `{}`",
                    app_did.to_string()
                ),
            ));
        }
        // 快照的 object id 必须与 body 重算一致。
        let (computed, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, value);
        if let Some(snapshot_id) = snapshot.app_doc_object_id.as_ref() {
            if *snapshot_id != computed {
                return Err(InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::VerificationFailed,
                    false,
                    format!(
                        "snapshot app_doc_object_id `{snapshot_id}` != recomputed `{computed}`"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// candidate body（来自 pikg/URL/ObjectId）与权威解析结果的绑定检查。
///
/// - `document.did == app_did` 永远强制；
/// - candidate 自声明 owner 只用于一致性检查：expected_owner 已知且不一致
///   必须拒绝并记录高风险（实现规则 4）；
/// - 返回 candidate 的 canonical Object ID；它是否为当前发布文档由调用方
///   根据权威解析结果决定，不影响 candidate 作为内容载体复用。
pub fn bind_candidate_document(
    app_did: &DID,
    snapshot: &DidResolutionSnapshot,
    candidate_value: &Value,
) -> Result<ObjId, InstallError> {
    let candidate_did = candidate_value
        .get("did")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if candidate_did != app_did.to_string() {
        return Err(InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::VerificationFailed,
            false,
            format!(
                "candidate document.did `{candidate_did}` != app did `{}`",
                app_did.to_string()
            ),
        )
        .with_action(InstallUserAction::None));
    }

    if let Some(expected_owner) = snapshot.expected_owner.as_ref() {
        let declared_owner = candidate_value
            .get("owner")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !declared_owner.is_empty() && declared_owner != expected_owner.to_string() {
            warn!(
                "candidate for `{}` declares owner `{}` but expected owner is `{}`",
                app_did.to_string(),
                declared_owner,
                expected_owner.to_string()
            );
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::VerificationFailed,
                false,
                format!(
                    "candidate declared owner `{declared_owner}` != expected owner `{}`",
                    expected_owner.to_string()
                ),
            ));
        }
    }

    Ok(build_named_object_by_json(OBJ_TYPE_APP_DOC, candidate_value).0)
}

// ---------------------------------------------------------------------------
// 生产 adapter（name-client）
// ---------------------------------------------------------------------------

pub struct NameClientAppResolver;

impl NameClientAppResolver {
    pub fn new() -> Self {
        Self
    }

    /// 重构后的 resolver 不再提供"未验证兜底 / Missing 自签名入场"开关:resolve
    /// 永远只返回验证通过的文档。TrustedShare / LocalDeveloper 场景下本机安装的
    /// App Document 由 `app_installer` 写入本机 locker(hosts 语义),因此这里对所有
    /// 策略使用同一套默认选项。
    fn resolve_opts_for(_policy: InstallPolicy) -> name_client::ResolveOpts {
        name_client::ResolveOpts::default()
    }

    /// AppDIDs under the current Zone (the App each Agent is built into shares
    /// the AgentDID) are only known through the local locker; the Zone locker
    /// would answer them Missing before the local locker is read.
    fn is_current_zone_subname(app_did: &DID) -> bool {
        buckyos_api::get_buckyos_api_runtime()
            .ok()
            .is_some_and(|runtime| app_did.upper_did().as_ref() == Some(&runtime.zone_id))
    }

    /// DID 级 / 类型级状态 → 协议状态。`invalid`(权威确定性校验拒绝)对安装器
    /// 等同 Missing。
    fn map_result_status(result: &name_client::DocResult) -> DocumentStatus {
        match result.did_state.as_ref() {
            Some(name_client::DidState::Tombstoned) => return DocumentStatus::Tombstoned,
            Some(name_client::DidState::Migrated { .. }) => return DocumentStatus::Migrated,
            _ => {}
        }
        match result.type_state.as_ref() {
            Some(name_client::TypeState::Revoked) => DocumentStatus::Revoked,
            Some(name_client::TypeState::Missing) | Some(name_client::TypeState::Invalid { .. }) => {
                DocumentStatus::Missing
            }
            Some(name_client::TypeState::Expired) => DocumentStatus::Expired,
            Some(name_client::TypeState::Active) => DocumentStatus::Active,
            // 没有权威回答 != Missing(实现规则 5):有验证通过的选择即 Active。
            None => {
                if result.select(&name_client::Revision::current()).is_some() {
                    DocumentStatus::Active
                } else {
                    DocumentStatus::Unknown
                }
            }
        }
    }

    fn map_evidence(choice: &name_client::Choice) -> DidEvidenceLevel {
        if choice.report.is_published || choice.source.tier == name_client::SourceTier::Authority {
            DidEvidenceLevel::Anchored
        } else {
            DidEvidenceLevel::NeedProof
        }
    }

    fn map_verification(choice: &name_client::Choice) -> DidVerificationStatus {
        if choice.report.is_signed || choice.source.tier != name_client::SourceTier::Normal {
            DidVerificationStatus::Passed
        } else {
            DidVerificationStatus::NotAttempted
        }
    }

    fn map_resolve_error_status(error: &name_lib::NSError) -> DocumentStatus {
        match error {
            name_lib::NSError::NotFound(_) => DocumentStatus::Missing,
            name_lib::NSError::Disabled(_) => DocumentStatus::Revoked,
            _ => DocumentStatus::Unknown,
        }
    }
}

#[async_trait]
impl AppDidResolver for NameClientAppResolver {
    async fn resolve_app(
        &self,
        app_did: &DID,
        policy: InstallPolicy,
    ) -> Result<ResolvedApp, InstallError> {
        let mut resolve_opts = Self::resolve_opts_for(policy);
        if Self::is_current_zone_subname(app_did) {
            resolve_opts = resolve_opts.without_zone_lock();
        }
        let doc_type = name_client::DidDocType::Custom(APP_DID_DOC_TYPE.to_string());

        let result = name_client::resolve_did_ex(
            app_did,
            Some(doc_type),
            name_client::Revision::current(),
            resolve_opts,
        )
        .await;

        let resolved = match result {
            Ok(resolved) => resolved,
            Err(err) => {
                let document_status = Self::map_resolve_error_status(&err);
                warn!(
                    "resolve_did_ex for `{}` (app) returned {:?}: {}",
                    app_did.to_string(),
                    document_status,
                    err
                );
                return Ok(ResolvedApp {
                    snapshot: DidResolutionSnapshot {
                        app_did: app_did.clone(),
                        doc_type: AppDocType,
                        app_doc_object_id: None,
                        local_authority_app_doc_object_id: None,
                        resolver_id: None,
                        document_status,
                        document_version: None,
                        authority_seq: None,
                        effective_owner: None,
                        expected_owner: app_did.upper_did(),
                        evidence: None,
                        verification_status: None,
                        cache_status: None,
                        doc_hash: None,
                        warnings: vec![format!("resolver error: {err}")],
                        migration_target: None,
                        resolved_at: Some(buckyos_get_unix_timestamp()),
                    },
                    document_value: None,
                });
            }
        };

        let warnings: Vec<String> = resolved.reasons.clone();
        let document_status = Self::map_result_status(&resolved);
        let choice = resolved
            .select(&name_client::Revision::current())
            .cloned();

        let document_value = if matches!(
            document_status,
            DocumentStatus::Active | DocumentStatus::Expired
        ) {
            match choice.as_ref() {
                Some(choice) => match choice.doc.clone().to_json_value() {
                    Ok(value) => Some(value),
                    Err(err) => {
                        return Err(InstallError::new(
                            InstallStage::Resolve,
                            InstallErrorCode::VerificationFailed,
                            false,
                            format!("decode resolved app document failed: {err}"),
                        ))
                    }
                },
                None => None,
            }
        } else {
            None
        };

        let app_doc_object_id = document_value
            .as_ref()
            .map(|value| build_named_object_by_json(OBJ_TYPE_APP_DOC, value).0);

        let migration_target = resolved.migration_target();

        // expected_owner:权威绑定优先,否则名字结构确定性上级;绝不读 candidate 自声明。
        let expected_owner = resolved
            .owner_binding
            .clone()
            .or_else(|| app_did.upper_did());

        let snapshot = DidResolutionSnapshot {
            app_did: app_did.clone(),
            doc_type: AppDocType,
            app_doc_object_id,
            local_authority_app_doc_object_id: None,
            resolver_id: choice.as_ref().map(|c| c.source.id.clone()),
            document_status,
            document_version: choice.as_ref().map(|c| c.revision.iat),
            authority_seq: None,
            effective_owner: resolved.owner_binding.clone(),
            expected_owner,
            evidence: choice.as_ref().map(Self::map_evidence),
            verification_status: choice.as_ref().map(Self::map_verification),
            // 本机 locker 回答(原 LocalAuthorityOverride)按本机命中标记;其余不区分。
            cache_status: if resolved.locked {
                Some(DidCacheStatus::Hit)
            } else {
                None
            },
            doc_hash: choice.as_ref().map(|c| c.revision.hash.clone()),
            warnings,
            migration_target,
            resolved_at: Some(buckyos_get_unix_timestamp()),
        };

        let resolved_app = ResolvedApp {
            snapshot,
            document_value,
        };
        enforce_resolution_invariants(app_did, &resolved_app)?;
        Ok(resolved_app)
    }
}

// ---------------------------------------------------------------------------
// 测试 fake
// ---------------------------------------------------------------------------

#[cfg(test)]
pub mod fake {
    use super::*;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    /// 单元测试用 fake resolver：按 DID 字符串预置回答，并统计调用次数
    /// （offline 断言零网络调用即断言零 resolve 调用之外的 fetch）。
    pub struct FakeAppResolver {
        answers: Mutex<HashMap<String, ResolvedApp>>,
        pub calls: AtomicU32,
    }

    impl FakeAppResolver {
        pub fn new() -> Self {
            Self {
                answers: Mutex::new(HashMap::new()),
                calls: AtomicU32::new(0),
            }
        }

        pub fn set_answer(&self, app_did: &DID, answer: ResolvedApp) {
            self.answers
                .lock()
                .unwrap()
                .insert(app_did.to_string(), answer);
        }
    }

    #[async_trait]
    impl AppDidResolver for FakeAppResolver {
        async fn resolve_app(
            &self,
            app_did: &DID,
            _policy: InstallPolicy,
        ) -> Result<ResolvedApp, InstallError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let answer = self
                .answers
                .lock()
                .unwrap()
                .get(&app_did.to_string())
                .cloned();
            let resolved = answer.unwrap_or_else(|| ResolvedApp {
                snapshot: DidResolutionSnapshot {
                    app_did: app_did.clone(),
                    doc_type: AppDocType,
                    app_doc_object_id: None,
                    local_authority_app_doc_object_id: None,
                    resolver_id: Some("fake".to_string()),
                    document_status: DocumentStatus::Unknown,
                    document_version: None,
                    authority_seq: None,
                    effective_owner: None,
                    expected_owner: app_did.upper_did(),
                    evidence: None,
                    verification_status: None,
                    cache_status: None,
                    doc_hash: None,
                    warnings: vec![],
                    migration_target: None,
                    resolved_at: Some(0),
                },
                document_value: None,
            });
            enforce_resolution_invariants(app_did, &resolved)?;
            Ok(resolved)
        }
    }

    /// 构造一个 Active/Anchored 的标准回答。
    pub fn active_answer(app_did: &DID, doc_value: Value, version: u64) -> ResolvedApp {
        let (obj_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, &doc_value);
        ResolvedApp {
            snapshot: DidResolutionSnapshot {
                app_did: app_did.clone(),
                doc_type: AppDocType,
                app_doc_object_id: Some(obj_id),
                local_authority_app_doc_object_id: None,
                resolver_id: Some("fake".to_string()),
                document_status: DocumentStatus::Active,
                document_version: Some(version),
                authority_seq: Some(version),
                effective_owner: None,
                expected_owner: doc_value
                    .get("owner")
                    .and_then(|v| v.as_str())
                    .and_then(|raw| DID::from_str(raw).ok()),
                evidence: Some(DidEvidenceLevel::Anchored),
                verification_status: Some(DidVerificationStatus::Passed),
                cache_status: Some(DidCacheStatus::ZoneHit),
                doc_hash: None,
                warnings: vec![],
                migration_target: None,
                resolved_at: Some(0),
            },
            document_value: Some(doc_value),
        }
    }

    /// 构造一个指定状态、无 body 的回答（Missing/Revoked/Unknown 等）。
    pub fn status_answer(app_did: &DID, status: DocumentStatus) -> ResolvedApp {
        ResolvedApp {
            snapshot: DidResolutionSnapshot {
                app_did: app_did.clone(),
                doc_type: AppDocType,
                app_doc_object_id: None,
                local_authority_app_doc_object_id: None,
                resolver_id: Some("fake".to_string()),
                document_status: status,
                document_version: None,
                authority_seq: None,
                effective_owner: None,
                expected_owner: app_did.upper_did(),
                evidence: None,
                verification_status: None,
                cache_status: None,
                doc_hash: None,
                warnings: vec![],
                migration_target: None,
                resolved_at: Some(0),
            },
            document_value: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{AppId, AppType, SubPkgDesc};

    fn demo_doc_value(app_did: &str, owner: &str) -> Value {
        let owner_did = DID::from_str(owner).unwrap();
        let app_did = DID::from_str(app_did).unwrap();
        let app_id = AppId::from_app_did(&app_did).unwrap();
        let doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner_did)
            .app_did(app_did)
            .web_pkg(
                SubPkgDesc::new(format!("web.{app_id}#0.1.0"))
                    .package_meta_object_id(ObjId::new_by_raw("pkg".to_string(), vec![1u8; 32])),
            )
            .build()
            .unwrap();
        serde_json::to_value(&doc).unwrap()
    }

    #[test]
    fn normalize_identifier_rules() {
        assert!(matches!(
            normalize_identifier("did:bns:demo.tester").unwrap(),
            NormalizedIdentifier::AppDid(_)
        ));
        assert!(matches!(
            normalize_identifier("filebrowser.buckyos").unwrap(),
            NormalizedIdentifier::DomainAlias(alias) if alias == "filebrowser.buckyos"
        ));
        assert!(matches!(
            normalize_identifier("filebrowser").unwrap(),
            NormalizedIdentifier::AppDid(did) if did.method == "bns"
        ));
        assert!(matches!(
            normalize_identifier("https://example.com/app.pikg").unwrap(),
            NormalizedIdentifier::Url(_)
        ));
        let obj = ObjId::new_by_raw("appdoc".to_string(), vec![1u8; 32]).to_string();
        assert!(matches!(
            normalize_identifier(&obj).unwrap(),
            NormalizedIdentifier::ObjectId(_)
        ));

        // key 类 DID 拒绝。
        assert!(normalize_identifier("did:dev:xxxx").is_err());
        assert!(normalize_identifier("did:key:z6Mk").is_err());
        assert!(normalize_identifier("").is_err());
        assert!(normalize_identifier("a/b").is_err());
    }

    #[test]
    fn authoritative_alias_txt_only_accepts_app_dids() {
        assert_eq!(
            app_did_from_alias_txt("buckyos-app-did=did:bns:demo")
                .unwrap()
                .to_string(),
            "did:bns:demo"
        );
        assert_eq!(
            app_did_from_alias_txt("\"app-did=did:web:demo.example.com\"")
                .unwrap()
                .to_string(),
            "did:web:demo.example.com"
        );
        assert!(app_did_from_alias_txt("https://example.com").is_none());
        assert!(app_did_from_alias_txt("did:key:z6Mk").is_none());
        assert!(app_did_from_alias_txt("did:dev:device").is_none());
    }

    #[test]
    fn candidate_binding_enforces_did_and_owner() {
        let app_did = DID::from_str("did:bns:demo-web.tester").unwrap();
        let doc_value = demo_doc_value("did:bns:demo-web.tester", "did:bns:tester");
        let snapshot = fake::active_answer(&app_did, doc_value.clone(), 1).snapshot;

        // 一致：绑定成功且命中已发布 body。
        let candidate_id = bind_candidate_document(&app_did, &snapshot, &doc_value).unwrap();
        assert_eq!(snapshot.app_doc_object_id.as_ref(), Some(&candidate_id));

        // document.did 不一致必须拒绝。
        let mut wrong_did = doc_value.clone();
        wrong_did["did"] = Value::String("did:bns:other.tester".to_string());
        let err = bind_candidate_document(&app_did, &snapshot, &wrong_did).unwrap_err();
        assert_eq!(err.code, InstallErrorCode::VerificationFailed);

        // candidate 自声明 owner 与 expected_owner 不同必须拒绝。
        let mut wrong_owner = doc_value.clone();
        wrong_owner["owner"] = Value::String("did:bns:mallory".to_string());
        let err = bind_candidate_document(&app_did, &snapshot, &wrong_owner).unwrap_err();
        assert_eq!(err.code, InstallErrorCode::VerificationFailed);
        assert!(err.message.contains("expected owner"));

        // 权威 body 不同（旧版本 candidate）：仍可绑定，但 Object ID 不同。
        let mut older = doc_value.clone();
        older["version"] = Value::String("0.0.9".to_string());
        let older_id = bind_candidate_document(&app_did, &snapshot, &older).unwrap();
        assert_ne!(snapshot.app_doc_object_id.as_ref(), Some(&older_id));
    }

    #[test]
    fn resolver_error_preserves_authoritative_missing() {
        assert_eq!(
            NameClientAppResolver::map_resolve_error_status(&name_lib::NSError::NotFound(
                "app document".to_string(),
            )),
            DocumentStatus::Missing
        );
        assert_eq!(
            NameClientAppResolver::map_resolve_error_status(&name_lib::NSError::Failed(
                "network unavailable".to_string(),
            )),
            DocumentStatus::Unknown
        );
    }

    #[tokio::test]
    async fn fake_resolver_matrix_missing_expired_unknown_are_distinct() {
        let resolver = fake::FakeAppResolver::new();
        let missing_did = DID::from_str("did:bns:missing.tester").unwrap();
        let expired_did = DID::from_str("did:bns:expired.tester").unwrap();
        let unknown_did = DID::from_str("did:bns:unknown.tester").unwrap();
        resolver.set_answer(
            &missing_did,
            fake::status_answer(&missing_did, DocumentStatus::Missing),
        );
        resolver.set_answer(
            &expired_did,
            fake::status_answer(&expired_did, DocumentStatus::Expired),
        );
        // unknown 不设置 → fake 默认 Unknown。

        let missing = resolver
            .resolve_app(&missing_did, InstallPolicy::Normal)
            .await
            .unwrap();
        let expired = resolver
            .resolve_app(&expired_did, InstallPolicy::Normal)
            .await
            .unwrap();
        let unknown = resolver
            .resolve_app(&unknown_did, InstallPolicy::Normal)
            .await
            .unwrap();

        assert_eq!(missing.snapshot.document_status, DocumentStatus::Missing);
        assert_eq!(expired.snapshot.document_status, DocumentStatus::Expired);
        assert_eq!(unknown.snapshot.document_status, DocumentStatus::Unknown);

        // 三者映射到不同错误码。
        let codes: Vec<_> = [&missing, &expired, &unknown]
            .iter()
            .map(|resolved| {
                InstallError::from_document_status(
                    InstallStage::Resolve,
                    resolved.snapshot.document_status,
                    &resolved.snapshot.app_did,
                )
                .code
            })
            .collect();
        assert_eq!(
            codes,
            vec![
                InstallErrorCode::IdentityNotPublished,
                InstallErrorCode::IdentityExpired,
                InstallErrorCode::TrustResolutionRequired,
            ]
        );
    }

    #[tokio::test]
    async fn fake_resolver_rejects_mismatched_document() {
        let resolver = fake::FakeAppResolver::new();
        let app_did = DID::from_str("did:bns:demo-web.tester").unwrap();
        // body 的 did 指向别的 DID：契约检查必须拒绝。
        let bad_value = demo_doc_value("did:bns:other.tester", "did:bns:tester");
        let mut answer = fake::active_answer(&app_did, bad_value, 1);
        answer.snapshot.app_did = app_did.clone();
        resolver.set_answer(&app_did, answer);

        let err = resolver
            .resolve_app(&app_did, InstallPolicy::Normal)
            .await
            .unwrap_err();
        assert_eq!(err.code, InstallErrorCode::VerificationFailed);
    }
}
