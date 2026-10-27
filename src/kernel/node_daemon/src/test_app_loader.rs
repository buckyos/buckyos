use crate::app_loader::{
    buckyos_tool_mounts, command_matches_agent_process, command_matches_exact_agent_process,
    container_list_contains_name, docker_deployment_health, docker_desc_requires_exact_match,
    docker_image_tar_candidates_for_arch, docker_missing_text, docker_runtime_matches_deployment,
    docker_runtime_matches_target, inspect_docker_image_layout, normalize_digest,
    parse_docker_container_inspect, primary_service_port, validate_app_launch_config, AppLoader,
    CommandSpec,
    ControlOperation, DockerCommandRunner,
    DockerExecOutcome, DockerRuntimeIdentity, PlatformArch, PlatformOs, PlatformTarget,
    RuntimeType, DOCKER_LABEL_APP_DOC_OBJECT_ID, DOCKER_LABEL_IMAGE_DIGEST, DOCKER_LABEL_PKG_ID,
    DOCKER_LABEL_PKG_OBJID, DOCKER_LABEL_SPEC_GENERATION,
};
use crate::paios_images::exttool_prepare_lock;
use crate::run_item::ControlRuntItemErrors;
use buckyos_api::{
    AppDoc, AppId, AppInstanceId, AppServiceInstanceConfig, AppServiceSpec, AppType,
    DeploymentIdentity, DeploymentPackage, LocalAppInstanceConfig, MountPointConfig,
    PermissionItem, ServiceEndpointConfig, ServiceInstanceState, ServiceSpecConfig,
    ServiceState, SubPkgDesc, APP_PERMISSION_USER_HOME, OBJ_TYPE_APP_DOC,
};
use name_lib::DID;
use ndn_lib::ObjId;
use package_lib::PackageId;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tempfile::tempdir;

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
    let app_did = DID::from_str("did:web:jarvis-agent.example").unwrap();
    AppDoc::builder(
        AppType::Agent,
        "jarvis-agent",
        "0.1.0",
        "did:bns:test",
        &owner,
    )
    .app_did(app_did)
    .agent_pkg(
        SubPkgDesc::new("agent.jarvis-agent.example#0.1.0")
            .package_meta_object_id(test_package_object_id(3)),
    )
    .agent_skills_pkg(
        SubPkgDesc::new("skills.jarvis-agent.example#0.1.0")
            .package_meta_object_id(test_package_object_id(4)),
    )
    .service_port("main", 4060)
    .build()
    .unwrap()
}

#[test]
fn container_system_config_mount_is_read_only_and_scoped() {
    if std::env::var_os("BUCKYOS_SYSTEM_CONFIG_MOUNT_TEST_CHILD").is_none() {
        let root = tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "test_app_loader::container_system_config_mount_is_read_only_and_scoped",
                "--nocapture",
            ])
            .env("BUCKYOS_ROOT", root.path())
            .env("BUCKYOS_SYSTEM_CONFIG_MOUNT_TEST_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }

    let root = buckyos_kit::get_buckyos_root_dir();
    let canonical_root = fs::canonicalize(&root).unwrap();
    let etc = root.join("etc");
    let pkg = root.join("test-package");
    fs::create_dir_all(&etc).unwrap();
    fs::create_dir_all(&pkg).unwrap();
    fs::create_dir_all(root.join("security")).unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(root.join("security/start_config.json"), "private-start-config").unwrap();
    fs::write(etc.join("node_identity.json"), "host-node-identity").unwrap();
    fs::write(root.join("security/device.key"), "private-device-key").unwrap();
    fs::write(root.join("bin/host-runtime"), "host-platform-binary").unwrap();
    let machine = etc.join("machine.json");
    let target = "/opt/buckyos/etc";
    let platform = PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64);
    let docker = build_service_loader(build_appservice_doc(), HashMap::new(), platform, true);
    let agent = build_agent_loader(platform);
    let script = AppLoader::new_for_local(
        "desktop-tool",
        LocalAppInstanceConfig {
            target_state: ServiceInstanceState::Started,
            enable: true,
            app_doc: build_local_service_doc(),
            user_id: "alice".to_string(),
            install_config: ServiceSpecConfig::default(),
        },
    )
    .with_platform(platform)
    .with_container_support_override(false)
    .with_worker_image_repo_override("paios/aios")
    .with_paios_images_root(local_paios_root());
    let loaders = [
        (RuntimeType::Docker, docker),
        (RuntimeType::Agent, agent),
        (RuntimeType::HostScript, script),
    ];

    for present in [false, true] {
        if present {
            fs::write(
                &machine,
                r#"{"web3_bridge":{"bns":"bridge.example.org"},"force_https":true}"#,
            )
            .unwrap();
        }
        for (runtime, loader) in &loaders {
            let mounts = match runtime {
                RuntimeType::Docker => loader.test_docker_volume_mounts(),
                RuntimeType::Agent => loader.test_worker_volume_mounts(&pkg, true),
                RuntimeType::HostScript => loader.test_worker_volume_mounts(&pkg, false),
                _ => unreachable!(),
            }
            .unwrap();
            let config_mounts = mounts
                .iter()
                .filter(|(container_path, _, _)| container_path == target)
                .collect::<Vec<_>>();
            assert_eq!(config_mounts.len(), 1, "{runtime:?}");
            assert_eq!(
                fs::canonicalize(&config_mounts[0].1).unwrap(),
                fs::canonicalize(&etc).unwrap(),
                "{runtime:?}"
            );
            assert_eq!(config_mounts[0].2, "ro", "{runtime:?}");
            for (container_path, host_path, _) in &mounts {
                let canonical_host_path = fs::canonicalize(host_path).unwrap();
                if canonical_host_path.starts_with(canonical_root.join("etc")) {
                    assert_eq!(
                        canonical_host_path,
                        fs::canonicalize(&etc).unwrap(),
                        "{runtime:?}: {host_path:?}"
                    );
                }
                assert!(
                    !canonical_host_path.starts_with(canonical_root.join("security")),
                    "{runtime:?}"
                );
                assert!(
                    !canonical_host_path.starts_with(canonical_root.join("bin")),
                    "{runtime:?}"
                );
                if Path::new(container_path).starts_with("/opt/buckyos/etc") {
                    assert_eq!(container_path, target, "{runtime:?}");
                }
                assert_ne!(container_path, "/opt/buckyos", "{runtime:?}");
                assert!(
                    !Path::new(container_path).starts_with("/opt/buckyos/security"),
                    "{runtime:?}"
                );
                assert!(
                    !Path::new(container_path).starts_with("/opt/buckyos/bin"),
                    "{runtime:?}"
                );
            }
            if present {
                if let Ok(image) = std::env::var("BUCKYOS_MACHINE_CONFIG_DOCKER_IMAGE") {
                    let mut command = std::process::Command::new("docker");
                    command.args([
                        "run", "--rm", "--network", "none", "--user", "0", "--cap-drop",
                        "ALL", "--security-opt", "no-new-privileges", "--entrypoint", "sh",
                    ]);
                    for (container_path, host_path, access) in &mounts {
                        command.args([
                            "-v",
                            &format!("{}:{container_path}:{access}", host_path.display()),
                        ]);
                    }
                    let script = r#"set -eu
test "$(cat /opt/buckyos/etc/machine.json)" = '{"web3_bridge":{"bns":"bridge.example.org"},"force_https":true}'
if (printf changed >> /opt/buckyos/etc/machine.json) 2>/dev/null; then exit 23; fi
test ! -e /opt/buckyos/etc/start_config.json
test "$(cat /opt/buckyos/etc/node_identity.json)" = 'host-node-identity'
if (touch /opt/buckyos/etc/new-config.json) 2>/dev/null; then exit 24; fi
test ! -e /opt/buckyos/security/start_config.json
test ! -e /opt/buckyos/security/device.key
test ! -e /opt/buckyos/bin/host-runtime
"#;
                    let output = command.args([&image, "-c", script]).output().unwrap();
                    assert!(
                        output.status.success(),
                        "{runtime:?}: {}\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
            }

            let preview = loader.preview_operation(ControlOperation::Start).unwrap();
            assert_eq!(&preview.runtime, runtime);
            let preview_etc = if cfg!(target_os = "windows") {
                etc.clone()
            } else {
                canonical_root.join("etc")
            };
            let config_mount = format!("{}:{target}:ro", preview_etc.display());
            let preview_mounts = preview.commands[1]
                .args
                .windows(2)
                .filter(|pair| pair[0] == "-v")
                .map(|pair| pair[1].as_str())
                .collect::<Vec<_>>();
            assert!(
                preview_mounts.contains(&config_mount.as_str()),
                "{runtime:?}"
            );
            for mount in preview_mounts {
                for forbidden in [
                    "start_config.json",
                    ":/opt/buckyos/security",
                    ":/opt/buckyos/bin",
                ] {
                    assert!(!mount.contains(forbidden), "{runtime:?}: {mount}");
                }
            }
        }
        assert_eq!(machine.is_file(), present);
        assert!(!machine.is_dir());
        if !present {
            assert!(!machine.exists());
        }
    }
    fs::create_dir_all(etc.join("scheduler")).unwrap();
    for name in ["start_config.json", "ca_key.pem", "scheduler/boot.template.toml"] {
        let misplaced = etc.join(name);
        fs::write(&misplaced, "-----BEGIN PRIVATE KEY-----").unwrap();
        for (runtime, loader) in &loaders {
            let result = match runtime {
                RuntimeType::Docker => loader.test_docker_volume_mounts(),
                RuntimeType::Agent => loader.test_worker_volume_mounts(&pkg, true),
                RuntimeType::HostScript => loader.test_worker_volume_mounts(&pkg, false),
                _ => unreachable!(),
            };
            assert!(result.is_err(), "{runtime:?}: {name}");
        }
        fs::remove_file(misplaced).unwrap();
    }
}

#[test]
fn agent_permission_mount_boundaries() {
    if std::env::var_os("BUCKYOS_AGENT_MOUNTS_TEST_CHILD").is_none() {
        let root = tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "test_app_loader::agent_permission_mount_boundaries", "--nocapture"])
            .env("BUCKYOS_ROOT", root.path())
            .env("BUCKYOS_AGENT_MOUNTS_TEST_CHILD", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
        return;
    }
    let root = buckyos_kit::get_buckyos_root_dir();
    let home = root.join("data/home/alice");
    let data = home.join("agents/jarvis-agent.example");
    let pkg = root.join("test-package");
    fs::create_dir_all(&pkg).unwrap();
    fs::create_dir_all(home.join("Documents")).unwrap();
    for (actions, expected_access) in [
        (vec!["read".into()], "ro"),
        (vec!["read".into(), "write".into()], "rw"),
    ] {
        let mut app_doc = build_agent_doc_without_category();
        app_doc.permissions.push(PermissionItem {
            scope_path: APP_PERMISSION_USER_HOME.into(),
            required: true,
            actions,
            exp: None,
        });
        let mut config = build_spec_config(&app_doc);
        config.data_mount_point.insert(
            "/opt/buckyos/data/home/alice".into(),
            MountPointConfig {
                target_path: "data/home/alice".into(),
                access: "read_write".into(),
            },
        );
        config.data_mount_point.insert(
            "/home-alias".into(),
            MountPointConfig {
                target_path: "data/home/alice/Documents".into(),
                access: "read_write".into(),
            },
        );
        config.external_mount_point.insert(
            "/external-home-alias".into(),
            MountPointConfig {
                target_path: "data/home/alice/Documents".into(),
                access: "read_write".into(),
            },
        );
        config.external_mount_point.insert(
            "/other-agent".into(),
            MountPointConfig {
                target_path: "data/home/alice/agents/other-agent.example".into(),
                access: "read_write".into(),
            },
        );
        config.external_mount_point.insert(
            "/opt/buckyos/data/home/alice/agents/jarvis-agent.example".into(),
            MountPointConfig {
                target_path: "data/home/alice/Documents".into(),
                access: "read_only".into(),
            },
        );
        config.external_mount_point.insert(
            "/opt/buckyos/data/home/alice/agents/jarvis-agent.example/subdir".into(),
            MountPointConfig {
                target_path: "data/home/alice/Documents".into(),
                access: "read_only".into(),
            },
        );
        let spec = build_test_app_spec(app_doc, config);
        let instance = AppServiceInstanceConfig::new("ood1", &spec).unwrap();
        let loader = AppLoader::new_for_service(&spec.app_instance_id.to_string(), instance);
        let mounts = loader.test_agent_volume_mounts(&pkg).unwrap();
        for target in [
            "/opt/buckyos/data/home/alice",
            "/home-alias",
            "/external-home-alias",
            "/other-agent",
        ] {
            let (_, _, access) = mounts.iter().find(|(path, _, _)| path == target).unwrap();
            assert_eq!(*access, expected_access, "{target}");
        }
        let (_, actual, access) = mounts
            .iter()
            .find(|(path, _, _)| {
                path == "/opt/buckyos/data/home/alice/agents/jarvis-agent.example"
            })
            .unwrap();
        assert_eq!(actual, &fs::canonicalize(&data).unwrap());
        assert_eq!(*access, "rw");
        assert!(!mounts.iter().any(|(path, _, _)| path.ends_with("/subdir")));
        let (_, storage, access) = mounts
            .iter()
            .find(|(path, _, _)| path == "/opt/buckyos/storage")
            .unwrap();
        assert_eq!(*access, "rw");
        assert!(storage.starts_with(fs::canonicalize(root.join("storage/apps")).unwrap()));
        assert_ne!(storage, &root.join("storage"));
        if let Ok(image) = std::env::var("BUCKYOS_AGENT_MOUNTS_DOCKER_IMAGE") {
            let mut command = std::process::Command::new("docker");
            command.args([
                "run", "--rm", "--network", "none", "--user", "0", "--cap-drop", "ALL",
                "--security-opt", "no-new-privileges", "--entrypoint", "sh",
            ]);
            for (target, source, access) in &mounts {
                command.args(["-v", &format!("{}:{target}:{access}", source.display())]);
            }
            let writes = [
                "/opt/buckyos/data/home/alice/Documents/agent-permissions-test",
                "/home-alias/agent-permissions-test",
                "/external-home-alias/agent-permissions-test",
            ]
            .map(|path| {
                if expected_access == "ro" {
                    format!("if printf denied > {path}; then exit 23; fi")
                } else {
                    format!("printf allowed > {path}")
                }
            })
            .join("\n");
            let script = format!(
                "set -eu\nmkdir -p /opt/buckyos/data/home/alice/agents/jarvis-agent.example/subdir\nprintf writable > /opt/buckyos/data/home/alice/agents/jarvis-agent.example/subdir/state\n{writes}"
            );
            let output = command.args([&image, "-c", &script]).output().unwrap();
            assert!(
                output.status.success(),
                "{expected_access}: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            assert_eq!(
                fs::read_to_string(data.join("subdir/state")).unwrap(),
                "writable"
            );
        }
    }
    #[cfg(unix)]
    {
        let alias = root.join("home-link");
        std::os::unix::fs::symlink(&home, &alias).unwrap();
        let mut app_doc = build_agent_doc_without_category();
        app_doc.permissions.push(PermissionItem {
            scope_path: APP_PERMISSION_USER_HOME.into(),
            required: true,
            actions: vec!["read".into()],
            exp: None,
        });
        let mut config = build_spec_config(&app_doc);
        config.external_mount_point.insert(
            "/symlink-alias".into(),
            MountPointConfig {
                target_path: "home-link/new-directory".into(),
                access: "read_write".into(),
            },
        );
        let spec = build_test_app_spec(app_doc, config);
        let instance = AppServiceInstanceConfig::new("ood1", &spec).unwrap();
        let loader = AppLoader::new_for_service(&spec.app_instance_id.to_string(), instance);
        let mounts = loader.test_agent_volume_mounts(&pkg).unwrap();
        let (_, host, access) = mounts.iter().find(|(path, _, _)| path == "/symlink-alias").unwrap();
        assert_eq!(host, &home.join("new-directory"));
        assert_eq!(*access, "ro");
        fs::remove_dir_all(&data).unwrap();
        std::os::unix::fs::symlink(home.join("Documents"), &data).unwrap();
        assert!(loader.test_agent_volume_mounts(&pkg).is_err());
    }
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
        .with_paios_images_root(local_paios_root())
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

#[test]
fn app_launch_requires_current_authorized_identity_and_deployment() {
    let doc = build_agent_doc_without_category();
    let spec = build_test_app_spec(doc.clone(), build_spec_config(&doc));
    let config = AppServiceInstanceConfig::new("ood1", &spec).unwrap();
    assert!(validate_app_launch_config(&config, &spec, "ood1").is_ok());
    assert!(validate_app_launch_config(&config, &spec, "other-node").is_err());
    let mut stopped = config.clone();
    stopped.target_state = ServiceInstanceState::Stopped;
    assert!(validate_app_launch_config(&stopped, &spec, "ood1").is_err());
    let mut updated = spec.clone();
    updated.deployment.spec_generation += 1;
    assert!(validate_app_launch_config(&config, &updated, "ood1").is_err());
    let mut revoked_grant = spec.clone();
    revoked_grant.permission.push(PermissionItem {
        scope_path: APP_PERMISSION_USER_HOME.into(),
        required: true,
        actions: vec!["read".into()],
        exp: None,
    });
    assert!(validate_app_launch_config(&config, &revoked_grant, "ood1").is_err());
    let mut other = config.clone();
    other.node_execution_spec.app_instance_id =
        AppInstanceId::new(spec.app_id().clone(), "bob").unwrap();
    assert!(validate_app_launch_config(&other, &spec, "ood1").is_err());
}

#[test]
fn agent_recreation_keeps_instance_storage_and_requests_new_startup_credentials() {
    let doc = build_agent_doc_without_category();
    let spec = build_test_app_spec(doc.clone(), build_spec_config(&doc));
    let instance_id = spec.app_instance_id.to_string();
    let config = AppServiceInstanceConfig::new("ood1", &spec).unwrap();
    let loader = || {
        AppLoader::new_for_service(&instance_id, config.clone())
            .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
            .with_container_support_override(true)
            .with_paios_images_root(local_paios_root())
            .with_worker_image_repo_override("paios/aios")
    };
    let before = loader().preview_operation(ControlOperation::Start).unwrap();
    let after = loader().preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(before.commands[0].args, after.commands[0].args);
    assert_eq!(&after.commands[0].args[..2], &["rm", "-f"]);
    assert!(!after.commands[0].args.iter().any(|arg| arg == "-v"));
    let mounts = |command: &CommandSpec| {
        command.args.windows(2)
            .filter(|pair| pair[0] == "-v")
            .map(|pair| pair[1].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(mounts(&before.commands[1]), mounts(&after.commands[1]));
    assert!(mounts(&after.commands[1]).iter()
        .any(|mount| mount.contains(":/opt/buckyos/instance:rw")));
    assert!(after.commands[1].args.iter()
        .any(|arg| arg == "BUCKYOS_APP_TOKEN=<value>"));
}

fn build_agent_loader(platform: PlatformTarget) -> AppLoader {
    build_agent_loader_with_paios_root(platform, local_paios_root())
        .with_worker_image_repo_override("paios/aios")
}

fn build_agent_loader_with_paios_root(platform: PlatformTarget, root: &Path) -> AppLoader {
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
        .with_paios_images_root(root)
}

/// A BuckyOS root without bin/paios-images/ or etc/devenv.json: local image
/// mode with the default repos, independent of the machine running the tests.
fn local_paios_root() -> &'static Path {
    static ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| tempdir().unwrap()).path()
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
fn default_tmp_mount_uses_platform_valid_host_path() {
    let loader = build_service_loader(
        build_appservice_doc(),
        HashMap::new(),
        PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64),
        true,
    );

    let (host_path, permission) = loader.test_default_tmp_mount();
    let expected_root = if cfg!(target_os = "windows") {
        std::env::temp_dir()
    } else {
        std::path::PathBuf::from("/tmp")
    };

    assert_eq!(permission, "rw");
    assert!(host_path.is_absolute());
    assert_eq!(
        host_path,
        expected_root
            .join("buckyos")
            .join("alice")
            .join("demo.example")
    );
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
            "State": {"Running": true, "Health": {"Status": "healthy"}},
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
    assert_eq!(
        docker_deployment_health(&inspect.state),
        buckyos_api::DeploymentHealth::Healthy
    );
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
fn docker_health_distinguishes_unhealthy_and_starting_containers() {
    for (status, expected) in [
        ("unhealthy", buckyos_api::DeploymentHealth::Unhealthy),
        ("starting", buckyos_api::DeploymentHealth::Unknown),
    ] {
        let inspect = parse_docker_container_inspect(
            format!(
                r#"{{"State":{{"Running":true,"Health":{{"Status":"{status}"}}}},"Config":{{}},"Image":"sha256:deadbeef"}}"#
            )
            .as_str(),
        )
        .unwrap();
        assert_eq!(docker_deployment_health(&inspect.state), expected);
    }
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
fn extracted_oci_docker_layout_binds_config_and_load_identity() {
    let root = tempdir().unwrap();
    let config_hash = "5fccaf14085240ea2f3ef3e732bbddd5aad0a61934456012ed942a2f940b8557";
    let manifest_hash = "80aec7dbfa7cd9ad2e73352c7189c88c4752dc2dbdce487e9dfbd2e27f36176c";
    let blobs = root.path().join("blobs/sha256");
    fs::create_dir_all(blobs.as_path()).unwrap();
    fs::write(
        root.path().join("oci-layout"),
        r#"{"imageLayoutVersion":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("manifest.json"),
        format!(r#"[{{"Config":"blobs/sha256/{config_hash}","RepoTags":null,"Layers":[]}}]"#),
    )
    .unwrap();
    fs::write(blobs.join(config_hash), "{}").unwrap();
    fs::write(
        root.path().join("index.json"),
        format!(r#"{{"schemaVersion":2,"manifests":[{{"digest":"sha256:{manifest_hash}"}}]}}"#),
    )
    .unwrap();
    fs::write(
        blobs.join(manifest_hash),
        format!(r#"{{"config":{{"digest":"sha256:{config_hash}"}},"layers":[]}}"#),
    )
    .unwrap();

    let layout = inspect_docker_image_layout(root.path()).unwrap().unwrap();
    assert_eq!(layout.config_digest, format!("sha256:{config_hash}"));
    assert_eq!(
        layout.load_references,
        vec![
            format!("sha256:{manifest_hash}"),
            format!("sha256:{config_hash}")
        ]
    );
}

#[test]
fn extracted_oci_docker_layout_rejects_config_identity_mismatch() {
    let root = tempdir().unwrap();
    let config_hash = "5fccaf14085240ea2f3ef3e732bbddd5aad0a61934456012ed942a2f940b8557";
    let other_hash = "6fccaf14085240ea2f3ef3e732bbddd5aad0a61934456012ed942a2f940b8557";
    let manifest_hash = "80aec7dbfa7cd9ad2e73352c7189c88c4752dc2dbdce487e9dfbd2e27f36176c";
    let blobs = root.path().join("blobs/sha256");
    fs::create_dir_all(blobs.as_path()).unwrap();
    fs::write(
        root.path().join("oci-layout"),
        r#"{"imageLayoutVersion":"1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        root.path().join("manifest.json"),
        format!(r#"[{{"Config":"blobs/sha256/{config_hash}","RepoTags":null,"Layers":[]}}]"#),
    )
    .unwrap();
    fs::write(blobs.join(config_hash), "{}").unwrap();
    fs::write(
        root.path().join("index.json"),
        format!(r#"{{"schemaVersion":2,"manifests":[{{"digest":"sha256:{manifest_hash}"}}]}}"#),
    )
    .unwrap();
    fs::write(
        blobs.join(manifest_hash),
        format!(r#"{{"config":{{"digest":"sha256:{other_hash}"}},"layers":[]}}"#),
    )
    .unwrap();

    assert!(inspect_docker_image_layout(root.path()).is_err());
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
    let agent_env = Path::new("/opt/buckyos/data/home/alice/agents/jarvis-agent.example");
    let expected_root =
        Path::new("/opt/buckyos/env/pkgs/agent.jarvis-agent.example#pkg:1234567890");
    let exact_cmd = vec![
        "opendan".to_string(),
        "--app-id".to_string(),
        "jarvis-agent.example".to_string(),
        "--agent-bin".to_string(),
        expected_root.to_string_lossy().to_string(),
        "--service-port".to_string(),
        "4060".to_string(),
    ];
    let old_cmd = vec![
        "opendan".to_string(),
        "--app-id".to_string(),
        "jarvis-agent.example".to_string(),
        "--agent-bin".to_string(),
        "/opt/buckyos/env/pkgs/agent.jarvis-agent.example#pkg:oldversion".to_string(),
        "--service-port".to_string(),
        "4060".to_string(),
    ];

    assert!(command_matches_agent_process(
        &exact_cmd,
        "jarvis-agent.example"
    ));
    assert!(command_matches_agent_process(
        &old_cmd,
        "jarvis-agent.example"
    ));
    assert!(command_matches_exact_agent_process(
        &exact_cmd,
        "jarvis-agent.example",
        agent_env,
        Some(expected_root),
        Some("pkg:1234567890"),
    ));
    assert!(!command_matches_exact_agent_process(
        &old_cmd,
        "jarvis-agent.example",
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
        &[
            "pkg-install",
            "pkg-install",
            "docker",
            "docker",
            "docker",
            "docker",
        ],
    );
    assert_eq!(
        deploy.commands[0].args,
        vec![test_exact_package_id("agent.jarvis-agent.example", 3)]
    );
    assert_eq!(
        deploy.commands[1].args,
        vec![test_exact_package_id("skills.jarvis-agent.example", 4)]
    );
    // Local image mode never pulls: images must already be in the store.
    assert_eq!(
        deploy.commands[2].args,
        vec![
            "image",
            "inspect",
            "--format={{json .}}",
            "paios/aios:latest-amd64"
        ]
    );
    assert_eq!(
        deploy.commands[3].args,
        vec![
            "image",
            "inspect",
            "--format={{json .}}",
            "paios/exttool:latest-amd64"
        ]
    );
    assert_eq!(
        deploy.commands[4].args,
        vec!["volume", "create", "buckyos-exttool-<image id 12>"]
    );
    assert!(deploy.commands[5]
        .args
        .iter()
        .any(|arg| arg == "buckyos-exttool-<image id 12>:/opt/buckyos/tools"));
    assert!(!deploy
        .commands
        .iter()
        .any(|command| command.args.first().map(String::as_str) == Some("pull")));

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(start.runtime, RuntimeType::Agent);
    assert_programs(&start.commands, &["docker", "docker"]);
    let runtime_key = test_runtime_key("jarvis-agent.example");
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
        .contains(&"BUCKYOS_APP_ID=jarvis-agent.example".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_APP_TYPE=agent".to_string()));
    assert!(start.commands[1].args.contains(
        &"BUCKYOS_DATA_DIR=/opt/buckyos/data/home/alice/agents/jarvis-agent.example"
            .to_string(),
    ));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_PKG_DIR=/opt/buckyos/bin/jarvis-agent.example".to_string()));
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
    // ExtTool Volume (§6.1) of the current ExtTool image, mounted ro.
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg == "buckyos-exttool-<image id 12>:/opt/buckyos/tools:ro"));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_EXTTOOL_DIR=/opt/buckyos/tools".to_string()));
    assert!(start.commands[1]
        .args
        .iter()
        .any(|arg| arg
            == "<app_data>:/opt/buckyos/data/home/alice/agents/jarvis-agent.example:rw"));
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
    // Host Tool scripts are mounted read-only for the image's `buckyos`
    // launcher; the host Deno runtime is not.
    for entry in ["cli", "dist", "package.json", "distribution.json"] {
        let mount = format!("<buckyos_tool>/{entry}:/opt/buckyos/libexec/buckyos-tool/{entry}:ro");
        assert!(start.commands[1].args.contains(&mount), "missing {mount}");
    }
    assert!(!start.commands[1]
        .args
        .iter()
        .any(|arg| arg.contains("buckyos-tool/runtime")));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_TOOL_ENDPOINT=http://host.docker.internal:3180".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"<agent_skills_pkg>:/mnt/buckyos/agent_skills:ro".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"BUCKYOS_AGENT_SKILLS_PKG_DIR=/mnt/buckyos/agent_skills".to_string()));

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
        vec![
            "image",
            "inspect",
            "--format={{json .}}",
            "paios/aios_dev:latest-amd64"
        ]
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
        .with_worker_image_repo_override("paios/aios")
        .with_paios_images_root(local_paios_root());

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
        .any(|a| a == "buckyos-exttool-<image id 12>:/opt/buckyos/tools:ro"));
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
    assert!(!preview.commands[1]
        .args
        .iter()
        .any(|a| a.contains("buckyos-tool") || a.contains("agent_skills")));
    assert!(!preview.commands[1]
        .args
        .iter()
        .any(|a| a.starts_with("BUCKYOS_TOOL_ENDPOINT=")));
}

#[test]
fn buckyos_tool_mounts_skip_missing_entries_and_runtime() {
    let root = tempdir().unwrap();
    let tool_root = root.path().join("buckyos-tool");
    fs::create_dir_all(tool_root.join("cli")).unwrap();
    fs::create_dir_all(tool_root.join("dist")).unwrap();
    fs::create_dir_all(tool_root.join("runtime")).unwrap();
    fs::write(tool_root.join("package.json"), "{}").unwrap();

    let mounts = buckyos_tool_mounts(&tool_root);
    assert_eq!(
        mounts,
        ["cli", "dist", "package.json"]
            .iter()
            .map(|entry| (
                format!("/opt/buckyos/libexec/buckyos-tool/{entry}"),
                tool_root.join(entry)
            ))
            .collect::<Vec<_>>()
    );
    // The missing manifest is skipped, not created.
    assert!(!tool_root.join("distribution.json").exists());
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
fn host_script_deploy_preview_checks_local_images_without_pulling() {
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
        .with_worker_image_repo_override("paios/aios")
        .with_paios_images_root(local_paios_root());

    let preview = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(preview.runtime, RuntimeType::HostScript);
    assert_programs(
        &preview.commands,
        &["pkg-install", "docker", "docker", "docker", "docker"],
    );
    assert_eq!(preview.commands[1].args[..2], ["image", "inspect"]);
    assert_eq!(preview.commands[1].args[3], "paios/aios:latest-amd64");
    assert_eq!(preview.commands[2].args[3], "paios/exttool:latest-amd64");
    assert_eq!(
        preview.commands[3].args,
        vec!["volume", "create", "buckyos-exttool-<image id 12>"]
    );
}

#[test]
fn pinned_deploy_preview_loads_bundled_tarballs() {
    let root = tempdir().unwrap();
    let images = root.path().join("bin/paios-images");
    fs::create_dir_all(&images).unwrap();
    fs::write(
        images.join("paios_images.json"),
        include_str!("testdata/paios_images.v1.json"),
    )
    .unwrap();
    let loader = build_agent_loader_with_paios_root(
        PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64),
        root.path(),
    );

    let deploy = loader.preview_operation(ControlOperation::Deploy).unwrap();
    let docker = deploy
        .commands
        .iter()
        .filter(|command| command.program == "docker")
        .map(|command| command.args.clone())
        .collect::<Vec<_>>();
    let aios_tar = images.join("aios-amd64-7329ae7f4108.tar.gz");
    let exttool_tar = images.join("exttool-amd64-444444444444.tar.gz");
    assert_eq!(docker[0], vec!["load", "-i", aios_tar.to_str().unwrap()]);
    assert_eq!(docker[1], vec!["load", "-i", exttool_tar.to_str().unwrap()]);
    assert_eq!(
        docker[2],
        vec!["volume", "create", "buckyos-exttool-444444444444"]
    );
    assert!(docker[3].contains(&"paios/exttool:0.7.0-07787a750c1e-amd64".to_string()));

    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert!(start.commands[1]
        .args
        .contains(&"paios/aios:0.7.0-835b5bd44125-amd64".to_string()));
    assert!(start.commands[1]
        .args
        .contains(&"buckyos-exttool-444444444444:/opt/buckyos/tools:ro".to_string()));
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
        .with_worker_image_repo_override("paios/aios")
        .with_paios_images_root(local_paios_root());

    let preview = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(preview.commands[1].args[3], "paios/aios:latest-aarch64");
    assert_eq!(preview.commands[2].args[3], "paios/exttool:latest-aarch64");
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

#[test]
fn exttool_prepare_lock_is_process_wide_and_exclusive() {
    let guard = exttool_prepare_lock();
    assert!(
        std::ptr::eq(guard, exttool_prepare_lock()),
        "the ExtTool guard must be a single process-wide lock"
    );

    // Tests running in parallel take this lock while they prepare ExtTool, so
    // wait for it instead of assuming it is free.
    let held = guard.blocking_lock();
    assert!(
        guard.try_lock().is_err(),
        "a second caller must not enter while the ExtTool guard is held"
    );
    drop(held);
    // Re-acquiring proves the guard is released once the holder is done.
    drop(guard.blocking_lock());
}

const FAKE_EXTTOOL_ID: &str =
    "sha256:4c1a2e5b6d7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f70819";

#[derive(Clone)]
struct FakeDockerState {
    log: Arc<Mutex<Vec<String>>>,
    volumes: Arc<Mutex<Vec<String>>>,
}

fn fake_docker_runner(state: FakeDockerState) -> DockerCommandRunner {
    Arc::new(move |args: Vec<String>| {
        let state = state.clone();
        Box::pin(async move {
            state
                .log
                .lock()
                .unwrap()
                .push(format!("docker {}", args.join(" ")));
            fake_docker_outcome(&args, &state).await
        })
    })
}

fn ok_outcome(stdout: impl Into<String>) -> DockerExecOutcome {
    DockerExecOutcome {
        ok: true,
        stdout: stdout.into(),
        stderr: String::new(),
    }
}

async fn fake_docker_outcome(args: &[String], state: &FakeDockerState) -> DockerExecOutcome {
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    match args.as_slice() {
        ["volume", "ls", ..] => ok_outcome(state.volumes.lock().unwrap().join("\n")),
        ["volume", "create", name] => {
            state.volumes.lock().unwrap().push(name.to_string());
            ok_outcome(format!("{name}\n"))
        }
        ["image", "inspect", "--format={{json .}}", "paios/exttool:latest-amd64"] => {
            ok_outcome(serde_json::json!({ "Id": FAKE_EXTTOOL_ID }).to_string())
        }
        ["run", ..] => {
            tokio::time::sleep(Duration::from_millis(120)).await;
            ok_outcome("")
        }
        _ => panic!("fake docker received an unexpected command: {args:?}"),
    }
}

fn count_entries(entries: &[String], needle: &str) -> usize {
    entries
        .iter()
        .filter(|entry| entry.contains(needle))
        .count()
}

#[tokio::test]
async fn concurrent_prepare_exttool_volume_seeds_exactly_once_without_pulling() {
    let state = FakeDockerState {
        log: Arc::new(Mutex::new(Vec::new())),
        volumes: Arc::new(Mutex::new(Vec::new())),
    };
    let runner = fake_docker_runner(state.clone());

    let platform = PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64);
    let agent_loader = build_agent_loader(platform).with_docker_command_runner(runner.clone());
    let host_script_loader = AppLoader::new_for_local(
        "desktop-tool",
        LocalAppInstanceConfig {
            target_state: ServiceInstanceState::Started,
            enable: true,
            app_doc: build_local_service_doc(),
            user_id: "alice".to_string(),
            install_config: ServiceSpecConfig::default(),
        },
    )
    .with_platform(platform)
    .with_container_support_override(false)
    .with_paios_images_root(local_paios_root())
    .with_docker_command_runner(runner);

    let prepare_both = async {
        tokio::join!(
            agent_loader.prepare_exttool_volume(),
            host_script_loader.prepare_exttool_volume()
        )
    };
    let (agent_result, host_script_result) =
        tokio::time::timeout(Duration::from_secs(5), prepare_both)
            .await
            .expect("both prepares should finish promptly");
    let volume = "buckyos-exttool-4c1a2e5b6d7f";
    assert_eq!(agent_result.unwrap().as_deref(), Some(volume));
    assert_eq!(host_script_result.unwrap().as_deref(), Some(volume));

    let entries = state.log.lock().unwrap().clone();
    assert_eq!(count_entries(&entries, "volume ls"), 2, "{entries:?}");
    assert_eq!(
        count_entries(
            &entries,
            "image inspect --format={{json .}} paios/exttool:latest-amd64"
        ),
        2,
        "both callers must check the local ExtTool image: {entries:?}"
    );
    assert_eq!(count_entries(&entries, "pull"), 0, "{entries:?}");
    assert_eq!(
        count_entries(&entries, &format!("volume create {volume}")),
        1,
        "the volume must be created exactly once: {entries:?}"
    );
    assert_eq!(
        count_entries(
            &entries,
            "touch /opt/buckyos/tools/.buckyos-seeded-4c1a2e5b6d7f"
        ),
        1,
        "the volume seeding run must happen exactly once: {entries:?}"
    );
    assert_eq!(
        count_entries(&entries, "test -f /seed/.buckyos-seeded-4c1a2e5b6d7f"),
        1,
        "the second caller only checks the seed marker: {entries:?}"
    );
}

struct DockerObservationFixture {
    commands: Vec<Vec<String>>,
    container: Option<serde_json::Value>,
    repo_digests: Vec<String>,
    image_exists: bool,
    image_inspect_fails: bool,
}

fn docker_observation_fixture(
    image_id: &str,
) -> (
    AppServiceInstanceConfig,
    Arc<Mutex<DockerObservationFixture>>,
) {
    let app_doc = build_appservice_doc();
    let mut spec = build_test_app_spec(app_doc.clone(), build_spec_config(&app_doc));
    let platform = PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64);
    spec.packages
        .retain(|package| package_matches_platform(&package.sub_pkg_name, platform));
    let config = AppServiceInstanceConfig::new("ood1", &spec).unwrap();
    let desc = config
        .node_execution_spec
        .packages
        .get("amd64_docker_image")
        .unwrap();
    let labels = HashMap::from([
        (DOCKER_LABEL_PKG_ID, desc.pkg_id.clone()),
        (
            DOCKER_LABEL_PKG_OBJID,
            desc.pkg_objid.as_ref().unwrap().to_string(),
        ),
        (DOCKER_LABEL_IMAGE_DIGEST, "sha256:deadbeef".to_string()),
        (
            DOCKER_LABEL_APP_DOC_OBJECT_ID,
            config.deployment.app_doc_object_id.to_string(),
        ),
        (
            DOCKER_LABEL_SPEC_GENERATION,
            config.deployment.spec_generation.to_string(),
        ),
    ]);
    let container = serde_json::json!({
        "Id": "container-one",
        "Image": image_id,
        "State": { "Running": true, "Health": { "Status": "healthy" } },
        "Config": { "Labels": labels },
    });
    (
        config,
        Arc::new(Mutex::new(DockerObservationFixture {
            commands: Vec::new(),
            container: Some(container),
            repo_digests: vec!["demo/service@sha256:deadbeef".to_string()],
            image_exists: true,
            image_inspect_fails: false,
        })),
    )
}

fn docker_observation_loader(
    config: &AppServiceInstanceConfig,
    fixture: &Arc<Mutex<DockerObservationFixture>>,
) -> AppLoader {
    let fixture = fixture.clone();
    let runner: DockerCommandRunner = Arc::new(move |args| {
        let fixture = fixture.clone();
        Box::pin(async move {
            let mut fixture = fixture.lock().unwrap();
            fixture.commands.push(args.clone());
            let stdout = match args
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .as_slice()
            {
                ["container", "inspect", ..] => {
                    let Some(container) = fixture.container.as_ref() else {
                        return DockerExecOutcome {
                            ok: false,
                            stdout: String::new(),
                            stderr: "No such container".to_string(),
                        };
                    };
                    container.to_string()
                }
                ["container", "ls", ..] => String::new(),
                ["image", "inspect", "--format={{json .RepoDigests}}", ..] => {
                    if fixture.image_inspect_fails {
                        return DockerExecOutcome {
                            ok: false,
                            stdout: String::new(),
                            stderr: "docker daemon unavailable".to_string(),
                        };
                    }
                    serde_json::to_string(&fixture.repo_digests).unwrap()
                }
                ["images", "-q", ..] => if fixture.image_exists {
                    "image-present"
                } else {
                    ""
                }
                .to_string(),
                _ => panic!("unexpected docker observation command: {args:?}"),
            };
            DockerExecOutcome {
                ok: true,
                stdout,
                stderr: String::new(),
            }
        })
    });
    AppLoader::new_for_service(
        &config.node_execution_spec.app_instance_id.to_string(),
        config.clone(),
    )
    .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
    .with_container_support_override(true)
    .with_docker_command_runner(runner)
}

#[tokio::test]
async fn deployment_observation_reads_state_health_and_identity_with_one_inspect() {
    let (config, fixture) = docker_observation_fixture("sha256:observation-label-match");
    let loader = docker_observation_loader(&config, &fixture);
    for (running, health, state, expected_health) in [
        (
            true,
            "healthy",
            ServiceInstanceState::Started,
            buckyos_api::DeploymentHealth::Healthy,
        ),
        (
            true,
            "unhealthy",
            ServiceInstanceState::Started,
            buckyos_api::DeploymentHealth::Unhealthy,
        ),
        (
            true,
            "starting",
            ServiceInstanceState::Started,
            buckyos_api::DeploymentHealth::Unknown,
        ),
        (
            false,
            "healthy",
            ServiceInstanceState::Exited,
            buckyos_api::DeploymentHealth::Unhealthy,
        ),
    ] {
        {
            let mut fixture = fixture.lock().unwrap();
            fixture.commands.clear();
            let container = fixture.container.as_mut().unwrap();
            container["State"]["Running"] = serde_json::json!(running);
            container["State"]["Health"]["Status"] = serde_json::json!(health);
        }
        let observation = loader.observe_deployment().await.unwrap();
        assert_eq!(observation.state, state);
        assert_eq!(observation.health, expected_health);
        assert_eq!(observation.container_id.as_deref(), Some("container-one"));
        assert_eq!(
            observation.identity.unwrap().image_id.as_deref(),
            Some("sha256:observation-label-match")
        );
        let fixture = fixture.lock().unwrap();
        assert_eq!(fixture.commands.len(), 1, "{:?}", fixture.commands);
        assert_eq!(&fixture.commands[0][..2], ["container", "inspect"]);
    }
}

#[tokio::test]
async fn deployment_observation_uses_container_image_id_without_image_inspect() {
    let (config, fixture) = docker_observation_fixture("sha256:deadbeef");
    fixture.lock().unwrap().container.as_mut().unwrap()["Config"]["Labels"]
        .as_object_mut()
        .unwrap()
        .remove(DOCKER_LABEL_IMAGE_DIGEST);
    let observation = docker_observation_loader(&config, &fixture)
        .observe_deployment()
        .await
        .unwrap();
    assert_eq!(observation.state, ServiceInstanceState::Started);
    assert_eq!(fixture.lock().unwrap().commands.len(), 1);
}

#[tokio::test]
async fn deployment_observation_caches_digest_across_loaders_and_refreshes_changed_image() {
    let (config, fixture) = docker_observation_fixture("sha256:observation-cached-image-one");
    fixture.lock().unwrap().container.as_mut().unwrap()["Config"]["Labels"]
        .as_object_mut()
        .unwrap()
        .remove(DOCKER_LABEL_IMAGE_DIGEST);
    for _ in 0..2 {
        let observation = docker_observation_loader(&config, &fixture)
            .observe_deployment()
            .await
            .unwrap();
        assert_eq!(observation.state, ServiceInstanceState::Started);
    }
    {
        let mut fixture = fixture.lock().unwrap();
        assert_eq!(fixture.commands.len(), 3, "{:?}", fixture.commands);
        let container = fixture.container.as_mut().unwrap();
        container["Id"] = serde_json::json!("container-two");
        container["Image"] = serde_json::json!("sha256:observation-cached-image-two");
    }
    let observation = docker_observation_loader(&config, &fixture)
        .observe_deployment()
        .await
        .unwrap();
    assert_eq!(observation.state, ServiceInstanceState::Started);
    assert_eq!(observation.container_id.as_deref(), Some("container-two"));
    let fixture = fixture.lock().unwrap();
    assert_eq!(fixture.commands.len(), 5, "{:?}", fixture.commands);
    assert_eq!(
        fixture.commands[4].last().unwrap(),
        "sha256:observation-cached-image-two"
    );
}

#[tokio::test]
async fn deployment_observation_rejects_stale_generation_without_inspecting_its_image() {
    let (config, fixture) = docker_observation_fixture("sha256:observation-stale-generation");
    {
        let mut fixture = fixture.lock().unwrap();
        fixture.image_exists = false;
        let labels = &mut fixture.container.as_mut().unwrap()["Config"]["Labels"];
        labels
            .as_object_mut()
            .unwrap()
            .remove(DOCKER_LABEL_IMAGE_DIGEST);
        labels[DOCKER_LABEL_SPEC_GENERATION] = serde_json::json!("0");
    }
    let observation = docker_observation_loader(&config, &fixture)
        .observe_deployment()
        .await
        .unwrap();
    assert_eq!(observation.state, ServiceInstanceState::NotExist);
    assert_eq!(observation.health, buckyos_api::DeploymentHealth::Unknown);
    let fixture = fixture.lock().unwrap();
    assert_eq!(fixture.commands.len(), 2, "{:?}", fixture.commands);
    assert_eq!(fixture.commands[1][0], "images");
}

#[tokio::test]
async fn deployment_observation_missing_container_preserves_installed_image_state() {
    let (config, fixture) = docker_observation_fixture("sha256:observation-missing");
    fixture.lock().unwrap().container = None;
    let observation = docker_observation_loader(&config, &fixture)
        .observe_deployment()
        .await
        .unwrap();
    assert_eq!(observation.state, ServiceInstanceState::Stopped);
    assert_eq!(observation.health, buckyos_api::DeploymentHealth::Unknown);
    assert!(observation.identity.is_none());
    fixture.lock().unwrap().image_exists = false;
    assert_eq!(
        docker_observation_loader(&config, &fixture)
            .observe_deployment()
            .await
            .unwrap()
            .state,
        ServiceInstanceState::NotExist
    );
}

#[tokio::test]
async fn deployment_observation_retries_digest_inspect_after_failure() {
    let (config, fixture) = docker_observation_fixture("sha256:observation-inspect-retry");
    {
        let mut fixture = fixture.lock().unwrap();
        fixture.image_inspect_fails = true;
        fixture.container.as_mut().unwrap()["Config"]["Labels"]
            .as_object_mut()
            .unwrap()
            .remove(DOCKER_LABEL_IMAGE_DIGEST);
    }
    let loader = docker_observation_loader(&config, &fixture);
    assert!(loader.observe_deployment().await.is_err());
    fixture.lock().unwrap().image_inspect_fails = false;
    assert_eq!(
        loader.observe_deployment().await.unwrap().state,
        ServiceInstanceState::Started
    );
    assert_eq!(fixture.lock().unwrap().commands.len(), 4);
}

#[tokio::test]
async fn deployment_observation_refreshes_cached_metadata_for_new_digest() {
    let (mut config, fixture) = docker_observation_fixture("sha256:observation-new-digest");
    fixture.lock().unwrap().container.as_mut().unwrap()["Config"]["Labels"]
        .as_object_mut()
        .unwrap()
        .remove(DOCKER_LABEL_IMAGE_DIGEST);
    assert_eq!(
        docker_observation_loader(&config, &fixture)
            .observe_deployment()
            .await
            .unwrap()
            .state,
        ServiceInstanceState::Started
    );
    config
        .node_execution_spec
        .packages
        .get_mut("amd64_docker_image")
        .unwrap()
        .docker_image_digest = Some("sha256:new-digest".to_string());
    fixture.lock().unwrap().repo_digests = vec!["demo/service@sha256:new-digest".to_string()];
    assert_eq!(
        docker_observation_loader(&config, &fixture)
            .observe_deployment()
            .await
            .unwrap()
            .state,
        ServiceInstanceState::Started
    );
    assert_eq!(fixture.lock().unwrap().commands.len(), 4);
}

#[tokio::test]
async fn deployment_observation_does_not_cache_missing_digest_metadata() {
    let (config, fixture) = docker_observation_fixture("sha256:observation-empty-digests");
    {
        let mut fixture = fixture.lock().unwrap();
        fixture.container.as_mut().unwrap()["Config"]["Labels"]
            .as_object_mut()
            .unwrap()
            .remove(DOCKER_LABEL_IMAGE_DIGEST);
        fixture.repo_digests.clear();
        fixture.image_exists = false;
    }
    let loader = docker_observation_loader(&config, &fixture);
    assert_eq!(
        loader.observe_deployment().await.unwrap().state,
        ServiceInstanceState::NotExist
    );
    fixture.lock().unwrap().repo_digests = vec!["demo/service@sha256:deadbeef".to_string()];
    assert_eq!(
        loader.observe_deployment().await.unwrap().state,
        ServiceInstanceState::Started
    );
    assert_eq!(fixture.lock().unwrap().commands.len(), 5);
}

fn build_native_agent_loader(support_container: bool, state_root: &Path, program: &Path) -> AppLoader {
    let app_doc = build_agent_doc_without_category();
    let install_config = build_spec_config(&app_doc);
    let app_spec = build_test_app_spec(app_doc, install_config);
    let app_instance_id = app_spec.app_instance_id.to_string();
    let mut config = AppServiceInstanceConfig::new("ood1", &app_spec).unwrap();
    config.service_ports_config =
        HashMap::from([("www".to_string(), 10080), ("main".to_string(), 14060)]);
    config.run_mode = buckyos_api::AppRunMode::Native;
    AppLoader::new_for_service(&app_instance_id, config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(support_container)
        .with_paios_images_root(local_paios_root())
        .with_native_agent(program, state_root)
}

/// A natively run Agent (the zone owner's Agent on its host-native
/// runtime) needs no worker image and no container support: deploy only
/// installs its packages, start runs the node's own OpenDAN.
#[test]
fn a_native_agent_runs_the_nodes_opendan_without_containers() {
    let state = tempdir().unwrap();
    for support_container in [true, false] {
        let loader = build_native_agent_loader(support_container, state.path(), Path::new("/opt/buckyos/bin/opendan/opendan"));
        let deploy = loader.preview_operation(ControlOperation::Deploy).unwrap();
        assert_eq!(deploy.runtime, RuntimeType::NativeAgent);
        assert!(deploy.commands.iter().all(|c| c.program == "pkg-install"), "{:?}", deploy.commands);
        assert!(!deploy.commands.is_empty());
        let start = loader.preview_operation(ControlOperation::Start).unwrap();
        assert_eq!(start.runtime, RuntimeType::NativeAgent);
        assert_eq!(start.commands.len(), 1);
        assert_eq!(start.commands[0].program, "/opt/buckyos/bin/opendan/opendan");
        let args = &start.commands[0].args;
        assert_eq!(args[0], "--appid");
        assert!(args.windows(2).any(|w| w[0] == "--service-port" && w[1] == "14060"), "{args:?}");
        assert!(!start.commands.iter().any(|c| c.program == "docker"));
    }
}

/// The native process is found again from its record (also by a new
/// node_daemon), and stopping it ends the process and forgets it.
#[tokio::test]
async fn a_native_agent_process_is_recorded_found_and_stopped() {
    let state = tempdir().unwrap();
    let loader = build_native_agent_loader(false, state.path(), Path::new("/bin/sleep"));
    assert_eq!(loader.test_running_native_agent_pid(), None);
    loader.test_spawn_native_agent(&["30".to_string()], state.path()).unwrap();
    let pid = loader.test_running_native_agent_pid().expect("the recorded process runs");
    let again = build_native_agent_loader(false, state.path(), Path::new("/bin/sleep"));
    assert_eq!(again.test_running_native_agent_pid(), Some(pid), "found from the record");
    again.test_stop_native_agent().await.unwrap();
    for _ in 0..50 {
        crate::service_pkg::reap_detached_children();
        if crate::service_pkg::read_process_identity(pid).is_none() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(crate::service_pkg::read_process_identity(pid).is_none(), "the process ended");
    assert_eq!(loader.test_running_native_agent_pid(), None);
    assert!(fs::read_dir(state.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".json")));
}

fn lite_product() -> buckyos_api::ProductProfile {
    buckyos_api::ProductProfile::parse(include_str!("../../../product/lite.json")).unwrap()
}

fn build_systest_doc() -> AppDoc {
    let owner = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(AppType::AppService, "buckyos-systest", "0.5.1", "did:bns:buckyos", &owner)
        .app_did(DID::from_str("did:bns:buckyos-systest.buckyos").unwrap())
        .script_pkg(
            SubPkgDesc::new("all.script.buckyos-systest.buckyos.bns.did#0.5.1")
                .package_meta_object_id(test_package_object_id(31)),
        )
        .service_port("www", 3000)
        .build()
        .unwrap()
}

fn build_native_script_loader(app_doc: AppDoc, state_root: &Path) -> AppLoader {
    let install_config = build_spec_config(&app_doc);
    let app_spec = build_test_app_spec(app_doc, install_config);
    let app_instance_id = app_spec.app_instance_id.to_string();
    let mut config = AppServiceInstanceConfig::new("ood1", &app_spec).unwrap();
    config.service_ports_config = HashMap::from([("www".to_string(), 10048)]);
    config.run_mode = buckyos_api::AppRunMode::Native;
    AppLoader::new_for_service(&app_instance_id, config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_product(lite_product())
        .with_native_script("/opt/buckyos/libexec/buckyos-tool/runtime/deno", state_root)
}

/// BuckyOS Desktop Lite never runs a container: docker apps, the worker
/// container of a script or local app and the Agent container are refused
/// before any docker/wslc command, even where the device claims containers.
#[test]
fn lite_refuses_every_container_runtime() {
    let platform = PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64);
    let docker = build_service_loader(build_appservice_doc(), HashMap::new(), platform, true)
        .with_product(lite_product());
    let agent = build_agent_loader(platform).with_product(lite_product());
    let local = AppLoader::new_for_local(
        "desktop-tool",
        LocalAppInstanceConfig {
            target_state: ServiceInstanceState::Started,
            enable: true,
            app_doc: build_local_service_doc(),
            user_id: "alice".to_string(),
            install_config: ServiceSpecConfig::default(),
        },
    )
    .with_platform(platform)
    .with_container_support_override(true)
    .with_product(lite_product());
    // The preinstalled script app, but scheduled into a worker container.
    let state = tempdir().unwrap();
    let systest_in_container = {
        let app_doc = build_systest_doc();
        let install_config = build_spec_config(&app_doc);
        let app_spec = build_test_app_spec(app_doc, install_config);
        let config = AppServiceInstanceConfig::new("ood1", &app_spec).unwrap();
        AppLoader::new_for_service(&app_spec.app_instance_id.to_string(), config)
            .with_platform(platform)
            .with_container_support_override(true)
            .with_product(lite_product())
            .with_native_script("/deno", state.path())
    };
    for loader in [docker, agent, local, systest_in_container] {
        for operation in [ControlOperation::Deploy, ControlOperation::Start, ControlOperation::Status] {
            assert!(
                matches!(loader.preview_operation(operation), Err(ControlRuntItemErrors::NotSupport(_))),
                "{operation:?}"
            );
        }
    }
}

/// A preinstalled script app run natively: deploy only installs its
/// package, start runs the node's Deno with the manifest's permissions on
/// the package entry; nothing touches docker.
#[test]
fn a_preinstalled_script_app_runs_with_the_nodes_deno() {
    let state = tempdir().unwrap();
    let loader = build_native_script_loader(build_systest_doc(), state.path());
    let deploy = loader.preview_operation(ControlOperation::Deploy).unwrap();
    assert_eq!(deploy.runtime, RuntimeType::NativeScript);
    assert_programs(&deploy.commands, &["pkg-install"]);
    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(start.runtime, RuntimeType::NativeScript);
    assert_eq!(start.commands.len(), 1);
    assert_eq!(start.commands[0].program, "/opt/buckyos/libexec/buckyos-tool/runtime/deno");
    let args = &start.commands[0].args;
    assert_eq!(&args[..2], ["run", "--no-prompt"]);
    assert!(args.contains(&"--allow-net".to_string()));
    assert!(args.last().unwrap().ends_with("main.ts"), "{args:?}");
    assert!(start.commands.iter().all(|c| c.program != "docker"));
}

/// A script app learns where to listen from `BUCKYOS_SERVICE_PORT`: the port
/// of its `www` service, or of its only service.
#[test]
fn a_script_app_is_handed_the_port_of_its_primary_service() {
    assert_eq!(primary_service_port([("api", 9001), ("www", 9000)]), Some(9000));
    assert_eq!(primary_service_port([("api", 9001)]), Some(9001));
    assert_eq!(primary_service_port([("api", 9001), ("admin", 9002)]), None);
    assert_eq!(primary_service_port(std::iter::empty()), None);
}

/// In a container the script app is handed the AppDoc inner port it listens
/// on there (Docker publishes the instance port onto it).
#[test]
fn a_script_app_in_a_container_is_handed_its_inner_port() {
    let app_doc = build_systest_doc();
    let install_config = build_spec_config(&app_doc);
    let app_spec = build_test_app_spec(app_doc, install_config);
    let mut config = AppServiceInstanceConfig::new("ood1", &app_spec).unwrap();
    config.service_ports_config = HashMap::from([("www".to_string(), 10032)]);
    let loader = AppLoader::new_for_service(&app_spec.app_instance_id.to_string(), config)
        .with_platform(PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64))
        .with_container_support_override(true)
        .with_worker_image_repo_override("paios/aios")
        .with_paios_images_root(local_paios_root());
    let start = loader.preview_operation(ControlOperation::Start).unwrap();
    assert_eq!(start.runtime, RuntimeType::HostScript);
    let run = start.commands.iter().find(|c| c.args.contains(&"run".to_string())).unwrap();
    assert!(run.args.contains(&"BUCKYOS_SERVICE_PORT=3000".to_string()), "{:?}", run.args);
}

/// Running natively is only for the product's own script apps.
#[test]
fn a_script_app_outside_the_product_manifest_does_not_run_natively() {
    let state = tempdir().unwrap();
    let owner = DID::from_str("did:bns:test").unwrap();
    let other = AppDoc::builder(AppType::AppService, "my-tool", "1.0.0", "did:bns:test", &owner)
        .app_did(DID::from_str("did:web:my-tool.example").unwrap())
        .script_pkg(SubPkgDesc::new("all.script.my-tool.example#1.0.0").package_meta_object_id(test_package_object_id(32)))
        .service_port("www", 3000)
        .build()
        .unwrap();
    let loader = build_native_script_loader(other, state.path());
    assert!(matches!(
        loader.preview_operation(ControlOperation::Start),
        Err(ControlRuntItemErrors::NotSupport(_))
    ));
}

/// What this edition cannot run does not exist on the node: a stopped
/// instance needs nothing done (no error every round), the reason is reported.
#[tokio::test]
async fn an_unsupported_runtime_is_observed_as_not_existing_with_its_reason() {
    let platform = PlatformTarget::new(PlatformOs::Linux, PlatformArch::Amd64);
    let docker = build_service_loader(build_appservice_doc(), HashMap::new(), platform, true)
        .with_product(lite_product());
    let observed = docker.observe_deployment().await.unwrap();
    assert_eq!(observed.state, ServiceInstanceState::NotExist);
    assert_eq!(observed.deployment_error.unwrap().code, "not_supported");
}
