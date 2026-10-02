# xAgent：Agent Session 分层验证工具设计

- 状态：草案 v0.1（2026-10-02），待 Review 后开始实施
- 位置：`src/frame/lib_opendan`（二进制 `xagent`，替代 `examples/session.rs`）
- 依据：[Agent Session SDK 实现计划](<./Agent Session SDK 实现计划.md>) v0.10、[LLM Context readme](../llm_context/readme.md)、[xllm Rust SDK 参考](../llm_context/xllm_rust_sdk.md)、`doc/opendan/protocol/`
- 读者：决定是否按本文实施 xagent 的人；文中“现状”都对照 2026-10-02 的 `libopendan` / `agent_tool` 代码

---

## 0. 结论先行

1. **xagent 是 xllm 的上一层**：xllm 加载 `.llm_context`，把一个 LLMContext run 推进到一个 Outcome；xagent 加载（或创建）一个 Agent Session，把它推进到**一个 Turn 关闭**，或常驻地不断完成 Turn。两者的 run 目录相同（`runs/` 就是 xllm 的 run 目录），所以同一个 run 可以在两者之间交接，这是验证 L2 / L3 边界的主要手段。
2. **Agent Session 构造 llm_context 复用 xllm 的宿主装配 API**（`XllmTask::prepare_hosted` / `hosted_request` / `hosted_waist_deps` / `rebuild_toolset` / `create_run_llm` / `RunStore`），但 **system 段、历史段、输入批次、工具调度包装、checkpoint 钩子、run 生命周期都由 Session 决定**；差异清单见 §3。
3. **Input 统一走 Session 的 InputBus**（每 session 一条 kmsg 队列的抽象，开发期是文件队列）。Session 配置里只有传输方式，没有“Input 从哪里来的业务配置”；命令行给的单条 Input 也是先投递到 InputBus 再推进，所以三种使用形态只有一套提交、receipt 与 worklog 语义。
4. **Runtime 是“exec 在哪里跑”的执行环境**（native / tmux / 以后的容器），按 id 选择、首次推进时绑定；它不是工具调度层。Session 的工具分三层：xllm 内置工具、`.runtime/bin` 中的 CLI 形态 session 工具（xagent 自身充当，经 `AgentStateClient` 访问 Agent State）、进程内 session-aware 工具（必须声明为 `app_tools`，会使 xllm 无法接手）。默认优先 CLI 形态。
5. **behavior 配置来自 Agent State，在 Session 构造时冻结进 `session_config.prompt`**；之后 Agent 的 behavior 文件更新不影响已存在的 Session。普通 / fork / independent 切换模式由**目标 behavior** 的冻结配置决定，取代现在的 `extensions.opendan.process_modes`。
6. **Agent State 不配置**：给定 `agent_did` 与调用者身份 `who`，`AgentStateClient::connect` 按“进程内组件 → 本机 AgentRoot 文件 → kRPC”解析；Runner 只依赖 trait。xagent v1 实现前两种，第三种只定义接口并用桩验证 Runner 不依赖实现。
7. 核心是 §9 的 Rust 风格伪代码：`xagent::main` → `turn_loop::drive`（`AgentSession::load` → 恢复 → 主循环）→ `ContextFactory::new_run` / `open_live_run` → `commit_input_batch` → `handle_outcome` / `finish_run`。它直接对应 `runner/drive.rs` 的现状，新增的只有 `StopWhen::TurnClosed`、`BehaviorCatalog`、`BehaviorAssembler`、`SessionToolProvider`、`AgentStateClient::connect`、`RuntimeRegistry`（改动清单见 §11）。

---

## 1. 目标、范围与要验证的分层

### 1.1 目标

用一个独立进程（不依赖 OpenDAN 服务）把下面四层各自的边界跑通，并能用实验证明边界成立（§10）：

| 层 | 组件 | 回答的问题 | xagent 要证明的事 |
|---|---|---|---|
| LLM Context | `llm_context` + xllm（`agent_tool::xllm`） | 一次上下文推理循环：Round / Step、工具、快照、resume | Session 装配的 run 可以被 xllm 原样接手 / 跑完；Session 不重新实现 Loop |
| Agent Session | `libopendan`：目录协议 + Runner | 以某个 Agent 的身份，把一次逻辑 Input 推进到 result（Turn） | Turn 的开启 / 并入 / 关闭只由 Session 决定；三种输入形态语义一致；崩溃后恢复同一 Turn |
| Agent Runtime | `libopendan::runtime` | exec 在哪里、以什么 PATH / cwd / 环境运行；后台进程的识别与停止 | 同一 session_config 可绑定不同 runtime；绑定后不可换；工具集与 runtime 解耦 |
| Agent State | `libopendan::state` | 跨 Session 的 Agent 状态：登记表、活动视图、感知、认知、产物、**behavior 目录** | Runner 只依赖 `AgentStateClient`；文件实现 / 进程内实现 / kRPC 桩行为一致；behavior 冻结后不受更新影响 |

### 1.2 范围

- 做：work / self_improve / self_check session；native 与 tmux runtime；文件版与进程内 Agent State；behavior 冻结与 `BehaviorAssembler`；CLI 形态 session 工具；单 Turn / 积压消费 / 常驻三种运行形态。
- 不做（沿用计划的后移项）：UI session 与 msg-center 输入、kRPC 服务端、DID Object 宿主、opendan 改造、TS 版。
- 不改协议目录结构（目录项由用户定稿，§4.1 of 计划）；新增字段只落在 `session_config.prompt` 与 `state.json`，schema 版本号递增。

### 1.3 与 xllm 的对位

```text
xllm   : .llm_context ──prepare──► LLMContextRequest + Deps ──run──► Outcome      （一个 run，到停止点）
xagent : AgentSession ──InputBus──► 输入批次 ──commit──► run(s) ──handle_outcome──► Turn 关闭   （一个 Turn，可含多个 run）
                 ▲                                      │
                 └──── Agent State（behavior、身份、登记表、活动、认知）──┘
```

---

## 2. 分层与部署形态

```text
 xagent 进程（App 身份 who = app:<appid>@<owner>）
 ┌──────────────────────────────────────────────────────────────┐
 │ xagent CLI                                                   │
 │  ├─ AgentSession（SessionDir + state.json 提交点 + lease）     │
 │  ├─ TurnLoop（= runner::drive，加 StopWhen::TurnClosed）       │
 │  ├─ InputBus（KmsgInput | DirMsgQueue）  ◄─ post / --input    │
 │  ├─ ContextFactory（BehaviorAssembler + history + xllm hosted）│
 │  ├─ AgentRuntime（native | tmux | container*）                 │
 │  └─ AgentStateClient（InProcess | Fs | Krpc*）                 │
 └───────────┬──────────────┬────────────────────┬──────────────┘
             │ 文件 + flock  │ kmsg / kevent        │ 文件 / kRPC
             ▼              ▼                     ▼
   <sid>/.opendan_agent_session/   每 session 一条输入队列     AgentRoot：behaviors/ role.md self.md state/ memory/ …
   runs/ = xllm run 目录 ◄── xllm --resume 可接手
```

`*` 只定义接口与桩。xagent 自身也会被包装进 `<sid>/.runtime/bin`（名字 `agent-session`，兼容现有 exec 内用法），exec 里的子进程用它向本 session 投递 control / perception、读写认知与登记表。

---

## 3. Agent Session 如何构造 llm_context：与 xllm 的关系

**回答：是复用 xllm.rs 的方法，但只复用“宿主装配”那一半。** xllm 自己的 `XllmTask::prepare` 负责从 `.llm_context` 文件发现 / 合并配置、选组、渲染模板、拼 system 与 user、建 run。libopendan 走的是 2026-09-29 为 Agent Session 加的宿主装配路径：

| 环节 | xllm（`XllmTask::prepare` → `XllmRun`） | Agent Session（libopendan `runner/drive.rs::new_run_context_plain`） |
|---|---|---|
| 配置来源 | 从 workdir 向上发现 `.llm_context` 文件并合并 | `session_config.prompt.llm_context`（同 schema 的 JSON，严格键）。**xagent 新增**：冻结 behavior 的 `[model]/[capabilities]/[budget]/mode` 叠加成本 run 的有效 `.llm_context`（§6.4），结果写进 run.json `config`，xllm 接手时不再需要 behavior |
| 有效配置与工具展开 | `prepare` 内部 | `XllmTask::prepare_hosted(workdir, llm_context_json, origin, host_system, deps)`：xllm 算 EffectiveConfig、展开 tools / actions / MCP、在宿主 system 后追加 `capabilities` / `cmd_manual` / `runtime_protocol` 段 |
| system 段 | 按行号 section 拼装，含当前时间等新鲜量 | 宿主给出：身份（role.md / self.md）→ 不可覆盖约束（含避让规则）→ 应用 prompt → 初始 context → objective；**不含新鲜量**（S-20 稳定前缀）。xagent 由 `BehaviorAssembler` 用冻结的 behavior 模板渲染这一段 |
| 历史段 | 没有（一个 run 一次任务） | `history.rs::build_history`：先读 summary.json，再从 worklog 已提交末尾**反向**读到起点，预算不够先 compact；渲染成一条 `<session_history>` user 消息 |
| user 输入 | 任务要求 + 附件 + stdin，构造时就在 `request.input` | 不在构造时给。由 `commit_input_batch` 以 `LLMContext::inject` 注入 `<session_input hook=…>`，连同 `InputReceipt` 写进快照（HostMeta `libopendan`），并按 ①快照 ②run.json 门槛 ③state.json ④清门槛 ⑤确认输入源 的顺序提交 |
| LlmClient | `llm_factory.create(provider)` | `hosted.create_llm` / `create_run_llm`（同一 factory，以 `who` 身份），外面再包 `CountingLlm` 计 Round |
| ToolManager | `XllmToolManager` | 同一个 `XllmToolManager`（`rebuild_toolset` 重建），外包 `SessionToolManager`：lease 检查、宿主门槛、inflight 记录、touching 推断；`bash_runner` 由 Runtime 提供（执行跟踪） |
| waist deps | `LLMContextDeps` + xllm `SnapshotHook`（InferenceHook） | `hosted_waist_deps`（behavior 时装 `XllmActionParser` + 无时间戳 `XmlStepRenderer`）+ `SessionCheckpointHook`（异步 CheckpointHook：观察边界注入 changes、持久化工具结果、心跳、stop 中断）。**不用 InferenceHook** |
| run 目录 | `RunStore` 在 `.llm_context` 的 runs_dir | 同一 `RunStore`，目录是 `<sid>/.opendan_agent_session/runs/`；RunRecord 多了 `host{assembled_by, session_id, runtime_kind, env_check}`、`host_commit_pending`、`inflight`、`executions`；`host.extra.finish` 存结束决定 |
| 工作目录锁 | `<lock_dir>/<hash(workdir)>.lock` | `skip_workdir_lock = true`，多 session 共享 workspace 靠活动视图避让 |
| run 结束 | 终态写 run.json，结果导出 | `handle_context_outcome` → `finish_run`：flush 历史进 worklog、关闭 Turn、提交 state.json、登记表回报、感知 digest |
| 接手 | — | native runtime 且无 `host_commit_pending`、无进程内专有工具时，`xllm --resume --run <id> --runs-dir <sid>/.opendan_agent_session/runs --dir <workdir>` 可接手 |

xagent 不改变这张表的分工，只在“配置来源”和“system 段”两格引入 behavior 冻结（§6），在“ToolManager”一格引入 `SessionToolProvider`（§5）。

### 3.1 已知的边界缺口（xagent 要用实验暴露，实施时一并修）

| # | 现状 | 影响 | 处理 |
|---|---|---|---|
| G1 | `prepare_hosted` 只用 `llm_context` JSON 的 provider / model / limits / tools / `loop_model`，`prompt.sections`、`prompt.system`、groups 的 section、`runs_dir` 被解析但忽略 | behavior 叠加只能落在这几个键上，正好够用；但 `prompt.llm_context` 里写了 sections 会静默无效 | xagent 在冻结时校验并报错；§6.4 的 overlay 只产生这几个键 |
| G2 | `hosted_waist_deps` 用无时间戳 `XmlStepRenderer`，而 `xllm --resume` 接手后重建 deps 时用带时间戳的渲染器 | 接手后历史渲染字节变化，只影响前缀缓存，不影响正确性 | run.json `host` 里记 `renderer_opts`，xllm resume 时沿用（E1 实验会观察到） |
| G3 | `build_runtime_protocol` 的开场白是“你在 xllm 一次性任务里运行，没有后续对话，不要向用户提问”，hosted 也原样追加 | 与 Session 的 `WAIT_USER_MSG` 语义冲突 | `prepare_hosted` 增加 `HostProtocolFlavor::Session`，由宿主给出 runtime_protocol 的开场白 |
| G4 | xllm action 解析把“无动作 + `<report>`”映射为 `next_behavior = "done"`，不是 `END`；依赖 `forbid_next_behavior = false` | libopendan 已按“done 即交付”处理（`classify_done`），fork 子 run 不设 forbid | 保持；文档化到协议 Spec |
| G5 | `XllmToolManager` 的工具总 deadline 与 cancel watch 只在 `XllmRun::execute` 内接线，hosted 没有 | Session 推进时工具只受 exec 自身超时与 lease / stop 中断约束 | `SessionToolManager` 自带 deadline（取 budget.max_wallclock_ms）与 stop 取消 |
| G6 | `HostRunInfo.env_check` 被记录但 xllm resume 不校验 | xllm 可能在环境已变的 workdir 上接手 | xllm resume 校验 `env_check`（PATH 层、workdir、runtime_id）|

---

## 4. Agent Input 的构造

### 4.1 三种候选与结论

| 候选 | 含义 | 评价 |
|---|---|---|
| A. Session 配置里写 Input 来源，启动后轮询 | 像 opendan 的 `[[channel]]` / driver `pull_msg` | 把“业务路由”塞进 Session；Runner 要理解多种来源；与 xllm 接手、ts-runner 都不兼容 |
| B. 不配置，使用标准 AgentSession InputBus | 每 session 一条输入队列（kmsg；开发期 `DirMsgQueue`），任何人用 `post_input` 投递，驱动者消费 | **推荐**。现状已是如此：`create_session` 建队列并写入 `channels.inputs`，`drive` 只从这里取。配置里只剩传输层参数（队列名、订阅者、wake_event） |
| C. 从命令行取一个 Input，完成一个 Turn 后结束 | xllm 式用法 | 作为 B 的语法糖：**先 `post_input` 到 InputBus，再 drive 到 Turn 关闭**。这样 CLI 输入也有 key / receipt / `turn_started` 记录，崩溃后可恢复，与 serve 形态投递的输入 worklog 形状相同（§10 E9） |

结论：**只有 B 一套机制**。A 的“来源配置”降级为 InputBus 的传输配置（`channels.inputs`）；上游（msg-center、kevent 桥、定时器、父 session）各自作为 *producer* 向 InputBus 投递，不进 Session 协议。C 是 CLI 形态。

### 4.2 InputBus

```rust
/// 每个 session 一条逻辑输入队列。协议是 kmsg 语义（累积 ack、无去重、按 index 顺序）；
/// `Input` 的 kind / key / payload 见 protocol/input.rs。
#[async_trait]
pub trait InputBus: Send + Sync {
    /// 任何有写权限的人；返回队列 index。会发布 wake_event。
    async fn post(&self, input: &Input, who: &str) -> Result<u64>;
    /// 只有持 lease 的驱动者；从 state.inputs[src].consumed_above 之后取，最多 max 条。
    async fn fetch(&self, progress: &SourceProgress, max: usize) -> Result<Vec<FetchedInput>>;
    /// 累积确认到 state.json 已提交的位置（先提交 state，再 ack）。
    async fn confirm(&self, progress: &SourceProgress) -> Result<()>;
    /// 等待唤醒（kevent）或轮询间隔到。
    async fn wait(&self, timeout: Duration);
}
```

现有 `InputSource` / `KmsgInput` / `DirMsgQueue` / `Waker` 已经覆盖这四个操作，xagent 不新造实现，只把它们收拢为一个名字。输入种类保持 `msg | event | change | control | perception`，只有 msg / event 触发推理并开启或并入 Turn，change 在观察边界注入，control / perception 由 Runner 直接应用。

### 4.3 xagent 的三种运行形态

| 形态 | 命令 | 行为 | 返回时机 |
|---|---|---|---|
| 单 Turn | `xagent run <sid> --input "…"` | `post_input(msg)` → `drive(StopWhen::TurnClosed)` | 当前 Turn 关闭；或 Turn 保持打开但没有输入可推进（`WAIT_USER_MSG` 未交付回复）→ 退出码 3 |
| 消费积压 | `xagent run <sid>` | 不投递，只 `drive(StopWhen::TurnClosed)`：有积压输入就处理到 Turn 关闭；没有就报告等待 | 同上 |
| 常驻 | `xagent serve <sid>…` | 循环 `drive(StopWhen::Idle)`，Idle 后 `InputBus::wait`，被唤醒再 drive；可同时托管多个 session（每个一个任务） | SIGINT / `control(stop)` / session finished |

另有 `--until finished|idle|outcomes:<n>` 保留现有 `StopWhen`，用于调试。

### 4.4 Turn 关闭与 `StopWhen::TurnClosed`

现有 `StopWhen` 只有 `Idle | Finished | MaxOutcomes`，都不是“一个 Turn”。新增：

```rust
pub enum StopWhen {
    Idle,
    Finished,
    MaxOutcomes { n: u64 },
    /// 本次 drive 开启或接续的 Turn 被关闭（completed | failed | budget_exhausted | stopped）后返回；
    /// Turn 仍打开但无输入可推进时也返回（DriveResult::TurnOpen）。
    TurnClosed,
}
pub enum DriveResult {
    /* 现有变体 … */
    /// Turn 关闭；status 来自 worklog turn_ended。
    TurnClosed { rev: u64, turn: u64, status: TurnStatus, answer: Option<String> },
    /// Turn 保持打开（WAIT_USER_MSG 未交付回复 / 可恢复挂起 / 等待工具），且没有输入可继续。
    TurnOpen { rev: u64, turn: u64, waiting_for: Option<WaitingFor> },
}
```

Turn 规则沿用 readme：没有打开的 Turn 时提交的输入批次开启新 Turn；切换、fork、observation、挂起、重写、重启都延续；只有 `finish_run` / `stop_session` 关闭。xagent 不新增 Turn 语义，只新增一个退出条件。

---

## 5. Runtime 的构造

### 5.1 名字先说清：Runtime 不是工具调度层

readme 里对 `AgentRuntime` 命名的疑问（“是不是 llm_context 依赖的 trait，该叫 llm_context_runtime？”）这样回答：

- **Agent Runtime（`libopendan::runtime::AgentRuntime`）= exec 的执行环境**：主机、工作目录视图、PATH 层（`.runtime/bin` > Agent `tools/` > Runtime bin > System bin）、环境变量契约（`OPENDAN_*`）、后台进程的识别与停止（执行跟踪）。它回答“命令在哪里、以什么身份跑”，与 LLM 无关，首次推进时绑定（`binding.json`），之后不可更换。
- **工具调度层 = llm_context 的 `ToolManager`**，由 Session 实现（`SessionToolManager` 包 `XllmToolManager`）：lease 检查、宿主门槛、inflight 记录、touching 推断、deadline / cancel（G5）。所有工具调用先进这里再分发，其中 `exec` 再进 Runtime。
- 半订阅状态、模板变量等“每个 Round 有机会编入 context 的东西”属于 **Session 的 CheckpointHook 与 Assembler**，不属于 Runtime。

因此名字保持 `AgentRuntime`，职责收紧为执行环境；readme 那段将按本文更新。

### 5.2 Runtime 类型、选择与绑定

| 类型 | runtime_id 形态 | 现状 | xagent |
|---|---|---|---|
| native | `native:<host_id>` | `NativeRuntime`（`LocalProcessBashRunner` + `TrackedBashRunner`） | 默认 |
| tmux | `tmux:<host_id>[:<socket>]` | `TmuxRuntime`（移植自 opendan `agent_bash`） | `--runtime tmux` |
| container | `container:<name>` | 无 | 只定义 descriptor；实现随 paios 容器设计 |

选择与绑定：

```rust
/// 进程内可用的 runtime 集合；xagent 从 CLI / 环境 / xagent.toml 构造。
pub trait RuntimeRegistry: Send + Sync {
    fn resolve(&self, id: Option<&str>) -> Result<Arc<dyn AgentRuntime>>;   // None → 默认 native
    fn list(&self) -> Vec<RuntimeDescriptor>;
}
```

- `session_config.runtime.requirement{runtime_id?, tools[], app_tools[]}`、`tool_plan`、`env` 不变。创建时可不指定 `runtime_id`；首次 `drive` 用 registry 解析出的 runtime 绑定，之后每次 drive 核验（`bind_or_verify`），不匹配返回 `RuntimeMismatch`，绑定失败不产生推理成本（Q3）。
- `xagent run --runtime <id>` 只在首次绑定时有意义；已绑定的 session 再给不同 id 直接报错退出（退出码 6）。
- Runtime 的 `status()` 进入每个输入批次的 `<runtime>` 段（新鲜量，不进 system）。

### 5.3 工具的三层来源

| 层 | 例子 | 由谁构造 | 访问 Agent State 的方式 | xllm 能否接手 |
|---|---|---|---|---|
| ① xllm 内置 / MCP / `bash_tools` | `read_file` `write_file` `edit_file` `exec` `glob` `grep`、MCP 工具 | `prepare_hosted` 按有效 `.llm_context` 展开；`exec` 的 runner 来自 Runtime | 不访问 | 能（`rebuild_toolset`） |
| ② CLI 形态 session 工具 | `agent-session activity / perceive / post / decide / recall / note / sessions / read-session / create-worksession / artifact` | Runtime 的 `.runtime/bin` 把 xagent 自身包装进去（现有 `bin_overlay`） | 子进程里 `AgentStateClient::connect(agent_did, who)`（§7）；对**本 session** 状态的修改一律投递 control 到 InputBus 由驱动者应用，不直接写 | 能（它们只是 PATH 上的命令；xllm 的 exec 同样能调） |
| ③ 进程内 session-aware 工具 | fork / `try_create_worksession`（要动当前 run 的 process 栈）、以后 UI 的 `sendmsg` 投递 | `SessionToolProvider` 注入 `XllmDeps::with_host_tool`，并以 `tools: - name: X` 进入有效配置 | 直接拿 `Arc<dyn AgentStateClient>` + `SessionHandle` | **不能**：名字自动写入 `runtime.requirement.app_tools`，xllm resume 发现 app_tools 非空即拒绝 |

规则：**能用 ② 的不用 ③**。理由：② 不破坏 xllm 接手，天然跨语言（ts-runner 只要能起子进程），权限边界清楚（子进程用自己的 token）。③ 只保留必须触碰 run 内存状态的操作。

```rust
/// 进程内 session-aware 工具的提供者（层 ③）。
pub trait SessionToolProvider: Send + Sync {
    /// 为某个 run 生成工具；工具实现持有 session 句柄与 Agent State 客户端。
    fn tools(&self, sess: &SessionHandle, agent: Arc<dyn AgentStateClient>) -> Vec<Arc<dyn AgentTool>>;
    /// 这些名字会被写进 runtime.requirement.app_tools，并在每次 drive 核验。
    fn names(&self) -> Vec<String>;
}
```

`RunnerDeps` 增加 `tool_providers: Vec<Arc<dyn SessionToolProvider>>`；现有 `app_tools: Vec<String>` 由 providers 自动汇总。

### 5.4 哪些 session 工具会访问 Agent State

| 工具（层 ②，CLI 子命令） | Agent State 门面 | 读 / 写 | 备注 |
|---|---|---|---|
| `activity --summary/--touch`、`perceive` | — | 投递 control / perception 到本 session 队列 | 驱动者在 `apply_controls` 应用；现状已有 |
| `recall <query>` | `cognition().recall_hints` | 读 | 任何 session 都可 |
| `note <text>` | `cognition().notebook_append(who)` | 写 | 显式声明，不经 lease |
| `sessions [--active]`、`read-session <sid>` | `sessions().query/lookup` + 目录读 | 读 | 遵守 `acl.agent_access` |
| `create-worksession`、`post <sid>` | `sessions().register` / `post_input` | 写 | 子 session 的 driver 默认是调用者 who |
| `artifact register/head` | `artifacts()` | 写 / 读 | 版本登记在 `finish_run` 由 Runner 做，CLI 只给 Agent 中途登记用 |
| `decide` | `artifacts().decide` | 写 | 走 control(decide)，finished 后由驱动者执行 |

这些都已经由 `AgentStateClient` 的现有 trait 覆盖，xagent 只是把它们暴露成子命令。

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
}
```

`BehaviorConfig` 沿用 opendan 现有 toml 的结构（`behavior_cfg.rs`），只做两处收敛：

```rust
pub struct BehaviorConfig {
    pub meta: Meta { name, objective, next: Vec<String> /* 可选：声明可切换目标，用于冻结闭包 */ },
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

**范围**：入口 behavior + `meta.next` 声明的可达闭包 + identity。切换到一个未冻结的目标时（LLM 自由跳转），驱动者在该次切换前从目录**补冻结**（追加进 `frozen.behaviors`，`config_rev + 1`，worklog 记 `control_applied{behavior_frozen}`）。这保证“用过的 behavior 从首次使用起不再变”，同时不要求提前冻结整个目录。补冻结用的是目录的当前版本，所以 `catalog_rev` 只描述首次冻结；审计看 worklog。

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
| `identity.*`、`behavior.{name,objective,mode}`、`session.{id,kind,objective,driver,scope}`、`paths.{session_root,workspace_root}`（相对或 binding 提供）、`workspace.id`、`xml_behavior_result_protocol`（由 xllm `runtime_protocol` 段提供） | `runtime.{clock_text,status}`、`<active_sessions>`、`<hints>`、`<changes>`、`<perceptions>`、`<inputs>`、`behavior_switch / process_result`、`session.current_todo*`（读 `todos.json`）、`notebook.last_items`、`workspace_list` |

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
| `sessions()` | 登记门槛 `lookup`、每批 `me`、订阅的 session、`scope_touching` | 每次提交后 `report_state`；创建时 `register` |
| `activity()` | 每批 `<active_sessions>`、观察边界的活动集合变化 | （活动摘要经 `report_state` 回报） |
| `perception()` | self_improve bootstrap 窗口、`last_seq` 补发 | perception 输入、`finish_run` 的 run_digest / task_outcome |
| `cognition()` | bootstrap 或有新 msg/event 时 `recall_hints` | self_improve 成功时 `commit_consolidation` |
| `artifacts()` | `head` / `version` | `register_version`、`decide` |
| `locks()` | — | `self_improve`、`artifact:<aid>` |
| `behaviors()`（新） | 冻结时 `identity / get / revision`；补冻结时 `get` | — |

---

## 8. xagent 命令行

```text
xagent — drive an Agent Session for one Turn (or keep driving it)

  xagent new    --agent <did> --objective <text> [--parent <dir>] [--behavior <name>]
                [--llm-context <json|@file>] [--runtime <id>] [--workspace <path>] [--class work|self_improve]
                [--input <text>] [--until turn|finished|idle|outcomes:<n>]
                 创建 session（冻结 behavior），可直接投递首条输入并推进
  xagent run    <session_dir|sid> [--input <text> | --input-file <path> | --event <json>]
                [--until turn|finished|idle|outcomes:<n>] [--runtime <id>]
                 投递（可选）后推进到 Turn 关闭；默认 --until turn
  xagent serve  <session_dir|sid>... [--idle-unload <secs>]
                 常驻：drive(Idle) → 等唤醒 → drive(Idle)…；多 session 各一个任务
  xagent post   <sid> (--text <t> | --event <json> | --change <key> <json> | --perception <t> | --stop | --decide accept|discard)
  xagent status <sid> [--worklog <n>] [--report] [--run]      session / 最近 run 的状态与统计
  xagent list   --agent <did> [--active]                      登记表
  xagent behaviors --agent <did> [--frozen <sid>]             目录 vs 某 session 冻结的 behavior（E2 用）
  xagent xllm   <sid> [--run <id>]                            打印让 xllm 接手当前 live run 的命令行（E1 用）

  在 exec 子进程内（PATH 上的 agent-session 即 xagent）：
  xagent activity [--summary <t>] [--touch <ref>]... | perceive <text> | recall <query> | note <text>
  xagent sessions [--active] | read-session <sid> | create-worksession --objective <t> [...] | artifact ...

身份与定位（都可用环境变量）：
  --agent <did>          [$OPENDAN_AGENT_DID]      --who <principal>       [$LIBOPENDAN_WHO，默认取 BuckyOS 运行时 app principal]
  --agent-root <dir>     [$OPENDAN_AGENT_ROOT]     --state-url <url>       [$OPENDAN_AGENT_STATE_URL]   （都是 connect 的 hint）
  --queue-dir <dir>      [$LIBOPENDAN_QUEUE_DIR]   开发用文件队列；不给则用 kmsg
```

退出码（对齐 xllm 的分类，便于脚本串联）：

| 码 | 含义 |
|---|---|
| 0 | Turn 以 `completed` 关闭（或 `--until finished` 的 session 结束） |
| 1 | Turn 以 `failed` / `budget_exhausted` 关闭，或 session 结束为 Failed |
| 2 | 参数 / 配置 / 冻结校验错误，未推进 |
| 3 | 没有关闭 Turn：Turn 保持打开等待输入（`WAIT_USER_MSG` 未回复）、等待工具、可恢复挂起；再 `run` 即续 |
| 4 | 被 stop / 中断（Turn 记 `stopped` 或 run 保留） |
| 5 | Busy：session lease 或 run 锁被他人持有（另一个 xagent / xllm） |
| 6 | 阻塞：NotDriver / Unregistered / BindFailed / RuntimeMismatch / RecoveryBlocked |

`--format json` 时 stdout 是 `DriveResult` 的 JSON（含 `rev`、`turn`、`status`、`answer`），文本模式打印 answer / report。

---

## 9. 核心伪代码（Rust 风格）

约定：省略错误类型细节、日志与统计；`?` 表示失败即按 §8 退出码返回；`commit!` 表示 state.json 原子替换（rev+1）并随后回报登记表。对应实现：`runner/drive.rs`（drive / commit_input_batch / handle_context_outcome / finish_run / suspend_run）、`runner/hook.rs`（SessionCheckpointHook）、`runner/history.rs`（build_history）、`runner/assembler.rs`。与现状不同的地方用 `// NEW` 标出。

### 9.1 组件与数据

```rust
/// 一个被打开的 session：目录 + 配置 + 提交点 + 推进权 + 输入总线。
pub struct AgentSession {
    dir: SessionDir,                      // <sid>/.opendan_agent_session/
    cfg: SessionConfig,                   // session_config.json（含 prompt.frozen）
    state: SessionState,                  // state.json（提交点）
    lease: Arc<Lease>,                    // 长期持有的 flock；驱动者身份已校验
    bus: Arc<dyn InputBus>,               // 本 session 的输入队列
    runs: RunStore,                       // runs/ = xllm run 目录
}

pub struct Deps {
    who: Principal,                                   // app:<appid>@<owner>
    agent: Arc<dyn AgentStateClient>,                 // connect() 得到，不配置
    runtime: Arc<dyn AgentRuntime>,                   // RuntimeRegistry 解析
    xllm: XllmDeps,                                   // llm factory、observer；bash_runner 每 run 由 runtime 给
    assembler: Arc<dyn SessionAssembler>,             // BehaviorAssembler
    tool_providers: Vec<Arc<dyn SessionToolProvider>>,// NEW 层 ③ 工具
    summarizer: Option<Arc<dyn Summarizer>>,
    options: RunnerOptions,
}

/// 本次 drive 正在推进的 LLMContext。
pub struct LiveCtx {
    ctx: LLMContext,
    run: RunHandle,            // 持有 runs/<run_id>/.lock
    behavior_mode: bool,
    ready: bool,               // 从中途恢复：没有新输入也可以继续 run()
    rounds: Arc<RoundCounter>, // CountingLlm 计 Round
}

/// handle_outcome 的决定；同时写进 run.json host.extra.finish，崩溃后重做得到同一结果。
pub struct Next {
    kind: NextKind,            // Done | Wait | ProcessDone | Switch(b) | Budget | Error | Stopped | Interrupted | PendingTool | ContextLimit
    run_ended: bool,
    suspended: bool,           // fork / independent：run 入栈
    finished: bool,            // session 结束
    waiting: bool,             // 等输入
    turn_end: Option<TurnStatus>, // Some → 本次提交关闭 Turn
    answer: Option<String>,
    error: Option<Value>,
}
```

### 9.2 xagent 入口

```rust
#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();                                             // §8
    let who = cli.who.or_else(buckyos_app_principal)?;                  // 身份不来自提示词
    let agent = AgentStateClient::connect(&cli.agent_did, &who, cli.state_hint()).await?;   // NEW §7.1
    let buses = InputBusFactory::new(cli.queue_dir, buckyos_kmsg_client());                  // 文件队列 | kmsg
    let runtime = RuntimeRegistry::for_host().resolve(cli.runtime.as_deref())?;             // NEW §5.2
    let deps = Deps {
        who: who.clone(), agent: agent.clone(), runtime,
        xllm: XllmDeps::default().with_observer(cli_observer()),
        assembler: Arc::new(BehaviorAssembler::default()),                                   // NEW §6.4
        tool_providers: vec![Arc::new(ForkToolProvider)],                                    // 只放必须进程内的
        summarizer: None, options: RunnerOptions::default(),
    };
    install_self_as_session_cli(&deps);                                 // 让 .runtime/bin/agent-session 指向本可执行文件

    match cli.cmd {
        Cmd::New { parent, spec, input, until } => {
            let spec = freeze_behaviors(spec, agent.behaviors()).await?;        // NEW §6.3：能读目录就现在冻结
            let sd = create_session(&parent, spec, agent.as_ref(), &who, &buses).await?;
            if let Some(text) = input { post_input(agent.as_ref(), sd.sid(), &Input::msg(new_key("cli"), text), &who).await?; }
            drive_once(&sd, &deps, until.unwrap_or(StopWhen::TurnClosed)).await
        }
        Cmd::Run { target, input, until } => {
            let sd = locate_session(agent.as_ref(), &target).await?;           // 目录路径或登记表 sid
            if let Some(i) = input { post_input(agent.as_ref(), sd.sid(), &i.into_input(), &who).await?; }   // C 形态 = 先投递
            drive_once(&sd, &deps, until.unwrap_or(StopWhen::TurnClosed)).await
        }
        Cmd::Serve { targets, idle_unload } => serve(targets, &deps, idle_unload).await,     // §9.6
        Cmd::Post { .. } | Cmd::Status { .. } | Cmd::List { .. } | Cmd::Behaviors { .. } | Cmd::Xllm { .. } => read_only_or_post(cli, agent).await,
        Cmd::Activity { .. } | Cmd::Perceive { .. } | Cmd::Recall { .. } | Cmd::Note { .. } | .. => session_tool_subcommand(cli, agent, &who).await, // 层 ② 工具
    }
}

async fn drive_once(sd: &SessionDir, deps: &Deps, until: StopWhen) -> ExitCode {
    let r = turn_loop::drive(sd, deps, until).await;
    print_result(&r);
    exit_code_of(&r)        // §8 表：TurnClosed(completed)→0, TurnClosed(failed|budget)→1, TurnOpen|Idle→3, Stopped→4, Busy|RunBusy→5, NotDriver|Unregistered|BindFailed|RecoveryBlocked→6
}
```

### 9.3 Agent Turn Loop（核心）

```rust
pub async fn drive(sd: &SessionDir, deps: &Deps, until: StopWhen) -> DriveResult {
    // ── 0. 推进权与登记 ─────────────────────────────────────────────────────────
    let lease = match sd.acquire(deps.holder()) {               // driver.principal == who，再 flock lease.json
        Ok(l) => Arc::new(l), Err(NotDriver{driver}) => return NotDriver{driver}, Err(Busy{holder}) => return Busy{holder},
    };
    let mut s = AgentSession::load(sd, lease.clone(), deps)?;    // schema 版本不符 → RecoveryBlocked
    if deps.agent.sessions().lookup(s.sid()).await?.map(|e| e.location) != Some(s.dir.canonical()) { return Unregistered; }
    let _kind_lease = s.acquire_kind_lease(deps).await?;          // self_improve：agent.locks()["self_improve"]，拿不到 → Busy

    // ── 1. 恢复：先把磁盘上的现场对齐，再碰任何新输入 ─────────────────────────
    let reconciled = s.reconcile_runs(deps).await?;              // 截 worklog 未提交尾 → 回收 state 不引用的 run → live_run：拿 run 锁、
                                                                 // 校验快照版本、停旧执行、按 receipt 补齐 state、清 host_commit_pending；
                                                                 // 终态 run（xllm 跑完 / 结束流程没走完）→ finish_run(recorded Next)
    s.bus.confirm(&s.state.inputs).await?;                        // 累积 ack 到已提交位置
    s.catch_up_reports(deps).await;                               // 登记表 rev、感知 seq 补发
    let mut inputs = s.bus.fetch(&s.state.inputs, FETCH_MAX).await?;
    s.apply_controls(&mut inputs, InRun::No).await?;              // stop / subscribe / activity / decide / perception / 重复 key；每次都 commit! 后 confirm
    if s.state.run_state == Finished { s.reject_leftovers(inputs).await?; return Finished{..}; }

    // ── 2. 执行环境：首次推理前绑定，之后每次核验（无推理成本） ─────────────
    s.ensure_frozen(deps).await?;                                 // NEW：prompt.frozen 为空且本进程能读目录 → 冻结并替换 session_config
    let plan = bin_plan_for(&s.cfg, deps.agent.agent_root(), deps.session_cli(), deps.runtime.path_layers());
    let binding = match bind_or_verify(sd, &lease, deps.runtime.as_ref(), &s.cfg, &plan, deps.app_tools()).await {
        Ok(b) => b, Err(e) => { s.state.last_error = Some(e.to_json()); commit!(s); return BindFailed{error: e.to_json()}; }
    };
    let env = deps.runtime.open_session_env(&binding, &s.env_ctx()).await?;
    let factory = ContextFactory::new(&s, deps, &env, lease.clone());
    let mut live: Option<LiveCtx> = match reconciled { Reconciled::Resume(run, snap) => Some(factory.resume(run, snap, ready = true).await?), _ => None };
    let (mut outcomes, mut turn_closed) = (0u64, None::<(u64, TurnStatus, Option<String>)>);

    // ── 3. 主循环：每圈 = 一个输入批次（可选）+ 一个 run 段 + 一个 Outcome ──────
    loop {
        lease.check()?;                                                                  // 丢锁即停
        if s.state.stop_requested { return s.stop_session(live, deps).await; }           // Turn 记 stopped，session Finished
        if let StopWhen::MaxOutcomes{n} = until { if outcomes >= n { return OutcomesHandled{..}; } }

        // 3a. 组装本批输入材料（新鲜量只在这里）
        let picked = inputs.take(Msg) ++ inputs.take(Event);                             // input_policy == None → 全部拒绝
        let changes = check_changes(&s, inputs.take(Change), deps.agent.as_ref(), include_active = false).await?;
        let hook = if !s.state.bootstrap_done { "on_init" } else if s.state.internal_continuation.is_some() { "on_behavior_switch" } else { "on_wakeup" };
        let hints  = if deps.options.load_hints && (hook == "on_init" || !picked.is_empty()) { deps.agent.cognition().recall_hints(&s.topic_query()).await? } else { vec![] };
        let active = deps.agent.activity().active(s.registry_entry().as_ref(), deps.options.active_sessions_limit).await?;
        let material = InputMaterial { hook, inputs: picked.clone(), changes: changes.items.clone(), hints, active, runtime_status: deps.runtime.status().await, now_ms: now_ms(), perceptions: s.self_improve_window(deps).await? };
        let triggered = !picked.is_empty() || hook != "on_wakeup";                       // bootstrap / 切换交接也触发
        let msg = if triggered { deps.assembler.render_input(&s.cfg, &s.state, &material).await? } else { None };

        // 3b. 没有可推理的输入，也没有可继续的 run → 等待或返回
        let resumable = live.as_ref().map_or(false, |l| l.ready);
        if msg.is_none() && !resumable {
            s.drop_consumed_only_changes(&changes).await?;                               // change_dropped + commit!
            s.mark_waiting_for_input(); commit!(s);
            match until {
                StopWhen::Idle                         => return Idle{..},
                StopWhen::MaxOutcomes{..}              => return OutcomesHandled{..},
                StopWhen::TurnClosed                   => return match turn_closed {           // NEW
                    Some((turn, status, answer)) => TurnClosed{turn, status, answer, ..},    // 本次 drive 已关过一个 Turn
                    None => TurnOpen{turn: s.state.current_turn(), waiting_for: s.state.waiting_for.clone(), ..},
                },
                StopWhen::Finished => {
                    if s.state.last_error.is_some() { return Error{..}; }
                    if started.elapsed() >= deps.options.max_wait { return Idle{..}; }
                    s.bus.wait(deps.options.poll_interval).await;                        // kevent 唤醒或轮询
                    inputs = s.bus.fetch(&s.state.inputs, FETCH_MAX).await?; s.apply_controls(&mut inputs, InRun::No).await?;
                    if s.state.run_state == Finished { return Finished{..}; }
                    continue;
                }
            }
        }

        // 3c. 取得要推进的 LLMContext：内存里的 live → state.live_run（重入 / fork 返回）→ 新建
        let mut lc = match live.take() {
            Some(l) => l,
            None if s.state.live_run.is_some() => factory.open_live_run(&mut s).await?,  // 拿 run 锁、停旧执行、resume；ready=false
            None => factory.new_run(&mut s).await?,                                      // §9.4：system(冻结 behavior) + <session_history>
        };

        // 3d. 提交输入批次：开启或并入 Turn（从不关闭）
        if let Some(text) = msg { s.commit_input_batch(&mut lc, &picked, &changes, text, hook).await?; }   // §9.5

        // 3e. 推进一个 run 段，直到 Outcome（内部 ≤3 次 context-limit 重写）
        lc.ready = false;
        let outcome = factory.run_compacting(&mut s, &mut lc).await;
        let next = s.handle_outcome(&mut lc, outcome, deps).await?;                      // §9.5
        outcomes += 1;
        if let Some(status) = next.turn_end { turn_closed = Some((s.state.last_closed_turn(), status, next.answer.clone())); }
        if !next.run_ended && !next.suspended { live = Some(lc); }                      // 普通切换 / 可恢复挂起：run 留在内存

        // 3f. 退出判定
        if next.finished { return Finished{..}; }
        if next.error.is_some() { return Error{..}; }                                   // budget / 不可重试 / 中断 / context_limit 停住
        if until == StopWhen::TurnClosed && next.turn_end.is_some() {                    // NEW：一个 Turn 关闭即返回
            let (turn, status, answer) = turn_closed.take().unwrap();
            return TurnClosed{turn, status, answer, ..};
        }
        if next.waiting && until == StopWhen::Idle { return Idle{..}; }

        inputs = s.bus.fetch(&s.state.inputs, FETCH_MAX).await?;
        s.apply_controls(&mut inputs, InRun::No).await?;                                 // 下一圈：WAIT 未回复时新输入并入同一 Turn
    }
}
```

要点：

- **Turn 的边界只在两处改变**：`commit_input_batch`（开启或并入）与 `finish_run` / `stop_session`（关闭）。普通切换、fork 调用与返回、independent 重入、观察注入、挂起、重写、崩溃恢复都不碰 `open_turn`。
- **`StopWhen::TurnClosed` 是 drive 的返回条件，不是 Turn 语义**：它在 3f 看 `next.turn_end`；若 Turn 没关而又没输入（3b），返回 `TurnOpen`，退出码 3，下次 `run` 从同一 run 续。
- **每个 run 段一个 Outcome**，`outcomes` 只给 `MaxOutcomes` 用，不是 Round 数、Turn 数。

### 9.4 ContextFactory：恢复或新建一个 run 的 LLMContext

```rust
impl ContextFactory<'_> {
    /// 新 run：system（冻结 behavior）+ <session_history>；fork 子 run 再继承父 steps。
    async fn new_run(&self, s: &mut AgentSession) -> Result<LiveCtx> {
        let entry = s.state.current_behavior.clone().or(s.cfg.prompt.behavior.clone()).unwrap_or("main".into());
        let behavior = s.frozen_behavior(&entry, self.deps).await?;                      // NEW：prompt.frozen.behaviors[entry]；没有 → 补冻结 → 还没有 → RecoveryBlocked
        let system = self.deps.assembler.system_text(&s.cfg, &entry).await?;             // 身份 → 约束 → on_init 渲染 → objective；无新鲜量

        // xllm 宿主装配：有效 .llm_context = 基线 + behavior 叠加（model / loop_model / tools / limits）
        let llm_context = behavior.overlay_llm_context(&s.cfg.prompt.llm_context);       // NEW §6.4
        let xdeps = self.deps.xllm.clone()
            .with_bash_runner(self.runtime.bash_runner(&self.env, Arc::new(LateRegistrar::new())))   // 执行跟踪
            .with_host_tools(self.provider_tools(s))                                     // NEW 层 ③；名字进 app_tools
            .with_skip_workdir_lock(true);
        let hosted = XllmTask::prepare_hosted(&self.env.workdir, &llm_context, "session_config.prompt", &system, &xdeps).await?;
        let llm = counted(hosted.create_llm(&xdeps).await?, rounds.clone());

        // 历史：先 summary.json，再从 worklog 已提交末尾反向读到起点；预算不够先 compact
        let budget = s.cfg.prompt.history_budget_tokens.unwrap_or(self.deps.options.history_budget_tokens);
        let history = build_history(s, &self.lease, self.summarizer(&llm), budget).await?;   // Option<AiMessage::User("<session_history>…")>

        // run 目录（xllm 布局）；在 state 引用它之前带 host_commit_pending，任何执行者不得推理
        let run_id = s.runs.create_locked()?;
        let record = hosted.new_record(&run_id, Some(s.runs.dir()), &s.cfg.session.objective,
            HostRunInfo { assembled_by: "libopendan", session_id: s.sid(), runtime_kind: self.binding.kind, runtime_id: self.binding.runtime_id, env_check: self.env.check_digest(), extra: json!({ "behavior": entry, "renderer_opts": {"timestamps": false} }) });   // G2
        let run = RunHandle::new(s.runs.clone(), record)?; run.write()?;

        let tools = Arc::new(SessionToolManager::new(Arc::new(hosted.manager), run.clone(), self.lease.clone(), self.env.workdir.clone(), self.touched.clone())
            .with_deadline(behavior.budget.max_wallclock_ms));                            // G5
        let mut input = vec![AiMessage::system(system)]; if let Some(h) = history { input.push(h); }
        let request = hosted_request(&hosted.config, ContextOwnerRef::Agent{session_id: s.sid()}, &run_id, &s.cfg.session.objective, &entry, input)
            .with_budget(behavior.budget.to_budget_spec());                              // NEW：wallclock / total tokens 来自 behavior
        let waist = hosted_waist_deps(&hosted.config, llm, tools)
            .with_checkpoint_hook(Arc::new(SessionCheckpointHook::new(self.shared.clone(), run.clone(), hosted.config.loop_model == Behavior)));

        let mut ctx = match s.fork_parent() {                                            // process_stack 顶是 Fork 且 live_run 为空
            Some(parent) => {                                                            // fork 子 run：继承父 steps，编号接续
                let mut snap = LLMContextSnapshot::fresh(request, host_meta(s, &entry));
                snap.inherit_steps_from(&parent.latest_snapshot()?);                     // steps / last_step / history_summaries / next_*_index；inherited_below 标记不重复 flush
                LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, waist)?
            }
            None => LLMContext::new(request, waist).with_host_meta(host_meta(s, &entry)),
        };
        self.shared.set_interrupt(ctx.interrupt_handle());
        Ok(LiveCtx { ctx, run, behavior_mode: hosted.config.loop_model == Behavior, ready: false, rounds })
    }

    /// state.live_run 指向的未结束 run：拿锁、校验、停旧执行、物化在途、resume。
    async fn open_live_run(&self, s: &mut AgentSession) -> Result<LiveCtx> {
        let lr = s.state.live_run.clone().unwrap();
        let run = s.runs.lock(&lr.run_id).ok_or(RunBusy{run_id: lr.run_id.clone()})?;    // xllm 正在接手 → Busy，不另起 run
        let (record, mut snap) = run.load_checked()?;                                    // 版本 / 引用完整性；失败 → RecoveryBlocked，保留现场
        self.runtime.stop_executions(&record).await?;                                    // kill runner 不代表旧工具退出
        if let Some(seq) = record.host_commit_pending { ensure!(lr.applied_input_seq >= seq); run.complete_host_commit()?; }
        let tools = rebuild_toolset(&record, &self.xdeps_for(s)).await?;                 // 同一有效配置重建（含层 ③ 具名工具）
        let llm = counted(create_run_llm(&record, &self.xdeps).await?, rounds.clone());
        if !record.inflight.is_empty() { snap = materialize_unresolved(&snap, &record.inflight, record.behavior_mode())?; run.checkpoint_with_results(&snap, None)?; }   // 结果未知，不重放工具
        if snap.request.behavior_name != s.state.current_behavior { snap.request.behavior_name = s.state.current_behavior.clone(); }       // 普通切换后的恢复
        if let Some(pr) = &s.state.process_result { snap.bump_ids_after(pr); }           // fork 返回：action / step 编号接续
        let waist = hosted_waist_deps(&record.config, llm, Arc::new(SessionToolManager::new(..))).with_checkpoint_hook(..);
        let ctx = match snap.suspended {
            Some(Suspension::PendingTool) => return Err(RecoveryBlocked("deferred tool results are not supplied by this runner")),
            Some(Suspension::ContextLimit) => self.rewrite_for_limit(s, snap, waist, attempt = 1).await?,   // flush → compact → 重建 system+history → resume(Rewritten*)
            None => LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, waist)?,
        };
        run.set_status(RunStatus::Running)?;
        Ok(LiveCtx { ctx, run, behavior_mode: record.behavior_mode(), ready: true, rounds })
    }

    /// 一个 run 段：ctx.run()；ContextLimitReached 时最多重写 3 次后再给出 Outcome。
    async fn run_compacting(&self, s: &mut AgentSession, lc: &mut LiveCtx) -> LLMContextOutcome {
        let mut attempts = 0;
        loop {
            match lc.ctx.run().await {
                LLMContextOutcome::ContextLimitReached{snapshot, ..} if attempts < MAX_LIMIT_COMPACTIONS => {
                    attempts += 1;
                    lc.ctx = self.rewrite_for_limit(s, snapshot, lc.deps(), attempts).await?;   // worklog flush(outcome=context_rewritten) → summary.json → 新 history_epoch
                }
                o => return o,
            }
        }
    }
}
```

### 9.5 输入批次提交与 Outcome 处理

```rust
impl AgentSession {
    /// 开启或并入 Turn。提交顺序固定：①快照 ②run.json 门槛 ③state.json ④清门槛 ⑤确认输入。
    async fn commit_input_batch(&mut self, lc: &mut LiveCtx, picked: &[FetchedInput], changes: &Changes, text: String, hook: &str) -> Result<()> {
        let opens = self.state.open_turn.is_none();
        let turn = if opens { self.state.turn_seq + 1 } else { self.state.open_turn.as_ref().unwrap().index };
        let mut receipt = InputReceipt { run_id: lc.run.id(), input_seq: self.state.live_run_applied_seq() + 1, turn, opens_turn: opens, hook: hook.into(),
            inputs: picked.ids() ++ changes.injected_inputs.ids(), changes: changes.receipts.clone(), consumed_only: changes.consumed_only.clone(),
            bootstrap: !self.state.bootstrap_done, after_step: lc.ctx.snapshot().next_step_index, continuation: self.state.internal_continuation.is_some(), .. };
        receipt.message_pos = lc.ctx.inject(Injection::user(text));                       // 消息与 receipt 必须在同一份快照里
        lc.ctx.host_meta_mut().input_receipts.push(receipt.clone());
        lc.run.publish_input_checkpoint(&lc.ctx.snapshot(), receipt.input_seq)?;         // ①②：fsync 快照；run.json latest_snapshot_idx + host_commit_pending
        apply_receipt(&mut self.state, &receipt)?;                                       // live_run.turns / open_turn|turn_seq / 消费位置 / 订阅游标 / bootstrap_done / 清 continuation
        self.state.run_state = Running; self.state.waiting_for = None; self.state.last_error = None;
        self.state.refresh_activity(picked, &self.cfg);                                  // 活动摘要、scope → touching
        commit!(self);                                                                   // ③
        lc.run.complete_host_commit()?;                                                  // ④：之后工具调用才被 SessionToolManager 放行
        self.bus.confirm(&self.state.inputs).await                                       // ⑤
    }

    /// 一个 Outcome 结束一个 run 段；Turn 是否结束由 next.turn_end 决定。
    async fn handle_outcome(&mut self, lc: &mut LiveCtx, outcome: LLMContextOutcome, deps: &Deps) -> Result<Next> {
        let snap = outcome.snapshot().cloned().unwrap_or_else(|| lc.ctx.snapshot());
        let mut next = match outcome {
            Done{behavior_result, response, ..} => {
                let nb = behavior_result.as_ref().and_then(|b| b.next_behavior.clone());
                let answer = behavior_result.self_report().or(response.text());
                let replied = snap.has_report() || !behavior_result.messages_to_send().is_empty();
                match (self.is_fork_child(&lc.run), nb.as_deref()) {
                    (true, nb) if nb != Some(WAIT_USER_MSG)          => Next::process_done(answer),                        // 子 process 返回父；Turn 不关
                    (_, Some(WAIT_USER_MSG))                          => Next::wait(answer, turn_end = replied.then_some(Completed)),   // 没交付回复 → Turn 保持打开
                    (_, Some(b)) if b != END && b != "done"           => Next::switch(b),                                  // run 不结束
                    _ => self.decide_end(answer)                                                                           // Completed；按 end_condition 判 finished / waiting
                }
            }
            BudgetExhausted{which, ..}  => Next::budget(which),                                                            // run_ended, turn_end = BudgetExhausted
            Error{error, ..} if is_retryable(&error) => Next::retryable(error),                                            // run 保留，Turn 打开
            Error{error, ..}            => Next::failed(error),                                                            // turn_end = Failed
            Interrupted{..} if self.state.stop_requested => Next::stopped(),                                               // finished, turn_end = Stopped
            Interrupted{..}             => Next::interrupted(),                                                            // run 保留
            PendingTool{..}             => Next::pending_tool(),                                                           // run 保留，waiting_for = tool
            ContextLimitReached{..}     => Next::context_limit(),                                                          // 3 次重写仍装不下：run 暂停
        };

        // 1. 先持久化结果与快照（覆盖的 inflight 由此清除），再累加 Round
        if next.run_ended { lc.run.checkpoint_finish(&snap, next.run_status(), &next)?; } else { lc.run.checkpoint_with_results(&snap, Some(next.run_status()))?; }
        lc.run.record_usage(outcome.usage(), lc.rounds.take())?; self.static_add_rounds(..);

        // 2. 分流
        match &next.kind {
            NextKind::Switch(b) => match deps.assembler.process_mode(&self.cfg, b) {                                     // NEW：读目标 behavior 的冻结 switch_mode
                None => {                                                                                                 // 普通切换：同一 run、同一 context、同一 Turn
                    self.ensure_frozen_behavior(b, deps).await?;                                                          // NEW：目标未冻结 → 补冻结
                    lc.ctx = LLMContext::resume(snap.with_behavior(b), ResumeFill::ResumeFromMidRun, lc.ctx.take_deps())?;
                    self.state.current_behavior = Some(b.clone()); self.state.internal_continuation = Some(b.clone()); self.state.run_state = Ready;
                    commit!(self);                                                                                        // 下一圈提交 on_behavior_switch 批次
                }
                Some(mode) => { self.suspend_run(lc, b, mode, &snap).await?; next.suspended = true; }                       // fork / independent：run 入栈
            },
            _ if next.run_ended => self.finish_run(lc, &snap, &mut next, deps).await?,
            _ => { self.state.run_state = if next.kind == PendingTool { Waiting(tool) } else { Ready }; self.state.last_error = next.error.clone(); commit!(self); }   // 可恢复挂起：worklog 不写
        }
        Ok(next)
    }

    /// 唯一关闭 Turn 的地方（stop_session 除外）。重做时幂等。
    async fn finish_run(&mut self, lc: &mut LiveCtx, snap: &LLMContextSnapshot, next: &mut Next, deps: &Deps) -> Result<()> {
        self.runtime_stop_executions(&lc.run).await?;
        let turn = self.state.current_turn();
        let mut entries = run_history_entries(&lc.run.id(), snap, FlushMarks::of(&self.state.live_run), turn);   // turn_started/input_batch/user_message/assistant_message|step/action_result，只写 flush 游标之后
        if let Some(status) = next.turn_end { if self.state.open_turn.is_some() { self.state.open_turn = None; if status == Completed { self.state.turns_completed += 1; } } }
        if next.finished { self.register_outputs(deps).await?; self.write_report(next.answer.as_deref())?; }
        entries.push(Outcome{run_id, turn, kind: next.kind.as_str(), next_behavior, report: next.answer.truncated(2000)});
        if let Some(status) = next.turn_end { entries.push(TurnEnded{run_id, turn, status, at_ms}); }
        self.worklog_append(entries)?;                                                                   // 一次写入 + fsync
        self.state.live_run = None; self.state.last_run = Some(lc.run.id());                             // 保留最后一次 run 的现场
        match next.kind {
            ProcessDone => { let f = self.state.process_stack.pop().unwrap(); self.state.live_run = Some(f.into_live()); self.state.current_behavior = Some(f.entry.clone()); self.state.internal_continuation = Some(f.entry); self.state.process_result = Some(json!({"behavior": .., "result": next.answer, ..})); }
            _ if next.finished => { self.state.run_state = Finished; self.state.process_stack.clear(); self.state.outcome = next.outcome(); self.state.acceptance = Pending; self.state.result = next.result(); }
            _ if next.waiting  => self.state.run_state = Waiting(input),
            _                  => self.state.run_state = Ready,
        }
        self.state.perception_seq += 1 + next.finished as u64;
        commit!(self);                                                                                   // ← 提交点
        self.gc_previous_last_run(); lc.run.prune(deps.options.keep_snapshots);
        maybe_compact(self, deps).await;                                                                 // 按占用比例触发 summary.json
        self.update_static(..);
        deps.agent.perception().append(&self.lease, self.sid(), digests(self, lc, turn, next)).await?;   // 提交之后；失败下次 catch_up 补
        if self.is_self_improve() && next.finished { deps.agent.cognition().commit_consolidation(..).await?; }
        Ok(())
    }

    /// fork / independent：当前 run 挂起入栈，Turn 继续。
    async fn suspend_run(&mut self, lc: &mut LiveCtx, target: &str, mode: ProcessMode, snap: &LLMContextSnapshot) -> Result<()> {
        self.runtime_stop_executions(&lc.run).await?;
        self.worklog_append(run_history_entries(..) ++ [Outcome{kind: "suspended", next_behavior: target}])?;   // 先 flush 已有历史，worklog 保持时序
        self.state.process_stack.push(ProcessFrame::from_live(&self.state.live_run, mode, snap));
        self.state.live_run = match (mode, self.state.process_stack.find_independent(target)) {                 // independent 重入：恢复它自己的 run
            (Independent, Some(frame)) => Some(frame.into_live()), _ => None };                                    // 否则下一圈 new_run()（fork 子继承父 steps）
        self.state.current_behavior = Some(target.into()); self.state.internal_continuation = Some(target.into()); self.state.run_state = Ready;
        lc.run.set_status(RunStatus::Paused)?;
        commit!(self)
    }
}
```

`SessionCheckpointHook`（观察边界，现状不变）：每次推理前 / Step 边界被 waist 调用 → `lease.check` → 持久化工具结果快照 → `fetch` 并 `apply_controls(in_run)`（stop 则 interrupt）→ 心跳合并 touching → `check_changes(include_active)` → 有变化则渲染 `<changes>` 并以 `hook = "observation"`、`opens_turn = false` 的 receipt 注入，第二次回调时按同一 ①–⑤ 顺序提交。msg / event 永远不在观察边界注入。

### 9.6 常驻形态

```rust
async fn serve(targets: Vec<Target>, deps: &Deps, idle_unload: Duration) {
    join_all(targets.into_iter().map(|t| async move {
        let sd = locate_session(deps.agent.as_ref(), &t).await?;
        let bus = deps.buses.open_for(&sd)?;
        loop {
            match drive(&sd, deps, StopWhen::Idle).await {
                Finished{..} if !bus.has_pending().await? => break,
                NotDriver{..} | Unregistered | BindFailed{..} | RecoveryBlocked(..) => break,       // 已回报，等人工处置
                Busy{..} | RunBusy{..} | LeaseLost => bus.wait(BUSY_RETRY).await,                  // 别的 xagent / xllm 在推进
                Idle{..} | Error{..} | TurnOpen{..} | TurnClosed{..} | OutcomesHandled{..} => {
                    if !bus.wait_until(idle_unload).await && !bus.has_pending().await? { break; }  // 卸载；下次 ensure 再起
                }
            }
        }
    })).await;
}
```

这与计划附录 A.2 的 `Supervisor::session_loop` 一致：OpenDAN 日后托管就是把 `serve` 的循环放进协程。

---

## 10. 验证矩阵：xagent 要跑通的实验

每个实验都是一条脚本（python mock LLM，见 xllm_rust_sdk.md §9；不需要 BuckyOS），结果写进 `tests/xagent/`，并作为 `cargo test -p libopendan --test xagent` 的用例。

| # | 验证的边界 | 步骤 | 判据 |
|---|---|---|---|
| E1 | LLM Context ↔ Session | `xagent new … --input "任务"` 跑到 `--until outcomes:1`（run 停在中途，mock 让它 Interrupted），`xagent xllm <sid>` 给出命令，`xllm --resume --runs-dir …` 跑完该 run，再 `xagent run <sid>` | xllm 接手成功且不重装配；xagent 的 `reconcile_runs` 把 xllm 跑完的 run flush 进 worklog 并关闭 Turn；Round 计数两边累加不覆盖；G2 的渲染差异被记录 |
| E2 | behavior 冻结 | 建 session（冻结 plan/do），改 `behaviors/plan.toml` 的 system 模板与 tool_whitelist；`xagent run` 两个 Turn；再 `xagent new` 一个新 session | 旧 session 每个新 run 的 system 段 digest 相同、工具集不变；新 session 用新配置；`xagent behaviors --frozen <sid>` 与目录可对比 |
| E3 | Runtime 可替换、绑定不可换 | 同一 session_config 分别用 `--runtime native` / `--runtime tmux` 创建两个 session；对已绑定 native 的 session 再 `--runtime tmux` | 两种 runtime 下 worklog 形状相同（exec 结果一致）；换 runtime 返回 RuntimeMismatch，退出码 6，无推理 |
| E4 | Agent State 实现可替换 | 同一 fixtures 分别用 `FsAgentStateClient`、`InProcess`、转发桩 `KrpcAgentStateClient` 驱动 | `doc/opendan/protocol/fixtures` 全部 expected.json 通过；Runner 代码不含实现分支 |
| E5 | 工具两种形态 | 场景 a：只用层 ② 工具（`agent-session recall/note/perceive`）；场景 b：注入一个层 ③ 工具 | a：xllm 可接手；b：`runtime.requirement.app_tools` 非空，`xllm --resume` 明确拒绝 |
| E6 | Turn 语义 | mock 先回 `WAIT_USER_MSG` 且无 report；再投递一条 msg；mock 回 report | 第一次 `run` 退出码 3、`open_turn` 保留；第二次输入并入同一 Turn（worklog `input_batch` 而非 `turn_started`）；最终 `turn_ended completed`，`turns_completed = 1` |
| E7 | 崩溃恢复同一 Turn | `LIBOPENDAN_FAULT=input_batch:after_state_commit`（及 `finish_run:after_flush`）让 xagent abort；再 `xagent run` | 恢复后不重复注入、不倒退消费位置；Turn 编号不变；xllm 在 `host_commit_pending` 期间拒绝接手 |
| E8 | 推进权 | 两个 xagent 同时 `run`；`serve` 期间另开 `run` | 后者 Busy（退出码 5）；lease 丢失即停 |
| E9 | 三种输入形态等价 | 同一条 msg 分别经 `run --input`、预先 `post` 再 `run`、`serve` 中 `post` 进入 | worklog 的 `turn_started.inputs` / receipt 结构一致，只有 key 与 from 不同 |
| E10 | 切换不断 Turn | behavior 图 plan →(fork) do → 返回 plan →(independent) review → END | 全程一个 Turn；`process_result` 注入父 run；independent 重入恢复同一 run；`static.json turns = 1` |
| E11 | 新鲜量不进 system | 同一 session 两个 Turn 间隔一段时间 | 两个 run 的 system 段字节相同；时间只出现在 `<session_input time=…>` |
| E12 | 冻结缺失的阻塞 | 删除 `prompt.frozen` 且让目录不可读 | `RecoveryBlocked`，退出码 6，不猜 behavior、不推理 |

---

## 11. 对 libopendan 的改动清单（实施顺序）

| # | 改动 | 位置 | 备注 |
|---|---|---|---|
| C1 | `StopWhen::TurnClosed`、`DriveResult::{TurnClosed, TurnOpen}` | `runner/mod.rs`、`drive.rs` 3b / 3f | 唯一的循环改动 |
| C2 | `BehaviorCatalog` trait + `AgentStateClient::behaviors()`；`FsAgentStateClient` 读 `behaviors/`、`role.md`、`self.md`、`i18n/`，include 展开复用 llm_context `PromptRenderEngine` 的 `__INCLUDE__` | `state/behaviors.rs`（新） | 文件版 `revision()` = 相关文件 (path, mtime, size) 的 sha256 |
| C3 | `BehaviorConfig`（移植 `opendan::behavior_cfg`，加 `meta.next`、`prompt.mode`、`switch_mode`）；`overlay_llm_context`、`to_budget_spec` | `protocol/behavior.rs`（新） | 进 JSON Schema 导出 |
| C4 | `PromptSection.frozen`；schema `opendan.session_config/3`；`create_session` 可选冻结；`drive` 的 `ensure_frozen` / `ensure_frozen_behavior`；worklog `control_applied{behavior_frozen}` | `protocol/config.rs`、`api.rs`、`drive.rs` | 旧 schema 不迁移（现有规则） |
| C5 | `BehaviorAssembler`（`system_text` / `render_input` / `process_mode`）；`DefaultAssembler` 保留为无 behavior 时的退路；`extensions.opendan.process_modes` 删除 | `runner/assembler.rs` | 渲染无副作用；游标进 state.json |
| C6 | `SessionToolProvider`、`RunnerDeps.tool_providers`，`app_tools` 自动汇总；`SessionToolManager` 加 deadline / cancel（G5） | `runner/mod.rs`、`runner/tools.rs` | 首个 provider：fork（`try_create_worksession` 后移） |
| C7 | `AgentStateClient::connect` + `StateLocator`；`InProcessAgentState`（测试 / 嵌入）；`KrpcAgentStateClient` 构造函数 + 转发桩 | `state/connect.rs`（新） | 真 kRPC 随 OpenDAN 改造 |
| C8 | `RuntimeRegistry`；`ContainerRuntime` 只有 descriptor | `runtime/registry.rs`（新） | |
| C9 | `src/bin/xagent.rs` 替代 `examples/session.rs`；`.runtime/bin/agent-session` 指向它；层 ② 子命令 | `bin/`、`runtime/bin_overlay.rs` | `agent_tool` 不能依赖 libopendan，所以 xagent 不进 `agent_tool xllm` 那套入口 |
| C10 | xllm 侧（`agent_tool`）：`prepare_hosted` 的 `HostProtocolFlavor`（G3）、`hosted_request` 接受 budget 覆盖、resume 读 `host.extra.renderer_opts`（G2）与校验 `env_check`（G6）、hosted `llm_context` 含被忽略键时报错（G1） | `xllm.rs` | 都是可选能力，xllm 自身行为不变 |
| C11 | 文档：readme 的 AgentRuntime / 命令行工具两段按本文更新；protocol Spec 加 `prompt.frozen`、`behavior` schema；fixtures 重生成 | `doc/llm_context/readme.md`、`doc/opendan/protocol/` | 实现后反写（V6） |

依赖顺序：C1 → C2/C3 → C4/C5 → C6/C7/C8 → C9 → C10 → C11。E1–E12 随 C9 落地。

---

## 12. 待确认

1. **冻结范围**（§6.3）：入口 + `meta.next` 闭包 + 按需补冻结，还是创建时冻结整个 behavior 目录？前者文件小、允许自由跳转；后者审计简单、`catalog_rev` 一次说清。本文按前者。
2. **`loop_mode` 的归属**：现在在 `agent.toml [session.<class>]`，本文移到 behavior 的 `prompt.mode`（一个 session 内不同 behavior 可用不同 loop）。这要求普通切换时 `loop_model` 变化 → 视为 independent（不同 parser / renderer 不能共用一个 run）。是否接受？
3. **`switch_mode` 放目标 behavior**：readme TODO 的回答。接受后 `extensions.opendan.process_modes` 直接删除，还是保留一个版本作覆盖？本文直接删除。
4. **层 ③ 工具的首个成员**：fork 原语是否在 xagent 阶段就要？不要的话 C6 只留接口，E5b 用一个 echo 工具验证拒绝接手。
5. **`WAIT_USER_MSG` 未回复时的退出码 3 与 `serve`**：serve 形态下这是正常等待；run 形态下脚本需要区分“Turn 打开等输入”与“可恢复挂起（工具 / 中断）”，`--format json` 的 `waiting_for.kind` 已能区分，是否还需要不同退出码？
6. **xagent 放哪**：本文放 `libopendan` 的 `src/bin/xagent.rs`。另一选项是独立 crate `src/frame/xagent`，便于以后放 TS 版对照与更多子命令；二者对设计无影响。
7. **`AgentStateClient::connect` 的 did → AgentRoot 映射来源**：system-config `users/<owner>/agents/<id>` 还没有 root 字段；v1 先用 `~/.opendan/agents.toml` + 环境变量，待 OpenDAN 改造时统一。
