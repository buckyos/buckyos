//! Server configuration. v1 is CLI-args-only (nfs_server.md N2: 静态配置起步).

use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct ExportRoot {
    /// Logical name: becomes the first path segment under `dfs://`.
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
    fn export_spec_parsing() {
        let r = parse_export_spec("home=/tmp/x").unwrap();
        assert_eq!(r.id, "home");
        assert_eq!(r.path, PathBuf::from("/tmp/x"));
        assert!(parse_export_spec("noequals").is_err());
        assert!(parse_export_spec("bad name=/x").is_err());
        assert!(parse_export_spec("rel=relative/path").is_err());
    }
}
