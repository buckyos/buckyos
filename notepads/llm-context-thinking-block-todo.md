# Thinking block 回传修复 TODO

日期：2026-09-26

状态：待实施。第 3 节现在做（LLMContext，加上 AICC 的 Thinking 来源绑定）；第 4 节（AgentSession 及可选项）等 AgentSession 改动整理完再做。

## 1. 背景

新一代顶尖模型（Claude Opus 5.5 / Fable 5.1、GPT-6 Astra）的 thinking 不能关闭，只能用 effort 调节深度。原始推理不对外返回，只给签名、加密内容或摘要，多轮对话和工具循环里要**原样回传**才能保持推理连续性。回传有几条硬约束：

- **Claude**：`thinking` block 必须带着 `signature` 原样回传。Opus 5.5 / Fable 5.1 启用了 preserved thinking：thinking block 只对产生它的模型和它之前的那段前缀（`system`、`tools`、之前的所有 messages，要求字节级一致）有效。改写过前缀后，其后的 thinking block 失效，2026-08-31 之后创建的账号会直接返回 400。允许的操作只有两种：从最早的开始按顺序移除开头连续的 thinking；或者移除全部 thinking。
- **旧轮次的 thinking 依然有用**："thinking 只在一个 user → assistant 的工具循环内有效"是旧模型的语义（Opus 4.5 之前 API 会自动忽略之前轮次的 thinking）。新模型会把之前轮次的 thinking 保留在上下文里继续用，所以**不能**在每轮结束时从历史中间剥离 thinking，这本身就算改写前缀。
- **OpenAI Responses**：reasoning item 通过 `encrypted_content` 或 `previous_response_id` 回传。
- **Kimi / GLM 等 Chat Completions 方言**：通过 `reasoning_content` 回传明文。
- 跨 provider / 跨模型时，thinking 应该**丢弃**，不能原样透传到别家。

## 2. 现状复核

### 已正确的部分

- **传统 loop**：`state.accumulated.push(response.message.clone())`（`context_loop.rs` 401/779/800 行），保留完整的 `AiMessage`，其中包括 `AiContent::Thinking`。
- **behavior 模式**：`StepRecord.assistant_message` 保存 provider 的原始消息（`context_loop.rs` 869/885/961 行）；`XmlStepRenderer::render_full` 优先用它，只有没有时才回退到 `assistant_text`（`step_record.rs` 72 行）。inherited step / summary 按文本渲染，不带 thinking。
- **AICC 协议层**：Claude 的 thinking + signature、Responses 的 reasoning item、Kimi/GLM 的 `reasoning_content` 都能 decode 出来并在 encode 时写回。

结论：**同一模型、一次 run 内、不改写前缀**这条主路径上，thinking 已经在回传。

### LLMContext 内部的问题（现在做）

按设计，LLMContext 一次 run 内不切换模型，但 llm_context crate 里有两个改写前缀的入口，改写后没有处理之前的 thinking：

1. **`snapshot_overrides::apply_overrides_to_snapshot`**（switch / fork / independent 三种模式共用）
   - `system_messages` 替换开头的 System（`replace_leading_system`）→ 前缀变了；
   - `tool_policy` 变化 → 发出去的 tools 变了，而 tools 也属于前缀；
   - `model_policy` 变化 → 模型可能变了，旧 thinking 新模型读不了；
   - `user_messages` 替换非 System 历史 → 前缀整体替换。
   以上任一项生效后，`accumulated`、`request.input`、`steps[].assistant_message`、`last_step.assistant_message` 里残留的 thinking 在 Opus 5.5 / Fable 5.1 上都会触发校验失败（新账号 400）；如果换到非 Claude 模型，还会遇到第 4 节 AICC 那个问题（缺签名报错或泄漏进 `reasoning_content`）。
2. **`ResumeFill::RewrittenHistory`**（`context_loop.rs` 145 行）：上层传入改写后的 history（例如 context-limit 压缩后"保留最近几轮"），里面保留的 thinking block 是在旧的完整前缀下产生的，同样会失效。

### LLMContext 之外的问题（问题 3 现在做，其余将来做）

3. **`AiContent::Thinking` 没有来源坐标**：`ProviderState` 带 `ProviderStateCoordinate`，跨 provider 时会降级；`Thinking` 没有。AICC 在两次调用之间可能把同一个 alias 路由到不同模型，`runtime_failover` 在同一次调用内也可能换到 backup（`aicc/src/execution/mod.rs` 1097-1251 行，只保证 API 类型相同）。于是：
   - Kimi / GLM 的 Thinking（没有 `signature`）发给 Claude → `claude_messages.rs:693` 报错，**整个请求失败**；
   - 其它 provider 的 Thinking 发给 Kimi / GLM → 被写成 `reasoning_content`（`chat_completions_dialects.rs:493`）。
4. **AgentSession 跨 user message 换模型**：session 生命周期很长，传统上允许每次处理 user message 时换模型。
5. **AgentSession 的压缩入口**：`llm_message_compress`、context-limit rewrite、手动 `/compress`。
6. **`redacted_thinking`** 被 decode 成 `ProviderState { source: unbound }`（`claude_messages.rs:946`）。回放时 Claude encode 对非 native ProviderState 会报错；需要确认 execution 层会不会补上坐标绑定。5.x 模型基本不再返回它。
7. **OneShot 本地直连路径**（`agent_tool/src/local_llm_context.rs:4508`）丢弃 Thinking，影响 Qwen3 这类需要回传 `reasoning_content` 的本地推理模型。

## 3. 现在做：LLMContext（+ AICC Thinking 来源绑定）

### 3.1 新增 `strip_thinking` helper

- 在 llm_context 里加一个公开函数，移除消息列表中的全部 thinking：
  - `AiContent::Thinking`；
  - 思考类 `ProviderState`：`value.type` 为 `redacted_thinking`（Claude）或 `reasoning`（OpenAI Responses）。等第 4 节 AICC 把 `redacted_thinking` 统一成 `Thinking` 后，这一条可以简化。
- 再提供一个作用于 `LLMContextSnapshot` 的版本，一次处理 `request.input`、`state.accumulated`、`state.steps[].assistant_message`、`state.last_step.assistant_message`。
- 剥离后 assistant message 为空的情况：保留 text / tool_use，不删除整条消息，否则会破坏 tool_use / tool_result 的配对。
- 以后 AgentSession 换模型、压缩时也复用这个 helper。

### 3.2 `apply_overrides_to_snapshot` 在改写前缀时剥离 thinking

- `system_messages`、`user_messages`、`tool_policy`、`model_policy` 任一 override 存在时，在返回之前对整个 snapshot 调用 3.1 的函数。
- `trace`、`objective`、`budget`、`human_policy`、`error_policy`、`output`、`behavior_name`、`reset_*` 这些不影响发出去的前缀，不触发剥离。
  - 需确认：`objective` / `behavior_name` 是否会被渲染进 prompt。如果会，也要算进触发条件。
- 先比较新旧值，只在真正变化时剥离（例如 switch 时 system 文本没变就不剥），避免不必要地丢掉推理。

### 3.3 `ResumeFill::RewrittenHistory` 剥离 thinking

- `context_loop.rs` 145 行接收改写后的 history 时，统一对它调用 3.1 的函数。改写就意味着前缀变了，改写方不需要再各自处理 thinking。
- 在 `outcome.rs` 的 `RewrittenHistory` 文档里写明这条语义。

### 3.4 锁定不变量的测试

- 传统 loop：多轮工具循环中，第 N 轮请求里前面各轮的 Thinking（含 `provider_metadata.signature`）字节级不变。
- behavior 模式：step sediment（`last_step` → `steps`）前后，`render_full` 输出的 assistant message 相同，Thinking 都保留。
- snapshot 序列化 → 反序列化后 Thinking 和 `provider_metadata` 完整。
- 3.2：各触发字段分别变化时，snapshot 中不再有 thinking；非触发字段变化时 thinking 保留。
- 3.3：`RewrittenHistory` 传入带 thinking 的 history，resume 后的第一次请求里没有 thinking。

### 3.5 AICC：Thinking 绑定来源（问题 3，现在做）

> [!WARNING]
> 如果因欠费必须切换 provider，历史 thinking 几乎必定无法由 backup 继续使用，即使切到同名模型也不能假设兼容。本修复的目标是让任务在丢失这部分推理连续性的情况下继续执行。旧 thinking 会从发给 backup 的请求中过滤，backup 基于保留的普通对话和工具结果重新推理；原始历史中的 thinking 仍可保留。

- 给 `AiContent::Thinking` 增加来源坐标（复用 `ProviderStateCoordinate`），在和 `ProviderState` 同样的位置完成绑定。
- 坐标必须包含 provider 实例（`provider_profile_id` + `adapter_type`），不能只看模型：同一个 Opus 5.5，官方 Anthropic 和 OpenRouter 之间也视为不同来源，thinking 直接丢弃。原因：
  - 两边协议格式不同（Claude Messages 的 `thinking` + `signature`，对比 OpenRouter Responses 的 reasoning item / `reasoning_details`），做格式转换既没有文档保证，也没法测试；
  - OpenRouter 可能把请求转到 Anthropic、Bedrock 或 Vertex 等不同上游，账号也不同，preserved thinking 的校验结果无法预期。
  - 现状：OpenRouter → 官方 Claude 方向，Responses decode 出的 Thinking 只有 `id` / `encrypted_content`，没有 `signature`，会触发 `claude_messages.rs:693` 报错。
  - 代价：切换 provider 时丢失一次历史推理，影响和一次压缩相当，可以接受。
- 各 adapter 的 encode：坐标匹配 → 原样回传；不匹配或 unbound → 静默丢弃，不报错，也不转成 `reasoning_content`。
- 为什么提前到现在做：产品上换 provider 通常是被迫的（例如第一个 provider 欠费）。AICC 把 402、`insufficient balance`、`余额不足` 等错误判定为需要换候选（`aicc/src/error/mod.rs` 554-570 行 `requires_candidate_change`），`runtime_failover` 会在**同一次 LLMContext run 内**透明地换到 backup。LLMContext 看到的 model alias 没变，3.2 的 `model_policy` 触发不了，只有 AICC 这一层能处理。
- 过滤必须是确定性的：每次请求都按坐标丢弃同一批 block，这样发给 backup 的前缀在多次请求之间保持稳定，backup 自己新产生的 thinking 也不会因为前缀变化而失效。primary 充值恢复、路由切回后，primary 的旧 thinking 又变回原生可回放，backup 的被丢弃，逻辑自然成立，不需要特殊处理。
- 不依赖 AgentSession。
- 直接修改 `buckyos_api::AiContent`，同步更新 `aicc_client`、TS `src/tools/buckyos-agent/lib/aicc.ts` 等调用方。

## 4. 将来做：等 AgentSession 改动整理后

### 4.1 AgentSession：跨 user message 换模型（问题 4）

- 如果 session 通过 `rebuild_with_inherit` 带着 `model_policy` override 进入新 run，3.2 已经覆盖。
- 如果 session 自己从 Round History 物化新 input，就要在模型变化时调用 3.1 的 helper。
- 判断"模型是否变化"应该用 AICC 响应返回的实际来源坐标（provider 实例 + 模型），不能用 alias。只换 provider、模型不变（例如官方 Anthropic → OpenRouter）也算变化。

### 4.2 AgentSession：压缩入口（问题 5）

- 走 `ResumeFill::RewrittenHistory` 的已经由 3.3 覆盖。其它直接改写 snapshot 并持久化的入口，需要改走 3.1 的 helper。
- 优先用"整段替换为摘要、不回放旧 thinking"的简单压缩。如果要"保留最近几轮原文"，保留下来的几轮必须剥离 thinking。
- 和 `llm_context_append_only_history.md` 一起做：推理进行中不改写前缀。

### 4.3 AICC：redacted_thinking（问题 6，可选）

- 在 `decode_content` 里加显式分支，统一映射成 Thinking，并补一个 decode → encode 往返测试。

### 4.4 OneShot 本地路径（问题 7，可选）

- 对支持 `reasoning_content` 的本地模型回传 Thinking。

## 5. 集成验证

- behavior session 先在 Kimi 上跑两步，再切到 Claude，确认不再报 signature 错误（4.1 / 3.5）。
- mock 欠费场景：一次多步工具循环的中途，官方 Anthropic primary 返回 402，runtime_failover 切到 OpenRouter 上的同款 Opus 5.5（以及非 Claude backup），历史里带 primary 的 thinking；确认 backup 请求正常，后续轮次继续回传 backup 自己的 thinking（3.5）。
- 反向：官方 Anthropic 充值恢复、路由切回后，历史里带着 OpenRouter 产生的 thinking，确认不再出现缺 signature 的报错，且 primary 早先的 thinking 仍被正常回传（3.5）。
- 真实后端：用 Opus 5.5 跑多步工具循环 + 一次 session 压缩，确认没有 preserved thinking 的 400。

## 6. 相关文件

- `src/frame/llm_context/src/snapshot_overrides.rs`、`context_loop.rs`、`outcome.rs`、`state.rs`、`step_record.rs`、`behavior_loop.rs`
- `src/kernel/buckyos-api/src/aicc_client.rs`（`AiContent` 定义）
- `src/frame/aicc/src/protocol/claude_messages.rs`、`openai_responses.rs`、`chat_completions_dialects.rs`、`provider_state.rs`
- `src/frame/aicc/src/execution/mod.rs`（runtime_failover）
- `src/frame/opendan/src/agent_session.rs`、`llm_context_helper.rs`
- `src/frame/agent_tool/src/local_llm_context.rs`
- `notepads/llm_context_append_only_history.md`
