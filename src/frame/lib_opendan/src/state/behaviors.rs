//! Behavior catalog of an agent (xAgent §6.2) and freezing it into a
//! session (§6.3).
//!
//! File layout: `<agent_root>/behaviors/<name>.toml` (`__INCLUDE(path)__`
//! expanded relative to the including file), `role.md`, `self.md`,
//! `i18n/<lang>.md`, and the owner's supplement to the role
//! (`.meta/role_supplement.md`, written by the host).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;

/// The agent's behaviors and identity text.
#[async_trait]
pub trait BehaviorCatalog: Send + Sync {
    async fn identity(&self) -> Result<IdentityText>;
    /// Parsed, includes expanded. `None`: the agent has no such behavior.
    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>>;
    async fn list(&self) -> Result<Vec<BehaviorMeta>>;
    /// Version of the catalog as a whole (audit of a freeze).
    async fn revision(&self) -> Result<String>;
}

const MAX_INCLUDE_DEPTH: usize = 8;

/// What the owner adds to the package's role, appended to it.
pub const ROLE_SUPPLEMENT_FILE: &str = ".meta/role_supplement.md";

fn expand_includes(text: &str, base: &Path, depth: usize) -> Result<String> {
    const OPEN: &str = "__INCLUDE(";
    const CLOSE: &str = ")__";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            break;
        };
        out.push_str(&rest[..start]);
        if depth >= MAX_INCLUDE_DEPTH {
            return Err(OpenDanError::InvalidArgument(format!(
                "behavior includes are nested deeper than {MAX_INCLUDE_DEPTH} at {}",
                base.display()
            )));
        }
        let path = base.join(after[..end].trim());
        let included = std::fs::read_to_string(&path).map_err(|e| OpenDanError::io(&path, e))?;
        out.push_str(&expand_includes(
            &included,
            path.parent().unwrap_or(base),
            depth + 1,
        )?);
        rest = &after[end + CLOSE.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Parse one behavior file (its includes expanded).
pub fn parse_behavior(name: &str, path: &Path) -> Result<BehaviorConfig> {
    let raw = std::fs::read_to_string(path).map_err(|e| OpenDanError::io(path, e))?;
    let mut cfg: BehaviorConfig = toml::from_str(&raw)
        .map_err(|e| OpenDanError::InvalidArgument(format!("{}: {e}", path.display())))?;
    let base = path.parent().unwrap_or(Path::new("."));
    for t in [
        &mut cfg.prompt.system,
        &mut cfg.prompt.on_init,
        &mut cfg.prompt.on_input,
        &mut cfg.prompt.on_context_switch,
        &mut cfg.prompt.semi_subscription_snapshot,
    ] {
        if let Some(text) = t {
            *text = expand_includes(text, base, 0)?;
        }
    }
    if cfg.meta.name.is_empty() {
        cfg.meta.name = name.to_string();
    } else if cfg.meta.name != name {
        return Err(OpenDanError::InvalidArgument(format!(
            "{}: meta.name `{}` differs from the file name",
            path.display(),
            cfg.meta.name
        )));
    }
    Ok(cfg)
}

/// Catalog on the AgentRoot files.
pub struct FsBehaviorCatalog {
    root: PathBuf,
}

impl FsBehaviorCatalog {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            root: agent_root.into(),
        }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("behaviors")
    }

    fn names(&self) -> Result<Vec<String>> {
        let dir = self.dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        let mut names: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".toml"))
                    .map(str::to_string)
            })
            .collect();
        names.sort();
        Ok(names)
    }
}

fn read_trimmed(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

#[async_trait]
impl BehaviorCatalog for FsBehaviorCatalog {
    async fn identity(&self) -> Result<IdentityText> {
        let mut i18n = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(self.root.join("i18n")) {
            for e in rd.flatten() {
                if let Some(lang) = e.file_name().to_str().and_then(|n| n.strip_suffix(".md")) {
                    i18n.insert(lang.to_string(), read_trimmed(&e.path()));
                }
            }
        }
        let role = [
            read_trimmed(&self.root.join("role.md")),
            read_trimmed(&self.root.join(ROLE_SUPPLEMENT_FILE)),
        ]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        Ok(IdentityText {
            role,
            self_text: read_trimmed(&self.root.join("self.md")),
            i18n,
        })
    }

    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>> {
        crate::ids::validate_session_id(name)
            .map_err(|_| OpenDanError::InvalidArgument(format!("bad behavior name `{name}`")))?;
        let path = self.dir().join(format!("{name}.toml"));
        if !path.is_file() {
            return Ok(None);
        }
        parse_behavior(name, &path).map(Some)
    }

    async fn list(&self) -> Result<Vec<BehaviorMeta>> {
        let mut out = Vec::new();
        for n in self.names()? {
            match parse_behavior(&n, &self.dir().join(format!("{n}.toml"))) {
                Ok(c) => out.push(c.meta),
                Err(e) => log::warn!("behavior {n}: {e}"),
            }
        }
        Ok(out)
    }

    async fn revision(&self) -> Result<String> {
        let mut h = Sha256::new();
        let mut files: Vec<PathBuf> = vec![
            self.root.join("role.md"),
            self.root.join(ROLE_SUPPLEMENT_FILE),
            self.root.join("self.md"),
        ];
        for d in [self.dir(), self.root.join("i18n")] {
            if let Ok(rd) = std::fs::read_dir(d) {
                files.extend(rd.flatten().map(|e| e.path()));
            }
        }
        files.sort();
        for f in files {
            if let Ok(m) = std::fs::metadata(&f) {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                h.update(format!("{}|{}|{mtime}\n", f.display(), m.len()).as_bytes());
            }
        }
        Ok(format!("sha256:{}", hex::encode(h.finalize())))
    }
}

/// Catalog held in memory (embedding hosts, tests).
#[derive(Default)]
pub struct MemBehaviorCatalog {
    identity: Mutex<IdentityText>,
    behaviors: Mutex<BTreeMap<String, BehaviorConfig>>,
    rev: Mutex<u64>,
}

impl MemBehaviorCatalog {
    pub fn new(identity: IdentityText) -> Self {
        Self {
            identity: Mutex::new(identity),
            ..Default::default()
        }
    }

    pub fn put(&self, name: &str, mut cfg: BehaviorConfig) {
        cfg.meta.name = name.to_string();
        self.behaviors
            .lock()
            .expect("behaviors")
            .insert(name.to_string(), cfg);
        *self.rev.lock().expect("rev") += 1;
    }
}

#[async_trait]
impl BehaviorCatalog for MemBehaviorCatalog {
    async fn identity(&self) -> Result<IdentityText> {
        Ok(self.identity.lock().expect("identity").clone())
    }

    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>> {
        Ok(self.behaviors.lock().expect("behaviors").get(name).cloned())
    }

    async fn list(&self) -> Result<Vec<BehaviorMeta>> {
        Ok(self
            .behaviors
            .lock()
            .expect("behaviors")
            .values()
            .map(|b| b.meta.clone())
            .collect())
    }

    async fn revision(&self) -> Result<String> {
        Ok(format!("mem:{}", self.rev.lock().expect("rev")))
    }
}

fn check_llm_context(cfg: &SessionConfig) -> Result<()> {
    // `prepare_hosted` ignores prompt sections of the `.llm_context`: a
    // frozen session's system comes from its behaviors only.
    for key in ["prompt", "groups", "runs_dir"] {
        if cfg.prompt.llm_context.get(key).is_some() {
            return Err(OpenDanError::InvalidArgument(format!(
                "prompt.llm_context.{key} has no effect in a session; the system text comes from the frozen behavior"
            )));
        }
    }
    Ok(())
}

/// Freeze the identity, the entry behavior and the closure it declares
/// (`meta.next`) into `cfg` (`prompt.frozen` and the runner's entry table).
/// Entry modes are validated here; nothing is frozen on an error.
pub async fn freeze_config(
    cfg: &mut SessionConfig,
    catalog: &dyn BehaviorCatalog,
    who: &str,
) -> Result<()> {
    check_llm_context(cfg)?;
    let mut frozen = FrozenPrompt {
        catalog_rev: catalog.revision().await?,
        frozen_at_ms: crate::now_ms(),
        frozen_by: who.to_string(),
        identity: catalog.identity().await?,
        behaviors: BTreeMap::new(),
    };
    let mut next = cfg.clone();
    let mut queue: Vec<String> = cfg.prompt.behavior.iter().cloned().collect();
    while let Some(name) = queue.pop() {
        if frozen.behaviors.contains_key(&name) {
            continue;
        }
        let b = catalog.get(&name).await?.ok_or_else(|| {
            OpenDanError::InvalidArgument(format!("the agent has no behavior `{name}`"))
        })?;
        next.set_frozen_entry(&name, &b)
            .map_err(OpenDanError::InvalidArgument)?;
        queue.extend(b.meta.next.iter().cloned());
        frozen.behaviors.insert(name, b);
    }
    next.prompt.frozen = Some(frozen);
    next.behaviors().map_err(OpenDanError::InvalidArgument)?;
    *cfg = next;
    Ok(())
}

/// Add a behavior used for the first time after the freeze. `false`: it is
/// frozen already.
pub async fn freeze_behavior(
    cfg: &mut SessionConfig,
    catalog: &dyn BehaviorCatalog,
    name: &str,
) -> Result<bool> {
    let Some(frozen) = &cfg.prompt.frozen else {
        return Err(OpenDanError::blocked(
            "the session has no frozen behaviors to add to",
            None,
        ));
    };
    if frozen.behaviors.contains_key(name) {
        return Ok(false);
    }
    let b = catalog.get(name).await?.ok_or_else(|| {
        OpenDanError::InvalidArgument(format!("the agent has no behavior `{name}`"))
    })?;
    let mut next = cfg.clone();
    next.set_frozen_entry(name, &b)
        .map_err(OpenDanError::InvalidArgument)?;
    if let Some(f) = next.prompt.frozen.as_mut() {
        f.behaviors.insert(name.to_string(), b);
    }
    *cfg = next;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_role_supplement_follows_the_role() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".meta")).unwrap();
        std::fs::write(dir.path().join("role.md"), "You are Jarvis.\n").unwrap();
        std::fs::write(dir.path().join("self.md"), "I keep notes.").unwrap();
        let catalog = FsBehaviorCatalog::new(dir.path());
        let plain = catalog.identity().await.unwrap();
        assert_eq!(plain.role, "You are Jarvis.");
        let before = catalog.revision().await.unwrap();

        std::fs::write(dir.path().join(ROLE_SUPPLEMENT_FILE), "Answer in Japanese.\n").unwrap();
        let with = catalog.identity().await.unwrap();
        assert_eq!(with.role, "You are Jarvis.\n\nAnswer in Japanese.");
        assert_eq!(with.self_text, "I keep notes.");
        let after = catalog.revision().await.unwrap();
        assert_ne!(before, after, "the supplement is part of the catalog revision");

        std::fs::remove_file(dir.path().join(ROLE_SUPPLEMENT_FILE)).unwrap();
        assert_eq!(catalog.identity().await.unwrap(), plain);
        assert_eq!(catalog.revision().await.unwrap(), before);
    }
}
