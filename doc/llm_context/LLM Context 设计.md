# LLM Context 设计

> 本文是 OpenDAN Agent 框架的第一篇文档。读完本文，你应该掌握：
>
> 1. 为什么需要 LLMContext，它和 `llm.complete` / `agent.sendMsg` 的差异
> 2. 它的心智模型（"LLM 进程上下文"）和六态退出
> 3. 它和 Behavior Loop / Prompt 管线 / Tool 调度 / Snapshot 之间的关系
> 4. 写新 scheduler / provider / tool 时哪些字段进 waist、哪些不该进
>
> 术语：Round = 一次推理，Step = Behavior Loop 的一次决策记录（`StepRecord`），Turn = AgentSession 的一次逻辑 Input → result，工具迭代 = 工具预算的计量单位；定义见 [readme](readme.md)。本文的 `run()` 指一次 `LLMContext::run()` 调用。

---

## 0. Preamble — 设计纪律：Narrow Waist

LLMContext 是一个 **narrow waist primitive（瘦腰原语）**，类比 IP 包之于互联网、POSIX file 之于 OS、LLVM IR 之于编译器。瘦腰不是设计目标，是**设计纪律**。

### 双向中立性判据

每个候选字段 / 方法，必须**同时**通过两条测试，才能进入 LLMContext：

1. **Scheduler 中立**：Agent / Workflow / Shell / Hook / Eval / Multi-agent 任何一种调度器换上来，这个字段都同样自然。偏向某一种 → 它属于**上面那层**。
2. **Provider 中立**：底层换成 Claude / GPT / Gemini / 本地模型 / MCP，这个字段都同样自然。偏向某一种 → 它属于**下面那层**。

任何一项不通过，**默认拒绝进入 waist**。

### 纪律

- **稳定性 > 完备性**：waist 的 breaking change 会同时打到所有上下游。宁可 waist 少一个字段、让某个 scheduler 自己在外面包一层。
- **瘦不下来就不加**：一个能力没办法在不破坏中立性的前提下进 waist —— 这是 **waist 在拒绝它**，不是 waist 不够强。它应该去 scheduler 层（上）或 effect / provider 实现层（下）找位置。
- **PR review 标准**：每个改动 LLMContext 公共类型的 PR，必须显式回答"这个改动是否破坏 scheduler 中立 / provider 中立"。
- **Non-Goals 是活清单，只增不减**：见 Appendix A。

### 真正的回报

不在 waist 自身有多优雅，而在 **waist 立住之后上下游各自的 Cambrian explosion**：

- **上面**：Agent / Workflow / Shell / Hook / Pipeline / Eval / Multi-agent 互不知道彼此地各自演化
- **下面**：LLM provider / tool / sandbox / memory backend 互不知道上面地各自演化

LLMContext 自己越薄，上下游能长出来的东西越多。

---

## 1. 背景与心智模型

### 1.1 为什么需要中间层

业界事实上只有两个粒度：

- **`llm.complete(prompt) → text`** —— 太低阶。每个 scheduler 都要自己拼 prompt、自己跑 tool loop、自己管 budget / 结构化输出 / 重试 / 审计，等于"在每个 scheduler 里重写一个迷你 agent runtime"。
- **`agent.sendMsg(session, msg)`** —— 太重型。一来就绑长生命会话、行为机、长期记忆、容器编排，scheduler 只是想"跑一次 LLM + 几个工具"也得吞下这整套。

`LLMContext` 是中间那一层：**进程粒度的 LLM 执行体**，有 agent runtime 的核心能力，但没有 session 的长生命语义。

### 1.2 心智模型：LLM Context as Process Context

`LLMContext` 不是"传给 LLM 的 messages 容器"，而是 OS 意义上的**进程上下文**（PCB）—— 一段可挂起、可恢复、可被调度器管理的有界 LLM 执行体。

| OS 概念 | LLMContext 对应物 |
|---|---|
| Process Control Block (PCB) | `LLMContext` 自身：prompt 编译产物 + tool loop 中间态 + token usage |
| Registers + Stack | `LLMContextState`：可序列化的运行时可变态 |
| Yield / context switch out | `Outcome::PendingTool` / `Outcome::ContextLimitReached`：cooperative yield |
| Preemptive interrupt | `LLMContextInterruptHandle::interrupt(...)`：从 run 外部抢占当前 inference |
| Context switch in | `LLMContext::resume(snapshot, fill, deps)`：恢复挂起态继续跑 |
| Killed by scheduler | `Outcome::BudgetExhausted`：quantum / token / wallclock 任一耗尽 |
| exit syscall | `Outcome::Done` / `Outcome::Error`：正常 / 异常终止 |
| Scheduler | Agent loop / Workflow engine：决定哪个 context 上 CPU |
| Process lifetime | 短生命：一个 LLMContext = "一次智能任务"，不是 Agent 的整段会话 |

由此推出后续所有设计：

- **为什么是对象不是函数**：进程上下文必须有可变 runtime 状态。
- **为什么六态退出**：进程要么 exit、要么 yield 等 IO、要么 yield 等 context 压缩、要么被外部 interrupt 抢占、要么被 kill —— 不可能只有"返回值"。
- **为什么 owner / scheduler 抽象**：scheduler 不关心进程跑什么业务，只关心生命周期。
- **为什么需要 snapshot**：挂起必须能完整保存执行态以便恢复。

**重要约束：cooperative yield 与 preemptive interrupt 是两条独立控制面。** `PendingTool` / `ContextLimitReached` 在推理边界产生（工具派发中、请求发送前或 provider 拒绝后），不打断进行中的 inference；`Interrupted` 在 inference 过程中由外部触发。"等待用户下一条消息"不属于 waist 挂起态，由 L4 / session 解释（典型 sentinel `next_behavior == "WAIT_USER_MSG"`）。

### 1.3 Loop 不变量：intent → effect → observation

LLMContext loop 不变量**不是** function call → tool result，而是：

```
intent → effect → observation → intent → effect → observation → ... → terminal
```

| 概念 | 在 waist 里的载体 | 谁产生 | 谁消费 |
|---|---|---|---|
| **intent** | `OutputSpec::Json` 解析出的结构化产物（典型字段 `tool_calls / do_actions`）或 provider-native tool_calls | LLM | waist 主循环 → ToolManager |
| **effect** | `ToolManager::call_tool` 内部的实际动作 | ToolManager（effect 实现层） | 外部世界 |
| **observation** | `Observation::{Success \| Error \| Pending \| Cancelled \| Unresolved}` | ToolManager（`Unresolved` 由 waist 在批次中断时补齐） | waist 主循环 → 喂回下一次推理（Round） |

**为什么不把 function call 抬成一等公民**：function call 是 provider-specific wire format（OpenAI tool_calls / Anthropic tool_use / Gemini function_call / 本地模型经常没有原生支持各家细节都不同），抬上来立刻丢掉 provider 中立性。provider adapter 负责把各家 wire format 归一化成 `AiResponseSummary.tool_calls: Vec<AiToolCall>`，waist 只看到归一化后的列表。

---

## 2. 四层分层

```
┌─────────────────────────────────────────────────────────────┐
│  L4  Scheduler-facing 语义层（DSL / 配置文件直接面向）      │
│  - LLMAgentContext     角色 / behavior 配置                 │
│  - LLMWorkflowContext  workflow DSL 节点                    │
│  - LLMOneShotContext   CLI 参数                             │
│  各自负责 lowering 到 L2 LLMContextRequest + Deps           │
│  scheduler-specific 字段（service endpoint / 上下游引用 /   │
│  角色 md / 行为状态机 / 容器句柄）只在这层出现              │
└──────────────────────────┬──────────────────────────────────┘
                           │ lowering
┌──────────────────────────▼──────────────────────────────────┐
│  L3  Scheduler 调度层（OS 类比：进程调度器）                │
│  - Agent loop          消息驱动，长生命                     │
│  - Workflow engine     DAG / 状态机驱动                     │
│  - OneShot scheduler   一次性脚本                           │
│  构造 LLMContextRequest（fork 新进程），按 Outcome 推进     │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│  L2  LLMContext 层（OS 类比：进程上下文）                   │
│  - 一次有界 LLM 执行：消息历史 → LLM → tool loop            │
│  - 结构化输出 / token 用量 / policy gate / interrupt        │
│  - 六态退出（终态 / 挂起态二分，见 §4）                     │
│  - cooperative yield / preemptive interrupt / resume        │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│  L1  Raw LLM 层（provider adapter）                         │
│  - 一次推理 request → response                              │
│  - 不做 tool loop / 不做可观测性 / 不做 policy              │
│  - 适合分类、摘要、结构化抽取等"一次说完"的任务             │
└─────────────────────────────────────────────────────────────┘
```

类比 LLVM：LLMContext 像 LLVM IR；各 `LLM*Context` 像 C / Rust / Swift 前端语言，各自有自己的语义层和工具链，但都 lowering 到同一份 IR 上。

### 2.1 承载方式（部署形态）

LLMContext 是一个 lib（`llm_context` crate），**不是 service**。三种承载方式共享 100% 执行语义：

1. **In-process lib** —— scheduler 直接 `LLMContext::run`，零序列化代价。
2. **Thunk 承载** —— L4 lowering 后封装为可序列化 thunk，由 workflow runtime 调度。
3. **跨设备 RPC** —— `LLMContextRequest` / `Outcome` 序列化跨节点投递。

承载方式由 scheduler 选，**不是 waist 属性**（见 §A.4）。

---

## 3. 核心抽象

### 3.1 LLMContext

```rust
pub struct LLMContext {
    request: LLMContextRequest,
    state:   LLMContextState,
    deps:    LLMContextDeps,
}

impl LLMContext {
    pub fn new(req: LLMContextRequest, deps: LLMContextDeps) -> Self;

    /// 可跨 task 持有的中断句柄。可在 run() 尚未返回时调用 interrupt(...)，
    /// 让 provider adapter 尽快取消当前 inference（§7）。
    pub fn interrupt_handle(&self) -> LLMContextInterruptHandle;

    /// 主驱动：从当前 state 向前推进，直到产生一个 outcome。
    /// done / error / budget_exhausted 是终态；
    /// pending_tool / context_limit_reached / interrupted 是挂起态。
    pub async fn run(&mut self) -> LLMContextOutcome;

    /// 从 snapshot 恢复（context switch in）。
    /// fill 的形态必须与 snapshot 的 state.suspended 对应；
    /// 不一致会返回 LLMComputeError::SnapshotCorrupted（在任何推理 / 工具调用之前）。
    /// 挂起期间的时间不计入 wallclock 预算。
    pub fn resume(
        snapshot: LLMContextSnapshot,
        fill: ResumeFill,
        deps: LLMContextDeps,
    ) -> Result<Self, LLMComputeError>;

    pub fn snapshot(&self) -> LLMContextSnapshot;
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResumeFill {
    /// PendingTool ⇒ 把 deferred 工具的结果填回。按 call_id 匹配集合：每个等待中的
    /// 调用恰好一个终态结果（Success / Error / Cancelled / Unresolved，不能是
    /// Pending，观察自带的 call_id 必须一致）；缺失、多余、重复、未知 ID 都拒绝。
    /// 顺序任意，按原调用顺序写回。回填 Cancelled 表示宿主在收尾：同批次尚未派发的
    /// 调用不再执行（记为 not executed）。
    ToolResults { results: Vec<(String, Observation)> },

    /// function call 模式的 ContextLimitReached ⇒ 重整后的历史成为新的稳定基线，
    /// 同时替换 request.input 与 accumulated。必须保持 tool_use / tool_result 配对；
    /// thinking 块被丢弃（只在产生它的前缀下有效）。如何重整完全由 scheduler 决定。
    RewrittenHistory { history: Vec<AiMessage> },

    /// behavior 模式的 ContextLimitReached ⇒ prompt 由 request.input、step 历史和热
    /// step 物化，只换 accumulated 不够。替换 request.input / history_summaries /
    /// steps / last_step；steps 与 last_step 只能保留（可压缩的）原 step，按 step_index
    /// 识别；step / action 编号、history_inputs 与进行中 Step 的 inner transcript
    /// （accumulated 中 request.input 之后的消息）保持不变；thinking 同样丢弃。把整个
    /// 物化历史折叠进 input（steps 为空、没有热 step）是合法的。
    RewrittenSteps {
        input: Vec<AiMessage>,
        history_summaries: Vec<HistorySummaryRecord>,
        steps: Vec<StepRecord>,
        last_step: Option<StepRecord>,
    },

    /// 快照不在挂起态：outcome 边界、checkpoint hook 推理前、Interrupted 之后，或
    /// fill 已应用后落盘的快照（批次 / step 可能尚未派发完，resume 后先续派）。
    /// 没有 payload；挂起态快照配此 fill 返回 SnapshotCorrupted。
    ResumeFromMidRun,
}
```

挂起后在恢复之前再调用 `run()`，返回 `Error{Internal}` 且不改动状态；`inject` 在挂起或批次未派发完时不放置任何内容（返回 `InjectionPosition::None`）。

### 3.2 LLMContextRequest

不可变输入。消息载体**直接复用 provider 抽象层的 `AiMessage`**（见 §5），waist 不再造一份 `ChatMessage`。

```rust
pub struct LLMContextRequest {
    pub owner: ContextOwnerRef,          // Agent(session_id) | Workflow(...) | OneShot(id) | Other
    pub trace: Option<String>,           // 调试 trace id
    pub objective: String,               // 自然语言目标，供 worklog 阅读，不进 prompt
    pub behavior_name: String,           // Behavior Loop 当前 behavior；普通切换由宿主改写后在同一快照上继续
    pub input: Vec<AiMessage>,           // L4 已展开的对话历史
    pub model_policy: ModelPolicy,
    pub tool_policy:  ToolPolicy,
    pub output:       OutputSpec,
    pub budget:       BudgetSpec,
    pub human_policy: HumanPolicy,
    pub error_policy: ErrorPolicy,
    /// Behavior Loop 用：true ⇒ 任何 <next_behavior> 都被丢弃。
    /// fork 子上下文用，强制本次执行结束就终止，不能跳到别的 behavior。
    pub forbid_next_behavior: bool,
}
```

设计要点：

- **不持有 session / 容器句柄**：模板展开、长期记忆注入都在 L4 lowering 阶段完成，进 waist 时 `input` 已经是具体 `Vec<AiMessage>`。
- **不重复 provider 抽象类型**：消息走 `AiMessage`、用量走 `AiUsage`、tool call 走 `AiToolCall`、最终响应走 `AiResponseSummary`。

### 3.3 ToolPolicy / Observation

```rust
pub struct ToolPolicy {
    pub mode: ToolMode,                  // None | Whitelist | All
    pub whitelist: Vec<String>,
    pub action_mode: ToolMode,           // behavior action 的派发策略（同上）
    pub action_whitelist: Vec<String>,
    /// 工具迭代额度（默认 8）：一个完成的原生工具批次，或一个派发 action 的 behavior
    /// Step，各计一次；不计推理（Round）。0 ⇒ 不派发工具（只推理一次）
    pub max_tool_iterations: u32,
    /// 单个 Round（一次模型 response）可请求的原生 tool call 数；不限制 Step 的 action 数
    pub max_calls_per_round: u32,
    pub max_observation_bytes: u32,
    pub parallel: bool,                  // 默认 false（串行）
    pub allow_deferred: bool,            // 是否允许 Pending{task_id}（宿主能在 Session 层等 task）
    pub finish_grace_ms: u64,            // 平滑结束时等待不可取消工具的时长（默认 30s）
}

pub enum Observation {
    Success { call_id, content: Value, bytes, truncated },
    Error   { call_id, message },
    /// effect 层把工作交给了 task（task_id 对 waist 不透明）→ Outcome::PendingTool；
    /// until_ms（epoch）到期后宿主按 task 当时的状态回填。缺 task_id 一律拒绝
    Pending { call_id, task_id, until_ms: Option<u64> },
    /// 调用在完成前被取消：打断 / 平滑结束 / 截止时间触发后由 ToolManager 内联返回，
    /// 或由宿主经 ResumeFill::ToolResults 回填。effect_unknown=false：工作已停止或
    /// 已知仍在运行（文本说明状态）；true：工作不能取消、等待被放弃，副作用不可确认
    Cancelled { call_id, reason, effect_unknown },
    /// 调度器没有为该调用产生结果：effect_unknown=true 表示基础设施在调用可能已
    /// 开始后失败（副作用不可确认）；false 表示批次在它开始前被中止。只由 waist
    /// 写入，用于让 transcript 与 StepRecord 保持配对、可审计。
    Unresolved { call_id, reason, effect_unknown },
}

/// 每次工具调用的上下文：与推理共用的打断 / 平滑结束信号，以及 run 的绝对截止时间
/// （按 budget.max_wallclock_ms 计算）。ToolManager 不再自己维护 cancel watch 或 deadline。
pub struct ToolCallCtx { abort: InferenceAbortToken, deadline_ms: Option<u64>, allow_deferred: bool }
pub enum CancelCause { Interrupted, Finishing, Deadline }
impl ToolCallCtx {
    pub async fn cancelled(&self) -> CancelCause;      // 打断、平滑结束或到期
    pub async fn cancelled_hard(&self) -> CancelCause; // 只有打断或到期（不可取消的工具用）
    pub fn cause(&self) -> Option<CancelCause>;
}

/// ToolManager 边界：业务失败走 Ok(Observation::Error)，基础设施故障走 Err。
/// Err 会让 waist 立即停止派发本批次剩余调用，保留已得结果，以
/// LLMComputeError::ToolRuntime 结束本次 run 交给 Runtime 处理。
/// 只有 ctx 的信号触发后才允许内联返回 Ok(Cancelled)；其它时候仍是契约违规。
trait ToolManager {
    async fn call_tool(&self, call: AiToolCall, ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError>;
}
pub struct ToolDispatchError { message: String, effect_unknown: bool }

/// 挂起记录：宿主等什么（task_id）、等到什么时候（until_ms，None = 等到 task 结束）
pub struct PendingToolCall {
    pub call: AiToolCall,                // name + args + call_id 三件套
    pub task_id: String,
    pub until_ms: Option<u64>,
}
```

工具执行委托给 `ToolManager` trait，policy gate 委托给 `PolicyEngine` trait。waist 不知道实现细节。

**工具迭代额度**（`max_tool_iterations` → `LLMContextState.tool_iterations_left`）：原生工具批次在整批派发完成后扣一次（被 `Pending` 截断的批次在回填、续派完成后才扣），带 action 的 behavior Step 在派发前扣一次；Step 内层的原生批次与外层 action 共享同一额度。无工具的最终回答、纯 report / 决策 Step、解析纠错都不扣，所以剩余额度不能用来推算推理次数。额度为 0 时推理请求的 `allow_tool_calls = false`；模型仍返回 tool call（或带 action 的 Step）⇒ `BudgetExhausted{ToolIterations}`。恢复不重复扣减。

`allow_deferred=false` 时 `Observation::Pending` 是契约违规（该调用记为结果未知、其余未执行，`Error{Internal}`）；为 true 时产生 `PendingTool`（§9.5）。`allow_deferred` 同时经 `ToolCallCtx` 传给工具：宿主不能挂起时，只拿到 task_id 的工具在调用内等待 task（最长 `MAX_IN_TOOL_WAIT_MS` = 30 分钟），到时带 task 当时的状态返回。

**运行中的 task**（`tasks.rs`，长命令 TODO §4）：waist 对工具结果只做两种机械判断——返回给 LLM，或 `Pending{task_id}` 挂起由宿主在进程外等待。task 由宿主装配的 `RunningTaskResolver`（`state` / `wait` / `cancel` / `can_resolve` / `watch` / `active`）解析，task_id 对 waist 不透明。`task_state_observation` 是 `TaskState` 到 observation 的唯一渲染，内联的 `wait_task` / `get_task_state` 与挂起后的回填读起来一致。`LLMContextDeps.tasks` 存在时，每次推理前 waist 用 `active()` 渲染一段 `<background_tasks>` 简介追加在请求末尾（不进历史，不破坏前缀缓存）；结果里带 `task_id` 的调用自动 `watch`。

### 3.4 OutputSpec / ContextOutput

```rust
pub enum OutputSpec {
    Text,
    Json { schema: Option<Value>, strict: bool },
}

pub enum ContextOutput {
    Text { content: String },
    Json { content: Value },
}
```

waist **不内置任何 scheduler-specific 复合输出类型**（见 §A.1）。Agent 的 `actions / next_behavior / set_memory` 等字段由 `LLMAgentContext` 在 lowering 时声明为 `Json { schema = BehaviorSchema }`，并在收到 `ContextOutput::Json` 后自己 deserialize（Behavior Loop 模式下这一步已经被 §6 的 `LLMResultParser` 内化）。

### 3.5 BudgetSpec / 终态 vs 挂起态在预算上的体现

```rust
pub struct BudgetSpec {
    pub max_total_tokens:      Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub max_wallclock_ms:      Option<u64>,
    pub max_cost_units:        Option<u32>,
    pub on_exhausted:          BudgetAction,        // Fail | ReturnPartial | EscalateHuman
    pub context_yield_threshold: Option<ContextThreshold>,
    /// 模型有效上下文窗口（token），由调用方 / provider adapter 提供，waist 不查询。
    pub context_window_tokens: Option<u32>,
}

pub enum ContextThreshold {
    Ratio { value: f32 },             // context_window_tokens 的比例，(0, 1]，需要窗口
    AbsoluteTokens { value: u32 },    // > 0
}
```

- **`max_total_tokens`** 是预算红线 → 触发 `BudgetExhausted`（终态，OOM kill）。它是累计花费，不能代替上下文窗口压力。
- **`context_yield_threshold`** 是预警阈值 → 待发送请求的估算 `>=` 阈值时触发 `ContextLimitReached{ApproachingWindow}`（挂起态，page fault yield 给 swap）。
- **`context_window_tokens`** 已知时，估算 + completion 预留（`model_policy.max_completion_tokens`）超过窗口的请求不发送 → `ContextLimitReached{HardLimit}`。
- 非法配置（阈值为 0、比例不在 (0,1]、Ratio 无窗口、窗口为 0、预留不小于窗口）在任何推理前以 `Error{Internal}` 结束，不静默忽略。

两者可以同时设置：前者必须 fail，后者可以被 scheduler 重整后 resume。估算口径见 §9.5。

### 3.6 ErrorPolicy 与错误模型

```rust
pub struct ErrorPolicy {
    /// 最多提供 N 次错误反馈；连续第 N+1 次失败升级为终态 Error。
    /// 计数单位是一次迭代（一次推理 response + 它的工具批次，或一个 behavior Step），
    /// 同一迭代多个工具错误只计 1；只有整个迭代无可纠正错误才清零，推理请求成功本身不清零。
    /// 0 关闭上限（只剩预算兜底，不推荐）。
    pub max_consecutive_errors: u32,   // 默认 3
}

pub enum LLMComputeError {
    Timeout, Cancelled,
    Provider { failure: Transient | Permanent | Unknown, message },
    OutputParse(msg), PolicyRejected(msg),
    ToolFailed { tool, call_id, message },
    ToolRuntime { tool, call_id, message, effect_unknown },     // 派发基础设施故障
    Checkpoint { stage: BeforeInference | OutcomeBoundary, message }, // 关键持久化失败
    SnapshotCorrupted(msg), Internal(msg),
}
// 三个正交维度，Runtime 按它们分发，不解析 message 文本：
//   source()           Provider | LlmOutput | Tool | Runtime | Snapshot | Internal
//   llm_correctable()  是否喂回 LLM 自纠正（只有 OutputParse / PolicyRejected / ToolFailed）
//   infra_retry_safe() 上层重跑是否不会重复副作用（Timeout / Provider Transient /
//                      Checkpoint / ToolRuntime{effect_unknown=false}）
```

`ErrorClass::{Recoverable, Fatal}` 由 `llm_correctable()` 穷尽推导，没有默认分支；新增错误种类必须显式决定。

| 故障 | 主要恢复责任方 | 是否反馈给 LLM | 核心循环行为 |
|---|---|---|---|
| Provider 临时故障 / 网络 / 超时 | adapter，其后由调度器决定是否再跑 | 否 | adapter 有界容错耗尽后结束本次 run（`Error{Provider{Transient}}` / `Timeout`），waist 不再隐式重推理 |
| Provider 鉴权 / 模型不存在 / 非法配置 | 配置管理 / 上层 Runtime | 否 | 直接结束本次 run（`Provider{Permanent}`），不消耗自纠正次数；`Unknown` 同样结束且不视为可安全重试 |
| LLM 输出不符合声明协议（严格 JSON 解析失败、Behavior 解析失败、超过 max_calls_per_round） | LLM 自纠正 | 是 | 失败输出留在 transcript，追加诊断，受 ErrorPolicy 与预算限制 |
| 工具参数 / 业务执行失败 / Policy 拒绝 | LLM 调整计划 | 是 | 对应 call_id 记录 observation 后再推理；传统 loop 跑完整批次，Behavior Action 首错停止并把未执行项记为 `Unresolved` |
| 工具派发基础设施故障、结果未知 | 工具 adapter / Runtime | 否 | 立即停止派发，已知结果保留、未执行项配对为 `Unresolved`，以 `Error{ToolRuntime}` 结束；快照仍可 resume，是否继续由 Runtime 决定，不自动重放 |
| 推理前 checkpoint 失败 | 持久化层 / Runtime | 否 | `CheckpointHook` / `InferenceHook` 返回 Err ⇒ 不发起推理，`Error{Checkpoint}`；状态仍是 s0，可只重试保存再 resume |
| 普通 worklog 失败 | 日志实现 | 否 | best effort，不进上下文、不计数、不改 outcome |
| Provider 以结构化错误码拒绝上下文长度（adapter 归一化为 `ProviderFailure::ContextLimit`，如 OpenAI `context_length_exceeded`） | 调度器（重整历史） | 否 | 挂起为 `ContextLimitReached{ProviderRefused}`，快照是这次推理（Round）前的状态；不凭异常文本或任意 HTTP 400 猜测 |
| 快照损坏 / ResumeFill 不匹配 / 未配对的 tool_use | Runtime / 调用方 | 否 | `resume()` 返回 `SnapshotCorrupted`，拒绝恢复 |
| 编程错误 / 状态不变量损坏 | 开发者 | 否 | `Internal`，终止本次 run |
| 主动 interrupt | 调度器 | 不作为故障 | `Interrupted` + s0 快照，不计数 |

**纪律**：
- Fatal 不可被 ErrorPolicy 改写；`infer()` 返回的任何错误都不会喂回 LLM。
- run 中被 `InferenceAbortToken` 触发的 cancelled 不走 ErrorPolicy，收敛到 `Outcome::Interrupted`；没有 interrupt 信号的 Provider `Cancelled` 是 Provider 故障。
- 可纠正错误的反馈只用 role ∈ {tool, user}：工具 / Policy 错误以 tool_result 配对到 call_id，输出协议错误以 user 消息追加；静态 system 前缀之外不再出现 system 消息。
- `Outcome::Error` 携带 `trace`，其中 `tool_trace[].status ∈ {succeeded, failed, unknown, not_executed}` 记录批次中每个调用的最终状态。
- `OutputSpec::Json.schema` 只透传给 provider；waist 只做 JSON 解析，不做 schema 校验，JSON 解析成功不等于 schema 校验成功。
- Provider retry / 退避 / fallback chain 都在 adapter 内部，**waist 自己绝不在外面再做一层 retry**；上层若要重跑，是显式决策，不通过伪造 observation 触发。

---

## 4. Outcome：六态退出

> "显式大于隐式"原则在 Outcome 设计上的硬约束：任何让 LLMContext 无法继续推进、但又不构成"失败"的情况，**都必须显式建模为挂起态**，而不是藏在 `Done` 或 `Error` 里。

一个 Outcome 结束一次 `run()` 调用：一次 `run()` 可以包含多个 Round 和 Step；Outcome 本身不表示 Step 或 Session Turn 完成，Turn 是否结束由 AgentSession 解释（libopendan 的 Turn 规则见 [readme](readme.md)）。

```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LLMContextOutcome {
    /// 终态：正常退出
    Done {
        reason: Option<String>,
        output: ContextOutput,
        usage: AiUsage,
        response: AiResponseSummary,          // 最后一次 LLM 响应原始摘要
        trace: ContextRunTrace,
        behavior_result: Option<LLMBehaviorResult>,   // Behavior Loop 产物；传统 Loop 为 None
    },
    /// 终态：异常。error.source()==Runtime 时内存快照仍有效，可 ResumeFromMidRun
    Error { error: LLMComputeError, usage: AiUsage, trace: ContextRunTrace },
    /// 终态：预算红线击穿
    BudgetExhausted { which: BudgetKind, partial: Option<ContextOutput>, usage: AiUsage },

    /// 挂起态：工具返回 Pending{task_id}，等宿主在 Session 层等 task 后回填。派发停在
    /// 该调用，同批次其后的调用留在快照里，回填后才执行；trace 是本段运行的审计。
    PendingTool {
        pending: Vec<PendingToolCall>,
        snapshot: LLMContextSnapshot,
        trace: ContextRunTrace,
    },
    /// 挂起态：待发送请求装不下 —— waist 只暴露"事实信号"，请求未发送，
    /// 具体压缩策略由 scheduler 在 resume 时通过 RewrittenHistory / RewrittenSteps 决定。
    /// accumulated 是可重写的历史：function call 为 state.accumulated；behavior 为
    /// 物化 prompt（input + step 历史 + 热 step），不含进行中 Step 的 inner transcript
    /// （由 waist 在快照中原样保留）。
    ContextLimitReached {
        which: ContextLimitKind,          // ApproachingWindow | HardLimit | ProviderRefused
        usage: AiUsage,
        accumulated: Vec<AiMessage>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
        trace: ContextRunTrace,
    },
    /// 挂起态：run 中被外部 interrupt 抢占（§8 打断）。
    /// 推理中被打断：snapshot 是这次 inference 发起前的状态，半截 assistant token / tool call
    /// 不进入 accumulated；工具执行中被打断：snapshot 已把该调用配对为 Cancelled、同批其余
    /// 配对为 Unresolved，恢复后不重跑。behavior 模式为外层快照（含进行中 Step 的 inner transcript）。
    Interrupted {
        reason: String,
        usage: AiUsage,
        snapshot: LLMContextSnapshot,
        abort: InferenceAbortTrace,
    },
    /// 挂起态：平滑结束（§8 finish）。没有推理或工具在进行，快照里每个调用都已配对
    /// （未开始的为 Cancelled{effect_unknown:false}），追加输入后 ResumeFromMidRun 即可继续。
    /// 不是 Session 的 stop；宿主用它（必要时用打断）实现 stop。
    Settled {
        reason: String,
        usage: AiUsage,
        snapshot: LLMContextSnapshot,
        trace: ContextRunTrace,
    },
}
```

### 4.1 二分对照表

|  | OS 对应 | snapshot | 可 resume |
|---|---|---|---|
| `Done` | `exit(0)` | 否 | 否 |
| `Error` | `exit(非0)` | 否 | 否 |
| `BudgetExhausted` | OOM kill / SIGKILL | 否 | 否 |
| `PendingTool` | `io_submit()` 后等待 | 是 | `ResumeFill::ToolResults` |
| `ContextLimitReached` | page fault → 等 swap | 是 | `RewrittenHistory`（function call）/ `RewrittenSteps`（behavior） |
| `Interrupted` | external interrupt | 是 | `ResumeFill::ResumeFromMidRun` |
| `Settled` | 平滑结束（SIGTERM 后干净退出） | 是 | `ResumeFill::ResumeFromMidRun` |

### 4.2 上层如何处理

| Outcome | Agent scheduler | Workflow engine |
|---|---|---|
| `Done` | 反序列化 Behavior 结果，按 `next_behavior` 切状态 | 写入 node output，进入下一节点 |
| `PendingTool` | session 进入"等事件" | workflow 挂起，pending 排到任务队列 |
| `ContextLimitReached` | 调用自家长期记忆 summarize 后 resume | 一般 fail-and-escalate，或换大窗口模型重跑 |
| `Interrupted` | 停止当前生成，保留 snapshot 待稍后 ResumeFromMidRun | 取消当前 node 执行 / 按策略重调度 |
| `Settled` | Session stop 的默认实现：Turn 结束，追加输入即可续跑 | 节点干净地停下，可按策略续跑 |
| `BudgetExhausted` | cost units 用尽 → 终止 | 走 retry / escalation / fail 分支 |
| `Error` | 走错误处理状态 | 走 error handler 节点 |

### 4.3 为什么把 ContextLimitReached 抬到挂起态

不同 scheduler 对上下文压缩的诉求**完全不同**：Agent 想 summarize-and-rewind、Workflow 想 fail-and-escalate、Eval 想 hard-truncate。任何"在 waist 里规定压缩策略"的字段都会偏向某一种。但**"接近阈值"这个事实信号是 provider-agnostic + scheduler-agnostic 的**，应在 waist 里有一席之地。waist 只暴露事实，策略留给 scheduler。

---

## 5. 外部依赖类型

waist 自己不重新定义"LLM 边界类型"，直接消费下层 provider 抽象（参考实现：`buckyos_api`）：

| 类型 | 关键字段 | 在 waist 中的用途 |
|---|---|---|
| `AiMessage` | `role`（system/user/assistant/tool）, `content: Vec<AiContent>` | `LLMContextRequest.input` / `ResumeFill::RewrittenHistory` / `accumulated` |
| `AiToolCall` | `name`, `args`, `call_id` | provider 归一化后的 tool 调用；`PendingToolCall.call` 直接持有 |
| `AiResponseSummary` | `text`, `tool_calls`, `artifacts`, `usage`, `cost`, `finish_reason`, `provider_task_ref` | `Outcome::Done.response` |
| `AiUsage` | `input_tokens`, `output_tokens`, `total_tokens`, `request_units` | 各 outcome 的 `usage` |
| `AiCost` / `AiArtifact` | — | 嵌在 `AiResponseSummary` 里，waist 不单独暴露 |

**为什么不再包一层**：零成本序列化路径；任何上层 scheduler 拿到 `Done.response` 就已经是 provider-agnostic 的归一化结构；换 provider 实现时 waist 完全不动。

---

## 6. Behavior Loop（在 waist 内一等公民）

Behavior 模式是 Agent 一侧最常见的 L4 语义，但因为它能完整覆盖在双中立性下成立的"step → step"调度协议，被作为 waist 的**可选执行模式**实现。它**不是** Agent 专属字段进入 waist —— 而是把"reply / observation / thought / action / next_behavior"这套结构化输出和分步沉淀做成一组 trait，让 Agent / Workflow / Eval 都能用。

### 6.1 模型

```
   ┌──────────── Behavior Loop（外层，run_behavior）─────────────┐
   │                                                              │
   │   step 1 ──┐                                                 │
   │   step 2 ──┤  全部沉淀到 LLMContextState.steps               │
   │   step 3 ──┘                                                 │
   │   ────────────────────────────                               │
   │   last_step（hot）：最近沉淀的一步，下一次推理 verbatim 渲染   │
   │                                                              │
   │   每个 Step：                                                │
   │     0. CheckpointHook（Step 边界，外层快照，可注入）         │
   │     1. render(history) + render(last_step) + inner transcript│
   │        → 内层 request 的 messages                            │
   │     2. run_inner_for_step：内层传统 Loop，可含多个 Round 与   │
   │        原生工具批次（InferenceHook 在每个 Round 前）         │
   │     3. parser.parse(response) → StepRecord（分配 step_index）│
   │     4. report_end 或无 action 的 next_behavior ⇒ Done           │
   │     5. 否则扣一次工具迭代，派发 actions（state.action_step）  │
   │        → 填 action_results → 沉淀为 last_step，继续          │
   │                                                              │
   └──────────────────────────────────────────────────────────────┘
```

### 6.2 关键 trait（`behavior_loop.rs`）

```rust
/// 一个 Step = 一次行为决策 + 它的 action 结果。Step 身份是 (run, meta.step_index)。
pub struct StepRecord {
    pub meta:              StepMeta,                // behavior_name / step_index / started_at_ms / ended_at_ms
    pub assistant_text:    String,
    pub assistant_message: Option<AiMessage>,       // 产生决策的 response（Step 内最后一个 Round）
    pub native_messages:   Vec<AiMessage>,          // 本 Step 内层原生调用与回执的审计记录；不含最终 assistant
    pub observation:       Option<String>,
    pub thought:           Option<String>,
    pub actions:           Vec<AiToolCall>,         // 一个 Step 可以有多个 action（<actions> 容器）
    pub next_behavior:     Option<String>,          // 见 §6.4
    pub self_report:       Option<String>,          // <report>，同时覆盖 state.last_report
    pub report_end:        bool,                    // 显式完成，缺省 false
    pub report_artifacts:  Vec<String>,
    pub report_result:     Option<Value>,
    pub messages_sent:     Vec<SendMessageRecord>,  // <sendmsg>
    pub action_results:    Vec<Observation>,        // 与 actions 按下标对齐，executor 填
    pub next_user_message: Option<AiMessage>,       // 覆盖默认的 action 结果渲染（StepResultHook / 注入）
}

impl StepRecord {
    /// 解析失败 / policy 拒绝产生的合成纠错 Step：喂回 LLM 自纠正，不是行为决策，
    /// 但同样占用一个 step_index。
    pub fn is_correction(&self) -> bool;
}

pub struct LLMBehaviorResult {
    pub do_actions:    Vec<AiToolCall>,
    pub next_behavior: Option<String>,
    pub assistant_text: String,
    pub observation:   Option<String>,
    pub thought:       Option<String>,
    pub self_report:   Option<String>,
    pub report_end:    bool,
    pub report_artifacts: Vec<String>,
    pub report_result: Option<Value>,
    pub messages_to_send: Vec<SendMessageRecord>,
}

pub trait LLMResultParser: Send + Sync {
    fn parse(&self, response: &AiResponseSummary) -> Result<LLMBehaviorResult, String>;
}

pub trait StepRenderer: Send + Sync {
    /// 一步沉淀回去 = 一对 (assistant, user)，严格角色交替
    fn render(&self, step: &StepRecord) -> (AiMessage, AiMessage);
    /// 摘要、历史输入、其它 behavior 的继承记录、当前 behavior 的完整 pair
    fn render_history(
        &self,
        steps: Vec<StepRecord>,
        current_behavior: &str,
        summaries: Vec<HistorySummaryRecord>,
        inputs: Vec<HistoryInputRecord>,
    ) -> Vec<AiMessage> { /* default */ }
}

/// Step 有 action 结果后、沉淀前调用：可覆盖下一次推理看到的 user 消息、追加
/// history_inputs，或结束本次 run。Err 一律降级为默认渲染。
#[async_trait]
pub trait StepResultHook: Send + Sync {
    async fn on_behavior_step_ob(&self, snapshot: &LLMContextSnapshot, step: &StepRecord)
        -> Result<StepResultHookOutput, String>;
}
```

Step 历史不在 waist 内压缩（原 `HistoryCompressor` 已移除）：需要时由宿主在 `ContextLimitReached` 之后用 `RewrittenSteps` 显式重写，见 [append-only history](llm_context_append_only_history.md)。

### 6.3 装配位置

Behavior Loop 通过往 `LLMContextDeps` 注入 trait 实例打开：

```rust
pub struct LLMContextDeps {
    // ... 通用依赖（llm / tools / policy / worklog / tokenizer）...
    pub inference_hook:   Option<Arc<dyn InferenceHook>>,    // 每个 Round 前（§9.2）
    pub checkpoint_hook:  Option<Arc<dyn CheckpointHook>>,   // 外层快照（§9.4）
    pub result_parser:    Option<Arc<dyn LLMResultParser>>,
    pub step_renderer:    Option<Arc<dyn StepRenderer>>,
    pub step_result_hook: Option<Arc<dyn StepResultHook>>,
}
```

- `result_parser = None` ⇒ **传统 Agent Loop**：accumulate AiMessage，直到 LLM 不再调工具。
- `result_parser = Some(_)` ⇒ **Behavior Loop**：每个 Step 跑一次内层 `run_inner`（可含多个 Round 和原生工具批次）+ parse + 派发 action + 沉淀，直到 `Done`（§6.4）。
- 内层上下文用 `into_traditional()` 得到：去掉 parser / renderer / step_result_hook / checkpoint_hook，保留 inference_hook。所以 `CheckpointHook` 只在外层 Step 边界看到外层快照；`InferenceHook` 在 Step 内每个 Round 前看到扁平化的内层快照。
- Construction-time invariant：`result_parser.is_some() ⇒ step_renderer.is_some()`（否则 `LLMContext::new` panic —— 这是 misconfiguration，不是 runtime error）。

参考实现：`XmlBehaviorParser` / `XmlStepRenderer`（XML 协议），位于 `xml_behavior.rs` / `step_record.rs`。

### 6.4 Behavior 模式下的 Outcome

- **终态 `Done.behavior_result: Some(_)`**：显式 `report_end=true` 或生效的 `next_behavior` 使本次 `run()` 返回。END / done 已移除，普通 report-only 继续，空决策纠错。结束报告与 XML 动作、sendmsg、同次原生 tool_calls、非空调度互斥；在副作用前拒绝。切换 / 等待由 Session 解释，目标 behavior 按 `switch_context` / `create_sub_context` / `fork` 构造，不复用其它 behavior 的 run 配置。返回 Done 不自动代表 Turn / Session 完成。
- **挂起态**：按 §4 走，Step 内层也可以让出。内层原生工具 deferred（`PendingTool`）、内层物化 prompt 装不下（`ContextLimitReached`）、内层推理被中断（`Interrupted`）都翻译为外层同名 Outcome，携带**外层**快照；deferred action 同样挂起为 `PendingTool`。
- **Step 未完成时的恢复状态**（都在外层 `LLMContextState`，§9.1）：
  - inner transcript：`accumulated` 中 `request.input` 之后的消息，即进行中 Step 的内层原生工具 Loop（tool_use 与已得结果），尚未折叠进 `StepRecord`。内层返回 Done 后，把该尾部转存到 `StepRecord.native_messages`，排除由 Step 自身承载的最终 assistant；解析 / 策略拒绝形成的纠错 Step 同样保留。进行中的尾部仍留在 accumulated。该字段用于原始调用与回执审计，StepRenderer 不额外渲染它，转存和写 worklog 都不发起推理、不增加 Round。
  - `tool_batch`：内层被 deferred 调用截断的原生批次（未派发的调用与 `batch_error`）。
  - `action_step`：已解析、action 尚未派发完的 Step 与它的 response；还没沉淀，`step.action_results` 是已得结果。
  - `next_step_index` / `next_action_id`：下一个待分配的编号。step_index 在解析出 Step 时分配（合成纠错 Step 也占一个），所以是分配位置，不是已完成 Step 数；`action_step` 里的 Step 已有编号但未完成。

  回填后内层续跑或续派其后的 action，不重跑已完成的工具、不重复解析或派发 action、不重复扣工具迭代。上下文检查作用在内层实际物化的 prompt 上；`ContextLimitReached.accumulated` 不含 inner transcript，`RewrittenSteps` 原样保留它。
- **`forbid_next_behavior = true`**：fork 子上下文专用，任何 `next_behavior` 字段被丢弃，确保子执行结束就终止。

---

## 7. Prompt 渲染管道（render-then-budget）

L4 lowering 时通常需要把"角色 md / behavior prompt / memory / 工具清单 / observations"等多个 section 拼成 `Vec<AiMessage>`。waist 提供一套与 scheduler / provider 都中立的渲染 + 预算 pipeline，让所有 L4 共用（`prompt_engine.rs` / `prompt_compose.rs` / `prompt_budget.rs`）。

```rust
pub struct SectionSpec {
    pub key: String,
    pub role: AiRole,
    pub template: String,        // 模板源文本（含 {{ var }} / {{ load value }} 等）
    pub priority: u8,            // 预算紧张时按 priority 决定丢弃顺序
    pub min_tokens: u32,         // 该 section 的最小保留 token 数
    pub trunc: TruncFrom,        // Head / Tail / Middle
    pub local_vars: Option<RenderVars>,
}

pub struct CompositionRequest<'a> {
    pub sections: Vec<SectionSpec>,
    pub total_budget_tokens: u32,
    pub vars: &'a RenderVars,
    pub engine: &'a PromptRenderEngine,
    pub tokenizer: &'a dyn Tokenizer,
}

pub async fn compose<L: ValueLoader + ?Sized>(
    request: CompositionRequest<'_>,
    loader: &L,
) -> Result<CompositionOutcome, CompositionError>;

pub struct CompositionOutcome {
    pub messages: Vec<AiMessage>,    // 给 LLMContextRequest.input 用
    pub dropped:  Vec<String>,
    pub render_stats: HashMap<String, RenderStats>,
    pub tokens_used: u32,
    pub tokens_remaining: u32,
}
```

两阶段：

1. **render** —— `PromptRenderEngine` 把每个 section 的模板 + vars + `ValueLoader`（异步加载远端值，如 memory query）渲染成纯文本。
2. **budget** —— `PromptBudgeter::fit` 按 `priority / min_tokens / trunc` 把渲染产物压进 `total_budget_tokens`，必要时按优先级丢 section，剩余 section 内部按 `TruncFrom` 截断。

为什么放进 `llm_context` crate：模板渲染 + token budgeting 是所有 L4 都需要做的事，做一份共用避免每个 scheduler 各写一份。它**不构成 waist 公共类型**（不出现在 `LLMContextRequest` 公开字段），只是 crate 内的 utility——L4 想用就用，不想用也可以自己拼 `Vec<AiMessage>`。

---

## 8. Interrupt 与 Finish（打断与平滑结束）

`run()` 一旦返回，当前 inference 已结束；再说"中断"已经太晚。`LLMContextInterruptHandle` 是一条独立的 preemptive 控制面，允许 scheduler 在 `run()` 尚未返回时作用于进行中的推理**和工具调用**。它提供两种结束方式（术语见长命令 TODO §1）：

- **打断**（`interrupt`，Ctrl-C 语义）：进行中的推理立即中止；正在执行的工具收到 `ToolCallCtx.abort`，能取消就取消，不能取消就放弃等待。结束于 `Interrupted`。
- **平滑结束**（`finish`）：不再发起新的推理和工具调用；推理中则等它完成、其中的工具调用不派发而配对为 `Cancelled{effect_unknown:false}`；工具执行中，支持取消的工具取消，不支持的最多等 `finish_grace_ms`（默认 30s）后转为打断。结束于 `Settled`，所有调用都已配对，追加输入即可继续。
- Session 的 **stop** 不是 waist 概念：宿主用平滑结束实现，必要时改用打断。

```rust
#[derive(Clone)]
pub struct LLMContextInterruptHandle { inner: Arc<InferenceAbortState> }

impl LLMContextInterruptHandle {
    pub fn interrupt(&self, reason: impl Into<String>) -> bool;
    pub fn finish(&self, reason: impl Into<String>) -> bool;
    pub fn standalone() -> Self;                 // 不挂在 context 上，直接驱动 ToolManager 时用
    pub fn token(&self) -> InferenceAbortToken;
}

#[derive(Clone)]
pub struct InferenceAbortToken { inner: Arc<InferenceAbortState> }

impl InferenceAbortToken {
    pub fn is_aborted(&self) -> bool;
    pub fn is_finishing(&self) -> bool;
    pub async fn cancelled(&self);               // 打断
    pub async fn stopping(&self);                // 打断或平滑结束
    pub fn reason(&self) -> Option<String>;
}

pub struct LlmInferenceRequest {
    pub messages: Vec<AiMessage>,
    pub model_alias: String,
    pub fallbacks: Vec<String>,
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    pub force_json: bool,
    pub json_schema: Option<Value>,
    pub provider_options: Option<Value>,
    pub tool_specs: Vec<ToolSpecLite>,
    pub allow_tool_calls: bool,
    pub abort: InferenceAbortToken,
}
```

**执行纪律**：

1. `LLMContext::new` 创建共享 `InferenceAbortState`；`interrupt_handle()` 派发外部句柄。
2. 每次 inference（Round）前先回调 `InferenceHook`（§9.2；function call 模式在它之前还有 `CheckpointHook`，§9.4），再构造携带 `InferenceAbortToken` 的 `LlmInferenceRequest`。behavior 模式的内层上下文共享外层的 abort state，一个 `interrupt_handle()` 能中断 Step 内任一 Round。
3. waist 把 `provider.infer()` future 和 `abort_token.cancelled()` 跑 `tokio::select!`，即便 adapter 完全忽略 abort，scheduler 线程也能立刻释放。
4. provider adapter 应把 abort 映射到底层 HTTP / SDK cancel；不支持远端 cancel 时丢弃 late response（仍能尽早释放本地）。
5. 收到 cancelled 后返回 `Outcome::Interrupted`，snapshot 是**被中断的这次 inference 发起前**的状态——半截 token / 半截 tool call 不进入 accumulated；behavior 模式是外层快照，进行中 Step 的 inner transcript 保留在其中。
6. resume 时用 `ResumeFill::ResumeFromMidRun`：context 从这次 inference 前重新推进（behavior 模式继续同一个 Step）。
7. **工具执行期间**：每次 `call_tool` 带 `ToolCallCtx { abort, deadline_ms, allow_deferred }`。工具是否支持取消由实现声明（`AgentTool::cancellable`，默认否）：支持的工具自己监听 `ctx.cancelled()`、处理当前命令后返回 `Cancelled{effect_unknown:false}`（文本写明命令状态与"可能已有部分副作用"）；不支持的工具由 ToolManager 在 `cancelled_hard()` 触发时放弃等待，返回 `Cancelled{effect_unknown:true}`。waist 按 `ctx.cause()` 收尾：打断 → 同批其余 `Unresolved{effect_unknown:false}`，`Interrupted`（快照含已配对的 Cancelled，恢复后不重跑）；到期 → `BudgetExhausted{Wallclock}`；平滑结束 → 其余 `Cancelled{effect_unknown:false}`，`Settled`。behavior 模式按"第一个非成功结果停止其余 action"处理，Step 被沉淀后再返回。
8. 平滑结束期间 waist 自己计时：工具在 `finish_grace_ms` 内没有返回就把 finishing 升级为 abort（§7 的放弃等待路径）。

`Interrupted` / `Settled` 与 cooperative yield 的边界：

- `PendingTool` / `ContextLimitReached` —— 在推理边界让出 CPU（工具派发中、请求发送前或 provider 拒绝后）。
- `Interrupted` —— scheduler 在 inference 或工具调用过程中抢占。
- `Settled` —— scheduler 要求平滑结束，waist 在下一个干净的边界停下。
- 等待用户输入 —— L4 / session 状态，不是 waist 概念。

---

## 9. Snapshot 与崩溃恢复

### 9.1 snapshot 不变量

```rust
pub struct LLMContextSnapshot {
    pub request: LLMContextRequest,         // 不可变"代码段"
    pub state:   LLMContextState,           // 可变"寄存器 + 栈"
}

pub struct LLMContextState {
    /// function call：完整 transcript；behavior：request.input + 进行中 Step 的
    /// inner transcript（§6.4）
    pub accumulated: Vec<AiMessage>,
    pub usage:       AiUsage,
    /// 剩余工具迭代（ToolPolicy.max_tool_iterations，§3.3），不是剩余推理次数
    pub tool_iterations_left: u32,
    pub started_at_ms: u64,
    pub cost_units:    u32,
    pub consecutive_errors: u32,
    /// 挂起原因：PendingTool { pending, at_ms } | ContextLimit { which, estimated_tokens, at_ms }
    pub suspended: Option<Suspension>,
    /// 被 deferred 调用截断的原生工具批次（function call，或 behavior 进行中 Step 的
    /// 内层）：{ remaining, batch_error }
    pub tool_batch: Option<ToolBatch>,
    /// behavior：已解析、action 尚未派发完的 Step（未沉淀）：{ step, response }
    pub action_step: Option<ActionStep>,
    pub llm_task_ids: Vec<String>,
    /// Behavior Loop：沉淀的历史 steps；run 内只追加，重写只经 RewrittenSteps
    pub steps: Vec<StepRecord>,
    pub history_summaries: Vec<HistorySummaryRecord>,
    pub history_inputs:    Vec<HistoryInputRecord>,
    /// Behavior Loop：最近沉淀的一步（hot），下一次推理 verbatim render
    pub last_step: Option<StepRecord>,
    pub last_report: Option<String>,
    pub report_end: bool,                // 最后报告的显式结束意图
    /// 下一个待分配的 step_index / action call_id：分配位置，不是已完成数量
    pub next_step_index: u32,
    pub next_action_id:  u32,
    pub snapshot_version: u32,           // §9.5
    pub host: Option<Value>,             // 宿主元数据，§9.4
}
```

**硬约束**（不是建议）：

- **自包含**：给定 snapshot S 在节点 A 上产生，节点 B 只要提供等价 `LLMContextDeps`，就必须能成功 `resume(S, fill, deps)`。
- **跨节点可序列化**：建议 < 32KB；超出时调用方走外部存储 + 在 snapshot 里只放引用 ID。
- **不持有 effect-side 真实世界状态**：snapshot 只持有逻辑层产物（call_id / observation / accumulated / step / usage），**绝不**持有 tmux 句柄、fd、network connection、容器 PID。
- **input 已展开**：跨节点 resume 不依赖任何模板环境。模板展开发生在 L4。

> **工程提醒**：如果你想往 snapshot 里塞"句柄""指针""长生命态引用"，停下来——那些东西属于 `LLMContextDeps`（resume 时重新提供），不属于 snapshot。

### 9.2 InferenceHook（可选 deps 扩展点）

```rust
pub trait InferenceHook: Send + Sync {
    /// 每次 LLM inference（每个 Round）之前同步回调。snapshot 是 LLMContext 的完整冻结。
    /// 必须 fast / 不可 panic / 不可修改 snapshot。
    /// Err(reason) ⇒ waist 不发起推理，以 Error{Checkpoint{BeforeInference}} 结束本次 run。
    fn before_inference(&self, snapshot: &LLMContextSnapshot) -> Result<(), String>;
}
// LLMContextDeps.inference_hook / with_inference_hook
```

- 它是推理（Round）边界的钩子，不在 AgentSession Turn 开始或结束时调用。
- 不注入也合法（纯内存运行）。一旦注入即承担关键 checkpoint：写失败不能继续 infer；此时 waist 状态仍是交给 hook 的那份 s0，Runtime 只需重试保存并 `ResumeFromMidRun`，不会重放任何副作用。
- behavior 模式下它跟随内层上下文，在 Step 内的每个 Round 前调用，看到的是扁平化的内层快照（`request.input` 是物化后的 prompt，不含 StepRecord 流），不能用来恢复外层 behavior 上下文；需要外层（Step 级）快照的宿主用 `CheckpointHook`（§9.4）。
- 只承担观测职责的 hook 应自己吞错并返回 Ok。
- snapshot 落到哪 / 加密 / 压缩 / 归档全部是 effect 层私事（§A.4）。
- 各 L4 调用频率诉求不同：OneShot 每次都落、workflow 节点采样、agent 长会话按 Round 数采样——scheduler 政策，waist 不裁决。

### 9.3 崩溃恢复流程（L4 持久化层用）

```
[Process A]                                  [Process B (after crash)]

ctx = LLMContext::new(req, deps)
s0 = ctx.snapshot();  sink.persist(s0)       // 启动前落盘
loop {
    // CheckpointHook / InferenceHook 在 inference 前再落一份
    outcome = ctx.run().await
    s1 = ctx.snapshot();  sink.persist(s1)   // outcome 边界落盘
    match outcome { ... }
}
           |
           ▼ 进程崩溃
                                            s = sink.load_latest()
                                            // snapshot 不在挂起态 → ResumeFromMidRun
                                            ctx = LLMContext::resume(
                                                s, ResumeFill::ResumeFromMidRun, deps
                                            )?
                                            loop { outcome = ctx.run().await; ... }
```

**纪律**：

- L4 必须能区分"崩在挂起态"与"崩在运行中"：前者用 `ToolResults` / `RewrittenHistory` / `RewrittenSteps`，后者用 `ResumeFromMidRun`。waist 在 resume 里做一致性校验拦截误用。
- "崩在挂起态 + 无外部 fill" 不能用 `ResumeFromMidRun` 兜底——会返回 `SnapshotCorrupted`；accumulated 里存在未配对的 `tool_use`（尚未派发的批次剩余调用除外）同样被拒绝。
- 挂起快照由宿主在 outcome 边界持久化：PendingTool 必须先落盘再把调用交给异步执行器；回填后应先持久化 `ctx.snapshot()` 再 `run()`，因为续派的调用在下一个 checkpoint 之前执行。关键保存失败以 `Error{Checkpoint}` 结束，不会被挂起结果掩盖。
- L4 要区分"算出 outcome"和"outcome 已提交"：边界写入失败时保留已算出的 outcome 与内存快照，只重试保存，不重新 `run()`。
- checkpoint 不能独自保证不重复扣费、不重复执行副作用：hook 写盘之后、工具执行完成之后到下一个 checkpoint 之间崩溃，恢复会重跑该段 inference / 工具调用。**ToolManager / provider 的幂等性是 effect 层私事**。Behavior 模式下 InferenceHook 收到的是内层传统上下文的快照（不含 StepRecord 流）；外层 Step 状态由 `CheckpointHook` 在 Step 边界、由 L4 在 outcome 边界提交，Step 内的挂起由外层快照（inner transcript / `tool_batch` / `action_step`）承载。

### 9.4 CheckpointHook、注入与宿主元数据（2026-09-29，Agent Session SDK §8.7 X2/X3/X4/X5）

以下均为可选能力，未使用时行为不变。

```rust
#[async_trait]
pub trait CheckpointHook: Send + Sync {
    /// 以**外层**快照调用：function call 模式在每次推理（Round）之前（首次推理、
    /// 每个工具批次的结果之后）；behavior 模式只在外层 Step 边界（每个 Step 的
    /// do-action 之后、下一个 Step 的内层推理之前），不在 Step 内层的推理前调用，
    /// 也不会交给内层逐步上下文（into_traditional 会去掉它）。Step 内层原生批次
    /// 被 deferred 截断、回填后续跑时不是 Step 边界，不调用。
    /// Ok(None)：快照已持久化，可以推理；Ok(Some(injection))：waist 应用注入后
    /// 再调用一次，使注入后的状态先落盘（每个边界最多 4 次注入）；
    /// Err：以 Error{Checkpoint{BeforeInference}} 结束，内存快照仍可恢复。
    async fn before_inference(&self, snapshot: &LLMContextSnapshot)
        -> Result<Option<Injection>, String>;
}
pub struct Injection { pub messages: Vec<AiMessage>, pub host: Option<Value> }
```

- `LLMContext::inject(injection) -> InjectionPosition`：function call 追加到 `accumulated`；behavior 模式并入热 step（或 `Done` 之后当前 behavior 的最后一个 step）的 `next_user_message`（接在默认的动作结果渲染之后），否则追加到 `request.input`。宿主用同一规则预测位置，把位置写进 receipt。
- `LLMContextState.host: Option<Value>`：宿主元数据（如 libOpenDAN 的输入 receipt，键为宿主名），随每个快照、resume 和其它执行者的续跑原样保留，waist 不解释。
- `LLMContextState.snapshot_version`：当前为 5（§9.5）；`resume` 只接受等于 `SNAPSHOT_FORMAT_VERSION` 的版本，更旧或更新的都返回 `SnapshotCorrupted`，调用方据此保留现场而不是新建上下文。
- behavior 模式的 `Interrupted` 返回**外层**快照（含进行中 Step 的 inner transcript），可直接 `ResumeFromMidRun`。
- 与 `InferenceHook`（§9.2）的分工：两者都在推理前、都可以以 `Error{Checkpoint{BeforeInference}}` 拦住推理；function call 模式两者都在每个 Round 前（先 CheckpointHook 再 InferenceHook），behavior 模式 CheckpointHook 只在外层 Step 边界看外层快照、可注入，InferenceHook 在 Step 内每个 Round 前看扁平化的内层快照、不能注入。
- `XmlStepRenderer::without_timestamps()`：历史记录不渲染 `started_at_ms` / `ended_at_ms`，同样的 steps 渲染出相同字节（X8）；宿主装配的 xllm run 使用它，默认渲染不变。
- 在途动作与执行跟踪（X6）属于 effect 层，见 `agent_tool::exec_tracking` 与 [Lease Protocol](../opendan/protocol/Lease%20Protocol.md) §5。

### 9.5 挂起与恢复（X7，2026-09-30）

- **快照版本 5**：版本 2 用 `suspended` / `tool_batch` / `action_step` 取代了 `pending_tool_calls`（§9.1）；版本 3（2026-10-01 Round / Step / Turn 术语统一）把工具预算改名为 `tool_iterations_left` 与 `ToolBatch.batch_error`；版本 4（2026-10-02 长命令）把挂起记录改为 `PendingToolCall {call, task_id, until_ms}`（取代 `eta_ms` / `tool_result`），并增加 `Observation::Pending.task_id`、`Observation::Cancelled.effect_unknown` 与 `shell` action 标签。版本 5（2026-10-10 H4）增加 report_end、report_artifacts、report_result 并废弃 END / 隐式 done；`resume` 只接受版本 5，更旧或更新的版本都以 `SnapshotCorrupted` 拒绝，不做迁移。
- **PendingTool**：工具串行派发，遇到 `Pending` 即停止并挂起；已执行（结果已在 transcript / step 中）、等待中（`suspended.pending`）、尚未执行（`tool_batch.remaining` 或 step 其后的 action）三类可区分，trace 状态分别为 succeeded/failed、`pending`、未记录。回填后本批次余下调用继续派发，整批完成时才扣一次工具迭代、最多计一次错误；带 action 的 Step 在派发前已扣过，回填后不再扣。usage、tool_iterations_left、错误计数、step / action / call 编号都不重置。behavior action 在第一个非成功结果（业务错误、回填的 Cancelled / Unresolved）后停止其余 action。Step 内层原生工具的挂起见 §6.4。
- **ContextLimitReached**：只在推理边界检查（首次推理、工具结果追加后、checkpoint hook 注入后、behavior 内层物化 prompt 后），顺序在 hook 与 abort 检查之后，因此 checkpoint 失败与主动 interrupt 优先于挂起；token / wallclock 预算终态也优先。估算：每条消息 4 token 开销 + 各文本部分（文本、tool_use 参数、tool_result、thinking、provider state）经 `Tokenizer` 计数，图片 / 文档等非文本部分每个按 1024 计，另计工具描述（允许调用工具时）与输出 schema；多模态 prompt 可能被低估。估算值只用于检查，不与 Provider usage 混用。重写后仍装不下时再次让出、不推理，压缩次数与失败策略由宿主决定。
- **宿主接入**（2026-09-30）：xllm 以 `.llm_context` 的 `context_window` 提供窗口（设置后阈值为 0.75），压缩后以 `RewrittenHistory` / `RewrittenSteps` 续跑、每个 run 最多 3 次，接手上下文上限挂起的快照时先压缩，等待 deferred 工具的快照不接手；OpenAI 兼容 adapter 把 `context_length_exceeded` 归一化为 `ProviderFailure::ContextLimit`。libOpenDAN（当前实现）的中途重写见 [Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md) §7：先把 run 尚未写入的历史 flush 进 Session worklog，再以 system + 重建的会话历史作为新 input 恢复（behavior 把 steps 全部折叠进 input，编号继续，进行中 Step 的 inner transcript 由 waist 保留），开始新的 history epoch；重写不改变当前 Turn，细节见 [append-only history](llm_context_append_only_history.md)。xllm 与 libOpenDAN 都使用 `allow_deferred=false`，PendingTool 路径尚无宿主接入。旧 opendan Runtime（`src/frame/opendan` 的 `AgentSession`）未配置阈值，按新抽象接入待下一阶段 opendan 重构。

---

## 10. 主循环骨架

```
LLMContext::new(req, deps)
  └─> run().await                         // 一次 run() 调用 → 一个 Outcome
        ├─> emit(LLMStarted)              // 每次 run() 一次，不能用来数 Round
        ├─> if behavior_mode:
        │     run_behavior()                  // §6，下面
        │   else:
        │     run_inner()                     // 下面
        ├─> emit(LLMFinished)
        └─> return outcome

run():
  suspended? → Error{Internal}（先 resume）；context limits invalid? → Error{Internal}

run_inner():                              // function call 模式；也是 behavior Step 的内层
  loop:
    ├─> tool_batch in progress? → dispatch its remaining calls (below), continue
    ├─> check wallclock budget → BudgetExhausted?
    ├─> checkpoint_hook (outer snapshot, may inject) → Err ⇒ Error{Checkpoint}   // 内层没有
    ├─> finishing (and not aborted)? → Settled(snapshot)        // nothing in flight
    ├─> s0 = snapshot()                                    // for InferenceHook / Interrupted
    ├─> inference_hook.before_inference(s0)? → Err ⇒ Error{Checkpoint}, no inference   // §9.2
    ├─> if abort.is_aborted(): return Interrupted(s0)
    ├─> background env = render(tasks.active())            // appended to the request only
    ├─> estimate(request) vs window / threshold → ContextLimitReached(s0), not sent  // §9.5
    ├─> tokio::select!:                                    // 一个 Round
    │     - cancelled() → Interrupted(s0)
    │     - llm.infer(req) → response
    │         ├─ Err(Provider{ContextLimit}) → ContextLimitReached{ProviderRefused}(s0)
    │         ├─ any other Err (adapter tolerance exhausted) → Error{err}, never fed back
    │         └─ ok → continue
    ├─> account usage; check token budget → BudgetExhausted?
    ├─> if no tool_calls or ToolMode::None:
    │     → Done; strict JSON parse failure ⇒ push output + diagnostic, bump, next inference
    ├─> tool_iterations_left == 0 → BudgetExhausted(ToolIterations)
    ├─> too many calls (> max_calls_per_round) / policy reject
    │     ⇒ push assistant msg + error tool_result per call, bump, next inference
    ├─> push assistant_tool_call message; tool_batch = { remaining: calls, batch_error: None }
    ├─> dispatch tool_batch, for each call:
    │     finishing? → answer rest as Cancelled{effect_unknown:false}, Settled
    │     ctx = ToolCallCtx { abort, deadline_ms, allow_deferred }
    │     match tools.call_tool(call, ctx)  (finish grace elapsed ⇒ escalate to abort):
    │       Err(dispatch) → record Unknown/NotExecuted, answer rest as Unresolved,
    │                       Error{ToolRuntime}
    │       Ok(Success)   → push tool message; watch tool_result.task_id
    │       Ok(Error)     → push tool message, remember batch_error (batch continues)
    │       Ok(Pending{task_id}) + allow_deferred → PendingTool (rest of the batch kept in tool_batch)
    │       Ok(Pending) without task_id / without allow_deferred, Ok(Unresolved),
    │         Ok(Cancelled) without ctx.cause()   → contract violation ⇒ Internal
    │       Ok(Cancelled) with cause → push tool message, record Cancelled;
    │           Interrupted ⇒ rest Unresolved, Interrupted(snapshot)
    │           Deadline    ⇒ rest Unresolved, BudgetExhausted{Wallclock}
    │           Finishing   ⇒ rest Cancelled, Settled(snapshot)
    ├─> batch complete: batch_error? bump once per batch : reset consecutive_errors
    ├─> tool_iterations_left -= 1
    └─> next inference (Round)

run_behavior():
  loop:
    ├─> action_step in progress? → dispatch its remaining actions (below), continue
    ├─> check wallclock budget → BudgetExhausted?
    ├─> no inner tool_batch? → checkpoint_hook (outer snapshot, Step boundary, may inject)
    ├─> run_inner_for_step():
    │     inner = LLMContext(materialized prompt + inner transcript, deps.into_traditional())
    │     seeded with usage / errors / tool_iterations_left / tool_batch, shares abort state
    │     inner.run_inner()                                // 一个或多个 Round
    │       Done → clear inner transcript, take response
    │       PendingTool / ContextLimitReached / Interrupted / Settled
    │            → keep inner transcript + tool_batch in the outer state,
    │              return the same outcome with the OUTER snapshot
    │       Error / BudgetExhausted → return as the outer outcome
    ├─> parser.parse(response); failure ⇒ correction step (takes a step_index), sediment, bump, continue
    ├─> step = prepare_step(result)                        // allocates step_index / action call_ids
    ├─> step has actions && tool_iterations_left == 0 → BudgetExhausted(ToolIterations)
    ├─> validate report and action policy before side effects (reject ⇒ correction step, continue)
    ├─> final report + actions/sendmsg/native calls/scheduling → rejected; non-final action jump targets → dropped
    ├─> actions? → tool_iterations_left -= 1               // charged before dispatch
    ├─> action_step = { step, response }
    ├─> dispatch actions in order; first non-success stops the rest (Unresolved);
    │     Pending + allow_deferred → PendingTool (step stays in action_step)
    └─> complete_step: next_behavior in force / nothing happened / hook skip → Done
                       else sediment as last_step (bump on action error), next Step
```

---

## 11. 模块布局

```
src/frame/llm_context/src/
│   ├── lib.rs                 // pub use
│   │
│   ├── request.rs             // LLMContextRequest / ModelPolicy / ToolPolicy /
│   │                          //   OutputSpec / BudgetSpec / HumanPolicy /
│   │                          //   ErrorPolicy / ContextOwnerRef
│   ├── outcome.rs             // LLMContextOutcome / ContextOutput /
│   │                          //   ContextRunTrace / ResumeFill / BudgetKind
│   ├── observation.rs         // Observation / PendingToolCall / ToolExecRecord
│   ├── error.rs               // LLMComputeError
│   ├── interrupt.rs           // LLMContextInterruptHandle / InferenceAbortToken /
│   │                          //   InferenceAbortTrace
│   ├── deps.rs                // LLMContextDeps + LlmClient / ToolManager /
│   │                          //   PolicyEngine / WorklogSink / Tokenizer /
│   │                          //   InferenceHook / CheckpointHook
│   ├── state.rs               // LLMContextState / LLMContextSnapshot /
│   │                          //   Suspension / ToolBatch / ActionStep
│   ├── suspension.rs          // ResumeFill 校验与应用 / inner_transcript_of
│   ├── context_window.rs      // ContextLimits / 请求 token 估算
│   ├── snapshot_overrides.rs  // RequestOverrides / rebuild_with_inherit / build_fresh
│   │
│   ├── context_loop.rs        // LLMContext::{new, run, resume, snapshot,
│   │                          //   interrupt_handle}; run_inner + run_behavior
│   ├── behavior_loop.rs       // StepRecord / LLMBehaviorResult /
│   │                          //   LLMResultParser / StepRenderer / StepResultHook
│   ├── step_record.rs         // XmlStepRenderer 参考实现
│   ├── xml_behavior.rs        // XmlBehaviorParser 参考实现
│   │
│   ├── prompt_engine.rs       // PromptRenderEngine + ValueLoader
│   ├── prompt_compose.rs      // compose() —— SectionSpec → Vec<AiMessage>
│   ├── prompt_budget.rs       // PromptBudgeter::fit
│   │
│   └── tests.rs
```

`behavior_loop` / `xml_behavior` / `step_record` 是 waist 内置的**通用执行模式**（双中立性下成立），不是 Agent 专属字段。`prompt_*` 三件是 L4 共用 utility，不出现在公开 waist 类型签名上。

---

## 12. 姊妹文档

| 文档 | 角色 |
|---|---|
| `LLMAgentContext 设计.md`（待写） | L4 scheduler-facing 层，Agent 一侧。承接所有角色 / 行为配置可见的字段，lowering 到本文档定义的 LLMContext。|
| `LLMWorkflowContext 设计.md`（待写） | L4 scheduler-facing 层，Workflow 一侧。承接 service endpoint / 上下游引用 / on_* 分支等。|

---

## 13. 一句话总结

> **LLMContext 是 LLM 执行的"进程上下文"：一次有界、可 cooperative yield、可 inference interrupt、可 resume、可计费、可审计的执行体。它填补 `llm.complete`（太低阶）与 `agent.sendMsg`（太重型）之间的空缺，让 Agent / Workflow / Shell / Hook / Eval 共用同一套进程语义。**

---

## Appendix A: Non-Goals（永久边界）

这些 **永远不进 waist**，因为它们会破坏中立性。任何要塞进来的提议，都应被退回到上面（scheduler 层）或下面（provider / effect 实现层）。本清单**只增不减**：每次 PR 决议拒绝后，补一条进来，让后人不必重新讨论。

### A.1 Scheduler-specific

- `next_behavior` / 行为切换字段作为 `Outcome` 一级字段 —— 已经通过 `OutputSpec::Json` + Behavior Loop 的 `behavior_result` 表达，不再追加新 outcome 变体。
- workflow node 的 retry / fallback 策略；hook trigger 的事件元数据；chat session 的 typing indicator；multi-agent 的 turn-taking；sub-agent 派生层级；优先级 / 抢占 / 公平性策略 —— 各自归属对应 scheduler。
- **scheduler-facing 语义字段**：任何"DSL 用户可见 / 配置文件可写"的字段都属于 L4 `LLM*Context`，不进 waist（service endpoint 引用、`${prev_node.output.x}` 上游引用、on_budget_exhausted 分支、角色描述、行为状态机配置、hook trigger debounce、容器 / session 句柄）。判定方法：**如果一个字段在 DSL/配置文件里被人直接写出来，它一定属于 L4。**
- **运行期动态修改 tool list**：一旦 `LLMContext::new` 完成，工具集合在生命周期内不变。换工具集 = 销毁当前 LLMContext + 新建一个。会破坏 snapshot 可重放性。

### A.2 Provider-specific

- 模型计费 / billing；provider 专属参数（anthropic `cache_control` / openai `seed` / gemini `safety_settings`）—— 通过 `model_policy.provider_options` 透传，waist 不解释。
- 模型能力探测（context window 大小 / 是否支持 vision / tool）—— provider adapter 内部决定。
- token 分价规则（input vs output、cached vs uncached）—— waist 只暴露 `AiUsage` 的归一化计量字段 + `AiResponseSummary.cost`。
- streaming 协议细节（SSE / chunked / batch）—— provider 适配层处理；waist 一次推理对外是原子的。
- **function call 作为 loop 强制协议**：拒绝。各家 wire format 不同，本地模型常无原生支持。归一化到 `AiToolCall` 在 provider adapter 内部完成，waist 看见的是 §1.3 的 intent/effect/observation。
- **Provider 层 retry / 退避 / jitter / 熔断 / 路由**：拒绝。属于 adapter 内部容错层；waist 看到的是兜底失败后的最终错误，自己绝不再做一层 retry。

### A.3 Container / 长生命态

- session memory / 长期记忆 —— L4 lowering 时展开成 `AiMessage` 注入 `input`，waist 不持任何长期记忆接口。
- workspace 路径 / 文件挂载；agent 身份 / 密钥；持久事件流的存储位置 / 索引策略；sub-agent / sub-context 注册表 / 生命周期；跨 LLMContext 的对话历史拼接 —— 容器编排关心。
- **执行环境绑定**（机器 / 容器 / 远程 session 句柄）—— LLMContext 通过 ToolManager 间接访问，不持任何句柄。
- **"特定 tool 抬到 waist"**（bash / browser / fs 等）—— 拒绝。waist 没有具名 tool 概念，每种 tool 都是 ToolManager 内部的实现。

### A.4 Effect-side 持久化与执行策略

- snapshot 存储介质 / 加密 / 跨节点复制策略 —— `SnapshotStore` 接口实现细节（L4 自己定义）。
- worklog sink 具体实现 / tool 调用录像 / replay / pending tool 任务队列后端 / tool 并发限流 / 熔断 —— effect 实现层私事。
- provider-specific cancel 协议（HTTP abort / SDK cancel / stream close）—— waist 只规定 `InferenceAbortToken` 语义，映射属于 adapter。
- **LLMContext 承载方式**（in-process lib / thunk / 跨设备 RPC）—— scheduler 部署选择，不是 waist 属性。三种共享 100% 执行语义（§2.1）。
- **上下文压缩策略**（summarize / sliding window / hierarchical recall / drop-oldest / 换模型）—— 拒绝进 waist。waist 只暴露 `ContextLimitReached` 这个事实信号，策略属于 scheduler 在 resume 时通过 `RewrittenHistory` / `RewrittenSteps` 提供。Behavior Loop 的 step 历史压缩同理：waist 内没有自动压缩入口（原 `HistoryCompressor` 已移除），由 scheduler 显式重写。
- **错误归一化的 wire format**：错误 message 字段结构、是否带 stack trace / hint、人类可读 prompt 措辞，都是 effect 层与对应 L4 的协议。waist 只规定 Recoverable 错误必须以合法 `AiMessage` 形态（role ∈ {tool, user}）进入 accumulated。
- **ToolManager / 工具实现内部的 retry**：单个 tool 的重试（HTTP 5xx 重发等）属于 ToolManager 私事，waist 把每次 `call_tool` 的最终结果当一次 `Observation`。
- **RPC 服务接口 tool 化策略 / 系统状态路径化（read_file 抽象）**：ToolManager 把后端服务暴露给 LLM 的协议，与 waist 解耦。

### A.5 不解决的更大问题（由其他文档负责）

- 长生命会话 / 容器编排 —— 各 scheduler 的运行时文档。
- Workflow DSL 与 DAG runtime —— workflow engine 自己的文档。
- LLM provider 统一封装 —— 下层 provider 抽象（参考实现：`buckyos_api` / aicc）。
- 长期记忆存储与压缩算法 —— 各 scheduler 的 memory 设计。
- Prompt / 模板编译的具体策略 —— 各 L4 `LLM*Context` 的 prompt 编译器（可以复用本 crate 的 `prompt_*` utility，也可以自己写）。
- Agent 的角色 / 行为 / 状态机定义 —— 各 Agent 框架自己的运行时文档。

### 使用此清单

每次有人提议向 LLMContext 添加新字段或新方法：

1. **先查 A**：是不是已经被显式列为 Non-Goal？是的话直接拒绝并指向已有条目。
2. **过双中立性测试**：scheduler 中立？provider 中立？任何一项不通过即拒绝。
3. **过完两个测试且不在 Non-Goals 里**，仍要在 PR 描述里说明 *"为什么必须进 waist 而不是上下游某层"*。
4. **被拒绝的提议补进 A 对应小节**，标注 PR 链接和拒绝理由。
5. **同意进入 waist 的字段，必须同步更新 §3 / §4 / §5 / §11**，并在 changelog 里登记 waist 版本。一旦进入，移除等同于 breaking change。

> 这套流程不是为了"难"，是为了让"瘦"成为默认状态。瘦腰原语的失败模式从来不是被一次大改打破的，而是被一百个"加一个小字段没关系吧"撑胖的。

---

## Appendix B: 参考实现 —— OpenDAN / BuckyOS（informative）

本附录是**资料性**的，主体设计不依赖本附录内容。其它工程语境实现 waist 时，可以把本附录当作一份参考样本。

### B.1 边界类型来源

§5 列出的边界类型在参考实现里由 `buckyos_api` crate 提供：

| waist 引用名 | 参考实现路径 |
|---|---|
| `AiMessage` / `AiContent` / `AiRole` | `buckyos_api::{AiMessage, AiContent, AiRole}` |
| `AiToolCall` / `AiToolResultContent` | `buckyos_api::{AiToolCall, AiToolResultContent}` |
| `AiResponseSummary` / `AiUsage` / `AiCost` / `AiArtifact` | `buckyos_api::*` |
| Provider 内部 request 类型（不进 waist） | `buckyos_api::AiMethodRequest`，由 aicc 路由层使用 |

aicc 是 BuckyOS 统一 LLM 调用 / 路由层（`src/frame/aicc`）。waist 不感知 aicc 的存在；参考实现的 `LLMContextDeps.llm` 通常是包了 `AiccClient` 的 adapter。

### B.2 OpenDAN behavior 拆解的映射

LLMContext 起点是把 OpenDAN 既有 agent 主循环里的"一次智能执行"拆出来。被替换的 OpenDAN 模块（`src/frame/opendan`）：

| OpenDAN 概念 | 在 LLMContext 里的归属 |
|---|---|
| `LLMBehavior::run_step(input)` | 被 `LLMContext::run`（Behavior 模式）取代 |
| `LLMBehaviorDeps` | 被 `LLMContextDeps` 取代 |
| `BehaviorExecInput`（含 `role_md / self_md / behavior_prompt / input_prompt`） | 在 L4 prompt compiler 里通过 §7 `compose()` 展开成 `AiMessage` 后灌入 `LLMContextRequest.input` |
| `BehaviorLLMResult`（`reply / actions / next_behavior / set_memory / new_work_session / shell_commands`） | 拆成两部分：通用结构（`do_actions / next_behavior / observation / thought / assistant_text`）进 waist 的 `LLMBehaviorResult`；Agent 专属字段（`set_memory / new_work_session / shell_commands`）通过 `OutputSpec::Json` schema 由 `LLMAgentContext` 自己 deserialize |
| `TokenUsage`（opendan 自定义） | 删除，统一用 `AiUsage` |
| `LLMTrackingInfo` | 拆分：provider 侧 → `AiResponseSummary`；waist 侧 → `ContextRunTrace` |
| `AgentSession` | 仍是 Agent 一侧长上下文持有者；它给 L4 提供 prompt sources，不出现在 waist 公开字段 |
| `AgentToolManager` | 实现 waist 的 `ToolManager` trait |
| `PromptBuilder` | 上提到 L4：调用 `prompt_compose::compose` 把模板 + vars + loader 渲染并预算 |

**核心定位**：在 OpenDAN 语境下，LLMContext 是对 `LLMBehavior::run_step_inner` 那段循环的**重新切片与重新封装**，加上 owner 抽象、cooperative yield（pending tool / context limit）、preemptive interrupt、显式 budget / output spec。Behavior 模式不是被删除而是被拆成"waist 内的 step 调度 + parser/renderer trait"和"waist 外的 Agent 状态机"两部分——**当前 Behavior 的结束和下一状态选择，仍然是 LLM 在结构化输出中显式表达的意图**。

Agent scheduler 承接旧 `BehaviorEngine`：根据 `Done.behavior_result.next_behavior` 推进状态机，根据 `do_actions` 调度外部动作，根据 `WAIT_USER_MSG` 等 sentinel 管理 session 等待态。

当前实现中，这个 scheduler 是 libopendan 的 `SessionRunner`：它解释 Outcome、决定 Session Turn 的开始 / 继续 / 结束，并把 run 历史写入 Session worklog（见 [readme](readme.md)、[Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md)）。`src/frame/opendan` 的旧 Runtime（`AgentSession`、round history 等）只做了共享 API 的编译适配，按新抽象重构待下一阶段 opendan 重构接入。

### H4 报告校验与恢复边界（2026-10-10）

两套 XML parser 输出相同的报告字段。`end` 精确为 true / false，重复 report / end / next_behavior、空最终报告、结束与动作或调度同现均反馈纠错。可选兄弟节点 artifacts 为 JSON 字符串数组、result 为任意 JSON；只传递内容，业务校验由宿主承担。

`CheckpointHook::validate_report(snapshot, result, step_index)` 在报告状态更新前验证并提交；`CheckpointStage::BeforeToolCall` 在工具副作用前持久化 assistant 调用批次。Function-call 的 report 工具由 Session 注入，LLMContext 不解析报告业务含义。结束提交接受后宿主请求 finish，回执配齐后完成交付，Settled 本身不是成功或停止的统一判据。恢复快照的最终 report Step 直接产生 Done；宿主按 journal 身份生成一次最终消息，与正常执行使用同一正文和产物引用。
