use buckyos_api::{
    parse_typed_task_data, MsgCenterClient, SendMessageTaskData, TaskFilter, TaskManagerClient,
    TaskStatus, TypedTaskData,
};
use log::{info, warn};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, Duration};

use crate::scheduled_task_manager::ScheduleStore;

const TASK_TYPE: &str = "workflow.send_message";
const RUNNER: &str = "workflow";
const SWEEP_INTERVAL: Duration = Duration::from_secs(10);

pub struct SendMessageTaskExecutor {
    task_mgr: Arc<TaskManagerClient>,
    msg_center: Arc<MsgCenterClient>,
    schedules: Arc<ScheduleStore>,
    buckyos_root: PathBuf,
}

impl SendMessageTaskExecutor {
    pub fn new(
        task_mgr: Arc<TaskManagerClient>,
        msg_center: Arc<MsgCenterClient>,
        schedules: Arc<ScheduleStore>,
        buckyos_root: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            task_mgr,
            msg_center,
            schedules,
            buckyos_root,
        })
    }

    pub fn start(self: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                self.sweep_pending().await;
                sleep(SWEEP_INTERVAL).await;
            }
        });
    }

    async fn sweep_pending(&self) {
        let filter = TaskFilter {
            task_type: Some(TASK_TYPE.to_string()),
            runner: Some(RUNNER.to_string()),
            status: Some(TaskStatus::Pending),
            ..Default::default()
        };
        let tasks = match self.task_mgr.list_tasks(Some(filter), None, None).await {
            Ok(tasks) => tasks,
            Err(err) => {
                warn!("workflow.send_message executor list_tasks failed: {err:?}");
                return;
            }
        };
        for task in tasks {
            if let Err(err) = self
                .execute_task(task.id, task.data.clone(), task.root_id)
                .await
            {
                warn!("workflow.send_message task {} failed: {}", task.id, err);
                if let Err(update_err) = self.task_mgr.mark_task_as_failed(task.id, &err).await {
                    warn!(
                        "workflow.send_message task {} mark failed failed: {update_err:?}",
                        task.id
                    );
                }
            }
        }
    }

    async fn execute_task(&self, task_id: i64, data: Value, root_id: String) -> Result<(), String> {
        self.task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Running),
                Some(0.1),
                Some("sending message".to_string()),
                None,
            )
            .await
            .map_err(|err| format!("mark running failed: {err:?}"))?;

        let mut task_data = parse_send_message_data(data)?;
        let schedule_id = task_data
            .request
            .trigger
            .as_ref()
            .and_then(|trigger| trigger.schedule_id.clone())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(root_id);
        let schedule = self
            .schedules
            .get(&schedule_id)
            .await
            .ok_or_else(|| format!("schedule `{schedule_id}` not found"))?;
        let sender = resolve_sender_did(
            &self.buckyos_root,
            &schedule.owner.user_id,
            &schedule.owner.app_id,
        )
        .ok_or_else(|| {
            format!(
                "cannot resolve sender DID for owner {}/{}",
                schedule.owner.user_id, schedule.owner.app_id
            )
        })?;
        let recipient = resolve_recipient_did(&task_data.request.to, &schedule.owner.user_id)?;
        let text = task_data.request.text.trim();
        if text.is_empty() {
            return Err("send_message text is empty".to_string());
        }

        let msg = build_text_message(sender.as_str(), recipient.as_str(), text)?;
        let idempotency_key = Some(format!("{TASK_TYPE}:{task_id}"));
        let result = self
            .msg_center
            .post_send(msg, idempotency_key)
            .await
            .map_err(|err| format!("msg_center.post_send failed: {err:?}"))?;
        if !result.ok {
            return Err(format!(
                "msg_center.post_send rejected: {}",
                result.reason.unwrap_or_else(|| "unknown".to_string())
            ));
        }

        task_data.result = Some(json!({
            "ok": true,
            "msg_id": result.msg_id.to_string(),
            "deliveries": result.deliveries,
        }));
        let updated_data = serde_json::to_value(&task_data)
            .map_err(|err| format!("serialize send_message result failed: {err}"))?;
        self.task_mgr
            .update_task_data(task_id, updated_data)
            .await
            .map_err(|err| format!("update result data failed: {err:?}"))?;
        self.task_mgr
            .update_task(
                task_id,
                Some(TaskStatus::Completed),
                Some(1.0),
                Some("message sent".to_string()),
                None,
            )
            .await
            .map_err(|err| format!("mark completed failed: {err:?}"))?;
        info!("workflow.send_message task {} sent", task_id);
        Ok(())
    }
}

fn parse_send_message_data(data: Value) -> Result<SendMessageTaskData, String> {
    match parse_typed_task_data(TASK_TYPE, data) {
        Ok(TypedTaskData::WorkflowSendMessage(data)) => Ok(data),
        Ok(other) => Err(format!(
            "expected workflow.send_message data, got {:?}",
            other.task_data_type()
        )),
        Err(err) => Err(format!("invalid workflow.send_message data: {err}")),
    }
}

/// Normalize the configured recipient into a determined DID. `post_send` only
/// accepts confirmed targets (shareable DID → MessageHub, shadow endpoint DID
/// → its tunnel instance); there is no contact→channel resolution on the send
/// path — the schedule author picks the exact DID when authoring the task.
fn resolve_recipient_did(raw: &str, owner_user_id: &str) -> Result<String, String> {
    let target = raw.trim();
    if target.eq_ignore_ascii_case("owner") || target.eq_ignore_ascii_case("self") {
        return Ok(format!("did:bns:{owner_user_id}"));
    }
    if target.starts_with("did:") {
        return Ok(target.to_string());
    }
    Err(format!("unsupported send_message recipient `{target}`"))
}

fn resolve_sender_did(root: &Path, user_id: &str, app_id: &str) -> Option<String> {
    if app_id.starts_with("did:") {
        return Some(app_id.to_string());
    }
    for app_dir in app_dir_candidates(app_id) {
        let path = root
            .join("data")
            .join("home")
            .join(user_id)
            .join(".local")
            .join("share")
            .join(app_dir)
            .join("agent.toml");
        if let Some(did) = read_agent_did(&path) {
            return Some(did);
        }
    }
    synthesize_agent_did(root, app_id)
}

fn app_dir_candidates(app_id: &str) -> Vec<String> {
    let trimmed = app_id.trim();
    let lowercase = trimmed.to_ascii_lowercase();
    let mut out = Vec::new();
    for candidate in [
        trimmed.to_string(),
        lowercase.clone(),
        lowercase.trim_start_matches("buckyos_").to_string(),
    ] {
        if !candidate.is_empty() && !out.contains(&candidate) {
            out.push(candidate);
        }
    }
    out
}

fn read_agent_did(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    content.lines().find_map(|line| {
        let line = line.trim();
        let value = line.strip_prefix("agent_did")?.split_once('=')?.1.trim();
        let value = value.trim_matches('"').trim();
        if value.starts_with("did:") {
            Some(value.to_string())
        } else {
            None
        }
    })
}

fn synthesize_agent_did(root: &Path, app_id: &str) -> Option<String> {
    let path = root.join("etc").join("node_gateway_info.json");
    let raw = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let zone_host = value
        .pointer("/node_info/this_zone_host")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    let app = app_id.trim().to_ascii_lowercase();
    if app.is_empty() {
        return None;
    }
    Some(format!("did:web:{app}.{zone_host}"))
}

fn build_text_message(
    sender: &str,
    recipient: &str,
    text: &str,
) -> Result<ndn_lib::MsgObject, String> {
    let value = json!({
        "from": sender,
        "to": [recipient],
        "kind": "chat",
        "created_at_ms": now_ms(),
        "content": {
            "format": "text/plain",
            "content": text,
        },
        "llm_role": "system",
        "parse_mode": "Plain",
    });
    serde_json::from_value(value).map_err(|err| format!("build MsgObject failed: {err}"))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipient_owner_maps_to_user_did() {
        assert_eq!(
            resolve_recipient_did("owner", "devtest").unwrap(),
            "did:bns:devtest"
        );
        assert_eq!(
            resolve_recipient_did("self", "devtest").unwrap(),
            "did:bns:devtest"
        );
    }

    #[test]
    fn app_dir_candidates_include_lowercase_and_stripped() {
        assert_eq!(
            app_dir_candidates("BuckyOS_Jarvis"),
            vec!["BuckyOS_Jarvis", "buckyos_jarvis", "jarvis"]
        );
    }

    #[test]
    fn build_text_message_uses_plain_chat_message() {
        let msg = build_text_message("did:web:jarvis.test.buckyos.io", "did:bns:devtest", "hello")
            .unwrap();
        assert_eq!(msg.from.to_string(), "did:web:jarvis.test.buckyos.io");
        assert_eq!(msg.to[0].to_string(), "did:bns:devtest");
        assert_eq!(msg.content.content, "hello");
    }

    #[test]
    fn recipient_keeps_determined_dids_as_is() {
        // Shareable DIDs and shadow endpoint DIDs both pass through untouched:
        // post_send resolves them deterministically, no selector lookup here.
        assert_eq!(
            resolve_recipient_did("did:bns:bob", "devtest").unwrap(),
            "did:bns:bob"
        );
        assert_eq!(
            resolve_recipient_did("did:msgtunnel:1.user.tg-main-tunnel", "devtest").unwrap(),
            "did:msgtunnel:1.user.tg-main-tunnel"
        );
        assert!(resolve_recipient_did("telegram", "devtest").is_err());
    }
}
