# BuckyOS 混合 SSO 实施 TODO

日期：2026-10-21。状态：待实施；本文是代码核对后的实施计划，不是功能完成记录。

目标：在现有本 Zone SSO 上实现 EA、EB、DID 和允许时的 Guest 入口，同时贯通 Message Hub 外部客户会话、AI Workspace 分享、同 Zone 跨应用身份复用。先完成身份、授权与撤销底座，再开放新登录方式。

依据：

- [BuckyOS 混合 SSO PRD](<../doc/arch/verify-hub/BuckyOS 混合SSO PRD.md>)：2026-10-21 的产品要求、AC-01～24、Q-01～13。
- [Session Only Member 设计](<../doc/message_hub/Session Only Member 设计.md>)：保留身份源、最小授权、Session Guest、全入口鉴权和撤销约束；与新 PRD 冲突处按下文对齐。
- [SSO](../doc/sdk/SSO.md)、[Self-Host-Groupv2](../doc/message_hub/Self-Host-Groupv2.md) 及当前源码。历史 TODO 的勾选不作为本次实现证据。
- 跨 Zone 联合登录的已有设计：[SSO](../doc/sdk/SSO.md)「跨 Zone SSO（去中心 IdP）」、[BNS 去中心的名字系统](<../doc/arch/BNS 去中心的名字系统.md>) 场景 6。SSO-09 在其基础上补齐，见 §0 D-7。

源码核对基线：BuckyOS `959624aa7` 及 2026-10-21 当前工作区，同时阅读同级 `buckyos-websdk`。本次只新增本文；未运行构建、单测或真实 Zone DV，也不据此宣称运行环境已具备这些能力。

## 0. 决策冻结（2026-10-21 review 追加）

本节列出开工前必须拍板的 9 项技术决策。每项附推荐、备选和受影响的后文条目。状态为“待拍板”的项，code agent 不得自行选定，也不得在代码常量中隐含选择。拍板后把状态改为“已定”，并按“影响”列修订后文；修订完成前，后文与已定决策冲突时以本节为准。

### 0.1 决策依据：前文未覆盖的源码事实

- **Message Hub 与 AI Workspace 同源。** 二者都是 Desktop SPA 路由（[App.tsx](../src/frame/desktop/src/App.tsx) 的 `/messagehub`、`/workspace/:workspaceId`），通过 `/kapi/<service>` 共用根域的 `System{control-panel}` 会话。PRD 两个场景中的“同 Zone 跨应用”不需要跨 origin 换票；外部受限用户在根域取得的会话与管理员桌面会话 target 相同。`auth.prepare_app_launch` 只与第三方 App 子域相关。
- **存在统一鉴权闸口。** 约 11 个服务经 [runtime.rs](../src/kernel/buckyos-api/src/runtime.rs) 的 `verify_trusted_session_token` 校验 token；`enforce`（RBAC）只有 aicc、msg-center、nfs-server 使用，其余服务验签后走自定义判断。`limited` RBAC 组当前零策略。自行解析 token 的路径还有 system-config 的自有 `enforce`、control_panel 的 `dashboard`/`pikg`/`zone_mgr`、repo_service、homestation `sign`、Gateway `check_oauth`。
- **跨 Zone 断言目前无法验证。** 已有联合登录设计依赖 `ZoneConfig.verify_hub_info.public_key`，但公开 ZoneDocument 中该字段已被注释（[zone.rs](../../buckyos-base/src/name-lib/src/zone.rs) `// pub verify_hub_info`）；verify-hub 公钥只在 Zone 内部 `boot/config`。verify-hub 签发 token 的 `iss` 是常量 `"verify-hub"`。
- **会话 family 只在内存。** verify-hub 的 `REFRESH_TOKEN_CACHE` 是内存表，重启即丢失全部浏览器会话，与 PRD D-04 / G-03 的长期复用冲突。
- **系统没有短信 / 邮件发送能力。** 源码中没有 SMTP / SMS 实现；EA 过期后向原渠道重发（AC-05）没有落点。
- **本地账号已有二级 DID 与托管密钥。** `user.create` 的 `generated_owner_config` 生成 `did:bns:<user_id>.<zone>`、设置 default zone，私钥写入 system-config。[UserType.md](../product/users_and_agents/UserType.md) 已将此类 DID 定义为该 Zone 的子账号；“limit user”仍标为规划中。

### 0.2 决策总表

| 编号 | 决策 | 推荐（一句话） | 状态 |
| --- | --- | --- | --- |
| D-1 | 受限账号的用户类型 | EA、EB、DID 访客均用 `Limited`，加凭据状态与来源字段 | 待拍板 |
| D-2 | EA / EB 的 DID 与对外证明 | 沿用二级 DID；verify-hub 不为 `Limited` 签对外联合断言 | 待拍板 |
| D-3 | Guest 形态 | v1 保持资源级 `guest:` 主体，verify-hub 不签 Guest 系统会话 | 待拍板 |
| D-4 | 默认拒绝落点与根域受限会话 | `verify_trusted_session_token` 统一闸口 + 服务显式开放方法；不新增 target | 待拍板 |
| D-5 | 撤销强度 | 资源级撤销保持实时；账号 / 来源 / 登出靠短 access TTL + 续期复核，残留列已知风险 | 待拍板 |
| D-6 | 会话 family 持久化 | verify-hub 本地 SQLite，单实例限制写明 | 待拍板 |
| D-7 | DID 联合断言的验证 | 后端授权码交换：目标 verify-hub 经 HTTPS 向 canonical verify-hub 兑换 | 待拍板 |
| D-8 | 分享入口对象与策略层级 | 入口归资源服务持有；有效方式 = 入口允许 ∩ Zone 总开关 | 待拍板（含 Q-06） |
| D-9 | EA 重发渠道 | v1 系统不内建发送，回调签发方经原渠道发送 | 待拍板 |

### 0.3 各项细节

**D-1 受限账号的用户类型**

- 推荐：EA、EB 账号和 DID 访客的本地 Profile 都使用 `UserType::Limited`，其语义重定义为“可登录的受限外部账号”。`UserSettings` 增加凭据状态（未设密码 / 已设密码）与账号来源（EA / EB / DID）字段，字段名在 SSO-02 冻结。`limited` 组保持零策略，所需能力逐项显式授予。
- 同时修正两处现状：scheduler `map_api_user_type` 的 `_ => User` 回退会把 Guest 及未知类型放进 `users` 组，改为不映射或映射到零权限组；`allow_password_change` 对 `Limited` 默认 false，改为按凭据状态判断。
- 备选：新增独立 UserType 变体，需同步 scheduler 的 UserType、RBAC 生成、`app_availability` 和 WebSDK 类型。
- 影响：SSO-01 第 2、7 条；SSO-04 零权限测试；SSO-06、07、09 的建号。

**D-2 EA / EB 的 DID 与对外证明**

- 推荐：EA / EB 沿用现有本地账号的二级 DID `did:bns:<user_id>.<zone>` 与 Zone 托管密钥，不另造标识体系，也不向用户交付私钥。verify-hub 拒绝为 `Limited` 账号签发对外联合断言（D-7 的 code），因此目标 Zone 无法借这些账号向第三方 Zone 证明身份。DID 访客的本地记录以其外部 DID 为主体，不生成本地 OwnerDocument、密钥或密码。
- 备选：EA / EB 不生成 OwnerDocument，改用纯本地 ID（SSO-01 第 4 条现方向）。需改造 msg-center 等以 DID 作成员标识的消费者。
- 影响：SSO-01 第 1、4 条；SSO-09 第 6 条。

**D-3 Guest 形态**

- 推荐：v1 的 Guest 是资源级匿名主体，沿用 AI Workspace 的 `gt_` 浏览器令牌 → `guest:<id>` 主体，只在资源显式授予 `public` / guest 的范围内有效。verify-hub 不为 Guest 签发系统会话，其它系统服务不接触 Guest。“统一”体现在入口页统一展示 Guest 选项、SDK 统一返回当前身份类别。Message Hub v1 不提供 Guest，企业场景用 EA。
- 备选：verify-hub 签发 `UserType::Guest` 系统会话。匿名主体随之进入 D-4 闸口及全部开放服务的审计范围。
- 影响：SSO-08 第 1、3 条；SSO-13 第 2 条。

**D-4 默认拒绝落点与根域受限会话**

- 推荐：受限标记写入 verify-hub 签名的 claims。`verify_trusted_session_token` 识别后默认拒绝；服务在 runtime 初始化或方法分发处显式声明接受受限会话的方法。用户升级时旧会话保持受限，不会放大；降级为受限时，旧普通 token 在 access TTL 内残留，列为已知风险。
- v1 开放清单：verify-hub 的 refresh / logout / 首次设密码；CP 的入口编排与 SSO 方法；msg-center 群 RPC / 群 HTTP / 群对象读取；AI Workspace。§0.1 列出的自行解析 token 的路径，逐一确认结论并补测试。
- 受限用户在根域仍使用 `System{control-panel}` target，权限上限由闸口与开放方法清单表达。
- 备选：为外部入口新增独立 System target，需扩展 `SYSTEM_LOGIN_TARGETS`、Gateway 路由、CP callback 与 SDK。
- 影响：SSO-02 第 1、2 条；SSO-04 审计表收窄为“闸口 + 已开放服务深审”；SSO-11 第 4、5 条与 SSO-14“同 Zone 跨应用”行：MH↔AIWS 按同源验证，跨 origin 测试只针对第三方 App 子域。

**D-5 撤销强度**

- 推荐：资源级撤销（移出 Session、grant 撤销、资源删除）保持每请求查实时状态。msg-center 群 ACL 与 AI Workspace grants 已是如此，只需确认受限会话路径不缓存授权结论。
- 账号停用、来源撤销和登出：受限会话使用较短的 access TTL（值随 Q-03 定，建议分钟级）；refresh、delegate、callback 一律复核账号与来源状态，并修复宽限期分支在复核前返回缓存 pair 的问题。持续流与订阅在下一次续期或重连时复核。TTL 内的残留列为已知风险。
- 备选：SSO-05 原文要求的“撤销提交后下一次受保护请求即生效”。需要持久 revocation 版本、各服务缓存与失效通知，并与 system-config / msg-center 的 CPU 优化缓存协调。
- 影响：SSO-05 第 3、5、7 条；§3 阶段 B 门槛。

**D-6 会话 family 持久化**

- 推荐：把 verify-hub 的 refresh family 持久化到 verify-hub 本地 SQLite（复用工作区已有的 `rusqlite`），只存 family 当前与上一枚 jti、到期时间、撤销标记、来源和 target，不存 token 明文。单实例限制写进部署说明；重启后已有会话可继续刷新。
- 备选：存入 system-config。多 OOD 可用，但每次 refresh 都产生一次写入，且需要 RBAC 隔离这些记录。
- 影响：SSO-03 第 5 条；SSO-05 的重启测试。

**D-7 DID 联合断言的验证**

- 推荐：在已有设计的流程上，把验证方式改为后端授权码交换，不改 ZoneDocument 格式。
  1. 目标 Zone 解析 `user_did` → OwnerDocument → default zone → ZoneDocument 的 hostname。OwnerDocument 必须来自 BNS 可信解析或由该 DID 的 owner key 签名。
  2. 浏览器跳转 canonical Zone 的 CP 联合登录页，携带目标 Zone DID、state、nonce 和回跳地址。用户在本 Zone 登录，并针对“该 DID + 目标 Zone”确认；consent 存在 canonical 侧，键为 (DID, 目标 Zone DID)。
  3. canonical verify-hub 只为 default zone 指向本 Zone 且非 `Limited` 的本地用户签发短期一次性 code，code 绑定 DID、目标 Zone DID、nonce 和到期时间。签发后回跳目标 Zone 的 callback。
  4. 目标 verify-hub 用 HTTPS 访问 canonical hostname 的 `/kapi/verify-hub` 兑换 code，得到身份断言（iss / aud 用 Zone DID）。TLS 主机名即信任根；远端 key 不进入本地 `verify-hub` 信任槽。DV 的 HTTP 需显式配置开启。
  5. 目标 verify-hub 按 D-1 映射本地 Profile，签发本 Zone 的受限 token pair。
- 备选：恢复公开 ZoneDocument 的 `verify_hub_info` 公布公钥，离线验签。需要 owner 离线密钥重签 ZoneDocument，现有 Zone 需重新激活或另设更新流程。
- 影响：SSO-09 全部条目；跨 Zone DV 需要 canonical、B、C 三个可互相 HTTPS 访问的 Zone。

**D-8 分享入口对象与策略层级（含 Q-06）**

- 推荐：入口归资源服务持有。AI Workspace 分享入口与 grants 同库存放，Message Hub 的 EA / Session 邀请入口存在 msg-center。
  - 入口记录至少包含：不透明入口 ID、资源引用、创建者、允许的登录方式（Guest / 本地注册 / 本地登录 / DID / EA 指定身份）、进入后授予的权限模板（按进入者主体落地为具体 grant）、到期与状态。
  - CP 只做编排：先调用资源服务的“描述入口”方法，取得可公开展示的信息和允许的方式，再走登录流程。
  - Zone 级设置只提供“允许外部注册”“允许 DID 联合”两个总开关；有效方式 = 入口允许 ∩ Zone 开关。
- 备选：CP 集中存储所有分享入口，资源服务只管 grant。需另行同步入口与 grant 的撤销。
- 影响：SSO-03 第 1、2 条；SSO-11 第 2 条；SSO-13 第 1 条；§4 的 Q-06 一并关闭。

**D-9 EA 重发渠道**

- 推荐：v1 系统不内建短信 / 邮件发送。EA 邀请创建时登记签发方（管理脚本或业务 App 实例）。用户在过期页申请重发后，系统生成新凭据并通知签发方，由签发方经原渠道发送；页面只显示已提交、受限或失败，不显示新链接。通知方式（kevent 事件或签发方登记的 kRPC 回调）在 SSO-02 冻结。首次邀请同样由签发方发送。
- 备选：系统新增邮件发送通道（SMTP 配置、发送记录、限流），短信另行集成。
- 影响：SSO-06 第 7 条；AC-05、06 的验收方式改为核对签发方收到的重发请求与实际送达。

### 0.4 范围与产品参数

- [ ] SSO-10（App access token）移出本批。PRD 不要求，企业客服场景由 EA 覆盖；保留在 Session Only 设计中，另行排期。待拍板。
- [ ] §4 中先需给值或取舍的产品项：Q-01（EA 默认有效期与调整主体）、Q-03（登录状态期限，同时决定 D-5 的 access TTL）、Q-05（DID 首次确认的展示资料）、Q-10（同客户多会话的邀请粒度）。其余可推迟到对应任务前。

### 0.5 不依赖上述决策、可先开工

- [ ] `validate_password_login` 拒绝空的存储密码。现状下，存储密码为空的账号可被任何人用 `sha256("" + nonce)` 登录。
- [ ] scheduler `map_api_user_type` 不再把 Guest 及未知类型回退为 `User`。
- [ ] SSO-12 中与身份模型无关的部分：群发送不依赖发送者 SENT 邮箱；附件读取携带 `context_path`；直接读取 host 群存储的数据层。先用现有 `Limited` 测试账号开发。
- [ ] SSO-04 的入口清单：列出所有 token 校验路径及当前判定方式，作为 D-4 闸口改造的输入。

## 1. 先对齐设计，避免从错误模型开始实现

| 问题 | 本轮实施约束 |
| --- | --- |
| 旧设计的 guest user 与 PRD Guest | PRD Guest 是无须注册正式账号的访客体验；旧文档 guest user 是有稳定主体、受限授权的系统用户。匿名请求、浏览器访客、外部本地账号、外部 DID Profile 必须可区分，不能都塞进一个 `guest` 字符串。 |
| EA / EB 与 Session Only Member | EA / EB 是目标 Zone 的 Local User；Session Guest 是某个 DID 在某个 Session 内的资源角色。EA、EB、DID 都可以获得 Session Guest 角色，登录方式不决定其群成员身份。不能把所有 EA 强制实现成 CRM guest 兑换。 |
| EA 链接是否一次性 | 新 PRD 要求有效期内可重复免密进入。拆开长期入口、限时免密资格、短期单次 callback / exchange code。后者可一次消费，不能因首次访问消费掉整个 EA 邀请。旧设计一次兑换规则继续适用于其 App 登录兑换凭据。 |
| Local User 是否形成 canonical zone | EA / EB 创建账号、设置密码、保存 Profile 都不能自动指定或变更 canonical zone。现有创建流程的 `set_default_zone_did` 不可照搬；本人明确建立身份归属是另一项操作。 |
| 系统 token 与资源权限 | 继续使用 `principal_kind=user` 和标准 `AuthTarget`；不新增 Message Hub 私有 token、`mhg_` 凭据或身份型 tunnel。认证成功不自动给邮箱、通讯录、群管理、任务、存储或 sudo 权限。 |
| App 身份源与当前客户端 | 身份源回答“谁证明此账号”；AuthTarget 回答“本次会话面向哪个 App 实例或系统服务”。两者分别校验。App 实例精确到 owner，不能只凭 AppId 授予代登录或转授权。 |
| DID 联合登录 | 只有用户本人唯一指定的 canonical zone 的 `verify-hub` 能在本路径代表该 DID。目标 Zone 的同名账号、本地 Profile、联系人合并和管理员签名都不能替代这条证明链。 |
| 首期范围 | PRD 核心闭环包含 EA、EB、DID、允许时的 Guest、同 Zone 跨应用。可分阶段交付，但不能把只完成 EA 或单应用演示称为混合 SSO 完成。钱包、复杂多身份工作台、历史归并、canonical zone 迁移产品流程后续独立做。 |

后续任务中的字段、接口名除已存在者和 `login_by_app_access_token` 外，均为待冻结的技术契约，不表示 PRD 已指定实现。产品待定项集中在 §4，不能在代码常量中偷偷作决定。

## 2. 当前实现：哪些复用，哪些必须改

| 模块 | 当前源码事实 | 实施落点 |
| --- | --- | --- |
| 标准认证目标 | [app_identity.rs](../src/kernel/buckyos-api/src/app_identity.rs) 已有 `AuthTarget::{App,System}`；[app_availability.rs](../src/kernel/buckyos-api/src/app_availability.rs) 校验 principal、target、token use、实例和 owner。 | 扩展可信主体和会话上下文，复用这些约束，不重建 target 模型。 |
| Verify Hub | [main.rs](../src/kernel/verify_hub/src/main.rs) 已有 password / JWT assertion / sudo / refresh / logout、token pair、内存 family 和 refresh 并发处理；没有混合登录入口。`login_by_jwt` 明确拒绝 Verify Hub session 作为 LoginAssertion。 | 联合登录需独立、限定用途的凭证交换；不能放开现有 JWT 登录来接受任意远端 session。 |
| 同 Zone 换票 | [sys_auth_backend.rs](../src/frame/control_panel/src/sys_auth_backend.rs) 的 `auth.prepare_app_launch` 已调用 `delegate_user_session`，从 Control Panel 普通用户会话换到精确 App 实例；callback 绑定路由、origin、完整 redirect，使用 host-only Cookie。 | 复用换票和 callback，不共享父域 Cookie；现有入口仅覆盖 CP→App，不等于外部受限身份和任意分享入口已接通。 |
| 撤销 | Verify Hub refresh / delegate 会查 family；[runtime.rs](../src/kernel/buckyos-api/src/runtime.rs) 的 `verify_trusted_session_token` 主要验签及校验 claims，`enforce` 加 RBAC，但无统一来源/scope/family 状态校验。Gateway 页面鉴权也不能仅靠 JWT 判断实时撤销。 | 不能把删除 refresh 缓存当作所有旧 access token 已失效。需要全入口可执行的撤销链。 |
| 用户创建和邀请 | [user_mgr.rs](../src/frame/control_panel/src/user_mgr.rs) 的 `user.create` 要管理员和密码，创建普通 User、密钥及设有 default zone 的 OwnerDocument；默认加入 `users`。`user.invite.accept` 接收已绑定目标 Zone 的 OwnerDocument，消费 pending invite，不签发登录态。 | 可复用事务、资料和管理校验，不能直接当作 EA 免密邀请、EB 公开注册或 DID 联合登录。 |
| 用户类型 | [control_panel.rs](../src/kernel/buckyos-api/src/control_panel.rs) 已有 User / Limited / Guest；`user_is_active` 排除 Guest；现有 guest availability 还表示无账号的匿名 `guest`。 | 分离账号状态、登录资格、权限类别和匿名公开访问；不能简单删除 Guest 拦截。同步 scheduler 的类型映射与 RBAC 生成。 |
| Message Hub 群角色 | [group_service.rs](../src/frame/msg_center/src/group_service.rs) 已实现 `invite_session_guest`、`accept_session_invitation`、`submit_guest_request`、会话 ACL / 历史区间 / Hook。 | 复用 Session Guest 状态机；缺统一外部身份登录、来源范围检查及无需个人邮箱的访问闭环。 |
| Message Hub 发送与附件 | [msg_center.rs](../src/frame/msg_center/src/msg_center.rs) 的 `handle_post_send` 先检查发送者 SENT 邮箱写权限；[object_access.rs](../src/frame/msg_center/src/object_access.rs) 有对象资源检查。前端 [objects.ts](../src/frame/desktop/src/app/messagehub/api/objects.ts) 不带 `context_path`，缓存仅按 ObjectId。 | 无个人邮箱的 Session Only 需要群发送鉴权及群对象上下文；不能为解决报错而授予整份邮箱权限。 |
| Message Hub 页面 | [MessageHubView.tsx](../src/frame/desktop/src/app/messagehub/MessageHubView.tsx) 及 datamodel 面向正常桌面用户，依赖个人会话索引、邮箱和联系人；已有 Session Guest 管理 UI。 | 新增受限分享入口和 host 群数据层，复用对话组件，不直接加载完整桌面数据。 |
| AI Workspace Guest / 分享 | [transport.ts](../src/frame/desktop/src/app/aiworkspace/api/transport.ts) 已保存 `gt_` 访客 token；服务端 `Caller::guest` 形成 `guest:` 主体，已有公开分享。它不是统一 BuckyOS Guest SSO。 | 迁入统一身份边界，同时保留无注册的公开参与体验；旧访客历史如何处理需显式决策，不能自动绑定 DID。 |
| AI Workspace 权限 | [access.rs](../src/frame/aiworkspace/core/src/access.rs) 已有 workspace / entity 子树的 read、append、update、comment 等能力，按祖先授权取并集，无 deny、无字段/记录级 grant。分享 UI 的 public read+comment 面向整个 workspace，surface 只是导航。 | 复用资源能力实现 DJ 权限；“某个 surface 链接”不能被当作仅授权该 surface。标准受限用户也不能自动继承所有已认证用户 `*` 的能力。 |
| SDK | 同级 [auth_client.ts](../../buckyos-websdk/src/auth_client.ts)、[runtime.ts](../../buckyos-websdk/src/runtime.ts)、[verify-hub-client.ts](../../buckyos-websdk/src/verify-hub-client.ts) 已支持本 Zone 登录、同源 refresh 和 target。 | 增加统一入口结果、外部身份、受限上下文及联合登录流程；避免每个应用另存一套正式身份。 |

## 3. 交付顺序与门槛

| 阶段 | 任务 | 依赖与完成门槛 |
| --- | --- | --- |
| A：冻结共用模型 | SSO-01～03 | 冻结身份、授权、scope、来源、存储和产品决策表，给出 wire / 状态机 fixtures。 |
| B：先完成权限底座 | SSO-04～05 | 受限用户的全入口默认拒绝、来源和会话撤销测试通过后，才开放新登录签发。开发期可用测试 fixture 验证，不能先对外开放再补鉴权。 |
| C：系统身份接入 | SSO-06～10 | EA、EB、Guest、DID 及 App access token 扩展共用 B 的底座；各入口可以分工并行。 |
| D：页面与资源闭环 | SSO-11～13 | 可提前做接口适配和页面；真实外部访问必须等 B 通过，并与相应登录入口一起验收。 |
| E：联调与交付 | SSO-14～15 | 两组 PRD 场景、旧 SSO 回归、负例和真实跨 Zone DV 都通过，才宣布核心闭环完成。 |

App access token 是旧 Session Only 设计保留的扩展任务，不是 EA / EB / DID 的同义词。可在共用底座上独立交付，不要求普通客户先具备 CRM 账号。

## 4. 产品待定项：在对应实现前关闭，不预设答案

| PRD 项 | 需要记录的决定 | 阻塞点 |
| --- | --- | --- |
| Q-01、Q-02 | EA 默认 TTL、可配置主体；重发是否撤销其它有效邀请。48 小时仅作测试样例。 | SSO-06 的参数和状态机；不改变有效期内可多次免密原则。 |
| Q-03、Q-04 | 长期状态期限、重认证条件、canonical zone logout / 改密向目标 Zone 的传播范围。 | SSO-05、09、11；现有 15 分钟 session / 7 天 refresh 是代码现状，不是新产品承诺。 |
| Q-05 | 首次 DID 确认展示、允许共享的 Profile 字段、后续重新确认条件。 | SSO-09 的 consent 契约和页面。 |
| Q-06 | Zone / App / 单资源各自可控制哪些入口及冲突优先级。 | SSO-03、07、08、13；技术建议是按有效策略求交，但需明确资源 owner 权限边界。 |
| Q-10 | 同客户多会话的账号复用与邀请粒度。 | SSO-06、12；先保证同账号与多资源授权可独立表示，不默认一个邀请开放所有历史工单。 |
| Q-13 | Verify Hub、Control Panel、SDK、业务 App 的登录处理责任和状态归属。 | SSO-01～03；本文建议系统统一管理身份/会话，CP 承担浏览器编排，资源方管理资格。 |
| Q-07、Q-08、Q-09、Q-11、Q-12 | Guest / Local User / DID 归并与绑定、忘密恢复、canonical zone 变更、无归属钱包身份。 | 独立设计；首期不自动合并、不用旧 EA 邀请绕过已设密码、不在普通登录中变更归属、不展示未实现钱包入口。 |
| PRD 补充建议 | 身份冲突、撤销、重发限制、返回原资源和错误体验的交付范围。 | SSO-06、11～14。不能借“待评审”放行串号、越权或凭过期凭据恢复权限。旧 Session Only 的受限会话撤销要求仍是开放门槛。 |

## 5. A：共享模型、协议与状态

### SSO-01 / P0：分开主体、登录凭据与身份归属

- [ ] 定义可信主体记录：稳定且不可复用的内部 ID、业务 DID / subject、账号类别、状态、凭据状态、本地 Profile、认证来源。明确 token `sub`、用户 ID、DID 的转换，由共享解析器完成，不能靠昵称或拼接 `did:bns:<username>` 推断。
- [ ] 给 EA / EB 定义“可设置密码的受限 Local User”，默认无普通 `users` 业务权限。复用或细分现有 UserType 的选择在这里一次确定；`Limited` 不是天然满足要求的现成答案。匿名请求、浏览器 Guest、App 映射受限用户和外部 DID Profile 有明确区别。
- [ ] “未设置密码”是独立凭据状态，password / sudo 入口在验证密码前显式拒绝该状态。当前 `validate_password_login` 未检查空 stored password，不能仅用空字符串或可预测占位密码表示 EA 尚未设密码；DID-only / Guest 同样不能意外走密码登录。
- [ ] 普通本地账户创建与“本人建立 canonical zone”拆开。EA / EB 不生成一份可被当成全局归属授权的 OwnerDocument；若内部表示仍使用 DID，必须与可对外证明的 DID authority 分离。也不要求这些用户交出身份根私钥。
- [ ] 注册身份域和绑定 `(identity_domain_id, external_account_id) → subject`，以唯一约束和事务保证重试/并发得到同一主体；身份域绑定可信安装实例及 owner，卸载重装或同名实例不得继承原域。
- [ ] 手机号/邮箱是可变绑定，不进入主体主键；同名、同手机号或私有 Contact canonical 合并不能作为登录依据。已有普通用户/DID 的关联必须验证控制权，不准借 App guest 入口冒认。
- [ ] 联动 `UserSettings` / `UserInfo` / Profile、scheduler 用户类型和 RBAC 映射、身份解析消费者。原有普通账号默认权限保持明确，外部参与者不被 fallback 映射为普通 User。

落点：`buckyos-api/{control_panel,app_availability,runtime}.rs`、`control_panel/user_mgr.rs`、scheduler 的用户加载和 RBAC 构建、WebSDK 用户类型。验收：EA 设密码、刷新、换设备均不换主体；EA / EB / DID Profile 均不产生 canonical zone 修改。

### SSO-02 / P0：统一会话上下文和生命周期协议

- [ ] 在现有 `AuthTarget`、`TokenPair`、`ValidatedTokenClaims` 上定义可验证的认证方式、来源身份域/issuer、认证强度、family/session 标识、批准的权限上限、授权来源关联和撤销版本。明确哪些在签名 claims，哪些通过服务端记录查询。
- [ ] principal 继续为 `user`，受限身份不得伪装成 system/app。App target 精确绑定实例和 owner；System target 单独批准。来源 App 与 target 分别记录，refresh token 不能作为业务 session，sudo 不能从这些入口顺带获得。
- [ ] 会话 scope 是上限，不是权限真相源。实际请求始终求交：当前主体授权 ∩ 当前 target 能力 ∩ 来源/会话上限 ∩ 当前资源规则。新增加的资源授权不能让旧的 App 代登录会话自动越过上限。
- [ ] 列出类型化协议：入口查询/准备、各方式认证或交换、EA 首次设密码/重发、DID authorize/return、当前身份、refresh/logout、来源和会话撤销。复用现有接口可满足的部分；每个新方法同时定义 caller、target、资源、origin、幂等键和预期失败状态。
- [ ] 在 `verify_hub_client.rs` 的 request、trait、adapter、Rust client 与 WebSDK 同步协议。现有 kRPC 错误经传输会变为字符串，业务需要分支的结果使用结构化 status，不让 UI 解析错误文本。
- [ ] 区分可重试错误、未认证、无权限、免密资格过期/已撤销、密码已设置、资源关闭/删除、身份冲突和来源不可用；公开入口的返回信息不泄露其他客户或私密资源。

验收：同一 wire fixture 在 Rust / TypeScript 解析一致；来源、scope、target 缺失或矛盾时拒绝，不能回退为无范围的旧会话。开发阶段直接迁移协议，不新增旧格式兼容旁路。

### SSO-03 / P0：明确状态的真相源和事务边界

- [ ] 为账户/绑定、身份域授权、分享入口、EA 免密资格、资源 grant、DID consent、会话 family/revocation、发送 outbox 和审计分别列出存储 owner、唯一键、版本/CAS、到期与删除规则。优先复用现有 system-config 事务及已有存储组件，不先引入新通用服务。
- [ ] 长期入口与凭据分表/分字段表示：入口 ID 只用于定位业务；凭据以不可逆摘要保存并设有效期；原始 secret 不落日志、审计、analytics 或 Referer。长期访问地址不能单凭公开 ID 重新获得免密资格。
- [ ] 资源授权保留授予者、接收者、资源、动作、期限、来源及转授依据；群/Session 成员表仍是成员资格真相源，Workspace grants 仍是对象权限真相源，不另建一份易失步的访客成员表。
- [ ] 账号预建、资源邀请和发送不能假设跨服务天然原子：定义可重试编排、幂等关联和补偿；发送失败不丢账号/会话关系，不伪报已送达。身份创建本身不隐式授权。
- [ ] 从内存 pending/family 中区分可丢失的短期过程与必须持久的长期状态。落实重启、多实例路由/原子消费、备份恢复、撤销记录防回滚；首期若仅支持单实例，明确部署限制并验证重启结果，不能宣称多副本可用。

验收：并发创建无重复身份；并发设密码与邀请登录有确定提交顺序；恢复备份不会复活已撤销凭据或将同名 App 继承为原身份源。

## 6. B：权限底座与撤销，签发之前完成

### SSO-04 / P0：全入口默认拒绝及授权交集

- [ ] 把受限主体判定和 scope/source 检查接到共享认证/授权上下文；仅给 token 增加字段不算完成。`runtime.enforce` 保留完整 AuthTarget 实例上下文，不能用 AppId 级 `authorization_key` 代替实例级管理和转授授权。
- [ ] 创建身份、邀请加入、代登录、资源使用、可转授是五类不同能力。App 的授予上限为“系统允许范围 ∩ owner+instance 获准转授范围 ∩ 资源方允许范围”；不能因 owner 是管理员或 App 有读取权就允许转授。
- [ ] 为下表产出逐方法审计清单：入口、认证来源、主体解析、RBAC、资源检查、scope、撤销、测试。未支持受限主体的方法明确拒绝；只验证 `principal_kind=user` 或“自己的数据”不够。

| 审计面 | 必查点 |
| --- | --- |
| Control Panel / system-config / scheduler | user.*、profile/doc/settings/key 写入、App 安装、Agent 创建、sudo、用户类型和角色生成；不得借 self 修改可信身份字段或默认加入 `users`。核查已有公开 `user.invite.accept`：JSON 解析 OwnerDocument 不等于验证控制权，不能成为新开户策略的旁路。 |
| msg-center | `caller_identity`、`authorize_resource`、`owner_session`、`group_rpc_authenticated`、group HTTP、`contact.*`、通知、批量接口；User 可观察 Agent、非 User/内部请求快捷分支必须明确边界。 |
| Message Hub 对象与代理 | 群消息、附件对象/内容、范围下载、NDN 代理和转发；确认资源引用与当前可见性，不能仅凭 ObjectId 或代理的 service token。 |
| AI Workspace | `RuntimeAuth`、Caller、`require_signed_in`、public / `*` / 指定主体 grants、RPC/HTTP/订阅/导出；系统受限用户不能因 sub 不以 `guest:` 开头就获得所有已认证用户能力。 |
| task-manager / workflow / OpenDAN / AICC | creator / owner 快捷分支、任务列表和结果、执行工具、计费操作、Agent 观察和订阅；调用者不能被替换成 App owner。 |
| nfs-server / ndn-mgr / 文件与上传 | 公开分享与受保护文件分别处理，路径、对象、上传和配额均需受限上下文。 |
| kevent / kmsg / stream | topic/资源过滤、订阅建立及持续输出、撤销后断开或停止输出；不能授予全局事件订阅。 |
| Gateway 与直连后端 | 页面路由、kapi、HTTP、批量和旁路端口结论一致；网络请求不能触发无 token 的内部放行。公开 bootstrap 仅公开壳和必要接口。 |

- [ ] 新建受限账号的零权限测试先落地：即使签名、target、exp 都合法，也不能枚举邮箱、联系人、其他 Session/Workspace、任务、系统配置或个人存储；已支持的最小授权逐项加入。
- [ ] 授权状态不可确认时拒绝受保护请求；保留已有明确公开资源的匿名行为。普通用户、系统服务和设备 bootstrap 回归不得被受限身份改造破坏。

验收：交集任一项缺失均拒绝；RPC/HTTP/对象/事件/代理/批量结果一致。审计清单与负例通过是开放所有新受限 token 签发的前置。

### SSO-05 / P0：会话、来源和资源撤销闭环

- [ ] 统一检查 family、账号状态、来源身份域/代登录权、会话上限和资源当前状态。复用 refresh 锁内复核与并发去重，补齐普通 access 请求验证，避免 logout 后 refresh 或 callback 重新插入有效 family。
- [ ] 专门改造 refresh grace 分支：当前命中宽限期会在 `validate_refresh_principal` 前返回缓存 pair，不能视为已经复查用户/来源状态。正常刷新、宽限重试、delegate 和 callback 都不能绕过新的撤销判定；family/JTI 的 CAS 不能替代这些检查。
- [ ] 实现受限会话的可持久撤销依据，以及跨进程缓存一致性。撤销提交后的下一次受保护请求生效；缓存 TTL 或 JWT 自然过期不能代替这条语义。测试命中缓存、连接保持、服务重启和状态服务故障。
- [ ] Guest 停用/身份域停用/代登录权撤销拒绝新登录、刷新和相关旧会话；只撤销某 Session/grant 时不注销无关资源的独立有效会话。App 卸载保留历史身份，停止旧来源代登录。
- [ ] 用户退出/单设备撤销使对应 session+refresh 链失效；清 Cookie 之外撤销服务端凭据。用户类型升级、身份绑定迁移不让旧会话自动获得新权限。
- [ ] 明确普通 CP→App 独立 family 与受限来源衍生会话的关系。现有桌面 logout 不撤销已签发 App family；混合 SSO 的来源/范围约束必须在换票后保留，不能借现有 delegate 洗掉受限来源。
- [ ] 对受限会话的持续流、订阅和附件请求实施撤销；UI 收到失效后停止拉取并清除相关内存缓存、Blob URL。已经合法送达客户端的历史内容不能声称可远程抹除。

验收：持旧 token、旧 refresh、未消费 callback、缓存及已建立订阅重试均不复活撤销权限。跨 Zone logout 的传播按 Q-04 明确结果，不额外承诺全局退出。

## 7. C：统一身份接入

### SSO-06 / P0：EA 定向邀请和首次设密码

- [ ] 提供获准管理方的“预建指定 Local User + 关联业务资源 + 生成专属入口”能力。user ID、绑定渠道和指定身份来自可信记录；链接持有者不能改成任意 DID/user ID。不同客户不同凭据，同客户多会话按 Q-10。
- [ ] EA 凭据绑定账号、入口、过期时间和凭据版本；每次登录重新检查密码状态、账号状态、邀请撤销、来源及 target。有效期内可重复换取标准会话，单次 callback code 独立防重放。
- [ ] EA 凭据到期限制新的免密登录，不等于会话、账号或历史删除。已有有效会话按独立生命周期和当前资源资格处理；邀请撤销对其衍生会话的影响在 SSO-05 中明确，不把“到期”与“来源代登录权被撤销”混为一谈。
- [ ] 采用下表实现状态机，并覆盖返回原业务所需的最小导航资料；链接登录本身不等于加入 Session，也不等于拥有分享文档权限。

| 条件 | 新的免密登录 | 页面及后续 |
| --- | --- | --- |
| 凭据有效、未设密码、账号可用 | 允许；首次访问不永久消费 EA 凭据 | 使用预设身份；提供不强迫的设密码入口。 |
| 凭据过期、未设密码 | 拒绝 | 可申请向原绑定渠道重发；不向页面返回新 secret。 |
| 已设密码、无适用会话 | 拒绝旧凭据 | 原地址可继续作入口，使用密码登录。 |
| 已设密码、有适用会话 | 不再使用旧免密资格 | 复用现有会话，再检查资源权限。 |
| 邀请/账号/资源被撤销或业务关闭 | 对相应操作拒绝 | 准确区分原因；重发不恢复已撤销资格。 |
| 当前 Bob 会话，链接指定 Alice | 不静默替换或沿用 Bob | 显示身份冲突并要求明确继续/取消；不把操作记给错误用户。 |

- [ ] 首次设置密码只针对当前已认证 EA 账号，复用标准密码格式校验，事务内写入密码并提升凭据版本/终止原免密资格。不能要求先用不存在的旧密码 sudo，也不能开放一般 settings 写入来完成此事。
- [ ] 设密码不换 user ID / DID、不丢成员关系、不授予普通用户权限、不改 canonical zone。并发登录与设密码以提交顺序复核，任何旧的尚未完成兑换不能在设密码后新签发免密会话；已有效登录终端的处理按 Q-03 单独定义。
- [ ] 实现向原绑定短信/邮箱的重发服务及可靠发送记录，限流、渠道脱敏、失败/重试和幂等。发送渠道作为集成边界选定；不能拿返回测试 URL 代替真实重发验收。已设密码不允许用重发恢复免密，忘密另行处理。
- [ ] 管理方提供查询、撤销和到期状态；邀请人有邀请权不等于有客户代登录权，管理 API 和登录 API 分开授权。

落点：Control Panel 用户/浏览器编排、Verify Hub 签发、统一身份存储、msg-center 资源邀请、SDK/CLI。验收对应 AC-01～06、21～23，必须含有效期内第二次进入与设密码并发负例。

### SSO-07 / P0：EB 自助注册和原账号登录

- [ ] 增加由分享有效策略开启的注册入口；服务端验证原资源/目标 Zone、用户名唯一性、密码格式、资料规则和配额。公开请求不能指定 admin、user_type、角色、owner、canonical zone 或任意资源授权。
- [ ] 注册创建受限 Local User，不调用现有 `user.create` 的默认普通用户授予和归属创建逻辑；凭据和业务 grant 分开提交/编排。重复提交不建第二个账号。
- [ ] 已有目标 Zone 本地账号可直接登录；注册/登录成功回原内容，资源已经关闭或未授权时显示对应结果。显示账号属于哪个 Zone。
- [ ] 密码登录和 refresh 接入 SSO-02/05，不能因使用 password 方式就去掉外部账号的来源与权限类别；不自动关联已有 DID 或 Guest 历史。

验收对应 AC-10、11、16、18、23，含公开接口伪造角色/账号类型/授权与并发抢名。

### SSO-08 / P0：统一浏览器 Guest

- [ ] 定义无须注册正式账号的浏览器访客主体和标准会话建立方式；只在有效分享策略允许时提供。可以有稳定的内部主体，但不能在界面或授权上冒充已验证手机号、Local User 或全局 DID。
- [ ] 复用同浏览器有效状态；不共用全站一个 `sub=guest`，也不把可自报的浏览器 ID 当成账号控制证明。丢失状态/换浏览器允许新 Guest，不承诺找回旧身份。
- [ ] 将 AI Workspace `gt_` / `guest:` 私有身份适配到统一上下文，避免新建另一份 Message Hub 匿名身份系统。记录旧匿名记录的处置决策；不默认历史合并、不根据相同昵称转移权限。
- [ ] Guest 的注册、DID 登录或身份切换走明确动作，清除旧身份缓存，资源重新授权。新认证状态不自动继承匿名记录的所有权。

验收对应 AC-09、16；同浏览器续用、状态丢失、关闭 Guest 入口后的有效权限变化均可验证。

### SSO-09 / P0：DID canonical zone 联合登录

- [ ] 核对现有 DID / OwnerDocument / ZoneDocument 解析能力，冻结“该 DID 本人授权的唯一 canonical zone → 该 Zone 的 verify-hub key”证明链。`default_zone` 字段只有在可信且适用于该主体的文档中才有证明力；目标 Zone 的本地写入不算授权。
- [ ] 支持 Alice 由 Bob 承载 canonical zone：验证 Alice 的归属授权和来源，不要求 Alice 等于 Zone owner，也不要求她拥有物理 OOD。当前 `OwnerDocument::get_default_zone_did` 取绑定列表首项，`is_bound_to_zone` 仅判断列表包含，bound 不等于 canonical。可绑定多个 Zone，但对外认证只认可信的唯一 canonical；归属缺失、证明冲突或无法验证时不猜测。
- [ ] 定义专用联合登录证明/短期 code 及后端交换：绑定 DID、来源 Zone、目标 Zone、认证用途、有效期、请求 nonce、浏览器流程状态和允许披露的资料。接收方与 redirect 来源可信，防重放、登录串会话和开放重定向；密钥和兑换地址不能由页面任意指定。
- [ ] canonical zone 首次向某个目标 Zone 使用 DID 时显示身份、目标和资料范围，明确同意后签发；取消不签发。consent 以 DID+目标 Zone 为边界，向 B 同意不能替代向 C/D 同意。
- [ ] 目标 Zone 验证证明后映射/创建本地 Profile，签发本 Zone 标准且受限的 token pair；不创建新密码，不覆盖已有同名本地账户，不改变 canonical zone。下游业务继续只验证目标 Zone 的标准会话。
- [ ] 独立限制本地签发与对外证明资格：目标 Zone 不能因持有本地账户、Profile 或自己的 Verify Hub 私钥，代表外来 DID 再登录第三方。不得把远端 key 混入普通 session 的 `verify-hub` 信任槽。
- [ ] 后续访问在有效 consent、会话和资源资格下复用状态；来源不可用、key/归属变化、过期或撤销时按已定策略处理，不能偷偷改走目标 Zone 或另一个身份。

落点：Verify Hub + Control Panel 专用联合流程、共享 DID/身份解析、SDK；同时核对同级 [name-lib/user.rs](../../buckyos-base/src/name-lib/src/user.rs) 与 [name-client/profile_resolver.rs](../../buckyos-base/src/name-client/src/profile_resolver.rs)，扩展文档类型或解析器时同步调用方。验收对应 AC-12～20、24，含第三方 Zone 接受伪造来源的明确负例。

### SSO-10 / Session Only 扩展：App access token 接入

- [ ] 按旧设计提供 `login_by_app_access_token`：登记身份域、公钥或受信服务端校验方式、获准 AppInstance/owner、允许 target 和范围；先选定验证契约，不能让 token 自报任意校验 URL。
- [ ] 校验真实性、用途、接收方、期限、稳定外部账号和绑定状态；App service token 不能换客户会话，App 不能指定任意普通用户/管理员 DID。凭据定位字段只能作查找线索。
- [ ] 身份域登记、建号与兑换分别限权、限流和限额；按来源 App/账号/请求来源统计，公开失败结果不泄露账号是否存在。记录签发和绑定审计，不保存凭证明文。
- [ ] 链接登录兑换凭据单次消费，以绑定该次兑换的 nonce 支持合法幂等重试；更换 nonce 重放拒绝。它与 EA 可重复免密资格分离：若 CRM 同时服务 EA，每次由仍有效资格产生独立兑换过程，不能混用生命周期。
- [ ] 签发标准受限用户 token pair，保留身份域、来源、强度和批准上限；refresh 与跨 target 换票重新检查来源状态且不得增权。
- [ ] App 停用/卸载/代登录权撤销立即影响依赖该来源的会话；已有其它独立登录来源按各自规则处理。外部账号换设备重新认证映射同一主体。

验收：跨 owner 同 AppId 冒认、跨身份域同账号、伪造主体、错误 audience、重复消费、来源停用和旧会话提权均失败。Message Hub 不增加自己的兑换中心或用户凭据库。

## 8. D：同 Zone SSO 与业务入口

### SSO-11 / P0：分享入口编排和跨应用复用

- [ ] 复用 Control Panel 的 target/origin/redirect resolver、pending callback、Cookie 和 refresh 流程，增加受限入口可用的统一身份选择/恢复能力。可复用 `auth.prepare_app_launch` / `delegate_user_session` 内部逻辑，但不能简单放宽调用者和来源检查。
- [ ] 从分享链接先解析可信入口上下文，再展示实际允许的 Guest、本地登录/注册、DID 等方式；EA 只展示指定身份路径。只公开必要的资源介绍，不先加载私密会话内容。
- [ ] 无 session 时公开 bootstrap、受控认证/重发/回调入口；验证 Zone 根域、系统入口、App 子域、移动端短信/邮件打开和跨 origin 请求。公开页面不等于放开数据 API，不能通配公开 `/kapi/*`。
- [ ] 同 Zone Message Hub→Workspace 复用主体；同 target 可复用适用会话，跨 target 重新签精确目标会话。Workspace 必须独立检查访问资格；若原会话只准某 Session，不能把 scope 直接放大，需按来源允许范围重新授权。
- [ ] 保留 host-only Cookie、精确 AppInstance/owner、完整 path/query/hash。不同 target、route 被重新指向其他实例、callback origin 不一致均拒绝；URL 不传长期 session/refresh token。
- [ ] 当前身份与 EA/DID/Guest 入口冲突时显式选择，取消不覆写 Cookie。初期不做复杂多身份管理，但不能让已有管理员 session 被静默混用；清理跨身份的用户、对象、会话和权限缓存。
- [ ] SDK 提供当前可信主体、登录方式、所属/目标 Zone、资源拒绝状态和续期接口。所有消费者删除自行猜 DID、以“登录成功”代替权限检查和所有错误跳 `/login` 的逻辑。

验收对应 AC-07、08、14、21、23；在不同 App origin 做真实浏览器测试，不能只用 Desktop 内两个面板证明跨应用 SSO。

### SSO-12 / P0：Message Hub 的 Session Only 闭环

- [ ] 复用 `Invited → Included`、`accept_session_invitation` 和 `submit_guest_request`。身份建立、群侧邀请/批准、本人接受分别记账；进入页面可顺序执行，但登录成功不自动成为群正式成员。
- [ ] 分清已有群 invite link（群加入凭据）、Session Guest 邀请和 EA 免密资格。当前 `SessionMembershipRecord` 无独立 invitation ID/expiry，需定义 Session 邀请版本、有效期、撤销/重新邀请和旧请求拒绝规则；采用既有幂等框架并让前端传入幂等键。登录成功而加入失败可重试，合法接受重试返回原结果，旧版本不能接受新邀请，移除/封禁后旧请求不能重新加入。
- [ ] 只允许具名、`allow_guests=true`、成员策略非 `inherit` 的 Session；保留封禁、上限、频率、join Hook 和历史可见区间。邀请接受前只授权本人查看最少邀请资料及接受动作，不预先开放历史。
- [ ] 给 `group_rpc_authenticated`、group HTTP、对象读取和事件入口统一接入 SSO-04 的 RBAC/source/scope 检查，再执行群/Session ACL；保留经验证的客户端身份以执行 `allowed_clients`，不得固定伪装为总允许的客户端。
- [ ] 改造群发送入口：按当前会话、消息操作和 `from=认证 DID` 校验后写群存储；无个人邮箱的用户不能先被强制要求 SENT 权限。不要用创建个人邮箱、替换 App owner 或授予 `obj://msg-center/*` 解决。
- [ ] 检查 `is_local_recipient` 与群消息、邀请通知、事件 fanout 的 SENT / INBOX / owner_sessions 投影，身份在本 Zone 有记录不代表具备个人投递能力。Session Only 默认不注册本地投递能力；用数据库断言验证接受邀请、发言和接收消息后没有隐式个人邮箱/投递记录，群记录及客服和 Agent 的既有投递正常。
- [ ] 增加直接读取 host 群存储的数据层，使用 `group.list_sessions/list_messages/changes`，不依赖 `owner_session` / INBOX 投影，也不要求通讯录权限。轮询可作为首期更新方案；使用 kevent 必须有相同资源过滤和撤销。
- [ ] 新增公开可达的受限入口（路径在路由设计中确定），仅加载身份流程与获准会话视图，复用 `ConversationView`。名字/头像用可读成员资料；不暴露群其他 Session、客服邮箱、通讯录、群管理或个人归档/置顶功能。
- [ ] 附件读取携带群/Session/消息引用上下文，后端同时验证 Session 可读、消息处于可见历史、对象确被引用；猜 ObjectId、跨会话 context、删除/撤回后不可读对象均拒绝。对象缓存按 Zone+主体+上下文隔离，撤销/切换时清理。
- [ ] 首期不开放 Session Only 媒体上传；后续单独增加写权限/配额。客户打开群内 Workspace 链接按 SSO-11 处理，不复用客服的身份或权限。
- [ ] 资源授权结束、移出、封禁、删除及时停止无权读取；Session 归档保留既定只读语义，不等同于删除。账号/DID 保持稳定，历史消息 `from` 不改写。Contact 的展示合并不改变授权主体；跨 Zone joined-group 自动同步是另一个交付项，不能与浏览器 DID 联合登录混称完成。

落点：`msg_center/{message_hub,owner_session,group_service,group_http,object_access,msg_center}.rs`、`buckyos-api/msg_center_client.rs`、Desktop messagehub API/datamodel/路由。验收：Alice/Bob 同一工单、Alice 其它未授权工单、无邮箱客户发消息/读附件、客服和 Agent 正常收发，以及所有撤销负例。

### SSO-13 / P0：AI Workspace 分享策略和操作权限

- [ ] 以现有 workspace/entity grants 为资源真相源，增加分享入口允许的认证方式及有效策略查询；控制层级按 Q-06，不把“允许 EB/DID”变成自动 grant 全 workspace。
- [ ] `RuntimeAuth` 和 Caller 消费统一受限上下文；修正仅凭 `guest:` 前缀区分访客/已认证的假设。明确 `public`、`*` 和具体主体对新身份类别的匹配，防止标准 Guest token 自动获得创建/导入/复制或 `*` 授权。
- [ ] DJ 场景优先复用资源结构：公开可读公告/播放状态，歌单实体子树仅授参与者 `append`，切歌和公告编辑只授 DJ。验证 append 的实际写操作所需能力；如果必须增加权限表达，先冻结最小变更，不能通过全 workspace `update` 绕过。
- [ ] 权限按祖先 grant 取并集，没有 deny 可补救上层宽授权。检查所有祖先授权；分享单 surface 时明确实际授权对象，避免链接只指一个 surface 却开放其它私密内容。
- [ ] RPC、HTTP 读取、批量变更、订阅、附件、导出和后端工具执行相同用户/scope/资源检查；页面隐藏按钮之外，直接请求切歌、改公告、改 grants 也必须失败。
- [ ] 身份来源/所有者/目标 Zone 可见，Guest 和 EB/DID 进来后不自动成为 workspace owner。资源关闭、grant 撤销和账号停用不循环跳登录。

验收对应 AC-08～20；用 Guest、EA/EB、DID 和 DJ 分别执行允许/禁止动作。播放同步、音频编排不属于本任务；轻量背景个性化列 P1，不阻塞认证闭环。

## 9. E：联调、文档与完成条件

### SSO-14 / P0：验证矩阵

每项记录执行命令/环境、预期、结果与失败证据；未运行写“未验证”。mock 只验证组件和协议，不替代真实 Cookie、发送渠道及跨 Zone 验收。

| 测试组 | 必须通过的内容 | 来源 |
| --- | --- | --- |
| EA 正向与生命周期 | 两客户各用专属链接；有效期重复进入；原账号设密码；旧凭据失效；有效现有会话可继续；过期向原渠道重发。 | AC-01～06 |
| EB / Guest | EB 注册返回原资源、旧账号可登录；同浏览器 Guest 延续、状态丢失；不自动合并历史或改变归属。 | AC-09～11、18 |
| 同 Zone 跨应用 | Message Hub→授权 Workspace 不重复注册；未授权文档拒绝；跨 App origin 换票仍精确绑定实例、owner、来源及 scope。 | AC-07、08、14 |
| DID 三 Zone | Alice canonical zone→B 首次确认/取消/再次访问→C 独立确认；无新密码；Bob 承载 Alice 的合法归属可用；DJ/B 伪称 Alice 来源在 C 被拒绝。 | AC-12～20 |
| DJ 权限 | Guest/本地/DID 可按规则 append 歌曲，不能切歌、改公告、改 grants；未实现钱包不展示。 | AC-16、24 |
| 默认拒绝与实例隔离 | 无 grant、App 无能力、scope 缺失、资源拒绝各自失败；同 AppId 不同 owner 隔离；不继承 `users`、owner、sudo；不可冒认普通用户。 | Session Only §11 |
| 群完整路径 | 邀请前/后、Included/Removed、历史边界、Hook、`from`、客户端限制、零邮箱投递断言、附件引用、RPC/HTTP/事件一致；接受响应丢失重试、邀请过期、旧版本重放、并发接受/撤销。 | Session Only §6、§11 |
| 并发与恢复 | 并发建号/设密码/重发/refresh/logout/callback；单次 code 防重放而 EA 多次有效；服务重启、备份恢复和缓存中的撤销。 | Session Only §9 + AC-03、04 |
| 页面与异常 | Bob/Alice 身份冲突；链接/资源/权限失效区别；原目标恢复；移动/桌面；来源不可用；无凭证日志。 | AC-21～23 + PRD §10 |
| 原功能回归 | 普通本地用户、Desktop/App SSO、refresh 并发、logout、App availability、群成员/Agent、现有公开 Workspace；LoginAssertion/session/refresh/sudo 边界。 | 当前实现 |

现有验证入口，按改动选择执行；新增混合 SSO 用例纳入相应套件。Rust 命令从 `buckyos/src` 运行：

```bash
cargo test -p buckyos-api -- --test-threads=1
cargo test -p verify_hub -- --test-threads=1
cargo test -p control_panel -- --test-threads=1
cargo test -p msg_center -- --test-threads=1
cargo test -p aiworkspace -- --test-threads=1
cargo test -p aiworkspace-store --test guest --test kernel -- --test-threads=1
```

SSO-04 涉及的其它服务分别跑其定向测试；协议改动完成后运行 workspace 测试及 `uv run buckyos-build.py`。从 `buckyos/src/frame/desktop` 运行 `pnpm check`、`pnpm lint`、`pnpm build`，并在 `tests/e2e` / `tests/datamodel` 新增统一身份和受限页面用例；WebSDK 在同级仓库跑 auth/refresh/runtime 定向测试与构建。

AI Workspace 另有独立测试目录，默认 Desktop `test:e2e` 不会发现。在 Desktop 目录运行：

```bash
pnpm exec playwright test --config=playwright.aiworkspace.config.ts permissions.spec.ts collab.spec.ts
pnpm exec playwright test --config=playwright.aiworkspace.zone.config.ts
```

前者使用真实 standalone 后端、静态测试身份及 mock Desktop，只验证资源权限/UI；后者需要真实 Zone。现有 `test_group_service.rs` 的测试 verifier 将 `authorize` 固定放行，测试 guest 还注册为 local recipient；必须新增接入真实授权上下文且无个人邮箱的测试，不能把既有群测试通过当作 SSO-04/12 已完成。

从 `buckyos` 根目录运行已有 Gateway 和真实 Zone 检查：

```bash
uv run src/test/test_boot_gatweay/run_debug_tests.py
uv run src/check.py
bash test/local_user_dv.sh
```

复用 [test_messagehub_groups.ts](../test/test_msg_center/test_messagehub_groups.ts) 和 [test_messagehub_sessions.ts](../test/test_msg_center/test_messagehub_sessions.ts) 扩充 Session Only 场景；真实环境需已激活、部署对应源码版本并配置测试账号。DID 联调至少准备 canonical、B、C 三个独立 Zone；原渠道重发使用可验证送达的测试短信/邮箱。不能用同域 mock 冒充跨 Zone / 多 origin 测试。

### SSO-15：文档、分发和交付

- [ ] 更新 SSO、verify-hub、RBAC、UserType、Local User 生命周期、Control Panel 协议、SDK 使用说明、Session Only、Self-host Group 接入、Message Hub UI_DATAMODEL 和 AI Workspace 分享文档。删除“guest 等同匿名且永不能登录”“任意本地 Profile 即归属证明”等混淆表述，并明确已实现/规划范围。
- [ ] 更新 [桌面 App 自动登录记录](desktop-app-sso-auto-login-todo.md) 中与受限来源、family 和撤销相关的取舍，保留普通桌面行为与本轮变化的准确边界；不把旧的独立 family 结论套给所有混合会话。
- [ ] Rust 共享类型、服务、SDK/CLI、前端、Gateway 模板和测试一起迁移。修改源码和生成入口，按构建流程分发 SDK；不只改 `dist` 或 `rootfs` 生成副本。如改 Gateway 公共配置，检查同级 `cyfs-gateway` 同步副本。
- [ ] 检查 Jarvis `cmd_skills/buckyos_admin.md`、`buckyos_app_dev.md` 等受影响手册，补充外部账号/邀请/授权管理的真实命令或说明无需变化的依据；业务脚本有权限才能创建身份、发邀请、转授资源。
- [ ] 记录部署/数据初始化与恢复方案。开发阶段不默认做旧协议兼容，但现有账号、群历史与已保存公开 Workspace 的处置须明确；结构变更不能无声删除业务数据。
- [ ] 输出交付记录：已实现任务、未决定事项、身份/权限/撤销协议、测试与 DV 结果、单实例或多实例限制、尚未验证的渠道和浏览器。所有 TODO 按实际证据勾选。

核心闭环完成条件：EA / EB / DID / Guest 四种用户体验均在系统统一边界下成立；Message Hub 受限客户不依赖个人邮箱权限；Workspace 操作角色正确；跨应用不重复注册；DID 跨 Zone 保留逐目标首次选择；EA 设密码终止旧免密资格；普通访问永不改变 canonical zone；所有已签发的受限会话都满足当前授权、来源和撤销规则。仅登录页面可用或 JWT 能验签不算完成。
