# System Notify：App 通知用户的接口

本文面向 App 开发者，说明 App 怎样通过 msg-center 给 Zone 用户发通知、向用户提问并等待回答。接口定义在 `buckyos-api` 的 `msg_notify.rs`（Rust）与 `buckyos-websdk` 的 `msg_center_client.ts`（TS）；服务端实现在 `src/frame/msg_center/src/notify.rs`。

## 1. 概念

一条通知（Notify）就是 msg-center 里一条 `kind = notify` 的消息，结构化内容放在 `content.machine`（intent `buckyos.notify`，数据见 §5）。通知有两类：

| `kind` | 用途 | 状态的真相 |
|---|---|---|
| `notice` | 告诉用户一件事，不需要回答 | msg-center 中这条消息的记录（已读、归档） |
| `input` | 向用户提问，需要回答 | 另挂的 TaskMgr 人工任务（HumanSet，schema `human.input/v1`）；回答就是任务结果 |

用户在 MessageHub 的“系统通知”里看到通知：按对用户的重要程度分为三个会话——**待办**（`todo`）、**Agent**（`agent`）、**应用通知**（`app`），每条通知是其中一张卡片，可以原地回答、确认或忽略。统一链接 `https://<zone host>/messagehub?notify=<notify_id>` 打开这张卡片。

## 2. 方法

kRPC 服务 `msg-center`（`/kapi/msg-center`）：

| 方法 | 调用方 | 作用 |
|---|---|---|
| `notify.send` | 发送方 | 发出通知，返回 `NotifyView` |
| `notify.get {notify_id}` | 发送方、目标用户、系统服务 | 读取当前状态 |
| `notify.list {…}` | 同上 | 列出调用方所属用户的通知 |
| `notify.cancel {notify_id, reason?}` | 发送方、系统服务 | 撤销；`input` 的任务以 Canceled 关闭 |
| `notify.ack {notify_id, on_behalf?}` | 目标用户 | `notice` 标为已读 |
| `notify.dismiss {notify_id}` | 目标用户 | `notice` 忽略（归档） |

回答 `input` 不走 msg-center，而是直接向 TaskMgr 提交任务结果（`commit_result`，带 `expected_revision`，先到先得）。SDK 的 `answer_notify` / `answerNotify` 封装了这一步；`notice` 调用它时等同于 `ack` / `dismiss`。

### 2.1 `notify.send` 请求

```json
{
  "to_user": "alice",
  "kind": "notice | input",
  "level": "quiet | normal | timely | urgent",
  "prompt": { "title": "…", "message": "…", "controls": [] },
  "source": { "session_id": "…", "goal_id": "…", "operation": "一句话：这是为了什么", "link": "App 自己的相关页面" },
  "expires_at_ms": 0,
  "idempotency_key": "…"
}
```

- `to_user`：目标用户的 Zone 用户名。
- `prompt` 就是 TaskMgr 的 `HumanTaskPrompt`。`input` 的约定：有选项时放一个 `Choice{id: "option", options}`，需要文字时放一个 `TextInput{id: "text"}`；`input_prompt(title, message, options, allow_text)` / `inputPrompt(...)` 按这个约定生成。`notice` 的 `controls` 为空。
- `level` 缺省 `normal`，表示“期望多强地触达”，具体投到哪里由系统决定，可能随版本变化（§3）。
- `source.app_id`、`source.agent_did` 由 msg-center 按调用方身份填写，调用方填了也会被覆盖。
- `category` 由 msg-center 推导：调用方是 Agent 时为 `agent`，其余为 `app`。只有系统服务可以显式指定（例如 `todo`）；普通 App 指定了不同的分类会被拒绝。
- `expires_at_ms`：到期后 `notice` 归档、`input` 的任务被撤销，状态为 `expired`。缺省不过期。
- `idempotency_key`：同一调用方、同一个 key 返回同一条通知，不会重复投递。中途失败时用同一个 key 重调即可从断点继续。

### 2.2 `NotifyView`

```json
{
  "notify_id": "nt-…",
  "url": "https://<zone host>/messagehub?notify=nt-…",
  "to_user": "alice",
  "kind": "input", "category": "app", "level": "normal",
  "status": "open | read | answered | dismissed | expired | canceled",
  "prompt": { … },
  "source": { "app_id": "…", "agent_did": null, … },
  "created_at_ms": 0, "expires_at_ms": null,
  "closed_at_ms": null, "closed_reason": null, "acked_by": null,
  "task_id": "…",
  "result": { "action": "answer | dismiss", "values": {"option": "…", "text": "…"}, "submitted_by": "alice", "submitted_at": 0, "committed_by": "alice|system:control-panel" },
  "home": { "owner": "<用户 DID>", "session_id": "sysnotify:app", "record_id": "…", "msg_id": "…", "from": "<系统通知发送者 DID>" },
  "deliveries": []
}
```

状态：

| 状态 | `notice` | `input` |
|---|---|---|
| `open` | 还没有任何记录被读 | 任务在等待 |
| `read` | 有记录已读，或被 `ack` | — |
| `answered` | — | 已回答（`result.action = answer`） |
| `dismissed` | 被 `dismiss` | 用户选择了忽略（`result.action = dismiss`） |
| `expired` | 过期 | 过期，任务被撤销 |
| `canceled` | 被发送方撤销 | 任务以其它原因被撤销（`closed_reason` 说明原因） |

`open` 与 `read` 是未结束；其余四个是最终状态。

### 2.3 `notify.list`

参数都可选：`status`（数组）、`kind`、`category`、`agent_did`、`app_id`、`session_id`、`goal_id`、`since_ms`、`cursor`、`limit`。只返回调用方所属用户的通知（系统服务可以用 `to_user` 指定用户）。按创建时间倒序；有更多时返回 `next_cursor`。

## 3. 投递（不属于契约，可能变化）

- 每条通知在目标用户的分类会话（`sysnotify:todo|agent|app`）里有一条主记录，发送者是“系统通知发送者”（Zone DID）。这三个会话不能打字。
- 来自 Agent 的通知还会按等级另投到用户与该 Agent 已有的会话里（原生或 Telegram）：`quiet` 不另投；`normal` 投到来源会话；`timely` 再加最近活跃的会话；`urgent` 投到全部会话。未读只落在用户最可能看的那一处。
- 非 Agent 来源的通知只有主记录（`quiet` 时为已读）。

## 4. 权限

| 调用方 | 能做什么 |
|---|---|
| App 主体 `app:<app>@<user>` | 只能通知 `<user>`；读取、列出 `<user>` 的通知；撤销自己发的通知；`ack` / `dismiss` `<user>` 的 `notice` |
| 用户会话（浏览器中的 App） | 同上，`<user>` 是登录用户 |
| 系统服务 | 可以通知任何用户、指定 `todo`、读取和撤销任何通知 |

RBAC 资源是 `obj://msg-center/notify`（`users`、`admin`、`app`、`frame` 组有读写权限）。回答 `input` 的权限由 TaskMgr 判断：目标用户是任务的 assignee。

## 5. 消息数据

主记录与副本的 `content.machine`：`{intent: "buckyos.notify", data: NotifyMessageData}`，其中 `data` 是 `{version: 1, notify_id, kind, category, level, home, source, prompt, task_id?, url, expires_at_ms?}`。`home = true` 是分类会话里的主记录，`false` 是另投到会话里的副本。消息正文是给不认识这种 intent 的客户端（例如 Telegram）看的纯文本，最后一行是处理链接。

`input` 的任务：`schema_id = human.input/v1`，`input = {notify: {notify_id, kind, category, level, source, expires_at_ms, url}, prompt}`，结果是 `{kind: "human", action: "answer" | "dismiss", values: {option?, text?}, submitted_by, submitted_at}`。

## 6. 用法

### 6.1 Rust（App 服务进程）

```rust
use buckyos_api::*;

let runtime = get_buckyos_api_runtime()?;
let msg_center = runtime.get_msg_center_client().await?;

// 告知
let notice = msg_center
    .notify_send(NotifySendReq::notice("alice", "备份完成", "今晚的备份已完成。", "backup:2026-10-11"))
    .await?;

// 提问并等待回答（最多 1 小时）
let mut ask = NotifySendReq::input(
    "alice",
    "是否覆盖旧版本？",
    "目标目录里已有同名文件。",
    vec![HumanPromptOption::new("yes", "覆盖"), HumanPromptOption::new("no", "保留")],
    false,
    "sync:conflict:42",
);
ask.expires_at_ms = Some(now_ms + 3_600_000);
let view = msg_center.notify_send(ask).await?;
let view = runtime.wait_notify(&view.notify_id, now_ms + 3_600_000).await?;
match view.status {
    NotifyStatus::Answered => println!("{:?}", view.result.and_then(|r| r.option().map(str::to_string))),
    _ => println!("没有回答：{}", view.status.as_str()),
}

// 撤销
msg_center.notify_cancel(&view.notify_id, Some("no_longer_needed".into())).await?;
```

### 6.2 TypeScript（websdk）

```ts
import { buckyos, inputPrompt, waitNotify, answerNotify } from 'buckyos'

const msgCenter = buckyos.getMsgCenterClient()
const deadline = Date.now() + 3_600_000
const view = await msgCenter.notifySend({
  to_user: 'alice',
  kind: 'input',
  prompt: inputPrompt('是否覆盖旧版本？', '目标目录里已有同名文件。', [
    { value: 'yes', label: '覆盖' },
    { value: 'no', label: '保留' },
  ]),
  expires_at_ms: deadline,
  idempotency_key: 'sync:conflict:42',
})
// 等到结束或截止时间，返回最后的视图；kevent 可省略（只轮询）
const done = await waitNotify(msgCenter, view.notify_id, deadline, { kevent: buckyos.getKEventClient() })
if (done.status === 'answered') console.log(done.result?.values?.option)
// 用户一侧（例如自定义的处理页面）：返回最终视图，别人先答了就是别人的答案
await answerNotify(msgCenter, buckyos.getTaskManagerClient(), view.notify_id, { option: 'yes' })
// 撤销
await msgCenter.notifyCancel(view.notify_id, 'no_longer_needed')
```

类型与 Rust 同名同字段（snake_case）。`MsgCenterClient` 上的方法：`notifySend(req)`、`notifyGet(id)`、`notifyList(req?)`、`notifyAck(id, onBehalf?)`、`notifyDismiss(id)`、`notifyCancel(id, reason?)`。辅助函数：`inputPrompt(title, message, options, allowText = false)`、`promptOptions(prompt)`、`validateNotifyAnswer(answer, prompt)`（返回错误文本或 `null`）、`notifyAnswerToTaskResult(answer, submittedBy, submittedAtMs)`、`notifyUrl(zoneHost, id)`、`notifyMessageData(msg)`、`commitNotifyAnswer(taskMgr, taskId, answer, submittedBy, nowMs?)`、`waitNotify(msgCenter, id, deadlineMs, { kevent? })`、`answerNotify(msgCenter, taskMgr, id, answer, submittedBy?)`（`submittedBy` 缺省为通知的 `to_user`）。

## 7. 不要这样用

- 不要把“已发出 / 已送达 / 已读”当成“已回答”。`input` 的回答只看任务结果。
- 不要为等待回答写没有截止时间的循环；`wait_notify` 必须给 `deadline`。
- 不要自己另存通知状态：`input` 看任务，`notice` 看 msg-center 的记录。
- 不要用它代替会话消息：通知不会出现在任何对话历史里。
