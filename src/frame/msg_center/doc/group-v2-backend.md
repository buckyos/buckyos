# Self-host Group v2 后台接口

本目录实现 [Self-Host-Groupv2.md](../../../../doc/message_hub/Self-Host-Groupv2.md) 的默认单 host 后台。持久化格式见 [group-v2-storage.md](group-v2-storage.md)。实现不恢复 v1 GroupMgr、subgroup、递归群展开或收益归属。

## 入口与认证

`main.rs` 在 `/kapi/msg-center` 将 `group.*` 请求交给 `group_service.rs`，群 CYFS 路径由 `group_http.rs` 处理。业务主体只取自经过验证的 token，禁止参数 `actor_did`、`host_owner`。群创建还检查 Zone RBAC 的 `obj://msg-center/group` / `create`。

RPC 使用现有 kRPC 请求封装和 token。所有方法的参数是对象；除 create/list_by_member 外必须含 `group_did`。默认 Session 的 `session_id` 省略或为 null；具名 Session 使用未编码的原始字符串。对外的 `session` 和 `state_ref.session_key` 始终是规范 MailboxAddress。

角色、状态和规则枚举采用 snake_case，例如 `member`、`pending_admin_approval`、`explicit`、`from_join`。带值枚举是 `{"roles":["owner","admin"]}`、`{"only":["admin"]}`，无值规则是 `"inherit"`、`"all_participants"`、`"nobody"`。Owner 的固定能力不由配置撤销；Admin/Member 使用配置中的 roles 映射。

## 管理 RPC

| 方法 | 主要参数/结果 |
|---|---|
| group.create | profile?、configuration?、sessions?、invitations?、idempotency_key；proof?、group_id?、group_did?；返回群 Doc 和默认 Session |
| group.get_doc / get_config | 返回公开 Doc / 原始完整配置 |
| group.apply_config | expected_revision、patch、idempotency_key；返回新 revision，未知配置字段保留 |
| group.archive / delete | 仅 Owner；delete 留不可复用墓碑 |
| group.transfer_owner | member_did；仅 DID controller，目标需 Active 且已有 Owner 角色上限的签名同意 |
| group.invite_member | member_did、role?、expires_at_ms?；返回 invite_id |
| group.revoke_invite | member_did |
| group.create_invite_link | expires_at_ms?、max_uses?、require_approval?；返回 token |
| group.revoke_invite_link | token |
| group.submit_member_proof / request_join | proof、invite?；只按邀请或加入策略授予角色 |
| group.approve_member / reject_member | member_did；审批仅推进 PendingAdminApproval |
| group.leave | 操作者本人退出，Owner 需先转让 |
| group.remove_member | member_did；同时生成个人通知 |
| group.update_member_role | member_did、role；提升不能超出成员签署的角色上限 |
| group.moderate | member_did、blocked?、muted_until_ms?；null 清除禁言期限 |
| group.list_members | 按配置的成员列表可见性授权，Guest 不可调用 |
| group.create_session | session_id?、template?、membership?、rule_overrides?、members?、title?/description?/announcement? |
| group.update_session | session_id、expected_revision、template?、membership?、rule_overrides?、add_members? |
| group.archive_session / delete_session | session_id、expected_revision；默认 Session 不可删除 |
| group.list_sessions | 返回读者可访问的 Session、共享状态、state_ref、has_guests、规则和 revision |
| group.invite_session_guest | session_id、member_did |
| group.submit_session_proof | session_id、proof |
| group.submit_guest_request | request_id、proof；按 guest_entry 模板原子创建，重复请求复用结果 |
| group.remove_session_member | session_id、member_did |
| group.leave_session | session_id；仅退出本人参与关系 |
| group.list_by_member | 返回操作者参与的 Hosted Group 和本地 Joined Group |
| group.check_access | action、session_id?；返回 allowed 和 reason |
| group.list_events | after_seq?、limit?；需 group.read_all，管理查询包含审计 |
| group.update_read_marker | session_id?、last_read_seq；仅推进本人水位 |

上述写操作可以带 idempotency_key；相同主体、方法、键重试返回同一结果，改变请求则拒绝。配置与状态 patch 必须带幂等键及 expected_revision。Session 修改必须带 expected_revision。proof 提交默认以 proof ObjectId 幂等。

后台和成员列表查询会将已过期的邀请持久化为 expired，并且只发布一次 entity.invite_expired。重新邀请产生新的邀请 ID，旧 proof 不能用于确认新邀请。

附加查询：`group.list_messages(session_id?,after_seq?,limit?)`、`group.changes(since?,limit?)`、`group.get_read_markers(session_id?,session_seq)`、`group.sync_joined(group_did)`。权威共享/成员状态使用 `group.get_shared_state`、`group.get_member_state(member_did?)`、`group.update_shared_state`、`group.update_member_state`；更新参数为 session_id?、expected_revision、set、unset、idempotency_key。成员仅能修改本人昵称，业务权限不能从共享状态 patch 改写。

## Proof 与消息

原生成员 proof 是 EdDSA JWT 字符串，或 claims 与 `proof` JWT 一起提交的对象；后一形式必须与签名 claims 完全匹配。claims 字段：

```json
{
  "obj_type": "buckyos.group_member_proof",
  "schema_version": 1,
  "group_did": "did:web:g.example",
  "member_did": "did:web:alice.example",
  "signer": "did:web:alice.example",
  "role": "member",
  "proof_scope": "group",
  "nonce": "random-nonce",
  "issued_at_ms": 1790812800000,
  "expires_at_ms": 1790812860000
}
```

邀请确认再带 invite_id；Guest 不带 role，scope 为 `{"session":"原始 Session ID"}` 或 `{"session_request":"请求 ID"}`。签名者可为 member_did 或该 DID Document 的 authentication 授权主体；授权必须匹配完整 kid 或明确授权该 signer DID，不能用其它密钥片段替代。成员实体类型从 DID Document 判断，不接受群作为嵌套成员。

本 Zone 自动创建 proof 仅在 runtime 有匹配用户的真实私钥时启用；否则调用者必须提供签名。tunnel 的例外仅允许已登记实例的 transport 主体在本地提交对应 shadow DID 的 `attested_by` 和带 event_id/user_consent 的 source_event；远端不能提交此例外。

消息保持 `from=认证主体`、`to=[group_did]`、`kind=group_msg`。`to_session` 决定 Session，topic 不参与路由。host 接受消息时分配序号，并保存原始 MsgObject；客户端 created_at_ms 不决定排序。编辑、撤回、回应和 @all 在关系目标、作者、时间窗、能力、成员资格和限流校验后执行。

## CYFS HTTP

提供设计文档 §11.1 的 inbox、sessions、join、guest_requests、read_markers、changes 和 objects 路径。Session ID 按 MailboxAddress 规则编码为单个段，允许编码后的中文、空格和斜杠。没有 `/@/` 形式的 inner path。

请求使用 host 能验证的 Bearer token，`cyfs-original-user` 必须与 token 主体匹配。读取/发言还携带 `cyfs-proofs: ["gproof:..."]`，proof ID 必须是 host 已接受且归属于该主体的证明。首次加入把待验证的 proof 放在 body。客户端身份跨 Zone 尚无标准证明，因此 `allowed_clients=Only` 默认拒绝远端访问；只有群配置显式开启 allow_unverified_remote 才放行。

inbox PUT 的 body 为 canonical MsgObject JSON 或 `application/cyfs-named-object+jwt`，携带 cyfs-obj-id；JWT 必须验证成功，委托签名也要校验 from 的 DID Document 中的 authentication 授权。成功返回标准 CYFS accepted 状态和 cyfs-session-seq，终局拒绝可查询和幂等重试。JWT 对象读取原样返回签名原文，不降级 JSON。

inbox GET 使用 after_seq/limit，返回 seq、obj_id、redacted；默认实现还返回 accepted_at_ms 供本地投影使用 host 排序。changes 使用 reader-bound opaque token，不暴露 group_seq；未知/过期 token 返回 limited。退群、移除、Session 删除和群删除保留当事人的最小通知，不能借此读取消息正文。

read_markers 的标准 body 为 `{ "session": "群的规范 MailboxAddress", "last_read_seq": 42 }`。默认实现也接受原始 session_id/null。附件必须携带 `context_path=<规范群 Session 地址>/<消息 ObjectId>`；host 校验该消息可读且实际引用了目标对象，再交给 NamedStore/CYFS 获取。

公开资料有额外 GET `/<group_did>/doc`（ObjectId+Doc）和 `/<group_did>/did.json`（Doc 内容）入口。GroupDoc 来源于配置，包含 group DID context 和 iat，不作为规则来源。创建、公开配置/Owner 改变、归档和删除均在本地事务中登记发布待办；后台通过 system-config 原子发布 Zone resolver 的 group 类型文档或 tombstoned 状态，失败后重试。

## Joined Group 配置与运行

远端 host 的路由、用户访问凭据和成员 proof 通过现有服务 Settings 登记；没有添加依赖或额外进程：

```json
{
  "cyfs_dispatch": {
    "target_zone": "host.example",
    "joined_groups": [{
      "owner_did": "did:web:alice.example",
      "group_did": "did:web:g.remote.example",
      "host": "remote.example",
      "upstream": "https://remote.example/",
      "authorization": "Bearer <host可验证的用户凭据>",
      "proof_ids": ["gproof:<成员证明ObjectId>"]
    }]
  }
}
```

Settings 的主机/凭据/proof 要与已建立的成员关系一致；凭据更新后用现有 reload_settings 刷新。后台每 30 秒拉取一次，也可调用 group.sync_joined。同步 Session 列表、变更流和必要正文后，在一个 RDB 事务中保存本地 INBOX、副本、Session 登记和水位。未知 token 按各 Session 的 session_seq 补同步。更换 proof 后可恢复新成员周期；已获取的副本继续保留。

`post_send` 根据该发送者的 Joined Group binding 建立命名 Session 投递快照，Message Hub 携带该用户凭据、original-user 和 proof 投递。host 本地成员直接投影，外部原生成员通过 pull 获取；tunnel 成员复用既有 DELIVERY_QUEUE。

Hook 使用 JSON HTTP POST，body 仅有 operation、actor、group_did、session_id，post 另带 msg。runtime 服务 token 或显式 `cyfs_dispatch.group_hook_authorization` 用于服务身份认证。Hook 拒绝是终局；Deny 不可用可重试且不保存消息结果，NativeOnly 只保留原生权限。read 允许缓存按群/Session revision、读者、客户端和 Hook 绑定隔离。

## 部署边界与验证

本实现通过现有 Zone resolver 的控制面 cache 发布 group 文档，不修改内核 resolver。部署时需让 Gateway 将群的协议路径转发给 msg-center，并授予其既有 kernel 服务身份 resolver/cache 写权限；自备一级 DID 必须已经发布授权本 host 的文档。Joined Group 路由使用显式绑定，暂未增加自动 DID endpoint 发现或用户凭据续期。

群基础状态、权限、序号、幂等、Guest、关系消息、签名原文、HTTP pull/发送、DID 发布重试、重启恢复和 Hook 故障策略均有模块测试。真实 Gateway/Zone resolver、跨 Zone token 签发、PostgreSQL、多副本、Telegram 平台关系消息映射和满容量性能需要部署联调；变更提示目前是本 Zone 不含内容的 kevent，远端依赖轮询。
