//! Extension controls over kRPC (扩展控件 P0, README “Extension controls”): `ext.build` authorizes each file object at
//! the NDM gateway with the caller's own token before the store stages and composes it; the library methods act on
//! the caller's own library (`aiworkspace_store::library`); `ext.install` takes a `.pikg` object or a Workspace export.

use crate::{authorize_object, blocking, s, strings, AppState};
use aiworkspace_core::{Code, WsError, WsResult};
use aiworkspace_store::ext::{BuildInput, BuildOpts, IncomingFile, SourceFile};
use aiworkspace_store::library::{read_pikg, Package};
use aiworkspace_store::service::require_may_own;
use aiworkspace_store::{Caller, CallerClass};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Services allowed to install for another user (`mode: installer`): the control panel's App Installer, by its
/// service id (`appid` of its session) or, with static test identities, its principal.
const INSTALLER_PRINCIPALS: &[&str] = &["control-panel", "control_panel"];

fn is_installer(caller: &Caller) -> bool {
    caller.class == CallerClass::Service
        && (INSTALLER_PRINCIPALS.contains(&caller.principal.as_str()) || caller.app_id.as_deref().is_some_and(|a| INSTALLER_PRINCIPALS.contains(&a)))
}

fn opts(p: &Value) -> WsResult<BuildOpts> {
    let expect = match p.get("expect").filter(|e| !e.is_null()) {
        None => None,
        Some(e) => Some((
            e.get("def_rev").and_then(Value::as_u64).ok_or_else(|| WsError::invalid_op("expect.def_rev required"))?,
            e.get("dir_rev").and_then(Value::as_u64).unwrap_or(0),
        )),
    };
    Ok(BuildOpts {
        folder_id: p.get("folder_id").and_then(Value::as_str).map(str::to_string),
        expect,
        force: p.get("force") == Some(&json!(true)),
        dry_run: p.get("dry_run") == Some(&json!(true)),
        // a build that names its package is an import across Zones: same rules as `ext.import`
        import: p.get("provenance").is_some_and(Value::is_object),
        idempotency_key: p.get("idempotency_key").and_then(Value::as_str).map(str::to_string),
    })
}

/// `ext.build { workspace_id, files, folder_id?, expect?, force?, provenance?, dry_run?, idempotency_key? }`.
pub async fn build(state: &Arc<AppState>, p: &Value, caller: &Caller, token: &str) -> WsResult<Value> {
    let workspace = s(p, "workspace_id")?.to_string();
    let files = IncomingFile::parse_list(p.get("files"))?;
    for f in &files {
        if let IncomingFile::Object { obj_id, .. } = f {
            let id = ndn_lib::ObjId::new(obj_id).map_err(|e| WsError::invalid_op(format!("{}: {e}", f.name())))?;
            authorize_object(state, token, &id).await?;
        }
    }
    let opts = opts(p)?;
    let provenance = p.get("provenance").filter(|v| v.is_object()).cloned();
    let (st, c) = (state.clone(), caller.clone());
    blocking(move || {
        let max = st.limits.max_asset_bytes;
        st.with_ws(&workspace, |ws| {
            let sources = ws.ext_stage_files(&c, files, max, opts.dry_run)?;
            ws.ext_build(&c, vec![BuildInput { files: sources, provenance, implicit: false }], &opts)
        })
    })
    .await
}

fn library_owner(caller: &Caller, p: &Value) -> WsResult<String> {
    match p.get("owner").and_then(Value::as_str) {
        Some(owner) if owner != caller.principal => {
            if is_installer(caller) {
                Ok(owner.to_string())
            } else {
                Err(WsError::denied(format!("only the App Installer installs into another user's library (caller {} via {})", caller.principal, caller.app_id.as_deref().unwrap_or("-"))))
            }
        }
        _ => {
            require_may_own(caller)?;
            Ok(caller.principal.clone())
        }
    }
}

/// `ext.install { pikg: { obj_id } | from: { workspace_id, entity_ids }, package?, mode, owner? }`.
pub async fn install(state: &Arc<AppState>, p: &Value, caller: &Caller, token: &str) -> WsResult<Value> {
    let owner = library_owner(caller, p)?;
    let mode = match p.get("mode").and_then(Value::as_str).unwrap_or("dev") {
        m @ ("dev" | "installer") => m.to_string(),
        m => return Err(WsError::invalid_op(format!("mode {m:?}: dev or installer"))),
    };
    if mode == "installer" && !is_installer(caller) {
        return Err(WsError::denied("mode installer is the App Installer's"));
    }
    if let Some(pikg) = p.get("pikg") {
        // a package the caller bound into its own `exports` bucket (consumed once installed), or an object: the
        // Installer hands over the staged package it already verified, anyone else proves it may read it
        let bound = pikg.get("bucket_id").is_some();
        if !bound && mode == "dev" {
            let id = ndn_lib::ObjId::new(s(pikg, "obj_id")?).map_err(|e| WsError::invalid_op(e.to_string()))?;
            authorize_object(state, token, &id).await?;
        }
        let (st, c, pikg) = (state.clone(), caller.clone(), pikg.clone());
        return blocking(move || {
            let binding = if bound { Some(crate::own_binding(&st, &c, &pikg, aiworkspace_store::objects::EXPORTS)?) } else { None };
            let obj_id = match &binding {
                Some((_, _, obj)) => obj.clone(),
                None => s(&pikg, "obj_id")?.to_string(),
            };
            if st.svc.objects.file_size(&obj_id)? > aiworkspace_store::library::MAX_PACKAGE_BYTES {
                return Err(WsError::limit("a control package is limited to 256 MiB"));
            }
            let tmp = aiworkspace_store::export::TempFile::new("control.pikg")?;
            st.svc.objects.read_to_file(&obj_id, &tmp.0)?;
            let pkg = read_pikg(&tmp.0)?;
            let result = st.library.install(&owner, pkg, &mode, (st.svc.clock)())?;
            if let Some((bucket, key, _)) = binding {
                st.svc.objects.unbind(&bucket, &key)?;
            }
            Ok(result)
        })
        .await;
    }
    let from = p.get("from").ok_or_else(|| WsError::invalid_op("pikg { obj_id } or from { workspace_id, entity_ids } required"))?;
    let workspace = s(from, "workspace_id")?.to_string();
    let ids = strings(from, "entity_ids")?;
    let package = p.get("package").cloned().unwrap_or(Value::Null);
    let pkg_id = package.get("name").and_then(Value::as_str).filter(|n| is_package_name(n)).ok_or_else(|| WsError::invalid_op("package.name: [a-z0-9][a-z0-9.-]{0,127}"))?.to_string();
    let version = package.get("version").and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 64).unwrap_or("0.1.0").to_string();
    let title = package.get("title").and_then(Value::as_str).unwrap_or(&pkg_id).to_string();
    let (st, c) = (state.clone(), caller.clone());
    blocking(move || {
        // export straight into the library: the same bundle a .pikg carries, without packing it
        let bundle = st.with_ws(&workspace, |ws| ws.ext_export(&c, &ids, "package"))?;
        let mut files = BTreeMap::new();
        for f in bundle["files"].as_array().into_iter().flatten() {
            let path = f["path"].as_str().unwrap_or("").to_string();
            let bytes = match (f.get("obj_id").and_then(Value::as_str), f.get("text").and_then(Value::as_str)) {
                (Some(oid), _) => {
                    use aiworkspace_core::materialize::ObjectSource;
                    st.svc.objects.get_file(oid)?
                }
                (None, Some(t)) => t.as_bytes().to_vec(),
                _ => continue,
            };
            files.insert(path, bytes);
        }
        let author = c.did().map(str::to_string).unwrap_or_else(|| c.principal.clone());
        let pkg = Package { pkg_id, version, title, author, manifest: bundle["manifest"].clone(), files };
        st.library.install(&owner, pkg, "dev", (st.svc.clock)())
    })
    .await
}

/// `ext.pack { workspace_id, entity_ids, package: { name, version?, title? } }` (P1, Q20): the controls (with what they
/// build on) packed as an `aiws` PIKG by the service and bound into the caller's own `exports` bucket as
/// `<name>-<version>.pikg`; the answer says how to fetch it (`download`, as `ws.export`) and how to install it
/// (`ext.install { pikg: { bucket_id, key } }`). The owner is the caller's DID; the package is not signed.
pub fn pack(state: &Arc<AppState>, p: &Value, caller: &Caller) -> WsResult<Value> {
    require_may_own(caller)?;
    let ids = strings(p, "entity_ids")?;
    let package = p.get("package").cloned().unwrap_or(Value::Null);
    let name = s(&package, "name")?.to_string();
    let version = package.get("version").and_then(Value::as_str).unwrap_or("0.1.0").to_string();
    let owner = match caller.did() {
        Some(did) if did.starts_with("did:bns:") => did.to_string(),
        _ => format!("did:bns:{}", caller.principal),
    };
    let (_, files, built) = state.with_ws(s(p, "workspace_id")?, |ws| ws.ext_bundle_files(caller, &ids))?;
    let spec = aiworkspace_store::pack::PackSpec {
        show_name: package.get("title").and_then(Value::as_str).unwrap_or(&name).to_string(),
        name,
        version,
        owner,
        timestamp: built.unwrap_or_else(|| ((state.svc.clock)() / 1000).max(0) as u64),
    };
    let packed = aiworkspace_store::pack::pack(&spec, &files)?;
    let key = format!("{}-{}.pikg", spec.name, spec.version);
    let tmp = aiworkspace_store::export::TempFile::new(&key)?;
    std::fs::write(&tmp.0, &packed.bytes).map_err(|e| WsError::io(e.to_string()))?;
    let objects = &state.svc.objects;
    let obj_id = objects.store_file(&tmp.0, &key)?;
    let bucket = aiworkspace_store::objects::bucket_id(&caller.principal, aiworkspace_store::objects::EXPORTS)?;
    let version = objects.bind(&bucket, &key, &obj_id)?;
    let token = objects.read_token(&bucket, &key, aiworkspace_store::export::DOWNLOAD_TOKEN_MS)?;
    let download = json!({ "bucket_id": bucket, "key": key, "version": version, "obj_id": obj_id, "token": token });
    Ok(json!({ "ok": true, "bucket_id": download["bucket_id"], "key": download["key"], "obj_id": download["obj_id"], "size": packed.bytes.len(),
               "digest": format!("sha256:{}", aiworkspace_core::canonical::sha256_hex(&packed.bytes)), "app_doc_object_id": packed.app_doc_object_id,
               "app_did": packed.app_did, "pkg_id": packed.pkg_id, "version": spec.version, "payload_digest": packed.payload_digest,
               "tar_digest": packed.tar_digest, "download": download }))
}

/// A package name of a library entry made from a Workspace (`[a-z0-9][a-z0-9.-]{0,127}`; a PIKG's is its AppId).
fn is_package_name(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty() && b.len() <= 128 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'-'))
}

/// `ext.import { workspace_id, folder_id?, items: [{ pkg_id, ident }] }`: same-Zone, from the caller's library.
pub fn import(state: &Arc<AppState>, p: &Value, caller: &Caller) -> WsResult<Value> {
    require_may_own(caller)?;
    let items = p.get("items").and_then(Value::as_array).filter(|i| !i.is_empty() && i.len() <= 64).ok_or_else(|| WsError::invalid_op("items: 1…64 { pkg_id, ident }"))?;
    let mut by_pkg: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for it in items {
        by_pkg.entry(s(it, "pkg_id")?.to_string()).or_default().push(s(it, "ident")?.to_string());
    }
    // what the items build on comes from the same package, once (P1, Q16)
    let mut controls = Vec::new();
    for (pkg, idents) in by_pkg {
        for c in state.library.controls(&caller.principal, &pkg, Some(&idents), true)? {
            let files = state.library.sources(&c)?;
            controls.push((c, files));
        }
    }
    let mut opts = opts(p)?;
    opts.import = true;
    let max = state.limits.max_asset_bytes;
    state.with_ws(s(p, "workspace_id")?, |ws| {
        let mut inputs = Vec::new();
        for (c, files) in controls {
            // the library's objects are in this service's store already: staging binds them into the Workspace
            let incoming = files.iter().map(|f| IncomingFile::Object { name: f.name.clone(), obj_id: f.obj_id.clone() }).collect();
            let staged: Vec<SourceFile> = ws.ext_stage_files(caller, incoming, max, opts.dry_run)?;
            inputs.push(BuildInput { files: staged, provenance: Some(c.provenance), implicit: c.implicit });
        }
        ws.ext_build(caller, inputs, &opts)
    })
}

/// The synchronous `ext.*` methods.
pub fn dispatch(state: &Arc<AppState>, method: &str, p: &Value, caller: &Caller) -> WsResult<Value> {
    match method {
        "ext.export" => state.with_ws(s(p, "workspace_id")?, |ws| ws.ext_export(caller, &strings(p, "entity_ids")?, p.get("purpose").and_then(Value::as_str).unwrap_or("package"))),
        "ext.list_library" => {
            require_may_own(caller)?;
            state.library.list(&caller.principal)
        }
        "ext.uninstall" => {
            require_may_own(caller)?;
            state.library.uninstall(&caller.principal, s(p, "pkg_id")?)
        }
        "ext.library_bundle" => {
            require_may_own(caller)?;
            let idents = p.get("idents").map(|_| strings(p, "idents")).transpose()?;
            state.library.bundle(&caller.principal, s(p, "pkg_id")?, idents.as_deref())
        }
        "ext.import" => import(state, p, caller),
        "ext.pack" => pack(state, p, caller),
        _ => Err(WsError::new(Code::InvalidOperation, crate::UNKNOWN_METHOD)),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn package_names() {
        assert!(super::is_package_name("acme-kpi"));
        assert!(super::is_package_name("kpi.alice.bns.did"));
        assert!(!super::is_package_name("Acme"));
        assert!(!super::is_package_name(""));
    }
}
