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
        if self.version.trim().is_empty() {
            return fail("version is required".to_string());
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
            if !dir.join(smoke).is_file() {
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
