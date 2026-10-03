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
//! - `LLMContextInterruptHandle` — scheduler-facing, `interrupt(reason)`
//!   (Ctrl-C semantics) and `finish(reason)` (graceful finish: no new
//!   inference or tool call is started, the current one is cancelled or
//!   waited for, every call ends paired).
//! - `InferenceAbortToken` — provider / tool-facing, `is_aborted()` /
//!   `cancelled().await` / `reason()`. Embedded in every `LlmInferenceRequest`
//!   and, through `ToolCallCtx`, handed to every tool call.
//! - `InferenceAbortTrace` — best-effort metadata carried back in
//!   `LLMContextOutcome::Interrupted`.
//!
//! Concurrency model: two atomic flags (`aborted`, `finishing`) drive the
//! boolean state; the reason string sits behind a std `Mutex` and is written
//! by the first request. Waiters parked on `cancelled().await` /
//! `finishing().await` are unparked via a tokio `Notify`.

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
    finishing: AtomicBool,
    reason: Mutex<Option<String>>,
    notify: Notify,
}

impl InferenceAbortState {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            aborted: AtomicBool::new(false),
            finishing: AtomicBool::new(false),
            reason: Mutex::new(None),
            notify: Notify::new(),
        })
    }

    /// Flip the abort bit. Returns `true` iff this is the first call.
    pub(crate) fn set(&self, reason: String) -> bool {
        if self
            .aborted
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let mut slot = self.reason.lock().expect("abort reason mutex poisoned");
        if slot.is_none() {
            *slot = Some(reason);
        }
        drop(slot);
        self.notify.notify_waiters();
        true
    }

    /// Flip the finishing bit. Returns `true` iff this is the first call.
    fn set_finishing(&self, reason: String) -> bool {
        if self
            .finishing
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let mut slot = self.reason.lock().expect("abort reason mutex poisoned");
        if slot.is_none() {
            *slot = Some(reason);
        }
        drop(slot);
        self.notify.notify_waiters();
        true
    }

    pub(crate) fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::SeqCst)
    }

    pub(crate) fn is_finishing(&self) -> bool {
        self.finishing.load(Ordering::SeqCst)
    }

    /// Resolves once the abort bit is set.
    pub(crate) async fn aborted(&self) {
        loop {
            if self.is_aborted() {
                return;
            }
            let notified = self.notify.notified();
            if self.is_aborted() {
                return;
            }
            notified.await;
        }
    }

    /// Resolves once the abort or the finishing bit is set.
    pub(crate) async fn stopping(&self) {
        loop {
            if self.is_aborted() || self.is_finishing() {
                return;
            }
            let notified = self.notify.notified();
            if self.is_aborted() || self.is_finishing() {
                return;
            }
            notified.await;
        }
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

    /// A handle not attached to any context (hosts and tests that drive a
    /// `ToolManager` directly through [`crate::ToolCallCtx`]).
    pub fn standalone() -> Self {
        Self {
            inner: InferenceAbortState::new(),
        }
    }

    /// The token sharing this handle's state.
    pub fn token(&self) -> InferenceAbortToken {
        InferenceAbortToken::from_state(self.inner.clone())
    }

    /// Request interruption of the current or next inference. Returns `true`
    /// iff this call is the one that flipped the abort bit; subsequent calls
    /// return `false` and leave the original reason untouched.
    pub fn interrupt(&self, reason: impl Into<String>) -> bool {
        self.inner.set(reason.into())
    }

    /// Request a graceful finish: the run starts no new inference or tool
    /// call. An inference in flight completes (its tool calls are paired as
    /// not executed), a running tool is cancelled when its implementation
    /// can cancel, otherwise waited for up to `ToolPolicy.finish_grace_ms`
    /// before the run falls back to an interrupt. The run ends with
    /// `LLMContextOutcome::Settled`. Returns `true` iff this call set the bit.
    pub fn finish(&self, reason: impl Into<String>) -> bool {
        self.inner.set_finishing(reason.into())
    }

    pub fn is_interrupted(&self) -> bool {
        self.inner.is_aborted()
    }

    pub fn is_finishing(&self) -> bool {
        self.inner.is_finishing()
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

    /// True once a graceful finish was requested (see
    /// [`LLMContextInterruptHandle::finish`]). An inference keeps running;
    /// a cancellable tool should stop.
    pub fn is_finishing(&self) -> bool {
        self.inner.is_finishing()
    }

    /// Future that resolves the moment the abort bit is flipped. Provider
    /// adapters that support `tokio::select!` can race this against their own
    /// I/O to abandon the request early.
    pub async fn cancelled(&self) {
        self.inner.aborted().await
    }

    /// Future that resolves when either an interrupt or a graceful finish
    /// was requested.
    pub async fn stopping(&self) {
        self.inner.stopping().await
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
