//! Skills of an agent on its AgentRoot: command manuals (`cmd_skills/`,
//! with the commands they bring in `tools/<name>/bin`) and the methods a
//! Goal uses (`goal_skills/`). They come with the agent package, or are
//! installed from a skill package (`skills.install`); `.meta/skills.json`
//! records the installed ones, so a newer agent package never replaces or
//! removes them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;

/// Installed skills of the AgentRoot (`<kind>:<name>` → where they came from).
pub const SKILLS_RECORD: &str = ".meta/skills.json";
pub const SKILLS_RECORD_SCHEMA: &str = "opendan.skills/1";
/// Sync manifest of the agent package (written by the Loader): its keys are
/// the files the package brought.
const PACKAGE_MANIFEST: &str = ".meta/rootfs_sync.json";
/// Commands of a cmd_skill: `tools/<name>/bin`.
pub const TOOLS_DIR: &str = "tools";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillsRecord {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub installed: BTreeMap<String, SkillInstallRecord>,
}

fn key(kind: SkillKind, name: &str) -> String {
    format!("{}:{name}", kind.as_str())
}

pub fn load_record(root: &Path) -> Result<SkillsRecord> {
    let path = root.join(SKILLS_RECORD);
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| OpenDanError::Other(format!("{} is damaged: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SkillsRecord::default()),
        Err(e) => Err(OpenDanError::io(&path, e)),
    }
}

fn save_record(root: &Path, record: &SkillsRecord) -> Result<()> {
    let mut r = record.clone();
    r.schema = SKILLS_RECORD_SCHEMA.into();
    crate::fsutil::atomic_replace_json(&root.join(SKILLS_RECORD), &r)
}

/// AgentRoot-relative paths (`/`-separated) of the installed skills' files:
/// the agent package's sync leaves them alone.
pub fn installed_paths(root: &Path) -> Result<Vec<String>> {
    let record = load_record(root)?;
    let mut out = Vec::new();
    for k in record.installed.keys() {
        let Some((kind, name)) = k.split_once(':') else { continue };
        let kind = if kind == "goal" { SkillKind::Goal } else { SkillKind::Cmd };
        out.push(format!("{}/{name}.md", kind.dir()));
        if kind == SkillKind::Cmd {
            out.push(format!("{TOOLS_DIR}/{name}/"));
        }
    }
    Ok(out)
}

fn package_files(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join(PACKAGE_MANIFEST))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("files").and_then(|f| f.as_object()).map(|f| f.keys().cloned().collect()))
        .unwrap_or_default()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_file())
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".md")).map(str::to_string))
                .filter(|n| crate::ids::validate_session_id(n).is_ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn tools_of(root: &Path, name: &str) -> Vec<String> {
    let mut tools: Vec<String> = std::fs::read_dir(root.join(TOOLS_DIR).join(name).join("bin"))
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().map(str::to_string)).collect())
        .unwrap_or_default();
    tools.sort();
    tools
}

/// The skills of `kind` (`None`: both) on the AgentRoot `root`.
pub fn list(root: &Path, kind: Option<SkillKind>) -> Result<Vec<SkillMeta>> {
    let record = load_record(root)?;
    let package = package_files(root);
    let mut out = Vec::new();
    for k in [SkillKind::Cmd, SkillKind::Goal] {
        if kind.is_some_and(|want| want != k) {
            continue;
        }
        let dir = root.join(k.dir());
        for name in names_in(&dir) {
            let text = std::fs::read_to_string(dir.join(format!("{name}.md"))).unwrap_or_default();
            let mut meta = skill_meta(k, &name, &text);
            meta.install = record.installed.get(&key(k, &name)).cloned();
            meta.origin = if meta.install.is_some() {
                "installed"
            } else if package.contains(&format!("{}/{name}.md", k.dir())) {
                "package"
            } else {
                "local"
            }
            .into();
            if k == SkillKind::Cmd {
                meta.tools = tools_of(root, &name);
            }
            out.push(meta);
        }
    }
    Ok(out)
}

/// A skill package, unpacked: `cmd_skills/<name>.md`, `goal_skills/<name>.md`
/// and `tools/<name>/…` for its cmd_skills.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPackage {
    /// Name of the skill package (the PIKG's App name).
    pub package: String,
    #[serde(default)]
    pub version: String,
    /// `sha256:<hex>` of the `.pikg` file it came from (none: a directory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
    /// Files by package-relative path (`/`-separated).
    pub files: BTreeMap<String, SkillFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFile {
    /// Base64 of the bytes.
    pub content: String,
    #[serde(default)]
    pub executable: bool,
}

impl SkillFile {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.content)
            .map_err(|e| OpenDanError::InvalidArgument(format!("file content is not base64: {e}")))
    }
}

/// `sha256:<hex>` over the sorted `(path, executable, sha256(bytes))` lines
/// of a package: the same content gives the same digest wherever it was
/// packed.
pub fn content_digest(files: &BTreeMap<String, SkillFile>) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (path, f) in files {
        let bytes = f.bytes()?;
        h.update(format!("{path}|{}|{}\n", u8::from(f.executable), hex::encode(Sha256::digest(&bytes))).as_bytes());
    }
    Ok(format!("sha256:{}", hex::encode(h.finalize())))
}

/// Read an unpacked skill package directory (its `cmd_skills/`,
/// `goal_skills/`, `tools/`; everything else is left out).
pub fn read_package_dir(dir: &Path, package: &str, version: &str) -> Result<SkillPackage> {
    use base64::Engine;
    fn walk(base: &Path, dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(OpenDanError::io(dir, e)),
        };
        for e in rd.flatten() {
            let ty = e.file_type().map_err(|err| OpenDanError::io(&e.path(), err))?;
            if ty.is_dir() {
                walk(base, &e.path(), out)?;
            } else if ty.is_file() {
                out.push(e.path());
            }
        }
        Ok(())
    }
    let mut paths = Vec::new();
    for top in [SkillKind::Cmd.dir(), SkillKind::Goal.dir(), TOOLS_DIR] {
        walk(dir, &dir.join(top), &mut paths)?;
    }
    let mut files = BTreeMap::new();
    for p in paths {
        let rel = p
            .strip_prefix(dir)
            .map_err(|e| OpenDanError::Other(e.to_string()))?
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("/");
        let bytes = std::fs::read(&p).map_err(|e| OpenDanError::io(&p, e))?;
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(&p).map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
        };
        #[cfg(not(unix))]
        let executable = false;
        files.insert(
            rel,
            SkillFile {
                content: base64::engine::general_purpose::STANDARD.encode(bytes),
                executable,
            },
        );
    }
    Ok(SkillPackage {
        package: package.to_string(),
        version: version.to_string(),
        pikg_digest: None,
        files,
    })
}

/// What `skill install <path>` installs: a skill PIKG (`*.pikg`), or a
/// directory laid out like its `agent_skills` sub package (`cmd_skills/`,
/// `goal_skills/`, `tools/`; package name = directory name).
pub fn read_source(path: &Path) -> Result<SkillPackage> {
    if path.is_dir() {
        let name = path
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "local".into());
        return read_package_dir(path, &name, "");
    }
    read_pikg(path)
}

/// The sub package of a skill PIKG.
pub const SKILLS_SUB_PKG: &str = "agent_skills";

/// Read a skill PIKG: an App of type `agent_skills` whose `agent_skills`
/// sub package (a `tar.gz` checked against `PACKAGE_META.json`) holds the
/// skills. Package name and version are the App's.
pub fn read_pikg(path: &Path) -> Result<SkillPackage> {
    use base64::Engine;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let bad = |m: String| OpenDanError::InvalidArgument(format!("{}: {m}", path.display()));
    let raw = std::fs::read(path).map_err(|e| OpenDanError::io(path, e))?;
    let pikg_digest = format!("sha256:{}", hex::encode(Sha256::digest(&raw)));
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&raw))
        .map_err(|e| bad(format!("not a PIKG (zip): {e}")))?;
    let mut json_entry = |name: &str| -> Result<serde_json::Value> {
        let mut f = zip.by_name(name).map_err(|e| bad(format!("{name}: {e}")))?;
        let mut text = String::new();
        f.read_to_string(&mut text).map_err(|e| bad(format!("{name}: {e}")))?;
        serde_json::from_str(&text).map_err(|e| bad(format!("{name}: {e}")))
    };
    let app = json_entry("APPDOC.json")?;
    let meta = json_entry("PACKAGE_META.json")?;
    if app["app_type"].as_str() != Some(SKILLS_SUB_PKG) {
        return Err(bad(format!(
            "app_type is `{}`, a skill package has `{SKILLS_SUB_PKG}` (`buckyos pikg init --kind agent-skills`)",
            app["app_type"].as_str().unwrap_or_default()
        )));
    }
    let pkg_objid = app["pkg_list"][SKILLS_SUB_PKG]["pkg_objid"]
        .as_str()
        .ok_or_else(|| bad(format!("APPDOC.json has no pkg_list.{SKILLS_SUB_PKG}.pkg_objid")))?;
    let size = meta["package_objects"][pkg_objid]["size"]
        .as_u64()
        .ok_or_else(|| bad(format!("PACKAGE_META.json has no package object {pkg_objid}")))?;
    let (_, entry) = meta["content_index"]
        .as_object()
        .and_then(|m| m.iter().find(|(_, e)| e["sub_pkg_name"].as_str() == Some(SKILLS_SUB_PKG)))
        .ok_or_else(|| bad(format!("PACKAGE_META.json has no content of `{SKILLS_SUB_PKG}`")))?;
    let entry_path = entry["path"].as_str().unwrap_or_default().to_string();
    let digest = entry["digest"].as_str().unwrap_or_default().to_string();
    let mut payload = Vec::new();
    zip.by_name(&entry_path)
        .map_err(|e| bad(format!("{entry_path}: {e}")))?
        .read_to_end(&mut payload)
        .map_err(|e| bad(format!("{entry_path}: {e}")))?;
    let actual = format!("sha256:{}", hex::encode(Sha256::digest(&payload)));
    if actual != digest || payload.len() as u64 != size {
        return Err(bad(format!(
            "{entry_path} does not match PACKAGE_META.json ({actual}, {} bytes; expected {digest}, {size} bytes)",
            payload.len()
        )));
    }
    let mut files = BTreeMap::new();
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(payload.as_slice()));
    for e in tar.entries().map_err(|e| bad(format!("{entry_path}: {e}")))? {
        let mut e = e.map_err(|e| bad(format!("{entry_path}: {e}")))?;
        let kind = e.header().entry_type();
        let rel = e.path().map_err(|e| bad(e.to_string()))?.to_string_lossy().to_string();
        let rel = rel.trim_start_matches("./").trim_end_matches('/').to_string();
        if kind.is_dir() || rel.is_empty() {
            continue;
        }
        if !kind.is_file() || rel.starts_with('/') || rel.split('/').any(|p| p == "..") {
            return Err(bad(format!("{entry_path}: `{rel}` is not a plain relative file")));
        }
        let top = rel.split('/').next().unwrap_or_default();
        if ![SkillKind::Cmd.dir(), SkillKind::Goal.dir(), TOOLS_DIR].contains(&top) {
            continue;
        }
        let executable = e.header().mode().map(|m| m & 0o111 != 0).unwrap_or(false);
        let mut bytes = Vec::new();
        e.read_to_end(&mut bytes).map_err(|e| bad(e.to_string()))?;
        files.insert(
            rel,
            SkillFile {
                content: base64::engine::general_purpose::STANDARD.encode(bytes),
                executable,
            },
        );
    }
    Ok(SkillPackage {
        package: app["name"].as_str().unwrap_or_default().to_string(),
        version: app["version"].as_str().unwrap_or_default().to_string(),
        pikg_digest: Some(pikg_digest),
        files,
    })
}

/// Where a package file lands, and the skill it belongs to.
fn place(path: &str) -> Result<(SkillKind, String)> {
    let bad = || {
        OpenDanError::InvalidArgument(format!(
            "`{path}` is not a skill file: a skill package holds cmd_skills/<name>.md, goal_skills/<name>.md and tools/<cmd_skill>/…"
        ))
    };
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(bad());
    }
    let (kind, name) = match parts.as_slice() {
        [dir, file] if *dir == SkillKind::Cmd.dir() => (SkillKind::Cmd, file.strip_suffix(".md").ok_or_else(bad)?),
        [dir, file] if *dir == SkillKind::Goal.dir() => (SkillKind::Goal, file.strip_suffix(".md").ok_or_else(bad)?),
        [dir, name, _, ..] if *dir == TOOLS_DIR => (SkillKind::Cmd, *name),
        _ => return Err(bad()),
    };
    crate::ids::validate_session_id(name).map_err(|_| bad())?;
    Ok((kind, name.to_string()))
}

/// What an install did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallReport {
    pub package: String,
    pub version: String,
    /// Content digest of the installed files ([`content_digest`]).
    pub digest: String,
    pub skills: Vec<SkillMeta>,
    /// Skills of the same names it replaced (`replace`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaced: Vec<String>,
}

/// Install the skills of `pkg` into the AgentRoot `root`. A skill whose
/// name is taken by a skill of the agent package, a local one, or one
/// installed from another package is refused unless `replace`; installing
/// a package again updates its own skills. Sessions created before keep
/// their frozen manuals.
pub fn install(root: &Path, pkg: &SkillPackage, replace: bool, who: &str) -> Result<InstallReport> {
    if pkg.package.trim().is_empty() {
        return Err(OpenDanError::InvalidArgument("a skill package needs its name (pkg_name)".into()));
    }
    let digest = content_digest(&pkg.files)?;
    let mut skills: BTreeMap<(SkillKind, String), Vec<(&str, &SkillFile)>> = BTreeMap::new();
    for (path, f) in &pkg.files {
        let (kind, name) = place(path)?;
        skills.entry((kind, name)).or_default().push((path.as_str(), f));
    }
    for (kind, name) in skills.keys() {
        if *kind == SkillKind::Cmd && !pkg.files.contains_key(&format!("{}/{name}.md", kind.dir())) {
            return Err(OpenDanError::InvalidArgument(format!(
                "tools/{name}/ belongs to no cmd_skill: the package has no cmd_skills/{name}.md"
            )));
        }
    }
    if skills.is_empty() {
        return Err(OpenDanError::InvalidArgument(format!(
            "package {} holds no skill (cmd_skills/<name>.md, goal_skills/<name>.md)",
            pkg.package
        )));
    }
    let mut record = load_record(root)?;
    let mut replaced = Vec::new();
    for (kind, name) in skills.keys() {
        let manual = root.join(kind.dir()).join(format!("{name}.md"));
        let tools = root.join(TOOLS_DIR).join(name);
        let taken = manual.exists() || (*kind == SkillKind::Cmd && tools.exists());
        let own = record
            .installed
            .get(&key(*kind, name))
            .is_some_and(|r| r.package == pkg.package);
        if taken && !own {
            if !replace {
                let by = record
                    .installed
                    .get(&key(*kind, name))
                    .map(|r| format!("installed from {}", r.package))
                    .unwrap_or_else(|| "a skill of the agent package or a local one".into());
                return Err(OpenDanError::InvalidArgument(format!(
                    "{} skill `{name}` exists already ({by}); install with replace to take its place",
                    kind.as_str()
                )));
            }
            replaced.push(key(*kind, name));
        }
    }
    let now = crate::now_ms();
    let mut report = InstallReport {
        package: pkg.package.clone(),
        version: pkg.version.clone(),
        digest: digest.clone(),
        replaced,
        ..Default::default()
    };
    for ((kind, name), files) in &skills {
        if *kind == SkillKind::Cmd {
            let tools = root.join(TOOLS_DIR).join(name);
            if tools.exists() {
                std::fs::remove_dir_all(&tools).map_err(|e| OpenDanError::io(&tools, e))?;
            }
        }
        for (path, f) in files {
            let target = root.join(path);
            crate::fsutil::atomic_replace(&target, &f.bytes()?)?;
            #[cfg(unix)]
            if f.executable {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| OpenDanError::io(&target, e))?;
            }
        }
        record.installed.insert(
            key(*kind, name),
            SkillInstallRecord {
                package: pkg.package.clone(),
                version: pkg.version.clone(),
                digest: digest.clone(),
                pikg_digest: pkg.pikg_digest.clone(),
                installed_at_ms: now,
                installed_by: who.to_string(),
            },
        );
    }
    save_record(root, &record)?;
    let all = list(root, None)?;
    report.skills = all
        .into_iter()
        .filter(|m| skills.contains_key(&(m.kind, m.name.clone())))
        .collect();
    Ok(report)
}

/// Remove an installed skill (its manual, and the commands of a cmd_skill).
/// A skill of the agent package is not removed here: the next package sync
/// would bring it back.
pub fn remove(root: &Path, kind: SkillKind, name: &str) -> Result<SkillMeta> {
    crate::ids::validate_session_id(name)
        .map_err(|_| OpenDanError::InvalidArgument(format!("bad skill name `{name}`")))?;
    let mut record = load_record(root)?;
    let meta = list(root, Some(kind))?
        .into_iter()
        .find(|m| m.name == name)
        .ok_or_else(|| OpenDanError::NotFound(format!("{} skill `{name}`", kind.as_str())))?;
    if record.installed.remove(&key(kind, name)).is_none() {
        return Err(OpenDanError::InvalidArgument(format!(
            "{} skill `{name}` was not installed ({}): only installed skills are removed",
            kind.as_str(),
            meta.origin
        )));
    }
    let manual = root.join(kind.dir()).join(format!("{name}.md"));
    if let Err(e) = std::fs::remove_file(&manual) {
        if e.kind() != std::io::ErrorKind::NotFound {
            return Err(OpenDanError::io(&manual, e));
        }
    }
    if kind == SkillKind::Cmd {
        let tools = root.join(TOOLS_DIR).join(name);
        if tools.exists() {
            std::fs::remove_dir_all(&tools).map_err(|e| OpenDanError::io(&tools, e))?;
        }
    }
    save_record(root, &record)?;
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> SkillFile {
        use base64::Engine;
        SkillFile {
            content: base64::engine::general_purpose::STANDARD.encode(text),
            executable: false,
        }
    }

    fn pkg(name: &str, files: &[(&str, &str)]) -> SkillPackage {
        SkillPackage {
            package: name.into(),
            version: "0.1.0".into(),
            pikg_digest: None,
            files: files.iter().map(|(p, t)| (p.to_string(), file(t))).collect(),
        }
    }

    #[test]
    fn front_matter_is_metadata_not_manual() {
        let (fields, body) = split_front_matter("---\nname: x\ndescription: \"Do x\"\nrequires: y\n---\n# X\nbody\n");
        assert_eq!(fields.get("description").map(String::as_str), Some("Do x"));
        assert_eq!(fields.get("requires").map(String::as_str), Some("y"));
        assert_eq!(body, "# X\nbody");
        assert_eq!(split_front_matter("plain\n").1, "plain");
        assert_eq!(split_front_matter("---\nno end").1, "---\nno end");
    }

    #[test]
    fn install_list_remove() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("cmd_skills")).unwrap();
        std::fs::write(root.join("cmd_skills/ui_core.md"), "---\ndescription: chat\n---\nbody").unwrap();
        let p = pkg(
            "skills-demo",
            &[
                ("cmd_skills/demo.md", "---\ndescription: demo things\nrequires: nothing\n---\nRun `demo`."),
                ("tools/demo/bin/demo", "#!/bin/sh\necho demo\n"),
                ("goal_skills/watch.md", "---\ndescription: watching\n---\nWatch."),
            ],
        );
        let r = install(root, &p, false, "did:bns:owner").unwrap();
        assert_eq!(r.skills.len(), 2);
        let cmd = list(root, Some(SkillKind::Cmd)).unwrap();
        let demo = cmd.iter().find(|m| m.name == "demo").unwrap();
        assert_eq!(demo.origin, "installed");
        assert_eq!(demo.description, "demo things");
        assert_eq!(demo.tools, vec!["demo".to_string()]);
        assert_eq!(cmd.iter().find(|m| m.name == "ui_core").unwrap().origin, "local");
        assert!(installed_paths(root).unwrap().contains(&"tools/demo/".to_string()));

        // A name taken by another skill is refused unless replaced.
        let other = pkg("other", &[("cmd_skills/ui_core.md", "x")]);
        assert!(install(root, &other, false, "o").is_err());
        assert_eq!(install(root, &other, true, "o").unwrap().replaced, vec!["cmd:ui_core".to_string()]);
        // The same package again updates its own skills.
        install(root, &p, false, "o").unwrap();

        let bad = pkg("bad", &[("tools/zzz/bin/x", "x")]);
        assert!(install(root, &bad, false, "o").is_err());
        let traversal = pkg("bad", &[("cmd_skills/../role.md", "x")]);
        assert!(install(root, &traversal, false, "o").is_err());

        remove(root, SkillKind::Cmd, "demo").unwrap();
        assert!(!root.join("tools/demo").exists());
        assert!(list(root, Some(SkillKind::Cmd)).unwrap().iter().all(|m| m.name != "demo"));
        assert!(remove(root, SkillKind::Cmd, "nope").is_err());
        std::fs::write(root.join("cmd_skills/local.md"), "x").unwrap();
        assert!(remove(root, SkillKind::Cmd, "local").is_err(), "only installed skills are removed");
    }
}
