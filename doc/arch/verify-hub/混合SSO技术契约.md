# 混合 SSO 技术契约

日期：2026-10-21。状态：阶段 A 交付物，随实现迭代；与代码冲突时以代码和本文同步修订为准。

依据：[混合 SSO PRD](<BuckyOS 混合SSO PRD.md>)（v0.2）、[实施 TODO](<../../../notepads/hybrid-sso-implementation-todo.md>) §4 已定决策（D-1～D-9、Q-01～Q-10、S-1、P-1、P-2）、[UserType.md](<../../../product/users_and_agents/UserType.md>)。本文只冻结工程契约，不新增产品决策；术语沿用 PRD：专属邀请、自助注册、DID 登录、Guest。

## 1. 组件职责

| 组件 | 负责 | 不负责 |
| --- | --- | --- |
| verify-hub | 校验凭据并签发 token pair：密码、专属邀请 secret、DID 联合 code；refresh / logout / delegate；family 持久化（§7）；签名 claims 中的认证方式与会话上限；canonical 侧签发与兑换联合 code | 浏览器 Cookie、入口与资源授权、发送通知 |
| Control Panel（CP） | 浏览器编排：`/sso_*` 回调与 Cookie、pending（state / nonce / 回跳）；专属邀请管理与重发通知；自助注册建号；Zone SSO 开关；入口描述编排；DID 登录发起与回调；canonical 侧 consent | 资源成员资格与 grant |
| 资源服务（msg-center、AI Workspace、HomeStation） | 入口记录、资源 grant、按方法开放受限会话、资源级撤销 | 签发会话 |
| WebSDK | 当前身份（Account / External / Anonymous）、拒绝原因码解析、jump URL 构造 | 判定授权 |
| Gateway | 路由与静态页面；`check_oauth` 只认 App target 的普通会话 | 受限会话授权（由服务判定） |
| jump | 只做“去哪个 Zone”的导航；不持有任何凭据 | 认证、授权 |

Origin / CSRF：浏览器写 Cookie 的接口只有 CP 的 `/sso_callback`、`/sso/did_callback`、`/sso_refresh`、`/sso_logout`，以及返回 token 的 `auth.*` kRPC。它们都是 host-only Cookie + `SameSite=Lax`；`auth.*` 只接受 POST JSON（不接受表单），跨站页面无法发出携带 JSON 的同源请求而不触发 CORS 预检，CP 不返回 CORS 允许头。回调 GET 只消费服务端 pending（一次性 nonce / state），不接受页面提供的 token。

## 2. 可信主体与当前身份

### 2.1 主体

| 类别 | 判定 | `sub` | 账号记录 | DID 来源 |
| --- | --- | --- | --- | --- |
| Account | `principal_kind=user` 且无 `external` | user_id | `users/<user_id>/settings` | `users/<user_id>/doc` 的 `id`（OwnerDocument） |
| External | `principal_kind=user` 且 `external: true` | DID 字符串 | 无 | `sub` 本身 |
| Anonymous | 无 verify-hub 会话 | — | 无 | 无；AIWS 资源主体 `guest:<hash>` |

- **账号 → DID 的唯一真相源是 `users/<id>/doc`**。`users/<id>/profile.did` 只是展示资料的一部分；system-config 拒绝非管理者写入与 doc 不一致的 `profile.did`（§3.5），所有服务改为读 doc（`ControlPanelClient::get_user_did`）。profile 中 `system_contact.did` 不再参与身份或收件人判定。
- **user_id 稳定且不可复用**：删除只把 `state` 置为 `deleted`，记录保留，`user.create` 以 `KVAction::Create` 写入，同名再建失败。
- **External 入驻**：同一 DID 之后被接受为本 Zone 账号时，DID 登录改为映射到该账号；旧 External 会话保持原 claims 直至过期，不升级。`user.invite.accept` 必须携带该 DID 经联合登录取得的 Control Panel 用户会话，并提交匹配的 OwnerDocument；匿名或密码会话不能接受邀请。
- 展示资料（昵称、头像）不决定授权主体。
- **App 代登录的账号**（§5.6）也是 Account：`sub` 是 CP 为 `(app_instance_id, external_account_id)` 创建的 Limited 账号；`auth_method=app_access_token`，target 是签发 code 的 App 实例。

### 2.2 当前身份响应

CP 的 `/sso_refresh` 与 `auth.current_identity` 返回 `identity`（Rust：`buckyos_api::CurrentIdentity`；TS：`CurrentIdentity`）：

```json
{"kind":"account","user_id":"acme-c1","did":"did:bns:acme-c1.test","user_type":"limited",
 "show_name":"Alice","zone_host":"test.buckyos.io","auth_method":"exclusive_invite",
 "restricted":true,"scopes":["account_self","msg_group","aiworkspace","homestation","aicc"],
 "password_set":false}
{"kind":"external","did":"did:bns:alice","show_name":"Alice","zone_host":"b.example",
 "auth_method":"did_federation","restricted":true,"scopes":["msg_group","aiworkspace","homestation"]}
{"kind":"anonymous"}
```

- `/sso_refresh` 返回 `{session_token, identity}`；`user_info` 字段删除（开发期直接迁移）。SDK 缓存按 `zone_host + kind + user_id|did` 绑定，身份变化时清空缓存，不回退旧身份。
- `auth.login` 的返回保留 `user_info`（密码登录必为 Account），另加 `identity`。

## 3. 会话 claims、上限与默认拒绝

### 3.1 新增签名 claims

| claim | 类型 | 说明 |
| --- | --- | --- |
| `auth_method` | `password` \| `exclusive_invite` \| `did_federation` \| `assertion` \| `app_access_token` | family 建立方式；delegate、refresh 继承。`app_access_token`：App 为其外部客户代登录（§5.6，2026-10-24） |
| `restricted` | bool，缺省 false | 受限会话；缺省即普通会话 |
| `scopes` | string[]，仅 `restricted=true` 时出现 | 批准的上限，见 3.2 |
| `external` | bool | 已有，见 2.1 |

服务端 family 记录（§7）另存来源（专属邀请 ID / 版本、联合 canonical）、父 family、绝对期限；这些不进 claims。

### 3.2 上限（`SessionScope`）

| scope | 开放内容 |
| --- | --- |
| `account_self` | 本人 settings 读取、profile 读写、通讯录（按 `can_edit_contacts`）、改密（按 `allow_password_change`）、首设密码、本人桌面状态、获准 App 列表 |
| `msg_group` | msg-center 群与 Session 路径（Session Guest）、群消息发送、带引用上下文的附件读取 |
| `aiworkspace` | AI Workspace（受 grant 约束，不含创建 / 导入 / 复制 / 列全体主体） |
| `homestation` | HomeStation 读与评论 |
| `aicc` | AICC 推理、模型与 Provider 列表、本人额度查询；能用的资源由可见性策略决定（AI Center Token 额度 PRD §3.4） |

| 主体 | `restricted` | `scopes` |
| --- | --- | --- |
| Admin / User（任何认证方式） | false | — |
| Limited（任何认证方式） | true | 全部五项 |
| External | true | `msg_group`、`aiworkspace`、`homestation` |
| 受限会话的 delegate 子会话 | 继承父会话 | 继承父会话 |

- **旧会话不增权**：refresh、grace 与 delegate 的新 claims = 旧 claims ∩ 当前账号类型的上限。Limited 升级为 User 后旧会话仍受限，要获得新权限须重新登录；User 降为 Limited 后，下一次 refresh 即收窄（access 至多残留 15 分钟）。
- sudo 只能经 `sudo_by_password`；受限会话不签发 sudo（Limited 的 `sudo_by_password` 拒绝）。

### 3.3 默认拒绝的闸口

`buckyos-api` runtime：

- `verify_trusted_session_token(token)`：验签后，若 `restricted=true` 且不是“本 App 运行时收到指向自身实例的会话”，返回拒绝 `session_kind_not_allowed`。约 11 个不认识受限会话的服务因此自动拒绝。
- `verify_session_token_opened(token, scope)`：服务对某方法显式开放受限会话时调用；`restricted=false` 直接通过，`restricted=true` 要求 `scope ∈ scopes`，否则 `session_ceiling_exceeded`。
- `enforce(req, action, resource)` 经 `verify_trusted_session_token`，受限会话默认拒绝；`enforce_opened(req, action, resource, scope)` 是开放版本。
- system-config 不用 runtime 验签，在 `process_request` 单独实现：受限会话只允许 `account_self` 范围的键（本人 `settings` 读、`profile` 读写）且仍过 RBAC。

开放清单（SSO-04 审计表在 TODO 附录维护）：verify-hub（refresh / logout / 首设密码由 CP 代调）；CP 的 `auth.*` 与 `account_self` 方法；msg-center `group.*`、群消息发送、群附件、本人通讯录；AI Workspace 全部方法（类别受限）；HomeStation 读与评论。

### 3.4 App target

受限会话可以换到获准 App 实例（App availability 在签发时判定），App 后端收到的 token target 等于自身实例时通过闸口；App 用自己的服务身份调用系统服务，不转发用户的受限会话。

### 3.5 身份字段写保护（PRE-01 / 02）

- scheduler 不再给 `su_<user>` 生成本人 `settings` / `doc` 写规则；改密、首设密码、类型 / 组 / 状态变更全部经 CP 业务方法，由 CP 校验后以服务身份写入。
- system-config 对 `users/<id>/profile` 的写入（set、set_by_json_path、append、exec_tx、create）：写入者不是管理者（kernel / system / ood / root / admin / su_admin）时，结果中的 `did` 必须等于 `users/<id>/doc` 的 `id`；删除 profile 同样拒绝。

## 4. 拒绝原因码（D-4）

### 4.1 Wire

kRPC 失败响应增加可选字段（buckyos-base kRPC）：

```json
{"error":"access denied: resource_grant_missing","code":"access_denied",
 "data":{"reason":"resource_grant_missing","adjust":{"party":"sharer","location":"share_panel"},
         "service":"aiworkspace","method":"doc.read"},"sys":[17]}
```

- Rust：`RPCErrors::Detailed(RPCErrorDetail{code, message, data})`；服务端 dispatch 把它序列化为上面的字段，客户端 kRPC 解析回同一变体。`buckyos_api::AccessDenial::from_rpc_error` 按 `code == "access_denied"` 取出并反序列化 `data`。
- TS：`RPCError.code` / `RPCError.data`；`accessDenialOf(error)` 返回类型化结果。
- 不带 `code` 的旧错误保持原样；页面只按 `code` / `reason` 分支，不解析 `error` 文本。

### 4.2 原因码与调整方

| reason | 含义 | adjust.party | adjust.location | 访客侧显示 |
| --- | --- | --- | --- | --- |
| `session_kind_not_allowed` | 此类会话不开放该方法（产品边界） | `product` | — | 此类账号不支持该功能 |
| `session_ceiling_exceeded` | 超出会话来源 / 上限 | `self` | `reauth` | 请重新登录后再试 |
| `resource_grant_missing` | 缺少资源授权，或资源不存在（访客侧合并） | `sharer` | `share_panel` | 联系分享者 |
| `entry_unavailable` | 入口关闭、过期或未允许该方式 | `sharer` | `share_panel` | 链接不可用，联系分享者 |
| `zone_method_disabled` | Zone 未开启自助注册 / DID 登录 | `admin` | `cp:sso_settings` | 联系管理员 |
| `account_state` | 账号停用、待激活或已删除 | `admin` | `cp:users` | 联系管理员 |
| `credential_state` | 专属邀请过期 / 被替换、密码已设置、无密码 | `self` | `credential` | 按页面提示操作 |
| `source_revoked` | 来源（专属邀请、联合）已撤销 | `admin` | `cp:exclusive_invites` | 联系分享者 |
| `authorization_unavailable` | 授权状态无法确认（依赖服务故障） | `none` | — | 稍后重试 |

- 管理员侧（非受限会话）看到 `adjust`；访客侧只看类别和“联系分享者 / 管理员”。对受限或匿名调用者，资源不存在与无授权一律返回 `resource_grant_missing`。
- 服务日志记录结构化拒绝（主体、类别、服务 / 方法、reason），不记录 token 与 secret。
- 登录、入口等预期失败的方法沿用“成功信封 + `status`”（如 `auth.prepare_app_launch`），`status` 为 `denied` 时附带同一 `AccessDenial`。

## 5. 凭据状态机

### 5.1 密码状态

`UserSettings.password_state`：`set`（缺省）| `unset`。`unset` 时 `password` 为空串。verify-hub 的 `login_by_password` / `sudo_by_password` 先判状态：`unset` 返回 `credential_state`，不进入摘要比较。DID-only 的 External 没有账号，不会走到密码路径。

### 5.2 专属邀请记录

存储：system-config `services/control_panel/exclusive_invites/<invite_id>`，owner 为 CP，写入用 CP 服务身份，`revision` 由 system-config 版本做 CAS（exec_tx 的 main key 版本）。

```json
{"invite_id":"ei-7q3kz9m2x4","user_id":"acme-c1",
 "issuer":{"kind":"user","user_id":"devtest"},
 "channel":{"kind":"email","target_hint":"a***@acme.com"},
 "resource":{"service":"msg-center","kind":"group_session","group_did":"did:bns:support.test","session_id":"t-123","title":"工单 #123"},
 "return_path":"/messagehub?sessionId=t-123",
 "state":"active",
 "credential":{"version":2,"digest":"<base64url sha256(secret)>","issued_at":1792540800,"expires_at":1792713600,"state":"active"},
 "resend":{"requested_at":1792800000,"count":1},
 "created_at":1792500000,"updated_at":1792800000}
```

- 链接：`https://<zone_host>/sso/invite/<invite_id>#s=<secret>`。secret 32 字节随机数 base64url，只在 fragment，不进入请求行、日志、Referer；只保存摘要。
- 有效期固定 48 小时（Q-01）。`credential.state`：`active` | `superseded`（被新版本替换）| `consumed_by_password`（账号已设密码）。
- 一个账号可有多个邀请（Q-10：同客户多工单复用账号，签发方显式指定 `user_id`）；重发只替换同一邀请的资格（Q-02）。

### 5.3 登录判定（`verify-hub.login_by_exclusive_invite`，由 CP 代调）

按顺序：

| 条件 | 结果 |
| --- | --- |
| 邀请不存在或 `state=revoked` | `source_revoked`（不存在与撤销合并） |
| 账号非 Active | `account_state` |
| `password_state=set` | `credential_state` + `password_required`：原地址作入口，用密码登录 |
| secret 摘要不等于当前版本，或 `credential.state≠active` | `credential_state` + `link_replaced` |
| `now > expires_at` | `credential_state` + `expired`，可申请重发 |
| 其它 | 签发 `auth_method=exclusive_invite` 的受限 token pair；family 来源 = `{invite_id, version}`；首次访问不消费资格 |

一次性 callback 仍由 CP pending nonce 保证（60 秒、单次取出）。

### 5.4 首设密码（P-1）

`auth.invite.set_password {new_password_hash}`，调用者为该账号本人会话且 `password_state=unset`。CP 以服务身份执行同一 exec_tx：`password`、`password_state=set`、该账号全部专属邀请 `credential.state=consumed_by_password`；随后调用 `verify-hub.revoke_families {subject, auth_method:"exclusive_invite", except_family:<当前>}`。保留当前终端；其它经免密资格建立的 family 立即不能 refresh，旧 access 至多 15 分钟。`allow_password_change` 不因此改变（Limited 默认仍不能常规改密）。

并发顺序：首设密码的 exec_tx 在 verify-hub 撤销之前提交；verify-hub 签发专属邀请会话时重读账号与邀请，读到 `password_state=set` 即拒绝，因此设密码提交之后不再产生新的免密会话。在提交前已签出的 family 由随后的 `revoke_families` 撤销。

### 5.5 重发（D-9）

- `auth.invite.request_resend {invite_id}`（公开，同一邀请 10 分钟内只受理一次）：仅当邀请 active、账号未设密码、资格已过期或被标记时受理；写 `resend.requested_at`，向签发方发 System Notify（`notify.send`，`to_user=issuer.user_id`，正文含账号与资源标题，不含 secret）。返回 `{status:"submitted"|"rate_limited"|"not_applicable"}`，不返回链接。
- 签发方在 CP“专属邀请”管理处调用 `user.exclusive_invite.rotate {invite_id}`：生成新版本资格（旧版本 `superseded`），新链接只在本次响应中出现一次；签发方经原渠道发出。系统不发送 SMTP / SMS。
- App 签发方的记录见 §5.6。

### 5.6 App 签发的登录码与 `verify-hub.login_by_app_access_token`（EXT-09，2026-10-24；`kind=link` 见 EXT-11）

App 为自己的外部客户代登录（[应用扩展实施 TODO](../../../notepads/self-host-group-extension-todo.md) D-5 / D-7）。复用专属邀请的记录结构与判定表，差别只在签发方、有效期和一次性。`credential.kind`（`link` 缺省 | `code`）标明记录的凭据是 `/sso/invite` 链接的 `#s=` 还是 App 页面兑换的一次性 code，两者不互换：`login_by_exclusive_invite` 只接受 `link`，`login_by_app_access_token` 只接受 `code`，不符返回 `entry_unavailable` + `credential_kind`（排在“记录不存在 / 撤销”之后、账号状态之前）。

- **开关**：`services/control_panel/app_login/<app_instance_id>` → `{enabled, updated_by, updated_at}`，缺省关；owner 以 CP 用户会话调 `apps.app_login.set`。关闭时 CP 对每个绑定账号调 `revoke_families {subject, source_app_instance_id}`。
- **绑定表**：`services/control_panel/app_identities/<app_instance_id>/<digest>` → `{user_id, created_at, last_issued_at, issue_count}`，`digest = base64url(sha256(app_instance_id "\n" external_account_id))`；外部账号原文不落盘。
- **签发** `user.app_login.issue {external_account_id, kind?: "code" | "link", user_id?, show_name?, resource?, return_path?}`：调用者必须是 `principal_kind=app`、target 为自身实例的服务会话。无绑定则新建 Limited 账号 `<app label>-<16 hex>`（不入系统组、`password_state=unset`、无 canonical zone）并写绑定；显式 `user_id` 必须等于已绑定账号。写一条 `ExclusiveInvite`：`issuer={"kind":"app","app_instance_id"}`、`channel.kind="app"`、`credential` 版本 1、有效期 **10 分钟**、`resource` 为 Session 时随后以 App 身份调 `group.invite_session_guest`。返回 `{user_id, code: "<invite_id>.<secret>", expires_at, invite, new_account}`，code 只出现这一次。每实例每分钟最多 60 次签发。
- **`kind: "link"`（EXT-11，D-8，2026-10-24）**：同一记录结构，`credential.kind="link"`、有效期 `EXCLUSIVE_INVITE_TTL_SECS`（48 小时），返回 `{user_id, link: "https://<zone>/sso/invite/<id>#s=<secret>", expires_at, invite, new_account}`，不返回 code。客户走 §5.3 / §10 的专属邀请路径（CP `auth.invite.login` → `login_by_exclusive_invite`，target 为 control-panel）：链接可反复登录、不被消费、设密码后 `password_required`、过期后 `expired`，`auth.invite.request_resend` 对 App 记录返回 `not_applicable`（由 App 自己重签）。verify-hub 为 App 签发的链接登录签出的 family 来源是 `{"kind":"app_login", app_instance_id, invite_id, version}`（而不是 `exclusive_invite`），因此开关、App 运行态与 `revoke_families {source_app_instance_id}` 对它同样生效；登录时也先查 `app_login_available`，关闭即 `source_revoked` + `app_login_disabled`。claims 仍是 `auth_method=exclusive_invite`、Limited 上限、非 restricted scopes。
- **兑换** `auth.app_login.exchange {code, nonce, return_path?}`（CP 公开方法，按来源 IP 限流 30/分钟）→ CP 代调 `verify-hub.login_by_app_access_token {code, nonce, target: App 实例}`（CP-only），判定顺序：

| 条件 | 结果 |
| --- | --- |
| code 格式错误、记录不存在或 `state=revoked` | `source_revoked` / `revoked` |
| 账号非 Active / 非 Limited | `account_state` |
| `password_state=set` | `credential_state` + `password_required` |
| 摘要不等于当前版本 | `credential_state` + `link_replaced` |
| `credential.state=consumed` | `credential_state` + `consumed` |
| `now > expires_at` | `credential_state` + `expired` |
| 记录不是 App 签发，或 `target` ≠ 签发实例 | `entry_unavailable` + `target_mismatch` |
| 开关关闭，或 App 停用 / 卸载（`check_app_runtime_session`） | `source_revoked` + `app_login_disabled` / `app_runtime_*` |
| 同一 code 已兑换且 nonce 相同 | 返回原 token pair（verify-hub 内存保存至 code 到期） |
| 同一 code 已兑换且 nonce 不同 | `credential_state` + `nonce_mismatch` |
| 其它 | 签发 `auth_method=app_access_token`、`restricted=true`、`scopes=[msg_group, account_self]`、target=该 App 实例的 token pair；family 来源 `{"kind":"app_login", app_instance_id, invite_id, version}` |

  兑换成功后 CP 把记录的 `credential.state` 置为 `consumed`（`SetByJsonPath`，只改该字段与 `updated_at`），并像专属邀请一样走 pending nonce → `/sso_callback`，cookie 落在 **App 子域**（host-only），再回到 `return_path`。
- **续期**：refresh 复核账号与类型（收窄）、记录 `state=active`、开关与 App 运行态；target 为 App 实例时 `app_availability` 策略不参与（App 本身为该账号作保），target 为 control-panel 的 `kind=link` family 照常走 target 策略。任一项失败 → `source_revoked`，family 删除。
- **范围**：这是 Account 的受限会话，CP 对它开放与 CP 受限会话相同的 `account_self` 方法（§3.3 的闸口把“App 签入、target 为 App 实例”的会话视同自助会话，因为它运行在 App 子域、经网关 `/kapi/control-panel` 到达）；msg-center 按 `msg_group` 放行 `group.*`，账号没有个人投递记录。App 后端继续用自己的服务身份调系统服务，不转发客户会话（§3.4）。

## 6. 会话与撤销矩阵（D-5、Q-03、Q-04）

| 事件 | 旧 access | refresh（含 grace） | delegate / callback | 流与订阅 |
| --- | --- | --- | --- | --- |
| 资源撤销（移出 / 封禁 Session、grant 撤销、入口关闭、资源删除、作为授权依据的移组） | 下一次受保护请求拒绝 | 不受影响 | 不受影响 | 下一次投递前复核 |
| 账号停用 / 删除 | ≤15 分钟 | 拒绝 `account_state` | 拒绝 | access 到期未续即断 |
| 类型降低 | ≤15 分钟 | 收窄上限 | 收窄 | 同左 |
| 类型升级 / 入组 | 不增权 | 不增权 | 不增权 | — |
| 首设密码 | 当前终端保留；其它免密终端 ≤15 分钟 | 其它免密 family 拒绝 | 同左 | 同左 |
| 专属邀请撤销 | ≤15 分钟 | 该来源 family 拒绝 `source_revoked` | 拒绝 | 同左 |
| 专属邀请自然到期 | 不影响 | 不影响 | 不影响 | — |
| App 代登录关闭 / App 停用、卸载（§5.6） | ≤15 分钟 | 该 App 签入的 family 拒绝 `source_revoked` | 拒绝 | 同左 |
| App 登录码到期 / 已兑换 | 不影响已签会话 | 不影响 | 新兑换拒绝 | — |
| logout / 单设备撤销 | ≤15 分钟 | 拒绝 | 未消费 callback 失效 | 同左 |
| DID 联合会话满 30 天 | ≤15 分钟 | 拒绝，须重新联合 | 拒绝 | 同左 |
| canonical logout / 改密 | 不传播（已知风险） | 不受影响 | 不受影响 | — |
| 受限会话的父 family 撤销 | ≤15 分钟 | 子 family 拒绝 | 拒绝 | 同左 |

- 普通（非受限）CP→App delegate 保持独立 family（现有行为）；受限会话的 delegate 子 family 依赖父 family。
- 授权状态无法确认（依赖服务读失败）一律拒绝 `authorization_unavailable`，不使用过期缓存。
- msg-center 组 reload 失败、HomeStation 联系人缓存：`contact.check_membership` 对系统组投影在请求时读取当前用户组；HomeStation 授权判断不使用 30 秒缓存，上游失败即拒绝。

## 7. family 持久化（D-6）

SQLite：`<BUCKYOS_ROOT>/data/var/verify-hub/sessions.db`（`get_buckyos_service_data_dir("verify-hub")`），单连接，写操作在事务内。

```sql
CREATE TABLE IF NOT EXISTS families (
  family_key TEXT PRIMARY KEY,      -- "<sub>_<target_key>_<session_id>"
  subject TEXT NOT NULL,
  principal_kind TEXT NOT NULL,
  target TEXT NOT NULL,             -- AuthTarget canonical key
  session_id TEXT NOT NULL,
  claims TEXT NOT NULL,             -- JSON: external / restricted / scopes / auth_method
  source TEXT,                      -- JSON SessionSource，可空：exclusive_invite / did_federation / app_login（§5.6）
  parent_key TEXT,                  -- 受限 delegate 的父 family
  absolute_exp INTEGER,             -- 联合会话 created_at+30d；本地会话为空
  created_at INTEGER NOT NULL,
  refresh_jti TEXT NOT NULL,
  refresh_exp INTEGER NOT NULL,
  session_jti TEXT NOT NULL,
  session_exp INTEGER NOT NULL,
  prev_refresh_jti TEXT,
  prev_rotated_at INTEGER
);
CREATE INDEX IF NOT EXISTS families_subject ON families(subject);
```

- 不存 token 明文。grace 分支按记录中的 jti / exp / claims 重新签出同 jti 的 pair（内容等价，签名可能不同）。
- 轮换 CAS：`UPDATE … WHERE family_key=? AND refresh_jti=?`，影响行数为 0 即按现有 Grace / LoggedOut / Reused 规则判定。撤销直接删除行；未消费 pending 随行消失。
- refresh 与 grace 都先复核：账号状态与类型（收窄上限）、来源（专属邀请未撤销）、`absolute_exp`、受限子 family 的父 family 存在。
- 登录 nonce 防重放仍用内存缓存（重启后 8 小时窗口内的重放保护丢失，现有风险，不在本批）。
- 单实例：scheduler 的 verify-hub 期望实例数为 1。数据目录不在 OOD 之间同步；verify-hub 被调度到另一 OOD 或恢复旧数据时，删除 `sessions.db`，所有会话重新登录。sessions.db 不进入备份（恢复旧库会复活已撤销 family），不宣称多副本或故障切换可用。

## 8. 入口与资源授权（D-8）

### 8.1 Zone SSO 设置

system-config `system/sso/settings`（缺省即全部默认值）：

```json
{"self_register_enabled": true, "did_login_enabled": true, "jump_base_url": "https://jump.buckyos.org"}
```

- 两个开关默认开启（D-8a）；没有全局 Guest 开关。`jump_base_url` 为空或缺省时用正式默认值。
- CP：`sso.settings.get`（管理员）/`sso.settings.set`（管理员，sudo 不要求）；公开方法 `auth.sso_config` 只返回 `{self_register_enabled, did_login_enabled, jump_base_url}`。

### 8.2 入口描述

```json
{"entry":{"service":"aiworkspace","entry_id":"se-3f9a"},
 "resource":{"kind":"workspace","title":"DJ 电台","owner_name":"DJ"},
 "methods":{"guest":true,"password":true,"self_register":true,"did_login":false},
 "zone_disabled":["did_login"],
 "state":"active","expires_at":null}
```

- 资源服务的公开方法返回入口原始允许方式（AIWS `entry.describe`、msg-center `group.describe_entry`）；CP `auth.entry.describe {service, entry_id}` 再与 Zone 开关求交，`zone_disabled` 列出被 Zone 关闭的方式（分享面板据此显示“已被 Zone 设置关闭”）。
- 公开描述只含标题、类型、所有者展示名，不含正文、成员或客户名单。入口不存在、关闭、过期统一 `state:"unavailable"`。
- `password` 表示“已有本 Zone 本地账号可登录”；`self_register` 隐含 `password`。

### 8.3 AI Workspace 入口

`local.sqlite` 新表（与 grants 同库）：

```sql
CREATE TABLE IF NOT EXISTS share_entries (
  entry_id TEXT PRIMARY KEY,
  scope_entity_id TEXT NOT NULL DEFAULT '',
  capabilities INTEGER NOT NULL,     -- 权限模板（CapSet，不含 manage）
  methods INTEGER NOT NULL,          -- bit0 guest, bit1 password, bit2 self_register, bit3 did_login
  state TEXT NOT NULL,               -- active | closed
  expires_at INTEGER,
  created_by TEXT NOT NULL,
  created_at INTEGER NOT NULL
) WITHOUT ROWID;
```

- 只有对该 scope 持有 `manage` 的人能创建、关闭入口。
- Guest：入口 active 且允许 guest 时，`guest:` 主体在该 scope 获得入口模板能力（每次请求计算，入口关闭即失效）。现有 `public` grant 保留为“公开阅读”。
- 已认证访问者：`ws.join_entry {entry_id}`，按 token 的 `auth_method` 与类别检查入口方式（`did_federation` 需 `did_login`；本地账号需 `password` 或 `self_register`），通过后为该主体写一条 grant（记录 `via_entry`）。关闭入口不回收已加入者的 grant，回收走 `ws.revoke`。
- 授权对象：`<user_id>`（账号）、`<did>`（External）、`group:<id>`（系统用户组，服务端解析调用者所属组）、`contact:<owner_did>#<tag>`（经 `contact.check_membership` 精确查询）、`public`、`*`。`*` 只匹配非受限的 Admin / User 账号。
- **工作区页面在自己的源上**（AI Workspace 内核架构改进 T4-8，2026-10-28）：`entry.describe` / `ws.join_entry` 返回的 `resource.path` 是资源页 URL——工作区为绝对地址 `https://aiws-<id>.<zonehost>/<workspace slug>`（无 public base 时为 `/`），不再是 Zone 相对路径。该源持有自己的会话：网关把 `aiws-<label>.<zone>` 折到 `aiws` 服务项，Control Panel 解析 `redirect_url` 时同样折叠，登录目标为 `System(aiworkspace)`（已加入可交互登录目标），`/sso_callback` 在该主机落 host-only cookie、`/sso_refresh` 续期。已登录的 Desktop 用户由 `auth.prepare_app_launch { service_id: "aiworkspace", redirect_url }` 委派（verify-hub `delegate_user_session` 允许 `App` 与可委派系统服务 `aiworkspace`；External 仍不可委派），Zone 根 `/login?redirect_url=<aiws url>` 对已登录者自动完成这一接力。所有 `aiws-*` 主机共用该目标：cookie 按主机隔离，token 不绑定工作区，服务按 grant 判权。

### 8.4 Message Hub 入口

- 专属邀请：管理员创建专属邀请时指定 Session，CP 以管理员会话调用 `group.invite_session_guest` 邀请该账号 DID；客户登录后在受限页面接受邀请（`accept_session_invitation`），登录本身不等于加入。
- 公开入口：群 `membership.guest_entry` 增加 `auth_methods: {password, self_register, did_login}`（缺省全 false，即只接受已有成员或邀请）；`group.describe_entry {group_did}` 公开返回群名与方式；访客认证后调用现有 `group.submit_guest_request`。Message Hub 不支持匿名 Guest。

## 9. DID 联合（D-7）

### 9.1 流程与方法

1. 目标 Zone B：页面调用 CP `auth.did.begin {did, redirect_url}`。CP 解析 `did → OwnerDocument → canonical zone → ZoneDocument.hostname`（canonical zone 取 `binded_zone_list` 首项；列表为空时取 DID 自身——自托管 Zone 的 owner DID 就是 Zone DID，如 `did:bns:alice` 拥有 `alice.bns.did`，见 `buckyos_api::owner_canonical_zone`），生成 `state`（32 字节）、`nonce`、PKCE `code_verifier`（S256 得 `code_challenge`），写 pending（10 分钟，内存），返回 `{status:"redirect", url}`：

   `https://<canonical_host>/sso/federate?did=<did>&client_zone=<B zone DID>&redirect_uri=https://<B host>/sso/did_callback&state=..&nonce=..&code_challenge=..`

2. canonical Zone A：页面 `/sso/federate`（需 A 的登录态）调用 CP `auth.federation.describe {…}`：校验 `redirect_uri` 的 host 等于 `client_zone` 的 ZoneDocument hostname、路径固定为 `/sso/did_callback`；当前账号 DID 等于 `did`；账号类型可对外联合（Admin / User）且 Active；该账号 OwnerDocument 的 canonical zone（同上规则）是本 Zone。返回 consent 内容与是否已同意。用户确认后 `auth.federation.approve {…, remember:true}`：CP 记录 consent（`users/<id>/federation_consents` 中 `client_zone → {granted_at, profile_scope:1}`），调用 verify-hub `issue_federation_code`，返回 `{status:"redirect", url:"<redirect_uri>?code=..&state=.."}`。取消返回 `<redirect_uri>?error=access_denied&state=..`。

3. B：`GET /sso/did_callback?code&state` 由 CP 处理：取出 pending（一次性），调用 B 的 `verify-hub.login_by_remote_verify_hub`。

4. B 的 verify-hub 独立解析 `did` 得到 canonical hostname，HTTPS POST `https://<canonical_host>/kapi/verify-hub` 方法 `exchange_federation_code`；校验返回的 `did`、`client_zone == 本 Zone`、`nonce`、`purpose == "login"`；映射到已入驻账号（`users/*/doc.id == did` 且 Active）或签发 External；family `auth_method=did_federation`，`absolute_exp = now + 30d`。CP 在同一响应写 Cookie 并 302 到 pending 的回跳地址。

### 9.2 Wire

```json
// verify-hub.issue_federation_code（仅 CP 服务会话可调）
{"user_id":"alice","did":"did:bns:alice","client_zone":"did:bns:b","redirect_uri":"https://b.example/sso/did_callback",
 "nonce":"…","code_challenge":"…","profile":{"show_name":"Alice","avatar":null}}
→ {"code":"fc-…","expires_in":60}

// verify-hub.exchange_federation_code（公开，HTTPS；code 本身是凭据，PKCE 绑定 verifier）
{"code":"fc-…","code_verifier":"…","client_zone":"did:bns:b","redirect_uri":"https://b.example/sso/did_callback"}
→ {"status":"ok","assertion":{"did":"did:bns:alice","client_zone":"did:bns:b","nonce":"…","purpose":"login",
   "issued_at":1792540800,"profile":{"show_name":"Alice","avatar":null}}}
→ {"status":"invalid_code"}            // 不存在、已用、过期、verifier / client_zone / redirect_uri 不符

// verify-hub.login_by_remote_verify_hub（仅 CP 服务会话可调）
{"did":"did:bns:alice","code":"fc-…","code_verifier":"…","nonce":"…","redirect_uri":"https://b.example/sso/did_callback","target":{"kind":"system","service_id":"control-panel"}}
→ {"status":"issued","user_id":"alice","token_pair":{…},"profile":{"show_name":"Alice"}}   // External 时无 user_id；CP 据此构造 CurrentIdentity
→ {"status":"denied","denial":{…AccessDenial…}}
→ {"status":"canonical_unavailable"}    // 解析或兑换网络失败，可重试
```

- DID 已对应本 Zone 账号但该账号非 Active 时拒绝（`account_state`），不降级签发 External；Zone 关闭 DID 登录时 verify-hub 同样拒绝（`zone_method_disabled`）。External 会话不能 delegate 到 App（v1）。
- code：`fc-` + 32 字节随机数，canonical verify-hub 内存保存 60 秒、单次使用；绑定 DID、client_zone、redirect_uri、nonce、code_challenge、资料范围。
- 兑换主机只来自 DID 解析链；远端 key 不进入本地 `verify-hub` 信任槽，`login_by_jwt` 继续拒绝 verify-hub session。
- 开发环境可在 `system/sso/settings` 设 `dev_federation_http: true` 允许 HTTP 兑换，只用于本机联调。
- consent 资料范围（Q-05）：DID、来源与目标 Zone、公开显示名 / 头像。

## 10. 浏览器流程

| 入口 | 页面（Desktop SPA，公开路由） | CP 方法 |
| --- | --- | --- |
| 分享入口 | `/sso/entry?service=&entry=` | `auth.entry.describe`、`auth.login`、`auth.register`、`auth.did.begin` |
| 专属邀请 | `/sso/invite/:inviteId#s=` | `auth.invite.login`、`auth.invite.request_resend`、`auth.invite.set_password` |
| DID 回跳 | `GET /sso/did_callback`（CP HTTP） | — |
| canonical consent | `/sso/federate?…` | `auth.federation.describe`、`auth.federation.approve`、`auth.federation.deny` |

- 登录类方法返回 `{status, sso_nonce?}`；成功后页面跳到 `/sso_callback?nonce=&redirect_url=`（现有 pending，60 秒单次）。`redirect_url` 由服务端解析为本 Zone 路由（现有 `resolve_sso_auth_target`），不接受外站。
- 身份冲突：页面先经 `/sso_refresh` 取当前身份；与入口指定身份不同时显示“继续以 X 进入 / 取消”，取消不调用登录方法、不覆写 Cookie。
- 资源恢复：入口记录的 `return_path`（专属邀请）或页面的原始 URL（分享入口）作为 `redirect_url`；目标失效时页面显示原因码对应提示。

## 11. jump

- 有效入口 = `system/sso/settings.jump_base_url`，缺省 `https://jump.buckyos.org`；SDK `buildJumpUrl(action, params, jumpBase)` 拼 `<jumpBase><registered-action-path>#<params>`，参数放 fragment 不进入 jump 的 HTTP 请求；jump 去掉候选参数 `zone` 后把其余参数作为 query 交给目标 Zone 页面。
- 登记的 action 只含目标 Zone 已有页面的：`app_installer → /sysdlg/app_installer`（jump 与目标 Zone 用同一路径）；`saveas` 等在 Zone 侧对话框实现后再登记，未登记 action 拒绝。目标 Zone 侧 action 地址 = `https://<hostname><registered-action-path>`，hostname 规范化后拒绝 userinfo、端口、path、query、fragment。
- jump 页面（静态）不在本仓库；本批交付 SDK 的 URL 构造与 hostname 校验、目标 Zone 侧页面的 pending 恢复（`/sysdlg/*` 路由在未登录时经 `/login?redirect_url=` 回到原 action，GET 只展示确认页）。

## 12. 持久状态清单

| 记录 | owner | 键 | 唯一性 / CAS | 期限与删除 |
| --- | --- | --- | --- | --- |
| 账号 | CP（system-config） | `users/<id>/{settings,doc,profile}` | Create 防重名；身份字段只经 CP | 删除=state deleted，保留 |
| 专属邀请 / App 登录码 | CP | `services/control_panel/exclusive_invites/<invite_id>` | exec_tx 主键版本 | 撤销后保留记录；App 登录码 10 分钟后只剩审计价值 |
| App 代登录开关 | CP | `services/control_panel/app_login/<app_instance_id>` | exec_tx 主键版本 | 卸载不自动删除；读为缺省关 |
| App 外部账号绑定 | CP | `services/control_panel/app_identities/<app_instance_id>/<digest>` | Create 防重；更新 CAS | 保留；账号删除后 `issue` 拒绝 |
| Zone SSO 设置 | CP | `system/sso/settings` | 覆盖写 | — |
| federation consent | CP（canonical 侧） | `users/<id>/federation_consents` | 覆盖写 | 用户撤回即删 |
| session family | verify-hub | `data/var/verify-hub/sessions.db` | 行级 CAS | refresh 到期 / 绝对期限 / 撤销即删 |
| 浏览器 pending（SSO、DID） | CP 内存 | nonce / state | 单次取出 | 60 秒 / 10 分钟 |
| 联合 code | canonical verify-hub 内存 | code | 单次 | 60 秒 |
| AIWS 入口 | AIWS `local.sqlite` | `share_entries.entry_id` | 主键 | 关闭保留 |
| AIWS grant | AIWS `local.sqlite` | `(subject, scope)` | 主键 | revoke 删除 |
| Session 成员与邀请 | msg-center 群存储 | 现有 | 现有 epoch | 现有 |
| jump 偏好 | jump 页面 localStorage | `current_zone_hostname`、`auto_jump` | — | 可丢失 |

编排：专属邀请创建依次提交“账号”（若新建）→“邀请记录”→“资源邀请”（msg-center），每步幂等（以 `invite_id` 为关联键，重复提交跳过已完成步骤）；资源邀请失败时邀请记录保留 `resource_state:"pending"`，可经 `user.exclusive_invite.retry_resource` 重试；身份创建本身不授予资源权限。

## 13. 已知风险与限制

- 管理员不能一键关闭全 Zone 匿名访问；Guest 只有资源级 `guest:` 主体（D-8）。
- canonical logout / 改密不传播，联合会话最长 30 天（Q-04）。
- 账号停用、来源撤销、logout 后旧 access 最长残留 15 分钟（D-5）。
- verify-hub 单实例，迁移或恢复后全部重新登录（D-6）。
- 登录 nonce 防重放缓存在内存，重启后丢失。
- App 登录码的“已兑换”标记在 verify-hub 内存与 CP 记录各一份：verify-hub 重启后，同 nonce 的幂等重试会得到 `consumed`（需重新领 code），不会重复签发。
- `allowed_clients` 不能区分系统 SPA：Message Hub、Desktop、AI Workspace 的 `appid` 都是 `control-panel`（扩展 TODO D-4）。
- External（DID 登录）仍不能 delegate 进 App；外部客户进 App 只能经 App 代登录（§5.6）。

### 登录请求方展示

登录页通过公开 CP RPC `auth.describe_target {redirect_url}` 获取 `{target, display_name}`。
CP 与 `auth.login` 共用 redirect URL 校验和网关路由解析；`target` 是结构化的
`AuthTarget`（App 的 `app_instance_id` 或 System 的 `service_id`）。App 名称取安装
规格的 `app_doc.show_name`，缺失时显示 appid；系统入口使用服务展示名。
前端解析失败时隐藏请求方，不从域名片段推断应用或用户名。此接口仅提供展示信息，
不签发 token；提交登录时仍重新解析和校验目标。
