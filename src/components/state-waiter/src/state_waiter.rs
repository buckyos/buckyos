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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;
    use tokio::time::timeout;

    #[tokio::test]
    async fn test_basic_wait_and_set() {
        // 创建一个简单的整数状态
        let (state, waiter) = StateWaiter::new(0);
        
        // 创建一个等待状态大于5的Future
        let wait_future = waiter.wait(|s| *s > 5);
        
        // 在另一个任务中更新状态
        let state_clone = state.clone();
        let handle = tokio::spawn(async move {
            // 等待一小段时间，模拟异步操作
            tokio::time::sleep(Duration::from_millis(100)).await;
            // 设置状态为3，这不应该触发waiter
            state_clone.set(3);
            // 再等待一小段时间
            tokio::time::sleep(Duration::from_millis(100)).await;
            // 设置状态为10，这应该触发waiter
            state_clone.set(10);
        });
        
        // 等待状态满足条件或超时
        let result = timeout(Duration::from_millis(500), wait_future).await;
        
        // 确保任务完成
        handle.await.unwrap();
        
        // 验证结果
        assert!(result.is_ok(), "等待操作超时");
        assert_eq!(result.unwrap(), 10, "返回的状态应该是10");
    }
    
    #[tokio::test]
    async fn test_multiple_waiters() {
        // 创建一个字符串状态
        let (state, waiter) = StateWaiter::new(String::from("initial"));
        
        // 创建两个不同条件的waiter
        let wait_future1 = waiter.wait(|s| s.contains("hello"));
        let wait_future2 = waiter.wait(|s| s.len() > 20);
        
        // 在另一个任务中更新状态
        let state_clone = state.clone();
        let handle = tokio::spawn(async move {
            // 设置一个包含"hello"的状态，应该触发第一个waiter
            tokio::time::sleep(Duration::from_millis(100)).await;
            state_clone.set(String::from("hello world"));
            
            // 设置一个长度大于20的状态，应该触发第二个waiter
            tokio::time::sleep(Duration::from_millis(100)).await;
            state_clone.set(String::from("this is a very long string that should trigger the second waiter"));
        });
        
        // 等待第一个条件满足
        let result1 = timeout(Duration::from_millis(300), wait_future1).await;
        assert!(result1.is_ok(), "第一个等待操作超时");
        assert_eq!(result1.unwrap(), "hello world");
        
        // 等待第二个条件满足
        let result2 = timeout(Duration::from_millis(300), wait_future2).await;
        assert!(result2.is_ok(), "第二个等待操作超时");
        assert!(result2.unwrap().len() > 20);
        
        // 确保任务完成
        handle.await.unwrap();
    }
    
    #[tokio::test]
    async fn test_immediate_satisfaction() {
        // 创建一个已经满足条件的状态
        let (state, waiter) = StateWaiter::new(42);
        
        // 创建一个条件已经满足的waiter
        let wait_future = waiter.wait(|s| *s > 10);
        
        // 这个future应该立即完成
        let result = timeout(Duration::from_millis(50), wait_future).await;
        
        assert!(result.is_ok(), "等待操作超时");
        assert_eq!(result.unwrap(), 42);
    }
    
    #[tokio::test]
    async fn test_waiter_cleanup() {
        // 测试当waiter被唤醒后是否从列表中移除
        let (state, waiter) = StateWaiter::new(0);
        
        // 创建一个标志来跟踪waiter是否被调用
        let was_called = Arc::new(AtomicBool::new(false));
        let was_called_clone = was_called.clone();
        
        // 创建一个waiter，当状态为1时返回true
        let wait_future = waiter.wait(move |s| {
            if *s == 1 {
                was_called_clone.store(true, Ordering::SeqCst);
                true
            } else {
                false
            }
        });
        
        // 在另一个任务中设置状态
        let state_clone = state.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            state_clone.set(1);
        });
        
        // 等待waiter完成
        let _ = wait_future.await;
        assert!(was_called.load(Ordering::SeqCst), "waiter应该被调用");
        
        // 再次设置状态为2，如果waiter已经从列表中移除，则不应该再次调用测试函数
        was_called.store(false, Ordering::SeqCst);
        state.set(2);
        
        // 给一点时间让任何潜在的回调发生
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // 验证测试函数没有被再次调用
        assert!(!was_called.load(Ordering::SeqCst), "waiter应该已经从列表中移除");
    }
}
