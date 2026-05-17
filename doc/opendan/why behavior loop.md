# 我们为什么做了 Behavior Loop

Agent Loop 这一层的设计,大部分实现共享着几个没被质疑过的假设:工具列表是一个固定集合、Loop 的结束由模型不返回 tool call 来隐式判断、状态机要么不存在要么是外挂框架。

这些假设在短任务里没问题。但任何做过 30 轮以上长任务的人都知道,这一层的协议是有结构性缺陷的 —— 不是某个具体实现不够好,是协议本身没给一些必要的语义留位置。

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

**输出方向**:LLM 不返回 tool call 就算结束了。但"它结束了"和"它觉得自己应该结束"是两回事 —— 前者是隐式推断,后者才是意图。中间断了你不知道是真的完成了还是只是这一轮没调工具,恢复的时候只能把整个历史重新喂回去让 LLM 自己判断"我刚才到哪了"。这本质上是把**调度器的状态**藏在了**模型的注意力**里 —— 一个无状态系统假装自己有状态。

**输入方向**:LLM 在第 5 轮 tool call 时,它不知道自己处于什么意图阶段 —— 还在探索?在收敛?在等待用户?Message array 没给它这个信号,只能从历史里猜。

Behavior Loop 的 Step Schema 强制每次输出都 commit 一个意图状态:

```
Step:
  结论:   上一步动作的结果观察
  思考:   当前的推理
  动作:   要执行什么
  next_behavior: 留空(继续) 或 跳转目标(显式结束当前 Behavior)
```

这四个槽位每个都是双向意图通道 —— 既是 LLM 告诉调度器"我处于什么阶段",也是调度器和后续 Step 读到"上一步 LLM 处于什么阶段"。

有了这个基础,前两条才有放置的位置:Action 视图能按 Behavior 切换,是因为 `next_behavior` 让 Behavior 边界变得显式;状态机能内生于 Loop,是因为 Step 本身就是状态迁移的最小单元。

---

## 四、History、Attention 和 KV Cache 的取舍

Behavior Loop 不是 Chat Message Loop。它更接近一个 Work Session:围绕明确 Objective 持续推进,完成后结束。因此它的历史策略不追求无限累积对话,而是优先保证每轮推理时关键信息落在 LLM attention 的"U 型区域"两端:

- 头部:稳定的 system prompt,包含 objective、process rules、result protocol、当前 Behavior 暴露的 Action 视图和 skills
- 尾部:最近若干个完整 StepRound,也就是 LLM 上一步输出的 Intent 和系统执行后的 Action Results

这和 KV Cache 的最优命中天然存在张力。为了让旧历史逐渐从中部让位给新的完整 StepRound,历史会发生压缩;一旦压缩发生,严格的长前缀 cache 命中会被破坏。这个代价是有意接受的:对 Work Session 来说,让当前推理看到正确的任务头部和最近执行尾部,比维持一条永远 append-only 的 Chat transcript 更重要。

Behavior Loop 的压缩分两层。

第一层是常规的 StepRecord 分级压缩。StepRecord 仍然保留结构化语义,但历史 step 的 detail 会随着它滑入 context 中部而逐渐消失。旧 step 可以从完整的:

```
assistant: Step Intent
user:      Step Action Results
```

降级为更短的 compact record。这样做的效果是:某个中部 StepRecord 被压缩后,它后面一段历史的 detail 可能都会被重新布局,但系统因此又为未来几个 StepRound 腾出尾部空间,让新的 Intent + Action Results 可以完整进入模型输入。

第二层是触顶后的强制有损压缩。它不是普通的 compact render,而是把一批旧 StepRecord 折叠成固定大小的 History Summary 块:

- 不再保留原始 Step 结构
- 记录被压缩的 step 数量
- 记录起止 step index、起止时间戳、所属 behavior 范围
- 摘要这批 step 大致完成了什么、留下了什么约束或结论

这层压缩是最后手段。它的目的不是让模型完整复盘每个动作,而是在 context window 快触顶时重新制造一个稳定的历史前缀,让后续 N 个 StepRound 可以继续以尽量少破坏 KV Cache 的方式运行。

因此,Behavior Loop 的历史不是"越完整越好",而是按位置和阶段承担不同职责:

- 当前 Behavior 的最近 StepRound:完整、强可见、位于尾部
- 当前 Behavior 的较旧 StepRecord:结构化但分级压缩
- 跨 Behavior 继承的旧历史:必须降级为系统可解释的 history record 或 summary,不能继续占用当前 Behavior 的 hot tail
- 触顶后的长期历史:固定大小的 summary block

## 五、Behavior 切换和 fork 语义

`next_behavior` 不是普通的下一轮提示词变化,而是状态机边界。切换 Behavior 时,等价于同时更换 Work Session 的"头"和"尾":

- 头部更换:新的 system prompt、生效的 process rules、Action 视图和 skills
- 尾部重置:新的 Behavior 开始自己的最近 StepRound hot area

这意味着 Step history 必须是 session 级别的,但每个 StepRecord 需要能被系统解释清楚它属于哪个 Behavior。设计要求是:StepRecord 或其外层 envelope 至少携带:

```
behavior_name
step_index
started_at / ended_at
compression_level
```

其中 `behavior_name` 是跨 Behavior 继承历史的关键。一个 step 在自己的 Behavior 中可以作为完整 StepRound 出现在尾部;一旦切换到另一个 Behavior,它就不应再以"当前轮 assistant Intent + user Action Results"的热区形式出现。新 Behavior 只能通过系统解释过的 history record 理解它:

```
history step record:
  behavior: plan
  index: 12
  summary/detail: ...
  result: ...
```

换句话说,完整的:

```
assistant: Step Intent
user:      Step Action Results
```

只属于当前 Behavior 的最近执行上下文。跨 Behavior 继承的历史必须变成单条 StepRecord message 或 summary block,由当前 Behavior 的 system prompt 解释其含义。这一点很重要:切换 Behavior 必然导致 system prompt 和 skills 重新匹配,也必然造成 KV Cache miss;这个 miss 是可接受的。但切换后不能把旧 Behavior 的 hot tail 原样带到新 Behavior 尾部,否则新 Behavior 会在错误的语义框架下读取旧 Intent。

fork 模型也是同一个原则。fork 出来的子上下文可以继承 parent 的 session history,但继承的是已经解释过的 StepRecord / History Summary,不是 parent 当前 Behavior 的完整热区。子上下文有自己的 system prompt、Action 视图和 hot tail;它运行结束后,结果再以 report、summary 或 join record 的形式回到 parent。

## 六、推理输入形态

从一次 Behavior step 的 LLM 输入看,理想结构是:

```
- system: current behavior objective + process_rules + action view + skills + result_protocol
- optional user: real user/event input with background environment
- optional user/assistant: history summary blocks produced by hard compression
- user: inherited StepRecord history from previous behaviors, already interpreted/compressed
- assistant/user pairs: current behavior recent full StepRounds
  - assistant Step -2 Intent
  - user      Step -2 Action Results
  - assistant Step -1 Intent
  - user      Step -1 Action Results
```

推理后得到:

```
assistant Step 0 Intent
```

系统执行 Step 0 actions 后得到:

```
user Step 0 Action Results
```

随后 Step 0 进入当前 Behavior 的 hot tail。再往后,它会逐渐进入当前 Behavior 的 StepRecord history;如果发生 Behavior 切换,它必须以带 `behavior_name` 和 `step_index` 的 history record 形式被继承,而不是继续作为新 Behavior 的完整 assistant/user hot round。


## 收束

这三条改动有一个共同的方法论:**好的抽象不是强制选择,而是提供可选维度**。

传统 Loop 的问题不是它选错了,而是它没让你选 —— 工具列表是固定的,结束信号是隐式的,状态机是外挂的。每一个被传统 Loop 焊死的决策,Behavior Loop 都重新打开成了一个可选项。

Behavior Loop 不是一个框架,是一组最小够用的语义槽位。这些槽位让原本需要外部框架才能表达的能力,变成 LLM 输出协议自身的一部分。
