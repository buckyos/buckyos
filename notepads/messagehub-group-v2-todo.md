# MessageHub 群聊（Self-host Group v2）后续 TODO

日期：2026-10-01

> 背景：Self-host Group v2 后端已在 commit a86f839f 提交（[后台接口](../src/frame/msg_center/doc/group-v2-backend.md)）。MessageHub WebUI 已接入（未提交），覆盖建群并选择联系人、群成员面板、邀请卡、群会话发送和具名会话，细节见 [UI_DATAMODEL.md §3.7](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。
>
> 目前全部流程只在 mock 模式下验证过。真实 Zone 上建群会被两道检查挡住（第 2 节），其中成员 proof 检查已决定取消（2.2）。部署的 msg-center 也还是旧版本（第 3 节）。
>
> 已确认建群权限（2.1）和 Agent 入群策略（4.4）。跨 Zone 认证沿用 DID Document 语义，关系信任由 Contact Mgr 管理；当前尚未形成完整接入流程，自动入群与凭据管理保留为 TODO（4.5），不阻塞同 Zone 落地。
>
> 设计依据：[Self-Host-Groupv2.md](../doc/message_hub/Self-Host-Groupv2.md)。

## 1. 现状

| 能力 | mock | 真实 Zone | 说明 |
|---|---|---|---|
| 建群并选择联系人 | 可用 | 不可用 | RBAC 未授权（2.1）；后端仍要求群主 proof，待按 2.2 取消 |
| 接受邀请 | 可用 | 不可用 | 后端仍要求成员签名的 proof，待按 2.2 取消 |
| 邀请、移出、撤销邀请、退出、解散 | 可用 | 未验证 | 依赖先建成群 |
| 群内收发消息、具名会话 | 可用 | 未验证 | 新消息最长约 20 秒后才显示（4.1） |
| Agent 入群 | — | 不可用 | OpenDAN 不处理群邀请（4.4） |
| 加入其他 Zone 托管的群 | — | 只能手工配置 | 需在 Settings 登记 joined_groups（4.5） |
| 编辑、撤回、回应、@提及 | 无 UI | 无 UI | 第 6 节 |

## 2. P0：建群与入群的阻塞

### 2.1 RBAC 没有授予建群权限（权限范围已决定）

`group.create` 先检查 `obj://msg-center/group` 的 `create` 权限（[group_service.rs](../src/frame/msg_center/src/group_service.rs) `group_rpc_authenticated`）。默认策略（[rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs)）只有 `kernel`、`root`、`su_admin` 的 `obj://*` 能覆盖它，`users` 和 `admin` 都没有这条规则。因此普通登录的用户（包括 admin 组的 devtest）建群会被拒绝，还走不到 proof 检查。

**决定（2026-10-01）**：普通登录用户和 admin 都可以建群。Zone RBAC 授予建群权限，群内操作由群的成员资格、角色与能力控制。

- [x] 确认建群权限覆盖普通用户和 admin。
- [ ] 在默认策略中分别加入 `p, users,obj://msg-center/group,create,allow` 和 `p, admin,obj://msg-center/group,create,allow`，并确认已激活 Zone 的 system-config 中的 RBAC 如何更新。
- [ ] 顺带处理 v2 §12.2 列出的问题：`users` 和 `admin` 对 `obj://msg-center/group_inbox/*` 仍有 `read|write` 权限，需要收窄；直接访问 GROUP_INBOX 仍须同时满足 RBAC 和群授权。

### 2.2 取消成员 proof，改为群和成员双方同意（已决定）

**决定（2026-10-01）**：群聊沿用「self-host 全听群主」的信任模型（v2 §1.5），入群不再要求成员签名的 proof。是否入群只看两件事：群一方是否同意，成员一方是否同意。

**原因**：成员 proof 来自 v1 的「成员关系双向可证」：群声明某个 DID 是成员，成员也签名证明自己愿意加入，第三方就能从两边分别验证。它服务的公开成员列表、嵌套群、收益归属，都已移到 v2 §13 后置。到了 v2，校验 proof 的只有 host 自己，而 host 就是群主，本来就能直接修改成员表，签名约束不了它，也没有第三方会去校验。成员能不能收到群消息，本来就由成员自己的 Zone 决定，因为原生成员是主动拉取的。保留 proof 只剩代价：它需要用户私钥，而浏览器和 msg-center 都拿不到。

**新规则**：

| 项目 | 规则 |
|---|---|
| 群一方同意 | Owner 或 Admin 发出的邀请视为群已同意，被邀请者接受后直接成为 `Active`，不再审批。主动申请需要有审批能力的成员批准。邀请链接和 `join_policy = open` 视为管理员事先批准。默认只有 Owner、Admin 有 `group.invite_member`；如果群配置让普通成员也能邀请，他发出的邀请在对方接受后进入 `PendingAdminApproval` |
| 成员一方同意 | 人类成员由自己的 Zone 按 Contact Mgr 策略决定：邀请人是好友就自动接受；陌生人的邀请进入 REQUEST_BOX，等本人确认；被屏蔽的邀请人按 Block 策略拒绝。Agent 按 4.4 的 owner 策略处理。转让群主不自动接受，必须本人确认 |
| 请求认证 | 同 Zone 用登录 token。跨 Zone 沿用 DID Document 的身份与认证授权语义，关系信任和准入由 Contact Mgr 决定，再查群成员表（或 Guest 记录）；现有 joined 绑定仍携带 host 可验证的 `authorization`，自动获取与续期流程按 4.5 保留 TODO。外部平台（tunnel）用户沿用接入证据，由 tunnel 的 transport 身份代为提交 |
| 角色 | 角色由邀请决定，接受邀请就是接受邀请中的角色。主动申请和邀请链接只能得到 `Member`。群主可以直接把成员升为 Admin，不需要本人同意 |
| Session Guest | 同样只看双方同意，不再需要 Session 作用域的 proof（v2 §6.5） |

将来做公开成员的双向证明时，可以由成员的 Zone 用 Zone 密钥自动签发可验证记录，不需要用户参与。

**后端现状**：proof 除了表示同意，还被挪作他用。删除时要给这些用途找到替代：

- 建群：只有 msg-center runtime 持有用户私钥时才会构造群主 proof（`automatic_owner_proof`），系统服务永远拿不到，所以返回 `owner-proof-required`。
- 入群：`group.submit_member_proof`、`request_join`、`submit_session_proof`、`submit_guest_request` 都要求签名的 JWT，否则返回 `member-proof-required`。
- 操作者身份：这四个 RPC 用 proof claims 里的 `member_did` 替换操作者，tunnel 就是靠这一点代外部平台用户提交（接入证据被包装成 `obj_type = buckyos.group_member_proof` 的对象）。幂等键也取 proof 的 ObjectId。
- 成员类型：`entity_kind`（user、agent、device）是在验 proof 时顺带从成员 DID 文档解析出来的，Agent 相关规则会用到（[group_service.rs](../src/frame/msg_center/src/group_service.rs) 约 2915 行）。
- 审批：被邀请者提交后，只要 `join_policy = request_and_approve` 就进入 `PendingAdminApproval`，不区分邀请人是不是管理员（约 1678 行）。
- 角色变更：升为 Admin 要求成员 proof 的 `role` 至少是 admin（`role-consent-required`），转让群主要求目标成员的 proof `role = owner`（`owner-consent-required`）。
- 跨 Zone 读取：`group_http.rs` 读 `inbox`、`sessions`、`changes`、`objects`、`read_markers` 时，要求 `cyfs-proofs` 里带本人的 proof ObjectId。joined 绑定的 `proof_ids` 是必填项，同步和发送时作为 `cyfs-proofs` 头带上。
- 删除群：`tombstone_readers` 记录「曾参与者 DID → proof ID」，靠 proof ID 让曾经的成员读到删除通知。
- 目前 UI 不带 proof 发起请求，并原样显示后端给出的原因。

**接口改名**（已决定改名，不保留旧名）：

| 现在 | 改为 | 参数 |
|---|---|---|
| `group.submit_member_proof` | `group.accept_invitation` | `group_did`、`invitation_id`；只用于接受直接邀请，`invitation_id` 用来防止误接受过期后重新发出的另一份邀请 |
| `group.request_join` | 不变 | `group_did`、`invite?`；主动申请和邀请链接入群都走这里 |
| `group.submit_session_proof` | `group.accept_session_invitation` | `group_did`、`session_id` |
| `group.submit_guest_request` | 不变 | `request_id` 直接传参，现在是从 proof 的 `proof_scope` 中取 |
| `group.create` | 不变 | 去掉 `proof` |
| `PUT /<group_did>/join[?invite=]` | 路径不变 | body 不再是 proof；有待接受的邀请就按 `accept_invitation` 处理，否则按 `request_join` |
| `PUT /<group_did>/sessions/<sid>/join` | 路径不变 | body 不再是 proof，对应 `accept_session_invitation` |
| `PUT /<group_did>/guest_requests` | 路径不变 | body 改为 `{ request_id }` |

外部平台用户的接入证据改为独立参数 `attestation`（`member_did`、`source_event`），由 tunnel 的 transport 身份提交，校验规则沿用现有的 `invalid-tunnel-attestation` 检查，不再伪装成 proof 对象。

待办（规则）：

- [ ] 建群：由 token 认证的调用者直接写入 Owner 记录。
- [ ] 审批规则按邀请人区分：Owner、Admin 发出的邀请，接受后直接 `Active`；其他人发出的邀请进入 `PendingAdminApproval`。
- [ ] 成员一方自动接受：同 Zone 由 msg-center 投递邀请时查接收者作用域的 Contact Mgr，好友邀请直接接受、陌生人邀请进入 REQUEST_BOX、Block 拒绝；Agent 按 4.4 处理。跨 Zone 自动接受与 joined 绑定的建立按 4.5 保留 TODO。
- [ ] `entity_kind` 改为在邀请或接受时解析成员 DID 文档得到，解析逻辑从 `verify_member_proof` 中抽出来保留。
- [ ] 跨 Zone 读取改为只看凭据和成员表（或 Guest 记录）。
- [ ] `tombstone_readers` 改为只记曾参与者的 DID。

待办（删除 msg-center 中的现有实现）：

- [ ] `group_service.rs`：
  - 删除 `verify_member_proof`（保留上面抽出的 `entity_kind` 解析）、`consume_proof`、`proof_scope`、`automatic_owner_proof`。
  - 删除 RPC 入口里按 proof ObjectId 查幂等、用 proof claims 替换操作者的两段逻辑，幂等统一用 `idempotency_key`。
  - 删除各分支中依赖 proof 的检查：`group.create` 的 `owner-proof-role-required`，入群分支的 `proof-invitation-mismatch`（改为比对 `invitation_id` 参数）和 `proof-role-mismatch`，`approve_member` 的 `member-proof-required`，`update_member_role` 的 `role-consent-required`，`transfer_owner` 的 `owner-consent-required`（新流程见第 5 节）。
  - 删除只为 proof 存在的错误码：`owner-proof-required`、`member-proof-required`、`signed-member-proof-required`、`proof-signer-mismatch`、`proof-payload-mismatch`、`invalid-proof-type`、`invalid-proof-lifetime`、`proof-scope-mismatch`、`proof-replayed`、`proof-key-unavailable`。
- [ ] `group_types.rs`：删除 `MemberRecord.proof_id`、`SessionMembershipRecord.proof_id`、`GroupState.proofs` 和 `nonces`；Guest 是否有效只看 `state == "included"`，去掉 `proof_id.is_some()` 条件。
- [ ] `group_http.rs`：删除读路径对 `cyfs-proofs` 的检查，以及 `join`、`sessions/<sid>/join`、`guest_requests` 把 body 当 proof 解析的逻辑。
- [ ] `group_sync.rs`、`cyfs_dispatch.rs`、`message_hub.rs`：删除 joined 绑定的 `proof_ids` 字段和必填校验，以及同步、发送时附带的 `cyfs-proofs` 头。`send_with_proofs` 合并回 `send`。
- [ ] `test_group_service.rs`：
  - 删除 `proof()` 辅助函数及其调用，去掉 `joined_route` 的 `proof_ids`。
  - 改写 `proofs_invite_links_and_approval_cannot_resurrect_removed_members`（只保留邀请链接与审批）、`shadow_members_require_registered_tunnel_consent_and_authenticated_transport`（改用 `attestation`）和 `group_http_signed_delivery_and_joined_sync_preserve_identity_order_and_read_markers`。
  - `delegated_signatures_are_bound_to_the_author_document_and_authorized_key` 测的是 MsgObject 签名，保留。
- [ ] 群后端还没部署过（第 3 节），不做旧数据和旧接口的兼容。

待办（测试与前端）：

- [ ] 新增测试：普通用户和 admin 同 Zone 建群；好友邀请自动入群；陌生人邀请进 REQUEST_BOX，确认后入群；被屏蔽的邀请人不能触发自动入群；普通成员发出的邀请需要审批；群主直接任命 Admin；tunnel 用 `attestation` 入群；删除群后曾经的成员仍能读到删除通知。
- [ ] 跨 Zone 接受邀请后凭据读取与同步的真实流程测试，待 4.5 落地后补充；本轮仍需验证删除 proof 后现有显式 joined 绑定的读取与同步。
- [ ] 前端删除 proof 相关实现：
  - `datamodel/sessionApi.ts` 的 `submitGroupMemberProof` 改名并改调 `group.accept_invitation`，传 `invitation_id`。
  - `i18n/messagehub.ts` 删除 `owner-proof-required`、`member-proof-required`、`signed-member-proof-required` 文案，`proof-invitation-mismatch` 换成后端的新错误码。
  - `groupModel.ts` 的 `knownGroupErrors` 同步修改；「成员只接受能自己签 proof 的单体 DID」的注释改写为单体 DID 的真实理由（嵌套群后置）。
  - `api/store.ts`、`store/types.ts` 中提到 proof 的注释，以及 `mock/store.ts` 的 `proof-invitation-mismatch`。
  - `UI_DATAMODEL.md` §3.7 的缺口 1 和 RPC 表。
- [ ] 前端体验：被自动接受的邀请，邀请卡显示「已加入」，不再显示「接受」按钮；成员发出的邀请需要审批后，`pending_approval` 卡片上的审批按钮（第 5 节）变成必需功能。

## 3. P0：部署与真实 Zone 验证

- [ ] 部署包含 a86f839f 的 msg-center 和新的 Desktop web。当前 `/opt/buckyos/bin/msg-center/msg_center` 构建于 2026-09-30 08:11，早于群后端的提交；自动模式会拦截部署，需要人工执行。
- [ ] 按 [group-v2-backend.md](../src/frame/msg_center/doc/group-v2-backend.md)「部署边界」配置：Gateway 把群的协议路径转发给 msg-center；kernel 服务身份要有 resolver / cache 写权限。
- [ ] 解决第 2 节后，在 test.buckyos.io 上走一遍：devtest 建群并邀请 bob → bob 在私聊中看到邀请卡并加入 → 双方收发 → 新建具名会话 → 移出、退出、解散。
- [ ] 在 `test/test_msg_center` 的 deno 脚本中增加 `group.*` RPC 用例，能和 `test_messagehub_sessions.ts` 一样复现。

## 4. P1：体验与正确性

### 4.1 新群消息只能靠轮询显示

后端写成员 INBOX 投影时走 `GroupTransaction::mailbox`（[group_store.rs](../src/frame/msg_center/src/group_store.rs)），不会发出常规投递路径那种 `box_changed` kevent。群事务只发 `/msg_center/group/<did>/changed`，前端没有订阅。结果是群里的新消息要等 20 秒的轮询才出现。

- [ ] 建议后端在群事务提交后，为每条投影记录发出 `box_changed`，与普通投递保持一致。这样前端无需改动。

### 4.2 自己发的群消息被投影了两份

`append_group_message` 为本地发送者写 SENT 的同时，又把同一条消息投影到发送者自己的 INBOX，状态为 UNREAD；joined 群的同步路径也会把自己的消息写进 INBOX。前端目前在群会话里丢掉这份副本并自动标为已读（[api/store.ts](../src/frame/desktop/src/app/messagehub/api/store.ts) `timelineItems`），但：

- 标记已读之前，未读数会短暂多出 1；
- OpenDAN、CLI 等其它客户端仍会看到重复，也会计入未读。

- [ ] 建议后端对发送者本人跳过 INBOX 投影，或者直接写成 READ。改完后删除前端的兜底逻辑。

### 4.3 发言权限缓存会过期

前端按会话缓存 `group.check_access(session.post)` 的结果，只在打开群面板或执行群操作后刷新。被别人移出、群被归档后，要等到发送被拒才会知道。

- [ ] 收到群变更事件时让缓存失效（依赖 4.1）。

### 4.4 Agent 无法入群（策略已决定）

成员选择器里可以选 Agent，但 OpenDAN 不处理 `buckyos.group_invitation` 通知，被邀请的 Agent 会一直停在「已邀请」。`msg_center_pump.rs` 已经能识别收到的群消息（`group_id`）。

**决定（2026-10-01）**：Agent 只自动接受其 owner 发出的邀请；其他人的邀请由 owner 确认，owner 的好友也不能直接触发 Agent 自动入群。确认只代表 Agent 一方同意，群一方的审批规则仍按 2.2 执行。

- [x] 确认 Agent 自动接受邀请的范围。
- [ ] 由 Agent 所在的 Zone 识别邀请人与 Agent owner 的关系，owner 邀请自动接受，其他邀请提供 owner 确认流程；按 2.2 入群，不需要签 proof。
- [ ] 验证 owner 邀请自动接受、owner 的好友和陌生人邀请均需 owner 确认。
- [ ] 验证 Agent 在群内回复时使用 `to=[group_did]` 和解码后的 `to_session`，而不是回成私聊。

### 4.5 加入其他 Zone 托管的群（方向已决定，接入流程保留 TODO）

收到远端群的邀请后，需要手工在服务 Settings 中登记 `cyfs_dispatch.joined_groups`（host、upstream、authorization、proof_ids）才能同步，UI 无法完成。按 2.2 取消 proof 后，`proof_ids` 不再需要。后端文档也注明「暂未增加自动 DID endpoint 发现或用户凭据续期」。

**决定（2026-10-01）**：跨 Zone 认证应复用 BuckyOS 基于 DID Document 的完整身份与认证授权语义，关系信任的核心是 msg-center 中的 Contact Mgr。DID Document 用于确认身份与认证授权关系，Contact Mgr 管理好友、陌生人、屏蔽等准入策略，群成员表和 Session 规则决定具体群权限。

**当前实现核对**：群 HTTP 入口通过 `group_actor` 使用 [RuntimeSessionTokenVerifier](../src/frame/msg_center/src/owner_session.rs)，最终调用 [runtime.rs](../src/kernel/buckyos-api/src/runtime.rs) 的 `verify_trusted_session_token`，校验本 Zone 信任的 verify-hub 签发的 session token。尚未接通基于远端 DID Document 的完整认证、joined 绑定自动建立和凭据续期流程，因此这部分保留为 TODO，不作为同 Zone 建群与入群落地的前置条件。

- [ ] 核对并复用系统通用的 DID Document 跨 Zone 认证流程，包括成员身份、代为请求的授权关系和客户端身份，不另起群专用认证协议。
- [ ] 将跨 Zone 邀请接入接收者作用域的 Contact Mgr：好友按策略自动接受，陌生人进入 REQUEST_BOX，Block 拒绝；Agent 按 4.4 处理。
- [ ] 在认证流程形成后，设计接受远端邀请时自动发现 host、建立 joined 绑定、获取和续期 host 可验证凭据的流程，与 2.2 的跨 Zone 自动接受一起落地。

### 4.6 群资料不能修改

群名只在创建时写入 `profile`。

- [ ] 增加修改群名、简介的 UI（`group.apply_config`，需要 `expected_revision` 和幂等键）。

## 5. P2：群管理（后端已有 RPC，UI 未做）

- [ ] 转让群主 `group.transfer_owner`：目标成员必须本人确认，不适用好友自动接受。后端目前要求目标成员持有 `role = owner` 的 proof；按 2.2 改为两步：群主发起，目标成员用认证请求接受。
- [ ] 调整角色 `group.update_member_role`。
- [ ] 禁言 / 封禁 `group.moderate`。
- [ ] 入群审批 `group.approve_member` / `reject_member`。目前 `pending_approval` 通知卡上没有操作按钮。按 2.2，主动申请和普通成员发出的邀请都要审批，这一项应在 2.2 落地时一起做。
- [ ] 邀请链接 `group.create_invite_link` / `revoke_invite_link`，以及加入时的 `group.request_join`。
- [ ] 具名会话管理：归档、删除、修改成员范围（`group.update_session` / `archive_session` / `delete_session`），以及会话成员的移出和退出。
- [ ] Session Guest 的邀请与加入（按 2.2 不需要 proof），以及 `list_sessions` 中 `has_guests` 对应的「含外部成员」标识（v2 §5.3）。
- [ ] 已读水位与群回执的展示（会话规则中的 `receipts`）。
- [ ] 会话共享状态与成员昵称（`group.update_shared_state` / `update_member_state`）。真实环境下，会话详情目前显示「尚未启用」。

## 6. P2：消息关系与提及

MsgObject v2 已有 `relates_to`（edit / redact / reaction / thread）和 `mentions` 字段，后端会按作者、时间窗、能力等规则校验，UI 尚未支持。

- [ ] 渲染：编辑后的内容、撤回占位、回应汇总、话题。
- [ ] 发送：编辑、撤回（含管理员删帖 `message.redact_any`）、回应、@成员和 @all（`session.mention_all`）。

## 7. 文档联动

- [ ] 将 2.1 的建群权限、4.4 的 Agent 入群策略、4.5 的 DID Document 认证与 Contact Mgr 信任边界同步到 v2 设计及相关实现文档；跨 Zone 自动接入标为 TODO。
- [ ] [Self-Host-Groupv2.md](../doc/message_hub/Self-Host-Groupv2.md) §12 仍写着「v2 尚未实现」，需要更新为当前实现状态（后端 a86f839f，UI 本次）。
- [ ] [Self-Host-Group.md](../doc/message_hub/Self-Host-Group.md)（v1）文首同样写着「v2 尚未实现」。
- [ ] v2 §14 的第 1、2 项（默认 Session 使用裸 `group_did`；本地 Session 键使用规范 MailboxAddress）已按文中建议实现，确认后移入「已确认」。
- [ ] 按 2.2 修订 v2 中所有与成员 proof 有关的内容：§1.5 写明入群同样只看群主一方的记录；§2.4 的 proof 一节改为双方同意规则；§6.1 删除 Owner 的签名 proof；§6.2 按邀请人区分审批；§6.5 Guest 加入不需要 proof；§10 第 4 条；§11.1 的 `join`、`sessions/<sid>/join`、`guest_requests` 和 `cyfs-proofs`；§11.2 按 2.2 的接口改名；§13 注明公开成员双向证明届时由成员 Zone 签发；§14 第 7 项（tunnel 接入证据）改为外部平台用户的同意方式。
- [ ] 按 2.2 修订 msg-center 的实现文档：[group-v2-backend.md](../src/frame/msg_center/doc/group-v2-backend.md) 的 RPC 表、「Proof 与消息」一节中的成员 proof 部分（MsgObject 的 JWT 签名保留）、跨 Zone 请求的 `cyfs-proofs`、Joined Group 配置示例的 `proof_ids`；[group-v2-storage.md](../src/frame/msg_center/doc/group-v2-storage.md) 中 members / participants 的 `proof_id`、`proofs` / `nonces`、`tombstone_readers`。
- [ ] v2 §15 中列给 UI_DATAMODEL 的项：群主可查看、Session 标题来自 `group.list_sessions` 已完成；「含外部成员」、撤回占位、回应展示待做（第 5、6 节）。

## 8. 本次前端改动（未提交）

- 新增：`messagehub/groupModel.ts`、`GroupDialogs.tsx`、`GroupPanel.tsx`、`tests/e2e/pages/messagehub-group.spec.ts`。
- 修改：`messagehub` 下的 store（api / mock / types）、`MessageHubView.tsx`、`EntityList.tsx`、`EntityDetails.tsx`、`ConversationView.tsx`、`SessionDetails.tsx`、`SessionDialogs.tsx`、`conversation/history/renderers.tsx` 和 `actions.ts`、`api/projection.ts`、`datamodel/sessionApi.ts`、`i18n/messagehub.ts`、`UI_DATAMODEL.md`。
- 后端只新增了契约测试 `test_group_service.rs::message_hub_group_ui_contract`。
- 验证命令（在 `src/frame/desktop` 下运行）：
  - `npx playwright test tests/e2e/pages/messagehub-group.spec.ts tests/e2e/pages/messagehub.spec.ts tests/e2e/pages/messagehub-media.spec.ts tests/e2e/pages/preview.spec.ts`（40 例通过）
  - `deno test --config tests/datamodel/messagehub.deno.json --no-check --allow-read --allow-env tests/datamodel/messagehub-*.test.ts`（20 例通过）
  - 在 `src` 下运行 `cargo test -p msg_center`（109 例通过）
