# Context 调度支持 TODO：SWITCH_CONTEXT、create-sub-context 与 fork

日期：2026-10-03

实施更新：2026-10-10。H4 / H5 已落地，实施细节与验证见 §9。H3 / V8 继续暂缓，不适用于 Work 生命周期；终态 Work 不 reopen，后续修改创建新 Task / Work。

状态：S1 / S3 / S4 / S5、H1 / H2 的既有实现见 §8；H4 两种 Loop 的显式报告、持久完成意图与崩溃恢复，以及 H5 的协议、提示词、模板、schema 和 fixtures 已同步。本文 §8 保留 2026-10-03 的历史记录；当前完成规则以 H4 / §9 及 [Session Directory Protocol §7.1](<../doc/opendan/protocol/Session Directory Protocol.md>) 为准。
## 1. 本次确定的语义与旧计划调整

| 事项 | 本次结论 | 对旧计划的影响 |
|---|---|---|
| 模式归属 | 进入模式由目标 behavior 决定；Session 负责执行调度 | 取消 Session 级统一 switch_mode，不允许 normal 缺省回退 |
| 普通切换 | 废弃“替换 system、沿用原 context 历史”的方式 | 取消旧 T1 `switch_in_place` 与 S2“run 中途替换 config” |
| SWITCH_CONTEXT | 目标有自己的 system、历史和 run；再次进入恢复原快照 | 原 independent 的设计名称统一为 SWITCH_CONTEXT；|
| create-sub-context | 新 system + 子任务输入，可显式选择父历史 | 旧 T2 中“使用 B 的 system、继承父 steps”的方案归入此类 |
| fork | 保留父 system 与分叉点的完整有效历史，再追加分支任务 | 需独立的完整历史派生原语，不能等同于旧代码只复制 steps 的 fork |
| report | 传统 Loop 通过显式工具提交，assistant 正文保持自由；Behavior 用 `<report end="true">` 请求结束，普通 `<report>` 只更新报告 | 两种 Loop 对齐报告与结束意图；`next_behavior` 保留切换 / 等待语义，不再用 END 结束；已实施，见 H4 / §9 |
| UI Stop 后补充输入 | H3 暂缓；A / B 仅保留为未终态 UI Session 的候选交互方案 | 移出当前实现与验收范围；Work 执行中修改走 Task 修订，终态后创建新 Task / 新 Work，不 reopen |

[xAgent.md](../doc/opendan/xAgent.md) §3.3–§3.7、C14、E19/E20 已同步新版调度语义。旧 T1、旧 fork 定义和 S2 不再作为实现依据；保留编号便于追踪，映射如下：

| 旧编号 | 新处理 |
|---|---|
| T0 / T5 / T6 | 续跑、显式历史重写、Turn 边界仍成立 |
| T1 | 废弃；不能在同一 run 中替换为另一 behavior 的 system / 配置 |
| T2 | 按构造方式拆分为 create-sub-context 或新版 fork；旧“换 B 的 system”属于前者 |
| T3 | SWITCH_CONTEXT |
| T4 | 工具触发的子调用协议；可承载两种子 context，按 call_id 返回 ToolResults |
| S2 | 取消，不再是前置任务，不应标成“已实现” |
| E19 | 改验 SWITCH_CONTEXT 的独立配置 / 历史及 hosted 交接；去掉“同一 run 换配置”验收 |
| E20 | 保留工具子调用、挂起 / 恢复 / 幂等返回，增加两种派生方式与合法历史边界 |

## 2. 范围与分层

- 底层先提供历史派生、快照一致性和 hosted run 交接原语，主要涉及 `llm_context` 与 `agent_tool::xllm`；宿主随后接入，避免自行复制快照内部字段。
- behavior 配置、目标模式选择、process stack、Turn / Session 边界、输入消费和子结果路由由 libopendan / xagent 负责。llm_context 不感知 behavior 注册表或 Session 是否完成 objective。
- SWITCH_CONTEXT 恢复目标原 run 的配置；create-sub-context 和 fork 为子 run 写独立配置。无需为这三种模式增加“同一 run 中途换目标配置”的 API。
- 源码与相关文档已按清单实施，既有调度工作见 §8，本轮 H4 / H5 及验收见 §9。
- 持久结构需要变化时按 beta 2.2 规则升版并拒绝旧格式，不增加 normal / independent 等旧配置的兼容回退。

## 3. 实施前的源码与缺口（2026-10-03 历史核对；除暂缓的 H3 外已由 §8 / §9 解决）

| 入口 | 已有能力 | 缺口 |
|---|---|---|
| `llm_context/src/snapshot_overrides.rs` | `build_fresh`、`rebuild_with_inherit`、`RequestOverrides` | `rebuild_with_inherit` 拒绝 suspended / mid-batch 父快照；允许替换 system，本身不能保证新版 fork 语义；尚无独立 `derive_child` / `fork_context` |
| `lib_opendan/src/runner/assembler.rs::process_mode` | 按目标名读取 `extensions.opendan.process_modes.<behavior>` | 缺省仍返回 normal 路径；尚未形成三种模式的目标 behavior 配置与校验 |
| `runner/outcome.rs::handle_context_outcome` | 普通切换改 behavior_name；Fork / Independent 入栈 | 普通分支仍在，且 system / 工具未更换；要删除此路径，而非补齐换配置 |
| `runner/live.rs::new_run_context` | 旧 Fork 为 behavior 子 run 复制 steps / last_step / summaries 和编号 | 不保证完整 request / transcript / history_inputs / system 前缀；function_call 全历史派生缺失 |
| `runner/live.rs::suspend_run` | Independent 可恢复目标挂起 run | 可作为 SWITCH_CONTEXT 调度基础；首次创建装配共享 worklog，需要明确历史范围 |
| `agent_tool/src/xllm.rs`、`runner/reconcile.rs` | 保存 run / snapshot，接手执行、恢复 Session | hosted 的 `Done{next_behavior=B}` 仍可能记 Completed；reconcile 把 Switch 改成 Done，目标交接丢失（旧 G8） |
| `runner/inputs.rs`、`runner/drive.rs::stop_session` | `ControlCommand::Stop` 终止 Turn 与 Session | 尚无 H3 的 UI 暂停 / 补充输入命令；2026-10-10 已决定暂缓，不作为当前必补缺口 |
| `protocol/config.rs::EndConditionType` | LlmDeclaresDone / OutputSchema / MaxTurns | 没有通过 report 提交结果与结束意图的宿主工具及显式完成策略 |

旧 `RequestOverrides.user_messages` 会清空 steps / summaries / history_inputs，适合构造新输入，不能用于声称“完整继承”的 fork。现有 `rebuild_with_inherit` 的前置条件应保留，不为子派生而放宽父 run 的恢复校验。

## 4. llm_context / xllm 原语

### S1：create-sub-context 的历史派生（P0，替换原 S1）

- [x] 提供纯派生接口（可沿用 `derive_child` 名称），输入为父快照、显式历史选择和独立 child request；返回子快照与继承边界信息。父可处于正常、PendingTool 或批次中途，派生不得修改父快照。
- [x] `None`：只使用子 system / 参数；`RecentDialogue`：由宿主选取近期对话并渲染；`Steps`：选择父已完成 steps / summaries。名称为设计描述，最终参数类型随实现定稿。
- [x] `RecentDialogue` 的筛选、来源标记和模板归宿主；只含文本的结果必须明确是筛选视图，不能丢掉工具 / 多模态内容后称为完整历史。
- [x] `Steps` 若作为结构化 StepRecord 继承，要求兼容的 behavior parser / renderer；跨 loop mode 时改用显式渲染的输入材料。进行中 `action_step` 不当作已完成记录复制。
- [x] 子快照不带父的 suspension、待派发调用、abort 状态或输入消费 receipt；子工具额度、usage / 错误计数按 child request 独立初始化。继承 step / action 编号与新增部分的边界明确返回给宿主，以免重复写 worklog。
- [x] 支持子 system、模型和工具配置独立于父。仅需新输入时复用 `build_fresh`；确需结构化继承才走派生接口，不为 None 新增一套运行循环。

### S2：取消 run 中途替换目标 config

**本项取消。** 原 `RunHandle::replace_config` 计划服务于已废弃的普通切换，不再实现，也不再列为 E19 的前置。新 context 写自己的有效配置；SWITCH_CONTEXT 恢复已有目标的配置。其它用途若确需改 run 配置，应单独说明，不能借本项恢复 normal switch。

### S3：hosted run 在交接点让出（P0，保留并更新）

- [x] hosted run 遇到指向其它 behavior 的 `Done` 时，保存非终态交接状态、目标与必要快照，不记为已完成 Turn。独立 xllm 的行为不因此变成 Session 调度器。
- [x] 优先复用 `Paused` + 持久化的交接原因 / 目标；仅在现有状态不能无歧义表达时考虑新增状态。xllm 记录交接事实，目标模式由宿主读取配置决定。
- [x] repeated `xllm --resume` 遇到待宿主处理的交接时仍停在原交接点，不重复推理或误完成；只有宿主提交了转移才可继续目标 run。
- [x] `reconcile` 不再把交接强制降为 Done；转移提交前 / 后崩溃都只能产生一次目标进入，保留原 Turn、已用预算和消费位置。
- [x] 子 run 正常完成仍可记录完成状态，由宿主按调用关系交回父；不能把每个 END 都判为 Session finished。

### S4：fork 的完整历史派生（P0，新增）

- [x] 提供单独的 fork 原语，强制父 system 保持相同，复制分叉点的完整有效 history，再追加子任务输入。不能通过 `RequestOverrides.system_messages` 换提示词；目标声明不同 system 时校验拒绝并提示使用 S1，不能静默改成另一种模式。
- [x] function_call 保留所有有效 AiMessage / AiContent（含工具配对、多模态和已有摘要），不是只复制 user / assistant 文本；behavior 保留 request、steps、热 Step、history_inputs 等构造相同历史前缀所需的状态，不能改 behavior 后把原历史自动降级为摘要。
- [x] “完整”只到记录的 fork point。工具触发时建议取触发批次之前最后一个合法前缀；父的当前批次、部分结果和待派发调用仍留在父快照中。禁止通过假 tool result 或固定 assistant 回答补洞。已执行同批兄弟调用的必要结果可作为新子任务输入附带，不能悄悄改写继承前缀。
- [x] 父在 Step 中途派生时，区分只读历史与工具续派所有权；父的 action_step / tool_batch 不能在子 run 重新执行。必要时拒绝无法取得合法分叉点的请求，不静默截取 recent history。
- [x] 派生结果返回继承消息 / Step 边界和 fork point，宿主只把子新增记录写入 worklog；child run 的 owner、host metadata、receipt、预算和统计独立。父恢复不重置其工具额度，Session 汇总子执行的实际用量。
- [x] 新工具集可以显式装配，但不承诺 KV Cache 命中。若更换 provider / loop mode 使历史内容无法无损继承，应拒绝该 fork 或由调用方明确选择 S1；不能悄悄转换角色或删内容。

### S5：合法边界与恢复验证（P0，复用现有能力）

- [x] SWITCH_CONTEXT 复用 `LLMContext::resume(..., ResumeFromMidRun)`；目标若等待工具结果，先走 `ToolResults`，若处于 context limit，先走相应历史重写。不能把任意挂起快照强行当 ResumeFromMidRun。
- [x] 保持 `inject` 的边界约束：未配对批次 / 进行中 Step 不能直接插入新 user 消息。S1 / S4 的只读派生能力不能变成放宽父注入校验的后门。
- [x] 复用 `finish` / `interrupt`、Cancelled / Unresolved 和现有快照配对语义供执行控制与 H4 使用；H3 暂缓不影响这些原语。仅补实际缺失的边界原语，llm_context 不内置 report 宿主工具或 Session / Turn 状态机。

## 5. 宿主配套工作

### H1：目标 behavior 配置与 SWITCH_CONTEXT（P0）

- [x] 冻结目标 behavior 的进入模式、system、工具、模型与历史装配策略；明确配置枚举和序列化拼写。目标缺失 / 模式非法在执行前报错，删除 normal 回退以及 Session 级模式覆盖。
- [x] SWITCH_CONTEXT 首次创建目标 context，之后恢复同一个挂起 run 的 system、工具、历史、编号和预算；交接输入由共享 Session 状态渲染。不能把 A 的原历史直接挂到新 system_B 下继续。
- [x] 记录当前 / 目标 run、模式、调用来源和交接 receipt；按“父已挂起 → 目标已创建 / 恢复 → 输入已提交”的阶段恢复，避免重复创建或重复消费。
- [x] 明确 `END` 的解释：普通目标按 Session 结束条件处理；子调用的完成返回调用方。切换与返回本身都延续 Turn。

### H2：两种子 context 的串行调用与返回（P0）

- [x] create-sub-context 调 S1，fork 调 S4；父入栈后只推进子 run，子 run 写入 `runs/`，不能在工具里隐式启动不可恢复的第二个执行循环。
- [x] 触发方式与创建模式分别记录：behavior 交接经 `process_result` 返回；工具调用经 `call_id` 对应的 `ToolResults` 返回。原 `ProcessMode::Fork` 需要拆分“构造语义”和“返回协议”，不能直接机械改名。
- [x] 工具子调用按现有 `Pending{task_id, until_ms}` 挂起。Session 持久登记 task_id → 子 run / 父 call_id，装配 `RunningTaskResolver` 查询 / 等待 / 取消。子 run id 预分配，崩溃后重做进入 / 返回均幂等。
- [x] 不再使用旧 `wait.source.kind = subrun`、`class` 等结构。[long-tool TODO §4](./llm-context-long-tool-todo.md#4-p1等待llm_context-的两种机械判断与-runningtaskresolver) 已将它们简化为不透明 task_id；接手方检查 resolver 能否解析等待对象。llm_context 不识别 task_id 前缀。
- [x] libopendan 为子调用开启 deferred 并接入等待 / 回填；不能只开启 `allow_deferred` 却仍在宿主返回 PendingTool 后无恢复路径。xllm 不具备所需 Session 工具 / resolver 时明确拒绝接手，不能将不可解析的子 run 当普通本地未知 task 丢弃。
- [x] 子任务 `Done` 返回结果，Error / Budget 作为失败结果交回父工具；`WAIT_USER_MSG` 的处理显式定义，建议工具子调用返回结构化 needs_user_input 让父询问用户。behavior 子调用是否直接等待、以及嵌套切换范围仍需定稿，不能沿用“所有 next_behavior 都立即返回”的假设。
- [x] 子新增 messages / steps 写入共享 worklog，但不合并到父快照；后续压缩和历史重建按 run / 调用关系筛选，父默认只带交接结果。保留完整子记录供追踪，继承的父记录不重复写入。
- [x] 栈深、嵌套和取消传播由 Session 统一校验；子 context 不能直接消费父输入队列或因继承控制工具而关闭父 Session。

### H3：Stop 后补充输入（暂缓，移出当前实现范围）

**2026-10-10 决定：暂不实施 H3，也不实施 finished Session 的强制 reopen。** H3 不适用于 Work 生命周期，不阻塞 H4 或 Work 的实现收敛。后续只有出现明确的 UI 暂停交互需求时，才重新评估 A / B 及其按钮和控制协议，不预设默认方案。

按 [OpenDAN Agent Session 架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>) §6、§12.12，Work 是一次有界任务执行，按终局结果与 Final Report 收尾：

| 情况 | 处理规则 |
|---|---|
| 执行中补充同目标修改 | UI / Goal 将授权修改追加为 Task 修订，Work 在下一次已有推理前通过半订阅观察；不接收普通 Message，不为修订额外开启 Turn 或中断当前工具 |
| 取消 Work | 终止执行，保存结果、副作用和终止原因；无法确认取消的外部工作如实记录 |
| Work 已结束后修改或重新尝试 | 创建新 Task / 新 Work，引用有效产物、已验证事实和必要 Report，不恢复旧 Work，也不默认继承全部旧 Context |
| 执行进程中断但 Work 尚未终结 | 从合法检查点恢复；这是执行恢复，不是终态 reopen |

原 H3 两种方案都以 Session **尚未 Finished** 为前提：A 保留原 Turn / run 等补充输入；B 结束当前 Turn，但同一 Session 保持开放，后续输入开启新 Turn。它们仅作为 UI 的候选方案保留在[消息示例](<../doc/llm_context/LLM Context Switch (histoiry samples).md>)中，不用于 Work 的重新执行。原“普通 Stop 默认 A”的建议不再作为本轮实施方向；现有终止 Session 的 Stop 语义保留。

Agent Homepage / BuckyOS CLI 即使提供“基于此结果继续工作”，也应创建后续 Work；本轮不增加强制 reopen 管理入口。未终态执行的中断、工具结果配对、受控恢复、取消和审计能力继续保留；暂停及父子级联的控制合同按架构设计另行收敛，不因 H3 暂缓而删除。

### H4：两种 Loop 的 report 显式结果提交（已实施）

本轮将原 `end_session` 建议改名为 `report`：报告既可以是阶段性结果，也可以是最终交付。传统 function_call Loop 不要求 assistant 正文满足统一 XML / JSON schema，也不从正文中提取控制指令；结构化结果由显式工具调用提交。Behavior Loop 继续使用 XML 决策协议，按 H4.1 将显式结束统一到 report。

#### H4.1：XML Behavior result 核心规则（2026-10-10 已实施）

1. `next_behavior` 保留目标 behavior 切换、`WAIT_USER_MSG` 等既有调度语义；不再用 `<next_behavior>END</next_behavior>` 表达正常结束，也不通过内部合成 `done` 隐式结束。
2. `<report>` 增加可选布尔属性 `end`。缺省或 `end="false"` 只提交 / 更新报告；`end="true"` 同时表达结束意图，与传统 Loop 的 `report({report: ..., is_end: true})` 对齐。报告正文仍是自由文本 / Markdown，不兼任控制指令。
3. `<report end="true">` 所在决策不能有 actions。先执行动作并观察结果，再在后续决策提交结束报告；不能保留旧的 `END + actions` 执行后直接收尾特例。冲突决策不应执行动作、更新报告或接受结束，而应反馈可修正的协议错误。

```xml
<!-- 阶段性报告：不请求结束 -->
<response>
  <report><![CDATA[已完成资料核对，继续验证实现。]]></report>
</response>

<!-- 切换：next_behavior 的调度语义保留 -->
<response>
  <report><![CDATA[实现已准备好，请进入检查。]]></report>
  <next_behavior>CHECK</next_behavior>
</response>

<!-- 最终报告：不带 actions，不再输出 next_behavior=END -->
<response>
  <report end="true"><![CDATA[检查完成，结果如下……]]></report>
</response>
```

`end="true"` 表达当前 context 的完成意图，不直接赋予关闭整个 Session 的权限：子 context 的报告返回调用方；顶层 context 由 Session 策略裁决 Turn / Session 的完成。它不等于向用户发送消息，也不等于人工验收通过。最终报告正文来自本次结束提交；宿主如何校验、持久化和投递仍按 H4.2 实施，不能继续让正常执行和接手恢复各自选择不同来源。

这三条同时约束 `XmlBehaviorParser` 与 xllm / libopendan 使用的 `XllmActionParser`，提示词、快照恢复和宿主判定必须一致。独立 xllm 的 Behavior 输出也需显式给出 `end="true"`，不再把缺少 end 的 report-only 当最终答案；这不意味着独立 xllm 暴露 Session 的 report 工具。XML 协议对照见 [Agent Actions §2.2](<../doc/llm_context/Agent Actions.md>)。

实施前后的行为对照（左列为旧实现）：

| 入口 | 旧实现 | 当前实现 |
|---|---|---|
| `XmlBehaviorParser` / `context_loop` | report-only 继续；END 可与成功 actions 同步收尾；尚不读取 report.end | 读取显式结束属性，删除 END 结束特例，校验结束与动作互斥 |
| `XllmActionParser` / xllm 提示词 | report-only 自动产生 `next_behavior=done`，提示词要求 report 仅用于最终结果 | 与上述规则一致，允许阶段性 report，最终报告显式 end=true |
| `handle_context_outcome` / `reconcile::derive_next` | 前者取终止 Step report 或响应原文，后者优先取 snapshot.last_report | 结束报告使用同一次提交，恢复不得替换为较早的阶段性报告或 XML 控制文本 |

补充边界已定稿并实施：

- end=true 与非空 next_behavior 同现拒绝；end=false 可与切换 / 等待共存。
- 空 actions 允许；实际 XML action、sendmsg、同一响应的 provider-native tool_calls 均与结束互斥，派发前校验。先前已执行并观察的 Round 不冲突。
- end 只接受 true / false；重复 report、非法 end、空结束报告拒绝。宿主阶段报告也拒绝空正文；不采用 last-one-wins。
- 无 report、无动作、无调度的空决策反馈可纠正协议错误；普通 function_call Done 按 Session 策略处理。
- response 同层 `<artifacts>["out.txt"]</artifacts>` 与 `<result>{任意JSON}</result>` 必须同次有 report；llm_context 仅透传，宿主校验文件与完成权限。

#### H4.2：宿主 report 工具与统一提交（已实施）

参数已定稿；`is_end` 表达可选结束意图：

| 参数 | 已实现形态 | 含义 |
|---|---|---|
| `report` | 自由文本 / Markdown 字符串 | 明确提交的报告正文，不固定报告内部格式 |
| `artifacts` | 可选的工作目录相对文件路径字符串列表 | 显式选择交付文件；宿主校验并保存稳定引用，不靠正文标签或目录扫描推断产物 |
| `result` | 可选 JSON 值 | 应用需要的机器可读结果；通用框架不固定业务 schema，具体 Session 可按需提供校验 |
| `is_end` | 可选 bool，缺省 false | false 只提交报告；true 同时请求结束，由宿主按当前调用关系和 Session 策略裁决 |

```text
report({report: "阶段性发现……", artifacts: [...]})
  → 记录报告，继续当前执行，不自动结束 Turn / Session
report({report: "完成说明……", artifacts: [...], is_end: true})
  → 校验并持久化 → 工具结果配对 → 宿主生成最终交付消息并收尾，无额外推理
```

- [x] 提供宿主工具 `report`，冻结到适用 Session 的工具配置中。独立 xllm 不默认暴露，llm_context 不解释 report / result 的业务内容。
- [x] 落实 H4.1：两套 parser、共享结果 / Step / 快照字段、提示词、xllm 接手、宿主及子返回同时识别 report 的结束意图；不把它重新编码为 next_behavior=END / done。更新旧 END 和隐式 report-only 的测试与示例，不新增旧终止语义的兼容回退。
- [x] 与 behavior 的 `<report>` 对齐报告的归属和最近报告更新语义：没有结束意图时只提交报告，不自动结束，也不等同于向用户发送消息；接收方、展示和产物投递由宿主决定。传统工具的 `is_end=true` 与 XML 的 `end="true"` 使用相同的宿主完成规则，`next_behavior` 继续负责切换 / 等待。
- [x] 增加可选的显式完成策略。该策略下普通 assistant `Done` 可以完成当前 Turn 并等待后续输入；只有获准的结束报告（工具或 XML）才正常关闭 Session。默认无输入队列的 WorkSession 必须另定漏报处理，不能无条件进入等待。单次输出型 Session 仍可按现有 end_condition 收尾，不要求所有 Agent 强制调用 report。
- [x] 按 context 调用关系确定报告归属与结束范围；子 context 的最终报告默认交给父 context，不能因继承 report 工具而关闭整个 Session。定义与未完成子调用、活动 task 和人工验收的关系，保持“执行完成”和“产物已验收”分开。
- [x] 接受 `is_end=true` 时登记独立、持久化的完成意图；工具结果配对和快照提交后由宿主完成 Turn / Session，不再推理一次来生成 report 或 ack。可复用平滑结束让 LLMContext 让出，不能把 Settled 一律算 stopped 或成功。
- [x] 仅在接受结束意图后停止派发同批余下工具，并逐项记录未执行、补齐结果配对；此前已经执行的调用不回滚。普通 report 不截断批次；参数、产物或权限校验拒绝时返回工具错误，保持可修正状态，不错误结束。
- [x] 最终交付时，Session 对话历史根据已提交的 report 和产物引用机械生成一条 assistant message，关联来源 call_id，不新增推理 Round；与 report 文件、UI 展示使用同一份结果。原始工具调用 / 回执保留，不伪造模型响应或为普通阶段性 report 重复生成交付消息。finished Session 拒绝继续输入；终态 Work 的后续修改创建新 Task / 新 Work，强制 reopen 不在本轮范围内。
- [x] 报告、产物引用、可选 result、完成意图、工具回执、派生 assistant message 与最终状态支持崩溃重做；重复 call_id / 提交请求不得重复登记产物、生成最终消息或关闭 Turn。历史重建和未完成提交的恢复不得重执行 report；构造模型输入时避免同时重复渲染同一份最终交付。

传统工具 report 被接受后跳过同批剩余工具的规则，与 XML 结束决策不得含 actions 是两个入口各自的批次规则；本次 XML 约束不自动改变传统工具批次的既定要求。XML 的报告身份需关联 run / Step，传统工具关联 run / call_id；持久身份、XML 表达和两种来源的交付投影已实施，见 §9。

公开参考：[OpenHands FinishTool](https://raw.githubusercontent.com/OpenHands/software-agent-sdk/main/openhands-sdk/openhands/sdk/tool/builtins/finish.py) 将最终消息放入调用参数；[Pydantic AI output functions](https://pydantic.dev/docs/ai/core-concepts/output/#output-functions) 接受参数后直接结束 run，校验失败可要求重试；[smolagents FinalAnswerTool](https://raw.githubusercontent.com/huggingface/smolagents/main/src/smolagents/default_tools.py) 接受任意类型的 answer。这些是显式结果提交的参考，Session 结束范围与历史投影仍由 OpenDAN 定义。

### H5：联动文档与协议（随实现同步）

- [x] 更新 xAgent.md 的 T1–T4、§3.4–§3.7、BehaviorConfig、C14、E19/E20 和待 review 问题；删除 S2 依赖。
- [x] 同步 llm_context/readme、snapshot_overrides 的说明与示例、Session Directory / SDK 协议、冻结配置和 fixtures。普通切换保留为已废弃说明，旧 Fork 与新版 fork 清楚区分。
- [x] 将 [LLM Context Switch 消息示例](<../doc/llm_context/LLM Context Switch (histoiry samples).md>) 中旧 end_session 建议同步为 H4 的 report；补齐显式提交、可选 is_end、派生最终消息及恢复示例，并与 Agent Actions / Agent Message 的报告与附件分层说明交叉引用。（工具、文档及恢复测试均已同步。）
- [x] 若 run / Session 持久字段或枚举变化，同步前后端类型、日志展示、CLI 接手逻辑和版本校验。（session_config /6、session_state /6、run.json 6、snapshot 5、mechanical/3；schema 与 fixtures 已重生成；无仓库内前端 / TS 镜像。）
- [x] H4 实施时同步 XML result 提示词、Agent behavior 模板、LLM Context 设计 / behavior loop 文档、Session 协议、schema 与 fixtures；实现与持久格式已升级，旧目录不迁移。

## 6. 验收矩阵

以下是验收要求。V1–V7、V10 已有对应测试（§8）；V8 随 H3 暂缓，不纳入当前验收；V9、V11、V12 已由 H4 的新测试覆盖，见 §9。

| 编号 | 场景 | 通过条件 |
|---|---|---|
| V1 | 不同目标配置三种进入模式；缺失 / normal 模式 | 按目标配置执行；非法配置提前报错，不落回同一 run 换 system |
| V2 | DO → CHECK → DO → CHECK | 各自 run_id、system / 工具、history 和预算不串；重入恢复目标原快照，全程一个 Turn |
| V3 | 两种 loop 的 create-sub-context，None / 最近对话 / Steps | system 使用子配置、输入范围符合声明；不把进行中 Step 当完成记录、不修改父快照 |
| V4 | fork 的文本 / 多模态 / 工具历史与 behavior 热 Step | 分叉点前有效消息前缀一致；不换 system、不隐式摘要；继承部分不重复写 worklog |
| V5 | fork / create-sub 工具与其它调用在同一批次，父 PendingTool / mid-action | 子请求合法；父批次保留；返回仅回填对应 call_id，余下工具按序续派，不重放已执行工具 |
| V6 | hosted run 交接处由 xllm 接手，多次 resume，转移中崩溃 | 停在交接点；宿主只完成一次转移、不误关闭 Turn、不丢目标 |
| V7 | 子 run 创建 / 完成 / 父结果提交各阶段崩溃 | 恢复同一子 run，结果恰好交回一次；父 history 只收到结果，审计可追踪完整子过程 |
| V8（暂缓） | 未终态 UI Session 的 Stop 后补充输入候选方案 | 不纳入当前验收；若重新启动 H3，再按 UI 需求冻结 Turn / run 边界、输入竞态、子调用与恢复要求；不覆盖 Work 续聊或终态 reopen |
| V9 | report 的 is_end 缺省 / false / true；成功 / 拒绝 / 同批后续工具 | 按定稿的默认值区分报告与结束；普通报告可继续，获准结束无需额外推理；结果完整配对，结束后未派发工具不执行；子报告归属正确、不关闭父，独立 xllm 不默认暴露该工具 |
| V10 | 历史压缩后再次进入或返回父 | 正常继承边界和编号继续；不把子完整 transcript 再混入父主干，不重置父预算 |
| V11 | report 的文本 / 产物 / 可选业务结果、重复提交、各提交阶段崩溃与历史重建 | 不要求普通 assistant 正文符合 schema；失败可修正；最终结果、稳定产物引用和派生消息一致且只提交一次，Round 不增加；重建不重执行工具或重复展示最终交付 |
| V12 | 两套 XML parser 的 report.end、切换 / 等待、动作冲突、子返回与 xllm 接手 | 缺省 / false 不请求结束；true 的无动作报告正常提交并请求结束；保留 next_behavior 调度，取消 END / 隐式 done 完成；冲突在副作用之前反馈错误；正常执行与恢复交付同一报告，子结束不关闭父 Session；其余边界按 H4.1 定稿规则验收 |

## 7. 实施范围与策略定稿

S1 / S3 / S4 / S5 → H1 / H2 → H4 / H5 已完成。H3 / V8 继续暂缓，S2 已取消，不存在强制 reopen 或旧终止语义兼容回退。

- function_call Session 的默认启用工具列表包含 report；自定义列表可选 `{name:"report"}`。`session.policy.completion=explicit_report` 会将 report 纳入该 run 的冻结配置；该策略与显式 tools.enabled=false 冲突时提前拒绝。Behavior 使用 XML 报告，不要求额外工具。
- 完成策略默认 natural，普通 Done 与已接受的 context 结束沿用 end_condition；可选 explicit_report 只有获准结束报告才能正常结束 Session。该策略下有输入队列的普通 Done 关闭 Turn 并等待，无输入队列则以 missing_end_report 失败，不无限等待。子 context 返回调用方。
- 提交 final 前拒绝未完成的 run task、顶层被跟踪 task、尚未收齐汇报的子 Session。执行成功与 Work 的 acceptance=pending 分开。先观察到的 Stop 拒绝新提交；已持久接受的提交先完成配对与交付，后到的 Stop 再作用于仍未结束的父 / Session。
- 报告身份按 run + tool call_id / behavior Step；阶段 latest 与获准结束分开。artifacts 明确选择文件并保存 SHA-256 稳定副本，result 接受任意 JSON（包括 null），不固定业务 schema。
- `report_delivery` 为机械 assistant 投影，保留真实模型调用 / 工具回执；历史构建只显示一份最终报告。重做依赖持久 journal 与配对快照，不重执行提交、不增加 Round。
- xllm 的 hosted XML 结束保存 paused 最终快照并交宿主校验；接手已有工具报告描述可保留占位，但不能自行接受 Session 工具报告。已有完成 journal 或待宿主验证的最终 XML 不允许 xllm 重复推进。

## 8. 实施记录（2026-10-03）

### 8.1 改了什么

**llm_context**（`src/frame/llm_context/src/context_derive.rs`，新模块；`snapshot_overrides.rs` 只更新说明）

- `derive_child(parent, child_request, InheritHistory::{None | Material(msgs) | Steps})`（S1）与 `fork_snapshot(parent, ForkOptions)`（S4）。都是纯函数，父快照只读，父可以处于正常、`PendingTool` 或批次 / Step 中途；派生结果不带父的 suspension、待派发调用、用量、错误计数与 host metadata，step / action 编号接着父的，并返回 `InheritBoundary {messages, steps_below, next_action_id, fork_point}`。
- `Material` 拒绝 system 消息与未配对的工具块、去掉 thinking；`Steps` 只复制已完成的 steps / summaries（`action_step` 不算），子请求不是 behavior loop 时拒绝。
- fork：有未完成调用时，分叉点取触发批次的 assistant 消息之前（function call）/ 进行中 Step 之前（behavior，丢弃其 inner transcript）；前缀里仍有未配对调用则拒绝。`ForkOptions.expect_system` 与父 system 不同则拒绝并提示改用 create-sub-context。function call 模式下继承的历史成为子的 `request.input`（宿主因此只写入子新增的消息）；behavior 模式保留 `request`、steps、热 Step、history_inputs。
- S5 没有新增原语：恢复仍走 `LLMContext::resume` 的既有 fill 校验，`inject` 的边界约束未放宽，`rebuild_with_inherit` 的前置条件不变。

**xllm**（`src/frame/agent_tool/src/xllm.rs`，S3）

- run.json 版本 5，新增 `handover {next_behavior, at_ms}`。宿主 run 的 `Done` 指向其它 behavior（不是 `END` / `done` / `WAIT_USER_MSG`）时记 `paused` + `handover`，不记 `completed`；`XllmRun::resume` 对带 `handover` 的 run 返回 `NotResumable`，重复 resume 不推理。独立 xllm run 行为不变。

**libopendan**（`src/frame/lib_opendan`，H1 / H2）

- 配置：`extensions.opendan.behaviors.<name> = {mode: switch_context | create_sub_context | fork, system_prompt?, llm_context?, inherit?: none | recent_dialogue | steps}`（`protocol/config.rs`）。`process_modes` 被拒绝；没有 normal 回退；表在推进开始时校验。初始 behavior 没有条目时视为使用基础配置的 `switch_context` 目标。
- 状态（`opendan.session_state/4`）：`ProcessFrame.role = parked | caller`，`caller` 帧带 `call {mode, behavior, trigger: behavior | tool{call_id, task_id}, task}`；`handover_at_ms` 记录已提交的交接。`ProcessMode` 删除，“构造语义”（`call.mode`）与“返回协议”（`call.trigger`）分开。
- 调度（`runner/outcome.rs`、`runner/live.rs`）：删除普通切换分支；`classify_done` 按目标进入模式决定挂起为 `parked` 还是 `caller`；新 run 由 `own_run_context`（目标自己的 system / 配置，`inherit` 决定历史，子 context 经 `derive_child`）或 `fork_run_context`（复制调用方的 run 配置，经 `fork_snapshot`）创建；恢复 run 时不再改写 `behavior_name`。
- 子 context 返回：任何结束都交回调用方，`process_result.status = ok | failed | needs_user_input`（`WAIT_USER_MSG`、不可重试错误、预算耗尽、未知目标都不结束 Turn）；子 context 进行期间 msg / event 留给调用方；`caller` 帧最多 4 层；子 context 内交接到 `switch_context` 目标等同于返回。
- 工具触发：宿主工具 `call_behavior({behavior, task})`（`runner/tools.rs`）返回 `Pending{task_id = "subctx:<call_id>"}`，run 只为它开启 `allow_deferred`（其它工具仍在调用内等待）；调用方挂起在 `PendingTool`，子结果在打开调用方 run 时用 `ResumeFill::ToolResults` 回填，然后续派同批余下的调用，不产生输入批次。
- 恢复：交接点先随快照写入 run.json（`checkpoint_handover` / `PendingTool` 快照），`reconcile::redo_transfer` 据此只提交一次转移；xllm 留下的 `handover` 走同一条路径（旧 G8：不再降为 Done）。
- 历史（`runner/history.rs`）：已返回的子 run 的过程不进入为其它 context 重建的会话历史与压缩输入，只渲染其 `process_done` 结果；worklog 保留完整子记录。

**文档与协议**：Session Directory Protocol §4.2 / §4.3 / §7 / §8 / §10、Session Input Protocol、protocol README、schema（新增 `behavior_entry`）、13 组 fixtures 重生成；`doc/llm_context/*`、xAgent.md、消息示例文档同步。

### 8.2 与清单的差异（实现时的取舍）

- H2「子 run id 预分配、Session 登记 task_id → 子 run、装配 RunningTaskResolver」：task id 由调用的 `call_id` 确定（`subctx:<call_id>`），`call_id` / `task_id` 登记在 `caller` 帧，子 run 就是随后的 `live_run`；提交点是引用子 run 的那次 state 提交，之前崩溃留下的孤儿 run 由 reconcile 删除并重建。没有为子 context 装配 `RunningTaskResolver`（调用方挂起期间不会查询它），因此**没有** `cancel_task` 取消单个子 context 的能力，取消只能通过 Session Stop。
- S4「同批兄弟调用的必要结果可作为子任务输入附带」：宿主不自动附带，由模型写进 `task`。
- fork 的工具集：原语支持 `ForkOptions.tool_policy`，但宿主的 fork 条目不允许声明 `llm_context` / `system_prompt`，fork 子 run 总是用调用方的配置（因此不存在换 provider / loop mode 的 fork）。
- `switch_context` 新建 context 的历史缺省为 `none`（只有 system 与交接输入），需要会话历史时显式写 `inherit: recent_dialogue`。
- 压缩时子 run 过滤按“被压缩片段内出现的 `process_done`”判断：子 run 的记录与它的 `process_done` 被压缩切点分开时，切点之前的那部分仍会进入摘要。

### 8.3 验证

- `cargo test -p llm_context`：207 通过（新增 `context_derive` 8 个）。
- `cargo test -p agent_tool --lib`：222 通过，5 ignored。
- `cargo test -p libopendan -- --test-threads=1`：全部通过。新增 / 改写的用例：
  - V1：`hand_over_without_an_entry_mode_fails_instead_of_switching_in_place`、`invalid_behavior_table_is_refused_before_anything_runs`
  - V2：`switch_context_targets_keep_their_own_system_and_history`、`switch_context_processes_keep_their_own_runs`
  - V3：`create_sub_context_uses_its_own_system_and_selected_history`（none / steps / recent_dialogue）、`tool_triggered_sub_context_has_its_own_system`
  - V4 / V5：`tool_triggered_fork_keeps_full_history_and_returns_to_the_call`、`action_triggered_fork_returns_into_the_step`、`fork_child_inherits_steps_and_returns_to_the_parent_run`
  - V6：`xllm_yields_at_a_hand_over_and_the_session_commits_it_once`、`crash_at_the_hand_over_point_commits_the_transfer_once`、`xllm_refuses_a_run_suspended_on_a_sub_context`
  - V7：`sub_context_call_survives_a_crash_at_every_stage`（4 个故障点）、`fork_return_survives_a_crash_after_the_child_finish`
  - V10：`returned_sub_context_transcript_stays_out_of_rebuilt_history`
  - 其它：`sub_context_wait_and_failure_return_to_the_caller`、`input_arriving_during_a_sub_context_goes_to_the_caller`
- 未覆盖：V4 的多模态内容（原语按消息整体复制，没有专门用例）；V10 中“压缩后再进入子 context”的组合；`uv run buckyos-build.py` 与 DV 环境未运行。

### 8.4 未做

- 2026-10-03 未实施 H3（Stop 后补充输入）、H4（`report` 工具与显式完成策略）。后续决定：H3 / V8 于 2026-10-10 暂缓；H4 的最新设计与待定边界见 H4 / §7，对应 V9 / V11 / V12。
- 旧 opendan Runtime（`src/frame/opendan`）仍用自己的 `RequestOverrides` 切换实现，本次未动。
- 已有 session 目录不迁移：`state.json` 为 `/3`、run.json 为版本 4 的会话会被拒绝（RecoveryBlocked），按 beta 2.2 规则不做兼容。


## 9. H4 / H5 实施记录（2026-10-10）

### 9.1 实现

- llm_context：两套 parser 共享显式 report_end / artifacts / result 合同；snapshot 5。协议冲突在任何派发前拒绝，宿主 `validate_report` 校验失败回到模型纠错。`before_tool_call` 保存完整未执行批次；平滑结束补齐跳过回执。最终 XML 快照恢复直接得到同一次报告，不推理；已接受工具报告先完成回执与平滑结束，过期 wallclock 不遮蔽完成。Step.native_messages 保留 Behavior 内层原生调用与结果，游标避免跨检查点重复写审计。
- agent_tool：xllm 提示词与 parser、run 版本 6、xllm/2；hosted XML final 交回宿主，已接受工具完成 journal 不再接手。执行中断回填同时移除待执行队列里的已回填 call_id，原 action_step 保持配对。
- libopendan：新增 `protocol/report.rs`、`runner/reports.rs`；工具 report、可选完成策略、按当前调用关系裁决范围。run.host.extra.reports 保存提交与独立完成意图；state.latest_report / final_report 保存顶层投影。最终 Work 的验收仍由 decide 决定。
- 持久化：稳定产物位于 `.opendan_agent_session/reports/<submission_id>/<index>-<sha256>`；提交身份按 run/source 幂等。接受后跳过同批其余工具，回执配对后通过既有 finish_run 提交。report.md、result、出站与 worklog.report_delivery 使用同一正文。机械历史 /3 去除重复最终正文，审计仍保留真实调用与回执。
- H5：核心 LLM/Session 协议、消息示例、behavior 模板、schema、现有 fixtures 与新增 `15_report_pending_commit` 同步。session_config /6、session_state /6；旧格式拒绝恢复，不增加迁移或兼容分支。H3 / V8 继续暂缓。

### 9.2 验证

在 `src/` 下执行：

- `cargo test -p llm_context -p agent_tool -p libopendan --no-fail-fast -- --test-threads=1`：**705 通过、0 失败、7 ignored**。其中 llm_context 222 通过；agent_tool 247 通过、5 ignored；libopendan 236 通过、1 ignored；另有 1 个文档示例 ignored。
- `cargo check -p opendan`：通过，验证共享类型与 Loader 消费方。
- `cargo run -p libopendan --example fixtures -- ../doc/opendan/protocol/fixtures`：重生成 schema 与 15 组 fixtures；fixture 回归全部通过。`git diff --check` 通过。

新增验收覆盖：

- `tests/report.rs` 的 19 个测试覆盖阶段 / 最终报告、参数与产物纠错、活动 task、子 context 返回、显式完成策略、稳定产物、JSON null、重复身份、原生工具审计和最终历史去重。
- 同文件故障矩阵覆盖 function_call、XML report、Behavior 内层原生 report 三条路径 × 四个提交窗口，恢复不增加推理或 Round，不重复工具效果、产物登记或最终交付。
- `tests/report_stop.rs` 覆盖持久接受 / 回执配对两个窗口 × 顶层单 Turn、自然 MaxTurns、子 context 三种场景；已接受结果先提交，Stop 随后作用于仍未结束的 Session / 父 context。
- llm_context 新增过期 wallclock × 已配对 / 待回执 × 两种 Loop 的恢复回归；两套 parser 的 end 属性、动作冲突、空决策与纠错，以及 xllm hosted final 让出与拒绝重复接手均有测试。
- 新 fixture `15_report_pending_commit` 保留已接受报告、稳定产物与配对快照，验证零推理恢复提交。

未运行完整 `buckyos-build.py` 打包与在线 Zone DV；真实 SSH / 外部服务相关 ignored 测试未启用。未收齐子 Session 时拒绝 report、xllm → Session 最终报告完整交接尚无专用组合测试，相关宿主校验与接手边界已有部件测试。H3 / V8 继续暂缓。

