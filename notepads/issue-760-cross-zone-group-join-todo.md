# #760 跨 Zone 入群入口补齐 TODO

2026-11-25 · 三项裁决已定，当天已实施（未提交、未部署） · 代码核对基线 `ce6dca07a`

[#760](https://github.com/buckyos/buckyos/issues/760) 报告两个问题。第一，在 Control Panel 用群 DID 或邀请链接查找他 Zone 的群，能找到，但界面提示“暂不支持”，并且没有加入按钮。第二，跨 Zone 的定向邀请不会出现在「我的网络 → 请求 → 群邀请」，只能从 MessageHub 的邀请卡片接受。

[my-network 一期](done/my-network-upgrade-todo.md#L245)当时有意不支持加入他 Zone 的群，前置条件是 group-v2 §4.5。现在 §4.5 已经实现：接受邀请时会建立 Joined Group 绑定，并完成跨 Zone 断言登录和后台恢复。群宿主的 `PUT /<group>/join[?invite=]` 也已经能处理主动申请和邀请链接（[group_http.rs](../src/frame/msg_center/src/group_http.rs#L479-L512)）。所以缺的只有成员 Zone 这一侧。

## 现状

- 本 Zone 的 `group_rpc_authenticated` 在群不在本地时，只把 `accept_invitation`、`get_doc` 和两个成员列表转给宿主，`request_join` 落进本地事务后得到 not-found（[group_service.rs](../src/frame/msg_center/src/group_service.rs#L488)）。
- 成员 Zone 靠 `invitation_id` 跟进审批结果：恢复循环每 30 秒对每个尚未生效的绑定重发 `PUT /join {invitation_id}`，宿主只返回当前状态，不改动记录（[resume_group_binding](../src/frame/msg_center/src/group_remote.rs#L379)、[recover_remote_joins](../src/frame/msg_center/src/group_remote.rs#L508)）。宿主为主动申请生成的成员记录 `invitation_id` 为空，所以照搬接受邀请的写法时，成员 Zone 永远发现不了审批结果。如果改成发空 body 查询，管理员拒绝之后，轮询会替用户再提交一次申请。
- 跨 Zone 的定向邀请只以一条 Operation 消息的形式到达成员 Zone，放在 `dm:<邀请人>` 里，成员 Zone 在用户接受之前不保存任何状态。`list_by_member` 只返回已有绑定、并且状态为 `pending_admin_approval` 的远端群（[remote_group_invitations](../src/frame/msg_center/src/group_remote.rs#L542)）。前端还要求条目带群文档（[realBackend.ts](../src/frame/desktop/src/components/network/realBackend.ts#L249)）。
- 查找结果里远端群的 `group_state` 总是空的（[contact_lookup.rs](../src/frame/msg_center/src/contact_lookup.rs#L460)）。加入以后再查同一个群，界面仍显示“加入”按钮。
- 前端 [JoinGroupDialog.tsx](../src/frame/desktop/src/components/network/JoinGroupDialog.tsx#L38) 遇到远端群就不取群文档、不显示加入按钮。其实后端的 `group.get_doc` 已经能把远端群文档转过来，宿主的 `/doc` 是公开的，里面带 `join_policy`。

## 裁决（owner 2026-11-25 按建议定）

| # | 问题 | 决定 |
| --- | --- | --- |
| D1 | 成员 Zone 怎样跟进主动申请的审批结果 | 宿主为主动申请生成一个申请 id，写进成员记录的 `invitation_id`。成员 Zone 复用接受邀请的恢复路径：批准后得到 active；拒绝或移除后得到 `invitation-mismatch`，删除绑定，不会再次申请。宿主和成员 Zone 都要升级 msg_center，不兼容旧版本。 |
| D2 | 收到的跨 Zone 邀请存在哪里 | 成员 Zone 新建一张小表 `group_received_invitations`，在收到消息时写入。不能存成 Joined Group 绑定，否则恢复循环会替用户接受邀请。 |
| D3 | 邀请列表怎样显示群名 | 宿主在 `invite` 通知里带上群 profile（`session_invite` 已经带了），成员 Zone 存进 D2 的表。列表直接显示群名，不再逐个请求远端。 |

## 实施清单

### 宿主（msg_center）

- [x] `request_group_join` 加入成功后，在成员记录里写一个新的申请 id（`invitation_id`），返回的记录带上它。`revoke_invite` 只作用于 `invited`，所以不受影响。
- [x] `invite_group_member` 发出的 `invite` 通知数据里加上 `profile`。

### 成员 Zone（msg_center）

- [x] `JoinedGroupBinding` 新增 `pending_request { idempotency_key, invite? }`，标记一次还没得到宿主确定答复的主动申请。
- [x] `group.request_join` 加入远端分支，鉴权检查与 `accept_remote_group_invitation` 相同：来源必须是本 Zone 非 External、非 restricted 的已认证主体，不能是远端请求，也不能带 `attestation`。流程如下：
  - 先按群文档解析宿主。
  - 上一次接受或申请如果还没完成，先把它完成。
  - 持久化 `pending_request`，然后 `PUT /<group>/join[?invite=] {idempotency_key}`。
- [x] `resume_group_binding` 处理 `pending_request`，并按宿主答复分三种情况：
  - **宿主接受申请**：校验返回记录，要求群、成员一致，状态为 active 或 pending_admin_approval，pending 时必须带 `invitation_id`。然后保存成员记录，之后按现有路径同步或等待审批。
  - **宿主给出确定拒绝**（NoPermission / ParseRequestError）：清除 `pending_request`。绑定上没有成员记录时，删除绑定。
  - **网络错误或 5xx**：保留 `pending_request`，恢复循环用同一个 key 重发。宿主按 `idempotency_key` 返回上次保存的结果，所以邀请链接的使用次数不会重复增加。
- [x] `recover_remote_joins` 遇到带 `pending_request` 的绑定时不跳过。
- [x] 新表 `group_received_invitations (owner, group_did, invitation_json)`，以 owner + group 为主键：
  - **写入**：另一个 Zone 发来、本 Zone 收件人已收下（没有被拒收）的 `buckyos.group_invitation` / `invite` 通知，并且该群不在本 Zone 托管时写入。同一个群只保留最新的一份邀请。每个 owner 最多 256 条。
  - **读出**：`list_by_member.invitations` 合并其中未过期、尚未加入的条目，`member.state = invited`。条目不带群文档，只带 `group_did` 和 `profile`。
  - **删除**：接受成功、申请成功，或宿主答复 mismatch / expired / revoked 时删除。已过期的条目在列出时清理。
- [x] `contact.lookup` 给远端群补 `group_state`：绑定已生效为 `active`，等待审批为 `pending_admin_approval`，只有收到的邀请为 `invited`。

### 前端（desktop）

- [x] `JoinGroupDialog` 去掉“本 Zone 托管”的限制：远端群也取群文档，显示加入按钮。保留“托管于其它 Zone”标签。
- [x] 「我的网络」群邀请列表接受没有群文档的条目，用 `group_did` 和 `profile` 兜底。
- [x] Mock 后端允许加入远端群。
- [x] 删除 `network.joinGroup.remoteUnsupported` 文案，9 种语言都要删。

### 文档

- [x] [Self-Host-Groupv2.md](../doc/message_hub/Self-Host-Groupv2.md) §11.1 的 join 说明：主动申请的记录带申请 id，成员 Zone 用它查询状态。
- [x] [group-v2-backend.md](../src/frame/msg_center/doc/group-v2-backend.md) 的 Joined Group 一节：主动申请的持久化与恢复、收到的邀请表。

## 验收

- [x] 用 `two_zones` 夹具覆盖以下场景：
  - **invite_only**：返回 `invite-required`，成员 Zone 不留绑定。
  - **request_and_approve**：申请后进入 pending；批准后经恢复变为 active，查找结果显示 active。另一人申请后被拒绝，绑定被删除，宿主上仍是 rejected，没有被重新申请。
  - **open**：直接 active，并同步到历史消息。
  - **邀请链接丢失响应**：重启后恢复为 active，链接 `used` 仍为 1。
  - **定向邀请**：通知送达后，成员 Zone 的 `list_by_member.invitations` 和查找结果显示 invited 和群名；接受后条目消失。被撤销的邀请在接受失败后消失；过期的邀请不显示。
- [x] 在仓库根运行 `cargo test --manifest-path src/Cargo.toml -p msg_center -- --test-threads=1`。
- [x] 前端运行 `tsc`，以及 my-network、messagehub-group 两套 e2e 用例。

## 已知风险

- 收到的邀请来自邀请人签名的消息，群名和 invite_id 只是邀请人的声明，要到接受时才由宿主校验。MessageHub 邀请卡片现在也是这样。陌生人可以发送伪造的群邀请：它们进 REQUEST_BOX，同时进入群邀请列表，数量受每个 owner 256 条的上限约束。
- 宿主撤销邀请时不通知受邀人，所以列表里可能有已经失效的邀请，要等用户点接受、得到 revoked 后才消失。这是原有行为。
- 等待审批的远端成员资格由恢复循环每 30 秒查询一次宿主。这也是原有行为。

## 实施记录（2026-11-25）

### 落点

- **宿主**：[group_service.rs](../src/frame/msg_center/src/group_service.rs) 中，`request_group_join` 在 `join_group` 之后写申请 id；`invite_group_member` 的通知数据加 `profile`；远端转发列表加 `group.request_join`；`list_by_member` 合并 `received_group_invitations`。
- **成员 Zone**：[group_remote.rs](../src/frame/msg_center/src/group_remote.rs)
  - `PendingJoinRequest`、`ReceivedInvitation` 两个结构，以及 `check_remote_join_caller`（接受邀请和主动申请共用）。
  - `request_remote_group_join`。
  - `resume_group_binding` 先处理 `pending_request`，再走原来的邀请路径；邀请路径的终止判定和响应校验抽成 `is_final_join_answer` 和 `check_join_reply`，删除绑定抽成 `keep_or_drop_binding`。
  - 收到邀请的存取：`note_received_group_invitation`、`received_group_invitations`、`forget_received_invitation`，以及给查找用的 `remote_group_state`。
- **路由**：[group_sync.rs](../src/frame/msg_center/src/group_sync.rs) 新增 `put_query`。原来的 `put` 和 `fetch` 拼路径的方式不变。
- **入站**：[msg_center.rs](../src/frame/msg_center/src/msg_center.rs) 的 `dispatch_internal` 提交成功后，只对另一个 Zone 发来的投递调用 `note_received_group_invitation`。
- **表**：[groups.sql](../src/frame/msg_center/src/groups.sql) 新增 `group_received_invitations`。msg_box_db 每次打开都会执行 `CREATE TABLE IF NOT EXISTS`，已有库升级后自动建表。
- **查找**：[contact_lookup.rs](../src/frame/msg_center/src/contact_lookup.rs) 对他 Zone 的群填 `relation.group_state`。
- **前端**：
  - `JoinGroupDialog` 远端群也取群文档、显示加入按钮。
  - `realBackend.listGroupInvitations` 在没有群文档时用 `group_did` 和 `profile` 兜底，并带上 `role` 和 `received_at_ms`。
  - Mock 允许加入远端群。
  - 删掉 `network.joinGroup.remoteUnsupported` 和不再有来源的错误码 `remote-group-unsupported`，以及它们在 9 种语言里的文案。
  - [UI_DATAMODEL.md](../src/frame/desktop/src/components/network/UI_DATAMODEL.md) 同步更新。

### 实施中的取舍

- **哪些宿主答复会清掉一次申请。** 只有宿主不可达（`remote-group-unavailable`）时保留 `pending_request` 等待重发。其余答复都视为宿主的确定答复，包括格式不对的响应。这和接受邀请不同：接受邀请只在列出的几种终止原因时清除。原因是申请用的是同一个 key，重发只会拿回同样的结果。
- **申请回执的校验。** 状态为 active 的回执不要求带申请 id；pending 的回执必须带 id，否则无法跟进审批。旧版宿主返回 pending 时，成员 Zone 会报 `invalid-group-response` 并丢掉这次申请，所以两端都要升级。
- **已在群里时再次申请。** 本 Zone 不预先拦截，照常转给宿主，由宿主返回 `member-already-participating`，绑定保持原样。这样宿主始终是权威：本地绑定显示已加入、实际已被移除时，用户仍然可以重新申请。
- **收到邀请的写入时机。** 在投递事务提交之后另行写入，失败只记日志，不影响消息投递。
- **成员已有定向邀请时用 DID 申请。** 宿主会把裸 join 当成接受那份邀请，回执里的 `invitation_id` 是邀请 id。测试覆盖了这种情况，收到的邀请条目随之清除。

### 验证

- `cargo test --manifest-path src/Cargo.toml -p msg_center -- --test-threads=1`：270 通过，0 失败，2 忽略。
  - 新增 4 个双 Zone 用例，在 [test_group_remote.rs](../src/frame/msg_center/src/test_group_remote.rs) 末尾：`cross_zone_join_requests_follow_the_hosts_join_policy`、`cross_zone_invite_link_join_survives_a_lost_reply_without_using_the_link_twice`、`cross_zone_invitations_wait_with_the_members_invitations_until_answered`、`expired_cross_zone_invitations_are_not_listed`。
  - 原有用例 `invitation_retries_follow_current_membership_instead_of_saved_operations` 断言“重新申请后 `invitation_id` 为空”，与 D1 直接冲突。现在改为断言它是新的申请 id，且不等于之前的邀请 id。该用例原本要防的“旧邀请重放”仍由下一条断言覆盖。
- 前端：`tsc -b` 通过；改动文件 eslint 通过。
- e2e：`my-network.spec.ts` 与 `messagehub-group.spec.ts` 共 21 条，20 条通过。
  - “加入他 Zone 的群”一步原来断言“不支持”，已改为加入成功，群数量相应加一。
  - 失败的是 `messagehub-group.spec.ts:315`（message relations，超时）。它在 HEAD 的干净 worktree 上同样失败，属于基线失败，与本次改动无关。
- 部署要求：宿主和成员两端的 msg_center 都要升级，Desktop 前端也要重新构建。

### PVE 双 Zone DV（2026-11-25）

环境是本机 PVE 上的 `ssotest`：SN 为 VM 300；alice 为 VM 301，作群宿主；charlie 为 VM 303，作成员。没有用 bob，原因是 alice 的 Zone 里有一个 DID 与 bob 相同的账号 `bobfed`，发给 bob 的消息会在 alice 本地投递。

部署内容：
- alice 和 charlie 都换上本树的 msg_center（sha256 前缀 `88b121d1`），旧文件保存为 `msg_center.bak-760`。
- charlie 换上本树的 Desktop（`bin/control-panel/web`），旧目录保存为 `web.bak-760`。
- 没有用 `ssotest update`：它会推送整个 base 目录，覆盖 alice 上为 Agent 浏览器二期手工放入的 `nfs_server` 和 `web`。

脚本和结果：
- [cross_zone_join_dv.mjs](../test/test_msg_center/cross_zone_join_dv.mjs)：**27/27 通过**。全程不调用 `group.sync_joined`。
  - 批准约 9 秒后、拒绝约 12 秒后，由 charlie 的 Zone 通过后台跟进自行得知。
  - 拒绝之后又等了 45 秒，宿主上的记录仍是 rejected，没有被重新申请。
  - 邀请链接加入和定向邀请都能收到宿主之后发出的消息。
  - 定向邀请约 3 秒后进入 charlie 的列表，带群名；被撤销的邀请在接受失败后消失。
- [cross_zone_join_ui_dv.mjs](../test/test_msg_center/cross_zone_join_ui_dv.mjs)：**9/9 通过**，页面无报错。在 charlie 的 Desktop 上：
  - 用 DID 查找 alice 的群，预览显示“Hosted by another Zone”和“Anyone can join”，点 Join 后成功加入；
  - 「请求 → 群邀请」列出 alice 的邀请和群名，点 Join 后加入。
  - 第一次运行时页面报 `registration.waiting` 错误。原因是脚本照搬 #737 DV 的写法屏蔽了 Service Worker，`register()` 因此返回 undefined。改为允许 Service Worker 后不再报错，与本次改动无关。

观察到的现象：alice 是 charlie 的陌生人，向他发了多份群邀请后，「请求」里还出现了一条“alice sent 5 message(s)”消息请求。原有规则只在会话里只有一条邀请通知时才不算消息请求。本地群也是同一规则，这次没有改。
