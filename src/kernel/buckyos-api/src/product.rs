//! Product edition of a BuckyOS installation.
//!
//! The release package decides the edition: `make_config.ts --edition` writes
//! `$BUCKYOS_ROOT/etc/product.json` from the version-managed manifest in
//! `src/product/`. node_daemon (activation, AppLoader) reads that file;
//! control-panel publishes it as `system/product` on every start, before it
//! serves anything, so control-panel and Desktop see the same profile. An
//! installation without the file is the full edition. The scheduler knows no
//! product: it places by what an App needs and what a node offers.
//!
//! BuckyOS Desktop Lite (`notepads/buckyos-desktop-lite-todo.md`) runs without
//! any container runtime: system services, the static web apps users add, the
//! script host apps listed in the product manifest (run natively) and at most
//! one Agent in the whole system. [`ProductProfile::admit_app`] is the single
//! admission rule control-panel and node_daemon both apply.

use crate::{AppDoc, AppType, SelectorType};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::path::Path;

/// `$BUCKYOS_ROOT`-relative location of the installed product profile.
pub const PRODUCT_PROFILE_PATH: &str = "etc/product.json";
/// system-config copy of the installed product profile.
pub const PRODUCT_PROFILE_KEY: &str = "system/product";
/// The run slot of the one Agent a product with `max_running_agents = 1` may
/// run ([`AgentSlot`]). Created together with the Agent's identity
/// reservation, removed only after the Agent's runtime is gone.
pub const AGENT_SLOT_KEY: &str = "system/agent_slot";
pub const PRODUCT_PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProductEdition {
    #[default]
    Full,
    Lite,
}

impl fmt::Display for ProductEdition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ProductEdition::Full => "full",
            ProductEdition::Lite => "lite",
        })
    }
}

/// What the product lets an installation run. Kept apart from the device's
/// `support_container`: that only says whether this node can run containers.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductCapabilities {
    /// Container runtimes (Docker, wslc) are part of the product: paios
    /// images, worker containers, docker apps, the Agent `container` runtime.
    pub container: bool,
    /// Users may add apps that run a service process (script, native binary
    /// or container) beyond the product's preinstalled script apps.
    pub user_service_apps: bool,
    /// Users may install, update and remove static web apps.
    pub static_web_apps: bool,
    /// Agents the whole system may run at once (`None`: no limit).
    pub max_running_agents: Option<u32>,
}

impl ProductCapabilities {
    pub fn full() -> Self {
        Self {
            container: true,
            user_service_apps: true,
            static_web_apps: true,
            max_running_agents: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScriptInterpreter {
    /// The node's private Deno (`libexec/buckyos-tool/runtime/deno`).
    Deno,
}

/// A script host app the product ships and runs natively. Its identity is
/// the AppDID, the publisher (AppDoc owner) and the exact version; a new
/// version comes only with a product release that updates this manifest.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreinstalledScriptApp {
    pub app_did: String,
    pub owner: String,
    pub version: String,
    /// Package name of the `script` sub package (`all.script.<app_id>`).
    pub script_package: String,
    pub interpreter: ScriptInterpreter,
    /// Entry file, relative to the package root.
    pub entry: String,
    /// Interpreter flags (Deno permissions), in order.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// The services the app listens on, each on the instance port the
    /// scheduler assigns in `app_instance_config.service_ports_config`.
    pub services: Vec<String>,
    /// Where updates come from; only `product_release` is defined.
    pub update_source: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductProfile {
    pub schema_version: u32,
    pub edition: ProductEdition,
    pub capabilities: ProductCapabilities,
    #[serde(default)]
    pub preinstalled_script_apps: Vec<PreinstalledScriptApp>,
}

impl Default for ProductProfile {
    fn default() -> Self {
        Self::full()
    }
}

/// How an AppDoc's packages run, by package key. Classification looks at the
/// whole AppDoc, not at the packages one target selects, so a docker or
/// script variant cannot hide behind a web sub package or a target filter.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AppExecutionForm {
    StaticWeb,
    Agent,
    Script,
    NativeBinary,
    Container,
    /// AI Workspace controls (`pkg_list.aiws`): a library entry of the aiworkspace service, never a running App.
    AiwsControls,
    /// A package key the product cannot classify.
    Unknown,
}

pub fn app_execution_forms(app_doc: &AppDoc) -> BTreeSet<AppExecutionForm> {
    app_doc
        .pkg_list
        .iter()
        .into_iter()
        .map(|(key, desc)| {
            if desc.docker_image_name.is_some() || key.ends_with("_docker_image") {
                AppExecutionForm::Container
            } else {
                match key.as_str() {
                    "web" => AppExecutionForm::StaticWeb,
                    "agent" | "agent_skills" | "agent_tools" => AppExecutionForm::Agent,
                    "script" => AppExecutionForm::Script,
                    crate::AIWS_SUB_PKG => AppExecutionForm::AiwsControls,
                    _ if key.ends_with("_app") => AppExecutionForm::NativeBinary,
                    _ => AppExecutionForm::Unknown,
                }
            }
        })
        .collect()
}

/// Who asks to run an app. Submissions carry an origin only control-panel
/// sets; control-panel (install inspection) and node_daemon check what is
/// already deployed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionOrigin {
    /// `apps.submit` / retries / batch updates from a user or another App.
    User,
    /// control-panel installing the product's preinstalled apps.
    Preinstall,
    /// control-panel installing the runtime App it constructs for an Agent.
    AgentRuntime,
    /// A spec or instance that is already deployed (control-panel, node_daemon).
    Deployed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AppAdmission {
    /// The product places no restriction (full edition).
    Unrestricted,
    StaticWeb,
    Agent,
    PreinstalledScript(PreinstalledScriptApp),
    /// An `aiws` package a user submits: it goes into their AI Workspace control library (no AppSpec, nothing runs).
    AiwsLibrary,
}

/// Why the product does not run an app. Callers match on the variant, never
/// on the message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdmissionDenied {
    /// The app needs a container runtime.
    ContainerApp,
    /// A script or native service that is not one of the product's
    /// preinstalled script apps (or one users may not add themselves).
    ServiceApp,
    /// A preinstalled script app at a version the product manifest does not list.
    PreinstalledVersion { expected: String, actual: String },
    /// Packages of different execution forms, or ones the product cannot
    /// classify, in one app.
    MixedPackages(Vec<AppExecutionForm>),
    /// Agent Apps are only constructed by the system for an Agent.
    AgentOrigin,
    StaticWebDisabled,
    /// An `aiws` package is installed by a user into the AI Workspace library, never preinstalled or deployed.
    AiwsOrigin,
}

impl AdmissionDenied {
    /// Stable code for RPC errors and deployment reports.
    pub fn code(&self) -> &'static str {
        match self {
            AdmissionDenied::ContainerApp => "product_container_app",
            AdmissionDenied::ServiceApp => "product_service_app",
            AdmissionDenied::PreinstalledVersion { .. } => "product_preinstalled_version",
            AdmissionDenied::MixedPackages(_) => "product_mixed_packages",
            AdmissionDenied::AgentOrigin => "product_agent_origin",
            AdmissionDenied::StaticWebDisabled => "product_static_web_disabled",
            AdmissionDenied::AiwsOrigin => "product_aiws_origin",
        }
    }
}

impl fmt::Display for AdmissionDenied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AdmissionDenied::ContainerApp => f.write_str(
                "this BuckyOS edition has no container runtime and does not run docker apps",
            ),
            AdmissionDenied::ServiceApp => f.write_str(
                "this BuckyOS edition only runs its preinstalled script apps; add static web apps instead",
            ),
            AdmissionDenied::PreinstalledVersion { expected, actual } => write!(
                f,
                "this BuckyOS edition ships version {expected} of this app, not {actual}; it is updated with the product"
            ),
            AdmissionDenied::MixedPackages(forms) => write!(
                f,
                "this BuckyOS edition does not run apps that mix or carry unknown package forms ({forms:?})"
            ),
            AdmissionDenied::AgentOrigin => {
                f.write_str("Agent Apps are only created by the system for an Agent")
            }
            AdmissionDenied::StaticWebDisabled => {
                f.write_str("this BuckyOS edition does not allow adding web apps")
            }
            AdmissionDenied::AiwsOrigin => f.write_str(
                "AI Workspace control packages are installed by a user into the AI Workspace library, not deployed as Apps",
            ),
        }
    }
}

impl std::error::Error for AdmissionDenied {}

impl ProductProfile {
    pub fn full() -> Self {
        Self {
            schema_version: PRODUCT_PROFILE_SCHEMA_VERSION,
            edition: ProductEdition::Full,
            capabilities: ProductCapabilities::full(),
            preinstalled_script_apps: Vec::new(),
        }
    }

    pub fn is_lite(&self) -> bool {
        self.edition == ProductEdition::Lite
    }

    pub fn container_enabled(&self) -> bool {
        self.capabilities.container
    }

    /// A profile must say exactly what its edition means: the full edition
    /// is unrestricted, Lite runs no containers, no user service apps and a
    /// bounded number of Agents.
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != PRODUCT_PROFILE_SCHEMA_VERSION {
            return Err(format!(
                "unsupported product profile schema_version {}",
                self.schema_version
            ));
        }
        match self.edition {
            ProductEdition::Full => {
                if self.capabilities != ProductCapabilities::full() {
                    return Err("the full edition must not restrict capabilities".to_string());
                }
                if !self.preinstalled_script_apps.is_empty() {
                    return Err("the full edition has no preinstalled script app manifest".to_string());
                }
            }
            ProductEdition::Lite => {
                let caps = &self.capabilities;
                if caps.container || caps.user_service_apps {
                    return Err("Lite runs no containers and no user service apps".to_string());
                }
                // One run slot (`AGENT_SLOT_KEY`) is all the system implements.
                if caps.max_running_agents != Some(1) {
                    return Err("Lite runs exactly one Agent (max_running_agents = 1)".to_string());
                }
            }
        }
        let mut seen = BTreeSet::new();
        for app in &self.preinstalled_script_apps {
            if !seen.insert(app.app_did.as_str()) {
                return Err(format!("preinstalled script app {} is listed twice", app.app_did));
            }
            if [&app.app_did, &app.owner, &app.version, &app.script_package, &app.entry]
                .iter()
                .any(|value| value.trim().is_empty())
            {
                return Err(format!("preinstalled script app {} is incomplete", app.app_did));
            }
            if app.services.is_empty() {
                return Err(format!("preinstalled script app {} declares no service", app.app_did));
            }
            if app.update_source != "product_release" {
                return Err(format!(
                    "preinstalled script app {} has unknown update_source {}",
                    app.app_did, app.update_source
                ));
            }
            let entry = Path::new(&app.entry);
            if entry.is_absolute()
                || entry
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err(format!(
                    "preinstalled script app {} entry must be a relative path inside the package",
                    app.app_did
                ));
            }
        }
        Ok(())
    }

    pub fn parse(raw: &str) -> Result<Self, String> {
        let profile: ProductProfile =
            serde_json::from_str(raw).map_err(|error| format!("invalid product profile: {error}"))?;
        profile.validate()?;
        Ok(profile)
    }

    /// The installed profile under `buckyos_root`; the full edition when the
    /// installation has none. A present but invalid file is an error, never
    /// a silent fallback to the full edition.
    pub fn load_installed(buckyos_root: &Path) -> Result<Self, String> {
        let path = buckyos_root.join(PRODUCT_PROFILE_PATH);
        match std::fs::read_to_string(&path) {
            Ok(raw) => Self::parse(&raw).map_err(|error| format!("{}: {error}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::full()),
            Err(error) => Err(format!("read {}: {error}", path.display())),
        }
    }

    /// The profile in a system-config snapshot (`system/product`); the full
    /// edition when the key is absent.
    pub fn from_system_config(input: &HashMap<String, String>) -> Result<Self, String> {
        match input.get(PRODUCT_PROFILE_KEY) {
            Some(raw) => Self::parse(raw),
            None => Ok(Self::full()),
        }
    }

    pub fn preinstalled_script_app(&self, app_did: &str) -> Option<&PreinstalledScriptApp> {
        self.preinstalled_script_apps
            .iter()
            .find(|app| app.app_did == app_did)
    }

    /// Whether the product runs `app_doc` for `origin`. Agent Apps are also
    /// bound by the Agent run slot, which callers check separately.
    pub fn admit_app(
        &self,
        app_doc: &AppDoc,
        origin: AdmissionOrigin,
    ) -> Result<AppAdmission, AdmissionDenied> {
        let caps = &self.capabilities;
        let forms = app_execution_forms(app_doc);
        // controls are a library entry in every edition: only a user's submission adds one, nothing deploys it
        if forms.contains(&AppExecutionForm::AiwsControls) || app_doc.app_type == AppType::Aiws {
            if forms.len() != 1 || app_doc.app_type != AppType::Aiws {
                return Err(AdmissionDenied::MixedPackages(forms.into_iter().collect()));
            }
            return match origin {
                AdmissionOrigin::User => Ok(AppAdmission::AiwsLibrary),
                _ => Err(AdmissionDenied::AiwsOrigin),
            };
        }
        if caps == &ProductCapabilities::full() {
            return Ok(AppAdmission::Unrestricted);
        }
        if forms.contains(&AppExecutionForm::Unknown) {
            return Err(AdmissionDenied::MixedPackages(forms.into_iter().collect()));
        }
        if forms.contains(&AppExecutionForm::Agent) {
            if forms.len() != 1 || app_doc.app_type != AppType::Agent {
                return Err(AdmissionDenied::MixedPackages(forms.into_iter().collect()));
            }
            return match origin {
                AdmissionOrigin::AgentRuntime | AdmissionOrigin::Deployed => Ok(AppAdmission::Agent),
                AdmissionOrigin::User | AdmissionOrigin::Preinstall => {
                    Err(AdmissionDenied::AgentOrigin)
                }
            };
        }
        if forms.len() == 1 && forms.contains(&AppExecutionForm::StaticWeb) {
            if app_doc.app_type != AppType::Web
                || app_doc.selector_type != SelectorType::Static
                || !app_doc.service_config_tips.service_endpoints.is_empty()
            {
                return Err(AdmissionDenied::MixedPackages(forms.into_iter().collect()));
            }
            return if caps.static_web_apps {
                Ok(AppAdmission::StaticWeb)
            } else {
                Err(AdmissionDenied::StaticWebDisabled)
            };
        }
        if forms.contains(&AppExecutionForm::Container) && !caps.container {
            return Err(AdmissionDenied::ContainerApp);
        }
        if caps.user_service_apps {
            return Ok(AppAdmission::Unrestricted);
        }
        let Some(manifest) = self.preinstalled_script_app(&app_doc.app_did().to_string()) else {
            return Err(AdmissionDenied::ServiceApp);
        };
        if forms.len() != 1 || !forms.contains(&AppExecutionForm::Script) {
            return Err(AdmissionDenied::MixedPackages(forms.into_iter().collect()));
        }
        if app_doc.owner.to_string() != manifest.owner {
            return Err(AdmissionDenied::ServiceApp);
        }
        let script_name = app_doc
            .pkg_list
            .script
            .as_ref()
            .map(|desc| package_lib::PackageId::get_pkg_id_unique_name(&desc.pkg_id))
            .unwrap_or_default();
        if script_name != manifest.script_package {
            return Err(AdmissionDenied::ServiceApp);
        }
        if app_doc.version != manifest.version {
            return Err(AdmissionDenied::PreinstalledVersion {
                expected: manifest.version.clone(),
                actual: app_doc.version.clone(),
            });
        }
        match origin {
            AdmissionOrigin::Preinstall | AdmissionOrigin::Deployed => {
                Ok(AppAdmission::PreinstalledScript(manifest.clone()))
            }
            AdmissionOrigin::User | AdmissionOrigin::AgentRuntime => Err(AdmissionDenied::ServiceApp),
        }
    }
}

/// Holder of the one Agent run slot ([`AGENT_SLOT_KEY`]).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSlot {
    pub owner_user_id: String,
    pub agent_id: String,
    pub acquired_at: u64,
}

impl AgentSlot {
    pub fn from_system_config(input: &HashMap<String, String>) -> Option<Self> {
        input
            .get(AGENT_SLOT_KEY)
            .and_then(|raw| serde_json::from_str(raw).ok())
    }

    pub fn holds(&self, owner_user_id: &str, agent_id: &str) -> bool {
        self.owner_user_id == owner_user_id && self.agent_id == agent_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lite() -> ProductProfile {
        ProductProfile::parse(include_str!("../../../product/lite.json")).unwrap()
    }

    fn pkg(name: &str, version: &str) -> serde_json::Value {
        json!({
            "pkg_id": format!("{name}#{version}"),
            "pkg_objid": "pkg:d0e3ae4d030ef2ad25d0c9b37af6b095e04cd389ef4c0226f41a00ca5450da9b",
        })
    }

    fn app_doc(app: &str, app_type: &str, version: &str, pkg_list: serde_json::Value) -> AppDoc {
        let mut doc = json!({
            "schema_version": 1,
            "doc_type": "app",
            "did": format!("did:bns:{app}.buckyos"),
            "name": app,
            "version": version,
            "app_type": app_type,
            "author": "did:bns:buckyos",
            "owner": "did:bns:buckyos",
            "controller": "did:bns:buckyos",
            "create_time": 1,
            "last_update_time": 1,
            "exp": 2,
            "show_name": app,
            "selector_type": if app_type == "web" { "static" } else { "single" },
            "pkg_list": pkg_list,
        });
        doc["service_config_tips"] = json!({});
        if app_type != "web" {
            doc["service_config_tips"] = json!({
                "service_endpoints": {
                    "www": { "protocol": "http", "inner_port": 3000, "required": true }
                }
            });
        }
        serde_json::from_value(doc).unwrap()
    }

    fn systest(version: &str) -> AppDoc {
        app_doc(
            "buckyos-systest",
            "dapp",
            version,
            json!({ "script": pkg("all.script.buckyos-systest.buckyos.bns.did", version) }),
        )
    }

    #[test]
    fn the_shipped_lite_manifest_is_a_valid_lite_profile() {
        let profile = lite();
        assert!(profile.is_lite());
        assert!(!profile.container_enabled());
        assert_eq!(profile.capabilities.max_running_agents, Some(1));
        assert!(profile
            .preinstalled_script_app("did:bns:buckyos-systest.buckyos")
            .is_some());
        assert_eq!(
            ProductProfile::parse(include_str!("../../../product/full.json")).unwrap(),
            ProductProfile::full()
        );
    }

    #[test]
    fn a_profile_must_mean_what_its_edition_says() {
        let mut half_lite = lite();
        half_lite.capabilities.container = true;
        assert!(half_lite.validate().is_err());
        let mut unbounded = lite();
        unbounded.capabilities.max_running_agents = None;
        assert!(unbounded.validate().is_err());
        let mut restricted_full = ProductProfile::full();
        restricted_full.capabilities.static_web_apps = false;
        assert!(restricted_full.validate().is_err());
        let mut escaping = lite();
        escaping.preinstalled_script_apps[0].entry = "../main.ts".to_string();
        assert!(escaping.validate().is_err());
        assert!(ProductProfile::parse(r#"{"schema_version":1,"edition":"lite"}"#).is_err());
    }

    #[test]
    fn a_missing_installed_profile_is_the_full_edition_but_a_broken_one_is_an_error() {
        let root = std::env::temp_dir().join(format!("product-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(ProductProfile::load_installed(&root).unwrap(), ProductProfile::full());
        std::fs::create_dir_all(root.join("etc")).unwrap();
        std::fs::write(root.join(PRODUCT_PROFILE_PATH), r#"{"edition":"lite"}"#).unwrap();
        assert!(ProductProfile::load_installed(&root).is_err());
        std::fs::write(root.join(PRODUCT_PROFILE_PATH), include_str!("../../../product/lite.json"))
            .unwrap();
        assert!(ProductProfile::load_installed(&root).unwrap().is_lite());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_full_edition_admits_everything() {
        let full = ProductProfile::full();
        let docker = app_doc(
            "filebrowser",
            "dapp",
            "1.0.0",
            json!({ "amd64_docker_image": {
                "pkg_id": "all.amd64_docker_image.filebrowser.buckyos.bns.did#1.0.0",
                "pkg_objid": "pkg:d0e3ae4d030ef2ad25d0c9b37af6b095e04cd389ef4c0226f41a00ca5450da9b",
                "docker_image_name": "filebrowser:1"
            }}),
        );
        assert_eq!(
            full.admit_app(&docker, AdmissionOrigin::User),
            Ok(AppAdmission::Unrestricted)
        );
    }

    #[test]
    fn aiws_packages_only_enter_the_library_from_a_user() {
        let controls = app_doc(
            "kpi-pack",
            "aiws",
            "0.1.0",
            json!({ "aiws": {
                "pkg_id": "all.aiws.kpi-pack.buckyos.bns.did#0.1.0",
                "pkg_objid": "pkg:d0e3ae4d030ef2ad25d0c9b37af6b095e04cd389ef4c0226f41a00ca5450da9b"
            }}),
        );
        for profile in [ProductProfile::full(), lite()] {
            assert_eq!(profile.admit_app(&controls, AdmissionOrigin::User), Ok(AppAdmission::AiwsLibrary));
            for origin in [AdmissionOrigin::Preinstall, AdmissionOrigin::AgentRuntime, AdmissionOrigin::Deployed] {
                assert_eq!(profile.admit_app(&controls, origin), Err(AdmissionDenied::AiwsOrigin));
            }
        }
    }

    #[test]
    fn lite_admits_static_web_and_rejects_docker_and_user_scripts() {
        let profile = lite();
        let web = app_doc(
            "notes",
            "web",
            "1.0.0",
            json!({ "web": pkg("all.web.notes.buckyos.bns.did", "1.0.0") }),
        );
        assert_eq!(profile.admit_app(&web, AdmissionOrigin::User), Ok(AppAdmission::StaticWeb));

        let docker = app_doc(
            "filebrowser",
            "dapp",
            "1.0.0",
            json!({ "amd64_docker_image": {
                "pkg_id": "all.amd64_docker_image.filebrowser.buckyos.bns.did#1.0.0",
                "pkg_objid": "pkg:d0e3ae4d030ef2ad25d0c9b37af6b095e04cd389ef4c0226f41a00ca5450da9b",
                "docker_image_name": "filebrowser:1"
            }}),
        );
        assert_eq!(
            profile.admit_app(&docker, AdmissionOrigin::Preinstall),
            Err(AdmissionDenied::ContainerApp)
        );
        let script = app_doc(
            "my-tool",
            "dapp",
            "1.0.0",
            json!({ "script": pkg("all.script.my-tool.buckyos.bns.did", "1.0.0") }),
        );
        assert_eq!(
            profile.admit_app(&script, AdmissionOrigin::User),
            Err(AdmissionDenied::ServiceApp)
        );
        assert_eq!(
            profile.admit_app(&script, AdmissionOrigin::Deployed),
            Err(AdmissionDenied::ServiceApp)
        );
    }

    #[test]
    fn a_web_sub_package_does_not_hide_a_service_or_container_variant() {
        let profile = lite();
        // A "web" app that also carries a script and a docker variant.
        let mixed = app_doc(
            "sneaky",
            "web",
            "1.0.0",
            json!({
                "web": pkg("all.web.sneaky.buckyos.bns.did", "1.0.0"),
                "script": pkg("all.script.sneaky.buckyos.bns.did", "1.0.0"),
            }),
        );
        assert!(matches!(
            profile.admit_app(&mixed, AdmissionOrigin::User),
            Err(AdmissionDenied::ServiceApp | AdmissionDenied::MixedPackages(_))
        ));
        // A web package declared under a dapp with service endpoints.
        let serviced_web = app_doc(
            "serviced",
            "dapp",
            "1.0.0",
            json!({ "web": pkg("all.web.serviced.buckyos.bns.did", "1.0.0") }),
        );
        assert!(matches!(
            profile.admit_app(&serviced_web, AdmissionOrigin::User),
            Err(AdmissionDenied::MixedPackages(_))
        ));
        // The preinstalled script app cannot gain a docker variant.
        let mut grown = systest(&lite().preinstalled_script_apps[0].version);
        grown.pkg_list.amd64_docker_image = Some(
            serde_json::from_value(json!({
                "pkg_id": "all.amd64_docker_image.buckyos-systest.buckyos.bns.did#0.5.1",
                "pkg_objid": "pkg:d0e3ae4d030ef2ad25d0c9b37af6b095e04cd389ef4c0226f41a00ca5450da9b",
                "docker_image_name": "systest:1"
            }))
            .unwrap(),
        );
        assert_eq!(
            profile.admit_app(&grown, AdmissionOrigin::Preinstall),
            Err(AdmissionDenied::ContainerApp)
        );
        let unknown = app_doc(
            "odd",
            "web",
            "1.0.0",
            json!({
                "web": pkg("all.web.odd.buckyos.bns.did", "1.0.0"),
                "plugin": pkg("all.plugin.odd.buckyos.bns.did", "1.0.0"),
            }),
        );
        assert!(matches!(
            profile.admit_app(&unknown, AdmissionOrigin::User),
            Err(AdmissionDenied::MixedPackages(_))
        ));
    }

    #[test]
    fn lite_runs_only_the_listed_version_of_a_preinstalled_script_app_and_only_from_the_product() {
        let profile = lite();
        let version = profile.preinstalled_script_apps[0].version.clone();
        let current = systest(&version);
        assert!(matches!(
            profile.admit_app(&current, AdmissionOrigin::Preinstall),
            Ok(AppAdmission::PreinstalledScript(_))
        ));
        assert!(matches!(
            profile.admit_app(&current, AdmissionOrigin::Deployed),
            Ok(AppAdmission::PreinstalledScript(_))
        ));
        assert_eq!(
            profile.admit_app(&current, AdmissionOrigin::User),
            Err(AdmissionDenied::ServiceApp)
        );
        let newer = systest("9.9.9");
        assert!(matches!(
            profile.admit_app(&newer, AdmissionOrigin::Preinstall),
            Err(AdmissionDenied::PreinstalledVersion { .. })
        ));
        let mut impostor = systest(&version);
        impostor.owner = name_lib::DID::from_str("did:bns:mallory").unwrap();
        assert_eq!(
            profile.admit_app(&impostor, AdmissionOrigin::Preinstall),
            Err(AdmissionDenied::ServiceApp)
        );
    }

    #[test]
    fn agent_apps_run_only_when_the_system_constructs_them() {
        let profile = lite();
        let agent = app_doc(
            "jarvis",
            "agent",
            "0.7.0",
            json!({ "agent": pkg("all.agent.jarvis.buckyos.bns.did", "0.7.0") }),
        );
        assert_eq!(
            profile.admit_app(&agent, AdmissionOrigin::AgentRuntime),
            Ok(AppAdmission::Agent)
        );
        assert_eq!(
            profile.admit_app(&agent, AdmissionOrigin::User),
            Err(AdmissionDenied::AgentOrigin)
        );
        let agent_with_script = app_doc(
            "jarvis",
            "agent",
            "0.7.0",
            json!({
                "agent": pkg("all.agent.jarvis.buckyos.bns.did", "0.7.0"),
                "script": pkg("all.script.jarvis.buckyos.bns.did", "0.7.0"),
            }),
        );
        assert!(matches!(
            profile.admit_app(&agent_with_script, AdmissionOrigin::AgentRuntime),
            Err(AdmissionDenied::MixedPackages(_))
        ));
    }

    #[test]
    fn the_agent_slot_names_one_agent() {
        let slot = AgentSlot {
            owner_user_id: "devtest".to_string(),
            agent_id: "jarvis.test.buckyos.io".to_string(),
            acquired_at: 1,
        };
        let input = HashMap::from([(
            AGENT_SLOT_KEY.to_string(),
            serde_json::to_string(&slot).unwrap(),
        )]);
        let read = AgentSlot::from_system_config(&input).unwrap();
        assert!(read.holds("devtest", "jarvis.test.buckyos.io"));
        assert!(!read.holds("devtest", "other.test.buckyos.io"));
    }
}
