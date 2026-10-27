//! Files projection of buckets (design §4.5.4, D11): the `data` buckets of a
//! user whose tier is `documents` appear read-only under
//! `cyfs:///home/<user>/Apps/<App>/<bucket>/<key...>`, `collected` ones under
//! `Collected/`. Listing and reading go to the bucket index; deleting a
//! projected file unbinds its key; nothing else writes here.
//!
//! The namespace is synchronous; bucket reads by the internal caller only await
//! an identity check that is already decided, so they are driven inline.

use crate::acl::{Actor, Level, HOME_ROOT};
use crate::error::*;
use crate::fsutil::validate_name;
use crate::namespace::{EntryRow, Node};
use crate::state::AppState;
use crate::types::WantMask;
use bucket_store::{Binding, BucketInfo, BucketKind, BucketResult, BucketScope, BucketStore, Caller, DataPolicy, ListRequest};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub const APPS_DIR: &str = "Apps";
pub const COLLECTED_DIR: &str = "Collected";
const ORPHAN_SUFFIX: &str = "（已卸载）";
/// A key `a` next to keys under `a/`: the directory keeps `a`, the file shows `a~file`.
const FILE_SUFFIX: &str = "~file";
const LIST_PAGE: usize = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Apps,
    Collected,
}

impl Section {
    pub const ALL: [Section; 2] = [Section::Apps, Section::Collected];

    pub fn from_dir(name: &str) -> Option<Self> {
        match name {
            APPS_DIR => Some(Section::Apps),
            COLLECTED_DIR => Some(Section::Collected),
            _ => None,
        }
    }

    pub fn dir(&self) -> &'static str {
        match self {
            Section::Apps => APPS_DIR,
            Section::Collected => COLLECTED_DIR,
        }
    }

    fn policy(&self) -> DataPolicy {
        match self {
            Section::Apps => DataPolicy::Documents,
            Section::Collected => DataPolicy::Collected,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppFolder {
    pub label: String,
    pub appid: String,
    /// `(folder name, bucket)`.
    pub buckets: Vec<(String, BucketInfo)>,
}

#[derive(Debug, Clone)]
pub enum ProjAt {
    Section(Section),
    App(AppFolder),
    /// A bucket (`prefix` empty) or a key prefix ending in `/`.
    Dir { bucket: BucketInfo, prefix: String },
    File { bucket: BucketInfo, binding: Binding },
}

/// A projected node; `rel` is its path under the `home` export root.
#[derive(Debug, Clone)]
pub struct Projection {
    pub user: String,
    pub rel: String,
    pub at: ProjAt,
}

impl Projection {
    pub fn is_file(&self) -> bool {
        matches!(self.at, ProjAt::File { .. })
    }

    pub fn name(&self) -> String {
        self.rel.rsplit('/').next().unwrap_or(&self.rel).to_string()
    }

    fn child_rel(&self, name: &str) -> String {
        format!("{}/{}", self.rel, name)
    }
}

pub(crate) fn drive<F: std::future::Future>(f: F) -> F::Output {
    futures::executor::block_on(f)
}

fn bucket_err(e: bucket_store::BucketError) -> NfsError {
    match e {
        bucket_store::BucketError::NoSuchKey(k) => not_found(format!("'{k}' not found")),
        bucket_store::BucketError::BucketNotFound(b) => not_found(format!("bucket '{b}' not found")),
        e => internal(format!("bucket index: {e}")),
    }
}

fn folder_name(raw: &str, fallback: &str) -> String {
    let name: String = raw.trim().chars().map(|c| if matches!(c, '/' | '\\' | '\0') { '_' } else { c }).collect();
    if validate_name(&name).is_ok() {
        name
    } else {
        fallback.to_string()
    }
}

const KEY_SEGMENT: &percent_encoding::AsciiSet =
    &percent_encoding::NON_ALPHANUMERIC.remove(b'-').remove(b'.').remove(b'_').remove(b'~');

/// `/bucket/v1/<bucket_id>/<key>`, segments percent-encoded.
pub fn data_url(bucket: &BucketInfo, key: &str) -> String {
    let encoded: Vec<String> = key.split('/').map(|s| percent_encoding::utf8_percent_encode(s, KEY_SEGMENT).to_string()).collect();
    format!("{}{}/{}", buckyos_api::BUCKET_DATA_PATH, bucket.bucket_id, encoded.join("/"))
}

struct Listing {
    dirs: Vec<String>,
    files: Vec<Binding>,
}

fn list_level(store: &BucketStore, bucket: &BucketInfo, prefix: &str) -> BucketResult<Listing> {
    let caller = Caller::system();
    let mut req = ListRequest { prefix: prefix.to_string(), delimiter: Some("/".into()), start_after: None, limit: Some(LIST_PAGE) };
    let mut out = Listing { dirs: Vec::new(), files: Vec::new() };
    loop {
        let page = drive(store.list_objects(&bucket.bucket_id, &caller, &req))?;
        out.dirs.extend(page.common_prefixes.iter().filter_map(|p| {
            p.strip_prefix(prefix).map(|rest| rest.trim_end_matches('/').to_string())
        }));
        out.files.extend(page.items);
        match page.next_start_after {
            Some(next) => req.start_after = Some(next),
            None => return Ok(out),
        }
    }
}

fn has_keys_under(store: &BucketStore, bucket: &BucketInfo, prefix: &str) -> BucketResult<bool> {
    let req = ListRequest { prefix: prefix.to_string(), delimiter: None, start_after: None, limit: Some(1) };
    Ok(!drive(store.list_objects(&bucket.bucket_id, &Caller::system(), &req))?.items.is_empty())
}

fn current(store: &BucketStore, bucket: &BucketInfo, key: &str) -> BucketResult<Option<Binding>> {
    match drive(store.get(&bucket.bucket_id, &Caller::system(), key, None)) {
        Ok(b) => Ok(Some(b)),
        Err(bucket_store::BucketError::NoSuchKey(_)) | Err(bucket_store::BucketError::InvalidKey(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

impl AppState {
    fn projection_store(&self) -> Option<Arc<BucketStore>> {
        self.buckets.loaded()
    }

    /// The app folders of one section for a user, labels unique and stable for a given set of apps.
    pub fn projection_folders(&self, user: &str, section: Section) -> Vec<AppFolder> {
        let Some(store) = self.projection_store() else { return Vec::new() };
        let Ok(buckets) = store.list_buckets(Some(user), None, Some(BucketScope::User)) else { return Vec::new() };
        let mut by_app: BTreeMap<String, Vec<BucketInfo>> = BTreeMap::new();
        for b in buckets.into_iter().filter(|b| b.kind == BucketKind::Data && b.data_policy == section.policy()) {
            by_app.entry(b.appid.clone()).or_default().push(b);
        }
        let mut used = HashSet::new();
        let mut out = Vec::new();
        for (appid, buckets) in by_app {
            let all_orphaned = buckets.iter().all(|b| b.orphaned_at.is_some());
            let mut label = folder_name(&self.buckets.show_name(&appid).unwrap_or_else(|| appid.clone()), &appid);
            if used.contains(&label) {
                label = format!("{label} ({appid})");
            }
            used.insert(label.clone());
            if all_orphaned {
                label.push_str(ORPHAN_SUFFIX);
            }
            let buckets = buckets
                .into_iter()
                .map(|b| {
                    let name = if b.orphaned_at.is_some() && !all_orphaned { format!("{}{ORPHAN_SUFFIX}", b.name) } else { b.name.clone() };
                    (name, b)
                })
                .collect();
            out.push(AppFolder { label, appid, buckets });
        }
        out.sort_by(|a, b| a.label.as_bytes().cmp(b.label.as_bytes()));
        out
    }

    /// The projected node at `rel` under the home root, when `rel` is inside
    /// `<user>/Apps` or `<user>/Collected` and that section has buckets.
    pub fn projection_at(&self, root: &str, rel: &str) -> NfsResult<Option<Projection>> {
        if root != HOME_ROOT {
            return Ok(None);
        }
        let segs: Vec<&str> = rel.split('/').collect();
        let (Some(user), Some(section)) = (segs.first(), segs.get(1).and_then(|s| Section::from_dir(s))) else { return Ok(None) };
        let folders = self.projection_folders(user, section);
        if folders.is_empty() {
            return Ok(None);
        }
        let mut node = Projection { user: user.to_string(), rel: format!("{user}/{}", section.dir()), at: ProjAt::Section(section) };
        for name in &segs[2..] {
            node = self.projection_child(&node, name)?;
        }
        Ok(Some(node))
    }

    pub fn projection_child(&self, parent: &Projection, name: &str) -> NfsResult<Projection> {
        let child = |at| Projection { user: parent.user.clone(), rel: parent.child_rel(name), at };
        match &parent.at {
            ProjAt::Section(section) => self
                .projection_folders(&parent.user, *section)
                .into_iter()
                .find(|f| f.label == name)
                .map(|f| child(ProjAt::App(f)))
                .ok_or_else(|| not_found(format!("'{name}' not found"))),
            ProjAt::App(folder) => folder
                .buckets
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, b)| child(ProjAt::Dir { bucket: b.clone(), prefix: String::new() }))
                .ok_or_else(|| not_found(format!("'{name}' not found"))),
            ProjAt::Dir { bucket, prefix } => {
                let store = self.projection_store().ok_or_else(|| stale("bucket service unavailable"))?;
                let dir_prefix = format!("{prefix}{name}/");
                if has_keys_under(&store, bucket, &dir_prefix).map_err(bucket_err)? {
                    return Ok(child(ProjAt::Dir { bucket: bucket.clone(), prefix: dir_prefix }));
                }
                if let Some(binding) = current(&store, bucket, &format!("{prefix}{name}")).map_err(bucket_err)? {
                    return Ok(child(ProjAt::File { bucket: bucket.clone(), binding }));
                }
                if let Some(base) = name.strip_suffix(FILE_SUFFIX) {
                    if has_keys_under(&store, bucket, &format!("{prefix}{base}/")).map_err(bucket_err)? {
                        if let Some(binding) = current(&store, bucket, &format!("{prefix}{base}")).map_err(bucket_err)? {
                            return Ok(child(ProjAt::File { bucket: bucket.clone(), binding }));
                        }
                    }
                }
                Err(not_found(format!("'{name}' not found")))
            }
            ProjAt::File { .. } => Err(NfsError::new(ErrorCode::NotAContainer, "a projected file is not a container")),
        }
    }

    /// Sections a user's home shows next to its native entries.
    pub fn projection_sections(&self, user: &str) -> Vec<Section> {
        Section::ALL.into_iter().filter(|s| !self.projection_folders(user, *s).is_empty()).collect()
    }

    pub fn projection_entries(&self, actor: &Actor, node: &Projection, want: &WantMask) -> NfsResult<Vec<EntryRow>> {
        let mut children = Vec::new();
        match &node.at {
            ProjAt::Section(section) => {
                for f in self.projection_folders(&node.user, *section) {
                    children.push(Projection { user: node.user.clone(), rel: node.child_rel(&f.label), at: ProjAt::App(f) });
                }
            }
            ProjAt::App(folder) => {
                for (name, b) in &folder.buckets {
                    children.push(Projection {
                        user: node.user.clone(),
                        rel: node.child_rel(name),
                        at: ProjAt::Dir { bucket: b.clone(), prefix: String::new() },
                    });
                }
            }
            ProjAt::Dir { bucket, prefix } => {
                let store = self.projection_store().ok_or_else(|| stale("bucket service unavailable"))?;
                let listing = list_level(&store, bucket, prefix).map_err(bucket_err)?;
                let dirs: HashSet<&str> = listing.dirs.iter().map(String::as_str).collect();
                for d in &listing.dirs {
                    if validate_name(d).is_ok() {
                        children.push(Projection {
                            user: node.user.clone(),
                            rel: node.child_rel(d),
                            at: ProjAt::Dir { bucket: bucket.clone(), prefix: format!("{prefix}{d}/") },
                        });
                    }
                }
                for b in listing.files {
                    let base = b.key[prefix.len()..].to_string();
                    let name = if dirs.contains(base.as_str()) { format!("{base}{FILE_SUFFIX}") } else { base };
                    if validate_name(&name).is_ok() {
                        children.push(Projection { user: node.user.clone(), rel: node.child_rel(&name), at: ProjAt::File { bucket: bucket.clone(), binding: b } });
                    }
                }
            }
            ProjAt::File { .. } => return Err(NfsError::new(ErrorCode::NotAContainer, "a projected file is not a container")),
        }
        Ok(children.into_iter().map(|c| self.projection_row(actor, c, want)).collect())
    }

    pub(crate) fn projection_row(&self, actor: &Actor, p: Projection, want: &WantMask) -> EntryRow {
        let name = p.name();
        let (kind, size, mtime) = match &p.at {
            ProjAt::File { binding, .. } => ("file", binding.size, binding.created_at / 1000),
            _ => ("dir", 0, 0),
        };
        let entry_ref = format!("pe_{}", crate::bucket::projection::encode_rel(&p.rel));
        let node = Node::Projection(Box::new(p));
        let mut attrs = serde_json::Map::new();
        if want.has("base") {
            attrs.insert("size".into(), json!(size));
            attrs.insert("mtime".into(), json!(mtime));
            attrs.insert("flags".into(), json!(["read_only"]));
        }
        if let Node::Projection(p) = &node {
            if let ProjAt::File { binding, bucket } = &p.at {
                if want.has("ident") {
                    attrs.insert("etag".into(), json!(binding.etag));
                    if let Some(id) = &binding.obj_id {
                        attrs.insert("obj_id".into(), json!(id));
                    }
                }
                if want.has("access") {
                    attrs.insert("access_urls".into(), json!(self.projection_access_urls(p, bucket, binding)));
                }
            }
        }
        let target = json!({
            "ref": self.node_ref(&node),
            "copy_ref": Value::Null,
            "kind": kind,
            "attrs": Value::Object(attrs),
            "access_level": self.node_level(actor, &node),
        });
        EntryRow {
            name: name.clone(),
            kind: kind.to_string(),
            mtime,
            size,
            order_index: 0,
            json: json!({"entry_ref": entry_ref, "name": name, "binding": "projection", "target": target}),
        }
    }

    pub(crate) fn projection_access_urls(&self, p: &Projection, bucket: &BucketInfo, binding: &Binding) -> Vec<Value> {
        vec![
            json!({"kind": "fs", "url": crate::namespace::canonical_path(HOME_ROOT, &p.rel), "primary": true}),
            json!({"kind": "read", "url": data_url(bucket, &binding.key)}),
        ]
    }

    /// Revision of a projected container: changes when the buckets below change.
    pub fn projection_revision(&self, p: &Projection) -> String {
        let stat = |b: &BucketInfo| self.projection_store().and_then(|s| s.bucket(&b.bucket_id).ok()).map(|i| (i.key_count, i.version_count)).unwrap_or_default();
        let parts: Vec<String> = match &p.at {
            ProjAt::Section(section) => self
                .projection_folders(&p.user, *section)
                .iter()
                .flat_map(|f| f.buckets.iter().map(|(n, b)| format!("{}:{n}:{:?}", f.label, stat(b))))
                .collect(),
            ProjAt::App(folder) => folder.buckets.iter().map(|(n, b)| format!("{n}:{:?}", stat(b))).collect(),
            ProjAt::Dir { bucket, .. } => vec![format!("{:?}", stat(bucket))],
            ProjAt::File { binding, .. } => vec![binding.etag.clone()],
        };
        let digest = sha2::Sha256::digest(parts.join("\n").as_bytes());
        format!("b-{}", hex::encode(&digest[..8]))
    }

    /// `delete` of an entry of a projected directory: unbinds the key, or every
    /// key under a prefix with `recursive`. Buckets and app folders stay.
    pub async fn projection_delete(&self, actor: &Actor, parent: &Projection, name: &str, recursive: bool) -> NfsResult<Value> {
        let child = self.projection_child(parent, name)?;
        if self.acls.level(actor, HOME_ROOT, &child.rel) < Level::Write {
            return Err(crate::access::denied("write", actor));
        }
        let store = self.projection_store().ok_or_else(|| stale("bucket service unavailable"))?;
        let caller = Caller::system();
        match &child.at {
            ProjAt::File { bucket, binding } => {
                store.delete(&bucket.bucket_id, &caller, &binding.key, None, None).await.map_err(bucket_err)?;
                log::info!("Files: {:?} deleted {}/{}", actor.user_id(), bucket.bucket_id, binding.key);
            }
            ProjAt::Dir { bucket, prefix } if !prefix.is_empty() => {
                if !recursive {
                    return Err(NfsError::new(ErrorCode::NotEmpty, format!("directory '{name}' is not empty (pass recursive:true)")));
                }
                loop {
                    let req = ListRequest { prefix: prefix.clone(), delimiter: None, start_after: None, limit: Some(LIST_PAGE) };
                    let page = store.list_objects(&bucket.bucket_id, &caller, &req).await.map_err(bucket_err)?;
                    if page.items.is_empty() {
                        break;
                    }
                    for b in page.items {
                        store.delete(&bucket.bucket_id, &caller, &b.key, None, None).await.map_err(bucket_err)?;
                        log::info!("Files: {:?} deleted {}/{}", actor.user_id(), bucket.bucket_id, b.key);
                    }
                }
            }
            _ => return Err(NfsError::new(ErrorCode::PolicyDenied, "apps and their buckets are removed through the app, not from Files")),
        }
        Ok(json!({ "revision": self.projection_revision(parent) }))
    }
}

/// `bk_` node ids and `pe_` entry refs carry the projected path.
pub fn encode_rel(rel: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(rel.as_bytes())
}

pub fn decode_rel(s: &str) -> NfsResult<String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s).map_err(|_| stale("bad projected node id"))?;
    String::from_utf8(bytes).map_err(|_| stale("bad projected node id"))
}

use sha2::Digest;
