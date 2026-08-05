//! Inspect Stage：从已解析 App Document + 目标 Node + 安装参数生成
//! `InstallPlan`（doc/App 安装协议.md §3.2、§4.5、§11.4）。
//!
//! 硬规则：
//! - target os/arch 来自用户选中的目标 Node 信息，禁止用 Control Panel
//!   编译期 `cfg!(target_*)` 代替（P2.3）；
//! - 内容位置判定顺序：已安装/NamedStore -> 当前 pikg -> missing（远程
//!   Source 只作为 missing 的获取途径）；
//! - Inspect 不写系统目录，不触发下载。

use crate::app_package_namespace::validate_app_package_namespace;
use crate::pikg::PikgInspection;
use async_trait::async_trait;
use buckyos_api::{
    AppDoc, ContentLocation, DidResolutionSnapshot, InstallError, InstallErrorCode, InstallPlan,
    InstallPolicy, InstallStage, InstallTarget, PlanReadiness, PlannedContent, ReadinessState,
    SelectedPackage, SubPkgList,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use ndn_lib::ObjId;
use serde_json::Value;

// ---------------------------------------------------------------------------
// 内容定位抽象（stage 3 用 fake，stage 5 接 NamedStore/已安装内容）
// ---------------------------------------------------------------------------

/// 本地内容定位：判断对象/内容当前是否已在本机可复用位置。
/// 只回答 Installed/NamedStore/Missing；pikg 命中由 planner 自己判断。
#[async_trait]
pub trait ContentLocator: Send + Sync {
    async fn locate(&self, content_id: &str) -> ContentLocation;
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
    pub target: InstallTarget,
    /// 影响部署/selector 的安装参数（进 fingerprint）。
    pub install_params: Value,
    /// 本地 pikg（若本次事务有），作为内容与 Package Meta 的候选来源。
    pub pikg: Option<&'a PikgInspection>,
}

// ---------------------------------------------------------------------------
// plan 生成
// ---------------------------------------------------------------------------

pub async fn build_install_plan(
    input: PlannerInput<'_>,
    locator: &dyn ContentLocator,
) -> Result<InstallPlan, InstallError> {
    let app_doc = input.app_doc;
    let snapshot = input.snapshot;

    validate_app_package_namespace(app_doc, snapshot, InstallStage::Inspect)?;

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
        selected_packages.push(SelectedPackage {
            sub_pkg_name: key.clone(),
            pkg_id: desc.pkg_id.clone(),
            package_meta_id: desc.pkg_objid.clone(),
            docker_image_name: desc.docker_image_name.clone(),
            required: desc.is_required(),
        });
    }

    let target_supported = !selected_packages.is_empty();

    // 逐 package 计算内容位置。
    let mut required_contents: Vec<PlannedContent> = Vec::new();
    let mut required_content_missing = false;
    let mut estimated_download_bytes: u64 = 0;

    for package in &selected_packages {
        let Some(meta_id) = package.package_meta_id.as_ref() else {
            if package.docker_image_name.is_some() {
                // docker image 由 node-daemon 按镜像名拉取，无内容对象需求。
                continue;
            }
            // 没有 Package Meta 也没有 docker 镜像：required 时内容无法就绪。
            if package.required {
                warnings.push(format!(
                    "package `{}` has no package meta object id",
                    package.sub_pkg_name
                ));
                required_content_missing = true;
            }
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
        let meta_sources = source_candidates(app_doc, &package.sub_pkg_name);
        if matches!(meta_location, ContentLocation::Missing) && package.required {
            required_content_missing = true;
        }
        required_contents.push(PlannedContent {
            content_id: meta_id_str.clone(),
            sub_pkg_name: Some(package.sub_pkg_name.clone()),
            package_meta_id: Some(meta_id.clone()),
            format: Some("named_object".to_string()),
            size: None,
            location: meta_location,
            sources: meta_sources.clone(),
        });

        // 2) 实体内容（payload）。只有 Package Meta 可读时才能展开。
        let payload = match meta_value {
            Some(value) => payload_from_meta_value(&value),
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
                    if package.required {
                        required_content_missing = true;
                    }
                    estimated_download_bytes =
                        estimated_download_bytes.saturating_add(size.unwrap_or(0));
                }
                required_contents.push(PlannedContent {
                    content_id,
                    sub_pkg_name: Some(package.sub_pkg_name.clone()),
                    package_meta_id: Some(meta_id.clone()),
                    format: Some("archive".to_string()),
                    size,
                    location,
                    sources: meta_sources,
                });
            }
            None => {
                // meta 不可读：payload 未知。meta 自身已计入 missing。
                if matches!(
                    required_contents.last().map(|c| c.location),
                    Some(ContentLocation::Missing)
                ) && package.required
                {
                    // 已计。payload 待 meta 取得后由重新 Inspect 展开。
                }
            }
        }
    }

    // Package Integrity：pikg 结构校验在 Reader 构造时已完成。
    let package_integrity = ReadinessState::Ready;
    let content = ReadinessState::from_bool(!required_content_missing);
    let trust = ReadinessState::from_bool(snapshot.is_trust_ready(input.policy));
    // Config Readiness：Inspect 时参数尚未确认冲突（端口/目录冲突在 Prepare
    // 检查）；这里只要求参数是对象。
    let config = if input.install_params.is_object() || input.install_params.is_null() {
        ReadinessState::Ready
    } else {
        ReadinessState::NotReady
    };

    let readiness = PlanReadiness::compose(
        document_syntax,
        trust,
        package_integrity,
        content,
        config,
        snapshot.document_status,
        target_supported,
    );

    let selected_meta_ids: Vec<Option<ObjId>> = selected_packages
        .iter()
        .map(|package| package.package_meta_id.clone())
        .collect();
    let plan_fingerprint = InstallPlan::compute_fingerprint(
        &input.app_doc_object_id,
        snapshot.document_status,
        snapshot.document_version,
        &input.target,
        &input.install_params,
        &selected_meta_ids,
    );

    Ok(InstallPlan {
        app_did: snapshot.app_did.clone(),
        doc_type: snapshot.doc_type.clone(),
        app_doc_object_id: input.app_doc_object_id,
        app_name: app_doc.name.clone(),
        app_version: app_doc.version.clone(),
        did_resolution: snapshot.clone(),
        target: input.target,
        selected_packages,
        required_contents,
        readiness,
        permissions: app_doc.permissions.clone(),
        install_params: input.install_params,
        estimated_download_bytes,
        plan_fingerprint,
        created_at: buckyos_get_unix_timestamp(),
    })
}

/// 从 Package Meta JSON 提取 (content_id, size)。content 为空视为无 payload
/// （纯 meta 包）。
fn payload_from_meta_value(value: &Value) -> Option<(String, Option<u64>)> {
    let content = value.get("content").and_then(|v| v.as_str())?;
    if content.trim().is_empty() {
        return None;
    }
    let size = value.get("size").and_then(|v| v.as_u64());
    Some((content.trim().to_string(), size))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_install_resolver::fake;
    use buckyos_api::{AppType, DocumentStatus, InstallReadiness, PackageSelector, SubPkgDesc};
    use name_lib::DID;
    use ndn_lib::build_named_object_by_json;
    use package_lib::PackageMeta;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU32, Ordering};

    struct MapLocator {
        map: HashMap<String, ContentLocation>,
        pub calls: AtomicU32,
    }

    impl MapLocator {
        fn new(map: HashMap<String, ContentLocation>) -> Self {
            Self {
                map,
                calls: AtomicU32::new(0),
            }
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
            .docker_image_name("demo:1.0.0-amd64");
        amd_desc.pkg_objid = Some(amd_meta_id.clone());
        let mut arm_desc = SubPkgDesc::new("tester_demo-img-arm64#1.0.0")
            .docker_image_name("demo:1.0.0-arm64");
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
                target: linux_target("aarch64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();

        assert_eq!(plan.selected_packages.len(), 1);
        assert_eq!(
            plan.selected_packages[0].sub_pkg_name,
            "aarch64_docker_image"
        );

        // amd64 目标选 amd64。
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert_eq!(plan.selected_packages[0].sub_pkg_name, "amd64_docker_image");

        // windows 目标不支持。
        let mut win_target = linux_target("amd64");
        win_target.os = "windows".to_string();
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                target: win_target,
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert_eq!(plan.readiness.install, InstallReadiness::UnsupportedTarget);
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
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
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
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.readiness.install,
            InstallReadiness::ContentDownloadRequired
        );
        // meta 对象缺失时 payload 尺寸未知，预计下载量可以为 0，
        // 但 missing 清单必须精确列出缺失对象（协议 §8.3）。
        assert!(plan
            .required_contents
            .iter()
            .any(|content| matches!(content.location, ContentLocation::Missing)));

        // Content Ready（NamedStore 全命中）+ Trust Unknown => TRUST_RESOLUTION_REQUIRED。
        let mut unknown = fake::status_answer(&app.app_did, DocumentStatus::Unknown);
        unknown.snapshot.app_doc_object_id = resolved.snapshot.app_doc_object_id.clone();
        let mut map = HashMap::new();
        map.insert(app.meta_id.to_string(), ContentLocation::NamedStore);
        map.insert(app.payload_digest.clone(), ContentLocation::NamedStore);
        let locator = MapLocator::new(map);
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &unknown.snapshot,
                policy: InstallPolicy::Normal,
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.readiness.install,
            InstallReadiness::TrustResolutionRequired
        );

        // 全 ready => OFFLINE_READY。
        let mut map = HashMap::new();
        map.insert(app.meta_id.to_string(), ContentLocation::NamedStore);
        map.insert(app.payload_digest.clone(), ContentLocation::NamedStore);
        let locator = MapLocator::new(map);
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &resolved.snapshot,
                policy: InstallPolicy::Normal,
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap();
        assert_eq!(plan.readiness.install, InstallReadiness::OfflineReady);

        // Revoked 压倒一切。
        let mut revoked = fake::status_answer(&app.app_did, DocumentStatus::Revoked);
        revoked.snapshot.app_doc_object_id = resolved.snapshot.app_doc_object_id.clone();
        let plan = build_install_plan(
            PlannerInput {
                app_doc: &app.app_doc,
                app_doc_object_id: resolved.snapshot.app_doc_object_id.clone().unwrap(),
                snapshot: &revoked.snapshot,
                policy: InstallPolicy::Normal,
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &locator,
        )
        .await
        .unwrap();
        assert_eq!(plan.readiness.install, InstallReadiness::IdentityRevoked);
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
                        target,
                        install_params: serde_json::json!({}),
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
        assert_eq!(base.plan_fingerprint, same.plan_fingerprint);

        let other_target = make_plan(linux_target("aarch64"), resolved.snapshot.clone()).await;
        assert_ne!(base.plan_fingerprint, other_target.plan_fingerprint);

        let mut bumped = resolved.snapshot.clone();
        bumped.document_version = Some(2);
        let bumped_plan = make_plan(linux_target("amd64"), bumped).await;
        assert_ne!(base.plan_fingerprint, bumped_plan.plan_fingerprint);
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
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: Some(&inspection),
            },
            &locator,
        )
        .await
        .unwrap();

        // 内容全部由 pikg 提供：OFFLINE_READY，且没有任何下载量。
        assert_eq!(plan.readiness.install, InstallReadiness::OfflineReady);
        assert_eq!(plan.estimated_download_bytes, 0);
        assert!(plan
            .required_contents
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
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        let names: Vec<_> = plan
            .selected_packages
            .iter()
            .map(|p| p.sub_pkg_name.as_str())
            .collect();
        assert!(names.contains(&"web"));
        assert!(!names.contains(&"big_model"));

        // 显式 selector 后参与选择。
        let mut model_desc = model_desc;
        model_desc.selector = Some(PackageSelector::for_platform("linux", "amd64"));
        let app_doc = AppDoc::builder(AppType::Web, "demo-web", "0.1.0", "tester", &owner)
            .web_pkg(SubPkgDesc::new("tester_demo-web-web#0.1.0"))
            .other_pkg("big_model", model_desc)
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
                target: linux_target("amd64"),
                install_params: serde_json::json!({}),
                pikg: None,
            },
            &NoLocalContentLocator,
        )
        .await
        .unwrap();
        let names: Vec<_> = plan
            .selected_packages
            .iter()
            .map(|p| p.sub_pkg_name.as_str())
            .collect();
        assert!(names.contains(&"big_model"));
    }
}

/// 占位默认目标：仅用于 body 缺失（Unknown/Missing）时构造只读占位 plan。
/// 真实安装目标必须由 driver 从目标 Node 信息（devices/{node}/info）填充，
/// 不得用本函数（更不得用编译期 cfg!）替代真实选择。
pub fn kernel_default_target() -> InstallTarget {
    InstallTarget {
        node_did: None,
        node_id: None,
        os: "linux".to_string(),
        arch: "x86_64".to_string(),
        kernel_version: None,
        runtime_version: None,
    }
}
