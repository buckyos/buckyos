# AiMessage 重构方案

把 `AiMessage` 从 `{role: String, content: String}` 换成业界共识的
"content blocks" 模型, 对齐 Anthropic content block / OpenAI Responses
items / Gemini parts 的最大公约数。

beta2.2 是 breaking change 版本, 不保留任何向前兼容: 旧
`AiMessage::new(role, content)` 直接删, 不加兼容层, 不写 snapshot 迁移,
老 snapshot bump version 后 load 时直接报错丢弃。

## 1. 目标类型

定义在 `buckyos-api` ([src/kernel/buckyos-api/src/aicc_client.rs](../src/kernel/buckyos-api/src/aicc_client.rs)
现 `AiMessage` 同位置):

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AiRole {
    System,
    User,
    Assistant,
    /// IR 内部 ToolResult 的载体角色。不假设任何 provider 都接受 `tool` role ——
    /// provider lowering 必须按各家原生形态重写 (见 §1.4)。
    Tool,
    /// OpenAI Responses 专用。其它 provider lowering 时退化为 system。
    Developer,
}

/// ToolResult 内嵌的内容子集。**不允许** Text/Image/Document 之外的块,
/// 排除掉 "tool result 里嵌 ToolUse / ToolResult / Thinking" 这类语义
/// 不成立的形态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiToolResultContent {
    Text { text: String },
    Image { source: ResourceRef },
    Document { source: ResourceRef, title: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AiContent {
    /// 普通文本块。
    Text { text: String },

    /// 图像块。复用现有 ResourceRef, 支持 URL / base64 / 命名对象。
    Image { source: ResourceRef },

    /// 文档块 (PDF / 长文本附件), 给 Claude document API 留位。
    Document { source: ResourceRef, title: Option<String> },

    /// assistant 决定调用 tool 的一次记录。
    ToolUse {
        call_id: String,
        name: String,
        args: HashMap<String, Value>,
    },

    /// tool 执行结果回写。携带 call_id 关联到对应 ToolUse。
    /// content 用收窄的 `AiToolResultContent` 而非 `Vec<AiContent>`,
    /// 防止嵌套出 ToolUse / Thinking 这种没语义的形态。
    ToolResult {
        call_id: String,
        content: Vec<AiToolResultContent>,
        #[serde(default, skip_serializing_if = "is_false")]
        is_error: bool,
    },

    /// extended thinking / reasoning 块。
    /// summary 给 OpenAI Responses reasoning summary 用; text 给 Claude
    /// thinking 的明文内容用; provider_metadata 兜底放各家 signature。
    /// 跨 provider 抽不了的 item 走下面的 ProviderState, 不要硬塞这里。
    Thinking {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        provider_metadata: Option<Value>,
    },

    /// 不可跨 provider 抽象、但需要 round-trip 保留的原生 item。
    /// 典型例子:
    /// - OpenAI Responses reasoning item 的 `id`/`status`/`encrypted_content`
    /// - Claude `server_tool_use` / `web_search_tool_result`
    /// - 其它各家私有 block 类型
    ///
    /// provider lowering 规则: 只有 `provider` 字段匹配当前 lowering 目标的
    /// ProviderState 会被还原成原生 item, 其它一律 drop。这样 IR 在多 provider
    /// 之间 round-trip 不会爆炸, 同时不强迫每家都解释别家的 state。
    ProviderState {
        provider: String,
        value: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiMessage {
    pub role: AiRole,
    pub content: Vec<AiContent>,
}

fn is_false(value: &bool) -> bool {
    !*value
}
```

### 1.1 role × content 合法组合

`validate()` 强校验, 不合法的组合在 IR 进 aicc 之前就拒掉:

| role | 允许的 AiContent |
|---|---|
| System | Text |
| Developer | Text |
| User | Text, Image, Document |
| Assistant | Text, ToolUse, Thinking, ProviderState |
| Tool | ToolResult (通常只有一个) |

其它组合 (`system + ToolUse` / `user + Thinking` / `tool + Text` ...) 一律
拒。`Thinking` 和 `ProviderState` 只在 assistant 上有意义。

### 1.2 helpers

```rust
impl AiMessage {
    /// 单段文本消息的快捷构造。90% 的简单调用点 (system / user prompt)
    /// 用这个, 不必写 vec![]。
    pub fn text(role: AiRole, text: impl Into<String>) -> Self { ... }

    /// 拼接所有 Text 块的 text 字段。ToolUse / ToolResult / Image / Document /
    /// Thinking / ProviderState 一律忽略。给"我只想把这条消息当字符串读"的
    /// 调用方用 (transcript 渲染 / 日志)。
    pub fn text_content(&self) -> String { ... }

    /// 取首个 Text 块的 text。给"原本写 &message.content 的"地方做轻量替换。
    pub fn first_text(&self) -> Option<&str> { ... }

    /// 人类可读的 debug 渲染, 把各种 block 用统一格式打印 (Text 直出, ToolUse
    /// 打成 `[tool_use name=X call_id=Y]`, 等等)。给 transcript 显示用。
    pub fn render_for_debug(&self) -> String { ... }

    /// 估算文本长度 (字节数), 给 llm_compress 算 budget 用。
    /// 非文本 block 用一个保守常量近似 (Image/Document 各算 ~256 字节)。
    pub fn estimate_text_len(&self) -> usize { ... }

    /// 校验 role × content 合法性。AiPayload 进 aicc 前统一调用。
    pub fn validate(&self) -> Result<(), AiMessageError> { ... }
}
```

`AiMessageError` 在 buckyos-api 新增, variants 至少包含
`InvalidBlockForRole { role, block_type }` / `MissingCallId` /
`EmptyToolResult` 几种。

### 1.3 设计要点

- `#[serde(tag = "type")]` 让 wire 形态跟 Anthropic content block 一一对应,
  debug transcript 直接是 `{"type":"tool_use","call_id":...}`, 可读。
- `AiRole` 收成 enum, 字符串 role 不一致问题在类型层消灭, aicc 里
  `tool→user` 的字符串映射顺便删掉。
- `Image` / `Document` 复用现有 `ResourceRef`, 不引入新概念。
- `AiPayload.resources` 顶层旁挂槽**保留**给 capability 输入是单一资源的
  场景 (`image.txt2img` / `vision.ocr`); chat 类调用的图像走 message-level
  `AiContent::Image`。

### 1.4 provider lowering 规则

`AiRole::Tool` 是 IR 内部约定, 不假设 provider 接受。每个 adapter 在
`build_messages` 阶段必须按下表重写:

| Provider | `AiRole::Tool + ToolResult` 的落地形态 |
|---|---|
| OpenAI Responses | 提升成顶层 `{type: "function_call_output", call_id, output}` item, 脱离 message |
| OpenAI Chat Completions | `{role: "tool", tool_call_id, content}` message (Chat Completions 原生接受 tool role) |
| Anthropic Claude | 包进 `{role: "user", content: [{type: "tool_result", tool_use_id, content}]}` |
| Gemini | 包进 `{role: "user"/"function", parts: [{functionResponse: {...}}]}` |
| Minimax | 退化路径, 拼成 `{role: "user", content: "<tool result text>"}` |

`AiRole::Developer` 同理: OpenAI Responses 原样发, 其它 provider lowering
时合并到最近的 `System` message 或退化为 `system` role。

`ProviderState`: lowering 时按 `provider` 字段过滤, 命中目标 provider 的
还原成原生 item (例如 OpenAI lowering 时把 `provider: "openai"` 的
ProviderState 还原成 reasoning item 完整字段), 其它 drop。

## 2. 迁移面

### 2.1 buckyos-api (类型源头)

- [src/kernel/buckyos-api/src/aicc_client.rs](../src/kernel/buckyos-api/src/aicc_client.rs):
  替换 `AiMessage`, 新增 `AiRole` / `AiContent` / `AiToolResultContent` /
  `AiMessageError`。`AiPayload` 的 Serialize/Deserialize (`protocol_input_json`
  和反向解析) 跟着改 messages 字段, 其余不动。
- 旧 `AiMessage::new(role: String, content: String)` 直接删。
- `AiPayload` 在被 aicc client 发出去之前调用 `validate_all_messages()`,
  返回 `AiMessageError` 立刻 fail-fast。

### 2.2 llm_context (waist, 协议核心)

- [src/frame/llm_context/src/context_loop.rs:557, 930, 941](../src/frame/llm_context/src/context_loop.rs):
  - 删 `assistant_tool_call_message` 的 envelope 序列化, 直接构造
    `AiMessage { role: Assistant, content: vec![Text{..}, ToolUse{..}, ...] }`。
  - 删 `tool_observation_message` 的 envelope 序列化, 直接构造
    `AiMessage { role: Tool, content: vec![ToolResult{call_id, content, is_error}] }`,
    `call_id` 从 observation 取。content 用 `AiToolResultContent::Text` 包裹
    observation payload。
- state.rs / outcome.rs / behavior_loop.rs / observation.rs: 凡是消费
  `AiMessage.content` 当字符串的地方, 改用 `text_content()` / `first_text()` /
  `render_for_debug()` 三个 helper, 按场景挑。
- tests.rs: 测试 fixture 重写。

### 2.3 aicc 适配器 (provider 出口, 工作量大头)

每个 provider 把 `Vec<AiContent>` 映射成各家原生 wire 形态。比当前简单 ——
不用再做 envelope 反向解析。具体 lowering 规则见 §1.4。

- [src/frame/aicc/src/openai.rs](../src/frame/aicc/src/openai.rs)
  `build_messages` (~line 926):
  - `Text` → `{type: "output_text"/"input_text", text}`
  - `ToolUse` → 顶层 `function_call` item
  - `Tool + ToolResult` → 顶层 `function_call_output` item
  - `Thinking` + 配套 `ProviderState { provider: "openai", ... }` →
    完整 reasoning item (id/status/summary/content/encrypted_content)
  - `Image` → `{type: "input_image", image_url}` block
- [src/frame/aicc/src/claude.rs](../src/frame/aicc/src/claude.rs) +
  [src/frame/aicc/src/claude_protocol.rs](../src/frame/aicc/src/claude_protocol.rs):
  最贴合, 几乎 1:1 映射到 Anthropic content block。`Thinking` 走
  thinking block, signature 走 provider_metadata。`Tool + ToolResult`
  按 §1.4 包进 user message。
- minimax.rs: 退化路径, 非 Text 块降级成 text 或忽略 (含
  `Tool + ToolResult` 降级)。
- 顺手清理:
  - aicc 里所有字符串 role 映射 (`tool→user` 等), 改为 match `AiRole`。
  - envelope 反向解析 / `canonical_message_texts` 里的相关 dead code。
  - `assistant_tool_call_message` 的 "keep transcript readable" 注释。
- 测试: adapter_protocol_tests.rs / protocol_expanded_tests.rs /
  claude_protocol.rs 内嵌测试的 fixture 全部重写。每个 provider 必须有
  覆盖到 §1.4 全部行的 wire 形态测试。

### 2.4 调用点 (机械迁移)

大多是 `AiMessage::new("user".to_string(), text)` 这种简单文本, 直接换
`AiMessage::text(AiRole::User, text)`:

- agent_tool: llm_compress.rs (4 处)、run_local_llm.rs (4 处)、
  llm_explore.rs (2 处)
- opendan: behavior/prompt.rs (3 处)、step_record.rs (1 处)、
  agent_environment.rs (测试 2 处)
- workflow/adapters/aicc.rs、control_panel/aicc_settings.rs (各 1 处)
- buckyos-api 自带 cli 示例 ([aicc_client.rs:813](../src/kernel/buckyos-api/src/aicc_client.rs))

llm_compress.rs 里读 `message.content` 算长度的地方改 `estimate_text_len()`。
编译器驱动改完。

### 2.5 持久化

- `state.accumulated: Vec<AiMessage>` 序列化形态变了, 老 snapshot 不兼容。
- bump snapshot schema version, load 时直接报错丢弃, 不写迁移脚本。
- step_record.rs 里持久化的 AiMessage 同步改 schema, 不保留旧形态读取。
- 开发者需要清掉 `~/.opendan` 下 snapshot, 写到 release note 里。

### 2.6 文档

- `LLM Context 设计.md §A.2`: "AiMessage 是中立 envelope" → "AiMessage
  是 provider 共通的 content-block 模型, role × content 合法性由 §1.1
  定义, provider lowering 规则见 §1.4"。
- [doc/opendan/notepad_tool_call_handoff.md](../doc/opendan/notepad_tool_call_handoff.md)
  在本次重构落地后可整篇标记为 resolved。

## 3. PR 切分

不分多阶段, 类型切换 + 全部调用点迁移在同一波改动里完成。但 review 友好
角度建议按 provider 拆 PR:

1. 类型 + waist + 调用点 + 一个 provider (openai-responses) + 持久化, 主 PR,
   控制在 ~700 行 (含 validate / helpers / lowering 规则)。
2. claude provider。
3. openai-chat-completions provider。
4. minimax provider。

每个 PR 都自带该 provider 的 wire 形态测试 (用 fixture 对比期望 JSON),
并覆盖 §1.4 表格里属于该 provider 的全部行。

## 4. 风险与缓解

| 风险 | 缓解 |
|---|---|
| 主 PR 太大 review 难 | 按 provider 拆 PR (见 §3), 主 PR ~700 行。 |
| `validate()` 没在所有入口被调到, 不合法消息漏到 provider 才崩 | `AiPayload` 在 aicc client 发出去之前**强制**调一次 `validate_all_messages()`, 这一个口子收住所有路径。lint: 调用 `AiMessage` 内部字段构造的代码必须经过 `validate()` 或一个内部 builder。 |
| `ProviderState` 被滥用成"什么都往里塞"的逃生窗 | 文档明确: 优先放 `Thinking.provider_metadata`; 只有"完整原生 item 需要 round-trip"才用 ProviderState。code review 时盯一下。 |
| 多模态调用点同时存在 `AiPayload.resources` 和 message-level `Image`, 谁优先 | 显式规则: chat 类 capability 看 message-level `Image` 块; 单图 capability (`vision.ocr` / `image.txt2img`) 看 `AiPayload.resources`。在 aicc 里加 assert 防错配。 |

## 5. 不在范围内

- streaming tool_call 重设计 —— 当前 streaming 在 outcome 层重建, 跟
  AiMessage 静态形态正交。
- prompt cache breakpoint 跨 provider 抽象 —— 先用 `Thinking.provider_metadata`
  / `ProviderState` escape hatch, 真要做再单独一轮。
- workflow/adapters/aicc.rs 的更深改造 —— 这次只跟着改类型, 不动语义。
