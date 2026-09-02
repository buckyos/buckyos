use super::{NativeTaskState, ProtocolError, ProtocolErrorKind, ProtocolResultValue};
use async_trait::async_trait;
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::time::Instant;

#[derive(Debug, Clone)]
pub(crate) struct PollPolicy {
    pub initial_delay: Duration,
    pub maximum_delay: Duration,
    pub multiplier: u32,
    pub maximum_attempts: Option<u32>,
}

impl Default for PollPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_millis(500),
            maximum_delay: Duration::from_secs(10),
            multiplier: 2,
            maximum_attempts: None,
        }
    }
}

impl PollPolicy {
    pub(crate) fn validate(&self) -> ProtocolResultValue<()> {
        if self.initial_delay.is_zero()
            || self.maximum_delay.is_zero()
            || self.initial_delay > self.maximum_delay
            || self.multiplier < 1
            || self.maximum_attempts == Some(0)
        {
            return Err(ProtocolError::invalid_configuration(
                "polling backoff configuration is invalid",
            ));
        }
        Ok(())
    }

    pub(crate) fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(1).min(31);
        let factor = (self.multiplier as u128).saturating_pow(exponent);
        let millis = self
            .initial_delay
            .as_millis()
            .saturating_mul(factor)
            .min(self.maximum_delay.as_millis());
        Duration::from_millis(millis.min(u64::MAX as u128) as u64)
    }
}

#[derive(Debug)]
struct CancellationInner {
    cancelled: AtomicBool,
    notify: Notify,
}

#[derive(Debug, Clone)]
pub(crate) struct Cancellation {
    inner: Arc<CancellationInner>,
}

#[derive(Debug, Clone)]
pub(crate) struct CancelHandle {
    inner: Arc<CancellationInner>,
}

pub(crate) fn cancellation_pair() -> (CancelHandle, Cancellation) {
    let inner = Arc::new(CancellationInner {
        cancelled: AtomicBool::new(false),
        notify: Notify::new(),
    });
    (
        CancelHandle {
            inner: Arc::clone(&inner),
        },
        Cancellation { inner },
    )
}

impl CancelHandle {
    pub(crate) fn cancel(&self) -> bool {
        if self.inner.cancelled.swap(true, Ordering::AcqRel) {
            false
        } else {
            self.inner.notify.notify_waiters();
            true
        }
    }
}

impl Cancellation {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    pub(crate) async fn cancelled(&self) {
        loop {
            let notified = self.inner.notify.notified();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[derive(Debug)]
pub(crate) enum PollOutcome<T> {
    Pending {
        state: NativeTaskState,
        retry_after: Option<Duration>,
    },
    Complete(T),
}

pub(crate) async fn poll_until_terminal<F, Fut, T>(
    policy: &PollPolicy,
    deadline: Instant,
    cancellation: &Cancellation,
    mut poll: F,
) -> ProtocolResultValue<T>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = ProtocolResultValue<PollOutcome<T>>>,
{
    policy.validate()?;
    let mut attempt = 1_u32;
    loop {
        if cancellation.is_cancelled() {
            return Err(ProtocolError::new(
                ProtocolErrorKind::Cancelled,
                "native task polling was cancelled",
            ));
        }
        if Instant::now() >= deadline {
            return Err(ProtocolError::new(
                ProtocolErrorKind::DeadlineExceeded,
                "native task polling deadline was exceeded",
            ));
        }
        if policy
            .maximum_attempts
            .is_some_and(|maximum| attempt > maximum)
        {
            return Err(ProtocolError::new(
                ProtocolErrorKind::DeadlineExceeded,
                "native task polling attempt limit was exceeded",
            ));
        }
        let outcome = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::Cancelled,
                    "native task polling was cancelled",
                ));
            }
            _ = tokio::time::sleep_until(deadline) => {
                return Err(ProtocolError::new(
                    ProtocolErrorKind::DeadlineExceeded,
                    "native task polling deadline was exceeded",
                ));
            }
            outcome = poll(attempt) => outcome?,
        };
        match outcome {
            PollOutcome::Complete(value) => return Ok(value),
            PollOutcome::Pending { state, retry_after } => {
                if state.is_terminal() {
                    return Err(ProtocolError::invalid_response(
                        "terminal provider task state did not include a result",
                    ));
                }
                if policy.maximum_attempts == Some(attempt) {
                    return Err(ProtocolError::new(
                        ProtocolErrorKind::DeadlineExceeded,
                        "native task polling attempt limit was exceeded",
                    ));
                }
                let delay = retry_after
                    .unwrap_or_else(|| policy.delay_for_attempt(attempt))
                    .min(policy.maximum_delay);
                let wake_at = Instant::now()
                    .checked_add(delay)
                    .unwrap_or(deadline)
                    .min(deadline);
                tokio::select! {
                    _ = cancellation.cancelled() => {
                        return Err(ProtocolError::new(
                            ProtocolErrorKind::Cancelled,
                            "native task polling was cancelled",
                        ));
                    }
                    _ = tokio::time::sleep_until(wake_at) => {}
                }
                attempt = attempt.saturating_add(1);
            }
        }
    }
}

const GATE_RUNNING: u8 = 0;
const GATE_CANCELLED: u8 = 1;
const GATE_COMPLETED: u8 = 2;

#[derive(Debug, Default)]
pub(crate) struct CompletionGate {
    state: AtomicU8,
}

impl CompletionGate {
    pub(crate) fn cancel(&self) -> bool {
        self.state
            .compare_exchange(
                GATE_RUNNING,
                GATE_CANCELLED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub(crate) fn try_complete(&self) -> bool {
        self.state
            .compare_exchange(
                GATE_RUNNING,
                GATE_COMPLETED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub(crate) fn state(&self) -> CompletionState {
        match self.state.load(Ordering::Acquire) {
            GATE_CANCELLED => CompletionState::Cancelled,
            GATE_COMPLETED => CompletionState::Completed,
            _ => CompletionState::Running,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompletionState {
    Running,
    Cancelled,
    Completed,
}

#[async_trait]
pub(crate) trait NativeTaskCanceller: Send + Sync {
    async fn cancel(&self, remote_task_id: &str) -> ProtocolResultValue<bool>;
}

pub(crate) async fn cancel_native_task(
    remote_task_id: &str,
    provider: Option<&dyn NativeTaskCanceller>,
    local: &CancelHandle,
    completion: &CompletionGate,
) -> ProtocolResultValue<bool> {
    if remote_task_id.trim().is_empty() {
        return Err(ProtocolError::invalid_request(
            "native task ID must not be empty",
        ));
    }
    if completion.state() != CompletionState::Running {
        return Ok(false);
    }
    let local_cancelled = local.cancel();
    let late_final_blocked = completion.cancel();
    let provider_cancelled = match provider {
        Some(provider) => provider.cancel(remote_task_id).await.unwrap_or(false),
        None => false,
    };
    Ok(provider_cancelled || (local_cancelled && late_final_blocked))
}

#[derive(Clone)]
pub(crate) struct WebhookGuard {
    token_hash: [u8; 32],
    max_body_bytes: usize,
}

impl std::fmt::Debug for WebhookGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WebhookGuard")
            .field("token", &"[REDACTED]")
            .field("max_body_bytes", &self.max_body_bytes)
            .finish()
    }
}

impl WebhookGuard {
    pub(crate) fn new(token: &str, max_body_bytes: usize) -> ProtocolResultValue<Self> {
        if token.is_empty() || max_body_bytes == 0 {
            return Err(ProtocolError::invalid_configuration(
                "webhook token and body limit must be configured",
            ));
        }
        Ok(Self {
            token_hash: Sha256::digest(token.as_bytes()).into(),
            max_body_bytes,
        })
    }

    pub(crate) fn verify(&self, token: &str, body: Bytes) -> ProtocolResultValue<WebhookDelivery> {
        let actual: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        if !constant_time_eq(&self.token_hash, &actual) {
            return Err(ProtocolError::new(
                ProtocolErrorKind::WebhookRejected,
                "webhook token is invalid",
            ));
        }
        if body.len() > self.max_body_bytes {
            return Err(ProtocolError::new(
                ProtocolErrorKind::ResponseTooLarge,
                "webhook body exceeds configured byte limit",
            ));
        }
        Ok(WebhookDelivery { body })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WebhookDelivery {
    pub body: Bytes,
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0_u8;
    for (left, right) in left.iter().zip(right) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[test]
    fn backoff_is_bounded_and_attempt_based() {
        let policy = PollPolicy {
            initial_delay: Duration::from_millis(10),
            maximum_delay: Duration::from_millis(50),
            multiplier: 2,
            maximum_attempts: None,
        };
        assert_eq!(policy.delay_for_attempt(1), Duration::from_millis(10));
        assert_eq!(policy.delay_for_attempt(3), Duration::from_millis(40));
        assert_eq!(policy.delay_for_attempt(10), Duration::from_millis(50));
    }

    #[tokio::test]
    async fn polling_honors_backoff_and_completes() {
        let policy = PollPolicy {
            initial_delay: Duration::from_millis(1),
            maximum_delay: Duration::from_millis(4),
            multiplier: 2,
            maximum_attempts: Some(3),
        };
        let (_handle, cancellation) = cancellation_pair();
        let attempts = Arc::new(AtomicU32::new(0));
        let observed = Arc::clone(&attempts);
        let result = poll_until_terminal(
            &policy,
            Instant::now() + Duration::from_secs(1),
            &cancellation,
            move |_| {
                let attempt = observed.fetch_add(1, Ordering::SeqCst) + 1;
                async move {
                    if attempt == 3 {
                        Ok(PollOutcome::Complete("done"))
                    } else {
                        Ok(PollOutcome::Pending {
                            state: NativeTaskState::Running,
                            retry_after: None,
                        })
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(result, "done");
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn polling_enforces_attempt_limit_without_extra_sleep() {
        let policy = PollPolicy {
            initial_delay: Duration::from_millis(1),
            maximum_delay: Duration::from_millis(1),
            multiplier: 1,
            maximum_attempts: Some(1),
        };
        let (_handle, cancellation) = cancellation_pair();
        let error = poll_until_terminal(
            &policy,
            Instant::now() + Duration::from_secs(1),
            &cancellation,
            |_| async {
                Ok::<_, ProtocolError>(PollOutcome::<()>::Pending {
                    state: NativeTaskState::Queued,
                    retry_after: Some(Duration::from_secs(10)),
                })
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::DeadlineExceeded);
    }

    #[tokio::test]
    async fn polling_rejects_terminal_state_without_result() {
        let policy = PollPolicy::default();
        let (_handle, cancellation) = cancellation_pair();
        let error = poll_until_terminal(
            &policy,
            Instant::now() + Duration::from_secs(1),
            &cancellation,
            |_| async {
                Ok::<_, ProtocolError>(PollOutcome::<()>::Pending {
                    state: NativeTaskState::Failed,
                    retry_after: None,
                })
            },
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::InvalidResponse);
    }

    #[tokio::test]
    async fn polling_stops_at_deadline_before_calling_provider() {
        let policy = PollPolicy::default();
        let (_handle, cancellation) = cancellation_pair();
        let called = Arc::new(AtomicBool::new(false));
        let observed = Arc::clone(&called);
        let error = poll_until_terminal(&policy, Instant::now(), &cancellation, move |_| {
            observed.store(true, Ordering::SeqCst);
            async { Ok::<_, ProtocolError>(PollOutcome::Complete(())) }
        })
        .await
        .unwrap_err();
        assert_eq!(error.kind, ProtocolErrorKind::DeadlineExceeded);
        assert!(!called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn cancellation_interrupts_wait_and_is_idempotent() {
        let (handle, cancellation) = cancellation_pair();
        let waiter = tokio::spawn(async move {
            cancellation.cancelled().await;
        });
        assert!(handle.cancel());
        assert!(!handle.cancel());
        waiter.await.unwrap();
    }

    #[test]
    fn completion_gate_suppresses_late_final_after_cancel() {
        let gate = CompletionGate::default();
        assert!(gate.cancel());
        assert!(!gate.try_complete());
        assert_eq!(gate.state(), CompletionState::Cancelled);
    }

    struct AcceptingCanceller;

    #[async_trait]
    impl NativeTaskCanceller for AcceptingCanceller {
        async fn cancel(&self, remote_task_id: &str) -> ProtocolResultValue<bool> {
            Ok(remote_task_id == "remote-1")
        }
    }

    #[tokio::test]
    async fn native_cancel_calls_provider_and_blocks_late_final() {
        let (handle, cancellation) = cancellation_pair();
        let gate = CompletionGate::default();
        assert!(
            cancel_native_task("remote-1", Some(&AcceptingCanceller), &handle, &gate,)
                .await
                .unwrap()
        );
        assert!(cancellation.is_cancelled());
        assert_eq!(gate.state(), CompletionState::Cancelled);
        assert!(!gate.try_complete());
        assert!(!cancel_native_task("remote-1", None, &handle, &gate)
            .await
            .unwrap());
    }

    #[test]
    fn webhook_guard_is_bounded_and_redacted() {
        let guard = WebhookGuard::new("hook-secret", 4).unwrap();
        assert!(!format!("{guard:?}").contains("hook-secret"));
        assert!(guard.verify("wrong", Bytes::from_static(b"ok")).is_err());
        assert!(guard
            .verify("hook-secret", Bytes::from_static(b"12345"))
            .is_err());
        assert_eq!(
            guard
                .verify("hook-secret", Bytes::from_static(b"done"))
                .unwrap()
                .body,
            Bytes::from_static(b"done")
        );
    }
}
