# 快速了解 OpenDAN 基于 LLM Context的Agent Loop

## 名词解释

Round：LLM 层的一次推理，即宿主一次 `LlmClient::infer` 调用；最后一次无工具的回答也是 Round。
Step：(Behavior Loop) 内层传统 LLM Loop 产出一次行为决策，并把决策及动作结果记为一个 `StepRecord`；内部可以有多个推理 Round，一个 Step 可以有多个 action，一个外层 LLMContext 可以执行多个 Step。
Turn：AgentSession 完成一次逻辑上的输入 (Input) 到输出 (result)，期间可以包含多个 Step、behavior/context 切换和多个 run。
    - 使用标准 function_call Loop 时，一次正常完成的 LLMContext run 通常对应一个 Turn，最后的 assistant message 是结果；不使用 Behavior Loop 的 Step 概念。

LLMContext run 的返回包括完成 (`Done`，Behavior 模式的结束报告 `<report end="true">`、`next_behavior` 交接 / `WAIT_USER_MSG`、无动作的让出都以 `Done` 返回)、错误 (`Error`)、预算耗尽 (`BudgetExhausted`)，以及可恢复的挂起 (`ContextLimitReached`、`Interrupted`、`Settled`)；挂起不等于 Step 或 Turn 已完成。loop 里没有等待：工具交出去的任务、子 context、子 session 都在 loop 外由 AgentSession 跟进，结局作为新输入回来。一次 `run()` 返回一个 Outcome，不能用 run 数、调用次数或 Outcome 数反推 Round 或 Turn 数；Turn 是否完成只由 AgentSession 解释 Outcome 和交接状态决定。

工具额度 `ToolPolicy.max_tool_iterations` 统计的是工具迭代：完成一个原生工具批次，或派发一个带 action 的 Behavior Step，各计一次；两者共享额度。它不是 Round 数，最后无工具的回答、纯 report 的 Step 和解析纠错都不扣额度。`max_calls_per_round` 限制的才是单次模型 response（一个 Round）里的原生 tool call 数。

收尾模式（`ToolPolicy.wrap_up_tools` / `wrap_up_rounds`）：额度剩 0 次时请求里只给宿主指定的收尾工具（libopendan：explicit report 的 function call run 为 `report`，以及 run 以函数提供时的 `ask_user`；behavior `[budget] wrap_up_tools` 可改），而不是一个工具都没有。只含收尾工具的批次（或 Behavior Step 的 action）不扣额度，最多 `wrap_up_rounds`（缺省 2）批，已用批数记在快照 `wrap_up_rounds_used`，宿主补满额度时一并清零。批次里有别的工具、或批数用完后再要工具，照旧 `BudgetExhausted{ToolIterations}`；`wrap_up_tools` 为空（缺省）时与原来一样。

### 当前实现中的身份与计数

| 单位 | 身份 | 记录位置（libopendan） |
|---|---|---|
| Round | 宿主的一次 `infer` 尝试 | `static.json` 的 `rounds` / `rounds_failed` / `rounds_interrupted`；run.json `usage.llm_requests` 累加各执行器的调用 |
| Step | `(run_id, step_index)` | worklog `step`（含 `step_index`，解析/策略纠错的合成 Step 带 `correction`）；function_call 的 assistant response 记为 `assistant_message`，不是 Step |
| Turn | Session 内递增的 `turn` | `state.json` 的 `turn_seq` / `open_turn` / `turns_completed`；worklog `turn_started` … `turn_ended`；`static.json` 的 `turns` |
| 输入批次 | `(run_id, input_seq)` | snapshot 里的 receipt；worklog `turn_started`（开启 Turn）或 `input_batch`（并入当前 Turn） |
| 工具调用 | `call_id` | worklog `action_result`；工具运行上下文的 `tool_call_index` 只是调用序号，不是 Step 或 Turn |

Turn 的规则：没有打开的 Turn 时提交的输入批次开启新 Turn（bootstrap、msg/event）；`switch_context` 切换、子 context（`create_sub_context` / `fork`）的调用与返回、观察阶段注入、可恢复挂起（中断、可重试错误、context limit）、为本 run 启动的工作保留 run（它的任务、还没听到结局的子 session 未结束）、history epoch 重写和重启都继续当前 Turn，期间消费的新输入并入当前 Turn。只有 AgentSession 在 run 结束时关闭 Turn：交付结果为 `completed`；`WAIT_USER_MSG` 只有已经交付答复（本 run 有 `<report>`）才算 `completed`，否则 Turn 保持打开；不可恢复错误、预算耗尽、stop 分别记为 `failed`、`budget_exhausted`、`stopped`。子 context 的 run 结束只返回调用方，不关闭 Turn。Session 结束条件 `max_turns` 只统计 `completed` 的 Turn。


## 在AgentSession中的LLM Context 的状态机切换

AgentSession 构造/管理 多个LLM Context，同一时刻只有一个当前 LLMContext/run 被推进。
AgentSession 通过 prompt/input 和工具向 LLMContext 提供所需状态，LLMContext 不直接持有或自动看到 Session 的全部状态。


切换方式由**目标 behavior 的进入配置**决定，不由 Session 统一指定，也没有默认模式（libopendan `protocol/config.rs`：`ContextMode`、`InheritMode`、`BehaviorEntry`）：

```json
"extensions": { "opendan": { "behaviors": {
    "<name>": { "mode": "switch_context | create_sub_context | fork",
                "system_prompt": "…", "llm_context": { … }, "inherit": "none | recent_dialogue | steps" }
} } }
```

- `system_prompt` 替换 `prompt.system_prompt`；`llm_context` 的顶层键替换 `prompt.llm_context` 的同名键（模型、工具、限制等）。
- `inherit`（默认 `none`）：`recent_dialogue` 装配会话历史，context 切换时使用宿主从 worklog 重建的会话历史消息；未压缩 UI 的跨 Turn 续聊保留原始消息列表，见下文。`steps` 只用于 `create_sub_context`。`fork` 条目不能声明 `system_prompt` / `llm_context` / `inherit`。
- Session 的初始 behavior（`prompt.behavior`）没有自己的条目时，隐含为使用基础配置的 `switch_context` 目标（`inherit` 为 `recent_dialogue`；没有声明任何 behavior 的 Session 同样继承会话历史）。交接到其它没有条目的 behavior 不回退：Turn 以 `behavior_config` 错误失败；发生在子 context 内则作为失败结果返回调用方。
- behavior 表在 drive 启动时校验；`extensions.opendan.process_modes` 被拒绝。“在同一个 run 里换 behavior / system、沿用原历史”的普通切换已移除。

三种方式都不结束 Turn，也都不隔离文件系统、Session 状态、worklog 和对外消息；缓存是否命中取决于模型、system prompt、工具定义及历史渲染是否保持相同前缀。

### ContextSwitch（`switch_context`）

目标拥有自己的 context（system、工具、模型、历史、run）。current run 挂起进 `process_stack`（`FrameRole::Parked`）；栈里有 target 自己挂起的 run 就恢复它的快照，否则按 target 自己的配置新建 run，并通过 `on_behavior_switch` 输入继续推进。
新建时的历史只按 `inherit` 装配，其它 context 的快照 / 历史不会被接到 target 上。这类 context 的显式结束报告 按 Session 结束条件处理，不会自动返回上一个 context；回去要显式 `next_behavior`。

### CreateSubContext（`create_sub_context`）

每次调用新建子 run：使用 target 自己的 system、工具和任务输入，只带入 `inherit` 选择的调用方历史（`steps` = 调用方已完成的 StepRecord 与 summaries）。调用方作为 `FrameRole::Caller` 帧入栈，子 context 结束后恢复调用方并交回结果，子 context 的 steps / messages 不并入调用方的推理历史。底层构造是 llm_context 的 `derive_child(parent_snapshot, child_request, InheritHistory::{None | Material(msgs) | Steps})`。

### Fork（`fork`）

每次调用新建分支 run：保留调用方的 system、配置和**分叉点之前的完整有效历史**，在其后追加分支任务；调用返回方式与 create_sub_context 相同。底层构造是 `fork_snapshot(parent_snapshot, ForkOptions)`：调用方有未完成的工具批次 / behavior Step 时，分叉点取在该批次 / Step 之前（它留给调用方）；`expect_system` 与调用方 system 不同则拒绝；function_call 模式下继承的历史成为子 run 的 `request.input`。

`derive_child` / `fork_snapshot`（`llm_context/src/context_derive.rs`）都是纯函数：不修改父快照，不带走父的挂起状态、待派发调用、usage、错误计数和宿主元数据，延续父的 step / action 编号，并返回 `InheritBoundary {messages, steps_below, next_action_id, fork_point}`，宿主据此只把子 run 新增的部分写入 worklog。交接从不在已有历史上替换 system：llm_context 没有在已有快照上改 system / behavior 重建的入口。thinking 只对产生它的模型和前缀（system、tools、之前的消息）有效：`ResumeFill::RewrittenHistory` / `RewrittenSteps` 重写历史时 waist 自动丢弃 thinking；宿主自己改写快照（换模型、压缩）时用 `strip_thinking` / `strip_snapshot_thinking` 丢弃快照里的全部 thinking（`request.input`、`accumulated`、各 step 的 `assistant_message`）。

### 子 context 的触发与返回

| 触发 | 调用方停在 | 结果返回 |
|---|---|---|
| behavior 的 `next_behavior=B` | 完整 Step 的 Done | 调用方 `on_behavior_switch` 批次里的 `<process_result behavior status>` |
| 工具 / action `call_behavior({behavior, task})` | 调用立即返回"已登记"（run.json 记 `host.extra.sub_call`），run 继续到这一段停止行动（回复或让出，没有 action / tool call）的 Done | 同上，`<process_result behavior status call_id>` 带触发它的 `call_id`；一个 run 一次只能登记一个 |

子 context 无论以什么结束都返回调用方：`report end="true"` / 获准的工具结束报告 → `status=ok`；`WAIT_USER_MSG` → `status=needs_user_input`（子 context 从不消费调用方的输入队列）；不可重试错误、预算耗尽、未知交接目标 → `status=failed`；在子 context 内交接到 `switch_context` 目标也只是返回。子 context 可以继续调用子 context，最深 `session.policy.max_process_depth` 层（默认 4，`MAX_CALL_DEPTH`）。

交接点随快照一起写进 run.json 的 `handover`，先于 state 提交；崩溃后 reconcile（`runner/reconcile.rs::redo_transfer`）恰好补交一次。已返回的子 context 在 worklog 中保留全部记录供审计，但重建会话历史时只渲染它的 `process_done` 结果（`runner/history.rs`）；压缩一次读入整段未压缩范围，输入做同样的过滤；压缩发生时仍在运行的子 context，其记录会进入摘要。

UI Stop 后补充输入（H3）已于 2026-10-10 暂缓，移出当前实现与验收范围；不适用于 Work，终态 Work 不 reopen，后续修改创建新 Task / 新 Work。`report` 工具与可选显式完成策略（H4）已实现。XML 完成规则为：`<report end="true">` 显式请求结束且同一决策不得有 actions，缺省 / false 只更新报告；`next_behavior` 保留切换 / 等待语义，不再使用 END 或隐式 done 完成。详见 [Context 调度支持 TODO](../../notepads/done/llm-context-switch-support-todo.md) H3 / H4.1。XML parser（`agent_tool::xllm::XllmActionParser`）拒绝 END / done、重复 report、非法 end、空最终报告及结束与动作或非空 next_behavior 的冲突；没有动作、不结束、不交接的决策（含空决策）是让出（Yield），本次 run 以 `Done` 结束，接下来等什么由 Session 决定。工具报告持久化与最终消息由 Session 处理，独立 xllm 不默认暴露 report 工具。


## AgentSession Tree

上面基于 LLMContext 的切换都是串行的：同一 AgentSession 同时只推进一个顶层 context。
设计上，Session 级并行可以使用 Sub AgentSession，由各自的 Runner 推进；当前已有 `create_session` + `origin.parent_session` 的创建/父子登记能力，但启动子 Runner、等待及结果回传尚需调用方组合，没有完整的 Sub AgentSession helper。
    parent AgentSession 可以创建全新的 sub session，也可以向已有且未 finished 的 session 投递输入（需要知道 session_id，推进还须满足 driver 身份和锁约束）。
    构造 input，推进到所需的结果/等待边界。
    读取结果或订阅状态变化；指定 `origin.parent_session` 创建子 Session 时，当前实现会尝试为 parent 添加对子 Session 的半订阅。

Sub AgentSession 按访问规则通过 AgentStateClient/工具读取 Parent AgentSession，按自己的配置和 Runner 依赖绑定 Runtime。`create_sub_session` 会复制父 Session 的 llm_context 与 runtime 配置，但为子 Session 确定自己的绑定身份；这不等于复制父 Session 的完整推理上下文。Runtime 继承与绑定细节见 [AgentRuntime 设计](<AgentRuntime 设计.md>)。

## Agent Loop的几种典型伪代码和history 消息结构

伪代码只保留主干，省略错误、预算、快照提交和崩溃恢复。`组件.函数` 对应实现：AgentSession 是 libopendan 的 `runner::drive`（`runner/drive.rs`），LLMContext 在 `llm_context/context_loop.rs`，其余是 LLMContext 的依赖（`deps.rs`）。消息结构里 `@` 标出消息所属的 Round / Step。

```text
AgentSession                  Turn：取输入批次、推进 run、解释 Outcome、决定 Turn 是否结束
└─ LLMContext.run             一次 run：Round / Step 循环，返回 Outcome
   ├─ LlmClient.infer         一个 Round
   ├─ ToolManager.call_tool   原生工具 / behavior action，串行派发；工作没做完就返回"仍在运行，任务 X"
   └─ CheckpointHook          唯一的钩子，不是 Turn 钩子：每个工具调用前存外层快照（崩溃恢复的唯一依据）；
                              推理前（function_call：每个 Round 前；behavior：Step 边界）存外层快照、注入半订阅变化
```

### AgentSession 外层

```text
AgentSession.drive():
    loop:
        inputs = AgentSession.fetch_inputs()                  # msg / event 形成输入批次；change 只随行
        text   = SessionAssembler.render_input(inputs, hook)  # hook = on_init | on_wakeup | on_behavior_switch
        if 没有 text 且没有可恢复的 run: 等待输入; continue
        ctx  = live run ?? AgentSession.new_run_context()     # input = [system, ...继承历史]
        AgentSession.commit_input_batch(ctx, text)            # LLMContext.inject(user:<session_input>)，提交 receipt；
                                                              # 没有打开的 Turn 就开新 Turn，否则并入当前 Turn
        outcome = AgentSession.run_compacting(ctx)            # LLMContext.run()，一个 Outcome
        next = AgentSession.handle_context_outcome(ctx, outcome)   # next.turn_end 决定 Turn 是否结束
        match next:
            report_end     -> AgentSession.finish_run()       # run 历史 flush 进 worklog；Turn completed；按 end_condition 结束 Session 或等待
            WAIT_USER_MSG  -> AgentSession.finish_run()       # 已交付答复则 Turn completed，否则 Turn 保持打开；等下一条输入
            switch(B)      -> 见“behavior 切换”，Turn 继续
            本 run 的工作未结束 -> run 保留（paused，task 记 watched_tasks、子 session 记 wait_children），Turn 保持打开；
                              # 让出，或回复时本 run 启动的任务 / 子 session 还在跑；它们的结局
                              # （<<task_result …>> / <event … source="session:<sid>">）作为输入注入同一快照
            挂起 / 可重试错误 -> run 保留，Turn 保持打开，恢复后继续同一 run   # ContextLimitReached / Interrupted
            失败 / 预算耗尽 / stop -> AgentSession.finish_run()  # Turn 以 failed / budget_exhausted / stopped 结束
```

run 结束后，历史写入 Session worklog，`last_run` 的结果快照也保留。UI Session 的 `function_call` context 在已关闭的 Turn 之后继续同一 context、继承 `recent_dialogue` 且尚未生成摘要时，新 run 从该快照的 `accumulated` 原样继承非 system 消息列表；角色、工具调用 / 结果、媒体等内容不改写、不截断。新 run 的快照持有完整前缀，因此后续清理旧 run 不会丢失历史；继承部分不再重复写入 worklog。

原始历史超过 `history_budget_tokens` 时先实际压缩。已有摘要、behavior loop 或 context 切换时，历史由 worklog 重建为一串消息，不截断（`libopendan.history/4`）：`user_message` → user 消息；function call 的 `assistant_message` → 带原生工具调用的 assistant 消息，每个 `action_result` → 对应的 tool 结果；behavior 的 `step` → assistant 文本 + 一条 `<<last_step_action_results behavior="…" step="N">>` user 消息；最终的 `report_delivery` → assistant 文本。behavior context 没有原生工具，function call 记录也按这种文本形式渲染。超出预算时由 `agent_tool::llm_compress` 压缩（唯一的压缩方式），结果为 `system + 开头（首个输入 + 第一个 Round）+ 一条摘要 user 消息（<history_summary> + <keep_exact>）+ 原样的近期 Round`。`session_summary` 是磁盘摘要的 schema 名，当前没有独立的同名消息标签。细节见 [Session Directory Protocol §6](<../opendan/protocol/Session Directory Protocol.md>)。

### 标准 function_call Loop

```text
LLMContext.run_inner():
    loop:
        CheckpointHook.before_inference(snapshot)        # 每个 Round 之前
        resp = LlmClient.infer(accumulated)              # 一个 Round
        if resp 没有 tool_calls:
            return Done(resp)                            # 最后一个 Round，assistant 文本就是结果
        if tool_iterations_left == 0 and 批次不全是 wrap_up_tools 或收尾批数已用完:
            return BudgetExhausted(ToolIterations)       # 剩 0 次时请求只广告收尾工具（见上文收尾模式）
        accumulated += resp.message                      # assistant: tool_calls
        for call in resp.tool_calls:                     # 一个工具批次，串行派发
            CheckpointHook.before_tool_call(snapshot)    # 每个调用之前
            accumulated += ToolManager.call_tool(call)   # tool: result
```

```text
[system] [...继承历史] [user:<session_input hook=on_wakeup> 输入]
[assistant:tool_calls] [tool] [tool]    @Round1
[assistant:tool_calls] [tool]           @Round2
[assistant:回答]                         @Round3 → Done
```

三个 Round，没有 Step，一个 Turn，扣两次工具迭代额度（每个工具批次一次）。worklog 里每个 assistant response 记为 `assistant_message`。工具结果驱动下一个 Round；run 进行中只有半订阅变化可能在 Round 边界注入，新的 msg/event 等 run 返回后再作为输入批次进入（Turn 仍打开时并入当前 Turn）。

未压缩 UI 的 `...继承历史` 是之前各 Turn 的原始 user / assistant / tool 消息序列；使用 worklog 视图时则是从 worklog 重建的消息序列（同样带原生工具调用；压缩后含一条 `<history_summary>` 摘要消息）。

### Behavior Loop

```text
LLMContext.run_behavior():
    loop:
        CheckpointHook.before_inference(外层 snapshot)   # Step 边界（观察阶段）
        resp = LLMContext.run_inner_for_step()          # 内层就是上面的 run_inner：可含多个 Round 和原生工具
        step = LLMResultParser.parse(resp)              # 解析失败 → 合成错误 Step 喂回，重来
        if step 没有 action:
            return Done(behavior_result)                # report end / next_behavior（WAIT_USER_MSG / 跳转目标）/ 让出
        for action in step.actions:                     # 按序派发，每个之前存快照，遇到业务错误即停；带 action 的 Step 扣一次工具迭代额度
            step.action_results += ToolManager.call_tool(action)
        LLMContext.sediment(step)                       # 成为热 step，下一次推理时看到它的结果
```

内层每次推理的消息由 `LLMContext.build_inner_request` 现场拼出：

```text
[system] [会话历史消息…] [user:<session_input hook=on_init>]                      ← request.input
[user:<<step_history>> 摘要 / 其它 behavior 的 step / 历史输入 <</step_history>>]
[assistant:决策] [user:<<last_step_action_results>>]                               @Step3
[assistant:决策] [user:<<last_step_action_results>> + 观察阶段注入]                  @Step4（热 step）
[assistant:tool_calls] [tool]                                                     @Step5 的 Round1
```

Step5 的下一个 Round 输出决策后，Step5 才完成并沉淀为 `StepRecord`；只保留决策和 action 结果，Step 内原生工具的消息（inner transcript）不再出现在之后的 prompt 中。Step5 未完成时挂起，inner transcript 留在外层快照里，恢复后继续这个 Step，不重放已执行的工具。

### behavior 切换

切换方式由 target behavior 的进入配置决定（`extensions.opendan.behaviors.<B>.mode`），没有默认模式。

```text
AgentSession.handle_context_outcome(Done{next_behavior: B}):     # classify_done 读取 B 的 BehaviorEntry
    B 没有条目 -> Turn failed（behavior_config）；在子 context 内 -> 以 failed 返回调用方

# switch_context：B 有自己的 context
AgentSession.suspend_run(run_A, Parked)        # run_A 入 process_stack（FrameRole::Parked），交接点已先写入 run.json
run_B = 栈里 B 自己挂起的 run ?? AgentSession.new_run_context()   # 新建：B 的 system / 工具 / 模型，历史只按 inherit
# 下一圈 commit_input_batch 注入 <session_input hook=on_behavior_switch><behavior_switch to=B/>（worklog: input_batch）
# B 的显式结束报告按 Session 完成策略处理，不自动回到 A

# create_sub_context / fork：B 是子 context（触发 a：next_behavior）
AgentSession.suspend_run(run_A, Caller{ChildCall{mode, behavior: B, trigger: Behavior}})
AgentSession.new_run_context()                 # run_B：derive_child（B 的 system + inherit）| fork_snapshot（A 的 system + 完整历史）
# run_B 的交接批次：<behavior_switch to=B/><sub_task mode="…">…</sub_task>
... run_B 推进到结束                             # 显式结束报告 / WAIT_USER_MSG / 错误 / 其它交接都视为返回调用方
AgentSession.finish_run(run_B)                 # outcome process_done；出栈，run_A 重新成为 live run；Turn 不结束
# 下一圈 commit_input_batch 把 <process_result behavior=B status=ok|failed|needs_user_input> 注入 run_A

# create_sub_context / fork（触发 b：工具 / action call_behavior({behavior: B, task})）
ToolManager.call_tool(call_behavior) -> "Sub context B is registered …"   # 立即返回；run.json 记 host.extra.sub_call
... run_A 继续，直到这一段停止行动（回复或让出，没有 action / tool call）返回 Done
AgentSession.suspend_run(run_A, Caller{ChildCall{mode, behavior: B, trigger: Tool{call_id, task_id: "subctx:<call_id>"}, task}})
... run_B 同上推进到结束
# 下一圈 commit_input_batch 把 <process_result behavior=B status=… call_id=…> 注入 run_A
```

create_sub_context（`next_behavior` 触发）前后的消息：

```text
run_A: [system:A] [会话历史消息…] [<session_input>] … [assistant:A 决策 next_behavior=B]                → 入栈（Caller）
run_B: [system:B] [<session_input on_behavior_switch><sub_task mode=create_sub_context>] [<<step_history>> A 的 step（inherit: steps 时）] … B 的 steps → 结束
run_A: … [assistant:A 决策 next_behavior=B] [user:A 该 Step 的结果 + <session_input on_behavior_switch><process_result behavior=B status=ok>…]   → 继续
```

fork 的 run_B 则以 run_A 的 system 和分叉点之前的完整历史开头（渲染与 run_A 相同），其后才是带 `<sub_task mode="fork">` 的交接批次。

返回时的交接批次并入 run_A 最后一个 Step 的 user 消息（`InjectionPosition::Step`）。run_B 的 steps 不并入 run_A 的 history；Session worklog 仍记录 run_B 的过程（Step 身份 `(run_B, step_index)`，继承的 step 不重复写入），run_B 返回后重建的会话历史只渲染它的 `process_done` 结果。各个 run 段都属于同一个 Turn。

### 挂起与恢复

```text
ContextLimitReached  -> AgentSession.rewrite_for_limit：flush 到 worklog → 压缩 summary → 以 system + 重建的历史消息（新 history epoch）
                        LLMContext.resume(snapshot, RewrittenHistory | RewrittenSteps)
Interrupted          -> 用推理前的快照 LLMContext.resume(snapshot, ResumeFromMidRun)，重做这次推理
崩溃                  -> 最近的 checkpoint（工具调用前 / 推理前快照）LLMContext.resume(snapshot, ResumeFromMidRun)；
                        没有结果的调用以同一 call_id 再派发，ToolManager 先按 call_id 找回之前的工作，
                        幂等工具重跑，其余答"结果未知"（Unresolved），不盲目重放
```

这些挂起都不结束 Step 或 Turn；恢复时不重扣已扣的工具迭代额度。Interrupted 只有真实发起过的推理才计入 Round。

loop 里没有等待，任何等待都是任务：工具在 Runtime 的等待预算（`RuntimeConfig.task_wait_ms`，默认 25 秒）内没做完，就返回"仍在运行，任务 X"。libopendan 在 loop 外跟进本 run 的任务和子 session，结局以 `<<task_result task=… call_id=… status=…>>` / `<event … source="session:<sid>" …>` 输入注入同一快照（run 保留期间 Turn 保持打开）；提问用户也是任务：websdk `ask_user` 发 `input` 通知，答复是这个任务的结果。xllm 自己的 run 由模型用 `wait_task` / `get_task_state` 跟进，让出或回复时仍有运行中的任务就停为 `paused`，`xllm --resume [message]` 继续。xllm 不能 resume 停在交接点（run.json `handover`）的宿主 run。


## AgentRuntime

完整设计见 [AgentRuntime 设计](<AgentRuntime 设计.md>)：分层、配置、执行体、工具与模板接入、长任务、恢复和 Session 绑定。

共享实现位于 agent_tool::runtime。RuntimeConfig / RuntimeRegistry 支持 native、remote_ssh，以及需要宿主 Node Executor connector 的 remote_node；AgentRuntime.open 解析工具并返回实现 Sandbox/ToolManager 的派发器。内置 shell、文件读写编辑使用同一执行体；MCP 服务与宿主进程内工具保留各自位置。PromptExec 有显式注入适配器，生产模板默认未启用，remote_node 尚不支持模板 EXEC。

RuntimeInfo 提供执行环境信息供提示词引用；native/SSH 刷新执行侧时间和时区，remote_node 当前使用控制侧 UTC 时间。Session 的稳定信息放 system，新鲜时间、半订阅与 active sessions 仍由 Session 输入批次管理。配置/日志/快照/INCLUDE 属于控制侧素材，runtime.workdir 属于执行侧。

恢复读取各执行体的命令目录或 Node 操作 locator，不核验、不停止旧进程，也不盲目重放：工具调用前快照里第一个没有结果的调用（`exec_tracking::in_flight_call`）再派发时，ToolManager 按 call_id 找回（shell 的任务 `local:shell:<call_id>` 已结束就用它的结果），幂等工具重跑，其余由 `AgentRuntime::describe_interrupted` 生成说明、以结果未知交给 LLM。有 task manager 且等待预算（`RuntimeConfig.task_wait_ms`）小于命令超时时，shell 命令在预算内没结束就移交 task manager；之后 xllm 的模型用 `wait_task` / `get_task_state` / `cancel_task` 跟进，Session 宿主在 loop 外跟进、结局作为输入回来。完整 Session 目前支持 native 与创建时冻结到节点 runtime 的 remote_node；SSH 远端 Session helper 未部署，独立 xllm 可使用 SSH/Node。policy、grant、approval 与其它执行体留到后续阶段。SDK 入口见 [xllm Rust SDK §11](xllm_rust_sdk.md#11-共享-agentruntime)。

## AgentState (RootFS)

- SessionMgr / WorkspaceMgr等
- 基于behavior name 构造LLM Context时，可能最要通过AgentState
- 为Runtime的一些Agent相关状态tool提供实现 （尤其是Memory相关）


## 命令行工具
- xllm 加载(创建) local_llm_context,并运行到一个Step结束
- xagent 加载（创建）AgentSession，并运行(到turn结束)。xagent拉起的也可以是一个持续获取input不断执行TurnLoop的AgentSession

## TS 运行时

xllm 的 TypeScript 版本在 websdk（`buckyos/xllm`）：同一套任务语义（配置、提示词装配、function_call 与 behavior 循环、预算与六态结束、Run 记录与恢复、观察事件），core 只用 Web 标准 API，同一份代码在浏览器页面、Node 和 Deno 上跑。浏览器里没有通用 runtime：网站用 `createSiteRuntime` 把自己的 JS 函数包成 `site` runtime（场景进身份，配置只能收窄），可选的基础工具 `siteTools` 把页面的资源和命令做成 `read` / `write` / `edit` / `exec`。Deno 宿主实现 `native` runtime，能接手同机 Rust 跑到一半的 run 目录。与 Rust 的一致性由共享向量证明（Rust `xllm::vectors` 生成并校验，websdk 消费）。详见 [xllm TS 运行时](xllm_ts_runtime.md)。
