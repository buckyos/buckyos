use futures::Future;
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct Config {
    pub limit: usize,
}

struct TransferTaskState {
    pending_tasks: HashMap<u32, usize>,
    waiting_tasks: HashMap<u32, Vec<state_waiter::State<Option<()>>>>,
}

impl TransferTaskState {
    fn pending_count(&self, priority: u32) -> usize {
        self.pending_tasks
            .iter()
            .filter(|(p, _)| **p <= priority)
            .map(|(_, count)| *count)
            .sum()
    }

    async fn weak_up_waiting_tasks(&mut self, limit: usize) {
        let mut weak_up_events = vec![];
        self.waiting_tasks.retain(|priority, events| {
            if self
                .pending_tasks
                .iter()
                .filter(|(p, _)| **p <= *priority)
                .map(|(_, count)| *count)
                .sum::<usize>()
                < limit
            {
                weak_up_events.extend(events.iter().cloned());
                false
            } else {
                true
            }
        });

        for st in weak_up_events {
            st.set(Some(()));
        }
    }
}

struct ChunkTransferImpl {
    task_state: Arc<tokio::sync::Mutex<TransferTaskState>>,
    config: Config,
}

#[derive(Clone)]
pub struct ChunkTransfer(Arc<ChunkTransferImpl>);

impl ChunkTransfer {
    pub fn new(config: Config) -> Self {
        Self(Arc::new(ChunkTransferImpl {
            task_state: Arc::new(Mutex::new(TransferTaskState {
                pending_tasks: HashMap::new(),
                waiting_tasks: HashMap::new(),
            })),
            config,
        }))
    }

    pub async fn push<F, P, R, A>(
        &self,
        proc: F,
        param: P,
        priority: u32,
        limit_duration: Duration,
    ) -> Result<
        state_waiter::Waiter<Option<Result<R, Arc<Box<dyn std::error::Error + Send + Sync>>>>>,
        (state_waiter::Waiter<Option<()>>, P),
    >
    where
        F: FnOnce(P) -> A + Send + 'static,
        A: Future<Output = Result<R, Box<dyn std::error::Error + Send + Sync>>> + Send,
        P: Send + 'static,
        R: Clone + Send + 'static,
    {
        let mut task_state = self.0.task_state.lock().await;
        let pending_count = task_state.pending_count(priority);

        if pending_count < self.0.config.limit {
            let (state, waiter) = state_waiter::StateWaiter::new(None);
            *task_state.pending_tasks.entry(priority).or_insert(0) += 1;

            let task_state_arc = self.0.task_state.clone();
            let cfg = self.0.config.clone();

            tokio::spawn(async move {
                let result = tokio::time::timeout(limit_duration, proc(param)).await;

                let mut task_state = task_state_arc.lock().await;
                let pending_count = task_state.pending_tasks.get_mut(&priority).unwrap();
                *pending_count -= 1;
                if *pending_count == 0 {
                    task_state.pending_tasks.remove(&priority);
                }

                task_state.weak_up_waiting_tasks(cfg.limit).await;

                state.set(Some(
                    result
                        .unwrap_or_else(|e| {
                            Err(Box::new(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "timeout",
                            )))
                        })
                        .map_err(|e| Arc::new(e)),
                ));
            });

            Ok(waiter)
        } else {
            let (state, waiter) = state_waiter::StateWaiter::new(None);
            task_state
                .waiting_tasks
                .entry(priority)
                .or_insert_with(Vec::new)
                .push(state);
            Err((waiter, param))
        }
    }
}
