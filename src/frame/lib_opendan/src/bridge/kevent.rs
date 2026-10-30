//! kevent bridge: `object_event` subscriptions of a session become
//! `AgentEvent{source: object:<object>}` posted through the registry.
//!
//! The subscription's `object` is an `obj://` URL; its kevent key is
//! `/obj/<host>/<path>/<event>` (doc/key url.md §7), every event of the object
//! when `event` is empty or `*`. Event data fields used when present: `seq`,
//! `summary`, `data_ref`; the event name is the last key segment.

use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::KEventClient;
use serde_json::Value;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::state::AgentStateClient;

use super::EventBridge;

/// `(subscription id, object url, kevent pattern)` of the session's object subscriptions.
fn objects_of(cfg: &SessionConfig) -> Vec<(String, String, String)> {
    cfg.subscriptions
        .iter()
        .filter_map(|s| match &s.source {
            SubscriptionSource::ObjectEvent { object, event } => match kevent_pattern(object, event) {
                Ok(pattern) => Some((s.id.clone(), object.clone(), pattern)),
                Err(e) => {
                    log::warn!("object subscription {} of {object}: {e}", s.id);
                    None
                }
            },
            _ => None,
        })
        .collect()
}

pub(crate) fn kevent_pattern(object: &str, event: &str) -> std::result::Result<String, buckyos_api::ObjectUrlError> {
    let url = buckyos_api::parse_object_url(object)?;
    if event.is_empty() || event == "*" {
        url.object_event_pattern()
    } else {
        url.object_event_key(event)
    }
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
            .create_event_reader(subs.iter().map(|(_, _, pattern)| pattern.clone()).collect())
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
            for (sub, object, _) in subs
                .iter()
                .filter(|(_, _, pattern)| buckyos_api::match_global_pattern(pattern, &ev.eventid))
            {
                let name = ev.eventid.rsplit('/').next().unwrap_or("changed").to_string();
                // The rendering names the source and the event; the summary is what happened.
                let mut summary = text("summary").unwrap_or_else(|| ev.data.to_string());
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
