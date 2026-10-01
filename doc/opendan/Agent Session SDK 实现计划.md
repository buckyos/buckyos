# Agent Session SDK 实现计划

**项目：OpenDAN / libOpenDAN**
**版本：0.10（草案）｜日期：2026-09-29**
**需求基线：** [《Agent Session SDK 化核心需求》](<Agent Session SDK化核心需求.md>)（下文简称“需求”，条目编号 S-xx / A-xx 沿用）

**现状依据：**
- [Agent Session](<Agent Session.md>)、[Agent RootFS](<Agent RootFS.md>)、[LLM Context 设计](<LLM Context 设计.md>)、[xllm Rust SDK](../agent_tool/xllm_rust_sdk.md)、[kmsg](../arch/kmsg.md)、[NewOpenDANRuntime](../../notepads/NewOpenDANRuntime.md)
- [agent-did-object-lib 需求](../../notepads/Agent元能力/agent-did-object-lib%20需求.md)、[Agent DID-Object Protocol Spec](../../notepads/Agent元能力/Agent%20DID-Object%20Protocol%20Spec.md)、[Agent Memory v2](../../notepads/Agent元能力/Agent%20Memory%20v2.md)
- 当前代码（2026-09-29）：`src/frame/opendan`、`llm_context`、`agent_tool`、`kernel/buckyos-api`、`kernel/kmsg`、`frame/msg_center`；对照代码的核对结果见 §1.5
- [Agent Memory 认知管理需求](<Agent Memroy 认知管理需求.md>)（已补回；设计尚未冻结，对本计划没有影响）

> **实现落地**（2026-09-29，Rust 参考实现）
>
> - 代码：`src/frame/lib_opendan`（package `libopendan`）；LX 改动在 `llm_context`（`CheckpointHook` / `inject` / 快照 `host` 与 `snapshot_version`）与 `agent_tool`（`exec_tracking`、RunRecord 宿主字段、`prepare_hosted`、resume 检查）。反写的 Spec、JSON Schema 与 fixtures 在 [`protocol/`](protocol/README.md)。
> - 已完成：L1、L2（native + tmux）、L3（work session；普通 / fork / independent 切换）、L4（感知、self_improve 锁与整理游标、认知门面）、L5（产物登记、decide、discard 报告）、L6；LX 的 X1 ~ X6 与 X8（宿主 run 的 step 渲染不带时间戳）；真实 kmsg 服务的 DV 用例 `tests/dv_kmsg.rs`（`--ignored`，在 DV Test OOD 上以 root 运行）。
> - 与本文的差异：X4 / X5 合为一个异步 `CheckpointHook`（外层快照、可注入；function call 模式在每次推理前调用，behavior 模式只在外层 Step 边界调用）；`AgentRuntime` 以 `bash_runner(env, registrar)` + `reconcile_execution` 表达执行准备与核对；执行跟踪按环境标记 `OPENDAN_EXECUTION_ID` 扫描 `/proc`（非 Linux 一律 Unknown → RecoveryBlocked）；`recent_keys` 放在 state.json 顶层；state 增加 `stop_requested` / `internal_continuation` / `process_result`，live_run / process_stack 增加 `flushed_input_seq`（behavior run 按身份记录已写入部分）；worklog 增加 `created` / `change_dropped` / `control_applied`。
> - X7（2026-09-30）：waist 能力完成——真正产出 `ContextLimitReached`（阈值 / 已知窗口 / Provider 结构化拒绝）与 `PendingTool`（`allow_deferred`），快照版本 2，传统与 behavior（action、step 内层原生工具）都能挂起后恢复，behavior 用 `RewrittenSteps` 重写；见《LLM Context 设计》§9.5。宿主接入：libopendan 实现上下文上限中途重写（先 flush、压缩 summary.json、新 history epoch，见 Session Directory Protocol §7），PendingTool 仍不接入（`allow_deferred=false`，遇到时 RecoveryBlocked）；xllm 压缩续跑与接手挂起快照；OpenDAN 只做类型适配。
> - Round / Step / Turn 术语统一（2026-10-01，breaking change，定义见 [LLM Context readme](../llm_context/readme.md)）：Round = 一次宿主推理（`LlmClient::infer`）；Step = behavior 的 `StepRecord`，身份 `(run_id, step_index)`；Turn = Session 的一次逻辑 Input → result。state.json 的 `round` 改为 `turn_seq` / `open_turn` / `turns_completed`：没有进行中的 Turn 时提交的输入批次（`(run_id, input_seq)`）开启 Turn，交接、观察注入、可恢复挂起、history epoch 重写与重启都延续它，只有 session 在 `finish_run` 中写 `turn_ended` 关闭它（§8.3）。`live_run.rounds` → `turns`，receipt 的 `round` / `opens_round` → `turn` / `opens_turn`，`epoch_round` → `epoch_turn`，`flushed_step` 拆为 `flushed_message_count`（function call）与 `flushed_step_index`（behavior）；worklog 改为 `turn_started` / `input_batch` / `assistant_message` / `step`（带 `step_index`）/ `turn_ended`，function call 的 response 不再写成 `step`；`end_condition.type = max_turns`；`StopWhen::MaxOutcomes`；`round_digest` → `run_digest`；`static.json` 的 `rounds` 为推理尝试数（另有 `rounds_failed` / `rounds_interrupted`、`turns`，去掉 `llm_requests`）；`begin_round` / `commit_round` → `commit_input_batch` / `handle_context_outcome`；`TurnHook` → `InferenceHook`；工具额度 `max_rounds` → `max_tool_iterations`。schema 升为 `session_state/2`、`session_config/2`、`session_summary/2`，快照版本 3，旧版本一律拒绝。下文示例与伪代码已同步；opendan 本身只做编译适配，待下一阶段重构。
> - 未完成：libopendan / xllm 的 deferred 工具（task_mgr）回填、ToolSpec 的 effect 字段与 `ToolUse.args` 规范键序（libopendan 按工具名分类、worklog 内规范化）；OpenDAN `BehaviorAssembler`（behaviors 配置、prompt_env、HintRecallEngine）与 session-aware 工具未移植（用 `DefaultAssembler`、`extensions.opendan.process_modes` 与 `agent-session` CLI 代替）；1 GB worklog 基准（以数 MB 文件验证读取量有界）。UI session、kRPC、DID Object 宿主按本文后移。

> **v0.10 Review 修订**（2026-09-29）
>
> 1. **sid 全局唯一**：覆盖不同 Agent、App 和 owner；统一随机与确定性生成规则，队列仍按 sid 命名（§4.1、§4.2）。
> 2. **工具结果先持久化，再清除在途标记**：异常、取消与 checkpoint 失败均保留恢复证据（§8.5）。
> 3. **文件锁与工具生命周期分别校验**：接管前确认旧工具执行已经停止，无法确认则阻塞恢复；runner 被 kill 不代表工具子进程退出（§5.2、§7.2）。
> 4. **恢复失败保留现场**：不支持的版本、损坏或缺失的快照返回明确错误，不清空 live_run 后自动重跑（§8.6）。
> 5. **输入提交可恢复**：所有进入上下文的输入都有结构化 receipt，与快照一起持久化；恢复时先补齐 state 消费状态，再取新输入（§4.5、§8.3）。
> 6. **产物 head 的所有移动都持 artifact 锁**：discard 在锁内重读 head，并检查回退版本有效性（§6.5）。
> 7. **runtime 准备可重试**：binding 已存在也必须修复、验证工具环境，成功后才允许推理（§7.1）。
>
> **v0.9 变更**（2026-09-29 反馈 V1 ~ V6，见 §1.3）
>
> 1. **协议边界收窄到目录结构**（V1）：Agent Session 的目录结构（§4、§5）是本期要先定稿的关键协议；Runner 是实现，不是协议。第三方 TS 应用将来使用 buckyos-websdk 中的 ts-runner。
> 2. **从 work session 出发**（V1）：本期只做 work session。UI session、msg-center 输入等依赖服务确认的部分后移；opendan 仍不改动。
> 3. **`runs/` 就是 xllm 的 run 目录**（V2）：native runtime 下，跑到一半的 llm_context 原则上可以交给 xllm 接手继续。llm_context / xllm 的改进因此是本期重点（§8.7）。
> 4. **Agent State 经 `AgentStateClient` 访问**（V3）：首版用纯文件 + 文件锁实现；OpenDAN 改造完成后部分操作迁到 kRPC；DFS 上线后对文件锁透明（§6.6）。
> 5. **文件锁重新设计**（V4）：推进权 = 长期持有的排他文件锁（与 xllm 的 run 锁同一模式），持有者退出即释放；去掉 TTL、续约和时钟依赖；锁文件只原地改写，永不替换（§5）。
> 6. **活动 Session 视图**（V5，新增 §6.7）：每个 session 的上下文都包含“系统里有哪些 session 在运行、大概在改什么”，由 Agent 自己避让。
> 7. **实现先行，反写 Spec**（V6）：先用 Rust 实现，再根据实现结果反写协议级 Spec 与 fixtures（原 L0 的“协议文档 + fixtures 先行”改为 L6）。
> 8. **按现状核对修正**（§1.5）：msg-center 的 `did + session_id` inbox 已经实现、记录状态是 `Read`、`.llm_context` 的真实字段、principal 格式、kmsg 的实际语义等。
> 9. **目录结构定稿**（2026-09-29 确认 §15.1 A 的 7 项，L0 完成）：`lease.json` 兼作锁文件；`runs/` 完全用 xllm 布局；**保留最后一次 run 的 llm context 状态**，每个 behavior process 对应一个 run（§4.4）；统计文件名为 `static.json`；接受 `.runtime/bin` 随主机绑定；附件可以用相对路径或 NamedStore 对象 id 引用（§4.1）；activity 的三种来源。
>
> **v0.8 变更**
>
> 1. **Q12 已定**：DFS 在设计上基本是独占单写（只有一个 client 能拿到文件写锁），原子语义通常强于单机系统。因此：
>    - §9.1 的文件原语全部满足；
>    - 锁直接建立在“文件写锁”上：单节点用 flock，多节点用 DFS 独占写锁；v0.10 修正：工具副作用还需要独立的执行生命周期校验，文件锁本身不提供该保证（§5.2）；
>    - 去掉 `lease_mode`、OpenDAN 锁服务和 service 模式的安全余量（§5）。
> 2. **Q14 / Q16 已定**：msg-center 的 inbox 正在改为以 `did + session_id` 标识，每个 UI session 有自己的 inbox，权限可以正确区分。
>    - UI session 直接消费自己的 inbox，OpenDAN 不再转投消息。
>    - session 的身份与权限由驱动它的进程的 appid 决定；回复消息、调用 LLM 都用这个身份（§4.5）。
> 3. 待确认问题清零（§15.1）。
>
> **v0.7 变更**
>
> 1. **llm_context 的构建与读取策略**：先读 `summary.json`，再从 `state.json` 记录的已提交末尾**反向读** `worklog.jsonl`，读到起点或预算用尽就停止。运行时从不全量扫描 worklog（§4.4）。
> 2. **Q8**：session 类型沿用 `work`，不改名为 task。
> 3. **Q12**：计划上线的 DFS 支持 ACL，由 BuckyOS 管理权限。
> 4. **Q13**：session 里的产物通常是一次性的，长期产物放 workspace，于是问题变成“Agent 内部 workspace 与外部 workspace”之分；**AgentSession 协议不承担 workspace 版本化的复杂度**。因此删除 v0.6 的交付方式与版本化策略（git_worktree / staged_copy / in_place），Agent State 的产物列表只负责登记与指向（§6.5）。
> 5. **Q14**：inbox 的读取权限由 BuckyOS 权限机制决定。
>    - OpenDAN 主进程有权读 Agent 的各类 inbox。
>    - 其它 runner 通常创建 work session。
>    - 如果权限允许某个 app 读取特定 inbox，这个 app 也可以创建并驱动对应的 UI session，也就是企业软件集成 Agent Chat Box（§4.5、§10.2）。
>
> 历史：v0.4 采用你整理的 session 目录，输入改走 kmsg；v0.5 明确 summary.json 只服务于压缩；v0.6 确定 `state.json` 为提交点、worklog 严格只追加。

---

## 0. 结论先行

本期把“以某个 Agent 身份推进一个 Session”的能力从 opendan 进程中剥离出来，做成独立的 **libOpenDAN**，从 **work session** 出发：

- **Agent Session**：自包含、与位置无关的目录协议，是本期要先定稿的**关键协议**。任何进程按协议在任意位置建出 session 目录，就能以某个 Agent 的身份工作。
- **Agent State**：Agent 跨 session 的状态，落在 AgentRoot 上，包括 session 登记表（session mgr）、活动视图、感知、认知和产物列表。统一经 `AgentStateClient` 访问；首版直接读写文件，用文件锁协调。
- **Agent Runtime**：为 `exec_bash` 提供执行环境（native 或 tmux）。session 首次推进时绑定 runtime，之后不再改变。
- **Session Runner**：把以上三者与 LM Context 组合起来，把 session 推进到结束条件。**Runner 是实现，不是协议**：Rust 版在 libopendan，TS 版将来是 buckyos-websdk 的 ts-runner。

session 的 `runs/` 直接采用 xllm 的 run 目录：native runtime 下，跑到一半的 llm_context 原则上可以交给 xllm 接手。

先实现，再根据实现结果反写协议级 Spec，指导其它语言。opendan 服务本期不动；它将来依赖 libOpenDAN，只保留托管职责（附录 A）。

**关键决策**

| # | 决策 | 主要对应 |
|---|---|---|
| D1 | 文件即真相。持久格式只用 JSON / JSONL / Markdown；SQLite 只做可删除、可重建的派生缓存 | S-03、跨语言 |
| D2 | **Session 目录**：<br>- 目录本身存放产物（通常一次性）；<br>- `.opendan_agent_session/` 存放状态；<br>- `.runtime/` 存放 runtime 配置。<br>只通过 `agent_did` 引用 Agent，位置不限 | 你的目录设计 |
| D3 | **Agent State 经 `AgentStateClient` 访问**：首版直接读写 AgentRoot 上的文件、用文件锁协调；OpenDAN 改造后部分操作迁到 kRPC；DFS 上线后对文件锁透明。不为访问 session 开 RPC。Agent 默认能看到自己的全部 session，可以配置为只看状态摘要 | S-03、Q10、Q12、V3 |
| D4 | **Session 登记表**是发现 Agent 全部 session 的唯一入口。驱动者每次提交后回报状态摘要（rev 单调），并追加感知 | S-14、Q9 |
| D5 | **Session 绑定驱动者身份**，同身份的不同进程可以接手。单写者分区；推进权 = **长期持有的排他文件锁**（与 xllm 的 run 锁同一模式），持有者退出即释放，没有 TTL 与续约；接管执行前还必须确认旧工具已停止，无法确认则阻塞恢复；DFS 上线后也遵循此边界 | S-22、A-14、Q9、Q12、V4 |
| D6 | **输入走 BuckyOS 基础设施**：<br>- 每个 session 一条 kmsg 队列，承载 msg / event / change / control / perception；<br>- kevent 负责唤醒；<br>- UI 消息来自 msg-center inbox（以 `did + session_id` 标识，每个 UI session 一个）——**UI session 后移**，等 msg-center 等服务确认。<br>“进入推理 = commit pop”落在各输入源的确认上 | S-15 ~ S-19、S-23、Q14、V1 |
| D7 | **`state.json` 是提交点**（小文件，整体原子替换）。**`worklog.jsonl` 严格只追加**：每次 llm_context run 结束时批量写入该 run 的历史，随后由 state.json 提交确认 | S-23 |
| D8 | **下一次 llm_context 的构成**：<br>- 有未结束的 run，就从 `runs/`（xllm run 目录）直接恢复；<br>- 否则先读 `summary.json`（历史摘要 + 起点 + 机械压缩配置），再从末尾反向读 worklog，读到起点为止。<br>运行时从不全量扫描 worklog | S-04、Q5 |
| D9 | 首次推进时绑定 runtime（含 workdir），之后不变；绑定失败发生在任何推理之前，没有成本 | S-07、Q3 |
| D10 | **产物**：session 产物放在 session 目录，通常一次性；长期产物放 workspace（Agent 内部 / 外部）。workspace 的版本化与回滚**不进 Session 协议**；Agent State 的产物列表只登记和指向 | S-24 ~ S-26、Q13 |
| D11 | 崩溃恢复**复用 llm_context / xllm 的 checkpoint / resume**，最细到一次 do-action 之后；输入 receipt 与工具结果随快照持久化；恢复失败保留现场；native runtime 下 xllm 满足恢复检查后可以接手未结束的 run | S-04、A-03、Q5、V2 |
| D12 | session 的身份与权限由**驱动它的进程的 appid** 决定：读 inbox、回复消息、调用 LLM 都用这个身份。外部 runner 即 App 身份，OpenDAN 托管时即 OpenDAN 的 appid（实际是该 Agent 的 runtime app，§1.4 A9） | S-08、Q7、Q16 |
| D13 | DID Object 宿主组件**随 OpenDAN 改造实施**：面向 Agent 的 `read` / `xcall` 和外部访问（经 cyfs-gateway），**不是** runner 的数据通路 | Q6 |
| D14 | **活动 Session 视图**：每个 session 把 activity（在做什么、大概在改什么）写入状态并回报登记表；runner 把其它活动 session 渲染进上下文，由 Agent 自己避让。它是协作提示，不是锁 | S-25、A-13、V5 |
| D15 | **目录结构先定，Spec 反写**：目录布局、提交顺序、锁语义在本计划中定稿；字段级 Spec 与 fixtures 在 Rust 实现后反写（L6）。Runner 不是协议 | S-03、V1、V6 |

---

## 1. 范围、已定决策与假设

### 1.1 你给出的思路（复述）

新建工程 libOpenDAN，原则上其中的代码都要能跨语言实现。它包括三部分：

1. **基于文件系统的 Agent State**（跨 session 状态）：session mgr、感知管理、认知管理、产物列表。
2. **Agent Session**：用协议级文档确定目录结构，实现创建和读取；本计划补上了推进与恢复。session 目录可以放在 AgentRoot 之外。
3. **Agent Runtime**：核心是为 `exec_bash` 提供环境，可以是 OS native，也可以是(容器里)的 tmux session。session 一旦确定了 runtime，就不再修改。

OpenDAN 的后续职责（本期不实施，见附录 A）：

- 基于 libOpenDAN，提供 Agent State 的 DID Object 访问服务，并承接 `AgentStateClient` 的 kRPC 实现（v0.9，§6.6）；锁仍是文件锁（§5）。
- 创建默认 Agent Runtime（paios 容器内 Agent 的私有 infra）。
- 用协程托管默认 session：基于 msg-tunnel 的 UI session，以及定期运行的 self-improve。

### 1.2 本期范围

| 本期交付 | 本期不做 |
|---|---|
| **Agent Session 目录结构定稿**（§4、§5；§15.1 的目录项确认） | 修改 opendan（附录 A 只描述对接方式） |
| `src/frame/libopendan` crate（work session 优先）：<br>- Session 目录<br>- 文件锁（§5）<br>- `AgentStateClient` 文件实现：登记表、活动视图、感知、认知门面、产物列表<br>- Runtime（native 优先，tmux 移植）<br>- Rust Runner<br>- kmsg 输入 | UI session 与 msg-center 输入（依赖 msg-center 等服务的确认，后移）；`agent_tool` CLI 改依赖（随 opendan 切换一起改） |
| **llm_context / xllm 改进**（§8.7）：run 目录公开 API、宿主装配的 run、快照版本化、checkpoint 与观察钩子、effect 与在途记录；使 xllm 能接手 session 的 run | `AgentStateClient` 的 kRPC 实现、DID Object 宿主组件（随 OpenDAN 改造） |
| 实现完成后**反写**协议级 Spec、JSON Schema 与 fixtures（L6）；libopendan 的开发 CLI（cargo example） | buckyos-websdk 的 ts-runner（依据反写的 Spec，后续）；msg-tunnel UI session 的发现与托管、self-improve 定时、agent.delegate 托管（属于 OpenDAN） |
| | **workspace 的版本化、合并与回滚**（Agent 内部 / 外部 workspace，另行设计）；Memory Graph 内部算法；提示词 |

### 1.3 已定决策（2026-09-29 多轮反馈）

| 项 | 结论 |
|---|---|
| 范围 | 本期只做 libopendan，不动 opendan；opendan 之后依赖 libopendan |
| Session 位置与目录 | 可以放在 AgentRoot 之外；目录结构按你的整理（§4.1） |
| inbox / outbox / changes | 用 BuckyOS 基础设施（message center、kmsg 等），不用文件系统重新实现 |
| summary.json | 只为 session history 压缩准备：历史摘要 + 起点 + 机械压缩配置 |
| state.json / worklog | 用两个文件：state.json 是提交点；worklog 严格只追加，每次 run 结束时批量写入 |
| llm_context 构建 | 先读 summary.json，再从末尾反向读 worklog（§4.4） |
| Q1 self-improve 启动 | 属于启动条件检查，至少要求 Agent 空闲（附录 A.5） |
| Q2 本机直写 | 同意。数据一律直接读写文件（DFS）；只有多节点下的锁需要走服务 |
| Q3 绑定时机 | 首次推进时绑定；启动失败发生在首次推理之前，没有成本 |
| Q4 + Q13 产物 | session 产物通常一次性；长期产物放 workspace（Agent 内部 / 外部），其版本化不由 Session 协议承担。默认 paios runtime 内置 git，可供 Agent 内部 workspace 使用 |
| Q5 崩溃恢复 | 复用 llm_context 的崩溃恢复策略，最细到一次 do-action 之后 |
| Q6 访问入口 | 正式的 DID Object 协议访问由 cyfs-gateway 路由到 OpenDAN |
| Q7 LLM 身份 | 用 App 身份 |
| Q8 命名 | session 类型沿用 `work`（Work Session），保持惯性 |
| Q9 登记与推进者 | 需要记录 session 上次被哪个 runner 推进；推进身份不变，但两次推进可以发生在不同进程 |
| Q10 可见性 | Agent 原则上能看到自己的所有 session，也可以通过配置拒绝；状态共享通过 DFS 实现，App 不需要开 RPC |
| Q12 DFS | 计划上线的 DFS 支持 ACL（由 BuckyOS 管理权限）。它在设计上基本是独占单写：只有一个 client 能拿到文件写锁，原子语义通常强于单机系统。因此 §9.1 的原语全部满足。v0.9：锁只依赖“可长期持有的排他文件锁”（§5），DFS 上线后对它透明；DFS 目前尚未上线（§1.5） |
| Q14 输入权限 | msg-center 的 inbox 改为以 `did + session_id` 标识（改造进行中），权限按 inbox 区分：<br>- OpenDAN 主进程可以读 Agent 的各类 inbox；<br>- 其它 runner 通常创建 work session；<br>- 获准读取某个 session inbox 的 app 可以创建并驱动该 UI session（企业软件集成 Agent Chat Box）<br>v0.9 核对：inbox 已按 `did + session_id` 划分（§1.5）；UI session 本期后移（V1） |
| Q16 身份 | session 的身份与权限由驱动它的进程的 appid 决定；回复消息也以该身份发送 |
| V1 协议边界与起点 | Agent Session 的目录结构是关键协议，先定稿；Runner 的实现不是协议，第三方 TS 应用将来用 buckyos-websdk 的 ts-runner。从 work session 出发做 libOpenDAN，开发中不动现有 opendan；msg-center 等服务的确认可以晚一点 |
| V2 llm_context | llm_context 的改进很重要。xllm 的 run 目录已经是一定程度的协议；runtime 是 native OS 时，跑到一半的 llm_context 原则上可以交给 xllm 接手（§4.4、§8.7） |
| V3 Agent State 访问 | 通过 libOpenDAN 的 `AgentStateClient` 抽象。先用纯文件 + 文件锁实现一版；OpenDAN 改造完成后，部分操作迁到 kRPC；DFS 上线后对文件锁透明（§6.6） |
| V4 文件锁 | 重新设计（§5） |
| V5 活动 session | 所有 session 的一个重要上下文：系统里有多少 AgentSession 在运行、大概在修改什么，让 Agent 自己避让同时修改同一个东西（§6.7） |
| V6 Spec | 先实现 AgentSession，再根据实现结果反写协议级 Spec，指导其它语言的实现（L6） |
| 目录定稿（L0） | ① `lease.json` 兼作锁文件；② `runs/<run_id>/` 完全用 xllm 布局，本 run 消费的输入记在 state.json（2026-10-01 起按 Turn 归并为 `live_run.turns`）；③ 选 a：值得保留最后一次 run 的 llm context 状态，被挂起的 process 所在的 run 由 state.json 引用、不删除；④ 统计文件名 `static.json`；⑤ 接受 `.runtime/bin` 随主机绑定；⑥ 附件用相对路径或 NamedStore 对象 id 都可以，由提示词决定（NamedStore 支持 LocalLink）；⑦ activity 的三种来源 |
| Review 补充 | sid 全局唯一。目录布局不变；v0.10 补齐输入 receipt、工具执行与 checkpoint 的提交纪律、恢复阻塞、artifact head 锁和 runtime 准备重试；对应验收见 L1 / LX / L2 / L3 / L5 |

### 1.4 假设

- **A2**：需求引用的《Agent Memory 认知管理需求》（`Agent Memroy 认知管理需求.md`）已补回，设计尚未冻结。感知 / 认知的内部 schema 以该文档为准，本计划只定义接口面；该文档的变化不影响本计划的目录与锁协议。
- **A3**：新协议不读取旧格式；opendan 切换时清理旧数据。
- **A5**：代码从 opendan **复制后改造**到 libopendan，过渡期两份并存（§12）。
- **A7**：`runns` 按 `runs` 处理；统计文件名确认为 `static.json`。
- **A8**：文件版 `AgentStateClient` 要求 runner 能看到 AgentRoot 路径。当前 AgentRoot 位于该 Agent 的 runtime app 的私有数据目录（§1.5），其它 App 的容器默认看不到。因此首版的“外部 App 驱动”只在能看到 AgentRoot 的环境成立（开发 CLI、测试、同主机或显式挂载）；跨容器的正式形态依赖 kRPC 实现或 DFS。
- **A9**：principal 使用 BuckyOS session token 中的格式 `app:<appid>@<owner>`（v0.8 示例中的 `did:app:…` 作废）。OpenDAN 的 appid 实际是每个 Agent 各自的 runtime app（例如 `jarvis.buckyos.bns.did`）；RBAC 中残留的 `system:opendan` 与任务记录里的 `app_id: "opendan"` 属于遗留。
- **需求术语映射**：需求里的 Task Session 即本文的 Work Session（`kind = work`）。

### 1.5 现状核对（2026-09-29）

对照代码逐项核对 v0.8 依赖的前提。下列事实会随各组件演进而变化，实施前以代码为准。

| 组件 | 现状 | 对本计划的影响 |
|---|---|---|
| 存储 / DFS | 没有可挂载的 DFS：`nfs_server` 是 HTTP RPC（NFSP），多数调用不鉴权；cyfs-ndn 的 `fs_daemon` 未部署，没有 flock 与 ACL。唯一真实的锁是单机 flock（xllm 的 run 锁即如此） | 锁只依赖 flock 语义（§5）；DFS 上线时要提供同一语义 |
| AgentRoot | 位于 `$BUCKYOS_ROOT/data/home/<owner>/.local/share/<runtime_appid>/agents/<agent_id>`，即 Agent runtime app 的私有数据目录 | 文件版 `AgentStateClient` 的可达性前提（A8） |
| 权限 | RBAC 里没有 AgentRoot / 文件路径这类资源；跨主体授权只能写字面 `p` 行，而 policy tail 会被 scheduler 整体重写；`DIDObjectRequestContext` 不带鉴权信息 | “以 Agent 身份”的授权模型另行设计（§15.1）；DID Object 宿主后移 |
| kmsg | `create_queue` / `subscribe` 不幂等（已存在时返回字符串错误）；`commit_ack` 直接把游标设为 `index + 1`（可回退、无 fencing）；游标不落盘（D-09），服务重启可能丢订阅；没有权限校验（D-07）；retention 未实现（D-06）；`delete_message_before` 与 post 有竞态，会复用 index 覆盖消息；sub_id 在所有队列间共用一个命名空间 | §4.5 的 kmsg 使用规则 |
| kevent | 跨节点投递未接线；事件名每段只允许字母、数字和 `_ - .`（DID 不能直接进路径）；TS SDK 没有 publish | wake_event 命名规则；唤醒丢失靠轮询兜底 |
| msg-center | `did + session_id` inbox **已实现**（`MailboxAddress`、`list_mailboxes`）；记录状态是 `Read`（没有 `Readed`）；`Reading` 没有租约，take 之后崩溃会一直卡住；inbox 的 session_id 取自发送方可控的 `thread.topic`；take 需要 inbox 的写权限，回复还需要 `sent/<did>/<session>` 的写权限 | UI session 后移时一并处理（§4.5） |
| llm_context | `PendingTool` / `ContextLimitReached` 在 v1 中不产出；`TurnHook`（2026-10-01 改名 `InferenceHook`）同步只读，但已有返回错误阻止推理的语义，behavior 模式下拿到的是扁平化的内层快照；没有 ObservationHook；工具没有 effect 分类与幂等键；快照没有版本字段，`AiMessage` 等类型是 `deny_unknown_fields` | §8.7 改进清单 |
| xllm | run 目录（`run.json` + `snapshots/` + `.lock`）已经存在；`.lock` 是长期持有的 flock，进程消失即判定为中断；`RunRecord` 没有在途动作字段；写入不 fsync；`RunStore` 没有公开的 create / remove；`.llm_context` 是 YAML、严格键（`loop_model`，`groups` / `sections` 在 `prompt` 下）；没有 TS 版 | §4.2、§4.4、§8.7 |
| opendan | 若干现有状态在新协议中还没有位置：fork / independent 的 process 快照、topic 权重、命令消息、interrupt 顺序屏障、task_mgr 反馈路径、全局 SQLite worklog 等 | §12、§15.1 |

---

## 2. 总体架构

### 2.1 分层与部署形态

```text
 应用进程（App 身份）                                OpenDAN（后续版本，附录 A；Agent runtime app 的 appid）
 ┌───────────────────────────────────────┐        ┌──────────────────────────────────────────┐
 │ libOpenDAN（Rust）| ts-runner（TS，后续）│        │ libOpenDAN ＋ 协程托管                     │
 │  SessionRunner ─ LM Context（xllm run） │        │ AgentStateClient 的 kRPC 服务端（后续）     │
 │  AgentStateClient（文件 | kRPC）        │        │ DID Object 宿主组件（后续）◄───────────────┼── cyfs-gateway
 │  Runtime（native | tmux）              │        └────────────────────┬─────────────────────┘
 └──┬──────────┬───────────┬─────────────┘                             │
    │ 文件      │ 文件 + 锁  │ kmsg / kevent                              │ 文件 / kmsg / msg-center
    ▼          ▼           ▼                                            ▼
 ┌──────────┐ ┌─────────────────────────────────────┐   ┌──────────────────────────────┐
 │ app data │ │ AgentRoot                            │   │ kmsg：每个 session 一条输入队列 │
 │ <sid>/   │ │ state/sessions/ state/perception/    │   │ kevent：唤醒通知               │
 │ 产物       │ │ state/artifacts/ memory/ notebook/   │   │ msg-center：inbox(did+sid)     │
 │ .opendan_│ │ sessions/<sid>/ workspace/ .locks/   │   │  （UI session，后移）           │
 │ agent_   │ └─────────────────────────────────────┘   └──────────────────────────────┘
 │ session/ │   （单节点时就是本地文件系统；DFS 上线后对文件锁透明）
 └──────────┘
```

| 层 | 组件 | 回答的问题 | 需求 §2.3 |
|---|---|---|---|
| L1 | AICC / 裸模型 | 一次模型输出 | 裸模型推理 |
| L2 | `llm_context`（CLI / run 目录为 xllm） | 一次上下文推理循环：工具、快照、resume | LM Context |
| L3 | **libOpenDAN** | 以某个 Agent 的身份，把一个 Session 推进到结束条件 | Agent Session |
| L4 | OpenDAN 服务（后续） | 让 Agent 常驻：收消息、定时自省、对外服务 | 长期 Agent 服务 |

依赖方向：`llm_context` ← `agent_tool`（含 xllm）← `libopendan` ← `opendan`（后续）。libopendan 通过 `buckyos-api` 使用 kmsg / kevent / AICC（放在 `buckyos` feature 中），测试时可以换成内存实现。

### 2.2 协议对象之间的关系

- **Session 目录 ↔ Agent State**：创建时登记；驱动者每次提交后回报状态摘要与 activity、追加感知；读取认知线索与活动视图；登记产物。都经 `AgentStateClient`，首版就是对 AgentRoot 文件的直接读写。
- **Session ↔ 输入源**：kmsg 输入队列（所有 session 都有）＋ 可选的 msg-center inbox（UI session 自己的 inbox，`did + session_id`；后移）。只有持有 session 锁的驱动者会消费输入。
- **Session ↔ workspace**：`session_config` 引用一个 workspace（Agent 内部或外部），`binding.json` 记录解析后的 workdir。workspace 自身的管理不属于 Session 协议。
- **Session ↔ 其它活动 session**：经登记表的 activity 互相可见，Agent 据此避让（§6.7）。
- **Session ↔ xllm**：`runs/` 是 xllm 的 run 目录，native runtime 下 xllm 可以接手未结束的 run（§4.4）。
- **Agent → 任意 session**：通过登记表找到位置，直接读取目录。默认允许；session 可以配置为只给状态摘要（`acl.agent_access`）。

### 2.3 访问方式

| 场景 | session 目录 | Agent State | 输入 | 锁 |
|---|---|---|---|---|
| 单节点（当前形态） | 直接读写本地文件 | `AgentStateClient`（文件） | kmsg + kevent | flock |
| OpenDAN 改造后 | 同上 | `AgentStateClient`：部分操作走 kRPC（OpenDAN 提供） | 同上 | 同上 |
| DFS 上线后 | 直接读写 DFS 文件 | 文件实现不变 | 同上 | DFS 提供同一语义的排他文件锁 |
| 应用驱动 work session（本期重点） | app 自己的 data 目录 | 同上（文件版要求能看到 AgentRoot，A8） | 应用自建的 kmsg 队列 | 同上 |
| 应用驱动 UI session（后移） | 同上 | 同上 | 获准的 session inbox（`did + session_id`）+ kmsg 队列 | 同上 |
| runtime 内的子进程 | 只读 | — | 向本 session 的 kmsg 队列投递（perception、control(activity) 等） | 不持有 |

各种环境下锁的语义相同：锁协议（§5）只依赖“可长期持有、持有者退出即释放的排他文件锁”这一个原语。

### 2.4 crate 结构

```text
src/frame/libopendan/
  Cargo.toml            # features: default=["local"]，另有 "buckyos"（kmsg / kevent / AICC）；"krpc"、"host" 随 OpenDAN 改造再加
  src/
    lib.rs
    protocol/           # 所有磁盘 / 消息结构（serde；L6 由此导出 JSON Schema 并反写 Spec）
    fsutil.rs           # atomic_replace / append_batch / truncate_to / reverse_lines / publish_noreplace / publish_dir
    lock.rs             # FileLock：长期持有的排他文件锁（flock；DFS 上线后同语义）；锁文件原地改写（§5）
    session/            # SessionDir：config / state / worklog / summary.json / runs（xllm RunStore）/ static / binding / lease
    channel/            # InputChannel：KmsgInput、内存实现；Waker：KEvent（MsgCenterInput / MsgCenterOutbound 后移）
    state/              # AgentStateClient trait；fs/：registry、activity、perception、cognition（门面）、artifacts
    runtime/            # AgentRuntime trait、native、tmux、bin overlay、paths
    runner/             # drive、assembler、next_llm_context、tools、deps（实现，非协议）
  examples/session.rs   # 开发 CLI：create / run / read / post / decide / active / holder
  tests/
```

依赖都已在 workspace 中，**不新增第三方依赖**：

- 核心：`llm_context`、`agent_tool`（xllm 的 RunStore 与 `.llm_context` 配置解析）、`fs2`、`uuid`、`sha2`、serde 系列。
- `buckyos` feature：`buckyos-api`（`MsgQueueClient`、`KEventClient`、AICC）。

---

## 3. 设计原则（硬约束）

1. **目录结构先定，Spec 反写**：目录布局、提交顺序、锁语义、输入消息格式是契约，在本计划中定稿；字段级 Spec、JSON Schema 与 fixtures 在 Rust 实现之后反写（L6），指导其它语言。Runner 是实现，不是协议。
2. **位置无关**：`.opendan_agent_session/` 里不出现 AgentRoot 路径；与主机相关的信息只出现在 `binding.json`、`.runtime/` 和进行中的 `runs/`（xllm 的 run.json 记录 workdir）。
3. **Agent State 经 `AgentStateClient`，投递靠消息基础设施**：首版对 AgentRoot 文件直接读写；多方投递走 kmsg；唤醒走 kevent。不在文件系统上重新实现队列，也不为 session 开 RPC。
4. **单写者分区**：`.opendan_agent_session/` 中的文件只由 session 锁的持有者写入；执行中的 `runs/<run_id>/` 由 run 锁的持有者写入（§5）。
5. **worklog 严格只追加、只反向读**：已提交部分从不改写，恢复时只截断未提交的尾部。运行时从末尾反向读到需要的位置为止；全量读取只用于审计 / 导出工具。
6. **先提交，后回报与通知**：回报失败的，下次 drive 时补发。
7. **身份不来自提示词**：runner 能做什么，只由驱动者身份、BuckyOS 权限和 session 锁决定（S-08）。
8. **崩溃恢复不另起炉灶**：遵循 llm_context / xllm 的 checkpoint / resume 纪律；`runs/` 保持 xllm 可接手。
9. **Session 协议不承担 workspace 的复杂度**：长期产物的版本、合并与回滚属于 workspace（Q13）。
10. **组合优于发明**：复用 `llm_context`、xllm 的 run 目录与 `.llm_context` 配置、`agent_tool`、kmsg / kevent；session 锁与 xllm 的 run 锁同一模式。
11. **本期不动 opendan**。
12. **避让靠可见，不靠锁**：session 之间对同一对象的并发修改，由活动 Session 视图提示 Agent 自己避让（§6.7）；硬互斥属于 workspace 机制。

---

## 4. Agent Session 目录协议（libOpenDAN::session）

> 本节与 §5 就是本期要先定稿的目录结构协议（V1）。字段级 Spec（`doc/opendan/protocol/Session Directory Protocol.md` 等）在 Rust 实现之后反写（L6，V6）。

### 4.1 目录结构

```text
<any_parent>/<sid>/                  # session 本身的目录：保存 session 里的产物（通常是一次性的）
  readme.md                          # 人读：标题 / 目标 / 起源（进入环境上下文）
  report.md                          # 人读：阶段 / 最终报告
  <产物…>                            # 一次性产物；需要长期保存的放到 workspace（§6.5）

  .opendan_agent_session/            # 保存 session 状态（只由 lease 持有者写）
    session_config.json              # 启动配置：runtime 配置、提示词配置（.llm_context 的超集）、订阅列表、输入通道、app 扩展项；
                                     #   大部分不可在运行中修改；单文件，每次整体原子替换
    state.json                       # 会话状态（提交点）：run_state / acceptance / 当前 run / 输入与订阅游标 / worklog 已提交边界；
                                     #   小文件，整体原子替换
    runs/<run_id>/                   # 就是 xllm 的 run 目录：run.json（RunRecord）+ snapshots/NNNN.json + .lock（run 执行锁）；
                                     #   native runtime 下 xllm 可以接手未结束的 run（§4.4）；
                                     #   只保留 state.json 引用的 run：进行中的 run、最后一次结束的 run、被挂起的 process 所在的 run
    summary.json                     # 重建 llm context 的核心状态 ①：为 history 压缩准备（历史摘要 + 起点 + 机械压缩配置）
    worklog.jsonl                    # 重建 llm context 的核心状态 ②：严格只追加的工作日志（通常是最大的文件）；
                                     #   每次 llm_context run 结束时批量写入该 run 的历史
    static.json                      # 统计信息（token、Round / Turn / run 数、耗时、费用）
    lease.json                       # session 锁文件：驱动者在推进期间一直持有它的排他锁（flock）；
                                     #   内容是持有者信息，只由持有者原地改写，永不替换、永不删除（§5）
    binding.json                     # 首次推进时写入：runtime + workdir（只写一次）

  .runtime/                          # runtime 相关配置（与绑定的 runtime 相关，不要求可迁移）
    bin/                             # session 级工具（进入 PATH 的 Session Bin）
```

**位置无关规则**：

1. session 目录位于任意父目录下（本地文件系统，或上线后的 DFS），目录名等于 `session_id`。
2. `.opendan_agent_session/` 中的路径都相对于 session 目录；只通过 `agent_did` 引用 Agent。
3. 与主机相关的信息只出现在 `binding.json`、`.runtime/` 与进行中的 `runs/`（xllm 的 run.json 记录 workdir），它们随 runtime 绑定（D9）。
4. session 未被持有、且没有未结束的 run 时，可以整体移动；移动后由驱动者更新登记表的 location。
5. `session_id`（sid）**全局唯一**，不同 Agent、App、owner 的不同 session 不得复用同一个 sid。创建者按 §4.2 生成；显式传入 sid 的调用方也承担此约束。登记时的 `publish_noreplace` 是本登记表内的冲突检查，不充当全局 ID 分配服务。
6. **附件引用**：worklog、report 与产物中的附件，可以用 session 目录内的相对路径，也可以用 NamedStore 对象 id（NamedStore 支持 LocalLink，本地文件不必复制）。两种都合法，用哪种由提示词决定；不允许绝对路径（`binding.json` 与 `runs/` 除外）。

旧文档要求“session 目录不放 `bin/`”，`.runtime/bin` 与这条约束不冲突。旧约束的出发点是 session 数据要能跨平台迁移；在新协议中，需要迁移的是 `.opendan_agent_session/` 与产物，`.runtime/` 明确随 runtime 绑定。runtime 自身的临时文件（例如 tmux 的 exec 脚本和输出日志）仍然放在 runtime 的实例卷里。

### 4.2 session_config.json（启动配置）

```jsonc
{
  "schema": "opendan.session_config/2",
  "config_rev": 1,                                     // 运行中允许修改的少数字段（动态订阅等）变更时 +1；整文件原子替换
  "session": {
    "session_id": "work-20260929T101500-550e8400e29b41d4a716446655440000",
    "agent_did": "did:bns:jarvis.alice",
    "kind": "ui | work | self_improve | self_check",   // Q8：沿用 work；本期先实现 work（V1）
    "class": "work",                                   // agent.toml [session.<class>]，决定 loop_mode / driver 配置
    "created_at_ms": 0,
    "created_by": { "principal": "app:app2@alice", "via": "app | ui_session:<sid> | opendan | task_mgr:<task_id>" },   // principal 格式见 A9
    "driver": { "principal": "app:app2@alice" },       // 推进身份 = 驱动进程的 appid，创建后不变（Q9、Q16）；为 OpenDAN 时由 OpenDAN 托管
    "idempotency_key": "…",
    "route_key": "msgtunnel:telegram:acc:chat42",      // 仅 UI
    "origin": { "parent_session": "ui-…", "intent_ref": "…", "reason_messages": ["…"] },
    "objective": "…",
    "end_condition": { "type": "llm_declares_done | output_schema | max_turns", "detail": {} },   // max_turns：完成 detail.n 个 Turn（默认 1）后结束，其间等待输入；内部交接不算 Turn
    "scope": { "objects": ["artifact:snake-game"], "paths": ["ws:snake/src/"] },   // 可选：本次打算修改什么，作为活动视图的初始声明（§6.7）
    "input_policy": "any | supplement_only | none",
    "acl": { "owner": "did:…", "readers": ["did:…"], "agent_access": "full | status_only" },   // Q10：默认 full
    "task_binding": { "task_id": "…" }                // 来自 agent.delegate 时（沿用）
  },
  "prompt": {                                          // .llm_context 的超集：保存合并后的有效配置
    "llm_context": {                                   // 与 xllm `.llm_context` 同一 schema 的 JSON 形式（原文件是 YAML、严格键）
      "provider": { … }, "model": "…", "loop_model": "behavior | function_call",
      "tools": { … }, "prompt": { "groups": { … }, "sections": { … } }   // 凭据只存 SecretRef（xllm 支持 ConfigFile / Env）
    },                                                 // xllm 接手时，run.json 的 EffectiveConfig 由它生成（§8.7 X2）
    "behavior": "plan",                                // 以下是 OpenDAN 扩展：behaviors/<name> 引用（BehaviorAssembler）
    "system_prompt": "…",                              // 应用自带的 system prompt（S-05；按 §8.1 的固定顺序合成）
    "context": ["…"],                                  // 应用提供的初始上下文材料
    "mechanical_compress": { "recent_full_responses": 2, "summary_chars": 280, "max_result_chars": 4096 }   // summary.json 的初始机械压缩配置
  },
  "runtime": { "requirement": { "runtime_id": null, "tools": ["node"], "app_tools": ["erp_query"] },   // app_tools：只存在于 runner 进程内的工具，恢复时校验（§7.1）
               "tool_plan": "minimal_safe", "env": { … } },
  "workspace": null,                                   // 可选：长期产物所在的 workspace（§6.5）
                                                       //   { "kind": "agent", "id": "ws-…" } | { "kind": "external", "path": "/…/app2/snake" }
  "artifact_id": null,                                 // 可选：在产物列表中登记为哪个长期产物（§6.5）
  "subscriptions": [                                   // 订阅声明（已感知游标在 state.json）
    { "id": "s1", "mode": "semi", "source": { "type": "session", "ref": "work-…" }, "watch": ["run_state", "outcome", "acceptance", "one_line_status"] },
    { "id": "s2", "mode": "active", "source": { "type": "object_event", "object": "https://…/cam01", "event": "motion" } }
  ],
  "channels": {
    "inputs": [
      { "id": "q", "kind": "kmsg", "queue": "app2::alice::opendan.session.work-…", "subscriber": "opendan.<agent_id>.work-…" }   // sub_id 全局共用命名空间，带 agent_id 前缀
      // UI session（后移）：{ "id": "inbox", "kind": "msg_center", "did": "did:bns:jarvis", "session_id": "tg:acc:chat42" }
    ],
    "outbound": null,                                                            // UI session（后移）：{ "kind": "msg_center" }
    "wake_event": "/opendan/<agent_id>/session/<sid>/input"                      // kevent：投递后发布；每段只能含字母、数字和 _ - .
  },
  "extensions": { "<app_id>": { } }                    // app 扩展配置项，libopendan 原样保留
}
```

**session_id 规则**（全局唯一；复用已有 `uuid`、`sha2`，不新增第三方依赖）：

| kind | id |
|---|---|
| ui（后移） | `ui-<H("ui", agent_did, route_key)>`；同一 Agent 的同一路由重用同一 session |
| work | `work-<yyyymmddThhmmss>-<uuid_v4_32hex>`；带 idempotency_key 时为 `work-<H("work", agent_did, creator_principal, key)>` |
| self_improve | `si-<yyyymmddThhmmss>-<uuid_v4_32hex>`；带 idempotency_key 时为 `si-<H("self_improve", agent_did, creator_principal, key)>` |
| self_check | `sc-<H("self_check", agent_did)>`；每个 Agent 的 singleton 有自己的全局 sid |

`H` 是参数数组按固定 JSON 编码（UTF-8、无额外空白）得到的 SHA-256 完整小写 hex；字段顺序固定，不拼接裸字符串、不截断 hash 或 UUID。相同幂等命名空间返回同一 sid，不同 Agent 下相同 key 返回不同 sid。外部显式传入的 sid 必须符合相同的全局唯一与字符约束。

session_id 只使用字母、数字和 `_ - .`，这样才能直接作为 kevent 事件名的一段和 kmsg 队列名的一部分。队列继续使用 `opendan.session.<sid>`，不需要再增加 Agent 维度；文中的 `work-…` 等短写只是展示缩写，不是生成规则。

**`prompt.llm_context` 与 xllm**：`llm_context` 子对象就是 xllm 能读的有效配置，其余字段是 OpenDAN 的扩展。libopendan 建 run 时用它生成 run.json 的 `EffectiveConfig`，所以 xllm 接手时不需要理解 OpenDAN 扩展。

### 4.3 state.json：会话状态（提交点）

```jsonc
{
  "schema": "opendan.session_state/2",
  "rev": 17,                                         // 每次提交 +1；回报与版本比对都用它
  "writer": { "runner_id": "rn-…", "principal": "app:app2@alice", "host": "did:dev:…", "pid": 1234, "lock_epoch": 42 },
  "run_state": "created | ready | running | waiting | finished",
  "waiting_for": null,                               // { kind: input|tool|event|children, refs: [], deadline_ms }
  "outcome": null,                                   // succeeded | failed | stopped
  "acceptance": "n/a",                               // n/a | pending | accepted | discarded（仅 work）
  "result": null,                                    // finished 时：{ answer_ref, artifact_ref, discard_report }
  "turn_seq": 12, "turns_completed": 11,             // 最近开启的 Turn 编号；以 completed 关闭的 Turn 数（§8.3）
  "open_turn": { "index": 12, "run_id": "20260929-101500-3f9c2a", "input_seq": 1,   // 进行中的 Turn；null = 没有。(run_id, input_seq) 是开启它的输入批次
                 "hook": "on_wakeup", "inputs": ["q#121"], "at_ms": 0 },              //   inputs：开启批次及之后并入的 msg / event
  "current_behavior": "do", "process_entry": "plan",
    "process_stack": [{ "entry": "plan", "mode": "fork | independent", "run_id": "20260929-100200-a1b2c3",
                        "turns": [ … ], "flushed_step_index": 6, "flushed_input_seq": 2, "applied_input_seq": 2 }],   // 挂起时保存该 process 的 run 与提交位置（§4.4）
  "pending_task_calls": [], "bootstrap_done": true,
  "topic": { "title": "…", "tags": ["snake", "ui"] },
  "live_run": {                                      // 未结束的 run（xllm run 目录）；null = 没有进行中的 llm_context
    "run_id": "20260929-101500-3f9c2a",
    "turns": [{ "turn": 12, "inputs": ["q#121"], "changes": ["s1@16"], "hook": "on_wakeup", "input_seq": 1, "at_ms": 0 }],
                                                      // 本 run 各 Turn 消费的输入：每个 Turn 一项，同一 Turn 的后续批次并入该项；由快照 receipt 幂等补齐
    "applied_input_seq": 3,                           // 本 run 已并入 state 的连续 receipt 批次；与快照的 input_receipts 对齐（§8.3）
    "flushed_message_count": 0,                       // function call run：history 前缀之后已写入 worklog 的消息数（在 flushed_epoch 内计）
    "flushed_step_index": 0,                          // behavior run：step_index 小于它的 Step 已写入 worklog（身份水位，不是计数）
    "flushed_input_seq": 0, "flushed_epoch": 0        // 已写入的输入批次；flushed_message_count 所在的 history epoch（挂起时会先写一部分，§4.4）
  },
  "last_run": "20260929-095501-77e0d4",              // 最后一次结束的 run：保留它的 llm context 状态（§4.4）
  "worklog": { "committed_seq": 340, "committed_bytes": 1048576 },   // 已提交边界；其后的尾部视为未提交，恢复时截断
  "inputs": {                                        // 按输入源分别记录消费进度（§4.5）
    "q":     { "acked_index": 118, "consumed_above": [121] },
    "inbox": { "reading": ["rec_…"] },                                // UI session（后移）：msg-center 已消费、尚未标记 Read 的记录
    "recent_keys": ["msg:…", "…"]
  },
  "subscription_cursors": { "s1": { "rev": 15, "run_state": "running" }, "s2": { "key": "cam01#motion", "version": "evt_…" } },
  "perception_seq": 88, "reported_rev": 17,          // 已追加的感知序号、已回报登记表的 rev（用于补发）
  "activity": {                                      // 活动声明（§6.7）：在做什么、大概在改什么；run 进行中节流更新
    "summary": "给贪吃蛇加穿墙模式：正在改碰撞检测",
    "touching": [{ "kind": "path", "ref": "ws:snake/src/collision.js", "mode": "write", "since_ms": 0 }],
    "heartbeat_ms": 0
  },
  "one_line_status": "…", "last_error": null, "updated_at_ms": 0
}
```

- **提交顺序**：同一次提交中的其它内容（worklog 追加、report.md、runs checkpoint）都先于 `state.json` 写入。`commit_state` 总是把 worklog 当前末尾记为 `committed_*`。读者看到新的 rev 时，它引用的内容一定已经存在（S-23）。
- **其它参与方**获取状态时，默认读登记表的 `status`（§6.2）。
- **Turn**：`turn_seq` / `open_turn` / `turns_completed` 记录 Session 的逻辑 Turn（一次 Input → result）。输入批次在没有进行中的 Turn 时开启新 Turn，否则并入当前 Turn；只有 session 在 run 结束提交时关闭它（规则见 §8.3）。它们不是推理 Round 数，也不是 run 数。

完成、停止、放弃用三个正交维度表达：

```text
run_state:  created ─► ready ◄──► running ◄──► waiting
                          │          │             │
                          └──────────┴──── stop ───┴──► finished { outcome: succeeded | failed | stopped }

acceptance (work):  n/a ─(finished)─► pending ─► accepted
                                         └─────► discarded ◄── accepted（事后放弃）
```

| 操作 | 合法前置 | 效果 |
|---|---|---|
| `stop` | run_state ∈ {ready, running, waiting} | 在下一个安全点结束：finished + outcome=stopped（沿用 Graceful 中断） |
| `accept` | work ∧ finished ∧ acceptance=pending | acceptance=accepted；产物列表把本 session 的版本置为有效（§6.5） |
| `discard` | work ∧ acceptance ∈ {pending, accepted} | acceptance=discarded；产物版本失效；对 workspace 的变更交给 workspace 处理，结果写入 `result.discard_report`；追加 `task_discarded` 感知 |

- 进入 `finished` 后不能再回到 running；追问或继续改进一律新建 work session（S-12）。
- stop / accept / discard **都由驱动者执行**（Q9）。其它参与方向该 session 的 kmsg 队列投递 `control(decide|stop)` 即可（§4.5）。
- 与现有状态的映射：Idle → ready，Running → running，WaitingInput / WaitingTool → waiting，Ended → finished，Error → ready + `last_error`。

### 4.4 worklog.jsonl、summary.json、runs/ 与下一次 llm_context

**worklog.jsonl** 是 session 的**严格只追加历史**，由现有的 round_history、topic_log 和 worklog 事件合并而来。每行一条，`seq` 单调。它不写状态，也从不改写；唯一允许的修改，是恢复时截掉 `state.worklog.committed_bytes` 之后的**未提交尾部**。

写入时机：

- **每次 llm_context run 结束时，批量追加该 run 的全部历史**：turn_started / input_batch（输入批次）、user_message、assistant_message（function call 的一次 response）或 step（behavior 的一个 `StepRecord`）、action_result、outcome，以及该提交关闭 Turn 时的 turn_ended。这通常是 worklog 最大的一次写入，只做一次 fsync。
- run 之外的少量条目（decide、input_rejected、compaction）在发生时追加。
- run 进行中的历史只在 `runs/`。
- 每次追加之后都要提交一次 state.json 作为确认。

```jsonc
{"seq":312,"t":"turn_started","run_id":"20260929-101500-3f9c2a","turn":12,"input_seq":1,"inputs":[{"src":"q","index":121,"key":"msg:…","kind":"msg"}],"changes":["s1@16"],"hook":"on_wakeup","at_ms":0}
{"seq":313,"t":"user_message","run_id":"…","turn":12,"content":"…"}
{"seq":314,"t":"step","run_id":"…","turn":12,"step_index":0,"behavior":"do","assistant":"…","actions":[{"call_id":"c-12-1","tool":"exec_bash","args":{…},"effect":"unknown"}]}
                                                                       // function call run 写 {"t":"assistant_message","run_id":"…","turn":12,"assistant":"…","tool_calls":[…]}
{"seq":315,"t":"action_result","run_id":"…","turn":12,"call_id":"c-12-1","status":"ok","result":"…"}
{"seq":316,"t":"outcome","run_id":"…","turn":12,"kind":"done","next_behavior":null}
{"seq":317,"t":"turn_ended","run_id":"…","turn":12,"status":"completed","at_ms":0}   // completed | failed | budget_exhausted | stopped
{"seq":318,"t":"compaction","summary_start_seq":301}                  // 生成了新的 summary.json（审计用）
{"seq":319,"t":"decide","decision":"accept","by":"did:user:alice","report":{…}}
```

**summary.json** 专门为 session history 压缩准备：用“历史摘要 + 起点 + 机械压缩配置”描述下一次 llm_context 如何由 worklog 构成。

```jsonc
{
  "schema": "opendan.session_summary/2",
  "history_summary": "…",               // 起点之前全部历史的摘要（LLM 生成）
  "start_seq": 301,                     // 起点：从这条 worklog 开始使用原始记录
  "start_offset": 912384,               // 起点在 worklog 中的字节偏移：反向读取读到这里即停止
  "mechanical": {                       // 机械压缩配置：起点之后的原始记录如何渲染
    "recent_full_responses": 2,         //   最近 N 个模型 response（behavior 的 step 或 function call 的 assistant_message）全量
    "summary_chars": 280,               //   更早 response 的 assistant 文本截断长度
    "max_result_chars": 4096,           //   单个动作结果上限
    "drop_kinds": ["created", "decide", "compaction", "input_rejected", "change_dropped", "turn_ended"]   //   不进入上下文的条目类型
  },
  "renderer": "libopendan.mechanical/2",   // 渲染器标识与版本：确定性只在同一渲染器版本内承诺
  "made_at_seq": 318, "made_by": "context_limit | ratio | manual", "updated_at_ms": 0
}
```

首次压缩之前，这个文件可以不存在，等价于 `start_seq = start_offset = 0`、没有摘要、机械压缩配置取 `session_config.prompt.mechanical_compress`。

**runs/** 就是 xllm 的 run 目录：`runs/<run_id>/run.json`（RunRecord）+ `snapshots/NNNN.json`（LLMContextSnapshot）+ `.lock`（run 执行锁）。一个 run 就是一个 LLMContext 从 new / resume 到终态 outcome 的生命周期。run 结束后不立即删除：**最后一次 run 的 llm context 状态总是保留**（`state.last_run`），直到下一个 run 结束取代它。

```text
开始：   经 xllm RunStore 建立 runs/<run_id>/ 并持有 run 锁 → 写入带 input_receipts 的 s0 快照（host_commit_pending）
         → 提交 state.json（live_run、本批次输入及其 Turn 归属、已消费输入）→ 清除 host_commit_pending → 确认输入源；之后才允许推理
进行中： checkpoint 写入 runs/<run_id>/snapshots/（最细到一次 do-action 之后）；worklog 不写；
         state.json 的 one_line_status / activity 按节流提交并回报登记表，其它参与方由此看到进度（§6.7）
挂起：   PendingTool 等：保留 run，下一次输入直接 resume
结束：   ① 从最终快照生成该 run 尚未写入的历史（flush 游标之后），一次性追加到 worklog（fsync）
         ② 提交 state.json（live_run = null，last_run = 本 run，worklog.committed_* 前移）   ← 提交点
         ③ 删除 state.json 不再引用的 run（原来的 last_run 等）
```

**下一次 llm_context 的构成与读取策略**：worklog 将来一定会很大，所以读取顺序固定为：先读 summary.json，再从 worklog 已提交的末尾**反向**读。

```python
def next_llm_context(s, lease, deps, env) -> LLMContext:
    live = resume_live_run(s, lease, deps, env)                  # ⓪ 有未结束的 run（state.live_run）：直接恢复（§8.6）
    if live: return live
    sm = s.summary_json() or Summary.initial(s.config)           # ① 先读 summary.json（小文件）：摘要 + 起点 + 机械压缩配置
    req = deps.assembler.build_request(s.config, env)            #    system：session_config.prompt（.llm_context 语义）+ 身份 / 约束段
    budget = history_budget(req, sm.history_summary)             #    上下文窗口减去 system 与摘要后的预算
    tail, reached_start = [], True
    for e in s.worklog.reverse_lines(end=s.state.worklog.committed_bytes,   # ② 从已提交末尾反向读（按块读取，由新到旧）
                                     stop=sm.start_offset):                 #    读到起点就停，从不扫描起点之前的内容
        if e.t in sm.mechanical.drop_kinds: continue
        r = render_mechanical(e, age=len(tail), cfg=sm.mechanical)          #    越新越完整，越旧截断越多
        if not budget.take(r): reached_start = False; break      #    预算先于起点耗尽：说明摘要已经过旧
        tail.append(r)
    if not reached_start:                                        # ③ 摘要与原始记录之间不能留空洞：先压缩，再构建
        sm = compact(s, lease, deps, cut_offset=offset_of(tail[-1]))        #   新起点 = 已保留条目中最旧的一条
    tail.reverse()                                               # ④ 恢复时间顺序
    return LLMContext.new(req.with_history(render(sm.history_summary, tail)), deps.llm_deps(s))
```

- **只反向读、按需停止**：正常情况下，读取量约等于起点之后的记录量，受上下文预算约束，与 worklog 总大小无关。`reverse_lines` 是各语言共用的原语：从指定字节位置向前按块读取，并按 `\n` 切行（§4.7）。
- **其它读取也用同一策略**：`read_session` 取最近历史、统计本 session 的副作用动作，都从末尾反向读（补发 run_digest 只读 state.json）。唯一的正向读取是压缩时读取 `[start_offset, cut_offset)` 这个有界区间；全量读取只留给审计 / 导出工具。
- **确定性**：同一渲染器版本（`summary.json.renderer`）下，相同的 summary.json + worklog 渲染出相同的历史段。时间、天气等需要新鲜的量放在每次输入批次消息（`<session_input>`）的变量段，不进入历史段（S-20）。system + 摘要在下一次压缩之前保持不变，构成稳定前缀，有利于 KV cache。跨语言只要求语义一致，不要求字节一致。
- **压缩**只产出新的 summary.json（摘要前移、`start_seq` / `start_offset` 后移），不改写 worklog。触发时机沿用现有的两种：
  - run 结束后，按上下文占用比例触发（`maybe_compact`，§8.3）；
  - run 进行中遇到 `ContextLimitReached` 时（X7，已实现）：先把该 run 已产生的历史写入 worklog 并以 `outcome(context_rewritten)` 收尾，再压缩（只写 summary.json），然后以 system + 重建的历史消息恢复同一 run（function call：`RewrittenHistory`；behavior：`RewrittenSteps`），一次推进内最多连续重写 3 次；仍装不下则 run 暂停在上下文上限，下次推进先重写。重写只改变历史的承载方式，当前 Turn 继续。run 的历史只经 worklog 重建，不在快照里另行摘要。

```python
def compact(s, lease, deps, cut_offset) -> Summary:          # 只写 summary.json（外加一条审计条目）
    sm = s.summary_json() or Summary.initial(s.config)
    segment = s.worklog.read_range(sm.start_offset, cut_offset) # 唯一的正向读取：有界区间
    new = Summary(history_summary=summarize(sm.history_summary, segment),
                  start_seq=seq_at(cut_offset), start_offset=cut_offset,
                  mechanical=sm.mechanical, made_at_seq=s.state.worklog.committed_seq, made_by="ratio|context_limit")
    fenced(lease, lambda: atomic_replace(s.state_dir / "summary.json", new))
    s.worklog.append(lease, compaction_entry(new)); s.commit_state(lease)   # 审计条目 + 确认
    return new

def reconcile_runs(lease, s, deps):                           # drive 开头、读取新输入之前调用
    s.worklog.truncate_to(lease, s.state.worklog.committed_bytes)   # 截掉未提交尾部
    live = s.state.live_run.run_id if s.state.live_run else None
    keep = {live, s.state.last_run} | {p.run_id for p in s.state.process_stack}   # state.json 引用的 run 都保留
    for run in s.runs.list():
        if run not in keep:
            s.runs.remove_if_safe(lease, run)                     # 持 run 锁，确认格式可读、没有仍存活或无法核对的旧工具；否则保留并报错
        elif run != live: continue                               # last_run 与挂起的 process 保留，实际恢复时执行同样的检查
        else:
            s.runs.ensure_locked(run)                            # 真正持有 run 锁；不以 is_live 探测代替加锁；持有到本次推进结束
            cp, record = s.runs.load_checked(run)                 # 不支持 / 损坏 / 缺失 → RecoveryBlocked，保留现场（§8.6）
            ensure_previous_execution_stopped(record, deps.runtime)  # kill runner 不代表旧工具退出（§5.2）
            reconcile_input_receipts(lease, s, cp, record)         # 先补齐 state 消费状态，再允许 fetch（§8.3）
            if record.status.is_terminal():                      # 结束流程未完成，或 xllm 已跑完
                finish_run(lease, s, run, cp, …)                   # §8.3；工具结果已持久化且执行已核对
```

**process 与 run 的对应**（§15.1 A3 = a）：每个拥有独立 llm context 的 behavior process 对应一个 run。

- **normal 切换**：同一个 context，同一个 run。
- **fork**：子 process 开一个新 run（继承父 process 的 steps）；父 process 的 run 挂起，记入 `state.process_stack`。
- **independent**：每个 process 各有自己的 run；切走时挂起，重新进入时恢复同一个 run。
- **Turn 不因切换结束**：三种切换（以及 fork 子 process 返回）都以 `on_behavior_switch` 输入批次（worklog `input_batch`）并入当前 Turn。Step 身份是 `(run_id, step_index)`：fork 子 run 接续父 run 的编号，independent 的各 run 各自编号，step_index 在 session 内不保证全局唯一。
- **挂起**：run 被挂起时，先把它到目前为止尚未写入的历史追加到 worklog 并提交（推进该 run 的 flush 游标：`flushed_message_count` / `flushed_step_index` / `flushed_input_seq`），worklog 因此保持时间顺序；fork 子 run 只写自己新产生的部分，不重复写继承来的 steps。
- **保留**：被挂起的 run 由 `state.process_stack` 引用，不会被删除；process 结束出栈后，它按普通结束处理（成为 last_run 或被取代）。
- **last_run**：保留用于审计、诊断，以及查看最后一次执行现场。下一次 llm_context 的构成规则（D8）不变。
- **恢复元数据**：挂起与恢复 process 时，连同 `turns`、flush 游标（含 `flushed_epoch`）、`applied_input_seq` 保存和恢复该 run 的提交位置；fork 子 run 不继承父 run 的输入 receipt 命名空间。快照中的 receipt 不随历史压缩丢弃（§8.3）。

**xllm 接手**（V2）：native runtime 下，未结束的 run 原则上可以交给 xllm 继续跑完：

```bash
xllm --resume --run <run_id> --runs-dir <sid>/.opendan_agent_session/runs --dir <binding.workdir>
```

- **条件**：binding 的 runtime 是 native；run 的工具能由 `prompt.llm_context.tools` 重建（session 专用的操作以 CLI 形式经 exec_bash 提供，而不是只存在于 runner 进程内的工具）；run.json 由 libopendan 按 xllm 的 RunRecord 写出（§8.7 X2）。格式校验、工具环境完整性校验与旧工具停止检查均通过，且 `host_commit_pending` 为空（§8.3）。不满足时明确拒绝；有待补交的宿主输入时先由 libopendan 恢复，xllm 不代写 state.json。
- **互斥**：run 的执行互斥由 `runs/<run_id>/.lock` 保证。libopendan runner 恢复 live_run 之前先拿 run 锁；拿不到就返回 Busy，不另起新 run。
- **写入范围**：接手的 xllm 只写 run 目录，不碰 state.json、worklog 和 Agent State。run 到达终态后，下一次 drive 的 `reconcile_runs` 照常把它的历史写入 worklog 并提交；xllm 执行期间的输入留在队列里。
- **workdir 锁**：xllm 自己的 workdir 互斥锁（`<lock_dir>/<hash(workdir)>.lock`）只在 xllm 接手时生效。libopendan runner 不使用它：多个 session 可以共享一个 workspace，由活动视图避让（§6.7）。
- **在途动作**：记录在 RunRecord 中（§8.7 X6）。先确认旧执行停止，再根据已持久化的结果区分“已有结果”和“结果未知”；xllm 使用同一恢复与 checkpoint 提交规则，不能仅因拿到 run 锁就继续执行。

### 4.5 输入通道：kmsg + kevent（不用文件系统）

session 的输入源在 `session_config.channels.inputs` 中声明。本期只有 kmsg；msg-center inbox 随 UI session 后移（V1）。

| 输入源 | 适用 | 承载 | 确认（commit pop） |
|---|---|---|---|
| **kmsg 队列**（`KmsgInput`） | 所有 session；创建时建立 | msg / event / change / control / perception | `commit_ack(index)`（累积） |
| **msg-center inbox**（`MsgCenterInput`，后移） | UI session：自己的 inbox，以 `did + session_id` 标识 | 用户 / peer 消息 | `update_record_state(Read)` |

kmsg 队列中的五类输入：

| type | 生产者 | 触发推理 | 说明 |
|---|---|---|---|
| `msg` | UI session 的 forward、应用、其它 session | 按 driver 的 pull 策略 | 消息 |
| `event` | 事件桥（active 订阅）、timer、task_mgr | 是（active） | 主动订阅的事件 |
| `change` | 事件桥（semi 订阅的外部对象事件） | **否**，只在观察边界注入 | 半订阅；同 key 在读取时合并，终态用独立 key |
| `control` | UI session、应用、OpenDAN、runtime 子进程 | 否；drive 开头与每个观察边界直接应用 | subscribe / unsubscribe / stop / decide / activity |
| `perception` | runtime 子进程（如 `perceive` CLI） | 否 | 持有者在 drive 开头与每个观察边界并入感知（同 control） |

**消息格式**：payload 是 JSON；headers 固定包含 `type`、`key`（去重键）、`from`（投递方 principal）、`at_ms`，可选 `intent`、`reply_to`。字段级定义在 L6 反写。kRPC 请求体上限 1 MB，payload 又以 JSON 数字数组编码，所以单条 payload 不超过约 250 KB；更大的内容放进 session 目录或 NamedStore，只投递引用。

```python
def post_input(cfg, inp: Input, who):                       # 投递到 kmsg 队列；任意有写权限的一方都可以调用
    if registry_status(cfg.sid).run_state == "finished" and not inp.allowed_after_finish():   # 尽力预检；以消费端为准
        raise SessionFinished
    q = cfg.channels.kmsg()
    kmsg.post_message(q.queue, Message(payload=to_json(inp.payload),
        headers={"type": inp.type, "key": inp.dedup_key, "from": who, "at_ms": now(), "intent": inp.intent}))
    kevent.publish(cfg.channels.wake_event, {"sid": cfg.sid})   # 只通知“有变化”；丢了靠轮询兜底

def fetch_inputs(s) -> Inputs:                               # 只由持有者调用
    out = Inputs()
    for src in s.config.channels.inputs:
        for m in src.fetch(s.state.inputs[src.id]):          # kmsg：从服务端游标开始，auto_commit=false
            if not s.state.inputs.is_consumed(src.id, m) and m.key not in s.state.inputs.recent_keys:
                out.add(src.id, m)                           # 过滤“已消费但未确认”的消息与重复投递
    return out

def confirm_committed_inputs(s):                            # 只确认 state.json 已提交的位置；重启时也调用
    for src in s.config.channels.inputs:
        src.confirm(s.state.inputs[src.id])                 # kmsg：连续已消费最大 index；不依赖是否取到了新消息

def commit_pop(lease, s, consumed, prepared=None):           # 上下文输入须先有带 receipt 的 prepared checkpoint（§8.3）
    st = s.state.copy(); st.inputs.mark_consumed(consumed)   # 各输入源分别记录；recent_keys 有上限
    s.commit_state(lease, st)                                # ① 先提交 state.json（本地持久）
    if prepared: s.runs.complete_host_commit(prepared)       # ② state 已持久化后清除 run.json.host_commit_pending
    confirm_committed_inputs(s)                             # ③ 再确认输入源
```

- **为什么先提交 state.json 再确认**：两步之间崩溃时，恢复先重试确认已提交位置；重投的消息由消费记录过滤。快照已写、state 尚未提交的窗口则按 receipt 补交（§8.3），不能把队列重投再次追加到已有快照。
- **输入标识**：队列输入以 `(source_id, index/record_id)` 标识投递实例，并保留生产者的 `key` 作为语义去重键；拉取的变化以 `(subscription_id, object_ref, version)` 标识。`recent_keys` 只是有界缓存，不能替代快照中持久化的 receipt 与 state 的消费位置。
- **选择性消费**：kmsg 的 ack 是累积的，只能 ack 到“连续已消费”的位置，跳过的消息会留在游标之后。永远不会处理的输入（例如 finished 之后的普通输入、不认识的类型），持有者要显式标记为已消费，并写一条带原因的 `input_rejected` worklog，避免游标被卡死。
- **kmsg 队列的所有权**：由创建者（通常是驱动者所属 App）创建，并开放 `other_app_can_write`，让 UI、OpenDAN 等其它参与方可以投递 control / msg。

**kmsg 使用规则**（按 §1.5 核对的现状）：

| 现状 | 规则 |
|---|---|
| `create_queue` / `subscribe` 不幂等，已存在时返回字符串错误 | 按“已存在即成功”处理，并用 `get_queue_stats` 确认队列存在 |
| sub_id 在所有队列、所有 App 之间共用一个命名空间 | sub_id 使用 `opendan.<agent_id>.<sid>` |
| 游标不落盘（D-09），服务重启可能丢订阅 | fetch 遇到 “Subscription not found” 时，以 `At(state.inputs.q.acked_index + 1)` 重新订阅 |
| `commit_ack` 直接设 `cursor = index + 1`，可回退，没有 fencing | 只 ack 到 state.json 已提交的连续消费位置；永远不 ack 小于已记录的值 |
| `delete_message_before` 与 post 有竞态，会复用 index 覆盖消息；retention 未实现（D-06） | 本期不删除消息；session 结束后整队列删除（GC 另议） |
| 没有权限校验（D-07），`headers.from` 是自报的 | `from` 只用于审计；control 输入的认证等 D-07 补齐（§15.1） |

**UI session 与 msg-center（后移，V1）**。以下是 UI session 恢复实施时要处理的现状（§1.5）：

- msg-center 已按 `did + session_id` 划分 inbox，UI session 可以直接用 `MsgCenterInput` 消费自己的 inbox；记录状态是 `Read`。
- take（`lock_on_take`）需要 inbox 的**写**权限；回复需要另一条 `sent/<did>/<session>` 的写权限；消息的 `from` 仍是 agent DID，App 只是授权主体。
- `Reading` 没有租约：take 之后崩溃的记录会一直卡住，“崩溃后重投”在 msg-center 上不成立。需要 msg-center 增加 `Reading` 回收，或 `MsgCenterInput` 启动时以 `[Unread, Reading]` 取回并依赖 state.json 去重。
- inbox 的 session_id 取自发送方可控的 `thread.topic`，需要规定谁能往哪个 inbox 投递。
- INBOX / GROUP_INBOX / REQUEST_BOX 是三个独立队列，一个 UI session 可能要消费多个。
- 谁能驱动哪类 session（Q14、Q16）：OpenDAN 主进程可以读 Agent 的各类 inbox，负责发现并托管 UI session（附录 A.3）；获准读写某个 session inbox 的 app 可以创建并驱动该 UI session（§10.2）；其它 runner 通常创建 work session。

### 4.6 订阅与变化（半订阅的落地）

- **本地状态对象**（其它 session、产物）：拉模式。在观察边界比较 Agent State 登记表中对方 `status` 的 rev 与 `state.subscription_cursors`，不依赖事件是否送达（S-16、A-09）。
- **外部对象事件**：以 `change` 类型进入 kmsg 队列，读取时按 key 合并。终态和用户放弃使用独立 key `…#terminal`，不会被进度覆盖（A-10）。消费进度落在 `state.inputs` 与 `state.subscription_cursors`（S-18）。因合并、过期或不相关而丢弃的变化，也要写入 worklog 并注明原因。
- 游标只存在于接收方 session 内：一个 UI session 已感知的变化，不会导致另一个 UI session 漏收。
- **active 与 semi 的区别**：active 源以 `event` 类型投递，会唤醒 session 并触发推理；semi 源以 `change` 类型投递，只在观察边界注入（S-15、A-07）。
- **其它活动 session**：每个 session 默认对“活动 session 集合”半订阅（§6.7），集合或交集变化时在观察边界注入。
- **run 中注入的消费标记**：观察钩子的 change 与普通 msg / event 使用同一 input receipt 协议（§8.3），内容与结构化 receipt 一起写入快照；恢复时先补交消费状态，再 fetch / render，避免重复注入。只收到、尚未注入或按规则丢弃的输入不得标成“已感知”。

### 4.7 文件原语（所有语言实现同一组语义）

```python
def atomic_replace(path, data: bytes):             # state.json / session_config / summary.json / static / 登记条目：整文件替换
    tmp = f"{path}.tmp-{uuid()}"
    write_all(tmp, data); fsync(tmp)
    rename(tmp, path); fsync(parent_dir(path))

def append_batch(path, objs) -> (seq, bytes):       # worklog / 感知：一次写入多行完整 JSON（单写者），一次 fsync
    fd = open(path, O_APPEND | O_CREAT)
    write(fd, "".join(to_json(o) + "\n" for o in objs)); fsync(fd)
    return end_position(fd)

def truncate_to(path, committed_bytes):             # 恢复时截掉未提交尾部（worklog 唯一允许的修改）
    if size(path) > committed_bytes: truncate(path, committed_bytes); fsync(path)

def reverse_lines(path, end, stop, block=64 * 1024):   # 从 end 向前逐块读，由新到旧产出完整行，到 stop 为止
    pos, carry = end, b""
    while pos > stop:
        n = min(block, pos - stop); pos -= n
        chunk = read_at(path, pos, n) + carry
        lines = chunk.split(b"\n")
        carry = lines[0] if pos > stop else b""     # 块首的半行留给下一块拼接
        for line in reversed(lines[1:] if pos > stop else lines):
            if line: yield parse(line)

def publish_noreplace(path, data: bytes) -> bool:   # 只写一次的文件：binding.json、登记条目
    tmp = f"{parent_dir(path)}/.tmp-{uuid()}"
    write_all(tmp, data); fsync(tmp)
    ok = link_noreplace(tmp, path)                  # POSIX link()/renameat2(NOREPLACE)；Windows MoveFileEx 不带 REPLACE_EXISTING
    unlink(tmp); return ok

def publish_dir(tmp_dir, final_dir) -> bool:        # 目录原子发布（session 创建）
    return rename_noreplace(tmp_dir, final_dir)

def lock_try(path) -> FileLock | None:               # 锁文件（§5）：O_CREAT 打开，带 CLOEXEC；非阻塞排他锁；锁文件永不替换、永不删除
    fd = open(path, O_RDWR | O_CREAT | O_CLOEXEC)
    if not try_flock(fd, LOCK_EX | LOCK_NB): close(fd); return None
    return FileLock(fd)                             # 持有到 release 或进程退出

def lock_rewrite(lock: FileLock, data: bytes):      # 持锁者经同一 fd 原地改写锁文件内容（持有者信息），不 rename
    pwrite(lock.fd, data, 0); ftruncate(lock.fd, len(data)); fsync(lock.fd)
```

- 其它参与方读锁文件内容只用于显示和诊断；读到写了一半的内容（解析失败）就重读。
- `atomic_replace` 不能用于锁文件：flock 锁在 inode 上，rename 之后新打开者拿到的是新 inode 的锁。

### 4.8 创建、打开与读取

```python
def create_session(parent_dir, spec, agent: AgentStateClient, who) -> SessionDir:
    sid = spec.session_id or derive_id(spec, creator=who)         # 按 §4.2 的全局规则生成，显式 sid 也须符合约束
    spec.driver = spec.driver or who                             # 缺省：创建者就是驱动者
    if spec.kind == "ui":                                        # 后移（V1）
        authorize_inbox(who, (spec.agent_did, spec.route_key))   # Q14：驱动进程的 appid 须获准读写该 session inbox（did + session_id）
    queue = kmsg_create_queue_idempotent(f"opendan.session.{sid}", appid=who.app, owner=who.owner,
                              config=QueueConfig(sync_write=True, other_app_can_write=True))   # “已存在”按成功处理（§4.5）
    kmsg_subscribe_idempotent(queue, sub_id=f"opendan.{agent.agent_id}.{sid}", position=Earliest)
    tmp = parent_dir / f".tmp-{uuid()}"
    write(tmp / ".opendan_agent_session/session_config.json", config_from(spec, who, queue))
    end = write(tmp / ".opendan_agent_session/worklog.jsonl", [created_entry(spec, who)])
    write(tmp / ".opendan_agent_session/state.json", initial_state(committed=end))   # rev=1，run_state=created
    write(tmp / "readme.md", render_readme(spec))
    apply_acl(tmp, spec.acl)                                     # 按 agent_access 设置访问控制（Q10）；当前没有文件级 ACL，本期只记录（§1.5）
    if not publish_dir(tmp, parent_dir / sid):
        rm_rf(tmp)
        c = read_config(parent_dir / sid)
        if not (spec.idempotency_key and same_session_identity(c, spec, who)):
            raise SessionIdConflict(sid)                        # 同 key 重试：目录已存在，继续补登记
    sd = SessionDir.open(parent_dir / sid)
    agent.sessions.register(RegistryEntry.of(sd.config(), location=sd.path), who)   # 幂等
    if spec.origin.parent_session:                               # 由 UI 派生的 work session：父 session 自动半订阅它（S-17）
        agent.sessions.post_input(spec.origin.parent_session,
            Input.control("subscribe", mode="semi", source=session(sid)), who)
    return sd

def read_session(agent, sid, who, scope=("status",)) -> SessionView:
    e = agent.sessions.lookup(sid)                               # 登记表：Agent 总能看到全部 session 的 status
    view = SessionView(entry=e)
    if e.agent_access == "status_only" and who.is_agent_side(): return view   # 配置为只给状态摘要（Q10）
    sd = SessionDir.open(e.location)                             # 直接读文件；无权读取时返回 status 并注明原因
    view.state = sd.state()                                      # 先读提交点 state.json
    if "report" in scope: view.report = sd.report()
    if "worklog" in scope: view.worklog = sd.worklog().reverse_lines(end=view.state.worklog.committed_bytes).take(scope.n)   # 只反向读
    return view
```

`same_session_identity` 同时核对 agent_did、kind、creator principal、driver 与 idempotency_key；不能仅因 key 字符串相同就把已有目录当成本次 session。

---

## 5. Lease（推进权）协议：基于文件锁

v0.9 重新设计（V4）：推进权就是**长期持有的排他文件锁**，与 xllm 的 run 锁（`runs/<run_id>/.lock`）同一模式。持有者释放或退出（包括崩溃）时，锁由操作系统自动释放。v0.8 的 TTL、续约与时钟依赖全部去掉；DFS 上线后由它提供同一语义，对协议透明。

下文的 `lease` 指一把已持有的锁：锁文件句柄 + 本次获取的 epoch。

### 5.1 资源与锁文件

| 资源 | 锁文件 | 持有期间 | 用途 |
|---|---|---|---|
| `session:<sid>` | `<sid>/.opendan_agent_session/lease.json` | 一次 drive 的全程 | 同一时刻只有一个进程推进；持有者身份必须等于 `session.driver` |
| `run:<run_id>` | `<sid>/.opendan_agent_session/runs/<run_id>/.lock`（xllm） | 执行该 run 期间 | run 目录的执行互斥；libopendan runner 与接手的 xllm 共用 |
| `self_improve` | `<agent_root>/.locks/self_improve.lease` | 一次整理的全程 | 全局只有一个认知整理 |
| `artifact:<aid>` | `<agent_root>/.locks/artifact/<aid>.lease` | 短临界区 | 串行化产物 head 的移动 |

锁文件的内容是持有者信息，只由持有者**原地改写**（§4.7 `lock_rewrite`），永不替换、永不删除：

```jsonc
{ "resource": "session:work-…", "epoch": 42,         // 每次成功获取 +1：写入 state.json.writer.lock_epoch，用于审计与诊断
  "holder": { "runner_id": "rn-…", "principal": "app:app2@alice", "host": "did:dev:…", "pid": 1234, "runtime_id": "rt-…" },
  "acquired_at_ms": 0, "released_at_ms": null }
```

### 5.2 操作

```python
def acquire(res, holder) -> Lease | Busy | NotDriver:
    if res.kind == "session" and holder.principal != config(res).session.driver.principal:
        return NotDriver                                # Q9：推进身份不变，进程可以换
    lk = lock_try(res.lock_path)                        # 非阻塞排他锁（flock；DFS 上线后同语义）
    if lk is None:
        return Busy(read_holder_info(res.lock_path))    # 只用于显示“谁在推进”
    prev = parse_or_none(read_via(lk))
    info = LockInfo(res, epoch=(prev.epoch if prev else 0) + 1, holder=holder, acquired_at_ms=now())
    lock_rewrite(lk, info)
    return Lease(lk, info)

def release(lease):
    lock_rewrite(lease.lock, lease.info.with_released(now()))
    unlock_and_close(lease.lock)

def fenced(lease, write_fn):                            # 受 lease 保护的写入与副作用都经过这里
    if not lease.held(): raise LeaseLost(lease.info)    # 单节点 flock 在进程存活时不会丢失；DFS 上线后由它通知丢失
    write_fn()
```

- **没有 TTL，也没有续约**：锁只在持有者释放或退出时释放，不存在“时钟偏差导致提前接管”的问题。
- **锁文件永不替换**：flock 锁在 inode 上。v0.8 在持锁期间用 `atomic_replace` 替换 lease.json，替换之后新打开者拿到的是新 inode，可以同时拿到锁。所以锁文件只原地改写；其它状态文件照常用 `atomic_replace`。
- **fd 不被子进程继承**：锁 fd 必须带 CLOEXEC（Rust 的 `std::fs` 默认如此）。否则 exec_bash 启动的后台进程会在 runner 退出后继续持锁。
- **同一进程不重复加锁**：flock 属于“打开的文件描述”，同一进程两次打开同一锁文件并加锁会互相阻塞。libopendan 在进程内维护已持有锁的表。
- **锁保护的边界**（S-22）：文件锁保证协作 runner 的状态写入互斥；`lease.held()` 是新动作的准入检查，不是对已经启动的进程或远端请求的 fencing。runner 被 `kill -9` 时，锁会释放，但工具子进程可能仍在运行，Drop / finally 均不能保证执行。
- **接管前先核对旧工具**：获得 session / run 锁后、启动新推理或应用会产生副作用的 control 之前，按 RunRecord 的执行标识查询旧执行；仍存活的本地进程组先终止并等待退出。只有确认旧执行已停止才继续。执行标识至少关联 runtime / host、主机启动标识及进程创建身份，不能只凭可能复用的 PID / PGID 杀进程。无法定位、无权终止或无法证明已经停止时返回 `RecoveryBlocked`，保留现场，不自动继续。
- **启动窗口也要覆盖**：工具执行标识必须在用户命令获准运行之前持久化。native / tmux 的启动握手应保证：记录失败或 runner 在放行前退出，用户命令不执行；放行之后崩溃，接管方能按持久标识核对。后台子进程仍属于该执行，不能因 shell 返回就解除跟踪。具体执行原语复用并改造 `agent_tool::llm_bash` 与 runtime，不增加锁服务。
- **远端动作与失锁**：有可查询句柄的远端动作按句柄核对；无法确认是否仍在执行时阻塞自动接管，等待显式核对，不能把“结果未知”当作“已经停止”。正常取消或 DFS 失锁时，runner 停止派发、尝试终止工具并返回 `Lost`；即使清理未完成，新持有者仍必须执行上述检查。DFS 文件锁不替代远端服务自身的 fencing / 幂等机制。
- **卡死的持有者**：进程还活着但不再推进时，锁不会自动释放。由运维按锁文件中的 host / pid 终止该进程（开发 CLI 的 `holder <sid>` 显示持有者）；协议不提供强制抢锁。
- **对 DFS 的唯一要求**：提供“可长期持有、持有者（客户端会话）失效即释放、失去时通知持有者”的排他文件锁。

### 5.3 写入范围

| 权限来源 | 可写范围 |
|---|---|
| `session:<sid>` lease（驱动者身份） | `.opendan_agent_session/` 下除执行中的 run 目录之外的全部文件；`.runtime/`；产物；session 绑定的 workspace（按权限）；Agent State 中**本 session 的**登记条目 status（含 activity）与 `state/perception/<sid>.jsonl`；输入源确认（kmsg `commit_ack`） |
| `run:<run_id>` 锁 | `runs/<run_id>/`（run.json、snapshots）。libopendan runner 执行 run 时同时持有 session lease 与 run 锁；接手的 xllm 只持有 run 锁 |
| 投递（无需 lease） | session 的 kmsg 队列 |
| `self_improve` | `state/perception/.cursor.json`、`memory/`、`attention_signals/`（`notebook/` 由自身锁串行） |
| `artifact:<aid>`（修改 session 自己的版本时还须持 session lease） | `state/artifacts/<aid>/artifact.json`（head）及版本有效性变更；accept / discard 使用同一临界区 |

---

## 6. Agent State 协议（libOpenDAN::state）

### 6.1 布局

```text
<agent_root>/                               # 位于 DFS
  agent.toml role.md self.md users/ behaviors/ tool_plans/ tools/ skills/ i18n/   # 包层（OpenDAN rootfs_sync 维护；libopendan 只读）
  sessions/<sid>/                           # session 目录的默认位置（可选）
  state/
    sessions/<sid>.json                     # session 登记表（所有 session，无论目录在哪）；status.activity 构成活动视图（§6.7）
    perception/<sid>.jsonl                  # 感知
    perception/.cursor.json                 # 认知整理游标
    artifacts/<aid>/artifact.json           # 产物列表：长期产物与 head
    artifacts/<aid>/versions/<ver>.json     #   各 work session 的贡献（每个 session 一个文件）
  memory/  notebook/  attention_signals/    # 认知（沿用 agent_tool 实现与各自的内部锁）
  workspace/<wid>/                          # Agent 内部 workspace（沿用 .workspace.json；管理方式见 §6.5）
  .locks/self_improve.lease  .locks/artifact/<aid>.lease
```

### 6.2 Session 登记表（session mgr）

```jsonc
// state/sessions/<sid>.json
{
  "session_id": "work-…", "kind": "work", "class": "work",
  "created_by": { "principal": "app:app2@alice", "via": "app" }, "idempotency_key": "…",
  "driver": { "principal": "app:app2@alice" },
  "location": "/…/app2/agent_sessions/work-…",       // DFS 路径
  "input_queue": "app2::alice::opendan.session.work-…",   // 其它参与方据此投递（例如 UI 投递 decide）
  "agent_access": "full",
  "origin": { "parent_session": "ui-…" },
  "workspace": { "kind": "external", "path": "/…/app2/snake" }, "artifact_id": "snake-game",
  "status": { "rev": 17, "run_state": "finished", "outcome": "succeeded", "acceptance": "pending",   // rev = state.json 的 rev
              "one_line_status": "…", "report_brief": "≤500 字",
              "pending_decision": null,
              "activity": { "summary": "…", "touching": [ … ], "heartbeat_ms": 0 },   // 与 state.json.activity 相同（§6.7）
              "last_runner": { "runner_id": "rn-…", "host": "did:dev:…", "pid": 1234, "lock_epoch": 42, "at_ms": 0 },   // Q9
              "updated_at_ms": 0 }
}
```

```python
def register(entry, who):                                   # 创建者调用一次；幂等
    authorize(who, "session.create", entry)                 # 文件版：以能否写 state/sessions/ 为准
    if publish_noreplace(reg(entry.sid), entry): return entry
    cur = read(reg(entry.sid))
    if cur.idempotency_key == entry.idempotency_key and cur.created_by == entry.created_by: return cur
    raise SessionIdConflict(entry.sid)

def report_state(lease, sid, status):                       # 驱动者在每次提交后调用；本条目 status 的唯一写者
    fenced(lease, lambda: update_status_if_newer(reg(sid), status))   # rev ≤ 已有值时忽略
```

- `query(filter)` 是派生视图：扫描登记表并按 mtime 缓存。可以额外维护 `state/sessions/.index.sqlite`，但它必须能删除重建。三个典型用法：
  - S-14：`query(run_state != finished or acceptance = pending)`。
  - S-10：`query(artifact_id = X)`。
  - V5：`query(run_state in {running, waiting})` → 活动视图（§6.7）。
- `status` 只是缓存，真相是 session 自己的 `state.json`。回报失败的，下次 drive 开头补发（依据 `state.reported_rev`）。
- `pending_decision` 由驱动者在发现队列里有未处理的 decide 时写入，用于让 UI 显示“等待 app2 处理”。
- `verify` 巡检：location 已不存在的条目只标记为 `unreachable`，不删除。

### 6.3 感知管理

感知是**跨 session、只追加、低成本**的观察流：写入时不调用 LLM，也不同步触发整理（S-29）。

```jsonc
// state/perception/<sid>.jsonl（单写者 = 本 session 的驱动者，持 session lease；seq 严格单调）
{"seq":31,"at_ms":0,"session_id":"work-…","kind":"run_digest",
 "source":"session|self","tags":["snake","ui"],"objects":["artifact:snake-game"],"summary":"<one_line_status>",
 "payload":{"run_id":"…","turn":12,"turn_status":"completed"},"refs":{"worklog_seq":317}}   // turn_status 只在该次 run 结束同时关闭 Turn 时出现
{"seq":32,"kind":"observation","source":"session","payload":{ /* Discover* 结构：event / object / relationship，含 evidence worklog refs */ }}
{"seq":33,"kind":"task_outcome","payload":{"outcome":"succeeded","acceptance":"pending","artifact_version":"…"}}
{"seq":34,"kind":"task_discarded","payload":{"session":"work-…","artifact_version":"…"}}   // S-27：保留来源
```

| 来源 | 时机 | 成本 |
|---|---|---|
| `run_digest` | runner 在每个 run 结束提交后自动写 | 零 LLM |
| `observation` | LLM 调用 `perceive` 工具，或子进程以 `perception` 类型投递到 kmsg 队列 | 一次工具调用 |
| `task_outcome` / `task_discarded` | finished / accept / discard 时自动写 | 零 LLM |

```python
def append_perceptions(lease, sid, records):                 # 驱动者调用
    last = tail_seq(perc(sid))                               # 反向读最后一行；重试幂等：跳过 seq ≤ last 的记录
    fenced(lease, lambda: append_batch(perc(sid), [r for r in records if r.seq > last]))

def backlog(cursor) -> Backlog:                              # 派生：只用 stat
    b = Backlog()
    for f in list(perception_root, "*.jsonl"):
        sid = stem(f); off = cursor.offsets.get(sid, 0)
        if size(f) > off and registry(sid).kind != "self_improve":   # 防自我回声
            b.add(sid, from_offset=off, to_offset=size(f))
    return b
```

- `perception/.cursor.json` 按 session 记录字节偏移，只由 `self_improve` lease 的持有者推进。它取代现有的 `already_improved` 字段。
- 提交后如果追加失败或进程崩溃，下次 drive 开头按 `state.perception_seq` 补发；缺失的 run_digest 按 state.json 重建（`last_run`、当前 Turn、`one_line_status`、worklog 已提交的 seq）。

### 6.4 认知管理

认知由三部分组成：Memory Graph（`memory/`）、Notebook（`notebook/`）、整理中间态（`attention_signals/`）。实现沿用 `agent_tool::{agent_memory, agent_notebook, agent_attention_signal}`，libopendan 只包一层门面：

```rust
pub trait Cognition {
    /// 任何 session 都可调用，只读；返回 time + sentence + id 形式的线索（HintRecallEngine 移植）
    async fn recall_hints(&self, q: RecallQuery) -> Result<Vec<Hint>>;
    /// 普通 session 的显式声明（用户要求“记一下”）
    async fn notebook_append(&self, item: NotebookItem, who: &Principal) -> Result<()>;
    /// 仅限 self_improve lease 持有者：提交一批整理结果后推进感知游标（at-least-once 整理）
    async fn commit_consolidation(&self, lease: &Lease, batch: ConsolidationBatch, upto: PerceptionCursor) -> Result<()>;
}
```

- **跨语言边界是 CLI**（Memory v2 §0）：非 Rust 实现通过调用 `agent_tool agent-memory|agent-notebook …` 完成，不重写 Memory Graph。
- **S-30**：`<hints>`（过去形成的线索）与 `<changes>`（当前世界的变化）在 prompt 中分块，共享上下文预算。
- **S-27**：`task_discarded` 保留了来源链；self-improve 如何处理相关认知，**策略待定**。

### 6.5 产物列表（只登记与指向）

按 Q13，产物的复杂度不进入 Session 协议：

| 产物 | 位置 | 管理者 | Session 协议负责什么 |
|---|---|---|---|
| 一次性产物 | session 目录本身 | session | 随 session 保存；accept / discard 只改变其有效性标记 |
| 长期产物 | workspace | workspace | session_config 引用 workspace；binding.json 记录 workdir；产物列表登记“哪个 work session 对哪个 workspace 做了什么” |

workspace 分两类，它们的版本化、合并与回滚都**另行设计**：

- **Agent 内部 workspace**（`<agent_root>/workspace/<wid>`）：由 Agent 自己管理。默认 paios runtime 内置 git，可以用来做版本管理。
- **外部 workspace**（用户或应用指定的目录）：由其所有者管理，Agent 侧不承诺能回滚。

Agent State 的产物列表只负责**登记与指向**，服务于三件事：

- 意图分析定位“用户接受过的那个”（S-10）；
- 新 session 从有效版本继承（S-12）；
- 放弃后标记失效（S-14）。

```jsonc
// state/artifacts/<aid>/artifact.json（head 由 artifact lease 串行化）
{ "aid": "snake-game", "workspace": { "kind": "agent", "id": "ws-…" }, "head": "v-work-A" }
// state/artifacts/<aid>/versions/v-<sid>.json（每个 work session 一个文件，由其驱动者单写；改变有效性还须持 artifact lease）
{ "ver": "v-work-B", "session": "work-B", "base": "v-work-A", "state": "produced | accepted | discarded",
  "outputs": ["report.md", "snake.js"],                          // session 目录中的一次性产物
  "workspace_ref": { "rev": "<git commit | 快照 id | null>" },   // 对 workspace 的变更引用，由 workspace 机制给出；外部 workspace 可以为空
  "side_effects": [{ "call_id": "c-9-1", "tool": "sendmsg", "note": "已发送的消息无法撤回" }] }   // 由反向读 worklog 得到
```

```python
def apply_decide(ls, sd, decision, agent):                 # 驱动者在 drive 开头应用 control(decide)（Q9）
    aid = sd.config().artifact_id
    v = None
    report = None
    if aid:
        with agent.leases.acquire(f"artifact:{aid}") as la:
            v = agent.artifacts.version(aid, f"v-{sd.id}")       # 在锁内重读版本与 head，不使用锁外缓存
            head = agent.artifacts.head(aid)
            if decision == "accept":
                validate_accept(v)
                agent.artifacts.set_state(la, v, "accepted", head=True)
            else:
                target = head
                if head == v.ver:
                    target = agent.artifacts.nearest_valid_base(la, v.base)   # 只选仍 accepted 的祖先；没有则为 null
                agent.artifacts.set_state(la, v, "discarded", head_to=target) # head 已指向其它版本时保持不变
    if decision == "discard":
        report = workspace_discard(sd.config().workspace, v)       # 交给 workspace：Agent 内部 workspace 可回滚；外部 workspace 通常 unsupported
        report.unsupported += v.side_effects if v else []          # S-26：外部副作用逐项列为不可撤销
    sd.worklog.append(ls, decide_entry(decision, report))          # 决策先进入历史
    sd.commit_state(ls, acceptance=decision_to_acceptance(decision), result=merge(result, discard_report=report))   # 提交点
    if decision == "discard": agent.perception.append(ls, sd.id, [task_discarded(v)])   # 提交后追加，失败按已提交决策补发
```

所有 head 移动（包括 discard 回退）和版本有效性变更都必须持 `artifact:<aid>` 锁，不能传空 lease。`nearest_valid_base` 在同一锁内检查祖先状态，跳过已 discarded 的版本；缺失或循环引用返回显式错误。锁顺序固定为 session → artifact，workspace 的慢操作不在 artifact 临界区内。这个约束只保护登记表，不表示 workspace 已回滚。

**意图分析定位对象**（S-10、A-05）：用 `sessions.query` / `artifacts.query` 找出相关的长期产物，取它的 **head**（用户接受过的版本），而不是最后一个 finished 的 session；候选不止一个时先澄清。新 work session 引用同一个 workspace，base 取 head（S-12、A-06）。

### 6.6 AgentStateClient（SDK 表面与实现）

Agent State 的所有访问都经过 `AgentStateClient`（V3）。它是 SDK 的抽象边界，不是协议；协议是它背后的文件布局与锁（§5、§6.1 ~ §6.5、§6.7）。

```rust
#[async_trait]
pub trait AgentStateClient: Send + Sync {
    fn agent_did(&self) -> &str;
    fn agent_id(&self) -> &str;                    // kevent / kmsg 命名用（只含字母、数字和 _ - .）
    fn sessions(&self) -> &dyn SessionRegistry;    // register / report_state / lookup / query / post_input(sid, …)
    fn activity(&self) -> &dyn ActivityView;       // active(filter)：活动 session 视图（§6.7）
    fn perception(&self) -> &dyn Perception;       // append / backlog / cursor
    fn cognition(&self) -> &dyn Cognition;
    fn artifacts(&self) -> &dyn Artifacts;         // head / versions / register_version / set_state / query
    fn locks(&self) -> &dyn LockManager;           // self_improve / artifact 锁（session 锁在 SessionDir 上）
}
```

| 实现 | 阶段 | 说明 |
|---|---|---|
| `FsAgentStateClient` | 本期 | 直接读写 AgentRoot 上的文件，用文件锁协调（§5）。要求 runner 能看到 AgentRoot（A8） |
| `KrpcAgentStateClient` | OpenDAN 改造后 | 部分操作走 OpenDAN 提供的 kRPC，其余仍走文件。候选是需要跨信任域或需要服务端校验的操作（登记写入、感知追加、recall_hints 等）；具体迁移哪些，按实现结果与权限模型再定 |
| DFS | DFS 上线后 | `FsAgentStateClient` 不变；文件锁由 DFS 提供同一语义 |

- 两种实现必须对外表现一致：同一组 fixtures（L6）同时跑在两者之上。
- session 目录本身（§4）不经过 `AgentStateClient`：它属于驱动者，永远按文件协议读写。
- 文件版的 `open(agent_root, who)` 只需要 AgentRoot 路径与驱动者身份，不需要额外服务。

### 6.7 活动 Session 视图（避让，V5）

每个 session 的上下文里都有一条重要信息：**系统里还有哪些 session 正在运行、大概在改什么**。Agent 据此自己避让，不去同时修改别的 session 正在修改的东西。这是协作提示，不是锁；并发修改的最终裁决属于 workspace 机制（Q13）。

**数据**：不新增文件。每个 session 的 `activity` 写在自己的 state.json 里（§4.3），随状态回报进入登记表 `status.activity`（§6.2）；本条目的唯一写者仍是驱动者。

```jsonc
"activity": {
  "summary": "给贪吃蛇加穿墙模式：正在改碰撞检测",       // 一句话：在做什么（缺省取 objective + one_line_status）
  "touching": [                                          // 大概在改什么；条数有上限（如 16），粒度由声明者决定
    { "kind": "path", "ref": "ws:snake/src/collision.js", "mode": "write", "since_ms": 0 },   // ws:<workspace>/… 或 session 目录内相对路径
    { "kind": "object", "ref": "https://…/calendar/alice", "mode": "write" },
    { "kind": "artifact", "ref": "snake-game", "mode": "write" }
  ],
  "heartbeat_ms": 0                                      // runner 节流刷新（例如每 60 秒，或 touching 变化时）
}
```

**来源**（由粗到细）：

1. **创建时的声明**：`session_config.session.scope`，加上 `workspace` / `artifact_id`（由应用或意图分析给出）。
2. **Agent 自己声明**：通过 CLI（例如 `agent-session activity --touch <ref>`，经 exec_bash）向本 session 的 kmsg 队列投递 `control(activity)`，持有者在观察边界合并。用 CLI 而不是进程内工具，是为了让 xllm 接手的 run 也能声明（§4.4）。
3. **runner 推断**（尽力而为）：从写类工具调用中能识别出的路径（例如文件写工具的参数）。exec_bash 内部的写入不推断。

**更新时机**：commit_input_batch、run 进行中的 checkpoint（节流）、handle_context_outcome。run 结束且没有挂起时清空 touching；session finished 时整个 activity 清空。

**活跃判定**：`run_state ∈ {running, waiting}`。`running` 且 `heartbeat_ms` 超过阈值（默认 5 分钟）的条目显示为“可能已中断”；`waiting` 不要求心跳。

**进入上下文**：

- `ActivityView::active(filter)` 返回同一 Agent 下的其它活动 session（按 `agent_access` 过滤），按与本 session 的相关度排序：同一 workspace / artifact 优先，其次 touching 有交集，最后是其它；数量受预算约束。
- `BehaviorAssembler` 在首个输入批次（on_init）渲染 `<active_sessions>` 段；之后本 session 对“活动 session 集合”默认半订阅，集合或交集变化时在观察边界以 change 注入（§8.4）。与本 session 的 scope / touching **有交集**的条目会被标出。
- 这些内容属于输入批次消息的变量段（`render_input` 渲染的 `<session_input>`），不进入 system 段，因此不破坏稳定前缀。
- 约束段（§8.1 第 2 段）包含避让规则：不要同时修改其它活动 session 正在写的对象；需要时等待（订阅对方状态）、先做其它部分，或在报告中说明冲突。

**边界**：

- 视图是尽力而为的提示：声明可能不完整，心跳可能滞后，不承诺互斥。
- 冲突检测、合并与回滚属于 workspace（S-25、A-13 的闭环依赖）。
- 跨用户、跨信道的可见范围与 S-21 一起设计；本期只按 `agent_access` 过滤。

---

## 7. Agent Runtime（libOpenDAN::runtime）

### 7.1 逻辑 runtime 与绑定

**Runtime** 是为 `exec_bash` 提供执行环境的**逻辑身份**：在哪台主机上执行、看到什么文件系统视图、PATH 上有哪些工具、用 native 还是 tmux 执行。它不是一个进程；tmux server 或容器重启后，仍是同一个 runtime。

```jsonc
// RuntimeDescriptor（由提供方声明）
{
  "runtime_id": "rt-jarvis-default", "kind": "tmux | native", "host": "did:dev:…", "provider": "opendan | app:<app_id>",
  "fs_view": { "agent_root": "/…/agents/jarvis", "workspace_root": "…/workspace", "extra": ["/…/app2"] },
  "path_layers": ["<sid>/.runtime/bin", "<agent_root>/tools", "<instance>/tools/bin", "/opt/buckyos/tools/store"],
  "capabilities": { "tools": { "git": "2.43", "node": "22" }, "network": true, "os": "linux" }
}
// .opendan_agent_session/binding.json（首次推进时写入，只写一次）
{ "runtime_id": "rt-jarvis-default", "kind": "tmux", "workdir": "/…/workspace/ws-…", "bound_at_ms": 0, "bound_by": "rn-…" }
```

```python
def bind_or_verify(sd, lease, rt: AgentRuntime, tools) -> Binding:
    b = sd.binding_opt()
    if b is None:
        workdir = resolve_workdir(sd.config().workspace, rt.descriptor()) or sd.dir   # 有 workspace 用 workspace，否则用 session 目录
        b = Binding(rt.descriptor().runtime_id, rt.kind, workdir)
    if b.runtime_id != rt.descriptor().runtime_id:
        raise RuntimeMismatch(b.runtime_id)          # 换进程只能在同一个逻辑 runtime 上恢复（A-03）
    check_requirement(sd.config().runtime.requirement, rt.descriptor())   # 每次推进重新验证，不只在首次绑定时验证
    check_app_tools(sd.config().runtime.requirement.app_tools, tools)
    if not rt.can_access(b.workdir): raise BindError("runtime 看不到 workspace")
    if sd.binding_opt() is None:
        if not publish_noreplace(sd.state_dir / "binding.json", to_json(b)):
            if sd.binding() != b: raise RuntimeMismatch(sd.binding().runtime_id)
    rt.prepare_session_bin(sd)                       # 每次幂等修复：Agent tools、tool_plan 墓碑、PATH 所需文件
    rt.verify_session_env(sd, b)                      # 确认完整后才允许 open / 推理；失败保留 binding，供同一 runtime 重试
    return b
```

- **xllm 接手要求 native runtime**（§4.4）。tmux runtime 的 run 只能由 libopendan runner 恢复。
- `NativeRuntime` 的 runtime_id 通常包含主机标识，所以 session 实际被钉在一台主机上；主机失效后 session 无法恢复。是否允许显式重新绑定见 §15.1。

驱动者身份（`session.driver`）与 runtime（`binding.json`）共同决定“谁、在哪里”推进这个 session，两者都不可变。绑定失败发生在任何推理之前：提交带 `last_error` 的 state.json 并回报，然后返回（Q3）。

`binding.json` 只证明 runtime 身份已经绑定，不证明环境准备完成。`prepare_session_bin` 先生成完整的预期工具集（包括禁止工具的墓碑），再发布并核验；中途失败留下的临时或部分文件必须能在下一次调用中修复。`open_session_env` 再检查 cwd、PATH、工具与墓碑完整性；失败返回 BindError，不在半成品环境中启动命令。xllm 接手也必须通过同一环境校验；需要修复 `.runtime/` 时先交回 libopendan。

### 7.2 接口与实现

```rust
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    fn descriptor(&self) -> &RuntimeDescriptor;
    /// 幂等修复 <sid>/.runtime/bin：Agent tools 同步 + tool_plan 墓碑；失败可重试
    async fn prepare_session_bin(&self, sd: &SessionDir) -> Result<()>;
    async fn verify_session_env(&self, sd: &SessionDir, binding: &Binding) -> Result<()>;
    /// 核验完整性后打开执行视图：tmux session、cwd、PATH（.runtime/bin 在最前）、env
    async fn open_session_env(&self, binding: &Binding, s: &SessionCtx) -> Result<SessionEnv>;
    /// 准备执行标识与启动握手；持久化标识并放行之前，用户命令不得执行（§5.2）
    async fn prepare_execution(&self, call: &ToolCall) -> Result<ExecutionTicket>;
    /// 确认旧执行已停止，必要时终止并等待；无法确认返回 RecoveryBlocked
    async fn reconcile_execution(&self, execution: &ExecutionRef) -> Result<()>;
    async fn exec_bash(&self, env: &SessionEnv, req: BashRunRequest) -> Result<BashRunOutput, AgentToolError>;
    async fn interrupt(&self, env: &SessionEnv) -> Result<()>;
    async fn status(&self) -> RuntimeStatus;   // 给提示词引擎：活跃 tmux 会话、后台进程、磁盘、工具清单
    async fn close_session_env(&self, env: SessionEnv) -> Result<()>;
}
```

| 实现 | 来源 | 执行方式 | 典型用途 |
|---|---|---|---|
| `NativeRuntime`（本期重点） | 包装并补齐 `agent_tool::llm_bash::LocalProcessBashRunner` 的执行跟踪 | 每条命令一个 `/bin/bash -c`，独立进程组；用户命令启动前持久化执行标识 | 外部应用、测试；xllm 共用启动与恢复规则 |
| `TmuxRuntime` | 从 `opendan::agent_bash::TmuxBashRunner` 移植 | 每个 session 一个 `od_<sid>` tmux session，可以 attach 审计 | 将来 OpenDAN 的默认 paios runtime（容器内） |

`prompt_env` 新增 `runtime.status` 变量，数据来自 `AgentRuntime::status()`。

执行跟踪的生命周期独立于 `inflight`：工具结果已经持久化，也可能还有后台子进程。只有确认整个受管执行已停止，才能移除执行记录。runtime 不支持可靠核对的后台 / 脱离进程组执行应拒绝，或返回 `RecoveryBlocked` 等待显式处置；不能沿用“shell 退出即解除所有跟踪”的行为。显式交给 task_mgr 的 PendingTool 使用其任务句柄与生命周期，不能误当作失联的本地进程清理。

### 7.3 exec_bash 的环境契约

| 变量 | 说明 |
|---|---|
| `OPENDAN_AGENT_DID` | Agent DID（新增） |
| `OPENDAN_AGENT_ROOT` | AgentRoot 的 DFS 路径（沿用） |
| `OPENDAN_SESSION_DIR` | session 目录（新增：session 可以在 AgentRoot 之外，不能再从 AgentRoot 推出） |
| `OPENDAN_SESSION_ID` / `OPENDAN_TRACE_ID` / `OPENDAN_RUNTIME_ID` | 标识 |
| `OPENDAN_INPUT_QUEUE` | 本 session 的 kmsg 队列 URN（新增）；子进程可向它投递 `perception`、`control(activity)` 等输入 |
| `BUCKYOS_APPCLIENT_SESSION_TOKEN` | 沿用：子进程以驱动者身份访问 BuckyOS 服务 |

- 子进程以驱动者身份运行，但**不持有 lease**，也不直接写 `.opendan_agent_session/`。
- 现有 `agent_tool` CLI 按 `OPENDAN_AGENT_ROOT` + `OPENDAN_SESSION_ID` 推导 session 目录，需要在 opendan 切换时改为支持 `OPENDAN_SESSION_DIR`（附录 A.7）。

---

## 8. Session Runner（libOpenDAN::runner）

> **Runner 是实现，不是协议**（V1）。本节描述 Rust 参考实现；buckyos-websdk 的 ts-runner 将来依据反写的 Spec（L6）实现同样的目录、提交与锁语义，内部结构可以不同。唯一的例外是 §8.7：xllm 的 run 目录是协议的一部分。

### 8.1 扩展点

```rust
pub struct RunnerDeps {
    pub who: Principal,                              // 驱动者身份 = 驱动进程的 appid（Q7、Q16）：决定读 inbox、回复、调用 LLM 的身份与权限
    pub agent: Arc<dyn AgentStateClient>,             // 本期为 FsAgentStateClient（§6.6）
    pub inputs: Arc<dyn InputChannelFactory>,        // 按 session_config.channels 建立 KmsgInput / MsgCenterInput
    pub waker: Arc<dyn Waker>,                       // kevent 订阅 wake_event + 轮询兜底
    pub runtime: Arc<dyn AgentRuntime>,
    pub llm: Arc<dyn llm_context::LlmClient>,        // 以 `who` 的身份调用 AICC（计费、审计归 App；Q7）
    pub tools: Arc<dyn ToolFactory>,
    pub assembler: Arc<dyn SessionAssembler>,        // 装配差异点（S-05、S-06）
    pub notifier: Arc<dyn Notifier>,                 // 状态变化通知（kevent）；默认 Noop
    pub outbound: Option<Arc<dyn OutboundSink>>,     // UI 回送（后移）：以驱动进程的 appid 身份发送（Q16）
}

#[async_trait]
pub trait SessionAssembler: Send + Sync {
    /// 由 session_config.prompt（.llm_context 语义）+ 身份 / 约束段生成 request 的 system 段与策略（历史部分见 §4.4）
    async fn build_request(&self, cfg: &SessionConfig, env: &PromptEnv) -> Result<LLMContextRequest>;
    /// 每个 hook point：把本次输入批次的 inputs / changes / hints / 活动 session 渲染成 user message（`<session_input>`）；
    /// 返回 None 表示无需推理。批次开启还是并入 Turn 由 runner 决定，不由 assembler 决定
    async fn render_input(&self, s: &SessionView, hook: HookPoint, m: &InputMaterial) -> Result<Option<AiMessage>>;
}
```

- **默认实现 `BehaviorAssembler`**：`session_config.prompt.behavior` 指向 `behaviors/<name>`，再加上 `prompt_env` 与 driver hook point；从 opendan 移植。
- **应用装配**（S-05）按以下固定顺序合成，应用 prompt 不能替换前两段（S-08）：
  1. Agent 身份（role.md / self.md）
  2. Agent 不可覆盖的约束段（含活动 session 的避让规则，§6.7）
  3. 应用 system prompt（`prompt.system_prompt`）
  4. hints 段
  5. objective / end_condition
- **新鲜度**（S-20）：时间、时区等由 PromptEnv 在渲染时求值，放在输入批次消息的变量段。
- **渲染无副作用**：现有 prompt_env 在渲染时推进的“上次看到”游标，移植后改为显式写入 state.json，渲染本身保持纯函数（§4.4 的确定性）。

### 8.2 drive：推进一个 session

```python
async def drive(sd: SessionDir, deps: RunnerDeps, until: StopWhen) -> DriveResult:
    lease = await deps.agent.leases.acquire(sd.lease_resource(), deps.holder())   # 内含 driver 身份校验（Q9）
    if isinstance(lease, NotDriver): return DriveResult.not_driver()
    if isinstance(lease, Busy): return DriveResult.busy(lease.holder)
    extra = []
    try:
        s = await sd.load()                              # session_config + state.json（都是小文件）
        reg = deps.agent.sessions.lookup(s.id)
        if reg is None or reg.location != sd.path: return DriveResult.unregistered()   # 未登记的 session 不推进
        s.channels = deps.inputs.open(s.config.channels) # 本期 KmsgInput；MsgCenterInput 后移
        reconcile_runs(lease, s, deps)                   # 持 run 锁；格式与旧执行校验；补交 input receipts；重做结束（§4.4）
        confirm_committed_inputs(s)                     # 即使没有新输入，也重试确认已提交的消费位置
        await catch_up_reports(s, lease, deps.agent)     # 补发登记表回报 / 感知（§6.2、§6.3）
        inputs = fetch_inputs(s)
        s.apply_controls(lease, inputs.take("control"))  # subscribe / unsubscribe / stop / decide / activity（finished 之后仍处理 decide）
        if s.state.run_state == "finished":
            s.reject_leftovers(lease, inputs); return DriveResult.finished(s.state)
        try:
            binding = bind_or_verify(sd, lease, deps.runtime, deps.tools)            # 首次推理之前（Q3）
            env = await deps.runtime.open_session_env(binding, s.ctx())             # 再次校验失败也按 bind_failed 回报
        except (BindError, RuntimeMismatch) as e:
            s.commit_state(lease, last_error=e.to_json()); report(s, lease, deps)
            return DriveResult.bind_failed(e)
        extra = await acquire_kind_leases(s, deps.agent) # self_improve：拿不到就以 stopped/busy 结束
        ctx = resume_live_run(s, lease, deps, env)       # §8.6：state.live_run 指向未结束的 llm_context 时（先拿 run 锁）直接恢复，否则为 None
        outcomes = 0                                     # 本次 drive 处理的 Outcome 数（StopWhen.max_outcomes）

        while lease.held():
            hook = s.next_hook_point()                   # on_init | on_wakeup | on_behavior_switch（driver 沿用）
            picked = inputs.select(s.driver.pull(hook))  # pull_msg / pull_event（沿用）；change 不在此列
            changes = await check_changes(s, inputs, deps.agent, budget=s.driver.change_budget)   # §8.4（含活动 session 集合的变化）
            hints = await deps.agent.cognition().recall_hints(s.topic()) if s.driver.load_hints(hook) else []
            msg = await deps.assembler.render_input(s.view(), hook, InputMaterial(picked, changes, hints, await deps.runtime.status()))
            if msg is None and not s.needs_bootstrap() and not (ctx and ctx.ready_to_run()):  # 已恢复的执行不能被“没有新输入”挡住
                if until.idle: break
                await deps.waker.wait(s, until)          # 等 kevent 唤醒或轮询间隔到
                inputs = fetch_inputs(s); s.apply_controls(lease, inputs.take("control")); continue

            ctx = ctx or next_llm_context(s, lease, deps, env)   # 先读 summary.json，再反向读 worklog（§4.4）
            if msg is not None or s.needs_bootstrap():
                await commit_input_batch(s, lease, ctx, picked, changes, msg, hook)   # 开启或并入 Turn；消息与 receipt 同快照 → state → 清除提交门槛 → 确认输入
            outcome = await ctx.run()                            # llm_context；工具经 §8.5 的适配层执行
            nxt = await handle_context_outcome(s, lease, ctx, outcome, deps)   # Outcome 结束一个 run 段，是否结束 Turn 由 nxt.turn_end 决定
            outcomes += 1
            ctx = None if nxt.run_ended else ctx
            if nxt.finished or until.satisfied(outcomes): break
            if nxt.waiting and until.idle: break
            inputs = fetch_inputs(s)

        return DriveResult.from_state(s.state)
    except LeaseLost:
        return DriveResult.lost()
    except RunBusy as e:
        return DriveResult.busy(run=e.run_id)            # 例如 xllm 正在接手该 run
    except RecoveryBlocked as e:
        s.commit_state(lease, last_error=e.to_json()); report(s, lease, deps)   # 只记录错误，保留 live_run / 消费位置 / run 引用
        return DriveResult.recovery_blocked(e)           # 不清空 run、不新建 context、不启动工具
    finally:
        await release_all([lease] + extra)               # 同时释放 run 锁（若持有）
```

`StopWhen` 的取值：

- `idle`：没有输入就退出。用于常驻托管（OpenDAN，或驱动 UI session 的应用）。
- `finished`：一直推进到 work session 结束。
- `max_outcomes(n)`（`StopWhen::MaxOutcomes`，CLI `--until outcomes:<n>`；2026-10-01 前名为 `max_rounds`）：drive 主循环处理完 n 个 LLMContext Outcome 后返回（每启动或恢复一个 run 段计一个，不论 done、切换、挂起还是错误）。它不是 Round（推理）数、`run()` 调用数（上下文上限重写会在一段内再次调用 `run()`），也不是 Turn 数。每段开始前检查，`n = 0` 只做恢复与 control；session 结束、出错或无事可做时提前返回。按完成的 Turn 数结束 session 用 `end_condition.type = max_turns`（§4.2）。

### 8.3 输入批次的提交顺序与 Turn 边界

**输入 receipt 协议**：所有进入上下文的 msg、event、change 和拉取的订阅变化，都带结构化 `input_receipts`，作为 xllm 快照的可选宿主元数据（X2 / X4）。receipt 至少包含 `run_id`、单调 `input_seq`、所属 Turn（`turn`）及是否开启它（`opens_turn`）、hook、输入的 source / index 或 record_id / key、订阅版本与游标更新、对应消息位置和 `turn_started` / `input_batch` 所需元数据；批次 ID 为 `(run_id, input_seq)`。消息正文与 receipt 必须在同一份快照中，不能靠解析提示词找 key。xllm 续跑、压缩与后续 checkpoint 都保留这些元数据。

写入顺序固定为：① 快照 fsync；② 原子发布 run.json 的 `latest_snapshot_idx` 与 `host_commit_pending=input_seq`；③ 将该批次的 Turn 归属（`open_turn` / `turn_seq`、`live_run.turns`）/ 消费位置 / 订阅游标提交到 state.json；④ 原子清除 run.json 的 `host_commit_pending`；⑤ 确认输入源。在④之前不允许开始新推理或工具调用。`host_commit_pending` 是宿主输入提交门槛，不表示工具动作在途；xllm 遇到它必须拒绝接手，由 libopendan 补交 state 后再接手。

```python
async def commit_input_batch(s, lease, ctx, picked, changes, msg, hook):   # 开启或并入 Turn；从不结束 Turn
    opens = s.state.open_turn is None                       # 没有进行中的 Turn：本批次开启新 Turn
    turn = s.state.turn_seq + 1 if opens else s.state.open_turn.index
    receipt = make_input_receipt(ctx, turn=turn, opens_turn=opens, hook=hook, picked=picked, changes=changes)   # 稳定批次 ID (run_id, input_seq)；含游标补交所需元数据
    ctx.inject_with_receipt(msg, receipt)
    prepared = s.runs.prepare_input_checkpoint(lease, ctx.run_id, ctx.snapshot(), receipt.input_seq)   # 新建或续用 run；持 run 锁；fsync 快照与带门槛的 run.json
    s.state.apply_input_receipt(receipt)                    # 幂等更新 live_run.turns、applied_input_seq、open_turn / turn_seq、消费位置、订阅游标、bootstrap_done
    s.state.update(run_state="running", activity=refresh_activity(s, picked))
    commit_pop(lease, s, picked + changes.consumed_inputs, prepared)   # state → 清除 host_commit_pending → 确认输入

def reconcile_input_receipts(lease, s, cp, record):
    cp.validate_input_receipts(record.run_id)               # 批次连续、消息与标识对应；失败保留现场并阻塞
    require_checkpoint_covers_state(cp, s.state.live_run)    # state 已消费的批次必须有证据；不能拿旧快照倒退恢复
    delta = cp.receipts_after(s.state.live_run.applied_input_seq)
    if delta:
        for receipt in delta: s.state.apply_input_receipt(receipt)
        s.commit_state(lease)                              # 只补交元数据；不再 append 已经在快照中的消息
    if record.host_commit_pending is not None:
        require_state_covers(s.state, record.host_commit_pending)
        s.runs.complete_host_commit(record)                 # state 已提交而门槛未清除时也走这里；输入源确认在 drive 中重试

async def handle_context_outcome(s, lease, ctx, outcome, deps) -> Next:   # 返回 Outcome 不等于 Turn 完成
    nxt = classify(outcome, s)       # 沿用 handle_outcome：Done / WAIT_USER_MSG / next_behavior / END / PendingTool / Budget / Error / Interrupted / ContextLimitReached；
                                     #   同时给出 nxt.turn_end（completed | failed | budget_exhausted | stopped，或 None = Turn 继续）
    s.runs.checkpoint_with_results(lease, ctx.run_id, ctx.snapshot(), status=nxt.run_status)   # 1. 结果与快照先持久化，再清除已覆盖的 inflight（§8.5）
    s.runs.record_usage(ctx.run_id, outcome.usage, ctx.rounds.take())      #    本段的推理尝试数累加到 run.json usage.llm_requests 与 static.json rounds
    return finish_run(lease, s, ctx.run_id, ctx.snapshot(), outcome, nxt, deps)

def finish_run(lease, s, run_id, snapshot, outcome, nxt, deps) -> Next:   # reconcile_runs 重做时也调用它（幂等）
    turn = s.state.current_turn()                                        #    进行中的 Turn，没有则为最近一个；本 run 的条目归属于它
    if nxt.run_ended:
        ensure_previous_execution_stopped(s.runs.record(run_id), deps.runtime)   # 结束前也要核对后台执行，不能只看 shell 的返回值
    if nxt.finished:
        v = register_outputs(lease, s, deps.agent)                       # 2. 产物登记（§6.5；幂等）
        fenced(lease, lambda: atomic_replace(s.dir / "report.md", render_report(outcome)))
        s.state.update(result=build_result(outcome, v), acceptance="pending")
    if nxt.run_ended:
        if nxt.turn_end and s.state.open_turn:                           #    只有 session 关闭 Turn，且与 run 结束同一提交
            s.state.close_turn(nxt.turn_end)                             #    open_turn = None；completed 时 turns_completed + 1
        flush_run(lease, s, run_id, snapshot, turn, nxt.turn_end)        # 3. 该 run 尚未写入的历史 + outcome（+ turn_ended）一次性追加到 worklog（§4.4）
        prev = s.state.last_run
        s.state.update(live_run=None, last_run=run_id)                   #    保留最后一次 run 的 llm context 状态
    s.commit_state(lease, **nxt.state_patch())                          # 4. 提交点：state.json 原子替换（rev+1，worklog.committed_* 前移）
    if nxt.run_ended and prev and not s.state.references(prev):
        s.runs.remove_if_safe(lease, prev)                               # 5. 持 run 锁并确认无未核对的执行后清理；失败则保留
    maybe_compact(s, lease, deps)                                        #    按上下文占用比例触发 compact（§4.4）
    if nxt.run_ended:
        s.static.update(lease, outcome.usage, runs=+1, turns=s.state.turns_completed)   # Round（推理尝试）已在 handle_context_outcome 累加
    # ── 以下在提交之后执行；失败或崩溃时，下次 drive 开头补发 ──
    deps.agent.sessions.report_state(lease, s.id, s.status())            # 6. 登记表 status（rev 单调）
    deps.agent.perception.append(lease, s.id,                            # 7. 感知（seq 单调）：run 结束时写 run_digest，Turn 同时关闭则带 turn_status
        ([run_digest(s, run_id, turn, nxt.turn_end)] if nxt.run_ended else []) + ([task_outcome(s, outcome)] if nxt.finished else []))
    deps.notifier.session_changed(s.id, s.state.rev)                     # 8. 通知
    if s.is_ui and outcome.has_text and deps.outbound: await deps.outbound.post(s, outcome.text)
    return nxt

def flush_run(lease, s, run_id, final_snapshot, turn, turn_end) -> WorklogEnd:
    entries = run_history_entries(run_id, final_snapshot, flush_marks(s.state.live_run), turn)
        # turn_started / input_batch（取自快照 receipt）/ user_message / assistant_message（function call）或 step（behavior，带 step_index）/ action_result，
        # 只写 flush 游标之后的部分；再追加 outcome，turn_end 非空时追加 turn_ended
    s.worklog.truncate_to(lease, s.state.worklog.committed_bytes)   # 截掉上次崩溃遗留的未提交尾部（如果有）
    return s.worklog.append_batch(lease, entries)                   # 一次写入 + fsync
```

- 读者只有看到新的 state.json rev 之后，才去读 report 或其它新内容（S-23）。
- **worklog 写入规则**：每次追加（run 结束时的批量写入、decide、compaction、input_rejected）之后，都要提交一次 state.json 作为确认；未确认的尾部在恢复时截掉。
- **恢复顺序**：读取新输入之前，先读取 run.json 已发布的 checkpoint，按 receipt 幂等补齐 state，再清除宿主提交门槛、重试输入确认。续用同一 run 时，即使快照领先于 state，也不能重新 render / append 其中已经存在的输入。state 已消费而所需 receipt / 快照缺失则阻塞恢复，不能倒退消费游标重跑。
- **新 run 的未提交目录**：在 state 首次引用它之前，run 必须带 `host_commit_pending`，任何执行者都不得推理。这个窗口崩溃产生的孤儿 run 只有在确认没有旧执行后才能清理；输入尚未确认，可按原稳定标识重投。
- **观察钩子和非推理输入**：观察边界注入使用同一协议，`turn` 沿用当前 Turn（`opens_turn = false`，hook 为 `observation`）、`input_seq` 递增，并在下一次推理前完成提交；worklog 只记录它的 `user_message`。control / perception 等不进入上下文的输入，由各自的持久效果或 worklog 条目与 state 消费位置建立提交关系，不伪造“已注入”receipt。
- **保留规则**：receipt 在 run 存续期间保留，历史压缩不能移除它；挂起的 process 同时保存对应 `applied_input_seq`。剪裁旧快照时必须保留最新已发布快照、待补交批次及尚未解决的工具动作所需证据。
- behavior 切换（normal / fork / independent）与 PendingTool 转 task_mgr，保留在 `classify` 之后。fork / independent 按 §4.4 的“process 与 run 的对应”处理：挂起的 process 就是一个被 `state.process_stack` 引用的 run。
- **Turn 边界**（Turn = Session 的一次逻辑 Input → result；Sub AgentSession 有自己的 Turn，父子不合并）：
  - **开启**：没有进行中的 Turn 时提交的输入批次（bootstrap 的 `on_init`、msg / event 的 `on_wakeup`）。
  - **延续**：普通 behavior 切换、fork 调用与子 process 返回、independent 切换（都是 `on_behavior_switch` 批次）、观察注入、可恢复挂起（未要求 stop 的 Interrupted、可重试的 Runtime / 瞬时错误、上下文上限、PendingTool）、history epoch 重写、重启与崩溃恢复。Turn 进行中消费的输入并入当前 Turn（补充输入）。
  - **关闭**：只由 session 在 `finish_run` 中、与 run 结束同一提交写 `turn_ended`：交付结果的 Done（最终回答 / report，随后按 `end_condition` 判断）→ `completed`；`WAIT_USER_MSG` 只有已经交付回复（本 run 有 `<report>`，或最后一步有 `<sendmsg>`）时才 `completed`，否则 Turn 保持打开，下一条输入并入；不可重试的错误 → `failed`；预算耗尽 → `budget_exhausted`；`control(stop)` → `stopped`。fork 子 process 结束（`process_done`）不关闭 Turn。
- **统计口径**：`static.json` 的 `rounds` 是本 runner 经该 run 的 `LlmClient::infer` 发起的推理尝试数（含失败与中断，另列 `rounds_failed` / `rounds_interrupted`），每个 Outcome 之后累加；历史摘要的推理不算 Round；`turns` 是以 completed 关闭的 Turn 数，`runs` 是结束的 run 数。xllm 接手 run 之后的推理只计入该 run 的 `run.json usage.llm_requests`（各执行者累加、不覆盖）。report.md 写 `- turns: N`。
- `BudgetExhausted` 不带快照，`handle_context_outcome` 用 `ctx.snapshot()`；`Interrupted` 在 behavior 模式下给出外层快照，Step 内尚未沉淀为 `StepRecord` 的原生工具 Loop 消息保存在 inner transcript 中，恢复时继续内层 Loop（X4 / X7 已实现）。

### 8.4 观察边界的变化注入（S-16、A-08）

```python
async def check_changes(s, inputs, agent, budget) -> Changes:
    out = []
    for sub in s.config.subscriptions:
        if sub.source.type == "session":                               # 拉模式：比对登记表 status
            e = agent.sessions.lookup(sub.source.ref); cur = s.state.subscription_cursors.get(sub.id)
            if e and e.status.rev > cur.rev and any(e.status[f] != cur[f] for f in sub.watch):
                out.append(Change.session(sub, e.status, text=one_line(e.status)))   # “任务 X 已完成：<report_brief>”（S-17）
    out += coalesce_by_key(inputs.peek("change"))                      # 推模式：队列中的 change 输入，读取时合并
    return prioritize_and_trim(out, budget, keep_terminal=True)        # 终态优先；超出预算的写入 worklog dropped 并注明原因
```

注入点有两类：

- **每个 hook point**：wakeup、behavior switch，现有机制已经支持。
- **每个 do-action 之后**：
  - Behavior 模式用现有的 `StepResultHook::on_behavior_step_ob`（异步，可注入；它的错误目前被忽略，快照不含当前 step，且在终态 step 上不调用）。
  - Agent（传统）模式需要 waist 新增一个可选钩子（§8.7 X5）：

```rust
// llm_context 新增（可选；未设置时行为不变）：一批工具结果都追加完、下一次推理之前调用
#[async_trait] pub trait ObservationHook: Send + Sync {
    async fn after_observations(&self, snap: &LLMContextSnapshot) -> Result<ObservationBatch, String>;   // 注入内容 + input_receipts；空批次不注入
}
```

run 进行中到达的 change，由钩子在 do-action 之间注入；消息与结构化 receipt 同时 checkpoint，再提交 state 消费位置与订阅游标，之后才能继续推理（§8.3）。钩子只负责组装注入材料；持久化或提交失败必须向外传播并暂停执行，不能沿用“忽略钩子错误后继续”的路径。

观察边界同时做两件事：

- **应用 control 输入**：`stop` 在一次 do-action 之后生效，而不是等整个 run 结束；`activity` 声明在这里合并。
- **活动 session 集合的变化**：新出现、结束、或与本 session 的 touching 有交集的 session，以 change 注入（§6.7）。

### 8.5 工具调用适配层（effect 层）

```python
class SessionToolManager(llm_context.ToolManager):          # 移植并改造 opendan 的 OpendanToolAdapter
    async def call_tool(self, call):
        tool = self.tools.get(call.name); eff = tool.effect()   # read_only | idempotent | side_effect | unknown（ToolSpec 新增字段，§8.7 X6）
        fenced(self.lease, lambda: None)
        self.run.require_execution_admitted()                # 宿主输入已提交、旧执行已核对；xllm 也必须遵守
        ticket = await self.runtime.prepare_execution(call) if tool.launches_process() else None   # 用户命令尚未放行
        self.run.register_attempt(call, eff, ticket, fsync=True)  # 先持久化：非只读调用的 inflight + 所有受管进程的执行标识
        if eff != "read_only":
            track_touching(self.session, call)                    # 尽力推断写入目标，更新 activity（§6.7）
        try:
            obs = await tool.run(call, runtime=self.env, execution=ticket)   # 记录成功后才放行；exec_bash 的 cwd = binding.workdir
        except DispatchError:
            raise ToolDispatchError(call.id, effect_unknown=(eff != "read_only"))   # 由 waist 生成 Unresolved；ToolManager 直接返回 Unresolved 会被判为内部错误
        return obs                                                # 此处不清除 inflight；异常 / 取消路径也不清除

def checkpoint_with_results(run, snapshot, status):        # 步边界、推理前与 outcome checkpoint 共用
    idx = run.write_snapshot_and_fsync(snapshot)            # ① 包含 action_result 或 Unresolved、call_id 与 input_receipts
    rec = run.record.copy()
    rec.latest_snapshot_idx = idx
    rec.status = status
    rec.inflight.remove_all(snapshot.persisted_action_outcome_ids())   # 只清除本快照已经覆盖的动作
    run.atomic_replace_record_and_fsync(rec)                # ② 发布快照指针与清除标记是同一次原子写入；失败就停止推进
```

- **恢复证据**：`inflight` 在动作开始前 fsync；只有相应结果（包括明确的 Unresolved）随快照持久化，才能清除。写完快照但未发布 run.json 就崩溃时，仍按旧指针与 inflight 保守恢复；发布成功则读取新快照里的结果。磁盘满、异常、取消、runner 退出不能提前清除标记。
- **结果未知与执行存活分开处理**：`inflight` 按 `call_id` 记录工具名、参数、effect、幂等键以及关联的执行标识。RunRecord 另保留未确认停止的 `executions`；即使清除了 inflight，也不能删除仍有后台进程的执行记录。对旧动作的核对、停止与 Unresolved 注入按 §5.2、§8.6 执行。
- session-aware 工具（`create_worksession`、`forward_msg`、`try_create_worksession`、`update_session_topic`、`read_session_history` …）改为调用 `AgentStateClient` 与输入通道 API，不再持有 `Weak<AIAgent>`。能做成 CLI（经 exec_bash）的优先做成 CLI，让 xllm 接手的 run 也能使用（§4.4）。
- 意图分析（S-09）继续使用 fork 原语；它是一个逻辑分支，不是持久化的 session。

### 8.6 崩溃恢复：复用 llm_context（Q5）

恢复遵循 [LLM Context 设计 §9](<LLM Context 设计.md>) 的纪律：

- checkpoint 写入 `runs/<run_id>/snapshots/`，run.json 沿用 xllm 的 RunRecord。
- `state.live_run` 指向的 run 就是未结束的 llm_context。
- 持 run 锁校验格式、快照与执行记录，确认旧工具已停止，按 receipt 补交输入消费状态；这些检查在读取新输入、应用 control 与开始推理之前完成。
- 如果这个 run 已经到达终态（说明结束阶段中途崩溃），仍须先通过上述检查，再由 `reconcile_runs` 重做结束流程（§4.4）。

| checkpoint 时机 | 来源 | 覆盖 |
|---|---|---|
| run 启动 / 续用与观察输入注入 | L3（`commit_input_batch` 与观察钩子） | 消息和 receipt 一起持久化；宿主提交门槛阻止提前执行 |
| 每次推理前 | 计划为 `TurnHook::before_inference` 增加异步支持（X4）；实现为异步 `CheckpointHook::before_inference`，同步的 `InferenceHook`（原 `TurnHook`）仍在每个 Round 前调用、失败阻止推理 | Agent（function call）模式：上一批工具结果之后；按 §8.5 发布结果 |
| **每个 step 的 do-action 之后** | **§8.7 X4**：Behavior 模式的步边界 checkpoint（sediment 之后、包含该 step 的外层快照） | Behavior 模式：一次 do-action 之后 |
| outcome 边界 | L3（`handle_context_outcome`） | 挂起态 / 终态；结果先持久化再清除 inflight |

```python
def resume_live_run(s, lease, deps, env) -> LLMContext | None:
    lr = s.state.live_run                                      # reconcile_runs 之后，这里只可能是未到终态的 run
    if lr is None: return None                                 # 没有进行中的 llm_context：下一个 run 按 §4.4 构成
    run = lr.run_id
    s.runs.ensure_locked(run)                                 # 复用本次 drive 已持有的 run 锁；否则尝试获取，失败 → RunBusy
    cp, record = s.runs.load_checked(run)                      # 版本不支持 / 损坏 / 缺失 → RecoveryBlocked；不改写或丢弃 run
    ensure_previous_execution_stopped(record, deps.runtime)   # 存活则终止并等待，无法确认停止则阻塞；xllm 同样执行
    reconcile_input_receipts(lease, s, cp, record)              # 已在快照中的输入只补元数据，不再追加正文
    confirm_committed_inputs(s)
    unresolved = record.inflight.without_outcome_in(cp)        # 已有持久结果的调用不能再次标成结果未知
    cp = materialize_unresolved_once(cp, unresolved)           # 按 call_id 注入，保留工具参数、幂等键；不重新调用工具
    fill = (ResumeFill.ToolResults(collect_task_results(s)) if cp.state.pending_tool_calls
            else ResumeFill.ResumeFromMidRun)                  # 挂起态 / 运行中：按 llm_context 纪律选择
    ctx = LLMContext.resume(cp, fill, deps.llm_deps(s, lease, env))
    # 首次继续推理前，checkpoint_with_results 必须先持久化上述 Unresolved；此时才清除其 inflight
    return ctx
```

- UI session 与 work session 使用同一套逻辑。
- 按 llm_context 的纪律，最后一个 checkpoint 之后的 inference 可能被重跑；工具幂等属于 effect 层，本层通过在途标记和 idempotency key 处理。
- **恢复失败保留现场**：不支持的 run / 快照版本、损坏或缺失的快照、输入 receipt 不一致、旧工具无法确认停止，都返回 `RecoveryBlocked { reason, run_id, refs }`。可在 session.last_error 与登记表回报错误，但保留 live_run、run 目录、已提交消费位置及执行证据；不自动删除、不清空 live_run、不从旧 worklog 新建 context。CLI 显示原因，后续用支持该格式的 runner、修复原快照或显式人工处置后再重试。本期不提供自动 abandon 降级；这不要求兼容旧格式。
- **xllm 的恢复边界**：同样校验格式、旧执行与结果提交。`host_commit_pending` 非空时返回需宿主补交的错误，不能越过门槛执行；已完成宿主提交的 run 保留 input_receipts 原样续跑。xllm 不修改 session.last_error 或 state.json。

### 8.7 LM Context / xllm 改进（V2，本期重点）

`runs/` 直接采用 xllm 的 run 目录后，run 目录与快照就是 session 协议的一部分。下列改进都在 `llm_context` 与 `agent_tool`（xllm）中完成；它们都是可选能力，现有调用方的行为不变；TS 版以后对齐。

| # | 改进 | 现状（§1.5） | 用途 |
|---|---|---|---|
| X1 | **RunStore 公开 API**：在指定 runs_dir 中 create_run、lock、is_live、list、remove_if_safe、按需裁剪旧快照；快照先 fsync，再原子发布 run.json；读取以已发布的 snapshot 指针为准 | `create_run` 私有；没有删除；写入不 fsync | 建 run、按引用保留与安全回收；提供输入和工具结果的提交原语 |
| X2 | **宿主装配的 run**：PromptPlan 标注宿主装配，EffectiveConfig 来自 `prompt.llm_context`；保存环境校验信息；快照保留可选 `input_receipts`，RunRecord 增加 `host_commit_pending`；xllm 有未完成宿主提交时拒绝接手，否则保留 request 与 receipt 续跑 | 只支持 xllm 自己装配（`protocol_version = xllm/1`） | 接手不重装配、不丢元数据、不越过宿主输入提交门槛（§4.4、§8.3） |
| X3 | **格式版本与恢复校验**：快照与 RunRecord 带版本并检查引用完整性；不支持 / 损坏 / 缺失时保留现场并阻塞；持久化类型容忍允许的未知字段，宿主元数据必须保留；`ToolUse.args` 按规范键序序列化 | 快照没有版本；`deny_unknown_fields`；args 是 HashMap | 明确拒绝不支持的格式，不自动 abandon / 新建 context；不要求兼容旧格式 |
| X4 | **checkpoint 钩子**：增加异步支持并保留失败即停止的语义；behavior 步边界给出含 steps / 连续 call_id / input_receipts 的**外层**快照；各 checkpoint 共用 §8.5 的结果提交顺序 | `TurnHook`（2026-10-01 改名 `InferenceHook`）已能返回错误阻止推理，但同步只读；behavior 模式只见内层快照 | 最细到一次 do-action 之后恢复；结果持久化后才清除 inflight |
| X5 | **观察钩子**：function-call 模式在一批工具结果之后、下一次推理之前返回注入内容与 receipt；两种模式均传播提交错误，完成 §8.3 提交后才继续 | function-call 没有此钩子；behavior 钩子错误被忽略 | 观察边界的变化注入与 control 检查（§8.4） |
| X6 | **effect、在途记录与执行跟踪**：ToolSpec 增加 effect；RunRecord 增加按 call_id 的 inflight 与受管 executions；启动前持久化执行标识，结果 checkpoint 后清除 inflight；resume 先核对旧执行，再注入 Unresolved | 没有 effect / inflight；LocalProcessBashRunner 的 Drop 无法覆盖 runner 被 kill 的情况；xllm resume 清空 pending | S-04 / S-22；与 L2 共同补齐启动握手、后台进程跟踪和停止确认，xllm 同样执行 |
| X7 | **挂起与上下文上限**：真正产出 `ContextLimitReached`（`context_yield_threshold`）与 `PendingTool`（延迟工具）；behavior 模式的 steps 可以压缩 | 两者都不产出；opendan / xllm 的相应分支是死代码；behavior 模式的 steps 无界增长 | §4.4 的“压缩后续跑”；task_mgr 结果恢复。**2026-09-30**：waist 完成；libopendan 接入上下文上限重写，deferred 工具回填未接入 |
| X8 | **渲染可确定**：step 渲染可以不带时间戳；时间等新鲜量不进入历史段 | step 渲染带 started / ended 时间戳；xllm 的 system 段带当前时间 | §4.4 的稳定前缀 |

- **优先级**：X1、X2、X3、X4、X6 是 L3 的前置；X6 的执行跟踪与 L2 联调。X5、X7、X8 可以在 L3 期间完成。
- **文档**：同步更新 `doc/agent_tool/local_llm_context_protocol.md` 与《LLM Context 设计》。前者目前写明“TS 版不绑定 Rust 格式”；run 目录成为 session 协议的一部分后，这条要按 X3 的版本规则重新表述。

---

## 9. 状态共享与对象协议

### 9.1 StateFs 与对存储的要求

`FsAgentStateClient` 与 `SessionDir` 都建立在 `StateFs` 上。

```rust
#[async_trait]
pub trait StateFs: Send + Sync {        // LocalFs：本地路径或 DFS 挂载；如果 DFS 只提供 API 访问，另加一个实现即可
    async fn read(&self, path: &Path) -> Result<Option<Bytes>>;
    async fn read_at(&self, path: &Path, offset: u64, len: u64) -> Result<Bytes>;   // reverse_lines / read_range 依赖它
    async fn list(&self, path: &Path) -> Result<Vec<DirEntry>>;
    async fn stat(&self, path: &Path) -> Result<Option<Meta>>;
    async fn atomic_replace(&self, path: &Path, data: Bytes) -> Result<()>;
    async fn append_batch(&self, path: &Path, lines: Bytes) -> Result<u64>;
    async fn truncate_to(&self, path: &Path, len: u64) -> Result<()>;
    async fn publish_noreplace(&self, path: &Path, data: Bytes) -> Result<bool>;
    async fn publish_dir(&self, tmp: &Path, dst: &Path) -> Result<bool>;
    async fn lock_try(&self, path: &Path) -> Result<Option<FileLock>>;             // §5：长期持有的排他文件锁
}
```

| 存储原语 | 用途 | 单节点本地文件系统（当前） | DFS（上线后） |
|---|---|---|---|
| 原子替换（rename 覆盖） | state.json / session_config / summary.json / static / 登记条目 | ✔ | ✔ |
| 不覆盖发布（link / rename-noreplace） | binding、登记、session 目录发布 | ✔ | ✔ |
| 单写者追加 + fsync、截断、按偏移随机读 | worklog（批量追加、反向读）、感知 | ✔ | ✔ |
| 排他文件锁（长期持有、持有者退出即释放） | session / run / self_improve / artifact 锁（§5） | flock（同一主机上的多个容器共享 bind mount 时同样有效） | 需要 DFS 提供同一语义 |
| 访问控制 | agent_access、App 隔离 | 只有容器挂载隔离，没有文件级 ACL（§1.5） | BuckyOS 管理权限 |

DFS 的原子语义通常强于单机文件系统，所以协议按单机语义设计，放到 DFS 上同样成立（Q12）。当前还没有可挂载的 DFS（§1.5）。

### 9.2 DID Object 宿主组件（`host` feature，随 OpenDAN 改造）

> 本期不实施。现状：buckyos-base 的 `DIDObjectRequestContext` 只带地址与路径参数，不带请求头 / token，鉴权需要先改 buckyos-base；OpenDAN 目前也没有对外提供 DID Object 路由（§1.5）。

实现 buckyos-base 的 `DIDObjectServer`，由 `DIDObjectHttpServer` 暴露。它**面向 Agent 的 `read` / `xcall` 和外部访问**；runner 不通过它读写数据。`KrpcAgentStateClient` 的服务端（§6.6）与它同在 OpenDAN 中，但两者是不同的入口。

| 对象（相对 URL 根） | Trait | properties | actions | events |
|---|---|---|---|---|
| `/` Agent | — | `card`、`status` | — | — |
| `/sessions` | `index@1` | `index_schema` | `query` / `page` | `registered` / `changed{rev}` |
| `/sessions/{sid}` | `opendan-session@1` | `status`；`agent_access=full` 时另有 `state` / `report` / `worklog_tail`（反向读） | `post_input`（宿主代为投递到 kmsg，供无法访问 kmsg 的外部调用方使用） | `state_changed{rev}` |
| `/perception` | `opendan-perception@1` | `backlog` | `query` | `appended` |
| `/cognition` | `opendan-cognition@1` | — | `recall_hints` / `notebook_append` | `changed` |
| `/artifacts`、`/artifacts/{aid}` | `index@1` / `opendan-artifact@1` | `head` / `versions` | — | `changed` |

libopendan 定义 `PrincipalResolver` 与 `Authorizer` 两个 trait；OpenDAN 切换时接入 verify-hub 与 RBAC。

---

## 10. 外部 Runner 示例

### 10.1 应用驱动一个 work session（A-01，本期重点）

```ts
// 示意：Rust API 同名；TS 版将来是 buckyos-websdk 的 ts-runner。app2 的 session 目录在自己的 data 目录
const me = app2Identity                                                    // 驱动者身份 = App 身份（Q7、Q9），principal = "app:app2@alice"
const agent = FsAgentStateClient.open(jarvisAgentRoot, me)                 // 文件版：要求能看到 AgentRoot（A8）
const sd = await SessionDir.create(`${app2DataDir}/agent_sessions`, {
  agentDid, kind: "work", driver: me,
  objective: "给贪吃蛇加穿墙模式", endCondition: { type: "llm_declares_done" },
  prompt: { llmContext: { provider: { type: "buckyos" } }, systemPrompt: appPrompt, behavior: "do" },   // .llm_context 的超集
  workspace: { kind: "external", path: `${app2DataDir}/snake` }, artifactId: "snake-game",   // 长期产物放在外部 workspace
  scope: { paths: ["ws:snake/src/"] },                                     // 活动视图的初始声明（§6.7）
  runtime: { requirement: { tools: ["node"] } }, acl: { agentAccess: "full" },
  extensions: { app2: { ticket: "T-42" } }, idempotencyKey: requestId,
}, agent)                                                                  // 同时创建 kmsg 输入队列并登记
const runner = SessionRunner.create(agent, {
  who: me, runtime: NativeRuntime.local({ runtimeId: `app2:${hostDid}` }),
  llm: AiccClient.as(me), inputs: InputChannels.buckyos(), waker: KEventWaker(), tools: defaultTools() })
await runner.drive(sd, { until: "finished" })                               // acceptance = pending
// 用户在 UI 里说“接受”：UI session 向该 session 的 kmsg 队列投递 control(decide: accept)，并发布 kevent
// app2 的任一进程被唤醒后执行 drive：登记产物 head；workspace 本身由 app2 自己管理
await runner.drive(sd, { until: "idle" })
```

### 10.2 企业软件集成 Agent Chat Box：由应用驱动 UI session（Q14，后移）

> 依赖 msg-center 等服务的确认，本期不实施；保留作为目标形态（§4.5 列出了要先处理的现状问题）。

```ts
// 前提：BuckyOS 权限机制允许 erpApp 读写该 session 的 inbox（did + session_id，Q14），并可写 sent/<did>/<session>
const me = erpAppIdentity                                                  // 身份与权限由驱动进程的 appid 决定（Q16）
const agent = FsAgentStateClient.open(jarvisAgentRoot, me)
const sd = await SessionDir.create(`${erpDataDir}/agent_sessions`, {
  agentDid, kind: "ui", driver: me, routeKey: `erp:approval:${flowId}`,     // routeKey 即 inbox 的 session_id
  prompt: { systemPrompt: approvalFlowPrompt, behavior: "chat_route" },
  inbox: true,                                                             // channels.inputs 加入 msg_center 源：{ did: agentDid, session_id: routeKey }
}, agent)
const runner = SessionRunner.create(agent, {
  who: me, runtime: NativeRuntime.local(...), llm: AiccClient.as(me),
  inputs: InputChannels.buckyos(), outbound: MsgCenterOutbound(), waker: KEventWaker(), tools: erpTools() })
for (;;) await runner.drive(sd, { until: "idle" })                         // 常驻：没有输入时由 kevent / 轮询唤醒
```

- 这个 UI session 与 OpenDAN 托管的 UI session 使用同一套协议，同样登记到 Agent State 的登记表。因此 Agent 能看到它的状态摘要，它产生的感知也进入 Agent 的记忆，跨入口保持连续（S-21、S-28）。
- 如果这个 UI session 派生出 work session，由谁驱动取决于该 work session 的 `driver`：可以是 erpApp 自己，也可以交给 OpenDAN 托管。

### 10.3 并存与恢复（A-02、A-03、A-14）

```text
app2 进程 P1                session 目录 / 登记表 / kmsg          app2 进程 P2（同一 App 身份）
 │ acquire session:T ─────► lease.json flock，epoch=7
 │ fetch inputs ──────────► kmsg[119..121]
 │ commit_input_batch ────► s0（含 receipt、开启 Turn 3，持 run 锁，host_commit_pending）→ state.json → 清门槛 → 确认连续消费位置
 │ do-action c-3-1 ───────► run.json: inflight=c-3-1 + execution 标识 fsync → 放行工具
 ✗ crash（进程退出，session 锁与 run 锁由操作系统立即释放）
                                                             │ acquire session:T ──► epoch=8（driver 身份相同 ✓）
                                                             │ 已登记 ✓；reconcile：截断 worklog 未提交尾部
                                                             │ runtime_id 相同 ✓；持 run 锁 ✓；格式与已发布 checkpoint 校验 ✓
                                                             │ 旧工具仍活着：终止并等退出；无法确认 → RecoveryBlocked
                                                             │ 按 receipt 补交 state，再确认输入；不重复追加正文
                                                             │ 有持久结果就恢复结果；没有结果的 inflight → 注入“结果未知”，不自动重放
 其它身份（例如 app3）尝试 acquire session:T ──► NotDriver
 P2 仍持锁时，同身份的 P3 尝试 acquire ──► Busy（显示 P2 的 host / pid）

 另一种恢复：运维在同一主机上执行 xllm --resume（只持 run 锁）；宿主提交已完成且格式、环境、旧执行检查通过后把 run 跑完；
 下一次 drive 的 reconcile 发现 run 已到终态，补写 worklog 并提交（§4.4）。
```

---

## 11. 跨语言策略

V1 / V6：协议是**目录结构 + 提交顺序 + 锁语义 + 输入消息格式 + xllm 的 run 目录**；Runner 是实现。先用 Rust 实现，再反写 Spec。

| 层级 | 内容 | 是否协议 | Rust（本期） | TS（buckyos-websdk，后续） |
|---|---|---|---|---|
| T0 | Session 目录结构（§4）、锁（§5）、Agent State 文件布局（§6）、输入消息格式（§4.5）、xllm run 目录（§8.7） | 是：本计划定稿，L6 反写字段级 Spec、JSON Schema 与 fixtures | 参考实现 | 按 Spec 实现，跑同一组 fixtures |
| T1 | `AgentStateClient`、`SessionDir`、kmsg 输入 | 否（SDK 抽象） | ✔ | ts-runner 的一部分（websdk 已有 `msg_queue_client.ts`；kevent 的 publish 需要补） |
| T2 | Runner：drive、assembler、next_llm_context | 否 | ✔ | ts-runner（依赖 TS 版 LM Context） |
| T3 | Runtime | 否 | native / tmux | native |
| T4 | kRPC 服务端、DID Object 宿主 | — | 随 OpenDAN 改造 | — |

- **反写的 Spec**（L6）：`doc/opendan/protocol/` 下的 `Session Directory Protocol.md`、`Agent State Protocol.md`、`Lease Protocol.md`、`Session Input Protocol.md`，以及由 `protocol/` 的类型导出的 JSON Schema。
- **fixtures** 由 Rust 实现生成（黄金目录），每种语言都要断言“解析结果 + 下一步动作”一致。覆盖以下场景：
  - 新建 work session；
  - 位于 AgentRoot 之外的 session；
  - 运行中崩溃（含 inflight）；
  - 工具已返回但结果尚未 checkpoint、快照已写但 run.json 尚未发布、结果已发布三个窗口的恢复；异常 / 取消 / checkpoint 失败保留证据；
  - runner 被 kill 后旧工具仍存活；核对并停止后才恢复，无法核对时保持 RecoveryBlocked；
  - 续用同一 run 时，msg / event / change 的快照领先于 state，按 receipt 补交而不重复注入；
  - run / 快照版本不支持、损坏或缺失时，保留现场并阻塞；
  - run 结束阶段中途崩溃（截断未提交尾部后重做）；
  - xllm 接手后 run 已到终态；
  - xllm 遇到 host_commit_pending 时拒绝，宿主补交后才可接手；
  - 已消费但未确认的重投；
  - 选择性消费；
  - finished 之后的投递与 decide；
  - 半订阅的 rev 比对；
  - 持有者退出后同一身份接管；
  - 非驱动者被拒；
  - 两个活动 session 的 touching 有交集；
  - 带摘要与起点的大 worklog：反向读取在起点停止，读取量与总大小无关；
  - 跨 Agent / App / owner 的 sid 生成与幂等命名空间；
  - binding 已发布但工具准备中断，重试修复工具与墓碑后才推理；
  - discard 与其它 session 的 accept 并发，不覆盖新的 head、不回退到 discarded 版本；
  - 同一渲染器版本下 llm_context 的历史段字节级稳定。
- **跨语言接手同一个 run 不是目标**：session 的驱动者身份固定，通常由同一语言推进；run 目录格式以 xllm（Rust）为准。TS 版 LM Context 需要接手 Rust 建的 run 时，再按 §8.7 X3 的版本规则对齐。
- **不跨语言复刻**：Memory Graph / Notebook / attention signal 统一调用 `agent_tool` CLI。

---

## 12. 从 opendan 移植（不修改 opendan）

原则是**复制后改造**，opendan 保持原样。在 libopendan 里修复的问题如果也影响 opendan，要在 PR 中注明。

| 来源（`src/frame/opendan/src/`） | libopendan 模块 | 改造要点 |
|---|---|---|
| `session_model.rs` | `protocol::{config, state, summary, input}` | `SessionMeta` 拆分：不可变部分进 session_config，可变部分进 state.json；`pending_inputs` 改为 kmsg / msg-center 输入 + `state.inputs` 游标；去掉 `already_improved` |
| `agent_session.rs` | `runner` | worker → `drive`；`run_one_round` → `commit_input_batch` / `handle_context_outcome`；`flush_meta` / `enqueue_pending` / `persist_snapshot` → state.json 提交、输入源确认、runs；opendan 的消息压缩 → summary.json；`Weak<AIAgent>` → `AgentStateClient`；`post_outbound_*` → `OutboundSink`；`mirror_status_to_task` 不移植 |
| `round_history.rs`、`session_topic.rs` | `session::worklog`；topic 归入 state.json 的 `topic` | round_logs + 每轮文件 → 单个 `worklog.jsonl`（run 结束时批量写入、反向读取）；条目类型按 Turn / 输入批次 / Step 重新定义（§4.4） |
| `hint_recall.rs` | `state::cognition::recall` | — |
| `ai_runtime.rs` | `runner::deps` | `SessionToolManager`（结果提交后清除在途标记）；`SessionSnapshotHook` → 带 input_receipts 的 `runs/` checkpoint；`AgentPolicy`；`AiccLlmClient` 按 `who` 的身份调用 |
| `behavior_cfg.rs`、`behavior_hooks.rs`、`hook_point.rs`、`prompt_env.rs`、`i18n.rs`、`llm_context_helper.rs` | `runner::assembler` | 成为 `BehaviorAssembler`；与 `session_config.prompt`（.llm_context 语义）对齐 |
| `agent_config.rs`（AgentLayout / session class / driver 配置） | `config` | `[[channel]]` / `[dispatch]` 不移植 |
| `agent_bash.rs`、`tool_plan.rs`、`paths.rs` | `runtime::{tmux, bin_overlay, paths}` | Session Exec Bin → `<sid>/.runtime/bin`；cwd 取 `binding.workdir`；准备可重试，执行标识与停止核对和 X6 对齐 |
| `local_workspace.rs` | 只移植 workspace 的引用与解析 | workspace 版本化另行设计（Q13） |
| `buildin_tool.rs`、`worksession_tools.rs`、`attachment_*.rs`、`task_util.rs` | `runner::tools` | 改为调用 AgentState 与输入通道 API |
| `msg_center_pump.rs` 的记录翻译部分（后移） | `channel::MsgCenterInput` / `MsgCenterOutbound` | 供获准的应用直接驱动 UI session（Q14）；随 UI session 实施 |
| `agent_tool::local_llm_context`（xllm） | 复用 `RunStore` / `RunRecord` 与 `.llm_context` 配置解析 | `runs/` 就是 xllm run 目录；`session_config.prompt.llm_context` 与 `.llm_context` 同一 schema；所需改进见 §8.7 |
| 不移植：`agent.rs`、`main.rs`、`dispatch*.rs`、`command_dispatcher.rs`、pump 的路由部分、`contact.rs`、`agent_task_executor.rs`、`task_dispatch.rs`、`worklog.rs` 的存储部分 | — | 属于托管职责，见附录 A |

**现有状态在新协议中的落位**（§1.5 核对；先列 work session 相关的）：

| 现有状态 | 位置 | 落位 |
|---|---|---|
| `internal_continuation`、`bootstrap_done` 的交接 | `SessionMeta` | state.json |
| prompt 渲染时推进的“上次看到”游标（schedule / workspace / notebook） | `SessionMeta` | state.json；渲染本身无副作用（§8.1） |
| `session_profile`（AICC 模型覆盖） | `SessionMeta` | `session_config.prompt` |
| fork 模式挂起的父 process 快照 | `.meta/behavior_<entry>.snap` | 父 process 自己的 run，由 `state.process_stack` 引用（§4.4） |
| independent 模式的 process 快照（跨 run 复用） | `.meta/behavior_<entry>.snap` | 该 process 自己的 run，挂起时由 `state.process_stack` 引用，重新进入时恢复（§4.4） |
| topic 的 tag 权重、tier、`topic_log` | `session_topic.rs` | state.json 的 `topic` 只放标题与标签；权重等需要时再扩展 |
| `todos.json`（由 `todo` CLI 写入） | session 目录 | 保持为 session 目录中的普通文件；它不是状态提交点 |
| CLI 直接改写 session 状态（`commit_session_history_improved`） | 子进程 | 改为向 kmsg 投递 control，由持有者应用 |
| i18n 字典（状态行、失败提示等） | Agent 包层 | assembler 读取，不进 session_config |
| task_mgr 绑定：`AgentTaskBinding` 与反馈路径（完成 / 等待人工 / 失败 / 取消）；`mirror_status_to_task` 不移植 | `SessionMeta`、agent.rs | 随 agent.delegate（附录 A.4）；本期只保留 `task_binding` 字段 |
| 全局 SQLite worklog（观测、`one_line_status` 来源） | `/opt/buckyos/opendan/worklog.db` | 各 session 的 worklog.jsonl + 登记表；观测用途附录 A.6 |
| UI 专有（后移）：命令消息、graceful / discard interrupt 的顺序屏障与抢占、群聊（`group_id`、@ 提及保护）、typing / status_line、work session 报告直发用户 | 多处 | 随 UI session 设计 |

---

## 13. 分阶段实施（libopendan + llm_context / xllm）

每个阶段结束时，都必须满足 `cargo test -p libopendan -- --test-threads=1`、`cargo test -p llm_context`、`cargo test -p agent_tool`、`uv run buckyos-build.py` 全部通过。本期不改 opendan，Jarvis 的行为不受影响。

### L0 目录结构定稿（✔ 2026-09-29）

- **交付**：本计划 §4、§5、§6 的目录结构、提交顺序与锁语义定稿；确认 §15.1 的“目录结构”类问题。**不先写**字段级协议文档与 fixtures（改在 L6 反写，V6）。
- **验收**：§15.1 A 组 7 项已全部确认。

### L1 骨架：文件原语、文件锁、Session 目录、kmsg 输入、登记表与活动视图

- **交付**：
  - crate 骨架、`fsutil`（含 `reverse_lines` / `append_batch` / `truncate_to`）、`FileLock`（§5）、`LocalFs`；
  - `SessionDir`：config / state / worklog / summary.json / static / binding / lease；
  - `KmsgInput`（按 §4.5 的 kmsg 使用规则）+ 内存实现，支持 `commit_pop` 与选择性消费；
  - `FsAgentStateClient`：registry（register / report_state / lookup / query / post_input）与 `ActivityView`。
- **验证**：
  - **锁**：多进程争抢时 epoch 单调；持有者被 `kill -9` 后，同身份的另一进程立即可以取得锁，但是否能执行还须通过 L2 / L3 的旧工具检查；非驱动者返回 NotDriver；锁文件的 inode 始终不变；exec 出的子进程不继承锁。
  - **sid**：随机 ID 保留完整 UUID；同一 Agent / creator / key 重试得到同一 sid，不同 Agent / creator 得到不同 sid；UI 路由与 self_check 的确定性 ID 包含 Agent DID；两个 owner 下同名 Agent 不冲突。显式 sid 的全局唯一约束写入 API 契约。
  - **输入**：在 state.json 提交与确认之间崩溃时，重投会被过滤；选择性消费不丢事件；finished 后的普通输入被拒绝并写入 worklog；kevent 丢失时由轮询兜底；订阅丢失后按 acked_index 重新订阅；ack 永不回退。
  - **worklog**：未提交尾部被截断；反向读在起点停止。基准：1 GB 的 worklog 上构建上下文时，读取量只与起点之后的记录有关。
  - **位置无关**：AgentRoot 与 session 目录分别放在两个 tempdir；移动 session 目录后更新 location。
  - **活动视图**：两个 session 的 touching 有交集时，彼此在 `active()` 中可见并标出交集；心跳过期的 running 条目显示为“可能已中断”。
  - **DV**：用真实 kmsg 执行 `uv run test/run.py -p <新 case>`。

### LX llm_context / xllm 改进（与 L1、L2 并行）

- **交付**：§8.7 的 X1 ~ X8；同步更新 `local_llm_context_protocol.md` 与《LLM Context 设计》。
- **优先级**：X1、X2、X3、X4、X6 是 L3 的前置；X6 与 L2 联调；其余可以在 L3 期间完成。
- **验证**：xllm 现有测试不回退；宿主装配的 run 满足恢复检查后可由 `xllm --resume` 接手；不支持的版本 / 缺失或损坏的快照返回明确错误并保留原 run；behavior 外层快照恢复后 steps 完整、call_id 不重复；host_commit_pending 非空时拒绝接手；receipt 在续跑与压缩后保留。对工具返回后、快照 fsync 后、run.json 原子发布后三个位置注入崩溃，并覆盖异常 / 取消 / checkpoint 失败：结果或 Unresolved 始终有持久证据，不提前清除 inflight。

### L2 Runtime

- **交付**：`AgentRuntime` trait、**`NativeRuntime`（本期重点）**、`TmuxRuntime`（移植，后续用于 OpenDAN）、可幂等修复的 `.runtime/bin`、`bind_or_verify` / 环境完整性校验、§7.3 的 env；与 X6 共用执行标识、启动握手、后台进程跟踪、停止并等待确认。
- **验证**：exec_bash 在 native 与 tmux 下结果一致；runtime 不匹配、缺少工具、看不到 workspace、缺少 app_tools 时均在推理前拒绝。binding 已发布但工具 / 墓碑准备失败或被 kill 时，重试会修复环境；连续失败不推理，binding 不变。启动放行前 kill 不执行用户命令；放行后 kill 仍能定位旧进程组；shell 退出但后台子进程存活时继续跟踪；无法确认执行身份或停止状态时阻塞接管，不按裸 PID 杀进程。

### L3 Runner（work session）

- **交付**：
  - `drive`、`commit_input_batch` / `handle_context_outcome`、`finish_run`、`flush_run`、`reconcile_runs` / `reconcile_input_receipts`、`next_llm_context`（summary.json + 反向读 worklog）、`compact` / `maybe_compact`、`resume_live_run`（run 锁、格式与旧执行检查、RecoveryBlocked）、`SessionToolManager`（effect、执行准入、inflight 与结果提交）、`check_changes`（含 control 与活动 session）、`BehaviorAssembler`（含活动 session 段与避让规则）；
  - 从 `agent_session.rs` 移植 outcome、切换、压缩、fork、report 逻辑；
  - 开发 CLI：`cargo run -p libopendan --example session -- create|run|read|post|decide|active|holder`。
- **验证**：用 OpenAI 兼容的 mock LLM 覆盖：
  - A-01：work session 在独立进程中完成；
  - A-02：两个 runner 共享同一个 AgentRoot；
  - A-03：在 do-action 执行中 kill，接管方先确认旧工具停止，再从 checkpoint 恢复；未持久化结果的动作以“结果未知”注入，已有结果的不重复调用。特别覆盖工具已返回但 checkpoint 未提交的窗口；run 结束阶段崩溃后正确重做且不重复写入 worklog；**xllm 接手**遵循同样的检查，跑完后下一次 drive 正确补写 worklog；
  - **恢复阻塞**：不支持的版本、缺失 / 损坏的快照、旧工具无法核对时，libopendan 与 xllm 均不启动推理或工具；live_run、run 目录及已提交消费位置保持不变；修复后可重试；
  - **输入提交窗口**：在“快照已发布 / state 未提交”“state 已提交 / host_commit_pending 未清除”“门槛已清除 / 输入源未确认”分别 kill，覆盖新 run 和续用同一 run 的 msg / event / change；重启只补交元数据与确认，不重复追加正文、不丢输入。即使队列没有新消息，已恢复且可执行的 context 也能继续；
  - A-07 ~ A-10：半订阅不引发额外推理；执行中的变化在观察边界注入；重放不重复注入（含 run 中注入后崩溃）；终态不被淹没；
  - V5：两个 work session 修改同一个 workspace 时，mock LLM 收到的上下文中出现对方的 activity，并标出交集；
  - stop 在一次 do-action 之后生效；
  - fork / independent 切换：挂起的 run 被保留并能恢复；worklog 保持时间顺序，fork 子 run 不重复写继承的 steps；run 结束后 last_run 保留、被取代的 run 被删除；
  - 预算先于起点耗尽时，先压缩再构建，摘要与原始记录之间没有空洞；同一渲染器版本下历史段字节级稳定。

### L4 感知与认知

- **交付**：感知的 append / backlog / cursor / 补发；`perceive` 工具与 `perception` 输入并入；`Cognition` 门面与 recall 注入；`self_improve` 锁，以及“一个 self-improve session 内的 behavior 链”。本期由测试手动触发；定时与空闲检查属于 OpenDAN。接口面按《Agent Memory 认知管理需求》当前版本实现，内部 schema 不冻结（A2）。
- **验证**：seq 单调且幂等；self-improve 自己产生的感知不计入水位（防自我回声）；整理失败时游标不推进；两个 self-improve 并发时只有一个运行。

### L5 产物列表与意图定位

- **交付**：
  - 产物列表：登记（`register_outputs`）、head、`apply_decide`、discard report（包括不可撤销的副作用）；
  - `workspace_discard` 只定义接口，由 workspace 机制实现（另行设计）；
  - 意图分析用的 `sessions.query` / `artifacts.query` 工具与定位规则。
- **验证**：
  - A-05：定位到 head；有歧义时先澄清。
  - A-06：新 session 引用同一个 workspace，并以 head 为 base。
  - A-12：decide(discard) 经 kmsg 送达驱动者，逐项报告。
  - 外部 workspace 的回滚报告为 unsupported，并列出变更与副作用。
  - A 的 discard 与 B 的 accept 并发：若 head 已由 B 更新，A 不覆盖 B；回退只选择仍 accepted 的祖先，没有有效祖先时为 null；缺失 / 循环引用显式报错；所有 head 写入与有效性变更均持 artifact 锁。

### L6 反写 Spec 与 fixtures（V6）

- **交付**：`doc/opendan/protocol/` 下的 Spec（§11）、由 `protocol/` 类型导出的 JSON Schema、由 Rust 实现生成的 fixtures。
- **验收**：Spec 与实现逐字段一致；fixtures 覆盖 §11 列出的场景；可以交给 buckyos-websdk 的 ts-runner 开发。

依赖关系：L0 → L1 → L2 → L3 → {L4, L5} → L6；LX 与 L1 / L2 并行，是 L3 的前置。

**本期之后**：

- UI session 与 msg-center 输入（服务确认之后）；
- `KrpcAgentStateClient` 与 DID Object 宿主（随 OpenDAN 改造）；
- 多节点 DFS（上线后重跑 L1 / L3 的锁与恢复用例）；
- buckyos-websdk 的 ts-runner（依据 L6）；
- OpenDAN 切换（附录 A）；
- workspace 管理（另行设计）。

---

## 14. 需求覆盖矩阵

“闭环依赖”一列打 ✔ 的项：libopendan 已提供所需原语，但端到端场景要等所列依赖完成后才成立。

| 需求 | 落点 | 阶段 | 闭环依赖 |
|---|---|---|---|
| S-01 Task 充分准备 | session_config（prompt / objective / end_condition / scope / workspace）；首个输入批次（on_init）装配 | L3 | |
| S-02 UI 持续反馈 | UI session 原语 | 后移 | msg-center 确认、OpenDAN ✔ |
| S-03 执行位置与身份解耦 | 位置无关的 session 目录 + `AgentStateClient` + 登记表 | L1 | 跨容器：kRPC 或 DFS ✔ |
| S-04 创建 / 恢复 / 推进 | create / drive / runs resume（含 xllm 接手）+ 反向读 worklog + 结果提交 / inflight + RecoveryBlocked 保留现场 | L1、LX、L2、L3 | |
| S-05 / S-06 应用装配、装配与循环分离 | session_config.prompt（.llm_context 超集）+ 固定合成顺序；`SessionAssembler` | L3 | |
| S-07 执行环境明确 | runtime.requirement（含 app_tools）+ 首次绑定 + 每次推进幂等修复并验证环境 | L2 | |
| S-08 权限不由提示词授予 | driver 身份 + 文件可达性 + session 锁 + 约束段不可覆盖 + App 身份调用 LLM | L1、L3 | 授权模型 ✔（§15.1） |
| S-09 ~ S-11 意图分析、定位对象、一次任务一次工作 | fork 原语 + query 工具 + head 定位；finished 之后不 reopen | L3、L5 | UI 端 ✔ |
| S-12 产物继承 | 引用同一个 workspace + base = head | L5 | workspace ✔ |
| S-13 稳定执行 | input_policy、stop（观察边界生效） | L3 | |
| S-14 管理未完成工作 | registry.query + acceptance 维度 + pending_decision | L1、L5 | UI 端 ✔ |
| S-15 ~ S-19 订阅、去重、合并 | event / change 输入 + 登记表 rev 比对 + state.json 与快照中的消费标记 + 终态独立 key + worklog 记录丢弃 | L1、L3 | 事件桥 ✔ |
| S-20 新鲜度 | PromptEnv 渲染时求值，放在变量段 | L3 | |
| S-21 多信道 | 登记表 + agent_access + 活动视图 | L1、L3 | UI session、可见范围设计 ✔ |
| S-22 / S-23 一致性、提交与通知 | 单写者 + 文件锁 + 旧工具停止核对；结果 checkpoint 后清除 inflight；输入 receipt → state → 确认；worklog 严格只追加；全局 sid 与幂等创建；head 修改持 artifact 锁 | L1、LX、L2、L3、L5 | |
| S-24 ~ S-26 产物版本、回滚与范围 | 产物列表登记 + session 级 accept / discard + discard report（不可撤销的副作用逐项列出）；workspace 级版本与回滚另行设计 | L5 | workspace ✔ |
| S-25 并发修改（派生约束） | 活动 Session 视图 + 避让规则（提示，不是互斥） | L1、L3 | workspace ✔ |
| S-27 任务回滚与 Memory 回滚 | `task_discarded` 感知保留来源；策略待定 | L4 | |
| S-28 ~ S-30 Memory 默认机制、异步整理、上下文入口 | 自动 run_digest + 游标 + hints / changes 分块 | L4 | 定时 ✔ |
| A-01 ~ A-03 | §10.1、§10.3（含 xllm 接手） | L3 | |
| A-14 | §5、§10.3 | L1（单节点）；多节点待 DFS | DFS ✔ |
| A-04、A-11 | UI 场景 | 后移 | OpenDAN、msg-center ✔ |
| A-05、A-06、A-12 | §6.5 | L5 | |
| A-13 | 活动视图提示冲突（§6.7）；并发修改的裁决属于 workspace | L3 | workspace ✔ |
| A-07 ~ A-10 | §4.6、§8.4 | L3 | |

---

## 15. 待决问题与风险

### 15.1 仍需确认

**A. 目录结构：已全部确认（2026-09-29）**

| # | 问题 | 结论 |
|---|---|---|
| 1 | 锁文件 | `lease.json` 兼作 session 锁文件，原地改写、永不替换；不新增 `lease.lock` |
| 2 | run 目录 | `runs/<run_id>/` 完全采用 xllm 布局（含 `.lock`），libopendan 不加专用文件；本 run 消费的输入记在 `state.live_run.turns`（2026-10-01 前为 `rounds`，现按 Turn 归并），快照中的 input_receipts 用于崩溃后补交（v0.10，§8.3） |
| 3 | process 快照 | 选 a：保留最后一次 run 的 llm context 状态（`state.last_run`）；被挂起的 process 所在的 run 由 `state.process_stack` 引用、不删除（§4.4） |
| 4 | 统计文件名 | `static.json` |
| 5 | `.runtime/bin` | 接受随主机绑定 |
| 6 | 附件引用 | 相对路径或 NamedStore 对象 id 都可以，由提示词决定（NamedStore 支持 LocalLink，§4.1 规则 6） |
| 7 | activity 声明 | 创建时的 `scope` + Agent 用 CLI 投递 `control(activity)` + runner 尽力推断（§6.7） |

**B. 另行设计（不阻塞本期）**

- **“以 Agent 身份”的授权模型**（S-08）：哪个 App 可以用哪个 Agent、可用工具与预算的上限、能否写感知、能读哪些记忆；对外动作的署名与授权主体分开（消息的 `from` 是 agent DID，授权主体是 App）并留审计。它决定 `KrpcAgentStateClient` 的形态，也决定文件版能否离开“能看到 AgentRoot”的环境（A8）。
- **UI session 与 msg-center**：§4.5 列出的现状问题。
- **control 输入的认证**（依赖 kmsg D-07）与 decide 的授权规则（谁能 accept / discard）。
- **runtime 重新绑定**：`NativeRuntime` 按主机标识，主机失效后 session 无法恢复。
- **驱动者长期离线或被卸载**：移交或放弃的操作；`waiting_for.deadline_ms` 到期由谁唤醒（kmsg 不支持延时投递）。
- **保留与回收**：kmsg 队列与订阅（kmsg 不能列出队列或订阅，也不会过期）、worklog 归档、感知保留、session 删除、“忘记这次工作”。
- **感知的来源与可信度**：外部 App 驱动的 session 写入的感知怎样参与整理；记忆读取边界（文件版等于把 `memory/` 交给能看到 AgentRoot 的 runner）。
- **S-21 的可见范围**：跨用户、群、信道。
- workspace 管理（Q13）；S-27 认知回滚；self-improve 空闲条件（附录 A.5）。

外部依赖：

- 本期：llm_context / xllm 的改进（§8.7）。
- UI session 恢复实施时：msg-center 的 `Reading` 回收与投递隔离。
- 跨信任域使用前：kmsg 的 D-06 / D-07 / D-09 与删除竞态；kevent 的跨节点投递与 TS publish。

### 15.2 风险

| 风险 | 影响 | 缓解 |
|---|---|---|
| 过渡期 opendan 与 libopendan 两份代码并存 | 修复不同步 | 移植 PR 中列出受影响的 opendan 位置；尽快进入 OpenDAN 切换 |
| `agent_session.rs` 体量大（8.5k 行），同时存储语义在变 | L3 周期长 | 先原样复制、编译通过，再逐类替换；把现有 56 个 session 单测一并移植作为回归集 |
| xllm 接手依赖 LX 改进与 runtime 执行跟踪 | 改进未齐时不能宣称可安全接手 | LX 与 L1 / L2 并行，X1 / X2 / X3 / X4 / X6 列为 L3 前置；宿主提交、环境与旧执行检查都通过后才接手 |
| 文件版 `AgentStateClient` 需要能看到 AgentRoot（A8） | 其它容器中的 App 暂时无法驱动 session | 本期验证在能看到 AgentRoot 的环境中进行；正式形态依赖 kRPC 实现或 DFS |
| 卡死的持有者不会自动释放锁 | 该 session 无法推进 | 锁文件记录 host / pid，开发 CLI 显示持有者，由运维终止进程；不提供强制抢锁 |
| 锁 fd 被子进程继承 | runner 退出后锁仍被后台进程持有 | 锁 fd 一律 CLOEXEC；L1 用例覆盖 |
| runner 被 kill 后工具子进程仍存活 | 新持有者与旧工具同时修改产物 | 启动前持久化执行标识；接管前终止并等待旧执行；无法确认则 RecoveryBlocked；CLOEXEC 不替代执行跟踪 |
| 工具返回与结果 checkpoint 之间崩溃 | 已产生副作用但恢复方没有结果证据 | 结果先 fsync，再原子发布快照指针并清除对应 inflight；异常 / 取消不提前清除；LX / L3 注入故障验证 |
| 续用 run 的快照领先于 state | 已注入输入被重投后再次追加 | 消息与 receipt 同快照；host_commit_pending 阻止提前执行；恢复先补交 state 再 fetch；xllm 不越过门槛 |
| binding 已发布但环境准备不完整 | 重试跳过工具或墓碑，进入错误执行环境 | 每次幂等修复并核验；失败不推理、不更换 binding；L2 覆盖准备中途崩溃 |
| discard 与其它 session 的 accept 并发 | head 回退覆盖新接受版本，或指向已失效祖先 | 所有 head 移动持 artifact 锁，在锁内重读并验证回退目标；L5 并发测试 |
| worklog 会变得很大 | 读取变慢、占用磁盘 | 只反向读、读到起点即停，读取量受上下文预算约束；全量读取只用于审计 / 导出；归档策略另议 |
| run 进行中的历史只在 `runs/` | 快照缺失 / 损坏时无法证明已确认输入与动作结果；格式不支持时当前实现无法读取 | checkpoint 与 run.json 写入 fsync；返回 RecoveryBlocked，保留 run、live_run、消费位置与执行证据；修复或更换支持的 runner 后重试，不自动从旧 worklog 重跑 |
| run 结束时 worklog 批量写入量大 | 单次 IO 峰值 | 一次顺序追加 + 一次 fsync；机械压缩配置可以限制单个动作结果落盘的长度 |
| 保留 last_run 与挂起的 run 占用磁盘 | 每个 session 多保留一到数个 run 的快照 | xllm 的快照是逐个追加的，按需裁剪旧快照（§8.7 X1），只留最新几份 |
| kmsg 的 ack 是累积的，永远不被消费的输入会卡住游标 | 输入窗口被堵死 | 持有者把不处理的输入显式标记为已消费并写入 worklog；`consumed_above` 设上限并告警 |
| kmsg 权限校验、retention、游标持久化未实现（D-06、D-07、D-09）；create / subscribe 不幂等；`commit_ack` 可回退；`delete_message_before` 有竞态 | 越权投递；队列无限增长；重启后丢订阅 | 按 §4.5 的使用规则；本期不删除消息；补齐 D-07 作为跨信任域使用的前置 |
| kevent 跨节点未接线 | 投递方与驱动者不在同一节点时唤醒延迟 | 轮询兜底 |
| worklog 渲染的确定性 | KV cache 失效、重建结果漂移 | 确定性只在同一渲染器版本内承诺；时间等放在变量段；fixtures 验证 |
| llm_context 改进需要 TS 版以后对齐 | 跨语言实现漂移 | 改进都是可选能力；TS 版在 L6 之后按版本规则对齐 |
| decide 必须由驱动者执行 | 驱动者离线时，决定会一直挂起 | 登记表的 `pending_decision` 让 UI 显示“等待 app2 处理”；该驱动者身份下的任一进程都可以处理 |
| 活动视图是尽力而为的提示 | 声明不完整时仍可能并发修改同一对象 | 创建时声明 scope；冲突裁决属于 workspace |

---

## 附录 A：OpenDAN 后续如何基于 libOpenDAN（本期不实施）

### A.1 启动

```python
async def opendan_main(args):
    rt = await init_buckyos_api_runtime(appid, owner, AppService); await rt.login()      # 沿用
    spec = await resolve_agent_spec(args.agent_did or bound_to(rt.app_instance_id))      # AgentDID 配置：system-config users/{owner}/agents/*
    root = agent_root(appid, owner, spec.agent_id)                                       # DFS 路径
    await sync_agent_rootfs_from_package(package_root(args), root)                       # agent pkg；保留本地修改（沿用）
    cfg = AgentConfig.load(root, overrides=spec)
    me = rt.app_principal()                                                              # 托管 session 的驱动者身份：OpenDAN 进程的 appid，即该 Agent 的 runtime app（Q16、A9）
    agent = libopendan.FsAgentStateClient.open(root, me)                                 # 锁是文件锁（§5）
    runtimes = RuntimeManager.from_config(cfg.runtimes)      # 默认：paios 容器内的 TmuxRuntime（Linux，内置 git）
    await runtimes.start_all()
    http = HttpServer(port=BUCKYOS_SERVICE_PORT or OPENDAN_SERVICE_PORT)                  # 与现有 Dispatch Runner 共用（4060）
    http.mount(f"/opendan/agents/{spec.agent_id}", libopendan.host.StateObjectServer(agent, auth=VerifyHubAuth(rt)))  # cyfs-gateway 路由（Q6）；需先让 DIDObjectRequestContext 带鉴权信息
    http.mount(AGENT_STATE_KAPI, libopendan.krpc.AgentStateService(agent, auth=VerifyHubAuth(rt)))   # KrpcAgentStateClient 的服务端（V3，§6.6）
    http.mount(RUNNER_KAPI_PATH, TaskRunnerHandler(...))     # agent.delegate（沿用）
    sup = Supervisor(cfg, agent, runtimes, who=me, notifier=OpendanNotifier(rt), outbound=libopendan.MsgCenterOutbound(rt))
    await gather(http.serve(), sup.run(), until=shutdown_signal())
```

### A.2 Supervisor：协程托管，无 awake queue

```python
class Supervisor:
    async def run(self):
        for e in agent.sessions.query(driver=me, run_state_not="finished") + agent.sessions.query(driver=me, pending_decision=True):
            self.ensure_task(e.sid)                           # 修复现状：重启后遗留的输入要等被触碰才恢复
        kevent.subscribe(f"/opendan/{agent_id}/session/*/input", on_event=lambda ev: self.ensure_task(ev.sid))   # 唤醒
        await gather(self.ui_session_discovery(), self.event_bridge(), self.self_improve_ticker(),
                     self.self_check_timer(), self.hosted_work_watcher(), self.poll_fallback())

    def ensure_task(self, sid):                               # 每个 session 一个协程；全部并行，没有全局队列
        if sid in self.tasks and not self.tasks[sid].done(): self.wake[sid].notify(); return
        self.wake[sid] = Notify(); self.tasks[sid] = spawn(self.session_loop(sid))

    async def session_loop(self, sid):
        sd = agent.sessions.open(sid); deps = self.deps_for(sd)
        while not self.shutdown:
            r = await libopendan.drive(sd, deps, until=StopWhen.idle)
            if r.not_driver or r.bind_failed or r.unregistered or r.recovery_blocked: return   # 恢复阻塞已回报，等待修复 / 显式处置后再调度
            if r.finished and not sd.has_pending_input(): return
            if r.busy or r.lost:                              # 锁被别的进程持有，或 run 正被 xllm 接手：等唤醒或重试间隔
                await wait_any(self.wake[sid].wait(), timeout=BUSY_RETRY); continue
            fired = await wait_any(self.wake[sid].wait(), timeout=self.idle_unload(sid))   # UI 15 分钟，其它 3 分钟（沿用）
            if not fired and not sd.has_pending_input(): return      # 卸载协程；下次 ensure_task 再起
```

### A.3 基于 msg-tunnel 的 UI session（发现与托管）

```python
async def ui_session_discovery(self):                         # OpenDAN 有权读取 Agent 的各类 inbox（Q14）
    async for ib in list_mailboxes_all_kinds(owner=agent_did):     # 显式枚举有未读消息的 (did, session_id) inbox（不用裸 DID 的 get_next 隐式扫描）；
                                                              #   现有 API 是 list_mailboxes(owner, box_kind)，按 INBOX / GROUP_INBOX / REQUEST_BOX 分别枚举，
                                                              #   只返回地址（没有 is_group / peer / tunnel，需要读一条记录获取）；kevent 加速 + 1s sweep
        sid = derive_ui_sid(agent_did, ib.session_id)           # §4.2：Agent Session 的全局 sid；inbox 的 session_id 作为 route_key
        if registry_has(sid) and registry_driver(sid) != me:
            continue                                          # 该 inbox 已由获准的应用驱动（§10.2），OpenDAN 不接管
        cls = cfg.dispatch.route("msg.group" if ib.is_group else "msg.chat")
        sd = ensure_ui_session(root / "sessions", sid, cls, driver=me, route_key=ib.session_id,
                               inbox=(agent_did, ib.session_id), peer=ib.peer, tunnel=ib.tunnel)   # inbox 标识沿用 msg-tunnel；Agent Session 按全局 sid 登记
        self.ensure_task(sd.id)                               # session 协程用 MsgCenterInput 直接消费自己的 inbox，由 commit_pop 标记 Read
```

MsgCenter 已按现有 session_id 划分 `$did/session_id` inbox，各 UI session 可直接用 `MsgCenterInput`。当 OpenDAN 需要重新决定会话归属时，由总路由器枚举待消费 inbox 再转投；不能用裸 DID 隐式扫描全部会话。

- 常规路径不再逐条转投到 kmsg。“投递成功后才 ack”的不变量由 §4.5 的 `commit_pop`（先提交 state.json 再标记 Read）保证；前提是 msg-center 解决 `Reading` 无租约的问题（§4.5）。
- 命令消息（原 `command_dispatcher`）改在 OpenDAN 注入的 session 输入预处理中执行，不进入推理，处理后同样标记为已消费。

### A.4 决策与托管 work session

- **UI 替用户做决定**：向目标 work session 的 kmsg 队列投递 `control(decide)`。驱动者是 OpenDAN 时，由 supervisor 被唤醒后执行；驱动者是应用时，由应用执行，此时登记表的 `pending_decision` 让 UI 显示等待状态。
- **应用委托托管**：应用创建 `driver = OpenDAN 的 appid` 的 work session，由 OpenDAN 用默认 runtime 执行。
- **agent.delegate**：改为 `create_session(kind=work, driver=opendan, task_binding)`；状态镜像通过 Notifier 完成。

### A.5 定期 self-improve：启动条件检查（Q1）

```python
async def self_improve_ticker(self):
    every(60s):
        c = cfg.session_class("self_improve")
        if c.enabled and self_improve_start_conditions(c):
            b = agent.perception.backlog(agent.perception.cursor())
            sd = SessionDir.create(root / "sessions", SessionSpec(kind="self_improve", driver=me,
                      objective="consolidate perceptions", prompt={"behavior": c.default_behavior},
                      extensions={"opendan": {"perception_window": b.window()}},
                      idempotency_key=f"si:{b.window_digest()}"), agent, me)
            self.ensure_task(sd.id)

def self_improve_start_conditions(c) -> bool:              # 启动条件检查，条件可配置
    return (agent_idle(c.idle_for)                         # ① 至少要空闲：没有 run_state=running 的其它 session，且 idle_for（默认 10 分钟）内没有新输入
            and not agent.leases.is_held("self_improve")   # ② 没有其它整理在运行（任意进程）
            and watermark_reached(c))                      # ③ 感知水位 ≥ threshold，或到达每日时点（沿用 20 / 03:00）
```

### A.6 其它保留职责

- **self-check**：timer 事件以 `event` 类型投递到该 Agent 的 singleton self_check session，sid 按 §4.2 的全局规则生成。
- **事件桥**：按订阅把外部对象事件投递为 `event`（active）或 `change`（semi）。
- **worklog SQLite**（观测用）：可以逐步由各 session 的 `worklog.jsonl` 取代。

### A.7 切换时的改动清单

- **删除**：
  - `agent.rs` 中的 `sessions` / `session_locks` / `tunnel_to_ui_session` / `ensure_session_inner`；
  - 串行 `main_loop`、`restore_session_routes`；
  - `.meta/self_improve_scheduler.json`；
  - 所有已移植到 libopendan 的模块副本。
- **`agent_tool_cli_dev`**：改为依赖 libopendan，支持 `OPENDAN_SESSION_DIR` / `OPENDAN_INPUT_QUEUE`；同时修复两个现有问题：
  - `get_session` 读取的是旧路径；
  - CLI 直接改写 `.meta/session.json`，与内存中的 meta 竞争。
- **`agent-did-object-lib::AgentRuntimeAdapter`**（目前是桩）：实现 `agent://self/...` → `{base}/opendan/agents/{aid}/...`。
- **清理旧数据**：执行 `remove_open_dan_data.sh`。
- **DV 验证**：`uv run src/check.py`、`./debug_jarvis.sh`，再用新增的 DV case（`uv run test/run.py -p <case>`）覆盖 A-04、A-11。
