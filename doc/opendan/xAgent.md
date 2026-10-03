# xAgent：Agent Session 分层验证工具设计

- 状态：草案 v0.2（2026-10-02，v0.2 增加 §3.2–§3.7 Context 调度、§4.10–§4.16 Sub Session），待 Review 后开始实施
- 位置：`src/frame/lib_opendan`（二进制 `xagent`，替代 `examples/session.rs`）
- 依据：[Agent Session SDK 实现计划](<./Agent Session SDK 实现计划.md>) v0.10、[长任务与执行体 RFC](<./OpenDAN Long Task & Sub-Agent.md>)、[LLM Context readme](../llm_context/readme.md)、[xllm Rust SDK 参考](../llm_context/xllm_rust_sdk.md)、`doc/opendan/protocol/`
- 读者：决定是否按本文实施 xagent 的人；文中“现状”都对照 2026-10-02 的 `libopendan` / `agent_tool` 代码

---

## 0. 结论先行

xagent 的目的是**在一个新产品里验证四层架构、发现设计问题并迭代**，而不是给现有 libopendan 做回归。所以本文的伪代码（§9）写的是目标设计，与现有 `runner/drive.rs` 的差距单列在 §11；凡是实验（§10）暴露出设计问题的，先改设计再改代码，等 OpenDAN 集成之后就改不动了。

**Agent Session 与 LLMContext 的根本区别是 Context 调度**（§3.2–§3.7）：LLMContext 跑到 `next_behavior`（或一个挂起点）就返回；下一个 context 从哪份快照起、换成什么配置、结果怎么交回，全部由 Session 决定。普通切换、fork、independent、继承 context 的工具（工具子上下文）、改写、Turn 边界新建 run 是同一张转移表上的几行。继承 context 的工具不做成进程内工具，而是由工具调用触发的转移，结果经 PendingTool 交回。

**Sub Session 是 SDK 化的主要用途之一**（§4.10–§4.16）：创建子 session 是一个层 ② 标准工具；父子各写各的 state，只经登记表、输入通道与控制协议沟通。进展以登记表为真相：父可以随时拉取，也可以按创建时选的汇报方式收到事件（进度不唤醒，需要关注和结束会唤醒）；子 LLM 也能主动给父发消息。等待分异步（默认）与同步（Pending + `session` resolver）两种；父结束前会等需要汇报的子 session。

1. **xagent 是 xllm 的上一层**：xllm 加载 `.llm_context`，把一个 LLMContext run 推进到一个 Outcome；xagent 加载（或创建）一个 Agent Session，把它推进到**一个 Turn 关闭**，或常驻地不断完成 Turn。两者的 run 目录相同（`runs/` 就是 xllm 的 run 目录），同一个 run 可以在两者之间交接，这是验证 L2 / L3 边界的主要手段。
2. **Agent Session 构造 llm_context 复用 xllm 的宿主装配 API**（`XllmTask::prepare_hosted` / `hosted_request` / `hosted_waist_deps` / `rebuild_toolset` / `create_run_llm` / `RunStore`），但 **system 段、历史段、输入批次、工具调度包装、checkpoint 钩子、run 生命周期都由 Session 决定**；差异清单见 §3。
3. **Agent 感知到的输入只有两种：`AgentMessage` 与 `AgentEvent`**（§4）。Session 不关心它们怎么来的，只要求信封（key、来源与 index、from、at_ms、subscription_id）；把系统事件（msg-center、kevent、timer、task_mgr、子 session）翻译成这两种输入的是上层 **bridge**（xagent serve、以后的 OpenDAN Supervisor、应用）。事件是唤醒还是只在观察边界注入，由 **Session 按自己的订阅配置决定**，不由 producer 决定；半订阅事件按 key 保留在 state.json，空闲时不丢。stop / decide / subscribe / activity / perceive 不是 Agent 输入，是**Session 控制协议**，只是搭同一条队列。
4. **Session 模板**（§4.7）决定一个 session 的形态：Turn 上限、`WAIT_USER_MSG` 的含义、要不要输入队列、观察注入开关、hints、默认 behavior。work 模板 = 一个 Turn、不等用户、默认不建队列；ui 模板 = 无限 Turn、有队列。模板是 `SessionSpec` 的预设，创建时解析进 session_config，不是新协议对象。
5. **Runtime 接管全部 agent-tool**（§5）：共享 AgentRuntime/Sandbox 已在 agent_tool 实现 native/tmux/remote_ssh、文件后端、环境与执行跟踪。Session 保留 lease、门槛、inflight、receipt、bin/helper 和绑定。ActionGuard、grant 与审批为后续 policy 设计，不属于已完成首版。
6. **behavior 配置来自 Agent State，在 Session 构造时冻结进 `session_config.prompt`**（§6）；切换模式由**目标 behavior** 的冻结配置决定。
7. **Agent State 不配置**（§7）：`AgentStateClient::connect(agent_did, who)` 按"进程内 → 本机 AgentRoot → kRPC"解析；Runner 只依赖 trait。
8. **验收项**是 E1（xllm 接手）、E4（换 Agent State 实现）、E5（工具形态）、E13（无队列 work session）、E14（半订阅注入时机）、E17（Runtime 沙箱接管全部工具）、E18（Do 前检查与授权）、E19（普通切换换配置、切换点交接）、E20（工具子上下文）、E21（Sub Session 派出与汇总），其余实验是回归（§10）。

---

## 1. 目标、范围与要验证的分层

### 1.1 目标

用一个独立进程（不依赖 OpenDAN 服务）把下面四层各自的边界跑通，并能用实验证明边界成立（§10）：

| 层 | 组件 | 回答的问题 | xagent 要证明的事 |
|---|---|---|---|
| LLM Context | `llm_context` + xllm（`agent_tool::xllm`） | 一次上下文推理循环：Round / Step、工具、快照、resume | Session 装配的 run 可以被 xllm 原样接手 / 跑完；Session 不重新实现 Loop |
| Agent Session | `libopendan`：目录协议 + Runner | 以某个 Agent 的身份，把一次逻辑 Input 推进到 result（Turn）；**Turn 内调度多个 LLMContext**（切换、fork、independent、工具子上下文）；**创建与协调 Sub Session** | Turn 的开启 / 并入 / 关闭只由 Session 决定；LLMContext 停在 `next_behavior`，之后的转移只由 Session 决定；三种输入形态语义一致；崩溃后恢复同一 Turn |
| Agent Runtime | `agent_tool::runtime` | 工具在哪里、以什么 PATH / cwd / 环境运行；后台进程的识别与停止 | 同一 session_config 可绑定不同 runtime；绑定后不可换；工具集与 runtime 解耦 |
| Agent State | `libopendan::state` | 跨 Session 的 Agent 状态：登记表、活动视图、感知、认知、产物、**behavior 目录** | Runner 只依赖 `AgentStateClient`；文件实现 / 进程内实现 / kRPC 桩行为一致；behavior 冻结后不受更新影响 |

### 1.2 范围

- 做：work / self_improve / self_check session；native 与 tmux runtime；文件版与进程内 Agent State；behavior 冻结与 `BehaviorAssembler`；CLI 形态 session 工具；单 Turn / 积压消费 / 常驻三种运行形态；Sub Session（创建工具、ChildDriver、汇报与等待，§4.10–§4.16）。
- 不做（沿用计划的后移项）：UI session 与 msg-center 输入、kRPC 服务端、DID Object 宿主、opendan 改造、TS 版；Sub-Agent Instance（另一个 Agent DID，RFC §16–§17）。
- 不改协议目录结构（目录项由用户定稿，§4.1 of 计划）；新增字段只落在 `session_config.prompt` 与 `state.json`，schema 版本号递增。

### 1.3 与 xllm 的对位

```text
xllm   : .llm_context ──prepare──► LLMContextRequest + Deps ──run──► Outcome      （一个 run，到停止点就结束）
xagent : AgentSession ──InputBus──► 输入批次 ──commit──► run ──Outcome──► Context 调度（§3.2）──┬─► 下一个 context（同一 Turn）
                 ▲                                                                            └─► Turn 关闭
                 └──── Agent State（behavior、身份、登记表、活动、认知）      （一个 Turn，可含多个 run 与多个 behavior 段）
```

### 1.4 验证方式：目标设计优先

- §9 的伪代码是目标设计；它与现有 `drive.rs` 的差距逐条列在 §11，实施就是消差距。
- 每个实验（§10）都写明"验证哪条边界、什么现象算设计问题"。发现设计问题时的处理顺序：改本文 → 改协议 Spec（`doc/opendan/protocol/`）→ 改代码 → 重生成 fixtures。不允许为了让实验通过往 Runner 里加特例。
- xagent 不复用 `examples/session.rs` 的命令面，它从 CLI 形态上就按目标设计来（§8），旧 CLI 删除。

---

## 2. 分层与部署形态

```text
 xagent 进程（App 身份 who = app:<appid>@<owner>）
 ┌──────────────────────────────────────────────────────────────┐
 │ xagent CLI                                                   │
 │  ├─ AgentSession（SessionDir + state.json 提交点 + lease）     │
 │  ├─ TurnLoop（= runner::drive，加 StopWhen::TurnClosed）       │
 │  ├─ InputBus（KmsgInput | DirMsgQueue | None）◄─ post / ctl / bridge │
 │  ├─ ContextFactory（BehaviorAssembler + history + xllm hosted）│
 │  ├─ AgentRuntime（native | tmux | host* | container*）         │
 │  └─ AgentStateClient（InProcess | Fs | Krpc*）                 │
 │     Runtime = Sandbox：所有工具经 dispatch（guard → grant → 执行） │
 └───────────┬──────────────┬────────────────────┬──────────────┘
             │ 文件 + flock  │ kmsg / kevent        │ 文件 / kRPC
             ▼              ▼                     ▼
   <sid>/.opendan_agent_session/   每 session 一条输入队列     AgentRoot：behaviors/ role.md self.md state/ memory/ …
   runs/ = xllm run 目录 ◄── xllm --resume 可接手
```

`*` 只定义接口与桩。xagent 自身也会被包装进 `<sid>/.runtime/bin`（名字 `agent-session`，兼容现有 exec 内用法），exec 里的子进程用它向本 session 投递 control / perception、读写认知与登记表。

---

## 3. Agent Session 与 llm_context：构造、调度，以及与 xllm 的关系

**回答：是复用 xllm.rs 的方法，但只复用“宿主装配”那一半。** xllm 自己的 `XllmTask::prepare` 负责从 `.llm_context` 文件发现 / 合并配置、选组、渲染模板、拼 system 与 user、建 run。libopendan 走的是 2026-09-29 为 Agent Session 加的宿主装配路径：

| 环节 | xllm（`XllmTask::prepare` → `XllmRun`） | Agent Session（libopendan `runner/live.rs::new_run_context_plain`） |
|---|---|---|
| 配置来源 | 从 workdir 向上发现 `.llm_context` 文件并合并 | `session_config.prompt.llm_context`（同 schema 的 JSON，严格键）。**xagent 新增**：冻结 behavior 的 `[model]/[capabilities]/[budget]/mode` 叠加成本 run 的有效 `.llm_context`（§6.4），结果写进 run.json `config`，xllm 接手时不再需要 behavior |
| 有效配置与工具展开 | `prepare` 内部 | `XllmTask::prepare_hosted(workdir, llm_context_json, origin, host_system, deps)`：xllm 算 EffectiveConfig、展开 tools / actions / MCP、在宿主 system 后追加 `capabilities` / `cmd_manual` / `runtime_protocol` 段 |
| system 段 | 按行号 section 拼装，含当前时间等新鲜量 | 宿主给出：身份（role.md / self.md）→ 不可覆盖约束（含避让规则）→ 应用 prompt → 初始 context → objective；**不含新鲜量**（S-20 稳定前缀）。xagent 由 `BehaviorAssembler` 用冻结的 behavior 模板渲染这一段 |
| 历史段 | 没有（一个 run 一次任务） | `history.rs::build_history`：先读 summary.json，再从 worklog 已提交末尾**反向**读到起点，预算不够先 compact；渲染成一条 `<session_history>` user 消息 |
| user 输入 | 任务要求 + 附件 + stdin，构造时就在 `request.input` | 不在构造时给。由 `commit_input_batch` 以 `LLMContext::inject` 注入 `<session_input hook=…>`，连同 `InputReceipt` 写进快照（HostMeta `libopendan`），并按 ①快照 ②run.json 门槛 ③state.json ④清门槛 ⑤确认输入源 的顺序提交 |
| LlmClient | `llm_factory.create(provider)` | `hosted.create_llm` / `create_run_llm`（同一 factory，以 `who` 身份），外面再包 `CountingLlm` 计 Round |
| ToolManager | `XllmToolManager`（本进程、本地 fs） | **Runtime 的 Sandbox**（按同一份有效 tools 配置解析与派发，内置文件/exec 使用执行体），外包 `SessionToolManager`：lease 检查、宿主门槛、inflight 记录、touching 推断。native 沙箱内部仍复用 xllm 的 `build_toolset` |
| waist deps | `LLMContextDeps` + xllm `SnapshotHook`（InferenceHook） | `hosted_waist_deps`（behavior 时装 `XllmActionParser` + 无时间戳 `XmlStepRenderer`）+ `SessionCheckpointHook`（异步 CheckpointHook：观察边界注入 changes、持久化工具结果、心跳、stop 中断）。**不用 InferenceHook** |
| run 目录 | `RunStore` 在 `.llm_context` 的 runs_dir | 同一 `RunStore`，目录是 `<sid>/.opendan_agent_session/runs/`；RunRecord 多了 `host{assembled_by, session_id, runtime_kind, env_check}`、`host_commit_pending`、`inflight`、`executions`；`host.extra.finish` 存结束决定 |
| 工作目录锁 | `<lock_dir>/<hash(workdir)>.lock` | `skip_workdir_lock = true`，多 session 共享 workspace 靠活动视图避让 |
| run 结束 | 终态写 run.json，结果导出 | `handle_context_outcome` → `finish_run`：flush 历史进 worklog、关闭 Turn、提交 state.json、登记表回报、感知 digest |
| 停止点之后 | xllm 结束；`next_behavior` 只留在快照与 run.json 里 | **Context 调度**（§3.2）：同一 run 换段、建子 run、回到父 run，或关闭 Turn |
| 接手 | — | 保存的 runtime/target/cwd、helper 环境核验通过，且无 `host_commit_pending`、无进程内专有工具时，`xllm --resume --run <id> --runs-dir <sid>/.opendan_agent_session/runs --dir <workdir>` 可接手 |

xagent 不改变这张表的分工，只在“配置来源”和“system 段”两格引入 behavior 冻结（§6），在“ToolManager”一格把执行权交给 Runtime 沙箱（§5）。这张表只讲一个 run 怎么构造；run 停下之后下一个 context 怎么来，是 §3.2–§3.7 的 Context 调度。

### 3.1 已知的边界缺口（xagent 要用实验暴露，实施时一并修）

| # | 现状 | 影响 | 处理 |
|---|---|---|---|
| G1 | `prepare_hosted` 只用 `llm_context` JSON 的 provider / model / limits / tools / `loop_model`，`prompt.sections`、`prompt.system`、groups 的 section、`runs_dir` 被解析但忽略 | behavior 叠加只能落在这几个键上，正好够用；但 `prompt.llm_context` 里写了 sections 会静默无效 | xagent 在冻结时校验并报错；§6.4 的 overlay 只产生这几个键 |
| G2 | `hosted_waist_deps` 用无时间戳 `XmlStepRenderer`，而 `xllm --resume` 接手后重建 deps 时用带时间戳的渲染器 | 接手后历史渲染字节变化，只影响前缀缓存，不影响正确性 | run.json `host` 里记 `renderer_opts`，xllm resume 时沿用（E1 实验会观察到） |
| G3 | `build_runtime_protocol` 的开场白是“你在 xllm 一次性任务里运行，没有后续对话，不要向用户提问”，hosted 也原样追加 | 与 Session 的 `WAIT_USER_MSG` 语义冲突 | `prepare_hosted` 增加 `HostProtocolFlavor::Session`，由宿主给出 runtime_protocol 的开场白 |
| G4 | xllm action 解析把“无动作 + `<report>`”映射为 `next_behavior = "done"`，不是 `END`；依赖 `forbid_next_behavior = false` | libopendan 已按“done 即交付”处理（`classify_done`），fork 子 run 不设 forbid | 保持；文档化到协议 Spec |
| G5 | Runtime 已提供 exec 请求超时、执行记录与取消停止核验 | Session 的整体 behavior deadline 仍属于后续 xagent 预算接线 | 保留 Session stop/lease 中断；整体预算经宿主接入，policy 不作为首版前置 |
| G6 | 已修：run 保存实际目标与 Session env_check | xllm 接管核验 PATH、环境、manifest 与 helper 内容 | 凭据重新读取环境引用；依赖缺失/变化则阻塞恢复 |
| G7 | 已修：prepare/prepare_hosted 调用共享 runtime.open | 内置文件工具使用目标侧后端，与 exec 共用 cwd | native/tmux/SSH 已验证；远端 Session helper 缺失时报 Capability |
| G8 | xllm 接手的 run 以 `next_behavior = B` 结束时记 `Completed`；libopendan `reconcile.rs` 重建决定时把切换丢掉（"The executor ended the run; nothing continues it"），按 Turn 结束处理 | 切换点不能交接：xllm 跑到切换点，Session 就丢了 B | hosted run 以跳转结束时，xllm 记非终态（`Paused`，并记下 `next_behavior`），由 Session reconcile 按 §3.3 完成转移（E19）；见 §3.7 |

### 3.2 Context 调度：Agent Session 与 LLMContext 的根本区别

LLMContext 的契约只到一个停止点：`run()` 返回一个 Outcome。Behavior 模式下，LLM 用 `<next_behavior>` 声明下一步（`END`、`WAIT_USER_MSG`、xllm 的 `done`，或一个 behavior 名），`run()` 随即以 `Done` 返回。LLMContext 不知道目标 behavior 的配置，也不知道还有别的 context、Turn 和输入队列。function_call 模式没有 `next_behavior`，`Done` 就是最终回答。

所以**下一个 context 由谁跑、从哪份快照起、换成什么配置、结果怎么交回，全部由 Agent Session 决定**。本文把这部分称为 Context 调度（Context Switch），它是 Session 比 LLMContext 多出来的核心能力。约束：

- **串行**：同一 Session 任一时刻只推进一个 context（live run），其余的以 `ProcessFrame` 挂在 `state.process_stack` 上，或已结束。要并行就用子 Session（readme 的 AgentSession Tree），不在一个 Session 里同时推进两个 run。
- **Turn 延续**：除 T6（Turn 边界新建 run）外，所有转移都延续当前 Turn（§4.9）。
- **每次转移都是一次提交**：转移写进 state.json（`live_run`、`process_stack`、`current_behavior`、`internal_continuation`），worklog 记一条；崩溃后 `reconcile_runs` 能把做了一半的转移做完。
- **停止点就是交接点**：xllm 接手的 run 跑到 `next_behavior = B` 就停，转移由下一次 `xagent run` 的 reconcile 完成（现状做不到，见 G8；E19 验证）。xllm 不需要理解任何调度概念。
- **llm_context 只提供构造原语**：`rebuild_with_inherit` / `build_fresh` / `RequestOverrides`（`llm_context/src/snapshot_overrides.rs`）、`forbid_next_behavior`、`ResumeFill`；别的 behavior 的 step 在 `<<step_history>>` 里渲染为继承记录（`step_record.rs`）。用哪个原语、什么时候用，是 Session 的事。

名词：**process** = 有自己 run 目录与 step 流的一个 context，即一个 run；**frame** = 挂在栈上的 process；**段** = 同一个 run 内一个 behavior 生效的区间（普通切换只开新段，不开新 run）。

### 3.3 转移表

| # | 转移 | 触发 | 新 context 从哪来 | 换掉什么 | 计数 | run 与栈 | 返回 |
|---|---|---|---|---|---|---|---|
| T0 | 续跑 | 输入批次、观察注入、可恢复挂起之后 | 当前 context | 不换 | 保留 | 同一 run | — |
| T1 | 普通切换 | `next_behavior = B`，B 冻结的 `switch_mode = normal`，`loop_model` 相同 | 当前 `Done` 快照 | B 的 system、工具、模型、budget、objective、behavior_name；A 的热 step 降为历史 | 工具额度与连续错误**不重置** | 同一 run 开新段；run.json `config` 换成 B 的 | 不返回 |
| T2 | fork | B 的 `switch_mode = fork` | 新 run：B 的 system + `<session_history>` + 父 run 已定的 steps | B 的全部配置 | 重置 | 新 run；父 run 入栈 `Fork` | 子 run 结束 → `process_result` 交接批次注入父 run |
| T3 | independent | B 的 `switch_mode = independent`，或 B 的 `loop_model` 与当前不同（§12 第 2 项） | 栈里 B 自己挂起的 run；没有就新建 | B 的全部配置 | B 自己的 | 当前 run 入栈 `Independent`；B 有挂起的 run 就出栈成为 live | 不自动返回：回 A 要显式跳转；`END` 按 end_condition 收尾 |
| T4 | **工具子上下文**（新） | 调用一个 `sub_context` 工具 | 新 run：子 behavior 的 system + 按 `inherit` 取父 run 的已定历史 + 调用参数 | 子 behavior 的全部配置 | 重置 | 新 run；父 run 以 PendingTool 挂起，入栈 `Call{call_id}` | 子 run 结束 → 父 run 以 `ToolResults{call_id}` 恢复 |
| T5 | 改写 | `ContextLimitReached` | 同一 run，历史重写为 summary + `<session_history>` | 不换 | 保留 | 同一 run，epoch + 1 | — |
| T6 | Turn 边界 | 上一个 run 已结束，又来了输入 | 新 run：当前 behavior 的 system + `<session_history>` | — | 新 | 新 run | — |

- T1–T3 只出现在 Behavior 模式；T4 两种模式都能用，也是 function_call 模式（如 opendan 现在的 ui session）唯一的调度手段。
- T2 与 T4 的子 run 声明的任何跳转都视为返回调用方。这由 Session 在 `classify_done` 里处理，不用 `forbid_next_behavior`（原因见 G4）。
- 栈深上限 `session.policy.max_process_depth`（默认 4，沿用 opendan `process_stack_limit` 的语义）。T2 / T3 超限时不切换，当前 run 收到交接批次 `<behavior_switch to=B refused="max_process_depth"/>` 后续跑；T4 超限时工具直接返回 Error 观察。
- 返回方向：fork 子 run 结束 → `ProcessDone`；调用子 run 结束 → `CallReturn`。两者都出栈，父 run 重新成为 live，都不关闭 Turn。

### 3.4 普通切换（T1）：同一 run，换配置

现状有两处差异：libopendan（`runner/outcome.rs`）只改 `snap.request.behavior_name`，system、工具、模型都沿用 A；opendan 的 `apply_switch_normal` 则用 `RequestOverrides` 整套替换。libopendan 这样做，是因为它还没有 behavior 配置。xagent 引入冻结 behavior（§6）之后必须替换，否则切到 B 用的仍是 A 的提示词和工具。

```rust
/// T1：同一 run 开新段。
async fn switch_in_place(&mut self, lc: &mut LiveCtx, b: &str, snap: LLMContextSnapshot, f: &ContextFactory<'_>) -> Result<()> {
    self.ensure_frozen_behavior(b, f.deps).await?;
    let seg = f.prepare_segment(self, b).await?;      // 与 new_run 前半相同：overlay_llm_context(B) + system_text(B) + prepare_hosted
                                                      // （共享 runtime.open，沿用同一 runtime 目标与 cwd）→ B 的 system、ToolManager、有效配置
    if seg.config.loop_model != lc.loop_model() { return self.suspend_run(lc, b, Independent, &snap).await; }   // parser / renderer 不能共用一个 run
    let req = seg.request(self, &lc.run);             // hosted_request + B 的 budget
    let ov = RequestOverrides {
        system_messages: Some(vec![AiMessage::system(seg.system)]),
        tool_policy: Some(req.tool_policy), model_policy: Some(req.model_policy), budget: Some(req.budget),
        objective: Some(req.objective), behavior_name: Some(b.into()),
        reset_behavior_hot_tail: true,                // A 的热 step 降为 <<step_history>> 里的继承记录
        reset_tool_iterations: false, reset_errors: false,   // 不能靠切换绕过工具额度与错误上限
        ..Default::default()
    };
    lc.ctx = rebuild_with_inherit(snap, ov, f.waist_for(&seg, &lc.run))?;   // ToolManager 换成 B 的；provider 变了就重建 LlmClient
    lc.run.replace_config(&seg.config)?;              // xllm 接手时 rebuild_toolset 读的就是它
    self.worklog_append([Outcome{kind: "switch", next_behavior: b, config_digest: seg.config.digest(), ..}])?;
    self.state.current_behavior = Some(b.into()); self.state.internal_continuation = Some(b.into()); self.state.run_state = Ready;
    commit!(self)
}
```

- **B 看到什么**：A 的 steps 作为继承记录留在 `<<step_history>>` 里，再加上 `on_behavior_switch` 交接批次（readme “behavior 切换”）。历史不丢，换的是“我是谁、能用什么”。
- **工具**：每段的 ToolManager 都来自本段有效配置的 `runtime.open`（同一 runtime 目标与 cwd，执行跟踪按 run 记录，不受换段影响）；waist 按 `tool_policy` 白名单过滤工具广告与派发（`llm_context/src/deps.rs`）。
- **xllm 接手**：run.json `config` 始终是当前段的有效配置，快照里的 request 同步被覆盖，两者一致；xllm 不需要知道发生过切换。
- **代价**：system 段换了，前缀缓存从切换点开始失效。同一 behavior 内仍然稳定（E11 按段比较）。

### 3.5 fork 与 independent（T2 / T3）

沿用 libopendan 现有的 process 语义（计划 §4.4）：挂起的 process 就是一个被 `process_stack` 引用的 run；fork 子 run 的 Step 编号接续父 run，继承来的 steps 不重复写进 worklog（`inherited_below`）；independent 各 run 各自编号，重新进入时恢复同一个 run。只改三处：

- **子 run 的构造**：现在 `runner/live.rs` 先建一个普通 run，再手工复制父快照的 `steps`、`last_step`、`history_summaries` 和编号。改为调用 llm_context 的 `derive_child(parent_snap, Inherit::Steps, child_request)`（§3.7），T2 与 T4 共用。
- **子 run 的配置**：子 run 走 `new_run`，用目标 behavior 的冻结配置（system、工具、模型），不再沿用父 run 的。
- **栈深上限**：见 §3.3。

independent 的 `END` 有分歧：libopendan 现状是按 end_condition 收尾，栈里的 frame 留着等下次显式跳回；opendan 是结束后回到下层 process。本文按 libopendan，见 §12 第 12 项。

### 3.6 工具子上下文（T4）：继承 context 的工具

**需求**：有些“工具”本身就是一次推理——带着父 context 的上下文，用另一套提示词和工具做一个小决定，再把结果交回父 context。opendan 的 `try_create_worksession` 是典型：判断复用已有 worksession 还是新建，需要时再调用 `create_worksession`。

**现状（opendan）**：工具持有 `Weak<AIAgent>`，在 `execute()` 里调用 `session.fork_and_run_agent_loop`（`worksession_tools.rs`）。子 context 由磁盘上的父快照经 `rebuild_with_inherit` 构造（`llm_context_helper.rs::run_fork_sub_context`），在这次工具调用内部同步跑完，`Done` 的输出就是工具结果。子 context 不持久化（`state.snap.fork-N` 跑完即删），挂起一律当错误。它其实不继承 steps：`user_messages` 覆盖会清空 steps，只放一条“父 session 最近对话”。libopendan 没有对应实现。

**“工具里嵌一个 run”在新架构下不成立**：

1. 父 context 停在工具调用中途，子 context 同时在跑，违反串行约束；Session 的 stop、lease、观察注入、心跳都只对着父 run。
2. 子 run 不在 `runs/`、不进 worklog，崩溃后丢失，父 run 只剩一个 inflight 调用。
3. 它要拿 Runner 的内存句柄，只能是层 ③，不经 Runtime 统一入口；父 run 也永远不能被 xllm 接手。

**目标：做成由工具调用触发的转移，结果走已有的 PendingTool 路径交回。**

```rust
/// 冻结在 behavior 里（§6.2 BehaviorConfig.sub_contexts）；对 LLM 就是一个普通工具（function 或 action）。
pub struct SubContextSpec {
    pub name: String, pub description: String, pub params: Value,   // JSON Schema
    pub behavior: String,          // 子 context 用哪个 behavior；进冻结闭包（§6.3）
    pub inherit: Inherit,
    pub output: OutputSpec,        // Text | Json
}
pub enum Inherit {
    Steps,                         // 父 run 已定的 steps 与 summaries（function_call 模式：已定的消息前缀）；要求父子 loop_model 相同；子 run 另有 <session_history>
    Transcript { recent: usize },  // 父 run 最近 n 条 user / assistant 文本渲染成一条 user 消息（try_create_worksession 现在的做法）
    None,                          // 只有子 behavior 的 system 与调用参数
}

impl SessionToolManager {
    async fn call_tool(&self, call: AiToolCall) -> Observation {
        if let Some(spec) = self.sub_context(&call.name) {                          // NEW：不送 Runtime
            if self.depth() >= self.policy.max_process_depth { return Observation::error("max_process_depth"); }
            let child = self.runs.allocate_id();                                     // 预分配，写进等待记录，进入时幂等
            return Observation::pending(&call, Wait { source: (SUBRUN, child), class: WaitForRuntimeTask, .. });
        }
        self.dispatch_to_runtime(call).await                                         // 原路径：lease / 门槛 / inflight → Runtime
    }
}

impl AgentSession {
    /// T4 进入：父 run 以 PendingTool 挂起入栈，子 run 成为 live。
    async fn enter_sub_context(&mut self, lc: &mut LiveCtx, p: &PendingToolCall, snap: &LLMContextSnapshot, f: &ContextFactory<'_>) -> Result<()> {
        let spec = self.frozen_sub_context(&self.current_behavior(), &p.tool_name)?;
        self.worklog_append(run_history_entries(..) ++ [Outcome{kind: "call", call_id: &p.call_id, next_behavior: &spec.behavior, ..}])?;
        self.state.process_stack.push(ProcessFrame::from_live(&self.state.live_run, Call{ call_id: p.call_id.clone(), tool: p.tool_name.clone(), args: p.args.clone() }, snap));
        let child = f.new_child_run(self, &spec.behavior, ChildOf{ parent: snap, inherit: spec.inherit, run_id: &p.wait.source.id }).await?;   // derive_child（§3.7）
        self.state.live_run = Some(child.live());
        self.state.current_behavior = Some(spec.behavior.clone()); self.state.internal_continuation = Some(spec.behavior.clone());
        // 子 run 的首批输入是 on_behavior_switch 批次，assembler 从栈顶 frame 渲染 <call tool=… call_id=…>参数</call>
        lc.run.set_status(RunStatus::Paused)?;
        commit!(self)
    }
}
```

流程：

1. **拦截**：`SessionToolManager` 认出 `sub_context` 工具，不送 Runtime（后续 policy 阶段的执行前检查也要覆盖它，它同样是一次 Do），返回带等待记录的 `Pending`，`wait.source = {kind: subrun, id: 子 run id}`。llm_context 停止派发，父 run 以 `PendingTool` 返回，同批剩余调用留在快照里，`ToolResults` 之后接着跑（`context_loop.rs` 现有语义）。声明了 `sub_contexts` 的段，`tool_policy.allow_deferred = true`，白名单加入这些名字。
2. **进入**：`handle_outcome` 看到等待记录的 `source.kind == subrun` → `enter_sub_context`：父 run checkpoint 并 flush worklog → 压栈 `Call{call_id}` → 用 `derive_child` 建子 run → 子 run 成为 live → `commit!`。
3. **推进**：子 run 就是一个普通的 live run：可以观察注入、stop、中断、ContextLimit 改写、崩溃恢复，也可以再 fork 或调用子上下文（受栈深限制）。
4. **返回**：除 stop（结束整个 session，栈一并清空）外，子 run 的任何结束都不关闭 Turn，而是 `CallReturn` 出栈。结果归一成一个 Observation：`Done` → Ok(输出)；Error / BudgetExhausted → Error；`WAIT_USER_MSG` → Error{needs_user_input, question}，由父 context 决定要不要问用户。父 run 重新成为 live；Session 是 `subrun` 的 resolver，读取子 run 的结束决定（run.json `host.extra.finish`），以 `ResumeFill::ToolResults{call_id → Observation}` 恢复父 run。父 context 看到的只是一次普通工具调用的结果。
5. **恢复**：崩溃发生在 1、2 之间（父快照已挂起，栈还没压）→ `reconcile_runs` 看到父 run 等待 `subrun` 而栈里没有对应 frame，重做“进入”（子 run id 已在等待记录里，幂等）。崩溃发生在 4 中间（子 run 已终态，frame 还在）→ 重做“返回”。
6. **接手**：父 run 挂起期间，等待记录要求 `subrun` resolver，xllm 没有，拒绝接手（[long-tool TODO](../../notepads/llm-context-long-tool-todo.md) §4 的接手能力检查）。段工具里有 `sub_context` 的 run 同样不让 xllm 接手（同 `app_tools` 规则）。子 run 自己不含 `sub_context` 工具时是普通 run，满足 §3 表“接手”一行的条件就能被 xllm 跑完，之后由 `xagent run` 完成返回。

**与 T2 的关系**：T2 由 `next_behavior` 触发，结果经 `process_result` 交接批次返回；T4 由工具调用触发，结果经 `ToolResults` 返回。子 run 的构造、栈、持久化、worklog 完全共用。

于是 `try_create_worksession` 的目标形态是：ui behavior 声明一个 `sub_context` 工具 `try_create_worksession{reason}`；子 behavior `route_worksession` 设 `inherit = Transcript{recent: 8}`、`output = Json`，只开放 `read`、`exec` 和层 ② 的 `agent-session create-worksession`。**不需要层 ③**：fork 不是一个进程内工具，而是 Session 的 Context 调度（回答 §12 第 7 项）。

### 3.7 需要 llm_context 层提供的原语

调度语义归 Session（本节以上）；llm_context 层只补三个原语，按“llm_context 先行”的规则单列在 [llm_context Context 调度支持 TODO](../../notepads/llm-context-switch-support-todo.md)：

- **`derive_child`**：从父快照只取已定历史，按 `Inherit` 生成子快照。父快照**可以处于 PendingTool 挂起或批次中途**（T4 必然如此），这正是 `rebuild_with_inherit` 现在拒绝的情况。它取代 libopendan 的手工复制。
- **run 中途替换 `config`**（T1）：xllm 的 run 记录允许宿主替换有效配置；resume 只认最新值。
- **hosted run 停在跳转处**（G8）：xllm 接手的 hosted run 以 `next_behavior = B` 结束时记非终态，交给宿主完成转移。

`subrun` 作为 `wait.source.kind` 已在 long-tool TODO §4 的词汇表里，resolver 由 Session 提供，llm_context 层没有新增。

---

## 4. Agent Input 与 Sub Session：原理、协议对象、模板、父子协作

### 4.1 原理

Agent Session 是协议，所以要先说清 Agent 在一个 Session 里**能感知到什么**：

- **`AgentMessage`**：有人（用户、别的 Agent、别的 session）对这个 session 说了什么。
- **`AgentEvent`**：这个 session 关心的某件事发生了（对象变化、定时器、任务结果、子 session 状态、系统通知）。

除此之外没有第三种。Session **不关心这两种东西怎么来的**：msg-center 记录、kevent、timer、task_mgr 回调、登记表 rev 变化，都由上层 bridge 翻译成这两种对象后投递；Session 只要求信封完整（§4.5）。

经过同一条队列的还有 **Session 控制协议**（§4.4：stop、decide、subscribe、unsubscribe、activity、perceive）。它们不是 Agent 的输入，不进上下文，由 Runner 直接执行；搭队列只是为了多方投递与持久化。协议文档要把两者分开写。

与现状的差距：现有 `Input` 有五种 kind（msg / event / change / control / perception）。按原理，`change` 不是一种输入而是 AgentEvent 的一种**投递策略**（§4.3）；`perception` 归控制协议；`msg` 的 payload 只有 `{text}`，缺附件与引用。

### 4.2 AgentMessage

```rust
pub struct AgentMessage {
    pub from: Principal,                 // 发送者身份（用户 DID / app principal / session 引用）
    pub text: String,                    // 正文；Session 渲染为 <msg from= key=>…</msg>，AiMessage 由 assembler 现场生成
    pub attachments: Vec<AttachmentRef>, // 相对路径或 NamedStore 对象 id；不内联内容（队列 payload 上限 250 KB）
    pub msg_ref: Option<ObjId>,          // 来源消息对象（cymsg MsgObject）的 id，供回复与审计
    pub reply_to: Option<String>,        // 回复哪条 key
    pub intent: Option<String>,          // bridge 或上游标注的意图（可选）
}
```

总线上放的是它，不是 `AiMessage`：Session 要先做渲染、附件解析、机械压缩与 receipt，LLM 线格式是最后一步。

### 4.3 AgentEvent 与投递策略

```rust
pub struct AgentEvent {
    pub subscription_id: Option<String>, // 由哪条订阅产生；系统事件（timer）可为空
    pub source: EventSource,             // Object{id} | Session{sid} | Task{id} | Timer{name} | System
    pub event: String,                   // 事件名，如 changed / finished / fired
    pub seq: Option<u64>,                // 来源内单调序号或游标；semi 合并时取最新
    pub summary: String,                 // LLM 看到的一句话（上限 1 KB）；大数据只给引用
    pub data_ref: Option<ObjId>,
    pub terminal: bool,                  // 该订阅的终结事件（子 session 结束等），不被 event_budget（原 change_budget） 挤掉
}

/// Session 自己决定一个事件怎么进入上下文；bridge 不决定。
pub enum Delivery {
    Wake,     // 唤醒：开启或并入 Turn（原 kind = event）
    Observe,  // 不唤醒：只在观察边界（CheckpointHook）或下一个 Turn 的首批输入注入（原 kind = change）
}

impl AgentSession {
    fn resolve_delivery(&self, e: &AgentEvent) -> Delivery {
        match e.subscription_id.as_deref().and_then(|id| self.cfg.subscription(id)) {
            Some(sub) => sub.mode.into(),                       // active → Wake，semi → Observe
            None => self.template().system_event_delivery(e),   // 模板默认：timer → Wake，其它 → Observe
        }
    }
}
```

**半订阅是状态，不是事件流**。Observe 投递的事件在 state.json 里按 `(subscription_id, source)` 只保留最新一份（`pending_events`，上限等于订阅数加系统事件数，很小），terminal 事件单列不覆盖。注入时机：有 run 在跑 → 下一个观察边界；空闲 → 下一个 Turn 的首批输入。注入后随 receipt 清除。**空闲时不丢弃**（现状的 `change_dropped` 行为取消；只有被更新的旧值才记一条 `event_superseded`）。

**内置 bridge**：Session 来源的订阅（`source = session`）现在由 Runner 在观察边界拉登记表比 rev，没有外部 producer。按原理它也要产出 AgentEvent，所以把它定义为 Runner 内的内置 bridge：比对 rev 后合成 `AgentEvent{subscription_id, source: Session, seq: rev, summary: watched 字段差异}`，再走同一套 `pending_events` 与注入逻辑。worklog 与实验断言只看一种形态。子 session（`origin.parent_session` 指向本 session）不需要显式订阅，内置 bridge 直接查登记表，事件的分类与投递见 §4.14；其中 Wake 的几类进入本批 `wake_events`，不进 `pending_events`。

### 4.4 Session 控制协议

| 命令 | 谁发 | 效果 | 不建队列时的替代 |
|---|---|---|---|
| `stop {reason}` | 任何有权者 | Runner 中断当前推理，Turn 记 `stopped`，session Finished | lease 持有者自己退出（SIGINT）；无法远程停 |
| `decide accept\|discard` | 决定者 | finished 后由驱动者在 artifact 锁下执行 | 决定者直接走 `artifacts().decide`（锁已存在），不经 session |
| `subscribe` / `unsubscribe` | 驱动者 app、父 session | 改 `config.subscriptions`，`config_rev + 1` | 创建时声明；运行中不可改 |
| `activity {summary, touch, clear}` | exec 子进程 | 合并进 `state.activity` | Runner 由工具调用推断 touching；summary 来自 scope |
| `perceive {kind, summary}` | exec 子进程 | 驱动者以 lease 追加到感知流 | 结束时只有 run_digest / task_outcome |

控制命令沿用 `ControlCommand`，但在协议文档里独立成《Session Control Protocol》。`perception` kind 并入控制协议。

### 4.5 InputBus：信封与 bridge 保证

```rust
pub enum SessionInput { Message(AgentMessage), Event(AgentEvent), Control(ControlCommand) }

pub struct Envelope {
    pub key: String,                 // producer 去重 key；Observe 事件按 (subscription_id, source) 合并，与 key 无关
    pub src: String, pub index: u64, // 来源 id 与单调 index：消费游标、累积 ack
    pub from: Principal,             // 自报身份，审计用；权限不来自它
    pub at_ms: u64,
}

#[async_trait]
pub trait InputBus: Send + Sync {
    async fn post(&self, input: &SessionInput, who: &str) -> Result<u64>;                       // 任何有写权限者；发布 wake_event
    async fn fetch(&self, progress: &SourceProgress, max: usize) -> Result<Vec<(Envelope, SessionInput)>>;  // 仅 lease 持有者
    async fn confirm(&self, progress: &SourceProgress) -> Result<()>;                            // 先提交 state.json，再累积 ack
    async fn wait(&self, timeout: Duration);                                                     // kevent 或轮询
}
```

bridge 必须保证：同一来源内有序、至少一次、key 唯一。Session 用 receipt 与 `recent_keys` 做幂等。现有 `KmsgInput` / `DirMsgQueue` / `Waker` / `confirm_inputs` 已覆盖这四个操作，`InputBus` 只是把它们收拢并换掉 payload 类型。

### 4.6 事件桥（bridge）

| bridge | 在哪 | 产出 |
|---|---|---|
| kevent 桥 | xagent `run` / `serve` 进程内；以后是 OpenDAN Supervisor | 按 session 的 ObjectEvent 订阅模式订阅 kevent → `AgentEvent{subscription_id, source: Object}` |
| timer 桥 | 同上（self_check） | `AgentEvent{source: Timer}` |
| task_mgr 桥 | 后移 | `AgentEvent{source: Task}` |
| msg-center 桥 | 后移（UI session） | `AgentMessage{msg_ref}` |
| 子 session 桥 | Runner 内置（§4.3、§4.14） | `AgentEvent{source: Session}`：订阅的 session 与本 session 的子 session |
| CLI | `xagent post --msg / --event` | 任意一种，用于实验与手工驱动 |

实验里用 `--no-bridge` 关掉进程内桥，手工 `post` 控制时序。

### 4.7 Session 模板

模板是 `SessionSpec` 的预设，按 `session.class` 选择；内置四个，`agent.toml [session.<class>]` 可覆盖；创建时解析进 session_config 的现有字段（`end_condition`、`input_policy`、`channels`、`subscriptions`、`prompt.behavior`）加一个新段 `session.policy`，与 behavior 一起冻结。

```rust
pub struct SessionTemplate {
    pub class: String,
    pub kind: SessionKind,
    pub turns: Turns,                 // One | Unbounded | N(n)   → end_condition
    pub wait_user_msg: WaitPolicy,    // Allowed（Turn 保持打开等输入）| FinishFailed（work：不许等人，记 failed 并结束）| FinishCompleted
    pub input: InputChannel,          // None（不建队列）| Queue
    pub observe: Observe,             // Off | Events | EventsAndActive     → 观察边界注入什么
    pub load_hints: bool,
    pub default_behavior: Option<String>,
    pub max_process_depth: u8,        // process_stack 深度上限（默认 4，§3.3）
    pub max_sub_sessions: u8,         // 同时未结束的子 session 数上限（默认 4，§4.11）
    pub max_session_depth: u8,        // 子 session 嵌套深度上限（默认 2，§4.11）
    pub system_event_delivery: fn(&AgentEvent) -> Delivery,
}
```

| class | turns | wait_user_msg | input | observe | hints | 说明 |
|---|---|---|---|---|---|---|
| `work` | One | FinishFailed | **None**（声明了订阅则升为 Queue） | Events | 否 | 一次性任务：一个 Turn 跑到结果；不等用户；默认没有队列 |
| `ui` | Unbounded | Allowed | Queue | EventsAndActive | 是 | 长生命周期对话入口（后移） |
| `self_improve` | One | FinishFailed | None | Off | 否 | 感知窗口整理 |
| `self_check` | Unbounded | FinishFailed | Queue（timer 事件） | Off | 是 | 定时自检 |

**无队列 work session 的后果**（接受这些才成立）：stop 只能由 lease 持有者自己退出；decide 走 artifacts 门面不经 session；exec 子进程的 `activity --touch` / `perceive` 没有通道，touching 由工具调用推断、感知只有结束时的 run_digest；父 session 收子 session 的状态事件不需要队列（内置 bridge 直接读登记表，§4.14），但子 LLM 主动发给父的消息需要父有队列。声明了订阅的 work session 需要队列承接 bridge 的事件，模板自动升为 Queue。

### 4.8 xagent 的三种运行形态

| 形态 | 命令 | 行为 | 返回时机 |
|---|---|---|---|
| 单 Turn | `xagent run <sid> --msg "…"` | `post(Message)` → `drive(StopWhen::TurnClosed)`；无队列 session 的首条输入是 objective 本身（bootstrap），`--msg` 直接成为 bootstrap 批次的一部分 | 当前 Turn 关闭；或 Turn 打开但无输入可推进 → 退出码 3 |
| 消费积压 | `xagent run <sid>` | 只 drive；有积压就处理到 Turn 关闭 | 同上 |
| 常驻 | `xagent serve <sid>…` | 起 bridge；循环 `drive(StopWhen::Idle)`，Idle 后 `InputBus::wait`，被唤醒再 drive | SIGINT / `stop` / session finished |

`--until finished|idle|outcomes:<n>` 保留现有 `StopWhen`，用于调试。

### 4.9 Turn 关闭与 `StopWhen::TurnClosed`

```rust
pub enum StopWhen { Idle, Finished, MaxOutcomes { n: u64 }, TurnClosed }
pub enum DriveResult {
    /* 现有变体 … */
    TurnClosed { rev: u64, turn: u64, status: TurnStatus, answer: Option<String> },
    TurnOpen   { rev: u64, turn: u64, waiting_for: Option<WaitingFor> },   // 只在 wait_user_msg = Allowed 的模板下出现
}
```

Turn 规则沿用 readme：没有打开的 Turn 时提交的输入批次开启新 Turn；切换、fork、independent、工具子上下文的进入与返回（§3.3）、观察注入、挂起、重写、重启都延续；只有 `finish_run` / `stop_session` 关闭。模板的 `wait_user_msg` 决定 `WAIT_USER_MSG` 在该 session 里是"保持打开"还是"结束"。

### 4.10 Sub Session：与子 context、Sub-Agent 的区别

Agent Session SDK 化的一个主要目的，是让 Agent（以及应用）方便地把一段工作交给**另一个 session**。[长任务与执行体 RFC](<./OpenDAN Long Task & Sub-Agent.md>) §14 把执行体分成 Tool（函数）、Session（线程）、Sub-Agent Instance（进程）三类，§21 的 long-once session 就是这里的 Sub Session。结合 §3 的 Context 调度，“把一段活交出去”有四种做法：

| | 工具子上下文（T4） | fork（T2） | **Sub Session** | Sub-Agent / 其它 Agent |
|---|---|---|---|---|
| 是什么 | 本 session 里由工具调用触发的子 run | 本 session 里由 `next_behavior` 触发的子 run | 同一个 Agent 的另一个 session：自己的目录、state.json、worklog、Turn、lease | 另一个 Agent DID |
| 能否并行 | 否，父 run 挂起 | 否 | **能**，各自的 Runner 推进 | 能 |
| 上下文 | 按 `inherit` 继承父 run 的历史 | 继承父 run 的 steps | 不继承；只有创建时给的 objective、首批输入、附件引用（可选附父最近对话摘录） | 只有消息 |
| 共享什么 | 父的 lease、state、Turn、runs/ | 同左 | 同一个 Agent State（登记表、认知、产物、behavior 目录），可共用 workspace | 不共享 |
| 生命周期 | 不超过一次工具调用 | 不超过父 Turn | 独立：可以比父 Turn 长，可以常驻 | 独立、长期 |
| 结果怎么回来 | `ToolResults` | `process_result` 交接批次 | 登记表状态 + AgentEvent + 子主动发的 AgentMessage；同步等待时作为工具结果（§4.14、§4.15） | 消息（msg-center） |
| 谁看得见 | 父 session 的 runs/ 与 worklog | 同左 | 登记表、`sessions` 列表、自己的目录与报告；可以单独验收 | 对方系统 |
| 适合 | 需要父上下文的窄意图小决定（路由、分类） | 换个角色做同一主任务的一段 | 一段独立工作：要并行、要长时间、要自己的 workspace / runtime / 产物 / 验收，或要在父结束后继续 | 宽意图、角色级能力域（RFC §12） |

选择顺序沿用 RFC §10–§11：只是想隔离上下文 → 先用子 context（T2 / T4）；需要并行、独立生命周期、独立产物与验收 → Sub Session；角色级的长期能力域 → Sub-Agent（不在 xagent 范围，§1.2）。

Sub Session 不是新的协议对象：它就是 `origin.parent_session` 指向父 session 的普通 session（计划 §4.8 `create_session`），父子关系只体现在登记表和下面几条约定里。**父子都只写自己的 state.json**，彼此只经过登记表、session 目录（只读）、输入通道和控制协议沟通。

### 4.11 创建：一个标准 agent tool

创建 Sub Session 是一个层 ② 工具（§5.5），由 `agent-session create-worksession` 实现，同时在 `bash_tools` 里声明 schema。LLM 看到的是一个普通 function / action，执行经 exec 进 Runtime，所以 xllm 接手的 run 也能用。

```text
agent-session create-worksession --objective <text>
    [--msg <text>]... [--attach <path|objid>]...   首批输入：AgentMessage，附件只给引用
    [--context recent:<n>|none]                     把父 run 最近 n 条对话摘录附进首批输入（默认 none；不继承 steps）
    [--class work|…] [--behavior <name>]            模板与入口 behavior（默认 work 模板）
    [--workspace inherit|new|<id>]                  默认 inherit：与父共用，靠活动视图避让
    [--runtime inherit|<id>]                        默认 inherit：要求与父相同的 runtime_id，首次推进时绑定
    [--report final|progress|none]                  父怎么收到汇报（§4.14），默认 final
    [--interactive]                                 子 session 建输入队列，父之后可以继续 post（默认不建，§4.7）
    [--wait]                                        同步等待（§4.15）：返回 Pending，父 run 挂起到子 session 结束
```

工具内部（都在 exec 子进程里，经 `AgentStateClient::connect`）：

1. 从 exec 环境（Session 注入的 helper 环境）取父 `session_id`、驱动者 principal 与 `(run_id, call_id)`；`origin = {parent_session, report, created_by_call}`，子的 driver = 父的 driver（§5.6）。
2. 幂等键 = `(父 sid, run_id, call_id)`：同一次调用被重放时，`create_session` 返回已有目录（计划 §4.8 `same_session_identity`），不会建出第二个。
3. 校验 `session.policy.max_sub_sessions`（父同时未结束的子数）与 `max_session_depth`（RFC §13.4 防无限递归的工程边界），超限返回错误。
4. 按模板创建（§4.7）；工具进程能读 behavior 目录就当场冻结（§6.3）；登记；把首批输入写成 bootstrap 批次。
5. 不推进子 session（推进见 §4.12）。默认立刻返回 `{session_id, status: "created"}`；`--wait` 时返回带 `task_id = "session:<sid>"` 的结构化 Pending，exec 原样转发给 llm_context（`agent_tool/src/llm_bash.rs::try_forward_inner_agent_tool_result`）。

**与现状的差距**：现在 `api.rs::create_session` 会向父 session 的队列投一条 `subscribe` 控制命令，让父半订阅子。这要求父有队列（§4.7 的无队列 work session 做不到），投递方式也写死为 semi。改为父对子的关注**隐式成立**：父的内置 session bridge 按 `origin.parent_session = 自己` 查登记表（§4.14），`origin.report` 记录创建时选的汇报方式。

### 4.12 推进：谁来跑子 session

**创建不等于推进。** 子 session 的 driver 是一个 principal（默认就是父的 driver），真正推进它的是这个 principal 的托管进程：OpenDAN 里是 Supervisor（计划附录 A.2），xagent 里是进程内的 `ChildDriver`：

```rust
/// xagent 进程内：推进本进程正在驱动的 session 所创建的子 session。
impl ChildDriver {
    async fn tick(&self, driving: &[SessionId], deps: &Deps) {
        for e in deps.agent.sessions().children_of(driving).await? {        // NEW：按 origin.parent_session 查登记表
            if e.driver != deps.who || e.status.run_state.is_terminal() || self.running(&e.session_id) { continue; }
            if self.running_count() >= deps.options.max_child_concurrency { break; }
            self.spawn(e.session_id, |sd| drive(sd, deps, StopWhen::Idle));  // 各持自己的 lease，与父并行；子的子也由同一个 ChildDriver 接管
        }
    }
}
```

- 父每次提交之后、以及登记表变化时 `tick`。子 session 各持自己的 lease，与父并行；父进程挂掉不影响子的状态，重启后 ChildDriver 重新接管未结束的子。
- 子的状态变化后，ChildDriver 也负责重新 `drive(parent)`，父才能收到 §4.14 的事件。
- `xagent run <parent>`：父 Turn 关闭后，默认继续推进本进程拉起的子 session，直到它们空闲或结束再退出；`--detach-children` 立即返回，留给 `xagent serve`。`xagent serve` 同样接管所服务 session 的子 session。
- 子 session 首次推进时自己绑定 runtime（`binding.json`），默认要求与父相同的 runtime_id；共用 workspace 时按活动视图避让，与多个顶层 session 共用 workspace 相同。

### 4.13 父子之间的沟通渠道

| 方向 | 渠道 | 内容 | 谁发起 | 怎么进入对方 |
|---|---|---|---|---|
| 父 → 子 | 创建参数 | objective、首批 AgentMessage、附件引用、可选的父对话摘录 | 父 LLM（创建工具） | 子的 bootstrap 批次 |
| 父 → 子 | `agent-session post <child> --msg` | `AgentMessage{from: Session(父)}`：补充要求、回答子的提问 | 父 LLM | 唤醒子；要求子是 `--interactive`（有队列） |
| 父 → 子 | 控制：`ctl <child> stop`、`decide accept\|discard` | 停止、验收 | 父 LLM，或父 Session（stop 级联，§4.16） | 不进上下文 |
| 子 → 父 | 登记表状态 `SessionStatus` | `run_state`、`outcome`、`one_line_status`、`report_brief`、`pending_decision`、`last_error` | 子 Session 每次提交后自动 `report_state` | 父被动读取，或父的内置 bridge 产出 AgentEvent（§4.14） |
| 子 → 父 | `agent-session post <parent> --msg` | `AgentMessage{from: Session(子)}`：提问、阶段性交付 | 子 LLM | 唤醒父（消息总是 Wake）；要求父有队列，否则子只能经状态汇报 |
| 子 → 父 | 报告与产物 | `report.md`、登记的产物、worklog | 子结束时 | 父按需 `read-session` / `artifact head` |

两条约定：

- **子 session 的“用户”是父 session**：子默认 headless，没有 msg-center 通道，不直接对人发消息（RFC §21.2）；需要人参与时向父提问，由父决定是否转给用户。子的 system 段里有 `session.parent`（冻结变量，§6.5），子 LLM 知道该找谁。
- **子要等输入时**：work 模板的子遇到 `WAIT_USER_MSG` 按 FinishFailed 结束，问题写进报告（§4.7）；`--interactive` 的子（`wait_user_msg = Allowed`）Turn 保持打开，状态变为等输入，父收到 `needs_input` 事件后用 `post` 回答。

### 4.14 进展与汇报：被动查询与主动推送

总原则沿用 RFC §6–§7：**登记表里的状态是真相，事件只是加速器**；被事件唤醒后要重新读状态，不能只信事件内容。有三种机制：

1. **被动查询（父拉取）**：父 LLM 随时可以 `agent-session sessions --children [--active]`（子列表与 `one_line_status`）、`read-session <sid> [--report] [--worklog <n>]`。读到的就是真相，不受订阅影响。
2. **状态推送（系统自动，子 LLM 不参与）**：子每次提交都 `report_state`；父的内置 session bridge（§4.3）在组批与观察边界时，按 `origin.parent_session = 自己` 比对子的 `status.rev`，合成 `AgentEvent{source: Session{child}, event, seq: rev, summary, terminal}`：
   - **进度**（`one_line_status`、`activity` 变化）→ `event = progress`，Observe：不唤醒，按 `(child, source)` 只留最新一份，在下一个观察边界或下一批注入（§4.3 的 `pending_events`）。
   - **需要关注**（`run_state` 进入等输入，或出现 `pending_decision`）→ `event = needs_input | needs_decision`，Wake。
   - **结束**（finished / failed / stopped）→ `event = finished`，`terminal = true`，Wake；`summary` 取 `report_brief`。
   - 创建时的 `--report` 决定父收哪些：`final` = 需要关注与结束；`progress` = 再加进度；`none` = 都不推，父只能拉取。它对应 opendan 的 `report_delivery`（final_only / top_level / all），但由**接收方**（父）选择，符合 §4.3“投递策略由 Session 决定”。
3. **显式汇报（子 LLM 主动）**：子 LLM 调 `agent-session post <parent> --msg …`，用于需要父决定的问题或阶段性交付。它是 AgentMessage，总是唤醒父；频率由子的 behavior 提示词约束。

父无队列也能收到 2 里的事件，因为它们来自登记表而不是队列；把父唤醒的是推进父的那个进程（ChildDriver / Supervisor），它在子状态变化后重新 `drive(parent)`。

### 4.15 等待：异步与同步

- **异步（默认）**：创建工具立即返回，父继续推进；结果按 §4.14 以事件到达，可能落在父后续的 run 甚至后续的 Turn。
- **同步（`--wait`，或对已有子 session 调 `agent-session wait <sid>`）**：工具返回 Pending，等待记录为 `wait.source = {kind: session, id: <sid>}`、`class = wait_for_task`（由 `task_id` 的 `session:` 前缀归一而来，见 [long-tool TODO](../../notepads/llm-context-long-tool-todo.md) §4）。父 run 以 PendingTool 挂起（父的段要开启 `allow_deferred`，同 T4），Turn 保持打开。Session 提供 `session` resolver，它只读登记表：
  - 子结束 → `Ready(Observation{session_id, outcome, report_brief, answer_ref, artifacts})`；
  - 子在等输入或等决定 → `Ready(Observation{status: needs_input, question})`，父用 `post` 回答后再 `wait`；
  - 其它情况继续等，ChildDriver 同时在推进子；
  - 子的结束事件与挂起调用按 `(kind, id)` 匹配，作为工具结果消费，不再作为事件重复注入。
- **一次等多个**：同一批次里对每个子各调一次 `wait`；批次串行派发，效果就是“等全部”。
- **与 T4 的区别**：T4 的子 run 在父 session 内串行，父挂起时没有别的东西在跑；`--wait` 的子 session 在自己的 lease 下推进，可以同时有别的子 session 在跑，父崩溃也不影响子。
- **接手**：`session` resolver 只依赖 Agent State，不依赖 Runner 内存；但 v1 只有 xagent 提供它，父 run 挂起期间 xllm 拒绝接手（同 T4）。

**父结束规则（汇总）**：父的 `decide_end` 发现还有未结束、且 `report != none` 的子 session 时，不结束 session：run 结束，Turn 保持打开，`waiting_for = Children{sids}`；子的结束事件唤醒父并入同一个 Turn，父 LLM 汇总后再 `END`。这样 work 父也能“先并行派出几个子 session，再汇总”。不想等的子，要么创建时选 `--report none`，要么先 `ctl <child> stop`。

### 4.16 生命周期与限制

- **stop 级联**：父 stop → 对未结束的子投 `stop`（子有队列时），或由 ChildDriver 直接中断（子无队列；驱动者是同一个 principal）。子被 stop 只在父那里产生 `finished{stopped}` 事件，不影响父。
- **验收**：work 子 session 结束后 `acceptance = pending`，默认由父（创建者）决定：父 LLM 读报告后 `ctl decide <child> accept|discard`。
- **数量与深度**：`session.policy.max_sub_sessions`（同时未结束的子，默认 4）与 `max_session_depth`（默认 2）。
- **Turn 与预算不合并**：子有自己的 Turn（计划 §8.3），父的 Turn、Round、预算都不含子；子按自己的 budget。
- **归档与清理**：子目录的归档与清理沿用顶层 session 的规则（后移）；登记表条目保留，供审计与 `sessions` 查询。

---

## 5. Runtime：Agent 的 Sandbox

> 2026-10-02：共享 Runtime 已落实在 `agent_tool::runtime`，独立于 xagent 完成；配置、验收与后续 policy 边界见 [Runtime 实施记录](../../notepads/llm-context-agent-runtime-todo.md)。本节 §5.1–5.3 描述当前接口；grant、ActionGuard 和审批仍为后续设计。

### 5.1 定位

Runtime 是所有 agent-tool 调用的统一入口，负责工具解析、派发、执行环境信息和执行跟踪。exec、read/write/edit、模板 EXEC 使用同一执行体；MCP 在所配置服务执行，进程内宿主工具在 Runner 执行，由统一入口管理注册与派发。native/tmux 不提供 OS 级隔离，workspace 只约束内置文件工具路径与 exec.cwd。

| 层 | 职责 |
|---|---|
| Session（SessionToolManager） | lease、提交门槛、inflight、touching、receipt；装配 Session bin/helper 与环境，核验 binding |
| Runtime（AgentRuntime / Sandbox） | 解析工具、派发调用、exec 与文件后端、执行身份握手及恢复、RuntimeInfo；为后续 policy 留执行前入口 |
| llm_context | 推理循环与工具调用；PromptExec 接口接收模板执行器，保持依赖方向 |

Runtime 配置顶层位于 .llm_context.runtime，默认 native；Session 的 runtime.requirement.runtime_id 是绑定要求。runtime 不包含 fs_view、path_layers、limits。Session 内部注入 PATH 层不属于配置 schema；工具的 filesystem_policy、exec 请求的超时和输出上限继续生效。

### 5.2 当前接口

```rust
#[async_trait]
pub trait AgentRuntime: Send + Sync {
    fn descriptor(&self) -> &RuntimeDescriptor;
    fn config(&self) -> RuntimeConfig;
    async fn info(&self) -> Result<RuntimeInfo>;
    async fn open(&self, ctx: &RuntimeOpenCtx, tools: &ToolsConfig,
                  host_tools: Vec<Arc<dyn AgentTool>>)
        -> Result<(EffectiveTools, XllmToolManager)>;
    async fn reconcile_execution(&self, rec: &ExecutionRecord) -> Result<()>;
}
#[async_trait]
pub trait Sandbox: ToolManager {
    fn workdir(&self) -> &str;
    fn env_check(&self) -> Value;
    async fn exec(&self, req: BashRunRequest, ctx: &SessionRuntimeContext)
        -> Result<BashRunOutput, AgentToolError>;
}
```

`RuntimeRegistry::from_config` 构造共享执行体，打开阶段探测目标并展开工具。XllmToolManager 实现 Sandbox；RuntimeOpenCtx 不依赖 SessionDir/Binding，只接收 run 身份、registrar、来源和宿主环境。Sandbox 的 ToolManager.call_tool 复用参数约束、目标侧真实路径核验、握手与结果归一；工具业务错误是 Observation::Error，传输/派发错误是 ToolDispatchError，可能已开始执行则 effect_unknown=true，不重试副作用。

`RuntimeDescriptor` 保存 runtime_id/kind/host/target/workdir/capabilities，打开后写入 run.json；恢复核验完整目标与 cwd，不只比较 kind。Session binding 升至 /3 并保存实际 target，核验早于推理与旧执行恢复。HostRunInfo.env_check 保存 runtime、Session 环境、PATH 和 bin manifest；独立 xllm 接管时校验目录、helper 内容及执行权限，凭据保存环境变量引用。缺少进程内宿主工具按 app_tools 规则拒绝。

RuntimeInfo 的 id/kind/os/arch/hostname/shell/cwd/tools 来自执行体，tools 摘要有上限。current_time/timezone 在执行处更新；Session 将新鲜量放输入批次，稳定值可用于 system。模板 PromptExec 适配同一 Sandbox，未注入执行器不会本地执行；INCLUDE 仍读控制侧素材。

### 5.3 Runtime 类型与首版能力

| kind | exec 与文件位置 | 当前状态 |
|---|---|---|
| native | Runner 同机 bash 与本机文件系统 | TrackedBashRunner 握手、标记核验、取消及恢复 |
| tmux | 同机指定 session 的专用 pane；本机文件系统 | create/attach/create_or_attach；pane 内串行派发；不注入用户 pane，不销毁继承 session |
| remote_ssh | SSH 目标 cwd、SFTP 文件后端 | 系统 OpenSSH 非交互连接；Linux/bash/SFTP；目标侧握手、停止核验、断线与强杀恢复；独立 xllm 可用 |
| container/container_host/remote_node/http_proxy_runtime | 后续评估 | 选择时 Capability 错误，不冻结协议或完整配置 |

SSH 的命令和文件内容通过数据通道传送，写入用目标侧临时文件替换。握手持久化 execution_id、host、boot_id、PID/start ticks 后才放行；目标不可达或无法证明停止时 RecoveryBlocked，没有持久结果记为未知、不重放。SSH alias 重定向拒绝旧 run 恢复。

配置素材、run.json、快照与日志在控制侧；runtime.workdir 属于执行侧。native/tmux 使用本地实际 cwd 锁；SSH 不取远端路径的本地 flock，跨 Runner 并发由宿主协调。libopendan 尚未部署远端 Session helper，明确报 Capability；不把控制侧 .runtime/bin 注入远端 PATH。

### 5.4 后续设计：临时授权（RuntimeGrant）

本节尚未实现，也不冻结 RuntimeGrant、ActionGuard 或审批接口。后续 policy 需分别定义工具授权、执行体强制能力、有效策略 revision 与审批；不能仅靠普通 SSH/tmux 声称隔离。

```rust
/// 由用户 / OpenDAN 签发，存放在 Agent State（state/runtime_grants/<id>.json），session 引用。
pub struct RuntimeGrant {
    pub id: String,
    pub runtime_id: String,          // 如 host:<host_id>
    pub issued_by: Principal, pub issued_at_ms: u64,
    pub expires_at_ms: u64,          // 时间限制
    pub scope: GrantScope { paths: Vec<String>, network: bool, tools: Vec<String> },
    pub sessions: Vec<String>,       // 空 = 该 Agent 所有 session
}
```

- session 的 **primary runtime** 仍按首次推进绑定、不可换（`binding.json`）。grant 是**叠加**：生效期间 `SandboxEnv` 的工具列表多出一组带前缀的工具（`host.exec`、`host.read_file` …），LLM 显式选择在哪个沙箱做事，审计可区分。
- 到期：下一个观察边界起工具列表里消失；到期后的调用 `dispatch` 直接 Deny。grant 变化作为 `AgentEvent{source: System, event: grant_changed}` 以 Observe 投递注入，LLM 知道权限变了。
- grant 的签发与撤销走 Agent State 门面 `runtime_grants()`（新），不走 session 控制协议；`xagent ctl grant --runtime host:… --ttl 30m --paths …` 只是写 Agent State。
- 当前 xllm 按保存的 runtime 构造与核验目标，native/tmux/SSH 可恢复；Session 的 helper 与 app_tools 依赖须可用。未来 grant 的接管核验待 policy 阶段设计。

### 5.5 工具的三层来源（修订）

| 层 | 例子 | 由谁解析 / 执行 | 访问 Agent State | xllm 能否接手 |
|---|---|---|---|---|
| ① 内置 / MCP / `bash_tools` | `read_file` `write_file` `edit_file` `exec` `glob` `grep`、MCP | **Runtime** 按有效工具配置派发；内置 exec/文件使用执行体，MCP 仍在所配置服务执行 | 不访问 | 保存目标与依赖核验通过时能 |
| ② CLI 形态 session 工具 | `agent-session ctl … / recall / note / sessions / read-session / create-worksession / artifact` | 作为沙箱 PATH 上的命令，经 `exec` 进 Runtime；子进程里 `AgentStateClient::connect` | 子进程直连（host / remote 沙箱里只能走 kRPC，这是 §7 `connect` 必须有 kRPC 形态的硬理由） | 能 |
| ③ 进程内 session-aware 工具 | 暂无成员：fork 与继承 context 的工具是 Session 的 Context 调度（§3.5、§3.6），不是进程内工具 | `SessionToolProvider` 注册给 Runtime，**仍经 `dispatch` 的 admit**，执行在 Runner 进程 | 直接拿 `AgentStateClient` + 会话句柄 | 不能（`app_tools`） |

规则不变：能用 ② 的不用 ③。当前 classify_effect 继续服务于 Session inflight；ToolSpec.effect 统一、guard、grant 与审批留到 policy 阶段。

### 5.6 哪些 session 工具会访问 Agent State

| 工具（层 ②） | Agent State 门面 | 读 / 写 | 备注 |
|---|---|---|---|
| `ctl activity --summary/--touch`、`ctl perceive` | — | 控制协议：投递到本 session 队列（§4.4） | 无队列 session 不可用 |
| `recall <query>` | `cognition().recall_hints` | 读 | |
| `note <text>` | `cognition().notebook_append(who)` | 写 | |
| `sessions`、`read-session <sid>` | `sessions().query/lookup` + 目录读 | 读 | 遵守 `acl.agent_access` |
| `create-worksession`、`post <sid>` | `sessions().register` / `post_input` | 写 | 创建 Sub Session（§4.11）；子 session 的 driver 默认是调用者 |
| `sessions --children`、`wait <sid>` | `sessions().children_of`（新）/ `lookup` | 读 | 查询子 session 进展（§4.14）；`wait` 返回 Pending，由 `session` resolver 解析（§4.15） |
| `artifact register/head` | `artifacts()` | 写 / 读 | |
| `ctl decide` | `artifacts().decide` | 写 | 有队列经驱动者；无队列直接走门面 |
| `ctl grant` / `revoke`（新） | `runtime_grants()` | 写 | 只有用户 / OpenDAN 身份可签发；Agent 自己不能给自己授权 |

---

## 6. behavior_cfg 的构造与冻结

### 6.1 原则

1. behavior 是 **Agent 的**配置（`<agent_root>/behaviors/<name>.toml` + `*.inc`、`role.md`、`self.md`、`i18n/`），因此由 Agent State 提供，不由应用随手写进 session。
2. llm_context 构造时要初始化 system 段，此刻需要 behavior；**Session 一旦构造，用的 behavior 就固定**：Agent 后来改了 behavior 文件，旧 session 不受影响，新 session 才看到。
3. 冻结的是“文本与策略”，不是渲染结果：时间、活动 session、hints 等新鲜量仍然每个输入批次现算，放在 `<session_input>`（S-20）。
4. 冻结后 system 段只依赖冻结材料 + session_config，所以同一 session 内每个新 run 的 system 相同（稳定前缀），也让 history 的 `<<step_history>>` 跨 run 可比。

### 6.2 BehaviorCatalog：Agent State 的新门面

```rust
/// Agent 的 behavior 目录与身份文本。文件实现读 <agent_root>/behaviors/、role.md、self.md、i18n/；
/// kRPC 实现由 OpenDAN 提供；进程内实现直接读 AgentConfig。
#[async_trait]
pub trait BehaviorCatalog: Send + Sync {
    async fn identity(&self) -> Result<IdentityText>;                 // role / self / i18n 片段
    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>>; // 已解析、include 已展开
    async fn list(&self) -> Result<Vec<BehaviorMeta>>;
    /// 目录当前版本（文件实现 = 相关文件 mtime/size 的摘要），记进冻结快照便于审计。
    async fn revision(&self) -> Result<String>;
}

pub trait AgentStateClient: Send + Sync {
    /* 现有：agent_did / agent_id / agent_root / sessions / activity / perception / cognition / artifacts / locks */
    fn behaviors(&self) -> &dyn BehaviorCatalog;   // 新增
    fn runtime_grants(&self) -> &dyn RuntimeGrants; // 新增（§5.4）：list_active(sid) / issue(who) / revoke(who)
}
```

`BehaviorConfig` 沿用 opendan 现有 toml 的结构（`behavior_cfg.rs`），只做两处收敛：

```rust
pub struct BehaviorConfig {
    pub meta: Meta { name, objective, next: Vec<String> /* 可选：声明可切换目标，与 sub_contexts 的 behavior 一起构成冻结闭包 */ },
    pub prompt: Prompt {
        mode: LoopModel,                 // agent(function_call) | behavior；原 agent.toml [session.<class>].loop_mode 移到 behavior
        on_init: String,                 // system 段模板
        on_wakeup / on_behavior_switch / on_behavior_step_ob: Option<String>,   // 输入批次 / 观察注入模板
        parser, parser_strict, output,
    },
    pub capabilities: Capabilities { tool_whitelist, action_whitelist, tool_plan, approval_required, disable_capabilities },
    pub budget: Budget { max_tool_iterations, max_consecutive_errors, max_total_tokens, max_completion_tokens, max_wallclock_ms },
    pub model: Model { preferred, fallbacks, temperature, provider_options },
    pub switch_mode: ProcessMode,        // normal | fork | independent：**进入本 behavior 时**的模式（原 driver.switch_mode）
    pub sub_contexts: Vec<SubContextSpec>, // 本 behavior 对 LLM 提供的工具子上下文（§3.6）
    pub hooks: Hooks,                    // on_context_limit_reached / on_llm_message_compress / on_provider_failed / on_interrupt_*
}
```

`switch_mode` 放在目标 behavior 上，正好回答 readme 里的 TODO（“切换模式由 target behavior 的配置决定，而不是由当前 session 决定？”）：是。`SessionAssembler::process_mode(cfg, behavior)` 改为读冻结的 `behaviors[target].switch_mode`，`extensions.opendan.process_modes` 废弃。

### 6.3 冻结：时机、位置、范围

**位置**：`session_config.prompt` 新增一段（schema 升为 `opendan.session_config/3`），不新增文件：

```jsonc
"prompt": {
  "llm_context": { … },                 // 不变：provider / tools / limits 基线
  "behavior": "plan",                   // 入口 behavior 名（不变）
  "frozen": {                           // 新增：构造时冻结
    "catalog_rev": "sha256:…",
    "frozen_at_ms": 0,
    "frozen_by": "app:xagent@alice",
    "identity": { "role": "…", "self": "…", "i18n": { … } },
    "behaviors": { "plan": { …BehaviorConfig… }, "do": { … } }
  },
  "system_prompt": "…", "context": [], "mechanical_compress": { … }
}
```

**时机**：
- `create_session` 时，若创建者的 `AgentStateClient` 能读目录（文件版能看到 AgentRoot、进程内、kRPC 有权限）→ 立即冻结。
- 否则留空，由驱动者在首次 drive 的 bootstrap（任何推理之前、`bind_or_verify` 之后）冻结并原子替换 session_config（`config_rev + 1`）。state.json 的 `bootstrap_done` 仍是“首批输入已提交”的含义，不复用。
- 冻结后 `BehaviorAssembler` **只读 `prompt.frozen`**，不再碰目录；读不到 frozen 又拿不到目录 → `RecoveryBlocked`，不猜。

**范围**：入口 behavior + `meta.next` 与 `sub_contexts[].behavior` 声明的可达闭包 + identity。切换到一个未冻结的目标时（LLM 自由跳转），驱动者在该次切换前从目录**补冻结**（追加进 `frozen.behaviors`，`config_rev + 1`，worklog 记 `control_applied{behavior_frozen}`）。这保证“用过的 behavior 从首次使用起不再变”，同时不要求提前冻结整个目录。补冻结用的是目录的当前版本，所以 `catalog_rev` 只描述首次冻结；审计看 worklog。

### 6.4 BehaviorAssembler：从冻结 behavior 到一个 run

```rust
pub struct BehaviorAssembler { engine: PromptRenderEngine /* llm_context::prompt_engine，__EXEC 关闭 */ }

#[async_trait]
impl SessionAssembler for BehaviorAssembler {
    /// system 段：身份 → 不可覆盖约束 → behavior.on_init 渲染结果（应用 prompt 与 context 并入其中）→ objective。
    /// 只用冻结材料与 session_config；变量集见 6.5；无副作用。
    async fn system_text(&self, cfg: &SessionConfig, behavior: &str) -> Result<String>;
    /// 输入批次：有 on_wakeup / on_behavior_switch 模板就渲染，否则退回 DefaultAssembler 的 <session_input>。
    async fn render_input(&self, cfg: &SessionConfig, state: &SessionState, m: &InputMaterial) -> Result<Option<String>>;
    fn process_mode(&self, cfg: &SessionConfig, target: &str) -> Option<ProcessMode> {
        cfg.prompt.frozen.behaviors.get(target).map(|b| b.switch_mode).filter(|m| *m != ProcessMode::Normal)
    }
}

impl BehaviorConfig {
    /// 叠加到 prompt.llm_context 上，产生本 run 的有效 .llm_context（只触碰 G1 允许的键）。
    pub fn overlay_llm_context(&self, base: &Value) -> Value {
        merge(base, json!({
            "model": self.model.preferred,
            "loop_model": self.prompt.mode,                      // function_call | behavior
            "max_tool_iterations": self.budget.max_tool_iterations,
            "max_tokens": self.budget.max_completion_tokens,
            "tools": { "tools": whitelist_sources(&self.capabilities.tool_whitelist),
                       "actions": whitelist_sources(&self.capabilities.action_whitelist) }
        }))
    }
}
```

这就是 §3 表“配置来源”一格的变化：`prepare_hosted(workdir, behavior.overlay_llm_context(&cfg.prompt.llm_context), …)`。叠加结果进 run.json `config`，xllm 接手时不需要理解 behavior。`budget.max_wallclock_ms` / `max_total_tokens` 直接进 `hosted_request` 的 `BudgetSpec`（需要 `hosted_request` 接受 budget 覆盖，见 §11）。

### 6.5 渲染变量：哪些冻结、哪些新鲜

| 进 system（冻结 / 构造期确定） | 进输入批次（每批现算） |
|---|---|
| `identity.*`、`behavior.{name,objective,mode}`、`session.{id,kind,objective,driver,scope,parent}`、`paths.{session_root,workspace_root}`（相对或 binding 提供）、`workspace.id`、`xml_behavior_result_protocol`（由 xllm `runtime_protocol` 段提供） | `runtime.{clock_text,status}`、`<active_sessions>`、`<hints>`、`<events>`（pending_events 注入）、`<perceptions>`、`<inputs>`、`behavior_switch / process_result`、`session.current_todo*`（读 `todos.json`）、`notebook.last_items`、`workspace_list` |

opendan 的 prompt_env 在渲染时推进“上次看到”游标并 `flush_meta`，这是副作用；移植时游标改为 `state.json` 字段，由 `commit_input_batch` 与 receipt 一起提交，渲染保持纯函数（计划 §8.1 已定）。

---

## 7. Agent State 的获取：不配置

### 7.1 `AgentStateClient::connect`

```rust
pub enum StateLocator {
    /// xagent 作为库嵌在 OpenDAN 进程里：直接拿组件。
    InProcess(Arc<dyn AgentStateClient>),
    /// 本机能看到 AgentRoot：文件 + flock。
    AgentRoot(PathBuf),
    /// 跨进程 / 跨容器：OpenDAN 提供的 kRPC 服务。
    Krpc { endpoint: Url },
}

impl dyn AgentStateClient {
    /// 只需要 agent_did 与调用者身份；解析顺序 InProcess → AgentRoot → Krpc。
    /// hint 来自 CLI / 环境（OPENDAN_AGENT_ROOT、OPENDAN_AGENT_STATE_URL），没有就查询：
    ///   1) 本进程注册表（嵌入场景）；
    ///   2) agent_did → AgentRoot 的本机映射（system-config users/<owner>/agents/<id>.root，或 ~/.opendan/agents.toml）；
    ///   3) agent_did 的 DID 文档 / zone 服务发现 → kRPC endpoint。
    pub async fn connect(agent_did: &str, who: &str, hint: Option<StateLocator>) -> Result<Arc<dyn AgentStateClient>>;
}
```

- **权限**：文件版靠 OS 权限 + lease（驱动者单写）；kRPC 版靠 `who` 的 BuckyOS 会话 token 与 RBAC；进程内靠宿主。Runner 不感知差别，`who` 只在 `register / notebook_append / post_input` 这类带署名的写入里传递。
- **xagent v1 实现**：`InProcess`（给测试与嵌入）、`AgentRoot`（`FsAgentStateClient`）。`Krpc` 只定义 trait 对象的构造函数并用一个把调用转发到 `FsAgentStateClient` 的桩实现跑同一组 fixtures（E4），证明 Runner 不依赖实现。
- 与现状的差别：现在 `FsAgentStateClient::open(agent_root, did, …)` 要求显式路径；`connect` 把路径解析收进来，CLI 不再必须给 `--agent-root`。

### 7.2 Runner 里谁在读写 Agent State（现状，xagent 不变）

| 门面 | 读 | 写（驱动者） |
|---|---|---|
| `sessions()` | 登记门槛 `lookup`、每批 `me`、订阅的 session、`children_of`（新：子 session 状态，§4.14）、`scope_touching` | 每次提交后 `report_state`；创建时 `register` |
| `activity()` | 每批 `<active_sessions>`、观察边界的活动集合变化 | （活动摘要经 `report_state` 回报） |
| `perception()` | self_improve bootstrap 窗口、`last_seq` 补发 | perception 输入、`finish_run` 的 run_digest / task_outcome |
| `cognition()` | bootstrap 或有新 msg/event 时 `recall_hints` | self_improve 成功时 `commit_consolidation` |
| `artifacts()` | `head` / `version` | `register_version`、`decide` |
| `locks()` | — | `self_improve`、`artifact:<aid>` |
| `behaviors()`（新） | 冻结时 `identity / get / revision`；补冻结时 `get` | — |
| `runtime_grants()`（新） | 打开沙箱与每个观察边界 `list_active(sid)` | 不由 Runner 写；用户 / OpenDAN 经 `ctl grant` 签发 |

---

## 8. xagent 命令行

```text
xagent — drive an Agent Session for one Turn (or keep driving it)

  xagent new    --agent <did> --objective <text> [--class work|ui|self_improve|self_check] [--parent <dir>]
                [--behavior <name>] [--llm-context <json|@file>] [--runtime <id>] [--workspace <path>]
                [--subscribe <spec>]... [--msg <text>] [--until turn|finished|idle|outcomes:<n>] [--no-run]
                 按模板创建 session（冻结 behavior 与模板），默认立刻推进一个 Turn
  xagent run    <session_dir|sid> [--msg <text> | --msg-file <path> | --event <json>]
                [--until turn|finished|idle|outcomes:<n>] [--runtime <id>] [--no-bridge] [--detach-children]
                 投递（可选）后推进到 Turn 关闭；默认 --until turn；随后推进本进程拉起的子 session 到空闲（§4.12）
  xagent serve  <session_dir|sid>... [--idle-unload <secs>] [--no-bridge]
                 常驻：起事件桥；drive(Idle) → 等唤醒 → drive(Idle)…；同时接管所服务 session 的子 session
  xagent post   <sid> (--msg <text> | --event <json>)                 Agent 输入（AgentMessage / AgentEvent）
  xagent ctl    <sid> (stop | decide accept|discard | approve <ticket> | subscribe <spec> | unsubscribe <id> | activity ... | perceive <text>
                       | grant --runtime <id> --ttl <dur> [--paths ...] | revoke <grant_id>)
                 Session 控制协议
  xagent status <sid> [--worklog <n>] [--report] [--run] [--events]    状态；--events 显示 pending_events 与最近注入
  xagent list   --agent <did> [--active]
  xagent behaviors --agent <did> [--frozen <sid>]
  xagent xllm   <sid> [--run <id>]                                     打印让 xllm 接手 live run 的命令行

  <spec> = active|semi:session:<sid>[:watch=f1,f2] | active|semi:object:<id>#<event>

  在 exec 子进程内（PATH 上的 agent-session 即 xagent）：
  xagent ctl activity [--summary <t>] [--touch <ref>]... | ctl perceive <text>
  xagent recall <query> | note <text> | sessions [--active] [--children] | read-session <sid> | artifact ...
  xagent create-worksession --objective <t> [--report final|progress|none] [--wait] [...]   （§4.11）
  xagent wait <sid> | post <sid> --msg <t>                                                  （§4.13、§4.15）

身份与定位（都可用环境变量）：
  --agent <did>          [$OPENDAN_AGENT_DID]      --who <principal>       [$LIBOPENDAN_WHO，默认取 BuckyOS 运行时 app principal]
  --agent-root <dir>     [$OPENDAN_AGENT_ROOT]     --state-url <url>       [$OPENDAN_AGENT_STATE_URL]   （都是 connect 的 hint）
  --queue-dir <dir>      [$LIBOPENDAN_QUEUE_DIR]   开发用文件队列；不给则用 kmsg
```

`post` 与 `ctl` 分开，对应 §4 的两套协议；`ctl` 对无队列 session 只接受 `decide`（直接走 artifacts 门面）。

退出码（对齐 xllm 的分类）：

| 码 | 含义 |
|---|---|
| 0 | Turn 以 `completed` 关闭（或 `--until finished` 的 session 结束） |
| 1 | Turn 以 `failed` / `budget_exhausted` 关闭（含 work 模板下的 `WAIT_USER_MSG`），或 session 结束为 Failed |
| 2 | 参数 / 配置 / 模板 / 冻结校验错误，未推进 |
| 3 | 没有关闭 Turn：Turn 打开等输入（仅 `wait_user_msg = Allowed`）、等待工具、可恢复挂起；再 `run` 即续 |
| 4 | 被 stop / 中断 |
| 5 | Busy：session lease 或 run 锁被他人持有 |
| 6 | 阻塞：NotDriver / Unregistered / BindFailed / RuntimeMismatch / RecoveryBlocked |

`--format json` 时 stdout 是 `DriveResult` 的 JSON（含 `rev`、`turn`、`status`、`answer`、`waiting_for`）。

---

## 9. 核心伪代码（Rust 风格，目标设计）

约定：这是**目标设计**。与现有 `runner/drive.rs` 一致的地方不标；不一致的标 `// NEW`，并在 §11 的差距表里有对应行。省略错误类型细节、日志与统计；`?` 表示失败即按 §8 退出码返回；`commit!` 表示 state.json 原子替换（rev+1）并随后回报登记表。

### 9.1 组件与数据

```rust
pub struct AgentSession {
    dir: SessionDir,                      // <sid>/.opendan_agent_session/
    cfg: SessionConfig,                   // 含 prompt.frozen（behavior）与 session.policy（模板）   // NEW
    state: SessionState,                  // 提交点；含 pending_events                             // NEW
    lease: Arc<Lease>,
    bus: Option<Arc<dyn InputBus>>,       // 模板 input = None 时为空                               // NEW
    runs: RunStore,
}

pub struct Deps {
    who: Principal,
    agent: Arc<dyn AgentStateClient>,                 // connect() 得到
    runtime: Arc<dyn AgentRuntime>,                   // 沙箱：全部工具的执行出口                         // NEW
    guard: Arc<dyn ActionGuard>,                      // NEW Do 前安全检查（AllowAll | DenyList）
    xllm: XllmDeps,
    assembler: Arc<dyn SessionAssembler>,             // BehaviorAssembler
    tool_providers: Vec<Arc<dyn SessionToolProvider>>,// NEW 层 ③，注册给 Runtime、仍经 admit
    bridges: Vec<Arc<dyn EventBridge>>,               // NEW 进程内事件桥（kevent / timer）；--no-bridge 为空
    children: Arc<ChildDriver>,                       // NEW §4.12：推进本进程驱动的 session 所创建的子 session
    summarizer: Option<Arc<dyn Summarizer>>,
    options: RunnerOptions,
}

/// 一批准备进入上下文的材料：唤醒输入 + 待注入事件 + 新鲜量。
pub struct Batch {
    hook: &'static str,                    // on_init | on_wakeup | on_behavior_switch
    messages: Vec<(Envelope, AgentMessage)>,
    wake_events: Vec<(Envelope, AgentEvent)>,
    observe_events: Vec<AgentEvent>,       // 来自 state.pending_events（含内置 session bridge 的产出）// NEW
    hints: Vec<Hint>, active: Vec<ActiveSession>, runtime_status: Value, now_ms: u64,
}

pub struct LiveCtx { ctx: LLMContext, run: RunHandle, behavior_mode: bool, ready: bool, rounds: Arc<RoundCounter> }

pub struct Next {
    kind: NextKind,            // Done | Wait | ProcessDone | Switch(b) | Call(pending) | CallReturn(call_id, obs) | Budget | Error | Stopped | Interrupted | PendingTool | ContextLimit
                               // NEW §3.6：Call = 父 run 等待 subrun 时进入子上下文；CallReturn = 调用子 run 结束，交回调用方
    run_ended: bool, suspended: bool, finished: bool, waiting: bool,
    turn_end: Option<TurnStatus>, answer: Option<String>, error: Option<Value>,
}
```

### 9.2 xagent 入口

```rust
#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let who = cli.who.or_else(buckyos_app_principal)?;
    let agent = AgentStateClient::connect(&cli.agent_did, &who, cli.state_hint()).await?;      // NEW §7.1
    let buses = InputBusFactory::new(cli.queue_dir, buckyos_kmsg_client());
    let runtime = RuntimeRegistry::for_host().resolve(cli.runtime.as_deref())?;                // NEW §5.2
    let deps = Deps {
        who: who.clone(), agent: agent.clone(), runtime, xllm: XllmDeps::default().with_observer(cli_observer()),
        assembler: Arc::new(BehaviorAssembler::default()),                                      // NEW §6.4
        tool_providers: vec![],                                                                 // 层 ③ 暂无成员：fork 与工具子上下文属于 Context 调度（§3.5、§3.6）
        bridges: if cli.no_bridge { vec![] } else { vec![Arc::new(KeventBridge::new(buckyos_kevent_client())), Arc::new(TimerBridge)] },
        summarizer: None, options: RunnerOptions::default(),
    };
    install_self_as_session_cli(&deps);

    match cli.cmd {
        Cmd::New { class, spec, msg, until, no_run } => {
            let template = SessionTemplate::resolve(&class, agent.behaviors()).await?;         // NEW §4.7：内置 + agent.toml 覆盖
            let spec = template.apply(spec);                                                    // end_condition / input_policy / channels / policy / 默认 behavior
            let spec = freeze_behaviors(spec, agent.behaviors()).await?;                       // NEW §6.3
            let sd = create_session(&spec.parent, spec, agent.as_ref(), &who, &buses).await?;   // input = None 时不建队列   // NEW
            if let Some(text) = msg { sd.post_or_bootstrap(Message(text), &who, &buses).await?; }   // 无队列：并入 bootstrap 批次   // NEW
            if no_run { return ExitCode::SUCCESS; }
            drive_once(&sd, &deps, until.unwrap_or(StopWhen::TurnClosed)).await
        }
        Cmd::Run { target, input, until } => {
            let sd = locate_session(agent.as_ref(), &target).await?;
            if let Some(i) = input { sd.post_or_bootstrap(i, &who, &buses).await?; }
            drive_once(&sd, &deps, until.unwrap_or(StopWhen::TurnClosed)).await
        }
        Cmd::Serve { targets, idle_unload } => serve(targets, &deps, idle_unload).await,       // §9.6
        Cmd::Post { .. } => post_agent_input(cli, agent, &buses, &who).await,                  // Message | Event
        Cmd::Ctl  { .. } => post_control_or_direct(cli, agent, &buses, &who).await,            // 无队列 session：decide 直接走 artifacts
        Cmd::Status { .. } | Cmd::List { .. } | Cmd::Behaviors { .. } | Cmd::Xllm { .. } => read_only(cli, agent).await,
        _ => session_tool_subcommand(cli, agent, &who).await,                                  // 层 ② 工具
    }
}

async fn drive_once(sd: &SessionDir, deps: &Deps, until: StopWhen) -> ExitCode {
    let r = turn_loop::drive(sd, deps, until).await;
    print_result(&r);
    exit_code_of(&r)   // TurnClosed(completed)→0, TurnClosed(failed|budget)→1, TurnOpen|Idle→3, Stopped→4, Busy|RunBusy→5, NotDriver|Unregistered|BindFailed|RecoveryBlocked→6
}
```

### 9.3 Agent Turn Loop（核心）

```rust
pub async fn drive(sd: &SessionDir, deps: &Deps, until: StopWhen) -> DriveResult {
    // ── 0. 推进权与登记 ─────────────────────────────────────────────────────────
    let lease = match sd.acquire(deps.holder()) { Ok(l) => Arc::new(l), Err(NotDriver{driver}) => return NotDriver{driver}, Err(Busy{holder}) => return Busy{holder} };
    let mut s = AgentSession::load(sd, lease.clone(), deps)?;                     // schema 不符 → RecoveryBlocked；bus 按 channels 打开，可能为 None
    if deps.agent.sessions().lookup(s.sid()).await?.map(|e| e.location) != Some(s.dir.canonical()) { return Unregistered; }
    let _kind_lease = s.acquire_kind_lease(deps).await?;
    let _bridges = start_bridges(&s, deps).await?;                                 // NEW：按 s.cfg.subscriptions 订阅 kevent/timer，产出 AgentEvent 投 bus；bus 为 None 则不起

    // ── 1. 恢复：先对齐磁盘现场，再碰新输入 ─────────────────────────────────
    let reconciled = s.reconcile_runs(deps).await?;                                // 截 worklog 尾、回收 run、补 receipt、清门槛、重做终态 run 的结束
    s.confirm().await?; s.catch_up_reports(deps).await;
    let mut inbox = s.fetch().await?;                                              // Vec<(Envelope, SessionInput)>；bus 为 None → 空
    s.apply_controls(&mut inbox, InRun::No).await?;                                // 控制协议：stop / subscribe / activity / decide / perceive；每次 commit! 后 confirm
    s.absorb_observe_events(&mut inbox).await?;                                    // NEW：Observe 投递的事件进 state.pending_events（按 (sub, source) 取最新），commit!
    if s.state.run_state == Finished { s.reject_leftovers(inbox).await?; return Finished{..}; }

    // ── 2. 执行环境：首次推理前绑定、每次核验 ─────────────────────────────────
    s.ensure_frozen(deps).await?;                                                  // NEW：behavior 与模板未冻结且本进程能读目录 → 冻结
    let binding = match bind_or_verify(sd, &lease, deps.runtime.as_ref(), &s.cfg, &bin_plan_for(&s, deps), deps.app_tools()).await {
        Ok(b) => b, Err(e) => { s.state.last_error = Some(e.to_json()); commit!(s); return BindFailed{error: e.to_json()}; } };
    let sandbox = deps.runtime.open_session_env(&binding, &s.env_ctx(), &s.effective_tools(), provider_tools(&s, deps)).await?;   // NEW：沙箱 = ToolManager；含生效 grant
    let factory = ContextFactory::new(&s, deps, sandbox.clone(), lease.clone());
    let mut live: Option<LiveCtx> = match reconciled { Reconciled::Resume(run, snap) => Some(factory.resume(run, snap, ready = true).await?), _ => None };
    let (mut outcomes, mut turn_closed) = (0u64, None::<(u64, TurnStatus, Option<String>)>);

    // ── 3. 主循环：每圈 = 一个输入批次（可选）+ 一个 run 段 + 一个 Outcome ──────
    loop {
        lease.check()?;
        if s.state.stop_requested { return s.stop_session(live, deps).await; }
        if let StopWhen::MaxOutcomes{n} = until { if outcomes >= n { return OutcomesHandled{..}; } }

        // 3a. 组批：唤醒输入 + 待注入事件 + 新鲜量
        let (messages, wake_events) = inbox.take_agent_inputs(|e| s.resolve_delivery(e) == Wake);   // NEW：投递策略由 Session 决定
        s.builtin_session_bridge(deps).await?;                                                       // NEW：Session 来源订阅比 rev → AgentEvent → pending_events
        let hook = if !s.state.bootstrap_done { "on_init" } else if s.state.internal_continuation.is_some() { "on_behavior_switch" } else { "on_wakeup" };
        let batch = Batch {
            hook, messages, wake_events,
            observe_events: s.state.pending_events.drain_for_batch(deps.options.event_budget),       // NEW：terminal 优先，超预算的留到下次，不丢
            hints: if s.policy().load_hints && (hook == "on_init" || !messages.is_empty()) { deps.agent.cognition().recall_hints(&s.topic_query()).await? } else { vec![] },
            active: if s.policy().observe.includes_active() { deps.agent.activity().active(s.me().as_ref(), deps.options.active_sessions_limit).await? } else { vec![] },
            runtime_status: deps.runtime.status().await, now_ms: now_ms(),
        };
        let triggered = !batch.messages.is_empty() || !batch.wake_events.is_empty() || hook != "on_wakeup";
        let msg = if triggered { deps.assembler.render_input(&s.cfg, &s.state, &batch).await? } else { None };

        // 3b. 没有可推理的输入，也没有可续的 run → 等待或返回
        let resumable = live.as_ref().map_or(false, |l| l.ready);
        if msg.is_none() && !resumable {
            s.mark_waiting_for_input(); commit!(s);                                                  // pending_events 留在 state，不丢   // NEW
            match until {
                StopWhen::Idle => return Idle{..},
                StopWhen::MaxOutcomes{..} => return OutcomesHandled{..},
                StopWhen::TurnClosed => return match turn_closed {                                   // NEW
                    Some((turn, status, answer)) => TurnClosed{turn, status, answer, ..},
                    None => TurnOpen{turn: s.state.current_turn(), waiting_for: s.state.waiting_for.clone(), ..},
                },
                StopWhen::Finished => {
                    if s.state.last_error.is_some() { return Error{..}; }
                    if s.bus.is_none() { return TurnOpen{..}; }                                      // NEW：无队列 session 不会再有输入
                    if started.elapsed() >= deps.options.max_wait { return Idle{..}; }
                    s.wait(deps.options.poll_interval).await;
                    inbox = s.fetch().await?; s.apply_controls(&mut inbox, InRun::No).await?; s.absorb_observe_events(&mut inbox).await?;
                    if s.state.run_state == Finished { return Finished{..}; }
                    continue;
                }
            }
        }

        // 3c. 取 LLMContext：内存里的 live → state.live_run（重入 / fork 返回）→ 新建
        let mut lc = match live.take() {
            Some(l) => l,
            None if s.state.live_run.is_some() => factory.open_live_run(&mut s).await?,
            None => factory.new_run(&mut s).await?,                                                  // §9.4
        };

        // 3d. 提交输入批次：开启或并入 Turn（从不关闭）；receipt 同时清掉已注入的 pending_events
        if let Some(text) = msg { s.commit_input_batch(&mut lc, &batch, text).await?; }               // §9.5

        // 3e. 一个 run 段
        lc.ready = false;
        let outcome = factory.run_compacting(&mut s, &mut lc).await;
        let next = s.handle_outcome(&mut lc, outcome, &factory).await?;                              // §9.5；WAIT_USER_MSG 按模板解释；Context 调度（§3.3）   // NEW
        outcomes += 1;
        if let Some(status) = next.turn_end { turn_closed = Some((s.state.last_closed_turn(), status, next.answer.clone())); }
        if !next.run_ended && !next.suspended { live = Some(lc); }
        if next.kind == CallReturn { live = Some(factory.open_live_run(&mut s).await?); }            // NEW §3.6：父 run 立刻以 ToolResults 恢复（ready = true），不等输入

        // 3f. 退出判定
        if next.finished { return Finished{..}; }
        if next.error.is_some() { return Error{..}; }
        if until == StopWhen::TurnClosed && next.turn_end.is_some() { let (turn, status, answer) = turn_closed.take().unwrap(); return TurnClosed{turn, status, answer, ..}; }   // NEW
        if next.waiting && until == StopWhen::Idle { return Idle{..}; }

        inbox = s.fetch().await?; s.apply_controls(&mut inbox, InRun::No).await?; s.absorb_observe_events(&mut inbox).await?;
    }
}
```

要点：

- **Turn 边界只在两处改变**：`commit_input_batch`（开启或并入）与 `finish_run` / `stop_session`（关闭）。切换、fork、independent、工具子上下文的进入与返回、观察注入、挂起、重写、恢复都不碰 `open_turn`。
- **Context 调度发生在 3e**：`handle_outcome` 按 §3.3 的转移表决定下一个 context（同一 run 换段、子 run、回到父 run），下一圈 3c 取到的就是它；调度本身不消费输入。
- **投递策略在 3a 由 Session 决定**：同一条 AgentEvent，订阅是 active 就进 `wake_events`，是 semi 就进 `pending_events` 等观察边界或下一批。bridge 看不到这个区别。
- **`pending_events` 是状态**：空闲时留在 state.json，被同一 `(subscription, source)` 的新事件覆盖，不被丢弃；terminal 事件不被预算挤掉。
- **无队列 session**：`fetch` 为空，bridge 不起，等待分支直接返回；它的全部输入就是 bootstrap 批次（objective + `--msg`）。
- **每个 run 段一个 Outcome**，`outcomes` 只给 `MaxOutcomes` 用。

### 9.4 ContextFactory：恢复或新建一个 run 的 LLMContext

```rust
impl ContextFactory<'_> {
    /// 新 run：system（冻结 behavior）+ <session_history>；fork 子 run 再继承父 steps。
    async fn new_run(&self, s: &mut AgentSession) -> Result<LiveCtx> {
        let entry = s.state.current_behavior.clone().or(s.cfg.prompt.behavior.clone()).unwrap_or("main".into());
        let behavior = s.frozen_behavior(&entry, self.deps).await?;                      // NEW：prompt.frozen.behaviors[entry]；没有 → 补冻结 → 还没有 → RecoveryBlocked
        let system = self.deps.assembler.system_text(&s.cfg, &entry).await?;             // 身份 → 约束 → on_init 渲染 → objective；无新鲜量

        let llm_context = behavior.overlay_llm_context(&s.cfg.prompt.llm_context);       // NEW §6.4：model / loop_model / tools / limits
        let xdeps = self.deps.xllm.clone().with_skip_workdir_lock(true);
        let hosted = XllmTask::prepare_hosted_with_tools(&self.env.workdir, &llm_context, "session_config.prompt", &system, &xdeps, self.sandbox.clone())
            .with_protocol_flavor(HostProtocolFlavor::Session).await?;                   // NEW G3 / G7：xllm 只算有效配置与 system 段；工具由 Runtime 沙箱提供
        // self.sandbox = runtime.open_session_env(binding, ctx, &tools_cfg, provider_tools(s)) 在 drive 第 2 步打开：
        //   内置文件工具绑定沙箱 fs 视图、exec 走沙箱、生效 grant 的工具带前缀、层 ③ 工具注册其中；每个 dispatch 先 guard.check   // NEW §5
        let llm = counted(hosted.create_llm(&xdeps).await?, rounds.clone());

        let budget = s.cfg.prompt.history_budget_tokens.unwrap_or(self.deps.options.history_budget_tokens);
        let history = build_history(s, &self.lease, self.summarizer(&llm), budget).await?;   // summary.json + 反向读 worklog；不够先 compact

        let run_id = s.runs.create_locked()?;
        let record = hosted.new_record(&run_id, Some(s.runs.dir()), &s.cfg.session.objective,
            HostRunInfo { assembled_by: "libopendan", session_id: s.sid(), runtime_kind: self.binding.kind, runtime_id: self.binding.runtime_id,
                          env_check: self.env.check_digest(), extra: json!({ "behavior": entry, "renderer_opts": {"timestamps": false} }) });   // NEW G2
        let run = RunHandle::new(s.runs.clone(), record)?; run.write()?;

        let tools = Arc::new(SessionToolManager::new(self.sandbox.clone(), run.clone(), self.lease.clone(), self.touched.clone())   // 协议纪律在外，执行在沙箱   // NEW
            .with_deadline(behavior.budget.max_wallclock_ms));                            // 进 DoContext.deadline（G5）
        let mut input = vec![AiMessage::system(system)]; if let Some(h) = history { input.push(h); }
        let request = hosted_request(&hosted.config, ContextOwnerRef::Agent{session_id: s.sid()}, &run_id, &s.cfg.session.objective, &entry, input)
            .with_budget(behavior.budget.to_budget_spec());                              // NEW
        let waist = hosted_waist_deps(&hosted.config, llm, tools)
            .with_checkpoint_hook(Arc::new(SessionCheckpointHook::new(self.shared.clone(), run.clone(), hosted.config.loop_model == Behavior)));

        // 栈顶 frame 是 Fork / Call 且还没有 live run → 这是子 run：从父快照派生（§3.5、§3.6）。
        // Inherit::Transcript / None 的子 run 不带 <session_history>（上面的 history 不放进 input）。
        let mut ctx = match s.child_of() {                                                // NEW：Fork → Inherit::Steps；Call → spec.inherit
            Some((parent, inherit)) => LLMContext::resume(derive_child(&parent.latest_snapshot()?, inherit, request, host_meta(s, &entry))?, ResumeFill::ResumeFromMidRun, waist)?,
            None => LLMContext::new(request, waist).with_host_meta(host_meta(s, &entry)),
        };
        self.shared.set_interrupt(ctx.interrupt_handle());
        Ok(LiveCtx { ctx, run, behavior_mode: hosted.config.loop_model == Behavior, ready: false, rounds })
    }

    /// state.live_run 指向的未结束 run：拿锁、校验、停旧执行、物化在途、resume。
    async fn open_live_run(&self, s: &mut AgentSession) -> Result<LiveCtx> {
        let lr = s.state.live_run.clone().unwrap();
        let run = s.runs.lock(&lr.run_id).ok_or(RunBusy{run_id: lr.run_id.clone()})?;    // xllm 正在接手 → Busy
        let (record, mut snap) = run.load_checked()?;                                    // 版本 / 引用完整性；失败 → RecoveryBlocked
        self.runtime.stop_executions(&record).await?;
        if let Some(seq) = record.host_commit_pending { ensure!(lr.applied_input_seq >= seq); run.complete_host_commit()?; }
        let tools = self.sandbox.clone();                                                // 沙箱按 record.config.tools 已打开；xllm 的 rebuild_toolset 只在 native 沙箱内部使用   // NEW
        let llm = counted(create_run_llm(&record, &self.xdeps).await?, rounds.clone());
        if !record.inflight.is_empty() { snap = materialize_unresolved(&snap, &record.inflight, record.behavior_mode())?; run.checkpoint_with_results(&snap, None)?; }
        if snap.request.behavior_name != s.state.current_behavior { snap.request.behavior_name = s.state.current_behavior.clone(); }
        if let Some(pr) = &s.state.process_result { snap.bump_ids_after(pr); }
        let waist = hosted_waist_deps(&record.config, llm, Arc::new(SessionToolManager::new(..))).with_checkpoint_hook(..);
        let ctx = match snap.suspended {
            Some(Suspension::PendingTool) => match self.resolvers.poll(snap.pending()).await? {   // NEW §3.6：subrun 由 Session 自己解析（读子 run 的结束决定）；其它 kind 见 long-tool TODO §4
                Ready(obs) => LLMContext::resume(snap, ResumeFill::ToolResults(obs), waist)?,
                Running{..} => return Ok(LiveCtx::waiting(run)),                         // 其它 kind 的等待（job、task）；subrun 不会走到这里：父 run 回到 live 时子 run 已结束
                Unknown{reason} => return Err(RecoveryBlocked(reason)),
            },
            Some(Suspension::ContextLimit) => self.rewrite_for_limit(s, snap, waist, attempt = 1).await?,
            None => LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, waist)?,
        };
        run.set_status(RunStatus::Running)?;
        Ok(LiveCtx { ctx, run, behavior_mode: record.behavior_mode(), ready: true, rounds })
    }

    async fn run_compacting(&self, s: &mut AgentSession, lc: &mut LiveCtx) -> LLMContextOutcome {
        let mut attempts = 0;
        loop {
            match lc.ctx.run().await {
                LLMContextOutcome::ContextLimitReached{snapshot, ..} if attempts < MAX_LIMIT_COMPACTIONS => { attempts += 1; lc.ctx = self.rewrite_for_limit(s, snapshot, lc.deps(), attempts).await?; }
                o => return o,
            }
        }
    }
}
```

### 9.5 输入批次提交与 Outcome 处理

```rust
impl AgentSession {
    /// 开启或并入 Turn。提交顺序：①快照 ②run.json 门槛 ③state.json ④清门槛 ⑤确认输入。
    async fn commit_input_batch(&mut self, lc: &mut LiveCtx, batch: &Batch, text: String) -> Result<()> {
        let opens = self.state.open_turn.is_none();
        let turn = if opens { self.state.turn_seq + 1 } else { self.state.open_turn.as_ref().unwrap().index };
        let mut receipt = InputReceipt {
            run_id: lc.run.id(), input_seq: self.state.live_run_applied_seq() + 1, turn, opens_turn: opens, hook: batch.hook.into(),
            inputs: batch.messages.ids() ++ batch.wake_events.ids(),
            events: batch.observe_events.keys(),                                         // NEW：注入了哪些 pending_events（(sub, source, seq)）
            bootstrap: !self.state.bootstrap_done, after_step: lc.ctx.snapshot().next_step_index, continuation: self.state.internal_continuation.is_some(), .. };
        receipt.message_pos = lc.ctx.inject(Injection::user(text));                       // 消息与 receipt 在同一份快照
        lc.ctx.host_meta_mut().input_receipts.push(receipt.clone());
        lc.run.publish_input_checkpoint(&lc.ctx.snapshot(), receipt.input_seq)?;         // ①②
        apply_receipt(&mut self.state, &receipt)?;                                       // live_run.turns / open_turn|turn_seq / 消费位置 / bootstrap_done / 清 continuation；
                                                                                         // NEW：按 receipt.events 清 pending_events（seq 相同才清，新来的保留）
        self.state.run_state = Running; self.state.waiting_for = None; self.state.last_error = None;
        self.state.refresh_activity(&batch.messages, &self.cfg);
        commit!(self);                                                                   // ③
        lc.run.complete_host_commit()?;                                                  // ④
        self.confirm().await                                                             // ⑤（无队列 session 为空操作）
    }

    /// 一个 Outcome 结束一个 run 段；Turn 是否结束由 next.turn_end 决定。
    async fn handle_outcome(&mut self, lc: &mut LiveCtx, outcome: LLMContextOutcome, f: &ContextFactory<'_>) -> Result<Next> {
        let deps = f.deps;
        let snap = outcome.snapshot().cloned().unwrap_or_else(|| lc.ctx.snapshot());
        let child = self.child_mode(&lc.run);                                                       // NEW：本 run 是不是某个 frame 的子 run（Fork / Call）
        let mut next = match outcome {
            Done{behavior_result, response, ..} => {
                let nb = behavior_result.as_ref().and_then(|b| b.next_behavior.clone());
                let answer = behavior_result.self_report().or(response.text());
                let replied = snap.has_report() || !behavior_result.messages_to_send().is_empty();
                match (&child, nb.as_deref()) {
                    (Some(Call{call_id, ..}), nb) => Next::call_return(call_id, match nb {               // NEW §3.6：任何声明都返回调用方
                        Some(WAIT_USER_MSG) => Observation::error(json!({"kind": "needs_user_input", "question": answer})),
                        _ => Observation::ok(output_of(&behavior_result, &response)),
                    }),
                    (Some(Fork), nb) if nb != Some(WAIT_USER_MSG) => Next::process_done(answer),
                    (_, Some(WAIT_USER_MSG)) => match self.policy().wait_user_msg {                 // NEW：模板解释 WAIT_USER_MSG
                        WaitPolicy::Allowed         => Next::wait(answer, turn_end = replied.then_some(Completed)),   // 没交付回复 → Turn 保持打开
                        WaitPolicy::FinishFailed    => Next::failed(json!({"kind": "needs_user_input", "question": answer})),   // work：不许等人
                        WaitPolicy::FinishCompleted => self.decide_end(answer),
                    },
                    (_, Some(b)) if b != END && b != "done" => Next::switch(b),
                    _ => self.decide_end(answer),                                                   // Completed；按 end_condition（模板 turns）判 finished / waiting；
                                                                                                    // NEW §4.15：还有 report != none 的未结束子 session → 不结束，Turn 保持打开，waiting_for = Children
                }
            }
            BudgetExhausted{which, ..} => Next::budget(which),
            Error{error, ..} if is_retryable(&error) => Next::retryable(error),
            Error{error, ..} => Next::failed(error),
            Interrupted{..} if self.state.stop_requested => Next::stopped(),
            Interrupted{..} => Next::interrupted(),
            PendingTool{pending, ..} if pending.wait.source.kind == SUBRUN => Next::call(pending),   // NEW §3.6：进入工具子上下文
            PendingTool{..} => Next::pending_tool(),
            ContextLimitReached{..} => Next::context_limit(),
        };
        if let Some(Call{call_id, ..}) = &child {                                                   // NEW §3.6：调用子 run 的 Error / Budget 也交回调用方，Turn 不关闭；stop 不交回，整个 session 停
            if next.run_ended && !matches!(next.kind, CallReturn | Stopped) { next = Next::call_return(call_id, Observation::error(next.error.clone())); }
        }

        if next.run_ended { lc.run.checkpoint_finish(&snap, next.run_status(), &next)?; } else { lc.run.checkpoint_with_results(&snap, Some(next.run_status()))?; }
        lc.run.record_usage(outcome.usage(), lc.rounds.take())?; self.static_add_rounds(..);

        match &next.kind {
            NextKind::Switch(b) => match deps.assembler.process_mode(&self.cfg, b) {               // NEW：目标 behavior 的冻结 switch_mode
                None => self.switch_in_place(lc, b, snap.clone(), f).await?,                        // NEW §3.4：同一 run 开新段，换成 B 的 system / 工具 / 模型（现状只改 behavior_name）
                Some(mode) => { self.suspend_run(lc, b, mode, &snap).await?; next.suspended = true; }
            },
            NextKind::Call(p) => { self.enter_sub_context(lc, p, &snap, f).await?; next.suspended = true; }   // NEW §3.6
            _ if next.run_ended => self.finish_run(lc, &snap, &mut next, deps).await?,
            _ => { self.state.run_state = if next.kind == PendingTool { Waiting(tool) } else { Ready }; self.state.last_error = next.error.clone(); commit!(self); }
        }
        Ok(next)
    }

    /// 唯一关闭 Turn 的地方（stop_session 除外）。重做时幂等。
    async fn finish_run(&mut self, lc: &mut LiveCtx, snap: &LLMContextSnapshot, next: &mut Next, deps: &Deps) -> Result<()> {
        self.runtime_stop_executions(&lc.run).await?;
        let turn = self.state.current_turn();
        let mut entries = run_history_entries(&lc.run.id(), snap, FlushMarks::of(&self.state.live_run), turn);
        if let Some(status) = next.turn_end { if self.state.open_turn.is_some() { self.state.open_turn = None; if status == Completed { self.state.turns_completed += 1; } } }
        if next.finished { self.register_outputs(deps).await?; self.write_report(next.answer.as_deref())?; }
        entries.push(Outcome{run_id, turn, kind: next.kind.as_str(), next_behavior, report: next.answer.truncated(2000)});
        if let Some(status) = next.turn_end { entries.push(TurnEnded{run_id, turn, status, at_ms}); }
        self.worklog_append(entries)?;
        self.state.live_run = None; self.state.last_run = Some(lc.run.id());
        match next.kind {
            ProcessDone => { let f = self.state.process_stack.pop().unwrap(); self.state.live_run = Some(f.into_live()); self.state.current_behavior = Some(f.entry.clone()); self.state.internal_continuation = Some(f.entry); self.state.process_result = Some(json!({"behavior": .., "result": next.answer, ..})); }
            CallReturn(..) => { let f = self.state.process_stack.pop().unwrap(); self.state.live_run = Some(f.into_live()); self.state.current_behavior = Some(f.entry); self.state.internal_continuation = None; self.state.run_state = Ready; }
                                                                                         // NEW §3.6：结果不走交接批次；父 run 恢复时由 subrun resolver 读本 run 的 host.extra.finish，以 ToolResults 回填
            _ if next.finished => { self.state.run_state = Finished; self.state.process_stack.clear(); self.state.outcome = next.outcome(); self.state.acceptance = Pending; self.state.result = next.result(); }
            _ if next.waiting  => self.state.run_state = Waiting(input),
            _                  => self.state.run_state = Ready,
        }
        self.state.perception_seq += 1 + next.finished as u64;
        commit!(self);                                                                   // ← 提交点
        self.gc_previous_last_run(); lc.run.prune(deps.options.keep_snapshots);
        maybe_compact(self, deps).await;
        self.update_static(..);
        deps.agent.perception().append(&self.lease, self.sid(), digests(self, lc, turn, next)).await?;
        if self.is_self_improve() && next.finished { deps.agent.cognition().commit_consolidation(..).await?; }
        Ok(())
    }

    /// fork / independent：当前 run 挂起入栈，Turn 继续。
    async fn suspend_run(&mut self, lc: &mut LiveCtx, target: &str, mode: ProcessMode, snap: &LLMContextSnapshot) -> Result<()> {
        self.runtime_stop_executions(&lc.run).await?;
        self.worklog_append(run_history_entries(..) ++ [Outcome{kind: "suspended", next_behavior: target}])?;
        self.state.process_stack.push(ProcessFrame::from_live(&self.state.live_run, mode, snap));
        self.state.live_run = match (mode, self.state.process_stack.find_independent(target)) { (Independent, Some(frame)) => Some(frame.into_live()), _ => None };
        self.state.current_behavior = Some(target.into()); self.state.internal_continuation = Some(target.into()); self.state.run_state = Ready;
        lc.run.set_status(RunStatus::Paused)?;
        commit!(self)
    }
}
```

**观察边界（`SessionCheckpointHook`，每次推理前 / Step 边界）**：`lease.check` → 持久化工具结果快照 → `fetch` + `apply_controls(in_run)`（stop 则 interrupt）→ `absorb_observe_events`（NEW：Wake 事件留在队列不注入，Observe 事件进 `pending_events`）→ 内置 session bridge → 心跳合并 touching → 若 `policy.observe != Off` 且 `pending_events` 非空（或活动集合变化）→ 渲染 `<events>` 并以 `hook = "observation"`、`opens_turn = false` 的 receipt 注入，第二次回调时按同一 ①–⑤ 顺序提交并清掉对应 `pending_events`。消息与 Wake 事件永远不在观察边界注入。

### 9.6 常驻形态

```rust
async fn serve(targets: Vec<Target>, deps: &Deps, idle_unload: Duration) {
    join_all(targets.into_iter().map(|t| async move {
        let sd = locate_session(deps.agent.as_ref(), &t).await?;
        let bus = deps.buses.open_for(&sd)?;                                              // 无队列 session 不能 serve：直接 run
        loop {
            match drive(&sd, deps, StopWhen::Idle).await {                                 // drive 内部起 bridge，返回时停
                Finished{..} if !bus.has_pending().await? => break,
                NotDriver{..} | Unregistered | BindFailed{..} | RecoveryBlocked(..) => break,
                Busy{..} | RunBusy{..} | LeaseLost => bus.wait(BUSY_RETRY).await,
                _ => { if !bus.wait_until(idle_unload).await && !bus.has_pending().await? { break; } }
            }
        }
    })).await;
}
```

与计划附录 A.2 的 `Supervisor::session_loop` 一致：OpenDAN 日后托管就是把 `serve` 放进协程，bridge 换成 Supervisor 的全局事件桥。

---

## 10. 验证矩阵：xagent 要跑通的实验

每个实验都是一条脚本（python mock LLM，见 xllm_rust_sdk.md §9；不需要 BuckyOS），结果写进 `tests/xagent/`，并作为 `cargo test -p libopendan --test xagent` 的用例。**验收项**（E1、E4、E5、E13、E14、E17、E18、E19、E20、E21）失败说明分层有问题，先改设计；**回归项**失败说明实现有问题。

| # | 类别 | 验证的边界 | 步骤 | 判据 / 什么算设计问题 |
|---|---|---|---|---|
| E1 | 验收 | LLM Context ↔ Session | `xagent new --class work --msg …` 跑到 `--until outcomes:1`（mock 让 run 停在中途），`xagent xllm <sid>` 给出命令，`xllm --resume …` 跑完该 run，再 `xagent run <sid>` | xllm 接手不重装配；xagent 把 xllm 跑完的 run flush 进 worklog 并关闭 Turn；Round 两边累加。**设计问题**：任何需要 xllm 理解 Session 概念（behavior、模板、receipt）才能接手的地方 |
| E2 | 回归 | behavior 冻结 | 建 session 后改 `behaviors/plan.toml`；再推进两个 Turn；再建新 session | 旧 session 每个新 run 的 system 段 digest 相同、工具集不变；新 session 用新配置 |
| E3 | 回归 | Runtime 可替换、绑定不可换 | 同一 session_config 分别 native / tmux；已绑定 native 的再给 tmux | worklog 形状相同；换 runtime → RuntimeMismatch，退出码 6，无推理 |
| E4 | 验收 | Agent State 实现可替换 | 同一 fixtures 分别用 `FsAgentStateClient`、`InProcess`、转发桩 `Krpc` | 全部 expected.json 通过。**设计问题**：Runner 里任何按实现分支的代码，或某个门面只有文件版能实现 |
| E5 | 验收 | 工具形态 | a：只用层 ② 工具；b：注入一个层 ③ echo 工具 | a：xllm 可接手；b：`app_tools` 非空，xllm 明确拒绝。**设计问题**：某个 session 能力只能用层 ③ 实现 |
| E6 | 回归 | Turn 语义（ui 模板） | mock 先回 `WAIT_USER_MSG` 无 report；再 `post --msg`；mock 回 report | 第一次退出码 3、`open_turn` 保留；第二次输入并入同一 Turn（`input_batch`）；`turn_ended completed`，`turns_completed = 1` |
| E7 | 回归 | 崩溃恢复同一 Turn | `LIBOPENDAN_FAULT=input_batch:after_state_commit` 等故障点 abort 后再 `run` | 不重复注入、不倒退消费位置；Turn 编号不变；门槛期间 xllm 拒绝接手 |
| E8 | 回归 | 推进权 | 两个 xagent 同时 `run`；`serve` 期间另开 `run` | 后者 Busy（退出码 5） |
| E9 | 回归 | 输入形态等价 | 同一条消息经 `run --msg`、`post` 后 `run`、`serve` 中 `post` | `turn_started.inputs` / receipt 结构一致 |
| E10 | 回归 | 切换不断 Turn | plan →(fork) do → 返回 →(independent) review → END | 全程一个 Turn；`process_result` 注入父 run；independent 重入恢复同一 run |
| E11 | 回归 | 新鲜量不进 system | 同一 session 两个 Turn 间隔一段时间 | 两个 run 的 system 段字节相同；时间只在 `<session_input time=…>` |
| E12 | 回归 | 冻结缺失的阻塞 | 删 `prompt.frozen` 且目录不可读 | `RecoveryBlocked`，退出码 6 |
| E13 | 验收 | Session 模板：无队列 work session | `xagent new --class work --msg …`（无订阅）；mock 回 `WAIT_USER_MSG` | 不创建 kmsg 队列、`channels.inputs` 为空；一个 Turn 跑到结果；`WAIT_USER_MSG` → Turn `failed{needs_user_input}`，退出码 1。**设计问题**：Runner 某条路径假定队列存在（stop、decide、activity、perceive、父订阅） |
| E14 | 验收 | AgentEvent 投递策略与 semi 状态 | session 声明 `semi:object:X#changed` 与 `active:object:Y#fired`；用 `post --event` 在三个时刻投递：run 中、空闲中、Turn 之间 | semi 事件从不唤醒、只在观察边界或下一批首部出现（worklog `user_message hook=observation` 或 `turn_started.events`）；空闲时投的 semi 事件不丢，被同 key 新事件覆盖时记 `event_superseded`；active 事件开启新 Turn。**设计问题**：bridge 需要知道 active/semi 才能正确投递 |
| E15 | 回归 | 控制协议与输入分离 | `ctl stop` / `ctl subscribe` / `ctl activity` 在 run 中与空闲时各发一次 | 都不开 Turn、不进上下文；worklog 只有 `control_applied`；`post` 与 `ctl` 的 payload schema 互不接受 |
| E17 | 已验收 Runtime 统一入口 | native/tmux/SSH 的 exec 与 read/write/edit 使用同一 cwd；SSH 测试核验 Runner 本地同名文件未改写 | MCP/宿主工具位置如实保留；远端 Session helper 能力独立核验 |
| E18 | 验收 | Do 前统一检查与授权 | DenyList guard 拦 `write_file` 到 scope 外；`RequireApproval` 拦 `exec rm`，`ctl approve` 后恢复；签发 30s 的 host grant 后等它过期 | Deny 变成 Error 观察、LLM 改做法；审批期间 run 处于 PendingTool、`ctl approve` 后以 ToolResults 续跑同一 Turn；过期后 `host.*` 工具从列表消失、调用被 Deny、出现 `grant_changed` 事件 |
| E16 | 回归 | 内置 session bridge | 父（ui）半订阅子（work）；子结束 | 父的下一个观察边界或下一批里出现 `AgentEvent{source: Session, terminal: true}`，与外部 bridge 产出同形 |
| E19 | 验收 | 普通切换换配置；停止点交接（§3.4、G8） | a：`plan`（只读工具）normal 切到 `do`（可写工具、另一模型）；b：同一流程在切换前用 `xllm --resume` 接手，xllm 跑到 `next_behavior = do` 停下，再 `xagent run` | a：同一 run_id、同一 Turn；切换后请求的 system 段、工具广告、模型都是 `do` 的，run.json `config` 是 `do` 的有效配置，`plan` 的 steps 作为继承记录可见，工具额度不重置；b：xllm 停在跳转处且 run 不是终态，xagent reconcile 完成切换后续跑，worklog 与 a 同形。**设计问题**：切换需要 xllm 理解 behavior，或切换后的配置只存在于 Session 内存 |
| E20 | 验收 | 工具子上下文（§3.6） | 父 behavior 声明 `sub_context` 工具 `pick{reason}`（子 behavior `route`，`inherit = Transcript{recent: 4}`，output Json），父分别用 function_call 与 behavior 模式；mock 父 context 在同一批次里调用 `pick` 与 `read`；子 run 中途用 `LIBOPENDAN_FAULT` abort 一次；再换 `Steps` / `None` 与栈深超限 | 父 run 以 PendingTool 挂起，栈顶是 `Call` frame；子 run 在 `runs/` 里、进 worklog；崩溃后续跑同一个子 run；子 run 结束后父 run 以 ToolResults 恢复，同批的 `read` 接着执行；全程一个 Turn；父 run 挂起期间 xllm 接手被拒，子 run 可由 xllm 跑完；超限时工具返回 Error 观察。**设计问题**：需要进程内工具或 Runner 内存句柄才能实现，或结果只能经交接批次、不能作为工具结果交回 |
| E21 | 验收 | Sub Session 派出与汇总（§4.10–§4.16） | 无队列的 work 父 session：mock 父 LLM 用 `create-worksession` 派出子 A（`--report final`）、子 B（`--report progress`），再派出子 C（`--wait`）；C 返回后父 `END`；A 运行中 abort 父进程一次，再 `xagent run <parent>` | A、B、C 由 ChildDriver 并行推进，各持 lease、各有 Turn；B 的进度以 Observe 出现在父的观察边界；C 结束时父 run 以 ToolResults 恢复；父 `END` 时 A 未结束 → 父 Turn 保持打开（`waiting_for = Children`），A 的结束事件唤醒父并入同一 Turn，父汇总后 finished；父进程崩溃不影响子，重启后重新接管。**设计问题**：父收子的事件需要父有队列；子要写父的状态；bridge 要知道父的汇报方式才能投递；同步等待只能做成进程内工具 |
| E22 | 回归 | 父子对话与 stop 级联（§4.13、§4.16） | 子以 `--interactive` 创建，mock 子回 `WAIT_USER_MSG` 提问；父 `post` 回答；子主动 `post <parent>` 一次；最后 `ctl stop <parent>` | 父收到 `needs_input`（Wake），`post` 后子在同一 Turn 里继续；子的消息以 AgentMessage 唤醒父；父 stop 后未结束的子被级联 stop，登记表状态为 stopped |

---

## 11. 与现有 libopendan 的差距与改动清单

目标设计（§4、§9）与 `runner/drive.rs` 等现状的差距，就是实施清单。顺序按依赖排。

| # | 差距（现状 → 目标） | 位置 | 备注 |
|---|---|---|---|
| C1 | `Input` 五种 kind → `SessionInput::{Message(AgentMessage), Event(AgentEvent), Control}`；`change` 并入 Event + `Delivery`；`perception` 并入 Control；`Envelope` 显式化 | `protocol/input.rs` | Session Input Protocol 拆成"Agent 输入"与"Session 控制"两篇；schema 升版 |
| C2 | 投递策略由 kind 决定 → 由 Session 按订阅 / 模板解析（`resolve_delivery`）；`check_changes` 的 change 合并 → `state.pending_events`（按 `(sub, source)` 取最新，terminal 单列）；空闲时 `change_dropped` → 保留，覆盖记 `event_superseded`；receipt 增 `events` 并据此清除 | `drive.rs` 3a/3b、`hook.rs::boundary`、`receipts.rs`、`protocol/state.rs` | state.json 升 `session_state/3` |
| C3 | Session 来源订阅在 `check_changes` 内直接比 rev → 内置 bridge 产出 `AgentEvent{source: Session}` 走同一路径 | `hook.rs`、新 `runner/bridge.rs` | 外部 bridge trait `EventBridge` 同文件；kevent / timer 实现在 xagent |
| C4 | 所有 session 都建 kmsg 队列 → 模板决定（`InputChannel::None` 不建）；`bus: Option`；`fetch/confirm/wait` 对 None 为空操作；等待分支对无队列直接返回 | `api.rs::create_session`、`drive.rs` | 父订阅子时校验父有队列 |
| C5 | `SessionTemplate` 与 `session.policy{wait_user_msg, observe, load_hints}`；内置四模板 + `agent.toml [session.<class>]` 覆盖；`classify_done` 按 `wait_user_msg` 解释 `WAIT_USER_MSG` | 新 `protocol/template.rs`、`drive.rs::classify_done` | 与 C7 同一 schema 升版 |
| C6 | `StopWhen::TurnClosed`、`DriveResult::{TurnClosed, TurnOpen}` | `runner/mod.rs`、`drive.rs` 3b/3f | |
| C7 | `BehaviorCatalog` + `AgentStateClient::behaviors()`；`BehaviorConfig`（含 `meta.next`、`prompt.mode`、`switch_mode`）；`PromptSection.frozen`；`ensure_frozen` / 补冻结；`BehaviorAssembler`；`process_mode` 读冻结 switch_mode，删 `extensions.opendan.process_modes` | `state/behaviors.rs`、`protocol/behavior.rs`、`protocol/config.rs`、`runner/assembler.rs` | session_config 升 `/3`；现有 fixtures 中 process_modes 的两个用例改写 |
| C8 | 共享 Runtime、native/tmux/SSH、目标侧文件后端与绑定已完成；ActionGuard、ToolSpec.effect、RuntimeGrant、审批及其它执行体后移 | agent_tool/runtime、runner/tools.rs | 首版不冻结 policy 或 daemon 协议 |
| C9 | `AgentStateClient::connect` + `StateLocator`；`InProcessAgentState`；`Krpc` 构造函数 + 转发桩 | 新 `state/connect.rs` | 真 kRPC 随 OpenDAN 改造 |
| C10 | `RuntimeRegistry`；`ContainerRuntime` / `RemoteRuntime` 只有 descriptor | 新 `runtime/registry.rs` | |
| C11 | `examples/session.rs` → `src/bin/xagent.rs`（§8 命令面：`new/run/serve/post/ctl/status/...`）；`.runtime/bin/agent-session` 指向它；层 ② 子命令 | `bin/`、`runtime/bin_overlay.rs` | 旧 CLI 删除 |
| C12 | xllm prepare_hosted 已由 runtime.open 解析并派发工具（G7），resume 已校验实际目标与 Session env_check（G6）；HostProtocolFlavor、renderer_opts、budget 与 deferred 能力仍为后续 xagent 改动 | xllm.rs | Runtime 首版已完成 |
| C13 | 文档：readme 的 AgentRuntime / 命令行工具两段、protocol Spec（输入与控制两篇、`prompt.frozen`、`session.policy`、`pending_events`、behavior schema）、fixtures 重生成 | `doc/llm_context/readme.md`、`doc/opendan/protocol/` | 实现后反写（V6）；readme 的“在 AgentSession 中的 LLM Context 的状态机切换”一节按 §3.2–§3.6 重写 |
| C14 | Context 调度（§3.2–§3.7）：普通切换由只改 `behavior_name` 改为 `switch_in_place`（换 system / 工具 / 模型，run.json `config` 换段）；fork 子 run 的手工复制改为 `derive_child`；`ProcessFrame.mode` 增加 `Call{call_id, tool, args}`；`SubContextSpec` 冻结进 behavior；`SessionToolManager` 拦截 `sub_context` 工具、返回 `subrun` 等待；`handle_outcome` 增加 `Call` / `CallReturn`；Session 实现 `subrun` resolver；`session.policy.max_process_depth`；reconcile 完成 xllm 停在跳转处的转移（G8）以及做了一半的进入 / 返回 | `runner/outcome.rs`、`runner/live.rs`、`runner/tools.rs`、`runner/reconcile.rs`、`protocol/state.rs`、`protocol/behavior.rs` | 先做 [llm_context Context 调度支持 TODO](../../notepads/llm-context-switch-support-todo.md)；`subrun` resolver 依赖 long-tool TODO §4；state.json 与 session_config 随 C2 / C7 同一次升版 |
| C15 | Sub Session（§4.10–§4.16）：`create-worksession` 增加 `--report / --wait / --context / --interactive / --workspace / --runtime`，幂等键 `(父 sid, run_id, call_id)`，数量与深度校验；`origin.report`；删除 `create_session` 向父队列投 subscribe 的做法，改由内置 session bridge 按 `origin.parent_session` 查子（新 `sessions().children_of`）并按 §4.14 分类投递；`wait` 子命令与 `session` resolver；`decide_end` 的父结束规则与 `waiting_for = Children`；ChildDriver（`xagent run` / `serve`）；stop 级联；子默认 headless；冻结变量 `session.parent` | `api.rs`、`state/registry.rs`、`runner/bridge.rs`、`runner/outcome.rs`、`protocol/config.rs`、`bin/xagent.rs` | 与 C3（内置 bridge）改同一处；`session` resolver 依赖 long-tool TODO §4 的等待记录 |

依赖顺序：C1 → C2/C3 → C4/C5 → C6 → C7 → C14 → C8/C9/C10 → C11 → C15 → C12 → C13。验收项 E1/E4/E5/E13/E14 随 C11 落地；E13 依赖 C4/C5，E14 依赖 C1–C3，E19/E20 依赖 C14 与 llm_context TODO，E21/E22 依赖 C15。

---

## 12. 待确认

1. **冻结范围**（§6.3）：入口 + `meta.next` 闭包 + 按需补冻结，还是创建时冻结整个 behavior 目录？本文按前者。
2. **`loop_mode` 的归属**：移到 behavior 的 `prompt.mode`。普通切换时 `loop_model` 变化 → 视为 independent（parser / renderer 不能共用一个 run）。是否接受？
3. **work 模板下 `WAIT_USER_MSG` 的收尾**：本文定为 `FinishFailed`（Turn `failed{needs_user_input}`，问题进 report）。另一选项 `FinishCompleted`（把问题当结果交付）。
4. **无队列 work session 的后果**（§4.7）：stop 无法远程、decide 不经 session、子进程无 activity / perceive 通道。是否全部接受？若 perceive 必须保留，就得给无队列 session 一条只收控制命令的轻量通道，等于又建队列。
5. **控制协议是否继续搭输入队列**：本文保持同一条队列（多方投递、持久化现成）。另一选项是控制走 kRPC 直达驱动者，队列只放 Agent 输入；代价是驱动者不在线时控制命令丢失。
6. **AgentEvent.summary 上限与 data_ref**：1 KB 是否够；大 payload 一律 NamedStore 引用。
7. **层 ③ 工具的首个成员**：由 §3.6 回答——fork 与继承 context 的工具是 Session 的 Context 调度，不是进程内工具；层 ③ 暂无成员，E5b 继续用 echo 工具。请确认。
8. **xagent 放哪**：`libopendan` 的 `src/bin/xagent.rs`，或独立 crate `src/frame/xagent`。
9. **grant 工具的呈现**（§5.4）：带前缀的并列工具（`host.exec`），还是 grant 生效期间整体切换沙箱？前者可审计、LLM 需理解两个环境；后者简单但 workdir 必须两边都能看到。本文按前者。
10. **`ActionGuard` 对层 ② CLI 工具的粒度**：它们经 `exec` 进沙箱，guard 只看到命令行；是否需要在 `agent-session` 子命令内再做一次 Agent State 级检查？
11. **`AgentStateClient::connect` 的 did → AgentRoot 映射来源**：v1 用 `~/.opendan/agents.toml` + 环境变量，OpenDAN 改造时统一。
12. **independent 的 `END`**（§3.5）：按 libopendan（按 end_condition 收尾，frame 留着等显式跳回），还是按 opendan（结束即回到下层 process）？本文按前者。
13. **普通切换留在同一个 run**（§3.4）：本文按同一 run 开新段，run.json `config` 换成 B 的。另一选项是每次切换都开新 run（run.json 不可变），代价是 run 数膨胀，`<<step_history>>` 要跨 run 拼接。
14. **`Inherit` 的三档**（§3.6）：`Steps` / `Transcript{recent}` / `None` 是否够用；`Transcript` 的渲染放在 Session（模板可定制），还是用 llm_context 的固定格式。
15. **工具子上下文里的 `WAIT_USER_MSG`**：本文转成 `Error{needs_user_input}` 交回父 context，由父决定是否问用户。另一选项是让 Turn 停下等用户（父 run 继续挂起），那样子 run 就要能并入用户输入。
16. **栈深上限**（§3.3）：默认 4；超限时 T2 / T3 拒绝切换、当前 run 续跑，T4 返回 Error 观察。是否接受？
17. **子 session 的 workspace 默认值**（§4.11）：默认共用父的（`inherit`，靠活动视图避让），还是默认新建（隔离干净，但产物要合并回父）？本文按前者。
18. **父结束规则**（§4.15）：还有需要汇报的未结束子 session 时，父 Turn 保持打开等它们（本文）；另一选项是允许父结束，子的汇报只留在登记表。
19. **父对子的关注改为隐式**（§4.11）：删除 `create_session` 向父队列投 subscribe 的现有做法，改由内置 bridge 查登记表。是否接受？
20. **`xagent run` 是否默认推进子 session**（§4.12）：本文默认在父 Turn 关闭后继续推进到子空闲，`--detach-children` 立即返回。
21. **子 session 的“用户”是父**（§4.13）：子默认 headless、不直接对人发消息。是否要允许某些子（如长期 work）直接面向用户？
22. **数量与深度默认值**（§4.16）：`max_sub_sessions = 4`、`max_session_depth = 2`。
