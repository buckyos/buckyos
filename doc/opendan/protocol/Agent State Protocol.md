# Agent State Protocol

版本 3 · 2026-10-11 · 由 `libopendan` 反写（`src/protocol/agent_state.rs`、`src/protocol/goal.rs`、`src/state/*`）

2026-10-11：增加 Goals（§9，`state/goals/`）、登记表 `goal_id`、查询 `goal_id`、kRPC `goals.*`。只是加法。

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
    goals/<goal_id>/                        Goal（§9）：goal.json revisions/ changes.jsonl state.json
                                            activations.jsonl epochs/<n>/state.json schedule.json .lock
  memory/  notebook/  attention_signals/    认知（agent_tool 实现与其内部锁）
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
  "objective": "…", "scope": { "paths": [], "objects": [] },
  "goal_id": "g-…",                          // Goal Session 的 Goal，或创建 Work 的 Goal（session.goal / origin.goal）
  "status": { "rev": 17, "run_state": "…", "outcome": null, "acceptance": "…", "one_line_status": "…",
              "report_brief": "≤500 字", "pending_decision": null,
              "waiting_for": "input | tool | event | children",   // 仅 run_state = waiting 时；父据此区分“子在等输入”
              "turn_open": false,
              "activity": { "summary": "…", "touching": [], "heartbeat_ms": 0 },
              "last_runner": { "runner_id": "…", "host": "…", "pid": 0, "lock_epoch": 0, "at_ms": 0 },
              "last_error": null, "updated_at_ms": 0 } }
```

- **注册**：创建者用不覆盖发布写入一次；已存在且 location 相同或同一身份（幂等键 + 创建者 + 驱动者 + kind）→ 返回已有条目；否则 SessionIdConflict。
- **回报**：驱动者持 session 锁、在每次提交之后写 `status`（整条原子替换）；`status.rev ≤` 已有值时忽略。回报失败由下次 drive 按 `state.reported_rev` 补发。`status` 只是缓存，真相是 session 的 state.json。
- **巡检**：location 不存在时只写 `.unreachable/<sid>` 标记，不删除、不改写条目（保持单写者）；驱动者下次回报时清除。
- **子 Session**：`children_of(parents)` = `origin.parent_session` 属于 `parents` 的条目（按 session_id 排序），是查询的一个派生视图。父对子的隐式关注、同步等待的 resolver 与子 Session 的托管进程都只用它和 `lookup`。
- **查询**是派生视图（扫描 + mtime 缓存），可以另建可删除重建的索引。`RegistryQuery.goal_id` 取出一个 Goal 的全部 Goal Session（各 epoch）与它派生的 Work。

## 3. 活动 Session 视图

- 活跃 = `run_state ∈ {running, waiting}`。`running` 且 `max(heartbeat_ms, updated_at_ms)` 超过 5 分钟 → 标为“可能已中断”；`waiting` 不要求心跳。
- 声明的引用 = `scope.paths` + `scope.objects` + `status.activity.touching[].ref` + `artifact:<aid>`。两个引用相等或一方是另一方的路径前缀（`ws:snake/src/` 与 `ws:snake/src/collision.js`）即有交集。
- 关系排序：同一 workspace 或同一产物（`same_target`）> touching 有交集（`overlap`）> 其它；`agent_access = status_only` 的条目只给 one_line_status。
- activity 来源：创建时 scope（bootstrap 输入批次写入）、`control(activity)`、runner 从写类工具参数推断（`write_file` / `edit_file` 的 path）；run 结束清空 touching，finished 清空整个 activity。
- 它是**避让提示**，不是锁；冲突裁决属于 workspace。

## 4. 感知

```jsonc
// state/perception/<sid>.jsonl，单写者 = 该 session 的驱动者（或组件阶段的模拟 Session），seq 严格递增
{"seq":31,"at_ms":0,"session_id":"…","kind":"run_digest|observation|task_outcome|task_discarded",
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
- 自动记录：每次 run 结束写 `run_digest`，payload `{run_id, turn, turn_status}`：`turn` 是该 run 结束时所属的 Turn，`turn_status`（`completed | failed | budget_exhausted | stopped`）只在这次 run 结束同时关闭了 Turn 时有值，否则为 null（例如 `WAIT_USER_MSG` 未交付答复、fork 子 process 返回；drive 补发缺失记录时也为 null）。它按 run 结束生成，不代表 Turn 完成；普通切换与 fork / independent 挂起不写。finished 追加 `task_outcome`；discard 追加 `task_discarded`（保留来源）；`perception` 类型输入并入 `observation`。`state.perception_seq` 在提交中预留，追加在提交之后，缺失的由下次 drive 补发。这三种 Runtime 记录只进入整理批次，不出现在其它 Session 的变化里。
- **待处理集合**：由 Memory 提交日志中的逐条处置计算（未处置 = 新增；`deferred` 且到期 = 待处置；已终局但正文未清 = 待重试清理），不按字节游标跳过暂缓材料。见 Memory 组件 `pending_work` / `begin`。
- **写入口**：Session 的 `ctl perceive` 控制输入由驱动者经 `Memory::record_perception` 写入：驱动者分配 `seq`、按 Session 绑定主体（UI 取对话 peer，Work 取 Agent 的 owner，Goal Session 取 Goal 的 owner，有 Workspace 绑定时附 `workspace:<id>`，grants 另含 Agent 自身）、绑定来源事件（`--source` 给出的本 Session 输入 key，否则这条控制输入本身、`actor_kind = agent`）与时区锚点，控制输入的 key 是幂等键。Goal 的 `extensions.opendan.records_perception = false`（整理 Goal）时该输入被拒绝（`input_rejected`），Runtime 记录也不写。
- **防回声**：Goal 带 `memory_consolidation` 能力的 Goal Session（各 epoch）的记录不成为材料；没有字节游标。

## 5. 认知（边界）

Memory Graph（`memory/`，schema 4.0：认知另带 `stability = stable | contextual | volatile`、`last_evidence_at`（独立证据时间，新建整理认知必填，修订不续期）、`review_at`（结构化复核日期）；`created_at` 是 `first_occasion` 时间的读取投影；召回淡出按稳定度 365 / 30 / 1 天一分、上限 20 分，`explicit` 不淡出；旧主版本根目录拒绝打开，不迁移）由 `agent_tool::agent_memory` 实现：`.meta/occasions.jsonl` 是追加写的提交真相源，一次整理提交是一条 occasion（图操作 + 逐条处置 + 幂等键 + `produced_by` + 真实 `actor_session_id` + 整理 lease 的 epoch）；读路径不取写锁。两层的统一门面是 `libopendan::memory::Memory`（感知写入、`query_topic` / `query` / 按引用读取、`changes_since`、`pending_work` / `due_reviews`、整理与清理）；topic 与观察进度（`ObservationState`）归 Session 保存，Memory 不登记订阅。认知只在 `memory_consolidation` lease 下经 `Consolidator::commit` 改变：该 lease 由 Goal 带 `memory_consolidation` 能力的 Goal Session 的 drive 全程持有，其宿主工具 `memory_consolidation_begin / commit` 是唯一写路径（§9.7）；普通 Session 只能写感知和读取，`agent-memory` CLI 在 Session shell 内拒绝写。

`recall_hints`（runner 的旧门面）没有 tag 时返回空列表（未触发），存储不可读时返回错误；`notebook_append` 与 `notebook/`、`attention_signals/` 是待退役的旧调用方，S 阶段移除。非 Rust 实现通过 `agent_tool agent-memory …` CLI 访问 Graph。

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

## 7. behavior 目录（BehaviorCatalog）

behavior 是 Agent 的配置，由 Agent State 提供（`AgentStateClient::behaviors()`：`identity / get / list / revision`），Session 在构造时冻结（[Session Directory Protocol](<Session Directory Protocol.md>) §3 `prompt.frozen`）。文件版读 `<agent_root>/behaviors/<name>.toml`（文本里的 `__INCLUDE(<相对路径>)__` 相对所在文件展开，最多 8 层）、`role.md`、`self.md`、`i18n/<lang>.md`；`revision` 是这些文件 `路径|长度|mtime` 的 sha256。schema：`schema/behavior_config.schema.json`。

```toml
[meta]
objective = "…"
next = ["do", "review"]            # 可切换 / 可调用的目标：冻结闭包

[prompt]
mode = "behavior"                  # function_call | behavior；缺省用会话的 loop_model
system = "… {{ session.id }} …"    # system 模板；变量：identity.* behavior.{name,objective,mode} session.{id,kind,objective,driver,scope,parent} workspace.id
on_init = "…"                      # 三类受控输入与半订阅快照的模板（缺省内建）
on_input = "…"
on_context_switch = "…"
semi_subscription_snapshot = "…"

[input]                            # 消费策略（缺省用会话的）
mode = "batch"                     # single | batch
media = "reference"                # reference | inline
background = "batch"               # batch | after_tools：after_tools 时每批工具结果后宿主追加变化的 TodoList / 子 Session 行（<<background_context>>）

[capabilities]
tool_whitelist = ["group:bash", "behavior:analyze_intent", "report"]   # → tools.tools；`group:<内置工具组>` → {groupname}，其余 → {name}（运行环境提供的工具）；`behavior:<name>` 是该 behavior 的 [tool] 函数
action_whitelist = ["shell"]       # → tools.actions
call_intent = false                # true（只用于 function_call）：每个原生调用必须带非空 `intent`，宿主加入 schema、执行前校验、交给工具前去掉

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

Session 模板的覆盖在 `agent.toml`：`[session.<class>]` 的 `turns = "one" | "unbounded" | <n>`、`wait_user_msg`（`allowed | finish_failed | finish_completed | end_turn`）、`input = "none" | "queue"`、`observe`、`load_hints`、`default_behavior`、`max_process_depth`、`max_sub_sessions`、`max_session_depth`；内建 class 为 `work / ui / self_check / goal`，其它 class 必须在这里声明。`goal` 类另有 `min_interval_secs`、`[session.goal.budget.activation|period]`（§9.4）与 `[session.goal.maintenance] enabled / decision / behavior`（§9.7）。

定位（不是协议）：`connect(agent_did, who)` 依次取进程内注册的实现、`$OPENDAN_AGENT_ROOT` 或 `~/.opendan/agents.toml`（`"<did>" = "<agent_root>"`）指向的 AgentRoot、`$OPENDAN_AGENT_STATE_URL` 的 kRPC 服务（服务端随 OpenDAN 改造提供）。

## 8. kRPC 形态

看不到 AgentRoot 的调用方（容器或远端沙箱里的层 ② 工具、WebUI、其它应用）经托管该 Agent 的 OpenDAN 进程访问 Agent State。WebUI 在 Agent 自己的域名使用 `/api/opendan`，由网关按应用路由并检查 SSO 登录；跨应用调用经 zone 网关使用 `/kapi/<agent_id>`，`agent_id` 是包含域名的完整 AgentId。直接连接 OpenDAN 服务端口时也可使用 `/kapi/opendan`；zone 网关没有名为 `opendan` 的全局服务路由。三条路径挂载同一服务，使用相同的 owner/root 鉴权，方法与 `AgentStateClient` 的门面一一对应（`sessions.*`、`activity.*`、`perception.*`、`cognition.*`、`artifacts.*`、`behaviors.*`、`skills.*`、`workspaces.*`、`runtimes.*`、`goals.*`，见 [opendan README](../../../src/frame/opendan/README.md)）。真相仍是上面的文件；服务只做鉴权与转发，不引入第二份存储。

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
| `goal.json` | `opendan.goal/1` | GoalStore（`.lock` 上 flock，读改写） | 内容修订 `revision` 只因定义 / 预算 / 计划变化而 +1，且要求调用方给出看到的修订号（不一致 → `goal_conflict`）；状态变化、reopen、restart 不增加它。`change_seq` 是 `changes.jsonl` 最后一条的 seq |
| `revisions/<rev>.json` | 同上 | GoalStore | 每个内容修订的不可变快照（不覆盖发布） |
| `changes.jsonl` | `GoalChange` | GoalStore | 每次变化一行：`kind = created / revised / status / reopened / restarted / epoch_ended`、`who`、`via`（`owner / ui_session:<sid> / goal_session:<sid> / opendan`）、`reason`、`revision`、`epoch`、`detail` |
| `state.json` | `opendan.goal_state/1` | 当前 epoch 的 Goal Session 的驱动者（持其 session lease） | 真相在该 Session 的 `state.goal.state`（与 Session 同一次提交），这里是提交后发布的投影：`rev` 不大于已发布的、或 `epoch` 更旧的写入被忽略 / 拒绝 |
| `activations.jsonl` | `opendan.goal_activation/1` | 同上 | 按 `(epoch, seq)` 幂等追加（只追加大于末行的记录） |
| `epochs/<n>/state.json` | `opendan.goal_state/1` | 宿主 | epoch n 结束时的 Goal State（不覆盖发布） |
| `schedule.json` | `opendan.goal_schedule/1` | 宿主 | 计划进度：`backend`、`every_secs`、`last_slot`、`pending`（已准入待投递的 fire）、`last_posted_key`、`start_posted_epoch`、`skipped[]`（最近 16 次：`paused / min_interval / period_budget / not_active`，内置整理 Goal 另有 gate 原因）、`last_error`、`gate`（§9.7，只有内置整理 Goal 有） |

`goal.json` 的主要字段：`goal_id`（`g-<yyyymmddThhmmss>-<8 hex>`；带幂等键创建时为 `g-k<24 hex>` = H(agent, 创建者, key)）、`title`、`objective`、`scope{objects, paths, workspace}`、`lifecycle = achieve | maintain`、`achieved_when` / `healthy_state`、`distance_evaluation`、`constraints`、`initial_skills`、`owner`、`created_by{principal, via}`、`beneficiaries`、`budget`、`schedule{every_secs, min_interval_secs, start_on_create}`、`behavior`、`status = active | paused | achieved | archived`（及 `status_changed_at_ms / status_reason / status_by`）、`session{epoch, sid}`、`epochs[]{epoch, sid, started_at_ms, start: created | reopened | restarted, reason, keep: all | facts, ended_at_ms, end: archived | achieved | restarted, end_reason}`。

`state.json`：`epoch`、`goal_revision_seen`、`last_activation`、`distance{text, observed_at_ms, evidence_refs, unknowns}`、`pending[]`、`questions[]`（每条可带 `candidate_id`）、`active_works[]{sid, objective, epoch, created_in_activation, goal_revision}`、`absorbed[]`（已吸收 Work，最多 512，跨 epoch 去重的依据）、`recent_works[]`、`current_skills[]`、`usage{period_start_ms, tokens, activations, works, work_tokens}`、`last_error`、`maintenance`（§9.7，restart 的 `facts` 也保留）。

`goal.json.extensions.opendan`（Schema `goal_opendan_extension`）是宿主的受信标记 `{builtin?, policy_version, capabilities[], records_perception}`：只有 owner 侧（`GoalCaller.session` 为空）的 `create` / `revise` 可以写，kRPC 的 `goals.create / revise` 带这个键一律 `forbidden`；Goal Session 按 `goal_id` 读取它得到能力，不复制进 SessionConfig。

### 9.2 Goal Session 与 epoch

- 同一时刻一个 Goal 只有一个当前 epoch；Goal Session 用 `goal-H("goal", agent_did, goal_id, epoch)` 身份键（`ids::goal_session_key`）在 Agent State 预留 `goal-YYYYMMDD-N` 短 sid，写进 Goal 定义后再创建 Session，重试沿用该预留 ID。幂等键为 `goal:<goal_id>:<epoch>`，`kind = goal`、`class = goal`、驱动者为宿主身份、**没有 parent**，目录默认 `<agent_root>/sessions/<sid>/`。登记表条目带 `goal_id`。
- 由宿主按 Goal 列表补建（reconcile）：创建 Goal 只写 Goal；`create_session` 拒绝不经宿主创建 Goal Session，`create_sub_session` 拒绝 `goal` 类子 Session。
- **reopen**（achieved / archived → active）与 **restart**（active / paused，开全新上下文）都在 `epochs[]` 追加新 epoch 并改 `session`；旧 Session 由其状态闸门在下一个提交点结束（restart：以 stopped 结束，`end = restarted`，**不停止 Work**）。宿主在旧 epoch 的 Session 结束且其 Goal State 全部发布后：`end_epoch` 记录结束时间；`prepare_epoch(n)` 把当前 `state.json` 存档到 `epochs/<旧 n>/state.json`，按新 epoch 的 `keep` 写出新的初始 state（`all`：整体保留；`facts`：只保留 `active_works / absorbed / recent_works / usage / goal_revision_seen`，skills 回到 `initial_skills`），再创建新 Session。各步幂等，中途崩溃由下一次 reconcile 补完。
- reopen 默认 `keep = all`（`fresh_state` → `facts`），restart 默认 `facts`（`carry_state` → `all`）。restart / reopen 不重置周期用量。

### 9.3 Work 的归属

Goal Session 派生的 Work 的 `origin.goal = {goal_id, revision, epoch, activation_seq, at_ms}`，登记表 `goal_id` 同步写入；Goal 修订后已运行的 Work 保留创建时的 `revision`。Work 的 `report.md` 与 `state.result.goal` 带同样的引用。Goal Session 吸收 Work 按 `goal_id` 而不是 `parent_session`：新 epoch 接管旧 epoch 的 Work，`absorbed` 保证只吸收一次。一个 Goal 的 Work 只在 Goal 被归档（或删除）时随之停止，epoch 结束不停止它。

### 9.4 预算

`budget = {activation: {max_total_tokens, max_runs}, period: {window_secs, max_total_tokens, max_activations, max_works}}`，缺省字段取 `agent.toml [session.goal] budget`，再缺省取内建值（activation 400000 tokens / 8 runs，period 86400 s / 24 works）。单次 Activation 的用量 = 该 Goal Session `usage.jsonl` 中 Activation 打开之后的记录（`OpenActivation.usage_offset`）。周期是按 `window_secs` 对齐的固定窗口。

### 9.5 调用方与权限（只按类型区分）

调用方 = `GoalCaller{principal, session}`；`session` 为空即 owner 侧（kRPC、`xagent goal`），否则按该 Session 的 kind 判定：

| 调用方 | 可做 |
|---|---|
| owner | 全部 |
| UI Session | create；revise 自己创建的 Goal（`created_by.via = ui_session:<sid>`）；pause / resume / archive / achieved / reopen / restart |
| Goal Session（自己的 Goal） | 读；archive（须给 reason）；lifecycle 为 achieve 时 achieved（须给 evidence）。不能 revise、pause / resume、reopen、restart |
| Work Session 等 | 只读 |

`achieved` 只在该 Goal 没有未结束的 Work 时允许（否则 `goal_conflict`）。越权返回 `forbidden`；Goal 不存在返回 `not_found`。

### 9.6 kRPC

`goals.list`、`goals.get {goal_id}`、`goals.revision {goal_id, revision}`、`goals.changes {goal_id, limit?}`、`goals.state {goal_id}`、`goals.epoch_state {goal_id, epoch}`、`goals.activations {goal_id, query?: {epoch?, limit?}}`、`goals.schedule {goal_id}`；写：`goals.create {request, session?}`、`goals.revise {goal_id, request: {expected_revision, …}, session?}`、`goals.set_status {goal_id, request: {status, reason?, evidence?}, session?}`、`goals.reopen {goal_id, request: {reason, fresh_state?}, session?}`、`goals.restart {goal_id, request: {reason, carry_state?}, session?}`。驱动者 / 宿主写一律拒绝（§8）。宿主进程额外提供 `goals.activate` / `goals.post`（见 [opendan README](../../../src/frame/opendan/README.md)）。

### 9.7 内置整理 Goal（memory_maintenance）

设计与验收见 [Jarvis 默认 Goal Session TODO](../../../notepads/jarvis-default-goal-session-todo.md)。要点：

- **播种**：`[session.goal.maintenance] enabled = true` 时宿主每进程一次在 `GoalHost::tick` 里以 `GoalCaller::owner(<Agent DID>)` 调 `GoalStore::create`，`key = builtin:memory_maintenance`（同 key 返回同一 Goal，重启不重建），`extensions.opendan = {builtin: "memory_maintenance", policy_version, capabilities: ["memory_consolidation"], records_perception: false}`，`schedule.every_secs = 1800`，预算 120000 tokens / 4 runs 每次、24 h 内 1200000 tokens / 48 次、`max_works = 0`，`behavior` 取 `maintenance.behavior`（Jarvis：`memory_maintenance`）。`policy_version` 更高时只用 owner 路径 revise 标记，不改 owner 改过的预算 / 计划 / 状态。`enabled = false` 时不播种、不检查（已有 Goal 原样保留，gate 记 `disabled`）；pause / archive 走现有生命周期，重启不复活。
- **gate**（宿主 `host/maintenance.rs`，在 reconcile 中对 `created / reopened / restarted` 与每个定时 fire 生效；不是 `AdmissionCheck`）：先按 Memory 版本向量（`memory_snapshot`，不打开 Graph）与上次摘要比对；非 start、非基线槽（`slot % 4 == 0`，UTC 对齐的整 2 小时）、摘要未变、无积压、暂缓未到期、复核未到期、无未完成 receipt 则 `no_change`；否则做完整确定性检查 `pending_work` + `due_reviews`：无材料 `no_work`；只有清理重试 / 未完成 receipt 则 `mechanical`（只 drive Session 做恢复与清理，不投运行机会）；有新 / 到期材料或到期复核才投运行机会。检查出错记 `unknown` 并退避 5m → 15m → 30m。`paused` / `period_budget` 也在这里记。水位在 `schedule.json.gate`：`status`、`last_checked_slot / last_baseline_slot / last_checked_at_ms / last_full_check_at_ms`、`last_skip_reason`、`last_inputs_digest`、`pending{new, due, waiting, retry_cleanup, next_deadline, oldest_at_ms}`、`due_reviews`、`next_review_at`、`unfinished_receipts`、`last_run_at_ms / last_run_slot`、`backoff_until_ms / backoff_level`、`last_error`。
- **整理**（runner `consolidation.rs`）：drive 开始先持 `memory_consolidation` lease（Busy 即返回，不开 Activation），先做机械维护（未完成 receipt 推进、读了未提交的批次丢弃、`sweep_expired`、清理重试），再处理输入。宿主工具只对带能力的 Goal Session 暴露：`memory_consolidation_begin {limit?}` 读一批材料并把完整 `Batch` 写成 receipt（`GoalState.maintenance.receipts[]`，`operation_id = mc-<hash>`，一次 Activation 最多 3 批，有未完成 receipt 时拒绝）；`memory_consolidation_commit {operation_id, summary, operations[], dispositions[], tags?}` 先把计划持久化进 receipt，再以 `operation_id` 为幂等键提交 Memory envelope，再发布候选（P1），再清理（失败记 `cleanup_pending` 由下次机械维护重试）；阶段 `prepared → memory_committed → published → completed`，每步可重入，`StaleLease` 记 `failed` 与 `last_error`（不自动修锁）。`abandon: true` 关闭批次不处置。Activation 的 `report` 带 `acted / created_work` 而有未提交批次时被拒。Activation 的 token 上限取 `min(activation 上限, 周期剩余)`。Goal 头多一块 `<memory_maintenance …>`（材料计数、到期复核、簿记）。
- `GoalState.maintenance`：`receipts[]`（完成的保留 32 条）、`candidates[]`（§5.1 契约，P1 填）、`last_successful_consolidation_at_ms`、`consolidations / operations / dispositions / abandoned_batches`、`last_error`。


## 10. Runtimes `state/runtimes/<runtime_id>.json`

Agent 的 Session 与 Workspace 能用哪些 Runtime。登记只说明“有这个 Runtime、本宿主现在能不能用、Workspace 建议放在哪”；在其中执行是 `agent_tool::runtime` 的事。schema：`schema/runtime_record.schema.json`。

```json
{ "version": 3, "runtime_id": "container", "kind": "container | host_native | remote_node | remote_ssh",
  "name": "Agent container", "description": "…", "runtime_host": "<native_host_id()>",
  "exec": { "kind": "native" },            // agent_tool RuntimeConfig；null = 未启用 / 还没有执行器
  "roots": ["<agent_root>/workspaces", "/home/<owner>"],
  "capabilities": { "network": true, "os": "linux" },
  "availability": "available | runtime_unavailable | permission_denied",
  "builtin": true, "default": false, "computer": "none | observe | control",
  "baseline": "standard | extended",       // 登记者声明（Q5）：决定其上 Session 的 cmd_skills
  "platform": { "os": "linux | macos | windows", "arch": "x86_64", "shell": "bash | pwsh | …" },
  "baseline_check": { "checked_at_ms": 0, "missing": [], "ok": true, "error": null },  // 只作提示
  "revision": 1, "registered_by": "…", "registered_at_ms": 0, "updated_at_ms": 0,
  "checked_at_ms": 0, "last_error": null }
```

- **内建 = 系统配置的投影**（2026-10-23）：宿主（Loader）启动时及 owner 改动 AgentSettings 后幂等补建 `container`（Agent 进程自己所在的环境，`exec = {kind: native}`）和 `AgentSettings.runtimes` 里的每个 runtime：`host`（owner 的桌面节点，`kind = host_native`）与其它节点（`kind = remote_node`），启用时 `exec = {kind: remote_node, remote_node: {node_did, profile_id: agent-<hash>}}`（profile 由 scheduler 从节点 `owner_agents` 策略派生），关闭时 `exec = null`、`capabilities.enabled = false`。`default` 标记 owner 的 `default_runtime_id`；`computer` 是 owner 给该 runtime 的 ComputerUse。settings 里没有 `host` 时仍补一个 `exec = null` 的占位说明原因。Agent Service 原生运行时（zone owner 的 Agent 默认 runtime 是节点 `host`，node_daemon 以 `OPENDAN_SERVICE_MODE=native` 启动 Loader，见 `doc/app_service/rust_app_loader.md` §9.3.1）没有 Agent 容器：`container` 记录 `exec = null`、`capabilities = {native_service: true}`、不是 `default`，宿主环境不被当作 `exec: native` 的 runtime。补建不覆盖 owner 改过的容器 `name / description / roots`；节点 runtime 的 roots 来自节点 `describe`（profile roots）。内建记录不能取消登记。
- **基线**（2026-10-25，primary-runtime TODO §2）：`baseline` 是登记者的声明——容器声明 `extended`（aios 镜像：标准工具 + 工具链与媒体工具），节点 runtime 取 owner 在 `AgentSettings.runtimes[id].baseline` 的声明（缺省 `standard`），`register` 的请求自带。`check` 在可用时核验标准工具（清单见 libopendan `baseline.rs` / `xagent runtime baseline`；容器查本进程 PATH 与程序目录，节点经执行器跑 `command -v` / PowerShell `Get-Command`），结果写 `baseline_check`：`missing` 为缺的命令，`error` 为检查本身没能完成的原因。它只是提示，从不改变 `baseline`。Loader 在投影（启动、settings 变化）后台对每个可用 runtime 做一次 check。版本 2 的记录读取失败，内建记录由宿主重写。
- **可用性**：`observe` 按配置判断——`exec` 为空 → `runtime_unavailable`（未启用 / 无执行器）、`container` 登记在别的宿主上 → `runtime_unavailable`；`container` 带 `native_service` → `runtime_unavailable`（原因：Agent Service 原生运行，没有容器，用 host）；节点 runtime 配置齐全即 `available`。`check` 对节点 runtime 另做真实探测（login + describe），失败写 `last_error`，成功时把节点 roots、os、identity_mode、desktop readiness 写进记录。
- **写**：`register`（owner 添加，同参数重试幂等，已存在的 id 拒绝）、`update`（CAS `expected_revision`，只改 `name / description / roots`）、`check`、`unregister`（CAS，内建拒绝）；`.locks/runtimes.lock` 互斥。`ensure_builtin` 只给宿主，kRPC 拒绝。
- **id**：小写字母、数字、`-`、`_`；`local` 保留为 `container` 的旧名，`lookup("local")` 返回 `container`。
- **使用者**：Workspace 位置的 `runtime_id` 是 `container`（旧记录的 `local`）或一个 `available` 的节点 runtime，目标侧目录操作经该 runtime 的执行器完成（[Agent Workspace Protocol](<Agent Workspace Protocol.md>)）；Session 创建时在 `api::freeze_runtime` 统一选择并冻结 session-runtime（显式 > 继承 > `default`；Workspace 不参与，它的 runtime 可与 session-runtime 不同），`create-worksession --runtime <id>` / Todo 的 `--runtime` 要求已登记且 `available`，否则拒绝。
- kRPC（宿主 opendan 另有）：`runtimes.settings`（owner 的选择：`runtimes`、`default_runtime_id`）、`runtimes.settings_set {runtime_id, enabled?, computer?, baseline?, default?}`（只改已有条目的启停 / ComputerUse / 声明的基线 / 默认，不新增目标；新增目标是 control-panel `agent.update` / `agent.runtime_directory` 的事）。
- kRPC：`runtimes.list`、`runtimes.lookup {runtime_id}`；写：`runtimes.register {request}`、`runtimes.update {runtime_id, request}`、`runtimes.check {runtime_id}`、`runtimes.unregister {runtime_id, expected_revision}`（署名为服务端看到的调用方）。
