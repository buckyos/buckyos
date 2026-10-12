# OpenDAN Agent 通知人 TODO（System Notify / ask-user 兜底 / 通知的观察与代理）

日期：2026-10-11

状态：v0.3.1。**第一段已实施（2026-10-11，记录见 §12），DV（N11 / V20、V21）未做**：共用的 DV zone 处于停止状态，另一会话正在改 node_daemon，未擅自重启与替换二进制，步骤见 §12.5。用户已答复的事项见 §8.1，其余待定项按推荐写（§8.2）。第二段（§9）只记方向，第一段验收后再细化。

修订记录：

- 2026-10-11 占位。
- 2026-10-11 v0.1：按讨论纪要与代码现状写成实施方案。
- 2026-10-11 v0.2：System Notify 放 msg-center；通知中心改为 MessageHub；Work 等待缺省 3 小时；workflow `remind` 暂不处理。
- 2026-10-11 v0.3：
  - Notify 本身是 msg-center 里的特殊 MsgObject，只有需要回复的才另挂 TaskMgr Task（Q1）；
  - MessageHub 按对用户的重要程度分三类展示（待办 / Agent / 应用）；
  - 第一段以“App 能稳定使用的接口”为核心，非 Agent 来源也进入第一段；
  - Q9（UI 代理）确认。
- 2026-10-11 v0.3.1：用户确认系统通知发送者用 Zone DID、主记录直写 INBOX、App 主体的 RBAC；websdk 封装放第一段。

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
| 机制分离 | 会话消息（Session Inbox / Outbox）、System Notify、`ask-user`、`post-message`、Input Task / Final Report 语义各自独立。第一段做 System Notify 这个系统能力（所有 App 可用），再在其上做三件事：Work 的 ask-user 兜底、Goal 的提问投递、UI Session 对通知的观察与代理。`post-message` 与机械审批放第二段 | 纪要 §一、§13.1 |
| 第一段的核心 | **App 使用系统通知的接口要稳定**（用户决定）：kRPC `notify.*`、请求 / 视图字段、状态枚举、分类、等级、幂等与权限（§3.0）。渠道选择与 MessageHub 的展示方式可以以后再改，甚至交给用户配置 | 用户答复 |
| Notify 是什么 | **msg-center 里的特殊 MsgObject**（`kind = Notify`，ndn-lib 已有这个 kind）：结构化内容放 `content.machine`（intent `buckyos.notify`）。告知（`notice`）只有消息，已读 / 忽略 / 过期以 msg-center 记录为准。需要回复的（`input`，以及第二段的 `approval`）**再挂一个 TaskMgr HumanSet Task**，等待与结果以 Task 为准 | 用户决定（Q1）；纪要 §5.4 的“等待复用 Task” |
| 分类 | `category` 按对用户的重要程度分三类（用户决定）：<ul><li>`todo`：别人把工作指派给你；最需要关注；预留给 workflow 等系统服务。</li><li>`agent`：来自 Agent，多半要人处理，否则工作推进不下去。</li><li>`app`：传统应用通知。</li></ul>分类由 msg-center 根据调用方推导，App 不能自报。MessageHub 按分类展示 | 用户答复 |
| 宿主 | **msg-center**（用户决定）新增 kRPC `notify.*`：分配 `notify_id`、写主记录、按等级另投、需要时建 Task、写观察记录、过期清扫、清单查询 | 渠道（MessageHub 原生、Telegram）、联系人、按用户隔离和可靠投递都已在 msg-center |
| 等待 | 只有 `input` 需要等待。等待者读 TaskMgr（`get_task`；kevent `/task_mgr/<id>` 只用来加速）；buckyos-api 提供带超时的等待辅助函数。这是逻辑等待，不占进程 | 纪要 §5.3；`runtime.rs:2146` 已有无超时的 `wait_for_task_end_kevent` |
| 通知中心与处理入口 | **MessageHub**（用户决定）：一个“系统通知”实体，下设三个分类会话（待办 / Agent / 应用），每个 Notify 是其中一张卡片，按 prompt 原地处理，状态实时。统一 URL `/messagehub?notify=<id>` 打开它的卡片。分类会话不是对话，不能打字。TaskCenter 不当通知中心 | 纪要 §6.1、§6.6；用户答复 |
| 触达 | 分类会话里总有一条主记录。来自 Agent 的通知再按等级另投到“用户与该 Agent 已有的会话”（原生 / Telegram）：`quiet` 不另投，`normal` 投来源会话，`timely` 再加最近活跃的会话，`urgent` 投全部。未读只落在用户最可能看的那一处。非 Agent 来源第一段只有主记录 | 纪要 §6.3—§6.4；现在只有这两种渠道，Telegram 必须由 Agent 的 bot 发 |
| Work 的 ask-user | LLM 仍调用同一个 `agent-session ask-user`。在 Work 中由 runner 改走 Notify（`input`），run 照旧挂在 `userinput:<call_id>` 上。等待**有界**：缺省 **3 小时**（用户决定），可由 `[session.work] ask_user_timeout_secs` 配置。超时、被忽略、被撤销都作为工具结果返回。提示词仍倾向快速失败 | 纪要 §7.3—§7.5、§10.1；走哪条路由 Session 决定，不让 LLM 选 |
| Goal 的提问 | Activation 结果中的 `questions` 由驱动者转成 `input` Notify，不阻塞。回答作为下一次 Activation 的原因 `answer` 投回。Goal 内调用 `ask-user` 仍然拒绝，只是提示改为“写进 questions” | 纪要 §10.2；补上 Goal TODO §8.3 中“questions 只能经 kRPC 看到”的缺口 |
| 主动告知 | `agent-session notify send`（`notice`），UI / Work / Goal Session 都可以用。智能提醒的最后一步就是它 | 纪要 §十一 |
| UI 观察 | Agent 来源的 Notify 另投进某个 Agent 会话时，msg-center 在该 Agent 对应会话的 INBOX 写一条观察记录（`kind = event`，intent `buckyos.notify.delivered`）。Loader 把它转成 `_notify` 半订阅事件，随下一批输入出现在 `<semi_subscription_snapshot>` 中。它既不是 user message，也不是 Assistant Message | 纪要 §12.1—§12.2；半订阅机制已有（`lib_opendan/src/runner/inputs.rs:330-365`），只是没有生产者 |
| UI 代理 | `agent-session notify answer / ack`：用户明确回应某条未结束的 Notify 时，UI Session 代为处理（`input` 提交到原 Task，`notice` 标为已读）。`approval` 一律拒绝；不能处理本 Session 自己发起的 Notify；代答留痕（用户确认 Q9） | 纪要 §12.4—§12.5 |
| workflow `remind` | 暂不处理（用户决定：workflow 还没正式用起来）；`todo` 分类的接口位置先留出来 | — |
| llm_context / agent_tool | 第一段不改。第二段的机械审批需要改 `PolicyEngine`（`llm_context/src/deps.rs:237-255`），按约定先单列 TODO | [`feedback-llm-context-first`] |
| 协议版本 | `buckyos.notify` 的数据带 `version = 1`；OpenDAN 协议只增加字段，结构有变化时重新生成 fixtures | `doc/opendan/protocol/README.md` |

---

## 1. 第一段范围

### 1.1 做

- **buckyos-api**：
  - `notify.*` 的请求与视图类型，以及 `MsgCenterClient` 方法；
  - 带超时的等待辅助函数、回答辅助函数（对 TaskMgr 提交）；
  - `human.input/v1` 的 notify input / result 约定；
  - App 主体调用 `notify.*` 的 RBAC。
- **msg-center**：
  - `notify.send / get / list / ack / dismiss / cancel`；`notifies` 表；
  - 系统通知发送者与三个分类会话、主记录；Agent 来源按等级另投；消息组成；
  - 观察记录；过期清扫；
  - `kind = notify` 计入会话活跃。
- **buckyos-websdk（同级仓）**：`notify.*` 的 TS 封装。App 侧要用这个能力，SDK 上就得有。
- **libopendan**：
  - `NotifyHost` 宿主接口，含本地文件实现；`AgentStateClient::notifies()` 与 kRPC 透传；
  - 来源计算；Work 的 ask-user 改走 Notify；Goal questions 与 Notify 互转；
  - ui 模板的 `_notify` 隐式半订阅；按截止时间与 Notify 变化唤醒 Session；Turn task 的等待原因；
  - `agent-session notify …` 与 `xagent notify …`。
- **OpenDAN Loader**：
  - `NotifyHost` 的 zone 实现；
  - 把 INBOX 观察记录转成 `_notify` 事件；kevent 加速；
  - Jarvis 提示词。
- **Desktop**：
  - MessageHub：“系统通知”实体与三个分类会话、通知卡片、`?notify=` 深链；
  - TaskCenter：只做兼容。
- **文档回写**（N10），包括面向 App 开发者的接口说明。

### 1.2 第一段不做

| 事项 | 去向 |
|---|---|
| `post-message`、收据、回复关联、共享邮箱的回复路由 | §9.1 |
| 机械审批：执行层授权、审批 Notify 的生产者、当前 UI 被阻塞时的模态遮罩、审批只能由用户本人提交的硬约束 | §9.2（先单列 llm_context TODO） |
| 分类展示的优化与用户配置、用户的通知设置、等级的回退与升级、短信 / 邮件 / App 推送渠道、非 Agent 来源的外部渠道、桌面状态栏托盘 | §9.3 |
| 把 Notify 状态变化（已回答 / 已过期）推给 UI Session 的观察事件 | §9.4 |
| 一次性定时（智能提醒的“提前唤醒”） | Goal TODO §9.1、§9.5 follow_up |
| 纪要 §三：处理中补充输入的归属与占位排序 | §9.5，另立 TODO |
| `todo` 分类的生产者（workflow 指派）；workflow `remind` 改走 Notify；`dcrontab` 的 PATH 与 owner 问题 | 暂不处理（用户决定，workflow 尚未正式使用） |
| TaskCenter 作为通知中心、详情页按 prompt 交互 | 不做（用户决定改在 MessageHub） |
| 会签、多级审批 | 不做（TaskMgr §10.2 要求由 Workflow 聚合） |

---

## 2. 当前基础与缺口

| 入口 | 已有基础 | 缺口 |
|---|---|---|
| ndn-lib `msgobj.rs`（`MsgObjKind::Notify`）；`frame/msg_center/src/msg_box_db.rs:1955-1974` | `Notify` 这个 kind 已经存在；msg-center 只把它映射成字符串 | 没有结构约定，也没有处理逻辑 |
| `kernel/buckyos-api/src/task_mgr.rs:552-570`（`TaskExecutor::HumanSet`）；`kernel/task_manager/src/server.rs:1018-1049, 1778-1792` | <ul><li>HumanSet 任务创建后即为 `Waiting(HumanInput)`。</li><li>assignee 用 `commit_result` + `expected_revision` 提交，先到先得；输家得到 `task_already_completed`。</li><li>每次变更发布 kevent `/task_mgr/<id>`，`list_task_events` 可补漏。</li><li>`human.input/v1`（:1005，宽松 schema）；`human.approval/v1`（:992，严格 schema）。</li></ul> | <ul><li>生产代码中没有 HumanSet 的创建者。</li><li>不支持过期。</li><li>assignee 没有 ReadResult（`task_manager/src/acl.rs:253-308`）。</li><li>commit 不检查 App。</li></ul> |
| `kernel/buckyos-api/src/taskdata.rs:20-200` | `HumanTaskPrompt{title, message, controls: Choice / TextInput}`、`TaskDataResult::Human{action, values, submitted_by, submitted_at}` | 无人使用 |
| `kernel/buckyos-api/src/runtime.rs:2146-2196` | `wait_for_task_end_kevent`：订阅加重读，500 ms 后每 30 s 兜底扫描 | 没有超时；无人调用 |
| `kernel/buckyos-api/src/rbac_config.rs:166-183` | 用户和 admin 可以读写自己的邮箱；`agent`、`agent_runtime`、`system:opendan`、`system:msg-center` 有 `obj://msg-center/*` | 普通 App 主体没有 msg-center 权限 |
| `frame/msg_center/src/msg_center.rs:1601-1830`（`post_send`）、`:1028-1118`（路由）、`:1002-1026`（Inbox / RequestBox 分级）；`main.rs:293-458`（投递泵）、`:843-922`（zone 用户与 Agent 的联系人种子）；`owner_session.rs:533-582`（`msg.create_session`） | <ul><li>可靠投递，支持幂等键。</li><li>路由只看目标 DID 的形式。</li><li>Telegram 发送要求 `from` 有 bot 绑定。</li><li>陌生发送者的消息进 RequestBox。</li><li>可以登记带 peer、标题的会话。</li></ul> | <ul><li>没有 notify RPC、系统通知发送者、分类会话、用户渠道偏好。</li><li>没有邮件、短信 tunnel。</li></ul> |
| `frame/msg_center/src/owner_session.rs:165-170`；desktop `app/messagehub/sessionModel.ts:17-20`、`protocol/msgobj.ts:421-426` | Notify 不计入会话活跃；MessageHub 把它渲染成状态胶囊 | 缺通知卡片投影（`UI_DATAMODEL.md:663, 700-701` 已记录这个缺口） |
| desktop `app/messagehub/MessageHubRoute.tsx:8-32`、`launch.ts:4-37`（`messageHubMessagePath`） | 深链 `entityId / sessionId / messageId`；按实体 → 会话展示；未读来自 `SessionSummary.unread_count` | 没有 `notify` 深链参数 |
| `frame/opendan/src/ui.rs:430-512, 594-715`；`lib_opendan/src/bridge/msg.rs:85-195` | Agent INBOX 的每个会话对应一个 UI Session；只放行主人发来的消息；用 kevent 唤醒 | 没有把“系统刚给用户看了什么”交给 UI Session 的路径 |
| `lib_opendan/src/runner/inputs.rs:330-365`；`protocol/config.rs:727-800`；`runner/drive.rs:1890-1903`；`runner/assembler.rs:166-185` | 半订阅事件进入 `pending_events`，随下一批输入渲染；已有隐式订阅样板 `_user_timezone`、`_goal_activation` | 没有 `system:notify` 来源和事件生产者 |
| `lib_opendan/src/runner/ask.rs:27-35, 83-114`；`template.rs:97-104, 147-165` | UI Session 的 ask-user 已实施 | Work 和 Goal 直接报错 “this session cannot ask the user …” |
| `runner/live.rs:638-691, 852-994`；`runner/drive.rs:1062-1133`；`host/mod.rs`（Supervisor） | `userinput:` 不经过 resolver；挂起状态可跨重启；有队列的 Session 卸载后每 30 s recheck | 无队列的 Work 卸载后不会再自动装载；也没有按截止时间唤醒 |
| `runner/turn_task.rs:167-196`；`opendan/src/tasks.rs:288-301` | Input 映射为 `Other("input")` | 经 Notify 等待时应改为 `HumanInput` + `related_task_id` |
| `lib_opendan/src/protocol/goal.rs:530-535, 595-597`；`runner/goal.rs:867-893`；Jarvis `behaviors/goal.toml:20` | Goal `questions[]` 只记录，最多 16 条 | 没有投递，回答也不能回流 |
| `lib_opendan/src/bin/xagent.rs:35-93`；`llm_context/src/deps.rs:329-336` | 没有 notify / send-msg 子命令；`<sendmsg>` 动作是空实现 | 缺 `agent-session notify …` |
| `llm_context/src/deps.rs:237-255`、`request.rs:217-222` | `PolicyEngine` 只能整批拒绝；`HumanPolicy.approval_required` 从未被读取 | 机械审批（第二段） |
| desktop `api/task_mgr.ts:551-622, 954-980`；`app/task-center/pages/HomePage.tsx` | TaskCenter 首页把每个未结束的 HumanSet 任务派生成一张 approve / reject 卡片 | Notify 的 Task 也会被派生成这种卡片，但提交结构不对（§3.4 末条） |
| `lib_opendan/src/bin/xagent.rs:433-437, 723-735` | `agent-session post <sid> --msg [--from <DID>]` 可以向任意有队列的 Session 投消息，`--from` 可以随便填 | 与纪要 §8.2 冲突（见 §8.4） |

---

## 3. 模型与契约

### 3.0 App 接口（第一段要稳定的部分）

以下是对 App 的承诺，写进面向开发者的文档（N10）。它们之后只能增加字段，不能改变已有语义：

| 项 | 契约 |
|---|---|
| 方法 | 发送方：`notify.send / get / list / cancel`。用户侧：`notify.ack / dismiss`（告知），回答走 TaskMgr `commit_result`（由 SDK 的 `answer_notify` 封装） |
| 发送请求 | `{to_user, kind: notice | input, level?, prompt: HumanTaskPrompt, source?: {session_id?, goal_id?, operation?, link?}, expires_at_ms?, idempotency_key}`。`source.app_id`、`source.agent_did` 由 msg-center 从调用方身份填写，不由调用方声称 |
| 返回 / 视图 | `NotifyView{notify_id, url, kind, category, level, status, prompt, source, created_at_ms, expires_at_ms, task_id?, result?, home, deliveries}` |
| 状态 | `open / read / answered / dismissed / expired / canceled`（§3.1） |
| 分类 | `todo / agent / app`，由 msg-center 推导：调用方是 Agent App 时为 `agent`；系统服务可以显式指定 `todo`；其余为 `app` |
| 等级 | `quiet / normal / timely / urgent`，缺省 `normal`。语义是“期望多强地触达”。具体投到哪里由系统决定，可以随版本变化 |
| 幂等 | 同一调用方、同一个 `idempotency_key` 返回同一个 Notify，不重复投递 |
| 权限 | App 主体 `app:<app>@<user>` 只能通知 `<user>`；系统服务可以通知任何用户；读取与撤销只限请求方、目标用户和系统服务 |
| 等待 | `input` 的结果在 Task 上。SDK 提供 `wait_notify(notify_id, deadline)` |

不承诺的部分：投递到哪些渠道、MessageHub 怎么分组与排序、卡片长什么样。

### 3.1 Notify 的数据：消息与 Task

**主记录**（写进目标用户 INBOX 的分类会话）：

```json
{
  "from": "<系统通知发送者 DID>",
  "to": ["<用户 DID>"], "to_session": "sysnotify:<category>",
  "kind": "notify",
  "expires_at_ms": null,
  "content": {
    "title": "…", "format": "text/plain",
    "content": "说明；input 时附选项列表；最后一行“处理：<url>”",
    "machine": { "intent": "buckyos.notify", "data": {
      "version": 1, "notify_id": "nt-…", "kind": "notice | input",
      "category": "todo | agent | app", "level": "normal", "home": true,
      "source": {
        "app_id": "…", "agent_did": "… | null",
        "session_id": "… | null", "turn": 3, "call_id": "… | null",
        "goal_id": "… | null", "operation": "一句话：这是为了什么",
        "link": "App 自己的相关页面（可空）",
        "conversation": { "to": "<用户端点 DID>", "to_session": "…" }
      },
      "prompt": { "title": "…", "message": "…", "controls": [] },
      "task_id": "… | null",
      "url": "https://<zone host>/messagehub?notify=<notify_id>"
    } }
  }
}
```

- `prompt` 的写法：
  - `notice` 的 `controls` 为空；
  - `input` 有选项时加 `Choice{id: "option", options, required: true}`；
  - 没有选项或 `allow_text` 时加 `TextInput{id: "text"}`。
- 另投给 Agent 会话的副本与主记录内容相同，只有三处不同：`from = agent_did`，`to` 与 `to_session` 是那个会话的，`home = false`。

**Task**（只有 `input` 才建）：

```text
create_task {
  name: "notify: <title 截断>",
  schema_id: "human.input/v1",
  executor: HumanSet { assignees: [<目标 user_id>] },
  storage_domain: User,
  idempotency_key: "notify:<notify_id>",
  input: { "notify": {notify_id, kind, category, level, source, expires_at_ms}, "prompt": HumanTaskPrompt }
}
→ grant_task_access:
    User{目标用户}             → ReadResult（assignee 默认没有）
    Principal{请求方 user, app} → ReadMeta / ReadInput / ReadResult / Control
```

result 的结构：`{"kind": "human", "action": "answer | dismiss", "values": {"option": "…", "text": "…"}, "submitted_by": "<user>", "submitted_at": 0}`。实际提交的 App 由 TaskMgr 写入 `completed_by`，代答所在的 Session 与触发消息记入 task note。

**状态**（`notify.get / list` 给出）：

| 状态 | `notice`（真相：msg-center 记录） | `input`（真相：Task） |
|---|---|---|
| `open` | 主记录与副本都未读 | Task `Waiting` |
| `read` | 任一记录已读，或调用了 `notify.ack` | —（`input` 看的是有没有回答，不看读没读） |
| `answered` | — | `Succeeded`，`action = answer` |
| `dismissed` | `notify.dismiss`（各记录归档） | `Succeeded`，`action = dismiss` |
| `expired` | 过了 `expires_at_ms`（各记录归档，记 `closed_reason`） | msg-center 以 `expired` 撤销了 Task |
| `canceled` | 请求方撤销 | 以其它原因撤销了 Task（`ask_timeout`、`turn_stopped`、`goal_dropped` 等） |

### 3.2 msg-center `notify.*`

**`notify.send`** 的处理步骤：

1. **权限与来源**：
   - 按 §3.0 检查调用方；
   - 填写 `source.app_id`；调用方是 Agent App（AppId = AgentId，msg-center 的 zone Agent 种子里已有这份名单）时填 `agent_did`；
   - 推导 `category`；只有系统服务可以指定 `todo`。
2. **幂等与分配 id**：按 `(请求方 principal, idempotency_key)` 查 `notifies` 表；已存在就从断点继续，否则分配 `notify_id` 并写行，状态为 `delivering`。
3. **建 Task**：只有 `input` 才建（§3.1），并授权，`task_id` 写回行。
4. **主记录**：确保目标用户的分类会话已登记（§3.3）。主记录直接写入用户的 INBOX，不走联系人分级（否则系统发送者会被当成陌生人放进 RequestBox）。按 §3.3 的规则决定已读或未读。幂等键 `notify:<notify_id>:home`。
5. **另投**：只对 Agent 来源。按等级选会话，逐个 `post_send`，幂等键 `notify:<notify_id>:<session_id>`；结果记入 `deliveries`。
6. **观察记录**：每个另投的 Agent 会话写一条（§3.9），幂等键 `notify-obs:<notify_id>:<session_id>`。
7. **完成**：行改为 `delivered`；返回 `NotifyView`。

中途崩溃时，同键重调会从断点继续；清扫器也会补完仍为 `delivering` 的行。

**其它方法**：

- **`notify.get{notify_id}`**：返回 `NotifyView`。
- **`notify.list{status?, kind?, category?, agent_did?, app_id?, session_id?, goal_id?, since_ms?, cursor, limit}`**：只返回调用方所属用户的 Notify（Zone 服务按用户隔离）。`input` 的状态逐条查 `get_task` 合并。
- **`notify.ack{notify_id}`**（`notice`）：把该 Notify 的全部记录标为已读。
- **`notify.dismiss{notify_id}`**（`notice`）：把全部记录归档。
- **`notify.cancel{notify_id, reason}`**：请求方或系统服务可调用。`input` 撤销 Task（msg-center 是 root creator，HumanSet 任务直接关闭为 Canceled）；`notice` 归档记录。原因记入 `closed_reason`。

**`notifies` 表**：`notify_id`、`owner_user`、`requester`、`category`、`kind`、`level`、`source`（JSON）、`title`、`task_id`、`created_at_ms`、`expires_at_ms`、`state`（`delivering | delivered | closed`）、`closed_reason`、`closed_at_ms`、`home`（JSON：会话、记录 id）、`deliveries`（JSON）。对 `notice` 来说，状态的真相是这些记录的状态；对 `input` 来说是 Task。

**清扫**（30 s 一次，与 settings 重读同一类循环）：

- 补完 `delivering` 的行；
- 过期的 `notice` 把记录归档；过期且未结束的 `input` 撤销 Task；两者都记 `expired`；
- 把已结束的行标为 `closed`。

### 3.3 分类会话、另投与消息形态

**系统通知发送者与分类会话**：

- 主记录统一由**系统通知发送者**发出，用 Zone DID（用户决定）。实施时核对 MessageHub 对它的展示，由 presentation 层显示为“系统通知”。真实来源放在 `machine.data.source`，显示在卡片上。
- 每个用户三个会话：`sysnotify:todo`、`sysnotify:agent`、`sysnotify:app`。peer 为系统通知发送者，标题分别是“待办”“Agent”“应用通知”。第一次需要时由 msg-center 用 `msg.create_session` 那套登记建好。
- 这三个会话不是对话：MessageHub 里不提供输入框。

**候选会话**（只对 Agent 来源）：目标用户在该 Agent INBOX 中说过话的会话，与 Loader 路由到 UI Session 的是同一批。每个候选记 `{session_id, platform, to, to_session, last_active_at}`：

- `to`、`to_session` 按 `ReplyRoute::of_msg`（`lib_opendan/src/protocol/input.rs:860-874`）的同一规则，从用户的最近一条消息推出；
- `last_active_at` 取该消息的时间。

只考虑私聊与 Telegram 私聊，不投进群。

| level | 主记录（分类会话） | 另投到（只对 Agent 来源） |
|---|---|---|
| `quiet` | 写入即标为已读 | 不另投 |
| `normal`（缺省） | 有另投时标为已读，否则未读 | `source.conversation`（仍在候选中时） |
| `timely` | 同上 | `normal` 的另投，再加最近活跃的会话（两者不同则两处） |
| `urgent` | 未读 | 全部候选会话 |

未读只落在用户最可能看的那一处，其余记录只是同一个 Notify 的入口。非 Agent 来源只有主记录（`quiet` 时为已读）。不预设“IM 一定比网页强”的排序（纪要 §6.4）。

**会话活跃**：`kind = notify` 计入会话活跃并解除归档（`owner_session.rs:165-170`）。

**Telegram**：副本只发纯文本加链接。链接在手机浏览器中打开后经 SSO 登录，进入 MessageHub 的卡片。

### 3.4 MessageHub：系统通知实体、分类会话与通知卡片

- **系统通知实体**：显示为“系统通知”，下设三个会话，按重要程度排列：待办、Agent、应用通知。没有内容的会话可以隐藏。会话的未读数就是待处理提示。会话内可筛“全部 / 待处理 / 已处理”（按卡片状态，在客户端过滤）。不提供输入框。
- **通知卡片 `NotifyCard`**：凡是 intent 为 `buckyos.notify` 的 `kind = notify` 消息（分类会话里的主记录和 Agent 会话里的副本）都渲染成卡片：
  - 显示标题、说明、真实来源（Agent / App 名，Goal 名或 Work 目标，`source.link`）、等级、状态徽标；
  - 状态读 `notify.get`；`input` 的 kevent `/task_mgr/<task_id>` 提示重读。同一个 Notify 的所有卡片状态一致（纪要 §6.1）；
  - 未结束时原地处理：
    - `input`：`Choice` 渲染为按钮或单选，`TextInput` 渲染为输入框加提交，另有“忽略”。提交时先读最新快照，再 `commit_result` 并带 `expected_revision`；冲突时重读，显示最终结果；
    - `notice`：“知道了”调用 `notify.ack`，“忽略”调用 `notify.dismiss`；
  - 结束后显示结果、提交者、时间；
  - **不改变所在会话输入框的状态**：Work 被阻塞不等于这个 UI 会话被阻塞（纪要 §6.5）。
- **深链**：`/messagehub?notify=<id>` 先调 `notify.get` 取 `home`，再复用现有的 `entityId / sessionId / messageId` 深链打开卡片。手机上从 Telegram 链接进入同样可用。
- **会话模型**：`kind = notify` 计入活跃（`sessionModel.ts:17-20`）；`protocol/msgobj.ts` 增加 `buckyos.notify` 的类型；在 `UI_DATAMODEL.md` 补上 notify 投影与系统通知实体。
- 三类的展示方式、排序和用户可配置，都放到以后优化（§9.3）。第一段只要能按分类看到、能处理。
- **TaskCenter 只做兼容**：
  - 带 `input.notify` 的 Task 不进入首页的“System Notifications”卡片；
  - 任务列表和详情给出“在 MessageHub 中处理”的链接（`?notify=`）；
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
    async fn answer(&self, notify_id: &str, a: NotifyAnswer, via: NotifyVia) -> Result<NotifyState, NotifyError>; // input 提交 / notice ack
}
```

**实现**：

- `opendan::notify::ZoneNotify`：调用 msg-center `notify.*` 与 buckyos-api 的 `answer_notify`；代答写 `add_task_note`。
- `libopendan::notify::LocalNotify`：文件实现 `<agent_root>/state/notifies/<id>.json`，供 `--dev`、xagent 和测试使用；`xagent notify answer` 扮演用户。

**CLI 的通路**：照 `goals.*` 的样板，`AgentStateClient::notifies()` 的文件版是 `LocalNotify`，kRPC 版由 Loader 透传给 `ZoneNotify`。

**来源计算**（`notify_source(session)`）：

- 基本字段：`session_id`、`turn`、`call_id`（CLI 取 `$XLLM_CALL_ID`）、`goal_id`（取自 `session.goal` 或 `origin.goal`）。`app_id` 与 `agent_did` 由 msg-center 填写。
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

   拿到的 `notify_id` 与 `task_id` 记入 `asks[call_id]`，然后与 `waiting_for = {kind: Input, refs: ["userinput:<call_id>"], deadline_ms}` 一起提交。顺序是先发后提交；崩溃后重做会拿到同一个 Notify。
4. **Turn task**：等待原因为 `HumanInput`，code `ask_user`，`related_task_id = task_id`，文案为 “waiting for the user's answer (notification)”。
5. **回填**：每次 drive 对经 Notify 等待的调用查询 `state`：
   - `open` 且未到截止时间：继续等。
   - `answered`：回填 `UserInputAnswer{text, option, in_reply_to_question: true, msg_id: null, from: submitted_by, at_ms: submitted_at}`，另加 `via: "notification"`。其中 `text` 取 `values.text`，没有时取选项 label。
   - `dismissed / expired / canceled`，或已到截止时间（此时先 `cancel(id, "ask_timeout")`）：回填 `{"answered": false, "reason": "timeout | dismissed | canceled", "note": "decide with what you have, or end with a failure report that says what is missing"}`。

   回填与提交沿用 `fill_answer` / `commit_answer` 的路径，但没有输入消息可确认，receipt 的 `inputs` 为空。
6. **唤醒**：Session 被卸载后，以下情况让它重新装载：
   - Supervisor 对 `waiting_for.kind = Input` 且无队列的 Session，按 `min(deadline_ms, 30 s recheck)` 重新装载；
   - Loader 订阅 `/task_mgr/<task_id>` 后调用 `ensure_task(sid)` 加速。
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

- **`send`**：打印 `NotifyView` 的要点 `{notify_id, url, status, category, deliveries}`；缺省 key 为 `<sid>:<turn>:notify:<call_id>`。
- **`list`**：缺省列出本用户的、来源是本 Agent 的、未结束的 Notify。
- **`answer`**（`input`）/ **`ack`**（`notice`）：
  - `approval` 返回 “approvals are decided by the user: <url>”；
  - 以下情况被拒，并输出当前状态：来源 Session 就是调用方自己；Notify 已结束；选项不存在。
- **`cancel`**：只能撤销来源是本 Agent 的 Notify。
- **`xagent notify list|show|answer|ack|cancel`**：主人一侧的 CLI，`--dev` 与测试用它扮演用户。

### 3.9 UI 观察：观察记录与 `_notify`

**观察记录**：msg-center 把 Agent A 来源的 Notify 另投进 A 的会话 S 后，进程内写一条记录到 A 的 INBOX `<A>/<S>`。分类会话里的主记录不写观察记录，因为那里不是对话。

```json
{ "from": "<A>", "to": ["<A>"], "to_session": "<S>", "kind": "event",
  "content": { "machine": { "intent": "buckyos.notify.delivered", "data": {
    "notify_id": "…", "url": "…", "kind": "input", "category": "agent", "level": "timely",
    "title": "…", "message": "…", "options": [ … ],
    "source": { "session_id": "…", "goal_id": "…", "operation": "…" },
    "delivered_msg_id": "…",
    "reply_route": { "to": "<用户端点>", "to_session": "<S>" },
    "at_ms": 0 } } } }
```

**Loader 路由**：在 `bridge/msg.rs::route_msg_record` 的主人过滤之前加一条规则：`from` 为 Agent 自己且 intent 为 `buckyos.notify.delivered` 的记录，转成 `AgentEvent{source: system:notify, event: delivered, key: notify:<id>:<S>, data}`；其它发给自己的记录照旧丢弃。

**Session 的建立**：S 没有 UI Session 时，`session_for` 照常新建一代，`OutboundBinding` 取自 `reply_route`。观察记录不会以别的方式决定回复坐标。

**订阅**：ui 模板（`group` 经 `base` 继承）加隐式半订阅 `_notify`，订阅 `System{id: "notify"}`，写法照 `_user_timezone`（`protocol/config.rs:785-800`）。

**半订阅的效果**：

- 事件不唤醒 Session，只进 `pending_events`；
- 用户下一次发消息时，随该批输入出现在 `<semi_subscription_snapshot>` 中，受 `change_budget` 限制。

**缺省渲染**（assembler builtin），每条一行：

```text
[notify <id>] <kind>/<level> from <来源：goal 名 / session> at <时间>: <title> — <message 摘要> (options: …) — check the current state with `agent-session notify show <id>`
```

第一段只有“已投递”事件。状态由 Agent 按需查询，因为“已在界面呈现”不等于“已读”（纪要 §12.2）。

### 3.10 UI 代理（用户已确认）

写进 `chat_route` / `groupchat_route` 的提示：

1. `[notify …]` 行表示用户在这个会话里看到过这条通知。
2. 用户的消息明确回应某条未结束的通知时（内容对得上，或直接回复了那条消息），先用 `notify show` 查当前状态，再 `notify answer` 或 `ack`，然后简短告诉用户。
3. 有多条未结束的通知、回复又含糊（如“好的”）时，问是哪一条，不要猜（纪要 §12.5 第一条）。
4. 改变安排的反馈不是回答，例如“今天在度假，别打扰我”：
   - 有 Goal 来源时用 `agent-session goal post <goal_id> --msg …` 转给 Goal；
   - 否则按普通请求处理；
   - 不要把一次性的话扩大成长期规则（纪要 §12.6）。
5. 不处理审批，引导用户打开链接。

宿主侧的检查见 §3.8。`input` 代答写 task note：`answered by <agent> in session <sid> after input <key>`；`notice` 代确认记在 `notifies` 行的 `closed_reason`（`ack_by_agent`）。

---

## 4. 第一段实施 TODO

### N1：buckyos-api

- [x] `msg_center_client.rs`：
  - 类型：`NotifyKind / NotifyCategory / NotifyLevel / NotifyStatus / NotifySource / NotifySendReq / NotifyView / NotifyListReq`；
  - 客户端方法：`notify_send / get / list / ack / dismiss / cancel`；方法常量。
- [x] `buckyos.notify` 的数据结构（§3.1），复用 `HumanTaskPrompt`、`TaskDataResult::Human`；`human.input/v1` 的 notify envelope 约定。
- [x] 辅助函数：
  - `wait_notify(notify_id, deadline)`：基于 `wait_for_task_end_kevent`，并补上超时；
  - `answer_notify(notify_id, answer)`：先 `get` 拿到 `task_id`，再读最新快照并带 CAS 提交。
- [x] `rbac_config.rs`：App 主体对 `obj://msg-center/notify` 的写权限。按现有规则中 App 主体的角色名写；`agent` 已有 `obj://msg-center/*`。
- [x] 核对 `TaskGrantSubject::Principal` 在 ACL 中的匹配（`acl.rs:238-243`），以及 assignee 加上 User grant 后能读 result。
- [x] 测试：数据结构往返；`wait_notify` 的超时；`answer_notify` 的并发冲突。

### N2：msg-center `notify.*`

- [x] kRPC 分发与权限、来源填写、分类推导（§3.0、§3.2 第 1 步）；`notifies` 表与迁移。
- [x] `send`：
  - 幂等与断点续投；`input` 建 Task 与授权；
  - 系统通知发送者、分类会话的登记、主记录直接写 INBOX，以及已读规则（§3.3）；
  - Agent 来源的候选会话与等级规则；消息组成与 `post_send`；
  - 观察记录。
- [x] `get / list / ack / dismiss / cancel`；清扫器（补投、过期、关闭）。
- [x] `kind = notify` 计入会话活跃（`owner_session.rs:165-170`）。
- [x] 测试（`test_msg_center.rs`）：
  - 来自普通 App 的 `notice`：只有主记录，进入 `sysnotify:app`，不进 RequestBox；
  - App 指定 `todo` 被拒；Agent 来源进入 `sysnotify:agent`；
  - 四个等级下主记录的已读状态与另投位置；没有候选；
  - `input` 建出 Task，`get` 合并状态；
  - 幂等重放不重复发；App 不能通知别的用户；`list` 按用户隔离；
  - `ack / dismiss`；过期与撤销；
  - Telegram（DryRun 网关）收到带链接的文本；
  - 观察记录写在 Agent 的 INBOX，`to_session` 正确。

### N3：libopendan 协议与宿主接口

- [x] `protocol/notify.rs`：`NotifyRequest / NotifyState / NotifyAnswer / NotifyVia`；schema 导出。
- [x] `notify.rs`：`NotifyHost`；`LocalNotify`；`notify_source`。
- [x] `state/`：`AgentStateClient::notifies()`；kRPC `notify.*` 透传（按调用方 Session 记录来源）。
- [x] `template.rs` / `protocol/config.rs`：ui 模板的隐式半订阅 `_notify`；assembler builtin 渲染 `system:notify` 事件。
- [x] `protocol/goal.rs`：`GoalQuestion` 的 `notify_id / status / answer`；Activation 原因 `answer`。
- [x] `[session.work] ask_user_timeout_secs`（缺省 10800，0 关闭）。
- [x] 协议文档与 fixtures（新增一个 `17_notify`：Work 经 Notify 问答、Goal 问题回流、`_notify` 事件）。

### N4：libopendan runner

- [x] `runner/ask.rs`：`via_notify` 分支、发出、截止时间、回填（§3.6）；Goal 报错文本。
- [x] `runner/drive.rs` / `live.rs`：经 Notify 等待的调用查询与回填；stop 时撤销；`commit_answer` 处理无输入的 receipt。
- [x] `runner/turn_task.rs`：`HumanInput` + `related_task_id`。
- [x] `runner/goal.rs`：问题发出与撤销（close 前）；`answer` 事件的消费与状态更新。
- [x] `host/`：Supervisor 对无队列、等输入的 Session 按截止时间与 recheck 重新装载；GoalHost 监视问题 Notify 并投递 `answer`。
- [x] 测试（`tests/ask_user.rs` 扩展、`tests/goal_session.rs`、`tests/crash.rs`）：§5 的 V07—V15。

### N5：xagent CLI

- [x] `agent-session notify send|list|show|answer|ack|cancel`（§3.8）；`xagent notify …`（主人一侧）。
- [x] USAGE、退出码（越权为 6，状态冲突为 5，与现有约定一致）。
- [x] `tests/xagent_cli.rs`：真实二进制 + `LocalNotify` 的往返。

### N6：OpenDAN Loader

- [x] `notify.rs`：`ZoneNotify`；kRPC 透传；监视器（kevent `/task_mgr/<id>`，并以 30 s 扫描兜底），驱动 `ensure_task` 与 Goal 的 `answer`。
- [x] `ui.rs` + `bridge/msg.rs`：观察记录路由；`session_for` 从 `reply_route` 取 binding。
- [x] `tasks.rs`：wait reason 映射（:288-301）。
- [x] `tests/loader.rs` 端到端（脚本化 LLM + 进程内假 msg-center / TaskMgr，或 `LocalNotify`）：Work 提问 → 回答 → 继续；观察事件进入下一批输入。

### N7：Desktop

- [x] MessageHub（§3.4）：
  - 系统通知实体与三个分类会话（不提供输入框、可筛选）；
  - `NotifyCard`：按 prompt 处理 `input`，`ack / dismiss` 处理 `notice`，实时状态，结束后的结果；
  - `?notify=` 深链；`sessionModel.ts` 活跃规则；`protocol/msgobj.ts` 的类型；
  - TS 客户端：`notify.*`、TaskMgr `get_task / commit_result`。
- [x] mock 场景：三个分类各有数据；Work 提问（Agent 会话里的副本 + 分类会话里的主记录）；Goal 提问；提醒；应用通知；已处理的历史。
- [x] TaskCenter 兼容：带 `input.notify` 的 Task 不进首页卡片；列表与详情链接到 `?notify=`。
- [x] i18n（中英）；`UI_DATAMODEL.md`。
- [x] 测试：
  - datamodel：notify 投影，同一 Notify 的多张卡片状态合并；
  - Playwright：卡片处理 choice / text / 知道了 / 忽略；两张卡片并发提交，一张成功、另一张显示最终结果；分类会话与筛选；深链；Agent 会话的输入框不被锁定；TaskCenter 首页不出现 Notify 的 Task。

### N8：Jarvis 包

- [x] `plan.toml` / `do.toml` / `work_simple.toml`：ask-user 兜底说明（§3.6 第 9 条）。
- [x] `goal.toml`：questions 改写（§3.7）；可以用 `notify send` 告知。
- [x] `chat_route.toml` / `groupchat_route.toml`：§3.10 的观察与代理规则。
- [x] `agent.toml`：`[session.work] ask_user_timeout_secs = 10800`。

### N9：（取消）workflow `remind` —— 用户决定暂不处理

### N10：文档

- [x] **面向 App 开发者的 System Notify 接口说明**：§3.0 的契约，加上 Rust / TS 用法示例（发告知、发提问并等待回答、撤销）。放在 `doc/message_hub/` 下，并在 `doc/sdk/` 加入口。
- [x] `Message Center.md`：`notify.*`、`buckyos.notify` 数据、系统通知发送者与分类会话、另投与已读规则、观察记录、`kind = notify` 计入活跃。
- [x] `UI_DATAMODEL.md`：系统通知实体、通知卡片、`?notify=` 深链。
- [x] `task data schema.md`：`human.input/v1` 的 notify envelope。
- [x] 架构设计：
  - §5.6 的“Work 与 Goal 报错”改为 §3.6 / §3.7 的规则；
  - §12.11 / §12.12.2 补上“Work 的有界 Notify 等待（缺省 3 小时）”及其与“长时间 PendingTool 等审批”的区别；
  - §15.5 增加一行 System Notify；
  - §17.2 标出已采用的部分。
- [x] `xAgent.md`（CLI、模板表、`_notify`）；协议文档：
  - Session Input：`_notify`、经 Notify 的 ask 结果、Goal `answer`；
  - Agent State：`notify.*`；
  - Session Directory：run.json `asks[].notify_id / task_id / deadline_ms`。
- [x] `src/frame/lib_opendan/README.md`、`src/frame/opendan/README.md`。
- [x] 纪要本身不改；在本文 §11 记录对应关系。

### N11：验证（第一段的退出条件）

- [ ] §5 全部通过；§6 全绿；DV 场景（V20）记录消息、Task 和 Activation 序列。——库级与 mock 的 §5 全部通过、§6 全绿（§12.4）；DV 未做（§12.5）。

### N12：buckyos-websdk（同级仓）

- [x] `notify.send / get / list / ack / dismiss / cancel` 的 TS 封装，以及 `waitNotify / answerNotify`，与 buckyos-api 保持同形。
- [x] 按该仓的构建与测试约定验证；`src/buckyos-build.py` 会从同级源码构建 SDK。

---

## 5. 第一段验收矩阵

| # | 场景 | 期望 |
|---|---|---|
| V01 | 普通 App 发 `notice` | 只产生 MsgObject，不建 Task；进入目标用户的 `sysnotify:app` 会话（自动登记、进 INBOX 不进 RequestBox），带链接；重放同一个 key 返回同一个 Notify，不重复发 |
| V02 | 发 `input` | 消息与 HumanSet Task 都建出，`task_id` 写进消息与视图；请求方能读结果、能撤销 |
| V03 | 分类与权限 | Agent 来源为 `agent`；普通 App 为 `app`，它指定 `todo` 被拒；系统服务可以指定 `todo`；App 不能通知别的用户；`list` 只返回调用方用户的 Notify |
| V04 | 等级与已读（Agent 来源） | `quiet` 只有主记录且已读；`normal` 有来源会话时另投一处、主记录已读，没有时只有主记录且未读；`timely` 另投来源会话和最近活跃会话；`urgent` 投全部候选，且都未读 |
| V05 | 状态 | `notice`：已读后为 `read`，`ack / dismiss` 生效，过期后为 `expired`；`input`：回答 / 忽略 / 过期 / 撤销各自正确 |
| V06 | MessageHub | 系统通知实体与三个分类会话、筛选、未读数；卡片按 prompt 处理；主记录与副本状态一致；并发提交时一处成功、另一处显示最终结果；`?notify=` 深链；分类会话没有输入框；Agent 会话的输入框不被锁定 |
| V07 | Work `ask-user` | 建出 `input` Notify（来源含 session / turn / call，`conversation` 是父 UI 的 binding，level `timely`）；`waiting_for = Input`，`deadline_ms = now + 3h`；Turn task 为 `Waiting(HumanInput, related_task_id = task_id)` |
| V08 | 在 MessageHub 卡片回答 | Work 恢复，工具结果的 text 与 option 正确，`via = notification` |
| V09 | 超时 | Notify 被撤销（`ask_timeout`）；工具结果为 `answered: false, reason: timeout`；Work 继续由 LLM 决定 |
| V10 | 用户忽略 / 请求方撤销 | 工具结果为 `dismissed` / `canceled` |
| V11 | 等待中 stop | Notify 被撤销（`turn_stopped`），Turn 以 Stopped 结束 |
| V12 | kill -9：发出前 / 发出后提交前 / 提交后 / 停机期间被回答 | 恰好一个 Notify、一次回填 |
| V13 | 等待中的 Work 被卸载 | 回答后由 kevent 唤醒；没人回答时到截止时间被唤醒 |
| V14 | Goal questions | 新问题发出 Notify；连续 Activation 中重复出现的问题不重发；消失的问题被撤销；回答后产生原因 `answer` 的 Activation，Goal 头带上回答；问题状态更新 |
| V15 | 回归 | Goal 内 `ask-user` 报新文本；UI 内 `ask-user`（意图分析 TODO V05—V08）不变；`deps.notify = None` 时 Work 维持原报错 |
| V16 | 观察 | 另投进 Agent 会话的 Notify 在对应 UI Session 产生 `_notify` 事件：不开 Turn，随下一批输入出现在快照中；没有 Session 时新建，binding 来自 `reply_route` |
| V17 | 代理 | UI Session 代答 `input` 后，Work 恢复，并留有 task note；代确认 `notice` 后为 `read`；代答 `approval` 被拒；代答本 Session 发起的 Notify 被拒；代答已结束的 Notify 返回最终状态 |
| V18 | `notify send` | UI / Work / Goal 中都可用；`list / show / cancel` 正常 |
| V19 | TaskCenter 兼容 | Notify 的 Task 不出现在 TaskCenter 首页的 approve / reject 卡片中；列表与详情有到 MessageHub 的链接 |
| V20 | DV | ① 一个非 Agent 的系统服务或测试 App 经 Rust 客户端发 `notice` 和 `input`，用 `wait_notify` 拿到在 MessageHub 中给出的回答；② Work 中途提问 → 用户在 MessageHub 卡片回答 → Work 完成；③ 同样的提问另投到 Telegram（DV 没有绑定时用原生会话），用户在会话里直接用文字回答 → UI Session 观察到并代答 → Work 继续；④ Goal（`every_secs`）的 Activation 用 `notify send` 发提醒 → 用户在会话里回复“今天在度假，别打扰” → UI Session 识别为反馈并执行 `goal post` → 下一次 Activation 据此调整；⑤ Goal 提问 → 用户回答 → 产生原因 `answer` 的 Activation |
| V21 | websdk | TS 封装与 Rust 客户端同形；在 DV 上用一个页面发送并等待回答 |

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

websdk：按 `buckyos-websdk` 仓的约定；改完后 `uv run buckyos-build.py` 会从同级源码构建 SDK / CLI 分发包（本地源码构建失败会中止）。

DV：

- msg-center：`uv run buckyos-build.py -s msg_center` 后 `uv run start.py`。这会停系统并更新安装文件，不是普通重启。改了 RBAC 时同时更新相关模块。
- Agent：`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`。
- desktop 静态文件：`cp -r dist/. /opt/buckyos/bin/control-panel/web/`。
- 驱动脚本：在 `test/test_opendan/` 新增 `test_agent_notify.ts`，覆盖 `send / answer / wait`，可参考 `test_agent_ask.ts`。V20 ① 用一个小的 Rust 测试程序或脚本，以 App 身份调用。
- 注意：
  - 已有的 UI Session 冻结了旧的 behavior，要发 `/stop` 开新一代；
  - DV 结束后把测试 Goal 归档；
  - 意图分析 TODO §11.5 记录的旧 `/5` UI Session 卡点要先解决。

---

## 7. 实施顺序

1. N1 → N2：接口与 msg-center，先把 App 能用的能力立起来，可以单独测（V01—V06 的后端部分、V20 ①）。N7 的 MessageHub 部分可以先用 mock 并行做；N12 跟在 N1 之后。
2. N3 → N4 → N5：libopendan 用 `LocalNotify` 跑通库级测试。
3. N6（Loader）、N7（接真实后端）、N8（Jarvis）。
4. N11 的 DV；最后回写 N10 文档。

---

## 8. 决定与待确认

### 8.1 用户答复（2026-10-11）

1. System Notify 放在 msg-center 里做。
2. UI 放在 MessageHub：它本身就擅长展示来自不同来源的会话。TaskCenter 不当通知中心，只做兼容。
3. Work 的等待缺省 3 小时。
4. workflow 先不管（还没正式用起来）。
5. MessageHub 里按通知对用户的重要程度分类：
   - **0 待办**：基于 workflow，别人把工作指派给你，你得到一个 TODO，通常最需要人关注；
   - **1 Agent**：多半需要人处理，否则工作推进不下去；
   - **2 应用**：传统的应用系统通知。

   怎么展示以后可以再优化，甚至给用户配置；**第一期要的是 App 能稳定使用系统通知这个系统能力的接口**。→ `category` 写进接口（§3.0），MessageHub 第一期按三个分类会话简单展示（§3.3、§3.4）。
6. Q1：放在 msg-center 里，告知这类用特殊类型的 MsgObject。→ 每个 Notify 都是 `kind = Notify` 的 MsgObject；只有需要回复的才另挂 TaskMgr Task（§3.1）。
7. Q9（UI 代理）：同意。
8. 系统通知发送者用 Zone DID；主记录直接写进用户 INBOX，不走联系人分级；RBAC 给 App 主体开 `obj://msg-center/notify` 写权限（§3.2 第 4 步、§3.3、N1）。
9. websdk 的 TS 封装（N12）放在第一段。

### 8.2 未单独答复、按推荐写的事项（实施前可再调）

| # | 事项（纪要 §13.2 对应） | 取值 |
|---|---|---|
| Q4 | 通知策略 | 四个等级加 §3.3 的规则；不回退、不升级；非 Agent 来源第一段只有主记录 |
| Q5 | 状态与时限（除 Work 时长外） | 状态见 §3.1；告知缺省不过期（可给 `expires_at_ms`）；Goal 提问不过期，从 `questions` 消失即撤销；迟到的回答因 Task 已结束而被拒，界面显示最终状态 |
| Q6 | `ask_user` 约束 | 所有无队列的 Work 都可用，0 表示关闭；交互式子 Session 与 self_* 仍报错；Goal 走 questions；提示词保持“优先快速失败” |
| Q7 | 可恢复等待 | run.json `asks[call_id]` 保存 `notify_id`、`task_id` 与 `deadline_ms`；先发（幂等）后提交 |
| Q8 | 多入口一致性 | `input` 的所有入口都直接写 TaskMgr，带 `expected_revision` CAS，先到先得；`notice` 的已读与忽略是 msg-center 记录状态，天然一致 |
| Q10 | 环境观察接口 | Agent 会话副本的 INBOX 观察记录，转成 `_notify` 半订阅事件；只有“已投递”事件 |
| Q12 | 第二段范围 | `post-message`、机械审批、触达设置与升级、分类展示的优化与用户配置；机械审批是否近期做，按“先能力后安全”另议 |
| — | Reply Receipt 与路由 | 第二段 §9.1 |

### 8.3 本文的实现决定（如需调整，请在实施前提出）

- **ask-user**：LLM 只看到 `ask-user` 一个命令；走原地问答还是 Notify 由 Session 决定。Work 的问题用 `timely`，Goal 的问题与 `notify send` 缺省用 `normal`。
- **未得到回答**：用成功结果加 `answered: false` 表示，不用错误观察，让 LLM 按正常分支处理。
- **id 与链接**：`notify_id` 由 msg-center 分配（`nt-…`），`input` 另有 `task_id`。统一 URL 是 `/messagehub?notify=<notify_id>`，发消息前就能确定，正文里可以直接写。
- **来源由服务端填写**：`source.app_id / agent_did` 由 msg-center 从调用方身份填写，`category` 由 msg-center 推导。这样 App 冒充不了别人，也不能把自己放进“待办”。
- **系统通知发送者与主记录**：主记录用系统通知发送者，直接写 INBOX；Agent 会话的副本用 Agent DID，因为 Telegram 必须经 Agent 的 bot 发。
- **Notify 与 Session 历史**：Notify 不进 Session 的 outbox（`OutboxPurpose`），也不写成会话历史里的 assistant 或 user message；UI Session 只通过 `_notify` 观察。
- **另投范围**：只走私聊与 Telegram 私聊，不投进群。
- **TaskCenter**：首页排除 Notify 的 Task，避免同一件事在两处各有一张卡，也避免 approve / reject 的错误提交。
- **未接入 Notify 时**：`deps.notify = None` 时一切维持今天的行为，xagent 和 fixtures 也一样。`LocalNotify` 只用于 `--dev` 和测试。

### 8.4 已知风险（按“先能力后安全”记录，不加机制）

- **Agent 能直接提交主人的人工任务**：Agent 进程的主体是 `app:<agent>@<owner>`（`opendan/src/tasks.rs:274-279`），TaskMgr 按 user_id 判断 assignee，commit 不检查 App（`server.rs:1778-1792`）。所以 Agent 可以绕过 CLI 规则，直接 commit 指派给主人的任何 HumanSet 任务。第一段的 `approval` 没有生产者，影响有限；在审批有生产者之前（§9.2）必须先关上这个口子。实施前核对 Agent token 的 user_id。
- **会话之间可以冒充消息**：`agent-session post <sid> --msg --from <任意 DID>`（`bin/xagent.rs:433-437, 723-735`）能把冒名的消息投进任何有队列的 Session，与纪要 §8.2 相反。建议之后在 Session 内把 `--from` 限制为调用方本身。
- **任何 App 都能给自己的用户发通知**：没有频率限制，可能变吵。分类只能防止 App 自称“待办”，防不了滥发。
- UI Session 可能把一句含糊的“好的”归到错误的 Notify 上，只靠提示词约束。
- 等待中的 Work 最多占着一个 `max_sub_sessions` 名额（缺省 4）3 小时。
- `notice` 的“已读”取自 msg-center 的阅读状态，只能说明用户打开过会话，不能证明他真的看到了。
- Notify 的 Task 仍出现在 TaskCenter 的任务列表里（只是不进首页卡片）。
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
- `kind = approval` 加入接口；`human.approval/v1` 的严格 schema 改为 notify envelope（目前没有生产者，可以直接改）。
- TaskMgr 对审批的 commit 只接受受信 UI App，关闭 §8.4 第一条风险。
- 超时的语义（按拒绝处理）。
- 当前 UI Session 的调用被阻塞时，MessageHub 用毛玻璃遮罩加模态框；后台 Work 被阻塞时只显示通知卡片（纪要 §6.5）。
- 审批 Notify 是否作为 Turn task 的子 task（TaskMgr §11.7，以及“stop 让审批作废”）：这需要 msg-center 在 Agent 的 Turn task 下有 CreateChild 权限，或改由请求方自己建 Task。

### 9.3 展示、触达策略与来源扩展

- 分类展示的优化与用户配置：排序、合并、哪些分类要提醒、按来源静音（用户答复 5）。
- 用户的通知设置：放在 control_panel 资料的 `system_contact`，还是 msg-center 自己存。
- 未处理时的升级（多久升一级）；新渠道：邮件、短信网关、BuckyOS App 推送；勿扰时段；频率限制。
- 非 Agent 来源的外部渠道（需要系统级 bot 或其它发送通道）。
- 桌面状态栏托盘接入：`notificationCount` 取待处理 Notify 数，点击进入 MessageHub 的系统通知。
- `todo` 分类的生产者：workflow 的指派（workflow 正式使用后）。

### 9.4 状态变化的观察

- 回答、过期等变化也作为 `_notify` 事件投给相关 UI Session。
- 请求方 Session 收到“你发出的通知已被处理”。

### 9.5 纪要 §三（不属于通知）

处理中补充输入的归属、占位与补充消息的排列、“明确排队 / 明确本轮吸收”两种输入方式，另立 MessageHub / Agent Chat TODO。现状是：runtime 在每次推理前读取输入，receipt 记录了 Turn 消费了哪些输入，MessageHub 的排列方式未核对。

### 9.6 智能提醒完整路线

Goal 安排一次性唤醒（Goal TODO §9.1 的 `Once` 与 §9.5 的 follow_up）→ 到点启动 Work 判断 → `notify send`。第一段只验证最后一步（V20 ④）。self_check 退役按 Goal TODO §9.3 另做；它用的 `dcrontab … remind` 本文不处理。

---

## 10. 第一段禁止的捷径

- 不要把 Notify 做成 Session outbox 中的 Assistant Message，也不要把它写进会话历史冒充 user 或 assistant 消息。
- 不要为了让 Work 收到回答而给它建 Inbox / 输入队列，也不要把用户在会话里的回复路由进 Work。
- 不要让 LLM 在 `ask-user` 里选择“原地还是通知”，也不要新增一个 `ask-user-notify` 命令。
- 不要把“已发出 / 已送达 / 已在界面呈现”当成“已回答 / 已批准”。
- 不要另存 Notify 状态：`input` 只看 Task，`notice` 只看 msg-center 记录。卡片状态不能只看某一条消息本身。
- 不要让 App 自报 `source.app_id`、`agent_did` 或 `category`。
- 不要让任何 Notify 等待没有截止时间。
- 不要因为 Work 或 Goal 的 Notify 锁住 MessageHub 输入框或整个会话。
- 不要让 Agent 处理审批，也不要让它处理本 Session 发起的 Notify。
- 不要用 Timer 或自我唤醒循环等待 Goal 的回答；回答以 `system:goal` 事件投回。
- 不要为了 `notice` 建 TaskMgr Task；不要把通知中心另做在 TaskCenter。
- 不要实现 `post-message`、机械审批、触达升级、workflow `remind` 改造。
- 不要改 llm_context / agent_tool；需要改时单列 TODO 先行。

---

## 11. 纪要与本文的对应

| 纪要 | 本文 |
|---|---|
| §一 机制分离 | §0 |
| §二 Session Inbox / Outbox 基线、Work 不建队列 | 不变；§3.6 不给 Work 建队列 |
| §三 Turn 与处理中补充输入 | §9.5 |
| §四 LLM 澄清与机械授权 | UI 原地澄清不变；工具化询问见 §3.6；机械授权见 §9.2 |
| §五 System Notify（独立通道、两类、异步可恢复、复用 Task） | §3.0—§3.2；“两类”对应 `notice`（只有消息）与 `input`（消息 + Task） |
| §六 触达（统一 URL、默认链接、来源与渠道分离、强度、阻塞展示、通知中心） | §3.3、§3.4（MessageHub 分类会话）；强度扩展与展示优化见 §9.3；模态遮罩见 §9.2 |
| §七 Work 与 `ask_user` 兜底 | §3.6 |
| §八 内部协作与外部通信 | 不变（树状 Session）；§8.4 第二条风险 |
| §九 `post_message` | §9.1 |
| §十 Work 有界、Goal 长期 | §3.6 截止时间（3 小时）；§3.7 |
| §十一 智能提醒 | §3.8 `notify send`；V20 ④；§9.6 |
| §十二 观察与代理 | §3.9、§3.10 |
| §13.2 待定项 | §8.1、§8.2 |


---

## 12. 第一段实施记录（2026-10-11）

### 12.1 落点

| 部分 | 位置 |
|---|---|
| App 契约（N1） | `kernel/buckyos-api/src/msg_notify.rs`（类型、常量、`input_prompt` / `prompt_options`、`NotifyAnswer::validate / to_task_result`、`commit_notify_answer`（CAS + 冲突重读，抽象为 `NotifyTaskAccess`）、`wait_notify_loop`、`input_status_of`、`machine_content / machine_data`）；`msg_center_client.rs` 的 `notify_send / get / list / ack / dismiss / cancel` 与 `MsgCenterHandler::handle_notify`（缺省实现，`notify.*` 一个入口分发）；`runtime.rs` 的 `wait_notify(id, deadline)`（任务 kevent + 轮询兜底）、`answer_notify`；`rbac_config.rs`：`users / admin / app / frame` 对 `obj://msg-center/notify` 读写 |
| msg-center（N2） | `frame/msg_center/src/notify.rs`（`NotifyStore` 建 `notifies` 表、`NotifyCaller` 按 token 推导请求方 / 用户 / App / Agent、`notify_send` 的各步与断点续做、候选会话与等级、副本与观察记录、`get / list / ack / dismiss / cancel`、清扫）；`NotifyTasks` 是对 TaskMgr 的窄接口（生产 `ZoneNotifyTasks`）；`owner_session(.rs/_db.rs)`：`notify` 计入活跃；`main.rs` 启动清扫；测试 `test_notify.rs` |
| websdk（N12） | `buckyos-websdk/src/msg_center_client.ts`（类型、`notifySend…`、`inputPrompt`、`waitNotify`、`answerNotify`、`commitNotifyAnswer` 等）、`task_mgr_client.ts`（HumanPrompt 类型）、`sdk_core.ts` 导出、`tests/msg_center_client.test.ts`；按该仓惯例重建了 `dist/` |
| libopendan（N3–N5） | `src/notify.rs`（`NotifyHost`、`NotifyVia`、`LocalNotify`、`StateNotify`、`complete_notify_source`、`notify_event`）；`protocol`：`SessionPolicy.ask_user_timeout_secs`、`_notify` 常量与 `system:<id>` 子 id 匹配、`UserInputAnswer.via / notify_id`、`AskNotify`（schema `ask_notify`）、`WaitingFor.human_tasks`、`GoalQuestion.notify_id / status / answer`、`ActivationReasonKind::Answer`；`template.rs`（ui 隐式 `_notify`、`ask_user_timeout_secs` 覆盖）；`runner/ask.rs`（`via_notify`、`send_notify_questions`、`notify_answers`）、`live.rs`（回填）、`drive.rs` / `outcome.rs`（先发后提交）、`turn_task.rs`、`goal.rs`（`questions_to_user`、`<open_questions>` 带状态与回答）、`assembler.rs`（Work 子 context 的提问说明）；`host/mod.rs`（`HostDeps.notify`、Supervisor / ChildDriver 的 recheck）、`host/goals.rs`（`collect_answers`、`recheck_answers`）；`state/`（`AgentStateClient::notifies()`、`WithNotify`、kRPC `notify.*`）；`bridge/msg.rs`（`MsgBridgeOutput::Observe`）；`bin/xagent.rs`（`notify` 命令）；`error.rs`（`NotifyConflict`）；测试 `tests/notify.rs`、`crash.rs`、`goal_session.rs`；fixtures `17_notify` |
| Loader（N6） | `frame/opendan/src/notify.rs`（`ZoneNotify`、`watch_answers`）；`loader.rs`（`LoaderEnv.notify`、`WithNotify` 包装、监视器）；`main.rs`（zone：`ZoneNotify`；`--dev`：`LocalNotify`）；`ui.rs`（观察记录路由、`session_for` 用 `reply_route`）；`tasks.rs`（`HumanInput` + `related_task_id`）；`tests/loader.rs` 两个端到端 |
| Desktop（N7） | `src/frame/desktop/src/app/messagehub/notify/`（`NotifyCard`、`watch`、`api`、`model`）、`mock/notify.ts`、`protocol/msgobj.ts`、`sessionModel.ts`、`api/projection.ts`（系统通知实体）、`MessageHubRoute.tsx`（`?notify=`）、`conversation/input/inputState.ts`、TaskCenter 兼容（`api/task_mgr.ts`、`TasksPage.tsx`、`TaskDetailPage.tsx`）、i18n、`UI_DATAMODEL.md` v0.12 §3.5.6；测试 `tests/datamodel/messagehub-notify.test.ts`、`tests/e2e/pages/messagehub-notify.spec.ts` |
| Jarvis（N8） | `plan.toml` / `do.toml` / `work_simple.toml`（Asking the User）、`goal.toml`（questions 经通知、`notify send`）、`chat_route.toml` / `groupchat_route.toml`（Notifications）、`agent.toml`（`ask_user_timeout_secs = 10800`） |
| 文档（N10） | `doc/message_hub/System Notify.md`（新）、`Message Center.md` §2.1.3 与 API 汇总、`task data schema.md`（notify envelope）、架构设计 §5.6 / §12.11 / §12.12.2 / §15.5 / §17.2、`xAgent.md`、`Session Input / Session Directory / Agent State Protocol`、两个 README、`doc/sdk/app-dev-quickstart.md` 入口；DV 驱动 `test/test_opendan/test_agent_notify.ts` |

### 12.2 与原文不同的决定

1. libopendan 不另写 `protocol/notify.rs`：宿主接口直接用 buckyos-api 的请求 / 视图类型；协议里只加 `AskNotify` 与几个字段，`NotifyVia` 在 `libopendan::notify`。
2. `_notify` 事件的来源是 `system:notify:<notify_id>`（不是 `system:notify`）：半订阅按 `(subscription, source)` 合并，同一来源只留最新版本，多条通知会互相覆盖。为此 `system:<id>` 订阅也匹配子 id `system:<id>:<sub>`。一行渲染就是事件的 `summary`（`notify_event` 生成），没有另加 assembler 内建。
3. 绑定了会话的 Session（有 `channels.outbound`，即 Loader 建的 UI Session）只在有受控输入时 bootstrap：否则为观察记录新建的一代会自己跑 `on_init` 并回复用户。本地 UI 与 Work 不变。
4. 经通知的回答走普通工具结果路径（与 task 等待相同），不写空 inputs 的输入 receipt。
5. Supervisor：无队列、`waiting_for = input` 的 Session 卸载后按 `recheck`（30 s）重新装载，没有单独按截止时间排期（截止时间由 drive 判断）；ChildDriver 对这类子 Session 每 `poll_interval × 6` 看一次（只在宿主有通知服务时）。
6. Goal 问题的状态与回答在 `answer` Activation 关闭时（读通知）写入，`reason.ref = notify_id`；GoalHost 每 `answers_every`（30 s）查一次未结束问题的通知，Loader 的监视器在任务 kevent 到来时 `recheck_answers`。没有用 `notify.list{goal_id}`。
7. 代确认 `notice` 记在视图的 `acked_by`（`notify.ack` 的 `on_behalf`），不写 `closed_reason`。
8. `notice` 的 `read` 只看“写入时未读”的记录（未读的主记录、原生副本）：预置为已读的主记录不算读过。
9. `notify.ack / dismiss` 只用于 `notice`；`input` 的忽略是向任务提交 `action = dismiss`。`notify.list` 允许系统服务用 `to_user` 指定用户；状态过滤逐条算视图。
10. msg-center 对 TaskMgr 走 `NotifyTasks` 窄接口（测试用内存实现）。Telegram 用例检查的是 tunnel 投递队列里的副本（纯文本 + 链接），没有起 DryRun 网关。
11. `LocalNotify` 以 `state/notifies/` 目录存在为启用条件（`--dev` 的 Loader 启动时创建；Session 里的 `agent-session` 经文件版 Agent State 也能用）。`xagent notify` 主人一侧另有 `dismiss`。
12. fixtures `17_notify` 只含 Work 经通知等待与 UI 的 `_notify` 观察；Goal 问题回流由 `goal_session.rs` 覆盖。

### 12.3 验收对照

V01—V05：`test_notify.rs`（6 个）；V06、V19：Desktop datamodel 与 Playwright（mock）；V07—V11、V13、V15—V18：`tests/notify.rs`（7 个）与 `tests/loader.rs`（2 个）；V12：`crash.rs::asking_through_a_notification_survives_a_crash_at_every_stage`（发出前、发出后提交前、提交后，各恰好一个通知、一次回填，且不重复推理）；V14：`goal_session.rs::goal_questions_reach_the_user_and_answers_come_back`；V21 的 TS 封装：websdk 单测。V20 与 V21 的 DV 部分未做。

### 12.4 验证（在 `src/`，2026-10-11）

- `cargo test -p buckyos-api --lib -- --test-threads=1 msg_notify rbac_config`：27 通过。
- `cargo test -p msg_center -- --test-threads=1`：143 通过（含 notify 6 个）。
- `cargo test -p libopendan -p opendan -- --test-threads=1`：33 个测试目标全部通过。
- `cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures` 重新生成；`--test fixtures` 19 通过。
- `cargo check --workspace --tests --exclude node_daemon` 通过；`node_daemon` 有他人正在进行的未提交改动，依赖更新版 `name_lib`，与本文无关。
- Desktop（子任务完成）：`tsc -b`、`eslint`（0 error）、deno datamodel 101 通过、Playwright（mock，`--workers=2` 分批）通过，`messagehub-agent-task` 有一个既有的偶发失败；`pnpm build` 通过。
- websdk：`tsc`、`jest --runInBand` 502 通过、`pnpm run build`。

### 12.5 DV（未做）与步骤

1. msg-center：`cargo build --release -p msg_center`（已构建），备份并替换 `/opt/buckyos/bin/msg-center/msg_center`（或 `uv run buckyos-build.py -s msg_center` + `uv run start.py`）。RBAC 新规则编在 msg-center 里，它自己的鉴权即可生效。
2. Desktop：`cp -r src/frame/desktop/dist/. /opt/buckyos/bin/control-panel/web/`（dist 已构建）。
3. Agent：`./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`（Loader 用 `ZoneNotify`；旧 UI Session 冻结了旧 behavior，要 `/stop`；先处理意图分析 TODO §11.5 的旧 `/5` Session）。
4. 驱动：`cd test/test_opendan && deno run --config ../deno.json --allow-net --allow-env --allow-read --allow-write --unsafely-ignore-certificate-errors test_agent_notify.ts send input "覆盖吗？" --option yes=覆盖 --option no=保留`，在 MessageHub 卡片回答后 `… wait <id>`（V20 ①、V21）；`… ask "<让 Work 中途 ask-user 的请求>"` 等待 Agent 的通知（V20 ②），再在卡片或会话里回答。
5. 需要在 DV 上确认的点：msg-center 以 `system:msg-center` 创建 `human.input/v1`（storage domain User）并授权是否被 TaskMgr 接受；Agent App 主体 `notify.list` 的 `resolve_user_did`；观察记录经 Loader 路由；MessageHub 对系统通知发送者（Zone DID）的展示。

### 12.6 DV 发现并修正（2026-10-12）

- 用户在 DV 让 Agent 发测试通知，`agent-session notify send` 报 `notifications are not available here`。原因：Session 的工具进程看得到 AgentRoot，连的是文件版 Agent State，而 `ZoneNotify` 只挂在 Loader 进程里（`WithNotify`）；库级测试都用 `LocalNotify`，没覆盖这个组合。Work 的 `ask-user` 与 Goal 提问走 runner（Loader 进程内），不受影响。
- 修正：`ZoneNotify` 移到 `libopendan::notify`（opendan 重新导出）；`agent-session notify` 先用 Agent State 的通知服务，没有时 `ZoneNotify::for_session_process()`：取宿主交给工具进程的 `BUCKYOS_APPCLIENT_SESSION_TOKEN`，`sub` 为收件人，`appid` 写入 `BUCKYOS_APP_ID` 后 `ensure_buckyos_runtime()`；失败时错误里带原因。
- 验证：在 Jarvis 容器里以容器环境 + 会话 token 运行新 xagent，`notify send` 成功（测试通知随后撤销）；`libopendan` notify / xagent_cli / crash 与 `opendan` 全部测试通过。需要重建 aios 镜像（或用 `debug_jarvis.sh`）才会生效。

### 12.7 遗留

- DV 验收（上节）；V20 ① 的 Rust 小程序未写（TS 驱动覆盖同一契约）。
- `notify_view` 对每个 `input` 读一次任务，`notify.list` 条数多时偏慢；Loader 监视器每 30 s 列一次本 Agent 的未结束提问。
- MessageHub 不渲染 Agent 自己 INBOX 里的观察记录（只有观察该 Agent 邮箱时可见）；launch schema 没有 `notify` 参数，只支持 `/messagehub?notify=` 路由；App 来源显示原始 `app_id`。
- 第二段 §9 全部。
