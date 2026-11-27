//! The session's `HintLedger` (simple Work Session TODO §3.8, B13; prompt
//! mechanics TODO D1, D4): it holds what the current context shows in full —
//! the dedup keys of the results it shows (with the call that showed each)
//! and the `once` hints shown with them. Whenever a context is built (a new
//! run's history, a rewrite, a run resumed) it is rebuilt from the results
//! that context still shows in full ([`SessionHintLedger::reset`]); results
//! the history no longer shows in full are not in it, so their content and
//! their hints are shown again. The `once` set is kept in the session state
//! (`hints_once`, written with every commit).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

use agent_tool::{HintLedger, HINT_NS_DEDUP, HINT_NS_ONCE};

use crate::protocol::{ResultMeta, SessionState};

#[derive(Debug, Default)]
pub struct SessionHintLedger {
    once: Mutex<BTreeSet<String>>,
    dedup: Mutex<BTreeMap<String, String>>,
}

impl SessionHintLedger {
    /// The ledger of a session whose state remembers `state.hints_once`.
    pub fn of(state: &SessionState) -> Self {
        Self {
            once: Mutex::new(state.hints_once.clone()),
            dedup: Mutex::new(BTreeMap::new()),
        }
    }

    /// Write what was shown once into the state about to be committed.
    pub fn store(&self, state: &mut SessionState) {
        let once = self.once.lock().expect("hints");
        if state.hints_once != *once {
            state.hints_once = once.clone();
        }
    }

    /// A context was built: it shows `results` (by call id) in full.
    pub fn reset(&self, results: &BTreeMap<String, ResultMeta>) {
        let mut dedup = BTreeMap::new();
        let mut once = BTreeSet::new();
        for (call_id, meta) in results {
            if meta.dedup_of.is_some() {
                continue;
            }
            if let Some(key) = &meta.dedup_key {
                dedup.entry(key.clone()).or_insert_with(|| call_id.clone());
            }
            once.extend(meta.hint_once_keys.iter().cloned());
        }
        *self.dedup.lock().expect("hints") = dedup;
        *self.once.lock().expect("hints") = once;
    }

    fn set(&self, ns: &str) -> Option<Ns<'_>> {
        match ns {
            HINT_NS_ONCE => Some(Ns::Once(&self.once)),
            HINT_NS_DEDUP => Some(Ns::Dedup(&self.dedup)),
            _ => None,
        }
    }
}

enum Ns<'a> {
    Once(&'a Mutex<BTreeSet<String>>),
    Dedup(&'a Mutex<BTreeMap<String, String>>),
}

impl HintLedger for SessionHintLedger {
    fn origin(&self, ns: &str, key: &str) -> Option<String> {
        match self.set(ns)? {
            Ns::Once(s) => s.lock().expect("hints").contains(key).then(String::new),
            Ns::Dedup(m) => m.lock().expect("hints").get(key).cloned(),
        }
    }

    fn mark(&self, ns: &str, key: &str, origin: &str) {
        match self.set(ns) {
            Some(Ns::Once(s)) => {
                s.lock().expect("hints").insert(key.to_string());
            }
            Some(Ns::Dedup(m)) => {
                m.lock()
                    .expect("hints")
                    .insert(key.to_string(), origin.to_string());
            }
            None => {}
        }
    }

    fn clear(&self, ns: &str) {
        match self.set(ns) {
            Some(Ns::Once(s)) => s.lock().expect("hints").clear(),
            Some(Ns::Dedup(m)) => m.lock().expect("hints").clear(),
            None => {}
        }
    }

    fn keys(&self, ns: &str) -> Vec<String> {
        match self.set(ns) {
            Some(Ns::Once(s)) => s.lock().expect("hints").iter().cloned().collect(),
            Some(Ns::Dedup(m)) => m.lock().expect("hints").keys().cloned().collect(),
            None => Vec::new(),
        }
    }
}
