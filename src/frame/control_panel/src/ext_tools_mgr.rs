//! Extension packages of the Zone (ExtTools TODO §3.9.2, X2): the catalog is what the Repo has published, the desired state
//! is what the administrator ticked, and the per-node status is what each node's reconciler reports. Nothing here installs:
//! the scheduler multiplies the desired state with each node's platform and node_daemon converges on that.

use crate::user_mgr::require_control_panel_admin;
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::pikg::PikgReader;
use buckyos_api::{
    ext_tools_desired_key, ext_tools_node_status_key, ext_tools_plan_disable, ext_tools_plan_enable,
    ext_tools_platform_of_device, ext_tools_sort_versions, get_buckyos_api_runtime, AppId, AppType,
    ExtToolsCatalogEntry, ExtToolsDesired, ExtToolsManifest, ExtToolsNodeStatus, ExtToolsPlan, PikgStagingPurpose,
    RepoListFilter, EXT_TOOLS_DESIRED_PREFIX, EXT_TOOLS_MANIFEST_FILE,
};
use name_lib::DeviceInfo;
use ndn_lib::{load_named_obj_and_verify, FileObject, NamedObject, ObjId};
use serde_json::{json, Value};
use std::collections::BTreeMap;

const META_APP_TYPE: &str = "app_type";
const META_MANIFEST: &str = "ext_manifest";
const META_DIGEST: &str = "pikg_digest";

fn reason(error: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(error.to_string())
}

/// A package's manifest, the same in every payload it carries: what the Repo keeps on the PIKG's file object, so that
/// listing the catalog does not open a multi-gigabyte file.
pub(crate) async fn ext_manifest_of_pikg(reader: &PikgReader) -> Result<ExtToolsManifest, RPCErrors> {
    let inspection = reader.inspection();
    let mut manifest: Option<ExtToolsManifest> = None;
    for (key, _) in inspection.app_doc.pkg_list.iter() {
        if !buckyos_api::is_ext_tools_sub_pkg(&key) {
            return Err(reason(format!("`{key}` is not an extension package's sub package")));
        }
        let raw = reader.read_payload_file(&key, EXT_TOOLS_MANIFEST_FILE).await.map_err(reason)?;
        let raw = raw.ok_or_else(|| reason(format!("{key} has no {EXT_TOOLS_MANIFEST_FILE}")))?;
        let found = ExtToolsManifest::parse(&String::from_utf8_lossy(&raw)).map_err(reason)?;
        match &manifest {
            Some(first) if first != &found => return Err(reason(format!("{key} carries a different manifest"))),
            Some(_) => {}
            None => manifest = Some(found),
        }
    }
    let manifest = manifest.ok_or_else(|| reason("the package has no payload"))?;
    if manifest.id != inspection.app_doc._base.name || manifest.version != inspection.app_doc.version {
        return Err(reason(format!(
            "the manifest is {} {}, the AppDoc is {} {}",
            manifest.id, manifest.version, inspection.app_doc._base.name, inspection.app_doc.version
        )));
    }
    let reserved = buckyos_api::ext_tools_reserved_provides(&manifest);
    if !reserved.is_empty() {
        return Err(reason(format!("{} provides standard commands: {}", manifest.id, reserved.join(", "))));
    }
    Ok(manifest)
}

pub(crate) fn ext_tools_file_meta(file: &mut FileObject, manifest: &ExtToolsManifest, pikg_digest: &str) {
    file.meta.insert(META_MANIFEST.to_string(), json!(manifest));
    file.meta.insert(META_DIGEST.to_string(), json!(normalize_digest(pikg_digest)));
}

pub(crate) fn normalize_digest(digest: &str) -> String {
    if digest.starts_with("sha256:") {
        digest.to_string()
    } else {
        format!("sha256:{digest}")
    }
}

fn record_value<'a>(meta: &'a Value, key: &str) -> Option<&'a Value> {
    meta.get(key).or_else(|| meta.get("meta").and_then(|inner| inner.get(key)))
}

fn catalog_entry(content_id: &str, meta: &Value) -> Option<ExtToolsCatalogEntry> {
    if record_value(meta, META_APP_TYPE).and_then(Value::as_str) != Some("ext_tools") {
        return None;
    }
    let manifest: ExtToolsManifest = serde_json::from_value(record_value(meta, META_MANIFEST)?.clone()).ok()?;
    let pikg_digest = record_value(meta, META_DIGEST)?.as_str()?.to_string();
    Some(ExtToolsCatalogEntry { manifest, pikg_digest, content_id: content_id.to_string() })
}

fn catalog_json(entries: &[ExtToolsCatalogEntry]) -> Vec<Value> {
    let mut grouped: BTreeMap<&str, Vec<ExtToolsCatalogEntry>> = BTreeMap::new();
    for entry in entries {
        grouped.entry(entry.manifest.id.as_str()).or_default().push(entry.clone());
    }
    grouped
        .into_iter()
        .map(|(id, mut versions)| {
            ext_tools_sort_versions(&mut versions);
            json!({"id": id, "latest": versions[0], "versions": versions.iter().map(|v| &v.manifest.version).collect::<Vec<_>>()})
        })
        .collect()
}

/// command -> the skills that rely on it, from the manuals of the Agents on this node: the static answer to "who would
/// this package help". Reads the front matter only.
fn skills_by_command(root: &std::path::Path) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let home = root.join("data").join("home");
    let dirs = std::fs::read_dir(home).into_iter().flatten().flatten().map(|owner| owner.path().join("agents"));
    let agents = dirs.flat_map(|agents| std::fs::read_dir(agents).into_iter().flatten().flatten().map(|agent| agent.path().join("cmd_skills")).collect::<Vec<_>>());
    for manuals in agents {
        for manual in std::fs::read_dir(manuals).into_iter().flatten().flatten() {
            let path = manual.path();
            let Some(name) = path.file_stem().and_then(|n| n.to_str()).map(str::to_string) else { continue };
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for command in front_matter_commands(&text) {
                let skills = out.entry(command).or_default();
                if !skills.contains(&name) {
                    skills.push(name.clone());
                }
            }
        }
    }
    out
}

fn front_matter_commands(text: &str) -> Vec<String> {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some("---") {
        return Vec::new();
    }
    lines
        .take_while(|line| line.trim() != "---")
        .find_map(|line| line.strip_prefix("commands:"))
        .map(|value| value.split(',').map(|c| c.trim().to_string()).filter(|c| !c.is_empty()).collect())
        .unwrap_or_default()
}

impl ControlPanelServer {
    pub(crate) async fn ext_tools_catalog_entries(&self) -> Result<Vec<ExtToolsCatalogEntry>, RPCErrors> {
        let repo = get_buckyos_api_runtime()?.get_repo_client().await.map_err(reason)?;
        let records = repo.list(Some(RepoListFilter { status: Some("pinned".to_string()), ..Default::default() })).await?;
        Ok(records.iter().filter_map(|record| catalog_entry(&record.content_id, &record.meta)).collect())
    }

    async fn ext_tools_desired(&self) -> Result<BTreeMap<String, ExtToolsDesired>, RPCErrors> {
        let client = get_buckyos_api_runtime()?.get_system_config_client().await?;
        let prefix = EXT_TOOLS_DESIRED_PREFIX.trim_end_matches('/');
        let ids = client.list(prefix).await.map_err(reason).unwrap_or_default();
        let mut desired = BTreeMap::new();
        for id in ids {
            let value = client.get(&ext_tools_desired_key(&id)).await.map_err(reason)?;
            desired.insert(id, serde_json::from_str(&value.value).map_err(reason)?);
        }
        Ok(desired)
    }

    /// `ext_tools.catalog`: what can be enabled, and what each package provides. An Agent asks this to tell which package
    /// would give a skill its missing command.
    pub(crate) async fn handle_ext_tools_catalog(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let entries = self.ext_tools_catalog_entries().await?;
        Ok(RPCResponse::new(RPCResult::Success(json!({"packages": catalog_json(&entries)})), req.seq))
    }

    /// `ext_tools.status`: package × node, for the administration page.
    pub(crate) async fn handle_ext_tools_status(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        let client = runtime.get_system_config_client().await?;
        let entries = self.ext_tools_catalog_entries().await?;
        let desired = self.ext_tools_desired().await?;
        let mut nodes = Vec::new();
        let mut reports: BTreeMap<String, BTreeMap<String, ExtToolsNodeStatus>> = BTreeMap::new();
        for node in client.list("devices").await.map_err(reason)? {
            if let Ok(info) = client.get(&format!("devices/{node}/info")).await {
                if let Ok(info) = serde_json::from_str::<DeviceInfo>(&info.value) {
                    nodes.push(json!({
                        "id": node,
                        "os": info.os,
                        "arch": info.arch,
                        "platform": ext_tools_platform_of_device(&info.os, &info.arch).map(|p| p.name),
                        "containers": info.device_doc.support_container,
                    }));
                }
            }
            for id in client.list(&format!("nodes/{node}/ext_tools")).await.unwrap_or_default() {
                if let Ok(value) = client.get(&ext_tools_node_status_key(&node, &id)).await {
                    if let Ok(status) = serde_json::from_str(&value.value) {
                        reports.entry(id).or_default().insert(node.clone(), status);
                    }
                }
            }
        }
        let mut ids: Vec<String> = entries.iter().map(|e| e.manifest.id.clone()).chain(desired.keys().cloned()).chain(reports.keys().cloned()).collect();
        ids.sort();
        ids.dedup();
        let catalog = catalog_json(&entries);
        let manuals = skills_by_command(&buckyos_kit::get_buckyos_root_dir());
        let packages: Vec<Value> = ids
            .into_iter()
            .map(|id| {
                let published = catalog.iter().find(|c| c["id"] == json!(id));
                let skills: BTreeMap<&String, &Vec<String>> = published
                    .and_then(|p| p["latest"]["manifest"]["provides"].as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|p| p["command"].as_str())
                    .filter_map(|command| manuals.get_key_value(command))
                    .collect();
                json!({
                    "id": id,
                    "latest": published.map(|p| &p["latest"]),
                    "versions": published.map(|p| &p["versions"]),
                    "desired": desired.get(&id),
                    "nodes": reports.get(&id),
                    "skills": skills,
                })
            })
            .collect();
        Ok(RPCResponse::new(RPCResult::Success(json!({"packages": packages, "nodes": nodes})), req.seq))
    }

    fn ext_tools_plan_of(entries: &[ExtToolsCatalogEntry], desired: &BTreeMap<String, ExtToolsDesired>, id: &str, version: Option<&str>, enabled: bool) -> ExtToolsPlan {
        if enabled {
            ext_tools_plan_enable(entries, desired, id, version)
        } else {
            ext_tools_plan_disable(entries, desired, id)
        }
    }

    fn ext_tools_request(req: &RPCRequest) -> Result<(String, Option<String>, bool), RPCErrors> {
        let id = Self::require_param_str(req, "id")?;
        let version = Self::param_str(req, "version");
        let enabled = req.params.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        Ok((id, version, enabled))
    }

    /// `ext_tools.plan`: what the same `set_desired` would do, for the confirmation page.
    pub(crate) async fn handle_ext_tools_plan(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let (id, version, enabled) = Self::ext_tools_request(&req)?;
        let entries = self.ext_tools_catalog_entries().await?;
        let desired = self.ext_tools_desired().await?;
        let plan = Self::ext_tools_plan_of(&entries, &desired, &id, version.as_deref(), enabled);
        Ok(RPCResponse::new(RPCResult::Success(serde_json::to_value(plan).map_err(reason)?), req.seq))
    }

    /// `ext_tools.set_desired`: enable, upgrade, downgrade or disable. The whole `requires` chain lands in one transaction,
    /// or nothing does.
    pub(crate) async fn handle_ext_tools_set_desired(&self, req: RPCRequest, principal: Option<&RpcAuthPrincipal>) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_control_panel_admin(principal)?;
        let (id, version, enabled) = Self::ext_tools_request(&req)?;
        let entries = self.ext_tools_catalog_entries().await?;
        let desired = self.ext_tools_desired().await?;
        let client = get_buckyos_api_runtime()?.get_system_config_client().await?;
        let now = buckyos_kit::buckyos_get_unix_timestamp() * 1000;
        // A seeded package nobody published: asking for it again means leaving it to the seed, which only needs the Zone to
        // have no more to say about it.
        if enabled && !entries.iter().any(|e| e.manifest.id == id) && desired.get(&id).is_some_and(|d| !d.enabled) {
            client.delete(&ext_tools_desired_key(&id)).await.map_err(reason)?;
            return Ok(RPCResponse::new(RPCResult::Success(json!({"ok": true, "plan": ExtToolsPlan::default()})), req.seq));
        }
        let plan = Self::ext_tools_plan_of(&entries, &desired, &id, version.as_deref(), enabled);
        if !plan.errors.is_empty() {
            return Err(reason(plan.errors.join("; ")));
        }
        let mut writes: Vec<(String, ExtToolsDesired)> = plan
            .enable
            .iter()
            .map(|entry| {
                (
                    entry.manifest.id.clone(),
                    ExtToolsDesired {
                        version: entry.manifest.version.clone(),
                        pikg_digest: entry.pikg_digest.clone(),
                        content_id: entry.content_id.clone(),
                        enabled: true,
                        platforms: entry.manifest.platforms.clone(),
                        by: principal.username.clone(),
                        at_ms: now,
                    },
                )
            })
            .collect();
        for gone in &plan.disable {
            let mut state = desired.get(gone).cloned().unwrap_or(ExtToolsDesired {
                version: String::new(),
                pikg_digest: String::new(),
                content_id: String::new(),
                enabled: false,
                platforms: Vec::new(),
                by: String::new(),
                at_ms: 0,
            });
            state.enabled = false;
            state.by = principal.username.clone();
            state.at_ms = now;
            writes.push((gone.clone(), state));
        }
        let mut tx = std::collections::HashMap::new();
        for (key, state) in writes {
            let raw = serde_json::to_string(&state).map_err(reason)?;
            tx.insert(ext_tools_desired_key(&key), buckyos_kit::KVAction::Update(raw));
        }
        client.exec_tx(tx, None).await.map_err(reason)?;
        Ok(RPCResponse::new(RPCResult::Success(json!({"ok": true, "plan": plan})), req.seq))
    }

    /// `ext_tools.import`: put an already built `.pikg` (an official release asset) into the Repo byte for byte, so that the
    /// Zone's digest is the one the installer's seed carries. `app.publish` builds its own PIKG from a project instead.
    pub(crate) async fn handle_ext_tools_import(&self, req: RPCRequest, principal: Option<&RpcAuthPrincipal>) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        require_control_panel_admin(principal)?;
        let handle = Self::require_param_str(&req, "handle")?;
        let runtime = get_buckyos_api_runtime()?;
        let (metadata, staged_id) = self
            .staging_store
            .resolve(&handle, principal.username.as_str(), principal.authenticated_app_id.as_str(), &runtime.zone_id, PikgStagingPurpose::Inspect, None)
            .await?;
        let reader = self.staging_store.reader(&staged_id, &metadata.pikg_digest).await?;
        let app_doc = reader.inspection().app_doc.clone();
        if app_doc.get_app_type() != AppType::ExtTools {
            return Err(reason(format!("{} is not an extension package", app_doc._base.name)));
        }
        let manifest = ext_manifest_of_pikg(&reader).await?;
        let store = self.staging_store.named_store().await?;
        let object = store.get_object(&staged_id).await.map_err(reason)?;
        let mut file: FileObject = load_named_obj_and_verify(&staged_id, &object).map_err(reason)?;
        file.name = format!("{}.pikg", AppId::from_app_did(app_doc.app_did()).map_err(RPCErrors::ReasonError)?);
        crate::app_installer::AppInstaller::published_pikg_meta(&mut file, &app_doc, &reader.inspection().app_doc_object_id);
        ext_tools_file_meta(&mut file, &manifest, reader.pikg_digest());
        let (file_id, file_body) = file.gen_obj_id();
        store.put_object(&file_id, &file_body).await.map_err(reason)?;
        let repo = runtime.get_repo_client().await.map_err(reason)?;
        let stored: ObjId = repo.store(&file_id.to_string()).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!({"ok": true, "id": manifest.id, "version": manifest.version, "pikg_digest": normalize_digest(reader.pikg_digest()), "content_id": stored.to_string()})),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repo_record_is_a_catalog_entry_only_when_it_is_an_extension_package() {
        let manifest = json!({
            "schema": "buckyos.ext_tools/1", "id": "ext-core", "version": "1.0.0",
            "provides": [{"command": "ffmpeg", "version_probe": "ffmpeg -version"}],
            "platforms": ["linux-x86_64"],
        });
        let flat = json!({"app_type": "ext_tools", "ext_manifest": manifest, "pikg_digest": "sha256:ab"});
        let entry = catalog_entry("file:1", &flat).unwrap();
        assert_eq!(entry.manifest.id, "ext-core");
        assert_eq!(entry.content_id, "file:1");
        let nested = json!({"name": "x", "meta": {"app_type": "ext_tools", "ext_manifest": manifest, "pikg_digest": "sha256:ab"}});
        assert_eq!(catalog_entry("file:1", &nested).unwrap(), entry);
        assert!(catalog_entry("file:2", &json!({"app_type": "dapp"})).is_none());
        assert!(catalog_entry("file:3", &json!({"app_type": "ext_tools"})).is_none(), "published before manifests were kept: nothing to resolve with");
    }

    #[test]
    fn the_skills_a_package_helps_come_from_the_manuals_front_matter() {
        assert_eq!(front_matter_commands("---\nname: x\ncommands: ffmpeg, ffprobe\n---\nbody\ncommands: no\n"), vec!["ffmpeg", "ffprobe"]);
        assert!(front_matter_commands("no front matter\ncommands: ffmpeg\n").is_empty());
        assert!(front_matter_commands("---\nname: x\n---\ncommands: ffmpeg\n").is_empty());
    }

    #[test]
    fn the_catalog_lists_each_package_once_with_its_newest_first() {
        let entry = |version: &str| ExtToolsCatalogEntry {
            manifest: serde_json::from_value(json!({"schema": "buckyos.ext_tools/1", "id": "ext-core", "version": version, "provides": [{"command": "x", "version_probe": "x"}], "platforms": ["linux-x86_64"]})).unwrap(),
            pikg_digest: format!("sha256:{version}"),
            content_id: format!("file:{version}"),
        };
        let listed = catalog_json(&[entry("1.9.0"), entry("1.10.0")]);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["latest"]["manifest"]["version"], "1.10.0");
        assert_eq!(listed[0]["versions"], json!(["1.10.0", "1.9.0"]));
    }
}
