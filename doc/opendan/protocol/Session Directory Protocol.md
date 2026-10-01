# Session Directory Protocol

版本 2 · 2026-10-01 · 由 `libopendan` 反写（`src/protocol/{config,state,summary,worklog,misc,input}.rs`）。Round / Step / Turn 的定义见 [LLM Context readme](../../llm_context/readme.md)；本文的 Turn 规则均为当前实现。

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
| ui（后移） | `ui-H("ui", agent_did, route_key)` |

字符集：字母、数字、`_ - .`，不以 `.` 开头，长度 1..=200。显式传入的 sid 必须满足同样的约束并由调用方保证全局唯一。

## 3. session_config.json

Schema：`schema/session_config.schema.json`。要点：

```jsonc
{
  "schema": "opendan.session_config/2",
  "config_rev": 1,                    // 运行中可改的字段（订阅）变化时 +1
  "session": {
    "session_id": "...", "agent_did": "did:bns:jarvis.alice",
    "kind": "work | self_improve | self_check | ui", "class": "work",
    "created_at_ms": 0,
    "created_by": { "principal": "app:app2@alice", "via": "app" },
    "driver": { "principal": "app:app2@alice" },   // 推进身份，创建后不变
    "idempotency_key": null, "route_key": null,
    "origin": { "parent_session": null, "intent_ref": null, "reason_messages": [] },
    "objective": "...",
    "end_condition": { "type": "llm_declares_done | output_schema | max_turns", "detail": {"n": 3} },
    "scope": { "objects": [], "paths": ["ws:snake/src/"] },     // 活动视图的初始声明
    "input_policy": "any | supplement_only | none",
    "acl": { "owner": null, "readers": [], "agent_access": "full | status_only" },
    "task_binding": null
  },
  "prompt": {
    "llm_context": { /* xllm .llm_context 的 JSON 形式（同 schema、严格键） */ },
    "behavior": "plan", "system_prompt": "...", "context": ["..."],
    "mechanical_compress": { "recent_full_responses": 2, "summary_chars": 280, "max_result_chars": 4096,
                             "drop_kinds": ["created","decide","compaction","input_rejected","change_dropped","turn_ended"] },
    "history_budget_tokens": null, "compact_ratio": null
  },
  "runtime": { "requirement": { "runtime_id": null, "tools": ["node"], "app_tools": [] },
               "tool_plan": null, "env": {} },
  "workspace": null | { "kind": "agent", "id": "ws-1" } | { "kind": "external", "path": "/abs" },
  "artifact_id": null,
  "subscriptions": [ { "id": "s1", "mode": "semi | active",
                       "source": { "type": "session", "ref": "<sid>" } | { "type": "object_event", "object": "...", "event": "..." },
                       "watch": ["run_state","outcome"] } ],
  "channels": {
    "inputs": [ { "kind": "kmsg", "id": "q", "queue": "<urn>", "subscriber": "opendan.<agent_id>.<sid>" } ],
    "outbound": null,
    "wake_event": "/opendan/<agent_id>/session/<sid>/input"
  },
  "extensions": { "<app_id>": {} , "opendan": { "process_modes": { "research": "fork" }, "perception_window": {...} } }
}
```

- `prompt.llm_context` 为 `null` 时，参考实现使用 `{"tools":{"enabled":true}}`（buckyos Provider、`llm.chat`、function_call、内置 bash 工具组）。工具预算键为 `max_tool_iterations`（工具迭代，不是推理次数），见 [xllm 协议](../../llm_context/local_llm_context_protocol.md) §9.2。
- `end_condition`：`llm_declares_done` / `output_schema` 在 `Done` 交付最终结果时 finished。`max_turns` 按**已完成**的 Turn（`turns_completed`）计：`Done` 交付最终结果时，算上本 Turn 已完成 `detail.n`（缺省 1）个则 finished，否则本 Turn 记为完成、session 等待下一输入。behavior 切换、fork、independent 等内部交接不是 Turn，不占额度。
- `mechanical_compress.recent_full_responses`：从最新往前，完整渲染的模型 response 个数；单位是一条记录下来的 response，即 behavior 的 `step` 或 function call 的 `assistant_message`（连同其后的条目），更早的条目截到 `summary_chars`。
- `extensions.opendan.process_modes`：behavior 名 → `fork | independent`（§8）；未列出的 behavior 为普通切换。
- `extensions.opendan.perception_window`：self_improve session 要整理的感知窗口（见 Agent State Protocol §4）。

## 4. state.json（提交点）

Schema：`schema/session_state.schema.json`。

```jsonc
{
  "schema": "opendan.session_state/2",
  "rev": 17,                                   // 每次提交 +1
  "writer": { "runner_id": "rn-…", "principal": "app:app2@alice", "host": "host:…", "pid": 1234, "lock_epoch": 42 },
  "run_state": "created | ready | running | waiting | finished",
  "waiting_for": null | { "kind": "input | tool", "refs": [], "deadline_ms": null },
  "outcome": null | "succeeded | failed | stopped",
  "acceptance": "n/a | pending | accepted | discarded",
  "result": null | { "answer": "...", "answer_ref": "report.md", "artifact_ref": {...}, "discard_report": {...} },
  "turn_seq": 5,                               // 最近打开的 Turn 编号（第一个输入批次之前为 0）
  "open_turn": null | { "index": 5, "run_id": "…", "input_seq": 3, "hook": "on_wakeup",
                        "inputs": ["q#121"], "at_ms": 0 },   // 进行中的 Turn；(run_id, input_seq) = 打开它的输入批次
  "turns_completed": 4,                        // 以 completed 关闭的 Turn 数
  "current_behavior": "plan", "process_entry": "plan",
  "process_stack": [ { "entry": "plan", "mode": "fork | independent", "run_id": "…",
                       "turns": [...], "flushed_message_count": 0, "flushed_step_index": 3,
                       "flushed_input_seq": 1, "flushed_epoch": 0, "applied_input_seq": 1 } ],
  "bootstrap_done": true,
  "topic": { "title": "", "tags": [] },
  "live_run": null | { "run_id": "…", "turns": [ { "turn": 5, "inputs": ["q#121"], "changes": ["s1@16"],
                                                   "hook": "on_wakeup", "input_seq": 3, "at_ms": 0 } ],
                       "applied_input_seq": 3, "flushed_message_count": 0, "flushed_step_index": 0,
                       "flushed_input_seq": 0, "flushed_epoch": 0, "process_entry": null },
                       // flushed_message_count / flushed_step_index / flushed_epoch 为 0 时省略
  "last_run": "…",
  "worklog": { "committed_seq": 340, "committed_bytes": 1048576 },
  "inputs": { "q": { "acked_index": 118, "consumed_above": [121], "reading": [] } },
  "recent_keys": ["msg:…"],                    // 有界（256）去重缓存
  "subscription_cursors": { "s1": { "rev": 15, "view": {...} }, "s2": { "key": "…", "version": "…" },
                            "_active_sessions": [ { "id": "…", "state": "running", "overlap": [] } ] },
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
- `live_run` = 未结束的 run；`last_run` = 最后一次结束的 run（保留其 llm context 状态）；`process_stack[].run_id` = 挂起的 process 的 run。三者之外的 run 目录可以删除（§7）。
- `run_state` 迁移：`created → ready ⇄ running ⇄ waiting → finished`；进入 finished 后不能回到 running。stop / decide 只能由驱动者执行，其它参与方投递 control。

**逻辑 Turn**（当前实现）：Turn = AgentSession 的一次逻辑 Input → result，与 run、`LLMContext` Outcome、输入批次 `(run_id, input_seq)` 都不一一对应，不能用 run 数或 Outcome 数推算 Turn 数。

| 规则 | 行为 |
|---|---|
| 打开 | 没有打开的 Turn 时提交的输入批次打开新 Turn（`turn_seq + 1`，receipt `opens_turn = true`）：bootstrap 的 `on_init`、msg / event 的 `on_wakeup` |
| 继续 | 普通 behavior 切换、fork 调用与子 process 返回、independent 切换（这些交接都是 `on_behavior_switch` 输入批次）、观察注入、可恢复挂起（未要求 stop 的 Interrupted、可重试的 Runtime / 暂时性错误使 run paused、ContextLimitReached、PendingTool）、上下文上限的 history epoch 重写、重启与崩溃恢复都不改变 `open_turn`；Turn 打开期间消费的 msg / event 加入它（`open_turn.inputs` 追加） |
| 关闭 | 只由 session 决定（run 结束时的 `finish_run`，或 stop），与 `open_turn = null`、worklog `turn_ended` 同一次提交：`Done` 交付结果 → `completed`（再按 `end_condition` 判定是否 finished）；`WAIT_USER_MSG` 只在本 run 已交付答复（快照 `last_report` 非空，或最后一个 Step 带 `<sendmsg>`）时 `completed`，否则 Turn 保持打开，下一条输入加入它；不可重试错误 → `failed`；预算耗尽 → `budget_exhausted`；`control(stop)` → `stopped`。fork 子 process 结束（`process_done`）不关闭 Turn |

- 条目归属的 Turn：有 `open_turn` 时为其 `index`，否则为 `turn_seq`。`turns_completed` 只计 `completed`。
- Sub AgentSession 是另一个 session，有自己的 Turn；父子 session 的 Turn 不合并计数（当前只有 `create_session` + `origin.parent_session` 的创建与登记，启动子 Runner、等待与结果回传的 helper 属设计接口，尚未实现）。

**static.json**（统计，可缺失；`schema/session_static.schema.json`）：`{input_tokens, output_tokens, total_tokens, rounds, rounds_failed, rounds_interrupted, turns, runs, tool_calls, busy_ms, cost, updated_at_ms}`。

- `rounds`：本 runner 经 run 的 `LlmClient::infer` 发起的推理尝试数（成功、失败、中断都计；provider adapter 内部的重试算一次），每个 outcome 后累加；`rounds_failed` / `rounds_interrupted` 是其中返回错误 / 被中途放弃的部分。历史摘要（压缩）推理不是 Round，不计入；xllm 接手 run 后发起的推理只计入该 run 的 `run.json usage.llm_requests`（§7）。
- `turns` = 已完成的 Turn 数（`state.turns_completed`）；`runs` = 已结束的 run 数。Round、Turn、run 三个计数互不推导。

**report.md**（finished 时写入）：标题 `# Report — <objective 首行>`，随后 `- session:`、`- outcome:`、`- turns: N`（已完成的 Turn 数）和答复正文。

## 5. worklog.jsonl

每行一条 `{"seq": n, "t": "<kind>", …}`，`seq` 从 1 开始严格 +1。Schema：`schema/worklog_entry.schema.json`。

| t | 写入时机 | 字段 |
|---|---|---|
| created | 创建 session（第 1 条） | session_id, kind, by, objective, at_ms |
| turn_started | run 结束 / 挂起 / 中途重写时批量写 | run_id, turn, input_seq, inputs[{src,index,key,kind}], changes[], hook, at_ms：打开 Turn 的输入批次 |
| input_batch | 同上 | run_id, turn, input_seq, inputs[], changes[], hook, at_ms：加入已打开 Turn 的批次（交接、fork 返回、补充输入）；观察批次（hook `observation`）不写此条，只写其 user_message |
| user_message | 同上 | run_id, turn, content（批次渲染后的消息） |
| assistant_message | 同上（只用于 function call run） | run_id, turn, assistant, tool_calls[{call_id, tool, args(键排序), effect}]：一次模型 response（一个 Round）及其原生工具调用；不是 Step |
| step | 同上（只用于 behavior run） | run_id, turn, step_index, behavior?, assistant, actions[{call_id, tool, args(键排序), effect}], correction?（解析失败 / 策略拒绝产生的合成纠错 Step 为 true，缺省 false） |
| action_result | 同上 | run_id, turn, call_id, status(ok/error/unresolved/cancelled/pending), result：原生工具结果（function call，`[not executed]` / `[unresolved…` 记为 unresolved，`[cancelled]` 记为 cancelled）或 action 结果（behavior） |
| outcome | run 结束 / 挂起 / 中途重写 | run_id, turn, kind（run 结束：done/wait/process_done/budget/error/stopped；process 挂起：suspended；中途重写：context_rewritten）, next_behavior?, report?。保留 run 的结果（PendingTool、上下文上限暂停、中断、可重试错误）不写 outcome |
| turn_ended | 关闭 Turn 的同一次提交 | run_id（stop 时没有 run 则为空串）, turn, status(completed/failed/budget_exhausted/stopped), at_ms |
| compaction | 生成新 summary.json | summary_start_seq, made_by |
| decide | 应用 decide | decision, by, report |
| input_rejected | 输入被拒绝但标记为已消费 | input{src,index,key,kind}, reason |
| change_dropped | 变化被合并 / 超预算 | change, reason |
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
- 机械渲染（`libopendan.mechanical/2`）：`turn_started` → `── turn N (hook) inputs: … ──`，`input_batch` → `── <hook> inputs: … ──`（如 `── on_behavior_switch ──`），`user_message` → `[input] …`，`assistant_message` → `[assistant] …`，`step` → `[step 3 plan] …` / `[step 3 plan correction] …`，`action_result` → `[result #<call_id> <status>] …`，`outcome` → `[outcome <kind> → <next_behavior>] …`，`turn_ended` → `── turn N completed ──`（默认在 `drop_kinds` 中，不渲染）。
- 反向读取时预算先于起点耗尽：必须先压缩（新起点 = 已保留的最旧一条，摘要覆盖 `[旧起点, 新起点)`），摘要与原始记录之间不能留空洞。
- 压缩只写 summary.json，并追加一条 `compaction` 条目后提交，从不改写 worklog。
- 确定性只在同一 `renderer` 版本内承诺；时间等新鲜量只出现在每个输入批次的消息里。

## 7. runs/（xllm run 目录）

```text
runs/<run_id>/run.json            xllm RunRecord（version 2）
runs/<run_id>/snapshots/NNNN.json LLMContextSnapshot（snapshot_version 3；先 fsync 再发布）
runs/<run_id>/.lock               run 执行锁（长期持有的 flock）
```

`run_id` = `YYYYMMDD-HHMMSS-<6hex>`。宿主装配的 run 在 RunRecord 中增加（均可缺省）：

| 字段 | 含义 |
|---|---|
| `host` | `{assembled_by:"libopendan", session_id, runtime_kind:"native|tmux", runtime_id, env_check}`；xllm 只接手 `native` |
| `host_commit_pending` | 宿主输入提交门槛（批次号）。非空时任何执行者都不得推理或调用工具，xllm 拒绝接手 |
| `inflight[]` | 已派发、结果尚未随快照持久化的动作：`{call_id, tool, args, effect, execution_ids, started_at_ms}` |
| `executions[]` | 尚未确认停止的受管进程执行：`{execution_id, call_id, kind, runtime_id, host, boot_id, pgid, leader_start_ticks, command, started_at_ms}` |
| `host.extra.finish` | 宿主的结束决定：与终态 `status` 在**同一次** run.json 写入中记录（`{kind: done|wait|process_done|budget|error|stopped, finished, outcome, waiting, answer, error, usage, turn_end}`；`turn_end` 为本次结束关闭 Turn 的状态，空表示 Turn 继续）。结束流程中途崩溃时，恢复按它重做，而不是重新推断 |
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
         last_run=本 run；关闭 Turn 时 open_turn=null 并更新 turns_completed；fork 子 process 结束时
         同一次提交里父 run 重新成为 live_run）→ ④ 删除不再被引用的旧 run（持其锁、确认无未核对执行）
切换：   普通切换 run 继续，status 保持 running；fork / independent 挂起时 status 为 paused。
         终态 status 只表示 run 已结束。
挂起：   （fork / independent，§8）先追加已产生的历史，run 由 process_stack 引用
重写：   （上下文上限，见下）① 追加本 run 未写入的历史 + outcome(context_rewritten) 并提交 flush 标记
         → ② 压缩 summary.json → ③ 以 system + 会话历史重建的上下文发布快照（history_epoch + 1）
```

**上下文上限中途重写**（llm_context X7）：run 以 `ContextLimitReached` 让出时，runner 不在快照里重写历史，而是把它交还 session 历史：

1. 用 flush 标记写入本 run 已产生而未写入的历史，末尾追加 `outcome(kind=context_rewritten)`，与 flush 标记同一次提交 state.json（没有新历史时两者都不写，重做不会重复）。
2. 压缩 session 历史：起点前移到只剩约 `history_budget >> 第几次` token 的原始记录（写 summary.json 与 `compaction` 条目），再按 §6 重建历史消息。
3. 以 `system + 历史消息` 作为新的输入恢复上下文（function call：`RewrittenHistory`；behavior：`RewrittenSteps`，step 全部折叠进输入，编号继续），HostMeta 进入新的 epoch，先发布快照（run.json `status=running`）再继续推理。

一次推进内连续最多 3 次；仍装不下时 run 以 `paused` + `last_error.kind=context_limit` 保留挂起快照，下一次推进先重写再继续。① 之后、③ 之前崩溃：恢复用旧 epoch 最后发布的快照，再次让出、重写，已写入的部分由 flush 标记跳过；③ 之后崩溃：新 epoch 的计数从 0 开始。receipt 的 `input_seq`、消费位置与打开的 Turn 都不随重写改变。

**保留**：`live_run`、`last_run`、`process_stack[].run_id` 引用的 run 保留；其它 run 在确认没有未核对执行后删除；旧快照按需裁剪（保留最新几份与已发布指针）。

## 8. behavior process 与 run

| 切换 | run |
|---|---|
| 普通（未声明 mode） | 同一 context、同一 run；`current_behavior` 更新，下一个输入批次为 `on_behavior_switch` |
| fork | 父 run 挂起入 `process_stack`；子 process 开新 run，继承父的 steps（`HostMeta.inherited_below` 之下的 step 不由子 run 写入 worklog）；子 process 结束（任何 next_behavior 都视为返回）时，**在子 run 结束的同一次提交里**出栈、父 run 成为 live_run，子结果作为 `process_result`（`{behavior, result, next_action_id, next_step_index}`）进入父 run 的下一个 `on_behavior_switch` 输入批次；父 run 恢复时 action / step 编号不小于子 run 的，保证 call_id 唯一 |
| independent | 当前 process 挂起入栈；目标 process 有挂起的 run 就恢复它，否则开新 run（不继承 steps，step 编号独立） |

三种切换都是同一 Turn 内的交接：交接批次（`input_batch`，receipt `opens_turn = false`）加入打开的 Turn，fork 子 process 结束（`process_done`）也不关闭它。

挂起时先写入该 run 已产生而未写入的历史（`flushed_message_count` 或 `flushed_step_index`，以及 `flushed_input_seq` 前移），worklog 因此保持时间顺序。

**flush 标记**（`live_run` / `process_stack[]` 中，两种 run 各用一个游标）：function call run 以位置计数（`flushed_message_count` = prefix 之后已写入的消息数，只在 `flushed_epoch == HostMeta.history_epoch` 时有效；快照的 epoch 更新时从 0 计，且只用 `input_seq > epoch_input_seq` 的 receipt 定位消息）；behavior run 以身份计：`step_index < flushed_step_index` 的 step（身份高水位，不是数量：fork 子 run 继承的 step 不由它写入）与 `input_seq ≤ flushed_input_seq` 的注入消息已写入，不受中途重写影响。只有已完成的 Step（`steps` / `last_step`）会写入，仍在派发 action 的 `action_step` 完成后才写。注入消息按 receipt 的位置排序：`request_input` 位于 `after_step` 之前，`step` 位于所附 step 之后。

## 9. binding.json 与 .runtime/bin

```jsonc
{ "runtime_id": "rt-…", "kind": "native | tmux", "workdir": "/abs", "bound_at_ms": 0, "bound_by": "rn-…" }
```

- 首次推进时用不覆盖发布写入（`link` / `renameat2(NOREPLACE)`），之后不变；runtime_id 不同必须拒绝（RuntimeMismatch），发生在任何推理之前。
- workdir：有 workspace 用 workspace（agent 内部 workspace 为 `<agent_root>/workspace/<id>`），否则为 session 目录。
- binding.json 只证明身份；每次推进都要幂等修复并核验环境：`.runtime/bin` 按期望集合（tool_plan 墓碑 + `agent-session` 包装脚本）重写，`.manifest.json`（条目 → sha256）最后写入；核验失败（缺文件、内容不符、不可执行）不得推理，下次推进修复。
- 墓碑：`#!/bin/sh` 输出 `{"blocked_by":"tool_plan",...}` 与人读说明到 stderr，`exit 127`。

## 10. 恢复顺序（drive 开头）

1. 取 session 锁（身份必须等于 `session.driver.principal`）；校验 schema 主版本（`state.json` / `session_config.json` 必须是 `/2`，否则 RecoveryBlocked；不迁移，也不按旧字段读取）。
2. 登记表 `location` 必须等于本目录，否则不推进。
3. 截掉 worklog 未提交尾部（文件短于 `committed_bytes` → RecoveryBlocked）。
4. 删除未被引用的 run（持锁、核对执行；无法核对则保留）。
5. live_run：持 run 锁（拿不到 → RunBusy）；读 run.json 与已发布快照（缺失 / 损坏 / 版本不支持：run.json `version ≠ 2` 或快照 `snapshot_version ≠ 3` → RecoveryBlocked，保留现场）；确认旧执行已停止；校验快照 receipt（批次 1..n 连续，state 已应用的批次必须在快照中）并补交 `input_seq > applied` 的 receipt 到 state（只补元数据，按 `opens_turn` 打开或加入 Turn，不重新追加消息）；门槛非空且 state 已覆盖 → 清门槛；run 已到终态 → 按 `host.extra.finish` 重做结束（没有记录时——例如 xllm 跑完——从最终快照推断：behavior 取最后一个 step 的 next_behavior 与 report，function call 取最后一条 assistant 文本）。恢复未结束的 behavior run 时，以 `state.current_behavior` 作为 behavior 名（普通切换可能晚于最后一个快照）。
6. 重试确认已提交的输入位置；补发登记表回报与感知。
7. 读取新输入、应用 control；finished 则拒绝剩余普通输入。
8. 绑定 / 核验 runtime；恢复 live run（在途动作物化为“结果未知”并先持久化），继续推进。快照挂起在等待 deferred 工具结果（本 runner 无法提供）→ RecoveryBlocked；挂起在上下文上限 → 先按 §7 重写再继续。

RecoveryBlocked 时只在 `state.last_error` 记录原因并回报，保留 live_run、run 目录、消费位置与执行证据，不自动放弃。
