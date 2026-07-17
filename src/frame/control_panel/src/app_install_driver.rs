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
    pub(crate) async fn open_candidate_pikg(
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
                // NDN 上传通道进入本机的 chunk：物化到 staging root 后按
                // digest 固定（与 pikg:sha256 handle 汇合到同一 immutable 面）。
                self.materialize_ndn_pikg(&obj_id).await?
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
            NormalizedIdentifier::Url(url) => {
                // 最小 Acquisition：仅支持内嵌 ObjId 的 URL（cyfs:// 等），
                // 下载该对象后按 ObjectId 入口继续；纯 HTTPS Manifest URL
                // 属 Web-to-Native 兼容层，本轮不做。
                let Some(obj_id) = infer_obj_id_from_url(&url) else {
                    return Err(Self::invalid_request(format!(
                        "url identifier `{url}` does not embed an object id; \
                         use apps.install_package with a staging handle instead"
                    )));
                };
                if read_named_object_value(&obj_id).await?.is_none() {
                    download_object_by_id(&url, &obj_id, "system", "control-panel", None).await?;
                }
                let candidate_value = read_named_object_value(&obj_id).await?.ok_or_else(|| {
                    InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::AcquisitionFailed,
                        true,
                        format!("object `{obj_id}` still missing after download"),
                    )
                })?;
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
                        source_url: Some(url),
                    }),
                    resolution: resolved.snapshot,
                    resolved_app_doc,
                })
            }
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

    /// NDN 上传的 pikg（chunk/fileobj）物化为 staging root 下的 immutable
    /// 文件，返回 (digest, staged_path)。
    async fn materialize_ndn_pikg(
        &self,
        obj_id: &ObjId,
    ) -> Result<(String, PathBuf), InstallError> {
        use tokio::io::AsyncWriteExt;
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
        if !obj_id.is_chunk() {
            return Err(Self::invalid_request(format!(
                "staging handle `{obj_id}` is not a chunk id"
            )));
        }
        let chunk_id = ndn_lib::ChunkId::from_obj_id(obj_id);
        let (mut reader, _total) =
            named_store
                .open_chunk_reader(&chunk_id, 0)
                .await
                .map_err(|err| {
                    InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::AcquisitionFailed,
                        true,
                        format!("staging chunk `{obj_id}` is not available locally: {err}"),
                    )
                })?;

        tokio::fs::create_dir_all(&self.staging_root)
            .await
            .map_err(|err| {
                InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::Internal,
                    true,
                    format!("create staging root failed: {err}"),
                )
            })?;
        let tmp_path = self.staging_root.join(format!(
            ".ndn-{}.tmp",
            chunk_id.to_string().replace(':', "_")
        ));
        {
            let mut file = tokio::fs::File::create(&tmp_path).await.map_err(|err| {
                InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::Internal,
                    true,
                    format!("create staging tmp failed: {err}"),
                )
            })?;
            tokio::io::copy(&mut reader, &mut file)
                .await
                .map_err(|err| {
                    InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::AcquisitionFailed,
                        true,
                        format!("copy staging chunk failed: {err}"),
                    )
                })?;
            file.flush().await.ok();
        }
        let result = PikgReader::stage_pikg_file(&tmp_path, &self.staging_root)
            .await
            .map_err(|err| {
                InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!("stage ndn pikg failed: {err}"),
                )
            });
        let _ = tokio::fs::remove_file(&tmp_path).await;
        result
    }

    /// 用真实位置重建 plan（fingerprint 只绑定身份/目标/参数，不含位置，
    /// 因此刷新不会作废已绑定的 approval）。
    async fn rebuild_plan(&self, data: &AppInstallTaskData) -> Result<InstallPlan, InstallError> {
        let snapshot = data.state.resolution.clone().ok_or_else(|| {
            InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::Internal,
                false,
                "acquire without resolution snapshot",
            )
        })?;
        let doc_value = data.state.resolved_app_doc.clone().ok_or_else(|| {
            InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::Internal,
                false,
                "acquire without resolved app document",
            )
        })?;
        let app_doc: buckyos_api::AppDoc =
            serde_json::from_value(doc_value.clone()).map_err(|err| {
                InstallError::new(
                    InstallStage::Acquire,
                    InstallErrorCode::VerificationFailed,
                    false,
                    format!("persisted app document invalid: {err}"),
                )
            })?;
        let (app_doc_object_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, &doc_value);
        let target = data
            .state
            .plan
            .as_ref()
            .map(|plan| plan.target.clone())
            .ok_or_else(|| {
                InstallError::new(
                    InstallStage::Acquire,
                    InstallErrorCode::Internal,
                    false,
                    "acquire without plan",
                )
            })?;
        let install_params = data
            .state
            .plan
            .as_ref()
            .map(|plan| plan.install_params.clone())
            .unwrap_or_else(|| json!({}));
        let pikg_reader = self.open_candidate_pikg(data).await?;
        let pikg_inspection: Option<&PikgInspection> =
            pikg_reader.as_ref().map(|reader| reader.inspection());
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
            &NamedStoreContentLocator,
        )
        .await
    }

    fn offline_requested(data: &AppInstallTaskData) -> bool {
        data.request
            .options
            .as_ref()
            .and_then(|options| options.get("offline"))
            .and_then(|value| value.as_bool())
            .unwrap_or(false)
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
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallPlan, InstallError> {
        let offline = Self::offline_requested(data);
        // 最多几轮：meta 下载后 payload 清单才展开，需要再补一轮。
        let mut plan = self.rebuild_plan(data).await?;
        for round in 0..4 {
            let missing: Vec<_> = plan
                .required_contents
                .iter()
                .filter(|content| matches!(content.location, ContentLocation::Missing))
                .cloned()
                .collect();
            if missing.is_empty() {
                break;
            }
            if offline {
                // offline 模式禁止创建 download task（P3.1）。
                return Err(InstallError::new(
                    InstallStage::Acquire,
                    InstallErrorCode::ContentDownloadRequired,
                    true,
                    format!(
                        "offline mode: {} required contents are missing locally",
                        missing.len()
                    ),
                )
                .with_action(buckyos_api::InstallUserAction::ConnectNetwork));
            }
            for content in &missing {
                self.download_missing_content(view, content).await?;
            }
            let refreshed = self.rebuild_plan(data).await?;
            let still_missing = refreshed
                .required_contents
                .iter()
                .filter(|content| matches!(content.location, ContentLocation::Missing))
                .count();
            if still_missing >= missing.len() && round > 0 {
                return Err(InstallError::new(
                    InstallStage::Acquire,
                    InstallErrorCode::AcquisitionFailed,
                    true,
                    format!("{still_missing} contents remain missing after acquisition round"),
                ));
            }
            plan = refreshed;
        }

        let still_missing: Vec<_> = plan
            .required_contents
            .iter()
            .filter(|content| matches!(content.location, ContentLocation::Missing))
            .map(|content| content.content_id.clone())
            .collect();
        if !still_missing.is_empty() {
            return Err(InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::AcquisitionFailed,
                true,
                format!("missing after acquire: {}", still_missing.join(", ")),
            ));
        }
        Ok(plan)
    }

    async fn verify(
        &self,
        _view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<VerificationReport, InstallError> {
        use buckyos_api::VerificationCheck;
        let plan = data.state.plan.clone().ok_or_else(|| {
            InstallError::new(
                InstallStage::Verify,
                InstallErrorCode::Internal,
                false,
                "verify without plan",
            )
        })?;
        let doc_value = data.state.resolved_app_doc.clone().ok_or_else(|| {
            InstallError::new(
                InstallStage::Verify,
                InstallErrorCode::Internal,
                false,
                "verify without resolved app document",
            )
        })?;

        let mut checks: Vec<VerificationCheck> = Vec::new();

        // 1. App Document 身份复核：Resolve 结论不可被包签名覆盖，这里只
        //    重新计算内容身份并核对与快照绑定。
        let (doc_obj_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, &doc_value);
        checks.push(if doc_obj_id == plan.app_doc_object_id {
            VerificationCheck::pass("appdoc", "obj_id")
        } else {
            VerificationCheck::fail(
                "appdoc",
                "obj_id",
                format!("recomputed {doc_obj_id} != plan {}", plan.app_doc_object_id),
            )
        });
        if let Some(published) = plan.did_resolution.app_doc_object_id.as_ref() {
            checks.push(if *published == plan.app_doc_object_id {
                VerificationCheck::pass("appdoc", "publication_binding")
            } else {
                VerificationCheck::fail(
                    "appdoc",
                    "publication_binding",
                    format!(
                        "published {published} != installing {}",
                        plan.app_doc_object_id
                    ),
                )
            });
        }
        let doc_id_ok =
            doc_value.get("id").and_then(|v| v.as_str()) == Some(plan.app_did.to_string().as_str());
        checks.push(if doc_id_ok {
            VerificationCheck::pass("appdoc", "doc_identity")
        } else {
            VerificationCheck::fail("appdoc", "doc_identity", "document.id != app did")
        });

        let pikg_reader = self.open_candidate_pikg(data).await?;

        // 2. 逐 selected Package Meta 重新读取并重算 ObjId（不信 Inspect 缓存）。
        for package in &plan.selected_packages {
            let Some(meta_id) = package.package_meta_id.as_ref() else {
                continue;
            };
            let subject = meta_id.to_string();
            let mut meta_value = read_named_object_value(meta_id).await.ok().flatten();
            if meta_value.is_none() {
                if let Some(reader) = pikg_reader.as_ref() {
                    if let Ok(Some(body)) = reader.read_object(meta_id).await {
                        meta_value = serde_json::from_str(&body).ok();
                    }
                }
            }
            match meta_value {
                Some(_) => checks.push(VerificationCheck::pass(&subject, "package_meta_obj_id")),
                None => checks.push(VerificationCheck::fail(
                    &subject,
                    "package_meta_obj_id",
                    "package meta unavailable or failed obj id recheck",
                )),
            }
        }

        // 3. 逐内容 digest：pikg 内容全量重哈希；NamedStore 内容按内容寻址
        //    存在性复核（chunk 写入即校验）。
        for content in &plan.required_contents {
            if content.format.as_deref() == Some("named_object") {
                continue; // 上面已查。
            }
            let subject = content.content_id.clone();
            match content.location {
                ContentLocation::Pikg => match pikg_reader.as_ref() {
                    Some(reader) => match reader.verify_content(&subject).await {
                        Ok(()) => checks.push(VerificationCheck::pass(&subject, "digest")),
                        Err(err) => checks.push(VerificationCheck::fail(
                            &subject,
                            "digest",
                            err.to_string(),
                        )),
                    },
                    None => checks.push(VerificationCheck::fail(
                        &subject,
                        "digest",
                        "content planned from pikg but pikg is not available",
                    )),
                },
                ContentLocation::NamedStore | ContentLocation::Installed => {
                    let located = NamedStoreContentLocator.locate(&subject).await;
                    checks.push(if matches!(located, ContentLocation::NamedStore) {
                        VerificationCheck::pass(&subject, "content_addressed_presence")
                    } else {
                        VerificationCheck::fail(
                            &subject,
                            "content_addressed_presence",
                            "content disappeared after acquisition",
                        )
                    });
                }
                ContentLocation::Missing => {
                    checks.push(VerificationCheck::fail(&subject, "presence", "missing"));
                }
            }
        }

        // 4. 目标约束复核。
        let app_doc: buckyos_api::AppDoc =
            serde_json::from_value(doc_value.clone()).map_err(|err| {
                InstallError::new(
                    InstallStage::Verify,
                    InstallErrorCode::VerificationFailed,
                    false,
                    format!("persisted app document invalid: {err}"),
                )
            })?;
        for package in &plan.selected_packages {
            let desc = app_doc.pkg_list.get(&package.sub_pkg_name);
            let ok = desc
                .and_then(|desc| {
                    buckyos_api::SubPkgList::effective_selector(&package.sub_pkg_name, desc)
                })
                .map(|selector| selector.matches_platform(&plan.target.os, &plan.target.arch))
                .unwrap_or(false);
            checks.push(if ok {
                VerificationCheck::pass(&package.sub_pkg_name, "selector")
            } else {
                VerificationCheck::fail(
                    &package.sub_pkg_name,
                    "selector",
                    "selected package no longer matches target",
                )
            });
        }

        // 5. 安装参数中的 mount 路径安全（绝对路径、无穿越）。
        if let Some(mounts) = plan
            .install_params
            .get("data_mount_point")
            .and_then(|value| value.as_array())
        {
            for mount in mounts {
                let raw = mount.as_str().unwrap_or_default();
                let safe = raw.starts_with('/') && !raw.contains("..");
                checks.push(if safe {
                    VerificationCheck::pass(raw, "mount_safety")
                } else {
                    VerificationCheck::fail(raw, "mount_safety", "mount path unsafe")
                });
            }
        }

        Ok(VerificationReport::from_checks(
            checks,
            buckyos_kit::buckyos_get_unix_timestamp(),
        ))
    }

    async fn prepare(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<PreparedDeployment, InstallError> {
        self.prepare_impl(view, data).await
    }

    async fn deploy(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        self.deploy_impl(view, data).await
    }

    async fn activate(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<ActivateOutcome, InstallError> {
        self.activate_impl(view, data).await
    }

    async fn rollback_deploy(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        self.rollback_impl(view, data).await
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

fn infer_obj_id_from_url(url: &str) -> Option<ObjId> {
    // cyfs://{objid}/... 或路径段中内嵌 objid 的 URL。
    let stripped = url.strip_prefix("cyfs://").unwrap_or(url);
    for segment in stripped.split(['/', '?', '#']) {
        if segment.contains(':') {
            if let Ok(obj_id) = ObjId::new(segment) {
                return Some(obj_id);
            }
        }
    }
    None
}

/// 通过 TaskManager child download task 按指定 ObjId 下载对象。
async fn download_object_by_id(
    url: &str,
    obj_id: &ObjId,
    user_id: &str,
    app_id: &str,
    parent: Option<(i64, String)>,
) -> Result<(), InstallError> {
    let runtime = get_buckyos_api_runtime().map_err(|err| {
        InstallError::new(
            InstallStage::Acquire,
            InstallErrorCode::Internal,
            true,
            format!("get runtime failed: {err}"),
        )
    })?;
    let task_mgr = runtime.get_task_mgr_client().await.map_err(|err| {
        InstallError::new(
            InstallStage::Acquire,
            InstallErrorCode::Internal,
            true,
            format!("get task mgr failed: {err}"),
        )
    })?;
    let opts = parent.map(|(parent_id, root_id)| buckyos_api::CreateTaskOptions {
        parent_id: Some(parent_id),
        root_id: if root_id.is_empty() {
            None
        } else {
            Some(root_id)
        },
        ..Default::default()
    });
    let download_task_id = task_mgr
        .create_download_task(url, Some(obj_id.clone()), None, user_id, app_id, opts)
        .await
        .map_err(|err| {
            InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::AcquisitionFailed,
                true,
                format!("create download task failed: {err}"),
            )
        })?;
    // kevent 加速 + 权威回读（内部自带 timeout sweep 兜底）。
    let task = task_mgr
        .wait_for_task_end_kevent(download_task_id)
        .await
        .map_err(|err| {
            InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::AcquisitionFailed,
                true,
                format!("wait download task failed: {err}"),
            )
        })?;
    if task.status != buckyos_api::TaskStatus::Completed {
        return Err(InstallError::new(
            InstallStage::Acquire,
            InstallErrorCode::AcquisitionFailed,
            true,
            task.message
                .unwrap_or_else(|| format!("download task {download_task_id} did not complete")),
        ));
    }
    Ok(())
}

impl ProductionInstallDriver {
    /// 下载单个缺失内容：Source 顺序尝试，全部失败才报错；
    /// 无论从哪个 Source 取得，最终都按同一 ObjId 验证（内容寻址）。
    async fn download_missing_content(
        &self,
        view: &InstallTaskView,
        content: &buckyos_api::PlannedContent,
    ) -> Result<(), InstallError> {
        let Ok(obj_id) = ObjId::new(&content.content_id) else {
            return Err(InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::AcquisitionFailed,
                false,
                format!("content id `{}` is not a valid obj id", content.content_id),
            ));
        };
        let mut urls = content.sources.clone();
        urls.push(format!("cyfs://{}", content.content_id));

        let mut last_error: Option<InstallError> = None;
        for url in urls {
            match download_object_by_id(
                url.as_str(),
                &obj_id,
                view.user_id.as_str(),
                view.app_id.as_str(),
                Some((view.id, view.root_id.clone())),
            )
            .await
            {
                Ok(()) => {
                    // 下载后按同一 ObjId 复核本地存在。
                    let located = NamedStoreContentLocator.locate(&content.content_id).await;
                    if matches!(located, ContentLocation::NamedStore) {
                        return Ok(());
                    }
                    last_error = Some(InstallError::new(
                        InstallStage::Acquire,
                        InstallErrorCode::AcquisitionFailed,
                        true,
                        format!(
                            "source `{url}` reported success but `{}` is not in named store",
                            content.content_id
                        ),
                    ));
                }
                Err(err) => {
                    warn!(
                        "download `{}` from `{url}` failed: {err}",
                        content.content_id
                    );
                    last_error = Some(err);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::AcquisitionFailed,
                true,
                format!("no source available for `{}`", content.content_id),
            )
        }))
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
