use crate::app_loader::{
    command_matches_agent_process, command_matches_exact_agent_process,
    container_list_contains_name, docker_desc_requires_exact_match,
    docker_image_tar_candidates_for_arch, docker_missing_text, docker_runtime_matches_deployment,
    docker_runtime_matches_target, normalize_digest, parse_docker_container_inspect,
    resolve_aios_image_repo_from_paths, AppLoader, CommandSpec, ControlOperation,
    DockerRuntimeIdentity, PlatformArch, PlatformOs, PlatformTarget, RuntimeType,
    DOCKER_LABEL_APP_DOC_OBJECT_ID, DOCKER_LABEL_IMAGE_DIGEST, DOCKER_LABEL_PKG_ID,
    DOCKER_LABEL_PKG_OBJID, DOCKER_LABEL_SPEC_GENERATION,
};
use crate::run_item::ControlRuntItemErrors;
use buckyos_api::{
    AppDoc, AppId, AppInstanceId, AppServiceInstanceConfig, AppServiceSpec, AppType,
    DeploymentIdentity, DeploymentPackage, LocalAppInstanceConfig, ServiceEndpointConfig,
    ServiceInstanceState, ServiceSpecConfig, ServiceState, SubPkgDesc, OBJ_TYPE_APP_DOC,
};
use name_lib::DID;
use ndn_lib::ObjId;
use package_lib::PackageId;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

fn assert_programs(commands: &[CommandSpec], expected: &[&str]) {
    let actual = commands
        .iter()
        .map(|command| command.program.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
}

fn build_appservice_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    let app_did = DID::from_str("did:web:demo.example").unwrap();
    AppDoc::builder(AppType::AppService, "demo", "0.1.0", "did:bns:test", &owner)
        .app_did(app_did)
        .amd64_docker_image(
            SubPkgDesc::new("image.demo.example#0.1.0")
                .package_meta_object_id(test_package_object_id(1))
                .docker_image_name("demo/service:0.1.0-amd64")
                .docker_image_digest("sha256:deadbeef"),
        )
        .aarch64_docker_image(
            SubPkgDesc::new("image-arm.demo.example#0.1.0")
                .package_meta_object_id(test_package_object_id(2))
                .docker_image_name("demo/service:0.1.0-aarch64")
                .docker_image_digest("sha256:beadfeed"),
        )
        .service_port("www", 80)
        .build()
        .unwrap()
}

fn build_agent_doc_without_category() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    let app_did = DID::from_str("did:web:jarvis-runtime.example").unwrap();
    AppDoc::builder(
        AppType::Agent,
        "jarvis-runtime",
        "0.1.0",
        "did:bns:test",
        &owner,
    )
    .app_did(app_did)
    .agent_pkg(
        SubPkgDesc::new("agent.jarvis-runtime.example#0.1.0")
            .package_meta_object_id(test_package_object_id(3)),
    )
    .agent_skills_pkg(
        SubPkgDesc::new("skills.jarvis-runtime.example#0.1.0")
            .package_meta_object_id(test_package_object_id(4)),
    )
    .service_port("main", 4060)
    .build()
    .unwrap()
}

fn build_web_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    let app_did = DID::from_str("did:web:portal.example").unwrap();
    AppDoc::builder(AppType::Web, "portal", "0.1.0", "did:bns:test", &owner)
        .app_did(app_did)
        .web_pkg(
            SubPkgDesc::new("web.portal.example#0.1.0")
                .package_meta_object_id(test_package_object_id(5)),
        )
        .build()
        .unwrap()
}

fn build_local_service_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    let app_did = DID::from_str("did:web:desktop-tool.example").unwrap();
    let mut doc = AppDoc::builder(
        AppType::Service,
        "desktop-tool",
        "0.1.0",
        "did:bns:test",
        &owner,
    )
    .app_did(app_did)
    .build()
    .unwrap();

    doc.pkg_list.aarch64_linux_app = Some(test_package_desc("linux-arm", 6));
    doc.pkg_list.amd64_linux_app = Some(test_package_desc("linux-amd", 7));
    doc.pkg_list.aarch64_apple_app = Some(test_package_desc("macos-arm", 8));
    doc.pkg_list.amd64_apple_app = Some(test_package_desc("macos-amd", 9));
    doc.pkg_list.aarch64_win_app = Some(test_package_desc("win-arm", 10));
    doc.pkg_list.amd64_win_app = Some(test_package_desc("win-amd", 11));

    doc
}

fn build_script_service_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    let app_did = DID::from_str("did:web:systest.example").unwrap();
    AppDoc::builder(AppType::Service, "systest", "0.1.0", "did:bns:test", &owner)
        .app_did(app_did)
        .script_pkg(
            SubPkgDesc::new("script.systest.example#0.1.0")
                .package_meta_object_id(test_package_object_id(12)),
        )
        .service_port("www", 3000)
        .build()
        .unwrap()
}

fn test_package_object_id(seed: u8) -> ObjId {
    ObjId::new_by_raw("pkg".to_string(), vec![seed; 32])
}

fn test_package_desc(name: &str, seed: u8) -> SubPkgDesc {
    SubPkgDesc::new(format!("{name}.desktop-tool.example#0.1.0"))
        .package_meta_object_id(test_package_object_id(seed))
}

fn test_exact_package_id(name: &str, seed: u8) -> String {
    PackageId::get_pkgid_with_objid(&format!("{name}#0.1.0"), Some(test_package_object_id(seed)))
        .unwrap()
}

fn test_runtime_key(app_id: &str) -> String {
    AppInstanceId::new(AppId::parse(app_id).unwrap(), "alice")
        .unwrap()
        .runtime_key()
}

fn test_container_name(app_host_name: &str) -> String {
    format!("buckyos-app-{app_host_name}")
}

fn build_service_loader(
    app_doc: AppDoc,
    service_ports_config: HashMap<String, u16>,
    platform: PlatformTarget,
    support_container: bool,
) -> AppLoader {
    let install_config = build_spec_config(&app_doc);
    let mut app_spec = build_test_app_spec(app_doc, install_config);
    app_spec
        .packages
        .retain(|package| package_matches_platform(&package.sub_pkg_name, platform));
    let app_instance_id = app_spec.app_instance_id.to_string();
    let mut config = AppServiceInstanceConfig::new("ood1", &app_spec).unwrap();
    config.service_ports_config = service_ports_config;
    AppLoader::new_for_service(&app_instance_id, config)
        .with_platform(platform)
        .with_container_support_override(support_container)
}

fn package_matches_platform(key: &str, platform: PlatformTarget) -> bool {
    match key {
        "amd64_docker_image" | "amd64_linux_app" => {
            platform.os == PlatformOs::Linux && platform.arch == PlatformArch::Amd64
        }
        "aarch64_docker_image" | "aarch64_linux_app" => {
            platform.os == PlatformOs::Linux && platform.arch == PlatformArch::Aarch64
        }
        "amd64_apple_app" => {
            platform.os == PlatformOs::Macos && platform.arch == PlatformArch::Amd64
        }
        "aarch64_apple_app" => {
            platform.os == PlatformOs::Macos && platform.arch == PlatformArch::Aarch64
        }
        "amd64_win_app" => {
            platform.os == PlatformOs::Windows && platform.arch == PlatformArch::Amd64
        }
        "aarch64_win_app" => {
            platform.os == PlatformOs::Windows && platform.arch == PlatformArch::Aarch64
        }
        _ => true,
    }
}

fn build_test_app_spec(app_doc: AppDoc, spec_config: ServiceSpecConfig) -> AppServiceSpec {
    let app_instance_id = AppInstanceId::from_app_did(app_doc.app_did(), "alice").unwrap();
    let app_doc_value = serde_json::to_value(&app_doc).unwrap();
    let (app_doc_object_id, _) =
        ndn_lib::build_named_object_by_json(OBJ_TYPE_APP_DOC, &app_doc_value);
    let packages = app_doc
        .pkg_list
        .iter()
        .into_iter()
        .map(|(sub_pkg_name, desc)| {
            let package_meta_object_id = desc.pkg_objid.clone().unwrap();
            DeploymentPackage {
                sub_pkg_name: sub_pkg_name.to_string(),
                pkg_id: PackageId::get_pkgid_with_objid(
                    &desc.pkg_id,
                    Some(package_meta_object_id.clone()),
                )
                .unwrap(),
                package_meta_object_id,
                docker_image_name: desc.docker_image_name.clone(),
                docker_image_digest: desc.docker_image_digest.clone(),
            }
        })
        .collect();
    AppServiceSpec {
        app_instance_id: app_instance_id.clone(),
        app_did: app_doc.app_did().clone(),
        deployment: DeploymentIdentity {
            app_instance_id,
            task_id: "test:install".to_string(),
            app_doc_object_id,
            spec_generation: 1,
            pikg_digest: None,
        },
        app_name: "test-app".to_string(),
        app_host_name: "test-app".to_string(),
        owner_user_id: "alice".to_string(),
        permission: app_doc.permissions.clone(),
        packages,
        app_doc,
        app_index: 1,
        selected_components: Vec::new(),
        enable: true,
        expected_instance_count: 1,
        state: ServiceState::Running,
        spec_config,
    }
}

fn build_spec_config(app_doc: &AppDoc) -> ServiceSpecConfig {
    let mut install_config = ServiceSpecConfig::default();
    for (service_name, endpoint) in &app_doc.service_config_tips.service_endpoints {
        install_config.service_config.insert(
            service_name.clone(),
            ServiceEndpointConfig {
                protocol: endpoint.protocol,
                inner_port: endpoint.inner_port,
            },
        );
    }
    install_config
}

fn build_agent_loader(platform: PlatformTarget) -> AppLoader {
    let app_doc = build_agent_doc_without_category();
    let install_config = build_spec_config(&app_doc);
    let app_spec = build_test_app_spec(app_doc, install_config);
    let app_instance_id = app_spec.app_instance_id.to_string();
    let mut config = AppServiceInstanceConfig::new("ood1", &app_spec).unwrap();
    config.service_ports_config =
        HashMap::from([("www".to_string(), 10080), ("main".to_string(), 14060)]);
    AppLoader::new_for_service(&app_instance_id, config)
        .with_platform(platform)
        .with_container_support_override(true)
        .with_worker_image_repo_override("paios/aios")
}

fn unique_temp_path(prefix: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
}

#[test]
fn helper_functions_keep_expected_normalization() {
    let amd64_candidates = docker_image_tar_candidates_for_arch("demo", PlatformArch::Amd64);
    assert_eq!(
        amd64_candidates,
        vec![
            "demo.tar",
            "amd64_docker_image.tar",
            "aarch64_docker_image.tar"
        ]
    );
    let aarch64_candidates = docker_image_tar_candidates_for_arch("demo", PlatformArch::Aarch64);
    assert_eq!(
        aarch64_candidates,
        vec![
            "demo.tar",
            "aarch64_docker_image.tar",
            "amd64_docker_image.tar"
        ]
    );

    assert_eq!(
        normalize_digest(Some("repo/image:tag@sha256:abc")),
        Some("sha256:abc")
    );
    assert_eq!(normalize_digest(Some("sha256:def")), Some("sha256:def"));
    assert_eq!(normalize_digest(Some("   ")), None);
    assert_eq!(normalize_digest(None), None);
}

#[test]
fn resolve_aios_image_repo_from_paths_reads_devenv_override() {
    let temp_dir = unique_temp_path("node-daemon-devenv");
    fs::create_dir_all(&temp_dir).unwrap();
    let devenv_path = temp_dir.join("devenv.json");
    fs::write(&devenv_path, r#"{"aios":"paios/aios_dev"}"#).unwrap();

    let resolved =
        resolve_aios_image_repo_from_paths([temp_dir.join("missing.json"), devenv_path.clone()]);

    assert_eq!(resolved.as_deref(), Some("paios/aios_dev"));

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn resolve_aios_image_repo_from_paths_ignores_missing_or_empty_values() {
    let temp_dir = unique_temp_path("node-daemon-devenv-empty");
    fs::create_dir_all(&temp_dir).unwrap();
    let devenv_path = temp_dir.join("devenv.json");
    fs::write(&devenv_path, "{\"aios\":\"   \"}").unwrap();

    let resolved =
        resolve_aios_image_repo_from_paths([temp_dir.join("missing.json"), devenv_path.clone()]);

    assert_eq!(resolved, None);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn docker_missing_text_matches_lowercase_runtime_errors() {
    assert!(docker_missing_text("error: no such object: 732418f568ce"));
    assert!(docker_missing_text(
        "Error response from daemon: No such container: demo"
    ));
    assert!(docker_missing_text("no such image: repo/demo:latest"));
    assert!(docker_missing_text(
        "Error response from daemon: get buckyos-exttool: no such volume"
    ));
    assert!(!docker_missing_text(
        "permission denied while trying to connect to docker daemon"
    ));
}

#[test]
fn parse_docker_container_inspect_extracts_state_labels_and_image() {
    let inspect = parse_docker_container_inspect(
        r#"{
            "State": {"Running": true},
            "Config": {
                "Labels": {
                    "buckyos.runtime_key": "alice-demo",
                    "buckyos.pkg_objid": "pkg:1234567890"
                }
            },
            "Image": "sha256:deadbeef"
        }"#,
    )
    .unwrap();

    assert!(inspect.state.running);
    assert_eq!(inspect.image.as_deref(), Some("sha256:deadbeef"));
    assert_eq!(
        inspect
            .config
            .labels
            .as_ref()
            .and_then(|labels| labels.get("buckyos.runtime_key"))
            .map(String::as_str),
        Some("alice-demo")
    );
}

#[test]
fn container_list_contains_name_only_matches_exact_container_name() {
    let names = "devtest-buckyos_filebrowser\nfoo-devtest-buckyos_filebrowser-old\n";
    assert!(container_list_contains_name(
        names,
        "devtest-buckyos_filebrowser"
    ));
    assert!(!container_list_contains_name(names, "buckyos_filebrowser"));
}

#[test]
fn docker_runtime_exact_match_uses_pkg_objid_and_digest() {
    let exact_pkg_id = test_exact_package_id("image.demo.example", 1);
    let mut desc = SubPkgDesc::new(exact_pkg_id.clone())
        .docker_image_name("demo/service:0.1.0-amd64")
        .docker_image_digest("demo/service@sha256:deadbeef");
    desc.pkg_objid = Some(test_package_object_id(1));

    assert!(docker_desc_requires_exact_match(&desc));
    assert!(docker_runtime_matches_target(
        &DockerRuntimeIdentity {
            image_id: Some("sha256:anotherhash".to_string()),
            repo_digests: vec!["demo/service@sha256:deadbeef".to_string()],
            labels: HashMap::from([
                (DOCKER_LABEL_PKG_ID.to_string(), exact_pkg_id.clone(),),
                (
                    DOCKER_LABEL_PKG_OBJID.to_string(),
                    test_package_object_id(1).to_string(),
                ),
                (
                    DOCKER_LABEL_IMAGE_DIGEST.to_string(),
                    "sha256:deadbeef".to_string(),
                ),
            ]),
        },
        &desc,
    ));
    assert!(!docker_runtime_matches_target(
        &DockerRuntimeIdentity {
            image_id: Some("sha256:deadbeef".to_string()),
            repo_digests: vec!["demo/service@sha256:deadbeef".to_string()],
            labels: HashMap::from([
                (DOCKER_LABEL_PKG_ID.to_string(), exact_pkg_id.clone(),),
                (
                    DOCKER_LABEL_PKG_OBJID.to_string(),
                    "pkg:oldversion".to_string(),
                ),
            ]),
        },
        &desc,
    ));
    assert!(docker_runtime_matches_target(
        &DockerRuntimeIdentity {
            image_id: Some("sha256:deadbeef".to_string()),
            labels: HashMap::from([(DOCKER_LABEL_PKG_ID.to_string(), exact_pkg_id.clone(),)]),
            ..Default::default()
        },
        &SubPkgDesc::new(exact_pkg_id)
            .docker_image_name("demo/service:0.1.0-amd64")
            .docker_image_digest("sha256:deadbeef"),
    ));
}

#[test]
fn docker_runtime_exact_match_rejects_another_pkg_version_without_objid() {
    let desc = SubPkgDesc::new("demo-img#0.2.0");
    assert!(docker_desc_requires_exact_match(&desc));
    assert!(!docker_runtime_matches_target(
        &DockerRuntimeIdentity {
            labels: HashMap::from([(
                DOCKER_LABEL_PKG_ID.to_string(),
                "demo-img#0.1.0".to_string(),
            )]),
            ..Default::default()
        },
        &desc,
    ));
}

#[test]
fn docker_runtime_exact_match_rejects_another_deployment_generation() {
    let app_doc = build_script_service_doc();
    let app_spec = build_test_app_spec(app_doc.clone(), build_spec_config(&app_doc));
    let deployment = &app_spec.deployment;
    let matching_labels = HashMap::from([
        (
            DOCKER_LABEL_APP_DOC_OBJECT_ID.to_string(),
            deployment.app_doc_object_id.to_string(),
        ),
        (
            DOCKER_LABEL_SPEC_GENERATION.to_string(),
            deployment.spec_generation.to_string(),
        ),
    ]);

    assert!(docker_runtime_matches_deployment(
        &DockerRuntimeIdentity {
            labels: matching_labels.clone(),
            ..Default::default()
        },
        Some(deployment),
    ));

    let mut stale_labels = matching_labels;
    stale_labels.insert(
        DOCKER_LABEL_SPEC_GENERATION.to_string(),
        (deployment.spec_generation + 1).to_string(),
    );
    assert!(!docker_runtime_matches_deployment(
        &DockerRuntimeIdentity {
            labels: stale_labels,
            ..Default::default()
        },
        Some(deployment),
    ));
}

#[test]
fn agent_process_matching_distinguishes_wildcard_and_exact_checks() {
    let agent_env = Path::new("/opt/buckyos/data/home/alice/.local/share/jarvis-runtime.example");
    let expected_root =
        Path::new("/opt/buckyos/env/pkgs/agent.jarvis-runtime.example#pkg:1234567890");
    let exact_cmd = vec![
        "opendan".to_string(),
        "--agent-id".to_string(),
        "jarvis-runtime.example".to_string(),
        "--agent-bin".to_string(),
        expected_root.to_string_lossy().to_string(),
        "--service-port".to_string(),
        "4060".to_string(),
    ];
    let old_cmd = vec![
        "opendan".to_string(),
        "--agent-id".to_string(),
        "jarvis-runtime.example".to_string(),
        "--agent-bin".to_string(),
        "/opt/buckyos/env/pkgs/agent.jarvis-runtime.example#pkg:oldversion".to_string(),
        "--service-port".to_string(),
        "4060".to_string(),
    ];

    assert!(command_matches_agent_process(
        &exact_cmd,
        "jarvis-runtime.example"
    ));
    assert!(command_matches_agent_process(
        &old_cmd,
        "jarvis-runtime.example"
    ));
    assert!(command_matches_exact_agent_process(
        &exact_cmd,
        "jarvis-runtime.example",
        agent_env,
        Some(expected_root),
        Some("pkg:1234567890"),
    ));
    assert!(!command_matches_exact_agent_process(
        &old_cmd,
        "jarvis-runtime.example",
        agent_env,
        Some(expected_root),
        Some("pkg:1234567890"),
    ));
}

#[test]
fn appservice_control_commands_match_linux_amd64_docker_runtime() {
    let loader = build_service_loader(
        build_appservice_doc(),
        HashMap::from([("www".to_string(), 10080)]),
        PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64),
        true,
    );

    let deploy = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(deploy.runtime, RuntimeType::Docker);
    assert_programs(&deploy.commands, &["pkg-install", "docker", "docker"]);
    assert_eq!(
        deploy.commands[0].args,
        vec![test_exact_package_id("image.demo.example", 1)]
    );
    assert_eq!(
        deploy.commands[2].args,
        vec!["pull", "demo/service:0.1.0-amd64@sha256:deadbeef"]
    );

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(start.runtime, RuntimeType::Docker);
    assert_programs(&start.commands, &["docker", "docker"]);
    let runtime_key = test_runtime_key("demo.example");
    let container_name = test_container_name("test-app");
    assert_eq!(
        start.commands[0].args,
        vec!["rm", "-f", container_name.as_str()]
    );
    assert!(start.commands[1].args.contains(&"run".to_string()));
    assert!(start.commands[1].args.contains(&"--add-host".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"host.docker.internal:host-gateway".to_string()));
    assert!(start.commands[1].args.contains(&"10080:80".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"demo/service:0.1.0-amd64".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_HOST_GATEWAY=<value>".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_KEVENT_DAEMON_ADDR=<value>".to_string()));
    assert!(start.commands[1].args.contains(&format!(
        "buckyos.pkg_id={}",
        test_exact_package_id("image.demo.example", 1)
    )));
    assert!(start.commands[1]
        .args
        .contains(&"buckyos.spec_generation=1".to_string()));
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg.starts_with("buckyos.app_doc_object_id=")));

    let stop = loader.preview_operation(ControlOperation::Stop).unwrap();
    assert_eq!(stop.runtime, RuntimeType::Docker);
    assert_programs(&stop.commands, &["docker"]);
    assert_eq!(
        stop.commands[0].args,
        vec!["rm", "-f", container_name.as_str()]
    );

    let status = loader.preview_operation(ControlOperation::Status).unwrap();
    assert_eq!(status.runtime, RuntimeType::Docker);
    assert_programs(&status.commands, &["docker", "docker", "docker"]);
    assert_eq!(
        status.commands[0].args,
        vec![
            "ps",
            "-q",
            "-f",
            format!("name=^{container_name}$").as_str()
        ]
    );
}

#[test]
fn appservice_control_commands_match_linux_aarch64_docker_runtime() {
    let loader = build_service_loader(
        build_appservice_doc(),
        HashMap::from([("www".to_string(), 10080)]),
        PlatformTarget::new(PlatformOs::Linux, PlatformArch::Aarch64),
        true,
    );

    let deploy = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(deploy.runtime, RuntimeType::Docker);
    assert_eq!(
        deploy.commands[0].args,
        vec![test_exact_package_id("image-arm.demo.example", 2)]
    );
    assert_eq!(
        deploy.commands[2].args,
        vec!["pull", "demo/service:0.1.0-aarch64@sha256:beadfeed"]
    );

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert!(start.commands[1]
        .args
        .contains(&"demo/service:0.1.0-aarch64".to_string()));
}

#[test]
fn appservice_without_container_support_is_rejected_when_only_docker_is_available() {
    let loader = build_service_loader(
        build_appservice_doc(),
        HashMap::new(),
        PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64),
        false,
    );

    let result = loader.preview_operation(ControlOperation::Start);
    assert!(matches!(result, Err(ControlRuntItemErrors::NotSupport(_))));
}

#[test]
fn agent_control_commands_match_expected_process_flow_on_linux() {
    let loader = build_agent_loader(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64));

    let deploy = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(deploy.runtime, RuntimeType::Agent);
    assert_programs(
        &deploy.commands,
        &["pkg-install", "pkg-install", "docker", "docker", "docker"],
    );
    assert_eq!(
        deploy.commands[0].args,
        vec![test_exact_package_id("agent.jarvis-runtime.example", 3)]
    );
    assert_eq!(
        deploy.commands[1].args,
        vec![test_exact_package_id("skills.jarvis-runtime.example", 4)]
    );
    assert_eq!(
        deploy.commands[2].args,
        vec!["pull", "paios/aios:latest-amd64"]
    );
    assert_eq!(
        deploy.commands[3].args,
        vec!["pull", "paios/exttool:latest-amd64"]
    );
    assert_eq!(
        deploy.commands[4].args,
        vec!["volume", "create", "buckyos-exttool"]
    );

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(start.runtime, RuntimeType::Agent);
    assert_programs(&start.commands, &["docker", "docker"]);
    let runtime_key = test_runtime_key("jarvis-runtime.example");
    let container_name = test_container_name("test-app");
    assert_eq!(
        start.commands[0].args,
        vec!["rm", "-f", container_name.as_str()]
    );
    assert!(start.commands[1].args.contains(&"run".to_string()));
    // Unified worker image has the dispatcher baked in, so we no longer
    // override the entrypoint or request SYS_ADMIN at the docker layer.
    assert!(!start.commands[1].args.contains(&"--entrypoint".to_string()));
    assert!(!start.commands[1].args.contains(&"SYS_ADMIN".to_string()));
    assert!(start.commands[1].args.contains(&"--add-host".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"host.docker.internal:host-gateway".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_APP_ID=jarvis-runtime.example".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_APP_TYPE=agent".to_string()));
    assert!(start.commands[1].args.contains(
        &"BUCKYOS_DATA_DIR=/opt/buckyos/data/home/alice/.local/share/jarvis-runtime.example"
            .to_string(),
    ));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_PKG_DIR=/opt/buckyos/bin/jarvis-runtime.example".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_PKG_SOURCE_DIR=/mnt/buckyos/pkg".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_SERVICE_PORT=14060".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_THIS_DEVICE=<value>".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_HOST_GATEWAY=<value>".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_KEVENT_DAEMON_ADDR=<value>".to_string()));
    assert!(start.commands[1].args.contains(&"14060:14060".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"paios/aios:latest-amd64".to_string()));
    // §6.1: upstream pkg mounted read-only, instance volume mounted rw.
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg == "<app_pkg>:/mnt/buckyos/pkg:ro"));
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg == &format!("buckyos-instance-{runtime_key}:/opt/buckyos/instance:rw")));
    // Default ExtTool Volume (§6.1) mounted ro — image seeds it with
    // FreeCADCmd + pre-warmed uv/deno caches on first start.
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg == "buckyos-exttool:/opt/buckyos/tools:ro"));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_EXTTOOL_DIR=/opt/buckyos/tools".to_string()));
    assert!(start.commands[1].args.iter().any(|arg| arg
        == "<app_data>:/opt/buckyos/data/home/alice/.local/share/jarvis-runtime.example:rw"));
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg == "<app_logs>:/opt/buckyos/logs:rw"));
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg == "<app_storage>:/opt/buckyos/storage:rw"));
    // No bootstrap script is passed at the docker layer anymore — the image
    // entrypoint handles instance-volume bootstrap + agent dispatch itself.
    assert!(!start.commands[1]
        .args
        .contains(&"<agent-bootstrap-script>".to_string()));

    let stop = loader.preview_operation(ControlOperation::Stop).unwrap();
    assert_eq!(stop.runtime, RuntimeType::Agent);
    assert_programs(&stop.commands, &["docker"]);
    assert_eq!(
        stop.commands[0].args,
        vec!["rm", "-f", container_name.as_str()]
    );

    let status = loader.preview_operation(ControlOperation::Status).unwrap();
    assert_eq!(status.runtime, RuntimeType::Agent);
    assert_programs(&status.commands, &["docker", "docker", "docker"]);
    assert_eq!(
        status.commands[0].args,
        vec![
            "ps",
            "-q",
            "-f",
            format!("name=^{container_name}$").as_str()
        ]
    );
    assert_eq!(
        status.commands[2].args,
        vec!["images", "-q", "paios/aios:latest-amd64"]
    );
}

#[test]
fn agent_control_commands_support_custom_runtime_image_repo() {
    let loader = build_agent_loader(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_worker_image_repo_override("paios/aios_dev");

    let deploy = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(
        deploy.commands[2].args,
        vec!["pull", "paios/aios_dev:latest-amd64"]
    );

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert!(start.commands[1]
        .args
        .contains(&"paios/aios_dev:latest-amd64".to_string()));

    let status = loader.preview_operation(ControlOperation::Status).unwrap();
    assert_eq!(
        status.commands[2].args,
        vec!["images", "-q", "paios/aios_dev:latest-amd64"]
    );
}

#[test]
fn agent_stop_command_uses_docker_on_windows() {
    let loader = build_agent_loader(PlatformTarget::new(
        PlatformOs::Windows,
        PlatformArch::Amd64,
    ));
    let stop = loader.preview_operation(ControlOperation::Stop).unwrap();
    assert_eq!(stop.runtime, RuntimeType::Agent);
    assert_eq!(stop.commands[0].program, "docker");
    assert_eq!(
        stop.commands[0].args,
        vec!["rm", "-f", test_container_name("test-app").as_str()]
    );
}

#[test]
fn agent_requires_container_support() {
    let loader = build_agent_loader(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(false);
    let result = loader.preview_operation(ControlOperation::Start);
    assert!(matches!(result, Err(ControlRuntItemErrors::NotSupport(_))));
}

#[test]
fn host_script_start_preview_uses_docker_with_script_service_image() {
    let config = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: build_local_service_doc(),
        user_id: "alice".to_string(),
        install_config: ServiceSpecConfig::default(),
    };
    let loader = AppLoader::new_for_local("desktop-tool", config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(false)
        .with_worker_image_repo_override("paios/aios");

    let preview = loader.preview_operation(ControlOperation::Start).unwrap();
    let runtime_key = test_runtime_key("desktop-tool");
    let container_name = test_container_name("desktop-tool");
    assert_eq!(preview.runtime, RuntimeType::HostScript);
    assert_eq!(preview.commands.len(), 2);
    assert_eq!(preview.commands[0].program, "docker");
    assert_eq!(
        preview.commands[0].args,
        vec!["rm", "-f", container_name.as_str()]
    );
    assert_eq!(preview.commands[1].program, "docker");
    assert!(preview.commands[1].args.contains(&"run".to_string()));
    assert!(preview.commands[1].args.contains(&"--add-host".to_string()));
    assert!(preview.commands[1]
        .args
        .contains(&"host.docker.internal:host-gateway".to_string()));
    assert!(preview.commands[1].args.contains(&container_name));
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a.contains("paios/aios:")));
    // Unified worker image mounts the instance volume and read-only pkg source.
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a == &format!("buckyos-instance-{runtime_key}:/opt/buckyos/instance:rw")));
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a == "buckyos-exttool:/opt/buckyos/tools:ro"));
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a == "<app_pkg>:/mnt/buckyos/pkg:ro"));
    assert!(preview.commands[1]
        .args
        .contains(&"BUCKYOS_APP_TYPE=script".to_string()));
    assert!(preview.commands[1]
        .args
        .contains(&"BUCKYOS_PKG_DIR=/opt/buckyos/bin/desktop-tool".to_string()));
    assert!(preview.commands[1]
        .args
        .contains(&"BUCKYOS_PKG_SOURCE_DIR=/mnt/buckyos/pkg".to_string()));
}

#[test]
fn host_script_stop_preview_uses_docker_rm() {
    let config = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: build_local_service_doc(),
        user_id: "alice".to_string(),
        install_config: ServiceSpecConfig::default(),
    };
    let loader = AppLoader::new_for_local("desktop-tool", config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(false);

    let preview = loader.preview_operation(ControlOperation::Stop).unwrap();
    assert_eq!(preview.runtime, RuntimeType::HostScript);
    assert_eq!(preview.commands.len(), 1);
    assert_eq!(preview.commands[0].program, "docker");
    assert_eq!(
        preview.commands[0].args,
        vec!["rm", "-f", test_container_name("desktop-tool").as_str()]
    );
}

#[test]
fn host_script_deploy_preview_includes_pkg_install_and_image_pull() {
    let config = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: build_local_service_doc(),
        user_id: "alice".to_string(),
        install_config: ServiceSpecConfig::default(),
    };
    let loader = AppLoader::new_for_local("desktop-tool", config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(false)
        .with_worker_image_repo_override("paios/aios");

    let preview = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(preview.runtime, RuntimeType::HostScript);
    assert_eq!(preview.commands.len(), 4);
    assert_eq!(preview.commands[0].program, "pkg-install");
    assert_eq!(preview.commands[1].program, "docker");
    assert_eq!(preview.commands[1].args[0], "pull");
    assert!(preview.commands[1].args[1].contains("paios/aios:"));
    assert_eq!(preview.commands[2].program, "docker");
    assert_eq!(preview.commands[2].args[0], "pull");
    assert!(preview.commands[2].args[1].contains("paios/exttool:"));
    assert_eq!(preview.commands[3].program, "docker");
    assert_eq!(
        preview.commands[3].args,
        vec!["volume", "create", "buckyos-exttool"]
    );
}

#[test]
fn host_script_aarch64_uses_correct_image_tag() {
    let config = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: build_local_service_doc(),
        user_id: "alice".to_string(),
        install_config: ServiceSpecConfig::default(),
    };
    let loader = AppLoader::new_for_local("desktop-tool", config)
        .with_platform(PlatformTarget::new(
            PlatformOs::Linux,
            PlatformArch::Aarch64,
        ))
        .with_container_support_override(false)
        .with_worker_image_repo_override("paios/aios");

    let preview = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(preview.commands[1].args[1], "paios/aios:latest-aarch64");
}

#[test]
fn script_pkg_field_routes_service_app_to_host_script() {
    let loader = build_service_loader(
        build_script_service_doc(),
        HashMap::from([("www".to_string(), 18080)]),
        PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64),
        true,
    )
    .with_worker_image_repo_override("paios/aios");

    let preview = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(preview.runtime, RuntimeType::HostScript);
    assert_eq!(preview.commands.len(), 2);
    assert_eq!(preview.commands[1].program, "docker");
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a.contains("paios/aios:")));
}

#[test]
fn script_pkg_field_works_on_any_platform() {
    for (os, arch) in [
        (PlatformOs::Linux, PlatformArch::Amd64),
        (PlatformOs::Linux, PlatformArch::Aarch64),
        (PlatformOs::Macos, PlatformArch::Aarch64),
        (PlatformOs::Windows, PlatformArch::Amd64),
    ] {
        let loader = build_service_loader(
            build_script_service_doc(),
            HashMap::from([("www".to_string(), 18080)]),
            PlatformTarget::new(os, arch),
            true,
        );
        let preview = loader.preview_operation(ControlOperation::Deploy).unwrap();
        assert_eq!(preview.runtime, RuntimeType::HostScript);
        assert_eq!(preview.commands[0].program, "pkg-install");
        assert_eq!(
            preview.commands[0].args,
            vec![test_exact_package_id("script.systest.example", 12)]
        );
    }
}

#[test]
fn web_app_type_is_rejected_by_runtime_selector() {
    let loader = build_service_loader(
        build_web_doc(),
        HashMap::new(),
        PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64),
        false,
    );
    for operation in [
        ControlOperation::Deploy,
        ControlOperation::Start,
        ControlOperation::Stop,
        ControlOperation::Status,
    ] {
        let result = loader.preview_operation(operation);
        assert!(matches!(result, Err(ControlRuntItemErrors::NotSupport(_))));
    }
}
