use anyhow::{anyhow, Result};
use buckyos_api::{
    AppDoc, AppServiceSpec, GatewaySettings, GatewayShortcut, KernelServiceDoc, KernelServiceSpec,
    NodeConfig, NodeState, ServiceInfo, ServiceInstallConfig, ServiceInstanceReportInfo,
    ServiceInstanceState, ServiceNode, ServiceState, UserSettings, UserState, UserType,
};
use jsonwebtoken::jwk::Jwk;
use name_lib::{DID, OwnerConfig, VerifyHubInfo, ZoneBootConfig, ZoneConfig};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::TryFrom;

const DEFAULT_OOD_ID: &str = "ood1";

#[derive(Debug, Deserialize)]
pub struct StartConfigSummary {
    pub user_name: String,
    pub admin_password_hash: String,
    pub public_key: Jwk,
    #[serde(default)]
    pub ood_jwt: Option<String>,
}


pub struct SystemConfigBuilder {
    entries: HashMap<String, String>,
}

impl SystemConfigBuilder {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    pub fn add_default_accounts(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let root_settings = UserSettings {
            user_type: UserType::Root,
            username: config.user_name.clone(),
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
        };
        self.insert_json("users/root/settings", &root_settings)?;

        let admin_key = format!("users/{}/settings", config.user_name);
        let admin_settings = UserSettings {
            user_type: UserType::Admin,
            username: config.user_name.clone(),
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
        };
        self.insert_json(&admin_key, &admin_settings)?;
        Ok(self)
    }

    pub fn add_user_doc(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let user_did = DID::new("bns", &config.user_name);
        let owner_jwk = config.public_key.clone();

        let owner_config = OwnerConfig::new(
            user_did,
            config.user_name.clone(),
            config.user_name.clone(),
            owner_jwk,
        );

        let key = format!("users/{}/doc", config.user_name);
        self.insert_json(&key, &owner_config)?;
        Ok(self)
    }

    pub fn add_default_apps(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let app_key = format!(
            "users/{}/apps/buckyos-filebrowser/config",
            config.user_name
        );
        let app_doc = build_filebrowser_app_doc()
            .map_err(|err| anyhow!("failed to build default app doc: {err}"))?;

        let mut install_config = ServiceInstallConfig::default();
        install_config.data_mount_point = HashMap::from([
            ("/srv/".to_string(), "home/".to_string()),
            (
                "/database/".to_string(),
                "buckyos-filebrowser/database/".to_string(),
            ),
            (
                "/config/".to_string(),
                "buckyos-filebrowser/config/".to_string(),
            ),
        ]);
        install_config.service_ports = HashMap::from([("www".to_string(), 80)]);

        let app_config = AppServiceSpec {
            app_doc,
            app_index: 1,
            user_id: config.user_name.clone(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Stopped,
            install_config,
        };

        self.insert_json(&app_key, &app_config)?;
        Ok(self)
    }

    pub fn add_device_doc(
        &mut self,
        config: &StartConfigSummary,
    ) -> Result<&mut Self> {
        let ood_jwt = config
            .ood_jwt
            .as_ref()
            .ok_or_else(|| anyhow!("start_config.json missing ood_jwt"))?;
        self.entries
            .insert(format!("devices/{}/doc", DEFAULT_OOD_ID), ood_jwt.clone());
        Ok(self)
    }

    pub fn add_system_defaults(&mut self) -> Result<&mut Self> {
        self.insert_json("system/system_pkgs", &json!({}))?;
        Ok(self)
    }

    pub fn add_verify_hub_entries(
        &mut self,
        verify_hub_private_key: &str,
    ) -> Result<&mut Self> {
        self.entries
            .insert("system/verify-hub/key".into(), verify_hub_private_key.to_string());

        let config = build_kernel_service_spec(
            "verify_hub",
            "Verify Hub",
            "verify hub is SSO service of buckyos",
            "kernel_service",
            "single",
            3300,
            1,
            ServiceState::Running,
        )?;
        self.insert_json("services/verify-hub/config", &config)?;

        let settings = VerifyHubSettings {
            trust_keys: vec![],
        };
        self.insert_json("services/verify-hub/settings", &settings)?;

        let info = ServiceInfo {
            selector_type: "random".to_string(),
            node_list: HashMap::from([(
                DEFAULT_OOD_ID.to_string(),
                ServiceNode {
                    node_did: "".to_string(),
                    node_net_id: None,
                    state: ServiceInstanceState::Started,
                    weight: 100,
                    service_port: HashMap::from([("main".to_string(), 3300)]),
                },
            )]),
        };
        self.insert_json("services/verify-hub/info", &info)?;

        let instance = ServiceInstanceReportInfo {
            instance_id: format!("verify-hub-{}", DEFAULT_OOD_ID),
            state: ServiceInstanceState::Started,
            service_ports: HashMap::from([("main".to_string(), 3300)]),
            last_update_time: 0,
            start_time: 0,
            pid: 0,
        };
        self.insert_json(&format!("services/verify-hub/instances/{}", DEFAULT_OOD_ID), &instance)?;
        
        Ok(self)
    }

    pub fn add_scheduler_service(&mut self) -> Result<&mut Self> {
        let config = build_kernel_service_spec(
            "scheduler",
            "Scheduler",
            "scheduler is the core service of buckyos",
            "kernel_service",
            "single",
            3400,
            1,
            ServiceState::Running,
        )?;
        self.insert_json("services/scheduler/config", &config)?;
        Ok(self)
    }

    pub fn add_gateway_settings(
        &mut self,
        config: &StartConfigSummary,
    ) -> Result<&mut Self> {
        let settings = GatewaySettings {
            shortcuts: HashMap::from([
                (
                    "www".to_string(),
                    GatewayShortcut {
                        target_type: "app".to_string(),
                        user_id: Some(config.user_name.clone()),
                        app_id: "buckyos-filebrowser".to_string(),
                    },
                ),
                (
                    "sys".to_string(),
                    GatewayShortcut {
                        target_type: "app".to_string(),
                        user_id: Some(config.user_name.clone()),
                        app_id: "control-panel".to_string(),
                    },
                ),
                (
                    "sys_test".to_string(),
                    GatewayShortcut {
                        target_type: "app".to_string(),
                        user_id: Some(config.user_name.clone()),
                        app_id: "sys-test".to_string(),
                    },
                ),
            ]),
        };
        self.insert_json("services/gateway/settings", &settings)?;
        Ok(self)
    }

    pub fn add_repo_service_entries(&mut self) -> Result<&mut Self> {
        let config = build_kernel_service_spec(
            "repo_service",
            "Repo Service",
            "repo service is the repo service of buckyos",
            "frame_service",
            "single",
            4000,
            1,
            ServiceState::Running,
        )?;
        self.insert_json("services/repo-service/config", &config)?;

        let settings = RepoServiceSettings {
            remote_source: HashMap::from([
                ("root".to_string(), "https://buckyos.ai/ndn/repo/meta_index.db".to_string())
            ]),
            enable_dev_mode: true,
        };
        self.insert_json("services/repo-service/settings", &settings)?;

        let pkg_list = HashMap::from([
            ("nightly-linux-amd64.node_daemon".to_string(), "no".to_string()),
            ("nightly-linux-aarch64.node_daemon".to_string(), "no".to_string()),
            ("nightly-windows-amd64.node_daemon".to_string(), "no".to_string()),
            ("nightly-apple-amd64.node_daemon".to_string(), "no".to_string()),
            ("nightly-apple-aarch64.node_daemon".to_string(), "no".to_string()),
            ("nightly-linux-amd64.buckycli".to_string(), "no".to_string()),
            ("nightly-linux-aarch64.buckycli".to_string(), "no".to_string()),
            ("nightly-windows-amd64.buckycli".to_string(), "no".to_string()),
            ("nightly-apple-amd64.buckycli".to_string(), "no".to_string()),
            ("nightly-apple-aarch64.buckycli".to_string(), "no".to_string()),
        ]);
        self.insert_json("services/repo-service/pkg_list", &pkg_list)?;
        Ok(self)
    }

    pub fn add_smb_service(&mut self) -> Result<&mut Self> {
        let config = build_kernel_service_spec(
            "smb_service",
            "SMB Service",
            "smb-service is the samba service of buckyos",
            "frame_service",
            "single",
            4100,
            1,
            ServiceState::Running,
        )?;
        self.insert_json("services/smb-service/config", &config)?;
        Ok(self)
    }

    pub fn add_node_defaults(&mut self) -> Result<&mut Self> {
        let config = NodeConfig {
            kernel: HashMap::new(),
            apps: HashMap::new(),
            frame_services: HashMap::new(),
            state: NodeState::Running,
        };
        self.insert_json(&format!("nodes/{}/config", DEFAULT_OOD_ID), &config)?;

        let gateway_config = json!({
            "servers": {
                "zone_gateway": {
                    "type": "cyfs_warp",
                    "bind": "0.0.0.0",
                    "http_port": 80,
                    "hosts": {}
                }
            },
            "dispatcher": {
                "tcp://0.0.0.0:80": {
                    "type": "server",
                    "id": "zone_gateway"
                },
                "tcp://0.0.0.0:443": {
                    "type": "server",
                    "id": "zone_gateway"
                }
            },
            "inner_services": {}
        });
        self.insert_json(&format!("nodes/{}/gateway_config", DEFAULT_OOD_ID), &gateway_config)?;
        Ok(self)
    }

    pub fn add_boot_config(
        &mut self,
        config: &StartConfigSummary,
        verify_hub_public_key: &Jwk,
        zone_boot_config: &ZoneBootConfig,
    ) -> Result<&mut Self> {
        let public_key_value = verify_hub_public_key.clone();
        let mut zone_config = ZoneConfig::new(DID::new("bns", &config.user_name), DID::new("bns", &config.user_name), config.public_key.clone());

        let verify_hub_info = VerifyHubInfo {
            public_key: public_key_value,
            port: 3300,
            node_name: DEFAULT_OOD_ID.to_string(),
        };
        zone_config.init_by_boot_config(zone_boot_config);
        zone_config.verify_hub_info = Some(verify_hub_info);

        self.insert_json("boot/config", &zone_config)?;
        Ok(self)
    }

    pub fn build(self) -> HashMap<String, String> {
        self.entries
    }

    fn insert_json<T: ?Sized + serde::Serialize>(
        &mut self,
        key: &str,
        value: &T,
    ) -> Result<()> {
        let content = serde_json::to_string_pretty(value)?;
        self.entries.insert(key.to_string(), content);
        Ok(())
    }
}

fn build_filebrowser_app_doc() -> Result<AppDoc> {
    let doc_value = json!({
        "pkg_name": "buckyos-filebrowser",
        "version": "0.4.0",
        "tag": "latest",
        "app_name": "BuckyOS File Browser",
        "description": {
            "detail": "BuckyOS File Browser"
        },
        "author": "did:web:buckyos.ai",
        "pub_time": 1743008063u64,
        "exp": 1837616063u64,
        "selector_type": "single",
        "install_config_tips": {
            "data_mount_point": ["/srv/", "/database/", "/config/"],
            "local_cache_mount_point": [],
            "service_ports": {
                "www": 80
            },
            "container_param": serde_json::Value::Null
        },
        "pkg_list": {
            "amd64_docker_image": {
                "pkg_id": "nightly-linux-amd64.buckyos-filebrowser-img#0.4.1",
                "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-amd64"
            },
            "aarch64_docker_image": {
                "pkg_id": "nightly-linux-aarch64.buckyos-filebrowser-img#0.4.1",
                "docker_image_name": "buckyos/nightly-buckyos-filebrowser:0.4.1-aarch64"
            },
            "amd64_win_app": {
                "pkg_id": "nightly-windows-amd64.buckyos-filebrowser-bin#0.4.1"
            },
            "aarch64_apple_app": {
                "pkg_id": "nightly-apple-aarch64.buckyos-filebrowser-bin#0.4.1"
            },
            "amd64_apple_app": {
                "pkg_id": "nightly-apple-amd64.buckyos-filebrowser-bin#0.4.1"
            }
        },
        "deps": {
            "nightly-linux-amd64.buckyos-filebrowser-img": "0.4.1",
            "nightly-linux-aarch64.buckyos-filebrowser-img": "0.4.1",
            "nightly-windows-amd64.buckyos-filebrowser-bin": "0.4.1",
            "nightly-apple-amd64.buckyos-filebrowser-bin": "0.4.1",
            "nightly-apple-aarch64.buckyos-filebrowser-bin": "0.4.1"
        }
    });
    serde_json::from_value(doc_value)
        .map_err(|err| anyhow!("failed to deserialize default filebrowser AppDoc: {err}"))
}

fn build_kernel_service_doc(
    pkg_name: &str,
    display_name: &str,
    description: &str,
    category: &str,
    selector_type: &str,
) -> Result<KernelServiceDoc> {
    let doc_value = json!({
        "pkg_name": pkg_name,
        "version": "0.4.0",
        "tag": "latest",
        "author": "did:bns:buckyos",
        "description": {
            "detail": description
        },
        "category": category,
        "pub_time": 1743008063u64,
        "name": display_name,
        "selector_type": selector_type
    });
    serde_json::from_value(doc_value)
        .map_err(|err| anyhow!("failed to deserialize kernel service doc for {pkg_name}: {err}"))
}

fn build_kernel_service_spec(
    pkg_name: &str,
    display_name: &str,
    description: &str,
    category: &str,
    selector_type: &str,
    port: u16,
    expected_instance_count: u32,
    state: ServiceState,
) -> Result<KernelServiceSpec> {
    let service_doc =
        build_kernel_service_doc(pkg_name, display_name, description, category, selector_type)?;

    let mut install_config = ServiceInstallConfig::default();
    install_config.service_ports = HashMap::from([("main".to_string(), port)]);

    Ok(KernelServiceSpec {
        service_doc,
        enable: true,
        expected_instance_count,
        state,
        install_config,
    })
}

#[derive(Serialize)]
struct VerifyHubSettings {
    trust_keys: Vec<String>,
}

#[derive(Serialize)]
struct RepoServiceSettings {
    remote_source: HashMap<String, String>,
    enable_dev_mode: bool,
}

impl TryFrom<&Value> for StartConfigSummary {
    type Error = anyhow::Error;

    fn try_from(value: &Value) -> Result<Self> {
        let user_public_key: Jwk = serde_json::from_value(
            value
                .get("public_key")
                .cloned()
                .ok_or_else(|| anyhow!("start_config.json missing public_key"))?,
        ).map_err(|e| anyhow!("Failed to parse public key: {}", e))?;
        Ok(Self {
            user_name: value
                .get("user_name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("start_config.json missing user_name"))?
                .to_string(),
            admin_password_hash: value
                .get("admin_password_hash")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("start_config.json missing admin_password_hash"))?
                .to_string(),
            public_key: user_public_key,

            ood_jwt: value
                .get("ood_jwt")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
        })
    }
}

impl StartConfigSummary {
    pub fn from_value(value: &Value) -> Result<Self> {
        Self::try_from(value)
    }
}
