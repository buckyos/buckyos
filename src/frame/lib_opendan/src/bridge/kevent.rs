//! kevent bridge: `object_event` subscriptions of a session become
//! `AgentEvent{source: object:<object>}` posted through the registry.
//!
//! The subscription's `object` is the kevent id (pattern) to read. Event
//! data fields used when present: `event` (name), `seq`, `summary`,
//! `data_ref`; otherwise the subscription's event name (or `changed`) and a
//! compact rendering of the data.

use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::KEventClient;
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::state::AgentStateClient;

use super::EventBridge;

fn objects_of(cfg: &SessionConfig) -> Vec<(String, String, String)> {
    cfg.subscriptions
        .iter()
        .filter_map(|s| match &s.source {
            SubscriptionSource::ObjectEvent { object, event } => {
                Some((s.id.clone(), object.clone(), event.clone()))
            }
            _ => None,
        })
        .collect()
}

pub struct KEventBridge {
    pub client: Arc<KEventClient>,
    pub agent: Arc<dyn AgentStateClient>,
    pub who: String,
}

#[async_trait]
impl EventBridge for KEventBridge {
    fn wants(&self, cfg: &SessionConfig) -> bool {
        !objects_of(cfg).is_empty()
    }

    async fn run(&self, cfg: SessionConfig) -> Result<()> {
        let sid = cfg.session.session_id.clone();
        let subs = objects_of(&cfg);
        let reader = self
            .client
            .create_event_reader(subs.iter().map(|(_, o, _)| o.clone()).collect())
            .await
            .map_err(|e| OpenDanError::Channel(format!("kevent reader: {e}")))?;
        loop {
            let ev = match reader.pull_event(Some(30_000)).await {
                Ok(Some(ev)) => ev,
                Ok(None) => continue,
                Err(e) => {
                    let _ = reader.close().await;
                    return Err(OpenDanError::Channel(format!("kevent: {e}")));
                }
            };
            let text = |k: &str| ev.data.get(k).and_then(Value::as_str).map(str::to_string);
            for (sub, object, event) in subs.iter().filter(|(_, o, _)| o == &ev.eventid) {
                let name = text("event").unwrap_or_else(|| {
                    if event.is_empty() || event == "*" {
                        "changed".to_string()
                    } else {
                        event.clone()
                    }
                });
                let mut summary = text("summary").unwrap_or_else(|| {
                    format!("{object} {name}: {}", ev.data)
                });
                if summary.len() > MAX_EVENT_SUMMARY_BYTES {
                    let mut end = MAX_EVENT_SUMMARY_BYTES;
                    while !summary.is_char_boundary(end) {
                        end -= 1;
                    }
                    summary.truncate(end);
                }
                let seq = ev.data.get("seq").and_then(Value::as_u64);
                let input = PostedInput::event(
                    &self.who,
                    format!("kevent:{}:{}", ev.eventid, seq.unwrap_or(ev.timestamp)),
                    AgentEvent {
                        subscription_id: Some(sub.clone()),
                        source: EventSource::new("object", object.clone()),
                        event: name,
                        seq,
                        summary,
                        data_ref: text("data_ref"),
                        terminal: false,
                    },
                );
                match self.agent.sessions().post_input(&sid, &input).await {
                    Ok(_) => {}
                    Err(OpenDanError::SessionFinished(_)) => {
                        let _ = reader.close().await;
                        return Ok(());
                    }
                    Err(e) => log::warn!("kevent {} of {sid}: {e}", ev.eventid),
                }
            }
        }
    }
}
