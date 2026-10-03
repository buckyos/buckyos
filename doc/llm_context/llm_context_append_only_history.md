# llm_context append-only history 修复思路

## 背景

OpenDAN 的 behavior loop 会把 `StepRecord` 渲染成 `AiMessage` 后交给底层 LLM 推理。为了让 provider 侧 KV Cache 稳定，推理过程中已经发送过的 message prefix 必须稳定：下一次推理（Round）应该在旧 message 后追加新 message，而不是重新改写旧 message 的内容或角色结构。

之前的问题是，`llm_context` 内部既负责推理循环，又有机会在渲染或 loop 内部压缩历史：

- renderer 可以按 recency 把旧 `StepRecord` 从 full 渲染改成 compact 渲染；
- behavior loop 内部存在 step 维度的 `HistoryCompressor` 旁路；
- 这些改写发生在推理循环内部，导致同一段历史在不同推理之间的渲染结果不稳定。

这和 KV Cache 的要求冲突。KV Cache 需要的是稳定 prefix，而不是每次推理都根据最新上下文重新塑形旧历史。

## 核心原则

`llm_context` 内的推理过程只做一件事：append message。

四类记录各管一件事，不能互相替代（Round / Step / Turn 的定义见 [readme](readme.md)）：

| 记录 | 归属 / 位置 | 职责 | 生命周期 |
|---|---|---|---|
| `LLMContextSnapshot` | llm_context；当前实现由 libopendan 持久化在 `runs/<run_id>/snapshots/` | 恢复执行的机器状态：request、`accumulated`、usage、工具迭代额度、挂起与续派状态（`suspended` / `tool_batch` / `action_step`）、behavior 的 `steps` / `last_step` / 编号游标、宿主元数据 `host` | 只服务于本 run 的恢复和接手；不承担长期历史展示 |
| inner transcript | 快照内：behavior 模式 `accumulated` 中 `request.input` 之后的消息 | 进行中 Step 的内层原生工具 Loop（tool_use 与已得结果），让挂起的 Step 不重放工具地续跑；`RewrittenSteps` 原样保留它 | Step 完成即清空，不进入 `StepRecord`，也不写入 worklog |
| `StepRecord` | 快照内 `steps` / `last_step` | 一个已完成的行为决策及其 action 结果，是 behavior prompt 的历史单元；身份 `(run_id, step_index)` | run 内只追加；完成后由宿主 flush 进 worklog（按 `step_index` 去重）；新 run 的历史从 worklog 重建（按目标 behavior 的 `inherit` 选择）；子 context 例外：`create_sub_context`（`inherit: steps`）与 `fork` 的子 run 由 `derive_child` / `fork_snapshot` 从调用方快照继承 steps，继承的部分不由子 run 写入 worklog |
| Session worklog | libopendan `.opendan_agent_session/worklog.jsonl` | 严格只追加的 Session 历史：输入批次（`turn_started` / `input_batch` / `user_message`）、function call 的 `assistant_message`、behavior 的 `step`、`action_result`、`outcome`、`turn_ended`、`compaction` 等；用于审计、展示，并经 `summary.json` 重建下一个 run 的 `<session_history>` | 长期；压缩只写 `summary.json` 和 `compaction` 条目，不改已写记录 |

- Snapshot 不应该承担长期历史展示职责；Session worklog 也不应该被 `llm_context` 当作恢复执行状态读取。需要从完整历史重建上下文时，由 Session 层读取 worklog / summary 后生成新的 input，`llm_context` 不理解 worklog。
- function call run 的 flush 单位是消息（每个 Round 的 assistant response 记为 `assistant_message`，工具结果记为 `action_result`）；behavior run 的 flush 单位是已完成的 `StepRecord`，inner transcript 不写入 worklog。
- 旧 opendan Runtime 的 `Round History`（`.meta/round_logs.jsonl`、`RoundFullPayload`）**不是**新抽象的宿主协议：新抽象的 Session 历史就是 libopendan worklog。旧 Runtime 按新抽象接入待下一阶段 opendan 重构接入（见下文“旧 opendan Round History”）。

具体不变量：

- `LLMContextState.accumulated` 已有部分在一次推理运行中视为只读。
- behavior mode 下已经 sediment 的 `StepRecord` 在渲染时不应因为 recency、预算、推理次数变化被自动降级。
- 每次 LLM/provider 返回后，只能把 assistant message、tool result message、或 behavior step 结果追加到状态尾部。
- `build_inner_request` 只能把当前状态稳定地物化为本次推理的输入，不能借渲染动作改写旧历史语义。
- `StepRenderer` 应是纯渲染器，不应承担压缩策略决策。

如果需要修改历史，只能在 `run()` 返回 Outcome 之后，由上层 session 层显式负责。

## 允许的历史改写点

历史压缩、裁剪、摘要化属于 session 层职责。允许发生在这些边界：

- `LLMContextOutcome::Done` 后：session 可以根据策略压缩已完成的上下文，然后持久化新 snapshot。libopendan 在 run 结束、历史 flush 进 worklog 并提交之后，（配置了 summarizer 时）按比例压缩 Session 历史（`summary.json`），下一个 run 的 input 由它重建。
- `LLMContextOutcome::ContextLimitReached` 后：session 重写历史后恢复执行——function call 用 `ResumeFill::RewrittenHistory`，behavior 用 `ResumeFill::RewrittenSteps`（进行中 Step 的 inner transcript 由 waist 保留）。libopendan 的做法见下面的 history epoch。
- 手动命令，如 `/compress`：只能在 session 非 running / 非 waiting tool 状态下执行，避免改写正在推理中的上下文。
- session 恢复或 behavior 交接前：上层可以把旧状态整理成新的初始状态，但整理结果必须落盘，成为之后推理的稳定输入。libopendan 的 behavior 交接从不改写已有 run 的历史或 system（在同一个 run 里换 behavior 的普通切换已移除）：`switch_context` 恢复目标自己的快照或按目标配置新建 run；`create_sub_context` / `fork` 用纯函数 `derive_child` / `fork_snapshot` 从调用方快照派生新 run 的初始快照，调用方快照原样保留，返回后从挂起点继续。fork 子 run 保留调用方的 system 和分叉点之前的完整有效历史，前缀与调用方一致。

这些改写必须是显式事件，应该写入 session history / worklog，方便调试和审计。

### history epoch：上下文上限的中途重写边界（libopendan 当前实现）

run 以 `ContextLimitReached` 让出时，libopendan 不在快照里改写旧历史，而是把它交还 Session 历史，并开始一个新的 history epoch（详见 [Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md) §7）：

1. 按 flush 游标把本 run 已产生而未写入 worklog 的历史写入，末尾追加 `outcome(kind=context_rewritten)`，与新游标在同一次 `state.json` 提交（没有新历史时两者都不写，重做不会重复）。
2. 压缩 Session 历史（`summary.json` + `compaction` 条目），重建历史消息。
3. 以 `system + 重建的历史` 作为新 input 恢复：function call 用 `RewrittenHistory`；behavior 用 `RewrittenSteps`，steps 全部折叠进 input、编号继续，进行中 Step 的 inner transcript 原样保留。快照的 `HostMeta`（`state.host["libopendan"]`）进入新 epoch：`history_epoch + 1`，`epoch_turn` = 当前 Turn，`epoch_input_seq` = 已有 receipt 的最大 `input_seq`，`base_input_len` 改为新 input 的长度；先发布快照再继续推理。

flush 游标（`LiveRun` / `ProcessFrame`）按两种 run 分开：

- function call run：`flushed_message_count` = history prefix（`request.input`）之后已写入的消息数，只在 `flushed_epoch == HostMeta.history_epoch` 时有效；快照进入新 epoch 后从 0 计，`input_seq ≤ epoch_input_seq` 的 receipt 不再用位置定位消息（身份仍有效）。
- behavior run：`flushed_step_index` 是身份高水位，不是计数——`step_index` 小于它的 step 已写入（子 context 继承的、`inherited_below` 之下的 step 从不由子 run 写入）；注入消息按 `flushed_input_seq` 去重。这两个游标不受中途重写影响。

崩溃恢复：③ 之前崩溃，用旧 epoch 的推理前快照再次让出、重写，已写入的部分由游标跳过；③ 之后崩溃，新 epoch 从 0 计。receipt 的 `input_seq` 与输入消费位置不随重写改变；重写也不是 Turn 边界，当前 Turn 保持打开。

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
- `InferenceHook` 只拿 `&LLMContextSnapshot`，按接口约束不能直接修改 waist 状态；`CheckpointHook` 的注入只在推理边界（behavior 模式为 Step 边界）追加消息，不改旧 prefix。

这些路径可以作为目标模型：`llm_context` 不理解 session 历史，只维护一次推理过程的输入、输出和恢复点。

### 让 LLMContext 变复杂的部分

behavior mode 当前把较多 session/agent 语义带进了 `LLMContextSnapshot`：

- `LLMContextState` 里有 `steps`、`history_summaries`、`history_inputs`、`last_step`、`last_report`、`next_step_index`、`next_action_id`。
- `build_inner_request` 在每个 Step 开始内层推理时调用 `renderer.render_history(state.steps, ...)`，再渲染 `last_step` 并接上 inner transcript，说明 behavior history 仍在 `llm_context` 内物化成 prompt。
- `snapshot_overrides` 位于 `llm_context` crate 内，可以替换 system/user message、清空 step/history state、移动 hot tail。它现在只定位为“从同一个 run 自己的快照改 request 侧参数重建”；context 之间的交接不在已有历史上替换 system，改由 `context_derive`（`derive_child` / `fork_snapshot`）派生子快照。`RequestOverrides.system_messages` / `user_messages` 只为旧 opendan Runtime 保留。
- message-level rewrite 与 behavior-step history 的双轨语义已由 `ResumeFill::RewrittenSteps` 收口：behavior 模式的 `ContextLimitReached` 只接受 `RewrittenSteps`（同时替换 `request.input` / `history_summaries` / `steps` / `last_step`），`RewrittenHistory` 只用于 function call 模式。

这些不是马上必须删除的 bug，但它们是复杂度来源。按“LLMContext 越简单越可靠”的目标，后续应逐步把这些上移到 session 层。

### 旧 opendan Round History（待下一阶段 opendan 重构接入）

旧 opendan Runtime（`src/frame/opendan`）有自己的 `round_history` 记录系统，这一阶段只做了共享 API 的编译适配，不是新抽象的宿主协议：

- Round History 有自己的 `.meta/round_logs.jsonl`、round summary、entry seq、`EntryPayload::Message/Step/Event`；它的 “round” 是旧 Runtime 由输入触发（`RoundTrigger`）的推进单位，不是 Round（一次推理），也不等于 Turn。
- `SessionHistoryRecorder::record_run_diff` 从 final snapshot 中抽取新增 message 或 step，写入 Round History。
- `HistoryEvent::Compaction` 只记录压缩事件，历史主体不应该因为 snapshot 压缩而被当场改写。

它体现的方向（审计 / 展示层与恢复状态分离）与新抽象一致；新抽象中对应的是 libopendan Session worklog。旧 Runtime 迁移到 worklog / Turn 语义属于下一阶段 opendan 重构。

### Session 层的正确边界

当前实现（libopendan）：

- run 快照先 fsync 再发布到 `runs/<run_id>/snapshots/`，`state.json` 是会话提交点，worklog 追加先于它写入。
- run 结束、交接挂起（`switch_context` 的 Parked 帧、子 context 调用方的 Caller 帧）、上下文上限重写前，都先按 flush 游标把未写入的历史追加进 worklog，再继续。
- 交接点（run.json 的 `handover`）随快照先落盘，再提交 `state.json`；崩溃后由 reconcile（`redo_transfer`）恰好补交一次，state 用 `handover_at_ms` 记住已提交的交接。
- 已返回的子 context（`process_done`）的记录留在 worklog 供审计，但重建 `<session_history>` 时被排除，只渲染它的 `process_done` 结果（`runner/history.rs`），避免子过程的完整内容混进其它 context 的输入。压缩输入做同样的过滤，但压缩只识别被压缩片段内的 `process_done`：切点把子 run 的记录与它的 `process_done` 分开时，切点之前的那部分仍会进入摘要。
- 上下文上限在 Session 层重写（上面的 history epoch），不在 waist 内。

旧 opendan Runtime（待下一阶段 opendan 重构接入）也有同方向的边界：

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
- 进行中 Step 的 inner transcript 接在物化历史之后，Step 内的每次推理只在它尾部追加；Step 完成后它被 `StepRecord` 取代，这一变化发生在 Step 边界。

中期更清晰的方向是把 behavior history 的物化也上移到 session 层：session 层决定 `StepRecord`、summary、用户输入和压缩摘要如何组成最终 `AiMessage` 序列，`llm_context` 只消费已经准备好的 `request.input / accumulated`。

## 修复方向

1. 移除 `XmlStepRenderer` 的 recency-based compact 逻辑。（已完成）
2. 移除 `LLMContextDeps.history_compressor` 及 behavior loop 内部自动 compressor 入口。（已完成）
3. 保持 `llm_context` 推理期间 append-only，不在内部修改旧 message / old step。
4. 宿主 Session 层负责 context-limit 重写、历史压缩和手动压缩，并在压缩后持久化 snapshot：当前实现是 libopendan `SessionRunner`；旧 opendan Runtime 的 `llm_message_compress` 等待下一阶段 opendan 重构接入。
5. 后续如果要压缩 `StepRecord` 维度历史，应在 session 层实现为显式状态改写（`RewrittenSteps`），而不是在 renderer 或 loop 内自动发生。

## 后续简化路线

目标是让 `LLMContext` 回到“非常简单”的 waist：

1. `LLMContextRequest.input` 是本次推理的完整输入前缀。
2. `LLMContextState.accumulated` 是 append-only 的运行中 transcript。
3. `LLMContextSnapshot` 只保存恢复执行必要的 request、accumulated、挂起与续派状态（`suspended` / `tool_batch` / `action_step`）、工具迭代额度等预算计数、usage、abort/task id 等机器状态。
4. behavior 的 `StepRecord`、report、behavior switch、process stack、Session worklog 的增量 flush 都由 Session 层管理。
5. session 层在启动或恢复一次 LLMContext 前，把自己维护的 history/step/summary 明确物化成 `AiMessage`，作为新的稳定 input。

按这个路线，`llm_context` 最终不需要知道 Session worklog，也不需要知道 `StepRecord` 的长期保存结构；它只负责把 `AiMessage` 送进 provider、执行 tool、append observation、返回 outcome/snapshot。

## 当前仍需注意的风险

- behavior mode 现在仍由 `llm_context::build_inner_request` 调 `render_history`，所以 StepRecord prompt 物化还没有完全上移到 session 层。
- libopendan 的上下文上限重写把 steps 整体折叠进 input，不做 StepRecord 维度的细粒度压缩；旧 opendan Runtime 的 `llm_message_compress` 主要压缩 `state.accumulated`，behavior prompt 的 token 主要来自 `steps/last_step` 时仍可能压不下来（待下一阶段 opendan 重构接入）。
- `snapshot_overrides` 仍在 `llm_context` crate 内，其中替换 system / user message 的部分只有旧 opendan Runtime 在用；libopendan 的交接不使用它，子 context 由 `context_derive` 的 `derive_child` / `fork_snapshot` 构造。派生规则（继承边界、分叉点、编号延续）目前也在 waist 内，长期看应评估哪些可以迁到 Session 层，减少 waist 对 agent 语义的认知。

## 判断标准

完成后应满足：

- 连续两次推理（Round）中，上一次已发送的 message prefix 字节级稳定。
- 新 action result / assistant response 只追加在尾部。
- 未触发 session 层压缩时，旧 `StepRecord` 不会从 full 变 compact。
- 触发压缩时，有明确的 session 事件记录，并且压缩后的 snapshot 成为新的稳定基线。
- 任意时刻都能回答：某个数据是“恢复执行状态”（snapshot / inner transcript / 未 flush 的 StepRecord）还是“Session worklog 审计记录”；如果回答不清，说明职责边界又混了。
