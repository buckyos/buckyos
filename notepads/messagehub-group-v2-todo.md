# MessageHub 群聊（Self-host Group v2）后续 TODO

日期：2026-10-01

> 背景：Self-host Group v2 后端已在 commit a86f839f 提交（[后台接口](../src/frame/msg_center/doc/group-v2-backend.md)）。MessageHub WebUI 已接入（未提交），覆盖建群并选择联系人、群成员面板、邀请卡、群会话发送和具名会话，细节见 [UI_DATAMODEL.md §3.7](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)。
>
> 目前全部流程只在 mock 模式下验证过。真实 Zone 上建群会被两道检查挡住（第 2 节），部署的 msg-center 也还是旧版本（第 3 节）。
>
> 设计依据：[Self-Host-Groupv2.md](../doc/message_hub/Self-Host-Groupv2.md)。

## 1. 现状

| 能力 | mock | 真实 Zone | 说明 |
|---|---|---|---|
| 建群并选择联系人 | 可用 | 不可用 | RBAC 未授权（2.1）；群主 proof 无法生成（2.2） |
| 接受邀请 | 可用 | 不可用 | 需要成员本人签名的 proof（2.2） |
| 邀请、移出、撤销邀请、退出、解散 | 可用 | 未验证 | 依赖先建成群 |
| 群内收发消息、具名会话 | 可用 | 未验证 | 新消息最长约 20 秒后才显示（4.1） |
| Agent 入群 | — | 不可用 | OpenDAN 不处理群邀请（4.4） |
| 加入其他 Zone 托管的群 | — | 只能手工配置 | 需在 Settings 登记 joined_groups（4.5） |
| 编辑、撤回、回应、@提及 | 无 UI | 无 UI | 第 6 节 |

## 2. P0：建群与入群的阻塞（需要决策）

### 2.1 RBAC 没有授予建群权限

`group.create` 先检查 `obj://msg-center/group` 的 `create` 权限（[group_service.rs](../src/frame/msg_center/src/group_service.rs) `group_rpc_authenticated`）。默认策略（[rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs)）只有 `kernel`、`root`、`su_admin` 的 `obj://*` 能覆盖它，`users` 和 `admin` 都没有这条规则。因此普通登录的用户（包括 admin 组的 devtest）建群会被拒绝，还走不到 proof 检查。

- [ ] 决定谁可以建群：全部用户，还是只有 admin。
- [ ] 在默认策略中加入对应规则，例如 `p, users,obj://msg-center/group,create,allow`，并确认已激活 Zone 的 system-config 中的 RBAC 如何更新。
- [ ] 顺带处理 v2 §12.2 列出的问题：`users` 对 `obj://msg-center/group_inbox/*` 仍有 `read|write` 权限，该规则至今还在。

### 2.2 成员 proof 需要本人签名，浏览器做不到

- 建群：后端只有在 msg-center runtime 持有该用户的私钥时，才会自动构造群主 proof（`automatic_owner_proof`）。`user_private_key` 只在 AppClient 类型的 runtime 中加载（[runtime.rs](../src/kernel/buckyos-api/src/runtime.rs)），msg-center 是系统服务，永远拿不到，所以会返回 `owner-proof-required`。
- 入群：`group.submit_member_proof` 必须由调用方提交签名的 JWT，否则返回 `member-proof-required`。
- 目前 UI 不带 proof 发起请求，并原样显示后端给出的原因。

可选方案：

| 方案 | 做法 | 优点 | 代价 |
|---|---|---|---|
| A. 本 Zone 代签（建议） | 用户在 UI 中确认后，msg-center 以登录 token 为授权依据，用 Zone 持有的密钥签 proof；校验方顺着成员 DID 文档声明的所属 Zone 找到签名者 | 正是 v2 §2.4「同 Zone 用户的 proof 可以由本 Zone 在用户授权下自动构造」；UI 不需要改动 | 要改 `verify_member_proof` 的签名者规则；要确认 OwnerDocument 能否声明或授权 Zone 签名者，以及 msg-center 能否使用设备或 Zone 私钥（目前需要 `load_device_private_key`） |
| B. 客户端签名 | 由 BuckyOS App 等持有用户私钥的端签名（扫码或深链） | 信任模型最干净 | 用户私钥通常离线保存，需要新的签名通道，体验差、工作量大 |
| C. 用登录 token 当同意证据 | 类似 tunnel 的 `attested_by` | 改动最小 | 不是真实签名，违反 §2.4，远端 host 无法验证。不建议 |

- [ ] 选定方案（A 会改变信任模型与 proof 校验规则）。
- [ ] 后端实现，并补充测试：本 Zone 用户建群、接受邀请，以及远端 host 校验代签的 proof。
- [ ] 前端：采用 A 时基本不用改；采用 B 时，需要在 `createGroup` 和 `acceptGroupInvitation` 中接入签名步骤。
- [ ] 更新 v2 §2.4 / §14 的结论。

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

### 4.4 Agent 无法入群

成员选择器里可以选 Agent，但 OpenDAN 不处理 `buckyos.group_invitation` 通知，被邀请的 Agent 会一直停在「已邀请」。`msg_center_pump.rs` 已经能识别收到的群消息（`group_id`）。

- [ ] 确定 Agent 自动接受邀请的策略（例如只接受其 owner 发出的邀请），用 Agent 自己的密钥签 proof。
- [ ] 验证 Agent 在群内回复时使用 `to=[group_did]` 和解码后的 `to_session`，而不是回成私聊。

### 4.5 加入其他 Zone 托管的群

收到远端群的邀请后，需要手工在服务 Settings 中登记 `cyfs_dispatch.joined_groups`（host、upstream、authorization、proof_ids）才能同步，UI 无法完成。后端文档也注明「暂未增加自动 DID endpoint 发现或用户凭据续期」。

- [ ] 设计接受远端邀请后自动建立 joined 绑定和获取凭据的流程。

### 4.6 群资料不能修改

群名只在创建时写入 `profile`。

- [ ] 增加修改群名、简介的 UI（`group.apply_config`，需要 `expected_revision` 和幂等键）。

## 5. P2：群管理（后端已有 RPC，UI 未做）

- [ ] 转让群主 `group.transfer_owner`（目标成员要有签名同意，同样依赖 2.2）。
- [ ] 调整角色 `group.update_member_role`。
- [ ] 禁言 / 封禁 `group.moderate`。
- [ ] 入群审批 `group.approve_member` / `reject_member`。目前 `pending_approval` 通知卡上没有操作按钮。
- [ ] 邀请链接 `group.create_invite_link` / `revoke_invite_link`，以及加入时的 `group.request_join`。
- [ ] 具名会话管理：归档、删除、修改成员范围（`group.update_session` / `archive_session` / `delete_session`），以及会话成员的移出和退出。
- [ ] Session Guest 的邀请与加入，以及 `list_sessions` 中 `has_guests` 对应的「含外部成员」标识（v2 §5.3）。
- [ ] 已读水位与群回执的展示（会话规则中的 `receipts`）。
- [ ] 会话共享状态与成员昵称（`group.update_shared_state` / `update_member_state`）。真实环境下，会话详情目前显示「尚未启用」。

## 6. P2：消息关系与提及

MsgObject v2 已有 `relates_to`（edit / redact / reaction / thread）和 `mentions` 字段，后端会按作者、时间窗、能力等规则校验，UI 尚未支持。

- [ ] 渲染：编辑后的内容、撤回占位、回应汇总、话题。
- [ ] 发送：编辑、撤回（含管理员删帖 `message.redact_any`）、回应、@成员和 @all（`session.mention_all`）。

## 7. 文档联动

- [ ] [Self-Host-Groupv2.md](../doc/message_hub/Self-Host-Groupv2.md) §12 仍写着「v2 尚未实现」，需要更新为当前实现状态（后端 a86f839f，UI 本次）。
- [ ] [Self-Host-Group.md](../doc/message_hub/Self-Host-Group.md)（v1）文首同样写着「v2 尚未实现」。
- [ ] v2 §14 的第 1、2 项（默认 Session 使用裸 `group_did`；本地 Session 键使用规范 MailboxAddress）已按文中建议实现，确认后移入「已确认」。
- [ ] v2 §15 中列给 UI_DATAMODEL 的项：群主可查看、Session 标题来自 `group.list_sessions` 已完成；「含外部成员」、撤回占位、回应展示待做（第 5、6 节）。

## 8. 本次前端改动（未提交）

- 新增：`messagehub/groupModel.ts`、`GroupDialogs.tsx`、`GroupPanel.tsx`、`tests/e2e/pages/messagehub-group.spec.ts`。
- 修改：`messagehub` 下的 store（api / mock / types）、`MessageHubView.tsx`、`EntityList.tsx`、`EntityDetails.tsx`、`ConversationView.tsx`、`SessionDetails.tsx`、`SessionDialogs.tsx`、`conversation/history/renderers.tsx` 和 `actions.ts`、`api/projection.ts`、`datamodel/sessionApi.ts`、`i18n/messagehub.ts`、`UI_DATAMODEL.md`。
- 后端只新增了契约测试 `test_group_service.rs::message_hub_group_ui_contract`。
- 验证命令（在 `src/frame/desktop` 下运行）：
  - `npx playwright test tests/e2e/pages/messagehub-group.spec.ts tests/e2e/pages/messagehub.spec.ts tests/e2e/pages/messagehub-media.spec.ts tests/e2e/pages/preview.spec.ts`（40 例通过）
  - `deno test --config tests/datamodel/messagehub.deno.json --no-check --allow-read --allow-env tests/datamodel/messagehub-*.test.ts`（20 例通过）
  - 在 `src` 下运行 `cargo test -p msg_center`（109 例通过）
