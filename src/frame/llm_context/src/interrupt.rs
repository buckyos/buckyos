//! Inference interrupt — preemptive control plane (§3.13 of the design doc).
//!
//! `LLMContext::run()` only returns once an outcome is produced; by then any
//! in-flight inference has already finished. To actually save the cost of
//! generating tokens the scheduler is no longer interested in, it must be
//! able to abort the running provider call from *outside* `run()`. That is
//! what this module exposes.
//!
//! Three types collaborate via a single shared `InferenceAbortState`:
//!
//! - `LLMContextInterruptHandle` — scheduler-facing, `interrupt(reason)`.
//! - `InferenceAbortToken` — provider-facing, `is_aborted()` /
//!   `cancelled().await` / `reason()`. Embedded in every `LlmInferenceRequest`.
//! - `InferenceAbortTrace` — best-effort metadata carried back in
//!   `LLMContextOutcome::Interrupted`.
//!
//! Concurrency model: a single atomic flag drives the boolean state; the
//! reason string sits behind a std `Mutex` and is written exactly once (the
//! first `interrupt(...)` call wins). Waiters parked on
//! `cancelled().await` are unparked via a tokio `Notify`.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

/// Inner state shared between the interrupt handle (scheduler side) and the
/// abort token (provider side). Held behind `Arc` in both directions.
#[derive(Debug)]
pub(crate) struct InferenceAbortState {
    aborted: AtomicBool,
    reason: Mutex<Option<String>>,
    notify: Notify,
}

impl InferenceAbortState {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            aborted: AtomicBool::new(false),
            reason: Mutex::new(None),
            notify: Notify::new(),
        })
    }

    /// Flip the abort bit. Returns `true` iff this is the first call.
    fn set(&self, reason: String) -> bool {
        if self
            .aborted
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        *self.reason.lock().expect("abort reason mutex poisoned") = Some(reason);
        self.notify.notify_waiters();
        true
    }

    pub(crate) fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }

    pub(crate) fn reason(&self) -> Option<String> {
        self.reason
            .lock()
            .expect("abort reason mutex poisoned")
            .clone()
    }
}

/// Scheduler-facing handle. Obtained from `LLMContext::interrupt_handle()`
/// before or during `run()`; cloning shares the same abort state.
#[derive(Clone)]
pub struct LLMContextInterruptHandle {
    inner: Arc<InferenceAbortState>,
}

impl LLMContextInterruptHandle {
    pub(crate) fn from_state(state: Arc<InferenceAbortState>) -> Self {
        Self { inner: state }
    }

    /// Request interruption of the current or next inference. Returns `true`
    /// iff this call is the one that flipped the abort bit; subsequent calls
    /// return `false` and leave the original reason untouched.
    pub fn interrupt(&self, reason: impl Into<String>) -> bool {
        self.inner.set(reason.into())
    }

    pub fn is_interrupted(&self) -> bool {
        self.inner.is_aborted()
    }

    pub fn reason(&self) -> Option<String> {
        self.inner.reason()
    }
}

/// Provider-facing token. Cloned into every `LlmInferenceRequest`. Provider
/// adapters use it to wire abort into vendor SDK / HTTP cancellation; the
/// waist also races the inference future against `cancelled().await` so even
/// adapters that ignore the token release the scheduler thread promptly.
#[derive(Clone)]
pub struct InferenceAbortToken {
    inner: Arc<InferenceAbortState>,
}

impl std::fmt::Debug for InferenceAbortToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InferenceAbortToken")
            .field("is_aborted", &self.inner.is_aborted())
            .finish()
    }
}

impl InferenceAbortToken {
    pub(crate) fn from_state(state: Arc<InferenceAbortState>) -> Self {
        Self { inner: state }
    }

    /// Convenience constructor for callers that need a never-aborted token
    /// (tests, ad-hoc provider calls that aren't going through a real
    /// LLMContext). The token still satisfies the type contract; it just
    /// never fires.
    pub fn noop() -> Self {
        Self {
            inner: InferenceAbortState::new(),
        }
    }

    pub fn is_aborted(&self) -> bool {
        self.inner.is_aborted()
    }

    /// Future that resolves the moment the abort bit is flipped. Provider
    /// adapters that support `tokio::select!` can race this against their own
    /// I/O to abandon the request early.
    pub async fn cancelled(&self) {
        if self.is_aborted() {
            return;
        }
        let notified = self.inner.notify.notified();
        if self.is_aborted() {
            return;
        }
        notified.await;
    }

    pub fn reason(&self) -> Option<String> {
        self.inner.reason()
    }
}

/// Best-effort trace metadata carried by `LLMContextOutcome::Interrupted`.
/// `provider_cancel_supported = false` should be set explicitly by an adapter
/// that cannot map abort to a real cancel signal; the waist defaults it to
/// `true` because dropping the inference future is always an effective
/// local-side cancel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferenceAbortTrace {
    pub reason: String,
    pub requested_at_ms: u64,
    pub observed_at_ms: u64,
    #[serde(default = "default_true")]
    pub provider_cancel_supported: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_task_ref: Option<String>,
}

fn default_true() -> bool {
    true
}
