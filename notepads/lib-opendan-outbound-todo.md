# libopendan 下层先行项：出站、UI kind、常驻托管、kRPC Agent State

- 状态：已实施（2026-10-03）。本文是 [OpenDAN Agent Loader 重构](./opendan-agent-loader-refactor-todo.md) §4.2.6 / §3.2 / §4.1 要求先于 Loader 完成的 libopendan 改动的记录；Loader 一侧只引用这里。
- 没有改 `llm_context` 与 `agent_tool`：出站内容转换复用 `llm_context::ai_message_to_msg_object_with_base_validated_async`。

## 1. 出站（`runner/outbound.rs`）

| 项 | 落点 |
|---|---|
| 待发列表 | `SessionState.outbox: Vec<OutboxEntry>`（`key`、完整 `msg`、`turn`、`status = pending / sent / failed`、`msg_id`、`deliveries`、`error`、`attempts`、`updated_at_ms`）；保留最近 16 条已结束条目 |
| 回复坐标 | `Channels.outbound: Option<OutboundBinding{to, to_session, kind}>`（原为 `Option<Value>` 占位）；`SessionSpec.outbound` |
| sink | `runner::OutboundSink { compose(cfg, base, &TurnReply) -> Option<MsgObject>; send(sid, &OutboundRecord) -> SendResult }`；`RunnerDeps.outbound` / `HostDeps.outbound`，缺省 `None` |
| 生成 | `commit_run_end` 关闭 Turn 时调用 `queue_reply`：`outbound_base(state.reply)` 给信封（`created_at_ms` 在此定死），`sink.compose` 填内容，条目与关闭 Turn 的提交一起落盘 |
| 发送 | `flush_outbox`：`after_run_end`（提交之后）与每次 drive 的恢复之后；按序、遇到 `Retry` 停下；结果立即提交 |
| 恢复 | 重启后的第一次 drive 重发仍为 `pending` 的条目：同一个键、同一个 MsgObject |

与需求文不同或文里没写的决定：

1. **没有 sink 就不产生 outbox**。xagent 不配置 sink，行为与 fixtures 不变。
2. **内容转换与文案由 sink 负责**。`compose` 的默认实现只把 `Completed` Turn 的回答转成文本消息（`compose_text`）；失败提示、`delivery_failure_notice` 等 i18n 文案由宿主的 sink 加（OpenDAN：`MsgCenterSink`）。libopendan 里没有文案。
3. **哪些 Turn 产生回复**：每个关闭的 Turn 都问一次 sink，带上 `status`、`answer`、Turn 的输入键（`TurnReply::answers_a_message` 区分“回答一条消息”与“只由系统事件触发”）。
4. **不产生出站的 Session**：有父 Session 的（结果交给父，哪怕它的首批输入是 Agent 自己发的消息）；回复对象是 Agent 自己的。
5. **路由核对**：`reply` 与 `channels.outbound` 不一致 → 条目直接 `failed`（`route_mismatch`），不发送。
6. **重试**：退避 `min(2^attempts 秒, 60 秒)`；240 次仍没送到 → `failed`，让后面的回复不再排队。

## 2. 正式的 ui kind

- `create_session` 接受 `SessionKind::Ui`；内建 `ui` 模板的 kind 改为 `ui`。sid = `ui-H("ui", agent_did, route_key)`；没给 `route_key` 时补 `local:<幂等键或 uuid>`（xagent `new --class ui` 不变）。同一身份重复创建同一个 ui Session 视为幂等。
- 登记表条目增加 `route_key`。
- `SessionTemplate::load`：包自定义的 class 用 `[session.<class>] base = "ui"` 指定从哪个内建模板起步（缺省 `work`）。

## 3. 常驻托管（`host::Supervisor`）

- 每个托管的 Session 一个 `serve_one` 循环（`drive(Idle)` → 通知或有界轮询）；`serve_one` 增加每次 drive 结果的 observer。
- `tick`：查登记表 `driver = me` 且未结束（或 `pending_decision`）的条目，对没有循环的补一个——顶层 Session 与子 Session 同一条路。第一次 `tick` 就是启动恢复。
- 卸载后再装载的条件：显式 `ensure_task`（新输入、decide、刚创建）；父被 stop；有队列且距卸载超过 `recheck`（30 秒，兜底事件丢失）；`pending_decision`。阻塞类结果（NotDriver / Unregistered / BindFailed / RecoveryBlocked）与“无队列且已见过”的不自动重试。
- `shutdown`：不再起新循环，`abort` 各循环并等待 JoinHandle（释放 lease）；不请求任何 Session stop。
- `status()`：只读的托管状态（内存）。`has_pending_work` 把未发完的 outbox 算作待推进工作。
- 未做：`serve` / `run_session`（xagent）仍用 `ChildDriver`，没有改到 `Supervisor` 上；需求文 §9.1 第 15 项的“只在登记表 rev 或队列有变化时再驱动空闲 Session”以 idle unload + recheck 的形式部分实现，装载期间仍按 `poll_interval` 驱动。

## 4. kRPC Agent State（`state/krpc.rs`）

- `serve_call(agent, who, method, params)`：传输无关的服务端分发；`KrpcAgentStateClient` + `StateTransport`（`KrpcTransport` 用进程的 BuckyOS session token，或 `$OPENDAN_AGENT_STATE_TOKEN`）。`state::connect` 的 `Krpc` 分支接到它，连接时用 `agent.info` 核对 Agent DID。
- 范围、错误编码见 `doc/opendan/protocol/Agent State Protocol.md` §8。
- 未做：DID 文档 / zone 服务发现定位 endpoint（只有 `OPENDAN_AGENT_STATE_URL` / `--state-url`）。

## 5. 验证

在 `src/` 下：`cargo test -p libopendan -- --test-threads=1`（新增 `tests/outbound.rs` 4 例）；`cargo test -p opendan -- --test-threads=1`（Supervisor、真 kRPC、出站经 Loader 的端到端）。Schema 已用 `xagent schema ../doc/opendan/protocol/schema` 重新生成（`session_state`、`session_config`、`registry_entry` 三个文件变化）；fixtures 没有重新生成（新字段都可缺省，现有 fixtures 仍通过）。

## 6. DV 验收中补的下层修改（2026-10-04）

真实 zone 验收（[opendan-agent-loader-refactor-todo.md](opendan-agent-loader-refactor-todo.md) §10.6）发现的问题，先于 Loader 修：

1. `BehaviorConfig::overlay_llm_context`：`tool_whitelist` / `action_whitelist` 的条目生成 `{"name": …}`（原来带 `kind`，被 xllm 的配置解析按未知键拒绝）；`group:<name>` 生成 `{"groupname": …}`，behavior 才能写出“内置 bash 组 + `call_behavior`”。
2. `OpenTurn.has_msg`（`state.json`，为 false 时省略）与 `TurnReply.has_msg`：Turn 的输入里是否有消息。原来按输入 id 前缀 `cymsg:` 判断，而 id 是 `<source>#<index>`，失败提示永远不发。
3. `outcome.rs::fail_unattended`：`wait_user_msg = finish_failed` 的 Session 在不可重试错误 / 预算耗尽时 `finished + failed`；`one_line` 在没有答复时带 `error.message`，父 Session 的结束事件里能看到原因。
4. `agent_tool::runtime::native_host_id`：`$AGENT_TOOL_HOST_ID` 存在时返回 `host:<值>:`。

测试：`opendan/tests/loader.rs` 新增 `a_failed_turn_is_reported_to_the_speaker`、`a_failed_sub_session_is_reported_to_its_ui_parent`，`the_jarvis_package_loads_and_answers` 增加工具清单与 `max_tokens` 断言。

待做（单独立项）：behavior 循环里的原生工具调用没有写进 worklog（只有 `step` 与 `outcome`）。


## 7. 占位 / 最终 edit 与 Turn task（2026-10-04）

随 [messagehub-agent-chat-task-todo.md](messagehub-agent-chat-task-todo.md) 实施，对本文接口的增量：

- `OutboxEntry.purpose`（`reply` 缺省 / `placeholder` / `final_edit`）；占位的幂等键是 `<sid>:<turn>:placeholder`，与回复的键不同。
- `OutboundSink::placeholder(cfg, base, turn) -> Option<MsgObject>`（默认 `None`）；`TurnReply.has_placeholder`：为 true 时 sink 对任何状态都应给出收尾正文，给不出时 runner 用固定英文摘要。
- `runner::TurnTaskSink`（`open / update / close`）与 `RunnerDeps.turn_tasks` / `HostDeps.turn_tasks`；`state.turn_tasks`、`origin.parent_task`；`RunnerOptions.placeholder_delay`。
- `flush_outbox` 串行化（占位定时器与 drive 循环都会进来）；占位结果落定后修正同一 Turn `final_edit` 的 target（按 msg-center 返回的 `msg_id`），占位失败则把它改回 `reply`。
- 无 live run 的 stop 现在也走 `queue_reply`（只在有占位时才会真的产生消息）。
- 恢复语义不变：同一键、同一 MsgObject 重发；新增故障点 `turn_task:after_open`、`turn_task:after_close`、`outbound:after_placeholder_commit`（`tests/turn_task.rs`）。
