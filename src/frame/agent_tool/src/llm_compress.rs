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
//! ## 接口
//!
//! ```ignore
//! pub async fn compress(
//!     history: &[AiMessage],
//!     deps: &LLMContextDeps,
//!     target_token_budget: u32,
//!     model_alias: &str,
//! ) -> Result<Vec<AiMessage>, LLMComputeError>;
//! ```
//!
//! 注：模块 doc 早期版本写的签名没有 `model_alias`，但 `LlmInferenceRequest`
//! 必须指定 model；`deps` 自身不持有"该用哪个模型 summarize"的信息，所以
//! 必须由调用方显式传入。OneShot 的 caller 知道自己的 `ModelPolicy.preferred`，
//! 可以直接转发；要复用更便宜的副本模型也是 caller 的事。
//!
//! 调用方在 `LLMContext::run` 的 `ContextLimitReached` 分支里调它，然后用
//! `ResumeFill::RewrittenHistory { history }` 喂回 `LLMContext::resume`。
//! 本模块还提供 [`LlmSummarizeCompressor`]——把上面那个自由函数包成
//! `local_llm_context::Compressor` 实现，可直接喂给
//! `LocalLLMContext::drive_to_terminal`。
//!
//! ## 实现注意事项
//!
//! - **复用 `deps.llm`**：压缩本身也是一次 LLM 调用，**不要**自己再
//!   实例化 client；这样 retry / quota / provider 路由都自动复用。
//! - **不写入 worklog 的"主流程"事件**：waist 已经会 emit
//!   `WorkEvent::ContextRewritten`，本模块不冒充。`WorkEvent` 目前没有
//!   summarize-粒度的变体，要加也要走单独的 sink 命名空间——本版先不引入。
//! - **错误传递**：summarize 自己失败时返回 `LLMComputeError::Provider(...)`
//!   / `LLMComputeError::OutputParse(...)`，让上层 OneShot 决定是 "再试一次"
//!   还是 "直接终态"。返回错误时**不**把它伪装成"压缩成功"——这会破坏
//!   §3.9 显式大于隐式的纪律。

use std::path::Path;

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiRole};

use llm_context::deps::{LLMContextDeps, LlmInferenceRequest};
use llm_context::error::LLMComputeError;

use crate::local_llm_context::{Compressor, LocalLLMContextError};

/// 默认保留尾部多少条非-system 消息（≈ 4 轮 user/assistant 对话）。
pub const DEFAULT_KEEP_RECENT_MESSAGES: usize = 8;

/// summary 自己最多吃 `target_token_budget` 的多少比例。剩下的留给
/// system 前缀 + 尾部对话。
const SUMMARY_BUDGET_RATIO: f32 = 0.33;
const SUMMARY_BUDGET_MIN: u32 = 256;
const SUMMARY_BUDGET_MAX: u32 = 2048;

/// 压缩对话历史到目标 token 预算内。
///
/// 策略：保留 leading system 前缀 + 尾部 K 条消息，把中间段交给 `deps.llm`
/// 用一次小推理 summarize 成一条 `[Conversation summary]` system 消息。
///
/// 三种"什么都不做"的快返：
/// - `history` 为空；
/// - tokenizer 估算结果已经 ≤ `target_token_budget`；
/// - middle 段为空（system 前缀 + tail 已覆盖全部，再压也没东西可压）。
///
/// 失败语义：summarize 调用本身失败（provider error / 空响应）直接把错误
/// 返出去，不退回未压缩 history——caller 需要据此决定是再试一次还是终态。
pub async fn compress(
    history: &[AiMessage],
    deps: &LLMContextDeps,
    target_token_budget: u32,
    model_alias: &str,
) -> Result<Vec<AiMessage>, LLMComputeError> {
    if history.is_empty() {
        return Ok(Vec::new());
    }

    let system_prefix_end = history
        .iter()
        .position(|m| m.role != AiRole::System)
        .unwrap_or(history.len());
    let system_prefix = &history[..system_prefix_end];
    let body = &history[system_prefix_end..];

    let mut tail_start_in_body = body.len().saturating_sub(DEFAULT_KEEP_RECENT_MESSAGES);
    // 避免 tail 以孤立 tool result 消息打头 —— 它需要前置 assistant 的
    // tool_use 才合法，否则 provider 会拒掉。
    while tail_start_in_body < body.len() && body[tail_start_in_body].role == AiRole::Tool {
        tail_start_in_body += 1;
    }
    let middle = &body[..tail_start_in_body];
    let tail = &body[tail_start_in_body..];

    let total_tokens = count_history_tokens(deps, history);
    if total_tokens <= target_token_budget || middle.is_empty() {
        return Ok(history.to_vec());
    }

    let head_tokens = count_history_tokens(deps, system_prefix);
    let tail_tokens = count_history_tokens(deps, tail);
    let room = target_token_budget
        .saturating_sub(head_tokens)
        .saturating_sub(tail_tokens);
    let mut summary_budget = ((target_token_budget as f32) * SUMMARY_BUDGET_RATIO) as u32;
    summary_budget = summary_budget.min(room.max(SUMMARY_BUDGET_MIN));
    summary_budget = summary_budget.clamp(SUMMARY_BUDGET_MIN, SUMMARY_BUDGET_MAX);

    let middle_text = render_dialogue(middle);
    let summarize_messages = vec![
        AiMessage::text(
            AiRole::System,
            "CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.\n\n\
             - You already have all the context you need in the conversation above.\n\
             - Your task is to create a detailed summary of the conversation so far, \
             paying close attention to the user's explicit requests and your previous actions.\n\
             - This summary should be thorough in capturing technical details, patterns, and \
             architectural decisions that would be essential for continuing work without losing context.",
        ),
        AiMessage::text(AiRole::User, middle_text),
    ];

    let req = LlmInferenceRequest {
        messages: summarize_messages,
        model_alias: model_alias.to_string(),
        fallbacks: Vec::new(),
        temperature: Some(0.0),
        max_completion_tokens: Some(summary_budget),
        force_json: false,
        json_schema: None,
        provider_options: None,
        tool_specs: Vec::new(),
        allow_tool_calls: false,
    };

    let resp = deps.llm.infer(req).await?;
    let summary_text = resp.text.unwrap_or_default();
    let summary_text = summary_text.trim();
    if summary_text.is_empty() {
        return Err(LLMComputeError::OutputParse(
            "compress: summarizer returned empty text".to_string(),
        ));
    }

    let mut out: Vec<AiMessage> = Vec::with_capacity(system_prefix.len() + 1 + tail.len());
    out.extend_from_slice(system_prefix);
    out.push(AiMessage::text(
        AiRole::System,
        format!("[Conversation summary]\n{}", summary_text),
    ));
    out.extend_from_slice(tail);
    Ok(out)
}

fn count_history_tokens(deps: &LLMContextDeps, msgs: &[AiMessage]) -> u32 {
    let mut total: u32 = 0;
    for m in msgs {
        total = total.saturating_add(deps.tokenizer.count_tokens(m.role.as_str()));
        total = total.saturating_add(deps.tokenizer.count_tokens(&m.render_for_debug()));
    }
    total
}

fn render_dialogue(msgs: &[AiMessage]) -> String {
    let mut s = String::new();
    for m in msgs {
        s.push_str(m.role.as_str());
        s.push_str(":\n");
        s.push_str(&m.render_for_debug());
        s.push_str("\n\n");
    }
    s
}

/// `Compressor` 适配器：把上面的自由函数包成 `LocalLLMContext::drive_to_terminal`
/// 接受的 trait object。caller 只要选定 `model_alias` 和目标预算即可。
///
/// 注意 `deps` 是 cheap-Clone（内部全是 `Arc`），这里按值持有不会引入额外开销。
pub struct LlmSummarizeCompressor {
    pub deps: LLMContextDeps,
    pub model_alias: String,
    pub target_token_budget: u32,
}

impl LlmSummarizeCompressor {
    pub fn new(deps: LLMContextDeps, model_alias: impl Into<String>, target_token_budget: u32) -> Self {
        Self {
            deps,
            model_alias: model_alias.into(),
            target_token_budget,
        }
    }
}

#[async_trait]
impl Compressor for LlmSummarizeCompressor {
    async fn compress(
        &self,
        accumulated: Vec<AiMessage>,
        _dir: &Path,
    ) -> Result<Vec<AiMessage>, LocalLLMContextError> {
        compress(
            &accumulated,
            &self.deps,
            self.target_token_budget,
            &self.model_alias,
        )
        .await
        .map_err(|e| LocalLLMContextError::CompressorFailed(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use buckyos_api::{AiMessage, AiResponseSummary, AiRole};

    use super::*;
    use llm_context::deps::{LLMContextDeps, LlmClient, LlmInferenceRequest};

    struct StaticSummarizer {
        reply: String,
    }

    #[async_trait]
    impl LlmClient for StaticSummarizer {
        async fn infer(
            &self,
            _req: LlmInferenceRequest,
        ) -> Result<AiResponseSummary, LLMComputeError> {
            Ok(AiResponseSummary {
                text: Some(self.reply.clone()),
                ..Default::default()
            })
        }
    }

    struct StubTools;
    #[async_trait]
    impl llm_context::deps::ToolManager for StubTools {
        async fn call_tool(
            &self,
            call: buckyos_api::AiToolCall,
        ) -> llm_context::observation::Observation {
            llm_context::observation::Observation::Error {
                call_id: call.call_id,
                message: "stub".to_string(),
            }
        }
    }

    fn make_deps(reply: &str) -> LLMContextDeps {
        let llm: Arc<dyn LlmClient> = Arc::new(StaticSummarizer {
            reply: reply.to_string(),
        });
        let tools: Arc<dyn llm_context::deps::ToolManager> = Arc::new(StubTools);
        LLMContextDeps::new(llm, tools)
    }

    fn msg(role: &str, content: &str) -> AiMessage {
        let role = match role {
            "system" => AiRole::System,
            "user" => AiRole::User,
            "assistant" => AiRole::Assistant,
            "tool" => {
                // Tool role requires a ToolResult block, not plain text. Tests
                // that simulate `tool` messages use this helper purely for shape;
                // wrap the text as a synthetic tool_result keyed by a dummy id.
                return AiMessage::new(
                    AiRole::Tool,
                    vec![buckyos_api::AiContent::tool_result_text(
                        "dummy-call",
                        content,
                        false,
                    )],
                );
            }
            "developer" => AiRole::Developer,
            other => panic!("unknown role: {other}"),
        };
        AiMessage::text(role, content)
    }

    #[tokio::test]
    async fn empty_history_returns_empty() {
        let deps = make_deps("");
        let out = compress(&[], &deps, 1024, "test-model").await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn under_budget_returns_as_is() {
        let deps = make_deps("");
        let history = vec![
            msg("system", "you are helpful"),
            msg("user", "hi"),
            msg("assistant", "hello"),
        ];
        let out = compress(&history, &deps, 10_000, "test-model").await.unwrap();
        assert_eq!(out, history);
    }

    #[tokio::test]
    async fn over_budget_summarizes_middle() {
        let deps = make_deps("SUMMARY_OK");
        let big_blob = "x".repeat(4_000); // ~1k tokens via heuristic
        let mut history = vec![msg("system", "you are helpful")];
        for i in 0..6 {
            history.push(msg("user", &format!("q{}: {}", i, big_blob)));
            history.push(msg("assistant", &format!("a{}: {}", i, big_blob)));
        }
        let out = compress(&history, &deps, 1024, "test-model").await.unwrap();
        // system prefix + summary + tail (<= DEFAULT_KEEP_RECENT_MESSAGES)
        assert_eq!(out[0].role, AiRole::System);
        assert_eq!(out[0].text_content(), "you are helpful");
        assert_eq!(out[1].role, AiRole::System);
        assert!(out[1].text_content().contains("SUMMARY_OK"));
        assert!(out.len() < history.len());
        // tail preserved verbatim
        assert_eq!(out.last().unwrap(), history.last().unwrap());
    }

    #[tokio::test]
    async fn empty_summary_text_errors() {
        let deps = make_deps("   ");
        let big_blob = "x".repeat(4_000);
        let mut history = vec![msg("system", "sys")];
        for i in 0..6 {
            history.push(msg("user", &format!("q{}: {}", i, big_blob)));
            history.push(msg("assistant", &format!("a{}: {}", i, big_blob)));
        }
        let err = compress(&history, &deps, 1024, "test-model").await.unwrap_err();
        matches!(err, LLMComputeError::OutputParse(_));
    }

    #[tokio::test]
    async fn tail_does_not_start_with_tool_message() {
        let deps = make_deps("S");
        let big_blob = "x".repeat(2_000);
        // Lay out so the natural K=8 cut would land on a `tool` message.
        let mut history = vec![msg("system", "sys")];
        for _ in 0..10 {
            history.push(msg("assistant", &big_blob));
            history.push(msg("tool", &big_blob));
        }
        let out = compress(&history, &deps, 512, "test-model").await.unwrap();
        // After system prefix + summary, the first kept message must not be `tool`.
        let first_non_system = out.iter().find(|m| m.role != AiRole::System).unwrap();
        assert_ne!(first_non_system.role, AiRole::Tool);
    }
}
