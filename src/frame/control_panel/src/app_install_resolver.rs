//! App Installer 的 Resolve 层：identifier 归一化 + `(App DID, "app")` 解析
//! 适配与 candidate 绑定校验（doc/App 安装协议.md §2、§9）。
//!
//! 生产实现只是 name-client `resolve_did_ex` 的 adapter：验证算法（expected
//! owner、doc_hash、iat 时刻 key、valid_iat）全部由 name-client 完成，
//! 这里不复制第二套验签，只做协议状态映射与硬约束复查。

use async_trait::async_trait;
use buckyos_api::{
    AppDoc, DidCacheStatus, DidEvidenceLevel, DidResolutionSnapshot, DidVerificationStatus,
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

    // 裸名视为 BNS 名称。
    if raw.contains('/') || raw.contains('\\') || raw.contains(char::is_whitespace) {
        return Err(invalid_request(format!(
            "identifier `{raw}` is not a did/name/objid/url"
        )));
    }
    Ok(NormalizedIdentifier::AppDid(DID::new("bns", raw)))
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

impl ResolvedApp {
    pub fn document(&self) -> Result<Option<AppDoc>, InstallError> {
        match self.document_value.as_ref() {
            None => Ok(None),
            Some(value) => {
                let doc: AppDoc = serde_json::from_value(value.clone()).map_err(|err| {
                    InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::VerificationFailed,
                        false,
                        format!("resolved app document schema invalid: {err}"),
                    )
                })?;
                Ok(Some(doc))
            }
        }
    }
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

/// resolver 结果的硬约束复查：`document.id == app_did`、doc_type、终止状态。
/// 这些约束 name-client 也会执行；此处复查是纵深防御 + fake 实现的契约测试面。
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
    if snapshot.doc_type != APP_DID_DOC_TYPE {
        return Err(InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::Internal,
            false,
            format!(
                "resolver snapshot doc_type `{}` != `app`",
                snapshot.doc_type
            ),
        ));
    }

    if let Some(value) = resolved.document_value.as_ref() {
        let body_id = value.get("id").and_then(|v| v.as_str()).unwrap_or_default();
        if body_id != app_did.to_string() {
            return Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::VerificationFailed,
                false,
                format!(
                    "resolved document.id `{body_id}` != app did `{}`",
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

/// 终止状态立即产生不可重试错误；清除/屏蔽正候选由调用方（引擎）负责。
pub fn reject_terminal_status(
    app_did: &DID,
    snapshot: &DidResolutionSnapshot,
) -> Result<(), InstallError> {
    if snapshot.document_status.is_terminal() {
        return Err(InstallError::from_document_status(
            InstallStage::Resolve,
            snapshot.document_status,
            app_did,
        ));
    }
    Ok(())
}

/// candidate body（来自 pikg/URL/ObjectId）与权威解析结果的绑定检查。
///
/// - `document.id == app_did` 永远强制；
/// - candidate 自声明 owner 只用于一致性检查：expected_owner 已知且不一致
///   必须拒绝并记录高风险（实现规则 4）；
/// - 权威给出 body 时，candidate 是否等于该 body 只影响"能否把 candidate
///   当作当前发布文档"，不影响 candidate 作为内容载体（内容按 digest 复用）。
pub fn bind_candidate_document(
    app_did: &DID,
    snapshot: &DidResolutionSnapshot,
    candidate_value: &Value,
) -> Result<CandidateBinding, InstallError> {
    let candidate_id = candidate_value
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if candidate_id != app_did.to_string() {
        return Err(InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::VerificationFailed,
            false,
            format!(
                "candidate document.id `{candidate_id}` != app did `{}`",
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

    let (candidate_obj_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, candidate_value);
    let matches_published = snapshot
        .app_doc_object_id
        .as_ref()
        .map(|published| *published == candidate_obj_id)
        .unwrap_or(false);

    Ok(CandidateBinding {
        candidate_obj_id,
        matches_published,
    })
}

#[derive(Debug, Clone)]
pub struct CandidateBinding {
    pub candidate_obj_id: ObjId,
    /// candidate 是否就是权威当前发布的 body。
    pub matches_published: bool,
}

// ---------------------------------------------------------------------------
// 生产 adapter（name-client）
// ---------------------------------------------------------------------------

pub struct NameClientAppResolver;

impl NameClientAppResolver {
    pub fn new() -> Self {
        Self
    }

    fn resolve_policy_for(policy: InstallPolicy) -> name_client::ResolvePolicy {
        let mut resolve_policy = name_client::ResolvePolicy::default();
        match policy {
            InstallPolicy::StrictPublic => {
                resolve_policy.allow_stale_cache = false;
                resolve_policy.allow_unverified_cache_when_unavailable = false;
                resolve_policy.allow_self_signed_when_missing = false;
            }
            InstallPolicy::Normal | InstallPolicy::SystemInternal => {}
            InstallPolicy::TrustedShare => {
                resolve_policy.allow_unverified_cache_when_unavailable = true;
            }
            InstallPolicy::LocalDeveloper => {
                resolve_policy.allow_unverified_cache_when_unavailable = true;
                resolve_policy.allow_self_signed_when_missing = true;
            }
        }
        resolve_policy
    }

    fn map_document_status(status: Option<&name_client::DocumentStatus>) -> DocumentStatus {
        match status {
            Some(name_client::DocumentStatus::Active) => DocumentStatus::Active,
            Some(name_client::DocumentStatus::Missing) => DocumentStatus::Missing,
            Some(name_client::DocumentStatus::Expired) => DocumentStatus::Expired,
            Some(name_client::DocumentStatus::Revoked) => DocumentStatus::Revoked,
            Some(name_client::DocumentStatus::Tombstoned) => DocumentStatus::Tombstoned,
            Some(name_client::DocumentStatus::Migrated) => DocumentStatus::Migrated,
            // 没有权威回答 != Missing（实现规则 5）。
            None => DocumentStatus::Unknown,
        }
    }

    fn map_evidence(evidence: Option<name_client::BodyEvidence>) -> Option<DidEvidenceLevel> {
        evidence.map(|value| match value {
            name_client::BodyEvidence::Anchored => DidEvidenceLevel::Anchored,
            name_client::BodyEvidence::NeedProof => DidEvidenceLevel::NeedProof,
            name_client::BodyEvidence::UnproofInfo => DidEvidenceLevel::UnproofInfo,
        })
    }

    fn map_verification(
        status: Option<name_client::VerificationStatus>,
    ) -> Option<DidVerificationStatus> {
        status.map(|value| match value {
            name_client::VerificationStatus::Passed => DidVerificationStatus::Passed,
            name_client::VerificationStatus::Failed => DidVerificationStatus::Failed,
            name_client::VerificationStatus::Unavailable => DidVerificationStatus::Unavailable,
            name_client::VerificationStatus::NotAttempted => DidVerificationStatus::NotAttempted,
        })
    }

    fn map_cache_status(status: Option<name_client::CacheStatus>) -> Option<DidCacheStatus> {
        status.map(|value| match value {
            name_client::CacheStatus::Disabled => DidCacheStatus::Disabled,
            name_client::CacheStatus::Miss => DidCacheStatus::Miss,
            name_client::CacheStatus::Hit => DidCacheStatus::Hit,
            name_client::CacheStatus::Refresh => DidCacheStatus::Refresh,
            name_client::CacheStatus::Fallback => DidCacheStatus::Fallback,
            name_client::CacheStatus::UnauthenticatedInfoHit => {
                DidCacheStatus::UnauthenticatedInfoHit
            }
            name_client::CacheStatus::ZoneHit => DidCacheStatus::ZoneHit,
            name_client::CacheStatus::ObservedFallback => DidCacheStatus::ObservedFallback,
        })
    }
}

#[async_trait]
impl AppDidResolver for NameClientAppResolver {
    async fn resolve_app(
        &self,
        app_did: &DID,
        policy: InstallPolicy,
    ) -> Result<ResolvedApp, InstallError> {
        let resolve_policy = Self::resolve_policy_for(policy);
        let doc_type = name_client::DidDocType::Custom(APP_DID_DOC_TYPE.to_string());

        let result = name_client::resolve_did_ex(app_did, Some(doc_type), resolve_policy).await;

        let resolved = match result {
            Ok(resolved) => resolved,
            Err(err) => {
                // 解析器没有回答：Unknown（不是 Missing）。
                warn!(
                    "resolve_did_ex for `{}` (app) got no answer: {}",
                    app_did.to_string(),
                    err
                );
                return Ok(ResolvedApp {
                    snapshot: DidResolutionSnapshot {
                        app_did: app_did.clone(),
                        doc_type: APP_DID_DOC_TYPE.to_string(),
                        app_doc_object_id: None,
                        resolver_id: None,
                        document_status: DocumentStatus::Unknown,
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

        let buckyos_meta = &resolved.document_metadata.buckyos;
        let document_status = Self::map_document_status(buckyos_meta.document_status.as_ref());

        let document_value = if matches!(
            document_status,
            DocumentStatus::Active | DocumentStatus::Expired
        ) {
            match resolved.document.clone().to_json_value() {
                Ok(value) => Some(value),
                Err(err) => {
                    return Err(InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::VerificationFailed,
                        false,
                        format!("decode resolved app document failed: {err}"),
                    ))
                }
            }
        } else {
            None
        };

        let app_doc_object_id = document_value
            .as_ref()
            .map(|value| build_named_object_by_json(OBJ_TYPE_APP_DOC, value).0);

        let warnings: Vec<String> = resolved
            .resolution_metadata
            .warnings
            .iter()
            .map(|warning| format!("{warning:?}"))
            .collect();

        // migration_target：name-client 元数据未直接暴露，Migrated 时从 body 提取。
        let migration_target = if matches!(document_status, DocumentStatus::Migrated) {
            resolved
                .document
                .clone()
                .to_json_value()
                .ok()
                .and_then(|value| {
                    value
                        .get("migration_target")
                        .and_then(|v| v.as_str())
                        .and_then(|raw| DID::from_str(raw).ok())
                })
        } else {
            None
        };

        // expected_owner：权威绑定优先，否则名字结构确定性上级；
        // 绝不读 candidate 自声明。
        let expected_owner = app_did.upper_did();

        let snapshot = DidResolutionSnapshot {
            app_did: app_did.clone(),
            doc_type: APP_DID_DOC_TYPE.to_string(),
            app_doc_object_id,
            resolver_id: resolved.resolution_metadata.resolver_id.clone(),
            document_status,
            document_version: buckyos_meta.document_version,
            authority_seq: buckyos_meta.authority_seq,
            effective_owner: None,
            expected_owner,
            evidence: Self::map_evidence(resolved.resolution_metadata.evidence),
            verification_status: Self::map_verification(
                resolved.resolution_metadata.verification_status,
            ),
            cache_status: Self::map_cache_status(resolved.resolution_metadata.cache_status),
            doc_hash: None,
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

#[cfg(any(test, feature = "testing"))]
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
                    doc_type: APP_DID_DOC_TYPE.to_string(),
                    app_doc_object_id: None,
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
                doc_type: APP_DID_DOC_TYPE.to_string(),
                app_doc_object_id: Some(obj_id),
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
                doc_type: APP_DID_DOC_TYPE.to_string(),
                app_doc_object_id: None,
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
    use buckyos_api::{AppType, SubPkgDesc};

    fn demo_doc_value(app_did: &str, owner: &str) -> Value {
        let owner_did = DID::from_str(owner).unwrap();
        let doc = AppDoc::builder(AppType::Web, "demo_web", "0.1.0", "tester", &owner_did)
            .app_did(DID::from_str(app_did).unwrap())
            .web_pkg(SubPkgDesc::new("demo_web-web#0.1.0"))
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
    fn candidate_binding_enforces_id_and_owner() {
        let app_did = DID::from_str("did:bns:demo_web.tester").unwrap();
        let doc_value = demo_doc_value("did:bns:demo_web.tester", "did:bns:tester");
        let snapshot = fake::active_answer(&app_did, doc_value.clone(), 1).snapshot;

        // 一致：绑定成功且命中已发布 body。
        let binding = bind_candidate_document(&app_did, &snapshot, &doc_value).unwrap();
        assert!(binding.matches_published);

        // document.id 不一致必须拒绝。
        let mut wrong_id = doc_value.clone();
        wrong_id["id"] = Value::String("did:bns:other.tester".to_string());
        let err = bind_candidate_document(&app_did, &snapshot, &wrong_id).unwrap_err();
        assert_eq!(err.code, InstallErrorCode::VerificationFailed);

        // candidate 自声明 owner 与 expected_owner 不同必须拒绝。
        let mut wrong_owner = doc_value.clone();
        wrong_owner["owner"] = Value::String("did:bns:mallory".to_string());
        let err = bind_candidate_document(&app_did, &snapshot, &wrong_owner).unwrap_err();
        assert_eq!(err.code, InstallErrorCode::VerificationFailed);
        assert!(err.message.contains("expected owner"));

        // 权威 body 不同（旧版本 candidate）：可绑定但 matches_published=false。
        let mut older = doc_value.clone();
        older["version"] = Value::String("0.0.9".to_string());
        let binding = bind_candidate_document(&app_did, &snapshot, &older).unwrap();
        assert!(!binding.matches_published);
    }

    #[test]
    fn terminal_status_is_not_retryable() {
        let app_did = DID::from_str("did:bns:demo_web.tester").unwrap();
        let revoked = fake::status_answer(&app_did, DocumentStatus::Revoked);
        let err = reject_terminal_status(&app_did, &revoked.snapshot).unwrap_err();
        assert_eq!(err.code, InstallErrorCode::IdentityRevoked);
        assert!(!err.retryable);

        let tombstoned = fake::status_answer(&app_did, DocumentStatus::Tombstoned);
        let err = reject_terminal_status(&app_did, &tombstoned.snapshot).unwrap_err();
        assert_eq!(err.code, InstallErrorCode::IdentityRevoked);
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
        let app_did = DID::from_str("did:bns:demo_web.tester").unwrap();
        // body 的 id 指向别的 DID：契约检查必须拒绝。
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
