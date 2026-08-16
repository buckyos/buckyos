//! App Installer beta 2.2 共享协议类型（真相源：doc/App 安装协议.md）。
//!
//! 本模块只定义跨进程共享的语义状态：安装来源/策略/Stage、DID 解析快照、
//! InstallPlan/readiness、VerificationReport、结构化错误、安装记录与面向
//! SDK/WebUI 的只读 snapshot。所有类型都会持久化进 TaskManager `Task.data`
//! 或 system-config `install_record`，字段演进属于协议变更。

use crate::taskdata::TaskDataProgress;
use crate::{
    AppClass, AppDocType, AppServiceSpec, MountPointConfig, PermissionItem, ServiceSettings,
    ServiceSpecConfig, TaskId, TaskOutcome, TaskPhase, TaskWaitReason, TaskWaitReasonKind,
};
use name_lib::DID;
use ndn_lib::{build_named_object_by_json, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::PathBuf;

/// Task.data、InstallPlan 与 install_record 的当前持久格式版本。
pub const APP_INSTALL_SCHEMA_VERSION: u32 = 3;

/// `apps.install_package` staging handle 的 digest 形式前缀。
/// 完整形式：`pikg:sha256:<hex>`，解析到 staging root 下的 immutable 文件。
pub const PIKG_STAGING_HANDLE_PREFIX: &str = "pikg:sha256:";

// ---------------------------------------------------------------------------
// 安装来源与策略
// ---------------------------------------------------------------------------

/// Installer 的两类逻辑入口（协议 §2.5）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstallSource {
    /// `install_app(identifier, referrer?)`：DID / 名称 / Object ID / URL / 分享对象。
    Identifier {
        identifier: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        referrer: Option<String>,
    },
    /// `install_package(staging_handle)`：本地 `.pikg`，只接受 staging handle（D5）。
    LocalPikg { staging_handle: String },
}

impl InstallSource {
    pub fn identifier(identifier: impl Into<String>, referrer: Option<String>) -> Self {
        Self::Identifier {
            identifier: identifier.into(),
            referrer,
        }
    }

    pub fn local_pikg(staging_handle: impl Into<String>) -> Self {
        Self::LocalPikg {
            staging_handle: staging_handle.into(),
        }
    }
}

/// 安装策略等级（协议 §9.6）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstallPolicy {
    StrictPublic,
    #[default]
    Normal,
    TrustedShare,
    LocalDeveloper,
    SystemInternal,
}

impl InstallPolicy {
    /// 该策略是否允许在权威源无回答（Unknown）时使用未作废的已验证 cache。
    pub fn allow_verified_cache_on_unknown(&self) -> bool {
        !matches!(self, InstallPolicy::StrictPublic)
    }

    /// 该策略是否允许显式 auto-confirm（跳过 WaitingForApproval）。
    pub fn allow_auto_confirm(&self) -> bool {
        matches!(self, InstallPolicy::SystemInternal)
    }
}

// ---------------------------------------------------------------------------
// Stage
// ---------------------------------------------------------------------------

/// 安装流水线 Stage（协议 §3.2）。Ord 顺序即流水线顺序，
/// 用于"从最早失效 Stage 重算"。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallStage {
    Resolve,
    Inspect,
    Acquire,
    Verify,
    Prepare,
    Deploy,
    Activate,
}

impl InstallStage {
    pub const ALL: [InstallStage; 7] = [
        InstallStage::Resolve,
        InstallStage::Inspect,
        InstallStage::Acquire,
        InstallStage::Verify,
        InstallStage::Prepare,
        InstallStage::Deploy,
        InstallStage::Activate,
    ];

    pub fn next(&self) -> Option<InstallStage> {
        let index = Self::ALL.iter().position(|stage| stage == self)?;
        Self::ALL.get(index + 1).copied()
    }
}

impl fmt::Display for InstallStage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            InstallStage::Resolve => "resolve",
            InstallStage::Inspect => "inspect",
            InstallStage::Acquire => "acquire",
            InstallStage::Verify => "verify",
            InstallStage::Prepare => "prepare",
            InstallStage::Deploy => "deploy",
            InstallStage::Activate => "activate",
        };
        f.write_str(value)
    }
}

// ---------------------------------------------------------------------------
// DID 解析快照
// ---------------------------------------------------------------------------

/// App Document 发布状态（协议 §2.3）。
/// `Missing` 是权威回答（从未发布），`Unknown` 是没有回答（断网/超时）；
/// 两者的错误码、重试性与 fallback 规则必须不同（实现规则 5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentStatus {
    Active,
    Missing,
    Expired,
    Revoked,
    Tombstoned,
    Migrated,
    Unknown,
}

impl DocumentStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, DocumentStatus::Revoked | DocumentStatus::Tombstoned)
    }
}

/// body 取回信道证据等级（对齐 name-client `BodyEvidence`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DidEvidenceLevel {
    Anchored,
    NeedProof,
    UnproofInfo,
}

/// 验证结果（对齐 name-client `VerificationStatus`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DidVerificationStatus {
    Passed,
    Failed,
    Unavailable,
    NotAttempted,
}

/// cache 命中语义（对齐 name-client `CacheStatus`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DidCacheStatus {
    Disabled,
    Miss,
    Hit,
    Refresh,
    Fallback,
    UnauthenticatedInfoHit,
    ZoneHit,
    ObservedFallback,
}

/// `(App DID, "app")` 解析证据快照。写入 plan / task data / install_record / proof，
/// 供升级检查、审计与 UI 风险提示复用（协议 §9.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DidResolutionSnapshot {
    pub app_did: DID,
    pub doc_type: AppDocType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_doc_object_id: Option<ObjId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolver_id: Option<String>,
    pub document_status: DocumentStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_seq: Option<u64>,
    /// 权威源 owner 绑定（resolver 输出）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_owner: Option<DID>,
    /// 验证使用的 expected_owner：权威绑定优先，否则名字结构确定性默认值；
    /// 绝不来自 candidate 自声明（实现规则 4）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_owner: Option<DID>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<DidEvidenceLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_status: Option<DidVerificationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_status: Option<DidCacheStatus>,
    /// 权威锚点 doc hash（`sha256:<hex>`，对权威信道 body 字节计算）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_hash: Option<String>,
    /// resolver warning（含 `LocalAuthorityOverride` 及其 scope 描述）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_target: Option<DID>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<u64>,
}

impl DidResolutionSnapshot {
    pub fn is_local_authority_override(&self) -> bool {
        self.warnings
            .iter()
            .any(|warning| warning.contains("LocalAuthorityOverride"))
    }

    /// 快照是否构成当前策略下可接受的 Trust Ready 证据。
    pub fn is_trust_ready(&self, policy: InstallPolicy) -> bool {
        if self.document_status.is_terminal() {
            return false;
        }
        match self.document_status {
            DocumentStatus::Active => {
                // ObservedFallback（宽松观察候选）公开安装默认拒绝。
                if matches!(self.cache_status, Some(DidCacheStatus::ObservedFallback)) {
                    return matches!(
                        policy,
                        InstallPolicy::LocalDeveloper | InstallPolicy::TrustedShare
                    ) && matches!(
                        self.verification_status,
                        Some(DidVerificationStatus::Passed)
                    );
                }
                match self.evidence {
                    Some(DidEvidenceLevel::Anchored) => true,
                    Some(DidEvidenceLevel::NeedProof) => matches!(
                        self.verification_status,
                        Some(DidVerificationStatus::Passed)
                    ),
                    _ => false,
                }
            }
            // Missing/Expired：只有策略明确允许且完成 owner 验证的候选才可降级使用，
            // 本轮实现不开放（LOCAL_DEVELOPER 也必须走 resolver 证据，D4）。
            DocumentStatus::Missing | DocumentStatus::Expired => false,
            DocumentStatus::Migrated => false,
            DocumentStatus::Unknown => false,
            DocumentStatus::Revoked | DocumentStatus::Tombstoned => false,
        }
    }
}

// ---------------------------------------------------------------------------
// InstallTarget 与 InstallPlan
// ---------------------------------------------------------------------------

/// 安装目标（协议 §11.4）。os/arch 必须来自目标 Node 信息，
/// 禁止用 Control Panel 编译期 `cfg!(target_*)` 代替（实现规则 / P2.3）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_did: Option<DID>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_id: Option<String>,
    pub os: String,
    pub arch: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_version: Option<String>,
    /// 目标 Node 可用于安装决策的数值能力快照。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub capabilities: BTreeMap<String, i64>,
}

/// 用户可确认的安装参数。最终运行配置由 AppDoc.service_config_tips 与本结构
/// 确定性合成，调用方不能直接提交 ServiceSpecConfig。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallParams {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_components: Vec<String>,
    /// 本次安装实际批准的权限条目；必须是 AppDoc.permissions 的子集，且条目
    /// 内容必须与 AppDoc 声明完全一致。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermissionItem>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub data_mount_points: HashMap<PathBuf, MountPointConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub local_cache_mount_points: HashMap<PathBuf, MountPointConfig>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub external_mount_points: HashMap<PathBuf, MountPointConfig>,
    #[serde(default)]
    pub service_settings: ServiceSettings,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub bash_envs: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub res_pool_id: Option<String>,
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
}

fn default_auto_start() -> bool {
    true
}

impl Default for InstallParams {
    fn default() -> Self {
        Self {
            selected_components: Vec::new(),
            permissions: Vec::new(),
            data_mount_points: HashMap::new(),
            local_cache_mount_points: HashMap::new(),
            external_mount_points: HashMap::new(),
            service_settings: ServiceSettings::default(),
            bash_envs: HashMap::new(),
            res_pool_id: None,
            auto_start: true,
        }
    }
}

/// 单维就绪状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadinessState {
    Ready,
    NotReady,
    Unknown,
}

impl ReadinessState {
    pub fn from_bool(ready: bool) -> Self {
        if ready {
            ReadinessState::Ready
        } else {
            ReadinessState::NotReady
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, ReadinessState::Ready)
    }
}

/// 综合安装就绪结论（协议 §4.6，序列化即协议状态字符串）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstallReadiness {
    OfflineReady,
    ContentDownloadRequired,
    TrustResolutionRequired,
    IdentityRevoked,
    UnsupportedTarget,
    InvalidPackage,
    ConfigBlocked,
}

impl InstallReadiness {
    pub fn is_ready(&self) -> bool {
        matches!(self, InstallReadiness::OfflineReady)
    }
}

/// 七维 readiness（协议 §4.5）。"包内内容齐全"不等价于"可安全离线安装"。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanReadiness {
    pub document_syntax: ReadinessState,
    pub trust: ReadinessState,
    pub package_integrity: ReadinessState,
    pub content: ReadinessState,
    pub target: ReadinessState,
    pub config: ReadinessState,
    pub install: InstallReadiness,
}

impl PlanReadiness {
    /// 按协议 §4.6 优先级合成 install readiness。
    pub fn compose(
        document_syntax: ReadinessState,
        trust: ReadinessState,
        package_integrity: ReadinessState,
        content: ReadinessState,
        target: ReadinessState,
        config: ReadinessState,
        document_status: DocumentStatus,
    ) -> Self {
        let install = if document_status.is_terminal() {
            InstallReadiness::IdentityRevoked
        } else if !target.is_ready() {
            InstallReadiness::UnsupportedTarget
        } else if matches!(document_syntax, ReadinessState::NotReady)
            || matches!(package_integrity, ReadinessState::NotReady)
        {
            InstallReadiness::InvalidPackage
        } else if matches!(document_syntax, ReadinessState::Unknown) && !trust.is_ready() {
            // 文档本身不可得（Unknown/Missing 无 body）时，内容清单无从谈起，
            // 信任解析先于内容下载。
            InstallReadiness::TrustResolutionRequired
        } else if !content.is_ready() {
            InstallReadiness::ContentDownloadRequired
        } else if !trust.is_ready() {
            InstallReadiness::TrustResolutionRequired
        } else if !config.is_ready() {
            InstallReadiness::ConfigBlocked
        } else {
            InstallReadiness::OfflineReady
        };

        Self {
            document_syntax,
            trust,
            package_integrity,
            content,
            target,
            config,
            install,
        }
    }
}

/// 内容当前位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentLocation {
    Installed,
    NamedStore,
    Pikg,
    Missing,
}

/// InstallPlan 选中的 package。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectedPackage {
    /// pkg_list key，即 sub_pkg_name。
    pub sub_pkg_name: String,
    pub pkg_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_meta_id: Option<ObjId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image_digest: Option<String>,
    pub required: bool,
}

/// plan 内单个必需内容的位置与来源。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedContent {
    /// 内容身份：ObjId 字符串或 `sha256:<hex>` digest。
    pub content_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_pkg_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_meta_id: Option<ObjId>,
    /// 该 package 物化后必须得到的 Docker image digest；仅 Docker package 存在。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_docker_image_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub location: ContentLocation,
    /// 候选获取 Source（URL 等）；offline 模式不得使用。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

/// Inspect Stage 的输出（协议 §11.4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub schema_version: u32,
    pub app: AppDocumentRef,
    pub resolution: DidResolutionSnapshot,
    pub target: InstallTarget,
    pub selected_packages: Vec<SelectedPackage>,
    pub required_contents: Vec<PlannedContent>,
    pub readiness: PlanReadiness,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub config_issues: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_options: Vec<PermissionItem>,
    pub install_params: InstallParams,
    /// Inspect 阶段确定并由用户批准的最终运行配置；Prepare 必须原样使用。
    pub service_spec_config: ServiceSpecConfig,
    #[serde(default)]
    pub estimated_download_bytes: u64,
    /// 绑定 schema、App Document、resolver 信任状态、target、强类型参数、
    /// 最终运行配置与 selected packages；任一变化必须重新 Inspect。
    pub plan_fingerprint: String,
    pub created_at: u64,
}

/// InstallPlan 对 App Document 的不可变引用。文档 body 仍由事务状态单独保存，
/// Plan 不复制 AppDoc 的展示与发布字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppDocumentRef {
    pub did: DID,
    pub object_id: ObjId,
    pub name: String,
    /// App Document.version，仅表示应用语义版本。
    pub version: String,
}

impl InstallPlan {
    /// 计算 plan fingerprint（JCS + sha256，复用 named-object 规范化路径）。
    pub fn compute_fingerprint(
        app: &AppDocumentRef,
        resolution: &DidResolutionSnapshot,
        target: &InstallTarget,
        install_params: &InstallParams,
        service_spec_config: &ServiceSpecConfig,
        selected_packages: &[SelectedPackage],
    ) -> String {
        let material = json!({
            "schema_version": APP_INSTALL_SCHEMA_VERSION,
            "app": app,
            "resolution": {
                "app_did": resolution.app_did,
                "doc_type": resolution.doc_type,
                "app_doc_object_id": resolution.app_doc_object_id,
                "resolver_id": resolution.resolver_id,
                "document_status": resolution.document_status,
                "document_version": resolution.document_version,
                "authority_seq": resolution.authority_seq,
                "effective_owner": resolution.effective_owner,
                "expected_owner": resolution.expected_owner,
                "evidence": resolution.evidence,
                "verification_status": resolution.verification_status,
                "cache_status": resolution.cache_status,
                "doc_hash": resolution.doc_hash,
                "warnings": resolution.warnings,
                "migration_target": resolution.migration_target,
            },
            "target": target,
            "install_params": install_params,
            "service_spec_config": service_spec_config,
            "selected_packages": selected_packages,
        });
        let (obj_id, _) = build_named_object_by_json("planfp", &material);
        obj_id.to_string()
    }

    pub fn matches_fingerprint(&self, fingerprint: &str) -> bool {
        self.plan_fingerprint == fingerprint
    }
}

// ---------------------------------------------------------------------------
// VerificationReport
// ---------------------------------------------------------------------------

/// 单项验证结果；失败必须能指出 subject/check/detail（实现规则 11）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationCheck {
    /// 被检对象：ObjId、包内 path、"appdoc"、"target" 等。
    pub subject: String,
    /// 检查类别：digest / obj_id / doc_identity / selector / permission /
    /// mount_safety / size / structure ...
    pub check: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl VerificationCheck {
    pub fn pass(subject: impl Into<String>, check: impl Into<String>) -> Self {
        Self {
            subject: subject.into(),
            check: check.into(),
            passed: true,
            detail: None,
        }
    }

    pub fn fail(
        subject: impl Into<String>,
        check: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            subject: subject.into(),
            check: check.into(),
            passed: false,
            detail: Some(detail.into()),
        }
    }
}

/// Verify Stage 的逐项输出，不允许只有一个 bool。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct VerificationReport {
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<VerificationCheck>,
    #[serde(default)]
    pub verified_at: u64,
}

impl VerificationReport {
    pub fn from_checks(checks: Vec<VerificationCheck>, verified_at: u64) -> Self {
        let passed = !checks.is_empty() && checks.iter().all(|check| check.passed);
        Self {
            passed,
            checks,
            verified_at,
        }
    }

    pub fn failed_checks(&self) -> impl Iterator<Item = &VerificationCheck> {
        self.checks.iter().filter(|check| !check.passed)
    }
}

// ---------------------------------------------------------------------------
// 结构化错误
// ---------------------------------------------------------------------------

/// 安装错误码。`Missing/Expired/Unknown` 有独立错误码，
/// 不得序列化成同一个状态（实现规则 5）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum InstallErrorCode {
    InvalidRequest,
    ContentDownloadRequired,
    /// 权威源没有回答（Unknown）且无可接受 cache。
    TrustResolutionRequired,
    /// 权威源明确 Revoked/Tombstoned，不可重试。
    IdentityRevoked,
    /// 权威源明确从未发布（Missing）。
    IdentityNotPublished,
    /// 权威源明确已过期（Expired）。
    IdentityExpired,
    UnsupportedTarget,
    InvalidPackage,
    ConfigBlocked,
    AcquisitionFailed,
    VerificationFailed,
    PrepareFailed,
    DeployFailed,
    ActivationFailed,
    /// 同一 user + App DID 已有进行中的事务，或重复安装冲突。
    Conflict,
    Canceled,
    Internal,
}

/// 用户可执行的修复动作（结构化，供 UI 渲染）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallUserAction {
    Retry,
    Confirm,
    ConnectNetwork,
    ResolveTrust,
    ChangeTarget,
    Cancel,
    None,
}

/// 结构化安装错误（实现规则 11：禁止用自由文本承载协议状态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallError {
    pub stage: InstallStage,
    pub code: InstallErrorCode,
    pub retryable: bool,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<InstallUserAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

impl InstallError {
    pub fn new(
        stage: InstallStage,
        code: InstallErrorCode,
        retryable: bool,
        message: impl Into<String>,
    ) -> Self {
        Self {
            stage,
            code,
            retryable,
            message: message.into(),
            action: None,
            details: None,
        }
    }

    pub fn with_action(mut self, action: InstallUserAction) -> Self {
        self.action = Some(action);
        self
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    /// 从解析状态构造对应错误（Missing/Expired/Unknown 独立错误码与动作）。
    pub fn from_document_status(
        stage: InstallStage,
        status: DocumentStatus,
        app_did: &DID,
    ) -> Self {
        match status {
            DocumentStatus::Revoked | DocumentStatus::Tombstoned => Self::new(
                stage,
                InstallErrorCode::IdentityRevoked,
                false,
                format!(
                    "App `{}` has been revoked/tombstoned by its authority",
                    app_did.to_string()
                ),
            )
            .with_action(InstallUserAction::None),
            DocumentStatus::Missing => Self::new(
                stage,
                InstallErrorCode::IdentityNotPublished,
                false,
                format!(
                    "App `{}` has never been published (authoritative Missing)",
                    app_did.to_string()
                ),
            )
            .with_action(InstallUserAction::None),
            DocumentStatus::Expired => Self::new(
                stage,
                InstallErrorCode::IdentityExpired,
                false,
                format!("App `{}` publication has expired", app_did.to_string()),
            )
            .with_action(InstallUserAction::ResolveTrust),
            DocumentStatus::Unknown => Self::new(
                stage,
                InstallErrorCode::TrustResolutionRequired,
                true,
                format!(
                    "No authoritative answer for `{}`; trust resolution required",
                    app_did.to_string()
                ),
            )
            .with_action(InstallUserAction::ConnectNetwork),
            DocumentStatus::Active | DocumentStatus::Migrated => Self::new(
                stage,
                InstallErrorCode::Internal,
                true,
                format!(
                    "Unexpected document status `{:?}` treated as error for `{}`",
                    status,
                    app_did.to_string()
                ),
            ),
        }
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{}/{:?}] {} (retryable={})",
            self.stage, self.code, self.message, self.retryable
        )
    }
}

// ---------------------------------------------------------------------------
// 事务中间态（进 Task.data）
// ---------------------------------------------------------------------------

/// candidate body / 包位置句柄（Resolve/Acquisition 的中间产物）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateHandle {
    /// candidate 种类：pikg / named_object / url / repo_record。
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
    /// staging root 下 immutable 文件路径（服务端内部，不对外暴露）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_doc_object_id: Option<ObjId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
}

/// 用户确认（confirm）内容；plan fingerprint 不符时确认无效，必须重新 Inspect。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallApproval {
    pub plan_fingerprint: String,
    pub target: InstallTarget,
    pub install_params: InstallParams,
    pub approved_by: String,
    pub approved_at: u64,
    #[serde(default)]
    pub auto_confirmed: bool,
}

/// Prepare Stage 输出：写 spec 前保存的全部部署与回滚材料。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreparedDeployment {
    pub spec_path: String,
    pub service_spec_id: String,
    pub app_index: u16,
    /// 升级场景保留旧 spec 供回滚；全新安装为 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_spec: Option<AppServiceSpec>,
    pub new_spec: AppServiceSpec,
    /// 为部署 materialize 进 NamedStore 的对象（审计/清理用）。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materialized_objects: Vec<String>,
    pub prepared_at: u64,
}

/// 事务最终结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct InstallTaskResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_record_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<u64>,
}

/// 可恢复安装事务的持久状态（进 Task.data；P0.3）。
/// 每个 Stage 成功后必须先完整写入本结构，再进入下一 Stage；
/// 重启恢复只相信这里的持久数据。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct InstallTransactionState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<InstallStage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_stages: Vec<InstallStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<CandidateHandle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolution: Option<DidResolutionSnapshot>,
    /// Resolve/绑定后用于 Inspect 的 App Document body（canonical JSON value）。
    /// 恢复只相信持久数据，因此 body 必须随事务落盘。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_app_doc: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<InstallPlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<InstallApproval>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prepared: Option<PreparedDeployment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<InstallError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<InstallTaskResult>,
    /// 用户期望的安装目标（初始 options / confirm 修改写入）。
    /// 不随 invalidate_from 清除：它是重算 plan 的输入而不是输出。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_target: Option<InstallTarget>,
    /// 用户期望的安装参数（同上）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_params: Option<InstallParams>,
    /// stage 状态修订号：每次持久化 +1，runner 幂等去重使用。
    #[serde(default)]
    pub stage_revision: u64,
}

impl InstallTransactionState {
    pub fn is_stage_completed(&self, stage: InstallStage) -> bool {
        self.completed_stages.contains(&stage)
    }

    pub fn mark_stage_completed(&mut self, stage: InstallStage) {
        if !self.is_stage_completed(stage) {
            self.completed_stages.push(stage);
        }
        self.stage = stage.next();
        self.stage_revision += 1;
    }

    /// 从最早失效 Stage 重算：作废该 Stage 及其后所有输出。
    pub fn invalidate_from(&mut self, stage: InstallStage) {
        self.completed_stages.retain(|done| *done < stage);
        if stage <= InstallStage::Inspect {
            self.plan = None;
            self.approval = None;
        }
        if stage <= InstallStage::Resolve {
            self.resolution = None;
            self.resolved_app_doc = None;
        }
        if stage <= InstallStage::Verify {
            self.verification = None;
        }
        if stage <= InstallStage::Prepare {
            self.prepared = None;
        }
        self.stage = Some(stage);
        self.stage_revision += 1;
    }

    /// 顶层可选状态 key（与字段一一对应）。
    /// 生成全量 patch 时缺席字段写 null：TaskManager 的 deep merge 用
    /// null 删除 key，否则被清空的字段（如 retry 前的 last_error、失效的
    /// plan/approval）会在 Task.data 里残留。
    pub const STATE_KEYS: [&'static str; 14] = [
        "stage",
        "completed_stages",
        "candidate",
        "resolution",
        "resolved_app_doc",
        "plan",
        "approval",
        "verification",
        "prepared",
        "last_error",
        "result",
        "requested_target",
        "requested_params",
        "stage_revision",
    ];

    /// 序列化为"全量镜像 patch"：所有状态 key 都出现，缺席的写 null。
    /// deep merge 之后 Task.data 与本结构严格一致。
    pub fn to_full_patch(&self) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or_else(|_| json!({}));
        if let Some(map) = value.as_object_mut() {
            for key in Self::STATE_KEYS {
                map.entry(key.to_string()).or_insert(Value::Null);
            }
        }
        value
    }
}

// ---------------------------------------------------------------------------
// 安装记录（长期真相，D3）
// ---------------------------------------------------------------------------

/// install_record 状态。`DeployedButActivationFailed` 与 `Installed` 必须区分。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallRecordState {
    Prepared,
    Deploying,
    Installed,
    DeployedButActivationFailed,
    RolledBack,
    Failed,
}

/// 长期安装记录，存 `users/{uid}/apps|agents/{app_name}/install_record`（D3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallRecord {
    pub schema_version: u32,
    pub app: AppDocumentRef,
    pub user_id: String,
    pub app_instance_id: String,
    pub app_class: AppClass,
    pub resolution: DidResolutionSnapshot,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_meta_ids: Vec<ObjId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
    pub target: InstallTarget,
    pub state: InstallRecordState,
    pub task_id: TaskId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proof_id: Option<String>,
    pub plan_fingerprint: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<InstallError>,
}

/// install_record 的 system-config key（app 与 agent 分树）。
pub fn install_record_key(
    app_class: AppClass,
    user_id: &str,
    app_name: &str,
    is_agent: bool,
) -> String {
    if is_agent {
        format!("users/{}/agents/{}/install_record", user_id, app_name)
    } else if app_class == AppClass::ZoneInstalled {
        format!("zone/apps/{}/install_record", app_name)
    } else {
        format!("users/{}/apps/{}/install_record", user_id, app_name)
    }
}

// ---------------------------------------------------------------------------
// 面向 SDK/WebUI 的只读 snapshot
// ---------------------------------------------------------------------------

/// 验证摘要（snapshot 用，不携带全部 checks）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct VerificationSummary {
    pub passed: bool,
    pub total_checks: u32,
    pub failed_checks: u32,
}

/// 等待确认时暴露给 UI 的内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub plan_fingerprint: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permission_options: Vec<PermissionItem>,
    pub target: InstallTarget,
    pub install_params: InstallParams,
    pub service_spec_config: ServiceSpecConfig,
    pub estimated_download_bytes: u64,
    pub readiness: PlanReadiness,
}

/// 面向 SDK/WebUI 的只读 typed snapshot（P0.2）。
/// 消费方不得解析 TaskManager 自由文本 message 或依赖内部 Task.data 布局。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppInstallStatusSnapshot {
    pub schema_version: u32,
    pub task_id: TaskId,
    pub task_phase: TaskPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_outcome: Option<TaskOutcome>,
    pub user_id: String,
    pub source: InstallSource,
    pub policy: InstallPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_did: Option<DID>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stage: Option<InstallStage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub completed_stages: Vec<InstallStage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readiness: Option<PlanReadiness>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<VerificationSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskDataProgress>,
    /// 非 None 表示任务正等待确认。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_request: Option<ApprovalRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<InstallError>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_actions: Vec<InstallUserAction>,
    pub updated_at: u64,
}

// ---------------------------------------------------------------------------
// 升级可用性
// ---------------------------------------------------------------------------

/// 升级检查状态。以权威 Resolve 的 App Document Object ID 与 document_version
/// 为准，不以 Repo 中更高 semver 为准。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AppUpdateState {
    UpToDate,
    UpdateAvailable,
    IncompatibleTarget,
    PermissionReconfirmRequired,
    TrustResolutionRequired,
    IdentityRevoked,
    Unknown,
}

/// 升级可用性检查结果（P0.2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppUpdateAvailability {
    pub app_did: DID,
    pub state: AppUpdateState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_app_doc_id: Option<ObjId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_app_doc_id: Option<ObjId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_document_version: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_document_version: Option<u64>,
    /// 语义版本（展示用，不作为升级判定依据）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did_resolution: Option<DidResolutionSnapshot>,
    /// 新版本相对已安装版本新增的权限请求。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions_added: Vec<PermissionItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_compatible: Option<bool>,
    pub checked_at: u64,
}

// ---------------------------------------------------------------------------
// staging handle
// ---------------------------------------------------------------------------

/// 解析 `apps.install_package` 的 staging handle。
/// 允许两种形式（D5）：
/// - `pikg:sha256:<hex>`：staging root 下按 digest 命名的 immutable 文件；
/// - NamedStore ObjId 字符串（chunk/fileobj，经 NDM 上传通道进入本机）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StagingHandle {
    PikgDigest(String),
    NamedObject(ObjId),
}

impl StagingHandle {
    pub fn parse(raw: &str) -> std::result::Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err("staging handle is empty".to_string());
        }
        if raw.contains('/') || raw.contains('\\') || raw.contains("..") {
            return Err("staging handle must not contain path segments".to_string());
        }
        if let Some(hex) = raw.strip_prefix(PIKG_STAGING_HANDLE_PREFIX) {
            if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err("staging handle digest must be 64 hex chars".to_string());
            }
            return Ok(StagingHandle::PikgDigest(hex.to_ascii_lowercase()));
        }
        let obj_id = ObjId::new(raw)
            .map_err(|err| format!("staging handle is not a valid obj id: {err}"))?;
        Ok(StagingHandle::NamedObject(obj_id))
    }
}

impl fmt::Display for StagingHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StagingHandle::PikgDigest(hex) => {
                write!(f, "{}{}", PIKG_STAGING_HANDLE_PREFIX, hex)
            }
            StagingHandle::NamedObject(obj_id) => write!(f, "{}", obj_id),
        }
    }
}

// ---------------------------------------------------------------------------
// misc helpers
// ---------------------------------------------------------------------------

/// snapshot 构建辅助：由事务状态与任务元数据合成只读 snapshot。
pub fn build_install_status_snapshot(
    task_id: TaskId,
    task_phase: TaskPhase,
    task_outcome: Option<TaskOutcome>,
    wait_reason: Option<&TaskWaitReason>,
    user_id: &str,
    source: &InstallSource,
    policy: InstallPolicy,
    state: &InstallTransactionState,
    progress: Option<TaskDataProgress>,
    updated_at: u64,
) -> AppInstallStatusSnapshot {
    let plan = state.plan.as_ref();
    let waiting_for_approval = task_phase == TaskPhase::Waiting
        && wait_reason
            .map(|reason| reason.kind == TaskWaitReasonKind::Authorization)
            .unwrap_or(false);
    let approval_request = if waiting_for_approval {
        plan.map(|plan| ApprovalRequest {
            plan_fingerprint: plan.plan_fingerprint.clone(),
            permission_options: plan.permission_options.clone(),
            target: plan.target.clone(),
            install_params: plan.install_params.clone(),
            service_spec_config: plan.service_spec_config.clone(),
            estimated_download_bytes: plan.estimated_download_bytes,
            readiness: plan.readiness.clone(),
        })
    } else {
        None
    };

    let mut warnings: Vec<String> = Vec::new();
    if let Some(resolution) = state.resolution.as_ref() {
        warnings.extend(resolution.warnings.iter().cloned());
    }

    let mut available_actions = Vec::new();
    if approval_request.is_some() {
        available_actions.push(InstallUserAction::Confirm);
    }
    if let Some(error) = state.last_error.as_ref() {
        if error.retryable {
            available_actions.push(InstallUserAction::Retry);
        }
        if let Some(action) = error.action {
            if action != InstallUserAction::None && !available_actions.contains(&action) {
                available_actions.push(action);
            }
        }
    }
    if !task_phase.is_terminal() {
        available_actions.push(InstallUserAction::Cancel);
    }

    AppInstallStatusSnapshot {
        schema_version: APP_INSTALL_SCHEMA_VERSION,
        task_id,
        task_phase,
        task_outcome,
        user_id: user_id.to_string(),
        source: source.clone(),
        policy,
        app_did: plan
            .map(|plan| plan.app.did.clone())
            .or_else(|| state.resolution.as_ref().map(|r| r.app_did.clone())),
        app_name: plan.map(|plan| plan.app.name.clone()),
        app_version: plan.map(|plan| plan.app.version.clone()),
        stage: state.stage,
        completed_stages: state.completed_stages.clone(),
        readiness: plan.map(|plan| plan.readiness.clone()),
        verification: state
            .verification
            .as_ref()
            .map(|report| VerificationSummary {
                passed: report.passed,
                total_checks: report.checks.len() as u32,
                failed_checks: report.failed_checks().count() as u32,
            }),
        progress,
        approval_request,
        warnings,
        error: state.last_error.clone(),
        available_actions,
        updated_at,
    }
}

/// 便捷类型：install options 的宽松容器（RPC 层收 JSON）。
pub type InstallOptions = BTreeMap<String, Value>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_target() -> InstallTarget {
        InstallTarget {
            node_did: None,
            node_id: Some("ood1".to_string()),
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            kernel_version: None,
            runtime_version: None,
            capabilities: BTreeMap::new(),
        }
    }

    #[test]
    fn install_readiness_serializes_to_protocol_strings() {
        assert_eq!(
            serde_json::to_string(&InstallReadiness::OfflineReady).unwrap(),
            "\"OFFLINE_READY\""
        );
        assert_eq!(
            serde_json::to_string(&InstallReadiness::TrustResolutionRequired).unwrap(),
            "\"TRUST_RESOLUTION_REQUIRED\""
        );
        assert_eq!(
            serde_json::to_string(&InstallErrorCode::IdentityRevoked).unwrap(),
            "\"IDENTITY_REVOKED\""
        );
        assert_eq!(
            serde_json::to_string(&InstallPolicy::LocalDeveloper).unwrap(),
            "\"LOCAL_DEVELOPER\""
        );
    }

    #[test]
    fn install_params_reject_unknown_fields_and_default_auto_start() {
        let params: InstallParams = serde_json::from_value(json!({})).unwrap();
        assert!(params.auto_start);
        assert!(params.permissions.is_empty());
        let params: InstallParams = serde_json::from_value(json!({
            "permissions": [{
                "scope_path": "user/home",
                "required": true,
                "actions": ["read"],
                "exp": null
            }]
        }))
        .unwrap();
        assert_eq!(params.permissions[0].scope_path, "user/home");
        assert!(serde_json::from_value::<InstallParams>(json!({
            "sub_hostname": "legacy"
        }))
        .is_err());
    }

    #[test]
    fn missing_expired_unknown_have_distinct_codes() {
        let did = DID::new("bns", "demo.tester");
        let missing = InstallError::from_document_status(
            InstallStage::Resolve,
            DocumentStatus::Missing,
            &did,
        );
        let expired = InstallError::from_document_status(
            InstallStage::Resolve,
            DocumentStatus::Expired,
            &did,
        );
        let unknown = InstallError::from_document_status(
            InstallStage::Resolve,
            DocumentStatus::Unknown,
            &did,
        );
        let revoked = InstallError::from_document_status(
            InstallStage::Resolve,
            DocumentStatus::Revoked,
            &did,
        );

        assert_eq!(missing.code, InstallErrorCode::IdentityNotPublished);
        assert_eq!(expired.code, InstallErrorCode::IdentityExpired);
        assert_eq!(unknown.code, InstallErrorCode::TrustResolutionRequired);
        assert_eq!(revoked.code, InstallErrorCode::IdentityRevoked);
        assert!(!missing.retryable);
        assert!(unknown.retryable);
        assert!(!revoked.retryable);

        let codes = [missing.code, expired.code, unknown.code, revoked.code];
        let unique: std::collections::BTreeSet<String> = codes
            .iter()
            .map(|code| serde_json::to_string(code).unwrap())
            .collect();
        assert_eq!(
            unique.len(),
            4,
            "Missing/Expired/Unknown/Revoked 不得共享错误码"
        );
    }

    #[test]
    fn readiness_compose_priority_matches_protocol() {
        // Trust Ready + Content Missing => CONTENT_DOWNLOAD_REQUIRED
        let readiness = PlanReadiness::compose(
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::NotReady,
            ReadinessState::Ready,
            ReadinessState::Ready,
            DocumentStatus::Active,
        );
        assert_eq!(readiness.install, InstallReadiness::ContentDownloadRequired);

        // Content Ready + Trust Missing => TRUST_RESOLUTION_REQUIRED
        let readiness = PlanReadiness::compose(
            ReadinessState::Ready,
            ReadinessState::NotReady,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            DocumentStatus::Unknown,
        );
        assert_eq!(readiness.install, InstallReadiness::TrustResolutionRequired);

        // Revoked 压倒一切。
        let readiness = PlanReadiness::compose(
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            DocumentStatus::Revoked,
        );
        assert_eq!(readiness.install, InstallReadiness::IdentityRevoked);

        // 全 ready => OFFLINE_READY
        let readiness = PlanReadiness::compose(
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            DocumentStatus::Active,
        );
        assert_eq!(readiness.install, InstallReadiness::OfflineReady);

        // 目标不支持。
        let readiness = PlanReadiness::compose(
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::Ready,
            ReadinessState::NotReady,
            ReadinessState::Ready,
            DocumentStatus::Active,
        );
        assert_eq!(readiness.install, InstallReadiness::UnsupportedTarget);
    }

    #[test]
    fn plan_fingerprint_is_stable_and_sensitive() {
        let obj_id = ObjId::new_by_raw("appdoc".to_string(), vec![1u8; 32]);
        let target = sample_target();
        let app = AppDocumentRef {
            did: DID::new("bns", "demo.tester"),
            object_id: obj_id.clone(),
            name: "demo".to_string(),
            version: "1.0.0".to_string(),
        };
        let mut resolution = DidResolutionSnapshot {
            app_did: app.did.clone(),
            doc_type: AppDocType,
            app_doc_object_id: Some(obj_id),
            resolver_id: Some("test".to_string()),
            document_status: DocumentStatus::Active,
            document_version: Some(3),
            authority_seq: Some(3),
            effective_owner: Some(DID::new("bns", "tester")),
            expected_owner: Some(DID::new("bns", "tester")),
            evidence: Some(DidEvidenceLevel::Anchored),
            verification_status: Some(DidVerificationStatus::Passed),
            cache_status: Some(DidCacheStatus::ZoneHit),
            doc_hash: None,
            warnings: vec![],
            migration_target: None,
            resolved_at: Some(1),
        };
        let params = InstallParams {
            selected_components: vec!["full".to_string()],
            ..Default::default()
        };
        let config = ServiceSpecConfig::default();
        let packages = vec![SelectedPackage {
            sub_pkg_name: "full".to_string(),
            pkg_id: "demo#1.0.0".to_string(),
            package_meta_id: Some(ObjId::new_by_raw("pkg".to_string(), vec![2u8; 32])),
            docker_image_name: None,
            docker_image_digest: None,
            required: true,
        }];

        let fp1 = InstallPlan::compute_fingerprint(
            &app,
            &resolution,
            &target,
            &params,
            &config,
            &packages,
        );
        let fp2 = InstallPlan::compute_fingerprint(
            &app,
            &resolution,
            &target,
            &params,
            &config,
            &packages,
        );
        assert_eq!(fp1, fp2, "同输入 fingerprint 必须稳定");

        let mut permission_params = params.clone();
        permission_params.permissions.push(PermissionItem {
            scope_path: "wan".to_string(),
            required: false,
            actions: vec![],
            exp: None,
        });
        let permission_fp = InstallPlan::compute_fingerprint(
            &app,
            &resolution,
            &target,
            &permission_params,
            &config,
            &packages,
        );
        assert_ne!(fp1, permission_fp, "权限选择必须进入 fingerprint");

        // document_version 变化 => fingerprint 变化。
        resolution.document_version = Some(4);
        let fp3 = InstallPlan::compute_fingerprint(
            &app,
            &resolution,
            &target,
            &params,
            &config,
            &packages,
        );
        assert_ne!(fp1, fp3);
        resolution.document_version = Some(3);

        // target 变化 => fingerprint 变化。
        let mut other_target = sample_target();
        other_target.arch = "aarch64".to_string();
        let fp4 = InstallPlan::compute_fingerprint(
            &app,
            &resolution,
            &other_target,
            &params,
            &config,
            &packages,
        );
        assert_ne!(fp1, fp4);

        resolution.verification_status = Some(DidVerificationStatus::Failed);
        let fp5 = InstallPlan::compute_fingerprint(
            &app,
            &resolution,
            &target,
            &params,
            &config,
            &packages,
        );
        assert_ne!(fp1, fp5, "trust evidence changes must invalidate approval");
    }

    #[test]
    fn transaction_state_stage_bookkeeping() {
        let mut state = InstallTransactionState::default();
        state.stage = Some(InstallStage::Resolve);
        state.mark_stage_completed(InstallStage::Resolve);
        state.mark_stage_completed(InstallStage::Inspect);
        assert!(state.is_stage_completed(InstallStage::Resolve));
        assert_eq!(state.stage, Some(InstallStage::Acquire));
        assert_eq!(state.stage_revision, 2);

        // 从 Inspect 失效：Resolve 保留，Inspect 及之后作废。
        state.plan = None;
        state.invalidate_from(InstallStage::Inspect);
        assert!(state.is_stage_completed(InstallStage::Resolve));
        assert!(!state.is_stage_completed(InstallStage::Inspect));
        assert_eq!(state.stage, Some(InstallStage::Inspect));
    }

    #[test]
    fn staging_handle_parse_rejects_paths() {
        assert!(StagingHandle::parse("/tmp/x.pikg").is_err());
        assert!(StagingHandle::parse("../x.pikg").is_err());
        assert!(StagingHandle::parse("").is_err());
        assert!(StagingHandle::parse("pikg:sha256:zz").is_err());

        let digest_handle =
            StagingHandle::parse(&format!("pikg:sha256:{}", "ab".repeat(32))).unwrap();
        assert!(matches!(digest_handle, StagingHandle::PikgDigest(_)));

        let obj_handle = StagingHandle::parse(
            ObjId::new_by_raw("chunk".to_string(), vec![3u8; 32])
                .to_string()
                .as_str(),
        )
        .unwrap();
        assert!(matches!(obj_handle, StagingHandle::NamedObject(_)));
    }

    #[test]
    fn trust_ready_rules() {
        let did = DID::new("bns", "demo.tester");
        let mut snapshot = DidResolutionSnapshot {
            app_did: did,
            doc_type: AppDocType,
            app_doc_object_id: Some(ObjId::new_by_raw("appdoc".to_string(), vec![1u8; 32])),
            resolver_id: None,
            document_status: DocumentStatus::Active,
            document_version: Some(1),
            authority_seq: None,
            effective_owner: None,
            expected_owner: Some(DID::new("bns", "tester")),
            evidence: Some(DidEvidenceLevel::Anchored),
            verification_status: Some(DidVerificationStatus::Passed),
            cache_status: Some(DidCacheStatus::ZoneHit),
            doc_hash: None,
            warnings: vec![],
            migration_target: None,
            resolved_at: None,
        };
        assert!(snapshot.is_trust_ready(InstallPolicy::Normal));

        // NeedProof 且验证未通过 => 不 ready。
        snapshot.evidence = Some(DidEvidenceLevel::NeedProof);
        snapshot.verification_status = Some(DidVerificationStatus::NotAttempted);
        assert!(!snapshot.is_trust_ready(InstallPolicy::Normal));
        snapshot.verification_status = Some(DidVerificationStatus::Passed);
        assert!(snapshot.is_trust_ready(InstallPolicy::Normal));

        // ObservedFallback 公开模式拒绝。
        snapshot.cache_status = Some(DidCacheStatus::ObservedFallback);
        assert!(!snapshot.is_trust_ready(InstallPolicy::Normal));
        assert!(!snapshot.is_trust_ready(InstallPolicy::StrictPublic));
        assert!(snapshot.is_trust_ready(InstallPolicy::LocalDeveloper));

        // Revoked 永不 ready。
        snapshot.cache_status = Some(DidCacheStatus::ZoneHit);
        snapshot.document_status = DocumentStatus::Revoked;
        assert!(!snapshot.is_trust_ready(InstallPolicy::LocalDeveloper));
    }
}
