//! `ext.pack` (扩展控件 P1, Q20): a bundle written as an `aiws` PIKG by the service, with the rules of the CLI
//! (buckyos-websdk `cli/modules/pikg_archive.ts` `createDeterministicTarGz` / `writeStoredZip`, `pikg.ts` build and
//! pack): a ustar archive of the bundle (directories first in each folder, names sorted, files 0644, directories
//! 0755, owner 0, mtime 0, no links), gzip, then a stored ZIP of `APPDOC.json`, `PACKAGE_META.json` and
//! `aiws.tar.gz`. The AppDoc is `app_type: aiws` with one sub-package `aiws` (package `all.aiws.<AppId>`); it is not
//! signed (the author signs with the CLI, `pikg sign`). The same bundle and timestamp give the same bytes. The gzip
//! layer is flate2's (level 6); the CLI's is Node zlib's, so the two archives match up to `aiws.tar.gz`'s
//! compressed bytes: the tar inside, the manifest digests and the object ids are the same.

use crate::library::{app_id_from_did, AIWS_SUB_PACKAGE};
use aiworkspace_core::canonical::sha256_hex;
use aiworkspace_core::{WsError, WsResult};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::io::Write;

const BLOCK: usize = 512;
/// An AppDoc stays valid this long (the CLI's `APP_DOCUMENT_LIFETIME_SECONDS`).
const APP_DOCUMENT_LIFETIME_SECONDS: u64 = 5 * 365 * 24 * 60 * 60;
pub const APPDOC_ENTRY: &str = "APPDOC.json";
pub const PACKAGE_META_ENTRY: &str = "PACKAGE_META.json";
pub const PACKAGE_META_SCHEMA: &str = "buckyos.pikg.package-meta.v1";

/// What a package is called and who owns it.
#[derive(Debug, Clone)]
pub struct PackSpec {
    /// The App name (`[a-z0-9][a-z0-9-]*`): the AppDoc `did` is `did:bns:<name>.<owner label>`.
    pub name: String,
    pub version: String,
    pub show_name: String,
    /// `did:bns:<label>` of the owner (and author).
    pub owner: String,
    /// Unix seconds of `create_time` (the newest build of the bundle's controls: the same bundle packs the same).
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct Packed {
    pub bytes: Vec<u8>,
    pub app_did: String,
    /// The package id a library records: the AppId of `app_did`.
    pub pkg_id: String,
    pub app_doc_object_id: String,
    pub payload_digest: String,
    /// `sha256:` of the uncompressed tar (what the CLI's archive holds too).
    pub tar_digest: String,
}

fn invalid(detail: impl Into<String>) -> WsError {
    WsError::invalid_op(detail)
}

/// `did:bns:<name>.<owner label>` (the CLI's `deriveAppDid`).
pub fn app_did(name: &str, owner: &str) -> WsResult<String> {
    let label = owner.strip_prefix("did:bns:").filter(|l| {
        !l.is_empty() && l.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') && !l.starts_with('-') && !l.ends_with('-')
    });
    let label = label.ok_or_else(|| invalid(format!("owner {owner} must be a single-label did:bns DID")))?;
    let name_ok = !name.is_empty() && name.len() <= 63 && name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-') && !name.starts_with('-') && !name.ends_with('-');
    if !name_ok {
        return Err(invalid(format!("package name {name:?} must be lowercase letters, digits and '-'")));
    }
    Ok(format!("did:bns:{name}.{label}"))
}

fn octal(header: &mut [u8], at: usize, len: usize, value: u64) -> WsResult<()> {
    let text = format!("{value:0width$o}", width = len - 1);
    if text.len() >= len {
        return Err(WsError::limit("a tar field overflows"));
    }
    header[at..at + text.len()].copy_from_slice(text.as_bytes());
    header[at + text.len()] = 0;
    Ok(())
}

/// ustar `name` / `prefix` of a path (the CLI's `splitUstarPath`).
fn split_path(path: &str) -> WsResult<(String, String)> {
    if path.len() <= 100 {
        return Ok((path.to_string(), String::new()));
    }
    let dir = path.ends_with('/');
    let core = path.trim_end_matches('/');
    let parts: Vec<&str> = core.split('/').collect();
    for i in (1..parts.len()).rev() {
        let prefix = parts[..i].join("/");
        let name = format!("{}{}", parts[i..].join("/"), if dir { "/" } else { "" });
        if prefix.len() <= 155 && name.len() <= 100 {
            return Ok((name, prefix));
        }
    }
    Err(invalid(format!("{path} is too long for a portable tar")))
}

fn tar_header(path: &str, size: u64, dir: bool) -> WsResult<[u8; BLOCK]> {
    let mut h = [0u8; BLOCK];
    let (name, prefix) = split_path(path)?;
    h[..name.len()].copy_from_slice(name.as_bytes());
    octal(&mut h, 100, 8, if dir { 0o755 } else { 0o644 })?;
    octal(&mut h, 108, 8, 0)?;
    octal(&mut h, 116, 8, 0)?;
    octal(&mut h, 124, 12, size)?;
    octal(&mut h, 136, 12, 0)?;
    h[148..156].fill(b' ');
    h[156] = if dir { b'5' } else { b'0' };
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix.as_bytes());
    let sum: u64 = h.iter().map(|b| *b as u64).sum();
    let text = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(text.as_bytes());
    Ok(h)
}

/// The deterministic tar of a bundle `{ path: bytes }`: a folder's entries sorted by name, each sub-folder as an entry
/// `<path>/` followed by its content, two zero blocks at the end.
pub fn bundle_tar(files: &BTreeMap<String, Vec<u8>>) -> WsResult<Vec<u8>> {
    #[derive(Default)]
    struct Dir<'a> {
        dirs: BTreeMap<&'a str, Dir<'a>>,
        files: BTreeMap<&'a str, &'a [u8]>,
    }
    let mut root = Dir::default();
    for (path, bytes) in files {
        if path.is_empty() || path.starts_with('/') || path.split('/').any(|s| s.is_empty() || s == "." || s == "..") {
            return Err(invalid(format!("bundle path {path:?} is not a safe relative path")));
        }
        let mut segs: Vec<&str> = path.split('/').collect();
        let file = segs.pop().unwrap_or_default();
        let mut d = &mut root;
        for s in segs {
            d = d.dirs.entry(s).or_default();
        }
        d.files.insert(file, bytes);
    }
    fn walk(d: &Dir, prefix: &str, out: &mut Vec<u8>) -> WsResult<()> {
        let mut names: Vec<(&str, bool)> = d.dirs.keys().map(|k| (*k, true)).chain(d.files.keys().map(|k| (*k, false))).collect();
        // the CLI sorts by JavaScript string order: UTF-16 code units; identical to bytes for the names a bundle has
        names.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
        for (name, is_dir) in names {
            let path = if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") };
            if is_dir {
                out.extend_from_slice(&tar_header(&format!("{path}/"), 0, true)?);
                walk(&d.dirs[name], &path, out)?;
            } else {
                let bytes = d.files[name];
                out.extend_from_slice(&tar_header(&path, bytes.len() as u64, false)?);
                out.extend_from_slice(bytes);
                out.resize(out.len() + (BLOCK - bytes.len() % BLOCK) % BLOCK, 0);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(&root, "", &mut out)?;
    out.resize(out.len() + BLOCK * 2, 0);
    Ok(out)
}

fn gzip(bytes: &[u8]) -> WsResult<Vec<u8>> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(6));
    enc.write_all(bytes).map_err(|e| WsError::io(e.to_string()))?;
    enc.finish().map_err(|e| WsError::io(e.to_string()))
}

/// A stored (uncompressed) ZIP with UTF-8 names and no timestamps (the CLI's `writeStoredZip`, without ZIP64: a
/// control package stays far below 4 GiB).
pub fn stored_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, bytes) in entries {
        let crc = {
            let mut h = flate2::Crc::new();
            h.update(bytes);
            h.sum()
        };
        let offset = out.len() as u32;
        let size = bytes.len() as u32;
        out.extend_from_slice(&0x04034b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&0x0800u16.to_le_bytes());
        out.extend_from_slice(&[0u8; 6]); // method stored, no time, no date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(bytes);
        central.extend_from_slice(&0x02014b50u32.to_le_bytes());
        central.extend_from_slice(&((3u16 << 8) | 45).to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&0x0800u16.to_le_bytes());
        central.extend_from_slice(&[0u8; 6]);
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&[0u8; 8]); // extra, comment, disk, internal attributes
        central.extend_from_slice(&(0o100644u32 << 16).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let central_offset = out.len() as u32;
    let central_size = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x06054b50u32.to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&central_size.to_le_bytes());
    out.extend_from_slice(&central_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

fn pretty(v: &Value) -> Vec<u8> {
    let mut s = serde_json::to_string_pretty(v).unwrap_or_default();
    s.push('\n');
    s.into_bytes()
}

/// Pack a bundle (`aiws-manifest.json` plus `controls/<dir>/<file>`) as an `aiws` PIKG.
pub fn pack(spec: &PackSpec, files: &BTreeMap<String, Vec<u8>>) -> WsResult<Packed> {
    if spec.version.is_empty() || spec.version.len() > 64 {
        return Err(invalid("package version: 1…64 characters"));
    }
    let app_did = app_did(&spec.name, &spec.owner)?;
    let pkg_id = app_id_from_did(&app_did)?;
    let tar = bundle_tar(files)?;
    let payload = gzip(&tar)?;
    let payload_hex = sha256_hex(&payload);
    let payload_digest = format!("sha256:{payload_hex}");
    let hash = hex::decode(&payload_hex).map_err(|e| WsError::io(e.to_string()))?;
    let content = ndn_lib::ChunkId::from_mix256_result(payload.len() as u64, &hash).to_string();
    let ts = spec.timestamp;
    let package_name = format!("all.{AIWS_SUB_PACKAGE}.{pkg_id}");
    let package_value = json!({ "name": package_name, "author": spec.owner, "owner": spec.owner, "create_time": ts, "last_update_time": ts,
                                "size": payload.len(), "content": content, "version": spec.version });
    let package_objid = ndn_lib::build_named_object_by_json(ndn_lib::OBJ_TYPE_PKG, &package_value).0.to_string();
    let mut pkg_list = Map::new();
    pkg_list.insert(AIWS_SUB_PACKAGE.into(), json!({ "pkg_id": format!("{package_name}#{}", spec.version), "pkg_objid": package_objid, "required": true }));
    let app_doc = json!({
        "schema_version": 1, "doc_type": "app", "did": app_did, "name": spec.name, "version": spec.version, "app_type": "aiws",
        "author": spec.owner, "owner": spec.owner, "controller": spec.owner, "create_time": ts, "last_update_time": ts,
        "exp": ts + APP_DOCUMENT_LIFETIME_SECONDS, "categories": ["aiws"], "pkg_list": pkg_list, "show_name": spec.show_name,
        "selector_type": "single", "service_config_tips": {},
    });
    let app_doc_object_id = ndn_lib::build_named_object_by_json("appdoc", &app_doc).0.to_string();
    let mut objects = Map::new();
    objects.insert(package_objid, package_value);
    let mut index = Map::new();
    index.insert(payload_digest.clone(), json!({ "sub_pkg_name": AIWS_SUB_PACKAGE, "path": format!("{AIWS_SUB_PACKAGE}.tar.gz"), "format": "tar.gz", "size": payload.len(), "digest": payload_digest }));
    let meta = json!({ "@schema": PACKAGE_META_SCHEMA, "app_doc_id": app_doc_object_id, "package_objects": objects, "content_index": index });
    let (app_bytes, meta_bytes) = (pretty(&app_doc), pretty(&meta));
    let payload_name = format!("{AIWS_SUB_PACKAGE}.tar.gz");
    let bytes = stored_zip(&[(APPDOC_ENTRY, &app_bytes), (PACKAGE_META_ENTRY, &meta_bytes), (&payload_name, &payload)]);
    Ok(Packed { bytes, app_did, pkg_id, app_doc_object_id, payload_digest, tar_digest: format!("sha256:{}", sha256_hex(&tar)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tar_lists_folders_before_their_files_in_name_order() {
        let mut files = BTreeMap::new();
        files.insert("controls/b/x.txt".to_string(), b"bb".to_vec());
        files.insert("controls/a/component.json".to_string(), b"{}".to_vec());
        files.insert("aiws-manifest.json".to_string(), b"{}".to_vec());
        let tar = bundle_tar(&files).unwrap();
        let names: Vec<String> = tar::Archive::new(tar.as_slice()).entries().unwrap().map(|e| e.unwrap().path().unwrap().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["aiws-manifest.json", "controls/", "controls/a/", "controls/a/component.json", "controls/b/", "controls/b/x.txt"]);
        let mut archive = tar::Archive::new(tar.as_slice());
        let e = archive.entries().unwrap().next().unwrap().unwrap();
        assert_eq!((e.header().mode().unwrap(), e.header().mtime().unwrap(), e.header().uid().unwrap()), (0o644, 0, 0));
        assert_eq!(bundle_tar(&files).unwrap(), tar, "deterministic");
    }

    #[test]
    fn app_dids_follow_the_cli() {
        assert_eq!(app_did("hello-controls", "did:bns:alice").unwrap(), "did:bns:hello-controls.alice");
        assert!(app_did("Hello", "did:bns:alice").is_err());
        assert!(app_did("x", "did:web:alice.example").is_err());
    }
}
