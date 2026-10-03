# xAgent：Agent Session 分层验证工具设计

- 状态：实施设计 v0.3（2026-10-03）；按已完成的 Input Bus / Turn Loop 更新基线，xagent CLI 与其配套扩展待实施。已实现、待接线和后移项分别见 §11。
- 位置：`src/frame/lib_opendan`（二进制 `xagent`，替代 `examples/session.rs`）
- 依据：[Agent Session SDK 实现计划](<./Agent Session SDK 实现计划.md>) v0.10、[长任务与执行体 RFC](<./OpenDAN Long Task & Sub-Agent.md>)、[LLM Context readme](../llm_context/readme.md)、[xllm Rust SDK 参考](../llm_context/xllm_rust_sdk.md)、`doc/opendan/protocol/`
- 读者：实施 xagent CLI 的开发者与 Code Agent；实现基线为 `5ef122a2`（输入与 Turn Loop）及 `b9d7fbbe`（媒体降级与实施记录补充），并对照当前仓库的 `libopendan` / `agent_tool` 源码。实施时先检查基线之后的相关变更。
- 输入设计 Review：2026-10-03，确定三类受控输入 `on_init / on_input / on_context_switch` 与输入前装配的 `semi_subscription_snapshot`（半订阅快照）；模板、装配及提交规则见 §6.2 / §6.4 / §9.5，实施清单见 [输入与 Turn Loop TODO](../../notepads/lib-opendan-input-and-turn-loop-todo.md)。

---

## 0. 结论先行

xagent 的目的是**在一个新产品里验证四层架构、发现设计问题并迭代**。CLI 复用 `libopendan::runner::drive`，在库内补齐模板、冻结、Turn 返回条件与 Sub Session；不在二进制里复制一套 Runner。本文的伪代码（§9）描述阶段和提交边界，实际类型与函数以源码为准，扩展范围列在 §11。凡是实验（§10）暴露出设计问题的，先修订设计与协议，再改代码和 fixtures。

**Agent Session 与 LLMContext 的根本区别是 Context 调度**（§3.2–§3.7）：LLMContext 跑到 `next_behavior`（或一个挂起点）就返回；下一个 context 从哪份快照起、用哪套配置、结果怎么交回，全部由 Session 决定，进入方式看**目标 behavior** 的进入模式。SWITCH_CONTEXT、create-sub-context、fork、工具触发的子调用、改写、Turn 边界新建 run 是同一张转移表上的几行；“同一 run 换 system”的普通切换已废弃。继承 context 的工具不做成进程内工具，而是由工具调用触发的子调用，结果经 PendingTool 交回。

**Sub Session 是 SDK 化的主要用途之一**（§4.10–§4.16）：创建子 session 是一个层 ② 标准工具；父子各写各的 state，只经登记表、输入通道与控制协议沟通。进展以登记表为真相：父可以随时拉取，也可以按创建时选的汇报方式收到事件（进度更新半订阅状态，需要关注和结束进入受控输入）；子 LLM 也能主动给父发消息。等待分异步（默认）与同步（Pending + `session` resolver）两种；父结束前会等需要汇报的子 session。

1. **xagent 是 xllm 的上一层**：xllm 加载 `.llm_context`，把一个 LLMContext run 推进到一个 Outcome；xagent 加载（或创建）一个 Agent Session，把它推进到**一个 Turn 关闭**，或常驻地不断完成 Turn。两者的 run 目录相同（`runs/` 就是 xllm 的 run 目录），同一个 run 可以在两者之间交接，这是验证 L2 / L3 边界的主要手段。
2. **Agent Session 构造 llm_context 复用 xllm 的宿主装配 API**（`XllmTask::prepare_hosted` / `hosted_request` / `hosted_waist_deps` / `rebuild_toolset` / `create_run_llm` / `RunStore`），但 **system 段、历史段、输入批次、工具调度包装、checkpoint 钩子、run 生命周期都由 Session 决定**；差异清单见 §3。
3. **Agent 感知到的输入只有两种：消息（MsgObject）与 `AgentEvent`**（§4）。Session 不关心它们怎么来的，只要求信封（key、来源与 index、from、at_ms、subscription_id）；把系统事件（msg-center、kevent、timer、task_mgr、子 session）翻译成这两种输入的是上层 **bridge**（xagent serve、以后的 OpenDAN Supervisor、应用）。事件进入受控输入还是更新半订阅状态，由 **Session 按自己的订阅配置决定**，不由 producer 决定；半订阅状态按 `(subscription_id, source)` 合并并持久化，在受控输入使用前渲染为快照，空闲时不丢。stop / decide / subscribe / activity / perceive 不是 Agent 输入，是**Session 控制协议**，只是搭同一条队列。
4. **Session 模板**（§4.7）决定一个 session 的形态：Turn 上限、`WAIT_USER_MSG` 的含义、要不要输入队列、半订阅快照包含哪些材料、hints、默认 behavior。work 模板 = 一个 Turn、不等用户、默认不建队列；ui 模板 = 无限 Turn、有队列。模板是 `SessionSpec` 的预设，创建时解析进 session_config，不是新协议对象。
5. **Runtime 接管全部 agent-tool**（§5）：共享 AgentRuntime/Sandbox 已在 agent_tool 实现 native/tmux/remote_ssh、文件后端、环境与执行跟踪。Session 保留 lease、门槛、inflight、receipt、bin/helper 和绑定。ActionGuard、grant 与审批为后续 policy 设计，不属于已完成首版。
6. **behavior 配置来自 Agent State，在 Session 构造时冻结进 `session_config.prompt`**（§6）；进入模式（`switch_context` / `create_sub_context` / `fork`）由**目标 behavior** 的冻结配置决定，没有缺省回退。
7. **Agent State 不配置**（§7）：`AgentStateClient::connect(agent_did, who)` 按"进程内 → 本机 AgentRoot → kRPC"解析；Runner 只依赖 trait。
8. **验收项**是 E1（xllm 接手）、E4（换 Agent State 实现）、E5（工具形态）、E13（无队列 work session）、E14（半订阅注入时机）、E19（SWITCH_CONTEXT）、E20（工具触发的子调用）、E21（Sub Session）及 E24–E28 的输入 / 等待 / CLI 验证。E17 是已有 Runtime 基线；E18 随 policy 后移，不阻塞本轮 CLI。库级测试通过与 xagent CLI 验收分别记录（§10）。

---

## 1. 目标、范围与要验证的分层

### 1.1 目标

用一个独立进程（不依赖 OpenDAN 服务）把下面四层各自的边界跑通，并能用实验证明边界成立（§10）：

| 层 | 组件 | 回答的问题 | xagent 要证明的事 |
|---|---|---|---|
| LLM Context | `llm_context` + xllm（`agent_tool::xllm`） | 一次上下文推理循环：Round / Step、工具、快照、resume | Session 装配的 run 可以被 xllm 原样接手 / 跑完；Session 不重新实现 Loop |
| Agent Session | `libopendan`：目录协议 + Runner | 以某个 Agent 的身份，把一次逻辑 Input 推进到 result（Turn）；**Turn 内调度多个 LLMContext**（SWITCH_CONTEXT、create-sub-context、fork、工具触发的子调用）；**创建与协调 Sub Session** | Turn 的开启 / 并入 / 关闭只由 Session 决定；LLMContext 停在 `next_behavior`，之后的转移只由 Session 决定；三种输入形态语义一致；崩溃后恢复同一 Turn |
| Agent Runtime | `agent_tool::runtime` | 工具在哪里、以什么 PATH / cwd / 环境运行；后台进程的识别与停止 | 同一 session_config 可绑定不同 runtime；绑定后不可换；工具集与 runtime 解耦 |
| Agent State | `libopendan::state` | 跨 Session 的 Agent 状态：登记表、活动视图、感知、认知、产物、**behavior 目录** | Runner 只依赖 `AgentStateClient`；文件实现 / 进程内实现 / kRPC 桩行为一致；behavior 冻结后不受更新影响 |

### 1.2 范围

- 做：work / self_improve / self_check session；native 与 tmux runtime；文件版与进程内 Agent State；behavior 冻结与 `BehaviorAssembler`；CLI 形态 session 工具；单 Turn / 积压消费 / 常驻三种运行形态；Sub Session（创建工具、ChildDriver、汇报与等待，§4.10–§4.16）。
- 不做（沿用计划的后移项）：正式 UI session 与 msg-center / TaskMgr 服务接入、出站消息发送、kRPC 服务端、DID Object 宿主、opendan 改造、TS 版；Sub-Agent Instance（另一个 Agent DID，RFC §16–§17）；ActionGuard / grant / 审批（§5.4、E18）。ui 模板仅用于库与 CLI 的等待语义测试。
- 沿用现有目录协议（含已实现的 `post.lock`）；新增持久字段落在 `session_config.json`（`prompt`、`session.policy`、`origin` 等）与 `state.json`。当前版本见 §1.5，后续不兼容变更再升版，不沿用旧草案的版本号。

### 1.3 与 xllm 的对位

```text
xllm   : .llm_context ──prepare──► LLMContextRequest + Deps ──run──► Outcome      （一个 run，到停止点就结束）
xagent : AgentSession ──InputBus──► 输入批次 ──commit──► run ──Outcome──► Context 调度（§3.2）──┬─► 下一个 context（同一 Turn）
                 ▲                                                                            └─► Turn 关闭
                 └──── Agent State（behavior、身份、登记表、活动、认知）      （一个 Turn，可含多个 run、多个 behavior）
```

### 1.4 验证方式：目标设计优先

- §9 的伪代码是目标设计；它与现有 `drive.rs` 的差距逐条列在 §11，实施就是消差距。
- 每个实验（§10）都写明"验证哪条边界、什么现象算设计问题"。发现设计问题时的处理顺序：改本文 → 改协议 Spec（`doc/opendan/protocol/`）→ 改代码 → 重生成 fixtures。不允许为了让实验通过往 Runner 里加特例。
- xagent 按 §8 实现命令面，复用 `examples/session.rs` 的构造 helper、身份环境和 bin 包装逻辑；旧 CLI 的 schema 导出与工具子命令迁入后，更新调用它的测试、脚本和文档，再删除旧入口。

### 1.5 实施基线与复用入口

| 已有能力 | 直接复用的入口 |
|---|---|
| `session_input/3`、`session_config/4`、`session_state/5`；binding `/3`、RunRecord v5、快照 v4 | `protocol/{input,config,state}.rs`、[协议索引](protocol/README.md)；类型与版本常量优先于旧 README 的摘要 |
| 顺序路由、拒绝、去重、active 接受状态、Observe 合并、session 拉取订阅 | `runner/inputs.rs::route_inputs`；不再拆成整批 controls 与整批 events 两遍处理 |
| InputView、命名格式、三类模板、媒体块、receipt 与回复来路 | `runner/{input_view,assembler,live,receipts,flush}.rs`；黄金结果在 `protocol/fixtures/14_input_bus/` |
| 普通 task 串行等待、Unknown 回填、后台 task 跟踪、长工具 stop | `runner/shared.rs::{Opened,WaitingRun}`、`live.rs::try_fill`、`drive.rs::{poll_watched_tasks,StopMonitor}` |
| Context 调度、子 context 工具调用、崩溃恢复、Runtime 绑定 | `runner/{live,outcome,reconcile,tools}.rs`、共享 `agent_tool::runtime` |
| CLI 可用基础 | `api::{create_session,post_input,read_session}`、`RunnerDeps`、`SessionRunner`、`examples/session.rs` |

`BehaviorCatalog`、behavior 冻结、SessionTemplate、`StopWhen::TurnClosed`、ChildDriver 与 `session` resolver **尚未实现**。本文中的 `AgentSession` / `ContextFactory` / `InputBus` 是职责示意，不要求创建同名包装层；现有 `Session`、`Shared`、`InputChannelFactory` 等能承载时直接扩展。§12 有默认选择的条目按该选择实施；明确未定且后移的能力不进入本轮命令面。

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
 │     Runtime = Sandbox：工具统一派发；guard / grant 后移 │
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
| waist deps | `LLMContextDeps` + xllm `SnapshotHook`（InferenceHook） | `hosted_waist_deps`（behavior 时装 `XllmActionParser` + 无时间戳 `XmlStepRenderer`）+ `SessionCheckpointHook`（异步 CheckpointHook：已改为发布工具结果、顺序路由和刷新心跳；检查点只保存半订阅更新，注入在受控输入之前，见 §9.5）。**不用 InferenceHook** |
| run 目录 | `RunStore` 在 `.llm_context` 的 runs_dir | 同一 `RunStore`，目录是 `<sid>/.opendan_agent_session/runs/`；RunRecord 多了 `host{assembled_by, session_id, runtime_kind, env_check}`、`host_commit_pending`、`inflight`、`executions`；`host.extra.finish` 存结束决定 |
| 工作目录锁 | `<lock_dir>/<hash(workdir)>.lock` | `skip_workdir_lock = true`，多 session 共享 workspace 靠活动视图避让 |
| run 结束 | 终态写 run.json，结果导出 | `handle_context_outcome` → `finish_run`：flush 历史进 worklog、关闭 Turn、提交 state.json、登记表回报、感知 digest |
| 停止点之后 | xllm 结束；`next_behavior` 只留在快照与 run.json 里 | **Context 调度**（§3.2）：进入目标自己的 run、建子 run、回到调用方，或关闭 Turn |
| 接手 | — | 保存的 runtime/target/cwd、helper 环境核验通过，且无 `host_commit_pending`、无进程内专有工具时，`xllm --resume --run <id> --runs-dir <sid>/.opendan_agent_session/runs --dir <workdir>` 可接手 |

xagent 不改变这张表的分工，只在“配置来源”和“system 段”两格引入 behavior 冻结（§6），在“ToolManager”一格把执行权交给 Runtime 沙箱（§5）。这张表只讲一个 run 怎么构造；run 停下之后下一个 context 怎么来，是 §3.2–§3.7 的 Context 调度。

### 3.1 已知的边界缺口（xagent 要用实验暴露，实施时一并修）

| # | 现状 | 影响 | 处理 |
|---|---|---|---|
| G1 | `prepare_hosted` 只用 `llm_context` JSON 的 provider / model / limits / tools / `loop_model`，`prompt.sections`、`prompt.system`、groups 的 section、`runs_dir` 被解析但忽略 | behavior 叠加只能落在这几个键上，正好够用；但 `prompt.llm_context` 里写了 sections 会静默无效 | xagent 在冻结时校验并报错；§6.4 的 overlay 只产生这几个键 |
| G2 | `hosted_waist_deps` 用无时间戳 `XmlStepRenderer`，而 `xllm --resume` 接手后重建 deps 时用带时间戳的渲染器 | 接手后历史渲染字节变化，只影响前缀缓存，不影响正确性 | run.json `host` 里记 `renderer_opts`，xllm resume 时沿用（E1 实验会观察到） |
| G3 | `build_runtime_protocol` 的开场白是“你在 xllm 一次性任务里运行，没有后续对话，不要向用户提问”，hosted 也原样追加 | 与 Session 的 `WAIT_USER_MSG` 语义冲突 | `prepare_hosted` 增加 `HostProtocolFlavor::Session`，由宿主给出 runtime_protocol 的开场白 |
| G4 | xllm action 解析把“无动作 + `<report>`”映射为 `next_behavior = "done"`，不是 `END`；依赖 `forbid_next_behavior = false` | libopendan 已按“done 即交付”处理（`classify_done`），子 context 的 run 不设 forbid | 保持；文档化到协议 Spec |
| G5 | Runtime 已提供 exec 请求超时、执行记录与取消停止核验 | Session 的整体 behavior deadline 仍属于后续 xagent 预算接线 | 保留 Session stop/lease 中断；整体预算经宿主接入，policy 不作为首版前置 |
| G6 | 已修：run 保存实际目标与 Session env_check | xllm 接管核验 PATH、环境、manifest 与 helper 内容 | 凭据重新读取环境引用；依赖缺失/变化则阻塞恢复 |
| G7 | 已修：prepare/prepare_hosted 调用共享 runtime.open | 内置文件工具使用目标侧后端，与 exec 共用 cwd | native/tmux/SSH 已验证；远端 Session helper 缺失时报 Capability |
| G8 | 已修：hosted run 的 `Done` 指向另一个 behavior 时，xllm 在 run.json（v5）记 `handover {next_behavior, at_ms}` 并停在交接点（`paused`），`xllm --resume` 拒绝继续它 | 交接点可以交接：xllm 跑到交接点，目标不会丢，Turn 也不会被误关闭 | Session reconcile 按 §3.3 把转移恰好提交一次（E19）；见 §3.7 |

### 3.2 Context 调度：Agent Session 与 LLMContext 的根本区别

LLMContext 的契约只到一个停止点：`run()` 返回一个 Outcome。Behavior 模式下，LLM 用 `<next_behavior>` 声明下一步（`END`、`WAIT_USER_MSG`、xllm 的 `done`，或一个 behavior 名），`run()` 随即以 `Done` 返回。LLMContext 不知道目标 behavior 的配置，也不知道还有别的 context、Turn 和输入队列。function_call 模式没有 `next_behavior`，`Done` 就是最终回答。

所以**下一个 context 由谁跑、从哪份快照起、用哪套配置、结果怎么交回，全部由 Agent Session 决定**。本文把这部分称为 Context 调度（Context Switch），它是 Session 比 LLMContext 多出来的核心能力。约束：

- **串行**：同一 Session 任一时刻只推进一个 context（live run），其余的以 `ProcessFrame` 挂在 `state.process_stack` 上，或已结束。要并行就用子 Session（readme 的 AgentSession Tree），不在一个 Session 里同时推进两个 run。
- **Turn 延续**：除 T6（Turn 边界新建 run）外，所有转移都延续当前 Turn（§4.9）。
- **每次转移都是一次提交**：转移写进 state.json（`live_run`、`process_stack`、`current_behavior`、`internal_continuation`），worklog 记一条；崩溃后 `reconcile_runs` 能把做了一半的转移做完，且只做一次。
- **进入方式由目标 behavior 决定**：`next_behavior = B` 或 `call_behavior` 指向 B 时，Session 读 B 冻结的进入模式（§6.2），只有三种：`switch_context`（SWITCH_CONTEXT）、`create_sub_context`（create-sub-context）、`fork`。没有 Session 级的统一模式，也没有缺省回退：目标没有声明进入模式是配置错误。
- **一个 run 只有一套 system 与配置**：不在同一个 run 里换成另一个 behavior 的 system / 工具 / 模型（旧“普通切换”已废弃，§3.4）。换 behavior 就是换 run：进入目标自己的 run，或为它新建子 run。
- **停止点就是交接点**：xllm 接手的 hosted run 跑到 `next_behavior = B` 就在交接点让出（run.json 记 `handover`，状态 `paused`），转移由下一次 `xagent run` 的 reconcile 提交一次（G8；E19 验证）。xllm 不需要理解任何调度概念。
- **llm_context 只提供构造原语**：`build_fresh`、`derive_child` / `fork_snapshot`（`llm_context::context_derive`，§3.7）、`ResumeFill`（含历史重写）。用哪个原语、什么时候用，是 Session 的事。

名词：**process** = 有自己 run 目录与 step 流的一个 context，即一个 run；**frame** = 挂在栈上的 process，`role` 为 `parked`（经 SWITCH_CONTEXT 离开、等着被再次进入）或 `caller`（子 context 的调用方、等子结果）；**子 context** = 由 create-sub-context 或 fork 构造、结果交回调用方的 run；**run 段** = 同一个 run 的一次 `run()` 到下一个 Outcome（§9.3），段与段之间 system 与配置不变。

### 3.3 转移表

| # | 转移 | 触发 | 新 context 从哪来 | system 与配置 | 计数 | run 与栈 | 返回 |
|---|---|---|---|---|---|---|---|
| T0 | 续跑 | 受控输入批次（可带半订阅快照）、可恢复挂起之后 | 当前 context | 不换 | 保留 | 同一 run | — |
| T1 | ~~普通切换~~（**废弃**） | — | 不再存在：不能在同一 run 里换成另一 behavior 的 system / 配置，也没有 normal 缺省回退；目标未声明进入模式是配置错误（§3.4） | — | — | — | — |
| T2 | 子 context：**create-sub-context** 或 **fork**（behavior 触发） | `next_behavior = B`，B 的进入模式为 `create_sub_context` / `fork` | create-sub-context：新 run = B 的 system + 任务输入 + 显式选择的父历史（`inherit`：none / recent_dialogue / steps）；fork：新 run = 父的 system + 分叉点的完整有效历史 + 分支任务输入 | create-sub-context：B 自己的；fork：与父相同 | 子独立（budget、usage、错误计数从头）；Step / action 编号接续父 | 新 run；父 run 入栈 `caller`，`call.trigger = behavior` | 子 run 结束 → `process_result` 交接批次注入父 run |
| T3 | **SWITCH_CONTEXT** | `next_behavior = B`，B 的进入模式为 `switch_context` | 栈里 B 自己 parked 的 run（恢复它自己的快照）；没有就新建：B 的 system + 交接输入，历史按 B 的 `inherit`（none / recent_dialogue） | B 自己的 | B 自己的 | 当前 run 入栈 `parked`；B 有 parked 的 run 就出栈成为 live | 不隐式返回：回 A 要显式切换；`END` 按 Session 结束条件收尾 |
| T4 | **工具触发的子调用** | 调用 `call_behavior({behavior, task})` | 同 T2：由目标的进入模式决定是 create-sub-context 还是 fork；fork 的分叉点在触发批次之前 | 同 T2 | 同 T2 | 新 run；父 run 以 PendingTool 挂起（task id `subctx:<call_id>`），入栈 `caller`，`call.trigger = tool{call_id, task_id}` | 子 run 结束 → 父 run 以 `ToolResults{call_id}` 恢复 |
| T5 | 改写 | `ContextLimitReached` | 同一 run，历史重写为 summary + `<session_history>` | 不换 | 保留 | 同一 run，epoch + 1 | — |
| T6 | Turn 边界 | 上一个 run 已结束，又来了输入 | 新 run：当前 behavior 的 system + `<session_history>` | 当前 behavior 的 | 新 | 新 run | — |

- T2 / T3 由 `next_behavior` 触发，只出现在 Behavior 模式；T4 两种模式都能用，也是 function_call 模式（如 opendan 现在的 ui session）唯一的调度手段。
- **构造方式与触发方式独立**：子 context 是哪一种（create-sub-context / fork）只看目标 behavior 的进入模式；T2 与 T4 的区别只在触发和返回协议。旧 T2 的“用 B 的 system、继承父 steps”就是 `create_sub_context` + `inherit = steps`。
- **子 context 的任何结束都返回调用方**，结果带 `status: ok | failed | needs_user_input`：`END` / `done` → ok；Error / BudgetExhausted → failed（对调用方是一次失败的结果，不是失败的 Turn）；`WAIT_USER_MSG` → needs_user_input（子 context 从不自己等用户，也不消费调用方的输入）；声明切换到一个 `switch_context` 目标也按 ok 返回（子 context 不离开自己的调用）。这由 Session 在 `classify_done` 里处理，不用 `forbid_next_behavior`（原因见 G4）。子 context 可以再调用子 context。
- **嵌套深度**：栈上的 `caller` frame 不超过 `session.policy.max_process_depth`（默认 4）；`parked` frame 不计入。超限时 T4 的工具直接返回 Error 观察；T2 超限只会发生在子 context 里，按 failed 交回它的调用方。
- **没有回退**：进入模式缺失或非法在冻结时校验报错（§6.3）；运行中 LLM 跳到这样的目标时，顶层 run 的 Turn 以 `failed{behavior_config}` 结束，子 context 里则按 failed 交回调用方。不会落回“同一 run 换 system”。
- 返回方向：子 run 结束 → `Return{status, result}`，`caller` frame 出栈，父 run 重新成为 live，不关闭 Turn；结果按 `call.trigger` 走 `process_result` 或 `ToolResults`。

### 3.4 普通切换（T1）：已废弃

“换成 B 的 system / 工具 / 模型，但沿用 A 的 context 历史在同一个 run 里继续”这种切换不再存在：既不是默认行为，也不是缺省回退。本文早期版本的 `switch_in_place` 以及为它准备的“run 中途替换 `config`”都已取消。原来想用它做的事按意图改用另外两种：

```text
废弃： [system:A] [A 的历史] → [system:B] [A 的历史] [user:交接输入]
替代： 保存 A → 进入 B 自己的 context（SWITCH_CONTEXT，§3.5）
       或以 B 的 system 新建子任务 context（create-sub-context，§3.5），完成后返回 A
```

由此得到两条不变量：run.json `config` 与快照里的 request 从建 run 起不再被换成另一个 behavior 的（xllm 接手时读到的始终是这个 run 自己的有效配置）；同一个 run 的 system 段稳定，前缀缓存不会因为切换而失效（E11）。

### 3.5 SWITCH_CONTEXT、create-sub-context 与 fork（T3 / T2）

沿用 libopendan 的 process 语义（计划 §4.4）：挂起的 process 就是一个被 `process_stack` 引用的 run。三种进入模式的边界：

| 维度 | SWITCH_CONTEXT | create-sub-context | fork |
|---|---|---|---|
| system、工具、模型 | 目标自己的 | 目标（子任务）自己的 | 与父相同；目标不能声明自己的 `prompt.system` / `llm_context` 覆盖 |
| 历史 | 目标自己的；首次进入按 `inherit`（none / recent_dialogue）装配，不带别的 context 的快照 | 任务输入 + 显式选择的父历史：none / recent_dialogue / steps | 分叉点的完整有效历史，再追加分支任务输入；不接受 `inherit` |
| 再次进入 | 恢复目标自己的快照（同一个 run） | 每次新建子 run | 每次新建分支 |
| 完成 | `END` 按 Session 结束条件收尾，不隐式返回上一个 behavior | 结果交回调用方 | 结果交回调用方 |
| 栈 | 离开的 run 入栈 `parked` | 调用方入栈 `caller` | 调用方入栈 `caller` |

三者都延续当前 Turn，都不自动隔离文件系统、Session 状态与 worklog。

**SWITCH_CONTEXT（T3）**。目标有自己的 system、工具、历史和 run。首次进入时新建：目标的 system +（按 `inherit`）`<session_history>` + `on_context_switch` 交接批次（由 Session 从共享状态渲染：当前任务状态、交接原因）。再次进入时把它 parked 的 run 出栈并恢复原快照，只追加新的交接批次；它的 system、已有历史、编号、预算都不动，不把另一个 context 的原始历史并进来。各 run 各自编号。`END` 在这里就是 Session 的结束条件（`decide_end`）；要回到上一个 behavior 必须显式 `next_behavior`。入口 behavior 没有“被进入”的调用方，按 `switch_context` 处理。

**create-sub-context（T2 / T4）**。子 run 用目标 behavior 冻结的 system、工具、模型，输入是任务（behavior 触发：交接批次；工具触发：`task` 参数）加上按 `inherit` 选的父历史，由 llm_context 的 `derive_child` 构造（§3.7）：

| `inherit` | 内容 | `derive_child` 的 `InheritHistory` |
|---|---|---|
| `none` | 只有子 system 与任务输入 | `None` |
| `recent_dialogue` | 宿主筛选并渲染的 `<session_history>`（summary + 最近记录）。这是标明来源的筛选视图，不是父的完整历史 | 宿主直接放进子 request 的输入（`None`），或作为 `Material` 交给派生 |
| `steps` | 父已完成的 steps 与 summaries，作为结构化记录继承。要求父子都在 behavior loop、parser / renderer 兼容；跨 loop 模式时改为渲染成材料。进行中的 `action_step` 不算已完成记录 | `Steps` |

即使选了 `steps`，只要 system 换了，就仍是 create-sub-context，不叫 fork。

**fork（T2 / T4）**。子 run 保留父的 system 与分叉点的完整有效历史（已有摘要、多模态内容、成对的工具消息；behavior 模式还包括 request、steps、热 Step、history inputs），再追加分支任务输入，由 `fork_snapshot` 构造。“完整”只到分叉点：父停在完整 Step 的 `next_behavior` 时，分叉点就是当前历史末尾；由工具触发、父还有未完成的工具批次 / Step 时，分叉点在这个批次**之前**，批次留在父快照里，不用假的 tool result 补洞。只取最近几条、换 system 或重渲染成摘要都属于 create-sub-context。目标声明了不同的 system 时配置校验拒绝，不静默换模式。

子 run 的共同规则：

- 派生是纯函数，父快照不变；子有独立的 budget、usage、host meta 与输入 receipt，不带父的挂起状态、待派发调用。父恢复时不重置自己的工具额度。
- 子的 Step / action 编号接续父；`derive_child` / `fork_snapshot` 返回的 `InheritBoundary`（fork 还有 `ForkPoint`）告诉 Session 哪些是继承来的，继承部分不重复写进 worklog（`inherited_below`）。
- 子新增的 messages / steps 写进共享 worklog 供审计，但不并入父快照；重建 `<session_history>` 时，已返回的子 context 的 transcript 被排除，只渲染它的结果。压缩做同样的过滤，但压缩只识别被压缩片段内的 `process_done`：切点把子 run 的记录与它的 `process_done` 分开时，切点之前的那部分仍会进入摘要。
- 返回与嵌套规则见 §3.3。

### 3.6 工具触发的子调用（T4）

**需求**：有些“工具”本身就是一次推理——带着父 context 的上下文，用另一套提示词和工具做一个小决定，再把结果交回父 context。opendan 的 `try_create_worksession` 是典型：判断复用已有 worksession 还是新建，需要时再调用 `create_worksession`。

**现状（opendan）**：工具持有 `Weak<AIAgent>`，在 `execute()` 里调用 `session.fork_and_run_agent_loop`（`worksession_tools.rs`）。子 context 由磁盘上的父快照经 `rebuild_with_inherit` 构造（`llm_context_helper.rs::run_fork_sub_context`），在这次工具调用内部同步跑完，`Done` 的输出就是工具结果。子 context 不持久化（`state.snap.fork-N` 跑完即删），挂起一律当错误。它名为 fork，其实不继承 steps：`user_messages` 覆盖会清空 steps，只放一条“父 session 最近对话”——按本文的分类是 create-sub-context + `recent_dialogue`。

**“工具里嵌一个 run”在新架构下不成立**：

1. 父 context 停在工具调用中途，子 context 同时在跑，违反串行约束；Session 的 stop、lease、输入装配、心跳都只对着父 run。
2. 子 run 不在 `runs/`、不进 worklog，崩溃后丢失，父 run 只剩一个 inflight 调用。
3. 它要拿 Runner 的内存句柄，只能是层 ③，不经 Runtime 统一入口；父 run 也永远不能被 xllm 接手。

**目标：做成由工具调用触发的子调用协议，结果走已有的 PendingTool 路径交回。** 协议本身不关心子 context 怎么构造：它可以承载 create-sub-context，也可以承载 fork，由目标 behavior 的进入模式决定。

```rust
/// 宿主工具：Session 里存在子 context 目标（进入模式为 create_sub_context / fork 的 behavior）时才提供。
/// 对 LLM 就是一个普通工具（function 或 action）。它只校验并挂起，子 run 由 Session 推进。
impl AgentTool for CallBehaviorTool {            // call_behavior({behavior, task})
    async fn call(&self, ctx: &ToolCallCtx, args: Value) -> Result<AgentToolResult> {
        let (behavior, task) = parse(args)?;                                         // task：子 context 要做什么、返回什么
        ensure!(self.targets.contains_key(behavior));                                // switch_context 目标不能被调用
        ensure!(self.depth < MAX_CALL_DEPTH);                                        // 超限 → Error 观察
        ensure!(ctx.allow_deferred && ctx.call_id.is_some());                        // 不能为子 context 挂起的执行方直接报错
        Ok(AgentToolResult::pending(task_id = format!("subctx:{}", ctx.call_id)))    // llm_context 把 task_id 当不透明值，只有 Session 解析
    }
}

pub struct ProcessFrame { entry: String, role: FrameRole /* parked | caller */, call: Option<ChildCall>, run_id: String, .. }
pub struct ChildCall {
    mode: ContextMode,            // create_sub_context | fork：子 context 怎么构造
    behavior: String,
    trigger: CallTrigger,         // behavior | tool{call_id, task_id}：结果从哪里交回
    task: Option<String>,         // 工具触发：调用的 task 参数
}

impl AgentSession {
    /// 进入子 context（T2 / T4 共用）：调用方入栈，子 run 在下一圈按 call.mode 派生。
    async fn enter_sub_context(&mut self, lc: &mut LiveCtx, call: ChildCall, snap: &LLMContextSnapshot) -> Result<()> {
        self.worklog_append(run_history_entries(..) ++ [Outcome{kind: "call", next_behavior: &call.behavior, ..}])?;
        self.state.process_stack.push(ProcessFrame::caller(&self.state.live_run, call.clone(), snap));
        self.state.live_run = None;                                                   // 栈顶是 caller 且没有 live run → new_run 建子 run（§9.4）
        self.state.current_behavior = Some(call.behavior.clone()); self.state.internal_continuation = Some(call.behavior);
        lc.run.set_status(RunStatus::Paused)?;
        commit!(self)
    }
}
```

流程：

1. **挂起**：`call_behavior` 是宿主工具，不送 Runtime 执行（后续 policy 阶段的执行前检查也要覆盖它，它同样是一次 Do），返回 `Pending{task_id = subctx:<call_id>}`。llm_context 停止派发，父 run 以 `PendingTool` 返回，同批剩余调用留在快照里，`ToolResults` 之后接着跑（`context_loop.rs` 现有语义）。提供 `call_behavior` 的 run 要开启 `tool_policy.allow_deferred`。
2. **进入**：`handle_outcome` 看到父 run 停在 `subctx:` 等待 → `enter_sub_context`：父 run checkpoint 并 flush worklog → 压栈 `caller`（`trigger = tool{call_id, task_id}`）→ `commit!`；下一圈按目标的进入模式用 `derive_child` 或 `fork_snapshot` 建子 run（§3.5），子 run 成为 live。fork 的分叉点在触发批次之前。
3. **推进**：子 run 就是一个普通的 live run：交接输入前可以装配半订阅快照，也支持 stop、中断、ContextLimit 改写、崩溃恢复，也可以再调用子 context（受嵌套深度限制）。它不消费父的输入队列。
4. **返回**：除 stop（结束整个 session，栈一并清空）外，子 run 的任何结束都不关闭 Turn，而是带着 `status` 出栈（§3.3）：ok → 子的输出；failed → Error 观察（子的 Error / Budget 不让父 Turn 失败）；needs_user_input → 带问题的结构化结果，由父 context 决定要不要问用户、拿到答案后再调用一次。父 run 重新成为 live，Session 按 `call_id` 以 `ResumeFill::ToolResults` 恢复它，同批余下的调用按序续派。父 context 看到的只是一次普通工具调用的结果。
5. **恢复**：崩溃发生在 1、2 之间（父快照已挂起，栈还没压）→ `reconcile_runs` 看到父 run 停在 `subctx:` 等待而栈里没有对应的 `caller` frame，重做“进入”；栈顶是 `caller` 而没有 live run → 建子 run；子 run 已终态而 frame 还在 → 重做“返回”。进入与返回都恰好提交一次，结果只交回一次。
6. **接手**：父 run 挂起期间，等待的 `subctx:` task 只有 Session 能解析，xllm 拒绝接手（[long-tool TODO](../../notepads/llm-context-long-tool-todo.md) §4 的接手能力检查）。工具集里有 `call_behavior` 的 run 同样不让 xllm 接手（宿主工具，同 `app_tools` 规则）。不含它的子 run 是普通 run，满足 §3 表“接手”一行的条件就能被 xllm 跑完，之后由 `xagent run` 完成返回。

**与 T2 的关系**：T2 由 `next_behavior` 触发，结果经 `process_result` 交接批次返回；T4 由工具调用触发，结果经 `ToolResults` 返回。子 run 的构造（两种方式）、栈、持久化、worklog 完全共用。

于是 `try_create_worksession` 的目标形态是：ui context 调用 `call_behavior({behavior: "route_worksession", task: …})`；`route_worksession` 声明 `mode = create_sub_context`、`inherit = recent_dialogue`，在自己的 system 里约定 JSON 输出，只开放 `read`、`exec` 和层 ② 的 `agent-session create-worksession`。**不需要层 ③**：子 context 不是一个进程内工具，而是 Session 的 Context 调度（回答 §12 第 7 项）。

### 3.7 llm_context / xllm 层提供的原语

调度语义归 Session（本节以上）；下层只提供机械的构造与交接原语，按“llm_context 先行”的规则单列在 [llm_context Context 调度支持 TODO](../../notepads/llm-context-switch-support-todo.md)：

- **`derive_child(parent_snapshot, child_request, InheritHistory::{None | Material | Steps})`**（`llm_context::context_derive`）：create-sub-context 的历史派生。子 request 自带 system、模型、工具配置；父快照**可以处于 PendingTool 挂起或批次中途**（T4 必然如此）。
- **`fork_snapshot(parent_snapshot, ForkOptions)`**（同模块）：fork 的完整历史派生。保持父 system 不变，目标声明不同 system 时拒绝；父有未完成的工具批次 / Step 时，分叉点取在它之前。
- 两者都是纯函数，父快照不动；子有独立的 budget / usage / host meta，编号接续父；返回 `InheritBoundary`（fork 另有 `ForkPoint`），供宿主区分继承部分与子新增部分。
- **hosted run 在交接点让出**（G8）：run.json v5 增加 `handover {next_behavior, at_ms}`；hosted run 的 `Done` 指向另一个 behavior 时停在交接点（`paused`），`xllm --resume` 拒绝继续它，交给宿主提交转移。
- **不提供“run 中途替换 `config`”**：它只服务于已废弃的 T1，已取消，不是任何实验的前置。

`call_behavior` 的等待用不透明的 task id（`subctx:<call_id>`），由 Session 解析；llm_context 不识别这个前缀，也不知道 behavior 注册表、进入模式或 Session 是否完成。

---

## 4. Agent Input 与 Sub Session：原理、协议对象、模板、父子协作

### 4.1 原理

Agent Session 是协议，所以要先说清 Agent 在一个 Session 里**能感知到什么**：

- **消息**：有人（用户、别的 Agent、别的 session）对这个 session 说了什么。消息体直接是 cyfs-ndn 的 MsgObject（§4.2）。
- **`AgentEvent`**：这个 session 关心的某件事发生了（对象变化、定时器、任务结果、子 session 状态、系统通知）。

除此之外没有第三种。Session **不关心这两种东西怎么来的**：msg-center 记录、kevent、timer、task_mgr 回调、登记表 rev 变化，都由上层 bridge 翻译成这两种对象后投递；Session 只要求信封完整（§4.5）。

经过同一条队列的还有 **Session 控制协议**（§4.4：stop、decide、subscribe、unsubscribe、activity、perceive）。它们不是 Agent 的输入，不进上下文，由 Runner 直接执行；搭队列只是为了多方投递与持久化。协议文档要把两者分开写。

已实施（2026-10-03，`opendan.session_input/3`）：总线记录只有 `msg / event / control` 三种。原 `change` 是 AgentEvent 的一种**投递策略**（§4.3），原 `perception` 并入控制协议（`perceive`），`msg` 的 payload 是 MsgObject 加投递层信息。线格式、校验与拒绝规则以 [Session Input Protocol](<protocol/Session Input Protocol.md>) 为准。

### 4.2 消息：直接使用 MsgObject

不定义协议级的 `AgentMessage`。`type = "msg"` 的 payload 是 MsgObject（原样，不改写、不裁剪）加一个很小的投递层结构：

```rust
pub struct SessionMsg {
    pub msg: MsgObject,            // cyfs-ndn MsgObject v2（CYFS 标准对象 §16）
    pub delivery: MsgDelivery,     // from_name / conversation_name / record_id / tunnel，全部可选
}
```

- 信封 `key` 必须等于消息的 ObjId（`thread.reply_to`、`relates_to.target` 因此直接对得上总线里的 key）；说话人是 `msg.from`，信封 `from` 只是投递者，用于审计。
- 附件是 `content.refs` 里的 `DataObj`（有 ObjId）；本机文件先登记进 NamedStore 再引用。
- 手工投递用构造 helper（`text_msg / attach / reply_to / PostedInput::msg`），CLI 的 `post --msg … [--from] [--attach] [--reply-to]` 是它的命令行形式。
- 总线上放的是 MsgObject，不是 `AiMessage`：Session 先做选批、模板视图、渲染与 receipt，`AiMessage` 只在渲染之后出现（一条链路：MsgObject → 模板视图 → 模板 → user AiMessage → AICC）。

### 4.3 AgentEvent 与投递策略

```rust
pub struct AgentEvent {
    pub subscription_id: Option<String>, // 显式订阅 id；Session 已登记的隐式订阅可为空
    pub source: EventSource,             // { kind: object | session | task | timer | system, id }
    pub event: String,                   // 事件名，如 changed / updated / finished / fired
    pub seq: Option<u64>,                // 来源内的版本号；Observe 合并用它比较新旧
    pub summary: String,                 // LLM 看到的一句话（上限 1 KB，超出拒绝）；不用于机械判断
    pub data_ref: Option<String>,        // ObjId 或相对 session 目录的路径
    pub terminal: bool,                  // 该来源的终结事件
}
```

投递者只描述事件；它是 Input 还是 Observe 由接收 Session 按订阅机械决定（`SessionConfig::subscription_for`）：

| 情况 | 处理 |
|---|---|
| 匹配 active 订阅 | Input：进入受控输入的候选批次，可开启或并入 Turn |
| 匹配 semi 订阅 | Observe：合并进 `state.pending_events` 并消费；不独立触发推理 |
| `source = task:<id>` 且有挂起调用在等这个 task | 只触发 resolver 查询，结果回填 ToolResult，不作为事件注入 |
| 未匹配有效订阅（含未知、已取消、`subscription_id` 与来源不符） | 丢弃并提交消费位置（`event_dropped`）；timer、系统事件没有兜底例外 |

订阅变更与事件按投递 index 生效；active 事件一经接受（`inputs[src].accepted`），之后的 unsubscribe 不改变它的处理。用户时区是每个 Session 的隐式 semi 订阅（`source = system:user_timezone`）。

**半订阅维护状态**。Observe 投递的事件在 state.json 里按 `(subscription_id, source)` 合并为最新的待注入状态（`pending_events`），terminal 事件单列不覆盖。渲染材料统一称为 **`semi_subscription_snapshot`（半订阅快照）**：只有在 `on_init / on_input / on_context_switch` 受控输入使用前，才选取状态版本并渲染、注入。检查点可以接收并保存更新，但不独立注入；没有受控输入时继续保留，空闲时同样保留。被新版本覆盖的旧值只计数（`superseded`）。渲染和选取不消费状态，提交时按 `(subscription_id, source, key)`（有 seq 时一并核对）只清除 receipt 覆盖的版本；处理 v7 时收到 v8，提交 v7 不得清掉 v8。总线容量满（每个 Session 最多 64 条 pending input）时拒绝 append；快照预算不足的版本留在 `pending_events`。

**已实现的内置 bridge**：Session 来源的拉取订阅（`source = session`）由 `runner/inputs.rs::poll_session_subscriptions` 在每次路由时拉登记表比 rev，没有外部 producer。合成 `AgentEvent{subscription_id, source: Session, seq: rev, summary: watched 字段差异}` 后直接并入 `pending_events`，**拉取路径总是 Observe**，即使该订阅配置为 active；总线上有投递位置的 session event 才按上表路由。活动 session 集合不使用半订阅游标，每次受控输入现算完整列表。

**待实施的父子桥（C15）**：按 `origin.parent_session` 查子，不需要显式订阅；§4.14 的需要关注与结束事件必须有可恢复的 Input 接受 / 提交记录，不能直接沿用上述 Observe 路径。合成事件进入候选批次，receipt 提交后才推进父的消费游标；父无队列也成立。此处的“已实现内置 bridge”不包含父子隐式关注。

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

总线上每条记录是一个 JSON 对象（逻辑记录）；kmsg 上信封字段是 headers，payload 是消息体：

```jsonc
{ "schema": "opendan.session_input/3", "type": "msg | event | control",
  "key": "…", "from": "<投递者 principal>", "at_ms": 0, "payload": { } }
```

```rust
#[serde(tag = "type", content = "payload")]
pub enum SessionInput { Msg(SessionMsg), Event(AgentEvent), Control(ControlCommand) }

pub struct PostedInput  { schema, key, from, at_ms, input: SessionInput }               // producer
pub struct FetchedInput { src, index, kind, key, from, at_ms,
                          input: Result<SessionInput, Rejected> }                         // consumer

// 投递（任何有写权限者，经登记表）：校验 + 容量检查（64 条 pending）+ append 在一个临界区内；满时返回 input_full
async fn post_input(&self, sid: &str, input: &PostedInput) -> Result<u64>;
// 消费（仅 lease 持有者）
async fn fetch(&self, progress: &SourceProgress, max: usize) -> Result<Vec<FetchedInput>>;
async fn confirm(&self, progress: &SourceProgress) -> Result<()>;   // 先提交 state.json，再累积 ack
```

投递与消费用同一套校验（`parse_record`）；消费时不合法的记录标记为已消费并写 `input_rejected{reason}`，不卡住累积确认。`src` / `index` 由通道给出，producer 不能提供。

bridge 必须保证：同一来源内有序、至少一次、key 唯一。Session 用 receipt 与 `recent_keys` 做幂等。现有 `KmsgInput` / `DirMsgQueue` / `Waker` / `confirm_inputs` 已覆盖这四个操作，`InputBus` 是职责名称；当前 payload 已换成 SessionInput，无需再做一次类型替换。

64 条上限由能读取 Session 目录的 `SessionRegistry::post_input` 在 `post.lock` 内检查并追加；kmsg 服务没有条件追加，直接 `post_to_queue` 或无法读取目录的跨主机 producer 不受此保证。xagent 的 `post`、`ctl` 与 bridge 必须经登记表入口；收到 `input_full` 不确认上游。msg / event / control 共用容量，不能承诺满队列时仍可追加 stop；`StopMonitor` 保证发现的是**已经入队**的 stop。去重依赖 `recent_keys` 的有限窗口，不承诺永久过滤重投。

队列通知只用于安排 Runner 检查持久化输入，不对应 LLM 的 wakeup 入口。是否注入由三类受控输入的条件决定；投递策略用 `Input / Observe` 表达，具体渲染入口见 §6.2。

### 4.6 事件桥（bridge）

| bridge | 在哪 | 产出 |
|---|---|---|
| kevent 桥 | xagent `run` / `serve` 进程内；以后是 OpenDAN Supervisor | 按 session 的 ObjectEvent 订阅模式订阅 kevent → `AgentEvent{subscription_id, source: Object}` |
| timer 桥 | 同上（self_check） | `AgentEvent{source: Timer}` |
| task_mgr 桥 | `libopendan::bridge::task::task_event`（映射已实施；接 TaskMgr 后移） | `AgentEvent{source: task:<task_id>}` |
| msg-center 桥 | `libopendan::bridge::msg::route_msg_record`（纯函数，已实施；接 msg-center 后移） | 过滤 / 分流后原样投递 `SessionMsg{msg, delivery}`，斜杠命令转 `control` |
| 子 session 桥 | Runner 内置（§4.3、§4.14） | `AgentEvent{source: Session}`：订阅的 session 与本 session 的子 session |
| CLI | `xagent post --msg / --json` | 消息构造 helper 或完整逻辑记录；`post` 只接受 msg / event，control 走 `ctl` |

实验用 `--no-bridge` 关闭外部投递桥，手工 post 控制时序；内部恢复、轮询与 stop 监视仍生效（§8）。

### 4.7 Session 模板

模板是 `SessionSpec` 的预设，按 `session.class` 选择；内置四个，`agent.toml [session.<class>]` 可覆盖；创建时解析进 session_config 的现有字段（`end_condition`、`input_policy`、`channels`、`subscriptions`、`prompt.behavior`）加一个新段 `session.policy`，与 behavior 一起冻结。

```rust
pub struct SessionTemplate {
    pub class: String,
    pub kind: SessionKind,
    pub turns: Turns,                 // One | Unbounded | N(n)   → end_condition
    pub wait_user_msg: WaitPolicy,    // Allowed（Turn 保持打开等输入）| FinishFailed（work：不许等人，记 failed 并结束）| FinishCompleted
    pub input: InputChannel,          // None（不建队列）| Queue
    pub observe: Observe,             // Off | Events | EventsAndActive     → 半订阅快照包含哪些状态
    pub load_hints: bool,
    pub default_behavior: Option<String>,
    pub max_process_depth: u8,        // process_stack 深度上限（默认 4，§3.3）
    pub max_sub_sessions: u8,         // 同时未结束的子 session 数上限（默认 4，§4.11）
    pub max_session_depth: u8,        // 子 session 嵌套深度上限（默认 2，§4.11）
    pub implicit_subscriptions: Vec<Subscription>, // 模板预先登记的订阅（如 self_check 的 timer、用户时区）；没有“未订阅也投递”的兜底策略（§4.3）
}
```

| class | turns | wait_user_msg | input | observe | hints | 说明 |
|---|---|---|---|---|---|---|
| `work` | One | FinishFailed | **None**（需要外部投递的订阅则升为 Queue） | Events | 否 | 一次性任务：一个 Turn 跑到结果；不等用户；默认没有队列 |
| `ui` | Unbounded | Allowed | Queue | EventsAndActive | 是 | 长生命周期对话入口（后移） |
| `self_improve` | One | FinishFailed | None | Off | 否 | 感知窗口整理 |
| `self_check` | Unbounded | FinishFailed | Queue（timer 事件） | Off | 是 | 定时自检 |

**无队列 work session 的后果**：stop 由 lease 持有者将 SIGINT 接到现有取消路径；decide 走 artifacts 门面；exec 子进程的 `activity --touch` / `perceive` 没有通道，touching 由工具调用推断、感知只有结束时的 run_digest。父接收子状态（C15）与 task resolver 查询不需要队列，但子 LLM 主动给父发消息需要父有队列。只有需要接收外部投递的订阅才自动升为 Queue；默认时区的初始快照、拉取 session 状态、已跟踪 task 不因此强制建队列。无队列时不承诺收到外部时区更新。等待 task / children 时仍须轮询，不能因 `bus = None` 提前结束。

### 4.8 xagent 的三种运行形态

| 形态 | 命令 | 行为 | 返回时机 |
|---|---|---|---|
| 单 Turn | `xagent run <sid> --msg "…"` | Queue Session 经 MsgObject helper 投递后 `drive(StopWhen::TurnClosed)`；无队列 Session 用创建时保存的 objective / 初始 MsgObject bootstrap，`run` 不接受投递参数（§9.2） | 当前 Turn 关闭；或 Turn 尚未关闭且本次无法继续 → 退出码 3 |
| 消费积压 | `xagent run <sid>` | 只 drive；有积压就处理到 Turn 关闭 | 同上 |
| 常驻 | `xagent serve <sid>…` | 持有 bridge / ChildDriver / resolver 的进程级生命周期；循环 `drive(StopWhen::Idle)`，在通知或有界轮询后重新 drive | SIGINT / `stop` / session finished；仅无待推进工作时允许 idle unload |

`--until finished|idle|outcomes:<n>` 保留现有 `StopWhen`，用于调试。

### 4.9 Turn 关闭与 `StopWhen::TurnClosed`

```rust
pub enum StopWhen { Idle, Finished, MaxOutcomes { n: u64 }, TurnClosed }
pub enum DriveResult {
    /* 现有变体 … */
    TurnClosed { rev: u64, turn: u64, status: TurnStatus, answer: Option<String> },
    TurnOpen   { rev: u64, turn: u64, waiting_for: Option<WaitingFor> },   // 真实 open_turn；可能等 input / tool / children
}
```

Turn 规则沿用 readme：没有打开的 Turn 时提交的输入批次开启新 Turn；SWITCH_CONTEXT、子 context（create-sub-context / fork）的进入与返回（§3.3）、挂起、重写、重启都延续，半订阅快照随受控输入同批提交；只有 `finish_run` / `stop_session` 关闭。模板的 `wait_user_msg` 决定 `WAIT_USER_MSG` 在该 session 里是"保持打开"还是"结束"。

`TurnClosed` 是 C6 的新增返回条件，不改变持久化的结束规则。`--until turn` 在本次推进或 reconcile 关闭 Turn 后立即返回该 Turn 的结果，即使同时将 Session 标为 finished；没有打开且没有本次关闭的 Turn 时返回 Idle，不伪造 `TurnOpen {turn: 0}`。停止关闭的 Turn 仍映射退出码 4。task 等待到 `max_wait` 时，work 模板也可以返回 `TurnOpen {waiting_for: tool}`；`FinishFailed` 只解释 `WAIT_USER_MSG`，不禁止工具等待。

### 4.9.1 已实现的普通 task 等待

| 状态 | Session 行为 |
|---|---|
| 工具返回 Pending | 保存快照的 pending calls；保留 run 锁与 resolver，以 `WaitingRun` 在上下文之外等待，不调用 LLM |
| resolver 不支持 task_id | `can_resolve = false` → RecoveryBlocked，保留现场 |
| Running 且未到 until_ms | 继续轮询；通知仅提前触发查询，空 inbox 也查询；msg / Input event 暂存，control 仍处理 |
| Finished / Unknown，或已到 until_ms | 所有挂起调用满足回填条件后，按 call_id 回填当时状态；到期时可回填 Running；发布快照后续跑同一 run / Turn，不重放任务 |
| 刚回填 ToolResults | 先续完工具批次，`filled` 期间不注入新的受控输入；可再次 PendingTool |
| 已返回 task 引用的后台调用 | run 未结束时由 llm_context 展示；run 结束且 Session 未 finished 时将仍运行的 task 记入 `watched_tasks`，随后查询并合成完成事件，显式 semi 优先，否则按隐式 active 输入 |
| stop / finished | stop 对当前 Turn 的 task 传导取消并配对结果；Session finished 清空 `pending_events` / `watched_tasks`，以后 task 完成不重开 Session |

当前 `StopWhen::Idle / Finished` 在串行 task 等待中均轮询至完成或 `options.max_wait`；`MaxOutcomes` 在计数到达或进入等待分支时返回。C6 的 `TurnClosed` 等待采用相同上限。无队列不妨碍查询，但必须有存活的宿主提供调度；进程内 task 随宿主退出可能丢失，重启后按 Unknown 回填。

后台任务的已知恢复缺口：正常结束通过 resolver 的 `active()` 接管；reconcile 重做结束时没有从快照 call_result 补建遗漏的 `watched_tasks`。此项在 §11 C16 单列，不能把已有轮询描述为完整崩溃恢复。正式 TaskMgr 适配、外部 dispatch intent / task 绑定仍后移。

### 4.10 Sub Session：与子 context、Sub-Agent 的区别

Agent Session SDK 化的一个主要目的，是让 Agent（以及应用）方便地把一段工作交给**另一个 session**。[长任务与执行体 RFC](<./OpenDAN Long Task & Sub-Agent.md>) §14 把执行体分成 Tool（函数）、Session（线程）、Sub-Agent Instance（进程）三类，§21 的 long-once session 就是这里的 Sub Session。结合 §3 的 Context 调度，“把一段活交出去”有四种做法：

| | 工具触发的子 context（T4） | behavior 触发的子 context（T2） | **Sub Session** | Sub-Agent / 其它 Agent |
|---|---|---|---|---|
| 是什么 | 本 session 里由工具调用触发的子 run | 本 session 里由 `next_behavior` 触发的子 run | 同一个 Agent 的另一个 session：自己的目录、state.json、worklog、Turn、lease | 另一个 Agent DID |
| 能否并行 | 否，父 run 挂起 | 否 | **能**，各自的 Runner 推进 | 能 |
| 上下文 | 由目标的进入模式决定：create-sub-context 按 `inherit` 选父历史，fork 带分叉点的完整有效历史 | 同左 | 不继承；只有创建时给的 objective、首批输入、附件引用（可选附父最近对话摘录） | 只有消息 |
| 共享什么 | 父的 lease、state、Turn、runs/ | 同左 | 同一个 Agent State（登记表、认知、产物、behavior 目录），可共用 workspace | 不共享 |
| 生命周期 | 不超过一次工具调用 | 不超过父 Turn | 独立：可以比父 Turn 长，可以常驻 | 独立、长期 |
| 结果怎么回来 | `ToolResults` | `process_result` 交接批次 | 登记表状态 + AgentEvent + 子主动发的消息；同步等待时作为工具结果（§4.14、§4.15） | 消息（msg-center） |
| 谁看得见 | 父 session 的 runs/ 与 worklog | 同左 | 登记表、`sessions` 列表、自己的目录与报告；可以单独验收 | 对方系统 |
| 适合 | 需要父上下文的窄意图小决定（路由、分类） | 换个角色做同一主任务的一段 | 一段独立工作：要并行、要长时间、要自己的 workspace / runtime / 产物 / 验收，或要在父结束后继续 | 宽意图、角色级能力域（RFC §12） |

选择顺序沿用 RFC §10–§11：只是想隔离上下文 → 先用子 context（T2 / T4）；需要并行、独立生命周期、独立产物与验收 → Sub Session；角色级的长期能力域 → Sub-Agent（不在 xagent 范围，§1.2）。

Sub Session 不是新的协议对象：它就是 `origin.parent_session` 指向父 session 的普通 session（计划 §4.8 `create_session`），父子关系只体现在登记表和下面几条约定里。**父子都只写自己的 state.json**，彼此只经过登记表、session 目录（只读）、输入通道和控制协议沟通。

### 4.11 创建：一个标准 agent tool

创建 Sub Session 是一个层 ② 工具（§5.5），由 `agent-session create-worksession` 实现，同时在 `bash_tools` 里声明 schema。LLM 看到的是一个普通 function / action，执行经 exec 进 Runtime，所以 xllm 接手的 run 也能用。

```text
agent-session create-worksession --objective <text>
    [--msg <text>]... [--attach <obj_id>[=<name>]]...   首批输入：MsgObject；附件只给已存在的 ObjId
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

- 父每次提交之后、以及登记表变化时 `tick`。子 session 各持自己的 lease，与父并行；宿主进程退出也会停止其中的子驱动协程，但不删除已提交状态；重启后 ChildDriver 从登记表与快照重新接管。
- 子的状态变化后，ChildDriver 唤醒父的调度器；父没有 drive 在执行时再 drive(parent)，已有驱动者则让其轮询发现，不能并发驱动同一父。工具 create-worksession 在父执行中也可能登记子，因此 ChildDriver 需独立轮询登记表，不只等父提交。
- `xagent run <parent>`：父 Turn 关闭后，默认继续推进本进程拉起的子 session，直到它们空闲或结束再退出；`--detach-children` 立即返回，留给 `xagent serve`。`xagent serve` 同样接管所服务 session 的子 session。
- 子 session 首次推进时自己绑定 runtime（`binding.json`），默认要求与父相同的 runtime_id；共用 workspace 时按活动视图避让，与多个顶层 session 共用 workspace 相同。

### 4.13 父子之间的沟通渠道

| 方向 | 渠道 | 内容 | 谁发起 | 怎么进入对方 |
|---|---|---|---|---|
| 父 → 子 | 创建参数 | objective、首批消息、附件引用、可选的父对话摘录 | 父 LLM（创建工具） | 子的 bootstrap 批次 |
| 父 → 子 | `agent-session post <child> --msg` | 消息（MsgObject，`from` 为所属 Agent）：补充要求、回答子的提问 | 父 LLM | 进入子的受控输入；要求子是 `--interactive`（有队列） |
| 父 → 子 | 控制：`ctl <child> stop`、`decide accept\|discard` | 停止、验收 | 父 LLM，或父 Session（stop 级联，§4.16） | 不进上下文 |
| 子 → 父 | 登记表状态 `SessionStatus` | `run_state`、`outcome`、`one_line_status`、`report_brief`、`pending_decision`、`last_error` | 子 Session 每次提交后自动 `report_state` | 父被动读取，或父的内置 bridge 产出 AgentEvent（§4.14） |
| 子 → 父 | `agent-session post <parent> --msg` | 消息（MsgObject，`from` 为所属 Agent）：提问、阶段性交付 | 子 LLM | 进入父的受控输入；要求父有队列，否则子只能经状态汇报 |
| 子 → 父 | 报告与产物 | `report.md`、登记的产物、worklog | 子结束时 | 父按需 `read-session` / `artifact head` |

两条约定：

- **子 session 的“用户”是父 session**：子默认 headless，没有 msg-center 通道，不直接对人发消息（RFC §21.2）；需要人参与时向父提问，由父决定是否转给用户。子的 system 段里有 `session.parent`（冻结变量，§6.5），子 LLM 知道该找谁。
- **子要等输入时**：work 模板的子遇到 `WAIT_USER_MSG` 按 FinishFailed 结束，问题写进报告（§4.7）；`--interactive` 的子（`wait_user_msg = Allowed`）Turn 保持打开，状态变为等输入，父收到 `needs_input` 事件后用 `post` 回答。

### 4.14 进展与汇报：被动查询与主动推送

总原则沿用 RFC §6–§7：**登记表里的状态是真相，事件只是加速器**；处理事件时要重新读状态，不能只信事件内容。有三种机制：

1. **被动查询（父拉取）**：父 LLM 随时可以 `agent-session sessions --children [--active]`（子列表与 `one_line_status`）、`read-session <sid> [--report] [--worklog <n>]`。读到的就是真相，不受订阅影响。
2. **状态推送（系统自动，子 LLM 不参与）**：子每次提交都 `report_state`；父的内置 session bridge（§4.3）在组批与观察边界时，按 `origin.parent_session = 自己` 比对子的 `status.rev`，合成 `AgentEvent{source: Session{child}, event, seq: rev, summary, terminal}`：
   - **进度**（`one_line_status`、`activity` 变化）→ `event = progress`，Observe：按来源保留最新状态，在下一次受控输入使用前作为半订阅快照注入（§4.3 的 `pending_events`）。
   - **需要关注**（`run_state` 进入等输入，或出现 `pending_decision`）→ `event = needs_input | needs_decision`，Input。
   - **结束**（finished / failed / stopped）→ `event = finished`，`terminal = true`，Input；`summary` 取 `report_brief`。
   - 创建时的 `--report` 决定父收哪些：`final` = 需要关注与结束；`progress` = 再加进度；`none` = 都不推，父只能拉取。它对应 opendan 的 `report_delivery`（final_only / top_level / all），但由**接收方**（父）选择，符合 §4.3“投递策略由 Session 决定”。
3. **显式汇报（子 LLM 主动）**：子 LLM 调 `agent-session post <parent> --msg …`，用于需要父决定的问题或阶段性交付。它是一条消息，进入父的受控输入；频率由子的 behavior 提示词约束。

父无队列也能收到 2 里的事件，因为它们来自登记表而不是队列；负责推进父的是持有父 Session 推进权的进程（ChildDriver / Supervisor），它在子状态变化后重新 `drive(parent)`。

### 4.15 等待：异步与同步

- **异步（默认）**：创建工具立即返回，父继续推进；结果按 §4.14 以事件到达，可能落在父后续的 run 甚至后续的 Turn。
- **同步（`--wait`，或对已有子 session 调 `agent-session wait <sid>`）**：工具返回 `Pending{task_id, until_ms}`，快照里的挂起记录是 `PendingToolCall {call, task_id, until_ms}`（见 [long-tool TODO](../../notepads/llm-context-long-tool-todo.md) §4）。`task_id` 对 llm_context 是不透明值，没有 `wait.source` / `class` 词汇，也不按前缀归一；它与子 session 的对应关系只由宿主 Session 的 resolver 解析（做法同 T4 的 `subctx:<call_id>`）。父 run 以 PendingTool 挂起（父 run 要开启 `allow_deferred`，同 T4），Turn 保持打开。Session 提供的 resolver 只读登记表：
  - 子结束 → `Ready(Observation{session_id, outcome, report_brief, answer_ref, artifacts})`；
  - 子在等输入或等决定 → `Ready(Observation{status: needs_input, question})`，父用 `post` 回答后再 `wait`；
  - 其它情况继续等，ChildDriver 同时在推进子；
  - resolver 把挂起调用的 `task_id` 解析到子 session；该子的结束事件作为这次调用的工具结果消费，不再作为事件重复注入。
- **一次等多个**：同一批次里对每个子各调一次 `wait`；批次串行派发，效果就是“等全部”。
- **与 T4 的区别**：T4 的子 run 在父 session 内串行，父挂起时没有别的东西在跑；`--wait` 的子 session 在自己的 lease 下推进，可以与其它子并行；持久恢复相互独立，进程存活范围见 §4.12。
- **接手**：这个 resolver 只依赖 Agent State，不依赖 Runner 内存；但 v1 只有 xagent 提供它，父 run 挂起期间 xllm 拒绝接手（同 T4）。

**父结束规则（汇总）**：父的 `decide_end` 发现还有未结束、且 `report != none` 的子 session 时，不结束 session：run 结束，Turn 保持打开，`waiting_for = Children{sids}`；子的结束事件作为受控输入并入同一个 Turn，父 LLM 汇总后再 `END`。这样 work 父也能“先并行派出几个子 session，再汇总”。不想等的子，要么创建时选 `--report none`，要么先 `ctl <child> stop`。

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
                  deps: &XllmDeps)
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
- 到期：下一个观察边界起工具列表里消失；到期后的调用 `dispatch` 直接 Deny。grant 变化作为 `AgentEvent{source: System, event: grant_changed}` 按 Observe 保存，下一次受控输入前通过半订阅快照告知 LLM；权限检查立即生效，不等待消息注入。
- grant 的签发与撤销走 Agent State 门面 `runtime_grants()`（新），不走 session 控制协议；`xagent ctl grant --runtime host:… --ttl 30m --paths …` 只是写 Agent State。
- 当前 xllm 按保存的 runtime 构造与核验目标，native/tmux/SSH 可恢复；Session 的 helper 与 app_tools 依赖须可用。未来 grant 的接管核验待 policy 阶段设计。

### 5.5 工具的三层来源（修订）

| 层 | 例子 | 由谁解析 / 执行 | 访问 Agent State | xllm 能否接手 |
|---|---|---|---|---|
| ① 内置 / MCP / `bash_tools` | `read_file` `write_file` `edit_file` `exec` `glob` `grep`、MCP | **Runtime** 按有效工具配置派发；内置 exec/文件使用执行体，MCP 仍在所配置服务执行 | 不访问 | 保存目标与依赖核验通过时能 |
| ② CLI 形态 session 工具 | `agent-session ctl … / recall / note / sessions / read-session / create-worksession / artifact` | 作为沙箱 PATH 上的命令，经 `exec` 进 Runtime；子进程里 `AgentStateClient::connect` | 子进程直连（host / remote 沙箱里只能走 kRPC，这是 §7 `connect` 必须有 kRPC 形态的硬理由） | 能 |
| ③ 进程内 session-aware 工具 | 暂无成员：子 context（create-sub-context / fork）是 Session 的 Context 调度（§3.5、§3.6），`call_behavior` 只校验并挂起、不拿会话句柄，推理不在工具里跑 | `SessionToolProvider` 注册给 Runtime，**仍经 `dispatch` 的 admit**，执行在 Runner 进程 | 直接拿 `AgentStateClient` + 会话句柄 | 不能（`app_tools`） |

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
4. 冻结后 system 段只依赖冻结材料 + session_config，所以同一 behavior 的新 run 保持相同 system 前缀；不同 behavior 可以有各自的 system / 配置，fork 沿用父的前缀。

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
    // runtime_grants() 属于 §5.4 的后续 policy 阶段，不作为本轮 trait 必需方法。
}
```

`BehaviorConfig` 基于 opendan 现有 toml 的结构（`behavior_cfg.rs`），在目标设计中分开 system 模板、三类受控输入模板、半订阅快照模板与消费策略：

```rust
pub struct BehaviorConfig {
    pub meta: Meta { name, objective, next: Vec<String> /* 可选：声明可切换 / 可调用的目标，构成冻结闭包 */ },
    pub prompt: Prompt {
        mode: LoopModel,                 // agent(function_call) | behavior；原 agent.toml [session.<class>].loop_mode 移到 behavior
        system: String,                  // system 段模板；原 prompt.on_init 改名
        on_init: Option<String>,         // Session bootstrap 的启动 user message
        on_input: Option<String>,        // 选中的外部 message / Input event
        on_context_switch: Option<String>, // 目标 context 的交接 user message
        semi_subscription_snapshot: Option<String>, // 受控输入之前的半订阅快照材料，不是触发入口
        parser, parser_strict, output,
    },
    pub input: InputConsumption { mode: Single | Batch,   // 消费策略；单条 / 组批，与正文模板分开；默认 Batch
                                  media: Reference | Inline }, // 附件是否以图片 / 文档块注入；默认 Reference
    pub capabilities: Capabilities { tool_whitelist, action_whitelist, tool_plan, approval_required, disable_capabilities },
    pub budget: Budget { max_tool_iterations, max_consecutive_errors, max_total_tokens, max_completion_tokens, max_wallclock_ms },
    pub model: Model { preferred, fallbacks, temperature, provider_options },
    pub entry: BehaviorEntry {           // **进入本 behavior 时**的方式（§3.3、§3.5）；由目标决定，没有缺省模式
        mode: ContextMode,               // switch_context | create_sub_context | fork
        inherit: InheritMode,            // none | recent_dialogue | steps；steps 只用于 create_sub_context，fork 不接受 inherit
    },
    pub hooks: Hooks,                    // on_context_limit_reached / on_llm_message_compress / on_provider_failed / on_interrupt_*
}
```

三类受控输入的边界如下。这里的“受控输入”指 Session 安排的新 user message，与总线上的 `ControlCommand` 无关。

| 入口 | 条件与材料 | Turn / 恢复边界 |
|---|---|---|
| `on_init` | Session 首次启动，使用配置、初始状态及当批允许消费的输入；没有外部输入也能生成启动消息 | 只在 bootstrap 时使用；不因新建 run、恢复或压缩再次执行 |
| `on_input` | 选中的 message / Input event；按 `input.mode` 单条或组批后交给模板 | 无打开的 Turn 时开启，否则并入；control、Observe event 和队列通知本身不构成此入口 |
| `on_context_switch` | 根据交接状态生成目标 context 的继续执行消息：behavior 切换、子 context 进入、通过交接批次返回的子结果 | 延续当前 Turn；工具触发的子调用返回走 ToolResults，普通快照恢复不重复注入交接消息 |

同一次装配只选一个入口，顺序为 `on_init` → `on_context_switch` → `on_input`；该批允许消费的外部输入可以并入启动或交接消息，不再重复生成 `on_input`。子 context 不消费调用方队列、未完成工具批次期间暂存输入的规则不变。模板只渲染已经选定的材料，不决定路由、出队或 Turn 边界。

`input.mode` 默认 Batch；Single 在按 source / index 排序的可处理 message / Input event 中总共选一条，Batch 最多取 `options.input_batch_max` 条，未选输入保留，后台 task 合成候选排在总线候选之后。首次进入目标 behavior 时先完成必要的冻结与校验，再读取策略和模板。模板缺省使用内建渲染；已有选中的外部输入却渲染为空必须报错并保留现场。没有外部输入时的 bootstrap / continuation 处理见 §9.3。

`semi_subscription_snapshot`（半订阅快照）是三类入口共用的前置材料，装配函数为 `render_semi_subscription_snapshot`。不设置 `on_observation` 或第四类输入 hook；旧 `on_behavior_step_ob` 不作为半订阅入口沿用，工具结果渲染仍属于 LLM Context 的执行协议。旧输入入口 `on_wakeup / on_behavior_switch` 对应新名 `on_input / on_context_switch`；旧 behavior cfg 中用作 system 的 `prompt.on_init` 对应 `prompt.system`。libopendan 宿主已按新名实施（`session_config/4`），不提供旧名兼容；C7 的 behavior 素材按新结构编写，现有 OpenDAN 配置改造仍按 §1.2 后移。附件块由 `input.media` 决定，模板不决定是否内联。

进入模式放在目标 behavior 上，正好回答 readme 里的 TODO（“切换模式由 target behavior 的配置决定，而不是由当前 session 决定？”）：是。校验规则：`fork` 目标不能声明自己的 system 与模型（要换就用 `create_sub_context`），也不接受 `inherit`；`switch_context` 目标不接受 `inherit = steps`；入口 behavior 未声明时按 `switch_context`；其它 behavior 没有进入模式是配置错误，不回退成任何默认模式。进入模式为 `create_sub_context` / `fork` 的 behavior 就是 `call_behavior` 可调用的目标（§3.6），不需要另外声明工具。`SessionAssembler::behavior_entry(cfg, behavior)` 读冻结的 `behaviors[target].entry`；Session 级的 `extensions.opendan.process_modes` 已废弃并被拒绝。libopendan 的 Session 宿主以 `extensions.opendan.behaviors.<name> = {mode, prompt{system?, on_init?, on_input?, on_context_switch?, semi_subscription_snapshot?}, input{mode, media}?, llm_context?, inherit?}` 承载同一份进入配置，冻结（C7）时由 BehaviorConfig 生成它。

### 6.3 冻结：时机、位置、范围

**位置**：在当前 `session_config/4` 的 `prompt` 中新增 `frozen`，不新增文件。下例是冻结后的目标形状，`frozen` 尚未实现；实施 C4 / C5 / C7 / C15 时统一确定下一版 schema 并更新 fixtures，不回退或复用 `/3`：

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
  "system": "…", "context": [], "mechanical_compress": { … },
  "initial_inputs": [],                 // C4 新增：无队列创建时的只读 MsgObject 逻辑记录
  "on_init": "…", "on_input": "…", "on_context_switch": "…",
  "semi_subscription_snapshot": "…",
  "input": { "mode": "batch", "media": "reference" }
}
```

**时机**：

- `create_session` 时，若创建者的 `AgentStateClient` 能读目录（文件版能看到 AgentRoot、进程内、kRPC 有权限）→ 立即冻结。
- 否则留空，由驱动者在首次 drive 的 bootstrap（任何推理之前、`bind_or_verify` 之后）冻结并原子替换 session_config（`config_rev + 1`）。state.json 的 `bootstrap_done` 仍是“首批输入已提交”的含义，不复用。
- 冻结后 `BehaviorAssembler` **只读 `prompt.frozen`**，不再碰目录；读不到 frozen 又拿不到目录 → `RecoveryBlocked`，不猜。

**范围**：入口 behavior + `meta.next` 声明的可达闭包（切换目标与 `call_behavior` 可调用的目标）+ identity。冻结与补冻结都校验进入模式（§6.2），缺失或非法即报错。切换到一个未冻结的目标时（LLM 自由跳转），驱动者在该次切换前从目录**补冻结**（追加进 `frozen.behaviors`，`config_rev + 1`，worklog 记 `control_applied{behavior_frozen}`）。这保证“用过的 behavior 从首次使用起不再变”，同时不要求提前冻结整个目录。补冻结用的是目录的当前版本，所以 `catalog_rev` 只描述首次冻结；审计看 worklog。

### 6.4 BehaviorAssembler：从冻结 behavior 到一个 run

```rust
pub struct BehaviorAssembler { engine: PromptRenderEngine /* llm_context::prompt_engine，__EXEC 关闭 */ }

#[async_trait]
impl SessionAssembler for BehaviorAssembler {
    /// system 段：身份 → 不可覆盖约束 → behavior.prompt.system 渲染结果（应用 prompt 与 context 并入其中）→ objective。
    /// 只用冻结材料与 session_config；变量集见 6.5；无副作用。
    async fn system_text(&self, cfg: &SessionConfig, agent_root: Option<&Path>) -> Result<String>;
    /// 当前受控输入：按 on_init / on_input / on_context_switch 渲染；缺省使用内建模板。
    async fn render_input(&self, cfg: &SessionConfig, state: &SessionState, templates: &InputTemplates, m: &InputMaterial) -> Result<Option<String>>;
    /// 输入前的半订阅快照：无待注入状态时返回 None；选取和渲染均不修改消费状态。
    async fn render_semi_subscription_snapshot(&self, templates: &InputTemplates, events: &[EventView]) -> Result<Option<String>>;
    /// 目标 behavior 的进入配置。没有声明进入模式 → Err（配置错误，不回退）；入口 behavior 未声明时是 switch_context。
    fn behavior_entry(&self, cfg: &SessionConfig, target: &str) -> Result<BehaviorEntry> {
        frozen_behavior_entry(cfg, target) // 将冻结材料转换为现有 BehaviorEntry（含 prompt / input / llm_context / inherit）
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

上述签名沿用现有 `SessionAssembler`，`frozen_behavior_entry` 是目标装配步骤的示意。复用 `DefaultAssembler` 的视图、模板渲染与内建格式；冻结只改变材料来源。新 run 装配时先把目标 behavior 的冻结覆盖叠到有效 cfg，再渲染 system，不能用入口 behavior 的模板覆盖所有目标。

输入注入顺序固定为：确定受控输入 → 选取并渲染 `semi_subscription_snapshot` → 快照 user message（没有则省略）→ 受控输入 user message（文本加可选媒体）→ 两条消息一次 inject，与 receipt 同快照提交 → 继续推理。受控输入文本可先渲染，必须保证实际注入时快照在前。快照没有独立 hook 或 Turn；未提交时不清状态，已发布快照的批次按 receipt 补交，见 §9.5。

这就是 §3 表“配置来源”一格的变化：`prepare_hosted(workdir, behavior.overlay_llm_context(&cfg.prompt.llm_context), …)`。叠加结果进 run.json `config`，xllm 接手时不需要理解 behavior。`budget.max_wallclock_ms` / `max_total_tokens` 直接进 `hosted_request` 的 `BudgetSpec`（需要 `hosted_request` 接受 budget 覆盖，见 §11）。

### 6.5 渲染变量：哪些冻结、哪些新鲜

| 进 system（冻结 / 构造期确定） | 进输入批次（每批现算） |
|---|---|
| `identity.*`、`behavior.{name,objective,mode}`、`session.{id,kind,objective,driver,scope,parent}`、`paths.{session_root,workspace_root}`（相对或 binding 提供）、`workspace.id`、`xml_behavior_result_protocol`（由 xllm `runtime_protocol` 段提供） | `runtime.{clock_text,status}`、`<active_sessions>`、`<hints>`、`semi_subscription_snapshot`（pending_events 的选定版本）、`<perceptions>`、`<inputs>`、`context_switch / process_result`、`session.current_todo*`（读 `todos.json`）、`notebook.last_items`、`workspace_list` |

上表包含后续宿主变量。当前 libopendan 已提供 `input.*`、`session.{id,kind,objective,timezone,is_bootstrap,current_todo,background_hint_changed,default_changed_background_hint_text}`、`runtime.{status,clock_text}`、`handover` 与内建块文本；todo / 背景提示没有数据源，分别为 null / false，不能据此声称 xagent 已读取 todos.json。额外变量由对应宿主明确装配，不在 CLI 中猜测 OpenDAN 的状态路径。

渲染复用八个命名格式：`input.xml`、`message.xml`、`message.markdown`、`event.xml`、`event.summary_text`、`attachments.xml`、`attachments.list`、`todo.summary_xml`。`input.text` 与 `input.xml` 使用同一渲染器；XML 正文 / 属性分别转义，块标签独占行不输出空行，整体 trim，未知格式 / 错误形状报模板错误。内建时间为 UTC；用户时区来自绑定的 `session.timezone` 与默认 semi 订阅，不取执行机时区。逐字节结果以 `14_input_bus` fixtures 为准。

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

### 7.2 Runner 读写 Agent State 的边界

| 门面 | 读 | 写（驱动者） |
|---|---|---|
| `sessions()` | 登记门槛 `lookup`、每批 `me`、订阅的 session、`children_of`（新：子 session 状态，§4.14）、`scope_touching` | 每次提交后 `report_state`；创建时 `register` |
| `activity()` | 受控输入现算完整 `<active_sessions>` | （活动摘要经 `report_state` 回报） |
| `perception()` | self_improve bootstrap 窗口、`last_seq` 补发 | perception 输入、`finish_run` 的 run_digest / task_outcome |
| `cognition()` | bootstrap 或有新 msg/event 时 `recall_hints` | self_improve 成功时 `commit_consolidation` |
| `artifacts()` | `head` / `version` | `register_version`、`decide` |
| `locks()` | — | `self_improve`、`artifact:<aid>` |
| `behaviors()`（新） | 冻结时 `identity / get / revision`；补冻结时 `get` | — |
| `runtime_grants()`（后移） | policy 阶段再接入 | 用户 / OpenDAN 签发；不纳入当前 trait 必需方法 |

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
                 常驻：drive(Idle) → 持久候选 / 通知 / 有界轮询 → drive(Idle)；同时调度子 session
  xagent post   <sid> (--msg <text> [--from <did>] [--attach <obj_id>[=<name>]]… [--reply-to <obj_id>] | --json <file | ->)
                 Agent 输入：只接受 MsgObject 消息 / event 逻辑记录
  xagent ctl    <sid> (stop | decide accept|discard | subscribe <spec> | unsubscribe <id> | activity ... | perceive <text>)
                 Session 控制协议
  xagent status <sid> [--worklog <n>] [--report] [--run] [--events]    状态；--events 显示 pending_events 与最近注入
  xagent list   --agent <did> [--active]
  xagent behaviors --agent <did> [--frozen <sid>]
  xagent xllm   <sid> [--run <id>]                                     打印让 xllm 接手 live run 的命令行
  xagent schema <dir>                                               导出现有 protocol::json_schemas()

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

`post` 与 `ctl` 分开，对应 §4 的两套协议；虽然 `parse_logical_record` 能解析三种 type，`post --json` 必须拒绝 control。`run --event` 接收完整 event 逻辑记录，`--msg-file` 读取 UTF-8 正文并调用同一消息 helper；都经登记表投递。重试 `--json` 保留原 key。`--attach` 首版只收 ObjId，不实现本机文件上传或 NamedStore 登记。`ctl` 对无队列 Session 只接受 `decide`；`approve / grant / revoke` 随 §5.4 后移，不在本轮 help 中广告。

`<spec>` 的 session 拉取订阅当前只能 Observe，CLI 对 `active:session:…` 明确报配置错误，直到为主动拉取补齐持久候选协议；父子 Input 由 C15 单独实现。`--no-bridge` 关闭 kevent / timer 等外部投递桥，不关闭输入轮询、StopMonitor、普通 task resolver 或恢复所需的内置查询。schema 导出、旧 `agent-session` 工具子命令及环境变量随旧 CLI 一并迁移。

退出码（对齐 xllm 的分类）：

| 码 | 含义 |
|---|---|
| 0 | Turn 以 `completed` 关闭，或 Session 成功完成；只读 / 投递 / 创建不运行等命令成功 |
| 1 | Turn 以 `failed` / `budget_exhausted` 关闭（含 work 模板下的 `WAIT_USER_MSG`），或 session 结束为 Failed |
| 2 | 参数 / 配置 / 模板 / 冻结校验错误；失败的输入批次未消费，不表示此前没有应用控制或恢复提交 |
| 3 | Idle / TurnOpen / OutcomesHandled：未正常完成所请求的推进，或调试停止条件已达到；等待用户、工具或子 Session 可再次 run；投递 input_full 也以可重试诊断返回 3 |
| 4 | 被 stop / 中断 |
| 5 | Busy：session lease 或 run 锁被他人持有 |
| 6 | 阻塞：NotDriver / Unregistered / BindFailed / RuntimeMismatch / RecoveryBlocked / session_readonly |

驱动命令 `--format json` 时 stdout 是带 kind 的 `DriveResult` JSON，各变体仅包含自己的字段；新增 TurnClosed / TurnOpen 带 §4.9 的字段。日志与诊断走 stderr，不能混入 JSON。现有 Idle 的详细等待信息可由 `status` 读取 state.waiting_for；不把本地推测补写回 state。

---

## 9. CLI 接线与 Turn Loop 实施参考

本节描述目标阶段与必须保留的提交边界；伪代码中的阶段函数不是新增 API 清单。实际入口是 `SessionRunner::new(deps).drive(&sd, until)` 与现有 `runner/` 模块。输入协议、receipt、普通 task 等待与 Context 调度复用基线；标为 C4–C16 的扩展在库内完成，CLI 负责参数、宿主生命周期和结果展示。不要照旧草案重新实现一套循环。

### 9.1 组件与状态

| 职责 | 现有实现 / 扩展位置 |
|---|---|
| 推进权、目录提交点、登记表回报 | `SessionDir`、`Session`、lease、`runner/shared.rs::Shared` |
| 输入通道与唤醒 | `InputChannelFactory`、`InputSource`、`Waker`；外部 sources 可为空；C4 初始 source 另从配置构造，通知与持久输入分别处理 |
| 可续跑的 context | `LiveCtx`：`ready` 表示无新输入也可继续；`filled` 表示刚回填 ToolResults，先续工具批次 |
| 串行等待 | `WaitingRun`：run、snapshot、deps、resolver、计数器；没有运行中的 LLMContext。`Opened::Ctx / Waiting` 表达恢复结果 |
| 后台 task | `state.watched_tasks` 保存跨 run 查询的 task id；不等于 pending calls，不补写已完成调用的 ToolResult |
| 输入视图与装配 | `InputMaterial / InputView`、`InputTemplates`、`DefaultAssembler`；C7 的 BehaviorAssembler 接入冻结材料，复用格式与媒体逻辑 |
| CLI 宿主 | 长生命周期的 `RunnerDeps / XllmDeps`、bridge、C15 ChildDriver 与 session resolver；均不写别的 Session 的 state |

一个 Session 同时最多有一个 live 或 waiting run；parked / caller 帧由 `process_stack` 保存。等待对象从 run 快照的 pending calls 重建，`waiting_for` 是提交给状态视图的摘要，不新增另一套 pending-task 协议。恢复必须先校验 Session / run 版本、锁、绑定和提交门槛。

### 9.2 xagent 入口

```text
parse CLI
  → 解析 who / agent / 定位 hint
  → 按命令选择只读、投递、创建或驱动入口

status / list / behaviors / xllm / schema
  → 只读或导出；不为展示而冻结配置、绑定 runtime 或推进 session
post / ctl
  → MsgObject helper / parse_logical_record / ControlCommand
  → 校验命令允许的 type
  → AgentStateClient.sessions().post_input（无队列 decide 走 artifacts 门面）
new
  → SessionTemplate + BehaviorCatalog 冻结（C5 / C7）
  → 构造 SessionSpec、确定是否建队列（C4）、create_session
  → 有队列的 --msg 经 post_input；无队列的初始材料随配置持久化
  → --no-run 时退出，否则 drive(--until，默认 TurnClosed)
run
  → 定位 Session → 可选投递 → drive(--until，默认 TurnClosed)
  → C15：接管子 session，按 --detach-children 决定返回时机
serve
  → 建立 bridge / ChildDriver 宿主 → 按 §9.6 调度 drive(Idle)
```

每个驱动命令构造 `RunnerDeps`，设置 assembler、runtime、任务 resolver 与 `session_cli = current_exe()`，由现有 bin overlay 包装成 `agent-session`。不要另建 Runtime 或工具派发协议；目标 behavior 的工具与模型按每个 run 的有效配置装配。

无队列 bootstrap 的持久输入（新增 C4）：创建前用同一消息 helper 构造 `PostedInput::msg`，写入目标字段 `prompt.initial_inputs: Vec<PostedInput>`，只接受合法 msg，最多 64 条；随 SessionConfig 一起发布，发布后不修改。这是待实施的 schema 扩展，当前 `/4` 没有该字段。Runner 将它适配为内部只读 source `_bootstrap`，index 从 1 开始，复用 SourceProgress / route_inputs / receipt；无需 kmsg，`channels.inputs` 仍为空，confirm 为空操作。Single 首批未选中的记录在后续 on_input 消费，不能因 bootstrap_done 就丢弃。初始消息与普通消息同样渲染为 user 输入，不挪进 system 的 prompt.context。

`new --no-run --msg` 必须在退出前落盘；已有无队列 Session 的 `run --msg / --event / --msg-file` 与 post 拒绝投递，提示在创建时提供材料或使用 Queue 模板。E9 的三种外部投递等价在有队列 Session 验证；E13 另验内部初始 source 的逐条消费与恢复。

### 9.3 Agent Turn Loop（阶段边界）

```text
0. 取得 lease，核对 driver / 登记 / kind lease；准备并核验 Runtime 绑定。
1. reconcile：截断未提交 worklog、恢复 run、补 receipt、清门槛、重做已落盘 Outcome。
   若本次恢复关闭了 Turn 且 until = TurnClosed，按 C6 返回；否则继续，之后才读新输入。
2. 从快照取得 pending task ids，按顺序 route_inputs；finished 时处理允许的控制后返回。
   必要时补冻结（C7）；打开恢复的 run → Opened::Ctx 或 Opened::Waiting。
3. 循环：
   a. 检查 lease / stop / MaxOutcomes；stop 走 §9.5 的配对与提交路径。
   b. 若有 WaitingRun：try_fill。
      - 仍等待：提交 waiting_for(tool, refs, deadline)，按 poll_interval / until_ms 等待；
        有任务通知则立即查询；重新顺序路由（带 pending ids），回到 a。
      - 超过 max_wait：Idle / Finished 返回 Idle；TurnClosed 返回真实 TurnOpen。
      - 可回填：发布 ToolResults 快照，得到 ready = true、filled = true 的 LiveCtx。
   c. 父工具调用返回时先 open_state_live_run，亦可能得到 WaitingRun。
      hold_inputs = tool_return_pending || child_call || live.filled。
   d. 校验 / 补冻结当前目标 behavior，取得 InputTemplates 与 InputSection。
      不 hold 时，从已路由候选 + poll_watched_tasks 的候选中按 Single / Batch 选批。
      C15 的子状态候选也在此并入；未选中的候选保持未消费。
   e. 选入口：on_init → on_context_switch → on_input。
      构造 InputView、hints、active、runtime、UTC 时间；渲染受控输入文本和媒体块。
      无受控输入、无 ready run 时，按下表返回或有界等待后重新路由。
   f. 取 context：已有 live → 打开 state.live_run → 新建 run（§9.4）。
      若打开后仍为 WaitingRun，保留未提交的输入，回到 a；不提交到挂起工具批次中。
   g. 有受控输入时，选取并渲染半订阅快照；两部分消息与 receipt 同批提交（§9.5）。
   h. 清 ready / filled，启动 StopMonitor，执行 run_compacting。
      停止监视任务；若它发现 stop，由驱动者消费控制并提交，再解释 Outcome。
   i. handle_context_outcome：调度 Context / 关闭 Turn / 保存等待，outcomes += 1。
      普通 PendingTool 将 LiveCtx 转为 WaitingRun；不能继续对挂起 ctx 调 run()。
   j. 本次 Turn 已关闭且 until = TurnClosed → 返回 TurnClosed（优先于 Finished）。
      否则 finished / error → 返回；其余重新 route_inputs 后回到 a。
```

`route_inputs` 必须按配置的 source 顺序、每源 index 顺序遍历，在同一遍里执行 control、判断 event、保留 msg。事件 A 已按 active 接受后，后续 unsubscribe 不改变 A；保存到 `inputs[src].accepted` 的候选不再按当前订阅重判。未订阅事件消费并记 `event_dropped`；pending call 的 task 通知按精确 task_id 匹配，只提前触发查询，不再次作为事件注入。

| 无可提交受控输入、也无可续跑 context 时 | 返回 / 调度 |
|---|---|
| `Idle` | 返回 Idle；进入此分支前已查看候选，不能仅凭上个 Outcome 是等待输入就返回 |
| `MaxOutcomes` | 返回 OutcomesHandled；保留等待状态 |
| `TurnClosed`（新增 C6） | 本次已关闭则 TurnClosed；确有 open_turn 则 TurnOpen；否则 Idle |
| `Finished` | 可重试错误按已有错误路径返回；否则有界等待并轮询，最多 max_wait |
| 无队列且没有 task / children / 其它可推进来源 | 可以直接返回等待结果；有 task / children 时不能以“无队列”为退出依据 |

装配失败时，未提交输入留在队列；已有选中 msg / Input event 却渲染为空必须报错。无选中外部输入时沿用当前 bootstrap / continuation 规则，交接空文本使用现有继续执行消息。仅保存 Observe 更新、工具完成或到达检查点不形成受控输入。输入装配 / 推理预算不足沿用 AICC 不可用的失败与重试路径；已提交批次以 receipt 恢复，不重新消费。

`select_snapshot` 先选 terminal，再选普通 latest，最多 `change_budget` 个版本；超预算的 terminal 也保留待下次注入，不承诺所有 terminal 必须在本批出现。选取和渲染没有消费副作用。活动 session 列表位于受控输入的新鲜材料中，不在检查点维护“已见活动集合”游标。

### 9.4 ContextFactory：复用 run 装配与恢复

这里的 ContextFactory 是现有 `runner/live.rs` 函数组的职责名。C7 仅接入冻结来源，C12 补齐宿主协议、renderer 与预算；不要把 §5.4 的 guard / grant 作为构造 context 的前置依赖。

| 路径 | 装配 / 恢复步骤 |
|---|---|
| 新根 run / 首次 SWITCH_CONTEXT 目标 | 当前 behavior 的有效配置 → `XllmTask::prepare_hosted` → system 与按规则选择的历史 → run record → `hosted_request / hosted_waist_deps`；Session 设置 `request.tool_policy.allow_deferred = true`；包装 SessionToolManager 与 checkpoint hook |
| create-sub-context | 用目标自己的 system / 模型 / 工具构造 request，调用 `derive_child`；按 inherit 选择 none / recent_dialogue / steps，保存继承边界，继承记录不重复 flush |
| fork | 先分支，使用父 record.config 与 `fork_snapshot`，不叠加目标 system / 模型 / 工具；分叉点在触发工具批次之前，未完成批次留给父 |
| 恢复 parked / 普通 live run | 持 run 锁，检查版本、host_commit_pending 与实际 runtime，按已有执行核验物化未决调用；使用 record.config 重建依赖，不重装 system 或重复执行输入模板 |
| PendingTool | 检查每个 task_id 的 can_resolve；不能解析则 RecoveryBlocked。能解析则构造 WaitingRun 并 try_fill；Running 留在等待，Unknown 是可回填状态，绝不因此重建任务 |
| 工具触发的子 context 返回 | 从 process_result 按 call_id 构造 ToolResults，发布回填快照后清对应返回标记；编号接续父 / 子，设置 filled。行为触发的返回仍通过 on_context_switch |

新 run 的有效配置、工具广告、Runtime descriptor 与宿主环境核验写进 run.json，使 xllm 接手不依赖冻结目录或 Runner 内存。`call_behavior` / `session` task 需要 Session 自己的 resolver，缺少该能力的 xllm 拒绝接手；普通外部 task 按 `can_resolve` 决定，不能一律拒绝所有 PendingTool。xllm 接手时按当时状态回填，不承担 Session 的等待循环。

**一个 run 段中的机械重试**复用 `live.rs::run_compacting`：

1. `ContextLimitReached` 在既有次数上限内重写历史、发布新 epoch 并继续；不产生新的 on_init / on_context_switch。
2. provider 返回永久或未知错误且 run 的 user 消息带媒体块时，`degrade_inline_media` 去掉媒体块、追加说明、保存 `host.libopendan.media_degraded = true` 后重试一次；同一 run 不反复降级，不再次注入原批次。
3. 其它 Outcome 交给 Session 解释。降级 / 压缩内部的再次 run 不单独增加 `MaxOutcomes` 计数，实际推理仍计 Round。

### 9.5 输入提交、Outcome 与 stop

输入装配必须复用 `InputView`、`input_formats`、`media_blocks` 与 `commit_input_batch`：

```text
有序 picked(msg / Input event)
  → InputView（说话人来自 msg.from，保留 ObjId / 回复 / 附件）
  → 选定模板渲染文本；input.media 独立生成媒体块
  → [可选 semi_subscription_snapshot user message,
     controlled user message(text + media)]
  → 一次 ctx.inject → positions_of → InputReceipt
  → 同一快照中的消息 + state.host.libopendan.input_receipts
```

`reference` 只有文本；`inline` 在受控输入文本后按消息 / 附件顺序追加 Image / Document 块，每批最多 8 个。两种模式的文本必须同样可定位附件；模板只给显示名时沿用现有 warning，不将显示名当可读地址。媒体由快照保存，worklog / 新 run 历史只保留文本引用。

receipt 已是当前协议，不再是待定示意：

```jsonc
{
  "run_id": "run-id", "input_seq": 2, "turn": 1, "opens_turn": false,
  "hook": "on_input",
  "inputs": [{"src": "q", "index": 3, "key": "cymsg:…", "kind": "msg"}],
  "events": [{"subscription_id": "watch", "source": {"kind": "object", "id": "doc"}, "seq": 8, "key": "doc:8"}],
  "reply": {"route": "message", "to": "did:bns:alice", "to_session": null, "kind": "chat", "reply_to": "cymsg:…", "tunnel": null},
  "parts": [
    {"part": "semi_subscription_snapshot", "pos": {"kind": "accumulated", "index": 6}, "text": "…"},
    {"part": "input", "pos": {"kind": "accumulated", "index": 7}, "text": "…"}
  ],
  "bootstrap": false, "after_step": 0, "extra": {}, "at_ms": 0
}
```

样例省略真实正文与对象 ID；字段 / 枚举以 [Session Input Protocol §7](<protocol/Session Input Protocol.md#7-输入-receipt-与提交>) 和 Rust 导出的 schema 为准。`parts[].pos` 来自 inject 返回位置，不猜测消息编号；behavior 模式可能是 request_input 或同一 step。`parts[].text` 保存文本，媒体从该位置的同一快照读取。

固定提交顺序：

1. 快照 fsync：含正文、媒体及 receipt。
2. run.json 发布快照指针与 `host_commit_pending = input_seq`。
3. state.json 应用 receipt 并提交：开启 / 并入 Turn、记录消费位置与 recent_keys、清理 accepted、按 `(subscription_id, source, key, seq)` 精确清理半订阅版本、更新 reply 与 bootstrap；`extra.continuation` 清交接标记。内部 `_task` 输入按 key 清对应 watched task，不伪造队列 ack。
4. 清 host_commit_pending。
5. 按已提交消费位置确认输入源。没有输入源则为空操作。

第 4 步前不继续推理或工具执行。恢复先按快照 receipt 补交原批次，不读总线、不重新渲染；已应用序号幂等忽略，序号缺口或 state 指向缺失快照则 RecoveryBlocked。`reply` 取消费顺序最后一条 msg 的来路；只有 event / bootstrap / 交接时沿用旧值，子 Session 无消息来路时使用创建时保存的 parent route。出站信封 helper 已有，持久发送仍随 UI 后移。

Outcome 提交沿用 `outcome.rs` 与 `reconcile.rs`：

| Outcome / 调度 | 提交结果 |
|---|---|
| SWITCH_CONTEXT | 记录 handover、flush 当前增量、park 当前 run 并选择目标 run；同一 Turn，配置不在 run 中途改变 |
| 子 context 进入 / 返回 | caller 帧与 live_run / process_result 一起提交；工具返回经 ToolResults，行为返回经交接批次；子 transcript 不重复进入 session history |
| 普通 PendingTool | 保存快照与 waiting_for，保持 open_turn，转 WaitingRun；不 finish_run |
| WAIT_USER_MSG | C5 按模板解释；允许等待且尚未交付回复时 Turn 保持打开；work 的 FinishFailed 产生 needs_user_input |
| run 正常结束 | checkpoint_finish → flush → `commit_run_end`；Turn 关闭与 run_state、结果、watch 接管同一次 state 提交；`after_run_end` 做后续清理 |
| Session finished | 清 process_stack、pending_events、watched_tasks；写报告 / 产物与结果；不因迟到输入或 task 完成重开 |
| C15 父等待子 | 尚有需汇报的子时不关闭 Turn，保存 waiting_for(children)；此分支待实施，不能用现有普通 Observe 拉取替代 |

当前正常 run 结束从 resolver.active() 接管后台 task；C16 补足 reconcile 从已持久化 call_result 重建的路径。该能力落地前，崩溃恢复测试必须将缺口列为未通过，不能以“正常轮询能完成”代替验证。

**stop 的两个入口共用一条提交路径**：正在执行的 run 由 StopMonitor 查看已入队 stop 并发 interrupt；监视任务不消费、不确认、不写 state，驱动者随后 route_inputs 并提交。WaitingRun 收到 stop 则对当前等待调用传导取消、以权威的当时状态配对 ToolResults，发布快照后按 Stopped 结束；不重放调用，也不为处理 stop 额外推理。SIGINT 应由 CLI 接到同一取消路径。StopMonitor 尚不刷新长工具执行期间的 activity 心跳，不能宣称持续心跳已完成。

**检查点**：发布工具结果快照 → 顺序 route_inputs → stop 检查 → 合并 touching / 刷新心跳。msg / Input event 留到可处理边界，Observe 保存到 pending_events；不注入消息，不维护独立 observation receipt。

### 9.6 常驻形态与宿主生命周期

`serve` 不能只等队列消息。task 在没有通知时也可能完成，session 拉取订阅与 C15 children 也依赖调度。进程级 bridge 在 drive 返回 Idle 后仍存活，重订阅按 config_rev 更新；只在服务结束或真正 unload 时关闭。每个 Session 的 drive 串行，不跨多个 drive 同时持有推进权。

```text
为 targets 建立长期宿主：RunnerDeps / task 服务 / bridge / ChildDriver
循环调度每个 Session：
  drive(Idle)                   # 串行 task 等待内部最多占用 max_wait
  Finished 且控制队列已处理 → 结束服务
  NotDriver / Unregistered / BindFailed / RecoveryBlocked → 报告并退出该 Session
  Busy / RunBusy / LeaseLost → 有界退避后重试
  Error → 输出诊断；仅按已有可重试分类退避，不因返回 Error 就忙循环
  Idle → 重新检查持久候选与待推进状态：
    已有候选输入 → 立即再次 drive
    waiting(tool/children) / watched_tasks / 拉取订阅 → 保留宿主，有界轮询或通知后 drive
    其它 → 等通知，最多 poll_interval；达到 idle_unload 且无待推进工作才卸载
退出时回收 bridge / ChildDriver 的本进程资源，并释放 lease / run 锁
```

C4 默认无队列 work 用 `run`；如果由父的 ChildDriver 调度，无输入队列仍可通过 resolver / 登记表推进。`run` 在 max_wait 到达而任务尚未结束时可以按 §8 返回等待结果；CLI 必须显示 waiting_for，不承诺本进程退出后进程内 task 继续。对可持久查询的外部 task，下次 run 按 task_id 查询恢复。`--detach-children` 立即返回时只保证子已持久登记，输出仍需托管的 sid，后续由 serve / Supervisor 接管；不承诺本进程中的子协程或 task 继续运行。

---

## 10. 验证矩阵：库级基线与 CLI 验收

以下是 xagent 的验收要求，**不是已运行结果**。输入 TODO §10 记录的验证为：libopendan 单元 18 + 集成 109、llm_context 209、opendan / agent_tool 构建；1 个真实 kmsg ignored 用例未跑。后续媒体降级有新增用例，实施者应记录自己运行时的数量与结果，不照抄历史数字。

| 基线证据 | 已有覆盖与限制 |
|---|---|
| `tests/input_tasks.rs` | task 等待 / Unknown / 无 resolver / stop、后台完成、长工具 stop、容量、Single / Batch、inline、去重 / reply、旧版本只读、模板失败、时区、链式 PendingTool、媒体单次降级 |
| `tests/runner_more.rs`、`tests/context_switch.rs` | 事件路由与订阅顺序、半订阅、behavior 切换与子 context、独立配置 / 历史 |
| `tests/crash.rs`、`tests/fixtures.rs` | 已有提交窗口、receipt 恢复与 14_input_bus 的逐字节结果 |
| 仍需补齐 | `on_context_switch + 半订阅快照` 独立端到端；reconcile 找回遗漏 watched_tasks；全部 xagent 进程级命令验收 |

保留实验编号，便于与旧计划对应。每项都检查 CLI 返回码 / JSON 与磁盘提交结果；不能只看终端文字。发现边界问题时先修订设计，不靠 CLI 特例绕开。

| # | 分类 / 边界 | 实验与通过条件 |
|---|---|---|
| E1 | 验收：xllm ↔ Session | mock 在 `--until outcomes:1` 停下；`xagent xllm` 打印命令，xllm 接手到终态，再 xagent 恢复。配置 / system 不重装配，worklog / Turn 恰好提交一次，Round 累加。xllm 不需要解析 Session 模板 / receipt |
| E2 | 回归：冻结 | 创建后修改 behavior 文件，推进同一 behavior 的新 run，再建新 Session；旧冻结不变，新 Session 使用新配置。补冻结按首次使用版本记录 |
| E3 | 回归：Runtime 绑定 | native / tmux 分别推进相同配置；已绑定后换 runtime → 退出码 6，推理前失败 |
| E4 | 验收：Agent State | 文件、进程内、kRPC 转发桩运行同一 fixtures，结果一致；Runner 不按具体实现分支 |
| E5 | 验收：工具形态 | 仅层 ② 工具时 xllm 可接手；注入层 ③ echo 时 app_tools 非空、xllm 明确拒绝 |
| E6 | 回归：Turn / 等用户 | ui 测试模板 WAIT_USER_MSG 无 report → 退出 3、open_turn 保留；post 后在同一 Turn 回复并 completed，计数只增一次 |
| E7 | 回归：输入崩溃 | 输入快照 / state / 清门槛 / ack 各窗口故障，恢复不重复注入、不倒退位置、延续同一 Turn；门槛期间 xllm 拒绝接手 |
| E8 | 回归：推进权 | 两个 run 或 serve 与 run 争抢同一 Session，后者 Busy（5）；不同 Session 可并行 |
| E9 | 回归：投递等价 | Queue Session 的 run --msg、post 后 run、serve 中 post 使用同内容合法 MsgObject，视图 / receipt 结构与提交语义相同 |
| E10 | 回归：调度不断 Turn | plan → create_sub_context do → 返回 → switch_context review → plan → review → END，全程一个 Turn，重复进入 review 恢复其自己的 run |
| E11 | 回归：新鲜量 | 同一 behavior 的两次 Turn 间隔运行，system 前缀相同；时间在输入，UTC 不受 Runner 机器时区影响 |
| E12 | 回归：缺失冻结 | 无 frozen 且目录不可读 → RecoveryBlocked（6），不猜配置、不推理 |
| E13 | 验收：无队列 work | 默认 work 无外部订阅，new --no-run --msg 后退出并重新 run：不建队列，初始输入不丢；WAIT_USER_MSG → needs_user_input（1）；无队列仍能等待 resolver，decide 可走 artifacts 门面 |
| E14 | 验收：半订阅 | run 中 / 空闲 / Turn 间到达 semi 更新，分别配合 on_init / on_input / on_context_switch；快照在前且与受控输入同 receipt。覆盖 v7 提交期间到 v8、无 seq 按 key 清理、terminal 超预算保留、崩溃补交；检查点不独立注入，active 正常开启 / 并入 Turn |
| E15 | 回归：控制分离 | 运行与空闲中 ctl stop / subscribe / activity，不作为 LLM 输入、不新开 Turn；stop 允许关闭现有 Turn 并写停止记录。post --json 拒绝 control，ctl 不接受 msg / event |
| E16 | 回归：session 拉取 | semi 关注无父子关系的 Session；对方结束，变化保存到 pending_events，在下一受控输入前展示。不把已有拉取桥验收为主动 Input；父子推送另看 E21 |
| E17 | 已有 Runtime 基线 | native / tmux / SSH 的 exec 与文件工具共享目标 cwd；SSH 不改 Runner 本地同名文件。远端 Session helper 未部署时明确拒绝，不因独立 xllm 的 SSH 可用而承诺 Session 可用 |
| E18 | 后移：policy | guard 拒绝、审批 PendingTool / approve、grant 到期撤销；随 ActionGuard / RuntimeGrant 实施，本轮不提供这些 CLI 子命令 |
| E19 | 验收：SWITCH_CONTEXT | do / check 使用不同模型与工具，往返后只有各自两个 run，配置、历史、编号、预算不串；xllm 到 handover 停为 paused，再次 resume 拒绝，xagent 提交转移一次；转移前后崩溃同样成立；未声明进入模式失败，无同 run 换 system 回退 |
| E20 | 验收：工具子 context | function_call / behavior 父分别调用 create_sub_context（none/recent_dialogue/steps）与 fork；与 read 同批，进入 / 返回前后崩溃。父以 subctx:<call_id> 挂起，fork 前缀截止触发批次前；子结果恰好回填一次，父剩余调用继续、已执行不重放。继承记录不重复 flush，子 transcript 不进重建历史，子 WAIT_USER_MSG / Error 交回 needs_user_input / failed，不关闭父 Turn；嵌套超限及 fork 自带配置被拒，xllm 缺 Session resolver 时拒接 |
| E21 | 验收：Sub Session | 无队列父派出 final / progress / --wait 三种子；各有 lease / Turn，progress 半订阅，wait 以 ToolResults 回填；父 END 时仍有需汇报子则 Turn 保持打开，结束事件 Input 后汇总。kill 宿主后登记与提交点可恢复、子不重复创建；同进程子协程也会停止，不宣称进程独立存活 |
| E22 | 回归：父子对话 / stop | interactive 子提问，父收到 needs_input 后 post，子续同一 Turn；子主动 post 要求父有队列。父 stop 级联未结束子，各自驱动者提交 stopped |
| E23 | 回归：模板 | 三入口分别渲染，Single / Batch 仅消费选中输入；启动 / 交接可合并外部输入；恢复、压缩、ToolResults 不重复触发入口；六个示例模板输出与 fixtures 一致 |
| E24 | 验收：普通 PendingTool | 无通知 / 空 inbox 时查询终态并续同 run / Turn；无 resolver 阻塞、Unknown 回填；until_ms 到期回填当时状态。等待中 msg 暂存，回填后先续工具批次，再次 PendingTool 不丢关联；stop 配对且不重放 |
| E25 | 验收：顺序 / 背压 / stop | A → unsubscribe → B 在不同 fetch 分批下结果相同；并发 producer 经登记表争抢第 64 个名额，第 65 条 input_full、bridge 不确认上游；普通输入后已入队的 stop 能打断长工具；不承诺满队列仍能投 stop |
| E26 | 验收：媒体 / reply | 同输入 reference / inline 文本相同，图片文档按序追加、上限 8，reply 指向最后 msg；重启用 receipt 恢复来路。provider 拒绝只降级一次，文本仍能定位 ObjId；新 run 历史不重新内联媒体 |
| E27 | 验收：常驻 / 后台恢复 | Idle 前已有输入继续处理；任务离线完成无通知也可查询。run 终态已落盘、state 尚未接管 watch 时崩溃，reconcile 从 call_result 找回并只交付一次（C16 未完成前此项不得标通过）；waiting task / children 不因空队列 idle unload |
| E28 | 验收：CLI 契约 | 测 new --no-run、run 各 until、post 文件/stdin、ctl、schema、状态与 xllm 命令；覆盖 §8 返回码、stdout 单个 JSON / stderr 日志、旧版本只读、终态迟到输入、无队列投递拒绝、SIGINT；旧 helper 与脚本全部迁移 |

---

## 11. 实现状态、剩余工作与实施顺序

### 11.1 状态清单

“已实施”指 §1.5 的库级基线，不代表 xagent 命令或全部验收已完成。实施时保留既有语义，只完成“剩余工作”列；不要重新执行已经结束的 Input / Turn Loop TODO。

| # | 当前状态 | xagent 剩余工作 | 主要入口 |
|---|---|---|---|
| C1 | 已实施：MsgObject / AgentEvent / Control、输入 `/3`、校验拒绝、去重、64 条 pending 检查、旧 Session 只读 | CLI / bridge 经 post_input；暴露 input_full / session_readonly；明确跨主机与直接 kmsg 写入的限制 | `protocol/input.rs`、`channel/kmsg.rs`、`state/registry.rs` |
| C2 | 已实施：顺序路由、accepted、pending_events、三类入口、双消息 receipt、reply、UTC / 用户时区 | 接到冻结 behavior；保留精确按 key / seq 清理和消费后 ack；补 E14 的 context-switch 快照场景。覆盖计数为 pending 状态中的 superseded，不要求新增 event_superseded worklog | `runner/{inputs,input_view,assembler,live,receipts,flush,hook}.rs` |
| C3 | 部分完成：session 拉取一律 Observe；后台 task 轮询；msg / task bridge 纯函数 | xagent kevent / timer 桥与宿主生命周期；父子隐式 Input 见 C15；后台恢复缺口见 C16。EventBridge 若需抽象在这里定义，当前无可直接调用的同名 trait | `runner/inputs.rs`、`runner/drive.rs`、`bridge/` |
| C4 | 部分基础已有：无 queue_client 时 create_session 可不建队列，Runner 接受空 sources；尚无模板级开关 | SessionSpec / 模板明确控制 Queue / None，即使宿主有 kmsg client 也能不建；按 §9.2 持久化无队列初始材料；task / children 等待不依赖队列 | `api.rs::create_session`、`channel/`、`runner/drive.rs` |
| C5 | 待实施：SessionTemplate 与冻结 policy | 四模板、agent.toml 覆盖；wait_user_msg / observe / load_hints；默认隐式订阅与队列需求；work 的 WAIT_USER_MSG 记 needs_user_input | `protocol/config.rs`、新增模板模块、`runner/outcome.rs::classify_done` |
| C6 | 待实施：TurnClosed / TurnOpen | 返回条件与持久 Turn 状态分离；包括 reconcile 关闭的 Turn、Session 同时 finished、工具等待超时、无 open_turn 的 Idle；退出码按 §8 | `runner/{mod,drive,outcome,reconcile}.rs` |
| C7 | 部分完成：prompt.system、输入模板、input.mode/media、目标进入配置与校验、旧 process_modes 拒绝 | BehaviorCatalog、behavior 配置解析、冻结 / 补冻结、BehaviorAssembler；将冻结材料映射为现有 BehaviorEntry 与有效 cfg；复用现有渲染 / 媒体 helper | `state/`、`protocol/config.rs`、`runner/assembler.rs` |
| C8 | 已实施：共享 Runtime、native/tmux/SSH、文件后端、绑定与 SessionToolManager | 接入 CLI；ActionGuard / grant / 审批后移，不是本轮依赖 | `agent_tool/runtime`、`lib_opendan/runtime`、`runner/tools.rs` |
| C9 | 待实施：connect / StateLocator、进程内实现、kRPC 转发桩 | 按 §7 定位；Runner 继续依赖 AgentStateClient trait，真 kRPC 服务后移 | `state/mod.rs`、新增 `state/connect.rs` 等 |
| C10 | 共享 RuntimeRegistry 已有 | CLI 的 runtime 选择接入共享构造与绑定要求；不再在 libopendan 新建 Registry；不支持的执行体报 Capability | `agent_tool/runtime`、`lib_opendan/runtime/mod.rs` |
| C11 | 待实施：xagent 二进制与新命令面；旧 example 已支持消息 helper / post --json 等 | `src/bin/xagent.rs`、new/run/serve/post/ctl/status/list/behaviors/xllm/schema、层 ② 子命令；迁移 bin helper、测试脚本及文档后删除旧 CLI | `examples/session.rs`、`runtime/bin_overlay.rs`、`Cargo.toml` |
| C12 | 部分完成：Runtime hosted/resume、Session allow_deferred、普通 task resolver 恢复 | 补 HostProtocolFlavor::Session、renderer_opts 一致性、behavior 预算接线；不重复实现 deferred；session resolver 在 C15 | `agent_tool/src/xllm.rs`、`runner/live.rs` |
| C13 | 输入 / 控制 Spec、schema、14_input_bus 与已有 fixtures 已反写 | 按本轮冻结 / policy / origin / 等待扩展再次升版与生成；同步实际 CLI 用法和 README 版本摘要 | `doc/opendan/protocol/`、`doc/llm_context/`、`lib_opendan/README.md` |
| C14 | Context 调度与恢复已实施：parked/caller、call_behavior、derive_child/fork_snapshot、handover、子历史过滤 | 接 C7 冻结来源与模板深度配置，跑 E19/E20；UI 暂停后补充输入及显式 report 完成策略后移 | `runner/{live,outcome,reconcile,tools,history}.rs` |
| C15 | 待实施：Sub Session 协作 | 创建参数、幂等身份 / 数量 / 深度、children_of、origin.report、父子 Input 接受与 receipt 游标、session resolver、ChildDriver、父等待与 stop 级联；替换向父投 subscribe 的旧路径 | `api.rs`、`state/registry.rs`、`protocol/{config,state}.rs`、`runner/`、`bin/xagent.rs` |
| C16 | 部分完成：WaitingRun / try_fill、Unknown、StopMonitor、watched_tasks 正常接管 | serve 有界轮询与 task 宿主保活；补 reconcile 从快照 call_result 找回已 watch task 的缺口并做崩溃验收；不重建 / 重放外部任务 | `runner/{shared,drive,live,outcome,reconcile}.rs` |

### 11.2 建议实施顺序

1. **保留基线并建 CLI 骨架**：C11 的只读 / schema / 投递 / 现有 run 入口，复用 C1/C2/C8/C10；先用现有 StopWhen 验证命令与错误输出，TurnClosed 完成前不把其它条件冒充 `--until turn`。
2. **创建与配置**：C9 的文件定位路径、C4/C5/C7；在新的 schema 上落实冻结、队列选择、bootstrap 材料及模板语义。为 tests 提供进程内 AgentState 实现，转发桩按 E4 验证。
3. **Turn 返回与交接**：C6、C12 剩余项、C14 接线；完成 E1/E2/E6/E10/E13/E19/E20/E23。
4. **常驻与普通任务**：C3 的 kevent / timer 宿主、C16；完成 E14/E24–E28。先补可靠恢复，再宣称常驻支持后台任务跨 run / 重启。
5. **Sub Session**：C15，完成 E21/E22；复用 WaitingRun 与普通 resolver 查询循环，不另造等待状态机。
6. **反写与收尾**：C13 随每次协议变更更新，最后确认 CLI 命令替换、schema 导出、fixtures、版本拒绝和旧入口删除。冻结、policy、origin 的新增 schema 基于 config `/4`、state `/5` 继续演进，不要求再改已经定稿的输入 `/3`。

在 `src/` 下运行 `cargo test -p libopendan -- --test-threads=1`、`cargo build -p libopendan --bin xagent`；触及 llm_context / agent_tool 共享装配时增加对应 crate 的测试与构建。库级用例通过后执行 §10 的实际 CLI 场景，记录返回码、JSON、磁盘状态与故障恢复结果。真实 kmsg 的 ignored 用例单独记录运行环境与结果，文件队列通过不能替代它。

### 11.3 后移项与保证范围

- 正式 msg-center / TaskMgr 接入、出站持久记录与 channels.outbound 发送、外部服务 dispatch intent / task 绑定；现有桥仅是映射与幂等 key helper。
- 本机附件登记到 NamedStore；首版 CLI 的 --attach 仅接受 ObjId。
- 长工具执行期间持续 activity 心跳；StopMonitor 当前仅查看 stop，不写 state。
- 旧 Session 的显式迁移工具、TS helper / Runner、正式 UI / OpenDAN 改造。
- ActionGuard / grant / 审批、额外 Runtime 类型与远端 Session helper 部署；E18 单列后移。

这些限制不会被“输入 TODO 已完成”自动解除。C16 的 watched_tasks 恢复缺口列入本轮剩余工作；`on_context_switch + 半订阅快照` 的独立端到端用例列入 E14。其它后移项不作为首版 CLI 完成条件，也不能在 help / status 中宣称已支持。

---

## 12. 默认选择与后续待定项

保留原问题编号。标为“默认”的条目沿用本文已有选择指导本轮实施，不要求 Code Agent 再把所有历史问题逐一询问；实施发现不可行或需改变协议语义时，先修订对应设计。标为“后移”的条目不阻塞首版 CLI。

1. **默认：冻结范围**。入口 + meta.next 闭包，首次使用未冻结目标时补冻结；记录版本与审计，不冻结整个目录。
2. **默认：loop_mode 归属**。使用 behavior.prompt.mode；每个 run 一种 loop。fork 沿用父配置，steps 继承遵循 llm_context 的模式约束。
3. **默认：work 的 WAIT_USER_MSG**。FinishFailed，问题写 report，Turn failed{needs_user_input}；不影响普通 task 等待。
4. **默认：无队列 work**。保留 §4.7 的能力边界；SIGINT 走持有者取消，decide 走 artifacts，activity / perceive 不另建隐藏通道。初始输入必须持久化（§9.2）。
5. **已实现：控制协议搭输入队列**。沿用 msg / event / control 同一通道和顺序路由，不新增控制 kRPC。
6. **已实现：AgentEvent 限制**。summary 最多 1024 字节；data_ref 可为 ObjId 或相对 Session 的路径，按当前校验处理。
7. **默认：层 ③ 工具**。暂无正式成员，E5b 用 echo；call_behavior 属于 Session Context 调度，工具中不执行推理。
8. **默认：二进制位置**。`src/frame/lib_opendan/src/bin/xagent.rs`，沿用当前 crate 与 §1 的定位。
9. **后移：grant 呈现**。保留 §5.4 的带前缀并列工具方案供 policy 阶段评审。
10. **后移：层 ② guard 粒度**。Agent State 级授权检查随 policy 设计，不把未实现的 ActionGuard 接口加成本轮依赖。
11. **默认：StateLocator**。文件定位用 §7 的环境变量 / 映射表；进程内与转发桩验证 trait，真 kRPC 随 OpenDAN 改造。
12. **后移：UI Stop 后补充输入**（[Context TODO](../../notepads/llm-context-switch-support-todo.md) H3）。保留原 Turn / run 或重开 Turn 的选择未定；本轮 ctl stop 仍终止 Turn 与 Session，不暗改为暂停。
13. **后移：report 工具参数与 is_end**（Context TODO H4）。显式报告、可选 result 校验、同批剩余工具与 task 的关系未定。
14. **后移：显式完成策略的模板范围**（Context TODO H4）。本轮沿用 end_condition 与 §4.7 的 WAIT_USER_MSG 解释，不让普通 Done 自动获得另一种完成语义。
15. **默认：子 context 嵌套范围**。沿用现行规则：可再调用子 context；跳到 switch_context 目标交回调用方，WAIT_USER_MSG 交回 needs_user_input；扩展内部切换或直接等用户后移。
16. **默认：进入配置落点**。冻结保存 BehaviorConfig，生成现有 `extensions.opendan.behaviors.<name>` 的 `{mode,prompt{system,…},input{mode,media},llm_context,inherit}`；冻结 Session 拒绝冲突的应用覆盖。未采用冻结的库调用仍使用现有 extensions 入口，不增加旧字段兼容。
17. **默认：子 workspace**。inherit，可明确选择 new / id；共用时依靠活动视图协调。
18. **默认：父结束规则**。有 report != none 的未结束子时，父 Turn 保持打开等汇报。
19. **默认：父对子隐式关注**。C15 取代创建时向父投 subscribe；父可无队列，但 Input 接受 / 消费必须可恢复。
20. **默认：run 推进子 session**。按 §4.12 推进到空闲或结束；detach 只保证子已持久登记，后续由 serve / Supervisor 接管，不保证同进程任务在 CLI 退出后继续。
21. **默认：子的用户是父**。子 headless；面向人的通道与例外随 UI 后移。
22. **默认：数量与深度**。max_sub_sessions = 4、max_session_depth = 2；Context 调度深度是另一项限制。
