# Session Input Protocol（Agent 输入）

版本 3（`opendan.session_input/3`）· 2026-10-03 · 由 `libopendan` 反写（`src/protocol/input.rs`、`src/channel/{mod,kmsg}.rs`、`src/runner/{inputs,input_view,assembler,receipts,live,drive,children}.rs`、`src/bridge/`）。§10（Sub Session）与内部输入源 `_bootstrap` / `_child` 随 session_config/5 加入。

总线上的记录有三种：`msg`、`event`（Agent 的输入）和 `control`（Session 控制，见 [Session Control Protocol](<Session Control Protocol.md>)）。本文说明记录的线格式、接收方如何处理、输入如何成为上下文里的 user 消息，以及崩溃后如何恢复。输入批次与逻辑 Turn 的关系见 [Session Directory Protocol](<Session Directory Protocol.md>) §4。机器可读样例与逐字节渲染结果在 `fixtures/14_input_bus/`。

## 1. 通道

| 通道 | 命名 |
|---|---|
| kmsg 队列 | 名称 `opendan.session.<sid>`，URN `<appid>::<owner>::opendan.session.<sid>`；由创建者（驱动者 App 身份）创建，`sync_write=true`、`keep_acked=false`（确认即删除）、`other_app_can_write=true` |
| kmsg 订阅 | `opendan.<agent_id>.<sid>`（sub id 在全部队列与 App 间共用一个命名空间）；创建时 `Earliest` |
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
  "terminal": false
}
```

`source.id` ≤ 256 字节，`system` 之外必填。未知的 `source.kind` 不因格式拒绝（没有订阅匹配时丢弃）。`source.kind = task` 的 `id` 就是挂起调用记录与 task 工具使用的 `task_id`。

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

每次路由（drive 入口、主循环每一轮、每个检查点）按“输入源声明顺序、源内 `index`”依次处理未消费的记录，不按 `at_ms` / `created_at_ms` 排序。模板只改变正文，不改变这里的任何一行。

| 记录 / 情况 | 机械处理 | 对 LLM / Turn 的影响 |
|---|---|---|
| 被拒绝的记录；finished 之后的非 `decide` | 消费，`input_rejected` | 无 |
| `key` 在 `recent_keys` 中，或同一轮已出现过 | 静默消费 | 无（不会第二次进入上下文） |
| `control` | 由驱动者按投递顺序应用并提交 | 不是 Agent 输入 |
| `msg` | 留在队列，作为受控输入的候选 | 开启新 Turn 或并入未关闭的 Turn |
| `event`，匹配 active 订阅 | 记入 `inputs[src].accepted`，留在队列作候选 | 同 msg；挂起工具期间暂存 |
| `event`，匹配 semi 订阅 | 合并进 `state.pending_events`，消费 | 不触发推理；在下一次受控输入之前渲染为半订阅快照 |
| `event`，`source = task:<id>` 且有挂起调用在等该 task | 消费，`event_dropped{reason: pending_call}` | 只触发一次 resolver 查询；结果以 ToolResult 回填原 run / Turn |
| `event`，没有匹配的有效订阅 | 消费，`event_dropped{reason: unsubscribed}` | 无；timer、系统事件没有兜底例外 |

- 订阅匹配：给出 `subscription_id` 时必须存在且来源相符；未给出时取第一个来源相符的显式或隐式订阅。每个 Session 都有一个隐式 semi 订阅：用户时区（`_user_timezone`，`source = system:user_timezone`）。
- subscribe / unsubscribe 与事件按投递 index 生效：`event A → unsubscribe → event B` 中 A 按当时有效的订阅处理，unsubscribe 清理该订阅尚未注入的状态，B 丢弃。已被 active 订阅接受的事件不因之后的 unsubscribe 改判。
- 挂起工具（PendingTool）或子 context 进行期间，msg / Input event 留在队列，不能填补缺失的 ToolResult；control 继续处理。
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
- “已接收保存”不等于“已注入上下文”：保存与消费同一次提交，receipt 只记录实际注入的版本。
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
   │ 落盘      1–2 条 user AiMessage 与 receipt 写进同一份快照
   ▼
AiMessage{role: user}              恢复、接手、压缩都从快照与 worklog 读取，不回到总线重新渲染
```

不变量：结构只编码一次（渲染之前 MsgObject 是消息结构的唯一载体；总线与 state 中不出现 `AiMessage`）；只渲染一次；文本自足（每个附件都以 ObjId 或可读路径出现在文本里，去掉媒体块仍可定位）；正文不能伪造结构（文本与属性值在渲染时转义，`from` / `key` / `mentioned` 等只来自结构字段）。

### 6.1 三类受控输入与消费策略

| 入口 / 模板 | 触发 | 边界 |
|---|---|---|
| `on_init` | Session 首次启动 | 不要求有外部输入；只在 bootstrap 使用 |
| `on_input` | 选中的 msg / Input event | control、Observe 事件、队列通知本身不构成此入口 |
| `on_context_switch` | 交接：切换 behavior、进入子 context、经交接批次返回的子结果 | 延续当前 Turn；工具触发的子调用返回仍是 ToolResult |

一次装配只选一个入口（`on_init` → `on_context_switch` → `on_input`），允许消费的外部输入并入该批。`input.mode`：`batch`（默认，批次预算内取多条）或 `single`（总共取一条），未选的留在队列。模板与策略取自接收批次的 context（`prompt.*` / `input.*`，behavior 条目按字段覆盖 Session 的）。模板报错、或已有选中的输入却渲染出空文本：报错并保留现场，不消费、不确认。

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
| `reply_to` / `edit_of` / `redacts` / `thread` | `thread.reply_to`、`relates_to` | 值为目标消息的 ObjId |
| `format` | `content.format` | 仅 `text/*` 且非 `text/plain` 时 |
| `title` | `content.title` | 非空时 |
| `time` | `created_at_ms`，缺省取信封 `at_ms` | UTC RFC 3339，`Z` 后缀，秒精度 |
| 正文 | `content.content`（trim） | 转义 `& < >`；非空时独占一行。正文只是 `[attachment]` 等占位词且有附件时置空 |
| `<attachment index media name mime obj_id path role/>` | `content.refs` 的 `DataObj` | 每个一行，按数组顺序编号；`media` / `mime` 由 label、uri_hint、消息 format 推断；`path` 仅在宿主已有可读映射时输出；`role` 仅非 `input` 时 |
| `<event key subscription source event seq data_ref terminal>` | AgentEvent | `source` 为 `kind:id`；正文是转义后的 `summary`；可选属性缺省不输出 |

属性值转义 `& < > "`。只有一条纯文本消息时也使用 `<msg>`。`machine` 与 `ServiceDid` 引用不在内建渲染中出现。

内建模板：`<session_input hook="…" time="<批次时间>">` 元素，依次包含交接状态（`<context_switch to>`、`<process_result>`、`<sub_task>`）、bootstrap 的 `<task>` / `<scope>`、`input.text`、`<perceptions>`、`<hints>`、活动 session 列表、`<runtime>`。半订阅快照的内建形式：`<semi_subscription_snapshot>` 内每个状态版本一个 `<event>` 元素（终结版本在前）。

协议时间与内建渲染一律 UTC。用户时区与 Session 绑定（`session.timezone`，IANA 名），其初始值与后续变化经隐式 semi 订阅进入半订阅快照，不取 Runner 所在机器的时区，变化本身不唤醒推理。

### 6.3 自定义模板

模板引擎是 `llm_context::prompt_engine`（upon，`__EXEC__` 关闭）。只含块标签（`{% for %}`、`{% if %}`、`{% endfor %}` …）的行不产生输出行；输出去掉首尾空白。可用变量：`input.{hook, time, messages[], events[], items[], text, count}`（视图字段见 `runner/input_view.rs`；条件用布尔字段 `msg.is_group`、`msg.mentions.me`、`ev.terminal`、`item.is_msg`）、`session.*`、`runtime.*`、`handover`、各内建块（`task_text`、`handover_text`、`perceptions_text`、`hints_text`、`active_sessions_text`、`runtime_text`、`builtin`）。

命名格式 `{{ value | render_format: "名称" }}`（未知格式或对象形状不符是模板错误；声明支持空值的格式对空值输出空串）：

| 格式 | 输入 | 结果 |
|---|---|---|
| `input.xml` | `input` | `<inputs>` 块，与 `input.text` 逐字节一致 |
| `message.xml` / `event.xml` | 一条消息 / 事件视图 | 内建元素，与 `input.text` 中的同一条一致 |
| `message.markdown` | 消息视图 | `## Request from …` 标题、引用正文、附件清单 |
| `event.summary_text` | 事件视图 | 单行：来源、事件名、终态标记、摘要（≤ 300 字符） |
| `attachments.xml` / `attachments.list` | `msg.attachments` | 每个附件一行 XML / 一行文本清单（有路径用路径，否则 ObjId） |
| `todo.summary_xml` | 宿主装配的 `{id, status, summary \| title}` | `<current_todo …>`；空值输出空串 |

底层通用 filter（llm_context 注册，system 模板同样可用）：`xml`、`attr`、`json`、`truncate: n`、`oneline`、`default: v`、`join: sep`、`quote`、`time: fmt`（UTC）。XML 格式自己转义，模板直接插入；直接拼装 XML 的模板自行用 `xml` / `attr`。渲染后 Session 逐个检查附件：文本中既没有其 ObjId 也没有可读路径时记 warning（显示名不算定位信息）。

### 6.4 媒体块（`input.media`）

与模板无关，由接收批次的 context 的 `input.media` 机械决定：`reference`（默认）只有文本；`inline` 在文本块之后按消息顺序、附件顺序为 `image` / `document` 附件追加 `Image{source: named_object}` / `Document{source: named_object, title}` 块，每批最多 8 个，其余只保留文本引用。两种取值下文本块相同。worklog 的 `user_message` 只记文本，新 run 重建的历史因此只有文本。

`inline` 的请求被 provider 拒绝（永久或未知的 provider 错误：模型不接受图片 / 文档，或对象不可读）时，Session 机械降级一次：去掉该 run 中 user 消息的媒体块，在其文本末尾追加一行说明，发布改写后的快照并重试；快照的 `host.libopendan.media_degraded = true` 记录它已发生，同一个 run 不再降级。文本自足保证附件仍可按 ObjId / 路径读取。

## 7. 输入 receipt 与提交

每个批次由有序的 1–2 条 user 消息组成：有待注入的半订阅状态时先是快照消息，再是受控输入消息，一次注入，中间不推理、不发布快照。receipt 与消息写在**同一份**快照的 `state.host.libopendan.input_receipts[]`：

```jsonc
{ "run_id": "…", "input_seq": 2, "turn": 5, "opens_turn": true,
  "hook": "on_input",                                   // on_init | on_input | on_context_switch
  "inputs": [ { "src": "q", "index": 121, "key": "cymsg:…", "kind": "msg" } ],
  "events": [ { "subscription_id": "watch-obj", "source": { "kind": "object", "id": "doc-1" }, "seq": 8, "key": "obj:doc-1:8" } ],
  "reply":  { "route": "message", "to": "did:bns:dev-team", "to_session": null, "kind": "group_msg", "reply_to": "cymsg:…", "tunnel": null },
  "parts": [ { "part": "semi_subscription_snapshot", "pos": { "kind": "accumulated", "index": 6 }, "text": "<semi_subscription_snapshot>…" },
             { "part": "input",                      "pos": { "kind": "accumulated", "index": 7 }, "text": "<session_input hook=\"on_input\" …" } ],
  "bootstrap": false, "after_step": 7, "extra": { "continuation": true }, "at_ms": 0 }
```

- 批次 ID = `(run_id, input_seq)`，每个 run 从 1 开始连续。`opens_turn` 只看提交时有没有打开的 Turn；半订阅快照不独立计 Turn。
- `inputs`：本批消费的 msg 与 Input event。Runner 自己合成的输入（被跟踪的后台 task 完成）用 `src = "_task"`、`key = task:<task_id>:terminal`，没有消费位置。
- `events`：快照消息实际注入的状态版本；应用时按 `(subscription_id, source, key)`（有 `seq` 时一并核对）从 `pending_events` 精确清除。处理 v7 期间收到 v8，提交 v7 不删除 v8。
- `reply`：本批提交后的默认回复路径（`state.reply`）：本批按消费顺序最后一条 msg 的来路（一对一 → 原发送者；群消息 → 原群；带 `to_session`、`tunnel`、`reply_to`）；本批没有 msg 时沿用原值；从未有过 msg 而有父 Session 时为 `{"route": "parent_session", "session_id": …}`。event、control、快照、交接本身不改变它。
- `parts[].pos`：function call → `accumulated`（两条相邻）；behavior 首个 Step 之前 → `request_input`；已有 Step → 两个 part 都是 `step{index}`（并入该 Step 的 `next_user_message`）。`text` 是该消息的文本块；媒体块按 `pos` 在同一份快照中读取。
- 应用 receipt（幂等：`input_seq ≤ live_run.applied_input_seq` 忽略；必须恰好是下一批；`opens_turn = false` 时 `turn` 必须等于 `open_turn.index`）：更新 `live_run.turns` 与 `open_turn`，标记 `inputs` 已消费并记入 `recent_keys`，清理 `events`，还原 `reply`，置 `bootstrap_done`，`extra.continuation` 清除 `internal_continuation` / `process_result`，`run_state = running`。
- **提交顺序**：① 快照 fsync ② run.json 发布快照指针与 `host_commit_pending = input_seq` ③ state.json 应用 receipt ④ 清门槛 ⑤ 确认输入源。④ 之前不得推理或调用工具；可恢复提交之前不出队。
- **恢复**：先按快照中的 receipt 补齐 state（不重新渲染、不重新注入、不读总线），再取新输入；state 已应用而快照缺失的批次 → RecoveryBlocked。
- 检查点（function call：每次推理前；behavior：每个 Step 的 do-action 之后）只发布工具结果、做一次路由并刷新心跳：到达检查点、工具完成或收到 Observe 事件都不独立注入消息。
- 推理 / 装配预算不足按 AICC 服务不可用的失败与重试路径处理：未提交的批次不消费，已提交的批次保留 receipt，恢复时不重复注入。

## 8. bridge

bridge 只转换来源并可靠投递（先 `post`，成功后再确认上游；重投由 `key` 去重）；不渲染、不决定投递策略、不写 Session state。

**msg bridge**（`bridge/msg.rs::route_msg_record`，纯函数）：MsgObject 原样上总线，只做过滤、分流和补 `delivery`。按顺序：记录发送者与 `msg.from` 不一致 → 丢弃；群邀请通知、Agent 自己的群消息回显 → 丢弃；正文 / refs / machine 全空 → 丢弃；`relates_to.rel = reaction` → 丢弃；纯文本且匹配已登记斜杠命令、发送者是 session 驱动者或 Agent owner → 映射到 Session 控制的命令投递 `control`（`/stop` → `stop`，key `ctl:<ObjId>`），其它已登记命令由应用自行处理；其余（含 `edit` / `redact`）原样投递。Session 不解析正文：未被识别的斜杠文本就是普通消息。

**task bridge**（`bridge/task.rs::task_event`）：把 task 状态映射成 `AgentEvent{source: task:<task_id>}`（`updated` / `finished` / `unknown`，终态 `terminal = true`）。通知只加速检查，task 状态 API 才是权威来源。

**反向链路**：assistant 消息按 `state.reply` 转成 MsgObject（`bridge::outbound_base` 给出信封，`llm_context::ai_message_to_msg_object_with_base_validated_async` 填内容），出站记录 `{key: "<sid>:<turn>:<run_id>:<n>", msg}` 幂等。记录在 Turn 关闭的提交里写入 `state.outbox`，提交后交给驱动者的出站 sink（OpenDAN：msg-center `post_send(msg, idempotency_key = key)`），重启后原样重发；`channels.outbound` 是创建时固定的回复坐标，出站时核对（[Session Directory Protocol](<Session Directory Protocol.md>) §4 `outbox`）。耗时较长的 Turn 先发占位、结束时以一条 edit 替换，并在锚点消息上携带 `agent_task`（同节 `outbox[].purpose` / `turn_tasks`）。

## 9. 长任务：挂起调用与后台 task

- **串行等待**：工具返回 `Pending{task_id, until_ms}` 时 run 挂起（PendingTool），Session 在上下文之外等待：`waiting_for = {kind: tool, refs: [task_id…], deadline_ms}`（由快照的挂起记录生成，不另存协议）。每次路由后用宿主的 `RunningTaskResolver` 查询；每个挂起调用的 task 结束、状态未知或到达 `until_ms` 后，用当时的状态以 `ResumeFill::ToolResults` 回填，恢复同一个 run 与 Turn。轮询不调用 LLM，等待与 inbox 是否为空无关。
- 接手：`can_resolve(task_id) = false` → RecoveryBlocked，保留现场；能解析而查不到（进程内 task 换了进程）→ 按 `Unknown` 回填，由 LLM 判断，不重建、不重放任务。
- stop：可取消的 task 被取消，每个挂起调用按当时状态回填，run 以 Stopped 结束。工具执行期间由监视任务查看队列中的 stop 并打断运行（只查看，不消费、不写 state）。
- **并行等待**：调用已返回、task 继续运行。run 结束而 Session 未 finished 时，仍在运行的 task 记入 `state.watched_tasks`；之后每轮用保存的 task id 查询，结束时合成 `AgentEvent{source: task:<id>, terminal: true}`：有显式订阅按其 active / semi 处理，否则作为 Input 事件。不依赖输入队列或通知；finished 的 Session 不因 task 完成而重开。
- 经外部服务创建任务的工具用在途记录里的 `idempotency_key`（由 run 与 call 的稳定身份导出）作为幂等键。
- 崩溃恢复：run 的终态已落盘而 state 尚未提交时，重做的结束从 run.json 的 `host.extra.tasks` 取回仍需跟踪的 task（[Session Directory Protocol](<Session Directory Protocol.md>) §7），其完成只交付一次。

## 10. Sub Session：创建、汇报与等待

Sub Session 是 `origin.parent_session` 指向父的普通 Session：自己的目录、state、worklog、Turn 与 lease，驱动者默认是父的驱动者。父子都只写自己的 state.json，彼此只经登记表、输入通道与控制协议沟通；登记表里的状态是真相，下面的事件只是加速。

- **创建**（参考实现 `api::create_sub_session`，CLI `agent-session create-worksession`）：`origin = {parent_session, report, created_by_call}`。幂等键由 `(父 session_id, run_id, call_id)` 导出：同一次调用重放得到同一个 Session。超过父的 `policy.max_sub_sessions`（同时未结束）或 `max_session_depth` 时拒绝。继承父的 `prompt.llm_context`（含 runtime 配置；绑定身份是子自己的）、默认共用父的工作目录。首批输入是所属 Agent 自己发出的 MsgObject：子没有队列时进 `prompt.initial_inputs`，有队列（interactive）时投递到队列。创建不推进子 Session。
- **隐式关注**：父的 Runner 按 `origin.parent_session = 自己` 查登记表（每次选批之前、以及等待期间的每次轮询），不需要订阅，父也不需要输入队列。`origin.report` 为 `none` 或缺省的子不产生任何事件。
  - 进度（仅 `report = progress`，子未结束）：`status.rev` 前进且 `{run_state, one_line_status, activity.summary}` 变化时，合成 `AgentEvent{subscription_id: "_child:<sid>", source: session:<sid>, event: "progress", seq: rev}` 并入 `pending_events`（Observe），游标存 `state.subscription_cursors["_child:<sid>"] = {rev, view}`。
  - 需要关注与结束：子 `finished` → `event = finished`（`terminal = true`，summary 含 `report_brief`）；有 `pending_decision` → `needs_decision`；`run_state = waiting` 且 `status.waiting_for = input` → `needs_input`。它们是 **Input**：作为内部输入源 `_child` 的候选（`key = child:<sid>:<event>@<rev>`，index 0）进入受控输入批次，可开启或并入 Turn。候选在每次选批时重新合成，直到消费它的批次的 receipt 提交：应用 receipt 时把 `subscription_cursors["_child:<sid>"].attention` 置为该事件名，同一状态不再交付；子离开该状态后清空，下次进入是新的事件。`_child` 与 `_task` 一样没有总线消费位置。
- **同步等待**：工具返回 `Pending{task_id: "session:<sid>"}`（`create-worksession --wait`、`wait <sid>`）时父 run 以 PendingTool 挂起，按 §9 的串行等待处理；该 task id 由宿主 Session 的 resolver 只读登记表解析：结束 → 结果 `{session_id, status: finished, outcome, acceptance, one_line_status, report_brief, answer_ref, artifact_id}`；等输入 / 等决定 → `{status: needs_input | needs_decision, question}`；其它继续等。被挂起调用等待的子不产生 `_child` 候选；回填时把当时的状态记为已交付，结束不再作为事件重复注入。等待不拥有该 Session：`cancel` 不支持，stop 是显式的控制命令。不具备这个 resolver 的执行方（独立 xllm）`can_resolve = false`，拒绝接手。
- **父结束规则**：见 [Session Directory Protocol](<Session Directory Protocol.md>) §4 的 `waiting_for = children`。`run`、`serve` 与等待循环在 `waiting_for.kind = children` 或 `watched_tasks` 非空时轮询登记表 / resolver，不以“没有输入队列”为返回依据。
- **stop 级联**：父被 stop 时，对未结束且有输入队列的子投递 `stop`；没有队列的子由驱动它的宿主以驱动者自己的停止请求结束（见 [Session Control Protocol](<Session Control Protocol.md>)）。子被 stop 只在父那里表现为一次 `finished`（outcome stopped）。
- **推进**：子由其驱动者身份的托管进程推进（参考实现 `host::ChildDriver`：轮询登记表，为每个未结束、未被驱动的子按其自己的配置装配 runtime 后 `drive(Idle)`，各持自己的 lease）。宿主进程退出只停止这些任务，不改变已提交状态；重启后从登记表重新接管。
