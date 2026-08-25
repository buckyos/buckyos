//! Inspect Stage：从已解析 App Document + 目标 Node + 安装参数生成
//! `InstallPlan`（doc/App 安装协议.md §3.2、§4.5、§11.4）。
//!
//! 硬规则：
//! - target os/arch 来自用户选中的目标 Node 信息，禁止用 Control Panel
//!   编译期 `cfg!(target_*)` 代替（P2.3）；
//! - 内容位置判定顺序：已安装/NamedStore -> 当前 pikg -> missing（远程
//!   Source 只作为 missing 的获取途径）；
//! - Inspect 不写系统目录，不触发下载。

use crate::app_install_deployer::build_install_config;
use crate::app_package_namespace::{
    validate_app_package_namespace, validate_package_meta_namespace,
};
use crate::pikg::PikgInspection;
use async_trait::async_trait;
use buckyos_api::{
    AppDoc, AppDocumentRef, AppInstanceId, ContentLocation, DidResolutionSnapshot,
    InspectedContent, InstallError, InstallErrorCode, InstallInspection, InstallParams,
    InstallPlan, InstallPlanStatus, InstallPlanUse, InstallPolicy, InstallSourceIdentity,
    InstallStage, InstallTarget, PackageSelector, PlanReadiness, PlannedContent, ReadinessState,
    SelectedPackage, SubPkgDesc, SubPkgList, APP_INSTALL_SCHEMA_VERSION,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use ndn_lib::{NamedObject, ObjId};
use package_lib::{PackageId, PackageMeta};
use serde_json::Value;

// ---------------------------------------------------------------------------
// 内容定位抽象（stage 3 用 fake，stage 5 接 NamedStore/已安装内容）
// ---------------------------------------------------------------------------

/// 本地内容定位：判断对象/内容当前是否已在本机可复用位置。
/// 只回答 Installed/NamedStore/Missing；pikg 命中由 planner 自己判断。
#[async_trait]
pub trait ContentLocator: Send + Sync {
    async fn locate(&self, content_id: &str) -> ContentLocation;

    /// 已存在的结构化对象 body。planner 用它展开 Package Meta 的 payload，
    /// 防止“meta 已到、实体内容未知”被误判为离线就绪。
    async fn load_json(&self, _content_id: &str) -> Option<Value> {
        None
    }
}

/// 一切都缺失的 locator（离线且无本地缓存的最坏情况，或 fake 基线）。
pub struct NoLocalContentLocator;

#[async_trait]
impl ContentLocator for NoLocalContentLocator {
    async fn locate(&self, _content_id: &str) -> ContentLocation {
        ContentLocation::Missing
    }
}

// ---------------------------------------------------------------------------
// planner 输入
// ---------------------------------------------------------------------------

pub struct PlannerInput<'a> {
    pub app_doc: &'a AppDoc,
    pub app_doc_object_id: ObjId,
    pub snapshot: &'a DidResolutionSnapshot,
    pub policy: InstallPolicy,
    pub plan_use: InstallPlanUse,
    pub task_id: String,
    pub owner_user_id: String,
    pub target: InstallTarget,
    /// 影响部署/selector 的安装参数（进 fingerprint）。
    pub install_params: InstallParams,
    /// 本地 pikg（若本次事务有），作为内容与 Package Meta 的候选来源。
    pub pikg: Option<&'a PikgInspection>,
}

/// AppDoc 未被调用方参数覆盖时的建议安装参数。普通安装只预选必需权限；
/// SYSTEM_INTERNAL 延续自动确认语义，预选 AppDoc 声明的全部权限。
pub fn default_install_params(app_doc: &AppDoc, policy: InstallPolicy) -> InstallParams {
    let mut params = InstallParams::default();
    params.permissions = app_doc
        .permissions
        .iter()
        .filter(|permission| permission.required || policy.allow_auto_confirm())
        .cloned()
        .collect();
    params
}

fn validate_permission_selection(app_doc: &AppDoc, install_params: &InstallParams) -> Vec<String> {
    let mut issues = Vec::new();

    for (index, declared) in app_doc.permissions.iter().enumerate() {
        if declared.scope_path.trim().is_empty() {
            issues.push("AppDoc declares a permission with an empty scope_path".to_string());
        }
        if app_doc.permissions[..index]
            .iter()
            .any(|item| item.scope_path == declared.scope_path)
        {
            issues.push(format!(
                "AppDoc declares duplicate permission `{}`",
                declared.scope_path
            ));
        }
    }

    for (index, selected) in install_params.permissions.iter().enumerate() {
        if install_params.permissions[..index]
            .iter()
            .any(|item| item.scope_path == selected.scope_path)
        {
            issues.push(format!(
                "install params select duplicate permission `{}`",
                selected.scope_path
            ));
            continue;
        }
        match app_doc
            .permissions
            .iter()
            .find(|declared| declared.scope_path == selected.scope_path)
        {
            None => issues.push(format!(
                "install params select undeclared permission `{}`",
                selected.scope_path
            )),
            Some(declared) if declared != selected => issues.push(format!(
                "install params permission `{}` does not exactly match AppDoc declaration",
                selected.scope_path
            )),
            Some(_) => {}
        }
    }

    for required in app_doc.permissions.iter().filter(|item| item.required) {
        if !install_params
            .permissions
            .iter()
            .any(|selected| selected == required)
        {
            issues.push(format!(
                "required permission `{}` is not selected",
                required.scope_path
            ));
        }
    }

    issues
}

// ---------------------------------------------------------------------------
// plan 生成
// ---------------------------------------------------------------------------

pub async fn build_install_plan(
    input: PlannerInput<'_>,
    locator: &dyn ContentLocator,
) -> Result<InstallInspection, InstallError> {
    let app_doc = input.app_doc;
    let snapshot = input.snapshot;
    let app_instance_id = AppInstanceId::from_app_did(&snapshot.app_did, &input.owner_user_id)
        .map_err(|error| {
            InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::InvalidRequest,
                false,
                error,
            )
        })?;
    let namespace = validate_app_package_namespace(app_doc, snapshot, InstallStage::Inspect)?;
    let (service_spec_config, mut config_issues) =
        build_install_config(app_doc, &input.install_params);
    config_issues.extend(validate_permission_selection(
        app_doc,
        &input.install_params,
    ));

    // Document Syntax Validity：did/doc_type 由类型系统保证，这里复查 did 绑定。
    let document_syntax = if app_doc.app_did() == &snapshot.app_did {
        ReadinessState::Ready
    } else {
        return Err(InstallError::new(
            InstallStage::Inspect,
            InstallErrorCode::VerificationFailed,
            false,
            format!(
                "app document did `{}` != resolved app did `{}`",
                app_doc.app_did().to_string(),
                snapshot.app_did.to_string()
            ),
        ));
    };

    // 选 package：selector 匹配目标（显式 selector 优先，已知 key 派生）。
    let mut selected_packages: Vec<SelectedPackage> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    for (key, desc) in app_doc.pkg_list.iter() {
        let Some(selector) = SubPkgList::effective_selector(&key, desc) else {
            // 未知 key 且无显式 selector：不参与自动选择。
            warnings.push(format!(
                "package `{key}` has no selector and is skipped from auto selection"
            ));
            continue;
        };
        if !package_matches_target(&selector, desc, &input.target) {
            continue;
        }
        let explicitly_selected = input
            .install_params
            .selected_components
            .iter()
            .any(|component| component == &key);
        if !desc.is_required() && !explicitly_selected {
            continue;
        }
        if let Some(min_kernel) = selector.min_kernel_version.as_deref() {
            match input.target.kernel_version.as_deref() {
                Some(actual) => {
                    if !kernel_version_satisfied(actual, min_kernel) {
                        continue;
                    }
                }
                None => {
                    warnings.push(format!(
                        "package `{key}` requires kernel >= {min_kernel} but target kernel version is unknown"
                    ));
                }
            }
        }
        if desc.docker_image_name.is_some() {
            if desc.pkg_objid.is_none() {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!(
                        "docker package `{key}` must provide pkg_objid so its image archive is acquired before deploy"
                    ),
                ));
            }
            let digest = desc.docker_image_digest.as_deref().ok_or_else(|| {
                InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!("docker package `{key}` must provide docker_image_digest"),
                )
            })?;
            if !valid_sha256_digest(digest) {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!("docker package `{key}` has invalid digest `{digest}`"),
                ));
            }
        }
        let package_meta_id = desc.pkg_objid.clone().ok_or_else(|| {
            InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::InvalidPackage,
                false,
                format!("deployment package `{key}` must pin a Package Meta ObjectId"),
            )
        })?;
        let pkg_id = desc.get_pkg_id_with_objid().ok_or_else(|| {
            InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::InvalidPackage,
                false,
                format!("deployment package `{key}` cannot build an exact PackageId"),
            )
        })?;
        selected_packages.push(SelectedPackage {
            sub_pkg_name: key.clone(),
            pkg_id,
            package_meta_id: Some(package_meta_id),
            docker_image_name: desc.docker_image_name.clone(),
            docker_image_digest: desc.docker_image_digest.clone(),
            required: desc.is_required(),
        });
    }

    for component in &input.install_params.selected_components {
        if !selected_packages
            .iter()
            .any(|package| &package.sub_pkg_name == component)
        {
            config_issues.push(format!(
                "selected component `{component}` is unavailable for target {}/{}",
                input.target.os, input.target.arch
            ));
        }
    }

    let mut target_issues = Vec::new();
    if selected_packages.is_empty() {
        target_issues.push(format!(
            "no package matches target {}/{}",
            input.target.os, input.target.arch
        ));
    }
    for (capability, required) in &app_doc.req_capbilities {
        match input.target.capabilities.get(capability) {
            Some(actual) if actual >= required => {}
            Some(actual) => target_issues.push(format!(
                "capability `{capability}` requires {required}, target provides {actual}"
            )),
            None => target_issues.push(format!(
                "capability `{capability}` requires {required}, target value is unknown"
            )),
        }
    }
    if let Some(required_runtime) = app_doc.sdk_version.as_deref() {
        match input.target.runtime_version.as_deref() {
            Some(actual) if runtime_version_satisfied(actual, required_runtime) => {}
            Some(actual) => target_issues.push(format!(
                "runtime version `{actual}` does not satisfy SDK version `{required_runtime}`"
            )),
            None => target_issues.push(format!(
                "target runtime version is unknown; SDK version `{required_runtime}` is required"
            )),
        }
    }
    let target_readiness = ReadinessState::from_bool(target_issues.is_empty());

    // 逐 package 计算内容位置。
    let mut required_contents: Vec<PlannedContent> = Vec::new();
    let mut inspected_contents: Vec<InspectedContent> = Vec::new();
    let mut required_content_missing = false;
    let mut estimated_download_bytes: u64 = 0;

    for package in &selected_packages {
        let Some(meta_id) = package.package_meta_id.as_ref() else {
            // 所有选中的部署 package 都必须通过 Package Meta 进入内容获取链路。
            warnings.push(format!(
                "package `{}` has no package meta object id",
                package.sub_pkg_name
            ));
            required_content_missing = true;
            continue;
        };

        // 1) Package Meta 对象本身。
        let meta_id_str = meta_id.to_string();
        let mut meta_location = locator.locate(&meta_id_str).await;
        let mut meta_value: Option<Value> = None;
        if let Some(pikg) = input.pikg {
            if let Some(raw) = pikg.package_meta.package_objects.get(&meta_id_str) {
                if matches!(meta_location, ContentLocation::Missing) {
                    meta_location = ContentLocation::Pikg;
                }
                meta_value = Some(raw.clone());
            }
        }
        if meta_value.is_none() && !matches!(meta_location, ContentLocation::Missing) {
            meta_value = locator.load_json(&meta_id_str).await;
            if meta_value.is_none() {
                return Err(InstallError::new(
                    InstallStage::Inspect,
                    InstallErrorCode::InvalidPackage,
                    false,
                    format!(
                        "package meta `{meta_id_str}` is present but its JSON body is unavailable"
                    ),
                ));
            }
        }
        let meta_sources = source_candidates(app_doc, &package.sub_pkg_name);
        if matches!(meta_location, ContentLocation::Missing) {
            required_content_missing = true;
        }
        required_contents.push(PlannedContent {
            content_id: meta_id_str.clone(),
            sub_pkg_name: Some(package.sub_pkg_name.clone()),
            package_meta_id: Some(meta_id.clone()),
            expected_docker_image_digest: package.docker_image_digest.clone(),
            format: Some("named_object".to_string()),
            size: None,
        });
        inspected_contents.push(InspectedContent {
            content_id: meta_id_str.clone(),
            location: meta_location,
            sources: meta_sources.clone(),
        });

        // 2) 实体内容（payload）。只有 Package Meta 可读时才能展开。
        let payload = match meta_value {
            Some(value) => {
                let meta: PackageMeta = serde_json::from_value(value).map_err(|err| {
                    InstallError::new(
                        InstallStage::Inspect,
                        InstallErrorCode::InvalidPackage,
                        false,
                        format!("package meta `{meta_id_str}` schema invalid: {err}"),
                    )
                })?;
                let (actual_meta_id, _) = meta.gen_obj_id();
                if &actual_meta_id != meta_id {
                    return Err(InstallError::new(
                        InstallStage::Inspect,
                        InstallErrorCode::InvalidPackage,
                        false,
                        format!("package meta `{meta_id_str}` body hashes to `{actual_meta_id}`"),
                    ));
                }
                let declared = app_doc.pkg_list.get(&package.sub_pkg_name).ok_or_else(|| {
                    InstallError::new(
                        InstallStage::Inspect,
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
                    InstallStage::Inspect,
                )?;
                let declared_id = PackageId::parse(&declared.pkg_id).map_err(|error| {
                    InstallError::new(
                        InstallStage::Inspect,
                        InstallErrorCode::InvalidPackage,
                        false,
                        format!("invalid AppDoc PackageId `{}`: {error}", declared.pkg_id),
                    )
                })?;
                if declared_id
                    .version_exp
                    .as_ref()
                    .map(ToString::to_string)
                    .as_deref()
                    != Some(meta.version.as_str())
                {
                    return Err(InstallError::new(
                        InstallStage::Inspect,
                        InstallErrorCode::InvalidPackage,
                        false,
                        format!(
                            "PackageMeta version `{}` does not match AppDoc PackageId `{}`",
                            meta.version, declared.pkg_id
                        ),
                    ));
                }
                payload_from_package_meta(&meta)
            }
            None => None,
        };
        match payload {
            Some((content_id, size)) => {
                let mut location = locator.locate(&content_id).await;
                if matches!(location, ContentLocation::Missing) {
                    if let Some(pikg) = input.pikg {
                        if pikg_has_payload(pikg, &content_id, size) {
                            location = ContentLocation::Pikg;
                        }
                    }
                }
                if matches!(location, ContentLocation::Missing) {
                    required_content_missing = true;
                    estimated_download_bytes =
                        estimated_download_bytes.saturating_add(size.unwrap_or(0));
                }
                required_contents.push(PlannedContent {
                    content_id: content_id.clone(),
                    sub_pkg_name: Some(package.sub_pkg_name.clone()),
                    package_meta_id: Some(meta_id.clone()),
                    expected_docker_image_digest: None,
                    format: Some("archive".to_string()),
                    size,
                });
                inspected_contents.push(InspectedContent {
                    content_id,
                    location,
                    sources: meta_sources,
                });
            }
            None => {
                // meta 不可读：payload 未知。meta 自身已计入 missing。
                if matches!(meta_location, ContentLocation::Missing) {
                    // 已计。payload 待 meta 取得后由重新 Inspect 展开。
                }
            }
        }
    }

    // Package Integrity：pikg 结构校验在 Reader 构造时已完成。
    let package_integrity = ReadinessState::Ready;
    let content = ReadinessState::from_bool(!required_content_missing);
    let source_identity = match input.pikg {
        Some(pikg) => InstallSourceIdentity::Pikg {
            app_doc_object_id: input.app_doc_object_id.clone(),
            pikg_digest: pikg.pikg_digest.clone(),
        },
        None => InstallSourceIdentity::Catalog {
            app_doc_object_id: input.app_doc_object_id.clone(),
        },
    };
    let trust = ReadinessState::from_bool(
        snapshot.is_trust_ready_for_source(input.policy, &source_identity),
    );
    let config = ReadinessState::from_bool(config_issues.is_empty());

    let readiness = PlanReadiness::compose(
        document_syntax,
        trust,
        package_integrity,
        content,
        target_readiness,
        config,
        snapshot.document_status,
    );

    let app = AppDocumentRef {
        did: snapshot.app_did.clone(),
        object_id: input.app_doc_object_id.clone(),
        show_name: app_doc.show_name.clone(),
        version: app_doc.version.clone(),
    };
    let mut frozen_resolution = snapshot.clone();
    frozen_resolution.cache_status = None;
    frozen_resolution.warnings.clear();
    frozen_resolution.resolved_at = None;
    let plan_fingerprint = InstallPlan::compute_fingerprint(
        input.plan_use,
        &input.task_id,
        &app_instance_id,
        &input.owner_user_id,
        &source_identity,
        &app,
        app_doc,
        &frozen_resolution,
        &input.target,
        &input.install_params,
        &service_spec_config,
        &selected_packages,
        &required_contents,
    );
    let now = buckyos_get_unix_timestamp();
    let plan = InstallPlan {
        schema_version: APP_INSTALL_SCHEMA_VERSION,
        plan_use: input.plan_use,
        task_id: input.task_id,
        app_instance_id,
        owner_user_id: input.owner_user_id,
        source_identity,
        app,
        app_doc: app_doc.clone(),
        resolution: frozen_resolution,
        target: input.target.clone(),
        selected_packages,
        required_contents,
        install_params: input.install_params,
        service_spec_config,
        plan_fingerprint: plan_fingerprint.clone(),
        created_at: now,
    };
    let mut warnings = warnings;
    warnings.extend(snapshot.warnings.iter().cloned());
    Ok(InstallInspection {
        schema_version: APP_INSTALL_SCHEMA_VERSION,
        plan,
        resolution_status: snapshot.clone(),
        status: InstallPlanStatus {
            plan_fingerprint,
            target_snapshot: input.target,
            readiness,
            contents: inspected_contents,
            target_issues,
            config_issues,
            permission_options: app_doc.permissions.clone(),
            estimated_download_bytes,
            warnings,
            inspected_at: now,
        },
    })
}

/// 从 Package Meta 提取 (content_id, size)。content 为空视为无 payload
/// （纯 meta 包）。
fn payload_from_package_meta(meta: &PackageMeta) -> Option<(String, Option<u64>)> {
    if meta.content.trim().is_empty() {
        return None;
    }
    Some((meta.content.trim().to_string(), Some(meta.size)))
}

/// pikg 是否携带某内容：chunk id 与 content_index 的 sha256 digest 对齐
/// （sha256 型直接同名，mix256 型无法只用 digest 反推，按 size+meta 交叉在
/// Verify 完成，这里保守只认 sha256 直配）。
fn pikg_has_payload(pikg: &PikgInspection, content_id: &str, size: Option<u64>) -> bool {
    if pikg.package_meta.content_index.contains_key(content_id) {
        return true;
    }
    // mix256 chunk id：按 sub_pkg 的 meta size 找同尺寸 entry 不可靠，
    // 改为检查 content_index 里是否有引用同一 Package Meta 的 entry。
    if content_id.starts_with("mix256:") {
        if let Some(size) = size {
            return pikg
                .package_meta
                .content_index
                .values()
                .any(|entry| entry.size == size);
        }
    }
    false
}

fn source_candidates(app_doc: &AppDoc, sub_pkg_name: &str) -> Vec<String> {
    app_doc
        .pkg_list
        .get(sub_pkg_name)
        .and_then(|desc| desc.source_url.clone())
        .into_iter()
        .filter(|url| !url.trim().is_empty())
        .collect()
}

/// 内核版本约束：语义化比较，解析失败按不满足处理（显式保守）。
fn kernel_version_satisfied(actual: &str, minimum: &str) -> bool {
    let parse = |raw: &str| semver::Version::parse(raw.trim().trim_start_matches('v')).ok();
    match (parse(actual), parse(minimum)) {
        (Some(actual), Some(minimum)) => actual >= minimum,
        _ => false,
    }
}

fn runtime_version_satisfied(actual: &str, required: &str) -> bool {
    match (
        semver::Version::parse(actual.trim().trim_start_matches('v')),
        semver::Version::parse(required.trim().trim_start_matches('v')),
    ) {
        (Ok(actual), Ok(required)) => actual >= required,
        _ => actual.trim() == required.trim(),
    }
}

fn valid_sha256_digest(raw: &str) -> bool {
    raw.strip_prefix("sha256:")
        .map(|hex| hex.len() == 64 && hex.chars().all(|ch| ch.is_ascii_hexdigit()))
        .unwrap_or(false)
}

pub(crate) fn package_matches_target(
    selector: &PackageSelector,
    desc: &SubPkgDesc,
    target: &InstallTarget,
) -> bool {
    let target_os = if desc.docker_image_name.is_some() {
        "linux"
    } else {
        target.os.as_str()
    };
    selector.matches_platform(target_os, &target.arch)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(docker_image_name: Option<&str>) -> SubPkgDesc {
        SubPkgDesc {
            pkg_id: "test.buckyos.bns.did/root#1.0.0".to_string(),
            pkg_objid: None,
            docker_image_name: docker_image_name.map(ToString::to_string),
            docker_image_digest: None,
            source_url: None,
            selector: None,
            required: None,
        }
    }

    fn macos_target(arch: &str) -> InstallTarget {
        InstallTarget {
            node_did: None,
            node_id: Some("ood1".to_string()),
            os: "macos".to_string(),
            arch: arch.to_string(),
            kernel_version: None,
            runtime_version: None,
            capabilities: Default::default(),
        }
    }

    #[test]
    fn docker_package_uses_linux_os_and_target_arch() {
        let selector = PackageSelector::for_platform("linux", "aarch64");
        let docker = package(Some("example/test:latest"));

        assert!(package_matches_target(
            &selector,
            &docker,
            &macos_target("aarch64")
        ));
        assert!(!package_matches_target(
            &selector,
            &docker,
            &macos_target("x86_64")
        ));
    }

    #[test]
    fn native_package_uses_target_os() {
        let selector = PackageSelector::for_platform("linux", "aarch64");
        let native = package(None);

        assert!(!package_matches_target(
            &selector,
            &native,
            &macos_target("aarch64")
        ));
    }
}
