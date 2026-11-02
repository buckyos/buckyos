//! The `intent` argument of a tool call (simple Work Session TODO §3.6): one
//! sentence on why the call is made now and what it should give. Host
//! metadata — kept with the call's record (snapshot, worklog), shown in the
//! progress of the Turn, never passed to the tool.
//!
//! [`IntentPolicy`] says whether a tool's schema offers it: `Optional` for
//! the runtime's standard tools (`read` / `read_file`, `write_file`,
//! `edit_file`, `shell`), `Required` for every tool of a context whose
//! behavior asks for it (`[capabilities] call_intent`), `Off` otherwise.

use buckyos_api::AiToolCall;
use llm_context::deps::ToolSpecLite;
use serde_json::{json, Value};

/// The argument a call states its intent in.
pub const INTENT_ARG: &str = "intent";

const INTENT_REQUIRED_DESCRIPTION: &str = "Why you make this call now and what you expect it to give you, in one short sentence; name the Todo and the accepted evidence it rests on when there are. The purpose of the action, not a transcript of your reasoning.";

const INTENT_OPTIONAL_DESCRIPTION: &str = "Optional: why you make this call now and what you expect it to give you, in one short sentence. Shown as the progress of your work.";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum IntentPolicy {
    #[default]
    Off,
    Optional,
    Required,
}

/// `spec` offering `intent` per `policy`.
pub fn with_intent(mut spec: ToolSpecLite, policy: IntentPolicy) -> ToolSpecLite {
    if policy == IntentPolicy::Off {
        return spec;
    }
    let schema = &mut spec.args_schema;
    if !schema.is_object() {
        *schema = json!({ "type": "object" });
    }
    if !schema.get("properties").is_some_and(Value::is_object) {
        schema["properties"] = json!({});
    }
    match policy {
        IntentPolicy::Required => {
            schema["properties"][INTENT_ARG] = json!({ "type": "string", "minLength": 1, "description": INTENT_REQUIRED_DESCRIPTION });
            match schema.get_mut("required").and_then(Value::as_array_mut) {
                Some(r) if r.iter().any(|v| v == INTENT_ARG) => {}
                Some(r) => r.insert(0, json!(INTENT_ARG)),
                None => schema["required"] = json!([INTENT_ARG]),
            }
        }
        IntentPolicy::Optional => {
            schema["properties"][INTENT_ARG] =
                json!({ "type": "string", "description": INTENT_OPTIONAL_DESCRIPTION });
            if let Some(r) = schema.get_mut("required").and_then(Value::as_array_mut) {
                r.retain(|v| v != INTENT_ARG);
            }
        }
        IntentPolicy::Off => {}
    }
    spec
}

/// The call as its tool runs it (`intent` taken out) and the intent it
/// stated. `Err`: the correction a call gets instead of running (a
/// `Required` intent missing or blank). `Off` leaves the call as it is.
pub fn take_intent(
    call: &AiToolCall,
    policy: IntentPolicy,
) -> Result<(AiToolCall, Option<String>), String> {
    let missing = |what: &str| {
        format!(
            "{what}: this call was not executed. Every function call of this context carries `intent`, one short non-empty sentence saying why you make the call now and what you expect it to give you. Call it again with `intent`."
        )
    };
    match policy {
        IntentPolicy::Off => Ok((call.clone(), None)),
        IntentPolicy::Optional => {
            let mut exec = call.clone();
            let intent = match exec.args.remove(INTENT_ARG) {
                Some(Value::String(s)) if !s.trim().is_empty() => Some(s.trim().to_string()),
                _ => None,
            };
            Ok((exec, intent))
        }
        IntentPolicy::Required => match call.args.get(INTENT_ARG) {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                let mut exec = call.clone();
                exec.args.remove(INTENT_ARG);
                Ok((exec, Some(s.trim().to_string())))
            }
            Some(Value::String(_)) => Err(missing("`intent` is blank")),
            Some(_) => Err(missing("`intent` must be a string")),
            None => Err(missing("`intent` is missing")),
        },
    }
}

/// The intent a call states, whatever the policy (records, progress).
pub fn intent_of(call: &AiToolCall) -> Option<String> {
    call.args
        .get(INTENT_ARG)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ToolSpecLite {
        ToolSpecLite {
            name: "shell".into(),
            description: "d".into(),
            args_schema: json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
        }
    }

    fn call(args: Value) -> AiToolCall {
        AiToolCall {
            name: "shell".into(),
            args: args.as_object().unwrap().clone().into_iter().collect(),
            call_id: "c1".into(),
        }
    }

    #[test]
    fn optional_intent_is_offered_not_required_and_stripped() {
        let s = with_intent(spec(), IntentPolicy::Optional);
        assert!(s.args_schema["properties"][INTENT_ARG]["description"]
            .as_str()
            .unwrap()
            .starts_with("Optional:"));
        assert_eq!(s.args_schema["required"], json!(["command"]));
        let (exec, intent) =
            take_intent(&call(json!({"command": "ls", "intent": " look around "})), IntentPolicy::Optional).unwrap();
        assert_eq!(intent.as_deref(), Some("look around"));
        assert!(!exec.args.contains_key(INTENT_ARG));
        let (exec, intent) =
            take_intent(&call(json!({"command": "ls"})), IntentPolicy::Optional).unwrap();
        assert!(intent.is_none());
        assert_eq!(exec.args.len(), 1);
    }

    #[test]
    fn required_intent_is_checked() {
        let s = with_intent(spec(), IntentPolicy::Required);
        assert_eq!(s.args_schema["required"], json!([INTENT_ARG, "command"]));
        assert!(take_intent(&call(json!({"command": "ls"})), IntentPolicy::Required)
            .unwrap_err()
            .contains("`intent` is missing"));
        assert!(take_intent(&call(json!({"command": "ls", "intent": " "})), IntentPolicy::Required)
            .unwrap_err()
            .contains("blank"));
        assert_eq!(with_intent(spec(), IntentPolicy::Off).args_schema, spec().args_schema);
    }
}
