//! Hints and dedup of an AgentToolResult (freeze TODO F11, simple Work
//! Session TODO §3.8).
//!
//! A result may carry guidance for the model (`hints`) and a dedup key
//! (`dedup`). Both are applied by the tool manager, never by the waist:
//! - a hint with `once` is shown once per session under its key;
//! - a result whose dedup key was already shown in this run has its output
//!   and detail replaced by the dedup hint.
//!
//! What was shown is kept in a [`HintLedger`] the host provides (`once` per
//! session, `dedup` per run and cleared when the history is rewritten). The
//! shown hints are rendered apart from the output, in one structure for
//! both loops:
//!
//! ```text
//! <output>…</output>
//! <runtime_hints>
//! - …
//! </runtime_hints>
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::AgentToolResult;

/// Namespace of the hints shown once per session.
pub const HINT_NS_ONCE: &str = "once";
/// Namespace of the dedup keys shown in this run.
pub const HINT_NS_DEDUP: &str = "dedup";

/// What the model was already shown. A host dependency: xllm keeps it in
/// run.json, an Agent Session in its state.
pub trait HintLedger: Send + Sync {
    fn seen(&self, ns: &str, key: &str) -> bool;
    fn mark(&self, ns: &str, key: &str);
    /// Forget a namespace (`dedup` when the history is rewritten).
    fn clear(&self, ns: &str);
    /// The keys of `ns`, for a host that persists them.
    fn keys(&self, ns: &str) -> Vec<String> {
        let _ = ns;
        Vec::new()
    }
}

/// A ledger in memory (the default; a host seeds and reads it back).
#[derive(Debug, Default)]
pub struct MemoryHintLedger {
    inner: Mutex<BTreeMap<String, BTreeSet<String>>>,
}

impl MemoryHintLedger {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// A ledger that already holds `keys` in `ns`.
    pub fn seeded<I: IntoIterator<Item = String>>(ns: &str, keys: I) -> Arc<Self> {
        let me = Self::default();
        me.inner
            .lock()
            .expect("hint ledger")
            .entry(ns.to_string())
            .or_default()
            .extend(keys);
        Arc::new(me)
    }

}

impl HintLedger for MemoryHintLedger {
    fn seen(&self, ns: &str, key: &str) -> bool {
        self.inner
            .lock()
            .expect("hint ledger")
            .get(ns)
            .is_some_and(|k| k.contains(key))
    }

    fn mark(&self, ns: &str, key: &str) {
        self.inner
            .lock()
            .expect("hint ledger")
            .entry(ns.to_string())
            .or_default()
            .insert(key.to_string());
    }

    fn clear(&self, ns: &str) {
        self.inner.lock().expect("hint ledger").remove(ns);
    }

    fn keys(&self, ns: &str) -> Vec<String> {
        self.inner
            .lock()
            .expect("hint ledger")
            .get(ns)
            .map(|k| k.iter().cloned().collect())
            .unwrap_or_default()
    }
}

/// Apply `ledger` to `result`: a dedup key already shown replaces the
/// output and detail with the dedup hint; a `once` hint already shown is
/// dropped. What is shown now is marked. `result.hints` keeps the hints to
/// show; their texts are returned.
pub fn apply_hints(result: &mut AgentToolResult, ledger: &dyn HintLedger) -> Vec<String> {
    if let Some(dedup) = &result.dedup {
        if ledger.seen(HINT_NS_DEDUP, &dedup.key) {
            result.output = Some(dedup.hint.clone());
            result.summary = dedup.hint.clone();
            result.details = json!({ "dedup": dedup.key });
            result.partial_output = None;
        } else {
            ledger.mark(HINT_NS_DEDUP, &dedup.key);
        }
    }
    let mut shown = Vec::new();
    result.hints.retain(|h| {
        if h.text.trim().is_empty() {
            return false;
        }
        match h.once.as_deref().filter(|k| !k.is_empty()) {
            Some(key) if ledger.seen(HINT_NS_ONCE, key) => false,
            Some(key) => {
                ledger.mark(HINT_NS_ONCE, key);
                true
            }
            None => true,
        }
    });
    for h in &result.hints {
        shown.push(h.text.trim().to_string());
    }
    shown
}

/// `text` with the hints shown after it, in the fixed structure; `text`
/// unchanged when there are none.
pub fn render_with_hints(text: &str, hints: &[String]) -> String {
    if hints.is_empty() {
        return text.to_string();
    }
    let mut s = String::with_capacity(text.len() + 64);
    s.push_str("<output>\n");
    s.push_str(text.trim_end());
    s.push_str("\n</output>\n");
    s.push_str(&render_hint_block(hints));
    s
}

/// The `<runtime_hints>` block alone.
pub fn render_hint_block(hints: &[String]) -> String {
    let mut s = String::from("<runtime_hints>\n");
    for h in hints {
        s.push_str("- ");
        s.push_str(h.trim());
        s.push('\n');
    }
    s.push_str("</runtime_hints>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentToolStatus, build_builtin_tool_result};

    fn result() -> AgentToolResult {
        build_builtin_tool_result(json!({"x": 1}), "t", "ok")
            .with_status(AgentToolStatus::Success)
            .with_output("body")
    }

    #[test]
    fn once_hints_show_once_and_plain_hints_always() {
        let ledger = MemoryHintLedger::new();
        let mut a = result()
            .with_hint("follow it with X", Some("shell.task_followup"))
            .with_hint("always", None);
        assert_eq!(
            apply_hints(&mut a, ledger.as_ref()),
            vec!["follow it with X".to_string(), "always".to_string()]
        );
        let mut b = result()
            .with_hint("follow it with X", Some("shell.task_followup"))
            .with_hint("always", None);
        assert_eq!(apply_hints(&mut b, ledger.as_ref()), vec!["always".to_string()]);
        assert_eq!(b.hints.len(), 1);
    }

    #[test]
    fn dedup_replaces_output_from_the_second_time_and_clears() {
        let ledger = MemoryHintLedger::new();
        let mut a = result().with_dedup("read:/a:1", "unchanged since you read it");
        apply_hints(&mut a, ledger.as_ref());
        assert_eq!(a.output.as_deref(), Some("body"));
        let mut b = result().with_dedup("read:/a:1", "unchanged since you read it");
        apply_hints(&mut b, ledger.as_ref());
        assert_eq!(b.output.as_deref(), Some("unchanged since you read it"));
        assert_eq!(b.details, json!({"dedup": "read:/a:1"}));
        ledger.clear(HINT_NS_DEDUP);
        let mut c = result().with_dedup("read:/a:1", "unchanged since you read it");
        apply_hints(&mut c, ledger.as_ref());
        assert_eq!(c.output.as_deref(), Some("body"), "history rewritten: shown again");
    }

    #[test]
    fn rendering_keeps_hints_out_of_the_output() {
        assert_eq!(render_with_hints("x", &[]), "x");
        assert_eq!(
            render_with_hints("line\n", &["h1".into(), "h2".into()]),
            "<output>\nline\n</output>\n<runtime_hints>\n- h1\n- h2\n</runtime_hints>"
        );
    }
}
