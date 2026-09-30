# LLMContext X7 挂起与恢复 TODO

日期：2026-09-30

状态：已实施（2026-09-30），实施记录见第 9 节。第 2 节保留实施前的基线；未勾选的条目与第 9.4 节列出的是有意未完成的部分。

## 1. 目标与边界

补齐 `src/frame/llm_context` 的 `ContextLimitReached`、`PendingTool` 及其恢复闭环。当前 Outcome 和 ResumeFill 类型已经定义，但核心循环尚不产出这两个挂起结果；只补枚举分支或上层处理分支不能视为完成。

核心约束是保持协议级实现小而清晰：`llm_context` 只负责检测、挂起、保存必要机器状态和校验恢复输入。上下文压缩策略、异步任务调度、消息队列、Agent State、session receipt 和历史持久化由宿主负责。

- 一次正常运行中的历史保持 append-only；历史重写只能发生在显式交还控制权后的恢复边界。
- Snapshot 是执行恢复状态，session worklog 是长期历史；不能要求 session 根据可重写数组的下标解释永久历史。
- 优先复用现有 Outcome、ResumeFill、Tokenizer、CheckpointHook 和错误分类；只增加恢复必需的状态，不引入通用调度框架或存储服务依赖。
- 遵循仓库 beta 2.2 规则，不增加旧接口／旧快照兼容层；若改变持久状态语义，应明确版本变化、拒绝不支持的快照，并同步调用方和文档。
- 实施前重新核对源码。既有 TODO 中的“已接入上层分支”不能作为 waist 已支持的证据。

相关文档：

- [Agent Session SDK 实现计划](<../doc/opendan/Agent Session SDK 实现计划.md>)：X7、run 中途压缩与恢复。
- [LLM Context 设计](<../doc/opendan/LLM Context 设计.md>)：挂起、ResumeFill 和职责边界。
- [append-only history 原则](llm_context_append_only_history.md)。
- [异常处理 TODO](llm-context-error-handling-todo.md)：沿用错误归属、关键 checkpoint 和副作用处理原则。
- [Thinking block TODO](llm-context-thinking-block-todo.md)：历史前缀改写的联动事项；具体处理复用其实施结果，本文不重订 provider 规则。

## 2. 当前实现基线

| 位置 | 当前行为 | 缺口 |
|---|---|---|
| `request.rs::BudgetSpec` | 定义 `context_yield_threshold` | 核心循环未使用该阈值产出 `ContextLimitReached` |
| `context_loop.rs` 传统工具循环 | 收到 `Observation::Pending` 转成 Internal 错误，`allow_deferred=true` 也一样 | 没有 deferred tool 挂起路径 |
| `run_behavior` | Pending action 转成 Internal 错误 | 没有 action 执行中间态的挂起／回填闭环 |
| `run_inner_for_step` | 内层 PendingTool／ContextLimitReached 转成 Internal 错误 | 内层执行位置和外层 behavior 状态无法一起恢复 |
| `LLMContext::resume` | ToolResults 分支按顺序 zip pending 与 results；ResumeFromMidRun 拒绝非空 pending | 已有局部校验，但没有完整挂起原因、批次进度和恢复匹配契约 |
| `ResumeFill::RewrittenHistory` | 直接替换 `state.accumulated` | 没有完整的重写合法性校验；behavior prompt 还来自 steps 等状态，仅换 accumulated 不足以完成 behavior 压缩 |
| `lib_opendan::commit_round` | 两种 outcome 均保存 snapshot 并暂停 run；context limit 另记录错误并返回 | 没有压缩恢复和工具结果回填闭环 |
| `lib_opendan::resume_live_run` | pending 非空时返回 RecoveryBlocked；其余使用 ResumeFromMidRun | 不能认为“下一条输入来了就能恢复 PendingTool” |
| `lib_opendan` receipt／flush | 使用 accumulated 下标、输入前缀长度及 flush 游标定位历史 | 中途重写后这些位置可能失效，需单独协调协议 |

当前 lib_opendan 的 worklog 摘要发生在构造新 context 前或 run 结束后；本项讨论的是尚未接通的 run 中途重写，不是宣称当前压缩已经造成历史损坏。

源码入口：

- [context_loop.rs](../src/frame/llm_context/src/context_loop.rs)、[request.rs](../src/frame/llm_context/src/request.rs)、[outcome.rs](../src/frame/llm_context/src/outcome.rs)。
- [state.rs](../src/frame/llm_context/src/state.rs)、[observation.rs](../src/frame/llm_context/src/observation.rs)、[deps.rs](../src/frame/llm_context/src/deps.rs)、[error.rs](../src/frame/llm_context/src/error.rs)。
- [Runner](../src/frame/lib_opendan/src/runner/drive.rs)、[receipt](../src/frame/lib_opendan/src/runner/receipts.rs)、[history flush](../src/frame/lib_opendan/src/runner/flush.rs)。

## 3. P0：先确定最小挂起与恢复契约

- [x] 明确各挂起点需要保存的最小状态：挂起原因、待回填 call、当前批次执行位置，以及 behavior 内层／action 的必要续执行状态。具体字段以最小可实现方案确定，不把队列、task_manager 或宿主 receipt 写进 waist 类型。
- [x] 明确 Outcome、snapshot 和 ResumeFill 的匹配关系。PendingTool 只能通过约定的 ToolResults 解除；ContextLimitReached 的重写恢复与普通中途恢复必须可区分；不匹配时在推理或执行工具前拒绝。
- [x] 挂起后禁止在未完成恢复前直接再次 `run()` 推进；公开 API 应显式校验，避免调用方误把暂停当作普通轮次结束。
- [x] 明确挂起与预算终止、主动 interrupt、Provider 故障的区别及相遇时的优先级。上下文窗口压力不能用累计 token 花费预算代替。
- [x] 约定暂停期间 wallclock 的计费方式；usage、rounds_left、错误计数、step/action/call 身份不得因恢复或重建 inner context 意外重置。
- [x] 挂起 snapshot 必须包含已获得的输出和已执行工具结果；序列化再恢复后不依赖旧对象、闭包或进程内句柄。
- [x] 明确宿主持久化责任和 checkpoint 时序。复用现有 hook 语义；关键保存失败不得被挂起结果掩盖，也不得继续推理或派发副作用。waist 不直接写磁盘。

## 4. P0：真正产出 ContextLimitReached

- [x] 实现 `AbsoluteTokens` 阈值检测，并约定 `>=` 边界、零值、异常值和未配置时的行为。
- [x] 实现 `Ratio` 阈值所需的有效上下文窗口来源。由调用方／provider adapter 提供确定的模型能力，waist 不查询 AICC、模型目录或环境变量；窗口未知时显式处理，不能静默忽略配置或硬编码模型窗口。
- [x] 基于实际待发送请求估算上下文占用，覆盖 system、历史、工具描述、结构化输出等；说明多模态估算限制和 completion 预留口径。复用现有 Tokenizer，区分估算值与 Provider 实际 usage。
- [x] 在定义清楚的安全边界检查阈值：首轮、工具结果追加后、宿主 injection 应用后，以及 behavior 实际 prompt 构造后。超限时返回可恢复的外层 snapshot，避免发送已知超限请求。
- [x] 明确 ContextLimitKind 中预警、本地已知硬限制、Provider 拒绝的映射。Provider 特定错误码在 adapter 归一化；waist 不通过异常文本或任意 HTTP 400 猜测 context limit。
- [x] 保持一般 Provider 错误的现有归属；只有结构化确认的上下文限制才走该挂起路径，不在 waist 内做重试、摘要、裁剪或换模型。
- [x] 若回填后的历史仍超限，再次明确 yield 或拒绝；不能在 waist 内反复重写／推理形成无进展循环。压缩次数和失败后策略留给宿主。

## 5. P0：真正产出 PendingTool 并支持结果回填

- [x] `allow_deferred=false` 时保持明确的协议违规处理；为 true 时，Observation::Pending 应产生 PendingTool，保留原始 call_id、调用内容和必要的等待信息。
- [x] 明确同一批次混合成功、失败、Pending 时的顺序。优先复用现有串行执行，不顺便引入并行工具调度；已执行、正在等待、尚未执行三类调用必须可区分。
- [x] 保存未执行的批次尾部或明确的处置结果；resume 后不能丢掉剩余调用，不能重新执行已经成功或已返回 Pending 的工具。
- [x] ToolResults 按 call_id 校验集合，拒绝缺失、多余、重复、未知 ID 及仍为 Pending 的“终态结果”；若接受乱序结果，按原调用顺序确定性地写回。第一版可以要求全量回填，部分完成由宿主收集，不扩展多次增量填充协议。
- [x] 合法的 Success／Error／Cancelled／Unresolved 回填沿用现有 observation 与错误归属语义，保持 tool_use／tool_result 配对；Pending 不伪装成成功或未知执行失败。
- [x] 回填、清除 pending、恢复批次执行位置必须形成一致的可恢复状态；结果不会重复追加，usage 和执行记录不会重复计入。
- [x] 超时、取消异步任务、任务持久化和结果投递由宿主处理；不向 llm_context 引入 task_manager、InputChannelFactory 或 AgentStateClient。

## 6. P0：behavior 模式与显式历史重写

- [x] 分别完成 behavior action Pending 和 inner traditional context Pending 的恢复路径。若分阶段交付，明确能力范围，在不支持的调用发生副作用前拒绝；不能把只支持传统模式标为 X7 全部完成。
- [x] 移除 `run_inner_for_step` 对 cooperative yield 一律转 Internal 的处理。保存足够的 inner continuation 和外层位置，避免恢复后重新运行已完成的 native tool、重复解析响应或重复派发 action。
- [x] behavior 的 ContextLimitReached 需要反映实际物化 prompt 的占用，同时交付能恢复 behavior 的状态；不能把临时扁平化 inner snapshot 当作完整外层 snapshot。
- [x] 明确 behavior 重写入口如何作用于 request.input、steps、last_step、history_summaries、history_inputs。只修改 accumulated 不算支持 behavior 压缩；优先复用现有 snapshot／history 表达，避免另外维护一套互相漂移的历史。
- [x] 重写的内容由宿主提供，waist 只验证和恢复；运行中的 renderer／loop 不因 recency 或预算自动改写已提交的 prefix。
- [x] 校验重写后的调用配对、保留状态与编号；未完成工具调用及恢复必需记录不能被摘要吞掉。对 PendingTool 状态的重写应明确拒绝，除非协议已定义并验证组合语义。
- [x] 与 Thinking block TODO 的前缀改写处理衔接，复用已有 helper；确保普通 append-only 运行仍原样保留消息中合法的结构化内容。

## 7. 宿主联动与历史协议约束

本节是接入 X7 的必要联动，不要求把 lib_opendan 的历史逻辑放进 llm_context，也不扩大为整个 AgentSession 的重构。

- [x] 复核 `agent_tool::local_llm_context`、`opendan::AgentSession`、`lib_opendan::SessionRunner` 的所有 Outcome／ResumeFill 分支。阈值开始生效后，已有配置会触发过去未执行的路径；支持范围必须逐一确认。
- [x] 对尚未完成恢复的宿主保留明确的保存并暂停／报错行为，不丢 run、不盲目重新执行；具备能力的宿主分别接入压缩回填和异步结果回填。（压缩回填：xllm、libopendan 已接入；异步结果回填：没有宿主启用 `allow_deferred`，见 9.4）
- [x] 单独确定 lib_opendan 的中途重写协议：现有 receipt 的 `message_pos` 和 flush 游标依赖数组布局；`input_seq`／输入身份不等于数组下标，不能一起重置或丢弃。
- [x] 在启用 lib_opendan 中途压缩前，验证“旧历史先完整提交、改写边界可恢复、后续新增历史准确定位”的方案。评估稳定记录标识或显式重写边界等最小方案，不在本 TODO 中预设新的通用日志系统。
- [x] `LLMContextState.host` 继续作为不透明宿主元数据保存；waist 不解析或修补 `host["libopendan"]`。位置有效范围及重写通知通过明确接口契约表达。
- [x] 同步快照版本、Outcome／ResumeFill 序列化、源码注释、LLM Context 设计和 SDK 实现计划。分别记录“waist 能力完成”和“各宿主接入完成”，避免笼统标记 X7 完成。

## 8. 验收测试

先用 scripted LlmClient、fake ToolManager 和 checkpoint hook 验证 waist，不依赖真实模型、kmsg 或 task_manager。

| 场景 | 必须验证的结果 |
|---|---|
| 阈值以下／恰好达到／超过阈值 | 推理次数正确，ContextLimitKind 正确；首轮就超限也能 yield |
| Ratio 窗口未知、非法比例、阈值未配置 | 按明确契约处理，没有静默失效或默认无限循环 |
| 工具结果或 injection 使请求超限 | 新增内容已进入可恢复 snapshot，下一次 infer 未发生 |
| Provider 拒绝上下文与普通 Provider 错误 | 前者结构化映射，后者保持原有错误归属 |
| 压缩后仍超限／压缩成功 | 前者有界交还控制权，后者从新历史继续且预算不重置 |
| Pending 开关、混合调用批次 | 正确挂起，已执行工具调用次数不增加，未执行项不丢失 |
| 错误／缺失／多余／重复／乱序回填 | 按 call_id 契约处理；非法 fill 在推理和副作用之前失败 |
| Pending snapshot 配 ResumeFromMidRun／RewrittenHistory | 拒绝不匹配恢复，不能绕过等待 |
| JSON snapshot round-trip 与两次连续挂起恢复 | 状态完整、编号连续、结果不重复、host 元数据保留 |
| behavior action／inner loop 挂起 | 外层位置和内层进度完整保留，工具、action、step 均不重复 |
| behavior 历史重写 | 实际发送 prompt 缩减，普通运行 prefix 保持稳定，结构化内容处理符合约定 |
| checkpoint 失败与 interrupt 交错 | 不启动后续副作用，保留原有错误和中断语义 |
| lib_opendan 压缩接入测试 | 旧消息不丢失／串轮／重复；receipt、消费确认和 flush 游标一致 |

- [x] 增加能从实际 `ctx.run()` 产出两个 Outcome 的测试；不能只构造枚举来测试上层 match。
- [x] 针对改变的公共类型编译所有直接调用方，并运行其相关回归测试。
- [x] 宿主接入中途重写时补充崩溃窗口测试：压缩前历史提交、重写 snapshot 发布、session 状态提交之间任一点退出，恢复不得错认输入消费或重放副作用。

在 `buckyos/src` 下执行以下检查；这些是未来实施时的验收命令，本文新增时未运行：

```bash
cargo test -p llm_context -- --test-threads=1
cargo check -p agent_tool -p opendan -p libopendan
cargo test -p libopendan -- --test-threads=1
```

完成标准：两个 Outcome 能真实触发并经序列化后恢复；传统和 behavior 的支持范围均有实际执行测试；核心不承担压缩／调度策略；宿主历史协议的未完成项单独列明，不用“类型已有”或“暂停分支已有”代替验收。

## 9. 实施记录（2026-09-30）

### 9.1 waist（`src/frame/llm_context`）

- 状态：`LLMContextState.pending_tool_calls` 由三项取代，快照版本升到 2（0 / 1 仍按未挂起快照恢复，大于 2 拒绝）：
  - `suspended: Option<Suspension>`：`PendingTool { pending, at_ms }` / `ContextLimit { which, estimated_tokens, at_ms }`；
  - `tool_batch: Option<ToolBatch { remaining, round_error }>`：被 deferred 调用截断的批次（传统 loop，或 behavior step 的内层原生工具）；
  - `action_step: Option<ActionStep { step, response }>`：behavior 派发中的 step（尚未沉淀）。
- Fill 与挂起一一对应（`suspension.rs::apply_fill`，全部在 `resume()` 内、推理与副作用之前校验）：`ToolResults` ↔ PendingTool；`RewrittenHistory`（function call，替换 `request.input` 与 `accumulated`）/ 新增 `RewrittenSteps`（behavior，替换 input / history_summaries / steps / last_step，只能保留原 step；编号、`history_inputs`、进行中的 turn 保留）↔ ContextLimit；`ResumeFromMidRun` ↔ 未挂起（可带批次 / step 续派）。不匹配、缺失 / 多余 / 重复 / 未知 ID、`Pending` 当终态、观察 call_id 不一致、重写后配对不完整（含孤立 tool_result）一律 `SnapshotCorrupted`。重写统一丢弃 thinking（`strip_thinking` / `is_thinking`，Thinking TODO 3.1 的消息级 helper 与 3.3）。
- `run()` 在挂起态直接返回 `Error{Internal}` 且不改状态；`inject` 在挂起或续派未完成时不放置（`InjectionPosition::None`）。
- PendingTool：串行派发，遇 `Pending` 停止；其后调用留在 `tool_batch.remaining` / step 其后的 action，回填后续派，整批完成才计一次轮数与错误。回填 `Cancelled` 视为宿主收尾，同批次未派发调用记为 not executed。behavior action 在第一个非成功结果后停止其余 action。`PendingToolCall` 增加 `tool_result`（等待信息），`ToolExecStatus` 增加 `pending` / `cancelled`；两个挂起 outcome 增加 `trace`（本段运行审计，llm_task_ids 不再重复计入）。
- ContextLimitReached：`BudgetSpec.context_window_tokens`（调用方提供）；`AbsoluteTokens`（>0）/ `Ratio`（(0,1]，需窗口）按 `>=` 让出 `ApproachingWindow`；估算 + `model_policy.max_completion_tokens` 超过窗口 → `HardLimit`；`ProviderFailure::ContextLimit`（adapter 结构化归一化）→ `ProviderRefused`。非法配置在推理前 `Error{Internal}`。检查位置：checkpoint hook、TurnHook、abort 检查之后，请求发送之前（首轮 / 工具结果后 / 注入后 / behavior 内层物化 prompt）；估算口径见 `context_window.rs`（每消息 4、非文本部分 1024、另计工具描述与 schema）。重写后仍超限再次让出、不推理。
- behavior：`run_inner_for_step` 不再把内层 yield 转 Internal；内层非 Done 时把本 step 的 turn（tool_use 与结果）作为 `accumulated` 在 `request.input` 之后的尾部保留在外层快照（Interrupted / Error 同样保留，恢复不重跑原生工具）。ContextLimit 的 `accumulated` 是物化 prompt 去掉进行中 turn；`LLMContext::rewritable_history(snapshot, deps)` 可从快照重建它。
- 挂起期间不计 wallclock：resume 时按 `at_ms` 平移 `started_at_ms`；usage / rounds_left / 错误计数 / 编号不重置。

### 9.2 宿主

| 宿主 | ContextLimitReached | PendingTool |
|---|---|---|
| xllm（`agent_tool::local_llm_context`） | `.llm_context` 新增 `context_window`（`RunLimits.context_window_tokens`），有窗口才设 Ratio 0.75；压缩后 function_call 用 `RewrittenHistory`、behavior 用 `RewrittenSteps`（物化历史折叠进 input），先提交压缩后快照再续跑，每 run 最多 3 次；`--resume` 遇到 ContextLimit 挂起快照先压缩；OpenAI 兼容 adapter 把 `error.code=context_length_exceeded` 归一化为 `ContextLimit`（不看文本） | `allow_deferred=false`；遇到仍 failed；resume 遇到 PendingTool 快照 `NotResumable` |
| libopendan | 中途重写：先 flush 本 run 历史并以 `outcome(context_rewritten)` 收尾（与 flush 标记同一次提交，无新历史不写），压缩 summary.json（保留原始记录 `history_budget >> 第几次`），以 system + 历史消息恢复同一 run，HostMeta 进入新 `history_epoch`（`epoch_round` / `epoch_input_seq`），先发布快照再推理；一次推进最多 3 次，仍超限则 paused + `last_error.kind=context_limit` 保留挂起快照，下次推进先重写。function call 的 flush 计数按 epoch（`LiveRun/ProcessFrame.flushed_epoch`），receipt 的 `input_seq` 与消费位置不变 | `allow_deferred=false`；恢复遇到 PendingTool 快照 → RecoveryBlocked |
| OpenDAN `AgentSession` | 不配置阈值 / 窗口，只可能来自 Provider 结构化拒绝（其 AICC adapter 目前不产生）；压缩循环改为按模式选 fill、先 resume 再持久化；启发式压缩不再留下孤立 tool_result | 休眠路径（task_dispatch）改用 `pending_calls()` / 清除挂起状态，未启用 |

协议文档与 fixtures：`Session Directory Protocol.md` §7 / §8 / §10、schema（host_meta / session_state / worklog_entry）、fixtures 全部重新生成（旧 fixture 快照带着无窗口的 Ratio，在新契约下是非法配置）。

### 9.3 验证（在 `buckyos/src` 下）

```bash
cargo test -p llm_context -- --test-threads=1        # 181（新增 suspension_tests.rs 20 个 + context_window 4 个）
cargo test -p agent_tool --lib -- --test-threads=1   # 212（新增 xllm 压缩 / 接手 / OpenAI 归一化 6 个）
cargo test -p libopendan -- --test-threads=1         # 81（新增 tests/context_limit.rs 3 个 + crash.rs 3 个崩溃窗口）
cargo test -p opendan --lib -- --test-threads=1      # 275
cargo check -p agent_tool_cli_dev -p agent-did-object-lib --tests
```

崩溃窗口（子进程故障注入）：`context_limit:after_flush` / `after_compact` / `after_publish`，恢复后工具不重跑、worklog 每条记录一次、只一个 `context_rewritten`。

### 9.4 未完成与风险

- **异步结果回填没有宿主接入**：waist 已支持，但 xllm / libopendan / OpenDAN 都保持 `allow_deferred=false`；task_mgr 结果回填（含超时、取消、任务持久化）仍属宿主待做。
- **AICC 未归一化上下文拒绝**：AICC 客户端错误没有结构化的 context-length 码，经 AICC 的请求只能靠已知窗口与阈值；只有 xllm 的 OpenAI 兼容 adapter 能产生 `ProviderRefused`。
- **窗口来源**：xllm 需要在 `.llm_context` 显式写 `context_window`，未读取 AICC 模型目录（`max_context_tokens`）；xllm PRD 未同步该配置项。
- **估算精度**：字节启发式 tokenizer 对中文偏低估，多模态按固定值计；阈值需要留余量。
- OpenDAN round history 的差量记录在中途重写后不可靠（按旧基线长度取差）；该路径目前不可达。
- 回填后到下一个 checkpoint 之间续派的调用在崩溃后可能重放（与普通批次相同的窗口，幂等属于 effect 层）；waist 要求宿主在回填后先持久化 `ctx.snapshot()`。
- Thinking TODO 3.2（overrides 触发剥离）与快照级 helper 未做。
