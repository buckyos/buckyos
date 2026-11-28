use buckyos_api::{AiccEventLevel, AiccRoutingCommand, AiccRoutingCommandStatus, AiccSystemEvent};
use serde_json::{json, Value};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use super::inference::now_ms;

const MAX_EVENTS: usize = 200;

pub(crate) const EVENT_ROUTING_COMMAND_STALE: &str = "routing_command_stale";
pub(crate) const EVENT_ROUTING_COMMAND_RECOVERED: &str = "routing_command_recovered";
pub(crate) const EVENT_ROUTING_COMMANDS_UPDATED: &str = "routing_commands_updated";
pub(crate) const EVENT_REGISTRY_BUILD_FAILED: &str = "registry_build_failed";
pub(crate) const EVENT_REGISTRY_RECOVERED: &str = "registry_recovered";
pub(crate) const EVENT_UNAVAILABLE_FALLBACK: &str = "unavailable_fallback";
pub(crate) const EVENT_USECASE_UPDATED: &str = "usecase_updated";
pub(crate) const EVENT_MODEL_FAILING: &str = "model_failing";
pub(crate) const EVENT_MODEL_RECOVERED: &str = "model_recovered";
pub(crate) const EVENT_PROVIDER_MARKED_FAULTY: &str = "provider_marked_faulty";
pub(crate) const EVENT_PROVIDER_FAULT_CLEARED: &str = "provider_fault_cleared";
pub(crate) const EVENT_USECASE_MODEL_MISSING: &str = "usecase_model_missing";
pub(crate) const EVENT_USECASE_MODEL_RECOVERED: &str = "usecase_model_recovered";

#[derive(Default)]
struct EventLogState {
    next_id: u64,
    events: VecDeque<AiccSystemEvent>,
    stale: BTreeMap<String, AiccSystemEvent>,
    build_error: Option<String>,
    /// Zone-layer usecase → model that is in no inventory.
    missing_models: BTreeMap<String, String>,
}

#[derive(Default)]
pub(crate) struct AiccEventLog {
    state: Mutex<EventLogState>,
}

impl AiccEventLog {
    pub(crate) fn record(
        &self,
        level: AiccEventLevel,
        kind: &str,
        message: impl Into<String>,
        details: Value,
    ) -> AiccSystemEvent {
        let mut state = self.state.lock().expect("event log lock");
        push_event(&mut state, level, kind, message.into(), details)
    }

    pub(crate) fn observe_routing_commands(&self, status: &[AiccRoutingCommandStatus]) {
        let mut state = self.state.lock().expect("event log lock");
        if let Some(error) = state.build_error.take() {
            push_event(
                &mut state,
                AiccEventLevel::Info,
                EVENT_REGISTRY_RECOVERED,
                "model registry rebuilt successfully".to_string(),
                json!({ "previous_error": error }),
            );
        }
        let current = status
            .iter()
            .filter_map(|entry| {
                entry
                    .stale_reason
                    .as_ref()
                    .map(|reason| (command_subject(&entry.command), (entry, reason)))
            })
            .collect::<BTreeMap<_, _>>();
        let recovered = state
            .stale
            .keys()
            .filter(|key| !current.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in recovered {
            let previous = state.stale.remove(&key).expect("stale entry exists");
            push_event(
                &mut state,
                AiccEventLevel::Info,
                EVENT_ROUTING_COMMAND_RECOVERED,
                format!("routing adjustment applies again: {key}"),
                json!({ "command": previous.details["command"].clone() }),
            );
        }
        for (key, (entry, reason)) in current {
            if state.stale.contains_key(&key) {
                continue;
            }
            let event = push_event(
                &mut state,
                AiccEventLevel::Warning,
                EVENT_ROUTING_COMMAND_STALE,
                format!("routing adjustment no longer applies: {reason}"),
                json!({ "command": entry.command, "reason": reason }),
            );
            state.stale.insert(key, event);
        }
    }

    /// Report once when a model the Zone default chose for a usecase leaves
    /// every inventory, and once when it is back. The choice is kept; the
    /// usecase follows the default meanwhile.
    pub(crate) fn observe_usecase_models(&self, missing: BTreeMap<String, String>) {
        let mut state = self.state.lock().expect("event log lock");
        let recovered = state
            .missing_models
            .iter()
            .filter(|(usecase, model)| missing.get(*usecase) != Some(model))
            .map(|(usecase, model)| (usecase.clone(), model.clone()))
            .collect::<Vec<_>>();
        for (usecase, model) in recovered {
            push_event(
                &mut state,
                AiccEventLevel::Info,
                EVENT_USECASE_MODEL_RECOVERED,
                format!("{model} is available again for {usecase}"),
                json!({ "usecase": usecase, "exact_model": model }),
            );
        }
        for (usecase, model) in &missing {
            if state.missing_models.get(usecase) == Some(model) {
                continue;
            }
            push_event(
                &mut state,
                AiccEventLevel::Warning,
                EVENT_USECASE_MODEL_MISSING,
                format!("{model} chosen for {usecase} is no longer in any inventory"),
                json!({ "usecase": usecase, "exact_model": model }),
            );
        }
        state.missing_models = missing;
    }

    /// A model started failing or recovered (reported once per period).
    pub(crate) fn observe_model_health(&self, transition: crate::health::HealthTransition) {
        let mut state = self.state.lock().expect("event log lock");
        match transition {
            crate::health::HealthTransition::Failing {
                exact_model,
                reason,
            } => push_event(
                &mut state,
                AiccEventLevel::Warning,
                EVENT_MODEL_FAILING,
                format!("{exact_model} keeps failing ({reason}); its usecases keep routing to it"),
                json!({ "exact_model": exact_model, "reason": reason }),
            ),
            crate::health::HealthTransition::Recovered { exact_model } => push_event(
                &mut state,
                AiccEventLevel::Info,
                EVENT_MODEL_RECOVERED,
                format!("{exact_model} succeeded again"),
                json!({ "exact_model": exact_model }),
            ),
        };
    }

    pub(crate) fn observe_registry_error(&self, error: &str) {
        let mut state = self.state.lock().expect("event log lock");
        if state.build_error.as_deref() == Some(error) {
            return;
        }
        state.build_error = Some(error.to_owned());
        push_event(
            &mut state,
            AiccEventLevel::Error,
            EVENT_REGISTRY_BUILD_FAILED,
            format!("model registry rebuild failed: {error}"),
            json!({ "error": error }),
        );
    }

    pub(crate) fn list(&self, limit: usize) -> (Vec<AiccSystemEvent>, Vec<AiccSystemEvent>) {
        let state = self.state.lock().expect("event log lock");
        let events = state.events.iter().rev().take(limit).cloned().collect();
        let active = state.stale.values().cloned().collect();
        (events, active)
    }
}

fn push_event(
    state: &mut EventLogState,
    level: AiccEventLevel,
    kind: &str,
    message: String,
    details: Value,
) -> AiccSystemEvent {
    state.next_id += 1;
    let event = AiccSystemEvent {
        event_id: state.next_id,
        created_at_ms: now_ms(),
        level,
        kind: kind.to_owned(),
        message,
        details,
    };
    if state.events.len() >= MAX_EVENTS {
        state.events.pop_front();
    }
    state.events.push_back(event.clone());
    event
}

pub(crate) fn command_subject(command: &AiccRoutingCommand) -> String {
    match command {
        AiccRoutingCommand::VendorFactor { vendor, .. } => format!("vendor:{vendor}"),
        AiccRoutingCommand::SpecFactor { spec, .. } => format!("spec:{spec}"),
        AiccRoutingCommand::ModelFactor { vendor, model, .. } => format!("model:{vendor}/{model}"),
        AiccRoutingCommand::ItemWeight { path, item, .. } => format!("item:{path}/{item}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status(command: AiccRoutingCommand, stale: Option<&str>) -> AiccRoutingCommandStatus {
        AiccRoutingCommandStatus {
            command,
            matched_items: 0,
            stale_reason: stale.map(str::to_owned),
        }
    }

    #[test]
    fn stale_commands_are_reported_once_and_recover() {
        let log = AiccEventLog::default();
        let command = AiccRoutingCommand::ItemWeight {
            path: "llm.chat".into(),
            item: "gpt-mini".into(),
            weight: 3.0,
        };
        let stale = [status(
            command.clone(),
            Some("item gpt-mini no longer exists"),
        )];
        log.observe_routing_commands(&stale);
        log.observe_routing_commands(&stale);
        let (events, active) = log.list(10);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, EVENT_ROUTING_COMMAND_STALE);
        assert_eq!(active.len(), 1);

        log.observe_routing_commands(&[status(command, None)]);
        let (events, active) = log.list(10);
        assert_eq!(events[0].kind, EVENT_ROUTING_COMMAND_RECOVERED);
        assert!(active.is_empty());
    }

    #[test]
    fn registry_errors_are_deduplicated_until_recovery() {
        let log = AiccEventLog::default();
        log.observe_registry_error("boom");
        log.observe_registry_error("boom");
        log.observe_routing_commands(&[]);
        let (events, _) = log.list(10);
        assert_eq!(
            events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            vec![EVENT_REGISTRY_RECOVERED, EVENT_REGISTRY_BUILD_FAILED]
        );
    }
}
