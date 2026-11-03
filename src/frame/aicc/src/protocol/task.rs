use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::Notify;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
