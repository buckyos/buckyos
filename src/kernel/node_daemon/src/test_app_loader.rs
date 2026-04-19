use crate::app_loader::{
    command_matches_agent_process, command_matches_exact_agent_process,
    container_list_contains_name, docker_desc_requires_exact_match,
    docker_image_tar_candidates_for_arch, docker_missing_text, docker_runtime_matches_target,
    normalize_digest, parse_docker_container_inspect, resolve_aios_image_repo_from_paths,
    AppLoader, CommandSpec, ControlOperation, DockerRuntimeIdentity, PlatformArch, PlatformOs,
    PlatformTarget, RuntimeType, DOCKER_LABEL_IMAGE_DIGEST, DOCKER_LABEL_PKG_OBJID,
};
use crate::run_item::ControlRuntItemErrors;
use buckyos_api::{
    AppDoc, AppServiceInstanceConfig, AppServiceSpec, AppType, LocalAppInstanceConfig,
    ServiceInstallConfig, ServiceInstanceState, ServiceState, SubPkgDesc,
};
use name_lib::DID;
use ndn_lib::ObjId;
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
    AppDoc::builder(AppType::AppService, "demo", "0.1.0", "did:bns:test", &owner)
        .amd64_docker_image(
            SubPkgDesc::new("demo-img#0.1.0")
                .docker_image_name("demo/service:0.1.0-amd64")
                .docker_image_digest("sha256:deadbeef"),
        )
        .aarch64_docker_image(
            SubPkgDesc::new("demo-img-arm#0.1.0")
                .docker_image_name("demo/service:0.1.0-aarch64")
                .docker_image_digest("sha256:beadfeed"),
        )
        .service_port("www", 80)
        .build()
        .unwrap()
}

fn build_agent_doc_without_category() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    let mut doc = AppDoc::builder(AppType::Agent, "jarvis", "0.1.0", "did:bns:test", &owner)
        .agent_pkg(SubPkgDesc::new("jarvis-agent#0.1.0"))
        .agent_skills_pkg(SubPkgDesc::new("jarvis-skills#0.1.0"))
        .build()
        .unwrap();
    doc._base.categories.clear();
    doc.install_config_tips
        .service_ports
        .insert("main".to_string(), 4060);
    doc
}

fn build_web_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    AppDoc::builder(AppType::Web, "portal", "0.1.0", "did:bns:test", &owner)
        .web_pkg(SubPkgDesc::new("portal-web#0.1.0"))
        .build()
        .unwrap()
}

fn build_local_service_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    let mut doc = AppDoc::builder(
        AppType::Service,
        "desktop-tool",
        "0.1.0",
        "did:bns:test",
        &owner,
    )
    .build()
    .unwrap();

    doc.pkg_list.aarch64_linux_app = Some(SubPkgDesc::new("desktop-tool-linux-arm#0.1.0"));
    doc.pkg_list.amd64_linux_app = Some(SubPkgDesc::new("desktop-tool-linux-amd#0.1.0"));
    doc.pkg_list.aarch64_apple_app = Some(SubPkgDesc::new("desktop-tool-macos-arm#0.1.0"));
    doc.pkg_list.amd64_apple_app = Some(SubPkgDesc::new("desktop-tool-macos-amd#0.1.0"));
    doc.pkg_list.aarch64_win_app = Some(SubPkgDesc::new("desktop-tool-win-arm#0.1.0"));
    doc.pkg_list.amd64_win_app = Some(SubPkgDesc::new("desktop-tool-win-amd#0.1.0"));

    doc
}

fn build_script_service_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:test").unwrap();
    AppDoc::builder(AppType::Service, "systest", "0.1.0", "did:bns:test", &owner)
        .script_pkg(SubPkgDesc::new("systest-script#0.1.0"))
        .service_port("www", 3000)
        .build()
        .unwrap()
}

fn build_service_loader(
    app_doc: AppDoc,
    service_ports_config: HashMap<String, u16>,
    platform: PlatformTarget,
    support_container: bool,
) -> AppLoader {
    let config = AppServiceInstanceConfig {
        target_state: ServiceInstanceState::Started,
        node_id: "ood1".to_string(),
        app_spec: AppServiceSpec {
            app_doc,
            app_index: 1,
            user_id: "alice".to_string(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            install_config: ServiceInstallConfig::default(),
        },
        service_ports_config,
    };
    AppLoader::new_for_service("demo@alice@ood1", config)
        .with_platform(platform)
        .with_container_support_override(support_container)
}

fn build_agent_loader(platform: PlatformTarget) -> AppLoader {
    let config = AppServiceInstanceConfig {
        target_state: ServiceInstanceState::Started,
        node_id: "ood1".to_string(),
        app_spec: AppServiceSpec {
            app_doc: build_agent_doc_without_category(),
            app_index: 1,
            user_id: "alice".to_string(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            install_config: ServiceInstallConfig::default(),
        },
        service_ports_config: HashMap::from([
            ("www".to_string(), 10080),
            ("main".to_string(), 14060),
        ]),
    };
    AppLoader::new_for_service("jarvis@alice@ood1", config)
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
                    "buckyos.full_appid": "alice-demo",
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
            .and_then(|labels| labels.get("buckyos.full_appid"))
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
    let mut desc = SubPkgDesc::new("demo-img#0.1.0")
        .docker_image_name("demo/service:0.1.0-amd64")
        .docker_image_digest("demo/service@sha256:deadbeef");
    desc.pkg_objid = Some(ObjId::new("pkg:1234567890").unwrap());

    assert!(docker_desc_requires_exact_match(&desc));
    assert!(docker_runtime_matches_target(
        &DockerRuntimeIdentity {
            image_id: Some("sha256:anotherhash".to_string()),
            repo_digests: vec!["demo/service@sha256:deadbeef".to_string()],
            labels: HashMap::from([
                (
                    DOCKER_LABEL_PKG_OBJID.to_string(),
                    "pkg:1234567890".to_string(),
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
            labels: HashMap::from([(
                DOCKER_LABEL_PKG_OBJID.to_string(),
                "pkg:oldversion".to_string(),
            )]),
        },
        &desc,
    ));
    assert!(docker_runtime_matches_target(
        &DockerRuntimeIdentity {
            image_id: Some("sha256:deadbeef".to_string()),
            ..Default::default()
        },
        &SubPkgDesc::new("demo-img#0.1.0")
            .docker_image_name("demo/service:0.1.0-amd64")
            .docker_image_digest("sha256:deadbeef"),
    ));
}

#[test]
fn agent_process_matching_distinguishes_wildcard_and_exact_checks() {
    let agent_env = Path::new("/opt/buckyos/data/home/alice/.local/share/jarvis");
    let expected_root = Path::new("/opt/buckyos/env/pkgs/jarvis-agent#pkg:1234567890");
    let exact_cmd = vec![
        "opendan".to_string(),
        "--agent-id".to_string(),
        "jarvis".to_string(),
        "--agent-env".to_string(),
        agent_env.to_string_lossy().to_string(),
        "--agent-bin".to_string(),
        expected_root.to_string_lossy().to_string(),
        "--service-port".to_string(),
        "4060".to_string(),
    ];
    let old_cmd = vec![
        "opendan".to_string(),
        "--agent-id".to_string(),
        "jarvis".to_string(),
        "--agent-env".to_string(),
        agent_env.to_string_lossy().to_string(),
        "--agent-bin".to_string(),
        "/opt/buckyos/env/pkgs/jarvis-agent#pkg:oldversion".to_string(),
        "--service-port".to_string(),
        "4060".to_string(),
    ];

    assert!(command_matches_agent_process(
        &exact_cmd, "jarvis", agent_env,
    ));
    assert!(command_matches_agent_process(&old_cmd, "jarvis", agent_env,));
    assert!(command_matches_exact_agent_process(
        &exact_cmd,
        "jarvis",
        agent_env,
        Some(expected_root),
        Some("pkg:1234567890"),
    ));
    assert!(!command_matches_exact_agent_process(
        &old_cmd,
        "jarvis",
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
    assert_eq!(deploy.commands[0].args, vec!["demo-img"]);
    assert_eq!(
        deploy.commands[2].args,
        vec!["pull", "demo/service:0.1.0-amd64@sha256:deadbeef"]
    );

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(start.runtime, RuntimeType::Docker);
    assert_programs(&start.commands, &["docker", "docker"]);
    assert_eq!(start.commands[0].args, vec!["rm", "-f", "alice-demo"]);
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

    let stop = loader.preview_operation(ControlOperation::Stop).unwrap();
    assert_eq!(stop.runtime, RuntimeType::Docker);
    assert_programs(&stop.commands, &["docker"]);
    assert_eq!(stop.commands[0].args, vec!["rm", "-f", "alice-demo"]);

    let status = loader.preview_operation(ControlOperation::Status).unwrap();
    assert_eq!(status.runtime, RuntimeType::Docker);
    assert_programs(&status.commands, &["docker", "docker", "docker"]);
    assert_eq!(
        status.commands[0].args,
        vec!["ps", "-q", "-f", "name=^alice-demo$"]
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
    assert_eq!(deploy.commands[0].args, vec!["demo-img-arm"]);
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
    assert_programs(&deploy.commands, &["pkg-install", "pkg-install", "docker"]);
    assert_eq!(deploy.commands[0].args, vec!["jarvis-agent"]);
    assert_eq!(deploy.commands[1].args, vec!["jarvis-skills"]);
    assert_eq!(
        deploy.commands[2].args,
        vec!["pull", "paios/aios:latest-amd64"]
    );

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(start.runtime, RuntimeType::Agent);
    assert_programs(&start.commands, &["docker", "docker"]);
    assert_eq!(start.commands[0].args, vec!["rm", "-f", "alice-jarvis"]);
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
        .contains(&"BUCKYOS_APP_ID=jarvis".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_APP_TYPE=agent".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_SERVICE_PORT=14060".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_THIS_DEVICE=<value>".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_HOST_GATEWAY=<value>".to_string()));
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
        .any(|arg| arg == "buckyos-instance-alice-jarvis:/opt/buckyos/instance:rw"));
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
    assert_eq!(stop.commands[0].args, vec!["rm", "-f", "alice-jarvis"]);

    let status = loader.preview_operation(ControlOperation::Status).unwrap();
    assert_eq!(status.runtime, RuntimeType::Agent);
    assert_programs(&status.commands, &["docker", "docker", "docker"]);
    assert_eq!(
        status.commands[0].args,
        vec!["ps", "-q", "-f", "name=^alice-jarvis$"]
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
    assert_eq!(stop.commands[0].args, vec!["rm", "-f", "alice-jarvis"]);
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
        install_config: ServiceInstallConfig::default(),
    };
    let loader = AppLoader::new_for_local("desktop-tool", config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(false)
        .with_worker_image_repo_override("paios/aios");

    let preview = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(preview.runtime, RuntimeType::HostScript);
    assert_eq!(preview.commands.len(), 2);
    assert_eq!(preview.commands[0].program, "docker");
    assert_eq!(
        preview.commands[0].args,
        vec!["rm", "-f", "alice-desktop-tool"]
    );
    assert_eq!(preview.commands[1].program, "docker");
    assert!(preview.commands[1].args.contains(&"run".to_string()));
    assert!(preview.commands[1].args.contains(&"--add-host".to_string()));
    assert!(preview.commands[1]
        .args
        .contains(&"host.docker.internal:host-gateway".to_string()));
    assert!(preview.commands[1]
        .args
        .contains(&"alice-desktop-tool".to_string()));
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a.contains("paios/aios:")));
    // Unified worker image mounts the instance volume and read-only pkg source.
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a == "buckyos-instance-alice-desktop-tool:/opt/buckyos/instance:rw"));
    assert!(preview.commands[1]
        .args
        .iter()
        .any(|a| a == "<app_pkg>:/mnt/buckyos/pkg:ro"));
    assert!(preview.commands[1]
        .args
        .contains(&"BUCKYOS_APP_TYPE=script".to_string()));
}

#[test]
fn host_script_stop_preview_uses_docker_rm() {
    let config = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: build_local_service_doc(),
        user_id: "alice".to_string(),
        install_config: ServiceInstallConfig::default(),
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
        vec!["rm", "-f", "alice-desktop-tool"]
    );
}

#[test]
fn host_script_deploy_preview_includes_pkg_install_and_image_pull() {
    let config = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: build_local_service_doc(),
        user_id: "alice".to_string(),
        install_config: ServiceInstallConfig::default(),
    };
    let loader = AppLoader::new_for_local("desktop-tool", config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(false)
        .with_worker_image_repo_override("paios/aios");

    let preview = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(preview.runtime, RuntimeType::HostScript);
    assert_eq!(preview.commands.len(), 2);
    assert_eq!(preview.commands[0].program, "pkg-install");
    assert_eq!(preview.commands[1].program, "docker");
    assert_eq!(preview.commands[1].args[0], "pull");
    assert!(preview.commands[1].args[1].contains("paios/aios:"));
}

#[test]
fn host_script_aarch64_uses_correct_image_tag() {
    let config = LocalAppInstanceConfig {
        target_state: ServiceInstanceState::Started,
        enable: true,
        app_doc: build_local_service_doc(),
        user_id: "alice".to_string(),
        install_config: ServiceInstallConfig::default(),
    };
    let loader = AppLoader::new_for_local("desktop-tool", config)
        .with_platform(PlatformTarget::new(
            PlatformOs::Linux,
            PlatformArch::Aarch64,
        ))
        .with_container_support_override(false)
        .with_worker_image_repo_override("paios/aios");

    let preview = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(
        preview.commands[1].args[1],
        "paios/aios:latest-aarch64"
    );
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
        assert_eq!(preview.commands[0].args, vec!["systest-script"]);
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
