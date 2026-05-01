//! 编排器侧 Apply 直执行通道。
//!
//! 一期范围内 `service::`、`http::`、`appservice::` 等 executor 都可以在编排器进程
//! 内直接调用，而不需要经过调度器 + Node Daemon Runner 的 Thunk 路径。
//! 本模块定义这些 adapter 的统一接入点：
//!
//! - [`ExecutorAdapter`]：实现某一类 executor 的真正调用逻辑（RPC、HTTP、AppService 等）。
//! - [`ExecutorRegistry`]：编排器持有的 adapter 表。`schedule_apply` 在准备投递 Thunk 之前，
//!   先在 registry 中查 adapter；命中即同步执行并把结果回填到 `node_outputs`。
//!
//! adapter 不感知 workflow 的状态机细节（缓存、重试、map shard），它只负责把
//! `(executor, input)` 映射成 `Result<Value>`。
//!
//! 对应文档：`doc/workflow/executor list.md`。
use crate::error::WorkflowResult;
use crate::types::ExecutorRef;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;

/// 编排器侧 executor 的执行适配器。一次调用 = 一个 Apply 节点 / 一个 Map shard。
#[async_trait]
pub trait ExecutorAdapter: Send + Sync {
    /// 该 adapter 能否处理给定 executor。
    fn supports(&self, executor: &ExecutorRef) -> bool;

    /// 同步（在编排器进程内）执行一次 Apply。返回值会原样回填为节点 output。
    async fn invoke(&self, executor: &ExecutorRef, input: &Value) -> WorkflowResult<Value>;
}

/// 编排器持有的 adapter 集合。注册顺序即匹配优先级（先注册先匹配）。
#[derive(Default, Clone)]
pub struct ExecutorRegistry {
    adapters: Vec<Arc<dyn ExecutorAdapter>>,
}

impl ExecutorRegistry {
    pub fn new() -> Self {
        Self {
            adapters: Vec::new(),
        }
    }

    pub fn register(&mut self, adapter: Arc<dyn ExecutorAdapter>) -> &mut Self {
        self.adapters.push(adapter);
        self
    }

    pub fn with(mut self, adapter: Arc<dyn ExecutorAdapter>) -> Self {
        self.register(adapter);
        self
    }

    pub fn find(&self, executor: &ExecutorRef) -> Option<Arc<dyn ExecutorAdapter>> {
        self.adapters
            .iter()
            .find(|adapter| adapter.supports(executor))
            .cloned()
    }

    pub fn is_empty(&self) -> bool {
        self.adapters.is_empty()
    }
}

/// 按 namespace（`service` / `http` / `appservice` / ...）匹配的简单 adapter，
/// 调用时把 `(executor, input)` 透传给一个用户提供的闭包。一期内主要用于测试与
/// 早期接入；正式场景里可以为 service / http 各写一个具名实现。
pub struct NamespaceAdapter<F> {
    namespaces: Vec<String>,
    handler: F,
}

impl<F> NamespaceAdapter<F>
where
    F: Send
        + Sync
        + 'static
        + for<'a> Fn(
            &'a ExecutorRef,
            &'a Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = WorkflowResult<Value>> + Send + 'a>,
        >,
{
    pub fn new<I, S>(namespaces: I, handler: F) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            namespaces: namespaces.into_iter().map(Into::into).collect(),
            handler,
        }
    }
}

#[async_trait]
impl<F> ExecutorAdapter for NamespaceAdapter<F>
where
    F: Send
        + Sync
        + 'static
        + for<'a> Fn(
            &'a ExecutorRef,
            &'a Value,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = WorkflowResult<Value>> + Send + 'a>,
        >,
{
    fn supports(&self, executor: &ExecutorRef) -> bool {
        match executor {
            ExecutorRef::Actual(_) => executor
                .namespace()
                .map(|ns| self.namespaces.iter().any(|allowed| allowed == ns))
                .unwrap_or(false),
            ExecutorRef::SemanticPath(_) => false,
        }
    }

    async fn invoke(&self, executor: &ExecutorRef, input: &Value) -> WorkflowResult<Value> {
        (self.handler)(executor, input).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_adapter() -> NamespaceAdapter<
        impl for<'a> Fn(
                &'a ExecutorRef,
                &'a Value,
            ) -> std::pin::Pin<
                Box<dyn std::future::Future<Output = WorkflowResult<Value>> + Send + 'a>,
            > + Send
            + Sync
            + 'static,
    > {
        NamespaceAdapter::new(["service"], |executor, input| {
            let executor = executor.clone();
            let input = input.clone();
            Box::pin(async move {
                Ok(serde_json::json!({
                    "executor": executor.as_str(),
                    "input": input,
                }))
            })
        })
    }

    #[tokio::test]
    async fn namespace_adapter_matches_actual_only() {
        let adapter = make_adapter();
        assert!(adapter.supports(&ExecutorRef::parse("service::aicc.complete").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("http::endpoint.x").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("/skill/fs-scanner").unwrap()));
    }

    #[tokio::test]
    async fn registry_returns_first_match() {
        let mut registry = ExecutorRegistry::new();
        registry.register(Arc::new(make_adapter()));
        let exec = ExecutorRef::parse("service::aicc.complete").unwrap();
        let adapter = registry.find(&exec).expect("should find adapter");
        let out = adapter.invoke(&exec, &serde_json::json!({"q": 1})).await.unwrap();
        assert_eq!(out["executor"], "service::aicc.complete");
        assert_eq!(out["input"]["q"], 1);
    }
}
