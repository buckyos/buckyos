# BuckyOS 混合 SSO 实施 TODO

日期：2026-10-21。状态：用户类型前置已交付；§4 的用户决策已于同日全部确定；混合 SSO 待实施。本文是代码核对后的实施计划，不是功能完成记录。

目标：在现有本 Zone SSO 上实现专属邀请、自助注册、DID 登录和允许时的 Guest 入口，同时贯通 Message Hub 外部客户会话、AI Workspace 分享、同 Zone 跨应用身份复用。先完成身份、授权与撤销底座，再开放新登录方式。

术语：**专属邀请**（早期讨论稿称 EA）指管理方预建指定的本地账号，并发送专属链接让对方免密进入；**自助注册**（早期称 EB）指访客在分享入口自建本地账号；**DID 登录**指已有 DID 经其 canonical zone 联合登录；**Guest** 指无须账号的游客访问。对应账号称专属邀请账号、自助注册账号。PRD 与 UserType.md 已使用同一组名称，AC / Q 编号不变。

依据：

- [BuckyOS 混合 SSO PRD](<../doc/arch/verify-hub/BuckyOS 混合SSO PRD.md>)：2026-10-21 的产品要求、AC-01～24、Q-01～13。
- [Session Only Member 设计](<../doc/message_hub/Session Only Member 设计.md>)：保留身份源、最小授权、Session Guest、全入口鉴权和撤销约束；与新 PRD 冲突处按下文对齐。
- [SSO](../doc/sdk/SSO.md)、[Self-Host-Groupv2](../doc/message_hub/Self-Host-Groupv2.md) 及当前源码。历史 TODO 的勾选不作为本次实现证据。
- 跨 Zone 联合登录的已有设计：[SSO](../doc/sdk/SSO.md)「跨 Zone SSO（去中心 IdP）」、[BNS 去中心的名字系统](<../doc/arch/BNS 去中心的名字系统.md>) 场景 6。SSO-09 在其基础上补齐，见 §4.1 D-7。
- jump 入口的已有讨论：[BuckyOSApp #34 的端到端流程](https://github.com/buckyos/BuckyOSApp/issues/34#issuecomment-4812488915)、[SDK 降级草案](https://github.com/buckyos/BuckyOSApp/issues/34#issuecomment-4813812031)、[安全边界补充](https://github.com/buckyos/BuckyOSApp/issues/34#issuecomment-5086032249)、[无 SN 账号的 Current Zone Locator 补充](https://github.com/buckyos/BuckyOSApp/issues/34#issuecomment-5088867966)。历史样例使用 `jump.buckyos.ai`，SDK 草案的 `jumpBase` 为 `https://jump.buckyos.ai/dlgs/saveas`；正式默认入口已由用户于 2026-10-21 确认为 `https://jump.buckyos.org`，并允许系统配置覆盖，历史 `.ai` 仅作为来源记录。按较新的 Locator 补充理解匿名路由，不把早期“Current User”表述当成已认证身份。
- [《App 安装协议》2026-08-04 历史版本 §6.3](https://github.com/buckyos/buckyos/blob/7869f8189699698f9fecf5fcccea472b6dd8fecf/doc/App%20%E5%AE%89%E8%A3%85%E5%8D%8F%E8%AE%AE.md#L750)：以 `/sysdlg/app_installer` 演示同一 jump 机制。该文与 Issue 中的 `/dlgs/saveas` 均作入口样例，不视为当前已实现或已冻结的路由契约。

源码核对基线：BuckyOS `24c1e5b67` 及 2026-10-21 当前工作区，同时阅读同级 `buckyos-websdk`、`buckyos-base` 源码。用户类型前置的历史验证结果见其实施记录；本轮 review 只运行 WebSDK 的 `auth_client.node.test.ts`、`session_refresh.unit.test.ts`，2 个套件、6 项通过，未重跑 Rust 测试、完整构建或真实 Zone DV。以下缺陷来自静态调用链核对，未在在线 Zone 上执行越权复现。

阅读顺序：§0 是已定基础与前置修复，§2 是实现现状，§3 是开工顺序，**§4 集中列出用户决策：§4.1 为技术路线，§4.2 为产品参数与范围，均已确定**；§5～9 是按决策落实的工程任务。实施中若发现须改变 §4 承诺的取舍，先补回 §4 并经用户确认，再改任务和验收条件。

## 0. 前置完成情况与本轮 review

### 0.1 已定基础：不再重新讨论用户类型与 jump 默认域名

| 编号 | 已定内容 | 实现边界 |
| --- | --- | --- |
| D-1 | 专属邀请账号与自助注册账号为 Limited；未入驻本 Zone 的 DID 访客为 External，无账号记录；自助注册账号不自动入系统组 | `UserType` 已为 Root / Admin / User / Limited；组、覆盖选项、投影及 `external: true` 标记已落地，专属邀请、自助注册与 DID 登录的签发尚未实现 |
| D-2 | 两类本地账号沿用二级 DID 与 Zone 托管密钥；对外联合只允许 Admin、User，且仍须证明本人指定的 canonical zone | `can_federate_out` 判定已提供，联合签发路径未实现；普通本地建号仍自动设置 default zone，专属邀请、自助注册建号不可照搬 |
| D-3 | 匿名访问不是用户类型；Guest 沿用 AI Workspace `gt_` → `guest:` 资源主体，verify-hub 不签发 Guest 系统会话 | 现有匿名访问可复用；统一入口和身份切换未接通 |
| jump 默认入口 | `https://jump.buckyos.org`，允许系统配置覆盖 | 域名已定；配置键、读取方式、action 路由与 pending intent 是 SSO-02 / 11.1 的工程契约 |

用户类型与组的权威定义见 [UserType.md](../product/users_and_agents/UserType.md)，决策与交付证据见 [用户类型定义 TODO](user-type-definition-todo.md)。系统用户组是可被引用的标签，不生成系统 RBAC `p` 规则；Limited 的“零权限”指没有默认业务与资源权限，不排除已允许的自身 settings / profile / 通讯录操作。External v1 的资源服务范围包括 msg-center 群路径、AI Workspace、HomeStation，不能在统一闸口实施时漏掉 HomeStation。

### 0.2 源码事实与可复用能力

- **用户类型基础已交付。** Guest 枚举已删除，scheduler 未知类型不再回退 User；`user.create` 支持 User / Limited 和系统组，组管理与通讯录投影已实现。`validate_password_login` 已拒绝空的存储密码；独立凭据状态仍未定义。
- **主体工具不等于生命周期已接通。** `ControlPanelClient::resolve_user_principal` 已定义，但尚无调用方；它不提供不可变账号 ID→DID 绑定。External 标记已有，verify-hub 尚无相应签发、续期路径；现有 logout 按 family 撤销，可复用，不必另造账号依赖接口。
- **Message Hub 与 AI Workspace 同源。** 二者是 Desktop SPA 的 `/messagehub`、`/workspace/:workspaceId`，共用根域 `System{control-panel}` 会话；两面板切换不证明跨 origin SSO。`auth.prepare_app_launch` 的精确 App 实例换票、callback 路由绑定与 host-only Cookie 可复用。
- **共享验签入口尚无受限权限上限。** [runtime.rs](../src/kernel/buckyos-api/src/runtime.rs) 的 `verify_trusted_session_token` 校验签名和 principal / target / token use；`enforce` 使用当前 RBAC，不校验新来源、scope 或 family。现有 token 无权限快照，Limited 升级或加入获授权组后，旧会话可能直接获得新权限。“旧会话不自动增权”是待实现约束。
- **自行解析 token 的路径仍需审计。** system-config 自有 `enforce`、Control Panel 的 dashboard / pikg / zone_mgr、repo_service、HomeStation sign、Gateway `check_oauth` 不能仅靠改共享验签入口覆盖。
- **浏览器身份仍要求本地账号。** CP `/sso_refresh` 按 `sub` 查询 `users/<sub>/settings`，WebSDK `BrowserUserInfo` 要求 `user_id` 和 `user_type`，缺失时回退缓存身份；不能直接支持无账号的 External，也不能将该回退用于身份切换。
- **跨 Zone 证明与持久会话未实现。** 公开 ZoneDocument 的 `verify_hub_info` 已注释，公钥目前在内部 `boot/config`，普通 token 的 `iss` 为 `verify-hub`。refresh family 在内存，重启后不能继续刷新；现有 access token 并不会因缓存丢失而立即失效。
- **资源撤销还包含缓存边界。** 组修改正常会主动刷新 RBAC 和 msg-center，但后者 reload 失败只告警；失败后的周期补偿为 30 秒。HomeStation 另缓存联系人 30 秒，上游失败还会继续返回旧缓存。未来按组授权不能据此宣称“每请求实时”。
- **专属邀请的重发没有现成落点。** 本轮核对范围内没有可直接复用的系统 SMTP / SMS 发送实现。按 D-9，系统不内建发送，由签发方经原渠道发送；人工签发方可经 msg-center System Notify（[notify.rs](../src/frame/msg_center/src/notify.rs)）接收通知。
- **D-6 / D-7 的落点核对（决策 review 补充）。** verify-hub 的 kernel spec 期望实例数为 1，verify_hub crate 尚未引用 workspace 已有的 `rusqlite`。现有浏览器登录是 CP `auth.login` 调用 verify-hub `login_by_password`，再由 CP 建立 pending SSO（[sys_auth_backend.rs](../src/frame/control_panel/src/sys_auth_backend.rs)），`login_by_remote_verify_hub` 沿用这一模式。kRPC 的 `RPCErrors` 只有 `NoPermission` 等粗粒度变体且附带文本，D-4 原因码需要新的结构化字段。

### 0.3 前置修复：新登录签发前必须验证

这些修复落实已有身份与权限边界，不等待重新定义用户类型。用户类型 TODO 的完成记录保留；本表记录 review 后补出的缺口，不把历史测试通过当作这些路径已受保护。

| 编号 | 当前缺口与证据 | 必须补齐的行为与验收 |
| --- | --- | --- |
| PRE-01 / P1 | [scheduler 的 RBAC 生成](../src/kernel/scheduler/src/system_config_agent.rs) 给 User sudo 整份自身 settings / doc 写权限；[system-config](../src/kernel/sys_config_service/src/main.rs) 只校验 key 级权限 | 普通 User 即使 sudo 也不能经原始写接口修改 `type/groups` 或其它身份管理字段；保护覆盖 set、JSON path、事务、删除与重建，允许的改密 / 展示资料操作有明确字段边界 |
| PRE-02 / P1 | User / Limited 可写自身整份 profile；[msg-center 主体解析](../src/frame/msg_center/src/owner_session.rs) 直接信任其中 `profile.did`，CP 的 DID 不变检查可被原始配置写绕过 | 账号→DID 来自不可由本人任意修改的真相源；篡改 Profile 后不能冒充他人访问群、owner 数据或联系人授权；不只修 CP 一条入口 |
| PRE-03 / P1 | [AIWS RuntimeAuth](../src/frame/aiworkspace/server/src/auth.rs) 丢失身份类别；[授权](../src/frame/aiworkspace/store/src/workspace.rs) 的 `*` 匹配所有非 guest；[创建资格](../src/frame/aiworkspace/store/src/service.rs) 只排除 guest | Limited / External 不因“已登录”获得 `*`、创建、导入、复制或列出全体主体的能力；按可信身份类别和显式授权检查，并保留普通用户与公开 Guest 的既有行为 |
| PRE-04 / P2 | [user.change_password](../src/frame/control_panel/src/user_mgr.rs) 检查 `allow_password_change` 后仍以调用者 token 写 settings，Limited 没有该写权限 | Limited 开启覆盖选项后可安全改密，关闭时拒绝；不能以开放整份 settings 写入修复。专属邀请账号首设密码按 P-1（独立的专用能力）和 SSO-06 落实 |

- [ ] 完成 PRE-01～04 并增加对应允许 / 拒绝用例；旧会话不增权和撤销另在 SSO-02 / 04 / 05 验证。
- [ ] 将上述身份字段写保护与不可变绑定作为 SSO-01 / 04 的交付物，覆盖业务 RPC 与原始存储入口。

### 0.4 已完成项与可并行工作

- [x] 空存储密码拒绝密码登录（UT-04）；不再列作未修复漏洞。
- [x] scheduler 删除 Guest / 未知类型回退 User（UT-03）；不再重复实施。
- [ ] SSO-04 的逐方法入口清单和上述前置修复。
- [ ] SSO-01～03 的契约草案与 fixtures，取值按 §4 已定结果。
- [ ] 按 P-2 将 `user.invite.create` 限为 User / Limited（`handle_user_invite_create` 目前只拒绝 Root），补拒绝 Admin 的用例，并同步 [UserType.md](../product/users_and_agents/UserType.md)、[用户类型 TODO](user-type-definition-todo.md) T-7 细节中的“Admin 保持现状 / 可邀请 Admin”表述。
- [ ] 按 Q-03 删除 Desktop 登录页的 `remember_me`（[LoginPage.tsx](../src/frame/desktop/src/auth/LoginPage.tsx) 传给 `auth.login`，CP 未使用）。
- [ ] SSO-12 的 host 群时间线 adapter、附件 `context_path` 传递和按 Zone / 主体 / Session / 消息隔离缓存。用现有 Limited 测试账号验证；群发送仍须补最小资源授权及零个人邮箱断言，不能直接删掉 SENT 检查后放行。

## 1. 先对齐设计，避免从错误模型开始实现

| 问题 | 本轮实施约束 |
| --- | --- |
| 旧设计的 guest user 与 PRD Guest | PRD Guest 是无须注册正式账号的访客体验；旧文档 guest user 是有稳定主体、受限授权的系统用户。匿名请求、浏览器访客、外部本地账号（Limited）、未入驻的 DID 访客（External）必须可区分，不能都塞进一个 `guest` 字符串；定义见 [用户类型定义 TODO](user-type-definition-todo.md)。 |
| 专属邀请 / 自助注册与 Session Only Member | 两类账号都是目标 Zone 的 Local User；Session Guest 是某个 DID 在某个 Session 内的资源角色。专属邀请账号、自助注册账号和 DID 登录者都可以获得 Session Guest 角色，登录方式不决定其群成员身份。不能把所有专属邀请强制实现成 CRM guest 兑换。 |
| 专属邀请链接是否一次性 | 新 PRD 要求有效期内可重复免密进入。拆开长期入口、限时免密资格、短期单次 callback / exchange code。后者可一次消费，不能因首次访问消费掉整个专属邀请。旧设计一次兑换规则继续适用于其 App 登录兑换凭据。 |
| Local User 是否形成 canonical zone | 专属邀请、自助注册创建账号、设置密码、保存 Profile 都不能自动指定或变更 canonical zone。现有创建流程的 `set_default_zone_did` 不可照搬；本人明确建立身份归属是另一项操作。 |
| 系统 token 与资源权限 | 继续使用 `principal_kind=user` 和标准 `AuthTarget`；不新增 Message Hub 私有 token、`mhg_` 凭据或身份型 tunnel。认证成功不自动给邮箱、通讯录、群管理、任务、存储或 sudo 权限。 |
| App 身份源与当前客户端 | 身份源回答“谁证明此账号”；AuthTarget 回答“本次会话面向哪个 App 实例或系统服务”。两者分别校验。App 实例精确到 owner，不能只凭 AppId 授予代登录或转授权。 |
| DID 联合登录 | 只有用户本人唯一指定的 canonical zone 的 `verify-hub` 能在本路径代表该 DID。目标 Zone 的同名账号、本地 Profile、联系人合并和管理员签名都不能替代这条证明链。 |
| jump Locator 与 canonical/current zone | PRD 中 canonical zone 与 current zone 同义，表示唯一身份归属；历史 jump 讨论的 Current Zone Locator 只回答“本次导航去哪个 Zone”。手填 hostname、SN 提供的候选地址、浏览器偏好都不证明 DID 控制权，不修改 OwnerDocument / `default_zone`，也不产生登录态或资源 grant。界面称“当前目标 Zone”，不称“已登录用户”。 |
| 固定资源入口与“到自己的 Zone 执行”入口 | 专属邀请、Workspace、Session 分享已有明确资源所属 Zone，登录后仍回该资源；不能按 jump 偏好重写为另一个 Zone。jump 用于第三方网页尚不知道用户目标 Zone 的保存、安装等调用；已有直达入口不强制经过 jump 或 SN。 |
| 首期范围 | PRD 核心闭环包含专属邀请、自助注册、DID 登录、允许时的 Guest、同 Zone 跨应用。可分阶段交付，但不能把只完成专属邀请或单应用演示称为混合 SSO 完成。钱包、复杂多身份工作台、历史归并、canonical zone 迁移产品流程后续独立做。 |

后续任务中的字段、接口名除已存在者、`login_by_app_access_token` 和 D-7 的 `login_by_remote_verify_hub` 外，均为待冻结的技术契约，不表示 PRD 已指定实现。需要用户拍板的事项集中在 §4；其余工程契约由实现者给出可评审结果，不能在代码常量中隐含选择未定方案。

## 2. 当前实现：哪些复用，哪些必须改

| 模块 | 当前源码事实 | 实施落点 |
| --- | --- | --- |
| 标准认证目标 | [app_identity.rs](../src/kernel/buckyos-api/src/app_identity.rs) 已有 `AuthTarget::{App,System}`；[app_availability.rs](../src/kernel/buckyos-api/src/app_availability.rs) 校验 principal、target、token use、实例和 owner。 | 扩展可信主体和会话上下文，复用这些约束，不重建 target 模型。 |
| Verify Hub | [main.rs](../src/kernel/verify_hub/src/main.rs) 已有 password / JWT assertion / sudo / refresh / logout、token pair、内存 family 和 refresh 并发处理；没有混合登录入口。`login_by_jwt` 明确拒绝 Verify Hub session 作为 LoginAssertion。 | 按 D-7 新增目标侧 `login_by_remote_verify_hub` 与 canonical 侧签发 / 兑换 code，不放开现有 `login_by_jwt` 接受远端 session；按 D-6 把 family 移入本地 SQLite。 |
| 同 Zone 换票 | [sys_auth_backend.rs](../src/frame/control_panel/src/sys_auth_backend.rs) 的 `auth.prepare_app_launch` 已调用 `delegate_user_session`，从 Control Panel 普通用户会话换到精确 App 实例；callback 绑定路由、origin、完整 redirect，使用 host-only Cookie。 | 复用换票和 callback，不共享父域 Cookie；现有入口仅覆盖 CP→App，不等于外部受限身份和任意分享入口已接通。 |
| 撤销 | Verify Hub refresh / delegate 会查 family；[runtime.rs](../src/kernel/buckyos-api/src/runtime.rs) 的 `verify_trusted_session_token` 主要验签及校验 claims，`enforce` 加 RBAC，但无统一来源/scope/family 状态校验。Gateway 页面鉴权也不能仅靠 JWT 判断实时撤销。 | 按 D-5：资源撤销在下一次受保护请求生效；账号 / 来源 / logout 的旧 access 在 TTL 内残留，refresh / delegate / callback 重查。删除 refresh 记录不等于旧 access 已失效。 |
| 用户创建和邀请 | [user_mgr.rs](../src/frame/control_panel/src/user_mgr.rs) 的 `user.create` 要管理员和密码，支持 User / Limited 与显式系统组，仍生成密钥及设有 default zone 的 OwnerDocument。`user.invite.accept` 接收已绑定目标 Zone 的 OwnerDocument，不签发登录态；`user.invite.create` 目前可邀请 Admin，按 P-2 收紧为 User / Limited（§0.4）。 | 复用创建事务与校验，新增凭据状态和不可变身份边界；专属邀请、自助注册账号不自动指定 canonical zone，公开接受邀请须验证身份控制权与原子消费。 |
| 用户类型、组与主体 | [control_panel.rs](../src/kernel/buckyos-api/src/control_panel.rs) 已为 Root / Admin / User / Limited，集中判定、系统组、投影、External bool claim 已提供；匿名 availability 保留 `guest` 伪组。共享 `resolve_user_principal` 尚未接入消费方。 | 复用前置类型，补账号→DID 可信绑定和实际消费者，不重做用户类型；不能把 bool claim 当作签发、授权与续期已经可用。 |
| Message Hub 群角色 | [group_service.rs](../src/frame/msg_center/src/group_service.rs) 已实现 `invite_session_guest`、`accept_session_invitation`、`submit_guest_request`、会话 ACL / 历史区间 / Hook。 | 复用 Session Guest 状态机；缺统一外部身份登录、来源范围检查及无需个人邮箱的访问闭环。 |
| Message Hub 发送与附件 | [msg_center.rs](../src/frame/msg_center/src/msg_center.rs) 已为 External 向本机托管群发送跳过 SENT 检查；Limited 仍先要求 SENT。后端已有群附件引用上下文校验，普通对象入口仍有全局 objects RBAC；前端 [objects.ts](../src/frame/desktop/src/app/messagehub/api/objects.ts) 不带 `context_path`，缓存仅按 ObjectId。 | 补 Limited 的最小群发送授权与受限对象入口，前端传上下文；不得授予整份邮箱 / objects 权限作为绕过。 |
| Message Hub 页面 | [MessageHubView.tsx](../src/frame/desktop/src/app/messagehub/MessageHubView.tsx) 及 datamodel 面向正常桌面用户，依赖个人会话索引、邮箱和联系人；已有 Session Guest 管理 UI。 | 新增受限分享入口和 host 群数据层，复用对话组件，不直接加载完整桌面数据。 |
| AI Workspace Guest / 分享 | [transport.ts](../src/frame/desktop/src/app/aiworkspace/api/transport.ts) 已保存 `gt_` 访客 token；服务端 `Caller::guest` 形成 `guest:` 主体，已有公开分享。它不是统一 BuckyOS Guest SSO。 | 迁入统一身份边界，同时保留无注册的公开参与体验；旧访客历史如何处理需显式决策，不能自动绑定 DID。 |
| AI Workspace 权限 | [access.rs](../src/frame/aiworkspace/core/src/access.rs) 支持实体子树能力、祖先 grant 并集，无 deny。当前 RuntimeAuth 只取 sub / target，`*` 与创建资格仅区分 guest；没有系统组或 owner 联系人分组 grant 消费者。分享 UI 的 public read+comment 面向整个 workspace，surface 只是导航。 | 先完成 PRE-03，再扩展统一 Caller、组授权与 DJ 操作矩阵；surface 导航不能冒充局部授权。 |
| SDK 与浏览器身份 | 同级 [runtime.ts](../../buckyos-websdk/src/runtime.ts) 已解析 External 标记，但 `BrowserUserInfo` 仍强制本地账号字段，refresh 缺资料时回退旧缓存；CP 同样依赖账号记录。 | 定义 Account / External / Anonymous 响应与缓存绑定，区分展示资料和可信身份；接通刷新与身份切换后再接新入口。 |
| 通讯录授权与 HomeStation | `contact.check_membership` 已提供精确 DID + owner + tag 的服务间查询，AIWS 服务有查询权限；HomeStation 仍按本地 Profile 解析会话 DID，未接通 External token，并有联系人缓存。 | 复用窄接口，冻结授权对象和 freshness；接通 HomeStation External 路径，不能把现有 reader proof 能力当作本地 External 会话支持。 |

## 3. 交付顺序与门槛

| 阶段 | 任务 | 依赖与完成门槛 |
| --- | --- | --- |
| 前置修复 | PRE-01～04，可与 A 并行 | 封住身份字段写绕过、AIWS 资格缺口并修通 Limited 改密；对应负例通过后才开放新受限签发。 |
| A：冻结共用模型 | SSO-01～03 | §4 已全部确定（S-1：SSO-10 移出本批）；交付 §5.0 的契约与 wire / 状态机 fixtures，时长、TTL 等取 §4.2 的值。 |
| B：先完成权限底座 | SSO-04～05 | PRE-01～04、逐方法默认拒绝、旧会话不增权、来源保留，以及按 D-5 实现、按 Q-03 / Q-04 时限验收的撤销矩阵全部通过后，才开放新登录签发。开发期用 fixture，不提前对外开放。 |
| C：系统身份接入 | SSO-06～09；SSO-10 按 S-1 移出本批 | 专属邀请、自助注册、Guest、DID 登录共用 B 的底座。 |
| D：页面与资源闭环 | SSO-11～13 | 可提前做接口适配和页面；真实外部访问必须等 B 通过，并与相应登录入口一起验收。SSO-11.1 补齐 jump 定位目标、在目标 Zone 登录并恢复原操作的衔接。 |
| E：联调与交付 | SSO-14～15 | 两组 PRD 场景、旧 SSO 回归、负例和真实跨 Zone DV 都通过，才宣布核心闭环完成。 |

App access token 是旧 Session Only 设计保留的扩展任务，不是专属邀请、自助注册或 DID 登录的同义词。按 S-1 不在本批，以后可在共用底座上独立交付，不要求普通客户先具备 CRM 账号。

## 4. 用户决策

本章是用户决策的唯一清单。D-1～D-3 和 jump 默认域名见 §0.1。§4.1 的 D-4～D-9 已由用户于 2026-10-21 确认，均采纳原推荐，D-4、D-7 另有补充；后文任务与验收已按此改写，不再保留备选分支。§4.2 的产品参数与范围也已确认，Q-03、Q-04 的具体数值按用户授权由 review 给出。

### 4.1 已定技术路线：D-4～D-9

| 编号 | 已定内容 | 未采纳的备选 | 落实到 |
| --- | --- | --- | --- |
| D-4 | 受限会话在共享验签 / 授权上下文默认拒绝，服务逐方法开放；根域沿用 `System{control-panel}`。用户补充：拒绝提示要清楚，便于管理员调整配置 | 独立 System target | SSO-02 / 04 / 11 / 14 |
| D-5 | 资源撤销在下一次受保护请求生效；账号 / 来源 / logout 允许旧 access 在 TTL 内残留，refresh（含 grace）、delegate、callback 重查 | 所有撤销都在下一请求生效 | SSO-05 / 14；时限见 Q-03 / Q-04 |
| D-6 | verify-hub 本地 SQLite 持久化 family，单实例部署，写明状态随迁与恢复要求 | system-config 共享状态 + CAS | SSO-03 / 05 |
| D-7 | 后端 HTTPS 一次性授权码交换，不改公开 ZoneDocument；目标 Zone 的签发入口为 `verify-hub.login_by_remote_verify_hub` | 公布 verify-hub 公钥，离线验签 | SSO-02 / 09 |
| D-8 | 入口归资源服务；有效认证方式 = 入口允许 ∩ Zone 开关；Zone 开关只管自助注册 / DID 登录，Guest 由资源 owner 在入口上控制；Q-06 随之关闭 | CP 集中持有入口；Zone 可全局关闭 Guest | SSO-03 / 07 / 08 / 09 / 11 / 13 |
| D-9 | 系统管理专属邀请资格与可靠通知，签发方经原绑定渠道发送；系统不内建 SMTP / SMS | 系统内建 SMTP，短信另行集成 | SSO-02 / 03 / 06，AC-05 / 06 |

**D-4：默认拒绝，拒绝结果可诊断**

- 受限标记与批准上限写入签名 claims，覆盖所有 Limited（含已有本地 Limited 与专属邀请、自助注册账号）、External 和受限来源衍生会话；具体方法由 SSO-04 审计表冻结。类型升级不让旧会话增权；类型降低后的残留窗口同 D-5。根域受限会话与管理员桌面的 target 同为 `System{control-panel}`，二者靠 claims 区分，不靠 target。
- 开放清单分三类：verify-hub 的必要认证 / 续期 / 登出 / 首设密码；CP 的入口编排和 SSO；资源服务按方法授权的 msg-center 群、AI Workspace、HomeStation。External 不得调用 CP 用户管理等业务方法。
- Limited 已允许的自身 settings 读取、Profile 编辑、通讯录读取 / 按覆盖选项编辑，以及获准 App 的访问保留明确路径，不被“受限会话一律拒绝”误伤；可编辑资料不含可信身份字段。
- Message Hub↔AIWS 按同源测试，第三方 App origin 按精确实例、owner、来源和 scope 换票测试。
- **拒绝原因码（用户补充）。** 每次拒绝在 wire 上携带原因码，Rust / TS 两端按枚举分支，不解析错误文本。至少区分：此类会话不开放该方法；超出会话来源 / 上限；缺少资源授权；入口关闭、过期或未允许该方式；Zone 未开启自助注册 / DID 登录；账号或凭据状态；来源已撤销；授权状态无法确认。
- **每个原因码登记由谁、在哪里调整。** 方法未开放属于产品边界，提示写明“此类账号不支持该功能”，不引导管理员寻找并不存在的开关。系统组、资源授权、入口方式、Zone 开关、覆盖选项等可调项，提示指出对应的 CP 页面或分享面板；分享面板中被 Zone 开关关闭的方式显示为“已被 Zone 设置关闭”。
- **访客侧不泄露。** 访客只看到类别和“联系分享者 / 管理员”；对其未开放的资源与不存在的资源返回同一结果，不暴露资源是否存在或他人信息。服务日志记录结构化拒绝（主体、类别、服务 / 方法、缺失条件、可调整项），不记录 token 与 secret。

**D-5：撤销的可观察承诺**

- 资源类撤销（移出 / 封禁 Session、grant 撤销、资源删除，以及作为授权依据的移组 / 删除投影）提交后，在下一次受保护请求生效。RPC / HTTP 读取、附件读取，以及已建立订阅 / 流的下一次投递都要复核；无法确认授权时拒绝。msg-center 组 reload 失败只告警并依赖 30 秒补偿，HomeStation 联系人缓存 30 秒且上游失败时返回旧缓存，二者都不满足此承诺，须改为版本化、主动失效或无法确认即拒绝。
- 账号停用、来源撤销、logout：旧 access 至多残留到其 TTL；refresh（含 grace）、delegate、callback 必须重查。清 Cookie 不等于撤销服务端 family。持续连接在 access 到期前未续上即断开（Q-03），连接一直保持也不能无限延长授权。
- 时限按 Q-03：access 15 分钟，即账号 / 来源 / logout 的最大残留窗口。canonical zone logout / 改密不传播到目标 Zone，联合会话最长 30 天（Q-04）；普通 CP→App 独立 family 与依赖来源的衍生 family 分开定义。

**D-6：family 持久化与单实例限制**

- verify-hub 引用 workspace 已有依赖 `rusqlite`（[src/Cargo.toml](../src/Cargo.toml)；verify_hub crate 当前尚未引用），保存 family、当前 / 上一枚 jti、签发与到期信息、撤销状态、来源 / 父关系和精确 target，不存 token 明文。字段由 SSO-03 冻结，须足以保留 refresh 轮换与 grace 幂等语义。
- scheduler 的 `add_verify_hub` 以期望实例数 1 生成 kernel spec（[system_config_builder.rs](../src/kernel/scheduler/src/system_config_builder.rs)），与单实例一致。数据目录按服务数据目录约定，即 `get_buckyos_service_data_dir` 对应的 `data/var/verify-hub/`，task-manager、msg-center 也这样做；具体路径由 SSO-03 冻结。
- 单实例不等于状态可以丢失：重启后的并发重试、grace 幂等、备份恢复和撤销状态不回滚都要满足。
- 已知限制：该目录不在 OOD 之间同步。多 OOD 下 verify-hub 被调度到另一节点时，family 不随之迁移，现有会话须重新登录。交付记录写明这一限制，不宣称多副本或故障切换可用；将来要求多 OOD 无粘性路由时，再就未采纳的共享状态方案另行决策。

**D-7：联合认证的信任链与入口**

- 目标 Zone 经可信解析取得 `user_did → OwnerDocument → 唯一 canonical zone → 可信 ZoneDocument hostname`。本人授权与 owner key 必须可验证；本地 Profile、手填 hostname、自报公钥都不是归属证明。公开 ZoneDocument 的 `verify_hub_info` 保持现状，本方案不恢复该字段。
- 用户对入口的理解成立：目标 Zone 签发会话的入口是 `verify-hub.login_by_remote_verify_hub`，命名与现有 `login_by_password`、`login_by_jwt` 一致。但它不是整条链的唯一方法，canonical 侧还需要“签发 code”“兑换 code”两个方法，名称与 wire 由 SSO-02 / 09 冻结。完整流程：
  1. 目标 CP 建立 pending，记录 DID、目标 AuthTarget、state / nonce 和经验证的回跳地址，再把浏览器导向 canonical CP。
  2. 用户在 canonical 处于登录状态；首次按 DID + 目标 Zone 显示 consent，资料范围见 Q-05。同意后 canonical CP 请 canonical verify-hub 签发 code。只为 Active 的 Admin / User 且有本人 canonical 归属证明者签发；仅改 UserType 不产生归属授权。
  3. 浏览器回到目标 CP callback。CP 复核 pending 后调用 `login_by_remote_verify_hub`，与现有 `auth.login` → `login_by_password` 是同一模式（[sys_auth_backend.rs](../src/frame/control_panel/src/sys_auth_backend.rs)）。浏览器不直接调用该方法，也不能提供 hostname、key 或兑换地址。
  4. `login_by_remote_verify_hub` 自行从 DID 解析 canonical hostname，经 HTTPS 调用 canonical 的兑换方法；校验响应中的 DID、audience（目标 Zone）、nonce、用途与资料范围后，映射到已入驻账号或签发 External token pair，交回 CP 沿用现有 pending SSO 写 Cookie。
- code 短期、单次使用，绑定 DID、来源 / 目标 Zone、用途、nonce、期限与资料范围；兑换方鉴权、重试和失败 wire 由 SSO-02 / 09 冻结。
- HTTPS 主机只来自上述信任链，远端 key 不加入本地普通 `verify-hub` 信任槽。现有 `login_by_jwt` 继续拒绝 verify-hub session，不用它代替新入口。开发 HTTP 只能显式开启，不能替代三 Zone HTTPS 验收。
- 该 DID 未入驻目标 Zone 时，以原 DID 签发 External 会话，不建 Profile / 密码。目标 Zone 续期时不向 canonical 复核，缺失、冲突、归属或 key 变化在至迟 30 天后的重新联合时发现并拒绝（Q-04），不静默改用其它来源。

**D-8：入口与资源授权的所有权（含 Q-06）**

- AIWS 入口与 grants 同库存放，Message Hub 入口由 msg-center 持有；入口 ID 定位资源，免密 secret 独立保存。CP 只查询可公开的入口描述并编排登录，不另建成员资格真相源。
- 入口至少表达资源引用、创建者、允许的认证方式、权限模板、期限与状态。只有对该资源有分享 / 转授权权限的人能创建入口；持有读取权或能创建专属邀请账号，都不等于能转授资源。App 代建入口时只是创建者，受同一权限约束，不构成另一层策略。
- 有效认证方式 = 入口允许 ∩ Zone 开关。Zone 只提供自助注册、DID 登录两个总开关，没有全局关闭 Guest 的开关；Guest 由资源 owner 在入口上开启或关闭。资源 owner 不能绕过 Zone 关闭项；开放自助注册不自动加入系统组；公开描述不含私密正文或客户名单。
- 已知风险：管理员无法一键关闭全 Zone 的匿名访问。Guest 只有资源级 `guest:` 主体，没有系统会话，影响仅限各 owner 自行开放的资源；按“先能力后安全”记为已知风险。两个 Zone 开关默认开启（§4.2 D-8a）。

**D-9：发送责任与验收边界**

- 首次邀请时登记可信签发方及固定的原渠道；重发由系统生成新资格，并可靠通知签发方发送。用户页面只返回已提交、受限或失败，不返回新 secret，也不允许改投其它渠道。系统不内建 SMTP / SMS。
- 通知方式属于工程契约，由 SSO-02 / 03 冻结。建议：人工签发方（管理员经 CP 创建专属邀请）复用 msg-center System Notify（[notify.rs](../src/frame/msg_center/src/notify.rs) 的 `notify.*`，可续发、各步骤幂等）；通知不带新 secret，签发方到 CP 的专属邀请管理处经鉴权取得一次性显示的新链接再发出，避免凭据进入消息存储。App 签发方（如 CRM）通过 outbox 和登记的回调取得新资格。
- 实施 SSO-06 前需确定可用签发方与测试短信 / 邮箱。验收包含通知与实际送达：签发方收到通知、取得新资格并经原渠道送到测试收件端，客户用新链接进入；mock 或页面直接返回测试链接都不能代替。

### 4.2 已定产品参数与范围

用户于 2026-10-21 确认以下各项。原推荐只列约束、没有给出数值的 Q-03、Q-04，按用户“按你推荐”的授权由本轮 review 补上数值，均以现有代码常量为基准。已有硬边界仍然有效：专属邀请的免密资格在有效期内可重复使用；入口寿命与免密资格分离；设置密码使旧免密资格失效；登录不自动扩权、合并身份或更改 canonical zone。

| 编号 | 已定内容 | 落实到 |
| --- | --- | --- |
| S-1 | SSO-10 App access token 移出本批，保留为独立扩展任务；专属身份域的登记 / 绑定一并后排，共用的来源与上限模型保留 | §3 阶段 C、SSO-01 / 10 |
| Q-01 | 专属邀请免密资格默认 48 小时。用户只给出默认值，v1 不提供按 Zone、业务或单个邀请调整，48 小时即上限；是否开放调整、由谁调整，留待以后另定 | SSO-06 |
| Q-02 | 重发只替换同一入口的免密资格，同账号的其它独立入口不受影响；已被密码或撤销替代的资格不能恢复 | SSO-06 |
| Q-03 | 会话时长见下表 | SSO-02 / 05 / 06 / 11，阶段 B 验收 |
| Q-04 | 首期不承诺全局 logout。目标 Zone 续期时不向 canonical 复核；联合会话最长独立有效 30 天，到期重新走联合登录；canonical 不可用只影响新登录和重新联合。canonical 的 logout / 改密不传播到目标 Zone，按“先能力后安全”记为已知风险，最长影响 30 天 | SSO-05 / 09 |
| Q-05 | consent 展示并共享 DID、来源与目标 Zone、公开显示名 / 头像，不共享私密联系方式。consent 记在 canonical 侧，以 DID + 目标 Zone 为键；资料范围扩大、用户撤回或 canonical zone 变化时重新确认 | SSO-09 |
| D-8a | Zone 的自助注册、DID 登录两个开关默认开启 | SSO-03 / 07 / 09 |
| Q-10 | 同 Zone 同客户共用一个账号，这正是稳定二级 DID 的用途；每个 Session / 入口各自持有邀请和 grant，一个链接不开放其它工单。复用由签发方显式指定已有账号，系统不按手机号 / 邮箱自动归并；账号已设密码时，新工单链接只作入口，用密码登录 | SSO-06 / 12 |
| P-1 | 首设密码是独立的专用能力，不受“没有旧密码 / 默认禁止常规改密”阻塞；之后按单用户 `allow_password_change`，默认不自动改为 true | PRE-04、SSO-01 / 06 / 07 |
| P-2 | `user.invite.create` 只邀请 User / Limited，Admin 经 `user.change_type` 提升。源码 `handle_user_invite_create`（[user_mgr.rs](../src/frame/control_panel/src/user_mgr.rs)）目前只拒绝 Root，按 §0.4 修改，并同步 UserType.md 与用户类型 TODO 的表述 | §0.4 |

**Q-03 会话时长**

| 项 | 取值 | 说明 |
| --- | --- | --- |
| access TTL | 15 分钟，所有用户会话一致 | 沿用 verify-hub `SESSION_TOKEN_EXPIRE_SECONDS`；也是 D-5 中账号 / 来源 / logout 撤销的最大残留窗口 |
| sudo access | 3 分钟 | 沿用 `SUDO_SESSION_TOKEN_EXPIRE_SECONDS` |
| refresh 滑动期限 | 7 天，每次刷新顺延 | 沿用 `REFRESH_TOKEN_EXPIRE_SECONDS`；7 天未使用即需重新登录，不另设闲置期限 |
| 绝对最长登录时长 | 本地会话（密码、专属邀请免密）不设；DID 联合会话 30 天 | 本地会话满足 PRD“尽量长期复用”；联合会话到期重新联合，已有 consent 时通常只是一次跳转 |
| refresh grace | 10 秒 | 沿用 `REFRESH_GRACE_SECONDS`；grace 分支同样复核（SSO-05） |
| 持续连接 | 连接所用会话的 access 到期前未续上有效 access 即断开；资源撤销按 D-5 在每次投递复核 | 连接最长 15 分钟不复核身份 |
| 需要重新认证的事件 | sudo 类操作沿用 `sudo_by_password`；类型 / 组变化后要获得新权限须重新登录（旧会话不增权）；身份切换；consent 资料范围扩大 | 降权按 D-5 在 access TTL 内生效 |
| 首设密码后的其它终端 | 保留当前终端；撤销该账号经专属邀请免密资格建立的其它 family，旧 access 至多残留 15 分钟 | 避免被转发的链接在设密后继续可用 |
| 专属邀请免密资格自然到期 | 已有会话不强退 | 到期只限制新的免密登录 |
| `remember_me` | 删除 | Desktop 登录页传了该参数，CP 并未使用；所有会话统一 7 天滑动，不保留无效选项 |

### 4.3 可后排事项与工程师负责冻结的内容

- Q-07 / 08 / 09 / 11 / 12（匿名历史归并、本地账号绑定 DID、忘密恢复、canonical zone 迁移、无归属钱包身份）保留独立设计；本批不自动合并、不用旧专属邀请恢复已设密码账号、不在普通登录中变更归属、不展示未实现钱包入口。新增这些产品流程时再确认范围，不阻塞当前核心闭环。
- 身份冲突不得静默覆盖 Cookie，撤销或重发不得复活已取消权限，认证后须恢复原业务上下文；这些作为本批异常路径验收，不能用 PRD“补充建议”绕开基本身份边界。
- Q-13 已由 PRD 交给技术设计：系统管理认证与会话，CP 编排浏览器流程，资源方持有资格 / grants，SDK 和 jump 承接调用与导航。工程师按此冻结具体组件职责、API、Origin / CSRF 检查责任，不另要求用户选方法名或字段名。
- §5 的统一身份响应、凭据状态机、claims / 存储 schema、授权对象、邀请版本、outbox wire、幂等 / CAS、浏览器 pending、jump 配置键与 action 路由均为工程交付物。若设计发现改变本章产品承诺的取舍，再补到本章；常规实现细节由工程师完成。

## 5. A：共享模型、协议与状态

### 5.0 阶段 A 必须交付的工程契约

本节不是新增用户决策；实现者依据 §4 已定结果完成设计，并把尚未决定的分支标清。各契约保存在本文附录或关联设计文档中，必须有实际字段 / 示例 / 状态转换，不能只复述任务名称。

| 契约 | 至少包含的内容 | 对应任务 |
| --- | --- | --- |
| 可信主体与当前身份 | Account / External / Anonymous 的 Rust / TS 共用 fixture；sub、account_id、DID、类型可空性、目标 / canonical Zone、认证方式与权限上限；不可变绑定、删除重建不复用、External 入驻后的映射；用户展示资料不决定授权主体 | SSO-01 / 02 / 11 |
| 凭据状态机 | 未设 / 已设密码、常规改密覆盖、专属邀请资格与版本（默认 48 小时）；首次设密、并发兑换 / 重发 / 撤销的提交顺序；原账号和资源关系不变；首设密码后撤销其它经专属邀请免密建立的 family（Q-03） | PRE-04、SSO-01 / 03 / 06 |
| 会话与撤销矩阵 | 升级 / 降级、入组 / 移组、External 入驻、设密 / 改密、logout、来源撤销、资源撤销、归属变化；逐项定义 access、refresh（含 grace）、callback、父子 family、流与缓存的结果、最大延迟和故障拒绝行为 | SSO-02 / 04 / 05 |
| 授权对象与资源邀请 | 本地账号 / DID、系统组 `group:<id>`、owner DID + 联系人 tag 的结构；public / `*` 的身份匹配表；Session invitation ID / version / expiry、重邀与幂等；入口与 grant 分开 | SSO-02 / 03 / 12 / 13 |
| 浏览器与外部 intent | CP / verify-hub / SDK / Gateway / jump / 资源方责任；入口 describe / prepare 与结构化 status；预期旧 / 新身份、state / nonce、取消、Origin / CSRF、回调复核、多标签页；jump 配置归属 / 键 / 读取与 action 参数 / 路由表 | SSO-02 / 11 / 11.1 |
| 拒绝原因码（D-4） | Rust / TS 共用的原因码枚举及 wire 字段；每个码的调整方（产品边界 / 管理员 / 资源 owner / 分享者 / 本人重新认证）与 CP 落点；访客侧合并后的结果和日志字段 | SSO-02 / 04 / 11 |
| DID 联合（D-7） | `login_by_remote_verify_hub` 与 canonical 侧签发 / 兑换 code 的请求、响应、错误码与重试；code 绑定字段、兑换方鉴权、audience 校验；目标 CP pending 字段 | SSO-02 / 09 |
| 持久状态与编排 | 每份记录的 owner、唯一键、CAS / 事务、期限与删除；verify-hub SQLite 的 family 表、数据目录、重启与 grace 幂等、迁移 / 备份（D-6）；Zone 自助注册 / DID 登录开关的键与默认值（D-8 / D-8a）；建号→邀请→通知签发方的 outbox 重试补偿与送达记录（D-9） | SSO-03 / 05 / 06 |


### SSO-01 / P0：分开主体、登录凭据与身份归属

- [ ] 定义可信主体记录：稳定且不可复用的内部 ID、业务 DID / subject、账号类别、状态、凭据状态、认证来源；本地 Profile 只属于有账号记录者。冻结 token `sub`、账号 ID、DID 的转换、删除重建与 External 入驻的映射，接入共享解析器。身份绑定不能读取本人可改的 Profile，不能靠昵称或拼接 `did:bns:<username>` 推断；完成 PRE-01 / 02 的字段写保护。
- [ ] 专属邀请、自助注册账号类型为 `Limited`（用户类型 TODO T-1）：零权限底座，权限经管理员维护的系统用户组或资源授权获得；自助注册不自动加入系统用户组（T-3）。未入驻本 Zone 的 DID 访客是 External，不建账号记录（T-4）；匿名访问不是用户类型（T-9）。
- [ ] “未设置密码”是独立凭据状态，password / sudo 入口在验证密码前显式拒绝该状态。当前 `validate_password_login` 已拒绝空 stored password，但尚无独立凭据状态；不能用可预测占位密码代替该状态。DID-only 不得意外走密码登录，Guest 不进入账号登录路径。
- [ ] 普通本地账户创建与“本人建立 canonical zone”拆开；两类账号的注册、首设密码及后续改类型均不得隐式写入归属授权。两类账号沿用二级 DID 与 Zone 托管密钥（用户类型 TODO T-6）；verify-hub 只为 Admin、User 签发对外联合断言，因此目标 Zone 不能借这些 Limited 账号代表用户登录第三方。不要求这些用户交出身份根私钥。
- [ ] 冻结本批身份来源范围：专属邀请的可信签发方、自助注册的本地凭据、DID 登录的 canonical issuer 均有稳定关联。按 S-1，App 身份域和 `(identity_domain_id, external_account_id) → subject` 唯一绑定随 SSO-10 后排，不作为本批交付门槛；共用来源与上限模型仍须保留，卸载重装不得继承旧来源权力。
- [ ] 手机号/邮箱是可变绑定，不进入主体主键；同名、同手机号或私有 Contact canonical 合并不能作为登录依据。已有普通用户/DID 的关联必须验证控制权，不准借 App guest 入口冒认。
- [ ] 用户类型、系统用户组、scheduler 映射与 RBAC 生成、主体解析函数由用户类型 TODO 的 UT-01～UT-05 完成；本任务在这些基础上补可信身份绑定、字段写保护、凭据状态与实际消费者接入；函数已定义不等于生命周期与权限上限已完成。

落点：`buckyos-api/{control_panel,app_availability,runtime}.rs`、`control_panel/user_mgr.rs`、scheduler 的用户加载和 RBAC 构建、WebSDK 用户类型。验收：专属邀请账号设密码、刷新、换设备均不换主体；两类本地账号与 External 登录均不产生 canonical zone 修改。

### SSO-02 / P0：统一会话上下文和生命周期协议

- [ ] 在现有 `AuthTarget`、`TokenPair`、`ValidatedTokenClaims` 上定义可验证的认证方式、来源身份域/issuer、认证强度、family/session 标识、批准的权限上限、授权来源关联和撤销版本。明确哪些在签名 claims，哪些通过服务端记录查询。
- [ ] principal 继续为 `user`，受限身份不得伪装成 system/app。External 标记已由用户类型 UT-01 实施为布尔 claim `external: true`（`TOKEN_EXTERNAL_CLAIM`、`bind_token_external`、`token_is_external`，主体解析 `ControlPanelClient::resolve_user_principal`）；verify-hub 尚无签发与续期路径。App target 精确绑定实例和 owner；System target 单独批准。来源 App 与 target 分别记录，refresh token 不能作为业务 session，sudo 不能从这些入口顺带获得。
- [ ] 给出 Account / External / Anonymous 当前身份响应及 Rust / TS fixtures，替换 CP refresh 的强制账号查询和 SDK 的账号必填假设；缓存按 Zone + 主体绑定，身份变化不能回退为旧 `user_info`。
- [ ] 会话 scope 是上限，不是权限真相源。实际请求始终求交：当前主体授权 ∩ 当前 target 能力 ∩ 来源/会话上限 ∩ 当前资源规则。新增加的资源授权不能让旧的 App 代登录会话自动越过上限；类型或组变化也不能通过动态 RBAC 自动提高旧会话上限，续期与换票必须保留该约束。
- [ ] 列出类型化协议：入口查询/准备、各方式认证或交换、专属邀请首次设密码/重发、DID authorize/return（按 D-7：目标侧 `login_by_remote_verify_hub`，canonical 侧签发 / 兑换 code）、当前身份、refresh/logout、来源和会话撤销。复用现有接口可满足的部分；每个新方法同时定义 caller、target、资源、origin、幂等键和预期失败状态。
- [ ] 为 SSO-11.1 冻结外部调用 intent：action 与允许的参数、路由候选、最终业务目标及登录后恢复上下文分别建模；明确 SDK / App / jump / CP 各自处理哪一段。复用现有 pending/回调机制，action→目标页面的映射必须登记，不能把任意 URL 当作登录成功后的可执行命令。
- [ ] 在 `verify_hub_client.rs` 的 request、trait、adapter、Rust client 与 WebSDK 同步协议。现有 kRPC 错误经传输会变为字符串，业务需要分支的结果使用结构化 status，不让 UI 解析错误文本。
- [ ] 区分可重试错误、未认证、无权限、免密资格过期/已撤销、密码已设置、资源关闭/删除、身份冲突和来源不可用；无权限再按 D-4 细分原因码，并登记每个码的调整方与位置。公开入口的返回信息不泄露其他客户或私密资源，访客侧按 D-4 合并“未开放”与“不存在”。

验收：同一 wire fixture 在 Rust / TypeScript 解析一致；来源、scope、target 缺失或矛盾时拒绝，不能回退为无范围的旧会话。开发阶段直接迁移协议，不新增旧格式兼容旁路。

### SSO-03 / P0：明确状态的真相源和事务边界

- [ ] 为账户/绑定、身份域授权、分享入口、专属邀请免密资格、资源 grant、DID consent、会话 family/revocation、发送 outbox 和审计分别列出存储 owner、唯一键、版本/CAS、到期与删除规则。已定 owner：分享入口在 AIWS（与 grants 同库）和 msg-center（D-8），Zone 自助注册 / DID 登录开关在 system-config，family 在 verify-hub 本地 SQLite（D-6）。其余优先复用现有 system-config 事务及已有存储组件，不先引入新通用服务。
- [ ] 长期入口与凭据分表/分字段表示：入口 ID 只用于定位业务；凭据以不可逆摘要保存并设有效期；原始 secret 不落日志、审计、analytics 或 Referer。长期访问地址不能单凭公开 ID 重新获得免密资格。
- [ ] 资源授权保留授予者、接收者、资源、动作、期限、来源及转授依据；群/Session 成员表仍是成员资格真相源，Workspace grants 仍是对象权限真相源，不另建一份易失步的访客成员表。
- [ ] 账号预建、资源邀请和发送不能假设跨服务天然原子：定义可重试编排、幂等关联和补偿；按 D-9，outbox 的终点是通知签发方而非系统直发；通知或发送失败不丢账号/会话关系，不伪报已送达。身份创建本身不隐式授权。
- [ ] 按 D-6 区分可丢失的短期 pending 与必须持久的 family：family 存 verify-hub 本地 SQLite（服务数据目录 `data/var/verify-hub/`，路径在此冻结）。定义原子轮换、重启后 grace 的同一 pair / 幂等语义、备份恢复与撤销不回滚；不保存 token 明文时仍需可重建必要签发信息。单实例也须保护状态与撤销记录；verify-hub 迁到其它 OOD 后旧会话须重新登录，写入交付记录，不宣称多副本或故障切换可用。
- [ ] jump 的 `{ current_zone_hostname, auto_jump }` 仅是该 origin 下可丢失的浏览器路由偏好，与账号、DID consent、专属邀请凭据、Guest 主体和会话 family 分开；存储不可用或被清除时回到手动选择，不据此新建身份或恢复授权。

验收：并发创建无重复身份；并发设密码与邀请登录有确定提交顺序；恢复备份不会复活已撤销凭据或将同名 App 继承为原身份源。

## 6. B：权限底座与撤销，签发之前完成

### SSO-04 / P0：全入口默认拒绝及授权交集

- [ ] External（用户类型 TODO T-4）在 v1 只被 verify-hub、msg-center 群路径、AI Workspace、HomeStation 接受，其余服务默认拒绝；Limited 的自身基线含 settings 读取、Profile 读写，以及 msg-center 绑定调用者本人的通讯录读取 / 按 `can_edit_contacts` 编辑；身份字段按 PRE-01 / 02 保护，系统用户组不带系统 RBAC 策略。零业务权限测试不能把这些已允许操作全部判为失败。
- [ ] 按 D-4 把受限主体判定和 scope/source 检查接到共享认证/授权上下文（`verify_trusted_session_token` 及其授权调用），默认拒绝，服务逐方法开放；仅给 token 增加字段不算完成。根域受限会话沿用 `System{control-panel}`，限制来自 claims 中的受限标记与上限。`runtime.enforce` 保留完整 AuthTarget 实例上下文，不能用 AppId 级 `authorization_key` 代替实例级管理和转授授权。
- [ ] 创建身份、邀请加入、代登录、资源使用、可转授是五类不同能力。App 的授予上限为“系统允许范围 ∩ owner+instance 获准转授范围 ∩ 资源方允许范围”；不能因 owner 是管理员或 App 有读取权就允许转授。
- [ ] 为下表产出逐方法审计清单：入口、认证来源、主体解析、RBAC、资源检查、scope、撤销、拒绝原因码与可调整项（D-4）、测试。未支持受限主体的方法明确拒绝；只验证 `principal_kind=user` 或“自己的数据”不够。

| 审计面 | 必查点 |
| --- | --- |
| Control Panel / system-config / scheduler | PRE-01 / 02：原始 set / JSON path / exec_tx / delete / 重建与 CP RPC 一致保护 type、groups、DID、doc / key；User sudo 不可绕过管理员边界。核查 App 安装、Agent 创建、角色生成和公开 `user.invite.accept` 的控制权证明 / 原子消费。 |
| msg-center | `caller_identity`、`authorize_resource`、`owner_session`、`group_rpc_authenticated`、group HTTP、`contact.*`、通知、批量接口；User 可观察 Agent、非 User/内部请求快捷分支必须明确边界。 |
| Message Hub 对象与代理 | 群消息、附件对象/内容、范围下载、NDN 代理和转发；确认资源引用与当前可见性，不能仅凭 ObjectId 或代理的 service token。 |
| AI Workspace | PRE-03：`RuntimeAuth`、Caller、create / import / fork / list_subjects、public / `*` / 具体主体 / 组 grants；RPC / HTTP / 订阅 / 导出均消费可信类别及上限，不能只按 `guest:` 前缀判断。 |
| HomeStation | 已定 External 范围须接通会话 DID 解析与资源读 / 评论授权；不存在本地 Profile 不能静默当作 Anonymous；reader proof 与本地会话区分。联系人缓存过期或上游失败按 D-5 处理，不无限使用旧关系授权。 |
| task-manager / workflow / OpenDAN / AICC | creator / owner 快捷分支、任务列表和结果、执行工具、计费操作、Agent 观察和订阅；调用者不能被替换成 App owner。 |
| nfs-server / ndn-mgr / 文件与上传 | 公开分享与受保护文件分别处理，路径、对象、上传和配额均需受限上下文。 |
| kevent / kmsg / stream | topic/资源过滤、订阅建立及持续输出、撤销后断开或停止输出；不能授予全局事件订阅。 |
| Gateway 与直连后端 | 页面路由、kapi、HTTP、批量和旁路端口结论一致；网络请求不能触发无 token 的内部放行。公开 bootstrap 仅公开壳和必要接口。 |

- [ ] 新建受限账号的零权限测试先落地：即使签名、target、exp 都合法，也不能枚举他人邮箱 / 联系人、未授权 Session / Workspace、任务、系统配置或个人存储；自身 settings / profile / 通讯录按基线单独验证，获准资源逐项加入。
- [ ] 授权状态不可确认时拒绝受保护请求；保留已有明确公开资源的匿名行为。普通用户、系统服务和设备 bootstrap 回归不得被受限身份改造破坏。

验收：交集任一项缺失均拒绝，并返回 D-4 对应的原因码；RPC/HTTP/对象/事件/代理/批量结果一致。审计清单与负例通过是开放所有新受限 token 签发的前置。

### SSO-05 / P0：会话、来源和资源撤销闭环

- [ ] 按 D-5 设检查点：资源状态与资源授权在每个受保护请求复核，包括附件读取和订阅 / 流的每次投递；账号状态、来源身份域 / 代登录权、会话上限与 family 在 refresh（含 grace）、delegate、callback 时复核，旧 access 至多残留到 TTL。复用 refresh 锁内复核与并发去重，避免 logout 后 refresh 或 callback 重新插入有效 family；不对所有 access 逐请求在线查询账号状态。
- [ ] 专门改造 refresh grace 分支：当前命中宽限期会在 `validate_refresh_principal` 前返回缓存 pair，不能视为已经复查用户/来源状态。正常刷新、宽限重试、delegate 和 callback 都不能绕过新的撤销判定；family/JTI 的 CAS 不能替代这些检查。
- [ ] 按 D-5 实现持久撤销依据与缓存一致性，逐类记录提交后允许的最大延迟：资源撤销为下一次受保护请求，账号 / 来源 / logout 为 access TTL 15 分钟（Q-03）。msg-center 组 reload 失败、HomeStation 联系人缓存与上游失败回退改为版本化、主动失效或无法确认即拒绝。覆盖命中缓存、连接保持、重启和状态服务故障。
- [ ] 账号 / 身份源 / 代登录权停用拒绝新的认证、刷新和兑换，相关旧 access 按 D-5 生效。Guest 没有系统 family：关闭 Guest 入口与撤销匿名资源授权分别处理。只撤销某 Session / grant 不注销无关资源的独立有效会话；App 卸载保留历史身份并停止旧来源代登录。
- [ ] 用户退出 / 单设备撤销终止对应服务端 family，旧 access 的剩余有效窗口按 D-5；未消费 callback 不得恢复该 family。类型 / 组 / 身份映射改变不让旧会话自动增权，明确重新认证与资源重新授权的入口。
- [ ] 明确普通 CP→App 独立 family 与受限来源衍生会话的关系。现有桌面 logout 不撤销已签发 App family；混合 SSO 的来源/范围约束必须在换票后保留，不能借现有 delegate 洗掉受限来源。
- [ ] 对持续流、订阅和附件落实同一撤销矩阵：资源撤销在下一次投递前生效；账号 / 来源 / logout 按 Q-03：连接所用 access 到期前未续上即断开，不依赖用户主动重连。按组和投影授权须覆盖 reload 失败、周期补偿、HomeStation 旧联系人缓存；状态不可确认按已定规则拒绝。UI 收到失效后停止拉取并清内存缓存、Blob URL，已合法送达的历史内容不承诺远程抹除。

验收：为旧 access、旧 refresh、未消费 callback、缓存及已建立订阅分别给出 D-5 决定的生效时刻；超过允许窗口不能再访问，任何重试不得恢复已撤销资格或延长窗口。类型 / 组升级不提高旧会话上限。跨 Zone logout 按 Q-04 验证，不额外承诺全局退出。

## 7. C：统一身份接入

### SSO-06 / P0：专属邀请和首次设密码

- [ ] 提供获准管理方的“预建指定 Local User + 关联业务资源 + 生成专属入口”能力。user ID、绑定渠道和指定身份来自可信记录；链接持有者不能改成任意 DID/user ID。不同客户不同凭据。按 Q-10，同 Zone 同客户由签发方显式指定复用已有账号，每个 Session / 入口独立邀请与 grant；系统不按手机号 / 邮箱自动归并，账号已设密码时新链接只作入口。
- [ ] 专属邀请凭据绑定账号、入口、过期时间和凭据版本，有效期固定 48 小时，v1 不可调整（Q-01）；每次登录重新检查密码状态、账号状态、邀请撤销、来源及 target。有效期内可重复换取标准会话，单次 callback code 独立防重放。
- [ ] 专属邀请凭据到期限制新的免密登录，不等于会话、账号或历史删除。已有有效会话按独立生命周期和当前资源资格处理；邀请撤销对其衍生会话的影响在 SSO-05 中明确，不把“到期”与“来源代登录权被撤销”混为一谈。
- [ ] 采用下表实现状态机，并覆盖返回原业务所需的最小导航资料；链接登录本身不等于加入 Session，也不等于拥有分享文档权限。

| 条件 | 新的免密登录 | 页面及后续 |
| --- | --- | --- |
| 凭据有效、未设密码、账号可用 | 允许；首次访问不永久消费专属邀请凭据 | 使用预设身份；提供不强迫的设密码入口。 |
| 凭据过期、未设密码 | 拒绝 | 可申请向原绑定渠道重发；不向页面返回新 secret。 |
| 已设密码、无适用会话 | 拒绝旧凭据 | 原地址可继续作入口，使用密码登录。 |
| 已设密码、有适用会话 | 不再使用旧免密资格 | 复用现有会话，再检查资源权限。 |
| 邀请/账号/资源被撤销或业务关闭 | 对相应操作拒绝 | 准确区分原因；重发不恢复已撤销资格。 |
| 当前 Bob 会话，链接指定 Alice | 不静默替换或沿用 Bob | 显示身份冲突并要求明确继续/取消；不把操作记给错误用户。 |

- [ ] 首次设置密码只针对当前已认证的专属邀请账号，按 §4 P-1 区分专用首设能力与常规 `allow_password_change`；由受控写入路径校验标准密码格式，并原子更新密码、凭据状态 / 版本和旧免密资格。不能要求不存在的旧密码 sudo，也不能开放整份 settings 写入；PRE-04 的常规改密覆盖同时验证。
- [ ] 设密码不换 user ID / DID、不丢成员关系、不授予普通用户权限、不改 canonical zone。并发登录与设密码以提交顺序复核，任何旧的尚未完成兑换不能在设密码后新签发免密会话；按 Q-03 保留当前终端，撤销该账号其它经专属邀请免密资格建立的 family。
- [ ] 按 D-9 实现重发：系统生成新资格，经可靠 outbox 通知签发方，由其经原绑定渠道发送；系统不实现 SMTP / SMS。人工签发方收到不含 secret 的 System Notify 后，在 CP 的专属邀请管理处取一次性显示的新链接；App 签发方经登记回调取得。冻结通知协议、限流、脱敏、失败重试与幂等，并核验实际送达。按 Q-02 只替换同一入口的免密资格，不能返回测试 URL 代替送达；已设密码不恢复免密，忘密另行处理。
- [ ] 管理方提供查询、撤销和到期状态；邀请人有邀请权不等于有客户代登录权，管理 API 和登录 API 分开授权。

落点：Control Panel 用户/浏览器编排、Verify Hub 签发、统一身份存储、msg-center 资源邀请、SDK/CLI。验收对应 AC-01～06、21～23，必须含有效期内第二次进入与设密码并发负例。

### SSO-07 / P0：自助注册和原账号登录

- [ ] 增加注册入口，仅在入口允许自助注册且 Zone 自助注册开关开启时提供（D-8），否则返回 D-4 原因码；服务端验证原资源/目标 Zone、用户名唯一性、密码格式、资料规则和配额。公开请求不能指定 admin、user_type、角色、owner、canonical zone 或任意资源授权。
- [ ] 注册创建 `Limited` 账号，不加入任何系统用户组（用户类型 TODO T-3），权限只来自分享资源的授权；不复用管理员专用的 `user.create`；凭据和业务 grant 分开提交/编排。重复提交不建第二个账号。
- [ ] 已有目标 Zone 本地账号可直接登录；注册/登录成功回原内容，资源已经关闭或未授权时显示对应结果。显示账号属于哪个 Zone。
- [ ] 密码登录和 refresh 接入 SSO-02/05，不能因使用 password 方式就去掉外部账号的来源与权限类别；不自动关联已有 DID 或 Guest 历史。

验收对应 AC-10、11、16、18、23，含公开接口伪造角色/账号类型/授权与并发抢名。

### SSO-08 / P0：统一浏览器 Guest

- [ ] 按用户类型 TODO T-9：Guest 是匿名访问者，由 AI Workspace 的 `gt_` / `guest:` 资源主体承载，verify-hub 不为其签发系统会话；只在资源 owner 于入口开启 Guest 时提供，Zone 不设全局 Guest 开关（D-8）。不能在界面或授权上冒充已验证手机号、Local User 或全局 DID。
- [ ] 复用同浏览器有效状态；不共用全站一个 `sub=guest`，也不把可自报的浏览器 ID 当成账号控制证明。丢失状态/换浏览器允许新 Guest，不承诺找回旧身份。
- [ ] 入口页与 SDK 统一展示匿名访问状态，不新建另一份 Message Hub 匿名身份系统。记录旧匿名记录的处置决策；不默认历史合并、不根据相同昵称转移权限。
- [ ] Guest 的注册、DID 登录或身份切换走明确动作，清除旧身份缓存，资源重新授权。新认证状态不自动继承匿名记录的所有权。

验收对应 AC-09、16；同浏览器续用、状态丢失、关闭 Guest 入口后的有效权限变化均可验证。

### SSO-09 / P0：DID canonical zone 联合登录

- [ ] 核对现有 DID / OwnerDocument / ZoneDocument 解析能力，按 D-7 冻结“本人授权的唯一 canonical zone → 可信 ZoneDocument hostname → canonical verify-hub 兑换端点”证明链；不依赖公开 verify-hub 公钥字段。`default_zone` 字段只有在可信且适用于该主体的文档中才有证明力；目标 Zone 的本地写入不算授权。
- [ ] 支持 Alice 由 Bob 承载 canonical zone：验证 Alice 的归属授权和来源，不要求 Alice 等于 Zone owner，也不要求她拥有物理 OOD。当前 `OwnerDocument::get_default_zone_did` 取绑定列表首项，`is_bound_to_zone` 仅判断列表包含，bound 不等于 canonical。可绑定多个 Zone，但对外认证只认可信的唯一 canonical；归属缺失、证明冲突或无法验证时不猜测。
- [ ] jump Locator、SN 登录态和 App 的 Active DID 提示只可辅助发现/选择；进入 DID 联合路径仍按 D-7 的授权码交换独立验证上述归属链。手填 hostname 不成为 issuer/key/兑换端点的信任根；定位成功不代表联合认证成功，也不能跳过 DID+目标 Zone 的首次 consent。
- [ ] 按 §4.1 D-7 的四步实现：目标 CP pending → canonical consent 与签发 code → 目标 CP callback 调 `login_by_remote_verify_hub` → 目标 verify-hub 经 HTTPS 向 canonical 兑换。code 绑定 DID、来源 / 目标 Zone、认证用途、期限、nonce、浏览器状态及披露资料；接收方与 redirect 来源可信，防重放、串会话和开放重定向，兑换地址只由 DID 解析得出，不能由页面或调用参数指定。兑换方鉴权建议采用 PKCE（S256）式绑定：verifier 只存在目标 CP 的服务端 pending，截获 code 无法兑换，也不需要公开 verify-hub 公钥。
- [ ] DID 方式仅在入口允许且 Zone DID 开关开启时提供（D-8）；canonical 侧不满足签发条件、兑换失败或来源不可用时返回 D-4 / SSO-02 的结构化原因。
- [ ] canonical zone 首次向某个目标 Zone 使用 DID 时显示身份、目标和资料范围，明确同意后签发；取消不签发。按 Q-05，只共享 DID、来源与目标 Zone、公开显示名 / 头像；consent 记在 canonical 侧，资料范围扩大、用户撤回或 canonical zone 变化时重新确认。consent 以 DID+目标 Zone 为边界，向 B 同意不能替代向 C/D 同意。
- [ ] 目标 Zone 验证证明后：该 DID 已入驻本 Zone（有账号记录）时映射到该账号；否则作为 External（用户类型 TODO T-4），不创建账号记录或本地 Profile，名字与头像从 DID 公开文档解析。签发本 Zone 标准且受限的 token pair，External 以原 DID 为主体并带 External 标记；不创建新密码，不覆盖已有同名本地账户，不改变 canonical zone。下游业务继续只验证目标 Zone 的标准会话。
- [ ] 独立限制本地签发与对外证明资格：目标 Zone 不能因持有本地账户、Profile 或自己的 Verify Hub 私钥，代表外来 DID 再登录第三方；对外联合断言只为 Admin、User 签发（用户类型 TODO T-6）。不得把远端 key 混入普通 session 的 `verify-hub` 信任槽。
- [ ] 后续访问在有效 consent、会话和资源资格下复用状态。按 Q-04，目标续期不向 canonical 复核，联合会话最长 30 天后重新联合；canonical 不可用只影响新登录和重新联合，key / 归属变化在重新联合时拒绝，不能偷偷改走目标 Zone 或另一个身份。

落点：Verify Hub + Control Panel 专用联合流程、共享 DID/身份解析、SDK；同时核对同级 [name-lib/user.rs](../../buckyos-base/src/name-lib/src/user.rs) 与 [name-client/profile_resolver.rs](../../buckyos-base/src/name-client/src/profile_resolver.rs)，扩展文档类型或解析器时同步调用方。验收对应 AC-12～20、24，含第三方 Zone 接受伪造来源的明确负例。

### SSO-10 / Session Only 扩展：App access token 接入

按 S-1 移出本批：本节保留为后续任务，不计入本批完成门槛。

- [ ] 按旧设计提供 `login_by_app_access_token`：登记身份域、公钥或受信服务端校验方式、获准 AppInstance/owner、允许 target 和范围；先选定验证契约，不能让 token 自报任意校验 URL。
- [ ] 校验真实性、用途、接收方、期限、稳定外部账号和绑定状态；App service token 不能换客户会话，App 不能指定任意普通用户/管理员 DID。凭据定位字段只能作查找线索。
- [ ] 身份域登记、建号与兑换分别限权、限流和限额；按来源 App/账号/请求来源统计，公开失败结果不泄露账号是否存在。记录签发和绑定审计，不保存凭证明文。
- [ ] 链接登录兑换凭据单次消费，以绑定该次兑换的 nonce 支持合法幂等重试；更换 nonce 重放拒绝。它与专属邀请可重复免密资格分离：若 CRM 同时服务专属邀请，每次由仍有效资格产生独立兑换过程，不能混用生命周期。
- [ ] 签发标准受限用户 token pair，保留身份域、来源、强度和批准上限；refresh 与跨 target 换票重新检查来源状态且不得增权。
- [ ] App 停用 / 卸载 / 代登录权撤销立即拒绝新兑换和续期，已有 access 按 D-5 的窗口失效；其它独立登录来源按各自规则处理。外部账号换设备重新认证映射同一主体。

验收：跨 owner 同 AppId 冒认、跨身份域同账号、伪造主体、错误 audience、重复消费、来源停用和旧会话提权均失败。Message Hub 不增加自己的兑换中心或用户凭据库。

## 8. D：同 Zone SSO 与业务入口

### SSO-11 / P0：分享入口编排和跨应用复用

- [ ] 复用 Control Panel 的 target / origin / redirect resolver、pending callback、Cookie 与 refresh，接入 §5.0 当前身份响应。扩展 pending 以绑定入口 / 原业务 intent、预期旧 / 新主体、state / nonce；callback 消费前复核来源 / 账号 / family。取消不覆写 Cookie，多标签页不串流程。可复用换票内部逻辑，不直接放宽调用者检查。
- [ ] 从分享链接先解析可信入口上下文，再展示入口允许 ∩ Zone 开关后实际可用的 Guest、本地登录/注册、DID 等方式（D-8）；专属邀请只展示指定身份路径。只公开必要的资源介绍，不先加载私密会话内容。
- [ ] 无 session 时公开 bootstrap、受控认证 / 重发 / 回调入口；验证根域、系统入口、App 子域、移动端及跨 origin。明确 Gateway / CP 对 Origin、CSRF 和 Fetch Metadata 的检查职责；现有 `request_origin` 仅从 Host + scheme 构造目标 origin，不等于校验请求 Origin。公开页面不等于放开数据 API，不能通配公开 `/kapi/*`。
- [ ] 同 Zone Message Hub→Workspace 复用主体；同 target 可复用适用会话，跨 target 重新签精确目标会话。Workspace 必须独立检查访问资格；若原会话只准某 Session，不能把 scope 直接放大，需按来源允许范围重新授权。
- [ ] 保留 host-only Cookie、精确 AppInstance/owner、完整 path/query/hash。不同 target、route 被重新指向其他实例、callback origin 不一致均拒绝；URL 不传长期 session/refresh token。
- [ ] 当前身份与专属邀请 / DID 登录 / Guest 入口冲突时显式选择，取消不覆写 Cookie。初期不做复杂多身份管理，但不能让已有管理员 session 被静默混用；清理跨身份的用户、对象、会话和权限缓存。
- [ ] SDK 提供当前可信主体、登录方式、所属/目标 Zone、D-4 拒绝原因码和续期接口；页面按原因码区分访客提示与管理员可调整项。所有消费者删除自行猜 DID、以“登录成功”代替权限检查和所有错误跳 `/login` 的逻辑。

验收对应 AC-07、08、14、21、23；在不同 App origin 做真实浏览器测试，不能只用 Desktop 内两个面板证明跨应用 SSO。

#### SSO-11.1：jump 外部调用入口与登录后恢复

本节承接已有 jump 讨论，归入 SSO-11；只完成路由、认证衔接和操作确认边界。保存、安装等 action 的业务实现由对应模块负责，示例可用无副作用处理器验证衔接，不能据此宣称业务功能完成。

- [ ] jump 服务入口由系统配置，正式默认 origin 为 `https://jump.buckyos.org`；未配置覆盖值时使用该默认值，配置覆盖后 SDK / App 使用有效值，不在各业务页面写死域名。action 路径按登记契约拼接；jump 服务入口与浏览器保存的目标 Zone hostname 是两项不同设置。默认值已定，配置键、配置归属及客户端读取方式仍需在实施前冻结。
- [ ] SDK 将 `buckyos://` action 调用与 HTTPS 降级封装在统一入口。按平台核对 App / 扩展、Universal/App Links、纯浏览器和内嵌浏览器路径；未安装或无法唤起时仍能进入 Web 流程。`blur` / 超时仅是启发式，不能作为“已登录”或“已执行”的证据；保留显式继续/重试入口，避免 App 与 Web 两路重复提交操作。
- [ ] Locator 可依次来自 App / 扩展根据 Active DID 提供的提示、可用的 SN 浏览器会话、用户手动选择。SN 是可选发现来源；没有 SN 账号、没有客户端、SN 不可用时，用户仍能填写自己的 Zone hostname，不要求先注册 SN，也不建立 jump 服务端的全局“用户→Zone”映射。
- [ ] 纯浏览器在 jump 自己的 `localStorage` 保存 `{ current_zone_hostname, auto_jump }`：首次或无记录时选择，后续预填并允许修改；保存和勾选“不再提示”必须来自明确操作，提供修改/清除入口。第三方传入 hostname 只作为候选展示，不覆盖已有设置、不触发自动跳转；`auto_jump` 只省去路由选择，不省去目标 Zone 登录、DID consent 或业务确认。
- [ ] hostname 规范化后固定构造 `https://<hostname>/<registered-action-path>`，允许用户使用自有域名，不把目标限制为 `*.web3.buckyos.ai`。输入拒绝完整 URL、userinfo、端口、path、query、fragment 及混淆编码；action 和参数按登记契约校验，不接受任意 `next` / `redirect_url` / callback。手动选择外部站点只是导航授权，不能让该站点获得其他 Zone 的凭据或联合认证信任。
- [ ] jump 只传递最小业务 intent。保存场景的 `res_url`、`page_url` 等参数按草案放在 fragment，避免进入 jump HTTP 请求；fragment 仍对页面脚本可见，不能当作加密或凭据保护。jump 不收集专属邀请 secret、SN/Zone session、refresh token、owner capability 或私钥；这些凭据继续只交给原认证入口，短期联合 code 只由 SSO-09 指定回调消费。
- [ ] 最终 Zone 验证 action 后建立独立 pending 上下文，绑定 action/参数摘要、实际目标 Zone、AuthTarget、浏览器流程 state/nonce、到期时间和经校验的返回位置。未知 action、过期、篡改和跨标签页串用拒绝。第三方自报的 `source` 不证明来源可信；本地会话、DID 联合或取消返回均恢复同一业务上下文，不能回系统首页后丢失原请求，也不能复用另一次邀请的身份。
- [ ] 外部 GET / deep link 只打开流程或确认页，不直接保存、安装、签名或付费。最终 Zone 展示目标、实际登录身份、action 和资源信息，再以经授权且有 CSRF 防护的 POST 提交；即使已有管理员登录态也不能静默执行。操作幂等/重放检查由最终处理器负责；实际资源拉取由业务模块处理，跳转参数不能绕过其 URL、重定向及 SSRF 校验。
- [ ] jump、SN、每个 Zone 分别使用 host-only Cookie；不用 `.buckyos.ai` / `.web3.buckyos.ai` 父域共享登录态，不把 SN 已登录当作目标 Zone 已认证。需防 CSRF 的 Cookie 请求校验精确 Origin / CSRF token，并结合 Fetch Metadata；同父域 Zone 的 same-site 关系不能代替 origin 隔离。
- [ ] jump 保持可静态部署，不加载第三方脚本/统计，配置严格 CSP、`Referrer-Policy: no-referrer`、`Cache-Control: no-store`；最终敏感确认页使用顶层页面并限制嵌入，不能由第三方 iframe 代替用户确认。日志与错误上报不记录 intent 原文中的敏感内容或认证凭据。
- [ ] 目标不可达、登录取消、身份冲突、无权限、action 不支持分别反馈；提供修改目标或重试导航的操作，但不能自动切换身份/Zone 后继续提交。专属邀请 / Workspace / Session 固定入口始终回原资源 Zone，只有“在自己的 Zone 执行”类 intent 使用 Locator 选择目的地。

验收：第三方网页发起 intent → 无客户端 / 无 SN 的 jump 选择 → 目标 Zone 必要认证 → 恢复并确认原 action；同时验证已有会话、取消、权限拒绝和重复唤起。浏览器返回、刷新及多标签页不能导致串身份或重复副作用。

### SSO-12 / P0：Message Hub 的 Session Only 闭环

- [ ] 复用 `Invited → Included`、`accept_session_invitation` 和 `submit_guest_request`。身份建立、群侧邀请/批准、本人接受分别记账；进入页面可顺序执行，但登录成功不自动成为群正式成员。
- [ ] 分清已有群 invite link（群加入凭据）、Session Guest 邀请和专属邀请免密资格。当前 `SessionMembershipRecord` 无独立 invitation ID/expiry，需定义 Session 邀请版本、有效期、撤销/重新邀请和旧请求拒绝规则；采用既有幂等框架并让前端传入幂等键。登录成功而加入失败可重试，合法接受重试返回原结果，旧版本不能接受新邀请，移除/封禁后旧请求不能重新加入。
- [ ] 只允许具名、`allow_guests=true`、成员策略非 `inherit` 的 Session；保留封禁、上限、频率、join Hook 和历史可见区间。邀请接受前只授权本人查看最少邀请资料及接受动作，不预先开放历史。
- [ ] 给 `group_rpc_authenticated`、group HTTP、对象读取和事件入口统一接入 SSO-04 的身份 / source / scope / 资源授权；区分普通系统 RBAC 路径与显式开放的受限群方法，再执行群 / Session ACL。保留真实客户端用于 `allowed_clients`，不得伪装为总允许客户端。
- [ ] 复用 External 向本机托管群发送的已有分支，补 Limited 路径与统一上限检查；按当前 Session、消息操作和 `from=可信绑定 DID` 校验后写群存储。无个人邮箱者不先要求 SENT，但不能直接删检查后放行；不创建邮箱、不替换 App owner、不授予 `obj://msg-center/*`。
- [ ] 检查 `is_local_recipient` 与群消息、邀请通知、事件 fanout 的 SENT / INBOX / owner_sessions 投影，身份在本 Zone 有记录不代表具备个人投递能力。Session Only 默认不注册本地投递能力；用数据库断言验证接受邀请、发言和接收消息后没有隐式个人邮箱/投递记录，群记录及客服和 Agent 的既有投递正常。客户在客服通讯录中的可见性按用户类型 TODO T-6 的系统用户组投影（如“客户组 → 客服组”）；投影不授予个人收件能力，见该文 §3.3-D。
- [ ] 增加直接读取 host 群存储的数据层，使用 `group.list_sessions/list_messages/changes`，不依赖 `owner_session` / INBOX 投影，也不要求通讯录权限。轮询可作为首期更新方案；使用 kevent 必须有相同资源过滤和撤销。
- [ ] 新增公开可达的受限入口（路径在路由设计中确定），仅加载身份流程与获准会话视图，复用 `ConversationView`。名字/头像用可读成员资料；不暴露群其他 Session、客服邮箱、通讯录、群管理或个人归档/置顶功能。
- [ ] 附件读取携带群/Session/消息引用上下文，后端同时验证 Session 可读、消息处于可见历史、对象确被引用；猜 ObjectId、跨会话 context、删除/撤回后不可读对象均拒绝。对象缓存按 Zone+主体+上下文隔离，撤销/切换时清理。
- [ ] 首期不开放 Session Only 媒体上传；后续单独增加写权限/配额。客户打开群内 Workspace 链接按 SSO-11 处理，不复用客服的身份或权限。
- [ ] 资源授权结束、移出、封禁、删除及时停止无权读取；Session 归档保留既定只读语义，不等同于删除。账号/DID 保持稳定，历史消息 `from` 不改写。Contact 的展示合并不改变授权主体；跨 Zone joined-group 自动同步是另一个交付项，不能与浏览器 DID 联合登录混称完成。

落点：`msg_center/{message_hub,owner_session,group_service,group_http,object_access,msg_center}.rs`、`buckyos-api/msg_center_client.rs`、Desktop messagehub API/datamodel/路由。验收：Alice/Bob 同一工单、Alice 其它未授权工单、无邮箱客户发消息/读附件、客服和 Agent 正常收发，以及所有撤销负例。

### SSO-13 / P0：AI Workspace 分享策略和操作权限

- [ ] 以现有 workspace/entity grants 为资源真相源，增加分享入口允许的认证方式及有效策略查询；按 D-8，入口与 grants 同库，有效方式 = 入口允许 ∩ Zone 自助注册 / DID 登录开关，Guest 由 owner 在入口上控制。不把“允许自助注册 / DID 登录”变成自动 grant 全 workspace。
- [ ] `RuntimeAuth` 和 Caller 消费统一受限上下文；修正仅凭 `guest:` 前缀区分访客/已认证的假设。明确 `public`、`*` 和具体主体对新身份类别的匹配，防止匿名访问者或 External 自动获得创建/导入/复制或 `*` 授权。授权对象新增 `group:<id>`（系统用户组，T-3）与 owner DID + 联系人 tag（含投影，T-5、T-6），两者采用不同类型 / 命名空间；复用 `contact.check_membership` 的精确 DID 查询。冻结现有 username grant 与 DID 的可信映射，以及组变更 / 服务不可用时的生效规则。External 按 T-4 处理，完成 PRE-03 的创建 / 导入 / 复制 / 主体列表负例。
- [ ] DJ 场景优先复用资源结构：公开可读公告/播放状态，歌单实体子树仅授参与者 `append`，切歌和公告编辑只授 DJ。验证 append 的实际写操作所需能力；如果必须增加权限表达，先冻结最小变更，不能通过全 workspace `update` 绕过。
- [ ] 权限按祖先 grant 取并集，没有 deny 可补救上层宽授权。检查所有祖先授权；分享单 surface 时明确实际授权对象，避免链接只指一个 surface 却开放其它私密内容。
- [ ] RPC、HTTP 读取、批量变更、订阅、附件、导出和后端工具执行相同用户/scope/资源检查；页面隐藏按钮之外，直接请求切歌、改公告、改 grants 也必须失败。
- [ ] 身份来源/所有者/目标 Zone 可见，Guest、自助注册和 DID 登录者进来后不自动成为 workspace owner。资源关闭、grant 撤销和账号停用不循环跳登录。

验收对应 AC-08～20；用 Guest、专属邀请、自助注册、DID 登录和 DJ 分别执行允许/禁止动作。播放同步、音频编排不属于本任务；轻量背景个性化列 P1，不阻塞认证闭环。

## 9. E：联调、文档与完成条件

### SSO-14 / P0：验证矩阵

每项记录执行命令 / 环境、预期、结果与失败证据；未运行写“未验证”。撤销期望按 D-5 的分类填写，时限取 Q-03 / Q-04：access 15 分钟、refresh 滑动 7 天、联合会话最长 30 天。mock 只验证组件和协议，不替代真实 Cookie、发送渠道及跨 Zone 验收。

| 测试组 | 必须通过的内容 | 来源 |
| --- | --- | --- |
| 前置修复 | User sudo 不可改自身 type / groups / 可信 doc；User / Limited 原始 profile 写不能冒充 DID，覆盖 set / JSON path / exec_tx / 删除重建；Limited 改密覆盖允许 / 拒绝正确；AIWS 不因非 guest 放开创建或 `*`。 | PRE-01～04 |
| 主体与会话矩阵 | Admin / User、有密码与无密码 Limited、External、Anonymous 共用 Rust / TS fixtures；类型 / 组升级不提高旧会话上限；刷新、换票保留来源；身份切换不沿用旧资料缓存。 | SSO-01～05 |
| 专属邀请正向与生命周期 | 两客户各用专属链接；48 小时内重复进入，超时拒绝新的免密登录；同客户第二个工单复用原账号且不开放第一个工单；原账号设密码；旧凭据失效；设密码后当前终端保留、其它免密终端在 15 分钟内失效；有效现有会话可继续；过期后签发方收到不含 secret 的通知，取得新链接并经原渠道送达（D-9）。 | AC-01～06 |
| 自助注册 / Guest | 自助注册返回原资源、旧账号可登录；同浏览器 Guest 延续、状态丢失；不自动合并历史或改变归属；Zone 关闭自助注册 / DID 登录后入口对应方式消失并给出原因，Guest 只受入口开关控制（D-8）。 | AC-09～11、18 |
| 同 Zone 跨应用 | Message Hub→授权 Workspace 不重复注册；未授权文档拒绝；跨 App origin 换票仍精确绑定实例、owner、来源及 scope。 | AC-07、08、14 |
| DID 三 Zone | Alice canonical zone→B 首次确认/取消/再次访问→C 独立确认；consent 只含已定字段；canonical 停机时已有会话继续、新联合失败；联合会话 30 天后须重新联合；无新密码；Bob 承载 Alice 的合法归属可用；DJ/B 伪称 Alice 来源在 C 被拒绝。 | AC-12～20 |
| jump 无中心账号路径 | 无客户端、无 SN 账号/会话及 SN 不可用时，手填自有域名到目标 Zone；本地预填、修改/清除、显式 auto_jump、存储禁用/丢失；这些操作不产生已认证身份或变更 canonical zone。 | Issue #34 Locator 补充 + SSO-11.1 |
| jump 与 SSO 连续性 | 未配置覆盖值时使用 `https://jump.buckyos.org`，覆盖后 SDK / App 使用配置入口；新登录/有效会话/DID consent 后恢复原 action；取消不执行；固定专属邀请/Workspace/Session 入口不被 Locator 改写；App 唤起与 Web 降级、刷新/返回/多标签页不串流程或重复提交。 | SSO-09、11.1 + AC-21、23 |
| jump 信任边界 | 伪造候选 hostname 不覆盖偏好、不自动跳转、不成为 canonical 证明；非法 hostname/action/返回地址拒绝；GET 无业务副作用；跨 Zone Cookie/CSRF 隔离；jump 请求、存储、日志无认证凭据，fragment 不进入 jump HTTP 请求。 | SSO-02、09、11.1 |
| DJ 权限 | Guest/本地/DID 可按规则 append 歌曲，不能切歌、改公告、改 grants；未实现钱包不展示。 | AC-16、24 |
| 默认拒绝与实例隔离 | 无 grant、App 无能力、scope 缺失、资源拒绝各自失败，并返回 D-4 对应原因码；访客侧未开放与不存在结果相同，管理员按提示能找到调整项；同 AppId 不同 owner 隔离；不继承 `users`、owner、sudo；不可冒认普通用户。 | Session Only §11、D-4 |
| 群完整路径 | 邀请前/后、Included/Removed、历史边界、Hook、`from`、客户端限制、零邮箱投递断言、附件引用、RPC/HTTP/事件一致；接受响应丢失重试、邀请过期、旧版本重放、并发接受/撤销。 | Session Only §6、§11 |
| 并发与恢复 | 并发建号 / 设密码 / 重发 / refresh / logout / callback；单次 code 与专属邀请多次有效分开；verify-hub 重启后 family 仍可刷新且 grace 幂等，迁移 / 备份不复活撤销资格；组 reload 失败、过期联系人缓存、持续连接均满足已定时限和故障规则。 | D-5 / D-6、Session Only §9 + AC-03、04 |
| HomeStation External | 原 DID 会话不依赖本地 Profile，不降为 Anonymous；读 / 评论按实际授权，联系人组 / 投影撤销与上游故障符合已定策略。 | T-4 / T-5、SSO-04 / 05 |
| 页面与异常 | Bob/Alice 身份冲突；链接/资源/权限失效区别；原目标恢复；移动/桌面；来源不可用；无凭证日志。 | AC-21～23 + PRD §10 |
| 原功能回归 | 普通本地用户、Desktop/App SSO、refresh 并发、logout、App availability、群成员/Agent、现有公开 Workspace；LoginAssertion/session/refresh/sudo 边界。 | 当前实现 |

真实环境准备：至少一个根域和两个不同 App origin、canonical / B / C 三个有可信解析与 HTTPS 的 Zone，以及 Admin / User、未设密码的专属邀请账号、已设密码的专属邀请账号与自助注册账号、无账号 External、AIWS Guest 测试身份。现有 Desktop 跨 App mock 只重定向而不设置真实 Cookie，部分 real 用例直接注入 Cookie，不能代替实际 callback / 身份冲突测试；专属邀请另需可核验签发方与送达渠道。

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

- [ ] 更新 SSO、verify-hub、RBAC、UserType、Local User 生命周期、Control Panel 协议、SDK 使用说明、Session Only、Self-host Group 接入、Message Hub UI_DATAMODEL 和 AI Workspace 分享文档。删除“guest 等同匿名且永不能登录”“任意本地 Profile 即归属证明”等混淆表述，并明确已实现/规划范围。其余文档按 PRD 使用专属邀请 / 自助注册 / DID 登录 / Guest。
- [ ] 补充 SDK / jump / 目标 Zone 的责任、action 路由契约、纯浏览器无 SN 路径及登录后恢复示例；明确 Locator 与 canonical zone 的差别。保存/安装入口文档引用同一契约，平台唤起与业务执行分别列验证结果，不把历史样例标成已交付。
- [ ] 更新 [桌面 App 自动登录记录](desktop-app-sso-auto-login-todo.md) 中与受限来源、family 和撤销相关的取舍，保留普通桌面行为与本轮变化的准确边界；不把旧的独立 family 结论套给所有混合会话。
- [ ] Rust 共享类型、服务、SDK/CLI、前端、Gateway 模板和测试一起迁移。修改源码和生成入口，按构建流程分发 SDK；不只改 `dist` 或 `rootfs` 生成副本。如改 Gateway 公共配置，检查同级 `cyfs-gateway` 同步副本。
- [ ] 检查 Jarvis `cmd_skills/buckyos_admin.md`、`buckyos_app_dev.md` 等受影响手册，补充外部账号/邀请/授权管理的真实命令或说明无需变化的依据；业务脚本有权限才能创建身份、发邀请、转授资源。
- [ ] 记录部署/数据初始化与恢复方案。开发阶段不默认做旧协议兼容，但现有账号、群历史与已保存公开 Workspace 的处置须明确；结构变更不能无声删除业务数据。
- [ ] 输出交付记录：已实现任务、未决定事项、身份/权限/撤销协议、测试与 DV 结果、单实例或多实例限制、尚未验证的渠道和浏览器。所有 TODO 按实际证据勾选。

核心闭环完成条件：专属邀请、自助注册、DID 登录、Guest 四种用户体验均在系统统一边界下成立；Message Hub 受限客户不依赖个人邮箱权限；Workspace 操作角色正确；跨应用不重复注册；DID 跨 Zone 保留逐目标首次选择；专属邀请账号设密码终止旧免密资格；普通访问永不改变 canonical zone；所有已签发的受限会话均按已定授权上限、来源约束及 D-5 / Q-03 / Q-04 的撤销窗口与检查点验证。仅登录页面可用或 JWT 能验签不算完成。

## 10. 本轮文档更新记录（2026-10-21）

- 已更新用户类型前置完成情况与源码基线；删除 Guest 尚存在、空密码未拦、scheduler 回退未修复等旧现状。
- 将需用户拍板的技术路线、产品参数、范围和两项补充澄清统一收至 §4；推荐仍为待定，未替用户选定撤销强度、存储、联合认证或发送方案（这是决策前的状态，见下方“决策回写”）。
- 新增 PRE-01～04、阶段 A 契约交付表，并联动 SSO-01～15 的身份、授权、凭据、撤销、浏览器与验证要求。
- 本轮只改实施计划，不修改运行代码、协议或 cmd_skills；管理手册仍在 SSO-15 随实际实现联动。本次文档检查核对本地链接、章节引用和 diff；未重跑构建或 DV。前述 6 项 SDK 测试来自上一轮 review，不是混合 SSO 已通过的证据。

决策回写（同日，用户确认 D-4～D-9 后的 review）：

- §4.1 改为已定：六项均采纳原推荐，去掉后文的备选分支。D-4 按用户补充增加拒绝原因码及“由谁、在哪里调整”的登记，访客侧不泄露资源存在性；D-7 以用户提出的 `verify-hub.login_by_remote_verify_hub` 为目标侧签发入口，并补全 canonical 侧签发 / 兑换 code 两个方法和四步流程；D-8 同时关闭 Q-06。
- review 补充的限制与建议：D-5 的“下一次受保护请求”包含订阅 / 流的每次投递，msg-center 组 reload 与 HomeStation 联系人缓存须改为无法确认即拒绝；D-6 写明单实例与 OOD 迁移后须重新登录的限制；D-8 没有全局 Guest 开关，记为已知风险，新增待定项 D-8a（Zone 自助注册 / DID 登录开关默认值）；D-9 建议复用 System Notify，通知不带 secret；SSO-09 建议用 PKCE 式绑定做兑换方鉴权。
- 联动修改 §0.2、§0.4、§2、§3、§5.0（新增拒绝原因码、DID 联合两项契约）、SSO-02～09、SSO-11、SSO-13、SSO-14。
- 当时仍待用户确认：S-1、Q-01～05、Q-10、P-1、P-2、D-8a；已在下方第二轮回写中确定。
- 本轮仍只改实施计划。新核对的源码：verify-hub kernel spec 实例数、workspace `rusqlite`、服务数据目录约定、CP `auth.login` → `login_by_password` 调用链、msg-center `notify.*`、kRPC `RPCErrors`；未运行构建、测试或 DV。

第二轮决策回写（同日）：

- §4.2 改为已定：S-1、Q-02、Q-05、D-8a、P-1、P-2 采纳推荐；Q-01 定为 48 小时，v1 不可调整；Q-10 定为同客户共用账号（用户：这正是稳定二级 DID 的用途），复用由签发方显式指定。
- Q-03、Q-04 的原推荐只有约束、没有数值，按用户“按你推荐”由 review 补上：access 15 分钟、sudo 3 分钟、refresh 滑动 7 天、grace 10 秒均沿用现有常量；本地会话不设绝对上限，DID 联合会话 30 天后重新联合，续期不向 canonical 复核；首设密码后撤销其它专属邀请免密终端；删除未生效的 `remember_me`。canonical logout 不传播，记为已知风险，最长影响 30 天。
- 按用户新增的 TODO，正文把 EA / EB 改为“专属邀请 / 自助注册”，入口语境的 DID 写作“DID 登录”；术语说明放在文首，AC / Q 编号保留。随后 PRD 与 UserType.md 也已改用同一组名称，PRD 术语表注明早期称 EA / EB。
- 联动修改 §0.3、§0.4（新增 P-2 邀请收紧、删除 `remember_me` 两项）、§2、§3、§4.1、§5.0、SSO-01、SSO-05、SSO-06、SSO-09、SSO-10、SSO-14。
- 本轮仍只改文档；新核对 verify-hub 时长常量、`handle_user_invite_create` 的类型检查与 `remember_me` 的调用方；未运行构建、测试或 DV。
- 随后把已定结论回写到 PRD（升为 v0.2）：Q-01～Q-06、Q-10、P-1 写入 §5～§9 对应节，新增 §6.6 登录保持与撤销；账号权限、Guest 范围、对外联合资格、拒绝提示、重发责任一并写入；§13 标为已确定，Q-13 标为移交技术设计。D-6 / D-7 等纯技术决策只把用户可见后果写进 PRD，S-1、P-2 不在 PRD 范围。
