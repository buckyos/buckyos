# Beta 2.2：Add Local User E2E TODO

> 状态：待实现  
> 整理日期：2026-08-11  
> 目标：把已经可通过 Control Panel KAPI 创建并登录的 local user，收口为 Desktop 中可由管理员完整操作、可重复验证的 Beta 2.2 功能。

## 1. Beta 2.2 功能边界

Beta 2.2 只交付下面这一条用户路径：

1. 已登录的管理员在 Desktop 的 Users & Agents 页面点击 Add User。
2. 输入本地用户名、显示名、初始密码和确认密码。
3. 管理员通过 sudo 密码确认本次高权限操作。
4. 系统创建一个 `is_local=true`、`user_type=User`、`state=Active` 的用户。
5. 页面重新读取真实用户列表并展示新用户。
6. 管理员退出后，新用户可以在当前 Zone 的登录页使用用户名和密码登录 Desktop。

### 1.1 本期明确支持

- 只支持 Zone-local 密码账户。
- 新用户类型固定为普通 `User`。
- 新用户创建后立即为 `Active`。
- 默认允许用户修改自己的密码，即 `allow_password_change=true`。
- 使用现有默认资源池和默认 users RBAC，不在创建表单中暴露策略配置。
- 用户数据目录仍按现有机制在首次使用 App 时按需创建，不要求 `user.create` 直接创建 home 目录。

### 1.2 本期明确不做

- 一级 BNS / DID 邀请、邀请链接和 `Pending` 激活流程。
- 开放注册、钱包登录、Passkey、跨 Zone 登录和身份迁移。
- 通过 Add User 创建 Admin、Limited、Guest 或 Root。
- 默认权限组编辑、自定义 RBAC、资源池和存储配额。
- 在创建用户时选择、安装或授权 App。
- Agent、Self-hosted Group、Message Tunnel 和 My Network 功能。
- 完整用户生命周期 UI，例如封禁、恢复、改类型、导出数据和物理清理。
- 多用户共享 App 模型；本期只保证新用户能登录 Desktop 和访问已有基础系统入口。

当前 Add User 向导里的 Primary DID、User Type、Available Apps、Storage Quota、Invitation Expiry 等选项必须移除或隐藏，不能展示一个后端不会执行的配置。

## 2. 当前实现基线

### 2.1 已具备

- Control Panel 已注册 `user.create / user.list / user.get` 等 KAPI。
- `user.create` 已要求管理员身份，并依赖 sudo token 完成敏感写入。
- 创建过程会事务写入用户 settings、OwnerDocument、local key 和 profile，再请求 scheduler 刷新动态 RBAC。
- scheduler 只把 `Active` 用户加入运行时用户表，并为普通用户生成 users RBAC。
- VerifyHub 已支持 local user 的密码登录和 sudo 密码验证。
- Desktop 已能从真实后端读取用户列表和详情。
- 2026-08-11 已在本地 DV 环境手工走通：Gateway → `user.create` → scheduler RBAC refresh → `auth.login` → 新用户 token 调用 `user.get`。

### 2.2 尚未完成

- `NewUserWizard` 提交时只调用 `store.addLocalUser()` 修改内存，没有调用后端。
- 向导仍混合 DID invitation、App 和 storage quota 等非 Beta 2.2 能力。
- 前端 `createUser()` 没有传入临时 sudo session token 的能力。
- 创建成功后没有以后端 reload 结果替换本地临时数据。
- 现有前端 E2E 只验证 mock state，不验证真实 Gateway、sudo、KAPI 和登录链路。
- `test/test_control_panel/test_user_mgr.ts` 虽覆盖 create/login，但没有被 `test/run.py --list` 收录，且完整用例最终会软删除测试用户。
- local key 的存储路径和 VerifyHub 用户状态校验仍有发布前安全缺口，见 P0。

## 3. 目标 E2E 流程

```text
Admin 登录 Desktop
  → Users & Agents / Add User
  → 输入 username、display name、password、confirm password
  → Review
  → sudo 对话框重新输入管理员密码
  → user.create（请求携带 sudo session token）
  → system-config 事务提交
  → scheduler 刷新 RBAC
  → Desktop reload 用户列表并选中新用户
  → Admin logout
  → 新用户通过 /kapi/control-panel 的 auth.login 登录
  → Desktop 加载成功，user.get 返回本人 Active local user
```

## 4. 实现 TODO

## P0：发布前安全与一致性

### [ ] P0-1 把 local user 私钥移入 security namespace

当前 `user.create` 把私钥写到 `users/{user_id}/key`，但 RBAC 设计定义的路径是 `security/{user_id}/key`。`users/*` 已有多种读取规则，不应保存私钥。

要求：

- 改为写入 `security/{user_id}/key`。
- 检查所有 local user key 的读取方并同步路径。
- 补 RBAC 测试：普通用户、普通管理员、frame/app/system service 均不能直接读取该 key；只有明确授权的 kernel/root/su_admin 路径可以访问。
- Beta 2.2 是 breaking change，不做旧路径兼容读取。

主要文件：

- `src/frame/control_panel/src/user_mgr.rs`
- `src/kernel/buckyos-api/src/rbac_config.rs`

### [ ] P0-2 VerifyHub 只给 Active 用户签发或刷新 token

当前密码校验读取 `UserSettings` 后只验证密码，没有验证状态；refresh token 也不会重新读取用户状态。

要求：

- `login_by_password` 和 `sudo_by_password` 只允许 `UserState::Active`。
- refresh token 换新 token 前重新读取 settings，并拒绝 Pending、Suspended、Deleted、Banned。
- 补每种非 Active 状态的登录、sudo 和 refresh 回归测试。
- 保留 Control Panel 对 Active 状态的二次检查，形成纵深防御。

主要文件：

- `src/kernel/verify_hub/src/main.rs`
- `src/kernel/verify_hub` 对应测试

### [ ] P0-3 明确定义 create 已提交但 RBAC refresh 失败的响应

system-config 事务和 scheduler refresh 不是同一个事务。当前可能出现“用户已经创建，但 API 返回失败；再次提交又提示重复”的歧义。

要求：

- API 响应能区分 `created` 和 `rbac_refreshed`，或采用其它能明确表达部分成功的结果。
- 前端遇到不确定结果时必须 reload 用户列表，不能直接重复创建。
- scheduler 最终 reconcile 后用户应自动进入正确 RBAC。
- 增加 refresh 失败场景测试。

## P0：Desktop 创建链路

### [ ] P0-4 将 Add User 向导简化为 local user 表单

要求：

- 删除或隐藏 Primary DID 分支以及邀请相关文案。
- 删除 User Type 选择，新用户固定为 `user`。
- 删除 Available Apps、Storage Quota 和 Invitation Expiry 步骤。
- 保留并校验：username、display name、password、confirm password。
- username 在前端遵循后端同一套格式限制；提交前 trim 并转小写。
- 密码至少检查非空、确认一致和合理长度；具体密码策略不得只存在于 UI，后端至少拒绝空或非法 hash。
- Review 页面明确提示：这是当前 Zone 内的本地账户，依赖本 Zone，创建后可以登录并占用本机资源。

主要文件：

- `src/frame/desktop/src/app/users-agents/components/shared/NewUserWizard.tsx`
- `src/frame/desktop/src/app/users-agents/datamodel/types.ts`

### [ ] P0-5 接入现有 sudo 对话框

要求：

- 点击最终 Create 后调用现有 `useSudoByPassword()`。
- 用户取消 sudo 时不创建用户，并保留表单内容。
- sudo 失败、过期或无权限时显示可理解的错误，允许重新申请。
- sudo token 只用于本次 `user.create`，不覆盖 Desktop runtime 的普通 session token，也不写入持久存储。

主要文件：

- `src/frame/desktop/src/components/sudo.tsx`
- `src/frame/desktop/src/app/users-agents/components/shared/NewUserWizard.tsx`

### [ ] P0-6 让 `user.create` API 支持单次 session token override

要求：

- 复用 Web SDK managed RPC client 的 per-call `sessionToken` override。
- 扩展通用 `callRpc` options，或给 `createUser` 增加明确的 token 参数；避免在向导里重复实现 KRPC。
- `password_hash` 必须使用 `hashPassword(username, initialPassword)` 的原始账户 hash，不带登录 nonce。
- 发送固定参数：

```json
{
  "user_id": "alice",
  "show_name": "Alice",
  "password_hash": "<hashPassword(username, password)>",
  "user_type": "user",
  "allow_password_change": true
}
```

主要文件：

- `src/frame/desktop/src/api/rpc.ts`
- `src/frame/desktop/src/api/user_mgr.ts`

### [ ] P0-7 用真实 reload 完成创建后的 UI 收敛

要求：

- 删除生产创建路径中的 `store.addLocalUser(fakeEntity)`。
- `user.create` 成功后调用 store/backend reload。
- 以后端返回的数据找到并选中新用户；UI entity id 不得再使用 `user-${slug}-${Date.now()}` 伪 ID。
- reload 失败时提示“用户可能已经创建”，保留 Retry reload，不再次自动提交 create。
- 用户列表刷新后必须展示正确的 display name、`User` 类型、Active 状态和 local credential 状态。

主要文件：

- `src/frame/desktop/src/app/users-agents/datamodel/store.ts`
- `src/frame/desktop/src/app/users-agents/datamodel/api.ts`
- `src/frame/desktop/src/app/users-agents/components/shared/NewUserWizard.tsx`
- 打开向导和处理 `onCreated` 的父组件

### [ ] P0-8 完整错误状态和重复提交保护

至少覆盖：

- username 已存在，包括已软删除但仍占用 ID 的用户。
- username 格式不合法。
- 密码或确认密码不合法。
- 当前用户不是管理员。
- sudo 密码错误、取消或 token 过期。
- control-panel / system-config / scheduler 暂时不可用。
- 双击 Create 或慢请求导致的重复提交。

提交期间禁用返回、关闭和重复提交；失败后恢复可操作状态。

## P1：新用户登录与最小可用体验

### [ ] P1-1 验证 Desktop 登录页的真实 local user 链路

要求：

- 使用 Desktop 当前实现的 `/kapi/control-panel` `auth.login`，不能只直打 VerifyHub 端口。
- 登录密码 hash 使用本次 login nonce。
- 登录成功后拿到 session/refresh token，并进入 Desktop。
- `user.get` 返回当前用户本人、`state=active`、`is_local=true`、`user_type=user`。
- 新用户不能调用 `user.create`，也不能读取或修改其他用户的敏感数据。

### [ ] P1-2 定义新用户首次登录能看到什么

Beta 2.2 不做创建时 App 分配，但必须明确最小可用界面：

- Desktop 能正常加载，不因缺少用户 App spec、Agent 或 home 目录而报错。
- Users & Agents 至少能显示本人资料。
- 未安装或未授权的功能显示正常 empty state，而不是 403/500 或无限 loading。
- 不承诺 FileBrowser、Jarvis 等用户 App 自动可用；如基础系统入口实际依赖 App spec，需在此任务中明确补默认 provisioning，或从 Beta 2.2 验收范围移除对应入口。

### [ ] P1-3 创建结果在服务重启后仍存在

要求：

- 创建用户后执行保留数据的 `stop.py` / `start.py`。
- 重启后用户仍在 list/get 中且可以再次登录。
- 不使用 `start.py --all` 做持久性验证，因为该命令会全量重装并清空 DV 数据。

## P1：自动化测试与门禁

### [ ] P1-4 把 Control Panel user DV 纳入标准 test runner

要求：

- 让相关测试出现在 `uv run test/run.py --list` 中。
- 可单独执行 local-user DV，不要求顺带运行所有 Agent、Invite 或长期规划用例。
- 测试必须经过 Gateway，不能直打 4020、3200 或 3300 服务端口。
- 测试账号使用明确的 `dvlocal*` 前缀和唯一 ID。
- 测试结束后软删除测试用户；输出清楚说明软删除记录仍会保留在 system-config。

主要文件：

- `test/test_control_panel/test_user_mgr.ts`，或拆出的 local-user 专用 DV
- `test/test_control_panel` 的 runner discovery 配置

### [ ] P1-5 增加真实 UI DV Test

Playwright 必须模拟真人完成：

1. 管理员登录。
2. 打开 Users & Agents。
3. Add User 填写 local user 表单。
4. 在真实 sudo 对话框中确认。
5. 等待用户出现在真实列表。
6. 管理员退出。
7. 新用户登录。
8. 验证 Desktop 和本人详情可用。
9. 使用管理员 KAPI 清理测试用户。

要求：

- 禁止启用 mock store。
- 失败时保存 screenshot、浏览器 console、最后一次 RPC 错误和相关服务日志位置。
- 测试密码只能来自环境变量或运行时生成，不能提交固定生产凭据。

### [ ] P1-6 增加必要的后端回归测试

- 普通 User 创建用户被拒绝。
- 普通 admin session 创建用户被拒绝，sudo admin 创建成功。
- 重复 username 被拒绝且原用户不被覆盖。
- create 事务任一 key 冲突时不产生部分用户数据。
- 创建后 scheduler RBAC 包含新用户的 users role。
- 新用户密码登录成功；错误密码失败。
- 非 Active 用户登录、sudo、refresh 均失败。
- local key 不可被非授权 principal 读取。

## 5. E2E Done Definition

以下条件全部满足才算 Beta 2.2 Add Local User 完成：

- [ ] 生产 UI 中只展示本期真实支持的 local user 字段和选项。
- [ ] 管理员必须通过真实 sudo 才能创建用户。
- [ ] 前端不再通过 fake entity 假装创建成功。
- [ ] 创建请求经过 Gateway，并在 system-config 中形成完整、原子的用户记录。
- [ ] 私钥保存在正确的 security namespace，并有 RBAC 回归测试。
- [ ] scheduler 刷新后，新用户获得普通 users RBAC。
- [ ] 新用户能通过 Desktop `auth.login` 登录并加载最小可用页面。
- [ ] 新用户不能创建其他用户或访问其他用户的敏感数据。
- [ ] Suspended、Deleted、Banned 用户不能登录、sudo 或 refresh token。
- [ ] 服务重启后用户仍存在且可登录。
- [ ] Service DV 和真实 UI DV 都通过，并进入标准测试入口。
- [ ] `cargo test` 和 `uv run buckyos-build.py` 通过。

## 6. 建议实施顺序

1. P0-1～P0-3：先关闭后端安全和部分成功歧义。
2. P0-4～P0-8：简化向导，接入 sudo、真实 KAPI 和 reload。
3. P1-1～P1-3：验证新用户登录、最小 Desktop 和持久性。
4. P1-4～P1-6：补齐 service/UI DV 与后端回归门禁。
5. 按 Done Definition 做一次全新 DV 环境的人工验收。

## 7. 与旧 TODO 的关系

`notepads/control_panel_gap_todo.md` 仍可作为完整 Users & Agents 长期缺口记录，但其中部分“当前现状”已经过时。Beta 2.2 实现和验收以本文的窄范围为准；DID 邀请、Limited、App 分配、Agent 和 Group 等内容后移，不应阻塞本期 Add Local User。
