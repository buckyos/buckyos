//! Crash-window fault injection for recovery tests and fixture generation.
//!
//! `LIBOPENDAN_FAULT=<point>` makes the process abort (no unwinding, no
//! destructors — like `kill -9`) when execution reaches `<point>`;
//! `<point>#<n>` aborts on the n-th hit. Unset in production; the check is
//! one environment lookup at a handful of commit boundaries.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

fn hits() -> &'static Mutex<HashMap<String, usize>> {
    static H: OnceLock<Mutex<HashMap<String, usize>>> = OnceLock::new();
    H.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Abort the process when `LIBOPENDAN_FAULT` names this point (and hit).
pub fn point(name: &str) {
    let Ok(v) = std::env::var("LIBOPENDAN_FAULT") else {
        return;
    };
    let (target, nth) = match v.split_once('#') {
        Some((p, n)) => (p.to_string(), n.parse::<usize>().unwrap_or(1)),
        None => (v, 1),
    };
    if target != name {
        return;
    }
    let n = {
        let mut h = hits().lock().expect("fault hits");
        let c = h.entry(target).or_insert(0);
        *c += 1;
        *c
    };
    if n == nth {
        eprintln!("libopendan: fault injected at {name} (hit {n})");
        std::process::abort();
    }
}
