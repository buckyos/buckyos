//! Timer bridge: `timer:<name>` subscriptions of a session become
//! `AgentEvent`s posted through the registry (the session decides Input /
//! Observe by its subscription, like for any producer).
//!
//! The interval of a timer is `extensions.opendan.timers.<name>.every_secs`
//! of the session (default [`DEFAULT_TIMER_SECS`]). The key of a firing is
//! `timer:<name>:<tick>` with `tick = now / interval`: a restarted bridge
//! never delivers the same tick twice.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::state::AgentStateClient;

use super::EventBridge;

pub const DEFAULT_TIMER_SECS: u64 = 300;

fn timers_of(cfg: &SessionConfig) -> Vec<(String, String, u64)> {
    cfg.subscriptions
        .iter()
        .filter_map(|s| match &s.source {
            SubscriptionSource::Timer { name } => {
                let every = cfg
                    .extensions
                    .get("opendan")
                    .and_then(|o| o.get("timers"))
                    .and_then(|t| t.get(name))
                    .and_then(|t| t.get("every_secs"))
                    .and_then(|v| v.as_u64())
                    .filter(|n| *n > 0)
                    .unwrap_or(DEFAULT_TIMER_SECS);
                Some((s.id.clone(), name.clone(), every))
            }
            _ => None,
        })
        .collect()
}

pub struct TimerBridge {
    pub agent: Arc<dyn AgentStateClient>,
    pub who: String,
}

#[async_trait]
impl EventBridge for TimerBridge {
    fn wants(&self, cfg: &SessionConfig) -> bool {
        !timers_of(cfg).is_empty()
    }

    async fn run(&self, cfg: SessionConfig) -> Result<()> {
        let sid = cfg.session.session_id.clone();
        let timers = timers_of(&cfg);
        let mut last: Vec<u64> = timers
            .iter()
            .map(|(_, _, every)| crate::now_ms() / 1000 / every)
            .collect();
        loop {
            let now = crate::now_ms() / 1000;
            let next = timers
                .iter()
                .map(|(_, _, every)| every - now % every)
                .min()
                .unwrap_or(DEFAULT_TIMER_SECS);
            tokio::time::sleep(Duration::from_secs(next.max(1))).await;
            let now = crate::now_ms() / 1000;
            for (i, (sub, name, every)) in timers.iter().enumerate() {
                let tick = now / every;
                if tick <= last[i] {
                    continue;
                }
                last[i] = tick;
                let input = PostedInput::event(
                    &self.who,
                    format!("timer:{name}:{tick}"),
                    AgentEvent {
                        subscription_id: Some(sub.clone()),
                        source: EventSource::new("timer", name.clone()),
                        event: "fired".into(),
                        seq: Some(tick),
                        summary: format!("timer {name} fired (every {every}s)"),
                        data_ref: None,
                        terminal: false,
                    },
                );
                match self.agent.sessions().post_input(&sid, &input).await {
                    Ok(_) => {}
                    Err(OpenDanError::SessionFinished(_)) => return Ok(()),
                    // A full bus is not confirmed upstream: the tick is lost
                    // like any coalesced timer firing.
                    Err(e) => log::warn!("timer {name} of {sid}: {e}"),
                }
            }
        }
    }
}
