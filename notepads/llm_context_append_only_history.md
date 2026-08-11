# llm_context append-only history 修复思路

## 背景

OpenDAN 的 behavior loop 会把 `StepRecord` 渲染成 `AiMessage` 后交给底层 LLM 推理。为了让 provider 侧 KV Cache 稳定，推理过程中已经发送过的 message prefix 必须稳定：下一轮推理应该在旧 message 后追加新 message，而不是重新改写旧 message 的内容或角色结构。

之前的问题是，`llm_context` 内部既负责推理循环，又有机会在渲染或 loop 内部压缩历史：

- renderer 可以按 recency 把旧 `StepRecord` 从 full 渲染改成 compact 渲染；
- behavior loop 内部存在 step 维度的 `HistoryCompressor` 旁路；
- 这些改写发生在推理循环内部，导致同一段历史在不同轮次的渲染结果不稳定。

这和 KV Cache 的要求冲突。KV Cache 需要的是稳定 prefix，而不是每轮根据最新上下文重新塑形旧历史。

## 核心原则

`llm_context` 内的推理过程只做一件事：append message。

`LLMContextSnapshot` 和 OpenDAN Session 保存的 Round History 不是同一个东西：

- `LLMContextSnapshot` 是恢复执行用的机器状态，目标是让一个未完成或可继续的 LLM 过程能够从同一个状态继续跑。
- Round History 是 session 层的审计、展示、调试记录，目标是让人和上层系统知道每一轮发生了什么。
- Snapshot 不应该承担长期历史展示职责；Round History 也不应该被 `llm_context` 当作恢复执行状态读取。

具体不变量：

- `LLMContextState.accumulated` 已有部分在一次推理运行中视为只读。
- behavior mode 下已经 sediment 的 `StepRecord` 在渲染时不应因为 recency、预算、轮次变化被自动降级。
- 每次 LLM/provider 返回后，只能把 assistant message、tool result message、或 behavior step 结果追加到状态尾部。
- `build_inner_request` 只能把当前状态稳定地物化为本轮输入，不能借渲染动作改写旧历史语义。
- `StepRenderer` 应是纯渲染器，不应承担压缩策略决策。

如果需要修改历史，只能在一次推理结束之后，由上层 session 层显式负责。

## 允许的历史改写点

历史压缩、裁剪、摘要化属于 session 层职责。允许发生在这些边界：

- `LLMContextOutcome::Done` 后：session 可以根据策略压缩已完成的上下文，然后持久化新 snapshot。
- `LLMContextOutcome::ContextLimitReached` 后：session 可以重写 snapshot 中的历史，再用 `ResumeFill::RewrittenHistory` 或等价机制恢复执行。
- 手动命令，如 `/compress`：只能在 session 非 running / 非 waiting tool 状态下执行，避免改写正在推理中的上下文。
- session 恢复或 behavior switch 前：上层可以把旧状态整理成新的初始状态，但整理结果必须落盘，成为之后推理的稳定输入。

这些改写必须是显式事件，应该写入 session history / worklog，方便调试和审计。

## 禁止的内部行为

`llm_context` 内部不应再做这些事：

- 在 `render_history` 里按“最近 N 个 full，其余 compact”的策略动态压缩当前 behavior 的旧 step。
- 在 behavior loop 每个 step 后自动调用 compressor。
- 根据 token budget 在推理中隐式重写 `state.steps`、`history_summaries` 或 `accumulated` 的旧 prefix。
- 让 renderer 修改或决定持久化历史形态。

如果 renderer 需要处理 already-compressed 的历史，它只能忠实渲染上层已经写入状态的 compressed / summary record。

## 本次 Review 发现的线索

### 已符合 append-only 的部分

traditional loop 的主线比较简单，和目标一致：

- `LLMContextState::from_request` 用 `request.input` 初始化 `state.accumulated`。
- traditional `build_inference_request` 直接把 `state.accumulated.clone()` 交给 provider。
- provider 返回 tool call 时，loop 先 append assistant message，再 append tool result message。
- provider 返回 final answer 时，`finish_done` append final assistant message。
- `TurnHook` 只拿 `&LLMContextSnapshot`，按接口约束不能直接修改 waist 状态。

这些路径可以作为目标模型：`llm_context` 不理解 session 历史，只维护一次推理过程的输入、输出和恢复点。

### 让 LLMContext 变复杂的部分

behavior mode 当前把较多 session/agent 语义带进了 `LLMContextSnapshot`：

- `LLMContextState` 里有 `steps`、`history_summaries`、`history_inputs`、`last_step`、`last_report`、`next_step_index`、`next_action_id`。
- `build_inner_request` 每轮会调用 `renderer.render_history(state.steps, ...)`，再渲染 `last_step`，说明 behavior history 仍在 `llm_context` 内物化成 prompt。
- `snapshot_overrides` 位于 `llm_context` crate 内，但它会替换 system/user message、清空 step/history state、移动 hot tail。这些操作更像 session/behavior 切换策略，不是推理 loop 本身。
- `ResumeFill::RewrittenHistory` 只替换 `state.accumulated`。在 behavior mode 下，真实 prompt 还会额外来自 `steps + last_step`，因此 message-level rewrite 和 behavior-step history 之间容易产生双轨语义。

这些不是马上必须删除的 bug，但它们是复杂度来源。按“LLMContext 越简单越可靠”的目标，后续应逐步把这些上移到 session 层。

### Round History 的边界

OpenDAN 的 `round_history` 已经是独立记录系统：

- Round History 有自己的 `round_logs.jsonl`、round summary、entry seq、`EntryPayload::Message/Step/Event`。
- `SessionHistoryRecorder::record_run_diff` 从 final snapshot 中抽取本轮新增 message 或 step，写入 Round History。
- `HistoryEvent::Compaction` 只记录压缩事件，历史主体不应该因为 snapshot 压缩而被当场改写。

这说明 Round History 应继续作为审计/展示层存在。`LLMContextSnapshot` 只保存恢复下一次执行所需的最小状态；如果需要从完整历史重建上下文，应由 session 层读取 Round History 后生成新的 snapshot，而不是让 `llm_context` 直接理解 Round History。

### Session 层已有的正确边界

OpenDAN session 层已经有一些符合目标的设计：

- `persist_snapshot` 用 tmp + rename 保存 `state.snap`，这是恢复执行状态的持久化点。
- `try_load_snapshot_for_prompt` 明确标注只读，只给 prompt-rendering 消费，不用于 resumption。
- context-limit 分支在 session 层压缩 `accumulated`，写 Compaction 事件，持久化 rewritten snapshot，再 `LLMContext::resume`。
- manual `/compress` 要求 session 不能处于 Running / WaitingTool，避免改写正在推理中的状态。

这些都支持“上层在推理边界显式改写，`llm_context` 内部保持简单”的方向。

## StepRecord 渲染要求

短期实现可以继续保留 `StepRenderer::render_history`，但它必须满足稳定性：

- 当前 behavior 的普通 step 始终渲染为完整 `(assistant, user)` pair。
- 新 step 只会让输出尾部增加新的 pair，不会改变旧 pair。
- inherited behavior 和 `HistorySummaryRecord` 可以集中渲染进 `<<step_history>>`，但这些内容必须来自上层已经确定的持久状态。
- `last_step` 仍作为 hot tail 单独渲染；当下一步完成后，旧 `last_step` sediment 到 `steps`，渲染结果应保持一致。

中期更清晰的方向是把 behavior history 的物化也上移到 session 层：session 层决定 `StepRecord`、summary、用户输入和压缩摘要如何组成最终 `AiMessage` 序列，`llm_context` 只消费已经准备好的 `request.input / accumulated`。

## 修复方向

1. 移除 `XmlStepRenderer` 的 recency-based compact 逻辑。
2. 移除 `LLMContextDeps.history_compressor` 及 behavior loop 内部自动 compressor 入口。
3. 保持 `llm_context` 推理期间 append-only，不在内部修改旧 message / old step。
4. OpenDAN session 层继续负责 `llm_message_compress`、context-limit recovery、manual compress，并在压缩后持久化 snapshot。
5. 后续如果要压缩 `StepRecord` 维度历史，应在 session 层实现为显式状态改写，而不是在 renderer 或 loop 内自动发生。

## 后续简化路线

目标是让 `LLMContext` 回到“非常简单”的 waist：

1. `LLMContextRequest.input` 是本次推理的完整输入前缀。
2. `LLMContextState.accumulated` 是 append-only 的运行中 transcript。
3. `LLMContextSnapshot` 只保存恢复执行必要的 request、accumulated、pending tool、预算计数、usage、abort/task id 等机器状态。
4. behavior 的 `StepRecord`、report、behavior switch、process stack、Round History 差量记录都由 OpenDAN session 层管理。
5. session 层在启动或恢复一次 LLMContext 前，把自己维护的 history/step/summary 明确物化成 `AiMessage`，作为新的稳定 input。

按这个路线，`llm_context` 最终不需要知道 Round History，也不需要知道 `StepRecord` 的长期保存结构；它只负责把 `AiMessage` 送进 provider、执行 tool、append observation、返回 outcome/snapshot。

## 当前仍需注意的风险

- behavior mode 现在仍由 `llm_context::build_inner_request` 调 `render_history`，所以 StepRecord prompt 物化还没有完全上移到 session 层。
- session 层 `llm_message_compress` 目前主要压缩 `state.accumulated`；如果 behavior prompt 的主要 token 来自 `steps/last_step`，还需要 session 层实现 StepRecord 维度的显式压缩或物化策略。
- `snapshot_overrides` 仍在 `llm_context` crate 内，承担了 behavior switch / fork / inheritance 的部分策略。长期看应评估是否迁到 OpenDAN session 层，减少 waist 对 agent 语义的认知。

## 判断标准

完成后应满足：

- 连续两轮推理中，上一轮已发送的 message prefix 字节级稳定。
- 新 action result / assistant response 只追加在尾部。
- 未触发 session 层压缩时，旧 `StepRecord` 不会从 full 变 compact。
- 触发压缩时，有明确的 session 事件记录，并且压缩后的 snapshot 成为新的稳定基线。
- 任意时刻都能回答：某个数据是“恢复执行状态”还是“Round History 审计记录”；如果回答不清，说明职责边界又混了。
