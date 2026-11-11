# Agent State Protocol

版本 3 · 2026-10-11 · 由 `libopendan` 反写（`src/protocol/agent_state.rs`、`src/protocol/goal.rs`、`src/state/*`）

2026-10-11：增加 Goals（§9，`state/goals/`）、登记表 `goal_id`、查询 `goal_id`、kRPC `goals.*`。只是加法。

2026-11-02：登记表 `status.retry_not_before_ms`（可重试错误的退避期，Session Directory Protocol §7“run 的预算”）。只是加法。

2026-10-11：增加 Runtime 登记（§10，`state/runtimes/`，kRPC `runtimes.*`）；behavior 增加 `[tool]`（§7）。只是加法；Workspace 位置的 `runtime_id` 由 `local` 改称 `container`，旧记录照常读。

## 1. AgentRoot 布局

```text
<agent_root>/
  role.md self.md i18n/<lang>.md agent.toml                   包层（OpenDAN 维护，libopendan 只读）
  behaviors/<name>.toml  behaviors/*.inc  tools/ tool_plans/<name>.toml
  sessions/<sid>/                           session 目录的默认位置（可选）
  state/
    sessions/<sid>.json                     登记表（全部 session，无论目录在哪）
    sessions/.ids/                         身份键到短 session_id 的持久预留映射
    sessions/.unreachable/<sid>             巡检标记（目录不可达）
    perception/<sid>.jsonl                  感知
    perception/<sid>.jsonl.lock             感知文件短锁（追加与清理互斥）
    artifacts/<aid>/artifact.json           产物 head
    artifacts/<aid>/versions/v-<sid>.json   各 work session 的贡献
    deliveries.jsonl                        交付索引（§6.1）：哪个 Session 何时交付了哪个文件
    goals/<goal_id>/                        Goal（§9）：goal.json revisions/ changes.jsonl state.json
                                            activations.jsonl epochs/<n>/state.json schedule.json .lock
  memory/  notebook/                        认知（agent_tool 实现与其内部锁）
  state/workspaces/                         Known Workspace 登记（项目目录可在 AgentRoot 外）
  state/runtimes/<runtime_id>.json          Runtime 登记（§10）
  .locks/memory_consolidation.lease  .locks/artifact/<aid>.lease  .locks/workspaces.lock  .locks/runtimes.lock
```

访问经 `AgentStateClient`：文件版直接读写上述文件（要求能看到 AgentRoot）；kRPC 版与 DFS 版必须对外表现一致（同一组 fixtures）。

## 2. 登记表 `state/sessions/<sid>.json`

```jsonc
{ "session_id": "work-…", "kind": "work", "class": "work",
  "created_by": { "principal": "app:app2@alice", "via": "app" }, "idempotency_key": null, "route_key": null,
  "driver": { "principal": "app:app2@alice" },
  "location": "/abs/path/<sid>", "location_rev": 0,
  "input_queue": "app2::alice::opendan.session.<H(agent_did,sid)>", "wake_event": "/opendan/<agent_id>/session/<sid>/input",
  "agent_access": "full | status_only", "origin": null, "workspace": null, "artifact_id": null,
  "objective": "…", "title": "…",          // title 可缺省：session_config 的 session.title
  "scope": { "paths": [], "objects": [] },
  "goal_id": "g-…",                          // Goal Session 的 Goal，或创建 Work 的 Goal（session.goal / origin.goal）
  "runtime_id": "private",                   // Session 冻结到的 runtime（runtime.frozen.runtime_id）
  "runtime_unavailable": "…",                // 派生：Session 没结束而它的 runtime 现在不能用（停用 / 移除 / 到期 / 不应答）的原因；缺省省略
  "status": { "rev": 17, "run_state": "…", "outcome": null, "acceptance": "…", "one_line_status": "…",
              "report_brief": "≤500 字", "pending_decision": null,
              "needs_input": false,   // run_state = waiting 且只有别人的输入能推动它（没有在跟的 task / 子 Session，不是 Goal Session）；父据此区分“子在等回答”
              "turn_open": false,
              "paused": "user",       // 暂停着：不再推理，值是来源 user | parent | interrupted | goal（state.json `paused.source` 的投影，Session Control Protocol §5）；没有暂停时省略
              "retry_not_before_ms": 0,   // 可重试错误的退避期：此刻之前不推理（state.json `retry.not_before_ms` 的投影，缺省省略）；宿主据此不去驱动退避中的子 Session
              "activity": { "summary": "…", "touching": [], "heartbeat_ms": 0 },
              "last_runner": { "runner_id": "…", "host": "…", "pid": 0, "lock_epoch": 0, "at_ms": 0 },
              "last_error": null, "updated_at_ms": 0 } }
```

- **注册**：创建者用不覆盖发布写入一次；已存在且 location 相同或同一身份（幂等键 + 创建者 + 驱动者 + kind）→ 返回已有条目；否则 SessionIdConflict。
- **回报**：驱动者持 session 锁、在每次提交之后写 `status`（整条原子替换）；`status.rev ≤` 已有值时忽略。回报失败由下次 drive 按 `state.reported_rev` 补发。`status` 只是缓存，真相是 session 的 state.json。
- **巡检**：location 不存在时只写 `.unreachable/<sid>` 标记，不删除、不改写条目（保持单写者）；驱动者下次回报时清除。
- **runtime 不可用标记**：宿主（`mark_runtime(runtime_id, reason | null)`）对冻结在某个 runtime 上、还没结束的 Session 写 `.runtime_unavailable/<sid>`（内容是原因），runtime 恢复可用时删除；同样不改写条目，读条目时合成 `runtime_unavailable`。Session 不会被迁到别的 runtime。
- **删除**：`delete(sid)`（kRPC `sessions.delete {sid}` → `{deleted: [sid…]}`）只接受已结束的 Work Session，且它的全部后代（`origin.parent_session` 逐层向下）都已结束；连同后代一起删除 Session 目录和登记项，清掉 `.unreachable/<sid>`、`.runtime_unavailable/<sid>` 标记。有未结束的后代、或是 UI / Goal Session 时拒绝。`state/perception/<sid>.jsonl` 不动（Memory 的来源）；用量统计随目录一起减少。
- **子 Session**：`children_of(parents)` = `origin.parent_session` 属于 `parents` 的条目（按 session_id 排序），是查询的一个派生视图。父对子的隐式关注、同步等待的 resolver 与子 Session 的托管进程都只用它和 `lookup`。
- **查询**是派生视图（扫描 + mtime 缓存），可以另建可删除重建的索引。`RegistryQuery.goal_id` 取出一个 Goal 的全部 Goal Session（各 epoch）与它派生的 Work；`RegistryQuery.runtime_id` 取出冻结在一个 runtime 上的 Session（别名按同一个 runtime 算）。

## 3. 活动 Session 视图

- 活跃 = `run_state ∈ {running, waiting}`。`running` 且 `max(heartbeat_ms, updated_at_ms)` 超过 5 分钟 → 标为“可能已中断”；`waiting` 不要求心跳。
- 声明的引用 = `scope.paths` + `scope.objects` + `status.activity.touching[].ref` + `artifact:<aid>`。两个引用相等或一方是另一方的路径前缀（`ws:snake/src/` 与 `ws:snake/src/collision.js`）即有交集。
- 关系排序：同一 workspace 或同一产物（`same_target`）> touching 有交集（`overlap`）> 其它；`agent_access = status_only` 的条目只给 one_line_status。
- activity 来源：创建时 scope（bootstrap 输入批次写入）、`control(activity)`、runner 从写类工具参数推断（`write_file` / `edit_file` 的 path）；run 结束清空 touching，finished 清空整个 activity。
- 它是**避让提示**，不是锁；冲突裁决属于 workspace。

## 4. 感知

```jsonc
// state/perception/<sid>.jsonl，单写者 = 该 session 的驱动者（或组件阶段的模拟 Session），seq 严格递增
{"seq":31,"at_ms":0,"session_id":"…","kind":"observation",   // 旧文件可能还有 run_digest|task_outcome|task_discarded，只读、不作材料
 "source":"session","tags":[],"objects":[],"summary":"…","payload":{…},"refs":{"worklog_seq":316},
 // Memory 需求 A.5 的扩展字段（缺省省略）
 "idempotency_key":"A/e31/observation-1","content_digest":"…",
 "scope":{"subjects":["user:u1"],"objects":["project:snake-alpha/game_page"],"exceptions":[]},
 "suggested_kind":"preference|correction|…","memory_intent":"explicit","cites":["item_c1@1"],
 "anchors":{"timezone":"Asia/Shanghai","locale":"zh-CN","intent_timezone":"Asia/Tokyo"},
 "occurred_at":"…","source_ref":{"type":"session_event","session_id":"ui-a","event_ref":"A/e31","actor_kind":"user"},
 "mentions":[{"text":"Bob","candidates":[{"object_id":"obj_…","canonical_name":"…","basis":"alias:Bob"}]}],
 "backfilled":false,
 "cleared":{"outcome":"absorbed","occasion_id":"occ_…","cognition_refs":["item_c1@1"],"at_ms":0}}
```

- 追加幂等：跳过 `seq ≤` 文件最后一条的记录（反向读最后一行得到）；带 `idempotency_key` 的记录按键去重：同键同内容（`content_digest`）是重放，同键不同内容报冲突。正文清理后键与摘要仍在，重放不会让材料复活。
- 文件级短锁 `state/perception/<sid>.jsonl.lock`：追加（driver 仍须持 Session lease）与整理清理都在短锁内进行。清理以“写新文件 → fsync → rename”替换原文件，只把已终局处置记录的正文换成 `cleared` 标记，保留 `seq`、键、摘要、`scope`、`source_ref` 与 `<sid>:<seq>` 身份；`seq` 不变。读取不取锁，忽略未以换行结束的尾部。
- 可见性：`scope.subjects` 由宿主按写入 Session 的绑定补齐（模型只能在授权主体集合内收窄）；读者的每个授权主体集合必须包含记录的全部主体才可见。
- 没有自动记录（2026-11-08 起）：Runtime 不再在 run 结束、finished 或 discard 时写 `run_digest` / `task_outcome` / `task_discarded`，进度由 Task / Goal 承载；感知只来自 `ctl perceive`（`perception` 类型输入并入 `observation`）。`state.perception_seq` 由驱动者在写入后推进，不再预留与补发，seq 允许有缺口。旧文件中残留的这三种 kind 仍可按引用读取，但不进入整理批次、不出现在任何 Session 的变化里，也不触发整理 Goal。
- **待处理集合**：由 Memory 提交日志中的逐条处置计算（未处置 = 新增；`deferred` 且到期 = 待处置；已终局但正文未清 = 待重试清理），不按字节游标跳过暂缓材料。见 Memory 组件 `pending_work` / `begin`。
- **写入口**：Session 的 `ctl perceive` 控制输入由驱动者经 `Memory::record_perception` 写入：驱动者分配 `seq`、按 Session 绑定主体（UI 取对话 peer，Work 取 Agent 的 owner，Goal Session 取 Goal 的 owner，有 Workspace 绑定时附 `workspace:<id>`，grants 另含 Agent 自身）、绑定来源事件（`--source` 给出的本 Session 输入 key，否则这条控制输入本身、`actor_kind = agent`）与时区锚点，控制输入的 key 是幂等键。Goal 的 `extensions.opendan.records_perception = false`（整理 Goal）时该输入被拒绝（`input_rejected`），Runtime 记录也不写。
- **防回声**：Goal 带 `memory_consolidation` 能力的 Goal Session（各 epoch）的记录不成为材料；没有字节游标。

## 5. 认知（边界）

Memory Graph（`memory/`，schema 4.0：认知另带 `stability = stable | contextual | volatile`、`last_evidence_at`（独立证据时间，新建整理认知必填，修订不续期）、`review_at`（结构化复核日期）；`created_at` 是 `first_occasion` 时间的读取投影；召回淡出按稳定度 365 / 30 / 1 天一分、上限 20 分，`explicit` 不淡出；旧主版本根目录拒绝打开，不迁移）由 `agent_tool::agent_memory` 实现：`.meta/occasions.jsonl` 是追加写的提交真相源，一次整理提交是一条 occasion（图操作 + 逐条处置 + 幂等键 + `produced_by` + 真实 `actor_session_id` + 整理 lease 的 epoch）；读路径不取写锁。两层的统一门面是 `libopendan::memory::Memory`（感知写入、`query_topic` / `query` / 按引用读取、`changes_since`、`pending_work` / `due_reviews`、整理与清理）；topic 与观察进度（`ObservationState`）归 Session 保存，Memory 不登记订阅。认知只在 `memory_consolidation` lease 下经 `Consolidator::commit` 改变：该 lease 由 Goal 带 `memory_consolidation` 能力的 Goal Session 的 drive 全程持有，其宿主工具 `memory_consolidation_begin / commit` 是唯一写路径（§9.7）；普通 Session 只能写感知和读取，`agent-memory` CLI 在 Session shell 内拒绝写。

Session 侧的观察状态（2026-11-09，[召回接线 TODO](../../../notepads/agent-memory-recall-wiring-todo.md)）存在 Session 的 state.json `memory`：topic 标题、每个 tag 的分数与最近提及时间（只有回复里再次提及才增强，每个输入批次先按时间衰减，离线时间也算）、基础范围（Session 的授权主体，含 Agent 自身）、Memory 快照、`pending`、`read_set`、`shown`；随输入批次 receipt 提交（Session Input Protocol §7）。`state.topic` 是它派生出来给其它 Session 看的视图。

主动查询以 Session 的身份进行：`cognition.query {session, query: MemoryQuery}` → `QueryResult`（`cognitions` 与 `recent_perceptions` 分开，带纠正、歧义别名、`pending_in_scope`；`limit` 只分页认知），`cognition.read {session, reference}` → `MemoryRead`（`item_x[@N]`：认知、当前版本与来源视图；`<sid>:<seq>`：感知）。服务端按登记表找到该 Session、读它的配置、用与写感知相同的规则绑定授权主体（`memory::session_caller`），调用参数不能扩大可见范围；找不到 Session 时报错，不做不过滤的读取，存储不可读时报错而不是空结果。`agent-session recall <words or tags>... [--object …] [--read <ref>]` 走这两个方法（Session 由宿主设置的 `$OPENDAN_SESSION_ID` 给出）。旧的 `cognition.recall_hints` 与 `state::Hint` 已删除。门面 `Hint` 带 `layer / reference / revision / state / basis / explicit / confidence / stability / valid_until / scope / corrections / reasons` 等，注入时渲染为一句限定语（`Hint::qualifier()`，见 Session Input Protocol §6.5），推断不显示成用户原话、局部经验带着范围、短期线索带着期限；`notebook_append` 与 `notebook/` 是待退役的旧调用方，S 阶段移除（`attention_signals/` 已于 2026-11-01 退役，旧目录不再被读写）。非 Rust 实现通过 `agent_tool agent-memory …` CLI 访问 Graph。

## 6. 产物列表（只登记与指向）

```jsonc
// artifact.json
{ "aid": "snake-game", "workspace": { "workspace_id": "ws-…", "access": "read_write" }, "head": "v-work-A", "rev": 3, "updated_at_ms": 0 }
// versions/v-<sid>.json
{ "ver": "v-work-B", "session": "work-B", "base": "v-work-A", "state": "produced|accepted|discarded",
  "outputs": ["report.md", "snake.js"], "workspace_ref": null,
  "side_effects": [ { "call_id": "c-1", "tool": "exec", "note": "exec cannot be undone by the session" } ], "updated_at_ms": 0 }
```

- work session finished 时由驱动者（持 session 锁）登记自己的版本：`base` = 当时的 head（新工作从用户接受过的版本继续），`state = produced`；`artifact.json` 不存在时以不覆盖发布创建（head = null）。已 accepted / discarded 的版本不再被改回 produced。
- **decide**（驱动者在 drive 开头应用 `control(decide)`）：持 `artifact:<aid>` 锁，**在锁内重读** head 与版本：
  - accept：版本 → accepted，head → 该版本（discarded 的版本不能 accept）。
  - discard：版本 → discarded；只有 head 正指向它时才回退到 `nearest_valid_base`（沿 base 链找仍 accepted 的祖先，没有则 null；base 缺失或成环报错）；head 指向其它版本时保持不变。
  - 拿不到 artifact 锁：control 留在队列，下次再试。
- discard 的报告写入 `state.result.discard_report`：`workspace` = `none`（无 workspace）或 `unsupported`（workspace 回滚另行设计），`unsupported[]` 逐项列出不可撤销的副作用（worklog 中 effect 非 read_only 的动作），`head_moved_to` 记录 head 结果。
- 顺序：decide 条目先进 worklog，state.json 提交（acceptance 变化），之后追加 `task_discarded` 感知。

### 6.1 交付索引 `state/deliveries.jsonl`

和上面的产物列表是两件事：产物列表记 Work 结果的版本与采纳，交付索引记交给 owner 的文件（报告的 `artifacts`、结构化回复的 `artifacts`，经宿主的 `ObjectStore` 存进 Agent 的 `artifacts` 桶之后）。只追加，一行一条，按交付顺序；行不改写、不删除，读的一方跳过解析不了的行。

```jsonc
{ "at_ms": 0, "name": "answer.txt", "obj_id": "cyfile:…", "url": "obj://bucket/user/<agent_id>/<owner>/artifacts/<key>",
  "mime": "text/plain", "size": 7, "session_id": "work-B", "ui_session_id": "ui-A", "route_key": "<agent did>/dm%3A…",
  "source": "report" }
```

- 写者是交付它的 Session 的驱动者，在对象存好之后写（`runner/reports.rs` 的 `store_artifacts`）。同一个对象（`obj_id`）由同一个 Session 再次交付不追加。写不进去只记日志，不算交付失败。
- `url` 是留住这个对象的桶绑定；按对象 id 直接转交的产物（没有桶留住它）没有 `url`。`mime` 按名字的扩展名推出，认不出就没有；`size` 只有工作目录里的文件有。
- `ui_session_id` / `route_key`：沿 `origin.parent_session` 往上找到的第一个 UI Session 和它的 `route_key`（UI Session 自己交付时就是它自己）。同一个对话的各代 UI Session `route_key` 相同，“当前会话的产物”按它过滤。上面没有 UI Session 的（Goal 派出的 Work）两项都没有。
- `source`：`report`（报告的产物）或 `reply`（回复的附件）。
- 没有对象存储的宿主（`--dev`）不写。回复正文里 `<attachment>` 标签带的文件目前不进索引（它在发送回复时才变成对象，那里没有写索引）。
- 读：opendan 服务的 `deliveries.recent {limit?, before?, route_key?}` → `{entries, next_before?}`，新的在前；`before` 是上一页的 `next_before`（行的位置，不是时间）。

## 7. behavior 目录（BehaviorCatalog）

behavior 是 Agent 的配置，由 Agent State 提供（`AgentStateClient::behaviors()`：`identity / get / list / revision`），Session 在构造时冻结（[Session Directory Protocol](<Session Directory Protocol.md>) §3 `prompt.frozen`）。文件版读 `<agent_root>/behaviors/<name>.toml`（文本里的 `__INCLUDE(<相对路径>)__` 相对所在文件展开，最多 8 层）、`role.md`、`self.md`、`i18n/<lang>.md`；`revision` 是这些文件 `路径|长度|mtime` 的 sha256。schema：`schema/behavior_config.schema.json`。

```toml
[meta]
objective = "…"
next = ["do", "review"]            # 可切换 / 可调用的目标：冻结闭包

[prompt]
mode = "behavior"                  # function_call | behavior；缺省用会话的 loop_model
system = "… {{ session.id }} …"    # system 模板；变量：identity.* behavior.{name,objective,mode} session.{id,kind,objective,driver,scope,parent} workspace.id
on_init = "…"                      # 三类受控输入的模板（缺省内建）；变量 background_context 是本批变化的背景行，不引用则半订阅不可见
on_input = "…"
on_context_switch = "…"

[input]                            # 消费策略（缺省用会话的）
mode = "batch"                     # single | batch
media = "reference"                # reference | inline

[capabilities]
tool_whitelist = ["group:bash", "behavior:analyze_intent", "report"]   # → tools.tools；`group:<内置工具组>` → {groupname}，其余 → {name}（运行环境提供的工具）；`behavior:<name>` 是该 behavior 的 [tool] 函数
action_whitelist = ["shell"]       # → tools.actions
call_intent = false                # true（只用于 function_call）：每个原生调用必须带非空 `intent`，宿主加入 schema、执行前校验、交给工具前去掉
workspace = "read_only"            # 以它为入口的 Session 对 Workspace 的访问（缺省：创建者给的）；只读的 Session 不取写者租约、不阻挡写它的子 Session

[budget]
max_tool_iterations = 16           # → max_tool_iterations
max_completion_tokens = 4096       # → max_tokens
max_total_tokens = 200000          # → run 的 budget
max_wallclock_ms = 600000
max_consecutive_errors = 3

[model]
preferred = "…"                    # → model
fallbacks = []
temperature = 0.2

[entry]                            # 进入本 behavior 的方式，由目标决定
mode = "switch_context"            # switch_context | create_sub_context | fork；仅 Session 的入口 behavior 可省略
inherit = "none"                   # none | recent_dialogue | steps

[tool]                             # 可选：把本 behavior（create_sub_context / fork）作为具名函数给调用方
name = "try_analyze_user_intent"   # 字母、数字、_；缺省 = behavior 名；同一 Session 内唯一
description = "…"                  # 必填：什么时候调用
task = "{{args.request}}"          # 子 context 的 task 模板，只能引用 args.<name>
[tool.parameters]                  # JSON Schema，type = object；缺省 {task: string}
```

`[tool]` 由调用方在 `tool_whitelist` 写 `behavior:<name>` 时提供给它的模型（目标必须在调用方的 `next` 闭包内）；内部协议同 `call_behavior`（[Session Directory Protocol](<Session Directory Protocol.md>) §6）。fork 目标的 `[budget]` 只作用于分支的 run，不进 `.llm_context`。

`[model] / [capabilities] / [budget] / prompt.mode` 叠加为该 behavior 的 run 的有效 `.llm_context`（只产生 `model / loop_model / max_tool_iterations / max_tokens / tools` 这几个键），结果进 run.json `config`，xllm 接手时不需要理解 behavior。`prompt.llm_context` 里的 `prompt / groups / runs_dir` 在宿主装配下无效，冻结时拒绝。

Session 模板的覆盖在 `agent.toml`：`[session.<class>]` 的 `turns = "one" | "unbounded" | <n>`、`wait_user_msg`（`allowed | finish_failed | finish_completed | end_turn`）、`input = "none" | "queue"`、`tick_secs`、`observe`、`load_hints`、`default_behavior`、`max_process_depth`、`max_sub_sessions`、`max_session_depth`；内建 class 为 `work / ui / goal`，其它 class 必须在这里声明（self_check 已于 2026-11-11 退役）。`goal` 类另有 `min_interval_secs`、`guarantee_secs`、`check_every_secs`、`[session.goal.budget.activation|period]`、`[session.goal.agent_budget]`（§9.4）、`[session.goal.maintenance] enabled / behavior`（§9.7）与 `[session.goal.crystallize] enabled / behavior`（§9.8）。

定位（不是协议）：`connect(agent_did, who)` 依次取进程内注册的实现、`$OPENDAN_AGENT_ROOT` 或 `~/.opendan/agents.toml`（`"<did>" = "<agent_root>"`）指向的 AgentRoot、`$OPENDAN_AGENT_STATE_URL` 的 kRPC 服务（服务端随 OpenDAN 改造提供）。

## 8. kRPC 形态

看不到 AgentRoot 的调用方（容器或远端沙箱里的层 ② 工具、WebUI、其它应用）经托管该 Agent 的 OpenDAN 进程访问 Agent State。WebUI 在 Agent 自己的域名使用 `/api/opendan`，由网关按应用路由并检查 SSO 登录；跨应用调用经 zone 网关使用 `/kapi/<agent_id>`，`agent_id` 是包含域名的完整 AgentId。直接连接 OpenDAN 服务端口时也可使用 `/kapi/opendan`；zone 网关没有名为 `opendan` 的全局服务路由。三条路径挂载同一服务，使用相同的 owner/root 鉴权，方法与 `AgentStateClient` 的门面一一对应（`sessions.*`、`activity.*`、`perception.*`、`cognition.*`、`artifacts.*`、`behaviors.*`、`skills.*`、`workspaces.*`、`runtimes.*`、`goals.*`，见 [opendan README](../../../src/frame/opendan/README.md)）。真相仍是上面的文件；服务只做鉴权与转发，不引入第二份存储。

- `cognition.query` / `cognition.read` 是读：`session` 参数是调用方自报的所在 Session，服务端按它的配置绑定可见范围（§5）。
- 只承载读与带署名的写：`sessions.register`（`created_by` 必须是调用方）、`sessions.post_input`（`from` 由服务端改写为调用方）、`cognition.notebook_append`、`artifacts.decide`（服务端在调用期间持有 `artifact:<aid>` 锁）、`goals.create / revise / set_status / reopen / restart`（调用方 = 服务端看到的身份，`session` 参数是调用方自报的所在 Session，§9.5）。
- Goal 的驱动者 / 宿主写（`goals.write_state`、`goals.append_activations`、`goals.prepare_epoch`、`goals.end_epoch`、`goals.write_schedule`）一律拒绝（错误说明需要 AgentRoot）。
- 驱动者在 lease 下的写入（`report_state`、`perception.append`、`commit_cursor`、`register_version`、`update_location`、`commit_consolidation`）与 Agent 级锁依赖 flock，不经 kRPC：驱动者必须能看到 AgentRoot。
- 通知（宿主有通知服务时）：`notify.send {request: NotifySendReq}`、`notify.state {notify_id}`、`notify.list {request?: NotifyListReq}`、`notify.cancel {notify_id, reason}`、`notify.answer {notify_id, answer: {dismiss?, option?, text?}, via?: {agent_did, session_id?, input?}}`，转给宿主的 `NotifyHost`（zone：msg-center `notify.*` + TaskMgr；`--dev`：`state/notifies/<id>.json` 文件实现 `LocalNotify`，目录存在即启用）。服务端按请求里 `source.session_id` 从登记表补全来源（所属 Goal、用户所在的会话）。Session 里的 `agent-session notify` 用文件版 Agent State 时没有宿主的通知服务，改以宿主交给工具进程的 `BUCKYOS_APPCLIENT_SESSION_TOKEN`（Agent App 身份，`appid` 即运行时 app id）直接调用 msg-center（`ZoneNotify::for_session_process`），来源在 CLI 侧补全。都没有时报 `notifications are not available here: <原因>`；通知已结束时报 `notify_conflict`（附当前状态）。请求与视图类型见 `doc/message_hub/System Notify.md`。
- **Session 进程**（2026-10-25，primary-runtime TODO §6.1 / Q1）：不在 AgentService 机器上的 Session 进程（节点 session-runtime 上的 `agent-session` 等）以 Session 工作目录里 `.opendan/session.token` 的 token 调用（`odsess1.<sid>.<expires_at_ms>.<hmac>`，HMAC-SHA256，密钥 `state/session_token.key`，只对本 Agent 的服务与本 Session 有效）。这类 token 只能调用 `session.helper {request: {argv, stdin?, call_id?}}`（在控制侧以该 Session 的环境运行一条 `agent-session` 命令，返回 `{exit_code, stdout, stderr}`）、`session.task {request: {op, task_id, wait_ms?}}`（本进程正在推进的 run 的 task resolver，等同 `XLLM_TASK_ENDPOINT`）、`session.token_refresh`（续期）与 `session.service_token`（Agent 自己的 BuckyOS 会话 token、app id、owner：节点上的 `buckyos` / AICC 命令以 Agent 身份调用服务）；`sid` 参数若给出必须与 token 一致。拒绝以 `forbidden` 种类返回。节点经本机 gateway 访问：`state_url = http://127.0.0.1:3180/kapi/<agent_id>`（AgentService 进程的 `OPENDAN_NODE_STATE_URL` 可覆盖）。
- 错误以 `{"kind","message"}` 的 JSON 文本传回，`kind` 与文件实现的错误种类相同（`input_full` 另带 `session_id`、`pending`，`queue_missing` 另带 `session_id`），调用方按种类处理（如 `input_full`、`queue_missing` 可重试）。


## Workspace Manager

已知长期目录由 `AgentStateClient.workspaces()` 管理；身份元信息随目录，Agent State 保存登记与位置修订，Session 保存不可变快照。格式、kRPC 管理方法、幂等重试与生命周期见 [Agent Workspace Protocol](<Agent Workspace Protocol.md>)。登记不是权限，activity 不是写锁，归档/取消登记不删除项目文件。

## 9. Goals `state/goals/<goal_id>/`

Goal 是 Agent State 中的持久对象，是唯一的真相；它的 Goal Session 只携带 `session.goal = {goal_id, epoch}`，不复制定义。字段以 `src/protocol/goal.rs` 为准，Schema 见 `schema/goal*.schema.json`。

### 9.1 文件与写者

| 文件 | schema | 写者 | 规则 |
|---|---|---|---|
| `goal.json` | `opendan.goal/1` | GoalStore（`.lock` 上 flock，读改写） | 内容修订 `revision` 只因定义 / 预算 / 计划变化而 +1，且要求调用方给出看到的修订号（不一致 → `goal_conflict`）；状态变化、reopen、restart、检查脚本与检查时间表的变化（`set_check`）不增加它。`change_seq` 是 `changes.jsonl` 最后一条的 seq |
| `revisions/<rev>.json` | 同上 | GoalStore | 每个内容修订的不可变快照（不覆盖发布） |
| `checks/<version>.ts` | TypeScript | GoalStore | 检查脚本的各版本（不可变；`goal.json.schedule.check.version` 指向当前的） |
| `changes.jsonl` | `GoalChange` | GoalStore | 每次变化一行：`kind = created / revised / status / reopened / restarted / epoch_ended / check`、`who`、`via`（`owner / ui_session:<sid> / goal_session:<sid> / opendan`）、`reason`、`revision`、`epoch`、`detail` |
| `state.json` | `opendan.goal_state/1` | 当前 epoch 的 Goal Session 的驱动者（持其 session lease） | 真相在该 Session 的 `state.goal.state`（与 Session 同一次提交），这里是提交后发布的投影：`rev` 不大于已发布的、或 `epoch` 更旧的写入被忽略 / 拒绝 |
| `activations.jsonl` | `opendan.goal_activation/1` | 同上 | 按 `(epoch, seq)` 幂等追加（只追加大于末行的记录） |
| `epochs/<n>/state.json` | `opendan.goal_state/1` | 宿主 | epoch n 结束时的 Goal State（不覆盖发布） |
| `schedule.json` | `opendan.goal_schedule/1` | 宿主 | 检查时间表的进度与闸门的记录：`backend`、`timetable`（`every:<秒>` 或 `cron:<表达式>@<时区>`，变了从当前槽重新计）、`last_slot`、`pending`（已准入待投递的 fire）、`last_posted_key`、`start_posted_epoch`、`skipped[]`（最近 16 次：`paused / not_active / min_interval / period_budget / agent_budget / check_false / check_error / guarantee`）、`last_error`、`last_check{at_ms, outcome: true / false / error, reason, script_version, elapsed_ms}`、`check_errors`（连续出错次数）、`last_check_error`、`check_state`（脚本上次返回的 `state`）、`stats`（本周期：被硬间隔 / 预算拦下的槽、检查 true / false / error 次数、硬保障次数）、`guarantee_posted_for_ms`、`switched_off`（内置 Goal 被开关关掉） |

`goal.json` 的主要字段：`goal_id`（`g-<yyyymmddThhmmss>-<8 hex>`；带幂等键创建时为 `g-k<24 hex>` = H(agent, 创建者, key)）、`title`、`objective`、`scope{objects, paths, workspace}`、`lifecycle = achieve | maintain`、`achieved_when` / `healthy_state`、`distance_evaluation`、`constraints`、`initial_skills`、`owner`、`created_by{principal, via}`、`beneficiaries`、`budget`、`schedule{every_secs | cron + timezone, check{version, sha256, source, package?, updated_at_ms}, min_interval_secs, guarantee_secs, start_on_create}`（§9.4）、`behavior`、`status = active | paused | achieved | archived`（及 `status_changed_at_ms / status_reason / status_by`）、`session{epoch, sid}`、`epochs[]{epoch, sid, started_at_ms, start: created | reopened | restarted, reason, keep: all | facts, ended_at_ms, end: archived | achieved | restarted, end_reason}`。

`state.json`：`epoch`、`goal_revision_seen`、`last_activation`、`distance{text, observed_at_ms, evidence_refs, unknowns}`、`pending[]`、`questions[]`（每条可带 `candidate_id`）、`active_works[]{sid, objective, epoch, created_in_activation, goal_revision}`、`absorbed[]`（已吸收 Work，最多 512，跨 epoch 去重的依据）、`recent_works[]`、`current_skills[]`、`usage{period_start_ms, tokens, activations, works, work_tokens}`（tokens 是本周期各 epoch 的 Goal Session、work_tokens 是它的 Work 及其子 Session 本周期的用量，按 `usage.jsonl` 记录的时间计，运行中的也算）、`consecutive_no_action`、`holding{reason: min_interval / period_budget / agent_budget, since_ms, until_ms, checked_at_ms}`（事件正被闸门留在队列里）、`holds{period_start_ms, min_interval, budget}`（本周期被拦的次数，一次拦截记一次）、`last_error`（最近一次 Activation 的错误，下一次无错的 Activation 清掉；Goal Session 因 Workspace 绑定失效而 failed 时是 `{kind: "workspace_binding_invalid", message, workspace_binding, recovery, …}`，这时 Goal 要 restart 才会再运行）、`maintenance`（§9.7，restart 的 `facts` 也保留）。

`goal.json.extensions.opendan`（Schema `goal_opendan_extension`）是宿主的受信标记 `{builtin?, policy_version, capabilities[], records_perception}`：只有 owner 侧（`GoalCaller.session` 为空）的 `create` / `revise` 可以写，kRPC 的 `goals.create / revise` 带这个键一律 `forbidden`；Goal Session 按 `goal_id` 读取它得到能力，不复制进 SessionConfig。

### 9.2 Goal Session 与 epoch

- 同一时刻一个 Goal 只有一个当前 epoch；Goal Session 用 `goal-H("goal", agent_did, goal_id, epoch)` 身份键（`ids::goal_session_key`）在 Agent State 预留 `goal-YYYYMMDD-N` 短 sid，写进 Goal 定义后再创建 Session，重试沿用该预留 ID。幂等键为 `goal:<goal_id>:<epoch>`，`kind = goal`、`class = goal`、驱动者为宿主身份、**没有 parent**，目录默认 `<agent_root>/sessions/<sid>/`。登记表条目带 `goal_id`。
- 由宿主按 Goal 列表补建（reconcile）：创建 Goal 只写 Goal；`create_session` 拒绝不经宿主创建 Goal Session，`create_sub_session` 拒绝 `goal` 类子 Session。
- **reopen**（achieved / archived → active）与 **restart**（active / paused，开全新上下文）都在 `epochs[]` 追加新 epoch 并改 `session`；旧 Session 由其状态闸门在下一个提交点结束（restart：以 stopped 结束，`end = restarted`，**不停止 Work**）。宿主在旧 epoch 的 Session 结束且其 Goal State 全部发布后：`end_epoch` 记录结束时间；`prepare_epoch(n)` 把当前 `state.json` 存档到 `epochs/<旧 n>/state.json`，按新 epoch 的 `keep` 写出新的初始 state（`all`：整体保留；`facts`：只保留 `active_works / absorbed / recent_works / usage / goal_revision_seen`，skills 回到 `initial_skills`），再创建新 Session。各步幂等，中途崩溃由下一次 reconcile 补完。
- reopen 默认 `keep = all`（`fresh_state` → `facts`），restart 默认 `facts`（`carry_state` → `all`）。restart / reopen 不重置周期用量。

### 9.3 Work 的归属

Goal Session 派生的 Work 的 `origin.goal = {goal_id, revision, epoch, activation_seq, at_ms}`，登记表 `goal_id` 同步写入；Goal 修订后已运行的 Work 保留创建时的 `revision`。Work 的 `report.md` 与 `state.result.goal` 带同样的引用。Goal Session 吸收 Work 按 `goal_id` 而不是 `parent_session`：新 epoch 接管旧 epoch 的 Work，`absorbed` 保证只吸收一次。一个 Goal 的 Work 只在 Goal 被归档（或删除）时随之停止，epoch 结束不停止它。Goal 休眠（`status = paused`）时，宿主暂停它未结束的 Work，唤醒时只恢复因此暂停的（[Session Control Protocol](<Session Control Protocol.md>) §7）。

### 9.4 预算与运行机会的门槛

`budget = {activation: {max_total_tokens, max_runs, max_wall_secs}, period: {window_secs, max_total_tokens, max_activations, max_works}}`，缺省字段取 `agent.toml [session.goal] budget`，再缺省取内建值（activation 400000 tokens / 8 runs / 1800 s 墙钟，period 86400 s / 2000000 tokens / 24 works）。单次 Activation 的用量 = 该 Goal Session `usage.jsonl` 中 Activation 打开之后的记录（`OpenActivation.usage_offset`）；墙钟从 Activation 打开时算，到了不再开新的 run 段，以 `budget_exhausted` 关闭。周期是按 `window_secs` 对齐的固定窗口；周期用量 = 这个 Goal 的所有 Session（各 epoch 的 Goal Session、它的 Work 及其子 Session）在本周期 `usage.jsonl` 里的 token。Agent 总量 `[session.goal.agent_budget] {window_secs = 86400, max_total_tokens = 5000000}`（0 表示不设）是所有 Goal 这样算出来的用量之和。

一次运行机会要不要变成推理（[架构设计](<../OpenDAN Agent Session架构设计.md>) §12.1.1），按这个顺序判断（宿主 `goal::gate`）：

1. Goal 不是 `active`：不推理。休眠压过硬保障。
2. 距上次推理（上一次 Activation 关闭）不到硬间隔 `schedule.min_interval_secs`（缺省 `[session.goal] min_interval_secs`，内建 900）：不推理。
3. 距上次推理（没有过则从 epoch 开始）超过硬保障 `schedule.guarantee_secs`（缺省 `[session.goal] guarantee_secs`，内建 259200；0 关闭，否则不小于 3600——它压过预算，太短就成了不受预算约束的定时推理；创建、修订和 `agent.toml` 都拒绝更小的值，已存的更小值按 3600 算）：推理，跳过 4、5。宿主每个 quiet 段只投一次 `guarantee` 运行机会，原因里写最近一次检查的结果、连续出错次数和最后一次错误。
4. 周期预算用完，或 Agent 总量用完：不推理，到周期重置。此时宿主让这个 Goal 正在运行的 Work 在下一个推理边界暂停（暂停来源 `budget`），周期重置后只恢复因此暂停的。
5. 检查时间表到点时跑检查脚本：`true` 推理（`reason` 成为这次 Activation 的原因），`false` 或出错不推理。没有脚本的 Goal 每个槽都通过。

检查时间表是 `every_secs`（不小于 60）或 `cron` + `timezone`（5 段 cron，按该时区的墙上时间计槽，夏令时跳过的时刻不触发、重复的取早的那个；`timezone` 缺省时 `goal create` / `goal check set` 取调用会话的用户时区，再缺省 UTC）；有脚本而没有时间表时按 `[session.goal] check_every_secs`（内建 300）。检查脚本是 TypeScript，默认导出 `check(ctx, state)`，返回 `{run, reason?, state?}`（只返回布尔值等于 `{run}`），`state` 由宿主存进 `schedule.json.check_state` 并在下一次传回（最大 64 KiB）；`ctx` 有 `goal_id / title / objective / epoch / scope / agent_root / trigger / now_ms / last_inference_at_ms / last_check_at_ms / last_check / pending / active_works`，`ctx.host.call(method, params)` 是宿主提供的只读调用（目前 `memory.status {full?}`）。宿主用标准层 Tool 的 Deno 和 `buckyos-agent` 的 `deno.json`（Agent 容器里是 `$BUCKYOS_ROOT/bin/opendan/deno` 与 `bin/opendan/buckyos-agent/deno.json`，宿主安装是 `libexec/buckyos-tool/runtime/deno` 与 `libexec/buckyos-agent/deno.json`，开发环境退回 `PATH` 上的 `deno`）只给 `--allow-read --allow-net --allow-env` 运行，30 秒超时，日志最多 64 KiB；抛异常、超时、返回值不合约定都记为出错。

事件（Work 结束、真正要人回答的问题、转来的消息、问题得到回答）不跑检查，在 Goal Session 打开 Activation 之前过 1–4：被拦下的留在队列里（`state.holding`），放行时合并成一次 Activation。手动激活、定时检查与硬保障投来的运行机会、epoch 的开始不再过这道闸（它们是 owner 的显式操作，或宿主已经放行的）。Goal 带检查脚本且不是 `--start` 创建时，epoch 的开始先跑一次检查。Activation 关闭时 Goal Session 不再跟它启动的后台任务（结束也不打开新的 Activation）；Goal 只为 Work 的结束和能被回答的问题醒来：不收消息的 Work 在两个 Turn 之间的 `needs_input` 不唤醒它。

每次 Activation 的 Goal 头（`<budget>`、`<gate>`）告诉 Goal：周期用量（自己的、Work 的）、上限、重置时刻、Agent 总量、硬间隔与硬保障、检查时间表与脚本版本、本周期检查与拦截的次数、最近一次检查、连续出错、连续 `no_action` 的次数。

### 9.5 调用方与权限（只按类型区分）

调用方 = `GoalCaller{principal, session}`；`session` 为空即 owner 侧（kRPC、`xagent goal`），否则按该 Session 的 kind 判定：

| 调用方 | 可做 |
|---|---|
| owner | 全部 |
| UI Session | create；revise、`set_check` 自己创建的 Goal（`created_by.via = ui_session:<sid>`）；pause / resume / archive / achieved / reopen / restart |
| Goal Session（自己的 Goal） | 读；archive（须给 reason）；lifecycle 为 achieve 时 achieved（须给 evidence）；`set_check` 改自己的检查脚本和检查时间表（不能指定包内脚本，`restore_package` 可以）。不能 revise、pause / resume、reopen、restart |
| Work Session 等 | 只读 |

`achieved` 只在该 Goal 没有未结束的 Work 时允许（否则 `goal_conflict`）。越权返回 `forbidden`；Goal 不存在返回 `not_found`。

### 9.6 kRPC

`goals.list`、`goals.get {goal_id}`、`goals.revision {goal_id, revision}`、`goals.changes {goal_id, limit?}`、`goals.state {goal_id}`、`goals.epoch_state {goal_id, epoch}`、`goals.activations {goal_id, query?: {epoch?, limit?}}`、`goals.schedule {goal_id}`、`goals.check_script {goal_id, version?}`（脚本源码）、`memory.status {full?}`（Memory 的版本摘要，`full` 时带待处理 / 到期数与到期复核数）；写：`goals.set_check {goal_id, request: {script?, package?（owner）, restore_package?, clear_script?, every_secs?, cron?, timezone?, reason?}, session?}`、`goals.create {request, session?}`、`goals.revise {goal_id, request: {expected_revision, …}, session?}`、`goals.set_status {goal_id, request: {status, reason?, evidence?}, session?}`、`goals.reopen {goal_id, request: {reason, fresh_state?}, session?}`、`goals.restart {goal_id, request: {reason, carry_state?}, session?}`。驱动者 / 宿主写一律拒绝（§8）。宿主进程额外提供 `goals.activate` / `goals.post`（见 [opendan README](../../../src/frame/opendan/README.md)）。

### 9.7 内置整理 Goal（memory_maintenance）

设计与验收见 [Jarvis 默认 Goal Session TODO](../../../notepads/jarvis-default-goal-session-todo.md)。要点：

- **播种**：`[session.goal.maintenance] enabled = true` 时宿主每进程一次在 `GoalHost::tick` 里以 `GoalCaller::owner(<Agent DID>)` 调 `GoalStore::create`，`key = builtin:memory_maintenance`（同 key 返回同一 Goal，重启不重建），`extensions.opendan = {builtin: "memory_maintenance", policy_version, capabilities: ["memory_consolidation"], records_perception: false}`，`schedule.every_secs = 1800`，预算 120000 tokens / 4 runs 每次、24 h 内 1200000 tokens / 48 次、`max_works = 0`，`behavior` 取 `maintenance.behavior`（Jarvis：`memory_maintenance`）。`policy_version` 更高时只用 owner 路径 revise 标记，不改 owner 改过的预算 / 计划 / 状态。`enabled = false` 时不播种、不检查（已有 Goal 原样保留，`schedule.json.switched_off = true`）；pause / archive 走现有生命周期，重启不复活。
- **检查脚本**：初始脚本是 AgentRoot 的 `goal_checks/memory_maintenance.ts`（随 Jarvis 包分发），播种时经 `set_check` 写成这个 Goal 的脚本（`source = package:goal_checks/memory_maintenance.ts`）。它只是初始状态：Goal 可以改写自己的脚本；包升级时只在 Goal 仍用包内版本（`source` 以 `package:` 开头）且内容变了时替换，Goal 或 owner 改过的保留；`restore_package` 恢复成包内版本。初始脚本先用 `memory.status` 比 Memory 的版本向量，变了、到了暂缓或复核的期限、上次看到积压、或距上次完整检查满 2 小时才做完整检查，有新 / 到期材料或到期复核才返回 `true`（`reason` 是材料计数）。没有材料时整理 Goal 只在硬保障到点时推理一次（缺省 72 小时）。
- **机械维护**：清理重试、未完成 receipt 这些不需要模型的事由宿主在每个检查槽 drive 一次 Goal Session 完成，不算运行机会。
- **整理**（runner `consolidation.rs`）：drive 开始先持 `memory_consolidation` lease（Busy 即返回，不开 Activation），先做机械维护（未完成 receipt 推进、读了未提交的批次丢弃、`sweep_expired`、清理重试），再处理输入。宿主工具只对带能力的 Goal Session 暴露：`memory_consolidation_begin {limit?}` 读一批材料并把完整 `Batch` 写成 receipt（`GoalState.maintenance.receipts[]`，`operation_id = mc-<hash>`，一次 Activation 最多 3 批，有未完成 receipt 时拒绝）；`memory_consolidation_commit {operation_id, summary, operations[], dispositions[], tags?}` 先把计划持久化进 receipt，再以 `operation_id` 为幂等键提交 Memory envelope，再发布候选（P1），再清理（失败记 `cleanup_pending` 由下次机械维护重试）；阶段 `prepared → memory_committed → published → completed`，每步可重入，`StaleLease` 记 `failed` 与 `last_error`（不自动修锁）。`abandon: true` 关闭批次不处置。Activation 的 `report` 带 `acted / created_work` 而有未提交批次时被拒。Activation 的 token 上限取 `min(activation 上限, 周期剩余)`。Goal 头多一块 `<memory_maintenance …>`（材料计数、到期复核、簿记）。
- `GoalState.maintenance`：`receipts[]`（完成的保留 32 条）、`candidates[]`（§5.1 契约，P1 填）、`last_successful_consolidation_at_ms`、`consolidations / operations / dispositions / abandoned_batches`、`last_error`。

### 9.8 内置经验结晶 Goal（skill_crystallize）

设计见 [Skills 体系 TODO](../../../notepads/opendan-skills-system-todo.md) §3.8。要点：

- **播种**：开关打开时宿主每进程一次以 `GoalCaller::owner(<Agent DID>)` 创建，`key = builtin:skill_crystallize`，`extensions.opendan = {builtin: "skill_crystallize", policy_version: 1, capabilities: [], records_perception: true}`。开关是 owner 在 Skills 页设的 `.meta/skills_state.json` 的 `crystallize`（kRPC `skills.crystallize_set {enabled}`，owner 专用），没设时取 `[session.goal.crystallize] enabled`；库和 Jarvis 模板的缺省都是关：不播种，已有的这个 Goal 也不建 Session（`schedule.json.switched_off = true`）。
- **时间表与预算**：`schedule.every_secs = 86400`、`min_interval_secs = 3600`；每次 300000 tokens / 6 runs，24 h 内 600000 tokens / 2 次，`max_works = 0`。`behavior` 取 `crystallize.behavior`（缺省 `skill_crystallize`）。
- 初始检查脚本 `goal_checks/skill_crystallize.ts`（规则同 §9.7）：手册使用记录里的 Work 数比上次看时多 3 个以上，或多了且距上次看满一周，才返回 `true`。一次查看约 18 万 token（2026-11-10 实测）。
- 没有专用的宿主工具，按普通 Goal 运行：读 `.meta/skills_state.json` 的使用记录和它指向的 Work Session，用 `agent-session skill revise` 改写来源为 `local` / `imported` 的手册（旧文本留在 `revisions/`），其余改动写成 `skills_drafts/<slug>/` 草稿并通知 owner。


## 10. Runtimes `state/runtimes/<runtime_id>.json`

Agent 的 Session 与 Workspace 能用哪些 Runtime。记录（版本 5，2026-11-09，[runtime 工作模型 TODO](../../../notepads/opendan-agent-runtime-lifecycle-todo.md) §1、§10）回答四件事：怎么连（`connector`）、机器能做什么（`caps`）、这台 runtime 允许什么（`grant`）、Agent 用过它没有（`usage`）。runtime 是一个 sandbox，自己声明能力和权限：`grant` 是它自己的配置（节点的 `owner_agents` 策略、设备签发时的 profile 与期限），由探测得来，没有按 Agent 的授权层；在其中执行是 `agent_tool::runtime` 的事。schema：`schema/runtime_record.schema.json`。

```json
{ "version": 5, "runtime_id": "private", "name": "Private runtime", "desc": "一句话，给人和模型看",
  "runtime_host": "<native_host_id()>",
  "connector": { "type": "native" }
             | { "type": "node_executor", "node_did": "did:dev:…", "profile_id": "agent-…" }        // 节点
             | { "type": "node_executor", "executor_id": "<id>", "profile_id": "" }                   // daemon URL 设备（form B），profile 到达时解析
             | { "type": "ssh", "host": "…", "user": null, "port": null, "identity_file": null },
  "enabled": true,                           // 分配里的开关（AgentSettings.runtimes[id].enabled）；关掉的记录保留并说明原因
  "caps": { "os": "linux | macos | windows", "arch": "x86_64", "shell": "bash | pwsh | …", "desktop": false, "identity_mode": "current_user",
            "baseline": "standard | extended",                     // runtime 的声明（节点策略 / 设备 profile 的 baseline，执行器 describe 报告；手工登记的由登记者声明），决定其上 Session 的 cmd_skills
            "baseline_check": { "checked_at_ms": 0, "missing": [], "ok": true, "error": null },   // 标准工具核验，只作提示
            "tools_check": { "checked_at_ms": 0, "present": ["ffmpeg"], "missing": [], "error": null } },   // `check --commands` 核验过的命令
  "grant": { "ops": ["exec", "read", "write", "…"],   // 执行器允许的操作（login 的 caps），探测前为空（空 = 未知，不据此拒绝）
             "roots": ["/home/owner"],                // 执行器强制的权限边界（私有为空 = 不限）；Agent 目录按目录约定定位，不记 home
             "computer": "none | observe | control",  // 这台 runtime 允许的 ComputerUse（执行器 describe 报告）
             "expires_at_ms": null },                 // daemon URL 的期限；到期 → runtime_unavailable，续期后恢复
  "availability": "available | runtime_unavailable | permission_denied",
  "builtin": true, "default": false,          // default = owner 的 default_runtime_id：owner 说“我电脑”指的机器、owner 给的绝对路径在哪解释；不再决定 chat 跑在哪
  "lifecycle": "active | retired",            // retired：从 Agent 移除 / 设备被 revoke 或 delete / unregister，永久保留，不参与候选；到期不算 retired
  "usage": { "sessions_total": 0, "last_used_at_ms": null, "last_session_id": null, "workspaces": 0 },
  "revision": 1, "registered_by": "…", "registered_at_ms": 0, "updated_at_ms": 0, "checked_at_ms": 0, "last_error": null }
```

- **私有 runtime 永远在**（`private`，别名 `container` / `local`，`lookup` 按别名解析）：Agent Service 进程自己的环境——aios 容器，或原生运行（只在不带容器的 Lite 版：zone owner 的 Agent 以节点 `host` 为默认）时的宿主进程环境。`connector: native`、恒 `available`、`grant` 全部；容器声明 `extended`，原生时基线与 ComputerUse 取节点策略的声明（向本机执行器 `describe` 学来，学到之前是 `standard` / `none`）。它一定拥有 `$BUCKYOS_ROOT` 布局（`bin/`、`data/home/<owner>/agents/<agent_id>`、`instance/`、`tools/`），Loader 启动时核验，缺了记为部署错误而不是“不可用”。
- **内建 = 系统配置的投影**：宿主（Loader）启动时及 owner 改动 AgentSettings 后幂等补建 `private` 与 `AgentSettings.runtimes` 里的每个 runtime：节点（`host` 与其它 `node-*`）为 `connector: node_executor {node_did, profile_id: agent-<hash>}`（profile 由 scheduler 从节点 `owner_agents` 策略派生），daemon URL 设备（`AgentRuntimeTarget::Executor`，id 为 `exec-<executor_id>`）为 `connector: node_executor {executor_id, profile_id}`（到达时经 control panel 的在线列表取路由，`expires_at_ms` 来自它）。settings 里的一条只有 `{enabled, target}`：`enabled` 投影成记录的开关，其余都不来自 settings。settings 里没有 `host` 时仍补一个 `enabled = false` 的占位说明原因。**settings 里被移除的条目，记录转 `retired`**（保留 `usage`），再次分配时回到 `active`。补建不覆盖 owner 改过的私有 runtime `name`；从 runtime 学来的 `grant` 与 `caps` 在 connector 不变时保留。内建记录不能取消登记。
- **grant 只有一个来源**：对 runtime 的探测（`login` 的 caps、`describe` 的 roots / ComputerUse / 基线、在线列表的期限）。要不同的权限就建不同的 runtime，不按 Agent 缩窄。两个开关是派生的：只读 = `ops` 已知且没有写操作；能建 Workspace = runtime 可用、不是只读、没有期限。`roots` 只是边界：Agent 在节点上的目录是 `<账户 home>/.opendan/<agent_id>`（home 由执行器 `describe` 报告；home 不在 roots 内时——典型是只开了一个工作目录的设备——用第一个 root 下的 `.opendan/<agent_id>`），建 Session 时就核验目标可达、目录可放（不行则创建失败，Todo 派发时进 `block_reason`），私有 runtime 上是 `<owner home>/agents/<agent_id>`。
- **可用性**：`observe` 按记录判断——`retired` / `enabled = false` / grant 到期 → `runtime_unavailable`（附原因）；私有记录登记在别的宿主 → `runtime_unavailable`，否则 `available`；节点配置齐全即 `available`；ssh → 本 runner 尚不能承载。`check {runtime_id, commands?}` 对节点另做真实探测（login + describe），失败写 `last_error`，成功时把执行器的 roots、`ops`、ComputerUse、声明基线、期限、平台、`identity_mode`、desktop 写进记录，再核验标准工具（`caps.baseline_check`）与 `commands`（`caps.tools_check`）。`register` 后立刻 check 一次；Loader 在投影后台对每个可用 runtime 做一次 check。
- **写**：`register`（owner / 开发添加 ssh 等，`grant.roots` 必填，同参数重试幂等，已存在的 id 拒绝）、`update`（CAS `expected_revision`，只改 `name / desc / roots`）、`check`、`unregister`（CAS，内建拒绝；**语义是 retire**：记录改 `lifecycle: retired` 永久保留）、`note_usage {runtime_id, event}`（`session {session_id}` / `workspace`，`freeze_runtime` 与 Workspace 的创建 / 导入处调用）；`.locks/runtimes.lock` 互斥。`ensure_builtin` 只给宿主，kRPC 拒绝。
- **id**：小写字母、数字、`-`、`_`；`container`、`local` 保留为 `private` 的旧名。列表里私有记录永远在最前。
- **使用者**：Workspace 位置的 `runtime_id` 是 `private`（旧记录的 `container` / `local`）或一个可用、不是只读、没有期限的节点 runtime（`workspaces.create / import` 对只读或有期限的 runtime 报 `runtime_no_workspace`；[Agent Workspace Protocol](<Agent Workspace Protocol.md>)）；Session 创建时 `api::freeze_runtime` 选择并冻结工作 runtime（UI / chat 恒私有；其它：显式 > 继承 > 私有；owner 的 `default` 不参与），`create-worksession --runtime <id>` / Todo 的 `--runtime` 要求已登记、`active`、`enabled`、`available`，只读的 runtime 不承载会写的 work，否则拒绝不回退；Todo 点名的 runtime 不能用时，Todo 带着原因（`block_reason`）等 plan 处理。Agent 级原语 `runtime_exec(runtime_id, command, cwd, timeout)` / `runtime_put(runtime_id, local, remote_dir)`（libopendan `runtime/mod.rs`）在 Session 之外对任一可用 runtime 执行命令 / 放文件，记录在 `state/runtime_ops/<op_id>.json`；`agent-session run` 走它。
- kRPC（宿主 opendan 另有，见 `src/frame/opendan/README.md`）：`runtimes.settings`（分配：`runtimes`、生效的 `default_runtime_id`、配置的 `configured_default_runtime_id`）、`runtimes.settings_set {runtime_id, enabled?, default?, confirm?}`（只改已分配条目的启停与默认，不新增目标也不改 runtime 的配置；停用默认 runtime 不清默认）、`runtimes.remove {runtime_id, confirm?}`（从 Agent 移除，记录转 `retired`）、`runtimes.impact {runtime_id}`（进行中的 Session、等待的 Todo、Workspace、是否 owner 默认）。分配与 runtime 自己的配置是 control-panel 的事（`agent.update`、`agent.runtime_catalog`、`node.owner_agents_set`、`executor.device.*`）。
- kRPC：`runtimes.list`、`runtimes.lookup {runtime_id}`；写：`runtimes.register {request}`、`runtimes.update {runtime_id, request}`、`runtimes.check {runtime_id, commands?}`、`runtimes.unregister {runtime_id, expected_revision}`、`runtimes.note_usage {runtime_id, event}`（署名为服务端看到的调用方）。
