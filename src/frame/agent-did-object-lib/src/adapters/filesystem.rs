use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::json;
use tokio::fs;
use url::Url;

use super::{
    unsupported_event_subscription, unsupported_event_unsubscription, AdapterEventSubscription,
    AdapterReadRequest, AdapterReadResponse, AdapterSubscribeEventRequest,
    AdapterUnsubscribeEventRequest, AdapterXCallRequest, AdapterXCallResponse, AgentObjectAdapter,
};
use crate::error::AgentDIDObjectError;
use crate::types::{ReadMeta, TrustGuidance};

const DEFAULT_TEXT_LIMIT_BYTES: u64 = 512 * 1024;

pub struct FilesystemAdapter {
    id: String,
}

impl FilesystemAdapter {
    pub fn new(id: String) -> Self {
        Self { id }
    }
}

#[async_trait]
impl AgentObjectAdapter for FilesystemAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let path = resolve_path(&req.object_ref.normalized)?;
        ensure_allowed_path(&path, &req.adapter_config.options)?;
        let metadata = fs::metadata(&path).await?;
        if !metadata.is_file() {
            return Err(AgentDIDObjectError::UnsupportedObjectRef(format!(
                "{} is not a file",
                path.display()
            )));
        }

        let size = metadata.len();
        let limit = req
            .adapter_config
            .options
            .get("max_bytes")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_TEXT_LIMIT_BYTES);
        let bytes = fs::read(&path).await?;
        let is_text = std::str::from_utf8(&bytes).is_ok();
        let content_type = if is_text {
            "text/plain"
        } else {
            "application/octet-stream"
        };
        let truncated = is_text && size > limit;
        let content = if is_text {
            let mut text = String::from_utf8_lossy(&bytes).to_string();
            if truncated {
                text = text.chars().take(limit as usize).collect::<String>();
                text.push_str("\n[truncated]");
            }
            Some(text)
        } else {
            Some(format!(
                "Binary file: {} ({} bytes). Content is not included.",
                path.display(),
                size
            ))
        };

        Ok(AdapterReadResponse {
            object: req.object_ref.raw.clone(),
            object_did: None,
            content,
            meta: ReadMeta {
                title: path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string()),
                content_type: Some(content_type.to_string()),
                size: Some(size),
                updated_at: metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs().to_string()),
                source: Some(path.display().to_string()),
                extra: json!({}),
            },
            prompt_guidance: vec![],
            trust_guidance: vec![TrustGuidance {
                message: "Content was read from the local filesystem.".to_string(),
            }],
            errors: vec![],
            cache_key: Some(path.display().to_string()),
            version: Some(format!(
                "{}:{}",
                size,
                metadata
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_secs())
                    .unwrap_or(0)
            )),
            route: req.route_trace,
            adapt_meta: json!({
                "resolved_path": path.display().to_string(),
                "content_type": content_type,
                "size": size,
                "truncated": truncated,
            }),
        })
    }

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
        Err(AgentDIDObjectError::UnsupportedMethod(format!(
            "filesystem adapter does not support x_call action {}",
            req.input.action
        )))
    }

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
        unsupported_event_subscription(&self.id, &req)
    }

    async fn unsubscribe_event(
        &self,
        _req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError> {
        unsupported_event_unsubscription(&self.id)
    }
}

fn ensure_allowed_path(
    path: &Path,
    options: &serde_json::Value,
) -> Result<(), AgentDIDObjectError> {
    let roots = options
        .get("allowed_read_roots")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if roots.is_empty() {
        return Ok(());
    }

    let normalized_path = normalize_abs_path(path);
    if roots
        .iter()
        .map(|root| normalize_abs_path(root))
        .any(|root| normalized_path.starts_with(root))
    {
        return Ok(());
    }

    Err(AgentDIDObjectError::UnsupportedObjectRef(format!(
        "read path not allowed by policy: {}",
        path.display()
    )))
}

fn normalize_abs_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    normalized
}

fn resolve_path(input: &str) -> Result<PathBuf, AgentDIDObjectError> {
    let url = Url::parse(input).map_err(|err| {
        AgentDIDObjectError::UnsupportedObjectRef(format!("invalid file URL {input}: {err}"))
    })?;
    if url.scheme() != "file" {
        return Err(AgentDIDObjectError::UnsupportedObjectRef(format!(
            "filesystem adapter requires file URL: {input}"
        )));
    }
    url.to_file_path().map_err(|_| {
        AgentDIDObjectError::UnsupportedObjectRef(format!("invalid file path URL {input}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AdapterConfig, AdapterType, ObjectRoute};
    use crate::router::{RouteMatchType, RouteMethod, RouteTrace};
    use crate::types::{ObjectRef, ReadInput};
    use serde_json::json;
    use tempfile::tempdir;

    fn req(object_url: String) -> AdapterReadRequest {
        AdapterReadRequest {
            object_ref: ObjectRef::parse(&object_url).unwrap(),
            input: ReadInput {
                object: object_url,
                purpose: None,
                session_id: None,
                content_only: false,
                range: None,
                max_tokens: None,
                options: json!({}),
            },
            route: ObjectRoute {
                id: "r".to_string(),
                priority: 0,
                match_type: RouteMatchType::Scheme,
                pattern: "file".to_string(),
                adapter: "filesystem".to_string(),
                methods: vec![RouteMethod::Read],
                options: json!({}),
            },
            route_trace: RouteTrace {
                route_id: "r".to_string(),
                adapter: "filesystem".to_string(),
                method: RouteMethod::Read,
            },
            adapter_config: AdapterConfig {
                id: "filesystem".to_string(),
                adapter_type: AdapterType::Filesystem,
                endpoint: None,
                auth_token_env: None,
                options: json!({"max_bytes": 4}),
            },
        }
    }

    #[tokio::test]
    async fn reads_text_file_with_truncation_meta() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, "abcdef").await.unwrap();
        let object_url = Url::from_file_path(&path).unwrap().to_string();
        let adapter = FilesystemAdapter::new("filesystem".to_string());
        let res = adapter.read(req(object_url)).await.unwrap();
        assert_eq!(res.content.unwrap(), "abcd\n[truncated]");
        assert_eq!(res.adapt_meta["truncated"], true);
    }

    #[tokio::test]
    async fn summarizes_binary_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a.bin");
        fs::write(&path, [0, 159, 146, 150]).await.unwrap();
        let object_url = Url::from_file_path(&path).unwrap().to_string();
        let adapter = FilesystemAdapter::new("filesystem".to_string());
        let res = adapter.read(req(object_url)).await.unwrap();
        assert_eq!(
            res.meta.content_type.as_deref(),
            Some("application/octet-stream")
        );
        assert!(res.content.unwrap().contains("Binary file"));
    }

    #[tokio::test]
    async fn rejects_file_outside_allowed_roots() {
        let dir = tempdir().unwrap();
        let allowed = dir.path().join("allowed");
        let denied = dir.path().join("denied");
        fs::create_dir_all(&allowed).await.unwrap();
        fs::create_dir_all(&denied).await.unwrap();
        let path = denied.join("a.txt");
        fs::write(&path, "secret").await.unwrap();

        let mut req = req(Url::from_file_path(&path).unwrap().to_string());
        req.adapter_config.options = json!({
            "allowed_read_roots": [allowed.display().to_string()]
        });
        let adapter = FilesystemAdapter::new("filesystem".to_string());
        let err = adapter.read(req).await.unwrap_err();
        assert!(err.to_string().contains("read path not allowed"));
    }
}
