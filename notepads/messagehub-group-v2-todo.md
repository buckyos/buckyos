# MessageHub 群聊（Self-host Group v2）后续 TODO

日期：2026-10-01（本次更新：2026-10-01 晚，完成 2.1 / 2.2 / 4.1–4.4 / 4.6 / 5 / 6 / 7，均未提交 git）

> 背景：Self-host Group v2 后端在 commit a86f839f 提交（[后台接口](../src/frame/msg_center/doc/group-v2-backend.md)），MessageHub WebUI 在 commit 39d6d6f0 提交，细节见 [UI_DATAMODEL.md §3.7](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。
>
> 本次按 2.2 的决定取消了成员 proof，并补齐了 RBAC 建群权限、Agent 入群、群管理 UI 和消息关系 UI。改动只在 mock 模式和模块测试中验证过；真实 Zone 仍需先部署新的 msg-center（第 3 节）再跑 `test_messagehub_groups.ts`。
>
> 仍然保留的 TODO 只有两类：部署与真实 Zone 验证（第 3 节，需要人工执行）；跨 Zone 的自动接入（4.5，用户决定保留为 TODO，不阻塞同 Zone 落地）。
>
> 设计依据：[Self-Host-Groupv2.md](../doc/message_hub/Self-Host-Groupv2.md)（已按本次修订）。

## 1. 现状

| 能力 | mock | 真实 Zone | 说明 |
|---|---|---|---|
| 建群并选择联系人 | 可用 | 待部署验证 | RBAC 已授权 users / admin（2.1）；建群不再需要 proof（2.2） |
| 接受邀请 | 可用 | 待部署验证 | `group.accept_invitation { invitation_id }`；好友邀请由 msg-center 自动接受 |
| 邀请、移出、撤销邀请、退出、解散 | 可用 | 待部署验证 | 依赖先建成群 |
| 群内收发消息、具名会话 | 可用 | 待部署验证 | 群事务提交后发 `box_changed`，不再等 20 秒轮询（4.1） |
| Agent 入群 | 可用 | 待部署验证 | owner 邀请自动入群；其他人邀请由 owner 代为接受（4.4） |
| 加入其他 Zone 托管的群 | — | 只能手工配置 | 仍需在 Settings 登记 joined_groups（host、upstream、authorization；已无 proof_ids），自动接入见 4.5 |
| 群管理（转让、角色、禁言、审批、邀请链接、Session 管理、Guest、回执、共享状态）| 可用 | 待部署验证 | 第 5 节 |
| 编辑、撤回、回应、@提及 | 可用 | 待部署验证 | 第 6 节 |

## 2. P0：建群与入群的阻塞（已完成）

### 2.1 RBAC 建群权限（已完成）

**决定（2026-10-01）**：普通登录用户和 admin 都可以建群。群内操作由群的成员资格、角色与能力控制。

- [x] 确认建群权限覆盖普通用户和 admin。
- [x] 默认策略（[rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs)）加入 `p, users,obj://msg-center/group,create,allow` 和 `p, admin,obj://msg-center/group,create,allow`。
- [x] `users` / `admin` 对 `obj://msg-center/group_inbox/*` 收窄为 `read`；直接访问 GROUP_INBOX 仍须同时满足 RBAC 和群授权（`authorize_mailbox` 已拒绝非读操作）。新增测试 `zone_users_and_admins_may_create_groups_but_not_write_group_inboxes`。
- [x] 已激活 Zone 的更新方式：默认策略编译在各服务里，system-config 的 `system/rbac/policy` 只是用户 / 组绑定的 tail（`build_current_rbac_config` 把两者拼接），所以**重新构建并部署 msg-center 和 sys_config_service 即生效**，不需要改 system-config。

### 2.2 取消成员 proof，改为群和成员双方同意（已完成）

规则与接口契约已写入 v2 设计 §2.4 / §6.2 / §11 与 [group-v2-backend.md](../src/frame/msg_center/doc/group-v2-backend.md)，要点：

| 项目 | 实现 |
|---|---|
| 群一方同意 | `MemberRecord.invited_by` 记录邀请人；接受时邀请人仍有 `group.approve_member` 能力 → 直接 `active`，否则 `pending_admin_approval` 并通知审批者（通知带 `invited_by`）。主动申请按 `join_policy`（`invite_only` → `invite-required`）；邀请链接视为已批准（除 `require_approval`） |
| 成员一方同意 | 同 Zone：`invite_member` 时 msg-center 用 `ContactMgr::peek_access_permission`（只读，群事务内不能用会写库的 `check_access_permission`，否则 SQLite 锁死）查接收者作用域：好友 → 自动接受；陌生人 → 邀请进 REQUEST_BOX；屏蔽 → 不投递。Agent：只自动接受 `AgentDocument.owner` 的邀请，其他邀请投给 owner（通知带 `member_did`），owner 用 `accept_invitation { invitation_id, member_did }` 代为接受（`SessionTokenVerifier::agent_owner`）。跨 Zone 成员仍手工接受（4.5） |
| 请求认证 | 同 Zone 用 token；跨 Zone HTTP 只看 Bearer token + `cyfs-original-user` 与成员表 / Guest 记录，`cyfs-proofs` 头、joined 绑定的 `proof_ids` 全部删除 |
| 角色 | 接受即接受邀请中的角色；`update_member_role` 升 Admin 不需本人同意 |
| 转让群主 | 两步：`transfer_owner { member_did }` 发起（`pending_owner_transfer`，通知 `owner_transfer`），目标成员 `accept_owner_transfer { transfer_id }`，群主可 `cancel_owner_transfer`；`list_members` 结果附带 `pending_owner_transfer`（仅成员可见） |
| Session Guest | `accept_session_invitation { session_id }`、`submit_guest_request { request_id }`，不需要 proof |
| 外部平台用户 | tunnel 的 transport 身份提交 `attestation { member_did, source_event: { event_id, user_consent } }`，校验沿用 `invalid-tunnel-attestation`，接受后写入 `audit` |
| `invite_member` 返回 | `{ invite_id, member_did, expires_at_ms, state }`，邀请通知 data 也带 `state`（`invited` / `active` / `pending_admin_approval`） |

接口改名：`submit_member_proof → accept_invitation(group_did, invitation_id)`、`submit_session_proof → accept_session_invitation`，`create` 去掉 `proof`，HTTP `join` body 为 `{ invitation_id? }`（省略且有待接受邀请时按接受处理，否则按申请），`sessions/<sid>/join` body 空，`guest_requests` body `{ request_id }`。幂等统一用 `idempotency_key`。

已删除：`verify_member_proof` / `consume_proof` / `proof_scope` / `automatic_owner_proof`、按 proof ObjectId 的幂等、`proof_id` / `proofs` / `nonces`、`send_with_proofs`、全部 proof 错误码（新增 `invitation-mismatch`、`transfer-mismatch`、`agent-owner-required`）。`entity_kind` 改为在邀请 / 接受 / 建群时解析（msgtunnel → user，本 Zone Agent → agent，did:dev → device，其它解析 DID 文档，拿不到为 `unknown` 并在激活时重试）。`tombstone_readers` 改为曾参与者 DID 集合，删除群后曾参与者仍能从 `changes` 读到删除通知。

顺带修复：`create_group` 原来只存原始配置 JSON，之后 `apply_config` 打 `roles.member` 补丁会把 `admin` 的默认能力整个丢掉；现在存完整配置。

- [x] 后端实现与测试（`test_group_service.rs` 新增：好友自动入群 / 陌生人确认 / 屏蔽、普通成员邀请需审批与群主直接任命 Admin、Agent 只接受 owner 邀请、两步转让群主、HTTP join 与无 proof 读取、tunnel attestation、删除群后的通知）。
- [x] 前端：`acceptGroupInvitation(groupDid, invitationId, memberDid?)`、i18n 与 `knownGroupErrors` 更新、注释改写、UI_DATAMODEL §3.7 与 RPC 表更新；邀请卡按 `state` 显示「已加入」/「已接受，等待审批」/ Agent 代接受；`pending_approval` 卡有批准 / 拒绝按钮。
- [ ] 跨 Zone 接受邀请后的凭据读取与同步真实流程测试，待 4.5 落地后补充。

## 3. P0：部署与真实 Zone 验证（需要人工）

- [ ] 重新构建并部署 msg-center、sys_config_service（RBAC 默认策略）、Desktop web 和 OpenDAN（4.4）。当前 `/opt/buckyos/bin/msg-center/msg_center` 构建于 2026-09-30 08:11，早于群后端；自动模式会拦截部署，需要人工执行。
- [ ] 按 [group-v2-backend.md](../src/frame/msg_center/doc/group-v2-backend.md)「部署边界」配置：Gateway 把群的协议路径转发给 msg-center；kernel 服务身份要有 resolver / cache 写权限。
- [ ] 部署后在 test.buckyos.io 运行 [test_messagehub_groups.ts](../test/test_msg_center/test_messagehub_groups.ts)（已写好并通过 `deno check`，需要第二个 Zone 用户，默认 `BUCKYOS_TEST_MEMBER_USER=bob`）：devtest 建群并邀请 bob → bob 接受 → 双方收发（发送者无 INBOX 副本）→ 具名会话 → 移出、退出、解散 → bob 自建群验证 users 的建群权限。再用 WebUI 走一遍邀请卡与群面板。

## 4. P1：体验与正确性

### 4.1 新群消息只能靠轮询显示（已完成）

- [x] `GroupTransaction` 记录事务内写入的邮箱记录，`commit` 成功后逐条发 `box_changed` kevent（`MessageCenter::publish_box_changed_event`），与普通投递一致，前端无需改动。

### 4.2 自己发的群消息被投影了两份（已完成）

- [x] host 的 `append_group_message` 对本地发送者跳过 INBOX 投影（仍登记 owner_sessions）；joined 群同步同样跳过本人发出的消息。前端删除了 `timelineItems` 的兜底逻辑。OpenDAN pump 另外过滤了自己群消息的回声。

### 4.3 发言权限缓存会过期（已完成）

- [x] 前端在摘要刷新时按会话 `updated_at_ms` 变化清掉该群的 `postAccess` 缓存（被移出时 `entity.member_removed` 事件会投影到被移出者的 INBOX，从而触发 box_changed 与刷新）。

### 4.4 Agent 无法入群（已完成）

**决定（2026-10-01）**：Agent 只自动接受其 owner 发出的邀请；其他人的邀请由 owner 确认。

- [x] msg-center 在邀请时识别 Agent 的 owner（`agent_owner`）：owner 邀请自动接受，其他邀请投递给 owner 确认，owner 代为接受后仍按 2.2 的群一方审批规则处理。
- [x] 验证（`agents_accept_only_their_owners_invitations_and_others_need_the_owner`）：owner 邀请自动接受；成员邀请需 owner 代接受且进入待审批；非 owner 代接受被拒（`agent-owner-required`）。
- [x] OpenDAN：原实现把群消息回成私聊（`to = [peer_did]`、`kind = chat`），已改为 `to = [group_did]`、`kind = group_msg`、`to_session` = 解码后的 sid（`SessionMeta.group_id`、`build_outbound_chat_base_msg` / `decode_group_session_key`），群会话不再读写人类发送者的 tunnel 绑定；pump 丢弃 `buckyos.group_invitation` 通知（不进 LLM）。单元测试已加；真实后端端到端待部署后验证。未做：群内斜杠命令仍回给发送者；具名群 Session 目录嵌套导致重启时不自动恢复路由（按需挂载）。

### 4.5 加入其他 Zone 托管的群（方向已决定，接入流程保留 TODO）

**决定（2026-10-01）**：跨 Zone 认证复用 BuckyOS 基于 DID Document 的身份与认证授权语义，关系信任由 msg-center 的 Contact Mgr 管理，群成员表和 Session 规则决定具体群权限。已写入 v2 §4.2。

**现状**：群 HTTP 入口通过 `RuntimeSessionTokenVerifier` 校验本 Zone 信任的 verify-hub session token；joined 绑定仍需手工在 Settings 登记 `cyfs_dispatch.joined_groups`（host、upstream、authorization）。本次没有接通远端 DID Document 认证、joined 绑定自动建立和凭据续期，这部分不阻塞同 Zone 落地。

- [ ] 核对并复用系统通用的 DID Document 跨 Zone 认证流程，包括成员身份、代为请求的授权关系和客户端身份，不另起群专用认证协议。
- [ ] 将跨 Zone 邀请接入接收者作用域的 Contact Mgr：好友按策略自动接受，陌生人进入 REQUEST_BOX，Block 拒绝；Agent 按 4.4 处理（同 Zone 的 `member_consent` 逻辑可直接复用）。
- [ ] 在认证流程形成后，设计接受远端邀请时自动发现 host、建立 joined 绑定、获取和续期 host 可验证凭据的流程。

### 4.6 群资料不能修改（已完成）

- [x] GroupPanel 增加群名、简介编辑（`group.apply_config` 带 `expected_revision` 与幂等键；revision 取自 `group.get_config`）。

## 5. P2：群管理 UI（已完成，mock 验证）

- [x] 转让群主：两步流程（后端见 2.2），群面板发起 / 取消，目标成员在 `owner_transfer` 通知卡接受。
- [x] 调整角色 `group.update_member_role`（成员菜单：设为 / 取消 Admin）。
- [x] 禁言 / 封禁 `group.moderate`。
- [x] 入群审批 `group.approve_member` / `reject_member`：`pending_approval` 卡和成员列表均可操作。
- [x] 邀请链接 `group.create_invite_link` / `revoke_invite_link`（群面板生成、复制、撤销，格式 `<group_did>?invite=<token>`），建群对话框可用链接加入（`group.request_join`）。
- [x] 具名会话管理：归档、删除、增加成员（`update_session` / `archive_session` / `delete_session`）、退出会话；移出会话成员的 store 方法已有但没有 UI 入口（后端没有列出 Session 参与者的 RPC）。
- [x] Session Guest 的邀请与接受（`invite_session_guest` / `accept_session_invitation`），会话列表按 `has_guests` 显示「含外部成员」。
- [x] 已读水位与群回执：`receipts` 非 hidden 时在本人最新一条消息下显示「已读 N」及读者（真实后端只覆盖本人最新一条，seq 来自 `group.list_messages`）；`update_read_marker` 随标记已读调用。
- [x] 会话共享状态与成员昵称（`update_shared_state` / `update_member_state`），真实后端的会话详情不再显示「尚未启用」。

## 6. P2：消息关系与提及（已完成，mock 验证）

- [x] 渲染：编辑后内容（「已编辑」标记）、撤回占位、回应汇总、回复引用、@提及标记（`conversation/history/relations.ts` 折叠 `relates_to`）。
- [x] 发送：编辑、撤回、管理员删帖（`message.redact_any`）、回应、@成员与 @all（`session.mention_all`），都是带 `relates_to` / `mentions` 的 `post_send`；提及用按钮选择器（无 `@` 自动补全）。关系操作只在群会话提供。

## 7. 文档联动（已完成）

- [x] v2 设计：§1.5、§2.4、§4.1、§4.2（建群权限、group_inbox 收窄、跨 Zone 认证方向）、§6.1、§6.2、§6.5、§10、§11.1、§11.2、§12（实现现状，§12.2 标注处理结果）、§13、§14（#1 #2 #7 移入已确认，新增已确认项）、§15。
- [x] [Self-Host-Group.md](../doc/message_hub/Self-Host-Group.md)（v1）文首改为「v2 已实现」。
- [x] [group-v2-backend.md](../src/frame/msg_center/doc/group-v2-backend.md)（RPC 表、「同意与消息」、CYFS HTTP、Joined Group 配置、部署边界含 RBAC 与 box_changed）、[group-v2-storage.md](../src/frame/msg_center/doc/group-v2-storage.md)。
- [x] UI_DATAMODEL.md §3.7 与 RPC 表（随前端改动）。

## 8. 本次改动（未提交）与验证

- 后端：`src/frame/msg_center/src/{group_service,group_types,group_http,group_sync,group_store,cyfs_dispatch,message_hub,msg_center,owner_session,contact_mgr,test_group_service,test_msg_center}.rs`；`src/kernel/buckyos-api/src/rbac_config.rs`。
- OpenDAN：`src/frame/opendan/src/{agent,agent_session,agent_session_test,msg_center_pump,session_model}.rs`。
- 前端：`src/frame/desktop/src/app/messagehub/` 下的 datamodel / groupModel / types / store / api / mock / conversation（新增 `history/relations.ts`、`input/mentions.ts`、`confirmDialog.tsx`）、GroupPanel、GroupDialogs、SessionDetails、SessionSidebar、ConversationView、MessageHubView、`src/i18n/messagehub.ts`、UI_DATAMODEL.md；测试 `tests/e2e/pages/messagehub-group.spec.ts`（9 例）、`tests/datamodel/messagehub-group.test.ts`。
- 验证命令：
  - `cd src && cargo test -p msg_center`（114 例通过）、`cargo test -p buckyos-api rbac_config`（19 例通过）、`cargo test -p opendan`（277 通过，`mini_agent_demo_parses` 为已知无关失败）。
  - 在 `src/frame/desktop` 下：`npx playwright test tests/e2e/pages/messagehub-group.spec.ts tests/e2e/pages/messagehub.spec.ts tests/e2e/pages/messagehub-media.spec.ts tests/e2e/pages/preview.spec.ts`（45 例，连续 3 次通过）、`deno test --config tests/datamodel/messagehub.deno.json --no-check --allow-read --allow-env tests/datamodel/messagehub-*.test.ts`（25 例通过）、`npx tsc -b`、`npx eslint .`（0 错误）。
  - 真实 Zone：`cd test/test_msg_center && deno run --config ../deno.json --allow-net --allow-env --unsafely-ignore-certificate-errors test_messagehub_groups.ts`（待部署后执行）。
