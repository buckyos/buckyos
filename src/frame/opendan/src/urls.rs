//! `read` of an OpenDAN Agent (agent-object-url TODO §6 第 3 段遗留 3): a file of another device,
//! `file://<device_id>/<path>` outside the Zone's data, is read through the node runtime the owner registered for
//! that device; every other URL goes to the object library (`ObjectUrlReader`).

use std::path::PathBuf;
use std::sync::Arc;

use agent_did_object_lib::ObjectUrlReader;
use agent_tool::read_tool::{UrlContent, UrlReader};
use agent_tool::{AgentToolError, SessionRuntimeContext};
use async_trait::async_trait;
use buckyos_api::ObjectUrl;
use libopendan::protocol::RuntimeRecord;
use libopendan::state::AgentStateClient;
use name_lib::DID;
use serde_json::json;

pub struct AgentUrlReader {
    pub objects: ObjectUrlReader,
    pub agent: Arc<dyn AgentStateClient>,
    /// Where node operations keep their records.
    pub ops_dir: PathBuf,
}

impl std::fmt::Debug for AgentUrlReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentUrlReader")
    }
}

/// The device a node runtime runs on: the first label of its `node_did` (`did:web:ood1.<zone>` → `ood1`).
fn runtime_device(record: &RuntimeRecord) -> Option<String> {
    let exec = record.exec.as_ref()?;
    let node_did = exec.pointer("/remote_node/node_did")?.as_str()?;
    let host = DID::from_str(node_did).ok()?.to_host_name();
    host.split('.').next().map(str::to_string)
}

impl AgentUrlReader {
    async fn read_device_file(&self, url: &str, device: &str, path: &str) -> Result<UrlContent, AgentToolError> {
        let records = self
            .agent
            .runtimes()
            .list()
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("runtimes: {e}")))?;
        let record = records
            .iter()
            .find(|r| r.kind.is_node() && runtime_device(r).as_deref() == Some(device))
            .ok_or_else(|| {
                AgentToolError::InvalidArgs(format!(
                    "{url}: no runtime of device {device} is registered for this Agent (`agent-session runtime list`), so its files are not reachable"
                ))
            })?;
        let target = libopendan::runtime::open_node_target(record, &self.ops_dir)
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("{url}: {e}")))?;
        let decoded = buckyos_api::decode_url_path(path);
        let bytes = target.read(&decoded).await?;
        let text = String::from_utf8(bytes).map_err(|_| {
            AgentToolError::InvalidArgs(format!("{url} is not text; use llm_understand_media to look at media files"))
        })?;
        Ok(UrlContent {
            text,
            details: json!({ "object": url, "source": format!("runtime:{}", record.runtime_id), "route": "node_runtime" }),
        })
    }
}

#[async_trait]
impl UrlReader for AgentUrlReader {
    async fn read_url(
        &self,
        ctx: &SessionRuntimeContext,
        url: &str,
        property: Option<&str>,
    ) -> Result<UrlContent, AgentToolError> {
        if let Ok(ObjectUrl::File { device_id: Some(device), path }) = buckyos_api::parse_object_url(url) {
            let data_root = buckyos_kit::get_buckyos_root_dir().join("data");
            if buckyos_api::data_path_to_dfs(&data_root, std::path::Path::new(&path)).is_none() && property.is_none() {
                return self.read_device_file(url, &device, &path).await;
            }
        }
        self.objects.read_url(ctx, url, property).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            tool_call_index: 0,
            wakeup_id: "w".into(),
            session_id: "s".into(),
            read_token_limit: agent_tool::DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    #[tokio::test]
    async fn another_devices_file_needs_a_runtime_registered_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let agent = Arc::new(libopendan::FsAgentStateClient::open(dir.path(), "did:web:jarvis.test.buckyos.io", None, None).unwrap());
        let reader = AgentUrlReader { objects: ObjectUrlReader::builtin(), agent, ops_dir: dir.path().join("ops") };
        let err = reader.read_url(&ctx(), "file://node2/tmp/a.md", None).await.unwrap_err();
        assert!(err.to_string().contains("no runtime of device node2"), "{err}");

        let record: RuntimeRecord = serde_json::from_value(json!({
            "version": 1, "runtime_id": "lab", "kind": "remote_node", "name": "lab", "runtime_host": "h",
            "exec": { "kind": "remote_node", "remote_node": { "node_did": "did:web:node2.test.buckyos.io", "profile_id": "p" } },
            "availability": "available", "baseline": "standard", "revision": 1, "registered_by": "x",
            "registered_at_ms": 1, "updated_at_ms": 1
        }))
        .unwrap();
        assert_eq!(runtime_device(&record).as_deref(), Some("node2"));
    }
}
