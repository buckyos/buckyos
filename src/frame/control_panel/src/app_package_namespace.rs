use buckyos_api::{
    AppDoc, AppId, DidResolutionSnapshot, InstallError, InstallErrorCode, InstallStage,
};
use package_lib::PackageId;
use serde_json::json;

pub(crate) const APP_PACKAGE_NAMESPACE_MISMATCH: &str = "APP_PACKAGE_NAMESPACE_MISMATCH";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AppPackageNamespace {
    value: String,
}

impl AppPackageNamespace {
    pub(crate) fn as_str(&self) -> &str {
        self.value.as_str()
    }
}

pub(crate) fn validate_app_package_namespace(
    app_doc: &AppDoc,
    snapshot: &DidResolutionSnapshot,
    stage: InstallStage,
) -> Result<AppPackageNamespace, InstallError> {
    if app_doc.app_did() != &snapshot.app_did {
        return Err(namespace_mismatch(
            stage,
            format!(
                "app document did `{}` != resolved app did `{}`",
                app_doc.app_did().to_string(),
                snapshot.app_did.to_string()
            ),
        ));
    }

    let namespace = AppId::from_app_did(&snapshot.app_did)
        .map_err(|error| namespace_mismatch(stage, error))?
        .to_string();

    for (sub_pkg_name, desc) in app_doc.pkg_list.iter() {
        validate_sub_pkg_id(&namespace, &sub_pkg_name, &desc.pkg_id, stage)?;
    }

    Ok(AppPackageNamespace { value: namespace })
}

pub(crate) fn validate_package_meta_namespace(
    namespace: &AppPackageNamespace,
    sub_pkg_name: &str,
    pkg_id: &str,
    package_meta_name: &str,
    stage: InstallStage,
) -> Result<(), InstallError> {
    let pkg_unique_name = validate_sub_pkg_id(namespace.as_str(), sub_pkg_name, pkg_id, stage)?;
    let meta_unique_name = parse_package_name(package_meta_name, stage, sub_pkg_name)?;
    if meta_unique_name != pkg_unique_name {
        return Err(namespace_mismatch(
            stage,
            format!(
                "subpackage `{sub_pkg_name}` Package Meta name `{package_meta_name}` resolves to `{meta_unique_name}`, expected `{pkg_unique_name}` from pkg_id `{pkg_id}`"
            ),
        ));
    }
    Ok(())
}

fn validate_sub_pkg_id(
    namespace: &str,
    sub_pkg_name: &str,
    pkg_id: &str,
    stage: InstallStage,
) -> Result<String, InstallError> {
    let package_id = PackageId::parse(pkg_id).map_err(|error| {
        namespace_mismatch(
            stage,
            format!("subpackage `{sub_pkg_name}` has invalid pkg_id `{pkg_id}`: {error}"),
        )
    })?;
    let unique_name = strip_package_env_qualifier(&package_id.name);
    let valid = unique_name == namespace
        || unique_name
            .strip_suffix(&format!(".{namespace}"))
            .is_some_and(is_safe_sub_package_name);
    if !valid {
        return Err(namespace_mismatch(
            stage,
            format!(
                "subpackage `{sub_pkg_name}` pkg_id `{pkg_id}` occupies `{unique_name}`, outside namespace `{namespace}`"
            ),
        ));
    }
    Ok(unique_name.to_string())
}

fn parse_package_name<'a>(
    package_name: &'a str,
    stage: InstallStage,
    sub_pkg_name: &str,
) -> Result<&'a str, InstallError> {
    let unique_name = strip_package_env_qualifier(package_name);
    if unique_name.is_empty() {
        return Err(namespace_mismatch(
            stage,
            format!("subpackage `{sub_pkg_name}` has an empty package name"),
        ));
    }
    Ok(unique_name)
}

fn strip_package_env_qualifier(package_name: &str) -> &str {
    package_name
        .split_once('.')
        .filter(|(qualifier, _)| is_recognized_package_env(qualifier))
        .map(|(_, rest)| rest)
        .unwrap_or(package_name)
}

fn is_safe_sub_package_name(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('.')
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn is_recognized_package_env(value: &str) -> bool {
    matches!(
        value,
        "all"
            | "nightly-linux-amd64"
            | "nightly-linux-aarch64"
            | "nightly-windows-amd64"
            | "nightly-windows-aarch64"
            | "nightly-apple-amd64"
            | "nightly-apple-aarch64"
    )
}

fn namespace_mismatch(stage: InstallStage, message: String) -> InstallError {
    InstallError::new(
        stage,
        InstallErrorCode::InvalidPackage,
        false,
        format!("{APP_PACKAGE_NAMESPACE_MISMATCH}: {message}"),
    )
    .with_details(json!({
        "reason_code": APP_PACKAGE_NAMESPACE_MISMATCH,
    }))
}

#[cfg(all(test, any()))]
mod tests {
    use super::*;
    use buckyos_api::{
        AppDocType, AppType, DidEvidenceLevel, DidVerificationStatus, DocumentStatus, SubPkgDesc,
    };
    use name_lib::DID;

    fn test_app_doc(pkg_id: &str) -> AppDoc {
        let owner = DID::new("bns", "user1");
        AppDoc::builder(AppType::Web, "app1", "1.0.0", "user1", &owner)
            .web_pkg(SubPkgDesc::new(pkg_id))
            .build()
            .unwrap()
    }

    fn snapshot(app_doc: &AppDoc) -> DidResolutionSnapshot {
        DidResolutionSnapshot {
            app_did: app_doc.app_did().clone(),
            doc_type: AppDocType,
            app_doc_object_id: None,
            resolver_id: Some("test".to_string()),
            document_status: DocumentStatus::Active,
            document_version: Some(1),
            authority_seq: None,
            effective_owner: Some(DID::new("bns", "user1")),
            expected_owner: Some(DID::new("bns", "user1")),
            evidence: Some(DidEvidenceLevel::Anchored),
            verification_status: Some(DidVerificationStatus::Passed),
            cache_status: None,
            doc_hash: None,
            warnings: vec![],
            migration_target: None,
            resolved_at: Some(1),
        }
    }

    fn assert_namespace_mismatch(result: Result<AppPackageNamespace, InstallError>) {
        let error = result.unwrap_err();
        assert_eq!(error.code, InstallErrorCode::InvalidPackage);
        assert!(!error.retryable);
        assert!(error.message.contains(APP_PACKAGE_NAMESPACE_MISMATCH));
        assert_eq!(
            error
                .details
                .as_ref()
                .and_then(|details| details.get("reason_code"))
                .and_then(|value| value.as_str()),
            Some(APP_PACKAGE_NAMESPACE_MISMATCH)
        );
    }

    #[test]
    fn accepts_exact_suffix_and_recognized_env_namespace() {
        for pkg_id in [
            "user1_app1#1.0.0",
            "user1_app1-web#1.0.0",
            "nightly-linux-amd64.user1_app1-web#1.0.0",
            "all.user1_app1-agent#1.0.0",
        ] {
            let app_doc = test_app_doc(pkg_id);
            let namespace = validate_app_package_namespace(
                &app_doc,
                &snapshot(&app_doc),
                InstallStage::Inspect,
            )
            .unwrap();
            assert_eq!(namespace.as_str(), "user1_app1");
        }
    }

    #[test]
    fn rejects_unbounded_prefix_unsafe_name_and_custom_env() {
        for pkg_id in [
            "user1_app10#1.0.0",
            "user1_other-app#1.0.0",
            "../user1_app1-web#1.0.0",
            "e2e.user1_app1-web#1.0.0",
            "nightly-linux-amd64.other.user1_app1-web#1.0.0",
        ] {
            let app_doc = test_app_doc(pkg_id);
            assert_namespace_mismatch(validate_app_package_namespace(
                &app_doc,
                &snapshot(&app_doc),
                InstallStage::Inspect,
            ));
        }
    }

    #[test]
    fn rejects_hidden_off_target_package() {
        let mut app_doc = test_app_doc("user1_app1-web#1.0.0");
        app_doc.pkg_list.aarch64_docker_image = Some(SubPkgDesc::new("control-panel#1.0.0"));
        assert_namespace_mismatch(validate_app_package_namespace(
            &app_doc,
            &snapshot(&app_doc),
            InstallStage::Inspect,
        ));
    }

    #[test]
    fn rejects_name_owner_and_nonstandard_did_mismatch() {
        let app_doc = test_app_doc("user1_app1-web#1.0.0");

        let mut wrong_name_value = serde_json::to_value(&app_doc).unwrap();
        wrong_name_value["name"] = json!("app2");
        let wrong_name: AppDoc = serde_json::from_value(wrong_name_value).unwrap();
        assert_namespace_mismatch(validate_app_package_namespace(
            &wrong_name,
            &snapshot(&app_doc),
            InstallStage::Inspect,
        ));

        let mut wrong_owner = snapshot(&app_doc);
        wrong_owner.expected_owner = Some(DID::new("bns", "attacker"));
        assert_namespace_mismatch(validate_app_package_namespace(
            &app_doc,
            &wrong_owner,
            InstallStage::Inspect,
        ));

        let mut nonstandard = snapshot(&app_doc);
        nonstandard.app_did = DID::new("bns", "app1.team.user1");
        let mut nonstandard_doc_value = serde_json::to_value(&app_doc).unwrap();
        nonstandard_doc_value["did"] = json!(nonstandard.app_did.to_string());
        let nonstandard_doc: AppDoc = serde_json::from_value(nonstandard_doc_value).unwrap();
        assert_namespace_mismatch(validate_app_package_namespace(
            &nonstandard_doc,
            &nonstandard,
            InstallStage::Inspect,
        ));
    }

    #[test]
    fn package_meta_must_resolve_to_same_unique_name() {
        let app_doc = test_app_doc("user1_app1-web#1.0.0");
        let namespace =
            validate_app_package_namespace(&app_doc, &snapshot(&app_doc), InstallStage::Inspect)
                .unwrap();

        validate_package_meta_namespace(
            &namespace,
            "web",
            "user1_app1-web#1.0.0",
            "nightly-linux-amd64.user1_app1-web",
            InstallStage::Verify,
        )
        .unwrap();

        let error = validate_package_meta_namespace(
            &namespace,
            "web",
            "user1_app1-web#1.0.0",
            "nightly-linux-amd64.user1_app1-agent",
            InstallStage::Verify,
        )
        .unwrap_err();
        assert_eq!(error.code, InstallErrorCode::InvalidPackage);
        assert!(error.message.contains(APP_PACKAGE_NAMESPACE_MISMATCH));
    }
}
