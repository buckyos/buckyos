# 快速了解 OpenDAN 基于 LLM Context的Agent Loop

## 名词解释

Round：LLM 层的一次推理；注意 libopendan 的 `state.round` 是 Session 层一次输入/内部 handoff 驱动的推进，不是单次模型调用的计数。
Step：(Behavior Loop) 内层传统 LLM Loop 产出一次行为决策，并把决策及动作结果记为一个 `StepRecord`；内部可以有多个推理 Round，一个外层 LLMContext 可以执行多个 Step。
Turn：AgentSession 完成一次逻辑上的输入 (Input) 到输出 (result)，期间可以包含多个 Step 和 behavior/context 切换。
    - 使用标准 function_call Loop 时，一次正常完成的 LLMContext run 通常对应一个 Turn，最后的 assistant message 是结果；不使用 Behavior Loop 的 Step 概念。

LLMContext run 的返回包括完成 (`Done`，Behavior 模式通过 `next_behavior` 表达结束/切换/等待输入)、错误 (`Error`)、预算耗尽 (`BudgetExhausted`)，以及可恢复的挂起 (`PendingTool`、`ContextLimitReached`、`Interrupted`)；挂起不等于 Step 或 Turn 已完成。


## 在AgentSession中的LLM Context 的状态机切换

AgentSession 通过 prompt/input 和工具向 LLMContext 提供所需状态，LLMContext 不直接持有或自动看到 Session 的全部状态。
AgentSession 里，同一时刻只有一个顶层 LLMContext/run 被推进。

### Fork

基于 parent LLMContext 的历史快照创建子 context，并提供子任务的 user message/工具；parent 挂起，子 context 结束后恢复 parent，结果通过 `process_result` 进入 parent 的下一轮输入，子 context 的完整 steps 不合并到 parent 的主干 history。

缓存是否命中取决于模型、system prompt、工具定义及历史渲染是否保持相同前缀，Fork 本身不保证命中；它隔离的是推理历史，文件修改等工具副作用仍然存在，Session 的 worklog 也会记录子过程。

### CreateSubContext

概念上，按新的 system prompt、工具和初始输入创建子 context，并在结束后向 parent 交接结果；当前 libopendan 没有同名 `CreateSubContext` API，父 context 的保存/恢复及结果交接由 Session 调度层负责。

### ContextSwitch

若 target 使用 `independent` 模式，current run 入栈挂起，target 有挂起的 run 就恢复，否则新建，并通过 `on_behavior_switch` 输入继续推进。
相当于 AgentSession 内多个 context 各自保存 steps/snapshot，轮流运行；首次新建仍会装配 Session 共享 worklog 的历史，并非完全隔离。

默认的普通 behavior switch 则继续使用同一个 context、同一个 run 和历史，只更新 behavior 并注入 `on_behavior_switch` 输入，不会自动创建独立 history。


## AgentSession Tree

上面基于 LLMContext 的切换都是串行的：同一 AgentSession 同时只推进一个顶层 context。
Session 级并行可以使用 Sub AgentSession，由各自的 Runner 推进。
    parent AgentSession 可以创建全新的 sub session，也可以向已有且未 finished 的 session 投递输入（需要知道 session_id，推进还须满足 driver 身份和锁约束）。
    构造 input，推进到所需的结果/等待边界。
    读取结果或订阅状态变化；指定 `origin.parent_session` 创建子 Session 时，当前实现会尝试为 parent 添加对子 Session 的半订阅。

Sub AgentSession 按访问规则通过 AgentStateClient/工具读取 Parent AgentSession，按自己的配置和 Runner 依赖绑定 Runtime；父子关系不自动继承 Runtime、工具或 parent 的完整上下文。

## Agent Loop的几种典型伪代码和history 消息结构

<TODO>