use crate::{owner_host, AgentDIDObjectError as Error, ObjectRef};
use buckyos_api::object_extensions::{
    load_object_extensions, object_alias_name, validate_object_link, LocalScriptObject,
    ObjectAlias, ObjectExtensions,
};

use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

pub(crate) enum Destination {
    Local(LocalScriptObject),
    Route(ObjectRef),
}

pub(crate) struct Resolved {
    pub target: Destination,
    pub context: Value,
    pub revision: Option<Value>,
}

pub(crate) fn failure(code: &str, message: impl Into<String>) -> Error {
    Error::Business {
        code: code.into(),
        message: message.into(),
        detail: Value::Null,
    }
}

struct Catalog {
    entries: ObjectExtensions,
    collisions: BTreeSet<String>,
    unavailable: BTreeSet<String>,
}

impl Catalog {
    fn installed() -> Result<Self, Error> {
        let root = buckyos_kit::get_buckyos_root_dir();
        let tools = root.join("tools");
        let mut packages = Vec::new();
        let mut unavailable = BTreeSet::new();
        let mut load =
            |id: &str, digest: &str, path: &Path, manifest: &str| match load_object_extensions(
                id, digest, path, manifest,
            ) {
                Ok(package) => packages.push(package),
                Err(_) => {
                    if let Some(names) = path
                        .canonicalize()
                        .ok()
                        .and_then(|root| {
                            buckyos_api::object_extensions::extension_file(&root, manifest).ok()
                        })
                        .and_then(|path| fs::read_to_string(path).ok())
                        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
                        .and_then(|v| v.get("aliases").and_then(toml::Value::as_array).cloned())
                    {
                        unavailable.extend(
                            names
                                .iter()
                                .filter_map(|a| a.get("name")?.as_str())
                                .filter(|name| object_alias_name(name))
                                .map(str::to_owned),
                        );
                    }
                }
            };
        let distribution = root.join("libexec/buckyos-tool");
        let builtin = distribution.join("dist/owner-extensions");
        if builtin.is_dir() {
            let release = fs::read(distribution.join("distribution.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
            let digest = release
                .as_ref()
                .and_then(|r| r["npm_files_sha256"].as_str())
                .unwrap_or(env!("CARGO_PKG_VERSION"));
            load("personal-tools", digest, &builtin, "object_extensions.toml");
        }
        #[derive(serde::Deserialize)]
        struct Active {
            digest: String,
            dir: String,
            manifest: String,
        }
        #[derive(serde::Deserialize, Default)]
        struct Snapshot {
            #[serde(default)]
            object_extensions: BTreeMap<String, Active>,
        }
        let snapshot: Snapshot = match fs::read(tools.join("current.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
                failure(
                    "EXTENSION_UNAVAILABLE",
                    "invalid installed extension snapshot",
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Snapshot::default(),
            Err(_) => {
                return Err(failure(
                    "EXTENSION_UNAVAILABLE",
                    "cannot read installed extension snapshot",
                ))
            }
        };
        for (id, active) in snapshot.object_extensions {
            let prefix = format!("store/{id}/");
            let Some(manifest) = active.manifest.strip_prefix(&prefix) else {
                continue;
            };
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                || active.dir.is_empty()
                || !active
                    .dir
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'+' | b'.' | b'_'))
                || active.dir == ".."
                || active.dir == "."
            {
                continue;
            }
            let path = tools
                .join("versions")
                .join(&id)
                .join(&active.dir)
                .join(&prefix);
            load(&id, &active.digest, &path, manifest);
        }
        let mut catalog = Self {
            unavailable,
            entries: ObjectExtensions::default(),
            collisions: BTreeSet::new(),
        };
        let mut counts = BTreeMap::new();
        let valid = packages;
        for package in &valid {
            for name in package.aliases.keys() {
                *counts.entry(name.clone()).or_insert(0) += 1;
            }
        }
        for package in valid {
            if package.aliases.keys().any(|name| counts[name] > 1) {
                catalog.collisions.extend(package.aliases.into_keys());
                continue;
            }
            catalog.entries.aliases.extend(package.aliases);
            catalog.entries.objects.extend(package.objects);
            catalog
                .entries
                .reserved_roots
                .extend(package.reserved_roots);
        }
        Ok(catalog)
    }

    fn local(&mut self, id: &str) -> Result<Destination, Error> {
        self.entries
            .objects
            .remove(id)
            .map(Destination::Local)
            .ok_or_else(|| {
                failure(
                    "EXTENSION_UNAVAILABLE",
                    "the selected extension object is unavailable",
                )
            })
    }
}

pub(crate) async fn resolve(raw: &str, cwd: &Path) -> Result<Option<Resolved>, Error> {
    if let Some(id) = raw.strip_prefix("ext-object://") {
        return Ok(Some(Resolved {
            target: Catalog::installed()?.local(id)?,
            context: owner_host::context().await?,
            revision: None,
        }));
    }
    if !object_alias_name(raw) {
        return Ok(None);
    }
    select(
        Catalog::installed()?,
        raw,
        cwd,
        owner_host::alias_context(raw).await,
    )
}

fn select(
    mut catalog: Catalog,
    raw: &str,
    cwd: &Path,
    identity: Result<Value, Error>,
) -> Result<Option<Resolved>, Error> {
    let reserved = catalog
        .entries
        .reserved_roots
        .contains(raw.split('.').next().unwrap_or(raw));
    let default = catalog.entries.aliases.remove(raw);
    let mut ctx = match identity {
        Ok(ctx) => ctx,
        Err(error)
            if !reserved
                && default.is_none()
                && !catalog.collisions.contains(raw)
                && !catalog.unavailable.contains(raw)
                && (cwd.join(raw).exists() || error.detail()["code"] == "IDENTITY_UNAVAILABLE") =>
        {
            return Ok(None)
        }
        Err(e) => return Err(e),
    };
    let record = ctx
        .as_object_mut()
        .and_then(|m| m.remove("object_alias"))
        .unwrap_or(Value::Null);
    let mut override_revision = None;
    let binding = if let Some(mut obj) = record.as_object().cloned() {
        if obj.remove("name").as_ref().and_then(Value::as_str) != Some(raw) {
            return Err(failure("ALIAS_INVALID", "alias name mismatch"));
        }
        override_revision = Some(
            obj.remove("alias_revision")
                .filter(Value::is_u64)
                .ok_or_else(|| failure("ALIAS_INVALID", "missing alias revision"))?,
        );
        let selection = serde_json::from_value::<ObjectAlias>(Value::Object(obj))
            .map_err(|_| failure("ALIAS_INVALID", "invalid alias binding"))?;
        (!matches!(selection, ObjectAlias::Default)).then_some(selection)
    } else if record.is_null() {
        None
    } else {
        return Err(failure("ALIAS_INVALID", "invalid alias binding"));
    };
    let (binding, revision) = match binding {
        Some(binding) => (binding, override_revision.unwrap()),
        None => {
            if catalog.unavailable.contains(raw) && default.is_none() {
                return Err(failure(
                    "EXTENSION_UNAVAILABLE",
                    "the installed extension is incomplete",
                ));
            }
            if catalog.collisions.contains(raw) {
                return Err(failure(
                    "ALIAS_CONFLICT",
                    format!("multiple installed packages bind {raw}"),
                ));
            }
            let Some((binding, package_revision)) = default else {
                if reserved {
                    return Err(failure("UNKNOWN_ALIAS", format!("unknown alias {raw}")));
                }
                return Ok(None);
            };
            let revision = match override_revision {
                Some(r) => json!({"override": r, "package": package_revision}),
                None => json!(package_revision),
            };
            (binding, revision)
        }
    };
    if fs::symlink_metadata(cwd.join(raw)).is_ok() {
        return Err(failure(
            "AMBIGUOUS_ALIAS",
            format!("{raw} is also a file; use ./{raw} for the file"),
        ));
    }
    ctx["alias_revision"] = revision.clone();
    let target = match binding {
        ObjectAlias::Local { object } => catalog.local(&object)?,
        ObjectAlias::Link { url } => {
            validate_object_link(&url).map_err(|e| failure("ALIAS_INVALID", e))?;
            Destination::Route(ObjectRef::parse(&url)?)
        }
        _ => {
            return Err(failure(
                "ALIAS_DISABLED",
                format!("alias {raw} is disabled"),
            ))
        }
    };
    Ok(Some(Resolved {
        target,
        context: ctx,
        revision: Some(revision),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Catalog {
        let mut entries = ObjectExtensions::default();
        entries.reserved_roots.insert("owner".into());
        for (name, id) in [("owner", "pkg/user"), ("owner.cal", "pkg/calendar")] {
            entries.aliases.insert(
                name.into(),
                (
                    ObjectAlias::Local { object: id.into() },
                    format!("rev-{name}"),
                ),
            );
            entries.objects.insert(
                id.into(),
                LocalScriptObject {
                    root: Default::default(),
                    profile: json!({"id":id}),
                    actions: Default::default(),
                },
            );
        }
        Catalog {
            entries,
            collisions: Default::default(),
            unavailable: Default::default(),
        }
    }

    #[test]
    fn complete_names_have_independent_targets_revisions_and_failures() {
        let dir = tempfile::tempdir().unwrap();
        let resolve = |name, record| {
            select(
                catalog(),
                name,
                dir.path(),
                Ok(json!({"object_alias":record})),
            )
        };
        let link = json!({"name":"owner","kind":"link","url":"obj://homestation/alice?v=1","alias_revision":2});
        let linked = resolve("owner", link).unwrap().unwrap();
        assert!(
            matches!(linked.target,Destination::Route(ref r) if r.normalized=="obj://homestation/alice?v=1")
        );
        assert_eq!(linked.revision, Some(json!(2)));
        let local = resolve("owner.cal", Value::Null).unwrap().unwrap();
        assert!(
            matches!(local.target, Destination::Local(ref o) if o.profile["id"]=="pkg/calendar")
        );
        assert_eq!(local.revision, Some(json!("rev-owner.cal")));
        for (kind, code) in [
            ("disabled", "ALIAS_DISABLED"),
            ("local", "EXTENSION_UNAVAILABLE"),
        ] {
            let mut rec = json!({"name":"owner","kind":kind,"alias_revision":3});
            if kind == "local" {
                rec["object"] = json!("missing/object");
            }
            assert_eq!(resolve("owner", rec).err().unwrap().detail()["code"], code);
        }
        assert_eq!(
            resolve("owner.caal", Value::Null).err().unwrap().detail()["code"],
            "UNKNOWN_ALIAS"
        );
        std::fs::write(dir.path().join("owner.cal"), "file").unwrap();
        assert_eq!(
            resolve("owner.cal", Value::Null).err().unwrap().detail()["code"],
            "AMBIGUOUS_ALIAS"
        );
        assert!(resolve("notes.md", Value::Null).unwrap().is_none());
    }
}
