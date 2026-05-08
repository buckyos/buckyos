# LLM Context 设计

> Status: Draft
> Owner: OpenDAN Runtime
> Related: `OpenDAN Agent Runtime 设计.md` / `Agent Session.md` / `Agent Prompt Compiler.md` / `Agent Worklog.md`

---

## Preamble — Design Stance: Narrow Waist

> 本节是整个文档**最高优先级的约束**，凌驾于所有具体设计之上。后续 §0 起的所有内容都是这条纪律的展开。

LLMContext 是一个 **narrow waist primitive（瘦腰原语）** —— 它的地位类比 IP 包之于互联网、POSIX file 之于 OS、LLVM IR 之于编译器、USB 之于外设。瘦腰不是设计目标，是**设计纪律**：一旦认定某个东西要做瘦腰，所有后续讨论的检验标准就变了。

### 判据：双向中立性

每个候选字段 / 方法 / 行为，必须**同时**通过下面两个测试，才能进入 LLMContext：

1. **Scheduler 中立**：把 Agent / Workflow / Shell / Hook / Pipeline / Eval / Multi-agent debate 任何一种上层调度器换上来，这个字段都同样自然。如果偏向某一种，它属于**上面那层**。
2. **Provider 中立**：把底层 LLM 换成 Claude / GPT / Gemini / 本地模型，工具实现换成不同 sandbox / MCP server，这个字段都同样自然。如果偏向某一种，它属于**下面那层**。

任何一项不通过，**默认拒绝进入 waist**。

### 纪律

- **稳定性 > 完备性**：waist 的 breaking change 会同时打到所有上下游。宁可 waist 少一个字段、让某个 scheduler 自己在外面包一层，也不要把字段塞进 waist 图省事。
- **瘦不下来就不加**：如果一个能力没办法在不破坏中立性的前提下加进 waist，**这是 waist 在拒绝它，不是 waist 不够强** —— 它应该在 waist 上面（scheduler 层）或下面（provider / effect 实现层）找位置。
- **PR review 标准**：每个改动 LLMContext 公共类型的 PR，必须在描述里显式回答"这个改动是否破坏 scheduler 中立 / provider 中立"。无法回答或回答勉强，默认拒绝。回答清楚了再讨论是否合理。
- **Non-Goals 是活清单，只增不减**：见 Appendix A。任何"看似合理但会污染 waist"的提议，决议拒绝后都补进 Appendix A，让后人不必重新讨论。

### 收益所在

真正的回报不在 waist 自身有多优雅，而在 **waist 立住之后上下游各自的 Cambrian explosion**：

- **上面**：Agent / Workflow / Shell / Hook / Pipeline / Eval / Multi-agent 互不知道彼此地各自演化
- **下面**：LLM provider / tool 实现 / sandbox / memory backend 互不知道上面地各自演化

这种**双向解耦**是瘦腰原语贡献的全部价值。LLMContext 自己越薄，上下游能长出来的东西越多。

---

## 0. 心智模型：LLM Context as Process Context

> 设计动机的一句话总结：
> **workflow 里直接集成 `llm.complete` 太低阶，集成 `agent.sendMsg` 又太重型。LLMContext 是中间那一层。**

`LLMContext` 不是"传给 LLM 的 messages 容器"，而是 OS 意义上的**进程上下文**（process context / PCB）—— 一段可挂起、可恢复、可被调度器管理的有界 LLM 执行体。整个设计应该按这个心智模型来读。

| OS 概念                       | LLMContext 对应物 |
|------------------------------|-------------------|
| Process Control Block (PCB)  | `LLMContext` 自身：装 prompt 编译产物 + tool loop 中间态 + token usage + 待填的 pending call_id |
| Registers + Stack            | `LLMContextState`：可序列化的运行时可变态 |
| Loaded segments / mmap       | `ContextSources`：context 自己不持有 memory/worklog，按需从外部映射 |
| Yield / context switch out   | `Outcome::PendingTool` / `Outcome::WaitInput`：cooperative yield，LLM 主动让出等 IO（外部 task）或 input（人） |
| Context switch in            | `LLMContext::resume(snapshot)`：调度器恢复挂起态继续跑 |
| Killed by scheduler          | `Outcome::BudgetExhausted`：quantum / 内存 / wallclock 任一耗尽，被回收 |
| exit syscall                 | `Outcome::Done` / `Outcome::Error`：进程正常/异常终止 |
| Scheduler                    | Agent loop / Workflow engine：决定哪个 context 上 CPU、何时回收、被 yield 后由谁负责喂回结果 |
| Process lifetime             | 短生命：一个 LLMContext 对应"一次智能任务"，不是 Agent 的整段会话 |
| Process isolation            | LLMContext 之间不共享可变态，只通过 owner（Agent session / Workflow instance）协作 |

这个心智模型决定了所有后续设计：

- **为什么是对象不是函数**：进程上下文必须有可变 runtime 状态
- **为什么有 5 态退出**：进程要么 exit，要么 yield 等 IO，要么 yield 等 input，要么被 kill —— 不可能只有"返回值"
- **为什么 owner / scheduler 抽象**：scheduler 不关心进程在跑什么业务，只关心生命周期；context 不关心被谁调度，只暴露 yield/resume 协议
- **为什么需要 snapshot**：挂起必须能完整保存执行态以便恢复

**重要约束：所有 yield 都是 cooperative 的**（合作式让出），不是 preemptive（抢占式）。LLM 不会被中途打断；只有 LLM 自己说"我需要这个工具结果 / 我需要人来回答"，或者预算用完，才会让出 CPU。这避免了"在 inference 中途切换"这种破坏 token stream 完整性的语义。

## 1. 背景与动机

### 1.1 直接动机：Workflow 缺中间层

AHL Workflow 接入 LLM 当前只有两个不合身的选择：

- **`llm.complete(prompt)`** —— 太低阶。每个 workflow node 自己拼 prompt、自己实现 tool loop、自己处理 retry/budget/结构化输出/人工节点，最后变成"在 workflow 里重写一个迷你 agent runtime"。
- **`agent.sendMsg(...)`** —— 太重型。强制带上 AgentSession、长上下文、行为机、Jarvis 状态切换，workflow node 只是想"跑一次 LLM + 几个工具"，不需要这一整套。

`LLMContext` 就是这中间缺失的层：**有 agent runtime 的能力（prompt 编译 / tool loop / worklog / budget / pending），但没有 agent session 的长生命语义**。

### 1.2 现状

当前 OpenDAN 的"一次智能执行"语义被绑死在 Agent 主循环里：

- `LLMBehavior::run_step(input: &BehaviorExecInput) -> Result<(BehaviorLLMResult, LLMTrackingInfo), LLMComputeError>`
  （`src/frame/opendan/src/behavior/behavior.rs:70`）
- 入参 `BehaviorExecInput` 强依赖 `session_id` / `SessionRuntimeContext` / `Arc<Mutex<AgentSession>>`
  （`src/frame/opendan/src/behavior/types.rs:52`）
- 出参 `BehaviorLLMResult` 是为 Jarvis 风格行为机准备的：`reply / actions / next_behavior / set_memory / new_work_session…`
  （`src/frame/opendan/src/behavior/types.rs:132`）
- worklog / tool loop / budget / policy 全部在 `run_step_inner` 内一把梭，并且通过 `LLMBehaviorDeps` 的 `worklog: Arc<dyn WorklogSink>`、`policy: Arc<dyn PolicyEngine>`、`tools: Arc<AgentToolManager>` 强绑到 Agent 运行环境。

这套对**"长生命 Agent 的一步"** 是合身的，但对**"AHL Workflow 的一个 LLM 节点"** 不适用：

- Workflow 没有 `AgentSession`，构造一个伪 session 只是为了过类型检查，是反向耦合。
- Workflow 不需要 `next_behavior / new_work_session` 这种 Jarvis 状态机产物。
- Workflow 需要的是：**一次有边界、可复算、可挂起、可重试的 LLM 执行闭包**，结果是结构化的 `output + 状态`，不是 Agent 行为决策。

### 1.3 目标

抽出 `LLMContext` —— **一次性、短生命、可被多种 owner（Agent / Workflow / 人工触发的一次性任务）构造的有界 LLM 执行单元**：

1. **不绑定 Jarvis 行为机**：不强求 `next_behavior` 这种字段，输出 schema 由调用方声明。
2. **不绑定 AgentSession**：通过抽象的 `ContextOwner` / `ContextSources` 注入需要的上下文（memory / 历史 / 模板变量）。
3. **统一的退出语义**：`done / wait_input / pending_tool / budget_exhausted / error` 五态，便于上层（Agent loop / Workflow engine）一致地推进状态机。
4. **统一的可观测性**：worklog、step record、token usage、tool trace 都通过 `LLMContext` 沉淀。
5. **复用现有实现**：`LLMContext` 不重写 LLM 调用与 tool loop，是 `LLMBehavior::run_step_inner` 的**重新切片与重新封装**。

### 1.4 非目标

- 不替换 `AgentSession`：Agent 的长上下文、状态机、行为切换继续由 `AgentSession` 管。
- 不替换 `WorkflowInstance`：Workflow 的 DAG / 人工节点 / 分支由 workflow engine 管。
- 不引入新的 LLM provider 抽象：仍走 `AICC / TaskMgr / AiMethodRequest`。
- 不动 worklog 存储格式：`LLMContext` 是 worklog 的**生产者**，不是新的存储层。

---

## 2. 三层能力分层

```
┌─────────────────────────────────────────────────────────────┐
│  L3  Scheduler 层（OS 类比：进程调度器）                    │
│  - AIAgent (run_agent_loop)        消息驱动，长生命         │
│  - WorkflowInstance (DAG engine)   状态机驱动，长生命       │
│  - OneShotScheduler                一次性脚本/CLI           │
│                                                             │
│   构造 LLMContextRequest（创建进程），根据 Outcome 推进：   │
│   exit / yield / kill —— 即调度器的标准动作                 │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│  L2  LLMContext 层（OS 类比：进程上下文）                   │
│  - 一次有界 LLM 执行：prompt 编译 + LLM 调用 + tool loop    │
│  - worklog 写入 / token & HP 计费 / policy gate             │
│  - 五态退出：done / wait_input / pending_tool /             │
│              budget_exhausted / error                       │
│  - 可 cooperative yield / 可 resume（LLMContextSnapshot）   │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│  L1  Raw LLM 层                                             │
│  - AiccClient / AiMethodRequest                             │
│  - 单次推理，无 tool loop / 无 worklog / 无 policy           │
│  - 适合分类、摘要、结构化抽取等"可一次说完"的任务           │
└─────────────────────────────────────────────────────────────┘
```

L1 已经有了，L3 部分有（`AIAgent`），workflow 那边还没有。**本设计只新增 L2，并把 L1↔L3 的直连改造成 L1↔L2↔L3。** 等价地，把"workflow 直接 syscall（`llm.complete`）"和"workflow 直接 fork-exec 一个完整 agent 进程（`agent.sendMsg`）"中间，加进真正的"进程上下文"层。

---

## 3. 核心抽象

### 3.1 LLMContext

`LLMContext` 是一次执行的**对象化封装**，不是一个静态函数。它持有：

- 不可变输入 `LLMContextRequest`
- 可变运行态 `LLMContextState`（剩余 budget、tool loop 计数、worklog buffer、当前 pending 项）
- 退出态 `LLMContextOutcome`

```rust
pub struct LLMContext {
    request: LLMContextRequest,
    state:   LLMContextState,
    deps:    LLMContextDeps,   // tools / policy / worklog / tokenizer / aicc
}

impl LLMContext {
    pub fn new(req: LLMContextRequest, deps: LLMContextDeps) -> Self;

    /// 主驱动：从当前 state 向前推进，直到产生一个 outcome。
    /// - done / budget_exhausted / error：终态，对象消耗
    /// - wait_input / pending_tool：可挂起态（cooperative yield），state 可序列化为 snapshot
    pub async fn run(&mut self) -> LLMContextOutcome;

    /// 从 snapshot 恢复（context switch in）。
    /// - pending_tool 拿到工具结果后填回，wait_input 拿到 input 后填回。
    pub fn resume(snapshot: LLMContextSnapshot, fill: ResumeFill, deps: LLMContextDeps) -> Self;

    pub fn snapshot(&self) -> LLMContextSnapshot; // 用于 step_record / 审计
}
```

### 3.2 输入：LLMContextRequest

```rust
pub struct LLMContextRequest {
    /// 上层 owner 标识，用于 worklog / tracing / 审计
    pub owner: ContextOwnerRef,           // Agent(session_id) | Workflow(instance, node) | OneShot(id)
    pub trace: SessionRuntimeContext,     // 复用现有 trace 类型

    /// 任务声明
    pub objective: String,                // 自然语言目标，用于 worklog & system prompt
    pub input:     ContextInput,          // 见 3.3，结构化输入

    /// Prompt 来源（已编译的片段或编译指令）
    pub prompt:    PromptSpec,            // 见 3.4

    /// 可用工具与工具策略
    pub tool_policy: ToolPolicy,          // 见 3.5

    /// 输出契约
    pub output:    OutputSpec,            // 见 3.6

    /// 模型策略
    pub model_policy: ModelPolicy,        // 复用 behavior::types::ModelPolicy

    /// 资源边界
    pub limits:    StepLimits,            // 复用 behavior::types::StepLimits
    pub budget:    BudgetSpec,            // 见 3.7

    /// Human-in-the-loop 策略
    pub human_policy: HumanPolicy,        // 见 3.8
}
```

设计要点：**`session: Option<Arc<Mutex<AgentSession>>>` 这种字段不再出现**。Agent 模板变量通过 `ContextSources` 注入（见 3.9），而不是把整个 session 塞进来。

### 3.3 ContextInput

```rust
pub enum ContextInput {
    /// 单段文本（最常见，AHL 的 LLM node、Agent 的当前消息）
    Text(String),
    /// 多模态消息列表
    Messages(Vec<ContextMessage>),
    /// 已结构化输入（workflow node 上一节点的 output）
    Structured(serde_json::Value),
}
```

### 3.4 PromptSpec

```rust
pub enum PromptSpec {
    /// 完全外部已编译好的 prompt（workflow 简单节点 / OneShot 适用）
    Prebuilt {
        system: Vec<ChatMessage>,
        user:   Vec<ChatMessage>,
    },

    /// 走 OpenDAN 的 PromptBuilder，由 ContextSources 提供变量
    Compiled {
        role_md:        Option<String>,
        self_md:        Option<String>,
        behavior_md:    Option<String>,
        last_step_md:   Option<String>,
        sources:        ContextSources,    // memory / worklog / template vars
    },
}
```

只有 `Compiled` 模式才需要 `ContextSources`。Workflow 简单 LLM 节点用 `Prebuilt` 即可，不必装载 OpenDAN 的全套 PromptBuilder。

### 3.5 ToolPolicy

```rust
pub struct ToolPolicy {
    pub mode:      ToolMode,           // None | Whitelist | All
    pub whitelist: Vec<String>,
    pub max_rounds:           u32,     // 0 = 禁止 tool loop（一次推理即返回）
    pub max_calls_per_round:  u32,
    pub max_observation_bytes: u32,
    /// 哪些工具的结果产生 PendingTool，需要由调用方异步喂回
    pub deferred:  Vec<String>,
}
```

工具执行委托给已有的 `AgentToolManager`，policy gate 委托给已有的 `PolicyEngine`。`deferred` 是**新增语义**：声明某些工具的结果不在 LLMContext 内同步等待，而是产生 `Outcome::PendingTool`，由上层（workflow engine 排到 task queue）异步喂回。

### 3.6 OutputSpec

```rust
pub enum OutputSpec {
    /// 自由文本，调用方自己解析
    Text,
    /// 强制 JSON，可校验 schema
    Json { schema: Option<serde_json::Value>, strict: bool },
    /// XML，按声明的 root 标签解析（Agent 的 BehaviorLLMResult 走这条）
    Xml  { root: String, strict: bool },
    /// 直接复用 BehaviorLLMResult（Agent 行为机专用，向后兼容）
    BehaviorLLMResult,
}
```

`OutputSpec::BehaviorLLMResult` 是过渡兼容，让现有 `LLMBehavior::run_step` 能直接改写为 `LLMContext::run` 的 thin wrapper，而 workflow 用 `Json{schema}` 或 `Text`。

### 3.7 BudgetSpec

```rust
pub struct BudgetSpec {
    pub max_total_tokens:     Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub max_wallclock_ms:     Option<u64>,
    pub max_hp_cost:          Option<u32>,   // 复用 AgentConfig 的 HP 模型
    pub on_exhausted:         BudgetAction,  // Fail | ReturnPartial | EscalateHuman
}
```

`StepLimits` 关注**单步硬限制**（deadline / tool round），`BudgetSpec` 关注**整次执行预算**。两者并存。

### 3.8 HumanPolicy

```rust
pub struct HumanPolicy {
    /// 哪些 action 需要人工批准
    pub approval_required: Vec<String>,
    /// 是否允许 LLM 主动请求人工输入（产生 wait_input）
    pub allow_request_input: bool,
    /// 等待人工响应的最长时间
    pub wait_timeout_ms: Option<u64>,
}
```

Agent 长上下文里这个一般是空的（Agent 自己处理交互），workflow 的人工节点会重度使用。

### 3.9 ContextSources / ContextOwnerRef

```rust
pub enum ContextOwnerRef {
    Agent { session_id: String },
    Workflow { instance_id: String, node_id: String },
    OneShot { id: String, label: String },
}

/// Compiled PromptSpec 用，提供 PromptBuilder 需要的变量
pub trait ContextSources: Send + Sync {
    fn render_template_var(&self, key: &str) -> Option<String>;
    fn memory_block(&self, token_budget: u32) -> Option<String>;
    fn worklog_block(&self, token_budget: u32) -> Option<String>;
    fn history_tail(&self, token_budget: u32) -> Option<String>;
}
```

`AgentSession` 实现 `ContextSources`（拿现有 PromptBuilder 那套数据）。
`WorkflowInstance` 实现 `ContextSources`（拿 workflow 上下文里的变量、上游 node output、人工 approval 历史）。
`OneShot` 给一个 `EmptyContextSources`。

这样 `LLMContext` 自身不知道也不关心 owner 是谁，Prompt 编译只通过 trait 调回去取数据。

---

## 4. 输出：LLMContextOutcome

OS 类比：一个进程在一次被 schedule 之后，只可能以下面五种方式离开 CPU。`LLMContextOutcome` 就是这五种 syscall return 的并集。

| Outcome              | OS 对应         | 是否终态 | snapshot 是否产出 |
|---------------------|----------------|----------|-------------------|
| `Done`              | `exit(0)`      | 是       | 否                |
| `Error`             | `exit(非0)`    | 是       | 否                |
| `BudgetExhausted`   | OOM kill / SIGKILL | 是   | 可选（见 partial） |
| `WaitInput`         | `read()` 阻塞  | 否       | 是                |
| `PendingTool`       | `io_submit()` 后等待 | 否 | 是                |

注意 `WaitInput` 和 `PendingTool` 都是 **cooperative yield**：LLM 在 inference 完成后才有机会让出，token stream 不会被切断在中间。这是和 OS preemptive scheduler 的关键区别 —— LLMContext 没有 timer interrupt，只有 LLM 自己说"这一段推完了，我要等外部"。


```rust
pub enum LLMContextOutcome {
    /// 终态：成功
    Done {
        output: ContextOutput,           // 按 OutputSpec 解析后的产物
        usage:  TokenUsage,
        tracking: LLMTrackingInfo,       // 复用现有类型
    },

    /// 暂停：等待人工输入或外部消息
    WaitInput {
        reason:  String,
        prompt_to_human: Option<String>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },

    /// 暂停：触发了 deferred 工具，由上层异步喂回结果
    PendingTool {
        pending: Vec<PendingToolCall>,   // tool_name + call_id + args
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },

    /// 终态：预算/边界耗尽
    BudgetExhausted {
        which: BudgetKind,               // Tokens | Wallclock | HP | ToolRounds
        partial: Option<ContextOutput>,  // BudgetAction::ReturnPartial 时填
        usage:  TokenUsage,
    },

    /// 终态：错误
    Error {
        error: LLMComputeError,          // 复用现有错误类型
        usage: TokenUsage,               // 即使出错也回报已消耗的 token
    },
}
```

### 4.1 上层如何处理

```text
┌───────────────────┬──────────────────────────────────────────┐
│ Outcome           │ Agent loop                  │ Workflow engine
├───────────────────┼─────────────────────────────┼────────────
│ Done              │ 写 step_record，按          │ 写入 node output，
│                   │ next_behavior 切换          │ 进入下一个 node
│ WaitInput         │ session 进入 WAIT_FOR_MSG   │ workflow 挂起，
│                   │                             │ 通知人工节点
│ PendingTool       │ session 进入 WAIT_FOR_EVENT │ workflow 挂起，
│                   │ （兼容现有 long task）      │ 把 pending 排到任务队列
│ BudgetExhausted   │ HP 不足 → END               │ 走 retry / escalation /
│                   │                             │ fail 分支
│ Error             │ 走 error behavior           │ 走 error handler 节点
└───────────────────┴─────────────────────────────┴────────────
```

---

## 5. 与现有抽象的映射

| 现有概念                              | LLMContext 中的位置               |
|---------------------------------------|----------------------------------|
| `LLMBehavior` (struct + run_step)     | 退化为 `LLMContext` 的 thin wrapper（保留 Agent 行为机入口） |
| `LLMBehaviorDeps`                     | 拆为 `LLMContextDeps`（公共部分）+ Agent 专有部分 |
| `BehaviorExecInput.session_id/trace`  | `LLMContextRequest.owner / trace` |
| `BehaviorExecInput.role_md/self_md/…` | `PromptSpec::Compiled` 的字段 |
| `BehaviorExecInput.session`           | **删除**，改为 `ContextSources` trait |
| `BehaviorExecInput.limits/behavior_cfg` | `LLMContextRequest.limits + tool_policy + output + model_policy` |
| `BehaviorLLMResult`                   | `OutputSpec::BehaviorLLMResult` 模式下的 `ContextOutput::Behavior(BehaviorLLMResult)` |
| `LLMTrackingInfo`                     | 原样复用，挂在 `Outcome::Done.tracking` |
| `LLMComputeError`                     | 原样复用，挂在 `Outcome::Error.error` |
| `WorklogSink::emit(AgentWorkEvent)`   | 由 `LLMContext` 内部调用，与现有 worklog 体系完全一致 |
| `PolicyEngine::gate_tool_calls`       | 原样复用 |
| `PromptBuilder::build`                | 仅在 `PromptSpec::Compiled` 时调用 |
| `AgentToolManager::call_tool`         | 原样复用，`deferred` 工具走新分支不调用 |
| `AICC do_inference_once`              | 原样复用 |

**核心结论**：`LLMContext` ≈ 把 `LLMBehavior::run_step_inner`（`behavior/behavior.rs:129-198`）那段循环切出来，加 owner 抽象、加 deferred / pending、加 BudgetSpec、加 OutputSpec 多态。**不引入新的 LLM 调用路径，不引入新的工具执行路径，不引入新的 worklog 通道。**

---

## 6. 关键执行流程

### 6.1 一次同步执行（最常见）

```
LLMContext::new(req, deps)
  └─> run().await
        ├─> compile_prompt()              // PromptSpec → AiMethodRequest
        ├─> emit(LLMStarted)
        ├─> loop:
        │     ├─> do_inference_once()
        │     ├─> if tool_calls.is_empty(): break
        │     ├─> gate by policy
        │     ├─> for call in calls:
        │     │     ├─> if call.tool ∈ deferred:
        │     │     │     return Outcome::PendingTool { ... }
        │     │     ├─> else: tools.call_tool() → observation
        │     │     └─> emit(ToolCallFinished)
        │     ├─> rounds_left -= 1; check budget
        │     └─> rebuild request with tool observations
        ├─> parse output by OutputSpec
        ├─> emit(LLMFinished)
        └─> return Outcome::Done { ... }
```

### 6.2 PendingTool 挂起 / 恢复

```
[Workflow engine]
   ├─ ctx = LLMContext::new(req, deps)
   ├─ outcome = ctx.run().await
   ├─ match outcome { PendingTool { pending, snapshot, .. } => ... }
   ├─ persist snapshot to workflow state
   ├─ enqueue pending calls as separate tasks
   │
   ├─ [task callback returns]
   │
   ├─ ctx2 = LLMContext::resume(snapshot,
   │           ResumeFill::ToolResults(observations), deps)
   └─ outcome2 = ctx2.run().await   // 继续 tool loop
```

`snapshot` 内含：原 `LLMContextRequest`（不变部分，相当于进程的代码段）+ `LLMContextState`（可变部分，相当于寄存器+栈：usage、rounds_left、待填的 pending call_id 列表、tool loop 已累积的 messages）。Snapshot 必须是**自包含的** —— 调度器拿着它跨进程/跨重启都能 resume。

### 6.3 WaitInput

LLM 在 reply 中显式声明需要人工输入（通过 `OutputSpec::Json` 里的 `request_human_input` 字段，或者 `BehaviorLLMResult` 里的 reply）。`LLMContext` 检测到这个信号，构造 `Outcome::WaitInput`。`HumanPolicy.allow_request_input = false` 时降级为 `Outcome::Done`，让上层自己决定。

---

## 7. Agent / Workflow 两个调用方

### 7.1 Agent（向后兼容）

```rust
// behavior/behavior.rs 里现有的 LLMBehavior::run_step 改写为：
impl LLMBehavior {
    pub async fn run_step(&self, input: &BehaviorExecInput)
        -> Result<(BehaviorLLMResult, LLMTrackingInfo), LLMComputeError>
    {
        let req = LLMContextRequest::from_behavior_input(input, &self.cfg);
        let mut ctx = LLMContext::new(req, self.deps.to_context_deps());
        match ctx.run().await {
            Outcome::Done { output: ContextOutput::Behavior(r), tracking, .. }
                => Ok((r, tracking)),
            Outcome::Error { error, .. } => Err(error),
            // 现阶段 Agent 主循环不消费 PendingTool / WaitInput，
            // 这两态由 BehaviorConfig 里 deferred = [] / allow_request_input = false 屏蔽
            other => Err(LLMComputeError::Internal(format!("unexpected: {other:?}"))),
        }
    }
}
```

`AgentSession` 实现 `ContextSources`，把现有 PromptBuilder 需要的 `{{key}}` 模板变量、memory、worklog 通过 trait 暴露出去。这样 PromptBuilder 不再持有 `Arc<Mutex<AgentSession>>`，只持有 `Arc<dyn ContextSources>`。

### 7.2 Workflow

```rust
// AHL workflow runtime 调用方
let req = LLMContextRequest {
    owner: ContextOwnerRef::Workflow { instance_id, node_id },
    objective: node.objective.clone(),
    input:    ContextInput::Structured(upstream_output),
    prompt:   PromptSpec::Prebuilt {
        system: node.system_messages.clone(),
        user:   node.user_messages.clone(),
    },
    tool_policy: node.tool_policy(),
    output: OutputSpec::Json { schema: Some(node.output_schema.clone()), strict: true },
    model_policy: node.model_policy(),
    limits: node.step_limits(),
    budget: node.budget(),
    human_policy: node.human_policy(),
    trace: SessionRuntimeContext::for_workflow(instance_id, node_id),
};

let mut ctx = LLMContext::new(req, workflow_deps.to_context_deps());
match ctx.run().await {
    Outcome::Done { output, .. }     => engine.advance_to_next_node(node_id, output),
    Outcome::WaitInput { snapshot, .. }
                                     => engine.suspend_for_human(node_id, snapshot),
    Outcome::PendingTool { pending, snapshot, .. }
                                     => engine.suspend_for_tasks(node_id, pending, snapshot),
    Outcome::BudgetExhausted { which, .. }
                                     => engine.handle_budget(node_id, which),
    Outcome::Error { error, .. }     => engine.fail_node(node_id, error),
}
```

Workflow 不需要 `AgentSession`，也不需要 PromptBuilder 的全套，**最小依赖就是 `LLMContextDeps`**。

---

## 8. 模块划分与文件落点

```
src/frame/opendan/src/
├── llm_context/
│   ├── mod.rs              // pub use
│   ├── request.rs          // LLMContextRequest / PromptSpec / ToolPolicy / OutputSpec / BudgetSpec / HumanPolicy
│   ├── outcome.rs          // LLMContextOutcome / ContextOutput / LLMContextSnapshot / ResumeFill
│   ├── owner.rs            // ContextOwnerRef / ContextSources trait
│   ├── deps.rs             // LLMContextDeps（tools / policy / worklog / tokenizer / aicc / taskmgr）
│   ├── state.rs            // LLMContextState / LLMContextSnapshot（可序列化的可变态）
│   ├── context.rs          // LLMContext::{new, run, resume, snapshot}
│   └── tests.rs
├── behavior/               // 现有，瘦身：把 run_step_inner 的循环搬到 llm_context
│   └── behavior.rs         // LLMBehavior 退化为 thin wrapper
└── ...
```

**改动原则**：先把 `behavior/behavior.rs` 里的循环原地复制到 `llm_context/context.rs`，跑通现有 Agent 测试；再切换 `LLMBehavior::run_step` 改为调用 `LLMContext::run`；最后删除 `behavior/behavior.rs` 中的循环代码。这是一个**纯重构 + 新增**，不破坏现有行为。

---

## 9. 与现有设计文档的关系

| 文档 | 关系 |
|------|------|
| `OpenDAN Agent Runtime 设计.md` | LLMContext 是该文档"Behavior Loop"小节的下沉抽象，原文档需要新增"L2 / L3 分层"段落引用本文档 |
| `Agent Session.md` | AgentSession 仍然是 Agent 的长上下文持有者，新增"实现 ContextSources trait"约束 |
| `Agent Prompt Compiler.md` | PromptBuilder 改为接收 `Arc<dyn ContextSources>` 而非 `Arc<Mutex<AgentSession>>` |
| `Agent Worklog.md` | 不变。LLMContext 是 worklog 的生产者，沿用 `AgentWorkEvent` |
| `OpenDAN Long Task & Sub-Agent.md` | `Outcome::PendingTool` 是 long task 在 LLMContext 层的统一表达；sub-agent 创建走 `deferred` 工具语义 |
| `opendan关键类型.md` | 新增 `LLMContext / LLMContextRequest / LLMContextOutcome / ContextSources` 章节 |

---

## 10. 实施路线

### Phase 1：抽象与并行实现（不破坏现有 Agent）
- [ ] 新建 `llm_context/` 模块，定义所有类型，先放空实现 + 测试桩
- [ ] `LLMContext::run` 复制 `LLMBehavior::run_step_inner` 的循环逻辑，仅支持 `OutputSpec::BehaviorLLMResult` + `PromptSpec::Compiled`
- [ ] `AgentSession` 实现 `ContextSources`
- [ ] 新增端到端测试：用 LLMContext 跑一个原 Agent 行为，比对输出

### Phase 2：切换 Agent 走 LLMContext
- [ ] `LLMBehavior::run_step` 改为 thin wrapper
- [ ] 删除 `behavior/behavior.rs` 中重复的循环代码
- [ ] 跑全部现有 Agent 测试（`behavior/tests.rs` / `agent.rs` 集成测试）

### Phase 3：扩展 LLMContext 能力
- [ ] 实现 `OutputSpec::Json{schema}` 与 `OutputSpec::Text`
- [ ] 实现 `PromptSpec::Prebuilt`
- [ ] 实现 `ToolPolicy.deferred` → `Outcome::PendingTool`
- [ ] 实现 `Outcome::WaitInput` 与 `HumanPolicy`
- [ ] 实现 `BudgetSpec` 与 `Outcome::BudgetExhausted`
- [ ] 实现 `LLMContext::resume` 与 `LLMContextSnapshot` 序列化

### Phase 4：AHL Workflow 接入
- [ ] AHL workflow 的 LLM node 直接构造 `LLMContextRequest`
- [ ] Workflow engine 实现五态分发与 snapshot 持久化
- [ ] 第一个 workflow 用例：含人工审批节点的 LLM 多步任务

---

## 11. 待决问题

1. **Tool 执行是否需要并发**：当前 `LLMBehavior::execute_tool_calls` 是串行的（`for call in gated_calls`），LLMContext 是否提供并发执行选项？倾向：默认仍串行，以 `ToolPolicy.parallel: bool` 开关，只在显式声明时启用。
2. **PendingTool 的粒度**：是"整轮 tool_calls 全部 deferred 才挂起"，还是"任一 deferred 就挂起，把这一轮的非 deferred 结果一并带回"？倾向后者，避免 workflow engine 做不必要的拆分。
3. **LLMContextSnapshot 的存储边界**：LLMContext 只产出 token，不负责持久化。Agent 把 token 存在 `agent_session.worklog`，Workflow 存在自己的 instance state。需要约定 token 大小上限（建议 < 32KB，超出走外部存储 + ID 引用）。
4. **HumanPolicy 与现有 Agent 的 reply**：当前 Agent 的"回复用户"是通过 `BehaviorLLMResult.reply` 隐式完成的，不属于 WaitInput。需要明确：`WaitInput` 仅指**LLMContext 主动挂起等待人工输入**，普通 reply 仍走 `Outcome::Done`。
5. **Sub-Agent 创建**：`CreateSubAgent` 工具应该是 deferred 还是同步？倾向：同步创建 + 异步执行，工具同步返回新 sub-agent 的 task_id，sub-agent 的执行结果通过另一个 LLMContext 实例承载。

---

## 12. 一句话总结

> **LLMContext 是 LLM 执行的"进程上下文"：一次有界、可 cooperative yield、可 resume、可计费、可审计的执行体。它把这层从 Jarvis 行为机里独立出来，填补 `llm.complete`（太低阶）与 `agent.sendMsg`（太重型）之间的空缺，让 Agent 和 Workflow 作为各自的调度器，共用同一套进程语义。**

---

## Appendix A: Non-Goals（永久边界）

下面这些 **不只是本期不做，而是永远不做** —— 因为它们会破坏 narrow waist 的中立性（见 Preamble）。任何要把它们塞进 LLMContext 的提议，都应该被退回到上面（scheduler 层）或下面（provider / effect 实现层）。

本清单是**活的，只增不减**：每次 PR review 拒绝一个"看似合理但会污染 waist"的提议，决议结果就补进这里，让后人不必重新讨论同一个问题。

### A.1 Scheduler-specific（永远不进 waist）

- `next_behavior` / 行为切换字段 —— Agent 行为机专属，应该走 `OutputSpec::BehaviorLLMResult` 这种特化输出，不进 LLMContext 通用接口
- workflow node 的 retry / fallback 策略 —— 上层 workflow engine 处理
- hook trigger 的事件元数据（trigger source / debounce / coalescing）—— 上层 hook scheduler 处理
- chat session 的 typing indicator / streaming UI 语义 —— 上层 shell / chat scheduler 处理
- multi-agent 的 turn-taking 协议 —— 上层 multi-agent scheduler 处理
- sub-agent 派生的层级关系字段（`parent_id` / `child_ids`）—— 上层调度器在 owner 维度记账，LLMContext 之间无父子关系
- 优先级 / 抢占 / 公平性策略 —— scheduler 政策，不是进程属性

### A.2 Provider-specific（永远不进 waist）

- 模型计费 / billing 字段 —— provider 自己 telemetry，与 waist 解耦
- provider 专属参数（anthropic `cache_control`、openai `seed`、gemini `safety_settings`）—— 通过 `model_policy.provider_options: opaque` **透传**，waist 不解释、不校验
- 模型能力探测（context window 大小 / 是否支持 vision / tool）—— provider adapter 内部决定，waist 不暴露
- token 计费方式（input vs output 不同价、cached vs uncached 不同价）—— 用 `TokenUsage` 抽象，不暴露 provider 计费规则
- streaming 协议细节（SSE / chunked / batch）—— provider 适配层处理；waist 一次推理对外是原子的

### A.3 Container / 长生命态（属于 AgentSession，不属于 LLMContext）

- session memory / 长期记忆 —— 通过 `ContextSources::memory_block()` 按需注入，LLMContext 自己不持有
- workspace 路径 / 文件挂载 —— 容器关心，进程不关心；通过工具调用访问
- agent identity / DID / 签名密钥 —— 容器属性
- 持久 worklog 的存储位置 / 索引策略 —— 上层 worklog 服务关心，LLMContext 只负责 emit 事件
- sub-agent registry / lifecycle —— 容器编排关心
- 跨 LLMContext 的对话历史拼接 —— 容器在外面拼，传进来时已经是 `ContextInput`

### A.4 Effect-side 持久化与执行策略（属于 EffectDeps 实现，不属于 waist）

- snapshot 的存储介质 / 加密 / 跨节点复制策略 —— `SnapshotStore` 接口的实现细节
- worklog sink 的具体实现（SQLite / 远程 / Kafka）—— `WorklogSink` 接口的实现细节
- tool 调用的审计 / 录像 / replay —— `ToolManager` 实现的可选行为
- pending tool 的任务队列后端（in-memory / Redis / BuckyOS task service）—— scheduler 决定
- tool 调用的并发 / 限流 / 熔断 —— `ToolManager` 实现，waist 只声明 `parallel: bool` 这种意图

### A.5 不解决的更大问题（本设计不替代，由其他文档负责）

- AgentSession 的容器编排 —— 见 §13 BuckyOS Integration（待写）
- Workflow DSL 与 DAG runtime —— AHL workflow 自己的 scope
- LLM provider 的统一封装 —— 走 BuckyOS AICC 现有抽象
- Memory 的存储与压缩算法 —— `Agent Memory v2.md` 的 scope
- Prompt 编译的具体策略 —— `Agent Prompt Compiler.md` 的 scope
- Agent 的角色 / 行为 / Jarvis 状态机定义 —— `OpenDAN Agent Runtime 设计.md` 的 scope

### 如何使用此清单

每次有人提议向 LLMContext 添加新字段或新方法，按以下顺序检查：

1. **先到 Appendix A 查重**：是不是已经被显式列为 Non-Goal？是的话直接拒绝并指向已有条目。
2. **过双中立性测试**（见 Preamble）：scheduler 中立？provider 中立？任何一项不通过即拒绝。
3. **过完两个测试且不在 Non-Goals 里**，仍要在 PR 描述里说明 *"为什么必须进 waist 而不是上下游某层"*。说不清楚就退回让提议人想清楚。
4. **被拒绝的提议，补进 Appendix A 对应小节**，标注 PR 链接和拒绝理由。这样下次有人提同一件事时，可以一句话回掉，不必重新论证。
5. **同意进入 waist 的字段，必须同步更新 §3 / §4 / §5 / §10 实施路线**，并在 changelog 里登记 waist 版本。waist 字段一旦进入，移除等同于 breaking change，必须走 deprecation 流程。

> 这套流程的目的不是为了"难"，而是为了让"瘦"成为默认状态。任何瘦腰原语的失败模式都不是被一次大改打破的，而是被一百个"加一个小字段没关系吧"的小改慢慢撑胖的。Appendix A 就是用来记住每一次"小字段没关系吧"被拒绝的理由，避免同一个争论开 N 次。



呃我用语音说一些站在最基础的这个框架的角度,这个agentloop这个过程中啊,或者我们叫做LM loop这个过程中,的这个我们的新增的这种behavior step模式相对于之前的模式的一些呃一些这个这种根本性的底层思考啊,包括说我们以及我们对于什么样的一个agent的loop才是一个所谓的,或者叫LLM loop吧,这个其实没到agent的层面,就一个LLM loop怎样才是一个好的loop的一个一个整体性的一个思考吧,我用语音说,你帮我记。


所谓的loop其实就是说它有一个明确的停止信号,也就是说相对于一次LM推理来说的话,LM loop的特点就是它有一个停止信号,当这个停止信号没有触发的时候,那么它就会有,就是说当然说也不可能说基于同样的提示词继续继续这个无限执行嘛,它通常是指拉上一次推理的结果,作为这一次推理的输入这样的一个模式,然后就相当于说这样的循环往复,然后完了直到这个触发的终止条件啊,所以说这个其实是LM loop相对于LM推理的一个核心的一个模式上的一个根本的不同。


那么这里面来讲的话,我们站在这个循环中间的一次典型的推理,我们来观察它的这个结构啊,就是传统的LM loop这种基于Function call的loop,我们把它叫经典经典loop吧。经典loop的话,它其实它在这里它是没有,就是说它叫做说它经典就属于它呃,我们对它的这个模式已经非常的了解了,对吧?就是说呃它的核心在于说它是由一组消息构成的,然后呢,这个这个消息就说最后一条消息啊,最后最后一条消息是一条不带这个这个Function call的消息,是吧?就说只要不带Function call那么就就认为loop停止了。如果带Function call的话,那么它这就相当于说它就增加了一条这个这个toerus消息,然后完了之后呢,这个这个调用发起之后呢,然后这个context执行执行层的话会去进行进行执行。然后执行的时候呢,根据执行的结果产生一个user消息,对吧?这个user消息是代表着一种呃这个这个工具的这个sideeffect嘛,就相当于说从原始的systemmessage加上第一条的这个input的message,然后呢,推完成第一次推理得到一个这个这个assistant的message之后,然后下面就不断往复,直到直到这个assistantmessage它不它不再发起Function call了,那么就就自然结束了,就自然结束。然后把最后一次的这个message作为这一次呃这一次这个LM loop的一个结果啊,这是一个传统的一个一个一个过程。然后这个过程里面它其实呃它的它的特点啊,就发现它很直观,就是说呃就在这里面永远不需要做任何的所谓的所谓的这个裁剪,是吧?这是最简单的一种,其实你的所有的这个精力其实都是放在了这个呃这个Function call的这个这个设计上,就是说相当于说说说推动这个LM loop的这个呃核心改进,其实就是在研究执行层嘛。 但后面我们其实就发现呃其实把所有的就是说当然这里面也有这个有些人是现在有些有些架构其实有一点点这个cache优先啊,但我觉得还是应该本质优先。就是说从这个注意力的优型优型框架来讲的话,其实我们会发现呃呃这里面其实它有些事情他其实是有有点别扭的,就你去关心关注一个很长的一个这个一个真正的一个现实中的这样的一个loop。你会发现其实这里面占最大头的内容啊,最大头的内容其实 其实是这个呃应该说是呃最大头的内容其实是一大堆的这个function call的结果吧,就比如说我这里面我要去搜索一个东西,对吧,就是说他为了得到一个一个,就是换句话讲,呃当当这个大语言模型根据任务的时候想要搜索一个结论,然后呢,当他当他这个他这个搜索结论的全过程都会以这个message recorder的这个形式存在于这个loop中。当他得到这个结论之后,其实从某种意义上讲,至少前面的这个搜索过程中的很多东西其实都应该都应该不叫了,但但这种这种loop它是没有办法的,也就是说呃如果说你是给每一次会话和结果都给他一个这样的loop,其实你还好,但其实九其实百分之九十九点九的这个agent它其实是永永久性的这个这个消息记录,也就是说呃如果不触发这个contextwindow的这个这个limitation对吧,那么其实呃相当于说agent的停住,这个loop停止之后,用户再发一条消息下来就继续触发,它说它的它对这个消息记录的继承呃是全量的,对吧,除非说除非说你这个这个这个将来说说我们其实是鼓励呃这其实是个明显的两个loop之间哈,就其实在两个loop之间就用户发消息之的那一瞬间,其实你是有我们是有机会呃把之前的上面一层的所有的消息,就说上一次的这个结果进行一次压缩的,对吧,然后这个压缩的话可能有些东西进到这个system提示信息去了,可能有些呃就是说可能把所有的中间的这个tool啊什么东西可能都干掉了吧。啊就这说说这种压缩呢,这个又会带来一些说说也许用户是追问啊什么的对吧,就可能你之前的搜索搜索的原始记录又很又很有价值的这样的一个做法,对,总之这个classical的loop有的时候想太多,反而不是件好事,对吧,既然说classical了,对吧,那其实classical的本质就是全量记录,对吧,全量记录


然后呢,我们现在Open单,其实我们自己在主推的这个叫做behavior loop,它的核心是什么呢?它的核心是把前面讲的这种基于函数的这种loop作为一种内部内部loop,就是说它上来第一第一天就认为这个内部loop是一定会丢掉的,就是说它是个纯内存状态,然后呢它我们现在的这个叫做behavior loop,它的核心是step record,就是说每一个step每一个step的这个record,然后呢这个step的这个结束是显示的,也就是说呃当时我们现在其实做了一个隐式触发,但这个隐式触发其实很就是说我们需要去去这个所谓的为下一次推理这个获得一个input,这是一个典型的这个操作系统出生的这种这种这种背景啊,就是我们我们希望说它有一个一个信号信号这个信号只要是保持激活,才可以到下一步,反正就是我们提供了两个两个这个变量嘛,就比如说呃input这层呢,如果说你已经没有新消息了,对吧,新消息之后呢,这个没有新消息,而且step已经停止了,对吧,那这个系统就就自动停下来,对吧,但如果有新消息其实还是可以继续下去的,因为你没有办法判断这个新消息是对上上一个loop的补充,还是说是一个新的这个这个是一个新的新的新的question啊,或者是一个新的新的这个问题吧,对,所以说呃在这个角度角度来讲的话,呃新的这个loop它模式,它其实呃我们其实是是要求大约模型本身自己去管理这个循环的状态机,本质上是这个样子,对吧,然后呢,在这个状态机内部的话,我们其实提供了呃更多的这个细节,当然说最重要的细节之一就是把这个呃把这个方形扩给彻底压缩掉了,就是说方形扩只会在一个step的内部, 就一个step一旦形成了这个结论,就我们我们现在模型叫做结论,思考,执行模型,就是跟跟传统的react相比,对吧,我们更加强调这个之前叫做观察,观察这个思考动作嘛,对吧,那我观观react跟跟我们模型还是不一样的,对,我们相当于说我们每一个step其实都有结论,对吧,那你我们看到的这个这个这个step recorder它其实就是一个呃结论当前思考,对吧,就我可以看到上一个step recorder的这个action执行结果其实就是一个结论嘛,对吧,那我拉到这个执行结果之后,我在这个step里我要先得出我在这一个step的结论,然后再思考,然后思考之后再再决定要不要做下面一个action,反正它决定的这个action里面其中就包含了这个认为这个事情已经结束的这个action,就end嘛,对,end的action


当然这种是一种模式性的这种改变啊,我们其实在实验中也发现,让agent其实去关注更多的,就多头的这种大语言模型的system提示词其实某种意义上来讲的话,当然说这个我们目前的时间来看,让agent去选择调某一个,就是说传统的这个function call它的最大的痛苦就是模型不会给你任何的理由,就是说你是没有办法让模型在调这个function call之前给你输出一段thinking的,对吧?你就看到的结果就是它,就是airmode推理完之后,啪唧太多它要调function call,对吧?就是说那我们其实相对来讲是比较容易,就是说我们我们这种step模型它的好处是什么呢?step的好处是action本身是可以被,就是说对于action的执行,我们是天然有理由的,就是说我们是可以观测到基于什么样的原因之后,因为你看得到这个consolution和thinking嘛,对吧,你可以看得到说每一次action是怎么去调的。对吧,所以我们最后在实践中其实99%的使用场景都把function call直接关掉了,就觉得这东西不好跟踪,对吧,就因为本身是等价的嘛,就是说只是一个是step内部调用,一个是step这个边边上调用嘛,所以说就会发现你所有的这个function call最后都转化成了action,对吧,那action的结果我们因为我们对于action的结果是属于会进入下会是再下一个step形成这个consolution的输入嘛。对吧,就是说我们在这个层面上,其实呃我们相对于step的caller本身又比想象中的变得更大了,就就很多人可能会把一些,我们现在并没有限制说action一定是要写操作啊,其实原来我们只有做这个设计,就是说这个function call呃是是读操作,然后呢这个action是写操作,但实际上来讲的话,就我们其实还是做到了做做了完全对的,因为确实有些人认为说我我就说有一个很很很就跟前面的原因是很强的,就说我现在这个这个我这个任务啊,比如说说说的更加纯粹直白这个场景,就是说很多很多很多这个里面它的这个这个上来的第一行就是根据某某某文档对把实现需求,对吧,那我这个文档我读取之后,对吧,我肯定是用这个要么用function call,要么用用这个action去读嘛,那我把这个文档读到这个读读到这个这个提示词里面去,并且在这个上下文里面,就在这个这个这个后期的这个loop里面进行保持保持,这是一个我一定要实现的一个功能,对吧,你不能说 你把我这个文档我读完之后,你给我形成一个结论,就帮我把这个原始输入给压缩掉了。对吧,所以说我觉得呃我们在这里其实我们看到啊,其实我们看到了的一个应该说是呃站在这个提示词编排的角度来讲,来讲的一些根本性的呃一些一些一些一些思考吧,就相当于说呃其实我们呃从结结果上讲哈,从结果上讲,就是说说我们在这个整个的这个contextwindow里占用大量的这个context window的那些读操作的结果,其实它从原理上讲,它是有两类的,有一类是它本质上讲是读了之后做系统提示词的,然后另一类,它其实是希望它只是要看一下探索性的,也就是说,也就是说呃如果说站在这个简单的二分法来讲,我们可以认为说一个读操作的结果要不要保存在这个历史记录里面,就是看你它它能不能够变成一个一个consolution哈,就当然说你我们你会我们随着这种呃这种潜在的意图的识别,其实这是一种意图识别,就是说换句话讲,这也是我们为什么喜欢现在现在新的这个loop的原因,对吧,很多时候倒不是因为它的这个这个压缩性更好,对吧,我们刚才讲了,其实它这种压缩性很快就被被放弃了,而是我们通过通过这种这种这种这种的这种reaction模型,其实我们更多的了解了呃大约模型去做一些action时候的意图。其实我们这些意图啊,就是说这也是一些呃一些思考啊,就通过这些意图,其实我们是有机会呃因为因为因为我们已经现在是用step loop,其实相当于说每次做推理之前,你其实都是已经做了重新的这个编排了嘛。因为你根本就不是这个message结构,其实原理上讲,你是一个大大的system提示词,我们现在就强制就是三个提示词,呃三个三个message,一个system message对吧,中间一个叫做叫做history message,然后最后一个叫做task message或者叫input的message。其实我们现在已经强制变成这个样子了。 对吧,就是说相当于说,我们其实现在并不特别在意这个cache命中啊,因为我们更关心效果,对吧,cache命中是一种实现架构,实现这个是因为现在的这个底层CPU这么设计的,对吧,它有下次,但我们觉得还是效果第一,对吧,说怎么样都说永远用更短的这个提示,用更短的context window能够得到这个更好的结果,我们一定会坚持做这个,一定会坚持做这个事情,因为根据大语言模型的原理,你的这个上下文越准确,这个信息密度越高,对吧,你把事情做对的概率就越大,而且从算力上讲,对吧,你也会消耗,原理上讲你就你消耗的算力就是更少。

然后说到具体使用我们的这套新的Behavior Loop去做Agent的提示词的时候,其实当然说跟我们之前选的这个这个技术方案有关,因为我们那个是用NASA写的,所以说我们自然而然会用一个比较传统的这种的模板引擎去做提示词的管理啊。这个很自然,对吧,但其实实际使用中来讲的话,就很多用户会抱怨说,模式这个因为很多是过去写python和typescript出身的嘛,对吧,那他们的很多时候他们的那些所谓的system提示词通常是一个python脚本,也就是说,当然从我们的角度来讲,我们认为用脚本去做提示词,这个从审计角度和权限角度来讲,这个有点太夸张了,因为它是个动态的东西,但确实很多时候用这玩意的能力就是比较的强。对,所以我们其实现在现在这个我们的提示词的这个引擎训练引擎其实现在已经扩展的很很,就已经可以在里面执行bash命令了。其实也就是为了为了这个对齐这个用脚本写这个提示词的一些一些需求嘛。但我们其实在这里面就会发现,其实很多提示词里面,它是隐含了一些在在system提示词里面可以编排一些文件文件的内容的这样的一些行为了。也就是说呃也就是说呃它其实也并不完全在赌这个这个整个循环过程中循环过程中这个我的cache一定要命中,比如说它的系统提示词在有在reload的时候啊,它是有机会载入一些文件的,就是说这些这种机制吧,基本上是现在的一些比较现在的一些主流的-agent框架的hookpoint的机制的这个起点,也就是说我的system提示词里面我并不是把内容直接写进去,而是呢,说到底就是就放了个符号链接在这里,对吧?然后呢,这个符号链接,现在当然说现在就是这个这个就全部都是往文件系统去引用啊。但其实我们在想的是,如果说去真的看穿这种意图之后,本上讲的问题就是我们要不要在这个我们的这个behavior loop里面,就是第一个是我们对于这种 系统pump的这个符号链接我们到底要怎么去准确地定位它。第二个是我们能不能用action去修改一个符号链接,或者添加一个符号链接,对吧?就相当于说,如果说我们把这东西显示化,其实某种意义上讲,我们就可以更加直白的把这个把这个或者说把把刚刚讲的这种我读一个文件,然后把这个文件做成当前这个loop的核心目标这件事情给显性化,也就是说系统里面我的system其实里面有一个所谓的,比如叫做go的一个一个这个currentgo的这样的一个符号链接,对吧,我可以有一个action,对吧,这个action的名字就是显示的这个这个这个linklink对吧,就是叫link,然后第一个参数是go,第二个参数是一个文件,对吧,你就把这个这个软件接就直接打上去了,对吧,那这个命令执行完之后,OK,那所有的这个就就换句话讲,从我的一个steprecording的角度来讲的话,我我就可以非常非常放心的就不要再在这一步里面去去保存这个文件了,因为你在下一次循环的时候,就下一次循环的时候,这个这个系统pump他在他在reload的时候,其实就把刚刚你在中间这个这可能是第一步或者第二步得到的这个这个文件的内容已经全部都写到这个系统提示里去了。

也就是说站在现在这个提示这个context window的最大的敌人啊,就是这个呃在探索阶段读文件的结果这件事情来讲,我觉得我们我就说我们其实在这个身上就弄得我们比较的痛苦,因为我们我们并不是classical模型,对吧?不是classicalloop,对吧,就我们竟然打算做做压缩,结果最后回头来,对吧?虽然说得到了一些别的好处,但但其实原始目的没实现嘛,就是说换句话讲,这一个声音就是说那还不如拉着这些精英回到这个classicalloop上去,对吧,这个是一个我们现在的一个纠结一个一个痛点啊,对吧,所以说呃,但我觉得这里面穿透性的思考还是在于呃这个agent就我们讲LLMloop里的根本还是因为它能够根据一个那个agent的tool的结果去推进下一个,就是说你可以改变下一次的这个这个这个推理的这个提示词的构成嘛。对吧,所以我们改变的方法,我觉得我们应该把它要要模式化,要更加的模式化的,就是说至少,对吧,就是说刚刚这个有的时候会导致导致这有两我觉得这个有两种思路哈,有两种思路,第一种思路叫做叫做就是我在我显示的在当前的loop里改变,显示的在当前loop里改变,也就是说呃我我要求就当时会导致llm的这个结论的多头化哈,就他不但他要想太多事情了,就而不是不是说说这种随心而为了,对吧,就他要去理解很多他的这个状态管理的目标。对,但但这就看嘛,就比如说我在我在发起一个这个readaction的时候,我完全可以去说明这个read操作的目的是什么?对吧,是什么目的,如果是去去去去最最低权限的就是一次探索,对吧,就读读看对吧,然后呢,读完之后我是会形成结论的,对吧,然后第二种是什么?第二种就是或者说我读完之后我形成结论的时候,我可以再提高它的等级。就我们至少是应该是有,我认为至少有三种等级吧,就第一种等级就是最最低最低呃最低的这种等级,就是结论就可以了,读的内容可以直接放弃掉,对吧,然后第二个就是OK,我把它加入了我的所谓的love列表,就是说我认为这个文件对这个文件对当前的目标,对当前loop的目标很重要,对吧,就是说我把它放到love列表里面去,然后第三层呢就是 OK,我这个认为它是可以替换掉这个,我理解我的这个System提示词的结构,我认为它可以替换掉我这个系统结构里面的某一个槽点,一个一个solid吧,就是说我当时去理解这个机制啊,去理解这是自己的system提示词是怎么拼装来的,对吧,又是又是一个问题,对吧,这可能这可能这可能就是可能这个事情又又会让它变得变得这个这个难度有点有点有点大,就是说本来他看到是一个拼装完的提示词,结果他还要了解这个这个提示词的这个system提示词的原始结构吧,对吧,反正我觉得呃如果说我们给他有不同的这个分级啊,不同的分级,其实某种意义上是给了我们呃主动的在这个,就是说换句话讲,当我们去坚定的去放弃这个functioncode的中间状态的时候,对吧,其实我们其实说的就是说在这件事情上,这个我们能够更加的让functioncode做读,让action做写嘛。 这是一个这是一个一个方向,然后还有一个方向就是所谓走旁路旁路什么意思呢? 就是就是我们呃当当说一般旁路是机械触发的,就是说变成主动触发又没意思了,机械触发就是比如说说运行了多少个step以后,或者说呃运行运行了这个就是说当contextwindows到达多少以后,你会有一个旁路触发,那旁路触发他要做的事情其实就是修正刚刚的结构,就是说他会去review现在的这个因为因为本质上讲他的这个成本跟一些标准标准输入是一样的,提示词是不同,对吧,他会去review现在已经累计好的这个超长的提示词上下文,但他的目的变成了说我要去分析哪些东西是可以删的,哪些东西可以留下来,对吧, 然后最后呃又让整个整个的这个这个提示词吧,就是说他去可以去修改这个system提示词区和这个historichips对吧historictics对吧,这个旁路是旁路的这个代价只是一次呃额外的提示,额外的一次推理而已,就多一个step而已,对吧,但但相对来讲,就他不会导致说我在主循环上每一个这个step都需要去进行这个多头的这个目标管理。

