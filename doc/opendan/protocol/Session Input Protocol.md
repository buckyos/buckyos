# Session Input Protocol

版本 2 · 2026-10-01 · 由 `libopendan` 反写（`src/protocol/input.rs`、`src/channel/kmsg.rs`、`src/runner/{receipts,hook,drive,assembler}.rs`）。输入批次与逻辑 Turn 的关系见 [Session Directory Protocol](<Session Directory Protocol.md>) §4

## 1. 通道

| 通道 | 命名 |
|---|---|
| kmsg 队列 | 名称 `opendan.session.<sid>`，URN `<appid>::<owner>::opendan.session.<sid>`；由创建者（驱动者 App 身份）创建，`sync_write=true`、`other_app_can_write=true` |
| kmsg 订阅 | `opendan.<agent_id>.<sid>`（sub id 在全部队列与 App 间共用一个命名空间）；创建时 `Earliest` |
| kevent 唤醒 | `/opendan/<agent_id>/session/<sid>/input`，投递后发布 `{"sid": …}`；只表示“有变化”，丢失由轮询兜底 |
| msg-center inbox | UI session 用（后移，本版本不支持） |

`agent_id` 默认取 agent DID 最后一段并把字母、数字、`_ - .` 之外的字符替换为 `_`。

**kmsg 使用规则**（按 kmsg 现状）：`create_queue` / `subscribe` 返回“already exists”视为成功（并用 `get_queue_stats` 确认队列）；读取用 `read_message(queue, acked_index + 1, n)`，不依赖服务端游标；`commit_ack` 只提交 state.json 已提交的**连续**消费位置，永不小于已记录值；遇到 “Subscription not found”（服务重启丢游标）以 `At(acked_index + 1)` 重新订阅；本期不删除消息。

## 2. 消息格式

kmsg `Message.payload` 是 JSON（≤ 250 KB；更大内容放 session 目录或 NamedStore，只投递引用）。headers：

| header | 必需 | 含义 |
|---|---|---|
| `type` | ✔ | `msg | event | change | control | perception` |
| `key` | ✔ | 生产者去重键；change 按 key 合并，终态用独立 key（`…#terminal`） |
| `from` | ✔ | 投递方 principal（自报，只用于审计） |
| `at_ms` | ✔ | 投递时间 |
| `intent`、`reply_to` | | 可选 |

| type | 触发推理 | payload |
|---|---|---|
| msg | 是（drive 推进时拉取，组成输入批次） | `{"text": "…", …}` |
| event | 是（active 订阅） | 任意 JSON（`text` 优先渲染） |
| change | 否，只在观察边界 / 下一个输入批次注入 | `{"text": "…", "subscription": "s2", "version": "…", "terminal": false}` |
| control | 否，drive 开头与每个观察边界应用 | 见 §4 |
| perception | 否，持有者追加到 Agent 感知 | `{"kind": "observation", "summary": "…", "tags": [], "objects": []}` |

无法识别的 type、非 JSON payload、未知 control 命令：标记为已消费并写 `input_rejected`（带原因），避免累积 ack 被卡住。

## 3. 消费进度（state.json `inputs` / `recent_keys`）

```jsonc
"inputs": { "q": { "acked_index": 118, "consumed_above": [121], "reading": [] } },
"recent_keys": ["m-1", "…"]
```

- `mark(i)`：`i ≤ acked_index` 忽略；否则加入 `consumed_above`，然后把连续前缀折叠进 `acked_index`（永不回退）。
- 取输入时跳过 `is_consumed(i)` 的投递；非 change 输入若 `key` 在 `recent_keys` 中视为重复投递，直接标记已消费（不进入上下文、不记 worklog）。
- **先提交 state.json，再确认输入源**。两步之间崩溃：恢复时重试确认；重投的消息由消费记录过滤。
- 选择性消费：跳过的消息留在 `acked_index` 之后；finished 之后的普通输入显式拒绝（`input_rejected`）。

## 4. control 命令（payload `{"command": …}`）

| command | 字段 | 应用规则 |
|---|---|---|
| `stop` | `reason?` | 置 `stop_requested`；运行中在下一个观察边界打断推理，run 结束，打开的 Turn 以 `stopped` 关闭，`finished + stopped`；finished 后拒绝 |
| `decide` | `decision: accept|discard`, `by`, `note?` | 仅 finished 的 work session；运行中或未 finished 时留在队列并写 `pending_decision`；accept 需 `acceptance=pending`，discard 需 `pending|accepted`，否则拒绝 |
| `subscribe` | `subscription` | 加入 `session_config.subscriptions`（`config_rev + 1`） |
| `unsubscribe` | `id` | 移除订阅及其游标 |
| `activity` | `summary?`, `touch[]`, `clear` | 合并进 `state.activity`（touching 上限 16） |

应用 control 时写 `control_applied`（decide 写 `decide`），与消费标记同一次提交。

## 5. 输入 receipt（进入上下文的输入）

每批进入上下文的 msg / event / change / 订阅变化都有结构化 receipt，与消息正文写在**同一份**快照的 `state.host.libopendan.input_receipts[]` 中：

```jsonc
{ "run_id": "…", "input_seq": 2, "turn": 5, "opens_turn": true, "hook": "on_wakeup",
  "inputs": [ { "src": "q", "index": 121, "key": "m-1", "kind": "msg" } ],
  "changes": [ { "id": "s1@16", "subscription": "s1", "cursor": { "rev": 16, "view": {...} } } ],
  "consumed_only": [ /* 被合并掉的 change 投递 */ ],
  "message_pos": { "kind": "request_input|accumulated|step|none", "index": 3 },
  "content": "<session_input hook=\"on_wakeup\" time=\"…\">…</session_input>", "bootstrap": false, "after_step": 7,
  "extra": { "continuation": true }, "at_ms": 0 }
```

- 批次 ID = `(run_id, input_seq)`，每个 run 从 1 开始连续；fork 子 run 不继承父 run 的命名空间。
- 输入批次不是 Turn。`turn` = 批次所属的逻辑 Turn；`opens_turn = true` 只用于打开 Turn 的那一批：是否打开只看提交时有没有打开的 Turn，不看 hook，实际是 bootstrap 的 `on_init` 或 Turn 关闭后 msg / event 的 `on_wakeup`。交接（`on_behavior_switch`）和补充输入在 Turn 打开期间提交，因而加入它；观察批次（hook = `observation`，常量 `OBSERVATION_HOOK`）固定 `opens_turn = false`。
- `content` 是批次渲染后的 user 消息：hook 点批次为 `<session_input hook="on_init|on_wakeup|on_behavior_switch" time="…">…</session_input>`（按需包含 `<behavior_switch to>`、子 context 交回的 `<process_result behavior status="ok|failed|needs_user_input">`、进入子 context 时的 `<sub_task mode="create_sub_context|fork">`、`<task>`、`<scope>`、`<inputs>`、`<changes>`、`<perceptions>`、`<hints>`、活动 session 列表、`<runtime>`）；观察批次只有 `<changes>…</changes>`。
- 应用 receipt（幂等，`input_seq ≤ live_run.applied_input_seq` 忽略；必须恰好是下一批；`opens_turn = false` 时 `turn` 必须等于 `open_turn.index`，否则 RecoveryBlocked）：追加 `live_run.turns` 条目，同一 Turn 的后续批次并入该 run 中这个 Turn 的条目；`opens_turn` 时设置 `open_turn`（`index`、打开批次的 `(run_id, input_seq)`、hook、msg / event 输入）并前移 `turn_seq`，否则把 msg / event 输入追加到 `open_turn.inputs`；非观察批次置 `run_state = running`、`waiting_for = null`；标记 `inputs + consumed_only` 为已消费并记入 `recent_keys`；写订阅游标；`bootstrap` 置 `bootstrap_done`；`extra.continuation` 清除 `internal_continuation` / `process_result`。恢复时补交 receipt 与正常提交的效果完全相同（否则交接批次会被再注入一次）。
- **提交顺序**：① 快照 fsync ② run.json 发布快照指针与 `host_commit_pending = input_seq` ③ state.json 应用 receipt ④ 清门槛 ⑤ 确认输入源。④ 之前不得推理或调用工具。
- **恢复**：先按快照中的 receipt 补齐 state（不重新追加消息），再取新输入；state 已应用而快照缺失的批次 → RecoveryBlocked，不回退消费位置。
- control / perception 不伪造 receipt：它们由各自的持久效果（worklog 条目、感知追加）与消费标记同次提交建立关系。

## 6. 观察边界与变化（半订阅）

- **拉模式**（订阅其它 session）：比较登记表 `status.rev` 与 `subscription_cursors[sub].rev`，被 watch 的字段变化才生成变化（`<sub>@<rev>`）。
- **推模式**（change 输入）：同 key 取最新一条，被合并的投递进入 `consumed_only` 并写 `change_dropped`；终态优先；超出预算的留在队列下次注入。
- **活动 session 集合**：默认半订阅（游标 `_active_sessions`）；每个 hook 点批次的消息渲染完整列表，并以变化 `_active_sessions@input` 记录游标；观察边界只在集合或交集变化时注入（变化 id `_active_sessions@<视图摘要>`）。
- 观察边界 = run 内的检查点（`CheckpointHook`；function call：每次推理前，即一批工具结果之后；behavior：每个 Step 的 do-action 之后，不在 Step 内层推理前）。变化以一批 receipt（hook `observation`，`opens_turn = false`，`turn` = 打开的 Turn）注入，同样走 ①–⑤；worklog 只记它的 `user_message`，Turn 数不变。
- 半订阅变化不触发推理，单独到达时也不组成输入批次（run 内在观察边界注入，否则随下一个 msg / event / 交接批次一起注入）；active 订阅以 event 投递并触发推理：没有打开的 Turn 时打开新 Turn，否则加入打开的 Turn。
