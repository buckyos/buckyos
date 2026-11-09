# Agent Memory 召回接线 TODO

状态：2026-11-09 起草；经 review 补齐双层 Hint、英文 tag、遗忘、半订阅和状态持久化约束。§5 的 D1—D4 已于 2026-11-09 全部裁决。**2026-11-09 R0—R6 已实施（未提交）**，实施记录、实测数据与未做事项见 §8；本机 Zone 真实 Agent 的 DV（要重建 aios 镜像）未做。

本文是[捕获／保留／权重 TODO](agent-memory-capture-retain-weight-todo.md) 里 Q2 的展开，也是 [jarvis 默认 Goal TODO](jarvis-default-goal-session-todo.md) T02 第二条移出去的那份“Memory S 阶段 TODO”中关于召回的部分。写入侧（感知、整理、提交）已经接通，本文主要处理读取侧：真实 Session 怎样显露认知、待整理感知及其后续更新；写入侧同步统一 tag 的语言规则。

版本兼容不考虑：旧的 `recall_hints` 路径、旧 `Hint` 类型和对应 kRPC 方法直接删除。

## 1. 现状与实测

2026-11-09 的全链路回放（[pipeline.md](evidence/agent-memory-capture-retain-weight/2026-11-09/pipeline.md)）：三句话形成三条认知之后，新开会话连续两轮谈“项目 A 的周报”，topic 始终为空，背景里没有任何 Hint 行。回答正确是因为模型自己执行了 `agent-session recall '项目 A' '周报'`。

代码核对的结论（符号名为准）：

| 事实 | 位置 |
|---|---|
| 推理前召回只有一处，只传 `state.topic.tags`，最多 5 条，出错被 `unwrap_or_default()` 吞成空 | [drive.rs](../src/frame/lib_opendan/src/runner/drive.rs) 的 `recall_hints` 调用 |
| `state.topic` 只有一个写入点：UI 会话结构化回复里的 `topic` 字段 | [outcome.rs](../src/frame/lib_opendan/src/runner/outcome.rs) 的 `apply_ui_reply`、`is_ui_reply` |
| Jarvis 包里只有 chat_route、groupchat_route 的回复 schema 带 `topic`，两个 prompt 都写“默认 `null`，拿不准就留 `null`” | [chat_route.toml](../src/apps/jarvis_agent/agent/behaviors/chat_route.toml)、[groupchat_route.toml](../src/apps/jarvis_agent/agent/behaviors/groupchat_route.toml) |
| topic 随回复产生，下一批输入才生效：任何会话的第一轮都没有 Hint | 同上 |
| Work 模板 `load_hints = false`，topic 永远为空，从不召回 | [template.rs](../src/frame/lib_opendan/src/template.rs) 的 `builtin` |
| 旧路径不传授权主体和范围，召回不做可见性过滤 | [state/cognition.rs](../src/frame/lib_opendan/src/state/cognition.rs) 的 `FsCognition::recall_hints` |
| 新门面 `query_topic`、`query`、`changes_since`、`observe`、`set_topic`、`ObservationState` 在 `memory/` 之外没有调用方 | [memory/](../src/frame/lib_opendan/src/memory/) |
| 当前 Session 已序列化 `topic.title` 和 `topic.tags: Vec<String>`；tag 的分数、增强时间及 Memory 观察状态尚未接入 Session 持久化 | [protocol/state.rs](../src/frame/lib_opendan/src/protocol/state.rs) 的 `Topic`、`SessionState`；[session/live.rs](../src/frame/lib_opendan/src/session/live.rs) 的 `commit_state` |

旧路径缺少可见性过滤的后果是按代码推断的，没有复现：群聊会话的 tag 命中 owner 私聊里形成的认知时，这条认知会进入群聊会话的上下文。它违反需求 M-25，接线必须解决；语言、生命周期、预算和半订阅约束见 §3。

## 2. 旧文档的思路

读了需求文档 §6、A.3.7、A.6、A.9、附录 B，架构文档 §6.3.1、§12.8—§12.10、§15.5，组件模拟 TODO §8.5，jarvis TODO §1 与 §8.0。下面按“旧文档怎么想、今天的实现在哪、本文怎么办”逐条对照。

| # | 旧文档的思路 | 今天的实现 | 本文 |
|---|---|---|---|
| 1 | 召回入口是 Runtime 给的机会，用当前输入、近期话题、已解析对象去找，不要求模型先想到要查（M-18、§6.1“UI 新输入”一行） | 入口只有模型在上一轮回复里写的 tag | 沿用旧思路：来信文本本身就是入口（D1） |
| 2 | 新输入改变主题时，要在相关决策之前补齐观察，不能等下一条用户消息（架构 §12.10） | 晚一轮 | 沿用。第 1 条做到了，这条就满足 |
| 3 | topic 由低成本的旁路 LLM 维护（架构 §12.10）；阀门打开时才让模型做语义筛选（§6.6） | 主模型在回复里顺带写 topic | 不做旁路 LLM。来信文本补充本批入口；回复里的 topic 保留，tag 同时承担后续召回入口和持续半订阅匹配条件，直到遗忘或清空 |
| 4 | Work 的基础范围由创建方在创建时确定性建立，正确性不依赖模型设 topic（M-21） | Work 不召回 | 沿用。创建时已知的有 owner、工作区绑定、标题、目标、`session.scope` |
| 5 | topic 是有上限的工作记忆：tag 增强、衰减、淘汰，标题漂移算切换（§6.6、A.9） | 组件只在 `set_topic` 时执行衰减淘汰，未接入 Session；`state.topic` 只保存 `{title, tags}` | 每批独立衰减，显式再次提及才增强；完整状态持久化，对外 topic 从它派生（§3.3） |
| 6 | 订阅状态归 Session；只有成功装配后才推进快照、`pending` 和 `read_set`（A.9、M-21） | 组件有 `accept`；drive 没有接 | 沿用，并且不新建提交点：背景行已有“渲染进消息才算显示、随批次 receipt 提交”的机制（`background::settle`、`RECEIPT_BACKGROUND`），Memory 材料走同一条路 |
| 7 | 每次推理前都有观察机会，包括工具返回后继续推理（M-21） | 架构文档后来定为“半订阅只在输入里被使用，工具结果之后不注入，待定”（§12.8、§15.5） | 两份文档不一致，D2 定为按较新的架构文档：每个受控输入批次观察一次；M-21 的表述在 R6 改写 |
| 8 | 预算分池，有效性变化优先；未整理的感知以“待整理”身份显露（M-20、M-21） | 组件已实现（`Budget`、`HintLayer`、`HintState`）；旧路径只有认知、固定 5 条 | 双层默认显露（D3）；话题召回与增量观察共享同批总预算（§3.4） |
| 9 | 未触发、已召回、失败要区分，失败不能是空列表（§6.4、TD-01） | 组件已区分；drive 仍吞错 | 沿用 |
| 10 | 宿主为每个 Session 绑定授权主体，可见性先于范围匹配（A.3.7） | 写感知时已绑定（`memory_caller`）；召回没有 | 沿用，复用 `memory_caller` |
| 11 | 线索带范围、证据性质、revision、命中原因、是否建议展开（TD-13） | 2026-11-09 在旧 `Hint` 上加了限定语 `qualifier()` | 限定语搬到门面的 `Hint` 上，旧类型删除 |
| 12 | 主动查询有检索、按引用读、来源视图三种（§6.3） | `agent-session recall <tag>…` 走旧路径；`Memory::query`、`read_cognition`、`read_provenance` 没接 | 检索换接；按引用读和来源视图放到 R3 |

旧文档里有四处今天已不成立，实施时不要照抄：

- A.6.1 说 runner 传的 `state.topic.tags`“恒为空”，TD-15 说 `Topic` “没有任何写入点”。现在 UI 回复会写它。
- 需求文档多处把接线统称“S 阶段”。写入侧已在 jarvis TODO P0 完成，剩下的就是本文。
- 运行摘要类感知（`run_digest` 等）已删除，观察路径不用再考虑排除它们。
- TD-09 的 Notebook 仍未退役（`agent-session note` 还在），不属于本文。

旧文档没有说清、接线时会咬人的三处：

- **基础范围的主体必须包含 Agent 自己。** 范围匹配要求主体有交集（`Scope::matches`）。`memory_caller` 的 `default_subjects` 是对话方和工作区，Agent 自己只在 `grants` 里。基础范围若照搬 `default_subjects`，主语为 Agent 的经验类认知永远匹配不上。这和整理批次里已经修过的 `related_view` 是同一个问题。基础范围的主体取 `grants`。
- **会话范围里没有对象时，对象范围过滤不起作用。** 查询方对象为空就与任何对象重叠。UI 会话的基础范围只有主体，所以接线之后，“项目 A 的认知在谈项目 B 时被想起”在 UI 会话里照旧发生，只能靠限定语 `applies to …` 让模型自己判断。要真正过滤，得有人解析出“当前在谈哪个对象”，今天没有这个环节，本文不做。
- **对象写法不一致会造成漏召回。** 双方都有对象而写法对不上时，条目被过滤掉，不是少加几分。感知的 `--object`、认知的 `scope.objects`、会话的 `session.scope` 三处写法没有统一规则。所以 R2 先不把 `session.scope` 的对象放进 Work 的基础范围，只用主体和文本入口；对象入口等写法核对过再加。

## 3. 方案

### 3.1 入口

每个受控输入批次，先按 §3.3 衰减并淘汰 tag，再按下表决定是否做话题召回：

| 会话 | 文本入口 | tag | 基础范围 |
|---|---|---|---|
| UI | 当前 topic 标题 + 本批用户消息正文 | 衰减清理后仍有效的 tag | 主体：对话方、Agent、已绑定的工作区 |
| Work | 首批：标题 + 目标；之后只观察，要求重建时除外 | 无 | 主体：owner、Agent、已绑定的工作区 |
| Goal | 本文不改 | — | — |

UI 本批消息正文不写进 `ObservationState.topic`，只作用于这一次查询；同一 topic 阶段显露过的条目靠 `shown` 去重。UI 标题和新增／再次提及的 tag 仍由回复里的 `topic` 更新，遗忘不依赖回复是否带 topic。文本入口只补充当批检索，不能替代持久 tag 对后续变化的关注。

Work 由宿主在创建时将标题和目标初始化为 `ObservationState.topic.title`，持久保留供后续增量匹配和重建；它是宿主提供的工作主题，不依赖模型设置 tag。否则在既无 tag、又暂不使用对象入口的情况下，后续 `observe` 只能跟踪已读条目，无法发现相关新内容。

这样做和 §6.6 的变化阀门有出入：阀门的本意是不必每轮都查，而这里 UI 每批都查。UI 的输入是人的节奏，查询是本机扫描，成本可以接受；只有事件、没有消息正文的批次通常只走 `observe`，需要初始化观察快照或重建时仍调用 `query_topic`。

文本入口的准入门槛是“共有两个词”（`fts_enters`），是为短标题定的。整段消息做查询时误命中多少没有数据，R0 先测。

### 3.2 观察

topic tag 是半订阅匹配条件：只要 tag 尚未被淘汰，Session 在后续观察机会中就能发现与它匹配的新感知、新认知及相关更新，不需要再次设置 topic，也不要求之前读过该条认知。Memory 不保存订阅注册表、不主动推送或唤醒 Session；Session 凭持久化的范围、tag 和快照拉取变化。按 D2，观察机会是后续受控输入批次，不含单纯工具返回后的继续推理。送达受授权、相关性和预算约束，是尽力而为的线索。

话题召回之外，每批再调一次 `observe`：取 `pending`，再取快照之后的变化，与本批话题召回合并后按 §3.4 分配预算。变化包括：

- 读过的认知被整理 Goal 修订或撤回（不看 topic 是否还命中）。
- 可见范围内、命中当前 topic 的新认知，以及尚未读过的认知修订和状态变化。
- 可见范围内、命中当前 topic 的新感知，在整理之前就能被别的会话看到，并标为待整理。
- 未整理的纠正叠加在旧认知上（待复核、可能已被纠正）。
- 已显示感知被吸收、丢弃或清理后的处置标记。

tag 淘汰后不再凭它匹配新内容，也不再据它新增关注；已经读过的认知仍由 `read_set` 跟踪修订，已检出但未显示的材料仍按 `pending` 规则处理。标题、其它有效 tag 或本批正文独立命中同一材料时仍可召回，遗忘 tag 不等于禁止整个主题，更不删除底层记忆。`resync_required` 必须触发重建，不能因本批没有新正文或 topic 更新而一直跳过。

快照没变时 `changes_since` 只读日志和感知文件的末行，不打开 Graph。

### 3.3 状态

当前 `SessionState.topic` 已随 Session 提交原子写入 `<session>/.opendan_agent_session/state.json`，重新加载会恢复标题和 tag 字符串列表。它没有保存 tag 分数、时间戳或 Memory 观察进度；`ObservationState` 虽已有序列化能力，尚未接入真实 Session。接线后采用以下规则：

- 完整 `ObservationState` 存进 `SessionState`，首次使用时用基础范围初始化。保存 `enabled`、topic 的 `revision/title/base_scope`、每个 tag 的 `tag/score/touched_at_ms`，以及 `snapshot/pending/read_set/shown/last_query/context_generation/resync_required`；不能只保存对外 tag 列表。
- `state.topic` 保留为对外派生视图（别的会话在背景里看到的“conversation … is about”），不作为另一份独立状态。回复带 topic 时更新 `ObservationState`，衰减淘汰或显式清空时也同步回写标题和现存 tag。
- **语言规则。** chat session 的通用概念 tag 使用英文；人名、项目名和标识使用原名或规范写法，不强制翻译。标题和记忆正文可用对话语言。感知写入与认知整理采用同一概念 tag 规则，避免英文 Session tag 与中文记忆 tag 仅因语言不同而不匹配。现有 `normalize_tags` 只做格式归一和校验，不负责翻译，也不能用禁止非 ASCII 来实现名字例外。
- **遗忘规则。** 每个受控输入批次在召回／观察前独立计算衰减、淘汰低分 tag；回复为 `topic:null` 也执行。只有当前对话确实再次提及、由模型明确提交的 tag 才增强，不能把全部存量 tag 每批回灌 `set_topic`。召回命中、收到 Hint 或恢复状态不算再次提及。沿用可配置初值：容量 8，衰减时间常数 30 分钟，最低分 0.05；30 分钟不是固定过期时间。主题显著漂移时移除未再次提及的 tag；基础范围不参与遗忘。
- **恢复规则。** Session 重启后恢复原分数和 `touched_at_ms`，首次召回前按实际经过时间衰减，离线时间也计入；不能用加载时间重置增强时间。恢复原观察快照、待显示队列和已读版本，不重新初始化半订阅进度。
- 提交沿用背景行的做法：合并候选经 `settle` 后，按实际渲染出的行统一计算一次确认态；`pending` 包含话题召回与观察两路未显示的引用，`shown/read_set` 只记录实际显示的材料，快照只推进到已覆盖且未显示项已保留的位置。完整结果随批次 receipt 提交，恢复时由 receipt 重放。不能直接串联现有 `accept_recall`／`accept`：后者清空并替换 `pending`，会覆盖前者加入的截断项。渲染失败或批次没提交，状态不动。
- 上下文压缩、Fork、重建时调 `bump_context_generation`。

### 3.4 层次、预算与显露

自动召回和观察进入上下文的记忆材料统一包装为 `Hint`；Hint 是短线索，不等同于认知。门面通过 `layer/state` 区分认知、待整理感知和低优先级澄清问题，完整认知及来源按引用展开。默认同时启用认知和普通待整理感知；纠正和有效性变化优先处理。

预算按受控输入批次计算，不是两层固定比例。沿用 `Budget { max_items: 8, cognitions: 3, perceptions: 3 }`：8 是总上限，两个 3 是初始分配份额，不是各层硬上限，也不保证一定占满。先分配有效性变化，再分配认知、感知初始份额，剩余容量依次给认知、感知、澄清／标记。候选充足时，没有有效性变化可以是 5 条认知 + 3 条感知；有 2 条有效性变化时可以是 2 + 3 + 3；有效性变化达到 8 条时可以占满预算。

Work 的同批总预算是 4 条（D4），首批话题召回和之后的观察都用它，分配顺序与上面相同。初始份额先取认知 2、感知 1，R5 看实际显露再调。

`query_topic` 和 `observe` 当前各自调用 `Budget::allocate`，接线时必须合并候选、按引用及状态／revision 去重后统一分配同批预算，不能各取 8 条再拼接，也不能让普通话题召回先挤掉有效性变化。未显示的保留为 `pending`；只有实际显示的材料推进 `shown/read_set`。这需要调整现有调用或抽取候选阶段，不是直接串联两个已截断结果。

仍是 `<<background_context>>` 里的行，一条一行，key 和去重须区分引用、revision 与显示状态（纠正叠加或感知处置可能不改变认知 revision）。行文区分以下身份：

- 认知：`hint: <陈述> (<限定语>; noticed <时间>)`，与现在一致。
- 未整理的感知：标明“noted, not yet consolidated”和来源会话。
- 有效性变化：`revised` / `withdrawn` / `may have been corrected: <纠正原话>`。
- 澄清问题与感知处置：标明待确认的问题，或已吸收／丢弃及承接认知引用，不当作新的有效认知。
- 召回失败：一行 `memory unavailable: <原因>`，同时记入 worklog；不推进任何进度。

内建输入模板里的 `<hints>` 块同样改用门面输出。门面的 `Hint` 没有 `stability`，现在的 `short-lived` 标签要么补字段、要么去掉，实施时定。

### 3.5 主动查询

`agent-session recall` 改走 `Memory::query`，调用者身份由宿主绑定而不是命令行参数：位置参数当作文本和 tag，新增 `--object`；输出分开认知与待整理感知，带歧义别名候选和“还有 N 条待整理”。kRPC 的 `cognition.recall_hints` 换成对应的新方法，旧方法删除。

该路径必须显式设置 `include_recent_perceptions = true`，因为 `MemoryQuery` 的默认值为 `false`。未整理纠正即使关闭普通近期感知也随相关认知返回。主动查询的 `limit` 当前只限制认知分页，不是 §3.4 的自动显露总预算，不能把它描述为所有结果的共同上限。

## 4. 任务

- [x] **R0 文本入口的误命中测量。** 不调模型：用 14 个整理夹具回放后留下的认知库，加一组无关消息和相关消息，统计每条消息进入候选的条数、前 8 条里相关的占比。入口用来信文本已定（D1），本项只决定门槛：维持两个词、提高到三个、或按命中比例。
- [x] **R1 UI 会话换接。** 基础范围与 `Caller`（复用 `memory_caller`，主体取 `grants`）；完整 `ObservationState` 进 `SessionState` 并随 receipt 提交、恢复；每批独立衰减，`state.topic` 作为派生视图回写；drive 里的 `recall_hints` 换成 §3.1 + §3.2，处理初始化与 `resync_required`；话题召回和观察候选合并去重，统一分配预算；去掉 `unwrap_or_default()`；§3.4 的行格式，限定语搬到门面 `Hint`。组件和 runner 测试覆盖下表中的遗忘、订阅、预算、恢复与可见性约束。
- [x] **R2 Work 启动装配。** Work 模板打开召回（D4），宿主持久初始化标题和目标作为观察主题；首批召回，之后每批只 `observe`，需要重建时例外；同批总预算 4 条（§3.4）。测试：Work 一批显露不超过 4 条；Work 的第一条输入带相关认知；后续新增相关材料能被观察；与 Work 无关的 owner 认知不进来（依赖 R0 的门槛）。
- [x] **R3 主动查询与清理。** `agent-session recall` 与 kRPC 换接，显式开启普通待整理感知并说明认知分页边界；删除 `FsCognition::recall_hints`、`state::Hint`、`state::RecallQuery`；按引用读认知和来源视图（`agent-session recall --read <ref>` 或单独子命令，实施时定）。
- [x] **R4 提示词与手册。** chat_route、groupchat_route 的 prompt 与回复 schema 明确英文概念 tag、名字／标识例外及持续半订阅用途；去掉现有“默认 `null`，拿不准就留 `null`”的写法（D1 的补充手段），改为主题首次明确、显著漂移时更新，同主题内确实再次提及的 tag 可显式提交以增强，`null` 不增强旧 tag。感知写入手册 `ui_core.md` 与整理 prompt `memory_maintenance.toml` 同步概念 tag 规则，保持 `scope.objects` 的对象身份语义。说明 §3.4 各类行的读法（待整理的不是定论，纠正叠加时先澄清）；work_simple、plan_simple 加一句 Hint 行的读法。同步 `ui_core.md` 的 recall 说明，检查 `opendan_agent_dev.md` 里对 `background_context` 的描述。
- [ ] **R5 验收。**（真实模型回放已过；本机 Zone 真实 Agent 未走） 扩展 `memory_replay.rs` 的 `conversation_to_hint_with_the_real_model`，覆盖下表；状态和预算规则优先用确定性测试，语言生成和跨会话使用用真实模型回放。对应需求的 B-13、B-18、B-24、B-47、B-48 和 jarvis TODO 的 V03。最后在本机 Zone 的真实 Agent 上走一遍（需要重建 aios 镜像）。
- [x] **R6 文档回写。** 需求文档 §6.6、A.6.1、A.9、TD-01、TD-13、TD-15、TD-18 的规则与进度；按 D2 改写 M-21、TD-15 和 §11“召回机会与预算”一行里“每次推理前、含工具返回后”的表述，改为每个受控输入批次；按 D3 更新 §11.1“整理前广播”一行。架构文档 §6.3.1、§12.7 的同一表述，§15.5 “半订阅时机”一行的“待定”改为已定不做，“Topic 与成本控制”一行和 §12.10 写明旁路 LLM 当前不做、入口用来信文本（D1）；Session Input Protocol 的 hint 行格式；Agent State Protocol 的完整观察状态与 `cognition.*`；本文与两份上游 TODO。

验收场景：

| 场景 | 预期 |
|---|---|
| 新会话第一次询问已记住的周报要求 | 首轮背景已有相关 Hint，模型不执行 `recall` 也能答对 |
| 会话 A 说记住，会话 B 在整理前询问 | B 看到待整理感知和来源，不当作已确认认知 |
| 中文对话讨论通用概念，同时提到中文名字或精确标识 | Session、感知、认知的概念 tag 均为英文；名字／标识遵循例外；英文 tag 可匹配相关材料 |
| tag 不变，另一会话写入相关感知或整理产生此前未读的新认知；本会话后续输入不重复这些词 | 后续观察能发现新 Hint，无需再次设置 topic；同时覆盖只有事件、没有正文的输入批次 |
| 连续回复 `topic:null`，经过足够时间；或 Session 长时间离线后恢复 | 召回前淘汰低分 tag，不再仅凭该 tag 接收新内容；底层记忆保留，基础范围不受影响 |
| 当前对话明确再次提及某 tag；另有仅被召回命中的旧 tag | 前者增强，后者不续期；容量超限淘汰最低分，显著换题淘汰未再次提及的 tag |
| 已读认知所属 tag 被淘汰后，该认知被修订或撤回 | 仍经 `read_set` 收到修订；同版本同状态不重复显示 |
| 同批话题召回和增量观察都有充足候选、重复条目及有效性变化 | 合并去重后最多 8 条，有效性变化优先；层次份额可借用，未显示条目保留待送 |
| 序列化后重启；渲染失败；提交前后崩溃 | 完整 tag 分数／时间戳和观察进度恢复；只提交实际显示的状态，未显示的不丢，已显示的不重；恢复不重置遗忘时间 |
| 变化或待送队列超过组件上限 | 无需等待模型更新 topic，也能重建并继续观察 |
| 群聊 topic 命中 owner 私聊认知 | 可见性过滤阻止认知和来源进入群聊上下文 |

顺序是 R0 → R1 → R2 → R3 → R4 → R5 → R6。R1 完成后就能单独验收和提交。

## 5. 裁决

2026-11-09 全部已定：D3 在 review 时确定，D1、D2、D4 由用户按建议裁决。2026-11-09 已按此实施（§8）。

| # | 问题 | 结论 |
|---|---|---|
| D1 | 入口用什么：来信文本做机械入口；旁路 LLM 在主推理前提取 topic；只放宽 prompt 让模型多设 topic | 来信文本做机械入口。零模型成本，第一轮就生效，符合 §6.6“默认走机械召回”。放宽 prompt 作为补充一起做（R4）。旁路 LLM 当前不做，等 R0、R5 的数据说明机械入口不够再考虑 |
| D2 | 观察时机：每个受控输入批次；还是每次推理前，含工具返回之后 | 每个受控输入批次，与架构文档现行原则一致。Work 长时间自己跑工具时看不到中途的新感知，接受 |
| D3 | 未整理的感知默认是否显露给其它会话（需求 §11.1 记过“整理前广播”的风险） | 默认显露并标“待整理”和来源；授权与 topic 匹配后按同批预算送达。整理间隔是 30 分钟到 2 小时，不显露的话“在一个会话说了记住，换个会话问不到”。可见性过滤保证不跨主体 |
| D4 | Work 是否默认召回 | 默认开，常规话题召回只在首批执行，观察要求重建时例外；同批总预算 4 条，比 UI 的 8 条小 |

## 6. 不做

- 工具返回后的观察（D2）。
- 旁路 LLM 维护 topic（D1）。
- 解析“当前在谈哪个对象”，以及由此而来的 UI 会话对象范围过滤（§2 第二个要点）。
- Goal 会话的召回。
- Task 修订的必达通道（需求 §9.4），属于 Session 架构。
- `agent-memory` 只读命令在 Session shell 里不按授权过滤。Session shell 是否本来就能直接读 `<agent_root>/memory` 没有核对；能读的话命令层过滤没有意义，这件事和容器隔离一起考虑。
- Notebook 退役（TD-09）、增量物化（TD-22）。

## 7. 风险

- 文本入口的误命中（R0）。候选进来之后主要按 `weight` 和 `confidence` 排序，门槛太低时等于“把 owner 权重最高的几条每次都送进来”，这正是 TD-10 要避免的。
- UI 每批都做话题召回，每次读整张图（按日志长度和 inode 缓存）。当前规模没有问题，认知上千条后要看 TD-22。
- 背景行增多。默认自动显露的同批总预算 UI 为 8 条、Work 为 4 条，认知／感知的份额是初始分配，不能当作独立上限；实际条数在 R5 检查，过多就调小。
- 现有回放的感知和认知含 `周报` 等中文概念 tag，当前匹配没有跨语言翻译。必须同步 Session、感知和整理端的英文概念 tag 规则；R5 用新规则生成材料验证中文对话下的召回和持续更新。已有材料不会因改 prompt 自动重写，不能宣称旧中文 tag 已获得英文匹配能力。
- 文本入口不替代持久 tag。R5 必须包含后续输入不重复主题词的场景，否则即使半订阅没接好，也可能靠当批正文召回掩盖问题。
- 已有 Session 冻结了旧的 behavior，新 prompt 只对新一代 Session 生效。

## 8. 实施记录（2026-11-09，未提交）

### 8.1 落点

| 部分 | 位置 |
|---|---|
| 门面：同批召回与观察 | `memory/observation.rs` 的 `observe_batch`（`BatchEntry`、`BatchDelivery`）、`ObservationState::commit_batch`、`decay`、`reply_topic`；`memory/recall.rs` 抽出 `topic_pools`（`query_topic` 与批次共用，未截断的四个池）；`memory/changes.rs` 增加 `ChangeKind::Recalled` |
| 门面 Hint | `memory/hint.rs`：补 `stability` 字段（保留 `short-lived`）、`applies_to()`、`qualifier()`、`note()`（末尾带 `ref <引用>`）；旧 `state::Hint`、`state::RecallQuery`、`FsCognition::recall_hints` 删除 |
| 身份 | `memory::session_caller`（原 `runner/consolidation.rs::memory_caller` 的规则搬到门面，runner 与服务端共用） |
| runner | `runner/memory_batch.rs`（新）：`wanted`、`prepare`（衰减 → `observe_batch`）、`line`（行格式）、`settle`（按渲染文本确认）、`apply_reply_topic`；`drive.rs` 换掉旧调用；`live.rs` 的 `InputBatch.memory` 写入 receipt；`receipts.rs` 应用 `extra.memory` / `extra.topic`；`outcome.rs` 的回复 topic 走 `reply_topic`；`history.rs::compact` 调 `bump_context_generation`；`flush.rs` 把 `extra.memory_error` 写进 worklog 批次标记 |
| 状态与协议 | `SessionState.memory`（完整 `ObservationState`）；`RECEIPT_MEMORY` / `RECEIPT_TOPIC` / `RECEIPT_MEMORY_ERROR`；`WorklogBody::TurnStarted/InputBatch.memory_error`；`doc/opendan/protocol/schema/` 的 `session_state`、`worklog_entry` 两份 schema 已重新生成（其余 schema 与代码原本就有出入，没动） |
| 模板 | `template.rs`：Work 内建模板 `load_hints = true`；`assembler.rs`：`InputMaterial.hints` 改为行文本，内建模板不再另放 `<hints>`（线索都在背景里，UI 与 Work 一致） |
| 主动查询 | `state/cognition.rs` 的 `Cognition::query(sid, MemoryQuery)` / `read(sid, ref)`：按登记表找到 Session、读配置、绑定身份；kRPC `cognition.query` / `cognition.read`（`cognition.recall_hints` 删除，opendan 服务的只读列表、README、WebUI 的 Agent State 页同步：页面要先填 Session id）；`xagent_cli.rs::cmd_recall`：`recall <words or tags>... [--object] [--alias] [--limit] [--json]`、`recall --read <ref>`，只认 `$OPENDAN_SESSION_ID` |
| 提示词 | chat_route / groupchat_route 的 `topic` 说明与 schema、Memory 段的各类行读法；`ui_core.md` 的 recall 用法与 `--tag` 规则；`memory_maintenance.toml` 与提交工具 schema（`runner/consolidation.rs`，非 Jarvis 也生效）的 tag 规则；work_simple、plan_simple 各一句；analyze_intent 的 recall 用法；`opendan_agent_dev.md` 的 `background_context` 说明 |
| 测试 | `tests/memory_recall.rs`（11 个，见 §8.3）、`tests/memory_recall_entry.rs`（R0）、`receipts.rs` 与 `hint.rs` 的单测、`memory_component.rs::s13` 改写、`memory_replay.rs` 的真实模型用例重写 |

### 8.2 实施时定下的细节

- **`shown` 只在真正换题时清空。** 原 `set_topic` 在标题从空变为非空时也清空 `shown`：首批召回已经显示的线索，在回复第一次给出 topic 后会被下一批重复显示（runner 测试复现）。改为只在“原标题非空且漂移超过阈值”时清空。
- **同一材料两路都找到时，保留观察的身份。** 已读认知的修订既被观察（`ReadRevision`，有效性池）又被本批召回（`Recalled`，普通池）时，原先先到先得，修订失去优先级。现在先收观察与待送，召回里同键的条目跳过；输出顺序仍是待送 → 召回 → 变化。召回只按 `shown` 去重（换题后可再显示），观察按 `already_shown`（含 `read_set`）。
- **本 Session 自己写的感知不回到自己**（召回路径也排除，与 `changes_since` 一致）；自己写的纠正仍叠加在认知上。
- **线索行不走 `background_shown`**，只由观察状态去重；`memory unavailable` 一行走 `background_shown`（错误持续时只说一次），每批 worklog 标记仍记 `memory_error`。没有 AgentRoot 的远端驱动者因此每批都是 `memory unavailable`（它本来也不能写感知）。
- **基础范围的主体每批按当前身份刷新**（工作区绑定变了随之变）；Work 标题为 `title` 加换行加 `objective`（两者相同只取一次）。
- **一批的确认态只看渲染文本是否包含该行**：模板没放的线索留在 `pending`，下批优先；`pending` 超上限置 `resync_required`，下一批自己重建，不等新 topic。
- **kRPC 的 `session` 参数是调用方自报的**（与 `goals.*` 一样），服务端只保证“以某个 Session 的身份”而不是“以调用方声称的任意范围”读；登记表里找不到就报错，不做不过滤的读取。
- 主动查询的位置参数既当文本也当 tag（不是合法 tag 的词只当文本）；输出里认知与待整理感知分开，带“还有 N 条待整理”、歧义别名和 `--read` 提示。

### 8.3 测试与实测

- R0（`cargo test -p libopendan --test memory_recall_entry -- --nocapture`，报告 `target/memory-replay/recall_entry.md`，存档 `evidence/agent-memory-recall-wiring/2026-11-09/`）：用 11-09 整理回放报告里的 19 条有效认知建库，28 条手写消息（12 条相关、12 条无关、4 条近似）。UI 身份下“共有两个词”：相关认知 17 条召回 15 条，前 8 条精确率 79%，无关和近似消息 16 条一条都没进候选；“三个词”或“消息的五分之一”召回降到 9/17，“三分之一”降到 3/17。**结论：门槛维持两个词。** Work（多一个工作区主体）结果相同，近似消息里有 1 条误入。
- R1—R3 确定性测试（`tests/memory_recall.rs`）：遗忘（每批衰减、只有提及才增强、召回命中与恢复不续期、离线计时、容量与换题淘汰、底层记忆保留）；半订阅（另一会话写入的感知、整理出的新认知与处置标记，在不重复词、只有事件的批次里送达）；tag 淘汰后已读认知的修订仍送达且同版本不重复；同批合并去重最多 8 条、有效性变化优先、份额可借、溢出待送；只确认实际渲染的线索、待送超限后自动重建；群聊与他人会话看不到 owner 私聊的认知和感知；runner：新会话首轮有 Hint、第二轮不重复、观察状态与 receipt 一致；群聊会话无线索；Work 首批 ≤4 条且无关认知不进；Memory 读坏时出 `memory unavailable` 行与 worklog 记录、进度不动；`cognition.query/read` 与 `xagent recall` 按会话身份回答。另有 receipt 重放的单测。
- 全量 `cargo test -p libopendan --no-fail-fast`：新增失败为零。仍失败的是基线已知的 `fixtures.rs` 5 个与 `xagent_cli.rs` 2 个；`plan_simple_async`（6）与 `runtimes`（1）在全量并行时超时失败，单独串行（`--test-threads=1`）全部通过，属负载问题。`opendan` 包测试全过；WebUI `tsc -b` 通过（Playwright 用例未跑）。
- R5 真实模型回放（`cargo test -p libopendan --test memory_replay conversation_to_hint_with_the_real_model -- --ignored --nocapture`，93 秒，报告存档同上 `pipeline.md`）：
  - 感知 tag：`weekly-report`、`travel`、`report-generation`、`tool-x`、`linux` 为英文概念 tag，`项目 a`、`上海` 是名字（规则允许）；整理后的认知 tag 相同。
  - 整理前另一会话问周报：背景出现 `noted, not yet consolidated: … (from session ui-…; …)`，回答正确。
  - 整理后新会话第一句就问周报：首批背景有周报认知的 `hint:` 行，模型**没有执行 `recall`** 就答对（一页以内、明细放附件、只限项目 A）；回复的 topic 为 `项目 A 的周报要求` + `["项目 a", "weekly-report"]`。
  - 第一个会话追加“周报每周五中午前发”后，新会话下一句“还有别的需要注意的吗？”不含这些词，背景送来了这条待整理感知，回答用上了它并追问时区。
  - **误命中一例**：同一批还送来了 tool-x 恢复线索（标题“项目 A 的周报要求”加“还有别的需要注意的吗”与它共有“要求”“需要”一类通用二元组）。模型没有用它。R0 的近似消息没测到这种“标题 + 泛问句”的组合，见 §8.4。

### 8.4 未做与后续

- 本机 Zone 真实 Agent 的 DV（`./build_aios --local-test` 重建镜像后走一遍）未做；已有 Session 冻结的 behavior 不变，新 prompt 只对新会话生效。
- Work 的“后续批次观察”只有组件级测试；runner 级没有构造第二批（需要 tick 或任务结束）。
- 中文全文入口的通用二元组（“要求”“需要”“还有”等）会让泛问句命中无关认知（§8.3 一例）。可选做法：二元组停用表、按 IDF 加权，或要求至少一个非通用词命中；需要先扩 R0 的消息集再定。
- 对象入口、Goal 会话召回、旁路 LLM、工具返回后的观察：按 §6 不做。
- 已有回放材料里的中文概念 tag（如 `周报`）不会自动变成英文；新规则只影响新写入的感知与认知。
