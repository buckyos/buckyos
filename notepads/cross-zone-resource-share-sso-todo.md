# 跨 Zone SSO 邀请、OAuth 2.0 登录与 DID 映射 · 设计评审与实施 TODO

2026-11-20 · 评审已定案，可以进入实施。裁决见 §4。PRD、技术契约、SDK 文档中与 §4 不一致的段落以本文为准，由第 7 步同步。A 是资源所在的 Zone，X 是安装在 A 的 App 实例，B 是收件人，C 是收到转发链接的另一人。使用原生 BuckyOS 身份的 B 有自己的归属 Zone。Alice 只有 Google 账号，`example.com` 是接入了 Google 的 Zone。[产品场景和 AC-25～39](<../doc/arch/verify-hub/BuckyOS 混合SSO PRD.md>) · [邀请与 OAuth 技术契约 §8.5、§9.3](../doc/arch/verify-hub/混合SSO技术契约.md) · [SDK 流程](../doc/sdk/SSO.md)

## 1. 设计动机

A 想请 B 编辑 X 中的一份文档。A 应能在 X 里选文档、选收件人、复制链接。B 本来就有 DID，应能看清用途，用自己的身份打开原文档。整个过程里，A 可以始终不允许 B 登录自己的系统。反过来，即使 A 允许 B 登录系统，私有文档也应继续按自己的 ACL 判权。

同一张开放邀请可接受入口允许的不同身份来源，定向邀请始终核对唯一的完整收件人 DID。原生用户通过自己的 BuckyOS Zone 证明 DID。仅有 Google 账号的受邀者可通过 A 已启用的外部来源访问指定 App 或 service。**身份来源回答“来的是谁”，A 的系统决定这次能签发什么会话，X 的资源服务决定这个人能对这份文档做什么。** 本地普通账号也可以绑定 Google 作为登录凭据，所得身份和系统登录资格仍属于那个本地账号。

**跨 Zone 稳定的 DID 都是可登录的 DID**：原生 DID，或某个 Zone 的本地账号 DID（如 `did:web:alice.example.com`）。外部 OAuth 来源合成的 DID 只在接入该来源的 Zone 内有效，不作为跨 Zone 身份使用。

## 2. 原有缺陷如何暴露

| 代码演进或现状 | 能解释的行为 | 对改动的要求 |
| --- | --- | --- |
| 10 月 22 日引入混合 SSO。归属证明成功后，资源 Zone 可向未映射本地账号的 DID 签发根域受限 External 会话。委派逻辑拒绝所有 External。 | B 能在 A 根域以自己的 DID 加入获准资源，跨主机交付身份却没有路径。 | 保留已有 DID 证明和后端 code 交换，在 A 明确本次邀请允许交付的目标。[历史委派判定](https://github.com/buckyos/buckyos/blob/0b9faa18a2784235423d378fd919e83ac2c9d964/src/kernel/verify_hub/src/main.rs#L1625) |
| 10 月 29 日 AI Workspace 迁到独立 `aiws-*` 主机。委派失败后前端直接打开目标 URL，根域的 host-only Cookie 不随请求送达。 | **B 已成功加入编辑组，到了工作区页面还是 Guest。** 这是此次 bug 的直接原因。e372a5257 中的窄修复为受限 External 提供工作区交接，并让工作区识别该身份。 | 保留这条回归并独立验收。普通 App 邀请需要自己的实例约束。[历史子域交接](https://github.com/buckyos/buckyos/blob/127a1eaad0ad1f7bac3785ab69402dd459b11355/src/frame/desktop/src/app/aiworkspace/launch.ts#L19) · [工作区回归用例](../src/frame/desktop/tests/dv/hybrid-sso-federation.spec.ts#L124) |
| AI Workspace 由资源服务检查 `manage`、写分享入口，前端用 ID 拼 `/sso/entry`。Message Hub 也会按群 DID 拼 SSO 链接。Control Panel（CP）在读取入口时才叠加 Zone 登录方式开关。 | 系统能关闭 DID 登录，却没有一个统一的创建入口来决定「X 可以定向邀请，但不能开放邀请」。 | 让会触发 SSO 的入口统一申请系统 URL，保留工作区组 ACL、群准入和匿名访问规则。[工作区分享](../src/frame/desktop/src/app/aiworkspace/ui/shell/ShareEntries.tsx#L102) · [群入口](../src/frame/desktop/src/app/messagehub/guest/model.ts#L22) · [CP 入口描述](../src/frame/control_panel/src/sso_entry.rs#L113) |
| 客服 App 已可用自己的服务身份调 `user.app_login.issue`。CP 查实例开关、登记本地客户邀请并返回 URL，客户兑换时才调用 verify-hub。`user.invite.create` 则是面向入驻的管理员接口。 | 旧实现已有「App 申请、系统返回链接」的先例，但客服的 `enabled` 只管本地客户代登录，未定义通用 DID 邀请策略。 | 复用对调用身份、安装实例和系统 URL 的检查。客服使用 Zone 内 Limited 身份，单独验证其业务连续性。[客服签发](../src/frame/control_panel/src/app_login.rs#L588) · [现有实例开关](../src/kernel/buckyos-api/src/hybrid_sso.rs#L864) · [入驻邀请](../src/frame/control_panel/src/user_mgr.rs#L1401) |
| Users & Agents 的 Add User 只能创建本地账户。PRD §9.2 的「邀请用户加入」只有后端 `user.invite.create/accept`，没有界面。只有 `user.create` 建出的账号以本 Zone 为归属、能向其他 Zone 联合，自助注册、专属邀请、客服建的账号都不能（D-2）。 | 想让只有 Google 的用户成为可登录、可跨 Zone 使用的本地用户，目前没有入口。 | 第 4 步补全「邀请添加用户」，经管理员邀请建出的账号与 `user.create` 同等。[Add User 向导](../src/frame/desktop/src/app/users-agents/components/shared/NewUserWizard.tsx#L159) · [账号记录](../src/frame/control_panel/src/user_mgr.rs#L674) · [联合出站检查](../src/frame/control_panel/src/federation.rs#L273) |

## 3. 新契约与用户流程

**系统是 SSO 邀请链接的签发方，App 可以是申请方。** X 用自己的实例身份向 A 的系统服务提交用途、明确的受众方式、资源引用、允许操作及返回位置。系统校验实例策略及分享者的资源授权，通过内部接口让 verify-hub 登记可核验的邀请，再由系统返回正确的 URL。URL 的域名与路径由已注册目标和路由确定。只拼出相似的 URL 不会获得可兑现的邀请。这里描述服务职责，具体 RPC 名称和记录存储位置在协议实现时确定。

| 用途 | 谁能向系统申请 | 谁可以接受 | 系统最终交付什么 |
| --- | --- | --- | --- |
| 访问 X 的指定资源 | X 代表对该资源有分享权的主体申请，始终绑定 X 的安装实例。系统按实例区分「禁止邀请」「仅指定 DID」「允许指定 DID 或开放邀请」。 | 定向时须匹配记录里的 DID。分享者可以按邮箱指定 Google 用户，由系统算出 DID（§3.1）。开放时，持链接且满足入口条件的人可各自用真实 DID 接受。 | 限于 X 的会话，业务操作继续查 ACL。AI Workspace 是 `System(aiworkspace)`，其多个工作区共用服务目标，服务仍须检查工作区和资源。 |
| 登录 A 的系统 | 有授予 Zone 登录资格权限的 service 通过系统入口申请，页面明确写出「允许登录 A」。 | 本轮按指定 DID 设计。若希望 Google 成为本地系统登录方式，需先有普通账号并显式绑定该凭据。 | 依据系统登录资格签发根域会话。进入其他 App 或具体文档仍查各自权限。 |
| 客服本地身份 | A 的客服业务可以请求系统为客户创建或复用本地 Limited 身份及临时凭据。 | 客户证明持有 A 提供的本地凭据，无需外部归属 Zone。 | 保留同一客户和指定工单。今天 `kind=code` 交付本 App，`kind=link` 可先登录根域再去 Message Hub，后者需迁移。[现有工单入口](../src/apps/sys_test/support_group.ts#L178) |

**B 打开一份 App 邀请的完整流程**

1. A 在 X 中邀请 B 编辑文档 D。获准后系统先登记邀请再返回 URL，记录区分实际申请实例、代表的分享者、授予目的、资源和受众。定向收件人 DID 必须能在 A 兑现，否则拒绝签发（§3.1 签发校验）。创建者只公开说明用途所需的信息，文档正文与成员名单继续受资源授权保护。开放邀请也要先有记录。`did:*:*` 只可作为“任意符合条件的 DID”的说明写法，不能当用户、解析目标或会话主体。
2. B 在用途页看清谁邀请他在哪个 App 做什么，再确认继续。定向链接从邀请记录取得目标 DID 和适用来源。开放链接允许 B 通过 `Login from` 选择来源，只有选择原生 BuckyOS 身份时才需提供自己的 DID。B 若已在 A 有适用的匹配会话，可直接继续。身份冲突时允许切换账号或取消，预览和取消保留邀请。
3. 需要登录时，A 先保存本次尝试的身份来源、邀请和资源返回位置。原生用户走 `Sign by DID`，浏览器前往其归属 Zone。B 第一次登录 A 上的这个 App 时，归属端的跨 Zone 登录确认页显示经核验的目标 App 主机及风险提示。同意按 (A, App) 记住，见 §4。随后回 A，由后端兑换联合授权码。选择 Google 等 OAuth 来源的用户则从 A 去提供方，授权码直接回 A 的 OAuth 回调，由 A 验证账号并得到 DID，详见 §3.1。
4. A 按记录复查受众、实例和授权，向资源服务提交受约束的身份，接受成功后关联真实 DID。目标主机通过一次性回调建立自己的会话，回到文档 D 的原路径、查询参数和片段。只有 X 访问资格的 B 只能进入 X。A 已为本次验证的账号开通系统登录时，该账号仍可按已有资格进入系统。开放邀请可以继续供 C 使用，是否限人数、何时过期按入口设置。

**分清记录的寿命。** 邀请记录决定谁还能新加入。接受时按实例当前的邀请等级、`app_doc` 声明的上限和分享者当前授权复查，任何一项不满足都不能兑现，记录保留并显示原因。接受记录把实际 DID、邀请来源和已获实例或资源资格关联起来。一次性登录尝试只负责本次往返，会话只在指定目标生效。管理员关闭新邀请、关闭一张入口、移除 B 的既有授权，是三个操作。B 以后直接重开深链接时，可根据仍有效的资格恢复访问。单张邀请关闭不应抹掉通过其他来源获得的独立权限。

### 3.1 外部 OAuth 身份：DID 规则与三个场景（拟实现）

**来源配置。** Google 是内置预设，身份域 `google.com` 和发行方 `https://accounts.google.com` 固定，管理员只填 client ID、凭据引用和本区的 HTTPS 回调。自定义来源的身份域由管理员填写，须满足三条：是发行方（纯 OAuth 时为账号接口）主机所在的域或其上级域；不是本 Zone 的主机或其下级域，否则会与本地账号 DID 冲突；同一身份域在本区只绑定一个来源。

**验证账号。** 用户在 `Login from` 里选择来源。CP 保存这次操作的用途、目标和邀请，回调时核对一次性 `state` 和来源，用 PKCE 兑换 code，再验证 ID Token 的签名、发行方、客户端受众、有效期和 `nonce`。Google 的 `iss` 可能是 `https://accounts.google.com` 或 `accounts.google.com`，两者都接受，之后一律记成来源配置里的规范 issuer。稳定主体是 `(规范 issuer, sub)`。纯 OAuth 2.0 来源用 access token 调用预配置的受保护账号接口，稳定主体是（来源命名空间，稳定账号 ID）。邀请按邮箱定向，所以来源必须给出已验证的邮箱：OIDC 取 `email` 且要求 `email_verified=true`。纯 OAuth 来源须配置邮箱字段和已验证标记字段，缺任一项不能启用。[Google 登录指南](https://developers.google.com/identity/openid-connect/openid-connect) · [OAuth 2.0 授权码流程](https://www.rfc-editor.org/rfc/rfc6749#section-4.1)

**`$accountid` 由已验证邮箱算出。** 外部 DID 是 `did:web:$accountid.$webhost`，规则写在代码里，不提供配置项：

1. 规范化：去掉首尾空白，整体转小写。Google 预设下，`googlemail.com` 视为 `gmail.com`，并删去用户名里的 `.` 以及 `+` 之后的部分。其他域名不改写用户名。
2. 标签：Google 预设下，规范化后域名是 `gmail.com` 的，标签就是用户名（只含 `[a-z0-9]`）。其他地址（含 Google Workspace 的企业邮箱）的标签是 `e-` 加上规范化地址 SHA-256 的 RFC 4648 base32（小写、无填充）的前 32 个字符。Gmail 用户名不含 `-`，两类标签不会相撞。自定义来源一律用哈希形式。
3. 同一邮箱在每个 Zone 算出同一个字符串，但它仍只在接入来源的 Zone 内有效（§1）。DID 本身会暴露 Gmail 用户名，这是按邮箱定向的代价。合成文档不额外披露邮箱或其他上游字段。

| 分享者输入或上游返回的邮箱 | 规范化 | 外部 DID |
| --- | --- | --- |
| `Alice@gmail.com` | `alice@gmail.com` | `did:web:alice.google.com` |
| `a.l.i.c.e+work@googlemail.com` | `alice@gmail.com` | `did:web:alice.google.com` |
| `alice@company.com`、`Alice@Company.com` | `alice@company.com` | `did:web:e-elezio5g7xybatgcdn5urmolneorulja.google.com` |
| `bob.smith@company.com` | `bob.smith@company.com` | `did:web:e-lfyluyqyaq2ucyjbqnol6xwg6rc4s3vf.google.com` |

**外部主体记录固定 DID。** 外部主体第一次接受邀请，或经第 4 步的流程绑定到本地账号时，写入外部主体记录 `(规范 issuer, sub) → 标签、DID、当时的邮箱`，同时写标签索引 `(webhost, 标签) → 主体`。之后有三条规则：

- 同一主体改了邮箱，仍使用原 DID，发往新邮箱的邀请按收件人不符处理。
- 标签已属于另一个主体时拒绝，不把它转给新主体。例如 Workspace 管理员删号后重建了同名邮箱。
- 只验证、没有接受邀请也没有绑定的登录，不写记录。

并发的首次接受用 KV 的 Create 语义，只有一个成功。记录建议放在 `services/control_panel/` 下，沿用专属邀请记录的写法。

**签发校验。** 定向收件人 DID 必须属于下面三种之一：本区本地账号 DID；原生 DID，能解析到 OwnerDocument 并找到归属 Zone；落在本区已启用 OAuth 来源的身份域下且标签格式合法。不满足时拒绝签发，提示“本区未启用该登录来源，请改邀对方的 BuckyOS 账号 DID”。落在内置预设身份域（如 `google.com`）下的 DID，即使来源未启用也不做 did:web 网络解析。邀请签发后来源被停用的，邀请保留，用途页提示来源已停用。

**按邮箱选收件人，绑定后用稳定 DID。** 分享者在 X 的收件人选择器里输入邮箱，系统按上面的规则算出外部 DID，再查外部主体记录、标签索引和凭据绑定。该主体已绑定本地账号时，收件人直接写成本地账号 DID（如 `did:web:alice.example.com`），选择器显示本地账号。没有绑定时使用外部 DID。

| 场景 | 完整流程及可观测结果 |
| --- | --- |
| 1. 在 `example.com` 接受 App/service 邀请 | 分享者输入 `alice@gmail.com`，邀请核对 `did:web:alice.google.com`，用途页预选 Google。开放邀请在成功接受后记录实际身份。Google 回调后 `example.com` 由已验证邮箱算出同一 DID，在本地合成文档并写入外部主体记录。verify-hub 签发只对获准 App 实例或 service 有效的 External token，业务服务再查资源 ACL。受邀者没有本地普通账号，不能用这张 token 登录系统。 |
| 2. 在 `example.com` 用 Google 登录本地普通账号 | 管理员在 Add User 里按 Google 邮箱邀请（第 4 步），或给已有账号发起绑定。被邀者用邮箱匹配的 Google 账号登录后，账号 `alice`（`did:web:alice.example.com`）生效并绑定这个 Google 主体。账号视同管理员添加，归属 Zone 是 `example.com`，可以向其他 Zone 联合。以后在系统入口选择 Google，verify-hub 按本地 Account 及其现有资格签发会话。账号也可设密码，用 `alice + 密码` 登录，身份和权限相同。未绑定本地账号时，OAuth 成功也不能进入系统。 |
| 3. 未接入 Google 的 Zone A 邀请 Alice | A 不能为 `alice@gmail.com` 或 `did:web:alice.google.com` 签发定向邀请，签发校验会拒绝并提示改邀账号 DID。A 改邀 `did:web:alice.example.com`，浏览器前往 Alice 的归属 `example.com`。Alice 在那里复用本地会话，或用绑定的 Google、密码登录为 `alice`。`example.com` 向 A 证明本地账号 DID，A 只兑换这个原生联合证明，比对邀请后签发自己 App 所需的会话。前提是 `alice` 是以 `example.com` 为归属的 Admin 或 User 账号，即 `user.create` 或第 4 步邀请添加的账号。自助注册的账号会被 `not_canonical_zone` 拒绝。 |

**绑定之前已有的邀请和授权。** 绑定前发给外部 DID、尚未接受的邀请，Alice 用 Google 验证，或用已绑定该主体的本地账号会话，都可以接受。接受记录和资源授权都记到本地账号 DID，会话签给本地账号。绑定前已经接受、授给外部 DID 的资源权限不迁移：Alice 在那张入口选择 Google 时，verify-hub 按接受记录签发外部 DID 的 App 会话。显示名或邮箱相同不能建立凭据绑定，凭据绑定必须经第 4 步的流程确认。

**解析与跨 Zone 边界。** 合成文档只由本区解析器按外部主体记录提供，不对其他 Zone 发布。外部 DID 不能作为 `auth.did.begin`、`/sso/federate` 或 CrossZoneAssertion 的主体，粘贴到 `Sign by DID` 时提示改用已启用的来源。其他 Zone 看到这类 DID（如群消息的发送者）只作显示，不能用本区同名的外部主体记录去解释它。[did:web 解析规则](https://w3c-ccg.github.io/did-method-web/#read-resolve) · [本区解析分类](../src/kernel/sys_config_service/src/zone_did_resolver.rs#L295)

### 3.2 从分享链接到原文档的 URL 验收

以下固定 A 为资源 Zone `a.example`，X 为 `appx.a.example`，最终资源为 `https://appx.a.example/docs/42?mode=edit#comments`。直接在 A 测试 Google App 邀请时，A 自己接入 Google。测试第三个场景时，只在 Alice 的归属 `example.com` 启用 Google。浏览器导航和服务端请求应分别断言。[SDK 逐跳示例](../doc/sdk/SSO.md#sso-url-examples)

| 链路 | 浏览器导航 | 后端断言 |
| --- | --- | --- |
| 分享入口（拟议） | X 从系统取得 `https://a.example/sso/share/inv-7m2`（定向）或 `https://a.example/sso/share/inv-8p3`（开放）。两者先到用途页。 | 能按邀请 ID 查到精确 App 实例和原始资源 URL。定向记录还包含目标 DID。 |
| 原生 DID（现有联合端点） | A 用途页 → 原生用户归属 Zone 的 `https://b.example/sso/federate` → `https://a.example/sso/did_callback` → 完成 App 交接 → 原文档。 | A 的 verify-hub 向 B 的公开入口 `POST https://b.example/kapi/verify-hub`，kRPC 方法为 `exchange_federation_code`。 |
| A 自行接入 Google，邀请外部账号（拟议） | A 用途页发起 `auth.oauth.begin` → `https://accounts.google.com/o/oauth2/v2/auth` → `https://a.example/sso/oauth/google/callback` → 完成 App 交接 → 原文档。 | A 后端向 `https://oauth2.googleapis.com/token` 兑换 code，在 A 核对由已验证邮箱算出的外部 DID。X 得到 A 的目标 token。全程只在 A 与 Google 之间认证。 |
| A 未接入 Google，改邀 `did:web:alice.example.com` | 新邀请用途页 → `https://example.com/sso/federate`。Alice 若选择 Google，浏览器前往 Google 并回到 `https://example.com/sso/oauth/google/callback`，恢复归属端的联合请求。随后携另一份 BuckyOS 授权码回 `https://a.example/sso/did_callback` → App 原文档。 | 只有 `example.com` 可兑换本次 Google code。A 的 verify-hub 向 `POST https://example.com/kapi/verify-hub` 兑换原生证明，核对 `did:web:alice.example.com`。使用归属端密码或已有登录态时省略 Google 往返。 |

实现时将 `/sso/share/<invite_id>` 接到桌面用途页，将 `/sso/oauth/<provider_id>/callback` 在网关和 CP 注册为 HTTP 回调。验证目标主机、路径、查询、`#comments` 和实际编辑操作，并核对 A 未接入 Google 时无法签发给 Google 身份的定向邀请。[网关 SSO 分流](../src/rootfs/etc/boot_gateway.yaml#L326) · [CP HTTP 入口](../src/frame/control_panel/src/main.rs#L1453)

## 4. 评审裁决

**App 的邀请权限上限**

> app首先要在自己的app_doc中申请这个权限，然后app安装的时候可以选择具体的等级（不会超过app申请的上限）。管理员可以随时调整

安装时先读 `app_doc` 申请的上限，再为具体实例选择「禁止」「仅定向」「定向及开放」中的一个等级。管理员可随时在这个上限内调整。没有申请邀请能力的 App 不能取得可兑现的链接。策略调整立即约束后续签发。

> （11-20 选定）接受时按当前等级拒绝

已发出、尚未接受的邀请在接受时按实例当前等级和声明上限复查，超出的不能兑现，记录保留并显示原因。不为此增加额外状态。

**归属 Zone 如何呈现外部 App 登录**

> 在B的verify-hub看来，就是在 appX.zoneA 上进行了一次登录。不过这个界面要与本zone登录有一些不同（多一个risk提示框）。

B 的外部登录确认页要显示正在登录的 App 主机，例如 `appx.a.example`，并显示跨 Zone 登录风险提示。显示的主机须与 A 的资源实例及本次登录请求可核验地绑定：A 按邀请记录签发带时效的 `app_context`，绑定本次请求、受邀实例及主机；B 核验签名，并检查主机在 A 公布的 Zone 主机之下。现有流程只传目标 Zone，开发时补齐。[SDK 中的 app_context 拟议](../doc/sdk/SSO.md#sso-url-examples)

> （11-20 选定）按 Zone+App 记

记住的同意从按 `client_zone` 改为按 (`client_zone`, App) 保存，App 取自核验过的 `app_context`。同一 Zone 上的新 App 第一次登录仍要确认并显示风险框，之后自动返回。AI Workspace 的所有工作区算同一个 App。本版是 breaking change，旧的同意记录不迁移。[现有同意记录](../src/frame/control_panel/src/federation.rs#L209) · [describe 时的判定](../src/frame/control_panel/src/federation.rs#L694)

**已有的系统登录资格**

> 按建议，如果系统本身对这个did给了更好的权限（比如已经配置了允许该did登录为普通用户），则按更好的权限生效

对同一已验证的本地账号或原生 DID，接受 App 邀请不降低其已有资格。Google 外部访客使用 App 邀请时，签发的是该访客的受限会话。要把 Google 作为系统登录方式，需要经第 4 步创建或选定普通账号并绑定外部主体，系统会话使用本地账号 DID。两种身份各自按 ACL 判权，封禁和停用继续生效。

**Breaking change 的范围**

> 没有旧会话，这个版本是breaking change

本版无需迁移旧会话、旧同意记录，也不为它们设计续期截止期。开发和验收从新签发的链接和当前会话开始，客服、匿名、原生 DID 等仍在产品范围内的入口按新版本分别回归。

**Google 的 client ID 与 sub**

> 我理解，在Google看来，本zone就是一个clinetID (管理员添加SSO端点的时候要添加的ID)，因此通过这个clientID ，同一个google account登录得到的sub是文档的?

每个 Zone 使用管理员为本区登记的 Google client ID，校验 ID Token 的 `aud` 包含该 ID。Google 公布的 subject 类型为 `public`，同一个 Google 账号即使面对不同 client ID，`sub` 也相同。本区按 `(规范 issuer, sub)` 识别外部主体、查找凭据绑定。其他 OIDC 来源若使用 `pairwise` subject，不同 sector 的 `sub` 可以不同，本区只按自己看到的 `sub` 记录。[Google Discovery 元数据](https://accounts.google.com/.well-known/openid-configuration) · [OIDC 主体标识类型](https://openid.net/specs/openid-connect-core-1_0.html#SubjectIDTypes)

**`$accountid` 的取值**

> 用邮箱算出来吧，这比较符合定向邀请的心智模型

按 §3.1 的规范化和标签规则实现，首次接受或绑定时把 DID 固定到上游主体。

**跨 Zone 稳定的 DID**

> 从设计上，跨zone的稳定的did都是可登录的did,因此都会是 alice.example.com这种。

外部 DID 不承担跨 Zone 身份，不做跨 Zone 解析、联合或断言。需要在其他 Zone 使用 Alice 时，邀请她的本地账号 DID。

**邀请添加的账号能否向其他 Zone 联合**

> 这相当于管理员在添加用户的时候就授权了，我记得现在 users&agents的需求里，把通过邀请添加用户标记为了待完成，这个TODO刚好就是补全这个功能。

管理员经 Add User 邀请添加的账号，与 `user.create` 建的账号同等，以本 Zone 为归属，可以向其他 Zone 联合。自助注册账号维持 D-2，专属邀请和客服的 Limited 账号不变。

**未接入来源时的定向邀请**

> 同意（A 未接入 Google 时，给 Google 身份的定向邀请在签发时拒绝）

按 §3.1 的签发校验实现。

**绑定后的 DID 选择**

> 绑定后优先用稳定did
>
> （11-20 选定）新邀请 + 未接受的邀请

新邀请按邮箱选人时直接用本地账号 DID。绑定前发出、尚未接受的外部 DID 邀请接受后记到本地账号 DID。已经授给外部 DID 的资源权限不迁移，见 §3.1。

## 5. 实施顺序与完成条件

- [ ] **0. 保住工作区修复。** 复验 e372a5257 的 External 工作区交接和私有工作区深链接。记录根域与工作区主机的身份、授权结果。
  - 对应 T1、T6 的旧路径基线。B 能在工作区主机写入，A 能读回，其他资源仍按 ACL 拒绝。该修复可独立交付。

- [ ] **1. 系统签发与实例策略。** 定义申请、登记和返回 URL 的接口。在安装界面按 `app_doc` 声明展示可授予的邀请等级，将安装时选定的等级及管理员后续调整保存到实例。签发时同时检查声明上限和实例当前等级。鉴权从真实服务身份推导安装实例，另查操作者分享权。定向必须提供通过 §3.1 签发校验的 DID，开放须显式选择。记录含系统签发方、申请来源、目标、受众、资源引用、期限和状态。写入成功才返回 URL，重复申请按约定幂等。接受时复查当前等级、声明上限和分享者授权。
  - T2、T10。直接调用接口与 UI 得到相同判定，拒绝后没有可兑现记录。登记失败可安全重试。

- [ ] **2. 工作区接入系统签发。** 把 AI Workspace 分享按钮接到系统申请入口。保留 `group_id` 及资源端 `manage`、加入和 ACL 检查。以相同契约接入 Message Hub 中触发 SSO 的入口。纯业务群邀请和匿名 Guest 继续用各自的准入规则。签发或接受分享时均按当前授权校验。旧客户端手拼 URL 或直接调用资源接口也不能自行取得资格。
  - T1～T6、T8、T10。开放受众在成功接受后才关联实际 DID。拒绝、取消、并发和中断均不误授、不重复占名额。

- [ ] **3. 区分两种会话。** 原生链路在 B 的外部登录页展示经过核验的目标 App 主机及风险提示：A 签发 `app_context`，B 核验并按 (`client_zone`, App) 记住同意。CP 恢复 A 根域 pending，复用既有 code 交换。verify-hub 核对邀请或已接受资格及准确目标。只有 App 资格时，以限于该次交付的证明向目标服务建立可撤销的会话，不依赖一个可登录系统的 CP 父会话。已有适用系统会话的委派则继续继承其权限上限和期限。经网关核对主机/安装者，在目标主机写 host-only Cookie。[同意记录](../src/frame/control_panel/src/federation.rs#L209)
  - T3～T7、T9、T12。从 `sys.<A>/login` 和根域两条起点都能恢复原资源。身份证明失败、入口失效、资源无权或暂时交付失败分别给出对应处理，重试时恢复原分享。创建链接时或浏览器离开前保存 `#fragment`，HTTP 请求本身不携带片段。

- [ ] **4. 系统登录、用户管理与邀请添加用户。** 补全 Users & Agents 的「邀请添加用户」，Add User 增加邀请路径（向导现在只调 `user.create`）：
  - (a) 原生 DID 用户：给现有 `user.invite.create/accept` 加上界面，流程按 Users & Agents PRD §9.2。
  - (b) Google 邮箱：管理员指定用户名、用户类型、用户组和邮箱，系统登记待生效账号和邀请。被邀者用已验证邮箱规范化后相同的 Google 账号登录，账号随即生效并绑定该主体，同时写入 §3.1 的外部主体记录和标签索引。
  - (c) 「不可登录用户」详情里的「允许登录本 Zone」：原生 DID 走 (a)，OAuth 访客预填其固定的邮箱走 (b)。
  - 经 (a)(b) 建出的账号与 `user.create` 同等，`new_account_actions` 的 `canonical_here` 为 true。自助注册、专属邀请、客服建号保持现状。
  - 凭据绑定只能经 (b)(c) 确认，一个外部主体只能绑定一个账号，重复或并发请求不能绑到两个账号。
  - 有权限的 service 能申请系统链接，未在 A 登录的 B 能先证明 DID 再接受，避免循环依赖。管理员可按 DID 汇总邀请、查看具体来源、开通登录并审计，不向普通用户开放完整邀请名单。原生账号映射按受信 OwnerDocument 取 DID，账号 `sub` 可能是本地用户名。用户管理展示本地账号 DID 与其登录来源的关联，外部 App 身份保留自己的邀请记录和资格。接好 `user.invite.accept` 所需的身份证明，保留系统已有的更高资格及封禁约束。
  - [Add User 向导](../src/frame/desktop/src/app/users-agents/components/shared/NewUserWizard.tsx#L159) · [user.create](../src/frame/control_panel/src/user_mgr.rs#L1025) · [账号记录与 canonical_here](../src/frame/control_panel/src/user_mgr.rs#L674) · [现有入驻路径](../src/frame/control_panel/src/user_mgr.rs#L1556) · [用户管理设计](../product/users_and_agents/Users_Agents_PRD.md)
  - T4、T8～T12、T16、T17。在 B 首访之前和之后均能开通。新登录使用新资格，本次 App 会话以签发目标为限。

- [ ] **5. 普通 App 与客服。** 接一个真实普通 App 和第二个安装实例。通过已注册实例定位应用，CP 调用它时只使用面向该实例的受限凭据。App 能识别 External，在每次读写时检查资源 ACL。另行迁移和回归客服的本地身份、同一工单、设密码和凭据到期流程。
  - T6～T9、T13。验证同 App 不同安装者、同实例不同资源、权限撤销及实例停用。

- [ ] **6. 接入 OAuth 来源与登录选择。**
  - 来源配置：扩展 CP 管理端和公开登录配置。Google 为内置预设，自定义来源校验 §3.1 的身份域三条约束。向前端返回适用于入口的来源 ID 和显示名。[SSO 配置入口](../src/frame/control_panel/src/sso_settings.rs#L95)
  - 登录选择：系统登录页、分享页、系统邀请页接入同一个 `Login from` 选择逻辑。定向邀请由受信的邀请记录和来源配置预选来源，来源在签发后停用时保留类型并提示。[系统登录页](../src/frame/desktop/src/auth/LoginPage.tsx#L344) · [分享登录页](../src/frame/desktop/src/sso/EntryPage.tsx#L387) · [系统邀请页](../src/frame/desktop/src/sso/UserInvitePage.tsx#L122)
  - 发起与回调：实现 OAuth 发起与回调，持久保存本次登录用途、目标和邀请。`iss` 规范化，要求已验证邮箱。
  - 映射与记录：按 §3.1 的规则从邮箱算出标签，带上测试向量。维护外部主体记录和标签索引，只在接受邀请或绑定时写入。本区解析器按记录合成文档。
  - 主体选择：外部邀请交付精确的 App/service token，本地账号登录查账号资格并返回本地身份。主体已绑定本地账号时，按 §3.1 选用稳定 DID：收件人选择器、未接受邀请的接受。
  - 认证方式：`AuthMethod` 增加 OAuth 方式并携带来源 ID，供下游核对。[AuthMethod](../src/kernel/buckyos-api/src/hybrid_sso.rs#L18)
  - 拒绝外部 DID：`auth.did.begin` 和 CrossZoneAssertion 拒绝外部 DID 作为主体。[auth.did.begin](../src/frame/control_panel/src/federation.rs#L422) · [跨 Zone 断言登录](../src/kernel/verify_hub/src/main.rs#L2332)
  - 停用：拒绝新验证和在途兑换，保留既有记录与绑定。
  - T14～T17。从同一 X 的授权入口实跑原生 DID、OIDC、普通 OAuth 三条路径。入口按显式认证来源判断。当前 `EntryMethods::allows_session` 会把其他方式归到密码/注册，工作区会把 External 归成 DID 联合，接入时要同时修正这两个分支。[入口规则](../src/kernel/buckyos-api/src/hybrid_sso.rs#L518) · [工作区加入](../src/frame/aiworkspace/server/src/lib.rs#L892)

- [ ] **7. SDK 与文档收口。** 同步 Rust/TypeScript 类型、SDK、CP 服务说明、产品文档和权限设置界面。用同一 App 示例演示替换身份来源，业务代码始终读取系统提供的 DID 和 App 会话。记录普通 App、工作区、系统登录、客服及 OAuth 来源的实测能力。[CP 接口说明](../doc/control_panel/Control_Panel_Service.md) · [用户类型](../product/users_and_agents/UserType.md)
  - 按 §4 改写以下段落：PRD §4.8、AC-36～39 中 `$accountid` 的含义和“已有授权仍按原主体判定”的说法；技术契约 §9.3 的“来源主体与本地合成文档”“恢复原请求”，删去“文档不披露邮箱”与按邮箱定向矛盾的部分；SDK SSO.md 中 Google 一节的 `$accountid` 说明；UserType 的 Google 段落；Users & Agents PRD §4.2「管理获邀 DID 的系统登录资格」和 §9.1、§9.2 的邀请路径状态。
  - 按 AGENTS.md 的 cmd_skills 联动表检查 `buckyos_admin`：身份、用户管理、登录来源变化后，手册里的设置与诊断命令仍然可用。

## 6. 测试矩阵

| 用例 | 要验证的结果 |
| --- | --- |
| T1 定向首访（AC-25） | B 在 A 无账号。A 从真实 App 申请给 B 编辑 D 的邀请。B 无需填 DID，已登录归属 Zone 不重复输密码。回到 X 后核对实际 DID，B 能编辑，A 能读回。 |
| T2 开放邀请（AC-26） | X 获准开放后，B、C 可分别输入自己的 DID 并接受同一条仍有效的邀请。两条接受记录指向同一邀请，保留各自的业务主体和授权。 |
| T3 复用（AC-27） | 分别预置 A 有匹配会话、仅 B 有会话并已同意 A 的 X、B 首次披露。在首次跨区确认页核对风险提示和真实 App 主机，伪造或改动 `app_context` 不能改变所展示并最终授权的主机。已同意 X 后登录 A 上的另一个 App 仍要确认并显示风险框，同一工作区服务下的不同工作区不再确认。记录在哪个 Zone 确认几次。撤回同意、资料范围或归属 Zone 改变时重新确认。 |
| T4 错人与取消（AC-28） | C 打开给 B 的链接、B 的归属 Zone 登录成 C、B 主动取消、A 已有另一个系统身份。错误 DID 均不能加入，不消耗 B 的机会，不静默切换身份。另测已映射本地账号的 B 可用同一 DID 接受。 |
| T5 生命周期与失败（AC-28/29） | 分别测试到期、关闭新加入、撤销已接受授权、资源删除、code/state 重用、双标签并发和交付中断。一个 DID 至多新增一次相同资格。已加入后交付失败可重试，其他来源的授权不受影响。 |
| T6 深链接（AC-29） | 最终实际 URL 的主机、实例、path、query、fragment 及页面资源标识都匹配。路由换绑、外部目的地址或实例停用时拒绝交付。直接重开获准的深链接仍能使用有效资格。 |
| T7 权限与续期 | 只读者不能写，可编辑者不能读相邻私有资源。普通 App 的会话不能进入同 App 另一安装实例。分别验证实例停用、资源授权撤销、相关会话退出或到期后的下次操作及续期。 |
| T8 新版本各身份入口 | 在全新会话验证多登录方式、匿名资源、入驻邀请和客服本地凭据。重开本版签发但已经关闭的入口时，应按该邀请的当前状态处理。 |
| T9 两项资格四种组合（AC-30/31） | 在没有额外权限的前提下，分别准备只有 X、只有系统、两者都有、两者都无的 DID。对每个 DID 实测打开系统入口和访问 X 中的私有 D。仅有 X 时可完成 D 的操作，且无通用根域系统会话。对已在 A 获得普通用户资格的原生身份，另验接受邀请后仍可按原有资格登录系统，X 的 ACL 仍有效。OAuth 外部访客及其可能绑定的本地账号按 T15、T16 分别验收。 |
| T10 申请方与登记（AC-30/32） | 对未声明邀请权限及分别声明定向、开放上限的 `app_doc` 安装实例，验证安装页的可选等级。安装时降级、管理员运行时调高或调低后分别请求定向、开放邀请。超出声明上限的操作应被拒绝。降级前已发出、尚未接受的超级别邀请在接受时被拒，记录保留并显示原因。另测 X 请求其他实例或系统登录、操作者无分享权、无权 service、绕过 UI 直调旧入口、登记部分失败或重试。只有通过全部检查时能得到有效 URL。verify-hub 与管理员看到同一记录。 |
| T11 用户管理（AC-32/33） | 两个 App 邀请 B。B 首访前即可在「不可登录用户」里看到他和两个来源。开放邀请尚未兑换时没有虚构用户。管理员开通后 B 在新会话中能使用获准的系统功能，仍不能打开未授权资源。分别经 Add User 邀请原生 DID 用户和 Google 邮箱用户：前者按 PRD §9.2 加入，后者在邮箱匹配的 Google 账号登录后生效并完成绑定，待生效时不能系统登录。重复开通不重复建号，也不自动迁移外部身份的资源权限。停用账号不被意外恢复。 |
| T12 首次系统登录（AC-31） | 获准 service 申请的系统邀请可从 A 无会话的浏览器开始。原生用户按系统邀请受众验身份，选择 OAuth 的用户须以明确绑定的本地普通账号取得会话。另测转发、过期、拒绝及撤销，同一失败不妨碍 B 使用仍有效的邀请。 |
| T13 客服本地身份（AC-34） | 无外部 DID 的客户使用临时链接进入指定工单、留言并再次访问。能区分本地 Limited、App `kind=code` 与外部 DID。全程无归属 Zone code 交换。新 App 客户链接不能用于他人的工单或通用系统登录。 |
| T14 `Login from` 与来源验证（AC-35/38） | 配置 Google 兼容 OIDC 及提供稳定账号资料和已验证邮箱的纯 OAuth 来源。验证来源选择、精确回调、取消、错来源/受众、两种写法的 Google `iss`、失效签名、过期、nonce/state/PKCE 错误、重放、中途停用和未绑定账号的系统登录。`email_verified=false` 或缺邮箱字段时拒绝。自定义来源的身份域为本 Zone 主机、与发行方无关或与已有来源重复时不能保存。失败时不发目标 token、不消耗邀请。普通用户不能改端点。 |
| T15 外部 App/service 身份（AC-36/37） | 对 §3.1 的测试向量逐条断言规范化与 DID，包括大小写、`.`、`+`、`googlemail.com` 和 Workspace 邮箱。在 `example.com` 分别以邮箱定向、开放邀请接受，合成文档只在本区可解析。只登录未接受时不写外部主体记录。同一主体改邮箱后仍得到原 DID，发往新邮箱的邀请按收件人不符处理。同名邮箱换了 `sub` 时拒绝，原记录不变。并发首次接受只写一条。token 主体为外部 DID，限目标 App 实例或 service，能完成获准操作，系统、其他实例和越权资源均拒绝。 |
| T16 本地普通账号绑定 Google（AC-37/38） | 经 Add User 邀请或对已有账号发起绑定，让 `alice` 绑定已验证的 Google 主体。分别用 Google、后设的 `alice + 密码` 登录，核对同一本地 Account、`did:web:alice.example.com`、本地权限及审计主体。绑定后在收件人选择器输入该邮箱，得到本地账号 DID。绑定前发给外部 DID、未接受的邀请，用 Google 或已登录的本地会话接受，接受记录与授权都记到本地账号 DID。绑定前已接受的外部授权仍需在原入口选 Google，以外部 DID 进入。另测系统登录无绑定时拒绝、同名或同邮箱不自动绑定、重复或冲突绑定不能换人。 |
| T17 未接入 Google 的 Zone A（AC-39） | 在没有 Google 端点的 A 上，为 `alice@gmail.com` 或 `did:web:alice.google.com` 申请定向邀请时被拒，提示改邀账号 DID，A 不对 `alice.google.com` 发网络请求。改邀 `did:web:alice.example.com`，浏览器跨到 `example.com`，分别用 Google 和密码验证本地 Alice 后回 A。断言 A 收到本地账号 DID 的原生证明，X 能完成获准操作，A 和 X 均未接收 Google token。`alice` 分别用 `user.create` 和第 4 步的邀请添加准备，都能通过。自助注册的账号被 `not_canonical_zone` 拒绝。把外部 DID 粘贴到 `Sign by DID`，或以它为主体发 CrossZoneAssertion，均被拒绝。 |

### 现有测试入口及交付证据

**Rust。** 在 `src/` 中运行以下测试。在对应包补新策略、记录和权限变更用例，核对过滤后实际执行的测试数。

```bash
cargo test -p buckyos-api --test hybrid_sso_fixtures
cargo test -p control_panel federation::tests
cargo test -p control_panel app_login::tests
cargo test -p verify_hub federation::tests
cargo test -p verify_hub delegate_external_workspace
cargo test -p aiworkspace-store --test sso --test acl
cargo test -p aiworkspace --test sso
```

**浏览器。** 在 `src/frame/desktop/` 中运行。第一条是 mock 交互回归，第二条使用两个允许 DID 登录、可相互解析并进行 HTTPS 交换的真实 Zone。按脚本默认值，Bob 是归属侧，Charlie 是资源侧。替换 IP，并准备 A 的资源管理账号、B 的归属账号、负例身份 C 及普通 App 实例主机。缺少任一 Zone 映射会跳过用例。

```bash
VITE_CP_USE_MOCK=true pnpm exec playwright test tests/e2e/pages/sso-entry.spec.ts tests/e2e/pages/sso-federate.spec.ts tests/e2e/pages/sso-login.spec.ts tests/e2e/pages/sso-invite.spec.ts tests/e2e/pages/sso-admin.spec.ts tests/e2e/pages/user-invite.spec.ts
BUCKYOS_TEST_HOST_RULES='MAP bob.web3.devtests.org <B_IP>, MAP charlie.me <A_IP>' pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/hybrid-sso-federation.spec.ts
```

**这条双 Zone 测试要改起点和终点。** 当前用例直接调 `ws.create_entry`、手拼 URL，最后还断言 A 根域存在 External 会话并能打开受限首页。保留子域身份、实际写入、owner 读回的断言。新链路从 X 正常申请并复制系统返回的 URL 开始。分别以「仅受邀 App」和「具有系统登录资格」验证根域结果，覆盖 T9。管理流程增加首访前可见和开通前后的真实浏览器断言。脚本里的 `zoneA` 是归属侧 Bob、`zoneB` 是资源侧 Charlie，与本文 A/B 相反，改写时换成 `homeZone` / `resourceZone`。[当前 DV](../src/frame/desktop/tests/dv/hybrid-sso-federation.spec.ts#L21)

系统入驻另跑 Node 24 的协议 DV，需三个开发 Zone、测试 SN、CA 和 Charlie 的 OwnerDocument。客服使用安装了 `sys-test` 的开发 Zone，运行现有浏览器与业务 DV。[协议 DV 环境及命令](../test/test_hybrid_sso/README.md) · [客服浏览器 DV](../src/frame/desktop/tests/dv/support-group-link.spec.ts) · [客服业务 DV](../test/test_sys_test/support_group_dv.ts)

OAuth 用例是待新增的测试。为接入来源的测试 Zone 准备可控的 OIDC 和纯 OAuth 服务（能返回已验证邮箱、可改邮箱、可让同一邮箱换 `sub`）、HTTPS 回调、一个只有外部身份的访客，以及一个经邀请添加并绑定 Google 的普通账号。T17 另设未启用 Google 的资源 Zone A。核对每次浏览器跳转、授权码的兑换方、目标 token 的身份、系统与资源操作结果。整组测试不应请求 Google 用户子域的 DID 文档。已有原生联合 Playwright DV 单独回归，有登记的 Google 测试客户端时可另跑真实登录。

完成时按各条身份与授权路径记录版本、实际命令、通过/失败/跳过数，以及脱敏的目标主机、主体、会话用途和业务操作结果。报告保留失败时的资格状态，不记录上游 token、client secret、code 或邀请 secret。
