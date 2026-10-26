//! Server configuration. v1 is CLI-args-only (nfs_server.md N2: 静态配置起步).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ExportRoot {
    /// Logical name: becomes the first path segment under current-Zone `cyfs:///`.
    pub id: String,
    /// Absolute local directory that backs this root.
    pub path: PathBuf,
}

/// How requests are authenticated (see auth.rs).
#[derive(Debug, Clone, Default)]
pub enum AuthMode {
    /// Standalone without an auth config: every request acts with full access.
    #[default]
    Disabled,
    /// BuckyOS service mode: verify-hub session tokens via the service runtime.
    Runtime,
    /// Standalone tests: tokens signed by a fixed key, `zone_owner` gets full access.
    TestKey { public_key: String, zone_owner: String },
}

/// Where share snapshots go (share/snapshot.rs).
#[derive(Debug, Clone, Default)]
pub enum NamedStoreSource {
    /// No named store: sharing reports itself blocked.
    #[default]
    None,
    /// BuckyOS service mode: the node's shared store ($BUCKYOS_ROOT/storage/named_store.json).
    Runtime,
    /// Standalone/tests: a single local bucket in this directory.
    Dir(PathBuf),
}

/// Share module settings (notepads/share-content-mgr-tech-design-todo.md).
#[derive(Debug, Clone)]
pub struct ShareConfig {
    /// Origin of share links, e.g. `https://share.<zone>`; links are `<base>/s/{token}`.
    pub base_url: Option<String>,
    /// Origin serving `srv/publish` anonymously, e.g. `https://public.<zone>`.
    pub public_base_url: Option<String>,
    /// Zone host; requests whose host is `share.<zone>` only reach `/s/*`.
    pub zone_host: Option<String>,
    /// Snapshots up to this many bytes finish inside the request (D7: 64 MiB).
    pub sync_threshold: u64,
    pub max_total_size: u64,
    pub max_items: u64,
}

impl Default for ShareConfig {
    fn default() -> Self {
        ShareConfig {
            base_url: None,
            public_base_url: None,
            zone_host: None,
            sync_threshold: 64 * 1024 * 1024,
            max_total_size: 64 * 1024 * 1024 * 1024,
            max_items: 20_000,
        }
    }
}

/// NDM gateway settings (ndm/).
#[derive(Debug, Clone)]
pub struct NdmConfig {
    /// An unfinished upload session idle this long is discarded with its staged bytes.
    pub session_ttl_secs: u64,
    /// Staged bytes of unfinished uploads per caller; older sessions are evicted first.
    pub principal_quota: u64,
}

impl Default for NdmConfig {
    fn default() -> Self {
        NdmConfig { session_ttl_secs: 3600, principal_quota: 512 * 1024 * 1024 }
    }
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AuthTestConfig {
    pub public_key: String,
    pub zone_owner: String,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub listen: String,
    /// Directory holding filedb.sqlite and the upload staging area.
    pub data_dir: PathBuf,
    pub exports: Vec<ExportRoot>,
    /// Reconciler scan interval; 0 disables the background loop.
    pub scan_interval_secs: u64,
    /// Enables `POST /nfs/v1/debug/{action}` (test/dev only).
    pub debug_api: bool,
    pub max_list: usize,
    pub max_batch: usize,
    pub replay_window: u64,
    pub lease_ttl_secs: u64,
    pub copy_test: Option<buckyos_api::NfsCopyTestConfig>,
    pub auth: AuthMode,
    pub named_store: NamedStoreSource,
    pub share: ShareConfig,
    pub ndm: NdmConfig,
    /// Standalone: what each app declares (`--bucket-specs`); service mode reads app specs.
    pub bucket_specs: std::collections::HashMap<String, crate::bucket::spec::AppBuckets>,
    /// The GC host may evict class-0 named-store objects (off until every writer retains).
    pub gc_evict: bool,
}

impl ServerConfig {
    pub fn new(data_dir: PathBuf, exports: Vec<ExportRoot>) -> Self {
        ServerConfig {
            listen: "127.0.0.1:3260".to_string(),
            data_dir,
            exports,
            scan_interval_secs: 0,
            debug_api: false,
            max_list: 1000,
            max_batch: 64,
            replay_window: 128,
            lease_ttl_secs: 600,
            copy_test: None,
            auth: AuthMode::Disabled,
            named_store: NamedStoreSource::None,
            share: ShareConfig::default(),
            ndm: NdmConfig::default(),
            bucket_specs: Default::default(),
            gc_evict: false,
        }
    }

    pub fn root(&self, id: &str) -> Option<&ExportRoot> {
        self.exports.iter().find(|r| r.id == id)
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("filedb.sqlite")
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.data_dir.join("staging")
    }

    pub fn ndm_staging_dir(&self) -> PathBuf {
        self.data_dir.join("ndm_uploads")
    }
}

/// Builds the buckyos-mode config: the whole `cyfs:///` namespace is backed
/// by `data_root` ($BUCKYOS_ROOT/data) — every visible first-level directory
/// becomes an export root — and filedb + staging live in `data_root/.fsdb`
/// (dot-prefixed, so the db itself never shows up in the namespace).
///
/// The export list is a startup snapshot: directories created directly under
/// `data_root` after start appear on the next service restart.
pub fn config_from_data_root(data_root: &Path) -> std::io::Result<ServerConfig> {
    std::fs::create_dir_all(data_root)?;
    let mut exports = Vec::new();
    for entry in std::fs::read_dir(data_root)? {
        let entry = entry?;
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue, // non-UTF-8 name: cannot be addressed over the protocol
        };
        if name.starts_with('.') {
            continue;
        }
        if !entry.file_type()?.is_dir() {
            continue;
        }
        exports.push(ExportRoot { id: name, path: entry.path() });
    }
    exports.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(ServerConfig::new(data_root.join(".fsdb"), exports))
}

/// Parses `name=/abs/path` export specs from the CLI.
pub fn parse_export_spec(spec: &str) -> Result<ExportRoot, String> {
    let (id, path) = spec
        .split_once('=')
        .ok_or_else(|| format!("invalid export spec '{}', expected name=/abs/path", spec))?;
    if id.is_empty()
        || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!(
            "invalid export root name '{}': [A-Za-z0-9_-]+ only",
            id
        ));
    }
    let p = PathBuf::from(path);
    if !p.is_absolute() {
        return Err(format!("export path '{}' must be absolute", path));
    }
    Ok(ExportRoot { id: id.to_string(), path: p })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_root_discovery() {
        let tmp = std::env::temp_dir().join(format!("nfs_cfg_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("home")).unwrap();
        std::fs::create_dir_all(tmp.join("srv")).unwrap();
        std::fs::create_dir_all(tmp.join(".fsdb")).unwrap();
        std::fs::create_dir_all(tmp.join(".hidden")).unwrap();
        std::fs::write(tmp.join("stray.txt"), b"x").unwrap();

        let cfg = config_from_data_root(&tmp).unwrap();
        let ids: Vec<&str> = cfg.exports.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["home", "srv"]); // sorted; dot-dirs and files skipped
        assert_eq!(cfg.data_dir, tmp.join(".fsdb"));
        assert_eq!(cfg.db_path(), tmp.join(".fsdb").join("filedb.sqlite"));

        // A missing data root is created rather than rejected.
        let fresh = tmp.join("nested").join("data");
        let cfg = config_from_data_root(&fresh).unwrap();
        assert!(cfg.exports.is_empty());
        assert!(fresh.is_dir());
        std::fs::remove_dir_all(&tmp).unwrap();
    }

    #[test]
    fn export_spec_parsing() {
        let r = parse_export_spec("home=/tmp/x").unwrap();
        assert_eq!(r.id, "home");
        assert_eq!(r.path, PathBuf::from("/tmp/x"));
        assert!(parse_export_spec("noequals").is_err());
        assert!(parse_export_spec("bad name=/x").is_err());
        assert!(parse_export_spec("rel=relative/path").is_err());
    }
}
