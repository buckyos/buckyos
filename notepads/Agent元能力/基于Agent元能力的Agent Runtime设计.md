# 基于 Agent 元能力的 Agent Runtime 设计

> 本文是理解 OpenDAN Agent Runtime 的总架构文档。
>
> 上游理念来自 [Agent元能力设计.md](Agent元能力设计.md)。本文不重复证明“为什么 Agent 需要元能力”，而是回答：为了把这些元能力落成可执行、可审计、可恢复的 Runtime，Agent Core 应该长成什么样。
>
> 本文的直接输入还包括 [基于Agent元能力的Agent Runtime设计 TODO.md](基于Agent元能力的Agent%20Runtime设计%20TODO.md) 与 [todo.md](todo.md)。前者给出 Runtime 最小闭包，后者给出当前工程实现需要推进的事项。本文负责把二者收敛成一篇可用于 review subsystem 文档和指导后续实现的架构设计。

---

## 0. 核心结论

如果把 Session 定义为：

```text
处理一次 input/event 触发的 LLM Loop
```

那么 Agent Core 的最小闭包是：

```text
Agent Core =
  Session Runtime
  + Event Runtime
  + Object Runtime
  + Governance Runtime
  + State & Recall Runtime
  + Self-Improve Runtime
```

一句话压缩：

```text
Session      负责“醒来之后怎么想、怎么做”
Event        负责“为什么醒来、如何等待、如何恢复”
Object       负责“世界是什么、如何发现、读取、操作”
Governance   负责“什么可做、谁授权、何时停下”
State&Recall 负责“我经历过什么、如何低成本想起来”
Self-Improve 负责“如何从历史里长出能力”
```

这 6 个组件中，`Session Runtime` 是热路径；另外 5 个是 Session 之外的最小闭包。少任何一个，Agent 都会退化：

| 缺失组件 | 退化结果 |
|---|---|
| Event Runtime | 只能被用户消息唤醒的 chatbox with tools |
| Object Runtime | 只能处理文本或预置按钮的 workflow executor |
| Governance Runtime | 没有 owner / 授权 / 风险边界的自动脚本 |
| State & Recall Runtime | 每个 Session 都重新出生的 stateless chatbot |
| Self-Improve Runtime | 只有记忆、不能生长能力的 memory assistant |

再多出来的顶层组件，大多应归入三类之一：

```text
1. 某个 Runtime 内部的 schema
2. 某个 Runtime 内部的策略
3. 某个具体 skill / workflow / 业务流程
```

Agent 框架要保留“代谢器官”，不要把代谢出来的具体内容硬编码进 Core。

---

## 1. 文档边界

本文是 Runtime 架构设计，不是每个 subsystem 的接口规格。

本文负责确定：

- Agent Core 的顶层组件；
- 每个组件的职责边界；
- 组件之间的数据流；
- 哪些能力必须由 Runtime 承载，不能只靠 prompt；
- 当前工程 TODO 应归属到哪个组件；
- 后续 subsystem 文档应如何分工。

本文不定死：

- Global Object document 的完整 schema；
- Event / Wakeup 的全部 API；
- Governance policy 的具体表达语言；
- Self-Improve 的阈值、打分模型和 LLM prompt；
- Skill 的最终文件格式；
- Value 学习的完整算法；
- A2A 契约、支付、背书和信用体系的完整实现。

这些属于后续 subsystem 详细设计。

---

## 2. 总体架构

```text
                  ┌─────────────────────────────┐
                  │         Owner / User         │
                  └──────────────┬──────────────┘
                                 │ input / approve / feedback
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                         Event Runtime                           │
│ EventLog / WakeupQueue / Timer / Condition / Task Checkpoint     │
└──────────────┬───────────────────────────────────┬──────────────┘
               │ wake / resume                     │ event / trace
               ▼                                   ▼
┌─────────────────────────────┐       ┌────────────────────────────┐
│       Session Runtime       │◄─────►│    State & Recall Runtime  │
│ UI Session / Work Session   │ hints │ Notebook / Memory / Skill  │
│ Prompt / LLM Loop / Trace   │ read  │ Recall / Self Model / Value│
└──────────────┬──────────────┘       └──────────────┬─────────────┘
               │ object read/call/sub                 │ candidates
               ▼                                      ▼
┌─────────────────────────────┐       ┌────────────────────────────┐
│       Object Runtime        │◄─────►│    Self-Improve Runtime    │
│ Global Object / Tool / Data │ history│ observe / verify / promote│
│ Indexer / Entity / Agent    │       │ rank / decay / forget      │
└──────────────┬──────────────┘       └──────────────┬─────────────┘
               │ request action / side effect         │ policy signal
               ▼                                      ▼
┌─────────────────────────────────────────────────────────────────┐
│                       Governance Runtime                         │
│ Identity / Owner / Capability / Risk / Approval / Audit / Value   │
└─────────────────────────────────────────────────────────────────┘
```

重要原则：

1. `Session Runtime` 不直接拥有世界。它通过 `Object Runtime` 观察和操作世界。
2. `Session Runtime` 不直接相信 hint。它通过 `State & Recall` 获得线索，再主动读取事实。
3. `Object Runtime` 不决定能不能做高风险动作。它把 owner、method risk、side effect 暴露给 `Governance Runtime`。
4. `State & Recall` 不是世界副本。内部状态里的 `object_id` 必须能回到外部 Global Object 重新读取。
5. `Self-Improve Runtime` 不在热路径里抢任务。它观察历史、生成候选、验证、提升、降权和遗忘。
6. `Governance Runtime` 不是 prompt 里的安全提醒。它是权限、确认、审计和停机边界。

---

## 3. Agent 生命周期

一个典型 Agent 行动链路如下：

```text
input / event / timer / condition
  -> Event Runtime 写入 EventLog，放入 WakeupQueue
  -> 选择或创建 Session
  -> Session Runtime 编译 prompt：
       system constraints
       behavior prompt
       tool/object affordance
       notebook registry / system notebook
       background hints
       recent history window
  -> LLM Loop：
       read hints
       read object / notebook / session history
       call tools / object methods
       update session topic
       append explicit notebook facts
       request approval if needed
  -> 产生 response / action / report
  -> 写 Session History / Event trace / Audit
  -> 必要时创建 checkpoint、订阅 object event、安排 task
  -> Self-Improve 在水位或事件触发后处理历史
```

这里最关键的分离是：

```text
EventLog 是 Agent 与世界发生关系的原始事件流；
Session History 是其中经过一次 LLM Loop 处理后的可读工作轨迹；
LLM Context Snapshot 只是下一次喂给模型的派生窗口。
```

因此不能再把“压缩后的 LLM context”当作 Agent 历史的唯一真相源。Session History 必须 append-only、可读、可追溯；context 可以压缩，history 不能被压缩破坏。

---

## 4. Session Runtime

Session Runtime 处理一次被唤醒后的当前推理和行动。

```text
input/event
  -> assemble context
  -> LLM loop
  -> tool/object calls
  -> response/action/report
  -> trace
```

Session 只解决“醒来以后怎么想、怎么做”。它不负责决定所有未来唤醒，不复制整个世界，不在热路径里深度总结自己。

### 4.1 Session 类型

Runtime 至少应区分两类 Session。

#### UI Session

UI Session 是 long lifetime session，负责与用户保持关系和理解输入真正含义。

职责：

- 接收用户输入；
- 基于历史理解 input intent；
- 更新 session topic / tags；
- 路由到合适的 Work Session；
- 处理用户 approve / reject / clarify；
- 在必要时读取 Notebook、Memory hint、其它 Session 给用户发过的消息；
- 维护与当前 UI 通道相关的上下文。

UI Session 的失败重点是路由错误。Input 如果自带 Target Session，就应直接送达；否则走：

```text
Input -> UI Session -> LLM Route -> Work Session
```

当前工程 TODO 中，以下事项归属 UI Session：

- 优化 `update_session_topic` 使用方式；
- 群聊支持；
- 看到更多历史消息；
- 在 UI Session 中用命令处理 approve；
- 识别机械 forward 标签；
- 优化 Route 相关 prompt。

#### Work Session

Work Session 面向一个确定任务，通常不是 long lifetime session。OpenDAN 的世界观里，Agent 应由少量 UI Session 和大量 Work Session 组成。

职责：

- 承接一个明确任务；
- 在 Plan 阶段充分了解上下文；
- 在 Do 阶段专注执行已确定 TODO；
- 快速失败，不在执行阶段无限探索；
- 处理 task report；
- 处理 workspace 和产物交付；
- 记录完整 Session History。

Work Session 的核心价值是天然缓解 context window 问题：复杂工作拆进多个任务 Session，而不是把所有历史塞进一个 UI Session。

### 4.2 Plan / Do 分层

Work Session 内应保留 Plan / Do 差异。

```text
Plan:
  目标是了解更多、形成任务图、确定依赖和风险。
  可以读取更多 history / object / notebook。
  可以提出需要用户确认的问题。

Do:
  目标是完成已明确的 TODO。
  默认认为 Plan 阶段已经收集足够信息。
  不做大范围探索。
  遇到关键缺口时快速失败或回到 Plan。
```

这意味着 Plan 阶段 prompt 需要基于元能力框架升级：它不仅要“制定计划”，还要会发现对象、风险、权限、可复用 skill、必要 checkpoint。

### 4.3 Session History

Session History 是 Session Runtime 的基础资产。

目标：

- 与 LLM Context Snapshot 解耦；
- append-only；
- 按 round 可读；
- 保留 chat / behavior 的结构化载荷；
- 支持审计、回放、前端展示和 self-improve；
- 允许 LLM 通过文件工具读取 `round_history/` 做自我回忆。

建议继续沿用 [session-history.md](session-history.md) 的方向：

```text
{session_dir}/
├── round_history/
│   ├── 000001.jsonl
│   ├── 000002.jsonl
│   └── ...
└── .meta/
    └── round_logs.jsonl
```

`round_history/` 是可读资产，`.meta/round_logs.jsonl` 是内部导航索引。普通文件工具不应改写 history。

### 4.4 Prompt Compiler

Prompt 不能承担全部架构责任，但 Prompt Compiler 是 Runtime 和 LLM 的接缝。

Prompt Compiler 应负责把以下内容按预算和优先级装配进 context：

- system constraints；
- behavior prompt；
- current input / event；
- Session state；
- Notebook registry；
- 少量 System Notebook；
- background hints；
- tool / object affordance；
- recent compressed LLM context；
-必要的 governance policy 摘要。

它不应把大量 Notebook、Memory、Skill、Session History 直接注入。默认只注入 registry 和 hint，让 Agent 在确认相关后主动读取。

### 4.5 Hook Point

Session Runtime 需要稳定 hook point，而不是让各模块靠 prompt 旁路猜测。

至少需要：

```text
before_context_compile
after_context_compile
before_llm_call
after_llm_call
before_tool_or_object_call
after_tool_or_object_call
before_session_finalize
after_session_finalize
```

TODO 中提到的 `process-channel` 重新实现 hook point，应该服务于这个目标：让 Notebook、Hint Recall、Governance、History Writer、Self-Improve trigger 都有明确挂载点。

---

## 5. Event Runtime

Event Runtime 回答：

> Agent 在没有用户输入时，凭什么继续存在？

Agent 不只是被用户消息唤醒，还会被时间、外部事件、对象状态变化、数据更新、长期任务条件和自排检查点唤醒。

### 5.1 职责

Event Runtime 至少包含：

```text
EventLog
WakeupQueue
Timer Trigger
Condition Trigger
Subscription Manager
Run / Task State
Checkpoint Manager
```

它负责：

- 记录原始事件；
- 接收 user input、system event、object event、tool callback；
- 根据订阅和计划把事件路由到 Session；
- 恢复 waiting / pending / interrupted task；
- 创建和维护长期 checkpoint；
- 给 Self-Check / Self-Improve 提供触发条件。

### 5.2 Event 与 Session 的关系

不是所有事件都属于用户对话 Session。

例如：

- timer tick；
- webhook；
- 文件变化；
- object event；
- 审批结果；
- 其它 Agent 回复；
- 后台 self-improve 完成；
- schedule task 执行报告。

这些事件都应先进入 Event Runtime，再决定是否创建或恢复 Session。

```text
EventLog 是原始记录；
Session 是 Event 被一次 LLM Loop 消费后的视图；
Round 是 Session 内一次具体消费。
```

### 5.3 Self-Check 的位置

Self-Check 属于 Event Runtime 与 State & Recall 之间的后台计划整理器。

它的职责不是执行任务，而是：

- 消费 Notebook 中的计划、提醒、周期任务、模糊意图；
- 判断是否需要创建 / 更新 / 取消 Schedule-Task；
- 写入 Self-Check Review 标记；
- 读取执行报告，维护计划系统；
- 控制成本和上下文窗口。

因此：

```text
Notebook Item
  -> Self-Check Review
  -> Schedule-Task / Reminder / Agent Task
  -> Event Runtime 到点唤醒
  -> Work Session 执行
  -> Execution Report
  -> Self-Check 复查计划是否仍有效
```

Self-Check 不直接替代 Work Session，也不应在没有计划任务的情况下临时执行开放任务。

---

## 6. Object Runtime

Object Runtime 回答：

> Agent 如何把世界理解成可发现、可读取、可操作、可重新验证的对象网络？

这里的核心抽象不是 tool，而是 Global Object。Tool 只是 object 的一种。人、组织、设备、服务、另一个 Agent、文件、API、数据库、Indexer、DID document，也都应是 object。

### 6.1 Object Kind

顶层 Runtime 不拆出 Entity Runtime / Data Runtime / Tool Runtime / Indexer Runtime。它们是 Object Runtime 内部的 object kind。

```text
Object {
  id
  kind: entity | data | tool | indexer | agent | ...
  owner
  manifest
  read
  methods
  events
  trust
  risk
  related_objects
}
```

#### Entity

Entity 是活的、可交互、状态会变化、可能有 owner 的对象。

例子：

- user；
- device；
- service；
- organization；
- another agent；
- home sensor；
- approval authority。

#### Data

Data 是静态或版本化的知识和快照。

例子：

- document；
- table；
- log；
- image；
- video frame；
- session history；
- entity snapshot；
- dataset。

#### Tool

Tool 是可执行能力。

它必须描述：

- input；
- output；
- side effects；
- permission；
- risk；
- install / run；
- verify。

#### Indexer

Indexer 是对象发现入口。它也是 object，核心能力是 list / search / resolve。

例子：

- home indexer；
- tool indexer；
- data indexer；
- organization service indexer；
- web search；
- DID resolver。

### 6.2 标准动作

Object Runtime 对 Session 暴露的最小动作应收敛为：

```text
read(object_id)
list(indexer_id, filter)
call(object_id, method, params)
subscribe(object_id, event_name, options)
unsubscribe(object_id, event_name)
resolve(alias_or_did_or_url)
```

其中 `read(object_id)` 是最重要的动作。

`read` 返回的不只是 raw data，而是一份给 Agent 使用的引导文本或 object document，让 Agent 知道：

- 这是什么；
- 谁拥有它；
- 如何验证；
- 可以读什么；
- 可以调什么；
- 能订阅什么事件；
- 相关对象有哪些；
- 哪些动作有副作用；
- 哪些动作需要确认。

### 6.3 Known Objects

Agent 不从虚无开始。每次 Session 都有一组 known objects：

- 用户明确给出的对象；
- 当前 workspace；
- Agent root dir；
- session dir；
- Notebook / Memory / Skill hint 指向的 id；
- 系统预置 indexer；
- DID resolver；
- tool registry；
- object registry。

Object discovery 是从 known objects 出发的探索过程：

```text
known object
  -> read self-description
  -> discover related objects
  -> list via indexer
  -> read more objects
  -> verify identity / owner / risk
  -> decide next action
```

### 6.4 Agent Tool 体系

当前工程里 Agent tool 体系包含 `agent-memory`、`agent-notebook`、文件工具、shell 以及其它 CLI / service 能力。Runtime 设计上应把 tool 视作 object kind，而不是把 tool 表硬编码成唯一世界入口。

TODO 中“支持 agent tool 索引器，可以了解当前环境、可安装工具、已有工具”归属 Object Runtime。

Tool Indexer 至少应能回答：

- 当前 Agent 环境里有哪些 tool；
- 每个 tool 的 manifest；
- tool 需要什么权限；
- tool 的运行目录和作用域；
- tool 是否可安装；
- tool 运行后如何验证；
- tool 是否属于高风险动作。

### 6.5 Object 与内部状态的接缝

Memory、Notebook、Skill 中引用外部对象时，必须保存 `object_id` 或可 resolve 到 `object_id` 的指针。

```text
Memory hint -> object_id -> read(object_id) -> current object document
```

内部状态不能复制世界。它只能保存 Agent 自己参与世界后留下的印象、声明、线索和经验。行动前必须能回到 Object Runtime 重新观察。

---

## 7. Governance Runtime

Governance Runtime 回答：

> Agent 看得见某个对象、调得动某个工具、想做某件事，但它到底有没有资格做？

核心原则：

> 可见不代表可用，可调用不代表应调用，能访问不代表拥有。

Governance 不是 skill，也不能只交给后天学习。具体偏好可以学习，但边界机制必须先存在。

### 7.1 职责

Governance Runtime 至少包含：

```text
Identity
Owner
Authorization
Capability
Risk Classification
Trust Policy
Approval Policy
Contract / Delegation
Value Policy
Audit
```

它负责：

- 确认 Agent 代表谁；
- 确认对象是谁的；
- 判断当前 capability 是否覆盖动作；
- 判断风险等级；
- 判断是否需要 owner / human / trusted entity 确认；
- 记录审批、拒绝、越权边界和高风险动作；
- 在多个目标冲突时提供 value / priority 仲裁；
- 对 A2A 协作做身份、授权、契约和结果验证约束。

### 7.2 Risk Gate

任何可能产生以下后果的动作，都必须经过 Governance：

- 删除或覆盖数据；
- 发布、推送、发消息；
- 支付、交易、购买；
- 访问隐私；
- 使用身份凭证；
- 改变系统配置；
- 控制硬件或物理世界；
- 请求另一个 Agent 代为行动；
- 安装或执行外部代码；
- 扩大权限范围；
- 产生长期承诺。

Governance 的输出不是简单 allow / deny，也可以是：

```text
allow
allow_with_audit
require_approval
require_stronger_identity
require_sandbox
require_readonly_mode
deny
```

### 7.3 Approval

Approval 不是“问用户一句话”的同义词。更一般地说，某些 Entity 可以在 workflow 中提供授权、确认、背书、拒绝或契约承诺。

例如：

- owner 本人批准；
- trusted device 确认；
- organization policy 授权；
- another agent 提供承诺；
- payment service 提供交易状态；
- admin role 授权。

UI Session 负责和用户交互；Governance Runtime 负责定义何时必须进入 approval，以及 approval 结果如何审计和影响后续动作。

### 7.4 Value 的位置

Value 不应作为独立顶层 Runtime。

它贯穿三处：

```text
Value State   存在 State & Recall 里，记录 owner 偏好、长期目标和反馈
Value Policy  作用在 Governance 里，做冲突仲裁和优先级约束
Value Signal  由 Self-Improve 收集，用于 skill / memory / attention 排名
```

框架提供 value slot、反馈收集、冲突处理和审计机制；具体 value 内容从 owner 长期行为、明确反馈、确认 / 拒绝、项目目标中学习。

---

## 8. State & Recall Runtime

State & Recall Runtime 回答：

> Agent 如何不是每个 Session 都重新出生？

核心定义：

```text
State 不是世界副本；
State 是 Agent 自己参与世界后留下的内部沉淀。
```

### 8.1 内部状态类型

Notebook、Memory、Skill 不应是三个顶层 Runtime。它们是 State & Recall 里的三种沉淀类型。

```text
State & Recall
  ├─ Notebook：被声明的事实 / 偏好 / 承诺 / 行动项
  ├─ Memory：被推断的印象 / 关系 / 线索
  ├─ Skill：被结晶的可复用能力
  ├─ Self Model：Agent 对自己能力、限制、承诺的模型
  ├─ Value State：偏好、优先级、owner 反馈的沉淀
  └─ Recall：time + sentence + id 的渐进召回机制
```

### 8.2 Notebook

Notebook 保存明确声明、长期事实、偏好、项目状态和行动项。

特点：

- 普通 Session 可以写；
- append-first；
- 保留 actor、source、reason、timestamp；
- 支持 stale / superseded / deleted；
- System Notebook 只允许少量高置信、长期有效、来源明确的事实强注入；
- registry 默认可注入，全文不默认注入；
- read 通过 tag list / title / latest / item id 读取。

Notebook 的详细设计由 [Agent Notebook.md](Agent%20Notebook.md) 承接。

### 8.3 Memory

Memory 保存从 Session History 中推断出的印象、关系、关注点和召回线索。

特点：

- 是推断，不是声明；
- 普通 Session 默认不直接写正式 Memory；
- 由 Self-Improve 基于完整历史产生；
- 每条 memory 带 provenance；
- 允许矛盾印象并存；
- 主要用于 hint，而不是替代外部事实；
- 需要时通过 session history 或 object_id 回读事实。

Memory 的核心不是“更大的上下文窗口”，而是低成本召回与可追溯索引。

### 8.4 Skill

Skill 是被结晶的可复用能力，不是普通记忆。

一个成熟 skill 至少应包含：

```text
trigger / when_to_use
procedure
dependencies / required tools
pitfalls
verification
source_event_ids
risk_level
owner_scope
lifecycle_state
verification_status
usage_history
ranking
```

Skill 有生命周期：

```text
candidate -> verified -> active -> degraded -> blocked / archived
```

Skill 可以由人安装，也可以由 Self-Improve 结晶；但进入 active 前必须验证。Skill 的使用效果要回写 usage history，供后续排名、降权、重测和遗忘。

当前 [Agent Skill.md](Agent%20Skill.md) 描述了现有 skill / tool prompt 注入和目录结构；后续需要把它升级为带生命周期、验证和 provenance 的标准定义。

### 8.5 Self Model

Self Model 保存 Agent 对自己的认识：

- 我有哪些已验证 skill；
- 哪些 skill 已过期或被 block；
- 哪些任务类型我可靠；
- 哪些任务类型我容易失败；
- 我对 owner 有哪些长期承诺；
- 我对外作为 Agent 能承诺什么；
- 哪些失败模式需要低频重测。

Self Model 不能让 Agent 永久切断纠错路径。BlockList、能力自信和自动降权都必须保留 owner 纠正、低频重测和失败回滚入口。

### 8.6 Recall 与 Hint

Recall 的目标是同时避免两种失败：

```text
不知道自己不知道  -> 相关历史存在，但 Agent 没意识到
塞满无用信息      -> 把大量可能相关内容灌进 context
```

OpenDAN 的 Recall 采用两段式：

```text
系统主动浮现 Hint
  -> Agent 判断是否相关
  -> Agent 主动 read 完整事实
```

Hint 统一形态：

```text
time + sentence + id
```

`id` 可以是：

```text
session_id
object_id
notebook_id
notebook_item_id
skill_id
event_id
data_id
```

所有 hint 都不是事实。Hint 只表示“这里可能有相关信息，必要时 read”。

自动召回只允许注入 hint；完整 facts 必须由 Session 在确认相关后主动读取。

### 8.7 半订阅

Recall 不应只在 Session 开头跑一次，也不应每轮机械灌入大量内容。自然触发点是 `update_session_topic`。

```text
topic / tags 更新
  -> 机械检索 Notebook / Memory / Session / Object / Skill
  -> 生成 Hint
  -> 注入低成本线索
  -> Agent 判断并主动 read
```

这是一种“半订阅”：Agent 没有显式订阅所有对象，但 Runtime 根据当前 topic drift 替它维护一组可能相关的线索。

当前工程 TODO 中：

- 固定 hint 架构；
- 打通 hint -> fact 路径；
- 基于 tag 的机械召回；
- 基于 LLM 的半订阅调用；
- 判断当前 session topic 应订阅哪些 Global Object；

都归属 State & Recall Runtime。

---

## 9. Self-Improve Runtime

Self-Improve Runtime 回答：

> Agent 的能力如何成为传记的函数，而不仅是 prompt 和 weights 的函数？

它不是总结器，而是结晶器。

### 9.1 输入与输出

输入：

```text
EventLog
Session History
tool calls
object interactions
user feedback
approval / rejection
Notebook
existing Memory
existing Skill
skill usage history
execution reports
```

输出：

```text
Memory candidate
Skill candidate
Notebook candidate
Value signal
Self-model update
attention change
decay / merge / archive / delete decision
active exploration task
```

### 9.2 处理流水线

Self-Improve 不应直接把所有输出写成正式状态。推荐流水线：

```text
observe
  -> candidate
  -> verify / evaluate
  -> promote
  -> rank
  -> decay / archive / delete
```

不同状态的提升条件不同：

- Memory candidate 需要 provenance、置信度和可回读路径；
- Notebook candidate 需要明确来源，必要时等待 owner / curator 确认；
- Skill candidate 需要验证、依赖声明、风险等级和使用记录；
- Value signal 需要 owner feedback 或长期一致信号；
- Self Model update 需要保留重测与纠错入口。

### 9.3 Attention Graph

Self-Improve 的第一层目标是建立 owner-centric 的 Attention Graph。

它不描述“世界客观上是什么”，而是描述：

> 在 Agent 看来，哪些事情和对象正在与自己、owner 和当前生活发生关系。

基本过程：

```text
发现事件
  -> 发现被卷入的 object
  -> 强化 object attention
  -> 探索 object 之间关系
  -> 形成 memory hint / active exploration task
```

TODO 中“发现事件、发现 Object、探索 Object 之间关系、整理 Attention 热度”归属 Self-Improve Runtime。

### 9.4 Skill / Shortcut Graph

Self-Improve 的第二层目标是管理捷径。

```text
重复出现的任务路径
  -> skill candidate
  -> 依赖工具和对象
  -> 验证方法
  -> 使用场景
  -> 风险等级
  -> active skill
  -> usage history
  -> ranking / decay / block / retest
```

Skill 是 Agent 与世界交互的捷径。安装 Skill 和安装 Agent Tool 是用户扩展 Agent 能力的主要方式。

### 9.5 防止自我回声

Self-Improve 必须区分外部新信号和自己产出的观察。

如果 attention 的新信号把自产 observation 也算进去，Agent 会越来越确信一些只有自己反复念叨过的东西。

因此：

- attention 的新信号只计入外部来源，如用户提及、外部事件、对象变化；
- self-improve 生成的 observation 可以被读取，但不能作为同等级新信号反复强化自己；
- 增量扫描要识别并跳过自产数据；
- 所有 candidate 都必须保留 source_event_id / source_session_id。

### 9.6 触发模型

Self-Improve 不应主要靠外挂 cron。它可以有低频 sweep 兜底，但主触发应来自 Runtime 自己的驱动模型。

触发条件包括：

- Session History 未处理数量到达水位；
- topic drift 明显；
- 某 object attention 升温；
- skill 使用失败或成功达到阈值；
- notebook / memory / skill 发生重要变化；
- owner feedback 到达；
- execution report 表明某类任务长期失败。

---

## 10. 组件之间的数据约束

### 10.1 Provenance

所有内部状态必须可追溯。

```text
Notebook item -> actor / source / reason / timestamp
Memory hint   -> source_session_id / source_event_id / object_id
Skill         -> source_event_ids / verification / usage_history
Value signal  -> feedback event / approval / rejection / behavior evidence
```

没有 provenance 的内部状态不应长期存在。

### 10.2 Fact 与 Hint

所有 hint 都不是事实。

正确：

```text
A previous session may be relevant: session:xxx, "Discussed Agent Runtime and Hint Recall."
```

错误：

```text
The user wants X.
```

Hint 只提供 awareness 和 pointer。事实必须通过 read 得到。

### 10.3 外部事实优先重新观察

内部状态保存的是 Agent 的历史印象，不是外部世界当前状态。

当 Agent 要行动时：

```text
Memory impression
  -> object_id
  -> read(object_id)
  -> verify identity / owner / current state
  -> governance check
  -> action
```

### 10.4 写权限

普通 Session：

- 可以写 Notebook 声明；
- 可以更新 topic / tags；
- 可以消费 Memory / Skill hint；
- 可以记录 task report；
- 不直接写正式 Memory impression；
- 不直接提升 Skill；
- 不直接修改高优先级 System Notebook；
- 不直接改 Value policy。

Self-Check：

- 可以写 Self-Check Review；
- 可以创建 / 更新 / 取消 Schedule-Task；
- 不执行计划任务内容。

Self-Improve：

- 可以生成 Memory / Skill / Notebook / Value candidate；
- 可以在验证后提升 Memory / Skill；
- 可以降权、合并、归档和遗忘；
- 对高风险或高优先级状态提升需要 owner / curator / policy。

Governance：

- 可以拒绝动作；
- 可以要求 approval；
- 可以记录 audit；
- 可以限制 capability。

---

## 11. Subsystem 分工

后续 subsystem 文档应按以下职责归属 review / 更新。

| Subsystem | 所属 Runtime | 详细文档 |
|---|---|---|
| AgentSession | Session Runtime | 本文 §4，现有 `opendan` 实现 |
| Session History | Session Runtime / Event Runtime | [session-history.md](session-history.md) |
| Prompt Compiler / Prompt Env | Session Runtime / State & Recall | 本文 §4.4 |
| Input Route | Session Runtime | 本文 §4.1 |
| Schedule / Wakeup / Task | Event Runtime | 本文 §5 |
| Self-Check | Event Runtime / State & Recall | [Agent Self-Check.md](Agent%20Self-Check.md) |
| Global Object | Object Runtime | 本文 §6，后续需补详细 schema |
| Tool Indexer / Tool Runtime | Object Runtime / Governance | [Agent Skill.md](Agent%20Skill.md) 需升级 |
| Notebook | State & Recall | [Agent Notebook.md](Agent%20Notebook.md) |
| Memory | State & Recall | [Agent Memory v2.md](Agent%20Memory%20v2.md) |
| Hint Recall | State & Recall | [Agent 浮现Hints.md](Agent%20浮现Hints.md) |
| Skill Registry | State & Recall / Self-Improve | [Agent Skill.md](Agent%20Skill.md) 需升级 |
| Self-Improve | Self-Improve Runtime | [Agent Self-Improve.md](Agent%20Self-Improve.md) |
| Governance | Governance Runtime | 后续需新增详细设计 |

---

## 12. 当前工程推进优先级

结合 [todo.md](todo.md)，建议按以下顺序推进。

### P0：修正 Session 和 History 基础

目标：让 Agent 的传记有可靠事实源。

- 用稳定 hook point 接入 History Writer；
- 确保 `round_history/` 不被 context compression 破坏；
- 明确 UI Session / Work Session / Plan / Do 行为边界；
- 优化 Input Route prompt，降低路由错误伤害；
- 强化 `update_session_topic` 的语义和调用时机。

### P1：打通 Notebook + Hint Recall

目标：让 Agent 能低成本想起已经声明过或讨论过的东西。

- 固定 hint 形态：`time + sentence + id`；
- 打通 hint -> read fact；
- Notebook registry / system notebook / read cache 稳定；
- topic tags 触发半订阅 recall；
- 避免已读且未变化内容重复污染 context。

### P2：补齐 Object Runtime 最小接口

目标：让 Agent 有统一的世界对象 I/O。

- 定义 Global Object 最小 object document；
- 实现 `read / list / call / subscribe / resolve` 的基础语义；
- Tool Indexer 能列出当前环境、已有工具、可安装工具；
- 对 tool side effect、permission、risk 提供 manifest；
- 将 object_id 与 Memory / Notebook / Skill hint 接通。

### P3：建立 Governance Runtime 最小可用版本

目标：把风险、owner、授权从 prompt 原则变成 runtime 边界。

- capability / approval / audit 的最小模型；
- 高风险动作 gate；
- approve / reject 结果回写 EventLog；
- A2A 和 tool install 先接入基础 risk gate；
- Value 先保留 slot，不急于实现复杂学习。

### P4：实现 Self-Improve 的 Level 1 / Level 2

目标：从历史里产生可用的 Memory 和 Skill candidate。

- 根据未处理 Session History 扫描 attention；
- 生成 Memory hint candidate；
- 发现重复工作路径，生成 Skill candidate；
- 定义 skill 格式、验证状态、usage history；
- 建立 rank / decay / block / retest 机制；
- 防止自产 observation 反复强化自己。

---

## 13. Home 场景检验

用户说：

> 我不在家的时候，如果下午有访客到来，告诉我，给我一个 report。

按 Runtime 架构，处理过程不是直接依赖预置“访客报告 skill”，而是：

```text
UI Session
  -> 理解用户意图，写入 Notebook / 创建计划或订阅意图

Object Runtime
  -> 从 home indexer 发现 doorbell / camera / motion sensor
  -> read object document
  -> 确认可订阅事件、可读取数据和相关 tool

Governance Runtime
  -> 检查 home device owner
  -> 检查 camera privacy risk
  -> 判断是否需要 owner approval

Event Runtime
  -> 订阅 afternoon visitor event
  -> 条件满足后唤醒 Work Session

Work Session
  -> 读取事件数据
  -> 调用识别 / 摘要 tool
  -> 生成 report
  -> 必要时通知用户

Session History / EventLog
  -> 记录完整过程、tool calls、approval、report

Self-Improve
  -> 如果流程稳定，生成 Home Visitor Report skill candidate
  -> 验证后进入 Skill Registry
```

这个场景说明：Agent 会做事，不是因为框架硬编码了“访客 report 流程”，而是因为 Runtime 支持对象发现、事件订阅、数据读取、工具使用、风险判断、owner 确认、历史记录和经验结晶。

---

## 14. 完整性判据

一个 Runtime 设计是否闭合，看四件事。

### 14.1 内外闭合

```text
外部事实 -> read / verify / observe again
内部状态 -> source_session_id / source_event_id / actor / reason
```

外部事实必须可重新观察；内部状态必须可追溯。

### 14.2 接缝闭合

```text
Memory / Notebook / Skill
  -> object_id
  -> Global Object
  -> object document / DID / owner / current state
```

如果这条链断了，Memory 就会变成孤立文本；如果内部状态复制外部状态，就会变成过期数据库。

### 14.3 驱动闭合

后台过程必须能被 Runtime 自己的驱动模型唤醒。

包括：

- update session topic；
- hint recall；
- notebook update hint；
- self-check；
- self-improve；
- skill 验证和重测；
- 长期任务检查；
- object event 订阅。

定时 sweep 可以兜底，但不应成为唯一主模型。

### 14.4 代谢闭合

Runtime 必须支持：

```text
探索
  -> 记录
  -> 召回
  -> 重新观察
  -> 验证
  -> 结晶
  -> 排名
  -> 遗忘
  -> 重新探索
```

如果只能记录不能遗忘，Agent 会越来越重；如果只能写 skill 不能验证，Agent 会越来越不可信；如果只能召回不能重新观察，Agent 会活在过期印象里。

---

## 15. 最终收敛

`Agent元能力设计.md` 给出的理念是：

> Agent 同时活在时间与物权里。它有传记，会被世界唤醒，对不属于自己的东西负责，并从自己的历史里长出能力。

本文把这个理念落成 Runtime 结构：

```text
Session Runtime：
  处理当前 LLM Loop，承接 UI / Work / Plan / Do / History。

Event Runtime：
  记录事件、管理唤醒、订阅、checkpoint、任务恢复和 Self-Check。

Object Runtime：
  把世界统一成 Global Object 网络，提供 read / list / call / subscribe / resolve。

Governance Runtime：
  处理 identity、owner、authorization、risk、approval、audit 和 value policy。

State & Recall Runtime：
  管理 Notebook、Memory、Skill、Self Model、Value State 和渐进式 Hint Recall。

Self-Improve Runtime：
  从 EventLog 和 Session History 中发现 attention、生成 candidate、验证、提升、排名、降权和遗忘。
```

这就是基于 Agent 元能力的 Agent Runtime 最小闭包。后续实现和 subsystem 文档 review，都应围绕这个闭包检查：每个职责是否有归属，每个状态是否可追溯，每个外部事实是否可重新观察，每个高风险动作是否有 governance gate，每个长期能力是否能被验证、排名和遗忘。
