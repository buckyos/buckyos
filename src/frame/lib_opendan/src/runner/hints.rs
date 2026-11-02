//! The session's `HintLedger` (simple Work Session TODO §3.8, B13): hints
//! a result shows once per session are remembered in the session state
//! (`hints_once`, written with every commit); dedup keys live with the
//! drive and are forgotten when the history is rewritten.

use std::collections::BTreeSet;
use std::sync::Mutex;

use agent_tool::{HintLedger, HINT_NS_DEDUP, HINT_NS_ONCE};

use crate::protocol::SessionState;

#[derive(Debug, Default)]
pub struct SessionHintLedger {
    once: Mutex<BTreeSet<String>>,
    dedup: Mutex<BTreeSet<String>>,
}

impl SessionHintLedger {
    /// The ledger of a session whose state remembers `state.hints_once`.
    pub fn of(state: &SessionState) -> Self {
        Self {
            once: Mutex::new(state.hints_once.clone()),
            dedup: Mutex::new(BTreeSet::new()),
        }
    }

    /// Write what was shown once into the state about to be committed.
    pub fn store(&self, state: &mut SessionState) {
        let once = self.once.lock().expect("hints");
        if state.hints_once != *once {
            state.hints_once = once.clone();
        }
    }

    /// The history was rewritten, or a new context starts.
    pub fn clear_dedup(&self) {
        self.dedup.lock().expect("hints").clear();
    }

    fn set(&self, ns: &str) -> Option<&Mutex<BTreeSet<String>>> {
        match ns {
            HINT_NS_ONCE => Some(&self.once),
            HINT_NS_DEDUP => Some(&self.dedup),
            _ => None,
        }
    }
}

impl HintLedger for SessionHintLedger {
    fn seen(&self, ns: &str, key: &str) -> bool {
        self.set(ns)
            .is_some_and(|s| s.lock().expect("hints").contains(key))
    }

    fn mark(&self, ns: &str, key: &str) {
        if let Some(s) = self.set(ns) {
            s.lock().expect("hints").insert(key.to_string());
        }
    }

    fn clear(&self, ns: &str) {
        if let Some(s) = self.set(ns) {
            s.lock().expect("hints").clear();
        }
    }

    fn keys(&self, ns: &str) -> Vec<String> {
        self.set(ns)
            .map(|s| s.lock().expect("hints").iter().cloned().collect())
            .unwrap_or_default()
    }
}
