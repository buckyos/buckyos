# Session Directory Protocol

版本 1 · 2026-09-29 · 由 `libopendan` 反写（`src/protocol/{config,state,summary,worklog,misc,input}.rs`）

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
  "schema": "opendan.session_config/1",
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
    "end_condition": { "type": "llm_declares_done | output_schema | max_rounds", "detail": {"n": 3} },
    "scope": { "objects": [], "paths": ["ws:snake/src/"] },     // 活动视图的初始声明
    "input_policy": "any | supplement_only | none",
    "acl": { "owner": null, "readers": [], "agent_access": "full | status_only" },
    "task_binding": null
  },
  "prompt": {
    "llm_context": { /* xllm .llm_context 的 JSON 形式（同 schema、严格键） */ },
    "behavior": "plan", "system_prompt": "...", "context": ["..."],
    "mechanical_compress": { "recent_full_steps": 2, "summary_chars": 280, "max_result_chars": 4096,
                             "drop_kinds": ["created","decide","compaction","input_rejected","change_dropped"] },
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

- `prompt.llm_context` 为 `null` 时，参考实现使用 `{"tools":{"enabled":true}}`（buckyos Provider、`llm.chat`、function_call、内置 bash 工具组）。
- `extensions.opendan.process_modes`：behavior 名 → `fork | independent`（§8）；未列出的 behavior 为普通切换。
- `extensions.opendan.perception_window`：self_improve session 要整理的感知窗口（见 Agent State Protocol §4）。

## 4. state.json（提交点）

Schema：`schema/session_state.schema.json`。

```jsonc
{
  "schema": "opendan.session_state/1",
  "rev": 17,                                   // 每次提交 +1
  "writer": { "runner_id": "rn-…", "principal": "app:app2@alice", "host": "host:…", "pid": 1234, "lock_epoch": 42 },
  "run_state": "created | ready | running | waiting | finished",
  "waiting_for": null | { "kind": "input | tool", "refs": [], "deadline_ms": null },
  "outcome": null | "succeeded | failed | stopped",
  "acceptance": "n/a | pending | accepted | discarded",
  "result": null | { "answer": "...", "answer_ref": "report.md", "artifact_ref": {...}, "discard_report": {...} },
  "round": 12, "current_behavior": "plan", "process_entry": "plan",
  "process_stack": [ { "entry": "plan", "mode": "fork | independent", "run_id": "…",
                       "rounds": [...], "flushed_step": 3, "flushed_input_seq": 1, "applied_input_seq": 1 } ],
  "bootstrap_done": true,
  "topic": { "title": "", "tags": [] },
  "live_run": null | { "run_id": "…", "rounds": [ { "round": 12, "inputs": ["q#121"], "changes": ["s1@16"],
                                                    "hook": "on_wakeup", "input_seq": 3, "at_ms": 0 } ],
                       "applied_input_seq": 3, "flushed_step": 0, "flushed_input_seq": 0, "process_entry": null },
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

## 5. worklog.jsonl

每行一条 `{"seq": n, "t": "<kind>", …}`，`seq` 从 1 开始严格 +1。Schema：`schema/worklog_entry.schema.json`。

| t | 写入时机 | 字段 |
|---|---|---|
| created | 创建 session（第 1 条） | session_id, kind, by, objective, at_ms |
| round_started | run 结束 / 挂起时批量写 | run_id, round, inputs[{src,index,key,kind}], changes[], hook, at_ms |
| user_message | 同上 | run_id, round, content |
| step | 同上 | run_id, round, behavior?, assistant, actions[{call_id, tool, args(键排序), effect}] |
| action_result | 同上 | run_id, round, call_id, status(ok/error/unresolved/cancelled/pending), result |
| outcome | run 结束 / 挂起 | run_id, round, kind(done/wait/switch/process_done/suspended/budget/error/stopped), next_behavior?, report? |
| compaction | 生成新 summary.json | summary_start_seq, made_by |
| decide | 应用 decide | decision, by, report |
| input_rejected | 输入被拒绝但标记为已消费 | input{src,index,key,kind}, reason |
| change_dropped | 变化被合并 / 超预算 | change, reason |
| control_applied | 应用 control | input, command, detail |

- **只追加**：已提交部分从不改写；唯一允许的修改是恢复时截掉 `committed_bytes` 之后的尾部（先于读取任何新输入）。
- **run 进行中的历史只在 runs/**：run 结束或挂起时，从快照一次性生成尚未写入的条目追加（一次 fsync），随后提交 state。
- **文件顺序 = 提交顺序**：run 进行中应用的 control 等条目在发生时追加，因此会出现在该 run 的历史之前。
- **读取**：运行时只从 `committed_bytes` **反向**读（按块，由新到旧），读到 `summary.start_offset` 或预算用尽即停；正向读取只用于压缩的有界区间和审计。

## 6. summary.json 与下一次 llm_context

```jsonc
{ "schema": "opendan.session_summary/1", "history_summary": "…", "start_seq": 301, "start_offset": 912384,
  "mechanical": { ...同 prompt.mechanical_compress... }, "renderer": "libopendan.mechanical/1",
  "made_at_seq": 318, "made_by": "context_limit | ratio | manual", "updated_at_ms": 0 }
```

- 缺失时等价于 `start_seq = start_offset = 0`、无摘要、`mechanical = session_config.prompt.mechanical_compress`。
- 下一次 llm_context（没有 live_run 时）= system（身份 + 不可覆盖约束 + 应用 prompt + context + objective + xllm 能力/协议段）+ 一条历史消息（`<session_history>`：摘要 + 起点之后按机械压缩渲染的记录）；本轮输入由第一批 receipt 注入。
- 反向读取时预算先于起点耗尽：必须先压缩（新起点 = 已保留的最旧一条，摘要覆盖 `[旧起点, 新起点)`），摘要与原始记录之间不能留空洞。
- 压缩只写 summary.json，并追加一条 `compaction` 条目后提交，从不改写 worklog。
- 确定性只在同一 `renderer` 版本内承诺；时间等新鲜量只出现在每轮消息里。

## 7. runs/（xllm run 目录）

```text
runs/<run_id>/run.json            xllm RunRecord（version 1）
runs/<run_id>/snapshots/NNNN.json LLMContextSnapshot（先 fsync 再发布）
runs/<run_id>/.lock               run 执行锁（长期持有的 flock）
```

`run_id` = `YYYYMMDD-HHMMSS-<6hex>`。宿主装配的 run 在 RunRecord 中增加（均可缺省）：

| 字段 | 含义 |
|---|---|
| `host` | `{assembled_by:"libopendan", session_id, runtime_kind:"native|tmux", runtime_id, env_check}`；xllm 只接手 `native` |
| `host_commit_pending` | 宿主输入提交门槛（批次号）。非空时任何执行者都不得推理或调用工具，xllm 拒绝接手 |
| `inflight[]` | 已派发、结果尚未随快照持久化的动作：`{call_id, tool, args, effect, execution_ids, started_at_ms}` |
| `executions[]` | 尚未确认停止的受管进程执行：`{execution_id, call_id, kind, runtime_id, host, boot_id, pgid, leader_start_ticks, command, started_at_ms}` |
| `host.extra.finish` | 宿主的结束决定：与终态 `status` 在**同一次** run.json 写入中记录（`{kind: done|wait|process_done|budget|error|stopped, finished, outcome, waiting, answer, error, usage}`）。结束流程中途崩溃时，恢复按它重做，而不是重新推断 |

快照 `state.host["libopendan"]`（HostMeta）：`{session_id, base_input_len, process_entry, inherited_below, input_receipts[]}`，各执行者必须原样保留。

**run 生命周期**：

```text
建立：   create run dir + 持 run 锁 → run.json（status running，无快照）
开轮：   注入消息 + receipt → ① 快照 fsync → ② run.json(latest_snapshot_idx, host_commit_pending=seq)
         → ③ state.json（live_run、rounds、消费位置、订阅游标）→ ④ 清 host_commit_pending → ⑤ 确认输入源
         （④ 之前不允许推理 / 工具）
进行中： 每次推理前（function call：一轮工具结果之后；behavior：每步 do-action 之后）
         快照 fsync → run.json 发布指针并只清除快照已覆盖的 inflight
结束：   ① 结果快照 + run.json（终态 status 与 host.extra.finish 同一次写入）
         → ② 生成 worklog 条目追加（fsync）→ ③ state.json（live_run=null、last_run=本 run；
         fork 子 process 结束时同一次提交里父 run 重新成为 live_run）→ ④ 删除不再被引用的旧 run
         （持其锁、确认无未核对执行）
切换：   普通切换 run 继续，status 保持 running；fork / independent 挂起时 status 为 paused。
         终态 status 只表示 run 已结束。
挂起：   （fork / independent，§8）先追加已产生的历史，run 由 process_stack 引用
```

**保留**：`live_run`、`last_run`、`process_stack[].run_id` 引用的 run 保留；其它 run 在确认没有未核对执行后删除；旧快照按需裁剪（保留最新几份与已发布指针）。

## 8. behavior process 与 run

| 切换 | run |
|---|---|
| 普通（未声明 mode） | 同一 context、同一 run；`current_behavior` 更新，下一轮 `on_behavior_switch` |
| fork | 父 run 挂起入 `process_stack`；子 process 开新 run，继承父的 steps（`HostMeta.inherited_below` 之下的 step 不由子 run 写入 worklog）；子 process 结束（任何 next_behavior 都视为返回）时，**在子 run 结束的同一次提交里**出栈、父 run 成为 live_run，子结果作为 `process_result`（`{behavior, result, next_action_id, next_step_index}`）进入下一轮；父 run 恢复时 action / step 编号不小于子 run 的，保证 call_id 唯一 |
| independent | 当前 process 挂起入栈；目标 process 有挂起的 run 就恢复它，否则开新 run（不继承 steps） |

挂起时先写入该 run 已产生而未写入的历史（`flushed_step` / `flushed_input_seq` 前移），worklog 因此保持时间顺序。

**flush 标记**：function call run 以位置计数（`flushed_step` = prefix 之后已写入的消息数）；behavior run 以身份计：`step_index < flushed_step` 的 step 与 `input_seq ≤ flushed_input_seq` 的注入消息已写入。注入消息按 receipt 的位置排序：`request_input` 位于 `after_step` 之前，`step` 位于所附 step 之后。

## 9. binding.json 与 .runtime/bin

```jsonc
{ "runtime_id": "rt-…", "kind": "native | tmux", "workdir": "/abs", "bound_at_ms": 0, "bound_by": "rn-…" }
```

- 首次推进时用不覆盖发布写入（`link` / `renameat2(NOREPLACE)`），之后不变；runtime_id 不同必须拒绝（RuntimeMismatch），发生在任何推理之前。
- workdir：有 workspace 用 workspace（agent 内部 workspace 为 `<agent_root>/workspace/<id>`），否则为 session 目录。
- binding.json 只证明身份；每次推进都要幂等修复并核验环境：`.runtime/bin` 按期望集合（tool_plan 墓碑 + `agent-session` 包装脚本）重写，`.manifest.json`（条目 → sha256）最后写入；核验失败（缺文件、内容不符、不可执行）不得推理，下次推进修复。
- 墓碑：`#!/bin/sh` 输出 `{"blocked_by":"tool_plan",...}` 与人读说明到 stderr，`exit 127`。

## 10. 恢复顺序（drive 开头）

1. 取 session 锁（身份必须等于 `session.driver.principal`）；校验 schema 主版本。
2. 登记表 `location` 必须等于本目录，否则不推进。
3. 截掉 worklog 未提交尾部（文件短于 `committed_bytes` → RecoveryBlocked）。
4. 删除未被引用的 run（持锁、核对执行；无法核对则保留）。
5. live_run：持 run 锁（拿不到 → RunBusy）；读 run.json 与已发布快照（缺失 / 损坏 / 版本不支持 → RecoveryBlocked，保留现场）；确认旧执行已停止；校验快照 receipt（批次 1..n 连续，state 已应用的批次必须在快照中）并补交 `input_seq > applied` 的 receipt 到 state（只补元数据，不重新追加消息）；门槛非空且 state 已覆盖 → 清门槛；run 已到终态 → 按 `host.extra.finish` 重做结束（没有记录时——例如 xllm 跑完——从最终快照推断：behavior 取最后一个 step 的 next_behavior 与 report，function call 取最后一条 assistant 文本）。恢复未结束的 behavior run 时，以 `state.current_behavior` 作为 behavior 名（普通切换可能晚于最后一个快照）。
6. 重试确认已提交的输入位置；补发登记表回报与感知。
7. 读取新输入、应用 control；finished 则拒绝剩余普通输入。
8. 绑定 / 核验 runtime；恢复 live run（在途动作物化为“结果未知”并先持久化），继续推进。

RecoveryBlocked 时只在 `state.last_error` 记录原因并回报，保留 live_run、run 目录、消费位置与执行证据，不自动放弃。
