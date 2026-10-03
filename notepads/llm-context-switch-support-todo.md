# Context 调度支持 TODO：SWITCH_CONTEXT、create-sub-context 与 fork

日期：2026-10-03

状态：设计更新，代码待实施。依据 [LLM Context Switch 消息示例](<../doc/llm_context/LLM Context Switch (histoiry samples).md>) 的本次 review，以及[提示词方式图解](<../doc/opendan/几种典型的提示词方式图解.drawio>)。本文区分已经确定的语义、源码现状和仍待选择的方案；未勾选的实施项均未因本次文档更新而完成。

## 1. 本次确定的语义与旧计划调整

| 事项 | 本次结论 | 对旧计划的影响 |
|---|---|---|
| 模式归属 | 进入模式由目标 behavior 决定；Session 负责执行调度 | 取消 Session 级统一 switch_mode，不允许 normal 缺省回退 |
| 普通切换 | 废弃“替换 system、沿用原 context 历史”的方式 | 取消旧 T1 `switch_in_place` 与 S2“run 中途替换 config” |
| SWITCH_CONTEXT | 目标有自己的 system、历史和 run；再次进入恢复原快照 | 原 independent 的设计名称统一为 SWITCH_CONTEXT；`SWITCH_CONTEXTG` 视为笔误 |
| create-sub-context | 新 system + 子任务输入，可显式选择父历史 | 旧 T2 中“使用 B 的 system、继承父 steps”的方案归入此类 |
| fork | 保留父 system 与分叉点的完整有效历史，再追加分支任务 | 需独立的完整历史派生原语，不能等同于旧代码只复制 steps 的 fork |
| end_session | 建议给多次交互的 function_call WorkSession 提供显式完成工具 | 建议，尚未确定默认结束策略；见 H4 |
| UI Stop 后补充输入 | A：保留历史继续原 Turn；B：结束本次交互、重建新 Turn | 两种方案及消息形状已补齐；默认行为仍待定，见 H3 |

[xAgent.md](../doc/opendan/xAgent.md) §3.3–§3.7、C14、E19/E20 仍使用旧转移表，不能继续把其中 T1、旧 fork 定义和 S2 作为本轮实现依据。保留编号便于追踪，映射如下：

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
- 本次只更新两份文档；下面的源码修改和验收为后续实施要求。其它文档的冲突列入 H5。
- 持久结构需要变化时按 beta 2.2 规则升版并拒绝旧格式，不增加 normal / independent 等旧配置的兼容回退。

## 3. 当前源码与缺口（2026-10-03 核对）

| 入口 | 已有能力 | 缺口 |
|---|---|---|
| `llm_context/src/snapshot_overrides.rs` | `build_fresh`、`rebuild_with_inherit`、`RequestOverrides` | `rebuild_with_inherit` 拒绝 suspended / mid-batch 父快照；允许替换 system，本身不能保证新版 fork 语义；尚无独立 `derive_child` / `fork_context` |
| `lib_opendan/src/runner/assembler.rs::process_mode` | 按目标名读取 `extensions.opendan.process_modes.<behavior>` | 缺省仍返回 normal 路径；尚未形成三种模式的目标 behavior 配置与校验 |
| `runner/outcome.rs::handle_context_outcome` | 普通切换改 behavior_name；Fork / Independent 入栈 | 普通分支仍在，且 system / 工具未更换；要删除此路径，而非补齐换配置 |
| `runner/live.rs::new_run_context` | 旧 Fork 为 behavior 子 run 复制 steps / last_step / summaries 和编号 | 不保证完整 request / transcript / history_inputs / system 前缀；function_call 全历史派生缺失 |
| `runner/live.rs::suspend_run` | Independent 可恢复目标挂起 run | 可作为 SWITCH_CONTEXT 调度基础；首次创建装配共享 worklog，需要明确历史范围 |
| `agent_tool/src/xllm.rs`、`runner/reconcile.rs` | 保存 run / snapshot，接手执行、恢复 Session | hosted 的 `Done{next_behavior=B}` 仍可能记 Completed；reconcile 把 Switch 改成 Done，目标交接丢失（旧 G8） |
| `runner/inputs.rs`、`runner/drive.rs::stop_session` | `ControlCommand::Stop` 终止 Turn 与 Session | 不能表示“暂停后补充信息”，没有 H3 两种可继续 Session 的产品命令 |
| `protocol/config.rs::EndConditionType` | LlmDeclaresDone / OutputSchema / MaxTurns | 没有显式完成策略和 end_session 工具 |

旧 `RequestOverrides.user_messages` 会清空 steps / summaries / history_inputs，适合构造新输入，不能用于声称“完整继承”的 fork。现有 `rebuild_with_inherit` 的前置条件应保留，不为子派生而放宽父 run 的恢复校验。

## 4. llm_context / xllm 原语

### S1：create-sub-context 的历史派生（P0，替换原 S1）

- [ ] 提供纯派生接口（可沿用 `derive_child` 名称），输入为父快照、显式历史选择和独立 child request；返回子快照与继承边界信息。父可处于正常、PendingTool 或批次中途，派生不得修改父快照。
- [ ] `None`：只使用子 system / 参数；`RecentDialogue`：由宿主选取近期对话并渲染；`Steps`：选择父已完成 steps / summaries。名称为设计描述，最终参数类型随实现定稿。
- [ ] `RecentDialogue` 的筛选、来源标记和模板归宿主；只含文本的结果必须明确是筛选视图，不能丢掉工具 / 多模态内容后称为完整历史。
- [ ] `Steps` 若作为结构化 StepRecord 继承，要求兼容的 behavior parser / renderer；跨 loop mode 时改用显式渲染的输入材料。进行中 `action_step` 不当作已完成记录复制。
- [ ] 子快照不带父的 suspension、待派发调用、abort 状态或输入消费 receipt；子工具额度、usage / 错误计数按 child request 独立初始化。继承 step / action 编号与新增部分的边界明确返回给宿主，以免重复写 worklog。
- [ ] 支持子 system、模型和工具配置独立于父。仅需新输入时复用 `build_fresh`；确需结构化继承才走派生接口，不为 None 新增一套运行循环。

### S2：取消 run 中途替换目标 config

**本项取消。** 原 `RunHandle::replace_config` 计划服务于已废弃的普通切换，不再实现，也不再列为 E19 的前置。新 context 写自己的有效配置；SWITCH_CONTEXT 恢复已有目标的配置。其它用途若确需改 run 配置，应单独说明，不能借本项恢复 normal switch。

### S3：hosted run 在交接点让出（P0，保留并更新）

- [ ] hosted run 遇到指向其它 behavior 的 `Done` 时，保存非终态交接状态、目标与必要快照，不记为已完成 Turn。独立 xllm 的行为不因此变成 Session 调度器。
- [ ] 优先复用 `Paused` + 持久化的交接原因 / 目标；仅在现有状态不能无歧义表达时考虑新增状态。xllm 记录交接事实，目标模式由宿主读取配置决定。
- [ ] repeated `xllm --resume` 遇到待宿主处理的交接时仍停在原交接点，不重复推理或误完成；只有宿主提交了转移才可继续目标 run。
- [ ] `reconcile` 不再把交接强制降为 Done；转移提交前 / 后崩溃都只能产生一次目标进入，保留原 Turn、已用预算和消费位置。
- [ ] 子 run 正常完成仍可记录完成状态，由宿主按调用关系交回父；不能把每个 END 都判为 Session finished。

### S4：fork 的完整历史派生（P0，新增）

- [ ] 提供单独的 fork 原语，强制父 system 保持相同，复制分叉点的完整有效 history，再追加子任务输入。不能通过 `RequestOverrides.system_messages` 换提示词；目标声明不同 system 时校验拒绝并提示使用 S1，不能静默改成另一种模式。
- [ ] function_call 保留所有有效 AiMessage / AiContent（含工具配对、多模态和已有摘要），不是只复制 user / assistant 文本；behavior 保留 request、steps、热 Step、history_inputs 等构造相同历史前缀所需的状态，不能改 behavior 后把原历史自动降级为摘要。
- [ ] “完整”只到记录的 fork point。工具触发时建议取触发批次之前最后一个合法前缀；父的当前批次、部分结果和待派发调用仍留在父快照中。禁止通过假 tool result 或固定 assistant 回答补洞。已执行同批兄弟调用的必要结果可作为新子任务输入附带，不能悄悄改写继承前缀。
- [ ] 父在 Step 中途派生时，区分只读历史与工具续派所有权；父的 action_step / tool_batch 不能在子 run 重新执行。必要时拒绝无法取得合法分叉点的请求，不静默截取 recent history。
- [ ] 派生结果返回继承消息 / Step 边界和 fork point，宿主只把子新增记录写入 worklog；child run 的 owner、host metadata、receipt、预算和统计独立。父恢复不重置其工具额度，Session 汇总子执行的实际用量。
- [ ] 新工具集可以显式装配，但不承诺 KV Cache 命中。若更换 provider / loop mode 使历史内容无法无损继承，应拒绝该 fork 或由调用方明确选择 S1；不能悄悄转换角色或删内容。

### S5：合法边界与恢复验证（P0，复用现有能力）

- [ ] SWITCH_CONTEXT 复用 `LLMContext::resume(..., ResumeFromMidRun)`；目标若等待工具结果，先走 `ToolResults`，若处于 context limit，先走相应历史重写。不能把任意挂起快照强行当 ResumeFromMidRun。
- [ ] 保持 `inject` 的边界约束：未配对批次 / 进行中 Step 不能直接插入新 user 消息。S1 / S4 的只读派生能力不能变成放宽父注入校验的后门。
- [ ] 复用 `finish` / `interrupt`、Cancelled / Unresolved 和现有快照配对语义供 H3 / H4 使用；仅补实际缺失的边界原语，llm_context 不新增 end_session 或 Turn 状态机。

## 5. 宿主配套工作

### H1：目标 behavior 配置与 SWITCH_CONTEXT（P0）

- [ ] 冻结目标 behavior 的进入模式、system、工具、模型与历史装配策略；明确配置枚举和序列化拼写。目标缺失 / 模式非法在执行前报错，删除 normal 回退以及 Session 级模式覆盖。
- [ ] SWITCH_CONTEXT 首次创建目标 context，之后恢复同一个挂起 run 的 system、工具、历史、编号和预算；交接输入由共享 Session 状态渲染。不能把 A 的原历史直接挂到新 system_B 下继续。
- [ ] 记录当前 / 目标 run、模式、调用来源和交接 receipt；按“父已挂起 → 目标已创建 / 恢复 → 输入已提交”的阶段恢复，避免重复创建或重复消费。
- [ ] 明确 `END` 的解释：普通目标按 Session 结束条件处理；子调用的完成返回调用方。切换与返回本身都延续 Turn。

### H2：两种子 context 的串行调用与返回（P0）

- [ ] create-sub-context 调 S1，fork 调 S4；父入栈后只推进子 run，子 run 写入 `runs/`，不能在工具里隐式启动不可恢复的第二个执行循环。
- [ ] 触发方式与创建模式分别记录：behavior 交接经 `process_result` 返回；工具调用经 `call_id` 对应的 `ToolResults` 返回。原 `ProcessMode::Fork` 需要拆分“构造语义”和“返回协议”，不能直接机械改名。
- [ ] 工具子调用按现有 `Pending{task_id, until_ms}` 挂起。Session 持久登记 task_id → 子 run / 父 call_id，装配 `RunningTaskResolver` 查询 / 等待 / 取消。子 run id 预分配，崩溃后重做进入 / 返回均幂等。
- [ ] 不再使用旧 `wait.source.kind = subrun`、`class` 等结构。[long-tool TODO §4](./llm-context-long-tool-todo.md#4-p1等待llm_context-的两种机械判断与-runningtaskresolver) 已将它们简化为不透明 task_id；接手方检查 resolver 能否解析等待对象。llm_context 不识别 task_id 前缀。
- [ ] libopendan 为子调用开启 deferred 并接入等待 / 回填；不能只开启 `allow_deferred` 却仍在宿主返回 PendingTool 后无恢复路径。xllm 不具备所需 Session 工具 / resolver 时明确拒绝接手，不能将不可解析的子 run 当普通本地未知 task 丢弃。
- [ ] 子任务 `Done` 返回结果，Error / Budget 作为失败结果交回父工具；`WAIT_USER_MSG` 的处理显式定义，建议工具子调用返回结构化 needs_user_input 让父询问用户。behavior 子调用是否直接等待、以及嵌套切换范围仍需定稿，不能沿用“所有 next_behavior 都立即返回”的假设。
- [ ] 子新增 messages / steps 写入共享 worklog，但不合并到父快照；后续压缩和历史重建按 run / 调用关系筛选，父默认只带交接结果。保留完整子记录供追踪，继承的父记录不重复写入。
- [ ] 栈深、嵌套和取消传播由 Session 统一校验；子 context 不能直接消费父输入队列或因继承控制工具而关闭父 Session。

### H3：Stop 后补充输入（P1，默认策略待定）

- [ ] 保留终止 Session 的现有 Stop 语义；为 UI 的“暂停 / 停止本次交互”设计单独控制意图，明确不使用 finished Session 继续收消息。
- [ ] 方案 A：保留配对历史，保持原 Turn / run，进入等待补充输入的状态；停止后不能由 Runner 自动立即续跑。新输入到达后，完成必要的恢复 / 配对，再追加输入批次。
- [ ] 方案 B：旧 Turn 记 stopped、Session 仍可接收输入；记录明确的裁剪 / 重建事件，新 run / 新 Turn 输入保留已完成动作、取消和未知副作用。原 worklog 不删，不重跑旧工具。
- [ ] 停止控制与新输入分别持久化；覆盖推理中、工具中、PendingTool、Step 边界、子 context 活跃、空闲时，以及“停止未完成消息先到”的竞态。队列中的补充输入不能丢失、提前消费或被子 context 抢走。
- [ ] 当前 Turn 的 task 取消 / 停止传播沿用 long-tool TODO，不能顺带终止其它 Turn 的 task。审计说明仍需保留已发生的副作用。
- [ ] 建议普通 Stop 用 A、明确重新开始用 B；这只是建议，按钮映射和字段未确定前不要实现为不可逆默认。

### H4：function_call WorkSession 的 end_session（P1，建议待定）

- [ ] 增加可选的显式完成策略。普通 assistant `Done` 可以完成当前 Turn 并等待后续输入；只有获准的 end_session 声明才正常关闭 Session。单次输出型 Session 继续按现有 end_condition 收尾。
- [ ] 提供宿主工具 `end_session`，参数至少能表达最终 report；冻结到适用 Session 的工具配置中。独立 xllm 不暴露，子 context 默认无权关闭整个 Session。
- [ ] 工具登记完成意图，工具结果配对和快照提交后由宿主完成 Turn / Session；不要求额外 ack 推理。可复用平滑结束让 LLMContext 让出，但必须用独立、持久化的完成意图解释结果，不能把 Settled 一律算 stopped 或成功。
- [ ] 接受完成意图后不执行同批余下工具，标为未执行；此前已经执行的调用不回滚。校验拒绝时返回普通错误，不能错误结束 Session。
- [ ] 完成意图、report、工具回执与最终状态支持崩溃重做；重复 call_id / 完成请求不得重复关闭 Turn 或生成多份结果。定义与未完成子调用、活动 task 和人工验收的关系，保持“执行完成”和“产物已验收”分开。

### H5：联动文档与协议（随实现同步）

- [ ] 更新 xAgent.md 的 T1–T4、§3.4–§3.7、BehaviorConfig、C14、E19/E20 和待 review 问题；删除 S2 依赖。
- [ ] 同步 llm_context/readme、snapshot_overrides 的说明与示例、Session Directory / SDK 协议、冻结配置和 fixtures。普通切换保留为已废弃说明，旧 Fork 与新版 fork 清楚区分。
- [ ] 若 run / Session 持久字段或枚举变化，同步前后端类型、日志展示、CLI 接手逻辑和版本校验；本次没有修改这些代码或文件。

## 6. 验收矩阵

以下是实施后的验收要求，本次未运行这些代码测试。

| 编号 | 场景 | 通过条件 |
|---|---|---|
| V1 | 不同目标配置三种进入模式；缺失 / normal 模式 | 按目标配置执行；非法配置提前报错，不落回同一 run 换 system |
| V2 | DO → CHECK → DO → CHECK | 各自 run_id、system / 工具、history 和预算不串；重入恢复目标原快照，全程一个 Turn |
| V3 | 两种 loop 的 create-sub-context，None / 最近对话 / Steps | system 使用子配置、输入范围符合声明；不把进行中 Step 当完成记录、不修改父快照 |
| V4 | fork 的文本 / 多模态 / 工具历史与 behavior 热 Step | 分叉点前有效消息前缀一致；不换 system、不隐式摘要；继承部分不重复写 worklog |
| V5 | fork / create-sub 工具与其它调用在同一批次，父 PendingTool / mid-action | 子请求合法；父批次保留；返回仅回填对应 call_id，余下工具按序续派，不重放已执行工具 |
| V6 | hosted run 交接处由 xllm 接手，多次 resume，转移中崩溃 | 停在交接点；宿主只完成一次转移、不误关闭 Turn、不丢目标 |
| V7 | 子 run 创建 / 完成 / 父结果提交各阶段崩溃 | 恢复同一子 run，结果恰好交回一次；父 history 只收到结果，审计可追踪完整子过程 |
| V8 | Stop 两方案后补充信息，含工具中和子 context 中 | 分别符合原 Turn 续跑 / 新 Turn 重建的消息形状；已发生动作可见、输入不丢失、不自动抢跑 |
| V9 | end_session 成功 / 拒绝 / 重复 / 崩溃 / 同批后续工具 | 正确完成或继续；结果完整配对，未派发工具不执行；子无权关闭父、独立 xllm 无该工具 |
| V10 | 历史压缩后再次进入或返回父 | 正常继承边界和编号继续；不把子完整 transcript 再混入父主干，不重置父预算 |

## 7. 实施顺序与待定项

顺序：S1 / S4 / S5 → H1 / H2；S3 可独立先做并与 H1 的恢复链路一起验收。H3 / H4 在产品默认策略明确后接入；H5 随对应实现同步。**S2 已取消，不在依赖链中。**

建议的验证命令：`cargo test -p llm_context`、`cargo test -p agent_tool xllm`、`cargo test -p libopendan`；xagent 集成用例按 V1–V10 覆盖，替换旧 E19 / 扩展 E20。只有实际修改代码后才据测试结果标记完成。

尚待决定：

1. UI Stop 默认采用 A，还是按 Session 类型区分；B 是否单独提供“重新开始”。
2. 哪些 WorkSession 使用显式 end_session，report schema、未完成子任务 / task 与验收规则如何限制完成。
3. 三种进入模式的最终配置拼写、子调用 WAIT_USER_MSG 和嵌套行为；fork 的工具触发边界按 S4 建议定稿。
4. hosted 交接能否完全用 Paused + metadata 表达；不足时才增加独立状态。
