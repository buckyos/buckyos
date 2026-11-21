use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use name_lib::ObjectProfile;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectActionScript {
    pub script: String,
    #[serde(default)]
    pub host_calls: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ObjectDefinition {
    id: String,
    profile: String,
    actions: BTreeMap<String, ObjectActionScript>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectAlias {
    Local { object: String },
    Link { url: String },
    Disabled,
    Default,
}

#[derive(Deserialize)]
struct DefaultAlias {
    name: String,
    #[serde(flatten)]
    target: ObjectAlias,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    #[serde(default)]
    reserved_roots: BTreeSet<String>,
    #[serde(default)]
    objects: Vec<ObjectDefinition>,
    aliases: Vec<DefaultAlias>,
}

#[derive(Clone)]
pub struct LocalScriptObject {
    pub root: PathBuf,
    pub profile: Value,
    pub actions: BTreeMap<String, ObjectActionScript>,
}

#[derive(Default)]
pub struct ObjectExtensions {
    pub objects: BTreeMap<String, LocalScriptObject>,
    pub aliases: BTreeMap<String, (ObjectAlias, String)>,
    pub reserved_roots: BTreeSet<String>,
}

pub fn object_alias_name(name: &str) -> bool {
    name.split('.').all(|part| {
        part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
            && part
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    })
}

pub fn extension_file(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let path = Path::new(relative);
    if !path.components().all(|p| matches!(p, Component::Normal(_))) || relative.contains('\\') {
        return Err("extension file must be package-relative".into());
    }
    let file = root
        .join(path)
        .canonicalize()
        .map_err(|_| format!("missing extension file {relative}"))?;
    if !file.starts_with(root) || !file.is_file() {
        return Err(format!("invalid extension file {relative}"));
    }
    Ok(file)
}

fn known_type(value: &Value) -> bool {
    matches!(
        value.as_str(),
        Some("object" | "array" | "string" | "number" | "integer" | "boolean" | "null")
    )
}

fn check_schema(schema: &Value) -> Result<(), String> {
    let schema = schema
        .as_object()
        .ok_or("local action requires a schema object")?;
    for (key, value) in schema {
        match key.as_str() {
            "type"
                if known_type(value)
                    || value
                        .as_array()
                        .is_some_and(|types| !types.is_empty() && types.iter().all(known_type)) => {
            }
            "description" | "title" if value.is_string() => {}
            "default" => {}
            "enum" if value.as_array().is_some_and(|items| !items.is_empty()) => {}
            "minimum" | "maximum" if value.is_number() => {}
            "minLength" | "maxLength" | "maxItems" if value.is_u64() => {}
            "required"
                if value
                    .as_array()
                    .is_some_and(|keys| keys.iter().all(Value::is_string)) => {}
            "additionalProperties" if value.is_boolean() => {}
            "properties" => {
                for child in value
                    .as_object()
                    .ok_or("invalid schema properties")?
                    .values()
                {
                    check_schema(child)?;
                }
            }
            "items" => check_schema(value)?,
            _ => return Err(format!("unsupported local action schema entry {key}")),
        }
    }
    Ok(())
}

pub fn load_object_extensions(
    id: &str,
    version: &str,
    root: &Path,
    manifest: &str,
) -> Result<ObjectExtensions, String> {
    let root = root
        .canonicalize()
        .map_err(|_| format!("missing object package {id}"))?;
    let manifest_path = extension_file(&root, manifest)?;
    let manifest: Manifest =
        toml::from_str(&fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if manifest.version != 1 {
        return Err("unknown object extension schema version".into());
    }
    let base = manifest_path.parent().unwrap();
    let mut out = ObjectExtensions::default();
    let mut digests = BTreeMap::new();
    for obj in manifest.objects {
        let Some(suffix) = obj.id.strip_prefix(&format!("{id}/")) else {
            return Err("object id must belong to its package".into());
        };
        if !object_alias_name(suffix) || out.objects.contains_key(&obj.id) {
            return Err(format!("invalid or duplicate object {}", obj.id));
        }
        let profile_bytes =
            fs::read(extension_file(base, &obj.profile)?).map_err(|e| e.to_string())?;
        let profile_json: Value =
            serde_json::from_slice(&profile_bytes).map_err(|e| e.to_string())?;
        let profile = ObjectProfile::from_value(profile_json.clone()).map_err(|e| e.to_string())?;
        profile.validate().map_err(|e| e.to_string())?;
        if !profile.events.is_empty() || profile.actions.len() != obj.actions.len() {
            return Err(
                "local events are unsupported and each declared action needs one script".into(),
            );
        }
        let mut digest = Sha256::new();
        digest.update(&profile_bytes);
        for (action, script) in &obj.actions {
            if !profile.actions.contains_key(action) {
                return Err(format!("undeclared action {action}"));
            }
            for key in ["input", "output"] {
                if let Some(schema) = profile_json["actions"][action].get(key) {
                    check_schema(schema)?;
                }
            }
            digest.update(serde_json::to_vec(&(action, script)).map_err(|e| e.to_string())?);
            digest.update(
                fs::read(extension_file(base, &script.script)?).map_err(|e| e.to_string())?,
            );
        }
        digests.insert(obj.id.clone(), format!("{:x}", digest.finalize()));
        out.objects.insert(
            obj.id,
            LocalScriptObject {
                root: base.to_owned(),
                profile: profile_json,
                actions: obj.actions,
            },
        );
    }
    for alias in manifest.aliases {
        let content = match &alias.target {
            ObjectAlias::Local { object } => digests
                .get(object)
                .ok_or_else(|| format!("unknown local object {object}"))?
                .as_str(),
            ObjectAlias::Link { url } => {
                validate_object_link(url)?;
                url.as_str()
            }
            _ => return Err("package defaults must be local or link".into()),
        };
        let revision = format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&(id, version, &alias.name, &alias.target, content)).unwrap()
            )
        );
        if !object_alias_name(&alias.name)
            || out
                .aliases
                .insert(alias.name, (alias.target, revision))
                .is_some()
        {
            return Err("invalid or duplicate alias name".into());
        }
    }
    for root in manifest.reserved_roots {
        if root.contains('.')
            || !out
                .aliases
                .keys()
                .any(|n| n.split('.').next() == Some(root.as_str()))
        {
            return Err(format!("invalid reserved root {root}"));
        }
        out.reserved_roots.insert(root);
    }
    Ok(out)
}

pub fn validate_object_link(value: &str) -> Result<(), String> {
    let url = url::Url::parse(value).map_err(|_| "invalid link URL")?;
    if url.cannot_be_a_base()
        || url.scheme() == "ext-object"
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("invalid link target".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::check_schema;
    use serde_json::json;

    #[test]
    fn local_profiles_require_constraints_the_executor_can_enforce() {
        assert!(check_schema(&json!({"type":["string","null"],"minLength":1})).is_ok());
        for malformed in [
            json!({"type":"unknown"}),
            json!({"required":"title"}),
            json!({"type":"array","items":{"anyOf":[]}}),
            json!({"minLength":-1}),
        ] {
            assert!(check_schema(&malformed).is_err());
        }
    }
}
