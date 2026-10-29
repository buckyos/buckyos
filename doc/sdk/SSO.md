# BuckyOS 中的 SSO（使用者指南）

本文从**接入方（App / Web 开发者）视角**讲清楚怎么在 BuckyOS 里完成登录，以及它和主流 SSO（OAuth / Google 登录 / 企业 IdP）"套路"上的关键区别。

> 这是一份使用文档，不是协议规范。
> - 当前 control panel 的 canonical 认证契约见 `doc/control_panel/Control_Panel_Service.md`。
> - 名字解析、跨 Zone 信任根、钱包登录的完整设计见 `doc/arch/BNS 去中心的名字系统.md`（场景 6）。
> - 用户类型、登录方式、sudo 的产品定义见 `product/users_and_agents/UserType.md`。
> - 标注为「协议目标 / 规划中」的部分尚未在本仓库完整落地，接入时以「当前实现」小节为准。

## 一句话套路

> 把每个用户自己的 **Zone（的 verify-hub）当成 IdP**，App 通过 **DID 解析**发现该 IdP，登录后拿到一个**绑定到明确 AuthTarget 的 session_token**，之后所有请求都只认这个 token + RBAC。

和主流 SSO 一样的部分：有登录页、有回调、有 access/refresh token、有 scope；接入方最终只关心"拿到一个可信 token"。
不一样的部分见下一节——这是本文的重点。

## 和主流 SSO 最不一样的地方

| 维度 | 主流 SSO（OAuth / Google / 企业 IdP） | BuckyOS SSO |
| --- | --- | --- |
| IdP 是谁 | 固定的中心化大平台 | **用户自己的 Zone**。每个 Zone 的 verify-hub 就是一个 IdP，没有中心 |
| 怎么找到 IdP | 写死的 endpoint / 配置 | **DID 解析**：`user_did -> owner -> default_zone -> ZoneConfig.verify_hub_info.public_key` |
| 信任根 | CA / 平台公钥 | Owner / Zone 的 DID 公钥（可不依赖 CA，见 BNS 文档场景 5） |
| token 绑定 | client_id | **AuthTarget**：普通 App 精确到 `AppInstanceId`，系统服务精确到 `SystemServiceId` |
| 没有账号的用户 | 必须先在平台注册 | 可以**用浏览器钱包**对登录 challenge 签名，证明"我控制这个 DID"，无需任何 Zone |
| 跨身份提供方 | 换一个大平台 | **跨 Zone**：current-zone 证明"你是谁"，target-zone 重新签发自己的 token |

实务上要记住三句话：

1. **token 是 target 绑定的**。页面 token 代表“当前操作者 + 这个精确 AppInstance/系统服务”，不要跨 target 复用，也不要拿 app-service 自己的 token 冒充页面 token（见后文「不要混用 token」）。
2. **登录方式对鉴权方透明**。业务服务只验证 session_token + 跑 RBAC，不关心用户是用密码、钱包还是 Passkey 登录的。新增登录方式不影响下游。
3. **强权限要 sudo**。高危操作需要二次输入密码换一个短时 sudo token，且仍受 appid 限制。

---

## 最小接入：同 Zone 内的 Web App

这是最常见的场景：你的 App 跑在用户 Zone 的某个子域（如 `myapp.alice.buckyos.io`），要让用户登录。

### 1. 跳转到登录页

用 websdk 的 `AuthClient`：

```ts
import { AuthClient } from "buckyos";

const auth = new AuthClient(zoneHostname /* 如 "alice.buckyos.io" */);

// 跳到 Zone 的登录页，登录完成后回到 redirectUri（默认当前页）
auth.login(/* redirectUri? */);
```

`buildLoginURL()` 生成的目标就是 Zone 的 control panel 登录页：

```text
https://sys.<zoneHostname>/login?redirect_url=<encoded redirect>
```

> `/login` 是 control panel 自身的登录页；当带上 `redirect_url` 时，它同时充当 SSO 授权页。登录目标由 redirect URL 对应的 Gateway 路由确定。（历史上也规划过独立的 `/sso/login` 授权弹窗页。）

### 2. 登录完成 → 回调写 Cookie

用户在登录页完成认证后，control panel 走 `/sso_callback?nonce=...&redirect_url=...`：

- 校验 `redirect_url` **必须是本 Zone 内由 Gateway 实际路由的目标**，并从 `app_info` 得到结构化 target：根域 `_` 是 `System(control-panel)`，普通 App entry 必须同时给出且互相匹配 `app_id + app_instance_id + app_owner_user_id`。
- 正式环境只接受 HTTPS；DV 的 HTTP 必须由显式配置开启。URL 禁止 credentials 和任意端口。pending login 会保存 canonical origin、完整 redirect 和 target；callback 的参数、实际 request origin 和当时 Gateway route 必须全部完全一致。
- 回调由目标 App origin 承载，同时写入两枚 host-only Cookie：短期 `buckyos_session_token` 供 gateway 在 App 页面加载前完成鉴权，长期 `buckyos_refresh_token` 使用 `HttpOnly`、只供刷新。随后 302 回 `redirect_url`。两枚 cookie 都不设置 `Domain`，因此不会与父域或其它 App 子域共享。

### 3. 用 refresh cookie 换 session_token

页面加载后，向 control panel POST `/sso_refresh`（自动带上 HttpOnly cookie）：

```text
POST /sso_refresh
-> 200 { "session_token": "...", "identity": { "kind": "account" | "external", ... } }
```

- 返回**短期 session_token**（放在响应体里，供 JS 读取并用于后续请求）和当前身份 `identity`：`account`（本 Zone 账号，含 `user_id`、`did`、`user_type`、`restricted`、`scopes`、`password_set`）或 `external`（本 Zone 无账号的已认证 DID）。形状与 fixtures 见 [混合 SSO 技术契约](<../arch/verify-hub/混合SSO技术契约.md>) §2.2。websdk 用 `buckyos.getCurrentIdentity()` 读取，`getAccountInfo()` 只对账号会话返回值；身份变化时清空缓存，不回退旧身份。
- 同时轮换 `buckyos_refresh_token`，并把短期 token 写入独立的 host-only `buckyos_session_token`。被替换的旧 refresh token 只有 10 秒宽限期：同一 host 的多个窗口共用这枚 cookie，同时续期时，后到的请求拿到的是同一对新 token，不会撤销会话；超出宽限期再出示旧 token 按重用处理，整条会话被撤销。
- session 快过期时再调一次 `/sso_refresh` 续期即可，无需重新登录。

### 4. 带着 session_token 调 kRPC

后续所有受保护请求都带上 session_token，支持以下任一方式：

```text
Header:  X-Auth: <session_token>
Header:  Authorization: Bearer <session_token>
Query:   ?session_token=<...>  或  ?auth=<...>
kRPC:    请求体的 token 字段
```

服务端用 Zone 的 trust keys 验签，确认用户 active，再走 RBAC 判权。

### 5. 退出

POST `/sso_logout`：吊销 refresh token，并清掉当前 App host 下的 `buckyos_refresh_token` 与 `buckyos_session_token`。

---

## 从桌面打开 App（免密启动）

用户已经登录 BuckyOS 桌面（Zone 根域上的 `System(control-panel)` 会话）时，从桌面打开一个有权访问的 Web App 不会再出现登录页。桌面不把自己的 token 交给 App，也不共享父域 Cookie，而是为目标 App 实例换一张一次性票据：

```text
1. 桌面先构造最终 App URL（含 path、query 和 AppFrame 的 #bfp=<frame nonce>），然后
   POST /kapi/control-panel  auth.prepare_app_launch
     params: { app_instance_id, redirect_url: <最终 App URL> }   sys[1] = 桌面 session token
2. Control Panel 校验来源是普通（非 sudo）Control Panel 用户会话，按 Gateway route 解析
   redirect_url 得到 AuthTarget::App，并要求与 app_instance_id 完全一致；
   再以自己的服务会话调用 verify-hub delegate_user_session，换出同一用户、目标实例的
   普通 User token pair，存进 60 秒的一次性 pending（与密码登录共用）。
   响应只含 callback_url，不含 token：
     { "status": "ready", "callback_url": "https://<app-host>/sso_callback?nonce=…&redirect_url=…" }
3. 桌面让 iframe 导航到 callback_url；后续与上文第 2–4 步相同：/sso_callback 写入目标 host 的
   两枚 host-only Cookie 并 302 回完整 redirect_url，App 页面再用 /sso_refresh 恢复会话。
```

新 token 的 `sub` 仍是操作者，`app_owner_user_id` 是安装者；它不继承桌面的 system target，也不带 sudo。用户仍受 App availability 和业务 RBAC 约束。

`auth.prepare_app_launch` 把预期内的失败放在成功响应的 `status` 里，桌面只按这个枚举分支（kRPC 错误在线上只是文本）：

| `status` | 含义 | 桌面处理 |
| --- | --- | --- |
| `ready` | 附带 `callback_url` | 导航 iframe |
| `source_session_inactive` | 桌面会话认证失败，或来源会话已注销、不存在（如 verify-hub 重启） | 强制刷新一次桌面会话后重试一次；仍是该状态显示错误和重试；刷新失败才进入登录页 |
| `access_denied` | availability 拒绝：无权、实例不存在或已删除、owner 不可用 | 显示无权访问，不自动重试 |
| `invalid_target` | URL/route 不合法、不在本 Zone、不是 App route，或与 `app_instance_id` 不一致 | 显示无法打开；内容打开回退到 Preview |

kRPC error、HTTP 非 200 和网络错误都按暂时性失败处理（显示重试，不跳登录页）。

verify-hub 的 `delegate_user_session` 只接受 Control Panel 服务会话作为调用方（`sys` 中的 token 必须是 verify-hub 签发、`principal_kind` 为 System 或 Device、target 为 `System(control-panel)`、非 sudo）。`/kapi/verify-hub` 对浏览器可达，用户会话或其它服务直接调用会被拒绝，浏览器因此拿不到委托出的 token pair。来源 token 必须是本 Zone verify-hub 签发、仍属于活跃会话（未注销）的 Control Panel 用户 session；来源会话 refresh 后，同一会话里旧但未过期的 access token 仍可用。桌面 logout 完成后不能再提交新委托。

退出语义与已知取舍（第一版有意接受）：

- 委托出的 App 会话与桌面会话相互独立：桌面 logout 只阻止之后的新委托，已签发的 callback 票据仍按 60 秒 TTL 可用，App 会话按自身 logout/有效期结束，refresh 轮换后可一直续期。因此泄露一枚 15 分钟有效的桌面 session 就能换出长期有效的 App 会话。
- 每次启动都签一条新会话并覆盖目标 host 的 Cookie，不探测已有会话。被覆盖的旧会话和过期未兑换的 pending 一样留在 verify-hub 内存里，直到 refresh 过期（7 天）被清理。
- refresh 宽限期内，出示上一枚 refresh 能拿到当前 token；被窃取的旧 refresh 只在合法轮换后的几秒内有用。
- 同一 App host 的多个窗口共享这组 Cookie：从另一桌面用户重新启动会把该 host 上的登录切换成新用户。
- pending 票据在 Control Panel 进程内存，会话缓存在 verify-hub 进程内存；第一版沿用单实例部署前提。

---

## Session Token 是什么

登录的目标是让 client 拿到一个可保存、可复用的 **Session Token**，它代表一个已完成认证的会话，至少包含：

```text
sub / userid   当前用户（username 或 DID）
principal_kind 登录主体类型；用户页面会话固定为 user
target_kind    app | system
appid          AppId 或 SystemServiceId 的兼容 claim
app_instance_id / app_owner_user_id  App target 必填且互相匹配；System target 禁止出现
token_use      session | refresh；sudo 仍是 session 的提权子类型
exp / jti      过期时间、nonce
iss            签发方（Zone 的 verify-hub）
```

> 一次登录通常返回**短期 session_token + 长期 refresh token**。session 过期用 refresh 流程换新；refresh token 一旦使用立即轮换，旧的只在 10 秒宽限期内返回当前那对 token，之后再用即视为重用并撤销整条会话。

对鉴权方而言，登录方式（密码 / 钱包 / Passkey）是**透明的**——只看 token 可信不可信、token 里声明的用户/App/权限上下文是什么。这就是为什么扩展登录方式不需要改下游业务。

---

## 跨 Zone SSO（DID 登录，去中心 IdP）

当用户想登录的目标 Zone 不是自己的 Zone 时：

- **canonical zone**（PRD 中与 current zone 同义）：用户本人唯一指定的身份归属 Zone，即他的 IdP。
- **目标 Zone**：他想进去操作的 Zone。

已实现的流程是后端 HTTPS 一次性授权码交换（D-7，不公布 verify-hub 公钥）：

```text
1. 目标 Zone 页面输入 DID → CP auth.did.begin：resolve(did) → OwnerDocument → canonical zone → ZoneDocument.hostname，
   （canonical zone = binded_zone_list 首项；为空时是 DID 自身，即 owner DID 就是 Zone DID 的自托管 Zone）
   生成 state / nonce / PKCE verifier，跳到 https://<canonical>/sso/federate?...
2. canonical Zone（用户已登录）显示 consent：DID、目标 Zone、公开显示名 / 头像；以 DID + 目标 Zone 记住同意。
   只有 Active 的 Admin / User、且该 Zone 是其 canonical zone 时才签发 code（60 秒、单次）。
3. 浏览器回到目标 Zone 的 /sso/did_callback，CP 调 verify-hub.login_by_remote_verify_hub。
4. 目标 verify-hub 自行解析 canonical 主机，HTTPS 调 exchange_federation_code（PKCE 校验），核对 DID / 目标 Zone / nonce，
   映射到已入驻账号，否则签发 External 会话；联合会话最长 30 天，续期不回 canonical 复核。
```

要点：

- 用户最终拿的是 **Target Zone 自己签发的 token**，并继续受 Target Zone 的 RBAC / scope 约束。
- Current Zone 的凭证只负责"证明身份"，**不自动代表在 Target Zone 有任何权限**。
- 和 Google SSO 本质相同（信任一个外部 IdP 完成认证），区别只在 IdP 是"用户自己的 Zone"，且发现/信任根来自 DID + ZoneConfig + verify-hub 公钥。

> 跨 Zone 内部 DID 通常是明确的——系统凭证核心是 username / DID，每个可登录用户都能映射到一个确定 DID。账号 → DID 只认 `users/<id>/doc`，本人可编辑的 profile 不是身份证明。canonical 的 logout / 改密不传播到目标 Zone（已知风险，最长 30 天）。

---

## 外部访问者入口（混合 SSO）

分享资源时，资源服务持有“入口”，入口声明允许的进入方式；CP 把它与 Zone 开关（`system/sso/settings` 的自助注册 / DID 登录，默认开启）求交后展示在 `/sso/entry?service=&entry=` 页面：

- **Guest**：AI Workspace 的 `gt_` → `guest:` 资源主体，无系统会话；由资源 owner 在入口上开启。
- **本 Zone 账号登录 / 自助注册**：自助注册建 Limited 账号（不入系统组、不设 canonical zone），之后按普通密码登录。
- **DID 登录**：见上一节。
- **专属邀请**：管理员预建 Limited 账号并生成 `https://<zone>/sso/invite/<id>#s=<secret>`，secret 只在 fragment；48 小时内可重复免密进入，设密码后旧链接只作入口；过期后访客申请重发，签发方收到 System Notify 后在“用户与 Agent / 专属邀请”生成新链接，经原渠道发送（系统不发短信 / 邮件）。

**AI Workspace 页面**（2026-10-28）：每个工作区是自己的站点 `https://aiws-<id>.<zone>/<工作区 slug>/<画布 slug>?id=<画布 id>`（`<id>` 去掉 `ws_`），由 aiworkspace 服务出页。它的登录态在该主机自持：从桌面或列表打开时 CP 用 `auth.prepare_app_launch { service_id: "aiworkspace", redirect_url }` 委派会话并经一次性 `/sso_callback` 落 cookie；直接打开链接而没有登录态时，公开工作区以访客身份打开，否则到 Zone 根 `/login?redirect_url=` 登录（已在 Zone 根登录者自动接力）。入口页进入工作区时也走这一步。

Limited、External 及其衍生会话是**受限会话**：token 带 `restricted: true` 与 `scopes`（`account_self` / `msg_group` / `aiworkspace` / `homestation` / `aicc`）。`BuckyOSRuntime::verify_trusted_session_token` 默认拒绝受限会话，服务对需要开放的方法改用 `verify_session_token_opened(token, scope)`；类型升级不会让旧会话增权，降级在下一次 refresh 收窄。拒绝在 kRPC 失败响应中带 `code: "access_denied"` 与 `data: AccessDenial`（原因码与调整方），websdk 用 `accessDenialOf(error)` 读取，不解析错误文本。

第三方网页不知道用户 Zone 时，用 websdk `buildJumpUrl(action, params)` 跳到 jump（默认 `https://jump.buckyos.org`，可由 `system/sso/settings.jump_base_url` 覆盖），由用户选择目标 Zone 后到达该 Zone 的同路径 action 页面，仍需在该 Zone 登录并确认操作；jump 页面见 `src/frame/jump/`。

---

## 没有 Zone 的用户：浏览器钱包登录

没有 Zone 的用户没有 verify-hub，签不出 Zone session token，但他仍能通过钱包证明"我控制这个 DID / owner key"。

```text
1. 联合登录页做 capability detection：检测浏览器是否有 BuckyOS wallet 扩展。
   - 没有 -> 回到普通 Zone SSO / 输入 DID / 装钱包 / 建 Zone 的引导。
   - 有   -> 展示"用钱包登录"。
2. App 生成登录 challenge（app_did + nonce + redirect_uri + requested_scope）。
3. 钱包返回 active DID / 公钥；若是 did:bns 则 resolve(wallet_did, owner) 拿可信公钥。
4. 钱包对登录断言签名（覆盖 app_did / challenge / redirect_uri / wallet_did / iat / exp / scope）。
5. App 用 BNS OwnerDocument 公钥验证签名。
```

结果是一个 **wallet-signed login assertion**，不是 Zone session_token。适合：跨应用登录、证明钱包身份、读公开 profile、买内容、引导建 Zone。**不能**自动获得任何已有 Zone 的私有资源权限——要访问 Zone 资源仍走该 Zone 的 verify-hub / RBAC。

> 产品建议：钱包登录优先用于"忘记密码 / 账号恢复"，日常登录不鼓励频繁暴露 Root/Control Key 的签名能力。

---

## 用户类型与匿名访问（接入方需要知道的）

不同用户类型决定了 App 能拿到什么权限（完整定义见 `product/users_and_agents/UserType.md`）：

- **admin / su_admin**：可登录用户中权限最大；敏感操作需 sudo。
- **user / su_user**：最常见的登录用户，能用 App 但不能管系统。
- **limited**：零权限底座的登录用户（企业客户、员工、未成年人等）。默认不能用任何 App，由管理员加入系统用户组（`group:<id>`）后，经 App 可用性规则和资源授权开放；不签发对外联合断言。
- **root**：Zone owner 的账号记录，不可登录。
- **External**：已认证、但本 Zone 没有账号记录的 DID（经 DID 登录产生）。token 的 `principal_kind` 仍为 `user`，另带 `external: true` 标记，`sub` 是原 DID；只有 verify-hub、msg-center 群路径、AI Workspace、HomeStation 接受，其它服务由受限会话闸口拒绝。
- **匿名访问者**：不是用户类型。默认**全部拒绝**；只有明确公开的资源（App 可用性的 `guest` 伪组、AI Workspace 的 `guest:` 主体等）才允许匿名访问。

让一个传统 app 提供公共服务的方式：把 app 设为 gateway 公共访问（= 授予特殊权限），或 app 自带用户管理、在自己的 app data 范围内管理读写。

---

## sudo：高权限二次确认

sudo 由 verify-hub 提供：弹出提权对话框，要求管理员**再次输入密码**，换发一个**短时（约 3 分钟）SUDO Session Token**，可能带明确操作边界。

接入要点：

- sudo token 仍受 **AuthTarget 限制**，必须是 `token_use=session, sudo=true`；它不能写进普通页面 session cookie。
- 所以 sudo 一般只在 Control Panel 这类**本身就有大权限的系统 UI**里才有意义。

---

## 不要混用两种 session_token

> 这是一条容易踩坑的架构不变量。

在 `app-web-page -> app-service -> kRPC` 链路里，**必须继续使用来自页面的 token**，不能换成 app-service 自己的 token：

- 页面 token 代表**当前操作者**。
- app-service 自己的 token 往往只代表 **app owner**。

用错会导致越权或审计主体错乱。

---

## 兼容（未接入 websdk 的历史 app）

历史 Web app 没接 `buckyos-web-sdk` 时，node-gateway 可以把首个请求导向 `login_index.html` 完成 cookie 写入。这是**兼容迁移路径**，新接入请直接用 `AuthClient` + `/sso_refresh`，不要依赖这条路径。

---

## 当前实现 vs 协议目标

**当前已经能用（本仓库已落地）：**

- verify-hub `auth.login`（用户名/DID + 密码）签发 session_token + refresh_token。
- websdk `AuthClient.login()` -> `sys.<zone>/login?redirect_url=`。
- control panel `/sso_callback`（把 nonce、canonical origin/route 与 AuthTarget 一起校验后才写 cookie）、`/sso_refresh`（只接受同 origin/route 的 refresh target并轮换 token，旧 refresh 有 10 秒宽限期）、`/sso_logout`。
- 桌面免密启动 App：control panel `auth.prepare_app_launch` + verify-hub `delegate_user_session`（仅 Control Panel 可调用）。
- session_token 多通道传递（X-Auth / Bearer / query / kRPC token）+ trust keys 验签 + RBAC。
- sudo 机制（提权对话框 + 短时 sudo token，受 appid 限制；Limited 不签发 sudo）。
- refresh family 持久化在 verify-hub 本地 SQLite（`data/var/verify-hub/sessions.db`），重启后可继续刷新；单实例，迁移到其它 OOD 后需重新登录。
- 混合 SSO：当前身份三态、受限会话闸口与原因码、专属邀请、自助注册、入口描述、DID 登录（授权码交换）、jump 静态页。

**协议目标 / 规划中（接入时不要假设已可用）：**

- 浏览器钱包扩展 detection 与 wallet-signed assertion 的标准格式（首期不展示钱包入口）。
- App access token 登录（`login_by_app_access_token`，S-1 后排）。
- 全局 logout、canonical zone 迁移、匿名历史归并、忘记密码恢复。
- Gmail 等外部账号绑定到 DID。

---

## 参考

- `doc/arch/verify-hub/混合SSO技术契约.md` — 混合 SSO 的 wire、状态机、撤销矩阵与持久状态。
- `doc/arch/BNS 去中心的名字系统.md` — 场景 6（联合登录）、场景 5（不依赖 CA 的强验证）、DID 解析与 IdP 发现。
- `product/users_and_agents/UserType.md` — 用户类型、登录/SSO 产品流程、sudo、用户数据隔离。
- `doc/control_panel/Control_Panel_Service.md` — control panel 认证契约。
- `doc/arch/07_identity_and_rbac.md` — verify-hub、session token、trust keys、RBAC 的实现说明。
</content>
</invoke>
