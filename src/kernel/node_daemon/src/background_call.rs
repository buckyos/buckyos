//! A network call of the main loop that may hang on a bad network: it runs
//! beside the loop, one at a time, and a failure backs off.

use std::future::Future;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio::time::Instant;

pub(crate) type CallResult<T> = Result<T, String>;

pub(crate) struct BackgroundCall<T> {
    /// From a success to the next call.
    interval: Duration,
    retry_base: Duration,
    retry_max: Duration,
    failures: u32,
    next_at: Option<Instant>,
    running: Option<JoinHandle<CallResult<T>>>,
    network_changed: bool,
}

impl<T: Send + 'static> BackgroundCall<T> {
    pub(crate) fn new(interval: Duration, retry_base: Duration, retry_max: Duration) -> Self {
        Self {
            interval,
            retry_base,
            retry_max,
            failures: 0,
            next_at: None,
            running: None,
            network_changed: false,
        }
    }

    pub(crate) fn is_due(&self) -> bool {
        self.running.is_none() && self.next_at.is_none_or(|at| at <= Instant::now())
    }

    pub(crate) fn start(&mut self, call: impl Future<Output = CallResult<T>> + Send + 'static) {
        self.network_changed = false;
        self.running = Some(tokio::spawn(call));
    }

    /// The result of a call that ended since the last look.
    pub(crate) async fn take_result(&mut self) -> Option<CallResult<T>> {
        if !self.running.as_ref()?.is_finished() {
            return None;
        }
        let result = self
            .running
            .take()?
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
        let wait = if result.is_ok() {
            self.failures = 0;
            self.interval
        } else {
            let wait = self
                .retry_base
                .saturating_mul(1 << self.failures.min(16))
                .min(self.retry_max);
            self.failures += 1;
            wait
        };
        self.next_at = Some(Instant::now() + wait);
        if self.network_changed {
            self.network_changed();
        }
        Some(result)
    }

    /// What failed on the old network may work on the new one, also a call
    /// still running: no backoff.
    pub(crate) fn network_changed(&mut self) {
        self.network_changed = true;
        if self.failures > 0 {
            self.failures = 0;
            self.next_at = None;
        }
    }
}

impl<T> Drop for BackgroundCall<T> {
    fn drop(&mut self) {
        if let Some(running) = &self.running {
            running.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    const LONG: Duration = Duration::from_secs(3600);

    fn elapse<T>(call: &mut BackgroundCall<T>, time: Duration) {
        call.next_at = call.next_at.and_then(|at| at.checked_sub(time));
    }

    async fn finish<T: Send + 'static>(call: &mut BackgroundCall<T>) -> CallResult<T> {
        loop {
            if let Some(result) = call.take_result().await {
                return result;
            }
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_hanging_call_runs_alone_and_never_holds_its_caller() {
        let started = Arc::new(AtomicUsize::new(0));
        let (release, released) = tokio::sync::oneshot::channel::<()>();
        let mut call = BackgroundCall::new(LONG, LONG, LONG);
        assert!(call.is_due());
        let counted = started.clone();
        call.start(async move {
            counted.fetch_add(1, Ordering::SeqCst);
            let _ = released.await;
            Ok(7)
        });
        for _ in 0..10 {
            tokio::task::yield_now().await;
            assert!(!call.is_due());
            assert!(call.take_result().await.is_none());
        }
        release.send(()).unwrap();
        assert_eq!(finish(&mut call).await, Ok(7));
        assert_eq!(started.load(Ordering::SeqCst), 1);
        assert!(!call.is_due());
        elapse(&mut call, LONG);
        assert!(call.is_due());
    }

    #[tokio::test]
    async fn failures_back_off_up_to_the_limit_and_a_new_network_starts_over() {
        let secs = Duration::from_secs;
        let mut call = BackgroundCall::<()>::new(LONG, secs(15), secs(60));
        for wait in [15, 30, 60, 60] {
            assert!(call.is_due());
            call.start(async { Err("no route".to_string()) });
            assert!(finish(&mut call).await.is_err());
            elapse(&mut call, secs(wait - 2));
            assert!(!call.is_due(), "{wait}");
            elapse(&mut call, secs(2));
        }
        call.start(async { Err("no route".to_string()) });
        assert!(finish(&mut call).await.is_err());
        assert!(!call.is_due());
        call.network_changed();
        assert!(call.is_due());

        call.start(async { Err("the old network".to_string()) });
        call.network_changed();
        assert!(finish(&mut call).await.is_err());
        assert!(call.is_due());

        call.start(async { panic!("a bug") });
        assert!(finish(&mut call).await.is_err());
        elapse(&mut call, secs(15));
        assert!(call.is_due());
        call.start(async { Ok(()) });
        assert!(finish(&mut call).await.is_ok());
        call.network_changed();
        assert!(!call.is_due(), "nothing failed");
    }

    #[tokio::test]
    async fn a_dropped_call_is_stopped() {
        let (kept, mut gone) = tokio::sync::oneshot::channel::<()>();
        let mut call = BackgroundCall::<()>::new(LONG, LONG, LONG);
        call.start(async move {
            let _kept = kept;
            std::future::pending().await
        });
        tokio::task::yield_now().await;
        assert!(matches!(
            gone.try_recv(),
            Err(tokio::sync::oneshot::error::TryRecvError::Empty)
        ));
        drop(call);
        assert!(gone.await.is_err());
    }
}
