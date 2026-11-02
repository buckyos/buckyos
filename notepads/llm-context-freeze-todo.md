# llm_context 设计简化与冻结 TODO

日期：2026-11-01

状态：**§6 第 1–4 步已实施（2026-11-01，未提交），xllm 已在本机 DV Zone 的 AICC 上验收（§8.4）；agent-session 的重构与 DV 移到 [Agent Session 全异步 task 化 TODO](./opendan-agent-session-async-task-todo.md)（用户 11-01 裁决）；第 5 步（F11、F6）按计划留作后续。** F1–F24 全部已定。历史：2026-11-01 第二轮讨论补了 §2.4、§3.6–§3.8 和 F9–F11（用户描述意图，Claude 按方案回写，待用户 review）。2026-11-01 第三轮：**F2 裁决为 B（删掉挂起原语）**，补了原则 8、§3.2 的 B 方案细节和 F12；同日用户提出 runtime 预设想法，写成 §3.9 + F13（Claude 按用户口述回写，待 review）。2026-11-01 第四轮：Claude 对照源码 review，修正了三处事实（§2.4 两个钩子并非重复、§3.2.1 `subctx:` 不是子 session、opendan 的等待预算），补了"再删清单"（§3.10）、F9–F11 的陷阱、实施顺序（§6），新增 F14–F16 待裁决。2026-11-01 第六轮：用户倾向 **ask-user 一般化为标准异步 function call**（§3.2.1，F20）：问题是 Zone TaskMgr 上的 `human.input/v1` 任务，xllm 这种在调用内等的 CLI 也经 notify 系统拿到答案；F14 并入 F18，F7 提前为前置。2026-11-01 第七轮：F20 裁决一般化；按双中立扫描 waist 公开面写成 §3.11，**F21–F24 用户全部按建议裁决**（删无使用方模块、`Observation.host` 不透明、msg_parser / prompt_engine / xml_util 搬出、RefusalKind / ContextOwnerRef / report_artifacts 改型）。待裁决只剩 F15、F16。2026-11-01 第五轮：两种 loop 的结束状态与续接（原则 9、§3.2.2、F17–F19、§3.10 增补、原则 10 串行派发；Claude 按用户口述回写，待 review）。

背景：LLM Context 的设计准备冻结，并用另一种语言（TypeScript，见 [xllm TS 运行时 TODO](./xllm-ts-runtime-todo.md)）再实现一遍。冻结之前先把 llm_context 这一层削薄：冻结的东西越少，第二份实现越容易对齐，以后的改动也越少碰到破坏性变更。

## 1. 原则

1. **llm_context 越薄越好，只看 loop。** 同步还是异步、任务怎么跟进、怎么提示模型，都交给 Runtime 加提示词。llm_context 只负责六件事：推理、派发工具调用、把结果配对回历史、决定在哪里停、怎么接着跑、什么时候打 checkpoint。
2. **原生异步假设。** BuckyOS 是原生分布式系统，function call 默认就是异步的。同步只是"返回之前任务已经结束"的特例。
3. **task_id 和 checkpoint 相关。**（用户 2026-11-01）调用一个工具时如果能拿到 task_id，恢复时就有机会接着跟这个任务，不用再发起一次 function call。派发前的 checkpoint 就是打点位置。
4. **call_id 是幂等键。** 任务身份由 call_id 决定，派发之前就能算出来。恢复时按 call_id 找回任务；找不到的才按"结果未知"处理。任何情况下都不自动重发（沿用现有的"不重放"原则）。
5. **llm_context 只定格式和时机，不做 I/O。** 快照里有什么、什么时候打 checkpoint、怎么恢复，由 llm_context 定。存到哪里、怎么重建依赖（runtime、工具、provider），由宿主定。
6. **注意冻结的代价不对称。** 现在留下一个不带语义的小原语很便宜；冻结之后再加，就要改快照格式，属于破坏性变更。删东西之前先问一句：以后会不会要加回来。
7. 沿用已有原则：判断错误类别只看枚举，不做字符串匹配。
8. **loop 里不允许等待，任何等待都是任务。**（用户 2026-11-01）类比 Windows UI 的消息循环：循环里不许调同步阻塞函数，否则各种机制跟着崩。`DoModal` 在循环里再开一个小循环是公认的烂设计；对应到这里，就是工具在调用里等用户回答、等审批、等子 session（挂起原语 + 回填）。ask-user 也应该是基于任务的逻辑：问题发出去就是一个任务，答案到了是新输入；等待期间 session 能不能继续推理，完全由 session 自己决定，不是必须停。
9. **两种 loop 都要能跑。**（用户 2026-11-01）普通 loop（function call）和 behavior loop 共用一个 waist，任何设计都要写出两边运行起来的样子；只在一边成立的方案不算定稿。
10. **多个调用默认按返回顺序串行派发。**（用户 2026-11-01）不管是 function call 的 `tool_calls` 还是 behavior 的 `<actions>`，一次推理返回多个时，语义上通常都是并行友好的（参数都已备好），但从并行安全的角度，除非特别设计，LLMContext 一律按返回顺序一个一个调，前一个有结果才派下一个。理由是可稳定重跑：每次派发前有 checkpoint（F5），崩溃后快照里"已有结果的前缀 + 没结果的 `remaining`"是确定的，只要工具幂等（F12 的 `idempotent()`），从 checkpoint 重新进入 loop 就能得到同样的历史；并行派发则前缀不确定，回填和重放都没法对齐。现状已是如此（function call 批次 `remaining.remove(0)` 逐个派发，action 按文档顺序，遇业务错误即停），要做的是写进冻结规范（I7），TS 版照做。并行是以后的扩展，由 Runtime 的 ToolManager 在调用内自己并发（例如把一批调用转成任务立刻返回"仍在运行"），对 loop 仍是串行返回。

## 2. 现状（2026-11-01 源码核对）

### 2.1 llm_context 对"任务"知道多少

| 类别 | 现在的位置 | 用途 |
| --- | --- | --- |
| A. 任务模型与渲染 | [`llm_context/src/tasks.rs`](../src/frame/llm_context/src/tasks.rs)：`RunningTaskResolver`（:90）、`TaskState / TaskResult / TaskBrief / TaskFollowup`、`task_state_observation`（:199）、`render_background_env_via`、`MAX_IN_TOOL_WAIT_MS` / `DEFAULT_TASK_WAIT_MS` | llm_context 自己只在渲染后台任务提示时用到它；其余调用方都在 agent_tool（xllm、tasks、tasks_zone、task_bridge、runtime/node）和 lib_opendan（drive、live、children、bridge/task） |
| B. 后台任务跟踪 | `LLMContextDeps.tasks`；[`context_loop.rs`](../src/frame/llm_context/src/context_loop.rs) 的 `watch_task_of`（:1025，工具结果里带 `task_id` 就登记跟踪）、`render_background_env`（:1296，每次推理前渲染）、`build_inference_request`（:1272，作为一条 User 消息追加在请求末尾，不写进历史） | `<background_tasks>` 提示 |
| C. 挂起原语 | `Observation::Pending { task_id, until_ms }`（[observation.rs:98](../src/frame/llm_context/src/observation.rs)）、`ToolCallCtx.allow_deferred`、`Outcome::PendingTool`（[outcome.rs:156](../src/frame/llm_context/src/outcome.rs)）、快照中的 `Suspension::PendingTool`、`ResumeFill::ToolResults`（[suspension.rs:45](../src/frame/llm_context/src/suspension.rs)），以及检查"Pending 必须带 task_id、必须 `allow_deferred`"（context_loop.rs:970） | 工具调用的结果稍后再给。目前只有 libopendan 在用：它从 2026-10-03（`5ef122a2c`）起对所有 hosted run 都设了 `allow_deferred = true`（[live.rs:706](../src/frame/lib_opendan/src/runner/live.rs)）；`call_behavior`（`subctx:`）和 ask-user 也依赖它。xllm 一直是 `false`：在工具调用里等，最长 30 分钟，到点就返回"仍在运行" |

### 2.2 保存与恢复分在哪里

| 部分 | 负责方 | 位置 |
| --- | --- | --- |
| 快照内容 | llm_context | `LLMContextSnapshot { request, state }`（[state.rs:223](../src/frame/llm_context/src/state.rs)）。快照是自包含的：包括还没派发的调用 `tool_batch`、`action_step`、`suspended`，以及一个 `host` 字段（宿主数据原样携带，llm_context 不解读）。`SNAPSHOT_FORMAT_VERSION = 5`（:123） |
| 恢复语义 | llm_context | `snapshot()` / `resume(snapshot, ResumeFill)`（context_loop.rs:194 / :157），版本不对就拒绝 |
| checkpoint 时机 | llm_context 定调用点 | 每次工具调用前 `before_tool_call`（:722），每次推理前 `before_inference`；每个 Outcome 都带一份快照 |
| 写盘 | 宿主 | llm_context 的非测试源码里没有文件读写 |
| xllm | 宿主 | 只挂推理前的钩子 `SnapshotHook`（[xllm.rs:7986](../src/frame/agent_tool/src/xllm.rs)），通过 `RunStore::put_snapshot`（:6276）写 `snapshots/NNNN.json`：先写临时文件，fsync 后 rename。**工具调用前不存快照**；正在跑的调用由 ToolManager 记进 `run.json` 的 `inflight[]`（:13169） |
| libopendan | 宿主 | `SessionCheckpointHook`（[hook.rs:141](../src/frame/lib_opendan/src/runner/hook.rs)）在工具调用前和推理前都存快照，同时检查 lease、提交 session 状态。run 目录沿用 xllm 那套 |
| 重建依赖 | xllm 的 run 目录协议 | `run.json`（RunRecord v6）：工具来源、runtime 配置和 descriptor、inflight、宿主环境核对；`XllmRun::resume`（:8220）→ `rebuild_toolset`（:7583）→ `settle_previous_executor`（:8481）→ `fill_suspended_calls`（:8525）。快照上的注释说"持有等价 deps 的调度器都能在另一个节点恢复"，而"等价 deps 从哪来"是这份协议补上的 |

### 2.3 恢复时工具调用的三种情况

| 崩溃时这次调用的状态 | 现在怎么处理 |
| --- | --- |
| 已经返回 task_id，并且写进了快照（shell auto 模式的"仍在运行"结果，或挂起状态里的 Pending） | 接着跟：挂起的调用按任务当前状态回填，后台任务可以继续 `wait_task` |
| 已派发、还没返回（in-flight） | 不重发，但也接不上：转成 Unresolved，附上"重复之前先自己查"的说明。shell 会读执行目录里的退出码（[runtime/mod.rs:455](../src/frame/agent_tool/src/runtime/mod.rs)、`describe_interrupted` :718） |
| 工具本身没有任务概念 | 同上，Unresolved |

第二种情况其实接得上，因为任务身份本来就是从 call_id 推出来的：`local:shell:<call_id>`、`exec/<call_id>/`，以及 TaskMgr 上的 idempotency_key `shell:<run_id>:<call_id>`（[tasks_zone.rs:198](../src/frame/agent_tool/src/tasks_zone.rs)）。只是恢复流程没有用这些信息。

### 2.4 推理前的 checkpoint 与注入（2026-11-01 核对）

推理前有两个钩子，循环里的顺序见 [context_loop.rs:406-452](../src/frame/llm_context/src/context_loop.rs)：

| 顺序 | 钩子 | 看到什么 | 能做什么 | 谁在用 |
| --- | --- | --- | --- | --- |
| 1 | `CheckpointHook::before_inference`（异步，[deps.rs:411](../src/frame/llm_context/src/deps.rs)） | 外层快照 | 返回 `Some(Injection)` 就注入再重打一次，最多 `MAX_INJECTIONS_PER_BOUNDARY = 4` 次；`Err` 则不推理、以 `Checkpoint { BeforeInference }` 结束 | libopendan `SessionCheckpointHook`（[hook.rs:141](../src/frame/lib_opendan/src/runner/hook.rs)）：commit、lease、after_tools 的 background 观察 |
| 2 | `InferenceHook::before_inference`（同步，deps.rs:366） | s0 快照 | 只能拒绝 | xllm `SnapshotHook`（[xllm.rs:7986](../src/frame/agent_tool/src/xllm.rs)）写 `snapshots/NNNN.json` |
| 3 | `render_background_env` | — | 渲染任务提示，只进本次请求，不进快照 | 有 `LLMContextDeps.tasks` 时 |
| 4 | `build_inference_request` → 推理 | | | |

`Injection`（deps.rs:375）的放置规则在 [`inject`](../src/frame/llm_context/src/context_loop.rs)（:236）：function call 模式追加到 `accumulated`，位置在整批 tool result 之后；behavior 模式并进当前 step 的 `next_user_message`；第一步之前追加到 `request.input`；批次没配对完或处于挂起状态时拒绝放置。注入的消息进历史、进快照，恢复后模型看到的和崩溃前一致。

已经走这条路的内容：libopendan 的 `<<background_context>>`（[observe.rs](../src/frame/lib_opendan/src/runner/observe.rs)，`[input] background = "after_tools"`）、新输入回执（receipts）。

两点出入：

- 两个钩子**不是**简单重复（2026-11-01 第四轮修正）：`InferenceHook` 会进 behavior 模式的内层 per-step context，每次内层推理前都看到扁平快照；`CheckpointHook` 被 `into_traditional` 剥掉（[deps.rs:591-596](../src/frame/llm_context/src/deps.rs)），只在 step 边界调，内层只拿到 `tool_checkpoint_hook` 用于 `before_tool_call`。所以 xllm 今天在 behavior 模式下对 step 内的每次推理都打快照，libopendan 只在 step 边界打。合并的代价见 §3.6。
- xllm 没接 `CheckpointHook`（只有 `SnapshotHook`），所以 xllm 单独跑时没有任何注入入口，也不在工具调用前打 checkpoint（与 §2.2 一致）。

## 3. 想法（待裁决）

### 3.1 把任务概念移出 llm_context

- **I1** `tasks.rs` 整个搬到 agent_tool，成为 runtime 层的规范。工具内等待、挂起回填、CLI bridge 三处的文本必须一致，这条规范保留，只是挪到 agent_tool / runtime 的规范里，照样进入共享向量。
- **I2** `watch_task_of` 搬进 `XllmToolManager::call_tool`。它拿到 AgentToolResult 的时间比 loop 还早，可以直接登记。
- **I3** `<background_tasks>` 提示和 `LLMContextDeps.tasks` 一起删（F4 / F16）。原先的三个备选：包装 `LlmClient`、llm_context 提供通用的"只进本次请求、不进历史"钩子、干脆不要后台任务提示。包装 `LlmClient` 不可取：token 估算 `estimate_request` 在 llm_context 内对 `infer_req` 做，现在 `background_env` 是算进去的，包装后这段在 llm_context 外面追加，context limit 的 yield 判断会漏算。**2026-11-01 第八轮裁决（F16）：不要后台任务提示。** `<background_tasks>`、`background_env`、`LLMContextDeps.tasks` 一起删；B 之下"仍在运行"结果里已经带了任务 id，模型靠 `wait_task` / `get_task_state` 跟，opendan 由 task done 事件推进。

做完 I1–I3 后，llm_context 不再知道任务状态、等待、取消、渲染和后台跟踪这些概念。

### 3.2 挂起原语：留一个不带语义的，还是删掉（2026-11-01 裁决：B，删掉）

- **A. 保留并去掉任务含义**（Claude 原建议，未采纳）：
  - 改成 `Observation::Deferred { call_id, host: Value }`：以 call_id 为键，`host` 原样存进快照，Runtime 自己的 `until_ms`、`check_after` 都放在里面。
  - 删掉 `allow_deferred` 和"必须带 task_id"的检查。要不要挂起，由 Runtime 构造 ToolManager 时决定。
  - llm_context 只负责：停止派发后续调用、快照里记下哪些调用在等结果、恢复时要求每个调用补一个结果。
  - 理由一：provider 协议要求每个 tool_call 后面先跟 tool_result 才能有下一条消息，这件事 Runtime 在外面做不出来。不挂起，就只能让进程一直等（还占着 run 锁），或者先编一个"仍在运行"的结果。
  - 理由二：见原则 6。
- **B. 删掉**（**用户 2026-11-01 裁决**）：
  - 工具永远当场返回：在调用里等一段有上限的时间，还没完就返回"仍在运行，任务 X"。任务完成后，Runtime 把结果作为新输入交给模型。
  - 好处：llm_context 只剩纯 loop，快照少一种挂起状态。
  - 代价一：libopendan 的 WaitingRun 和回填路径要重写（drive.rs 里 PendingTool 转 WaitingRun 的部分、live.rs 的 `fill / try_fill`），xllm 的 `fill_suspended_calls` 删掉。
  - 代价二：ask-user 变成"结束本轮并提问"；`call_behavior` 的子 session 结果变成新输入；需要审批的操作变成"先返回待审批，批准后执行，再把结果作为新输入"。
  - 代价三：模型要多推理一次，还要自己把后来到达的结果和当初的调用对上。
  - **用户裁决理由**：原则 8。A 的"挂起 + 回填"就是 `DoModal` 式的嵌套小循环，只是把阻塞从工具搬到了 loop；B 才是"循环里没有等待"。A 的理由一（provider 要求 tool_call 后紧跟 tool_result）由"仍在运行"这条合成结果满足，和 shell auto 模式今天的做法一样。代价二正是想要的效果：ask-user、审批、子 session 全部变成任务，等待期间 session 是否继续推理由 session 自己定。代价三接受。

xllm 现在的行为已经等同于 B。A 和 B 的差别只影响 libopendan。

#### 3.2.1 B 的形状

xllm 和 opendan 共用同一个 loop，差别只在 Runtime 的三个旋钮，llm_context 一个都看不到：

| 旋钮 | xllm | opendan |
| --- | --- | --- |
| ToolManager 在调用内等多久 | auto 30 秒、上限 30 分钟（`MAX_IN_TOOL_WAIT_MS`） | 与 xllm 相同的 auto 30 秒（第四轮修正：原写"短预算或 0"，0 会让每条 `ls` 变成两次推理） |
| "仍在运行"之后谁跟任务 | 模型自己 `wait_task`（没有事件源） | session 登记任务，等 task done 事件 |
| 什么事件再触发推理 | 只有人工 `resume` | task done、新输入、定时、子 session 结束，由 session 类型决定 |

"所有工具都返回异步"的准确说法是：按原则 2 每个工具都是异步的，调用内的等待预算是 Runtime 的参数；快的工具在预算内结束，当场拿到最终结果，是同一个原语的特例。预算值不进冻结面。

等待规则只有一条：每个任务都在 Runtime 预算内等，不按工具分类（第六轮改：原写"ask-user 等由 `pending_reason` 声明 0 等待"，与 ask-user 一般化冲突，xllm 正是要在调用内等答案）。`pending_reason` 保留为给模型看的说明，ToolManager 可以把它当跳过调用内等待的**可选优化**（opendan 有事件源，等人 30 秒没意义），不是语义。`call_behavior`（`subctx:`）不是例外：`subctx:` 任务的后端就是 session（F13 按名注册的后端之一），它对 `wait` 立即返回 Running，预算规则不变（第七轮改，原写"工具自己立即返回"的例外已删）。

- **删除的原语**：`Observation::Pending`、`ToolCallCtx.allow_deferred`、`Outcome::PendingTool`、`Suspension::PendingTool`、`ResumeFill::ToolResults`，以及 context_loop.rs:970 的检查。`ResumeFill` 只剩 `ContextLimitReached` 的两个变体。xllm.rs:9111 对 PendingTool 的 Failed 分支随之消失。
- **"仍在运行"结果**：一条普通的终态观察（AgentToolResult v1 信封，`pending_reason` 字符串，F8），正文由 Runtime 渲染（I1 搬到 agent_tool 的 tasks 规范），必须带 call_id、任务 id、工具名和参数摘要，模型靠这些把后到的结果和当初的调用对上。信封的 `status = pending` 和 `ToolResultView.status` **保留**，它们是"仍在运行"结果的载体；删的只是 waist 的 `Observation::Pending` 变体。step_record.rs 三处对 `Observation::Pending` 的渲染改成按 view status 判断。
- **任务完成怎么进上下文**：provider 不允许给已经有结果的 tool_call 再补一条 tool_result，所以完成结果只能以**输入**进来。opendan：task done 是 session 路由的一条输入（和 `<<background_context>>`、新消息同一条路，`route_inputs`），渲染成独立块 `<<task_result call_id=… task=…>>`，正文就是任务的 AgentToolResult 信封（F7）。它经 F10 的持久注入写进历史和快照，不伪装成工具结果（§3.7）。xllm 单 run：没有事件源，维持模型在预算内 `wait_task`。
- **ask-user = 标准异步 function call**（用户倾向 2026-11-01 第六轮，F20）。现状有两条路（[ask.rs](../src/frame/lib_opendan/src/runner/ask.rs)，463 行）：Work session 走通知；有消息队列的 session 走 Turn 回复路径，Turn 保持打开，队列里下一条消息被当成这次调用的 tool result。一般化之后只剩一条：
  - **任务就是通知系统的任务。** 源码核对：`input` 通知在 msg-center 侧就创建一个 Zone TaskMgr 任务（`human.input/v1`，[msg_notify.rs:513](../src/kernel/buckyos-api/src/msg_notify.rs) `NotifyTaskInput`，`NotifyView.task_id`），按 requester + idempotency_key 幂等（msg-center `notifies` 表唯一索引），答案经 `NotifyAnswer::to_task_result` 提交成任务结果，过期 / 撤销也是终态。所以 ask-user 一般化不需要新的任务后端：工具发一条 `input` 通知，幂等键 `ask:<run_id>:<call_id>`，返回"仍在运行，任务 t-…"的信封；答案到了就是 task done。F3 / F12 的"按 call_id 先找"由 msg-center 的幂等自动满足。
  - **xllm 拿到反馈的条件**：(1) 工具不再是 libopendan 的 `agent-session ask-user`（要 `OPENDAN_SESSION_ID`），改成按"Agent 工具优先 TS + websdk"写成 websdk CLI（websdk 已有 `notifySend / waitNotify / answerNotify`），用会话 token 的 `sub` 作收件人，返回 AgentToolResult 信封；(2) xllm 拿到 BuckyOS 会话 token 时按 F13 构造 Zone TaskMgr 任务后端（现在 `buckyos_tasks` 只有 libopendan live.rs:148 和 opendan tasks.rs:289 注入，CLI 单独跑是 `None`），没有就是 Capability 错；(3) **F7 提前为前置**：`ZoneShellTasks` 认领所有 `t-` id 又按 shell schema 渲染（§4），`human.input/v1` 的结果会被丢；Zone 解析器要按任务 `schema_id` 分派，或者通用规则"结果是 AgentToolResult 信封就用信封，否则 JSON 当正文"。有了这三条，xllm 在调用内等（预算内）或 `wait_task`，答案经通知系统回来，与 shell 长命令无区别。
  - **opendan 这边删的**：`ask::allowed / via_notify / on_pending / save_ask / AskNotify / send_notify_questions / notify_answers / queue_questions / take_answer / fill_answer`，`SessionToolManager::call_tool` 里的 userinput 截获，`WaitingKind::Input`。"有消息队列的 session 在对话里提问"变成通知系统的一种投递（`NotifyConversation reply_route` / `NotifyDelivery` 已有，MessageHub 卡片已有），不再靠"队列里下一条消息 = 答案"。能不能问由工具白名单和 Goal 的 `questions` 回流决定，不再是 session 策略。超时 = 通知的 `expires_at_ms`，默认沿用 3 小时。
  - **`WAIT_USER_MSG` 不再等价于 ask-user**：它是"Yield 且没有任务、下一条消息开新 Turn"，ask-user 是"有一个未完任务"。两者按 §3.2.2 同一条规则处理，F14 的 Turn 规则因此并入 F18："run 保活 = 本 run 有未完任务"，userinput 没有特殊地位。
  - **要定的一个细节**：问题任务开着时，用户在同一对话里发了普通消息而不是答卡片。建议 session 把本 run 的开放提问任务以 `superseded` 撤销（一行），否则任务挂到过期才关；模型则把这条消息当新输入处理。
- **`call_behavior`（`subctx:`）**（第四轮修正：原写"子 session 已启动…poll_children 已在跟"，混了两条路）：`subctx:` 是**同一 session 内的子 context**，走 `process_stack` 的 Caller 帧，session 一次只跑一个 context；`poll_children` 跟的是 `session:<sid>` 前缀的子 session，是另一条路。B 之下 call_behavior 的形状是复用已有的 **behavior 触发的 hand-over 路径**：[outcome.rs:1168-1174](../src/frame/lib_opendan/src/runner/outcome.rs) 里 `CallTrigger::Tool` 走回填、非 Tool 触发走 `internal_continuation` 把子 context 的结果渲染进调用方的 hand-over 批次；B 就是删掉前一条分支，工具触发也走后一条。工具返回"子 context 已登记，本 run 结束后切换"，session 在 run 结束（任何 outcome）时做切换。子 session（`session:<sid>`）完成已经是输入批次的候选（children.rs 头注释），不需要新东西。两处都是净删除。
- **审批**：返回"待审批，任务 X"；审批通过后由 Runtime 执行并结束任务，结果作为新输入。今天没有审批门（审批门裁决维持现状），"审批后由 Runtime 在 run 之外执行工具"要求工具可在 `call_tool` 之外被执行，**不在本次冻结范围**，只保证信封和任务模型容得下它。
- **libopendan 要改的**：`WaitingRun` 不再是 run 的一个状态，session 只保留"正在跟的任务列表"（现在 `pending_now` / `watched_tasks` 那套，outcome.rs 已有"run 结束后 session 继续跟后台任务"的路径，B 是让它成为唯一路径）；drive.rs:2046 的 PendingTool 分支、drive.rs:1550-1615 的等待段、live.rs 的 `fill / try_fill / fill_answer`、tools.rs 的 `pending_sub_call`、xllm 的 `fill_suspended_calls` 都删。
- **崩溃恢复（F3 在 B 下的形状）**：没有挂起态可回填，恢复只有一种：从工具调用前的 checkpoint（F5）重新进入 loop，`tool_batch` 里没结果的调用再次派发，**ToolManager 按 call_id 先找再做**：找到任务就返回它的当前状态（完成就是最终结果，没完成就是"仍在运行"），找不到才决定执行或返回 Unresolved。"不重放"原则从 llm_context 移到 ToolManager：没有任务记录、又不幂等的工具，由 ToolManager 返回 Unresolved 而不是执行。这条要单独裁决（F12）。第四轮核对：**llm_context 侧零改动**，`run_inner` 本来就先派发 `tool_batch.remaining`；宿主只要不再调 `materialize_unresolved`。shell 的 `describe_interrupted` 读 `exec/<call_id>/` 就是"按 call_id 找"的现成实现。要补的是工具级 `idempotent` 标记（`AgentTool` 上一个默认 `false` 的方法）：read / list / write_file 可重跑，edit_file 和无任务记录的 shell 转 Unresolved。串行派发（原则 10）是这条成立的前提：checkpoint 里"已有结果的前缀"确定，`remaining` 从第一个没结果的调用开始，重放顺序和首次一致。

#### 3.2.2 两种 loop 的结束状态与续接（第五轮，Claude 按用户口述回写，待 review）

**结束状态。**

| | 普通 loop（function call） | behavior loop |
| --- | --- | --- |
| waist 终态 | `Done`，assistant message 就是结果 | 仍是 `Done`，Step 形状分三种：End（`<report end="true">`）、Handover（非空 `next_behavior`）、**Yield**（没有 action、不是 end；`<report>` 可有可无） |
| 含义 | 本轮回复，这个 context 的推理结束 | End：context 结束。Yield：这个 context 的推理到此为止，context 没有结束，下一次推理要等新输入 |
| 宿主要判的 | Turn 关不关（F14） | Yield 之后等什么、怎么续（F17 / F18） |

Yield 是 behavior 版的"没有 tool call 的 assistant message"。现状正好拧着：没 action、没 report、没 next_behavior 在 [behavior_loop.rs:305](../src/frame/llm_context/src/behavior_loop.rs) 是校验错误，进纠错 Step；只带 report 的 Yield 在 [todos.rs:159](../src/frame/lib_opendan/src/runner/todos.rs) 的 `review_yield` 得到 None 之后同一 run 内立刻再推理，`MAX_IDLE_YIELDS = 2` 次后才拒绝。按原则 8 这两处"同 run 内再推理"都是循环里的等待，删：**Yield 一律立刻结束 run**。waist 不加新的 Outcome 变体（`Done` 加 `decision()` 的形状就够，冻结面不变），也不判断"有没有东西可等"，那是 session 的事。普通 loop 不加 yield 概念（F19）：异步工具返回"仍在运行"后模型 `Done`，形状上分不出是回复还是等待，靠任务登记判，Done 永远是本轮回复。

**什么能推进推理。** [drive.rs:1715](../src/frame/lib_opendan/src/runner/drive.rs) 的 `triggered` 只看三样：选中的输入批次、bootstrap、内部续接；`<<background_context>>` 只搭便车。写成明文：**注入是推理边界上的观察，不是触发**；能开一次新推理的只有输入：`<<task_result>>`、消息、子 session 结束、userinput 的答案、Goal 定时激活。两种 loop 相同。

**Done / Yield 之后 session 的裁决**（两种 loop 同一条规则，并入 F14）：

- 本 run 登记了未完的任务（shell 任务、userinput、子 session 都是任务）：run 保活，等输入。等待期间来的普通消息也注入同一 run（原则 8：等待期间 session 可继续推理）。
- 什么都没有：一次性裁决。behavior 的 Yield 有 report 文本就当回复，没有就以"nothing to wait for"结束 Turn（原 `MAX_IDLE_YIELDS` 的拒绝语搬到 session，只说一次）；普通 loop 的 Done 按 F14 关 Turn。loop 里不再数 trailing yields。

**续接方式（F18）：resume 同一快照并注入，不是新 run。**

- 矛盾：§3.2.1 写 task_result"经 F10 的持久注入写进历史和快照"，但注入只在 run 内的推理边界存在。今天 run 结束就 `live_run = None`（[outcome.rs:1078](../src/frame/lib_opendan/src/runner/outcome.rs)），下一个输入走 `new_run_context`，旧 Step 变成文本历史，没有注入点。
- 现成机制：[drive.rs:1982](../src/frame/lib_opendan/src/runner/drive.rs) 只要 `live_run` 还在，就走 `open_state_live_run` 把快照 resume 回来，再用 `commit_input_batch` 把输入批次 `inject` 进去，receipt 记 `after_step`。现在只有 hand-over 的 Caller 帧在用。
- 改动：behavior 的 Yield、普通 loop 有未完任务的 Done 都**不清 `live_run`**，run 状态 Paused；End 和无任务的 Done 才清，下一个 Turn 开新 run、用文本历史。Yield 和 End 的差别就落在这一点上。
- 后果：step 在同一 run 里继续编号；`<<task_result>>` 是 step 边界上的持久注入，模型对 call_id 时看到的是原来的 action 和它的"仍在运行"结果（StepRecord），不是文本摘要。普通 loop 同理：task_result 作为 User 消息注入在上一轮 assistant message 之后，和聊天多轮一样。上下文增长靠已有的 ContextLimit 压缩路径；崩溃恢复没有特殊情况，Yield 快照本身就是 step 边界的 checkpoint。
- xllm 单 run：没有事件源，run 停在 Paused；`resume` 命令就是"注入一条输入并续跑"的人工版，同一快照。

**两边运行起来的样子。**

| 场景 | 普通 loop | behavior loop |
| --- | --- | --- |
| opendan，shell 超过调用内预算 | round n 调 shell → 结果"仍在运行 task T"→ round n+1 模型回复"已启动，完成后汇报"→ `Done` → session 有未完任务 T：Turn 不关、run Paused → T 完成 → task_result 作为 User 消息注入 → round n+2 → `Done` 且无任务 → Turn 关 | Step k 派 shell → 结果"仍在运行 task T"→ Step k+1 Yield → run Paused，session 跟 T → T 完成 → `<<task_result call_id=… task=T>>` 注入 → Step k+2 |
| opendan，ask-user | 同上，任务是 Zone TaskMgr 的 `human.input/v1` 任务（F20），答案是 task_result 输入 | 同上；`WAIT_USER_MSG` 保留但**不等价**：它是 Yield 且无任务，下一条消息开新 Turn |
| xllm，ask-user | "仍在运行 task t-…"→ 模型 `wait_task`（预算内）→ 用户在通知里作答 → 任务结果就是答案；超过预算则 `Done` → run Paused → 人工 `resume` | 同左 |
| opendan，什么都没派就停 | `Done` 即回复，Turn 按 F14 关 | Yield 无任务：有 report 当回复，否则 Turn 以"nothing to wait for"结束 |
| xllm | "仍在运行"→ 模型 `wait_task`（预算内）或 `Done` → run Paused → 人工 `resume` 注入 | 同左，`Done` 换成 Yield |

**删除面**（并入 §3.10）：`validate` 的空决策错误；`review_yield` 的 Yield 分支、`trailing_yields`、`MAX_IDLE_YIELDS`；`WaitingKind::Children` 泛化成"在跟的任务列表"，`wait_for` 的 children 专用字段并入 `watched_tasks` 那套；Agent Actions §2.4 第 5 条、§2.5 的"空决策纠错"改写。

### 3.3 call_id 即幂等键

- **I5** Runtime 提供"call_id → 任务"的查询。恢复时 loop 重新派发 in-flight 的调用，ToolManager 先按 call_id 找任务（§3.2.1）：
  - 找得到：返回任务当前状态（完成即最终结果，未完成即"仍在运行"），不重新执行；
  - 找不到：幂等的工具可以执行，否则转 Unresolved（F12）。
- 外部服务分配 id 的工具（例如 AIGC 视频），把 call_id 作为幂等键传过去（TaskMgr 的 `create_task` 已经支持 idempotency_key）。不支持幂等的工具，照旧标 Unresolved。
- 由此带来的后果：Runtime 层的任务 id 只用于"call_id → 任务"的映射，llm_context 看不到它。

### 3.4 checkpoint 只留一个事实来源

- **I6** "哪个调用正在跑"现在有两份记录：xllm 用推理前快照加 `run.json` 的 `inflight[]`，libopendan 用工具调用前快照里的 `tool_batch`。建议统一为：以 llm_context 在工具调用前打的 checkpoint 为唯一事实来源。
- **第四轮推到底：`inflight[]` 和 `InflightAction` 整个删（F15）。** call、args、call_id 都在工具调用前快照的 `tool_batch` 里；执行目录由 call_id 派生（`exec/<call_id>/`）；开始时间读执行目录；Zone TaskMgr 的幂等键由 run_id + call_id 派生（tools.rs:256 已是 `h(run_id, call_id)`）。RunRecord 升 v7，`persisted_outcome_ids` 对账和 `register_inflight` 一起删。

### 3.5 冻结对象分成两份，分别写规范

- **I7** 一份是 llm_context 快照（v5，简化后升版本），一份是 xllm runtime 的 run 目录 / RunRecord v6。TS 版要接手 Rust 跑出来的 run，两份都得实现。core 只依赖第一份。 派发顺序是第一份规范的一部分（原则 10）：批次内按返回顺序串行，前一个有结果才派下一个；action 批次遇业务错误即停、其余记 Unresolved；function call 批次每个调用都派完。两种 loop、两种语言逐条一致。

### 3.6 合并两个推理前钩子

- **I8** 删掉 `InferenceHook`，只留 `CheckpointHook`：`before_tool_call(snapshot)` + `before_inference(snapshot) -> Option<Injection>`（行为/报告相关的 `validate_report`、`review_decision` 照旧）。xllm 的 `SnapshotHook` 迁到 `CheckpointHook`，顺带实现 `before_tool_call` 写快照。
- 这正好落实 F5：xllm 和 libopendan 都以工具调用前的 llm_context 快照为唯一事实来源，`run.json` 的 `inflight[]` 随 F15 删除。
- 冻结面少一个 trait，TS 版只实现一个钩子。
- **代价（第四轮）**：合并后 behavior 模式下 step 内的推理不再打快照（§2.4：`CheckpointHook` 只在 step 边界调）。崩溃发生在 step 内某次推理中时，从上一个工具调用前快照或 step 边界恢复，最多多付一次推理；副作用由工具调用前快照兜住。可以接受，要写进设计文档；"扁平快照"这个概念随之从钩子契约消失，也是简化。`InferenceHook` 在 tests/error_policy、function_call_loop 里的用法改接 `CheckpointHook`。

### 3.7 推理前注入是 llm_context 唯一的上下文原语（用户意图 2026-11-01）

用户意图：传统 function call 的 loop 里，`new_inputs` 和 `background_context` 的更新由 llm_context 在推理前拼装时填入，**不**做成 AgentToolResult 的一个 `context` 字段伪装成工具结果的一部分。

现状已经是这样（§2.4）：`CheckpointHook::before_inference` 返回的 `Injection` 就是这个入口。理由记下来，避免以后再回到"工具结果带 context"的想法：

- 工具不知道 session 级背景，让它携带是职责错位；plan_simple TODO §4.4 已裁决"宿主观察使用独立块，不伪装成工具自身的输出"。
- 一批多个调用时 context 挂在哪个结果上没有答案；一个独立块放在整批之后才干净（`inject` 已保证批次配对完才放）。
- 注入进了历史就被 checkpoint 住，恢复时模型看到的完全一致；塞进工具结果则被永久冻结进那条结果里。

- **I9** 把"推理前装配注入"定为 llm_context 冻结的唯一注入原语，只有一种形态：
  - **只有持久一种形态**（2026-11-01 第八轮裁决 F16）：进历史、进快照，就是现有 `Injection { messages, host }`，不加任何字段。用于新输入、`<<background_context>>` 变化行、`<<task_result>>`。
  - **不做临时形态**。原建议是 `Injection` 加 `ephemeral: Option<String>` 承接 `<background_tasks>` 的每轮刷新；用户裁决不要：每轮变化的内容影响 LLM cache，且 B 之下模型已持有任务 id，不需要每轮刷新的任务清单。`background_env`、`LLMContextDeps.tasks`、`render_background_env*` 随 I3 一起删，llm_context 零临时概念；模型要状态就调 `get_task_state`。
- 撤销 [jarvis-chat-prompt-todo.md](./jarvis-chat-prompt-todo.md) §1.2 和 [llm-context-chat-prompt-prereq-todo.md](./llm-context-chat-prompt-prereq-todo.md)"以后"节里"function result 里也出现 background_context，要改 agent_tool_result_protocol / `BackgroundProvider` 钩子"的想法：都由 I9 的持久注入覆盖，不改 AgentToolResult 协议。
- xllm 接上 `CheckpointHook`（I8）后，CLI 单独跑也有了注入入口。

### 3.8 AgentToolResult 的 hint：结果替代与渐进式披露（用户意图 2026-11-01，第九轮按用户口述重写）

用户的两条核心意图：

1. **result 可以有条件地被一句 hint 替代。** 例如 read_file 的内容未变，就不回正文，只回一句"与上次读到的相同"。条件由 Runtime 按历史判，工具只声明键和替代句。
2. **渐进式披露。** 工具可以随 result 返回操作 hint："拿到这个 result 之后，下一步可用的 CLI 命令有哪些"。这类 hint 也可以有条件地只出一次。提示词渲染器要把 hint 渲染成 LLM 不会理解错的结构：它是 Runtime 附带的使用说明，不是命令输出。

之前 ReadFile"未修改就出 hint"是要工具实现者自己仔细设计的特例，这两条把它变成通用机制。

- **协议**（同主版本内加可选字段是合法演进，见 [agent_tool_result_protocol.md](../doc/llm_context/agent_tool/agent_tool_result_protocol.md)"版本演进规则"）：

  ```json
  "hints": [
    { "text": "..." },
    { "text": "...", "once": "shell.long_running" }
  ],
  "dedup": { "key": "read_file:<abs_path>:<range>:<blake3(content)>", "hint": "与上次读到的内容相同，正文省略" }
  ```

  - `hints[].text` 必填。没有 `once` 的每次都出；带 `once` 的同一 session 内只写进 LLM 输入一次，之后同 key 的被丢弃。工具每次照常返回，不自己记状态；要"变化之后再提醒一次"就把版本放进 key（`<tool>.<topic>[:<version>]`）。
  - `dedup` 可选。Runtime 看到同 `key` 已在本 run 的历史里出现过，就把 `output` / `detail` 整体换成 `dedup.hint`（缺省一句"与之前的结果相同，正文省略"）；`title` / `summary` 和控制字段不动。ReadFile 不用自己设计，任何工具都能用。
- **I10 替换与渲染都在 Runtime 的 ToolManager**（`XllmToolManager::call_tool`，[xllm.rs:4096](../src/frame/agent_tool/src/xllm.rs)，agent_tool 内）。llm_context 不知道 hint 这个概念（原则 1）；F22 之后它连信封字段都看不见，`hints` / `dedup` 随信封在 `Observation.host` 里原样携带，LLM 文本由 agent_tool 渲染。
- **渲染结构**（两种 loop 同一个形状，进共享向量）：hint 放在输出之后一个固定标记的区块里，output 区块里不得出现 hint 文本，output 除 dedup 替换外不被改写。例如：

  ```
  <output>
  …命令输出…
  </output>
  <runtime_hints>
  - 下一步可用：`buckyos task wait <id>`、`buckyos task cancel <id>`
  </runtime_hints>
  ```

  behavior 模式由 `XmlStepRenderer`（F22 后在 agent_tool）渲染成结果内的独立子元素；function call 模式的 `content` 字符串用同一对标记，模型在两种 loop 里看到相同结构。标记名由 agent_tool 定，协议文档写明"`runtime_hints` 是 Runtime 的说明，不是工具输出"。
- **账本是宿主依赖**：`HintLedger` trait（`seen(ns, key)` / `mark(ns, key)`），两个命名空间 `once` / `dedup`，由 `XllmDeps` 注入，缺省内存实现。`once` 按 session 存（xllm 单 run 存 run.json；libopendan 存 `SessionState`，与 `background_shown` 同款），丢了只是多显示一次，无害。`dedup` 随 run 存，上下文压缩重写历史（`RewrittenHistory` / `RewrittenSteps`）或派生子 context 时清空，因为替换的前提是"之前的正文还在模型视野里"。两者都**不进 llm_context 快照**。
- **归属和 §3.7 不矛盾**：hint 是工具对自己用法的说明，归属就是这条结果；background 是宿主的观察，归属是推理边界。
- **CLI 路径自动覆盖**：cmd_skills 经 bash 返回的 envelope 走同一个映射。手册里"拿到结果后下一步命令"的段落可以搬进 hint 按需出，手册变短。
- 文档：`agent_tool_result_protocol.md` 加 `hints` / `dedup` 两节和渲染结构；共享向量加 `HintLedger` 语义与渲染样例（TS 版同样实现）。
- **排期**：加字段不是冻结必需，排在 §6 的删除工作之后单独做。

### 3.9 Runtime 预设：xllm 与 opendan 的差别做在 AgentRuntime 配置里（用户想法 2026-11-01）

用户意图：既然有 AgentRuntime 这层抽象，xllm 和 opendan 的主要差别就应该做在这里。系统预设一组 runtime（kind 仍只区分执行能力，xllm 大多数时候用 native），xllm 和 opendan session 都能用，只是配置不同；有的预设依赖 agent session 的状态，只有 opendan session 能开。

现状先例：`remote_node` 只能在注入了 `node_executor` 连接器的宿主里构造，`RuntimeRegistry::from_config_with` 拿不到就报 Capability 错（[runtime/mod.rs:755](../src/frame/agent_tool/src/runtime/mod.rs)）。预设就是把这个模式从一个 kind 推广成一组配置。

§3.2.1 的三个旋钮里，两个能进 runtime，一个不能：

| 旋钮 | 现在在哪 | 进 AgentRuntime？ |
| --- | --- | --- |
| 调用内等待预算 | `ShellSettings`（`tools.shell.mode / wait_ms`，只管 shell）；`allow_deferred` 在 request 上由 libopendan 设 | 进。B 之后只剩这一个旋钮，应对所有能转任务的工具生效，不只 shell |
| "仍在运行"之后谁跟任务 | `XllmDeps` 的 `local_tasks / shell_tasks / buckyos_tasks / shell_task_context / task_cli`，宿主注入的对象 | 进。预设写任务后端名（进程内 / Zone TaskMgr），注册表按名从 deps 取对象，取不到即 Capability 错。Zone TaskMgr 要 App token 和 session id（幂等键 `shell:<run_id>:<call_id>`），所以这类预设天然只有 opendan 能开 |
| 什么事件再触发推理 | 只在 opendan 的 drive / `route_inputs` | 不进。AgentRuntime 设计 §9：不接管 Session 状态机。runtime 最多在 `Capabilities` 声明"任务有完成事件"，订不订阅由 session 定；xllm 不订阅，靠模型 `wait_task` |

预设示例（配置层，不是新 kind）：

- xllm 默认：native + 进程内任务 + auto 30 秒 / 上限 30 分钟 + 无事件；
- session 本机：native + Zone TaskMgr + 短预算或 0 + 有事件，需要 session 上下文；
- session 节点：remote_node + Zone TaskMgr，需要 node connector 和 session 上下文。

要提前定的三点：

- **runtime_id 不受预设影响。** 它由 host + uid 派生（runtime/mod.rs:563），已在冻结向量里，session 的 requirement / binding 按它核验。策略字段若算进身份，同一台机器上 xllm 和 session 的 id 不同，§4 "CLI xllm 接手 libopendan 的 run"就断了。身份只看执行体。
- **`tools.shell.wait_ms` 的去留。** 建议删掉工具级字段，由 runtime 预设统一给，少一层合并。
- **run.json 记预设。** RunRecord 已存 RuntimeConfig 和 descriptor，加字段是兼容演进；恢复时核对"接手方能否开这个预设"，与 F6 的工具 spec 对账同类。TS 版同样实现。

与 F2=B 互相加强：B 之后 llm_context 没有任何旋钮，"同步只是异步的特例"落在 runtime 预设的等待预算上，xllm 与 opendan 的差别全部成为配置差别。

**第四轮建议：不建预设注册表。** B 之后差别只剩等待预算和任务后端两项：预算加一个 `RuntimeConfig.task_wait_ms` 字段；任务后端仍由 `XllmDeps` 注入，取不到报 Capability 错是现状（`remote_node` 先例）。"预设"就是文档里的几份配置示例，不是代码。`tools.shell.wait_ms` 删除照旧；`ShellMode` 的 wait / auto 一并删（§3.10）。runtime_id 不含策略字段、run.json 记预算字段并在恢复时核对，这两条保留。

### 3.10 再删清单（第四轮）

B 之后还能顺手拿掉的，全部是净删除：

| 项 | 现状 | 删后 |
| --- | --- | --- |
| `inflight[]` / `InflightAction`（F15） | run.json 一份、快照 `tool_batch` 一份 | 只剩快照；执行目录、开始时间、幂等键都从 call_id 派生 |
| `ShellMode` wait / auto（F15） | `tools.shell.mode / wait_ms` | 一个预算字段：wait 模式 = 预算等于超时。TS 的 T4-2 不用再区分 |
| `ToolCallCtx` | `{ abort, deadline_ms, allow_deferred }` + `wait_until_ms` | `{ abort, deadline_ms }`；`wait_until_ms` 与 `MAX_IN_TOOL_WAIT_MS` 搬到 agent_tool |
| `Suspension` 枚举 | `PendingTool` / `ContextLimit` | 只剩一个变体，扁平为 `suspended: Option<ContextLimit { which, estimated_tokens, at_ms }>`；`SNAPSHOT_FORMAT_VERSION` 升 6，不兼容旧快照 |
| `LLMContextOutcome` | 七态 | 六态；`ToolExecStatus::Pending` 删 |
| `ResumeFill` | 四态 | `RewrittenHistory` / `RewrittenSteps` / `ResumeFromMidRun` |
| 空决策纠错 + idle yield（F17） | `validate` 报错、`review_yield` 的 Yield 分支、`trailing_yields`、`MAX_IDLE_YIELDS` | 空 Step 就是 Yield，立刻结束 run；"没东西可等"由 session 一次性裁决 |
| ask-user 的 session 专用路径（F20） | `runner/ask.rs` 全部、`xagent_cli` 的 `ask-user` 子命令、`WaitingKind::Input`、`SessionToolManager::call_tool` 的 userinput 截获、`session.policy.wait_user_msg / ask_user_timeout_secs` | ask-user 是 websdk CLI 工具 + Zone TaskMgr 的 `human.input/v1` 任务；session 只剩"在跟的任务列表" |
| `WaitingRun` / `WaitingKind::Children`（F18） | run 的等待状态 + children 专用字段 | 只剩 `live_run` 保活 + 在跟的任务列表；子 session、userinput 都是任务 |

### 3.11 waist 里"有特殊地位"的东西（第七轮扫描，2026-11-01）

**2026-11-01 用户裁决：A、B 两类全部按下表处置（F21–F24），C 类保留。**

判据就是设计文档 §1 的双向中立：换一种调度器或一种 provider 仍然自然的才留在 waist。扫的是 `lib.rs` 的全部公开导出、请求 / 状态 / 结果的每个字段、以及各模块在 agent_tool / libopendan / opendan 里的使用方。基数：llm_context 共 14951 行，其中**不在 loop 职责内的模块 8468 行**（xml_behavior 1179、xml_util 121、msg_parser 1501、prompt_engine 1756、prompt_compose 358、prompt_budget 466、snapshot_overrides 491、tasks 403、step_record 1598、context_derive 595）。

**A. 没有外部使用方或 loop 从不读的，直接删（F21）**

| 项 | 事实 | 处置 |
| --- | --- | --- |
| `XmlBehaviorParser`、`XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT`、`V2_ACTION_TAGS`（xml_behavior.rs 1179 行） | xllm 用的是自己的 `XllmActionParser`（xllm.rs:3366），waist 里这份无人调用；提示词里直接写着 `write_file / edit_file / read / shell` 的用法，是 Agent 工具集的说明 | 删。`xml_util` 的 `escape_xml_*` / 扫描函数有 6 个宿主文件在用，随 B 类搬去 agent_tool |
| `snapshot_overrides.rs`（`RequestOverrides` / `rebuild_with_inherit` / `build_fresh`） | 无外部使用方；`user_messages` 注释写"legacy opendan runtime only" | 删 |
| `prompt_compose.rs` + `prompt_budget.rs` | 无外部使用方 | 删 |
| `StepResultHook` / `StepResultHookOutput` / `HistoryInputRecord` / `state.history_inputs` / `deps.step_result_hook` | 无实现者 | 删；快照少一个字段 |
| `HumanPolicy.approval_required` | loop 从不读 | 删；审批走任务（§3.2.1） |
| `ToolPolicy.parallel`、`max_observation_bytes` | loop 从不读（xllm 设了 64K 也没人用） | 删 |
| `state.llm_task_ids` / `ContextRunTrace.llm_task_ids` | 无外部读者；provider 侧取消只靠 `InferenceAbortTrace.provider_task_ref` | 删 |
| `messages_to_send` / `SendMessageRecord` / `<sendmsg>` / `WorkEvent::MessageSent` | 注释"stub-delivered in v2 first cut"，三年没落地；libopendan 只在 reconcile.rs:155 拿 `messages_sent` 非空当"已回话"信号 | 删整条；"已回话"由 send_message 工具的结果判 |

**B. 宿主协议进了 waist，搬走（F22 / F23）**

| 项 | 事实 | 处置 |
| --- | --- | --- |
| `msg_parser.rs`（1501 行，`MsgObject` ↔ `AiMessage`） | 只有 libopendan 的 outbound.rs / bridge/msg.rs 用 | 搬 libopendan |
| `prompt_engine.rs`（1756 行） | `fs::read` 做 include 展开，违反原则 5；agent_tool runtime/mod.rs 与 libopendan assembler.rs 用 | 搬 agent_tool |
| `ToolResultView`（`Observation.*.tool_result`） | AgentToolResult v1 字段的镜像（cmd_name / cmd_args / pending_reason / check_after / task_id / return_code / partial_output），注释说"放这里是让渲染器不依赖 agent_tool"，结果是 agent_tool 协议进了冻结面 | `Observation` 改带不透明 `host: Option<Value>`（信封原样），渲染器在宿主侧解读；waist 不知道任何信封字段（F22） |
| `step_record.rs` `XmlStepRenderer`（1598 行） | 渲染 `ToolResultView` 字段；只有 xllm 用 | 随 `ToolResultView` 搬 agent_tool；waist 只留 `StepRenderer` trait（F22） |
| `RefusalKind { QuotaExhausted, QuotaBudgetExceeded, ResourceNotPermitted, AttributionUnavailable }` | AICC 额度 / 归属语义进了 waist 错误枚举 | `ProviderFailure::Refused { code: String }`，code 由 provider 适配器定义；waist 只区分可重试 / 不可重试 / 被拒绝 / 上下文超限（F24） |
| `ContextOwnerRef { Agent, Workflow, OneShot, Other }` | 调度器名单进了 waist；loop 只在 worklog 一处原样输出 | 改 `owner: Value`，不透明（F24） |
| `tasks.rs` | F1 | 已定移出 |
| outcome.rs 文档里的 `WAIT_USER_MSG`、state.rs / behavior_loop.rs / xml_behavior.rs 引用 `doc/opendan/Agent Actions.md`、snapshot_overrides.rs 的"legacy opendan runtime" | 文档层面的特殊地位 | 随代码删除消失；留下的注释改中立表述，协议文档归宿主 |

**C. behavior 模式里要判定去留的（保留理由写进设计文档）**

| 项 | 判定 | 处置 |
| --- | --- | --- |
| `self_report` / `report_end` / `report_result` + `state.last_report` / `report_end` + `CheckpointHook::validate_report` | "step 的结构化产物 + 结束标记 + 宿主校验"对 Workflow / Eval 同样自然，中立 | 保留 |
| `report_artifacts: Vec<String>` | cyfile id 列表，是 Agent 的附件约定；且 behavior_loop.rs:280 单独校验它 | 并入 `report_result`（Value），附件由宿主在 Value 里约定（F24） |
| `review_decision` / `StepDecision` / `DecisionVerdict` | 宿主对 step 的裁决，`Refuse` = 同 run 内纠错，不是等待（原则 8 不反对） | 保留；注释去掉"Todo executor"措辞 |
| `OutputValidator` | 回复契约的宿主校验，libopendan `ReplyValidator` 在用 | 保留 |
| `forbid_next_behavior` | fork 子 context 的控制流约束，纯 loop 规则 | 保留 |
| `context_derive`（`derive_child` / `fork_snapshot`，live.rs 用） | 快照代数 | 保留；模块注释去 `PendingTool` |
| `objective` / `trace` | 审计，不透明 | 保留 |
| `CheckpointHook` 四个方法 | `before_tool_call` / `before_inference` 通用；`validate_report` / `review_decision` 只在 behavior 模式触发 | 保留一个 trait，文档写明后两个的触发条件 |
| `WorkEvent` | `SelfReportSet` 随 report 保留；`MessageSent` 随 A 类删 | 其余保留 |

**TODO 自身里的特殊地位**：§3.2.1 给 `call_behavior` 写了"工具自己立即返回"的例外。按 F13 的任务后端模型它不该是例外：`subctx:` 任务的后端就是 session（按名注册的后端之一），它对 `wait` 立即返回 Running，Runtime 预算规则不变。回写时把这句例外删掉。

做完 A + B 后，llm_context 剩下的是 request / state / outcome / observation / error / interrupt / deps / suspension / context_window / json_check / context_derive / behavior_loop / context_loop，约 6500 行，公开类型里没有任何一个字段需要解释"这是哪个宿主的"。

## 4. 冻结前要修的已知出入

- **工具 spec 不对账。** `rebuild_toolset` 把重新展开的工具丢掉了（`_eff`，xllm.rs:7608）。模型看到的 spec 来自重建后的 manager，只按 whitelist 过滤名字（[deps.rs:604](../src/frame/llm_context/src/deps.rs)）。建议：以 `run.json` 里的 `ResolvedTool`（name、args_schema、description）为契约，恢复时对模型广告保存的 spec，并要求每个名字都能派发，否则 `NotResumable`。具体场景是 CLI xllm 接手 libopendan 的 `bash_core` run：没有 `task_cli`，task 工具又不在 whitelist 里（按代码推断，未实测）。
- **runtime descriptor 的派生规则本身就是协议。** native 的 `runtime_id = "native:" + blake3(紧凑 JSON {"host": "...", "uid": "<字符串>"})[..24]`（runtime/mod.rs:563），其他语言必须逐字节复现。放进共享向量。
- **`AgentToolResult` 的结构和协议文档有出入**（[agent_tool/src/lib.rs:423](../src/frame/agent_tool/src/lib.rs)）：多了一个文档里没有的 `estimated_wait`；`pending_reason` 是严格枚举，新增取值会让老解析器整个失败，建议改成字符串；`detail` 默认序列化为 `{}`、`summary` 总是序列化，跨语言比对 JSON 时要注意。
- **TaskMgr 适配只为 shell 写过。** `ZoneShellTasks` 认领所有 `t-` id，读取时不检查 schema，渲染却完全按 shell 来（[tasks_zone.rs:521](../src/frame/agent_tool/src/tasks_zone.rs)、`result_of`）。其他服务的 TaskMgr 任务结束后，真正的结果（例如 obj_id）会被丢掉（按代码推断，未实测）。想法：
  - 拆成"shell 任务本身"和"通用 TaskMgr 解析器"两部分；
  - 和工具有关的 TaskMgr 任务，result 约定为 AgentToolResult v1 信封；
  - `XllmDeps.buckyos_tasks` 改名为 `host_tasks`。
- **`classify_error` 靠字符串判断"疑似凭据问题"**（匹配 `token / 401 / expired` 等），违反"只看枚举"原则。TS TODO 的 Q6 同时写了"照搬"和"不做字符串匹配"。需要 AICC 给出结构化的凭据错误码。
- **文档与代码不一致：**
  - [agent_tool_result_protocol.md](../doc/llm_context/agent_tool/agent_tool_result_protocol.md) 写"`allow_deferred=false`（xllm、libopendan 当前都是）"；
  - [AgentRuntime 设计](<../doc/llm_context/AgentRuntime 设计.md>) §7 写"libopendan 只允许 `call_behavior` deferred"；
  - [local_llm_context_protocol.md](../doc/llm_context/local_llm_context_protocol.md) §9.1 工具名还写 `exec`，§8.3 的 inflight 说法过时，§11 还在讲已删除的 `executions[]`、`TrackedBashRunner`、`stop_execution`；
  - [xllm_rust_sdk.md](../doc/llm_context/xllm_rust_sdk.md) §5/§6/§10 写 RunRecord v5 / 快照 v4，与文首的 v6 / v5 矛盾。

  按"doc/llm_context 是内部开发指南"的规则，等分层定下来后直接按新分层重写。
- **TS TODO 要跟着调整**（等本单裁决后再改）：Q3（`XllmDeps` 不含任务解析器）、Q7（不做 PendingTool）、Q8（工具检查方式）、T4-2（Deno shell 只有 wait 模式）；F9–F11 定了之后再加：单一 `CheckpointHook`、`Injection` 只有持久形态（F16 不做临时）、`HintLedger` 与 `hints` / `dedup_key` 的 Runtime 去重。另外，之前"把 resolver / TaskState / task 工具放进 TS core P0"的建议作废，改为放进 TS 的 runtime 层；F2 已定 B，core 不需要任何挂起原语，Q7 的"不做 PendingTool / `allow_deferred`"从"xllm 本就关闭"变成"原语已删"。F15 定了之后 T4-2 的"Deno shell 只有 wait 模式"改成"预算字段"，RunRecord 改 v7 且没有 `inflight[]`；Q8 的"`inflight[]` 按 `describe_interrupted` 物化"改成"ToolManager 按 call_id 先找再做"。F21–F24 之后 TS core 不再有 `ToolResultView`、`RefusalKind`、`ContextOwnerRef` 枚举、`XmlBehaviorParser`、prompt 三件、msg_parser：Q1 的"不能直接编 wasm"原因之一（`prompt_engine` 用 `std::fs`）随 F23 消失，core 只剩 `buckyos-api` 的 `AiMessage` 类型依赖；`XmlStepRenderer` 属 TS runtime 层。

## 5. 待裁决

| # | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| F1 | 任务概念是否整体移出 llm_context（I1–I3） | 移出 / 保留 | 移出 |
| F2 | 挂起原语（3.2） | A：`Deferred { call_id, host }` / B：删掉 | **已裁决 B（用户 2026-11-01，原则 8）** |
| F3 | call_id 是否作为幂等键，恢复时重新派发、ToolManager 按 call_id 先找再做（I5） | 是 / 否 | 是 |
| F4 | 后台任务提示放在哪（I3） | 包装 LlmClient / 通用临时钩子 / 不要 | **已裁决：不要（用户 2026-11-01，随 F16）** |
| F5 | checkpoint 的事实来源（I6） | 工具调用前的 llm_context 快照 / 维持两份 | 工具调用前快照 |
| F6 | 恢复时工具 spec 的契约 | `run.json` 保存的 spec / 重建结果 | 保存的 spec |
| F7 | TaskMgr 上工具相关任务的 result 格式；Zone 解析器不再只认 shell | AgentToolResult v1 信封 / 按 schema 各自渲染 | 信封；解析器通用规则"是信封就用信封，否则 JSON 当正文"。**随 F20 提前为前置**，`human.input/v1` 任务靠它渲染 |
| F8 | `pending_reason` 的类型 | 字符串 / 严格枚举 | 字符串 |
| F9 | 合并 `InferenceHook` 进 `CheckpointHook`，xllm 补 `before_tool_call`（I8） | 合并 / 维持两个 | 合并 |
| F10 | 推理前持久注入是 llm_context 唯一的上下文原语（I9） | 是 / 否 | **已裁决：是**；`Injection { messages, host }` 不加字段，临时形态见 F16 |
| F11 | AgentToolResult 加 `hints[]`（每次 / `once` 一次性）和 `dedup { key, hint }`（结果替代），Runtime 去重 + 固定结构渲染（§3.8） | 只加 hints / 两个都加 / 不加 | 两个都加；**用户 2026-11-01 确认两条意图**，字段形状待 review；排在删除工作之后 |
| F12 | "不重放"移到 ToolManager：恢复时重新派发，没有任务记录又不幂等的工具由 ToolManager 返回 Unresolved（§3.2.1） | 移到 ToolManager / llm_context 继续把 in-flight 直接标 Unresolved | 移到 ToolManager（否则 F3 落不了地）；`AgentTool` 加 `idempotent()` |
| F13 | xllm / opendan 的差别做成 AgentRuntime 预设：等待预算 + 任务后端进 RuntimeConfig，runtime_id 不受影响，`tools.shell.wait_ms` 删除（§3.9） | 预设注册表 / 只加 `task_wait_ms` 字段 / 维持 XllmDeps 散放 | 只加字段，"预设"是配置示例不是代码；runtime_id 只看执行体 |
| F14 | ask-user 改任务后 Turn 的关闭规则（§3.2.1） | Turn 关闭条件 = 没有未答的 userinput 任务 / 提问即关 Turn、答案开新 Turn | **并入 F18**（第六轮）：userinput 只是任务，"run 保活 = 本 run 有未完任务"一条规则覆盖 |
| F15 | 删 `inflight[]` / `InflightAction`（RunRecord v7）和 `ShellMode`（§3.4、§3.10） | 删 / 保留 | **已裁决：删（用户 2026-11-01）**，shell mode 是 runtime 的配置，不属于工具 |
| F16 | F10 临时形态 | `ephemeral: Option<String>` / 不要临时形态 | **已裁决：不要（用户 2026-11-01）**，每轮变化的内容影响 LLM cache；`<background_tasks>` 随 F4 删 |
| F17 | behavior 的 Yield 定义（§3.2.2） | 没有 action、不是 end 的 Step = Yield，report 可选，立刻结束 run / 维持空决策纠错 + 同 run 内 idle yield | 前者；删 `MAX_IDLE_YIELDS`，waist 不判"有没有东西可等" |
| F18 | Yield / 有未完任务的 Done 之后怎么续（§3.2.2） | resume 同一快照 + 注入（`live_run` 保留，step 连续）/ 新 run + 文本历史 | 前者；End 才开新 run；与 F14 合并成一条"run 保活 = 本 run 有未完任务" |
| F19 | 普通 loop 要不要 yield 概念 | 不要（Done 永远是本轮回复，保活靠任务登记）/ 加等价标记 | 不要 |
| F20 | ask-user 一般化为标准异步 function call（§3.2.1）：任务 = 通知系统的 `human.input/v1` TaskMgr 任务，工具改 websdk CLI，xllm 经 F13 任务后端 + F7 通用解析拿到答案，`runner/ask.rs` 全删 | 一般化 / 维持 session 专用路径 | **已裁决：一般化（用户 2026-11-01）**；附带裁决：同一对话里来了普通消息时，开放的提问任务以 `superseded` 撤销 |
| F21 | 删 §3.11 A 类：`XmlBehaviorParser` + 协议提示词、`snapshot_overrides`、`prompt_compose` / `prompt_budget`、`StepResultHook` / `history_inputs`、`HumanPolicy`、`parallel` / `max_observation_bytes`、`llm_task_ids`、`<sendmsg>` 整条 | 删 / 保留 | **已裁决：删（用户 2026-11-01）** |
| F22 | `Observation.tool_result: ToolResultView` 改为不透明 `host: Option<Value>`；`XmlStepRenderer` 随之搬 agent_tool，waist 只留 `StepRenderer` trait | 不透明 / 维持镜像字段 | **已裁决：不透明（用户 2026-11-01）**，agent_tool 信封不进冻结面 |
| F23 | `msg_parser` 搬 libopendan，`prompt_engine` + `xml_util` 搬 agent_tool | 搬 / 留作"L4 共用 utility" | **已裁决：搬（用户 2026-11-01）**，原则 5 waist 不做 I/O |
| F24 | `RefusalKind` → `ProviderFailure::Refused { code: String }`；`ContextOwnerRef` → `owner: Value`；`report_artifacts` 并入 `report_result` | 改 / 维持 | **已裁决：改（用户 2026-11-01）** |

## 6. 实施顺序（F1–F24 全部裁决，用户最终 review 后开工）

| 步 | 内容 | 主要删除面 |
| --- | --- | --- |
| 1 | llm_context：F2 B、F1 + F4 / F16（任务模块与 `background_env` 全删）、F9、F10（`Injection` 不加字段）、F17（`validate` 空决策改 Yield）、§3.10 收缩、§3.11 的 F21 删除 + F22 不透明 host + F24 三处改型、快照 v6；F23 的三个模块搬到目标仓后再删（msg_parser → libopendan，prompt_engine / xml_util → agent_tool） | Outcome 七态→六态；`tests/suspension/pending_tool.rs`（428 行）、`context_derive.rs` 两例、`tests/cancel.rs` 的 Pending 用例删；`behavior.rs` 的 `ToolResults` 用例删 |
| 2 | agent_tool：ToolManager 按 call_id 先找再做（F12）+ `idempotent()`、`task_wait_ms` + Zone TaskMgr 任务后端按 token 可用（F13）、删 `inflight[]` / `ShellMode`（F15）、`SnapshotHook` 迁 `CheckpointHook` 并补 `before_tool_call`、`pending_reason` 改字符串（F8）、`tasks.rs` 迁入（I1）、Zone 解析器通用化（F7，`human.input/v1` 可渲染）；接收 F23 搬来的 `prompt_engine` / `xml_util`，接收 F22 搬来的 `XmlStepRenderer` 并改为从 `Observation.host` 读信封；websdk：`ask-user` CLI 工具（F20，另一仓） | xllm.rs:9111 分支、`fill_suspended_calls`、`settle_previous_executor`、`exec_tracking::materialize_unresolved / persisted_outcome_ids`、`ZoneShellTasks` 的 shell 专用渲染 |
| 3 | libopendan：删 `WaitingRun` / `fill` / `try_fill` / `fill_answer` / `pending_sub_call`；ask-user 一般化（F20：删 `runner/ask.rs`、`xagent_cli` 的 `ask-user`、`WaitingKind::Input`，Turn 规则并入 F18）；call_behavior 走 behavior 触发的 hand-over 路径；task done 经 `route_inputs` 成 `<<task_result>>` 输入；F18 保活与续接（Yield / 有任务的 Done 不清 `live_run`，输入注入同一快照；删 `review_yield` Yield 分支、`WaitingKind::Children` 泛化）；`register_inflight` 删；接收 F23 搬来的 `msg_parser`；reconcile.rs:155 的"已回话"改由 send_message 工具结果判（F21 删 `messages_sent`）；`report_artifacts` 的读取改从 `report_result` 取（F24） | drive.rs:1550-1615 等待段、drive.rs:2046 分支；tests 的 `ask_user`、`crash`、`notify`、`goal_session`、`xagent_cli`、`plan_simple` 重写 |
| 4 | 文档按新分层重写（§4 末条）、TS TODO 回改（Q3 / Q7 / Q8 / T4-2） | |
| 5 | F11（hints / dedup_key）、F6（spec 对账）作为后续单独项（F7 已提前到第 2 步） | |

第 3 步的测试重写是最大成本：libopendan 六个测试文件都碰到挂起语义。每一步结束都要 `cargo test -p llm_context` / `-p agent_tool` / `-p libopendan` 过，第 3 步后在单 OOD 上跑一轮 ask-user、call_behavior、shell 长命令和崩溃恢复的 DV。

## 7. 关联

- [xllm TS 运行时 TODO](./xllm-ts-runtime-todo.md)：第二份实现，本单裁决后要回改。
- [llm_context 长命令 TODO](./llm-context-long-tool-todo.md)：现有任务模型（`RunningTaskResolver`、后台任务提示、shell auto）的出处。
- [X7 挂起与恢复 TODO](./llm-context-x7-suspension-todo.md)：`PendingTool` / `ResumeFill` 的出处。
- [AgentRuntime TODO](./llm-context-agent-runtime-todo.md)：runtime descriptor、run 目录恢复。
- [plan_simple TODO](./opendan-plan-simple-todo.md) §4.4、[Jarvis UI Session 提示词 TODO](./jarvis-chat-prompt-todo.md) §1.2、[chat prompt 先行 TODO](./llm-context-chat-prompt-prereq-todo.md)"以后"节：`<<background_context>>` 注入的出处，以及被 §3.7 撤销的"工具结果带 context"想法。
- 文档：[LLM Context 设计](<../doc/llm_context/LLM Context 设计.md>)、[AgentRuntime 设计](<../doc/llm_context/AgentRuntime 设计.md>)、[AgentToolResult 协议](../doc/llm_context/agent_tool/agent_tool_result_protocol.md)、[xllm Rust SDK](../doc/llm_context/xllm_rust_sdk.md)。

## 8. 实施记录（2026-11-01，未提交）

验证：`cargo test --no-fail-fast -p llm_context -p libopendan -p agent_tool -- --test-threads=1` 与 `cargo test -p opendan` 全过；websdk `ask_user` 跑了 Deno 单测；xllm 的 DV 见 §8.4。agent-session 的 DV（ask-user、call_behavior、shell 长命令、崩溃恢复）移到 [Agent Session TODO](./opendan-agent-session-async-task-todo.md) §3。

### 8.1 各步落点

- **第 1 步 llm_context**：`Observation` 去掉 `Pending`，`Success / Error` 带不透明 `host`；`Outcome` 六态（无 `PendingTool`）；快照 v6，`Suspension { which, estimated_tokens, at_ms }` 只剩上下文超限，`ResumeFill` 无 `ToolResults`；`CheckpointHook` 是唯一钩子（`before_tool_call` / `before_inference` / `validate_report` / `review_decision`），`InferenceHook`、`StepResultHook`、`deps.tasks`、`WorkEvent::MessageSent` 删；`ToolCallCtx { abort, deadline_ms }`；Yield 立即 `Done`（`validate` 不再拒空 step，`decision()` 给 Handover / Yield）；`Refused { code: String }`、`owner: Value`、`report_result.artifacts`；删 `xml_behavior / snapshot_overrides / prompt_compose / prompt_budget`、`sendmsg`、`HumanPolicy`、`history_inputs`、`llm_task_ids`；`msg_parser` → libopendan，`prompt_engine / xml_util / step_record / tasks` → agent_tool（`tasks` 改名 `task_model`）。恢复测试改用 `before_tool_call` 快照。
- **第 2 步 agent_tool**：`result_view.rs`（`ToolResultView::from_host / into_host`）；`pending_reason` 字符串；`AgentTool::idempotent()`；`exec_tracking::in_flight_call` + `XllmToolManager::set_recovering`，重新派发时 shell 先查 `local:shell:<call_id>`、幂等工具重跑、其余 `Unresolved { effect_unknown: true }`；`RuntimeConfig.task_wait_ms`（默认 30 秒、上限 30 分钟），`ShellMode` 私有化为推导值，`tools.shell.mode / wait_ms` 删；RunRecord v7 无 `inflight[]`；`SnapshotHook` 实现 `CheckpointHook`；xllm 的 Yield / 有运行中任务的回复 → `paused`（`last_error.kind = yield / tasks_running`），`xllm --resume [message]` 注入新输入（`XllmRun::inject_input`）；`tasks_zone.rs` 按 schema 通用渲染（F7）；`XLLM_RUN_ID` 进 shell 环境；CLI xllm 有 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 时以 Zone TaskMgr 作任务后端（`run_local_llm::with_zone_tasks`，F13 / F20 条件 2），没有会话时 `t-` 任务对该 run 是 unknown。
- **websdk**（`src/tools/buckyos-agent`）：`ask_user <question> [--option id=label]... [--allow-text] [--expires secs]`，幂等键 `ask:$XLLM_RUN_ID:$XLLM_CALL_ID`，返回 pending 信封（`t-…`，`pending_reason = user_input`）。`baseline.rs` 的 STANDARD_TOOLS 加 `ask_user`（顺带补了 `import_office` / `import_table`，原来就让一条测试失败）。
- **第 3 步 libopendan**：删 `WaitingRun`、`fill / try_fill / fill_answer`、`pending_sub_call`、`register_inflight`、`runner/ask.rs`、`xagent ask-user`、出站提问、`ask_user_timeout_secs`、`WaitingFor.human_tasks`、`UserInput*`、`review_yield` 与 `MAX_IDLE_YIELDS`。F18：`outcome::keep_while_running` + `commit_kept`（run.json `host.extra.kept`，reconcile 重做），worklog 记 `outcome kind = wait`；任务结束经 `poll_watched_tasks` 成 `<<task_result task=… call_id=… status=…>>` 输入（全文 ≤ 16 KB，call_id 取 run 的 `task_calls`）；call_behavior 只登记 `host.extra.sub_call`，段结束时 hand-over，结果以带 `call_id` 的 `<process_result>` 回到调用方批次。jarvis 包：behaviors 的 `agent-session ask-user` 换成 `ask_user` + task_result 语义，`self_check` 去掉 `sendmsg`，`agent.toml` / `opendan_agent_dev.md` 去掉 `ask_user_timeout_secs`，`xllm.md` 补恢复 / paused / `--resume "<message>"`。
- **第 4 步文档**：`doc/llm_context`（LLM Context 设计、behavior loop、readme、prompt 引擎、Agent Actions / Message、AgentRuntime 设计、local 协议、xllm Rust SDK、AgentToolResult 协议与 agent_tool 指南）与 `doc/opendan`（xAgent、Session Directory / Input Protocol、Agent TodoList、架构设计、`doc/sdk` 开发指南）按新分层改写；协议 fixtures 与 JSON Schema 重生成；[TS TODO](./xllm-ts-runtime-todo.md) 回改 L2 行、Q1 / Q3 / Q7 / Q8 / Q9 (e) / T4-2 / T4-3 (ii)。

### 8.2 与本单的偏差（实施中自定，请 review）

1. **`WaitingKind::Input` 保留**（§6 第 3 步说删）：它不再表示提问，只表示会话空闲等用户下一条消息（`WAIT_USER_MSG`），idle 会话和子 session 的 `needs_input` 还在用。
2. **子 session（`session:<sid>`）的等待立即答复**：`SessionTaskResolver.wait` 不在预算内等，直接给当前状态，相当于把 §3.2.1 对 `subctx:` 的规则用到子 session。原因：工作区委托给子 session 时，父 run 在调用内等，子 session 拿不到写者，只能死锁到预算用完。`agent-session wait <sid>` 去掉 `--wait-ms`，`create-worksession --wait` 同；plan_simple 提示词与 `opendan_agent_dev.md` 改成"仍在运行就结束回复，结局是下一条输入"。
3. **新增 `TaskFollowup::Input`**：session 宿主（`HostProtocolFlavor::Session`）的"仍在运行"提示说"结果会作为输入回来，不用等"，不再叫模型 `wait_task` / `agent-session task wait`（否则模型会在 loop 里等，违反原则 8）。
4. **运行中的工作优先于完成策略**：显式完成、没有输入队列的 work session（plan_simple）在子 session / 任务未完时回复，本会判 `missing_end_report` 失败，现在照样保活。
5. **报告校验与工作区交接忽略子 session 的 noted task**：`session:` 也记进 run 的 noted tasks（保活要用），但"任务未完不能结束"只看非 `session:` 的任务（子 session 由 unsettled 检查管）；工作区交接检查忽略已结束的子 session。调用结果本身就是子 session 结局时 `children::mark_heard`，不再重复投递事件。
6. **`<result>null</result>` 与 `<artifacts>` 同给**时按"无结果"处理（并入 `report_result` 后无法区分 null 与缺省）。
7. **"已回话"**（reconcile）只看 `has_report`（F21 删了 `messages_sent`，没有另做 send_message 工具结果判断）。
8. 测试与工具：context_limit 测试共用进程 logger、各测试的 session id 相同，加了文件级串行锁并在每例开始清空；fixtures 的 compact 阈值改为同时满足"场景内 160 / 仓库相对 210"（`002630be0` 把 kmsg 队列名改成哈希后，旧阈值放过了 214 字符的路径）；fixture 17 只保留 UI 会话观察通知的部分（Work 经 libopendan 提问的那一半随 ask-user 删除）；fixture 06 的生成器原来 `pkill` 的模式匹配不到命令行，被杀的 runner 留下的 `sleep 60` 事后把 `exit = 0` 写进 fixture，按 F12 恢复会读到这个结果，改成按命令文本 kill，期望改为"按 call_id 重新派发，执行目录无 exit → Unresolved"。opendan `loader.rs` 删了三条只测旧提问流程的用例（UI Turn 内问答、群聊提问 @owner、Work 提问走通知），提问改由 `ask_user`（`input` 通知 + TaskMgr 任务）承担，对话内的投递由 msg-center 的通知投递决定，本次没验证 MessageHub 侧的呈现。

### 8.3 没做 / 待验证（agent-session 相关的已并入 [Agent Session TODO](./opendan-agent-session-async-task-todo.md) §2）

- 移到 [Agent Session TODO](./opendan-agent-session-async-task-todo.md)：提问撤销（R7）、保活中 UI Turn 的中间回复（R6）、`waiting_for` 的形状（R1）、`<<task_result>>` 的 `failed` 状态（R8）、`remote_node` 滤掉 `XLLM_*`（R9）、agent-session 的 DV 与 `ask_user` 端到端（§3）。
- F11（hints / dedup）、F6（spec 对账）未做（§6 第 5 步）。
- 偏历史 / 方案性质的文档没改：`doc/opendan` 的 Agent Session SDK 实现计划、NewOpenDANRuntime、Agent配置改进，`doc/llm_context` 的 why behavior loop、LLM Context Switch 样例、improve-behavior-report，以及 behavior loop 文档 §10"实施顺序（历史记录）"。

### 8.4 xllm DV（2026-11-01，本机 DV Zone，buckyos provider → AICC `llm.chat`）

二进制 `cargo build -p agent_tool_cli_dev` → `/tmp/dev-cache-root/cargo-target/debug/agent_tool xllm …`（`</dev/null`），每个场景一个工作目录，函数调用场景用本地 `.llm_context` 关掉全局的 `tools2actions`。

| 场景 | 结果 |
| --- | --- |
| T1 问答（behavior loop，全局配置） | completed，RunRecord v7 |
| T2 function call 写文件 + shell | 工具调用前有 checkpoint（快照 v6，`tool_batch.remaining` 含该调用）；全局 `result_format: result.report` 对纯文本回答报 exit 6，与本单无关 |
| T3 `task_wait_ms: 2000` + `sleep 8` | 2 秒转任务 → 模型 `wait_task` → completed（模型若自己传 `wait_ms` 会改成在调用内等，这是 shell 的设计） |
| T4 任务未完时回复 | `paused (tasks_running)`，exit 3，给出 resume 提示；`--resume --run <id> "<message>"` 注入同一快照继续 |
| T4b 暂停后任务自己结束再 resume | **发现缺陷并已修**：暂停后 CLI 退出时进程内任务被进程组守卫杀掉，"resume once they ended" 永远等不到。修法：xllm 因任务暂停时 `InProcessTaskManager::release_running()` 把任务交给 OS（`LocalTask::release`，shell 任务解除进程组守卫），执行目录记下结局；新单测 `released_shell_tasks_outlive_their_task_manager`（去掉 release 会失败）。修后 resume 经 `get_task_state` 读到结果 → completed |
| T5 behavior loop：shell 动作转任务 → 只带 report 的 Yield | `paused (yield)`；任务结束后 `--resume "<message>"` → 读到结果 → `report end="true"` completed；XML 动作的 call_id 跨 step 唯一 |
| T6 shell 执行中 SIGKILL xllm，命令跑完后 `--resume` | 按 call_id 从执行目录读到 exit 0 的结局，不重跑（`ran.log` 一行）→ completed |
| T6b SIGKILL 后立即 `--resume` | `[unresolved: result unknown]`，不重跑，模型如实说明 → completed |
| T7 `ask_user`（用户在 DV 上点卡片） | 通过：用 devtest 密码向 verify-hub 换用户会话（appid `control-panel`）作 `BUCKYOS_APPCLIENT_SESSION_TOKEN`，websdk `ask_user` 经包装脚本放进 PATH。shell 调用返回 pending（`t-…`、`nt-…`、`pending_reason: user_input`）→ CLI 以 Zone TaskMgr 作任务后端，预算内读到"仍在运行" → 模型 `wait_task(600000)` → 用户在 MessageHub 选"能用" → `human.input` 任务完成，结果 `{action: answer, values: {option: yes}, submitted_by: devtest}` 原样作为 JSON 正文回来（F7 通用规则）→ completed。OpenDAN 注入的 App 会话（收件人是 App owner）这条路仍在 Agent Session TODO T1 |

附带：`xllm --help` 的 `--tools` 说明里工具名 `exec` 改为 `shell`。一次性 xllm 的协议开场白仍写"没有后续对话、不要向用户提问"，所以 behavior Yield 在 xllm 里只用于等自己启动的任务（T5），不用于提问。

