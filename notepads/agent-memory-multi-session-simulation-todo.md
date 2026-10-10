# Agent Memory 多 Session 模拟与组件实现 TODO

日期：2026-10-10

状态：待实施，已对齐 Memory 需求 v1.6（2026-10-10，含本 TODO review 后补入附录 A 的 A.3.7、A.9 等契约）。本文的模拟记录、输出及逻辑调用不表示现有能力；组件模型与提交契约以需求附录 A 为准。

目标：在 Agent Memory 作为 tool 接入 Agent Session 之前，用多个模拟 Session 直接调用 Memory 组件，验证“添加感知 → set_topic 召回并半订阅 → 接收相关变化 → Memory 整理 Goal 将感知整理为认知”的完整流程。例子应展示每一步的调用、返回、持久状态与通知，让设计可以逐项检查。

依据：[Agent Memory 认知管理需求 v1.6](<../doc/opendan/Agent Memory 认知管理需求.md>)、[OpenDAN Agent Session 架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>)。需求 §10.5 决定实施阶段，附录 A 决定组件契约（范围与可见性见 A.3.7，topic 与观察进度见 A.9），附录 B 的 TD-01—TD-26 用于跟踪实现差距；本文负责组件阶段的实施顺序、模拟 history 和验收映射，不另建一套接口规格。实现中发现契约要改时，先改附录 A，再改本文。旧元能力文档仅作参考。

写入和整理 fixture 遵循 §4.4—§4.6、§5.6—§5.9。本次同时召回认知和未整理感知，两者保持不同身份；相关感知无需等整理完成。每次推理前默认有半订阅观察机会，但 Memory 是尽力而为的线索，运行中修改必须走 Task 修订。正式 Session 接入及模型判断质量分别留到 S、Q 阶段。

团队 Review 建议先看 §3.4 的概念对照，再读 §3.5～§3.9 的基本 history、§3.11 的 Task 修订边界，以及 §3.12—§3.14 的修订关注（含整理前纠正不经已读关注的反向断言）、暂缓到期和歧义对象案例。用 §3.10 和附录 A.9 判断接口增量，用 §5—§6 检查开发项和验收覆盖。批次、清理和确认可以由组件内部承担，不要求每个逻辑动作都新增公共方法。

## 1. 本阶段边界

- 实现 Memory 的数据、写入、查询、topic 关注、变化记录、整理提交和清理能力，以及直接调用这些能力的示例与测试。
- Session 用轻量 `SimSession` 表示，只携带身份与授权主体集合、Session ID、A.9 的 `ObservationState`（topic、快照、pending、read_set）及必要的模拟运行状态；`ObservationState` 持久化到模拟 Session 目录，用于重启验证。Work 变体没有 Message input，由宿主模拟推理前观察机会。不启动真实 Runner、LLM、工具或消息总线。
- Memory 整理 Goal 用预设的整理决策模拟，读写真实 Memory 组件；下文保留 `SI` 作为模拟客户端别名，表示整理业务，不代表第四种 Session 类型。模拟的是推理结果，存储、匹配、提交、通知和恢复逻辑必须真实执行，不实现真实 Goal 调度。
- 新设计取消独立 Agent Notebook 模块，统一由 Memory 承载感知和认知。优先复用现有 Memory Graph、感知存储和锁机制；旧 Notebook 中有用的版本、事务和读取进度代码可按需迁入 Memory，不保留第二套笔记语义或存储入口。`agent_tool` crate 内相关库代码属于组件范围；工具注册、CLI 工具封装、tool schema、提示词与 behavior 不在本阶段修改。
- 不修改 Session 的执行循环、默认模板、输入协议或 Loader 调度；本阶段的 `set_topic` 是 Memory 使用端的逻辑调用，不提前定义正式 Session tool。
- 普通模拟客户端只能追加感知、查询、设置 topic 和读取变化；认知只由持有整理 lease 的 SI 提交变更（M-32）。模拟宿主确定性建立基础范围，模型追加的 topic tag 与关注单独管理；基础范围不随 tag 淘汰（M-21、§6.6）。
- 不新增独立服务、向量库或通用框架。现有组件的依赖方向应保持清楚，尤其不能让 `agent_tool` 反向依赖 `libopendan`。

| 阶段 | 本 TODO 的交付与边界 |
|---|---|
| C 组件，本阶段 | 真实感知存储、Graph 提交、查询、topic 与观察进度、清理和恢复；预设判断、时钟与来源事件；按附录 A 扩展现有库，不运行模型 |
| S Session 接入，后续 | 正式 CLI/tool、PATH 与身份授权绑定、Runner 每次推理前装配、Task 修订、来源锚点采集、整理 Goal 调度、DID Object、旧调用方退役 |
| Q 模型质量，后续 | 同一组 fixture 接真实模型，评估是否值得写、明确纠正与本次例外、指代与时间解读、结论范围和综合质量 |

M-30—M-34 从本阶段约束设计；C 阶段的拒写与关闭召回测试只覆盖组件和模拟宿主，B-40—B-43 的真实使用方验收仍在 S 阶段完成。Task 修订的必达机制不由 Memory 测试代为证明。

## 2. 当前基础与复用入口

| 入口 | 已有基础 | 本阶段需要补齐 |
|---|---|---|
| [agent_memory.rs](../src/frame/agent_tool/src/agent_memory.rs) | Graph 对象、别名、证据观察、认知条目、批量 commit、提交日志、写锁、派生索引；`load` 已有对象/对象对加分 | 按附录 A 扩展字段、revision、状态校验、整理 envelope 和查询；补关系展开、中文检索与完整得分传递 |
| [agent_notebook.rs](../src/frame/agent_tool/src/agent_notebook.rs) | Note、版本、冲突检测、跨 Session 更新提示、读取进度 | 作为待退役实现评估可迁入 Memory 的机制；新例子不依赖独立 Notebook，旧入口随调用方切换移除 |
| [state/perception.rs](../src/frame/lib_opendan/src/state/perception.rs) | 按 Session 的感知流、seq 幂等、整理游标、backlog；每个文件只有持 Session lease 的 driver 一个写者 | 保留为唯一感知管线；扩展来源、cites、锚点、`idempotency_key` 等字段，提供库级适配、逐条处置；按 A.5 加文件级短锁，使正文清理与追加互斥 |
| [state/cognition.rs](../src/frame/lib_opendan/src/state/cognition.rs) | Graph Hint 查询、Notebook 写入、整理审计与游标推进 | 后续将 Notebook 写入改为统一 Memory 路径，移除 Notebook 专属类型和统计；整理提交需核验实际认知更新 |
| [tests/self_improve.rs](../src/frame/lib_opendan/tests/self_improve.rs) | 感知幂等、整理锁、游标及防自我回声测试 | 增加真实认知变化、通知、清理和失败恢复断言，不以模拟回答“已记住”作为成功依据 |
| [agent_attention_signal.rs](../src/frame/agent_tool/src/agent_attention_signal.rs) | 独立 SQLite 管线、Stage1 扫描与待处理信号接口 | TD-20 要求并入或退役；可复用校验与提及过滤，不为新例子再建一个待整理池 |

实现现状的完整核对与调整清单见 [Memory 需求附录 B](<../doc/opendan/Agent Memory 认知管理需求.md>)（TD-01—TD-26），接口目标见附录 A；本 TODO 的 T01—T04 是其中组件阶段的子集。

存储基线已经确定（A.1、A.2、A.8）：感知沿用 `<agent_root>/state/perception/<sid>.jsonl`；认知沿用 `<agent_root>/memory` 的 Graph，`.meta/occasions.jsonl` 是追加写的提交真相源。索引、SQLite 与快照可重建，不另选一套事实库。需要收敛的是两层之间的库级适配、提交与处置恢复、统一可见版本。topic、观察进度与已读认知按 A.9 归 Session，不进 `memory_root`，也不另建变化日志。旧 Notebook 和 attention signal 调用方的切换与整体退役放到 S 阶段，不启动旧 Stage1/Stage2 behavior。

### 2.1 已确定的抽象边界：显式记忆请求也进入 Memory

本次设计收敛为：取消独立 Agent Notebook，已同步到需求文档 M-01；本 TODO 不再把“Notebook 是否承载认知”列为待决项。

“你记一下”是用户表达记忆意图的一种普通说法。它与 Agent 主动注意到的信息共用感知、认知、召回和修订生命周期，没有必要为它单设 Notebook、Note 对象或写入工具。用户无需知道 Session 或跨 Session 存储，也不必先说出特定口令才能获得连续体验。

| 用户表达 | 新抽象中的处理 |
|---|---|
| “你记一下，这个项目游戏页以后不要闪烁。” | 写入带原始来源和项目范围的感知；保留“用户明确要求记住”的意图。成功持久化后确认，立即可由其它相关 Session 召回，后续由 SI 整理 |
| “这个项目游戏页以后不要闪烁。” | 同样可以触发感知写入；同样属于用户明确表达的要求。未说“记住”不降低这条要求的证据性质 |
| “把这段原文保存成一份会议笔记，放到项目文档里。” | 创建用户可查看、编辑和引用的文档等世界对象；Memory 可保存其关联线索，不承担原文文档的管理职责 |

这里需要保留的是来源、适用范围和记忆意图，不预先增加独立类型或永久置顶机制。明确要求记住的内容不能被整理器当作普通噪声静默丢弃：完成吸收时应能指出承接它的认知；被纠正或替代时保留处置依据。明确意图也不意味着所述外部事实永久正确，仍按来源、时间和范围使用。

组件阶段抽取必要机制并让新例子走统一 Memory；正式接入阶段切换旧调用方，再移除 Notebook 模块、工具及模板引用。不为旧 Notebook 新增兼容层，也不因本次文档决策直接删除现有数据。

### 2.2 附录 B 的实施归属

保留 T01—T04 作为组件工作包。下表覆盖全部 TD 编号；跨阶段项只有在对应接入也完成后才能整体关闭，不能因模拟通过就标为完成。

| 附录 B 优先级与编号 | 本阶段工作包 | 后续部分或退出边界 |
|---|---|---|
| P0：TD-01、TD-10 | T02/T04：失败与空结果分开，无入口不全库召回 | S：Runner 不吞错，真实运行记录与 load_hints 测试 |
| P0：TD-02、TD-05、TD-08、TD-11 | T01/T03/T04：整理写权限、证据来源、枚举/状态迁移、路径段校验 | S：真实 lease/Session 绑定、CLI 分派及 PATH 限制 |
| P0：TD-03、TD-04 | T03/T04：单 envelope 原子提交、幂等、逐条处置、正文清理 | S：`commit --plan` 与成功终局接线，不再仅凭 Session 成功推进游标 |
| P0：TD-06、TD-07 | T01/T02：别名歧义、明确认知不淡出 | S：透传歧义与完整 Hint 元数据 |
| P0：TD-09 | T01：组件新路径不依赖 Notebook，列出可复用机制与数据清点范围 | S：整体移除旧模块、note 命令、模板与 Self-Check 调用 |
| P1：TD-12、TD-16 | T01/T03：字段、稳定 ID/revision、来源链、感知扩展与迁移方案 | S：Runtime 自动提供锚点、来源及补写标记，CLI 参数接线；Q：真实解读 |
| P1：TD-13、TD-14 | T02：两层候选、结构/全文召回、纠正兜底、分池预算与解释性输出 | S：真实 `<hints>`/附加块装配；Q：语义筛选质量 |
| P1：TD-15、TD-18 | T01/T02：A.9 的版本向量快照、Session 侧 `ObservationState`、有界 topic、变化阀门、读过即关注、成功装配后确认 | S：创建方建立基础范围、每次推理前执行，并把 `ObservationState` 存入真实 Session 状态 |
| P1：TD-17 | T01/T03/T04：库返回真实 ID/状态、证据必填、多别名与冲突结果 | S：CLI JSON/文本返回与行为模板不再猜 ID |
| P1：TD-19 | T03：廉价待处理查询、到期处置、清理重试与无自回声 | S：维护型整理 Goal/Loader 接线；Q：实际整理判断 |
| P1：TD-20 | T01：唯一感知管线，不接旧信号池 | S：迁移调用方、退役旧 CLI/behavior；事后回扫是否采用仍待决 |
| P2：TD-21 | T02：读路径不取写锁，`changes_since` 快速路径不打开 Graph（每次推理前都会调用，不能等到 T04） | S：真实 Runner 的读并发 |
| P2：TD-22—TD-25 | T04：成本基线、派生物重建、replay/compact/verify、schema 升级读写策略（含 `primary_language` 与分词器） | 大规模增量物化和读性能优化单列后续 P2，不阻塞最小例子；涉及正确性的恢复与错误可见性不能延后 |
| P2：TD-26 | T04 与 §6：补组件测试 | S：CLI 图操作、真实召回注入与运行时测试 |

## 3. 模拟例子

### 3.1 参与者和初始材料

所有名字和内容均为测试数据；`a1/u1/snake-alpha` 等是例子别名，不规定最终 DID/ObjectRef 编码。

| 参与者 | 身份与关注范围 | 用途 |
|---|---|---|
| Session A | Agent a1，用户 u1，项目 snake-alpha 的游戏页 | 记录用户表达和纠正 |
| Session B | 同一 Agent、用户及项目 | 设置 topic，读取 Hint，观察后续变化 |
| Session B2 | 与 B 相同的可见范围，独立 Session | 验证召回和通知进度互不消费 |
| Session C | 同一 Agent、用户，项目 snake-beta | 验证 alpha 的项目偏好不会串入 beta（相关性过滤） |
| Session G | 同一 Agent，用户 u2，同样在 snake-alpha 游戏页工作；授权主体集合只有 u2 和 Agent 自身 | 对象范围与 B 完全重叠，验证按 A.3.7 的可见性过滤看不到 u1 的认知、感知、变化提示和来源存在性（权限过滤，B-13） |
| 模拟 Goal Session SI | 同一 Agent，持有由模拟宿主绑定的整理 lease | 模拟维护型 Memory 整理 Goal 的一轮运行，读取待整理材料并提交预设认知更新 |

初始化一条有原始事件依据的认知 `c0@1`：“u1 在 snake-alpha 游戏页更重视操作稳定性。”初始化也走真实提交接口。后续查询同时包含它和新感知，验证两层内容能够共同显露。

世界对象 fixture 单独保存项目的当前选择与版本。Memory 只引用它，不把“曾采用 alpha”解释成永远采用 alpha；本例不实现 Workspace 或产物回滚。

### 3.2 主流程和预期变化

| 步骤 | 调用与输入 | 预期返回、状态与通知 |
|---|---|---|
| 1 添加感知 | A 记录 `p1`：“这个项目游戏页面以后用静态背景，不要闪烁。” | 返回感知引用和写入版本；`p1=pending`，带 u1、项目/页面范围和原始事件引用；不自动形成认知 |
| 2 设置话题 | B 调用 `set_topic`，topic 为“修改游戏背景效果”，主体 u1、对象 snake-alpha/game_page，包含两层 Hint | 返回 `c0@1` 认知 Hint 和 `p1` 待整理感知 Hint 及观察起点快照；B 的 `ObservationState` 记下 topic revision 1，Memory 侧不登记订阅 |
| 3 独立关注 | B2 设置相同 topic；C 设置 snake-beta 的背景效果 topic；G 设置与 B 相同对象的 topic | B2 独立收到 `c0@1+p1`；C 因相关性、G 因可见性都收不到 alpha 中 u1 的内容；四者各有自己的观察状态 |
| 4 新增相关感知 | B2 记录 `p2`：“静态版操作不受干扰，就保持这样；演示页另说。” | 写入只推进 B2 感知分量的 seq；B 下次观察时可拉到变化，C、G 拉不到；任何 Session 都不因此立即启动推理 |
| 5 观察变化 | B 在正常观察点调用 `changes_since`，成功装配后推进自己的进度 | B 收到 `p2` 的待整理 Hint；未变化且已显露的 `c0@1/p1` 不机械重发；B2 是 `p2` 写入方，不把该感知推回它自己 |
| 6 模拟整理 | SI 读取 `p1/p2`、已有认知与原始事件，提交预设结果 `c1@1` | 形成“仅在 snake-alpha 游戏页默认静态、不闪烁，以免干扰操作”的偏好；保留演示页例外和原始来源；可靠提交后清理 `p1/p2` 正文 |
| 7 观察升级 | B 先读取并确认；B2 随后独立读取 | 两者都可收到 `c1@1` 及材料吸收标记；进度各存在自己的状态里，B 的确认不影响 B2。B2 不会因曾写 `p2` 而漏掉 SI 形成的认知 |
| 8 明确纠正 | A 记录 `p3`：“游戏页可以缓慢移动，但仍不能闪烁。”并关联 `c1` | 在同一可见提交中记录 `p3`；组件据其附带的 `c1` 引用，把 `c1@1` 派生显示为待复核，不改写认知本身；后续查询和观察不会无提示地返回“必须静态” |
| 9 模拟修订 | SI 按预设决策将 `p3` 整理为 `c1@2` | 同一逻辑认知更新为“允许缓慢移动，不闪烁、不干扰操作”；旧版被替代，`p3` 成功吸收后清理；B/B2 可再次收到新版 Hint |
| 10 新 Session 与重启 | 重开 Memory，创建同范围 Session D；B/B2 从各自保存的 `ObservationState` 恢复 | D 召回 `c0@1+c1@2`；不召回已清理感知，不把旧版作为当前有效认知召回；B/B2 的已确认进度、pending 与 read_set 仍有效，Memory 侧没有需要恢复的订阅 |

“包含认知和感知”指变化通道支持两层内容，不要求每次新增感知都生成认知或重发所有认知。初次 `set_topic` 返回有界快照；后续返回同一关注范围的有界增量，认知新增、修订、失效和感知新增、清理都能被表达。

例子的 console/JSON 输出应逐步展示：调用者与角色、基础范围和追加 tag、两层 `memory_snapshot`、Hint 的 layer/ref/revision/scope/basis/weight/confidence、命中原因、已读认知关注、实际装配与未装配条目、逐条处置和清理结果。可阅读摘要与机器断言来自同一执行结果；fixture 的来源事件和预设决策单独标出。

### 3.3 拟议调用示意

以下伪代码用于确认边界，不是最终 Rust 签名；每个客户端的身份与授权上下文由模拟宿主绑定。

```text
p1 = A.memory.record_perception(content, source_event, scope, idempotency_key)

// set_topic 是 Session 侧库函数：更新 B.obs（A.9 ObservationState），
// 按变化阀门决定是否调用 memory.query_topic（先取快照再读内容）
initial = B.set_topic(
    topic_id="main",
    text="修改游戏背景效果",
    subjects=[u1], objects=[snake_alpha_game_page],
    layers=[cognition, perception], budget=fixture_budget
)
// initial = { cognitions, perceptions, snapshot, truncated }
if B.observe(initial) { B.obs.accept(initial) }   // 成功装配后才推进快照、登记 read_set

p2 = B2.memory.record_perception(content, source_event, scope, idempotency_key)
delta = B.memory.changes_since(B.obs.snapshot, B.obs.scope(), B.obs.read_set, limit)
if B.observe(delta) { B.obs.accept(delta) }       // 预算外引用进 B.obs.pending

batch = SI.memory.begin_consolidation(limit=fixture_batch_limit)
materials = SI.memory.read_batch(batch)
plan = scripted_decision(materials, existing_cognitions, source_fixture)
// plan = A.4.1 ConsolidationCommit:
// { idempotency_key, produced_by, operations, dispositions, ... }
commit = SI.memory.commit_consolidation(batch, plan, consolidation_lease)
SI.memory.cleanup_consolidated(commit)

delta = B.memory.changes_since(B.obs.snapshot, B.obs.scope(), B.obs.read_set, limit)
if B.observe(delta) { B.obs.accept(delta) }
```

`B.observe` 只更新模拟 Session 的可见材料并输出 trace，不拼接真实 prompt；模拟装配失败时 `accept` 不执行，快照、pending 与 read_set 都不推进。`accept` 只写 Session 自己的状态，不调用 Memory。`commit_consolidation` 是案例逻辑名，最终复用 `AgentMemory::commit` 的单 occasion 提交。普通客户端不能绕过整理 lease 直接调用图写入。组件提供按引用读取正文和来源的接口；Hint 本身保留必要摘要、范围、证据性质及展开引用。

### 3.4 用同一件事区分 history 感知 认知和 Hint

| 层次 | 本例的具体内容 | 生命周期和作用 |
|---|---|---|
| UI Session history | A 在 09:00 收到用户消息：“这个项目游戏页面以后用静态背景，不要闪烁。” | 当次会话的原始消息；不是每条 history 都自动成为 Memory |
| 感知 `p1` | “u1 明确要求 snake-alpha 游戏页以后用静态、不闪烁背景。”并引用 A 的原始消息 | 低成本记下这次注意到的要求；尚未整理，但可以作为有出处的新近观察提醒其它 Session |
| 证据观察 `obs1` | SI 整理时保存的用户表达依据，`source_ref.event_ref=A/e31` | Graph 的 `MemoryObservation`，与待清理的 Perception 不同；认知的 `evidence` 引用它，它再指向原始事件 |
| 认知 `c1@1` | “u1 在 snake-alpha 游戏页希望默认静态、不闪烁，以免干扰操作；不自动适用于演示页或其它项目。” | 整理 `p1/p2` 后形成的可复用判断，带原因、适用边界、证据和版本；可以修订 |
| Hint | “当前项目游戏页有不闪烁的背景要求，详见 c1@1。”连同范围和证据性质 | 当前情境中显露的短线索，指向同一认知；不是另外复制一份认知 |
| 半订阅变化 | “你关注的游戏背景要求有新观察 p2”或“c1 已修订为 @2” | 在下一次观察时提醒当前 Session；确认进度属于 Session 运行状态（A.9），不是新的认知 |

感知和认知的区别不由字数、JSON 还是自然语言决定。`p1` 的用户原话可以很明确，认知也可能仍是暂定判断；关键在于是否经过整理，是否给出了可复用的结论、适用条件和依据。一次明确表达也可以形成窄范围认知，不要求凑够两次观察。

下面的 history 是拟议行为样例，不是运行日志。时间统一为 UTC+08:00，人物、消息和工具输出均为 fixture。A/B/B2 延续 §3.1；工具经验和工作区选择使用独立 fixture，避免混淆对象范围。

表中的“组件调用”是模拟宿主直接执行的库调用，不是用户说出命令，也不是要求 LLM 操作锁或游标。身份、真实 Session、时间和事件引用由宿主绑定，模型只需提供观察内容及必要对象线索。自然语言写入决策、topic 提取和整理决策在例子中预设。

每段“下一次模型输入中的 Memory 材料”列出 **SimSession.history 应出现的完整 Memory 片段**。正式接入时，Runtime 在每次推理前检查已订阅增量，可将观察随 function call result 一起装配，也可使用明确标注的观察片段，具体消息格式不在这里冻结。需要区分工具实际返回与 Runtime 附加材料，不伪装成用户新发言，不提升为 system 指令。本阶段只生成 history 预览，不改正式 prompt 模板。写入回执、快照、锁、批次和确认由宿主处理，不全部塞给模型。

### 3.5 场景一 新 UI Session 尚未想到存在背景偏好

B 是新开的手机 UI Session，没有 A 的对话历史。它只知道当前 UI 选中的项目和页面是 snake-alpha/game_page。用户没有提“不闪烁”，B 也没有先发起“查询用户偏好”。

| 时间与 Session | history 内容或可见动作 | Memory 组件调用与结果 |
|---|---|---|
| 10-08 18:00，历史 Session H | 用户：“alpha 的游戏页先保证操作稳定，装饰其次。”原始事件 `H/e7` | fixture 先写感知，再用预设整理计划提交 `c0@1`；正文为 §3.1 的操作稳定性认知，来源 `H/e7` |
| 10-09 09:00，A 网页 UI | 用户：“这个项目游戏页面以后用静态背景，不要闪烁。”原始事件 `A/e31` | `A.memory.record_perception(content="u1 要求 snake-alpha 游戏页以后用静态、不闪烁背景", scope=alpha_game, source=A/e31, key="A/e31/observation-1")` → `p1, pending` |
| 09:00:02，A | Assistant：“已记下这个项目游戏页的背景要求。” | 写入回执只代表感知已记录；没有运行 SI，也没有声称认知已形成 |
| 09:30，B 手机 UI | 用户：“把这个游戏背景再做得活泼一点。”原始事件 `B/e1` | 暂无 Memory 调用；B 的可见 history 中没有“不闪烁” |
| 09:30:01，B 的模拟宿主 | 根据当前请求设置普通业务 topic：“调整游戏背景效果” | `B.set_topic(topic_id="main", text="调整游戏背景效果", subjects=[u1], objects=[snake_alpha_game_page], layers=[cognition, perception])` → 内部 `query_topic` 返回认知 `c0@1`、感知 `p1` 和快照 `S0` |
| 09:30:02，B | Runtime 将下面的 Memory 材料加入本次输入 | `B.observe(result)`；装配成功后宿主把 B 的进度推进到 `S0`，`read_set` 记入 `c0@1`；这一步只写 B 的状态，不新增 history 文本 |
| 09:30:03，B | Assistant：“我会先通过配色和图案让背景更活泼，保留这个项目游戏页静态、不闪烁的要求。” | 后续任务参数同时包含本次目标和上述项目边界；本例只输出参数预览，不创建真实 Work Session |

下一次模型输入中的 Memory 材料：

```text
[Memory 线索：调整游戏背景效果]
以下是与当前任务相关的历史材料；适用范围和证据性质随条目给出。

[认知 c0@1｜有效｜依据：用户明确表达]
u1 在 snake-alpha 游戏页优先考虑操作稳定性，再考虑装饰效果。
范围：u1 / snake-alpha / game_page。来源：H/e7。
相关原因：当前任务修改同一游戏页的视觉效果。

[感知 p1｜待整理｜来源：用户消息，2026-10-09 09:00+08:00]
u1 在另一会话中要求这个项目游戏页以后使用静态、不闪烁的背景。
范围：u1 / snake-alpha / game_page。原始事件：A/e31。
这是一条尚未整理的新近观察，不代表用户在其它项目中也反对动画。
相关原因：当前对象相同，当前 topic 涉及背景效果。
```

此处解决的“不知道自己不知道”是：**B 不知道另一会话里出现过背景要求，所以不会主动搜索它；B 只声明当前在做什么，Memory 就提供了具体相关线索。** `set_topic` 的输入不包含“不闪烁”、`p1` 或 `c0`，这些内容来自 Memory。缺少这一步时，B 的当前 history 只有“活泼一点”，没有依据自行知道那条项目边界。

例子用对象范围和明确的背景/效果主题索引完成匹配，可以在 fixture 中定义“背景效果、动效”关联。不能把目标认知 ID 藏在查询参数里，也不能把手工指定命中结果当成检索实现。真实语言下的 topic 提取、相关性排序和召回质量仍需后续评估。

同一场景再运行一个独立变体：将 A 在 09:00 的原话改为“你记一下，这个项目游戏页面以后用静态背景，不要闪烁。”仍调用同一个 `record_perception`，并从来源中保留明确记忆意图；09:00:02 的确认以持久化成功为前提。09:30 的 B 无需等待 SI，也无需读取 Notebook。其感知片段可以写为：

```text
[感知 p1｜待整理｜用户明确要求记住]
u1 要求记住：snake-alpha 游戏页以后使用静态、不闪烁的背景。
范围：u1 / snake-alpha / game_page。原始事件：A/e31。
相关原因：当前任务涉及同一游戏页的背景效果。
```

这个变体与原场景使用相同存储和召回路径。显式请求帮助确定写入意图；未加“你记一下”的原场景也必须能够成立。

### 3.6 场景二 其它 Session 有新观察 正在工作的 Session 获得提示

B 已有 §3.5 的 history；B2 是同一用户的另一 UI Session，09:30:10 设置相同 topic 并确认了自己的初始快照。C 此时只关注 snake-beta。下面继续同一时间线。

| 时间与 Session | history 内容或可见动作 | Memory 组件调用与结果 |
|---|---|---|
| 09:31，B2 | 用户：“刚试了静态版，操作不受干扰，就保持这样；演示页另说。”事件 `B2/e12` | `B2.memory.record_perception(content="用户确认静态背景不干扰操作，要求本游戏页保持，演示页另说", scope=alpha_game, source=B2/e12, key="B2/e12/observation-1")` → `p2, pending` |
| 09:31:01，Memory | 只追加了 `p2`，B2 的感知分量推进；Memory 不知道 B 在关注，B 当前也没有新的推理机会 | 不给 B 增加用户消息，不触发 LLM。C 的范围不匹配，G 不可见 |
| 09:32，B | 当前预览检查返回，模拟下一次正常观察 | `B.memory.changes_since(B.snapshot, B 的范围, B.read_set)` → `perceptions=[p2]`；宿主将下面第一段加入 history，装配成功后推进 B 的进度 |
| 09:32:01，B | Assistant：“游戏页继续保持这个边界；演示页按它自己的需求处理。” | 同版本 `c0@1/p1` 不需要再次注入 |
| 09:35，SI | 开始后台整理 | `SI.memory.begin_consolidation(limit=10)` → 批次 `batch-1`；`read_batch(batch-1)` → `p1/p2`；`read(c0, view="cognition")` 和 fixture 原始事件读取提供既有认知及证据 |
| 09:35:01，SI | 预设整理决策：两次明确表达范围一致，第二次补充了原因和例外 | `commit_consolidation(batch-1, plan-1, key="batch-1/commit")` → 已提交 `c1@1`，`p1/p2` 已吸收，产生升级变化 |
| 09:35:02，SI | 清理成功提交的材料 | `cleanup_consolidated(commit)` → `p1/p2` 正文删除；保留最小处置记录及原始来源引用 |
| 09:36，B | 又一个正常观察机会 | `changes_since(...)` → `c1@1` 和 `p1/p2 → c1@1` 的吸收标记；第二段进入 history，装配成功后推进进度并把 `c1@1` 记入 `read_set` |

09:32 实际新增的半订阅材料只有这段：

```text
[Memory 半订阅变化：调整游戏背景效果]
[感知 p2｜待整理｜来源：用户消息，2026-10-09 09:31+08:00]
u1 在另一会话中确认静态版操作不受干扰，要求 snake-alpha 游戏页保持；演示页另说。
范围：u1 / snake-alpha / game_page。原始事件：B2/e12。
相关原因：补充了你正在关注的游戏背景要求的原因和边界。
这是新近观察，不扩大为对所有页面或其它项目的统一要求。
```

SI 的 `plan-1` 应实际提交以下内容，而不只是返回一句“已记住”：

```text
新增认知：c1@1，kind=attribute，semantic_kind=preference，status=active
证据性质：basis=user_statement，explicit=true；weight 与 confidence 分开保存。
正文：u1 在 snake-alpha 游戏页希望默认使用静态、不闪烁的背景，以免干扰操作。
范围：u1 / snake-alpha / game_page
例外：不自动适用于演示页；不自动适用于其它项目。
依据：evidence=[obs1, obs2]；obs1.source_ref.event_ref=A/e31，obs2 指回 B2/e12。
证据观察保留，p1/p2 正文清理；认知不只引用将删除的感知。
复核条件：用户改变要求；准备应用于其它页面或项目。
召回入口：当前项目的背景效果、视觉改动、操作干扰，均引用 c1。
感知处置：两条 disposition 均为 outcome=absorbed，cognition_refs=[c1@1]。
提交：图操作与逐条处置合在一条 occasion envelope，带幂等键和 produced_by。
```

09:36 实际新增的半订阅材料：

```text
[Memory 半订阅变化：调整游戏背景效果]
[认知 c1@1｜有效｜依据：用户明确表达及后续反馈]
u1 在 snake-alpha 游戏页希望默认使用静态、不闪烁的背景，以免干扰操作。
范围：u1 / snake-alpha / game_page。演示页及其它项目不自动适用。
来源：A/e31、B2/e12。相关原因：你关注的背景要求已完成整理。

[材料状态变化]
此前的感知 p1、p2 已被 c1@1 吸收，感知正文已清理。
后续使用引用 c1；这是同一要求的整理结果，不是三条独立证据。
```

B 因此能知道“另一个会话刚补充了边界”和“原材料已经整理”。B2 不接收自己刚写的 `p2`，但有独立的认知变化进度，B 的确认不会替它消费 `c1@1`。另运行 B 延迟到清理后才观察的变体：只能得到当前认知与吸收标记，不能重放 `p2` 正文。清理不等待所有 Session 读完感知。C 的 topic 只关注 beta，因此没有相关通知，这是相关性过滤；G 与 B 关注同一对象，却因 `c1`、`p1/p2` 的主体是 u1 而看不到任何条目、吸收标记或“存在相关记忆”的提示，这是权限过滤。两道过滤分别断言。

### 3.7 场景三 已经读入的旧认知被用户纠正

B 的 history 中已经有 `c1@1`。仅修改数据库不能使它自动知道旧结论失效，下面展示需要额外进入 history 的内容。

| 时间与 Session | history 内容或可见动作 | Memory 组件调用与结果 |
|---|---|---|
| 09:40，A | 用户：“换一下，这个游戏页可以用缓慢移动的背景，但仍然不能闪烁，别影响其它项目。”事件 `A/e33` | `A.memory.set_topic(text="调整游戏背景效果", objects=[snake_alpha_game_page], ...)` 得到当前 `c1@1`；模拟决策将这次纠正关联到该认知 |
| 09:40:01，A | 记录新要求，不等待完整 SI | `record_perception(content="游戏页允许缓慢移动，仍禁止闪烁，仅限本项目", source=A/e33, scope=alpha_game, suggested_kind="correction", cites=[c1@1], key="A/e33/observation-1")` → `p3` 可见时，`c1@1` 同时派生显示为待复核；认知自身未改写 |
| 09:41，B | 正常观察点；原 history 中仍保留“默认静态” | `changes_since(...)` → `p3` 及其派生的 `c1@1` 待复核提示（p3 在 B 的范围内，经范围匹配返回）；宿主追加下面第一段 |
| 09:41:01，B | Assistant：“这个项目的要求刚更新为可以缓慢移动，仍不闪烁；我会按更新后的边界继续。” | 即使 SI 尚未完成，也不会继续无提示地坚持“必须静态” |
| 09:45，SI | 整理 `p3` 和 `c1@1`，核对同一主体明确调整了同一范围的要求 | 同一 plan 新增证据观察 `obs3 → A/e33`，以 `expected_revision=1, evidence=[obs1,obs2,obs3]` 修订 `c1` 并处置 `p3`；带幂等键提交 → `c1@2`，随后清理 `p3` |
| 09:46，B | 下一次观察 | `changes_since(...)` 返回修订和吸收变化，追加第二段；已见过 `c1@1` 不会抑制 `c1@2` |

09:41 注入的内容：

```text
[Memory 半订阅变化：调整游戏背景效果]
[旧认知待复核 c1@1]
你此前收到的“该游戏页默认静态”出现了同一用户的明确纠正，不能继续作为当前默认要求。

[感知 p3｜待整理纠正｜来源：用户消息，2026-10-09 09:40+08:00]
u1 现在允许 snake-alpha 游戏页使用缓慢移动的背景，仍禁止闪烁；其它项目不受影响。
范围：u1 / snake-alpha / game_page。原始事件：A/e33。关联旧认知：c1@1。
正式认知修订尚未完成；本条呈现的是用户的新要求及旧结论需要复核的状态。
```

09:46 注入的内容：

```text
[Memory 半订阅变化：调整游戏背景效果]
[认知 c1@2｜有效｜替代 c1@1]
u1 允许 snake-alpha 游戏页背景缓慢移动，但不应闪烁或干扰操作。
范围：u1 / snake-alpha / game_page。演示页及其它项目不自动适用。
当前修订依据：A/e33。历史要求及边界依据：A/e31、B2/e12。
p3 已被本次修订吸收；“必须静态”不再是当前有效要求。
```

这里新增的是有出处的修订材料，不能假设已经删掉 B 的旧 history。对 topic 仍命中的 B，认知 revision 与“待复核”状态变化都必须触发有效性提示，不能因为 B 读过同一条认知就压掉通知；话题已转走的 Session 只在整理提交后经 `read_set` 收到修订提示（§3.12）。例子预设 `suggested_kind="correction"` 与 `cites` 中的纠正目标，不能由此声称组件能自动理解任意自然语言冲突。

待复核是组件根据纠正及其 `cites` 目标派生的召回状态，A 并没有改写 `c1`（Memory 需求 M-32、§4.3）。A 刚通过 `set_topic` 拿到 `c1@1`，附带引用几乎没有成本。如果只标为 correction、未给出认知引用，则走兜底路径：任何 Session 查询或召回 `c1` 时，仍附带同主体、同范围下未整理的纠正感知 `p3`，标注“可能已被纠正”。普通回声中的 cites 只表达来源关系，不能一概当成纠正。

### 3.8 场景四 工具观察形成带条件的经验

这组 fixture 使用测试工具 `effect-validator v1` 和 `sprite-effect` 资源。来源是工具结果，不能写成用户偏好，也不能把一次校验成功当作用户认可产物。

| 时间与 Session | history 内容或可见动作 | Memory 组件调用与结果 |
|---|---|---|
| 10-07 14:00，Work V1 | 工具返回 `{"tool":"effect-validator","version":"v1","resource_type":"sprite-effect","error":"ASSET_KEY_MISSING","field":"tick_ms"}`，调用 `V1/call8` | `record_perception(content="v1 校验 sprite-effect 时提示缺 tick_ms", attributes=工具结构化结果, source=V1/call8)` → `pv1` |
| 14:03，Work V1 | 补字段后工具返回 `{"ok":true}`，调用 `V1/call9` | `record_perception(content="补齐 tick_ms 后同工具同资源校验通过", source=V1/call9)` → `pv2`；不记录“用户接受了版本” |
| 10-08 11:00，Work V2 | 独立任务出现相同工具版本、资源类型和缺字段错误，调用 `V2/call4` | `record_perception(content="独立任务重复出现相同现象", source=V2/call4, ...)` → `pv3` |
| 11:30，SI | 预设决策将三次观察整理为有条件的检查经验 | 提交 `cv1@1`：“处理 effect-validator v1 的 sprite-effect 资源时可优先检查 tick_ms；版本或资源类型改变后重新核对规范。”保留三条工具来源；提交后清理 `pv1/pv2/pv3` |
| 10-09 10:00，新 UI Session V3 | 用户：“给这个游戏加一个尾焰效果。”当前环境已知将生成 sprite-effect 并用 v1 校验 | `set_topic(text="制作并校验游戏效果资源", objects=[当前资源, effect_validator_v1], ...)` → `cv1@1`；请求中没有 `tick_ms` 或错误码 |
| 10:00:01，V3 | 宿主加入下面的 Memory 材料 | `observe(result)` 后确认；V3 的后续任务参数加入“生成前核对资源规范及 tick_ms”的检查步骤 |

具体注入内容：

```text
[Memory 线索：制作并校验游戏效果资源]
[认知 cv1@1｜经验建议｜依据：工具观察]
过去两个任务中，effect-validator v1 校验 sprite-effect 时曾提示缺少 tick_ms；
其中一次补字段后校验通过。当前使用同工具版本和资源类型，可以优先核对该字段。
适用条件：effect-validator v1 / sprite-effect。版本或资源类型变化时需重新检查当前规范。
来源：V1/call8、V1/call9、V2/call4。
相关原因：本任务将使用相同校验工具和资源类型。
这是检查线索，不证明该字段是唯一原因，也不代表用户已经认可任何产物。
```

V3 没有经历过这个报错，也没有想到查询它；当前工具和资源线索让经验提前出现。如果环境换成 v2，不能继续无条件返回“必须补 tick_ms”，应排除旧版本经验或明确标为需要重新验证。fixture 还应包含“一次用户未回复”等弱观察，SI 对它输出暂缓或丢弃，而不是强行产生“用户偏好短回答”的认知。

### 3.9 场景五 历史选择提醒去查当前世界状态

这是独立 fixture：项目为 `project:snake`，候选工作区为 `ws-alpha/ws-beta`，不要与前述按项目隔离的 `snake-alpha/snake-beta` 混为一组对象。新 UI Session E 通过当前页面知道项目是 snake，但尚不知道选定哪个工作区。

| 时间与 Session | history 内容或可见动作 | 组件调用与结果 |
|---|---|---|
| 10-08 16:00，UI Session W | 用户：“这次用 alpha，操作比较稳定。”事件 `W/e22` | `record_perception(content="用户本次比较选择 ws-alpha，理由是操作稳定", scope=project_snake, source=W/e22)` → `p-choice` |
| 16:30，SI | 预设整理为历史选择依据 | 提交 `c-choice@1`，保留 `project:snake/selection` 引用及“操作前查当前选择”的适用条件；清理 `p-choice` |
| 10-09 09:10，世界对象 fixture | 后来用户已通过其它入口采用 beta，权威选择对象变为 `workspace=ws-beta, version=v7` | 更新世界对象 fixture；Memory 不因此把过去“曾选 alpha”的历史改写成从未发生 |
| 10:30，E | 用户：“把刚刚那个贪吃蛇继续加点动效。”事件 `E/e1` | `set_topic(text="继续修改贪吃蛇效果", objects=[project_snake], ...)` → `c-choice@1`；未把 ws-alpha 预设成当前工作基础 |
| 10:30:01，E | 加入下面的 Memory 线索后，意识到需要确认工作区 | 调用世界对象 fixture 的 `objects.read(project:snake/selection)` → `{"workspace":"ws-beta","version":"v7"}`；这不是 Memory 查询结果 |
| 10:30:02，E | Assistant：“当前采用的是 beta v7，我会基于它继续改。” | 模拟新任务参数 `workspace=ws-beta, base_version=v7`；必要时对已确定对象再次 set_topic 查询它的适用认知 |

具体注入内容：

```text
[Memory 线索：继续修改贪吃蛇效果]
[认知 c-choice@1｜历史选择依据]
u1 曾在一次比较中选择 ws-alpha，理由是操作更稳定；该项目存在多个候选工作区。
范围：u1 / project:snake。来源：W/e22，2026-10-08 16:00+08:00。
当前选择由对象 project:snake/selection 管理；本条不能证明 alpha 仍然是当前版本。
相关原因：当前请求继续修改同一项目，但尚未确定工作区。
```

线索使 E 从“只知道要继续修改项目”变成“知道存在多个候选，需要读取 selection”。它没有替 E 猜目标。只有随后世界对象的返回值才能确定 beta v7；没有可读选择记录时，预期行为是保留歧义并澄清，而不是按 Memory 时间或 Session 更新时间选一个。

### 3.10 从案例反推需要多少接口

| 使用者及目的 | 案例中的逻辑调用 | 与现有组件的关系和改动边界 |
|---|---|---|
| 普通 Session 记下观察 | `record_perception` | 对应现有 `Perception::append` 一类能力；需要补齐来源、范围及不依赖输入队列的库级入口，不要求另造一套事实库 |
| 宿主建立基础范围，模型追加情境 | `set_topic`（Session 侧库函数）→ `query_topic` | 更新 Session 的 `ObservationState`，按阀门调用组合了 `load/recall_hints` 的 `query_topic`；基础范围、可淘汰 tag、`read_set` 分开保存。不是要求 Work 靠模型建立基础订阅 |
| 主动搜索及读详情/来源 | `query / read(ref, view)` | 复用 Graph `load/get`；补按主体和来源视图、逻辑 ID 当前版及固定 revision 读取。感知仅为内部临时材料，不默认暴露为 DID Object；真实 DID 映射属于 S |
| 模拟宿主收取变化 | `changes_since`，确认写 Session 状态 | Memory 无订阅注册表，只按快照、范围和 `read_set` 返回变化（A.9）；确认不调用 Memory。属宿主内部协议，不变成给模型调用的工具 |
| SI 整理与清理 | `begin_consolidation / read_batch / commit_consolidation / cleanup_consolidated` | 复用 backlog、整理 lease、Graph commit 与写锁；落到 A.4.1 单 envelope 的 operations/dispositions，不以 SI 回答成功或字节游标代替提交 |

一次普通 UI 交互通常只涉及“写一条观察”或“设置当前 topic 并接收结果”。批次、清理和确认是支撑这一体验的组件职责。团队需要 Review 的重点是：这些 history 中出现的材料是否恰当、是否足以让 Session 发现遗漏、是否保留范围与证据，以及能否用现有组件的少量扩展实现；接口命名和拆分方式可据此收敛。

### 3.11 场景六 运行中 Work 的修改不经过 Memory

这是边界 fixture，验证 Memory **没有**被当作 Session 之间的通信通道（Memory 需求 M-30、M-31、§9.4）。UI Session U 创建 Task T，为 `snake-alpha/game_page` 增加计分显示，由 Work W 执行，子 Session W-page 负责页面。运行中的修改写为 Task 修订，由 Runtime 建立的 Task 对象订阅送达。Task 修订、订阅和 Final Report 在本阶段只用 fixture 表示，不在 Memory 组件中实现；本场景的断言只针对 Memory。

| 时间与 Session | history 内容或可见动作 | Task 修订 fixture | Memory 组件调用与结果 |
|---|---|---|---|
| 10-10 10:00，U → W | 用户：“给游戏加计分显示。”形成 Task T 并启动 W | 任务书 T；宿主为 W 建立 T 的修订订阅，为 W-page 建立 `T/game_page` 子范围订阅 | 宿主代表创建方为 W 设置基础 topic（Task T、snake-alpha 游戏页与计分数据），召回已有认知，如 `c1@2` 的不闪烁要求 |
| 10:01:10，U | 用户：“正在做的这个页面，背景先改成蓝色。”事件 `U/e2` | 修订 #1：背景蓝色，范围 `game_page`，原文 `U/e2` | 无调用；Memory 中不出现针对本任务的感知 |
| 10:01:20，U | 用户：“计分数字也大一点。”事件 `U/e3` | 修订 #2：计分数字加大 | 无调用 |
| 10:02，W-page | 工具返回旧版预览，同一次运行继续推理，没有新 Message | 宿主装配修订 #1、#2；必达，不占 Memory 预算 | 宿主同时读取 Memory 增量：无相关变化 |
| 10:02:20，U | 用户：“蓝色改成深灰，数字加大的要求保留；以后这个项目都别用亮色背景。”事件 `U/e4` | 修订 #3：纠正 #1 为深灰，#2 继续有效 | `record_perception(content="u1 要求 snake-alpha 以后不用亮色背景", source=U/e4, scope=u1/snake-alpha, suggested_kind="preference")` → `p-bg-1`；只写长期部分，范围是项目而不是 Task T |
| 10:03，W-page 与 W | 各自下一次推理前观察 | 装配修订 #3 | 读取 Memory 增量：`p-bg-1` 作为待整理感知出现；即使这条没被召回，W 仍从修订 #3 得知要用深灰 |
| 10:04:50，U | 用户：“分数改成红色。”事件 `U/e5`；W 已完成最后一次推理，正在收尾 | 修订 #4 | 无调用 |
| 10:05，W | 输出 Final Report，W 结束 | Report：最后观察修订 #3；#2、#3 已落实，#1 被 #3 纠正 | 无调用 |
| 10:05:10，U | 宿主比对修订编号 | #4 大于 Report 记录的 #3，判定未处理；U 以有效产物为基础创建 T2/W2，任务书包含 #4 | 无调用；不靠模型对照反馈和 Report |
| 10:10，SI | 本轮整理 | — | 批次只有 `p-bg-1`，按预设决策形成项目范围的偏好认知或补充既有认知；没有一次性修改需要处置 |

断言：

- 整个场景中，Memory 只新增 `p-bg-1` 一条感知；修订 #1—#4 都不在 Memory 中，SI 不需要为一次性修改花一次整理。
- 只关闭 W 的 Memory 半订阅时，W 不注入 Memory 增量，U 仍可正常写 `p-bg-1`。另一个变体中，U 处于“本次不记忆”策略，才不写 `p-bg-1`。两种情况下 Task 修订都照常送达（fixture 断言），记录策略与观察开关独立。
- W 的 Memory 召回预算为 0 时，任务书和修订仍然完整，缺少的只是 `c1@2` 这类历史线索（Memory 需求 B-40）。

10:03 W-page 下一次模型输入的附加材料，修订和 Memory 材料分开标注：

```text
[Task T 修订｜必达｜来源：UI Session U]
#3 纠正 #1：游戏页背景改为深灰。原文 U/e4，2026-10-10 10:02:20+08:00。
当前有效修订：#2 计分数字加大；#3 深灰背景。
是否落实由后续执行验证，并在 Final Report 中说明。

[Runtime Memory 观察｜topic：Task T / snake-alpha]
[新近感知 p-bg-1｜待整理｜来源：用户明确表达]
u1 要求 snake-alpha 以后不用亮色背景。范围：u1 / snake-alpha。原始事件：U/e4。
这是供后续工作参考的线索，不是本任务的修订。
```

正式实现时，修订的数据结构、授权和订阅属于 Session 架构，见架构文档中 Work 执行边界一节，不进入 Memory 组件。

### 3.12 场景七 换了 topic 仍能知道已读认知被修订

独立 fixture：J 对 alpha 和 beta 均有读取权限，但当前 topic 只关注其中一个项目。SI 和用户事件沿用场景三的预设修订内容；J 的变化读取仍调用真实组件。

| 时间与 Session | history 内容或动作 | 组件调用与预期结果 |
|---|---|---|
| 11:00，J | 讨论 alpha 背景，成功读入 `c1@1` | `set_topic / observe`；装配成功后 J 把 `c1@1` 记入自己的 `read_set`，即建立修订关注 |
| 11:01，J | 话题转向 beta 的计分数据 | 更新动态 topic，revision 加一；旧 tag 不再进入查询范围，`read_set` 中的 `c1` 保留 |
| 11:01:30，A | 用户纠正，A 写入 `p3`（`cites=[c1@1]`，范围 alpha 游戏页）；SI 尚未运行 | 反向断言：J 在 beta 话题下观察，`changes_since` 不返回 `p3`，也不返回 `c1@1` 待复核。待复核是召回视图的派生状态，`read_set` 只对整理提交产生提示（M-21、A.9） |
| 11:02，SI | 提交 `c1@2`：alpha 游戏页允许缓慢移动，仍不闪烁，吸收 `p3` | 单 occasion 提交；J 的 `read_set` 含 `c1`，不依赖新 topic 再次命中 alpha |
| 11:03，J | 本次观察预算为 0，或模拟装配失败 | 记录未装配；快照不推进，`c1@2` 进 `pending` 或下次重新取到；也不额外启动一次推理 |
| 11:04，J | 下一次已有的观察机会，预算足够 | `changes_since / observe`，装配成功后推进；加入下面的修订片段，随后相同版本不机械重发 |
| 11:05，J | 在自己的推理事件 J/e9 中复述刚读过的要求 | 写入方提交 `cites=[c1@2]`，来源为 J/e9；SI 按预设计划识别同源，不增加独立证据或召回强度 |

```text
[Memory 已读认知修订｜c1@1 → c1@2]
你此前读到的 alpha 游戏页“默认静态”要求已被修订：允许缓慢移动，仍不能闪烁。
范围：u1 / snake-alpha / game_page。修订来源：A/e33。
显露原因：你此前读过 c1@1；当前 beta 话题不改变这条修订的适用范围。
```

J/e9 是新的推理记录，但不是新的独立事实来源。直接把 Runtime 注入的 Memory 片段当成用户事件写入，应被来源校验拒绝。若 J 已失去 alpha 的读取权限，修订提示也应被过滤；读过不是永久授权。

对照变体：J 在 11:01:40 把话题切回 alpha。此时 `query_topic` 返回 `c1@1` 并标为待复核，同时附带 `p3`（§4.3 的兜底与精确标记）；这一路径靠范围匹配，不靠 `read_set`。

### 3.13 场景八 暂缓必须到期处置，不反复唤醒整理

用可推进的测试时钟，配置观察窗口为 72 小时，仅作 fixture 参数。三条有来源的工具异常先由预设计划暂缓，分别缺验证结果、缺后续用途、缺可判明原因的上下文。

| 模拟时间 | 原始材料或动作 | 真实组件应产生的结果 |
|---|---|---|
| t0 | SI 读取 `pd1/pd2/pd3`，预设三条均需暂缓 | 一次提交三条 `deferred` 处置，带 reason、reevaluate_when、deadline；正文仍在待处理集合 |
| t0 + 1h | 只有 SI 自己的提交和运行记录变化 | 廉价待处理查询显示无新增/到期/待重试项；不再次调用预设深度整理，不把提交生成感知 |
| t0 + 24h | 同源 Report 又转述一次 pd1，形成新感知 `pd1-r`，无独立验证 | 廉价查询报“新增”，给一次运行机会；预设计划把 `pd1-r` 处置为 `duplicate`（reason 指向与 pd1 同源），pd1 保持 deferred、deadline 不变；不加强证据，原始来源和同源关系可追溯 |
| t0 + 72h | 到期，同时得到 pd1 的直接验证 fixture | 到期查询选中这些材料；预设 pd1 形成有条件经验、pd2 丢弃并说明理由、pd3 只保留带“原因未知”的来源线索 |
| 提交后 | 认知/线索与逐条处置成功，模拟首次清理失败 | 返回已提交、清理待重试；下次只重试清理，不重复生成认知或延长 deferred |

“只留来源线索”仍按 A.4.1 的处置契约表达：最小线索保留原始引用及未知条件，材料关联其承接记录后可清理，不新增一种永久待处理状态。查询路径发现到期材料不能只隐藏它；需留下可执行的到期处理结果或待处理项，由模拟宿主驱动兜底清扫。正式运行机会由 S 阶段的 Runtime 提供。

另加两个对照：一轮完整表达跨多条感知时保持同批读取；读取因预算截断时，只处置实际读到的材料。缺用户确认的暂缓项可提供低优先级澄清 Hint，由相关 UI 决定是否问，不由 SI 主动发消息，也不重复追问。

### 3.14 场景九 同名对象与相对时间不能靠猜

| 时间与 Session | history / fixture | 组件调用与预设整理 |
|---|---|---|
| 周一 09:00，UI K | 用户：“Bob 说周五交初稿。”上下文有两个不同联系人都叫 Bob | 原始提及保留“Bob”；组件附两个候选及依据，不在写感知时选人、合并或创建第三个对象 |
| 周一 09:01，K | 用户补充：“产品经理 Bob，不是邻居 Bob。” | 新感知引用独立原始事件；预设 SI 归属到产品经理对象，邻居对象保持独立 |
| 周三，SI | 才读取两条材料 | fixture 提供周一的 observed_at、原始“周五”、时区/区域锚点；预设解读为当周周五的绝对日期，不按周三重新计算 |
| 周三提交后，K | 按 Bob 名称查询、再按确切对象读详情 | 名称查询仍可返回两个候选；按对象/认知 ID 能读到本次来源与整理版本，不把截止日期当作任务调度数据 |

本例只验证字段保留、歧义候选和预设归属能正确提交；真实时间/指代理解属于 B-46 的 S/Q 验收。“周五交初稿”的正式安排仍归 Task/Goal，本例的 Memory 只记录值得复用的事件背景及来源，不负责提醒或确保交付。

合并用另一个独立 fixture：两条对象记录被原始证据确认是同一个人，由 SI 的 commit 合并；旧对象及别名入口重定向并保留修订链。不能因为两个联系人都叫 Bob 就合并。再加入有明确意图锚点的时区变体，确认传给预设整理的是意图锚点优先的材料；模型是否正确解读留到 Q。

## 4. 需要由例子确认的契约

### 4.1 感知与认知

- 感知沿用 `<sid>:<seq>` 身份，补 A.5 的主体、scope、suggested_kind、memory_intent、cites、锚点、occurred_at、原始提及/解析候选和原始事件引用；宿主提供身份、observed_at、Task/Goal 与权限，不由模型伪造。补写的 run_digest 标为补写，不能冒充原始观察。
- 认知沿用 A.3.4 的稳定 `item_id + revision`、Graph kind/claim，补 scope、basis、explicit、valid_until、review_when、produced_by；语义类别放 semantic_kind。多个 topic 入口引用同一认知，同一语义与适用范围内的关系三元组更新原逻辑条目；不同项目/版本范围不自动归并覆盖，旧 revision 可读取且内容固定。
- `MemoryObservation` 是长期证据观察，必须能经 `source_ref.event_ref` 回到原始事件；感知正文清理不删它。自由条目同样必须有依据与 write_reason，不能成为无来源摘要出口。
- Hint 显式标注 `perception` 或 `cognition`。未整理、暂定、待复核与已确认内容不得合并成无差别事实。
- 认知只由 SI 的提交变更；普通客户端新增、修改或撤回认知的调用被拒绝。纠正不直接改认知：查询或召回认知时，同主体、同范围下未整理的纠正类感知总会随之返回，不受是否请求近期感知影响；纠正附带认知引用时，组件派生“待复核”状态。
- 纠正兜底覆盖自动召回、主动查询、按 ID/主体/来源读取和缓存返回。预算不足时不能单独输出未带纠正提示的旧结论；未整理的派生待复核不改写 Graph `status`。预设“这次先不用”为普通观察，不触发纠正；真实分类质量属 Q。
- 用户显式要求记住和 Agent 主动记录共用写入路径；来源与记忆意图在整理中可追溯。写入成功后即可参与相关召回和变化通知，不以完成 SI 或创建 Notebook 为前提。
- 同一来源事件可以有多条感知，按 A.5 的 `idempotency_key` 去重：键要同时识别来源和本次观察，不能只按事件 ID。相同键、不同内容报冲突；正文清理后键仍保留，重放不会复活已清理材料。
- 召回引出的判断带 cites，不能把 Runtime Memory 附加块当新来源；同源不加强证据或召回强度。原始提及可暂不解析，歧义不自动合并，确认归属/合并只经 SI 提交。
- 保留策略和可见范围在持久化前确定；不记忆的输入不能先写后删。匹配、Hint、变化提示、别名候选与来源读取都先过 A.3.7 的可见性，再按范围匹配；对象重叠只说明相关，不代表可见。
- kind/status/alias_type/source type 按 A.3 校验；`disputed` 可带分歧显露，正式 stale/过期、superseded、deleted 不作为当前有效认知召回。非法状态迁移与旧事件复活须拒绝或提供明确的授权依据；路径段统一编码或校验。

### 4.2 set_topic 与半订阅

契约见附录 A.9：订阅状态归 Session，Memory 只按快照、范围和 `read_set` 返回变化，不保存订阅注册表，也不另建变化日志。本节只列例子必须确认的行为。

- 关注的是查询范围，不是本次命中的 ID，否则发现不了之后新增的相关感知。`query_topic` 先取快照再读内容，快照与后续增量之间不留空隙；重叠部分按引用加 revision 去重。
- 基础范围由宿主确定性建立，不参与淘汰；模型追加的 tag 随再次提及增强、随时间衰减、满额淘汰最低分，淘汰后不再进入查询范围。相同参数重复 `set_topic` 幂等；切换或清除时 topic revision 加一，旧范围的结果不混入。`read_set` 不因 topic 漂移或 tag 淘汰而丢失，返回前重新校验权限。
- 变化阀门决定是否再次检索：tag/标题变化大则深检索，只到达间隔则轻检索，否则不检索。阀门不阻断 `changes_since` 的增量和已读修订提示。使用假时钟和可配置阈值测试；8 个 tag、30 分钟、0.5 比例、5 轮只是评估起点，不是固定性能承诺，C 阶段不调用真实模型筛选。
- 除非显式关闭，每次推理前都调用 `changes_since`，包括同一次运行中工具返回后继续推理；无 Message input 的 Work 也有观察机会。半订阅不唤醒 Session、不额外触发 LLM；快照未变时走不打开 Graph、不取写锁的快速路径。
- 只有成功装配才推进快照、pending 和 `read_set`，且只覆盖实际送出的条目；未送出的下次仍能取到，重复内容不重复应用；一个 Session 的进度不影响另一个。确认只表示进入观察上下文，不证明要求已被理解或落实。
- Session 自己写的感知不推回自己；随后 SI 形成的认知仍按范围或 `read_set` 显露。Runtime 自动写的 `run_digest`、`task_outcome`、`task_discarded` 只进整理批次，不出现在其他 Session 的变化里。
- 匹配先过 A.3.7 的可见性，再看范围；相关子 Session 有各自的观察状态，父子关系不等于全部广播。同一 topic 下的多条独立感知不能用“只留最后一条”合并掉。
- 初次召回、后续变化和按引用读取都有预算与截断/续读信息。按类别分池，有效性变化优先，新感知和已有认知各保留额度；weight、confidence、basis 分开表达。明确提出的认知不计时间淡出，但仍受权限、范围、有效期及修订约束。
- 同一 Session、topic 阶段与同一内容版本避免重复显露；新 Session、新 topic 阶段或新版本可再次显露。快照进度与 Hint 显露去重不是同一概念。
- 对 topic 命中的 Session，待复核、撤回和替代都是有效性变化，原正文版本已经显露也要提示，不能按“这个 ID 读过了”抑制。`read_set` 只对整理提交的修订和撤回产生提示；整理前的纠正是查询/召回视图的派生提示，不冒充新版认知，也不经 `read_set` 提示话题已转走的 Session（§3.12）。
- 快照后新增、但已被清理的感知只返回处置和承接认知，不带旧正文；已进入上下文的旧内容通过修订提示纠正。变化过多或 pending 溢出时返回 `resync_required`，Session 用 `query_topic` 重建。
- Memory 的观察是尽力而为的线索：预算截断、整理清理都可能让某条感知没被看到。必须送达的内容（如运行中 Work 的修改）不走这条路径（Memory 需求 M-30、§9.4）。

### 4.3 Memory 整理 Goal 的提交与清理

- 读取有界感知批次，并按权限、范围读取相关认知及原始来源；本例用预设决策产生新增、合并、修订、撤回、丢弃或暂缓结果。
- 批次逐条记录 absorbed、duplicate、discarded 或 deferred；逐条处置是权威状态，字节游标只能用于扫描加速，不能跳过暂缓项。处理中新增材料不属于本批次；整轮表达尽量同批，截断读取不能处置未读部分。
- 整理计划保留原始来源关系、证据性质、反例及结论适用范围；Report/摘要转述同一事件不增加独立证据。暂缓结果标出缺失依据与重评条件，既无新信息、也未到达复核条件时不反复分析。
- 按 A.4.1/A.8 将 Graph operations、逐条 dispositions、幂等键、produced_by 与真实 actor_session_id 写入同一 occasion envelope，追加并 fsync 成功即提交。校验或追加前失败无可见变更；写入/fsync 期间失败时结果可能待恢复确认，须重开校验日志并按幂等键查询，不能直接再次追加。提交后派生文件/索引失败返回“已提交、待修复”。纯重复/丢弃批次允许 operations 为空。
- 清理删除 absorbed/duplicate/discarded 正文，保留最小处置、幂等键和来源信息；不等待全部 Session 读完。清理按 A.5 持文件级短锁重写文件，与该文件的追加互斥，`seq` 不倒退。deferred 保留至实际处置；来源经证据观察指回原始事件，而非仅指向将清理的感知。
- 认知已提交而清理失败时，只重试清理；同批提交重试返回已有结果。按 A.4.1 校验：已终局处置的材料再次处置、`expected_revision` 不符，都整条拒绝；重新读取并合并，不能用旧版覆盖新版。
- 明确纠正的模拟输入可以直接引用受影响认知，以验证提交前的失效窗口；从自然语言自动定位目标是后续能力，不由本例假装完成。
- 整理 Goal 产生认知变化通知，但不会把自己的提交日志重新记录成新的待整理感知；本轮无材料可以结束，长期维护职责仍存在。
- deferred 必须有原因、重评条件与窗口上限。到期实际形成认知、丢弃或只留来源线索；查询时到期检查与兜底清扫不能只有状态词汇。廉价查询区分新增、到期和待重试工作，排除整理自身提交；72 小时仅为可配置评估初值。

### 4.4 查询、可见版本与恢复

- 先按权限、scope、状态、有效期、纠正硬过滤，再选对象/对象对/一到两跳有向关系、tag、中文全文（含证据观察与自由条目）和未整理感知候选。`recall_hints` 保留 `load` 的完整分数，salience 仅供审计、不参与召回或全文索引。
- 无有效入口返回“未触发”，正常已查但未命中返回空结果；非空 tag 全非法返回参数错误。失败、锁超时、损坏、索引未就绪与部分结果必须可区分，失败不推进去重或观察进度。
- `memory_snapshot` 按 A.9 为版本向量（Graph seq 加各 Session 感知 seq），覆盖感知新增/吸收及认知提交、状态变化，读取不改变它；写感知不引入 Agent 级计数器或锁。验证仅新增感知也使缓存失效。
- “未变化”优化须同时匹配身份/权限、查询范围、版本和上下文代次；有未整理纠正不能按未变化省略。时间推进不一定改变 snapshot，缓存命中仍须检查 valid_until、deferred deadline 等时效条件。Fork、压缩或重建使上下文缓存失效；分页使用一致视图，不静默拼接不同版本。
- 主动查询、按逻辑 ID/固定 revision 读取、主体视图和 provenance 复用同一认知及来源。索引不可用时，按已知 ID 和来源读取仍可用；待整理数和最近整理时间先过权限过滤。
- 提交日志与处置是真相，索引和 SQLite 可删除重建。恢复不能改写真相、静默合并别名冲突或复活撤回内容；版本迁移与不支持版本的读写边界按 A.8/TD-12/TD-24 单列验证，不自动迁移或删除旧 Notebook 数据。

## 5. 实施 TODO

### T01 组件边界和最小数据模型

- [ ] 按 A.1/A.8 落定现有 Perception 与 Graph 的库级适配：单一感知流、单 Agent memory_root、Graph 提交日志和单写锁；保持依赖方向，不引入新服务或第二套 Memory。统一的 Memory 组件门面放在 `lib_opendan`（它已依赖 `agent_tool`，能同时访问两层）。列出 S 阶段 Notebook/attention signal 调用方与数据清点范围（TD-09/20）。
- [ ] 在组件边界区分普通写感知、持整理 lease 提交、显式管理操作；模拟 lease 由宿主绑定，提交校验身份/权限和 lease 有效性，普通客户端不能直改认知（TD-02）。正式 PATH 与真实 Session lease 接线留给 S。
- [ ] 按 A.3.1/A.5 扩展感知与 SourceRef：`idempotency_key`、事件引用、Task/Goal、主体与对象范围、cites、记忆意图、情境锚点、双时间、提及候选及补写标记；删除 Notebook 来源字段。写入回执可附现有认知引用，范围索引失败仅省略此可选提示（TD-05/16）。
- [ ] 按 A.5 加感知文件级短锁：driver 追加与整理清理互斥，清理以写新文件、fsync、rename 完成，保留 `seq`、幂等键与处置标记（TD-04）。
- [ ] 按 A.3.7 实现 `Scope`、对象路径的分段重叠、例外和可见性过滤；模拟宿主为每个 SimSession 绑定授权主体集合（TD-13 的范围部分）。
- [ ] 按 A.3.4 扩展 MemoryItem，区分 Graph kind 与 semantic_kind，保存稳定 item_id/revision、scope、basis、explicit、有效期、复核条件、整理版本；支持对象背景、关系和私有用语等语义，不另造人物档案副本（TD-12）。
- [ ] 明确长期证据观察与待清理感知的区别，所有认知（含 free）能经 evidence 追到原始事件；计划和回执返回真实生成 ID，不猜测 obs 编号，支持多个有证据的别名（TD-05/17）。
- [ ] 校验 kind/status/alias_type/source type、路径段和非法状态跃迁；对象合并级联别名并保留跳转。歧义返回候选与依据，不写入时自动合并、不把冲突当普通未命中（TD-06/08/11）。
- [ ] 固定 A.4.1 的 ConsolidationCommit envelope、逐条 disposition、提交校验（重复终局处置、`expected_revision`）和提交结果类型；区分未提交、待恢复确认、幂等已提交、已提交但派生物/清理待修复，为 T03 故障恢复提供明确边界（TD-03/04）。
- [ ] 按 A.9 实现 `MemorySnapshot` 版本向量、`query_topic` 先取快照再读的边界，以及 Session 侧 `ObservationState` 类型和持久化（TD-18）。
- [ ] 按 §3.10 和 A.9 列出普通客户端、宿主、SI 的最少逻辑接口和与现有方法的映射；保留库级测试入口，正式 tool/CLI 不在本阶段注册。按 A.8/TD-12/24 列明 schema 升级、历史字段缺省和读写策略，不扩展成旧 Notebook 产品兼容。

### T02 真实存储上的多 Session 例子

- [ ] 建立隔离目录、来源事件、绑定身份/权限、整理 lease、可控时钟及模拟客户端；初始化认知也走实际 SI commit。来源 fixture 可标为不可读，不能用生成摘要冒充原文。
- [ ] 先落实 TD-01/10：空入口为未触发，正常空查询为已召回但未命中，全非法 tag 是错误；锁超时、损坏、不可用与部分结果可见。失败不确认、不污染“已显示/未变化”缓存。
- [ ] 同时落实 TD-21：读路径不取写锁；`changes_since` 快照未变时只比较 `occasions.jsonl` 和各感知文件的末行 seq，不打开 Graph、不 replay。用 SI 写入期间多个 Session 连续观察的用例验证没有锁超时。
- [ ] 按 A.6.1 实现授权/范围/有效期/状态硬过滤、对象与对象对命中、一到两跳有向关系、tag 和实际中文全文候选；保留完整得分，排除 salience 审计项，限制查询输入与展开量（TD-13）。
- [ ] 合并同范围未整理感知；所有认知读取均带纠正兜底，有 cites 时派生待复核，无 cites 时也不能返回无提示旧结论；验证关闭普通近期感知、缓存和低预算场景（TD-14）。
- [ ] 实现类别分池和有效性变化优先；explicit 不随时间淡出但不豁免有效期与权限；输出 revision/scope/basis/weight/confidence、命中原因及展开建议（TD-07/13）。
- [ ] 在 `ObservationState` 上实现基础范围与动态 tag 分层、增强/衰减/淘汰、topic revision、轻/深检索变化阀门；用测试时钟验证阈值，不接真实模型语义筛选（TD-15）。
- [ ] 实现 `changes_since`：范围内变化加 `read_set` 的整理修订提示、自写感知不回推、运行摘要不做变化提示、已清理感知只给处置标记、`resync_required`；Session 侧成功装配后才推进快照、pending 与 `read_set`。覆盖 topic 漂移、整理前纠正不经 `read_set`、权限变化和重启（TD-18）。
- [ ] 提供主动 query、当前逻辑 ID/固定 revision、主体和来源视图；已知 ID 读取不依赖检索索引。缓存键覆盖两层版本、范围、权限和上下文代次，命中后仍复核到期与纠正，支持未变化提示与一致分页（B-23/24）。
- [ ] 将 §3.5～§3.9、§3.11—§3.14 的带时间 history 做成 fixture；trace 同时列原始事件、预设决策、真实返回、模型可见片段和模拟下一步，不硬编码 Hint/通知代替查询，不把模拟回复当模型质量证据。
- [ ] 运行显式“你记一下”、无值得写入内容、明确纠正/本次例外、引用回声、窄范围推断等对照。§3.11 的 Task 修订只用 fixture，分别验证关闭观察、禁止记忆和零预算；不能宣称真实修订送达已经实现。

### T03 模拟整理及可靠生命周期

- [ ] 先打通持 lease 的最小可靠提交，再扩展批次：一条 occasion 包含全部图操作和逐条处置，日志追加/fsync 是提交点，幂等键重试返回原结果。空 operations 的纯处置批次同样有效（TD-02/03）。
- [ ] 以 dispositions 计算待处理集合，deferred 不被整批字节游标跳过；新增材料不混入在处理批次，一轮完整表达整体读取，截断只允许处置已读部分。按 A.4.1 拒绝重复终局处置；模拟 lease 被接管后重叠批次的提交被拒（TD-03/16）。
- [ ] 让预设计划真实产生 `c1@1` 再修订为 `c1@2`，检查 `expected_revision` 冲突、同范围/语义的关系三元组逻辑 ID 稳定、跨范围不覆盖、历史 revision 固定、合法状态变化与旧事件不复活（TD-08/12）。
- [ ] 增加需求 §5.8 的输出目录混合批次：同源转述、独立复现、版本反例、一次明确约定、进度噪声和缺证异常分别处置；输出证据链、范围、入口、整理版本和暂缓原因。补充第三方主张与用户明确表达/行为推断分歧，保留 basis，不擅自采纳或扩大范围。
- [ ] 实现正文清理与独立重试：absorbed/duplicate/discarded 可清理，deferred 留存；不等待其他 Session 读完。清理持文件级短锁，与同一文件的追加并发时不丢记录、`seq` 不倒退、重放不复活。模拟日志已提交而索引物化/清理失败，读取或重开恢复已提交状态，重试不双写（TD-04）。
- [ ] 实现 §3.13 的廉价待处理查询、观察窗口和到期实际处置、查询时到期检查及兜底清扫；新材料/到期/重试才进入预设整理，排除 SI 自身提交。带 cites 的新判断仍逐条处置，纯复述可廉价判重，不加强同源证据/强度或反复深度分析（TD-19，B-44/45）。
- [ ] 提供低优先级澄清线索及重复控制；不主动打扰用户，不让需要保证正确性的澄清事项只存 Memory。候选结晶引用认知 revision，撤回/缩小范围时可反查或通知，由后续对应 Goal 处理资产。
- [ ] 保存真实 actor_session_id、来源事件和包装 occasion 的 parent_occasion；按对象/来源/整理版本能追踪受影响认知。来源不可读明确说明，证据摘录不能冒充仍可读取的原文（TD-05）。

### T04 组件验收和成本记录

- [ ] 将 §6 的 S01—S45 做成组件测试；补两个独立进程写入/观察同一 Agent 的用例，校验一写者、独立读进度与权限隔离，不以进程内单例替代共享状态（TD-26）。
- [ ] 故障点覆盖追加前、写入/fsync 期间结果未知、日志已提交但物化失败、清理失败、幂等重试、版本冲突、锁超时、坏行/digest 和观察装配失败；重开后区分完整提交与损坏日志，不静默修复真相或重复追加；验证状态、正文及提示，不只测试返回码。
- [ ] 删除/损坏派生索引与 SQLite 后，按已知 ID/来源仍可读，重建结果与提交日志一致；verify 只报告，repair 只重建派生物、不改写真相或静默合并歧义（TD-25）。
- [ ] 明确 compact 快照与 replay/归档清单的一致方案；固定 revision、撤回状态和未清理处置在恢复后不丢。若最小例子暂不依赖 compact，TD-23 仍列未完成，不以生成快照文件代替恢复验证。
- [ ] 按 TD-12/24 列明并验证版本升级：支持的旧 schema 显式迁移，不支持主版本拒写、次版本按 A.8 只读；迁移不能静默丢字段或覆盖旧认知。`meta.json` 现硬校验 `primary_language == "en"`、分词器为 `unicode61`，中文全文检索要连同这两项一起迁移。不触碰未经清点的 Notebook 历史数据。
- [ ] 记录感知写入、查询候选量/耗时、Hint 大小、分类预算、topic 阀门命中、`changes_since` 快速路径命中率、批次与提交日志增长及全量 replay 成本。全量物化重建追踪 TD-22（TD-21 已在 T02 完成）；建立基线后独立安排 P2 优化，不把未做的优化记为通过。
- [ ] 为示例提供实际可运行入口、完整输出、失败与恢复说明；列清 TD 组件部分、尚未接线部分和 P2 未完成项。实现完成时同步需求附录 A 的“现状”与附录 B 状态，本次计划更新不提前勾选。

实施先做 T01 的角色/数据边界和 T02 的错误分型，再打通 T03 的最小提交与恢复；随后完成 T02 的召回/观察和 T03 的整理生命周期，最后汇总 T04 验收。T 编号是工作包，不代表先实现全部查询再补正确性。

组件示例入口放在 `src/frame/lib_opendan/examples/memory_sessions.rs`：主流程第一步就要写感知，感知流只在 `lib_opendan`，`agent_tool` 访问不到。示例经 `FsAgentStateClient` / `AgentStateClient::perception()` 或 T01 的库级适配访问，不要求公开底层 `FsPerception`，也不让 `agent_tool` 反向依赖。拟议运行命令为在 `src/` 下运行 `cargo run -p libopendan --example memory_sessions`；现在不宣称可用。测试复用 Cargo、tempfile 和可控 fixture 时钟。

## 6. 验收清单

| 编号 | 场景 | 必须观察到的结果 |
|---|---|---|
| S01 | A 写感知，B 首次 set_topic | 同时得到认知和待整理感知的有限 Hint 及观察起点快照，B 的 `ObservationState` 已记录范围；对应 B-03/B-15 |
| S02 | B2 写相关感知，A 写无关感知 | B 只有相关变化；B2 自己写的感知不回推给自己，但能看到 SI 的整理结果；C 无跨项目内容；B 与 B2 各自推进进度；对应 B-02/B-09/B-22 |
| S03 | `query_topic` 取快照与读内容之间并发写入 | 写入出现在结果或后续增量中，不落入空隙；两边重叠时按引用加 revision 去重，不重复显露；若已整理，返回处置/当前认知，不承诺原始感知必达 |
| S04 | 重复设置、切换和清除动态 topic | 幂等设置不改变 topic revision；切换后旧 tag 不再进入查询范围，基础范围与 `read_set` 保留 |
| S05 | 同一事件重放、多条不同感知 | 同 `idempotency_key` 重放不重复写；同事件不同观察都可保留；同键不同内容报冲突；正文清理后重放不复活 |
| S06 | 多 Session 材料整理与清理 | 认知有范围、证据和多入口；清理后正文确实不在感知存储中，原始来源仍可追溯；对应 B-04/B-06/B-17 |
| S07 | SI 处理时新增感知，或暂缓部分材料 | 未纳入批次和暂缓的材料保持待处理，不被整批游标误吞 |
| S08 | 追加前失败、写入/fsync 期间未知、提交后物化/清理失败 | 追加前不丢材料；结果未知时恢复核查，不重复追加；已提交可单独修复，幂等重试返回同一结果；纯处置空 operations 同样有效；对应 B-07 |
| S09 | 重叠批次、旧版本更新、同三元组不同范围 | 已终局处置的材料再次处置、`expected_revision` 不符的提交整条被拒；重叠材料不重复吸收，旧 revision 不覆盖新版；不同项目/版本的认知互不覆盖 |
| S10 | 用户明确纠正但 SI 尚未完成 | 带引用派生待复核，无引用也附同范围纠正；关闭近期感知、缓存命中、按 ID 读或低预算均不能返回无提示旧结论；认知未改写，新版可再次显露；对应 B-12/B-18 |
| S11 | 延迟消费、重启、预算截断 | 已清理材料不复活；未装配的条目下次仍能取到；`ObservationState` 重启后恢复，Memory 侧无订阅需恢复；预算外内容进 pending 可续读，pending 溢出或变化过多时返回 `resync_required` 并重建 |
| S12 | 不记忆与不可见范围 | 不记忆输入未持久化；G（u2）与 B 对象范围重叠，仍看不到 u1 的 Hint、变化提示、吸收标记、别名候选或来源存在性；对应 B-13/B-24 |
| S13 | 锁超时、存储损坏与正常未命中 | 故障返回明确状态，正常未命中为空；失败不推进观察或缓存进度；索引不可用不冒充无记忆；对应 B-24/TD-01 |
| S14 | 弱观察、互相矛盾的来源和外部指令文本 | 预设计划可暂缓/保留分歧；输出保留来源与不确定性，不升级为用户确认或系统指令；对应 B-05/B-14 |
| S15 | 模拟任务弃用和世界对象换版 | 按来源定位依赖认知，预设 SI 提交必要修订/状态变化；普通客户端不直接改认知；独立经验可保留，Memory 不覆盖世界状态；对应 B-10/B-11/B-21 的 C 部分 |
| S16 | 新 Session 未主动想到已有要求 | §3.5 中 set_topic 只含当前任务与已知对象，不含“不闪烁”或目标记忆 ID；真实查询返回两层 Hint，history 中出现给定范围及来源的具体文本 |
| S17 | 半订阅与修订在 history 中的表现 | §3.6～§3.7 的新感知、吸收和旧认知失效均产生可见材料；没有正常观察机会时不自行追加推理回合；投递和确认元数据不挤入模型上下文 |
| S18 | 经验及选择线索推动下一步查询 | §3.8 提示核对字段而不推断用户采纳；§3.9 提供 selection 线索而不代替最新对象值；模拟动作可断言，真实 LLM 是否如此行动另行评估 |
| S19 | 显式“你记一下”与普通后续要求 | §3.5 两个变体共用感知写入、set_topic 和半订阅，不依赖 Notebook；持久化成功后确认，SI 前能召回，SI 后有认知承接，保留来源及记忆意图 |
| S20 | 不值得写入与值得保留的观察对照 | 按预设决策记录偏好/有用异常，进度与任务书仍留原对象；不要求每条输入产生感知；对应 B-26/B-32 |
| S21 | 多来源关系与同批不同处置 | 执行 Memory 需求 §5.8 预设计划，保留同源、独立证据及版本反例；项目约定与工具经验分开，缺证不强造结论；对应 B-28/B-29/B-30 |
| S22 | 暂缓后的下一轮无新增依据 | 宿主模拟下一次运行机会，材料仍存在但不重复进入预设深度分析；新证据或复核条件满足后可重新选择；对应 B-31 |
| S23 | 认知有效状态与证据性质分开 | 真实返回的 Hint 保留用户表达、工具经验或暂定推断的身份和条件；普通 topic 能召回，`active` 不抹去不确定性；对应 B-34 |
| S24 | 运行中 Work 的一次性修改 | 修订只在 Task 修订 fixture 中，Memory 没有对应感知，SI 无需处置；对应 B-26/B-36 |
| S25 | 修改中带长期信号 | 只为长期部分写一条项目范围感知，与修订互不依赖；后续同项目 Session 可召回，不形成 Task 范围的认知；对应 M-31 |
| S26 | 分别关闭 Memory 观察、禁止本次记忆 | 前者不注入但不阻止其他 Session 记忆，后者不写长期感知；fixture 中 Task 修订均不受影响；对应 B-38 的 C 边界验证 |
| S27 | 普通客户端或失效整理 lease 直接写认知 | 新增、修改、撤回被拒绝；认知只来自有效 SI 提交；管理入口明确分离；对应 B-41 的组件部分 |
| S28 | 召回预算为零 | 模拟 Session 的任务书与修订 fixture 仍然完整，只是缺少 Memory 线索；对应 B-40 |
| S29 | 召回回声与 SI 自身提交 | cites 与原始来源关系保留；不增加独立证据/召回强度，Runtime 附加块不能充当用户事件，SI 提交不构成新待处理材料；对应 B-44 |
| S30 | deferred 到期、完整表达与截断批次 | §3.13 三种到期处置实际落地，deferred 不永久积压；同轮材料整体读取，截断不处置未读项；对应 B-45/B-30 |
| S31 | 已读 c@1 后换 topic，c 被修订或撤回 | §3.12 中整理前的纠正不经 `read_set` 提示（反向断言），整理提交后仍提供有效性变化；预算/装配失败不推进，重启后续读；失去权限不泄露；对应 B-47 |
| S32 | 新感知涌入、旧明确边界与弱证据高强度并存 | 有效性变化优先、两层保留额度；explicit 不淡出但到期仍过滤；weight/confidence/basis 分开显露；对应 B-48 |
| S33 | 两个 Bob，以及确认同一对象后的合并 | 歧义返回候选与依据，未命中不自动建对象；SI 确认合并后旧对象/别名可追踪跳转；对应 B-50 |
| S34 | topic 持续追加及长时间闲置 | tag 不超上限，增强/衰减/淘汰和绑定关注清理实际生效，基础范围保留；深/轻/不检索按阀门触发；对应 B-51 |
| S35 | 相对时间锚点与“这次先不用”对照 | 保存 observed_at/occurred_at 与锚点，预设解读使用原始及意图锚点；预设普通观察不触发 correction；B-46/B-49 的真实判断留 S/Q |
| S36 | 空入口、全非法 tag、正常空查询 | 分别为未触发、参数错误、已查询但未命中；不返回全库前几条；中文/短语 tag 按 A.6.2 的字节长度和字符规则校验 |
| S37 | 感知独立变化、重复查询/Fork、仅时钟推进 | 两层 snapshot 与缓存正确失效；同范围同版本同上下文才可未变化，纠正不得省略；snapshot 不变时仍检查有效期/暂缓到期；分页不混版 |
| S38 | 当前逻辑 ID、固定 revision、主体/来源视图 | 指向同一认知，修订不改变旧版正文；证据观察可追原始事件，缺失来源如实说明，产出版本可检索；对应 B-23 |
| S39 | 状态与路径校验、旧事件重放 | 拒绝非法枚举/跃迁与逃逸路径；disputed 带分歧，stale/deleted/superseded 不作当前有效认知；旧事件不复活撤回内容 |
| S40 | 损坏派生物后重开/verify/repair | 已提交真相与处置不丢，已知 ID/来源读取可用；索引可重建，repair 不改日志或静默合并对象；与 S08 的提交故障一起验证 |
| S41 | 对象对、有向关系、中文全文与 salience | 有限展开与中文候选真实参与；recall_hints 保留完整分数和解释字段，salience 不进召回/FTS；证据失效被过滤；对应 TD-13/26 |
| S42 | 局部推断、第三方主张与业务对象分离 | 预设计划保留最窄范围和 basis，不无证据扩至 Agent 全局；模拟任务/授权逻辑不读取 Memory 内容作分支；B-42/B-43 的真实代码和模型验收仍在 S/Q |
| S43 | 清理与同一感知文件的追加并发 | 文件级短锁使两者互斥；追加记录不丢、`seq` 不倒退，已清理条目只剩处置标记；对应 B-06/B-07 |
| S44 | SI 写入期间多个 Session 每次推理前观察 | 快照未变时快速路径不打开 Graph、不取写锁；有变化时读取不因整理写入而锁超时；对应 B-22 |
| S45 | Runtime 自动写入 `run_digest`/`task_outcome` | 只进整理批次，可按引用读取，不出现在其他 Session 的变化里；对应 B-26 的组件部分与 M-31 |

S15 只用 fixture 改变外部对象状态并调用 Memory 组件验证，不实施真实任务回滚。B-01/B-08/B-25 中关于真实 Agent 自动记录、未主动检索时获益及新任务行为的验收，留给 Session 接入阶段。

S20—S23 检验组件能否承载和执行内容规则的预设结果；不能据此声称模型已会筛选、综合或遵循 Hint。Memory 需求 §10.3 的真实语义评估，以及 Fork/其它 Goal 接入验证，在组件稳定后进行。

S24—S28 验证组件边界；Task 修订的真实送达、确定性订阅和 Final Report 对账属于 Session 接入（§9.4、B-36—B-39）。S29—S42 补齐 v1.4/v1.5 机制，S43—S45 补齐 v1.6 的感知文件并发、观察成本与运行摘要边界；含自然语言判断的场景仍使用标注好的输入和预设计划，不宣称 B-46/B-49 的模型能力已实现。

### 6.1 与需求 §10.5 的 C 阶段验收映射

| C 阶段需求验收 | 本 TODO 的组件验证 |
|---|---|
| B-02—B-07、B-09、B-12、B-13 | S01—S10、S12—S14、S43，T04 的跨进程与故障测试 |
| B-15—B-18、B-20、B-22—B-24 | S01、S06、S10、S13、S16—S18、S21、S36、S38、S41、S44 |
| B-27—B-31、B-34 | S19、S21—S23、S29—S30；判断内容按预设计划 |
| B-44、B-45、B-47、B-48、B-50、B-51 | 分别为 S29、S30、S31、S32、S33、S34 |

其他 B 编号在表中只说明提前验证的组件部分或边界 fixture，不改变需求的首次验收阶段。S 阶段还需真实 Runner、来源与授权集成；Q 阶段复验真实模型筛选、综合及行为，不把本表当全需求完成清单。

## 7. 后续接入项

组件例子稳定后，再处理以下 S/Q 项；附录 B 的跨阶段 TD 保持未完成，直到这些部分也验收：

- 按 A.7 接入感知写入、topic、主动查询与正文读取；提供 `agent-memory commit --plan`，整理 lease/调用者校验和普通 Session 的只读 PATH，逐条图操作仅管理模式。CLI 返回真实 ID/状态，不让 behavior 猜 obs 编号；跨语言调用不自行计算 digest 或 replay（TD-02/03/05/17）。
- 修复感知写入对输入队列的依赖，Work 继续保持无 Message input；接入 Task 启动装配、UI 新输入与执行观察，将现有 Notebook 写入调用方切到统一 Memory 入口。
- 移除旧 Notebook 模块、公开类型、专属统计、工具注册、CLI 及模板引用；复用机制应已进入 Memory。清点旧数据并单独列明处理范围，不以移除代码为由自动删除历史数据。
- 将组件半订阅接到每次推理前的附加块装配，`ObservationState` 存入真实 Session 状态（架构文档 §11.6），覆盖同次运行工具返回、显式关闭、创建方建立基础范围、读过即关注、自写不回推、实际装配后确认。不得在运行中改 system prompt；开启 load_hints 测试，移除召回错误变空结果的路径（TD-01/13/15/18）。Work 保持无 Message input。
- 运行中 Work 的修改由 Session 架构实现为 Task 修订（Memory 需求 §9.4）：修订数据与授权、Runtime 建立的确定性订阅、必达送达、Final Report 记录最后观察修订编号，以及按编号把未处理修订交给新 Work。Memory 只接收 UI 从修改中另写的长期感知。
- Runtime 提供稳定来源事件、时区/区域及必要位置、Task/Goal、原始提及和补写标记；被待处理感知引用的事件不能提前清理。Q 阶段验证意图锚点优先与相对时间解读（TD-16、B-46）。
- 将整理接到维护型 Goal：廉价变化闸门后再给模型运行机会，按 dispositions 与 deadline 选材料，清理单独重试。终局成功必须有实际 commit 结果，不再只推进游标；真实整理测试断言认知与材料变化（TD-03/19）。
- 切换并退役 attention signal SQLite/CLI、旧 `self_improve_signals` 与 `self_improve_set_memory` behavior，只保留一条感知管线（TD-20）。事后回扫仍是待决补充来源，不在此默认实现；若采用，遵守原始历史、来源去重和成功后推进游标的规则。
- 补齐远程 Runner/kRPC、真实授权和 DID Object 映射：逻辑 ID 读当前版，revision 对象固定，感知不默认对外暴露。组件 fixture 不代表已经完成生产鉴权与任务失效联动。
- Q 阶段按需求 §10.3/§10.6 的标签集评估写入遗漏/噪声、召回回声、弱否定、同名归属、表达与行为分歧、窄范围综合及第三方主张；将预设计划替换为真实模型，保留“无内容值得记”和“不形成认知”的负例。
- 定义显式遗忘覆盖的受控副本范围并实现清除；本阶段清理已整理感知，不宣称已经删除 Session 历史、外部源或所有派生副本。

完成组件阶段的标准是：开发者可运行无 LLM 例子，看到感知、证据观察与认知的来源链、两层召回、topic/已读关注、可靠提交、暂缓到期及清理；§6 的 C 验收有真实组件证据，Task 修订等 fixture 清楚标注。正式 tool/Session 接入、模型质量和未完成 P2 优化分别跟踪，不能靠模拟输出宣称已完成整个 Memory 重构。
