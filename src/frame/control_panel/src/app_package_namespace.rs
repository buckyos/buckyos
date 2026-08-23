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
    package_meta_version: &str,
    stage: InstallStage,
) -> Result<(), InstallError> {
    validate_sub_pkg_id(namespace.as_str(), sub_pkg_name, pkg_id, stage)?;
    let package_id = PackageId::parse(pkg_id).map_err(|error| {
        namespace_mismatch(
            stage,
            format!("subpackage `{sub_pkg_name}` has invalid pkg_id `{pkg_id}`: {error}"),
        )
    })?;
    if package_meta_name != package_id.name {
        return Err(namespace_mismatch(
            stage,
            format!(
                "subpackage `{sub_pkg_name}` Package Meta name `{package_meta_name}` does not exactly match `{}` from pkg_id `{pkg_id}`",
                package_id.name
            ),
        ));
    }
    let expected_version = package_id
        .version_exp
        .as_ref()
        .map(ToString::to_string)
        .ok_or_else(|| {
            namespace_mismatch(
                stage,
                format!("subpackage `{sub_pkg_name}` pkg_id `{pkg_id}` has no version"),
            )
        })?;
    if package_meta_version != expected_version {
        return Err(namespace_mismatch(
            stage,
            format!(
                "subpackage `{sub_pkg_name}` Package Meta version `{package_meta_version}` does not exactly match `{expected_version}` from pkg_id `{pkg_id}`"
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

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AppDocType, AppType, DidEvidenceLevel, DidVerificationStatus, DocumentStatus, SubPkgDesc,
    };
    use name_lib::DID;
    use ndn_lib::ObjId;

    fn test_app_doc(pkg_id: &str) -> AppDoc {
        let owner = DID::from_str("did:web:publisher.example").unwrap();
        AppDoc::builder(
            AppType::Web,
            "Notes",
            "1.0.0",
            "did:web:publisher.example",
            &owner,
        )
        .app_did(DID::from_str("did:web:notes.example.com").unwrap())
        .web_pkg(
            SubPkgDesc::new(pkg_id)
                .package_meta_object_id(ObjId::new_by_raw("pkg".to_string(), vec![7; 32])),
        )
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
            effective_owner: Some(DID::from_str("did:web:publisher.example").unwrap()),
            expected_owner: Some(DID::from_str("did:web:publisher.example").unwrap()),
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
    fn accepts_root_subpackage_and_recognized_environment_namespace() {
        for pkg_id in [
            "notes.example.com#1.0.0",
            "web.notes.example.com#1.0.0",
            "nightly-linux-amd64.web.notes.example.com#1.0.0",
            "all.agent.notes.example.com#1.0.0",
        ] {
            let app_doc = test_app_doc(pkg_id);
            let namespace = validate_app_package_namespace(
                &app_doc,
                &snapshot(&app_doc),
                InstallStage::Inspect,
            )
            .unwrap();
            assert_eq!(namespace.as_str(), "notes.example.com");
        }
    }

    #[test]
    fn rejects_outside_nested_unsafe_and_custom_environment_namespaces() {
        for pkg_id in [
            "notes.example.org#1.0.0",
            "nested.web.notes.example.com#1.0.0",
            "../notes.example.com#1.0.0",
            "e2e.web.notes.example.com#1.0.0",
            "nightly-linux-amd64.nested.web.notes.example.com#1.0.0",
        ] {
            let mut app_doc = test_app_doc("web.notes.example.com#1.0.0");
            app_doc.pkg_list.web.as_mut().unwrap().pkg_id = pkg_id.to_string();
            assert_namespace_mismatch(validate_app_package_namespace(
                &app_doc,
                &snapshot(&app_doc),
                InstallStage::Inspect,
            ));
        }
    }

    #[test]
    fn rejects_hidden_off_target_package() {
        let mut app_doc = test_app_doc("web.notes.example.com#1.0.0");
        app_doc.pkg_list.aarch64_docker_image = Some(SubPkgDesc::new("control-panel#1.0.0"));
        assert_namespace_mismatch(validate_app_package_namespace(
            &app_doc,
            &snapshot(&app_doc),
            InstallStage::Inspect,
        ));
    }

    #[test]
    fn rejects_resolved_did_mismatch() {
        let app_doc = test_app_doc("web.notes.example.com#1.0.0");
        let mut mismatched = snapshot(&app_doc);
        mismatched.app_did = DID::from_str("did:web:other.example.com").unwrap();
        assert_namespace_mismatch(validate_app_package_namespace(
            &app_doc,
            &mismatched,
            InstallStage::Inspect,
        ));
    }

    #[test]
    fn package_meta_name_must_exactly_match_pkg_id_name() {
        let app_doc = test_app_doc("nightly-linux-amd64.web.notes.example.com#1.0.0");
        let namespace =
            validate_app_package_namespace(&app_doc, &snapshot(&app_doc), InstallStage::Inspect)
                .unwrap();

        validate_package_meta_namespace(
            &namespace,
            "web",
            "nightly-linux-amd64.web.notes.example.com#1.0.0",
            "nightly-linux-amd64.web.notes.example.com",
            "1.0.0",
            InstallStage::Verify,
        )
        .unwrap();

        let error = validate_package_meta_namespace(
            &namespace,
            "web",
            "nightly-linux-amd64.web.notes.example.com#1.0.0",
            "all.web.notes.example.com",
            "1.0.0",
            InstallStage::Verify,
        )
        .unwrap_err();
        assert_eq!(error.code, InstallErrorCode::InvalidPackage);
        assert!(error.message.contains(APP_PACKAGE_NAMESPACE_MISMATCH));

        assert!(validate_package_meta_namespace(
            &namespace,
            "web",
            "nightly-linux-amd64.web.notes.example.com#1.0.0",
            "nightly-linux-amd64.web.notes.example.com",
            "1.0.1",
            InstallStage::Verify,
        )
        .is_err());
    }
}
