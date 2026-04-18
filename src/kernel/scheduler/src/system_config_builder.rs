use ::kRPC::{RPCSessionToken, RPCSessionTokenType};
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
    ServiceInstanceReportInfo, ServiceInstanceState, ServiceNode, ServiceState, SubPkgDesc,
    UserContactSettings, UserSettings, UserState, UserTunnelBinding, UserType,
    OPENDAN_SERVICE_PORT, OPENDAN_SERVICE_UNIQUE_ID, SCHEDULER_SERVICE_UNIQUE_ID,
    VERIFY_HUB_UNIQUE_ID,
};
use buckyos_api::{
    AICC_SERVICE_SERVICE_PORT, AICC_SERVICE_UNIQUE_ID, CONTROL_PANEL_SERVICE_PORT,
    CONTROL_PANEL_SERVICE_UNIQUE_ID, MSG_CENTER_SERVICE_PORT, MSG_CENTER_SERVICE_UNIQUE_ID,
    REPO_SERVICE_UNIQUE_ID, SMB_SERVICE_UNIQUE_ID, TASK_MANAGER_SERVICE_PORT,
    TASK_MANAGER_SERVICE_UNIQUE_ID,
};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_system_etc_dir};
use jsonwebtoken::jwk::Jwk;
use log::{debug, info, warn};
use name_lib::{
    generate_ed25519_key_pair, load_private_key, AgentDocument, OwnerConfig, VerifyHubInfo,
    ZoneBootConfig, ZoneConfig, DID,
};
use package_lib::PackageId;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::TryFrom;

const DEFAULT_OOD_ID: &str = "ood1";
const DEFAULT_SN_OPENAI_MODELS: &[&str] = &["gpt-5", "gpt-5-mini", "gpt-5-nono", "gpt-5-pro"];
const DEFAULT_SN_OPENAI_IMAGE_MODELS: &[&str] = &["dall-e-3", "dall-e-2"];
const SN_OPENAI_MODELS_API: &str = "https://sn.buckyos.ai/api/v1/ai/models";
const SN_OPENAI_CHAT_COMPLETIONS_API: &str = "https://sn.buckyos.ai/api/v1/ai/chat/completions";

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct AIProviderConfigSummary {
    #[serde(default)]
    pub openai_api_token: String,
    #[serde(default)]
    pub claude_api_token: String,
    #[serde(default)]
    pub google_api_token: String,
    #[serde(default)]
    pub openrouter_api_token: String,
    #[serde(default)]
    pub glm_api_token: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct JarvisMsgTunnelConfigSummary {
    #[serde(default)]
    pub telegram_bot_api_token: String,
    #[serde(default)]
    pub telegram_account_id: String,
}

#[derive(Debug, Deserialize)]
pub struct StartConfigSummary {
    pub user_name: String,
    pub admin_password_hash: String,
    pub public_key: Jwk,
    pub zone_name: String, //zone hostname
    #[serde(default)]
    pub sn_active_code: String,
    #[serde(default)]
    pub ood_jwt: Option<String>,
    #[serde(default)]
    pub ai_provider_config: AIProviderConfigSummary,
    #[serde(default)]
    pub jarvis_msg_tunnel_config: JarvisMsgTunnelConfigSummary,
}

#[derive(Serialize, Deserialize)]
pub struct SystemInstallSettings {
    pub pre_install_apps: HashMap<String, PreInstallAppConfig>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PreInstallAppConfig {
    #[serde(alias = "app-doc")]
    pub app_doc: AppDoc,
    #[serde(flatten)]
    pub install_config: ServiceInstallConfig,
}

pub struct SystemConfigBuilder {
    entries: HashMap<String, String>,
}

impl SystemConfigBuilder {
    pub fn new(init_map: HashMap<String, String>) -> Self {
        Self { entries: init_map }
    }

    pub fn add_default_accounts(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let admin_contact = build_zone_user_contact_settings(config)?;
        let root_settings = UserSettings {
            user_type: UserType::Root,
            user_id: config.user_name.clone(),
            show_name: config.user_name.clone(),
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            contact: None,
        };
        self.insert_json_if_absent("users/root/settings", &root_settings)?;

        let admin_key = format!("users/{}/settings", config.user_name);
        let admin_settings = UserSettings {
            user_type: UserType::Admin,
            user_id: config.user_name.clone(),
            show_name: config.user_name.clone(),
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            contact: admin_contact,
        };
        self.insert_json_if_absent(&admin_key, &admin_settings)?;
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

    pub async fn add_default_agents(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        //add jarvis agent as default agent
        // agents/jarvis/doc -> agent doc
        let zone_did = DID::from_str(&config.zone_name)?;
        let jarvis_did = DID::new(
            zone_did.method.as_str(),
            format!("jarvis.{}", zone_did.id.as_str()).as_str(),
        );
        let owner_did = DID::new("bns", &config.user_name);

        let (jarvis_private_key_pem, jarvis_public_key_jwk) = generate_ed25519_key_pair();
        let jarvis_public_key_jwk: Jwk = serde_json::from_value(jarvis_public_key_jwk)
            .map_err(|e| anyhow!("invalid generated jarvis public key: {}", e))?;

        let mut jarvis_doc = AgentDocument::new(jarvis_did, owner_did, jarvis_public_key_jwk);
        jarvis_doc.public_description = Some("Default built-in OpenDAN agent for BuckyOS".into());

        self.insert_json("agents/jarvis/doc", &jarvis_doc)?;
        self.entries
            .insert("agents/jarvis/key".to_string(), jarvis_private_key_pem);

        let legacy_app_spec_key = format!("users/{}/apps/jarvis/spec", config.user_name);
        if self.entries.remove(&legacy_app_spec_key).is_some() {
            warn!(
                "removed conflicting legacy jarvis app spec at {} while installing default agent spec",
                legacy_app_spec_key
            );
        }

        let jarvis_spec_key = format!("users/{}/agents/jarvis/spec", config.user_name);
        let jarvis_spec = build_default_jarvis_agent_spec(config)?;
        self.insert_json(&jarvis_spec_key, &jarvis_spec)?;

        // agents/jarvis/settings -> agent settings,
        let jarvis_settings = json!({
            "enabled": true,
            "auto_start": true
        });
        self.insert_json_if_absent("agents/jarvis/settings", &jarvis_settings)?;
        Ok(self)
    }

    pub async fn add_default_apps(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let install_settings = self.entries.get("system/install_settings");
        if install_settings.is_none() {
            return Err(anyhow!("system/install_settings not found"));
        }
        let install_settings: SystemInstallSettings =
            serde_json::from_str(install_settings.unwrap())?;
        let mut app_index = 10;
        for (app_id, pre_install_app) in install_settings.pre_install_apps.iter() {
            let app_doc = pre_install_app.app_doc.clone();
            if app_doc.name != *app_id {
                return Err(anyhow!(
                    "pre_install_apps[{}].app_doc.name={} does not match app_id",
                    app_id,
                    app_doc.name
                ));
            }
            let app_key = format!("users/{}/apps/{}/spec", config.user_name, app_id);
            debug!("app_key: {}", app_key);

            let app_spec = AppServiceSpec {
                app_doc,
                app_index: app_index,
                user_id: config.user_name.clone(),
                enable: true,
                expected_instance_count: 1,
                state: ServiceState::default(),
                install_config: pre_install_app.install_config.clone(),
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
        self.insert_json_if_absent("services/verify-hub/settings", &settings)?;

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
        let mut config = build_kernel_service_spec(
            TASK_MANAGER_SERVICE_UNIQUE_ID,
            TASK_MANAGER_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        config.install_config.rdb_instances.insert(
            buckyos_api::TASK_MANAGER_RDB_INSTANCE_ID.to_string(),
            buckyos_api::task_manager_default_rdb_instance_config(),
        );
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

    pub async fn add_aicc(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let service_doc = generate_aicc_service_doc();
        let service_spec = build_kernel_service_spec(
            AICC_SERVICE_UNIQUE_ID,
            AICC_SERVICE_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/aicc/spec", &service_spec)?;
        let sn_openai_models = if trim_to_option(config.sn_active_code.as_str()).is_some() {
            fetch_sn_openai_models(config.user_name.as_str()).await
        } else {
            None
        };
        let settings = build_aicc_settings_with_sn_models(config, sn_openai_models.as_deref());
        self.insert_json_if_absent("services/aicc/settings", &settings)?;
        Ok(self)
    }

    pub async fn add_msg_center(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let service_doc = generate_msg_center_service_doc();
        let service_spec = build_kernel_service_spec(
            MSG_CENTER_SERVICE_UNIQUE_ID,
            MSG_CENTER_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/msg-center/spec", &service_spec)?;
        let settings = build_msg_center_settings(config)?;
        self.insert_json_if_absent("services/msg-center/settings", &settings)?;
        Ok(self)
    }

    pub fn add_gateway_settings(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let settings = GatewaySettings {
            shortcuts: HashMap::from([
                // (
                //     "www".to_string(),
                //     GatewayShortcut {
                //         target_type: "app".to_string(),
                //         user_id: Some(config.user_name.clone()),
                //         app_id: "buckyos_filebrowser".to_string(),
                //     },
                // ),
                (
                    "_".to_string(),
                    GatewayShortcut {
                        target_type: "service".to_string(),
                        user_id: None,
                        app_id: "control-panel".to_string(),
                    },
                ),
            ]),
        };
        self.insert_json("services/gateway/settings", &settings)?;
        Ok(self)
    }

    pub async fn add_repo_service(&mut self) -> Result<&mut Self> {
        let service_doc = generate_repo_service_doc();
        let mut config =
            build_kernel_service_spec(REPO_SERVICE_UNIQUE_ID, 4000, 1, service_doc).await?;
        config.install_config.rdb_instances.insert(
            buckyos_api::REPO_SERVICE_RDB_INSTANCE_ID.to_string(),
            buckyos_api::repo_service_default_rdb_instance_config(),
        );
        self.insert_json("services/repo-service/spec", &config)?;

        let settings = RepoServiceSettings {
            remote_source: HashMap::from([(
                "default".to_string(),
                "https://buckyos.ai/ndn/repo/meta_index.db".to_string(),
            )]),
            enable_dev_mode: true,
        };
        self.insert_json_if_absent("services/repo-service/settings", &settings)?;

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

        let gateway_info = json!({});
        self.insert_json(&format!("nodes/{}/gateway_info", ood_name), &gateway_info)?;

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

    fn insert_json_if_absent<T: ?Sized + serde::Serialize>(
        &mut self,
        key: &str,
        value: &T,
    ) -> Result<()> {
        if self.entries.contains_key(key) {
            return Ok(());
        }
        self.insert_json(key, value)
    }
}

fn default_jarvis_agent_doc(config: &StartConfigSummary) -> Value {
    let jarvis_did = config
        .zone_name
        .strip_prefix("did:web:")
        .map(|zone_host| format!("did:web:jarvis.{zone_host}"))
        .unwrap_or_else(|| "did:web:jarvis.test.buckyos.io".to_string());
    json!({
        "id": jarvis_did,
        "name": "Jarvis",
        "kind": "root-agent",
        "description": "Default built-in OpenDAN agent for BuckyOS"
    })
}

fn build_default_jarvis_agent_spec(config: &StartConfigSummary) -> Result<AppServiceSpec> {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    const JARVIS_APP_ID: &str = "jarvis";
    const JARVIS_PKG_NAME: &str = "buckyos_jarvis";

    let owner_did = DID::from_str("did:bns:buckyos")?;
    let app_doc = AppDoc::builder(
        AppType::Agent,
        JARVIS_APP_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Jarvis")
    .description_detail("Default built-in OpenDAN agent for BuckyOS")
    .selector_type(SelectorType::Single)
    .service_port("www", OPENDAN_SERVICE_PORT)
    .agent_pkg(SubPkgDesc::new(format!("{JARVIS_PKG_NAME}")))
    .build()
    .map_err(|err| anyhow!("build default jarvis app doc failed: {err}"))?;

    let mut install_config = ServiceInstallConfig::default();
    install_config
        .expose_config
        .insert("www".to_string(), ServiceExposeConfig::default());

    Ok(AppServiceSpec {
        app_doc,
        app_index: 1,
        user_id: config.user_name.clone(),
        enable: true,
        expected_instance_count: 1,
        state: ServiceState::default(),
        install_config,
    })
}

fn trim_to_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn resolve_jarvis_agent_did(config: &StartConfigSummary) -> Result<DID> {
    let zone_did = DID::from_str(&config.zone_name)?;
    Ok(DID::new(
        zone_did.method.as_str(),
        format!("jarvis.{}", zone_did.id.as_str()).as_str(),
    ))
}

fn resolve_telegram_tunnel_did(config: &StartConfigSummary) -> String {
    config
        .zone_name
        .strip_prefix("did:web:")
        .map(|zone_host| format!("did:web:tg-tunnel.{}", zone_host))
        .unwrap_or_else(|| "did:bns:msg-center-default-tunnel".to_string())
}

fn normalize_telegram_contact_account_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if trimmed.starts_with("user:")
        || trimmed.starts_with("group:")
        || trimmed.starts_with("channel:")
    {
        trimmed.to_string()
    } else if trimmed.parse::<i64>().is_ok() {
        format!("user:{}", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn normalize_telegram_default_chat_id(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some((kind, value)) = trimmed.split_once(':') {
        if matches!(kind, "user" | "group" | "channel") {
            return value.trim().to_string();
        }
    }
    trimmed.to_string()
}

fn build_zone_user_contact_settings(
    config: &StartConfigSummary,
) -> Result<Option<UserContactSettings>> {
    let Some(account_id) =
        trim_to_option(config.jarvis_msg_tunnel_config.telegram_account_id.as_str())
    else {
        return Ok(None);
    };

    let tunnel_id = resolve_telegram_tunnel_did(config);
    let normalized_account_id = normalize_telegram_contact_account_id(&account_id);
    if normalized_account_id.is_empty() {
        return Ok(None);
    }

    Ok(Some(UserContactSettings {
        did: Some(DID::new("bns", &config.user_name).to_string()),
        note: None,
        groups: vec!["zone_user".to_string()],
        tags: vec!["zone_user".to_string()],
        bindings: vec![UserTunnelBinding {
            platform: "telegram".to_string(),
            account_id: normalized_account_id,
            display_id: Some(account_id),
            tunnel_id: Some(tunnel_id),
            meta: HashMap::new(),
        }],
    }))
}

fn build_aicc_settings(config: &StartConfigSummary) -> Value {
    build_aicc_settings_with_sn_models(config, None)
}

fn build_aicc_settings_with_sn_models(
    config: &StartConfigSummary,
    sn_openai_models: Option<&[String]>,
) -> Value {
    const DEFAULT_PROVIDER_TIMEOUT_MS: u64 = 600_000;
    let mut settings = serde_json::Map::new();
    let mut openai_alias_map = serde_json::Map::new();
    let mut openai_instances = Vec::<Value>::new();
    let openai_api_token =
        trim_to_option(config.ai_provider_config.openai_api_token.as_str()).unwrap_or_default();

    if !openai_api_token.is_empty() {
        openai_alias_map.insert("gpt-fast".to_string(), json!("gpt-5-mini"));
        openai_alias_map.insert("gpt-plan".to_string(), json!("gpt-5"));
        openai_instances.push(json!({
            "instance_id": "openai-default",
            "provider_type": "openai",
            "base_url": "https://api.openai.com/v1",
            "timeout_ms": DEFAULT_PROVIDER_TIMEOUT_MS,
            "models": ["gpt-5", "gpt-5-mini", "gpt-5-nono", "gpt-5-pro"],
            "default_model": "gpt-5-mini",
            "image_models": ["dall-e-3", "dall-e-2"],
            "default_image_model": "dall-e-3",
            "features": ["plan", "json_output", "tool_calling", "web_search"]
        }));
    }

    if trim_to_option(config.sn_active_code.as_str()).is_some() {
        let sn_model_settings = build_sn_openai_model_settings(sn_openai_models);
        if !openai_alias_map.contains_key("llm.default") {
            openai_alias_map.insert(
                "llm.default".to_string(),
                json!(sn_model_settings.default_model.as_str()),
            );
        }
        if !openai_alias_map.contains_key("llm.chat.default") {
            openai_alias_map.insert(
                "llm.chat.default".to_string(),
                json!(sn_model_settings.default_model.as_str()),
            );
        }
        if !openai_alias_map.contains_key("llm.plan.default") {
            openai_alias_map.insert(
                "llm.plan.default".to_string(),
                json!(sn_model_settings.plan_default_model.as_str()),
            );
        }
        if !openai_alias_map.contains_key("llm.code.default") {
            openai_alias_map.insert(
                "llm.code.default".to_string(),
                json!(sn_model_settings.default_model.as_str()),
            );
        }

        openai_instances.push(json!({
            "instance_id": "sn-openai-default",
            "provider_type": "sn-openai",
            "base_url": SN_OPENAI_CHAT_COMPLETIONS_API,
            "timeout_ms": DEFAULT_PROVIDER_TIMEOUT_MS,
            "models": sn_model_settings.models,
            "default_model": sn_model_settings.default_model,
            "image_models": sn_model_settings.image_models,
            "default_image_model": sn_model_settings.default_image_model,
            "features": ["plan", "json_output", "tool_calling", "web_search"],
            "auth_mode": "device_jwt"
        }));
    }

    if !openai_instances.is_empty() {
        settings.insert(
            "openai".to_string(),
            json!({
                "enabled": true,
                "api_token": openai_api_token,
                "alias_map": Value::Object(openai_alias_map),
                "instances": openai_instances
            }),
        );
    }

    if let Some(api_token) = trim_to_option(config.ai_provider_config.google_api_token.as_str()) {
        settings.insert(
            "google".to_string(),
            json!({
                "enabled": true,
                "api_token": api_token,
                "alias_map": {
                    "gemini-ops": "gemini-2.5-flash"
                },
                "instances": [
                    {
                        "instance_id": "google-gimini-default",
                        "provider_type": "google-gimini",
                        "base_url": "https://generativelanguage.googleapis.com/v1beta",
                        "timeout_ms": DEFAULT_PROVIDER_TIMEOUT_MS,
                        "models": ["gemini-2.5-flash", "gemini-2.5-pro"],
                        "default_model": "gemini-2.5-flash",
                        "image_models": [
                            "gemini-2.0-flash-exp-image-generation",
                            "gemini-2.5-flash-image-preview"
                        ],
                        "default_image_model": "gemini-2.5-flash-image-preview",
                        "features": ["plan", "json_output"]
                    }
                ]
            }),
        );
    }

    if let Some(api_token) = trim_to_option(config.ai_provider_config.claude_api_token.as_str()) {
        settings.insert(
            "claude".to_string(),
            json!({
                "enabled": true,
                "api_token": api_token,
                "alias_map": {
                    "claude-reasoning": "claude-3-7-sonnet-20250219"
                },
                "instances": [
                    {
                        "instance_id": "claude-default",
                        "provider_type": "claude",
                        "base_url": "https://api.anthropic.com/v1",
                        "timeout_ms": DEFAULT_PROVIDER_TIMEOUT_MS,
                        "models": ["claude-3-7-sonnet-20250219", "claude-3-5-haiku-20241022"],
                        "default_model": "claude-3-7-sonnet-20250219",
                        "features": ["plan", "json_output", "tool_calling"]
                    }
                ]
            }),
        );
    }

    if settings.is_empty() {
        json!({
            "openai": {
                "enabled": false,
                "api_token": "",
                "alias_map": {},
                "instances": []
            }
        })
    } else {
        Value::Object(settings)
    }
}

#[derive(Debug)]
struct SnOpenAIModelSettings {
    models: Vec<String>,
    default_model: String,
    plan_default_model: String,
    image_models: Vec<String>,
    default_image_model: String,
}

fn build_sn_openai_model_settings(sn_openai_models: Option<&[String]>) -> SnOpenAIModelSettings {
    let mut models = sn_openai_models
        .unwrap_or(&[])
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(|item| item.to_string())
        .collect::<Vec<_>>();
    if models.is_empty() {
        models = DEFAULT_SN_OPENAI_MODELS
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>();
    }

    let default_model =
        pick_preferred_model(models.as_slice(), &["gpt-5-mini", "gpt-5", "gpt-4.1-mini"])
            .unwrap_or_else(|| models[0].clone());
    let plan_default_model =
        pick_preferred_model(models.as_slice(), &["gpt-5", "gpt-5-mini", "gpt-4.1"])
            .unwrap_or_else(|| default_model.clone());

    let mut image_models = models
        .iter()
        .filter(|item| is_image_model(item.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if image_models.is_empty() {
        image_models = DEFAULT_SN_OPENAI_IMAGE_MODELS
            .iter()
            .map(|item| item.to_string())
            .collect::<Vec<_>>();
    }
    let default_image_model =
        pick_preferred_model(image_models.as_slice(), &["dall-e-3", "gpt-image-1"])
            .unwrap_or_else(|| image_models[0].clone());

    SnOpenAIModelSettings {
        models,
        default_model,
        plan_default_model,
        image_models,
        default_image_model,
    }
}

fn pick_preferred_model(models: &[String], preferred: &[&str]) -> Option<String> {
    for target in preferred.iter() {
        if let Some(matched) = models.iter().find(|item| item == target) {
            return Some(matched.clone());
        }
    }
    None
}

fn is_image_model(model_id: &str) -> bool {
    let value = model_id.to_ascii_lowercase();
    value.contains("dall-e")
        || value.contains("gpt-image")
        || value.contains("image")
        || value.contains("vision")
}

async fn fetch_sn_openai_models(user_name: &str) -> Option<Vec<String>> {
    match fetch_sn_openai_models_impl(user_name).await {
        Ok(models) => Some(models),
        Err(err) => {
            warn!(
                "fetch sn-openai models from {} failed: {}",
                SN_OPENAI_MODELS_API, err
            );
            None
        }
    }
}

async fn fetch_sn_openai_models_impl(user_name: &str) -> Result<Vec<String>> {
    let token = build_device_jwt_token_for_sn(user_name)?;
    let client = Client::new();
    let response = client
        .get(SN_OPENAI_MODELS_API)
        .bearer_auth(token)
        .send()
        .await
        .map_err(|err| anyhow!("request failed: {}", err))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("request failed with status {}", status));
    }
    let response_text = response
        .text()
        .await
        .map_err(|err| anyhow!("failed to read models response body: {}", err))?;
    info!("sn-openai models endpoint raw response: {}", response_text);
    let body: Value = serde_json::from_str(response_text.as_str())
        .map_err(|err| anyhow!("invalid models response json: {}", err))?;
    let models = extract_model_ids_from_response(&body);
    if models.is_empty() {
        return Err(anyhow!("models response does not contain model ids"));
    }

    info!("fetched {} sn-openai models: {:?}", models.len(), models);
    Ok(models)
}

fn build_device_jwt_token_for_sn(user_name: &str) -> Result<String> {
    let device_name = read_default_device_subject();
    let private_key_path = get_buckyos_system_etc_dir().join("node_private_key.pem");
    let private_key = load_private_key(private_key_path.as_path()).map_err(|err| {
        anyhow!(
            "failed to load device private key '{}': {}",
            private_key_path.display(),
            err
        )
    })?;
    let now = buckyos_get_unix_timestamp();
    let claims = RPCSessionToken {
        token_type: RPCSessionTokenType::JWT,
        token: None,
        aud: None,
        exp: Some(now + 60 * 15),
        iss: Some(device_name),
        jti: None,
        session: None,
        sub: Some(user_name.to_string()),
        appid: Some("aicc".to_string()),
        extra: HashMap::new(),
    };
    claims
        .generate_jwt(None, &private_key)
        .map_err(|err| anyhow!("generate sn models jwt failed: {}", err))
}

fn read_default_device_subject() -> String {
    let device_cfg_path = get_buckyos_system_etc_dir().join("node_device_config.json");
    let content = std::fs::read_to_string(device_cfg_path.as_path());
    if let Ok(content) = content {
        if let Ok(json_value) = serde_json::from_str::<Value>(content.as_str()) {
            if let Some(name) = json_value
                .get("name")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return name.to_string();
            }
        }
    }
    DEFAULT_OOD_ID.to_string()
}

fn extract_model_ids_from_response(payload: &Value) -> Vec<String> {
    let mut result = Vec::<String>::new();

    if let Some(items) = payload.get("items").and_then(|value| value.as_array()) {
        for item in items {
            if let Some(model_id) = item
                .get("model")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                result.push(model_id.to_string());
            }
        }
    }

    if let Some(default_model) = payload
        .get("default_model")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        result.push(default_model.to_string());
    }

    result.sort_unstable();
    result.dedup();
    result
}

fn build_msg_center_settings(config: &StartConfigSummary) -> Result<Value> {
    let tunnel_did = resolve_telegram_tunnel_did(config);
    let bot_token = trim_to_option(
        config
            .jarvis_msg_tunnel_config
            .telegram_bot_api_token
            .as_str(),
    );
    let telegram_account_id =
        trim_to_option(config.jarvis_msg_tunnel_config.telegram_account_id.as_str());

    let (gateway_mode, bindings) =
        if let (Some(bot_token), Some(account_id)) = (bot_token, telegram_account_id) {
            let jarvis_did = resolve_jarvis_agent_did(config)?;
            let default_chat_id = normalize_telegram_default_chat_id(&account_id);
            (
                "bot_api",
                vec![json!({
                    "owner_did": jarvis_did.to_string(),
                    "bot_token": bot_token,
                    "default_chat_id": default_chat_id
                })],
            )
        } else {
            ("dry_run", Vec::new())
        };

    Ok(json!({
        "telegram_tunnel": {
            "enabled": true,
            "tunnel_did": tunnel_did,
            "supports_ingress": true,
            "supports_egress": true,
            "gateway": {
                "mode": gateway_mode
            },
            "bindings": bindings
        }
    }))
}

async fn build_kernel_service_spec(
    pkg_name: &str,
    port: u16,
    expected_instance_count: u32,
    mut service_doc: AppDoc,
) -> Result<KernelServiceSpec> {
    let _service_did = PackageId::unique_name_to_did(pkg_name);
    attach_current_platform_service_pkg(&mut service_doc);

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

fn attach_current_platform_service_pkg(service_doc: &mut AppDoc) {
    let current_pkg = SubPkgDesc::new(service_doc.get_package_id().to_string());

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        if service_doc.pkg_list.amd64_linux_app.is_none() {
            service_doc.pkg_list.amd64_linux_app = Some(current_pkg);
        }
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        if service_doc.pkg_list.aarch64_linux_app.is_none() {
            service_doc.pkg_list.aarch64_linux_app = Some(current_pkg);
        }
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        if service_doc.pkg_list.amd64_win_app.is_none() {
            service_doc.pkg_list.amd64_win_app = Some(current_pkg);
        }
    }
    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        if service_doc.pkg_list.aarch64_win_app.is_none() {
            service_doc.pkg_list.aarch64_win_app = Some(current_pkg);
        }
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        if service_doc.pkg_list.amd64_apple_app.is_none() {
            service_doc.pkg_list.amd64_apple_app = Some(current_pkg);
        }
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        if service_doc.pkg_list.aarch64_apple_app.is_none() {
            service_doc.pkg_list.aarch64_apple_app = Some(current_pkg);
        }
    }
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
            sn_active_code: value
                .get("sn_active_code")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),

            ood_jwt: value
                .get("ood_jwt")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            ai_provider_config: serde_json::from_value(
                value
                    .get("ai_provider_config")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .map_err(|e| anyhow!("Failed to parse ai_provider_config: {}", e))?,
            jarvis_msg_tunnel_config: serde_json::from_value(
                value
                    .get("jarvis_msg_tunnel_config")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            )
            .map_err(|e| anyhow!("Failed to parse jarvis_msg_tunnel_config: {}", e))?,
        })
    }
}

impl StartConfigSummary {
    pub fn from_value(value: &Value) -> Result<Self> {
        Self::try_from(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_aicc_settings, build_aicc_settings_with_sn_models, build_default_jarvis_agent_spec,
        build_kernel_service_spec, build_msg_center_settings, build_zone_user_contact_settings,
        extract_model_ids_from_response, StartConfigSummary, SystemConfigBuilder,
    };
    use buckyos_api::{
        generate_verify_hub_service_doc, AppDoc, AppServiceSpec, AppType, OPENDAN_SERVICE_PORT,
    };
    use name_lib::DID;
    use serde_json::json;
    use std::collections::HashMap;

    fn sample_preinstall_app_doc(app_id: &str, version: &str) -> AppDoc {
        let owner = DID::from_str("did:web:example.com").expect("valid owner did");
        AppDoc::builder(
            AppType::Service,
            app_id,
            version,
            "did:web:example.com",
            &owner,
        )
        .show_name("Demo App")
        .build()
        .expect("build sample app doc")
    }

    #[test]
    fn start_config_summary_parses_optional_bootstrap_configs() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com",
            "ai_provider_config": {
                "openai_api_token": "sk-openai",
                "google_api_token": "google-token"
            },
            "jarvis_msg_tunnel_config": {
                "telegram_bot_api_token": "123:bot",
                "telegram_account_id": "@alice"
            }
        });

        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        assert_eq!(summary.ai_provider_config.openai_api_token, "sk-openai");
        assert_eq!(summary.ai_provider_config.google_api_token, "google-token");
        assert_eq!(
            summary.jarvis_msg_tunnel_config.telegram_bot_api_token,
            "123:bot"
        );
        assert_eq!(
            summary.jarvis_msg_tunnel_config.telegram_account_id,
            "@alice"
        );
    }

    #[test]
    fn build_aicc_settings_uses_supported_boot_tokens() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com",
            "ai_provider_config": {
                "openai_api_token": "sk-openai",
                "google_api_token": "google-token",
                "claude_api_token": "claude-token"
            }
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let settings = build_aicc_settings(&summary);

        assert_eq!(settings["openai"]["api_token"], "sk-openai");
        assert_eq!(settings["openai"]["alias_map"]["gpt-fast"], "gpt-5-mini");
        assert_eq!(
            settings["openai"]["instances"][0]["default_model"],
            "gpt-5-mini"
        );
        assert_eq!(settings["openai"]["instances"][0]["timeout_ms"], 600000);
        assert_eq!(settings["google"]["api_token"], "google-token");
        assert_eq!(
            settings["google"]["alias_map"]["gemini-ops"],
            "gemini-2.5-flash"
        );
        assert_eq!(
            settings["google"]["instances"][0]["provider_type"],
            "google-gimini"
        );
        assert_eq!(settings["google"]["instances"][0]["timeout_ms"], 600000);
        assert_eq!(settings["claude"]["api_token"], "claude-token");
        assert_eq!(
            settings["claude"]["alias_map"]["claude-reasoning"],
            "claude-3-7-sonnet-20250219"
        );
        assert_eq!(
            settings["claude"]["instances"][0]["default_model"],
            "claude-3-7-sonnet-20250219"
        );
    }

    #[test]
    fn build_aicc_settings_adds_sn_provider_when_active_code_present() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com",
            "sn_active_code": "invite-code-001"
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let settings = build_aicc_settings(&summary);

        assert_eq!(settings["openai"]["enabled"], true);
        assert_eq!(
            settings["openai"]["instances"][0]["instance_id"],
            "sn-openai-default"
        );
        assert_eq!(
            settings["openai"]["instances"][0]["provider_type"],
            "sn-openai"
        );
        assert_eq!(
            settings["openai"]["instances"][0]["base_url"],
            "https://sn.buckyos.ai/api/v1/ai/chat/completions"
        );
        assert_eq!(
            settings["openai"]["instances"][0]["auth_mode"],
            "device_jwt"
        );
        assert_eq!(settings["openai"]["alias_map"]["llm.plan.default"], "gpt-5");
    }

    #[test]
    fn extract_model_ids_from_response_supports_items_models_and_data_shapes() {
        let payload = json!({
            "items": [
                { "provider": "openai", "model": "gpt-5.4" },
                { "provider": "openai", "model": "gpt-5.4-mini" }
            ],
            "default_model": "gpt-5.4-mini",
            "models": ["legacy-ignored"],
            "data": [{ "model_id": "legacy-ignored" }]
        });
        let models = extract_model_ids_from_response(&payload);
        assert!(models.iter().any(|m| m == "gpt-5.4"));
        assert!(models.iter().any(|m| m == "gpt-5.4-mini"));
        assert!(!models.iter().any(|m| m == "legacy-ignored"));
    }

    #[test]
    fn build_aicc_settings_uses_fetched_sn_model_list() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com",
            "sn_active_code": "invite-code-001"
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");
        let sn_models = vec![
            "gpt-5".to_string(),
            "gpt-5-mini".to_string(),
            "gpt-image-1".to_string(),
        ];

        let settings = build_aicc_settings_with_sn_models(&summary, Some(sn_models.as_slice()));

        assert_eq!(
            settings["openai"]["instances"][0]["models"],
            json!(["gpt-5", "gpt-5-mini", "gpt-image-1"])
        );
        assert_eq!(
            settings["openai"]["instances"][0]["default_image_model"],
            "gpt-image-1"
        );
    }

    #[test]
    fn build_msg_center_settings_maps_jarvis_tunnel_to_bot_api() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com",
            "jarvis_msg_tunnel_config": {
                "telegram_bot_api_token": "123:bot",
                "telegram_account_id": "5397330802"
            }
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let settings = build_msg_center_settings(&summary).expect("build msg-center settings");

        assert_eq!(settings["telegram_tunnel"]["gateway"]["mode"], "bot_api");
        assert_eq!(
            settings["telegram_tunnel"]["tunnel_did"],
            "did:web:tg-tunnel.alice.example.com"
        );
        assert_eq!(
            settings["telegram_tunnel"]["bindings"][0]["owner_did"],
            "did:web:jarvis.alice.example.com"
        );
        assert_eq!(
            settings["telegram_tunnel"]["bindings"][0]["default_chat_id"],
            "5397330802"
        );
    }

    #[test]
    fn build_zone_user_contact_settings_maps_telegram_user_binding() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com",
            "jarvis_msg_tunnel_config": {
                "telegram_bot_api_token": "123:bot",
                "telegram_account_id": "5397330802"
            }
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let contact =
            build_zone_user_contact_settings(&summary).expect("build zone user contact settings");
        let contact = contact.expect("contact should exist");

        assert_eq!(contact.did.as_deref(), Some("did:bns:alice"));
        assert_eq!(contact.bindings.len(), 1);
        assert_eq!(contact.bindings[0].platform, "telegram");
        assert_eq!(contact.bindings[0].account_id, "user:5397330802");
        assert_eq!(
            contact.bindings[0].tunnel_id.as_deref(),
            Some("did:web:tg-tunnel.alice.example.com")
        );
    }

    #[test]
    fn build_default_jarvis_agent_spec_uses_agent_pkg() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com"
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let spec = build_default_jarvis_agent_spec(&summary).expect("build jarvis spec");
        assert_eq!(spec.user_id, "alice");
        assert_eq!(spec.app_doc.get_app_type(), buckyos_api::AppType::Agent);
        assert_eq!(spec.app_doc.show_name, "Jarvis");
        assert_eq!(spec.app_doc.name, "jarvis");
        assert_eq!(
            spec.app_doc.install_config_tips.service_ports.get("www"),
            Some(&OPENDAN_SERVICE_PORT)
        );
        assert!(spec.install_config.expose_config.contains_key("www"));
        assert_eq!(
            spec.app_doc
                .pkg_list
                .agent
                .as_ref()
                .map(|pkg| pkg.pkg_id.as_str()),
            Some("buckyos_jarvis")
        );
    }

    #[test]
    fn add_default_agents_writes_user_scoped_agent_spec() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com"
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let mut builder = SystemConfigBuilder::new(HashMap::new());
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        rt.block_on(builder.add_default_agents(&summary))
            .expect("add default agents");

        let entries = builder.build();
        let spec = entries
            .get("users/alice/agents/jarvis/spec")
            .expect("jarvis spec should exist");
        let spec: buckyos_api::AppServiceSpec =
            serde_json::from_str(spec).expect("parse jarvis spec");

        assert_eq!(spec.user_id, "alice");
        assert_eq!(spec.app_doc.name, "jarvis");
        assert_eq!(
            spec.app_doc.install_config_tips.service_ports.get("www"),
            Some(&OPENDAN_SERVICE_PORT)
        );
        assert!(spec.install_config.expose_config.contains_key("www"));
        assert_eq!(
            spec.app_doc
                .pkg_list
                .agent
                .as_ref()
                .map(|pkg| pkg.pkg_id.as_str()),
            Some("buckyos_jarvis")
        );
    }

    #[test]
    fn add_default_agents_removes_conflicting_legacy_app_spec() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com"
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let mut entries = HashMap::new();
        entries.insert(
            "users/alice/apps/jarvis/spec".to_string(),
            json!({
                "app_doc": {
                    "name": "jarvis",
                    "show_name": "Jarvis",
                    "categories": ["dapp"],
                    "pkg_list": {
                        "amd64_docker_image": {
                            "pkg_id": "jarvis#0.1.0"
                        }
                    },
                    "service_dock": {},
                    "permission": {},
                    "install_config_tips": {
                        "service_ports": {}
                    }
                },
                "app_index": 42,
                "user_id": "alice",
                "enable": true,
                "expected_instance_count": 1,
                "state": {},
                "install_config": {
                    "data_mount_point": {},
                    "cache_mount_point": [],
                    "local_cache_mount_point": [],
                    "service_ports": {},
                    "expose_config": {},
                    "bind_address": "0.0.0.0",
                    "res_pool_id": "default"
                }
            })
            .to_string(),
        );

        let mut builder = SystemConfigBuilder::new(entries);
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        rt.block_on(builder.add_default_agents(&summary))
            .expect("add default agents");

        let entries = builder.build();
        assert!(
            !entries.contains_key("users/alice/apps/jarvis/spec"),
            "legacy app spec should be removed"
        );
        assert!(
            entries.contains_key("users/alice/agents/jarvis/spec"),
            "agent spec should exist"
        );
    }

    #[test]
    fn build_kernel_service_spec_inserts_current_platform_native_pkg() {
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        let spec = rt
            .block_on(build_kernel_service_spec(
                "verify-hub",
                3300,
                1,
                generate_verify_hub_service_doc(),
            ))
            .expect("build kernel service spec");

        assert_eq!(
            spec.service_doc.pkg_list.get_app_pkg_id().as_deref(),
            Some("verify-hub")
        );
    }

    #[test]
    fn add_default_apps_uses_app_doc_from_preinstall_settings() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com"
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");
        let app_doc = sample_preinstall_app_doc("demo_app", "1.2.3");

        let mut entries = HashMap::new();
        entries.insert(
            "system/install_settings".to_string(),
            json!({
                "pre_install_apps": {
                    "demo_app": {
                        "app_doc": app_doc,
                        "data_mount_point": {},
                        "cache_mount_point": [],
                        "local_cache_mount_point": [],
                        "res_pool_id": "default"
                    }
                }
            })
            .to_string(),
        );

        let mut builder = SystemConfigBuilder::new(entries);
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        rt.block_on(builder.add_default_apps(&summary))
            .expect("add default apps");

        let entries = builder.build();
        let spec = entries
            .get("users/alice/apps/demo_app/spec")
            .expect("demo app spec should exist");
        let spec: AppServiceSpec = serde_json::from_str(spec).expect("parse app spec");

        assert_eq!(spec.user_id, "alice");
        assert_eq!(spec.app_doc.name, "demo_app");
        assert_eq!(spec.app_doc.version, "1.2.3");
    }

    #[test]
    fn add_default_apps_requires_app_doc_in_preinstall_settings() {
        let value = json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "public_key": {
                "kty": "OKP",
                "crv": "Ed25519",
                "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
            },
            "zone_name": "did:web:alice.example.com"
        });
        let summary = StartConfigSummary::from_value(&value).expect("parse start config");

        let mut entries = HashMap::new();
        entries.insert(
            "system/install_settings".to_string(),
            json!({
                "pre_install_apps": {
                    "demo_app": {
                        "data_mount_point": {},
                        "cache_mount_point": [],
                        "local_cache_mount_point": [],
                        "res_pool_id": "default"
                    }
                }
            })
            .to_string(),
        );

        let mut builder = SystemConfigBuilder::new(entries);
        let rt = tokio::runtime::Runtime::new().expect("create runtime");
        let err = match rt.block_on(builder.add_default_apps(&summary)) {
            Ok(_) => panic!("missing app_doc should fail"),
            Err(err) => err,
        };

        assert!(
            err.to_string().contains("app_doc"),
            "unexpected error: {}",
            err
        );
    }
}
