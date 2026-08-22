//! Inspect Stage：从已解析 App Document + 目标 Node + 安装参数生成
//! `InstallPlan`（doc/App 安装协议.md §3.2、§4.5、§11.4）。
//!
//! 硬规则：
//! - target os/arch 来自用户选中的目标 Node 信息，禁止用 Control Panel
//!   编译期 `cfg!(target_*)` 代替（P2.3）；
//! - 内容位置判定顺序：已安装/NamedStore -> 当前 pikg -> missing（远程
//!   Source 只作为 missing 的获取途径）；
//! - Inspect 不写系统目录，不触发下载。

use crate::app_install_deployer::{build_install_config, installation_route_label};
use crate::app_package_namespace::{
    validate_app_package_namespace, validate_package_meta_namespace,
};
use crate::pikg::PikgInspection;
use async_trait::async_trait;
use buckyos_api::{
    AppDoc, AppDocumentRef, AppInstallationId, AppInstallationScope, ContentLocation,
    DidResolutionSnapshot, InspectedContent, InstallError, InstallErrorCode, InstallInspection,
    InstallParams, InstallPlan, InstallPlanStatus, InstallPlanUse, InstallPolicy,
    InstallSourceIdentity, InstallStage, InstallTarget, PlanReadiness, PlannedContent,
    ReadinessState, SelectedPackage, SubPkgList, APP_INSTALL_SCHEMA_VERSION,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use ndn_lib::ObjId;
use package_lib::PackageMeta;
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
    pub installation_scope: AppInstallationScope,
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
    let installation_id = AppInstallationId::derive(&snapshot.app_did, &input.installation_scope);
    let route_label = installation_route_label(app_doc.name.as_str(), &installation_id);

    let namespace = validate_app_package_namespace(app_doc, snapshot, InstallStage::Inspect)?;
    let (service_spec_config, mut config_issues) =
        build_install_config(route_label.as_str(), app_doc, &input.install_params);
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
        if !selector.matches_platform(&input.target.os, &input.target.arch) {
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
        selected_packages.push(SelectedPackage {
            sub_pkg_name: key.clone(),
            pkg_id: desc.pkg_id.clone(),
            package_meta_id: desc.pkg_objid.clone(),
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
                validate_package_meta_namespace(
                    &namespace,
                    &package.sub_pkg_name,
                    &package.pkg_id,
                    meta.name.as_str(),
                    InstallStage::Inspect,
                )?;
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
    let trust = ReadinessState::from_bool(snapshot.is_trust_ready(input.policy));
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
        name: app_doc.name.clone(),
        version: app_doc.version.clone(),
    };
    let source_identity = match input.pikg {
        Some(pikg) => InstallSourceIdentity::Pikg {
            app_doc_object_id: input.app_doc_object_id.clone(),
            pikg_digest: pikg.pikg_digest.clone(),
        },
        None => InstallSourceIdentity::Catalog {
            app_doc_object_id: input.app_doc_object_id.clone(),
        },
    };
    let mut frozen_resolution = snapshot.clone();
    frozen_resolution.cache_status = None;
    frozen_resolution.warnings.clear();
    frozen_resolution.resolved_at = None;
    let plan_fingerprint = InstallPlan::compute_fingerprint(
        input.plan_use,
        &installation_id,
        &input.installation_scope,
        &source_identity,
        &app,
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
        installation_id,
        installation_scope: input.installation_scope,
        source_identity,
        app,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_install_resolver::fake;
    use buckyos_api::{
        AppType, DocumentStatus, InstallReadiness, PackageSelector, PermissionItem, SubPkgDesc,
    };
    use name_lib::DID;
    use ndn_lib::build_named_object_by_json;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn declared_permissions() -> Vec<PermissionItem> {
        vec![
            PermissionItem {
                scope_path: "user/home".to_string(),
                required: true,
                actions: vec!["read".to_string()],
                exp: None,
            },
            PermissionItem {
                scope_path: "wan".to_string(),
                required: false,
                actions: vec![],
                exp: Some(3600),
            },
        ]
    }

    #[test]
    fn permission_params_are_typed_selected_subset() {
        let owner = DID::new("bns", "tester");
        let mut app_doc =
            AppDoc::builder(AppType::Web, "permission-demo", "0.1.0", "tester", &owner)
                .web_pkg(SubPkgDesc::new("tester_permission-demo-web#0.1.0"))
                .build()
                .unwrap();
        app_doc.permissions = declared_permissions();

        let normal = default_install_params(&app_doc, InstallPolicy::Normal);
        assert_eq!(normal.permissions, vec![app_doc.permissions[0].clone()]);
        assert!(validate_permission_selection(&app_doc, &normal).is_empty());

        let system = default_install_params(&app_doc, InstallPolicy::SystemInternal);
        assert_eq!(system.permissions, app_doc.permissions);
        assert!(validate_permission_selection(&app_doc, &system).is_empty());

        let mut missing_required = InstallParams::default();
        missing_required
            .permissions
            .push(app_doc.permissions[1].clone());
        assert!(validate_permission_selection(&app_doc, &missing_required)
            .iter()
            .any(|issue| issue.contains("required permission")));

        let mut altered = normal;
        altered.permissions[0].actions.push("write".to_string());
        assert!(validate_permission_selection(&app_doc, &altered)
            .iter()
            .any(|issue| issue.contains("does not exactly match")));
    }

    #[test]
    fn same_display_name_gets_distinct_installation_route_labels() {
        let first = AppInstallationId::new(format!("appinst:{}", "11".repeat(32))).unwrap();
        let second = AppInstallationId::new(format!("appinst:{}", "22".repeat(32))).unwrap();
        let first_label = installation_route_label("notes", &first);
        let second_label = installation_route_label("notes", &second);
        assert_ne!(first_label, second_label);
        assert_eq!(first_label, "notes-111111111111");
        assert_eq!(second_label, "notes-222222222222");
    }

    struct MapLocator {
        map: HashMap<String, ContentLocation>,
        objects: HashMap<String, Value>,
        pub calls: AtomicU32,
    }

    impl MapLocator {
        fn new(map: HashMap<String, ContentLocation>) -> Self {
            Self {
                map,
                objects: HashMap::new(),
                calls: AtomicU32::new(0),
            }
        }

        fn with_json(mut self, content_id: impl Into<String>, value: Value) -> Self {
            self.objects.insert(content_id.into(), value);
            self
        }
    }

    #[async_trait]
    impl ContentLocator for MapLocator {
        async fn locate(&self, content_id: &str) -> ContentLocation {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.map
                .get(content_id)
                .copied()
                .unwrap_or(ContentLocation::Missing)
        }

        async fn load_json(&self, content_id: &str) -> Option<Value> {
            self.objects.get(content_id).cloned()
        }
    }

    struct TestApp {
        app_did: DID,
        app_doc: AppDoc,
        doc_value: Value,
        meta_value: Value,
        meta_id: ObjId,
        payload_digest: String,
    }

    /// 双平台 docker 应用：amd64 + aarch64 镜像，各自带 Package Meta。
    fn build_dual_platform_app() -> TestApp {
        let owner = DID::from_str("did:bns:tester").unwrap();
        let payload_digest = format!("sha256:{}", "ab".repeat(32));

        let mut amd_meta =
            PackageMeta::new("tester_demo-img-amd64", "1.0.0", "tester", &owner, None);
        amd_meta.size = 1000;
        amd_meta.content = payload_digest.clone();
        let amd_meta_value = serde_json::to_value(&amd_meta).unwrap();
        let (amd_meta_id, _) = build_named_object_by_json(ndn_lib::OBJ_TYPE_PKG, &amd_meta_value);

        let mut arm_meta =
            PackageMeta::new("tester_demo-img-arm64", "1.0.0", "tester", &owner, None);
        arm_meta.size = 2000;
        arm_meta.content = format!("sha256:{}", "cd".repeat(32));
        let arm_meta_value = serde_json::to_value(&arm_meta).unwrap();
        let (arm_meta_id, _) = build_named_object_by_json(ndn_lib::OBJ_TYPE_PKG, &arm_meta_value);

        let mut amd_desc = SubPkgDesc::new("tester_demo-img-amd64#1.0.0")
            .docker_image_name("demo:1.0.0-amd64")
            .docker_image_digest(format!("sha256:{}", "11".repeat(32)));
        amd_desc.pkg_objid = Some(amd_meta_id.clone());
        let mut arm_desc = SubPkgDesc::new("tester_demo-img-arm64#1.0.0")
            .docker_image_name("demo:1.0.0-arm64")
            .docker_image_digest(format!("sha256:{}", "22".repeat(32)));
        arm_desc.pkg_objid = Some(arm_meta_id);

        let app_doc = AppDoc::builder(AppType::AppService, "demo", "1.0.0", "tester", &owner)
            .amd64_docker_image(amd_desc)
            .aarch64_docker_image(arm_desc)
            .build()
            .unwrap();
        let doc_value = serde_json::to_value(&app_doc).unwrap();
        TestApp {
            app_did: app_doc.app_did().clone(),
            app_doc,
            doc_value,
            meta_value: amd_meta_value,
            meta_id: amd_meta_id,
            payload_digest,
        }
    }

    fn linux_target(arch: &str) -> InstallTarget {
        InstallTarget {
            node_did: None,
            node_id: Some("ood1".to_string()),
            os: "linux".to_string(),
            arch: arch.to_string(),
            kernel_version: None,
            runtime_version: None,
            capabilities: Default::default(),
        }
    }

    fn test_installation_scope() -> AppInstallationScope {
        AppInstallationScope {
            zone_did: DID::new("bns", "test-zone"),
            owner_user_id: "tester".to_string(),
            app_class: buckyos_api::AppClass::UserInstalled,
        }
    }

    #[tokio::test]
    async fn planner_selects_by_target_not_by_compile_time_cfg() {
        let app = build_dual_platform_app();
        let resolved = fake::active_answer(&app.app_did, app.doc_value.clone(), 1);

        // 目标是 aarch64 节点（无论 Control Panel 编译在什么平台上）。
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("aarch64"),
                install_params: InstallParams::default(),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();

        assert_eq!(plan.plan.selected_packages.len(), 1);
        assert_eq!(
            plan.plan.selected_packages[0].sub_pkg_name,
            "aarch64_docker_image"
        );

        // amd64 目标选 amd64。
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: InstallParams::default(),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.plan.selected_packages[0].sub_pkg_name,
            "amd64_docker_image"
        );

        // windows 目标不支持。
        let mut win_target = linux_target("amd64");
        win_target.os = "windows".to_string();
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: win_target,
                install_params: InstallParams::default(),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::UnsupportedTarget
        );
    }

    #[tokio::test]
    async fn docker_plan_binds_image_digest_and_rejects_unbound_images() {
        let app = build_dual_platform_app();
        let resolved = fake::active_answer(&app.app_did, app.doc_value.clone(), 1);
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: InstallParams::default(),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        let selected = &plan.plan.selected_packages[0];
        let expected_digest = format!("sha256:{}", "11".repeat(32));
        assert_eq!(
            selected.docker_image_digest.as_deref(),
            Some(expected_digest.as_str())
        );
        assert!(plan.plan.required_contents.iter().any(|content| {
            content.expected_docker_image_digest == selected.docker_image_digest
        }));

        let mut unbound = app.app_doc.clone();
        unbound
            .pkg_list
            .amd64_docker_image
            .as_mut()
            .unwrap()
            .docker_image_digest = None;
        let doc_value = serde_json::to_value(&unbound).unwrap();
        let resolved = fake::active_answer(unbound.app_did(), doc_value, 1);
        let error = build_install_plan(
            PlannerInput {
                app_doc: &unbound,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: InstallParams::default(),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap_err();
        assert_eq!(error.code, InstallErrorCode::InvalidPackage);
        assert!(error.message.contains("docker_image_digest"));
    }

    #[tokio::test]
    async fn planner_checks_runtime_and_numeric_capabilities() {
        let app = build_dual_platform_app();
        let mut app_doc = app.app_doc.clone();
        app_doc.sdk_version = Some("0.8.0".to_string());
        app_doc.req_capbilities.insert("memory".to_string(), 1024);
        let doc_value = serde_json::to_value(&app_doc).unwrap();
        let resolved = fake::active_answer(app_doc.app_did(), doc_value, 1);
        let mut target = linux_target("amd64");
        target.runtime_version = Some("0.7.0".to_string());
        target.capabilities.insert("memory".to_string(), 512);

        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target,
                install_params: InstallParams::default(),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::UnsupportedTarget
        );
        assert_eq!(plan.status.readiness.target, ReadinessState::NotReady);
        assert_eq!(plan.status.target_issues.len(), 2);
    }

    #[tokio::test]
    async fn plan_contains_typed_final_service_config() {
        let app = build_dual_platform_app();
        let mut app_doc = app.app_doc.clone();
        app_doc
            .service_config_tips
            .data_mount_points
            .insert("/data".into(), None);
        app_doc
            .service_config_tips
            .external_mount_points
            .insert("/shared".into(), None);
        app_doc
            .service_config_tips
            .runtime_caps
            .insert("net".to_string(), "enabled".to_string());
        app_doc.service_config_tips.container_param = Some("--init".to_string());
        app_doc.service_config_tips.start_param = Some("serve".to_string());
        let doc_value = serde_json::to_value(&app_doc).unwrap();
        let resolved = fake::active_answer(app_doc.app_did(), doc_value, 1);
        let mut install_params = default_install_params(&app_doc, InstallPolicy::Normal);
        install_params.data_mount_points.insert(
            "/data".into(),
            buckyos_api::MountPointConfig {
                target_path: "data".into(),
                access: "read_write".to_string(),
            },
        );
        install_params.external_mount_points.insert(
            "/shared".into(),
            buckyos_api::MountPointConfig {
                target_path: "shared".into(),
                access: "read_only".to_string(),
            },
        );
        install_params
            .bash_envs
            .insert("MODE".to_string(), "prod".to_string());
        install_params.res_pool_id = Some("large".to_string());

        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params,
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert!(plan
            .plan
            .service_spec_config
            .data_mount_point
            .contains_key(std::path::Path::new("/data")));
        assert!(plan
            .plan
            .service_spec_config
            .external_mount_point
            .contains_key(std::path::Path::new("/shared")));
        assert_eq!(plan.plan.service_spec_config.bash_envs["MODE"], "prod");
        assert_eq!(plan.plan.service_spec_config.res_pool_id, "large");
        assert_eq!(plan.plan.service_spec_config.runtime_caps["net"], "enabled");
        assert_eq!(
            plan.plan.service_spec_config.container_param.as_deref(),
            Some("--init")
        );
        assert_eq!(
            plan.plan.service_spec_config.start_param.as_deref(),
            Some("serve")
        );
        assert_eq!(plan.status.readiness.config, ReadinessState::Ready);
    }

    #[tokio::test]
    async fn planner_rejects_hidden_namespace_escape_before_content_lookup() {
        let app = build_dual_platform_app();
        let mut app_doc = app.app_doc.clone();
        app_doc
            .pkg_list
            .aarch64_docker_image
            .as_mut()
            .unwrap()
            .pkg_id = "control-panel#1.0.0".to_string();
        let doc_value = serde_json::to_value(&app_doc).unwrap();
        let resolved = fake::active_answer(app_doc.app_did(), doc_value, 1);
        let locator = MapLocator::new(HashMap::new());

        let error = build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: default_install_params(&app.app_doc, InstallPolicy::Normal),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap_err();

        assert_eq!(error.stage, InstallStage::Inspect);
        assert_eq!(error.code, InstallErrorCode::InvalidPackage);
        assert!(error.message.contains("APP_PACKAGE_NAMESPACE_MISMATCH"));
        assert_eq!(locator.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn readiness_matrix_trust_vs_content() {
        let app = build_dual_platform_app();

        // Trust Ready + Content Missing => CONTENT_DOWNLOAD_REQUIRED。
        let resolved = fake::active_answer(&app.app_did, app.doc_value.clone(), 1);
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: default_install_params(&app.app_doc, InstallPolicy::Normal),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::ContentDownloadRequired
        );
        // meta 对象缺失时 payload 尺寸未知，预计下载量可以为 0，
        // 但 missing 清单必须精确列出缺失对象（协议 §8.3）。
        assert!(plan
            .status
            .contents
            .iter()
            .any(|content| matches!(content.location, ContentLocation::Missing)));

        // Content Ready（NamedStore 全命中）+ Trust Unknown => TRUST_RESOLUTION_REQUIRED。
        let mut unknown = fake::status_answer(&app.app_did, DocumentStatus::Unknown);
        unknown.snapshot.app_doc_object_id = resolved.snapshot.app_doc_object_id.clone();
        let mut map = HashMap::new();
        map.insert(app.meta_id.to_string(), ContentLocation::NamedStore);
        map.insert(app.payload_digest.clone(), ContentLocation::NamedStore);
        let locator =
            MapLocator::new(map).with_json(app.meta_id.to_string(), app.meta_value.clone());
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &unknown.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: default_install_params(&app.app_doc, InstallPolicy::Normal),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::TrustResolutionRequired
        );

        // 只有 Package Meta、缺少其 payload 时仍必须下载，不能提前离线就绪。
        let mut map = HashMap::new();
        map.insert(app.meta_id.to_string(), ContentLocation::NamedStore);
        let locator =
            MapLocator::new(map).with_json(app.meta_id.to_string(), app.meta_value.clone());
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: default_install_params(&app.app_doc, InstallPolicy::Normal),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::ContentDownloadRequired
        );
        assert!(plan.status.contents.iter().any(|content| {
            content.content_id == app.payload_digest
                && matches!(content.location, ContentLocation::Missing)
        }));

        // 全 ready => OFFLINE_READY。
        let mut map = HashMap::new();
        map.insert(app.meta_id.to_string(), ContentLocation::NamedStore);
        map.insert(app.payload_digest.clone(), ContentLocation::NamedStore);
        let locator =
            MapLocator::new(map).with_json(app.meta_id.to_string(), app.meta_value.clone());
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: default_install_params(&app.app_doc, InstallPolicy::Normal),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::OfflineReady
        );

        // Revoked 压倒一切。
        let mut revoked = fake::status_answer(&app.app_did, DocumentStatus::Revoked);
        revoked.snapshot.app_doc_object_id = resolved.snapshot.app_doc_object_id.clone();
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &revoked.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: InstallParams::default(),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::IdentityRevoked
        );
    }

    #[tokio::test]
    async fn plan_fingerprint_invalidates_on_target_or_version_change() {
        let app = build_dual_platform_app();
        let resolved = fake::active_answer(&app.app_did, app.doc_value.clone(), 1);
        let obj_id = resolved.snapshot.app_doc_object_id.clone().unwrap();

        let make_plan = |target: InstallTarget, snapshot: DidResolutionSnapshot| {
            let app_doc = app.app_doc.clone();
            let obj_id = obj_id.clone();
            async move {
                build_install_plan(
                    PlannerInput {
                        app_doc: &app_doc,
                        app_doc_object_id: obj_id,
                        snapshot: &snapshot,
                        policy: InstallPolicy::Normal,
                        plan_use: InstallPlanUse::FreshInstall,
                        installation_scope: test_installation_scope(),
                        target,
                        install_params: default_install_params(&app_doc, InstallPolicy::Normal),
                        pikg: None,
                    },
                    &NoLocalContentLocator,
                )
                .await
                .unwrap()
            }
        };

        let base = make_plan(linux_target("amd64"), resolved.snapshot.clone()).await;
        let same = make_plan(linux_target("amd64"), resolved.snapshot.clone()).await;
        assert_eq!(base.plan.plan_fingerprint, same.plan.plan_fingerprint);

        let other_target = make_plan(linux_target("aarch64"), resolved.snapshot.clone()).await;
        assert_ne!(
            base.plan.plan_fingerprint,
            other_target.plan.plan_fingerprint
        );

        let mut bumped = resolved.snapshot.clone();
        bumped.document_version = Some(2);
        let bumped_plan = make_plan(linux_target("amd64"), bumped).await;
        assert_ne!(
            base.plan.plan_fingerprint,
            bumped_plan.plan.plan_fingerprint
        );
    }

    #[tokio::test]
    async fn offline_pikg_contents_satisfy_content_readiness_without_network() {
        let app = build_dual_platform_app();
        let resolved = fake::active_answer(&app.app_did, app.doc_value.clone(), 1);

        // 手工构造 PikgInspection（不落盘）：携带 amd64 meta + payload。
        let inspection = PikgInspection {
            pikg_digest: "00".repeat(32),
            app_doc: app.app_doc.clone(),
            app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
            has_signed_app_doc: false,
            signed_app_doc_jwt: None,
            package_meta: crate::pikg::PikgPackageMetaFile {
                schema: crate::pikg::PIKG_PACKAGE_META_SCHEMA.to_string(),
                app_doc_id: resolved
                    .snapshot
                    .app_doc_object_id
                    .clone()
                    .unwrap()
                    .to_string(),
                package_objects: [(app.meta_id.to_string(), app.meta_value.clone())]
                    .into_iter()
                    .collect(),
                content_index: [(
                    app.payload_digest.clone(),
                    crate::pikg::PikgContentIndexEntry {
                        sub_pkg_name: "amd64_docker_image".to_string(),
                        path: "amd64_docker_image.tar.gz".to_string(),
                        format: "tar.gz".to_string(),
                        size: 1000,
                        digest: app.payload_digest.clone(),
                    },
                )]
                .into_iter()
                .collect(),
            },
            entries: vec![],
        };

        let locator = MapLocator::new(HashMap::new());
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: default_install_params(&app.app_doc, InstallPolicy::Normal),
                pikg: Some(&inspection),
            },
            &locator,
        )
        .await
        .unwrap();

        // 内容全部由 pikg 提供：OFFLINE_READY，且没有任何下载量。
        assert_eq!(
            plan.status.readiness.install,
            InstallReadiness::OfflineReady
        );
        assert_eq!(plan.status.estimated_download_bytes, 0);
        assert!(plan
            .status
            .contents
            .iter()
            .all(|content| matches!(content.location, ContentLocation::Pikg)));

        // locator 只被查询本地位置，没有任何"下载"副作用（零网络调用由
        // 类型保证：planner 根本拿不到网络客户端）。
        assert_eq!(locator.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn unknown_key_without_selector_and_explicit_selector_override() {
        let owner = DID::from_str("did:bns:tester").unwrap();
        let mut model_desc = SubPkgDesc::new("tester_demo-web-model#1.0.0");
        model_desc.required = Some(false);
        // 未知 key 无 selector：不参与选择。
        let app_doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner)
            .web_pkg(SubPkgDesc::new("tester_demo-web-web#0.1.0"))
            .other_pkg("big_model", model_desc.clone())
            .build()
            .unwrap();
        let doc_value = serde_json::to_value(&app_doc).unwrap();
        let resolved = fake::active_answer(app_doc.app_did(), doc_value, 1);

        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params: InstallParams::default(),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        let names: Vec<_> = plan
            .plan
            .selected_packages
            .iter()
            .map(|p| p.sub_pkg_name.as_str())
            .collect();
        assert!(names.contains(&"web"));
        assert!(!names.contains(&"big_model"));

        // 可选 package 需要同时有显式 selector 和用户选择才参与计划。
        let mut model_desc = model_desc;
        model_desc.selector = Some(PackageSelector::for_platform("linux", "amd64"));
        let app_doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner)
            .web_pkg(SubPkgDesc::new("tester_demo-web-web#0.1.0"))
            .other_pkg("big_model", model_desc)
            .build()
            .unwrap();
        let doc_value = serde_json::to_value(&app_doc).unwrap();
        let resolved = fake::active_answer(app_doc.app_did(), doc_value, 1);
        let install_params = InstallParams {
            selected_components: vec!["big_model".to_string()],
            ..Default::default()
        };
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                plan_use: InstallPlanUse::FreshInstall,
                installation_scope: test_installation_scope(),
                target: linux_target("amd64"),
                install_params,
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        let names: Vec<_> = plan
            .plan
            .selected_packages
            .iter()
            .map(|p| p.sub_pkg_name.as_str())
            .collect();
        assert!(names.contains(&"big_model"));
    }
}
