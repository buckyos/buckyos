//! `llm_compress` — 上下文压缩策略（属于 OneShot 这一 L4 调度器，
//! **不属于 waist**）。
//!
//! ## 为什么这是 L4 层、不是 waist 层
//!
//! 设计文档 §3.9 / §6.4 / §A.4 反复强调：waist 只产出
//! `Outcome::ContextLimitReached` 这个"事实信号"，**绝不**在内部决定如何
//! 压缩。压缩策略在不同 scheduler 那里诉求完全不同：
//!
//! - Agent loop ⇒ summarize-and-rewind（保留 memory 关键事实）
//! - Workflow engine ⇒ fail-and-escalate（让上一 node 走 retry 分支）
//! - Eval ⇒ hard-truncate（看模型在压力下的行为）
//! - **OneShot ⇒ graceful-degrade**：本模块要实现的就是这一种
//!
//! 任何想把通用压缩逻辑提到 waist 的提议都直接退回 §A.4。
//!
//! ## OneShot 的压缩目标
//!
//! 1. **保留 system / 角色描述消息**：通常在 `accumulated[0..n_system]`。
//! 2. **保留最近 K 轮对话**：默认 K=4，让 LLM 至少有连贯的当前任务上下文。
//! 3. **把中间被掏掉的部分压成一条 system summary**：用同一个 LlmClient
//!    跑一次"summarize the following conversation in <budget> tokens"
//!    的小推理。
//! 4. **数量保证**：压缩后 token 总数 ≤ 目标比例（默认 50% window）。
//!    Resume 后还要继续累计——这正是 §6.4 末段防"无限压缩 + 无限运行"
//!    的设计：累计撞红线仍走 `BudgetExhausted`。
//!
//! ## 接口（待实现）
//!
//! ```ignore
//! pub async fn compress(
//!     history: &[AiMessage],
//!     deps: &LLMContextDeps,
//!     target_token_budget: u32,
//! ) -> Result<Vec<AiMessage>, LLMComputeError>;
//! ```
//!
//! 调用方在 `LLMOneShotContext::run` 的 `ContextLimitReached` 分支里调
//! 它，然后用 `ResumeFill::RewrittenHistory { history }` 喂回
//! `LLMContext::resume`。
//!
//! ## 实现注意事项
//!
//! - **复用 `deps.llm`**：压缩本身也是一次 LLM 调用，**不要**自己再
//!   实例化 client；这样 retry / quota / provider 路由都自动复用。
//! - **不写入 worklog 的"主流程"事件**：waist 已经会 emit
//!   `WorkEvent::ContextRewritten`，本模块可以再加更细粒度的
//!   "summarize started / finished" 事件，但要走自己的 sink 命名空间，
//!   不要冒充 waist 事件。
//! - **错误传递**：summarize 自己失败时返回 `LLMComputeError::Provider(...)`，
//!   让上层 OneShot 决定是 "再试一次" 还是 "直接终态"。注意：本模块返回
//!   错误时**不**应自动把它伪装成"压缩成功"——这会破坏 §3.9 显式大于
//!   隐式的纪律。

// TODO: implement compress(history, deps, target_token_budget) per the module doc.
