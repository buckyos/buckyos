# 我们为什么做了 Behavior Loop

Agent Loop 这一层的设计,大部分实现共享着几个没被质疑过的假设:工具列表是一个固定集合、Loop 的结束由模型不返回 tool call 来隐式判断、状态机要么不存在要么是外挂框架。

这些假设在短任务里没问题。但任何做过 30 次推理(Round)以上长任务的人都知道,这一层的协议是有结构性缺陷的 —— 不是某个具体实现不够好,是协议本身没给一些必要的语义留位置。

这篇想讲的就是:在 Loop 这一层,有三个被普遍焊死的耦合点,其实是可以拆开的。Behavior Loop 是我们拆完之后的样子。

---

## 一、Function 和 Action 应该分开

这是改动最深的一条,先讲。

传统 Agent Loop 里,工具列表是**一个集合**。你在 prompt 里塞什么,LLM 就看到什么,也就是它能调的全部。这个看似自然的设计,把两件本来不同的事情焊在了一起:

- **物理能力清单** —— 系统里所有可调用的原子能力
- **语义动作集** —— 当前推理步骤里,LLM 应该看到的、能用的动作

这两件事的焊死,直接导致了所谓的"死工具流":LLM 在 prompt 里看到 50 个工具,实际每次只用 2 个,剩下 48 个白白消耗 context 和注意力。更深的问题是,调度器没办法**临时收窄或扩展**LLM 的认知能力集 —— 因为根本没有"工具的引用"和"工具的执行"这两个分离的概念。

Behavior Loop 把这两层拆开:

- **Function 层**是物理能力清单,工程师管,后端怎么实现、参数是什么,跟 LLM 无关
- **Action 层**是当前 Behavior 暴露给 LLM 的语义动作集,可以是 Function 的子集、组合、或者重命名

这本质上是一种**读写分离**:执行走 Function 层,认知走 Action 层。同一个 `http_get` 可以在调研 Behavior 里以 `research_web` 的语义出现,在调试 Behavior 里以 `fetch_api_response` 出现 —— 后端没动,但 LLM 看到的"我现在能做什么"完全不同。

这个分离带来的连锁后果不止是 context 优化:

- Context 注入策略可以独立于 prompt 工程演化 —— Function 池不变,Action 视图按需裁剪,死工具流自然消失
- 后端能力升级不需要重写 prompt —— Function 实现可以替换,Action 语义不变,LLM 不需要"重新学"
- Action 层成了 Behavior 的语义边界 —— 不同 Behavior 共享同一个 Function 池但暴露不同的 Action 视图,这是一种比"换 prompt"更深的角色分化

---

## 二、状态机应该是 Loop 输出协议里的一个可选槽位

Agent 圈里有两个长期对立的流派:

**宪法派**相信 LLM 足够强,给一个好的角色提示词加一组工具,它自己会规划好。状态机是工程师不信任模型的拐杖。

**状态机派**相信 LLM 不够可靠,必须用外部状态机锁住执行路径。LangGraph、Temporal-style workflow 都是这一派的产物。

这两派互相看不上,但他们其实在共享一个错误前提:**状态机要么不存在,要么是 Loop 之外的外挂框架**。

LangGraph 这类外挂状态机的存在,本身就是 Loop 协议设计不够的证据 —— 如果 Loop 自己能表达状态迁移,你不需要在它外面再搭一层。

Behavior Loop 在 Step 的输出协议里留了一个字段:`next_behavior`。

- 不填,Loop 继续在当前 Behavior 里推理 —— 这时它就是一个朴素的 ReAct Loop,宪法派可以完全无视这个字段的存在
- 填了,就是显式跳转到下一个 Behavior —— 系统提示词切换、Action 视图切换,LLM 进入一个新的认知上下文

这一个字段消解了整个派系对立:

- 你想做宪法派?永远留空,你得到的就是单 Behavior 的纯推理 Loop
- 你想做状态机派?在 Behavior 之间显式跳转,你得到的就是一个 **LLM 自己驱动**的有限状态机 —— 状态是 Behavior(以及它的系统提示词、Action 视图),迁移是 LLM 在 Step 里输出的 `next_behavior`
- 关键是这两种模式用的是**同一个执行核** —— 不换框架、不换工具协议,只是同一个 Step Schema 上的不同使用风格

更值得说的是,这种状态机不是被强加在 LLM 之上的约束,而是承认了一个事实:**LLM 在每次推理里本来就在做状态决策**(我下一步该探索还是收敛?该交给用户还是继续自动?),只是传统 Loop 没给这个决策一个表达通道。Behavior Loop 不是给 Agent 加状态机,是把 LLM 一直在做的状态机让它显式说出来。

---

## 三、意图信号必须显式

前两条是结构性的改动。这一条是基础设施,但它解释了为什么前两条能成立。

传统 Loop 里,意图信号是**双向缺失**的。

**输出方向**:LLM 不返回 tool call 就算结束了。但"它结束了"和"它觉得自己应该结束"是两回事 —— 前者是隐式推断,后者才是意图。中间断了你不知道是真的完成了还是只是这一次推理没调工具,恢复的时候只能把整个历史重新喂回去让 LLM 自己判断"我刚才到哪了"。这本质上是把**调度器的状态**藏在了**模型的注意力**里 —— 一个无状态系统假装自己有状态。

**输入方向**:LLM 在第 5 个 Round 发出 tool call 时,它不知道自己处于什么意图阶段 —— 还在探索?在收敛?在等待用户?Message array 没给它这个信号,只能从历史里猜。

Behavior Loop 的 Step Schema 强制每次输出都 commit 一个意图状态:

```
Step:
  观察:   上一步动作的结果观察
  思考:   基于系统提示词的要求和当前观察结论的思考
  动作:   要执行什么
  next_behavior: 留空(继续) 或 跳转目标(显式结束当前 Behavior)
```

这四个槽位每个都是双向意图通道 —— 既是 LLM 告诉调度器"我处于什么阶段",也是调度器和后续 Step 读到"上一步 LLM 处于什么阶段"。

有了这个基础,前两条才有放置的位置:Action 视图能按 Behavior 切换,是因为 `next_behavior` 让 Behavior 边界变得显式;状态机能内生于 Loop,是因为 Step 本身就是状态迁移的最小单元。

这里的 Step 是一次行为决策,连同它派发的 action 及其结果记为一个 `StepRecord`:一个 Step 内部可以有多个 Round(推理),也可以带多个 action;标准 function_call Loop 没有 Step。Round / Step / Turn 的定义见 [readme.md](readme.md)。

---

## 四、History、Attention 和 KV Cache 的取舍

Behavior Loop 不是 Chat Message Loop。它更接近一个 Work Session:围绕明确 Objective 持续推进,完成后结束。因此它的历史策略不追求无限累积对话,而是优先保证每次推理时关键信息落在 LLM attention 的"U 型区域"两端:

- 头部:稳定的 system prompt,包含 objective、process rules、result protocol、当前 Behavior 暴露的 Action 视图和 skills
- 尾部:最近若干个完整渲染的 Step(StepRecord 渲染成的 assistant/user pair),也就是 LLM 上一步输出的决策和系统执行后的 Action Results

这和 KV Cache 的最优命中天然存在张力。为了让旧历史逐渐从中部让位给新的完整 Step,历史会发生压缩;一旦压缩发生,严格的长前缀 cache 命中会被破坏。这个代价是有意接受的:对 Work Session 来说,让当前推理看到正确的任务头部和最近执行尾部,比维持一条永远 append-only 的 Chat transcript 更重要。

Behavior Loop 的压缩分两层。

第一层是常规的 StepRecord 分级压缩。StepRecord 仍然保留结构化语义,但历史 step 的 detail 会随着它滑入 context 中部而逐渐消失(这个别称作简单机械压缩，不会自动发生)。旧 step 可以从完整的:

```
assistant: Step Intent
user:      Step Action Results
```

降级为更短的 compact record。这样做的效果是:某个中部 StepRecord 被压缩后,它后面一段历史的 detail 可能都会被重新布局,但系统因此又为未来几个 Step 腾出尾部空间,让新的 Intent + Action Results 可以完整进入模型输入。

第二层是触顶后的强制有损压缩。它不是普通的 compact render,而是把一批旧 StepRecord 折叠成固定大小的 History Summary 块:

- 不再保留原始 Step 结构
- 记录被压缩的 step 数量
- 记录起止 step index、起止时间戳、所属 behavior 范围
- 摘要这批 step 大致完成了什么、留下了什么约束或结论

这层压缩是最后手段。它的目的不是让模型完整复盘每个动作,而是在 context window 快触顶时重新制造一个稳定的历史前缀,让后续 N 个 Step 可以继续以尽量少破坏 KV Cache 的方式运行。

因此,Behavior Loop 的历史不是"越完整越好",而是按位置和阶段承担不同职责:

- 当前 Behavior 的最近 Step:完整、强可见、位于尾部
- 当前 Behavior 的较旧 StepRecord:结构化但分级压缩
- 跨 Behavior 继承的旧历史:必须降级为系统可解释的 history record 或 summary,不能继续占用当前 Behavior 的 hot tail
- 触顶后的长期历史:固定大小的 summary block

## 五、Behavior switch 的三种模式

`next_behavior` 不是普通的提示词变化,而是状态机边界。LLM 只在 Step 输出里声明"我要去哪个 behavior";具体怎么进入由**目标 behavior 的进入配置**决定,不由 Session 统一指定。当前实现(libopendan Runner,`SessionAssembler::behavior_entry` / `SessionConfig::behavior_entry`)读 `session_config.extensions.opendan.behaviors.<behavior>`:

```json
"behaviors": {
  "do":       { "mode": "create_sub_context", "system_prompt": "…", "llm_context": { … }, "inherit": "steps" },
  "research": { "mode": "fork" },
  "writer":   { "mode": "switch_context", "system_prompt": "…", "inherit": "recent_dialogue" }
}
```

- `mode`:`switch_context` / `create_sub_context` / `fork`(`ContextMode`),没有默认值。交接到没有条目的 behavior 不回退:Turn 以 `behavior_config` 错误失败;发生在子 context 内则作为失败结果返回调用方。Session 的初始 behavior(`prompt.behavior`)没有自己的条目时,隐含为使用基础配置的 `switch_context` 目标(`inherit` 为 `recent_dialogue`)。
- `system_prompt` 替换 `prompt.system_prompt`;`llm_context` 的顶层键替换 `prompt.llm_context` 的同名键(模型、工具、限制等)。
- `inherit`(`InheritMode`,默认 `none`):`recent_dialogue` = 宿主渲染的 `<session_history>`;`steps` = 调用方已完成的 StepRecord,只用于 `create_sub_context`。`fork` 条目不能声明 `system_prompt` / `llm_context` / `inherit`。
- behavior 表在 drive 启动时校验;旧的 `extensions.opendan.process_modes` 被拒绝。"在同一个 run 里只换 `behavior_name` / system、沿用原历史"的普通切换已移除。

| 模式 | 心智模型 | context / run | system 与历史 | 结束语义 |
| --- | --- | --- | --- | --- |
| `switch_context` | 切到另一个各自保有历史的 context | 当前 run 挂起进 `process_stack`(`FrameRole::Parked`);目标有自己挂起的 run 就恢复,否则新建 | 目标自己的 system / 工具 / 模型;新建时的历史只按 `inherit`;其它 context 的快照 / 历史不会接上来 | 回到原 context 要显式 `next_behavior`;`END` / `done` 按 Session 结束条件处理,不会自动弹回 |
| `create_sub_context` | 换 system 的子任务调用,结束后返回 | 调用方作为 `FrameRole::Caller` 帧入栈;每次调用新建子 run | 目标自己的 system 和配置 + 任务输入 + `inherit` 选择的调用方历史(`derive_child`) | 子 context 无论以什么结束都返回调用方(`process_done`),只交回结果 |
| `fork` | 保留 system 和完整历史的分支,结束后返回 | 同上 | 调用方的 system、配置和分叉点之前的完整有效历史(`fork_snapshot`) | 同上 |

三种方式都不结束 Session Turn:交接和子 context 返回都并入当前 Turn(hook `on_behavior_switch` 的交接输入批次,或工具结果回填),Turn 只由 Session 在 `finish_run` 中关闭(见 [readme.md](readme.md))。

切换 Behavior 会同时更换 Work Session 的"头"和"尾":

- 头部更换:新的 system prompt、生效的 process rules、Action 视图和 skills
- 尾部重置:新的 Behavior 只把自己的 Step 渲染成完整的 assistant/user pair

libopendan 用"换 run"实现这两点:`switch_context` / `create_sub_context` 的目标 run 按 behavior 条目装配自己的 system 和 `llm_context`(工具 / Action 视图、模型);一个 run 的 system 和 `behavior_name` 自始至终不变,交接从不在已有历史上替换 system。"尾"由 `XmlStepRenderer` 按 `behavior_name` 区分:当前 behavior 的 Step 渲染成完整 pair,其它 behavior 的 Step(`create_sub_context` 以 `inherit: steps` 继承来的)渲染成 `<<step_history>>` 里的 `<step_record>`。`fork` 是例外:它两头都不换,目的就是保持与调用方相同的前缀。

因此跨 behavior 继承的历史只能作为系统解释过的 history record 进入新 behavior,不能继续占用新 behavior 的 hot tail。

### `switch_context`:各自保有历史的 context

`switch_context` 让每个目标 behavior 拥有自己的 run 和 Step 流。`handle_context_outcome` 收到 `Done{next_behavior: B}` 且 B 是 `switch_context` 目标时:

- `suspend_run` 先把当前 run 到目前为止的历史 flush 进 worklog(outcome `suspended`),run 进 `process_stack`(`ProcessFrame { role: Parked }`)。
- 栈里有 B 自己的 Parked run 时,恢复它作为 live run,用它自己的 snapshot;没有就新建 run:B 的 system / 工具 / 模型,历史只按 `inherit`(`recent_dialogue` 时装配 `<session_history>`,它来自 Session 共享 worklog,是宿主筛选过的视图)。
- 不把前一个 context 的 `steps`、`history_summaries` 或 hot tail 复制给 B,也不把别的 context 的快照接到 B 已有的 run 上。
- 每个 run 的 `step_index` 各自编号,在 Session 内不全局唯一。
- 下一次 drive 循环在 B 的 run 上提交 `on_behavior_switch` 输入批次,`<session_input hook="on_behavior_switch">` 里带 `<behavior_switch to="B"/>`。
- 回到原 context 需要 LLM 显式 `next_behavior` 切回;`END` / `done` 按 Session 结束条件处理(Turn completed,再按 `end_condition` 结束 Session 或等待输入),不会自动弹回上一个 context。

所以 `switch_context` 适合长期并列的独立工作流,不是"带上下文的分支执行"。

### `create_sub_context`:换 system 的子任务调用

- 调用方 run 的历史先 flush 进 worklog,run 作为 `ProcessFrame { role: Caller, call: ChildCall { mode, behavior, trigger, task } }` 入栈。
- `new_run_context` 为 child 新建 run(存在 `runs/` 下,可停止、挂起、崩溃恢复):request 用目标自己的 system 和配置,再用 `llm_context::derive_child(parent_snapshot, child_request, InheritHistory)` 派生初始快照。`inherit: none` 只有 system + 任务输入;`recent_dialogue` 多一条 `<session_history>`;`steps` 带入调用方已完成的 StepRecord 和 summaries(仍在派发 action 的 `action_step` 不算已完成,不带)。
- child 延续调用方的 step / action 编号;`HostMeta.inherited_below` 标出继承的 Step,flush 时不重复写入 worklog。继承来的 Step 在 child 里是 `<step_record>`,不是 child 的 hot pair。
- child 的交接批次(`on_behavior_switch`)带 `<behavior_switch to="<child>"/>` 和 `<sub_task mode="create_sub_context">…</sub_task>`(任务文本 + "结果返回调用方,不要向用户提问")。

### `fork`:保留 system 与完整历史的分支

调用 / 返回与 `create_sub_context` 完全相同,区别只在 child 的构造:`llm_context::fork_snapshot(parent_snapshot, ForkOptions)` 保留调用方的 request(system、`behavior_name`、模型、输出)和分叉点之前的完整有效历史,因此继承的历史在 child 里的渲染与调用方一致;child 的 run 记录沿用调用方的配置。

- 分叉点是最后一个合法前缀:调用方没有未完成调用时是全部历史;有未完成的工具批次(function_call)或进行中的 Step(behavior)时取在该批次 / Step 之前,这个批次、它已有的部分结果和尚未派发的调用都留给调用方,分支不会执行它们。分叉点记录在 `InheritBoundary.fork_point`。
- behavior 模式保留 `request.input`、`steps`、`last_step`、`history_summaries`、`history_inputs`;function_call 模式下继承的历史成为 child 的 `request.input`。
- `ForkOptions.expect_system` 与调用方 system 不同则拒绝(要换 system 应使用 `create_sub_context`)。
- child 的交接批次带 `<sub_task mode="fork">…</sub_task>`。

`derive_child` / `fork_snapshot` 都是纯函数:不修改父快照,不带走父的挂起状态、待派发调用、usage、错误计数和宿主元数据,返回 `InheritBoundary {messages, steps_below, next_action_id, fork_point}`。

### 子 context 的触发与返回

创建方式决定 system 与历史如何构造,触发方式决定结果从哪里返回,两者独立:

| 触发 | 调用方停在 | 结果返回 |
| --- | --- | --- |
| behavior 的 `next_behavior=B`(`CallTrigger::Behavior`) | 完整 Step 的 Done | 调用方交接批次里的 `<process_result behavior="B" status="…">…</process_result>` |
| 工具 / action `call_behavior({behavior, task})`(`CallTrigger::Tool{call_id, task_id}`) | `PendingTool`:工具返回 `Pending{task_id="subctx:<call_id>"}`,未完成的批次 / Step 原样保留 | `ResumeFill::ToolResults` 按 `call_id` 回填,然后继续批次余下的调用;不产生输入批次 |

子 context 无论以什么结束都返回调用方(`finish_run` 写 outcome `process_done`、出栈让调用方 run 重新成为 live run,结果放进 `state.process_result`):

- `END` / 只带 `<report>` 的终止 Step → `status=ok`,结果是终止 Step 的 `<report>`,没有就取最后的回答文本;
- `WAIT_USER_MSG` → `status=needs_user_input`:子 context 从不消费调用方的输入队列,由调用方决定是否去问用户;
- 不可重试错误、预算耗尽、交接到未配置的 behavior → `status=failed`;
- 在子 context 内交接到 `switch_context` 目标也只是返回(子 context 不离开自己的调用)。

工具触发时,`failed` 回填为工具错误,`needs_user_input` 回填为结构化 JSON,其余是结果文本。子 context 可以再调用子 context,最深 4 层(`MAX_CALL_DEPTH`)。只有 `call_behavior` 可以让 run 以 deferred 方式挂起,其它工具的 `allow_deferred` 被屏蔽;xllm 不能接手挂起在子 context 上的 run。

child 的 Step 不写回调用方的 context;Session worklog 记录 child 的过程,Step 身份是 `(child_run_id, step_index)`。child 返回后,重建 `<session_history>` 时这些记录被排除,只渲染它的 `process_done` 结果(`runner/history.rs`),避免子过程的完整内容又混进其它 context。压缩输入做同样的过滤,但只识别被压缩片段内的 `process_done`:切点把 child 的记录与它的 `process_done` 分开时,切点之前的那部分仍会进入摘要。

交接点(run.json 的 `handover`)随快照先落盘,再提交 state;崩溃后 reconcile(`runner/reconcile.rs::redo_transfer`)恰好补交一次,state 用 `LiveRun.handover_at_ms` / `ProcessFrame.handover_at_ms` 记住已提交的交接。

尚未实现:UI Stop 后补充输入(H3)、`report` 工具与显式完成策略(H4)。

> 旧 opendan Runtime(`src/frame/opendan/src/agent_session.rs` 的 `switch_behavior` / `apply_switch_*` / `handle_process_end`)是另一套实现:按 session class 的 `switch_mode` 切换,用 `apply_overrides_to_snapshot` 替换 request 侧(system prompt、tool policy 等),挂起的 process 快照存成 `.meta/behavior_<entry>.snap`,child `END` 时把 report 写成 parent `step_history` 里的 `<history_input source="process_return:...">`,independent 再入时 `reset_tool_iterations` / `reset_errors`,`END` 弹回上一 process、栈空才结束。这些是旧 Runtime 的私有 helper 和磁盘布局,不是新设计接口,待下一阶段 opendan 按 libopendan 的抽象重构接入。

## 六、Step history 和推理输入形态

Behavior Loop 每次推理的输入不是简单 append chat transcript,而是由 `LLMContext::build_inner_request` 现场拼出:request 头部(`request.input`)、`<<step_history>>`、当前 behavior 的 Step pair,以及进行中 Step 的 inner transcript。当前实现(llm_context + libopendan)的 message 序列是:

```text
system: run 的 system text(libopendan:身份 + 约束 + 应用 prompt + 初始 context + objective;应用 prompt 可由 behavior 条目的 system_prompt 替换,一个 run 内不变)
optional user: <session_history> ... </session_history>      (libopendan:Session worklog 的摘要 + 最近记录;按 behavior 条目的 inherit 装配)
user: <session_input hook="…" time="…"> ... </session_input>  (注入时最近一个 Step 不属于当前 behavior 的输入批次,留在 request.input)
optional user: <<step_history>> ... <</step_history>>
assistant: current behavior step decision
user:      current behavior step action results [+ 之后在这个 Step 上注入的输入批次 / 观察]
assistant: current behavior hot step decision
user:      current behavior hot step action results
assistant/tool: 进行中 Step 的 inner transcript(Step 内原生工具 loop 的消息)
```

`step_history` 是一条 user message,承载已经不该占 hot tail 的历史语义。`XmlStepRenderer::render_history` 按下面的顺序把它们拼进同一个 wrapper:

- 压缩后的 `<history_summary>`
- 其它 behavior 的 `<step_record>`(跨 behavior 继承:`create_sub_context` 以 `inherit: steps` 带入的调用方 Step)
- runtime 生成的 `<history_input>`(来自 `StepResultHook` 返回的 `history_inputs`;旧 opendan Runtime 用它承载 fork join handoff,libopendan / xllm 不挂这个 hook)

示例(默认 `XmlStepRenderer` 带时间戳;libopendan / xllm 用 `without_timestamps()`,没有 `started_at_ms` / `ended_at_ms`):

````xml
<<step_history>>
<step_record behavior="plan" index="1" started_at_ms="..." ended_at_ms="..." compression="full">
<observation>Todos were created successfully.</observation>
<thought>The plan is ready, so execution should start.</thought>
<actions>
- Run todo add "T01"

```output
Created T01.
```
</actions>
</step_record>
<</step_history>>
````

输入批次(包括 `on_behavior_switch` 交接批次)是 synthetic UserMessage,不属于 `step_history`。它的位置由 `LLMContext::inject` 决定:最近一个 Step 属于当前 behavior 时,并入这个 Step 的 user message,接在 `<</last_step_action_results>>` 之后;否则(run 的第一个批次,包括 `switch_context` 目标首次进入和 `create_sub_context` child 刚启动)追加到 `request.input`,因此渲染在 `<<step_history>>` 之前(下例是 `inherit: steps` 的 `create_sub_context` child):

```text
user:
<session_input hook="on_behavior_switch" time="..."><behavior_switch to="do"/><sub_task mode="create_sub_context">…</sub_task></session_input>

user:
<<step_history>>
...(plan 的 step_record)
<</step_history>>
```

这和旧 opendan Runtime 不同:旧 Runtime 把 `on_behavior_switch` 模板输出排在 `step_history` 之后,用位置表达"沉淀的 StepRecord 历史比交接指令更早发生"。当前实现里先后关系由 worklog(`input_batch` / `step` 记录)和输入批次 receipt(`input_seq`、`after_step`)保存,prompt 中的位置不表达时间顺序;是否要恢复"`step_history` 在前"的顺序尚未定,待下一阶段 opendan 重构时一并确定。

完整 step 渲染为严格相邻的 pair:

````text
assistant: <response>...</response>
user:
<<last_step_action_results behavior="<behavior_name>" step="<step_index>">>
- AgentToolResult.title

```output
AgentToolResult.output | AgentToolResult.detail
```
<</last_step_action_results>>
````

完整 pair 只属于当前 behavior:当前 behavior 的所有 Step 按追加顺序都渲染成完整 pair,渲染器不按新旧自动压缩,以保持前缀稳定;压缩只通过显式重写历史发生(例如 `ContextLimitReached` 后 `ResumeFill::RewrittenSteps`)。这些 Step 被另一个 behavior 的子 context 继承(`create_sub_context` + `inherit: steps`)时,在那个 context 里渲染成 `step_history` 里的 `<step_record>`,并携带至少这些元数据:

```text
behavior_name
step_index
started_at / ended_at(可关闭)
compression_level
```

随后推理产生子 context 的第一个 Step 决策(`step_index` 接着调用方编号,不从 0 开始);系统执行它的 actions 后得到 Action Results,这个 Step 成为当前 behavior 的 hot tail。它如果再被别的 behavior 继承,也只会以 `step_record` 或 summary 的形式出现,而不是继续作为新 Behavior 的完整 assistant/user pair。

## 七、交接输入、子 context 返回和真实用户输入的区别

当前实现(libopendan)里,进入 context 的输入都以输入批次提交:`SessionAssembler::render_input`(默认 `DefaultAssembler`)按 `InputMaterial` 渲染一条 `<session_input hook=… time=…>` user message,`commit_input_batch` 把它注入 context 并写 receipt `(run_id, input_seq)`。批次按来源区分:

- 真实用户 / peer message、业务 event:hook `on_wakeup`(Session 的第一个批次是 `on_init`),消息放在 `<inputs>` 里,同一批次还可以带 `<changes>`、`<hints>`、`<active_sessions>`、`<runtime>`。没有打开的 Turn 时它开启新 Turn,否则并入当前 Turn。
- behavior 交接:hook `on_behavior_switch`,带 `<behavior_switch to="…"/>`;进入的是子 context 时还带 `<sub_task mode="create_sub_context | fork">…</sub_task>`。它不是用户真实发来的消息,并入当前 Turn。
- 子 context 返回(`next_behavior` 触发):同样是 `on_behavior_switch` 批次,在调用方 run 上提交,带 `<process_result behavior="…" status="ok | failed | needs_user_input">…</process_result>`。它是输入批次,不进 `<<step_history>>`。
- 子 context 返回(`call_behavior` 触发):不是输入批次,结果作为该调用的工具结果回填(`ResumeFill::ToolResults`)。
- 观察边界的半订阅变化:`CheckpointHook` 在 Step 边界注入 `<changes>`(receipt hook `observation`),不开 Turn。

同一次 drive 循环里既有交接又有新消息时,它们渲染进同一条 `<session_input>`(`<behavior_switch>` / `<process_result>` / `<sub_task>` 在前,`<inputs>` 在后),provider message list 里只有一个交接锚点。

旧 opendan Runtime 的做法不同:真实输入作为本次输入的 tail,`on_behavior_switch` 由 behavior 配置的模板渲染成 synthetic UserMessage 排在 `step_history` 后面,fork child `END` 把 report / join marker 渲染为 `HistoryInputRecord` 进入 parent 的 `step_history`。这些属于旧 Runtime,待下一阶段 opendan 重构接入。

## 八、状态机和输入构造的精确定义

Behavior Loop 的 Runtime 状态不是 provider message list 本身。message list 只是每次推理前从状态寄存器渲染出来的视图。实现和日志分析应先看状态寄存器,再看它如何投影成 messages。

### 8.1 状态寄存器

一个 behavior-loop context(`LLMContextState`,外加 Session 的 process 栈)至少包含这些状态:

| 寄存器 | 含义 | 是否直接渲染为 message |
| --- | --- | --- |
| `request` | 当前 run 的 `input`(system、`<session_history>`、注入时最近一个 Step 不属于当前 behavior 的输入批次)、`behavior_name`、模型/预算策略 | `request.input` 原样在最前 |
| `steps` | 已沉淀的 StepRecord 流 | 当前 behavior 的 Step → 完整 pair;其它 behavior 的 Step → `step_history` 里的 `<step_record>` |
| `history_summaries` | 显式重写产生的历史摘要 | `step_history` 内的 `<history_summary>` |
| `last_step` | 当前 behavior 的 hot step,也就是最近一次决策及其 action results | assistant/user hot pair |
| `action_step` | 已解析、正在派发 actions 的 Step(尚未沉淀) | 不渲染;完成后沉淀为 `last_step` |
| inner transcript | 进行中 Step 的原生工具 loop 消息(`accumulated` 中 `request.input` 之后的部分) | 原样接在最后 |
| `history_inputs` | `StepResultHook` 返回的历史输入(旧 Runtime 用于 fork join handoff) | `step_history` 内的 `history_input` |
| 输入批次 | libopendan 由 InputSource 队列取输入、`commit_input_batch` 注入,身份是 receipt `(run_id, input_seq)`;不在 `LLMContextState` 里(旧 Runtime 是 `SessionMeta.pending_inputs`) | 一条 `<session_input>` user message,位置见第六节 |
| `process_stack` | 挂起的 context(libopendan `SessionState.process_stack`,每帧是一个挂起的 run:`Parked` = 经 `switch_context` 离开的 context,`Caller` = 等待子 context 返回的调用方,带 `ChildCall`) | 不直接渲染,决定恢复哪个 run、子结果返回给谁 |

`step_index` 是一个 run 的 Step 流里按分配顺序递增的序号(解析失败 / 策略拒绝产生的合成纠错 Step,即 `StepRecord::is_correction()`,也占一个号),不是每个 Turn 或每个 behavior 从 0 重开。子 context(`create_sub_context` / `fork`)延续调用方的 `next_step_index` / `next_action_id`,所以 child 产生的第一条新 step 不从 0 开始,调用方恢复后再接着 child 的编号;`switch_context` 新建的 run 从 0 编号。Step 身份是 `(run_id, step_index)`,`step_index` 在 Session 内不全局唯一。

### 8.2 Step 的生命周期

一个 StepRecord 有三个阶段:

1. `building`:当前 LLM 输出刚被解析成 Step 决策,存在 `action_step`,Runtime 正在执行 actions。
2. `hot`:actions 执行完毕后,该 step 存在于 `last_step`,下一次推理以完整 `(assistant, user)` pair 渲染。
3. `sedimented`:下一次产生新 step 时,旧 `last_step` 被沉淀进 `steps`。当前 behavior 的 sedimented Step 仍渲染成完整 pair;切到其它 behavior 后才渲染成 `<step_record>`;summary 只来自显式重写。

Step 内原生工具 loop 的消息(inner transcript)不属于任何 StepRecord,Step 完成后只保留决策和 action 结果。Step 未完成时 run 可以挂起(`PendingTool` / `ContextLimitReached` / `Interrupted`),inner transcript 和 `action_step` 留在外层快照里,恢复后继续同一个 Step,不重放已执行的工具。

`<<last_step_action_results>>` 是 hot step 的 user-side 占位。即使 step 没有 actions,也可以渲染为空:

```text
assistant:
  <response>
    <actions/>
    <next_behavior>DO</next_behavior>
  </response>

user:
  <<last_step_action_results behavior="plan" step="3">>

  <</last_step_action_results>>
```

这个空 block 表示"step 3 的 actions 已经观察完毕,结果为空",不是用户输入,也不是子 context 返回 / on_behavior_switch 的载体。真实用户补充不应该塞进这个 block:当前实现把之后注入的输入批次作为同一条 user message 的后续 content,接在 `<</last_step_action_results>>` 之后。

### 8.3 输入来源分类

进入 context 的内容按来源分成这几类(表中"模板"指旧 opendan Runtime 的 behavior 模板;libopendan 的对应见表后):

| 类别 | 例子 | 语义归属 | 渲染位置 |
| --- | --- | --- | --- |
| `runtime_step_result` | action 执行结果 | 上一个 Step 的 observation | `last_step_action_results` |
| `runtime_history_input` | fork child END、process-end marker | 已完成 runtime 状态变更 | `step_history` 内 |
| `runtime_auto_user` | `on_behavior_switch` / `on_init` 模板输出 | Runtime 生成的继续执行指令 | 本次输入批次的 tail |
| `external_user` | 用户消息、forwarded usermsg | 新事实 / 新约束 / 人类补充 | 本次输入批次的 tail,或嵌入 runtime tail 的补充小节 |
| `external_event` | kevent / msg event | 外部事件唤醒 | 本次输入批次的 tail |

libopendan 当前的对应:`runtime_step_result` 相同;`runtime_history_input` 不出现(libopendan 不挂 `StepResultHook`,不产生 `history_inputs`),子 context 返回改为输入批次里的 `<process_result>`(`call_behavior` 触发时是工具结果);`runtime_auto_user` 是 `<session_input hook="on_init" | "on_behavior_switch">` 里的 `<task>` / `<behavior_switch>` / `<sub_task>`;`external_user` / `external_event` 是同一条 `<session_input>` 里的 `<inputs>`;半订阅变化在 Step 边界以 `<changes>` 注入。

这几个类别不能互相混用。尤其是:

- `external_user` 不是 step result,不能放入 `last_step_action_results`。
- `runtime_auto_user` 不是历史事实,不能放入 `step_history`。
- `runtime_history_input` 是 Runtime 对已发生状态变更的解释,不应伪装成普通用户补充。

### 8.4 同一输入批次内多种输入的排序规则

一次输入批次可能同时取到多种输入。旧 opendan Runtime 的规范顺序如下:

```text
system: current behavior prompt
user:   step_history, including inherited steps and runtime_history_input
assistant/user hot pairs for current behavior
user:   runtime_auto_user, with embedded external_user supplement if both exist
user:   external_event, if any
user:   external_user, only when no runtime_auto_user can carry it
```

当 `external_user` 和 `runtime_auto_user` 在同一批次出现时,它们不是两条并列指令。`runtime_auto_user` 是 behavior 状态机恢复/切换后必须执行的锚点,`external_user` 是对这个锚点的新增事实。因此应把用户补充嵌入到 runtime tail 中,位置在继续执行锚点之前(下面是旧 Runtime behavior 模板的写法):

```text
## Last Finish Todo Report:
...

## Current TodoList:
...

## 刚刚用户补充的信息

玩法上要确定是Nokia玩法

Continue from PROCESS_RULES.
```

这样 provider message list 仍然只有一个明确的 tail instruction:先吸收补充事实,再按当前 behavior 的 process rules 继续。Session worklog 仍应记录原始输入(libopendan:`turn_started` / `input_batch` 记输入引用,`user_message` 记渲染后的批次内容;旧 Runtime 是 `.meta/round_logs.jsonl`),因为它是审计日志,不是 provider prompt 的规范化形态。

libopendan 的 `DefaultAssembler` 天然满足"一个锚点":同一批次的交接和输入渲染进同一条 `<session_input>`,顺序是 `<behavior_switch>`、`<process_result>`、`<sub_task>`、`<task>` / `<scope>`(仅首个批次)、`<inputs>`、`<changes>`、`<perceptions>`、`<hints>`、`<active_sessions>`、`<runtime>`。

如果 `external_user` 含有图片、文档或其它非纯文本 block,Runtime 不应把它嵌入补充小节,以免破坏结构化内容;此时保留为独立 user message。

### 8.5 Canonical message sequence

当前实现(llm_context + libopendan)的一个可判定的 provider message list 应满足:

```text
system:
  run 的 system(libopendan:Session system text,应用 prompt 按 behavior 条目)

optional user:
  <session_history>(libopendan)

zero or more user:
  注入时最近一个 Step 不属于当前 behavior 的输入批次 <session_input …>

optional user:
  <<step_history>>
  summaries
  其它 behavior 的 StepRecord(<step_record>)
  runtime_history_input
  <</step_history>>

zero or more pairs(当前 behavior 的 Step,按追加顺序):
  assistant: current behavior Step decision
  user:      <<last_step_action_results behavior="..." step="...">> ... <</...>> [+ 之后注入的 <session_input> / <changes>]

optional:
  进行中 Step 的 inner transcript(assistant tool_calls / tool results)
```

不满足这条序列的典型错误包括:

- 跨 behavior 的旧 step 仍作为 assistant/user hot pair 出现在新 behavior 里。
- 子 context 的完整 step stream 被合并回调用方的 context。
- 在已有历史上替换 system 或 `behavior_name` 后继续同一个 run。
- 用户补充被放进 `last_step_action_results` block 内部。
- 同一次交接的用户补充和交接锚点被拆成两条 user message,导致模型先读到一个无结构事实,再读到真正的 continue anchor。

## 九、三种切换模式的典型 message list

下面的例子是当前实现(libopendan Runner + llm_context `XmlStepRenderer` + xllm 的 behavior 解析器)的形状,只展示 Behavior 切换边界附近的 messages,省略模型供应商协议里的 `content[]` 细节,`<session_input>` 省略 `time` 属性。三种模式共享同一条渲染路径(`SessionAssembler::render_input` 渲染输入批次,`LLMContext::inject` 决定位置,`XmlStepRenderer::render_history` 渲染 Step),差别只在这几件事:

- 交接后在哪个 run 上推理(恢复目标自己挂起的 run,还是新建 run),以及它的 system 是谁的
- 新 run 以什么历史开头(无 / `<session_history>` / 调用方的 `<step_record>` / 调用方的完整历史)
- 目标结束后是否返回调用方,结果从哪里回来(交接批次里的 `<process_result>`,或工具结果)
- 交接批次落在 `request.input`(最近一个 Step 不属于当前 behavior),还是并入当前 behavior 最近那个 Step 的 user message

下面把各 context 的 system 写成 `system: <plan 的 system text>` 等。和 [Agent Context Messages.md](../opendan/Agent%20Context%20Messages.md) 对照时注意,那篇按旧 opendan Runtime 描述;它标为"理论上反模式"的普通切换(同一历史流里只换 behavior)在 libopendan 中已移除。

### `switch_context`:各自保有历史的 context

场景:`plan` 正在处理用户请求,中途切到长期存在的 `writer`(`behaviors.writer = { mode: "switch_context", system_prompt: … }`,`inherit` 缺省为 `none`),做完一段 `writer` 工作后再切回 `plan`(`plan` 是 Session 的初始 behavior,隐含为 `switch_context` 目标)。

行为:`suspend_run` 把 `plan` 的 run(run_A)挂起进 `process_stack`(Parked)。栈里没有 `writer` 的 Parked run,于是新建 run_C:`writer` 自己的 system / 工具 / 模型,不复制任何 Step;`inherit: none` 时也不装配 `<session_history>`。

`writer` 首次进入:

```text
system: <writer 的 system text>

user:
  <session_input hook="on_behavior_switch"><behavior_switch to="writer"/></session_input>

assistant:
  <response><actions>…</actions></response>

user:
  <<last_step_action_results behavior="writer" step="0">> … <</last_step_action_results>>
```

注意没有 `<<step_history>>`:run_C 从 step 0 开始编号,`plan` 的 step 不以 `<step_record>` 出现。`writer` 需要了解之前发生了什么时,配置 `inherit: recent_dialogue`,system 之后会多一条 `<session_history>`(含 `plan` 已 flush 的记录,是宿主筛选的视图);更细的共享状态靠工作区产物或读取工具。

`writer` 输出 `<next_behavior>plan</next_behavior>` 后,run_C 挂起进栈(Parked);栈里有 `plan` 的 Parked run,于是恢复 run_A。恢复后的下一次推理看起来就像 `plan` 自己从中断点醒来:

```text
system: <plan 的 system text>

user:
  <session_history>…(run_A 创建时装配的历史)</session_history>

user:
  <session_input hook="on_init">…</session_input>

assistant / user:
  plan 自己的 step 0 … step 1(next_behavior=writer)

user:
  <<last_step_action_results behavior="plan" step="1">>

  <</last_step_action_results>>
  <session_input hook="on_behavior_switch"><behavior_switch to="plan"/></session_input>
```

唯一表明刚才发生过切出的痕迹,是并入 step 1 的交接批次。**`writer` 的 step stream 不会被合并进 `plan` 的 context**;`switch_context` 不交回 report,`writer` 的结果要靠 worklog / 工作区里的产物才能看到。

如果之后再切入 `writer`,会恢复 run_C,看到的是它自己越来越长的 Step 流。多条历史流并行存在,只通过交接批次衔接。`writer` 或 `plan` 输出 `END` / `done` 时按 Session 结束条件处理(Turn completed,再按 `end_condition` 结束 Session 或等待输入),不会自动弹回另一个 context;Session 结束时 `process_stack` 被清空。

### `create_sub_context`:换 system 的子任务

场景:`plan` 完成任务拆解,最后一个 Step 输出 `<next_behavior>do</next_behavior>`;`behaviors.do = { mode: "create_sub_context", system_prompt: …, inherit: "steps" }`。

行为:`suspend_run` 把 `plan` 的 run(run_A)的历史 flush 进 worklog,run_A 作为 Caller 帧入栈(`ChildCall { mode: create_sub_context, behavior: do, trigger: Behavior }`)。`new_run_context` 新建 run_B:`do` 的 system 和配置,`derive_child(.., InheritHistory::Steps)` 带入 run_A 已完成的 Step 和 summaries,编号接着 run_A。

child 启动后的 message list:

```text
system: <do 的 system text>

user:
  <session_input hook="on_behavior_switch"><behavior_switch to="do"/><sub_task mode="create_sub_context">Continue the work handed over to this behavior.
  Your result returns to the caller: finish with your report; do not ask the user.</sub_task></session_input>

user:
  <<step_history>>
  <step_record behavior="plan" index="0" compression="full">
  <observation></observation>
  <thought>The task should be split into doc inspection, edit, and verification.</thought>
  <actions>
  - …(compact action result)
  </actions>
  </step_record>
  <step_record behavior="plan" index="1" compression="full">
  <observation>The edit scope is one Markdown document.</observation>
  <thought>The plan phase is complete; execution should start.</thought>
  <actions>
  No action.
  </actions>
  </step_record>
  <</step_history>>

assistant:
  <response><thinking>I should edit the document now.</thinking><actions>…</actions></response>

user:
  <<last_step_action_results behavior="do" step="2">>
  - #2 …
  <</last_step_action_results>>

assistant:
  <response><report><![CDATA[edit done: X]]></report></response>
```

关键点:

- `plan` 的 step(包括最后一条带 `next_behavior=do` 的 step)以 `<step_record>` 形式进入 `do` 的 `step_history`,**不**作为 `do` 的 hot pair。`<step_record>` 只有 observation / thought / actions 三个槽位(没有 `<thinking>` 时 thought 回退为原始回复文本),没有单独的 `<report>` / `<next_behavior>` 字段。**不**要在 prompt / template 层人为造出跨 behavior 的 hot pair。
- `do` 的第一个 Step 编号接着 `plan`(这里是 2),不从 0 开始;继承的 step 不由 run_B 重复写入 worklog。
- `inherit: none` 时没有 `<<step_history>>`,child 只有 system + 交接批次;`recent_dialogue` 时 system 之后是 `<session_history>`。由 `next_behavior` 触发时 `<sub_task>` 是固定文本,任务要靠继承的历史或共享状态理解;由 `call_behavior` 触发时是调用的 `task` 参数。

最后一个 Step 只有 `<report>`,xllm 的 behavior 解析器把它判为终止(`done`)。`finish_run(run_B)` 写 outcome `process_done`,出栈让 run_A 重新成为 live run,`state.process_result` 记下 child 的结果和 `status`。下一次 drive 循环在 run_A 上提交交接批次;它并入 `plan` 最近那个 Step(带 `next_behavior=do` 的 step 1)的 user message。parent 恢复后的下一次推理:

```text
system: <plan 的 system text>

user:
  <session_history>…(run_A 创建时装配的历史)</session_history>

user:
  <session_input hook="on_init">…</session_input>

assistant:
  <response><actions>…</actions></response>                     (plan step 0)

user:
  <<last_step_action_results behavior="plan" step="0">> … <</last_step_action_results>>

assistant:
  <response><thinking>start doing</thinking><next_behavior>do</next_behavior></response>   (plan step 1)

user:
  <<last_step_action_results behavior="plan" step="1">>

  <</last_step_action_results>>
  <session_input hook="on_behavior_switch"><behavior_switch to="plan"/><process_result behavior="do" status="ok">edit done: X</process_result></session_input>
```

child 的中间 step 不会出现在 run_A 的 prompt 中,只通过 `<process_result>` 交回结果;Session worklog 保留 run_B 的 step(身份 `(run_B, step_index)`),但之后重建的 `<session_history>` 只渲染 run_B 的 `process_done` 结果。run_A 继续使用 child 返回的 `next_step_index` / `next_action_id`,编号不和 child 冲突。整个过程是同一个 Turn:一条 `turn_started` 加两条 `input_batch`。

child 以 `WAIT_USER_MSG` 结束时 `status="needs_user_input"`(结果是它的问题),出错或预算耗尽时 `status="failed"`;都由 `plan` 决定下一步,child 不会直接等用户输入。

### `fork`:保留 system 与完整历史的分支

场景:`plan` 发现一个子任务适合在"同样的上下文"里分支完成,输出 `<next_behavior>research</next_behavior>`;`behaviors.research = { mode: "fork" }`。

行为:入栈与返回同上(`ChildCall { mode: fork, … }`)。run_B 由 `fork_snapshot` 派生:request(system、`behavior_name`、模型)和 `request.input`、`steps`、`last_step`、summaries 都与 run_A 在分叉点时相同,run 记录沿用 run_A 的配置。

child 启动时的 message list 以 run_A 的完整前缀开头,其后才是分支任务:

```text
system: <plan 的 system text>

user:
  <session_history>…(run_A 创建时装配的历史)</session_history>

user:
  <session_input hook="on_init">…</session_input>

assistant / user:
  plan 的 step 0 … step 1(next_behavior=research),与 run_A 中的渲染相同

user:
  <<last_step_action_results behavior="plan" step="1">>

  <</last_step_action_results>>
  <session_input hook="on_behavior_switch"><behavior_switch to="research"/><sub_task mode="fork">Continue the work handed over to this behavior.
  Your result returns to the caller: finish with your report; do not ask the user.</sub_task></session_input>

assistant:
  <response><thinking>Run the research actions.</thinking><actions>…</actions></response>   (分支的 step 2)
…
assistant:
  <response><report><![CDATA[research result X]]></report></response>
```

返回 run_A 的形状与 `create_sub_context` 相同(`<process_result behavior="research" status="ok">research result X</process_result>` 并入 step 1 的 user message)。

**完整历史**:指分叉点的有效 LLM history,包括当时已有的摘要和成对的工具消息;不恢复已经压缩掉的原始历史,也不含未提交的半截输出。只取部分历史、换 system 或重渲染为摘要,都属于 `create_sub_context` 的输入构造。

**隔离边界**:子 context 的 invariant 只在 message list 一层 —— child 的 step 不进调用方的 context。文件系统、worklog、对外发出的消息、session 全局状态,**统统不隔离**。fork 不是沙箱,如果要 dry-run,得在工具 / Runtime 层限制副作用。缓存是否命中取决于实际请求前缀与模型 / 工具定义,模式名称不保证命中。

### 工具触发的子 context(`call_behavior`)

Session 声明了子 context behavior 时,run 的工具集里有 `call_behavior({behavior, task})`(function_call 的原生工具或 behavior 的 action 都可以)。它对 `create_sub_context` / `fork` 目标都成立:

```text
调用方: … [assistant:tool_calls(call_behavior{behavior: research, task}, other)]
        call_behavior → Pending{task_id: "subctx:<call_id>"} → run 以 PendingTool 挂起,整个未完成批次留在快照里
        入栈:Caller + ChildCall { trigger: Tool{call_id, task_id}, task }
child:  按目标模式构造(fork 的分叉点在这个工具批次 / Step 之前) → <sub_task mode="…">task</sub_task> → 推理 → 结果
调用方: … [assistant:tool_calls(call_behavior, other)] [tool:子结果] [tool:other 结果] → 下一 Round,原 Turn 继续
```

子结果用 `ResumeFill::ToolResults` 按 `call_id` 回填(`failed` → 工具错误;`needs_user_input` → 结构化 JSON,提示调用方自己去问用户再重新调用),回填后先发布快照,再继续批次里尚未派发的调用;不产生输入批次。

**三种方式的对照**:

|  | `switch_context` | `create_sub_context` | `fork` |
| --- | --- | --- | --- |
| system | 目标自己的 | 目标自己的 | 与调用方相同 |
| 入口形状 | `[system:B] [<session_history>(inherit: recent_dialogue)] [<session_input on_behavior_switch>]` | `[system:B] [<session_history>(recent_dialogue)] [<session_input … <sub_task>>] [<<step_history>> 调用方 step(inherit: steps)]` | `[调用方在分叉点的完整前缀] [<session_input … <sub_task>>]` |
| 调用方历史 | 不复制 | 显式选择,可不继承 | 分叉点的完整有效历史 |
| 再次进入 | 恢复目标自己挂起的 run | 每次都是新 run | 每次都是新 run |
| 结束 | Session 结束条件,或显式切换 | 结果返回调用方 | 结果返回调用方 |

旧 opendan Runtime 的同名例子(`apply_switch_*`、`.meta/behavior_<entry>.snap`、`<history_input source="process_return:...">`、`END` 弹回上一 process)见第五节末尾的说明,待下一阶段 opendan 重构接入。

## 收束

这三条改动有一个共同的方法论:**好的抽象不是强制选择,而是提供可选维度**。

传统 Loop 的问题不是它选错了,而是它没让你选 —— 工具列表是固定的,结束信号是隐式的,状态机是外挂的。每一个被传统 Loop 焊死的决策,Behavior Loop 都重新打开成了一个可选项。

Behavior Loop 不是一个框架,是一组最小够用的语义槽位。这些槽位让原本需要外部框架才能表达的能力,变成 LLM 输出协议自身的一部分。
