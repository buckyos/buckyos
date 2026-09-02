//! Server configuration. v1 is CLI-args-only (nfs_server.md N2: 静态配置起步).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ExportRoot {
    /// Logical name: becomes the first path segment under current-Zone `cyfs:///`.
    pub id: String,
    /// Absolute local directory that backs this root.
    pub path: PathBuf,
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
