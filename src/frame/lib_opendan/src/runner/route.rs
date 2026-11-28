//! The session's AICC routing group and the fallbacks its answers report
//! (AICC user-centric routing TODO §3.5, §3.6, §4.6): every inference of a
//! run carries the routing group in force for the session, looked up along
//! `origin.parent_session` at the call, and an answer AICC marks as not
//! coming from the usecase's own model is noted in the session once per
//! usecase and model (`state.route_fallbacks`).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use buckyos_api::{AiResponse, RouteTrace};
use llm_context::deps::{LlmClient, LlmInference, LlmInferenceRequest};
use llm_context::error::LLMComputeError;

use crate::protocol::{RouteFallbackKind, RouteFallbackNote, SessionState};
use crate::state::AgentStateClient;

use super::shared::Shared;

/// The fallbacks a session ran into, seeded from its state; written into
/// the state with every commit ([`RouteNotes::store`]).
#[derive(Debug, Default)]
pub struct RouteNotes {
    notes: Mutex<Vec<RouteFallbackNote>>,
}

impl RouteNotes {
    pub fn of(state: &SessionState) -> Self {
        Self {
            notes: Mutex::new(state.route_fallbacks.clone()),
        }
    }

    /// `true`: first time for its usecase and model in this session.
    pub fn record(&self, note: RouteFallbackNote) -> bool {
        let mut notes = self.notes.lock().expect("route notes");
        if notes
            .iter()
            .any(|n| n.usecase == note.usecase && n.model == note.model)
        {
            return false;
        }
        notes.push(note);
        true
    }

    /// Write the notes not yet in `state` (into the open Turn) and return
    /// them.
    pub fn store(&self, state: &mut SessionState) -> Vec<RouteFallbackNote> {
        let notes = self.notes.lock().expect("route notes");
        if notes.len() == state.route_fallbacks.len() {
            return Vec::new();
        }
        let turn = state.open_turn.as_ref().map(|t| t.index);
        let fresh: Vec<RouteFallbackNote> = notes
            .iter()
            .filter(|n| {
                !state
                    .route_fallbacks
                    .iter()
                    .any(|s| s.usecase == n.usecase && s.model == n.model)
            })
            .map(|n| RouteFallbackNote {
                turn: n.turn.or(turn),
                ..n.clone()
            })
            .collect();
        state.route_fallbacks.extend(fresh.iter().cloned());
        fresh
    }
}

/// The note an answer's route calls for (`usecase`: what the call asked
/// for).
pub fn note_of(usecase: &str, trace: &RouteTrace) -> Option<RouteFallbackNote> {
    let fallback = trace.fallback.as_ref()?;
    let (kind, usecase, model, replaced) = match fallback.kind {
        buckyos_api::RouteFallbackKind::Coverage => (
            RouteFallbackKind::Coverage,
            fallback.from.clone(),
            trace.final_model.clone().unwrap_or_else(|| fallback.to.clone()),
            None,
        ),
        buckyos_api::RouteFallbackKind::Unavailable => (
            RouteFallbackKind::Unavailable,
            usecase.to_string(),
            fallback.to.clone(),
            Some(fallback.from.clone()),
        ),
    };
    Some(RouteFallbackNote {
        kind,
        usecase,
        model,
        replaced,
        turn: None,
        at_ms: crate::now_ms(),
    })
}

/// The run's client with the session's routing group.
pub struct RoutedLlm {
    inner: Arc<dyn LlmClient>,
    agent: Arc<dyn AgentStateClient>,
    sid: String,
    notes: Arc<RouteNotes>,
}

impl RoutedLlm {
    pub fn new(
        inner: Arc<dyn LlmClient>,
        agent: Arc<dyn AgentStateClient>,
        sid: impl Into<String>,
        notes: Arc<RouteNotes>,
    ) -> Self {
        Self {
            inner,
            agent,
            sid: sid.into(),
            notes,
        }
    }
}

#[async_trait]
impl LlmClient for RoutedLlm {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.infer_routed(req).await.map(|r| r.response)
    }

    async fn infer_routed(
        &self,
        mut req: LlmInferenceRequest,
    ) -> Result<LlmInference, LLMComputeError> {
        if req.route_group.is_none() {
            req.route_group = match crate::api::route_group_of(self.agent.as_ref(), &self.sid).await {
                Ok(group) => group.map(|g| g.group_id),
                // An unreadable chain never stops the inference.
                Err(e) => {
                    log::warn!("session {}: routing group not read: {e}", self.sid);
                    None
                }
            };
        }
        let usecase = req.model_alias.clone();
        let result = self.inner.infer_routed(req).await?;
        if let Some(note) = result.route_trace.as_ref().and_then(|t| note_of(&usecase, t)) {
            let (kind, usecase, model) = (note.kind, note.usecase.clone(), note.model.clone());
            if self.notes.record(note) {
                log::warn!("session {}: {usecase} answered by {model} ({kind:?} fallback)", self.sid);
            }
        }
        Ok(result)
    }
}

/// `llm` with the session's routing group and fallback notes.
pub(super) fn routed(sh: &Shared, llm: Arc<dyn LlmClient>) -> Arc<dyn LlmClient> {
    Arc::new(RoutedLlm::new(
        llm,
        sh.deps.agent.clone(),
        sh.dir.sid(),
        sh.route_notes.clone(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::WorklogBoundary;

    fn trace(value: serde_json::Value) -> RouteTrace {
        serde_json::from_value(value).unwrap()
    }

    fn note(usecase: &str, model: &str) -> RouteFallbackNote {
        RouteFallbackNote {
            kind: RouteFallbackKind::Coverage,
            usecase: usecase.into(),
            model: model.into(),
            replaced: None,
            turn: None,
            at_ms: 1,
        }
    }

    #[test]
    fn a_route_without_fallback_needs_no_note() {
        assert_eq!(note_of("llm.plan", &trace(serde_json::json!({ "final_model": "m@p" }))), None);
    }

    #[test]
    fn coverage_names_the_usecase_and_the_model_that_answered() {
        let n = note_of(
            "llm.plan",
            &trace(serde_json::json!({
                "final_model": "small@local",
                "fallback": { "kind": "coverage", "from": "llm.plan", "to": "llm.fallback" }
            })),
        )
        .unwrap();
        assert_eq!((n.kind, n.usecase.as_str(), n.model.as_str(), n.replaced), (RouteFallbackKind::Coverage, "llm.plan", "small@local", None));
        let n = note_of(
            "llm.code",
            &trace(serde_json::json!({
                "final_model": "b@two",
                "fallback": { "kind": "unavailable", "from": "a@one", "to": "b@two", "policy": "next_model" }
            })),
        )
        .unwrap();
        assert_eq!(n.kind, RouteFallbackKind::Unavailable);
        assert_eq!((n.usecase.as_str(), n.model.as_str(), n.replaced.as_deref()), ("llm.code", "b@two", Some("a@one")));
    }

    #[test]
    fn a_usecase_and_model_is_noted_once_per_session() {
        let mut state = SessionState::initial(WorklogBoundary { committed_seq: 0, committed_bytes: 0 }, 1);
        let notes = RouteNotes::of(&state);
        assert!(notes.record(note("llm.plan", "a@p")));
        assert!(!notes.record(note("llm.plan", "a@p")), "the same pair again");
        assert!(notes.record(note("llm.plan", "b@p")), "another model");
        state.open_turn = Some(
            serde_json::from_value(serde_json::json!({ "index": 3, "run_id": "r", "input_seq": 1, "at_ms": 1 })).unwrap(),
        );
        let fresh = notes.store(&mut state);
        assert_eq!(fresh.len(), 2);
        assert!(fresh.iter().all(|n| n.turn == Some(3)));
        assert!(notes.store(&mut state).is_empty(), "stored once");
        assert_eq!(state.route_fallbacks.len(), 2);

        // A later drive starts from the state: still once.
        let again = RouteNotes::of(&state);
        assert!(!again.record(note("llm.plan", "a@p")));
        assert!(again.store(&mut state).is_empty());
    }
}
