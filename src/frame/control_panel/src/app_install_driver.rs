//! 生产 Stage driver：把 resolver / planner / pikg 与系统设施拼装成
//! `InstallStageDriver`。
//!
//! 分层落地（对应实现计划的提交顺序）：
//! - Resolve / Inspect：本文件完成（resolver adapter + planner + pikg staging）；
//! - Acquire / Verify：接 TaskManager 下载与 NamedStore/pikg 校验；
//! - Prepare / Deploy / Activate / rollback：接 system-config/scheduler。

use crate::app_install_engine::{
    ActivateOutcome, InstallStageDriver, InstallTaskView, ResolveOutcome,
};
use crate::app_install_planner::{build_install_plan, ContentLocator, PlannerInput};
use crate::app_install_resolver::{
    bind_candidate_document, normalize_identifier, AppDidResolver, NameClientAppResolver,
    NormalizedIdentifier,
};
use crate::pikg::{PikgInspection, PikgReader};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, AppInstallTaskData, CandidateHandle, ContentLocation,
    DidResolutionSnapshot, InstallError, InstallErrorCode, InstallPlan, InstallSource,
    InstallStage, InstallTarget, PlanReadiness, PreparedDeployment, ReadinessState, StagingHandle,
    VerificationReport, OBJ_TYPE_APP_DOC,
};
use log::warn;
use name_lib::{DeviceInfo, DID};
use ndn_lib::{build_named_object_by_json, ObjId};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;

/// pikg staging root：Control Panel 数据目录下的受控子目录（D5）。
pub fn pikg_staging_root() -> PathBuf {
    buckyos_kit::get_buckyos_root_dir()
        .join("cache")
        .join("control_panel")
        .join("pikg_staging")
}

pub struct ProductionInstallDriver {
    resolver: Arc<dyn AppDidResolver>,
    staging_root: PathBuf,
}

impl ProductionInstallDriver {
    pub fn new() -> Self {
        Self {
            resolver: Arc::new(NameClientAppResolver::new()),
            staging_root: pikg_staging_root(),
        }
    }

    pub fn with_parts(resolver: Arc<dyn AppDidResolver>, staging_root: PathBuf) -> Self {
        Self {
            resolver,
            staging_root,
        }
    }

    fn invalid_request(message: impl Into<String>) -> InstallError {
        InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::InvalidRequest,
            false,
            message,
        )
    }

    /// 打开事务 candidate 对应的 staged pikg（若有）。
    async fn open_candidate_pikg(
        &self,
        data: &AppInstallTaskData,
    ) -> Result<Option<PikgReader>, InstallError> {
        let Some(candidate) = data.state.candidate.as_ref() else {
            return Ok(None);
        };
        if candidate.kind != "pikg" {
            return Ok(None);
        }
        let (Some(path), Some(digest)) = (
            candidate.staging_path.as_ref(),
            candidate.pikg_digest.as_ref(),
        ) else {
            return Ok(None);
        };
        let reader = PikgReader::open(std::path::Path::new(path), Some(digest))
            .await
            .map_err(|err| {
                InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!("reopen staged pikg failed: {err}"),
                )
            })?;
        Ok(Some(reader))
    }

    /// 解析本地 pikg 入口：staging handle -> 打开 staged 文件。
    async fn resolve_local_pikg(
        &self,
        staging_handle: &str,
        policy: buckyos_api::InstallPolicy,
    ) -> Result<ResolveOutcome, InstallError> {
        let handle = StagingHandle::parse(staging_handle).map_err(Self::invalid_request)?;
        let (digest, staged_path) = match handle {
            StagingHandle::PikgDigest(hex) => {
                let path = self.staging_root.join(format!("{hex}.pikg"));
                if !path.exists() {
                    return Err(Self::invalid_request(format!(
                        "staging handle does not resolve to a staged pikg: {staging_handle}"
                    )));
                }
                (hex, path)
            }
            StagingHandle::NamedObject(obj_id) => {
                // NDN 上传通道的 chunk 物化在 Acquire 设施就绪后支持。
                return Err(InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::AcquisitionFailed,
                    true,
                    format!(
                        "named-store staging handle `{obj_id}` materialization is not available yet"
                    ),
                ));
            }
        };

        // canonical path 必须仍在 staging root 内（D5）。
        let canonical = staged_path
            .canonicalize()
            .map_err(|err| Self::invalid_request(format!("staging path invalid: {err}")))?;
        let root_canonical = self
            .staging_root
            .canonicalize()
            .map_err(|err| Self::invalid_request(format!("staging root invalid: {err}")))?;
        if !canonical.starts_with(&root_canonical) {
            return Err(Self::invalid_request(
                "staging handle escaped staging root".to_string(),
            ));
        }

        let reader = PikgReader::open(&canonical, Some(&digest))
            .await
            .map_err(|err| {
                InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!("open staged pikg failed: {err}"),
                )
            })?;
        let inspection = reader.inspection();
        let app_did = inspection.app_doc.id.clone();
        let candidate_value = serde_json::to_value(&inspection.app_doc).map_err(|err| {
            Self::invalid_request(format!("serialize candidate app doc failed: {err}"))
        })?;

        let resolved = self.resolver.resolve_app(&app_did, policy).await?;
        // candidate 绑定：id/owner 硬约束（owner 冒充在这里拒绝）。
        let binding = bind_candidate_document(&app_did, &resolved.snapshot, &candidate_value)?;

        // 文档取舍：权威 body 优先；无权威 body 时 candidate 作为待信任候选
        // 进入 Inspect（trust 未 ready 时引擎会停在 WAITING_FOR_TRUST_RESOLUTION）。
        let resolved_app_doc = resolved
            .document_value
            .clone()
            .or(Some(candidate_value.clone()));

        Ok(ResolveOutcome {
            candidate: Some(CandidateHandle {
                kind: "pikg".to_string(),
                pikg_digest: Some(inspection.pikg_digest.clone()),
                staging_path: Some(canonical.to_string_lossy().to_string()),
                app_doc_object_id: Some(binding.candidate_obj_id),
                source_url: None,
            }),
            resolution: resolved.snapshot,
            resolved_app_doc,
        })
    }

    async fn resolve_identifier(
        &self,
        identifier: &str,
        policy: buckyos_api::InstallPolicy,
    ) -> Result<ResolveOutcome, InstallError> {
        match normalize_identifier(identifier)? {
            NormalizedIdentifier::AppDid(app_did) => {
                let resolved = self.resolver.resolve_app(&app_did, policy).await?;
                Ok(ResolveOutcome {
                    candidate: None,
                    resolution: resolved.snapshot,
                    resolved_app_doc: resolved.document_value,
                })
            }
            NormalizedIdentifier::ObjectId(obj_id) => {
                // 最小 Acquisition：本地 NamedStore 已有该对象时读 candidate。
                let candidate_value = read_named_object_value(&obj_id).await?;
                let Some(candidate_value) = candidate_value else {
                    return Err(InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::AcquisitionFailed,
                        true,
                        format!(
                            "app document object `{obj_id}` is not available locally; remote minimal acquisition is not available yet"
                        ),
                    ));
                };
                let app_did_raw = candidate_value
                    .get("id")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        Self::invalid_request("candidate app document has no id".to_string())
                    })?;
                let app_did = DID::from_str(app_did_raw).map_err(|err| {
                    Self::invalid_request(format!("candidate app document id invalid: {err}"))
                })?;
                let resolved = self.resolver.resolve_app(&app_did, policy).await?;
                let binding =
                    bind_candidate_document(&app_did, &resolved.snapshot, &candidate_value)?;
                let resolved_app_doc = resolved
                    .document_value
                    .clone()
                    .or(Some(candidate_value.clone()));
                Ok(ResolveOutcome {
                    candidate: Some(CandidateHandle {
                        kind: "named_object".to_string(),
                        pikg_digest: None,
                        staging_path: None,
                        app_doc_object_id: Some(binding.candidate_obj_id),
                        source_url: None,
                    }),
                    resolution: resolved.snapshot,
                    resolved_app_doc,
                })
            }
            NormalizedIdentifier::Url(url) => Err(InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::AcquisitionFailed,
                true,
                format!("url identifier `{url}` minimal acquisition is not available yet"),
            )),
        }
    }

    /// 目标解析：requested_target 优先；否则读默认 OOD 的 devices/{node}/info。
    /// 禁止编译期 cfg!() 代替目标 Node 信息（P2.3）。
    async fn resolve_target(
        &self,
        data: &AppInstallTaskData,
    ) -> Result<InstallTarget, InstallError> {
        if let Some(target) = data.state.requested_target.clone() {
            return Ok(target);
        }
        match default_node_target().await {
            Ok(target) => Ok(target),
            Err(err) => Err(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::ConfigBlocked,
                true,
                format!("can not determine default install target: {err}"),
            )),
        }
    }

    fn placeholder_plan(
        snapshot: &DidResolutionSnapshot,
        policy: buckyos_api::InstallPolicy,
        target: InstallTarget,
    ) -> InstallPlan {
        // body 不可得（Unknown/Missing 无候选）：构造只读占位 plan，
        // readiness 表达 TRUST_RESOLUTION_REQUIRED，引擎据此停靠。
        let readiness = PlanReadiness::compose(
            ReadinessState::Unknown,
            ReadinessState::from_bool(snapshot.is_trust_ready(policy)),
            ReadinessState::Unknown,
            ReadinessState::Unknown,
            ReadinessState::Ready,
            snapshot.document_status,
            true,
        );
        InstallPlan {
            app_did: snapshot.app_did.clone(),
            doc_type: snapshot.doc_type.clone(),
            app_doc_object_id: snapshot
                .app_doc_object_id
                .clone()
                .unwrap_or_else(|| ObjId::new_by_raw(OBJ_TYPE_APP_DOC.to_string(), vec![0u8; 32])),
            app_name: snapshot.app_did.to_string(),
            app_version: String::new(),
            did_resolution: snapshot.clone(),
            target,
            selected_packages: vec![],
            required_contents: vec![],
            readiness,
            permissions: vec![],
            install_params: json!({}),
            estimated_download_bytes: 0,
            plan_fingerprint: "unresolved".to_string(),
            created_at: buckyos_kit::buckyos_get_unix_timestamp(),
        }
    }

    fn stage_not_ready(stage: InstallStage, what: &str) -> InstallError {
        InstallError::new(
            stage,
            InstallErrorCode::Internal,
            false,
            format!("{what} driver is not wired up yet in this build"),
        )
    }
}

#[async_trait]
impl InstallStageDriver for ProductionInstallDriver {
    async fn resolve(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<ResolveOutcome, InstallError> {
        match &data.request.source {
            InstallSource::Identifier { identifier, .. } => {
                self.resolve_identifier(identifier, data.request.policy)
                    .await
            }
            InstallSource::LocalPikg { staging_handle } => {
                self.resolve_local_pikg(staging_handle, data.request.policy)
                    .await
            }
        }
    }

    async fn inspect(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallPlan, InstallError> {
        let snapshot = data.state.resolution.clone().ok_or_else(|| {
            InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::Internal,
                false,
                "inspect without resolution snapshot",
            )
        })?;
        let target = self.resolve_target(data).await?;

        let Some(doc_value) = data.state.resolved_app_doc.clone() else {
            return Ok(Self::placeholder_plan(
                &snapshot,
                data.request.policy,
                target,
            ));
        };
        let app_doc: buckyos_api::AppDoc =
            serde_json::from_value(doc_value.clone()).map_err(|err| {
                InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::VerificationFailed,
                    false,
                    format!("persisted app document invalid: {err}"),
                )
            })?;
        let (app_doc_object_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, &doc_value);

        let pikg_reader = self.open_candidate_pikg(data).await?;
        let pikg_inspection: Option<&PikgInspection> =
            pikg_reader.as_ref().map(|reader| reader.inspection());

        let install_params = data
            .state
            .requested_params
            .clone()
            .unwrap_or_else(|| json!({}));

        let locator = NamedStoreContentLocator;
        build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id,
                snapshot: &snapshot,
                policy: data.request.policy,
                target,
                install_params,
                pikg: pikg_inspection,
            },
            &locator,
        )
        .await
    }

    async fn acquire(
        &self,
        _view: &InstallTaskView,
        _data: &AppInstallTaskData,
    ) -> Result<InstallPlan, InstallError> {
        Err(Self::stage_not_ready(InstallStage::Acquire, "acquire"))
    }

    async fn verify(
        &self,
        _view: &InstallTaskView,
        _data: &AppInstallTaskData,
    ) -> Result<VerificationReport, InstallError> {
        Err(Self::stage_not_ready(InstallStage::Verify, "verify"))
    }

    async fn prepare(
        &self,
        _view: &InstallTaskView,
        _data: &AppInstallTaskData,
    ) -> Result<PreparedDeployment, InstallError> {
        Err(Self::stage_not_ready(InstallStage::Prepare, "prepare"))
    }

    async fn deploy(
        &self,
        _view: &InstallTaskView,
        _data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        Err(Self::stage_not_ready(InstallStage::Deploy, "deploy"))
    }

    async fn activate(
        &self,
        _view: &InstallTaskView,
        _data: &AppInstallTaskData,
    ) -> Result<ActivateOutcome, InstallError> {
        Err(Self::stage_not_ready(InstallStage::Activate, "activate"))
    }

    async fn rollback_deploy(
        &self,
        _view: &InstallTaskView,
        _data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        Err(Self::stage_not_ready(InstallStage::Deploy, "rollback"))
    }
}

// ---------------------------------------------------------------------------
// NamedStore content locator（Inspect 用的只读位置查询）
// ---------------------------------------------------------------------------

pub struct NamedStoreContentLocator;

#[async_trait]
impl ContentLocator for NamedStoreContentLocator {
    async fn locate(&self, content_id: &str) -> ContentLocation {
        let Ok(obj_id) = ObjId::new(content_id) else {
            return ContentLocation::Missing;
        };
        let Ok(runtime) = get_buckyos_api_runtime() else {
            return ContentLocation::Missing;
        };
        let Ok(named_store) = runtime.get_named_store().await else {
            return ContentLocation::Missing;
        };
        if obj_id.is_chunk() {
            let chunk_id = ndn_lib::ChunkId::from_obj_id(&obj_id);
            if named_store.have_chunk(&chunk_id).await {
                return ContentLocation::NamedStore;
            }
            return ContentLocation::Missing;
        }
        match named_store.is_object_exist(&obj_id).await {
            Ok(true) => ContentLocation::NamedStore,
            _ => ContentLocation::Missing,
        }
    }
}

async fn read_named_object_value(obj_id: &ObjId) -> Result<Option<Value>, InstallError> {
    let runtime = get_buckyos_api_runtime().map_err(|err| {
        InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::Internal,
            true,
            format!("get runtime failed: {err}"),
        )
    })?;
    let named_store = runtime.get_named_store().await.map_err(|err| {
        InstallError::new(
            InstallStage::Resolve,
            InstallErrorCode::Internal,
            true,
            format!("open named store failed: {err}"),
        )
    })?;
    match named_store.get_object(obj_id).await {
        Ok(body) => {
            let value: Value = serde_json::from_str(&body).map_err(|err| {
                InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::VerificationFailed,
                    false,
                    format!("named object `{obj_id}` is not json: {err}"),
                )
            })?;
            // 内容寻址复核。
            let (computed, _) = build_named_object_by_json(&obj_id.obj_type, &value);
            if computed != *obj_id {
                return Err(InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::VerificationFailed,
                    false,
                    format!("named object `{obj_id}` failed obj id recheck"),
                ));
            }
            Ok(Some(value))
        }
        Err(_) => Ok(None),
    }
}

/// 默认安装目标：zone 默认 OOD 的 devices/{node}/info（DeviceInfo.os/arch）。
async fn default_node_target() -> Result<InstallTarget, String> {
    let runtime = get_buckyos_api_runtime().map_err(|err| err.to_string())?;
    let client = runtime
        .get_system_config_client()
        .await
        .map_err(|err| err.to_string())?;

    // 默认节点：devices 下第一个有 info 的 OOD。
    let device_names = client
        .list("devices")
        .await
        .map_err(|err| format!("list devices failed: {err}"))?;
    for name in device_names {
        let key = format!("devices/{name}/info");
        let Ok(value) = client.get(&key).await else {
            continue;
        };
        let Ok(info) = serde_json::from_str::<DeviceInfo>(&value.value) else {
            warn!("devices/{name}/info is not a DeviceInfo");
            continue;
        };
        return Ok(InstallTarget {
            node_did: None,
            node_id: Some(name),
            os: buckyos_api::normalize_os(info.os.as_str()),
            arch: buckyos_api::normalize_arch(info.arch.as_str()),
            kernel_version: None,
            runtime_version: None,
        });
    }
    Err("no device info found under devices/".to_string())
}
