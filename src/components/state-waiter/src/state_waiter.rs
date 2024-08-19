// 实现一个事件StateWaiter，其中包含一个状态(泛型)，其提供两个接口：async wait(&self, state_tester: S: Fn(state) -> bool)，当状态能够使state_tester函数返回true时返回；set_state(state)，更新状态，如果这个状态能使某个state_tester返回true就唤醒相应的wait接口
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

#[derive(Clone)]
pub struct StateWaiter<S: Clone> {
    state: Arc<Mutex<S>>,
    waiters: Arc<Mutex<Vec<WaiterFuture<S>>>>,
}

impl<S: Clone> StateWaiter<S> {
    pub fn new(initial_state: S) -> (State<S>, Waiter<S>) {
        let obj = StateWaiter {
            state: Arc::new(Mutex::new(initial_state)),
            waiters: Arc::new(Mutex::new(Vec::new())),
        };

        (State(obj.clone()), Waiter(obj))
    }
}

#[derive(Clone)]
pub struct State<S: Clone>(StateWaiter<S>);

impl<S: Clone> State<S> {
    pub fn set(&self, new_state: S) {
        *self.0.state.lock().unwrap() = new_state.clone();
        let mut waiters = self.0.waiters.lock().unwrap();
        let mut woken_waiters = Vec::new();
        let mut retain_waiters = Vec::new();

        for waiter in waiters.iter() {
            if waiter.test(&new_state) {
                waiter.wake();
                woken_waiters.push(waiter.clone());
            } else {
                retain_waiters.push(waiter.clone());
            }
        }

        *waiters = retain_waiters;
    }
}

#[derive(Clone)]
pub struct Waiter<S: Clone>(StateWaiter<S>);

impl<S: Clone> Waiter<S> {
    pub fn wait<F>(&self, state_tester: F) -> WaiterFuture<S>
    where
        F: Fn(&S) -> bool + Send + Sync + 'static,
    {
        let waiter = WaiterFuture::new(state_tester, self.0.state.clone());
        self.0.waiters.lock().unwrap().push(waiter.clone());

        waiter
    }
}

struct WaiterFutureImpl<S: Clone> {
    state: Arc<Mutex<S>>,
    state_tester: Box<dyn Fn(&S) -> bool + Send + Sync + 'static>,
    waker: Mutex<Option<std::task::Waker>>,
}

#[derive(Clone)]
pub struct WaiterFuture<S: Clone>(Arc<WaiterFutureImpl<S>>);

impl<S: Clone> WaiterFuture<S> {
    fn new<F>(state_tester: F, state: Arc<Mutex<S>>) -> Self
    where
        F: Fn(&S) -> bool + Send + Sync + 'static,
    {
        WaiterFuture(Arc::new(WaiterFutureImpl {
            state,
            state_tester: Box::new(state_tester),
            waker: Mutex::new(None),
        }))
    }

    fn test(&self, state: &S) -> bool {
        (self.0.state_tester)(state)
    }

    fn wake(&self) {
        let waker = self.0.waker.lock().unwrap().clone();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<S: Clone> Future for WaiterFuture<S> {
    type Output = S;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        {
            let state = self.0.state.lock().unwrap();
            if self.test(&*state) {
                return Poll::Ready(state.clone());
            }
        }

        let waker = cx.waker().clone();
        *self.0.waker.lock().unwrap() = Some(waker);
        Poll::Pending
    }
}
