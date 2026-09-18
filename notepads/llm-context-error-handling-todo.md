# LLMContext 异常处理完善 TODO

日期：2026-09-18

状态：已实现（2026-09-18，见第 11 节实施记录）

用途：原为交给 CodeAgent 的实施清单；第 2 节"当前实现基线"描述的是实施前的状态，第 11 节记录实际改动、验证与剩余风险。

## 1. 目标与范围

围绕三类故障完善 `src/frame/llm_context` 及其直接调用方：

1. **推理故障**：Provider 请求失败、超时、鉴权或模型配置错误，以及推理完成后的输出解析失败。
2. **执行故障**：工具／Action 参数错误、业务执行失败，以及执行基础设施故障。
3. **Runtime 故障**：快照读写、状态提交、worklog、hook 等运行支撑设施失败。

核心目标是明确“谁能恢复、谁负责恢复、失败后能否继续产生副作用”，避免把基础设施故障一律反馈给 LLM，也避免关键状态未保存时仍无条件推进。

实施约束：

- 保持 scheduler/provider 中立；重试退避、模型路由、磁盘路径、session 状态等具体策略留在 adapter 或上层 Runtime。
- 以实施时的仓库源码为准，先复核本文列出的现状；已有修复直接复用，不重复实现。
- 遵循仓库 beta 2.2 规则，不为旧接口、旧快照添加兼容层；公共类型修改必须同步直接调用方、序列化和文档。
- 优先复用现有错误类型、Outcome、快照和存储接口。引入新依赖或通用组件前，遵循 `AGENTS.md` 的确认要求。
- 本次不扩展 deferred tool、上下文压缩、通用事务或全局调度能力；不承诺工具副作用的 exactly-once。

## 2. 当前实现基线

| 位置 | 当前行为 | 待解决问题 |
|---|---|---|
| `classify()` | 仅 `SnapshotCorrupted`、非 interrupt 路径的 `Cancelled` 默认 Fatal；其余默认 Recoverable | Provider 永久错误、adapter 返回的 Internal 也可能进入 LLM 自纠正循环 |
| 推理错误分支 | 记录 `LLMInferenceFailed`，追加 User 错误消息，然后继续推理 | adapter 容错结束后，循环仍会再次调用 Provider；没有这一层的退避，职责不清晰 |
| 输出解析 | Behavior parser 失败生成错误 step；严格 JSON 解析失败直接返回 Error；非严格 JSON 失败返回 Text/Done | 相似输出错误处理不一致；文档中的 schema 校验承诺需要与实际实现核对 |
| ToolManager | 直接返回 Observation；OpenDAN/OneShot adapter 将底层工具管理器 Err 转为 Observation::Error | 业务执行错误与执行基础设施故障缺少独立通道 |
| 工具批次 | 传统 tool loop 在未超错误上限时继续剩余调用；Behavior Actions 在首个错误后停止 | 两种语义需要明确，尤其是部分副作用和未执行项的记录 |
| ErrorPolicy | 仅有 `max_consecutive_errors`，默认 3；计数 `> 3` 才终止；0 禁用上限 | 计数粒度、清零时机、内外层循环关系需要明确 |
| TurnHook / WorklogSink | 接口不返回错误；TurnHook 契约要求不能 panic | 核心循环无法感知关键 checkpoint 保存失败 |
| OpenDAN 快照/worklog | 写入失败后 warning 并继续 | 关键持久化与普通日志采用近似相同的失败策略 |
| OneShot 持久化 | 轮前 hook 忽略错误；outcome 边界用 `?` 返回 LocalLLMContextError | 同一运行的持久化错误语义不一致；边界写入失败可能遮蔽已计算出的 outcome |
| StepResultHook | 返回 Err 后 warning 并继续 | 需要明确哪些 hook 允许降级，哪些失败必须阻止推进 |
| OpenDAN outcome 分发 | 所有 Outcome::Error 都进入 `on_provider_failed` 分支 | 工具错误、解析错误、Runtime 故障可能被错误地当成 Provider 故障处理 |

关键入口：

- [核心循环与错误分类](../src/frame/llm_context/src/context_loop.rs)：`run_inner`、`run_behavior`、`handle_error`、`bump_consecutive_errors`、`classify`。
- [错误类型](../src/frame/llm_context/src/error.rs)、[策略](../src/frame/llm_context/src/request.rs)、[依赖接口](../src/frame/llm_context/src/deps.rs)。
- [Observation](../src/frame/llm_context/src/observation.rs)、[Outcome/ResumeFill](../src/frame/llm_context/src/outcome.rs)、[状态与快照](../src/frame/llm_context/src/state.rs)。
- [OpenDAN adapter、worklog、快照 hook](../src/frame/opendan/src/ai_runtime.rs)。
- [OpenDAN session](../src/frame/opendan/src/agent_session.rs)：`persist_snapshot_to`、`handle_outcome`、快照加载与恢复入口。
- [OneShot Runtime](../src/frame/agent_tool/src/local_llm_context.rs)：`step`、`SnapshotPersistingTurnHook`、`SnapshotStore`、`LocalLLMContextError`。
- [OneShot Provider adapter](../src/frame/agent_tool/src/run_local_llm.rs)。

## 3. 目标处理矩阵

下表是本 TODO 的建议实施基线。具体类型和函数命名由实现者按最小改动确定，但必须保留这些语义区别。

| 故障 | 主要恢复责任方 | 是否反馈给 LLM | 核心循环行为 |
|---|---|---|---|
| Provider 临时故障、网络错误、超时 | adapter；必要时由上层调度器决定后续尝试 | 否 | adapter 有界容错耗尽后结束本次 run，保留结构化原因，不在循环外再次隐式重试 |
| Provider 鉴权、模型不存在、非法配置 | 配置管理／上层 Runtime | 否 | 直接结束本次 run，不消耗 LLM 自纠正次数 |
| LLM 输出不符合声明协议 | LLM 自纠正 | 是 | 保存失败输出和可操作诊断，受自纠正次数与运行预算限制 |
| 工具参数、业务执行失败、Policy 拒绝 | LLM 调整计划；必要时上层介入 | 是 | 记录关联调用的 observation，再推理；超过上限结束 |
| 工具调度、执行设施故障，执行结果未知 | 工具 adapter／Runtime | 不作为可自纠正错误直接反馈 | 停止继续派发；保留已知结果与不确定状态，由上层处理 |
| 关键 checkpoint／状态提交失败 | 持久化层／Runtime | 否 | 停止下一次推理及后续副作用；向调用方报告保存阶段和失败原因 |
| 普通 worklog／诊断日志失败 | 日志实现 | 否 | 降级并继续；保留可用的独立诊断途径，不改写业务 outcome |
| 快照损坏、恢复输入不匹配 | Runtime／调用方 | 否 | 拒绝恢复，明确报告原因 |
| 编程错误／状态不变量损坏 | Runtime／开发者 | 否 | 不进入自纠正循环；终止相关运行并保留诊断 |
| 主动 interrupt | 调度器 | 不作为故障反馈 | 保持 Interrupted + 快照语义，不增加错误计数 |

这里的“结束本次 run”不代表故障永久不可恢复，也不默认允许重放已有副作用。基础设施是否可重试，与是否能让 LLM 自纠正是两个不同维度。

## 4. P0：明确错误模型和分发边界

- [x] 去掉 `classify()` 对未知类别一律 Recoverable 的兜底，采用穷尽分类；新增错误种类必须显式确定处理方式。
- [x] 区分错误来源、是否适合 LLM 自纠正、是否允许基础设施重试；使用现有类型或最小必要扩展，不只根据 message 文本猜测。
- [x] 明确 `Internal` 的语义：不得因为来自 adapter 就变成可由 LLM 修复的错误。
- [x] Provider adapter 保留可获得的错误码和原因，至少区分临时故障、永久配置/鉴权错误、取消。未知故障不得自动等同于“安全重试”。
- [x] 明确 Runtime 错误的承载方式：复用带来源的 Outcome::Error，或在必要边界返回独立 Result。避免同一失败在多层被转换成不同类别。
- [x] 工具边界为基础设施故障提供独立表达，例如 `Result<Observation, RuntimeError>`，或等价的最小类型扩展；业务失败仍保留为 Observation::Error。
- [x] 对可能已产生副作用但无法确认结果的执行失败，保留“不确定”语义，不伪装成普通参数错误，也不自动重放。
- [x] 保留 task/call/trace 标识及必要诊断；面向 LLM 的错误说明与 Runtime 内部诊断分开，避免把凭证等内部信息带入 prompt。

验收：对各错误来源做类型和序列化测试；相同类型从不同入口进入时，不能因隐式默认分支获得相反的处理策略。

## 5. P0：收敛推理与输出自纠正流程

- [x] Provider 错误经过 adapter 容错耗尽后，核心循环不再追加 `error: provider...` 并自动发起下一次推理。
- [x] 核查 OpenDAN、OneShot 的 adapter 和实际 Provider 调用链，确认已有 retry/fallback 的实现与预算；未实现的配置不得仅凭注释宣称已支持。
- [x] 保持 retry/backoff/fallback 的实现位于 adapter；上层如需重启运行，必须是显式决策，不通过伪造 LLM observation 触发。
- [x] 严格 JSON 与 Behavior parser 的输出协议错误共享有界自纠正语义，提供失败输出及解析诊断；非严格 JSON 保留允许返回原始文本的明确语义。
- [x] 核对 `OutputSpec` 中 schema 的执行责任。若声明必须校验，则落实校验；若仅传给 Provider，则同步收紧契约，不能把 JSON parse 成功宣称为 schema 校验成功。
- [x] 统一自纠正计数规则：默认 3 表示最多提供 3 次错误反馈，连续第 4 次失败终止；记录选定的计数单位与 0 禁用上限的风险。
- [x] 明确同轮多个工具错误如何计数；成功的 Provider HTTP 请求不能清除随后解析/执行失败的连续计数。成功完成相应逻辑轮后才按约定清零。
- [x] 检查 Behavior 内外循环的新实例、usage 合并及计数传播，确保重新创建 inner context 不会绕过自纠正或预算上限。
- [x] 保持 interrupt 独立于错误处理；增加对“主动取消”与“无 interrupt 信号的 Provider Cancelled”的区分测试。

验收：持续失败的 Provider 不在 waist 层触发第 2 次 infer；格式错误可被下一轮修正；持续格式错误按约定上限终止；无无限循环、错误计数提前清零或预算重置。

## 6. P0：执行失败与部分结果

- [x] 工具参数/业务错误保留 `call_id`、结构化结果和适合 LLM 修正的说明；Policy 拒绝继续支持有界自纠正。
- [x] 基础设施失败立即阻止剩余调用派发，不复用普通 Observation::Error 的继续执行路径。
- [x] 明确传统 tool loop 与 Behavior Action 的批次策略。优先保留当前“传统工具可继续、Action 首错停止”的区别，写入接口说明和测试，避免无意改变执行顺序。
- [x] 保存每批调用中已成功、已失败、未执行及结果未知项；终止时仍可审计已经发生的副作用。
- [x] 可恢复流程继续推理前，检查消息中的 tool-call/tool-result 配对；中途终止留下的未完成调用不能被误认为可直接恢复的完整对话。
- [x] 不添加通用回滚或自动重放机制。已有工具幂等能力由 adapter 复用；无法安全重试的调用明确交给上层处置。

验收：A 成功、B 失败、C 尚未执行时，Action 模式不执行 C，A/B 结果可读；基础设施故障不会继续派发 C；失败后的恢复不会默认再执行 A。

## 7. P0：关键持久化失败必须可见并阻止推进

- [x] 让关键轮前 checkpoint hook 可以报告失败，例如将 `before_inference` 改为返回 Result；更新 OpenDAN、OneShot 和测试中的所有实现。
- [x] 没有配置持久化 hook 的纯内存运行仍合法；一旦该 hook 承担关键 checkpoint，写入失败就不能继续 infer。
- [x] OpenDAN 的 outcome 边界保存函数返回明确结果，由调用方处理；禁止保存失败后仍无条件切换 behavior、派发后续工作或宣告持久化完成。
- [x] OneShot 移除轮前写入的静默吞错；轮前和 outcome 边界采用一致的关键持久化失败策略。
- [x] 区分“计算得到 outcome”与“outcome 已持久化提交”。边界写入失败时保留已计算的 outcome、当前内存快照及保存失败原因，不能简单丢失这些信息。
- [x] 支持调用方只重试保存，避免为了补写结果再次执行 `ctx.run()`；特别检查 OneShot `step()` 中 `ctx.take()` 后的错误退出路径。
- [x] 检查快照、run 元信息、outcome 的写入顺序及部分提交场景；不能因先写 Completed、后写 outcome 失败就被误判成结果完整。
- [x] 定义关键持久化失败后的 Runtime 状态和恢复入口。若复用现有错误/暂停状态，应保证其不会自动触发副作用重放。
- [x] 快照缺失、读取 I/O 错误、反序列化损坏、ResumeFill 不匹配分别处理；禁止把应恢复的损坏快照静默降级为新运行。

验收：注入 mkdir/write/rename/store 失败后，Provider 调用数或后续工具调用数不增加；outcome 边界失败仍能取得已算结果；恢复存储后只补写数据，不重做推理和工具。

## 8. P1：日志、hook 与上层 Runtime 联动

- [x] 普通 worklog 保持 best effort；日志失败不进入 LLM 上下文，不增加自纠正计数，不覆盖原始 outcome。
- [x] 为日志失败提供独立、有限的 fallback 诊断，避免通过同一个失败 sink 递归记录自身错误。
- [x] 检查异步 sink 永久不返回的情况，在实现层提供有界等待或已有的隔离机制；不能让非关键日志无限阻塞推理开始或结果返回。
- [x] 若某类审计记录被定义为必须成功，明确使用关键提交语义，不继续借用普通 best-effort worklog 的契约。
- [x] 明确 StepResultHook 等扩展点的失败语义：允许降级的 hook 失败后继续；承担关键状态/输入提交职责的 hook 失败必须上报，不能一律 warning。
- [x] hook 不得 panic 的契约与错误返回分别处理；检查任务边界如何报告 panic。不要用全局 catch-unwind 将编程错误伪装成可恢复业务错误。
- [x] OpenDAN 根据错误来源分发，`on_provider_failed` 仅处理 Provider 故障；工具、输出协议、Runtime 故障使用各自明确的处置路径。
- [x] 明确 fallback behavior 是切换后等待事件，还是立即安排执行；实现、配置说明与测试保持一致，避免注释与实际 `NextAction` 不一致。
- [x] 关键 Runtime 故障保留诊断和恢复所需状态，不能走普通失败分支直接丢弃唯一可用快照。
- [x] OneShot/CLI 区分计算失败与 Runtime 保存失败，向调用方报告可机器判断的状态，并保留已得到的计算结果。
- [x] 为成功、失败、预算耗尽、主动中断建立正确的事件映射，避免将所有非 Done outcome 都展示为同一种推理失败。

## 9. P1：验证恢复边界，修正文档承诺

- [x] 保留并验证临时文件加 rename 的写入方式；明确原子替换与断电持久性的区别，按实际承诺决定是否需要同步文件/目录。
- [x] 检查 OneShot 轮前新增快照与 `latest_snapshot_idx`、恢复入口之间的关系，确保恢复能定位约定的最新已提交快照。
- [x] 对“推理成功后、工具执行前”“工具已执行、结果未保存”“快照成功、元信息失败”等窗口进行故障注入，记录实际恢复行为。
- [x] checkpoint 不能独自保证不重复扣费、不重复执行副作用。删除或修正代码注释和设计文档中的过强承诺；说明剩余窗口及 adapter 幂等性的责任。
- [x] 对 PendingTool、ContextLimitReached 等已有类型区分“协议已定义”与“当前循环已实现”，不得因存在枚举或恢复代码就宣称完整支持。
- [x] 修改序列化类型或外部结果字段时，搜索所有消费方并同步必要的 SDK/CLI/前端；没有实际影响的模块不扩改。
- [x] 同步 [LLM Context 设计](../doc/opendan/LLM%20Context%20设计.md)、[OneShot 协议](../doc/agent_tool/local_llm_context_protocol.md) 和受影响的错误/恢复接口注释。

## 10. 测试与完成标准

优先使用现有 mock、临时目录和可注入的 SnapshotStore/sink；不依赖真实 Provider 或破坏宿主磁盘来构造失败。

| 场景 | 必须验证的结果 |
|---|---|
| Provider 临时故障，adapter 容错耗尽 | waist 不重复 infer，不追加给 LLM 的基础设施错误消息 |
| 鉴权/模型配置错误、adapter Internal | 不进入自纠正路径，保留明确分类和原因 |
| Behavior/严格 JSON 输出错误后修正成功 | 失败诊断可见，成功收敛，usage 正确累计 |
| 连续输出/工具错误 | 第 4 次失败终止；多调用、内外循环的计数符合约定 |
| 部分工具成功后发生业务失败 | 批次策略正确、结果完整、无自动回滚或重复执行 |
| 工具基础设施故障/结果未知 | 停止后续派发，不误当普通参数错误 |
| 轮前 checkpoint 失败 | infer 次数为 0，错误可被 Runtime 接收 |
| outcome 已计算但保存失败 | outcome 与内存快照可取，只重试保存不会重复副作用 |
| 多文件状态部分写入/重启 | 不误判完整完成，不加载损坏或不匹配状态继续执行 |
| 普通日志失败或挂起 | 不改变业务结果、不增加错误计数、不无限阻塞 |
| 主动 interrupt 与恢复 | 保持 Interrupted 语义，不增加自纠正次数 |
| OpenDAN 非 Provider 错误 | 不误触发 provider fallback，不无条件丢弃恢复状态 |

- [x] 分阶段运行 `cargo test -p llm_context`，补充实际改动涉及的 `agent_tool`、`opendan` 定向测试。
- [x] 公共 trait/Outcome/序列化修改后，编译检查全部直接消费者；按仓库要求完成相关构建和测试。未运行或受环境限制的验证必须说明。
- [x] 更新与目标语义冲突的旧测试，例如当前验证 Provider 错误进入 User 消息的测试，不能为保留旧断言而继续保留错误行为。
- [x] 最终交付说明：错误分类与流程变化、修改入口、协议/快照影响、验证结果、仍无法保证的恢复窗口。

建议顺序：先完成第 4 节的类型与边界，再落实第 5–7 节主流程，随后处理第 8–9 节联动；第 10 节测试随各阶段执行。不能只调整错误枚举或更新文档，就宣称异常处理闭环完成。

## 11. 实施记录（2026-09-18）

### 改了什么

**llm_context（waist）**

- `error.rs`：`Provider { failure: Transient | Permanent | Unknown, message }`；新增 `ToolRuntime { tool, call_id, message, effect_unknown }`、`Checkpoint { stage, message }`；新增 `source()` / `llm_correctable()` / `infra_retry_safe()` 三个正交维度。`ErrorClass` 由 `llm_correctable()` 穷尽推导（`From<LLMComputeError>`），没有默认 Recoverable 兜底。序列化：`provider` 多出 `failure` 字段，新增 `tool_runtime` / `checkpoint` 两种 kind。
- `deps.rs`：`ToolManager::call_tool -> Result<Observation, ToolDispatchError>`（Err = 派发基础设施故障，带 `effect_unknown`）；`TurnHook::before_inference -> Result<(), String>`（Err ⇒ 不推理，`Error{Checkpoint{BeforeInference}}`）；新增 `WorkEvent::ToolDispatchFailed` / `CheckpointFailed`。
- `observation.rs`：`Observation::Unresolved { call_id, reason, effect_unknown }`（只由 waist 写入，用于批次中断后保持 transcript / StepRecord 配对）；`ToolExecRecord.ok` 改为 `status: Succeeded | Failed | Unknown | NotExecuted`（保留 `ok()` 方法）。
- `outcome.rs`：`Error { error, usage, trace }`，终止时仍能审计批次内每个调用的状态。
- `context_loop.rs`：
  - `infer()` 的任何错误直接结束 run，不再追加 `error: provider...` user 消息、不再自动发起下一次推理。
  - 严格 JSON 解析失败：失败输出留在 transcript，追加诊断 user 消息，走有界自纠正（与 Behavior parser 错误一致）；非严格保持原样返回 Text。
  - Policy 拒绝 / 超过 `max_calls_per_round`：推入 assistant 消息并对每个 call 配对一条错误 tool_result，再计数；不再留下未配对的 tool_use。
  - 计数：按逻辑轮计数，同轮多个工具错误只计 1；只有整轮无可纠正错误才清零；推理成功本身不清零。Behavior 模式下每步内层 ctx 继承外层 `usage` / `started_at_ms` / `consecutive_errors` 并回传，内层 `Done`/`Error` 的 trace 合并到外层。
  - 传统 loop：业务错误跑完整批次；派发 `Err` 立即停止，失败项记 `Unknown`/`NotExecuted`，剩余项配对为 `Unresolved`，以 `Error{ToolRuntime}` 结束。Behavior Action：首个业务错误停止，未执行项记为 `Unresolved{effect_unknown:false}` 并渲染为 "Not executed"；派发 `Err` 沉淀真实的部分 step 后以 `Error{ToolRuntime}` 结束。
  - `resume()` 对三种 fill 都校验 accumulated 中没有未配对的 tool_use，否则 `SnapshotCorrupted`。
  - Behavior 解析错误 / Policy 拒绝的合成 step 现在带失败的 assistant 输出。
  - `StepResultHook` 失败明确为可降级（trait 文档写明：hook 必须在能返回 Ok 前不提交状态）。
- `step_record.rs` 渲染 `Unresolved`；`request.rs` 文档收紧：`OutputSpec::Json.schema` 只透传 provider，waist 不做 schema 校验。

**opendan**

- `ai_runtime.rs`：kRPC 错误按变体映射为 provider failure 类别（`provider_error_from_rpc`），AICC `Failed`/解码失败 ⇒ Unknown，task Canceled ⇒ `Cancelled`；不再用 `Internal` 表示 provider 侧故障。`SessionSnapshotHook` 写失败返回 Err（`write_snapshot_file` 供 session 复用）；worklog 处理两个新事件；`OpendanToolAdapter` 返回 `Ok(Observation)`（`AgentToolError` 全部视为 LLM 可应对的业务错误）。
- `agent_session.rs`：`persist_snapshot(_to)` 返回 `Result`，所有关键提交点（Done 边界、PendingTool 派发前、Interrupted、compress、behavior switch / fork / independent / process pop）失败即 `?` 上抛，不再切换 behavior、派发任务或宣告已持久化；`try_load_snapshot*` 对损坏 / 不可读快照返回 Err，不再静默当成新 run（prompt 渲染路径仍降级）。`handle_outcome` 的 Error 分支按 `error.source()` 分发：Provider ⇒ `on_provider_failed`；Runtime ⇒ 保留快照、上报、等待；LlmOutput/Tool/Snapshot/Internal ⇒ 上报并丢弃快照。
- `round_history.rs`：渲染 `Unresolved`。

**agent_tool（OneShot / CLI）**

- `local_llm_context.rs`：`RunMetaState` 与轮前 hook 共享；轮前 checkpoint = 写 snapshot + 提交索引到 state.json，失败 ⇒ waist 不推理。`step()` 区分：Runtime 来源错误 ⇒ `Err(RuntimeFailure)` 并保留内存上下文（再次 `step()` 只重做 checkpoint）；outcome 提交分三阶段 snapshot → final.json → state.json，失败 ⇒ `Err(CommitFailed{stage})`，`pending_outcome()` 可读，`retry_commit()` 只补写。`resume_or_new` 识别 "final.json 已写、state 仍 Running" 的半提交并补齐，不重跑。新增 `SnapshotMissing`。
- `run_local_llm.rs`：kRPC 错误映射同 opendan；退出码 3 = outcome 已输出但提交失败，4 = 运行时故障（run 可 resume）。
- 其它 `ToolManager` 实现（`llm_understand_media`、`llm_compress` 测试桩）与 `Error` 模式匹配同步。

**文档**：`doc/opendan/LLM Context 设计.md` §1.3/§3.5/§3.6/§3.10/§9.2/§9.3/§10/§A.4；`doc/agent_tool/local_llm_context_protocol.md` §4.4/§6.3/§7.1/§7.2 trace/§7.3/§8.4/§9.4/§11/§13.2；`Agent配置改进.md` 与 `Agent RootFS.md` 的 `[on_provider_failed]` 触发范围。

### 为什么这样改

- Provider 故障与 LLM 自纠正是两个维度：adapter 已经做过容错，再把基础设施错误塞给 LLM 只会浪费推理并掩盖配置问题。
- 关键 checkpoint 失败继续推理会让"重启不重复扣费"成为假承诺；让 hook 返回错误并停在 s0，是最小改动。
- 把"算出 outcome"和"outcome 已提交"分开，才能在存储恢复后只补写数据。
- 批次中断时用 `Unresolved` 补齐配对，比留下未配对的 tool_use 更安全：transcript 始终可被 provider 接受，且"未执行 / 结果未知"对审计和 LLM 都是真实信息。

### 验证

- `cargo test -p llm_context`：151 通过（新增 17 个场景测试，覆盖第 10 节全部行：provider 故障不重推理、永久错误 / Internal / Cancelled / Timeout 终态且不计数、严格 JSON 自纠正与第 4 次终止、Behavior 解析错误反馈为 user 消息且第 4 次终止、同轮多错只计 1、推理成功不清零、干净轮清零、Policy 拒绝配对、派发故障停止批次 + 部分结果 + resume 不重放、Action 首错停止 + 未执行项、Action 派发故障沉淀部分 step、TurnHook 失败零推理 + resume、未配对 tool_use 拒绝恢复、内外层预算继承、interrupt 不计数）。
- `cargo test -p agent_tool --lib local_llm_context`：7 通过（轮前 checkpoint 失败零推理并可重试、outcome / snapshot 阶段提交失败 + retry 不重跑、轮前索引提交 + 中途崩溃后从工具轮之后恢复、provider 故障作为终态提交、半提交修复、快照缺失独立报错）。`cargo test -p agent_tool --lib` 中 `grep_tool` 的 3 个失败为环境缺少 `rg`，在改动前的基线上同样失败。
- `cargo test -p opendan --lib`：276 通过。`cargo check -p agent_tool_cli_dev -p agent-did-object-lib` 通过。
- 未运行：`buckyos-build`、DV Test；真实 AICC 下的 provider 错误分类只按 kRPC 变体映射，未在真实故障上验证。

### 仍无法保证 / 未做

- 工具副作用不是 exactly-once：TurnHook 写盘之后、工具执行完成之后到下一次 checkpoint 之间崩溃，恢复会重跑该段推理和工具；幂等性仍是 adapter 责任。
- Behavior 模式下 `TurnHook` 收到的是内层传统上下文的快照（不含 StepRecord 流），外层步骤状态只在 outcome 边界由 L4 提交；opendan 的 fork 依赖这一现状，本次未改。
- `AgentToolError` 没有区分基础设施故障，两处 adapter 目前把全部 manager 错误映射为业务错误；`ToolDispatchError` 通道已就绪但 agent_tool 层尚无生产者。
- `OutputSpec::Json.schema` 未做校验（需要新依赖，未引入），改为收紧契约并写入文档。
- OneShot 写入没有 fsync，只保证崩溃一致性，不保证断电持久性。
- `llm_explore` / `llm_understand_media` 两个 dev 工具对 `CommitFailed` / `RuntimeFailure` 仅按错误文本报告，没有像 `run_local_llm` 那样给独立退出码。
