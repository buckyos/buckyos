# Self Improve 设计的深度思考

> 本文是基于一段连续语音思考整理出来的设计文档。它不是最终规格书，也不是实现手册，而是对 Self Improve 这个 Agent 元能力的结构化推导。文中会保留一些尚未完全收敛的概念、设计假设和开放问题。

---

## 0. 摘要

Self Improve 不是针对某一个具体问题的优化流程，而是一种 Agent 的基础元能力。

它的核心不是“把 Session 总结成 Memory”，也不是“构造一个严格知识图谱”，而是让 Agent 能够从自己持续参与世界的历史中，逐渐回答三个问题：

1. **哪些事情与我和我的主人有关？**
2. **当这些事情再次出现时，我有没有更好的观察、思考和行动捷径？**
3. **我应该朝什么方向持续改进自己？**

因此，Self Improve 可以暂时分成三层：

```text
Level 1: Attention Graph
    发现事件、强化对象、主动探索，建立以主人为中心的世界参与图。

Level 2: Skill / Shortcut Graph
    发现、安装、验证、升级、淘汰各种观察世界和改变世界的捷径。

Level 0: Self-Thinking / Value System
    定义什么叫“更好”，并基于 Agent 对自身的判断生成新的自我改进任务。
```

这里把第三层叫 Level 0，是因为它在执行顺序上可能后出现，但在因果关系上更像元层：它决定了哪些改进值得做，什么样的 Skill 更好，以及 Agent 想成为怎样的 Agent。

当前最明确、最适合优先落地的是 **Level 2: Skill / Shortcut 管理机制**。Level 1 是它的输入基础，Level 0 暂时作为长期演化的架构占位。

---

## 1. 背景：Self Improve 要解决什么问题

传统 Agent 的工作方式通常是输入驱动的：

```text
用户消息 / 系统事件 / 计划任务
        ↓
Agent Session
        ↓
执行任务
        ↓
产生结果
```

这种模式可以处理明确问题，但有几个缺陷：

1. **Agent 很难知道哪些事情正在变得重要。**  
   单次 Session 里出现的一句话可能没有意义，但跨多个 Session 后，它可能代表一个长期事件正在形成。

2. **Agent 很难记住“处理某类事情的捷径”。**  
   每次遇到类似问题都从搜索开始、从头推理，浪费时间，也容易重复犯错。

3. **Agent 很难知道自己“不知道什么”。**  
   用户做计划时可能因为知识盲区没有提出关键问题，例如国家公园预约政策、季节性道路关闭、签证限制、平台流程变化等。

4. **Agent 很难判断什么叫改进。**  
   更快、更便宜、更可信、更能赚钱、更符合主人偏好，这些目标之间可能冲突。

Self Improve 的设计目标，就是让 Agent 不只是被动响应，而是能够在历史中形成经验、在经验中形成捷径、在捷径中形成能力，并在更高层次上逐渐理解自己应该如何改进。

---

## 2. 核心抽象：Session History + Memory Graph → 新的 Memory Graph

从计算结构上看，Self Improve 可以抽象为：

```text
G_current + H_unprocessed → G_next
```

其中：

- `G_current`：当前 Agent 已经形成的 Memory / Attention / Skill 图。
- `H_unprocessed`：还没有被 Self Improve 处理过的 Session History。
- `G_next`：经过整理、强化、合并、淘汰、更新之后的新图。

进一步扩展后，可以写成：

```text
SelfImprove(
    current_memory_graph,
    unprocessed_session_history,
    skill_usage_history,
    active_exploration_results,
    self_thinking_state
) -> updated_agent_state
```

Self Improve 的结果不是一段总结，而是一组结构变化：

- 新增事件；
- 新增对象；
- 新增观察；
- 增强对象权重；
- 建立对象关系；
- 合并重复节点；
- 标记失效 Skill；
- 提升或降低某个 Skill 的排名；
- 生成新的主动探索任务；
- 生成新的自我改进任务。

---

## 3. Self Improve 与知识图谱的关系

Self Improve 的结果在数学结构上可以近似看成一张图，但它不是严格意义上的知识图谱。

知识图谱通常要求：

- 节点定义清楚；
- 边定义清楚；
- 关系类型严谨；
- 插入节点和边时有明确 schema；
- 尽量避免矛盾。

Self Improve 形成的是 Agent 自己的经验图。它允许：

- 模糊关系；
- 临时判断；
- 自然语言备注；
- 未完全定义的对象；
- 互相矛盾的观点；
- 多种解释并存；
- 尚未验证的线索；
- 主观标注和可信度差异。

可以认为：

> 一个高质量、人工编辑、结构严谨的知识图谱，通常可以恢复出 Self Improve 需要的一部分结果；但 Self Improve 的结果不保证能够 100% 转换成严格知识图谱。

原因是 Agent 的 Memory 更接近人的记忆和理解结构，而不是数据库式事实结构。

Agent 的经验图里既有对象，也有事件、备注、观点、观察、Skill、Principle 和注意力权重。它是一种动态、带时间维度、允许矛盾和遗忘的图。

---

## 4. 从静态世界到 Attention Graph

如果没有事件视角，世界可以被理解成一个静态对象集合：

```text
Object
Object
Object
Object
```

对象之间可能有关系：

```text
Object A -- Relation --> Object B
```

但 Agent 真正活在世界里，不是通过静态对象模型，而是通过 Session 参与世界。

Session 代表 Agent “活着”的过程。每一次 Session 都是在时间轴上发生的参与行为：

```text
Session 1
Session 2
Session 3
Session 4
...
```

Self Improve 的第一层任务，就是把这些离散 Session 串成一条全局时间轴，并观察：

> 我和我的主人，是如何参与到这个世界里的？

于是静态的对象图会变成带时间维度的 Attention Graph：

```text
Object
  ↑
Event
  ↓
Object

Observation
  ↓
Attention Weight
  ↓
Active Exploration
```

Attention Graph 不是描述“世界客观上是什么”，而是描述：

> 在 Agent 看来，哪些事情和对象正在与自己、主人和当前生活发生关系。

这是 Owner-Centric 的图，而不是 Global-Centric 的图。

---

## 5. Level 1：Attention Graph 的建立

Level 1 是 Self Improve 的第一层探索。它的核心目标是：

> 建立一张以主人为中心、随时间演化的 Attention Graph。

它包含三种第一层原能力：

```text
1. Event Discovery
   发现正在发生的事情。

2. Object Reinforcement
   强化被卷入事件的对象。

3. Active Exploration
   对高关注对象和事件主动创建探索任务。
```

---

### 5.1 Event Discovery：发现事件

Self Improve 首先要从 Session History 中提炼出“事件”。

这里的事件不是程序里的事件驱动概念，不是 click、message、timer，而是：

> 一件真实发生或正在发展的事情。

例如：

```text
LZC 正在研究 2026 年暑假出游。
OpenDAN 的 Agent Session 架构正在持续演化。
用户最近多次讨论 Work Session 和 UI Session 的边界。
某个工具在多个任务中被错误调用。
```

事件不是事实。事件带有时间维度、参与对象和演化过程。

#### 示例：2026 暑假出游

Agent 通过历史 Session 发现：

```text
LZC 正在研究 2026 年暑假出游。
```

这个时候可以产生一个 Event：

```text
Event: 2026 暑假出游
```

最初它可能只关联一个人：

```text
LZC
```

随着讨论继续，它会逐渐关联更多对象：

```text
妻子
儿子
国家公园 A
国家公园 B
预算表
照片文件夹
酒店
航班
游记
消费记录
```

这个 Event 可能经历多个阶段：

```text
规划 → 家庭讨论 → 预订 → 实际出行 → 照片整理 → 花费统计 → 游记沉淀 → 后续回忆
```

因此，Event 本质上是一个能够容纳子图的语义集合体：

```text
Event
 ├─ Person
 ├─ Place
 ├─ File
 ├─ Decision
 ├─ Observation
 ├─ Expense
 └─ Follow-up
```

它不是普通节点，而更像一个虚拟主体、语义容器或局部子图。

---

### 5.2 Object Reinforcement：强化对象

当对象被卷入事件之后，Agent 会不断给对象增加观察、属性、备注和关系。

这里的重点不是增加客观事实，而是增加 Agent 观察到的东西。

例如，在“2026 暑假出游”事件中讨论三个国家公园：

```text
国家公园 A
国家公园 B
国家公园 C
```

Session History 里可能出现：

```text
妻子觉得国家公园 A 太干。
儿子觉得国家公园 A 有点无聊。
LZC 喜欢国家公园 A，因为那里适合观星。
```

这些内容会强化对应对象：

```text
Object: 国家公园 A
  Observation:
    - 妻子不喜欢太干燥的地方，因此对这里兴趣较低。
    - 儿子可能觉得这里不够好玩。
    - LZC 对这里的夜景和观星条件感兴趣。
```

此时，国家公园 A 不再只是搜索引擎里的一个地点，而变成了 Agent 世界中的对象。

它被赋予了主人家庭语境中的意义。

---

### 5.3 Attention Rank：关注对象列表

Agent 不可能对整个世界保持同等注意力。

因此，它需要维护一个动态关注对象集合：

```text
TopObjects(Agent)
```

这不是世界上最重要的对象，而是：

> 在 Agent 当前视角下，与自己和主人最相关的对象。

例如：

```text
LZC
妻子
儿子
OpenDAN
Self Improve
Agent Session
2026 暑假出游
国家公园 A
```

每个对象都有 Attention Weight。权重来源可能包括：

- 最近是否被提及；
- 被提及次数；
- 是否参与高关注事件；
- 是否产生新的观察；
- 是否和高权重对象有关系；
- 是否触发过成功或失败的 Work Session；
- 是否被用户显式反馈为重要。

权重也需要自然衰减：

```text
Attention(t+1) = Attention(t) * Decay + NewSignals
```

一个对象长期不被提及，就逐渐被遗忘；一个对象反复出现，就进入更高关注区。

---

### 5.4 一跳、二跳与扩散

Attention Graph 需要区分对象与 Agent 的距离。

例如：

```text
LZC ↔ 2026 暑假出游
```

这是一跳，因为用户直接在研究这个事件。

如果妻子对这次出游表达了观点：

```text
妻子 ↔ 2026 暑假出游
```

这也是一跳。

但如果出游计划中只是顺带提到朋友 X 要参加，而 Agent 没有关于 X 的任何其他观察，那么：

```text
Agent → 2026 暑假出游 → 朋友 X
```

朋友 X 对 Agent 来说更像二跳对象。

这个区分很重要，因为它决定召回、主动探索和 Skill 加载时的优先级。

可以有多层关注网络：

```text
TopObjects(Agent)
TopObjects(LZC)
TopObjects(2026 暑假出游)
TopObjects(国家公园 A)
```

如果每层取 32、64 或 100 个对象，三层扩散就已经形成一个很大的局部网络。因此必须有 Top-N、衰减和边界限制。

---

### 5.5 Active Exploration：主动探索

Self Improve 不应只整理 Session History。

当某些对象或事件的 Attention 足够高时，Agent 可以主动创建 Subtask，去研究 Session History 中没有显式提供的信息。

例如，用户正在讨论国家公园出游，但没有问：

```text
这个国家公园是否需要预约？
夏季道路是否开放？
是否有山火风险？
是否适合带小孩？
是否有车辆限流？
```

这些可能是用户的知识盲区。用户不知道，就不会问。

Agent 可以基于 Attention Rank 判断：

```text
2026 暑假出游热度很高。
国家公园 A、B、C 被多次提到。
这些对象值得主动研究。
```

于是创建 Subtask：

```text
Research: 国家公园 A 的当前开放状态、预约政策、道路关闭、家庭游玩注意事项。
```

Active Exploration 不是全图扩散，而是只针对高 Attention 对象和事件进行。否则会发生组合爆炸。

它会形成一个反馈循环：

```text
对象被频繁提及
    ↓
Attention Rank 上升
    ↓
触发主动探索
    ↓
产生更多观察
    ↓
对象变得更丰富
    ↓
Attention Rank 进一步上升
```

这会让某些事情在短时间内变得非常热。事件结束后，随着时间流逝和提及减少，它又会逐渐冷却。

---

## 6. Level 2：Skill / Shortcut Graph 的建立

如果 Level 1 回答的是：

> 哪些事情与我有关？

那么 Level 2 回答的是：

> 下次遇到类似事情时，我能不能做得更快、更稳、更便宜、更可信？

Skill 在这里不是传统意义上的插件，也不只是工具调用。更准确地说：

> Skill 是 Agent 与世界交互的捷径。

Agent 的基本循环是：

```text
Observe → Think → Act
```

因此 Skill 可以是：

```text
Observation Shortcut
Thinking Shortcut
Action Shortcut
```

它可以帮助 Agent：

- 更快观察世界；
- 更快获得可信数据；
- 更好地思考和组织任务；
- 更稳定地执行真实世界流程；
- 更少重复犯错；
- 更少依赖从头搜索和从头推理。

---

### 6.1 为什么 Skill 必须依赖 Attention

如果没有 Attention Graph，Skill 会组合爆炸。

世界上每个对象、每个平台、每类数据、每个公司内部流程、每种写作范式都可能产生大量 Skill。

Agent 不可能也不应该全量维护。

因此，Skill 的发现和加载应当依赖 Level 1 的 Attention：

```text
High Attention Object / Event
        ↓
主动探索或用户安装
        ↓
发现 Shortcut
        ↓
形成 Skill Coverage Gap / Candidate Signal
        ↓
真实使用与验证
        ↓
进入 Skill Ranking
```

也就是说：

> 热点对象周围的捷径才值得优先发现和维护。

例如，只有当“2026 暑假出游”升温后，探索国家公园官网、预约页面和开放状态查询捷径才变得重要。

---

### 6.1.1 Skill Signal 不等于 Skill 改写

这里需要区分两件事情：

```text
Self Improve Stage 1
    从 Session History 中发现 attention signals。

Skill Improvement LLM
    消费 skill 相关 signals，生成 Skill Update / New Skill / Anti-pattern proposal。
```

Self Improve 的标准流程更像是在维护 Attention Graph：

- 哪些事件变热；
- 哪些对象被反复提及；
- 哪些观察值得写入；
- 哪些对象需要主动探索。

Skill 相关信号可以进入这个 Stage 1，但不应该由标准 Self Improve LLM 直接完成 Skill 改写。

原因是 Skill 改进需要一套不同的判断标准：

```text
这个 Skill 是否被 list / select / load / use？
它是否真的参与了成功路径？
它失败是因为 Skill 错，还是选择错？
是否应该改描述、改 category、拆分、合并、降级或 Block？
是否需要回放历史 case 做 regression test？
```

这些问题和普通 Attention 更新不同。因此，Self Improve Stage 1 更适合作为信号抽取层：

```text
Session History
    ↓
Attention / Skill Signal Extraction
    ↓
Attention Graph Update
    ↓
Skill Signals Queue
    ↓
独立 Skill Improvement LLM
```

也就是说：

> Skill signal 可以由 Self Improve Stage 1 发现，但 Skill signal 的消费应当交给独立 LLM 流程。

---

### 6.2 Skill 的三大类

当前观察到的 Skill 可以分成三大类。

#### 6.2.1 数据获取类 Skill

核心作用：

> 更快、更可靠地获得某类数据。

例如：

- 股票历史 K 线 CSV 下载方法；
- 某金融数据网站的参数化 URL；
- YouTube 视频下载工具；
- 某平台数据 API；
- 国家公园官网的开放状态页面；
- 某类公开文件的稳定入口。

这类 Skill 是典型的观察世界的捷径。

它通常绑定：

```text
数据类型
数据平台
数据格式
入口 URL
参数规则
可信度
更新频率
```

它的价值很容易验证：

```text
能不能拿到数据？
数据是否完整？
来源是否可信？
格式是否稳定？
成本是否更低？
```

---

#### 6.2.2 流程 / 行动类 Skill

核心作用：

> 指导 Agent 按某个真实世界或组织内部流程完成外部影响行为。

例如：

- 公司提交代码前要走的流程；
- 内部系统上线流程；
- 发布一个产品到某个平台；
- 在 Booking.com 上完成酒店预订；
- 把旅行计划真正转化成订单；
- 把报告提交到某个内部系统。

这类 Skill 是改变世界的捷径。

它通常绑定：

```text
组织
平台
权限
流程
角色
审批
产物
风险
外部影响
```

这类 Skill 往往是私有的、环境绑定的。例如，一个用户在公司 A 时安装的内部流程 Skill，离开公司 A 后可能完全失效。

它的价值也相对容易验证：

```text
流程是否走通？
产物是否成功发布？
有没有权限问题？
有没有造成错误外部影响？
```

---

#### 6.2.3 范式类 Skill

核心作用：

> 补充大语言模型在某些任务上的能力短板。

例如：

- 如何写好 PPT；
- 如何写商业计划书；
- 如何拆解复杂设计文档；
- 如何进行可信信息源判断；
- 如何设计一个好的产品路线图；
- 如何写出结构清晰的代码审查意见。

范式类 Skill 不是直接连接外部世界，而是在改善 Agent 的思考、写作、判断和生成质量。

这类 Skill 对应的是 LLM 能力曲线上的“低谷补洞”。

例如，写代码目前是很多模型能力曲线上的高点，因为工具验证链路成熟：

```text
写代码 → 运行测试 → 查看错误 → 修复 → 再测试
```

但写 PPT、写商业计划书、判断表达是否有说服力，则很难通过工具快速闭环。这时范式类 Skill 就更有价值。

不过，范式类 Skill 也是最容易冲突的一类，因为它涉及：

```text
什么叫好？
什么叫专业？
什么叫清晰？
什么叫有说服力？
```

这些判断天然带有主观性。

---

## 7. Skill 安装不是直接加载，而是 LLM 编译

用户手工安装 Skill 是一个很直观的触发点，但不能简单地把用户安装的文本直接放入运行时候选列表。

因为一个用户安装的 Skill 可能只是自然语言经验总结：

- 很流水账；
- 没有模块化；
- 可能包含多个技能；
- 可能包含多个原则；
- 可能包含废话；
- 可能内部矛盾；
- 可能已经过时；
- 可能只适合某个场景。

因此，安装过程应当是一个 LLM Install Skill / LLM Skill Compiler 过程。

```text
Skill Source Text
        ↓
LLM Skill Installer
        ↓
分类、拆解、提取、命名、归档
        ↓
Runtime Skill Candidates
        ↓
后续验证、排名、加载
```

---

### 7.1 Skill Source ≠ Runtime Skill

用户安装的是 Skill Source，不是最终运行时 Skill。

例如用户安装：

```text
How to download YouTube video
```

这个文本里可能包含：

- 使用某个工具；
- 输入 YouTube URL；
- 选择清晰度；
- 失败时切换镜像；
- 保存文件命名规则；
- 法律或平台限制提示；
- 某些过时命令；
- 作者个人偏好。

LLM Installer 要把它拆成：

```text
Data Acquisition Skill:
    YouTube 视频下载方法

Workflow Skill:
    下载、检查、保存、命名流程

Principle / Warning:
    注意版权、平台规则、失败重试策略
```

最终进入不同目录或不同 Skill Group。

---

### 7.2 安装过程需要保留来源

每个 Skill 都必须记录来源。

来源可能包括：

```text
系统内置
用户手工安装
Agent 自主探索
群聊记录观察到
官方文档
第三方博客
同事推荐
历史失败修复总结
```

用户安装不等于绝对可信。

普通用户可能会批量安装很多 Skill，但并不知道：

- 是否互相冲突；
- 是否过期；
- 是否安全；
- 是否适合当前场景；
- 是否包含反模式。

所以来源只影响初始信任和优先级，不代表最终质量。

---

### 7.3 Skill 之间可能竞争

同一个目标可能有多个 Skill：

```text
How to download YouTube video
    Skill A
    Skill B
    Skill C
```

Agent 不应该简单加载所有 Skill，而应该形成排名：

```text
Skill Group: YouTube Video Download
    Rank 1: Skill B
    Rank 2: Skill A
    Rank 3: Skill C
```

排名应当来自真实 Work Session 的使用结果，而不是安装时的说明。

---

## 8. Skill 使用与 Self Improve 反馈闭环

Work Session 会加载 Skill、使用 Skill，并形成 Session History。

Self Improve 会从这些历史中抽取 skill 相关 signal，但不应该把“发现 signal”和“修改 Skill”合并成同一次 LLM 调用。

```text
Work Session
    ↓
Load Skill
    ↓
Use Skill
    ↓
Task Result
    ↓
Session Report
    ↓
Session History
    ↓
Self Improve
    ↓
Extract Skill Signals
    ↓
Skill Improvement LLM
    ↓
Update Skill Score / Rank / Status / Proposal
```

这里有两类需要分开的 LLM 激活流程。

### 8.1 已有 Skill 改进是独立激活流程

对已有 Skill 的改进，不应依赖标准 Self Improve 的常规触发条件。

标准 Self Improve 的触发信号通常是：

```text
新的 Session History 未处理
某些 Event / Object attention 上升
需要合并、强化或遗忘 Memory
```

已有 Skill 改进的触发信号则是另一组：

```text
Skill 经常被 list 但不被 select
Skill 经常被 select / load 但没有实际使用
Skill 被使用但不在成功关键路径上
Skill 被使用后任务失败率高
Skill 触发 fallback / redo / supervisor rejection
Reporter 明确说 Skill 不适用
同类 Skill 之间出现重复或冲突
外部平台、协议、页面或流程变化导致 Skill 过期
```

因此它应该是一个独立的 LLM activation flow：

```text
SkillUsageTrace
    ↓
SkillEffectiveness / Misuse / Expiration Signals
    ↓
Skill Improvement LLM
    ↓
Skill Evaluation Case
Skill Update Proposal
Skill Deprecation Proposal
Skill Regression Test Plan
```

这个流程的目标不是泛化地更新 Memory，而是围绕已有 Skill 做诊断：

- 是否修改 Skill 内容；
- 是否调整 Skill 描述或触发条件；
- 是否调整 category / ranking；
- 是否拆分、合并或淘汰；
- 是否加入 BlockList；
- 是否需要人工 review。

---

### 8.2 新 Skill 发现是 Attention Signal，但消费流程独立

新 Skill 发现没有现成 Skill 对象，因此它不应从“所有成功任务”中盲目提取。

更可靠的入口是把它作为一种新的 attention signal：

```text
Skill Coverage Gap Signal
```

它在 Self Improve Stage 1 中被抽取，和 Event / Object / Observation signal 一起进入 attention 处理，但它的语义不同：

```text
Plan 中有明确 TODO
但没有合适 Skill 覆盖
Do 阶段通过通用推理 / Global Object 探索完成
Reporter / Supervisor / User 认可结果
```

或：

```text
Plan 分配了 Skill
但实际 Skill 没有被使用
最终仍然成功
```

这类信号说明：

> 当前 Skill Graph 对某类任务的 coverage 可能不足，Agent 临时摸索出了一条可复用路径。

但是否真的要生成新 Skill，应该由独立流程判断：

```text
Skill Coverage Gap Signal
    ↓
Candidate Skill Miner LLM
    ↓
是否重复出现？
是否有稳定触发条件？
是否有稳定执行路径？
是否能写成可验证 Skill？
是否比现有 Skill 更好？
    ↓
Candidate Skill Case / Skill Draft / Reject
```

这避免两个误判：

- 把一次偶然成功沉淀成 Skill；
- 把普通任务总结误认为能力增长。

---

Skill 的元数据至少应包括：

```text
Skill ID
Skill Group
Skill Type
Source
Scope
Created At
Installed At
Compiled At
Last Loaded At
Last Used At
Used Sessions
Success Count
Failure Count
Average Cost
Average Latency
Average Output Quality
Last Verification Result
Conflict Notes
Current Rank
Status
```

可能的状态包括：

```text
Source Installed
Compiled Candidate
Active
Preferred
Deprecated
Blocked
Needs Verification
Conflict Detected
Expired
```

---

### 8.3 不同类型 Skill 的评价方式不同

#### 数据获取类 Skill

评价指标相对明确：

```text
是否拿到数据
数据是否完整
数据是否可信
格式是否可解析
是否比搜索更快
是否过期
```

#### 流程 / 行动类 Skill

评价指标包括：

```text
流程是否成功完成
是否产生正确外部影响
是否违反权限或安全约束
是否需要人工补救
是否比手动流程更快
```

#### 范式类 Skill

评价更复杂，可能需要：

```text
用户反馈
产物评分
后续修改次数
是否被复用
是否与其他原则冲突
是否提升了任务完成质量
```

范式类 Skill 不应轻易全局化。它应该有明确适用范围、领域、上下文和冲突记录。

---

## 9. Paradigm Skill 与 Principle 的吸收

范式类 Skill 更像一种 Principle，而不是工具。

安装后，Agent 不应立即把它当作自己的信念，而应该区分：

```text
外部 Principle
内部 Principle
```

---

### 9.1 外部 Principle

例如某个 Skill 说：

```text
商业计划书应该先讲市场规模。
```

安装后，Agent 可以先记录为：

```text
某某来源认为：商业计划书应该先讲市场规模。
```

这表示 Agent 知道有这个观点，但不一定认同。

---

### 9.2 内部 Principle

如果经过多次使用、验证、用户反馈后，Agent 认为这个原则确实有效，则可以吸收为：

```text
Agent 认为：在某些类型的商业计划书中，优先讲清市场规模通常是有效结构。
```

这才是进入 Agent 自身 Self-Thinking 的东西。

也就是说：

```text
Install ≠ Believe
Install → Observe → Validate → Adopt
```

---

### 9.3 Principle 的冲突管理

范式类 Skill 最容易互相矛盾。

例如：

```text
Skill A: PPT 应该信息密度低，每页只表达一个重点。
Skill B: 投资人 Deck 应该尽可能高密度呈现商业信息。
```

两者可能都对，但适用场景不同。

因此 Self Improve 要做的不是简单判断谁对谁错，而是提取上下文：

```text
低密度表达：适合演讲型展示。
高密度表达：适合投资人快速浏览型 Deck。
```

这类冲突管理是 Paradigm Skill 管理的核心。

---

## 10. BlockList 与能力自信模型

Skill 系统不是越多越好。

有些领域 Agent 本身已经很强，额外加载 Skill 只会增加噪音、Token 成本和冲突风险。

例如，如果系统认为当前模型已经擅长写 Bash 脚本，则可以把一些低质量代码写作范式 Skill 加入 BlockList：

```text
BlockList:
    Generic Bash Coding Advice
    Outdated Python Style Guide
    Low Quality Coding Prompt
```

BlockList 有两种来源。

### 10.1 系统内置 BlockList

代表系统开发者的品味和判断：

```text
这个领域模型已经足够好。
这类 Skill 通常弊大于利。
这类 Skill 容易污染上下文。
```

### 10.2 Agent 自生成 BlockList

代表 Agent 对自己能力的理解：

```text
我已经不需要加载这类 Skill。
我在这个任务类型上表现稳定。
这个 Skill 多次没有帮助。
这个 Skill 经常与更好的 Skill 冲突。
```

这相当于 Agent 的能力自信模型。

它不是单纯记住“我会什么”，而是判断：

> 哪些外部经验对我仍然有价值，哪些已经不值得占用上下文。

---

## 11. Skill 的生命周期

一个 Skill 从进入系统到被淘汰，大致可能经历如下生命周期：

```text
1. Discovered / Installed
   被用户安装、Agent 发现、系统内置或从历史中提取。

2. Compiled
   经过 LLM Installer 拆解、分类、命名、归档。

3. Candidate
   成为某个 Skill Group 下的候选捷径。

4. Loaded
   在某次 Work Session 中被加载。

5. Used
   被实际用于观察、思考或行动。

6. Evaluated
   通过 Session Report 和独立 Skill Improvement Flow 获得评分。

7. Ranked
   在同类 Skill 中被提升或降低排名。

8. Promoted / Adopted
   成为优先 Skill，或被吸收成 Agent 的 Principle。

9. Deprecated / Blocked
   因失败、过期、冲突或低收益被淘汰。
```

可以用一个更简洁的状态机表示：

```text
Source
  ↓ install
Compiled Candidate
  ↓ verification
Active
  ↓ repeated success
Preferred
  ↓ decay / failure / conflict
Deprecated
  ↓ severe issue
Blocked
```

---

## 12. Level 0：Self-Thinking / Value System 的位置

前面两层分别回答：

```text
Level 1: What matters?
Level 2: How to do it better?
```

但“better”本身需要定义。

两个 Skill 谁更好，可能取决于：

- 更快；
- 更省 Token；
- 更可靠；
- 更可信；
- 更能赚钱；
- 更安全；
- 更符合主人偏好；
- 更适合长期沉淀。

这些评价标准本身构成 Value System。

---

### 12.1 为什么这一层很难

用户显式说出来的价值观，和用户真实行动体现出来的偏好可能不一致。

用户可能说自己最重视质量，但实际总是选择更快交付；也可能说自己重视成本，但在关键任务上愿意消耗大量 Token 做验证。

因此，Agent 的价值观很可能不能只靠用户手工配置，而要从长期行为和反馈中归纳。

这一层涉及：

```text
Agent 自我认知
用户偏好推断
长期目标
任务优先级
自我打分
理想 Agent 形象
自我改进任务生成
```

这些目前过于宽泛，开发者很难提前穷举。

---

### 12.2 这一层的输入和输出

它的输入可以是：

```text
Attention Graph
Skill Graph
历史 Work Session 结果
用户反馈
任务失败记录
Agent 自己的能力评估
系统默认原则
```

它的输出不是直接动作，而是：

```text
Self Improvement Tasks
```

例如：

```text
提升信息源可信度判断能力。
降低高频任务 Token 成本。
为旅行规划建立更好的主动风险检查流程。
整理并淘汰低质量 PPT 范式 Skill。
为金融数据获取建立更可靠的数据源优先级。
```

---

### 12.3 为什么它可以暂时作为架构占位

这一层非常重要，但不适合一开始做成复杂系统。

更合理的策略是：

```text
先给它留位置。
先定义输入输出。
先允许它生成有限类型的自我改进任务。
通过长期测试观察它会如何演化。
```

当前优先级应当是 Level 2，因为 Skill / Shortcut 管理是明确的能力扩展点，且更容易观测、评估和迭代。

---

## 13. 几个关键设计原则

### 13.1 记录观察，不急于声明事实

Self Improve 不应过早把自然语言观察变成强事实。

更安全的做法是：

```text
Observation first, Fact later.
```

例如：

```text
妻子觉得国家公园 A 太干。
```

这不是国家公园 A 的客观缺点，而是一个具体人的具体偏好观察。

---

### 13.2 Install 不等于 Trust

用户安装 Skill 只是引入候选经验，不代表 Skill 已经可信。

必须保留来源、验证记录、冲突记录和使用反馈。

---

### 13.3 Skill Source 不等于 Runtime Skill

自然语言经验必须经过 LLM Compiler 拆解、分类、命名、归档，才进入运行时 Skill 候选集合。

---

### 13.4 高 Attention 才值得主动探索

主动探索和 Skill 发现都必须被 Attention Graph 限制，否则会组合爆炸。

---

### 13.5 Skill 有时效性

很多 Skill 绑定平台、页面、组织流程或政策。

它们需要过期、验证、降级和淘汰机制。

---

### 13.6 泛化 Skill 比对象绑定 Skill 生命周期更长

例如：

```text
Yosemite 预约页面
```

是对象绑定 Skill，可能很快过期。

```text
如何查询美国国家公园当前开放与预约状态
```

是模式 Skill，生命周期更长。

---

### 13.7 范式类 Skill 需要上下文隔离

范式类 Skill 不应轻易全局生效，否则容易污染 Agent 的判断。

它应当带有：

```text
适用领域
适用任务
来源
冲突观点
是否已被 Agent 吸收
```

---

### 13.8 Skill Signal 的生产和消费分离

Self Improve 可以生产 skill signal，但不直接消费 skill signal。

更准确的职责划分是：

```text
Self Improve Stage 1:
    从 Session History 中抽取 Event / Object / Observation / Skill signals。

Attention Graph Updater:
    消费普通 attention signals，更新事件、对象、观察和权重。

Skill Improvement LLM:
    消费已有 Skill 的使用效果 signals，生成更新、降级、合并、拆分或淘汰建议。

Candidate Skill Miner LLM:
    消费 Skill Coverage Gap signals，判断是否形成新 Skill 候选。
```

这样 Skill 系统仍然依赖 Attention，但不会让通用 Self Improve LLM 同时承担过多职责。

---

## 14. 一个可能的数据结构草案

下面不是最终 schema，只是为了表达结构。

### 14.1 EventItem

```yaml
event_id: evt_2026_summer_trip
title: 2026 暑假出游
status: planning
participants:
  - LZC
  - wife
  - son
related_objects:
  - national_park_a
  - budget_sheet
  - photo_folder
observations:
  - obs_001
  - obs_002
attention_weight: 0.87
created_from_sessions:
  - session_001
  - session_009
last_updated: 2026-xx-xx
```

### 14.2 ObjectObservation

```yaml
observation_id: obs_001
subject: national_park_a
context_event: evt_2026_summer_trip
observer: agent
content: 妻子觉得这个地方太干燥，兴趣较低。
source_session: session_009
confidence: medium
timestamp: 2026-xx-xx
```

### 14.3 AttentionEdge

```yaml
from: LZC
to: evt_2026_summer_trip
relation: interested_in
weight: 0.93
hop_distance_from_agent: 1
last_reinforced: 2026-xx-xx
```

### 14.4 SkillSource

```yaml
source_id: skill_source_youtube_download_001
title: How to download YouTube video
source_type: user_installed
raw_text_path: skills_sources/youtube_download_001.md
installed_by: user
installed_at: 2026-xx-xx
initial_trust: medium
```

### 14.5 RuntimeSkill

```yaml
skill_id: skill_youtube_download_ytdlp
skill_group: youtube_video_download
type: data_acquisition
source_id: skill_source_youtube_download_001
scope:
  platform: YouTube
  data_type: video
status: active
rank: 1
success_count: 12
failure_count: 1
last_used_at: 2026-xx-xx
last_verified_at: 2026-xx-xx
notes:
  - 使用 yt-dlp 获取视频，比浏览器探索稳定。
```

### 14.6 Principle

```yaml
principle_id: principle_bp_market_first
source_type: paradigm_skill
source_id: skill_source_business_plan_001
status: external_opinion
statement: 某来源认为商业计划书应优先讲清市场规模。
applicable_context:
  - investor_pitch
  - fundraising_deck
conflicts_with:
  - principle_story_first_pitch
adoption_status: not_adopted
```

### 14.7 SkillUsageTrace

```yaml
trace_id: skill_trace_001
task_context:
  user_request: 帮我整理这份 PDF 里的流程
  todo: 抽取流程步骤并生成 Markdown
skill_listing:
  listed_skills:
    - pdf
    - documents
skill_selection:
  selected_skills:
    - pdf
skill_loading:
  loaded_skills:
    - pdf
execution:
  actually_used_skills:
    - pdf
  fallback_used: false
  tool_errors: []
report:
  status: success
  summary: 成功渲染 PDF 并抽取文本。
supervisor_review:
  accepted: true
```

这是已有 Skill 改进流程的主要输入。

### 14.8 SkillCoverageGapSignal

```yaml
signal_id: skill_gap_001
source_session: session_123
todo:
  description: 分析一组未知格式日志并定位失败原因
assigned_skills: []
actual_execution:
  used_skills: []
  critical_actions:
    - 探索目录结构
    - 识别日志格式
    - 编写临时解析脚本
    - 归纳失败模式
result:
  status: success
  supervisor_accepted: true
gap_signal:
  no_skill_assigned: true
  success_without_skill: true
  repeated_pattern: unknown
consumer:
  target_llm_flow: candidate_skill_miner
```

这是 Stage 1 发现的新 attention signal。它只说明“这里可能有 Skill coverage gap”，不直接等同于新 Skill。

### 14.9 SkillImprovementCase

```yaml
case_id: skill_case_001
case_type: existing_skill_update
input_signals:
  - skill_trace_001
affected_skills:
  - pdf
finding: Skill 经常被加载并成功使用，但缺少图片型 PDF 的 OCR fallback。
proposal:
  action: update_skill
  change: 增加图片型 PDF 的预处理路径和失败判断。
validation:
  replay_cases:
    - session_123
status: needs_review
```

这是独立 Skill Improvement LLM 的输出，而不是 Self Improve Stage 1 的直接输出。

---

## 15. 一个可能的系统目录结构

```text
self_improve/
  attention_graph/
    events/
    objects/
    observations/
    edges/
    attention_rank/
    signals/
      event_signals/
      object_signals/
      skill_gap_signals/

  skills/
    sources/
      user_installed/
      system_builtin/
      self_discovered/

    runtime/
      data_acquisition/
      workflows/
      paradigms/

    groups/
      youtube_video_download/
      stock_kline_data/
      national_park_status_lookup/

    evaluation/
      usage_logs/
      traces/
      scorecards/
      conflicts/
      deprecated/
      blocklist/
      cases/

    improvement_flows/
      existing_skill_update/
      candidate_skill_mining/
      anti_pattern_mining/

  principles/
    external/
    adopted/
    rejected/
    conflicts/

  self_thinking/
    values/
    competence_model/
    improvement_tasks/
    self_scores/
```

这只是表达结构，不一定对应真实文件系统实现。

---

## 16. 优先落地建议

当前可以先不实现完整 Level 0，而从更明确的 Skill 管理开始。

### Phase 1：Self Improve Stage 1 抽取 Skill Signals

实现：

- 从 Session History 中抽取已有 Skill 使用链路；
- 从 Plan / Do / Report / Review 中抽取 Skill Coverage Gap；
- 将 Skill Gap 作为新的 attention signal 写入 signal queue；
- 只生成 signals 和 traces，不直接改 Skill。

这个阶段的输出包括：

```text
SkillUsageTrace
SkillCoverageGapSignal
AntiPatternSignal
```

其中，新 Skill 发现只在这里产生 signal，不在这里生成 Skill。

### Phase 2：Skill Source 与 Runtime Skill 分离

实现：

- 用户安装 Skill Source；
- LLM Installer 拆解成三类候选；
- 保留来源；
- 生成 Runtime Skill；
- 按 Skill Group 归档。

### Phase 3：Work Session 记录 Skill 使用

实现：

- 记录加载了哪些 Skill；
- 记录哪些 Skill 被实际使用；
- 记录任务结果；
- 记录失败原因；
- 生成 Session Report。

### Phase 4：独立 Existing Skill Improvement Flow

实现：

- 消费 `SkillUsageTrace`；
- 根据 Work Session History 给已有 Skill 打分；
- 同组 Skill 排名；
- 标记失效 Skill；
- 发现冲突 Skill；
- 生成 Skill Update / Deprecation / Block proposal；
- 必要时生成 regression test plan。

这个 Flow 的激活信号不同于标准 Self Improve：

```text
高失败率
高 unused rate
fallback / redo / supervisor rejection
Reporter 负反馈
过期或冲突迹象
```

### Phase 5：最小 Attention Graph

实现：

- 从 Session History 中发现 Event；
- 记录 Event 关联对象；
- 维护 Top-N Attention Objects；
- 通过衰减机制遗忘冷对象。

### Phase 6：独立 Candidate Skill Miner Flow

实现：

- 消费 `SkillCoverageGapSignal`；
- 聚合同类 gap；
- 判断是否重复出现；
- 判断是否有稳定触发条件和执行路径；
- 生成 Candidate Skill Case 或明确 Reject。

这个 Flow 不应把一次成功直接变成 Skill。它只在 coverage gap 重复、路径稳定、触发条件清晰时推动后续 Skill Draft。

### Phase 7：主动探索 Subtask

实现：

- 对 Top Attention Event / Object 创建 Research Task；
- 将研究结果作为 Observation 写回；
- 如果发现稳定捷径，生成 Skill Coverage Gap / Candidate Skill signal。

### Phase 8：Principle 与 BlockList

实现：

- Paradigm Skill 不直接全局生效；
- 先作为 External Principle；
- 通过使用结果决定是否 Adopt；
- 建立系统和 Agent 自生成 BlockList。

---

## 17. 仍然开放的问题

1. Event 与 Topic 的边界是什么？
2. Event 是否必须有生命周期状态？
3. Event 是否允许合并、拆分和嵌套？
4. Attention Rank 的权重公式应该由系统固定，还是允许 Agent 自我调整？
5. 主动探索的边界如何控制，避免过度消耗资源？
6. Skill Source 的可信度如何初始化？
7. 用户手工安装的 Skill 是否需要安全沙箱或权限声明？
8. Paradigm Skill 什么时候可以从 External Principle 变成 Adopted Principle？
9. Agent 自生成 BlockList 是否可能导致过度自信？
10. Level 0 的 Self-Thinking 如何避免失控？
11. 用户显式价值观和行为偏好冲突时，Agent 应该相信谁？
12. Skill 的评价是否应该区分“任务成功”和“用户满意”？
13. 内部流程类 Skill 离开原组织后如何自动失效？
14. 对外部世界的观察 Skill 如何处理网页、API、平台政策变化？
15. Self Improve 的执行频率应该是每晚、每 N 个 Session，还是事件触发？

---

## 18. 当前结论

Self Improve 的核心不在于“总结历史”，而在于让 Agent 形成一个持续演化的经验系统。

这个经验系统至少包括三张图：

```text
Attention Graph
    记录哪些事情与我和主人有关。

Skill / Shortcut Graph
    记录处理这些事情的捷径，以及它们的有效性。

Self-Thinking / Value Graph
    记录 Agent 对什么叫更好、自己想成为什么样的 Agent 的长期判断。
```

其中，Attention Graph 让 Agent 知道世界里哪里正在变热；Skill Graph 让 Agent 在热点周围积累可复用能力；Self-Thinking 则决定这些能力应该朝什么方向演化。

最值得优先落地的是 Skill / Shortcut 管理，因为它具备明确的工程边界：

- 用户可以安装 Skill；
- LLM 可以编译 Skill；
- Work Session 可以使用 Skill；
- Self Improve Stage 1 可以从 Session History 中抽取 Skill signals；
- 独立 Skill Improvement LLM 可以推动排名、升级和淘汰 Skill；
- 独立 Candidate Skill Miner LLM 可以把 coverage gap 转化为候选 Skill case。

这使得 Agent 不再只是依赖模型内置能力，而是逐步拥有一套可增长、可竞争、可遗忘、可验证的经验生态系统。

最终，Self Improve 想实现的是：

> Agent 通过自己的历史经验，持续理解自己正在参与什么世界，持续发现更好的交互捷径，并逐渐形成关于自身能力和改进方向的长期判断。
