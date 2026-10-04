# Session Control Protocol（Session 控制）

版本 3 · 2026-10-03（同日补充驱动者停止请求与无队列 Session）· 由 `libopendan` 反写（`src/protocol/input.rs::ControlCommand`、`src/runner/inputs.rs`）。

控制命令与 Agent 输入走同一条总线（`type = "control"`，信封见 [Session Input Protocol](<Session Input Protocol.md>) §2），但不是 Agent 的输入：由 Runner（lease 持有者）执行，不进入 LLM 上下文，不产生输入 receipt。搭同一条队列只是为了多方投递、持久化和与事件保持同一个顺序。

```json
{ "schema": "opendan.session_input/3", "type": "control", "key": "ctl:stop:1790899260000",
  "from": "did:bns:alice", "at_ms": 1790899260000,
  "payload": { "command": "stop", "reason": "用户取消" } }
```

| `command` | 字段 | 应用规则 |
|---|---|---|
| `stop` | `reason?` | 置 `stop_requested`。运行中：在下一个检查点打断推理；工具执行期间由监视任务发现队列中的 stop 并打断（监视任务只查看，由驱动者消费并提交）。run 结束，打开的 Turn 以 `stopped` 关闭，`finished + stopped`。等待 task 的挂起调用按当时状态回填，可取消的 task 被取消 |
| `decide` | `decision: accept \| discard`、`by`、`note?` | finished 之后唯一仍可接受的输入，仅 work session。运行中或未 finished 时留在队列并写 `pending_decision`；accept 需 `acceptance = pending`，discard 需 `pending \| accepted`，否则拒绝（`input_rejected`） |
| `subscribe` | `subscription: {id, mode: active \| semi, source, watch[]}` | 加入 `session_config.subscriptions`（`config_rev + 1`）。`source`：`{type: session, ref}`、`{type: object_event, object, event?}`、`{type: task, task_id}`、`{type: timer, name}`、`{type: system, id}` |
| `unsubscribe` | `id` | 移除订阅、其拉模式游标和 `pending_events` 中尚未注入的状态；之后到达的该订阅事件丢弃 |
| `activity` | `summary?`、`touch[]`、`clear` | 合并进 `state.activity`（touching 上限 16） |
| `perceive` | `kind`（默认 `observation`）、`summary`、`tags[]`、`objects[]` | 驱动者以 lease 追加到 Agent 感知流（按 seq 幂等），再消费输入 |

- 控制命令与事件按投递 index 依次生效（同一源内）。
- 应用时写 `control_applied`（decide 写 `decide`），与消费标记同一次提交；之后确认输入源。
- 未知 `command` → `unknown_command`；字段不合法 → `invalid_payload`。
- 控制命令与 msg / event 共用每个 Session 64 条 pending 记录的上限。
- **驱动者自己的停止请求**：驱动进程可以不经队列请求停止（收到 SIGINT、宿主停止一个没有队列的子 Session）。效果与已消费的 `stop` 相同：置 `stop_requested`，写 `control_applied{command: stop, input.src: "_runner"}`，运行中的推理 / 工具被打断，Turn 以 `stopped` 关闭。没有输入队列的 Session 只有这一种 stop。
- 没有输入队列的 Session 不接受队列型控制；`decide` 由决定者在 `artifact:<aid>` 锁下直接走产物列表（Agent State Protocol §6），不经 Session。
- 斜杠命令不由 Session 解析：msg bridge 在入队前按登记表与发送者身份把 `/stop` 等转成 `control`。
