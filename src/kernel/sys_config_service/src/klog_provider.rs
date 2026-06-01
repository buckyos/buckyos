use crate::kv_provider::{KVStoreErrors, KVStoreProvider, Result};
use async_trait::async_trait;
use buckyos_kit::{set_json_by_path, KVAction};
use klog::error::KLogErrorCode;
use klog::network::{
    KLogMetaDeleteRequest, KLogMetaPutRequest, KLogMetaQueryRequest, KLogMetaQueryResponse,
};
use klog::rpc::{KLogClient, KLogClientError};
use klog::{KLogMetaEntry, KLogMetaTxAction, KLogMetaTxGuard, KLogMetaTxRequest};
use log::*;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

const DEFAULT_KLOG_NODE_NAME: &str = "system_config";

// Optional. Overrides the klog JSON-RPC endpoint used by this provider.
// Default: local BuckyOS service route, http://127.0.0.1:4080/kapi/klog-service.
const ENV_KLOG_ENDPOINT: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_ENDPOINT";

// Optional. Identifies system_config writes in KLogMetaEntry.updated_by_node_name.
// Default: BUCKYOS_THIS_DEVICE.name, then "system_config" when the device doc
// is unavailable. Set this only when tests or diagnostics need a fixed writer id.
const ENV_KLOG_NODE_NAME: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_NODE_NAME";

// Optional. Page size for klog prefix list requests.
// Default: 2000, matching the current klog meta query max limit.
const ENV_KLOG_META_QUERY_LIMIT: &str = "BUCKYOS_SYSTEM_CONFIG_KLOG_META_QUERY_LIMIT";

// Optional. Injected by node-daemon for kernel services. It contains the local
// DeviceConfig JSON; this provider uses its `name` field as the BuckyOS node id.
const ENV_THIS_DEVICE: &str = "BUCKYOS_THIS_DEVICE";

const INTERNAL_META_PREFIX: &str = "__meta/";
const DEFAULT_META_QUERY_LIMIT: usize = 2_000;

pub struct KLogStore {
    client: KLogClient,
    node_name: String,
    meta_query_limit: usize,
}

impl KLogStore {
    pub fn new_from_env() -> Self {
        let node_name = resolve_node_name_from_env();
        let meta_query_limit = resolve_meta_query_limit_from_env();
        let client = std::env::var(ENV_KLOG_ENDPOINT)
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .map(|endpoint| KLogClient::new(endpoint, node_name.clone()))
            .unwrap_or_else(|| KLogClient::local_default(node_name.clone()));

        Self {
            client,
            node_name,
            meta_query_limit,
        }
    }

    fn now_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|v| v.as_millis().try_into().unwrap_or(u64::MAX))
            .unwrap_or(0)
    }

    fn meta_entry(&self, key: String, value: String) -> KLogMetaEntry {
        KLogMetaEntry {
            key,
            value,
            updated_at: Self::now_ms(),
            updated_by_node_name: self.node_name.clone(),
            ..KLogMetaEntry::default()
        }
    }

    fn is_internal_key(key: &str) -> bool {
        key.starts_with(INTERNAL_META_PREFIX)
    }

    fn map_error(err: KLogClientError) -> KVStoreErrors {
        error!("KLogStore request failed: {}", err);
        KVStoreErrors::InternalError(err.to_string())
    }

    fn map_conflict(err: KLogClientError, key: &str, expected: u64) -> KVStoreErrors {
        if err.error_code == KLogErrorCode::VersionConflict {
            let actual = parse_current_revision(&err.message).unwrap_or(expected);
            return KVStoreErrors::RevisionMismatch {
                key: key.to_string(),
                expected,
                actual,
            };
        }

        Self::map_error(err)
    }

    async fn query_meta(&self, key: &str) -> Result<Option<KLogMetaEntry>> {
        let resp = self
            .client
            .query_meta(KLogMetaQueryRequest {
                key: Some(key.to_string()),
                prefix: None,
                limit: Some(1),
                cursor: None,
                revision: None,
                strong_read: Some(true),
            })
            .await
            .map_err(Self::map_error)?;
        Ok(resp.items.into_iter().next())
    }

    async fn list_meta(&self, prefix: &str) -> Result<KLogMetaQueryResponse> {
        let mut items = Vec::new();
        let mut cursor = None;
        loop {
            let resp = self
                .client
                .query_meta(KLogMetaQueryRequest {
                    key: None,
                    prefix: Some(prefix.to_string()),
                    limit: Some(self.meta_query_limit),
                    cursor: cursor.clone(),
                    revision: None,
                    strong_read: Some(true),
                })
                .await
                .map_err(Self::map_error)?;
            items.extend(resp.items);
            if !resp.has_more {
                return Ok(KLogMetaQueryResponse {
                    items,
                    next_cursor: None,
                    has_more: false,
                });
            }

            let Some(next_cursor) = resp.next_cursor else {
                let msg = format!(
                    "KLogStore list prefix missing next_cursor: prefix={}, loaded={}",
                    prefix,
                    items.len()
                );
                error!("{}", msg);
                return Err(KVStoreErrors::InternalError(msg));
            };
            if cursor.as_ref() == Some(&next_cursor) {
                let msg = format!(
                    "KLogStore list prefix cursor did not advance: prefix={}, cursor={}",
                    prefix, next_cursor
                );
                error!("{}", msg);
                return Err(KVStoreErrors::InternalError(msg));
            }
            cursor = Some(next_cursor);
        }
    }

    async fn put_with_expected(
        &self,
        key: String,
        value: String,
        expected_revision: Option<u64>,
    ) -> Result<()> {
        self.client
            .put_meta(KLogMetaPutRequest {
                key: key.clone(),
                value,
                node_name: Some(self.node_name.clone()),
                expected_revision,
            })
            .await
            .map(|_| ())
            .map_err(|err| {
                if expected_revision == Some(0) && err.error_code == KLogErrorCode::VersionConflict
                {
                    warn!("KLogStore create failed, key exists: {}", key);
                    KVStoreErrors::KeyExist(key)
                } else if let Some(expected) = expected_revision {
                    Self::map_conflict(err, &key, expected)
                } else {
                    Self::map_error(err)
                }
            })
    }

    async fn current_value_revision(&self, key: &str) -> Result<(String, u64)> {
        let Some(item) = self.query_meta(key).await? else {
            return Err(KVStoreErrors::KeyNotFound(key.to_string()));
        };
        let revision = item.effective_mod_revision();
        Ok((item.value, revision))
    }
}

#[async_trait]
impl KVStoreProvider for KLogStore {
    async fn get(&self, key: String) -> Result<Option<String>> {
        Ok(self
            .get_with_revision(key)
            .await?
            .map(|(value, _revision)| value))
    }

    async fn get_with_revision(&self, key: String) -> Result<Option<(String, u64)>> {
        let Some(item) = self.query_meta(&key).await? else {
            return Ok(None);
        };

        debug!(
            "KLogStore Get key:[{}] value length:[{}] revision:[{}]",
            key,
            item.value.len(),
            item.effective_mod_revision()
        );
        let revision = item.effective_mod_revision();
        Ok(Some((item.value, revision)))
    }

    async fn set(&self, key: String, value: String) -> Result<()> {
        self.put_with_expected(key.clone(), value, None).await?;
        debug!("KLogStore Set key:[{}]", key);
        Ok(())
    }

    async fn set_by_path(&self, key: String, json_path: String, value: &Value) -> Result<()> {
        let (current_value, current_revision) = self.current_value_revision(&key).await?;
        let mut current_value: Value = serde_json::from_str(&current_value)
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        set_json_by_path(&mut current_value, &json_path, Some(value));
        let updated_value = serde_json::to_string(&current_value)
            .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
        self.put_with_expected(key, updated_value, Some(current_revision))
            .await
    }

    async fn exec_tx(
        &self,
        tx: HashMap<String, KVAction>,
        main_key: Option<(String, u64)>,
    ) -> Result<()> {
        if tx.is_empty() && main_key.is_none() {
            return Ok(());
        }

        let mut actions = BTreeMap::new();
        for (key, action) in tx {
            match action {
                KVAction::Create(value) => {
                    actions.insert(
                        key.clone(),
                        KLogMetaTxAction::Put {
                            item: self.meta_entry(key, value),
                            expected_revision: Some(0),
                        },
                    );
                }
                KVAction::Update(value) => {
                    actions.insert(
                        key.clone(),
                        KLogMetaTxAction::Put {
                            item: self.meta_entry(key, value),
                            expected_revision: None,
                        },
                    );
                }
                KVAction::Append(value) => {
                    let (current_value, current_revision) =
                        self.current_value_revision(&key).await?;
                    actions.insert(
                        key.clone(),
                        KLogMetaTxAction::Put {
                            item: self.meta_entry(key, format!("{}{}", current_value, value)),
                            expected_revision: Some(current_revision),
                        },
                    );
                }
                KVAction::SetByJsonPath(value) => {
                    let (current_value, current_revision) =
                        self.current_value_revision(&key).await?;
                    let mut current_value: Value = serde_json::from_str(&current_value)
                        .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                    for (path, sub_value) in value.iter() {
                        set_json_by_path(&mut current_value, path, sub_value.as_ref());
                    }
                    let updated_value = serde_json::to_string(&current_value)
                        .map_err(|err| KVStoreErrors::InternalError(err.to_string()))?;
                    actions.insert(
                        key.clone(),
                        KLogMetaTxAction::Put {
                            item: self.meta_entry(key, updated_value),
                            expected_revision: Some(current_revision),
                        },
                    );
                }
                KVAction::Remove => {
                    actions.insert(
                        key.clone(),
                        KLogMetaTxAction::Delete {
                            key,
                            expected_revision: None,
                        },
                    );
                }
            }
        }

        if actions.is_empty() {
            let Some((key, expected_revision)) = main_key.clone() else {
                return Ok(());
            };
            let (value, current_revision) = self.current_value_revision(&key).await?;
            if current_revision != expected_revision {
                return Err(KVStoreErrors::RevisionMismatch {
                    key,
                    expected: expected_revision,
                    actual: current_revision,
                });
            }
            actions.insert(
                key.clone(),
                KLogMetaTxAction::Put {
                    item: self.meta_entry(key, value),
                    expected_revision: Some(expected_revision),
                },
            );
        }

        let guard = main_key.map(|(key, expected_revision)| KLogMetaTxGuard {
            key,
            expected_revision,
        });
        self.client
            .exec_meta_tx(KLogMetaTxRequest { actions, guard })
            .await
            .map(|_| ())
            .map_err(|err| {
                if err.error_code == KLogErrorCode::VersionConflict {
                    let key = parse_conflict_key(&err.message).unwrap_or_else(|| "-".to_string());
                    let expected = parse_expected_revision(&err.message).unwrap_or(0);
                    let actual = parse_current_revision(&err.message).unwrap_or(expected);
                    return KVStoreErrors::RevisionMismatch {
                        key,
                        expected,
                        actual,
                    };
                }

                Self::map_error(err)
            })
    }

    async fn create(&self, key: &str, value: &str) -> Result<()> {
        self.put_with_expected(key.to_string(), value.to_string(), Some(0))
            .await
    }

    async fn delete(&self, key: &str) -> Result<()> {
        let resp = self
            .client
            .delete_meta(KLogMetaDeleteRequest {
                key: key.to_string(),
            })
            .await
            .map_err(Self::map_error)?;
        if !resp.existed {
            return Err(KVStoreErrors::KeyNotFound(key.to_string()));
        }
        debug!("KLogStore Delete key:[{}]", key);
        Ok(())
    }

    async fn list_data(&self, key_prefix: &str) -> Result<HashMap<String, String>> {
        let resp = self.list_meta(key_prefix).await?;
        let mut result = HashMap::new();
        for item in resp.items {
            if Self::is_internal_key(&item.key) {
                continue;
            }
            result.insert(item.key, item.value);
        }
        Ok(result)
    }

    async fn list_keys(&self, key_prefix: &str) -> Result<Vec<String>> {
        let resp = self.list_meta(key_prefix).await?;
        Ok(resp
            .items
            .into_iter()
            .map(|item| item.key)
            .filter(|key| !Self::is_internal_key(key))
            .collect())
    }

    async fn list_direct_children(&self, prefix: String) -> Result<Vec<String>> {
        let list_prefix = if prefix.is_empty() || prefix.ends_with('/') {
            prefix.clone()
        } else {
            format!("{}/", prefix)
        };
        let keys = self.list_keys(&list_prefix).await?;
        Ok(direct_children_from_keys(prefix, keys))
    }
}

fn resolve_node_name_from_env() -> String {
    resolve_node_name(
        std::env::var(ENV_KLOG_NODE_NAME).ok(),
        std::env::var(ENV_THIS_DEVICE).ok(),
    )
}

fn resolve_meta_query_limit_from_env() -> usize {
    resolve_meta_query_limit(std::env::var(ENV_KLOG_META_QUERY_LIMIT).ok())
}

fn resolve_meta_query_limit(raw: Option<String>) -> usize {
    let Some(raw) = normalize_non_empty(raw) else {
        return DEFAULT_META_QUERY_LIMIT;
    };

    match raw.parse::<usize>() {
        Ok(value) if value > 0 && value <= DEFAULT_META_QUERY_LIMIT => value,
        _ => {
            warn!(
                "Invalid {}, fallback meta_query_limit={}: {}",
                ENV_KLOG_META_QUERY_LIMIT, DEFAULT_META_QUERY_LIMIT, raw
            );
            DEFAULT_META_QUERY_LIMIT
        }
    }
}

fn resolve_node_name(explicit_node_name: Option<String>, device_doc: Option<String>) -> String {
    if let Some(node_name) = normalize_non_empty(explicit_node_name) {
        return node_name;
    }

    if let Some(device_doc) = normalize_non_empty(device_doc) {
        if let Some(node_name) = parse_device_doc_name(&device_doc) {
            return node_name;
        }
        warn!(
            "KLogStore failed to parse {}.name, fallback node_name={}",
            ENV_THIS_DEVICE, DEFAULT_KLOG_NODE_NAME
        );
    }

    DEFAULT_KLOG_NODE_NAME.to_string()
}

fn parse_device_doc_name(device_doc: &str) -> Option<String> {
    let value: Value = serde_json::from_str(device_doc).ok()?;
    value
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(ToString::to_string)
}

fn normalize_non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn direct_children_from_keys(prefix: String, keys: Vec<String>) -> Vec<String> {
    let prefix = if prefix.is_empty() || prefix.ends_with('/') {
        prefix
    } else {
        format!("{}/", prefix)
    };
    let mut result = Vec::new();
    for key in keys {
        let suffix = key.strip_prefix(prefix.as_str()).unwrap_or(key.as_str());
        let child = suffix
            .trim_start_matches('/')
            .split('/')
            .next()
            .unwrap_or("");
        if !child.is_empty() && !result.iter().any(|existing| existing == child) {
            result.push(child.to_string());
        }
    }
    result
}

fn parse_conflict_key(message: &str) -> Option<String> {
    parse_after(message, "key=").map(|raw| raw.split(',').next().unwrap_or(raw).to_string())
}

fn parse_expected_revision(message: &str) -> Option<u64> {
    parse_after(message, "expected_revision=")
        .and_then(|raw| raw.split(',').next().unwrap_or(raw).parse::<u64>().ok())
}

fn parse_current_revision(message: &str) -> Option<u64> {
    let raw = parse_after(message, "current_revision=")?;
    let raw = raw.split(',').next().unwrap_or(raw).trim();
    if raw == "None" {
        return Some(0);
    }
    raw.strip_prefix("Some(")
        .and_then(|v| v.strip_suffix(')'))
        .unwrap_or(raw)
        .parse::<u64>()
        .ok()
}

fn parse_after<'a>(message: &'a str, marker: &str) -> Option<&'a str> {
    let start = message.find(marker)? + marker.len();
    Some(message[start..].trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_children_from_prefixed_keys() {
        let children = direct_children_from_keys(
            "users".to_string(),
            vec![
                "users/alice/profile".to_string(),
                "users/alice/settings".to_string(),
                "users/bob/profile".to_string(),
            ],
        );
        assert_eq!(children, vec!["alice".to_string(), "bob".to_string()]);
    }

    #[test]
    fn parse_conflict_fields() {
        let msg = "meta tx version conflict: key=users/alice, expected_revision=1, current_revision=Some(2)";
        assert_eq!(parse_conflict_key(msg), Some("users/alice".to_string()));
        assert_eq!(parse_expected_revision(msg), Some(1));
        assert_eq!(parse_current_revision(msg), Some(2));
        assert_eq!(parse_current_revision("current_revision=None"), Some(0));
    }

    #[test]
    fn resolve_node_name_prefers_override_then_device_doc() {
        assert_eq!(
            resolve_node_name(
                Some("explicit-ood".to_string()),
                Some(r#"{"name":"ood1"}"#.to_string())
            ),
            "explicit-ood"
        );
        assert_eq!(
            resolve_node_name(None, Some(r#"{"name":"ood1"}"#.to_string())),
            "ood1"
        );
        assert_eq!(
            resolve_node_name(
                Some("  ".to_string()),
                Some(r#"{"name":"ood2"}"#.to_string())
            ),
            "ood2"
        );
        assert_eq!(
            resolve_node_name(None, Some(r#"{"id":"device"}"#.to_string())),
            DEFAULT_KLOG_NODE_NAME
        );
    }

    #[test]
    fn resolve_meta_query_limit_from_env_default_without_env() {
        assert_eq!(resolve_meta_query_limit(None), DEFAULT_META_QUERY_LIMIT);
    }

    #[test]
    fn resolve_meta_query_limit_from_env_accepts_valid_value() {
        assert_eq!(resolve_meta_query_limit(Some("17".to_string())), 17);
    }

    #[test]
    fn resolve_meta_query_limit_from_env_rejects_invalid_value() {
        assert_eq!(
            resolve_meta_query_limit(Some("0".to_string())),
            DEFAULT_META_QUERY_LIMIT
        );
        assert_eq!(
            resolve_meta_query_limit(Some((DEFAULT_META_QUERY_LIMIT + 1).to_string())),
            DEFAULT_META_QUERY_LIMIT
        );
    }
}
