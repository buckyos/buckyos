# OpenDAN Agent 长任务与执行体模型 RFC（完整稿）

- 状态：Draft
- 作者：基于讨论整理
- 目标读者：OpenDAN Runtime / Agent Framework / SDK / App Store / Skill & Behavior 设计者

---

## 0. 摘要

本文定义 OpenDAN 中与“长任务（Long Task）”和“执行体（Execution Unit）”相关的一组统一模型，用于回答以下问题：

1. 当一个 action 进入长时间运行状态时，Agent Loop 应该如何停、等、醒、恢复。
2. 当通知系统（event）不可靠时，如何保证长任务语义不出错。
3. 当 Agent 主动把任务委托出去时，如何区分 tool、临时 session、sub-agent，以及它们各自的边界。
4. 什么情况下“真的需要 sub-agent”，什么情况下只是伪需求。
5. sub-agent 的模板、实例、运行时配置、隔离策略、生命周期如何定义。
6. 从最终生态和使用体验上，如何把整个系统收口到少数几种稳定、可理解、可实现的运行时结构。

本文的核心结论是：

- OpenDAN 不应以“多 Agent”作为默认答案，而应以**问题动机**为出发点决定采用 tool、session 还是 sub-agent。
- 对所有长任务，**event 只是唤醒提示，不是真实状态承诺；恢复后必须重新观测真实状态**。
- 系统最终应把运行时收口为三类执行模型：
  - **Tool**：函数式能力调用
  - **Session**：一次性或短期工作线程
  - **Sub-Agent Instance**：长期、可复用、具备自身状态的执行体
- 对 sub-agent，需要严格区分：
  - **Template**：能力组装逻辑
  - **Instance**：真正运行的实体

---

## 1. 背景与问题

OpenDAN 的 Agent Loop 不是传统的同步函数调用系统，而是一个需要持续处理以下因素的运行时：

- action 可能快速返回，也可能演化为长任务；
- Agent 可能同时面向用户输入、系统任务、审批、构建、安装、外部代理等多类等待源；
- 系统内部存在 session 分离、消息分发、行为循环、技能装载、memory 演化等机制；
- event 系统在很多场景下是弱通知、不可重放、可丢失的；
- Agent 本身既要保证推进效率，又要避免因为错误通知而推进错状态；
- 从产品形态看，主 Agent、Tool、Skill/Behavior、Sub-Agent、外部协作体并不属于同一类对象，不能用同一种方式建模。

如果没有统一模型，系统会出现以下常见问题：

- 长任务卡住当前 step，无法恢复；
- event 丢失后，Agent 永久卡死在 wait 状态；
- 误把通知内容当成真实状态，导致工作流错误推进；
- 为了解决上下文污染而滥造 sub-agent，导致系统复杂度失控；
- 外部委托的结果无法准确路由回原工作现场；
- sub-agent 的模板、实例、隔离边界、长期记忆、生命周期混杂在一起，难以分发和调试。

---

## 2. 设计目标

### 2.1 目标

本文希望建立一套统一原则，使系统能够：

1. 让任何 action 都可自然演化为长任务，而不会破坏 Agent Loop。
2. 允许 Agent 在 wait、poll、event、message、task 等多种唤醒机制间切换。
3. 在 event 不可靠的前提下，仍然保证长任务语义正确。
4. 尽量把复杂性下沉到稳定的运行时原语，减少 prompt 编写者面对的概念负担。
5. 清晰区分：
   - 工具能力
   - 临时执行线程
   - 角色级长期 Agent
6. 为应用商店、模板分发、Skill/Behavior 生态、Sub-Agent 实例化建立统一基础。

### 2.2 非目标

本文不试图：

- 定义具体的 UI 设计；
- 定义完整的 memory 存储格式；
- 解决多机分布式一致性的全部问题；
- 给出所有 SDK API 的最终命名；
- 鼓励在 LLM Loop 中自由生成复杂运行时结构。

---

## 3. 术语

### 3.1 Action

Agent 在一个 step 中调用的能力入口。对 Agent 而言，action 是黑盒：

- 可能同步返回结果；
- 也可能返回 pending 与 task_id；
- Agent 不必知道 action 内部是否用到了 LLM、脚本、子进程或其他系统。

### 3.2 Long Task

不能在当前 step 内稳定完成，需要进入等待、轮询、异步恢复或外部协作的任务。

### 3.3 Wait State

Agent Loop 在 step 结束时为自己设置的等待态，用于表达“接下来在等什么”。

### 3.4 Event

一种唤醒提示。event 可以帮助 Agent 更快恢复，但不是权威状态源。

### 3.5 Task

系统中可被跟踪、可被查询状态的执行对象。通常拥有 task_id。

### 3.6 Session

一个工作现场 / 上下文容器，用于承载某段 loop 的上下文、行为与临时状态。Session 可用于 UI 对话，也可用于纯工作线程。

### 3.7 Main Agent

用户长期使用、逐步养大、带有 global memory 的主智能体。它通常不是频繁替换的对象。

### 3.8 Skill / Behavior

- **Skill**：能力片段、技能提示、规则或技能接口集合。
- **Behavior**：驱动 Agent 进行某种循环或工作模式的行为模板。

### 3.9 Sub-Agent Template

只描述能力组装逻辑，不包含运行时状态的定义体。

### 3.10 Sub-Agent Instance

由模板与运行时配置实例化出来的、真正可运行且有唯一 ID 的 Agent 实体。

---

## 4. 总体运行时视角

OpenDAN 可以被理解为一个 Agent OS 风格的运行时。为了统一理解，本文给出如下映射：

| OpenDAN 概念 | 类 OS 类比 |
|---|---|
| Tool | 系统调用 / 命令 |
| Session | 线程 / 临时工作容器 |
| Sub-Agent Template | 可执行模板 / 镜像 |
| Sub-Agent Instance | 进程 |
| Event | 中断 / 唤醒信号 |
| Task | 可跟踪作业 |

这个类比不是为了把系统机械地等同于 OS，而是强调一件事：

> Agent 不应直接暴露全部内部机制，而应在稳定运行时原语之上做组合。

---

## 5. 长任务模型一：Action 自然长任务化

### 5.1 基本模式

最基础、也必然存在的长任务模式是：

> 在任意一个 step 中，Agent 触发某个 action，该 action 不能立即完成，因而返回 pending，并使当前行为循环进入等待。

此时对 Agent 来说：

- action 仍然只是一个 action；
- 它不需要知道 action 后面究竟是 shell、构建、安装、审批还是其他系统；
- 唯一重要的是：当前 step 未完成，且有一个明确的等待对象。

### 5.2 标准返回

推荐 action 在进入长任务时返回如下信息：

```json
{
  "status": "pending",
  "task_id": "task_xxx",
  "reason": "runtime_task",
  "hint": "this task may take long time",
  "suggested_next": {
    "type": "poll",
    "delay": 15
  }
}
```

关键点：

- `status` 表示当前不是最终结果；
- `task_id` 用于后续状态查询；
- `reason` 用于帮助 Agent 理解等待类型；
- `suggested_next` 只是建议，不是强制。

### 5.3 Wait Reason 分类

建议把最常见的等待态收口为以下几类：

#### 5.3.1 `wait_for_user_input`

表示需要用户补充信息。

特点：

- 无法自动推进；
- 必须等 message；
- 常用于信息不充分、交互补全。

#### 5.3.2 `wait_for_task`

表示等待一个明确 task 的结果。

典型场景：

- 审批
- 权限确认
- 用户选择
- 外部判定

特点：

- 有明确 task_id；
- 可以无限等待；
- 是非常适合权限控制的模型。

#### 5.3.3 `wait_for_runtime_task`

表示等待一个系统内部长时间运行任务。

典型场景：

- build
- install
- deploy
- 长 bash 任务

特点：

- 可轮询；
- 通常应结合 timeout；
- 需要 crash-safe 恢复。

#### 5.3.4 `wait_for_event(timer)`

表示 Agent 主动决定未来某个时间再恢复。

典型场景：

- 暂时无其他工作可做；
- 需要稍后检查某个任务；
- 避免 busy loop。

### 5.4 每个 step 结束时的核心决策

Agent Loop 每个 step 结束时，都必须做一个统一决策：

> 现在是否进入下一轮 LLM 调用，还是把自己挂起到某个 wait state。

可以抽象为：

```ts
function decide_next_behavior(context): NextBehavior
```

其中可能的结果包括：

- 继续执行下一轮 loop；
- wait_for_user_input；
- wait_for_task(task_id)；
- wait_for_event(timer)；
- 结束当前任务。

### 5.5 系统与 LLM 的职责边界

OpenDAN 的一个重要设计取向是：

> “等什么、怎么等”尽量交给 LLM 决策；系统负责提供稳定的执行原语与触发机制。

系统负责：

- 提供 task 状态查询；
- 提供 timer；
- 提供 message 唤醒；
- 提供 event 通知；
- 提供 wait 原语。

LLM 负责：

- 判断是否继续；
- 选择等待类型；
- 决定轮询间隔；
- 决定是否去做其他工作。

这种设计更灵活，但也要求运行时原语足够稳。

---

## 6. 长任务模型二：弱事件系统下的可靠等待

### 6.1 问题背景

在 OpenDAN 中，很多 event 只是弱通知：

- 可以丢失；
- 不可重放；
- 在系统重启或 Agent 进程崩溃期间，可能无人接收；
- 即使投递到了，也未必包含完整数据。

典型问题：

1. Agent 发起 build task；
2. Agent 进入 `wait_for_event(task_complete)`；
3. Agent 进程中途崩溃；
4. build 在 Agent 离线期间完成；
5. complete event 已经发出但无人接收；
6. Agent 恢复后继续等，结果永久卡死。

### 6.2 核心原则：Event 是加速器，不是事实

本文定义一条强原则：

> **Event 只是唤醒提示，不是真实状态源。**

因此：

- event 不能作为唯一触发条件；
- event 到了，不等于任务一定完成；
- event 丢了，也不等于任务没有完成；
- 任何关键推进都必须回到 task / state 查询。

### 6.3 State 才是真相

权威状态应保存在任务系统中，可由 `check_task(task_id)` 等能力显式查询。

因此可靠等待模型必须是：

- event：加速唤醒
- timeout：兜底恢复
- poll / check：最终判定

### 6.4 推荐实现：Event + Timeout + Poll Fallback

建议所有 wait_for_task 类型的等待都具备 timeout：

```ts
WaitForTask = {
  task_id: string,
  timeout: 30s,
  strategy: "event + fallback_poll"
}
```

执行路径：

1. 进入 wait；
2. 若 event 正常到达，则恢复；
3. 若 event 未到达，timeout 到期也恢复；
4. 恢复后统一 `check_task(task_id)`；
5. 若已完成，则推进；
6. 若仍未完成，则重新设置 wait。

### 6.5 自愈能力

这意味着 Agent 必须具备一种自愈能力：

> 即使 event 丢失，也能通过 timeout + state re-check 自己把自己从错误等待里救出来。

因此，OpenDAN 的 wait 不是“盲等事件”，而是“等待一次恢复执行的机会，并在恢复后重新对齐现实状态”。

---

## 7. 长任务模型三：通知不是状态，恢复后必须重新观测

上一节的原则可以进一步收敛成更硬的一条规则：

> **凡是与长任务相关的恢复，Agent 的第一反应都应该是重新观测，而不是直接相信通知内容。**

### 7.1 为什么必须重新观测

因为通知系统可能出现：

- 丢失
- 提前
- 重复
- 顺序错乱
- 数据不全
- 误触发

因此即使收到一个 `task_complete` 事件，也不能直接认为任务已经完成，而应：

1. 恢复 loop；
2. 调用状态查询 action；
3. 根据真实状态决定继续等待还是推进。

### 7.2 误唤醒是可接受的，误推进是不可接受的

这是一条非常关键的工程原则：

- 若 event 误报，最多浪费一次推理与一次检查；
- 若不检查就推进，则可能造成状态错乱、工作流出错、结果污染。

因此系统应宁可允许：

- 多一次恢复；
- 多一次检查；
- 多一次 timeout 触发；

也不能允许：

- 未确认完成就进入后续逻辑。

### 7.3 对 Action 设计的反向要求

任何会进入等待的 action，最好都配套一个状态读取 action。

例如：

- `start_build()` + `check_build(task_id)`
- `submit_approval_task()` + `check_approval(task_id)`
- `start_install()` + `check_install(task_id)`

否则恢复时将缺乏稳定观测入口。

---

## 8. 长任务模型四：Agent 主动委托的 Delegate Task

### 8.1 定义

与 action 被动变长不同，另一类长任务是：

> Agent 主动识别某件工作不适合在当前 step / 当前执行上下文内完成，因此明确把它委托出去。

这类任务称为 **Delegate Task**。

### 8.2 Delegate 的本质

Delegate 不是 fire-and-forget，而是：

- 主动发起；
- 有状态感知；
- 有归属关系；
- 有后续等待与结果回流。

也就是说：

> 委托出去的，不只是一次消息，而是一段长期协作语义。

### 8.3 外部代理与内部代理

#### 8.3.1 External Delegate

任务被通过 message 委托给外部对象：

- human
- 外部 agent
- 其他系统中的执行者

其核心特点是：

- 完全 message-based；
- 对主 Agent 本身没有内部 side effect；
- 从主 Agent 看，本质上就是标准网络通信：`message out -> message in`。

这是一种边界最清晰、隔离性最强的委托方式。

#### 8.3.2 Internal Delegate

任务被委托给系统内部其他执行体：

- 子 Agent
- worker
- 其他工作单元

但本文强调：

> 很多所谓 internal delegate 的需求，其实并不是真正需要另一个 Agent，而只是需要新的 session 或新的 context。

---

## 9. 外部 Delegate 的复杂点：消息回流与 Session 路由

### 9.1 问题

在有 session 机制的系统中：

- 主 Agent 可以在某个 work session 中向外发消息；
- 但外部回复回来时，默认往往不会直接落回原 session；
- 它通常先进入统一的 dispatch / UI session。

此时系统面临的关键问题是：

> 这条回复到底属于哪个工作现场？

### 9.2 为什么这是硬问题

如果路由错了，会产生：

- 原 session 一直等不到结果；
- 回复在错误上下文里被处理；
- 用户界面与后台工作现场脱钩；
- debug 非常困难。

### 9.3 两种解决思路

#### 思路 A：依赖 Message 引用链

要求被代理对象回复时引用原消息。然后 dispatch 通过 message metadata 反推出：

- 这是哪个 delegate task 的回复；
- 这个 task 属于哪个 session；
- 因而应路由回哪个工作现场。

优点：

- 简洁；
- 协议自然；
- 不必引入太多全局状态。

缺点：

- 依赖对端配合；
- 外部回复不规范时不稳；
- 很难覆盖所有情况。

#### 思路 B：把 Delegate Task 提升到 Agent 级 Memory / Registry

让系统保存：

- 哪些委托任务还活着；
- 每个任务归属哪个 session；
- assignee 是谁；
- 相关 message id 是什么；
- 当前在等什么。

这样 dispatch session 在处理外部消息时，可以从 Agent 全局态势中推断路由。

优点：

- 更稳；
- 跨 session 可见；
- 更适合长期任务。

缺点：

- 全局状态复杂度更高；
- 需要处理 session-local 语义与 agent-global 索引的一致性。

### 9.4 推荐模型

本文建议折中：

> **任务归属仍然是 session 的，但委托任务的索引与路由 hint 提升到 agent 级可见。**

也就是说：

- session 负责详细工作上下文；
- agent registry 负责 task_id -> owner_session_id 的索引。

---

## 10. 为什么不应默认做多 Agent：内部 Delegate 的伪需求

实践中很多系统喜欢引入内部多 Agent，但大量场景并没有真的解决问题。

本文认为，设计 internal delegate 之前，必须先问：

> 我真正想解决的是什么？

最常见的动机有两个。

### 10.1 动机一：上下文隔离

很多人说“我需要 sub-agent”，实际上只是因为：

> 不想污染当前 session 的上下文。

例如 UI session 与 work session 分离之后，大量原本要靠 sub-agent 解决的问题已经不复存在。

### 10.2 动机二：LLM Context / 执行环境隔离

另外一些需求不是为了另一个 Agent，而是为了另一个执行空间：

- 不同 model；
- 不同 tool set；
- 不同隐私策略；
- 不同 cost profile；
- 不同默认 prompt。

这更像传统系统中的“切换线程 / 切换上下文”，而不是创建一个新的角色主体。

### 10.3 核心结论

> **当真实需求只是隔离上下文或切换执行环境时，应优先使用 session 或 tool，而不是创建新的 sub-agent。**

---

## 11. Tool-Encapsulated LLM Task：把内部复杂性下沉为 Tool

一个非常重要的设计方向是：

> 许多内部复杂逻辑，即使内部跑了一段 LLM Loop，也应优先被封装成 tool，而不是暴露成显式的 sub-agent。

例如代码扫描：

传统多 Agent 方式：

```text
主 Agent -> sub-agent -> 扫描目录 -> 返回结果
```

更合适的方式：

```text
主 Agent -> 调用 lm_explore_files -> 返回结果
```

对主 Agent 而言，它只是在调用一个工具。至于这个工具内部是不是跑了一段 LLM loop，不应该成为调用侧必须理解的概念。

这带来的好处是：

- 简化 prompt 设计；
- 降低运行时复杂度；
- 使能力可安装、可分发、可优化；
- 把复杂性留在 SDK / Runtime / Tool 实现层。

因此本文鼓励：

> **尽可能把“内部多 Agent”退化成“有智能的 Tool”。**

---

## 12. 什么情况下真的需要 Sub-Agent

当需求不再是“一个窄意图工具”，而是“一个宽意图角色”，就开始接近真正的 sub-agent 需求。

### 12.1 工具是窄意图

工具的特点是：

- 输入输出明确；
- 解决问题范围窄；
- 调用意图清晰；
- 通常只完成一小段明确工作。

### 12.2 Sub-Agent 是宽意图

真正的 sub-agent 更像：

- 一个岗位；
- 一个角色；
- 一个长期独立的能力域；
- 一整类事情的承接者。

它的存在意义不在于“帮我做一步”，而在于：

> “这类事情天然属于它的能力域，应由它自行判断、规划和执行。”

### 12.3 典型例子：Windows Operator

主 Agent 主要运行在 bash / Linux 环境，但某类工作必须依赖 Windows：

- Windows-only 软件；
- GUI 工具；
- AutoCAD 等环境。

此时系统中存在一个专门的 Windows Sub-Agent：

- 它的系统空间围绕管理 Windows 虚拟机展开；
- 它的意图面很宽；
- 它不是单一工具，而是一个角色。

当主 Agent 判断某件事情本质上属于 Windows 能力域时，就应把任务委托给这个角色。

### 12.4 真需求判断标准

一个需求更接近真实 sub-agent 的条件包括：

1. 能力域是宽的，而不是单点工具能力；
2. 需要长期存在的独立执行环境；
3. 需要围绕该能力域形成自己的默认 prompt / skill / behavior；
4. 主 Agent 不希望把整类逻辑污染到自己的常驻提示空间；
5. 该执行体未来可能反复被调用，而不是一次性使用。

---

## 13. 共享型与隔离型 Sub-Agent

真正需要 sub-agent 后，马上会遇到下一个问题：

> 它与创建它的 Agent 之间，到底共享什么、不共享什么？

本文建议把子 Agent 分为两类。

### 13.1 共享型 Sub-Agent（Forked / Shared-State）

特点：

- session 隔离；
- 但可共享部分 agent 级 memory / state；
- 更像主 Agent 的一个分叉执行体；
- 默认行为、模型、skills 可以变化；
- 但它仍然服务于主 Agent 的连续工作流。

适合场景：

- 需要上下文隔离，但又不希望完全丢掉主 Agent 的状态；
- 在特殊环境中承接同一主任务的一段工作；
- 执行过程中需要读写部分共享状态。

例如：

- 从 Linux 侧把 CAD 文件交给 Windows 环境处理，再挂回主工作目录。

### 13.2 隔离型 Sub-Agent（Spawned / Isolated）

特点：

- memory 隔离；
- 生命周期可独立；
- 行为、工具、模型、长期状态都独立；
- 本质上是一个真正独立的 Agent，只是当前被主 Agent 委托。

适合场景：

- 角色级分工；
- 长期独立运行；
- 不希望共享 side effect；
- 可视为系统中的另一个长期主体。

### 13.3 命名建议

由于两者语义差异很大，不建议都叫 `create_subagent`。

更合理的是提供两个明显不同的原语，例如：

- `fork_agent_context(...)`：共享型
- `spawn_agent_instance(...)`：隔离型

最终命名可调整，但重要的是让 LLM 能区分语义。

### 13.4 防止无限递归

为避免实例爆炸和无限 delegate，系统可加一条硬约束：

> 已被标记为子 Agent 的执行体不得继续创建子 Agent。

这不是概念上的绝对真理，而是工程上的安全边界。

---

## 14. Tool、Session、Sub-Agent：运行时三分法

在梳理完所有模式后，本文建议最终把系统运行时收口成三类执行体。

### 14.1 Tool

特征：

- 函数式调用；
- 短生命周期；
- 无或弱长期状态；
- 可同步返回，也可返回 pending。

对 Agent 来说，tool 是最轻的能力调用方式。

### 14.2 Session

特征：

- 一个工作现场；
- 通常承载一段临时 loop；
- 可是 UI-visible，也可以是 headless；
- 一次性完成某任务后可直接归档。

本文建议引入“long-once session”作为明确模型：

> 只为一个子任务创建、执行完即归档的临时工作线程。

### 14.3 Sub-Agent Instance

特征：

- 长期存在；
- 有独立唯一 ID；
- 有自己的长期 memory；
- 可反复被 delegate；
- 可被应用商店和实例化系统管理。

### 14.4 三者统一理解

| 类型 | 类比 | 生命周期 | 状态 |
|---|---|---|---|
| Tool | 函数 | 短 | 无 / 弱 |
| Session | 线程 | 短 / 一次性 | 临时 |
| Sub-Agent Instance | 进程 | 长 | 强 |

这三分法是本文最重要的收口之一。

---

## 15. 主 Agent、Tool、Skill/Behavior 的分发与扩展

### 15.1 Tool 的产品形态

Tool 是独立实现、可安装、可分发的能力模块：

- 可由 SDK 开发；
- 可调用 OpenDAN Runtime / BuckyOS / AI Compute Center；
- 内部可以包含固定流程或小型 loop；
- 对外仍然暴露为标准 action。

例如 `llm_file_explorer` 这类工具，本质上应是可复用、可分发的系统能力，而不是某个主 Agent 私有的内嵌逻辑。

### 15.2 主 Agent 的产品形态

主 Agent 与 Tool 完全不同。主 Agent 是：

- 逐步养大的；
- 陪伴型的；
- 长期持有 global memory 的；
- 用户很少替换的主体。

因此，主 Agent 不适合通过“频繁安装新 Agent 替换旧 Agent”的方式扩展。

### 15.3 主 Agent 的扩展方式

主 Agent 更适合通过增量扩展成长：

- 安装新的 skills；
- 安装新的 behaviors；
- 接入新的 tools；
- 调整少量全局设定。

也就是说：

> 主 Agent 的核心扩展方式是增量增长，而不是整体替换。

---

## 16. Sub-Agent 的模板与实例

### 16.1 Template 与 Instance 必须解耦

这是本文另一条核心原则：

> **Sub-Agent 的组装逻辑与运行实体必须分离。**

#### Template

只描述：

- skills
- behaviors
- context 参数
- 默认模型 / 工具 / 运行配置

它不包含：

- 当前 memory
- 当前 session
- 当前任务状态
- 唯一运行 ID

#### Instance

由 template 和 runtime config 实例化出来，具备：

- agent_id
- memory
- 生命周期
- 隔离策略
- UI / session 策略
- 实际运行状态

### 16.2 为什么必须分离

因为一个 template 可以产生多个 instance：

- 能力定义相同；
- 运行时逻辑不同；
- 隔离模式不同；
- 生命周期不同；
- UI 策略不同；
- memory 完全不同。

因此：

> 功能性由模板决定，运行性由实例决定。

系统中真正被引用和调度的，永远都应该是 instance id，而不是 template。

---

## 17. Sub-Agent 的两种来源

### 17.1 预制型 Sub-Agent

应用商店可以直接提供“开箱即用”的 sub-agent。

其本质上是：

- 一个预制 template；
- 配上一套默认实例化策略；
- 用户无需理解组装细节；
- 一创建就是一个标准、独立、可直接 delegate 的 instance。

典型例子：

- Windows Operator
- 某角色型岗位 Agent
- 某独立系统侧专职 Agent

### 17.2 组装型 Sub-Agent

应用商店也可以提供 skill / behavior / context 模板，供用户或系统自行拼装。

这类对象更像“材料”，而不是完整 Agent：

- 先组装成 template；
- 再根据需求实例化；
- 可用于 fork 主 Agent，或生成新 instance。

---

## 18. 为什么不应把所有模板永久装进主 Agent

下载回来的 skill / behavior 不应默认永久进入主 Agent 的常驻装载空间。

原因包括：

- 主 Agent 提示空间会越来越臃肿；
- 默认行为会变得不稳定；
- 很多能力只在特定任务需要；
- 长期污染很难管理。

因此本文建议：

> **Skill / Behavior 更应被看作拼装材料，而不是默认永久入侵主 Agent 的常驻能力。**

主 Agent 只保留少数长期常驻的内容，其余能力应按场景装入 session 或 sub-agent template。

---

## 19. 组装式 Sub-Agent：从主 Agent Fork

一种非常重要的构造方式是：

> 从主 Agent 派生出一个任务期 Sub-Agent。

过程大致如下：

1. 选择一组 skills / behaviors / context config；
2. 基于这些内容形成 template；
3. 从主 Agent 的身份和必要 memory 出发生成一个 forked instance；
4. 限定其默认能力面，使其更专注于当前角色；
5. 在完成任务后保留或回收该 instance。

这种模式的意义在于：

- 可以继承主 Agent 的基础身份感；
- 但不会被主 Agent 全部提示空间污染；
- 更容易让子执行体稳定使用被指定的那些 skill；
- 适合围绕特定任务或角色进行专门化。

---

## 20. Template 构造不应在普通 LLM Loop 中频繁发生

本文建议：

> **Template 的拼装应主要发生在人工配置阶段，或主 Agent 的专门 self-improve 阶段，而不应在普通工作 loop 中临时自由生成。**

原因：

- 成本高；
- 结果不稳定；
- 难以复现；
- 不利于生态管理；
- 会让 LLM 在做任务时同时承担系统设计工作。

因此在常规运行中，Agent 更适合：

- 使用已有 tool；
- 开已有样式的 long-once session；
- delegate 给已存在的 sub-agent instance。

而不是现场设计一个新 Agent。

---

## 21. Session 作为轻量级子任务执行体

除了 sub-agent instance，系统还应提供一种更轻的模型：

> **专门为某个子任务创建一个新的 session。**

这种模式：

- 不一定需要长期 memory；
- 不一定对 UI 可见；
- 可只装载少量 behavior / skill；
- 执行完即可归档；
- 非常适合“fork 自己做一个子任务”的场景。

这正是前文的 `long-once session`。

### 21.1 优点

- 复杂度低；
- 无需引入新的 Agent identity；
- 一次性完成后即清理；
- 对状态污染有限。

### 21.2 风险

如果一个 session 不对 UI / dispatch 可见，却在运行时尝试发消息或需要交互，容易出现非常难调的 bug。

因此 session 至少应区分：

- **UI-visible session**：允许与消息系统交互；
- **Headless session**：纯执行，不允许承担交互职责。

选择错误会造成严重调试困难。

---

## 22. Runtime Config：实例化时一次性定好

无论是 sub-agent instance 还是特殊 session，凡是涉及长期执行语义的对象，在创建时就应该把关键运行时参数确定下来。

建议包含以下维度：

1. **隔离模式**
   - 完全隔离
   - 继承部分 memory
   - 共享某些全局状态

2. **UI 策略**
   - 是否 UI-visible
   - 是否允许消息输入输出
   - 是否仅 headless

3. **生命周期策略**
   - 一次性
   - 长期保留
   - 完成后自动删除
   - 失败后保留用于调试

4. **Behavior / Skill 装载策略**
   - 固定装载
   - 最小装载
   - 从模板继承
   - 从主 Agent 部分继承

5. **模型与工具策略**
   - 默认 model
   - 可用 tool set
   - 隐私策略 / 成本策略

这些参数应当是 runtime config 的一部分，而不应在任务运行中不断漂移。

---

## 23. Agent 使用视角的最终收口

系统内部虽然复杂，但从 Agent 的可见使用面上，本文建议尽量收口为三种选择：

### 23.1 调一个 Tool

用于：

- 明确窄任务；
- 一次能力调用；
- 或封装好的智能能力。

### 23.2 开一个临时 Session

用于：

- 做一个一次性子任务；
- 与主流程上下文隔离；
- 不需要长期角色身份。

### 23.3 Delegate 给一个现有 Sub-Agent Instance

用于：

- 角色级能力域；
- 宽意图任务；
- 长期存在且可复用的执行体。

也就是说，最终运行态不应暴露太多概念给 Agent。对于 prompt 编写者和调用者来说，核心可理解面就是：

- tool
- session
- sub-agent instance

这三种足够了。

---

## 24. 实例列表必须受控，不能无限膨胀

由于系统运行时最终操作的是 sub-agent instance，而不是 template，因此实例数控制很关键。

### 24.1 原则

- Agent 不应在普通 loop 中随时自由创建大量新 instance；
- 通常应优先使用已有 instance；
- 临时型 instance 用完应及时删除或归档；
- `list_available_subagents` 的结果不应过多，否则选择成本和误用风险都会快速上升。

### 24.2 生命周期建议

实例可分两类：

#### 长期型 Instance

例如 Windows Operator：

- 长期存在；
- 持续积累 memory；
- 可反复承担同类任务。

#### 临时型 Instance

- 为某个任务或阶段创建；
- 完成后即删除；
- 不建议无限保留。

---

## 25. 参考对象模型

以下给出一个便于实现的参考结构。

### 25.1 Wait State

```ts
type WaitState =
  | { type: "user_input" }
  | { type: "task"; task_id: string; timeout?: number }
  | { type: "runtime_task"; task_id: string; timeout?: number }
  | { type: "event"; event: "timer"; wake_at: number }
```

### 25.2 Delegate Task

```ts
type DelegateTask = {
  task_id: string
  kind: "internal" | "external"
  owner_session_id: string
  created_by_agent: string
  assignee: {
    type: "human" | "agent" | "worker" | "external_system"
    id: string
  }
  status:
    | "created"
    | "delegated"
    | "waiting_reply"
    | "in_progress"
    | "resolved"
    | "failed"
    | "canceled"
  related_message_ids?: string[]
  summary?: string
}
```

### 25.3 Sub-Agent Template

```ts
type SubAgentTemplate = {
  template_id: string
  name: string
  skills: string[]
  behaviors: string[]
  context_config: {
    model?: string
    toolset?: string[]
    privacy_policy?: string
    cost_profile?: string
  }
  default_runtime_policy?: {
    isolation: "shared" | "isolated"
    ui_mode: "visible" | "headless"
    lifecycle: "persistent" | "ephemeral"
  }
}
```

### 25.4 Sub-Agent Instance

```ts
type SubAgentInstance = {
  agent_id: string
  template_id: string
  runtime_policy: {
    isolation: "shared" | "isolated"
    ui_mode: "visible" | "headless"
    lifecycle: "persistent" | "ephemeral"
  }
  inherited_from?: string
  memory_ref?: string
  status: "idle" | "busy" | "archived"
}
```

这些结构不是最终 API，只是帮助统一概念。

---

## 26. 设计原则汇总

1. **Event 只是加速器，不是真相。**
2. **恢复后必须重新观测真实状态。**
3. **误唤醒可以接受，误推进不可接受。**
4. **上下文隔离不等于需要 sub-agent。**
5. **尽量把内部复杂性下沉为 Tool。**
6. **真正的 sub-agent 面向宽意图、角色级能力域。**
7. **Template 与 Instance 必须解耦。**
8. **Session 是线程，Sub-Agent Instance 是进程。**
9. **运行时创建必须受控，不能让 instance 数量爆炸。**
10. **Agent 的可见使用面应尽量收口为 Tool / Session / Sub-Agent 三种。**

---

## 27. 实现建议

### 27.1 Action SDK

SDK 应支持：

- 返回 pending；
- 返回 task_id；
- 注册状态查询接口；
- 标准化 timeout / poll 建议；
- 允许 tool 内部调用系统能力与 AI 能力。

### 27.2 Runtime

Runtime 应负责：

- wait state 存储与恢复；
- timer 触发；
- task 状态查询；
- event best-effort 通知；
- crash 恢复后重新进入 reconciliation。

### 27.3 Session Framework

Session 系统应支持：

- UI-visible / headless 区分；
- long-once session；
- 行为与 skill 的最小装载；
- 与 dispatch 的路由协作。

### 27.4 Sub-Agent Framework

Sub-Agent 框架应支持：

- template registry；
- instance lifecycle 管理；
- runtime policy 固化；
- fork / spawn 两类实例化方式；
- agent-level registry 与 session-level 归属映射。

### 27.5 App Store / 生态

应用商店可分发的对象可分层：

1. Tool
2. Skill / Behavior 模板
3. 预制型 Sub-Agent Template

而真正运行时操作的对象，仍然主要是 instance。

---

## 28. 未决问题

以下问题仍需后续细化：

1. Session 与 Agent memory 的共享粒度如何表达。
2. Headless session 是否允许受限消息输出。
3. Runtime Config 中哪些参数可被模板默认，哪些必须实例化时显式指定。
4. Template 的版本升级如何影响已有 instance。
5. Self-improve 阶段拼出的 template 是否允许进入应用商店或用户私有仓库。
6. Dispatch session 的路由错误如何回滚或补救。
7. 实例删除后的 memory 清理与审计如何处理。
8. 子 Agent 禁止再创子 Agent 是永久规则，还是早期保护规则。

---

## 29. 结论

本文提出了一套以长任务与执行体为中心的统一模型。其最终收口非常明确：

- 对于长任务：
  - action 可自然长任务化；
  - event 不可靠，但 wait 可通过 timeout + re-check 自愈；
  - 所有推进都应基于真实状态观测，而不是通知本身。

- 对于执行体：
  - tool 解决窄意图能力调用；
  - session 解决轻量级、一次性子任务；
  - sub-agent instance 解决宽意图、角色级、长期能力域。

- 对于生态与构造：
  - 主 Agent 是陪伴型、逐步养大的主体；
  - skill / behavior 是拼装材料；
  - template 定义能力，instance 承载运行；
  - 应用商店分发模板与工具，运行时操作实例。

如果把 OpenDAN 视为一个 Agent OS，那么本文给出的就是它在“长任务、线程、进程、模板、实例”这一层的最小稳定运行时模型。

