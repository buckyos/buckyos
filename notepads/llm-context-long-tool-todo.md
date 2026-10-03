# llm_context 层修改 TODO：长命令、长工具与等待

日期：2026-10-02（macOS / Windows 验证：2026-10-03，见 §11.4 / §11.5）

状态：**已实施（2026-10-02，见 §11）**；xAgent.md 的修改（§8）仍待 review 后进行。依据 2026-10-02 对长命令处理方式（同步硬等、同步执行中崩溃、串行等待、并行等待）的讨论与源码核对，并已结合 [lib_opendan 输入协议与 Turn Loop TODO](./lib-opendan-input-and-turn-loop-todo.md) 的 review 意见。2026-10-02 按 review 意见补充 §3.2：遵循标准父子进程语义，exec 跨平台、少做非标处理，恢复时只按 runtime 给出“被打断”的结果、交给 LLM 判断；§9 第 4 项据此定稿。同日按 review 简化 §4 / §5：长命令的执行模式由配置决定（wait / auto），auto 到期转为 task；llm_context 只做“返回结果 / 挂起等 task”两种机械判断；不区分 job 与 run；§9 第 1、2、3、5 项随之定稿；随后按 review 明确 stop 的两种结束方式、可取消性由工具与 task 的实现声明、`wait_ms` 默认 30s 上限 30 分钟（§9 第 7、8 项）；补充 §1 术语，明确 30 分钟内必须返回 LLM、更长的等待只在 Session 层挂起，xllm 不挂起；§9 第 7、8 项全部定稿（stop 经 task-mgr 父子关系传导、审批作废，硬等上限默认 60 分钟可设为不限）；用哪个 task-mgr 由工具实现决定，`shell` 只用进程内 task-mgr，buckyos task-mgr 是可选依赖。实施前重新确认基线。

## 1. 范围与原则

- 范围：`llm_context` crate，以及 `agent_tool` 中属于 llm_context 层的 exec、exec_tracking、xllm。libopendan 的宿主接入写在 [lib_opendan TODO](./lib-opendan-input-and-turn-loop-todo.md)，本文只在需要时指向它。
- **llm_context 层的修改总是先进行**，libopendan / xagent 的宿主接入在其后。需要 review 确认的 llm_context 事项也记在本文。
- Runtime 配置、Sandbox 与统一派发沿用 [AgentRuntime 下移 TODO](./llm-context-agent-runtime-todo.md)，本文只补长命令相关的契约，交叉处已注明。
- 对 xAgent.md 的修改列在 §8，review 通过后再改该文档。
- 遵循 beta 2.2 规则：持久格式变化时显式升版并拒绝旧版本，不做旧格式兼容。

**术语**（2026-10-02 review）。简单术语最容易被各自理解成不同的意思，本文只按下面的含义使用：

- **stop**：Session 术语，指结束当前 Turn，之后还能继续输入。llm_context 层没有 stop，宿主用平滑结束实现，必要时改用打断。stop 作用于本 Turn 中出现的全部 task：buckyos 的 task 经 Turn 的 task 传导给 sub task（包括审批），进程内的 task 由宿主逐个取消（§4）。
- **打断**（interrupt）：llm_context 层，相当于 Ctrl-C。正在进行的推理立即中止；正在执行的工具收到打断信号（`ToolCallCtx.abort`），能取消就取消，不能取消就放弃等待。xllm 的 Ctrl-C 就是打断。
- **平滑结束**（graceful finish）：llm_context 层。不再发起新的推理和工具调用，等当前推理完成、当前工具取消或完成后再结束。结束时所有调用都已配对结果，追加输入即可继续。
- **取消**（cancel）：工具或 task 层，让正在进行的工作可靠地停下来。是否支持由实现声明，不是所有工作都能可靠取消。打断和平滑结束都会先尝试取消。
- **挂起**（suspend）：工具返回 `Pending{task_id}`，llm_context 结束，等待转到 Session 层（不占进程），task 结束后回填结果、续跑。
- **task**：可以脱离一次工具调用继续运行、可按 task_id 查询的工作。用哪个 task-mgr 管理由工具的实现决定：调用 buckyos 服务得到的 task 由 buckyos task-mgr 管理，其余由进程内 task-mgr 管理（§4）。

实现计划“实现落地”里 llm_context 层遗留的三项未完成，本文处理方式：

- deferred 工具回填：由 §4 接手。
- `ToolSpec.effect`：仍按 Runtime TODO §7 后移；§6 说明它对长命令作用有限。
- `ToolUse.args` 规范键序：与长命令无关，不在本文。

## 2. 现状（2026-10-02 源码核对）

| 场景 | 当前行为 | 问题 |
|---|---|---|
| 同步硬等 | waist 直接 `call_tool(call).await`（`context_loop.rs` 的 `run_tool_batch` 与 behavior action 派发）；interrupt 只与推理竞争，wallclock 只在迭代之间检查。xllm 的 `XllmToolManager` 自带 cancel watch 与总 deadline，触发时返回 `Observation::Error`，且只在 `XllmRun::execute` 接线（xAgent G5）。exec 默认 30 分钟、最长 60 分钟，超时杀进程组 | 宿主 run（libopendan）里 stop 和 wallclock 都打断不了工具，最长要等到 exec 超时；取消被记成业务失败，而不是 Cancelled |
| 同步执行中崩溃 | 派发前 fsync `InflightAction`；`TrackedBashRunner` 握手持久化 `ExecutionRecord`，子进程带 `OPENDAN_EXECUTION_ID`；恢复先 `stop_execution`（无法核验 → RecoveryBlocked），再 `materialize_unresolved` 注入“结果未知” | 方向正确：交给 Agent 判断，不重放。但 Agent 拿到的信息少：不知道命令跑了多久、恢复时是否仍在运行并被停止、已有哪些输出。恢复前的进程核验依赖 Linux /proc，见 §3.2 |
| 异步串行（PendingTool） | waist 完整：挂起、快照、续派、`ResumeFill::ToolResults`。`PendingToolCall.eta_ms` 与 `Outcome::PendingTool.deadline_ms` 恒为 None；等待对象只在 `tool_result.task_id` 里，没有类型。三个宿主都是 `allow_deferred=false`：工具返回 Pending → 调用记 Unknown，run 以 `Error{Internal}` 结束；xllm 不接手 PendingTool 快照，libopendan 返回 RecoveryBlocked | 没有结构化等待记录、截止时间、查询接口和宿主等待循环。`llm_explore` / `llm_understand_media` 的子 run 暂停或被打断时会返回 Pending（`task_id=run_id`），实际就会触发上述 Internal 错误 |
| 异步并行 | 没有任务化的后台执行。exec 工具说明建议 `nohup cmd > log 2>&1 &` 后轮询日志；命令正常退出时不杀进程组，后台进程能活过本次调用 | 后台进程继承执行标记，`ExecutionRecord` 留在 run 记录里（`TrackedBashRunner::sweep` 在生产代码中没有调用）。libopendan 的 `finish_run` 先 `stop_executions`，run 一结束后台进程就被杀；xllm 接手该 run 时由 `settle_previous_executor` 杀掉，而原调用早已返回，不会注入任何说明，Agent 只会看到截断的日志。用 nohup 起服务时同样会被杀，见 §3.2 |

## 3. P0：同步长命令——取消、时限、子进程生命周期与工具命名

不依赖输入协议，可以立即开工。

### 3.1 工具执行期间的取消与时限

- [x] waist 为每次工具 / action 调用提供打断信号和截止时间，例如 `call_tool(call, ToolCallCtx { abort, deadline_ms })`，直接改 trait。`abort` 与推理共用 `LLMContextInterruptHandle` 的状态，打断和平滑结束（§1 术语）都能作用于正在执行的工具；`deadline_ms` 由 waist 按 `budget.max_wallclock_ms` 计算，各 ToolManager 不再自己维护。这同时解决 xAgent G5；Runtime TODO 的 `Sandbox::call_tool` 直接使用这个 ctx。
- [x] **打断与平滑结束**（含义见 §1 术语）。llm_context 提供这两种结束方式，Session 的 stop 由宿主用它们实现：
  - 打断：推理立即中止，返回推理前快照；正在执行的工具能取消就取消，不能取消就放弃等待。
  - 平滑结束：
    - 推理中：等推理完成并记录其输出。其中的工具调用不派发，配对为“因结束未执行”的结果（`Cancelled`，`effect_unknown: false`），保证快照可以续跑。
    - 工具执行中：工具支持取消就取消；不支持就等它完成，最多等一段时间（默认 30s，可配置），超时后按打断处理。
    - 结束时返回所有调用都已配对结果的快照，并新增一种 outcome 与 `Interrupted` 区分。命名避开 stop，以免与 Session 术语混淆。宿主追加新输入即可继续。
- [x] **工具是否支持取消，由工具实现声明**。`AgentTool` 增加声明，默认不支持，因为不是所有任务都能可靠取消。`shell` 按 runtime 支持：native 在 Unix 结束其进程组，在 Windows 只结束直接子进程；tmux 停止等待，命令继续运行（§3.2）。不支持取消的工具被打断时只能放弃等待，结果记为“被打断，结果未知”（`effect_unknown: true`）。
- [x] 打断信号或截止时间触发后，允许 ToolManager 内联返回 `Observation::Cancelled`；其它情况下内联返回 Cancelled 仍属违反契约。waist 记 `ToolExecStatus::Cancelled`，同批余下调用记 `Unresolved{effect_unknown: false}`；打断给出 `Interrupted`，到期给出 `BudgetExhausted{Wallclock}`。
- [x] 定义打断时的快照语义。推理被打断时返回推理前快照 s0，恢复后重做推理；工具被打断时返回**已含配对 Cancelled 结果**的快照，恢复后不重跑该工具，LLM 看到的是“已取消”。behavior 模式按“第一个非成功结果停止其余 action”的规则处理。同步修改《LLM Context 设计》§8。
- [x] 取消时，执行体按 §3.2 的 runtime 规则处理当前命令后返回 Cancelled，不做进程核验。渲染文本按 runtime 写明命令状态（native：已结束；tmux：仍在运行及查看方式），并写明“中途取消，可能已有部分副作用”，不暗示没有副作用。取消动作本身失败（如 ssh 断线）时返回 `ToolDispatchError{effect_unknown: true}`。
- [x] xllm：`XllmToolManager` 去掉自建的 cancel watch / deadline 分支，Ctrl-C 经 interrupt handle 进入 ctx。exec 自身超时仍是工具的 Error（`timed_out`），与 run 级打断区分开。
- [x] 测试：function call 与 behavior 各覆盖以下情况，并验证快照恢复后不重跑已取消的工具：
  - 执行中打断、执行中 wallclock 到期、取消动作失败；
  - 平滑结束：推理中（工具调用配对为未执行）、可取消工具执行中、不可取消工具在等待时长内完成、不可取消工具超时后按打断处理。

宿主侧的配套（工具执行期间读取 Session stop 的监视任务）在 lib_opendan TODO §4。

### 3.2 子进程生命周期：标准语义，恢复时交给 LLM 判断

Review 意见（2026-10-02）：

- xllm 结束时杀掉自己启动过的全部进程是不对的。exec 的目的可能就是用 nohup 起一个服务，所以应遵循标准的父子进程语义。
- native runtime 下的 exec 要跨平台，非标处理越多，兼容负担越重。tmux 本身就用来隔离环境和抗打断：调用 xllm 的 shell 崩了，tmux 里的命令还在跑。
- 恢复时不核验、不停止进程，只需按 runtime 类型给出正确的 call_result（exec 执行被打断），由 LLM 决定下一步。

现状（源码核对）：

| 路径 | 当前行为 | 问题 |
|---|---|---|
| 第一次 Ctrl-C 或 `--timeout` 到期，且工具在执行 | `runner.cancel()`：`TrackedBashRunner::cancel` 对 `pending` 里**全部**执行按 `OPENDAN_EXECUTION_ID` 扫描 /proc 并 SIGKILL；当前命令由 `ProcessGroupGuard` 整组 SIGKILL | `pending` 里还有早已返回、但留下后台进程的执行，nohup 起的服务、setsid 的守护进程一并被杀 |
| 第一次 Ctrl-C，在推理中 | 不杀进程 | `executions[]` 留着后台进程的记录（`sweep()` 没有调用）。之后 `xllm --resume` 时，`settle_previous_executor` → `reconcile_execution` → `stop_execution` 把它们杀掉 |
| 第二次 Ctrl-C | 前台进程组 SIGKILL，`exit(4)` | 无 |
| exec 自身超时 | 整组 SIGKILL，再按标记 `stop_execution` | 追杀已脱离进程组的进程 |
| resume / 接手 | 先按 /proc 核验并停止 `executions[]` 里的全部执行，无法核验时返回 RecoveryBlocked；再把 inflight 物化为“结果未知” | 核验依赖 Linux /proc、boot_id 与进程启动时间，其它平台无法恢复；结果文本不区分 runtime |
| tmux / remote_ssh | 包装脚本用 `/proc/$pid/stat` 记录身份，用 `setsid` 脱离；`cancel`、超时、`ExecutionGuard` 的 drop 都走 `stop_execution` | tmux 因此只能在 Linux 上用（macOS 没有 /proc，默认也没有 setsid） |

目标语义：

- **只处理正在执行的命令，只用现有的标准手段。** native 沿用现有机制：Unix 上用独立进程组，超时或取消时结束整组；其它平台用 `kill_on_drop` 结束直接子进程。不新增信号升级序列、进程身份核验或针对特定平台的处理。
- **命令返回后留下的进程不归 xllm 管**，包括 `&`、nohup、setsid 和守护进程。Ctrl-C、总时长到期、run 结束、resume 或接手时都不停止它们，也不追杀。
- **tmux 下打断 xllm 不结束命令**，这正是 tmux 的用途。Cancelled 结果写明“命令仍在 tmux 中运行”以及查看方式。exec 自身超时仍要结束命令，用 tmux 自身的手段（例如每条命令一个 window，超时时 `kill-window`），不依赖 /proc 和 setsid。具体做法在 tmux runner 改造时定。
- **resume 不核验、不停止、不阻塞。** 执行器异常退出（kill -9、SIGTERM、SIGHUP、断电）时，留下的 inflight exec 一律物化为“exec 执行被打断”的 call_result，内容按 runtime 区分：
  - native：命令、开始时间。说明上一次执行器在命令执行中退出：命令可能已部分执行，通常随执行器一起结束，但不保证（例如执行器被 kill -9 时命令可能仍在运行）；它启动的后台进程不受影响。
  - tmux：读取执行目录。已有 `exit` 文件时，给出退出码和输出尾部；没有时，说明“可能仍在 tmux 中运行”，给出输出文件、`exit` 文件的位置和 tmux 目标。
  - remote_ssh：同 tmux，读取远端执行目录；连不上时如实说明。
  - 这类结果的状态都是“被打断”，不是 Success，不重放命令。LLM 可以自己查看（`ps`、日志、`exit` 文件），再决定重试、等待还是继续。
- run 锁照旧防止两个执行器同时跑同一个 run。旧命令可能仍在运行、LLM 又重跑一遍的风险，由 call_result 写明后交给 LLM 判断。

修改项：

- [x] 删除 exec_tracking 的进程跟踪：`probe_execution`、`stop_execution`、`ExecutionProbe`、环境标记 `OPENDAN_EXECUTION_ID`、启动握手、`ExecutionRegistrar`、`run.executions[]` 与 `sweep()`。native 回到 `LocalProcessBashRunner`。
- [x] tmux 与 remote_ssh 的执行目录改为由 `(run_id, call_id)` 推导的固定位置：tmux 放在 run 目录下（目前在系统临时目录，可能被清理），remote_ssh 放在远端 runtime 目录下。resume 据此找到输出和 `exit` 文件。runtime 种类和目标已在 run.json 的 runtime descriptor 中，`InflightAction` 不需要新增字段。
- [x] `materialize_unresolved` 按上面的规则生成 call_result。删除 `settle_previous_executor` 中停止旧执行的步骤和 `AgentRuntime::reconcile_execution`，进程状态不再导致 RecoveryBlocked。
- [x] `BashRunner::cancel` 只处理当前命令（实现为 `CommandHandle::kill` / `detach`），不再遍历历史执行：native 结束其进程组或子进程；tmux 不结束，只停止等待；remote_ssh 关闭通道。取消动作本身失败时，返回 `effect_unknown`。
- [x] tmux 与 remote_ssh 的包装脚本去掉 `/proc` 身份记录和 `setsid`，只保留 `command`、`stdout`、`stderr`、`exit` 文件。
- [x] Ctrl-C 处理：第一次、第二次都保持现状。SIGTERM / SIGHUP 不新增处理，按异常退出走 resume 路径。
- [x] libopendan：删除 `finish_run` 中的 `stop_executions` 以及 live.rs 中相应的恢复核验。
- [x] 本节推翻 [Runtime TODO](./llm-context-agent-runtime-todo.md) 中已完成的“SSH 执行跟踪与恢复”（远端核验、不可证明停止即 RecoveryBlocked）和首版限制里的“原生恢复的进程核验沿用 Linux /proc”，实施时同步修改该文档。
- [x] 同步文档与 fixtures（Session Directory Protocol 升到 run.json version 4 / snapshot 4，fixtures 已重新生成）：
  - xllm_rust_sdk.md：exec 一段，以及 resume 检查、执行跟踪（X6）两条；
  - Session Directory Protocol：删除 `executions[]`，按规则升版；重新生成 `06_killed_during_exec` 等 fixtures；
  - Agent Session SDK 实现计划：§5.2 与 §8.7 X6。
- [x] 测试（自动化覆盖见 §11；2026-10-03 已完成 macOS 上的 native / tmux 验证，见 §11.4）：
  - exec `nohup sleep 300 >/dev/null 2>&1 &` 返回后，分别在另一个长命令执行中按 Ctrl-C、`--timeout` 到期、run 正常结束、打断后 `--resume`，该进程都仍在运行；
  - 前台命令执行中 kill -9 xllm，resume 不阻塞、不杀进程，按 native 规则给出“被打断”；
  - tmux：命令执行中 kill -9 xllm，resume 时命令已结束的，给出退出码和输出尾部；仍在运行的，给出“可能仍在运行”和查看方式；
  - 以上在 macOS 上各跑一遍 native 与 tmux。

### 3.3 命令执行工具改名为 `shell`，说明按 runtime 生成

已定（2026-10-02 review）：命令执行工具统一叫 `shell`。现有的两个名字都改掉：agent_tool、opendan、Jarvis behavior 和 llm_context 的 XML 动作用 `exec_bash`，xllm 和协议 fixtures 用 `exec`。理由：

- `exec` 让人联想到 exec(2)，暗示工具完全掌控进程，与 §3.2 的语义不符；`bash` 是实现细节，语法在说明里写清即可。
- `shell` 表示“当前 runtime 的 shell”，模型也熟悉这类名字（同类工具名有 `bash`、`shell`、`run_shell_command`）。
- 名字不随 runtime 变化。接手、fixtures 和按名字查的副作用表（libopendan `runner/tools.rs`）都依赖固定的名字。

本文 §2 和 §3.2 的“现状”沿用现在的名字；其余各节的新设计中，命令执行工具都指 `shell`。

runtime 相关的信息通过说明和结果传达：

- [x] **工具说明按 runtime descriptor 和执行模式（§5）生成**，取代现在写死的 “Run bash command at target node”：
  - 第一句：在本 run 的 runtime 中执行命令，使用 bash 语法；
  - 按 runtime 写一条生命周期说明，与 §3.2 一致：
    - native：命令是执行器的子进程，被打断或超时时 Unix 结束其进程组、Windows 只结束直接子进程；Windows 的后代进程可能仍在运行，命令返回后留下的进程不受管理；
    - tmux：命令在 tmux 会话 `<session>` 中运行，执行器被打断或退出时命令继续运行；
    - remote_ssh：命令在远端 `<host>` 上运行；
  - 按模式说明参数：wait 模式说明 `timeout_ms`；auto 模式说明 `wait_ms`，以及到期后转为 task、用 `wait_task` / `get_task_state` 继续（§5）；
  - 后台进程：常驻服务用 nohup / setsid 启动后立即返回。Unix 上前台命令被打断或超时时，同组用 `&` / nohup 启动的进程也会结束。
- [x] **结果标明 runtime**：`BashRunOutput.engine` 渲染进 call_result；§3.2 的“被打断”结果同样带上 runtime。
- [x] **改名范围**（不保留旧名）：
  - agent_tool：`TOOL_EXEC_BASH` 与 xllm 的 `TOOL_EXEC` 合并成一个常量，同步 `llm_compress`、`llm_explore`、`llm_understand_media`、`llm_tool_carft`、`todo_tools` 中的引用。`ExecBashTool`、`llm_bash` 等类型名和模块名不进入协议，实施时顺手改。
  - llm_context：XML behavior 的内置动作标签 `<exec_bash>` 改为 `<shell>`，涉及 `xml_behavior.rs` 的标签表、body → `command` 的映射和协议提示词，以及 `request.rs` 的注释和相关测试。
  - libopendan：`runner/tools.rs` 的副作用表，`lock.rs` 的注释和测试。
  - opendan 与 Jarvis：`opendan/src` 中的引用，以及 `jarvis_runtime/agent/behaviors/*.toml` 中的提示词。
  - buckyos-api：TaskMgr 的任务数据类型 `tool.exec_bash` 与 schema `tool.exec_bash/v1`（`taskdata.rs`、`task_mgr.rs`）。`shell` 不使用 buckyos task-mgr（§4），本仓库也没有其它引用。确认没有外部调用方后删除；有调用方则改名为 `tool.shell` 与 `tool.shell/v1`。这是 kernel 的共享类型。
  - 其它：`msg_center/src/tg_tunnel.rs` 的测试数据、`src/read_aicc_log.py`、`tools/buckyos-agent/readme.md`。
  - 文档：xllm PRD 与 xllm_rust_sdk.md；`doc/llm_context/` 下的 Agent Actions、agent_tool_result_protocol、local_llm_context_protocol 等；`doc/opendan/` 下的 build-in agent-tool 手册、NewOpenDANRuntime、Agent Session SDK 实现计划、xAgent.md 等。
  - 协议 fixtures：工具名出现在 run.json 和快照里，随 §3.2 的升版一起重新生成。
  - xllm 的内置工具组名 `bash`（`BUILTIN_TOOL_GROUP_BASH`）是配置项，LLM 看不到，保持不变。
- [x] **合并说明改写**：§5 的“改写工具说明”并入本节第一项，一次写完。
- [x] **验证**：`grep -rnw exec_bash` 只剩历史记录；xllm、llm_context、libopendan 中断言工具名的测试已更新；`cargo test -p llm_context`、`cargo test -p agent_tool --lib`、`cargo test -p libopendan -- --test-threads=1`、`cargo check -p opendan -p buckyos-api -p msg_center` 通过。

## 4. P1：等待——llm_context 的两种机械判断与 RunningTaskResolver

Review 意见（2026-10-02）：简化设计、渐进式披露，减少 LLM 在调用前要做的决策。llm_context 执行工具时只做机械判断，结果只有两种：

1. **把结果返回给 LLM。** auto 模式下 `shell` 转为 task 后返回的“仍在运行”结果也属于这一种（§5）。
2. **工具返回 `Pending{task_id}`。** 结束当前 llm_context（`Outcome::PendingTool`），由宿主在 Session 层等 task 结束（不占进程），再构造 tool_result 续跑（`ResumeFill::ToolResults`）。

两条时间规则（2026-10-02 review）：

- **30 分钟内必须返回 LLM。** llm_context 里任何在工具内等待 task 的调用，最长 30 分钟必须返回，并带上 task 当时的状态。这类调用包括 auto 模式的 `shell`、`wait_task`，以及宿主不能挂起时等待 Pending 的工具。task 继续运行，由 LLM 决定继续等、做别的，还是取消。这样一次工具调用不会把 llm_context 彻底卡住。wait 模式的硬等是配置显式选择的，不受这条规则约束（§5）。
- **更长的等待放在 Session 层。** 需要等得更久时就挂起（第 2 种）：llm_context 结束，由 Session 等待，不占进程。只有能这样等待的宿主（libopendan 的 Session）才挂起；xllm 没有 Session，不挂起。

挂起记录只需支持这个判断和之后的回填。原设计中的 `wait{source, class, check_after_ms, deadline_ms, detail}` 删除。

- [x] **挂起记录**：`PendingToolCall` 增加 `task_id`（必填）和 `until_ms`（可选：到这个时间 task 仍未结束，也按当时的状态回填）。
  - 缺 `task_id` 的 Pending 一律拒绝（同现在 `exec_bash` 不转发无 `task_id` 的 Pending）。
  - `task_id` 对 llm_context 不透明，不再按前缀归一。
  - 快照升版。Session 的 `waiting_for.refs` 就是这些 `task_id`，`pending_task_calls` 删除（lib_opendan TODO §6.1）。
- [x] **`RunningTaskResolver`**（取代原设计的 `DeferredResolver`）定义在 llm_context 层，由宿主装配：

  ```rust
  #[async_trait]
  pub trait RunningTaskResolver: Send + Sync {
      /// Running { brief, output_tail, cancellable } | Finished(AgentToolResult) | Unknown { reason }
      async fn state(&self, task_id: &str) -> TaskState;
      /// 等到 task 结束或到达 until，返回当时的状态。
      async fn wait(&self, task_id: &str, until_ms: Option<u64>) -> TaskState;
      /// 只有 cancellable 的 task 能取消，其余返回 Unsupported。
      async fn cancel(&self, task_id: &str) -> Result<TaskState, CancelUnsupported>;
      /// 本 context 关注的 task 简介，供 background env 使用。
      fn active(&self) -> Vec<TaskBrief>;
  }
  ```

  - 两种实现：
    - 用哪种 task-mgr 由工具的实现决定，语义正确优先（2026-10-02 review）：
      - 工具调用的 buckyos 服务本身返回 task id 时，直接用 buckyos task-mgr。按 buckyos 的流程，这类 task 挂在当前 Turn 的 task 下（宿主经 `SessionRuntimeContext` 传入 Turn 的 task id）。
      - 其余情况用进程内 task-mgr，包括任何 runtime 下的 `shell`。不为了用上 buckyos task-mgr，而把 `shell` 改成 buckyos 的 `run_at(node_id, cmd)` 之类的服务调用。
      - buckyos task-mgr 是工具实现可用的依赖，不是必需的。
    - resolver 因此是组合的：进程内 task-mgr，加上可访问时的 buckyos task-mgr，按 task_id 的来源分派（例如进程内 task 的 id 带固定前缀）。分派是 resolver 内部的事，对 llm_context 而言 task_id 仍不透明。
  - `TaskState` 渲染成 tool_result 只用一个函数。内联的 `wait_task` / `get_task_state` 和挂起后的回填都用它，所以两条路径的结果完全一致。Finished 的结果走与内联结果相同的 `AgentToolResult → Observation` 映射。
  - 通知只用来提前唤醒，结果以 `state` / `wait` 为准。
  - task 能否取消由 task 的实现声明（同 §3.1 的工具），不是所有 task 都能可靠取消。
- [x] **task 工具**（auto 模式下，或环境中可能出现 task 时注册）：
  - `wait_task(task_id, wait_ms)`：等到 task 结束或 `wait_ms` 到期，返回结果，或当前状态与新增输出。
    - `wait_ms` 默认 30s。不超过 30 分钟时，在工具内等待，受 §3.1 的打断与时限约束；等待本身总能取消，不影响 task。
    - 超过 30 分钟：宿主能在 Session 层等待时，返回 `Pending{task_id, until_ms}`，走第 2 种；否则（如 xllm）按 30 分钟截断，到时返回当时的状态。
  - `get_task_state(task_id)`：立即返回状态与输出尾部。
  - `cancel_task(task_id)`：只对声明可取消的 task 生效，其余返回“不支持取消”。
  - task 仍在运行时，结果附上下一步提示：如何继续等待或稍后查询；只有可取消的 task 才提示 `cancel_task`。
- [x] **崩溃恢复与接手不做额外判断**。task_id 已写在 call_result 或挂起记录里，崩溃恢复后由 LLM 或宿主再查一次即可：
  - 挂起中的 run：由 Session 继续等待。xllm 接手已挂起的 run 时不等待，用 `resolver.state` 按当时的状态立即回填，然后续跑。
  - 进程内 task-mgr 重启后查不到旧 task：返回 Unknown，照常回填续跑，不返回 RecoveryBlocked。shell task 能从执行目录读到 `exit` 文件时，给出退出码和输出尾部。
  - 挂起记录的 task_id 来自 buckyos，而接手方访问不了 buckyos task-mgr（例如单独运行的 xllm）：拒绝接手并说明原因，做法同 `app_tools`。进程内 task 换了进程后查不到，按上一项回填，不拒绝。
- [x] **background env：第一次使用半自动订阅。**
  - 结果里带有运行中 task_id 的调用，自动登记为本 context 关注的 task。
  - 每次推理前，waist 用 `resolver.active()` 渲染一段 background env，每个 task 一行：task_id、命令简述、状态、已运行时长、最后一行输出。
  - 这里只放简介，完整输出由 LLM 按需调用 `get_task_state` / `wait_task` 获取（渐进式披露）。已结束的 task 显示到 LLM 读取过一次结果为止；没有 task 时不渲染。
  - 这段内容每次推理重新生成，不写进历史，放在请求末尾以免破坏前缀缓存。现有 `CheckpointHook` 返回的 `Injection` 会把消息追加进历史，不适用，waist 需要新增这个临时插槽。
  - run 结束后 task 才完成时是否唤醒 Session，在 lib_opendan TODO §6.2 处理。
- [x] **xllm 不挂起**：xllm 没有 Session，`allow_deferred` 保持关闭，所有等待都在工具内进行，最长 30 分钟返回 LLM。原设计中的 xllm 等待循环、run 状态 `waiting`、`xllm status` 显示等待对象都不做。xllm 原先“不接手 PendingTool 快照”，改为上一项的做法：按当时的状态回填后续跑。
- [x] **宿主不能挂起时**（xllm，或 Session 未开启挂起）：`wait_task` 最长等 30 分钟，结果是当时的状态。其它工具返回 Pending 时，ToolManager 用 resolver 在工具内等待，同样最长 30 分钟，到时以“仍在运行”和 task_id 返回 LLM（第 1 种），受 §3.1 约束。都不再以 Internal 错误结束 run。
- [x] **子 run 暂停或被打断**（`llm_explore` / `llm_understand_media`）：不再返回 Pending，直接作为结果返回，附上 `xllm --resume --run <id>` 接手提示。
- [x] 一次挂起只等一个调用，同批后续调用等回填后再派发。保持现状（串行语义），写进设计文档。
- [ ] **Session stop 对 task 的影响**（宿主侧，lib_opendan TODO §4 / §6，本次未做）（2026-10-02 review）：stop 作用于本 Turn 中出现过的全部 task，即结果或挂起记录里带 task_id 的 task，与 task 来自哪个 task-mgr 无关：
  - buckyos 的 task：Turn 的 task 进入 stop 状态，按 task-mgr 的父子关系传导给所有 sub task，审批票据随之作废；
  - 进程内的 task：宿主对本 Turn 登记过的 task 逐个调用 `resolver.cancel`；
  - 支持取消的被取消；不支持的继续运行，状态如实记录；
  - 之前 Turn 创建、仍在运行的 task 不受影响。
- [ ] **挂起期间收到 Session stop**（宿主侧，本次未做）：挂起时 llm_context 已经结束。宿主停止等待，Turn 的 task 按上一项传导 stop，再用 task 当时的状态（已取消、已作废或仍在运行）回填，Turn 结束。仍在运行的 task 继续显示在 background env 中。如果结果已经回填并提交，先完成回填再处理 stop。
- [~] 测试（已覆盖项见 §11；未覆盖：挂起→宿主回填→续跑的 behavior 变体、`until_ms` 到期回填、buckyos 组合分派）：
  - 挂起 → 快照 → 宿主用 resolver 回填 → 同一 run 续跑（测试宿主模拟 Session 层等待）；
  - 工具内等待到 30 分钟上限（测试用小值）时返回当时的状态，task 继续运行；
  - xllm 接手已挂起的 run：按当时的状态回填后续跑；
  - `until_ms` 到期，按“仍在运行”回填；
  - 进程内 task-mgr 重启后，按 Unknown 回填；
  - 组合 resolver 按 task_id 的来源分派；挂起在 buckyos task 上、接手方访问不了 buckyos 时拒绝；
  - 内联 `wait_task` 与挂起后回填的结果一致；
  - background env 随 task 状态变化，结束的 task 被读取后不再显示。

## 5. P1：`shell` 的执行模式与 task

长命令的执行方式由配置决定，不让 LLM 在调用前选择。

- [x] **模式**（配置项 `shell.mode`）：
  - `wait`（硬等）：现有行为，执行到结束或 `timeout_ms` 到期（到期结束命令）。xllm 的很多用法会这样配置。`timeout_ms` 的上限是配置项，默认 60 分钟；手工选择硬等时可以设为 0，表示不限。不受 §4 的 30 分钟规则约束。
  - `auto`（默认）：先在工具内等 `wait_ms`（默认 30s，上限 30 分钟；LLM 可以为长编译等命令调大）。到期仍未结束，命令转为 task 继续运行，并立即返回“仍在运行”的结果（§4 第 1 种），内容包括 task_id、已有输出、已运行时长，以及 `wait_task` / `get_task_state` 的用法提示。
  - LLM 只看到当前模式的参数：§3.3 的工具说明按模式生成，wait 模式说明 `timeout_ms`，auto 模式说明 `wait_ms` 和转 task 的行为。不提供 `background` 之类让 LLM 选择执行方式的参数。
- [x] **执行目录**：native 的 `shell` 一律把 stdout / stderr 写入执行目录（由 `(run_id, call_id)` 推导，同 §3.2 的 tmux / remote_ssh），退出时写 `exit` 文件，不经管道。
  - 这样转为 task 后，即使执行器退出，命令也不会因管道断开收到 SIGPIPE；task-mgr 重启后仍能读到输出和退出码。
  - 崩溃后 §3.2 的“被打断”结果也能附上输出尾部（§6）。
- [x] **转 task 前后的归属**：
  - 转 task 之前，按前台命令处理（§3.1 / §3.2 的取消与恢复）。
  - 转 task 时解除 `ProcessGroupGuard` 和 `kill_on_drop`，命令不再属于 run。run 被打断、结束或被接手都不停止它（同 §3.2 的父子进程语义），由进程内 task-mgr 管理。Session 的 stop 是另一回事：它作用于本 Turn 的 task（§4）。
  - 本层不给 task 设最长存活时间；task 能否取消由实现声明（§4）。native 的 shell task 可以取消（Unix 结束其进程组，Windows 只结束直接子进程，后代进程可能继续运行）。
- [x] **不区分 job 与 run**：删除原设计中的 detached job 子系统，包括 `background` 参数、`job_*` 工具、`OPENDAN_JOB_ID`、由 `(session, run, call_id)` 推导的稳定身份、启动握手、job 存储与 GC、完成通知钩子。
  - task_id 写在 call_result 里。
  - 转 task 之前崩溃：按 §3.2 给出“被打断”结果。
  - 转 task 之后崩溃：由 LLM 再查一次 task_id。
- [x] **前台 `shell` 命令自己留下的后台进程**（命令里写了 `&`、nohup 或 setsid）：按 §3.2 不停止、不跟踪。常驻服务用 nohup / setsid 启动后立即返回；需要结果的长命令直接在前台运行，由 auto 模式转为 task。
- [x] 工具说明与 §3.3 一起改写。
- [x] 实施位置：
  - 首版只做 native 与进程内 task-mgr；
  - tmux / remote_ssh 的执行目录已有输出与 `exit` 文件，按同一契约接入；
  - `shell` 只用进程内 task-mgr。buckyos task-mgr 的接入只在有工具调用返回 buckyos task id 的服务时才做。
- [~] 测试（已覆盖项见 §11；未覆盖：转 task 后 kill -9 执行器再 resume 的端到端）：
  - `wait_ms` 内结束的命令，结果与 wait 模式一致；
  - 超过 `wait_ms` 后转为 task，返回 task_id 和已有输出，之后 `wait_task` 拿到最终结果；
  - 转 task 后 run 结束、Ctrl-C、kill -9 执行器，命令都继续运行，输出与 `exit` 正常写入；resume 后 `get_task_state` 给出结果或 Unknown；
  - 转 task 之前 kill -9 执行器，resume 给出 §3.2 的“被打断”结果。

## 6. P2：同步崩溃恢复的信息补全

- [x] native 的执行目录由 §5 提供后，§3.2 的“被打断”结果附上输出尾部和已运行时长，帮助 Agent 判断进度和是否重试。
- [x] 保持不重放。`ToolSpec.effect` 的迁移仍按 Runtime TODO §7 后移。`shell` 的副作用取决于具体命令，effect 只能是 unknown，所以长命令崩溃后始终交给 Agent 判断。长命令在 auto 模式下转为 task，崩溃后依据 task 状态判断，不重放命令。

## 7. 与 lib_opendan TODO 的对应

| 本文 | lib_opendan TODO | 关系 |
|---|---|---|
| §3 工具取消与时限 | §4 工具执行期间的 stop 监视；§5 单写者纪律 | 监视任务依赖 `ToolCallCtx`；只查看控制输入并触发打断或平滑结束，不确认 / 消费输入、不写 state |
| §4 `Pending{task_id}` | §4 挂起调用与事件的匹配 | 按 `task_id` 相等匹配 |
| §4 挂起记录 | §6.1 `waiting_for.refs`、删除 `pending_task_calls` | refs 即挂起记录中的 `task_id` |
| §4 `RunningTaskResolver` | §6.1 宿主查询能力；§7 xagent / xllm 交接 | 接口在本层；组合进程内 task-mgr 与工具需要时的 buckyos task-mgr |
| §4 宿主不支持挂起时在工具内等待 | §6.1 缺能力时 RecoveryBlocked | 两个不同时机 |
| §4 background env、§5 auto 转 task | §6.2 自动订阅 | 本层提供 `active()` 和 env 插槽；run 结束后的唤醒在 Session |
| §5 不区分 job 与 run | §6.3 dispatch intent 与幂等身份 | shell 不再需要推导稳定身份：task_id 写在 call_result 里，崩溃后再查 |
| §3.1 两种结束方式、§4 stop 对 task 的影响 | §4 停止、审批和结果完成的先后 | Session 的 stop 按 §1 的含义，用平滑结束实现，必要时打断；Turn 的 task 的 stop 传导给 sub task（审批作废）；lib_opendan TODO 按此同步 |

lib_opendan TODO 中引用本文原设计的条目（§4 的 `wait.source{kind, id}` 匹配，§6.1–§6.3 的 job、bridge、稳定身份），review 通过后按本次简化同步。[switch-support TODO](./llm-context-switch-support-todo.md) 中 T4 引用的 `subrun` 种类也一样。

## 8. 对 xAgent.md 的修改（review 通过后再改）

| 位置 | 修改 |
|---|---|
| §0 第 5 点、§5.2 | `DoContext.deadline` 与取消改由 waist 的 `ToolCallCtx` 提供（§3）；`RequireApproval` 返回 `Pending{task_id: 审批票据}`，审批结果经 resolver 回填；审批是当前 Turn 的 task 的 sub task，Turn 被 stop 时作废 |
| §1.1 Agent Runtime 行 | 删除“后台进程的识别与停止”，改为“打断时按 runtime 处理当前命令；恢复时按 runtime 给出被打断命令的结果”（§3.2）；职责补上 shell 的 auto 转 task，task 的查询与等待由 `RunningTaskResolver` 提供（§4、§5） |
| §3.1 G5 | 改为由 waist ctx 解决，`SessionToolManager` 不再自带 deadline |
| §3.6 工具子上下文（T4） | 不再用 `wait.source.kind == SUBRUN` 识别子 run：Session 按 `task_id` 在自己的登记表里识别；接手检查改为“接手方的 resolver 能否解析挂起的 task_id”（§4） |
| §4.3 / §4.6 | 删除 `wait.source` 词汇；task 状态经 background env 呈现（§4 的半自动订阅），run 结束后的唤醒按 lib_opendan TODO §6.2 |
| §4.7 Session 模板 | 无队列模板中 task 的呈现与唤醒（§9 第 6 项）；Session 结束时 task 的清理见 §9 第 4 项 |
| §4.15 同步等待子 session | 不再由 `session:` 前缀归一；`task_id` 对 llm_context 不透明，由 Session 提供的 resolver 解析 |
| §9.3 / §9.5 | PendingTool 分支：`waiting_for.refs` 即挂起记录的 `task_id`；等待出口与 inbox 是否为空无关，统一调用 `resolver.wait`，结束后回填 ToolResults，续跑同一 run / Turn；挂起期间收到 stop 时，Turn 的 task 的 stop 传导给 sub task 后再回填（§4）；Session stop 用 llm_context 的平滑结束实现，必要时打断（§1、§3.1）；工具执行期间的 stop 监视任务 |
| §9.4 | 删除 PendingTool → `RecoveryBlocked`；恢复时用 resolver 继续等待，只有挂起在 buckyos task 上、接手方访问不了 buckyos 时才拒绝 |
| §10 验证矩阵 | 新增实验：① `wait_task` 挂起：等待中 kill -9，恢复后继续等，结果回填到同一 Turn。② auto 转 task：命令超过 `wait_ms` 后转为 task，background env 显示其状态；Turn 关闭后 task 完成；Runner 被杀不影响 task。③ 长 shell 中途 stop：Cancelled、快照配对、不重跑。④ 同步 shell 中途崩溃：恢复不阻塞、不杀进程，按 runtime 给出“被打断”的结果，不重放。⑤ nohup 起的服务在 stop、run 结束和接手后都仍在运行。⑥ stop 传导：Turn 的 task 被 stop 后，审批作废，本 Turn 中可取消的 task 被取消，之前 Turn 的 task 不受影响 |
| §11 C8 / C12 | C12 的“`allow_deferred` 可开”改为依赖本文 §4 的 resolver 与 Session 层等待（xllm 不开）；C8 的 Sandbox 包含 shell 的执行目录与 auto 转 task；进程内 task-mgr 与组合 resolver 新增一项或并入 C8 |
| 全文 | 工具名 `exec` / `exec_bash` 改为 `shell`（§3.3） |

## 9. 待 review 决定

1. ~~deferred 截止时间到期时回填 `Cancelled` 还是 `Error{timeout}`~~ 随 §4 的简化已定：`until_ms` 到期按当时的状态（仍在运行）回填，不算错误。
2. ~~子 run 暂停 / 被打断时返回 Pending 的工具如何处理~~ 随 §4 的简化已定：直接作为结果返回，附接手提示。
3. ~~派发时宿主未开 deferred 如何降级~~ 随 §4 的简化已定：在工具内等待，不需要按工具声明。
4. ~~xllm 是否也在 run 结束时停止 run-owned 的后台进程~~ 已定（2026-10-02 review），见 §3.2：
   - 遵循标准父子进程语义，不停止命令留下的后台进程；
   - 恢复时不核验、不停止进程，按 runtime 给出“被打断”的 call_result；
   - libopendan 共用 exec_tracking，一并处理；
   - Session / Sandbox 销毁时的清理，建议交给 runtime 自身（例如停止容器），native 不另做。
5. ~~后台执行的接口形态与词汇表~~ 随 §4 / §5 的简化已定：没有让 LLM 选择后台执行的参数，auto 模式到期转 task，task 工具只有 `wait_task` / `get_task_state`，没有 `wait.source` 词汇表。`cancel_task` 是否可用由 task 的实现决定（2026-10-02 review）：不能可靠取消的 task 返回“不支持取消”，结果提示里也不出现它（§4）。
6. 无输入队列的模板（work session）：run 内由 background env 呈现 task 状态；run 结束后 task 才完成时是否唤醒 Session，由 lib_opendan TODO §6.2 决定。
7. ~~挂起期间收到 stop~~ 已定（2026-10-02 review）：stop 的含义见 §1 术语，llm_context 层用打断和平滑结束实现（§3.1）；stop 作用于本 Turn 的全部 task：buckyos 的 task 经 Turn 的 task 传导（审批作废），进程内的 task 由宿主逐个取消（§4）。
8. ~~默认值~~ 已定（2026-10-02 review）：`wait_ms` 默认 30s；任何在工具内等待 task 的调用最长 30 分钟必须返回 LLM，更长的等待只能挂起到 Session 层（§4）；本层不设 task 的最长存活时间；平滑结束时等待不可取消工具的时长默认 30s；wait 模式（硬等）的 `timeout_ms` 上限默认 60 分钟，手工选择硬等时可设为 0（不限）（§5）。

## 10. 实施顺序与验证

命令在 `src/` 下执行。

1. §3（P0）：立即开工，不依赖 lib_opendan 的输入协议，可与 lib_opendan TODO §8 第 1、2 步并行。§3.1（含两种结束方式与工具的可取消声明）与 §3.2 一起做，两者共用 cancel 路径；§3.3 的改名与 §3.2 的协议升版、fixtures 重新生成一起做。验证 `cargo test -p llm_context`、`cargo test -p agent_tool --lib`，§3.2 另跑 `cargo test -p libopendan -- --test-threads=1`，确认 `finish_run` 的变化。
2. §4（P1）：挂起记录 `{task_id, until_ms}` 与快照升版、`RunningTaskResolver` 与进程内 task-mgr、`wait_task` / `get_task_state` / `cancel_task`、background env 插槽、xllm 接手已挂起 run 时的立即回填。
3. §5（P1）：`shell` 的 wait / auto 模式、native 执行目录、到期转 task，与 Runtime TODO 的 Sandbox 协调先后；同步工具说明。可与第 2 步合并实施。
4. §6（P2）：恢复信息补全。
5. 之后才进入 lib_opendan TODO §6 的宿主接入（`cargo test -p libopendan -- --test-threads=1`）；review 通过后修改 xAgent.md，同步《LLM Context 设计》、xllm Rust SDK、Session Directory Protocol 和 fixtures。

参考入口：`llm_context/src/{context_loop,suspension,observation,request,interrupt}.rs`，`agent_tool/src/{llm_bash,exec_tracking,xllm}.rs`；[LLM Context 设计](<../doc/llm_context/LLM Context 设计.md>) §8–§9；[长任务 RFC](<../doc/opendan/OpenDAN Long Task & Sub-Agent.md>) §5–§7。

## 11. 实施记录（2026-10-02）

实施前基线：`cargo check -p llm_context -p agent_tool -p libopendan --tests` 通过；`agent_tool` 212 测试、`llm_context` 188 测试、`libopendan` 81 测试、`opendan` 278 测试。

### 11.1 改了什么

**llm_context**（`src/frame/llm_context`）

- `interrupt.rs`：`InferenceAbortState` 增加 `finishing` 位；`LLMContextInterruptHandle::finish`（平滑结束）、`standalone`、`token`；`InferenceAbortToken::{is_finishing, stopping}`。
- `deps.rs`：新增 `ToolCallCtx { abort, deadline_ms, allow_deferred }`、`CancelCause::{Interrupted, Finishing, Deadline}`、`cancelled()` / `cancelled_hard()` / `cause()` / `wait_until_ms()`；`ToolManager::call_tool(call, ctx)`；`LLMContextDeps.tasks: Option<Arc<dyn RunningTaskResolver>>`。
- `tasks.rs`（新）：`RunningTaskResolver`（`state` / `wait` / `cancel` / `can_resolve` / `watch` / `active`）、`TaskState`、`TaskResult`、`TaskBrief`、`task_state_observation`（唯一渲染）、`render_background_env`、常量 `MAX_IN_TOOL_WAIT_MS = 30 min`、`DEFAULT_TASK_WAIT_MS = 30 s`。
- `observation.rs`：`Pending { call_id, task_id, until_ms, tool_result }`、`Cancelled { call_id, reason, effect_unknown }`、`PendingToolCall { call, task_id, until_ms }`（删 `eta_ms` / `tool_result`）。
- `outcome.rs`：`PendingTool` 删 `deadline_ms`；新增 `Settled { reason, usage, snapshot, trace }`。
- `request.rs`：`ToolPolicy.finish_grace_ms`（默认 30 s）。`state.rs`：`SNAPSHOT_FORMAT_VERSION = 4`。
- `context_loop.rs`：每次调用构造 `ToolCallCtx`（deadline 由 `budget.max_wallclock_ms` 推出）；`dispatch_tool` 在平滑结束超过 grace 后把 finishing 升级为 abort；内联 `Cancelled` 只在 `ctx.cause()` 存在时合法，按 cause 收尾（Interrupted 快照含配对结果 / `BudgetExhausted{Wallclock}` / `Settled`）；finishing 在推理边界、批次内、step 内各处停下；`Pending` 缺 `task_id` 或无 `allow_deferred` 为契约违规；结果里的 `task_id` 自动 `watch`；推理前渲染 background env 追加到请求末尾（不进历史）。behavior 模式同样处理（`settle_step`）。
- `xml_behavior.rs` / `step_record.rs`：动作标签 `<exec_bash>` → `<shell>`；`Cancelled(result unknown)` 渲染。
- 测试：新增 `tests/cancel.rs`（打断 / 到期 / 平滑结束 / 契约违规 / background env，function call 与 behavior）。

**agent_tool**（`src/frame/agent_tool`）

- `llm_bash.rs` 重写：`TOOL_SHELL = "shell"`、`ShellTool`（取代 `ExecBashTool`）、`ShellMode::{Wait, Auto}`、`RunBinding` / `RunBindingSlot`（run 目录晚绑定）、`exec_dir_for`、`ShellRuntimeNote`（说明按 runtime 与模式生成）；`BashRunner::start → CommandHandle { wait, progress, kill, detach, cancellable, locator }`，`run` 由 `start` 推导（只实现 `run` 的旧 runner 仍可用）；`LocalProcessBashRunner` 用包装脚本把输出写进执行目录 `runs/<run_id>/exec/<call_id>/{command,stdout,stderr,exit}`（无 run 目录时用临时目录并清理），只用进程组 / `kill_on_drop`，命令返回后不再跟踪；取消经任务本地 `CURRENT_TOOL_CTX`：可取消 runtime 结束命令，否则停止等待，文本写明命令状态与“可能已有部分副作用”；auto 模式到期把 `CommandHandle` 交给进程内 task-mgr 并返回 `task_id`。
- `tasks.rs`（新）：`InProcessTaskManager`（`local:shell:<call_id>` / `local:<kind>:<n>`）、`ShellTask`、`LocalTask`、`CompositeTaskResolver`（进程内 task + 执行目录回读 + 可选 buckyos resolver；`can_resolve` 对非 `local:` 且无 buckyos 为 false）、`wait_task` / `get_task_state` / `cancel_task`、`FinishedTask`。
- `exec_tracking.rs` 瘦身：只剩 `InflightAction`（删 `execution_ids`）、`HostRunInfo`、`materialize_unresolved(snapshot, inflight, behavior, reasons)`、`persisted_outcome_ids`；删除 `ExecutionRecord`、`ExecutionRegistrar`、`MemoryRegistrar`、`TrackedBashRunner`、`probe_execution` / `stop_execution`、`OPENDAN_EXECUTION_ID`、启动握手。
- `runtime/mod.rs`：删 `SwitchRegistrar` / `reconcile_execution`；`RuntimeOpenCtx.run: RunBindingSlot`；`AgentRuntime::describe_interrupted(run, action)` 按 runtime 生成“被打断”文本；task-local `CURRENT_TOOL_CTX`。`runtime/tmux.rs`：每条命令一个 window，`kill-window` 结束，不用 /proc / setsid，不再有专用 pane。`runtime/ssh.rs`：包装脚本后台启动并写 `pid` / `exit`，探测不再要求 setsid 与 /proc，`remote_exec_dir` / `read_exec_dir`。
- `xllm.rs`：`XllmToolManager::call_tool(call, ctx)`（可取消工具自行处理，不可取消的在 `cancelled_hard` 时放弃等待 → `Cancelled{effect_unknown:true}`；`Pending` 在 `allow_deferred=false` 时用 resolver 在工具内等最长 30 min）；删除自建 deadline / cancel watch 与 `set_deadline`；`ToolsConfig.shell`（`mode` / `wait_ms` / `timeout_ms` / `max_timeout_ms`，YAML 解析与合并）与 `EffectiveTools.shell: ShellSettings`；auto 模式下随 bash 组注册三个 task 工具；`RUN_RECORD_VERSION = 4`，`RunRecord` 删 `executions`；`XllmDeps.execution_registrar` → `buckyos_tasks`；`settle_previous_executor` 用 `describe_interrupted` 生成文本；新增 `fill_suspended_calls`（接手挂起 run 时按 task 当时状态立即回填，访问不了的 task-mgr 则拒绝）；`execute()` 新增 `Settled` 分支（记为 Interrupted）；`XllmInterrupter::finish`；waist deps `with_tasks(resolver)`。
- `lib.rs` / `tool.rs`：`AgentToolError::Cancelled { message, effect_unknown }`；`AgentTool::cancellable()`（默认 false）与 `TypedTool::cancellable()`。
- `llm_explore.rs` / `llm_understand_media.rs`：子 run 暂停 / 被打断不再返回 Pending，改为 Error 结果并附 `xllm --resume --run <id>`。

**libopendan**：`runner/tools.rs` 删 `RunRegistrar` / `CURRENT_CALL`，`call_tool` 透传 ctx，副作用表用 `TOOL_SHELL`；`runner/live.rs` 删 `LateRegistrar` / `stop_executions` / 恢复核验，`materialize_unresolved` 用 `runtime.describe_interrupted`，`remove_if_safe` 不再核对执行；`reconcile.rs` / `outcome.rs` 删 `stop_executions`（含 `finish_run`）；`session/runs.rs` 删 `executions` API；`runtime/mod.rs` 删 `MemoryRegistrar`；`outcome.rs` 新增 `Settled` 分支（同 Interrupted）；tests 与 `examples/fixtures.rs` 相应修改（kill -9 测试改为“不杀旧工具、结果写明 previous executor exited”）。

**其它**：`opendan`（`ShellTool`、`TOOL_SHELL`、`call_tool(call, ctx)`、`Observation::Pending` 需 `task_id`、`Cancelled.effect_unknown`、`Settled` 分支、字符串改名）；Jarvis `behaviors/*.toml` 与 `aicc_image_cli.inc`；`buckyos-api` task 数据类型 `tool.exec_bash` → `tool.shell`（`TASK_DATA_TYPE_TOOL_SHELL`、`tool.shell/v1`、`ToolShellTaskData`）及 `agent_tool_cli_dev`；`msg_center` 测试数据；`read_aicc_log.py`；`tools/buckyos-agent/readme.md`。

**文档**：《LLM Context 设计》§3.3 / §4 / §8 / §10；`xllm_rust_sdk.md` §3 / §4 / X6 段；Session Directory Protocol（run.json version 4、`exec/` 目录、删 `executions[]`、恢复步骤）；Lease Protocol §5 改为“在途动作”；Agent Session SDK 实现计划（§5.2 伪码、X6 行、相关条目）；Runtime TODO（SSH 执行跟踪条目作废、首版限制）；`doc/llm_context` / `doc/opendan` 各文档与 xllm PRD 中 `exec_bash` / `exec` → `shell`；协议 fixtures 重新生成（`cargo run --manifest-path src/Cargo.toml -p libopendan --example fixtures -- doc/opendan/protocol/fixtures`）。**xAgent.md 未改**（§8，待 review）。

### 11.2 跑了什么验证

- `cargo test -p llm_context`：199 通过（新增 11 个 cancel 测试与 3 个 tasks 测试）。
- `cargo test -p agent_tool --lib`：215 通过、4 ignored（SSH 真机）。含：wait 模式超时杀进程组、`nohup` 起的后台进程在调用返回后存活、执行目录写入 `exit`、auto 模式转 task 后 `wait_task` 拿到最终结果并能从执行目录回读、`shell` 任务取消、run deadline 与 Ctrl-C 经 `ToolCallCtx` 返回 `Cancelled`、xllm 接手挂起 run（访问不了的 task-mgr 拒绝；本地 Unknown 回填后续跑）、task 工具渲染一致。
- `cargo test -p libopendan -- --test-threads=1`：全部通过（l1 17、context_limit 3、crash 20、fixtures 13、runner_basic 4、runner_more 17、self_improve 3）。crash 中 `kill_9_during_shell_reports_interrupted_result_and_leaves_the_command_alone` 验证恢复不阻塞、不杀进程、结果带 `previous executor exited` 且不重放。
- `cargo test -p opendan --lib`：277 通过、1 失败 `agent_config::tests::mini_agent_demo_parses`——它读取 `doc/opendan/mini_agent_demo/`，该目录在 HEAD（`267a426f update openDAN docs`）中已不存在，与本次改动无关。
- `cargo check -p buckyos-api -p msg_center -p agent_tool_cli_dev` 通过；`cargo check --workspace --tests` 通过（exit 0）。

### 11.3 未做与风险

- 宿主侧（lib_opendan TODO §4 / §6）：Session 层等待挂起 task、`waiting_for.refs`、stop 对 task 的传导、挂起期间收到 stop、工具执行期间的 stop 监视任务、run 结束后 task 完成的唤醒。libopendan 仍 `allow_deferred=false`，`Pending` 在工具内等待（最长 30 分钟）。
- buckyos task-mgr resolver 未实现（`XllmDeps.buckyos_tasks` 预留注入点；组合 resolver 按 `local:` 前缀分派）。
- xAgent.md 的 §8 修改清单待 review 后进行。
- macOS 上的 native / tmux 验证见 §11.4，Windows native 验证见 §11.5；SSH 真机测试（`--ignored`）未重跑，`remote_ssh` 的 `kill` 只杀记录的命令 bash 进程，不追子进程。
- buckyos websdk（npm `buckyos` 包）的 `task_mgr_client.d.ts` 仍带 `tool.exec_bash`，属于另一仓库，需随 `tool.shell` 改名同步。
- `shell` 的 `target` 参数已从 schema 删除（runtime 决定执行位置）；`BashRunner::run` 与 `start` 互为默认实现，实现方至少覆盖其一。

### 11.4 macOS 验证（2026-10-03）

环境：macOS 15.7.7（24G720，arm64）、tmux 3.6a、Cargo 1.95.0；验证基线为 `4def5a3bf`，加上下述取消路径修复。

**发现并修复**：`ShellTool::cancel_command` 原来只按 `CommandHandle::cancellable()` 判断是否杀命令；tmux 支持显式取消 task，因此返回 true，导致 Ctrl-C、平滑结束和 run 总超时错误地调用 `kill-window`。`src/frame/agent_tool/src/llm_bash.rs` 改为 tmux 在 run 取消时只 detach、停止等待，结果包含“仍在运行”、session/window 和执行目录。shell 自身超时及显式 task 取消仍走 `kill-window`。`runtime/tests.rs` 新增 `tmux_run_cancellation_keeps_the_command_running`，覆盖打断、平滑结束、deadline，验证及时返回配对的 Cancelled 结果后，命令仍完成并写出 `exit=0` 与输出。

**端到端**：本地 OpenAI 兼容 mock HTTP 服务驱动当前源码的 `agent_tool::run_local_llm::run_subcommand`；临时 Tokio 启动器复用该 CLI 入口，实际向执行器发送 SIGINT / SIGKILL，并读取 PID、run.json、快照和执行目录。wait 模式下 11 个场景全部通过：

| 场景 | native | tmux |
|---|---|---|
| `nohup sleep 300 >/dev/null 2>&1 &` 返回后，run 正常结束 | 后台 PID 存活 | 后台 PID 存活 |
| 另一个长命令执行中 Ctrl-C，再 `--resume --run <id>` | 当前命令结束，Cancelled 配对；恢复不重跑，后台 PID 存活 | 当前命令继续运行，Cancelled 带查看位置；恢复不重跑，后台 PID 存活 |
| 另一个长命令执行中 `--timeout 2` 到期 | 当前命令结束，Cancelled 配对，run 为 limit_reached；后台 PID 存活 | 当前命令继续运行，Cancelled 配对，run 为 limit_reached；后台 PID 存活 |
| 前台命令执行中 kill -9，命令仍运行时 resume | 0.128s 完成恢复；不杀旧命令、不重放，结果为 native 被打断、可能部分执行；后台 PID 存活 | 0.235s 完成恢复；不杀旧命令、不重放，结果为可能仍在运行，并给 session、输出 / exit 文件位置；后台 PID 存活 |
| kill -9 后等命令结束再 resume | — | 0.243s 完成恢复；结果仍为被打断 / unknown，含 exit code 7、stdout / stderr 尾部，不重放；后台 PID 存活 |
| shell 自身 `timeout_ms=500` | 当前命令结束，返回 timed_out 工具结果；后台 PID 存活 | kill-window 结束当前命令，返回 timed_out 工具结果；后台 PID 存活 |

每个场景均检查后台 PID；所有长命令的启动 marker 只出现一次，恢复后 inflight 清空。验证结束后已清理全部测试 PID 和专用 tmux server。后台启动命令在返回前用 `ps` 确认进程已进入 `sleep`：macOS 下 tmux window 立即关闭可能在 nohup 设置忽略 SIGHUP 前结束子进程，独立于 xllm 的 bash 包装命令也能复现；就绪检查用于验证已成功启动的后台进程生命周期。

**自动化验证**（在 `src/` 下；`TMPDIR=/private/tmp` 避免 macOS `/var` 与 `/private/var` 指向同一路径导致字符串断言失败）：

- `TMPDIR=/private/tmp cargo test -p llm_context -- --test-threads=1`：199 通过。
- `TMPDIR=/private/tmp cargo test -p agent_tool --lib -- --test-threads=1`：221 通过、5 ignored（3 个 SSH 真机、2 个开发压缩测试）；包含新增 tmux 取消回归、现有 tmux 超时和按 window id 取消测试。
- `TMPDIR=/private/tmp cargo test -p libopendan --lib --test l1 --test context_limit --test crash --test runner_basic --test runner_more --test self_improve -- --test-threads=1`：70 通过（lib 6、l1 17、context_limit 3、crash 20、runner_basic 4、runner_more 17、self_improve 3），含真实 kill -9 后恢复与 tmux Session 测试。

**其余检查的限制**：完整 `cargo test -p llm_context -p agent_tool -p libopendan -- --test-threads=1` 在 fixtures 的 f04 / f06 / f07 / f10 / f12 失败，其余 8 个 fixture 通过；这些 fixture 保存的 `config.runtime_descriptor.capabilities.os` 为 `linux`，loader 只替换目录、host、uid、hostname，macOS 接手因 descriptor 不同返回 RuntimeMismatch / RecoveryBlocked。`cargo build -p agent_tool_cli_dev --bin agent_tool` 因现有 `opendan/src/agent_session.rs:7356,7405` 使用已删除的 `MsgObject.to_session` 字段失败，因此端到端验证使用上述 CLI 入口的临时启动器。未运行完整 buckyos-build；上述 fixture、OpenDAN 编译问题与 SSH 真机验证仍待处理。

本机验证证据：`/private/tmp/buckyos-macos-long-tool-e2e-final-20261003/report.json`（11 项结果、恢复文本与清理结果）；同目录各场景保留请求、CLI stdout / stderr、run 与快照；脚本为 `/tmp/buckyos_macos_long_tool_verify.py`，启动器源码为 `/private/tmp/buckyos-macos-xllm-driver/`。自动化日志为 `/tmp/buckyos-macos-llm-context-tests.log`、`/tmp/buckyos-macos-agent-tool-final-tests.log`、`/tmp/buckyos-macos-libopendan-final-tests.log`；完整测试及 CLI 构建失败日志分别为 `/tmp/buckyos-macos-long-tool-tests.log`、`/tmp/buckyos-macos-long-tool-build.log`。

### 11.5 Windows 验证（2026-10-03）

环境：Windows 11 Pro 10.0.26200（x64）、Rust / Cargo 1.97.1、uv 0.11.2、Git Bash 5.2.37（x86_64-pc-msys）。基线为 `9a1b155c0cbc`，加上下述 Windows 修复。验证使用原生 Windows Rust 进程和 Git Bash；将 Git Bash 放在 PATH 首部，避免选择本机 System32 的 WSL `bash.exe`。

**发现并修复**：

- `agent_tool/src/llm_bash.rs`、`runtime/mod.rs`、`runtime/ssh.rs`：原来的 `/bin/bash`、Windows 执行目录参数和分号分隔的 PATH 不能直接交给 Git Bash。现在从父进程 PATH 解析 Bash 的完整路径，转换驱动器 / UNC / 扩展路径及 PATH，probe 使用 Windows 的规范 cwd 并报告 `os=windows`。Git Bash 启动器还会把自身目录插到 PATH 首部，因此包装脚本恢复指定 PATH，再用当前 `$BASH` 执行命令，保留 Session / overlay 工具目录优先级。现有 overlay 测试扩展到 Windows，覆盖中文、空格、单引号目录及只有一个 Windows 目录的 PATH；执行目录持久化测试也使用特殊字符路径。
- `lib_opendan/src/lock.rs`：原来的非 Unix 锁身份为 `(0, 文件长度)`，不同空锁文件会冲突，写 holder 后长度改变还会使已持有的 lease 失效。Windows 改用系统文件句柄查询卷号与完整 128 位文件 ID（工作盘 D: 使用 ReFS），Unix 保持 dev / ino；新增不同文件同时加锁、改写后仍有效、替换锁文件后失效的回归。未新增第三方依赖。Windows 的独占锁会阻止其它句柄读取 holder，Busy 的 holder / run_id 因此可以是 None / unknown，相应测试保留 Busy 断言并按平台检查可读性。
- `llm_context/src/prompt_engine.rs`：虚拟 include 根路径用 `has_root()` 判断，修复 Windows 上 `/role.md` 被错误拼成驱动器根目录路径的问题；原有 include 根目录 / 白名单测试通过。
- `lib_opendan/examples/fixtures.rs`、`tests/crash.rs`、`tests/l1.rs`：真实强制结束改用标准库 `Child::kill()`（Windows 为 TerminateProcess，Unix 仍为 SIGKILL），Unix inode / chmod 检查按平台编译；继续验证强制退出后锁释放、恢复不重放且不停止遗留命令。
- fixtures loader / replacement helper：替换 Windows 路径时正确转义 JSON，按当前 OS 调整保存的 runtime capability，并规范化 Windows 路径；补回生成器创建、Git 未保存的 f10 空 `ws` 目录。13 个 fixture 全部通过，解决 §11.4 提到的 loader 限制；macOS 未在本次重跑。其它测试调整 CRLF、路径分隔符、Git Bash `pwd -W` 和 MSYS 路径别名断言。
- `llm_bash.rs`、`tasks.rs`：说明、超时 / 取消结果明确 Windows 只结束直接子进程，后代可能继续运行；保持 §3.2 的标准进程语义，不增加进程树追杀。无协议字段、持久格式、前端或依赖声明变化；xAgent.md 未改。

**自动化回归**（PowerShell，`src/` 下）：

```powershell
$env:PATH = 'C:/Program Files/Git/bin;C:/Program Files/Git/usr/bin;' + $env:PATH
$env:CARGO_BUILD_JOBS = '2'
cargo test -p llm_context -p agent_tool -p libopendan -- --test-threads=1 --skip workspace::tests::external_workspace_tools_bind_and_list_from_agent_tool_backend
cargo check -p llm_context -p agent_tool -p libopendan --tests
```

- `llm_context`：199 通过。
- `agent_tool`：208 通过、5 ignored、1 filtered；另有 1 个 doc-test ignored。新增 Windows overlay 覆盖，包含 native 超时 / 取消、auto 转 task、task 查询 / 等待 / 取消、run deadline、恢复与 Session 接手。
- `libopendan`：85 通过、1 ignored（lib 6、context_limit 3、crash 20、fixtures 13、l1 19、runner_basic 4、runner_more 17、self_improve 3）。
- `cargo check -p llm_context -p agent_tool -p libopendan --tests`：通过，exit 0；保留原有 unused / private_interfaces 等警告。
- 文件锁采用完整 128 位 ID 后再跑 `cargo test -p libopendan -- --test-threads=1`；另外分别在默认 C: 临时目录（NTFS）和 `TEMP=TMP=D:/tmp/buckyos-long-tool-refs-20261003`（ReFS）运行 l1，均 19 通过，覆盖文件身份、替换检测、跨进程抢锁与强制退出后的恢复。
- 唯一显式排除的测试 `external_workspace_tools_bind_and_list_from_agent_tool_backend` 需要创建 Windows symlink，本机实际运行失败为 `os error 1314`（缺少所需特权）；未更改系统权限或开发者模式。5 个 agent_tool ignored 是 3 个 SSH 真机、2 个开发压缩测试；libopendan ignored 需要正在运行的 DV kmsg。没有 tmux，相关测试会自行跳过，因此不算 Windows tmux 验证通过。

**命令行端到端**：本地 OpenAI 兼容 mock HTTP 服务驱动当前源码的 `agent_tool::run_local_llm::run_subcommand`，临时 Tokio 启动器复用 CLI 入口；在专用隐藏 Windows 控制台实际发送 Ctrl-C，崩溃用 TerminateProcess，检查真实 Windows PID、请求、run.json、快照和执行目录。工作路径包含中文、空格、单引号。8 个场景全部通过：

| 场景 | Windows native 结果 |
|---|---|
| nohup 后台命令返回，run 正常结束 | 后台 PID 仍存活 |
| 前台长命令中 Ctrl-C，再 resume | 直接子进程结束，Cancelled 配对，后台 PID 存活，恢复不重放 |
| 前台长命令中 run `--timeout 2` | Cancelled 配对，run 为 limit_reached，后台 PID 存活 |
| 执行器被强制结束，旧命令仍运行时 resume | 约 0.96s 完成恢复，结果 unknown；不核验 / 不停止旧命令，不重放，后台 PID 存活 |
| 执行器被强制结束，旧命令结束后 resume | 约 0.98s 完成恢复，读到 exit code 7 与 stdout / stderr 尾部，不重放，后台 PID 存活 |
| shell 自身 `timeout_ms=350` | 返回 timed_out，直接子进程结束，后台 PID 存活 |
| auto 转 task，再 wait_task | 返回 task_id、最终输出和退出码，后台 PID 存活 |
| auto 转 task，再 cancel_task | 返回 Cancelled 并注明只结束直接子进程，后台 PID 存活 |

所有恢复场景的长命令启动 marker 均只有一次，恢复后 inflight 清空；每项均确认后台 PID 存活，验证结束后清理测试 PID 并确认结束。后台启动时等待 `ps` 的命令名成为 sleep，再记录 Windows PID，避免 MSYS exec 换 PID 的启动竞态。启动器源码仅保留在证据目录，已从仓库删除。

**全仓 / 构建限制**：实际执行 `cargo check --workspace --tests` 失败，现有 msg_center 测试仍引用已删除的 `MsgRelType` / `MsgRelation` / `CyfsNamedObjectEncoding` 等 API（146 个错误），opendan 的 `agent_session.rs:7356,7405` 仍写已删除的 `MsgObject.to_session`。实际执行 `uv run buckyos-build.py --skip-web` 的 Windows 原生 release 构建也因同一 opendan 字段错误失败。因此不能声明全仓测试或 buckyos-build 通过；未运行 Web 构建、安装启动、DV / SSH 真机或 Windows tmux 验证。Windows 文件锁验证在本机 NTFS / ReFS 上完成，其它文件系统未验证。上述范围外的消息协议问题未改动。

证据目录：`C:/Users/water/AppData/Local/Temp/buckyos-windows-long-tool-20261003/`。最终端到端结果为 `cli-report.json`，详细请求、stdout / stderr、run / 快照保留在 `cli-20261003-013606/`；脚本 `verify_cli.py`、启动器 `windows_long_tool_driver.rs`，最终 CLI 构建 / 运行日志 `cli-driver-final-source-build.log` / `cli-e2e-final-source.log`。自动化最终日志 `windows-regression-final.log`、`libopendan-final-source.log`、`affected-check-final.log`，两种文件系统的 l1 日志为 `windows-file-id-ntfs.log` / `windows-file-id-refs.log`；全仓检查 / release 构建失败日志 `workspace-check.log` / `buckyos-build.log`。
