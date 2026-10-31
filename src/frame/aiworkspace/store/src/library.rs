//! The control library (扩展控件 P0, A5): what each user of this deployment may bring into any Workspace. Packages
//! come as an `aiws` PIKG (the Installer, or a client in dev mode) or straight from a Workspace export; their files
//! are FileObjects bound in the user's `library` bucket (`<pkg_id>/<version>/<path>`), the index is
//! `library.sqlite` in the service data folder. Workspaces keep their own copies: uninstalling never touches them.

use crate::docdb::db_err;
use crate::ext::{compose, content_digest, file_object_id, SourceFile};
use crate::objects::{bucket_id, ObjectStore, StoreSink};
use aiworkspace_core::canonical::sha256_hex;
use aiworkspace_core::materialize::{ObjectSink, ObjectSource};
use aiworkspace_core::types::MANIFEST_VERSION;
use aiworkspace_core::{Code, WsError, WsResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The bucket of a user's library.
pub const LIBRARY: &str = "library";
/// The root file of a bundle.
pub const MANIFEST_FILE: &str = "aiws-manifest.json";
/// The PIKG sub-package of a control package.
pub const AIWS_SUB_PACKAGE: &str = "aiws";
/// Files of one package (all controls).
pub const MAX_PACKAGE_FILES: usize = 4096;
/// Unpacked bytes of one package.
pub const MAX_PACKAGE_BYTES: u64 = 256 * 1024 * 1024;

fn invalid(detail: impl Into<String>) -> WsError {
    WsError::new(Code::LibraryPackageInvalid, detail)
}

/// A package ready to be recorded: who it is, its manifest, and its files by path (`controls/<dir>/<name>`).
#[derive(Debug, Clone)]
pub struct Package {
    pub pkg_id: String,
    pub version: String,
    pub title: String,
    pub author: String,
    pub manifest: Value,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// The AppId of an App DID (buckyos-websdk `appIdFromDid`): `did:web:<host>` → `<host>`, `did:<m>:<id>` → `<id>.<m>.did`.
pub fn app_id_from_did(did: &str) -> WsResult<String> {
    let mut parts = did.splitn(3, ':');
    let (Some("did"), Some(method), Some(id)) = (parts.next(), parts.next(), parts.next()) else { return Err(invalid(format!("{did} is not a DID"))) };
    let label_ok = |l: &str| !l.is_empty() && l.len() <= 63 && l.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') && !l.starts_with('-') && !l.ends_with('-');
    if method.is_empty() || !id.split('.').all(label_ok) {
        return Err(invalid(format!("App DID {did} must use lowercase DNS labels")));
    }
    Ok(if method == "web" { id.to_string() } else { format!("{id}.{method}.did") })
}

/// Check a bundle: the manifest (README “AIWS manifest”), every control's files, digest and `component.json` (the same
/// composer a build uses), nothing outside the controls' directories. Answers each control with its files.
pub fn verify_bundle(manifest: &Value, files: &BTreeMap<String, Vec<u8>>) -> WsResult<Vec<(Value, Vec<SourceFile>)>> {
    let m = manifest.as_object().ok_or_else(|| invalid("aiws-manifest.json must be an object"))?;
    for k in m.keys() {
        if !matches!(k.as_str(), "manifest_version" | "purpose" | "host_api" | "controls" | "workspace_templates") {
            return Err(invalid(format!("aiws-manifest.json: unknown key {k}")));
        }
    }
    match m.get("manifest_version").and_then(Value::as_u64) {
        Some(v) if (1..=MANIFEST_VERSION).contains(&v) => {}
        v => return Err(WsError::new(Code::UnsupportedVersion, format!("aiws-manifest.json manifest_version {v:?}: this service reads 1…{MANIFEST_VERSION}"))),
    }
    if m.get("purpose").and_then(Value::as_str) != Some("aiws") {
        return Err(invalid("aiws-manifest.json purpose must be aiws"));
    }
    if !m.get("workspace_templates").and_then(Value::as_array).is_some_and(Vec::is_empty) {
        return Err(WsError::new(Code::UnsupportedVersion, "Workspace templates are not supported yet: this package needs a newer AI Workspace"));
    }
    let controls = m.get("controls").and_then(Value::as_array).filter(|c| !c.is_empty() && c.len() <= 256).ok_or_else(|| invalid("aiws-manifest.json lists 1…256 controls"))?;
    let mut out = Vec::new();
    let mut claimed: Vec<String> = Vec::new();
    for c in controls {
        let dir = c.get("dir").and_then(Value::as_str).filter(|d| d.starts_with("controls/") && d.ends_with('/') && d.len() > 10 && !d[9..d.len() - 1].contains('/'))
            .ok_or_else(|| invalid(format!("control {}: dir must be controls/<name>/", c["ident"])))?;
        if claimed.iter().any(|d| d == dir) {
            return Err(invalid(format!("{dir} is listed twice")));
        }
        claimed.push(dir.to_string());
        let mut sources = Vec::new();
        for (path, bytes) in files.range(dir.to_string()..) {
            let Some(name) = path.strip_prefix(dir) else { break };
            sources.push(SourceFile { name: name.to_string(), obj_id: file_object_id(bytes)?, bytes: bytes.clone() });
        }
        let digest = content_digest(sources.iter().map(|f| (f.name.as_str(), f.bytes.as_slice())));
        if c.get("digest").and_then(Value::as_str) != Some(digest.as_str()) {
            return Err(invalid(format!("{dir}: the files do not match the manifest digest (tampered or incomplete)")));
        }
        let composed = compose(&sources).map_err(|d| invalid(format!("{dir}: {}", d.iter().filter(|x| x["severity"] == "error").map(|x| format!("{}: {}", x["file"].as_str().unwrap_or(""), x["message"].as_str().unwrap_or(""))).collect::<Vec<_>>().join("; "))))?;
        if c.get("ident").and_then(Value::as_str) != Some(composed.ident.as_str()) || c.get("kind").and_then(Value::as_str) != Some(composed.kind.as_str())
            || c.get("name").and_then(Value::as_str) != Some(composed.name.as_str()) {
            return Err(invalid(format!("{dir}: ident, name and kind must match its component.json")));
        }
        if !c.get("version").and_then(Value::as_u64).is_some_and(|v| v >= 1) {
            return Err(invalid(format!("{dir}: version must be a positive integer")));
        }
        let listed: Vec<&str> = c.get("deps").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        let mut declared: Vec<&str> = composed.deps.iter().map(String::as_str).collect();
        let mut sorted = listed.clone();
        sorted.sort_unstable();
        declared.sort_unstable();
        if sorted != declared {
            return Err(invalid(format!("{dir}: deps must match its component.json")));
        }
        out.push((c.clone(), sources));
    }
    // what a control builds on travels with it (P1, Q16): every dependency is another control of this bundle
    let idents: Vec<&str> = controls.iter().filter_map(|c| c.get("ident").and_then(Value::as_str)).collect();
    for (c, _) in &out {
        for dep in c.get("deps").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
            if !idents.contains(&dep) {
                return Err(invalid(format!("{} depends on {dep}, which the package does not carry", c["ident"].as_str().unwrap_or(""))));
            }
        }
    }
    for path in files.keys() {
        if path != MANIFEST_FILE && !claimed.iter().any(|d| path.starts_with(d.as_str())) {
            return Err(invalid(format!("{path} belongs to no control of the manifest")));
        }
    }
    Ok(out)
}

/// A bundle's files from a `tar.gz` (the `aiws` sub-package): regular files only, no links, nothing escaping the root.
pub fn untar_bundle(bytes: &[u8]) -> WsResult<BTreeMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(bytes));
    let mut out = BTreeMap::new();
    let mut total = 0u64;
    for entry in archive.entries().map_err(|e| invalid(format!("aiws payload: {e}")))? {
        let mut entry = entry.map_err(|e| invalid(format!("aiws payload: {e}")))?;
        let kind = entry.header().entry_type();
        let path = entry.path().map_err(|e| invalid(format!("aiws payload path: {e}")))?.to_string_lossy().to_string();
        let path = path.trim_start_matches("./").to_string();
        if kind.is_dir() || path.is_empty() {
            continue;
        }
        if !kind.is_file() {
            return Err(invalid(format!("aiws payload entry {path} is not a regular file")));
        }
        if path.starts_with('/') || path.contains('\\') || path.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
            return Err(invalid(format!("aiws payload entry {path} escapes its root")));
        }
        total += entry.size();
        if total > MAX_PACKAGE_BYTES || out.len() >= MAX_PACKAGE_FILES {
            return Err(WsError::limit("a control package is limited to 256 MiB and 4096 files"));
        }
        let mut data = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut data).map_err(|e| invalid(format!("aiws payload {path}: {e}")))?;
        if out.insert(path.clone(), data).is_some() {
            return Err(invalid(format!("aiws payload lists {path} twice")));
        }
    }
    Ok(out)
}

fn zip_entry(zip: &mut zip::ZipArchive<std::fs::File>, name: &str, limit: u64) -> WsResult<Option<Vec<u8>>> {
    let mut f = match zip.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(invalid(format!("{name}: {e}"))),
    };
    if f.size() > limit {
        return Err(invalid(format!("{name} exceeds {limit} bytes")));
    }
    let mut data = Vec::with_capacity(f.size() as usize);
    f.read_to_end(&mut data).map_err(|e| invalid(format!("{name}: {e}")))?;
    Ok(Some(data))
}

/// The AppDoc claims of `APPDOC.jwt` (the reader takes the form, not a signature: doc/pikg/format.md §1, §4).
fn jwt_claims(jwt: &[u8]) -> WsResult<Value> {
    use base64::Engine;
    let text = std::str::from_utf8(jwt).map_err(|_| invalid("APPDOC.jwt is not text"))?.trim();
    let payload = text.split('.').nth(1).ok_or_else(|| invalid("APPDOC.jwt is not a JWT"))?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).map_err(|e| invalid(format!("APPDOC.jwt: {e}")))?;
    serde_json::from_slice(&bytes).map_err(|e| invalid(format!("APPDOC.jwt claims: {e}")))
}

/// Read an `aiws` PIKG (doc/pikg/format.md): the AppDoc (`app_type: aiws`, one sub-package `aiws`, nothing to run),
/// `PACKAGE_META.json`'s content index, the payload's size and SHA-256, then the bundle inside it. The AppDoc object
/// id and its signature are the Installer's (`control_panel` PikgReader, trust); this checks the content.
pub fn read_pikg(path: &Path) -> WsResult<Package> {
    let file = std::fs::File::open(path).map_err(|e| WsError::io(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| invalid(format!("not a PIKG (ZIP): {e}")))?;
    if zip.len() > 4096 {
        return Err(invalid("too many PIKG entries"));
    }
    for i in 0..zip.len() {
        let name = zip.by_index_raw(i).map_err(|e| invalid(e.to_string()))?.name().to_string();
        if name.is_empty() || name.starts_with('/') || name.contains('\\') || name.split('/').any(|p| p == "..") {
            return Err(invalid(format!("PIKG entry {name:?} is not a safe name")));
        }
    }
    let app_doc: Value = match (zip_entry(&mut zip, "APPDOC.json", 1 << 20)?, zip_entry(&mut zip, "APPDOC.jwt", 1 << 20)?) {
        (Some(json), _) => serde_json::from_slice(&json).map_err(|e| invalid(format!("APPDOC.json: {e}")))?,
        (None, Some(jwt)) => jwt_claims(&jwt)?,
        (None, None) => return Err(invalid("a PIKG carries APPDOC.json or APPDOC.jwt")),
    };
    if app_doc.get("doc_type").and_then(Value::as_str) != Some("app") || app_doc.get("app_type").and_then(Value::as_str) != Some("aiws") {
        return Err(invalid("not a control package: the AppDoc's app_type is not aiws"));
    }
    let pkg_list = app_doc.get("pkg_list").and_then(Value::as_object).ok_or_else(|| invalid("APPDOC.pkg_list missing"))?;
    if pkg_list.len() != 1 || !pkg_list.contains_key(AIWS_SUB_PACKAGE) {
        return Err(invalid("an aiws package has exactly one sub-package, `aiws` (no docker, web or native packages)"));
    }
    let did = app_doc.get("did").and_then(Value::as_str).ok_or_else(|| invalid("APPDOC.did missing"))?;
    let pkg_id = app_id_from_did(did)?;
    let version = app_doc.get("version").and_then(Value::as_str).filter(|v| !v.is_empty() && v.len() <= 64).ok_or_else(|| invalid("APPDOC.version missing"))?.to_string();
    let meta: Value = zip_entry(&mut zip, "PACKAGE_META.json", 8 << 20)?
        .map(|b| serde_json::from_slice(&b).map_err(|e| invalid(format!("PACKAGE_META.json: {e}"))))
        .transpose()?
        .ok_or_else(|| invalid("PACKAGE_META.json missing"))?;
    if meta.get("@schema").and_then(Value::as_str) != Some("buckyos.pikg.package-meta.v1") {
        return Err(invalid("PACKAGE_META.json @schema must be buckyos.pikg.package-meta.v1"));
    }
    let (digest, entry) = meta.get("content_index").and_then(Value::as_object).into_iter().flatten()
        .find(|(_, e)| e.get("sub_pkg_name").and_then(Value::as_str) == Some(AIWS_SUB_PACKAGE))
        .ok_or_else(|| invalid("PACKAGE_META.json indexes no `aiws` payload"))?;
    let payload_path = entry.get("path").and_then(Value::as_str).ok_or_else(|| invalid("content index entry without path"))?;
    let payload = zip_entry(&mut zip, payload_path, MAX_PACKAGE_BYTES)?.ok_or_else(|| invalid(format!("payload {payload_path} missing")))?;
    if entry.get("size").and_then(Value::as_u64) != Some(payload.len() as u64) || digest.strip_prefix("sha256:") != Some(sha256_hex(&payload).as_str())
        || entry.get("digest").and_then(Value::as_str) != Some(digest.as_str()) {
        return Err(invalid("the aiws payload does not match PACKAGE_META.json (size or SHA-256)"));
    }
    let mut files = untar_bundle(&payload)?;
    let manifest_bytes = files.remove(MANIFEST_FILE).ok_or_else(|| invalid("the aiws payload has no aiws-manifest.json"))?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes).map_err(|e| invalid(format!("aiws-manifest.json: {e}")))?;
    let title = app_doc.get("show_name").and_then(Value::as_str).or(app_doc.get("name").and_then(Value::as_str)).unwrap_or(&pkg_id).to_string();
    let author = app_doc.get("author").and_then(Value::as_str).unwrap_or("").to_string();
    Ok(Package { pkg_id, version, title, author, manifest, files })
}

pub struct Library {
    conn: Mutex<Connection>,
    objects: Arc<ObjectStore>,
}

const DDL: &str = "
CREATE TABLE IF NOT EXISTS library_packages (
  owner TEXT NOT NULL, pkg_id TEXT NOT NULL, version TEXT NOT NULL, title TEXT NOT NULL, author TEXT NOT NULL,
  mode TEXT NOT NULL, digest TEXT NOT NULL, manifest_json TEXT NOT NULL, installed_at INTEGER NOT NULL,
  PRIMARY KEY (owner, pkg_id));
CREATE TABLE IF NOT EXISTS library_controls (
  owner TEXT NOT NULL, pkg_id TEXT NOT NULL, ident TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL, version INTEGER NOT NULL,
  digest TEXT NOT NULL, dir TEXT NOT NULL, info_json TEXT NOT NULL, files_json TEXT NOT NULL,
  PRIMARY KEY (owner, pkg_id, ident));
";

impl Library {
    pub fn open(path: &Path, objects: Arc<ObjectStore>) -> WsResult<Library> {
        let conn = Connection::open(path).map_err(db_err)?;
        conn.pragma_update(None, "journal_mode", "WAL").map_err(db_err)?;
        conn.busy_timeout(std::time::Duration::from_secs(5)).map_err(db_err)?;
        conn.execute_batch(DDL).map_err(db_err)?;
        Ok(Library { conn: Mutex::new(conn), objects })
    }

    /// Record a package for `owner` (`ext.install`): verify it, store its files, index it. The same id and version
    /// with the same content is `satisfied`, with other content refused; another version replaces the older one.
    pub fn install(&self, owner: &str, pkg: Package, mode: &str, now_ms: i64) -> WsResult<Value> {
        let controls = verify_bundle(&pkg.manifest, &pkg.files)?;
        let digests: Vec<&str> = controls.iter().filter_map(|(c, _)| c["digest"].as_str()).collect();
        let digest = format!("sha256:{}", sha256_hex(serde_json::to_string(&digests).unwrap_or_default().as_bytes()));
        let conn = self.conn.lock().map_err(|_| WsError::io("library lock poisoned"))?;
        let existing: Option<(String, String)> = conn
            .query_row("SELECT version, digest FROM library_packages WHERE owner = ?1 AND pkg_id = ?2", params![owner, pkg.pkg_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(db_err)?;
        let summary = |action: &str| json!({ "ok": true, "pkg_id": pkg.pkg_id, "version": pkg.version, "action": action,
            "controls": controls.iter().map(|(c, _)| json!({ "ident": c["ident"], "name": c["name"], "kind": c["kind"], "version": c["version"], "title": c["title"] })).collect::<Vec<_>>() });
        let replaced = match &existing {
            Some((v, d)) if *v == pkg.version && *d == digest => return Ok(summary("satisfied")),
            Some((v, _)) if *v == pkg.version => return Err(invalid(format!("{} {} is already in the library with other content: give the package a new version", pkg.pkg_id, pkg.version))),
            Some((v, _)) => Some(v.clone()),
            None => None,
        };
        let bucket = bucket_id(owner, LIBRARY)?;
        let mut sink = StoreSink::new(&self.objects);
        let mut rows = Vec::new();
        for (c, files) in &controls {
            let dir = c["dir"].as_str().unwrap_or("");
            let mut listed = Vec::new();
            for f in files {
                let oid = sink.put_file(&f.bytes)?;
                self.objects.bind(&bucket, &format!("{}/{}/{dir}{}", pkg.pkg_id, pkg.version, f.name), &oid)?;
                listed.push(json!({ "name": f.name, "obj_id": oid, "size": f.bytes.len() }));
            }
            rows.push((c.clone(), listed));
        }
        let tx = conn.unchecked_transaction().map_err(db_err)?;
        if let Some(old) = &replaced {
            self.unbind_package(&tx, owner, &pkg.pkg_id, old)?;
        }
        tx.execute("DELETE FROM library_controls WHERE owner = ?1 AND pkg_id = ?2", params![owner, pkg.pkg_id]).map_err(db_err)?;
        tx.execute(
            "INSERT OR REPLACE INTO library_packages (owner, pkg_id, version, title, author, mode, digest, manifest_json, installed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![owner, pkg.pkg_id, pkg.version, pkg.title, pkg.author, mode, digest, pkg.manifest.to_string(), now_ms],
        )
        .map_err(db_err)?;
        for (c, files) in rows {
            let info = json!({ "title": c.get("title"), "description": c.get("description"), "catalog": c.get("catalog"), "deps": c.get("deps") });
            tx.execute(
                "INSERT INTO library_controls (owner, pkg_id, ident, name, kind, version, digest, dir, info_json, files_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![owner, pkg.pkg_id, c["ident"].as_str(), c["name"].as_str(), c["kind"].as_str(), c["version"].as_i64(), c["digest"].as_str(), c["dir"].as_str(),
                        info.to_string(), Value::Array(files).to_string()],
            )
            .map_err(db_err)?;
        }
        tx.commit().map_err(db_err)?;
        Ok(summary(if replaced.is_some() { "replaced" } else { "installed" }))
    }

    fn unbind_package(&self, conn: &Connection, owner: &str, pkg_id: &str, version: &str) -> WsResult<()> {
        let bucket = bucket_id(owner, LIBRARY)?;
        for (dir, files) in self.rows(conn, owner, pkg_id)?.into_iter().map(|r| (r.dir, r.files)) {
            for f in files {
                let _ = self.objects.unbind(&bucket, &format!("{pkg_id}/{version}/{dir}{}", f["name"].as_str().unwrap_or("")));
            }
        }
        Ok(())
    }

    fn rows(&self, conn: &Connection, owner: &str, pkg_id: &str) -> WsResult<Vec<ControlRow>> {
        let mut st = conn.prepare("SELECT ident, name, kind, version, digest, dir, info_json, files_json FROM library_controls WHERE owner = ?1 AND pkg_id = ?2 ORDER BY dir").map_err(db_err)?;
        let rows = st
            .query_map(params![owner, pkg_id], |r| {
                Ok(ControlRow {
                    ident: r.get(0)?,
                    name: r.get(1)?,
                    kind: r.get(2)?,
                    version: r.get(3)?,
                    digest: r.get(4)?,
                    dir: r.get(5)?,
                    info: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or(Value::Null),
                    files: serde_json::from_str::<Vec<Value>>(&r.get::<_, String>(7)?).unwrap_or_default(),
                })
            })
            .map_err(db_err)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db_err)?;
        Ok(rows)
    }

    /// `ext.list_library`: the owner's packages and their controls.
    pub fn list(&self, owner: &str) -> WsResult<Value> {
        let conn = self.conn.lock().map_err(|_| WsError::io("library lock poisoned"))?;
        let mut st = conn.prepare("SELECT pkg_id, version, title, author, mode, installed_at FROM library_packages WHERE owner = ?1 ORDER BY title, pkg_id").map_err(db_err)?;
        let pkgs: Vec<(String, String, String, String, String, i64)> = st
            .query_map([owner], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?)))
            .map_err(db_err)?
            .collect::<rusqlite::Result<_>>()
            .map_err(db_err)?;
        let mut out = Vec::new();
        for (pkg_id, version, title, author, mode, installed_at) in pkgs {
            let controls: Vec<Value> = self.rows(&conn, owner, &pkg_id)?.into_iter().map(|r| r.summary()).collect();
            out.push(json!({ "pkg_id": pkg_id, "version": version, "title": title, "author": author, "mode": mode,
                             "installed_at": aiworkspace_core::value::format_utc_ms(installed_at), "controls": controls }));
        }
        Ok(json!({ "ok": true, "packages": out }))
    }

    /// `ext.uninstall`: the package leaves the library (Workspaces keep what they imported, D4).
    pub fn uninstall(&self, owner: &str, pkg_id: &str) -> WsResult<Value> {
        let conn = self.conn.lock().map_err(|_| WsError::io("library lock poisoned"))?;
        let version: Option<String> = conn.query_row("SELECT version FROM library_packages WHERE owner = ?1 AND pkg_id = ?2", params![owner, pkg_id], |r| r.get(0)).optional().map_err(db_err)?;
        let Some(version) = version else { return Err(WsError::not_found(format!("{pkg_id} is not in your library"))) };
        self.unbind_package(&conn, owner, pkg_id, &version)?;
        conn.execute("DELETE FROM library_controls WHERE owner = ?1 AND pkg_id = ?2", params![owner, pkg_id]).map_err(db_err)?;
        conn.execute("DELETE FROM library_packages WHERE owner = ?1 AND pkg_id = ?2", params![owner, pkg_id]).map_err(db_err)?;
        Ok(json!({ "ok": true, "removed": pkg_id, "version": version }))
    }

    /// The controls of a package with their files (objects in the named store) and the provenance an import records.
    /// With `with_deps`, what the chosen controls build on comes along (marked `implicit`), each once.
    pub fn controls(&self, owner: &str, pkg_id: &str, idents: Option<&[String]>, with_deps: bool) -> WsResult<Vec<LibraryControl>> {
        let conn = self.conn.lock().map_err(|_| WsError::io("library lock poisoned"))?;
        let version: Option<String> = conn.query_row("SELECT version FROM library_packages WHERE owner = ?1 AND pkg_id = ?2", params![owner, pkg_id], |r| r.get(0)).optional().map_err(db_err)?;
        let Some(version) = version else { return Err(WsError::not_found(format!("{pkg_id} is not in your library"))) };
        let rows = self.rows(&conn, owner, pkg_id)?;
        let mut wanted: Vec<(String, bool)> = idents.map(<[String]>::to_vec).unwrap_or_else(|| rows.iter().map(|r| r.ident.clone()).collect()).into_iter().map(|i| (i, false)).collect();
        let mut out: Vec<LibraryControl> = Vec::new();
        let mut i = 0;
        while i < wanted.len() {
            let (want, implicit) = wanted[i].clone();
            i += 1;
            if out.iter().any(|c| c.summary["ident"] == want.as_str()) {
                continue;
            }
            let row = rows.iter().find(|r| r.ident == want).ok_or_else(|| WsError::not_found(format!("{pkg_id} has no control {want}")))?;
            let summary = row.summary();
            if with_deps {
                for dep in summary["deps"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    if !wanted.iter().any(|(w, _)| w == dep) {
                        wanted.push((dep.to_string(), true));
                    }
                }
            }
            out.push(LibraryControl {
                summary,
                provenance: json!({ "pkg_id": pkg_id, "version": version, "digest": row.digest }),
                files: row.files.iter().map(|f| (f["name"].as_str().unwrap_or("").to_string(), f["obj_id"].as_str().unwrap_or("").to_string())).collect(),
                implicit,
            });
        }
        Ok(out)
    }

    /// `ext.library_bundle`: a package's chosen controls with their bytes (base64), for a client that builds them
    /// into a Workspace of another Zone.
    pub fn bundle(&self, owner: &str, pkg_id: &str, idents: Option<&[String]>) -> WsResult<Value> {
        use base64::Engine;
        let mut controls = Vec::new();
        for c in self.controls(owner, pkg_id, idents, true)? {
            let mut files = Vec::new();
            for (name, oid) in &c.files {
                let bytes = self.objects.get_file(oid)?;
                files.push(json!({ "name": name, "obj_id": oid, "size": bytes.len(), "base64": base64::engine::general_purpose::STANDARD.encode(&bytes) }));
            }
            let mut v = c.summary.clone();
            v["files"] = json!(files);
            v["provenance"] = c.provenance.clone();
            controls.push(v);
        }
        Ok(json!({ "ok": true, "pkg_id": pkg_id, "controls": controls }))
    }

    /// The files of a library control read back for a build.
    pub fn sources(&self, control: &LibraryControl) -> WsResult<Vec<SourceFile>> {
        control.files.iter().map(|(name, oid)| Ok(SourceFile { name: name.clone(), obj_id: oid.clone(), bytes: self.objects.get_file(oid)? })).collect()
    }
}

struct ControlRow {
    ident: String,
    name: String,
    kind: String,
    version: i64,
    digest: String,
    dir: String,
    info: Value,
    files: Vec<Value>,
}

impl ControlRow {
    fn summary(&self) -> Value {
        json!({ "ident": self.ident, "name": self.name, "kind": self.kind, "version": self.version, "digest": self.digest, "dir": self.dir,
                "title": self.info.get("title"), "description": self.info.get("description"), "catalog": self.info.get("catalog"),
                "deps": self.info.get("deps").filter(|d| d.is_array()).cloned().unwrap_or(json!([])) })
    }
}

/// A control of the library as an import needs it.
#[derive(Debug, Clone)]
pub struct LibraryControl {
    pub summary: Value,
    pub provenance: Value,
    /// `(name, obj_id)` of its files.
    pub files: Vec<(String, String)>,
    /// Brought along as a dependency of a chosen control.
    pub implicit: bool,
}
