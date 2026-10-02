# xAgent：Agent Session 分层验证工具设计

- 状态：草案 v0.1（2026-10-02），待 Review 后开始实施
- 位置：`src/frame/lib_opendan`（二进制 `xagent`，替代 `examples/session.rs`）
- 依据：[Agent Session SDK 实现计划](<./Agent Session SDK 实现计划.md>) v0.10、[LLM Context readme](../llm_context/readme.md)、[xllm Rust SDK 参考](../llm_context/xllm_rust_sdk.md)、`doc/opendan/protocol/`
- 读者：决定是否按本文实施 xagent 的人；文中“现状”都对照 2026-10-02 的 `libopendan` / `agent_tool` 代码

---

## 0. 结论先行

xagent 的目的是**在一个新产品里验证四层架构、发现设计问题并迭代**，而不是给现有 libopendan 做回归。所以本文的伪代码（§9）写的是目标设计，与现有 `runner/drive.rs` 的差距单列在 §11；凡是实验（§10）暴露出设计问题的，先改设计再改代码，等 OpenDAN 集成之后就改不动了。

1. **xagent 是 xllm 的上一层**：xllm 加载 `.llm_context`，把一个 LLMContext run 推进到一个 Outcome；xagent 加载（或创建）一个 Agent Session，把它推进到**一个 Turn 关闭**，或常驻地不断完成 Turn。两者的 run 目录相同（`runs/` 就是 xllm 的 run 目录），同一个 run 可以在两者之间交接，这是验证 L2 / L3 边界的主要手段。
2. **Agent Session 构造 llm_context 复用 xllm 的宿主装配 API**（`XllmTask::prepare_hosted` / `hosted_request` / `hosted_waist_deps` / `rebuild_toolset` / `create_run_llm` / `RunStore`），但 **system 段、历史段、输入批次、工具调度包装、checkpoint 钩子、run 生命周期都由 Session 决定**；差异清单见 §3。
3. **Agent 感知到的输入只有两种：`AgentMessage` 与 `AgentEvent`**（§4）。Session 不关心它们怎么来的，只要求信封（key、来源与 index、from、at_ms、subscription_id）；把系统事件（msg-center、kevent、timer、task_mgr、子 session）翻译成这两种输入的是上层 **bridge**（xagent serve、以后的 OpenDAN Supervisor、应用）。事件是唤醒还是只在观察边界注入，由 **Session 按自己的订阅配置决定**，不由 producer 决定；半订阅事件按 key 保留在 state.json，空闲时不丢。stop / decide / subscribe / activity / perceive 不是 Agent 输入，是**Session 控制协议**，只是搭同一条队列。
4. **Session 模板**（§4.7）决定一个 session 的形态：Turn 上限、`WAIT_USER_MSG` 的含义、要不要输入队列、观察注入开关、hints、默认 behavior。work 模板 = 一个 Turn、不等用户、默认不建队列；ui 模板 = 无限 Turn、有队列。模板是 `SessionSpec` 的预设，创建时解析进 session_config，不是新协议对象。
5. **Runtime 是"exec 在哪里跑"的执行环境**（§5），按 id 选择、首次推进时绑定；它不是工具调度层。工具分三层：xllm 内置、`.runtime/bin` 中 CLI 形态的 session 工具（优先）、进程内 session-aware 工具（声明为 `app_tools`，xllm 不能接手）。
6. **behavior 配置来自 Agent State，在 Session 构造时冻结进 `session_config.prompt`**（§6）；切换模式由**目标 behavior** 的冻结配置决定。
7. **Agent State 不配置**（§7）：`AgentStateClient::connect(agent_did, who)` 按"进程内 → 本机 AgentRoot → kRPC"解析；Runner 只依赖 trait。
8. **验收项**是 E1（xllm 接手）、E4（换 Agent State 实现）、E5（工具形态）、E13（无队列 work session）、E14（半订阅注入时机），其余实验是回归（§10）。

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

| 环节 | xllm（`XllmTask::prepare` → `XllmRun`） | Agent Session（libopendan `runner/live.rs::new_run_context_plain`） |
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

## 4. Agent Input：原理、协议对象、模板

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

**内置 bridge**：Session 来源的订阅（`source = session`）现在由 Runner 在观察边界拉登记表比 rev，没有外部 producer。按原理它也要产出 AgentEvent，所以把它定义为 Runner 内的内置 bridge：比对 rev 后合成 `AgentEvent{subscription_id, source: Session, seq: rev, summary: watched 字段差异}`，再走同一套 `pending_events` 与注入逻辑。worklog 与实验断言只看一种形态。

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
| 子 session 桥 | Runner 内置（§4.3） | `AgentEvent{source: Session}` |
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
    pub system_event_delivery: fn(&AgentEvent) -> Delivery,
}
```

| class | turns | wait_user_msg | input | observe | hints | 说明 |
|---|---|---|---|---|---|---|
| `work` | One | FinishFailed | **None**（声明了订阅则升为 Queue） | Events | 否 | 一次性任务：一个 Turn 跑到结果；不等用户；默认没有队列 |
| `ui` | Unbounded | Allowed | Queue | EventsAndActive | 是 | 长生命周期对话入口（后移） |
| `self_improve` | One | FinishFailed | None | Off | 否 | 感知窗口整理 |
| `self_check` | Unbounded | FinishFailed | Queue（timer 事件） | Off | 是 | 定时自检 |

**无队列 work session 的后果**（接受这些才成立）：stop 只能由 lease 持有者自己退出；decide 走 artifacts 门面不经 session；exec 子进程的 `activity --touch` / `perceive` 没有通道，touching 由工具调用推断、感知只有结束时的 run_digest；父 session 对子 session 的半订阅（S-17）要求**父**有队列，父通常是 ui 类。声明了订阅的 work session 需要队列承接 bridge 的事件，模板自动升为 Queue。

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

Turn 规则沿用 readme：没有打开的 Turn 时提交的输入批次开启新 Turn；切换、fork、观察注入、挂起、重写、重启都延续；只有 `finish_run` / `stop_session` 关闭。模板的 `wait_user_msg` 决定 `WAIT_USER_MSG` 在该 session 里是"保持打开"还是"结束"。

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
| ② CLI 形态 session 工具 | `agent-session ctl activity|perceive|decide / post / recall / note / sessions / read-session / create-worksession / artifact` | Runtime 的 `.runtime/bin` 把 xagent 自身包装进去（现有 `bin_overlay`） | 子进程里 `AgentStateClient::connect(agent_did, who)`（§7）；对**本 session** 状态的修改一律投递 control 到 InputBus 由驱动者应用，不直接写 | 能（它们只是 PATH 上的命令；xllm 的 exec 同样能调） |
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
| `ctl activity --summary/--touch`、`ctl perceive` | — | 控制协议：投递到本 session 队列（§4.4） | 驱动者在 `apply_controls` 应用；无队列 session 不可用 |
| `recall <query>` | `cognition().recall_hints` | 读 | 任何 session 都可 |
| `note <text>` | `cognition().notebook_append(who)` | 写 | 显式声明，不经 lease |
| `sessions [--active]`、`read-session <sid>` | `sessions().query/lookup` + 目录读 | 读 | 遵守 `acl.agent_access` |
| `create-worksession`、`post <sid>` | `sessions().register` / `post_input` | 写 | 子 session 的 driver 默认是调用者 who |
| `artifact register/head` | `artifacts()` | 写 / 读 | 版本登记在 `finish_run` 由 Runner 做，CLI 只给 Agent 中途登记用 |
| `ctl decide` | `artifacts().decide` | 写 | 有队列：control(decide)，finished 后由驱动者执行；无队列：直接走门面 |

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
| `identity.*`、`behavior.{name,objective,mode}`、`session.{id,kind,objective,driver,scope}`、`paths.{session_root,workspace_root}`（相对或 binding 提供）、`workspace.id`、`xml_behavior_result_protocol`（由 xllm `runtime_protocol` 段提供） | `runtime.{clock_text,status}`、`<active_sessions>`、`<hints>`、`<events>`（pending_events 注入）、`<perceptions>`、`<inputs>`、`behavior_switch / process_result`、`session.current_todo*`（读 `todos.json`）、`notebook.last_items`、`workspace_list` |

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

  xagent new    --agent <did> --objective <text> [--class work|ui|self_improve|self_check] [--parent <dir>]
                [--behavior <name>] [--llm-context <json|@file>] [--runtime <id>] [--workspace <path>]
                [--subscribe <spec>]... [--msg <text>] [--until turn|finished|idle|outcomes:<n>] [--no-run]
                 按模板创建 session（冻结 behavior 与模板），默认立刻推进一个 Turn
  xagent run    <session_dir|sid> [--msg <text> | --msg-file <path> | --event <json>]
                [--until turn|finished|idle|outcomes:<n>] [--runtime <id>] [--no-bridge]
                 投递（可选）后推进到 Turn 关闭；默认 --until turn
  xagent serve  <session_dir|sid>... [--idle-unload <secs>] [--no-bridge]
                 常驻：起事件桥；drive(Idle) → 等唤醒 → drive(Idle)…
  xagent post   <sid> (--msg <text> | --event <json>)                 Agent 输入（AgentMessage / AgentEvent）
  xagent ctl    <sid> (stop | decide accept|discard | subscribe <spec> | unsubscribe <id> | activity ... | perceive <text>)
                 Session 控制协议
  xagent status <sid> [--worklog <n>] [--report] [--run] [--events]    状态；--events 显示 pending_events 与最近注入
  xagent list   --agent <did> [--active]
  xagent behaviors --agent <did> [--frozen <sid>]
  xagent xllm   <sid> [--run <id>]                                     打印让 xllm 接手 live run 的命令行

  <spec> = active|semi:session:<sid>[:watch=f1,f2] | active|semi:object:<id>#<event>

  在 exec 子进程内（PATH 上的 agent-session 即 xagent）：
  xagent ctl activity [--summary <t>] [--touch <ref>]... | ctl perceive <text>
  xagent recall <query> | note <text> | sessions [--active] | read-session <sid> | create-worksession --objective <t> [...] | artifact ...

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
    runtime: Arc<dyn AgentRuntime>,
    xllm: XllmDeps,
    assembler: Arc<dyn SessionAssembler>,             // BehaviorAssembler
    tool_providers: Vec<Arc<dyn SessionToolProvider>>,// NEW 层 ③
    bridges: Vec<Arc<dyn EventBridge>>,               // NEW 进程内事件桥（kevent / timer）；--no-bridge 为空
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
    kind: NextKind,            // Done | Wait | ProcessDone | Switch(b) | Budget | Error | Stopped | Interrupted | PendingTool | ContextLimit
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
        tool_providers: vec![],                                                                 // fork 原语是否在本阶段做：§12
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
    let env = deps.runtime.open_session_env(&binding, &s.env_ctx()).await?;
    let factory = ContextFactory::new(&s, deps, &env, lease.clone());
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
        let next = s.handle_outcome(&mut lc, outcome, deps).await?;                                  // §9.5；WAIT_USER_MSG 按模板解释   // NEW
        outcomes += 1;
        if let Some(status) = next.turn_end { turn_closed = Some((s.state.last_closed_turn(), status, next.answer.clone())); }
        if !next.run_ended && !next.suspended { live = Some(lc); }

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

- **Turn 边界只在两处改变**：`commit_input_batch`（开启或并入）与 `finish_run` / `stop_session`（关闭）。切换、fork、independent、观察注入、挂起、重写、恢复都不碰 `open_turn`。
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
        let xdeps = self.deps.xllm.clone()
            .with_bash_runner(self.runtime.bash_runner(&self.env, Arc::new(LateRegistrar::new())))
            .with_host_tools(self.provider_tools(s))                                     // NEW 层 ③
            .with_skip_workdir_lock(true);
        let hosted = XllmTask::prepare_hosted(&self.env.workdir, &llm_context, "session_config.prompt", &system, &xdeps)
            .with_protocol_flavor(HostProtocolFlavor::Session).await?;                   // NEW G3：runtime_protocol 不再说"一次性、不要提问"
        let llm = counted(hosted.create_llm(&xdeps).await?, rounds.clone());

        let budget = s.cfg.prompt.history_budget_tokens.unwrap_or(self.deps.options.history_budget_tokens);
        let history = build_history(s, &self.lease, self.summarizer(&llm), budget).await?;   // summary.json + 反向读 worklog；不够先 compact

        let run_id = s.runs.create_locked()?;
        let record = hosted.new_record(&run_id, Some(s.runs.dir()), &s.cfg.session.objective,
            HostRunInfo { assembled_by: "libopendan", session_id: s.sid(), runtime_kind: self.binding.kind, runtime_id: self.binding.runtime_id,
                          env_check: self.env.check_digest(), extra: json!({ "behavior": entry, "renderer_opts": {"timestamps": false} }) });   // NEW G2
        let run = RunHandle::new(s.runs.clone(), record)?; run.write()?;

        let tools = Arc::new(SessionToolManager::new(Arc::new(hosted.manager), run.clone(), self.lease.clone(), self.env.workdir.clone(), self.touched.clone())
            .with_deadline(behavior.budget.max_wallclock_ms));                            // NEW G5
        let mut input = vec![AiMessage::system(system)]; if let Some(h) = history { input.push(h); }
        let request = hosted_request(&hosted.config, ContextOwnerRef::Agent{session_id: s.sid()}, &run_id, &s.cfg.session.objective, &entry, input)
            .with_budget(behavior.budget.to_budget_spec());                              // NEW
        let waist = hosted_waist_deps(&hosted.config, llm, tools)
            .with_checkpoint_hook(Arc::new(SessionCheckpointHook::new(self.shared.clone(), run.clone(), hosted.config.loop_model == Behavior)));

        let mut ctx = match s.fork_parent() {
            Some(parent) => { let mut snap = LLMContextSnapshot::fresh(request, host_meta(s, &entry)); snap.inherit_steps_from(&parent.latest_snapshot()?); LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, waist)? }
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
        let tools = rebuild_toolset(&record, &self.xdeps_for(s)).await?;
        let llm = counted(create_run_llm(&record, &self.xdeps).await?, rounds.clone());
        if !record.inflight.is_empty() { snap = materialize_unresolved(&snap, &record.inflight, record.behavior_mode())?; run.checkpoint_with_results(&snap, None)?; }
        if snap.request.behavior_name != s.state.current_behavior { snap.request.behavior_name = s.state.current_behavior.clone(); }
        if let Some(pr) = &s.state.process_result { snap.bump_ids_after(pr); }
        let waist = hosted_waist_deps(&record.config, llm, Arc::new(SessionToolManager::new(..))).with_checkpoint_hook(..);
        let ctx = match snap.suspended {
            Some(Suspension::PendingTool) => return Err(RecoveryBlocked("deferred tool results are not supplied by this runner")),
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
    async fn handle_outcome(&mut self, lc: &mut LiveCtx, outcome: LLMContextOutcome, deps: &Deps) -> Result<Next> {
        let snap = outcome.snapshot().cloned().unwrap_or_else(|| lc.ctx.snapshot());
        let mut next = match outcome {
            Done{behavior_result, response, ..} => {
                let nb = behavior_result.as_ref().and_then(|b| b.next_behavior.clone());
                let answer = behavior_result.self_report().or(response.text());
                let replied = snap.has_report() || !behavior_result.messages_to_send().is_empty();
                match (self.is_fork_child(&lc.run), nb.as_deref()) {
                    (true, nb) if nb != Some(WAIT_USER_MSG) => Next::process_done(answer),
                    (_, Some(WAIT_USER_MSG)) => match self.policy().wait_user_msg {                 // NEW：模板解释 WAIT_USER_MSG
                        WaitPolicy::Allowed         => Next::wait(answer, turn_end = replied.then_some(Completed)),   // 没交付回复 → Turn 保持打开
                        WaitPolicy::FinishFailed    => Next::failed(json!({"kind": "needs_user_input", "question": answer})),   // work：不许等人
                        WaitPolicy::FinishCompleted => self.decide_end(answer),
                    },
                    (_, Some(b)) if b != END && b != "done" => Next::switch(b),
                    _ => self.decide_end(answer),                                                   // Completed；按 end_condition（模板 turns）判 finished / waiting
                }
            }
            BudgetExhausted{which, ..} => Next::budget(which),
            Error{error, ..} if is_retryable(&error) => Next::retryable(error),
            Error{error, ..} => Next::failed(error),
            Interrupted{..} if self.state.stop_requested => Next::stopped(),
            Interrupted{..} => Next::interrupted(),
            PendingTool{..} => Next::pending_tool(),
            ContextLimitReached{..} => Next::context_limit(),
        };

        if next.run_ended { lc.run.checkpoint_finish(&snap, next.run_status(), &next)?; } else { lc.run.checkpoint_with_results(&snap, Some(next.run_status()))?; }
        lc.run.record_usage(outcome.usage(), lc.rounds.take())?; self.static_add_rounds(..);

        match &next.kind {
            NextKind::Switch(b) => match deps.assembler.process_mode(&self.cfg, b) {               // NEW：目标 behavior 的冻结 switch_mode
                None => {                                                                           // 普通切换：同一 run、同一 context、同一 Turn
                    self.ensure_frozen_behavior(b, deps).await?;                                    // NEW
                    lc.ctx = LLMContext::resume(snap.with_behavior(b), ResumeFill::ResumeFromMidRun, lc.ctx.take_deps())?;
                    self.state.current_behavior = Some(b.clone()); self.state.internal_continuation = Some(b.clone()); self.state.run_state = Ready; commit!(self);
                }
                Some(mode) => { self.suspend_run(lc, b, mode, &snap).await?; next.suspended = true; }
            },
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

每个实验都是一条脚本（python mock LLM，见 xllm_rust_sdk.md §9；不需要 BuckyOS），结果写进 `tests/xagent/`，并作为 `cargo test -p libopendan --test xagent` 的用例。**验收项**（E1、E4、E5、E13、E14）失败说明分层有问题，先改设计；**回归项**失败说明实现有问题。

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
| E16 | 回归 | 内置 session bridge | 父（ui）半订阅子（work）；子结束 | 父的下一个观察边界或下一批里出现 `AgentEvent{source: Session, terminal: true}`，与外部 bridge 产出同形 |

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
| C8 | `SessionToolProvider`、`RunnerDeps.tool_providers`，`app_tools` 自动汇总；`SessionToolManager` 加 deadline / cancel（G5） | `runner/mod.rs`、`runner/tools.rs` | 首个成员待定（§12） |
| C9 | `AgentStateClient::connect` + `StateLocator`；`InProcessAgentState`；`Krpc` 构造函数 + 转发桩 | 新 `state/connect.rs` | 真 kRPC 随 OpenDAN 改造 |
| C10 | `RuntimeRegistry`；`ContainerRuntime` 只有 descriptor | 新 `runtime/registry.rs` | |
| C11 | `examples/session.rs` → `src/bin/xagent.rs`（§8 命令面：`new/run/serve/post/ctl/status/...`）；`.runtime/bin/agent-session` 指向它；层 ② 子命令 | `bin/`、`runtime/bin_overlay.rs` | 旧 CLI 删除 |
| C12 | xllm 侧（`agent_tool`）：`prepare_hosted` 的 `HostProtocolFlavor`（G3）、`hosted_request` 接受 budget 覆盖、resume 读 `host.extra.renderer_opts`（G2）与校验 `env_check`（G6）、hosted `llm_context` 含被忽略键时报错（G1） | `xllm.rs` | 可选能力，xllm 自身行为不变 |
| C13 | 文档：readme 的 AgentRuntime / 命令行工具两段、protocol Spec（输入与控制两篇、`prompt.frozen`、`session.policy`、`pending_events`、behavior schema）、fixtures 重生成 | `doc/llm_context/readme.md`、`doc/opendan/protocol/` | 实现后反写（V6） |

依赖顺序：C1 → C2/C3 → C4/C5 → C6 → C7 → C8/C9/C10 → C11 → C12 → C13。验收项 E1/E4/E5/E13/E14 随 C11 落地；E13 依赖 C4/C5，E14 依赖 C1–C3。

---

## 12. 待确认

1. **冻结范围**（§6.3）：入口 + `meta.next` 闭包 + 按需补冻结，还是创建时冻结整个 behavior 目录？本文按前者。
2. **`loop_mode` 的归属**：移到 behavior 的 `prompt.mode`。普通切换时 `loop_model` 变化 → 视为 independent（parser / renderer 不能共用一个 run）。是否接受？
3. **work 模板下 `WAIT_USER_MSG` 的收尾**：本文定为 `FinishFailed`（Turn `failed{needs_user_input}`，问题进 report）。另一选项 `FinishCompleted`（把问题当结果交付）。
4. **无队列 work session 的后果**（§4.7）：stop 无法远程、decide 不经 session、子进程无 activity / perceive 通道。是否全部接受？若 perceive 必须保留，就得给无队列 session 一条只收控制命令的轻量通道，等于又建队列。
5. **控制协议是否继续搭输入队列**：本文保持同一条队列（多方投递、持久化现成）。另一选项是控制走 kRPC 直达驱动者，队列只放 Agent 输入；代价是驱动者不在线时控制命令丢失。
6. **AgentEvent.summary 上限与 data_ref**：1 KB 是否够；大 payload 一律 NamedStore 引用。
7. **层 ③ 工具的首个成员**：fork 原语是否在 xagent 阶段做？不做则 C8 只留接口，E5b 用 echo 工具。
8. **xagent 放哪**：`libopendan` 的 `src/bin/xagent.rs`，或独立 crate `src/frame/xagent`。
9. **`AgentStateClient::connect` 的 did → AgentRoot 映射来源**：v1 用 `~/.opendan/agents.toml` + 环境变量，OpenDAN 改造时统一。
