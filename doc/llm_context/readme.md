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

设计上，按新的 system prompt、工具和初始输入创建子 context，并在结束后向 parent 交接结果；当前 libopendan 尚未提供这种独立子 context 的完整调用/返回 helper，llm_context 的 `build_fresh` / `rebuild_with_inherit` 只提供底层构造能力。

### ContextSwitch

若 target 使用 `independent` 模式，current run 入栈挂起，target 有挂起的 run 就恢复，否则新建，并通过 `on_behavior_switch` 输入继续推进。
相当于 AgentSession 内多个 context 各自保存 steps/snapshot，轮流运行；首次新建仍会装配 Session 共享 worklog 的历史，并非完全隔离。

默认的普通 behavior switch 则继续使用同一个 context、同一个 run 和历史，只更新 behavior 并注入 `on_behavior_switch` 输入，不会自动创建独立 history。


## AgentSession Tree

上面基于 LLMContext 的切换都是串行的：同一 AgentSession 同时只推进一个顶层 context。
设计上，Session 级并行可以使用 Sub AgentSession，由各自的 Runner 推进；当前已有 `create_session` + `origin.parent_session` 的创建/父子登记能力，但启动子 Runner、等待及结果回传尚需调用方组合，没有完整的 Sub AgentSession helper。
    parent AgentSession 可以创建全新的 sub session，也可以向已有且未 finished 的 session 投递输入（需要知道 session_id，推进还须满足 driver 身份和锁约束）。
    构造 input，推进到所需的结果/等待边界。
    读取结果或订阅状态变化；指定 `origin.parent_session` 创建子 Session 时，当前实现会尝试为 parent 添加对子 Session 的半订阅。

Sub AgentSession 按访问规则通过 AgentStateClient/工具读取 Parent AgentSession，按自己的配置和 Runner 依赖绑定 Runtime；父子关系不自动继承 Runtime、工具或 parent 的完整上下文。

## Agent Loop的几种典型伪代码和history 消息结构

伪代码只保留主干，省略错误、预算、快照提交和崩溃恢复。`组件.函数` 对应实现：AgentSession 是 libopendan 的 `SessionRunner`（`runner/drive.rs`），LLMContext 在 `llm_context/context_loop.rs`，其余是 LLMContext 的依赖（`deps.rs`）。消息结构里 `@` 标出消息所属的 Round / Step。

```text
AgentSession                  一个 Turn：取输入、推进 run、解释 Outcome
└─ LLMContext.run             一次 run：Round / Step 循环，返回 Outcome
   ├─ LlmClient.infer         一个 Round
   ├─ ToolManager.call_tool   原生工具 / behavior action
   └─ CheckpointHook          推理或 Step 边界：存快照、注入半订阅变化
```

### AgentSession 外层

```text
AgentSession.drive():
    loop:
        inputs = AgentSession.fetch_inputs()                  # msg / event 驱动推进；change 只随行
        text   = SessionAssembler.render_turn(inputs, hook)   # hook = on_init | on_wakeup | on_behavior_switch
        if 没有 text 且没有可恢复的 run: 等待输入; continue
        ctx  = live run ?? AgentSession.new_run_context()     # input = [system, <session_history>]
        AgentSession.begin_round(ctx, text)                   # LLMContext.inject(user:<turn>)，提交 receipt
        outcome = AgentSession.run_compacting(ctx)            # LLMContext.run()
        next = AgentSession.commit_round(ctx, outcome)
        match next:
            END / done     -> AgentSession.finish_run()       # run 历史 flush 进 worklog；按 end_condition 结束 Session 或等待
            WAIT_USER_MSG  -> AgentSession.finish_run()       # 等下一条输入
            switch(B)      -> 见“behavior 切换”，Turn 继续
            挂起            -> run 保留，恢复后继续同一 run      # PendingTool / ContextLimitReached / Interrupted
```

run 结束后，它的消息只留在 Session worklog 中。下一个 Turn 新建 run，之前的 Turn 以 `<session_history>`（摘要 + 最近记录）进入 input，而不是原样的消息列表。

### 标准 function_call Loop

```text
LLMContext.run_inner():
    loop:
        CheckpointHook.before_inference(snapshot)        # 每个 Round 之前
        resp = LlmClient.infer(accumulated)              # 一个 Round
        if resp 没有 tool_calls:
            return Done(resp)                            # 最后一个 Round，assistant 文本就是结果
        accumulated += resp.message                      # assistant: tool_calls
        for call in resp.tool_calls:                     # 一个工具批次，串行派发
            accumulated += ToolManager.call_tool(call)   # tool: result
```

```text
[system] [user:<session_history>] [user:<turn hook=on_wakeup> 输入]
[assistant:tool_calls] [tool] [tool]    @Round1
[assistant:tool_calls] [tool]           @Round2
[assistant:回答]                         @Round3 → Done
```

三个 Round，没有 Step，一个 Turn。工具结果驱动下一个 Round；run 进行中只有半订阅变化可能在 Round 边界注入，新的 msg/event 等 run 返回后再进入。

### Behavior Loop

```text
LLMContext.run_behavior():
    loop:
        CheckpointHook.before_inference(外层 snapshot)   # Step 边界（观察阶段）
        resp = LLMContext.run_inner_for_step()          # 内层就是上面的 run_inner：可含多个 Round 和原生工具
        step = LLMResultParser.parse(resp)              # 解析失败 → 合成错误 Step 喂回，重来
        for action in step.actions:                     # 按序派发，遇到业务错误即停
            step.action_results += ToolManager.call_tool(action)
        if step.next_behavior 或 这一步什么都没做:
            return Done(behavior_result)                # END / WAIT_USER_MSG / 跳转目标
        LLMContext.sediment(step)                       # 成为热 step，下一次推理时看到它的结果
```

内层每次推理的消息由 `LLMContext.build_inner_request` 现场拼出：

```text
[system] [user:<session_history>] [user:<turn hook=on_init>]                      ← request.input
[user:<<step_history>> 摘要 / 其它 behavior 的 step / 历史输入 <</step_history>>]
[assistant:决策] [user:<<last_step_action_results>>]                               @Step3
[assistant:决策] [user:<<last_step_action_results>> + 观察阶段注入]                  @Step4（热 step）
[assistant:tool_calls] [tool]                                                     @Step5 的 Round1
```

Step5 的下一个 Round 输出决策后，Step5 才完成并沉淀为 `StepRecord`；只保留决策和 action 结果，Step 内原生工具的消息不再出现在之后的 prompt 中。

### behavior 切换

```text
# 普通切换：同一 context、同一 run
AgentSession.commit_round(Done{next_behavior: B}):
    ctx = LLMContext.resume(snapshot{behavior: B}, ResumeFromMidRun)
    # 下一圈 begin_round 注入 <turn hook=on_behavior_switch><behavior_switch to=B/>
    # A 的 step 降为 <<step_history>> 里的继承记录，B 的 step 完整呈现

# Fork：B 被配置为 fork
AgentSession.suspend_run(run_A, Fork)          # run_A 入 process_stack，Paused
AgentSession.new_run_context()                 # run_B：复制 run_A 的 steps 作为继承历史
... run_B 推进到 Done                           # 任何 next_behavior 都视为返回调用方
AgentSession.finish_run(run_B)                 # 出栈，run_A 重新成为 live run，state.process_result = 结果
# 下一圈 begin_round 把 <process_result> 注入 run_A

# Independent：B 被配置为 independent
AgentSession.suspend_run(run_A, Independent)   # run_A 入栈
run_B = 栈里 B 的挂起 run ?? AgentSession.new_run_context()
```

Fork 前后的消息：

```text
run_A: [system] [<session_history>] [<turn>] … [assistant:A 决策 next_behavior=B]                → Paused
run_B: [system] [<session_history>] [<turn on_behavior_switch>] [<<step_history>> A 的 step] … B 的 steps → Done
run_A: … [assistant:A 决策] [user:<turn on_behavior_switch><process_result behavior=B>…]          → 继续
```

run_B 的 steps 不并入 run_A 的 history；Session worklog 仍记录 run_B 的过程。

### 挂起与恢复

```text
PendingTool          -> 等工具结果，LLMContext.resume(snapshot, ToolResults)，从未完成的批次 / action 继续
ContextLimitReached  -> AgentSession.rewrite_for_limit：flush 到 worklog → 压缩 summary → 以新的 [system, <session_history>]
                        LLMContext.resume(snapshot, RewrittenHistory | RewrittenSteps)
Interrupted          -> 用推理前的快照 LLMContext.resume(snapshot, ResumeFromMidRun)，重做这次推理
```

当前三个宿主都没有开启 deferred 工具，PendingTool 的结果回填还没接入。