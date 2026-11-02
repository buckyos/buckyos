# LLM Context 设计

> 本文是 OpenDAN Agent 框架的第一篇文档。读完本文，你应该掌握：
>
> 1. 为什么需要 LLMContext，它和 `llm.complete` / `agent.sendMsg` 的差异
> 2. 它的心智模型（"LLM 进程上下文"）和终态 / 挂起态二分的 Outcome（六个变体）
> 3. 它和 Behavior Loop / Prompt 管线 / Tool 调度 / Snapshot 之间的关系
> 4. 写新 scheduler / provider / tool 时哪些字段进 waist、哪些不该进
> 5. 一个最小宿主（xllm）怎样用它把一次任务跑完、停下、再接着跑（§11 伪代码）
>
> 术语：Round = 一次推理，Step = Behavior Loop 的一次决策记录（`StepRecord`），Turn = AgentSession 的一次逻辑 Input → result，工具迭代 = 工具预算的计量单位；定义见 [readme](readme.md)。本文的 `run()` 指一次 `LLMContext::run()` 调用。
>
> 核对：2026-11-01 按工作区源码逐节核对（`src/frame/llm_context`，快照 v6；`src/frame/agent_tool/src/xllm.rs`，RunRecord v7）。本文描述**现状**；设计取舍的裁决记录见 [llm_context 简化与冻结 TODO](../../notepads/llm-context-freeze-todo.md)，尚未落地的两项在 §11.6 标注"冻结 TODO Fn"。
>
> 分层一句话：**llm_context 只跑 loop**（推理、派发工具调用、把结果配对回历史、决定在哪里停、怎么接着跑、什么时候打 checkpoint）；工具、结果信封、任务模型、调用内的等待预算、按 call_id 的恢复、XML 协议与 prompt 引擎在 Runtime 层（`agent_tool`）；Session、Turn、输入、子 context、子 session 以及在 loop 外跟进任务在宿主（libopendan）。loop 里不允许等待，任何等待都是任务。

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
| Yield / context switch out | `Outcome::ContextLimitReached`：cooperative yield（等 scheduler 重写历史） |
| Preemptive interrupt | `LLMContextInterruptHandle::interrupt(...)`：从 run 外部抢占当前 inference |
| Context switch in | `LLMContext::resume(snapshot, fill, deps)`：恢复挂起态继续跑 |
| Killed by scheduler | `Outcome::BudgetExhausted`：quantum / token / wallclock 任一耗尽 |
| exit syscall | `Outcome::Done` / `Outcome::Error`：正常 / 异常终止 |
| Scheduler | Agent loop / Workflow engine：决定哪个 context 上 CPU |
| Process lifetime | 短生命：一个 LLMContext = "一次智能任务"，不是 Agent 的整段会话 |

由此推出后续所有设计：

- **为什么是对象不是函数**：进程上下文必须有可变 runtime 状态。
- **为什么 Outcome 不止一个返回值**：进程要么 exit、要么 yield 等 context 压缩、要么被外部 interrupt 抢占、要么平滑停下、要么被 kill —— 不可能只有"返回值"。它**不会** yield 等 IO：工作没做完的工具调用返回"仍在运行，任务 X"，等任务是 Runtime / 宿主的事（§3.3）。
- **为什么 owner / scheduler 抽象**：scheduler 不关心进程跑什么业务，只关心生命周期。
- **为什么需要 snapshot**：挂起必须能完整保存执行态以便恢复。

**重要约束：cooperative yield 与 preemptive interrupt 是两条独立控制面。** `ContextLimitReached` 在推理边界产生（请求发送前或 provider 拒绝后），不打断进行中的 inference；`Interrupted` 在 inference 或工具调用过程中由外部触发。**等待不是 waist 概念**：loop 不等任务、不等人、不等别的 context。等用户下一条消息（典型 sentinel `next_behavior == "WAIT_USER_MSG"`）、等任务结束、等子 context 返回，都由 L4 / session 解释：run 以 `Done` 返回，宿主在 loop 外等，结果作为新输入注入同一快照或新的 run。

### 1.3 Loop 不变量：intent → effect → observation

LLMContext loop 不变量**不是** function call → tool result，而是：

```
intent → effect → observation → intent → effect → observation → ... → terminal
```

| 概念 | 在 waist 里的载体 | 谁产生 | 谁消费 |
|---|---|---|---|
| **intent** | `OutputSpec::Json` 解析出的结构化产物（典型字段 `tool_calls / do_actions`）或 provider-native tool_calls | LLM | waist 主循环 → ToolManager |
| **effect** | `ToolManager::call_tool` 内部的实际动作 | ToolManager（effect 实现层） | 外部世界 |
| **observation** | `Observation::{Success \| Error \| Cancelled \| Unresolved}`，每个都是终态 | ToolManager（`Unresolved` 由 waist 在批次中断时补齐，或由 ToolManager 在恢复时对无法确认的调用给出） | waist 主循环 → 喂回下一次推理（Round） |

**为什么不把 function call 抬成一等公民**：function call 是 provider-specific wire format（OpenAI tool_calls / Anthropic tool_use / Gemini function_call / 本地模型经常没有原生支持各家细节都不同），抬上来立刻丢掉 provider 中立性。provider adapter 负责把各家 wire format 归一化成 `AiResponse.message` 里的 tool_use 内容块（`AiMessage::tool_calls() -> Vec<AiToolCall>`），waist 只看到归一化后的列表。

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
│  - Outcome 终态 / 挂起态二分（见 §4）                       │
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

现状：图中的 `LLM*Context` 没有以这个名字落地。Agent 一侧的 L3 / L4 是 libopendan 的 Session / Runner（[readme](readme.md)、[Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md)），OneShot 一侧是 xllm（§11），两者共用 `agent_tool` 的 Runtime 层（[AgentRuntime 设计](<AgentRuntime 设计.md>)）；Workflow 一侧尚无实现。

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
    /// context_limit_reached / interrupted / settled 是挂起态。
    pub async fn run(&mut self) -> LLMContextOutcome;

    /// 从 snapshot 恢复（context switch in）。
    /// fill 的形态必须与 snapshot 的 state.suspended（只有上下文上限这一种）对应；
    /// 不一致会返回 LLMComputeError::SnapshotCorrupted（在任何推理 / 工具调用之前）。
    /// 挂起期间的时间不计入 wallclock 预算。
    pub fn resume(
        snapshot: LLMContextSnapshot,
        fill: ResumeFill,
        deps: LLMContextDeps,
    ) -> Result<Self, LLMComputeError>;

    pub fn snapshot(&self) -> LLMContextSnapshot;

    // 观察与宿主元数据（§9.4）
    pub fn behavior_mode(&self) -> bool;
    pub fn suspension(&self) -> Option<&Suspension>;
    pub fn host_meta(&self) -> Option<&Value>;
    pub fn set_host_meta(&mut self, host: Option<Value>);
    /// 推理边界注入消息 / 替换宿主元数据（§9.4）；挂起或批次未派发完时不放置。
    /// 宿主往运行中的 loop 放上下文的唯一方式：注入的消息进历史、进快照
    pub fn inject(&mut self, injection: Injection) -> InjectionPosition;
    /// ContextLimitReached 快照里宿主可重写的历史（压缩前取用，与 Outcome.accumulated 同一口径）
    pub fn rewritable_history(snapshot: &LLMContextSnapshot, deps: &LLMContextDeps) -> Vec<AiMessage>;
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResumeFill {
    /// function call 模式的 ContextLimitReached ⇒ 重整后的历史成为新的稳定基线，
    /// 同时替换 request.input 与 accumulated。必须保持 tool_use / tool_result 配对；
    /// thinking 块被丢弃（只在产生它的前缀下有效）。如何重整完全由 scheduler 决定。
    RewrittenHistory { history: Vec<AiMessage> },

    /// behavior 模式的 ContextLimitReached ⇒ prompt 由 request.input、step 历史和热
    /// step 物化，只换 accumulated 不够。替换 request.input / history_summaries /
    /// steps / last_step；steps 与 last_step 只能保留（可压缩的）原 step，按 step_index
    /// 识别；step / action 编号与进行中 Step 的 inner transcript
    /// （accumulated 中 request.input 之后的消息）保持不变；thinking 同样丢弃。把整个
    /// 物化历史折叠进 input（steps 为空、没有热 step）是合法的。
    RewrittenSteps {
        input: Vec<AiMessage>,
        history_summaries: Vec<HistorySummaryRecord>,
        steps: Vec<StepRecord>,
        last_step: Option<StepRecord>,
    },

    /// 快照不在挂起态：outcome 边界、checkpoint hook（工具调用前 / 推理前）、
    /// Interrupted / Settled 之后落盘的快照。批次 / step 可能尚未派发完：resume 后
    /// 先续派，没有结果的调用以同一 call_id 再派发一次，由 ToolManager 按 call_id
    /// 决定找回之前的工作还是重跑（§9.3）。没有 payload；挂起态快照配此 fill 返回
    /// SnapshotCorrupted。
    ResumeFromMidRun,
}
```

挂起后在恢复之前再调用 `run()`，返回 `Error{Internal}` 且不改动状态；`inject` 在挂起或批次未派发完时不放置任何内容（返回 `InjectionPosition::None`）。

**派生**（`context_derive.rs`，只产生新的 request + state，不碰 effect 层）：

- `derive_child(snapshot, ForkPoint / InheritHistory, …)` / `fork_snapshot(snapshot, ForkOptions)`：从快照派生子上下文。`ForkOptions` 可换 owner / trace / objective / tool_policy / budget / output，可选换 system（`expect_system`）；返回 `DerivedContext { snapshot, boundary: InheritBoundary }`，boundary 记录继承到哪条消息 / 哪个 step，供宿主把子结果接回父上下文。
- behavior 交接总是换到另一个 run（目标自己的，或用上面两个函数新建的子 run），waist 不提供在已有快照上替换 system / behavior 的重建入口。

### 3.2 LLMContextRequest

不可变输入。消息载体**直接复用 provider 抽象层的 `AiMessage`**（见 §5），waist 不再造一份 `ChatMessage`。

```rust
pub struct LLMContextRequest {
    pub owner: Value,                    // 调度器自己的身份形状，只用于追踪 / 审计，waist 不解释
                                         //   （libopendan {"kind":"agent","session_id":…}，xllm {"kind":"one_shot","id":…}）
    pub trace: Option<String>,           // 调试 trace id
    pub objective: String,               // 自然语言目标，供 worklog 阅读，不进 prompt
    pub behavior_name: String,           // Behavior Loop 当前 behavior；普通切换由宿主改写后在同一快照上继续
    pub input: Vec<AiMessage>,           // L4 已展开的对话历史
    pub model_policy: ModelPolicy,
    pub tool_policy:  ToolPolicy,
    pub output:       OutputSpec,
    pub budget:       BudgetSpec,
    pub error_policy: ErrorPolicy,
    /// Behavior Loop 用：true ⇒ 任何 <next_behavior> 都被丢弃。
    /// fork 子上下文用，强制本次执行结束就终止，不能跳到别的 behavior。
    pub forbid_next_behavior: bool,
}
```

设计要点：

- **不持有 session / 容器句柄**：模板展开、长期记忆注入都在 L4 lowering 阶段完成，进 waist 时 `input` 已经是具体 `Vec<AiMessage>`。
- **不重复 provider 抽象类型**：消息走 `AiMessage`、用量走 `AiUsage`、tool call 走 `AiToolCall`、最终响应走 `AiResponse`。

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
    /// 透传给 adapter 的能力禁用列表（如 thinking / web_search），waist 不解释
    pub disable_capabilities: Vec<String>,
    pub finish_grace_ms: u64,            // 平滑结束时等待不可取消工具的时长（默认 30s）
}

/// 每个 observation 都是终态：waist 从不等任何东西。工作在 Runtime 的等待预算内没做完的
/// 调用，返回一个说明"任务 X 仍在运行"的 Success。
pub enum Observation {
    /// host：Runtime 的结果信封（如 AgentToolResult），waist 原样携带、从不读取；
    /// Runtime 的渲染器读它（agent_tool::ToolResultView::from_host）
    Success { call_id, content: Value, bytes, truncated, host: Option<Value> },
    Error   { call_id, message, host: Option<Value> },
    /// 调用在完成前被取消：打断 / 平滑结束 / 截止时间触发后由 ToolManager 内联返回。
    /// effect_unknown=false：工作已停止或已知仍在运行（文本说明状态）；true：工作不能
    /// 取消、等待被放弃，副作用不可确认
    Cancelled { call_id, reason, effect_unknown },
    /// 没有这个调用的结果：effect_unknown=true 表示调用可能已开始、副作用不可确认，
    /// false 表示它没有开始。waist 在批次中断时用它补齐未执行的调用，让 transcript 与
    /// StepRecord 保持配对、可审计；ToolManager 在恢复时对之前可能已派发、又不能安全
    /// 再跑的调用也返回它（§9.3）。
    Unresolved { call_id, reason, effect_unknown },
}

/// 每次工具调用的上下文：与推理共用的打断 / 平滑结束信号，以及 run 的绝对截止时间
/// （按 budget.max_wallclock_ms 计算）。ToolManager 不再自己维护 cancel watch 或 deadline；
/// 一次调用为工作等多久是 Runtime 的预算，不是 waist 的。
pub struct ToolCallCtx { abort: InferenceAbortToken, deadline_ms: Option<u64> }
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
/// 恢复时未完成批次里的调用会以同一 call_id 再派发：ToolManager 先按 call_id 找它
/// 之前可能已开始的工作；找不到且工具不幂等时答 Unresolved{effect_unknown}，不盲目
/// 再跑（waist 自己从不重放）。
trait ToolManager {
    async fn call_tool(&self, call: AiToolCall, ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError>;
    /// 广告给模型的工具目录；ToolPolicy.mode / whitelist 再过滤一次（resolve_tool_specs）。可以为空
    fn list_tool_specs(&self) -> Vec<ToolSpecLite> { vec![] }
    fn has_tool(&self, name: &str) -> bool;
}
pub struct ToolSpecLite { name, description, args_schema: Value }
pub struct ToolDispatchError { message: String, effect_unknown: bool }
```

工具执行委托给 `ToolManager` trait，policy gate 委托给 `PolicyEngine` trait（`gate_tool_calls` / `gate_action_calls` 在派发前过滤整批调用，拒绝走 ErrorPolicy）。waist 不知道实现细节。

宿主实际使用的值（xllm `hosted_request` / `build_initial_context`，libopendan 沿用）：`mode = Whitelist`（工具集为空则 `None`），`max_calls_per_round = 16`，`finish_grace_ms = 30_000`。观察的大小上限由工具 / Runtime 自己截断（`Observation.truncated`），waist 不设。

**派发**：一个批次的调用按 provider 返回的顺序**串行**派发，每个调用派发前先打一次工具调用前 checkpoint（§9.2）。原生 function call 批次派发全部调用（业务错误不中断，模型一次看到整批结果）；behavior Step 的 action 遇到第一个非成功结果即停，其余记为未执行（§6.4）。

**工具迭代额度**（`max_tool_iterations` → `LLMContextState.tool_iterations_left`）：原生工具批次在整批派发完成后扣一次（恢复后续派的批次在续派完成时才扣），带 action 的 behavior Step 在派发前扣一次；Step 内层的原生批次与外层 action 共享同一额度。无工具的最终回答、纯 report / 决策 Step、解析纠错都不扣，所以剩余额度不能用来推算推理次数。额度为 0 时推理请求的 `allow_tool_calls = false`；模型仍返回 tool call（或带 action 的 Step）⇒ `BudgetExhausted{ToolIterations}`。恢复不重复扣减。

**任务不是 waist 概念**：waist 对工具结果只做一种处理——配对回历史、交给下一次推理。工作比一次调用活得久的，是 Runtime 的任务（`agent_tool::task_model`）：

- **调用内等待预算**：工具结果是 pending 信封（AgentToolResult `status=pending` + `task_id`）时，Runtime 的 ToolManager 在预算内（工具给的 `check_after`，否则 `RuntimeConfig.task_wait_ms`，默认 30 秒、上限 `MAX_IN_TOOL_WAIT_MS` = 30 分钟，再受 run 截止时间约束，`task_model::wait_until_ms`）经 `RunningTaskResolver::wait` 等它；结束了就是普通结果，还在运行就返回一个 Success（"task X is still running …" + 跟进提示，`host` 里的视图带 task_id）。打断 / 平滑结束只取消等待，任务本身继续。
- **谁跟进任务**：`RunningTaskResolver`（`state` / `wait` / `cancel` / `can_resolve` / `watch` / `active` / `followup`）由 Runtime 装配，task_id 对 waist 不透明。`task_state_observation_via` 是 `TaskState` 到 observation 的唯一渲染，"仍在运行"的提示措辞由 `followup()` 决定：`TaskFollowup::Tools`（模型用 `wait_task` / `get_task_state` / `cancel_task` 跟进）、`Command(cli)`（经 `shell` 运行的 task CLI 桥）、`Input(cli)`（Session 宿主在 loop 外跟进，结局作为输入回来，模型不必等）。
- 没有每轮附加在请求末尾的后台任务简介：每轮变化的内容会破坏前缀缓存。宿主要让模型看到什么，就经 `CheckpointHook::before_inference` 注入（进历史、进快照，§9.4）。

### 3.4 OutputSpec / ContextOutput

```rust
pub enum OutputSpec {
    Text,
    Json { schema: Option<Value>, strict: bool, max_repairs: Option<u32> },
}

pub enum ContextOutput {
    Text { content: String },
    Json { content: Value },
    /// 回复契约（`max_repairs: Some(_)`）在允许的纠正后仍未满足：原文与原因。
    Invalid { content: String, error: String },
}
```

**回复契约**（`OutputSpec::Json { max_repairs: Some(n) }`）：最终回复（无工具调用的 assistant 文本）必须是一个 JSON 值（容忍一层 Markdown 代码围栏）、符合 `schema`（`json_check`：JSON Schema 子集——`type`、`enum`、`const`、`properties`、`required`、`additionalProperties`、`items`、`min/maxItems`、`min/maxLength`、`minimum/maximum`、`anyOf/oneOf/allOf`，其余关键字忽略），并通过宿主的 `LLMContextDeps.output_validator`（schema 说不了的检查，如文件存在、计数一致）。不满足时把失败的回复留在历史里，追加一条 user 纠正消息（原因 + schema），再推理一次；每个 run 至多 `n` 次（`LLMContextState.output_repairs`，不计入连续错误）。用完仍不满足返回 `ContextOutput::Invalid`，由宿主决定如何出站。schema 同时作为 provider 原生结构化输出交给 adapter（AICC 映射 Claude `output_config.format` 与 OpenAI `response_format.json_schema`），waist 的校验只是兜底。没有 `max_repairs` 时保持原语义（只解析：`strict` 决定解析失败走 ErrorPolicy 还是原文返回 `Text`）。fork 可以用 `ForkOptions.output` 换掉分支的回复约束（只换约束，不换 system / tools / model，前缀缓存不变）。

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
    Provider { failure: ProviderFailure, message },   // Transient | Permanent | Unknown | ContextLimit | Refused { code }
    OutputParse(msg), PolicyRejected(msg),
    ToolFailed { tool, call_id, message },
    ToolRuntime { tool, call_id, message, effect_unknown },     // 派发基础设施故障
    Checkpoint { stage: BeforeInference | BeforeToolCall | OutcomeBoundary, message }, // 关键持久化失败
    SnapshotCorrupted(msg), Internal(msg),
}
// 三个正交维度，Runtime 按它们分发，不解析 message 文本：
//   source()           Provider | LlmOutput | Tool | Runtime | Snapshot | Internal
//   llm_correctable()  是否喂回 LLM 自纠正（只有 OutputParse / PolicyRejected / ToolFailed）
//   infra_retry_safe() 上层重跑是否不会重复副作用（Timeout / Provider Transient /
//                      Checkpoint / ToolRuntime{effect_unknown=false}）
// ProviderFailure::ContextLimit 不是错误：waist 把它翻译成 ContextLimitReached{ProviderRefused}（§9.5）。
// ProviderFailure::Refused { code: String }：AI 服务以结构化错误码拒绝（额度 / 权限 / 归因）。code 由
// provider adapter 定义、对 waist 不透明（agent_tool 的 REFUSED_QUOTA_EXHAUSTED / REFUSED_QUOTA_BUDGET_EXCEEDED /
// REFUSED_RESOURCE_NOT_PERMITTED / REFUSED_ATTRIBUTION_UNAVAILABLE）。不可自纠正、不视为可安全重试；
// Runtime 按 code 常量分发（xllm classify_error），不解析 message 文本。
```

`ErrorClass::{Recoverable, Fatal}` 由 `llm_correctable()` 穷尽推导，没有默认分支；新增错误种类必须显式决定。

| 故障 | 主要恢复责任方 | 是否反馈给 LLM | 核心循环行为 |
|---|---|---|---|
| Provider 临时故障 / 网络 / 超时 | adapter，其后由调度器决定是否再跑 | 否 | adapter 有界容错耗尽后结束本次 run（`Error{Provider{Transient}}` / `Timeout`），waist 不再隐式重推理 |
| Provider 鉴权 / 模型不存在 / 非法配置 | 配置管理 / 上层 Runtime | 否 | 直接结束本次 run（`Provider{Permanent}`），不消耗自纠正次数；`Unknown` 同样结束且不视为可安全重试 |
| Provider 结构化拒绝（额度耗尽 / 预算超限 / 资源不允许 / 归因不可用） | 额度 / 权限管理，宿主提示用户 | 否 | `Provider{Refused{code}}` 结束本次 run；code 进 run 记录 |
| LLM 输出不符合声明协议（严格 JSON 解析失败、Behavior 解析失败、超过 max_calls_per_round） | LLM 自纠正 | 是 | 失败输出留在 transcript，追加诊断，受 ErrorPolicy 与预算限制 |
| 工具参数 / 业务执行失败 / Policy 拒绝 | LLM 调整计划 | 是 | 对应 call_id 记录 observation 后再推理；传统 loop 跑完整批次，Behavior Action 首错停止并把未执行项记为 `Unresolved` |
| 工具派发基础设施故障、结果未知 | 工具 adapter / Runtime | 否 | 立即停止派发，已知结果保留、未执行项配对为 `Unresolved`，以 `Error{ToolRuntime}` 结束；快照仍可 resume，是否继续由 Runtime 决定，不自动重放 |
| 推理前 / 工具调用前 checkpoint 失败 | 持久化层 / Runtime | 否 | `CheckpointHook` 返回 Err ⇒ 不发起推理 / 不派发该调用，`Error{Checkpoint{BeforeInference \| BeforeToolCall}}`；状态仍是 hook 看到的那份，可只重试保存再 `ResumeFromMidRun` |
| 普通 worklog 失败 | 日志实现 | 否 | best effort，不进上下文、不计数、不改 outcome |
| Provider 以结构化错误码拒绝上下文长度（adapter 归一化为 `ProviderFailure::ContextLimit`，如 OpenAI `context_length_exceeded`） | 调度器（重整历史） | 否 | 挂起为 `ContextLimitReached{ProviderRefused}`，快照是这次推理（Round）前的状态；不凭异常文本或任意 HTTP 400 猜测 |
| 快照损坏 / ResumeFill 不匹配 / 未配对的 tool_use | Runtime / 调用方 | 否 | `resume()` 返回 `SnapshotCorrupted`，拒绝恢复 |
| 编程错误 / 状态不变量损坏 | 开发者 | 否 | `Internal`，终止本次 run |
| 主动 interrupt | 调度器 | 不作为故障 | `Interrupted` + s0 快照，不计数 |

**纪律**：
- Fatal 不可被 ErrorPolicy 改写；`infer()` 返回的任何错误都不会喂回 LLM。
- run 中被 `InferenceAbortToken` 触发的 cancelled 不走 ErrorPolicy，收敛到 `Outcome::Interrupted`；没有 interrupt 信号的 Provider `Cancelled` 是 Provider 故障。
- 可纠正错误的反馈只用 role ∈ {tool, user}：工具 / Policy 错误以 tool_result 配对到 call_id，输出协议错误以 user 消息追加；静态 system 前缀之外不再出现 system 消息。
- `Outcome::Error` 携带 `trace`，其中 `tool_trace[].status ∈ {succeeded, failed, unknown, not_executed, cancelled}` 记录批次中每个调用的最终状态。
- 没有 `max_repairs` 时 `OutputSpec::Json.schema` 只透传给 provider，waist 只做 JSON 解析；回复契约（`max_repairs: Some(n)`）下 waist 另做 schema 子集校验与宿主校验，纠正有次数上限，用完返回 `Invalid`（§3.4）。
- Provider retry / 退避 / fallback chain 都在 adapter 内部，**waist 自己绝不在外面再做一层 retry**；上层若要重跑，是显式决策，不通过伪造 observation 触发。

---

## 4. Outcome：终态与挂起态（六个变体）

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
        response: AiResponse,                 // 最后一次 LLM 响应（message / usage / cost / finish_reason / provider_task_ref）
        trace: ContextRunTrace,
        behavior_result: Option<LLMBehaviorResult>,   // Behavior Loop 产物；传统 Loop 为 None
    },
    /// 终态：异常。error.source()==Runtime 时内存快照仍有效，可 ResumeFromMidRun
    Error { error: LLMComputeError, usage: AiUsage, trace: ContextRunTrace },
    /// 终态：预算红线击穿
    BudgetExhausted { which: BudgetKind, partial: Option<ContextOutput>, usage: AiUsage },

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

`LLMContextOutcome::is_terminal()` 对 Done / Error / BudgetExhausted 为 true。`ContextRunTrace { trace_id, latency_ms, tool_trace: Vec<ToolExecRecord> }`，`ToolExecRecord { tool_name, call_id, status: Succeeded | Failed | Unknown | NotExecuted | Cancelled, duration_ms, error }`。

`Done` 只说明本次 run 停在一个干净的边界，不说明"任务完成"：behavior 模式的结束报告、交接（Handover）和让出（Yield）都是 `Done`（§6.4），function call 模式最后一个没有 tool call 的回复也是。工具交出去的任务、子 context、子 session 还在运行时，宿主可以保留这个 run，把它们的结局作为新输入注入同一快照再 `run()`（libopendan，§11.5），或把 run 停为 paused（xllm，§11.2）。

### 4.1 二分对照表

|  | OS 对应 | snapshot | 可 resume |
|---|---|---|---|
| `Done` | `exit(0)` | 否 | 否 |
| `Error` | `exit(非0)` | 否 | 否 |
| `BudgetExhausted` | OOM kill / SIGKILL | 否 | 否 |
| `ContextLimitReached` | page fault → 等 swap | 是 | `RewrittenHistory`（function call）/ `RewrittenSteps`（behavior） |
| `Interrupted` | external interrupt | 是 | `ResumeFill::ResumeFromMidRun` |
| `Settled` | 平滑结束（SIGTERM 后干净退出） | 是 | `ResumeFill::ResumeFromMidRun` |

### 4.2 上层如何处理

| Outcome | Agent scheduler | Workflow engine |
|---|---|---|
| `Done` | 反序列化 Behavior 结果，按 `next_behavior` 切状态；让出或本 run 的工作未结束时保留 run 等输入 | 写入 node output，进入下一节点 |
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
| `AiToolCall` | `name`, `args`, `call_id` | provider 归一化后的 tool 调用；`ToolBatch.remaining`、`StepRecord.actions` 直接持有；`call_id` 是恢复时找回调用的键 |
| `AiResponse` | `message: AiMessage`（assistant 回复，tool_use 块在内；`text_content()` / `tool_calls()`）, `usage`, `cost`, `finish_reason`, `provider_task_ref`, `extra` | `LlmClient::infer` 的返回；`Outcome::Done.response`、`ActionStep.response`、`LLMResultParser::parse` 的输入 |
| `AiUsage` | `input_tokens`, `output_tokens`, `total_tokens`, `request_units` | 各 outcome 的 `usage` |
| `AiCost` | — | 嵌在 `AiResponse` 里，waist 不单独暴露 |

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
   │        原生工具批次（每个调用前打工具前 checkpoint）         │
   │     3. parser.parse(response) → StepRecord（分配 step_index）│
   │     4. 没有 action：结束 / 交接 / 让出 ⇒ Done                │
   │     5. 否则扣一次工具迭代，派发 actions（state.action_step）  │
   │        → 填 action_results → 沉淀为 last_step，继续          │
   │                                                              │
   └──────────────────────────────────────────────────────────────┘
```

### 6.2 关键 trait（`behavior_loop.rs`）

```rust
/// 一个 Step = 一次行为决策 + 它的 action 结果。Step 身份是 (run, meta.step_index)。
pub struct StepRecord {
    pub meta:              StepMeta,                // behavior_name / step_index / started_at_ms / ended_at_ms / compression_level
    pub assistant_text:    String,
    pub assistant_message: Option<AiMessage>,       // 产生决策的 response（Step 内最后一个 Round）
    pub native_messages:   Vec<AiMessage>,          // 本 Step 内层原生调用与回执的审计记录；不含最终 assistant
    pub observation:       Option<String>,
    pub thought:           Option<String>,
    pub actions:           Vec<AiToolCall>,         // 一个 Step 可以有多个 action（<actions> 容器）
    pub next_behavior:     Option<String>,          // 见 §6.4
    pub self_report:       Option<String>,          // <report>，同时覆盖 state.last_report
    pub report_end:        bool,                    // 显式完成，缺省 false
    pub report_result:     Option<Value>,           // 报告的结构化载荷，形状（含附件）是宿主约定
    pub action_results:    Vec<Observation>,        // 与 actions 按下标对齐，executor 填
    pub next_user_message: Option<AiMessage>,       // 覆盖默认的 action 结果渲染（注入，§9.4）
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
    pub report_result: Option<Value>,
}

impl LLMBehaviorResult {
    /// 形状校验：拒绝 END / done、没有报告的 report_result、空的结束报告、结束报告带
    /// action 或非空 next_behavior。没有 action、不结束、不交接的空决策是合法的（让出）。
    pub fn validate(&self) -> Result<(), String>;
    /// 无 action 且不是结束报告时：next_behavior 非空 ⇒ Handover，否则 ⇒ Yield；其它为 None
    pub fn decision(&self) -> Option<StepDecision>;
}

pub trait LLMResultParser: Send + Sync {
    fn parse(&self, response: &AiResponse) -> Result<LLMBehaviorResult, String>;
}

pub trait StepRenderer: Send + Sync {
    /// 一步沉淀回去 = 一对 (assistant, user)，严格角色交替
    fn render(&self, step: &StepRecord) -> (AiMessage, AiMessage);
    /// 其它 behavior 继承来的 step：只渲染一条（默认实现）
    fn render_inherited(&self, step: &StepRecord) -> AiMessage { /* default */ }
    fn render_summary(&self, summary: &HistorySummaryRecord) -> AiMessage { /* default */ }
    /// 摘要、其它 behavior 的继承记录、当前 behavior 的完整 pair
    fn render_history(
        &self,
        steps: Vec<StepRecord>,
        current_behavior: &str,
        summaries: Vec<HistorySummaryRecord>,
    ) -> Vec<AiMessage> { /* default */ }
}

/// 换人决策：next_behavior 指向别的 behavior 或宿主 sentinel（Handover），或无动作、不结束、
/// 不交接（Yield，可带阶段报告）：本 context 停止推理，直到宿主给它新输入。waist 不判断有没有
/// 东西可等。由 CheckpointHook::review_decision 审核（§9.4）：None / Accept ⇒ 本次 run 以 Done
/// 结束；Refuse(msg) ⇒ 变成纠错 Step，同一 run 再推理（计一次连续错误）。
pub enum StepDecision { Handover, Yield }
pub enum DecisionVerdict { Accept, Refuse(String) }
```

Step 历史不在 waist 内压缩（原 `HistoryCompressor` 已移除）：需要时由宿主在 `ContextLimitReached` 之后用 `RewrittenSteps` 显式重写，见 [append-only history](llm_context_append_only_history.md)。

### 6.3 装配位置

Behavior Loop 通过往 `LLMContextDeps` 注入 trait 实例打开：

```rust
pub struct LLMContextDeps {
    pub llm:       Arc<dyn LlmClient>,          // 一次 infer = 一个 Round；retry / fallback 在 adapter 内
    pub tools:     Arc<dyn ToolManager>,
    pub policy:    Arc<dyn PolicyEngine>,       // 默认 AllowAllPolicy
    pub worklog:   Arc<dyn WorklogSink>,        // 默认 NoopWorklogSink
    pub tokenizer: Arc<dyn Tokenizer>,          // 默认 ByteHeuristicTokenizer；只用于预算估算
    pub checkpoint_hook:  Option<Arc<dyn CheckpointHook>>,   // 唯一的钩子：工具调用前 / 推理前 / 报告与决策审核（§9.4）
    pub result_parser:    Option<Arc<dyn LLMResultParser>>,  // Some ⇒ Behavior 模式
    pub step_renderer:    Option<Arc<dyn StepRenderer>>,
    pub output_validator: Option<Arc<dyn OutputValidator>>,  // 回复契约的宿主校验（§3.4）
}
// LLMContextDeps::new(llm, tools) 之后用 with_policy / with_worklog / with_tokenizer / with_checkpoint_hook /
// with_result_parser / with_step_renderer / with_output_validator 逐个挂上；into_traditional() 见下。
```

- `result_parser = None` ⇒ **传统 Agent Loop**：accumulate AiMessage，直到 LLM 不再调工具。
- `result_parser = Some(_)` ⇒ **Behavior Loop**：每个 Step 跑一次内层 `run_inner`（可含多个 Round 和原生工具批次）+ parse + 派发 action + 沉淀，直到 `Done`（§6.4）。
- 内层上下文用 `into_traditional()` 得到：去掉 parser / renderer / checkpoint_hook，保留 output_validator。所以 `CheckpointHook::before_inference` 只在外层 Step 边界看到外层快照，Step 内层的推理前不打 checkpoint。例外：外层把自己的 `checkpoint_hook` 以 `tool_checkpoint_hook` 交给内层，只用于 `before_tool_call`（内层原生批次每个调用派发前仍打工具前 checkpoint，交的是**外层**快照）和原生 `report` 工具的 `validate_report`。
- Construction-time invariant：`result_parser.is_some() ⇒ step_renderer.is_some()`（否则 `LLMContext::new` panic —— 这是 misconfiguration，不是 runtime error）。

waist 只有 trait，不带协议实现。XML 协议在 Runtime 层：parser 是 `agent_tool::xllm::XllmActionParser`，renderer 是 `agent_tool::XmlStepRenderer`（`agent_tool/src/step_record.rs`，从 `Observation.host` 读结果信封分级渲染）；`llm_context` 的单测用 `tests/protocol.rs` 里的最小协议。

### 6.4 Behavior 模式下的 Outcome

- **终态 `Done.behavior_result: Some(_)`**：一个没有 action 的 Step 结束本次 `run()`，形状有三种：**结束**（`<report end="true">`，`report_end=true`）、**交接**（Handover：非空 `next_behavior`）、**让出**（Yield：不结束、不交接，可带阶段报告；空决策也是让出）。让出立即以 Done 返回，loop 不数连续让出、不在同一 run 里再推理。END / done 被拒绝。结束报告与 XML 动作、同次原生 tool_calls、非空调度互斥；带 action 的 Step 里的 `next_behavior` 被丢弃（结果必须先被观察）；都在副作用前处理。交接 / 让出 / 等待由 Session 解释，目标 behavior 按 `switch_context` / `create_sub_context` / `fork` 构造，不复用其它 behavior 的 run 配置。返回 Done 不自动代表 Turn / Session 完成。
- **挂起态**：按 §4 走，Step 内层也可以停下。内层物化 prompt 装不下（`ContextLimitReached`）、内层推理或工具被打断（`Interrupted`）、平滑结束（`Settled`）都翻译为外层同名 Outcome，携带**外层**快照。
- **Step 未完成时的恢复状态**（都在外层 `LLMContextState`，§9.1）：
  - inner transcript：`accumulated` 中 `request.input` 之后的消息，即进行中 Step 的内层原生工具 Loop（tool_use 与已得结果），尚未折叠进 `StepRecord`。内层返回 Done 后，把该尾部转存到 `StepRecord.native_messages`，排除由 Step 自身承载的最终 assistant；解析 / 策略拒绝形成的纠错 Step 同样保留。进行中的尾部仍留在 accumulated。该字段用于原始调用与回执审计，StepRenderer 不额外渲染它，转存和写 worklog 都不发起推理、不增加 Round。
  - `tool_batch`：内层未派发完的原生批次（还没有结果的调用与 `batch_error`）。
  - `action_step`：已解析、action 尚未派发完的 Step 与它的 response；还没沉淀，`step.action_results` 是已得结果。
  - `next_step_index` / `next_action_id`：下一个待分配的编号。step_index 在解析出 Step 时分配（合成纠错 Step 也占一个），所以是分配位置，不是已完成 Step 数；`action_step` 里的 Step 已有编号但未完成。

  恢复后内层续派或续派其后的 action（没有结果的调用以同一 call_id 再派发，§9.3），不重复解析或派发已有结果的 action、不重复扣工具迭代。上下文检查作用在内层实际物化的 prompt 上；`ContextLimitReached.accumulated` 不含 inner transcript，`RewrittenSteps` 原样保留它。
- **宿主审核**（都经 `CheckpointHook`，§9.4，都在任何副作用之前）：带 `<report>` 的 Step 先 `validate_report`，失败即纠错 Step；交接 / 让出（`StepDecision::Handover` / `Yield`）先 `review_decision`，`Refuse` 即纠错 Step、同一 run 再推理，`None` / `Accept` 则决策生效、本次 run 以 Done 结束，等什么、何时再跑由宿主决定。
- **`forbid_next_behavior = true`**：fork 子上下文专用，任何 `next_behavior` 字段被丢弃，确保子执行结束就终止。

---

## 7. Prompt 构造（不在 waist）

L4 lowering 时把"角色 md / behavior prompt / memory / 工具清单 / 输入"等拼成 `Vec<AiMessage>`，进 waist 时 `request.input` 已经是展开好的消息。waist 自己只物化一样东西：behavior 模式的 Step 历史（`StepRenderer`，§6.2）。模板、提示词段落、输入格式都是宿主的事。

模板渲染的共用实现在 Runtime 层：`agent_tool::prompt_engine::PromptRenderEngine`，一个不带 agent 词汇的纯模板引擎（`RenderVars` 静态值、`ValueLoader` 异步取值、`__INCLUDE__` 受 `include_roots` 约束、`__EXEC__` 默认关闭并经注入的 `PromptExec` 执行、宿主经 `RenderExtensions` 注册命名格式）。当前使用方：

- libopendan：behavior 的 system 模板与自定义输入模板（`runner/assembler.rs::render_template`，`__EXEC__` 关闭，命名格式由 `input_formats()` 注册）；没有配置模板时 `SessionAssembler` 直接拼接。
- `agent_tool::runtime`：`RuntimeValueLoader` 提供 `$runtime.*` 执行体信息，`SandboxPromptExec` 把 `__EXEC__` 交给同一 Sandbox。
- xllm：system 文本由 `PromptPlan` 按段拼接（宿主 / 组文本 + capabilities + cmd_manual + runtime_protocol，§11.2）。

没有共用的 token 预算装配：各宿主按自己的历史结构控制长度（libopendan 的会话历史压缩、xllm 的上下文压缩），装不下由 waist 的 `ContextLimitReached` 报出事实（§4.3）。引擎细节见 [prompt_render_engine](prompt_render_engine.md)。

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
    pub fn is_interrupted(&self) -> bool;
    pub fn is_finishing(&self) -> bool;
    pub fn reason(&self) -> Option<String>;
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
    pub trace_id: Option<String>,
    pub messages: Vec<AiMessage>,
    pub model_alias: String,
    pub fallbacks: Vec<String>,
    pub temperature: Option<f32>,
    pub max_completion_tokens: Option<u32>,
    pub force_json: bool,
    pub json_schema: Option<Value>,
    pub provider_options: Option<Value>,
    pub disable_capabilities: Vec<String>,
    pub tool_specs: Vec<ToolSpecLite>,
    pub allow_tool_calls: bool,
    pub abort: InferenceAbortToken,
}
```

**执行纪律**：

1. `LLMContext::new` 创建共享 `InferenceAbortState`；`interrupt_handle()` 派发外部句柄。
2. 推理前的 checkpoint（function call 模式每次 inference 前，behavior 模式在 Step 边界，`CheckpointHook::before_inference`，§9.2）之后，构造携带 `InferenceAbortToken` 的 `LlmInferenceRequest`。behavior 模式的内层上下文共享外层的 abort state，一个 `interrupt_handle()` 能中断 Step 内任一 Round。
3. waist 把 `provider.infer()` future 和 `abort_token.cancelled()` 跑 `tokio::select!`，即便 adapter 完全忽略 abort，scheduler 线程也能立刻释放。
4. provider adapter 应把 abort 映射到底层 HTTP / SDK cancel；不支持远端 cancel 时丢弃 late response（仍能尽早释放本地）。
5. 收到 cancelled 后返回 `Outcome::Interrupted`，snapshot 是**被中断的这次 inference 发起前**的状态——半截 token / 半截 tool call 不进入 accumulated；behavior 模式是外层快照，进行中 Step 的 inner transcript 保留在其中。
6. resume 时用 `ResumeFill::ResumeFromMidRun`：context 从这次 inference 前重新推进（behavior 模式继续同一个 Step）。
7. **工具执行期间**：每次 `call_tool` 带 `ToolCallCtx { abort, deadline_ms }`。工具是否支持取消由实现声明（`AgentTool::cancellable`，默认否）：支持的工具自己监听 `ctx.cancelled()`、处理当前命令后返回 `Cancelled{effect_unknown:false}`（文本写明命令状态与"可能已有部分副作用"）；不支持的工具由 ToolManager 在 `cancelled_hard()` 触发时放弃等待，返回 `Cancelled{effect_unknown:true}`。waist 按 `ctx.cause()` 收尾：打断 → 同批其余 `Unresolved{effect_unknown:false}`，`Interrupted`（快照含已配对的 Cancelled，恢复后不重跑）；到期 → `BudgetExhausted{Wallclock}`；平滑结束 → 其余 `Cancelled{effect_unknown:false}`，`Settled`。behavior 模式按"第一个非成功结果停止其余 action"处理，Step 被沉淀后再返回。
8. 平滑结束期间 waist 自己计时：工具在 `finish_grace_ms` 内没有返回就把 finishing 升级为 abort（§7 的放弃等待路径）。

`Interrupted` / `Settled` 与 cooperative yield 的边界：

- `ContextLimitReached` —— 在推理边界让出 CPU（请求发送前或 provider 拒绝后）。
- `Interrupted` —— scheduler 在 inference 或工具调用过程中抢占。
- `Settled` —— scheduler 要求平滑结束，waist 在下一个干净的边界停下。
- 等待（用户输入、任务、子 context）—— L4 / session 状态，不是 waist 概念。

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
    pub output_repairs: u32,             // 回复契约已用的纠正次数（§3.4）
    /// 唯一的挂起：上下文上限 Suspension { which, estimated_tokens, at_ms }
    pub suspended: Option<Suspension>,
    /// 未派发完的原生工具批次（function call，或 behavior 进行中 Step 的内层）：
    /// { remaining, batch_error }；remaining 里的调用按序派发
    pub tool_batch: Option<ToolBatch>,
    /// behavior：已解析、action 尚未派发完的 Step（未沉淀）：{ step, response }
    pub action_step: Option<ActionStep>,
    /// Behavior Loop：沉淀的历史 steps；run 内只追加，重写只经 RewrittenSteps
    pub steps: Vec<StepRecord>,
    pub history_summaries: Vec<HistorySummaryRecord>,
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
- **不持有 effect-side 真实世界状态**：snapshot 只持有逻辑层产物（call_id / observation / accumulated / step / usage），**绝不**持有进程句柄、fd、network connection、容器 PID。waist 也不记录任务 id（结果信封 `host` 里的 task_id 对它不透明）：一个调用变成了哪个任务，由 Runtime 按 call_id 找回（§9.3）。
- **input 已展开**：跨节点 resume 不依赖任何模板环境。模板展开发生在 L4。

> **工程提醒**：如果你想往 snapshot 里塞"句柄""指针""长生命态引用"，停下来——那些东西属于 `LLMContextDeps`（resume 时重新提供），不属于 snapshot。

### 9.2 Checkpoint 时机

waist 决定**什么时候**打 checkpoint，宿主决定存到哪里。两个时机都经 `CheckpointHook`（§9.4），交的都是**外层**快照：

- **工具调用前**（`before_tool_call`）：每个原生工具调用 / behavior action 派发前一次，包括 behavior Step 内层的原生批次。快照里这个调用还在 `tool_batch.remaining` / 还不在 `action_step.step.action_results` 里。调用串行派发，所以最新的工具调用前快照准确说明了哪个调用可能正在跑：第一个没有结果的调用（`agent_tool::exec_tracking::in_flight_call`）。这是崩溃恢复的**唯一事实来源**，宿主不另记在途调用。
- **推理前**（`before_inference`）：function call 模式在每次推理（首次推理、每个工具批次的结果之后）前；behavior 模式只在外层 Step 边界，不在 Step 内层的推理前。Step 内崩溃时从 Step 边界或最近的工具调用前 checkpoint 恢复，最多重付一次推理。

规则：

- 不注入 hook 也合法（纯内存运行）。一旦注入即承担关键 checkpoint：写失败不能继续推理 / 派发；此时 waist 状态仍是交给 hook 的那份，Runtime 只需重试保存并 `ResumeFromMidRun`，不会重放任何副作用。
- 只承担观测职责的 hook 应自己吞错并返回 Ok。
- snapshot 落到哪 / 加密 / 压缩 / 归档全部是 effect 层私事（§A.4）。
- 它是推理 / 工具调用边界的钩子，不在 AgentSession Turn 开始或结束时调用；outcome 边界由宿主自己再存一次（§9.3）。
- 现状：xllm 的 `SnapshotHook` 在两个时机都写 `snapshots/NNNN.json` 并更新 `run.json` 的 `latest_snapshot_idx`（§11）；libopendan 的 `SessionCheckpointHook` 在两个时机存 run 快照，另做 lease 检查与 Session 提交（§9.4）。

### 9.3 崩溃恢复流程（L4 持久化层用）

```
[Process A]                                  [Process B (after crash)]

ctx = LLMContext::new(req, deps)
s0 = ctx.snapshot();  sink.persist(s0)       // 启动前落盘
loop {
    // CheckpointHook 在每个工具调用前、推理前再落一份（§9.2）
    outcome = ctx.run().await
    s1 = ctx.snapshot();  sink.persist(s1)   // outcome 边界落盘
    match outcome { ... }
}
           |
           ▼ 进程崩溃
                                            s = sink.load_latest()
                                            call = in_flight_call(s)   // 可能已开始的调用
                                            tools.set_recovering(call) // Runtime 按 call_id 找回
                                            // snapshot 不在挂起态 → ResumeFromMidRun
                                            ctx = LLMContext::resume(
                                                s, ResumeFill::ResumeFromMidRun, deps
                                            )?
                                            loop { outcome = ctx.run().await; ... }
```

**纪律**：

- L4 必须能区分"崩在上下文上限挂起"与"崩在运行中"：前者用 `RewrittenHistory` / `RewrittenSteps`，后者用 `ResumeFromMidRun`。waist 在 resume 里做一致性校验拦截误用：挂起态快照配 `ResumeFromMidRun` 返回 `SnapshotCorrupted`；accumulated 里存在未配对的 `tool_use`（`tool_batch.remaining` 里尚未派发的调用除外）同样被拒绝。
- **恢复从最近的 checkpoint 重新进入 loop**：没有结果的调用（`tool_batch.remaining`、`action_step` 里其后的 action）以同一 call_id 再派发。**不重放是 ToolManager 的责任**：它先按 call_id 找之前可能已开始的工作（xllm：`shell` 的任务 `local:shell:<call_id>` 已结束就直接用它的结果），幂等工具（`AgentTool::idempotent()`，如 read / glob / grep / wait_task / get_task_state）直接再跑，其余答 `Unresolved{effect_unknown: true}` 并附 Runtime 对现场的描述（`AgentRuntime::describe_interrupted`）。只有第一个没有结果的调用可能已开始（`in_flight_call`），其后的调用必然没有开始。任务后端的幂等键也由 call_id 派生（Zone TaskMgr `shell:<run_id>:<call_id>`、websdk `ask_user` 的 `ask:$XLLM_RUN_ID:$XLLM_CALL_ID`）。
- L4 要区分"算出 outcome"和"outcome 已提交"：边界写入失败时保留已算出的 outcome 与内存快照，只重试保存，不重新 `run()`。
- checkpoint 不能独自保证不重复扣费：推理前 checkpoint 之后崩溃会重付那次推理；behavior Step 内层的推理前不打 checkpoint，崩溃时从 Step 边界或最近的工具调用前快照恢复。**provider 侧的幂等是 effect 层私事**。外层 Step 状态由 `CheckpointHook` 在 Step 边界与工具调用前、由 L4 在 outcome 边界提交，Step 内的进度由外层快照（inner transcript / `tool_batch` / `action_step`）承载。

### 9.4 CheckpointHook、注入与宿主元数据（2026-09-29，Agent Session SDK §8.7 X2/X3/X4/X5）

`CheckpointHook` 是 loop 唯一的钩子；不注入时行为不变（纯内存运行）。

```rust
#[async_trait]
pub trait CheckpointHook: Send + Sync {
    /// 每个原生工具调用 / behavior action 派发前（含 Step 内层的原生批次，§6.3），以**外层**快照调用。
    /// 快照里 tool_batch / action_step 记着正要派发的调用（§9.2：崩溃恢复的唯一事实来源）。
    /// Err ⇒ 该调用不派发，Error{Checkpoint{BeforeToolCall}}。默认 Ok(())。
    async fn before_tool_call(&self, snapshot: &LLMContextSnapshot) -> Result<(), String> { Ok(()) }

    /// 带 <report> 的 Step 解析后、报告状态更新前：宿主验证并提交报告（H4）。Err ⇒ 纠错 Step。默认 Ok(())。
    async fn validate_report(&self, snapshot: &LLMContextSnapshot, result: &LLMBehaviorResult, step_index: u32)
        -> Result<(), String> { Ok(()) }

    /// 交接 / 让出（Handover / Yield）生效前的审核（§6.4）。None / Accept ⇒ 决策生效，run 以 Done 结束；
    /// Refuse(msg) ⇒ 纠错 Step，同一 run 再推理。默认 None。
    async fn review_decision(&self, snapshot: &LLMContextSnapshot, result: &LLMBehaviorResult, step_index: u32,
        decision: StepDecision) -> Option<DecisionVerdict> { None }

    /// 以**外层**快照调用：function call 模式在每次推理（Round）之前（首次推理、
    /// 每个工具批次的结果之后）；behavior 模式只在外层 Step 边界（每个 Step 的
    /// do-action 之后、下一个 Step 的内层推理之前），不在 Step 内层的推理前调用，
    /// 也不会交给内层逐步上下文（into_traditional 会去掉它）。Step 内层原生批次
    /// 未派发完、恢复后续派时不是 Step 边界，不调用。
    /// Ok(None)：快照已持久化，可以推理；Ok(Some(injection))：waist 应用注入后
    /// 再调用一次，使注入后的状态先落盘（每个边界最多 MAX_INJECTIONS_PER_BOUNDARY = 4 次）；
    /// Err：以 Error{Checkpoint{BeforeInference}} 结束，内存快照仍可恢复。
    async fn before_inference(&self, snapshot: &LLMContextSnapshot)
        -> Result<Option<Injection>, String>;
}
pub struct Injection { pub messages: Vec<AiMessage>, pub host: Option<Value> }
pub enum InjectionPosition { RequestInput(usize), Accumulated(usize), Step(u32), None }
```

- `before_tool_call` 的现状：libopendan 的 `SessionCheckpointHook` 在这里检查 lease 与工作区准入、存快照、同步报告；xllm 的 `SnapshotHook` 在这里写 `snapshots/NNNN.json`。两者都以它为"哪个调用可能正在跑"的唯一依据（§9.2），`run.json` 不另记在途调用。
- **注入是唯一的上下文原语**：宿主放进运行中上下文的内容都走 `Injection`，注入的消息进历史、进快照，恢复后模型看到的与崩溃前一致；没有只进本次请求的临时注入（每轮变化的内容会破坏前缀缓存）。现状：libopendan 在 run 之间用 `inject` 提交输入批次（新消息、`<<task_result>>`、子 session 事件）及其回执，在 `before_inference` 注入 after_tools 的 `<<background_context>>`（Todo / 子会话变化行）；xllm `--resume <message>` 用 `inject` 把新消息接在快照停下的边界（§11.4）。
- `LLMContext::inject(injection) -> InjectionPosition`：function call 追加到 `accumulated`；behavior 模式并入热 step（或 `Done` 之后当前 behavior 的最后一个 step）的 `next_user_message`（接在默认的动作结果渲染之后），否则追加到 `request.input`。宿主用同一规则预测位置，把位置写进 receipt。
- `LLMContextState.host: Option<Value>`：宿主元数据（如 libOpenDAN 的输入 receipt，键为宿主名），随每个快照、resume 和其它执行者的续跑原样保留，waist 不解释。
- `LLMContextState.snapshot_version`：当前为 6（§9.5）；`resume` 只接受等于 `SNAPSHOT_FORMAT_VERSION` 的版本，更旧或更新的都返回 `SnapshotCorrupted`，调用方据此保留现场而不是新建上下文。
- behavior 模式的 `Interrupted` 返回**外层**快照（含进行中 Step 的 inner transcript），可直接 `ResumeFromMidRun`。
- `agent_tool::XmlStepRenderer::without_timestamps()`：历史记录不渲染 `started_at_ms` / `ended_at_ms`，同样的 steps 渲染出相同字节（X8）；宿主装配的 xllm run 使用它，默认渲染不变。
- 在途动作与执行跟踪（X6）属于 effect 层，见 `agent_tool::exec_tracking`（`in_flight_call`）与 [Lease Protocol](../opendan/protocol/Lease%20Protocol.md) §5。

### 9.5 挂起与恢复（X7，2026-09-30）

- **快照版本 6**：`resume` 只接受版本 6，更旧或更新的版本都以 `SnapshotCorrupted` 拒绝，不做迁移；格式变化直接升版本。
- **只有上下文上限会挂起**：`Suspension { which: ContextLimitKind, estimated_tokens, at_ms }`。工具从不挂起 loop：每个 observation 都是终态，工作没做完就是一个"仍在运行"的结果（§3.3）。快照里的调用分三类：已执行（结果已在 transcript / step 中）、可能已开始（工具调用前快照里第一个没有结果的调用）、尚未执行（其后的 `tool_batch.remaining` 或 step 其后的 action）。恢复后余下调用继续派发（§9.3），整批完成时才扣一次工具迭代、最多计一次错误；带 action 的 Step 在派发前已扣过，恢复后不再扣。usage、tool_iterations_left、错误计数、step / action / call 编号都不重置。behavior action 在第一个非成功结果（业务错误、Cancelled / Unresolved）后停止其余 action。
- **ContextLimitReached**：只在推理边界检查（首次推理、工具结果追加后、checkpoint hook 注入后、behavior 内层物化 prompt 后），顺序在 hook 与 abort 检查之后，因此 checkpoint 失败与主动 interrupt 优先于挂起；token / wallclock 预算终态也优先。估算：每条消息 4 token 开销 + 各文本部分（文本、tool_use 参数、tool_result、thinking、provider state）经 `Tokenizer` 计数，图片 / 文档等非文本部分每个按 1024 计，另计工具描述（允许调用工具时）与输出 schema；多模态 prompt 可能被低估。估算值只用于检查，不与 Provider usage 混用。重写后仍装不下时再次让出、不推理，压缩次数与失败策略由宿主决定。
- **宿主接入**（2026-09-30）：xllm 以 `.llm_context` 的 `context_window` 提供窗口（设置后阈值为 0.75），压缩后以 `RewrittenHistory` / `RewrittenSteps` 续跑、每个 run 最多 3 次，接手上下文上限挂起的快照时先压缩（§11.4）；OpenAI 兼容 adapter 把 `context_length_exceeded` 归一化为 `ProviderFailure::ContextLimit`。libOpenDAN（当前实现）的中途重写见 [Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md) §7：先把 run 尚未写入的历史 flush 进 Session worklog，再以 system + 重建的会话历史作为新 input 恢复（behavior 把 steps 全部折叠进 input，编号继续，进行中 Step 的 inner transcript 由 waist 保留），开始新的 history epoch；重写不改变当前 Turn，细节见 [append-only history](llm_context_append_only_history.md)。
- **等待在宿主**（2026-11-01 核对）：libOpenDAN 在 loop 外跟进任务与子 session。一个让出、或本 run 启动的工作（它的任务、还没听到结局的子 session）未结束时的回复，**保留 run**：run 状态 paused，Session `run_state = waiting`，任务记 `watched_tasks`、子 session 记 `wait_children`，Turn 保持打开（多 Turn 的 Work Session 由这次回复关闭 Turn）；保留记在 run.json `host.extra.kept`，崩溃后由 reconcile 重做。它们的结局以输入（`<<task_result task=… call_id=… status=…>>`、`<event … source="session:<sid>" …>`）注入同一快照再 `run()`，step 编号继续。`call_behavior` 只登记子 context，这一段 run 结束时 Session 交接给它，结果以带 `call_id` 的 `<process_result>` 输入回到调用方。提问用户也是任务：websdk `ask_user` 发 `input` 通知（Zone TaskMgr 的 `human.input/v1` 任务），答案是该任务的结果。xllm 由模型在调用内等待预算之外用 `wait_task` / `get_task_state` 跟进；让出、或回复时 resolver 还列着运行中任务，run 停为 `paused`（`last_error.kind` 为 `yield` / `tasks_running`），`xllm --resume [message]` 继续同一快照（§11.2 / §11.4）。

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
    ├─> finishing (and not aborted)? → Settled(snapshot)        // nothing in flight
    ├─> check wallclock budget → BudgetExhausted?
    ├─> checkpoint_hook.before_inference (outer snapshot, may inject, re-checkpoint ≤ 4) → Err ⇒ Error{Checkpoint}   // 内层没有
    ├─> s0 = snapshot()                                    // for Interrupted
    ├─> if abort.is_aborted(): return Interrupted(s0)
    ├─> estimate(request) vs window / threshold → ContextLimitReached(s0), not sent  // §9.5
    ├─> tokio::select!:                                    // 一个 Round
    │     - cancelled() → Interrupted(s0)
    │     - llm.infer(req) → response
    │         ├─ Err(Provider{ContextLimit}) → ContextLimitReached{ProviderRefused}(s0)
    │         ├─ any other Err (adapter tolerance exhausted) → Error{err}, never fed back
    │         └─ ok → continue
    ├─> account usage; check token budget → BudgetExhausted?
    ├─> if no tool_calls or ToolMode::None:
    │     reply contract (Json{max_repairs: Some(n)})? parse / json_check / output_validator fail
    │       ⇒ push reply + correction user msg, output_repairs += 1, next inference; n used up ⇒ Done{Invalid}
    │     native `report` tool (hosted)? → tool_checkpoint_hook.validate_report, written to the OUTER state
    │     → Done; strict JSON parse failure ⇒ push output + diagnostic, bump, next inference
    ├─> tool_iterations_left == 0 → BudgetExhausted(ToolIterations)
    ├─> too many calls (> max_calls_per_round) / policy reject
    │     ⇒ push assistant msg + error tool_result per call, bump, next inference
    ├─> push assistant_tool_call message; tool_batch = { remaining: calls, batch_error: None }
    ├─> dispatch tool_batch serially, for each call:
    │     finishing? → answer rest as Cancelled{effect_unknown:false}, Settled
    │     checkpoint_hook.before_tool_call(outer snapshot) → Err ⇒ Error{Checkpoint{BeforeToolCall}}, call not dispatched
    │     ctx = ToolCallCtx { abort, deadline_ms }
    │     match tools.call_tool(call, ctx)  (finish grace elapsed ⇒ escalate to abort):
    │       Err(dispatch) → record Unknown/NotExecuted, answer rest as Unresolved,
    │                       Error{ToolRuntime}
    │       Ok(Success)   → push tool message                // "still running, task X" is a Success too
    │       Ok(Error)     → push tool message, remember batch_error (batch continues)
    │       Ok(Unresolved) → push tool message, record Unknown/NotExecuted (batch continues)
    │                       // the manager's answer for a call it cannot safely run again (§9.3)
    │       Ok(Cancelled) without ctx.cause()  → contract violation ⇒ Internal
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
    │       ContextLimitReached / Interrupted / Settled
    │            → keep inner transcript + tool_batch in the outer state,
    │              return the same outcome with the OUTER snapshot
    │       Error / BudgetExhausted → return as the outer outcome
    ├─> parser.parse(response) + result.validate(); failure ⇒ correction step (takes a step_index), sediment, bump, continue
    │     (final report + actions / native calls / scheduling → rejected here, before side effects)
    ├─> report? → checkpoint_hook.validate_report; handover / yield? → checkpoint_hook.review_decision
    │     Refuse ⇒ correction step, bump, continue
    ├─> step = prepare_step(result)                        // allocates step_index / action call_ids
    ├─> step has actions && tool_iterations_left == 0 → BudgetExhausted(ToolIterations)
    ├─> report → last_report / report_end
    ├─> yield (no action, not an end, no hand-over) → Done now        // the loop never waits
    ├─> action policy gate (reject ⇒ correction step, continue); actions present → next_behavior dropped
    ├─> actions? → tool_iterations_left -= 1               // charged before dispatch
    ├─> action_step = { step, response }
    ├─> dispatch actions serially (each: finishing? → settle step; checkpoint_hook.before_tool_call(outer snapshot));
    │     first non-success stops the rest (Unresolved / NotExecuted)
    └─> complete_step: report_end / next_behavior (step without actions) → Done
                       else sediment as last_step (bump on action error), next Step
```

---

## 11. 参考：xllm 怎样驾驭 LLMContext（伪代码）

xllm（`src/frame/agent_tool/src/xllm.rs`，CLI `agent_tool xllm`，SDK 见 [xllm Rust SDK](xllm_rust_sdk.md)，run 目录协议见 [local_llm_context_protocol](local_llm_context_protocol.md)）是最小的宿主：没有 Session、没有长生命态，**一次任务 = 一个 run 目录 + 一个 LLMContext**。它把 §2 里 L3 / L4 要做的事压成一份：装配 request 与 deps、持久化快照、解释六种 Outcome、崩溃后接手。libopendan 的 Session 在同一骨架上再加 Turn / lease / 输入回执 / 在 loop 外跟进任务（§11.5）。下面的伪代码按源码整理，省略了日志、锁的错误分支和远程 runtime。

### 11.1 对象与落盘

```
RunStore   ~/.xllm/runs/<run_id>/
             ├── run.json            RunRecord v7：status、config(EffectiveConfig: provider / model / loop_model /
             │                       tools{native, actions} / limits / runtime{…, task_wait_ms} / runtime_descriptor)、
             │                       prompt(PromptPlan)、pending_input、latest_snapshot_idx、last_error、host、handover、
             │                       result、usage、compactions（不记在途调用：派发了什么只看工具调用前快照，§9.2）
             ├── snapshots/NNNN.json LLMContextSnapshot（v6），先写临时文件、fsync、rename
             ├── exec/<call_id>/     shell 的执行目录（由 call_id 派生；退出码、输出；接手时 describe_interrupted 读它）
             └── .lock               run 锁；另有 ~/.xllm/locks/ 下的工作目录锁

RunStatus  running | interrupted | paused | completed | failed | limit_reached     // 后三个是终态

XllmRun { store, record: Arc<Mutex<RunRecord>>, deps: XllmDeps, waist_deps: LLMContextDeps,
          llm: TimeoutLlmClient, manager: Arc<XllmToolManager>,
          ctx: Option<LLMContext>, limit_snapshot: Option<LLMContextSnapshot>, interrupter: XllmInterrupter }

XllmDeps { llm_factory, host_tools, observer: RunObserver, runtime: Option<AgentRuntime>, node_executor,
           buckyos_tasks / local_tasks / shell_tasks（task 解析与 shell 转 task 的归属）, runtime_env, lock_dir,
           skip_workdir_lock, host_protocol / host_prompt_sections / shell_note / task_cli（hosted run 用）, url_reader }
```

### 11.2 新建并执行（`xllm "question"`）

```
xllm "question" [--file …] [--select <group> | --system …] [--json] [--loop-model behavior|function_call]
│
├─ prepare(workdir, input, overrides, deps) -> PreparedTask                         // 不碰模型，不建 run
│    输入互斥检查
│    读 .llm_context 配置层并合并：provider / model / file_model / loop_model / result_format / limits / runtime
│    模式与选组（prompt group）
│    工具：host_tools + 内置组 → XllmToolManager::register → EffectiveTools { native, actions }；
│          manager.set_task_wait_ms(runtime.task_wait_ms)（调用内等待预算，§3.3）
│    提示词：PromptPlan { system_prompt = 宿主 / 组文本 + capabilities + cmd_manual + runtime_protocol 段 }
│    任务要求 / 附件 / 消息组装：initial_messages = [system, user(question + 附件)]
│    有图片且配了 file_model ⇒ file_stage（先让文件模型分析，主模型只收文本）
│
├─ XllmRun::start(prepared, deps)
│    run_id = store.create_run()                                   // runs/<id>/、snapshots/
│    manager.bind_run_dir(run_id); 取 run 锁 + 工作目录锁（有工具且 runtime 非远程）
│    llm = TimeoutLlmClient(llm_factory.create(provider), llm_timeout_secs)
│    record = RunRecord { status: Running, pending_input: Some(initial…), latest_snapshot_idx: None }
│    store.write_record(record)                                    // 首次模型请求前已落盘
│    waist_deps = LLMContextDeps::new(llm, manager)
│        .with_worklog(ObserverWorklog)                            // WorkEvent → RunObserver（CLI 进度 / 日志）
│        .with_checkpoint_hook(SnapshotHook { store, record })     // 每个工具调用前、推理前写 snapshots/NNNN.json
│                                                                  //   并更新 latest_snapshot_idx（§9.2）
│        + behavior 模式: .with_result_parser(XllmActionParser(actions)).with_step_renderer(XmlStepRenderer)
│    // policy 是 AllowAllPolicy；任务由 manager 自己的 CompositeTaskResolver（进程内 task + 宿主 task-mgr）跟进，waist 不知道
│
└─ execute() -> RunOutcome
     interrupter.requested()?           ⇒ status Interrupted，返回
     limit_snapshot.take()?             ⇒ compact_context 后继续（接手时停在上下文上限的 run，§11.4）
     ctx.is_none():
         file_stage 且未做               ⇒ run_file_stage()：文件模型分析图片，结果写 record.file_model_stage
         build_initial_context():
             request = LLMContextRequest {
                 owner: {"kind":"one_shot","id":run_id}, trace: run_id, objective: summary,
                 behavior_name: behavior ? "xllm" : "",
                 input: initial_messages                         // 有 file stage 时换成 system + user(文本 + <image_analysis>)
                 model_policy  { preferred: model, max_completion_tokens: limits.max_tokens },
                 tool_policy   { Whitelist(native) / Whitelist(actions)（空则 None）, max_tool_iterations（工具关闭则 0）,
                                 max_calls_per_round: 16, disable_capabilities, finish_grace_ms: 30_000 },
                 output: json && function_call ? Json { schema, strict: false, max_repairs: None } : Text,
                 budget        { max_wallclock_ms: timeout_secs, context_window_tokens: .llm_context.context_window,
                                 context_yield_threshold: Ratio(0.75)（有窗口时） },
                 error_policy: default(3), forbid_next_behavior: false }
             ctx = LLMContext::new(request, waist_deps); interrupter.handle = ctx.interrupt_handle()
             SnapshotHook.commit(ctx.snapshot())                         // 首次推理前的快照
     loop:
         interrupter.requested()?        ⇒ status Interrupted，返回
         outcome = ctx.run().await                                       // 一个或多个 Round（§10）
         SnapshotHook.commit(ctx.snapshot())                             // outcome 边界快照
             失败 ⇒ 保留 ctx 与 outcome，status Paused(storage)，返回        // 不丢 outcome，只重试保存
         match outcome:
           Done { output, usage, response, behavior_result }:
               hosted && report_end                 ⇒ status Paused（最终报告由宿主 Session 交付）
               hosted && next_behavior 指向别的 behavior ⇒ status Paused + record.handover（宿主提交切换）
               behavior 让出（不结束、不交接） ⇒ 保留 ctx，status Paused，last_error.kind = "yield"
                                                   （message = 阶段报告；`--resume <message>` 带新输入继续）
               function call 回复时 manager.resolver().active() 还有 running 的任务
                                                 ⇒ 保留 ctx，status Paused，last_error.kind = "tasks_running"
                                                   （任务结束后 `--resume` 继续，可带消息）
               否则 raw = response.message.text_content()（Json 输出且无文本时用 content）
                    finish_with_response(raw)：按 result_format 抽取（--json 时校验）→ status Completed，record.result
           BudgetExhausted { which }                 ⇒ status LimitReached，limit_reason（工具迭代 / 总时长 / token / cost）
           Error { error }                           ⇒ classify_error（按枚举：Timeout / Provider{Transient} / Checkpoint … 可恢复
                                                        ⇒ Paused，否则 Failed）；error.source()==Runtime 时保留内存 ctx
           Interrupted { reason } | Settled { reason } ⇒ status Interrupted，interrupt_reason
                                                        （CLI：Ctrl-C → interrupter.interrupt("user interrupt")；第二次 Ctrl-C 直接退出）
           ContextLimitReached { accumulated, snapshot, usage } ⇒ compact_context：
               compactions >= 3                      ⇒ Failed(context_capacity)
               rewritten = LlmSummarizeCompressor(waist_deps.into_traditional(), model).compress(accumulated)
               fill = behavior ? RewrittenSteps { input: rewritten, steps: [], last_step: None } : RewrittenHistory { rewritten }
               ctx = LLMContext::resume(snapshot, fill, waist_deps); SnapshotHook.commit(ctx.snapshot()); compactions += 1
               continue

deliver_outcome(RunOutcome)：Completed → 打印结果（--json 输出 record 摘要）；Paused / Interrupted → 打印 resume 命令；
                            Failed / LimitReached → 打印原因；退出码按状态
```

### 11.3 工具调用（`ToolManager` 的实现）

```
XllmToolManager::call_tool(call: AiToolCall, ctx: ToolCallCtx) -> Result<Observation, ToolDispatchError>
    tool = tools[call.name]?                     否则 Ok(Observation::Error("tool … is not available in this run"))
    call_id 是接手时登记的在途调用（set_recovering，§11.4）⇒ recover_call：
        shell 且任务 local:shell:<call_id> 已结束 ⇒ 返回该任务的结果
        tool.idempotent()                    ⇒ 照常再跑
        否则                                  ⇒ Ok(Unresolved { effect_unknown: true, reason: describe_interrupted 的说明 })
    fut = tool.call(SessionRuntimeContext, args)  // 在 CURRENT_TOOL_CTX / CURRENT_TOOL_CALL 作用域内：工具能读到 ctx 与 call_id
    result = tool.cancellable()
        ? fut.await                               // 工具自己观察 ctx.cancelled()，返回 Cancelled
        : select! { fut, ctx.cancelled_hard() ⇒ return Ok(Cancelled { effect_unknown: true }) }
    match result:
        Ok(AgentToolResult { status: Pending, task_id, check_after, .. }):
            task_id 为空                          ⇒ Observation::Error（协议违规）
            否则 wait_in_tool（预算 = check_after | task_wait_ms，再受 30 min 与 run 截止时间约束，task_model::wait_until_ms）：
                 select! { resolver.wait(task_id, until), ctx.cancelled() ⇒ Cancelled{effect_unknown:false}（任务本身继续） }
                 → task_state_observation_via(call_id, task_id, state, resolver.followup())
                   // 结束了：任务结果；还在运行：Success "task X is still running …"，host 的视图带 task_id
        Ok(res) ⇒ write_file / edit_file 成功 ⇒ 记入 artifacts
                  map_result_to_observation(res)：
                      Success ⇒ Observation::Success { content: output | summary | detail, bytes, truncated, host: res.to_host() }
                      Error   ⇒ Observation::Error { message: summary + output, host: res.to_host() }
        Err(AgentToolError::Transport { message, effect_unknown }) ⇒ Err(ToolDispatchError)    // waist 停派发、Error{ToolRuntime}
        Err(AgentToolError::Cancelled { .. })                      ⇒ Observation::Cancelled
        Err(other)                                                 ⇒ Observation::Error
    结果信封里有 task_id ⇒ resolver.watch(task_id)（task 工具 / 宿主之后跟进它）

shell：有 task manager 且等待预算小于命令超时时，命令在调用内等预算（task_wait_ms）这么久，没结束就转成 task
    （local:shell:<call_id>，或宿主 Zone TaskMgr，幂等键 shell:<run_id>:<call_id>），返回 status=success + task_id +
    partial_output（"still running as task X" + 跟进提示），manager 据 task_id watch 它。之后的跟进按 followup()：
    xllm 的模型用 wait_task / get_task_state / cancel_task 或 task CLI 桥；Session 宿主（TaskFollowup::Input）在 loop
    外跟进，结局作为输入回来。
```

### 11.4 接手（`xllm --resume`）

```
xllm --resume [--run <id>] [message] [--max-tool-iterations …] [--timeout …]
XllmRun::resume(store, run_id | 工作目录最近的未完成 run, limits, deps) -> ResumeStart
    record = store.read_record(id)
    record.status 终态                               ⇒ ResumeStart::Terminal(record)（只回显，不接受新 limits）
    status Running 且 .lock 仍被持有                  ⇒ RunBusy
    record.version != 7 | prompt.protocol_version != "xllm/2" ⇒ NotResumable
    hosted 门禁：workspace_binding（要 Session 宿主驱动）/ 报告待提交 / 最终 Step 待提交 /
                 host_commit_pending（输入批次未提交）/ handover（等宿主切 behavior） ⇒ NotResumable
    workdir 不存在                                   ⇒ NotResumable
    取锁；持锁后重读 record 并重做上述门禁
    runtime = deps.runtime | RuntimeRegistry::from_config(record.config.runtime)
    manager = rebuild_toolset(record, deps)          // 按 run.json 记的工具来源重建（spec 对账见冻结 TODO F6）
    套用本次命令的 limits（timeout / max_tokens / max_tool_iterations；已消耗的工具迭代沿用）
    llm 同 start
    snapshot = latest_snapshot_idx 对应的快照（最后一次工具调用前 / 推理前 / outcome 边界的 checkpoint）
    call = exec_tracking::in_flight_call(snapshot)   // 第一个没有结果的调用：上一个执行者可能已开始它
        ⇒ manager.set_recovering(call_id, runtime.describe_interrupted(binding, call))   // 再派发时按 call_id 找回（§11.3）
    waist_deps 同 start
    有快照：
        started_at_ms = now（本次命令重新计时；§3.1 挂起时间不计入预算）
        suspended None                 ⇒ ctx = LLMContext::resume(snap, ResumeFromMidRun, waist_deps)
        Some(上下文上限)                ⇒ limit_snapshot = snap（execute 先压缩再跑）
    无快照（首个 Round 前崩溃）         ⇒ 从 record.pending_input 取回初始输入，execute 时 build_initial_context
    status = Running，pid = 本进程；write_record
    → ResumeStart::Run(XllmRun)
    CLI 带 message ⇒ run.inject_input(message)：ctx.inject(user 消息) 接在快照停下的边界，SnapshotHook.commit；
                      停在批次中间（InjectionPosition::None）⇒ 报错，先不带消息 resume
    → execute()（同 §11.2）
```

### 11.5 hosted run：libopendan 装配、xllm 执行

```
record.host = Some(HostRunInfo)：提示词与输入来自宿主，xllm 接手时不重新装配
    prepare_hosted(...)      xllm 只在宿主 system 文本后追加 capabilities / cmd_manual / runtime_protocol 段（可关），
                             runtime_protocol 照算照记进 run.json
    hosted_request(...)      与 build_initial_context 同一套 tool_policy / budget；owner = {"kind":"agent","session_id":…}；
                             json ⇒ Json { schema, strict: true, max_repairs: Some(1) }（回复契约，§3.4）
    宿主再改（runner/live.rs）冻结 behavior 的预算 / fallbacks 覆盖到 request；host_protocol = Session
                             （"仍在运行"提示按 TaskFollowup::Input 措辞：结局作为输入回来，不必等）
    waist_deps               宿主的 SessionCheckpointHook（before_tool_call：lease + 工作区准入 + 存快照 + 同步报告；
                             before_inference：同上 + 路由控制输入 + 注入 <<background_context>>；review_decision：
                             审核交给 Todo 执行者的交接）替代 xllm 的 SnapshotHook；渲染器 without_timestamps
    Outcome 解释             Done 的 report_end / handover 不在 xllm 内完成，记 Paused 交给 Session；
                             本 run 登记了 call_behavior ⇒ 这一段结束时交接给子 context（process_stack Caller 帧）；
                             让出、或本 run 启动的任务 / 子 session 未结束 ⇒ 保留 run（§9.5 "等待在宿主"），
                             结局作为输入注入同一快照后再 run()
```

### 11.6 这个骨架里的接缝（现状，不是结论）

- 接手时工具 spec 来自重建后的 manager，而不是 `run.json` 保存的 spec（冻结 TODO F6）。
- 工具结果里的提示没有去重：同类结果每次都带同样的跟进提示，AgentToolResult 还没有 `hints` / `dedup` 字段，Runtime 也不按 session 去重（冻结 TODO F11）。

---

## 12. 模块布局

```
src/frame/llm_context/src/
│   ├── lib.rs                 // pub use
│   │
│   ├── request.rs             // LLMContextRequest / ModelPolicy / ToolPolicy /
│   │                          //   OutputSpec / BudgetSpec / ErrorPolicy
│   ├── outcome.rs             // LLMContextOutcome / ContextOutput /
│   │                          //   ContextRunTrace / ResumeFill / BudgetKind / ContextLimitKind
│   ├── observation.rs         // Observation / ToolExecRecord / ToolExecStatus
│   ├── error.rs               // LLMComputeError / ProviderFailure / CheckpointStage
│   ├── interrupt.rs           // LLMContextInterruptHandle / InferenceAbortToken /
│   │                          //   InferenceAbortTrace
│   ├── deps.rs                // LLMContextDeps + LlmClient / ToolManager / ToolCallCtx /
│   │                          //   PolicyEngine / WorklogSink / Tokenizer /
│   │                          //   CheckpointHook / Injection / OutputValidator
│   ├── state.rs               // LLMContextState / LLMContextSnapshot /
│   │                          //   Suspension / ToolBatch / ActionStep
│   ├── suspension.rs          // ResumeFill 校验与应用 / inner_transcript_of / strip_thinking
│   ├── context_window.rs      // ContextLimits / 请求 token 估算
│   ├── json_check.rs          // 回复契约的 JSON Schema 子集校验（§3.4）
│   ├── context_derive.rs      // derive_child / fork_snapshot / ForkOptions / InheritBoundary（§3.1）
│   │
│   ├── context_loop.rs        // LLMContext::{new, run, resume, snapshot, inject,
│   │                          //   interrupt_handle}; run_inner + run_behavior
│   ├── behavior_loop.rs       // StepRecord / LLMBehaviorResult / LLMResultParser /
│   │                          //   StepRenderer / StepDecision / DecisionVerdict
│   │
│   └── tests/                 // protocol.rs：测试用的最小 behavior 协议
```

`behavior_loop` 只有类型与 trait：Behavior Loop 是 waist 内置的**通用执行模式**（双中立性下成立），不是 Agent 专属字段，具体协议由宿主注入。waist 外的相关模块：

| 模块 | 位置 | 角色 |
|---|---|---|
| XML 协议 parser / renderer | `agent_tool::xllm::XllmActionParser`、`agent_tool::step_record::XmlStepRenderer`、`agent_tool::xml_util` | Runtime 层 |
| 结果信封视图 | `agent_tool::result_view::{ToolResultView, ToolResultStatusView}`（`from_host` / `into_host`） | Runtime 层 |
| 任务模型 | `agent_tool::task_model`（`RunningTaskResolver` / `TaskState` / `TaskFollowup` / `task_state_observation_via` / `wait_until_ms`），实现在 `tasks.rs` / `tasks_zone.rs` | Runtime 层（§3.3） |
| 恢复时的在途调用 | `agent_tool::exec_tracking::in_flight_call` | Runtime 层（§9.3） |
| 模板引擎 | `agent_tool::prompt_engine` | Runtime 层共用 utility（§7） |
| MsgObject ↔ AiMessage | `libopendan::msg_parser` | 宿主（[Agent Message](<Agent Message.md>)） |

---

## 13. 姊妹文档

| 文档 | 角色 |
|---|---|
| [readme](readme.md) | 术语（Round / Step / Turn）、AgentSession 状态机、Agent Loop 伪代码与消息结构 |
| [llm_context_behavior_loop](llm_context_behavior_loop.md)、[why behavior loop](<why behavior loop.md>) | Behavior Loop 的扩展方案与论证 |
| [append-only history](llm_context_append_only_history.md) | 历史只追加、重写只经 ResumeFill 的纪律 |
| [AgentRuntime 设计](<AgentRuntime 设计.md>) | `agent_tool::runtime`：执行体、工具集装配、恢复时解释在途调用 |
| [xllm Rust SDK](xllm_rust_sdk.md)、[local_llm_context_protocol](local_llm_context_protocol.md) | §11 的细节：run 目录、RunRecord、CLI 协议 |
| [agent_tool_result_protocol](agent_tool/agent_tool_result_protocol.md) | AgentToolResult ↔ `Observation.host` / ToolResultView 的映射与渲染分级 |
| [prompt_render_engine](prompt_render_engine.md) | `agent_tool::prompt_engine` 模板引擎（§7） |
| [Agent Actions](<Agent Actions.md>) | Behavior 模式 `<actions>` / `<report>` 的最终形态 |
| [Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md) | libopendan Session 宿主：Turn、lease、历史重写、hosted run |
| [llm_context 简化与冻结 TODO](../../notepads/llm-context-freeze-todo.md) | 分层取舍的裁决记录 F1–F24；未落地的 F6 / F11 见 §11.6 |

早期设想的 `LLMAgentContext 设计.md` / `LLMWorkflowContext 设计.md` 没有单独成文：Agent 一侧由 libopendan 的 Session 承担，Workflow 一侧尚无实现。

---

## 14. 一句话总结

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
- token 分价规则（input vs output、cached vs uncached）—— waist 只暴露 `AiUsage` 的归一化计量字段 + `AiResponse.cost`。
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
- worklog sink 具体实现 / tool 调用录像 / replay / 任务队列后端 / tool 并发限流 / 熔断 —— effect 实现层私事。
- **loop 内等待与任务语义**（挂起等工具结果、task_id、后台任务提示、ask-user 等待）—— 拒绝。loop 里不允许等待，任何等待都是任务：调用内的等待预算属于 Runtime（§3.3），loop 外的跟进与把结局作为输入带回属于 Session（§9.5）。
- **临时注入**（只进本次请求、不进历史与快照的内容）—— 拒绝。每轮变化的内容破坏前缀缓存，恢复后模型看到的也与崩溃前不同；宿主要放进上下文的内容走持久的 `Injection`（§9.4）。
- **崩溃恢复时的重放判断** —— 拒绝进 waist。waist 只把没有结果的调用以同一 call_id 再派发，找回 / 重跑 / 判定结果未知由 ToolManager 按 call_id 决定（§9.3）。
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
- Prompt / 模板编译的具体策略 —— 各 L4 `LLM*Context` 的 prompt 编译器（可以复用 `agent_tool::prompt_engine`，也可以自己写，§7）。
- Agent 的角色 / 行为 / 状态机定义 —— 各 Agent 框架自己的运行时文档。

### 使用此清单

每次有人提议向 LLMContext 添加新字段或新方法：

1. **先查 A**：是不是已经被显式列为 Non-Goal？是的话直接拒绝并指向已有条目。
2. **过双中立性测试**：scheduler 中立？provider 中立？任何一项不通过即拒绝。
3. **过完两个测试且不在 Non-Goals 里**，仍要在 PR 描述里说明 *"为什么必须进 waist 而不是上下游某层"*。
4. **被拒绝的提议补进 A 对应小节**，标注 PR 链接和拒绝理由。
5. **同意进入 waist 的字段，必须同步更新 §3 / §4 / §5 / §12**，并在 changelog 里登记 waist 版本。一旦进入，移除等同于 breaking change。

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
| `AiResponse` / `AiUsage` / `AiCost` | `buckyos_api::*`（`aicc_client.rs`） |
| Provider 内部 request 类型（不进 waist） | `buckyos_api::AiMethodRequest`，由 aicc 路由层使用 |

aicc 是 BuckyOS 统一 LLM 调用 / 路由层（`src/frame/aicc`）。waist 不感知 aicc 的存在；参考实现的 `LLMContextDeps.llm` 通常是包了 `AiccClient` 的 adapter。

### B.2 OpenDAN behavior 拆解的映射

LLMContext 起点是把 OpenDAN 既有 agent 主循环里的"一次智能执行"拆出来。被替换的 OpenDAN 模块（`src/frame/opendan`）：

| OpenDAN 概念 | 在 LLMContext 里的归属 |
|---|---|
| `LLMBehavior::run_step(input)` | 被 `LLMContext::run`（Behavior 模式）取代 |
| `LLMBehaviorDeps` | 被 `LLMContextDeps` 取代 |
| `BehaviorExecInput`（含 `role_md / self_md / behavior_prompt / input_prompt`） | 在 L4 prompt compiler 里展开成 `AiMessage` 后灌入 `LLMContextRequest.input`（§7） |
| `BehaviorLLMResult`（`reply / actions / next_behavior / set_memory / new_work_session / shell_commands`） | 拆成两部分：通用结构（`do_actions / next_behavior / observation / thought / assistant_text`）进 waist 的 `LLMBehaviorResult`；Agent 专属字段（`set_memory / new_work_session / shell_commands`）通过 `OutputSpec::Json` schema 由 `LLMAgentContext` 自己 deserialize |
| `TokenUsage`（opendan 自定义） | 删除，统一用 `AiUsage` |
| `LLMTrackingInfo` | 拆分：provider 侧 → `AiResponse`；waist 侧 → `ContextRunTrace` |
| `AgentSession` | 仍是 Agent 一侧长上下文持有者；它给 L4 提供 prompt sources，不出现在 waist 公开字段 |
| `AgentToolManager` | 实现 waist 的 `ToolManager` trait |
| `PromptBuilder` | 上提到 L4：宿主自己的 assembler（libopendan `SessionAssembler`），模板用 `agent_tool::prompt_engine` 渲染 |

**核心定位**：在 OpenDAN 语境下，LLMContext 是对 `LLMBehavior::run_step_inner` 那段循环的**重新切片与重新封装**，加上 owner 抽象、cooperative yield（context limit）、preemptive interrupt、显式 budget / output spec。Behavior 模式不是被删除而是被拆成"waist 内的 step 调度 + parser/renderer trait"和"waist 外的 Agent 状态机"两部分——**当前 Behavior 的结束和下一状态选择，仍然是 LLM 在结构化输出中显式表达的意图**。

Agent scheduler 承接旧 `BehaviorEngine`：根据 `Done.behavior_result.next_behavior` 推进状态机，根据 `do_actions` 调度外部动作，根据 `WAIT_USER_MSG` 等 sentinel 管理 session 等待态。

当前实现中，这个 scheduler 是 libopendan 的 `SessionRunner`：它解释 Outcome、决定 Session Turn 的开始 / 继续 / 结束，并把 run 历史写入 Session worklog（见 [readme](readme.md)、[Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md)）。`src/frame/opendan` 的旧 Runtime（`AgentSession`、round history 等）只做了共享 API 的编译适配，按新抽象重构待下一阶段 opendan 重构接入。

### H4 报告校验与恢复边界（2026-10-10）

XML parser（`agent_tool::xllm::XllmActionParser`）输出报告字段 `self_report` / `report_end` / `report_result`。`end` 精确为 true / false，重复 report / end / next_behavior、空最终报告、结束与动作或调度同现均反馈纠错。可选兄弟节点 artifacts 为 JSON 字符串数组、result 为任意 JSON；parser 把 artifacts 并进 `report_result` 的 `artifacts` 键（`REPORT_ARTIFACTS_KEY`，宿主用 `report_artifacts` / `split_report_result` 取回），此时 result 必须是 JSON 对象，`null` 视为没有 result。waist 只传递内容，业务校验由宿主承担。

`CheckpointHook::validate_report(snapshot, result, step_index)` 在报告状态更新前验证并提交；`CheckpointStage::BeforeToolCall` 在工具副作用前持久化 assistant 调用批次。Function-call 的 report 工具由 Session 注入，LLMContext 不解析报告业务含义。结束提交接受后宿主请求 finish，回执配齐后完成交付，Settled 本身不是成功或停止的统一判据。恢复快照的最终 report Step 直接产生 Done；宿主按 journal 身份生成一次最终消息，与正常执行使用同一正文和产物引用。
