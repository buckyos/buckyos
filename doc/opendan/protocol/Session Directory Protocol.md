# Session Directory Protocol

版本 5 · 2026-10-03 · 由 `libopendan` 反写（`src/protocol/{config,behavior,state,summary,worklog,misc,input}.rs`）。Round / Step / Turn 的定义见 [LLM Context readme](../../llm_context/readme.md)；本文的 Turn 规则均为当前实现。

## 1. 目录结构

```text
<any_parent>/<sid>/                  session 目录，目录名 = session_id；存放一次性产物
  readme.md                          人读：标题 / 目标 / 起源
  report.md                          人读：结束报告（finished 时写入）
  <产物…>                            一次性产物（相对路径引用）
  .opendan_agent_session/            session 状态：只由 session 锁的持有者写
    session_config.json              启动配置（整文件原子替换）
    state.json                       会话状态 = 提交点（整文件原子替换）
    worklog.jsonl                    严格只追加的历史
    summary.json                     历史压缩状态（首次压缩前可以不存在）
    static.json                      统计（可缺失）
    lease.json                       session 锁文件（见 Lease Protocol）
    binding.json                     runtime 绑定（只写一次）
    runs/<run_id>/                   xllm run 目录：run.json + snapshots/NNNN.json + .lock
  .runtime/
    bin/                             Session Bin：PATH 第一层（墓碑、会话辅助脚本、.manifest.json）
```

**位置无关**：

1. `.opendan_agent_session/` 中不出现 AgentRoot 路径；只通过 `agent_did` 引用 Agent。
2. 与主机相关的信息只在 `binding.json`（workdir）、`.runtime/` 和 `runs/`（run.json 的 `workdir`）中出现。
3. 没有被持有、没有进行中的 run 时，session 目录可以整体移动；移动后由驱动者更新登记表 `location`（`update_location`，`location_rev + 1`）。在登记表更新之前，runner 必须拒绝推进（登记表 `location` 与实际目录不一致 = 未登记）。
4. 附件引用可以是 session 目录内的相对路径，也可以是 NamedStore 对象 id；不允许绝对路径（`binding.json` 与 `runs/` 除外）。

## 2. session_id（全局唯一）

`H(args)` = 参数数组按紧凑 JSON 编码（UTF-8、无空白，如 `["work","did:bns:a","app:x@o","k"]`）的 SHA-256 完整小写 hex。

| kind | 规则 |
|---|---|
| work | `work-<yyyymmddThhmmss>-<uuid_v4 32hex>`（UTC 时间）；带幂等键时 `work-H("work", agent_did, creator_principal, key)` |
| self_improve | `si-<yyyymmddThhmmss>-<uuid>`；带幂等键时 `si-H("self_improve", agent_did, creator_principal, key)` |
| self_check | `sc-H("self_check", agent_did)` |
| ui | `ui-H("ui", agent_did, route_key)`；`route_key` 是该 Session 绑定的会话（OpenDAN 的 UI Session 用 msg-center 的 `MailboxAddress` 字符串）。未给 `route_key` 的 ui Session（本地对话）由创建函数补一个 `local:<幂等键或 uuid>`。宿主需要同一个 `route_key` 下的多代 Session 时显式传 sid（OpenDAN：`ui-H("ui", agent_did, route_key, <代次>)`） |

字符集：字母、数字、`_ - .`，不以 `.` 开头，长度 1..=200。显式传入的 sid 必须满足同样的约束并由调用方保证全局唯一。

## 3. session_config.json

Schema：`schema/session_config.schema.json`。要点：

```jsonc
{
  "schema": "opendan.session_config/5",
  "config_rev": 1,                    // 运行中可改的字段（订阅）变化时 +1
  "session": {
    "session_id": "...", "agent_did": "did:bns:jarvis.alice",
    "kind": "work | self_improve | self_check | ui", "class": "work",
    "created_at_ms": 0,
    "created_by": { "principal": "app:app2@alice", "via": "app" },
    "driver": { "principal": "app:app2@alice" },   // 推进身份，创建后不变
    "idempotency_key": null, "route_key": null,
    "origin": { "parent_session": null, "intent_ref": null, "reason_messages": [],
                "report": "final | progress | none", "created_by_call": "<run_id>/<call_id>" },   // Sub Session：父怎么收到汇报、创建它的调用
    "objective": "...",
    "end_condition": { "type": "llm_declares_done | output_schema | max_turns", "detail": {"n": 3} },
    "scope": { "objects": [], "paths": ["ws:snake/src/"] },     // 活动视图的初始声明
    "input_policy": "any | supplement_only | none",
    "acl": { "owner": null, "readers": [], "agent_access": "full | status_only" },
    "task_binding": null,
    "timezone": null,                               // 用户时区（IANA 名），与 Session 绑定；默认半订阅
    "policy": { "wait_user_msg": "allowed | finish_failed | finish_completed",   // Session 模板在创建时解析出的策略（缺省值时省略）
                "observe": "off | events | events_and_active", "load_hints": true,
                "max_process_depth": 4, "max_sub_sessions": 4, "max_session_depth": 2 }
  },
  "prompt": {
    "llm_context": { /* xllm .llm_context 的 JSON 形式（同 schema、严格键） */ },
    "behavior": "plan", "system": "...", "context": ["..."],
    "frozen": { "catalog_rev": "sha256:…", "frozen_at_ms": 0, "frozen_by": "app:xagent@alice",   // 从 Agent 的 behavior 目录冻结（schema behavior_config）
                "identity": { "role": "…", "self": "…", "i18n": {} },
                "behaviors": { "plan": { /* BehaviorConfig */ } } },
    "initial_inputs": [ /* 无输入队列的 Session：创建时给出的 msg 逻辑记录（≤ 64），发布后不变 */ ],
    "on_init": null, "on_input": null, "on_context_switch": null, "semi_subscription_snapshot": null,   // 输入模板；缺省内建
    "input": { "mode": "batch | single", "media": "reference | inline" },
    "mechanical_compress": { "recent_full_responses": 2, "summary_chars": 280, "max_result_chars": 4096,
                             "drop_kinds": ["created","decide","compaction","input_rejected","event_dropped","turn_ended"] },
    "history_budget_tokens": null, "compact_ratio": null
  },
  "runtime": { "requirement": { "runtime_id": null, "tools": ["node"], "app_tools": [] },
               "tool_plan": null, "env": {} },
  "workspace": null | { "kind": "agent", "id": "ws-1" } | { "kind": "external", "path": "/abs" },
  "artifact_id": null,
  "subscriptions": [ { "id": "s1", "mode": "semi | active",
                       "source": { "type": "session", "ref": "<sid>" } | { "type": "object_event", "object": "...", "event": "" }
                                 | { "type": "task", "task_id": "…" } | { "type": "timer", "name": "…" } | { "type": "system", "id": "…" },
                       "watch": ["run_state","outcome"] } ],
  "channels": {
    "inputs": [ { "kind": "kmsg", "id": "q", "queue": "<urn>", "subscriber": "opendan.<agent_id>.<sid>" } ],
    "outbound": null | { "to": "<对端或群的 DID>", "to_session": null, "kind": "chat" },  // 回复坐标，创建时固定
    "wake_event": "/opendan/<agent_id>/session/<sid>/input"
  },
  "extensions": { "<app_id>": {} , "opendan": { "behaviors": { "check": { "mode": "switch_context", "prompt": { "system": "…" } },
                                                           "research": { "mode": "fork" } },
                                            "perception_window": {...} } }
}
```

- `prompt.llm_context` 为 `null` 时，参考实现使用 `{"tools":{"enabled":true}}`（buckyos Provider、`llm.chat`、function_call、内置 bash 工具组）。工具预算键为 `max_tool_iterations`（工具迭代，不是推理次数），见 [xllm 协议](../../llm_context/local_llm_context_protocol.md) §9.2。
- `end_condition`：`llm_declares_done` / `output_schema` 在 `Done` 交付最终结果时 finished。`max_turns` 按**已完成**的 Turn（`turns_completed`）计：`Done` 交付最终结果时，算上本 Turn 已完成 `detail.n`（缺省 1）个则 finished，否则本 Turn 记为完成、session 等待下一输入。context 切换、子 context 的调用与返回等内部交接不是 Turn，不占额度。
- `mechanical_compress.recent_full_responses`：从最新往前，完整渲染的模型 response 个数；单位是一条记录下来的 response，即 behavior 的 `step` 或 function call 的 `assistant_message`（连同其后的条目），更早的条目截到 `summary_chars`。
- `extensions.opendan.behaviors.<name>`：目标 behavior 的进入配置（§8，schema `behavior_entry`）。进入模式由**目标**决定，没有缺省模式，也没有“同一 run 换 behavior”的普通切换：
  - `mode`（必填）：`switch_context | create_sub_context | fork`。
  - `prompt.system`：该 context 的应用 system prompt（替换会话的 `prompt.system`）；`llm_context`：按顶层键替换 `prompt.llm_context`（模型、工具、限额）。`fork` 不允许声明这两项（fork 保持调用方的 system 与配置）。
  - `prompt.on_init / on_input / on_context_switch / semi_subscription_snapshot`：该 context 的输入模板，按字段覆盖会话的同名模板；`input{mode, media}`：该 context 的消费策略（缺省用会话的）。见 [Session Input Protocol](<Session Input Protocol.md>) §6。
  - `inherit`：新 context 的历史来源，`none`（缺省）| `recent_dialogue`（宿主渲染的 `<session_history>`：摘要 + 近期 worklog 记录）| `steps`（调用方已完成的 Step，按结构化记录继承；仅 `create_sub_context`，且双方都是 behavior loop）。`fork` 总是继承分叉点的完整有效历史，不接受 `inherit`。
  - 会话的初始 behavior（`prompt.behavior`）没有自己的条目时，视为使用会话基础配置的 `switch_context` 目标。交接到其它没有条目的 behavior 是配置错误：当前 Turn 以 `failed` 结束（`last_error.kind = behavior_config`）；发生在子 context 内时作为失败结果交回调用方。
  - 表在每次推进开始时校验，非法（未知 mode、fork 带 system 等）则不做任何推理。旧的 `extensions.opendan.process_modes` 不再接受。
- 工具 `call_behavior({behavior, task})`：在 `prompt.llm_context.tools.tools` 中以 `{"name": "call_behavior"}` 显式装配后，context 可以用工具调用（或 behavior action）发起子 context（目标必须是 `create_sub_context` / `fork`）。只由会话自己的 runner 提供；xllm 不具备该工具，拒绝接手装配了它的 run。
- `extensions.opendan.perception_window`：self_improve session 要整理的感知窗口（见 Agent State Protocol §4）。
- `session.policy`（版本 5）：Session 模板（`work / ui / self_improve / self_check`，`agent.toml [session.<class>]` 可覆盖）在创建时解析的结果，随配置冻结。
  - `wait_user_msg`：`WAIT_USER_MSG` 在该 Session 的含义。`allowed`：Turn 保持打开等输入（已交付回复则完成该 Turn）；`finish_failed`：Turn 以 `failed` 关闭、Session `finished + failed`，`last_error.kind = needs_user_input`，问题写入 report（不影响工具 / task 等待）；`finish_completed`：按交付结果处理。
  - `observe`：`off` 不注入半订阅快照；`events` 注入快照；`events_and_active` 另在受控输入里给出活动 Session 列表。`load_hints`：是否装配 hints。
  - `max_process_depth`：`process_stack` 上 `caller` 帧的上限（子 context 嵌套）。`max_sub_sessions` / `max_session_depth`：同时未结束的子 Session 数与子 Session 的嵌套深度；创建时校验，超限拒绝。
- `session.origin.report`：父 Session 怎么收到这个子 Session 的汇报（由创建者选择）：`final` = 需要关注（等输入 / 等决定）与结束；`progress` = 另加进度；`none` 或缺省 = 不推送，父只能读登记表。父对子的关注是隐式的（按 `origin.parent_session` 查登记表），不需要订阅，父也不需要输入队列，见 [Session Input Protocol](<Session Input Protocol.md>) §10。
- `prompt.frozen`（版本 5）：Session 构造时从 Agent 的 behavior 目录（[Agent State Protocol](<Agent State Protocol.md>) §7）冻结的身份文本与 behavior 配置。范围 = 入口 behavior + `meta.next` 声明的闭包；之后目录的修改不影响该 Session。冻结同时为每个 behavior 生成 `extensions.opendan.behaviors.<name>`（`mode / prompt / input / llm_context / inherit`；入口 behavior 未声明进入模式时为 `switch_context`，其它 behavior 未声明是配置错误），应用提供的同名条目与之不同则拒绝。创建者读不到目录时留空，由驱动者在首次推进（绑定之后、任何推理之前）冻结并替换配置（`config_rev + 1`）；首次用到闭包之外的 behavior 时补冻结（`config_rev + 1`，worklog `control_applied{command: behavior_frozen}`）。有 `frozen` 的 Session 的 system 段只来自冻结材料与配置；既没有 `frozen` 又读不到目录时 RecoveryBlocked，不猜。未采用冻结的 Session 仍直接使用 `extensions.opendan.behaviors`。
- `prompt.initial_inputs`（版本 5）：没有输入队列的 Session 的首批输入。只允许 `msg` 记录；Runner 把它当作只读输入源 `_bootstrap`（index 从 1 起），按同样的路由、消费策略与 receipt 消费，`state.inputs["_bootstrap"]` 记录进度，确认是空操作。有输入队列的 Session 不使用它（投递到队列）。
- 输入队列是可选的：`channels.inputs` 为空的 Session 不能被投递（`post` / 队列型控制返回错误），stop 由驱动进程自己发起，等待工具 / task / 子 Session 仍靠轮询推进。

## 4. state.json（提交点）

Schema：`schema/session_state.schema.json`。

```jsonc
{
  "schema": "opendan.session_state/5",
  "rev": 17,                                   // 每次提交 +1
  "writer": { "runner_id": "rn-…", "principal": "app:app2@alice", "host": "host:…", "pid": 1234, "lock_epoch": 42 },
  "run_state": "created | ready | running | waiting | finished",
  "waiting_for": null | { "kind": "input | tool", "refs": ["<task_id>"], "deadline_ms": null },   // tool：由快照的挂起调用生成
  "outcome": null | "succeeded | failed | stopped",
  "acceptance": "n/a | pending | accepted | discarded",
  "result": null | { "answer": "...", "answer_ref": "report.md", "artifact_ref": {...}, "discard_report": {...} },
  "turn_seq": 5,                               // 最近打开的 Turn 编号（第一个输入批次之前为 0）
  "open_turn": null | { "index": 5, "run_id": "…", "input_seq": 3, "hook": "on_input",
                        "inputs": ["q#121"], "at_ms": 0 },   // 进行中的 Turn；(run_id, input_seq) = 打开它的输入批次
  "turns_completed": 4,                        // 以 completed 关闭的 Turn 数
  "current_behavior": "plan", "process_entry": "plan",
  "process_stack": [ { "entry": "plan", "role": "parked | caller", "run_id": "…",
                       // role = caller 时：它等待的子 context
                       "call": { "mode": "create_sub_context | fork", "behavior": "research",
                                 "trigger": { "kind": "behavior" } | { "kind": "tool", "call_id": "…", "task_id": "subctx:…" },
                                 "task": "…" },
                       "turns": [...], "flushed_message_count": 0, "flushed_step_index": 3,
                       "flushed_input_seq": 1, "flushed_epoch": 0, "applied_input_seq": 1,
                       "handover_at_ms": 0 } ],
  "bootstrap_done": true,
  "topic": { "title": "", "tags": [] },
  "live_run": null | { "run_id": "…", "turns": [ { "turn": 5, "inputs": ["q#121"], "events": ["obj:doc-1:8"],
                                                   "hook": "on_input", "input_seq": 3, "at_ms": 0 } ],
                       "applied_input_seq": 3, "flushed_message_count": 0, "flushed_step_index": 0,
                       "flushed_input_seq": 0, "flushed_epoch": 0, "process_entry": null, "handover_at_ms": 0 },
                       // flushed_message_count / flushed_step_index / flushed_epoch / handover_at_ms 为 0 时省略
  "last_run": "…",
  "worklog": { "committed_seq": 340, "committed_bytes": 1048576 },
  "inputs": { "q": { "acked_index": 118, "consumed_above": [121], "accepted": [120] } },
  "recent_keys": ["cymsg:…"],                  // 有界（256）去重缓存
  "subscription_cursors": { "s1": { "rev": 15, "view": {...} } },     // 只有拉模式（session 订阅）的游标
  "pending_events": [ /* 已接收、尚未注入的半订阅状态；见 Session Input Protocol §5 */ ],
  "reply": null | { "route": "message", "to": "…", "to_session": null, "kind": "chat", "reply_to": "cymsg:…", "tunnel": null }
                | { "route": "parent_session", "session_id": "…" },   // 默认回复路径
  "outbox": [ { "key": "<sid>:<turn>:<run_id>:<n>", "msg": { /* 完整的 MsgObject，含 created_at_ms */ }, "turn": 3,
                "status": "pending | sent | failed", "msg_id": null, "deliveries": [], "error": null,
                "attempts": 0, "updated_at_ms": 0 } ],                 // 出站消息，见下
  "watched_tasks": ["<task_id>"],              // run 结束后仍在跟踪的后台 task
  "perception_seq": 88, "reported_rev": 17,
  "activity": { "summary": "…", "touching": [ { "kind": "path", "ref": "ws:…", "mode": "write", "since_ms": 0 } ], "heartbeat_ms": 0 },
  "one_line_status": "…", "last_error": null,
  "pending_decision": null, "stop_requested": false,
  "internal_continuation": null, "process_result": null,
  "updated_at_ms": 0
}
```

规则：

- 同一次提交的其它内容（worklog 追加、report.md、run 快照、static）都**先于** state.json 写入；state.json 用原子替换发布。读者看到新的 `rev` 时，它引用的内容一定已存在。
- `worklog.committed_*` 总是等于提交时 worklog 的末尾；其后的内容是未提交尾部。
- `process_stack`：`parked` 帧是经 SWITCH_CONTEXT 离开、等待再次进入的 context；`caller` 帧是子 context 的调用方，子 context 返回时出栈。子 context 不做 SWITCH_CONTEXT，所以栈的形状总是若干 `parked` 帧之后跟调用链上的 `caller` 帧；最后一帧是 `caller` 时，`live_run`（或即将新建的 run）就是它的子 context。`caller` 帧最多 4 层。
- `process_result`：子 context 交回调用方的结果 `{behavior, result, status: ok | failed | needs_user_input, next_action_id, next_step_index}`；工具触发的调用另带 `call_id`（§8）。
- `handover_at_ms`：本 state 已提交的交接记录（run.json `handover.at_ms`）的时间戳；带相同时间戳的记录不是待办的转移（§7、§8）。
- `live_run` = 未结束的 run；`last_run` = 最后一次结束的 run（保留其 llm context 状态）；`process_stack[].run_id` = 挂起的 process 的 run。三者之外的 run 目录可以删除（§7）。
- `outbox`（出站，驱动者配置了出站 sink 时才产生）：Turn 关闭的那次提交里，由 `reply` 给出信封、sink 填内容，生成完整的 MsgObject 连同幂等键写入，`status = pending`；`created_at_ms` 在这里定死。提交之后驱动者把 `pending` 条目按序交给 sink：成功 → `sent`（记 `msg_id` / `deliveries`）；被拒 → `failed`（记 `error`，不重试）；没送到 → 保持 `pending`，`attempts + 1`，退避后由之后的 drive 原样重发（同一个键、同一个 MsgObject，ObjId 不变）。发送失败不让 Turn 失败，也不阻塞后续输入；排在它后面的条目等它。`channels.outbound` 存在而 `reply` 的 `to / to_session / kind` 与它不一致时，条目直接记为 `failed`（`route_mismatch`），不发送。有父 Session 的 Session、以及回复对象是 Agent 自己时不产生条目。已结束（`sent` / `failed`）的条目保留最近 16 条供展示。
- `run_state` 迁移：`created → ready ⇄ running ⇄ waiting → finished`；进入 finished 后不能回到 running。stop / decide 只能由驱动者执行，其它参与方投递 control。

**逻辑 Turn**（当前实现）：Turn = AgentSession 的一次逻辑 Input → result，与 run、`LLMContext` Outcome、输入批次 `(run_id, input_seq)` 都不一一对应，不能用 run 数或 Outcome 数推算 Turn 数。

| 规则 | 行为 |
|---|---|
| 打开 | 没有打开的 Turn 时提交的输入批次打开新 Turn（`turn_seq + 1`，receipt `opens_turn = true`）：bootstrap 的 `on_init`、msg / Input event 的 `on_input` |
| 继续 | SWITCH_CONTEXT、子 context 的调用与返回（behavior 触发的交接是 `on_context_switch` 输入批次；工具触发的返回是该调用的工具结果，没有输入批次）、半订阅快照（它是受控输入批次的一部分，不独立计 Turn）、可恢复挂起（未要求 stop 的 Interrupted、可重试的 Runtime / 暂时性错误使 run paused、ContextLimitReached、PendingTool）、上下文上限的 history epoch 重写、重启与崩溃恢复都不改变 `open_turn`；Turn 打开期间消费的 msg / event 加入它（`open_turn.inputs` 追加） |
| 关闭 | 只由 session 决定（run 结束时的 `finish_run`，或 stop），与 `open_turn = null`、worklog `turn_ended` 同一次提交：`Done` 交付结果 → `completed`（再按 `end_condition` 判定是否 finished）；`WAIT_USER_MSG` 只在本 run 已交付答复（快照 `last_report` 非空，或最后一个 Step 带 `<sendmsg>`）时 `completed`，否则 Turn 保持打开，下一条输入加入它；不可重试错误 → `failed`；预算耗尽 → `budget_exhausted`；`control(stop)` → `stopped`。fork 子 process 结束（`process_done`）不关闭 Turn |

- 条目归属的 Turn：有 `open_turn` 时为其 `index`，否则为 `turn_seq`。`turns_completed` 只计 `completed`。
- Sub AgentSession 是另一个 session，有自己的 Turn；父子 session 的 Turn 不合并计数。父 Session 将要成功结束、而仍有 `origin.report` 不为 `none` 的子 Session 未结束（或其结束尚未交付给父）时不结束：run 结束，Turn 保持打开，`waiting_for = {kind: children, refs: [sid…]}`；子的结束作为受控输入并入同一个 Turn。创建、汇报与等待见 [Session Input Protocol](<Session Input Protocol.md>) §10。

**static.json**（统计，可缺失；`schema/session_static.schema.json`）：`{input_tokens, output_tokens, total_tokens, rounds, rounds_failed, rounds_interrupted, turns, runs, tool_calls, busy_ms, cost, updated_at_ms}`。

- `rounds`：本 runner 经 run 的 `LlmClient::infer` 发起的推理尝试数（成功、失败、中断都计；provider adapter 内部的重试算一次），每个 outcome 后累加；`rounds_failed` / `rounds_interrupted` 是其中返回错误 / 被中途放弃的部分。历史摘要（压缩）推理不是 Round，不计入；xllm 接手 run 后发起的推理只计入该 run 的 `run.json usage.llm_requests`（§7）。
- `turns` = 已完成的 Turn 数（`state.turns_completed`）；`runs` = 已结束的 run 数。Round、Turn、run 三个计数互不推导。

**report.md**（finished 时写入）：标题 `# Report — <objective 首行>`，随后 `- session:`、`- outcome:`、`- turns: N`（已完成的 Turn 数）和答复正文。

## 5. worklog.jsonl

每行一条 `{"seq": n, "t": "<kind>", …}`，`seq` 从 1 开始严格 +1。Schema：`schema/worklog_entry.schema.json`。

| t | 写入时机 | 字段 |
|---|---|---|
| created | 创建 session（第 1 条） | session_id, kind, by, objective, at_ms |
| turn_started | run 结束 / 挂起 / 中途重写时批量写 | run_id, turn, input_seq, inputs[{src,index,key,kind}], events[]（本批快照注入的半订阅状态版本的 key）, hook, at_ms：打开 Turn 的输入批次 |
| input_batch | 同上 | run_id, turn, input_seq, inputs[], events[], hook, at_ms：加入已打开 Turn 的批次（交接、fork 返回、补充输入） |
| user_message | 同上 | run_id, turn, content：批次的每条消息一条（半订阅快照在前，受控输入在后），只记文本块 |
| assistant_message | 同上（只用于 function call run） | run_id, turn, assistant, tool_calls[{call_id, tool, args(键排序), effect}]：一次模型 response（一个 Round）及其原生工具调用；不是 Step |
| step | 同上（只用于 behavior run） | run_id, turn, step_index, behavior?, assistant, actions[{call_id, tool, args(键排序), effect}], correction?（解析失败 / 策略拒绝产生的合成纠错 Step 为 true，缺省 false） |
| action_result | 同上 | run_id, turn, call_id, status(ok/error/unresolved/cancelled/pending), result：原生工具结果（function call，`[not executed]` / `[unresolved…` 记为 unresolved，`[cancelled]` 记为 cancelled）或 action 结果（behavior） |
| outcome | run 结束 / 挂起 / 中途重写 | run_id, turn, kind（run 结束：done/wait/process_done/budget/error/stopped；process 挂起：suspended；中途重写：context_rewritten）, next_behavior?, report?。保留 run 的结果（PendingTool、上下文上限暂停、中断、可重试错误）不写 outcome |
| turn_ended | 关闭 Turn 的同一次提交 | run_id（stop 时没有 run 则为空串）, turn, status(completed/failed/budget_exhausted/stopped), at_ms |
| compaction | 生成新 summary.json | summary_start_seq, made_by |
| decide | 应用 decide | decision, by, report |
| input_rejected | 输入被拒绝但标记为已消费 | input{src,index,key,kind}, reason（拒绝原因名）, detail? |
| event_dropped | 事件被消费但不进入上下文也不进 `pending_events` | input, reason（`unsubscribed`：没有匹配的有效订阅；`pending_call`：只用于唤醒挂起调用的等待） |
| control_applied | 应用 control | input, command, detail |

- **身份**：输入批次 = `(run_id, input_seq)`，Turn = `turn`（session 内编号），behavior Step = `(run_id, step_index)`，工具调用 / action = `call_id`。fork 子 run 延续父 process 的 step 编号（继承的 step 不重写），independent context 有各自的 run 与 step 编号，所以 `step_index` 在 session 内不全局唯一。`outcome` 记录 session 对一次运行结果的解释，不等于 Turn 结束；Turn 关闭只看 `turn_ended`。
- **只追加**：已提交部分从不改写；唯一允许的修改是恢复时截掉 `committed_bytes` 之后的尾部（先于读取任何新输入）。
- **run 进行中的历史只在 runs/**：run 结束或挂起时，从快照一次性生成尚未写入的条目追加（一次 fsync），随后提交 state。
- **文件顺序 = 提交顺序**：run 进行中应用的 control 等条目在发生时追加，因此会出现在该 run 的历史之前。
- **读取**：运行时只从 `committed_bytes` **反向**读（按块，由新到旧），读到 `summary.start_offset` 或预算用尽即停；正向读取只用于压缩的有界区间和审计。

## 6. summary.json 与下一次 llm_context

```jsonc
{ "schema": "opendan.session_summary/2", "history_summary": "…", "start_seq": 301, "start_offset": 912384,
  "mechanical": { ...同 prompt.mechanical_compress... }, "renderer": "libopendan.mechanical/2",
  "made_at_seq": 318, "made_by": "context_limit | ratio | manual", "updated_at_ms": 0 }
```

- 缺失时等价于 `start_seq = start_offset = 0`、无摘要、`mechanical = session_config.prompt.mechanical_compress`。
- 下一次 llm_context（没有 live_run 时）= system（身份 + 不可覆盖约束 + 应用 prompt + context + objective + xllm 能力/协议段）+ 一条历史消息（`<session_history>`：摘要 + 起点之后按机械压缩渲染的记录）；本次的输入由 run 的第一个输入批次（`<session_input hook=… time=…>` 消息 + receipt）注入。
- 机械渲染（`libopendan.mechanical/2`）：`turn_started` → `── turn N (hook) inputs: … ──`，`input_batch` → `── <hook> inputs: … ──`（如 `── on_context_switch ──`），`user_message` → `[input] …`，`assistant_message` → `[assistant] …`，`step` → `[step 3 plan] …` / `[step 3 plan correction] …`，`action_result` → `[result #<call_id> <status>] …`，`outcome` → `[outcome <kind> → <next_behavior>] …`，`turn_ended` → `── turn N completed ──`（默认在 `drop_kinds` 中，不渲染）。
- 反向读取时预算先于起点耗尽：必须先压缩（新起点 = 已保留的最旧一条，摘要覆盖 `[旧起点, 新起点)`），摘要与原始记录之间不能留空洞。
- 压缩只写 summary.json，并追加一条 `compaction` 条目后提交，从不改写 worklog。
- 确定性只在同一 `renderer` 版本内承诺；时间等新鲜量只出现在每个输入批次的消息里。

## 7. runs/（xllm run 目录）

```text
runs/<run_id>/run.json            xllm RunRecord（version 5）
runs/<run_id>/snapshots/NNNN.json LLMContextSnapshot（snapshot_version 4；先 fsync 再发布）
runs/<run_id>/.lock               run 执行锁（长期持有的 flock）
runs/<run_id>/exec/<call_id>/     shell 命令的执行目录：command、stdout、stderr、exit（命令结束时写）
```

`run_id` = `YYYYMMDD-HHMMSS-<6hex>`。宿主装配的 run 在 RunRecord 中增加（均可缺省）：

| 字段 | 含义 |
|---|---|
| `host` | `{assembled_by:"libopendan", session_id, runtime_kind, runtime_id, env_check}`；xllm 按保存的实际 runtime/target/cwd 接手，核验 Session PATH、环境、bin manifest/helper 内容；凭据只保存环境引用 |
| `host_commit_pending` | 宿主输入提交门槛（批次号）。非空时任何执行者都不得推理或调用工具，xllm 拒绝接手 |
| `inflight[]` | 已派发、结果尚未随快照持久化的动作：`{call_id, tool, args, effect, started_at_ms}`。没有进程身份：恢复时不核验、不停止进程，按 runtime 从执行目录读到什么就说什么（长命令 TODO §3.2） |
| `host.extra.finish` | 宿主的结束决定：与终态 `status` 在**同一次** run.json 写入中记录（`{kind: done|wait|process_done|budget|error|stopped, finished, outcome, waiting, answer, error, usage, turn_end}`；`turn_end` 为本次结束关闭 Turn 的状态，空表示 Turn 继续）。结束流程中途崩溃时，恢复按它重做，而不是重新推断 |
| `handover` | run 停在 behavior 交接点：`{next_behavior, at_ms}`，与让出时的快照、`status = paused` 在**同一次** run.json 写入中记录（run.json 版本 5）。非空时任何执行者都不得在该 run 上继续推理：xllm 遇到宿主 run 的 `Done{next_behavior=B}` 只记录它（不记 `completed`），`xllm --resume` 拒绝接手；由会话读取 B 的进入模式并提交转移。会话重新打开该 run 执行（status 回到 `running`）时清除 |
| `usage.llm_requests` | 本 run 的推理尝试数：libopendan 在每个 outcome 后加上本段经 run 的 `LlmClient::infer` 发起的 Round 数（含失败 / 中断），xllm 接手后在其上继续累加；任何执行者都不重置或覆盖 |

快照 `state.host["libopendan"]`（HostMeta）：`{session_id, base_input_len, process_entry, inherited_below, input_receipts[], history_epoch?, epoch_turn?, epoch_input_seq?}`，各执行者必须原样保留。后三项在第一次中途重写后出现（为 0 时省略）：`history_epoch` 是本 run 已发生的中途重写次数，`epoch_turn` 是当前 epoch 开始时所在的 Turn（新 epoch 中先于任何 receipt 的条目归属于它），`input_seq ≤ epoch_input_seq` 的 receipt 属于更早的 epoch（身份仍有效，位置不再适用）。

**run 生命周期**：

```text
建立：   create run dir + 持 run 锁 → run.json（status running，无快照）
输入批次：注入消息 + receipt → ① 快照 fsync → ② run.json(latest_snapshot_idx, host_commit_pending=seq)
         → ③ state.json（live_run.turns、open_turn / turn_seq、消费位置、订阅游标）→ ④ 清 host_commit_pending
         → ⑤ 确认输入源（④ 之前不允许推理 / 工具；参考实现的故障注入点
         input_batch:after_input_checkpoint / after_state_commit / after_gate_clear 分别在 ②③④ 之后）
进行中： 检查点（CheckpointHook；function call：每次推理前，即一批工具结果之后；
         behavior：每个 Step 的 do-action 之后，不在 Step 内层推理前）
         快照 fsync → run.json 发布指针并只清除快照已覆盖的 inflight
结束：   ① 结果快照 + run.json（终态 status 与 host.extra.finish 同一次写入）
         → ② 生成 worklog 条目追加（fsync；关闭 Turn 时末尾为 turn_ended）→ ③ state.json（live_run=null、
         last_run=本 run；关闭 Turn 时 open_turn=null 并更新 turns_completed；子 context 结束时
         同一次提交里调用方的 run 重新成为 live_run）→ ④ 删除不再被引用的旧 run（持其锁、确认无未核对执行）
交接：   （§8）① 快照 + run.json（status paused；next_behavior 交接同时写 handover，工具触发的调用
         快照本身停在 PendingTool）→ ② 追加该 run 已产生的历史与 outcome(suspended) → ③ state.json
         （run 入 process_stack、目标成为当前 behavior、internal_continuation）。③ 是提交点：之前崩溃，
         恢复时由 handover 记录 / 挂起的调用重做同一次转移，不重新推理；之后 run 不再是 live_run，
         不会重复进入目标。终态 status 只表示 run 已结束。
重写：   （上下文上限，见下）① 追加本 run 未写入的历史 + outcome(context_rewritten) 并提交 flush 标记
         → ② 压缩 summary.json → ③ 以 system + 会话历史重建的上下文发布快照（history_epoch + 1）
```

**上下文上限中途重写**（llm_context X7）：run 以 `ContextLimitReached` 让出时，runner 不在快照里重写历史，而是把它交还 session 历史：

1. 用 flush 标记写入本 run 已产生而未写入的历史，末尾追加 `outcome(kind=context_rewritten)`，与 flush 标记同一次提交 state.json（没有新历史时两者都不写，重做不会重复）。
2. 压缩 session 历史：起点前移到只剩约 `history_budget >> 第几次` token 的原始记录（写 summary.json 与 `compaction` 条目），再按 §6 重建历史消息。
3. 以 `system + 历史消息` 作为新的输入恢复上下文（function call：`RewrittenHistory`；behavior：`RewrittenSteps`，step 全部折叠进输入，编号继续），HostMeta 进入新的 epoch，先发布快照（run.json `status=running`）再继续推理。

一次推进内连续最多 3 次；仍装不下时 run 以 `paused` + `last_error.kind=context_limit` 保留挂起快照，下一次推进先重写再继续。① 之后、③ 之前崩溃：恢复用旧 epoch 最后发布的快照，再次让出、重写，已写入的部分由 flush 标记跳过；③ 之后崩溃：新 epoch 的计数从 0 开始。receipt 的 `input_seq`、消费位置与打开的 Turn 都不随重写改变。

**保留**：`live_run`、`last_run`、`process_stack[].run_id` 引用的 run 保留；其它 run 在确认没有未核对执行后删除；旧快照按需裁剪（保留最新几份与已发布指针）。
- `host.extra.tasks`（宿主字段）：run 的工具结果里引用过、LLM 还没有看到其结束的后台 task id。结果首次引用一个 task（且不是一条已结束命令的结果）或说明它“still running”时加入；之后关于它的其它结果（它的结束）移除。run 结束而 Session 未 finished 时，这些 id 与当时 resolver 报告仍在运行的 task 一起记入 `state.watched_tasks`；run 的结束在崩溃后被重做时同样从这里取，不依赖见过这个 run 的进程。

## 8. behavior process 与 run

同一时刻只推进一个 run。模型用 `next_behavior = B`（behavior Step）或 `call_behavior({behavior: B, task})`（工具 / action）指定目标，会话读取 **B 的进入模式**（§4.2 `extensions.opendan.behaviors`）决定如何调度：

| 进入模式 | 当前 run | 目标 run |
|---|---|---|
| `switch_context` | 挂起为 `parked` 帧 | B 有 `parked` 的 run 就恢复它（它自己的 system、工具、历史、编号与剩余预算）；否则按 B 的配置新建：B 自己的 system / 工具 / 模型，历史只按 `inherit` 装配，不带入其它 context 的快照。B 的 `END` 按会话结束条件处理，不会自动回到上一个 behavior |
| `create_sub_context` | 挂起为 `caller` 帧 | 每次新建子 run：B 自己的 system 与配置 + 任务输入，父历史按 `inherit` 显式选择（`llm_context::derive_child`）；结束后结果交回调用方 |
| `fork` | 挂起为 `caller` 帧 | 每次新建子 run：调用方的配置、system 与分叉点的完整有效历史（`llm_context::fork_snapshot`），再追加任务输入；结束后结果交回调用方 |

没有“同一个 run 换 behavior / system”的切换。`switch_context` 只能由 `next_behavior` 触发；两种子 context 可以由 `next_behavior` 或 `call_behavior` 触发：

| 触发 | 调用方的停止点 | 子结果返回 |
|---|---|---|
| `next_behavior = B` | 完整 Step 的 `Done` | `process_result` 渲染进调用方的下一个 `on_context_switch` 输入批次（`<process_result behavior status>`） |
| `call_behavior` | `PendingTool`（task id `subctx:<call_id>`），未完成的批次 / Step 留在调用方快照 | 调用方的 run 被打开时用 `ResumeFill::ToolResults` 按 `call_id` 回填，然后续派同批余下的调用；不产生输入批次。`failed` 回填为工具错误，`needs_user_input` 回填为结构化结果 |

子 context 的规则：

- 子 run 写入 `runs/`，进入时的交接批次（`on_context_switch`）带 `<sub_task mode="…">任务</sub_task>`；它继承的部分不由它写入 worklog（function call：`request.input` 即继承的前缀；behavior：`HostMeta.inherited_below` 之下的 step），step / action 编号接着调用方的，调用方恢复时编号不小于子 run 的，保证 `(run_id, step_index)` 与 `call_id` 唯一。预算、用量、错误计数与输入 receipt 独立；调用方恢复时不重置自己的预算。
- fork 的分叉点：调用方没有未完成调用时是它的全部历史；工具触发时是触发批次（function call）/ 进行中的 Step（behavior）之前的最后一个已配对前缀，触发批次只留在调用方。
- 子 context 无论以什么结束都返回调用方，**在子 run 结束的同一次提交里**出栈、调用方的 run 成为 `live_run`、写入 `process_result`：`END` / 交付结果 → `ok`；`WAIT_USER_MSG` → `needs_user_input`（子 context 不消费调用方的输入队列，由调用方去问用户）；不可重试错误、预算耗尽、交接到没有进入模式的 behavior → `failed`（不结束 Turn）；交接到 `switch_context` 目标也只是返回。子 context 可以再调用子 context，`caller` 帧最多 4 层，超出时调用失败。
- 子 run 新增的记录进入 worklog 供审计，但不进入调用方的上下文：为其它 context 重建会话历史（§6）时，已返回的子 run（有 `process_done` outcome）只渲染这条 outcome（即交回的结果），不渲染它的过程。压缩时同样过滤，但压缩只识别被压缩片段内的 `process_done`：切点把子 run 的记录与它的 `process_done` 分开时，切点之前的那部分仍会进入摘要（当前实现的限制）。

交接都发生在同一 Turn 内：交接批次（`input_batch`，receipt `opens_turn = false`）加入打开的 Turn，子 context 结束（`process_done`）也不关闭它。

挂起时先写入该 run 已产生而未写入的历史（`flushed_message_count` 或 `flushed_step_index`，以及 `flushed_input_seq` 前移），worklog 因此保持时间顺序。

**flush 标记**（`live_run` / `process_stack[]` 中，两种 run 各用一个游标）：function call run 以位置计数（`flushed_message_count` = prefix 之后已写入的消息数，只在 `flushed_epoch == HostMeta.history_epoch` 时有效；快照的 epoch 更新时从 0 计，且只用 `input_seq > epoch_input_seq` 的 receipt 定位消息）；behavior run 以身份计：`step_index < flushed_step_index` 的 step（身份高水位，不是数量：子 run 继承的 step 不由它写入）与 `input_seq ≤ flushed_input_seq` 的注入消息已写入，不受中途重写影响。只有已完成的 Step（`steps` / `last_step`）会写入，仍在派发 action 的 `action_step` 完成后才写。注入消息按 receipt 的位置排序：`request_input` 位于 `after_step` 之前，`step` 位于所附 step 之后。

## 9. binding.json 与 .runtime/bin

```jsonc
{ "schema": "opendan.binding/3", "runtime_id": "rt-…", "kind": "native | tmux", "target": {"host":"host:…", "uid":"1000"}, "workdir": "/abs", "bound_at_ms": 0, "bound_by": "rn-…" }
```

- 首次推进时用不覆盖发布写入（`link` / `renameat2(NOREPLACE)`），之后不变；runtime_id 不同必须拒绝（RuntimeMismatch），发生在任何推理之前。
- workdir：显式 runtime.workdir 优先；否则有 workspace 用 workspace（agent 内部 workspace 为 `<agent_root>/workspace/<id>`），无 workspace 为 session 目录。
- binding.json 核验打开后的 runtime_id、kind、target、workdir，不只比较声明的 kind；每次推进都要幂等修复并核验环境：`.runtime/bin` 按期望集合（tool_plan 墓碑 + `agent-session` 包装脚本）重写，`.manifest.json`（条目 → sha256）最后写入；核验失败（缺文件、内容不符、不可执行）不得推理，下次推进修复。
- 墓碑：`#!/bin/sh` 输出 `{"blocked_by":"tool_plan",...}` 与人读说明到 stderr，`exit 127`。
- **tmux runtime 属于 Session**：`runtime_id`（配置、run 的 runtime descriptor、binding 三处）等于 session_id；tmux session 名由 session_id 导出（`od_` + 非 `[A-Za-z0-9_]` 字符替换为 `_`）。配置、`runtime.requirement.runtime_id` 或显式参数给出别的 id / 名称是配置错误，在创建目标和推理之前拒绝。首次绑定时目标不存在则创建、已存在则复用；已有 binding 时只 attach 绑定记录的目标，目标丢失或身份改变按绑定错误停止，不以同名新目标继续旧执行。目标上的 tmux 选项 `@opendan_session` 记录所属 session_id：两个 session_id 的名称规整后相同，后者被拒绝。同一 Session 的全部 Turn、behavior、子 context、fork 与恢复使用这一个目标；子 Session 继承的是配置，按自己的 session_id 得到自己的目标。创建 Session 不创建目标；run / Turn 结束与宿主退出不销毁它。

共享 Runtime 来自 agent_tool::runtime，执行实现不再保留在 Session 层；Session 仅装配 bin/helper 与通用环境。prompt.llm_context.runtime 是构造配置，runtime.requirement 是绑定要求；推理、工具及旧执行恢复前先核验 binding。control workdir（run.json.workdir）与执行 cwd（config.runtime.workdir）分开保存。Session helper 未部署到远端时 remote_ssh 报 Capability，不把本地 bin 路径放进远端 PATH。

## 10. 恢复顺序（drive 开头）

1. 取 session 锁（身份必须等于 `session.driver.principal`）；校验 schema 主版本（`state.json` 必须是 `/4`、`session_config.json` 必须是 `/3`，否则 RecoveryBlocked；不迁移，也不按旧字段读取）。
2. 登记表 `location` 必须等于本目录，否则不推进。
3. 截掉 worklog 未提交尾部（文件短于 `committed_bytes` → RecoveryBlocked）。
4. 打开 runtime 并核验完整 binding 与 bin/helper 环境；删除未被引用的 run（持锁；记录不可读则保留）。命令留下的进程不归 run 管，不核对也不停止。
5. live_run：持 run 锁（拿不到 → RunBusy）；读 run.json 与已发布快照（缺失 / 损坏 / 版本不支持：run.json `version ≠ 5` 或快照 `snapshot_version ≠ 4` → RecoveryBlocked，保留现场）；把没有结果的 `inflight[]` 物化为“被打断、结果未知”（文本由 runtime 按执行目录生成）；校验快照 receipt（批次 1..n 连续，state 已应用的批次必须在快照中）并补交 `input_seq > applied` 的 receipt 到 state（只补元数据，按 `opens_turn` 打开或加入 Turn，不重新追加消息）；门槛非空且 state 已覆盖 → 清门槛；run 已到终态 → 按 `host.extra.finish` 重做结束（没有记录时——例如 xllm 跑完——从最终快照推断：behavior 取最后一个 step 的 next_behavior 与 report，function call 取最后一条 assistant 文本）。run 停在未提交的交接点（run.json `handover.at_ms ≠ live_run.handover_at_ms`，或快照挂起在 `call_behavior` 而 state 还没有它的结果）→ 按 §8 提交这次转移（挂起入栈 / 子 context 返回），不在该 run 上推理。
6. 重试确认已提交的输入位置；补发登记表回报与感知。
7. 读取新输入、应用 control；finished 则拒绝剩余普通输入。
8. 恢复 live run（在途动作物化为“结果未知”并先持久化），继续推进。快照挂起在子 context 调用上且 `process_result` 带着它的 `call_id` → 回填工具结果、发布快照、清除 `process_result` 后继续（回填后崩溃：快照已不再挂起，只清除 `process_result`）；挂起在其它 deferred 工具结果（本 runner 无法提供）→ RecoveryBlocked；挂起在上下文上限 → 先按 §7 重写再继续。

RecoveryBlocked 时只在 `state.last_error` 记录原因并回报，保留 live_run、run 目录、消费位置与执行证据，不自动放弃。
