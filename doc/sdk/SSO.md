# BuckyOS 中的 SSO（使用者指南）

本文从**接入方（App / Web 开发者）视角**讲清楚怎么在 BuckyOS 里完成登录，以及它和主流 SSO（OAuth / Google 登录 / 企业 IdP）"套路"上的关键区别。

> 这是一份使用文档，不是协议规范。
> - 当前 control panel 的 canonical 认证契约见 `doc/control_panel/Control_Panel_Service.md`。
> - 名字解析、跨 Zone 信任根、钱包登录的完整设计见 `doc/arch/BNS 去中心的名字系统.md`（场景 6）。
> - 用户类型、登录方式、sudo 的产品定义见 `product/users_and_agents/UserType.md`。
> - 标注为「协议目标 / 规划中」的部分尚未在本仓库完整落地，接入时以「当前实现」小节为准。

## 一句话套路

> 把每个用户自己的 **Zone（的 verify-hub）当成 IdP**，App 通过 **DID 解析**发现该 IdP，登录后拿到一个**绑定到自己 appid 的 session_token**，之后所有请求都只认这个 token + RBAC。

和主流 SSO 一样的部分：有登录页、有回调、有 access/refresh token、有 scope；接入方最终只关心"拿到一个可信 token"。
不一样的部分见下一节——这是本文的重点。

## 和主流 SSO 最不一样的地方

| 维度 | 主流 SSO（OAuth / Google / 企业 IdP） | BuckyOS SSO |
| --- | --- | --- |
| IdP 是谁 | 固定的中心化大平台 | **用户自己的 Zone**。每个 Zone 的 verify-hub 就是一个 IdP，没有中心 |
| 怎么找到 IdP | 写死的 endpoint / 配置 | **DID 解析**：`user_did -> owner -> default_zone -> ZoneConfig.verify_hub_info.public_key` |
| 信任根 | CA / 平台公钥 | Owner / Zone 的 DID 公钥（可不依赖 CA，见 BNS 文档场景 5） |
| token 绑定 | client_id | **appid**（= App 的 DID 或 gateway 注册的 app_id），token 是 audience-scoped 的 |
| 没有账号的用户 | 必须先在平台注册 | 可以**用浏览器钱包**对登录 challenge 签名，证明"我控制这个 DID"，无需任何 Zone |
| 跨身份提供方 | 换一个大平台 | **跨 Zone**：current-zone 证明"你是谁"，target-zone 重新签发自己的 token |

实务上要记住三句话：

1. **token 是 appid 绑定的**。页面 token 代表"当前操作者 + 这个 app"，不要跨 app 复用，也不要拿 app-service 自己的 token 冒充页面 token（见后文「不要混用 token」）。
2. **登录方式对鉴权方透明**。业务服务只验证 session_token + 跑 RBAC，不关心用户是用密码、钱包还是 Passkey 登录的。新增登录方式不影响下游。
3. **强权限要 sudo**。高危操作需要二次输入密码换一个短时 sudo token，且仍受 appid 限制。

---

## 最小接入：同 Zone 内的 Web App

这是最常见的场景：你的 App 跑在用户 Zone 的某个子域（如 `myapp.alice.buckyos.io`），要让用户登录。

### 1. 跳转到登录页

用 websdk 的 `AuthClient`：

```ts
import { AuthClient } from "buckyos";

const auth = new AuthClient(zoneHostname /* 如 "alice.buckyos.io" */, appId /* 你的 appid */);

// 跳到 Zone 的登录页，登录完成后回到 redirectUri（默认当前页）
auth.login(/* redirectUri? */);
```

`buildLoginURL()` 生成的目标就是 Zone 的 control panel 登录页：

```text
https://sys.<zoneHostname>/login?client_id=<appId>&redirect_url=<encoded redirect>
```

> `/login` 是 control panel 自身的登录页；当带上 `client_id` + `redirect_url` 时，它同时充当 SSO 授权页。（历史上也规划过独立的 `/sso/login` 授权弹窗页。）

### 2. 登录完成 → 回调写 Cookie

用户在登录页完成认证后，control panel 走 `/sso_callback?nonce=...&redirect_url=...`：

- 校验 `redirect_url` **必须是本 Zone 内的目标**（host 等于 zone host，或形如 `<app>.<zonehost>`），并据此解析出真正的 appid——防止把 token 发给 Zone 外的站点。
- 回调由目标 App origin 承载，同时写入两枚 host-only Cookie：短期 `buckyos_session_token` 供 gateway 在 App 页面加载前完成鉴权，长期 `buckyos_refresh_token` 使用 `HttpOnly`、只供刷新。随后 302 回 `redirect_url`。两枚 cookie 都不设置 `Domain`，因此不会与父域或其它 App 子域共享。

### 3. 用 refresh cookie 换 session_token

页面加载后，向 control panel POST `/sso_refresh`（自动带上 HttpOnly cookie）：

```text
POST /sso_refresh
-> 200 { "session_token": "...", "user_info": { user_id, show_name, user_type, ... } }
```

- 返回**短期 session_token**（放在响应体里，供 JS 读取并用于后续请求）。
- 同时轮换 `buckyos_refresh_token`（旧的立即失效），并把短期 token 写入独立的 host-only `buckyos_session_token`。
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

## Session Token 是什么

登录的目标是让 client 拿到一个可保存、可复用的 **Session Token**，它代表一个已完成认证的会话，至少包含：

```text
sub / userid   当前用户（username 或 DID）
appid / aud    绑定的 App（audience）
exp / jti      过期时间、nonce
iss            签发方（Zone 的 verify-hub）
```

> 一次登录通常返回**短期 session_token + 长期 refresh token**。session 过期用 refresh 流程换新；refresh token 一旦使用立即轮换，旧的失效。

对鉴权方而言，登录方式（密码 / 钱包 / Passkey）是**透明的**——只看 token 可信不可信、token 里声明的用户/App/权限上下文是什么。这就是为什么扩展登录方式不需要改下游业务。

---

## 跨 Zone SSO（去中心 IdP）

当用户想登录的目标 Zone 不是自己的 Zone 时：

- **Current Zone**：用户绑定/拥有的 Zone（他的 IdP）。
- **Target Zone**：他想进去操作的 Zone。

```text
1. 用户在 Target Zone 输入/选择一个 DID。
2. Target Zone resolve(user_did, owner) -> default_zone_did
                 resolve(default_zone_did, zone) -> verify_hub 信息 + 公钥
3. Target Zone 拉起 Current Zone 的 verify-hub 登录页。
4. 用户在自己 Zone 完成认证（密码 / 钱包 / Passkey）。
5. Current Zone verify-hub 签发一个"联合登录凭证"（只证明"你是谁"）。
6. Target Zone 验证该凭证，映射到一个明确 DID / 本地账号。
7. Target Zone 的 verify-hub 再签发**本 Zone 内可用的 session_token**。
```

要点：

- 用户最终拿的是 **Target Zone 自己签发的 token**，并继续受 Target Zone 的 RBAC / scope 约束。
- Current Zone 的凭证只负责"证明身份"，**不自动代表在 Target Zone 有任何权限**。
- 和 Google SSO 本质相同（信任一个外部 IdP 完成认证），区别只在 IdP 是"用户自己的 Zone"，且发现/信任根来自 DID + ZoneConfig + verify-hub 公钥。

> 跨 Zone 内部 DID 通常是明确的——系统凭证核心是 username / DID，每个可登录用户都能映射到一个确定 DID。未来若支持 Gmail 等外部账号，登录后也必须绑定到一个 DID（绑定到已有 DID，或在 Target Zone 建一个二级 DID）。

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
- **users / su_user**：最常见的登录用户，能用 app 但不能管系统。
- **limit user（规划中）**：受限登录用户（如不允许改密码、未成年）。
- **friends**：非系统登录用户，在某些 app 内有登录态，通常有 SNS 读 + 评论类写权限。
- **guest（匿名）**：默认**全部拒绝**；只有 zone 明确列入白名单的 public resource 才允许匿名访问。

让一个传统 app 提供公共服务的方式：把 app 设为 gateway 公共访问（= 授予特殊权限），或 app 自带用户管理、在自己的 app data 范围内管理读写。

---

## sudo：高权限二次确认

sudo 由 verify-hub 提供：弹出提权对话框，要求管理员**再次输入密码**，换发一个**短时（约 3 分钟）SUDO Session Token**，可能带明确操作边界。

接入要点：

- sudo token 仍受 **appid 限制**——非系统类 app 申请 sudo 意义不大，因为还是被 appid 框住。
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
- websdk `AuthClient.login()` -> `sys.<zone>/login?client_id=&redirect_url=`。
- control panel `/sso_callback`（写 HttpOnly refresh cookie，校验 redirect 在 Zone 内并解析 appid）、`/sso_refresh`（换 session_token + 轮换 refresh）、`/sso_logout`。
- session_token 多通道传递（X-Auth / Bearer / query / kRPC token）+ trust keys 验签 + RBAC。
- sudo 机制（提权对话框 + 短时 sudo token，受 appid 限制）。

**协议目标 / 规划中（接入时不要假设已可用）：**

- 跨 Zone 的标准 redirect / audience / scope / challenge / app-DID 校验 / 跨 Zone token 验证规则。
- 浏览器钱包扩展 detection 与 wallet-signed assertion 的标准格式。
- 全局 BNS resolver（当前 `ZoneDidResolver` 只是 Zone 内 resolver）、链上 owner / alias 状态。
- 开放注册、limit user、Gmail 等外部账号绑定到 DID。

---

## 参考

- `doc/arch/BNS 去中心的名字系统.md` — 场景 6（联合登录）、场景 5（不依赖 CA 的强验证）、DID 解析与 IdP 发现。
- `product/users_and_agents/UserType.md` — 用户类型、登录/SSO 产品流程、sudo、用户数据隔离。
- `doc/control_panel/Control_Panel_Service.md` — control panel 认证契约。
- `doc/arch/07_identity_and_rbac.md` — verify-hub、session token、trust keys、RBAC 的实现说明。
</content>
</invoke>
