use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub const QUEUE_STATUS_QUEUED: &str = "排队中";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueAdmission {
    Running,
    Queued { position: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteRequestQueueStats {
    pub max_in_flight: usize,
    pub in_flight: usize,
    pub queued: usize,
}

#[derive(Clone)]
pub struct CompleteRequestQueue {
    inner: Arc<QueueInner>,
}

impl CompleteRequestQueue {
    pub fn new(max_in_flight: usize) -> Self {
        Self {
            inner: Arc::new(QueueInner {
                max_in_flight: max_in_flight.max(1),
                state: Mutex::new(QueueState {
                    in_flight: 0,
                    next_ticket_id: 1,
                    waiters: VecDeque::new(),
                }),
            }),
        }
    }

    pub fn enqueue(&self) -> QueueTicket {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("queue lock should be available");
        let ticket_id = state.next_ticket_id;
        state.next_ticket_id = state.next_ticket_id.saturating_add(1);

        if state.in_flight < self.inner.max_in_flight {
            state.in_flight += 1;
            QueueTicket {
                queue: self.clone(),
                ticket_id,
                waiter: None,
                admission: QueueAdmission::Running,
                consumed: false,
            }
        } else {
            let position = state.waiters.len() + 1;
            let waiter = Arc::new(QueuedWaiter {
                ticket_id,
                ready: AtomicBool::new(false),
                notify: Notify::new(),
            });
            state.waiters.push_back(waiter.clone());
            QueueTicket {
                queue: self.clone(),
                ticket_id,
                waiter: Some(waiter),
                admission: QueueAdmission::Queued { position },
                consumed: false,
            }
        }
    }

    pub fn stats(&self) -> CompleteRequestQueueStats {
        let state = self
            .inner
            .state
            .lock()
            .expect("queue lock should be available");
        CompleteRequestQueueStats {
            max_in_flight: self.inner.max_in_flight,
            in_flight: state.in_flight,
            queued: state.waiters.len(),
        }
    }

    fn remove_waiter(&self, ticket_id: u64) {
        let mut state = self
            .inner
            .state
            .lock()
            .expect("queue lock should be available");
        if let Some(index) = state
            .waiters
            .iter()
            .position(|waiter| waiter.ticket_id == ticket_id)
        {
            state.waiters.remove(index);
        }
    }

    fn release_slot(&self) {
        let next_waiter = {
            let mut state = self
                .inner
                .state
                .lock()
                .expect("queue lock should be available");
            if let Some(waiter) = state.waiters.pop_front() {
                Some(waiter)
            } else {
                state.in_flight = state.in_flight.saturating_sub(1);
                None
            }
        };

        if let Some(waiter) = next_waiter {
            waiter.ready.store(true, Ordering::Release);
            waiter.notify.notify_waiters();
        }
    }
}

struct QueueInner {
    max_in_flight: usize,
    state: Mutex<QueueState>,
}

struct QueueState {
    in_flight: usize,
    next_ticket_id: u64,
    waiters: VecDeque<Arc<QueuedWaiter>>,
}

struct QueuedWaiter {
    ticket_id: u64,
    ready: AtomicBool,
    notify: Notify,
}

pub struct QueueTicket {
    queue: CompleteRequestQueue,
    ticket_id: u64,
    waiter: Option<Arc<QueuedWaiter>>,
    admission: QueueAdmission,
    consumed: bool,
}

impl QueueTicket {
    pub fn admission(&self) -> &QueueAdmission {
        &self.admission
    }

    pub async fn wait_for_turn(mut self) -> QueuePermit {
        if let Some(waiter) = self.waiter.as_ref() {
            loop {
                if waiter.ready.load(Ordering::Acquire) {
                    break;
                }
                waiter.notify.notified().await;
            }
        }

        self.consumed = true;
        QueuePermit {
            queue: self.queue.clone(),
            released: false,
        }
    }
}

impl Drop for QueueTicket {
    fn drop(&mut self) {
        if self.consumed {
            return;
        }

        match self.admission {
            QueueAdmission::Running => self.queue.release_slot(),
            QueueAdmission::Queued { .. } => {
                if let Some(waiter) = self.waiter.as_ref() {
                    if waiter.ready.load(Ordering::Acquire) {
                        self.queue.release_slot();
                    } else {
                        self.queue.remove_waiter(self.ticket_id);
                    }
                }
            }
        }
    }
}

pub struct QueuePermit {
    queue: CompleteRequestQueue,
    released: bool,
}

impl Drop for QueuePermit {
    fn drop(&mut self) {
        if self.released {
            return;
        }
        self.queue.release_slot();
        self.released = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn queue_holds_fifo_order_when_capacity_is_full() {
        let queue = CompleteRequestQueue::new(2);
        let t1 = queue.enqueue();
        let t2 = queue.enqueue();
        let t3 = queue.enqueue();

        assert_eq!(t1.admission(), &QueueAdmission::Running);
        assert_eq!(t2.admission(), &QueueAdmission::Running);
        assert_eq!(t3.admission(), &QueueAdmission::Queued { position: 1 });

        let p1 = t1.wait_for_turn().await;
        let _p2 = t2.wait_for_turn().await;

        let waiter = tokio::spawn(async move {
            let _p3 = t3.wait_for_turn().await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());

        drop(p1);
        tokio::time::timeout(Duration::from_millis(200), waiter)
            .await
            .expect("queued ticket should be promoted after slot release")
            .expect("waiter task should not panic");
    }

    #[tokio::test]
    async fn dropping_queued_ticket_removes_it_from_waiters() {
        let queue = CompleteRequestQueue::new(1);
        let t1 = queue.enqueue();
        let t2 = queue.enqueue();
        let t3 = queue.enqueue();

        let p1 = t1.wait_for_turn().await;
        assert_eq!(t2.admission(), &QueueAdmission::Queued { position: 1 });
        assert_eq!(t3.admission(), &QueueAdmission::Queued { position: 2 });
        assert_eq!(queue.stats().queued, 2);

        drop(t2);
        assert_eq!(queue.stats().queued, 1);

        let waiter = tokio::spawn(async move {
            let _p3 = t3.wait_for_turn().await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!waiter.is_finished());

        drop(p1);
        tokio::time::timeout(Duration::from_millis(200), waiter)
            .await
            .expect("remaining queued ticket should be promoted")
            .expect("waiter task should not panic");
    }

    #[tokio::test]
    async fn dropping_promoted_ticket_releases_slot() {
        let queue = CompleteRequestQueue::new(1);
        let t1 = queue.enqueue();
        let t2 = queue.enqueue();
        let p1 = t1.wait_for_turn().await;

        drop(p1);
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(queue.stats().in_flight, 1);

        drop(t2);
        assert_eq!(queue.stats().in_flight, 0);
        assert_eq!(queue.stats().queued, 0);
    }
}
