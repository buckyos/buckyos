use anyhow::{anyhow, Result};
use buckyos_api::load_local_node_identity_config;
use buckyos_api::msg_queue::{
    generate_kmsg_service_doc, KMSG_SERVICE_MAIN_PORT, KMSG_SERVICE_UNIQUE_ID,
};
use buckyos_api::{
    generate_aicc_service_doc, generate_control_panel_service_doc, generate_msg_center_service_doc,
    generate_opendan_service_doc, generate_repo_service_doc, generate_scheduler_service_doc,
    generate_smb_service_doc, generate_task_manager_service_doc, generate_verify_hub_service_doc,
    generate_workflow_service_doc, AgentId, AgentServiceBinding, AgentSpec, AppDoc, AppId,
    AppInstanceId, AppRegistry, BuckyOSDevConfig, BuckyOSInfo, GatewaySettings, GatewayShortcut,
    KernelServiceSpec, NodeConfig, NodeState, ServiceEndpointConfig, ServiceExposeConfig,
    ServiceExposeRouteConfig, ServiceInfo, ServiceInstanceReportInfo, ServiceInstanceState,
    ServiceNode, ServiceProtocol, ServiceSpecConfig, ServiceState, SubPkgDesc, UserContactSettings,
    UserPrivateProfile, UserProfile, UserSettings, UserState, UserTunnelBinding, UserType,
    ZoneConfig, AGENT_SPEC_SCHEMA_VERSION, APP_REGISTRY_KEY, BUCKYOS_DEV_CONFIG_KEY,
    BUCKYOS_INFO_KEY, OPENDAN_SERVICE_UNIQUE_ID, SCHEDULER_SERVICE_UNIQUE_ID, VERIFY_HUB_UNIQUE_ID,
    ZONE_OWNER_USER_ID_KEY,
};
use buckyos_api::{
    AICC_SERVICE_SERVICE_PORT, AICC_SERVICE_UNIQUE_ID, CONTROL_PANEL_SERVICE_PORT,
    CONTROL_PANEL_SERVICE_UNIQUE_ID, MSG_CENTER_SERVICE_PORT, MSG_CENTER_SERVICE_UNIQUE_ID,
    REPO_SERVICE_UNIQUE_ID, SMB_SERVICE_UNIQUE_ID, TASK_MANAGER_SERVICE_PORT,
    TASK_MANAGER_SERVICE_UNIQUE_ID, WORKFLOW_SERVICE_PORT, WORKFLOW_SERVICE_UNIQUE_ID,
};
use buckyos_kit::{
    buckyos_get_unix_timestamp, get_buckyos_system_etc_dir, get_channel, get_target, get_version,
};
use jsonwebtoken::jwk::Jwk;
use log::{debug, info, warn};
use name_lib::{generate_ed25519_key_pair, AgentDocument, OwnerDocument, VerifyHubInfo, DID};
use ndn_lib::build_named_object_by_json;
use package_lib::PackageId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::TryFrom;
use url::Url;

const DEFAULT_OOD_ID: &str = "ood1";
const DEFAULT_JARVIS_APP_DID: &str = "did:bns:jarvis.buckyos";
const PROFILE_SYSTEM_CONTACT_KEY: &str = "system_contact";
const DEFAULT_PROVIDER_TIMEOUT_MS: u64 = 600_000;
const SN_AI_PROVIDER_ACTIVATION_KEY: &str = "sn-ai-provider-activated";

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct SnAiProviderEndpoints {
    pub login_url: String,
    pub responses_url: String,
}

pub(crate) fn derive_sn_ai_provider_endpoints(
    zone_sn: Option<&str>,
) -> Result<SnAiProviderEndpoints> {
    let zone_sn = zone_sn
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("ZoneDocument.sn is required for the SN AI provider"))?;
    let normalized = if zone_sn.contains("://") {
        zone_sn.to_string()
    } else {
        format!("https://{zone_sn}")
    };
    let mut origin = Url::parse(&normalized)
        .map_err(|err| anyhow!("invalid ZoneDocument.sn {zone_sn:?}: {err}"))?;
    if origin.scheme() != "https" {
        return Err(anyhow!("ZoneDocument.sn must use https"));
    }
    if origin.host_str().is_none() {
        return Err(anyhow!("ZoneDocument.sn must include a host"));
    }
    if !origin.username().is_empty() || origin.password().is_some() {
        return Err(anyhow!("ZoneDocument.sn must not include user info"));
    }
    if !matches!(origin.path(), "" | "/") || origin.query().is_some() || origin.fragment().is_some()
    {
        return Err(anyhow!(
            "ZoneDocument.sn must be an HTTPS origin without path, query, or fragment"
        ));
    }

    origin.set_path("/");
    let login_url = origin
        .join("api/user/login_by_device_token")
        .map_err(|err| anyhow!("failed to derive SN user login URL: {err}"))?
        .to_string();
    let responses_url = origin
        .join("api/v1/ai/")
        .map_err(|err| anyhow!("failed to derive SN AI responses URL: {err}"))?
        .to_string();
    Ok(SnAiProviderEndpoints {
        login_url,
        responses_url,
    })
}

pub(crate) fn reconcile_managed_sn_ai_provider(
    current: &Value,
    endpoints: std::result::Result<&SnAiProviderEndpoints, &anyhow::Error>,
    user_name: Option<&str>,
) -> Result<Option<Value>> {
    let mut next = current.clone();
    let valid_config = match (endpoints.as_ref(), user_name.map(str::trim)) {
        (Ok(endpoints), Some(user_name)) if !user_name.is_empty() => Some((*endpoints, user_name)),
        _ => None,
    };
    let Some(root) = next.as_object_mut() else {
        return Ok(None);
    };
    let activated = root
        .get(SN_AI_PROVIDER_ACTIVATION_KEY)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !activated {
        return Ok(None);
    }
    if !root.contains_key("sn-ai-provider") {
        let Some((endpoints, user_name)) = valid_config else {
            return Ok(None);
        };
        root.insert(
            "sn-ai-provider".to_string(),
            json!({
                "enabled": true,
                "api_token": "",
                "alias_map": {},
                "instances": [managed_sn_ai_provider_instance(endpoints, user_name)]
            }),
        );
        return Ok(Some(next));
    }
    let Some(provider) = root
        .get_mut("sn-ai-provider")
        .and_then(Value::as_object_mut)
    else {
        return Ok(None);
    };
    if provider.get("enabled").and_then(Value::as_bool) != Some(true) {
        return Ok(None);
    }
    if provider
        .get("instances")
        .and_then(Value::as_array)
        .is_none()
    {
        if valid_config.is_none() {
            return Ok(None);
        }
        provider.insert("instances".to_string(), json!([]));
    }
    let instances = provider
        .get_mut("instances")
        .and_then(Value::as_array_mut)
        .expect("instances was normalized to an array");

    let mut managed_found = false;
    let mut changed = false;
    for instance in instances.iter_mut().filter_map(Value::as_object_mut) {
        let is_managed =
            instance.get("provider_driver").and_then(Value::as_str) == Some("sn-ai-provider");
        if !is_managed {
            continue;
        }
        managed_found = true;
        if let Ok(endpoints) = endpoints {
            if instance.get("base_url").and_then(Value::as_str)
                != Some(endpoints.responses_url.as_str())
            {
                instance.insert(
                    "base_url".to_string(),
                    Value::String(endpoints.responses_url.clone()),
                );
                changed = true;
            }
            if instance.get("login_url").and_then(Value::as_str)
                != Some(endpoints.login_url.as_str())
            {
                instance.insert(
                    "login_url".to_string(),
                    Value::String(endpoints.login_url.clone()),
                );
                changed = true;
            }
            if let Some(user_name) = user_name.map(str::trim).filter(|value| !value.is_empty()) {
                if instance.get("user_name").and_then(Value::as_str) != Some(user_name) {
                    instance.insert(
                        "user_name".to_string(),
                        Value::String(user_name.to_string()),
                    );
                    changed = true;
                }
            }
        }
    }

    if !managed_found {
        let Some((endpoints, user_name)) = valid_config else {
            return Ok(None);
        };
        instances.push(managed_sn_ai_provider_instance(endpoints, user_name));
        changed = true;
    }

    Ok(changed.then_some(next))
}

fn managed_sn_ai_provider_instance(endpoints: &SnAiProviderEndpoints, user_name: &str) -> Value {
    json!({
        "provider_instance_name": "sn-ai-provider-default",
        "provider_type": "cloud_api",
        "provider_driver": "sn-ai-provider",
        "base_url": endpoints.responses_url,
        "login_url": endpoints.login_url,
        "user_name": user_name,
        "timeout_ms": DEFAULT_PROVIDER_TIMEOUT_MS,
    })
}

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

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct EnabledFeaturesSummary {
    #[serde(default)]
    pub llm_router: bool,
}

#[derive(Debug, Deserialize)]
pub struct StartConfigSummary {
    pub user_name: String,
    pub admin_password_hash: String,
    pub owner_document: OwnerDocument,
    pub zone_name: String, //zone hostname
    #[serde(default)]
    pub sn_active_code: String,
    #[serde(default)]
    pub ood_jwt: Option<String>,
    #[serde(default)]
    pub enabled_features: EnabledFeaturesSummary,
    #[serde(default)]
    pub ai_provider_config: AIProviderConfigSummary,
    #[serde(default)]
    pub jarvis_msg_tunnel_config: JarvisMsgTunnelConfigSummary,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BootstrapAgentProvision {
    pub schema_version: u32,
    pub owner_user_id: String,
    pub agent_spec: AgentSpec,
    pub private_key_pem: String,
    pub settings: Value,
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
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: None,
        };
        self.insert_json("users/root/settings", &root_settings)?;

        let admin_key = format!("users/{}/settings", config.user_name);
        let admin_settings = UserSettings {
            user_type: UserType::Admin,
            user_id: config.user_name.clone(),
            password: config.admin_password_hash.clone(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: None,
        };
        self.insert_json(&admin_key, &admin_settings)?;
        self.insert_json_if_absent(ZONE_OWNER_USER_ID_KEY, &config.user_name)?;
        self.append_policy(&format!("g, {}, admin", config.user_name));
        Ok(self)
    }

    pub fn add_user_doc(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let owner_config = config.owner_document.clone();
        if owner_config.name != config.user_name {
            return Err(anyhow!(
                "OwnerDocument name {} does not match start config user {}",
                owner_config.name,
                config.user_name
            ));
        }

        let key = format!("users/{}/doc", config.user_name);
        self.insert_json(&key, &owner_config)?;

        let mut profile = UserPrivateProfile::from(UserProfile {
            did: owner_config.id.clone(),
            name: Some(owner_config.name.clone()),
            display_name: Some(owner_config.display_name.clone()),
            avatar: owner_config.avatar.clone(),
            meta: owner_config.meta.clone(),
            headline: None,
            bio: None,
            location: None,
            organization: None,
            title: None,
            birthday: None,
            tags: Vec::new(),
            bkg_image: None,
            links: HashMap::new(),
            public_contacts: HashMap::new(),
            extra: owner_config.extra_info.clone(),
        });
        if let Some(contact) = build_zone_user_contact_settings(config)? {
            profile.private_extra.insert(
                PROFILE_SYSTEM_CONTACT_KEY.to_string(),
                serde_json::to_value(contact)?,
            );
        }
        let profile_key = format!("users/{}/profile", config.user_name);
        self.insert_json_if_absent(&profile_key, &profile)?;
        Ok(self)
    }

    pub async fn add_default_agents(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        // Stage Jarvis as an Agent identity. Its OpenDAN App runtime is
        // independently installed by the rootfs pre-install PIKG reconciler.
        let zone_did = DID::from_str(&config.zone_name)?;
        let jarvis_did = DID::new(
            zone_did.method.as_str(),
            format!("jarvis.{}", zone_did.id.as_str()).as_str(),
        );
        let owner_did = config.owner_document.id.clone();

        let (jarvis_private_key_pem, jarvis_public_key_jwk) = generate_ed25519_key_pair();
        let jarvis_public_key_jwk: Jwk = serde_json::from_value(jarvis_public_key_jwk)
            .map_err(|e| anyhow!("invalid generated jarvis public key: {}", e))?;

        let mut jarvis_doc = AgentDocument::new(jarvis_did, owner_did, jarvis_public_key_jwk);
        jarvis_doc.public_description = Some("Default built-in OpenDAN agent for BuckyOS".into());

        let agent_id = AgentId::from_agent_did(&jarvis_doc.id).map_err(|error| anyhow!(error))?;
        let agent_doc_json = serde_json::to_value(&jarvis_doc)?;
        let (agent_doc_object_id, _) = build_named_object_by_json("agentdoc", &agent_doc_json);
        let runtime_instance_id = default_jarvis_runtime_instance_id(config)?;
        let agent_spec = AgentSpec {
            schema_version: AGENT_SPEC_SCHEMA_VERSION,
            agent_id: agent_id.clone(),
            agent_did: jarvis_doc.id.clone(),
            agent_doc_object_id: agent_doc_object_id.clone(),
            agent_doc: jarvis_doc.clone(),
            binding: AgentServiceBinding {
                schema_version: AGENT_SPEC_SCHEMA_VERSION,
                agent_did: jarvis_doc.id.clone(),
                agent_doc_object_id,
                target_app_instance_id: runtime_instance_id,
                service_name: "www".to_string(),
                generation: 1,
            },
            generation: 1,
        };
        agent_spec.validate().map_err(|error| anyhow!(error))?;
        let jarvis_settings = json!({
            "enabled": true,
            "auto_start": true
        });
        let provision = BootstrapAgentProvision {
            schema_version: AGENT_SPEC_SCHEMA_VERSION,
            owner_user_id: config.user_name.clone(),
            agent_spec,
            private_key_pem: jarvis_private_key_pem,
            settings: jarvis_settings,
        };
        self.insert_json(
            &format!("system/scheduler/bootstrap_agents/{agent_id}"),
            &provision,
        )?;
        Ok(self)
    }

    pub async fn add_default_apps(&mut self, _config: &StartConfigSummary) -> Result<&mut Self> {
        let install_settings = self.entries.get("system/install_settings");
        if install_settings.is_none() {
            return Err(anyhow!("system/install_settings not found"));
        }
        let install_settings: buckyos_api::SystemInstallSettings =
            serde_json::from_str(install_settings.unwrap())?;
        let _ = install_settings;

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
        let installed_at = buckyos_get_unix_timestamp();
        let release_channel = get_channel().to_string();
        let buckyos_info = BuckyOSInfo::from_runtime(
            get_version(),
            release_channel.as_str(),
            get_target(),
            installed_at,
        );
        buckyos_info.validate().map_err(anyhow::Error::msg)?;
        self.insert_json(BUCKYOS_INFO_KEY, &buckyos_info)?;
        self.insert_json(BUCKYOS_DEV_CONFIG_KEY, &BuckyOSDevConfig::default())?;
        self.insert_json("system/system_pkgs", &json!({}))?;
        self.insert_json_if_absent(APP_REGISTRY_KEY, &AppRegistry::default())?;
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
            "security/verify-hub/key".into(),
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
        config.spec_config.rdb_instances.insert(
            buckyos_api::TASK_MANAGER_RDB_INSTANCE_ID.to_string(),
            buckyos_api::task_manager_default_rdb_instance_config(),
        );
        // Task Dispatch Center rides in the same process/deployment unit but
        // owns an independent rdb instance (no shared tables/schema/joins).
        config.spec_config.rdb_instances.insert(
            buckyos_api::TASK_DISPATCHER_RDB_INSTANCE_ID.to_string(),
            buckyos_api::task_dispatcher_default_rdb_instance_config(),
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

    pub async fn add_aicc(
        &mut self,
        config: &StartConfigSummary,
        zone_sn: Option<&str>,
    ) -> Result<&mut Self> {
        let service_doc = generate_aicc_service_doc();
        let mut service_spec = build_kernel_service_spec(
            AICC_SERVICE_UNIQUE_ID,
            AICC_SERVICE_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        service_spec.spec_config.rdb_instances.insert(
            buckyos_api::AICC_USAGE_LOG_RDB_INSTANCE_ID.to_string(),
            buckyos_api::aicc_usage_log_default_rdb_instance_config(),
        );
        self.insert_json("services/aicc/spec", &service_spec)?;
        let sn_ai_provider_endpoints = if config.llm_router_enabled() {
            Some(derive_sn_ai_provider_endpoints(zone_sn)?)
        } else {
            None
        };
        let settings =
            build_aicc_settings_with_endpoints(config, sn_ai_provider_endpoints.as_ref());
        self.insert_json_if_absent("services/aicc/settings", &settings)?;
        Ok(self)
    }

    pub async fn add_msg_center(&mut self, config: &StartConfigSummary) -> Result<&mut Self> {
        let service_doc = generate_msg_center_service_doc();
        let mut service_spec = build_kernel_service_spec(
            MSG_CENTER_SERVICE_UNIQUE_ID,
            MSG_CENTER_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        service_spec.spec_config.rdb_instances.insert(
            buckyos_api::MSG_CENTER_RDB_INSTANCE_ID.to_string(),
            buckyos_api::msg_center_default_rdb_instance_config(),
        );
        self.insert_json("services/msg-center/spec", &service_spec)?;
        let settings = build_msg_center_settings(config)?;
        self.insert_json_if_absent("services/msg-center/settings", &settings)?;
        Ok(self)
    }

    pub async fn add_workflow(&mut self) -> Result<&mut Self> {
        let service_doc = generate_workflow_service_doc();
        let config = build_kernel_service_spec(
            WORKFLOW_SERVICE_UNIQUE_ID,
            WORKFLOW_SERVICE_PORT,
            1,
            service_doc,
        )
        .await?;
        self.insert_json("services/workflow/spec", &config)?;
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
                    GatewayShortcut::System {
                        service_id: "control-panel".to_string(),
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
        config.spec_config.rdb_instances.insert(
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
        let policy_str = self.entries.get("system/rbac/policy");
        if policy_str.is_none() {
            self.entries
                .insert("system/rbac/policy".to_string(), policy.to_string());
            return Ok(self);
        }
        let policy_str = policy_str.unwrap();
        let new_policy_str = format!("{}\n{}", policy_str, policy);
        self.entries
            .insert("system/rbac/policy".to_string(), new_policy_str);
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
        _config: &StartConfigSummary,
        verify_hub_public_key: &Jwk,
        zone_document: &str,
    ) -> Result<&mut Self> {
        let public_key_value = verify_hub_public_key.clone();
        let verify_hub_info = VerifyHubInfo {
            public_key: public_key_value,
        };
        let mut boot_config = ZoneConfig::new(zone_document.to_string());
        boot_config.verify_hub_info = Some(verify_hub_info);
        info!(
            "add_boot_config: boot_config: {}",
            serde_json::to_string_pretty(&boot_config)?
        );
        self.insert_json("boot/config", &boot_config)?;
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

fn default_jarvis_runtime_instance_id(config: &StartConfigSummary) -> Result<AppInstanceId> {
    let app_did = DID::from_str(DEFAULT_JARVIS_APP_DID)?;
    AppInstanceId::from_app_did(&app_did, &config.user_name).map_err(|error| anyhow!(error))
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

/// The Telegram tunnel instance's transport DID (DELIVERY_QUEUE owner).
fn resolve_telegram_transport_did(config: &StartConfigSummary) -> String {
    config
        .zone_name
        .strip_prefix("did:web:")
        .map(|zone_host| format!("did:web:tg-tunnel.{}", zone_host))
        .unwrap_or_else(|| "did:bns:msg-center-default-tunnel".to_string())
}

/// The stable tunnel instance id embedded in shadow endpoint DIDs. This is a
/// short logical id, never the transport DID.
const TELEGRAM_TUNNEL_INSTANCE_ID: &str = "tg-main-tunnel";

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

fn build_zone_user_contact_settings(
    config: &StartConfigSummary,
) -> Result<Option<UserContactSettings>> {
    let Some(account_id) =
        trim_to_option(config.jarvis_msg_tunnel_config.telegram_account_id.as_str())
    else {
        return Ok(None);
    };

    let normalized_account_id = normalize_telegram_contact_account_id(&account_id);
    if normalized_account_id.is_empty() {
        return Ok(None);
    }

    Ok(Some(UserContactSettings {
        did: Some(config.owner_document.id.to_string()),
        note: None,
        groups: vec!["users".to_string()],
        tags: vec!["zone_user".to_string()],
        bindings: vec![UserTunnelBinding {
            platform: "telegram".to_string(),
            account_id: normalized_account_id,
            display_id: Some(account_id),
            tunnel_instance_id: Some(TELEGRAM_TUNNEL_INSTANCE_ID.to_string()),
            status: None,
            last_sync_at: None,
            meta: HashMap::new(),
        }],
    }))
}

#[cfg(all(test, any()))]
fn build_aicc_settings(config: &StartConfigSummary) -> Value {
    let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.ai"))
        .expect("test SN AI endpoint must be valid");
    build_aicc_settings_with_endpoints(config, Some(&endpoints))
}

fn build_aicc_settings_with_endpoints(
    config: &StartConfigSummary,
    sn_ai_provider_endpoints: Option<&SnAiProviderEndpoints>,
) -> Value {
    let mut settings = serde_json::Map::new();
    let mut openai_alias_map = serde_json::Map::new();
    let mut openai_instances = Vec::<Value>::new();
    let sn_ai_provider_alias_map = serde_json::Map::new();
    let mut sn_ai_provider_instances = Vec::<Value>::new();
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

    if config.llm_router_enabled() {
        let endpoints = sn_ai_provider_endpoints
            .expect("SN AI endpoints are validated before building enabled provider settings");
        let instance = managed_sn_ai_provider_instance(endpoints, config.user_name.as_str());
        sn_ai_provider_instances.push(instance);
        settings.insert(SN_AI_PROVIDER_ACTIVATION_KEY.to_string(), Value::Bool(true));
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

    if !sn_ai_provider_instances.is_empty() {
        settings.insert(
            "sn-ai-provider".to_string(),
            json!({
                "enabled": true,
                "api_token": "",
                "alias_map": Value::Object(sn_ai_provider_alias_map),
                "instances": sn_ai_provider_instances
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
                        "instance_id": "google-gemini-default",
                        "provider_type": "google-gemini",
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

fn read_default_device_subject() -> String {
    let node_identity_path = get_buckyos_system_etc_dir().join("node_identity.json");
    if let Ok(node_identity) = load_local_node_identity_config(node_identity_path.as_path()) {
        return node_identity.device_name;
    }
    DEFAULT_OOD_ID.to_string()
}

fn build_msg_center_settings(config: &StartConfigSummary) -> Result<Value> {
    let transport_did = resolve_telegram_transport_did(config);
    let bot_token = trim_to_option(
        config
            .jarvis_msg_tunnel_config
            .telegram_bot_api_token
            .as_str(),
    );

    // No default_chat_id: the delivery address always comes from the target
    // shadow endpoint DID resolved at post_send time (no routing fallback).
    let (gateway_mode, bindings) = if let Some(bot_token) = bot_token {
        let jarvis_did = resolve_jarvis_agent_did(config)?;
        (
            "bot_api",
            vec![json!({
                "owner_did": jarvis_did.to_string(),
                "bot_token": bot_token
            })],
        )
    } else {
        ("dry_run", Vec::new())
    };

    Ok(json!({
        "telegram_tunnel": {
            "enabled": true,
            "transport_did": transport_did,
            "tunnel_instance_id": TELEGRAM_TUNNEL_INSTANCE_ID,
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
    attach_current_platform_service_pkg(&mut service_doc, pkg_name);

    let mut install_config = ServiceSpecConfig::default();
    let service_expose_config = ServiceExposeConfig {
        route: ServiceExposeRouteConfig::Web {
            sub_hostname: Vec::new(),
            expose_uri: Some(format!("/kapi/{}", pkg_name)),
        },
        scope: String::new(),
        allow_guest: false,
        bind_address: None,
    };
    install_config.service_config.insert(
        "www".to_string(),
        ServiceEndpointConfig {
            protocol: ServiceProtocol::Http,
            inner_port: port,
        },
    );
    install_config
        .expose_config
        .insert("www".to_string(), service_expose_config);

    Ok(KernelServiceSpec {
        service_doc,
        app_index: 0,
        enable: true,
        expected_instance_count,
        state: ServiceState::default(),
        spec_config: install_config,
    })
}

fn attach_current_platform_service_pkg(service_doc: &mut AppDoc, service_id: &str) {
    let current_pkg = SubPkgDesc::new(service_id.to_string());

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
        let owner_document: OwnerDocument = serde_json::from_value(
            value
                .get("owner_document")
                .cloned()
                .ok_or_else(|| anyhow!("start_config.json missing owner_document"))?,
        )
        .map_err(|e| anyhow!("Failed to parse OwnerDocument: {}", e))?;
        let sn_active_code = value
            .get("sn_active_code")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let mut enabled_features: EnabledFeaturesSummary = serde_json::from_value(
            value
                .get("enabled_features")
                .cloned()
                .unwrap_or_else(|| json!({})),
        )
        .map_err(|e| anyhow!("Failed to parse enabled_features: {}", e))?;
        if !enabled_features.llm_router && trim_to_option(sn_active_code.as_str()).is_some() {
            enabled_features.llm_router = true;
        }
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
            owner_document,
            zone_name: value
                .get("zone_name")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("start_config.json missing zone_name"))?
                .to_string(),
            sn_active_code,

            ood_jwt: value
                .get("ood_jwt")
                .and_then(Value::as_str)
                .map(|s| s.to_string()),
            enabled_features,
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

    pub fn llm_router_enabled(&self) -> bool {
        self.enabled_features.llm_router
    }
}

#[cfg(test)]
mod beta22_tests {
    use super::*;

    fn start_config() -> StartConfigSummary {
        StartConfigSummary::from_value(&json!({
            "user_name": "alice",
            "admin_password_hash": "hashed",
            "owner_document": {
                "@context": [
                    "https://www.w3.org/ns/did/v1",
                    "https://buckyos.org/ns/owner/v1"
                ],
                "id": "did:bns:alice",
                "verificationMethod": [{
                    "type": "Ed25519VerificationKey2020",
                    "id": "#main_key",
                    "controller": "did:bns:alice",
                    "publicKeyJwk": {
                        "kty": "OKP",
                        "crv": "Ed25519",
                        "x": "mWQ4l0Q4v0m2lj9g0WW4MZ6z9M0D7u2xN3Zf3nq4Lys"
                    }
                }],
                "authentication": ["#main_key"],
                "assertion_method": ["#main_key"],
                "capabilityInvocation": ["#main_key"],
                "exp": 4102444800u64,
                "iat": 1700000000u64,
                "name": "alice",
                "display_name": "Alice"
            },
            "zone_name": "did:web:alice.example.com"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn generated_kernel_service_uses_system_service_package_id() {
        let spec = build_kernel_service_spec(
            VERIFY_HUB_UNIQUE_ID,
            3300,
            1,
            generate_verify_hub_service_doc(),
        )
        .await
        .unwrap();
        let packages = spec.service_doc.pkg_list.iter();
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].1.pkg_id, VERIFY_HUB_UNIQUE_ID);
    }

    #[tokio::test]
    async fn bootstrap_preserves_preinstall_seed_without_execution_or_registry_allocation() {
        let config = start_config();
        let app_id = AppId::parse("app.buckyos.bns.did").unwrap();
        let preinstall = buckyos_api::PreInstallAppConfig {
            schema_version: buckyos_api::PRE_INSTALL_APP_SCHEMA_VERSION,
            pikg_path: "data/cache/app.buckyos.bns.did-1.0.0.pikg".to_string(),
            install_plan: buckyos_api::PreInstallPlanSeed::default(),
        };
        let mut builder = SystemConfigBuilder::new(HashMap::from([(
            "system/install_settings".to_string(),
            serde_json::to_string(&buckyos_api::SystemInstallSettings {
                pre_install_apps: HashMap::from([(app_id.to_string(), preinstall)]),
            })
            .unwrap(),
        )]));
        builder.add_system_defaults().unwrap();
        builder.add_default_apps(&config).await.unwrap();

        let registry: AppRegistry =
            serde_json::from_str(builder.entries.get(APP_REGISTRY_KEY).unwrap()).unwrap();
        assert!(registry.apps.is_empty());
        assert!(registry.instances.is_empty());
        assert!(!builder.entries.keys().any(|key| key.starts_with("users/")
            && key.contains("/apps/")
            && key.ends_with("/spec")));
        let records = builder
            .entries
            .iter()
            .filter(|(key, _)| key.starts_with("system/scheduler/install_plan_executions/"))
            .collect::<Vec<_>>();
        assert!(records.is_empty());
        let persisted: buckyos_api::SystemInstallSettings =
            serde_json::from_str(builder.entries.get("system/install_settings").unwrap()).unwrap();
        assert_eq!(persisted.pre_install_apps.len(), 1);
    }

    #[tokio::test]
    async fn bootstrap_stages_agent_binding_for_preinstalled_runtime() {
        let config = start_config();
        let mut builder = SystemConfigBuilder::new(HashMap::new());
        builder.add_system_defaults().unwrap();
        builder.add_default_agents(&config).await.unwrap();

        assert!(!builder
            .entries
            .keys()
            .any(|key| key.starts_with("users/") && key.ends_with("/spec")));
        assert_eq!(
            builder
                .entries
                .keys()
                .filter(|key| key.starts_with("system/scheduler/bootstrap_agents/"))
                .count(),
            1
        );
        assert_eq!(
            builder
                .entries
                .keys()
                .filter(|key| key.starts_with("system/scheduler/install_plan_executions/"))
                .count(),
            0
        );
        let provision = builder
            .entries
            .iter()
            .find(|(key, _)| key.starts_with("system/scheduler/bootstrap_agents/"))
            .map(|(_, value)| serde_json::from_str::<BootstrapAgentProvision>(value).unwrap())
            .unwrap();
        assert_eq!(
            provision
                .agent_spec
                .binding
                .target_app_instance_id
                .to_string(),
            format!("jarvis.buckyos.bns.did@{}", config.user_name)
        );
    }

    #[test]
    fn aicc_settings_persist_sn_router_activation() {
        let mut config = start_config();
        config.enabled_features.llm_router = true;
        let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.ai")).unwrap();

        let settings = build_aicc_settings_with_endpoints(&config, Some(&endpoints));

        assert_eq!(settings[SN_AI_PROVIDER_ACTIVATION_KEY], true);
        assert_eq!(settings["sn-ai-provider"]["enabled"], true);
    }

    #[test]
    fn reconcile_does_not_activate_unrequested_sn_router() {
        let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.ai")).unwrap();

        let reconciled =
            reconcile_managed_sn_ai_provider(&json!({}), Ok(&endpoints), Some("did:bns:alice"))
                .unwrap();

        assert!(reconciled.is_none());
    }

    #[test]
    fn reconcile_restores_activated_sn_router_section() {
        let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.ai")).unwrap();
        let current = json!({ (SN_AI_PROVIDER_ACTIVATION_KEY): true });

        let reconciled =
            reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("did:bns:alice"))
                .unwrap()
                .unwrap();

        assert_eq!(reconciled["sn-ai-provider"]["enabled"], true);
        assert_eq!(
            reconciled["sn-ai-provider"]["instances"][0]["user_name"],
            "did:bns:alice"
        );
    }

    #[test]
    fn reconcile_preserves_explicitly_disabled_sn_router() {
        let endpoints = derive_sn_ai_provider_endpoints(Some("sn.buckyos.ai")).unwrap();
        let current = json!({
            (SN_AI_PROVIDER_ACTIVATION_KEY): true,
            "sn-ai-provider": {
                "enabled": false,
                "instances": []
            }
        });

        let reconciled =
            reconcile_managed_sn_ai_provider(&current, Ok(&endpoints), Some("did:bns:alice"))
                .unwrap();

        assert!(reconciled.is_none());
    }

    #[test]
    fn reconcile_keeps_activated_config_when_endpoint_is_temporarily_invalid() {
        let current = json!({
            (SN_AI_PROVIDER_ACTIVATION_KEY): true,
            "sn-ai-provider": {
                "enabled": true,
                "instances": [{
                    "provider_driver": "sn-ai-provider",
                    "base_url": "https://sn.example/api/v1/ai/"
                }]
            }
        });
        let endpoint_error = anyhow!("temporary endpoint error");

        let reconciled =
            reconcile_managed_sn_ai_provider(&current, Err(&endpoint_error), None).unwrap();

        assert!(reconciled.is_none());
    }
}
