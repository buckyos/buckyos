# 快速了解 OpenDAN 基于 LLM Context的Agent Loop

## 名词解释

Round：LLM 层的一次推理，即宿主一次 `LlmClient::infer` 调用；最后一次无工具的回答也是 Round。
Step：(Behavior Loop) 内层传统 LLM Loop 产出一次行为决策，并把决策及动作结果记为一个 `StepRecord`；内部可以有多个推理 Round，一个 Step 可以有多个 action，一个外层 LLMContext 可以执行多个 Step。
Turn：AgentSession 完成一次逻辑上的输入 (Input) 到输出 (result)，期间可以包含多个 Step、behavior/context 切换和多个 run。
    - 使用标准 function_call Loop 时，一次正常完成的 LLMContext run 通常对应一个 Turn，最后的 assistant message 是结果；不使用 Behavior Loop 的 Step 概念。

LLMContext run 的返回包括完成 (`Done`，Behavior 模式通过 `next_behavior` 表达结束/切换/等待输入)、错误 (`Error`)、预算耗尽 (`BudgetExhausted`)，以及可恢复的挂起 (`PendingTool`、`ContextLimitReached`、`Interrupted`)；挂起不等于 Step 或 Turn 已完成。一次 `run()` 返回一个 Outcome，不能用 run 数、调用次数或 Outcome 数反推 Round 或 Turn 数；Turn 是否完成只由 AgentSession 解释 Outcome 和交接状态决定。

工具额度 `ToolPolicy.max_tool_iterations` 统计的是工具迭代：完成一个原生工具批次，或派发一个带 action 的 Behavior Step，各计一次；两者共享额度。它不是 Round 数，最后无工具的回答、纯 report 的 Step 和解析纠错都不扣额度。`max_calls_per_round` 限制的才是单次模型 response（一个 Round）里的原生 tool call 数。

### 当前实现中的身份与计数

| 单位 | 身份 | 记录位置（libopendan） |
|---|---|---|
| Round | 宿主的一次 `infer` 尝试 | `static.json` 的 `rounds` / `rounds_failed` / `rounds_interrupted`；run.json `usage.llm_requests` 累加各执行器的调用 |
| Step | `(run_id, step_index)` | worklog `step`（含 `step_index`，解析/策略纠错的合成 Step 带 `correction`）；function_call 的 assistant response 记为 `assistant_message`，不是 Step |
| Turn | Session 内递增的 `turn` | `state.json` 的 `turn_seq` / `open_turn` / `turns_completed`；worklog `turn_started` … `turn_ended`；`static.json` 的 `turns` |
| 输入批次 | `(run_id, input_seq)` | snapshot 里的 receipt；worklog `turn_started`（开启 Turn）或 `input_batch`（并入当前 Turn） |
| 工具调用 | `call_id` | worklog `action_result`；工具运行上下文的 `tool_call_index` 只是调用序号，不是 Step 或 Turn |

Turn 的规则：没有打开的 Turn 时提交的输入批次开启新 Turn（bootstrap、msg/event）；普通切换、fork 调用与返回、independent 切换、观察阶段注入、可恢复挂起（中断、可重试错误、context limit、PendingTool）、history epoch 重写和重启都继续当前 Turn，期间消费的新输入并入当前 Turn。只有 AgentSession 在 run 结束时关闭 Turn：交付结果为 `completed`；`WAIT_USER_MSG` 只有已经交付答复（`<report>` 或 `<sendmsg>`）才算 `completed`，否则 Turn 保持打开；不可恢复错误、预算耗尽、stop 分别记为 `failed`、`budget_exhausted`、`stopped`。fork 子 run 结束不关闭 Turn。Session 结束条件 `max_turns` 只统计 `completed` 的 Turn。


## 在AgentSession中的LLM Context 的状态机切换

AgentSession 构造/管理 多个LLM Context，同一时刻只有一个当前 LLMContext/run 被推进。
AgentSession 通过 prompt/input 和工具向 LLMContext 提供所需状态，LLMContext 不直接持有或自动看到 Session 的全部状态。


### Fork

基于 parent LLMContext 的历史快照创建子 context，并提供子任务的 user message/工具；parent 挂起，子 context 结束后恢复 parent，结果通过 `process_result` 进入 parent 的交接输入批次（仍属同一 Turn），子 context 的完整 steps 不合并到 parent 的主干 history。

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
AgentSession                  Turn：取输入批次、推进 run、解释 Outcome、决定 Turn 是否结束
└─ LLMContext.run             一次 run：Round / Step 循环，返回 Outcome
   ├─ LlmClient.infer         一个 Round
   ├─ ToolManager.call_tool   原生工具 / behavior action
   ├─ CheckpointHook          function_call：每个 Round 前；behavior：Step 边界。存外层快照、注入半订阅变化
   └─ InferenceHook           每个 Round 前的同步快照（behavior 模式在内层），不是 Turn 钩子
```

### AgentSession 外层

```text
AgentSession.drive():
    loop:
        inputs = AgentSession.fetch_inputs()                  # msg / event 形成输入批次；change 只随行
        text   = SessionAssembler.render_input(inputs, hook)  # hook = on_init | on_wakeup | on_behavior_switch
        if 没有 text 且没有可恢复的 run: 等待输入; continue
        ctx  = live run ?? AgentSession.new_run_context()     # input = [system, <session_history>]
        AgentSession.commit_input_batch(ctx, text)            # LLMContext.inject(user:<session_input>)，提交 receipt；
                                                              # 没有打开的 Turn 就开新 Turn，否则并入当前 Turn
        outcome = AgentSession.run_compacting(ctx)            # LLMContext.run()，一个 Outcome
        next = AgentSession.handle_context_outcome(ctx, outcome)   # next.turn_end 决定 Turn 是否结束
        match next:
            END / done     -> AgentSession.finish_run()       # run 历史 flush 进 worklog；Turn completed；按 end_condition 结束 Session 或等待
            WAIT_USER_MSG  -> AgentSession.finish_run()       # 已交付答复则 Turn completed，否则 Turn 保持打开；等下一条输入
            switch(B)      -> 见“behavior 切换”，Turn 继续
            挂起 / 可重试错误 -> run 保留，Turn 保持打开，恢复后继续同一 run   # PendingTool / ContextLimitReached / Interrupted
            失败 / 预算耗尽 / stop -> AgentSession.finish_run()  # Turn 以 failed / budget_exhausted / stopped 结束
```

run 结束后，它的消息只留在 Session worklog 中。下一个 Turn 通常新建 run，之前的 Turn 以 `<session_history>`（摘要 + 最近记录）进入 input，而不是原样的消息列表。

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
[system] [user:<session_history>] [user:<session_input hook=on_wakeup> 输入]
[assistant:tool_calls] [tool] [tool]    @Round1
[assistant:tool_calls] [tool]           @Round2
[assistant:回答]                         @Round3 → Done
```

三个 Round，没有 Step，一个 Turn，扣两次工具迭代额度（每个工具批次一次）。worklog 里每个 assistant response 记为 `assistant_message`。工具结果驱动下一个 Round；run 进行中只有半订阅变化可能在 Round 边界注入，新的 msg/event 等 run 返回后再作为输入批次进入（Turn 仍打开时并入当前 Turn）。

### Behavior Loop

```text
LLMContext.run_behavior():
    loop:
        CheckpointHook.before_inference(外层 snapshot)   # Step 边界（观察阶段）
        resp = LLMContext.run_inner_for_step()          # 内层就是上面的 run_inner：可含多个 Round 和原生工具
        step = LLMResultParser.parse(resp)              # 解析失败 → 合成错误 Step 喂回，重来
        for action in step.actions:                     # 按序派发，遇到业务错误即停；带 action 的 Step 扣一次工具迭代额度
            step.action_results += ToolManager.call_tool(action)
        if step.next_behavior 或 这一步什么都没做:
            return Done(behavior_result)                # END / WAIT_USER_MSG / 跳转目标
        LLMContext.sediment(step)                       # 成为热 step，下一次推理时看到它的结果
```

内层每次推理的消息由 `LLMContext.build_inner_request` 现场拼出：

```text
[system] [user:<session_history>] [user:<session_input hook=on_init>]             ← request.input
[user:<<step_history>> 摘要 / 其它 behavior 的 step / 历史输入 <</step_history>>]
[assistant:决策] [user:<<last_step_action_results>>]                               @Step3
[assistant:决策] [user:<<last_step_action_results>> + 观察阶段注入]                  @Step4（热 step）
[assistant:tool_calls] [tool]                                                     @Step5 的 Round1
```

Step5 的下一个 Round 输出决策后，Step5 才完成并沉淀为 `StepRecord`；只保留决策和 action 结果，Step 内原生工具的消息（inner transcript）不再出现在之后的 prompt 中。Step5 未完成时挂起，inner transcript 留在外层快照里，恢复后继续这个 Step，不重放已执行的工具。

### behavior 切换

```text
# 普通切换：同一 context、同一 run、同一 Turn
AgentSession.handle_context_outcome(Done{next_behavior: B}):
    ctx = LLMContext.resume(snapshot{behavior: B}, ResumeFromMidRun)
    # 下一圈 commit_input_batch 注入 <session_input hook=on_behavior_switch><behavior_switch to=B/>（worklog: input_batch）
    # A 的 step 降为 <<step_history>> 里的继承记录，B 的 step 完整呈现

# Fork：B 被配置为 fork
AgentSession.suspend_run(run_A, Fork)          # run_A 入 process_stack，Paused
AgentSession.new_run_context()                 # run_B：复制 run_A 的 steps 作为继承历史
... run_B 推进到 Done                           # 任何 next_behavior 都视为返回调用方
AgentSession.finish_run(run_B)                 # 出栈，run_A 重新成为 live run，state.process_result = 结果；Turn 不结束
# 下一圈 commit_input_batch 把 <process_result> 注入 run_A

# Independent：B 被配置为 independent
AgentSession.suspend_run(run_A, Independent)   # run_A 入栈
run_B = 栈里 B 的挂起 run ?? AgentSession.new_run_context()
```

Fork 前后的消息：

```text
run_A: [system] [<session_history>] [<session_input>] … [assistant:A 决策 next_behavior=B]                → Paused
run_B: [system] [<session_history>] [<session_input on_behavior_switch>] [<<step_history>> A 的 step] … B 的 steps → Done
run_A: … [assistant:A 决策 next_behavior=B] [user:A 该 Step 的结果 + <session_input on_behavior_switch><process_result behavior=B>…]   → 继续
```

返回时的交接批次并入 run_A 最后一个 Step 的 user 消息（`InjectionPosition::Step`）；普通切换时最后一个 Step 属于另一个 behavior，交接批次追加到 `request.input`，渲染在 `<<step_history>>` 之前。run_B 的 steps 不并入 run_A 的 history；Session worklog 仍记录 run_B 的过程（Step 身份 `(run_B, step_index)`，继承的 step 不重复写入）。三个 run 段都属于同一个 Turn。

### 挂起与恢复

```text
PendingTool          -> 等工具结果，LLMContext.resume(snapshot, ToolResults)，从未完成的批次 / action 继续
ContextLimitReached  -> AgentSession.rewrite_for_limit：flush 到 worklog → 压缩 summary → 以新的 [system, <session_history>]
                        LLMContext.resume(snapshot, RewrittenHistory | RewrittenSteps)
Interrupted          -> 用推理前的快照 LLMContext.resume(snapshot, ResumeFromMidRun)，重做这次推理
```

这些挂起都不结束 Step 或 Turn；恢复时不重扣已扣的工具迭代额度。Interrupted 只有真实发起过的推理才计入 Round。

当前三个宿主都没有开启 deferred 工具，PendingTool 的结果回填还没接入。


## AgentRuntime 

简单的说，决定了tool的执行环境
- 在


## AgentState