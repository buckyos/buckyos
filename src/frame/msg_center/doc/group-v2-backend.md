# Self-host Group v2 后台接口

本目录实现 [Self-Host-Groupv2.md](../../../../doc/message_hub/Self-Host-Groupv2.md) 的默认单 host 后台。持久化格式见 [group-v2-storage.md](group-v2-storage.md)。实现不恢复 v1 GroupMgr、subgroup、递归群展开或收益归属。

## 入口与认证

`main.rs` 在 `/kapi/msg-center` 将 `group.*` 请求交给 `group_service.rs`，群 CYFS 路径由 `group_http.rs` 处理。kRPC 业务主体取自经过验证的 token，跨 Zone HTTP 请求使用目标 Zone 会话；禁止参数 `actor_did`、`host_owner`。App 自己的服务会话（`principal_kind=app`，`sub` 为 owner）在 `group.*` 中以其 owner 的 DID 行事，`appid` 作为 `allowed_clients` 判定的客户端（2026-10-24，App 代登录预置邀请用）。群创建还检查 Zone RBAC 的 `obj://msg-center/group` / `create`；默认策略对 `users` 和 `admin` 都授予该权限（2026-10-01），群内操作不再经过 RBAC，由成员资格、角色与能力控制。

RPC 使用现有 kRPC 请求封装和 token。所有方法的参数是对象；除 create/list_by_member 外必须含 `group_did`。默认 Session 的 `session_id` 省略或为 null；具名 Session 使用未编码的原始字符串。对外的 `session` 和 `state_ref.session_key` 始终是规范 MailboxAddress。

角色、状态和规则枚举采用 snake_case，例如 `member`、`pending_admin_approval`、`explicit`、`from_join`。带值枚举是 `{"roles":["owner","admin"]}`、`{"only":["admin"]}`，无值规则是 `"inherit"`、`"all_participants"`、`"nobody"`。Owner 的固定能力不由配置撤销；Admin/Member 使用配置中的 roles 映射。

## 管理 RPC

| 方法 | 主要参数/结果 |
|---|---|
| group.create | profile?、configuration?、sessions?、invitations?、idempotency_key；group_id?、group_did?；Owner 记录直接以 token 认证的调用者写入；返回群 Doc 和默认 Session |
| group.get_doc / get_config | 返回公开 Doc（不含面板）/ 完整配置（Hook `secret` 已去除） |
| group.apply_config | expected_revision、patch、idempotency_key；返回新 revision，未知配置字段保留。`extensions.panels`（面板声明，整体替换）、`display`、`rules.display` 在此校验：面板 `entry_url` 必须是群 host Zone 的 App 子域（`invalid-panel-decl:*`），`rules.panels` 只能引用已声明 id（`unknown-panel:<id>`），`display.featured_members` 须是活跃成员（`invalid-display-member`）。不带 `secret` 的 Hook 绑定由服务生成 32 字节随机 secret，只在本次响应的 `hook_secrets: [{scope, index, secret}]` 中出现一次 |
| group.rotate_hook_secret | session_id?、index；默认 Session / 模板 Hook 需 group.update_config，Session 覆盖项 Hook 需 session.manage；返回 `{revision, secret}` |
| group.archive / delete | 仅 Owner；delete 留不可复用墓碑 |
| group.transfer_owner | member_did；仅 DID controller，目标需 Active 且在 host Zone（否则 owner-must-be-in-the-host-zone）；发起两步转让，返回 transfer_id、member_did、expires_at_ms，目标成员收到 owner_transfer 通知；不自动接受 |
| group.accept_owner_transfer | transfer_id；由 host Zone 的目标成员调用，接受后返回 owner；无对应待接受转让或已过期返回 transfer-mismatch |
| group.cancel_owner_transfer | 由群主调用，取消待接受的转让 |
| group.invite_member | member_did、role?、expires_at_ms?；记录 invited_by；返回 invite_id、member_did、expires_at_ms、state（invited / active / pending_admin_approval，后两者表示同 Zone 成员一方已自动接受） |
| group.revoke_invite | member_did |
| group.create_invite_link | expires_at_ms?、max_uses?、require_approval?；返回 token |
| group.revoke_invite_link | token |
| group.accept_invitation | invitation_id（必填，防止误接受过期后重新发出的另一份邀请）、member_did?（仅 Agent 的 owner 代为接受时传 Agent DID，否则 agent-owner-required；Agent 的 `allow_group` 关闭时返回 agent_group_disabled）、attestation?；接受即接受邀请中的角色；邀请人在接受时仍有 group.approve_member 能力则直接 active，否则 pending_admin_approval。当前成员仍持有同一 invitation_id 且为 active / pending_admin_approval 时重试返回当前成员状态；过期、撤销、重签发或退出后不能用旧邀请重新加入。本地未托管的群由本 Zone msg-center 解析宿主并转发；宿主已接受但首次同步未完成时返回 sync_pending=true |
| group.request_join | invite?、attestation?；只授予 member；join_policy open 直接 active，request_and_approve 进入 pending_admin_approval，invite_only 拒绝 invite-required；邀请链接直接 active，除非 require_approval。成员记录的 invitation_id 是这次申请新生成的 id，成员 Zone 用它跟进结果；群在他 Zone 时由本 Zone 转发（见 Joined Group 一节） |
| group.approve_member / reject_member | member_did；审批仅推进 PendingAdminApproval |
| group.leave | 操作者本人退出，Owner 需先转让 |
| group.remove_member | member_did；同时生成个人通知 |
| group.update_member_role | member_did、role（admin / member）；不再要求本人同意 |
| group.moderate | member_did、blocked?、muted_until_ms?；null 清除禁言期限 |
| group.list_members | 按配置的成员列表可见性授权，Guest 不可调用 |
| group.create_session | session_id?、template?、membership?、rule_overrides?、members?、title?/description?/announcement?；`rule_overrides.hooks` 缺 secret 时生成并随 `hook_secrets` 返回一次 |
| group.update_session | session_id、expected_revision、template?、membership?、rule_overrides?、add_members?；同上 |
| group.archive_session / delete_session | session_id、expected_revision；默认 Session 不可删除 |
| group.list_sessions | 返回读者可访问的 Session、共享状态、state_ref、has_guests、规则（Hook secret 已去除）、`panels`（生效面板 `{id,title,entry_url}`：`scope` 覆盖 ∪ 规则引用）、`display`（Session 覆盖群）和 revision |
| group.list_session_members | session_id?；读者须能读取该 Session。能看群成员列表的成员（或有 group.read_all）得到全部有效参与者，complete=true；Guest 或成员列表对其隐藏的成员只得到显式参与者（Included 记录）、在该 Session 发过言者与本人，complete=false（v2 §3.3）。条目为 member_did、kind（group_member / guest）、role、entity_kind、state；有 session.invite_guest 的读者另见 state=invited 的待接受 Guest。结果带 Session 记录 revision（默认 Session 为配置 revision），供 remove_session_member 的 expected_revision |
| group.invite_session_guest | session_id、member_did |
| group.accept_session_invitation | session_id、attestation?；Guest 接受 Session 邀请 |
| group.submit_guest_request | request_id、attestation?；按 guest_entry 模板原子创建，重复请求复用结果 |
| group.remove_session_member | session_id、member_did |
| group.leave_session | session_id；仅退出本人参与关系 |
| group.list_by_member | 返回操作者参与的 Hosted Group 和本地 Joined Group |
| group.check_access | action、session_id?；返回 allowed（`true` / `false`，`session.post` 在配置了 `may_grant` 的 post Hook 时为 `"hook_dependent"`）和 reason |
| group.list_events | after_seq?、limit?；需 group.read_all，管理查询包含审计 |
| group.update_read_marker | session_id?、last_read_seq；仅推进本人水位 |

上述写操作可以带 idempotency_key；除 `group.accept_invitation` 外，相同主体、方法、键重试返回同一结果，改变请求则拒绝。`group.accept_invitation` 按认证成员和 invitation_id 检查当前成员状态，不重放可能早于撤权的通用 operations 缓存；同一邀请从 pending_admin_approval 变为 active 后重试返回 active。配置与状态 patch 必须带幂等键及 expected_revision。Session 修改必须带 expected_revision。

后台和成员列表查询会将已过期的邀请持久化为 expired，并且只发布一次 entity.invite_expired。重新邀请产生新的邀请 ID；accept_invitation 必须带当前邀请的 invitation_id，否则返回 invitation-mismatch。尚未接受的过期、撤销邀请分别返回 invitation-expired、invitation-revoked；已成功接受的同一邀请不再受原邀请截止时间影响，但仍须满足当前群、成员和客户端访问策略。

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

通知（kind = operation，machine.intent = buckyos.group_invitation）：`invite` 的 data 带 `state`（active 表示已自动加入，pending_admin_approval 表示已接受待审批，invited 表示等待本人接受）和代为接受时的 `member_did`；`pending_approval` 发给有 group.approve_member 能力的成员，带 `member_did`、`invited_by?`；新增 `owner_transfer`（`transfer_id`、`expires_at_ms`）发给转让目标。`invite`、`session_invite` 的 `from` 是邀请人，按接收者对邀请人的联系人策略投递到 `dm:<inviter>`；其他通知（`pending_approval`、`rejected`、`removed`、`session_removed`、`owner_transfer`）的 `from` 是群，`machine.data.by` 是操作者，`to_session` 是群的默认 Session，只有接收者屏蔽了这个群才丢弃（Self-Host-Groupv2 §5.4）。

消息保持 `from=认证主体`、`to=[group_did]`、`kind=group_msg`。`to_session` 决定 Session，topic 不参与路由。host 接受消息时分配序号，并保存原始 MsgObject；客户端 created_at_ms 不决定排序。编辑、撤回、回应和 @all 在关系目标、作者、时间窗、能力、成员资格和限流校验后执行。

## CYFS HTTP

提供设计文档 §11.1 的 inbox、sessions、join、guest_requests、read_markers、changes 和 objects 路径。Session ID 按 MailboxAddress 规则编码为单个段，允许编码后的中文、空格和斜杠。没有 `/@/` 形式的 inner path。

群路径支持 `/<group_did>/...` 和 `/kapi/msg-center/groups/<group_did>/...`；自动跨 Zone 请求使用后者，经过现有 Gateway 的 service 转发，不要求另外配置根路径路由。前缀内合法群 DID 不存在时返回 404 / remote-group-not-found。

自动跨 Zone 请求通过 `CrossZoneClient` 使用设备签名断言向目标 verify-hub 登录，再携带目标 msg-center 的 Bearer 会话。拉取使用成员 DID，推送使用群宿主 Zone DID。显式配置的拉取路由也可使用宿主签发的 Bearer。身份取自已验证的会话。

远端 actor 不携带可信客户端身份，因此 `allowed_clients=Only` 默认拒绝远端访问；只有群配置显式开启 allow_unverified_remote 才放行。`join[?invite=]` 的 body 可带 invitation_id、member_did、idempotency_key；显式 invitation_id 接受该邀请，否则存在本人待接受邀请时按其处理，再否则按 group.request_join。对申请的 id 发 `{invitation_id}` 只返回当前记录、不改状态：审批通过后是 active，被拒绝或移除后是 invitation-mismatch。`sessions/<sid>/join` 对应 group.accept_session_invitation；`guest_requests` 带 request_id。删除群后曾参与者按 DID（tombstone_readers）读到删除通知。

inbox PUT 的 body 为 canonical MsgObject JSON 或 `application/cyfs-named-object+jwt`，携带 cyfs-obj-id；JWT 必须验证成功，委托签名按 from 的 DID Document authentication 或 G3 Zone custody 授权校验。成功返回标准 CYFS accepted 状态和 cyfs-session-seq，终局拒绝可查询和幂等重试。JWT 对象读取原样返回签名原文，不降级 JSON。

inbox GET 使用 after_seq/limit，返回 seq、obj_id、redacted；默认实现还返回 accepted_at_ms 供本地投影使用 host 排序。changes 使用 reader-bound opaque token，不暴露 group_seq；未知/过期 token 返回 limited。响应的 member 为当前调用者的 `{state, epoch}` 或 null，供重入同步区分历史移除事件与当前成员状态。退群、移除、Session 删除和群删除保留当事人的最小通知，不能借此读取消息正文。

read_markers 的标准 body 为 `{ "session": "群的规范 MailboxAddress", "last_read_seq": 42 }`。默认实现也接受原始 session_id/null。附件必须携带 `context_path=<规范群 Session 地址>/<消息 ObjectId>`；host 校验该消息可读、实际引用了目标对象，且该消息在 `zone/msg-center/_/attachments` 的绑定（`<session>/<msg_id>/<n>`）持有它，再从 NamedStore 读取。远端成员的附件没有绑定时，host 以群 DID 向作者 Zone 取回（接受消息后后台进行，读取和票据请求也会触发）；其他 Zone 的成员经 `msg.attachment_ticket {obj_id, context_path}` 取令牌再读 host 的 `/ndn/`，规则相同，见 Message Center 附件一节。跨 Zone 断言登录的群成员可以发带附件的消息（host 不绑定，由上述流程取回）；访客和 Session Guest 不行。

公开资料有额外 GET `/<group_did>/doc`（ObjectId+Doc）和 `/<group_did>/did.json`（Doc 内容）入口。GroupDoc 来源于配置，包含 group DID context 和 iat，不作为规则来源。创建、公开配置/Owner 改变、归档和删除均在本地事务中登记发布待办；后台通过 system-config 原子发布 Zone resolver 的 did.json 文档（实际类型 group，由本 Zone OOD 设备 key 签成 JWT）或 tombstoned 状态，失败后重试。did:bns 下普通来源要求签名，远端按 owner → OOD 设备文档 → 群文档的 Zone 托管链验证。

## Joined Group 配置与运行

用户接受跨 Zone 邀请时，本地 msg-center 从群 DID Document 的 host 解析宿主。来源须为本 Zone 非 External、非 restricted 的已认证主体；代表 Agent 接受时还校验本 Zone 的 owner 关系及 allow_group。请求发出前持久化 owner/group/host/待确认 invitation_id，成功后保存宿主成员记录并同步；网络失败、成功响应丢失或进程重启后，后台从该登记恢复同一次接受。宿主进入 pending_admin_approval 时保留待审批展示，获批后再登记可读 Session；active 成员首次同步失败时返回 sync_pending=true，后台继续恢复。

主动申请（`group.request_join`，可带邀请链接 token）的来源要求相同。请求发出前在绑定里持久化 `pending_request { idempotency_key, invite? }`，再 `PUT join[?invite=] {idempotency_key}`。宿主的答复（接受、拒绝、需要邀请、已在群中、响应格式错误）都会清掉这次申请，没有成员记录时删除绑定；宿主不可达时保留，恢复循环用同一个 key 重发，宿主返回保存的结果，邀请链接不会多用一次。之后按申请 id 跟进，与接受邀请相同。上一次接受或申请还没得到答复时，先完成它。

他 Zone 群的定向邀请只以邀请人发来的 `buckyos.group_invitation` / `invite` 通知到达。另一个 Zone 发来、已投进本地收件人信箱的这类通知，若群不在本 Zone 托管，记入 `group_received_invitations`（owner + group 一条，后到的覆盖，每个 owner 最多 256 条），保存 invitation_id、邀请人、role、过期时间和通知里的群 profile。`group.list_by_member` 把其中未过期、尚未加入或申请的条目并进 `invitations`：`member.state = invited`、`doc = null`，带 `group_did` 和 `profile`。接受或申请成功、宿主答复邀请不匹配 / 过期 / 已撤销时删除；过期条目在列出时清理。这些字段是邀请人的声明，宿主在接受时校验；宿主撤销邀请不通知受邀人。`contact.lookup` 对他 Zone 的群按这里的登记给出 `group_state`（active / pending_admin_approval / invited）。

自动登记保存在 `group_joined_bindings`，不存用户 token 或 OOD 私钥；跨 Zone 客户端使用当前 Zone signer 登录并复用有时限的目标会话。`upstream` 来自现有 Zone origin 解析，默认 HTTPS；测试或专用网络可用服务 Settings 的 zone_origins 映射。已有部署也可以显式配置 Joined Group：

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

显式 Settings 的主机/凭据要与已建立的成员关系一致；有 authorization 时使用配置的 Bearer 凭据和根路径，省略时使用跨 Zone 会话和 kapi 前缀。显式 Bearer 凭据更新后用现有 reload_settings 刷新。补偿调度独立于入群恢复、DID 发布、清理和待上报的已读水位。已登记和显式配置的绑定均有独立的补偿调度，正常周期按绑定稳定错开在 13.5 至 15 分钟之间，首次建立基线可以立即同步。每个绑定分别退避；分页和投影按 `(owner_did, group_did)` 协调，可手动调用 `group.sync_joined`。满 100 项继续取下一页，未知或过期 token 另按各 Session 的 `session_seq` 补历史。

### 宿主向成员推送

宿主根据读者 DID 的公开托管关系解析目标 Zone，再读取 Zone 文档的主机名。实际发送前重新核对。读者在宿主上的联邦登录别名可以保留本地 INBOX，同时其托管 OOD 也应收到推送。

宿主提交后，先用 80 毫秒合并短时间内的群变更，再按群和目标 Zone 排队。群发现队列和目标队列各保留最多 256 个键，每个目标键最多积累 1024 个读者。全局最多 4 个发送任务，同一 Zone 跨群串行发送，单次请求含登录、重试及回执的时限为 15 秒。对端接收入口为 `POST /kapi/msg-center/groups/<group_did>/updates`，Content-Type 为 `application/json`。每个请求最多 32 个读者、256 KiB；单个消息原文上限 64 KiB。接收端校验请求目标为本 Zone，断言会话的主体为本地 Joined Group 绑定的宿主，并逐个匹配本地读者。无正文提示可省略 `delta`。

```json
{
  "entries": [
    {
      "reader_did": "did:web:alice.member.example",
      "member_epoch": 2,
      "delta": {
        "previous_token": "<此读者已拉取或确认的 token>",
        "next_token": "<可直接用于 changes 的 token>",
        "change": {
          "kind": "message",
          "session": "did:web:team.host.example",
          "session_id": null,
          "message": {"seq": 17, "obj_id": "<ObjId>", "redacted": false, "accepted_at_ms": 1795000000000}
        },
        "content_type": "application/cyfs-named-object+json",
        "object": "<原始 MsgObject JSON 字符串>"
      }
    }
  ]
}
```

前驱和后继 token 各为 1 至 256 字节，值必须不同。两者包围该读者整条变更流中唯一可直接投影的消息。积压多项、基线未知、Guest 缺少群成员周期或无法判定正文权限时使用提示。跨 Zone 登录成功后才构建正文，每次更换传输方式或刷新登录后重试都重新按该读者实际权限检查 Session、历史区间、客户端策略、read Hook 和撤回。签名对象的 `content_type` 为 `application/cyfs-named-object+jwt`，`object` 保存 JWT 原文。

成员 OOD 在本地已提交游标等于前驱时校验对象并直接投影，保留宿主排序和本人仅写 SENT 的语义。对象、副本、INBOX 和新游标在同一事务提交，随后发出 `box_changed`。响应示例为 `{"committed":[{"reader_did":"did:web:alice.member.example","next_token":"<已入库 token>"}]}`。当前后继可重复确认，最近 32 个已提交的推送游标可直接识别重复投递。更早的重放按读者、群、成员周期、Session、序号和 ObjId 核对已提交投影，匹配后保留当前水位和已读状态。缺正文或遇到未知前驱时唤醒补拉。绑定、周期或对象校验失败的条目不确认。

`post_send` 根据该发送者的 Joined Group binding 建立命名 Session 投递快照，Message Hub 使用跨 Zone 会话或显式 Bearer 凭据投递。自动登记须为 active，收到移除、退出或删除变更后停止同步和发送；新邀请接受后按新 epoch 恢复。host 本地成员直接投影（发送者本人不投影到自己的 INBOX，只有 SENT 记录），外部原生成员通过推送或增量拉取入库；tunnel 成员复用既有 DELIVERY_QUEUE。

远端暂时不可达或签名公钥解析暂时失败返回 remote-group-unavailable；身份验证失败返回 remote-group-auth-failed；群不存在返回 remote-group-not-found。邀请过期、撤销、标识不匹配和权限拒绝保留宿主的明确原因，不统一解释为群不存在。

Hook 使用 JSON HTTP POST，body 仅有 operation、actor、group_did、group_host、session_id，post 另带 msg。出站不携带任何 BuckyOS 服务 token：请求带 `X-BuckyOS-Hook-Timestamp`（Unix 秒）与 `X-BuckyOS-Hook-Signature`（hex `HMAC-SHA256(binding.secret, "<timestamp>." + body)`），App 用绑定的 secret 验签（`buckyos_api::verify_group_hook_signature`，允许 5 分钟偏差）；secret 为空的绑定按不可用处理。`cyfs_dispatch.group_hook_authorization` 已删除。Hook 拒绝是终局；Deny 不可用可重试且不保存消息结果，NativeOnly 只保留原生权限。read 允许缓存按群/Session revision、读者、客户端和 Hook 绑定隔离。

## 部署边界与验证

本实现通过 Zone resolver 控制面 cache 的 did.json 槽位发布 group 文档；查找方对 Zone 子名字先读 did.json，通常一次查询即得到群文档。自动群请求经现有 Gateway 的 `/kapi/msg-center/*` 路由；根路径仍用于显式配置或自定义路由。msg-center 需要既有 kernel 服务身份的 resolver/cache 写权限；自备一级 DID 必须已经发布授权本 host 的文档。自动跨 Zone 接受与同步需要本 Zone OOD signer 以及可解析的群、成员和签名设备 DID 文档，沿用原生消息的 Zone custody 信任模型；无需新增登录委托权限或持久用户凭据。远端主动申请加入、邀请链接和其它群管理操作的自动路由不在本次范围内。

RBAC 默认策略已把 `obj://msg-center/group` 的 `create` 授予 `users` 和 `admin`，并把 `users` / `admin` 对 `obj://msg-center/group_inbox/*` 收窄为只读。默认策略编译进各服务（system-config 的 `system/rbac/policy` 只保存尾部追加规则），因此 msg-center 和 sys_config_service 必须重新构建并重新部署才能生效。

群事务提交后，为每条成员 INBOX 投影记录发出常规 `box_changed` kevent（与普通投递相同），前端无需订阅群事件；发送者本人的群消息不再投影到本人 INBOX，只有 SENT 记录（session_id 为群的规范 MailboxAddress）。

群基础状态、权限、序号、幂等、Guest、关系消息、签名原文、HTTP pull/发送、DID 发布重试、重启恢复和 Hook 故障策略均有模块测试。跨 Zone 测试使用独立 MessageCenter、签名设备及 HTTP 服务，覆盖自动接受、恢复、同步和发送；请求认证测试覆盖身份、群、host、路径/查询、方法、正文、签名和时间边界。真实 Gateway/Zone resolver、生产 OOD DID 文档、PostgreSQL、多副本、Telegram 平台关系消息映射和满容量性能仍需部署联调。
