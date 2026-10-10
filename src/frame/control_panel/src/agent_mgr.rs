//! Agent identities and their constructed runtime Apps.
//!
//! Creating an Agent reserves its identity, builds an App from the selected
//! Agent template whose AppDID is the AgentDID, installs that App for the
//! creator, binds the AgentSpec to it and waits for the Loader to report the
//! Agent loaded. Every step is a CAS transition of the Agent install record,
//! so the background driver can resume from any persisted state.

use crate::app_installer::{register_app_doc_authority, AppInstaller, InternalInstall};
use crate::pikg::{preferred_archive_name, PikgBuilder, PikgReader};
use crate::pre_install_reconciler::{
    canonical_pikg_path, INSTALL_SETTINGS_KEY, PREINSTALL_PIKG_DIR,
};
use crate::user_mgr::{profile_system_contact, refresh_rbac_by_scheduler};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use async_trait::async_trait;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use buckyos_api::{
    agent_info_key, agent_install_record_key, agent_key_path, agent_profile_key,
    agent_settings_key, agent_short_name, agent_spec_key, agent_template_record_key,
    get_buckyos_api_runtime, user_app_spec_key, AgentCreateError, AgentCreateStep, AgentId,
    AgentInstallRecord, AgentInstallState, AgentMsgTunnelSettings, AgentProfile, AgentRuntimeInfo,
    AgentServiceBinding, AgentSettings, AgentSpec, AgentTemplateConfig, AgentTemplateRecord,
    AgentTemplateSource, AgentTunnelState, AppDataDisposition, AppDoc, AppId, AppInstallTaskData,
    AppInstanceId, AppRegistry, AppServiceSpec, AppType, AppUpdateTaskData, InstallError,
    InstallErrorCode, InstallInspection, InstallSource, InstallSourceIdentity, InstallStage,
    PikgStagingPurpose, PreInstallPlanSeed, SystemConfigClient, SystemConfigError,
    SystemInstallSettings, TaskOutcome, TaskPhase, UserPrivateProfile, UserType,
    AGENT_SPEC_SCHEMA_VERSION, APP_INSTALL_TASK_SCHEMA_ID, APP_REGISTRY_KEY,
    APP_UNINSTALL_TASK_SCHEMA_ID, APP_UPDATE_TASK_SCHEMA_ID, MSG_CENTER_SERVICE_NAME,
};
use buckyos_kit::{buckyos_get_unix_timestamp, get_buckyos_root_dir, KVAction};
use futures::future::BoxFuture;
use jsonwebtoken::jwk::Jwk;
use log::{info, warn};
use name_lib::{generate_ed25519_key_pair, AgentDocument, DID};
use ndn_lib::{build_named_object_by_json, ObjId};
use package_lib::PackageId;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const AGENT_TEMPLATE_DIR: &str = "data/srv/control-panel/agent_templates";
const AGENT_NAME_PREFIX: &str = "services/control_panel/agent_names";
const GATEWAY_SETTINGS_KEY: &str = "services/gateway/settings";
const DEFAULT_TEMPLATE_APP_ID: &str = "jarvis.buckyos.bns.did";
const AGENT_SERVICE_NAME: &str = "www";
const AGENT_LOADER: &str = "opendan";
const INTERNAL_APP_ID: &str = "system:control-panel";
const TELEGRAM: &str = "telegram";
const TELEGRAM_API: &str = "https://api.telegram.org";
const TELEGRAM_TIMEOUT: Duration = Duration::from_secs(15);
const RESERVED_NAMES: [&str; 8] = [
    "root",
    "system",
    "admin",
    "guest",
    "_",
    "www",
    "sys",
    "homestation",
];
const NAME_SUGGESTION_LIMIT: usize = 50;
const MAX_AVATAR_PX: u32 = 192;
const MAX_AVATAR_BYTES: usize = 512 * 1024;
const START_TIMEOUT_SECS: u64 = 5 * 60;
const DRIVE_POLL: Duration = Duration::from_secs(2);
const DRIVE_SWEEP: Duration = Duration::from_secs(30);
const MAX_DRIVE_STEPS: usize = 64;

fn reason(code: &str, message: impl Display) -> RPCErrors {
    RPCErrors::ReasonError(format!("{code}: {message}"))
}

fn to_json<T: Serialize>(value: &T) -> Result<String, RPCErrors> {
    serde_json::to_string(value)
        .map_err(|error| RPCErrors::ReasonError(format!("serialize failed: {error}")))
}

// ---------------------------------------------------------------------------
// system-config access
// ---------------------------------------------------------------------------

#[async_trait]
pub(crate) trait AgentKv: Send + Sync {
    async fn get(&self, key: &str) -> Result<Option<(String, u64)>, RPCErrors>;
    async fn list(&self, key: &str) -> Result<Vec<String>, RPCErrors>;
    async fn exec_tx(
        &self,
        actions: HashMap<String, KVAction>,
        main_key: Option<(String, u64)>,
    ) -> Result<(), RPCErrors>;
}

pub(crate) struct ZoneKv(Arc<SystemConfigClient>);

impl ZoneKv {
    pub(crate) async fn connect() -> Result<Self, RPCErrors> {
        Ok(Self(
            get_buckyos_api_runtime()?
                .get_system_config_client()
                .await?,
        ))
    }
}

#[async_trait]
impl AgentKv for ZoneKv {
    async fn get(&self, key: &str) -> Result<Option<(String, u64)>, RPCErrors> {
        self.0.invalidate_cache(key).await;
        match self.0.get(key).await {
            Ok(value) => Ok(Some((value.value, value.version))),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
            Err(error) => Err(RPCErrors::ReasonError(error.to_string())),
        }
    }

    async fn list(&self, key: &str) -> Result<Vec<String>, RPCErrors> {
        self.0
            .list(key)
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))
    }

    async fn exec_tx(
        &self,
        actions: HashMap<String, KVAction>,
        main_key: Option<(String, u64)>,
    ) -> Result<(), RPCErrors> {
        self.0
            .exec_tx(actions, main_key)
            .await
            .map(|_| ())
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))
    }
}

async fn get_json<T: DeserializeOwned>(
    kv: &dyn AgentKv,
    key: &str,
) -> Result<Option<(T, u64)>, RPCErrors> {
    match kv.get(key).await? {
        Some((raw, version)) => serde_json::from_str(&raw)
            .map(|value| Some((value, version)))
            .map_err(|error| RPCErrors::ReasonError(format!("invalid `{key}`: {error}"))),
        None => Ok(None),
    }
}

async fn get_value<T: DeserializeOwned>(kv: &dyn AgentKv, key: &str) -> Option<T> {
    match kv.get(key).await {
        Ok(Some((raw, _))) => serde_json::from_str(&raw).ok(),
        _ => None,
    }
}

fn reservation_key(agent_id: &AgentId) -> String {
    format!("{AGENT_NAME_PREFIX}/{agent_id}")
}

async fn load_record(
    kv: &dyn AgentKv,
    owner: &str,
    agent_id: &AgentId,
) -> Result<Option<(AgentInstallRecord, u64)>, RPCErrors> {
    get_json(kv, &agent_install_record_key(owner, agent_id)).await
}

async fn list_records(
    kv: &dyn AgentKv,
    owners: Option<&str>,
) -> Result<Vec<(String, AgentInstallRecord)>, RPCErrors> {
    let owners = match owners {
        Some(owner) => vec![owner.to_string()],
        None => kv.list("users").await?,
    };
    let mut records = Vec::new();
    for owner in owners {
        for raw in kv.list(&format!("users/{owner}/agents")).await? {
            let Ok(agent_id) = AgentId::parse(&raw) else {
                continue;
            };
            match load_record(kv, &owner, &agent_id).await {
                Ok(Some((record, _))) => records.push((owner.clone(), record)),
                Ok(None) => {}
                Err(error) => warn!("skip Agent {owner}/{agent_id}: {error}"),
            }
        }
    }
    Ok(records)
}

/// Commits `record` (CAS on its revision) together with `extra` actions.
/// Returns the new record revision, or None when another writer won.
async fn commit_record(
    kv: &dyn AgentKv,
    owner: &str,
    record: &AgentInstallRecord,
    version: u64,
    mut extra: HashMap<String, KVAction>,
) -> Result<Option<u64>, RPCErrors> {
    let key = agent_install_record_key(owner, &record.agent_id);
    extra.insert(key.clone(), KVAction::Update(to_json(record)?));
    if let Err(error) = kv.exec_tx(extra, Some((key.clone(), version))).await {
        return match kv.get(&key).await? {
            Some((_, current)) if current != version => Ok(None),
            None => Ok(None),
            _ => Err(error),
        };
    }
    Ok(kv.get(&key).await?.map(|(_, current)| current))
}

// ---------------------------------------------------------------------------
// names
// ---------------------------------------------------------------------------

fn is_dns_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 63
        && !value.starts_with('-')
        && !value.ends_with('-')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn dns_label_of(value: &str) -> Option<String> {
    let mut label = String::new();
    for byte in value.to_ascii_lowercase().bytes() {
        let next = if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            byte as char
        } else {
            '-'
        };
        if next != '-' || !label.ends_with('-') {
            label.push(next);
        }
    }
    let label = label.trim_matches('-');
    is_dns_label(label).then(|| label.to_string())
}

fn agent_did_for(zone: &DID, name: &str) -> DID {
    DID::new(zone.method.as_str(), &format!("{name}.{}", zone.id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NameConflict {
    pub reason: &'static str,
    pub message: String,
}

impl NameConflict {
    fn new(reason: &'static str, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
        }
    }

    fn into_rpc(self) -> RPCErrors {
        if self.reason == "invalid" {
            reason("invalid_name", self.message)
        } else {
            reason("name_conflict", self.message)
        }
    }
}

/// Snapshot of every name that a new user or Agent must not collide with.
pub(crate) struct AgentNameIndex {
    zone: DID,
    users: HashSet<String>,
    agents: HashSet<String>,
    hostnames: HashMap<String, Option<String>>,
}

impl AgentNameIndex {
    pub(crate) async fn load(kv: &dyn AgentKv, zone: &DID) -> Result<Self, RPCErrors> {
        let users: HashSet<String> = kv.list("users").await?.into_iter().collect();
        let mut agents: HashSet<String> = kv.list(AGENT_NAME_PREFIX).await?.into_iter().collect();
        for user in &users {
            agents.extend(kv.list(&format!("users/{user}/agents")).await?);
        }
        let mut hostnames = HashMap::new();
        if let Some((registry, _)) = get_json::<AppRegistry>(kv, APP_REGISTRY_KEY).await? {
            for (app_id, app) in registry.apps {
                hostnames.insert(app.app_name, Some(app_id.to_string()));
            }
            for allocation in registry.instances.into_values() {
                hostnames.insert(
                    allocation.app_host_name,
                    Some(allocation.app_id.to_string()),
                );
            }
        }
        if let Some((settings, _)) = get_json::<Value>(kv, GATEWAY_SETTINGS_KEY).await? {
            if let Some(shortcuts) = settings.get("shortcuts").and_then(Value::as_object) {
                for name in shortcuts.keys() {
                    hostnames.insert(name.clone(), None);
                }
            }
        }
        Ok(Self {
            zone: zone.clone(),
            users,
            agents,
            hostnames,
        })
    }

    pub(crate) fn check(&self, name: &str) -> Result<AgentId, NameConflict> {
        if !is_dns_label(name) {
            return Err(NameConflict::new(
                "invalid",
                "use 1-63 lowercase letters, digits or hyphens, not starting or ending with a hyphen",
            ));
        }
        if RESERVED_NAMES.contains(&name) {
            return Err(NameConflict::new(
                "reserved",
                format!("`{name}` is reserved by the system"),
            ));
        }
        let agent_id = AgentId::from_agent_did(&agent_did_for(&self.zone, name))
            .map_err(|error| NameConflict::new("invalid", error))?;
        if self.users.contains(name) {
            return Err(NameConflict::new(
                "user_exists",
                format!("a user already uses `{name}`"),
            ));
        }
        if self.agents.contains(agent_id.as_str()) {
            return Err(NameConflict::new(
                "agent_exists",
                format!("an Agent already uses `{name}`"),
            ));
        }
        if let Some(app_id) = self.hostnames.get(name) {
            if app_id.as_deref() != Some(agent_id.as_str()) {
                return Err(NameConflict::new(
                    "host_taken",
                    format!("an App already uses `{name}` as its host name"),
                ));
            }
        }
        Ok(agent_id)
    }

    pub(crate) fn suggest(&self, owner: &str, name: &str) -> Option<String> {
        let base = format!("{}-{name}", dns_label_of(owner)?);
        (1..=NAME_SUGGESTION_LIMIT)
            .map(|index| {
                if index == 1 {
                    base.clone()
                } else {
                    format!("{base}-{index}")
                }
            })
            .find(|candidate| self.check(candidate).is_ok())
    }
}

/// Shared by `user.create` and `agent.create`: users and Agents share one
/// `<name>.<zone>` DID namespace with App host names.
pub(crate) async fn ensure_name_available(zone: &DID, name: &str) -> Result<(), RPCErrors> {
    let kv = ZoneKv::connect().await?;
    AgentNameIndex::load(&kv, zone)
        .await?
        .check(name)
        .map(|_| ())
        .map_err(NameConflict::into_rpc)
}

// ---------------------------------------------------------------------------
// templates
// ---------------------------------------------------------------------------

fn template_id_of(source: AgentTemplateSource, app_id: &AppId) -> String {
    match source {
        AgentTemplateSource::Bundled => format!("bundled:{app_id}"),
        AgentTemplateSource::Installed => format!("installed:{app_id}"),
    }
}

pub(crate) fn parse_template_id(raw: &str) -> Result<(AgentTemplateSource, AppId), RPCErrors> {
    let not_found = || reason("template_not_found", raw);
    let (source, app_id) = raw.split_once(':').ok_or_else(not_found)?;
    let source = match source {
        "bundled" => AgentTemplateSource::Bundled,
        "installed" => AgentTemplateSource::Installed,
        _ => return Err(not_found()),
    };
    Ok((source, AppId::parse(app_id).map_err(|_| not_found())?))
}

pub(crate) struct AgentTemplate {
    pub source: AgentTemplateSource,
    pub app_id: AppId,
    pub reader: PikgReader,
}

impl AgentTemplate {
    fn app_doc(&self) -> &AppDoc {
        &self.reader.inspection().app_doc
    }

    fn object_id(&self) -> &ObjId {
        &self.reader.inspection().app_doc_object_id
    }

    fn template_id(&self) -> String {
        template_id_of(self.source, &self.app_id)
    }

    fn built_from(&self) -> BuiltFromTemplate {
        BuiltFromTemplate {
            app_did: self.app_doc().app_did().clone(),
            app_doc_object_id: self.object_id().clone(),
            version: self.app_doc().version.clone(),
        }
    }

    fn summary(&self) -> Value {
        let doc = self.app_doc();
        let description = doc
            .presentation
            .as_ref()
            .and_then(|presentation| {
                presentation.description.get("en").cloned().or_else(|| {
                    presentation
                        .description
                        .iter()
                        .min_by(|left, right| left.0.cmp(right.0))
                        .map(|(_, text)| text.clone())
                })
            })
            .unwrap_or_default();
        json!({
            "template_id": self.template_id(),
            "source": self.source,
            "app_id": self.app_id,
            "app_did": doc.app_did().to_string(),
            "name": self.app_id.as_str().split('.').next().unwrap_or_default(),
            "show_name": doc.show_name,
            "description": description,
            "version": doc.version,
            "icon": doc.app_icon_url(),
            "loader": AGENT_LOADER,
            "is_default": self.source == AgentTemplateSource::Bundled
                && self.app_id.as_str() == DEFAULT_TEMPLATE_APP_ID,
        })
    }
}

fn template_error(error: impl Display) -> RPCErrors {
    reason("template_invalid", error)
}

async fn open_bundled_template(
    root: &Path,
    config: &AgentTemplateConfig,
) -> Result<PikgReader, RPCErrors> {
    config.validate().map_err(template_error)?;
    let path = canonical_pikg_path(root, &config.pikg_path, PREINSTALL_PIKG_DIR)
        .await
        .map_err(|error| template_error(error.message))?;
    PikgReader::open(&path, None).await.map_err(template_error)
}

fn validate_template(reader: &PikgReader, app_id: &AppId) -> Result<(), RPCErrors> {
    let doc = &reader.inspection().app_doc;
    if doc.get_app_type() != AppType::Agent {
        return Err(template_error(format!("{app_id} is not an Agent template")));
    }
    if AppId::from_app_did(doc.app_did()).as_ref() != Ok(app_id) {
        return Err(template_error(format!(
            "{app_id} does not match its AppDID {}",
            doc.app_did().to_string()
        )));
    }
    if !doc
        .service_config_tips
        .service_endpoints
        .contains_key(AGENT_SERVICE_NAME)
    {
        return Err(template_error(format!(
            "{app_id} has no `{AGENT_SERVICE_NAME}` service"
        )));
    }
    Ok(())
}

async fn load_install_settings(kv: &dyn AgentKv) -> Result<SystemInstallSettings, RPCErrors> {
    get_json::<SystemInstallSettings>(kv, INSTALL_SETTINGS_KEY)
        .await?
        .map(|(settings, _)| settings)
        .ok_or_else(|| reason("template_unavailable", "system/install_settings is missing"))
}

pub(crate) async fn load_agent_template(
    kv: &dyn AgentKv,
    root: &Path,
    owner: &str,
    template_id: &str,
) -> Result<AgentTemplate, RPCErrors> {
    let (source, app_id) = parse_template_id(template_id)?;
    let reader = match source {
        AgentTemplateSource::Bundled => {
            let settings = load_install_settings(kv).await?;
            let config = settings
                .agent_templates
                .get(app_id.as_str())
                .ok_or_else(|| reason("template_not_found", template_id))?;
            open_bundled_template(root, config).await?
        }
        AgentTemplateSource::Installed => {
            let (record, _) =
                get_json::<AgentTemplateRecord>(kv, &agent_template_record_key(owner, &app_id))
                    .await?
                    .ok_or_else(|| reason("template_not_found", template_id))?;
            let path = canonical_pikg_path(root, &record.pikg_path, AGENT_TEMPLATE_DIR)
                .await
                .map_err(|error| template_error(error.message))?;
            PikgReader::open(&path, Some(&record.digest))
                .await
                .map_err(template_error)?
        }
    };
    validate_template(&reader, &app_id)?;
    Ok(AgentTemplate {
        source,
        app_id,
        reader,
    })
}

pub(crate) async fn list_agent_templates(
    kv: &dyn AgentKv,
    root: &Path,
    owner: &str,
) -> Result<Vec<Value>, RPCErrors> {
    let mut bundled = load_install_settings(kv)
        .await?
        .agent_templates
        .into_iter()
        .collect::<Vec<_>>();
    bundled.sort_by(|left, right| left.0.cmp(&right.0));
    let mut templates = Vec::new();
    for (raw_app_id, config) in bundled {
        let app_id = AppId::parse(&raw_app_id).map_err(template_error)?;
        let reader = open_bundled_template(root, &config).await?;
        validate_template(&reader, &app_id)?;
        templates.push(
            AgentTemplate {
                source: AgentTemplateSource::Bundled,
                app_id,
                reader,
            }
            .summary(),
        );
    }
    let mut installed = kv.list(&format!("users/{owner}/agent_templates")).await?;
    installed.sort();
    for raw_app_id in installed {
        let app_id = AppId::parse(&raw_app_id).map_err(template_error)?;
        let template_id = template_id_of(AgentTemplateSource::Installed, &app_id);
        templates.push(
            load_agent_template(kv, root, owner, &template_id)
                .await?
                .summary(),
        );
    }
    Ok(templates)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TemplateStoreAction {
    Satisfied,
    Registered,
    Updated,
}

impl TemplateStoreAction {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Satisfied => "satisfied",
            Self::Registered => "template_registered",
            Self::Updated => "template_updated",
        }
    }
}

/// Copies an inspected template PIKG out of the short-lived staging area into
/// the owner's template store and records it.
pub(crate) async fn store_agent_template(
    kv: &dyn AgentKv,
    root: &Path,
    owner: &str,
    app_doc: &AppDoc,
    app_doc_object_id: &ObjId,
    digest: &str,
    source: &Path,
    now: u64,
) -> Result<TemplateStoreAction, RPCErrors> {
    let app_id = AppId::from_app_did(app_doc.app_did()).map_err(template_error)?;
    let key = agent_template_record_key(owner, &app_id);
    let existing = get_json::<AgentTemplateRecord>(kv, &key).await?;
    let relative = format!("{AGENT_TEMPLATE_DIR}/{owner}/{app_id}/{digest}.pikg");
    let action = match existing.as_ref() {
        None => TemplateStoreAction::Registered,
        Some((record, _)) if &record.app_doc_object_id == app_doc_object_id => {
            if root.join(&record.pikg_path).is_file() {
                return Ok(TemplateStoreAction::Satisfied);
            }
            TemplateStoreAction::Satisfied
        }
        Some(_) => TemplateStoreAction::Updated,
    };

    let target = root.join(&relative);
    let parent = target
        .parent()
        .ok_or_else(|| template_error("invalid template path"))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|error| reason("template_store_failed", error))?;
    let tmp = parent.join(format!(".{digest}.tmp"));
    tokio::fs::copy(source, &tmp)
        .await
        .map_err(|error| reason("template_store_failed", error))?;
    if let Err(error) = PikgReader::open(&tmp, Some(digest)).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(template_error(error));
    }
    tokio::fs::rename(&tmp, &target)
        .await
        .map_err(|error| reason("template_store_failed", error))?;

    let record = AgentTemplateRecord {
        app_did: app_doc.app_did().clone(),
        app_doc_object_id: app_doc_object_id.clone(),
        version: app_doc.version.clone(),
        digest: digest.to_string(),
        pikg_path: relative.clone(),
        installed_at: now,
    };
    let raw = to_json(&record)?;
    let (action_kv, main_key) = match existing.as_ref() {
        Some((_, version)) => (KVAction::Update(raw), Some((key.clone(), *version))),
        None => (KVAction::Create(raw), None),
    };
    kv.exec_tx(HashMap::from([(key, action_kv)]), main_key)
        .await?;
    if let Some((old, _)) = existing {
        if old.pikg_path != relative {
            let _ = tokio::fs::remove_file(root.join(old.pikg_path)).await;
        }
    }
    Ok(action)
}

// ---------------------------------------------------------------------------
// constructed runtime App
// ---------------------------------------------------------------------------

pub(crate) struct AgentRuntimePikg {
    pub app_doc: AppDoc,
    pub app_doc_object_id: ObjId,
    pub path: PathBuf,
}

fn rebase_package_name(
    name: &str,
    template_app_id: &AppId,
    agent_id: &AgentId,
) -> Result<String, RPCErrors> {
    if name == template_app_id.as_str() {
        return Ok(agent_id.to_string());
    }
    name.strip_suffix(&format!(".{template_app_id}"))
        .map(|prefix| format!("{prefix}.{agent_id}"))
        .ok_or_else(|| {
            template_error(format!(
                "package `{name}` is outside template namespace {template_app_id}"
            ))
        })
}

/// Builds the Agent's own App from a template: the AppDoc is the template's
/// with the AgentDID as AppDID and the Zone as owner; package metadata is
/// rebuilt in the new AppId namespace while payloads are reused unchanged.
pub(crate) async fn build_agent_runtime_pikg(
    template: &PikgReader,
    agent_id: &AgentId,
    show_name: &str,
    work_dir: &Path,
) -> Result<AgentRuntimePikg, RPCErrors> {
    let inspection = template.inspection();
    let template_doc = &inspection.app_doc;
    let template_app_id = AppId::from_app_did(template_doc.app_did()).map_err(template_error)?;
    let agent_did = agent_id.agent_did();
    let zone_did = agent_did
        .upper_did()
        .ok_or_else(|| template_error("AgentDID has no Zone"))?;
    let mut app_doc = template_doc.clone();
    app_doc.did = Some(agent_did);
    app_doc.name = agent_short_name(agent_id);
    app_doc.show_name = show_name.to_string();
    app_doc.owner = zone_did;
    app_doc.base_on = Some(inspection.app_doc_object_id.clone());

    tokio::fs::create_dir_all(work_dir)
        .await
        .map_err(|error| reason("runtime_build_failed", error))?;
    let mut builder = PikgBuilder::new();
    for (sub_pkg_name, desc) in template_doc.pkg_list.iter() {
        let meta_id = desc
            .pkg_objid
            .as_ref()
            .ok_or_else(|| template_error(format!("package `{sub_pkg_name}` has no meta")))?;
        let mut meta = inspection
            .package_meta
            .package_objects
            .get(&meta_id.to_string())
            .cloned()
            .ok_or_else(|| {
                template_error(format!("package meta of `{sub_pkg_name}` is not bundled"))
            })?;
        let package_id = PackageId::parse(&desc.pkg_id).map_err(template_error)?;
        let name = rebase_package_name(&package_id.name, &template_app_id, agent_id)?;
        meta["name"] = json!(name);
        meta["version"] = json!(app_doc.version);
        let (next, new_meta_id) = builder
            .add_package_meta_value(meta)
            .map_err(template_error)?;
        builder = next;
        let mut new_desc = desc.clone();
        new_desc.pkg_id = format!("{name}#{}", app_doc.version);
        new_desc.pkg_objid = Some(new_meta_id);
        new_desc.source_url = None;
        AppInstaller::set_sub_pkg_desc(&mut app_doc, &sub_pkg_name, new_desc)?;
        if let Some(digest) = inspection
            .package_meta
            .content_index
            .iter()
            .find(|(_, entry)| entry.sub_pkg_name == sub_pkg_name)
            .map(|(digest, _)| digest.clone())
        {
            let payload = work_dir.join(preferred_archive_name(&sub_pkg_name));
            template
                .copy_content_to_file(&digest, &payload)
                .await
                .map_err(template_error)?;
            builder = builder
                .add_payload_file(sub_pkg_name.as_str(), payload)
                .map_err(template_error)?;
        }
    }
    app_doc.validate()?;
    let path = work_dir.join(format!("{agent_id}.pikg"));
    builder
        .app_doc(&app_doc)
        .map_err(template_error)?
        .write_to(&path)
        .await
        .map_err(|error| reason("runtime_build_failed", error))?;
    let reader = PikgReader::open(&path, None)
        .await
        .map_err(|error| reason("runtime_build_failed", error))?;
    reader
        .verify_all_contents()
        .await
        .map_err(|error| reason("runtime_build_failed", error))?;
    Ok(AgentRuntimePikg {
        app_doc,
        app_doc_object_id: reader.inspection().app_doc_object_id.clone(),
        path,
    })
}

fn new_agent_spec(
    agent_id: &AgentId,
    owner: &str,
    owner_did: DID,
    public_key: Jwk,
) -> Result<AgentSpec, RPCErrors> {
    let agent_did = agent_id.agent_did();
    let agent_doc = AgentDocument::new(agent_did.clone(), owner_did, public_key);
    let doc_value = serde_json::to_value(&agent_doc)
        .map_err(|error| RPCErrors::ReasonError(format!("serialize AgentDocument: {error}")))?;
    let (agent_doc_object_id, _) = build_named_object_by_json("agentdoc", &doc_value);
    let target = AppInstanceId::new(
        AppId::parse(agent_id.as_str()).map_err(RPCErrors::ReasonError)?,
        owner,
    )
    .map_err(RPCErrors::ReasonError)?;
    let spec = AgentSpec {
        schema_version: AGENT_SPEC_SCHEMA_VERSION,
        agent_id: agent_id.clone(),
        agent_did: agent_did.clone(),
        agent_doc_object_id: agent_doc_object_id.clone(),
        agent_doc,
        binding: AgentServiceBinding {
            schema_version: AGENT_SPEC_SCHEMA_VERSION,
            agent_did,
            agent_doc_object_id,
            target_app_instance_id: target,
            service_name: AGENT_SERVICE_NAME.to_string(),
            generation: 1,
        },
        generation: 1,
    };
    spec.validate().map_err(RPCErrors::ReasonError)?;
    Ok(spec)
}

// ---------------------------------------------------------------------------
// creation request
// ---------------------------------------------------------------------------

fn validate_bot_token(token: &str) -> Result<(), RPCErrors> {
    let valid = token.split_once(':').is_some_and(|(bot_id, secret)| {
        !bot_id.is_empty()
            && bot_id.bytes().all(|byte| byte.is_ascii_digit())
            && !secret.is_empty()
            && secret
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    });
    if valid {
        Ok(())
    } else {
        Err(reason(
            "invalid_bot_token",
            "a Telegram bot token looks like `123456:ABC-def_123`",
        ))
    }
}

/// Telegram account ids are stored bare; older writers used a `user:` prefix.
pub(crate) fn bare_telegram_account(account_id: &str) -> &str {
    let account_id = account_id.trim();
    account_id
        .strip_prefix("user:")
        .unwrap_or(account_id)
        .trim()
}

/// Online check of a bot token; returns the bot's own Telegram id.
async fn verify_telegram_bot(bot_token: &str) -> Result<String, AgentFailure> {
    let unreachable = |error: reqwest::Error| {
        AgentFailure::new(
            "telegram_unreachable",
            format!("cannot reach Telegram: {}", error.without_url()),
            true,
        )
    };
    let client = reqwest::Client::builder()
        .timeout(TELEGRAM_TIMEOUT)
        .build()
        .map_err(unreachable)?;
    let body: Value = client
        .get(format!("{TELEGRAM_API}/bot{bot_token}/getMe"))
        .send()
        .await
        .map_err(unreachable)?
        .json()
        .await
        .map_err(unreachable)?;
    let bot_id = body
        .get("result")
        .and_then(|result| result.get("id"))
        .and_then(|id| {
            id.as_i64()
                .map(|id| id.to_string())
                .or_else(|| id.as_str().map(str::to_string))
        });
    match (body.get("ok").and_then(Value::as_bool), bot_id) {
        (Some(true), Some(bot_id)) => Ok(bot_id),
        _ => Err(AgentFailure::new(
            "telegram_bot_invalid",
            body.get("description")
                .and_then(Value::as_str)
                .unwrap_or("Telegram rejected the bot token"),
            true,
        )),
    }
}

fn validate_avatar(avatar: &str) -> Result<(), RPCErrors> {
    let invalid =
        |message: &str| RPCErrors::ParseRequestError(format!("invalid avatar: {message}"));
    let (header, data) = avatar
        .strip_prefix("data:image/")
        .and_then(|rest| rest.split_once(";base64,"))
        .ok_or_else(|| invalid("expect a base64 image data URL"))?;
    if header.is_empty() || data.len() > MAX_AVATAR_BYTES * 4 / 3 + 4 {
        return Err(invalid("image is too large"));
    }
    let bytes = STANDARD
        .decode(data)
        .map_err(|_| invalid("bad base64 payload"))?;
    let (width, height) = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid("unknown image format"))?
        .into_dimensions()
        .map_err(|_| invalid("unreadable image"))?;
    if width > MAX_AVATAR_PX || height > MAX_AVATAR_PX {
        return Err(invalid("image must be at most 192px"));
    }
    Ok(())
}

fn optional_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn parse_profile(value: Option<&Value>) -> Result<AgentProfile, RPCErrors> {
    let Some(value) = value.filter(|value| !value.is_null()) else {
        return Ok(AgentProfile::default());
    };
    if !value.is_object() {
        return Err(RPCErrors::ParseRequestError(
            "profile must be an object".to_string(),
        ));
    }
    let profile = AgentProfile {
        display_name: optional_text(value.get("display_name")),
        avatar: optional_text(value.get("avatar")),
        bio: optional_text(value.get("bio")),
    };
    if let Some(avatar) = profile.avatar.as_deref() {
        validate_avatar(avatar)?;
    }
    Ok(profile)
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct AgentCreateRequest {
    pub idempotency_key: String,
    pub name: String,
    pub profile: AgentProfile,
    pub role_supplement: String,
    pub allow_group: bool,
    pub allow_other_users: bool,
    pub template_id: String,
    pub template_auto_update: bool,
    pub desktop_entry: Option<String>,
    pub bot_token: Option<String>,
}

impl AgentCreateRequest {
    pub(crate) fn parse(params: &Value) -> Result<Self, RPCErrors> {
        let required = |key: &str| {
            optional_text(params.get(key))
                .ok_or_else(|| RPCErrors::ParseRequestError(format!("Missing {key}")))
        };
        let flag = |key: &str, default: bool| match params.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err(RPCErrors::ParseRequestError(format!(
                "{key} must be a boolean"
            ))),
        };
        let bot_token = match params.get("msg_tunnel") {
            None | Some(Value::Null) => None,
            Some(tunnel) => {
                if tunnel.get("platform").and_then(Value::as_str) != Some(TELEGRAM) {
                    return Err(RPCErrors::ParseRequestError(
                        "msg_tunnel.platform must be telegram".to_string(),
                    ));
                }
                let bot_token = optional_text(tunnel.get("bot_token")).ok_or_else(|| {
                    RPCErrors::ParseRequestError("Missing msg_tunnel.bot_token".to_string())
                })?;
                validate_bot_token(&bot_token)?;
                Some(bot_token)
            }
        };
        Ok(Self {
            idempotency_key: required("idempotency_key")?,
            name: params
                .get("name")
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or_default()
                .to_string(),
            profile: parse_profile(params.get("profile"))?,
            role_supplement: params
                .get("role_supplement")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_string(),
            allow_group: flag("allow_group", false)?,
            allow_other_users: flag("allow_other_users", false)?,
            template_id: required("template_id")?,
            template_auto_update: flag("template_auto_update", true)?,
            desktop_entry: optional_text(params.get("desktop_entry")),
            bot_token,
        })
    }

    pub(crate) fn fingerprint(&self) -> String {
        let material = json!({
            "name": self.name,
            "profile": self.profile,
            "role_supplement": self.role_supplement,
            "allow_group": self.allow_group,
            "allow_other_users": self.allow_other_users,
            "template_id": self.template_id,
            "template_auto_update": self.template_auto_update,
            "desktop_entry": self.desktop_entry,
            "bot_token": self.bot_token,
        });
        build_named_object_by_json("agentreq", &material)
            .0
            .to_string()
    }

    fn settings(&self) -> AgentSettings {
        AgentSettings {
            allow_group: self.allow_group,
            role_supplement: self.role_supplement.clone(),
            template_auto_update: self.template_auto_update,
            desktop_entry: self.desktop_entry.clone(),
            msg_tunnels: self
                .bot_token
                .iter()
                .map(|bot_token| AgentMsgTunnelSettings {
                    platform: TELEGRAM.to_string(),
                    bot_token: bot_token.clone(),
                    bot_account_id: None,
                })
                .collect(),
            ..AgentSettings::default()
        }
    }
}

pub(crate) fn initial_record(
    spec: AgentSpec,
    request: &AgentCreateRequest,
    fingerprint: &str,
    template_id: String,
    template_source: AgentTemplateSource,
    template: BuiltFromTemplate,
    now: u64,
) -> AgentInstallRecord {
    AgentInstallRecord {
        schema_version: AGENT_SPEC_SCHEMA_VERSION,
        agent_id: spec.agent_id.clone(),
        agent_doc_object_id: spec.agent_doc_object_id.clone(),
        target_app_instance_id: spec.binding.target_app_instance_id.clone(),
        service_name: spec.binding.service_name.clone(),
        generation: spec.generation,
        state: AgentInstallState::Provisioning,
        step: AgentCreateStep::Runtime,
        last_error: None,
        idempotency_key: request.idempotency_key.clone(),
        request_fingerprint: fingerprint.to_string(),
        pending_spec: Some(spec),
        runtime_task_id: None,
        tunnel_state: if request.bot_token.is_some() {
            AgentTunnelState::Pending
        } else {
            AgentTunnelState::None
        },
        template_id,
        template_source,
        template_app_did: template.app_did,
        template_app_doc_object_id: template.app_doc_object_id,
        template_version: template.version,
        created_at: now,
        updated_at: now,
    }
}

/// Step 1: the identity reservation. Everything is created in one
/// transaction; no `spec` exists yet, so nothing loads or routes the Agent.
pub(crate) fn reservation_actions(
    owner: &str,
    record: &AgentInstallRecord,
    private_key_pem: &str,
    settings: &AgentSettings,
    profile: &AgentProfile,
) -> Result<HashMap<String, KVAction>, RPCErrors> {
    let agent_id = &record.agent_id;
    Ok(HashMap::from([
        (
            agent_install_record_key(owner, agent_id),
            KVAction::Create(to_json(record)?),
        ),
        (
            agent_key_path(owner, agent_id),
            KVAction::Create(private_key_pem.to_string()),
        ),
        (
            agent_settings_key(owner, agent_id),
            KVAction::Create(to_json(settings)?),
        ),
        (
            agent_profile_key(owner, agent_id),
            KVAction::Create(to_json(profile)?),
        ),
        (
            reservation_key(agent_id),
            KVAction::Create(json!({ "owner_user_id": owner }).to_string()),
        ),
    ]))
}

/// Applies `agent.create.retry` to a failed record. Returns the settings to
/// write back when the tunnel is skipped.
pub(crate) fn prepare_retry(
    record: &mut AgentInstallRecord,
    settings: Option<AgentSettings>,
    skip_tunnel: bool,
    now: u64,
) -> Result<Option<AgentSettings>, RPCErrors> {
    if record.state != AgentInstallState::Failed {
        return Err(reason(
            "not_failed",
            "only a failed creation can be retried",
        ));
    }
    let step = record
        .last_error
        .take()
        .map(|error| error.step)
        .unwrap_or(record.step);
    let mut settings_change = None;
    if step == AgentCreateStep::Tunnel && skip_tunnel {
        record.tunnel_state = AgentTunnelState::Skipped;
        record.step = AgentCreateStep::Start;
        let mut settings = settings.unwrap_or_default();
        settings.msg_tunnels.clear();
        settings_change = Some(settings);
    } else {
        record.step = step;
        if step == AgentCreateStep::Tunnel {
            record.tunnel_state = AgentTunnelState::Pending;
        }
    }
    record.state = match record.step {
        AgentCreateStep::Runtime | AgentCreateStep::Bind => AgentInstallState::Provisioning,
        _ => AgentInstallState::Bound,
    };
    record.updated_at = now;
    Ok(settings_change)
}

fn cancel_allowed(record: &AgentInstallRecord) -> bool {
    match record.state {
        AgentInstallState::Provisioning => true,
        AgentInstallState::Failed => matches!(
            record.last_error.as_ref().map(|error| error.step),
            Some(AgentCreateStep::Runtime | AgentCreateStep::Bind)
        ),
        _ => false,
    }
}

/// Deletion step 1: the record becomes `removed` and every identity key the
/// runtime could load goes away in the same transaction.
pub(crate) async fn begin_agent_removal(
    kv: &dyn AgentKv,
    owner: &str,
    agent_id: &AgentId,
    now: u64,
) -> Result<(), RPCErrors> {
    for _ in 0..3 {
        let (mut record, version) = load_record(kv, owner, agent_id)
            .await?
            .ok_or_else(|| reason("agent_not_found", agent_id))?;
        if record.state == AgentInstallState::Removed {
            return Ok(());
        }
        record.state = AgentInstallState::Removed;
        record.last_error = None;
        record.pending_spec = None;
        record.updated_at = now;
        let actions = [
            agent_spec_key(owner, agent_id),
            agent_key_path(owner, agent_id),
            agent_settings_key(owner, agent_id),
            agent_profile_key(owner, agent_id),
            agent_info_key(owner, agent_id),
        ]
        .into_iter()
        .map(|key| (key, KVAction::Remove))
        .collect();
        if commit_record(kv, owner, &record, version, actions)
            .await?
            .is_some()
        {
            return Ok(());
        }
    }
    Err(reason(
        "agent_busy",
        "the Agent changed concurrently; try again",
    ))
}

// ---------------------------------------------------------------------------
// creation driver
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AgentFailure {
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

impl AgentFailure {
    pub(crate) fn new(code: &str, message: impl Display, retryable: bool) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            retryable,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct BuiltFromTemplate {
    pub app_did: DID,
    pub app_doc_object_id: ObjId,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RuntimeSubmit {
    Satisfied(BuiltFromTemplate),
    Task(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskKind {
    Install,
    Uninstall,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TaskState {
    Running,
    Succeeded,
    Failed(AgentFailure),
    /// Paused, or waiting on an error it can be resumed from.
    Stalled(AgentFailure),
}

/// Side effects of the creation/removal state machine.
#[async_trait]
pub(crate) trait AgentDriverOps: Send + Sync {
    fn kv(&self) -> &dyn AgentKv;
    async fn submit_runtime(
        &self,
        owner: &str,
        record: &AgentInstallRecord,
    ) -> Result<RuntimeSubmit, AgentFailure>;
    async fn task_state(
        &self,
        owner: &str,
        task_id: &str,
    ) -> Result<(TaskKind, TaskState), RPCErrors>;
    async fn resume_runtime(&self, owner: &str, task_id: &str) -> Result<String, AgentFailure>;
    async fn refresh_rbac(&self) -> Result<(), RPCErrors>;
    async fn verify_telegram_bot(&self, bot_token: &str) -> Result<String, AgentFailure>;
    async fn reload_msg_center(&self) -> Result<(), RPCErrors>;
    async fn start_uninstall(
        &self,
        owner: &str,
        record: &AgentInstallRecord,
        after_failed_task: Option<&str>,
    ) -> Result<Option<String>, RPCErrors>;
    fn now(&self) -> u64;
    fn poll_interval(&self) -> Duration;
    fn start_timeout_secs(&self) -> u64;
}

async fn fail_step(
    ops: &dyn AgentDriverOps,
    owner: &str,
    mut record: AgentInstallRecord,
    version: u64,
    step: AgentCreateStep,
    failure: AgentFailure,
) -> Result<bool, RPCErrors> {
    warn!(
        "Agent {} creation failed at {:?}: {} {}",
        record.agent_id, step, failure.code, failure.message
    );
    record.state = AgentInstallState::Failed;
    record.last_error = Some(AgentCreateError {
        step,
        code: failure.code,
        message: failure.message,
        retryable: failure.retryable,
    });
    if step == AgentCreateStep::Tunnel {
        record.tunnel_state = AgentTunnelState::Failed;
    }
    record.updated_at = ops.now();
    Ok(
        commit_record(ops.kv(), owner, &record, version, HashMap::new())
            .await?
            .is_none(),
    )
}

/// Sleeps one poll interval and reports whether the record is unchanged.
async fn still_current(
    ops: &dyn AgentDriverOps,
    owner: &str,
    agent_id: &AgentId,
    version: u64,
) -> Result<bool, RPCErrors> {
    tokio::time::sleep(ops.poll_interval()).await;
    Ok(load_record(ops.kv(), owner, agent_id)
        .await?
        .is_some_and(|(_, current)| current == version))
}

/// Runs the step the record currently points at. Returns true when the record
/// moved and the caller should evaluate it again.
pub(crate) async fn drive_agent_step(
    ops: &dyn AgentDriverOps,
    owner: &str,
    agent_id: &AgentId,
) -> Result<bool, RPCErrors> {
    let Some((record, version)) = load_record(ops.kv(), owner, agent_id).await? else {
        return Ok(false);
    };
    match (record.state, record.step) {
        (AgentInstallState::Provisioning, AgentCreateStep::Runtime) => {
            step_runtime(ops, owner, record, version).await
        }
        (AgentInstallState::Provisioning, AgentCreateStep::Bind) => {
            step_bind(ops, owner, record, version).await
        }
        (AgentInstallState::Bound, AgentCreateStep::Tunnel) => {
            step_tunnel(ops, owner, record, version).await
        }
        (AgentInstallState::Bound, AgentCreateStep::Start) => {
            step_start(ops, owner, record, version).await
        }
        (AgentInstallState::Ready, _) if record.runtime_task_id.is_some() => {
            step_runtime_update(ops, owner, record, version).await
        }
        (AgentInstallState::Removed, _) => step_remove(ops, owner, record, version).await,
        (AgentInstallState::Provisioning | AgentInstallState::Bound, step) => {
            fail_step(
                ops,
                owner,
                record,
                version,
                step,
                AgentFailure::new(
                    "invalid_step",
                    "record step does not match its state",
                    false,
                ),
            )
            .await
        }
        _ => Ok(false),
    }
}

fn apply_template(record: &mut AgentInstallRecord, template: BuiltFromTemplate) {
    record.template_app_did = template.app_did;
    record.template_app_doc_object_id = template.app_doc_object_id;
    record.template_version = template.version;
}

/// Step 2. A runtime install task that is still running is only awaited, so
/// a restarted control_panel never resubmits over its own in-flight task.
async fn step_runtime(
    ops: &dyn AgentDriverOps,
    owner: &str,
    mut record: AgentInstallRecord,
    mut version: u64,
) -> Result<bool, RPCErrors> {
    let running = match record.runtime_task_id.clone() {
        Some(task_id) => match ops.task_state(owner, &task_id).await?.1 {
            TaskState::Running => Some(task_id),
            TaskState::Stalled(_) => match ops.resume_runtime(owner, &task_id).await {
                Ok(task_id) => Some(task_id),
                Err(failure) => {
                    return fail_step(
                        ops,
                        owner,
                        record,
                        version,
                        AgentCreateStep::Runtime,
                        failure,
                    )
                    .await
                }
            },
            _ => None,
        },
        None => None,
    };
    let task_id = match running {
        Some(task_id) => Some(task_id),
        None => match ops.submit_runtime(owner, &record).await {
            Err(failure) => {
                return fail_step(
                    ops,
                    owner,
                    record,
                    version,
                    AgentCreateStep::Runtime,
                    failure,
                )
                .await
            }
            Ok(RuntimeSubmit::Satisfied(_)) => None,
            Ok(RuntimeSubmit::Task(task_id)) => Some(task_id),
        },
    };
    if let Some(task_id) = task_id {
        if record.runtime_task_id.as_deref() != Some(task_id.as_str()) {
            record.runtime_task_id = Some(task_id.clone());
            record.updated_at = ops.now();
            match commit_record(ops.kv(), owner, &record, version, HashMap::new()).await? {
                Some(next) => version = next,
                None => return Ok(true),
            }
        }
        loop {
            match ops.task_state(owner, &task_id).await?.1 {
                TaskState::Running => {
                    if !still_current(ops, owner, &record.agent_id, version).await? {
                        return Ok(true);
                    }
                }
                TaskState::Succeeded => break,
                TaskState::Failed(failure) | TaskState::Stalled(failure) => {
                    return fail_step(
                        ops,
                        owner,
                        record,
                        version,
                        AgentCreateStep::Runtime,
                        failure,
                    )
                    .await
                }
            }
        }
    }
    record.step = AgentCreateStep::Bind;
    record.runtime_task_id = None;
    apply_installed_template(ops, owner, &mut record).await?;
    record.updated_at = ops.now();
    commit_record(ops.kv(), owner, &record, version, HashMap::new()).await?;
    Ok(true)
}

/// The installed runtime AppDoc names the template it was built from.
async fn apply_installed_template(
    ops: &dyn AgentDriverOps,
    owner: &str,
    record: &mut AgentInstallRecord,
) -> Result<(), RPCErrors> {
    let app_id = AppId::parse(record.agent_id.as_str()).map_err(RPCErrors::ReasonError)?;
    if let Some(spec) =
        get_value::<AppServiceSpec>(ops.kv(), &user_app_spec_key(owner, &app_id)).await
    {
        if let Some(base_on) = spec.app_doc.base_on.clone() {
            record.template_app_doc_object_id = base_on;
            record.template_version = spec.app_doc.version.clone();
        }
    }
    Ok(())
}

async fn step_bind(
    ops: &dyn AgentDriverOps,
    owner: &str,
    record: AgentInstallRecord,
    version: u64,
) -> Result<bool, RPCErrors> {
    let app_id = AppId::parse(record.agent_id.as_str()).map_err(RPCErrors::ReasonError)?;
    let runtime_ready = get_value::<AppServiceSpec>(ops.kv(), &user_app_spec_key(owner, &app_id))
        .await
        .is_some_and(|spec| {
            spec.is_installed()
                && spec.app_instance_id == record.target_app_instance_id
                && spec
                    .spec_config
                    .service_config
                    .contains_key(&record.service_name)
        });
    if !runtime_ready {
        let failure = AgentFailure::new(
            "runtime_missing",
            format!(
                "runtime App {} with service `{}` is not installed",
                record.target_app_instance_id, record.service_name
            ),
            true,
        );
        return fail_step(ops, owner, record, version, AgentCreateStep::Bind, failure).await;
    }
    let Some(spec) = record.pending_spec.clone() else {
        let failure = AgentFailure::new("pending_spec_missing", "no AgentSpec to bind", false);
        return fail_step(ops, owner, record, version, AgentCreateStep::Bind, failure).await;
    };
    let mut bound = record.clone();
    bound.state = AgentInstallState::Bound;
    bound.step = if bound.tunnel_state == AgentTunnelState::Pending {
        AgentCreateStep::Tunnel
    } else {
        AgentCreateStep::Start
    };
    bound.pending_spec = None;
    bound.updated_at = ops.now();
    let actions = HashMap::from([(
        agent_spec_key(owner, &record.agent_id),
        KVAction::Create(to_json(&spec)?),
    )]);
    match commit_record(ops.kv(), owner, &bound, version, actions).await {
        Ok(Some(_)) => {
            if let Err(error) = ops.refresh_rbac().await {
                warn!(
                    "RBAC refresh after binding Agent {} failed: {error}",
                    record.agent_id
                );
            }
            Ok(true)
        }
        Ok(None) => Ok(true),
        Err(error) => {
            let failure = AgentFailure::new("bind_failed", error, true);
            fail_step(ops, owner, record, version, AgentCreateStep::Bind, failure).await
        }
    }
}

/// Step 4, after the spec exists: verify the bot token online, pin the bot's
/// Telegram id (msg-center derives session ids from it, so it never changes
/// once written) and let msg-center pick the binding up.
async fn step_tunnel(
    ops: &dyn AgentDriverOps,
    owner: &str,
    mut record: AgentInstallRecord,
    version: u64,
) -> Result<bool, RPCErrors> {
    let settings_key = agent_settings_key(owner, &record.agent_id);
    let mut settings = get_value::<AgentSettings>(ops.kv(), &settings_key)
        .await
        .unwrap_or_default();
    record.tunnel_state = AgentTunnelState::Skipped;
    if let Some(tunnel) = settings
        .msg_tunnels
        .iter_mut()
        .find(|tunnel| tunnel.platform == TELEGRAM)
    {
        if tunnel.bot_account_id.is_none() {
            match ops.verify_telegram_bot(&tunnel.bot_token).await {
                Ok(bot_id) => tunnel.bot_account_id = Some(bot_id),
                Err(failure) => {
                    return fail_step(
                        ops,
                        owner,
                        record,
                        version,
                        AgentCreateStep::Tunnel,
                        failure,
                    )
                    .await
                }
            }
            if let Err(error) = ops
                .kv()
                .exec_tx(
                    HashMap::from([(settings_key, KVAction::Update(to_json(&settings)?))]),
                    None,
                )
                .await
            {
                let failure = AgentFailure::new("tunnel_save_failed", error, true);
                return fail_step(
                    ops,
                    owner,
                    record,
                    version,
                    AgentCreateStep::Tunnel,
                    failure,
                )
                .await;
            }
        }
        if let Err(error) = ops.reload_msg_center().await {
            let failure = AgentFailure::new("tunnel_reload_failed", error, true);
            return fail_step(
                ops,
                owner,
                record,
                version,
                AgentCreateStep::Tunnel,
                failure,
            )
            .await;
        }
        record.tunnel_state = AgentTunnelState::Bound;
    }
    record.step = AgentCreateStep::Start;
    record.updated_at = ops.now();
    commit_record(ops.kv(), owner, &record, version, HashMap::new()).await?;
    Ok(true)
}

async fn step_start(
    ops: &dyn AgentDriverOps,
    owner: &str,
    mut record: AgentInstallRecord,
    version: u64,
) -> Result<bool, RPCErrors> {
    let Some(spec) =
        get_value::<AgentSpec>(ops.kv(), &agent_spec_key(owner, &record.agent_id)).await
    else {
        let failure = AgentFailure::new("spec_missing", "AgentSpec is missing", false);
        return fail_step(ops, owner, record, version, AgentCreateStep::Start, failure).await;
    };
    let info_key = agent_info_key(owner, &record.agent_id);
    loop {
        if get_value::<AgentRuntimeInfo>(ops.kv(), &info_key)
            .await
            .is_some_and(|info| {
                info.agent_doc_object_id == spec.agent_doc_object_id
                    && info.generation == spec.generation
            })
        {
            record.state = AgentInstallState::Ready;
            record.step = AgentCreateStep::Done;
            record.updated_at = ops.now();
            commit_record(ops.kv(), owner, &record, version, HashMap::new()).await?;
            return Ok(true);
        }
        if ops.now() >= record.updated_at.saturating_add(ops.start_timeout_secs()) {
            let failure = AgentFailure::new(
                "start_timeout",
                format!(
                    "the Agent runtime did not report the Agent loaded within {} seconds",
                    ops.start_timeout_secs()
                ),
                true,
            );
            return fail_step(ops, owner, record, version, AgentCreateStep::Start, failure).await;
        }
        if !still_current(ops, owner, &record.agent_id, version).await? {
            return Ok(true);
        }
    }
}

async fn step_runtime_update(
    ops: &dyn AgentDriverOps,
    owner: &str,
    mut record: AgentInstallRecord,
    version: u64,
) -> Result<bool, RPCErrors> {
    let task_id = record.runtime_task_id.clone().unwrap_or_default();
    loop {
        match ops.task_state(owner, &task_id).await?.1 {
            TaskState::Running => {
                if !still_current(ops, owner, &record.agent_id, version).await? {
                    return Ok(true);
                }
            }
            TaskState::Succeeded => {
                apply_installed_template(ops, owner, &mut record).await?;
                break;
            }
            TaskState::Failed(failure) | TaskState::Stalled(failure) => {
                warn!(
                    "template update of Agent {} failed: {} {}",
                    record.agent_id, failure.code, failure.message
                );
                break;
            }
        }
    }
    record.runtime_task_id = None;
    record.updated_at = ops.now();
    commit_record(ops.kv(), owner, &record, version, HashMap::new()).await?;
    Ok(true)
}

/// Deletion steps 2-5: refresh RBAC, reload msg-center, uninstall the runtime
/// App (its AgentSpec is already gone), then drop the record and name.
async fn step_remove(
    ops: &dyn AgentDriverOps,
    owner: &str,
    mut record: AgentInstallRecord,
    mut version: u64,
) -> Result<bool, RPCErrors> {
    if let Err(error) = ops.refresh_rbac().await {
        warn!(
            "RBAC refresh while removing Agent {} failed: {error}",
            record.agent_id
        );
    }
    if let Err(error) = ops.reload_msg_center().await {
        warn!(
            "msg-center reload while removing Agent {} failed: {error}",
            record.agent_id
        );
    }
    loop {
        let mut failed_uninstall = None;
        if let Some(task_id) = record.runtime_task_id.clone() {
            match ops.task_state(owner, &task_id).await? {
                (_, TaskState::Running) => {
                    if !still_current(ops, owner, &record.agent_id, version).await? {
                        return Ok(true);
                    }
                    continue;
                }
                (TaskKind::Uninstall, TaskState::Succeeded) => {
                    return finish_removal(ops, owner, &record, version).await;
                }
                (TaskKind::Uninstall, TaskState::Failed(failure) | TaskState::Stalled(failure)) => {
                    warn!(
                        "uninstalling runtime of Agent {} failed: {}",
                        record.agent_id, failure.message
                    );
                    failed_uninstall = Some(task_id);
                }
                (TaskKind::Install, _) => {}
            }
        }
        match ops
            .start_uninstall(owner, &record, failed_uninstall.as_deref())
            .await?
        {
            None => return finish_removal(ops, owner, &record, version).await,
            Some(task_id) => {
                record.runtime_task_id = Some(task_id);
                record.updated_at = ops.now();
                match commit_record(ops.kv(), owner, &record, version, HashMap::new()).await? {
                    Some(next) if failed_uninstall.is_none() => version = next,
                    _ => return Ok(false),
                }
            }
        }
    }
}

async fn finish_removal(
    ops: &dyn AgentDriverOps,
    owner: &str,
    record: &AgentInstallRecord,
    version: u64,
) -> Result<bool, RPCErrors> {
    let key = agent_install_record_key(owner, &record.agent_id);
    let actions = HashMap::from([
        (key.clone(), KVAction::Remove),
        (reservation_key(&record.agent_id), KVAction::Remove),
    ]);
    if let Err(error) = ops
        .kv()
        .exec_tx(actions, Some((key.clone(), version)))
        .await
    {
        return match ops.kv().get(&key).await? {
            Some((_, current)) if current == version => Err(error),
            _ => Ok(true),
        };
    }
    info!("Agent {} removed", record.agent_id);
    Ok(false)
}

fn needs_drive(record: &AgentInstallRecord) -> bool {
    match record.state {
        AgentInstallState::Provisioning | AgentInstallState::Bound | AgentInstallState::Removed => {
            true
        }
        AgentInstallState::Ready => record.runtime_task_id.is_some(),
        AgentInstallState::Failed => false,
    }
}

fn install_failure(error: RPCErrors) -> AgentFailure {
    if let RPCErrors::ReasonError(raw) = &error {
        if let Ok(install) = serde_json::from_str::<InstallError>(raw) {
            return install_error_failure(&install);
        }
    }
    AgentFailure::new("runtime_install_failed", error, true)
}

fn install_error_failure(error: &InstallError) -> AgentFailure {
    let code = serde_json::to_value(error.code)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    AgentFailure::new(
        "runtime_install_failed",
        format!("{code}: {}", error.message),
        error.retryable,
    )
}

pub(crate) async fn reload_msg_center() -> Result<(), RPCErrors> {
    get_buckyos_api_runtime()?
        .get_zone_service_krpc_client(MSG_CENTER_SERVICE_NAME)
        .await?
        .call("reload_settings", json!({}))
        .await
        .map(|_| ())
}

fn internal_principal(owner: &str) -> RpcAuthPrincipal {
    RpcAuthPrincipal {
        username: owner.to_string(),
        owner_user_id: owner.to_string(),
        authenticated_app_id: INTERNAL_APP_ID.to_string(),
        user_type: UserType::Admin,
        owner_did: String::new(),
        is_user_session: false,
        is_control_panel_session: true,
    }
}

struct ZoneAgentOps {
    server: ControlPanelServer,
    kv: ZoneKv,
}

#[async_trait]
impl AgentDriverOps for ZoneAgentOps {
    fn kv(&self) -> &dyn AgentKv {
        &self.kv
    }

    async fn submit_runtime(
        &self,
        owner: &str,
        record: &AgentInstallRecord,
    ) -> Result<RuntimeSubmit, AgentFailure> {
        self.server
            .submit_agent_runtime(&self.kv, owner, record)
            .await
    }

    async fn task_state(
        &self,
        owner: &str,
        task_id: &str,
    ) -> Result<(TaskKind, TaskState), RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let task = runtime
            .get_task_mgr_client()
            .await?
            .get_task(task_id)
            .await?;
        if task.schema_id == APP_UNINSTALL_TASK_SCHEMA_ID {
            let state = match (task.phase, task.outcome) {
                (TaskPhase::Terminal, Some(TaskOutcome::Succeeded)) => TaskState::Succeeded,
                (TaskPhase::Terminal, _) => TaskState::Failed(AgentFailure::new(
                    "runtime_uninstall_failed",
                    task.error
                        .map(|error| error.message)
                        .or(task.message)
                        .unwrap_or_else(|| "uninstall did not succeed".to_string()),
                    true,
                )),
                _ => TaskState::Running,
            };
            return Ok((TaskKind::Uninstall, state));
        }
        let status = self
            .server
            .install_engine
            .status(task_id, owner, true)
            .await
            .map_err(ControlPanelServer::install_error_to_rpc)?;
        let failure = || {
            status
                .error
                .as_ref()
                .map(install_error_failure)
                .unwrap_or_else(|| {
                    AgentFailure::new(
                        "runtime_install_failed",
                        format!("install task {task_id} did not succeed"),
                        true,
                    )
                })
        };
        let state = match (status.task_phase, status.task_outcome) {
            (TaskPhase::Terminal, Some(TaskOutcome::Succeeded)) => TaskState::Succeeded,
            (TaskPhase::Terminal, _) => TaskState::Failed(failure()),
            (TaskPhase::Paused, _) => TaskState::Stalled(failure()),
            (TaskPhase::Waiting, _) if status.error.is_some() => TaskState::Stalled(failure()),
            _ => TaskState::Running,
        };
        Ok((TaskKind::Install, state))
    }

    async fn resume_runtime(&self, owner: &str, task_id: &str) -> Result<String, AgentFailure> {
        let resumed = self
            .server
            .install_engine
            .retry(
                task_id,
                owner,
                true,
                buckyos_api::CONTROL_PANEL_SERVICE_NAME,
                &format!("agent-runtime-resume:{task_id}"),
            )
            .await
            .map_err(|error| install_error_failure(&error))?;
        self.server.install_runner.spawn_run(resumed.clone());
        Ok(resumed)
    }

    async fn refresh_rbac(&self) -> Result<(), RPCErrors> {
        refresh_rbac_by_scheduler("agent").await
    }

    async fn verify_telegram_bot(&self, bot_token: &str) -> Result<String, AgentFailure> {
        verify_telegram_bot(bot_token).await
    }

    async fn reload_msg_center(&self) -> Result<(), RPCErrors> {
        reload_msg_center().await
    }

    async fn start_uninstall(
        &self,
        owner: &str,
        record: &AgentInstallRecord,
        after_failed_task: Option<&str>,
    ) -> Result<Option<String>, RPCErrors> {
        self.server
            .start_agent_runtime_uninstall(&self.kv, owner, record, after_failed_task)
            .await
    }

    fn now(&self) -> u64 {
        buckyos_get_unix_timestamp()
    }

    fn poll_interval(&self) -> Duration {
        DRIVE_POLL
    }

    fn start_timeout_secs(&self) -> u64 {
        START_TIMEOUT_SECS
    }
}

/// Re-registers the local AppDoc authority of every Agent runtime App. The
/// override only lives in memory, so it must exist again before any resumed
/// install task resolves the AgentDID.
pub(crate) async fn register_agent_runtime_authorities() {
    let result = async {
        let kv = ZoneKv::connect().await?;
        let tasks = get_buckyos_api_runtime()?.get_task_mgr_client().await?;
        for (owner, record) in list_records(&kv, None).await? {
            let Ok(app_id) = AppId::parse(record.agent_id.as_str()) else {
                continue;
            };
            let mut app_doc = get_value::<AppServiceSpec>(&kv, &user_app_spec_key(&owner, &app_id))
                .await
                .filter(AppServiceSpec::is_installed)
                .map(|spec| spec.app_doc);
            if let Some(task_id) = record.runtime_task_id.as_deref() {
                if let Ok(task) = tasks.get_task(task_id).await {
                    let plan = match task.schema_id.as_str() {
                        APP_INSTALL_TASK_SCHEMA_ID => {
                            serde_json::from_value::<AppInstallTaskData>(task.input)
                                .ok()
                                .and_then(|data| data.request.submitted_plan)
                        }
                        APP_UPDATE_TASK_SCHEMA_ID => {
                            serde_json::from_value::<AppUpdateTaskData>(task.input)
                                .ok()
                                .and_then(|data| data.request.submitted_plan)
                        }
                        _ => None,
                    };
                    if let Some(plan) = plan {
                        app_doc = Some(plan.app_doc);
                    }
                }
            }
            let Some(app_doc) = app_doc else {
                continue;
            };
            let value = serde_json::to_value(&app_doc)
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
            register_app_doc_authority(app_doc.app_did(), value, "agent-runtime")?;
        }
        Ok::<(), RPCErrors>(())
    }
    .await;
    if let Err(error) = result {
        warn!("register Agent runtime authorities failed: {error}");
    }
}

// ---------------------------------------------------------------------------
// views
// ---------------------------------------------------------------------------

fn settings_view(settings: &AgentSettings) -> Value {
    json!({
        "enabled": settings.enabled,
        "auto_start": settings.auto_start,
        "allow_other_users": settings.allow_other_users,
        "allow_group": settings.allow_group,
        "role_supplement": settings.role_supplement,
        "template_auto_update": settings.template_auto_update,
        "desktop_entry": settings.desktop_entry,
        "msg_tunnels": settings.msg_tunnels.iter().map(|tunnel| json!({
            "platform": tunnel.platform,
            "bot_account_id": tunnel.bot_account_id,
        })).collect::<Vec<_>>(),
    })
}

fn runtime_view(spec: &AppServiceSpec) -> Value {
    json!({
        "app_instance_id": spec.app_instance_id,
        "app_host_name": spec.app_host_name,
        "has_web": spec.spec_config.expose_config.values().any(|expose| {
            expose.sub_hostname().iter().any(|host| !host.trim().is_empty())
        }),
        "state": spec.state,
    })
}

fn status_view(owner: &str, record: &AgentInstallRecord) -> Value {
    let mut status = json!({
        "agent_id": record.agent_id,
        "agent_did": record.agent_id.agent_did().to_string(),
        "owner_user_id": owner,
        "state": record.state,
        "step": record.step,
        "tunnel_state": record.tunnel_state,
        "created_at": record.created_at,
        "updated_at": record.updated_at,
    });
    if let Some(error) = record.last_error.as_ref() {
        status["last_error"] = json!(error);
    }
    if let Some(task_id) = record.runtime_task_id.as_ref() {
        status["runtime_task_id"] = json!(task_id);
    }
    status
}

// ---------------------------------------------------------------------------
// control_panel service
// ---------------------------------------------------------------------------

fn agent_key(owner: &str, agent_id: &AgentId) -> String {
    format!("{owner}/{agent_id}")
}

impl ControlPanelServer {
    fn require_agent_creator(
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<&RpcAuthPrincipal, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        match principal.user_type {
            UserType::Root | UserType::Admin | UserType::User => Ok(principal),
            UserType::Limited | UserType::Guest => {
                Err(reason("limited_user", "limited users cannot create Agents"))
            }
        }
    }

    fn principal_admin(principal: &RpcAuthPrincipal) -> bool {
        matches!(principal.user_type, UserType::Admin | UserType::Root)
    }

    fn param_agent_id(req: &RPCRequest) -> Result<AgentId, RPCErrors> {
        let raw = Self::require_param_str(req, "agent_id")?;
        AgentId::parse(raw.trim()).map_err(|_| reason("agent_not_found", raw))
    }

    /// Finds an Agent the caller may manage: its owner or an admin.
    async fn locate_agent(
        kv: &dyn AgentKv,
        principal: &RpcAuthPrincipal,
        agent_id: &AgentId,
        include_removed: bool,
    ) -> Result<(String, AgentInstallRecord, u64), RPCErrors> {
        let not_found = || reason("agent_not_found", agent_id);
        let owner = get_value::<Value>(kv, &reservation_key(agent_id))
            .await
            .and_then(|value| {
                value
                    .get("owner_user_id")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .ok_or_else(not_found)?;
        if owner != principal.owner_user_id && !Self::principal_admin(principal) {
            return Err(not_found());
        }
        let (record, version) = load_record(kv, &owner, agent_id)
            .await?
            .ok_or_else(not_found)?;
        if record.state == AgentInstallState::Removed && !include_removed {
            return Err(not_found());
        }
        Ok((owner, record, version))
    }

    fn claim_agent(&self, key: &str) -> bool {
        let mut drives = self.agent_drives.lock().unwrap();
        if let Some(dirty) = drives.get_mut(key) {
            *dirty = true;
            return false;
        }
        drives.insert(key.to_string(), false);
        true
    }

    /// Releases a claim; true when someone asked for another pass meanwhile.
    fn release_agent(&self, key: &str) -> bool {
        let mut drives = self.agent_drives.lock().unwrap();
        match drives.get_mut(key) {
            Some(dirty) if *dirty => {
                *dirty = false;
                true
            }
            _ => {
                drives.remove(key);
                false
            }
        }
    }

    pub(crate) fn schedule_agent_drive(&self, owner: &str, agent_id: &AgentId) {
        let key = agent_key(owner, agent_id);
        if !self.claim_agent(&key) {
            return;
        }
        let server = self.clone();
        let owner = owner.to_string();
        let agent_id = agent_id.clone();
        tokio::spawn(async move {
            loop {
                match ZoneKv::connect().await {
                    Ok(kv) => {
                        let ops = ZoneAgentOps {
                            server: server.clone(),
                            kv,
                        };
                        for _ in 0..MAX_DRIVE_STEPS {
                            match drive_agent_step(&ops, &owner, &agent_id).await {
                                Ok(true) => continue,
                                Ok(false) => break,
                                Err(error) => {
                                    warn!("Agent {key} drive deferred: {error}");
                                    break;
                                }
                            }
                        }
                    }
                    Err(error) => warn!("Agent {key} drive deferred: {error}"),
                }
                if !server.release_agent(&key) {
                    break;
                }
            }
        });
    }

    async fn sweep_agents(&self) -> Result<(), RPCErrors> {
        let kv = ZoneKv::connect().await?;
        for (owner, record) in list_records(&kv, None).await? {
            if needs_drive(&record) {
                self.schedule_agent_drive(&owner, &record.agent_id);
            }
        }
        Ok(())
    }

    /// Startup scan and low-frequency sweep that resume every unfinished
    /// creation, removal or runtime update from its persisted record.
    pub(crate) fn start_agent_driver(&self) {
        let server = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) = server.sweep_agents().await {
                    warn!("Agent sweep failed: {error}");
                }
                tokio::time::sleep(DRIVE_SWEEP).await;
            }
        });
    }

    async fn submit_agent_runtime(
        &self,
        kv: &dyn AgentKv,
        owner: &str,
        record: &AgentInstallRecord,
    ) -> Result<RuntimeSubmit, AgentFailure> {
        let template = load_agent_template(kv, &get_buckyos_root_dir(), owner, &record.template_id)
            .await
            .map_err(|error| AgentFailure::new("template_unavailable", error, true))?;
        let profile = get_value::<AgentProfile>(kv, &agent_profile_key(owner, &record.agent_id))
            .await
            .unwrap_or_default();
        let show_name = profile.resolved_display_name(&record.agent_id);
        let work_dir = crate::app_install_driver::pikg_staging_root()
            .join(format!(".agent-runtime-{}", uuid::Uuid::new_v4().simple()));
        let prepared = async {
            let built =
                build_agent_runtime_pikg(&template.reader, &record.agent_id, &show_name, &work_dir)
                    .await
                    .map_err(|error| AgentFailure::new("runtime_build_failed", error, true))?;
            let runtime = get_buckyos_api_runtime()
                .map_err(|error| AgentFailure::new("runtime_stage_failed", error, true))?;
            let staged = self
                .staging_store
                .stage_preinstall_file(&built.path, owner, INTERNAL_APP_ID, &runtime.zone_id)
                .await
                .map_err(|error| AgentFailure::new("runtime_stage_failed", error, true))?;
            Ok::<_, AgentFailure>((built, staged))
        }
        .await;
        let _ = tokio::fs::remove_dir_all(&work_dir).await;
        let (built, staged) = prepared?;
        let app_id = AppId::parse(record.agent_id.as_str())
            .map_err(|error| AgentFailure::new("runtime_build_failed", error, false))?;
        let outcome = self
            .submit_internal_install(
                InternalInstall::AgentRuntime {
                    agent_doc_object_id: &record.agent_doc_object_id,
                    supersedes: record.runtime_task_id.as_deref(),
                },
                owner,
                &app_id,
                staged.pikg_digest.as_str(),
                &built.app_doc_object_id,
                &built.app_doc,
                staged.handle.as_str(),
                &PreInstallPlanSeed::default(),
            )
            .await
            .map_err(install_failure)?;
        Ok(match outcome.task_id {
            None => RuntimeSubmit::Satisfied(template.built_from()),
            Some(task_id) => RuntimeSubmit::Task(task_id),
        })
    }

    async fn start_agent_runtime_uninstall(
        &self,
        kv: &dyn AgentKv,
        owner: &str,
        record: &AgentInstallRecord,
        after_failed_task: Option<&str>,
    ) -> Result<Option<String>, RPCErrors> {
        let app_id = AppId::parse(record.agent_id.as_str()).map_err(RPCErrors::ReasonError)?;
        let Some(spec) = get_value::<AppServiceSpec>(kv, &user_app_spec_key(owner, &app_id))
            .await
            .filter(AppServiceSpec::is_installed)
        else {
            return Ok(None);
        };
        let principal = internal_principal(owner);
        let idempotency_key = format!(
            "agent-runtime-uninstall:{}:{}",
            record.agent_doc_object_id,
            after_failed_task.unwrap_or("first")
        );
        if let Some(task) = self
            .find_app_submit_replay(&principal, &idempotency_key)
            .await?
        {
            self.app_installer
                .spawn_lifecycle_task(task.task_id.clone());
            return Ok(Some(task.task_id));
        }
        let lease = Self::acquire_app_mutation(
            &spec.app_instance_id,
            owner,
            INTERNAL_APP_ID,
            &idempotency_key,
        )
        .await?;
        let task_id = match self
            .app_installer
            .uninstall_app(
                &spec,
                &spec.app_instance_id.to_string(),
                AppDataDisposition::Retain,
                owner,
                INTERNAL_APP_ID,
                &idempotency_key,
            )
            .await
        {
            Ok(task_id) => task_id,
            Err(error) => {
                Self::release_app_mutation_key(&lease).await;
                return Err(error);
            }
        };
        Self::bind_app_mutation_task(&lease, &task_id).await?;
        self.app_installer.spawn_lifecycle_task(task_id.clone());
        Ok(Some(task_id))
    }

    /// apps.submit of an Agent PIKG: register it as the submitter's template
    /// instead of deploying it.
    pub(crate) async fn register_agent_template(
        &self,
        req: &RPCRequest,
        principal: &RpcAuthPrincipal,
        inspection: &InstallInspection,
    ) -> Result<Value, RPCErrors> {
        let InstallSource::LocalPikg { staging_handle } = Self::parse_install_source(req)? else {
            return Err(reason(
                "template_source_unsupported",
                "Agent templates are installed from a PIKG",
            ));
        };
        let InstallSourceIdentity::Pikg { pikg_digest, .. } = &inspection.plan.source_identity
        else {
            return Err(reason(
                "template_source_unsupported",
                "Agent templates are installed from a PIKG",
            ));
        };
        let readiness = &inspection.status.readiness;
        if !readiness.trust.is_ready() || !readiness.package_integrity.is_ready() {
            return Err(Self::install_error_to_rpc(InstallError::new(
                InstallStage::Inspect,
                InstallErrorCode::TrustResolutionRequired,
                false,
                "Agent template PIKG is not trusted or not intact",
            )));
        }
        let runtime = get_buckyos_api_runtime()?;
        let (metadata, staged_path) = self
            .staging_store
            .resolve(
                &staging_handle,
                &principal.username,
                &principal.authenticated_app_id,
                &runtime.zone_id,
                PikgStagingPurpose::Inspect,
                None,
            )
            .await?;
        let reader = PikgReader::open(&staged_path, Some(&metadata.pikg_digest))
            .await
            .map_err(template_error)?;
        let template = reader.inspection();
        if &metadata.pikg_digest != pikg_digest
            || template.app_doc_object_id != inspection.plan.app.object_id
        {
            return Err(template_error(
                "the staged PIKG is not the inspected App Document",
            ));
        }
        validate_template(&reader, inspection.plan.app_instance_id.app_id())?;
        let owner = inspection.plan.owner_user_id.clone();
        let kv = ZoneKv::connect().await?;
        let action = store_agent_template(
            &kv,
            &get_buckyos_root_dir(),
            &owner,
            &template.app_doc,
            &template.app_doc_object_id,
            pikg_digest,
            &staged_path,
            buckyos_get_unix_timestamp(),
        )
        .await?;
        info!(
            "Agent template {} for {owner}: {}",
            inspection.plan.app_instance_id.app_id(),
            action.as_str()
        );
        if action == TemplateStoreAction::Updated {
            let server = self.clone();
            tokio::spawn(async move {
                if let Err(error) = server.refresh_agent_templates().await {
                    warn!("Agent template update failed: {error}");
                }
            });
        }
        Ok(json!({
            "action": action.as_str(),
            "task_id": null,
            "template_id": template_id_of(
                AgentTemplateSource::Installed,
                inspection.plan.app_instance_id.app_id()
            ),
            "app_doc_object_id": template.app_doc_object_id,
        }))
    }

    fn refresh_agent_templates(&self) -> BoxFuture<'static, Result<(), RPCErrors>> {
        let server = self.clone();
        Box::pin(async move {
            let kv = ZoneKv::connect().await?;
            let settings = load_install_settings(&kv).await?;
            server
                .reconcile_agent_template_updates(&settings.agent_templates)
                .await
        })
    }

    /// Rebuilds the runtime App of every auto-updating Agent whose template
    /// AppDoc changed since it was built.
    pub(crate) async fn reconcile_agent_template_updates(
        &self,
        bundled: &HashMap<String, AgentTemplateConfig>,
    ) -> Result<(), RPCErrors> {
        let kv = ZoneKv::connect().await?;
        let root = get_buckyos_root_dir();
        let mut latest: HashMap<String, Option<ObjId>> = HashMap::new();
        for (owner, record) in list_records(&kv, None).await? {
            if record.state != AgentInstallState::Ready || record.runtime_task_id.is_some() {
                continue;
            }
            let settings =
                get_value::<AgentSettings>(&kv, &agent_settings_key(&owner, &record.agent_id))
                    .await
                    .unwrap_or_default();
            if !settings.template_auto_update {
                continue;
            }
            let cache_key = match record.template_source {
                AgentTemplateSource::Bundled => record.template_id.clone(),
                AgentTemplateSource::Installed => format!("{owner}/{}", record.template_id),
            };
            if !latest.contains_key(&cache_key) {
                let current = match parse_template_id(&record.template_id) {
                    Ok((AgentTemplateSource::Bundled, app_id)) => {
                        match bundled.get(app_id.as_str()) {
                            Some(config) => open_bundled_template(&root, config)
                                .await
                                .ok()
                                .map(|reader| reader.inspection().app_doc_object_id.clone()),
                            None => None,
                        }
                    }
                    Ok((AgentTemplateSource::Installed, app_id)) => {
                        get_value::<AgentTemplateRecord>(
                            &kv,
                            &agent_template_record_key(&owner, &app_id),
                        )
                        .await
                        .map(|template| template.app_doc_object_id)
                    }
                    Err(_) => None,
                };
                latest.insert(cache_key.clone(), current);
            }
            let Some(Some(current)) = latest.get(&cache_key) else {
                continue;
            };
            if *current == record.template_app_doc_object_id {
                continue;
            }
            if let Err(error) = self
                .start_agent_template_update(&kv, &owner, &record.agent_id)
                .await
            {
                warn!(
                    "template update of Agent {} failed: {error}",
                    record.agent_id
                );
            }
        }
        Ok(())
    }

    async fn start_agent_template_update(
        &self,
        kv: &dyn AgentKv,
        owner: &str,
        agent_id: &AgentId,
    ) -> Result<(), RPCErrors> {
        let key = agent_key(owner, agent_id);
        if !self.claim_agent(&key) {
            return Ok(());
        }
        let result = async {
            let Some((mut record, version)) = load_record(kv, owner, agent_id).await? else {
                return Ok(());
            };
            if record.state != AgentInstallState::Ready || record.runtime_task_id.is_some() {
                return Ok(());
            }
            match self.submit_agent_runtime(kv, owner, &record).await {
                Ok(RuntimeSubmit::Satisfied(template)) => apply_template(&mut record, template),
                Ok(RuntimeSubmit::Task(task_id)) => {
                    info!("Agent {agent_id} runtime update task {task_id}");
                    record.runtime_task_id = Some(task_id);
                }
                Err(failure) => {
                    return Err(reason(&failure.code, failure.message));
                }
            }
            record.updated_at = buckyos_get_unix_timestamp();
            commit_record(kv, owner, &record, version, HashMap::new()).await?;
            Ok(())
        }
        .await;
        self.release_agent(&key);
        self.schedule_agent_drive(owner, agent_id);
        result
    }

    async fn runtime_progress(&self, owner: &str, task_id: &str) -> Option<Value> {
        let status = self
            .install_engine
            .status(task_id, owner, true)
            .await
            .ok()?;
        if status.task_phase.is_terminal() {
            return None;
        }
        Some(json!({
            "phase": status.stage,
            "percent": status.completed_stages.len() * 100 / InstallStage::ALL.len(),
            "message": status.progress.and_then(|progress| progress.message),
        }))
    }

    async fn agent_status(&self, owner: &str, record: &AgentInstallRecord) -> Value {
        let mut status = status_view(owner, record);
        if let Some(task_id) = record.runtime_task_id.as_deref() {
            if let Some(progress) = self.runtime_progress(owner, task_id).await {
                status["runtime_progress"] = progress;
            }
        }
        status
    }

    async fn agent_entry(
        &self,
        kv: &dyn AgentKv,
        owner: &str,
        record: &AgentInstallRecord,
    ) -> Result<Value, RPCErrors> {
        let agent_id = &record.agent_id;
        let settings = get_value::<AgentSettings>(kv, &agent_settings_key(owner, agent_id))
            .await
            .unwrap_or_default();
        let profile = get_value::<AgentProfile>(kv, &agent_profile_key(owner, agent_id))
            .await
            .unwrap_or_default();
        let spec = get_value::<AgentSpec>(kv, &agent_spec_key(owner, agent_id)).await;
        let info = get_value::<AgentRuntimeInfo>(kv, &agent_info_key(owner, agent_id)).await;
        let owner_did = match spec.as_ref().or(record.pending_spec.as_ref()) {
            Some(spec) => spec.agent_doc.owner.to_string(),
            None => get_value::<Value>(kv, &format!("users/{owner}/doc"))
                .await
                .and_then(|doc| doc.get("id").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_default(),
        };
        let app_id = AppId::parse(agent_id.as_str()).map_err(RPCErrors::ReasonError)?;
        let runtime = get_value::<AppServiceSpec>(kv, &user_app_spec_key(owner, &app_id))
            .await
            .filter(AppServiceSpec::is_installed)
            .map(|spec| runtime_view(&spec))
            .unwrap_or(Value::Null);
        Ok(json!({
            "agent_id": agent_id,
            "agent_did": agent_id.agent_did().to_string(),
            "owner_user_id": owner,
            "owner_did": owner_did,
            "name": agent_short_name(agent_id),
            "display_name": profile.resolved_display_name(agent_id),
            "profile": profile,
            "settings": settings_view(&settings),
            "install": self.agent_status(owner, record).await,
            "template": {
                "template_id": record.template_id,
                "source": record.template_source,
                "app_did": record.template_app_did.to_string(),
                "version": record.template_version,
                "loaded_version": info.and_then(|info| info.template_version),
            },
            "runtime": runtime,
        }))
    }

    async fn owner_has_telegram(kv: &dyn AgentKv, owner: &str) -> bool {
        get_value::<UserPrivateProfile>(kv, &format!("users/{owner}/profile"))
            .await
            .as_ref()
            .and_then(profile_system_contact)
            .is_some_and(|contact| {
                contact.bindings.iter().any(|binding| {
                    binding.platform == TELEGRAM
                        && !bare_telegram_account(&binding.account_id).is_empty()
                })
            })
    }

    // ── agent.check_name ────────────────────────────────────────────────

    pub(crate) async fn handle_agent_check_name(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_agent_creator(principal)?;
        let name = Self::require_param_str(&req, "name")?.trim().to_string();
        let runtime = get_buckyos_api_runtime()?;
        let kv = ZoneKv::connect().await?;
        let index = AgentNameIndex::load(&kv, &runtime.zone_id).await?;
        let agent_did = agent_did_for(&runtime.zone_id, &name);
        let mut result = json!({
            "name": name,
            "available": true,
            "agent_id": agent_did.to_raw_host_name(),
            "agent_did": agent_did.to_string(),
        });
        if let Err(conflict) = index.check(&name) {
            result["available"] = json!(false);
            result["reason"] = json!(conflict.reason);
            result["message"] = json!(conflict.message);
            if conflict.reason != "invalid" {
                if let Some(suggestion) = index.suggest(&principal.owner_user_id, &name) {
                    result["suggestion"] = json!(suggestion);
                }
            }
        }
        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }

    // ── agent.list_templates ────────────────────────────────────────────

    pub(crate) async fn handle_agent_list_templates(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let kv = ZoneKv::connect().await?;
        let templates =
            list_agent_templates(&kv, &get_buckyos_root_dir(), &principal.owner_user_id).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "templates": templates })),
            req.seq,
        ))
    }

    // ── agent.create ────────────────────────────────────────────────────

    pub(crate) async fn handle_agent_create(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_agent_creator(principal)?;
        let owner = principal.owner_user_id.clone();
        let request = AgentCreateRequest::parse(&req.params)?;
        if request.allow_other_users {
            return Err(reason(
                "sharing_unsupported",
                "sharing an Agent with other users is not available yet",
            ));
        }
        let runtime = get_buckyos_api_runtime()?;
        let kv = ZoneKv::connect().await?;
        let fingerprint = request.fingerprint();
        let created = |owner: &str, record: &AgentInstallRecord| {
            json!({
                "agent_id": record.agent_id,
                "agent_did": record.agent_id.agent_did().to_string(),
                "status": status_view(owner, record),
            })
        };

        if let Ok(agent_id) =
            AgentId::from_agent_did(&agent_did_for(&runtime.zone_id, &request.name))
        {
            if let Some((record, _)) = load_record(&kv, &owner, &agent_id).await? {
                if record.idempotency_key == request.idempotency_key
                    && record.state != AgentInstallState::Removed
                {
                    if record.request_fingerprint != fingerprint {
                        return Err(reason(
                            "idempotency_conflict",
                            "the idempotency key was used for a different request",
                        ));
                    }
                    self.schedule_agent_drive(&owner, &agent_id);
                    return Ok(RPCResponse::new(
                        RPCResult::Success(created(&owner, &record)),
                        req.seq,
                    ));
                }
            }
        }
        for (_, record) in list_records(&kv, Some(&owner)).await? {
            if record.idempotency_key == request.idempotency_key {
                return Err(reason(
                    "idempotency_conflict",
                    "the idempotency key was used for a different request",
                ));
            }
        }

        let agent_id = AgentNameIndex::load(&kv, &runtime.zone_id)
            .await?
            .check(&request.name)
            .map_err(NameConflict::into_rpc)?;
        let template =
            load_agent_template(&kv, &get_buckyos_root_dir(), &owner, &request.template_id).await?;
        if request.bot_token.is_some() && !Self::owner_has_telegram(&kv, &owner).await {
            return Err(reason(
                "owner_identity_missing",
                "add your Telegram account to your profile before binding a Telegram bot",
            ));
        }
        let owner_did = get_value::<Value>(&kv, &format!("users/{owner}/doc"))
            .await
            .and_then(|doc| doc.get("id").and_then(Value::as_str).map(str::to_string))
            .or_else(|| Some(principal.owner_did.clone()).filter(|did| !did.is_empty()))
            .ok_or_else(|| reason("owner_identity_missing", "the owner has no DID document"))?;
        let owner_did =
            DID::from_str(&owner_did).map_err(|error| reason("owner_identity_missing", error))?;

        let (private_key, public_key) = generate_ed25519_key_pair();
        let public_key: Jwk = serde_json::from_value(public_key)
            .map_err(|error| RPCErrors::ReasonError(format!("invalid generated key: {error}")))?;
        let spec = new_agent_spec(&agent_id, &owner, owner_did, public_key)?;
        let record = initial_record(
            spec,
            &request,
            &fingerprint,
            template.template_id(),
            template.source,
            template.built_from(),
            buckyos_get_unix_timestamp(),
        );
        let actions = reservation_actions(
            &owner,
            &record,
            &private_key,
            &request.settings(),
            &request.profile,
        )?;
        if let Err(error) = kv.exec_tx(actions, None).await {
            if let Some((existing, _)) = load_record(&kv, &owner, &agent_id).await? {
                if existing.idempotency_key == request.idempotency_key
                    && existing.request_fingerprint == fingerprint
                {
                    return Ok(RPCResponse::new(
                        RPCResult::Success(created(&owner, &existing)),
                        req.seq,
                    ));
                }
            }
            warn!("reserve Agent {agent_id} failed: {error}");
            return Err(reason(
                "name_conflict",
                format!("`{}` was taken concurrently", request.name),
            ));
        }
        info!(
            "Agent {agent_id} reserved by {owner} from {}",
            record.template_id
        );
        self.schedule_agent_drive(&owner, &agent_id);
        Ok(RPCResponse::new(
            RPCResult::Success(created(&owner, &record)),
            req.seq,
        ))
    }

    // ── agent.create.status / retry / cancel ────────────────────────────

    pub(crate) async fn handle_agent_create_status(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let kv = ZoneKv::connect().await?;
        let (owner, record, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(self.agent_status(&owner, &record).await),
            req.seq,
        ))
    }

    pub(crate) async fn handle_agent_create_retry(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let skip_tunnel = Self::param_bool(&req, "skip_tunnel").unwrap_or(false);
        let kv = ZoneKv::connect().await?;
        let (owner, mut record, version) =
            Self::locate_agent(&kv, principal, &agent_id, false).await?;
        let settings_key = agent_settings_key(&owner, &agent_id);
        let settings = get_value::<AgentSettings>(&kv, &settings_key).await;
        let mut actions = HashMap::new();
        if let Some(settings) = prepare_retry(
            &mut record,
            settings,
            skip_tunnel,
            buckyos_get_unix_timestamp(),
        )? {
            actions.insert(settings_key, KVAction::Update(to_json(&settings)?));
        }
        commit_record(&kv, &owner, &record, version, actions)
            .await?
            .ok_or_else(|| reason("agent_busy", "the Agent changed concurrently; try again"))?;
        self.schedule_agent_drive(&owner, &agent_id);
        Ok(RPCResponse::new(
            RPCResult::Success(self.agent_status(&owner, &record).await),
            req.seq,
        ))
    }

    pub(crate) async fn handle_agent_create_cancel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let kv = ZoneKv::connect().await?;
        let (owner, record, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        if !cancel_allowed(&record) {
            return Err(reason(
                "cancel_unavailable",
                "the Agent is already bound; delete it instead",
            ));
        }
        begin_agent_removal(&kv, &owner, &agent_id, buckyos_get_unix_timestamp()).await?;
        self.schedule_agent_drive(&owner, &agent_id);
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true })),
            req.seq,
        ))
    }

    // ── agent.list / agent.get ──────────────────────────────────────────

    pub(crate) async fn handle_agent_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let kv = ZoneKv::connect().await?;
        let scope = (!Self::principal_admin(principal)).then_some(principal.owner_user_id.as_str());
        let mut agents = Vec::new();
        for (owner, record) in list_records(&kv, scope).await? {
            if record.state == AgentInstallState::Removed {
                continue;
            }
            agents.push(self.agent_entry(&kv, &owner, &record).await?);
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "agents": agents })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_agent_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let kv = ZoneKv::connect().await?;
        let (owner, record, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(self.agent_entry(&kv, &owner, &record).await?),
            req.seq,
        ))
    }

    // ── agent.update ────────────────────────────────────────────────────

    pub(crate) async fn handle_agent_update(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let allow_group = Self::param_bool(&req, "allow_group").ok_or_else(|| {
            RPCErrors::ParseRequestError("allow_group must be a boolean".to_string())
        })?;
        let kv = ZoneKv::connect().await?;
        let (owner, record, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        let key = agent_settings_key(&owner, &agent_id);
        let mut settings = get_value::<AgentSettings>(&kv, &key)
            .await
            .unwrap_or_default();
        settings.allow_group = allow_group;
        kv.exec_tx(
            HashMap::from([(key, KVAction::Update(to_json(&settings)?))]),
            None,
        )
        .await?;
        Ok(RPCResponse::new(
            RPCResult::Success(self.agent_entry(&kv, &owner, &record).await?),
            req.seq,
        ))
    }

    // ── agent.profile.get / agent.profile.set ───────────────────────────

    pub(crate) async fn handle_agent_profile_get(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let kv = ZoneKv::connect().await?;
        let (owner, _, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        let profile = get_value::<AgentProfile>(&kv, &agent_profile_key(&owner, &agent_id))
            .await
            .unwrap_or_default();
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "profile": profile })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_agent_profile_set(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let kv = ZoneKv::connect().await?;
        let (owner, _, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        let key = agent_profile_key(&owner, &agent_id);
        let mut profile = get_value::<AgentProfile>(&kv, &key)
            .await
            .unwrap_or_default();
        for (field, slot) in [
            ("display_name", &mut profile.display_name),
            ("avatar", &mut profile.avatar),
            ("bio", &mut profile.bio),
        ] {
            if let Some(value) = req.params.get(field).filter(|value| !value.is_null()) {
                *slot = optional_text(Some(value));
            }
        }
        if let Some(avatar) = profile.avatar.as_deref() {
            validate_avatar(avatar)?;
        }
        kv.exec_tx(
            HashMap::from([(key, KVAction::Update(to_json(&profile)?))]),
            None,
        )
        .await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "profile": profile })),
            req.seq,
        ))
    }

    // ── agent.delete ────────────────────────────────────────────────────

    pub(crate) async fn handle_agent_delete(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let kv = ZoneKv::connect().await?;
        let (owner, _, _) = Self::locate_agent(&kv, principal, &agent_id, true).await?;
        begin_agent_removal(&kv, &owner, &agent_id, buckyos_get_unix_timestamp()).await?;
        info!(
            "Agent {agent_id} removal requested by {}",
            principal.username
        );
        self.schedule_agent_drive(&owner, &agent_id);
        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "ok": true })),
            req.seq,
        ))
    }

    // ── agent.set_msg_tunnel / agent.remove_msg_tunnel ──────────────────

    pub(crate) async fn handle_agent_set_msg_tunnel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let platform = Self::require_param_str(&req, "platform")?;
        if platform != TELEGRAM {
            return Err(RPCErrors::ParseRequestError(
                "only the telegram tunnel is supported".to_string(),
            ));
        }
        let bot_token = optional_text(req.params.get("bot_token"))
            .ok_or_else(|| RPCErrors::ParseRequestError("Missing bot_token".to_string()))?;
        validate_bot_token(&bot_token)?;
        let kv = ZoneKv::connect().await?;
        let (owner, _, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        if !Self::owner_has_telegram(&kv, &owner).await {
            return Err(reason(
                "owner_identity_missing",
                "add your Telegram account to your profile before binding a Telegram bot",
            ));
        }
        let key = agent_settings_key(&owner, &agent_id);
        let mut settings = get_value::<AgentSettings>(&kv, &key)
            .await
            .unwrap_or_default();
        let bot_account_id = match settings
            .msg_tunnels
            .iter()
            .find(|tunnel| tunnel.platform == platform && tunnel.bot_token == bot_token)
            .and_then(|tunnel| tunnel.bot_account_id.clone())
        {
            Some(bot_id) => bot_id,
            None => verify_telegram_bot(&bot_token)
                .await
                .map_err(|failure| reason(&failure.code, failure.message))?,
        };
        settings.msg_tunnels = vec![AgentMsgTunnelSettings {
            platform: platform.clone(),
            bot_token,
            bot_account_id: Some(bot_account_id),
        }];
        kv.exec_tx(
            HashMap::from([(key, KVAction::Update(to_json(&settings)?))]),
            None,
        )
        .await?;
        if let Err(error) = reload_msg_center().await {
            warn!("msg-center reload after binding {agent_id} tunnel failed: {error}");
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "agent_id": agent_id,
                "platform": platform,
                "total_bindings": settings.msg_tunnels.len(),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_agent_remove_msg_tunnel(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let agent_id = Self::param_agent_id(&req)?;
        let platform = Self::require_param_str(&req, "platform")?;
        let kv = ZoneKv::connect().await?;
        let (owner, _, _) = Self::locate_agent(&kv, principal, &agent_id, false).await?;
        let key = agent_settings_key(&owner, &agent_id);
        let mut settings = get_value::<AgentSettings>(&kv, &key)
            .await
            .unwrap_or_default();
        let before = settings.msg_tunnels.len();
        settings
            .msg_tunnels
            .retain(|tunnel| tunnel.platform != platform);
        if settings.msg_tunnels.len() == before {
            return Err(reason(
                "tunnel_not_found",
                format!("Agent {agent_id} has no {platform} tunnel"),
            ));
        }
        kv.exec_tx(
            HashMap::from([(key, KVAction::Update(to_json(&settings)?))]),
            None,
        )
        .await?;
        if let Err(error) = reload_msg_center().await {
            warn!("msg-center reload after removing {agent_id} tunnel failed: {error}");
        }
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "ok": true,
                "agent_id": agent_id,
                "platform": platform,
                "remaining_bindings": settings.msg_tunnels.len(),
            })),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AppAllocation, AppInstanceAllocation, DeploymentIdentity, ServiceEndpointConfig,
        ServiceProtocol, ServiceSpecConfig, ServiceState, APP_REGISTRY_SCHEMA_VERSION,
    };
    use ndn_lib::{ChunkId, ChunkType, OBJ_TYPE_PKG};
    use sha2::{Digest, Sha256};
    use std::collections::{BTreeMap, VecDeque};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Mutex;

    const OWNER: &str = "devtest";

    #[derive(Default)]
    struct MemoryKv {
        data: Mutex<HashMap<String, (String, u64)>>,
        revision: AtomicU64,
    }

    impl MemoryKv {
        fn put<T: Serialize>(&self, key: &str, value: &T) {
            let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
            self.data.lock().unwrap().insert(
                key.to_string(),
                (serde_json::to_string(value).unwrap(), revision),
            );
        }

        fn has(&self, key: &str) -> bool {
            self.data.lock().unwrap().contains_key(key)
        }

        fn read<T: DeserializeOwned>(&self, key: &str) -> T {
            serde_json::from_str(&self.data.lock().unwrap()[key].0).unwrap()
        }
    }

    #[async_trait]
    impl AgentKv for MemoryKv {
        async fn get(&self, key: &str) -> Result<Option<(String, u64)>, RPCErrors> {
            Ok(self.data.lock().unwrap().get(key).cloned())
        }

        async fn list(&self, key: &str) -> Result<Vec<String>, RPCErrors> {
            let prefix = format!("{key}/");
            let mut children = self
                .data
                .lock()
                .unwrap()
                .keys()
                .filter_map(|item| item.strip_prefix(&prefix))
                .map(|rest| rest.split('/').next().unwrap().to_string())
                .collect::<Vec<_>>();
            children.sort();
            children.dedup();
            Ok(children)
        }

        async fn exec_tx(
            &self,
            actions: HashMap<String, KVAction>,
            main_key: Option<(String, u64)>,
        ) -> Result<(), RPCErrors> {
            let mut data = self.data.lock().unwrap();
            if let Some((key, expected)) = main_key {
                if data.get(&key).map(|(_, revision)| *revision).unwrap_or(0) != expected {
                    return Err(RPCErrors::ReasonError("revision mismatch".to_string()));
                }
            }
            for (key, action) in &actions {
                if matches!(action, KVAction::Create(_)) && data.contains_key(key) {
                    return Err(RPCErrors::ReasonError(format!("{key} exists")));
                }
            }
            for (key, action) in actions {
                let revision = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
                match action {
                    KVAction::Create(value) | KVAction::Update(value) => {
                        data.insert(key, (value, revision));
                    }
                    KVAction::Remove => {
                        data.remove(&key);
                    }
                    _ => unreachable!(),
                }
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeOps {
        kv: MemoryKv,
        submits: Mutex<VecDeque<Result<RuntimeSubmit, AgentFailure>>>,
        tasks: Mutex<HashMap<String, (TaskKind, VecDeque<TaskState>)>>,
        uninstalls: Mutex<VecDeque<Option<String>>>,
        telegram: Mutex<Option<Result<String, AgentFailure>>>,
        calls: Mutex<Vec<String>>,
        start_timeout: u64,
    }

    impl FakeOps {
        fn new() -> Self {
            Self {
                start_timeout: 300,
                ..Default::default()
            }
        }

        fn call(&self, call: impl Into<String>) {
            self.calls.lock().unwrap().push(call.into());
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn task(&self, task_id: &str, kind: TaskKind, states: Vec<TaskState>) {
            self.tasks
                .lock()
                .unwrap()
                .insert(task_id.to_string(), (kind, states.into()));
        }

        fn record(&self, agent_id: &AgentId) -> AgentInstallRecord {
            self.kv.read(&agent_install_record_key(OWNER, agent_id))
        }

        async fn drive(&self, agent_id: &AgentId) {
            for _ in 0..MAX_DRIVE_STEPS {
                if !drive_agent_step(self, OWNER, agent_id).await.unwrap() {
                    return;
                }
            }
            panic!("drive did not settle");
        }
    }

    #[async_trait]
    impl AgentDriverOps for FakeOps {
        fn kv(&self) -> &dyn AgentKv {
            &self.kv
        }

        async fn submit_runtime(
            &self,
            _owner: &str,
            _record: &AgentInstallRecord,
        ) -> Result<RuntimeSubmit, AgentFailure> {
            self.call("submit_runtime");
            self.submits
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected submit")
        }

        async fn task_state(
            &self,
            _owner: &str,
            task_id: &str,
        ) -> Result<(TaskKind, TaskState), RPCErrors> {
            let mut tasks = self.tasks.lock().unwrap();
            let (kind, states) = tasks.get_mut(task_id).expect("unknown task");
            let state = if states.len() > 1 {
                states.pop_front().unwrap()
            } else {
                states.front().cloned().unwrap()
            };
            Ok((*kind, state))
        }

        async fn resume_runtime(&self, _owner: &str, task_id: &str) -> Result<String, AgentFailure> {
            self.call(format!("resume_runtime {task_id}"));
            Ok(task_id.to_string())
        }

        async fn refresh_rbac(&self) -> Result<(), RPCErrors> {
            self.call("refresh_rbac");
            Ok(())
        }

        async fn verify_telegram_bot(&self, bot_token: &str) -> Result<String, AgentFailure> {
            self.call(format!("verify_telegram {bot_token}"));
            self.telegram
                .lock()
                .unwrap()
                .clone()
                .expect("unexpected verify")
        }

        async fn reload_msg_center(&self) -> Result<(), RPCErrors> {
            self.call("reload_msg_center");
            Ok(())
        }

        async fn start_uninstall(
            &self,
            owner: &str,
            record: &AgentInstallRecord,
            after_failed_task: Option<&str>,
        ) -> Result<Option<String>, RPCErrors> {
            let spec_present = self.kv.has(&agent_spec_key(owner, &record.agent_id));
            self.call(format!(
                "start_uninstall spec_present={spec_present} after={}",
                after_failed_task.unwrap_or("-")
            ));
            Ok(self.uninstalls.lock().unwrap().pop_front().flatten())
        }

        fn now(&self) -> u64 {
            buckyos_get_unix_timestamp()
        }

        fn poll_interval(&self) -> Duration {
            Duration::from_millis(1)
        }

        fn start_timeout_secs(&self) -> u64 {
            self.start_timeout
        }
    }

    fn zone() -> DID {
        DID::from_str("did:web:test.buckyos.io").unwrap()
    }

    fn built(version: &str) -> BuiltFromTemplate {
        BuiltFromTemplate {
            app_did: DID::from_str("did:bns:jarvis.buckyos").unwrap(),
            app_doc_object_id: ObjId::new_by_raw(
                "appdoc".to_string(),
                vec![version.len() as u8; 32],
            ),
            version: version.to_string(),
        }
    }

    fn template_doc(meta_id: &ObjId, version: &str) -> AppDoc {
        serde_json::from_value(json!({
            "schema_version": 1,
            "doc_type": "app",
            "did": "did:bns:jarvis.buckyos",
            "name": "buckyos-jarvis",
            "version": version,
            "app_type": "agent",
            "author": "did:bns:buckyos",
            "owner": "did:bns:buckyos",
            "controller": "did:bns:buckyos",
            "create_time": 1791542214u64,
            "last_update_time": 1791542214u64,
            "exp": 1949222214u64,
            "categories": ["agent"],
            "pkg_list": {
                "agent": {
                    "pkg_id": format!("all.agent.jarvis.buckyos.bns.did#{version}"),
                    "pkg_objid": meta_id,
                    "required": true
                }
            },
            "show_name": "Jarvis",
            "selector_type": "single",
            "service_config_tips": {
                "service_endpoints": {
                    "www": {
                        "protocol": "http",
                        "inner_port": 4060,
                        "required": true,
                        "expose": {"route": {"type": "web"}, "scope": "", "allow_guest": false}
                    }
                }
            }
        }))
        .unwrap()
    }

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "buckyos-agent-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    async fn template_pikg(dir: &Path, version: &str) -> PathBuf {
        use flate2::{write::GzEncoder, Compression};
        let payload = dir.join(format!("agent-{version}.tar.gz"));
        let encoder = GzEncoder::new(
            std::fs::File::create(&payload).unwrap(),
            Compression::default(),
        );
        let mut archive = tar::Builder::new(encoder);
        let body = format!("role = \"jarvis {version}\"\n");
        let mut header = tar::Header::new_gnu();
        header.set_size(body.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "agent.toml", body.as_bytes())
            .unwrap();
        archive.into_inner().unwrap().finish().unwrap();
        let bytes = std::fs::read(&payload).unwrap();
        let content = ChunkId::from_mix_hash_result(
            bytes.len() as u64,
            &Sha256::digest(&bytes),
            ChunkType::Mix256,
        );
        let mut meta = package_lib::PackageMeta::new(
            "all.agent.jarvis.buckyos.bns.did",
            version,
            "did:bns:buckyos",
            &DID::from_str("did:bns:buckyos").unwrap(),
            None,
        );
        meta.size = bytes.len() as u64;
        meta.content = content.to_string();
        let meta = serde_json::to_value(meta).unwrap();
        let (meta_id, _) = build_named_object_by_json(OBJ_TYPE_PKG, &meta);
        let (builder, _) = PikgBuilder::new()
            .app_doc(&template_doc(&meta_id, version))
            .unwrap()
            .add_package_meta_value(meta)
            .unwrap();
        let path = dir.join(format!("jarvis-{version}.pikg"));
        builder
            .add_payload_file("agent", &payload)
            .unwrap()
            .write_to(&path)
            .await
            .unwrap();
        path
    }

    async fn reserve(ops: &FakeOps, name: &str, bot_token: Option<&str>) -> AgentId {
        let agent_id = AgentId::from_agent_did(&agent_did_for(&zone(), name)).unwrap();
        let (private_key, public_key) = generate_ed25519_key_pair();
        let spec = new_agent_spec(
            &agent_id,
            OWNER,
            DID::from_str("did:web:devtest.test.buckyos.io").unwrap(),
            serde_json::from_value(public_key).unwrap(),
        )
        .unwrap();
        let request = AgentCreateRequest::parse(&json!({
            "idempotency_key": format!("key-{name}"),
            "name": name,
            "template_id": "bundled:jarvis.buckyos.bns.did",
            "profile": {"display_name": "Xiao Bai"},
            "msg_tunnel": bot_token.map(|token| json!({"platform": "telegram", "bot_token": token})),
        }))
        .unwrap();
        let record = initial_record(
            spec,
            &request,
            &request.fingerprint(),
            request.template_id.clone(),
            AgentTemplateSource::Bundled,
            built("0.7.0"),
            buckyos_get_unix_timestamp(),
        );
        ops.kv
            .exec_tx(
                reservation_actions(
                    OWNER,
                    &record,
                    &private_key,
                    &request.settings(),
                    &request.profile,
                )
                .unwrap(),
                None,
            )
            .await
            .unwrap();
        agent_id
    }

    fn runtime_spec(agent_id: &AgentId, base_on: &ObjId, version: &str) -> AppServiceSpec {
        let app_instance_id =
            AppInstanceId::new(AppId::parse(agent_id.as_str()).unwrap(), OWNER).unwrap();
        let mut app_doc = template_doc(&ObjId::new_by_raw("pkg".to_string(), vec![1; 32]), version);
        app_doc.base_on = Some(base_on.clone());
        let mut spec_config = ServiceSpecConfig::default();
        spec_config.service_config.insert(
            AGENT_SERVICE_NAME.to_string(),
            ServiceEndpointConfig {
                protocol: ServiceProtocol::Http,
                inner_port: 4060,
            },
        );
        AppServiceSpec {
            app_instance_id: app_instance_id.clone(),
            app_did: agent_id.agent_did(),
            deployment: DeploymentIdentity {
                app_instance_id,
                task_id: "t-runtime".to_string(),
                app_doc_object_id: ObjId::new_by_raw("appdoc".to_string(), vec![9; 32]),
                spec_generation: 1,
                pikg_digest: None,
            },
            app_doc,
            app_name: agent_short_name(agent_id),
            app_host_name: agent_short_name(agent_id),
            app_index: 7,
            owner_user_id: OWNER.to_string(),
            permission: Vec::new(),
            selected_components: Vec::new(),
            packages: Vec::new(),
            enable: true,
            expected_instance_count: 1,
            state: ServiceState::Running,
            spec_config,
        }
    }

    fn install_runtime(ops: &FakeOps, agent_id: &AgentId) {
        let app_id = AppId::parse(agent_id.as_str()).unwrap();
        ops.kv.put(
            &user_app_spec_key(OWNER, &app_id),
            &runtime_spec(agent_id, &built("0.7.0").app_doc_object_id, "0.7.0"),
        );
    }

    fn report_loaded(ops: &FakeOps, agent_id: &AgentId) {
        let spec: AgentSpec = ops.kv.read(&agent_spec_key(OWNER, agent_id));
        ops.kv.put(
            &agent_info_key(OWNER, agent_id),
            &AgentRuntimeInfo {
                agent_doc_object_id: spec.agent_doc_object_id,
                generation: spec.generation,
                loaded_at: 1,
                template_version: Some("0.7.0".to_string()),
            },
        );
    }

    #[test]
    fn agent_and_constructed_app_share_one_identifier() {
        let did = agent_did_for(&zone(), "xiaobai");
        let agent_id = AgentId::from_agent_did(&did).unwrap();
        let app_id = AppId::from_app_did(&did).unwrap();
        assert_eq!(agent_id.as_str(), app_id.as_str());
        assert_eq!(agent_id.as_str(), "xiaobai.test.buckyos.io");
        assert_eq!(did.upper_did(), Some(zone()));
        let bns = agent_did_for(&DID::from_str("did:bns:alice").unwrap(), "xiaobai");
        assert_eq!(
            AgentId::from_agent_did(&bns).unwrap().as_str(),
            AppId::from_app_did(&bns).unwrap().as_str()
        );
    }

    #[tokio::test]
    async fn name_check_covers_users_agents_hosts_and_suggests_alternatives() {
        let kv = MemoryKv::default();
        kv.put("users/devtest/settings", &json!({}));
        kv.put("users/alice/settings", &json!({}));
        kv.put(
            "users/alice/agents/jarvis.test.buckyos.io/install_record",
            &json!({}),
        );
        kv.put(
            &format!("{AGENT_NAME_PREFIX}/pending.test.buckyos.io"),
            &json!({"owner_user_id": "bob"}),
        );
        let notes = AppId::parse("notes.publisher.bns.did").unwrap();
        let old_agent = AppId::parse("xiaobai.test.buckyos.io").unwrap();
        let registry = AppRegistry {
            schema_version: APP_REGISTRY_SCHEMA_VERSION,
            next_app_index: 3,
            apps: BTreeMap::from([
                (
                    notes.clone(),
                    AppAllocation {
                        app_did: notes.app_did(),
                        app_name: "notes".to_string(),
                        allocated_at: 1,
                    },
                ),
                (
                    old_agent.clone(),
                    AppAllocation {
                        app_did: old_agent.app_did(),
                        app_name: "xiaobai".to_string(),
                        allocated_at: 1,
                    },
                ),
            ]),
            instances: BTreeMap::from([(
                AppInstanceId::new(notes.clone(), "alice").unwrap(),
                AppInstanceAllocation {
                    app_id: notes.clone(),
                    owner_user_id: "alice".to_string(),
                    app_host_name: "notes-alice".to_string(),
                    app_index: 1,
                    allocated_at: 1,
                },
            )]),
            updated_at: 1,
        };
        kv.put(APP_REGISTRY_KEY, &registry);
        kv.put(
            GATEWAY_SETTINGS_KEY,
            &json!({"shortcuts": {"files": {"type": "app"}}}),
        );
        let index = AgentNameIndex::load(&kv, &zone()).await.unwrap();
        let reason_of = |name: &str| index.check(name).err().map(|conflict| conflict.reason);
        assert_eq!(reason_of("Bad_Name"), Some("invalid"));
        assert_eq!(reason_of("-edge"), Some("invalid"));
        assert_eq!(reason_of(&"a".repeat(64)), Some("invalid"));
        assert_eq!(reason_of("homestation"), Some("reserved"));
        assert_eq!(reason_of("alice"), Some("user_exists"));
        assert_eq!(reason_of("jarvis"), Some("agent_exists"));
        assert_eq!(reason_of("pending"), Some("agent_exists"));
        assert_eq!(reason_of("notes"), Some("host_taken"));
        assert_eq!(reason_of("notes-alice"), Some("host_taken"));
        assert_eq!(reason_of("files"), Some("host_taken"));
        assert_eq!(
            index.check("xiaobai").unwrap().as_str(),
            "xiaobai.test.buckyos.io"
        );
        assert_eq!(
            index.suggest("devtest", "jarvis").as_deref(),
            Some("devtest-jarvis")
        );
        assert_eq!(
            index.suggest("dev_test", "alice").as_deref(),
            Some("dev-test-alice")
        );

        kv.put(
            "users/bob/agents/devtest-jarvis.test.buckyos.io/install_record",
            &json!({}),
        );
        let index = AgentNameIndex::load(&kv, &zone()).await.unwrap();
        assert_eq!(
            index.suggest("devtest", "jarvis").as_deref(),
            Some("devtest-jarvis-2")
        );
    }

    #[test]
    fn create_request_is_normalized_and_fingerprinted() {
        let params = json!({
            "idempotency_key": "k1",
            "name": "xiaobai",
            "profile": {"display_name": " 小白 ", "bio": ""},
            "allow_group": true,
            "template_id": "bundled:jarvis.buckyos.bns.did",
            "desktop_entry": "jarvis_guide",
            "msg_tunnel": {"platform": "telegram", "bot_token": "123:abc_DEF-9"},
        });
        let request = AgentCreateRequest::parse(&params).unwrap();
        assert_eq!(request.profile.display_name.as_deref(), Some("小白"));
        assert_eq!(request.profile.bio, None);
        assert!(request.template_auto_update);
        assert!(!request.allow_other_users);
        let settings = request.settings();
        assert!(settings.allow_group);
        assert_eq!(settings.desktop_entry.as_deref(), Some("jarvis_guide"));
        assert_eq!(settings.msg_tunnels[0].bot_token, "123:abc_DEF-9");
        assert_eq!(settings.msg_tunnels[0].bot_account_id, None);
        assert_eq!(
            request.fingerprint(),
            AgentCreateRequest::parse(&params).unwrap().fingerprint()
        );
        let mut changed = params.clone();
        changed["allow_group"] = json!(false);
        assert_ne!(
            request.fingerprint(),
            AgentCreateRequest::parse(&changed).unwrap().fingerprint()
        );
        let mut bad_token = params.clone();
        bad_token["msg_tunnel"]["bot_token"] = json!("not a token");
        assert!(AgentCreateRequest::parse(&bad_token).is_err());
        let mut missing = params;
        missing.as_object_mut().unwrap().remove("idempotency_key");
        assert!(AgentCreateRequest::parse(&missing).is_err());
        assert_eq!(bare_telegram_account(" user:42 "), "42");
        assert_eq!(bare_telegram_account("42"), "42");
    }

    #[test]
    fn avatar_must_be_a_small_image_data_url() {
        let encode = |size: u32| {
            let mut bytes = Vec::new();
            image::RgbaImage::new(size, size)
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            format!("data:image/png;base64,{}", STANDARD.encode(bytes))
        };
        assert!(validate_avatar(&encode(192)).is_ok());
        assert!(validate_avatar(&encode(193)).is_err());
        assert!(validate_avatar("https://example.com/a.png").is_err());
        assert!(validate_avatar("data:image/png;base64,!!!").is_err());
    }

    #[tokio::test]
    async fn constructed_runtime_pikg_rebases_identity_and_packages() {
        let dir = temp_dir("construct");
        let template_path = template_pikg(&dir, "0.7.0").await;
        let template = PikgReader::open(&template_path, None).await.unwrap();
        let agent_id = AgentId::parse("xiaobai.test.buckyos.io").unwrap();
        let built = build_agent_runtime_pikg(&template, &agent_id, "小白", &dir.join("a"))
            .await
            .unwrap();
        let doc = &built.app_doc;
        assert_eq!(doc.app_did().to_string(), "did:web:xiaobai.test.buckyos.io");
        assert_eq!(doc.owner, zone());
        assert_eq!(doc.name, "xiaobai");
        assert_eq!(doc.show_name, "小白");
        assert_eq!(doc.version, "0.7.0");
        assert_eq!(doc.get_app_type(), AppType::Agent);
        assert_eq!(
            doc.base_on.as_ref(),
            Some(&template.inspection().app_doc_object_id)
        );
        let agent = doc.pkg_list.agent.as_ref().unwrap();
        assert_eq!(agent.pkg_id, "all.agent.xiaobai.test.buckyos.io#0.7.0");
        doc.validate().unwrap();

        let reader = PikgReader::open(&built.path, None).await.unwrap();
        reader.verify_all_contents().await.unwrap();
        let inspection = reader.inspection();
        assert_eq!(inspection.app_doc_object_id, built.app_doc_object_id);
        let meta = &inspection.package_meta.package_objects
            [&agent.pkg_objid.as_ref().unwrap().to_string()];
        assert_eq!(meta["name"], "all.agent.xiaobai.test.buckyos.io");
        assert_eq!(meta["version"], "0.7.0");
        let template_meta = template
            .inspection()
            .package_meta
            .package_objects
            .values()
            .next()
            .unwrap();
        assert_eq!(meta["content"], template_meta["content"]);
        assert_eq!(
            inspection
                .package_meta
                .content_index
                .keys()
                .collect::<Vec<_>>(),
            template
                .inspection()
                .package_meta
                .content_index
                .keys()
                .collect::<Vec<_>>()
        );

        let again = build_agent_runtime_pikg(&template, &agent_id, "小白", &dir.join("b"))
            .await
            .unwrap();
        let again = PikgReader::open(&again.path, None).await.unwrap();
        assert_eq!(again.pikg_digest(), reader.pikg_digest());
        let other = build_agent_runtime_pikg(
            &template,
            &AgentId::parse("helper.test.buckyos.io").unwrap(),
            "Helper",
            &dir.join("c"),
        )
        .await
        .unwrap();
        assert_ne!(other.app_doc_object_id, built.app_doc_object_id);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn submitted_agent_templates_are_registered_satisfied_and_updated() {
        let root = temp_dir("templates");
        let kv = MemoryKv::default();
        let source = root.join("src");
        std::fs::create_dir_all(&source).unwrap();
        let first = PikgReader::open(&template_pikg(&source, "0.7.0").await, None)
            .await
            .unwrap();
        let store = |reader: &PikgReader| {
            let inspection = reader.inspection().clone();
            let digest = reader.pikg_digest().to_string();
            let path = source.join(format!("jarvis-{}.pikg", inspection.app_doc.version));
            let root = root.clone();
            let kv = &kv;
            async move {
                store_agent_template(
                    kv,
                    &root,
                    OWNER,
                    &inspection.app_doc,
                    &inspection.app_doc_object_id,
                    &digest,
                    &path,
                    1,
                )
                .await
                .unwrap()
            }
        };
        assert_eq!(store(&first).await, TemplateStoreAction::Registered);
        assert_eq!(store(&first).await, TemplateStoreAction::Satisfied);
        let app_id = AppId::parse("jarvis.buckyos.bns.did").unwrap();
        let record: AgentTemplateRecord = kv.read(&agent_template_record_key(OWNER, &app_id));
        assert_eq!(
            record.pikg_path,
            format!(
                "{AGENT_TEMPLATE_DIR}/{OWNER}/{app_id}/{}.pikg",
                first.pikg_digest()
            )
        );
        let first_file = root.join(&record.pikg_path);
        assert!(first_file.is_file());

        let second = PikgReader::open(&template_pikg(&source, "0.7.1").await, None)
            .await
            .unwrap();
        assert_eq!(store(&second).await, TemplateStoreAction::Updated);
        let record: AgentTemplateRecord = kv.read(&agent_template_record_key(OWNER, &app_id));
        assert_eq!(record.version, "0.7.1");
        assert_eq!(
            record.app_doc_object_id,
            second.inspection().app_doc_object_id
        );
        assert!(!first_file.exists());
        let loaded = load_agent_template(&kv, &root, OWNER, "installed:jarvis.buckyos.bns.did")
            .await
            .unwrap();
        assert_eq!(loaded.object_id(), &second.inspection().app_doc_object_id);
        assert!(loaded.summary()["is_default"] == json!(false));
        let _ = std::fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn creation_drives_runtime_bind_tunnel_and_start_in_order() {
        let ops = FakeOps::new();
        let agent_id = reserve(&ops, "xiaobai", Some("123:abc")).await;
        assert!(!ops.kv.has(&agent_spec_key(OWNER, &agent_id)));
        ops.submits
            .lock()
            .unwrap()
            .push_back(Ok(RuntimeSubmit::Task("t-install".to_string())));
        ops.task(
            "t-install",
            TaskKind::Install,
            vec![TaskState::Running, TaskState::Succeeded],
        );
        *ops.telegram.lock().unwrap() = Some(Ok("777".to_string()));
        install_runtime(&ops, &agent_id);

        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        let record = ops.record(&agent_id);
        assert_eq!(
            (record.state, record.step),
            (AgentInstallState::Provisioning, AgentCreateStep::Bind)
        );
        assert_eq!(record.runtime_task_id, None);
        assert_eq!(record.template_version, "0.7.0");
        assert_eq!(
            record.template_app_doc_object_id,
            built("0.7.0").app_doc_object_id
        );

        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        let record = ops.record(&agent_id);
        assert_eq!(
            (record.state, record.step),
            (AgentInstallState::Bound, AgentCreateStep::Tunnel)
        );
        assert!(record.pending_spec.is_none());
        let spec: AgentSpec = ops.kv.read(&agent_spec_key(OWNER, &agent_id));
        spec.validate().unwrap();
        assert_eq!(
            spec.binding.target_app_instance_id.to_string(),
            "xiaobai.test.buckyos.io@devtest"
        );
        assert_eq!(
            spec.agent_doc.owner.to_string(),
            "did:web:devtest.test.buckyos.io"
        );

        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        let record = ops.record(&agent_id);
        assert_eq!(record.tunnel_state, AgentTunnelState::Bound);
        let settings: AgentSettings = ops.kv.read(&agent_settings_key(OWNER, &agent_id));
        assert_eq!(
            settings.msg_tunnels[0].bot_account_id.as_deref(),
            Some("777")
        );

        report_loaded(&ops, &agent_id);
        ops.drive(&agent_id).await;
        let record = ops.record(&agent_id);
        assert_eq!(
            (record.state, record.step),
            (AgentInstallState::Ready, AgentCreateStep::Done)
        );
        assert_eq!(
            ops.calls(),
            vec![
                "submit_runtime",
                "refresh_rbac",
                "verify_telegram 123:abc",
                "reload_msg_center",
            ]
        );
        assert!(!needs_drive(&record));
    }

    #[tokio::test]
    async fn creation_resumes_a_running_runtime_task_without_resubmitting() {
        let ops = FakeOps::new();
        let agent_id = reserve(&ops, "xiaobai", None).await;
        let key = agent_install_record_key(OWNER, &agent_id);
        let mut record = ops.record(&agent_id);
        record.runtime_task_id = Some("t-install".to_string());
        ops.kv.put(&key, &record);
        ops.task(
            "t-install",
            TaskKind::Install,
            vec![TaskState::Running, TaskState::Running, TaskState::Succeeded],
        );
        install_runtime(&ops, &agent_id);
        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        let record = ops.record(&agent_id);
        assert_eq!(
            (record.state, record.step),
            (AgentInstallState::Bound, AgentCreateStep::Start)
        );
        assert_eq!(ops.calls(), vec!["refresh_rbac"]);
        assert!(needs_drive(&record));

        report_loaded(&ops, &agent_id);
        ops.drive(&agent_id).await;
        assert_eq!(ops.record(&agent_id).state, AgentInstallState::Ready);
        assert_eq!(ops.calls(), vec!["refresh_rbac"]);
    }

    #[tokio::test]
    async fn a_finished_runtime_task_is_resubmitted_for_its_replay_outcome() {
        let ops = FakeOps::new();
        let agent_id = reserve(&ops, "xiaobai", None).await;
        let key = agent_install_record_key(OWNER, &agent_id);
        let mut record = ops.record(&agent_id);
        record.runtime_task_id = Some("t-old".to_string());
        ops.kv.put(&key, &record);
        ops.task(
            "t-old",
            TaskKind::Install,
            vec![TaskState::Failed(AgentFailure::new("x", "paused", true))],
        );
        ops.submits
            .lock()
            .unwrap()
            .push_back(Ok(RuntimeSubmit::Task("t-retry".to_string())));
        ops.task("t-retry", TaskKind::Install, vec![TaskState::Succeeded]);
        install_runtime(&ops, &agent_id);
        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        let record = ops.record(&agent_id);
        assert_eq!(record.step, AgentCreateStep::Bind);
        assert_eq!(record.runtime_task_id, None);
        assert_eq!(ops.calls(), vec!["submit_runtime"]);
    }

    #[tokio::test]
    async fn a_stalled_runtime_task_is_resumed_instead_of_resubmitted() {
        let ops = FakeOps::new();
        let agent_id = reserve(&ops, "xiaobai", None).await;
        let key = agent_install_record_key(OWNER, &agent_id);
        let mut record = ops.record(&agent_id);
        record.runtime_task_id = Some("t-paused".to_string());
        ops.kv.put(&key, &record);
        ops.task(
            "t-paused",
            TaskKind::Install,
            vec![
                TaskState::Stalled(AgentFailure::new("x", "waiting for trust resolution", true)),
                TaskState::Running,
                TaskState::Succeeded,
            ],
        );
        install_runtime(&ops, &agent_id);
        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        let record = ops.record(&agent_id);
        assert_eq!(record.step, AgentCreateStep::Bind);
        assert_eq!(ops.calls(), vec!["resume_runtime t-paused"]);
    }

    #[tokio::test]
    async fn failed_steps_keep_the_name_and_resume_on_retry() {
        let mut ops = FakeOps::new();
        ops.start_timeout = 0;
        let agent_id = reserve(&ops, "xiaobai", Some("123:abc")).await;
        ops.submits.lock().unwrap().push_back(Err(AgentFailure::new(
            "runtime_install_failed",
            "boom",
            true,
        )));
        ops.drive(&agent_id).await;
        let mut record = ops.record(&agent_id);
        assert_eq!(record.state, AgentInstallState::Failed);
        assert_eq!(
            record.last_error.as_ref().unwrap().step,
            AgentCreateStep::Runtime
        );
        assert!(cancel_allowed(&record));
        assert!(ops.kv.has(&reservation_key(&agent_id)));

        assert!(prepare_retry(&mut record, None, false, 1)
            .unwrap()
            .is_none());
        assert_eq!(
            (record.state, record.step),
            (AgentInstallState::Provisioning, AgentCreateStep::Runtime)
        );
        let key = agent_install_record_key(OWNER, &agent_id);
        ops.kv.put(&key, &record);
        ops.submits
            .lock()
            .unwrap()
            .push_back(Ok(RuntimeSubmit::Satisfied(built("0.7.0"))));
        ops.drive(&agent_id).await;
        let record = ops.record(&agent_id);
        assert_eq!(
            record.last_error.as_ref().unwrap().step,
            AgentCreateStep::Bind
        );
        assert_eq!(record.last_error.as_ref().unwrap().code, "runtime_missing");

        let mut record = record;
        prepare_retry(&mut record, None, false, 1).unwrap();
        ops.kv.put(&key, &record);
        install_runtime(&ops, &agent_id);
        *ops.telegram.lock().unwrap() = Some(Err(AgentFailure::new(
            "telegram_bot_invalid",
            "Unauthorized",
            true,
        )));
        ops.drive(&agent_id).await;
        let mut record = ops.record(&agent_id);
        assert_eq!(record.state, AgentInstallState::Failed);
        assert_eq!(record.tunnel_state, AgentTunnelState::Failed);
        assert_eq!(
            record.last_error.as_ref().unwrap().step,
            AgentCreateStep::Tunnel
        );
        assert!(!cancel_allowed(&record));
        assert!(ops.kv.has(&agent_spec_key(OWNER, &agent_id)));

        let settings = ops.kv.read(&agent_settings_key(OWNER, &agent_id));
        let settings = prepare_retry(&mut record, Some(settings), true, 1)
            .unwrap()
            .unwrap();
        assert!(settings.msg_tunnels.is_empty());
        assert_eq!(
            (record.state, record.step, record.tunnel_state),
            (
                AgentInstallState::Bound,
                AgentCreateStep::Start,
                AgentTunnelState::Skipped
            )
        );
        ops.kv.put(&key, &record);
        ops.drive(&agent_id).await;
        let mut record = ops.record(&agent_id);
        assert_eq!(record.last_error.as_ref().unwrap().code, "start_timeout");
        assert!(record.last_error.as_ref().unwrap().retryable);

        prepare_retry(&mut record, None, false, buckyos_get_unix_timestamp()).unwrap();
        ops.kv.put(&key, &record);
        report_loaded(&ops, &agent_id);
        ops.drive(&agent_id).await;
        let record = ops.record(&agent_id);
        assert_eq!(record.state, AgentInstallState::Ready);
        assert!(prepare_retry(&mut record.clone(), None, false, 1).is_err());
    }

    #[tokio::test]
    async fn removal_drops_identity_then_refreshes_reloads_uninstalls_and_releases_name() {
        let ops = FakeOps::new();
        let agent_id = reserve(&ops, "xiaobai", None).await;
        ops.submits
            .lock()
            .unwrap()
            .push_back(Ok(RuntimeSubmit::Satisfied(built("0.7.0"))));
        install_runtime(&ops, &agent_id);
        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        assert!(drive_agent_step(&ops, OWNER, &agent_id).await.unwrap());
        report_loaded(&ops, &agent_id);
        ops.drive(&agent_id).await;
        assert_eq!(ops.record(&agent_id).state, AgentInstallState::Ready);
        ops.calls.lock().unwrap().clear();

        begin_agent_removal(&ops.kv, OWNER, &agent_id, 2)
            .await
            .unwrap();
        for key in [
            agent_spec_key(OWNER, &agent_id),
            agent_key_path(OWNER, &agent_id),
            agent_settings_key(OWNER, &agent_id),
            agent_profile_key(OWNER, &agent_id),
            agent_info_key(OWNER, &agent_id),
        ] {
            assert!(!ops.kv.has(&key), "{key} survived removal");
        }
        assert_eq!(ops.record(&agent_id).state, AgentInstallState::Removed);
        assert!(ops.kv.has(&reservation_key(&agent_id)));
        begin_agent_removal(&ops.kv, OWNER, &agent_id, 3)
            .await
            .unwrap();

        ops.uninstalls
            .lock()
            .unwrap()
            .push_back(Some("t-uninstall".to_string()));
        ops.task(
            "t-uninstall",
            TaskKind::Uninstall,
            vec![TaskState::Running, TaskState::Succeeded],
        );
        ops.drive(&agent_id).await;
        assert_eq!(
            ops.calls(),
            vec![
                "refresh_rbac",
                "reload_msg_center",
                "start_uninstall spec_present=false after=-",
            ]
        );
        assert!(!ops.kv.has(&agent_install_record_key(OWNER, &agent_id)));
        assert!(!ops.kv.has(&reservation_key(&agent_id)));
        let index = AgentNameIndex::load(&ops.kv, &zone()).await.unwrap();
        assert!(index.check("xiaobai").is_ok());
    }

    #[tokio::test]
    async fn cancel_waits_for_the_runtime_install_and_retries_failed_uninstall() {
        let ops = FakeOps::new();
        let agent_id = reserve(&ops, "xiaobai", None).await;
        let key = agent_install_record_key(OWNER, &agent_id);
        let mut record = ops.record(&agent_id);
        record.runtime_task_id = Some("t-install".to_string());
        ops.kv.put(&key, &record);
        ops.task(
            "t-install",
            TaskKind::Install,
            vec![TaskState::Running, TaskState::Succeeded],
        );
        begin_agent_removal(&ops.kv, OWNER, &agent_id, 2)
            .await
            .unwrap();
        ops.uninstalls.lock().unwrap().extend([
            Some("t-uninstall".to_string()),
            Some("t-uninstall-2".to_string()),
        ]);
        ops.task(
            "t-uninstall",
            TaskKind::Uninstall,
            vec![TaskState::Failed(AgentFailure::new("x", "busy", true))],
        );
        ops.task(
            "t-uninstall-2",
            TaskKind::Uninstall,
            vec![TaskState::Running, TaskState::Succeeded],
        );
        ops.drive(&agent_id).await;
        let record = ops.record(&agent_id);
        assert_eq!(record.state, AgentInstallState::Removed);
        assert_eq!(record.runtime_task_id.as_deref(), Some("t-uninstall-2"));
        ops.drive(&agent_id).await;
        assert!(!ops.kv.has(&key));
        let uninstall_calls = ops
            .calls()
            .into_iter()
            .filter(|call| call.starts_with("start_uninstall"))
            .collect::<Vec<_>>();
        assert_eq!(
            uninstall_calls,
            vec![
                "start_uninstall spec_present=false after=-",
                "start_uninstall spec_present=false after=t-uninstall",
            ]
        );
    }

    #[tokio::test]
    async fn template_update_task_moves_the_template_version_when_done() {
        let ops = FakeOps::new();
        let agent_id = reserve(&ops, "xiaobai", None).await;
        let key = agent_install_record_key(OWNER, &agent_id);
        let mut record = ops.record(&agent_id);
        record.state = AgentInstallState::Ready;
        record.step = AgentCreateStep::Done;
        record.runtime_task_id = Some("t-update".to_string());
        ops.kv.put(&key, &record);
        let next = built("0.7.10");
        let app_id = AppId::parse(agent_id.as_str()).unwrap();
        ops.kv.put(
            &user_app_spec_key(OWNER, &app_id),
            &runtime_spec(&agent_id, &next.app_doc_object_id, "0.7.10"),
        );
        ops.task(
            "t-update",
            TaskKind::Install,
            vec![TaskState::Running, TaskState::Succeeded],
        );
        assert!(needs_drive(&record));
        ops.drive(&agent_id).await;
        let record = ops.record(&agent_id);
        assert_eq!(record.runtime_task_id, None);
        assert_eq!(record.template_app_doc_object_id, next.app_doc_object_id);
        assert_eq!(record.template_version, "0.7.10");
        assert_eq!(record.state, AgentInstallState::Ready);
    }

    #[test]
    fn template_ids_round_trip() {
        let (source, app_id) = parse_template_id("bundled:jarvis.buckyos.bns.did").unwrap();
        assert_eq!(source, AgentTemplateSource::Bundled);
        assert_eq!(
            template_id_of(source, &app_id),
            "bundled:jarvis.buckyos.bns.did"
        );
        assert!(parse_template_id("installed:jarvis.buckyos.bns.did").is_ok());
        assert!(parse_template_id("market:jarvis.buckyos.bns.did").is_err());
        assert!(parse_template_id("jarvis").is_err());
    }
}
