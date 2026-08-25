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
use crate::app_install_planner::{
    build_install_plan, default_install_params, ContentLocator, PlannerInput,
};
use crate::app_install_resolver::{
    bind_candidate_document, normalize_identifier, resolve_domain_alias, AppDidResolver,
    NameClientAppResolver, NormalizedIdentifier,
};
use crate::app_package_namespace::{
    validate_app_package_namespace, validate_package_meta_namespace,
};
use crate::app_staging::PikgStagingStore;
use crate::pikg::{PikgInspection, PikgReader};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, install_record_key, AppId, AppInstallTaskData, AppServiceSpec,
    CandidateHandle, ContentLocation, InspectedContent, InstallError, InstallErrorCode,
    InstallInspection, InstallParams, InstallPlan, InstallPlanStatus, InstallPlanUse,
    InstallRecord, InstallSource, InstallSourceIdentity, InstallStage, InstallTarget,
    PikgStagingPurpose, PreparedDeployment, ServiceExposeSetting, ServiceSetting, ServiceState,
    SystemConfigError, VerificationReport, APP_CAPABILITY_MINI_GPU_MEMORY,
    APP_CAPABILITY_MINI_GPU_TFLOPS, APP_CAPABILITY_MINI_MEMORY, OBJ_TYPE_APP_DOC,
    TASK_DATA_TYPE_APP_UPDATE,
};
use log::warn;
use name_lib::{DeviceInfo, DID};
use ndn_lib::{build_named_object_by_json, ChunkId, ChunkReader, ObjId};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

/// pikg staging root：Control Panel 数据目录下的受控子目录（D5）。
pub fn pikg_staging_root() -> PathBuf {
    buckyos_kit::get_buckyos_root_dir()
        .join("cache")
        .join("control_panel")
        .join("pikg_staging")
}

fn inherit_upgrade_params(mut params: InstallParams, spec: &AppServiceSpec) -> InstallParams {
    params.auto_start = spec.enable && spec.state != ServiceState::Stopped;
    params.expected_instance_count = spec.expected_instance_count;
    params.selected_components = spec.selected_components.clone();
    params.permissions = spec.permission.clone();
    params.data_mount_points = spec.spec_config.data_mount_point.clone();
    params.local_cache_mount_points = spec.spec_config.local_cache_mount_point.clone();
    params.external_mount_points = spec.spec_config.external_mount_point.clone();
    params.bash_envs = spec.spec_config.bash_envs.clone();
    params.res_pool_id = Some(spec.spec_config.res_pool_id.clone());
    for service_name in spec.app_doc.service_config_tips.service_endpoints.keys() {
        let enabled = spec.spec_config.service_config.contains_key(service_name);
        let expose = spec
            .spec_config
            .expose_config
            .get(service_name)
            .map(|current| ServiceExposeSetting {
                route: current.route.clone(),
                scope: current.scope.clone(),
                allow_guest: current.allow_guest,
            });
        params
            .service_settings
            .services
            .insert(service_name.clone(), ServiceSetting { enabled, expose });
    }
    params
}

pub struct ProductionInstallDriver {
    resolver: Arc<dyn AppDidResolver>,
    staging_root: PathBuf,
    staging_store: Arc<PikgStagingStore>,
}

impl ProductionInstallDriver {
    pub fn new(staging_store: Arc<PikgStagingStore>) -> Self {
        Self {
            resolver: Arc::new(NameClientAppResolver::new()),
            staging_root: pikg_staging_root(),
            staging_store,
        }
    }

    pub(crate) async fn materialize_candidate_pikg(
        &self,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let Some(reader) = self.open_candidate_pikg(data).await? else {
            return Ok(());
        };
        let plan = data.state.plan.as_ref().ok_or_else(|| {
            InstallError::new(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                false,
                "materialize without install plan",
            )
        })?;
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            InstallError::new(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                true,
                format!("get runtime for pikg materialization failed: {error}"),
            )
        })?;
        let named_store = runtime.get_named_store().await.map_err(|error| {
            InstallError::new(
                InstallStage::Prepare,
                InstallErrorCode::Internal,
                true,
                format!("open named store for pikg materialization failed: {error}"),
            )
        })?;

        let app_doc_value = data.state.resolved_app_doc.clone().ok_or_else(|| {
            InstallError::new(
                InstallStage::Prepare,
                InstallErrorCode::InvalidPackage,
                false,
                "resolved AppDoc is missing during materialization",
            )
        })?;
        let (app_doc_id, app_doc_body) =
            build_named_object_by_json(OBJ_TYPE_APP_DOC, &app_doc_value);
        if app_doc_id != plan.app.object_id {
            return Err(InstallError::new(
                InstallStage::Prepare,
                InstallErrorCode::VerificationFailed,
                false,
                "AppDoc changed after verification",
            ));
        }
        named_store
            .put_object(&app_doc_id, app_doc_body.as_str())
            .await
            .map_err(|error| {
                InstallError::new(
                    InstallStage::Prepare,
                    InstallErrorCode::AcquisitionFailed,
                    true,
                    format!("materialize AppDoc `{app_doc_id}` failed: {error}"),
                )
            })?;

        for package in &plan.selected_packages {
            let Some(meta_id) = package.package_meta_id.as_ref() else {
                continue;
            };
            let body = reader
                .read_object(meta_id)
                .await
                .map_err(|error| {
                    InstallError::new(
                        InstallStage::Prepare,
                        InstallErrorCode::InvalidPackage,
                        false,
                        format!("read PackageMeta `{meta_id}` from pikg failed: {error}"),
                    )
                })?
                .ok_or_else(|| {
                    InstallError::new(
                        InstallStage::Prepare,
                        InstallErrorCode::InvalidPackage,
                        false,
                        format!("PackageMeta `{meta_id}` is absent from pikg"),
                    )
                })?;
            named_store
                .put_object(meta_id, body.as_str())
                .await
                .map_err(|error| {
                    InstallError::new(
                        InstallStage::Prepare,
                        InstallErrorCode::AcquisitionFailed,
                        true,
                        format!("materialize PackageMeta `{meta_id}` failed: {error}"),
                    )
                })?;
        }

        for content in &plan.required_contents {
            if !reader.has_content(content.content_id.as_str()) {
                continue;
            }
            let chunk_id = ChunkId::new(content.content_id.as_str()).map_err(|error| {
                InstallError::new(
                    InstallStage::Prepare,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!(
                        "pikg content `{}` is not a ChunkId: {error}",
                        content.content_id
                    ),
                )
            })?;
            if named_store.have_chunk(&chunk_id).await {
                continue;
            }
            let temp_path = self.staging_root.join(format!(
                ".materialize-{}-{}.tmp",
                plan.task_id,
                uuid::Uuid::new_v4().simple()
            ));
            let result = async {
                reader
                    .copy_content_to_file(content.content_id.as_str(), &temp_path)
                    .await
                    .map_err(|error| {
                        InstallError::new(
                            InstallStage::Prepare,
                            InstallErrorCode::InvalidPackage,
                            false,
                            format!(
                                "extract pikg content `{}` failed: {error}",
                                content.content_id
                            ),
                        )
                    })?;
                let file = tokio::fs::File::open(&temp_path).await.map_err(|error| {
                    InstallError::new(
                        InstallStage::Prepare,
                        InstallErrorCode::AcquisitionFailed,
                        true,
                        format!("open materialized content failed: {error}"),
                    )
                })?;
                let size = file
                    .metadata()
                    .await
                    .map_err(|error| {
                        InstallError::new(
                            InstallStage::Prepare,
                            InstallErrorCode::AcquisitionFailed,
                            true,
                            format!("stat materialized content failed: {error}"),
                        )
                    })?
                    .len();
                let chunk_reader: ChunkReader = Box::pin(file);
                named_store
                    .put_chunk_by_reader(&chunk_id, size, chunk_reader)
                    .await
                    .map_err(|error| {
                        InstallError::new(
                            InstallStage::Prepare,
                            InstallErrorCode::AcquisitionFailed,
                            true,
                            format!(
                                "write pikg content `{}` into named store failed: {error}",
                                content.content_id
                            ),
                        )
                    })?;
                Ok(())
            }
            .await;
            let _ = tokio::fs::remove_file(&temp_path).await;
            result?;
        }
        Ok(())
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
        view: &InstallTaskView,
        data: &AppInstallTaskData,
        staging_handle: &str,
        policy: buckyos_api::InstallPolicy,
    ) -> Result<ResolveOutcome, InstallError> {
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            Self::invalid_request(format!("get runtime for staging handle failed: {error}"))
        })?;
        let purpose = if view.id == "inspect-only" {
            PikgStagingPurpose::Inspect
        } else {
            PikgStagingPurpose::Install
        };
        let lease = (view.id != "inspect-only").then_some(view.id.as_str());
        let (metadata, staged_path) = self
            .staging_store
            .resolve(
                staging_handle,
                data.request.creator_user_id.as_str(),
                data.request.creator_app_id.as_str(),
                &runtime.zone_id,
                purpose,
                lease,
            )
            .await
            .map_err(|error| Self::invalid_request(error.to_string()))?;
        let digest = metadata.pikg_digest;

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
        let app_did = inspection.app_doc.app_did().clone();
        let candidate_value = serde_json::to_value(&inspection.app_doc).map_err(|err| {
            Self::invalid_request(format!("serialize candidate app doc failed: {err}"))
        })?;

        let mut resolved = self.resolver.resolve_app(&app_did, policy).await?;
        // candidate 绑定：did/owner 硬约束（owner 冒充在这里拒绝）。
        let binding = bind_candidate_document(&app_did, &resolved.snapshot, &candidate_value)?;

        let resolved_app_doc = if matches!(policy, buckyos_api::InstallPolicy::LocalDeveloper) {
            resolved.snapshot.evidence =
                Some(buckyos_api::DidEvidenceLevel::LocalDeveloperAuthority);
            resolved.snapshot.local_authority_app_doc_object_id =
                Some(binding.candidate_obj_id.clone());
            Some(candidate_value.clone())
        } else {
            resolved
                .document_value
                .clone()
                .or(Some(candidate_value.clone()))
        };

        Ok(ResolveOutcome {
            candidate: Some(CandidateHandle {
                kind: "pikg".to_string(),
                pikg_digest: Some(inspection.pikg_digest.clone()),
                staging_path: Some(canonical.to_string_lossy().to_string()),
                staging_handle: Some(staging_handle.to_string()),
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
            NormalizedIdentifier::DomainAlias(alias) => {
                let app_did = resolve_domain_alias(alias.as_str()).await?;
                let resolved = self.resolver.resolve_app(&app_did, policy).await?;
                Ok(ResolveOutcome {
                    candidate: None,
                    resolution: resolved.snapshot,
                    resolved_app_doc: resolved.document_value,
                })
            }
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
                    .get("did")
                    .and_then(|value| value.as_str())
                    .ok_or_else(|| {
                        Self::invalid_request("candidate app document has no did".to_string())
                    })?;
                let app_did = DID::from_str(app_did_raw).map_err(|err| {
                    Self::invalid_request(format!("candidate app document did invalid: {err}"))
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
                        staging_handle: None,
                        app_doc_object_id: Some(binding.candidate_obj_id),
                        source_url: None,
                    }),
                    resolution: resolved.snapshot,
                    resolved_app_doc,
                })
            }
            NormalizedIdentifier::Url(url) => {
                Err(Self::invalid_request(format!(
                    "Control Panel does not fetch client URLs (`{url}`); the Tool must upload bytes and finalize an opaque staging handle"
                )))
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

    /// 用真实位置重建 plan（fingerprint 只绑定身份/目标/参数，不含位置，
    /// 因此刷新不会作废已绑定的 approval）。
    async fn rebuild_plan(
        &self,
        data: &AppInstallTaskData,
    ) -> Result<InstallInspection, InstallError> {
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
        let persisted_plan = data.state.plan.as_ref().ok_or_else(|| {
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
            .unwrap_or_default();
        let pikg_reader = self.open_candidate_pikg(data).await?;
        let pikg_inspection: Option<&PikgInspection> =
            pikg_reader.as_ref().map(|reader| reader.inspection());
        build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id,
                snapshot: &snapshot,
                policy: data.request.policy,
                plan_use: persisted_plan.plan_use,
                task_id: persisted_plan.task_id.clone(),
                owner_user_id: persisted_plan.owner_user_id.clone(),
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

    async fn release_candidate_staging(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        let handle = data
            .state
            .candidate
            .as_ref()
            .and_then(|candidate| candidate.staging_handle.as_deref());
        let runtime = get_buckyos_api_runtime().map_err(|error| {
            InstallError::new(
                InstallStage::Resolve,
                InstallErrorCode::Internal,
                true,
                format!("get runtime for staging release failed: {error}"),
            )
        })?;
        if let Some(handle) = handle {
            self.staging_store
                .release(
                    handle,
                    data.request.creator_user_id.as_str(),
                    data.request.creator_app_id.as_str(),
                    &runtime.zone_id,
                    Some(view.id.as_str()),
                )
                .await
                .map_err(|error| {
                    InstallError::new(
                        InstallStage::Resolve,
                        InstallErrorCode::Internal,
                        true,
                        format!("release staging lease failed: {error}"),
                    )
                })?;
        }
        if let Some(plan) = data.state.plan.as_ref() {
            let key = format!(
                "services/control_panel/app_mutations/{}",
                plan.app_instance_id
            );
            let client = runtime.get_system_config_client().await.map_err(|error| {
                InstallError::new(
                    InstallStage::Resolve,
                    InstallErrorCode::Internal,
                    true,
                    format!("get system config for mutation release failed: {error}"),
                )
            })?;
            if let Ok(current) = client.get(&key).await {
                let owned = serde_json::from_str::<Value>(&current.value)
                    .ok()
                    .and_then(|value| {
                        value
                            .get("task_id")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .as_deref()
                    == Some(view.id.as_str());
                if owned {
                    let mut actions = std::collections::HashMap::new();
                    actions.insert(key.clone(), buckyos_kit::KVAction::Remove);
                    let _ = client.exec_tx(actions, Some((key, current.version))).await;
                }
            }
        }
        Ok(())
    }
}

#[async_trait]
impl InstallStageDriver for ProductionInstallDriver {
    async fn resolve(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<ResolveOutcome, InstallError> {
        match &data.request.source {
            InstallSource::Identifier { identifier, .. } => {
                self.resolve_identifier(identifier, data.request.policy)
                    .await
            }
            InstallSource::LocalPikg { staging_handle } => {
                self.resolve_local_pikg(view, data, staging_handle, data.request.policy)
                    .await
            }
        }
    }

    async fn inspect(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<Option<InstallInspection>, InstallError> {
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
            return Ok(None);
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

        let locator = NamedStoreContentLocator;
        let runtime = get_buckyos_api_runtime().map_err(|err| {
            InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::Internal,
                true,
                format!("get runtime failed: {err}"),
            )
        })?;
        let owner_user_id = data.request.owner_user_id.clone();
        let app_id = AppId::from_app_did(app_doc.app_did()).map_err(|error| {
            InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::InvalidRequest,
                false,
                error,
            )
        })?;
        let is_update = view.task_type == TASK_DATA_TYPE_APP_UPDATE;
        let inherited_params = if is_update {
            let record_key = install_record_key(&owner_user_id, &app_id);
            let client = runtime.get_system_config_client().await.map_err(|error| {
                InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::Internal,
                    true,
                    format!("get system config for upgrade baseline failed: {error}"),
                )
            })?;
            let record_value = client.get(&record_key).await.map_err(|error| match error {
                SystemConfigError::KeyNotFound(_) => InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::PlanNotApplicable,
                    false,
                    "upgrade target is not installed",
                ),
                other => InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::Internal,
                    true,
                    format!("read upgrade install record failed: {other}"),
                ),
            })?;
            let record: InstallRecord =
                serde_json::from_str(&record_value.value).map_err(|error| {
                    InstallError::new(
                        InstallStage::Inspect,
                        InstallErrorCode::Internal,
                        false,
                        format!("invalid upgrade install record: {error}"),
                    )
                })?;
            let spec_path = buckyos_api::user_app_spec_key(&owner_user_id, &app_id);
            let spec: AppServiceSpec = client
                .get(&spec_path)
                .await
                .map_err(|error| {
                    InstallError::new(
                        InstallStage::Inspect,
                        InstallErrorCode::PlanNotApplicable,
                        false,
                        format!("upgrade desired spec is missing: {error}"),
                    )
                })
                .and_then(|value| {
                    serde_json::from_str(&value.value).map_err(|error| {
                        InstallError::new(
                            InstallStage::Inspect,
                            InstallErrorCode::Internal,
                            false,
                            format!("invalid upgrade desired spec: {error}"),
                        )
                    })
                })?;
            Some(inherit_upgrade_params(record.install_params, &spec))
        } else {
            None
        };
        let requested_params = data.state.requested_params.clone();
        let install_params = requested_params.clone().unwrap_or_else(|| {
            inherited_params
                .unwrap_or_else(|| default_install_params(&app_doc, data.request.policy))
        });

        let inspection = build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id,
                snapshot: &snapshot,
                policy: data.request.policy,
                plan_use: if is_update {
                    InstallPlanUse::Upgrade
                } else {
                    InstallPlanUse::FreshInstall
                },
                task_id: view.planning_task_id.clone(),
                owner_user_id,
                target,
                install_params,
                pikg: pikg_inspection,
            },
            &locator,
        )
        .await?;
        Ok(Some(inspection))
    }

    async fn acquire(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<InstallPlanStatus, InstallError> {
        let offline = Self::offline_requested(data);
        // 最多几轮：meta 下载后 payload 清单才展开，需要再补一轮。
        let persisted_plan = data.state.plan.as_ref().ok_or_else(|| {
            InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::Internal,
                false,
                "acquire without plan",
            )
        })?;
        let mut inspection = self.rebuild_plan(data).await?;
        if inspection.plan.plan_fingerprint != persisted_plan.plan_fingerprint {
            return Err(InstallError::new(
                InstallStage::Acquire,
                InstallErrorCode::PlanStale,
                false,
                "source or immutable plan material changed before acquisition",
            ));
        }
        for round in 0..4 {
            let missing: Vec<_> = inspection
                .status
                .contents
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
            if refreshed.plan.plan_fingerprint != persisted_plan.plan_fingerprint {
                return Err(InstallError::new(
                    InstallStage::Acquire,
                    InstallErrorCode::PlanStale,
                    false,
                    "acquisition revealed immutable content not present in the approved plan",
                ));
            }
            let still_missing = refreshed
                .status
                .contents
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
            inspection = refreshed;
        }

        let still_missing: Vec<_> = inspection
            .status
            .contents
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
        Ok(inspection.status)
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

        let recomputed_fingerprint = InstallPlan::compute_fingerprint(
            plan.plan_use,
            &plan.task_id,
            &plan.app_instance_id,
            &plan.owner_user_id,
            &plan.source_identity,
            &plan.app,
            &plan.app_doc,
            &plan.resolution,
            &plan.target,
            &plan.install_params,
            &plan.service_spec_config,
            &plan.selected_packages,
            &plan.required_contents,
        );
        checks.push(if recomputed_fingerprint == plan.plan_fingerprint {
            VerificationCheck::pass("plan", "fingerprint")
        } else {
            VerificationCheck::fail(
                "plan",
                "fingerprint",
                format!(
                    "recomputed {recomputed_fingerprint} != persisted {}",
                    plan.plan_fingerprint
                ),
            )
        });
        let approval_matches = data.state.approval.as_ref().is_some_and(|approval| {
            approval.plan_fingerprint == recomputed_fingerprint
                && approval.target == plan.target
                && approval.install_params == plan.install_params
        });
        checks.push(if approval_matches {
            VerificationCheck::pass("plan", "approval_binding")
        } else {
            VerificationCheck::fail(
                "plan",
                "approval_binding",
                "approval does not bind the recomputed plan, target, and install params",
            )
        });

        // 1. App Document 身份复核：Resolve 结论不可被包签名覆盖，这里只
        //    重新计算内容身份并核对与快照绑定。
        let (doc_obj_id, _) = build_named_object_by_json(OBJ_TYPE_APP_DOC, &doc_value);
        checks.push(if doc_obj_id == plan.app.object_id {
            VerificationCheck::pass("appdoc", "obj_id")
        } else {
            VerificationCheck::fail(
                "appdoc",
                "obj_id",
                format!("recomputed {doc_obj_id} != plan {}", plan.app.object_id),
            )
        });
        let uses_local_developer_authority = plan
            .source_identity
            .uses_local_developer_authority(data.request.policy)
            && plan
                .resolution
                .has_local_developer_authority_for(&plan.source_identity);
        if uses_local_developer_authority {
            checks.push(VerificationCheck::pass(
                "appdoc",
                "local_developer_authority",
            ));
        } else if let Some(published) = plan.resolution.app_doc_object_id.as_ref() {
            checks.push(if *published == plan.app.object_id {
                VerificationCheck::pass("appdoc", "publication_binding")
            } else {
                VerificationCheck::fail(
                    "appdoc",
                    "publication_binding",
                    format!("published {published} != installing {}", plan.app.object_id),
                )
            });
        }
        let plan_app_did = plan.app.did.to_string();
        let doc_did_ok =
            doc_value.get("did").and_then(|v| v.as_str()) == Some(plan_app_did.as_str());
        checks.push(if doc_did_ok {
            VerificationCheck::pass("appdoc", "doc_identity")
        } else {
            VerificationCheck::fail("appdoc", "doc_identity", "document.did != app did")
        });

        let pikg_reader = self.open_candidate_pikg(data).await?;

        match (&plan.source_identity, pikg_reader.as_ref()) {
            (
                InstallSourceIdentity::Pikg {
                    app_doc_object_id,
                    pikg_digest,
                },
                Some(reader),
            ) => {
                checks.push(if reader.pikg_digest() == pikg_digest {
                    VerificationCheck::pass("pikg", "digest_binding")
                } else {
                    VerificationCheck::fail(
                        "pikg",
                        "digest_binding",
                        format!("staged {} != plan {pikg_digest}", reader.pikg_digest()),
                    )
                });
                checks.push(
                    if reader.inspection().app_doc_object_id == *app_doc_object_id
                        && plan.app.object_id == *app_doc_object_id
                    {
                        VerificationCheck::pass("pikg", "appdoc_binding")
                    } else {
                        VerificationCheck::fail(
                            "pikg",
                            "appdoc_binding",
                            "PIKG AppDoc ObjectId, source identity and plan AppDoc do not match",
                        )
                    },
                );
                checks.push(if reader.inspection().app_doc.app_did() == &plan.app.did {
                    VerificationCheck::pass("pikg", "app_did_binding")
                } else {
                    VerificationCheck::fail(
                        "pikg",
                        "app_did_binding",
                        "PIKG AppDID does not match the plan AppDID",
                    )
                });
            }
            (InstallSourceIdentity::Pikg { .. }, None) => checks.push(VerificationCheck::fail(
                "pikg",
                "source_binding",
                "PIKG plan has no available immutable staged source",
            )),
            (InstallSourceIdentity::Catalog { .. }, Some(_)) => {
                checks.push(VerificationCheck::fail(
                    "pikg",
                    "source_binding",
                    "staged PIKG candidate produced a Catalog source identity",
                ))
            }
            (InstallSourceIdentity::Catalog { .. }, None) => {}
        }

        let app_doc: buckyos_api::AppDoc =
            serde_json::from_value(doc_value.clone()).map_err(|err| {
                InstallError::new(
                    InstallStage::Verify,
                    InstallErrorCode::VerificationFailed,
                    false,
                    format!("persisted app document invalid: {err}"),
                )
            })?;
        let namespace =
            validate_app_package_namespace(&app_doc, &plan.resolution, InstallStage::Verify)?;

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
                Some(value) => {
                    let meta: package_lib::PackageMeta =
                        serde_json::from_value(value).map_err(|err| {
                            InstallError::new(
                                InstallStage::Verify,
                                InstallErrorCode::InvalidPackage,
                                false,
                                format!("package meta `{subject}` schema invalid: {err}"),
                            )
                        })?;
                    let declared =
                        app_doc.pkg_list.get(&package.sub_pkg_name).ok_or_else(|| {
                            InstallError::new(
                                InstallStage::Verify,
                                InstallErrorCode::InvalidPackage,
                                false,
                                format!(
                                    "selected package `{}` is absent from AppDoc",
                                    package.sub_pkg_name
                                ),
                            )
                        })?;
                    validate_package_meta_namespace(
                        &namespace,
                        &package.sub_pkg_name,
                        &declared.pkg_id,
                        meta.name.as_str(),
                        meta.version.as_str(),
                        InstallStage::Verify,
                    )?;
                    checks.push(VerificationCheck::pass(&subject, "package_meta_obj_id"));
                    checks.push(VerificationCheck::pass(&subject, "package_namespace"));
                }
                None => checks.push(VerificationCheck::fail(
                    &subject,
                    "package_meta_obj_id",
                    "package meta unavailable or failed obj id recheck",
                )),
            }
        }

        // 3. 逐内容 digest：pikg 内容全量重哈希；NamedStore 内容按内容寻址
        //    存在性复核（chunk 写入即校验）。
        let plan_status = data.state.plan_status.as_ref().ok_or_else(|| {
            InstallError::new(
                InstallStage::Verify,
                InstallErrorCode::Internal,
                false,
                "verify without dynamic plan status",
            )
        })?;
        for content in &plan.required_contents {
            if content.format.as_deref() == Some("named_object") {
                continue; // 上面已查。
            }
            let subject = content.content_id.clone();
            let location = plan_status
                .contents
                .iter()
                .find(|item| item.content_id == content.content_id)
                .map(|item| item.location)
                .unwrap_or(ContentLocation::Missing);
            match location {
                ContentLocation::Pikg => match pikg_reader.as_ref() {
                    Some(reader) => match reader.verify_content(&subject).await {
                        Ok(()) if content.format.as_deref() == Some("archive") => {
                            match reader.verify_package_archive(&subject).await {
                                Ok(()) => checks.push(VerificationCheck::pass(
                                    &subject,
                                    "digest_and_archive_safety",
                                )),
                                Err(err) => checks.push(VerificationCheck::fail(
                                    &subject,
                                    "archive_safety",
                                    err.to_string(),
                                )),
                            }
                        }
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
        for package in &plan.selected_packages {
            let desc = app_doc.pkg_list.get(&package.sub_pkg_name);
            let ok = desc
                .and_then(|desc| {
                    buckyos_api::SubPkgList::effective_selector(&package.sub_pkg_name, desc)
                        .map(|selector| (desc, selector))
                })
                .map(|(desc, selector)| {
                    crate::app_install_planner::package_matches_target(
                        &selector,
                        desc,
                        &plan.target,
                    )
                })
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

            if package.docker_image_name.is_some() {
                let digest_matches = desc
                    .and_then(|desc| desc.docker_image_digest.as_ref())
                    .is_some_and(|digest| {
                        package.docker_image_digest.as_ref() == Some(digest)
                            && valid_sha256_digest(digest)
                    });
                checks.push(if digest_matches {
                    VerificationCheck::pass(&package.sub_pkg_name, "docker_image_digest")
                } else {
                    VerificationCheck::fail(
                        &package.sub_pkg_name,
                        "docker_image_digest",
                        "selected Docker image is not bound to the App Document by a valid sha256 digest",
                    )
                });
            }
        }

        // 5. 最终运行配置中的 mount 路径安全（绝对路径、无穿越）。
        for mounts in [
            &plan.service_spec_config.data_mount_point,
            &plan.service_spec_config.local_cache_mount_point,
            &plan.service_spec_config.external_mount_point,
        ] {
            for (container_path, mount) in mounts {
                let container_raw = container_path.display().to_string();
                checks.push(if is_safe_absolute_path(container_path) {
                    VerificationCheck::pass(&container_raw, "mount_container_path")
                } else {
                    VerificationCheck::fail(
                        &container_raw,
                        "mount_container_path",
                        "container mount path must be absolute and contain no traversal",
                    )
                });
                let target_raw = mount.target_path.display().to_string();
                checks.push(if is_safe_mount_target_path(&mount.target_path) {
                    VerificationCheck::pass(&target_raw, "mount_target_path")
                } else {
                    VerificationCheck::fail(
                        &target_raw,
                        "mount_target_path",
                        "mount target path contains traversal or a platform prefix",
                    )
                });
                checks.push(
                    if matches!(
                        mount.access.as_str(),
                        "" | "read_only" | "ro" | "read_write" | "read_write_append" | "rw"
                    ) {
                        VerificationCheck::pass(&container_raw, "mount_access")
                    } else {
                        VerificationCheck::fail(
                            &container_raw,
                            "mount_access",
                            format!("invalid mount access `{}`", mount.access),
                        )
                    },
                );
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

    async fn release_staging(
        &self,
        view: &InstallTaskView,
        data: &AppInstallTaskData,
    ) -> Result<(), InstallError> {
        self.release_candidate_staging(view, data).await
    }
}

fn valid_sha256_digest(raw: &str) -> bool {
    let Some(hex) = raw.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_safe_absolute_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|component| !matches!(component, Component::ParentDir | Component::CurDir))
}

fn is_safe_mount_target_path(path: &Path) -> bool {
    path.components().all(|component| {
        !matches!(
            component,
            Component::ParentDir | Component::CurDir | Component::Prefix(_)
        )
    })
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

    async fn load_json(&self, content_id: &str) -> Option<Value> {
        let obj_id = ObjId::new(content_id).ok()?;
        read_named_object_value(&obj_id).await.ok().flatten()
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

/// 通过 TaskManager child download task 按指定 ObjId 下载对象。
async fn download_object_by_id(
    url: &str,
    obj_id: &ObjId,
    _user_id: &str,
    _app_id: &str,
    _parent: Option<(String, String)>,
) -> Result<(), InstallError> {
    // 2.0: acquisition runs in-process (the task-manager built-in download
    // executor is gone); progress surfaces through the install task itself.
    crate::ndn_download::download_object_to_named_store(
        url,
        obj_id,
        &buckyos_api::DownloadTaskOptions::default(),
    )
    .await
    .map(|_| ())
    .map_err(|err| {
        InstallError::new(
            InstallStage::Acquire,
            InstallErrorCode::AcquisitionFailed,
            true,
            format!("download {url} failed: {err}"),
        )
    })
}

impl ProductionInstallDriver {
    /// 下载单个缺失内容：Source 顺序尝试，全部失败才报错；
    /// 无论从哪个 Source 取得，最终都按同一 ObjId 验证（内容寻址）。
    async fn download_missing_content(
        &self,
        view: &InstallTaskView,
        content: &InspectedContent,
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
                Some((view.id.clone(), view.root_id.clone())),
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
        let mut capabilities: BTreeMap<String, i64> = info
            .device_doc
            .capbilities
            .iter()
            .map(|(name, value)| (name.clone(), *value))
            .collect();
        if let Some(total_mem) = info.total_mem {
            capabilities.insert(
                APP_CAPABILITY_MINI_MEMORY.to_string(),
                i64::try_from(total_mem).unwrap_or(i64::MAX),
            );
        }
        if let Some(gpu_total_mem) = info.gpu_total_mem {
            capabilities.insert(
                APP_CAPABILITY_MINI_GPU_MEMORY.to_string(),
                i64::try_from(gpu_total_mem).unwrap_or(i64::MAX),
            );
        }
        if let Some(gpu_tflops) = info.gpu_tflops {
            capabilities.insert(
                APP_CAPABILITY_MINI_GPU_TFLOPS.to_string(),
                gpu_tflops.floor() as i64,
            );
        }
        let runtime_version = info
            .device_doc
            .extra_info
            .get("runtime_version")
            .or_else(|| info.device_doc.extra_info.get("buckyos_version"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let kernel_version = info
            .device_doc
            .extra_info
            .get("kernel_version")
            .and_then(Value::as_str)
            .map(str::to_string);
        return Ok(InstallTarget {
            node_did: Some(info.device_doc.id.clone()),
            node_id: Some(name),
            os: buckyos_api::normalize_os(info.os.as_str()),
            arch: buckyos_api::normalize_arch(info.arch.as_str()),
            kernel_version,
            runtime_version,
            capabilities,
        });
    }
    Err("no device info found under devices/".to_string())
}
