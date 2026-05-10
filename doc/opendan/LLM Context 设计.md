# LLM Context 设计

> Status: 1.1


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
- **为什么有 6 态退出**：进程要么 exit，要么 yield 等 IO，要么 yield 等 input，要么 yield 等 context 压缩，要么被 kill —— 不可能只有"返回值"
- **为什么 owner / scheduler 抽象**：scheduler 不关心进程在跑什么业务，只关心生命周期；context 不关心被谁调度，只暴露 yield/resume 协议
- **为什么需要 snapshot**：挂起必须能完整保存执行态以便恢复

**重要约束：所有 yield 都是 cooperative 的**（合作式让出），不是 preemptive（抢占式）。LLM 不会被中途打断；只有 LLM 自己说"我需要这个工具结果 / 我需要人来回答"，或者预算用完，才会让出 CPU。这避免了"在 inference 中途切换"这种破坏 token stream 完整性的语义。

### 0.1 Scheduler-facing 语义层（L4）—— 谁直接面向用户

LLMContext 是 waist，**不是 scheduler-facing 的语义层**。DSL 编写者、Agent role 编写者、配置文件编写者**永远不直接接触 LLMContext**，他们接触的是各 scheduler 自己的一等公民语义层：

- `LLMWorkflowContext` —— Workflow DSL 直接配置的对象。知道 service endpoint、上游 node 引用、retry / fallback 分支。
- `LLMAgentContext` —— Agent role md / behavior 配置直接产生的对象。知道 AgentSession、Jarvis 行为机、tmux session、agent-tool manifest。
- `LLMOneShotContext` —— OneShot CLI 调用直接产生的对象。知道 cwd、命令行参数、stdout 重定向。
- 其他 scheduler（Hook / Eval / Multi-agent debate）按需各自定义自己的 `LLM*Context`。

它们**互相独立、互不知道**，但都通过 **lowering**（降级）产出同一个 `LLMContextRequest + LLMContextDeps`，再喂给同一个 `LLMContext::run`。

类比 LLVM：LLMContext 像 LLVM IR，是稳定的中间产物；各 `LLM*Context` 像各前端语言（C / Rust / Swift），各自有自己的语义层和工具链，但都降级到 IR 上。

这个 L4 ↔ L2 的分层在 §2 三层能力分层中作为完整 4 层重新呈现，并在 §10 实施路线中显式划入 Phase 2 / Phase 4 的产物清单。**任何"DSL 用户可见 / 配置文件可写"的字段都不进 waist，应该落在对应的 `LLM*Context` 里**（参见 §A.1 补充条款）。

### 0.2 Loop 不变量：intent / effect / observation 三元组

> 本节是对 LLMContext **loop 语义骨架**的硬约束，凌驾于 §3 / §4 具体字段之上。任何 §3 字段的设计、任何 ToolManager 的实现选择，都必须先满足本节的不变量。

LLMContext 的 loop 不变量是：

```
intent → effect → observation → intent → effect → observation → ... → terminal
```

**不是** function call → tool result → function call 的循环。Function call 只是 OpenAI / Anthropic / Gemini 各家在 wire 层选择的一种 effect 编码格式，**不是 waist 的语义骨架**。把 function call 抬成一等公民既不 scheduler 中立（Agent 行为机和 workflow service-call 的诉求不一致），也不 provider 中立（各家 wire format 不通用，本地模型经常根本没有原生 tool support）。

**三个概念在 waist 内的归属：**

| 概念 | 在 waist 里的载体 | 谁产生 | 谁消费 |
|---|---|---|---|
| **intent**（意图） | `OutputSpec::{Json / Xml / BehaviorLLMResult}` 解析出的结构化产物；其中的 actions / tool_calls 字段 | LLM（输出端） | LLMContext 主循环 → ToolManager |
| **effect**（执行） | `ToolManager::call_tool` 内部的实际动作（CLI 调用 / kRPC / native fn / async task 提交） | ToolManager（effect 实现层） | 外部世界 |
| **observation**（观察） | `Observation::{Success | Error | Pending(call_id)}` | ToolManager | LLMContext 主循环 → 喂回下一轮 LLM |

**Effect 在 waist 上有两种合法承载形态**：

1. **结构化 output 里的意图集合** —— LLM 通过 `OutputSpec::Json / Xml / BehaviorLLMResult` 在 reply 中显式声明一组带意图的 effect 请求。例：`BehaviorLLMResult.actions = [...]`，每个 action 都是一段语义完整、可被人类读懂的"想做什么"。
2. **provider-native tool_calls** —— 仅在 provider 支持且 `ToolPolicy.mode != None` 时启用，由 provider adapter 解析为同样的 effect 请求。

两种形态在 waist 内部经由 ToolManager **归一化为同一组 `Observation`**。"哪种 provider 走哪种 wire format"由 provider adapter 自己决定，waist 不偏向、不强制。

**为什么这个不变量值得作为 waist 纪律**：

- **意图先于格式**：waist 的设计哲学要求每个 loop step 都能被审计、被人理解、被 worklog 真实地复述。Function call 那种"只有调用没有意图"的协议，把"为什么调"的语义全部塞进了 system prompt 的灰色地带，破坏了 waist 的可观测性承诺。
- **格式不应硬编码**：把 function call 钉进 waist 立刻丢掉 provider 中立性。本地模型走 grammar-constrained decoding、Agent 行为机走 XML、workflow 简单节点走 JSON schema —— 同一个 effect 概念有多种 wire 编码完全正常。
- **与 §3.10 终态/挂起态二分自洽**：observation 的 `Pending` 状态正是 cooperative yield 的载体，把它和 function call 解耦后，"哪些工具是异步的"完全是 effect 实现层的私事，waist 只看见 `Observation`。

**PR review 落点**：任何想把"function call schema / tool_use block / OpenAI strict mode"这类 wire-format 概念引入 waist 公共类型的提议，按本节直接退回到 ToolManager 与 provider adapter 实现层。具体条目见 §A.2。

## 1. 背景与动机

### 1.1 直接动机：Workflow 缺中间层

AHL Workflow 接入 LLM 当前只有两个不合身的选择：

- **`llm.complete(prompt)`** —— 太低阶。每个 workflow node 自己拼 prompt、自己实现 tool loop、自己处理 retry/budget/结构化输出/人工节点，最后变成"在 workflow 里重写一个迷你 agent runtime"。
- **`agent.sendMsg(...)`** —— 太重型。强制带上 AgentSession、长上下文、行为机、Jarvis 状态切换，workflow node 只是想"跑一次 LLM + 几个工具"，不需要这一整套。

`LLMContext` 就是这中间缺失的层：**有 agent runtime 的能力（prompt 编译 / ReAct loop / worklog / budget / pending），但没有 agent session 的长生命语义**。

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

抽出 `LLMContext` —— **一次性、短生命、可被多种 owner（Agent / Workflow / 人工触发的一次性任务）构造的有界 LLM Loop 执行单元**：

1. **不绑定 Jarvis 行为机**：不强求 `next_behavior` 这种字段，输出 schema 由调用方声明。
2. **不绑定 AgentSession**：通过抽象的 `ContextOwner` / `ContextSources` 注入需要的上下文（memory / 历史 / 模板变量）。
3. **统一的退出语义**：`done / wait_input / pending_tool / context_limit_reached / budget_exhausted / error` 六态，便于上层（Agent loop / Workflow engine）一致地推进状态机。结构上分为终态与挂起态两类，详见 §3.10。
4. **统一的可观测性**：worklog、step record、token usage、tool trace 都通过 `LLMContext` 沉淀。
5. **复用现有实现**：`LLMContext` 不重写 LLM 调用与 tool loop，是 `LLMBehavior::run_step_inner` 的**重新切片与重新封装**。

### 1.4 非目标

- 不替换 `AgentSession`：Agent 的长上下文、状态机、行为切换继续由 `AgentSession` 管。
- 不替换 `WorkflowInstance`：Workflow 的 DAG / 人工节点 / 分支由 workflow engine 管。
- 不引入新的 LLM provider 抽象：仍走 `AICC / TaskMgr / AiMethodRequest`。
- 不动 worklog 存储格式：`LLMContext` 是 worklog 的**生产者**，不是新的存储层。

---

## 2. 四层能力分层

```
┌─────────────────────────────────────────────────────────────┐
│  L4  Scheduler-facing 语义层（DSL / 配置文件直接面向）      │
│  - LLMWorkflowContext   workflow DSL 配置的对象             │
│  - LLMAgentContext      agent role md / behavior 配置       │
│  - LLMOneShotContext    CLI 参数                            │
│  - 各自有 Def（静态）+ Instance（运行时）                   │
│  - 各自负责 lowering 到 L2 LLMContextRequest + Deps         │
│  - 持有 scheduler-specific 字段（endpoint 引用 / 上下游引用 │
│    / 行为机配置 / 容器句柄...），这些字段绝不进 waist       │
└──────────────────────────┬──────────────────────────────────┘
                           │ lowering
┌──────────────────────────▼──────────────────────────────────┐
│  L3  Scheduler 调度层（OS 类比：进程调度器）                │
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
│  - 六态退出：done / wait_input / pending_tool /             │
│              context_limit_reached / budget_exhausted /     │
│              error  （二分：终态 / 挂起态，见 §3.10）       │
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

L1 已经有了，L3 部分有（`AIAgent`），workflow 那边还没有，**L4 全部待建**。本设计**新增 L2，并显式建立 L4 的两个最关键实例（`LLMAgentContext` 与 `LLMWorkflowContext`）作为姊妹文档**，把 L1↔L3 的直连改造成 L1↔L2↔L4↔L3。等价地，把"workflow 直接 syscall（`llm.complete`）"和"workflow 直接 fork-exec 一个完整 agent 进程（`agent.sendMsg`）"中间，加进真正的"进程上下文"层，并把"DSL 用户面对什么"这层显式化。

### 2.1 L3 vs L4 的区别

L3 是**命令式控制流**（Agent loop / Workflow engine 的运行时代码），L4 是**声明式 schema**（YAML / TOML / role md 描述）。同一个 scheduler 同时拥有 L3 和 L4 两块产物：L4 描述"这个 LLM 调用长什么样"，L3 描述"什么时候调它、Outcome 怎么处理"。

### 2.2 为什么 L4 必须显式存在（不能藏在 builder 里）

1. **L4 有自己的生命周期**：Def 编译期 → Instance 实例化 → lowering → LLMContext 跑完 → Instance 处理 outcome → 节点完整结束。L4 比 LLMContext 活得长。
2. **L4 的可序列化形态 ≠ LLMContext 的可序列化形态**：L4 持有 symbolic 引用（`endpoint: kRPC://...`、`${prev_node.output.x}`），LLMContextRequest 持有 resolved 句柄（已绑定的 ToolManager、已展开的 prompt）。lowering 是这两种形态之间的转换。
3. **L4 是 §A.1 / §A.3 Non-Goals 的栖息地**：所有 scheduler-specific 的字段（service endpoint / 上下游引用 / on_budget_exhausted 分支策略 / Jarvis 行为配置 / tmux session 句柄...）必须有地方放。L4 就是它们的家，否则它们会被塞进 waist。

### 2.3 承载方式（部署形态）

LLMContext 是一个 lib（`llm_context` crate），**不是一个 service**。它有三种承载方式：

1. **In-process lib 调用**（Agent service 默认）—— `LLMAgentContext` lowering 后直接在 Agent 进程内调 `LLMContext::run`。稳定、自由度高、无序列化代价。
2. **Workflow thunk 承载**（workflow LLM node 默认）—— `LLMWorkflowContext` lowering 后封装为 thunk，由 workflow runtime 通过 node_daemon 调度到指定容器/tmux 执行。snapshot 持久化、跨节点迁移等工程问题由 workflow runtime 统一解决。
3. **AICC RPC 承载**（跨设备场景）—— 例如手机上的 OpenDAN 让 home OOD 跑一个 LLMContext，输入走 `LLMContextRequest` 序列化，输出走 `LLMContextOutcome` 序列化。

三种方式共享 100% 的执行语义，差异只在 deps 注入和 outcome 投递路径上。**承载方式由 scheduler 根据部署语义选择，不是 waist 的属性，也不是二选一**。Agent service 内部的 `LLMAgentContext` 应当走 (1)，workflow 的 LLM node 应当走 (2)，跨设备场景按需走 (3)。

---

## 3. 核心抽象

### 3.1 LLMContext

`LLMContext` 是一次执行的**对象化封装**，不是一个静态函数。它持有：

- 不可变输入 `LLMContextRequest`
- 可变运行态 `LLMContextRunState`（剩余 budget、tool loop 计数、worklog buffer、当前 pending 项）
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
    /// - wait_input / pending_tool / context_limit_reached：挂起态（cooperative yield），
    ///   state 可序列化为 snapshot（见 §3.10 终态/挂起态二分）
    pub async fn run(&mut self) -> LLMContextOutcome;

    /// 从 snapshot 恢复（context switch in）。fill 的形态与产生 snapshot 时的挂起态对应。
    pub fn resume(snapshot: LLMContextSnapshot, fill: ResumeFill, deps: LLMContextDeps) -> Self;

    pub fn snapshot(&self) -> LLMContextSnapshot; // 用于 step_record / 审计
}

/// Resume 时上层喂回的数据，形态由产生 snapshot 时的挂起态决定。
pub enum ResumeFill {
    /// 对应 PendingTool：上层把 deferred 工具的执行结果填回。
    ToolResults(Vec<(CallId, Observation)>),
    /// 对应 WaitInput：上层把人工输入填回。
    HumanInput(ContextMessage),
    /// 对应 ContextLimitReached：上层把重整后的对话历史填回。
    /// LLMContext 用这份新 history 替换 state 里的 accumulated messages 后继续。
    /// 这里的"重整"语义完全由 scheduler 决定：summarize / drop oldest /
    /// hierarchical recall / 换模型重灌 system prompt ... waist 不介入。
    RewrittenHistory(Vec<ChatMessage>),
}
```

### 3.2 输入：LLMContextRequest

```rust
pub struct LLMContextRequest {
    /// 上层 owner 标识，用于 worklog / tracing / 审计
    pub owner: ContextOwnerRef,           // Agent(session_id) | Workflow(instance, node) | OneShot(id)
    pub trace: Option<String>,            // 用于调试

    /// 任务声明
    pub objective: String,                // 自然语言目标，不进提示词
    /// Prompt 来源（已编译的片段或编译指令，已经包含了Input?)
    /// AiMessage的内容可以是Prompt模板
    pub input:     Vec<AiMessage>,         

    /// 模型策略
    pub model_policy: ModelPolicy,        // 复用 behavior::types::ModelPolicy

    /// 可用工具与工具策略
    pub tool_policy: ToolPolicy,          // 见 3.5

    /// 输出契约
    pub output:    OutputSpec,            // 见 3.6

    /// 资源边界
    pub budget:    BudgetSpec,            // 见 3.7

    /// Human-in-the-loop 策略
    pub human_policy: HumanPolicy,        // 见 3.8
}
```

设计要点：**`session: Option<Arc<Mutex<AgentSession>>>` 这种字段不再出现**。Agent 模板变量通过 `ContextSources` 注入（见 3.9），而不是把整个 session 塞进来。



### 3.5 ToolPolicy

```rust
pub struct ToolPolicy {
    pub mode:      ToolMode,           // None | Whitelist | All
    pub whitelist: Vec<String>,
    pub max_rounds:           u32,     // 0 = 禁止 tool loop（一次推理即返回）
    pub max_calls_per_round:  u32,
    pub max_observation_bytes: u32,
    /// 是否允许同一轮的 tool_calls 并发执行。默认串行。
    pub parallel: bool,
}
```

工具执行委托给已有的 `AgentToolManager`，policy gate 委托给已有的 `PolicyEngine`。

**Pending 语义**（与 BuckyOS AgentTool 物化的"统一 Tool Result JSON 协议"对齐）：
- ToolManager 的 `call_tool` 返回值扩展为 `Observation::{Success | Error | Pending(call_id)}`。
- 当 `allow_deferred = true` 且某次 `call_tool` 返回 `Pending(call_id)`，LLMContext 立即产生 `Outcome::PendingTool`，把已完成的非 pending observation 一并带回（不强制等齐整轮再挂起）。
- 当 `allow_deferred = false`，ToolManager 不应返回 Pending；如果返回了，LLMContext 视为 `Outcome::Error`。

这条把"哪个工具是异步的"的决定权交给 effect 层（具体的 CLI 工具 / kRPC 服务自己根据语义决定），waist 只控制"是否允许"。

**关于 tool 调用的 wire format**（与 §0.2 loop 不变量配套）：waist **不规定** LLM 如何在输出里编码 tool 调用意图 —— OpenAI tool_calls / Anthropic tool_use block / 自定义 XML / structured output 里的 actions 数组 / grammar-constrained decoding 皆可。这是 ToolManager 与 provider adapter 协商的私事。waist 只看见经过归一化的 `Observation` 序列；具体哪段 LLM 输出被解析成哪个 tool call，由 provider adapter 与 ToolManager 在内部协议里说清楚。这条边界使得"换 provider"和"换 effect 实现"互不打扰。

### 3.6 OutputSpec

```rust
pub enum OutputSpec {
    /// 自由文本，调用方自己解析
    Text,
    /// 强制 JSON，可校验 schema
    Json { schema: Option<serde_json::Value>, strict: bool },
}
```

### 3.7 BudgetSpec

```rust
pub struct BudgetSpec {
    pub max_total_tokens:     Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub max_wallclock_ms:     Option<u64>,
    pub max_hp_cost:          Option<u32>,   // 复用 AgentConfig 的 HP 模型
    pub on_exhausted:         BudgetAction,  // Fail | ReturnPartial | EscalateHuman

    /// 接近 context window 阈值时触发 `Outcome::ContextLimitReached`（见 §4）。
    /// default None ⇒ 不开启，撞到 provider 边界时仍走 Outcome::Error。
    /// 开启后 waist 只负责"达到阈值"这个事实信号，具体如何压缩/重整
    /// 由 scheduler 在 resume 时决定（参见 §A.4 上下文压缩策略 Non-Goal）。
    pub context_yield_threshold: Option<ContextThreshold>,
}

pub enum ContextThreshold {
    /// 已用 token 占 provider context window 的比例（0.0 ~ 1.0）。
    /// 例 0.85 表示用满 85% 时 yield。Provider adapter 负责换算 window 大小。
    Ratio(f32),
    /// 已用 token 的绝对值。适合 provider window 大小未知或不稳定的场景。
    AbsoluteTokens(u32),
}
```

**`max_total_tokens` 与 `context_yield_threshold` 的区别**：
- `max_total_tokens` 是**预算耗尽**，触发 `BudgetExhausted`（终态，类比 OOM kill）。
- `context_yield_threshold` 是**接近 window**，触发 `ContextLimitReached`（挂起态，类比 page fault yield 给 swap 处理）。
- 两者可以同时设置，前者是上限红线（必须 fail），后者是预警阈值（可以被上层修复后 resume）。这正是 §3.10 终态/挂起态二分在预算这一维度的具体体现。

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

当LLM Context支持 input里不是完整提示词，而是提示词模板时，就需要通过某种方法得到一个”用来查找模版变量的环境“

### 3.10 Outcome 的二分：终态 vs 挂起态

> 本节是"显式大于隐式"原则在 Outcome 设计上的硬约束。新增 Outcome 变体必须明确归入下面其中一类，并满足对应的不变量。

LLMContext 的 Outcome 在结构上分成两类：

**【终态】**（LLMContext 对象消耗完毕，**不可** resume）

| Outcome | 语义 | OS 类比 |
|---|---|---|
| `Done` | 正常退出，产出 `ContextOutput` | `exit(0)` |
| `Error` | 异常退出，产出 `LLMComputeError` | `exit(非0)` |
| `BudgetExhausted` | 预算红线击穿（token / wallclock / HP / tool rounds）| `SIGKILL` / OOM kill |

**【挂起态】**（产出 snapshot，等待外部填回后可 resume）

| Outcome | 语义 | OS 类比 |
|---|---|---|
| `WaitInput` | 等待人工输入 | `read()` 阻塞 |
| `PendingTool` | 等待 deferred 工具回填 | `io_submit()` 后等待 |
| `ContextLimitReached` | 接近 context window，等待上层决定如何压缩/重整 | page fault → 等 swap 处理 |

**挂起态的设计纪律**：

1. **任何让 LLMContext 无法继续推进、但又不构成"失败"的情况，都必须显式建模为某一种挂起态**，而不是隐藏在 `Done` 或 `Error` 里。这是"显式大于隐式"在 waist 上的具体落点。反面例子：在 LLMContext 内部偷偷做 history summarize 然后假装正常返回 `Done` —— 这会破坏 worklog 的真实性、破坏 snapshot 的可重放性、破坏 token usage 的可审计性，被本节纪律禁止。
2. **挂起态必须产出 snapshot，且 snapshot 满足 §6.2 不变量**（自包含、跨节点可 resume、不持有 effect-side 真实世界状态）。
3. **挂起态的产生条件必须是 cooperative**（§0 不变量）：LLM inference 完成后才有机会让出，不允许中途打断。
4. **新增挂起态需要走 waist 字段变更流程**，不是 minor change —— 因为它会同时影响所有 scheduler 的 outcome 分发逻辑。

**为什么把 ContextLimitReached 抬到挂起态而不是终态**：上下文压缩这件事在不同 scheduler 那里诉求**完全不同** —— Agent 想 summarize-and-rewind（保留 memory 关键事实）、Workflow 想 fail-and-escalate（直接报错给上一节点 retry）、Eval 想 hard-truncate（看模型在压力下的行为）、OneShot 想 graceful-degrade。任何"在 waist 里规定压缩策略"的字段都会偏向某一种 scheduler。但**"接近阈值"这个事实信号本身是 provider-agnostic + scheduler-agnostic 的**，应该在 waist 里有一席之地。waist 只暴露事实，策略留给 scheduler，资源回收行为是可逆的 —— 这三条加在一起决定了它是挂起态而非终态。具体 Non-Goal 边界见 §A.4。

---

## 4. 输出：LLMContextOutcome

OS 类比：一个进程在一次被 schedule 之后，只可能以下面六种方式离开 CPU。`LLMContextOutcome` 就是这些 syscall return 的并集。结构上的二分（终态 vs 挂起态）见 §3.10。

| Outcome              | OS 对应         | 是否终态 | snapshot 是否产出 |
|---------------------|----------------|----------|-------------------|
| `Done`              | `exit(0)`      | 是       | 否                |
| `Error`             | `exit(非0)`    | 是       | 否                |
| `BudgetExhausted`   | OOM kill / SIGKILL | 是   | 可选（见 partial） |
| `WaitInput`         | `read()` 阻塞  | 否       | 是                |
| `PendingTool`       | `io_submit()` 后等待 | 否 | 是                |
| `ContextLimitReached` | page fault → 等 swap | 否 | 是              |

注意 `WaitInput` / `PendingTool` / `ContextLimitReached` 都是 **cooperative yield**：LLM 在 inference 完成后才有机会让出，token stream 不会被切断在中间。这是和 OS preemptive scheduler 的关键区别 —— LLMContext 没有 timer interrupt，只有 LLM 自己说"这一段推完了，我要等外部" 或者 budget 检查在轮间发现接近阈值。


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

    /// 暂停：context window 触达阈值（由 BudgetSpec.context_yield_threshold 声明）。
    /// LLMContext 只暴露这个事实信号，**不**规定如何压缩 —— summarize / rewind /
    /// abort / 换更大窗口的模型，全部由 scheduler 决定（见 §A.4）。
    /// Resume 时 scheduler 通过 ResumeFill::RewrittenHistory(...) 喂回重整后的对话历史。
    ContextLimitReached {
        which:    ContextLimitKind,
        usage:    TokenUsage,
        /// 当前已累积的对话历史。Scheduler 据此判断如何压缩（summarize / drop oldest /
        /// 保留 system + 最近 N 轮 / ...），重写后通过 ResumeFill 喂回。
        accumulated: Vec<ChatMessage>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },
}

pub enum ContextLimitKind {
    /// 根据 BudgetSpec.context_yield_threshold 计算得出，未到 provider 硬边界。
    /// 这是最常见的情况，由 LLMContext 主循环在每轮 LLM 调用前主动检查。
    ApproachingWindow,
    /// 已经撞到 provider 实际硬边界（例 Claude 200k）。
    /// 由 provider adapter 在 inference 时探测并回传。
    HardLimit,
    /// Provider 主动拒绝（例如 OpenAI 返回 context_length_exceeded 错误）。
    /// 这是一个比 HardLimit 更晚的兜底信号，通常意味着 token 估算失准。
    ProviderRefused,
}
```

### 4.1 上层如何处理

```text
┌───────────────────────┬───────────────────────────────┬────────────────────────────
│ Outcome               │ Agent loop                    │ Workflow engine
├───────────────────────┼───────────────────────────────┼────────────────────────────
│ Done                  │ 写 step_record，按            │ 写入 node output，
│                       │ next_behavior 切换            │ 进入下一个 node
│ WaitInput             │ session 进入 WAIT_FOR_MSG     │ workflow 挂起，
│                       │                               │ 通知人工节点
│ PendingTool           │ session 进入 WAIT_FOR_EVENT   │ workflow 挂起，
│                       │（兼容现有 long task）         │ 把 pending 排到任务队列
│ ContextLimitReached   │ 走 memory v2 的 summarize-    │ 一般直接 fail / 走分支；
│                       │ and-replace，重写 history     │ 也可在 LLMWorkflowContext
│                       │ 后 resume                     │ 里声明压缩策略后 resume
│ BudgetExhausted       │ HP 不足 → END                 │ 走 retry / escalation /
│                       │                               │ fail 分支
│ Error                 │ 走 error behavior             │ 走 error handler 节点
└───────────────────────┴───────────────────────────────┴────────────────────────────
```

**关于 ContextLimitReached 的处理范式**：scheduler 在 resume 时必须通过 `ResumeFill::RewrittenHistory(messages)` 提供重写后的对话历史。这一步**会破坏原 LLMContext 的对话历史完整性**（这是 LLMContext 内部的事），但**不会破坏原始用户输入与 worklog**（这两者由 owner / scheduler 持有）。Worklog 在压缩发生时会 emit 一条 `ContextRewritten` 事件供审计。

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
| `AgentToolManager::call_tool`         | 原样复用，返回值扩展为 `Observation::{Success｜Error｜Pending(call_id)}`；Pending 时 LLMContext 产生 `Outcome::PendingTool` |
| `AICC do_inference_once`              | 原样复用 |

**核心结论**：`LLMContext` ≈ 把 `LLMBehavior::run_step_inner`（`behavior/behavior.rs:129-198`）那段循环切出来，加 owner 抽象、加 deferred / pending、加 BudgetSpec、加 OutputSpec 多态。**不引入新的 LLM 调用路径，不引入新的工具执行路径，不引入新的 worklog 通道。**

---

## 6. 关键执行流程

> TODO:要增加对自动错误处理的流程，LLM Loop允许把错误也作为一种 "观察" 进入下一轮的推理

### 6.1 一次同步执行（最常见）

```
LLMContext::new(req, deps)s
  └─> run().await
        ├─> compile_prompt()              // input --OwnerContext--> final inpu
        ├─> emit(LLMStarted)
        ├─> loop:
        │     ├─> if context_yield_threshold reached:
        │     │     return Outcome::ContextLimitReached { ... }   // §6.4
        │     ├─> do_inference_once()
        │     │     ├─> on provider HardLimit / refusal:
        │     │     │     return Outcome::ContextLimitReached { ... }
        │     │     └─> ok: continue
        │     ├─> if tool_calls.is_empty(): break
        │     ├─> gate by policy
        │     ├─> for call in calls:
        │     │     ├─> if call.tool ∈ deferred:
        │     │     │     return Outcome::PendingTool { ... }
        │     │     ├─> else: tools.call_tool() → observation
        │     │     └─> emit(ToolCallFinished)
        │     ├─> rounds_left -= 1; check budget
        │     │     └─> if hard budget exhausted:
        │     │           return Outcome::BudgetExhausted { ... }
        │     └─> rebuild request with tool observations
        ├─> parse output by OutputSpec
        ├─> emit(LLMFinished)
        └─> return Outcome::Done { ... }
```

阈值检查与硬边界探测的分工：`context_yield_threshold` 是 LLMContext 主循环主动检测的预警信号（`ApproachingWindow`），在每轮 LLM 调用**之前**先看一眼；`HardLimit` / `ProviderRefused` 是 provider adapter 在 inference 失败时回传的兜底信号。两者最终都收敛到同一种 outcome，scheduler 不需要区分（除了 `which` 字段供日志记录）。

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

#### 不变量：snapshot 自包含且跨节点可 resume

这是 waist 对 effect 层的一条**硬约束**，不是建议：

- **自包含**：给定 snapshot S 在节点 A 上产生，节点 B 只要提供等价的 `LLMContextDeps`，就必须能成功 `LLMContext::resume(S, fill, deps)`。
- **跨节点**：snapshot 必须可序列化（建议 < 32KB；超出时由调用方走外部存储 + 在 snapshot 里只放引用 ID）。
- **不持有 effect side 真实世界状态**：snapshot 只持有逻辑层产物（call_id / observation / accumulated messages / token usage），**绝不**持有 tmux session 句柄、文件描述符、network connection、容器 PID 等。
- **"等价 deps"由实现层定义，waist 不解释**：例如新节点上的 ToolManager 如何处理"原节点的 tmux session 在我这里不存在"，是 ToolManager 的策略选择（拒绝 / 重建 / 透明迁移），waist 不规定。
- **ContextSources 的 freeze 协议**：`PromptSpec::Compiled` 模式下的 `ContextSources` 是 trait（带回调），跨节点 resume 时原 trait 实例可能不可用。waist 要求：snapshot 在产生时把 `ContextSources` **freeze** 成具体的 `FrozenContextSources`（拍平的字符串数据）。Resume 时如果新节点能提供原 `ContextSources` 就用原的，否则降级到 frozen 形态。这条约束让 §3.9 的 trait 设计在跨节点场景下站得住。

> **工程提醒**：开工时如果你发现自己想往 snapshot 里塞"句柄"、"指针"、"长生命态引用"，停下来 —— 那些东西属于 `LLMContextDeps`（由 scheduler 重新提供），不属于 snapshot。

### 6.3 WaitInput

LLM 在 reply 中显式声明需要人工输入（通过 `OutputSpec::Json` 里的 `request_human_input` 字段，或者 `BehaviorLLMResult` 里的 reply）。`LLMContext` 检测到这个信号，构造 `Outcome::WaitInput`。`HumanPolicy.allow_request_input = false` 时降级为 `Outcome::Done`，让上层自己决定。

### 6.4 ContextLimitReached 挂起 / 恢复

```
[Agent loop（典型场景：长会话）]
   ├─ ctx = LLMContext::new(req, deps)
   │      where req.budget.context_yield_threshold = Some(Ratio(0.85))
   ├─ outcome = ctx.run().await
   ├─ match outcome {
   │      ContextLimitReached { which, accumulated, snapshot, .. } => ...
   │  }
   ├─ persist snapshot
   ├─ # 调用 scheduler 自己的压缩策略
   ├─ rewritten = memory_v2.summarize_and_replace(accumulated)
   │              // 例：把前 N 轮压成一个 system summary block，保留最近 K 轮
   ├─ emit_worklog(ContextRewritten { from_messages: accumulated.len(),
   │                                   to_messages: rewritten.len(),
   │                                   reason: which })
   │
   ├─ ctx2 = LLMContext::resume(snapshot,
   │           ResumeFill::RewrittenHistory(rewritten), deps)
   └─ outcome2 = ctx2.run().await   // 用重整后的 history 继续
```

**关键纪律**（与 §3.10 一致）：

- **LLMContext 自己绝不调用压缩**。它只产生 `ContextLimitReached`，把 `accumulated` 完整暴露给 scheduler。任何压缩都发生在 LLMContext 之外，由 scheduler（典型实现：`LLMAgentContext` 调用 Agent memory v2 / `LLMWorkflowContext` 直接 fail-and-escalate）决定。
- **重写会破坏原 LLMContext 的对话历史，但不破坏 worklog 与原始用户输入**。原始输入由 owner 持有（`AgentSession.history` / `WorkflowInstance.node_input`），worklog 是 append-only 的事件流（包括 `ContextRewritten` 事件），都不受影响。这条让 ContextLimitReached 的处理保持可审计。
- **Resume 后 token usage 从重整后的 history 重新累计**。如果 scheduler 设了 `max_total_tokens` 红线，压缩后的新 history token 数会被算入累计，避免"无限压缩 + 无限运行"的恶性循环 —— 一旦累计撞红线，仍然走 `BudgetExhausted` 终止。
- **scheduler 也可以选择不 resume**。例如 workflow 收到 `ContextLimitReached` 后决定 fail 当前 node、走 retry 分支用更大窗口的模型重跑，这是合法的，因为 `ContextLimitReached` 是挂起态、不是强制 resume。

---
## 8. 模块划分与文件落点

```
src/frame/llm_context/src/                   //
│   ├── mod.rs              // pub use
│   ├── request.rs          // LLMContextRequest / PromptSpec / ToolPolicy / OutputSpec / BudgetSpec / HumanPolicy
│   ├── outcome.rs          // LLMContextOutcome / ContextOutput / LLMContextSnapshot / ResumeFill
│   ├── owner.rs            // ContextOwnerRef / ContextSources trait / FrozenContextSources
│   ├── deps.rs             // LLMContextDeps（tools / policy / worklog / tokenizer / aicc / taskmgr）
│   ├── state.rs            // LLMContextState / LLMContextSnapshot（可序列化的可变态）
│   ├── context_loop.rs          // LLMContext::{new, run, resume, snapshot}，实现核心的Loop
│   └── tests.rs

```

---

## 9. 与现有设计文档的关系

| 文档 | 关系 |
|------|------|
| **`LLMAgentContext 设计.md`**（姊妹文档，待写） | L4 scheduler-facing 层，Agent 一侧。承接所有 Agent role md / behavior 配置可见的字段，lowering 到本文档定义的 LLMContext。 |
| **`LLMWorkflowContext 设计.md`**（姊妹文档，待写） | L4 scheduler-facing 层，Workflow 一侧。承接所有 workflow DSL 可见的字段（service endpoint / 上下游引用 / on_* 分支），lowering 到本文档定义的 LLMContext。 |
| `OpenDAN Agent Runtime 设计.md` | LLMContext 是该文档"Behavior Loop"小节的下沉抽象，原文档需要新增"L2 / L3 / L4 分层"段落引用本文档与 `LLMAgentContext 设计.md` |
| `Agent Session.md` | AgentSession 仍然是 Agent 的长上下文持有者，新增"实现 ContextSources trait（含 freeze）"约束；它通过 `LLMAgentContext` 间接驱动 LLMContext |
| `Agent Prompt Compiler.md` | PromptBuilder 改为接收 `Arc<dyn ContextSources>` 而非 `Arc<Mutex<AgentSession>>` |
| `Agent Worklog.md` | 不变。LLMContext 是 worklog 的生产者，沿用 `AgentWorkEvent` |
| `OpenDAN Long Task & Sub-Agent.md` | `Outcome::PendingTool` 是 long task 在 LLMContext 层的统一表达；sub-agent 创建走"同步创建 + 异步执行"约定（§11） |
| `opendan关键类型.md` | 新增 `LLMContext / LLMContextRequest / LLMContextOutcome / ContextSources / FrozenContextSources` 章节 |

---



## 10. 一句话总结

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
- **scheduler-facing 的语义字段**（**新增**）—— 任何"DSL 用户可见 / 配置文件可写"的字段都不进 waist，必须落在对应 scheduler 的 L4 `LLM*Context` 里。例：service_endpoint 引用、`${prev_node.output.x}` 上游引用、on_budget_exhausted 分支策略、role_md 里的角色描述、Jarvis behavior 配置、hook trigger debounce、eval ground truth、tmux session 句柄。判定方法：如果一个字段**在 DSL/配置文件里被人直接写出来**，它一定属于 L4，不属于 waist。
- **运行期动态修改 tool list**（**新增**）—— 一旦 `LLMContext::new` 完成，工具集合在该实例生命周期内不变。"中途加工具"是 scheduler 的诉求：销毁当前 LLMContext，构造一个新的，等价于换工具集；waist 不提供运行期修改 tool list 的接口。理由：动态修改会破坏 snapshot 可重放性、破坏 worklog audit、破坏 cooperative yield 的语义不变量。

### A.2 Provider-specific（永远不进 waist）

- 模型计费 / billing 字段 —— provider 自己 telemetry，与 waist 解耦
- provider 专属参数（anthropic `cache_control`、openai `seed`、gemini `safety_settings`）—— 通过 `model_policy.provider_options: opaque` **透传**，waist 不解释、不校验
- 模型能力探测（context window 大小 / 是否支持 vision / tool）—— provider adapter 内部决定，waist 不暴露
- token 计费方式（input vs output 不同价、cached vs uncached 不同价）—— 用 `TokenUsage` 抽象，不暴露 provider 计费规则
- streaming 协议细节（SSE / chunked / batch）—— provider 适配层处理；waist 一次推理对外是原子的
- **function call 作为 loop 强制协议**（**新增**）—— 拒绝。Function call 是 provider-specific 的 wire format（OpenAI tool_calls / Anthropic tool_use block / Gemini function_call 各家细节都不同；本地模型经常根本没有原生支持）。通过 ToolManager 与 provider adapter 内部归一化处理，**不进 waist**。waist 的 loop 不变量是 §0.2 定义的 intent / effect / observation 三元组，effect 的承载形态由 `OutputSpec` 声明（structured output 里的 actions 数组 / provider-native tool_calls 皆可），不强制任何一种 wire 编码。详见 §0.2 与 §3.5 末段。

### A.3 Container / 长生命态（属于 AgentSession，不属于 LLMContext）

- session memory / 长期记忆 —— 通过 `ContextSources::memory_block()` 按需注入，LLMContext 自己不持有
- workspace 路径 / 文件挂载 —— 容器关心，进程不关心；通过工具调用访问
- agent identity / DID / 签名密钥 —— 容器属性
- 持久 worklog 的存储位置 / 索引策略 —— 上层 worklog 服务关心，LLMContext 只负责 emit 事件
- sub-agent registry / lifecycle —— 容器编排关心
- 跨 LLMContext 的对话历史拼接 —— 容器在外面拼，传进来时已经是 `ContextInput`
- **执行环境绑定**（机器 / 容器 / tmux session 句柄）（**新增**）—— 属于 AgentSession 的容器编排或 workflow node_daemon 的调度结果，LLMContext 通过 ToolManager 间接访问，不持有任何句柄。
- **`AgentBash → LLMContextBash` 直接映射**（**新增**）—— 拒绝。LLMContext 没有 bash 概念，bash 是 ToolManager 内部的一种 tool 实现；具体 bash 跑在哪个 tmux/容器里，是 ToolManager 实现的事，对 waist 不可见。

### A.4 Effect-side 持久化与执行策略（属于 EffectDeps 实现，不属于 waist）

- snapshot 的存储介质 / 加密 / 跨节点复制策略 —— `SnapshotStore` 接口的实现细节
- worklog sink 的具体实现（SQLite / 远程 / Kafka）—— `WorklogSink` 接口的实现细节
- tool 调用的审计 / 录像 / replay —— `ToolManager` 实现的可选行为
- pending tool 的任务队列后端（in-memory / Redis / BuckyOS task service）—— scheduler 决定
- tool 调用的并发 / 限流 / 熔断 —— `ToolManager` 实现，waist 只声明 `parallel: bool` 这种意图
- **BuckyOS 服务的 kRPC CLI 化策略**（**新增**）—— 是 ToolManager 暴露 BuckyOS 资源的内部决策，与 waist 解耦；具体把哪些 kRPC 接口包成什么样的 tool schema，由 effect 实现层决定。
- **系统状态路径化 / read_file 抽象**（**新增**）—— 同上，是工具实现的对外协议，不是 waist 字段。
- **LLMContext 的承载方式**（in-process lib / workflow thunk / AICC RPC）（**新增**）—— 是 scheduler 根据部署语义的选择，不是 waist 的属性。waist 不规定也不偏向任何承载方式，三种共享 100% 的执行语义（见 §2.3）。
- **上下文压缩策略**（summarize prompt / sliding window / hierarchical recall / drop-oldest / 换模型重灌）（**新增**）—— 拒绝进 waist。waist 只暴露 `Outcome::ContextLimitReached` 这个**事实信号**（见 §3.10 / §4 / §6.4），具体压缩算法属于 scheduler 在 resume 时通过 `ResumeFill::RewrittenHistory(...)` 提供的策略。典型实现：`LLMAgentContext` 走 Agent memory v2 的 summarize-and-replace；`LLMWorkflowContext` 一般直接 fail-and-escalate；Eval scheduler 可能选择 hard-truncate 观察模型在压力下的行为。任何"在 waist 里规定如何压缩"的字段都会破坏 scheduler 中立性，应当退回到对应的 L4 `LLM*Context`。

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