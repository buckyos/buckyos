use anyhow::{anyhow, Result};
use buckyos_api::msg_queue::{
    generate_kmsg_service_doc, KMSG_SERVICE_MAIN_PORT, KMSG_SERVICE_UNIQUE_ID,
};
use buckyos_api::{
    generate_aicc_service_doc, generate_control_panel_service_doc, generate_msg_center_service_doc,
    generate_opendan_service_doc, generate_repo_service_doc, generate_scheduler_service_doc,
    generate_smb_service_doc, generate_task_manager_service_doc, generate_verify_hub_service_doc,
    AppDoc, AppServiceSpec, AppType, GatewaySettings, GatewayShortcut, KernelServiceSpec,
    NodeConfig, NodeState, SelectorType, ServiceExposeConfig, ServiceInfo, ServiceInstallConfig,
    ServiceInstanceReportInfo, ServiceInstanceState, ServiceNode, ServiceState, UserSettings,
    UserState, UserType, OPENDAN_SERVICE_PORT, OPENDAN_SERVICE_UNIQUE_ID,
    SCHEDULER_SERVICE_UNIQUE_ID, VERIFY_HUB_UNIQUE_ID,
};
use buckyos_api::{
    AICC_SERVICE_SERVICE_PORT, AICC_SERVICE_UNIQUE_ID, CONTROL_PANEL_SERVICE_PORT,
    CONTROL_PANEL_SERVICE_UNIQUE_ID, MSG_CENTER_SERVICE_PORT, MSG_CENTER_SERVICE_UNIQUE_ID,
    REPO_SERVICE_UNIQUE_ID, SMB_SERVICE_UNIQUE_ID, TASK_MANAGER_SERVICE_PORT,
    TASK_MANAGER_SERVICE_UNIQUE_ID,
};
use buckyos_kit::get_buckyos_root_dir;
use jsonwebtoken::jwk::Jwk;
use log::{debug, info, warn};
use name_client::resolve_did;
use name_lib::{OwnerConfig, VerifyHubInfo, ZoneBootConfig, ZoneConfig, DID};
use package_lib::PackageId;
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
    pub zone_name: String, //zone hostname
    #[serde(default)]
    pub ood_jwt: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct SystemInstallSettings {
    pub pre_install_apps: HashMap<String, ServiceInstallConfig>,
}

pub struct SystemConfigBuilder {
    entries: HashMap<String, String>,
}

impl SystemConfigBuilder {
    pub fn new(init_map: HashMap<String, String>) -> Self {
        Self { entries: init_map }
    }

    pub fn add_default_accounts(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let root_settings = UserSettings {
            user_type: UserType::Root,
            user_id: config.user_name.clone(),
            show_name: config.user_name.clone(),
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
        };
        self.insert_json("users/root/settings", &root_settings)?;

        let admin_key = format!("users/{}/settings", config.user_name);
        let admin_settings = UserSettings {
            user_type: UserType::Admin,
            user_id: config.user_name.clone(),
            show_name: config.user_name.clone(),
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
        };
        self.insert_json(&admin_key, &admin_settings)?;
        self.append_policy(&format!("g, {}, admin", config.user_name));
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

    pub async fn build_app_doc(&self, app_id: &str) -> Result<AppDoc> {
        let app_did = PackageId::unique_name_to_did(app_id);
        let did_raw_host = app_did.to_raw_host_name();
        let cache_doc = get_buckyos_root_dir()
            .join("local")
            .join("did_docs")
            .join(format!("{}.doc.json", did_raw_host));
        let app_doc = resolve_did(&app_did, None).await.map_err(|e| {
            let cache_hint = if cache_doc.exists() {
                format!("cache_present={}", cache_doc.display())
            } else {
                format!(
                    "cache_missing={}, hint=populate did_docs cache",
                    cache_doc.display()
                )
            };
            anyhow!(
                "resolve_did failed for app_id={}, did_raw_host={}, {}, err={}",
                app_id,
                did_raw_host,
                cache_hint,
                e
            )
        })?;
        let doc_value = app_doc.to_json_value()?;
        let app_doc = serde_json::from_value(doc_value)?;
        Ok(app_doc)
    }

    pub async fn add_default_apps(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let install_settings = self.entries.get("system/install_settings");
        if install_settings.is_none() {
            return Err(anyhow!("system/install_settings not found"));
        }
        let install_settings: SystemInstallSettings =
            serde_json::from_str(install_settings.unwrap())?;
        let mut app_index = 10;
        for (app_id, app_install_config) in install_settings.pre_install_apps.iter() {
            let app_doc = self.build_app_doc(app_id).await?;
            let app_key = format!("users/{}/apps/{}/spec", config.user_name, app_id);
            debug!("app_key: {}", app_key);

            let app_spec = AppServiceSpec {
                app_doc,
                app_index: app_index,
                user_id: config.user_name.clone(),
                enable: true,
                expected_instance_count: 1,
                state: ServiceState::default(),
                install_config: app_install_config.clone(),
            };

            self.insert_json(&app_key, &app_spec)?;
            app_index += 1;
        }

        Ok(self)
    }

    pub fn add_device_doc(
        &mut self,
        ood_name: &str,
        config: &StartConfigSummary,
    ) -> Result<&mut Self> {
        let ood_jwt = config
            .ood_jwt
            .as_ref()
            .ok_or_else(|| anyhow!("start_config.json missing ood_jwt"))?;
        self.entries
            .insert(format!("devices/{}/doc", ood_name), ood_jwt.clone());
        Ok(self)
    }

    pub fn add_system_defaults(&mut self) -> Result<&mut Self> {
        self.insert_json("system/system_pkgs", &json!({}))?;
        Ok(self)
    }

    pub async fn add_control_panel(&mut self) -> Result<&mut Self> {
        // NOTE: scheduler loads any `services/<name>/spec` as `KernelServiceSpec`.
        // We follow the same pattern as other kernel-like services to make
        // control-panel available through the existing scheduling pipeline.
        let service_doc = generate_control_panel_service_doc();

        let config = build_kernel_service_spec(
            CONTROL_PANEL_SERVICE_UNIQUE_ID,
            CONTROL_PANEL_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;

        self.insert_json("services/control-panel/spec", &config)?;
        Ok(self)
    }

    pub async fn add_verify_hub(&mut self, verify_hub_private_key: &str) -> Result<&mut Self> {
        self.entries.insert(
            "system/verify-hub/key".into(),
            verify_hub_private_key.to_string(),
        );

        let service_doc = generate_verify_hub_service_doc();

        let config = build_kernel_service_spec(VERIFY_HUB_UNIQUE_ID, 3300, 1, service_doc).await?;
        self.insert_json("services/verify-hub/spec", &config)?;

        let settings = VerifyHubSettings { trust_keys: vec![] };
        self.insert_json("services/verify-hub/settings", &settings)?;

        Ok(self)
    }

    pub async fn add_scheduler(&mut self) -> Result<&mut Self> {
        let service_doc = generate_scheduler_service_doc();
        let config =
            build_kernel_service_spec(SCHEDULER_SERVICE_UNIQUE_ID, 3400, 1, service_doc).await?;
        self.insert_json("services/scheduler/spec", &config)?;
        Ok(self)
    }

    pub async fn add_task_mgr(&mut self) -> Result<&mut Self> {
        let service_doc = generate_task_manager_service_doc();
        let config = build_kernel_service_spec(
            TASK_MANAGER_SERVICE_UNIQUE_ID,
            TASK_MANAGER_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/task-manager/spec", &config)?;
        Ok(self)
    }

    pub async fn add_kmsg(&mut self) -> Result<&mut Self> {
        let service_doc = generate_kmsg_service_doc();
        let config = build_kernel_service_spec(
            KMSG_SERVICE_UNIQUE_ID,
            KMSG_SERVICE_MAIN_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/kmsg/spec", &config)?;
        Ok(self)
    }

    pub async fn add_aicc(&mut self) -> Result<&mut Self> {
        let service_doc = generate_aicc_service_doc();
        let config = build_kernel_service_spec(
            AICC_SERVICE_UNIQUE_ID,
            AICC_SERVICE_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/aicc/spec", &config)?;
        let settings = json!({
            "openai": {
                "enabled": false,
                "api_token": "",
                "alias_map": {},
                "instances": []
            }
        });
        self.insert_json("services/aicc/settings", &settings)?;
        Ok(self)
    }

    pub async fn add_msg_center(&mut self) -> Result<&mut Self> {
        let service_doc = generate_msg_center_service_doc();
        let config = build_kernel_service_spec(
            MSG_CENTER_SERVICE_UNIQUE_ID,
            MSG_CENTER_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/msg-center/spec", &config)?;
        let settings = json!({
            "telegram_tunnel": {
                "enabled": true,
                "tunnel_did": "did:bns:msg-center-default-tunnel",
                "supports_ingress": true,
                "supports_egress": true,
                "gateway": {
                    "mode": "dry_run"
                },
                "bindings": []
            }
        });
        self.insert_json("services/msg-center/settings", &settings)?;
        Ok(self)
    }

    pub async fn add_opendan(&mut self) -> Result<&mut Self> {
        let service_doc = generate_opendan_service_doc();
        let config = build_kernel_service_spec(
            OPENDAN_SERVICE_UNIQUE_ID,
            OPENDAN_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/opendan/spec", &config)?;
        Ok(self)
    }

    pub fn add_gateway_settings(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        // let settings = GatewaySettings {
        //     shortcuts: HashMap::from([
        //         (
        //             "www".to_string(),
        //             GatewayShortcut {
        //                 target_type: "app".to_string(),
        //                 user_id: Some(config.user_name.clone()),
        //                 app_id: "buckyos_filebrowser".to_string(),
        //             },
        //         ),
        //         (
        //             "_".to_string(),
        //             GatewayShortcut {
        //                 target_type: "app".to_string(),
        //                 user_id: Some(config.user_name.clone()),
        //                 app_id: "buckyos_filebrowser".to_string(),
        //             },
        //         )
        //     ]),
        // };
        // self.insert_json("services/gateway/settings", &settings)?;
        Ok(self)
    }

    pub async fn add_repo_service(&mut self) -> Result<&mut Self> {
        let service_doc = generate_repo_service_doc();
        let config =
            build_kernel_service_spec(REPO_SERVICE_UNIQUE_ID, 4000, 1, service_doc).await?;
        self.insert_json("services/repo-service/spec", &config)?;

        let settings = RepoServiceSettings {
            remote_source: HashMap::from([(
                "default".to_string(),
                "https://buckyos.ai/ndn/repo/meta_index.db".to_string(),
            )]),
            enable_dev_mode: true,
        };
        self.insert_json("services/repo-service/settings", &settings)?;

        let pkg_list = HashMap::from([
            (
                "nightly-linux-amd64.node_daemon".to_string(),
                "no".to_string(),
            ),
            (
                "nightly-linux-aarch64.node_daemon".to_string(),
                "no".to_string(),
            ),
            (
                "nightly-windows-amd64.node_daemon".to_string(),
                "no".to_string(),
            ),
            (
                "nightly-apple-amd64.node_daemon".to_string(),
                "no".to_string(),
            ),
            (
                "nightly-apple-aarch64.node_daemon".to_string(),
                "no".to_string(),
            ),
            ("nightly-linux-amd64.buckycli".to_string(), "no".to_string()),
            (
                "nightly-linux-aarch64.buckycli".to_string(),
                "no".to_string(),
            ),
            (
                "nightly-windows-amd64.buckycli".to_string(),
                "no".to_string(),
            ),
            ("nightly-apple-amd64.buckycli".to_string(), "no".to_string()),
            (
                "nightly-apple-aarch64.buckycli".to_string(),
                "no".to_string(),
            ),
        ]);
        self.insert_json("services/repo-service/pkg_list", &pkg_list)?;
        Ok(self)
    }

    pub async fn add_smb_service(&mut self) -> Result<&mut Self> {
        let service_doc = generate_smb_service_doc();
        let config = build_kernel_service_spec(SMB_SERVICE_UNIQUE_ID, 4100, 1, service_doc).await?;
        self.insert_json("services/smb-service/spec", &config)?;
        Ok(self)
    }

    pub fn append_policy(&mut self, policy: &str) -> Result<&mut Self> {
        let policy_str = self.entries.get("system/rbac/base_policy");
        if policy_str.is_none() {
            self.entries
                .insert("system/rbac/base_policy".to_string(), policy.to_string());
            return Ok(self);
        }
        let policy_str = policy_str.unwrap();
        let new_policy_str = format!("{}\n{}", policy_str, policy);
        self.entries
            .insert("system/rbac/base_policy".to_string(), new_policy_str);
        Ok(self)
    }

    pub fn add_node(&mut self, ood_name: &str) -> Result<&mut Self> {
        let config = NodeConfig {
            node_id: ood_name.to_string(),
            node_did: format!("did:bns:{ood_name}"),
            kernel: HashMap::new(),
            apps: HashMap::new(),
            frame_services: HashMap::new(),
            state: NodeState::Running,
        };
        self.insert_json(&format!("nodes/{}/config", ood_name), &config)?;

        let gateway_config = json!({});
        self.insert_json(
            &format!("nodes/{}/gateway_config", ood_name),
            &gateway_config,
        )?;

        self.append_policy(&format!("g, {ood_name}, ood"))?;
        Ok(self)
    }

    pub fn add_boot_config(
        &mut self,
        config: &StartConfigSummary,
        verify_hub_public_key: &Jwk,
        zone_boot_config: &ZoneBootConfig,
    ) -> Result<&mut Self> {
        let public_key_value = verify_hub_public_key.clone();
        //TODO: add zoone did here:
        let zone_did = DID::from_str(&config.zone_name)?;
        let mut zone_config = ZoneConfig::new(
            zone_did,
            DID::new("bns", &config.user_name),
            config.public_key.clone(),
        );

        let verify_hub_info = VerifyHubInfo {
            public_key: public_key_value,
        };
        let boot_jwt = config.ood_jwt.clone().unwrap_or_default();
        zone_config.init_by_boot_config(zone_boot_config, &boot_jwt);
        zone_config.verify_hub_info = Some(verify_hub_info);
        info!(
            "add_boot_config: zone_config: {}",
            serde_json::to_string_pretty(&zone_config)?
        );
        self.insert_json("boot/config", &zone_config)?;
        Ok(self)
    }

    pub fn build(self) -> HashMap<String, String> {
        self.entries
    }

    fn insert_json<T: ?Sized + serde::Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        let content = serde_json::to_string_pretty(value)?;
        self.entries.insert(key.to_string(), content);
        Ok(())
    }
}

async fn build_kernel_service_spec(
    pkg_name: &str,
    port: u16,
    expected_instance_count: u32,
    service_doc: AppDoc,
) -> Result<KernelServiceSpec> {
    let service_did = PackageId::unique_name_to_did(pkg_name);

    let mut install_config = ServiceInstallConfig::default();
    let service_expose_config = ServiceExposeConfig {
        sub_hostname: Vec::new(),
        expose_uri: Some(format!("/kapi/{}", pkg_name)),
        expose_port: Some(port),
    };
    install_config
        .expose_config
        .insert("www".to_string(), service_expose_config);

    Ok(KernelServiceSpec {
        service_doc,
        app_index: 0,
        enable: true,
        expected_instance_count,
        state: ServiceState::default(),
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
        )
        .map_err(|e| anyhow!("Failed to parse public key: {}", e))?;
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
            zone_name: value
                .get("zone_name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("start_config.json missing zone_name"))?
                .to_string(),

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
