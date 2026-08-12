//! 编排器侧 service / http / appservice adapter 集合。
//!
//! 这些 adapter 实现 `ExecutorAdapter`，把 workflow Step 的 `(executor, input)`
//! 翻译成对应底层服务的真正调用。schema 定义"面向 workflow"——只挑 workflow
//! 实际会用到的子集，不与 buckyos-api 的完整协议绑定。
pub mod aicc;
