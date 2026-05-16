

**任务意图：将 `AiResponseSummary` 重构为以 `AiMessage` 为主 IR 的 `AiResponse`**

背景：`AiMessage` 已经为现代 LLM API 做过重构，能表达有序多 content blocks，包括 `Text`、`Image`、`Document`、`ToolUse`、`ToolResult`、`Thinking`、`ProviderState`。但当前 AICC provider 的直接返回仍是 `AiResponseSummary { text, tool_calls, artifacts, ... }`。这个结构是摘要视图，不是 `AiMessage` 的对偶，会丢失现代 LLM 返回里的 block 顺序、多模态混排、provider state/thinking 在消息流中的位置。这个版本允许 breaking change，应趁机把协议改干净。

目标：废弃 `AiResponseSummary` 作为主返回协议，改为：

```rust
pub struct AiResponse {
    pub message: AiMessage, // must be AiRole::Assistant

    pub usage: Option<AiUsage>,
    pub cost: Option<AiCost>,
    pub finish_reason: Option<String>,
    pub provider_task_ref: Option<String>,
    pub extra: Option<Value>,
}
```

核心语义：
- `message` 是唯一保真的 assistant response IR。
- `message.role` 必须是 `AiRole::Assistant`。
- `message.content` 必须按 provider 原始返回顺序保留 block 顺序。
- `text/tool_calls/artifacts` 不再是主协议字段。
- 如仍需要文本摘要、tool call 列表、artifact 列表，应通过 helper 从 `message.content` 派生，不能成为第二真相源。
- `usage/cost/finish_reason/provider_task_ref/extra` 是 telemetry / provider metadata，不属于消息正文。

需要修改的主要范围：
- `src/kernel/buckyos-api/src/aicc_client.rs`
  - 定义 `AiResponse`。
  - 替换 `AiResponseSummary` 的公共使用。
  - `AiMethodResponse.result: Option<AiResponse>`。
  - 调整所有 request/response validate。
  - 增加 helper：
    - `AiResponse::text_content()`
    - `AiResponse::tool_calls()`
    - `AiResponse::artifacts()` 可选，仅作为派生视图。
- `src/frame/aicc/src/*`
  - 所有 provider adapter 返回 `AiResponse`。
  - OpenAI / Claude / Gemini / MiniMax / Fal 等 provider 需要逐个改。
  - 文本输出构造成 `AiContent::Text`。
  - tool call 构造成 `AiContent::ToolUse`，保留顺序。
  - 图片输出构造成 `AiContent::Image { source: ResourceRef }`。
  - PDF/文件/长文档构造成 `AiContent::Document { source, title }`。
  - reasoning/thinking 构造成 `AiContent::Thinking`。
  - provider-native 必须 roundtrip 的状态构造成 `AiContent::ProviderState`。
- `src/frame/llm_context/src/*`
  - `LlmClient::infer()` 返回类型从 `AiResponseSummary` 改为 `AiResponse`。
  - `LLMContextOutcome::Done.response` 改为 `AiResponse`。
  - `LLMContextState.accumulated` 追加 `response.message.clone()`，不要再从 `text/tool_calls` 手拼 assistant message。
  - `ContextOutput` 继续保留为调度层最终输出视图，但从 `response.message.text_content()` 派生；不要用它承载附件。
  - Tool loop 判断 tool calls 改为从 `response.message.content` 里找 `AiContent::ToolUse`。
- `src/frame/opendan/src/*`
  - outbound 不能再只读 `ContextOutput::Text`。
  - 对 chat/messagehub 出站，优先消费 `Done.response.message`。
  - 使用 `llm_context::msg_parser::ai_message_to_msg_object*()` 转成 `MsgObject`，这样 image/document 才能进入 MessageHub 附件路径。
- `src/kernel/workflow/src/adapters/aicc.rs`
  - 更新 workflow adapter 返回结构。
  - 如果 workflow 只需要文本，使用 `response.message.text_content()`。
- 测试：
  - 更新所有 `AiResponseSummary` 构造。
  - 增加多模态顺序测试：Text -> Image -> Text -> ToolUse 顺序必须保留。
  - 增加 provider artifact 回归测试：图片生成结果必须出现在 `response.message.content` 的 `AiContent::Image` 中。
  - 增加 LLMContext history 测试：Done 后 accumulated 里应包含完整 assistant `AiMessage`，包括非文本 blocks。
  - 增加 OpenDAN outbound 转换测试：assistant message with image/document 能生成 `MsgObject.content.refs` 或明确的附件表达。

迁移规则：
- 旧：
```rust
AiResponseSummary {
    text: Some("hello".into()),
    tool_calls: vec![call],
    artifacts: vec![artifact],
    usage,
    cost,
    finish_reason,
    provider_task_ref,
    extra,
}
```

- 新：
```rust
AiResponse {
    message: AiMessage::new(
        AiRole::Assistant,
        vec![
            AiContent::Text { text: "hello".into() },
            AiContent::ToolUse {
                call_id: call.call_id,
                name: call.name,
                args: call.args,
            },
            AiContent::Image { source: artifact.resource },
        ],
    ),
    usage,
    cost,
    finish_reason,
    provider_task_ref,
    extra,
}
```

注意：如果 provider 原始返回顺序是 Text -> Image -> Text，不允许先合并所有 text 再放 image。必须按原始顺序 lower 到 `Vec<AiContent>`。

验收标准：
- 仓库中不再有 provider 主路径构造 `AiResponseSummary { text, tool_calls, artifacts }`。
- AICC 返回的主协议是 `AiResponse { message, ... }`。
- `AiMessage` 是 AICC response、LLMContext accumulated、OpenDAN outbound 的共同主 IR。
- 图片/文档类 provider 输出不会只存在于 telemetry/summary 字段里。
- `cargo test` 通过；至少 `cargo test -p buckyos-api -p aicc -p llm_context -p opendan` 通过，若全量太慢，记录未跑原因。