use async_trait::async_trait;
use std::future::Future;
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait EventListenerAsyncRoutine<P, R, E>: Send + Sync + 'static
where
    P: Send + Sync + 'static,
    R: 'static,
    E: 'static,
{
    async fn call(&self, param: &P) -> Result<R, E>;
}

#[async_trait]
impl<F, Fut, P, R, E> EventListenerAsyncRoutine<P, R, E> for F
where
    P: Send + Sync + 'static,
    R: 'static,
    F: Send + Sync + 'static + Fn(&P) -> Fut,
    E: 'static,
    Fut: Future<Output = Result<R, E>> + Send + 'static,
{
    async fn call(&self, param: &P) -> Result<R, E> {
        (self)(param).await
    }
}

#[async_trait]
pub trait EventListenerSyncRoutine<P, R, E>: Send + Sync + 'static
where
    P: Send + Sync + 'static,
    R: 'static,
    E: 'static,
{
    fn call(&self, param: &P) -> Result<R, E>;
}

#[async_trait]
impl<F, P, R, E> EventListenerSyncRoutine<P, R, E> for F
where
    P: Send + Sync + 'static,
    R: 'static,
    E: 'static,
    F: Send + Sync + 'static + Fn(&P) -> Result<R, E>,
{
    fn call(&self, param: &P) -> Result<R, E> {
        (self)(param)
    }
}

pub struct SyncEventManager<P, R, E>
where
    P: Send + Sync + 'static,
    R: 'static,
    E: 'static,
{
    next_cookie: u32,
    listeners: Vec<(u32, Box<dyn EventListenerSyncRoutine<P, R, E>>),>,
}

impl<P, R, E> SyncEventManager<P, R, E>
where
    P: Send + Sync + 'static,
    R: 'static,
    E: 'static,
{
    pub fn new() -> Self {
        Self {
            next_cookie: 1,
            listeners: Vec::new(),
        }
    }

    pub fn listener_count(&self) -> usize {
        self.listeners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.listeners.is_empty()
    }

    pub fn on(&mut self, listener: Box<dyn EventListenerSyncRoutine<P, R, E>>) -> u32 {
        let cookie = self.next_cookie;
        self.next_cookie += 1;
        if self.next_cookie == u32::MAX {
            self.next_cookie = 1;
        }

        self.listeners.push((cookie, listener));

        cookie
    }

    pub fn off(&mut self, cookie: u32) -> bool {
        let ret = self.listeners.iter().enumerate().find(|v| v.1 .0 == cookie);

        match ret {
            Some((index, _)) => {
                self.listeners.remove(index);
                true
            }
            None => false,
        }
    }

    pub fn emit(&self, param: &P) -> Result<Option<R>, E> {
        let mut ret = None;
        for item in &self.listeners {
            ret = Some(item.1.call(param)?);
        }

        Ok(ret)
    }
}

#[derive(Clone)]
pub struct SyncEventManagerSync<P, R, E>(Arc<Mutex<SyncEventManager<P, R, E>>>)
where
    P: Send + Sync + 'static,
    R: 'static,
    E: 'static;

impl<P, R, E> SyncEventManagerSync<P, R, E>
where
    P: Send + Sync + 'static,
    R: 'static,
    E: 'static,
{
    pub fn new() -> Self {
        let inner = SyncEventManager::new();
        Self(Arc::new(Mutex::new(inner)))
    }

    pub fn listener_count(&self) -> usize {
        self.0.lock().unwrap().listeners.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.lock().unwrap().is_empty()
    }

    pub fn on(&self, listener: Box<dyn EventListenerSyncRoutine<P, R, E>>) -> u32 {
        self.0.lock().unwrap().on(listener)
    }

    pub fn off(&self, cookie: u32) -> bool {
        self.0.lock().unwrap().off(cookie)
    }

    pub fn emit(&self, param: &P) -> Result<Option<R>, E> {
        self.0.lock().unwrap().emit(param)
    }
}
