# LLM Context 设计

> 本文是 OpenDAN Agent 框架的第一篇文档。读完本文，你应该掌握：
>
> 1. 为什么需要 LLMContext，它和 `llm.complete` / `agent.sendMsg` 的差异
> 2. 它的心智模型（"LLM 进程上下文"）和六态退出
> 3. 它和 Behavior Loop / Prompt 管线 / Tool 调度 / Snapshot 之间的关系
> 4. 写新 scheduler / provider / tool 时哪些字段进 waist、哪些不该进

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

**重要约束：cooperative yield 与 preemptive interrupt 是两条独立控制面。** `PendingTool` / `ContextLimitReached` 在 inference 完成后产生；`Interrupted` 在 inference 过程中由外部触发。"等待用户下一条消息"不属于 waist 挂起态，由 L4 / session 解释（典型 sentinel `next_behavior == "WAIT_USER_MSG"`）。

### 1.3 Loop 不变量：intent → effect → observation

LLMContext loop 不变量**不是** function call → tool result，而是：

```
intent → effect → observation → intent → effect → observation → ... → terminal
```

| 概念 | 在 waist 里的载体 | 谁产生 | 谁消费 |
|---|---|---|---|
| **intent** | `OutputSpec::Json` 解析出的结构化产物（典型字段 `tool_calls / do_actions`）或 provider-native tool_calls | LLM | waist 主循环 → ToolManager |
| **effect** | `ToolManager::call_tool` 内部的实际动作 | ToolManager（effect 实现层） | 外部世界 |
| **observation** | `Observation::{Success \| Error \| Pending \| Cancelled}` | ToolManager | waist 主循环 → 喂回下一轮 LLM |

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
    /// fill 的形态必须与产生 snapshot 时的挂起态对应；
    /// 不一致会返回 LLMComputeError::SnapshotCorrupted。
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
    /// PendingTool ⇒ 把 deferred 工具的执行结果填回；results.len() 必须等于
    /// snapshot.pending_tool_calls.len()，且 call_id 一一对应。
    ToolResults { results: Vec<(String, Observation)> },

    /// ContextLimitReached ⇒ 把重整后的对话历史填回。如何重整（summarize /
    /// drop oldest / hierarchical recall / 换模型）完全由 scheduler 决定。
    RewrittenHistory { history: Vec<AiMessage> },

    /// 运行中崩溃 / interrupt 后恢复 —— snapshot 不是任何挂起态的产物，
    /// 而是 outcome 边界（或 TurnHook 触发的轮前）落盘的中途快照。
    /// 没有 payload；resume 时校验 pending_tool_calls 必须为空，
    /// 否则返回 SnapshotCorrupted。
    ResumeFromMidRun,
}
```

### 3.2 LLMContextRequest

不可变输入。消息载体**直接复用 provider 抽象层的 `AiMessage`**（见 §5），waist 不再造一份 `ChatMessage`。

```rust
pub struct LLMContextRequest {
    pub owner: ContextOwnerRef,          // Agent(session_id) | Workflow(...) | OneShot(id) | Other
    pub trace: Option<String>,           // 调试 trace id
    pub objective: String,               // 自然语言目标，供 worklog 阅读，不进 prompt
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
    pub max_rounds: u32,                 // 0 ⇒ 禁止 tool loop
    pub max_calls_per_round: u32,
    pub max_observation_bytes: u32,
    pub parallel: bool,                  // 默认 false（串行）
    pub allow_deferred: bool,            // 是否允许 Pending(call_id)
}

pub enum Observation {
    Success { call_id, content: Value, bytes, truncated },
    Error   { call_id, message },
    /// effect 层声明"异步，结果将通过外部回调喂回" → Outcome::PendingTool
    Pending { call_id },
    /// 仅允许 session 层 interrupt pending tool 时通过 ResumeFill::ToolResults 注入
    Cancelled { call_id, reason },
}

pub struct PendingToolCall {
    pub call: AiToolCall,                // name + args + call_id 三件套
    pub eta_ms: Option<u64>,
}
```

工具执行委托给 `ToolManager` trait，policy gate 委托给 `PolicyEngine` trait。waist 不知道实现细节；`Observation::Pending` 路径在当前 v1 实现里尚未闭环（`allow_deferred=true` 时会返回 "deferred tool path not yet implemented" 的 error），但语义已在 outcome 层就位。

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
}

pub enum ContextThreshold {
    Ratio { value: f32 },             // 已用 token / provider window，0.0~1.0
    AbsoluteTokens { value: u32 },
}
```

- **`max_total_tokens`** 是预算红线 → 触发 `BudgetExhausted`（终态，OOM kill）。
- **`context_yield_threshold`** 是预警阈值 → 触发 `ContextLimitReached`（挂起态，page fault yield 给 swap）。

两者可以同时设置：前者必须 fail，后者可以被 scheduler 重整后 resume。

### 3.6 ErrorPolicy

```rust
pub struct ErrorPolicy {
    /// 连续 Recoverable 错误超限后升级为终态 Error，防止"调错 → 看到 → 再调错"死循环。
    pub max_consecutive_errors: u32,   // 默认 3
}

pub enum ErrorClass {
    Recoverable(LLMComputeError),     // 喂回 observation，下一轮 LLM 自我修复
    Fatal(LLMComputeError),           // 直接走 Outcome::Error 终态
}
```

| 错误来源 | 默认 Class |
|---|---|
| LLM 输出格式错误 / JSON schema 校验失败 | Recoverable |
| 工具参数错误 / 执行错误 / PolicyEngine 拒绝 | Recoverable |
| Provider 临时不可用（容错层兜底失败后上抛） | Recoverable |
| Provider 永久错误（鉴权 / 模型 ID 错） | Fatal |
| Snapshot 损坏 / call_id mismatch | Fatal |

**纪律**：
- Fatal 不可被 ErrorPolicy 改写。
- run 中被 `InferenceAbortToken` 触发的 cancelled 不走 ErrorPolicy，收敛到 `Outcome::Interrupted`。
- Recoverable 错误喂回的 `AiMessage` 形态由 effect 层决定（waist 只规定 role ∈ {tool, system}）。
- Provider retry / 退避 / fallback chain 都在 adapter 内部，**waist 自己绝不在外面再做一层 retry**。

---

## 4. Outcome：六态退出

> "显式大于隐式"原则在 Outcome 设计上的硬约束：任何让 LLMContext 无法继续推进、但又不构成"失败"的情况，**都必须显式建模为挂起态**，而不是藏在 `Done` 或 `Error` 里。

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
    /// 终态：异常
    Error { error: LLMComputeError, usage: AiUsage },
    /// 终态：预算红线击穿
    BudgetExhausted { which: BudgetKind, partial: Option<ContextOutput>, usage: AiUsage },

    /// 挂起态：等待 deferred 工具回填
    PendingTool {
        pending: Vec<PendingToolCall>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },
    /// 挂起态：接近 / 撞到 context window —— waist 只暴露"事实信号"，
    /// 具体压缩策略由 scheduler 在 resume 时通过 RewrittenHistory 决定。
    ContextLimitReached {
        which: ContextLimitKind,          // ApproachingWindow | HardLimit | ProviderRefused
        usage: AiUsage,
        accumulated: Vec<AiMessage>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },
    /// 挂起态：run 中被外部 interrupt 抢占。
    /// snapshot 是本轮 inference 前的状态；半截 assistant token / tool call 不进入 accumulated。
    Interrupted {
        reason: String,
        usage: AiUsage,
        snapshot: LLMContextSnapshot,
        abort: InferenceAbortTrace,
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
| `ContextLimitReached` | page fault → 等 swap | 是 | `ResumeFill::RewrittenHistory` |
| `Interrupted` | external interrupt | 是 | `ResumeFill::ResumeFromMidRun` |

### 4.2 上层如何处理

| Outcome | Agent scheduler | Workflow engine |
|---|---|---|
| `Done` | 反序列化 Behavior 结果，按 `next_behavior` 切状态 | 写入 node output，进入下一节点 |
| `PendingTool` | session 进入"等事件" | workflow 挂起，pending 排到任务队列 |
| `ContextLimitReached` | 调用自家长期记忆 summarize 后 resume | 一般 fail-and-escalate，或换大窗口模型重跑 |
| `Interrupted` | 停止当前生成，保留 snapshot 待稍后 ResumeFromMidRun | 取消当前 node 执行 / 按策略重调度 |
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
| `AiUsage` | `input_tokens`, `output_tokens`, `total_tokens` | 各 outcome 的 `usage` |
| `AiCost` / `AiArtifact` | — | 嵌在 `AiResponseSummary` 里，waist 不单独暴露 |

**为什么不再包一层**：零成本序列化路径；任何上层 scheduler 拿到 `Done.response` 就已经是 provider-agnostic 的归一化结构；换 provider 实现时 waist 完全不动。

---

## 6. Behavior Loop（在 waist 内一等公民）

Behavior 模式是 Agent 一侧最常见的 L4 语义，但因为它能完整覆盖在双中立性下成立的"step → step"调度协议，被作为 waist 的**可选执行模式**实现。它**不是** Agent 专属字段进入 waist —— 而是把"reply / observation / thought / action / next_behavior"这套结构化输出和分步沉淀做成一组 trait，让 Agent / Workflow / Eval 都能用。

### 6.1 模型

```
   ┌──────────── Behavior Loop（外层）─────────────┐
   │                                                │
   │   step 1 ──┐                                   │
   │   step 2 ──┤  全部沉淀到 LLMContextState.steps │
   │   step 3 ──┘                                   │
   │   ────────────────────────────                 │
   │   last_step（hot）：当前正在处理的最新一步     │
   │                                                │
   │   每次外层迭代：                                │
   │     1. render(history) + render(last_step)     │
   │        → 拼成下一次 inference 的 messages       │
   │     2. 调 run_inner 跑一次内层（传统 Loop）     │
   │     3. parser.parse(response) → BehaviorResult │
   │     4. 若 next_behavior == Some(_) ⇒ 终态        │
   │     5. 否则 dispatch action → fill action_result│
   │        → 沉淀 last_step 到 steps，继续          │
   │                                                │
   └────────────────────────────────────────────────┘
```

### 6.2 关键 trait（`behavior_loop.rs`）

```rust
pub struct StepRecord {
    pub assistant_text: String,
    pub observation:    Option<String>,
    pub thought:        Option<String>,
    pub action:         Option<AiToolCall>,         // v1：每步最多一个 action
    pub next_behavior:  Option<String>,             // Some(_) ⇒ 终态，action 不再 dispatch
    pub action_result:  Option<Observation>,        // executor 填
}

pub struct LLMBehaviorResult {
    pub do_actions:    Vec<AiToolCall>,
    pub next_behavior: Option<String>,
    pub assistant_text: String,
    pub observation:   Option<String>,
    pub thought:       Option<String>,
}

pub trait LLMResultParser: Send + Sync {
    fn parse(&self, response: &AiResponseSummary) -> Result<LLMBehaviorResult, String>;
}

pub trait StepRenderer: Send + Sync {
    /// 一步沉淀回去 = 一对 (assistant, user)，严格角色交替
    fn render(&self, step: &StepRecord) -> (AiMessage, AiMessage);
    fn render_history(&self, steps: Vec<StepRecord>) -> Vec<AiMessage> { /* default */ }
}

#[async_trait]
pub trait HistoryCompressor: Send + Sync {
    async fn compress(
        &self,
        steps: Vec<StepRecord>,
        budget: CompressBudget,
    ) -> Result<Vec<StepRecord>, CompressError>;
}
```

### 6.3 装配位置

Behavior Loop 通过往 `LLMContextDeps` 注入 trait 实例打开：

```rust
pub struct LLMContextDeps {
    // ... 通用依赖 ...
    pub result_parser:      Option<Arc<dyn LLMResultParser>>,
    pub step_renderer:      Option<Arc<dyn StepRenderer>>,
    pub history_compressor: Option<Arc<dyn HistoryCompressor>>,
}
```

- `result_parser = None` ⇒ **传统 Agent Loop**：accumulate AiMessage，直到 LLM 不再调工具。
- `result_parser = Some(_)` ⇒ **Behavior Loop**：每轮跑 `run_inner` + parse + 沉淀，直到 `next_behavior == Some(_)`。
- Construction-time invariant：`result_parser.is_some() ⇒ step_renderer.is_some()`（否则 `LLMContext::new` panic —— 这是 misconfiguration，不是 runtime error）。

参考实现：`XmlBehaviorParser` / `XmlStepRenderer`（XML 协议），位于 `xml_behavior.rs` / `step_record.rs`。

### 6.4 Behavior 模式下的 Outcome

- **终态 `Done.behavior_result: Some(_)`**：parser 解出 `next_behavior == Some(_)` 即终止，action（如有）不 dispatch，由 L4 / session 解释 `next_behavior` 字符串（含 `WAIT_USER_MSG` 这类 sentinel）。
- **挂起态语义不变**：tool 等 deferred、撞到 context window、外部 interrupt 都按 §4 走。
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

## 8. Inference Interrupt（节省生成 token）

`run()` 一旦返回，当前 inference 已结束；再说"中断"已经太晚。`LLMContextInterruptHandle` 是一条独立的 preemptive 控制面，允许 scheduler 在 `run()` 尚未返回时抢占当前 provider inference。

```rust
#[derive(Clone)]
pub struct LLMContextInterruptHandle { inner: Arc<InferenceAbortState> }

impl LLMContextInterruptHandle {
    pub fn interrupt(&self, reason: impl Into<String>) -> bool;
}

#[derive(Clone)]
pub struct InferenceAbortToken { inner: Arc<InferenceAbortState> }

impl InferenceAbortToken {
    pub fn is_aborted(&self) -> bool;
    pub async fn cancelled(&self);
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
2. 每轮 inference 前先回调 `TurnHook`（§9），再构造携带 `InferenceAbortToken` 的 `LlmInferenceRequest`。
3. waist 把 `provider.infer()` future 和 `abort_token.cancelled()` 跑 `tokio::select!`，即便 adapter 完全忽略 abort，scheduler 线程也能立刻释放。
4. provider adapter 应把 abort 映射到底层 HTTP / SDK cancel；不支持远端 cancel 时丢弃 late response（仍能尽早释放本地）。
5. 收到 cancelled 后返回 `Outcome::Interrupted`，snapshot 是**本轮 inference 发起前**的状态——半截 token / 半截 tool call 不进入 accumulated。
6. resume 时用 `ResumeFill::ResumeFromMidRun`：context 从本轮 inference 前重新推进。

`Interrupted` 与 cooperative yield 的边界：

- `PendingTool` / `ContextLimitReached` —— LLM 完成一次 inference 后让出 CPU。
- `Interrupted` —— scheduler 在 inference 过程中抢占 CPU。
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
    pub accumulated: Vec<AiMessage>,
    pub usage:       AiUsage,
    pub rounds_left: u32,
    pub started_at_ms: u64,
    pub cost_units:    u32,
    pub consecutive_errors: u32,
    pub pending_tool_calls: Vec<PendingToolCall>,
    pub llm_task_ids: Vec<String>,
    /// Behavior Loop：沉淀的历史 steps（可被 HistoryCompressor 压缩）
    pub steps: Vec<StepRecord>,
    /// Behavior Loop：最新一步（hot），下一轮 verbatim render
    pub last_step: Option<StepRecord>,
}
```

**硬约束**（不是建议）：

- **自包含**：给定 snapshot S 在节点 A 上产生，节点 B 只要提供等价 `LLMContextDeps`，就必须能成功 `resume(S, fill, deps)`。
- **跨节点可序列化**：建议 < 32KB；超出时调用方走外部存储 + 在 snapshot 里只放引用 ID。
- **不持有 effect-side 真实世界状态**：snapshot 只持有逻辑层产物（call_id / observation / accumulated / step / usage），**绝不**持有 tmux 句柄、fd、network connection、容器 PID。
- **input 已展开**：跨节点 resume 不依赖任何模板环境。模板展开发生在 L4。

> **工程提醒**：如果你想往 snapshot 里塞"句柄""指针""长生命态引用"，停下来——那些东西属于 `LLMContextDeps`（resume 时重新提供），不属于 snapshot。

### 9.2 TurnHook（可选 deps 扩展点）

```rust
pub trait TurnHook: Send + Sync {
    /// 每次 LLM inference 之前同步回调。snapshot 是 LLMContext 的完整冻结。
    /// 必须 fast / 不可 panic / 不可修改 snapshot。
    fn before_inference(&self, snapshot: &LLMContextSnapshot);
}
```

- 不注入也合法。注入后 L4 可在每轮推理前落盘，做到"重启不重复扣已付费推理"。
- snapshot 落到哪 / 加密 / 压缩 / 归档全部是 effect 层私事（§A.4）。
- 各 L4 调用频率诉求不同：OneShot 每次都落、workflow 节点采样、agent 长会话按轮数采样——scheduler 政策，waist 不裁决。

### 9.3 崩溃恢复流程（L4 持久化层用）

```
[Process A]                                  [Process B (after crash)]

ctx = LLMContext::new(req, deps)
s0 = ctx.snapshot();  sink.persist(s0)       // 启动前落盘
loop {
    // TurnHook 在 inference 前再落一份
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

- L4 必须能区分"崩在挂起态"与"崩在运行中"：前者用 `ToolResults` / `RewrittenHistory`，后者用 `ResumeFromMidRun`。waist 在 resume 里做一致性校验拦截误用。
- "崩在挂起态 + 无外部 fill" 不能用 `ResumeFromMidRun` 兜底——会返回 `SnapshotCorrupted`。
- outcome 边界 snapshot 之后、TurnHook 之前进程崩溃（罕见但存在）会重跑该轮 inference + 工具调用。**ToolManager / provider 的幂等性是 effect 层私事**。

---

## 10. 主循环骨架

```
LLMContext::new(req, deps)
  └─> run().await
        ├─> emit(LLMStarted)
        ├─> if behavior_mode:
        │     run_behavior()                  // §6
        │   else:
        │     run_inner()                     // 下面
        ├─> emit(LLMFinished)
        └─> return outcome

run_inner():
  loop:
    ├─> check wallclock budget → BudgetExhausted?
    ├─> turn_hook.before_inference(snapshot)               // §9.2
    ├─> snapshot_before_inference = snapshot()             // s0 for Interrupted
    ├─> if abort.is_aborted(): return Interrupted(s0)
    ├─> tokio::select!:
    │     - cancelled() → Interrupted(s0)
    │     - llm.infer(req) → response
    │         ├─ provider error after fault-tolerance →
    │         │     handle_error(class) → Recoverable feeds back; Fatal → Error
    │         └─ ok → continue
    ├─> account usage; check token budget → BudgetExhausted?
    ├─> if context_yield_threshold reached: ContextLimitReached
    ├─> if no tool_calls or ToolMode::None: → Done
    ├─> policy.gate_tool_calls() → Recoverable on reject
    ├─> push assistant_tool_call message
    ├─> for each call:
    │     observation = tools.call_tool(call)
    │     match observation:
    │       Pending → (deferred 路径尚未闭环)
    │       Success → push tool message
    │       Error   → feed back as observation, count consecutive_errors
    │       Cancelled → internal error (inline 返回非法)
    ├─> rounds_left -= 1; if 0: BudgetExhausted(ToolRounds)
    └─> next round
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
│   │                          //   PolicyEngine / WorklogSink / Tokenizer / TurnHook
│   ├── state.rs               // LLMContextState / LLMContextSnapshot
│   ├── snapshot_overrides.rs  // RequestOverrides / rebuild_with_inherit / build_fresh
│   │
│   ├── context_loop.rs        // LLMContext::{new, run, resume, snapshot,
│   │                          //   interrupt_handle}; run_inner + run_behavior
│   ├── behavior_loop.rs       // StepRecord / LLMBehaviorResult /
│   │                          //   LLMResultParser / StepRenderer / HistoryCompressor
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
- token 分价规则（input vs output、cached vs uncached）—— waist 只暴露 `AiUsage` 三个数 + `AiResponseSummary.cost`。
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
- **上下文压缩策略**（summarize / sliding window / hierarchical recall / drop-oldest / 换模型）—— 拒绝进 waist。waist 只暴露 `ContextLimitReached` 这个事实信号，策略属于 scheduler 在 resume 时通过 `RewrittenHistory` 提供。Behavior Loop 的 `HistoryCompressor` 同理：是注入的 trait，不是字段。
- **错误归一化的 wire format**：错误 message 字段结构、是否带 stack trace / hint、人类可读 prompt 措辞，都是 effect 层与对应 L4 的协议。waist 只规定 Recoverable 错误必须以合法 `AiMessage` 形态（role ∈ {tool, system}）进入 accumulated。
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
