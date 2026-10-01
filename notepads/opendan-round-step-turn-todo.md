# lib_opendan / llm_context Round / Step / Turn 术语统一 TODO

日期：2026-10-01

状态：第一阶段（lib_opendan + llm_context + 配套文档）已于 2026-10-01 实施，D1–D6 按第 6 节建议方案落地，实施记录见第 9 节；第二阶段（opendan 重构）未开始。

Review 基线：仓库 HEAD `39d6d6f0`，以及工作区当前的 [LLM Context readme](../doc/llm_context/readme.md)（重点为第 3–10 行）。

本次交付是现有实现的 Review 和实施 TODO。设计文档中的伪代码、旧 TODO 和实现计划作为参考材料，不自动扩展为本次需要执行的任务。按 beta 2.2 breaking change 处理后续修改，不要求保留旧字段、旧接口或旧磁盘格式的兼容别名。

本轮范围为 `lib_opendan`、`llm_context` 和 `doc/llm_context` 目录：代码、接口、持久化协议、配置示例及文档同步统一 Round / Step / Turn。xllm / `agent_tool` 按这两层的接口变化做必要联动。`doc/llm_context` 包括 `agent_tool/` 子目录，目录内涉及术语、状态机或旧实现抽象的内容都要检查并同步，不再仅将其作为参考材料。

实施分为两个阶段：先完成 `lib_opendan` 与 `llm_context` 的新抽象及配套文档，再按该抽象重构 `opendan`。现有 `src/frame/opendan` 的 Runtime／历史工具／Jarvis／Topic/Recall／Telegram 整体改造属于后续阶段，不在本轮逐项迁移；共享 API 变化引起的调用点编译适配不等于提前实施旧 Runtime 重构。本次仍交付 TODO 供 Review，不开始执行这两个阶段的代码修改。

## 1. 设计口径与 Review 结论

| 术语 | 采用的设计定义 | 必须区分的边界 |
|---|---|---|
| Round | LLM 层的一次推理 | 不是一次 `LLMContext::run()`、一个工具批次或一次 Session 输入推进；最后一次无工具的回答也是 Round |
| Step | Behavior Loop 产生一次行为决策，并记录决策及动作结果的 `StepRecord` | 内层可以有多个 Round；一个 Step 可以有多个 action；标准 function-call Loop 不使用 Behavior Step 概念 |
| Turn | AgentSession 完成一次逻辑 Input → result | 可以跨多个 Step、behavior、context/run；内部交接和可恢复挂起不自动结束 Turn |
| run / run 调用 | LLMContext 的执行与一次 `run()` 返回边界 | 保留执行层术语；不能用 run 数、调用次数或 Outcome 数反推 Turn 数 |

llm_context 的 Behavior Loop 分层基本符合设计：`run_inner_for_step` 复用传统 Loop，获得最终 response 后解析决策、派发 action、沉淀 `StepRecord`。主要问题是两层的 Session 推进、工具预算、hook、历史记录和统计仍复用了 Round / Step / Turn 名称，部分计数与完成边界也确实不同。

本 TODO 的实施入口：

- [lib_opendan Runner](../src/frame/lib_opendan/src/runner/drive.rs)：新 Session 目录协议及参考 Runner。
- `src/frame/lib_opendan/src/protocol`、`session/runs.rs`、`state/perception.rs`、Runner history/flush，以及 `examples` / `tests`。
- `src/frame/llm_context` 的 request/state/outcome、两种 Loop、hook、snapshot overrides、挂起恢复、renderer 和测试；xllm / `agent_tool` 联动入口为 `local_llm_context.rs` 与工具运行上下文。
- 文档同步覆盖整个 `doc/llm_context`，以及 libopendan 的 README、Session SDK 文档和 `doc/opendan/protocol` 生成材料。此次搜索未在 desktop 或相邻 `buckyos-websdk` 源码中发现上述 Session 历史字段的直接消费者；实施时仍需复查。

## 2. P0：Session 的 round 不能直接批量改成 turn

### 2.1 libopendan 的 round 是输入／交接驱动的推进编号

证据：

- [drive.rs](../src/frame/lib_opendan/src/runner/drive.rs) `begin_round`（约 1235–1329 行）为每次输入注入计算 `state.round + 1`；bootstrap、msg/event 和 `internal_continuation` 都可以触发。最后一项包括普通 behavior switch、fork 子过程返回及 independent 切换。
- [receipts.rs](../src/frame/lib_opendan/src/runner/receipts.rs) `apply_receipt`（约 105–138 行）按 `opens_round` 更新 `state.round`；观察阶段的 receipt 则合并到当前项。[hook.rs](../src/frame/lib_opendan/src/runner/hook.rs) 的观察注入明确写 `opens_round: false`。
- [runner_more.rs](../src/frame/lib_opendan/tests/runner_more.rs) `normal_switch_continues_the_same_run`：三个模型调用、同一个 run，写出两个 `round_started`。`fork_child_inherits_steps_and_returns_to_the_parent_run`：五个模型调用，写出三个 `round_started`。

因此当前 `state.round` 既不是 Round 计数，也不是设计中的已完成 Turn 计数；它在输入被提交时增长，而且内部切换也增长。

- [x] **先确定逻辑 Turn 的开始、继续和结束规则（D1、D2），再改 `state.round`。** 建议普通切换、fork 调用／返回和 independent 切换都继续当前 Turn；新增外部输入是否开 Turn 由 Session 调度决定，不能以是否生成了 UserMessage 判断。
- [x] 如果需要保留现有推进编号用于提交、恢复和输入排序，将其命名为 `advance_seq`，不要直接叫 `turn_index`；真正的 `turn_index` 必须按逻辑 Turn 的规则产生。是否保留该编号见 D5。
- [x] 将 `begin_round` / `commit_round` 改为描述实际职责的名称，例如 `commit_input_batch` / `handle_context_outcome`；函数返回一个 Outcome 时不意味着逻辑 Turn 已完成。
- [x] 同步审查整个持久化链：`SessionState.round`、`RoundInputs`、`LiveRun.rounds`、`ProcessFrame.rounds`、`InputReceipt.round/opens_round`、`HostMeta.epoch_round`、所有 `WorklogBody` 的 `round`。输入批次身份仍用已有 `(run_id, input_seq)`，恢复仍按 receipt 补交且不重复注入消息。
- [x] 明确保存当前 Turn 身份的位置，使其跨普通切换、fork/independent、history epoch 重写和重启保持稳定。Turn 结束判定由 Session 解释 Outcome 和交接状态，不能放到 waist 层按 `Done` 一刀切。

### 2.2 两种 MaxRounds 现在统计的对象也不同

证据：

- [config.rs](../src/frame/lib_opendan/src/protocol/config.rs) `EndConditionType::MaxRounds` 在 `drive.rs::decide_end`（约 1343 行）比较 `state.round`，实际比较开始过的输入／交接推进次数。
- [runner/mod.rs](../src/frame/lib_opendan/src/runner/mod.rs) `StopWhen::MaxRounds` / `DriveResult::RoundsDone` 在 `drive.rs`（约 2220、2403–2420 行）使用 `rounds_done`，每次 `run_compacting` 返回并处理 Outcome 后加一；恢复同一 run 也可能计入，挂起也可能计入。
- `normal_switch_across_drives_runs_the_next_behavior` 使用 `MaxRounds { n: 1 }` 在 behavior 交接后让 drive 返回；此时任务还没产生最终结果。

- [x] 将 Session 的结束条件与 drive 的让出控制权条件分开命名。若保持现有 drive 行为，建议 `StopWhen::MaxOutcomes` / `DriveResult::OutcomesHandled`，明确只统计 drive 主循环处理的 Outcome；`run_compacting` 内部可以多次调用 `ctx.run()`，所以这个额度也不能叫模型调用数或 run 调用数。不要把它直接改成 `MaxTurns` 后仍在每个 Outcome 加一。
- [x] Session 结束条件若改为 `max_turns`，必须按已经完成的逻辑 Turn 计数，内部 handoff 不占额度；若仍要限制现有推进次数，则明确命名为 `max_advances`。这两项行为选择见 D5。
- [x] 明确 `n = 0`、一次 drive 从挂起恢复、等待输入提前返回和中途错误时的返回／计数语义。

## 3. P0：工具预算中的 rounds 不是推理 Round

证据：

- [request.rs](../src/frame/llm_context/src/request.rs) `ToolPolicy.max_rounds`、[state.rs](../src/frame/llm_context/src/state.rs) `rounds_left`、[outcome.rs](../src/frame/llm_context/src/outcome.rs) `BudgetKind::ToolRounds` 描述的是工具预算。
- [context_loop.rs](../src/frame/llm_context/src/context_loop.rs) `run_tool_batch`（约 729–741 行）完成一批原生工具才扣一次；`run_behavior`（约 1268–1272 行）对一个带 action 的 Step 扣一次。内层原生工具与外层 action 共享该额度（约 1598–1607 行）。
- 无工具的最终回答仍会推理，不扣该额度；纯 report、解析纠错等也不能按这个剩余额度推算推理次数。原生工具 Pending 的扣减在批次完成后，behavior action 的扣减在派发前，恢复时均不能再扣一次。
- [tests.rs](../src/frame/llm_context/src/tests.rs) 的 `behavior_action_batch_uses_one_round_and_allows_final_response`、`behavior_native_tools_and_actions_share_rounds_across_steps` 已明确验证这个计数对象。例：额度为 1 时可以执行一个 action 批次，再推理一次给最终答案。

- [x] 在 llm_context 与 libopendan 中统一工具额度命名，建议 `ToolPolicy.max_tool_iterations`、`LLMContextState.tool_iterations_left`、`BudgetKind::ToolIterations`，同步配置说明、报告和错误文案。一次 iteration 是一个原生工具批次，或一个带 action 的 Behavior Step，不是 Round，也不是所有 Step 的计数。最终名称及是否拆分预算见 D3。
- [x] 同步修改 [snapshot_overrides.rs](../src/frame/llm_context/src/snapshot_overrides.rs) 的 `reset_rounds`，并审查 libopendan fork / normal / independent 调用链的继承／重置；术语修改保持现有额度消费策略。
- [x] 联动 [local_llm_context.rs](../src/frame/agent_tool/src/local_llm_context.rs) 的配置字段、严格键校验、override、`RunLimits`、`ResumeLimits`、请求转换和持久 `RunRecord.config`，使 `prompt.llm_context` 与 hosted 执行采用一致的新名称。resume 调整总额度时仍应扣掉已经消费的额度。
- [x] 同步 llm_context / xllm / libopendan 的示例、测试、协议文档和 fixtures；代码及文档使用同一组新字段，不新增重复配置或兼容别名。
- [x] `max_calls_per_round` 当前限制单次模型 response 的原生 tool calls 数量，符合 Round 定义；应补清楚其适用范围，不必随工具预算一起机械改名，也不要用它表示一个 Behavior Step 的 action 数量。
- [x] 仅在确实需要限制推理总次数时另行定义推理预算。本次术语统一不自动新增一个行为不同的 `max_rounds` 限制，更不能把现有配置值直接迁移为推理上限。

## 4. P0：worklog 中的 Step 和 flush 游标混用了两种粒度

证据：

- [flush.rs](../src/frame/lib_opendan/src/runner/flush.rs) 的传统模式把每条 `AiRole::Assistant` 写为 `WorklogBody::Step`（约 228 行），包括原生 tool-call response 和最终答案。这些并没有 Behavior `StepRecord`。
- 同文件的 `FlushMarks.step` 在传统模式统计 `accumulated` 前缀之后的**消息数**，User / Assistant / Tool 消息均计入；在 behavior 模式却表示已经 flush 的 Step 索引边界。
- [protocol/state.rs](../src/frame/lib_opendan/src/protocol/state.rs) `LiveRun.flushed_step` / `ProcessFrame.flushed_step` 把这种双重语义公开为磁盘协议。
- `step_entries` 从 `StepRecord` 导出 worklog 时没有保存 `step.meta.step_index`；worklog 的全局 `seq` 能排序，但不能直接还原具体 Step 身份。

- [x] 将标准 function-call response 写为独立的 `assistant_message` 记录，`step` 只用于 Behavior 决策记录；`actions` / `action_result` 的关联仍用稳定 `call_id`。
- [x] 为 Behavior worklog 的 Step 保留 `step_index` 及必要的 Turn/run 归属；其适用范围按 D4 明确。不要为了统一命名把每条传统 assistant message 包装为假 StepRecord。
- [x] 拆清游标：建议传统模式用 `flushed_message_count`，behavior 模式用 `flushed_step_index`，或者使用带模式标签的游标。history epoch 与 `input_seq` 的去重语义仍需保留。
- [x] 同步 [history.rs](../src/frame/lib_opendan/src/runner/history.rs) 的渲染／压缩策略：`recent_full_steps` 不能在传统模式继续默默指 assistant response；若两种模式共用配置，明确采用何种通用历史单元。
- [x] 保留未完成 Step 的恢复状态与已完成 Step 的区分。`action_step` 中正在等待结果的记录、`next_step_index` 已分配的编号，都不能直接计为“已完成 Step 数”。解析／策略失败产生的合成 Step 计数规则见 D4。

## 5. P1：推理边界、工具调用编号和统计

### 5.1 TurnHook 与 inner turn tail

- [x] 将 [deps.rs](../src/frame/llm_context/src/deps.rs) 的 `TurnHook` / `turn_hook` / `with_turn_hook` 改为 `InferenceHook` 等准确名称：它在每次推理前执行，不在 Session Turn 开始或结束时执行。同步 llm_context、libopendan、xllm 的接口引用、测试及 `doc/llm_context` 中的定义／伪代码。
- [x] 保留 `CheckpointHook` 的职责区分。传统模式在推理边界调用它，behavior 模式在外层 Step 边界调用它，不能因统一术语而把外层观察／提交 hook 自动挪到每次内层推理；既有同步 hook 与异步 hook 的快照含义也不同。
- [x] 将 `context_loop.rs` 的 `behavior_turn_tail` / `set_behavior_turn_tail` / `clear_behavior_turn_tail` 和 [suspension.rs](../src/frame/llm_context/src/suspension.rs) 的 `turn_tail` 改为描述 inner transcript 的名称。它们保存的是当前 Step 内尚未完成的原生工具 Loop 消息尾部，不是整个 Session Turn；同步 libopendan 的恢复／压缩说明和 ResumeFill 文档。

### 5.2 xllm 的工具调用编号不等于 Behavior Step

证据：[local_llm_context.rs](../src/frame/agent_tool/src/local_llm_context.rs) `XllmToolManager` 的 `step_idx` 在每次工具调用时增长；它是 libopendan hosted 工具执行链的依赖。该值不是 `StepRecord.meta.step_index`，也不是 Session Turn 编号。

- [x] libopendan 日志、结果归属和恢复定位使用各自真实身份：工具用 `call_id`，Behavior Step 用 `(run_id, step_index)`，Turn 用 Session 的逻辑 Turn 身份。不能从 [agent_tool/lib.rs](../src/frame/agent_tool/src/lib.rs) 的 `SessionRuntimeContext.step_idx` 推导 Step／Turn 数。
- [x] 将这条调用链按工具调用增长的 `step_idx` 改为 `tool_call_index`，同步工具运行上下文、xllm 派发及调用点，避免新抽象继续混用调用编号和 Behavior Step。现有 opendan 的 Topic/Recall 或历史工具的业务重构留到下一阶段。

### 5.3 推理统计与感知摘要

证据：`drive.rs::commit_round` 使用 `lc.run.record_usage(u, 0)`；[session/runs.rs](../src/frame/lib_opendan/src/session/runs.rs) `record_usage` 将这个 0 赋给 `RunRecord.usage.llm_requests`。`finish_run` 只更新 `SessionStatic.runs/rounds/tokens/cost`，没有累计 `llm_requests`；`rounds` 被赋为 `state.round`。当前并没有可靠的 Session Round 计数。

- [x] 明确 Round 统计口径，建议从宿主可见的 `LlmClient::infer` 调用边界计数，成功、失败／中断的尝试如何展示按 D6 决定。xllm 已有 `TimeoutLlmClient.calls()`，优先复用其计数方式；不能数 `WorkEvent::LLMStarted`，后者是在 `LLMContext::run` 入口发出。
- [x] 补清 libopendan hosted 执行与 xllm 接手执行的 `llm_requests` 保存／累计规则，避免宿主把已有统计覆盖为 0。是否在本轮补齐计数能力见 D6；未补齐时不得把旧 `rounds` 改名后声称是推理统计。
- [x] Session 统计与 report 明确区分 Round 数、已完成 Turn 数和 run 数；暂停／重启不重复累计，fork 的继承历史不重复计数，摘要推理是否计入单独说明。
- [x] [perception.rs](../src/frame/lib_opendan/src/state/perception.rs) 的 `round_digest` 当前在 `finish_run` 等位置生成，payload 的 `round` 也是 Session 推进编号。根据真实写入边界选择 `run_digest` 或 `turn_digest`；后者只能在逻辑 Turn 完成时发出。同步 `protocol/agent_state.rs`、self-improve 输入、fixtures 和 [history.rs](../src/frame/lib_opendan/src/runner/history.rs) 的 `── round … ──` 展示。

## 6. 待 Review 的歧义／决策

下列项无法仅靠替换标识符决定。建议方案用于 Review，尚未视为已批准的设计。

| 编号 | 问题 | 建议方案 |
|---|---|---|
| D1 | Turn 的 Input 是一条消息、一次消费批次，还是一个逻辑请求？bootstrap、event 和 Step 观察阶段消费的新消息如何归属？ | 调度器形成逻辑输入批次，bootstrap/event 也可开 Turn；处理中的补充／观察输入加入当前 Turn，排队的新请求等下一 Turn。保留原始输入引用，明确合并策略 |
| D2 | `WAIT_USER_MSG`、Error、BudgetExhausted、stop/cancel 怎样结束 Turn？结果消息和 `<report>` 是否必然结束？ | behavior/context 切换及可恢复挂起继续 Turn；等待输入若已交付一个逻辑答复则完成 Turn，否则保留未完成任务的归属；可重试 Runtime 错误不自动完成。不可恢复错误／预算终止／明确 stop 记录失败或终止边界。`sendmsg` / 中间 report 本身不等于 Turn 完成 |
| D3 | 工具额度采用什么新名称？native tools 与 action Step 是否继续共享，是否另加推理额度？ | 本轮统一 llm_context / libopendan / xllm 的工具额度名称，建议 `max_tool_iterations`，保留现有共享消费语义；拆分 `max_native_tool_batches` / `max_action_steps` 或新增推理额度作为独立行为变化 Review |
| D4 | Step 编号在哪个范围内唯一？合成解析／策略错误是否算 Step？ | 保持现有编号产生方式，先用 `(run_id, step_index)` 定位；fork 继承记录不算子 run 新产生的 Step。合成纠错 Step 与正常决策分开展示，`next_step_index` 只表示分配位置；暂不假定 independent contexts 共用全 Session Step 序列 |
| D5 | 旧 Session 推进编号及 MaxRounds 要保留原语义吗？ | 恢复需要的输入批次编号继续复用 `input_seq`；确有独立推进限制／审计用途才保留 `advance_seq/max_advances`。限制 Turn 的接口用 `max_turns`，drive 主循环的 Outcome 配额用 `max_outcomes`，各自有明确计数对象 |
| D6 | Round 是一次宿主推理尝试、一次成功 response，还是 provider 实际尝试次数？是否必须本轮落地统计？ | 默认以宿主 `infer` 为一次尝试，另列成功／失败／中断；provider adapter 内重试不作为宿主 Round。摘要／file-model 推理明确归属。若本轮只统一命名，则把计数补齐列为后续项并标明统计尚不可用 |

确定 D1/D2 后，还需给出 independent context 轮换中 Turn 所属 Session 的一致规则；Sub AgentSession 则有自己的 Turn，父子 Session 的 Turn 不合并计数。本次不包含创建新的 Sub AgentSession / CreateSubContext helper。

## 7. P0：doc/llm_context 与代码同步；示例和生成材料联动

`doc/llm_context` 的同步修改是第一阶段完成条件，不能仅加一段新术语定义后保留与它矛盾的接口、伪代码和旧 Runtime 说明。检查整个目录（含 `agent_tool/`）；涉及同一抽象的正文、代码片段、表格、示例和链接一起更新，没有相关内容的文件不作无意义改写。

| 文档入口 | 必须同步的内容 |
|---|---|
| [readme.md](../doc/llm_context/readme.md) | 三层定义、LLMContext run / Outcome 与 Turn 的区别、串行 context 切换、父子 Session 边界；保持并核对工作区已有的伪代码和消息结构，新实现落地后更新 `state.round` 的说明 |
| [LLM Context 设计.md](<../doc/llm_context/LLM Context 设计.md>)、[llm_context_behavior_loop.md](../doc/llm_context/llm_context_behavior_loop.md) | 工具预算、InferenceHook、snapshot/ResumeFill、新字段及状态机；删除或修正“内层不允许 yield”“切 behavior 总是新 context”等已过时描述；保留 Step 未完成时的恢复状态 |
| [local_llm_context_protocol.md](../doc/llm_context/local_llm_context_protocol.md)、[xllm_rust_sdk.md](../doc/llm_context/xllm_rust_sdk.md) | 新配置键、RunLimits / ResumeLimits、错误与预算结果、hosted 执行／接手恢复和样例；工具额度与 Round 计数分别说明 |
| [llm_context_append_only_history.md](../doc/llm_context/llm_context_append_only_history.md) | snapshot、inner transcript、StepRecord 和 Session worklog 的职责；旧 `Round History` / `.meta/round_logs.jsonl` 不再作为新抽象的宿主协议；明确 history epoch 重写边界 |
| [why behavior loop.md](<../doc/llm_context/why behavior loop.md>) | `StepRound` 改为 Step / StepRecord；消息和切换示例对齐 libopendan 的 normal / fork / independent，旧 opendan 私有 helper／磁盘布局不作为新设计接口 |
| [Agent Message.md](<../doc/llm_context/Agent Message.md>)、[Agent Actions.md](<../doc/llm_context/Agent Actions.md>)、[improve-behavior-report.md](../doc/llm_context/improve-behavior-report.md) | message、decision、action batch、report、behavior Done 和 Session Turn 完成的区别；核对跨文档引用，避免沿用旧“大 Round”定义 |
| [LLM Compress.md](<../doc/llm_context/LLM Compress.md>)、[llm_compress_implementation_guide.md](../doc/llm_context/llm_compress_implementation_guide.md) | 压缩作用的消息／Step／worklog 单元、稳定前缀和显式恢复边界；旧 `RoundFullPayload`、旧 Runtime 测试入口与新抽象的对应关系，不能用 UserMessage 数充当 Turn 数 |
| [LLM Understand Media.md](<../doc/llm_context/LLM Understand Media.md>) | `per-turn` 持久化的实际粒度、旁路推理与父 Step／Turn 的归属；模型推理 checkpoint、工具执行结果提交和 Turn 完成分别说明 |
| [prompt_render_engine.md](../doc/llm_context/prompt_render_engine.md)、[Render_Prompt_Template_Variables.md](../doc/llm_context/Render_Prompt_Template_Variables.md) | Round / Step / Turn 变量、历史和动作回响的实际来源；模板示例与 renderer 接口一致 |
| [agent_tool/](../doc/llm_context/agent_tool) 下的协议、指南、内置工具、实例和需求文档 | 工具调用身份、Step action 结果和 Pending 回填；嵌套模型调用不冒充 Session Turn；预算及运行上下文的新字段同步到工具示例 |

- [x] 按上表完成目录内文档同步，标明“当前实现”“设计接口”和“待下一阶段 opendan 重构接入”的内容，避免把新抽象文档写成旧 Runtime 已完成迁移。
- [x] 目录中的接口／字段示例采用第一阶段实际实现的新名称；如果某项设计尚待 D1–D6 裁定，明确待定点，不通过保留两套相互矛盾的定义解决。
- [x] 同步 libopendan README、Session SDK 需求／实现计划中相关字段说明、`doc/opendan/protocol` 的目录／输入／感知说明及开发示例。
- [x] 重生成 libopendan `schema/*.schema.json` 和 `fixtures/*`，包括 `expected.json`、report、感知 payload、receipt、host metadata；由 Rust 类型和 fixture 工具导出，避免只手改样例。已有 schema 中的 `snapshot_version` 等旧说明也应与当前导出类型核对。
- [x] 对被修改语义的 session/worklog/host/snapshot/run 格式明确新版本及旧格式拒绝策略；本次 breaking change 不加 serde alias 或双读双写。仅改函数／局部变量名不必无故改变无关格式版本。

## 8. 实施顺序与验收

### 第一阶段：lib_opendan + llm_context + 配套文档

1. Review 并确定 D1–D6，给出两层职责及 Turn 开始／继续／完成的边界表；统一接口与字段名称。
2. 修改 llm_context 的工具预算、InferenceHook、inner transcript、Step／挂起恢复接口和测试；联动 xllm / agent_tool 必要调用点。
3. 修改 libopendan 的 Session Turn 边界、receipt/worklog 身份、传统历史记录、flush 游标、感知／统计和 hosted 执行身份。
4. 代码修改同时同步整个 `doc/llm_context` 及相关 Session 协议文档，重新导出 schema/fixtures，核对例子和新接口。文档同步不延后到 opendan 重构。
5. 通过两层及联动依赖的测试，交付可作为下一阶段重构依据的抽象、协议和文档。

### 第二阶段：按新抽象重构 opendan

第一阶段完成后，以其 Round / Step / Turn、Session Runner、worklog、context 切换及恢复契约为基础重构 opendan，再处理原 Runtime、历史工具、Jarvis 和业务集成的迁移。该阶段另立实施任务；本 TODO 不要求先对现有 opendan 做一轮独立术语清理，也不为保留旧 Runtime 的私有抽象建立兼容层。

建议在已有测试基础上增加真正验证三层关系的断言，改实现后再运行：

| 场景 | 预期关系／不变量 |
|---|---|
| 标准 Loop 两次 tool-call response 后最终回答 | 3 个 Round，0 个 Behavior Step，1 个完成 Turn；两个工具批次额度 |
| 一个 Behavior Step 内先执行 native tool，再输出决策并执行多个 action | Step 内至少 2 个 Round；多个 action 仍属于 1 个 Step；工具额度与 Round 数独立 |
| 普通 behavior switch（现有 runner_more 用例） | 3 个 Round、同一 run、一个逻辑 Input 到最终 result；内部交接不增加 Turn 数 |
| fork child 返回（现有 runner_more 用例） | 5 个 Round、父子 run；父 Turn 连续，子 steps 不合入父主干 history，Session worklog 保留子过程且不重复 flush |
| independent 切走再恢复 | 各 context 保留自己的 Step/snapshot；不把 context 切换视为已完成 Turn；不假定各自 Step 索引在 Session 内全局唯一 |
| native tool Pending / behavior action Pending | 未完成批次或 Step 继续；回填不重新推理已完成决策、不重放工具，不重复扣工具额度，不新开 Turn |
| ContextLimitReached、Interrupted 及重启 | 继续同一 Turn；重写 epoch 后消息／Step 游标有效；Interrupted 按是否真实发起模型调用统计，不能以 `run()` 次数计 Round |
| 输入／半订阅变化在观察边界注入 | receipt 顺序及确认正常；不因多了 UserMessage 自动增加 Turn 数；子 context 输入策略保持既有隔离规则 |
| 同一 Step 多个工具、history epoch 重写 | 工具调用编号不冒充 Step／Turn；Step 身份不依赖 `steps.len()`；flush 不重复／遗漏记录 |
| libopendan 感知与 self-improve 输入 | digest 的名称、payload 归属及生成边界一致；可恢复挂起不被错误标记为已完成 Turn |

可复用的测试入口：`llm_context/src/tests.rs` / `suspension_tests.rs`，`lib_opendan/tests/runner_more.rs`、`context_limit.rs`、`crash.rs`、`fixtures.rs`，以及 xllm 现有测试。命名修改后同步更新测试名，但断言要验证计数、归属和恢复行为，不能只有字段拼写检查。

后续实现验收命令（在 `src/` 下）：

```bash
cargo test -p llm_context -p libopendan -p agent_tool -- --test-threads=1
```

文档验收包括目录内旧术语检索、接口示例／伪代码与代码对照、链接检查，以及跨文档定义一致性；不能仅通过关键字替换数量判断完成。生成材料使用 `libopendan --example fixtures`。完成上述定向检查后按仓库要求做整体测试／构建检查。

Review 阶段的验证仅为源码、设计文档、配置和现有测试断言的静态对照，以及 TODO 的路径／差异检查。第一阶段实施后的测试与生成材料见 9.3。

## 9. 实施记录（2026-10-01，第一阶段）

### 9.1 D1–D6 的落地口径

D1–D6 均按第 6 节“建议方案”实施，具体取舍如下，供后续 Review 时对照：

| 编号 | 实施结果 |
|---|---|
| D1 | `SessionState.open_turn` 为空时提交的输入批次开启新 Turn（bootstrap、msg/event 都可以）；Turn 打开期间消费的 msg/event 并入当前 Turn，记入 `open_turn.inputs`；观察阶段注入的变化也属于当前 Turn，但不计入逻辑输入。runner 没有新增“新请求排队到下一 Turn”的拉取策略，仍沿用原来的输入拉取顺序 |
| D2 | 普通切换、fork 调用／返回、independent 切换、可恢复挂起（非 stop 的中断、可重试错误、context limit、PendingTool）、history epoch 重写和重启都继续 Turn。Turn 只在 `finish_run` 中关闭，与 run 结束同一次提交：Done 交付结果为 `completed`；`WAIT_USER_MSG` 只有 run 内有 `<report>`（`last_report`）或最后一步有 `<sendmsg>` 才记为 `completed`，否则 Turn 保持打开；不可恢复错误为 `failed`，预算耗尽为 `budget_exhausted`，stop 为 `stopped`。fork 子 run 结束（`process_done`）不关闭 Turn |
| D3 | 统一为 `max_tool_iterations` / `tool_iterations_left` / `BudgetKind::ToolIterations`，原生工具批次与带 action 的 Step 继续共享额度，消费时机不变；没有拆分预算，也没有新增推理额度 |
| D4 | 沿用 `(run_id, step_index)`，worklog `step` 写出 `step_index`；fork 子 run 继承的 step（`inherited_below` 以下）不写入；合成纠错 Step 由 `StepRecord::is_correction()` 识别，worklog 标 `correction`；不假定 independent context 的 step_index 在 Session 内全局唯一 |
| D5 | 不保留 `advance_seq` / `max_advances`：恢复与排序仍靠 `(run_id, input_seq)`。Session 结束条件改为 `max_turns`（按 `completed` 的 Turn 计数）；drive 让出控制权改为 `StopWhen::MaxOutcomes` / `DriveResult::OutcomesHandled` |
| D6 | 本轮补齐计数：每个 run 的 LLM client 包一层 `CountingLlm`（`runner/rounds.rs`），宿主每次 `infer` 计一个 Round，分列失败与中断（推理 future 被丢弃或 provider 报 Cancelled）；中断请求早于推理时不计。每处理一个 Outcome，把增量加到 run.json `usage.llm_requests`（改为累加）和 `static.json` 的 `rounds` / `rounds_failed` / `rounds_interrupted`。历史摘要用未计数的 client，不算 Round；xllm 接手执行的推理只累加到该 run 的 `usage.llm_requests`，不进 Session 统计 |

### 9.2 主要改动

- llm_context：工具额度改名（含 `RequestOverrides.reset_tool_iterations`、`ToolBatch.batch_error`）；`TurnHook` → `InferenceHook`；behavior “turn tail” → inner transcript（`inner_transcript*`）；注释和文档统一 Round／Step／工具迭代；`StepRecord::is_correction()`；快照格式升为 3，`resume` 拒绝其它任何版本。
- xllm / agent_tool：`.llm_context` 键与 CLI 改为 `max_tool_iterations` / `--max-tool-iterations`，`RunLimits` / `ResumeLimits` / `TaskOverrides` / `DEFAULT_MAX_TOOL_ITERATIONS` 同步；`RUN_RECORD_VERSION` 升为 2，resume 只接受当前版本；`SessionRuntimeContext.step_idx` → `tool_call_index`（agent_tool、agent_tool_cli_dev、agent-did-object-lib 和 opendan 调用点）；`usage.llm_requests` 补充说明。`product/xllm/PRD.md` 的配置样例同步改名（agent_tool 有测试直接解析该样例）。
- libopendan 协议：`opendan.session_state/2`、`session_config/2`、`session_summary/2`、renderer `libopendan.mechanical/2`。`SessionState.round` 删除，新增 `turn_seq` / `open_turn` / `turns_completed`；`InputReceipt.turn/opens_turn`；`HostMeta.epoch_turn`；`LiveRun/ProcessFrame.turns`、`flushed_message_count`、`flushed_step_index`；worklog 新增 `turn_started`、`input_batch`、`assistant_message`、`turn_ended`，`step` 带 `step_index` / `correction`，所有条目以 `turn` 归属；`EndConditionType::MaxTurns`；`MechanicalCompress.recent_full_responses`；`static.json` 新增 `rounds_failed` / `rounds_interrupted` / `turns`，删除 `llm_requests`；感知 `round_digest` → `run_digest`（`run_id` / `turn` / `turn_status`）。
- libopendan Runner：`begin_round` → `commit_input_batch`，`commit_round` → `handle_context_outcome`，故障点改为 `input_batch:*`；`Next.turn_end` 决定 Turn 结束；`SessionAssembler::render_turn` / `TurnMaterial` → `render_input` / `InputMaterial`，输入批次消息改为 `<session_input …>`；`report.md` 改为输出 `turns`；function_call 的 tool result 状态补齐 `unresolved` / `cancelled` 的识别（原实现的 `[unresolved]` 前缀永远匹配不上）。
- opendan：仅做共享接口改名的编译适配（ToolPolicy 字段、`reset_tool_iterations`、`InferenceHook`、`tool_call_index`），旧 Runtime 语义、behavior toml 的 `budget.max_rounds` 键和 `.meta/round_logs.jsonl` 等留待第二阶段。
- 文档：`doc/llm_context` 下 readme、LLM Context 设计、llm_context_behavior_loop、llm_context_append_only_history、why behavior loop、Agent Message、Agent Actions、improve-behavior-report、prompt_render_engine、Render_Prompt_Template_Variables、LLM Compress、llm_compress_implementation_guide、LLM Understand Media、local_llm_context_protocol、xllm_rust_sdk，以及 `agent_tool/` 下 readme、agent_tool_result_protocol、OpenDAN AgentTool 开发指南、agent-tool实例分析、无需返回的agent-tool；`doc/opendan/protocol` 的 README、Session Directory / Session Input / Agent State Protocol；libopendan README；《Agent Session SDK 实现计划》。《Agent Session SDK化核心需求》、builtin_agent_tools、Agent 计划任务cli工具需求、Lease Protocol、Agent DID-Object Protocol Spec 没有受影响的内容，未改。旧 opendan Runtime 的内容统一标注“待下一阶段 opendan 重构接入”。`LLM Compress.md` 的设计字段改用 `pair_index` / `min_pairs_between_compress`（Message Pair 不是 Turn），旧 Runtime 现用键 `min_turns_between_compress` 已注明待第二阶段改名。
- 生成材料：用 `cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures` 和 `--example session -- schema <绝对路径>/doc/opendan/protocol/schema` 重生成（必须给绝对路径，否则场景 10 的外部 workspace 路径是相对路径，绑定失败）。

### 9.3 验证

- `cargo test -p llm_context -p libopendan -p agent_tool -- --test-threads=1`（`--no-fail-fast`）：llm_context 184、agent_tool lib 212（2 ignored）、libopendan lib 6 + context_limit 3 + crash 20 + fixtures 13 + l1 17 + runner_basic 4 + runner_more 17 + self_improve 3 全部通过。
- `cargo build -p opendan -p agent_tool_cli_dev -p agent-did-object-lib --tests` 通过；`cargo test -p opendan --lib -- --test-threads=1` 274/275，失败的 `agent_config::tests::mini_agent_demo_parses` 读取的 `doc/opendan/mini_agent_demo` 已在更早的提交 267a426f 中删除，与本次无关。多线程运行时 `agent::tests::create_work_session*` 偶发 “build session tools: No such file or directory”，单独或单线程运行都通过（既有的并发不稳定）。
- 新增／加强的断言对应第 8 节验收表：标准 Loop 的 Round／工具迭代（llm_context `function_call_loop_counts_rounds_and_tool_iterations_separately`、libopendan `a01`）；Step 内多 Round、一个 Step 多 action（`behavior_step_spans_rounds_and_holds_several_actions`）；普通切换、fork、independent、观察注入、可重试错误后的新输入、context limit 暂停重开都只有一个 Turn；`max_turns` 只计完成的 Turn（`max_turns_counts_completed_turns_not_hand_overs`）；`MaxOutcomes` 的 n=1 / n=0；`CountingLlm` 的失败／中断分类；旧快照版本被拒绝。

### 9.4 未做与风险

- PendingTool 在三个宿主上仍未开启（`allow_deferred=false`），libopendan 遇到 PendingTool 快照直接 RecoveryBlocked；相关 Turn 规则只在 llm_context 层由测试覆盖。
- D1 中“排队的新请求等下一 Turn”没有实现成调度策略：Turn 打开期间（例如 WAIT_USER_MSG 未交付答复、可重试错误后）到达的 msg 会并入当前 Turn。
- xllm 接手执行的 Round 不进入 Session 的 `static.json`；进程被 kill 时尚未处理 Outcome 的 Round 增量会丢失（不会重复计数）。
- Sub AgentSession / CreateSubContext helper 仍未提供，父子 Session 的 Turn 各自独立计数。
- 既有行为（未改）：behavior 的策略拒绝会先为解析出的 Step、再为合成纠错 Step 各分配一次 `step_index`，编号出现空洞；flush 按身份水位处理，不受影响。xllm 自己执行时，上下文压缩的摘要请求经同一 client 计入 `usage.llm_requests`，而 libopendan 的摘要不计入，两边口径不同。
- 文档同步中发现、未在本轮处理的问题：`doc/opendan/Agent Context Messages.md` 仍是旧“大 Round”定义（旧 opendan 设计，第二阶段处理）；`LLM Understand Media.md` 大部分仍基于已不存在的 `OneShotRequest` / `drive_to_terminal` 设计名，本轮只修正持久化、归属与崩溃段落；`agent_tool` 的 pending → `check_task` 流程在 `allow_deferred=false` 的宿主里实际以 Internal 错误结束；fork 子 run 会同时在 `<session_history>` 和 `<<step_history>>` 看到父 run 的 step；普通切换的交接批次进入 `request.input`，渲染在 `<<step_history>>` 之前；independent 下 `END` 直接结束 Turn，report 归属仍待定。
- 第二阶段（按新抽象重构 opendan）另立任务。
