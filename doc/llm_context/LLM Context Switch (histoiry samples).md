# LLM Context Switch (with history samples)

说明 LLMContext + AgentSession 如何构造、切换和恢复典型的 Message List。Round / Step / Turn 的定义与 [readme](readme.md) 一致；Context 调度采用本次 review 和[提示词方式图解](<../opendan/几种典型的提示词方式图解.drawio>)确定的新语义。

本文区分**已实现的行为**、**已定稿但尚未实现的设计**和**待定建议**。切换模式由目标 behavior 决定；普通切换已删除；`SWITCH_CONTEXT` 是目标自己保有 context 的切换；更换 system prompt 的子调用称为 `create-sub-context`，保留 system 与完整历史的子分支才称为 `fork`。Context 调度一节（三种进入模式、两种触发方式、hosted run 交接）已在 llm_context / xllm / libopendan 落地，配置拼写见 [Session Directory Protocol](../opendan/protocol/Session%20Directory%20Protocol.md) §4.2 / §8。2026-10-10 已实现 XML 用 `<report end="true">` 显式结束且不得有 actions，两套 parser 与宿主使用同一提交语义，见下文 report 一节。Stop 后补充输入（H3）已暂缓，不适用于 Work 生命周期，强制 reopen 不在本轮范围；report 宿主策略与验收见 [Context 调度支持 TODO](../../notepads/llm-context-switch-support-todo.md) H3 / H4。END / 隐式 report-only 完成已移除；旧 opendan Runtime 的历史描述不作为当前协议。

## Round、Step、Turn 与 run

| 术语 | 含义 | 消息 / 历史边界 |
|---|---|---|
| Round | 宿主一次 `LlmClient::infer` 调用，即一次模型推理尝试 | 成功时产出一次 assistant response；失败 / 中断的尝试、无工具的最终回答也计入 Round |
| Step | Behavior Loop 的一次行为决策及其 action 结果，记录为 `StepRecord` | 内部可有多个 Round 和原生工具批次；function_call Loop 没有 Step |
| Turn | AgentSession 的一次逻辑 Input → result | 可跨多个 Step、behavior/context 切换和 run；由 Session 决定何时结束 |
| `run()` | 一次 `LLMContext::run()` 调用 | 返回一个 Outcome；返回不等于 Step、Turn 或 Session 已结束 |
| run / `run_id` | 宿主保存的执行记录及快照 | 同一 run 可以经多次 `run()` / resume 继续；fork / create-sub-context 创建子 run，SWITCH_CONTEXT 恢复或新建目标 run |
| 工具迭代 | 工具预算的计量单位 | 一个完成派发的原生工具批次，或一个带 action 的 Step，各计一次 |

原文把“一条 user 输入 → 最终 assistant 回答”称为 Round，现在应按 **Turn** 理解；把“一段 behavior 执行到 `next_behavior`”称为 Round，现在应称为一次 **`run()` 执行段**。这两种旧用法不能统一替换成同一个词。

`ToolPolicy.max_tool_iterations` 统计工具迭代，不统计 Round。最后无工具的回答、纯 report 的 Step 和解析纠错不扣工具迭代额度；`max_calls_per_round` 才限制一个 Round 中的原生 tool call 数。

本文用以下记号表示消息形状；省略的标签闭合、工具参数和消息正文不影响边界。示例中的 Round 编号仅表示推理先后，Step 的实际身份是 `(run_id, step_index)`，不能把它们当作 Session Turn 编号。

```text
[system:...]                         系统提示词
[user:...]                           user 角色消息，可能由 Session / renderer 构造
[assistant:tool_calls] [tool:结果]    原生工具调用及结果；结果不是新的用户输入
[assistant:决策] [user:动作结果]      一个 StepRecord 的渲染形状
@RoundN / @StepN / @TurnN             对应层次的边界标注
```

### 标准 function_call Loop：一个 Turn，多个 Round

```text
[system] [user:<session_history>] [user:<session_input hook=on_wakeup> 输入]
[assistant:tool_calls(a,b)] [tool:a结果] [tool:b结果]  @Round1
[assistant:tool_calls(c)]   [tool:c结果]               @Round2
[assistant:最终回答]                                  @Round3 → Done
                                                       → Session 完成 Turn1
```

这里有三个 Round、两次工具迭代、没有 Step。一个 Round 可以发出多个 tool call，工具结果驱动下一个 Round，不需要再来一条外部 user message。

Turn1 完成后，下一批输入可以开启 Turn2。libopendan 通常为下一个 Turn 新建 run，之前的历史从 Session worklog / summary 重建为 `<session_history>`，不是把所有 Turn 的原始消息直接接在一个永久增长的 LLMContext 后面。

### Behavior Loop：一个 Step 内也可有多个 Round

```text
[system] [user:<session_history>] [user:<session_input hook=on_init> 输入]
[assistant:tool_calls(a)] [tool:a结果]                  @Step0 / Round1
[assistant:决策 actions=[b,c]]                         @Step0 / Round2
    → 依次执行 b、c，结果记入 Step0.action_results
    → Step0 完成，沉淀为 StepRecord
```

Step0 内的原生工具消息只属于进行中 Step 的 inner transcript。Step 完成后，下一次推理看到的是决策和 action 结果，不再回放 Step 内的原生工具消息：

```text
[system] [user:<session_history>] [user:<session_input hook=on_init> 输入]
[assistant:Step0决策] [user:<<last_step_action_results>> b、c结果]
[assistant:Step1决策 next_behavior=CHECK]               @Step1 / Round3 → Done
```

以上两个 Step、三个 Round，共消耗两次工具迭代（一个原生批次 + 一个带 action 的 Step）。`next_behavior=CHECK` 交给 AgentSession 处理；切换到 CHECK 后仍继续当前 Turn，不能因为 `run()` 返回 `Done` 就算交互结束。

若 Step 尚未完成就挂起，快照会保留 inner transcript、待续派的工具批次或 `action_step`；恢复后继续同一个 Step，不重放已完成的工具。不能把每次 `run()` 返回都画成新增一个完整 StepRecord。

`last_step_result` / `last_step_observation` 在旧文中都是对“上一步结果输入”的泛称。当前默认 renderer 使用 `<<last_step_action_results>>`，内容来自 `StepRecord.action_results`；`StepRecord.observation` 则是模型在决策里写出的观察，两者不应混用。

## User Message：输入批次与继续推理

这一节属于 AgentSession 层：Session 决定什么时候取外部输入，LLMContext 决定如何继续推理。**user 角色消息不等于新 Turn，assistant 角色消息也不等于 Turn 结束。**

### Turn 不变量：没有打开的 Turn 时，输入批次才开启新 Turn

- 没有打开的 Turn：提交 bootstrap、msg / event 等输入批次时开启新 Turn。
- 已有打开的 Turn：SWITCH_CONTEXT、fork / create-sub-context 调用与返回，以及期间消费的新输入，都并入当前 Turn。
- `PendingTool`、`ContextLimitReached`、`Interrupted` 和可重试错误不关闭 Turn；恢复后继续。历史压缩、快照保存和进程重启也不是 Turn 边界。
- 交付结果时记为 `completed`；`WAIT_USER_MSG` 只有已交付答复（report / sendmsg）才完成 Turn，否则后续输入仍并入当前 Turn。
- 不可恢复错误、预算耗尽、Session stop 分别以 `failed`、`budget_exhausted`、`stopped` 关闭 Turn。`max_turns` 只统计 `completed`。

因此，不能保留“Round 中间不应插入 UserMessage”或“一条 UserMessage 必然推动 NextRound / NextTurn”这类不变量。真正要保证的是输入只在允许的推理边界注入，不能插进尚未配对完成的工具批次。

### Pending Message、Event 与半订阅变化

| 输入类别 | 驱动方式 | 进入上下文的方式 |
|---|---|---|
| Message | Session 按输入策略接收并消费 | 作为输入批次渲染进 `<session_input>` |
| Event | 由订阅、过滤和当前 Session 策略决定是否消费 | 被选中后进入输入批次；不保证每个 event 都触发推理 |
| 半订阅变化（change） | 不独立驱动一个新 Turn | 随输入批次或在观察边界注入变化 / 快照 |

Timer 是典型例子：若每个 tick 都驱动新推理，Session 就难以等待。半订阅只在需要时把当前状态或变化带入上下文。

旧 Runtime 曾用“Ban / mask”描述按 Session 状态屏蔽 event 的设计；其中 `PullEventPolicy::Filter(...)` 是静态字符串过滤，不能据此认为已经实现了按状态动态生成 mask。

### Driver：输入消费策略属于 Session

Session 负责取输入、处理交接和判定 Turn；behavior 决定如何解释输入并产出决策。相同 behavior 可以由不同 Session 的输入策略驱动，不能把输入消费方式固定在 behavior 上。

当前 libopendan 的边界如下：

| 边界 | function_call Loop | Behavior Loop |
|---|---|---|
| Session 输入批次 | `on_init` / `on_wakeup` 等批次进入 run | 还包括 `on_behavior_switch` 交接批次 |
| `CheckpointHook` | 每个 Round 前 | 外层 Step 边界；不在 Step 内每个 Round 消费 Session 输入 |
| 观察阶段注入 | 半订阅变化可追加为 user 消息 | 半订阅变化可并入热 Step 的结果消息 |
| `InferenceHook` | 每个 Round 前提供同步快照 | Step 内每个 Round 前提供内层快照；不是 Turn 钩子 |

运行期间，新的 msg / event 等 `run()` 返回后再作为输入批次处理；如果 Turn 仍打开，就并入当前 Turn。观察边界当前只注入半订阅变化，不能从“支持 user 消息注入”推导出任意 behavior 都会从 Session 队列拉取消息。

旧 `src/frame/opendan` Runtime 使用 `[session.<class>.driver]`，包含 `keep_alive` / `switch_mode` / `inject_background_environment` / `report_delivery` 以及 `on_init` / `on_behavior_switch` / `on_behavior_step_ob` / `on_wakeup` 四个 hook；每个 hook 配置 `filter`、`pull_msg`、`pull_event`。这是旧配置模型，详见 [Agent配置改进](../opendan/Agent配置改进.md)，不要与 libopendan 的 Driver 身份及输入批次协议混为一谈。

旧模型允许在 Step 观察阶段拉取 pending input；其“pull 即消费，模板没引用也会出队”同样是旧 Runtime 的语义。当前 libopendan 通过快照中的 input receipt 提交消费位置并恢复输入批次，不能沿用“无需提交对账”的描述。

### Turn 结束与 Session 结束

| 边界 | 含义 | 新输入行为 |
|---|---|---|
| `LLMContext::run()` 返回 | 本次执行段让出控制权 | 由宿主解释 Outcome，可能恢复、切换或结束 |
| Turn 完成 | 一次逻辑交互已交付结果 | Session 若仍接受输入，下一批输入开启新 Turn |
| Session finished | Session 达到结束条件或被终止 | 按 Session 协议拒绝继续推进，另建 Session 承载后续任务 |

按 [OpenDAN Agent Session 架构设计](<../opendan/OpenDAN Agent Session架构设计.md>) §6，WorkSession 执行一个明确、有界的 Task，成功或失败后输出 Final Report 并结束；终态不 reopen。Work 无普通 Message input，执行中同目标修改走 Task 修订，在下一次已有推理前观察；结束后修改创建新 Task / 新 Work。不能把每次 behavior 的结束都解释成整个 Work 已完成：当前 libopendan 按 `end_condition` 决定 Session 收尾，子 context 的结束先返回调用方。

WorkSession 与 Workspace 分离：Session 承载一次任务，Workspace 承载持续可修改的状态。新 Session 可以按需要读取产物和已有记录，但不必原样继承上一个 Session 的完整消息序列。

### 两种 Loop 的 report：显式结果提交

**传统 function_call Loop 通过显式调用 Session 层的 `report` 工具提交报告和产物，assistant 正文保持自由。** 报告可以是阶段性结果，也可以是最终交付；可选的 `is_end` 表达结束意图。这与 `finish()` 不同：后者只让当前 LLMContext 平滑停止，结束报告才提交完成意图，由宿主按调用关系和 Session 策略裁决。宿主不从普通 assistant 正文里提取控制指令，也不要求正文符合统一 schema。报告的归属、展示与产物投递的分层见 [Agent Actions](<Agent Actions.md>) 与 [Agent Message](<Agent Message.md>)。

XML Behavior Loop 按 2026-10-10 定稿规则表达同一语义：

| XML 决策 | 含义 |
|---|---|
| `<report>阶段性发现</report>` 或 `end="false"` | 提交 / 更新报告，继续执行，不请求结束 |
| `<report end="true">完成说明</report>` | 提交最终报告并请求结束；同一决策不得有 actions |
| 普通 report + `<next_behavior>CHECK</next_behavior>` | 更新报告并按原语义调度 CHECK |
| 普通 report + `<next_behavior>WAIT_USER_MSG</next_behavior>` | 更新报告并按原语义请求等待输入 |

`next_behavior` 保留切换 / 等待职责，不再输出 END，也不再由 xllm 为 report-only 合成 done。动作先在前面的决策执行并观察结果，最后另行提交结束报告：

```text
[assistant:决策 actions=[write_file]]                  @Step0 / Round1
[user:动作执行结果]
[assistant:<response><report end="true"><![CDATA[完成说明]]></report></response>]
                                                       @Step1 / Round2
  → 接受报告与结束意图；无需再推理生成确认
  → 子 context：结果返回调用方，原 Turn 继续
  → 顶层 context：宿主按 Session 策略完成 Turn / Session
```

结束报告与 actions 同现是协议冲突，应反馈纠错，不能先执行动作或先更新报告。`end=true` 与非空 next_behavior、原生 tool_calls / sendmsg 同现也拒绝；空 actions 允许。重复 report / end / next_behavior、非法布尔值、空最终报告和无行为的空决策进入纠错。

工具参数：`report`（自由文本 / Markdown）、`artifacts`（可选，显式选择的产物引用，宿主校验并保存稳定引用）、`result`（可选 JSON，业务自定的机器可读结果）、`is_end`（可选 bool，缺省 false）。

当前 `EndConditionType` 只有 `LlmDeclaresDone`、`OutputSchema`、`MaxTurns`；function_call 的 `Done` 会按该配置收尾，默认 `LlmDeclaresDone` 会结束 Session。`session.policy.completion` 缺省 `natural` 保留原结束条件；`explicit_report` 要求获准结束报告才正常关闭 Session。单次输出型 WorkSession 可以继续使用现有结束条件，不要求所有 Session 增加这次调用。

可交互 Session 采用显式完成策略后的示例（不用于 Work 的需求确认或终态 reopen）：

```text
Turn1：[user:先给我方案] … [assistant:方案，请确认] → Done → Turn1 completed，Session 等待
Turn2：[user:按方案执行] … [assistant:tool_calls(执行)] [tool:执行结果]
       [assistant:tool_call(report, {report:阶段性发现, artifacts:[…]})]
       [tool:report 已记录] → 继续执行，不结束 Turn / Session
       … [assistant:tool_call(report, {report:完成说明, artifacts:[…], is_end:true})]
       [tool:report 已接受] → 宿主校验并持久化 → 生成最终交付消息并收尾，无额外推理
       → Turn2 completed，Session finished
```

- 普通 `report` 只登记报告与产物，不截断同批其它调用，也不等同于向用户发送消息；接收方、展示与产物投递由宿主决定。
- 接受 `is_end=true` 时登记独立、持久化的完成意图：工具结果配对、快照和 worklog 提交之后由宿主关闭 Turn / Session，不再推理一次去生成报告或确认。此后同批未派发的调用明确记为未执行，已经执行的不回滚；参数、产物或权限校验拒绝时返回工具错误，Session 保持可修正状态。
- 最终交付由 Session 根据已提交的 report 与产物引用**机械生成**一条 assistant message（关联来源 `call_id`，不新增 Round），与 report 文件、UI 展示使用同一份结果；原始工具调用 / 回执保留，普通阶段性 report 不生成交付消息。
- 提交可重做：重复的 `call_id` / 提交请求不重复登记产物、生成最终消息或关闭 Turn；历史重建不重执行 `report`，也不把同一份最终交付渲染两遍。

宿主工具属于 Session 控制面：LLMContext 不解释 report / result 的业务内容，独立 xllm 不暴露此工具；独立 xllm 的 XML Behavior 仍需遵守新的显式 end 规则。子 context 的报告默认交给父 context，不得因为继承了工具表而关闭整个 Session。XML 的 end 和工具 is_end 均缺省 false。显式策略下，有输入队列的 Session 遇普通 Done 会完成 Turn 并等待；无输入队列的 WorkSession 漏报会失败。完成提交要求活动 task 与子 Work 已收敛；它不表示人工验收通过。XML 用与 report 并列的 `<artifacts>["relative/file"]</artifacts>`、`<result>{...}</result>` 表达同一提交。

提交 journal 写在 `run.json.host.extra.reports`，身份关联 run 与 tool call_id / behavior step_index。`state.latest_report` 与 `state.final_report` 分开保存；文件副本位于 `.opendan_agent_session/reports/{id}/{index}-{sha256}`。`ReportDelivery` worklog 记录来源、提交和机械生成的 assistant 消息；恢复复用该身份，结果不重执行、不重复展示。

## 打断、平滑结束与恢复

控制面作用于运行中的推理或工具执行；停止一次 Round、暂停一个 run 和结束 Turn 是不同层的决定。当前接口为 `LLMContextInterruptHandle.interrupt(reason)` / `finish(reason)`，详见 [LLM Context 设计](<LLM Context 设计.md>)。

### Interrupt：打断后保留恢复点

推理中收到 `interrupt`：当前推理立即中止，半截 token / 半截 tool call 不进入 accumulated。返回 `Interrupted`，保留发起这次推理前的快照：

```text
打断前：[system] [user:输入] [assistant:tool_calls(a)] [tool:a结果] → Round2 推理中
打断后：[system] [user:输入] [assistant:tool_calls(a)] [tool:a结果] → Interrupted
恢复：  ResumeFromMidRun → 再次推理；a 不重跑，当前 Turn 继续
```

若打断发生在工具执行中，已得到的结果保留；当前工具记为 `Cancelled`，同批未执行的调用配为 `Unresolved`，恢复后不重放。无法确认副作用的工具用 `effect_unknown=true` 表达不确定性。Behavior action 被打断时，同样保留决策和结果记录。

在发起推理前就收到打断，不增加 Round；确实发起过但被打断的推理仍是一次 Round。恢复重试是下一次推理尝试，不是新的 Turn。

### Finish：平滑结束，不追加一次收尾推理

`finish` 不再发起新的推理和工具调用。正在推理则等它完成，其中新产生的工具调用不派发而配为 `Cancelled{effect_unknown:false}`：

```text
[system] [user:输入]
[assistant:文本 + tool_calls(a,b)] [tool:a=Cancelled] [tool:b=Cancelled] → Settled
```

不再额外请求 assistant 生成 ack。工具执行中，支持取消的工具取消；不支持的最多等待 `finish_grace_ms`，超时升级为打断。`Settled` 保留可继续的快照；是否以 `stopped` 结束 Turn / Session，由宿主的 stop 策略决定。

### 旧 Runtime 的 Graceful / Discard

原文的两种策略属于旧 `src/frame/opendan` Runtime，不是当前 `interrupt` / `finish` 的别名：

| 旧策略 | 对 pending 工具及历史的处理 | 与当前接口的区别 |
|---|---|---|
| `InterruptMode::Graceful` | 给 pending 调用补 `Cancelled`，把 `max_tool_iterations` 设为 0，resume 后尝试生成收尾文本 | 可能追加收尾推理；当前 `finish` 不追加 |
| `InterruptMode::Discard` | 删除持有未完成 tool call 的末尾 assistant 消息及其后消息，清空挂起状态 | 会裁剪历史；当前 `interrupt` 保留已配对的调用和结果 |

```text
旧 Graceful：[user:输入] [assistant:tool_calls] [tool:Cancelled] [assistant:收尾文本]
旧 Discard： [user:输入] [assistant:较早已完成的tool_calls] [tool:结果]
```

Discard 后消息列表里缺少被裁掉的内容，不代表工具副作用被回滚，也不代表审计记录中没有中断痕迹。末尾是工具结果更不等于 Turn 已正常交付。

### UI 暂停后补充信息：两种 history 策略（H3 暂缓）

**2026-10-10 决定暂不实施 H3；以下 A / B 仅保留为 UI 交互候选，不纳入当前实现与验收，也不是强制 reopen 方案。** Work 不使用这两种方式续聊：运行中的同目标修改走 Task 修订，终态后的修改创建新 Task / 新 Work。未终态执行的检查点恢复及受控暂停 / 取消仍属于 Runtime 能力，不因 H3 暂缓而删除。

先区分按钮意图与现有协议。当前 libopendan 的 `ControlCommand::Stop` 会将 Turn 记为 `stopped`、Session 记为 `Finished`；不能直接在这个 Session 里追加信息继续。A / B 都要求 UI Session 始终尚未 Finished。以后若有“先停一下，我要补充条件”的明确需求，再设计独立的暂停 / 停止本次交互语义，不能直接复用终止 Session 的命令。

**方案 A：保留配对历史，继续同一 Turn。** 用 `finish` 平滑停止，必要时 `interrupt`；保存已有 response、工具结果和未执行标记，保持当前 run / Turn 打开，等待补充输入：

```text
停止前：[system] [user:做 A] [assistant:tool_calls(a,b)] [tool:a=Success] → b 执行中
停止后：[system] [user:做 A] [assistant:tool_calls(a,b)] [tool:a=Success] [tool:b=Cancelled]
补充后：… [user:先不要做 A，改为 B；保留 a 已完成的结果] → 下一 Round，仍属原 Turn
```

若只是在推理中打断，尚未提交的半截 response 不入历史；不能为了保持交替而补一条“任务完成”的 assistant。Behavior Loop 中，补充输入在合法 Step 边界并入热 Step 的结果消息；若仍有进行中 Step / 工具批次，须先完成结果配对，再注入，不允许插进 tool call 与 result 之间。

**方案 B：放弃当前交互，整理历史后开启新 Turn。** 先把旧 Turn 以 `stopped` 收尾，但 Session 保持可接收输入；宿主显式裁剪未提交尾段或重建输入，为补充信息创建新 run：

```text
旧 run / Turn1：… [tool:a=Success] [tool:b=Cancelled] → Turn1 stopped；完整记录留在 worklog
新 run / Turn2：[system] [user:<session_history> 已完成的 a、b 的取消/副作用状态、停止原因]
                [user:改为 B] → 下一 Round
```

这可以实现 Discard 风格的重新开始，但不能把整个已发生的工具批次删除后假装什么都没做。已确认的副作用与 `effect_unknown` 必须在新输入中可见，裁剪不改写审计记录。新建 run 也不意味着重跑旧工具。

| 维度 | A：保留上下文续跑 | B：结束本次交互后重建 |
|---|---|---|
| Turn / run | 原 Turn、原 run | 旧 Turn stopped，新 Turn、新 run |
| LLM history | 保留配对后的消息，追加补充输入 | 从完整记录重建摘要 / 选定历史，再加补充输入 |
| 前缀缓存 | 尽量复用已保留的前缀 | 重建处之后需要重新计算 |
| 适用 | 补充条件、纠正方向 | 明确放弃本次交互，重新开始 |

原“普通 Stop 默认 A、重新开始用 B”的建议不再作为本轮实施方向；现有终止 Session 的 Stop 语义保留。以后重新评估 H3 时，再冻结 UI 默认行为、输入持久化边界及恢复规则，确保停止期间到达的消息不丢失、不抢先触发推理。Agent Homepage / BuckyOS CLI 的后续工作入口也应创建新 Work，本轮不提供强制 reopen，详见实施清单 H3。

## 压缩 History Message

历史压缩是宿主在明确边界上的状态改写，不是每次 Round / Step 渲染时自动降级旧消息。已发送的消息前缀在正常推进时保持稳定；压缩只影响被改写位置及其后的缓存复用，不能笼统理解为所有 KV Cache 必然失效。

### 机械压缩与 StepRecord 摘要

- Tool call / result 应保持配对。可以按工具结果协议缩短结果；需要删除时成对处理，不能留下孤立的 tool result 或未回答的调用。
- 已完成的多个 Step 可以由宿主显式整理为摘要，保留决策、观察、report 和必要的动作结果。
- 当前 behavior 的普通 Step 默认保持完整 `(assistant, user)` pair，不会仅因变旧就自动改为 compact；其它 behavior 的继承记录和显式 summary 渲染进 `<<step_history>>`。
- Step 完成时丢弃 inner transcript、改用 StepRecord，是 Step 的记录边界，不是把已沉淀的旧 Step 动态压缩。

压缩策略可以减少 action result 的细节，但渲染器不能在后续 Round 中自行改写先前历史。具体结果协议见 [Agent Tool Result Protocol](agent_tool/agent_tool_result_protocol.md)，状态改写约束见 [append-only history](llm_context_append_only_history.md)。

### 在 Outcome 边界重写历史并继续当前 Turn

```text
压缩前（function_call，同一 run / Turn）：
[system] [user:历史与输入] [assistant:tool_calls(a)] [tool:a结果]
... [assistant:tool_calls(n)] [tool:n结果] → ContextLimitReached

宿主整理后：
[system] [user:<session_history> 摘要与保留的最近记录]
    → ResumeFill::RewrittenHistory → 继续推理
```

function_call 使用 `RewrittenHistory`；Behavior Loop 使用 `RewrittenSteps`，进行中 Step 的 inner transcript 保留，Step / action 编号继续。libopendan 会先把已完成但未写出的历史 flush 到 worklog，压缩 summary，发布新的历史快照，再恢复执行。

这不是新 Turn，也不会重新执行已完成的工具。Session worklog 保持只追加，压缩修改的是供推理使用的历史表示。Turn 完成后，也可以压缩 Session 历史供下一个 run 使用。

## Context 调度：由目标 behavior 决定

AgentSession 管理多个 context / run，同一时刻只推进一个当前 run。模型用 `next_behavior=B` 表达交接目标，宿主读取 **B 的进入模式**决定如何调度；Session 提供调度器和共享状态，不再用 Session 级 `switch_mode` 统一覆盖所有目标。

| 目标进入模式 | system prompt | 历史来源 | 再次进入与完成 |
|---|---|---|---|
| `SWITCH_CONTEXT` | 目标 context 自己的 system | 目标自己的历史；首次创建时由宿主装配初始输入 | 恢复同一目标 run；通过显式切换离开 |
| `create-sub-context` | 使用子任务 / 目标 behavior 的 system | 子任务参数及显式选择的共享状态 / 历史摘要 | 每次新建子 run；完成后交回父 context |
| `fork` | 保留父 system | 完整复制父在分叉点的有效历史 | 每次新建分支；完成后只把结果交回父 context |

配置拼写为 `extensions.opendan.behaviors.<B>.mode = switch_context | create_sub_context | fork`（libopendan `ContextMode`）；旧 `ProcessMode::Fork / Independent` 与 Session 级的 `process_modes` 表已删除。`SWITCH_CONTEXT` 用于目标 behavior 间交接；两种子 context 还可以由工具调用触发，function_call Loop 不需要借用 `next_behavior`。

### 普通切换：废弃的模式

“更换 system prompt，但沿用原 context 的历史继续推理”已决定废弃，不再作为默认切换或缺省回退：

```text
废弃： [system:DO] [DO 原历史] → [system:CHECK] [DO 原历史] [user:交接输入]
替代：保存 DO context → 进入 CHECK 自己的 context（SWITCH_CONTEXT）
      或以 CHECK system 新建子任务 context（create-sub-context），完成后返回 DO
```

libopendan 已删除这条调度路径（原 `handle_context_outcome` 只改 `behavior_name`、不换 system / 工具），“run 中途换成目标配置”的实施项也已取消。目标 behavior 的进入模式写在 `extensions.opendan.behaviors.<name>.mode`；表在推进开始时校验，交接到没有进入模式的 behavior 是配置错误（Turn 以 `failed` 结束，子 context 内则作为失败结果交回调用方），不会回退为 normal。旧 opendan Runtime 的同 run 替换配置是另一套实现，不是保留普通切换的理由。

### SWITCH_CONTEXT：在多个各自保有历史的 context 间切换

```text
run_DO / system_DO：      DO steps → next_behavior=CHECK → 保存并挂起
run_CHECK / system_CHECK：新建或恢复 → on_behavior_switch → CHECK steps → next_behavior=DO
run_DO / system_DO：      恢复原快照 → on_behavior_switch → 更多 DO steps → next_behavior=CHECK
run_CHECK / system_CHECK：恢复原快照 → on_behavior_switch → 更多 CHECK steps → END
```

首次进入 CHECK：

```text
[system:CHECK]
[user:CHECK 的初始历史 / Session 共享状态摘要（按配置选取）]
[user:<session_input hook=on_behavior_switch> 当前任务状态、交接原因]
    → CHECK 的第一个 Step
```

再次进入 CHECK：

```text
[system:CHECK] [CHECK 原有输入]
[assistant:CHECK Step0决策] [user:CHECK Step0结果]
[assistant:CHECK Step1 next_behavior=DO]
[user:该 Step 的结果 + on_behavior_switch 构造的最新共享状态]
    → CHECK Step2；沿用 run_CHECK，当前 Turn 继续
```

交接输入由 Session 根据共享状态构造，不必搬运 DO 的全部历史。共享状态不会自动进入 LLM：宿主仍须把所需事实渲染为输入或提供读取工具。context 自己的 system 和既有历史随恢复保持一致，不在每次切换时改写成另一套配置。

新建目标 context 的历史装配范围由目标的 `inherit` 决定：缺省 `none`，只有 system 与交接输入；`recent_dialogue` 才带入宿主渲染的 `<session_history>`（摘要 + 近期 worklog 记录，是筛选视图）。其它 context 的快照与原始历史从不自动并进目标 context；恢复已有目标时只用它自己的快照。

切换不完成 Turn。`<report end="true">` 由 Session 根据执行位置与完成策略裁决：顶层 context 完成当前 Turn，并按策略关闭 Session；子 context 完成后返回调用方。SWITCH_CONTEXT 不隐含调用关系，结束时不会自动返回上一 behavior。

## create-sub-context：更换 system，构造较短的子任务上下文

这是图解中的 `create-sub`。子 context 使用自己的 system、工具和任务输入，可以选择性带入父的摘要 / StepRecord，但不要求继承完整历史；常用于 Behavior Loop 的 PLAN → DO。子任务参数应说明目标和输出要求，避免靠完整父历史猜测任务。

```text
PLAN / system_PLAN
  ├─ create-sub-context(DO, 任务1) → DO / system_DO → report → 返回 PLAN
  ├─ create-sub-context(DO, 任务2) → 新 DO / system_DO → report → 返回 PLAN
  └─ report end=true → Session 判定 Turn / Session 完成
```

子 context 的输入形状：

```text
[system:DO 的任务提示词]
[user:子任务参数 + 必要的共享状态]
[user:选择的父历史摘要 / <<step_history>>（可选）]
    → 子 context 的 Round / Step → report
```

继承粒度是显式的输入构造策略：

| 粒度 | 内容 | 适用 |
|---|---|---|
| None | 子 system + 任务参数 | 独立子任务，优先保持输入简洁 |
| 最近对话 / 摘要 | 由宿主筛选并标明来源的事实 | 路由、意图判断等小任务 |
| StepRecord | 已完成的决策及必要的动作结果 | 需要理解父的任务分解 |

这些历史作为子任务的参考材料。即使选择完整 StepRecord，只要换了 system，就仍称为 create-sub-context。父在工具调用中途挂起时，进行中的 `action_step` 不能伪装成已完成 Step；已执行动作的结果如对子任务有用，应显式作为输入材料提供。

实现上由 `llm_context::derive_child(parent, child_request, InheritHistory)` 构造：`None` 只用子请求，`Material` 接收宿主渲染的材料，`Steps` 复制父已完成的 steps / summaries（要求双方都是 behavior loop）。libopendan 的 `inherit: none | recent_dialogue | steps` 对应这三种粒度。旧代码里名为 `fork`、实际“换 behavior 名并复制父 steps”的做法属于这里的 `steps` 继承，不是 fork。

## fork：保留 system 与完整有效历史的子分支

这是图解中的 `fork`。父 system 和分叉点之前的有效消息历史保持原样，在此基础上追加子任务输入；可按子任务配置工具，但改变工具定义仍可能影响缓存命中。

若目标声明了不同的 system，配置校验应要求选择 create-sub-context，不能在 fork 中静默替换。子任务的行为身份与继承历史的渲染身份也需分开，避免仅因目标名称变化就重新组织父消息。

```text
父： [system:S] [历史摘要] [user:原任务] [assistant:call(a)] [tool:a结果] …
子： [system:S] [历史摘要] [user:原任务] [assistant:call(a)] [tool:a结果] …
     [user:分支任务输入] → 多个 Round / Step → 分支结果
父： 保留原历史，只接收分支结果；子新增的推理历史不合并进父主干
```

“完整”指**分叉点的有效 LLM history**，包含当时已有的摘要、多模态内容和成对的工具消息；不要求恢复已经压缩掉的原始历史，也不包含尚未提交的半截模型输出。只取最近几条文本、替换 system 或把所有历史重渲染为一条摘要，都属于 create-sub-context 的输入构造，不能称为完整历史 fork。

Behavior 模式还必须保留能产生相同历史前缀的 request、steps、热 Step、history inputs 等状态；仅复制 `steps` 再按另一个 behavior 重新渲染，不能保证完整继承。宿主的 child run 身份、预算和输入 receipt 需独立初始化，不复制父的运行所有权或让子过程接管父的待执行工具。

### 工具触发时的分叉点

如果触发 fork 的 tool call 尚未得到结果，不能把它原样带进子请求后立即追加 user 输入，否则 tool call / result 不配对。建议把该工具批次之前的最后一个完整前缀作为明确分叉点：

```text
H = [system:S] + 分叉点之前的完整、已配对历史
父：H [assistant:tool_calls(fork_task, other)] → PendingTool，保存整个未完成批次
子：H [user:fork_task 的参数] → 分支结果
父：H [assistant:tool_calls(fork_task, other)] [tool:fork_task结果] [tool:other结果] → 继续
```

触发批次不属于子分支继承前缀，父快照完整保留它；同批先前已完成调用的结果仍在父和 worklog 中，子若需要则作为明确输入附带。分叉点必须记录，不能任意丢掉较早历史却仍声称“完整继承”。这一边界由 `llm_context::fork_snapshot` 处理并随派生结果返回（`ForkPoint`）：function call 取触发批次的 assistant 消息之前的前缀，behavior 取进行中 Step 之前的已完成 steps / 热 Step；前缀里仍有未配对调用时拒绝 fork。不能通过伪造“已完成”的 assistant / tool 回答填补。

## 两种子 context 的调用、返回与历史归属

创建方式决定 system 与历史如何构造；触发方式决定结果从哪里返回，两者独立：

| 触发方式 | 父 context 的停止点 | 子结果返回 |
|---|---|---|
| Behavior 交接到子任务 | 完整 Step 的 `next_behavior` | `<process_result>` 并入父 Step 的结果 / 交接输入 |
| function_call 或 behavior action 调用子任务工具 | `PendingTool`，保存未完成批次 / action | `ResumeFill::ToolResults` 按 `call_id` 配对，继续未派发的调用 |

Behavior 交接返回示例：

```text
父 PLAN：… [assistant:next_behavior=DO]
          [user:该 Step 的结果 + <session_input hook=on_behavior_switch>
                 <process_result behavior=DO>子结果</process_result>]
          → PLAN 的下一个 Step，仍是原 Turn
```

工具返回示例对 fork / create-sub-context 都成立：

```text
父：… [assistant:tool_call(child_task, 参数)]
子：按所选方式构造 → 推理 → 结果
父：… [assistant:tool_call(child_task, 参数)] [tool:子结果] → 下一 Round，原 Turn 继续
```

子 run 需要保存到 `runs/`，支持停止、挂起和崩溃恢复；不能依赖工具函数里的临时内存上下文。子新增 steps / messages 不进入父 context 的推理主干，但会进入 **Session worklog**。图中的“不会进入 parent session history”在此指父 context 的 Message List，不指 Session 审计记录；宿主后来重建历史时还须按记录归属筛选，避免又把子过程完整内容塞回父输入。

子任务无论以什么结束都只返回父 context，不关闭父 Turn，返回时带状态：正常完成 `ok`；`WAIT_USER_MSG` 返回 `needs_user_input`（子 context 不直接等待用户，也不消费父的输入队列，由父去询问）；不可重试错误、预算耗尽、交接到没有进入模式的 behavior 返回 `failed`。工具触发的调用由 `call_behavior({behavior, task})` 发起，父挂起在 `PendingTool`（task id `subctx:<call_id>`），子结果按 `call_id` 回填后续派同批余下的调用。子 context 可以再调用子 context（最多 4 层），但不做 SWITCH_CONTEXT：交接到 `switch_context` 目标等同于返回。

### 三种方式的边界对照

| 维度 | SWITCH_CONTEXT | create-sub-context | fork |
|---|---|---|---|
| system | 目标自己的 | 新子任务的 | 与父相同 |
| 父历史 | 不自动复制到已有目标 | 显式选择 / 摘要，可不继承 | 分叉点的完整有效历史 |
| 再次进入 | 恢复目标的原 run | 新建子 run | 新建子分支 |
| 完成 | Session 判定结束或下一次显式切换 | 结果交回父 | 结果交回父 |
| Turn | 切换继续当前 Turn | 调用 / 返回继续当前 Turn | 调用 / 返回继续当前 Turn |

三者都不自动隔离文件系统、Session 状态、worklog 或对外消息。需要 dry-run 时由工具与 Runtime 限制副作用；同一 Session 仍串行推进，任务并行属于子 Session 能力。缓存复用取决于实际请求前缀与模型 / 工具定义，不能仅凭模式名称保证命中。
