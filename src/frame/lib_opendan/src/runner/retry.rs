//! The host's retries of a run that stopped on a retryable error (DV
//! follow-up TODO H1): the run is kept and infers again after a backoff
//! (`state.retry`, projected as `retry_not_before_ms`); retries in a row are
//! bounded by count and duration, after which the run fails as
//! `provider_unavailable`. Any other outcome, and a retryable error after
//! the segment made progress, starts over. Not llm_context's
//! `error_policy.max_consecutive_errors` (errors inside one run).

use std::time::Duration;

use serde_json::{json, Value};

use crate::protocol::*;

use super::RunnerOptions;

/// `last_error.kind` of a run whose retries are used up.
pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";

/// What a retryable error leads to.
pub(super) enum Retry {
    /// The run is kept; it infers again from `not_before_ms` on.
    Scheduled(RetryState),
    /// The run fails with this `last_error`.
    Exhausted(Value),
}

/// Delay before the `attempts`-th retry (`attempts ≥ 1`).
fn backoff(opts: &RunnerOptions, attempts: u32) -> Duration {
    let factor = 2u32.saturating_pow(attempts.saturating_sub(1));
    opts.retry_backoff_base
        .saturating_mul(factor)
        .min(opts.retry_backoff_cap)
}

/// A retryable error at `now_ms` of a run that took in `applied_input_seq`
/// batches; `prev`: the retries in a row so far (`None` when the segment
/// made progress or none came before).
pub(super) fn on_retryable(
    prev: Option<&RetryState>,
    opts: &RunnerOptions,
    now_ms: u64,
    applied_input_seq: u64,
    message: &str,
) -> Retry {
    let attempts = prev.map(|r| r.attempts).unwrap_or(0);
    let since_ms = prev.map(|r| r.since_ms).unwrap_or(now_ms);
    let elapsed = Duration::from_millis(now_ms.saturating_sub(since_ms));
    if attempts >= opts.max_retry_attempts || elapsed >= opts.max_retry_duration {
        return Retry::Exhausted(json!({
            "kind": PROVIDER_UNAVAILABLE,
            "retries_exhausted": true,
            "attempts": attempts,
            "since_ms": since_ms,
            "at_ms": now_ms,
            "message": message,
            "recoverable": false,
        }));
    }
    let attempts = attempts + 1;
    let delay = backoff(opts, attempts).as_millis() as u64;
    Retry::Scheduled(RetryState {
        attempts,
        since_ms,
        not_before_ms: now_ms.saturating_add(delay),
        applied_input_seq,
    })
}

/// The live run waits for its next retry now.
pub(super) fn backing_off(state: &SessionState, now_ms: u64) -> bool {
    let applied = state
        .live_run
        .as_ref()
        .map(|l| l.applied_input_seq)
        .unwrap_or(0);
    state
        .retry
        .as_ref()
        .is_some_and(|r| r.backing_off(now_ms, applied))
}

/// The inputs bring a user message: retried at once (TODO Q3).
pub(super) fn has_user_msg(inputs: &[FetchedInput]) -> bool {
    inputs
        .iter()
        .any(|i| matches!(i.input, Ok(SessionInput::Msg(_))))
}

/// What a session in backoff returns from `drive`: its retryable error.
pub(super) fn backoff_error(state: &SessionState) -> Value {
    let not_before_ms = state.retry.as_ref().map(|r| r.not_before_ms);
    let mut error = state.last_error.clone().unwrap_or_else(
        || json!({ "kind": "retry_backoff", "message": "waiting to retry", "recoverable": true }),
    );
    if let Some(o) = error.as_object_mut() {
        o.insert("retry_not_before_ms".into(), json!(not_before_ms));
    }
    error
}

/// One line of a Todo observation for an exhausted `last_error`.
pub fn exhausted_line(error: &Value) -> Option<String> {
    if error.get("kind").and_then(Value::as_str) != Some(PROVIDER_UNAVAILABLE)
        || error.get("retries_exhausted") != Some(&Value::Bool(true))
    {
        return None;
    }
    let attempts = error.get("attempts").and_then(Value::as_u64).unwrap_or(0);
    let since_ms = error.get("since_ms").and_then(Value::as_u64).unwrap_or(0);
    let at_ms = error
        .get("at_ms")
        .and_then(Value::as_u64)
        .unwrap_or(since_ms);
    let minutes = at_ms.saturating_sub(since_ms).div_ceil(60_000);
    let last = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some(format!(
        "{PROVIDER_UNAVAILABLE}: the AI provider stayed unavailable; gave up after {attempts} retries over {minutes} min (last error: {last})"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts() -> RunnerOptions {
        RunnerOptions::default()
    }

    #[test]
    fn backoff_doubles_from_the_base_up_to_the_cap() {
        let o = opts();
        let secs: Vec<u64> = (1..=7).map(|n| backoff(&o, n).as_secs()).collect();
        assert_eq!(secs, vec![30, 60, 120, 240, 480, 600, 600]);
        assert_eq!(backoff(&o, 64).as_secs(), 600, "no overflow");
    }

    #[test]
    fn six_retries_then_the_run_fails() {
        let o = opts();
        let mut now = 1_000_000;
        let mut prev: Option<RetryState> = None;
        let mut waited = 0;
        for n in 1..=6 {
            match on_retryable(prev.as_ref(), &o, now, 1, "busy") {
                Retry::Scheduled(r) => {
                    assert_eq!((r.attempts, r.since_ms), (n, 1_000_000));
                    waited += r.not_before_ms - now;
                    now = r.not_before_ms;
                    prev = Some(r);
                }
                Retry::Exhausted(e) => panic!("{e}"),
            }
        }
        assert_eq!(waited, 1_530_000, "25.5 minutes of backoff");
        match on_retryable(prev.as_ref(), &o, now, 1, "still busy") {
            Retry::Exhausted(e) => {
                assert_eq!(e["kind"], PROVIDER_UNAVAILABLE);
                assert_eq!(e["retries_exhausted"], true);
                assert_eq!(e["attempts"], 6);
                assert_eq!(e["since_ms"], 1_000_000);
                assert_eq!(e["message"], "still busy");
                assert_eq!(e["recoverable"], false);
            }
            Retry::Scheduled(r) => panic!("{r:?}"),
        }
    }

    #[test]
    fn the_duration_limit_comes_first_when_it_is_shorter() {
        let mut o = opts();
        o.max_retry_duration = Duration::from_secs(60);
        let prev = RetryState {
            attempts: 1,
            since_ms: 0,
            not_before_ms: 30_000,
            applied_input_seq: 1,
        };
        assert!(matches!(
            on_retryable(Some(&prev), &o, 59_000, 1, "x"),
            Retry::Scheduled(_)
        ));
        assert!(matches!(
            on_retryable(Some(&prev), &o, 60_000, 1, "x"),
            Retry::Exhausted(_)
        ));
    }
}
