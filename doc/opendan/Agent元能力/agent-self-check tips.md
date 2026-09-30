# 一句话提醒：从一个简单场景看 Agent 架构的深水区

## 引子

"明天上午 9 点提醒我吃药" —— 这大概是 AI 助理产品里最朴素的需求之一。但当我们认真做这个 demo 时，发现它像一把刻刀，一层层切开了 Agent 框架几乎所有的核心设计问题。本文记录这次讨论的关键结论。

## 起点：环境信息怎么进入 LLM？

最初的问题很具体：收到用户消息后，要不要再插一条 "环境信息" 的 user message 进 agent loop？

结论是不要。连续两条 user message 违反协议语义，会导致 turn 边界混乱、resume hash 不稳定、provider 行为不一致。

那环境信息放哪里？讨论中走过几个备选：

- **System prompt 注入**：会破坏推理顺序的解释一致性。session 中段改 system prompt 会让 LLM 用 "新世界观" 重新解释历史回复，产生沉默漂移。这条作为硬约束写入设计原则：**system prompt 在 session 生命周期内必须稳定**。
- **Tool result 注入**：有 result 没 call，违反协议。
- **中段 system message**：协议上合法，但会破坏 prefix cache。
- **User message 的 multi-content-block**：最终方案。一个 turn 一条 message，多个 content block 区分环境信息与用户输入。UI 上做相应的视觉分层，让用户看见但不抢戏。

## 转折点：物理位置变化的微妙

讨论一个真实场景：用户出差到东京，说 "买明天上午飞东京的票"。

这里 "明天上午" 的时间锚点不是用户当前所在地，而是**起飞地时区**（航班语义自带的硬规则）。如果系统粗暴地用 "当前在东京 → 一切按东京时间" 去解释，会订错票。

这暴露了一个关键原则：**环境信息不是用来覆盖默认值的，而是用来在歧义出现时做消歧的**。意图本身可能自带时空锚点，这种锚点优先级高于环境锚点。

所有时间值在系统内部必须显式携带锚点（`Absolute` / `InTimezone` / `SemanticAnchor` / `UserLocal`），不允许 naive datetime。LLM 不做时间算术，全部交给确定性的 `time_resolve` 工具。

## 真正的解法：UI Session vs Self-Check 的职责分离

提醒功能涉及大量复杂推理：起飞时间反推安检时间反推交通时间反推出门时间，还要考虑天气、机场、用户偏好。把这些塞进 UI Session 是把同步对话的延迟预算用错了地方。

实际架构：

- **UI Session**：轻量、低延迟，负责"忠实记录意图"，不立刻执行复杂决策
- **Self-Check**：后台异步任务，专门 skill 做深度推理
- **Notebook**：两者之间唯一的共享介质，机械保存 3-5 条用户消息原文作为来源

关键点：**Notebook 跨多个 UI Session，是 Agent 的长期工作记忆。原文不做总结，把"解读"推迟到信息最全的 self-check 阶段。**

## 状态模型的范式升级

Self-check 需要还原 "用户提需求那一刻的 session 状态"，这个 "状态" 不再只是 message history，而是复合体：消息序列 + 环境快照 + 工具执行记录 + spawned artifacts。

这是一次真正的范式升级。传统 Agent 框架隐含的假设是 "重建对话只需要重放消息"，这个假设在简单场景下没问题，但在跨 session 的异步深度推理面前彻底崩塌。

## 进一步简化：DID 锚定的全局对象

BuckyOS 的底层有 CYFS 协议，意味着每个有意义的实体（用户、设备、资源）都有 DID 锚定的全局对象，状态变更是事件流，支持按版本/时点查询。

这带来一个史诗级的简化：**session 状态不需要复制环境快照，只需要持有时点引用**。任何 self-check 想知道 "bob 在 2026-05-14 15:00 的位置"，直接 `read /did:bob/location?version=v(t)` 即可。

环境快照从 "显式存储的对象" 变成 "可计算的视图"。

## 双范式：Pull 与 Push

进一步看，DID 路径系统统一了寻址，让两件事变成同一个空间的两种访问模式：

- **Pull**（self-check 用）：按需查询任意时点状态
- **Push**（UI Session 用）：订阅相关路径，变化时主动注入

UI Session 里最高频的 push 内容不是物理环境，而是 **AgentMemory**——通过 `update_session_topic` 召回。这是 session 级 RAG，召回颗粒度是话题而不是单次提问。

## 主动 vs 被动：Agent 范式的根本选择

进一步抽象，传统 toolcall 是被动模式（LLM 在推理压力下决定查什么），订阅是主动模式（旁路系统根据 topic 推断该订阅哪些 state path，变化主动注入）。

LLM 推理用 toolcall 是被动模式，用 topic 是主动模式。这跟 OS 从 polling 演化到 interrupt-driven 是同构的进步。

## 最终的架构图景：LLMContext + Fork 作为基础设施

所有这些都建立在一个核心能力上：LLMContext 是可 fork 的一等公民。

Fork 让旁路从"功能"变成"基础设施"。topic 发现是对最近信息的旁路，上下文压缩是对老信息的旁路——两者完全同构，都是"主 context fork 出去的专门推理"。

整个 BuckyOS Agent 框架因此构成一个完整的 OS 类比：

- **进程抽象** = LLMContext
- **进程派生** = fork
- **文件系统** = `/$did/$state_path` 路径寻址
- **事件订阅** = topic-based subscriptions
- **进程间通信** = 通过共享 state graph
- **持久化** = checkpoint + resume

LLM 不再是 "全知全能的中枢"，而是 "擅长语言推理的组件"。状态感知、变化检测、相关性判断都交给更适合的旁路承载。

## 几条沉淀下来的硬约束

这次讨论中确立的几条原则，应该写入设计文档：

1. System prompt 在 session 生命周期内必须稳定，所有动态状态通过 message 序列承载
2. 不允许连续两条同 role 的 message，turn 是原子单位
3. 所有时间值必须显式携带语义锚点，LLM 不做时间算术
4. Notebook 保存用户原文，不做 LLM 总结，解读推迟到最需要的时刻
5. 环境信息是消歧工具，不是默认覆盖器，意图自带的锚点优先级最高
6. UI Session 负责记录意图，Self-Check 负责深度推理，两者通过 notebook 通信
7. State 信息优先通过 DID 路径系统寻址，而非复制进 session

## 结语

一句话提醒看起来简单，但要做对，需要一个 OS 级的 Agent 运行时来支撑。这次 demo 暴露的不是 reminder 功能的问题，而是**整个 Agent 框架的状态模型问题**。

主流 SaaS-style Agent 框架（LangChain、AutoGen、CrewAI 等）都还停留在"消息序列即状态"的旧范式上。BuckyOS 选择的是 OS engineering 视角下的 Agent 架构——这不是又一个 Agent 框架，而是 Agent 时代的运行时基础设施。

口头提醒只是一个起点。把这个场景做对的所有抽象，都会在其他复杂 Agent 场景里复用。

==




计划任务如何触发 -> Agent.do

另一种模式：通过task_mgr给agent指派任务（比sendmsg好）

AgentRuntime
    读取task_mgr指派给自己但没开始的任务
    根据任务数据找到合适的session
    更新任务状态为运行中
    等待session的状态结束，标识任务状态


session如何确认？
    创建计划任务的时候就建好？（包括workspace)
    每次都是新session => 必然没有worksapce

理想情况在on_init中
    能拿到历史处理记录（reports，如果不是每次独立运行的话）
    能拿到当前task的 content

每次触发通过创建UserMessage 产生驱动力：
    当前时间，按计划执行（比较固定）

