# 简单 Work Session 全异步化 TODO（agent-session 重构第一刀）

日期：2026-11-01

状态：**待 review / 待裁决。** 用户 2026-11-01 口述思路（§1），Claude 对照源码写成本单；同日用户纠正：Work Session 是**多 Turn**（assistant message 关闭本 Turn、事件 Input 打开下一个 Turn），第一版写成"一个 Turn 内多 Round"是错的，已按纠正改写 §0 / §3.3 / §3.4 / Q4 / Q9 / Q11 / Q12 / B1 / B3 / B8；同日用户补充四点（§1 第 5–8 条）：timer 是 work session 的自动订阅、work_simple.toml 要加输入处理模板、F11 hints / dedup 在本单做、function loop 下工具结果后经 hook 注入 background context，写成 §3.7–§3.9、Q13–Q14、A7、B11–B14；同日用户裁决 Q5：Session 宿主不再向 LLM 暴露 wait-task（订阅代替等待，省一次推理），见 §1 第 9 条、§3.10；**2026-11-01 晚用户把裁决写回 §4（Q1 统一进程内、Q3 30 s 可配且背景只带变化、Q10 整体改但本单只做 work_simple、Q12 / Q13 按建议、Q14 统一为推理前注入），未置评的 Q 按建议值执行；Claude 据此回写 §3 / §5–§7，并把 [Agent Session 全异步 task 化 TODO](./opendan-agent-session-async-task-todo.md) 的剩余条目并入 §7（本单是它的超集，原单只留索引）。可开工。** **2026-11-01 晚已按本单实施（§5 A / B / C / D，B15 与 §6 DV 除外），实施记录与偏差见 §9。** 承接 [llm_context 冻结 TODO](./llm-context-freeze-todo.md)（loop 里不允许等待、任何等待都是任务、Yield / Done 立即结束 run、结局以输入回来）和 [Agent Session 全异步 task 化 TODO](./opendan-agent-session-async-task-todo.md)（原 R1–R10 / T1–T5 全部并入本单：R1 / R3 / R4 / R6-Work / R8 / R9 / R10 / T1 / T3 / T5 在 §3–§6，R2 / R5 / R7 / R6-UI / T2 / T4 在 §7 作为后续刀）。

范围（Q10 裁决）：**本单只改 `work_simple`**——class `work` 的 function_call 会话里用 `work_simple.toml` 的那一种（含 plan_simple 派出的子会话，`todos.rs::SERIAL_SESSION_BEHAVIOR = work_simple`）；测试与 DV 都围绕 work_simple。class 级改动（多 Turn、explicit_report、timer 订阅）对 `explore` 与 plan_simple 父会话同样生效，但它们的提示词 / 模板 / 等待形状留到下一刀（§7），这一刀的问题都搞定了再动 plan_simple。UI / group / goal 会话不动。

路径均相对 `buckyos/`。实施前重新读 `AGENTS.md` 与当前源码；工作区有大量未提交改动（冻结实施、agent-session 重构前的状态），不得覆盖。按 [llm_context 修改先行](./llm-context-freeze-todo.md) 的规则，§5 的 A 部分（agent_tool / runtime）先做、单独可测，B 部分（libopendan）只依赖 A 的契约。

## 0. 术语（本单内固定）

| 词 | 含义 | 出处 |
| --- | --- | --- |
| Turn | 会话对外的一次工作单元：一批输入打开它，模型的 assistant message（回复）或 `report` 关闭它，Turn 有自己的 Turn task 与结果。**现状** class `work` 是 `Turns::One`（整个会话一个 Turn，Turn 关闭会话结束）；**本单改为多 Turn**：Work Session 的每条回复关闭一个 Turn，事件 Input 打开下一个，会话只由 `report(is_end=true)` 结束。这是新架构从结果上最明显的变化（用户 2026-11-01） | `template.rs:138`、Round/Step/Turn 统一 |
| Round | function_call loop 的一次"推理 + 一批工具调用 + 结果配对" | llm_context |
| run | 一段 LLMContext 的执行；Turn 内可有多个 run 段（compress、hand-over），也可以一个 run 停住（Paused / Waiting）再被输入续接 | 冻结 TODO §3.2.2 F18 |
| 任务 | 一次工具调用交出去、在调用返回后继续跑的工作，有 task_id：`local:shell:<call_id>`（进程内）、`t-…`（Zone TaskMgr）、`session:<sid>`（子 session） | agent_tool `task_model.rs` |
| 事件 | 会话的一条输入（`SessionInput::Event`），有 `source = kind:id`、`event` 名。**Active**：进入输入批次、触发推理。**Semi**：并进 `pending_events`，只在下一次推理前以半订阅快照呈现，自己不触发 | `protocol/config.rs` SubscriptionMode、`runner/inputs.rs` 表 |
| 等待（原"保活"） | 本 run 启动的工作还没完时 Turn 以回复关闭，会话转 Waiting 等事件；上下文是否留在同一个 run（Paused、下一 Turn resume 同一快照）还是下一 Turn 开新 run 用文本历史，见 Q12。**现状**的 `keep_while_running` 是"Turn 不关、run 不结束"，本单要改掉 | `outcome.rs::keep_while_running` |

"多 Turn"按字面：一个 Work Session 可以有多个 Turn。典型轨迹：Turn 1 由任务输入打开，模型派出几条 build，回复"所有任务还在跑，目前……"关闭 Turn 1；task done / tick 事件作为 Input 打开 Turn 2，模型回复或 `report`；直到某个 Turn 以 `report(is_end=true)` 结束会话。每个 Turn 的回复都是外界能看到的进度（Turn task 结果、worklog），而不是一个 Turn 里憋到最后。

## 1. 用户思路（2026-11-01 原文要点 → 本单条目）

1. shell 调用在 25 秒内返回视作同步，否则自动转异步返回。按道理不用每个 shell 调用都进 task-mgr 也能实现进程内异步。→ §3.1、Q1、Q2
2. 转异步后，会话自动订阅该任务的事件：只有完成（成功 / 失败）是 Active 事件，其它变化是半订阅（background context）。→ §3.2、Q8
3. 在此机制下，一个 Work Session 同时调了几个 shell（比如 rust build）都转异步后，实现多 Turn：
   - 3.1 用 `report` 调用明确结束 Work Session，而不是 assistant message。→ §3.3、Q4、Q10
   - 3.2 除了 task 事件，还有自动创建的 timer 事件（30 秒一次）推进推理；timer 事件里触发 background context，看到各任务 update 的状态。→ §3.4、Q3
   - 3.3 task done 事件应包含全部 Result（类似同步调用的返回），免得再走一次 get task result。→ §3.5、Q7
4. runtime 提供的 4 个标准工具都带可选的"意图参数"，在标准 function loop 下也能观测到一定的意图。→ §3.6、Q6
5. timer 事件是 agent work session **自动订阅**的，需要新增。→ §3.4、B11
6. 这个 work session 用的是 `apps/jarvis_agent/agent/behaviors/work_simple.toml` 的提示词，需要改造：增加 input 处理模板。→ §3.7、B12
7. llm_context 里留给 agent session 处理的能力（AgentToolResult 的 hint）也在这里处理。→ §3.8、A7、B13
8. 标准 agent tool result 进推理前，通过 hook 加入 background context（用户记得 function loop 之前不支持）。→ §3.9、B14
9. 另一个关键的可观察结果：不再需要把 wait-task 暴露给 LLM 调用，agent session 自动订阅事件搞定，省一次推理。→ §3.10、Q5（已裁决）、B4

## 2. 现状（2026-11-01 源码核对；实施前复核）

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| 调用内等待预算 | `RuntimeConfig.task_wait_ms`，缺省 `DEFAULT_TASK_WAIT_MS = 30_000`，上限 30 min；shell 的 `wait_ms` 由它来（`ShellSettings::from_config`）；`wait_ms >= timeout_ms` 时等到命令结束。libopendan 与 Jarvis 包都没设 `task_wait_ms`，用的是缺省 30 s | `agent_tool/src/task_model.rs:24-27`、`runtime/mod.rs:49-91`、`xllm.rs:549-610` |
| shell 转任务 | 到 `wait_ms` 未结束 → `detach_into_task` → `ShellTaskManager::take_over`，结果是一条 status=Success、带 `task_id`、`still_running: true` 的信封 + `TaskFollowup` 提示；schema 仍暴露 `wait_ms` 参数并写"Raise wait_ms for commands you expect to take long (builds, tests)" | `agent_tool/src/llm_bash.rs:1645-1700`、`:1785`、`:1839` |
| 任务后端 | opendan 进程一律 `ZoneShellTasks`（每个转出的 shell 在 Zone TaskMgr 建 `tool.shell/v1` 任务、挂 Turn task 下、每任务一个 5 s watcher 写进度），TaskMgr 不可达才退回进程内 `InProcessTaskManager`（`local:`）。xllm CLI 有 App 会话时同样用 Zone，没有则 `local:`。2026-10-18 裁决"Jarvis 相关 Session 默认 TaskMgr"的起因（run 重建后新 `InProcessTaskManager` 不认旧任务）已由进程级 `XllmDeps.local_tasks` 解决 | `opendan/src/tasks.rs:281`、`main.rs:304`、`agent_tool/src/tasks_zone.rs:177-240`、`tasks.rs` 头注释 |
| 进程内任务的生命周期 | `InProcessTaskManager::release_running` 只有 xllm 暂停时调用（任务交给 OS 继续跑）；opendan 的 agent service 退出时没有这一步，`local:` 任务随进程组结束；重启后只能从执行目录 `exec/<call_id>/exit` 回填（R10） | `agent_tool/src/tasks.rs:351`、`runtime/mod.rs:718 describe_interrupted` |
| 任务登记 | 工具结果带 `task_id` → `SessionToolManager` 的 `seen_tasks` + `run.note_task`（run.json `host.extra.{tasks, task_calls}`）；run 结束判保活时写 `SessionState.watched_tasks` 与 `waiting_for { kind: Tool, refs }` | `lib_opendan/src/runner/tools.rs:138`、`session/runs.rs:362`、`outcome.rs:860-880`、`:1280-1305` |
| 任务结局进会话 | `poll_watched_tasks` 在每次取输入时（保活 run 的空转循环每 `poll_interval = 5 s` 醒一次）查 resolver：Running **跳过**；Finished / Unknown → 没有显式订阅就当 Active 输入事件 `source = task:<id>`，正文 `task_result_text` = `task_state_observation` 的文本（≤ 16 KB），`status` 只有 `finished` / `unknown`（失败写在正文，R8），`call_id` 从 run 的 `task_calls` 取；有显式 Semi 订阅才并进 `pending_events` | `runner/drive.rs:1012-1180`、`:1590`、`:1850-1880`、`runner/mod.rs:217` |
| 什么能触发推理 | `triggered = !picked.is_empty() || bootstrap || 内部续接`；输入批次之外的注入只是观察 | `drive.rs:1638` |
| timer 事件源 | `EventSource.kind` 允许 `timer`，`SubscriptionSource::Timer { name }` 可配，但**没有任何代码产生 timer 事件**；唯一引用是 self_check 模板的 `_self_check_timer` 订阅声明（无人投递） | `protocol/config.rs:864`、`protocol/input.rs:105`、`template.rs:177` |
| 半订阅快照 | 非 UI 会话：`select_snapshot` → `render_semi_subscription_snapshot`，随受控输入批次一起提交（任何触发推理的批次都带）；UI 会话走 background lines。class `work` 的 `observe = ObserveScope::Events` | `drive.rs:1183`、`:1931`、`template.rs:143` |
| 续接 | run 保活后状态 Waiting，`live_run` 仍在；下一批输入走 `open_state_live_run` resume 同一快照 + `commit_input_batch` 注入（F18 已实施，`host.extra.kept`） | `drive.rs:1982`、`outcome.rs::commit_kept` |
| 结束规则 | class `work`：`Turns::One`、`WaitPolicy::FinishFailed`、`InputChannel::None`、completion 缺省 **natural**（只有 `[session.plan_simple]` 是 `explicit_report`）。explicit_report 下：`report(is_end=true)` 通过校验 → Turn Completed；纯文本 Done + 无输入队列 → `missing_end_report` 失败，除非 `keep_while_running` 发现本 run 有在跑的任务 / 子 session（保活优先，R4）。本单把这整套改成多 Turn（§3.3） | `template.rs:138`、`apps/jarvis_agent/agent/agent.toml:61-73`、`outcome.rs:455-470`、`:904-960` |
| report 工具 | 宿主工具 `report { report, artifacts, result, is_end }`；`validate`：`is_end` 时本 run 的非 `session:` noted task 未完 → 拒绝 | `runner/reports.rs:34`、`:87`、`:183-230` |
| 中间文本 | 保活时那条"已启动，完成后汇报"的 assistant 文本：Work 会话无出站通道，只进历史；Turn task 进度文案 `status_of` 只有 "running shell" / "waiting for a tool" 这类 | `runner/turn_task.rs:160-190` |
| intent | 只有 behavior `[capabilities] call_intent = true`（plan_simple）时，`SessionToolManager` 用 `with_intent` 把 `intent` 加进**每个**工具 schema 的 `required`，`without_intent` 剥离后再交工具；原始调用留在快照 / worklog。agent_tool 不认识 `intent`；Turn task 文案、worklog 摘要都没用它 | `runner/tools.rs:50-110`、`:239`、`:325`、`runner/live.rs:534` |
| runtime 标准工具 | `bash_core` 组 = `read`、`write_file`、`edit_file`、`shell`；有 `task_cli`（opendan 给 `agent-session task`）时 `wait_task / get_task_state / cancel_task` 不再是原生工具，走 shell 命令 | `agent_tool/src/xllm.rs:12730-12760`、`:3834` |
| 任务 CLI | `agent-session task state|wait|cancel <task_id> [--wait-ms]`，经 task bridge 到本 run 的 resolver；Session 宿主的"仍在运行"提示已是 `TaskFollowup::Input`（结果作为输入回来，不叫模型等） | `xagent_cli.rs:799-830`、冻结 TODO §8.2 偏差 3 |
| 任务状态模型 | `TaskState::Running { brief, elapsed_ms, output_tail, cancellable }` / `Finished(TaskResult { success, output, tool_result: Option<ToolResultView> })` / `Unknown { reason }` | `agent_tool/src/task_model.rs:31-59` |

## 3. 目标形状

### 3.1 调用内等待 25 秒

每个工具调用都是异步的（冻结原则 2）；shell 在调用内最多等 **25 s**，结束了就是同步结果，没结束就转任务返回"仍在运行 task X"。25 s 是 Runtime 策略，不进 `runtime_id`，不进冻结面。

- 落点：改共享缺省 `DEFAULT_TASK_WAIT_MS = 25_000`（Q2 按建议）。
- Session 宿主**隐藏** shell 的 `wait_ms` 参数并删掉"Raise wait_ms for builds"那句（Q2b）：有事件源的会话里，模型不该在调用前决定等多久（[渐进式披露原则](./llm-context-long-tool-todo.md) §4：执行方式由配置定）。xllm 保留。
- 任务后端**统一进程内**（Q1 裁决）：转出去的 shell 留在 `InProcessTaskManager`（`local:shell:<call_id>`），由会话自己跟（§3.2）。只有命令本身的实现就是 krpc、服务端返回任务 id 的（`ask_user` 的 `human.input/v1`、AICC 之类）才是进程外任务（`t-`），Zone 只作为这些任务的**解析器**存在，不再接管 shell。xllm CLI 今天已经是这个形状（`run_local_llm.rs::with_zone_tasks` 只设 `buckyos_tasks` 解析器，不设 `shell_tasks`），opendan 对齐它；不加 `task_backend` 旋钮。代价与补救：agent service 退出时 `release_running`（命令交给 OS 继续跑），重启后 `local:` 任务由执行目录 `exit` 回填 Finished，没有 `exit` 则 Unknown "executor restarted; the command may still be running, check <exec_dir>"，模型决定；MessageHub 的 Turn task 下看不到子命令；stop 对任务的传导由会话自己做（现状已是）。

### 3.2 自动订阅：完成是事件，其它是背景

工具结果一带 `task_id`，会话就对这个任务有一份**隐式订阅**（runner 内置规则：任务 id 是运行时才知道的，不写进 `subscriptions` 配置；timer 不同，它是 class `work` 模板里的固定隐式订阅，见 §3.4）：

| 任务变化 | 模式 | 进会话的形状 |
| --- | --- | --- |
| Finished（成功 / 失败）、Unknown | Active | 输入事件 `<<task_result task=… call_id=… status=finished|failed|unknown>>`，正文是完整结果（§3.5），触发一次 Round |
| Running 且自上次呈现以来有**新输出**（Q8） | 半订阅（background context） | 不造事件、不进 `pending_events`：推理前的宿主 hook（§3.9）直接读 resolver 的当前状态，把**增量**（新增的输出、elapsed）作为 `<<background_context>>` 的任务行注入；自己不触发推理 |
| Running 且无新输出 | — | 不出现 |

"半订阅"在这里的实现就是 §3.9 的推理前注入（Q14 裁决：背景统一为推理前注入），`poll_watched_tasks` 对 Running 仍然跳过，只管结局。每任务记 `shown_output_len`（已呈现到的输出字节数），下次只给 `output[shown..]`，上限 2 KB，超出只留末尾并标注省略——用户要求"不需要带上所有 shell output，有变化的部分就好"。子 session（`session:<sid>`）的结局仍走 `poll_children`，不改。

### 3.3 多 Turn 的结束规则（class `work` 一律 explicit_report、Turns::Unbounded）

把 `[session.work]` 设为 `completion = "explicit_report"`（Q10），class `work` 的模板从 `Turns::One` 改为 `Turns::Unbounded`。于是回复文本关闭的是 **Turn**，不是会话；**只有 `report(is_end=true)` 结束 Work Session**。一次推理返回后的裁决（`outcome.rs::decide_end` + `keep_while_running` 合并成一张表，两处补丁式判断改为按表实现）：

| Round 的结束形状 | 本 run 在跟的任务 / 子 session | 处置 |
| --- | --- | --- |
| 有工具调用 | — | 原生 loop 继续，不到这里 |
| `report(is_end=true)` 通过校验 | 无 | Turn Completed，会话 finished（现状） |
| `report(is_end=true)` | 有未完 | 校验拒绝（现状）：提示先 `agent-session task cancel` 或等 `<<task_result>>`；不自动取消 |
| `report(is_end=false)` 且无工具调用 | 有未完 | 进度报告记录；Turn Completed（报告文本即回复）；会话 Waiting 等事件 |
| 纯文本 Done（无 report） | 有未完 | **Turn Completed，文本就是本 Turn 的回复**（"所有任务还在跑，目前……"），写进 Turn task 结果与 worklog（Q9）；会话 Waiting 等事件 |
| 纯文本 Done 或 `report(is_end=false)` | 无 | Q4：注入一次纠正输入"nothing is running; finish with `report(is_end=true)` or start the next step"，再来一次 → `missing_end_report` 失败（现状的失败语保留，只说一次） |
| 事件到达：`<<task_result>>`、timer tick、子 session 结束 | — | 作为输入批次**打开新 Turn**；上下文续接方式见 Q12 |
| `stop` 控制 | — | 取消可取消的在跟任务，Turn 以 stop 结束（现状） |

### 3.4 timer tick

timer 是 work session **自动订阅**的（用户 2026-11-01）：class `work` 的模板加一条隐式订阅 `Subscription { id: "_task_watch_timer", mode: Active, source: Timer { name: "task_watch" } }`（与 self_check 的 `_self_check_timer` 同款，`template.rs:177`），`spec.subscriptions` 里能看到它，`agent-session subscribe` 也能对别的 session 加同类订阅。今天 `timer:` 只有源类型没有产生者，要**新增 timer 产生者**：会话 Waiting 期间（有在跟的任务 / 子 session）runner 每 **30 s** 为每条 Active 的 `timer:` 订阅产生一条输入事件 `source = timer:task_watch, event = tick`（与 `poll_watched_tasks` 一样是合成输入，不经队列；匹配走现有 `subscription_for("timer", name, "tick")`，没有订阅就不产生）。tick 打开新 Turn，推理前 hook 注入 `<<background_context>>`（各任务的增量，§3.2 / §3.9），所以模型看到的是：tick 一行 + 各任务的新输出。这就是用户说的"timer 事件里触发 background context"。

**不门控**（Q3 裁决）：目标是每 30 s 让用户感觉到一次反馈——每个 tick 打开一个 Turn，模型回复一句现状关闭它，MessageHub 按 Turn 看到进度。间隔可配置：`agent.toml [session.work] tick_secs = 30`（进 `SessionPolicy`，会话创建时冻结；0 = 不产生 tick）。省推理的手段是调大间隔，不是门控。成本写明：一个 10 分钟的 build 约 20 个 Turn，每个 Turn 一次推理，背景只带增量所以单次增长小；上下文增长靠 ContextLimit 压缩。

状态：`SessionState.task_watch { last_tick_ms, shown: BTreeMap<task_id, shown_output_len> }`，随 state.json 提交；崩溃恢复后从头计。tick 不改 `waiting_for`。

### 3.5 task done 事件带完整结果

`<<task_result>>` 的正文必须和同步返回时模型看到的一样：同一个 AgentToolResult 信封经同一渲染器（status、exit code、输出 tail、执行目录、用时），外加 `call_id` 与原调用的命令 / intent 摘要，让模型不用再调 `agent-session task state`。`status` 加 `failed`（R8）。`TaskResult.tool_result: Option<ToolResultView>` 已有，shell 任务结束时填上即可；文本上限 16 KB 保留，截断时留"用 task id 取全文"的提示。

### 3.6 标准工具的可选意图参数

runtime 的 4 个标准工具（`read`、`write_file`、`edit_file`、`shell`）schema 里加**可选** `intent`（一句话：为什么现在调、期望得到什么）。`IntentPolicy { Off, Optional, Required }`：
- agent_tool 侧实现剥离与记录，`bash_core` 组默认 Optional；
- libopendan 的 `[capabilities] call_intent = true` 映射为 Required 且作用于全部工具（plan_simple 现状不变）；
- `intent` 不传给工具实现；原始调用（含 intent）留在快照 / worklog；宿主把它用在 Turn task 进度文案（Q6）。

### 3.7 work_simple.toml 的改造：提示词 + 输入处理模板

现状：`work_simple.toml` 只有 `[prompt] system` 和 `[capabilities]`，没有 `[input]`；输入渲染全靠 libopendan 内置模板（`assembler.rs::render_input`，`HOOK_ON_INIT / HOOK_ON_CONTEXT_SWITCH / on_input` 三个钩子，`InputTemplates` 还有 `semi_subscription_snapshot`）。多 Turn 之后 Work 会话会收到四类输入，每类都要让模型一眼知道"这是什么、该做什么"：

| 输入 | 现状渲染 | 本单要的模板 |
| --- | --- | --- |
| 首条任务输入（on_init / 第一条 on_input） | 内置 | 保留；系统提示词补"长命令会继续为任务，结局以 `<<task_result>>` 回来打开下一个 Turn；回复一句现状即可关闭本 Turn；只有 `report(is_end=true)` 结束会话" |
| `<<task_result task=… call_id=… status=…>>` | `event_view` 通用事件行 + 正文 | `on_input` 模板里为 `source.kind == "task"` 的事件给专门段落：对应哪次调用（命令 / intent 摘要）、status、完整结果（§3.5）；末尾一句"其余任务见上方 background；都结束了就 `report`" |
| `timer:task_watch` tick | 无 | 同一 `on_input` 模板的 timer 分支："距上次 Turn 过了 N 秒，还有 M 个任务在跑（状态见半订阅快照）；要么继续别的工作，要么回复一句现状关闭本 Turn，要么 `agent-session task cancel`" |
| 任务增量（background context） | 只有 plan_simple 的 Todo 行 | 推理前注入的任务行（§3.9）：每行 `task_id · 命令摘要 · elapsed · 新输出（增量）`；由 `observe.rs` 渲染，不是 `[input]` 模板，但措辞要与模板一致 |
| nudge（Q4） | 无 | `on_input` 的 `system:runner` 分支：一句"nothing is running; finish with `report(is_end=true)` or start the next step" |

模板用 `[input]` 的 `on_input`（`semi_subscription_snapshot` 留给真正的 Semi 总线事件，本单不用；渲染器 `assembler.rs::render_template`，变量按 `render_input` 现有 `vars`；实施时核对 `vars` 里事件的字段名，缺 `source.kind` / `call_id` / 正文就补）。写法参考 `chat_route.toml` 的 `[input]`（UI 模板）与 `doc/opendan/OpenDAN Agent配置开发指导.md`。`explore.toml` 下一刀再加（§7）。

### 3.8 AgentToolResult 的 hints / dedup（冻结 TODO F11，§3.8 的形状，在本单做）

冻结时留给 agent session 的能力，现在接上：

- **协议**：AgentToolResult 加可选 `hints: [{ text, once? }]` 与 `dedup: { key, hint }`（同主版本可选字段）。`once` 是 session 级一次性键（`<tool>.<topic>[:<version>]`）；`dedup.key` 相同且本 run 历史里已出现过 → `output` / `detail` 整体换成 `dedup.hint`。
- **落点全在 agent_tool**（llm_context 看不见信封字段）：`XllmToolManager::call_tool` 做替换与渲染；渲染成固定结构，两种 loop 同形：

  ```
  <output>…</output>
  <runtime_hints>
  - …
  </runtime_hints>
  ```

  behavior 模式由 `XmlStepRenderer` 出独立子元素，function call 模式 `content` 字符串用同一对标记；hint 不得混进 output。
- **账本是宿主依赖**：`HintLedger { seen(ns, key), mark(ns, key) }`，命名空间 `once` / `dedup`，经 `XllmDeps` 注入，缺省内存实现。`once` 按 session 存（libopendan `SessionState.hint_ledger`，与 `background_shown` 同款；xllm 存 run.json）；`dedup` 随 run，历史重写（压缩 `RewrittenHistory` / `RewrittenSteps`）或派生子 context 时清空。都不进 llm_context 快照。
- **本单用到的 hint**（Q13）：shell 转任务那段 `TaskFollowup` 提示改成 `hints[{text, once: "shell.task_followup"}]`（现在每次转任务都重复一整段）；`read` / `read_file` 的"内容未变"改成 `dedup`（替代 ReadFile 特例）；`<<task_result>>` 正文如果也经信封渲染，可带 `hints[{ "都结束了就 report" , once }]`。cmd_skills 经 bash 返回的 envelope 自动覆盖。

### 3.9 function loop 下工具结果后的 background context（hook 注入）

用户记得 function loop 之前不支持；**现状已支持**：plan_simple（2026-10-25）起 `CheckpointHook::before_inference` 可返回 `Injection`，libopendan 的 `SessionCheckpointHook` 在批次结果配对完之后、下一次推理前注入一条 `<<background_context>>` User 消息（`runner/observe.rs`，由 behavior `[input] background = "after_tools"` 打开，只 plan_simple 开了）。这就是冻结 TODO §3.7 的"推理前注入是唯一上下文原语"，持久形态（进历史、进快照，F16 决定不要临时形态）。llm_context 早先那套 `<background_tasks>` 临时提示已随 F16 删除，不复活。

**Q14 裁决：background context 统一为"每次推理前注入"**，与 [事件订阅文档](../doc/opendan/Agent%20Session的事件订阅.md) 写的目标"半订阅默认每次推理前观察"一致。本单要做的：

- `[input] background` 加新值 `"before_inference"`：hook 在**每次**推理前（Turn 的第一次推理、每批工具结果之后）都检查候选行，有变化的行就注入一条 `<<background_context>>`，没有就不注入；`"after_tools"` 保留原义给 plan_simple（下一刀统一）。`work_simple` 用 `before_inference`。
- `observe.rs` 的候选行从"TodoList + 子 session"扩成通用：**在跟任务**（每任务一行：id、命令摘要、elapsed、自上次呈现以来的新输出，§3.2 的增量规则）、子 session（已有）、Todo（plan_simple 才有）；`HostMeta.observed` 的 digest 去重照旧。
- 任务的 Running 变化只走这一条路；`pending_events` / `semi_subscription_snapshot` 留给真正的总线 Semi 事件，本单不往里放任务更新。Turn 内（工具结果之后）与 Turn 边界（tick / task_result 打开新 Turn）的呈现因此是同一段代码、同一种文案。
- 注入位置：紧随本次输入批次的消息之后、推理之前（`Injection` 现有放置规则：fc 模式追加在 accumulated 末尾；第一步之前追加到 `request.input`），实施时核对 Turn 第一次推理时 hook 的调用顺序（`commit_input_batch` 注入输入 → `before_inference`）。

### 3.10 两个可观察结果（验收时就看这两条）

1. **多 Turn 的进度回复。** Work Session 不再憋到最后：派出长任务后回复"所有任务还在跑，目前……"关闭 Turn，MessageHub / 父会话按 Turn 看到进度；事件打开下一个 Turn；`report(is_end=true)` 结束会话（§3.3）。
2. **LLM 不再调 wait-task。** 现状：shell 转任务后，模型要再发一次 `agent-session task wait <id>`（或 xllm 的 `wait_task`）才能拿到结果，这是一次额外推理，而且是在 loop 里等（违反冻结原则 8）。本单之后：会话自动订阅任务事件，结局以 `<<task_result>>` 打开下一个 Turn，正文就是完整结果（§3.5），**省掉这一次推理**。所以 Session 宿主对模型只暴露 `agent-session task state | cancel`：`wait` 从 CLI 用法行、task bridge（`TaskOp::Wait` 对 Session 宿主拒绝并提示"its result comes back to you as input"）、提示词、`TaskFollowup` 文本里全部去掉；`state` 保留给"想看一眼输出 tail"的场景，平时由 background / 半订阅快照代劳。xllm 单 run 没有事件源，`wait_task` 不变。

## 4. 裁决记录（2026-11-01 用户写回；未置评的按建议值）

| Q | 裁决 | 落点 |
| --- | --- | --- |
| Q1 任务后端 | **统一进程内，除非命令本身的实现就是 krpc 才走进程外**（用户原话）。推翻 2026-10-18"Jarvis 默认 TaskMgr"；不加 `task_backend` 旋钮；Zone 只作 `t-` 任务解析器 | §3.1、A3、B5 |
| Q2 25 s 落点 | 按建议：共享常量 | A1 |
| Q2b Session 宿主隐藏 shell `wait_ms` | 按建议 | A2 |
| Q3 tick | **30 s 可配置，目标是 30 s 让用户感觉到一次反馈；background context 不带全部 shell output，只带有变化的部分**（用户原话）。不门控 | §3.2、§3.4、B2、B3 |
| Q4 无任务的纯文本 Done | 按建议：一次 nudge 后失败 | B1 |
| Q5 `task wait` | 去掉（用户 11-01 裁决） | §3.10、B4 |
| Q6 intent 呈现 | 按建议：Turn task 文案 + worklog | B6 |
| Q7 `task_result.status` | 按建议：`finished / failed / unknown` | A4、B2 |
| Q8 "有变化" | 按用户 Q3 的补充改为**输出增量**：自上次呈现以来有新输出才算变化，只呈现新增部分 | §3.2、B2 |
| Q9 中间 Turn 回复 | 按建议：Turn 结果 + worklog，不投递 | B7 |
| Q10 class work 整体 explicit_report | **整体修改。本单先改 work_simple（测试围绕 work_simple），问题都搞定了下一步才到 plan_simple**（用户原话） | B8、§7 |
| Q11 工具预算 | 随 Q12 同一 run：按 Turn 重置 | B9 |
| Q12 Turn 间上下文 | **按建议**：同一 run Paused，解耦"Turn 关闭"与"run 结束" | B1 |
| Q13 接哪些 hint | **机制验证，基于相关测试覆盖就好；后续有 TODO 覆盖所有 CLI 的重新设计**（用户原话）。做 §3.8 列的三处，够测试覆盖即可 | A7、B13 |
| Q14 两条呈现路 | **background context 统一为推理前注入**（用户提问，Claude 按其意图回写 §3.9：`before_inference` 模式，任务更新不进 `pending_events`） | §3.2、§3.9、B14 |

## 5. 实施清单

### A. runtime 层（agent_tool，先行；每条独立可测）

- **A1 等待缺省 25 s**（Q2）：`task_model.rs::DEFAULT_TASK_WAIT_MS`；所有引用它的提示词文本（`tasks.rs:793`、`llm_bash.rs:1785`）由常量生成，确认无硬编码 "30"。
- **A2 宿主 flavor 控制 shell 的 `wait_ms` 参数**（Q2b）：`ShellTool` 按 `HostProtocolFlavor`（或新 `ShellSettings.expose_wait_ms`）决定 schema 是否含 `wait_ms`，描述分两版（Session 版说"long commands continue as a task; its result comes back to you as input"）。
- **A3 shell 任务统一进程内**（Q1）：`ShellTool.with_tasks` 只接 `InProcessTaskManager`；`XllmDeps.shell_tasks` / `shell_task_context` 删除，`ShellTaskManager` trait 若只剩一个实现就内联；`tasks_zone.rs` 删掉 shell 接管部分（`hand_over` / `take_over`、watcher、`recover`、`ShellTaskCreate`），只保留 `t-` 任务的解析器（`resolver` / `briefs` / F7 通用渲染，改名 `ZoneTaskResolver`）；`InProcessTaskManager::release_running` 供宿主退出时调用；`local:` 任务重启后的解析统一在 `CompositeTaskResolver`：执行目录有 `exit` → Finished（结果从 `stdout/stderr` tail 组装，与 `describe_interrupted` 共用读取函数），否则 Unknown（文本见 Q1）。
- **A4 完整结果进 TaskResult**（§3.5）：`ShellTask` 结束时填 `TaskResult.tool_result`（与同步路径同一个 `build_builtin_tool_result`），`task_state_observation` 渲染 Finished 时优先用 `tool_result`；加 `TaskResult::failed()` 判定辅助。`tasks_zone.rs` 的 Zone 任务同样填（F7 通用解析器读 `result.tool_result`）。
- **A5 IntentPolicy**（§3.6）：把 `lib_opendan/src/runner/tools.rs` 的 `with_intent / without_intent / INTENT_ARG` 搬到 agent_tool（`intent.rs`），加 `IntentPolicy::{Off, Optional, Required}`：Optional 不进 `required`、描述前缀 "Optional:"；`XllmToolManager::call_tool` 按策略剥离（Required 缺失 → 现有可修正错误；Optional 缺失 → 照常），原始 args 留在 `call`；`XllmToolManager` 暴露 `last_intent(call_id)` 或在 `ToolCallCtx` 旁提供给宿主读取。`bash_core` 四个工具缺省 Optional，其它工具 Off。`RunRecord` 记策略（恢复时一致）。
- **A7 hints / dedup**（§3.8）：`AgentToolResult` 加 `hints` / `dedup` 字段（`agent_tool/src/lib.rs` 信封 + `result_view.rs`）；`HintLedger` trait + `MemoryHintLedger`，`XllmDeps.hint_ledger`；`XllmToolManager::call_tool` 里替换 + 渲染（`<output>` / `<runtime_hints>`），`XmlStepRenderer` 同形；压缩 / 派生清 `dedup` 命名空间；shell `TaskFollowup` 文本改走 `hints[once]`，`read` 工具改 `dedup`；协议文档 `agent_tool_result_protocol.md` 加两节；xllm run.json 存 `once` 账本。
- **A6 测试**：`cargo test -p agent_tool --lib -- llm_bash tasks task_model intent runtime`；新增：25 s 缺省断言、Session flavor 无 `wait_ms`、shell 只进 `local:`、Zone 解析器只认 `t-`、`local:` 任务重启后 exit 回填 / Unknown、Finished 带 `tool_result` 的渲染与同步结果逐字一致、Optional intent 剥离与记录；hints 渲染结构、`once` 只出一次、`dedup` 替换与压缩后清空、behavior / fc 两种渲染同形。xllm 单测里断言工具 schema 的用例要改。

### B. session 层（libopendan）

- **B1 结束规则按 §3.3 表重写**：`outcome.rs::decide_end`（explicit_report 分支）与 `keep_while_running` 合并为一个按表的函数，输入 = (结束形状, 在跟任务 / 子 session 是否为空, completion)，输出里 "Turn Completed + 会话 Waiting + run Paused（Q12）" 是新组合：`commit_run_end` 要能关 Turn 而不清 `live_run`，`host.extra.kept` 这套随之改名为 Turn 边界的 pause 记录；`watched_tasks` / `wait_children` 的登记不变；纠正输入（Q4）作为 runner 合成输入 `source = system:runner, event = nudge`，`SessionState.nudged: u8` 计数随 run 清零；`missing_end_report` 的失败语只说一次。
- **B2 任务跟踪**：`poll_watched_tasks` 对 Running 仍跳过，Finished / Unknown 走 A4 的 `tool_result`，`status` 按 Q7（`failed` 只看 `TaskResult.success`）；任务增量由 B14 的 hook 读 resolver 直接渲染，`SessionState.task_watch.shown: BTreeMap<task_id, shown_output_len>` 记已呈现到的输出长度（随注入的那次提交写入，与 `HostMeta.observed` 同时机）；`task_summary` 的 `cut(240)` 只用于 worklog 摘要。
- **B3 timer tick**：新 `runner/ticks.rs`：`poll_tick(sh) -> Option<FetchedInput>`，在 `drive.rs:1590` 取候选处与 `poll_watched_tasks` 并列调用；条件 = 会话 Waiting 且有在跟的工作（`watched_tasks` 非空或 `wait_children` 非空）∧ 距 `last_tick_ms ≥ tick_secs` ∧ 配置里有匹配的 Active `timer:` 订阅（B11）；`tick_secs` 来自 `SessionPolicy`（`agent.toml [session.work] tick_secs = 30`，0 关闭），不门控（Q3）；事件 `source = timer:task_watch, event = tick, summary = "<n> task(s) running for <elapsed>; see the updates above"`；`SessionState.task_watch.last_tick_ms` 每次产生 tick 时写。Waiting 空转循环（`drive.rs:1850 / 1872`）的 `poll_interval` 5 s 不变，tick 由它驱动；tick 打开的是一个新 Turn（有自己的 Turn task）。
- **B4 `wait_ms` 与 task CLI**（§3.10）：Session 宿主装配时传 A2 的 flavor；`agent-session task` 去掉 `wait`（`xagent_cli.rs:799` 用法行与解析、task bridge 对 Session 宿主的 `TaskOp::Wait` 返回拒绝文本、`--wait-ms` 参数删）；`TaskFollowup::Input` 文本与 bash_core 组的 task_cli 说明（`xllm.rs:3834`）只提 `state | cancel`；全文 grep "task wait" / "wait_task" 确认 Jarvis 包与 `opendan_agent_dev.md` 无残留。
- **B5 任务后端接线**（Q1）：`opendan/src/tasks.rs::zone_xllm_deps` 只装 `buckyos_tasks`（A3 的 `ZoneTaskResolver`，给 `ask_user` 等 `t-` 任务），不再设 `shell_tasks`；agent service 退出路径（`opendan/src/main.rs` / `loader.rs` 的 shutdown）调用 `local_tasks.release_running()`；`loader.rs:527` 的 `shell_tasks.recover()` 删；`live.rs::host_task_resolver` 去掉 `shell_tasks` 分支。
- **B6 intent 呈现**（Q6）：`SessionToolManager` 用 A5 的策略（`call_intent` → Required 全工具，否则 Optional 由 agent_tool 缺省给）；`turn_task.rs::status_of` 的 "running {tool}" 带 intent；worklog 调用条目加 `intent`。`sh.current_tool` 旁加 `current_intent`。
- **B7 中间 Turn 的回复**（Q9）：作为 Turn 结果写 Turn task（`turn_task.rs` 的关闭路径已有 `report` 字段，复用）与 worklog；`Turns::Unbounded` 下每个 Turn 一个 Turn task，父会话 / MessageHub 按 Turn 看到进度。
- **B8 class `work` 多 Turn + explicit_report**（Q10）：`template.rs:138` 的 `Turns::One` → `Turns::Unbounded`（`WaitPolicy::FinishFailed`、`InputChannel::None` 不变：Work 仍没有消息队列，打开 Turn 的只有任务 / tick / 子 session 事件）；`agent.toml` `[session.work] completion = "explicit_report"`，`[session.plan_simple]` 的同名键删或留注释。检查所有假设"Work 只有一个 Turn"的代码：`decide_end` 的 `completed + 1` 计数、`turn_open_result`、plan_simple 父会话对子会话"Turn 关闭 = 会话结束"的判断（`children.rs` / `todos::sync` 应只看会话 finished 与最终 report，不看 Turn）。
- **B9 工具预算按 Turn 重置**（Q11）：Turn 打开时把本 run 的 `max_tool_iterations` 计数清零（llm_context 的计数若在 run 内不可重置，就在宿主侧记每 Turn 的起点、比较时扣除）。
- **B11 timer 自动订阅**（§3.4）：`template.rs` class `work` 加 `_task_watch_timer` 隐式订阅；B3 的产生者按 `cfg.subscription_for("timer", …)` 产生事件，Semi 订阅的 tick 并进 `pending_events`（一般不用）；`tick_secs` 进 `SessionPolicy` 与 `agent.toml` schema。`agent-session subscribe active:timer:<name>` 的解析已存在（`xagent_cli.rs:483`），文档补"timer 由 runner 在 Waiting 时产生"。
- **B12 work_simple.toml**（§3.7，只改这一个）：`[input] on_input` 模板（task / timer / system:runner 三个分支）、系统提示词改写、`[input] background = "before_inference"`；`render_input` 的 `vars` 补事件字段（`source.kind`、`call_id`、`status`、原调用摘要）。`SessionConfig` 冻结在会话创建时：已有的 Work 会话不受影响，DV 用新建会话。
- **B13 HintLedger 宿主存储**（§3.8）：`SessionState.hint_ledger { once: BTreeSet<String> }` 随 state 提交；`dedup` 命名空间随 run（`live_run` 内存即可，压缩路径 `consolidation.rs` / `history.rs` 重写历史时清空）；`live.rs` 组 `XllmDeps` 时注入。
- **B14 背景统一为推理前注入**（§3.9，Q14）：`observe.rs` 加 `background = "before_inference"` 模式（`enabled` 返回模式枚举，`after_tool_results` 检查只对 `after_tools` 生效）；候选行加在跟任务行（从 `watched_tasks` + resolver 当前状态，增量按 B2 的 `shown_output_len`，单任务上限 2 KB）；`work_simple` 打开；核对 Turn 第一次推理前 hook 的调用顺序与注入位置。
- **B15 `waiting_for` 收敛**（原 R1）：`WaitingFor` 不再单独写，由 `watched_tasks` + `wait_children` + `run_state` 推导（`WaitingKind::Tool / Children / Event` 合成一个"在等的工作"列表，`Input` 保留给空闲 UI 会话）；run.json `host.extra.{tasks, task_calls, sub_call, kept}` 里与之重复的只留 `task_calls`（call_id 对应）与 Turn 边界 pause 记录。
- **B16 `remote_node` 透传 `XLLM_RUN_ID / XLLM_CALL_ID`**（原 R9）：`runtime/node.rs` 的环境过滤放行这两个，`ask_user` 的幂等键在远端节点也成立。
- **B10 测试**（`cargo test -p libopendan --test <name> -- --test-threads=1`，新建 `work_simple_async`，ScriptedLlm，shell 用 `sleep`，禁止测试线程 `std::thread::sleep`）：
  1. 短命令 < 25 s：同步结果，不建任务，一个 Turn 内完成；
  2. 一条长命令：转任务 → 纯文本回复 → **Turn 1 Completed、回复进 Turn task 结果、会话 Waiting、未 finished** → 任务结束 → `<<task_result status=finished>>` 打开 Turn 2，正文含 exit code 与输出，模型看到的历史与 Turn 1 连续（Q12） → `report(is_end=true)` → Turn 2 Completed、会话 finished；
  3. 两条并发长命令（先后结束）：两个 `<<task_result>>` 各打开一个 Turn，顺序与结束顺序一致；中间 tick 打开的 Turn 推理前注入的 `<<background_context>>` 有两条任务行且只含新输出；每个 Turn 都有 Turn task 与回复；
  4. tick：`tick_secs = 1`（测试加速）下每秒一个 tick 打开一个 Turn；无在跟工作时不产生；`tick_secs = 0` 不产生；没有 `timer:` 订阅的 class 不产生；
  5. 失败任务：`status=failed`，正文 exit code 非零；
  6. 无任务纯文本 Done：收到一次 nudge → 再纯文本 → `missing_end_report` 失败；
  7. `report(is_end=true)` 时任务未完 → 拒绝 → `agent-session task cancel` → report 通过；
  8. 崩溃恢复：Waiting 中杀 drive（Turn 边界 pause 记录重做）→ 任务结束后仍打开新 Turn；`local:` 任务在进程重启后按执行目录 `exit` 回填 / Unknown；
  9. Optional intent：带与不带都执行；worklog 与 Turn task 进度文案含 intent；plan_simple 的 Required 行为不变（`--test plan_simple` 回归）；
  10. 输入模板：task_result / tick / nudge 三类输入渲染出模板里的专门段落；
  11. before_inference 背景：Turn 第一次推理前与每批工具结果后都注入在跟任务行；只含自上次呈现以来的新输出；无新输出不注入；单任务超 2 KB 只留末尾；
  11b. `waiting_for` 推导与 `task_calls` 对应（B15）；`remote_node` 环境含 `XLLM_RUN_ID`（B16，单测即可）；
  12. hints：shell 转任务的 followup 只在会话里出一次；`read` 同一文件未变第二次只回 dedup 句；
  12b. 无 wait：Session 宿主下 `agent-session task wait` 被拒且提示结果会作为输入回来；shell 转任务的结果文本不含 "wait"；场景 2 全程推理次数 = 2（Turn 1 派发 + Turn 2 收结果并 report），断言 ScriptedLlm 的调用次数；
  13. ask_user（原 T1）：work_simple 里 `ask_user` → `t-` 任务（Zone 解析器）→ 回复关闭 Turn → 答案以 `<<task_result>>` 打开新 Turn；用 fake TaskMgr 后端；
  14. 回归：`--test plan_simple`、`--test plan_do_todo`、`--test input_tasks`、`--test ui_chat`、`cargo test -p opendan --test loader`；fixtures 重生成只在 schema 变时提交。

### C. Jarvis 包与提示词

- **C1** `work_simple.toml`（B12 的内容归属这里；`explore.toml` 下一刀）：系统提示词补"长命令会继续为任务，结局以 `<<task_result>>` 回来打开下一个 Turn；期间你可以继续别的工作，或只回复一句现状关闭本 Turn；只有 `report(is_end=true)` 结束本会话；不要为长命令调大 wait_ms（参数已不存在）"；`[input]` 三类输入模板 + `background = "before_inference"`；`agent.toml [session.work] tick_secs = 30`、`completion = "explicit_report"`。删掉与 `wait_ms` 有关的措辞。
- **C2** `opendan_agent_dev.md`（agent 开发手册）同步 §3.3 表与 tick 的说明；`builtin_agent_tools.md` §6 改为"shell 任务一律进程内，`t-` 任务来自 krpc 工具"。

### D. 文档

- **D1** `doc/opendan/OpenDAN Agent Session架构设计.md` §12.7（三种等待模式：PendingTool 已删，改为"任务 + Active/Semi 隐式订阅"）、§12.8（timer 隐式订阅 + runner 产生 tick；背景统一推理前注入）、§12.13（shell 任务进程内、`t-` 任务来自 krpc 工具、`local:` 重启语义）、§15.5 差距表更新。
- **D2** `doc/llm_context/AgentRuntime 设计.md`：`task_wait_ms` 缺省 25 s、shell 任务进程内、IntentPolicy；`doc/llm_context/agent_tool/`（AgentToolResult 协议）：Finished 任务的 `tool_result`、`hints` / `dedup` 两节与 `<runtime_hints>` 渲染结构；共享向量加 `HintLedger` 语义（TS TODO 回改）。冻结 TODO §6 第 5 步的 F11 标"在本单做"。
- **D3** [Agent Session 全异步 task 化 TODO](./opendan-agent-session-async-task-todo.md) 已并入本单（状态行指过来，正文留作历史）；实施完成后在本单 §9 写实施记录，不再更新原单。

## 6. DV（本机 DV Zone，单 OOD）

- 环境：`./debug_jarvis.sh devtest --agent <test agent>`（见 [plan_simple TODO](./opendan-plan-simple-todo.md) §10.4 的跑法）；`agent-session create-worksession --class work --objective "在 <repo> 并行 cargo build 两个 crate（各自 nohup 到文件），汇报耗时与结果"`。
- 观察（对照 §3.10）：模型全程没有调用 `agent-session task wait`；MessageHub 上每个 Turn 一个 task 卡片、进度文案含 intent、中间 Turn 的回复"所有任务还在跑，目前……"可见；两条命令转任务后会话 Waiting；tick 按 `tick_secs` 产生、每个 tick 一条可见的进度回复；背景只含新输出；两条 `<<task_result>>` 正文含完整输出；最终 `report(is_end=true)`；全程 token 用量记进本单 §8。
- 故障：Waiting 中 `kill` Loader 再起（取 pid 用 `pgrep -f "[c]argo-target/debug/opendan --app-id X"`，别把自己 shell 杀了）：`local:` 任务结束后由执行目录回填；仍在跑的回 Unknown 且模型据此处理。
- ask_user（原 T1）：work_simple 里让模型 `ask_user`，用户在 MessageHub 作答，答案打开新 Turn；答案目前是 `human.input` 任务的原始 JSON（`{action, values, submitted_by}`），要不要渲染成更短的答案文本在模板里顺手定。
- plan_simple 回归：`PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --mode plan_simple --variant normal`。

## 7. 后续刀（原 async-task TODO 的剩余条目，本单不做）

**第二刀 plan_simple / explore**（work_simple 的问题都搞定后）：
- `explore.toml` 与 `plan_simple.toml` 的提示词 / `[input]` 模板 / `background = "before_inference"`（统一掉 `after_tools`）。
- 子 session 的等待形状（原 R2）：`SessionTaskResolver.wait("session:<sid>")` 立即答复、`agent-session wait <sid>` 与 `create-worksession --wait` 的含义；子 session 作为任务与 `t-` / `local:` 同一套跟进（§3.2 的表加一行）。
- noted task 的分工（原 R5）：`session:` 记进 noted tasks、报告校验只看非 `session:`、`children::mark_heard` 三处补丁随 B15 收敛。
- call_behavior 端到端（原 T2）、子 session 委托工作区与 plan_simple 真实模型（原 T4）。

**第三刀 UI 会话**：
- 等待时的中间回复（原 R6 的 UI 部分：占位消息编辑 / 单独消息 / Turn 内多条）。
- 提问撤销（原 R7）：同一对话来了普通消息时未答的 `ask_user` 任务要不要 superseded。

**另立项**：按工具 / 命令名的等待预算 tool_policy → [xllm runtime review TODO](./xllm-agent-runtime-review-todo.md)；所有 CLI 的 hint 化重新设计（Q13 用户提到的后续 TODO）。

## 8. 关联

- [llm_context 冻结 TODO](./llm-context-freeze-todo.md) §1 原则 8–10、§3.2.1–3.2.2、§8。
- [Agent Session 全异步 task 化 TODO](./opendan-agent-session-async-task-todo.md)（已并入本单；原 R1–R10、T1–T5 的去向见状态行与 §7）。
- [llm_context 长命令 TODO](./llm-context-long-tool-todo.md) §4 / §5（任务模型、OS 进程语义、渐进式披露）。
- [plan_simple TODO](./opendan-plan-simple-todo.md)（intent 契约 §3、DV 跑法 §10）。
- [xllm 视角 runtime review TODO](./xllm-agent-runtime-review-todo.md)（tool_policy、buckyos 三档）。

## 9. 实施记录（2026-11-01，未提交、未 DV）

按 §5 顺序实施：A（agent_tool）先行、单独可测，B（libopendan）只依赖 A 的契约，C / D 同步。

### 9.1 落点

| 条目 | 落点 |
| --- | --- |
| A1 25 s | `task_model::DEFAULT_TASK_WAIT_MS = 25_000`；`task_bridge::DEFAULT_CLI_WAIT_MS` 由它推出（20 s）；`wait_task` / shell 描述都由常量生成 |
| A2 Session 无 `wait_ms` | `LlmBashConfig.expose_wait_ms`（Session flavor 时 false）：schema 无 `wait_ms`、描述改为“结束会作为输入带回”、模型给的 `wait_ms` 被忽略；Session flavor 下 `bash` 组不注册 `wait_task`（只留 `get_task_state` / `cancel_task`） |
| A3 shell 一律进程内 | 删 `ShellTaskManager` / `ShellTaskContext` / `ZoneShellTasks` / `XllmDeps.{shell_tasks, shell_task_context}`；`LlmBashConfig.tasks: Arc<InProcessTaskManager>` + `run`（`XLLM_RUN_ID`）；`tasks_zone.rs` 重写为只读的 `ZoneTaskResolver`（`ZoneTaskBackend { get, request_cancel }`、`ZoneTaskMgrBackend`、测试用 `MemoryTaskBackend`）；执行目录无 `exit` 的 unknown 文本按 Q1 |
| A4 完整结果 | `ShellResultShape` + `shell_finished_result` / `shell_cancelled_result`（同步与 task 共用）；`TaskResult::from_tool_result` / `failed()` / `return_code()`；`AgentToolResult::observation_text()` 是同步映射与 task 结果的同一文本函数 |
| A5 IntentPolicy | `agent_tool/src/intent.rs`（`IntentPolicy::{Off, Optional, Required}`、`with_intent`、`take_intent`、`intent_of`）；`XllmToolManager` 按工具记策略，`bash` / `bash_core` 四个标准工具 Optional，取出的 intent 存 `intent_of(call_id)`；libopendan 的 `call_intent` 用 Required 覆盖全部工具 |
| A7 hints / dedup | 信封 `hints: [{text, once?}]`、`dedup: {key, hint}`；`agent_tool/src/hints.rs`（`HintLedger`、`MemoryHintLedger`、`apply_hints`、`render_with_hints`）；`XllmToolManager` 套用并渲染 `<output>` / `<runtime_hints>`，`XmlStepRenderer` 同形；xllm `RunRecord.hints_once` 持久化 once、压缩时清 dedup；shell 转 task 的跟进说明改为 `hints[{once: "shell.task_followup"}]`；`read` / `read_file`（`DedupReadFile` 包装）用内容摘要做 dedup key |
| B1 结束规则 | `outcome.rs::multi_turn_work(cfg)`（Work + explicit_report + MaxTurns + 无队列）；`keep_while_running` 在有在跟工作时给 `turn_end = Completed` + 回复文本（Turn 关闭、run Paused、会话 Waiting），无工作的纯文本 / 缺 report 给一次 `nudge`；`commit_kept` 抽出 `close_open_turn`（与 `commit_run_end` 共用：outbox、Turn task、计数、Goal Activation）并写 `TurnEnded`；`Next.nudge`，`SessionState.nudge {count, pending}`（新 Turn 清零） |
| B2 任务跟踪 | `poll_watched_tasks` 的 event 名 `finished / failed / unknown`；正文前加 ``Call <call_id>: <tool> `<命令摘要>` — intent: …``（run.json `host.extra.task_briefs`，`note_task` 记录） |
| B3 / B11 timer | `runner/ticks.rs::poll_ticks`（Waiting、无打开的 Turn、有在跟工作、到期、有 `timer:` 订阅；Active → 输入，Semi → pending_events）；`SessionPolicy.tick_secs`（缺省 30，`[session.<class>] tick_secs`）；`SessionState.task_watch.last_tick_ms`（进入等待时与每个 tick 的 receipt 写）；`work` 模板加 `_task_watch_timer`；`settle_input` 把 timer 视为 runner 产生（不升为队列）；`TimerBridge` 跳过 `task_watch` |
| B4 无 wait | `agent-session task state | cancel`（`wait` 给出说明并拒绝，`--wait-ms` 删）；task 桥对 `TaskFollowup::Input` 的 `Wait` 返回 `refused` |
| B5 后端接线 | `opendan::tasks::zone_xllm_deps()` 只装 `ZoneTaskResolver`；`Loader::shutdown` 调 `local_tasks.release_running()`；`recover()` 删；`host_task_resolver` 只包 `buckyos_tasks` |
| B6 intent 呈现 | `Shared.current_intent`；Turn task 文案 `running <tool>: <intent>`；worklog `ActionEntry.intent` |
| B7 中间回复 | `close_open_turn` 把回复写进 Turn task 结果、outbox 规则不变（Work 无出站通道）、worklog `Outcome kind=wait report=<回复>` + `TurnEnded` |
| B8 模板 | 内建 `work`：`Turns::Unbounded` + `CompletionPolicy::ExplicitReport` + timer 订阅；`agent.toml [session.work] completion / tick_secs`；`[session.plan_simple]` 的 `completion` 删（继承内建 work） |
| B9 工具预算 | `resume_live_run` 在无打开的 Turn、无未完成批次、多 Turn Work 时把 `tool_iterations_left` 填满；关闭 Turn 的 keep 不留内存中的 LiveCtx，下一 Turn 一律经 `open_state_live_run` 从检查点打开 |
| B12 模板 | `work_simple.toml`：系统提示词“Long commands and Turns”节、`on_input`（task_result / tick / nudge / 其它事件 / 消息五个分支）、`[input] background = "before_inference"`；`InputView.items` 每项带 `is_task_result / is_tick / is_nudge / is_other_event`（upon 模板不能比较字符串） |
| B13 账本 | `runner/hints.rs::SessionHintLedger`（once 存 `SessionState.hints_once`，随 `commit_and_report` / `commit_run_end` 写；dedup 存在 drive 内，新 context 与 context limit 重写时清空），经 `XllmDeps.hint_ledger` 注入 |
| B14 推理前注入 | `InputBackground::BeforeInference`；`observe::Background` 模式；`task_lines` 读执行目录的 stdout / stderr 增量（≤ 2 KB/任务，首见只记位置），远端 runtime 用 output tail；已呈现位置存 `HostMeta.task_output`（与 `observed` 同时机提交） |
| B16 | `buckyos_api::is_forbidden_env_name` 放行 `XLLM_RUN_ID` / `XLLM_CALL_ID`（客户端过滤与 Node Executor 校验同用它；Node Executor 需重新构建才生效） |

### 9.2 偏差与顺手修的问题

1. **场景 2 推理次数是 3 不是 2**：function call loop 里派发是一次推理，模型看到“仍在运行”后要再推理一次才能回复关闭 Turn 1，Turn 2 收结果并 `report` 是第三次。省掉的是“调 `task wait`”那一次（以及等待超时后的每一次重复）。测试断言 3。
2. **增量位置存 `HostMeta.task_output` 而非 `SessionState.task_watch.shown`**：与 `observed` 同时机随注入的快照提交，崩溃前不提交就重显、提交后不重复；任务只在启动它的 run 里呈现（Q12 同一 run），不需要跨 run 状态。
3. **work_simple 用 `group:bash`**：task CLI 桥只在 `bash_core` + `task_cli`（UI 会话）时提供，work_simple 的跟进方式因此是原生工具 `get_task_state` / `cancel_task`（Session 宿主不注册 `wait_task`），提示词与模板按此写，而不是 `agent-session task cancel`。
4. **报告校验改为问 resolver**（原来只看 run 记过的 task 列表，结局作为输入送达后列表不清，多 Turn 下 `report(is_end=true)` 会被永远拒绝）：Running 算未完；Finished 算已结束；Unknown 在其结局送达（`recent_keys` 有内部键）后才算结束；`watched_tasks` 里未送达的也算未完。拒绝文本列出 task id。
5. **竞态**：task 在模型回复前已结束但结局还没送达时，也算在跟工作（`!result_seen && 未送达`），回复关闭 Turn 并等它的 `<<task_result>>`，不会被误判为“无事可等”而 nudge。
6. **多 Turn Work 等它要听取的全部子 Session**（不只本 run 创建的），否则父会话的回复会被 nudge（`progress_is_observed…` 用例暴露）。
7. **内建 `work` 模板也设为 explicit_report**（不只 agent.toml）：`Unbounded` + `natural` 的 Work 回复后会永远等下去。
8. **xllm 接手的 Work run 不能交最终报告**（xllm 里 `report` 是 `HostedReportUnavailable`）：多 Turn Work 被 xllm 跑完时以 `missing_end_report` 失败；`xagent_cli` 用例按此改写。是否给 xllm 接手提供报告通道，留作后续。
9. 旧的 Zone shell task 用例（`ui_chat`）改为进程内：`a_session_host_refuses_task_wait`、`a_task_checked_through_the_cli_comes_back_once_as_input`。
10. `once` 的跟进提示是通用文本（`<task_id>` 占位），因为同一 session 只说一次。
11. dedup key 用渲染后结果文本的摘要（任何后端都成立，不依赖 mtime）。

### 9.3 未做

- ~~**B15**~~ 2026-11-01 晚已做，形状按用户裁决改为**删除**而不是推导（见 §9.5）。
- ~~**§6 DV**~~ 2026-11-02 用真实模型在本机跑（xagent + 正式 Jarvis 包，见 §9.6）；Zone 侧（Loader 托管、MessageHub 的 Turn task 卡片）未跑：本机 Zone 当前没有 Agent，Turn task 行为由 `work_simple_async` 的 TurnTaskSink 用例覆盖。
- 测试 13 的 `ask_user` 以 `cat` 打印 pending 信封 + `MemoryTaskBackend` 模拟 krpc 工具，没走真实 msg-center。

### 9.4 测试

- `cargo test -p agent_tool --lib -- --test-threads=4`：新增 `hints::*`、`intent::*`、`tasks_zone::zone_tasks_are_read_until_they_end`、`xllm::tests::session_host_shell_is_followed_by_input`（无 `wait_ms`、无 `wait_task`、Optional intent、once hint、read dedup 与压缩后清空）；改 `auto_mode_turns_a_long_command_into_a_task` / `bash_core_follows_tasks_through_the_cli_bridge`（task 结果与同步逐字一致）。
- `cargo test -p libopendan --test work_simple_async -- --test-threads=1`（新建，10 项）：短命令同步；长命令关闭 Turn 1、结局打开 Turn 2、同一 run、推理 3 次、Turn task 两个且第一个带回复、进度文案含 intent、worklog intent；两条并发任务 + 1 s tick + 背景只含增量；失败任务 `status="failed"`；nudge 一次后 `missing_end_report`；任务未完时 report 被拒、`cancel_task` 后通过；同进程新 drive 续接；重启后从执行目录读结局；once hint 与 read dedup；`ask_user` 的 `t-` 任务答案打开新 Turn。
- 回归：`cargo test -p libopendan -- --test-threads=1` 全部、`cargo test -p opendan`、`cargo test -p buckyos-api --lib node_executor`；随语义改动更新的用例：`xagent_lib`（11 项：子 Session 与父会话以 `report` 结束、等待中的回复关闭 Turn、跑到 `Finished`）、`xagent_cli`（3 项）、`plan_simple` / `plan_do_todo`（“一个 Turn”断言改为“每个 Turn 都完成”）、`ui_chat`（3 项）、`opendan --test loader`（3 项 work 会话改用 `CALL:report` 结束；`ui_core.md` 不得含 `<<`）、fixtures `14_input_bus/rendering/vars.json`（items 新增四个标记）。Jarvis 手册中 `agent-session task wait` 的说法（`ui_core` / `aigc_*` / `xllm` / `opendan_agent_dev`）与 `tools/buckyos-agent/readme.md` 同步改掉。

### 9.5 B15：删除 `waiting_for`（2026-11-01 晚，用户裁决）

用户：`waiting_for` 已经没有推导价值，除非 agent session 的某个 wait 函数显式说明（那其实是输入过滤，另立项）。裁决：删 state 字段，status 只留一位。

- `SessionState.waiting_for`、`WaitingFor`、`WaitingKind` 删除；等什么由事实表达：`watched_tasks`（在跟的 task）、新字段 `wait_children`（要听取的子 Session，原来只存在 `waiting_for.refs` 里）、`nudge.pending`。辅助：`SessionState::follows_work()`、`needs_input(goal)`（Waiting、不是 Goal Session、没有在跟的工作）。
- 登记表 `SessionStatus.waiting_for: Option<WaitingKind>` → `needs_input: bool`；读方（`children.rs` 的 attention、`todos.rs` 的 Blocked、背景行、`observe.rs`、宿主是否继续托管）都改读它；宿主轮询判断改为 `follows_work()`。
- `DriveResult::TurnOpen.waiting_for` → `needs_input`；`xagent` 的 `sessions` / `status` 输出 `needs_input`。
- Turn task（`TurnTaskStatus`）带事实：`tasks`、`children`、`needs_input`；opendan 的 TaskMgr 等待原因由它投影（子 Session → ChildTask，task → Dependency 且 `related_task_id` = 第一个 task，例如提问的人工任务，needs_input → input，否则 event）。
- WebUI：`SessionStatus.needs_input`、`SessionState.wait_children`，详情页列出在等的子 Session 与 task。
- run.json `host.extra`：`tasks` 保留——它不是 state 的副本，是本 run 自己未了结的 task 集合（run 进行中 state 还没跟进时报告校验、保活判断、子 context 的任务过滤都靠它；`note_task(running=false)` 会从中移除），`task_calls` / `task_briefs` / `kept` / `sub_call` 各有用途，不删。
- schema（`doc/opendan/protocol/schema/*.json`，`xagent schema` 重生成）、fixtures `16_goal_session`、协议文档（Agent State / Session Directory / Session Input Protocol、README 变更记录）、xAgent.md 等同步。

### 9.6 真实模型 DV（2026-11-02）

驱动 `test/work_simple_dv/main.py`（README 同目录）：正式 Jarvis 包（模型别名换成 `gpt-6-sol`，本机 CLIProxyAPI）+ `xagent new --class work --behavior work_simple` + `xagent run`；夹具是两个构建脚本分别跑 60 s / 100 s 的 crate，目标要求“两条 shell 并行构建，报告结果与耗时”。证据在 `/home/lzc/project/work_simple_dv_runs/<case>/`（`verify.json`、`evidence.json`、会话目录）。

| 用例 | 结果 | Turn / tick / task_result | 推理 / token |
| --- | --- | --- | --- |
| `20261102T000221-normal`（改动前） | 6/10：会话成功，但只有 1 个 Turn——模型对还在跑的 beta 连续调 `get_task_state` 19 次轮询 | 1 / 0 / 0 | 21 / 201 282 |
| `20261102T000711-normal` | 10/10 | 4 / 1 / 2 | 7 / 33 627 |
| `20261102T001000-restart` | 12/12 | 3 / 1 / 2（含 1 条 `unknown`） | 6 / 39 269 |

- **DV 发现并已修**：Session 宿主提供的 `get_task_state` 被真实模型当忙等用（每次一轮推理，离 24 次工具预算只差一点），抵消了“订阅代替等待”。改为 Session 宿主的 `bash` 组只注册 `cancel_task`（不注册 `wait_task` / `get_task_state`）；跟进提示改为“结局会作为输入回来、新输出在推理前给你，不要等也不要反复查看；没事可做就回复现状——这会结束本 Turn”；work_simple 提示词加“Never check on a task in a loop”。UI 会话（`bash_core` + `agent-session task state`）没有推理前的任务背景，`state` 保留并提示只看一次。改后推理次数 21 → 7、token 20 万 → 3.4 万。
- normal（改后）：Turn 1 派发两条构建（各带 intent）并回复现状 → alpha 的 `<<task_result status="finished">>` 开 Turn 2 → 30 s tick 开 Turn 3（回复“beta 还在跑”）→ beta 结局开 Turn 4 → `report(is_end=true)`。模型把 cargo 输出重定向进日志文件，所以没有可显示的背景增量（第一例里背景增量正常出现 8 次）。
- restart：Turn 1 关闭后 SIGKILL 驱动进程（命令在自己的进程组里继续跑），新 `xagent run` 的任务管理器不认识它们：两条构建按 Q1 以 `status="unknown"` 交回；模型读执行目录与进程确认 alpha 已结束，另起一条命令等原 beta 构建的 `exit` 文件出现（没有重跑构建），这条命令的结局 `finished` 打开 Turn 3，最终报告两条耗时（60.139 s / 100.143 s）正确。
- **建议（待议，未改）**：重启后执行目录在、没有 `exit` 的 `local:shell` 任务，runtime 可以继续按执行目录跟进（视为仍在运行，`exit` 出现即结束），而不是直接报 unknown 让模型自己写轮询——restart 例里模型正是这样做的。这与 Q1 的裁决（直接 unknown、模型决定）不同，需要用户定。
- **plan_simple 回归**（`PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --mode plan_simple --variant normal`，`/home/lzc/project/plan_do_todo_dv_runs/20261102T001306-plan_simple-normal/`）：309 s、534 134 tokens（10-25 为 600 522），父会话 5 个 Turn（回复在子会话运行时关闭 Turn、子会话结束打开下一个）、全部开闭配对；原 `turn.single` 检查按新规则改为 `turn.all_closed` 后不调模型重新 verify 40/40（其余 39 项首次即过：intent、并行探索、串行 Todo、workspace 继承、交付物、背景、无 403）。
- 小瑕疵（未改）：同一段 run 里有多条带文本的助手消息时，关闭 Turn 的回复（`Outcome.report` / Turn task 结果）把它们直接拼接、没有分隔（来自 llm_context 的输出文本）。

