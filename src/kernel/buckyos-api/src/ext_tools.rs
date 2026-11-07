//! Extension tool packages (`app_type = ext_tools`, doc/pikg/format.md §5): tools every Agent of a machine shares.
//! A package is a PIKG with one payload per platform, each a tree for `$BUCKYOS_ROOT/tools` and an `ext-manifest.json`
//! at its root. It is never an App: no AppSpec, no service; a node's reconciler installs it from the Zone's desired state.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// `pkg_list` is a map, so each platform's payload has its own key: `ext_tools_<os>_<arch>`.
pub const EXT_TOOLS_SUB_PKG_PREFIX: &str = "ext_tools_";
/// At the root of every payload.
pub const EXT_TOOLS_MANIFEST_FILE: &str = "ext-manifest.json";
pub const EXT_TOOLS_MANIFEST_SCHEMA: &str = "buckyos.ext_tools/1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtToolsPlatform {
    /// The name `ext-manifest.json` and the payload directories use.
    pub name: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
    pub key: &'static str,
}

const fn platform(name: &'static str, os: &'static str, arch: &'static str, key: &'static str) -> ExtToolsPlatform {
    ExtToolsPlatform { name, os, arch, key }
}

pub const EXT_TOOLS_PLATFORMS: [ExtToolsPlatform; 5] = [
    platform("linux-x86_64", "linux", "x86_64", "ext_tools_linux_x86_64"),
    platform("linux-aarch64", "linux", "aarch64", "ext_tools_linux_aarch64"),
    platform("darwin-arm64", "macos", "aarch64", "ext_tools_darwin_arm64"),
    platform("darwin-x86_64", "macos", "x86_64", "ext_tools_darwin_x86_64"),
    platform("windows-x86_64", "windows", "x86_64", "ext_tools_windows_x86_64"),
];

pub fn ext_tools_platform(name: &str) -> Option<&'static ExtToolsPlatform> {
    EXT_TOOLS_PLATFORMS.iter().find(|p| p.name == name)
}

pub fn ext_tools_platform_of_key(key: &str) -> Option<&'static ExtToolsPlatform> {
    EXT_TOOLS_PLATFORMS.iter().find(|p| p.key == key)
}

pub fn is_ext_tools_sub_pkg(key: &str) -> bool {
    key.starts_with(EXT_TOOLS_SUB_PKG_PREFIX)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtToolsRequire {
    pub id: String,
    pub version: String,
}

/// A command the package guarantees: `bin/<command>` in every payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtToolsCommand {
    pub command: String,
    pub version_probe: String,
    /// A no-network check, relative to the payload root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smoke: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtToolsManifest {
    pub schema: String,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub requires: Vec<ExtToolsRequire>,
    /// Packages that cannot be enabled together with this one: two packages providing one command must say so.
    #[serde(default)]
    pub conflicts: Vec<String>,
    pub provides: Vec<ExtToolsCommand>,
    pub platforms: Vec<String>,
    #[serde(default)]
    pub size_bytes: BTreeMap<String, u64>,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub desc: String,
}

fn safe_id(id: &str) -> bool {
    !id.is_empty()
        && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

fn safe_command(command: &str) -> bool {
    !command.is_empty()
        && !command.starts_with('.')
        && command.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.starts_with('/')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

impl ExtToolsManifest {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let manifest: Self = serde_json::from_str(raw).map_err(|e| format!("{EXT_TOOLS_MANIFEST_FILE}: {e}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        let fail = |message: String| Err(format!("{EXT_TOOLS_MANIFEST_FILE}: {message}"));
        if self.schema != EXT_TOOLS_MANIFEST_SCHEMA {
            return fail(format!("schema must be {EXT_TOOLS_MANIFEST_SCHEMA}"));
        }
        if !safe_id(&self.id) {
            return fail(format!("id `{}` must be lowercase letters, digits and `-`", self.id));
        }
        if self.version.trim().is_empty() || !self.version.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')) {
            return fail(format!("version `{}` must be letters, digits and `._+-`", self.version));
        }
        if self.provides.is_empty() {
            return fail("provides must not be empty".to_string());
        }
        let mut commands = BTreeSet::new();
        for provided in &self.provides {
            if !safe_command(&provided.command) {
                return fail(format!("`{}` is not a command name", provided.command));
            }
            if !commands.insert(provided.command.as_str()) {
                return fail(format!("command `{}` is listed twice", provided.command));
            }
            if provided.version_probe.trim().is_empty() {
                return fail(format!("command `{}` has no version_probe", provided.command));
            }
            if provided.smoke.as_deref().is_some_and(|s| !safe_relative(s)) {
                return fail(format!("command `{}`: smoke must be a relative path inside the payload", provided.command));
            }
        }
        if self.platforms.is_empty() {
            return fail("platforms must not be empty".to_string());
        }
        if let Some(unknown) = self.platforms.iter().find(|p| ext_tools_platform(p).is_none()) {
            return fail(format!("unknown platform `{unknown}`"));
        }
        if let Some(required) = self.requires.iter().find(|r| r.id == self.id || !safe_id(&r.id)) {
            return fail(format!("requires `{}`: not another package's id", required.id));
        }
        if self.conflicts.contains(&self.id) {
            return fail("a package cannot conflict with itself".to_string());
        }
        Ok(())
    }
}

/// `true`: the file or symlink target is a program this payload's platform can run.
fn launcher_runs(path: &Path, windows: bool) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    if !windows {
        use std::os::unix::fs::PermissionsExt;
        return metadata.permissions().mode() & 0o111 != 0;
    }
    let _ = windows;
    true
}

/// The script a payload runs for `provides[].smoke`: a Windows payload has a `.ps1` next to where the `.sh` would be.
pub fn ext_tools_smoke_path(dir: &Path, smoke: &str, windows: bool) -> std::path::PathBuf {
    let path = dir.join(smoke);
    if windows {
        return path.with_extension("ps1");
    }
    path
}

/// What is wrong with the payload directory of `platform`: the layout of a package (`ext-manifest.json`, `bin/`,
/// `smoke/`, `store/<id>/` and nothing else), and a launcher in `bin/` for every command it provides and for nothing
/// else: `provides` is all the system knows of which package provides a command.
pub fn ext_tools_payload_errors(manifest: &ExtToolsManifest, platform: &ExtToolsPlatform, dir: &Path) -> Vec<String> {
    let mut errors = Vec::new();
    let manifest_path = dir.join(EXT_TOOLS_MANIFEST_FILE);
    match std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string()).and_then(|raw| ExtToolsManifest::parse(&raw)) {
        Ok(found) if &found == manifest => {}
        Ok(_) => errors.push(format!("{}: differs from the package's manifest", manifest_path.display())),
        Err(e) => errors.push(format!("{}: {e}", manifest_path.display())),
    }
    let windows = platform.os == "windows";
    for provided in &manifest.provides {
        let bin = dir.join("bin");
        let suffixes: &[&str] = if windows { &["", ".exe", ".cmd", ".bat", ".ps1"] } else { &[""] };
        if !suffixes.iter().any(|s| launcher_runs(&bin.join(format!("{}{s}", provided.command)), windows)) {
            errors.push(format!("bin/{}: no executable launcher", provided.command));
        }
        if let Some(smoke) = &provided.smoke {
            if !ext_tools_smoke_path(dir, smoke, windows).is_file() {
                errors.push(format!("{smoke}: the smoke script of `{}` is missing", provided.command));
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("bin")) {
        let suffixes: &[&str] = if windows { &[".exe", ".cmd", ".bat", ".ps1", ""] } else { &[""] };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let declared = suffixes.iter().any(|s| name.strip_suffix(s).is_some_and(|command| manifest.provides.iter().any(|p| p.command == command)));
            if !declared {
                errors.push(format!("bin/{name}: not in provides"));
            }
        }
    }
    let read = std::fs::read_dir(dir).map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect::<Vec<_>>());
    match read {
        Ok(entries) => {
            for name in entries {
                if ![EXT_TOOLS_MANIFEST_FILE, "bin", "store", "smoke"].contains(&name.as_str()) {
                    errors.push(format!("{name}: a package only holds {EXT_TOOLS_MANIFEST_FILE}, bin/, smoke/ and store/{}/", manifest.id));
                }
            }
        }
        Err(e) => errors.push(format!("{}: {e}", dir.display())),
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("store")) {
        for entry in entries.flatten() {
            if entry.file_name().to_string_lossy() != manifest.id {
                errors.push(format!("store/{}: a package only writes store/{}/", entry.file_name().to_string_lossy(), manifest.id));
            }
        }
    }
    errors
}

/// The commands of the standard layer (`baseline::STANDARD_TOOLS`; libopendan's tests keep the two equal). No package may
/// provide one: `tools/bin` comes before `bin/agent-tools` on the PATH, so it would shadow an Agent's identity and control
/// commands.
pub const EXT_TOOLS_RESERVED_COMMANDS: &[&str] = &[
    "agent-session", "agent_tool", "ai_provider", "ai_provider_test", "ai_quota", "aicc-tool", "ask_user", "buckyos",
    "caption_image", "detect_image", "edit_image", "edit_video", "enhance_audio", "extend_video", "gen_image", "gen_music",
    "gen_video", "import_office", "import_table", "inpaint_image", "llm_understand_media", "materialize_resource",
    "ocr_image", "remove_bg", "resume_aicc_task", "segment_image", "speech_to_text", "text_to_speech", "upscale_image",
    "upscale_video", "xagent",
];

/// Administrator's choice for one package: `system/ext_tools/desired/<id>`, written by control-panel only.
pub const EXT_TOOLS_DESIRED_PREFIX: &str = "system/ext_tools/desired/";

pub fn ext_tools_desired_key(id: &str) -> String {
    format!("{EXT_TOOLS_DESIRED_PREFIX}{id}")
}

/// What a node's reconciler reports: `nodes/<node>/ext_tools/<id>`.
pub fn ext_tools_node_status_key(node_id: &str, id: &str) -> String {
    format!("nodes/{node_id}/ext_tools/{id}")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsDesired {
    pub version: String,
    /// `sha256:<hex>` of the `.pikg` file.
    pub pikg_digest: String,
    /// The FileObject of the `.pikg`: the named object the Repo pins.
    pub content_id: String,
    pub enabled: bool,
    /// The platforms the package has a payload for.
    pub platforms: Vec<String>,
    pub by: String,
    pub at_ms: u64,
}

/// One package in a node's target state: the scheduler multiplied the desired state with the node's platform.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeExtTools {
    pub version: String,
    pub pikg_digest: String,
    pub content_id: String,
    pub enabled: bool,
    /// The payload for the node's own OS; `None`: the package does not support the platform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<String>,
    /// The payload for the node's containers; `None`: the node has none, or the package has no such payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container_payload: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtToolsStage {
    Queued,
    Downloading,
    Verifying,
    Staging,
    Checking,
    Active,
    Removing,
    Removed,
    Failed,
    Unsupported,
}

/// A node's report for one package and one target (`native`: `$BUCKYOS_ROOT/tools`; `container`: the node's tools volume).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsTargetStatus {
    pub stage: ExtToolsStage,
    /// The version in use there, which a failed upgrade leaves in place.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsNodeStatus {
    /// The version the node works toward.
    pub version: String,
    pub stage: ExtToolsStage,
    #[serde(default)]
    pub targets: BTreeMap<String, ExtToolsTargetStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub at_ms: u64,
}

/// `<os>-<arch>` of a device as `DeviceInfo` reports it (`os`: `linux | windows | apple`, `arch`: `amd64 | aarch64 | ...`).
pub fn ext_tools_platform_of_device(os: &str, arch: &str) -> Option<&'static ExtToolsPlatform> {
    let os = match os.to_ascii_lowercase().as_str() {
        "linux" => "linux",
        "windows" => "windows",
        "apple" | "macos" | "darwin" => "macos",
        _ => return None,
    };
    let arch = match arch.to_ascii_lowercase().as_str() {
        "amd64" | "x86_64" | "x64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return None,
    };
    EXT_TOOLS_PLATFORMS.iter().find(|p| p.os == os && p.arch == arch)
}

/// The platform of a device's Linux containers.
pub fn ext_tools_container_platform(arch: &str) -> Option<&'static ExtToolsPlatform> {
    ext_tools_platform_of_device("linux", arch)
}

/// The platform this program runs on.
pub fn ext_tools_current_platform() -> Option<&'static ExtToolsPlatform> {
    ext_tools_platform_of_device(std::env::consts::OS, std::env::consts::ARCH)
}

/// A published version of a package, as much as resolving needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsCatalogEntry {
    pub manifest: ExtToolsManifest,
    pub pikg_digest: String,
    pub content_id: String,
}

/// What enabling, upgrading or disabling a package comes to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtToolsPlan {
    /// Packages to enable or move to another version, dependencies first.
    pub enable: Vec<ExtToolsCatalogEntry>,
    /// Packages to disable.
    pub disable: Vec<String>,
    /// Why it cannot be done; an empty list means it can.
    pub errors: Vec<String>,
}

fn version_of(value: &str) -> Option<semver::Version> {
    let value = value.trim().trim_start_matches('v');
    semver::Version::parse(value).ok().or_else(|| {
        let parts = value.split('.').count();
        (parts < 3).then(|| semver::Version::parse(&format!("{value}{}", ".0".repeat(3 - parts))).ok()).flatten()
    })
}

/// `true`: `version` satisfies a package's `requires` version, e.g. `>=1.0`.
pub fn ext_tools_version_matches(version: &str, requirement: &str) -> bool {
    let requirement = requirement.trim();
    if requirement.is_empty() || requirement == "*" {
        return true;
    }
    match (version_of(version), semver::VersionReq::parse(requirement)) {
        (Some(version), Ok(requirement)) => requirement.matches(&version),
        _ => false,
    }
}

/// Newest first.
pub fn ext_tools_sort_versions(entries: &mut [ExtToolsCatalogEntry]) {
    entries.sort_by(|a, b| version_of(&b.manifest.version).cmp(&version_of(&a.manifest.version)));
}

/// A package's commands that are not its to provide.
pub fn ext_tools_reserved_provides(manifest: &ExtToolsManifest) -> Vec<String> {
    manifest.provides.iter().filter(|p| EXT_TOOLS_RESERVED_COMMANDS.contains(&p.command.as_str())).map(|p| p.command.clone()).collect()
}

fn pick_version<'a>(catalog: &'a [ExtToolsCatalogEntry], id: &str, accepts: impl Fn(&str) -> bool) -> Option<&'a ExtToolsCatalogEntry> {
    let mut found: Vec<&ExtToolsCatalogEntry> = catalog.iter().filter(|e| e.manifest.id == id && accepts(&e.manifest.version)).collect();
    found.sort_by(|a, b| version_of(&b.manifest.version).cmp(&version_of(&a.manifest.version)));
    found.first().copied()
}

fn entry_conflicts(a: &ExtToolsManifest, b: &ExtToolsManifest) -> Option<String> {
    if a.conflicts.contains(&b.id) || b.conflicts.contains(&a.id) {
        return Some(format!("{} and {} declare a conflict", a.id, b.id));
    }
    let shared: Vec<&str> = a.provides.iter().filter(|p| b.provides.iter().any(|q| q.command == p.command)).map(|p| p.command.as_str()).collect();
    (!shared.is_empty()).then(|| format!("{} and {} both provide {}", a.id, b.id, shared.join(", ")))
}

/// Enabling `id` (at `version`, or the newest): its `requires` chain comes along, dependencies first. Whatever stays enabled
/// must still be satisfied afterward. Pure, so the same answer goes to the confirmation page and the commit.
pub fn ext_tools_plan_enable(catalog: &[ExtToolsCatalogEntry], desired: &BTreeMap<String, ExtToolsDesired>, id: &str, version: Option<&str>) -> ExtToolsPlan {
    fn visit<'a>(
        catalog: &'a [ExtToolsCatalogEntry],
        desired: &BTreeMap<String, ExtToolsDesired>,
        entry: &'a ExtToolsCatalogEntry,
        visiting: &mut Vec<String>,
        plan: &mut ExtToolsPlan,
    ) {
        if visiting.contains(&entry.manifest.id) {
            plan.errors.push(format!("dependency cycle: {} -> {}", visiting.join(" -> "), entry.manifest.id));
            return;
        }
        if plan.enable.iter().any(|e| e.manifest.id == entry.manifest.id) {
            return;
        }
        visiting.push(entry.manifest.id.clone());
        for required in &entry.manifest.requires {
            let enabled = desired.get(&required.id).filter(|d| d.enabled);
            if enabled.is_some_and(|d| ext_tools_version_matches(&d.version, &required.version)) {
                continue;
            }
            match pick_version(catalog, &required.id, |v| ext_tools_version_matches(v, &required.version)) {
                Some(found) => visit(catalog, desired, found, visiting, plan),
                None => plan.errors.push(format!("{} requires {} {}, which is not in the Repo", entry.manifest.id, required.id, required.version)),
            }
        }
        visiting.pop();
        plan.enable.push(entry.clone());
    }

    let mut plan = ExtToolsPlan::default();
    let target = match version {
        Some(version) => pick_version(catalog, id, |v| v == version),
        None => pick_version(catalog, id, |_| true),
    };
    let Some(target) = target else {
        plan.errors.push(match version {
            Some(version) => format!("{id} {version} is not in the Repo"),
            None => format!("{id} is not in the Repo"),
        });
        return plan;
    };
    visit(catalog, desired, target, &mut Vec::new(), &mut plan);
    for entry in &plan.enable {
        let reserved = ext_tools_reserved_provides(&entry.manifest);
        if !reserved.is_empty() {
            plan.errors.push(format!("{} provides standard commands: {}", entry.manifest.id, reserved.join(", ")));
        }
    }
    for (i, a) in plan.enable.iter().enumerate() {
        for b in &plan.enable[i + 1..] {
            plan.errors.extend(entry_conflicts(&a.manifest, &b.manifest));
        }
    }
    let manifest_of = |id: &str, version: &str| catalog.iter().find(|e| e.manifest.id == id && e.manifest.version == version).map(|e| &e.manifest);
    for (other, state) in desired.iter().filter(|(_, d)| d.enabled) {
        if plan.enable.iter().any(|e| &e.manifest.id == other) {
            continue;
        }
        let Some(other_manifest) = manifest_of(other, &state.version) else { continue };
        for entry in &plan.enable {
            plan.errors.extend(entry_conflicts(&entry.manifest, other_manifest));
            if let Some(required) = other_manifest.requires.iter().find(|r| r.id == entry.manifest.id) {
                if !ext_tools_version_matches(&entry.manifest.version, &required.version) {
                    plan.errors.push(format!("{other} needs {} {}, not {}", required.id, required.version, entry.manifest.version));
                }
            }
        }
    }
    plan.errors.sort();
    plan.errors.dedup();
    plan
}

/// Disabling `id`: refused while an enabled package needs it.
pub fn ext_tools_plan_disable(catalog: &[ExtToolsCatalogEntry], desired: &BTreeMap<String, ExtToolsDesired>, id: &str) -> ExtToolsPlan {
    let mut plan = ExtToolsPlan { disable: vec![id.to_string()], ..Default::default() };
    for (other, state) in desired.iter().filter(|(other, d)| d.enabled && other.as_str() != id) {
        let manifest = catalog.iter().find(|e| &e.manifest.id == other && e.manifest.version == state.version);
        if manifest.is_some_and(|e| e.manifest.requires.iter().any(|r| r.id == id)) {
            plan.errors.push(format!("{other} requires {id}: disable it first"));
        }
    }
    plan
}

/// A node's target state from the Zone's desired state: `native`, `arch` and whether it runs containers come from the device.
pub fn ext_tools_for_node(desired: &BTreeMap<String, ExtToolsDesired>, os: &str, arch: &str, containers: bool) -> BTreeMap<String, NodeExtTools> {
    let native = ext_tools_platform_of_device(os, arch);
    let container = containers.then(|| ext_tools_container_platform(arch)).flatten();
    desired
        .iter()
        .map(|(id, d)| {
            let offered = |platform: Option<&'static ExtToolsPlatform>| platform.filter(|p| d.platforms.iter().any(|name| name == p.name)).map(|p| p.name.to_string());
            (
                id.clone(),
                NodeExtTools {
                    version: d.version.clone(),
                    pikg_digest: d.pikg_digest.clone(),
                    content_id: d.content_id.clone(),
                    enabled: d.enabled,
                    payload: offered(native),
                    container_payload: offered(container),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ExtToolsManifest {
        ExtToolsManifest {
            schema: EXT_TOOLS_MANIFEST_SCHEMA.to_string(),
            id: "ext-media".to_string(),
            version: "1.2.0".to_string(),
            requires: vec![ExtToolsRequire { id: "ext-core".to_string(), version: ">=1.0".to_string() }],
            conflicts: vec![],
            provides: vec![ExtToolsCommand { command: "ffmpeg".to_string(), version_probe: "ffmpeg -version".to_string(), smoke: Some("smoke/ffmpeg.sh".to_string()) }],
            platforms: vec!["linux-x86_64".to_string()],
            size_bytes: BTreeMap::new(),
            license: "LGPL-2.1".to_string(),
            desc: String::new(),
        }
    }

    fn entry(id: &str, version: &str, commands: &[&str], requires: &[(&str, &str)]) -> ExtToolsCatalogEntry {
        ExtToolsCatalogEntry {
            manifest: ExtToolsManifest {
                schema: EXT_TOOLS_MANIFEST_SCHEMA.to_string(),
                id: id.to_string(),
                version: version.to_string(),
                requires: requires.iter().map(|(id, version)| ExtToolsRequire { id: id.to_string(), version: version.to_string() }).collect(),
                conflicts: vec![],
                provides: commands.iter().map(|c| ExtToolsCommand { command: c.to_string(), version_probe: format!("{c} --version"), smoke: None }).collect(),
                platforms: vec!["linux-x86_64".to_string()],
                size_bytes: BTreeMap::new(),
                license: String::new(),
                desc: String::new(),
            },
            pikg_digest: format!("sha256:{id}-{version}"),
            content_id: format!("file:{id}-{version}"),
        }
    }

    fn enabled(entries: &[&ExtToolsCatalogEntry]) -> BTreeMap<String, ExtToolsDesired> {
        entries
            .iter()
            .map(|e| {
                (
                    e.manifest.id.clone(),
                    ExtToolsDesired {
                        version: e.manifest.version.clone(),
                        pikg_digest: e.pikg_digest.clone(),
                        content_id: e.content_id.clone(),
                        enabled: true,
                        platforms: e.manifest.platforms.clone(),
                        by: "root".to_string(),
                        at_ms: 1,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn enabling_a_package_brings_its_chain_dependencies_first() {
        let core = entry("ext-core", "1.2.0", &["python3"], &[]);
        let old_core = entry("ext-core", "0.9.0", &["python3"], &[]);
        let media = entry("ext-media", "1.0.0", &["ffmpeg"], &[("ext-core", ">=1.0")]);
        let studio = entry("ext-studio", "2.0.0", &["blender"], &[("ext-media", "*")]);
        let catalog = vec![old_core.clone(), core.clone(), media.clone(), studio.clone()];
        let plan = ext_tools_plan_enable(&catalog, &BTreeMap::new(), "ext-studio", None);
        assert!(plan.errors.is_empty(), "{:?}", plan.errors);
        let order: Vec<(&str, &str)> = plan.enable.iter().map(|e| (e.manifest.id.as_str(), e.manifest.version.as_str())).collect();
        assert_eq!(order, vec![("ext-core", "1.2.0"), ("ext-media", "1.0.0"), ("ext-studio", "2.0.0")]);

        let already = ext_tools_plan_enable(&catalog, &enabled(&[&core]), "ext-media", None);
        assert_eq!(already.enable.len(), 1, "what is enabled and satisfies stays out of the plan");

        let too_old = ext_tools_plan_enable(&catalog, &enabled(&[&old_core]), "ext-media", None);
        assert_eq!(too_old.enable[0].manifest.version, "1.2.0", "an enabled version that does not satisfy moves up");

        let missing = ext_tools_plan_enable(&[media.clone()], &BTreeMap::new(), "ext-media", None);
        assert_eq!(missing.errors, vec!["ext-media requires ext-core >=1.0, which is not in the Repo".to_string()]);
        assert_eq!(ext_tools_plan_enable(&catalog, &BTreeMap::new(), "ext-none", None).errors, vec!["ext-none is not in the Repo".to_string()]);
        assert_eq!(ext_tools_plan_enable(&catalog, &BTreeMap::new(), "ext-core", Some("3.0.0")).errors, vec!["ext-core 3.0.0 is not in the Repo".to_string()]);
    }

    #[test]
    fn plans_refuse_cycles_conflicts_shadowing_and_broken_dependents() {
        let a = entry("ext-a", "1.0.0", &["a"], &[("ext-b", "*")]);
        let b = entry("ext-b", "1.0.0", &["b"], &[("ext-a", "*")]);
        assert!(ext_tools_plan_enable(&[a, b], &BTreeMap::new(), "ext-a", None).errors.iter().any(|e| e.contains("cycle")));

        let core = entry("ext-core", "1.0.0", &["python3", "ffmpeg"], &[]);
        let media = entry("ext-media", "1.0.0", &["ffmpeg"], &[]);
        let errors = ext_tools_plan_enable(&[core.clone(), media.clone()], &enabled(&[&core]), "ext-media", None).errors;
        assert_eq!(errors, vec!["ext-media and ext-core both provide ffmpeg".to_string()]);

        let mut declared = entry("ext-other", "1.0.0", &["other"], &[]);
        declared.manifest.conflicts = vec!["ext-core".to_string()];
        let errors = ext_tools_plan_enable(&[core.clone(), declared], &enabled(&[&core]), "ext-other", None).errors;
        assert_eq!(errors, vec!["ext-other and ext-core declare a conflict".to_string()]);

        let shadow = entry("ext-shadow", "1.0.0", &["xagent", "buckyos"], &[]);
        assert_eq!(ext_tools_plan_enable(&[shadow], &BTreeMap::new(), "ext-shadow", None).errors, vec!["ext-shadow provides standard commands: xagent, buckyos".to_string()]);

        let dependent = entry("ext-dep", "1.0.0", &["dep"], &[("ext-core", "^1.0")]);
        let next_core = entry("ext-core", "2.0.0", &["python3", "ffmpeg"], &[]);
        let catalog = vec![core.clone(), next_core, dependent.clone()];
        let errors = ext_tools_plan_enable(&catalog, &enabled(&[&core, &dependent]), "ext-core", Some("2.0.0")).errors;
        assert_eq!(errors, vec!["ext-dep needs ext-core ^1.0, not 2.0.0".to_string()]);

        let refused = ext_tools_plan_disable(&catalog, &enabled(&[&core, &dependent]), "ext-core");
        assert_eq!(refused.errors, vec!["ext-dep requires ext-core: disable it first".to_string()]);
        assert!(ext_tools_plan_disable(&catalog, &enabled(&[&core, &dependent]), "ext-dep").errors.is_empty());
    }

    #[test]
    fn a_node_gets_the_payloads_of_its_platforms() {
        let mut package = entry("ext-core", "1.0.0", &["python3"], &[]);
        package.manifest.platforms = vec!["linux-x86_64".to_string(), "darwin-arm64".to_string()];
        let desired = enabled(&[&package]);
        let linux = &ext_tools_for_node(&desired, "linux", "amd64", true)["ext-core"];
        assert_eq!(linux.payload.as_deref(), Some("linux-x86_64"));
        assert_eq!(linux.container_payload.as_deref(), Some("linux-x86_64"));
        let mac = &ext_tools_for_node(&desired, "apple", "aarch64", false)["ext-core"];
        assert_eq!(mac.payload.as_deref(), Some("darwin-arm64"));
        assert_eq!(mac.container_payload, None);
        let mac_with_docker = &ext_tools_for_node(&desired, "apple", "aarch64", true)["ext-core"];
        assert_eq!(mac_with_docker.container_payload, None, "no linux-aarch64 payload");
        let windows = &ext_tools_for_node(&desired, "windows", "amd64", true)["ext-core"];
        assert_eq!(windows.payload, None, "unsupported");
        assert_eq!(windows.container_payload.as_deref(), Some("linux-x86_64"));
    }

    #[test]
    fn versions_compare_as_versions() {
        assert!(ext_tools_version_matches("1.2.0", ">=1.0"));
        assert!(!ext_tools_version_matches("0.9.0", ">=1.0"));
        assert!(ext_tools_version_matches("1.2", ">=1.0"));
        assert!(ext_tools_version_matches("anything", "*"));
        assert!(!ext_tools_version_matches("anything", ">=1.0"));
        let mut entries = vec![entry("x", "1.9.0", &["x"], &[]), entry("x", "1.10.0", &["x"], &[])];
        ext_tools_sort_versions(&mut entries);
        assert_eq!(entries[0].manifest.version, "1.10.0");
    }

    #[test]
    fn keys_and_platforms_name_each_other() {
        for platform in EXT_TOOLS_PLATFORMS {
            assert_eq!(platform.key, format!("{EXT_TOOLS_SUB_PKG_PREFIX}{}", platform.name.replace('-', "_")));
            assert_eq!(ext_tools_platform_of_key(platform.key), Some(&platform));
            assert_eq!(ext_tools_platform(platform.name), Some(&platform));
        }
        assert!(ext_tools_platform_of_key("ext_tools_windows_aarch64").is_none(), "Windows arm64 is not a platform");
    }

    #[test]
    fn a_manifest_says_what_it_provides_and_nothing_else() {
        assert!(manifest().validate().is_ok());
        let raw = serde_json::to_string(&manifest()).unwrap();
        assert_eq!(ExtToolsManifest::parse(&raw).unwrap(), manifest());
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["writes"] = serde_json::json!(["/usr/bin"]);
        assert!(ExtToolsManifest::parse(&value.to_string()).is_err(), "a package does not declare what it writes");
        for change in [
            (|m: &mut ExtToolsManifest| m.schema = "other".to_string()) as fn(&mut ExtToolsManifest),
            |m| m.id = "Ext Media".to_string(),
            |m| m.provides.clear(),
            |m| m.provides.push(m.provides[0].clone()),
            |m| m.provides[0].command = "../sh".to_string(),
            |m| m.provides[0].smoke = Some("../escape.sh".to_string()),
            |m| m.platforms = vec!["windows-aarch64".to_string()],
            |m| m.requires[0].id = "ext-media".to_string(),
            |m| m.conflicts = vec!["ext-media".to_string()],
        ] {
            let mut bad = manifest();
            change(&mut bad);
            assert!(bad.validate().is_err(), "{bad:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_payload_has_its_launchers_and_only_its_own_tree() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ext-tools-payload-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let root = dir.as_path();
        let manifest = manifest();
        let linux = ext_tools_platform("linux-x86_64").unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::create_dir_all(root.join("smoke")).unwrap();
        std::fs::create_dir_all(root.join("store/ext-media")).unwrap();
        std::fs::write(root.join(EXT_TOOLS_MANIFEST_FILE), serde_json::to_string(&manifest).unwrap()).unwrap();
        std::fs::write(root.join("smoke/ffmpeg.sh"), "#!/bin/sh\n").unwrap();
        let errors = ext_tools_payload_errors(&manifest, linux, root);
        assert_eq!(errors, vec!["bin/ffmpeg: no executable launcher".to_string()]);

        std::fs::write(root.join("bin/ffmpeg"), "#!/bin/sh\n").unwrap();
        assert_eq!(ext_tools_payload_errors(&manifest, linux, root).len(), 1, "not executable yet");
        std::fs::set_permissions(root.join("bin/ffmpeg"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(ext_tools_payload_errors(&manifest, linux, root).is_empty());

        std::fs::create_dir_all(root.join("store/pandoc")).unwrap();
        std::fs::write(root.join("uv-cache"), "").unwrap();
        std::fs::remove_file(root.join("smoke/ffmpeg.sh")).unwrap();
        let errors = ext_tools_payload_errors(&manifest, linux, root);
        assert_eq!(errors.len(), 3, "{errors:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
