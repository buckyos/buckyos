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

Turn 的规则：没有打开的 Turn 时提交的输入批次开启新 Turn（bootstrap、msg/event）；`switch_context` 切换、子 context（`create_sub_context` / `fork`）的调用与返回、观察阶段注入、可恢复挂起（中断、可重试错误、context limit、PendingTool）、history epoch 重写和重启都继续当前 Turn，期间消费的新输入并入当前 Turn。只有 AgentSession 在 run 结束时关闭 Turn：交付结果为 `completed`；`WAIT_USER_MSG` 只有已经交付答复（`<report>` 或 `<sendmsg>`）才算 `completed`，否则 Turn 保持打开；不可恢复错误、预算耗尽、stop 分别记为 `failed`、`budget_exhausted`、`stopped`。子 context 的 run 结束只返回调用方，不关闭 Turn。Session 结束条件 `max_turns` 只统计 `completed` 的 Turn。


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
- `inherit`（默认 `none`）：`recent_dialogue` 装配宿主渲染的 `<session_history>`；`steps` 只用于 `create_sub_context`。`fork` 条目不能声明 `system_prompt` / `llm_context` / `inherit`。
- Session 的初始 behavior（`prompt.behavior`）没有自己的条目时，隐含为使用基础配置的 `switch_context` 目标（`inherit` 为 `recent_dialogue`；没有声明任何 behavior 的 Session 同样装配 `<session_history>`）。交接到其它没有条目的 behavior 不回退：Turn 以 `behavior_config` 错误失败；发生在子 context 内则作为失败结果返回调用方。
- behavior 表在 drive 启动时校验；`extensions.opendan.process_modes` 被拒绝。“在同一个 run 里换 behavior / system、沿用原历史”的普通切换已移除。

三种方式都不结束 Turn，也都不隔离文件系统、Session 状态、worklog 和对外消息；缓存是否命中取决于模型、system prompt、工具定义及历史渲染是否保持相同前缀。

### ContextSwitch（`switch_context`）

目标拥有自己的 context（system、工具、模型、历史、run）。current run 挂起进 `process_stack`（`FrameRole::Parked`）；栈里有 target 自己挂起的 run 就恢复它的快照，否则按 target 自己的配置新建 run，并通过 `on_behavior_switch` 输入继续推进。
新建时的历史只按 `inherit` 装配，其它 context 的快照 / 历史不会被接到 target 上。这类 context 的 `END` 按 Session 结束条件处理，不会自动返回上一个 context；回去要显式 `next_behavior`。

### CreateSubContext（`create_sub_context`）

每次调用新建子 run：使用 target 自己的 system、工具和任务输入，只带入 `inherit` 选择的调用方历史（`steps` = 调用方已完成的 StepRecord 与 summaries）。调用方作为 `FrameRole::Caller` 帧入栈，子 context 结束后恢复调用方并交回结果，子 context 的 steps / messages 不并入调用方的推理历史。底层构造是 llm_context 的 `derive_child(parent_snapshot, child_request, InheritHistory::{None | Material(msgs) | Steps})`。

### Fork（`fork`）

每次调用新建分支 run：保留调用方的 system、配置和**分叉点之前的完整有效历史**，在其后追加分支任务；调用返回方式与 create_sub_context 相同。底层构造是 `fork_snapshot(parent_snapshot, ForkOptions)`：调用方有未完成的工具批次 / behavior Step 时，分叉点取在该批次 / Step 之前（它留给调用方）；`expect_system` 与调用方 system 不同则拒绝；function_call 模式下继承的历史成为子 run 的 `request.input`。

`derive_child` / `fork_snapshot`（`llm_context/src/context_derive.rs`）都是纯函数：不修改父快照，不带走父的挂起状态、待派发调用、usage、错误计数和宿主元数据，延续父的 step / action 编号，并返回 `InheritBoundary {messages, steps_below, next_action_id, fork_point}`，宿主据此只把子 run 新增的部分写入 worklog。`snapshot_overrides`（`rebuild_with_inherit`）只用于从**同一个 run** 自己的快照改 request 侧参数重建，不用于 context 之间的交接：交接从不在已有历史上替换 system。thinking 只对产生它的模型和前缀（system、tools、之前的消息）有效：override 真正改变了 system、历史、提供给模型的工具 / action 集合、模型（`preferred` / `fallbacks`）或 `behavior_name` 时，`apply_overrides_to_snapshot` 用 `strip_snapshot_thinking` 丢弃快照里的全部 thinking（`request.input`、`accumulated`、各 step 的 `assistant_message`）；值没变或只改预算、trace、objective 等不进前缀的字段时保留。宿主自己改写快照（换模型、压缩）时复用 `strip_thinking` / `strip_snapshot_thinking`。

### 子 context 的触发与返回

| 触发 | 调用方停在 | 结果返回 |
|---|---|---|
| behavior 的 `next_behavior=B` | 完整 Step 的 Done | 调用方 `on_behavior_switch` 批次里的 `<process_result behavior status>` |
| 工具 / action `call_behavior({behavior, task})` | `PendingTool`（工具返回 `Pending{task_id="subctx:<call_id>"}`），保留未完成的批次 / Step | `ResumeFill::ToolResults` 按 `call_id` 回填，批次余下的调用继续；不产生输入批次 |

子 context 无论以什么结束都返回调用方：`END` / report → `status=ok`；`WAIT_USER_MSG` → `status=needs_user_input`（子 context 从不消费调用方的输入队列）；不可重试错误、预算耗尽、未知交接目标 → `status=failed`；在子 context 内交接到 `switch_context` 目标也只是返回。工具触发时 failed 回填为工具错误，needs_user_input 回填为结构化 JSON。子 context 可以继续调用子 context，最深 4 层（`MAX_CALL_DEPTH`）。只有 `call_behavior` 可以 deferred，其它工具的 `allow_deferred` 被屏蔽。

交接点随快照一起写进 run.json 的 `handover`，先于 state 提交；崩溃后 reconcile（`runner/reconcile.rs::redo_transfer`）恰好补交一次。已返回的子 context 在 worklog 中保留全部记录供审计，但重建 `<session_history>` 时只渲染它的 `process_done` 结果（`runner/history.rs`）；压缩输入做同样的过滤，但压缩只识别被压缩片段内的 `process_done`：切点把子 run 的记录与它的 `process_done` 分开时，切点之前的那部分仍会进入摘要。

未实现：UI Stop 后补充输入（H3）、`report` 工具与显式完成策略（H4）。2026-10-10 已确定 H4 的 XML 核心规则：`<report end="true">` 显式请求结束且同一决策不得有 actions，缺省 / false 只更新报告；`next_behavior` 保留切换 / 等待语义，不再使用 END 或隐式 done 完成。详见 [Context 调度支持 TODO H4.1](../../notepads/llm-context-switch-support-todo.md)。本页其余 END / report-only 描述及执行伪代码仍是实施前基线，尚未代表新规则已落地。


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

切换方式由 target behavior 的进入配置决定（`extensions.opendan.behaviors.<B>.mode`），没有默认模式。

```text
AgentSession.handle_context_outcome(Done{next_behavior: B}):     # classify_done 读取 B 的 BehaviorEntry
    B 没有条目 -> Turn failed（behavior_config）；在子 context 内 -> 以 failed 返回调用方

# switch_context：B 有自己的 context
AgentSession.suspend_run(run_A, Parked)        # run_A 入 process_stack（FrameRole::Parked），交接点已先写入 run.json
run_B = 栈里 B 自己挂起的 run ?? AgentSession.new_run_context()   # 新建：B 的 system / 工具 / 模型，历史只按 inherit
# 下一圈 commit_input_batch 注入 <session_input hook=on_behavior_switch><behavior_switch to=B/>（worklog: input_batch）
# B 的 END 按 Session 结束条件处理，不自动回到 A

# create_sub_context / fork：B 是子 context（触发 a：next_behavior）
AgentSession.suspend_run(run_A, Caller{ChildCall{mode, behavior: B, trigger: Behavior}})
AgentSession.new_run_context()                 # run_B：derive_child（B 的 system + inherit）| fork_snapshot（A 的 system + 完整历史）
# run_B 的交接批次：<behavior_switch to=B/><sub_task mode="…">…</sub_task>
... run_B 推进到结束                             # END / WAIT_USER_MSG / 错误 / 其它交接都视为返回调用方
AgentSession.finish_run(run_B)                 # outcome process_done；出栈，run_A 重新成为 live run；Turn 不结束
# 下一圈 commit_input_batch 把 <process_result behavior=B status=ok|failed|needs_user_input> 注入 run_A

# create_sub_context / fork（触发 b：工具 / action call_behavior({behavior: B, task})）
ToolManager.call_tool(call_behavior) -> Pending{task_id: "subctx:<call_id>"}    # run_A 以 PendingTool 挂起，批次 / Step 未完成
AgentSession.suspend_run(run_A, Caller{ChildCall{mode, behavior: B, trigger: Tool{call_id, task_id}, task}})
... run_B 同上推进到结束
LLMContext.resume(run_A 快照, ToolResults{call_id: 子结果})     # 不产生输入批次，批次余下的调用继续
```

create_sub_context（`next_behavior` 触发）前后的消息：

```text
run_A: [system:A] [<session_history>] [<session_input>] … [assistant:A 决策 next_behavior=B]                → 入栈（Caller）
run_B: [system:B] [<session_input on_behavior_switch><sub_task mode=create_sub_context>] [<<step_history>> A 的 step（inherit: steps 时）] … B 的 steps → 结束
run_A: … [assistant:A 决策 next_behavior=B] [user:A 该 Step 的结果 + <session_input on_behavior_switch><process_result behavior=B status=ok>…]   → 继续
```

fork 的 run_B 则以 run_A 的 system 和分叉点之前的完整历史开头（渲染与 run_A 相同），其后才是带 `<sub_task mode="fork">` 的交接批次。

返回时的交接批次并入 run_A 最后一个 Step 的 user 消息（`InjectionPosition::Step`）。run_B 的 steps 不并入 run_A 的 history；Session worklog 仍记录 run_B 的过程（Step 身份 `(run_B, step_index)`，继承的 step 不重复写入），run_B 返回后重建的 `<session_history>` 只渲染它的 `process_done` 结果。各个 run 段都属于同一个 Turn。

### 挂起与恢复

```text
PendingTool          -> 等工具结果，LLMContext.resume(snapshot, ToolResults)，从未完成的批次 / action 继续
ContextLimitReached  -> AgentSession.rewrite_for_limit：flush 到 worklog → 压缩 summary → 以新的 [system, <session_history>]
                        LLMContext.resume(snapshot, RewrittenHistory | RewrittenSteps)
Interrupted          -> 用推理前的快照 LLMContext.resume(snapshot, ResumeFromMidRun)，重做这次推理
```

这些挂起都不结束 Step 或 Turn；恢复时不重扣已扣的工具迭代额度。Interrupted 只有真实发起过的推理才计入 Round。

deferred 工具只有一种：libopendan 的 `call_behavior`（子 context 调用），run 以 `PendingTool` 挂起，子 context 返回后由 Session 用 `ToolResults` 回填。其它工具的 `allow_deferred` 被屏蔽（xllm 自己的 run 始终关闭）：工具返回 `pending` 时派发器在调用内等 task（最长 30 分钟）。xllm 不能接手挂起在子 context 上的 run，也不能 resume 停在交接点（run.json `handover`）的宿主 run。


## AgentRuntime

共享实现位于 agent_tool::runtime。RuntimeConfig / RuntimeRegistry 构造 native、tmux、remote_ssh，AgentRuntime.open 解析工具并返回实现 Sandbox/ToolManager 的派发器。内置 shell、文件读写编辑、注入 PromptExec 的模板执行使用同一执行体；MCP 服务与宿主进程内工具保留各自位置。

RuntimeInfo 提供实际执行侧的 id、kind、os、arch、hostname、shell、cwd、tools、current_time、timezone，供提示词引用；Session 的稳定信息放 system，新鲜时间、半订阅与 active sessions 仍由 Session 输入批次管理。配置/日志/快照/INCLUDE 属于控制侧素材，runtime.workdir 属于执行侧。

shell 命令的输出与退出码写入执行目录 `runs/<run_id>/exec/<call_id>/`；恢复时不核验、不停止任何进程，也不重放，没有结果的调用由 `AgentRuntime::describe_interrupted` 按 runtime 生成“被打断”的说明交给 LLM。shell 默认 auto 模式，超过 `wait_ms` 的命令转为进程内 task，模型用 `wait_task` / `get_task_state` / `cancel_task` 跟进。native/tmux 不提供 OS 隔离；SSH 目标需 Linux/bash/SFTP，远端 Session helper 未部署时报 Capability。policy、grant、approval 与其它执行体留到后续阶段。接口、配置示例与恢复规则见 [xllm Rust SDK §10](xllm_rust_sdk.md#10-共享-agentruntime)。

## AgentState (RootFS)

- SessionMgr / WorkspaceMgr等
- 基于behavior name 构造LLM Context时，可能最要通过AgentState
- 为Runtime的一些Agent相关状态tool提供实现 （尤其是Memory相关）


## 命令行工具
- xllm 加载(创建) local_llm_context,并运行到一个Step结束
- xagent 加载（创建）AgentSession，并运行(到turn结束)。xagent拉起的也可以是一个持续获取input不断执行TurnLoop的AgentSession
