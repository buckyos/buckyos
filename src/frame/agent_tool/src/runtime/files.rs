use crate::{path_utils::resolve_path_from_root, AgentToolError};
use async_trait::async_trait;
use std::path::{Path, PathBuf};

#[async_trait]
pub trait FileBackend: Send + Sync + std::fmt::Debug {
    async fn resolve(
        &self,
        root: &Path,
        raw: &str,
        allowed: &[PathBuf],
    ) -> Result<PathBuf, AgentToolError>;
    async fn exists(&self, path: &Path) -> Result<bool, AgentToolError>;
    async fn read(&self, path: &Path) -> Result<Vec<u8>, AgentToolError>;
    async fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), AgentToolError>;
}

#[derive(Debug)]
pub struct LocalFileBackend;

fn canonical_missing(path: &Path) -> Result<PathBuf, AgentToolError> {
    let mut ancestor = path.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(&ancestor) {
            Ok(mut real) => {
                for part in missing.iter().rev() {
                    real.push(part);
                }
                return Ok(crate::path_utils::normalize_abs_path(&real));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(&ancestor).is_ok() {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "dangling symlink: {}",
                        ancestor.display()
                    )));
                }
                let name = ancestor
                    .file_name()
                    .ok_or_else(|| {
                        AgentToolError::InvalidArgs(format!("cannot resolve {}", path.display()))
                    })?
                    .to_os_string();
                missing.push(name);
                if !ancestor.pop() {
                    return Err(AgentToolError::InvalidArgs(
                        "path has no existing ancestor".into(),
                    ));
                }
            }
            Err(e) => return Err(AgentToolError::ExecFailed(e.to_string())),
        }
    }
}

pub fn check_allowed(path: &Path, roots: &[PathBuf]) -> Result<(), AgentToolError> {
    if roots.is_empty() || roots.iter().any(|root| path.starts_with(root)) {
        return Ok(());
    }
    Err(AgentToolError::InvalidArgs(format!(
        "path not allowed by policy: {}",
        path.display()
    )))
}

#[async_trait]
impl FileBackend for LocalFileBackend {
    async fn resolve(
        &self,
        root: &Path,
        raw: &str,
        allowed: &[PathBuf],
    ) -> Result<PathBuf, AgentToolError> {
        let lexical = resolve_path_from_root(root, raw)?;
        check_allowed(&lexical, allowed)?;
        let candidate = if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            root.join(raw)
        };
        let real = canonical_missing(&candidate)?;
        let roots = allowed
            .iter()
            .map(|r| canonical_missing(r))
            .collect::<Result<Vec<_>, _>>()?;
        check_allowed(&real, &roots)?;
        Ok(real)
    }
    async fn exists(&self, path: &Path) -> Result<bool, AgentToolError> {
        match tokio::fs::metadata(path).await {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(AgentToolError::ExecFailed(e.to_string())),
        }
    }
    async fn read(&self, path: &Path) -> Result<Vec<u8>, AgentToolError> {
        tokio::fs::read(path)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("read file failed: {e}")))
    }
    async fn write(&self, path: &Path, bytes: &[u8]) -> Result<(), AgentToolError> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| AgentToolError::ExecFailed(e.to_string()))?;
        }
        tokio::fs::write(path, bytes)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("write file failed: {e}")))
    }
}
