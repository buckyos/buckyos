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

LLMContext 是 waist，**不是 scheduler-facing 的语义层**。DSL 编写者、角色 / 行为配置编写者、配置文件编写者**永远不直接接触 LLMContext**，他们接触的是各 scheduler 自己的一等公民语义层。典型例子：

- `LLMWorkflowContext` —— Workflow DSL 直接配置的对象，知道 service endpoint、上游 node 引用、retry / fallback 分支。
- `LLMAgentContext` —— Agent 角色 / 行为定义直接产生的对象，知道长生命会话、状态机、容器句柄、工具清单。
- `LLMOneShotContext` —— 一次性 CLI 调用产生的对象，知道 cwd、命令行参数、stdout 重定向。
- 其他 scheduler（Hook / Eval / Multi-agent debate / Pipeline 等）按需各自定义自己的 `LLM*Context`。

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
| **intent**（意图） | `OutputSpec::Json` 解析出的结构化产物；其中的 `tool_calls: Vec<AiToolCall>` / `actions` 字段（`AiToolCall` 见 §5） | LLM（输出端） | LLMContext 主循环 → ToolManager |
| **effect**（执行） | `ToolManager::call_tool` 内部的实际动作（CLI 调用 / kRPC / native fn / async task 提交） | ToolManager（effect 实现层） | 外部世界 |
| **observation**（观察） | `Observation::{Success | Error | Pending(call_id)}` | ToolManager | LLMContext 主循环 → 喂回下一轮 LLM |

**Effect 在 waist 上有两种合法承载形态**：

1. **结构化 output 里的意图集合** —— LLM 通过 `OutputSpec::Json` 在 reply 中显式声明一组带意图的 effect 请求（典型字段：`tool_calls: Vec<AiToolCall>` / `actions: Vec<...>`），每个 effect 都是一段语义完整、可被人类读懂的"想做什么"。具体 schema 形态由 L4 `LLM*Context` 决定，waist 只看到归一化后的 `AiToolCall` 列表。
2. **provider-native tool_calls** —— 仅在 provider 支持且 `ToolPolicy.mode != None` 时启用，由 provider adapter 解析为 `AiResponseSummary.tool_calls`，与第 1 种形态使用同一组 `AiToolCall` 类型。

两种形态在 waist 内部经由 ToolManager **归一化为同一组 `Observation`**。"哪种 provider 走哪种 wire format"由 provider adapter 自己决定，waist 不偏向、不强制。

**为什么这个不变量值得作为 waist 纪律**：

- **意图先于格式**：waist 的设计哲学要求每个 loop step 都能被审计、被人理解、被 worklog 真实地复述。Function call 那种"只有调用没有意图"的协议，把"为什么调"的语义全部塞进了 system prompt 的灰色地带，破坏了 waist 的可观测性承诺。
- **格式不应硬编码**：把 function call 钉进 waist 立刻丢掉 provider 中立性。本地模型走 grammar-constrained decoding、Agent 行为机走 XML、workflow 简单节点走 JSON schema —— 同一个 effect 概念有多种 wire 编码完全正常。
- **与 §3.10 终态/挂起态二分自洽**：observation 的 `Pending` 状态正是 cooperative yield 的载体，把它和 function call 解耦后，"哪些工具是异步的"完全是 effect 实现层的私事，waist 只看见 `Observation`。

**PR review 落点**：任何想把"function call schema / tool_use block / OpenAI strict mode"这类 wire-format 概念引入 waist 公共类型的提议，按本节直接退回到 ToolManager 与 provider adapter 实现层。具体条目见 §A.2。

## 1. 背景与动机

### 1.1 LLM 执行栈缺少的中间层

当一个系统需要把 LLM 接入更大的应用编排（workflow、Agent、shell、hook、eval、multi-agent…）时，业界事实上只有两个粒度可用：

- **`llm.complete(prompt) → text`** —— 太低阶。每个调用点都要自己拼 prompt、自己实现 tool loop、自己处理 retry / budget / 结构化输出 / 人工介入，最后等于"在每个 scheduler 里各重写一个迷你 agent runtime"。
- **`agent.sendMsg(session, msg) → ...`** —— 太重型。一来就绑定长生命会话、行为机、长期记忆、容器编排，scheduler 只是想"跑一次 LLM + 几个工具"也得吞下这一整套。

两者之间缺一层"**进程粒度**的 LLM 执行体"：

- 有 agent runtime 的核心能力（prompt 已编译 / 工具循环 / 预算管理 / 结构化输出 / cooperative yield / 审计）
- 但**没有** session 的长生命语义、行为状态机、容器编排
- 可以被多种 scheduler 一致地构造、调度、挂起、恢复

`LLMContext` 就是这个中间层。它要做的不是"再封一个 agent"，而是把 agent / workflow / shell / hook 都共同需要的"一次有界 LLM 执行"语义**抽成一个原语**，让上面各 scheduler 自由演化、下面各 provider 自由实现。

### 1.2 目标

`LLMContext` —— **一次性、短生命、可被多种 owner（Agent / Workflow / Shell / Hook / Eval / OneShot…）构造的有界 LLM Loop 执行单元**：

1. **不绑定任何一种 scheduler 的状态机**：输出 schema 由调用方声明（§3.6），不预设"下一个行为是什么"这类调度语义。
2. **不绑定长生命会话**：waist 只看到 `ContextOwnerRef`（owner 身份）+ 已展开的 `Vec<AiMessage>`（input / 历史 / memory block 在 L4 lowering 阶段全部展开），不接触 session、容器、模板环境。
3. **统一的退出语义**：`done / wait_input / pending_tool / context_limit_reached / budget_exhausted / error` 六态，便于上层 scheduler 一致地推进状态机。结构上分为终态与挂起态两类，详见 §3.10。
4. **统一的可观测性**：每一步的 LLM 输入、tool 调用、token 用量、错误信号都以稳定 schema 沉淀（§4 `ContextRunTrace`），上层 worklog / tracing 系统按需消费。
5. **承载方式不进 waist**：同一份执行语义可以以 in-process lib / workflow thunk / 跨设备 RPC 任意承载（§2.3），承载方式由 scheduler 选，不是 waist 属性。

### 1.3 非目标

- 不替换"长生命会话"：长上下文、状态机、行为切换由各 Agent 实现自己持有
- 不替换工作流引擎：DAG 调度、人工节点、retry / fallback 分支由 workflow engine 持有
- 不引入新的 LLM provider 抽象：底层 LLM 调用走既有 provider 层（参考实现里是 BuckyOS 的 aicc，见 Appendix B）
- 不定义可观测性的存储后端：waist 只生产事件、不规定 sink；具体落 SQLite / Kafka / 远程服务都是 effect 实现的事

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
│  - Agent loop          消息驱动，长生命                      │
│  - Workflow engine     DAG / 状态机驱动，长生命              │
│  - OneShot scheduler   一次性脚本 / CLI                      │
│                                                             │
│   构造 LLMContextRequest（创建进程），根据 Outcome 推进：   │
│   exit / yield / kill —— 即调度器的标准动作                 │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│  L2  LLMContext 层（OS 类比：进程上下文）                   │
│  - 一次有界 LLM 执行：消息历史 → LLM 调用 → tool loop        │
│  - 结构化输出 / token 用量 / 抽象成本计费 / policy gate     │
│  - 六态退出：done / wait_input / pending_tool /             │
│              context_limit_reached / budget_exhausted /     │
│              error  （二分：终态 / 挂起态，见 §3.10）       │
│  - 可 cooperative yield / 可 resume（LLMContextSnapshot）   │
└──────────────────────────┬──────────────────────────────────┘
                           │
┌──────────────────────────▼──────────────────────────────────┐
│  L1  Raw LLM 层（provider adapter）                          │
│  - 一次推理 request → response                              │
│  - 不做 tool loop / 不做可观测性 / 不做 policy               │
│  - 适合分类、摘要、结构化抽取等"可一次说完"的任务           │
└─────────────────────────────────────────────────────────────┘
```

本设计**新增 L2，并显式要求每个 scheduler 在 L4 建立自己的 `LLM*Context`**，把 L1↔L3 的直连改造成 L1↔L2↔L4↔L3。等价地，把"scheduler 直接 syscall（`llm.complete`）"和"scheduler 直接 fork-exec 一个完整 agent 进程（`agent.sendMsg`）"中间，加进真正的"进程上下文"层，并把"DSL / 配置文件用户面对什么"这层显式化。

### 2.1 L3 vs L4 的区别

L3 是**命令式控制流**（Agent loop / Workflow engine 的运行时代码），L4 是**声明式 schema**（YAML / TOML / role md 描述）。同一个 scheduler 同时拥有 L3 和 L4 两块产物：L4 描述"这个 LLM 调用长什么样"，L3 描述"什么时候调它、Outcome 怎么处理"。

### 2.2 为什么 L4 必须显式存在（不能藏在 builder 里）

1. **L4 有自己的生命周期**：Def 编译期 → Instance 实例化 → lowering → LLMContext 跑完 → Instance 处理 outcome → 节点完整结束。L4 比 LLMContext 活得长。
2. **L4 的可序列化形态 ≠ LLMContext 的可序列化形态**：L4 持有 symbolic 引用（`endpoint: kRPC://...`、`${prev_node.output.x}`），LLMContextRequest 持有 resolved 句柄（已绑定的 ToolManager、已展开的 prompt）。lowering 是这两种形态之间的转换。
3. **L4 是 §A.1 / §A.3 Non-Goals 的栖息地**：所有 scheduler-specific 的字段（service endpoint / 上下游引用 / on_budget_exhausted 分支策略 / 行为状态机配置 / 容器与 session 句柄...）必须有地方放。L4 就是它们的家，否则它们会被塞进 waist。

### 2.3 承载方式（部署形态）

LLMContext 是一个 lib（`llm_context` crate），**不是一个 service**。它有三种承载方式：

1. **In-process lib 调用** —— L4 lowering 后直接在 owner 所在进程内调 `LLMContext::run`。稳定、自由度高、无序列化代价。适合 Agent / Shell 等已有自己进程的 scheduler。
2. **Thunk 承载** —— L4 lowering 后封装为可序列化 thunk，由 workflow runtime / 任务编排器调度到指定执行器执行。snapshot 持久化、跨节点迁移等工程问题由编排器统一解决。适合 workflow LLM node 这类需要 DAG 调度的场景。
3. **跨设备 RPC 承载** —— 把 `LLMContextRequest` 序列化送到另一台机器跑、把 `LLMContextOutcome` 序列化回来。适合"端上发起、云端执行"或反过来的拓扑。

三种方式共享 100% 的执行语义，差异只在 deps 注入和 outcome 投递路径上。**承载方式由 scheduler 根据部署语义选择，不是 waist 的属性，也不是二选一**。

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
    deps:    LLMContextDeps,   // tools / policy / worklog / tokenizer / llm provider
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
    /// CallId 与 PendingToolCall.call_id（即 AiToolCall.call_id）一一对应。
    ToolResults(Vec<(CallId, Observation)>),
    /// 对应 WaitInput：上层把人工输入填回（`AiMessage` 见 §5）。
    HumanInput(AiMessage),
    /// 对应 ContextLimitReached：上层把重整后的对话历史填回。
    /// LLMContext 用这份新 history 替换 state 里的 accumulated messages 后继续。
    /// 这里的"重整"语义完全由 scheduler 决定：summarize / drop oldest /
    /// hierarchical recall / 换模型重灌 system prompt ... waist 不介入。
    RewrittenHistory(Vec<AiMessage>),
}
```

### 3.2 输入：LLMContextRequest

`LLMContextRequest` 是 LLMContext 的不可变输入。**消息载体直接复用 provider 抽象层的 `AiMessage`**（语义见 §5；参考实现里类型路径是 `buckyos_api::AiMessage`），不在 waist 再造一份 `ChatMessage / ContextMessage` —— waist 与 provider adapter 共享同一种消息类型，让"从 waist 喂到 provider 请求"这条路径在类型层零拷贝。

```rust
pub struct LLMContextRequest {
    /// 上层 owner 标识，用于 worklog / tracing / 审计
    pub owner: ContextOwnerRef,           // Agent(session_id) | Workflow(instance, node) | OneShot(id)
    pub trace: Option<String>,            // 调试用 trace id，可空

    /// 自然语言目标，供 worklog / 审计阅读，不进 prompt。
    pub objective: String,

    /// 已经编译好的对话历史（含 system / user / assistant / tool）。
    /// L4 的 prompt compiler 负责把模板、角色描述、长期记忆等展开成具体
    /// AiMessage 序列后填入。waist 不解析模板、不接触 session，只把 input
    /// 透传给 provider。
    pub input: Vec<AiMessage>,

    /// 模型策略（路由偏好、温度、回退链等），由 waist 定义在 llm_context crate 内
    pub model_policy: ModelPolicy,

    /// 可用工具与工具策略
    pub tool_policy: ToolPolicy,          // 见 3.5

    /// 输出契约
    pub output: OutputSpec,               // 见 3.6

    /// 资源边界
    pub budget: BudgetSpec,               // 见 3.7

    /// Human-in-the-loop 策略
    pub human_policy: HumanPolicy,        // 见 3.8

    /// 错误处理策略（错误 → observation 喂回 vs 挂起等人工修复）
    pub error_policy: ErrorPolicy,        // 见 3.11
}
```

设计要点：

- **不持有 session / 容器句柄**：所有"模板变量从哪里来"的问题都在 L4 lowering 阶段解决，传到 waist 时 `input` 已经是展开完毕的 `Vec<AiMessage>`。`ContextSources`（§3.9）只在 L4 `LLM*Context` 内部使用，不出现在 `LLMContextRequest` 公共字段里。
- **不重复 provider 抽象已有的类型**：消息走 `AiMessage`、用量走 `AiUsage`、tool call 走 `AiToolCall`、最终 LLM 响应走 `AiResponseSummary`（见 §4）。这些类型是 waist 与 provider 层共用的"边界类型"，避免两边各定义一份再来回转换。参考实现（BuckyOS aicc）见 Appendix B。



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
    /// 是否允许 deferred 工具（返回 Pending(call_id) 触发 cooperative yield）。
    pub allow_deferred: bool,
}
```

工具执行委托给 `ToolManager` trait（具体实现可以是 Agent 进程内的工具管理器、workflow 的 service router、独立 sandbox 等），policy gate 委托给 `PolicyEngine` trait（具体实现可以是黑/白名单、ACL、动态审批…）。waist 只看到这两个 trait，不知道实现细节。

`ToolManager::call_tool` 接受 `AiToolCall` 作为输入，返回归一化的 `Observation`：

```rust
use buckyos_api::AiToolCall;

pub enum Observation {
    Success { call_id: String, content: serde_json::Value, bytes: usize, truncated: bool },
    Error   { call_id: String, message: String },
    /// Effect 层声明"这次调用是异步的，结果会通过外部回调喂回"。
    /// waist 据此产生 Outcome::PendingTool（cooperative yield）。
    Pending { call_id: String },
}

#[derive(Clone)]
pub struct PendingToolCall {
    /// 直接复用 §5 的 AiToolCall —— 它已经携带 name / args / call_id 三件套。
    pub call: AiToolCall,
    /// effect 层声明的预计就绪时间，供 scheduler 决定 deadline。
    pub eta_ms: Option<u64>,
}
```

**Pending 语义**：
- 当 `tool_policy.allow_deferred = true` 且某次 `call_tool` 返回 `Pending(call_id)`，LLMContext 立即产生 `Outcome::PendingTool`，把已完成的非 pending observation 一并带回（不强制等齐整轮再挂起）。
- 当 `allow_deferred = false`，ToolManager 不应返回 Pending；如果返回了，LLMContext 视为 `Outcome::Error`。

这条把"哪个工具是异步的"的决定权交给 effect 层（具体的工具实现自己根据语义决定），waist 只控制"是否允许"。

**关于 tool 调用的 wire format**（与 §0.2 loop 不变量配套）：waist **不规定** LLM 如何在输出里编码 tool 调用意图 —— OpenAI tool_calls / Anthropic tool_use block / 自定义 XML / `OutputSpec::Json` 里的 actions 数组 / grammar-constrained decoding 皆可。这是 ToolManager 与 provider adapter 协商的私事。Provider adapter 负责把各家原生 wire format 翻译成 `AiResponseSummary.tool_calls: Vec<AiToolCall>`；waist 只接触归一化后的 `Vec<AiToolCall>` 与 `Vec<Observation>`。这条边界使得"换 provider"和"换 effect 实现"互不打扰。

### 3.6 OutputSpec / ContextOutput

```rust
pub enum OutputSpec {
    /// 自由文本，调用方自己解析
    Text,
    /// 强制 JSON，可校验 schema
    Json { schema: Option<serde_json::Value>, strict: bool },
}

/// LLMContext 在 Outcome::Done 里产出的"解析后产物"。waist 自己不知道字段含义，
/// 只按 OutputSpec 做最小限度的类型校验；具体 schema 由 L4 LLM*Context 解释。
pub enum ContextOutput {
    Text(String),
    Json(serde_json::Value),
}
```

waist **不内置任何 scheduler-specific 的复合输出类型**（见 §A.1）。例如某个 Agent 实现需要 `actions / next_state / set_memory` 这类行为机字段，由它对应的 L4 `LLM*Context` 在 lowering 时声明 `OutputSpec::Json { schema: ... }`，再在收到 `ContextOutput::Json` 后自己 deserialize 成具体业务类型。

LLM 一侧的"原始响应"则由 provider adapter 包成 `AiResponseSummary` 透传给上层（见 §4 `Outcome::Done.response`），其中包含 `text / tool_calls / artifacts / usage / cost / finish_reason / provider_task_ref`，waist 不重新发明这一层。

### 3.7 BudgetSpec

```rust
pub struct BudgetSpec {
    pub max_total_tokens:     Option<u32>,
    pub max_completion_tokens: Option<u32>,
    pub max_wallclock_ms:     Option<u64>,
    pub max_cost_units:       Option<u32>,   // scheduler 定义的抽象成本单位（credit / quota / "HP" 等）
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

### 3.9 Outcome 的二分：终态 vs 挂起态

> 本节是"显式大于隐式"原则在 Outcome 设计上的硬约束。新增 Outcome 变体必须明确归入下面其中一类，并满足对应的不变量。

LLMContext 的 Outcome 在结构上分成两类：

**【终态】**（LLMContext 对象消耗完毕，**不可** resume）

| Outcome | 语义 | OS 类比 |
|---|---|---|
| `Done` | 正常退出，产出 `ContextOutput` | `exit(0)` |
| `Error` | 异常退出，产出 `LLMComputeError` | `exit(非0)` |
| `BudgetExhausted` | 预算红线击穿（token / wallclock / cost units / tool rounds）| `SIGKILL` / OOM kill |

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

### 3.11 ErrorPolicy

> 本节是对 Loop 中**错误处理**的统一约定。它与 §3.9 终态/挂起态二分配套：错误 **不** 被默认当成"无法继续"，而是先经过分类 + 策略才决定走"喂回 observation"、"挂起待修"还是"直接终态"。具体执行时序见 §6.5。

```rust
pub struct ErrorPolicy {
    /// 默认错误处理模式（仅作用于 Recoverable 错误；Fatal 错误恒走 Outcome::Error）
    pub mode: ErrorMode,

    /// 同一 call_id 或同一类错误连续发生多少次仍未恢复，自动降级为挂起。
    /// 防止 FeedAsObservation 模式下陷入"调错 → 看到 → 再调错"的死循环空烧 token / cost。
    /// 0 = 不限制（不推荐）。
    pub max_consecutive_errors: u32,
}

pub enum ErrorMode {
    /// 任何 Recoverable 错误立即产生 Outcome::WaitInput，
    /// reason 里结构化携带 ErrorClass + 错误源（tool name / call_id / provider task ref）。
    /// 适合"高可信编排"：workflow 关键节点、生产 pipeline、强可审计场景。
    Suspend,

    /// 把 Recoverable 错误归一化成一条 role=tool 或 role=system 的 AiMessage 追加到 accumulated，
    /// 作为下一轮 LLM 的"观察"喂回，由 LLM 自己看着错误信息决定重试 / 改参 / 放弃 / 请求人工。
    /// 适合"自治 agent loop"：让 LLM 真正"看到"它自己犯的错。
    FeedAsObservation,
}

/// 所有进入 waist 主循环的错误，先归一化为 LLMComputeError，再分类为下面两类。
pub enum ErrorClass {
    /// 可作为下一轮 observation 喂回，或挂起等人工修复后 resume。
    Recoverable(LLMComputeError),
    /// 不可恢复，直接走 Outcome::Error 终态，不可 resume。
    Fatal(LLMComputeError),
}
```

**错误来源与默认 Class**（waist 自带的最小分类，可被 effect 实现层细化）：

| 错误来源 | 典型例子 | 默认 Class | 备注 |
|---|---|---|---|
| LLM 输出格式错误 | `OutputSpec::Json` 校验失败、找不到合法 tool_calls 字段、JSON parse 失败 | Recoverable | LLM 看到 schema 错误后通常能自我修正 |
| 工具调用参数错误 | tool 名不存在 / args schema mismatch / required 字段缺失 | Recoverable | ToolManager 返回 `Observation::Error`，正是为这类错误设计 |
| 工具执行错误 | tool 执行抛错（subprocess 非 0 退出 / RPC 业务错误 / 超时） | Recoverable | 同上 |
| PolicyEngine 拒绝 | tool 调用被 policy gate 拦下、需要审批 | Recoverable | Suspend 模式下挂起等审批；FeedAsObservation 模式下让 LLM 知道被拒并改方式 |
| Provider 临时不可用 | network timeout / rate limit / 5xx | Recoverable | 由下层 provider 容错层先重试，重试耗尽后才上抛到 waist |
| Provider 永久错误 | 鉴权失效 / 模型 ID 不存在 / API 协议不兼容 / quota 永久超限 | Fatal | 上层修不了，直接 Outcome::Error |
| Snapshot / state 损坏 | resume 时 snapshot 反序列化失败、call_id mismatch、accumulated messages 不一致 | Fatal | 数据已经回不去，强行继续会污染审计流 |
| Budget 红线击穿 | token / wallclock / cost units / tool rounds 超限 | (n/a) | 走 `Outcome::BudgetExhausted`，已在 §3.9 单独建模 |
| Context window 撞顶 | 超过 provider 硬边界 | (n/a) | 走 `Outcome::ContextLimitReached`，已在 §3.9 单独建模 |

**Provider 容错与 waist 的边界**：provider adapter 内部应当持有 retry / fallback chain（参考实现：BuckyOS aicc 的多 provider 路由、模型降级、网络重试），把瞬时错误吸收掉。waist 看到的"provider 错误"已经是容错层兜底失败的结果。**waist 自己绝不在外面再做一层 retry**，否则等于双重重试策略，既影响可观测性也容易产生重复扣费。

**默认值的归属**：`ErrorPolicy` 的默认值由 L4 `LLM*Context` 在 lowering 阶段决定，waist 不内置默认：

- `LLMAgentContext` 倾向 `mode = FeedAsObservation` —— Agent 的常态就是"自我修复 loop"。
- `LLMWorkflowContext` 倾向 `mode = Suspend` —— workflow 节点强调可预测性，错就 escalate 给人 / 走 retry 分支。
- `LLMOneShotContext` 倾向 `mode = Suspend`。

**纪律**：

- **Fatal 错误的 Class 不可被 ErrorPolicy 改写**。`mode = FeedAsObservation` 不会让 snapshot 损坏变得"可观察" —— Fatal 永远是 Fatal。
- **错误事件流必须 emit**。无论 mode 如何，错误都通过 `LLMContextDeps.worklog` 落一条事件（`ToolCallFailed` / `LLMInferenceFailed` / `OutputParseFailed` / ...），保证审计性，与 §3.9 "显式大于隐式"一致。
- **Recoverable → 喂回的 AiMessage 形态由 effect 层决定**。waist 只规定"必须是合法 AiMessage 且 role ∈ {tool, system}"，不规定错误 message 的 wire format（JSON / 自然语言 / 结构化 envelope 都行）。

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
// AiMessage / AiUsage / AiResponseSummary / AiToolCall 等"边界类型"定义见 §5。
// 参考实现路径在 Appendix B。

pub enum LLMContextOutcome {
    /// 终态：成功
    Done {
        //可选的终止理由 
        reason:Option<String> 
        /// 按 OutputSpec 解析后的产物（Text / Json）
        output: ContextOutput,
        /// 累计 token 用量（input / output / total）
        usage: AiUsage,
        /// 最后一次 LLM 推理的原始响应摘要，含 tool_calls / artifacts / cost /
        /// finish_reason / provider_task_ref 等 provider-agnostic 字段
        response: AiResponseSummary,
        /// waist 自己额外记录的执行级 trace（trace_id / latency / tool 调用流水）
        trace: ContextRunTrace,

    },

    /// 暂停：等待人工输入或外部消息
    WaitInput {
        reason: String,
        prompt_to_human: Option<String>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },

    /// 暂停：触发了 deferred 工具，由上层异步喂回结果
    PendingTool {
        /// 每一项内部直接持有 AiToolCall（name + args + call_id，见 §5）
        pending: Vec<PendingToolCall>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },

    /// 终态：预算/边界耗尽
    BudgetExhausted {
        which: BudgetKind,               // Tokens | Wallclock | CostUnits | ToolRounds
        partial: Option<ContextOutput>,  // BudgetAction::ReturnPartial 时填
        usage: AiUsage,
    },

    /// 终态：错误
    Error {
        error: LLMComputeError,          // waist 自己定义的错误枚举（见 §5）
        usage: AiUsage,                  // 即使出错也回报已消耗的 token
    },

    /// 暂停：context window 触达阈值（由 BudgetSpec.context_yield_threshold 声明）。
    /// LLMContext 只暴露这个事实信号，**不**规定如何压缩 —— summarize / rewind /
    /// abort / 换更大窗口的模型，全部由 scheduler 决定（见 §A.4）。
    /// Resume 时 scheduler 通过 ResumeFill::RewrittenHistory(...) 喂回重整后的对话历史。
    ContextLimitReached {
        which: ContextLimitKind,
        usage: AiUsage,
        /// 当前已累积的对话历史。Scheduler 据此判断如何压缩（summarize / drop oldest /
        /// 保留 system + 最近 N 轮 / ...），重写后通过 ResumeFill::RewrittenHistory 喂回。
        accumulated: Vec<AiMessage>,
        snapshot: LLMContextSnapshot,
        deadline_ms: Option<u64>,
    },
}

/// waist 自己生成的执行 trace；与 AiResponseSummary 的"单次推理产物"互补，
/// 记录"这次 LLMContext.run 跑了多久 / 触发了哪些 tool / 路由到了哪些 task"。
pub struct ContextRunTrace {
    pub trace_id: String,
    pub latency_ms: u64,
    pub tool_trace: Vec<ToolExecRecord>,
    pub llm_task_ids: Vec<String>,
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
│ Outcome               │ Agent scheduler 例             │ Workflow engine 例
├───────────────────────┼───────────────────────────────┼────────────────────────────
│ Done                  │ 写 step 记录，按状态机          │ 写入 node output，
│                       │ 决定下一步                    │ 进入下一个 node
│ WaitInput             │ session 进入"等用户输入"      │ workflow 挂起，
│                       │                               │ 通知人工节点
│ PendingTool           │ session 进入"等事件"          │ workflow 挂起，
│                       │                               │ 把 pending 排到任务队列
│ ContextLimitReached   │ 调度器自己的长期记忆压缩      │ 一般直接 fail / 走分支；
│                       │ 策略，重写 history 后 resume  │ 也可在 L4 声明压缩策略
│                       │                               │ 后 resume
│ BudgetExhausted       │ cost units 用尽 → 终止        │ 走 retry / escalation /
│                       │                               │ fail 分支
│ Error                 │ 走错误处理状态                │ 走 error handler 节点
└───────────────────────┴───────────────────────────────┴────────────────────────────
```

**关于 ContextLimitReached 的处理范式**：scheduler 在 resume 时必须通过 `ResumeFill::RewrittenHistory(messages)` 提供重写后的对话历史。这一步**会破坏原 LLMContext 的对话历史完整性**（这是 LLMContext 内部的事），但**不会破坏原始用户输入与可观测性事件流**（这两者由 owner / scheduler 持有）。waist 在压缩发生时会 emit 一条 `ContextRewritten` 事件供审计。

---

## 5. 外部依赖类型

LLMContext waist 自己不重新定义"LLM 边界类型"，而是直接消费下层 provider 抽象的语义类型。这些类型是 waist 与 provider adapter 共用的接口语言，**也是上层 scheduler 与 waist 交互时唯一会遇到的"非 waist 自有"类型**。

参考实现中，这些类型由 BuckyOS 的 `buckyos_api` crate 提供（即 aicc 路由层的边界类型）。同样接口的类型也可以由任何其它 provider 抽象提供，只要满足下表语义。

| 类型 | 字段（关键部分） | 在 waist 中的用途 |
|---|---|---|
| `AiMessage` | `role`（system/user/assistant/tool）, `content` | `LLMContextRequest.input` / `ResumeFill::HumanInput` / `ResumeFill::RewrittenHistory` / `Outcome::ContextLimitReached.accumulated` 的元素 |
| `AiToolCall` | `name`, `args: map`, `call_id` | provider adapter 把各家 native wire format 归一化后的 tool 调用；`PendingToolCall.call` 直接持有 |
| `AiResponseSummary` | `text`, `tool_calls`, `artifacts`, `usage`, `cost`, `finish_reason`, `provider_task_ref` | `Outcome::Done.response`：每次 LLM 推理的原始响应 |
| `AiUsage` | `input_tokens`, `output_tokens`, `total_tokens` | `Outcome::*.usage`：累计 token 用量 |
| `AiCost` | `amount`, `currency` | 出现在 `AiResponseSummary.cost`，waist 不单独暴露 |
| `AiArtifact` | `name`, `resource`, `mime`, `metadata` | 出现在 `AiResponseSummary.artifacts`，waist 不单独暴露 |

waist 不暴露 provider 内部的 request 类型（参考实现里叫 `AiMethodRequest`）：那是 provider adapter 与 LLM 服务通信的私事，应当被 `LLMContextDeps` 屏蔽掉。

**为什么直接复用而不是再包一层**：

- waist 与 provider 共用边界类型 ⇒ 零成本的序列化路径；
- 任何上层 scheduler 拿到 `Outcome::Done.response: AiResponseSummary` 时，**已经是 provider-agnostic 的归一化结构**，不需要再判断哪家厂商；
- 即便换一套 provider 实现，只要按上表语义提供同名/同形结构（或换名字 + 适配层），waist 本身完全不动。

---

## 6. 关键执行流程

错误处理在主循环的每个分支上都有触点（inference / output 解析 / policy gate / tool 调用），统一收敛到 §6.5 的错误处理流程。本节其它子节描述的"理想路径"不再重复列出错误分支，下游任何一处出错都按 §6.5 的统一逻辑处理。

### 6.1 一次同步执行（最常见）

```
LLMContext::new(req, deps)
  └─> run().await
        ├─> compile_prompt()              // input --OwnerContext--> final input
        ├─> emit(LLMStarted)
        ├─> loop:
        │     ├─> if context_yield_threshold reached:
        │     │     return Outcome::ContextLimitReached { ... }   // §6.4
        │     ├─> do_inference_once()
        │     │     ├─> on provider HardLimit / refusal:
        │     │     │     return Outcome::ContextLimitReached { ... }
        │     │     ├─> on provider error (after fault-tolerance layer gives up):
        │     │     │     handle_error(err)                       // §6.5
        │     │     └─> ok: continue
        │     ├─> if tool_calls.is_empty(): break
        │     ├─> on output parse / schema validation error:
        │     │     handle_error(err)                             // §6.5
        │     ├─> gate by policy
        │     │     └─> on policy reject:
        │     │           handle_error(err)                       // §6.5
        │     ├─> for call in calls:
        │     │     ├─> if call.tool ∈ deferred:
        │     │     │     return Outcome::PendingTool { ... }
        │     │     ├─> else: tools.call_tool() → observation
        │     │     │     └─> if Observation::Error:
        │     │     │           handle_error(err)                 // §6.5
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
- **input 已展开**：snapshot 里 `accumulated messages` 全部是具体 `Vec<AiMessage>`，跨节点 resume 不依赖任何模板环境。模板展开发生在 L4，与 waist 解耦（见 §3.9）。

> **工程提醒**：开工时如果你发现自己想往 snapshot 里塞"句柄"、"指针"、"长生命态引用"，停下来 —— 那些东西属于 `LLMContextDeps`（由 scheduler 重新提供），不属于 snapshot。

### 6.3 WaitInput

LLM 在 reply 中通过 `OutputSpec::Json` 里的 `request_human_input` 字段（具体 schema 由 L4 `LLM*Context` 决定）显式声明需要人工输入。`LLMContext` 检测到这个信号，构造 `Outcome::WaitInput`。`HumanPolicy.allow_request_input = false` 时降级为 `Outcome::Done`，让上层自己决定。

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

- **LLMContext 自己绝不调用压缩**。它只产生 `ContextLimitReached`，把 `accumulated` 完整暴露给 scheduler。任何压缩都发生在 LLMContext 之外，由 scheduler（例：Agent 类调度器调用自己的长期记忆 summarize；workflow 类调度器直接 fail-and-escalate）决定。
- **重写会破坏原 LLMContext 的对话历史，但不破坏可观测性事件流与原始用户输入**。原始输入由 owner 持有（例如 agent 的 session.history、workflow 的 node 输入），事件流是 append-only 的（包括 `ContextRewritten` 事件），都不受影响。这条让 ContextLimitReached 的处理保持可审计。
- **Resume 后 token usage 从重整后的 history 重新累计**。如果 scheduler 设了 `max_total_tokens` 红线，压缩后的新 history token 数会被算入累计，避免"无限压缩 + 无限运行"的恶性循环 —— 一旦累计撞红线，仍然走 `BudgetExhausted` 终止。
- **scheduler 也可以选择不 resume**。例如 workflow 收到 `ContextLimitReached` 后决定 fail 当前 node、走 retry 分支用更大窗口的模型重跑，这是合法的，因为 `ContextLimitReached` 是挂起态、不是强制 resume。

### 6.5 错误处理流程（handle_error）

§6.1 流程图里的所有 `handle_error(err)` 触点共用本节统一逻辑：先归一化错误为 `LLMComputeError`，再按 §3.11 `ErrorPolicy` 决定走"喂回 observation"、"挂起待修"还是"直接终态"。

```
handle_error(err):
    # 1. 分类（waist 默认分类见 §3.11 表；effect 实现层可在 LLMComputeError 内携带细化提示）
    class = classify(err)

    # 2. Fatal 直接终态，不可被 ErrorPolicy 改写
    if class is Fatal:
        emit(<对应失败事件>)               # ToolCallFailed / LLMInferenceFailed / ...
        return Outcome::Error { error: err, usage }

    # 3. Recoverable：按 mode 分流
    emit(<对应失败事件>)                   # 无论哪种 mode，事件流必须落

    match error_policy.mode:
      Suspend =>
        return Outcome::WaitInput {
            reason: structured_error_summary(class, err),    # 含 ErrorClass / tool name / call_id / provider task ref
            prompt_to_human: human_readable(err),
            snapshot,
            deadline_ms: human_policy.wait_timeout_ms,
        }

      FeedAsObservation =>
        # 把错误归一化为一条 role=tool 或 role=system 的 AiMessage，追加到 accumulated
        push_observation_message(err)
        consecutive_errors += 1
        if consecutive_errors > error_policy.max_consecutive_errors:
            # 安全网：避免"调错 → 看到 → 再调错"的死循环空烧
            return Outcome::WaitInput {
                reason: "too many consecutive errors",
                prompt_to_human: last_n_errors_summary(),
                snapshot,
                deadline_ms,
            }
        else:
            continue loop                                   # 喂给下一轮 LLM 自行处理
```

**与现有挂起态机制的关系**：

- Suspend 分支产出的 `WaitInput` 与 §6.3 共用挂起态机制；区别在 `reason` 字段结构化携带 `ErrorClass + 错误源`，便于上层批量诊断与自动化修复。
- Resume 时上层通过 `ResumeFill::HumanInput(AiMessage)` 喂回修复指令（典型：人工重写参数、人工授权、人工重试请求）。waist 不规定修复指令的 schema。
- FeedAsObservation 分支不挂起、不产生 snapshot，错误以 `AiMessage` 形态进入 accumulated，下一轮 LLM 看到的 input 与"工具正常返回"是同一种结构，让 LLM 能用同一种推理路径来"看到 + 反应"自己犯的错。

**Provider 容错的二段式**（与 §3.11 中"provider 容错与 waist 边界"一致）：

```
provider adapter 内部                       waist 主循环
┌───────────────────────────────┐         ┌───────────────────────────┐
│  retry / fallback chain       │ ──ok──> │  正常 inference 推进       │
│  - rate limit backoff         │         │                           │
│  - 5xx exponential retry      │ ──fail─>│  do_inference_once()      │
│  - 多 provider 路由 / 模型降级 │         │     surface error         │
└───────────────────────────────┘         │     handle_error(err)     │
                                          │     (Recoverable / Fatal) │
                                          └───────────────────────────┘
```

provider 一过性故障在第一段就被吸收，根本不进入 waist；只有"多次重试 + 多路由 + 多模型降级都救不回"的错误才上抛。这段是 effect 实现层的内部决策，**waist 不暴露 retry 计数 / 退避参数 / 路由策略**（详见 §A.2 与 §A.4 新增条目）。

**显式不属于 waist 的细节**（一并补到 §A）：

- 错误归一化的具体 wire format（错误 message 的字段结构、是否带 stack trace、是否带 hint）—— effect 层决定。
- Retry / 退避 / jitter / 熔断 / 路由的具体算法 —— provider adapter 与 ToolManager 内部决定。
- 如何把 LLM 的错误响应"翻译"成更友好的人类提示 —— scheduler 在 Suspend 模式下处理 `WaitInput.prompt_to_human` 的事。

---
## 8. 模块划分与文件落点

```
src/frame/llm_context/src/
│   ├── lib.rs              // pub use
│   ├── request.rs          // LLMContextRequest / ModelPolicy / ToolPolicy / OutputSpec /
│   │                       //   BudgetSpec / HumanPolicy / ContextOwnerRef
│   ├── outcome.rs          // LLMContextOutcome / ContextOutput / ContextRunTrace /
│   │                       //   PendingToolCall / ResumeFill
│   ├── observation.rs      // Observation / ToolExecRecord（与 AiToolCall 配对的 effect 侧产物）
│   ├── deps.rs             // LLMContextDeps：llm provider / tools / policy / worklog / tokenizer / task scheduler
│   ├── state.rs            // LLMContextState / LLMContextSnapshot（可序列化的可变态）
│   ├── context_loop.rs     // LLMContext::{new, run, resume, snapshot}，实现核心 Loop
│   ├── error.rs            // LLMComputeError
│   └── tests.rs

```

---

## 9. 姊妹文档

LLMContext 自己只是 waist，要真正给上层 scheduler 用，还需要 L4 `LLM*Context` 把"DSL 用户面对什么"那一层定义出来。本文档预期与下列姊妹文档配套（详细 lowering 协议见各自文档）：

| 文档 | 角色 |
|---|---|
| `LLMAgentContext 设计.md`（待写） | L4 scheduler-facing 层，Agent 一侧。承接所有角色定义 / 行为配置可见的字段，lowering 到本文档定义的 LLMContext。|
| `LLMWorkflowContext 设计.md`（待写） | L4 scheduler-facing 层，Workflow 一侧。承接所有 workflow DSL 可见的字段（service endpoint / 上下游引用 / on_* 分支等），lowering 到本文档定义的 LLMContext。|
| `LLMOneShotContext 设计.md`（待写，可选） | L4 scheduler-facing 层，一次性 CLI / 脚本入口。|

---

## 10. 一句话总结

> **LLMContext 是 LLM 执行的"进程上下文"：一次有界、可 cooperative yield、可 resume、可计费、可审计的执行体。它从 scheduler 状态机里独立出来，填补 `llm.complete`（太低阶）与 `agent.sendMsg`（太重型）之间的空缺，让 Agent / Workflow / Shell / Hook / Eval 等各自的调度器共用同一套进程语义。**

---

## Appendix A: Non-Goals（永久边界）

下面这些 **不只是本期不做，而是永远不做** —— 因为它们会破坏 narrow waist 的中立性（见 Preamble）。任何要把它们塞进 LLMContext 的提议，都应该被退回到上面（scheduler 层）或下面（provider / effect 实现层）。

本清单是**活的，只增不减**：每次 PR review 拒绝一个"看似合理但会污染 waist"的提议，决议结果就补进这里，让后人不必重新讨论同一个问题。

### A.1 Scheduler-specific（永远不进 waist）

- `next_behavior` / 行为切换字段 —— Agent 状态机专属，由 `LLMAgentContext` 在 `OutputSpec::Json { schema = ... }` 上面自己 deserialize，不进 LLMContext 通用接口
- workflow node 的 retry / fallback 策略 —— 上层 workflow engine 处理
- hook trigger 的事件元数据（trigger source / debounce / coalescing）—— 上层 hook scheduler 处理
- chat session 的 typing indicator / streaming UI 语义 —— 上层 shell / chat scheduler 处理
- multi-agent 的 turn-taking 协议 —— 上层 multi-agent scheduler 处理
- sub-agent 派生的层级关系字段（`parent_id` / `child_ids`）—— 上层调度器在 owner 维度记账，LLMContext 之间无父子关系
- 优先级 / 抢占 / 公平性策略 —— scheduler 政策，不是进程属性
- **scheduler-facing 的语义字段**（**新增**）—— 任何"DSL 用户可见 / 配置文件可写"的字段都不进 waist，必须落在对应 scheduler 的 L4 `LLM*Context` 里。例：service_endpoint 引用、`${prev_node.output.x}` 上游引用、on_budget_exhausted 分支策略、角色描述 / role 文件、行为状态机配置、hook trigger debounce、eval ground truth、容器与 session 句柄。判定方法：如果一个字段**在 DSL/配置文件里被人直接写出来**，它一定属于 L4，不属于 waist。
- **运行期动态修改 tool list**（**新增**）—— 一旦 `LLMContext::new` 完成，工具集合在该实例生命周期内不变。"中途加工具"是 scheduler 的诉求：销毁当前 LLMContext，构造一个新的，等价于换工具集；waist 不提供运行期修改 tool list 的接口。理由：动态修改会破坏 snapshot 可重放性、破坏 worklog audit、破坏 cooperative yield 的语义不变量。

### A.2 Provider-specific（永远不进 waist）

- 模型计费 / billing 字段 —— provider 自己 telemetry，与 waist 解耦
- provider 专属参数（anthropic `cache_control`、openai `seed`、gemini `safety_settings`）—— 通过 `model_policy.provider_options: opaque` **透传**，waist 不解释、不校验
- 模型能力探测（context window 大小 / 是否支持 vision / tool）—— provider adapter 内部决定，waist 不暴露
- token 计费方式（input vs output 不同价、cached vs uncached 不同价）—— waist 暴露的是 `AiUsage`（只含 input/output/total tokens）+ `AiResponseSummary.cost`（amount/currency），不解释 provider 内部的分价规则
- streaming 协议细节（SSE / chunked / batch）—— provider 适配层处理；waist 一次推理对外是原子的
- **function call 作为 loop 强制协议**（**新增**）—— 拒绝。Function call 是 provider-specific 的 wire format（OpenAI tool_calls / Anthropic tool_use block / Gemini function_call 各家细节都不同；本地模型经常根本没有原生支持）。通过 ToolManager 与 provider adapter 内部归一化处理，**不进 waist**。waist 的 loop 不变量是 §0.2 定义的 intent / effect / observation 三元组，effect 的承载形态由 `OutputSpec` 声明（structured output 里的 actions 数组 / provider-native tool_calls 皆可），不强制任何一种 wire 编码。详见 §0.2 与 §3.5 末段。
- **Provider 层的 retry / 退避 / jitter / 熔断 / 路由策略**（**新增**）—— 拒绝。这些都是 provider adapter 内部容错层的决策（参考实现：BuckyOS aicc 的多 provider 路由 + 模型降级）。waist 看到的是容错层兜底失败之后的最终错误，自己不在外面再做一层 retry。详见 §3.11 与 §6.5。

### A.3 Container / 长生命态（属于长生命会话，不属于 LLMContext）

- session memory / 长期记忆 —— 由 L4 在 lowering 阶段展开成 `AiMessage` 注入 `input`，LLMContext 自己不持有任何长期记忆接口
- workspace 路径 / 文件挂载 —— 容器关心，进程不关心；通过工具调用访问
- agent 身份 / 密钥 / 签名材料 —— 容器属性
- 持久事件流的存储位置 / 索引策略 —— 上层可观测性服务关心，LLMContext 只负责 emit 事件
- sub-agent / sub-context 注册表 / 生命周期 —— 容器编排关心
- 跨 LLMContext 的对话历史拼接 —— 容器在外面拼，传进来时已经是 `Vec<AiMessage>`
- **执行环境绑定**（机器 / 容器 / 远程 session 句柄）（**新增**）—— 属于上层调度器的容器编排或任务调度结果，LLMContext 通过 ToolManager 间接访问，不持有任何句柄。
- **"特定 tool 直接抬到 waist"**（**新增**）—— 拒绝。LLMContext 没有任何具名 tool 概念（包括 bash / browser / fs 这种"明显通用"的工具）；每一种 tool 都是 ToolManager 内部的一种实现，具体跑在哪里是 effect 层的事，对 waist 不可见。

### A.4 Effect-side 持久化与执行策略（属于 EffectDeps 实现，不属于 waist）

- snapshot 的存储介质 / 加密 / 跨节点复制策略 —— `SnapshotStore` 接口的实现细节
- 可观测性事件 sink 的具体实现（SQLite / 远程 / Kafka）—— `LLMContextDeps.worklog` trait 的实现细节，waist 只发事件不规定持久化
- tool 调用的审计 / 录像 / replay —— `ToolManager` 实现的可选行为
- pending tool 的任务队列后端（in-memory / Redis / 分布式 task service）—— scheduler 决定
- tool 调用的并发 / 限流 / 熔断 —— `ToolManager` 实现，waist 只声明 `parallel: bool` 这种意图
- **RPC 服务接口的 tool 化策略**（**新增**）—— 是 ToolManager 把后端服务暴露给 LLM 的内部决策，与 waist 解耦；具体把哪些远程接口包成什么样的 tool schema，由 effect 实现层决定。
- **系统状态路径化 / read_file 抽象**（**新增**）—— 同上，是工具实现的对外协议，不是 waist 字段。
- **LLMContext 的承载方式**（in-process lib / thunk / 跨设备 RPC）（**新增**）—— 是 scheduler 根据部署语义的选择，不是 waist 的属性。waist 不规定也不偏向任何承载方式，三种共享 100% 的执行语义（见 §2.3）。
- **上下文压缩策略**（summarize prompt / sliding window / hierarchical recall / drop-oldest / 换模型重灌）（**新增**）—— 拒绝进 waist。waist 只暴露 `Outcome::ContextLimitReached` 这个**事实信号**（见 §3.10 / §4 / §6.4），具体压缩算法属于 scheduler 在 resume 时通过 `ResumeFill::RewrittenHistory(...)` 提供的策略。典型选择：Agent 类调度器走自己的长期记忆 summarize-and-replace；workflow 类调度器一般直接 fail-and-escalate；Eval scheduler 可能选择 hard-truncate 观察模型在压力下的行为。任何"在 waist 里规定如何压缩"的字段都会破坏 scheduler 中立性，应当退回到对应的 L4 `LLM*Context`。
- **错误归一化的 wire format**（**新增**）—— 错误 message 的具体字段结构、是否带 stack trace、是否带 hint、人类可读 prompt 的措辞，都是 effect 实现层（ToolManager / provider adapter）与对应 L4 `LLM*Context` 的协议。waist 只规定"Recoverable 错误若走 FeedAsObservation 必须以合法 AiMessage 形态喂回，且 role ∈ {tool, system}"，不规定 message 内容形态。详见 §3.11 / §6.5。
- **ToolManager / 工具实现的 retry**（**新增**）—— 单个 tool 内部的重试（例：HTTP 请求 5xx 重试、tmux 命令重发）属于 ToolManager 实现的私事。waist 把每次 `call_tool` 的最终结果当作一次 `Observation`，不感知内部是否重试过几次。

### A.5 不解决的更大问题（本设计不替代，由其他文档负责）

- 长生命会话 / 容器编排 —— 由各 scheduler 的运行时文档负责
- Workflow DSL 与 DAG runtime —— 由 workflow engine 自己的文档负责
- LLM provider 的统一封装 —— 由下层 provider 抽象（参考实现见 Appendix B）负责
- 长期记忆的存储与压缩算法 —— 由各 scheduler 的 memory 设计负责
- Prompt / 模板编译的具体策略 —— 由各 L4 `LLM*Context` 的 prompt 编译器负责
- Agent 的角色 / 行为 / 状态机定义 —— 由各 Agent 框架自己的运行时文档负责

### 如何使用此清单

每次有人提议向 LLMContext 添加新字段或新方法，按以下顺序检查：

1. **先到 Appendix A 查重**：是不是已经被显式列为 Non-Goal？是的话直接拒绝并指向已有条目。
2. **过双中立性测试**（见 Preamble）：scheduler 中立？provider 中立？任何一项不通过即拒绝。
3. **过完两个测试且不在 Non-Goals 里**，仍要在 PR 描述里说明 *"为什么必须进 waist 而不是上下游某层"*。说不清楚就退回让提议人想清楚。
4. **被拒绝的提议，补进 Appendix A 对应小节**，标注 PR 链接和拒绝理由。这样下次有人提同一件事时，可以一句话回掉，不必重新论证。
5. **同意进入 waist 的字段，必须同步更新 §3 / §4 / §5 / §10 实施路线**，并在 changelog 里登记 waist 版本。waist 字段一旦进入，移除等同于 breaking change，必须走 deprecation 流程。

> 这套流程的目的不是为了"难"，而是为了让"瘦"成为默认状态。任何瘦腰原语的失败模式都不是被一次大改打破的，而是被一百个"加一个小字段没关系吧"的小改慢慢撑胖的。Appendix A 就是用来记住每一次"小字段没关系吧"被拒绝的理由，避免同一个争论开 N 次。

---

## Appendix B: 参考实现 —— OpenDAN / BuckyOS 适配（informative）

> 本附录是**资料性**（informative）的，主体设计不依赖本附录内容。LLMContext 主文档的目标读者**不需要**了解 OpenDAN / BuckyOS 也能完整使用本规范。本附录只是说明：在最初催生这套设计的工程语境（BuckyOS 的 OpenDAN agent 栈）里，waist 的各类型 / trait 落到具体什么实现上。其它工程语境（独立的 agent 框架、其它 workflow runtime）实现 waist 时，可以把本附录当作一份参考样本。

### B.1 边界类型来源

主体文档 §5 列出的"外部依赖类型"（`AiMessage / AiToolCall / AiResponseSummary / AiUsage / AiCost / AiArtifact`）在参考实现里由 BuckyOS 的 `buckyos_api` crate 提供：

| waist 引用名 | 参考实现路径 |
|---|---|
| `AiMessage` | `buckyos_api::AiMessage`（`src/kernel/buckyos-api/src/aicc_client.rs`） |
| `AiToolCall` | `buckyos_api::AiToolCall` |
| `AiResponseSummary` | `buckyos_api::AiResponseSummary` |
| `AiUsage / AiCost / AiArtifact` | `buckyos_api::{AiUsage, AiCost, AiArtifact}` |
| Provider 内部 request 类型（不进 waist） | `buckyos_api::AiMethodRequest`，由 aicc 路由层使用 |

aicc 是 BuckyOS 提供的统一 LLM 调用 / 路由层（`src/frame/aicc`）。waist 不感知 aicc 的存在，但参考实现的 `LLMContextDeps` 拿到的 LLM client 通常是 `AiccClient`，inference 时把 `LLMContextRequest` 转译成 `AiMethodRequest`、把响应解析回 `AiResponseSummary`。

### B.2 LLMContext 的来源：OpenDAN behavior 拆解

LLMContext 的设计起点是把 OpenDAN 既有 agent 主循环里的"一次智能执行"拆出来。被替换的 OpenDAN 模块（`src/frame/opendan`）：

| OpenDAN 概念 | 在 LLMContext 设计里的归属 |
|---|---|
| `LLMBehavior::run_step(input)`（`behavior/behavior.rs:70`） | 被 `LLMContext::run` 取代；老类型在迁移 PR 中删除或私有化 |
| `LLMBehaviorDeps`（`behavior/behavior.rs:28`） | 被 `LLMContextDeps` 取代 |
| `BehaviorExecInput`（`behavior/types.rs:52`） | 被 `LLMContextRequest` 取代；`session_id / trace` → `ContextOwnerRef + trace`；`role_md / self_md / behavior_prompt / input_prompt` 等模板字段在 L4 的 prompt compiler 里展开后灌入 `LLMContextRequest.input` |
| `BehaviorLLMResult`（`behavior/types.rs:132`，含 `reply / actions / next_behavior / set_memory / new_work_session / shell_commands`） | **不进 waist**。这是 Jarvis 风格行为机的产物，应当落在 L4 `LLMAgentContext` —— 由它在 lowering 时声明 `OutputSpec::Json { schema = BehaviorSchema }`，再在 `Outcome::Done.output` 上自行 deserialize |
| `TokenUsage`（opendan 自定义，`behavior/types.rs:100`） | 删除，统一用 `AiUsage` |
| `LLMTrackingInfo`（`behavior/types.rs:676`） | 拆分：provider 侧 → `AiResponseSummary`；waist 侧（trace_id / latency / tool_trace / llm_task_ids）→ `ContextRunTrace` |
| `LLMComputeError`（`behavior/behavior.rs:540`） | 语义保留，但移到 `llm_context` crate 内重新定义，不再 re-export 老路径 |
| `AgentSession` | 仍是 Agent 一侧的长上下文持有者；它实现 L4 `LLMAgentContext` 内部的 `ContextSources` trait，**不出现在** `LLMContextRequest` 公共字段里 |
| `AgentToolManager` | 实现 waist 的 `ToolManager` trait；`call_tool` 返回值切到 `Observation::{Success｜Error｜Pending}` |
| `WorklogSink` / `AgentWorkEvent` | `LLMContextDeps.worklog` 持有的 trait 实例；事件 schema 在 OpenDAN 里仍是 `AgentWorkEvent`，waist 不规定 schema |
| `PolicyEngine` | `LLMContextDeps.policy` 持有的 trait 实例，职责不变 |
| `PromptBuilder` | 上提到 L4：`LLMAgentContext` 的 prompt compiler 接收 `Arc<dyn ContextSources>`，把模板展开成 `Vec<AiMessage>` 后填入 `LLMContextRequest.input` |

**核心定位**：在 OpenDAN 语境下，LLMContext 是对 `LLMBehavior::run_step_inner`（`behavior/behavior.rs:129-198`）那段循环的**重新切片与重新封装**，加上 owner 抽象、cooperative yield（pending tool / context limit / wait input）、显式 budget / output spec。它**不是** `LLMBehavior` 的 thin wrapper，迁移走的是 clean break。

### B.3 相关 OpenDAN 设计文档

| 文档 | 与 LLMContext 的关系 |
|---|---|
| `OpenDAN Agent Runtime 设计.md` | LLMContext 是该文档"Behavior Loop"小节的下沉抽象；原文档预期新增"L2 / L3 / L4 分层"段落引用本文档与 `LLMAgentContext 设计.md` |
| `Agent Session.md` | AgentSession 是 L4 `LLMAgentContext` 的 `ContextSources` 实现，通过 `LLMAgentContext` 间接驱动 LLMContext |
| `Agent Prompt Compiler.md` | OpenDAN 的 PromptBuilder 是 `LLMAgentContext` 的成员，把模板展开为 `Vec<AiMessage>` 后灌入 waist |
| `Agent Worklog.md` | waist 的 `LLMContextDeps.worklog` 在 OpenDAN 实例化为 `WorklogSink<AgentWorkEvent>` |
| `OpenDAN Long Task & Sub-Agent.md` | `Outcome::PendingTool` 是 long task 在 LLMContext 层的统一表达；sub-agent 创建走"同步创建 + 异步执行"约定 |
| `opendan关键类型.md` | 预期新增 LLMContext 相关章节，并标注 waist 与 aicc 边界类型的依赖关系 |