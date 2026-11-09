# Session Input Protocol（Agent 输入）

版本 3（`opendan.session_input/3`）· 2026-10-03 · 由 `libopendan` 反写（`src/protocol/input.rs`、`src/channel/{mod,kmsg}.rs`、`src/runner/{inputs,input_view,assembler,receipts,live,drive,children}.rs`、`src/bridge/`）。§10（Sub Session）与内部输入源 `_bootstrap` / `_child` 随 session_config/5 加入。

总线上的记录有三种：`msg`、`event`（Agent 的输入）和 `control`（Session 控制，见 [Session Control Protocol](<Session Control Protocol.md>)）。本文说明记录的线格式、接收方如何处理、输入如何成为上下文里的 user 消息，以及崩溃后如何恢复。输入批次与逻辑 Turn 的关系见 [Session Directory Protocol](<Session Directory Protocol.md>) §4。机器可读样例与逐字节渲染结果在 `fixtures/14_input_bus/`。

## 1. 通道

| 通道 | 命名 |
|---|---|
| kmsg 队列 | 名称 `opendan.session.<H(agent_did,sid)>`，URN `<appid>::<owner>::opendan.session.<H(agent_did,sid)>`；哈希为参数数组紧凑 JSON 的 SHA-256 完整小写 hex（固定 64 字符），隔离共享驱动 App 的不同 Agent；由创建者（驱动者 App 身份）创建，`sync_write=true`、`keep_acked=false`（确认即删除）、`other_app_can_write=true` |
| kmsg 订阅 | `opendan.<H(agent_did,sid)>`（sub id 在全部队列与 App 间共用一个命名空间）；创建时 `Earliest` |
| kevent 唤醒 | `/opendan/<agent_id>/session/<sid>/input`，投递后发布 `{"sid": …}`；只表示“队列可能有变化”，重复与丢失都由轮询兜底，不对应任何推理入口 |

Session 不直接读 msg-center：msg bridge 消费 inbox 后投递到总线（§7）。`agent_id` 默认取 agent DID 最后一段并把字母、数字、`_ - .` 之外的字符替换为 `_`。

**kmsg 使用规则**：`create_queue` / `subscribe` 返回“already exists”视为成功（并用 `get_queue_stats` 确认队列）；读取用 `read_message(queue, acked_index + 1, n)`，不依赖服务端游标；`commit_ack` 只提交 state.json 已提交的**连续**消费位置（累积确认）；遇到 “Subscription not found” 以 `At(acked_index + 1)` 重新订阅。队列是生产者-消费者语义：确认过的记录由 kmsg 删除（不声明 `keep_acked`），消费正常时队列基本为空；Session 自己不调用 `delete_message_before`，也不设 `retention_seconds` / `max_messages`（它们不看消费进度）。

**队列释放**：Session 不再接收输入时（finished，且 acceptance 不是 `pending` / `accepted`、也没有排队的 `decide`；`accepted` 仍可被 `discard`），驱动者在结束它的那次推进末尾删除队列；之后的推进不再打开输入源。向这样的 Session 投递任何记录都在登记表入口返回 `session_finished`。

**队列丢失**：队列名由 `sid` 决定，kmsg 数据在节点本地，可能丢失而 Session 目录还在（如重装）。此时只由驱动者重建（仍接收输入的 Session）：每次推进在恢复 run 与 receipt 之后、读取输入之前检查队列，不存在则先把该源的消费进度（`inputs[src]`）清零并提交 state.json（worklog `control_applied{command: input_source_reset, input.src: "_runner", detail: {src, lost_acked_index}}`），再以同一名称创建队列并以 `Earliest` 订阅。新队列从 1 重新编号，所以必须先清进度再建：两步之间崩溃，下次仍看到队列不存在、再清一次；建好之后投递的记录不会被旧进度跳过。投递方不创建队列，返回可重试的 `queue_missing`（不确认上游，并唤醒驱动者）；旧队列里尚未消费的记录随数据一起丢失。

## 2. 逻辑记录

```jsonc
{
  "schema": "opendan.session_input/3",
  "type": "msg",                      // msg | event | control
  "key": "cymsg:…",
  "from": "app:msg-bridge@alice",
  "at_ms": 1790899200000,
  "payload": { /* 由 type 决定 */ }
}
```

| 字段 | 约束 | 含义 |
|---|---|---|
| `schema` | 必须等于 `opendan.session_input/3` | 线格式版本；其它值一律拒绝 |
| `type` | `msg` / `event` / `control` | 决定 `payload` 的结构 |
| `key` | 1–256 字节，不含控制字符 | 逻辑输入的去重键：重投沿用，不同输入不同。`msg` 的 key 必须是消息的 ObjId |
| `from` | 1–256 字节 | **投递者** principal（自报，只用于审计）；不是说话人，也不是权限依据 |
| `at_ms` | u64 | 投递时间；只用于展示与诊断，不参与排序 |
| `payload` | JSON 对象，序列化后 ≤ 250 KB | 更大的内容放 NamedStore 或 session 目录，只投递引用 |

kmsg 映射：headers `schema` / `type` / `key` / `from` / `at_ms`（全部为字符串），`Message.payload` 是 `payload` 的 UTF-8 JSON。开发文件队列（`DirMsgQueue`）实现同一个 kmsg 客户端接口，映射相同。消费侧由通道补上 `src`（`channels.inputs[].id`）与 `index`（源内单调的投递位置），producer 不能提供。

**容量**：每个 Session 的总线最多保留 64 条尚未提交消费的记录（三种类型共用）。满时 append 被拒绝，返回可重试的 `input_full`，不覆盖已有记录；已消费但还在等累积 ack 的位置不计入。容量检查与 append 在同一临界区内（`SessionRegistry::post_input`，`.opendan_agent_session/post.lock`）。bridge 遇到满队列不确认上游，稍后重投。

**旧 Session**：`state.json` / `session_config.json` 不是当前 schema 的 Session 在显式迁移前只读：`post_input` 返回 `session_readonly`，drive 返回 RecoveryBlocked。

CLI：`post <sid> --json <file | ->` 接受逻辑记录（`schema`、`from`、`at_ms` 可省略；`msg` 的 `key` 可省略，由 MsgObject 计算；出现 `src` / `index` 报错）；`post <sid> --msg <text> [--from <did>] [--attach <obj_id>[=<name>]]… [--reply-to <obj_id>]` 用构造 helper。

### 2.1 `msg`：直接使用 MsgObject

```jsonc
"payload": {
  "msg": { /* cyfs-ndn MsgObject v2，原样，不改写、不裁剪 */ },
  "delivery": { "from_name": "Bob", "conversation_name": "Dev Team", "record_id": "r-102", "tunnel": null }  // 全部可选
}
```

| 项 | 规则 |
|---|---|
| `key` | 等于 `msg` 的 ObjId（JSON 中的字符串形式 `cymsg:<hex>`）。消费侧用 `MsgObject::from_json_value_checked` 重新计算并核对，不一致 → `invalid_envelope` |
| 合法性 | `MsgObject::validate()` 通过、JSON 是规范形式；`content.content` / `refs` / `machine` 至少一项非空 |
| 说话人 | `msg.from`。总线不携带签名：可信度来自投递者（bridge 只投递 msg-center 已验证的记录）；权限只来自 Session 的驱动身份 |
| `to` / `to_session` | Session 不校验、不路由；只用于渲染所在会话和推出回复目标 |
| 附件 | `content.refs` 的 `DataObj`（有 ObjId）；本机文件先登记进 NamedStore |
| `delivery` | 各字段 ≤ 256 字节；未知字段忽略 |
| `delivery.context` | 为 `true` 时这条消息只是会话里较早的消息，作为其后输入的资料（如群聊请求附带的群上下文）：它不单独组成批次、不构成请求，说话人照常保留；缺省 `false`，不输出 |

构造 helper（只做构造，不引入新的线格式）：`text_msg(from, agent, text)`（`kind = chat`，带 `created_at_ms` 与 `nonce`）、`attach(msg, obj_id, name)`、`reply_to(msg, target)`、`PostedInput::msg(poster, msg, delivery)`（校验并以 ObjId 为 key）。

### 2.2 `event`：AgentEvent

```jsonc
"payload": {
  "subscription_id": "watch-t1",                 // 可选
  "source": { "kind": "task", "id": "t1" },      // object | session | task | timer | system
  "event": "updated",                            // 1–64 字节
  "seq": 7,                                      // 可选：来源内的版本号
  "summary": "任务 t1 的状态发生变化",              // ≤ 1 KB，超出拒绝；给 LLM 看，不用于机械判断
  "data_ref": null,                              // 可选：ObjId 或相对 session 目录的路径
  "terminal": false,
  "call_id": null                                // 可选：task 的结束，交出这个 task 的那次工具调用（§9）
}
```

`source.id` ≤ 256 字节，`system` 之外必填。未知的 `source.kind` 不因格式拒绝（没有订阅匹配时丢弃）。`source.kind = task` 的 `id` 就是工具结果（AgentToolResult 的 `task_id`）与 task 工具 / 命令使用的 task id。

### 2.3 校验与拒绝

投递时与消费时用同一个函数（`parse_record`）。投递时不合法直接报错、不入队；消费时不合法：标记为已消费、写 `input_rejected{input, reason, detail}`、不进入上下文、不卡住累积确认。结果是确定的：同一条记录无论何时、由哪种语言的 Runner 消费，原因相同。

| `reason` | 条件 |
|---|---|
| `unsupported_schema` | `schema` 缺失或不等于当前版本 |
| `unknown_type` | `type` 不是 `msg / event / control`（含旧的 `change`、`perception`） |
| `invalid_envelope` | `key` / `from` / `at_ms` 缺失或越界；`msg` 的 `key` 不等于消息的 ObjId |
| `payload_not_json` | payload 不是 JSON 对象 |
| `payload_too_large` | payload 超过 250 KB |
| `invalid_payload` | 必需字段缺失、类型不符、超出约束；`msg` 未通过 MsgObject 校验 |
| `unknown_command` | control 的 `command` 未知 |
| `session_finished` | finished 之后收到 `decide` 以外的输入 |
| `input_policy` | Session 的 `input_policy = none` 而收到 msg / Input event |

payload 中的未知字段忽略（producer 可以比 consumer 新）；未知 control `command` 拒绝。

## 3. 路由：记录到达后发生什么

每次路由（drive 入口、主循环每一轮、每个推理前检查点）按“输入源声明顺序、源内 `index`”依次处理未消费的记录，不按 `at_ms` / `created_at_ms` 排序。模板只改变正文，不改变这里的任何一行。

| 记录 / 情况 | 机械处理 | 对 LLM / Turn 的影响 |
|---|---|---|
| 被拒绝的记录；finished 之后的非 `decide` | 消费，`input_rejected` | 无 |
| `key` 在 `recent_keys` 中，或同一轮已出现过 | 静默消费 | 无（不会第二次进入上下文） |
| `control` | 由驱动者按投递顺序应用并提交 | 不是 Agent 输入 |
| `msg` | 留在队列，作为受控输入的候选 | 开启新 Turn 或并入未关闭的 Turn |
| `event`，匹配 active 订阅 | 记入 `inputs[src].accepted`，留在队列作候选 | 同 msg |
| `event`，匹配 semi 订阅 | 合并进 `state.pending_events`，消费 | 不触发推理；作为下一次受控输入的背景行（§6.6），模板不放置就不可见 |
| `event`，`source = task:<id>` 且该 task 在 `state.watched_tasks` 中 | 消费，`event_dropped{reason: watched_task}` | 只提前触发一次查询；task 的结束按 §9 作为 task 结果输入交回 |
| `event`，没有匹配的有效订阅 | 消费，`event_dropped{reason: unsubscribed}` | 无；timer、系统事件没有兜底例外 |

- 订阅匹配：给出 `subscription_id` 时必须存在且来源相符；未给出时取第一个来源相符的显式或隐式订阅。每个 Session 都有一个隐式 semi 订阅：用户时区（`_user_timezone`，`source = system:user_timezone`）。Goal Session（模板 `goal`）另有隐式 active 订阅 `_goal_activation`（`source = system:goal`）。UI Session（模板 `ui`，`group` 经 `base` 继承）另有隐式 semi 订阅 `_notify`（`source = system:notify`）。`system:<id>` 的订阅也匹配子 id `system:<id>:<sub>`。
- **通知的观察（`_notify`）**：Agent 的系统通知另投进它与用户的某个会话后，msg-center 在 Agent 自己的 INBOX 该会话写一条 `buckyos.notify.delivered` 记录；宿主（OpenDAN Loader 的 msg bridge）把它转成该会话 UI Session 的 `AgentEvent{subscription_id: "_notify", source: system:notify:<notify_id>, event: "delivered", summary}`（key `notify:<notify_id>:<会话>`）。每条通知是独立的半订阅状态（来源带通知 id），不唤醒 Session、不开 Turn，随下一批受控输入出现在快照中；`summary` 是一行 `[notify <id>] <kind>/<level> from <goal …|session …> at <UTC>: <title> — <message 摘要> (options: id=label, …) — check the current state with `agent-session notify show <id>``。会话还没有 UI Session 时宿主照常新建一代，回复坐标取自记录的 `reply_route`；UI Session 只在有受控输入时 bootstrap，单有观察不会开口。第一段只有“已投递”事件，状态由 Agent 按需查询。
- **Goal 问题的回答**：Goal 的 `questions` 由驱动者在 Activation 关闭前发成 `input` 通知（幂等键 `goal:<goal_id>:q:<sha256(text) 前 16 位>`）；通知结束后宿主投递 `AgentEvent{subscription_id: "_goal_activation", source: system:goal, event: "answer", summary}`（key `goal:<goal_id>:answer:<notify_id>`），成为 Activation 原因 `answer`（`ref` = notify_id）；该 Activation 关闭时问题记下 `status` 与 `answer`，Goal 头的 `<open_questions>` 带上它们。
- **Goal 的运行机会**：运行时（宿主或 CLI）向 Goal 当前 epoch 的 Goal Session 投递 `AgentEvent{subscription_id: "_goal_activation", source: system:goal, event, summary}`：`event = created | reopened | restarted`（epoch 开始，key `goal:<goal_id>:start:<epoch>`，每个 epoch 只投一次）、`scheduled`（定时，key `goal:<goal_id>:scheduled:<slot>:<missed>`，`missed` = 自上次以来经过的槽数，>1 表示停机期间错过的槽已合并为这一次）、`activate`（手动，key `goal:<goal_id>:activate:<request_id>`）。UI 转交的反馈是 Agent 自己发给自己的 `msg`（`delivery.from_name = "feedback from <来源>"`、`conversation_name = "goal <goal_id>"`）。积压的多个运行机会、Work 的结束（`_child`，Goal Session 按 `goal_id` 收取，跨 epoch 只收一次）与反馈合成一个 Activation 的输入批次，各自成为该 Activation 的 `reasons[]`。Goal 暂停时这些输入留在队列。
- subscribe / unsubscribe 与事件按投递 index 生效：`event A → unsubscribe → event B` 中 A 按当时有效的订阅处理，unsubscribe 清理该订阅尚未注入的状态，B 丢弃。已被 active 订阅接受的事件不因之后的 unsubscribe 改判。
- 子 context 进行期间，msg / Input event 留给调用方、留在队列（子 context 只收它自己启动的 task 的结果）；恢复到工具批次 / Step 中途的 run 先把这批跑完，再放输入批次；control 始终处理。run 为它启动的工作保活期间（§9）没有这个限制：新到的 msg / event 照常注入同一个 run。
- 先提交 state.json（消费位置、`pending_events`、worklog），再确认输入源。

## 4. 消费进度（state.json）

```jsonc
"inputs": { "q": { "acked_index": 118, "consumed_above": [121], "accepted": [120] } },
"recent_keys": ["cymsg:…", "…"]              // 有界（256）
```

- `mark(i)`：`i ≤ acked_index` 忽略；否则加入 `consumed_above`，再把连续前缀折叠进 `acked_index`（永不回退）。
- `accepted`：已被 active 订阅接受、尚未进入批次的事件位置。
- pending 条数 = `last_index − acked_index − |consumed_above|`。
- 不承诺跨任意时间、跨协议版本的逻辑去重：`recent_keys` 只覆盖近期重投，投递位置与 receipt 负责恢复幂等。

## 5. 半订阅状态（`state.pending_events`）

```jsonc
"pending_events": [ {
  "subscription_id": "watch-obj",
  "source": { "kind": "object", "id": "doc-1" },
  "latest":   { "seq": 8, "key": "obj:doc-1:8", "event": "changed", "summary": "…", "data_ref": null,
                "received_at_ms": 0, "input": { "src": "q", "index": 131 } },
  "terminal": null,                 // 终结事件单列，结构同 latest，不被后续版本覆盖
  "superseded": 2                   // 注入前被覆盖的版本数
} ]
```

- 合并键 `(subscription_id, source)`。两侧都有 `seq` 时，较旧的版本不能覆盖仍保留的较新版本；没有 `seq` 时按消费顺序更新，用 `key` 区分版本。同一 `key` 再次到达不改变状态。
- 空闲时保留。只在被注入（由 receipt 精确清理）、取消订阅或被新版本覆盖时清除；Session finished 时清空。
- “已接收保存”不等于“已注入上下文”：保存与消费同一次提交，receipt 只记录输入消息里实际出现的版本。
- 拉模式（订阅其它 session）：内置 bridge 比较登记表 `status.rev` 与 `subscription_cursors[sub].rev`，被 watch 的字段变化时合成 `AgentEvent{source: session:<sid>, seq: rev}`（key `session:<sid>@<rev>`）并入 `pending_events`；拉模式订阅总是 Observe。

## 6. Input 链路：从记录到 user 消息

```text
SessionInput{msg, delivery}        总线：持久、有序、至少一次
   │ 选批      校验、去重、input.mode、Turn 归属；MsgObject → 模板视图（进程内，不是协议对象）
   ▼
InputView (input.*)
   │ 渲染      behavior 的 on_init / on_input / on_context_switch 模板（缺省内建）；纯函数
   ▼
文本 + 媒体块
   │ 落盘      一条 user AiMessage 与 receipt 写进同一份快照
   ▼
AiMessage{role: user}              恢复、接手、压缩都从快照与 worklog 读取，不回到总线重新渲染
```

不变量：结构只编码一次（渲染之前 MsgObject 是消息结构的唯一载体；总线与 state 中不出现 `AiMessage`）；只渲染一次；文本自足（每个附件都以 ObjId 或可读路径出现在文本里，去掉媒体块仍可定位）；正文不能伪造结构（文本与属性值在渲染时转义，`from` / `key` / `mentioned` 等只来自结构字段）。

### 6.1 三类受控输入与消费策略

| 入口 / 模板 | 触发 | 边界 |
|---|---|---|
| `on_init` | Session 首次启动 | 不要求有外部输入；只在 bootstrap 使用 |
| `on_input` | 选中的 msg / Input event | control、Observe 事件、队列通知本身不构成此入口 |
| `on_context_switch` | 交接：切换 behavior、进入子 context、经交接批次返回的子结果（behavior 触发与工具触发相同） | 延续当前 Turn |

一次装配只选一个入口（`on_init` → `on_context_switch` → `on_input`），允许消费的外部输入并入该批。`input.mode`：`batch`（默认，批次预算内取多条）或 `single`（总共取一条），未选的留在队列。`delivery.context` 的消息不计入预算，随其后第一条被选中的输入一起进入批次；其后还没有输入时整组留在队列，不单独触发。模板与策略取自接收批次的 context（`prompt.*` / `input.*`，behavior 条目按字段覆盖 Session 的）。模板报错、或已有选中的输入却渲染出空文本：报错并保留现场，不消费、不确认。

### 6.2 内建渲染（跨语言逐字节一致）

`input.text`（= `{{ input | render_format: "input.xml" }}`）：

```xml
<inputs>
<msg key="cymsg:…" from="Bob" from_id="bob.bns.did" group="Dev Team" mentioned="true" reply_to="cymsg:…" time="2026-10-02T00:00:00Z">
@jarvis 帮我看一下这张截图里的报错 &lt;build&gt;，日志在附件里
<attachment index="0" media="image" name="screenshot.png" mime="image/png" obj_id="cyfile:…"/>
<attachment index="1" media="document" name="build.log" mime="text/plain" obj_id="cyfile:…"/>
</msg>
<event key="task:t1:revision:7" subscription="watch-t1" source="task:t1" event="updated" seq="7">任务 t1 的状态发生变化</event>
</inputs>
```

| 元素 / 属性 | 来源 | 规则 |
|---|---|---|
| `<msg key>` | 信封 `key` | 总是输出；属性顺序固定为下列各行的顺序 |
| `kind` | MsgObject `kind` | 仅 `chat` / `group_msg` 之外时 |
| `from` / `from_id` | `delivery.from_name` / `msg.from.to_raw_host_name()` | 没有显示名时 `from` 取 id，不输出 `from_id` |
| `group` | `delivery.conversation_name`，缺省取 `to[0]` | 仅群消息 |
| `session` | `to_session` | 非空时 |
| `mentioned` | `mentions.dids` 含 Agent DID | 仅为 true 时（`all` 不算） |
| `context` | `delivery.context` | 仅为 true 时 |
| `reply_to` / `edit_of` / `redacts` / `thread` | `thread.reply_to`、`relates_to` | 值为目标消息的 ObjId |
| `format` | `content.format` | 仅 `text/*` 且非 `text/plain` 时 |
| `title` | `content.title` | 非空时 |
| `time` | `created_at_ms`，缺省取信封 `at_ms` | UTC RFC 3339，`Z` 后缀，秒精度 |
| 正文 | `content.content`（trim） | 转义 `& < >`；非空时独占一行。正文只是 `[attachment]` 等占位词且有附件时置空 |
| `<attachment index media name mime obj_id path role/>` | `content.refs` 的 `DataObj` | 每个一行，按数组顺序编号；`media` / `mime` 由 label、uri_hint、消息 format 推断；`path` 仅在宿主已有可读映射时输出；`role` 仅非 `input` 时 |
| `<event key subscription source event seq data_ref terminal>` | AgentEvent | `source` 为 `kind:id`；正文是转义后的 `summary`；可选属性缺省不输出 |
| `<<task_result task call_id status>>` … `<</task_result>>` | Runner 合成的 task 结束（`source.kind = task` 且没有 `subscription_id`，§9） | 不用 `<event>`：独立块，块标签各占一行；`task` 为 task id，`call_id` 有值才输出，`status` 为事件名（`finished`，含失败，成败看正文；状态查不到时 `unknown`）；正文是 task 的结果，原样输出（不转义） |

属性值转义 `& < > "`。只有一条纯文本消息时也使用 `<msg>`。`machine` 与 `ServiceDid` 引用不在内建渲染中出现。

内建模板（UI Session 除外，见 §6.5）：`<session_input hook="…" time="<批次时间>">` 元素，依次包含交接状态（`<context_switch to>`、`<process_result>`、`<sub_task>`）、bootstrap 的 `<task>` / `<scope>`、`input.text`、`<perceptions>`、`<hints>`（每条 `<hint id time [object] [note]>`，`note` 是 §6.6 `hint:<id>` 行的 `qualifier`：证据性质、范围与有效期）、活动 session 列表、`<runtime>`；有变化的背景行时，`<<background_context>>` 块放在 `<session_input>` 之前（§6.6）。

协议时间与内建渲染一律 UTC。用户时区与 Session 绑定（`session.timezone`，IANA 名），其初始值与后续变化经隐式 semi 订阅进入半订阅状态，不取 Runner 所在机器的时区，变化本身不唤醒推理。

### 6.3 自定义模板

模板引擎是 `agent_tool::prompt_engine`（upon，`__EXEC__` 关闭）。只含块标签（`{% for %}`、`{% if %}`、`{% endfor %}` …）的行不产生输出行；输出去掉首尾空白。可用变量：`input.{hook, time, messages[], events[], items[], text, count}`（视图字段见 `runner/input_view.rs`；条件用布尔字段 `msg.is_group`、`msg.mentions.me`、`msg.context`、`ev.terminal`、`item.is_msg`）、`session.*`、`runtime.*`、`handover`、各内建块（`task_text`、`handover_text`、`perceptions_text`、`hints_text`、`active_sessions_text`、`runtime_text`、`builtin`）、背景 `background` / `background_context`（§6.6）。

命名格式 `{{ value | render_format: "名称" }}`（未知格式或对象形状不符是模板错误；声明支持空值的格式对空值输出空串）：

| 格式 | 输入 | 结果 |
|---|---|---|
| `input.xml` | `input` | `<inputs>` 块，与 `input.text` 逐字节一致 |
| `message.xml` / `event.xml` | 一条消息 / 事件视图 | 内建元素，与 `input.text` 中的同一条一致 |
| `message.markdown` | 消息视图 | `## Request from …` 标题、引用正文、附件清单 |
| `event.summary_text` | 事件视图 | 单行：来源、事件名、终态标记、摘要（≤ 300 字符） |
| `attachments.xml` / `attachments.list` | `msg.attachments` | 每个附件一行 XML / 一行文本清单（有路径用路径，否则 ObjId） |
| `todo.summary_xml` | 宿主装配的 `{id, status, summary \| title}` | `<current_todo …>`；空值输出空串 |

底层通用 filter（`agent_tool::prompt_engine` 注册，system 模板同样可用）：`xml`、`attr`、`json`、`truncate: n`、`oneline`、`default: v`、`join: sep`、`quote`、`time: fmt`（UTC）。XML 格式自己转义，模板直接插入；直接拼装 XML 的模板自行用 `xml` / `attr`。渲染后 Session 逐个检查附件：文本中既没有其 ObjId 也没有可读路径时记 warning（显示名不算定位信息）。

### 6.4 媒体块（`input.media`）

与模板无关，由接收批次的 context 的 `input.media` 机械决定：`reference`（默认）只有文本；`inline` 在文本块之后按消息顺序、附件顺序为 `image` / `document` 附件追加 `Image{source: named_object}` / `Document{source: named_object, title}` 块，每批最多 8 个，其余只保留文本引用。两种取值下文本块相同。worklog 的 `user_message` 只记文本，新 run 重建的历史因此只有文本。

`inline` 的请求被 provider 拒绝（永久或未知的 provider 错误：模型不接受图片 / 文档，或对象不可读）时，Session 机械降级一次：去掉该 run 中 user 消息的媒体块，在其文本末尾追加一行说明，发布改写后的快照并重试；快照的 `host.libopendan.media_degraded = true` 记录它已发生，同一个 run 不再降级。文本自足保证附件仍可按 ObjId / 路径读取。

### 6.5 UI Session 的 chat 格式

`session.kind = ui`（一对一与群聊的会话 Session）的内建模板是 chat 格式：只有变化了的背景行，然后是输入本身。没有 `<session_input>` 包装、`<task>`、`<runtime>` JSON、`<active_sessions>`。

```text
<<background_context>>
- current_time 2026-10-16T10:45:45-07:00 (America/Los_Angeles)
- runtime native on lzc-pve, cwd …/sessions/<sid>
- work work-12481d…: running — reviewing PR #667 diff
- [notify n_01] input/timely from session work-… : Which branch? … — check the current state with `agent-session notify show n_01`
- hint: owner prefers replies in Chinese (the agent's inference, not confirmed; applies to workspace:proj-a; noticed 2026-10-02)
<</background_context>>
你好 Jarvis
```

事件批次（没有消息）：

```text
<<inputs>>
<event key="child:work-12481d…:finished@26" source="session:work-12481d…" event="finished" seq="26" terminal="true">sub session work-12481d… finished (succeeded): 1 P2 issue found — read: agent-session read-session work-12481d… --report</event>
<</inputs>>
```

背景行（每行一个稳定 key；文本为一行）：

| key | 内容 | 来源 |
|---|---|---|
| `time` | `current_time <用户时区的本地时间> (<IANA 名>)`；没有 `session.timezone` 时 UTC | 批次时间 + `session.timezone` |
| `runtime` | `runtime <kind> on <hostname>, cwd <cwd>` | 运行时状态 |
| `channel` | 本批最后一条消息经过的隧道 DID | `delivery.tunnel` |
| `work:<sid>` | 本 Session 创建、仍在运行的子 Session：状态与活动摘要。它的结束、等输入、等决定是它的事件，不成行 | 活动视图（`parent = 本 sid`） |
| `ui:<sid>` | 其它 UI Session 的 topic 标题 | 活动视图（登记表 `status.topic`） |
| `session:<sid>` | 与本 Session 目标相同或范围重叠的其它活动 Session（避让规则） | 活动视图 |
| `event:<key>` | 半订阅状态版本：`_notify` 的通知行原样（`[notify <id>] …`）；其它为 `<source> <event>: <summary>`。用户时区事件不单列（由 `time` 行承载） | `pending_events`（同 §5 的选择与预算） |
| `hint:<id>` | `hint: <sentence> (<qualifier>; noticed <time>; about <object url>)`。`qualifier` 说明这条线索凭什么成立、适用于哪里、何时结束，缺哪项就省哪项：证据性质（`the user's explicit statement` / `stated by the user` / `observed` / `a third party's claim, not verified` / `the agent's inference, not confirmed`，confidence 低于 0.5 另加 `weak evidence`）、范围（`applies to <workspace 主体与 scope 对象，至多 3 个>`、`not to <例外>`）、有效期（`holds until <valid_until>`，没有期限的 volatile 为 `short-lived`） | 按 `state.topic.tags` 召回的 hints（`Hint::note()`） |

- **只显示变化的行**：`state.background_shown` 记录每个 key 上次显示时的内容摘要；摘要变了才显示。`time` 行另按 5 秒粒度：距上次显示不足 `BACKGROUND_TIME_STEP_MS = 5000` 且时区未变时不显示。本批候选里没有的 key 从记录中删除（再出现时重新显示）。所有行都没有变化时整个 `<<background_context>>` 块省略。
- 新记录随批次 receipt 提交（`extra.background`，见 §7），恢复时与 receipt 一起应用；显示了哪些半订阅版本仍记在 receipt 的 `events` 里。只有输入消息里实际出现的行才记为已显示（§6.6）。
- **消息**：一对一时只写原文，不写 key / from / time；附件每个一行 `[attachment <media> "<name>" -> <path 或 ObjId>]`；回复 / 编辑等关系有才写一行（`(replying to <ObjId>)`）。一个批次有多条消息时每条前加一行 `[<发言人> <本地时间 HH:MM:SS>]`。**群聊**保留 §6.2 的 `<msg>` 元素（发言人与 `context="true"` 是群聊语义的根本），与事件一起放在 `<<inputs>>` 里。
- **事件**：保留 `<event>` 元素（属性是用 CLI 回查的依据），放在 `<<inputs>> … <</inputs>>` 中、消息原文之前；子 Session 事件的摘要对 UI 父 Session 只有一行，不附 `report:`，结束时末尾附 `— read: agent-session read-session <sid> --report`。
- 交接块（`<context_switch>` 等）与 TodoList 视图有时照旧放在最前。自定义模板可用 `background`（行数组）与 `background_context`（整块文本）。

### 6.6 背景是输入的素材

半订阅永远不是一条独立的输入，只能在输入里被使用：每个受控输入批次算出自上次显示以来变化的背景行，作为模板变量 `background`（行数组，`{key, text}`）与 `background_context`（整块 `<<background_context>>` 文本，无变化时为空串）交给该批次的模板。模板不引用它，半订阅对这个 Session 就不可见。没有单独的快照消息，工具结果之后也不注入背景。

- 候选行：UI Session 为 §6.5 的各行；其它 Session 为半订阅状态版本（`event:<key>`，格式同 §6.5，用户时区事件也成行）；所有 Session 另有在跟的运行中 task 的新输出（`task:<id>`：`task <id> · <命令摘要> · running <时长> · new output:` 加自上次显示以来的新输出，最多 2 KB，超出只留末尾）。
- 子 Session 的结束、等输入、等决定，task 的结束，Todo 的状态都不成行：它们是事件（或调用结果、命令输出）。
- 已显示的判定：渲染后的输入文本包含该行文本才算显示。未出现的行保持原记录（下次放置它的输入照样显示），未出现的状态版本不进 receipt 的 `events`，留在 `pending_events`。
- `task:<id>` 的记录值是已显示到的输出位置 `<stdout 字节>,<stderr 字节>`；调用转为 task 后的第一个推理边界记下当时位置（调用结果已显示到这里），不产生消息。
- 内建模板（§6.2、§6.5）都放置背景；Jarvis 的 `plan_simple` / `work_simple` 在 `on_input` 首行写 `{{ background_context }}`。

## 7. 输入 receipt 与提交

每个批次是一条 user 消息（Goal Session 开启 Activation 的批次前面另有一条 Goal 头部），一次注入，中间不推理、不发布快照。receipt 与消息写在**同一份**快照的 `state.host.libopendan.input_receipts[]`：

```jsonc
{ "run_id": "…", "input_seq": 2, "turn": 5, "opens_turn": true,
  "hook": "on_input",                                   // on_init | on_input | on_context_switch
  "inputs": [ { "src": "q", "index": 121, "key": "cymsg:…", "kind": "msg" } ],
  "events": [ { "subscription_id": "watch-obj", "source": { "kind": "object", "id": "doc-1" }, "seq": 8, "key": "obj:doc-1:8" } ],
  "reply":  { "route": "message", "to": "did:bns:dev-team", "to_session": null, "kind": "group_msg", "reply_to": "cymsg:…", "tunnel": null },
  "parts": [ { "part": "input", "pos": { "kind": "accumulated", "index": 7 }, "text": "<<background_context>>\n- object:doc-1 changed: …\n<</background_context>>\n<session_input hook=\"on_input\" …" } ],
  "bootstrap": false, "after_step": 7, "extra": { "continuation": true }, "at_ms": 0 }
```

- `extra.background`：本批之后的 `state.background_shown`，应用 receipt 时整体替换。
- 批次 ID = `(run_id, input_seq)`，每个 run 从 1 开始连续。`opens_turn` 只看提交时有没有打开的 Turn。
- `inputs`：本批消费的 msg 与 Input event。Runner 自己合成的输入（Session 在跟的 task 结束，§9）用 `src = "_task"`、`key = task:<task_id>:terminal`，没有消费位置。
- `events`：输入消息里实际出现（作为背景行）的状态版本；应用时按 `(subscription_id, source, key)`（有 `seq` 时一并核对）从 `pending_events` 精确清除。处理 v7 期间收到 v8，提交 v7 不删除 v8。
- `reply`：本批提交后的默认回复路径（`state.reply`）：本批按消费顺序最后一条 msg 的来路（一对一 → 原发送者；群消息 → 原群；带 `to_session`、`tunnel`、`reply_to`）；本批没有 msg 时沿用原值；从未有过 msg 而有父 Session 时为 `{"route": "parent_session", "session_id": …}`。event、control、背景、交接本身不改变它。
- `parts[].pos`：function call → `accumulated`（Goal 头部与输入相邻）；behavior 首个 Step 之前 → `request_input`；已有 Step → 各 part 都是 `step{index}`（并入该 Step 的 `next_user_message`）。`text` 是该消息的文本块；媒体块按 `pos` 在同一份快照中读取。
- 应用 receipt（幂等：`input_seq ≤ live_run.applied_input_seq` 忽略；必须恰好是下一批；`opens_turn = false` 时 `turn` 必须等于 `open_turn.index`）：更新 `live_run.turns` 与 `open_turn`，标记 `inputs` 已消费并记入 `recent_keys`，清理 `events`，还原 `reply`，置 `bootstrap_done`，`extra.continuation` 清除 `internal_continuation` / `process_result`，`run_state = running`。
- **提交顺序**：① 快照 fsync ② run.json 发布快照指针与 `host_commit_pending = input_seq` ③ state.json 应用 receipt ④ 清门槛 ⑤ 确认输入源。④ 之前不得推理或调用工具；可恢复提交之前不出队。
- **恢复**：先按快照中的 receipt 补齐 state（不重新渲染、不重新注入、不读总线），再取新输入；state 已应用而快照缺失的批次 → RecoveryBlocked。
- 检查点（每个工具调用 / action 派发之前；以及推理之前：function call 每次推理前，behavior 只在 Step 边界）发布快照；推理前的检查点另做一次路由并刷新心跳：到达检查点、工具完成或收到 Observe 事件都不独立注入消息。
- 推理 / 装配预算不足按 AICC 服务不可用的失败与重试路径处理：未提交的批次不消费，已提交的批次保留 receipt，恢复时不重复注入。

## 8. bridge

bridge 只转换来源并可靠投递（先 `post`，成功后再确认上游；重投由 `key` 去重）；不渲染、不决定投递策略、不写 Session state。

**msg bridge**（`bridge/msg.rs::route_msg_record`，纯函数）：MsgObject 原样上总线，只做过滤、分流和补 `delivery`。按顺序：记录发送者与 `msg.from` 不一致 → 丢弃；群邀请通知、Agent 自己的群消息回显 → 丢弃；正文 / refs / machine 全空 → 丢弃；`relates_to.rel = reaction` → 丢弃；发送者身份（`record.ingress.extra.principal_did`，没有时 `record.from`）不是 Agent 的 Owner → 丢弃（推理之前过滤，正文里的名字或声明不算）；群消息在宿主不允许群聊时、或 `mentions.dids` 不含 Agent DID 时 → 丢弃；纯文本且匹配已登记斜杠命令 → 映射到 Session 控制的命令投递 `control`（`/stop` → `stop`，key `ctl:<ObjId>`），其它已登记命令由应用自行处理；其余（含 `edit` / `redact`）原样投递。Session 不解析正文：未被识别的斜杠文本就是普通消息。`context_msg_record` 把同一会话里较早的记录（任何人的，Agent 自己的除外）转成 `delivery.context = true` 的上下文消息，供宿主随请求一起投递。

**task bridge**（`bridge/task.rs::task_event`）：把 task 状态映射成 `AgentEvent{source: task:<task_id>}`（`updated` / `finished` / `unknown`，终态 `terminal = true`）。通知只加速检查，task 状态 API 才是权威来源。

**反向链路**：assistant 消息按 `state.reply` 转成 MsgObject（`bridge::outbound_base` 给出信封，`llm_context::ai_message_to_msg_object_with_base_validated_async` 填内容），出站记录 `{key: "<sid>:<turn>:<run_id>:<n>", msg}` 幂等。记录在 Turn 关闭的提交里写入 `state.outbox`，提交后交给驱动者的出站 sink（OpenDAN：msg-center `post_send(msg, idempotency_key = key)`），重启后原样重发；`channels.outbound` 是创建时固定的回复坐标，出站时核对（[Session Directory Protocol](<Session Directory Protocol.md>) §4 `outbox`）。耗时较长的 Turn 先发占位、结束时以一条 edit 替换，并在锚点消息上携带 `agent_task`（同节 `outbox[].purpose` / `turn_tasks`）。

## 9. 长任务：调用内等待、保活与 task 结果

loop 里不等待，任何等待都是 task。

- **调用内等待**：工具的工作在 runtime 的等待预算内完成时，调用直接得到结果。预算是 `.llm_context` 的 `runtime.task_wait_ms`（缺省 30 秒、上限 30 分钟；工具结果给出 `check_after` 时用它）。到期仍未完成的工作是一个 task，调用返回一条普通结果“task X 仍在运行”（AgentToolResult 信封 `status = pending`，带 `task_id` 与说明用的 `pending_reason`）。Session 中这条结果的提示说它的结束会作为输入回来、不必等待，并给出随时查看的方法（`get_task_state`；UI Session 的 function call run 是 `agent-session task state <id>`）。
- **本 run 启动的工作**：结果首次引用、仍在运行的 task 记入 run.json `host.extra.tasks`，交出它的调用记入 `host.extra.task_calls`（[Session Directory Protocol](<Session Directory Protocol.md>) §7）。
- **保活**：模型回复（function call 的 `Done`）或 Yield 时这些 task 仍在运行，run 保活：不结束、Turn 保持打开（多 Turn 的 Work Session 由这次回复关闭 Turn）、不推理，task 记入 `state.watched_tasks`（Session Directory Protocol §4）。之后的输入都注入这个 run。
- **task 结果是输入**：每轮路由后用宿主的 `RunningTaskResolver` 按保存的 task id 查询 `watched_tasks`，与通知、inbox 是否为空无关；task 的通知只提前触发查询（§3 `watched_task`）。task 结束（或状态查不到）时合成 `AgentEvent{source: task:<id>, event: finished | unknown, terminal: true, call_id}`（内部源 `_task`）：有显式订阅时按其 active / semi 处理，否则是 Input，进入下一个受控输入批次（run 保活时注入这个 run）。内建渲染是独立块（§6.2）：

  ```text
  <<task_result task="local:shell:c7" call_id="c7" status="finished">>
  …task 的结果，与调用当场完成时得到的相同…
  <</task_result>>
  ```

  正文最多 16 KB（16×1024 字符），更长的截断并注明用 task id 读全文。它不伪装成工具结果：调用当初已经有了“仍在运行”的结果，模型凭 `call_id` 与 task id 把两者对上。
- 查不到的 task（进程内 task 换了进程、没有 task 管理器认识它）按 `unknown` 交回，由 LLM 判断，不重建、不重放任务。
- **run 结束后的 task**：run 结束而 Session 未 finished 时，仍在运行的 task（`host.extra.tasks` 与 resolver 报告的）记入 `state.watched_tasks`，之后同样以 task 结果输入交回，可能开启新 Turn。finished 的 Session 清空 `watched_tasks`，不因 task 完成而重开。
- stop：本 Turn 可取消的 task（resolver 报告在运行的，以及 run 记录的）被取消，run 以 Stopped 结束。工具执行期间由监视任务查看队列中的 stop 并打断运行（只查看，不消费、不写 state）。
- pause / resume（`control(pause | resume, reason?)`）：pause 记入 `state.paused = {at_ms, reason, by, in_run}`，Session 不再推理：不打开新 Turn（到达的 msg / event 留在队列，task 的结束留在 `watched_tasks`，不产生 tick），运行中的 run 在下一个推理边界（在飞行的调用返回、检查点写入之后）以中断原因 `session paused` 停下，run 保留在检查点、Turn 保持打开（worklog `outcome.kind = paused`，`in_run = true`）。在跟的 task 按 OS 进程语义继续。resume 清除 `paused`；被暂停在 Turn 内的 run 置 `state.resume_run`，下一次推进从检查点接着推理（与崩溃恢复同一路径），Turn 之间暂停的则由积压的输入打开下一个 Turn。登记表 `status.paused`；暂停中的 Session 不报 `needs_input`。对已结束的 Session 是空操作（`applied: false`）。
- 经外部服务创建任务的工具以 run 与 call 的稳定身份导出幂等键（如 shell 交给 Zone TaskMgr 的 `shell:<run_id>:<call_id>`）；shell 命令的环境里有 `XLLM_RUN_ID` / `XLLM_CALL_ID`。
- 崩溃恢复：调用中途崩溃时，恢复从派发前的检查点再派发它，由 runtime 的工具管理器按 `call_id` 找回（Session Directory Protocol §10）；run 的终态已落盘而 state 尚未提交时，重做的结束从 run.json 的 `host.extra.tasks` 取回仍需跟踪的 task；保活决定（`host.extra.kept`）同样按记录重做。task 结果只交付一次。
- **向用户提问**：提问是一个普通的 task。websdk 命令 `ask_user <question> [--option <id>=<label>]... [--allow-text] [--expires <secs>]`（`src/tools/buckyos-agent`）发出一条 `input` 系统通知，即 Zone TaskMgr 上的 `human.input/v1` 任务（幂等键 `ask:<run_id>:<call_id>`，取自 `XLLM_RUN_ID` / `XLLM_CALL_ID`，所以它必须是一条独立的 shell 命令），返回该 task 的 pending 结果（`pending_reason = user_input`）；回答是 task 的结果，按上面的规则以 `<<task_result>>` 交回，过期 / 撤销也是它的终态。Session 不解析问题，也不把下一条消息当作答案：提问期间用户发来的消息照常是新输入。Goal 不用它提问，问题写进 Goal 结果的 `questions`（§3）。

## 10. Sub Session：创建、汇报与等待

Sub Session 是 `origin.parent_session` 指向父的普通 Session：自己的目录、state、worklog、Turn 与 lease，驱动者默认是父的驱动者。父子都只写自己的 state.json，彼此只经登记表、输入通道与控制协议沟通；登记表里的状态是真相，下面的事件只是加速。

- **创建**（参考实现 `api::create_sub_session`，CLI `agent-session create-worksession`）：`origin = {parent_session, report, created_by_call}`。幂等键由 `(父 session_id, run_id, call_id)` 导出：同一次调用重放得到同一个 Session。超过父的 `policy.max_sub_sessions`（同时未结束）或 `max_session_depth` 时拒绝。继承父的 `prompt.llm_context`（含 runtime 配置；绑定身份是子自己的）、默认共用父的工作目录。首批输入是所属 Agent 自己发出的 MsgObject：子没有队列时进 `prompt.initial_inputs`，有队列（interactive）时投递到队列。创建不推进子 Session。
- **隐式关注**：父的 Runner 按 `origin.parent_session = 自己` 查登记表（每次选批之前、以及等待期间的每次轮询），不需要订阅，父也不需要输入队列。`origin.report` 为 `none` 或缺省的子不产生任何事件。
  - 进度（仅 `report = progress`，子未结束）：`status.rev` 前进且 `{run_state, one_line_status, activity.summary}` 变化时，合成 `AgentEvent{subscription_id: "_child:<sid>", source: session:<sid>, event: "progress", seq: rev}` 并入 `pending_events`（Observe），游标存 `state.subscription_cursors["_child:<sid>"] = {rev, view}`。
  - 需要关注与结束：子 `finished` → `event = finished`（`terminal = true`，summary 含 `report_brief`）；有 `pending_decision` → `needs_decision`；登记表 `status.needs_input`（等待且没有在跟的工作）→ `needs_input`。它们是 **Input**：作为内部输入源 `_child` 的候选（`key = child:<sid>:<event>@<rev>`，index 0）进入受控输入批次，可开启或并入 Turn。候选在每次选批时重新合成，直到消费它的批次的 receipt 提交：应用 receipt 时把 `subscription_cursors["_child:<sid>"].attention` 置为该事件名，同一状态不再交付；子离开该状态后清空，下次进入是新的事件。`_child` 与 `_task` 一样没有总线消费位置。
- **没有“等一个子 Session”**：`agent-session wait` 与 `create-worksession --wait` 已删除（调用时返回说明）。Session 在 loop 之外跟进它的子 Session：模型结束回复即可，多 Turn Work 的 run 为需要汇报的子保活（`wait_children`），子的结束以 `_child` 的 `finished` 事件作为下一条输入。宿主的 resolver 仍能回答 `session:<sid>` 形式的 task（只读登记表，立即回答），但没有命令再产生它。
- **plan 发给 Todo 的 work session 的事件**：plan session 的 Todo 由宿主派出的 work session 带输入队列（`SubSessionSpec.queued`；不是 inbox，等待策略仍是 `finish_failed`）。plan 的 `todo update` 向运行中的 attempt 投一条 `event`：`source = todo:<todo_id>`、`event = updated`、`seq = Todo 的 rev`、`summary = "Todo Tx was updated (<字段>): <原因>"`（≤ 1 KiB）、`data_ref = <plan sid>#<todo_id>@<attempt>`、key `<plan sid>:<todo_id>:<attempt>#updated@<rev>`。有 `origin.parent_session` 的 Session 把 `source.kind = todo` 的事件当作 Active 输入接收（不需要订阅）；取入批次时宿主按 `data_ref` 读 plan 的列表，把 Todo 当前的完整任务接在 summary 后。模板变量 `is_todo_update`、`todo_id`。work session 在这类事件取入之前提交的 `report(is_end=true)` 被拒绝（可修正）。
- **父结束规则**：见 [Session Directory Protocol](<Session Directory Protocol.md>) §4 的 `wait_children` 与保活。`run`、`serve` 与等待循环在 `wait_children` 或 `watched_tasks` 非空（或 runner 的纠正待取）时轮询登记表 / resolver，不以“没有输入队列”为返回依据。
- **stop 级联**：父被 stop 或结束（结束时只针对需要汇报的子）时，对未结束且有输入队列的子投递 `stop`；没有队列的子由驱动它的宿主以驱动者自己的停止请求结束（`host::stopped_from_above`；见 [Session Control Protocol](<Session Control Protocol.md>)）。子被 stop 只在父那里表现为一次 `finished`（outcome stopped）。
- **pause 级联**：父被 `pause` 时，对未结束且有输入队列的子投递 `pause`（reason `parent session <sid> was paused`）；父 `resume` 时只对因这次级联而暂停的子投递 `resume`。
- **推进**：子由其驱动者身份的托管进程推进（参考实现 `host::ChildDriver`：轮询登记表，为每个未结束、未被驱动的子按其自己的配置装配 runtime 后 `drive(Idle)`，各持自己的 lease）。宿主进程退出只停止这些任务，不改变已提交状态；重启后从登记表重新接管。
