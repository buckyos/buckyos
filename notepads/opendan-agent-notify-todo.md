# OpenDAN Agent 通知人 TODO（System Notify / ask-user 兜底 / 通知的观察与代理）

日期：2026-10-11

状态：v0.2。用户已答复四项（§8.1）；其余待定项按推荐写（§8.2），实施前仍可调整。第一段（§1—§7）可以开始实施；第二段（§9）只记方向，第一段验收后再细化。

修订记录：

- 2026-10-11 占位。
- 2026-10-11 v0.1：按讨论纪要与代码现状写成实施方案。
- 2026-10-11 v0.2：按用户答复修改——System Notify 放 msg-center；通知中心与处理入口改为 MessageHub（每个来源一个“通知”会话），TaskCenter 只做兼容；Work 等待缺省 3 小时；workflow `remind` 暂不处理。

依据：

- 讨论纪要 [Agent 主动给用户发"消息"的方法总结](<../doc/opendan/Agent 主动给用户发_消息_的方法总结.md>)（下称“纪要”），全部章节；§13.2 的待定项在本文 §8 给出取值。
- [TaskMgr 2.0 设计](<../doc/task_mgr/task-mgr 2.0.md>) §3（URL）、§5.2（WaitReason）、§10.2（HumanSet）、§11.7（执行期人工任务）、§14；[task data schema](<../doc/task_mgr/task data schema.md>)（`HumanTaskPrompt`）。
- [Message Center](<../doc/message_hub/Message Center.md>)、[MessageHub UI 数据模型](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。
- [TaskCenter 需求](<../product/task_center/TaskCenter_mgr.md>) §4.2、§13：本文不改它的定位，只做 §3.4 末条的兼容。
- [OpenDAN Agent Session 架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>) §5.6、§8.1、§12.7、§12.11、§12.12.2、§17.2。
- [意图分析 TODO](opendan-analyze-user-intent-todo.md) §3.7—§3.9（UI Session 内的 ask-user，已实施）；[Goal Session TODO](opendan-goal-session-todo.md) §3.2（questions）、§8.3、§9.1、§9.3。

依赖本文的地方：Goal Session TODO（questions 的投递、§9.3 self_check 退役）；意图分析 TODO §1.2 列为不做的“Agent 主动通知人”。

读者：实施本文的 code agent。路径相对 `buckyos/`。行号是 2026-10-11 的源码，实施前先核对。

---

## 0. 结论先行

| 事项 | 决定 | 依据 / 理由 |
|---|---|---|
| 机制分离 | 会话消息（Session Inbox / Outbox）、System Notify、`ask-user`、`post-message`、Input Task / Final Report 语义各自独立。第一段做 System Notify 和建立在它上面的三件事：Work 的 ask-user 兜底、Goal 的提问投递、UI Session 对通知的观察与代理。`post-message` 与机械审批放第二段 | 纪要 §一、§13.1 |
| Notify 的真相 | 一个 Notify 就是一个 TaskMgr 2.0 Task：`executor = HumanSet{[目标用户]}`，schema 为 `human.notice/v1`（新增，无需回复）、`human.input/v1`（需要回答）或 `human.approval/v1`（审批，第二段才有生产者）。`notify_id = task_id`；状态、结果、“谁先提交谁生效”都沿用 TaskMgr 现有语义 | 纪要 §5.4、§6.1“唯一真相源”；HumanSet 的 first-CAS-wins 已实现（`kernel/task_manager/src/server.rs:1778-1792`） |
| Notify 的内容 | task input = `{notify: 来源 / 等级 / 过期 / 会话, prompt: HumanTaskPrompt}`；result = `TaskDataResult::Human{action, values, submitted_by, submitted_at}` | 类型已在 `buckyos-api/src/taskdata.rs:20-200`，至今无人使用 |
| 宿主 | **msg-center**（用户决定）新增 kRPC `notify.*`：建 Task、写通知会话、按等级另投、写观察记录、过期清扫、清单查询。msg-center 不保存状态真相，只保存投递记录 | 渠道（MessageHub 原生、Telegram）、联系人、按用户隔离和可靠投递都已在 msg-center |
| 等待 | 等待者直接读 TaskMgr（`get_task`；kevent `/task_mgr/<id>` 只用来加速）。是逻辑等待，不占进程 | 纪要 §5.3；TaskMgr 没有 long-poll |
| 通知中心与处理入口 | **MessageHub**（用户决定）。每个来源一个“通知”会话（第一段的来源只有 Agent：Agent 实体下的会话 `notify:<agent_did>`），每个 Notify 在其中是一张卡片，按 prompt 原地处理，状态实时。另有跨来源的“待处理通知”入口。统一 URL `/messagehub?notify=<id>` 打开它的卡片。TaskCenter 不当通知中心 | 纪要 §6.1、§6.6；MessageHub 本来就擅长按来源展示会话 |
| 触达 | 通知会话里总有一条主记录；再按等级另投到“用户与该 Agent 已有的会话”（原生 / Telegram）：`quiet` 不另投，`normal` 投来源会话，`timely` 再加最近活跃的会话，`urgent` 投全部。未读只落在用户最可能看的那一处。非 Agent 来源第一段不支持 | 纪要 §6.3—§6.4；现在只有这两种渠道 |
| Work 的 ask-user | LLM 仍调用同一个 `agent-session ask-user`。在 Work 中由 runner 改走 Notify（`human.input`），run 照旧挂在 `userinput:<call_id>` 上。等待**有界**：缺省 **3 小时**（用户决定），可由 `[session.work] ask_user_timeout_secs` 配置。超时、被忽略、被撤销都作为工具结果返回。提示词仍倾向快速失败 | 纪要 §7.3—§7.5、§10.1；走哪条路由 Session 决定，不让 LLM 选 |
| Goal 的提问 | Activation 结果中的 `questions` 由驱动者转成 `human.input` Notify，不阻塞。回答作为下一次 Activation 的原因 `answer` 投回。Goal 内调用 `ask-user` 仍然拒绝，只是提示改为“写进 questions” | 纪要 §10.2；补上 Goal TODO §8.3 中“questions 只能经 kRPC 看到”的缺口 |
| 主动告知 | `agent-session notify send`（`human.notice`），UI / Work / Goal Session 都可以用。智能提醒的最后一步就是它 | 纪要 §十一 |
| UI 观察 | msg-center 把 Agent 来源的 Notify 写进某个会话（包括通知会话）时，在该 Agent 对应会话的 INBOX 写一条观察记录（`kind = event`，`machine.intent = buckyos.notify`）。Loader 把它转成 `_notify` 半订阅事件，随下一批输入出现在 `<semi_subscription_snapshot>` 中。它既不是 user message，也不是 Assistant Message | 纪要 §12.1—§12.2；半订阅机制已有（`lib_opendan/src/runner/inputs.rs:330-365`），只是没有生产者 |
| UI 代理 | `agent-session notify answer / ack`：用户明确回应某条未结束的 Notify 时，UI Session 代为提交到原 Task。`human.approval` 一律拒绝；不能处理本 Session 自己发起的 Notify；每次代答都写 task note 留痕 | 纪要 §12.4—§12.5 |
| workflow `remind` | 暂不处理（用户决定：workflow 还没正式用起来） | — |
| llm_context / agent_tool | 第一段不改。第二段的机械审批需要改 `PolicyEngine`（`llm_context/src/deps.rs:237-255`），按约定先单列 TODO | [`feedback-llm-context-first`] |
| 协议版本 | 只增加字段；结构有变化时重新生成 fixtures | `doc/opendan/protocol/README.md` |

---

## 1. 第一段范围

### 1.1 做

- **buckyos-api / TaskMgr**：`notify.*` 的请求与视图类型和客户端；内建 schema `human.notice/v1`；约定 `human.input/v1` 的 notify input / result 结构。
- **msg-center**：
  - `notify.send / get / list / cancel`；
  - `notifies` 表（索引与投递记录）；
  - 通知会话的登记与主记录；按等级另投；消息组成（正文 + 链接 + `system_notify` meta）；
  - 观察记录；过期清扫；
  - `kind = notify` 计入会话活跃。
- **libopendan**：
  - `NotifyHost` 宿主接口，含本地文件实现；`AgentStateClient::notifies()` 与 kRPC 透传；
  - 来源计算；Work 的 ask-user 改走 Notify；Goal questions 与 Notify 互转；
  - ui 模板的 `_notify` 隐式半订阅；按截止时间与 Notify 变化唤醒 Session；Turn task 的等待原因；
  - `agent-session notify …` 与 `xagent notify …`。
- **OpenDAN Loader**：
  - `NotifyHost` 的 zone 实现（msg-center + TaskMgr）；
  - 把 INBOX 观察记录转成 `_notify` 事件；kevent 加速；
  - Jarvis 提示词。
- **Desktop**：
  - MessageHub：通知卡片、通知会话、待处理通知入口、`?notify=` 深链；
  - TaskCenter：只做兼容（§3.4 末条）。
- **文档回写**（N10）。

### 1.2 第一段不做

| 事项 | 去向 |
|---|---|
| `post-message`、收据、回复关联、共享邮箱的回复路由 | §9.1 |
| 机械审批：执行层授权、审批 Notify 的生产者、当前 UI 被阻塞时的模态遮罩、审批只能由用户本人提交的硬约束 | §9.2（先单列 llm_context TODO） |
| 用户的通知设置、等级的回退与升级、短信 / 邮件 / App 推送渠道、桌面状态栏托盘、非 Agent 来源（系统发送者与它的通知会话） | §9.3 |
| 把 Notify 状态变化（已回答 / 已过期）推给 UI Session 的观察事件 | §9.4 |
| 一次性定时（智能提醒的“提前唤醒”） | Goal TODO §9.1、§9.5 follow_up |
| 纪要 §三：处理中补充输入的归属与占位排序 | §9.5，另立 TODO |
| workflow `remind` 改走 Notify、`dcrontab` 的 PATH 与 owner 问题 | 暂不处理（用户决定，workflow 尚未正式使用） |
| TaskCenter 作为通知中心、详情页按 prompt 交互 | 不做（用户决定改在 MessageHub）；TaskCenter 自己的 P1 不在本文 |
| 会签、多级审批 | 不做（TaskMgr §10.2 要求由 Workflow 聚合） |

---

## 2. 当前基础与缺口

| 入口 | 已有基础 | 缺口 |
|---|---|---|
| `kernel/buckyos-api/src/task_mgr.rs:552-570`（`TaskExecutor::HumanSet`）；`kernel/task_manager/src/server.rs:1018-1049, 1778-1792` | <ul><li>HumanSet 任务创建后即为 `Waiting(HumanInput)`。</li><li>assignee 用 `commit_result` + `expected_revision` 提交，先到先得；输家得到 `task_already_completed`。</li><li>每次变更发布 kevent `/task_mgr/<id>`，`list_task_events` 可补漏。</li><li>`human.approval/v1`（:992, 1202-1228，严格 schema）；`human.input/v1`（:1005，宽松 schema）。</li></ul> | <ul><li>生产代码中没有 HumanSet 的创建者。</li><li>不支持过期：表、结构、请求里都没有 deadline。</li><li>没有 `human.notice`。</li><li>assignee 没有 ReadResult（`task_manager/src/acl.rs:253-308`）。</li><li>commit 不检查 App。</li></ul> |
| `kernel/buckyos-api/src/taskdata.rs:20-200` | `TaskDataRequest::Human{prompt}`、`HumanTaskPrompt{title, message, controls: Choice / TextInput}`、`TaskDataResult::Human{action, values, submitted_by, submitted_at}` | 无人使用 |
| `frame/msg_center/src/msg_center.rs:1601-1830`（`post_send`）、`:1028-1118`（路由）；`main.rs:293-458`（投递泵）；`owner_session.rs:533-582`（`msg.create_session`） | <ul><li>可靠投递，支持幂等键。</li><li>路由只看目标 DID 的形式：`did:msgtunnel:*` 走 tunnel，其余走原生。</li><li>Telegram 发送要求 `from` 有 bot 绑定，实际上只有 Agent 能发。</li><li>可以登记带 peer、标题的会话。</li></ul> | <ul><li>没有 notify RPC、通知会话、系统发送者、用户渠道偏好。</li><li>没有邮件、短信 tunnel。</li></ul> |
| `frame/msg_center/src/owner_session.rs:165-170`；desktop `app/messagehub/sessionModel.ts:17-20`、`protocol/msgobj.ts:421-426` | `MsgObjKind::Notify` 只映射成字符串；不计入会话活跃；MessageHub 渲染成状态胶囊 | 缺通知卡片投影（`UI_DATAMODEL.md:663, 700-701` 已记录这个缺口） |
| desktop `app/messagehub/MessageHubRoute.tsx:8-32`、`launch.ts:4-37`（`messageHubMessagePath`）；`EntityList.tsx`、`SessionSidebar.tsx` | 深链 `entityId / sessionId / messageId`；按实体 → 会话展示；未读徽标来自 `SessionSummary.unread_count` | 没有 `notify` 深链参数，也没有跨会话的待处理入口 |
| `frame/opendan/src/ui.rs:430-512, 594-715`；`lib_opendan/src/bridge/msg.rs:85-195` | Agent INBOX 的每个会话对应一个 UI Session（route_key = 邮箱地址）；只放行主人发来的消息；用 kevent 唤醒 | 没有把“系统刚给用户看了什么”交给 UI Session 的路径 |
| `lib_opendan/src/runner/inputs.rs:330-365`；`protocol/config.rs:727-800`；`runner/drive.rs:1890-1903`；`runner/assembler.rs:166-185` | 半订阅事件进入 `pending_events`，随下一批输入渲染为 `<semi_subscription_snapshot>`；已有隐式订阅样板 `_user_timezone`、`_goal_activation` | 没有 `system:notify` 来源和事件生产者 |
| `lib_opendan/src/runner/ask.rs:27-35, 83-114`；`template.rs:97-104, 147-165` | UI Session 的 ask-user 已实施：`userinput:` PendingTool、问题出站、下一条消息回填 | Work（无队列）和 Goal 直接报错 “this session cannot ask the user …” |
| `runner/live.rs:638-691, 852-994`；`runner/drive.rs:1062-1133`；`host/mod.rs`（Supervisor） | `userinput:` 不经过 resolver；挂起状态可跨重启；有 `watched_tasks` 轮询样板；有队列的 Session 卸载后每 30 s recheck | 无队列的 Work 卸载后不会再自动装载；也没有按截止时间唤醒 |
| `runner/turn_task.rs:167-196`；`opendan/src/tasks.rs:288-301` | Turn 的等待会映射为 TaskMgr wait reason；Input 映射为 `Other("input")` | 经 Notify 等待时应改为 `HumanInput` + `related_task_id` |
| `lib_opendan/src/protocol/goal.rs:530-535, 595-597`；`runner/goal.rs:867-893`；Jarvis `behaviors/goal.toml:20` | Goal `questions[]` 只记录，最多保留 16 条；behavior 提示写着 “nobody answers in this session” | 没有投递，回答也不能回流 |
| `lib_opendan/src/bin/xagent.rs:35-93`；`llm_context/src/deps.rs:329-336` | 没有 notify / send-msg 子命令；`<sendmsg>` 动作是空实现 | 缺 `agent-session notify …` |
| `llm_context/src/deps.rs:237-255`、`request.rs:217-222` | `PolicyEngine` 只能整批拒绝工具调用；`HumanPolicy.approval_required` 从未被读取 | 机械审批（第二段） |
| desktop `api/task_mgr.ts:551-622, 954-980`；`app/task-center/pages/HomePage.tsx` | TaskCenter 首页把每个未结束的 HumanSet 任务派生成一张 approve / reject 卡片 | Notify 任务也会被派生成这种卡片，而 approve / reject 的提交结构不符合 `human.notice` 的 schema（§3.4 末条） |
| desktop `desktop/StatusBar.tsx`、`DesktopRoute.tsx:750-839` | 托盘计数是写死的 mock | 第二段（§9.3） |
| workflow `send_message_executor.rs`；`agent_tool/src/dcrontab_tool.rs` | `remind` → `workflow.send_message` → `post_send` 一条 `kind=chat` 的消息 | 本文暂不处理 |
| `lib_opendan/src/bin/xagent.rs:433-437, 723-735` | `agent-session post <sid> --msg [--from <DID>]` 可以向任意有队列的 Session 投消息，`--from` 可以随便填 | 与纪要 §8.2 冲突（见 §8.4） |

---

## 3. 模型与契约

### 3.1 Notify 对象（TaskMgr Task）

msg-center 创建（字段名是约定，可调；语义不变）：

```text
create_task {
  name: "notify: <title 截断>",
  schema_id: "human.notice/v1" | "human.input/v1",        // human.approval/v1 第二段
  executor: HumanSet { assignees: [<目标 user_id>] },
  storage_domain: User,
  idempotency_key: "notify:<请求方 principal>:<请求方给的 key>",
  input: { "notify": {…}, "prompt": HumanTaskPrompt }
}
→ grant_task_access:
    User{目标用户}             → ReadResult（assignee 默认没有）
    Principal{请求方 user, app} → ReadMeta / ReadInput / ReadResult / Control
```

```json
{
  "notify": {
    "version": 1,
    "kind": "notice | input",
    "level": "quiet | normal | timely | urgent",
    "source": {
      "app_id": "…", "agent_did": "… | null",
      "session_id": "… | null", "turn": 3, "call_id": "… | null",
      "goal_id": "… | null",
      "operation": "一句话：这是为了什么（可空）",
      "conversation": { "to": "<用户端点 DID>", "to_session": "…" }
    },
    "expires_at_ms": null
  },
  "prompt": { "title": "…", "message": "…", "controls": [] }
}
```

- `notice`：`controls` 为空，用户的动作是 `ack`（知道了）或 `dismiss`。
- `input`：
  - 问题文本放进 `prompt.message`；
  - 有选项时加 `Choice{id: "option", options, required: true}`；
  - 没有选项或 `allow_text` 时加 `TextInput{id: "text"}`。
- result：`{"kind": "human", "action": "answer | ack | dismiss", "values": {"option": "…", "text": "…"}, "submitted_by": "<user>", "submitted_at": 0}`。实际提交的 App 由 TaskMgr 写入 `completed_by`，代答所在的 Session 与触发消息记入 task note（`add_task_note`）。

**状态推导**（`notify.get` / `notify.list` 给出；真相在 TaskMgr）：

| TaskMgr | Notify 状态 |
|---|---|
| `Waiting` | `open` |
| `Terminal / Succeeded`，`action = answer` | `answered` |
| `Terminal / Succeeded`，`action = ack` | `acknowledged` |
| `Terminal / Succeeded`，`action = dismiss` | `dismissed` |
| `Terminal / Canceled`，且 msg-center 记录了 `closed_reason = expired` | `expired` |
| 其它 `Terminal / Canceled` | `canceled`（附 `closed_reason`，如 `ask_timeout`、`turn_stopped`、`goal_dropped`） |

### 3.2 msg-center `notify.*`

**`notify.send`**

```json
{ "to_user": "<user_id>", "kind": "notice | input", "level": "normal",
  "prompt": { … }, "source": { … }, "expires_at_ms": null, "idempotency_key": "…" }
→ { "notify_id": "…", "url": "https://<zone host>/messagehub?notify=<id>", "status": "open",
    "home": { "session_id": "notify:<agent_did>", "msg_id": "…" },
    "deliveries": [ { "session_id": "…", "platform": "native | telegram", "msg_id": "…", "state": "sent | pending | failed" } ] }
```

1. **权限**：
   - 调用方是 App 主体 `app:<app>@<user>` 时，只能通知 `<user>`，且 `source.app_id` 必须等于调用方的 App。`source.agent_did` 必须是这个 App 对应的 Agent（AppId = AgentId）。
   - 系统服务可以通知任何用户。
   - 第一段要求 `source.agent_did` 存在（非 Agent 来源见 §9.3）。
   - RBAC 方面，`agent`、`agent_runtime`、`system:*` 已有 `obj://msg-center/*`。
2. **建 Task**：按 §3.1 创建并授权，幂等。
3. **写索引**：写 `notifies` 行，状态为 `delivering`。
4. **主记录**：确保目标用户的通知会话已登记（§3.3），写入主记录，按 §3.3 的规则决定已读或未读；幂等键 `notify:<notify_id>:home`。
5. **另投**：按等级选会话，逐个 `post_send`，幂等键 `notify:<notify_id>:<session_id>`；结果记入 `deliveries`。
6. **写观察记录**：主记录和每个另投的会话都写一条（§3.9），幂等键 `notify-obs:<notify_id>:<session_id>`。
7. **完成**：行改为 `delivered`。

中途崩溃时，同键重调会从断点继续；清扫器也会补完仍为 `delivering` 的行。

**`notify.get{notify_id}`**：返回 `NotifyView{notify_id, url, kind, level, status, prompt, source, created_at_ms, expires_at_ms, home, deliveries, result?, completed_by?, closed_reason?}`。目标用户、请求方和系统服务可以读。

**`notify.list{status: open | closed | all, kind?, agent_did?, session_id?, goal_id?, since_ms?, cursor, limit}`**：只返回调用方所属用户的 Notify（Zone 服务按用户隔离）。状态以 TaskMgr 为准：先按行取，再逐条查 `get_task` 合并。

**`notify.cancel{notify_id, reason}`**：请求方、目标用户或系统服务可调用。msg-center 作为 root creator 发 `request_control(Cancel)`；HumanSet 任务会直接关闭为 Canceled。原因记入 `closed_reason`。

**`notifies` 表**：`notify_id`、`owner_user`、`requester`、`source`（JSON）、`kind`、`level`、`title`、`created_at_ms`、`expires_at_ms`、`state`（`delivering | delivered | closed`）、`closed_reason`、`closed_at_ms`、`home`（JSON）、`deliveries`（JSON）。它只是索引与投递记录，不是状态真相。

**清扫**（30 s 一次，与 settings 重读同一类循环）：

- 补完 `delivering` 的行；
- 对 `expires_at_ms < now` 且 Task 未结束的 Notify 发 Cancel，记 `expired`；
- 把已结束的 Task 对应的行标为 `closed`。

### 3.3 通知会话、另投与消息形态

**通知会话**（每个来源一个）：

- 第一段的来源只有 Agent。会话 id 为 `notify:<agent_did>`，peer 为该 Agent，标题“通知”。第一次投递时，msg-center 用 `msg.create_session` 那套登记在目标用户名下建好；MessageHub 中它显示在该 Agent 实体下。
- 每个 Notify 在其中写一条主记录：`from = agent_did`，`to = [用户 DID]`，`to_session = notify:<agent_did>`，走原生投递。
- 用户可以在通知会话里直接打字。消息按普通路由进入该 Agent 的一个 UI Session（route `<agent>/notify:<agent_did>`），这个 Session 能观察到所有通知（§3.9），适合说“这类提醒以后别发了”之类的话。

**候选会话**（另投用）：目标用户在该 Agent INBOX 中说过话的会话，与 Loader 路由到 UI Session 的是同一批，但不包括通知会话本身。每个候选记 `{session_id, platform, to, to_session, last_active_at}`：

- `to`、`to_session` 按 `ReplyRoute::of_msg`（`lib_opendan/src/protocol/input.rs:860-874`）的同一规则，从用户在该会话的最近一条消息推出；
- `last_active_at` 取该消息的时间。

只考虑私聊与 Telegram 私聊，不投进群。

| level | 通知会话的主记录 | 另投到 |
|---|---|---|
| `quiet` | 写入即标为已读 | 不另投 |
| `normal`（缺省） | 有另投时标为已读，否则未读 | `source.conversation`（仍在候选中时） |
| `timely` | 同上 | `normal` 的另投，再加最近活跃的会话（两者不同则两处） |
| `urgent` | 未读 | 全部候选会话 |

未读只落在用户最可能看的那一处，其余副本只是同一个 Notify 的入口。没有候选时只有主记录。不预设“IM 一定比网页强”的排序（纪要 §6.4）。

**消息**：

- `kind = notify`；
- 正文为 `text/plain`：标题、说明、选项列表（`input` 时），最后一行“处理：<url>”。这样不认识卡片的客户端（如 Telegram）也能读懂；
- `meta.system_notify = {notify_id, url, kind, level, home: true | false, source: {session_id, goal_id}}`。

**会话活跃**：`kind = notify` 计入会话活跃并解除归档（改 `owner_session.rs:165-170`）。

**Telegram**：只发纯文本加链接，链接在手机浏览器中打开后经 SSO 登录，进入 MessageHub 的卡片。

### 3.4 MessageHub：通知卡片、通知会话与待处理入口

- **通知卡片 `NotifyCard`**：凡带 `system_notify` 的 `kind = notify` 消息（主记录和另投副本）都渲染成卡片：
  - 显示标题、说明、来源（Goal 名 / Work 的目标 / 来源会话链接）、等级、状态徽标；
  - 状态读 `notify.get`（或 `get_task`），kevent `/task_mgr/<id>` 提示重读。同一个 Notify 的所有卡片状态一致（纪要 §6.1）；
  - 未结束时按 prompt 原地处理：`Choice` 渲染为按钮或单选，`TextInput` 渲染为输入框加提交，`notice` 显示“知道了 / 忽略”；
  - 提交时先读最新快照，再按 §3.1 的 result 结构 `commit_result` 并带 `expected_revision`。冲突时重读并显示最终结果；
  - 结束后显示结果、提交者、时间；
  - **不改变会话输入框的状态**：Work 被阻塞不等于这个 UI 会话被阻塞（纪要 §6.5）。
- **通知会话**：显示在 Agent 实体下，可筛“全部 / 待处理 / 已处理”（按卡片状态，在客户端过滤）。
- **待处理通知入口**：MessageHub 中一个跨来源的入口，带数量，列出 `notify.list{status: open}` 的结果，点击跳到对应卡片，可按类型和来源筛选。位置按 MessageHub 现有布局定，倾向放在 EntityList 顶部作为伪实体。
- **深链**：`/messagehub?notify=<id>` 先调 `notify.get` 取 `home`，再复用现有的 `entityId / sessionId / messageId` 深链打开卡片。手机上从 Telegram 链接进入同样可用。
- **会话模型**：`kind = notify` 计入活跃（`sessionModel.ts:17-20`）；`protocol/msgobj.ts` 增加 `system_notify` 类型；在 `UI_DATAMODEL.md` 补上 notify 投影。
- **TaskCenter 只做兼容**：
  - 带 `input.notify` 的任务不进入首页的“System Notifications”卡片；
  - 任务列表和详情给出“在 MessageHub 中处理”的链接（指向 `?notify=`）；
  - TaskCenter 的定位与需求文档不改。

### 3.5 Agent 侧：`NotifyHost` 与来源

libopendan 新增宿主接口（`RunnerDeps.notify` / `HostDeps.notify`，缺省 `None`；为 `None` 时维持今天的行为，CLI 报 “notifications are not available here”）：

```rust
#[async_trait]
pub trait NotifyHost: Send + Sync {
    async fn send(&self, req: NotifyRequest) -> Result<NotifyTicket, NotifyError>;          // 按 key 幂等
    async fn state(&self, notify_id: &str) -> Result<NotifyState, NotifyError>;
    async fn list(&self, q: NotifyQuery) -> Result<Vec<NotifyState>, NotifyError>;
    async fn cancel(&self, notify_id: &str, reason: &str) -> Result<NotifyState, NotifyError>;
    async fn answer(&self, notify_id: &str, a: NotifyAnswer, via: NotifyVia) -> Result<NotifyState, NotifyError>;
}
```

**实现**：

- `opendan::notify::ZoneNotify`：调用 msg-center `notify.*`；`answer` 直接对 TaskMgr 做 `commit_result` 并写 `add_task_note`。
- `libopendan::notify::LocalNotify`：文件实现 `<agent_root>/state/notifies/<id>.json`，供 `--dev`、xagent 和测试使用；`xagent notify answer` 扮演用户。

**CLI 的通路**：照 `goals.*` 的样板，`AgentStateClient::notifies()` 的文件版是 `LocalNotify`，kRPC 版由 Loader 透传给 `ZoneNotify`。

**来源计算**（`notify_source(session)`）：

- 基本字段：`agent_did`、`app_id`、`session_id`、`turn`、`call_id`（CLI 取 `$XLLM_CALL_ID`）、`goal_id`（取自 `session.goal` 或 `origin.goal`）。
- `conversation` 按 Session 类型取：
  - UI Session：自己的 `channels.outbound`；
  - Work：沿 `origin.parent_session` 找到最近的 UI 祖先，取它的 `channels.outbound`；
  - Goal：`created_by.via = ui_session:<sid>` 那个 Session 的 `channels.outbound`。
- 找不到会话时为空。

### 3.6 Work 的 ask-user

1. **入口不变**：CLI `ask-user` 照旧打印 `Pending{task_id: "userinput:<call_id>"}`。由 runner 按 Session 决定走哪条路：
   - `ask::allowed`（UI，原地问答）不变；
   - 新增 `ask::via_notify`：`SessionKind::Work`、没有输入队列、`deps.notify` 存在且 `ask_user_timeout_secs > 0`。
   - 其余情况维持报错：交互式子 Session、self_*、Goal。Goal 的报错文本改为 “in a goal session put the question in `questions` of your report; it reaches the user and the answer comes back as a later activation”。
2. **登记**：`on_pending` 校验后把问题记入 run.json `asks[call_id]`，附 `deadline_ms = now + ask_user_timeout_secs`（缺省 10800，即 3 小时）。
3. **发出**：在提交等待状态之前，对尚未发出的问题调用 `notify.send`：
   - `kind = input`，`level = timely`；
   - 幂等键 `<sid>:<turn>:ask:<call_id>`，`expires_at_ms = deadline_ms`。

   拿到的 `notify_id` 记入 `asks[call_id]`，然后与 `waiting_for = {kind: Input, refs: ["userinput:<call_id>"], deadline_ms}` 一起提交。顺序是先发后提交；崩溃后重做会拿到同一个 Notify。
4. **Turn task**：等待原因为 `HumanInput`，code `ask_user`，`related_task_id = notify_id`，文案为 “waiting for the user's answer (notification)”。
5. **回填**：每次 drive 对经 Notify 等待的调用查询 `state`：
   - `open` 且未到截止时间：继续等。
   - `answered`：回填 `UserInputAnswer{text, option, in_reply_to_question: true, msg_id: null, from: submitted_by, at_ms: submitted_at}`，另加 `via: "notification"`。其中 `text` 取 `values.text`，没有时取选项 label。
   - `dismissed / expired / canceled`，或已到截止时间（此时先 `cancel(id, "ask_timeout")`）：回填 `{"answered": false, "reason": "timeout | dismissed | canceled", "note": "decide with what you have, or end with a failure report that says what is missing"}`。

   回填与提交沿用 `fill_answer` / `commit_answer` 的路径，但没有输入消息可确认，receipt 的 `inputs` 为空。
6. **唤醒**：Session 被卸载后，以下情况让它重新装载：
   - Supervisor 对 `waiting_for.kind = Input` 且无队列的 Session，按 `min(deadline_ms, 30 s recheck)` 重新装载；
   - Loader 订阅 `/task_mgr/<notify_id>` 后调用 `ensure_task(sid)` 加速。
7. **stop**：现有路径回填 “the user did not answer: the turn was stopped”，另外 `cancel(id, "turn_stopped")`。
8. **崩溃窗口**：发出前、发出后提交前、提交后、Session 停机期间被回答，四种都要恰好一个 Notify、一次回填。
9. **提示词**：Jarvis 的 `plan / do / work_simple` 加一段说明：
   - 缺关键前提时，优先用失败报告说明缺什么；
   - 只有已投入大量工作时才用 `agent-session ask-user`；
   - 用户可能很晚回答或不回答，最多等 3 小时。

### 3.7 Goal 的提问

- **`GoalQuestion` 增加字段**：`notify_id`、`status`（`open | answered | dismissed | expired | canceled`）、`answer`。
- **发出与撤销**：驱动者在 `close_activation` 提交之前处理（与 Goal State 单写者一致）：
  - 对新出现的问题调用 `notify.send`：`kind = input`，只带 `TextInput`，`level = normal`；
  - 幂等键 `goal:<goal_id>:q:<sha256(text) 前 16 位>`；`source.goal_id` 与 `conversation` 按 §3.5；
  - 从 `questions` 中消失的问题执行 `cancel(id, "goal_dropped")`。

  `questions` 每次 Activation 都被整体替换（`runner/goal.rs:870-880`）。同一文本命中同一个 key，所以不会重复发。
- **回答回流**：GoalHost（宿主）监视本 Goal 未结束的问题 Notify（`notify.list{goal_id, status: open}` 加 kevent）。一旦结束，`post_input` 到当前 epoch 的 Goal Session：
  - `AgentEvent{source: system:goal, event: "answer", key: goal:<id>:answer:<notify_id>, data: {question, notify_id, status, answer}}`；
  - Activation 原因增加 `answer`，Goal 头的 `<open_questions>` 带上状态与回答；
  - 驱动者消费该事件时更新问题状态。
- **WAIT_USER_MSG**：Goal 遇到它时，现在会把最后的回答追加为问题（`runner/goal.rs:885-893`），这类问题走同一条路。
- **`goal.toml` 提示改写**：
  - questions 会作为通知到达用户，回答会在以后的 Activation 中以原因 `answer` 回来；
  - 只问查不到的事；问题要能脱离上下文读懂。

### 3.8 `agent-session notify` CLI

```text
agent-session notify send --title <t> [--message <text>] [--level quiet|normal|timely|urgent] [--expires-in <dur>] [--key <k>]
agent-session notify list [--open] [--any-source] [--session <sid>] [--goal <id>]
agent-session notify show <notify_id>
agent-session notify answer <notify_id> (--option <value> | --text <t>)
agent-session notify ack <notify_id>
agent-session notify cancel <notify_id> [--reason <r>]
```

- **`send`**：打印 `{notify_id, url, status, home, deliveries}`；缺省 key 为 `<sid>:<turn>:notify:<call_id>`。
- **`list`**：缺省列出本用户的、来源是本 Agent 的、未结束的 Notify。
- **`answer` / `ack`**：只允许 `input` / `notice`；`approval` 返回 “approvals are decided by the user: <url>”。以下情况被拒，并输出当前状态：来源 Session 就是调用方自己；Notify 已结束；选项不存在。
- **`cancel`**：只能撤销来源是本 Agent 的 Notify。
- **`xagent notify list|show|answer|ack|cancel`**：主人一侧的 CLI，`--dev` 与测试用它扮演用户。

### 3.9 UI 观察：观察记录与 `_notify`

**观察记录**：msg-center 把 Agent A 来源的 Notify 写进会话 S 后（S 包括通知会话 `notify:<A>` 和每个另投的会话），进程内写一条记录到 A 的 INBOX `<A>/<S>`：

```json
{ "from": "<A>", "to": ["<A>"], "to_session": "<S>", "kind": "event",
  "content": { "machine": { "intent": "buckyos.notify", "data": {
    "event": "delivered", "notify_id": "…", "url": "…", "kind": "input", "level": "timely",
    "title": "…", "message": "…", "options": [ … ],
    "source": { "session_id": "…", "goal_id": "…", "operation": "…" },
    "home": false, "delivered_msg_id": "…",
    "reply_route": { "to": "<用户端点>", "to_session": "<S>" },
    "at_ms": 0 } } } }
```

**Loader 路由**：在 `bridge/msg.rs::route_msg_record` 的主人过滤之前加一条规则：`from` 为 Agent 自己且 intent 为 `buckyos.notify` 的记录，转成 `AgentEvent{source: system:notify, event: delivered, key: notify:<id>:<S>, data}`；其它发给自己的记录照旧丢弃。

**Session 的建立**：S 没有 UI Session 时，`session_for` 照常新建一代，`OutboundBinding` 取自 `reply_route`。观察记录不会以别的方式决定回复坐标。所以每个 Agent 的通知会话也会有一个 UI Session：它平时只积累半订阅事件，用户在通知会话里打字时才运行。

**订阅**：ui 模板（`group` 经 `base` 继承）加隐式半订阅 `_notify`，订阅 `System{id: "notify"}`，写法照 `_user_timezone`（`protocol/config.rs:785-800`）。

**半订阅的效果**：

- 事件不唤醒 Session，只进 `pending_events`；
- 用户下一次发消息时，随该批输入出现在 `<semi_subscription_snapshot>` 中，受 `change_budget` 限制。

**缺省渲染**（assembler builtin），每条一行：

```text
[notify <id>] <kind>/<level> from <来源：goal 名 / session> at <时间>: <title> — <message 摘要> (options: …) — check the current state with `agent-session notify show <id>`
```

第一段只有“已投递”事件。状态由 Agent 按需查询，因为“已在界面呈现”不等于“已读”（纪要 §12.2）。

### 3.10 UI 代理

写进 `chat_route` / `groupchat_route` 的提示：

1. `[notify …]` 行表示用户在这个会话里看到过这条通知。
2. 用户的消息明确回应某条未结束的通知时（内容对得上，或直接回复了那条消息），先用 `notify show` 查当前状态，再 `notify answer` 或 `ack`，然后简短告诉用户。
3. 有多条未结束的通知、回复又含糊（如“好的”）时，问是哪一条，不要猜（纪要 §12.5 第一条）。
4. 改变安排的反馈不是回答，例如“今天在度假，别打扰我”：
   - 有 Goal 来源时用 `agent-session goal post <goal_id> --msg …` 转给 Goal；
   - 否则按普通请求处理；
   - 不要把一次性的话扩大成长期规则（纪要 §12.6）。
5. 不处理审批，引导用户打开链接。

宿主侧的检查见 §3.8。每次代答都写 task note：`answered by <agent> in session <sid> after input <key>`。

---

## 4. 第一段实施 TODO

### N1：buckyos-api 与 TaskMgr

- [ ] `msg_center_client.rs`：`NotifyKind / NotifyLevel / NotifySource / NotifySendReq / NotifyView / NotifyListReq / NotifyCancelReq`；客户端方法 `notify_send / get / list / cancel`；方法常量。
- [ ] `taskdata.rs`：`NotifyEnvelope` 与 §3.1 的 input / result 约定（复用 `HumanTaskPrompt`、`TaskDataResult::Human`）。
- [ ] `task_mgr.rs`：内建 schema `human.notice/v1`（HumanSet only；input 为 envelope，output 为 `action ∈ {ack, dismiss}`），加入内建 schema 表（:1113 附近、:1185-1228 的样板）。
- [ ] 核对 `TaskGrantSubject::Principal` 在 ACL 中的匹配（`acl.rs:238-243`），以及 assignee 加上 User grant 后能读 result。
- [ ] 测试：schema 校验；HumanSet notice 的 ack / dismiss；grant 后请求方可读 result 并可 cancel。

### N2：msg-center `notify.*`

- [ ] kRPC 分发与权限（§3.2 第 1 步）；`notifies` 表与迁移。
- [ ] `send`：
  - 建 Task 与授权；
  - 通知会话的登记与主记录，以及已读规则（§3.3）；
  - 候选会话与等级规则；消息组成与 `post_send`；
  - 观察记录；幂等与断点续投。
- [ ] `get / list / cancel`；清扫器（补投、过期、关闭）。
- [ ] `kind = notify` 计入会话活跃（`owner_session.rs:165-170`）。
- [ ] 测试（`test_msg_center.rs`）：
  - 四个等级下主记录的已读状态与另投位置；没有候选；
  - 幂等重放不重复发消息；
  - App 不能通知别的用户；
  - `list` 按用户隔离；过期与撤销；
  - Telegram（DryRun 网关）收到带链接的文本；
  - 观察记录写在 Agent 的 INBOX，`to_session` 正确（含通知会话）。

### N3：libopendan 协议与宿主接口

- [ ] `protocol/notify.rs`：`NotifyRequest / NotifyState / NotifyAnswer / NotifyVia`；schema 导出。
- [ ] `notify.rs`：`NotifyHost`；`LocalNotify`；`notify_source`。
- [ ] `state/`：`AgentStateClient::notifies()`；kRPC `notify.*`（读方法、`send / answer / cancel` 都按调用方 Session 记录来源）。
- [ ] `template.rs` / `protocol/config.rs`：ui 模板的隐式半订阅 `_notify`；assembler builtin 渲染 `system:notify` 事件。
- [ ] `protocol/goal.rs`：`GoalQuestion` 的 `notify_id / status / answer`；Activation 原因 `answer`。
- [ ] `[session.work] ask_user_timeout_secs`（缺省 10800，0 关闭）。
- [ ] 协议文档与 fixtures（新增一个 `17_notify`：Work 经 Notify 问答、Goal 问题回流、`_notify` 事件）。

### N4：libopendan runner

- [ ] `runner/ask.rs`：`via_notify` 分支、发出、截止时间、回填（§3.6）；Goal 报错文本。
- [ ] `runner/drive.rs` / `live.rs`：经 Notify 等待的调用查询与回填；stop 时撤销；`commit_answer` 处理无输入的 receipt。
- [ ] `runner/turn_task.rs`：`HumanInput` + `related_task_id`。
- [ ] `runner/goal.rs`：问题发出与撤销（close 前）；`answer` 事件的消费与状态更新。
- [ ] `host/`：Supervisor 对无队列、等输入的 Session 按截止时间与 recheck 重新装载；GoalHost 监视问题 Notify 并投递 `answer`。
- [ ] 测试（`tests/ask_user.rs` 扩展、`tests/goal_session.rs`、`tests/crash.rs`）：§5 的 V07—V15。

### N5：xagent CLI

- [ ] `agent-session notify send|list|show|answer|ack|cancel`（§3.8）；`xagent notify …`（主人一侧）。
- [ ] USAGE、退出码（越权为 6，状态冲突为 5，与现有约定一致）。
- [ ] `tests/xagent_cli.rs`：真实二进制 + `LocalNotify` 的往返。

### N6：OpenDAN Loader

- [ ] `notify.rs`：`ZoneNotify`；kRPC 透传；监视器（kevent `/task_mgr/<id>`，并以 30 s 扫描兜底），驱动 `ensure_task` 与 Goal 的 `answer`。
- [ ] `ui.rs` + `bridge/msg.rs`：观察记录路由；`session_for` 从 `reply_route` 取 binding。
- [ ] `tasks.rs`：wait reason 映射（:288-301）。
- [ ] `tests/loader.rs` 端到端（脚本化 LLM + 进程内假 msg-center / TaskMgr，或 `LocalNotify`）：Work 提问 → 回答 → 继续；观察事件进入下一批输入。

### N7：Desktop

- [ ] MessageHub（§3.4）：
  - `protocol/msgobj.ts` 的 `system_notify` 类型；
  - `NotifyCard`：按 prompt 渲染与提交、实时状态、结束后的结果；
  - 通知会话的标题与筛选；待处理通知入口（`notify.list`）；
  - `?notify=` 深链；`sessionModel.ts` 活跃规则；
  - TS 客户端：`notify.get / list`，以及 TaskMgr `get_task / commit_result`。
- [ ] mock 场景：Work 提问（来源会话里的副本 + 通知会话主记录）、Goal 提问、提醒、已处理的历史。
- [ ] TaskCenter 兼容：带 `input.notify` 的任务不进首页卡片；列表与详情链接到 `?notify=`。
- [ ] i18n（中英）；`UI_DATAMODEL.md`。
- [ ] 测试：
  - datamodel：notify 投影，同一 Notify 的多张卡片状态合并；
  - Playwright：卡片的 choice / text / notice 提交；两张卡片并发提交，一张成功、另一张显示最终结果；通知会话筛选；待处理入口；深链；输入框不被锁定；TaskCenter 首页不出现 Notify 任务。

### N8：Jarvis 包

- [ ] `plan.toml` / `do.toml` / `work_simple.toml`：ask-user 兜底说明（§3.6 第 9 条）。
- [ ] `goal.toml`：questions 改写（§3.7）；可以用 `notify send` 告知。
- [ ] `chat_route.toml` / `groupchat_route.toml`：§3.10 的观察与代理规则。
- [ ] `agent.toml`：`[session.work] ask_user_timeout_secs = 10800`。

### N9：（取消）workflow `remind` —— 用户决定暂不处理

### N10：文档

- [ ] 架构设计：
  - §5.6 的“Work 与 Goal 报错”改为 §3.6 / §3.7 的规则；
  - §12.11 / §12.12.2 补上“Work 的有界 Notify 等待（缺省 3 小时）”及其与“长时间 PendingTool 等审批”的区别；
  - §15.5 增加一行 System Notify；
  - §17.2 标出已采用的部分。
- [ ] `xAgent.md`（CLI、模板表、`_notify`）；协议文档：
  - Session Input：`_notify`、经 Notify 的 ask 结果、Goal `answer`；
  - Agent State：`notify.*`；
  - Session Directory：run.json `asks[].notify_id / deadline_ms`。
- [ ] `Message Center.md`：`notify.*`、通知会话与主记录、另投与已读规则、观察记录、`kind = notify` 计入活跃。
- [ ] `UI_DATAMODEL.md`：通知卡片、通知会话、待处理入口、`?notify=` 深链。
- [ ] `task data schema.md`：`human.notice/v1` 与 notify envelope。
- [ ] `src/frame/lib_opendan/README.md`、`src/frame/opendan/README.md`。
- [ ] 纪要本身不改；在本文 §11 记录对应关系。

### N11：验证（第一段的退出条件）

- [ ] §5 全部通过；§6 全绿；DV 四个场景（V20）记录消息、Task 和 Activation 序列。

---

## 5. 第一段验收矩阵

| # | 场景 | 期望 |
|---|---|---|
| V01 | `notify.send` notice / normal | 建出 HumanSet `Waiting` 的 Task；第一次投递时自动登记通知会话（标题“通知”）并写入主记录；另投到来源会话，消息带链接；重放同一个 key 返回同一个 Notify，不重复发消息 |
| V02 | 等级与已读 | `quiet` 只有主记录且已读；`normal` 有来源会话时另投一处、主记录已读，没有时只有主记录且未读；`timely` 另投来源会话和最近活跃会话；`urgent` 投全部候选，且都未读 |
| V03 | 权限与隔离 | App 不能通知别的用户；`source.agent_did` 与调用方不符时被拒；没有 `agent_did` 时被拒（第一段）；`list` 只返回调用方用户的 Notify |
| V04 | 过期与撤销 | 清扫器关闭为 `expired`；`cancel` 后为 `canceled`，原因正确 |
| V05 | MessageHub 卡片 | 按 prompt 处理 choice / text / notice；主记录与副本的状态一致；两处并发提交时一处成功，另一处显示最终结果；输入框不被锁定 |
| V06 | 通知中心 | 通知会话的“全部 / 待处理 / 已处理”筛选；待处理入口的数量与列表；`?notify=` 深链打开对应卡片；在通知会话里打字会进入该 Agent 的 UI Session，并能看到通知 |
| V07 | Work `ask-user` | 建出 `human.input` Notify（来源含 session / turn / call，`conversation` 是父 UI 的 binding，level `timely`）；`waiting_for = Input`，`deadline_ms = now + 3h`；Turn task 为 `Waiting(HumanInput, related_task_id)` |
| V08 | 在 MessageHub 卡片回答 | Work 恢复，工具结果的 text 与 option 正确，`via = notification` |
| V09 | 超时 | Notify 被撤销（`ask_timeout`）；工具结果为 `answered: false, reason: timeout`；Work 继续由 LLM 决定 |
| V10 | 用户忽略 / 请求方撤销 | 工具结果为 `dismissed` / `canceled` |
| V11 | 等待中 stop | Notify 被撤销（`turn_stopped`），Turn 以 Stopped 结束 |
| V12 | kill -9：发出前 / 发出后提交前 / 提交后 / 停机期间被回答 | 恰好一个 Notify、一次回填 |
| V13 | 等待中的 Work 被卸载 | 回答后由 kevent 唤醒；没人回答时到截止时间被唤醒 |
| V14 | Goal questions | 新问题发出 Notify；连续 Activation 中重复出现的问题不重发；消失的问题被撤销；回答后产生原因 `answer` 的 Activation，Goal 头带上回答；问题状态更新 |
| V15 | 回归 | Goal 内 `ask-user` 报新文本；UI 内 `ask-user`（意图分析 TODO V05—V08）不变；`deps.notify = None` 时 Work 维持原报错 |
| V16 | 观察 | 写进会话的 Notify 在对应 UI Session 产生 `_notify` 事件：不开 Turn，随下一批输入出现在快照中；没有 Session 时新建，binding 来自 `reply_route` |
| V17 | 代理 | UI Session 代答 `input` 后，Work 恢复，并留有 task note；代答 `approval` 被拒；代答本 Session 发起的 Notify 被拒；代答已结束的 Notify 返回最终状态 |
| V18 | `notify send` | UI / Work / Goal 中都可用；`list / show / cancel` 正常 |
| V19 | TaskCenter 兼容 | Notify 任务不出现在 TaskCenter 首页的 approve / reject 卡片中；列表与详情有到 MessageHub 的链接 |
| V20 | DV | ① Work 中途提问 → 用户在 MessageHub 卡片回答 → Work 完成；② 同样的提问另投到 Telegram（DV 没有绑定时用原生会话），用户在会话里直接用文字回答 → UI Session 观察到并代答 → Work 继续；③ Goal（`every_secs`）的 Activation 用 `notify send` 发提醒 → 用户在会话里回复“今天在度假，别打扰” → UI Session 识别为反馈并执行 `goal post` → 下一次 Activation 据此调整；④ Goal 提问 → 用户回答 → 产生原因 `answer` 的 Activation |

---

## 6. 验证命令

Rust（在 `src/`）：

```bash
cargo test -p buckyos-api -- --test-threads=1
cargo test -p task_manager -- --test-threads=1
cargo test -p msg_center -- --test-threads=1
cargo test -p libopendan -- --test-threads=1
cargo test -p opendan -- --test-threads=1
cargo check --workspace --tests
cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures   # 改协议后
```

Desktop（在 `src/frame/desktop/`）：tsc、eslint、deno datamodel；Playwright 跑 MessageHub（含新的 notify spec）与 TaskCenter 的 spec，`--workers=2` 并分批，避免与 cargo 并行时被 OOM 杀掉。

DV：

- msg-center 与 TaskMgr：`uv run buckyos-build.py -s msg_center`（以及 `-s task_manager`）后 `uv run start.py`。这会停系统并更新安装文件，不是普通重启。
- Agent：`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`。
- desktop 静态文件：`cp -r dist/. /opt/buckyos/bin/control-panel/web/`。
- 驱动脚本：在 `test/test_opendan/` 新增 `test_agent_notify.ts`，覆盖 `send / answer / wait`，可参考 `test_agent_ask.ts`。
- 注意：
  - 已有的 UI Session 冻结了旧的 behavior，要发 `/stop` 开新一代；
  - DV 结束后把测试 Goal 归档；
  - 意图分析 TODO §11.5 记录的旧 `/5` UI Session 卡点要先解决。

---

## 7. 实施顺序

1. N1 → N2：类型与 schema，然后是 msg-center。不依赖 OpenDAN，可以单独测。N7 的 MessageHub 部分可以先用 mock 并行做。
2. N3 → N4 → N5：libopendan 用 `LocalNotify` 跑通库级测试。
3. N6（Loader）、N7（接真实后端）、N8（Jarvis）。
4. N11 的 DV；最后回写 N10 文档。

---

## 8. 决定与待确认

### 8.1 用户答复（2026-10-11）

1. System Notify 放在 msg-center 里做。
2. UI 放在 MessageHub：MessageHub 本身就擅长展示来自不同来源的会话。→ 每个来源一个通知会话，Notify 是其中的卡片，另有待处理入口和 `?notify=` 深链（§3.3、§3.4）；TaskCenter 不当通知中心，只做兼容。
3. Work 的等待缺省 3 小时。
4. workflow 先不管（那边还没正式用起来）。→ `remind`、`dcrontab` 不在本文范围。

### 8.2 未单独答复、按推荐写的事项（实施前可再调）

| # | 事项（纪要 §13.2 对应） | 取值 |
|---|---|---|
| Q1 | Notify 与 Task 的关系 | 一对一，`notify_id = task_id`；无需回复的告知也建 Task（`human.notice/v1`） |
| Q4 | 通知策略 | 四个等级加 §3.3 的规则；不回退、不升级；第一段只支持 Agent 来源 |
| Q5 | 状态与时限（除 Work 时长外） | 状态为 `open / answered / acknowledged / dismissed / expired / canceled`；告知缺省不过期（可给 `expires-in`）；Goal 提问不过期，从 `questions` 消失即撤销；迟到的回答因 Task 已结束而被拒，界面显示最终状态 |
| Q6 | `ask_user` 约束 | 所有无队列的 Work 都可用，0 表示关闭；交互式子 Session 与 self_* 仍报错；Goal 走 questions；提示词保持“优先快速失败” |
| Q7 | 可恢复等待 | run.json `asks[call_id]` 保存 `notify_id` 与 `deadline_ms`；先发（幂等）后提交 |
| Q8 | 多入口一致性 | 所有入口都直接写 TaskMgr，带 `expected_revision` CAS，先到先得 |
| Q9 | 自然语言代理 | UI Session 可以代答 `input`、代确认 `notice`；`approval` 只能用户本人处理；不能处理本 Session 发起的；代答写 task note |
| Q10 | 环境观察接口 | INBOX 观察记录转成 `_notify` 半订阅事件；只有“已投递”事件 |
| Q12 | 第二段范围 | `post-message`、机械审批、触达设置与升级；机械审批是否近期做，按“先能力后安全”另议 |
| — | Reply Receipt 与路由 | 第二段 §9.1 |

### 8.3 本文的实现决定（如需调整，请在实施前提出）

- LLM 只看到 `ask-user` 一个命令；走原地问答还是 Notify 由 Session 决定。Work 的问题用 `timely`，Goal 的问题与 `notify send` 缺省用 `normal`。
- 未得到回答时用成功结果加 `answered: false` 表示，不用错误观察，让 LLM 按正常分支处理。
- 代答所在的 Session 与触发输入写 task note，不扩展 `TaskDataResult::Human`。
- Notify 不进 Session 的 outbox（`OutboxPurpose`），也不写成会话历史里的 assistant 或 user message；UI Session 只通过 `_notify` 观察。
- 通知会话 id 为 `notify:<agent_did>`；主记录的已读规则见 §3.3。另投只走私聊与 Telegram 私聊，不投进群。
- 统一 URL 是 `/messagehub?notify=<id>`，不带消息 id。这样链接在发消息之前就能确定，正文里可以直接写。
- TaskCenter 首页排除 Notify 任务，避免同一件事在两处各有一张卡，也避免 approve / reject 的错误提交。
- `deps.notify = None` 时一切维持今天的行为，xagent 和 fixtures 也一样。
- `LocalNotify` 只用于 `--dev` 和测试，不进 zone 形态。

### 8.4 已知风险（按“先能力后安全”记录，不加机制）

- **Agent 能直接提交主人的人工任务**：Agent 进程的主体是 `app:<agent>@<owner>`（`opendan/src/tasks.rs:274-279`），TaskMgr 按 user_id 判断 assignee，commit 不检查 App（`server.rs:1778-1792`）。所以 Agent 可以绕过 CLI 规则，直接 commit 指派给主人的任何 HumanSet 任务。第一段的 `approval` 没有生产者，影响有限；在审批有生产者之前（§9.2）必须先关上这个口子。实施前核对 Agent token 的 user_id。
- **会话之间可以冒充消息**：`agent-session post <sid> --msg --from <任意 DID>`（`bin/xagent.rs:433-437, 723-735`）能把冒名的消息投进任何有队列的 Session，与纪要 §8.2 相反。建议之后在 Session 内把 `--from` 限制为调用方本身。
- UI Session 可能把一句含糊的“好的”归到错误的 Notify 上，只靠提示词约束。
- Notify 没有频率限制，Agent 可能滥发，通知会话会变吵。
- 等待中的 Work 最多占着一个 `max_sub_sessions` 名额（缺省 4）3 小时。
- 每个 Agent 的通知会话都会对应一个 UI Session，平时只积累半订阅事件。
- Notify 任务仍出现在 TaskCenter 的任务列表里（只是不进首页卡片）。
- 观察事件只在投递时产生，Agent 看到的状态可能已经过时。CLI 提交冲突时会返回最终状态，可以兜底。
- kevent 是尽力而为的，丢失时靠 30 s 扫描兜底，回答回流因此可能延迟到 30 s。
- Telegram 链接在手机上要先经 SSO 登录。

---

## 9. 第二段（第一段验收后细化，现在不实施）

### 9.1 `post-message` 与回复关联

- 用 CLI `agent-session post-message --to <did | 联系人> [--via <platform>] --text … [--subject …]` 投递，立即返回收据：`{receipt_id, msg_id, to, platform, account, sent_at, state, correlation}`。
- 发送路径要接上 `contact.resolve_target / get_preferred_binding`（`contact_mgr.rs:266-385`，目前没有调用方）。
- msg-center 增加 `post_receipts` 表。入站时先做关联：
  - 原生消息看 `relates_to{thread}` / `thread.reply_to`；
  - Telegram 看 reply_to_message；
  - 有邮件 tunnel 之后，用标题标识。

  关联上的回复存在收据下面，不再路由进 UI Session 的 INBOX；关联不上的照常路由（纪要 §9.5）。
- 取回复：
  - Work 用 `agent-session replies <receipt>` 拉取，或用有界的 `wait-reply`（PendingTool `reply:<receipt>`）；
  - Goal 在收到回复时产生原因为 `reply` 的 Activation。
- 待定：标识被删改时怎么处理；多重匹配；迟到的回复。

### 9.2 机械审批

- 先单列 llm_context TODO：
  - `PolicyEngine` 在“需要审批”时让调用挂起为 Pending `approval:<notify_id>`，LLM 只看到工具未返回；
  - 状态显示 `wait_user_approve`；
  - 哪些操作要审批：`HumanPolicy.approval_required` 已有但从未被读取，需要决定由谁配置。
- `human.approval/v1` 的严格 schema 改为 notify envelope（目前没有生产者，可以直接改）。
- TaskMgr 对审批的 commit 只接受受信 UI App（control-panel），关闭 §8.4 第一条风险。
- 超时的语义（按拒绝处理）。
- 当前 UI Session 的调用被阻塞时，MessageHub 用毛玻璃遮罩加模态框；后台 Work 被阻塞时只显示通知卡片（纪要 §6.5）。
- 审批 Notify 是否作为 Turn task 的子 task（TaskMgr §11.7，以及“stop 让审批作废”）：这需要 msg-center 在 Agent 的 Turn task 下有 CreateChild 权限，或改由请求方自己建 Task。

### 9.3 触达策略与来源扩展

- 用户的通知设置：放在 control_panel 资料的 `system_contact`，还是 msg-center 自己存。
- 未处理时的升级（多久升一级）；新渠道：邮件、短信网关、BuckyOS App 推送；勿扰时段；频率限制。
- 桌面状态栏托盘接入：`notificationCount` 取待处理 Notify 数，点击进入 MessageHub 的待处理入口。
- 非 Agent 来源（系统服务、App、计划任务）：需要一个系统发送者，以及它在 MessageHub 中的通知会话（`notify:<app_id>`）。

### 9.4 状态变化的观察

- 回答、过期等变化也作为 `_notify` 事件投给相关 UI Session。
- 请求方 Session 收到“你发出的通知已被处理”。

### 9.5 纪要 §三（不属于通知）

处理中补充输入的归属、占位与补充消息的排列、“明确排队 / 明确本轮吸收”两种输入方式，另立 MessageHub / Agent Chat TODO。现状是：runtime 在每次推理前读取输入，receipt 记录了 Turn 消费了哪些输入，MessageHub 的排列方式未核对。

### 9.6 智能提醒完整路线

Goal 安排一次性唤醒（Goal TODO §9.1 的 `Once` 与 §9.5 的 follow_up）→ 到点启动 Work 判断 → `notify send`。第一段只验证最后一步（V20 ③）。self_check 退役按 Goal TODO §9.3 另做；它用的 `dcrontab … remind` 本文不处理。

---

## 10. 第一段禁止的捷径

- 不要把 Notify 做成 Session outbox 中的 Assistant Message，也不要把它写进会话历史冒充 user 或 assistant 消息。
- 不要为了让 Work 收到回答而给它建 Inbox / 输入队列，也不要把用户在会话里的回复路由进 Work。
- 不要让 LLM 在 `ask-user` 里选择“原地还是通知”，也不要新增一个 `ask-user-notify` 命令。
- 不要把“已发出 / 已送达 / 已在界面呈现”当成“已回答 / 已批准 / 已读”。
- 不要在 msg-center、desktop 或 libopendan 中保存 Notify 状态的另一份真相，一律读 TaskMgr。卡片状态也不能只看消息本身。
- 不要让任何 Notify 等待没有截止时间。
- 不要因为 Work 或 Goal 的 Notify 锁住 MessageHub 输入框或整个会话。
- 不要让 Agent 处理审批，也不要让它处理本 Session 发起的 Notify。
- 不要用 Timer 或自我唤醒循环等待 Goal 的回答；回答以 `system:goal` 事件投回。
- 不要把通知中心另做在 TaskCenter；也不要实现 `post-message`、机械审批、触达升级、workflow `remind` 改造。
- 不要改 llm_context / agent_tool；需要改时单列 TODO 先行。

---

## 11. 纪要与本文的对应

| 纪要 | 本文 |
|---|---|
| §一 机制分离 | §0 |
| §二 Session Inbox / Outbox 基线、Work 不建队列 | 不变；§3.6 不给 Work 建队列 |
| §三 Turn 与处理中补充输入 | §9.5 |
| §四 LLM 澄清与机械授权 | UI 原地澄清不变；工具化询问见 §3.6；机械授权见 §9.2 |
| §五 System Notify（独立通道、两类、异步可恢复、复用 Task） | §3.1、§3.2、§3.6 第 3 / 8 条 |
| §六 触达（统一 URL、默认链接、来源与渠道分离、强度、阻塞展示、通知中心） | §3.3、§3.4（MessageHub）；强度扩展见 §9.3；模态遮罩见 §9.2 |
| §七 Work 与 `ask_user` 兜底 | §3.6 |
| §八 内部协作与外部通信 | 不变（树状 Session）；§8.4 第二条风险 |
| §九 `post_message` | §9.1 |
| §十 Work 有界、Goal 长期 | §3.6 截止时间（3 小时）；§3.7 |
| §十一 智能提醒 | §3.8 `notify send`；V20 ③；§9.6 |
| §十二 观察与代理 | §3.9、§3.10 |
| §13.2 待定项 | §8.1、§8.2 |
