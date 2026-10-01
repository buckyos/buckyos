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

`next_behavior` 不是普通的提示词变化,而是状态机边界。LLM 只在 Step 输出里声明"我要去哪个 behavior";具体怎么切换由 Session 决定。当前实现(libopendan Runner,`SessionAssembler::process_mode`)读 `session_config.extensions.opendan.process_modes.<behavior>`:取值 `"fork"` / `"independent"`,未配置就是 `normal`。

| 模式 | 心智模型 | context / run | 是否继承上一个 behavior 的 Step | 结束语义 |
| --- | --- | --- | --- | --- |
| `normal` | 带历史的跳转 | 同一个 context、同一个 run,只换 `behavior_name` | 继承:上一个 behavior 的 StepRecord 降为 `<<step_history>>` 里的 `<step_record>` | `END` / `done` 是 Session 的 Done |
| `fork` | 带历史的调用,结束后返回 | 当前 run 挂起进 `process_stack`;child 新建 run | 继承:child run 复制 parent 的 `steps`、`history_summaries`、`next_step_index` | child 的 Done(`WAIT_USER_MSG` 除外)一律是返回调用方(`process_done`),结果进 parent 的交接输入批次 |
| `independent` | 切到另一个独立历史流 | 当前 run 挂起进 `process_stack`;目标有挂起的 run 就恢复,否则新建 | 不继承 Step;新建的 run 仍会装配 Session 共享 worklog 的 `<session_history>` | 回到原 process 要显式 `next_behavior` 切回;`END` / `done` 与 `normal` 相同,不会自动弹回 |

三种切换都不结束 Session Turn:切换和 fork 返回都是并入当前 Turn 的交接输入批次(hook `on_behavior_switch`),Turn 只由 Session 在 `finish_run` 中关闭(见 [readme.md](readme.md))。

设计上,切换 Behavior 会同时更换 Work Session 的"头"和"尾":

- 头部更换:新的 system prompt、生效的 process rules、Action 视图和 skills
- 尾部重置:新的 Behavior 只把自己的 Step 渲染成完整的 assistant/user pair

当前 libopendan 只实现了"尾":`XmlStepRenderer` 按 `behavior_name` 区分,当前 behavior 的 Step 渲染成完整 pair,其它 behavior 的 Step 渲染成 `<<step_history>>` 里的 `<step_record>`。system prompt 和工具 / Action 视图来自 Session 配置,不随 behavior 变化;按 behavior 配置 prompt 和 Action 视图(OpenDAN `BehaviorAssembler`)待下一阶段 opendan 重构接入。

因此跨 behavior 继承的历史只能作为系统解释过的 history record 进入新 behavior,不能继续占用新 behavior 的 hot tail。

### `normal`:同一历史流里的跳转

`normal` 是最直接的状态机跳转。`handle_context_outcome` 收到 `Done{next_behavior: B}` 后:

- 用同一个 snapshot `LLMContext::resume(.., ResumeFromMidRun)`,只把 `request.behavior_name` 改成 B;run 保持 Running,没有新 run。
- 保留同一 run 的 `steps`、`history_summaries`、`next_step_index`、`last_report`,`tool_iterations_left` 和 `consecutive_errors` 也不重置,防止 LLM 靠切 behavior 绕过预算和错误上限。
- 带 `next_behavior` 的那个 Step 在 Done 时已经沉淀进 `steps`,切换后和旧 behavior 的其它 Step 一起渲染成 `<step_record>`,新 behavior 没有继承的 hot pair。
- 下一次 drive 循环在同一个 run 上提交 `on_behavior_switch` 输入批次,`<session_input hook="on_behavior_switch">` 里带 `<behavior_switch to="B"/>`。
- 没有"返回调用方"概念。

如果从 `plan` normal 切到 `do`,那么 `plan` 的 StepRecord 会进入 `do` 的 `<<step_history>>`;`do` 自己随后产生的 Step 才作为 `assistant/user` pair 出现在尾部。

### `fork`:继承 history 的子调用

`fork` 是 fork-join 模型:

- `suspend_run` 先把 parent run 到目前为止的历史 flush 进 worklog(outcome `suspended`),run 进 `process_stack`(`ProcessFrame { mode: fork }`),状态 Paused。
- `new_run_context` 为 child 新建 run:system 和 `<session_history>` 照常装配(里面已有 parent flush 的记录),再复制 parent 的 `steps`(含 `last_step`)、`history_summaries`、`next_step_index`、`next_action_id`。`HostMeta.inherited_below` 标出继承的 Step,flush 时不重复写入 worklog。
- child 只有自己的 hot tail;parent 的 Step 在 child 里是 `<step_record>`,不会作为 child 的 hot pair。
- child 的 Done(`WAIT_USER_MSG` 除外)都当作返回:它声明的跳转目标被忽略,`finish_run` 写 outcome `process_done`、出栈让 parent run 重新成为 live run,并把 child 的结果(终止 Step 的 `<report>`,没有就取最后的回答文本)放进 `state.process_result`。
- 下一次 drive 循环在 parent run 上提交交接批次:`<session_input hook="on_behavior_switch">` 里带 `<behavior_switch to="<parent>"/>` 和 `<process_result behavior="<child>">…</process_result>`。parent 从 fork 点之后继续推理。
- child 的 Step 不写回 parent 的 context;Session worklog 记录 child 的过程,Step 身份是 `(child_run_id, step_index)`。

因此 `fork` 和 `normal` 的共同点是"子/目标 behavior 能理解之前发生了什么";区别是 `fork` 有调用栈和返回点,且返回时只把子分支结果汇入 parent,不把 child 的全部执行历史并入 parent 的 context。

### `independent`:独立历史流

`independent` 让每个 process entry 拥有自己的 run 和 Step 流。切换时:

- `suspend_run` 同 fork,frame 标为 independent。
- 栈里有目标 behavior 的 independent 挂起 run 时,恢复它作为 live run:用它自己的 snapshot,`tool_iterations_left` 和错误计数沿用它自己的剩余值,不重置;没有就新建 run(按配置拿到完整额度)。
- 不把 parent 的 `steps`、`history_summaries` 或 hot tail 复制给 target;但新建 run 的 `<session_history>` 来自 Session 共享 worklog,并非完全隔离。
- 每个 run 的 `step_index` 各自编号,在 Session 内不全局唯一。
- 回到原 process 需要 LLM 显式 `next_behavior` 切回;`END` / `done` 按 Session 的 Done 处理(Turn completed,再按 `end_condition` 结束 Session 或等待输入),不会自动弹回上一 process。

所以 `independent` 适合长期并列的独立工作流,不是"带上下文的分支执行"。

> 旧 opendan Runtime(`src/frame/opendan/src/agent_session.rs` 的 `switch_behavior` / `apply_switch_*` / `handle_process_end`)是另一套实现:按 session class 的 `switch_mode` 切换,用 `apply_overrides_to_snapshot` 替换 request 侧(system prompt、tool policy 等),挂起的 process 快照存成 `.meta/behavior_<entry>.snap`,child `END` 时把 report 写成 parent `step_history` 里的 `<history_input source="process_return:...">`,independent 再入时 `reset_tool_iterations` / `reset_errors`,`END` 弹回上一 process、栈空才结束。这些是旧 Runtime 的私有 helper 和磁盘布局,不是新设计接口,待下一阶段 opendan 按 libopendan 的抽象重构接入。

## 六、Step history 和推理输入形态

Behavior Loop 每次推理的输入不是简单 append chat transcript,而是由 `LLMContext::build_inner_request` 现场拼出:request 头部(`request.input`)、`<<step_history>>`、当前 behavior 的 Step pair,以及进行中 Step 的 inner transcript。当前实现(llm_context + libopendan)的 message 序列是:

```text
system: Session system text(libopendan:身份 + 约束 + 应用 prompt + 初始 context + objective,不随 behavior 变化)
optional user: <session_history> ... </session_history>      (libopendan:Session worklog 的摘要 + 最近记录)
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
- 其它 behavior 的 `<step_record>`(跨 behavior 继承)
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

输入批次(包括 `on_behavior_switch` 交接批次)是 synthetic UserMessage,不属于 `step_history`。它的位置由 `LLMContext::inject` 决定:最近一个 Step 属于当前 behavior 时,并入这个 Step 的 user message,接在 `<</last_step_action_results>>` 之后;否则(run 的第一个批次、刚做完 normal 切换、fork child 刚启动)追加到 `request.input`,因此渲染在 `<<step_history>>` 之前:

```text
user:
<session_input hook="on_behavior_switch" time="..."><behavior_switch to="do"/></session_input>

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

完整 pair 只属于当前 behavior:当前 behavior 的所有 Step 按追加顺序都渲染成完整 pair,渲染器不按新旧自动压缩,以保持前缀稳定;压缩只通过显式重写历史发生(例如 `ContextLimitReached` 后 `ResumeFill::RewrittenSteps`)。一旦切到另一个 behavior,这些 Step 在新 behavior 里渲染成 `step_history` 里的 `<step_record>`,并携带至少这些元数据:

```text
behavior_name
step_index
started_at / ended_at(可关闭)
compression_level
```

随后推理产生新 behavior 的第一个 Step 决策(`step_index` 接着编号,不从 0 开始);系统执行它的 actions 后得到 Action Results,这个 Step 成为当前 behavior 的 hot tail。如果之后再发生 Behavior 切换,它会以 `step_record` 或 summary 的形式被继承,而不是继续作为新 Behavior 的完整 assistant/user pair。

## 七、交接输入、fork 返回和真实用户输入的区别

当前实现(libopendan)里,进入 context 的输入都以输入批次提交:`SessionAssembler::render_input`(默认 `DefaultAssembler`)按 `InputMaterial` 渲染一条 `<session_input hook=… time=…>` user message,`commit_input_batch` 把它注入 context 并写 receipt `(run_id, input_seq)`。批次按来源区分:

- 真实用户 / peer message、业务 event:hook `on_wakeup`(Session 的第一个批次是 `on_init`),消息放在 `<inputs>` 里,同一批次还可以带 `<changes>`、`<hints>`、`<active_sessions>`、`<runtime>`。没有打开的 Turn 时它开启新 Turn,否则并入当前 Turn。
- behavior 切换:hook `on_behavior_switch`,带 `<behavior_switch to="…"/>`。它不是用户真实发来的消息,并入当前 Turn。
- fork child 返回:同样是 `on_behavior_switch` 批次,在 parent run 上提交,带 `<process_result behavior="…">…</process_result>`。它是输入批次,不进 `<<step_history>>`。
- 观察边界的半订阅变化:`CheckpointHook` 在 Step 边界注入 `<changes>`(receipt hook `observation`),不开 Turn。

同一次 drive 循环里既有交接又有新消息时,它们渲染进同一条 `<session_input>`(`<behavior_switch>` / `<process_result>` 在前,`<inputs>` 在后),provider message list 里只有一个交接锚点。

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
| `process_stack` | fork / independent 挂起的 process(libopendan `SessionState.process_stack`,每帧是一个挂起的 run) | 不直接渲染,决定切回哪个 run |

`step_index` 是一个 run 的 Step 流里按分配顺序递增的序号(解析失败 / 策略拒绝产生的合成纠错 Step,即 `StepRecord::is_correction()`,也占一个号),不是每个 Turn 或每个 behavior 从 0 重开。`fork` child 继承 parent 的 `next_step_index`,所以 child 产生的第一条新 step 不从 0 开始;independent 新建的 run 从 0 编号。Step 身份是 `(run_id, step_index)`,`step_index` 在 Session 内不全局唯一。

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

这个空 block 表示"step 3 的 actions 已经观察完毕,结果为空",不是用户输入,也不是 fork/on_behavior_switch 的载体。真实用户补充不应该塞进这个 block:当前实现把之后注入的输入批次作为同一条 user message 的后续 content,接在 `<</last_step_action_results>>` 之后。

### 8.3 输入来源分类

进入 context 的内容按来源分成这几类(表中"模板"指旧 opendan Runtime 的 behavior 模板;libopendan 的对应见表后):

| 类别 | 例子 | 语义归属 | 渲染位置 |
| --- | --- | --- | --- |
| `runtime_step_result` | action 执行结果 | 上一个 Step 的 observation | `last_step_action_results` |
| `runtime_history_input` | fork child END、process-end marker | 已完成 runtime 状态变更 | `step_history` 内 |
| `runtime_auto_user` | `on_behavior_switch` / `on_init` 模板输出 | Runtime 生成的继续执行指令 | 本次输入批次的 tail |
| `external_user` | 用户消息、forwarded usermsg | 新事实 / 新约束 / 人类补充 | 本次输入批次的 tail,或嵌入 runtime tail 的补充小节 |
| `external_event` | kevent / msg event | 外部事件唤醒 | 本次输入批次的 tail |

libopendan 当前的对应:`runtime_step_result` 相同;`runtime_history_input` 不出现(libopendan 不挂 `StepResultHook`,不产生 `history_inputs`),fork 返回改为输入批次里的 `<process_result>`;`runtime_auto_user` 是 `<session_input hook="on_init" | "on_behavior_switch">` 里的 `<task>` / `<behavior_switch>`;`external_user` / `external_event` 是同一条 `<session_input>` 里的 `<inputs>`;半订阅变化在 Step 边界以 `<changes>` 注入。

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

libopendan 的 `DefaultAssembler` 天然满足"一个锚点":同一批次的交接和输入渲染进同一条 `<session_input>`,顺序是 `<behavior_switch>`、`<process_result>`、`<task>` / `<scope>`(仅首个批次)、`<inputs>`、`<changes>`、`<perceptions>`、`<hints>`、`<active_sessions>`、`<runtime>`。

如果 `external_user` 含有图片、文档或其它非纯文本 block,Runtime 不应把它嵌入补充小节,以免破坏结构化内容;此时保留为独立 user message。

### 8.5 Canonical message sequence

当前实现(llm_context + libopendan)的一个可判定的 provider message list 应满足:

```text
system:
  run 的 system(libopendan:Session system text)

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
- fork child 的完整 step stream 被合并回 parent 的 context。
- 用户补充被放进 `last_step_action_results` block 内部。
- 同一次交接的用户补充和交接锚点被拆成两条 user message,导致模型先读到一个无结构事实,再读到真正的 continue anchor。

## 九、三种切换模式的典型 message list

下面的例子是当前实现(libopendan Runner + llm_context `XmlStepRenderer` + xllm 的 behavior 解析器)的形状,只展示 Behavior 切换边界附近的 messages,省略模型供应商协议里的 `content[]` 细节,`<session_input>` 省略 `time` 属性。三种模式共享同一条渲染路径(`SessionAssembler::render_input` 渲染输入批次,`LLMContext::inject` 决定位置,`XmlStepRenderer::render_history` 渲染 Step),差别只在三件事:

- 切换后在哪个 run 上推理(同一个 run、新建 run,还是恢复挂起的 run)
- `step_history` 里有没有上一个 behavior 的 `<step_record>`
- 交接批次落在 `request.input`(最近一个 Step 不属于当前 behavior),还是并入当前 behavior 最近那个 Step 的 user message

system 在三种模式下都是同一段 Session system text,下面写成 `system: <Session system text>`。和 [Agent Context Messages.md](../opendan/Agent%20Context%20Messages.md) 对照时注意,那篇按旧 opendan Runtime 描述。

### `normal`:同一历史流里的轻量跳转

场景:`plan` 完成任务拆解,最后一个 Step 输出 `<next_behavior>do</next_behavior>`;`do` 没有配置 `process_modes`,按 normal 切换。

行为:`handle_context_outcome` 在同一个 run 上把 `behavior_name` 改成 `do`;`steps` / `history_summaries` / `next_step_index` / `next_action_id` / `last_report` 全部保留,`tool_iterations_left` 和 `consecutive_errors` 不重置。下一次 drive 循环提交 `on_behavior_switch` 批次;此时最近的 Step 属于 `plan`,批次追加到 `request.input`。

切到 `do` 后的下一次推理:

```text
system: <Session system text>

user:
  <session_history>…</session_history>                       (有历史时)

user:
  <session_input hook="on_init"><task>Start working on the objective of this session.</task>…</session_input>

user:
  <session_input hook="on_behavior_switch"><behavior_switch to="do"/></session_input>

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
```

关键点:

- `plan` 的全部 step(包括最后一条带 `next_behavior=do` 的 step)以 `<step_record>` 形式进入 `do` 的 `step_history`,**不**作为 `do` 的 hot pair。`<step_record>` 只有 observation / thought / actions 三个槽位(没有 `<thinking>` 时 thought 回退为原始回复文本),没有单独的 `<report>` / `<next_behavior>` 字段。
- `do` 的第一个 Step 编号接着 `plan`(这里是 2),不从 0 开始。
- 没有 `process_stack` push,没有新 run;Session worklog 写一条 `input_batch`(hook `on_behavior_switch`),Turn 不变。

[Agent Context Messages.md](../opendan/Agent%20Context%20Messages.md#状态机切换仅限-behavior-loop) 把普通切换标为"理论上反模式",担心的形状是:`plan` 的最后一条 step 作为 hot pair 暴露在 `do` 的尾部,形成跨 behavior 的 `[assistant: next_behavior=do → user: do.on_switch]` 边界 pair。当前渲染器按 `behavior_name` 把其它 behavior 的 Step 一律降为 `<step_record>`,正好回避了这个形状,所以 `normal` 在实现里是"看似反模式但被规范化掉的可用模式",**不**要在 prompt / template 层再人为造出那种跨边界 hot pair。

### `fork`:带历史的子调用

场景:parent behavior `plan` 发现一个子任务可以交给 child behavior `research`(`process_modes.research = "fork"`),child 完成后把结果交回 parent。

行为:`suspend_run` 把 parent run(run_A)的历史 flush 进 worklog,run_A 进 `process_stack` 并 Paused;`new_run_context` 新建 run_B,复制 run_A 的 `steps`(含 `last_step`)、`history_summaries`、`next_step_index`、`next_action_id`。run_B 的 `<session_history>` 是新装配的,已经包含 run_A flush 的记录。

child 启动时的典型 message list:

```text
system: <Session system text>

user:
  <session_history>
  …
  [step 0 plan] …
  [step 1 plan] …
  [outcome suspended → research]
  </session_history>

user:
  <session_input hook="on_behavior_switch"><behavior_switch to="research"/></session_input>

user:
  <<step_history>>
  <step_record behavior="plan" index="0" compression="full">…</step_record>
  <step_record behavior="plan" index="1" compression="full">…</step_record>
  <</step_history>>

assistant:
  <response><thinking>Run the research actions.</thinking><actions>…</actions></response>

user:
  <<last_step_action_results behavior="research" step="2">>
  - #2 …
  <</last_step_action_results>>

assistant:
  <response><report><![CDATA[research result X]]></report></response>
```

最后一个 Step 只有 `<report>`,xllm 的 behavior 解析器把它判为终止(`done`);child 的任何 Done(`WAIT_USER_MSG` 除外)都返回调用方。`finish_run(run_B)` 写 outcome `process_done`,出栈让 run_A 重新成为 live run,`state.process_result` 记下 child 的结果。下一次 drive 循环在 run_A 上提交交接批次;`plan` 已经有 Step,批次并入它最近那个 Step(带 `next_behavior=research` 的 step 1)的 user message。parent 恢复后的下一次推理:

```text
system: <Session system text>

user:
  <session_history>…(run_A 创建时装配的历史)</session_history>

user:
  <session_input hook="on_init">…</session_input>

assistant:
  <response><actions>…</actions></response>                     (plan step 0)

user:
  <<last_step_action_results behavior="plan" step="0">> … <</last_step_action_results>>

assistant:
  <response><thinking>need research</thinking><next_behavior>research</next_behavior></response>   (plan step 1)

user:
  <<last_step_action_results behavior="plan" step="1">>

  <</last_step_action_results>>
  <session_input hook="on_behavior_switch"><behavior_switch to="plan"/><process_result behavior="research">research result X</process_result></session_input>
```

child 的中间 step 不会出现在 run_A 的 prompt 中,只通过 `<process_result>` 交回结果;Session worklog 保留 run_B 的 step(身份 `(run_B, step_index)`,继承来的 step 不重复写入)。run_A 继续使用 child 返回的 `next_step_index` / `next_action_id`,编号不和 child 冲突。整个过程是同一个 Turn:一条 `turn_started` 加两条 `input_batch`。

**继承粒度**:child 看到的 `<step_record>` 字段不是 0/1。设计上 fork 可以选择只继承到上一次 behavior 边界、只继承 `thought + next_behavior` 骨架、只继承每段的 `self_report`,甚至完全不继承(只靠交接批次传共享状态)。粒度越薄,child 的 context window 越轻;粒度越厚,child 越能理解 parent "为什么走到这一步"。当前 libopendan 固定复制 parent 的全部 Step,粒度选项未实现。

**隔离边界**:fork 的 invariant 只在 message list 一层 —— child 的 step 不进 parent 的 context。文件系统、worklog、对外发出的消息、session 全局状态,**统统不隔离**。fork 不是沙箱,如果要 dry-run,得在 child 的 system prompt / Action 视图层自己造隔离,Runtime 不负责。

**和 independent 首次进入的形状对照**:

|  | fork child 入口 | independent 首次进入 |
| --- | --- | --- |
| 形状 | `[system] [<session_history>] [<session_input on_behavior_switch>] [<<step_history>> 继承 parent step]` | `[system] [<session_history>] [<session_input on_behavior_switch>]` |
| `step_history` 是否含 parent step | 是 | 否(parent 的过程只以 worklog 记录出现在 `<session_history>` 里) |
| child 的 step 流向 | 写进 worklog;返回后 child run 结束,不再进入 | 留在自己挂起的 run 里,可再入 |
| 再次进入 | 每次都是新 run | 恢复自己上次挂起的 run |

结构相近,所有权完全不同。

### `independent`:独立历史流

场景:`plan` 正在处理用户请求,中途切到长期存在的 `writer` process(`plan`、`writer` 都配置为 `independent`),做完一段 `writer` 工作后再切回 `plan`。

行为:`suspend_run` 把 `plan` 的 run(run_A)挂起进 `process_stack`。栈里没有 `writer` 的挂起 run,于是新建 run_C:不复制任何 Step,只装配 system 和当前的 `<session_history>`。

`writer` 首次进入:

```text
system: <Session system text>

user:
  <session_history>…(含 plan 已 flush 的记录)</session_history>

user:
  <session_input hook="on_behavior_switch"><behavior_switch to="writer"/></session_input>

assistant:
  <response><actions>…</actions></response>

user:
  <<last_step_action_results behavior="writer" step="0">> … <</last_step_action_results>>
```

注意没有 `<<step_history>>`:run_C 从 step 0 开始编号,`plan` 的 step 不以 `<step_record>` 出现。

`writer` 输出 `<next_behavior>plan</next_behavior>` 后,run_C 挂起进栈;栈里有 `plan` 的 independent 挂起 run,于是恢复 run_A。恢复后的下一次推理看起来就像 `plan` 自己从中断点醒来:

```text
system: <Session system text>

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

唯一表明刚才发生过切出的痕迹,是并入 step 1 的交接批次。**`writer` 的 step stream 不会被合并进 `plan` 的 context**;independent 切换不交回 report,`writer` 的结果要靠 worklog / 工作区里的产物,或 `plan` 下次新建 run 时的 `<session_history>` 才能看到。

如果之后再切入 `writer`,会恢复 run_C,看到的是它自己越来越长的 Step 流。多条历史流并行存在,只通过交接批次衔接。`writer` 或 `plan` 输出 `END` / `done` 时按 Session 的 Done 处理(Turn completed,再按 `end_condition` 结束 Session 或等待输入),不会自动弹回另一个 process;Session 结束时 `process_stack` 被清空。

旧 opendan Runtime 的同名例子(`apply_switch_*`、`.meta/behavior_<entry>.snap`、`<history_input source="process_return:...">`、`END` 弹回上一 process)见第五节末尾的说明,待下一阶段 opendan 重构接入。

## 收束

这三条改动有一个共同的方法论:**好的抽象不是强制选择,而是提供可选维度**。

传统 Loop 的问题不是它选错了,而是它没让你选 —— 工具列表是固定的,结束信号是隐式的,状态机是外挂的。每一个被传统 Loop 焊死的决策,Behavior Loop 都重新打开成了一个可选项。

Behavior Loop 不是一个框架,是一组最小够用的语义槽位。这些槽位让原本需要外部框架才能表达的能力,变成 LLM 输出协议自身的一部分。
