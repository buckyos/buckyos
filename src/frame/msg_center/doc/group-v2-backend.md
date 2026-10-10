# Self-host Group v2 后台接口

本目录实现 [Self-Host-Groupv2.md](../../../../doc/message_hub/Self-Host-Groupv2.md) 的默认单 host 后台。持久化格式见 [group-v2-storage.md](group-v2-storage.md)。实现不恢复 v1 GroupMgr、subgroup、递归群展开或收益归属。

## 入口与认证

`main.rs` 在 `/kapi/msg-center` 将 `group.*` 请求交给 `group_service.rs`，群 CYFS 路径由 `group_http.rs` 处理。业务主体只取自经过验证的 token，禁止参数 `actor_did`、`host_owner`。群创建还检查 Zone RBAC 的 `obj://msg-center/group` / `create`；默认策略对 `users` 和 `admin` 都授予该权限（2026-10-01），群内操作不再经过 RBAC，由成员资格、角色与能力控制。

RPC 使用现有 kRPC 请求封装和 token。所有方法的参数是对象；除 create/list_by_member 外必须含 `group_did`。默认 Session 的 `session_id` 省略或为 null；具名 Session 使用未编码的原始字符串。对外的 `session` 和 `state_ref.session_key` 始终是规范 MailboxAddress。

角色、状态和规则枚举采用 snake_case，例如 `member`、`pending_admin_approval`、`explicit`、`from_join`。带值枚举是 `{"roles":["owner","admin"]}`、`{"only":["admin"]}`，无值规则是 `"inherit"`、`"all_participants"`、`"nobody"`。Owner 的固定能力不由配置撤销；Admin/Member 使用配置中的 roles 映射。

## 管理 RPC

| 方法 | 主要参数/结果 |
|---|---|
| group.create | profile?、configuration?、sessions?、invitations?、idempotency_key；group_id?、group_did?；Owner 记录直接以 token 认证的调用者写入；返回群 Doc 和默认 Session |
| group.get_doc / get_config | 返回公开 Doc / 原始完整配置 |
| group.apply_config | expected_revision、patch、idempotency_key；返回新 revision，未知配置字段保留 |
| group.archive / delete | 仅 Owner；delete 留不可复用墓碑 |
| group.transfer_owner | member_did；仅 DID controller，目标需 Active；发起两步转让，返回 transfer_id、member_did、expires_at_ms，目标成员收到 owner_transfer 通知；不自动接受 |
| group.accept_owner_transfer | transfer_id；由目标成员调用，接受后返回 owner；无对应待接受转让或已过期返回 transfer-mismatch |
| group.cancel_owner_transfer | 由群主调用，取消待接受的转让 |
| group.invite_member | member_did、role?、expires_at_ms?；记录 invited_by；返回 invite_id、member_did、expires_at_ms、state（invited / active / pending_admin_approval，后两者表示同 Zone 成员一方已自动接受） |
| group.revoke_invite | member_did |
| group.create_invite_link | expires_at_ms?、max_uses?、require_approval?；返回 token |
| group.revoke_invite_link | token |
| group.accept_invitation | invitation_id（必填，防止误接受过期后重新发出的另一份邀请）、member_did?（仅 Agent 的 owner 代为接受时传 Agent DID，否则 agent-owner-required；Agent 的 `allow_group` 关闭时返回 agent_group_disabled）、attestation?；接受即接受邀请中的角色；邀请人在接受时仍有 group.approve_member 能力则直接 active，否则 pending_admin_approval；不匹配返回 invitation-mismatch |
| group.request_join | invite?、attestation?；只授予 member；join_policy open 直接 active，request_and_approve 进入 pending_admin_approval，invite_only 拒绝 invite-required；邀请链接直接 active，除非 require_approval |
| group.approve_member / reject_member | member_did；审批仅推进 PendingAdminApproval |
| group.leave | 操作者本人退出，Owner 需先转让 |
| group.remove_member | member_did；同时生成个人通知 |
| group.update_member_role | member_did、role（admin / member）；不再要求本人同意 |
| group.moderate | member_did、blocked?、muted_until_ms?；null 清除禁言期限 |
| group.list_members | 按配置的成员列表可见性授权，Guest 不可调用 |
| group.create_session | session_id?、template?、membership?、rule_overrides?、members?、title?/description?/announcement? |
| group.update_session | session_id、expected_revision、template?、membership?、rule_overrides?、add_members? |
| group.archive_session / delete_session | session_id、expected_revision；默认 Session 不可删除 |
| group.list_sessions | 返回读者可访问的 Session、共享状态、state_ref、has_guests、规则和 revision |
| group.list_session_members | session_id?；读者须能读取该 Session。能看群成员列表的成员（或有 group.read_all）得到全部有效参与者，complete=true；Guest 或成员列表对其隐藏的成员只得到显式参与者（Included 记录）、在该 Session 发过言者与本人，complete=false（v2 §3.3）。条目为 member_did、kind（group_member / guest）、role、entity_kind、state；有 session.invite_guest 的读者另见 state=invited 的待接受 Guest。结果带 Session 记录 revision（默认 Session 为配置 revision），供 remove_session_member 的 expected_revision |
| group.invite_session_guest | session_id、member_did |
| group.accept_session_invitation | session_id、attestation?；Guest 接受 Session 邀请 |
| group.submit_guest_request | request_id、attestation?；按 guest_entry 模板原子创建，重复请求复用结果 |
| group.remove_session_member | session_id、member_did |
| group.leave_session | session_id；仅退出本人参与关系 |
| group.list_by_member | 返回操作者参与的 Hosted Group 和本地 Joined Group |
| group.check_access | action、session_id?；返回 allowed 和 reason |
| group.list_events | after_seq?、limit?；需 group.read_all，管理查询包含审计 |
| group.update_read_marker | session_id?、last_read_seq；仅推进本人水位 |

上述写操作统一可以带 idempotency_key；相同主体、方法、键重试返回同一结果，改变请求则拒绝。配置与状态 patch 必须带幂等键及 expected_revision。Session 修改必须带 expected_revision。

后台和成员列表查询会将已过期的邀请持久化为 expired，并且只发布一次 entity.invite_expired。重新邀请产生新的邀请 ID；accept_invitation 必须带当前邀请的 invitation_id，否则返回 invitation-mismatch。

附加查询：`group.list_messages(session_id?,after_seq?,limit?)`、`group.changes(since?,limit?)`、`group.get_read_markers(session_id?,session_seq)`、`group.sync_joined(group_did)`。权威共享/成员状态使用 `group.get_shared_state`、`group.get_member_state(member_did?)`、`group.update_shared_state`、`group.update_member_state`；更新参数为 session_id?、expected_revision、set、unset、idempotency_key。成员仅能修改本人昵称，业务权限不能从共享状态 patch 改写。

## 同意与消息

入群不要求成员签名的 proof（2026-10-01 取消）。是否入群只看群一方和成员一方是否同意：

- 群一方：Owner / Admin（接受时邀请人仍具有 `group.approve_member` 能力）发出的邀请视为群已同意，接受后直接 `active`；普通成员发出的邀请在对方接受后进入 `pending_admin_approval`；主动申请按 `join_policy`（open → active，request_and_approve → pending_admin_approval，invite_only → invite-required）；邀请链接视为事先批准，直接 active，除非 `require_approval`。
- 成员一方：同 Zone 用户由 msg-center 在投递邀请时查接收者作用域的 Contact Mgr：邀请人是好友（target_box = INBOX）自动接受；陌生人（REQUEST_BOX）进入 REQUEST_BOX 等本人接受；被屏蔽（DROP）不投递、不接受，邀请保持 `invited` 直到过期。Agent 的 `settings.allow_group` 关闭时，任何人邀请它或它（包括 owner 代为）接受、申请加入都返回 agent_group_disabled；打开时只自动接受其 owner（`AgentSpec.agent_doc.owner`）发出的邀请，其他邀请投递给 owner 确认，owner 用 `group.accept_invitation` 带 `member_did` 代为接受。跨 Zone 成员的自动接受保留 TODO。
- 角色由邀请决定，接受即接受邀请中的角色；申请和邀请链接只得到 member。群主可直接 `update_member_role` 升为 admin，不需要本人同意；转让群主为两步（transfer_owner → accept_owner_transfer）。
- Session Guest 同样只看双方同意（`accept_session_invitation`、`submit_guest_request`）。

成员实体类型 `entity_kind` 在邀请、接受或建群时解析成员 DID 得到：msgtunnel → user，did:dev → device，本 Zone Agent → agent，其它解析 DID Document；解析不到为 unknown。不接受群作为嵌套成员（member-must-be-single-entity）。

外部平台（tunnel）用户的同意由 tunnel 的 transport 身份以独立参数 `attestation` 提交：`{ "member_did": "did:msgtunnel:...", "source_event": { "event_id": "...", "user_consent": true } }`。校验调用者不是远端请求、且是该 tunnel 实例登记的 transport DID，否则返回 `invalid-tunnel-attestation`；通过后操作者替换为 `member_did`。

已删除的错误码：owner-proof-required、member-proof-required、signed-member-proof-required、proof-signer-mismatch、proof-payload-mismatch、invalid-proof-type、invalid-proof-lifetime、proof-scope-mismatch、proof-replayed、proof-key-unavailable、owner-proof-role-required、proof-invitation-mismatch、proof-role-mismatch、role-consent-required、owner-consent-required。新增：invitation-mismatch、transfer-mismatch、owner-transfer-pending、agent-owner-required、agent_group_disabled。

通知（kind = operation，machine.intent = buckyos.group_invitation）：`invite` 的 data 带 `state`（active 表示已自动加入，pending_admin_approval 表示已接受待审批，invited 表示等待本人接受）和代为接受时的 `member_did`；`pending_approval` 发给有 group.approve_member 能力的成员，带 `member_did`、`invited_by?`；新增 `owner_transfer`（`transfer_id`、`expires_at_ms`）发给转让目标。

消息保持 `from=认证主体`、`to=[group_did]`、`kind=group_msg`。`to_session` 决定 Session，topic 不参与路由。host 接受消息时分配序号，并保存原始 MsgObject；客户端 created_at_ms 不决定排序。编辑、撤回、回应和 @all 在关系目标、作者、时间窗、能力、成员资格和限流校验后执行。

## CYFS HTTP

提供设计文档 §11.1 的 inbox、sessions、join、guest_requests、read_markers、changes 和 objects 路径。Session ID 按 MailboxAddress 规则编码为单个段，允许编码后的中文、空格和斜杠。没有 `/@/` 形式的 inner path。

请求使用 host 能验证的 Bearer token，`cyfs-original-user` 必须与 token 主体匹配；不再有 `cyfs-proofs` 头，读取、发言只看认证主体与成员表 / Guest 记录。`join[?invite=]` 的 body 为 `{ "invitation_id"?: "..." }` 或空：带 invitation_id，或省略但调用者有待接受邀请时按 `group.accept_invitation` 处理，否则按 `group.request_join`；`sessions/<sid>/join` 的 body 为空或 `{}`，对应 `group.accept_session_invitation`；`guest_requests` 的 body 为 `{ "request_id": "..." }`。删除群后曾参与者按 DID（tombstone_readers）读到删除通知。客户端身份跨 Zone 尚无标准证明，因此 `allowed_clients=Only` 默认拒绝远端访问；只有群配置显式开启 allow_unverified_remote 才放行。

inbox PUT 的 body 为 canonical MsgObject JSON 或 `application/cyfs-named-object+jwt`，携带 cyfs-obj-id；JWT 必须验证成功，委托签名也要校验 from 的 DID Document 中的 authentication 授权。成功返回标准 CYFS accepted 状态和 cyfs-session-seq，终局拒绝可查询和幂等重试。JWT 对象读取原样返回签名原文，不降级 JSON。

inbox GET 使用 after_seq/limit，返回 seq、obj_id、redacted；默认实现还返回 accepted_at_ms 供本地投影使用 host 排序。changes 使用 reader-bound opaque token，不暴露 group_seq；未知/过期 token 返回 limited。退群、移除、Session 删除和群删除保留当事人的最小通知，不能借此读取消息正文。

read_markers 的标准 body 为 `{ "session": "群的规范 MailboxAddress", "last_read_seq": 42 }`。默认实现也接受原始 session_id/null。附件必须携带 `context_path=<规范群 Session 地址>/<消息 ObjectId>`；host 校验该消息可读且实际引用了目标对象，再交给 NamedStore/CYFS 获取。

公开资料有额外 GET `/<group_did>/doc`（ObjectId+Doc）和 `/<group_did>/did.json`（Doc 内容）入口。GroupDoc 来源于配置，包含 group DID context 和 iat，不作为规则来源。创建、公开配置/Owner 改变、归档和删除均在本地事务中登记发布待办；后台通过 system-config 原子发布 Zone resolver 的 group 类型文档或 tombstoned 状态，失败后重试。

## Joined Group 配置与运行

远端 host 的路由和用户访问凭据通过现有服务 Settings 登记；没有添加依赖或额外进程：

```json
{
  "cyfs_dispatch": {
    "target_zone": "host.example",
    "joined_groups": [{
      "owner_did": "did:web:alice.example",
      "group_did": "did:web:g.remote.example",
      "host": "remote.example",
      "upstream": "https://remote.example/",
      "authorization": "Bearer <host可验证的用户凭据>"
    }]
  }
}
```

Settings 的主机/凭据要与已建立的成员关系一致；凭据更新后用现有 reload_settings 刷新。后台每 30 秒拉取一次，也可调用 group.sync_joined。同步 Session 列表、变更流和必要正文后，在一个 RDB 事务中保存本地 INBOX、副本、Session 登记和水位。未知 token 按各 Session 的 session_seq 补同步。同步跳过本人发出的消息；已获取的副本继续保留。

`post_send` 根据该发送者的 Joined Group binding 建立命名 Session 投递快照，Message Hub 携带该用户凭据和 original-user 投递。host 本地成员直接投影（发送者本人不投影到自己的 INBOX，只有 SENT 记录），外部原生成员通过 pull 获取；tunnel 成员复用既有 DELIVERY_QUEUE。

Hook 使用 JSON HTTP POST，body 仅有 operation、actor、group_did、session_id，post 另带 msg。runtime 服务 token 或显式 `cyfs_dispatch.group_hook_authorization` 用于服务身份认证。Hook 拒绝是终局；Deny 不可用可重试且不保存消息结果，NativeOnly 只保留原生权限。read 允许缓存按群/Session revision、读者、客户端和 Hook 绑定隔离。

## 部署边界与验证

本实现通过现有 Zone resolver 的控制面 cache 发布 group 文档，不修改内核 resolver。部署时需让 Gateway 将群的协议路径转发给 msg-center，并授予其既有 kernel 服务身份 resolver/cache 写权限；自备一级 DID 必须已经发布授权本 host 的文档。Joined Group 路由使用显式绑定，暂未增加自动 DID endpoint 发现或用户凭据续期（跨 Zone 认证沿用 DID Document 语义与 Contact Mgr 信任，自动建立 joined 绑定与凭据续期保留 TODO）。

RBAC 默认策略已把 `obj://msg-center/group` 的 `create` 授予 `users` 和 `admin`，并把 `users` / `admin` 对 `obj://msg-center/group_inbox/*` 收窄为只读。默认策略编译进各服务（system-config 的 `system/rbac/policy` 只保存尾部追加规则），因此 msg-center 和 sys_config_service 必须重新构建并重新部署才能生效。

群事务提交后，为每条成员 INBOX 投影记录发出常规 `box_changed` kevent（与普通投递相同），前端无需订阅群事件；发送者本人的群消息不再投影到本人 INBOX，只有 SENT 记录（session_id 为群的规范 MailboxAddress）。

群基础状态、权限、序号、幂等、Guest、关系消息、签名原文、HTTP pull/发送、DID 发布重试、重启恢复和 Hook 故障策略均有模块测试。真实 Gateway/Zone resolver、跨 Zone token 签发、PostgreSQL、多副本、Telegram 平台关系消息映射和满容量性能需要部署联调；变更提示目前是本 Zone 不含内容的 kevent（群变更事件与成员投影的 box_changed），远端依赖轮询。
