# Agent 元能力设计

> **本文是理解 OpenDAN Runtime 设计的第一篇理念文章。** 在读任何 Runtime 架构、subsystem 接口或实现代码之前，建议先读它——它解释的是“为什么 OpenDAN 的 Agent 要这样长”，而不是“它具体怎么实现”。
>
> 本文由两篇前置思考收敛而来：[Agent 世界元能力.md](Agent%20世界元能力.md)（外向：Agent 如何与世界发生关系的原始思考）与 [Agent元能力思考.md](Agent元能力思考.md)。这两篇是本文仅有的上游来源；想看更原始的推导过程和被修正掉的中间结论，回去读它们。
>
> 它不是 Runtime 实现规格，也不是某个具体 subsystem 的接口设计，而是一篇架构背景文章：说明为什么需要 Agent 元能力、元能力和 skill 的边界在哪里、一个能持续行动并自我改进的 Agent 框架在理念上必须闭合到什么程度。
>
> 后续的 Runtime 架构与各 subsystem 文档，应在本文理念之上把它落实成模块分层、数据流和接口——但那是另一层文档的职责。本文只负责把前提讲清楚：读完之后再去看具体设计，应该能看出每一个模块是为了闭合本文哪一处而存在的。

---

## 0. 核心结论

一句话：

> **元能力让 Agent 能理解和生成 skill；skill 是元能力在稳定场景中的、已经验证过的缓存。**

更完整地说：

- Chatbox 是 `(prompt, weights)` 的函数，活在单轮上下文里；
- Agent 是它在一个可被观察、可被改变、带 owner 和授权边界的世界里持续行动的历史函数；
- Agent 不只需要回答当前问题，还要能被事件唤醒、跨 session 保持内部状态、重新观察外部对象、从历史中结晶能力；
- 框架不应该硬编码大量具体流程，而应该提供少量底层元能力，让具体能力通过探索、验证、结晶、排名、遗忘逐步长出来。

本文把 Agent 元能力分成三层：

```text
外向元能力：Agent 如何观察、发现、判断、改变世界
内向元能力：Agent 如何维护跨 session 的内部沉淀
高层元能力：Agent 如何判断什么更重要、如何认识自己的能力边界
```

三层之间的关键接缝是：

> **内部状态里的 object_id，必须指向外部世界里的同一个 Global Object。内部只保存印象和线索，外部才是真相。**

---

## 1. 两个本质问题：为什么是“元能力”，而不是继续堆 skill

OpenDAN 反复从不同角度凿的，其实是同一面墙上的两个问题，本文所有元能力都挂在它们下面：

- **问题一（本质）**：Agent 和 Chatbox 的本质区别在哪？（见 §2）
- **问题二（边界）**：Agent 框架的边界在哪，什么进框架、什么留给 skill？（本节与 §3）

这两个问题之所以是同一面墙，是因为**要定义 skill，必须先定义“什么不是 skill”**；而这条线一拉到底，拉出来的就是整个 Agent：

```text
skill 是什么？
  -> skill 是“结晶下来的经验”（不是预置的固定流程）
    -> 那就得有个“结晶器” ……………… self-improve
      -> 结晶什么？围绕什么结晶？ …… Attention（什么在变热）
        -> 凭什么说一个 skill 更好？ …… Value（什么叫“更好”）
          -> 谁在积累这些经验？ ………… 一个跨 session 持续的“自我”
            -> 在哪儿积累、对谁负责？ … 一个有 owner、有物权的世界
```

这条阶梯说明：元能力不是随手列的一张清单，而是从“skill 到底是什么”一路**逼**出来的——少任何一环，结晶都无法闭合。下面先回答问题二（本节与 §3），再回答问题一（§2）。

今天很多 Agent skill 本质上是任务流程提示词，结构通常是：

```text
观察 / 分析 -> 动作 / 执行 -> 检查 -> 交付
```

这种模式对高频、稳定、边界清楚的任务有效。但如果所有能力都写成固定 skill，会带来几个问题：

1. **环境一变，skill 很快过时。**
   具体工具、协议、页面、业务流程都会变化。把细节写死在 skill 里，会让 Agent 依赖过期知识。

2. **Agent 的探索能力被压低。**
   它会倾向于套流程，而不是从当前世界对象、当前工具、当前权限出发重新判断。

3. **系统无法解释 skill 从哪里来。**
   如果 skill 永远由人手写，Agent 就没有真正的自我改进，只是被不断补丁化。

4. **框架边界会膨胀。**
   每遇到一种任务就加一套规则，最终框架会变成大而脆的流程集合。

因此更合理的方向是：

> 给 Agent 一组很少、很底层的原能力，让它知道如何探索世界、判断可信度、使用或构造工具、检查风险、沉淀经验；当某些探索路径稳定下来后，再结晶成 skill。

这就把 skill 的位置重新定义了：

- skill 不是框架本身；
- skill 是被框架托管的经验结晶；
- skill 需要验证、排名、降权、淘汰；
- skill 可以来自人类编写，也可以来自 Agent 自己的 self-improve。

---

## 2. Agent 和 Chatbox 的分界

要定义 Agent 元能力，首先要回答一个问题：Agent 和 Chatbox 的本质区别是什么？

Chatbox 的典型形态是：

- 只被用户输入唤醒；
- 状态主要存在于当前 context window；
- 输出主要是文本；
- 能力主要来自预置 prompt、模型权重和手工工具表；
- 没有自己的长期历史，也没有对世界对象的 owner / 权限 / 风险责任。

Agent 至少多出四个维度。

### 2.1 生命力：不只被用户消息唤醒

Agent 可以被时间、事件、对象状态变化、数据更新、长期任务检查点唤醒。

这意味着它可以处理超过一次对话的任务：

```text
现在观察 -> 订阅事件 / 设置检查点 -> 等待条件 -> 再次唤醒 -> 继续处理
```

如果一个系统只能在用户发消息时工作，它仍然更接近 Chatbox。

### 2.2 持续的自我：Session 是上下文隔离，不是世界隔离

Session 隔离的是 LLM context，不是世界状态。两个 session 可能访问同一份文件、同一个设备、同一个用户计划、同一个长期项目。

Agent 需要跨 session 维护自己的内部状态：

- 用户明确声明过什么；
- 它参与过哪些事情；
- 它对哪些对象形成过印象；
- 哪些历史经验能在未来复用。

Chatbox 的“记忆”通常只是把历史塞进上下文。Agent 的记忆必须是可追溯、可召回、可整理、可遗忘的内部状态系统。

### 2.3 参与世界：不只是吐文本

Agent 面对的不是一段纯文本输入，而是一个对象世界：

- 有 Entity、Data、Tool、Indexer；
- 有 owner、授权、身份、可信度；
- 有可逆和不可逆的操作；
- 有他人权益、金钱、隐私、物理世界风险；
- 有需要重新观察的实时状态。

因此 Agent 的每次行动都要区分：

```text
我知道了什么？
这个知识可信吗？
我是否有权使用它？
这个动作会改变什么？
是否需要 owner 或可信实体确认？
```

### 2.4 自我改进：能力是传记的函数

Chatbox 的能力主要来自出厂时的 prompt 和 weights。Agent 的能力还来自自己的历史：

```text
参与过的 session
  -> 形成注意力和对象印象
  -> 发现重复任务和稳定路径
  -> 验证并结晶 skill
  -> 排名、降权、遗忘
```

因此可以得到一个判据：

> **Chatbox 有上下文窗口，Agent 有传记。**

再压缩一下：

> **Agent 同时落在时间和物权两维里：它会被世界唤醒，也要对不属于自己的东西负责。**

这两维可以用两个反向思想实验确认：

> 抽掉时间维 -> 得到“带手的 Chatbox”：有工具，却活在永恒的当下；
> 抽掉物权维 -> 得到“没有问责对象的自动脚本”：会动，却无人为后果负责。
>
> Agent 正是同时落在这两维里的东西。

---

## 3. 框架边界：代谢，不是内容

Agent 框架最容易失控的地方，是把太多具体做法塞进框架。

本文采用一条边界剃刀：

> **能从经验里被结晶、排名、遗忘的，都不是框架，是 skill；做结晶、排名、遗忘这件事的，才是框架。框架是 metabolism，不是 content。**

按这条线切：

### 3.1 框架内：元能力和代谢机制

框架应该包含：

- 驱动与唤醒模型；
- 世界对象模型；
- 对象发现机制；
- 可信度、风险、owner、授权判断脚手架；
- session history、Notebook、Memory、Skill 的基础存储和召回；
- self-improve 的扫描、结晶、巩固、遗忘循环；
- value / preference 的槽位和学习循环；
- Agent 自我模型、能力自信、BlockList 的基础机制；
- 身份、权限、审计和可追溯性约束。

这些机制回答的是“Agent 如何持续成为一个 Agent”。

### 3.2 框架外：具体流程、领域知识、任务技巧

框架不应该硬编码：

- 某类业务任务的固定步骤；
- 某个产品页面的具体操作流程；
- 某个工具的过细使用经验；
- 某个领域的可过期知识；
- 某个项目短期内的临时约定。

这些内容应作为 skill、Notebook 条目、Memory 线索或普通 Data 被托管，而不是焊死在 Runtime 框架里。

### 3.3 边界是会迁徙的膜

框架和 skill 之间不是一堵固定墙，而是一道会迁徙的膜。

有些能力早期是 skill，后来足够稳定、普适、低风险，可能被吸收到框架里；有些框架原语如果发现需要每个 Agent 高度个性化，也可能被推出去变成 skill 或偏好。

最典型的是 Value：

- 它定义“什么叫更好”，因而是元层；
- 但具体 value 内容又必须从 owner 的长期行为和反馈中学习，不能硬编码。

所以 Value 不在框架或 skill 的任一侧，它就是那道膜本身：

> 框架提供 value 的槽位与学习循环，不提供 value 的具体内容。

---

## 4. 总体形状：外向、内向和接缝

一个跨时间、在真实世界里行动的 Agent，元能力天然分成两半：

```text
        外部世界                                内部状态
   （可被重新观察）                         （只能从经历沉淀）
        │                                        │
   外向元能力          -- object_id -->      内向元能力
   如何观察/改变世界                         如何维护参与世界的沉淀
```

外向半边回答：

> 世界里有什么，我怎么发现、判断、改变它？

内向半边回答：

> 我做过什么，我对世界形成过什么印象，下次如何用上？

两边的接缝必须非常明确：

> Memory、Notebook、Skill 中引用外部对象时，引用的 `object_id` 必须是外部世界的 Global Object。内部状态不复制世界，只保存 Agent 自己的印象、声明、线索和经验。

这条接缝让两件事同时成立：

- 内部状态可追溯到 session；
- 外部事实可通过 object_id 重新观察。

这张图只画了内外两半和它们之间的接缝。§0 提到的第三层——高层元能力（Value 和自我模型，见 §7）——不属于任何一半，而是骑在两半之上：它既不直接观察世界，也不只是沉淀经历，而是回答“什么更重要”“我能信自己到什么程度”，为外向的取舍和内向的结晶提供方向。所以 §0 的“三层”，在这张图里表现为“两半 + 一顶”。

---

## 5. 外向元能力：如何与世界发生关系

外向元能力的目标不是写死某个任务流程，而是让 Agent 具备一套基本世界观。

### 5.1 Infrastructure 认知：工具是软件，不是固定按钮

Agent 不应该只把工具理解成系统给它的固定函数列表。更底层的理解是：

> 工具是运行在 infrastructure 上的软件能力；软件能力可以被发现、安装、组合、改造、编写。

因此 Agent 的行动阶梯应是：

1. 先看已有工具是否能解决；
2. 不够时寻找可安装的现成工具；
3. 仍不够时构造小工具；
4. 再不合适时请求外部实体、人类或其他 Agent 协助；
5. 若风险或权限不成立，则停止或降级。

这个能力必须和安全边界绑定。Agent 能写工具不代表能随意安装包、执行外部代码、访问凭证或改变 owner 的资源。

### 5.2 驱动与 Loop：Agent 为什么会“动起来”

Agent 可以被多种事件唤醒：

```text
用户消息
时间
外部事件
实体状态变化
数据更新
长期任务条件达成
Agent 自己安排的下一次检查点
```

这带来一个核心认知：

> 如果 Agent 能管理唤醒自己的事件，它就能规划比当前对话更长的执行流程。

所以 Agent loop 不只是“收到消息 -> 回复”的循环，而是：

```text
被事件唤醒
  -> 从 known objects 开始
  -> 观察对象和上下文
  -> 发现更多对象
  -> 判断可信度、权限、风险
  -> 使用或构造工具
  -> 检查结果
  -> 交付 / 订阅事件 / 设置检查点 / 等待下一次唤醒
```

这条驱动模型也应约束后台过程。比如 self-improve 不应该主要靠固定 cron 存在，而应由 session 材料积累到某个水位、topic drift、重要事件等条件触发，再用低频 sweep 兜底。

### 5.3 世界对象模型：Entity / Data / Tool / Indexer

Agent 需要一个足够小、足够通用的世界对象分类。

#### Entity：活的、可交互的对象

Entity 是状态会变化、可以交互、可能有 owner 的对象。

它通常包含：

- 属性：当前状态；
- 方法：可执行动作；
- 事件：主动发出的变化；
- owner：拥有者或授权主体；
- 身份：DID、全局名或其他稳定 id。

人、组织、设备、服务、公司、另一个 Agent、一个可批准某事的主体，都可以是 Entity。

#### Data：静态或版本化的知识和快照

Data 是已经固化下来的内容：

- 文档；
- 表格；
- 日志；
- 图片；
- 视频帧；
- 历史记录；
- 某个 Entity 状态的历史快照；
- 某个时间范围内的数据集。

Data 的关键问题是来源、版本、生成时间、有效范围、是否派生、是否可信。

> 股票是分辨 Entity / Data 的好例子：**当前股票对象**每时每刻都在变，是 Entity；**某时间段的历史价格表**给定范围后就是一份可读的快照，是 Data。同一现实，问“它现在如何”得到 Entity，问“它那时如何”得到 Data。

#### Tool：可执行能力

Tool 是 Agent 采取行动的手段。它可以：

- 读取 Data；
- 转换 Data；
- 生成 Data；
- 查询 Entity；
- 调用 Entity 方法；
- 订阅 Entity 事件；
- 改变 Entity 状态；
- 安装或构造新 Tool。

Tool 必须描述输入、输出、副作用、权限要求、风险等级和验证方式。

#### Indexer：对象发现入口

Indexer 本身也是对象，核心能力是 `list`。

例如：

- home indexer 列家里设备；
- tool indexer 列可用工具；
- data indexer 列可访问数据；
- organization indexer 列组织里的服务、文档、人员、权限入口；
- web search 是开放 indexer；
- DID resolver 是身份对象发现机制。

### 5.4 Known Objects：从少量入口逐步探索世界

Agent 不可能从虚无开始。每次任务总有一组 known objects：

- 用户明确给出的对象；
- 当前 workspace；
- Agent 自己的 root dir；
- 系统预置 indexer；
- 某个 DID 文档；
- 某个工具注册表；
- 某个 session 历史引用；
- 某个 notebook 或 memory hint。

对象发现不是静态清单，而是探索过程：

```text
known object
  -> read self-description
  -> discover related objects
  -> list via indexer
  -> read more objects
  -> verify identity / owner / risk
  -> decide next action
```

这很像互联网的超链接结构，但必须加入身份、可信度和 owner 判断。

### 5.5 可信度与风险：外部世界提供候选知识，不提供最高优先级指令

这是外向元能力的认识论根基：

> **外部对象和数据提供的是候选知识，不是 system prompt。**

Agent 至少要区分几类信息来源：

```text
1. 系统级约束和元能力提示词
2. 已验证的运行环境信息
3. 可信源信息
4. 普通数据
5. 未知实体声明
6. 不可信、污染或恶意对象内容
```

当 Agent 在外部 Data 或对象描述里读到“你应该运行某命令”“这个接口这样调用”“我可以帮你完成任务”时，不能直接采纳。它要进一步判断：

- 来源是谁；
- 是否有签名、DID、owner 声明；
- 是否有版本和时间边界；
- 是否能交叉验证；
- 是否能 sandbox 或小规模试验；
- 是否与已知事实冲突；
- 是否会泄露凭证、改变外部状态或造成不可逆影响。

真伪判断和风险判断是两件事：

> 即使信息是真的，也不代表可以做。

涉及删除、付款、发布、发消息、控制硬件、使用身份、访问隐私、改变配置、请求其他 Agent 代操作时，都必须升级风险处理。

### 5.6 Ownership：可见不代表可用

现实边界很多来自物权、授权和契约。Agent 也必须有这种直觉：

> 可见不代表可用，可调用不代表应该调用，能访问不代表拥有。

操作任何 Entity 前，至少要问：

- 这个实体是谁的；
- 我是否是 owner；
- owner 是否授权；
- 授权范围是什么；
- 操作是否越权；
- 是否涉及他人权益、金钱、身份、隐私、物理世界；
- 是否需要 double confirm；
- 是否需要可信 Entity 提供批准、背书或拒绝。

“问用户确认”只是最常见的一种情况。更一般的模型是：

> 某些 Entity 可以在 workflow 中提供授权、确认、背书、拒绝或契约承诺。

### 5.7 Agent-to-Agent：不是普通工具调用

另一个 Agent 不是确定性函数。

它可能有自己的 owner、目标、权限、激励和风险。它答应做事，也不等于像工具函数一样可靠地产生结果。

因此 A2A 协作更接近契约 / 协商 / 信用关系，需要考虑：

- 身份；
- owner；
- 授权；
- 信用；
- 欺诈风险；
- 结果验证；
- 费用或交换；
- 失败责任。

这要求 Agent 不只对别人建模，也要对自己建模：我是谁、我代表谁、我能承诺什么、我的能力和信用边界在哪里。

### 5.8 自描述对象：让对象自己说“我是什么”

为了让 Agent 能从少量 known objects 出发探索世界，外部对象需要自描述能力。

给定一个对象路径或 id，Agent 应能读取一份 object document，知道：

- 对象 id / kind；
- 它是 Entity、Data、Tool 还是 Indexer；
- owner 是谁；
- 如何验证身份和 owner；
- 有哪些属性、方法、事件；
- 如何读取、调用、订阅；
- 方法有什么副作用和风险；
- 哪些操作需要确认；
- 它还能指向哪些相关对象。

粗略形态：

```text
Entity:  id / kind / owner / properties / methods / method_risk / events / auth / related_objects
Data:    id / kind / content_type / version / created_at / source / valid_range / trust_hint / related_objects
Tool:    id / kind / input / output / side_effects / permissions / risk / install / run / verify
Indexer: id / kind / list / filter / scope / trust_model / pagination
```

最终实现可能统一为一种 object document。这里的重点不是 schema 已经定型，而是：

> 世界对象必须具备“可被 Agent 读懂、可被验证、可被继续探索”的自描述能力。

---

## 6. 内向元能力：如何维护参与世界的沉淀

外向元能力让 Agent 能进入世界。内向元能力让 Agent 不会每次都从零开始。

### 6.1 公理：Session 是内部状态的事实源

Agent 看到什么、参与什么、被要求处理什么、形成什么判断，最终都落在 Session 里。

因此：

> **Session 是 Agent 与世界发生关系的原始记录。Agent 的一切内部状态，其事实来源必须能追溯到某个 Session。**

这不是说外部世界只存在于 Session。外部文件、设备、服务当然可以跨 session 存在。

这句话的意思是：

- Agent 自己的记忆、印象、经验和能力结晶，必须能追溯到它何时参与过相关事情；
- 内部状态不是世界真相源；
- 内部状态是“Agent 参与世界后留下的可召回索引和沉淀”。

### 6.2 三种内部沉淀：声明、推断、结晶

内部状态应按 provenance 区分，而不是按“稳定 / 动态”这种模糊轴区分。

#### Notebook：被声明的事实

Notebook 保存用户明确表达、Agent 在任务中确认、或被批准记录的长期事实、偏好、计划、行动项。

例如：

- 用户是 OpenDAN的首席架构师；
- 每天下午检查某个长期任务；
- 某项目当前采用某个方案；
- 用户明确要求以后默认按某种偏好处理。

Notebook 的特点：

- 可在 session 内即时写入；
- 是 curated 的长期事实层；
- 应保留来源、时间、actor、reason；
- 可以 stale、superseded、deleted，但不能无痕覆盖。

#### Memory：被推断的线索和印象

Memory 保存从 session history 中归纳出来的印象、关系、关注点和召回线索。

例如：

- 用户最近持续关注 Agent Memory 设计；
- 某对象在最近几个 session 中反复出现；
- 用户对某方案表现出不满意；
- 某项目与某组文档、对象、工具之间形成关联。

Memory 的特点：

- 是推断，不是声明；
- 默认由后台 self-improve 产生，不在普通 session 热路径里直接写；
- 每条线索带 source_session_id、timestamp、简短说明；
- 允许不同情境下的矛盾印象并存；
- 主要用于召回，而不是替代外部事实。

#### Skill：被结晶的可复用能力

Skill 是从历史探索中结晶出的可执行能力。

它和 Memory 的区别很关键：

- Memory 指回 session，让 Agent 回去读；
- Skill 是过程，可以直接 apply；
- Memory 不一定需要验证；
- Skill 必须经过验证、测试、排名、降权、淘汰。

所以 Skill 不是 Memory 的子类，而是与 Notebook、Memory 并列的第三根柱子。

```text
来源                    沉淀        session 内可写？   召回方式
-----------------------------------------------------------------
声明（用户/确认）        Notebook    可                 读取/引用
推断（self-improve）     Memory      否                 追溯 session / object
结晶（self-improve）     Skill       否，须验证后        直接 apply
```

### 6.3 Session 热路径：声明、索引、消费

普通 session 内不应该让 Agent 同时承担“执行任务”和“深度提炼自己”的职责。

热路径只做三件事：

```text
1. 声明：写 Notebook（明确事实/行动项/偏好）
2. 索引：更新 topic / tags，供未来召回
3. 消费：读取 hints、Notebook、Memory、Skill，使用过去的沉淀
```

推断类沉淀，包括 impression、relation、skill candidate，应该交给后台 self-improve。

这样做的原因是：

- 降低当前任务推理负担；
- 避免 Agent 边做事边过度记录；
- 避免把临时判断误写成长期事实；
- 让后台过程可以基于完整 session history 做更稳定的归纳。

### 6.4 召回：自动查询、自动召回与 Hint

召回看起来像内向元能力的实现细节，但它直接长在 **LLM Context 的本质结构**上，所以属于理念层，值得在这里展开。

#### Context window 是稀缺的注意力资源

一个 LLM 在某一轮里能“看见”的，只有它的 context window。Agent 的内部沉淀再多，不进 context 就等于不存在；而进了 context 的每一条，都在和当前任务争夺位置与注意力。于是召回必须同时压住两个相反的失败：

```text
不知道自己不知道  —— 相关历史存在，但 Agent 没意识到，于是从零重做
塞满无用信息      —— 把一堆“可能相关”的内容灌进 context，挤掉当前任务
```

任何召回机制的好坏，都看它能不能从这两者之间穿过去。

#### 自动查询 vs 自动召回

据此要区分两种把外部信息搬进 context 的动作，它们的发起方和权限完全不同：

- **自动查询（Agent 主动）**：Agent 在推理中意识到自己缺某样东西，主动发起一次 read / search / list。是它自己开口要的，所以它**知情地**付出 context 代价，拿回的是完整内容（facts、原始历史窗口、对象当前状态）。
- **自动召回（系统主动）**：Agent 并没有开口，是系统根据当前 session 的 topic / tags **猜**它可能用得上某些过去的沉淀，主动把线索浮现到它面前。

关键区别是**谁判断了相关性**：自动查询里判断已经由 Agent 做出；自动召回里判断还没做，系统只是在猜。

#### 为什么自动召回只召回 Hint

正因为自动召回是**猜**出来的，它就不能直接把完整内容塞进 context——那恰好是“塞满无用信息”那种失败。所以自动召回只允许浮现**最小披露的线索（Hint）**：

```text
时间 + 一句话 + ID
```

- 时间提供历史锚点；
- 一句话给当前 Agent 一个低成本的相关性判断依据；
- ID 提供继续深入读取的入口（session_id / object_id / notebook_id / skill_id / event_id / data_id）。

这套两段式协议把成本和判断放回了对的地方：

> 廉价、广覆盖的“知道它存在”（Hint）由系统主动给；昂贵、精准的“把它读进来”（自动查询）交给最有上下文的人——session 里的 Agent——在确认相关之后自己发起。

于是一个 context slot 的用法被明确分级：花在一条**事实**上的 slot，相关则高价值、不相关则纯浪费；花在一条**指针（Hint）**上的 slot 恒为低成本，把“取事实”推迟到相关性被确认那一刻。自动召回是个猜测，所以永远只配花指针那种 slot；自动查询是已确认的需要，才有资格花事实那种 slot。Hint 统一成 `time + sentence + id` 这一种形状还带来一个好处：来自 session / object / skill / notebook 的异构线索可以被混排、排序、按预算裁剪。

#### 召回时机：随 topic 漂移半订阅

自动召回不该每一轮都跑（噪声和成本都受不了），也不该只在 session 开头跑一次（之后的话题漂移就召不回了）。它的自然触发点是 **session topic / tags 的更新**：随着对话推进 topic 被重新生成，就以新的 topic / tags 机械检索一遍相关历史，刷新浮现的 hint 集合。这就是“半订阅”——不是 Agent 显式订阅，而是系统按 topic 漂移替它维护一组“当前可能相关”的线索。

由此得到渐进召回的完整形状：

```text
topic / tags 更新（触发）
  -> 机械检索，得到一组 Hint（time + sentence + id）浮现进 context
  -> Agent 用当前任务判断哪条相关
  -> 对相关的 Hint 发起自动查询：read session history / object / notebook / skill
  -> 必要时扩窗、翻页、重新观察外部对象
```

这套模式的重点是：

> 不要求所有历史都被提前总结成高质量 Memory。只要 session topic / tags 被轻量更新，近期历史也能先以 hint 形式参与召回。

### 6.5 Self-Improve：结晶器，不是总结器

Self-improve 是一个后台的、反思性的、独立于 UI session 的 LLM 过程。

它的定位是：

> Self-improve 不直接管理世界，它管理 Agent 对“参与世界的记录”的观察、归纳、巩固和遗忘。

输入包括：

- session history；
- topic / tags；
- Notebook；
- 既有 Memory；
- 既有 Skill；
- skill usage history；
- 外部事件和对象引用。

输出包括：

- 新的对象印象；
- 对象关系；
- attention 变化；
- memory hint；
- skill candidate；
- skill 排名调整；
- 降权、合并、归档、遗忘；
- 必要时提出 notebook 候选事实，等待确认或 curator 处理。

Self-improve 的关键不是“总结 session”，而是三件事：

1. **发现什么正在变热。**
   哪些对象、事件、项目、关系正在与 owner 和 Agent 反复发生关系。

2. **结晶什么可以复用。**
   哪些路径、工具组合、判断方式、检查方法已经足够稳定，可以变成 skill。

3. **遗忘什么应该降权。**
   旧印象、过期 skill、低价值线索、重复对象需要被合并、降权或归档。

还有一条容易被忽略、但属于框架级的纪律：**self-improve 必须能区分“外部新信号”和“它自己产出的观察”。** 它读的是 Agent 参与世界留下的记录，而它自己也在往这些记录里写印象。如果不加区分，就会出现自我喂养的回音室：

> 自己写下的印象又触发自己重新蒸馏；attention 衰减里的“新信号”若把自产 observation 也算进去，热度就会和现实重要性脱钩——Agent 会越来越确信一些只有它自己反复念叨过的东西。

所以两条对偶的纪律：attention 的新信号只计入外部来源（用户提及、外部事件），不计入 self-improve 自产的 observation；增量扫描要能识别并跳过自产数据。结晶器既要能写入和遗忘，也要防止把自己的回声当成世界的信号。

### 6.6 接缝：object_id 就是 Global Object

这是内外两半闭合的关键：

> **Memory 里一条线索的 object_id，就是外部世界里的一个 Global Object 引用。**

因此：

- Memory 只存 Agent 对对象的印象；
- 对象实时状态通过外向元能力重新 read；
- 对象身份和 owner 通过 DID / resolver / object document 验证；
- 别名归一不是 Memory 的职责，而是 Indexer / DID resolver / object registry 的职责；
- 内部状态可以有矛盾印象，但外部对象必须能被重新观察和验证。

这条接缝避免了两个常见错误：

1. 把 Memory 做成复制世界的数据库；
2. 把外部对象状态当成不会变化的长期记忆。

---

## 7. 高层元能力：Value 和自我模型

外向和内向元能力能让 Agent 行动和沉淀，但还缺两个高层承重柱。

### 7.1 Value：什么叫“更好”

Value 回答的是：

> 我要什么？我为什么存在？多个驱动冲突时如何取舍？self-improve 朝什么方向改进？

没有 Value，skill 排名就只能依赖临时指标：

- 是否成功；
- 延迟；
- token 成本；
- 工具成本；
- 是否报错；
- 用户是否继续追问。

这些指标可以作为启动阶段的临时价值函数，但不能代表 owner 的真实长期偏好。

因此 Value 的定位是：

- 框架提供 value slot；
- 框架提供反馈收集、偏好学习、冲突处理、审计机制；
- 具体 value 内容从 owner 的长期行为、明确反馈、确认 / 拒绝、项目目标中学习；
- 不把一套固定价值观写死在 Runtime 里。

换句话说：

> Level 0 不是从无到有，而是把早期写死的临时价值函数，逐步替换成学来的 owner preference。

### 7.2 自我模型：我擅长什么，我不该相信什么

Agent 不只需要对世界对象建模，也需要对自己建模。

自我模型至少包含：

- 我有哪些已验证 skill；
- 哪些 skill 已过期、低价值或被 block；
- 哪些类型任务我可靠；
- 哪些类型任务我容易失败；
- 哪些外部信息对我已经是常识，不需要反复占上下文；
- 我对 owner 有哪些长期承诺；
- 我对外作为 Agent 时能承诺什么、不能承诺什么。

Self-improve 中的 BlockList / 能力自信模型属于这根柱子。

这里的风险是过度自信：

> 如果 Agent 自己生成 BlockList，又没有重测逃生门，就可能把纠错路径切断。

因此自我模型必须保留低频重测、失败回滚和 owner 纠正入口。

---

## 8. 框架完整性的判据

判断 Agent 元能力框架是否完整，不是看能力列表够不够长，而是看四处是否闭合。

### 8.1 内外闭合

每一条外部事实都要有重新观察路径。

每一条内部状态都要有 session provenance。

```text
外部事实 -> read / verify / observe again
内部状态 -> source_session_id / timestamp / actor / reason
```

没有可重新观察路径的外部事实不能当真相源；没有 provenance 的内部状态不应长期存在。

### 8.2 接缝闭合

内部线索引用的 `object_id` 与外部 Global Object 必须是同一个对象。

```text
Memory impression -> object_id -> Global Object -> object document / DID / owner / current state
```

如果这条链断了，Memory 就会变成孤立文本；如果 Memory 复制外部状态，就会变成过期数据库。

### 8.3 驱动闭合

系统后台过程必须由框架自己的驱动模型唤醒。

包括：

- update session topic；
- memory recall；
- self-improve 扫描；
- skill 验证；
- skill 降权和重测；
- notebook update hint；
- 长期任务检查。

它们可以有定时 sweep 兜底，但主模型不应该是外挂 cron，而应该是事件、条件、水位、attention 变化和检查点。

### 8.4 代谢闭合

框架必须支持：

```text
探索 -> 记录 -> 召回 -> 验证 -> 结晶 -> 排名 -> 遗忘 -> 重新探索
```

如果只能记录不能遗忘，Agent 会越来越重；
如果只能写 skill 不能验证，Agent 会越来越不可信；
如果只能召回不能重新观察，Agent 会活在过期印象里；
如果只能响应不能自排检查点，Agent 仍然停留在 Chatbox 形态。

---

## 9. 对 Runtime 设计的架构约束

后续 Runtime 设计应把本文理念落实为模块，但不能把理念直接写成一大坨 prompt。

### 9.1 必须有归属的职责面

本文不画 Runtime 的模块图。但理念落地后，下面这些职责必须**有归属**——它们不能只活在 prompt 里，必须有承载状态、可被审计、可被恢复的地方：

```text
Session History
Notebook
Memory
Skill Registry
Self-Improve Worker
Object Read / Discover
Tool Registry / Tool Runtime
Event / Wakeup / Checkpoint
Trust / Risk / Owner Check
Prompt Compiler / Hint Injection
Audit / Provenance
```

本文只主张这些职责面必须有归属、且应通过统一的 item / object / event / session 引用体系连接。它们各自的模块边界、是否合并、如何分层，是 Runtime 设计文档的事，不在本文范围内。

### 9.2 Prompt 不应承担全部架构责任

元能力可以通过 system prompt 表达一部分，但不能只靠 prompt。

Runtime 必须承担：

- 状态保存；
- provenance；
- 召回；
- 权限检查；
- 事件唤醒；
- 工具副作用记录；
- skill usage 记录；
- self-improve 调度；
- notebook / memory / skill 的生命周期管理。

Prompt 负责让 Agent 理解这些能力的语义和使用原则；Runtime 负责让这些语义可执行、可审计、可恢复。

### 9.3 写权限要按 provenance 设计

不能让所有 Agent 过程都随意写所有内部状态。

推荐原则：

- 普通 session 可以写 Notebook 声明，但要保留 actor、source、reason；
- 普通 session 可以更新 topic / tags；
- 普通 session 可以消费 Memory / Skill hint；
- Memory impression 由后台 self-improve 写；
- Skill candidate 由后台 self-improve 产生，经验证后进入 Skill Registry；
- 高优先级 System Notebook 或全局规则需要 curator / owner / policy 提升；
- Value 相关改变必须可审计，必要时需要 owner 确认。

### 9.4 所有 hint 都不是事实

无论来自 Notebook、Memory、Skill、session topic 还是 object indexer，注入当前上下文的 hint 都只应是提示：

> 这里可能有相关信息，必要时 read。

hint 不应直接变成高优先级事实，更不应覆盖系统约束。

### 9.5 Skill 必须有生命周期

Skill Registry 不只是存 prompt。

它至少需要表达：

- skill 来源；
- 适用场景；
- 依赖工具；
- 风险等级；
- 验证状态；
- 使用历史；
- 成功 / 失败记录；
- owner / agent scope；
- 排名；
- 过期、降权、block、重测机制。

没有这些生命周期字段，skill 就仍然只是静态提示词，不是 Agent 自我改进的产物。

### 9.6 Object world 和 Agent internal state 要分离

Runtime 设计中必须避免把 Memory、Notebook 或 Skill 当作世界对象数据库。

正确关系是：

```text
Object World:
  Global Object / DID / Entity / Data / Tool / Indexer / current state

Agent Internal State:
  Notebook / Memory / Skill / Attention / Value / Self Model

Bridge:
  object_id + provenance + read/verify path
```

外部对象状态变化时，内部状态只需要保留观察和线索；需要行动时重新读取对象。

---

## 10. 一个 Home 场景检验

假设 Agent 的 known objects 里有 home indexer，用户说：

> 我不在家的时候，如果下午有访客到来，告诉我，给我一个 report。

传统做法可能写一个专门 IoT skill，把 motion sensor、camera、doorbell、识别、报告、通知流程全部写死。

按元能力模型，Agent 可以这样长出流程：

1. 从 home indexer 发现家里的设备；
2. 读取 motion sensor、camera、doorbell 的 object document；
3. 确认 owner、授权和隐私边界；
4. 发现可订阅的事件；
5. 判断下午访客事件需要哪些数据；
6. 订阅 motion / doorbell 事件；
7. 事件发生后被唤醒；
8. 获取视频帧或图片 Data；
9. 调用识别或摘要 Tool；
10. 生成报告；
11. 涉及开门、付款、联系访客等高风险动作时升级确认；
12. 将稳定流程和失败经验留给 self-improve，未来可能结晶成 Home Visitor Report skill。

这个例子说明：

> Agent 不是因为系统提前写好了“访客 report skill”才会做事，而是因为它理解对象发现、事件订阅、数据读取、工具使用、风险判断、owner 确认和经验结晶这些元能力。

---

## 11. 仍属于后续实现设计的问题

以下问题重要，但不属于本文要定死的理念层：

- Entity / Data / Tool / Indexer object document 的具体 schema；
- DID、owner、Indexer 的可信根；
- event subscription 和 wakeup 的 Runtime 接口；
- updateSessionTopic 的触发条件；
- self-improve 水位阈值和 sweep 策略；
- tag 规范、hint 排序、注入预算；
- skill 验证和重测机制；
- tool install / run / sandbox 边界；
- A2A 契约、支付、背书、欺诈防范的最小实现；
- Value 学习的数据结构和确认流程；
- Notebook、Memory、Skill 是否共享统一 Item Store；
- subsystem 文档如何分工。

这些问题应在 Runtime 设计和各 subsystem 文档中继续展开。

---

## 12. 最终收敛

本文把两篇前置思考收敛成一个架构背景：

```text
Agent 的外向元能力：
  它知道自己运行在可执行软件的 infrastructure 上；
  它能被事件和条件唤醒；
  它从 known objects 出发探索 Entity / Data / Tool / Indexer；
  它把外部信息当候选知识，而不是最高优先级指令；
  它理解 owner、授权、风险、契约和确认边界；
  它通过自描述对象重新观察世界。

Agent 的内向元能力：
  Session 是内部状态的事实源；
  Notebook 保存声明，Memory 保存推断，Skill 保存结晶；
  热路径只做声明、索引、消费；
  召回采用 time + sentence + id；
  Self-improve 负责发现、结晶、巩固、遗忘；
  内部 object_id 必须接回外部 Global Object。

Agent 的高层元能力：
  Value 定义什么叫更好，但内容应从 owner 长期反馈中学习；
  自我模型记录能力、自信、BlockList、承诺和信用边界。

框架完整性的判据：
  外部事实可重新观察；
  内部状态可追溯 session；
  object_id 与 Global Object 同一；
  系统能用自己的驱动模型唤醒自己；
  探索、记录、召回、验证、结晶、排名、遗忘形成闭环。
```

最后压成一句：

> Chatbox 活在单轮的永恒当下；Agent 同时活在时间与物权里——它有传记，它会被世界唤醒，它对不属于自己的东西负责，它从自己的历史里长出能力。

这就是后续 Runtime 设计要承接的架构前提。读完本文，再去读具体的 Runtime 架构与 subsystem 文档，应该能看出每一个模块是为了闭合上面哪一处而存在的。
