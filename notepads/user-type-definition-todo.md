# BuckyOS 用户类型定义 TODO

日期：2026-10-21。状态：§3 的 9 项决策与 §3.3 的 4 项补充决定已于 2026-10-21 全部拍板；§4 的 UT-01～UT-11 已于 2026-10-21 实施（未提交），实施记录与验证结果见 §6。External 的签发、续期、登出与对外联合断言随混合 SSO SSO-02、09 实现。

本文是混合 SSO 的前置工作：[混合 SSO 实施 TODO](hybrid-sso-implementation-todo.md) §0 的 D-1、D-2、D-3 以本文结论为准，相关条目已回写（UT-12）。

目标：明确系统中每种用户的定义——能否登录、可用哪些 App、能否改自己的 Profile / 密码、会出现在谁的通讯录里，以及权限从哪里来。verify-hub、scheduler / RBAC、Control Panel、msg-center、HomeStation、Desktop、WebSDK 对同一种用户必须给出一致的行为。

范围：用户类型、系统用户组及其通讯录投影、无账号记录的外部用户（External）。Agent、设备、App、系统服务走 `principal_kind`，不在本文范围；群内 Session Guest 角色和各服务的资源授权只在需要时说明边界。

依据：

- 产品：[UserType.md](../product/users_and_agents/UserType.md)、[Users & Agents PRD](../product/users_and_agents/Users_Agents_PRD.md)、[BuckyOS 混合 SSO PRD](<../doc/arch/verify-hub/BuckyOS 混合SSO PRD.md>)。
- 架构：[BuckyOS Security Arch](<../doc/arch/BuckyOS Security Arch.md>) 的 L0–L3 层级、[用户生命周期与权限](../doc/arch/10_user_lifecycle_and_permissions.md)、[RBAC 权限设计](<../doc/arch/verify-hub/BuckyOS RBAC权限设计.md>)、[Session Only Member 设计](<../doc/message_hub/Session Only Member 设计.md>)。
- 当前源码。文档之间以及文档与代码之间存在冲突，见 §2.5。

## 1. 已定方向（2026-10-21 讨论）

### 1.1 UserType 是互斥的逻辑 RBAC 组

每个有账号记录的用户恰好属于一个 UserType 分组。要换组就必须换类型，因此用户不会同时处于两个 UserType 分组中。UserType 只表达这一层含义，不承担“账号从哪里来”“用什么登录”等语义。

### 1.2 Limited 是零权限底座

Limited 用户默认没有任何权限，显式加入某个**系统用户组**后，才获得该组的权限。例如企业为客户建立 customer 组并给予定制权限，客户账号的类型是 Limited，再加入 customer 组。企业员工、未成年人等受限账号同样用 Limited 加不同的组表达。组能带来哪些权限见 T-3；v1 的组不带系统 RBAC 权限，已知限制见 §3.3-B。

### 1.3 External 是本 Zone 没有账号记录的已认证用户

External 的 DID 不在本 Zone，本 Zone 也没有该用户的任何 Profile。这类用户的权限通常来自更贴近应用的规则组合，而不是系统 RBAC。例如：

1. Alice 在自己的 Workspace 中配置“同事有编辑权限”。
2. Bob 通过 SSO 登录本 Zone。
3. Workspace 查询到 Bob 在 owner（Alice）的同事组中。
4. Bob 获得这个 Workspace 的编辑权限。

判定条件以“本 Zone 有没有该用户的账号记录”为准：持有外部全局 DID、但已被邀请入驻本 Zone 的用户（Security Arch L2）有账号记录，类型是 User 或 Limited，不是 External。

### 1.4 系统用户组由管理员在 Users & Agents 管理

系统用户组参与授权，由管理员在 Users & Agents 管理界面维护。它不是用户在 My Network 中维护的个人联系人分组。

### 1.5 区分不同用户的四个维度

1. 能否登录；
2. 能用的 App 列表；
3. 能否修改自己的 Profile / 密码 / 通讯录；
4. 会出现在哪些用户的通讯录中（由 1.6 的投影决定）。

### 1.6 通讯录 = 原始通讯录 + 系统用户组投影

用户看到的通讯录，是自己的原始通讯录加上系统用户组的投影。管理员决定哪些分组投影到哪些分组：例如属于“客服组”的用户，通讯录里会看到属于“客户组”的用户。用户没有权限修改自己的通讯录时，看到的通讯录只剩系统投影。App 的内部逻辑依赖通讯录里的“分组逻辑”时（例如 HomeStation 的联系人分组受众、T-5 的按 owner 联系人分组授权），投影会对其产生固定的影响。细节见 T-6 与 §3.3-D。

### 1.7 不是用户类型的概念

| 概念 | 当前落点 | 说明 |
| --- | --- | --- |
| 匿名访问者 | App 可用性中 `user_id="guest"` 且无账号记录；AI Workspace 的 `gt_` 令牌 → `guest:` 主体 | 见 T-9 |
| Session Guest | 群 / Session 的成员角色 | 资源角色；EA 客户、External 都可以获得 |
| 联系人与联系人分组 | My Network / msg-center 按 owner 分库的 contact | 个人通讯录，叠加系统用户组投影（1.6）；资源方可以引用它做授权（T-5），但它不是系统用户组 |
| Agent | 独立 AgentId DID，`principal_kind=Agent` | 归属某个用户 |
| 混合 SSO 的 EA / EB 账号 | 未实现 | 本地账号，类型为 Limited，权限经系统用户组或资源授权获得 |
| 混合 SSO 的 DID 访客 | 未实现 | 未入驻本 Zone 时即 External |
| Session Only 设计的 guest user | 未实现 | 按身份源选择：有本地账号时为 Limited，否则按 T-4 的范围处理 |

## 2. 现状（2026-10-21 源码核对）

### 2.1 类型定义与各层映射

`UserType` 定义在 [control_panel.rs](../src/kernel/buckyos-api/src/control_panel.rs)，共 5 个值：`Admin`、`User`、`Root`、`Limited`、`Guest`。scheduler 另有一个只有 `Admin`、`User`、`Limited` 的内部枚举（[scheduler.rs](../src/kernel/scheduler/src/scheduler.rs)）。它的 `From<String>` 和 [system_config_agent.rs](../src/kernel/scheduler/src/system_config_agent.rs) 的 `map_api_user_type` 都会把未识别的类型回退为 `User`。

| 类型 | 创建入口 | 能否登录 | RBAC 组 | sudo 组 | App 可用性组 | 其它行为 |
| --- | --- | --- | --- | --- | --- | --- |
| `Root` | 只在引导期创建；运行期不能创建、提升或变更 | verify-hub 拒绝 | 映射为 `admin` | `su_admin` | 无 | `require_admin` 视同 Admin |
| `Admin` | 引导期；`user.invite.create`；`user.change_type` | 能 | `admin` | `su_admin` | `admins` | 管理用户、App、Zone、任务 |
| `User` | `user.create`（只支持这一种）；邀请；改类型 | 能 | `users` | `su_users` | `users` | 只能读写自己名下的数据 |
| `Limited` | 只能经邀请或改类型产生 | 能 | `limited`，**没有任何策略** | 无 | `limited` | 默认不能自改密码；不能创建 Agent |
| `Guest` | 只能经邀请或改类型产生 | verify-hub 拒绝（`user_is_active` 排除） | **回退为 `users`** | `su_users` | `guest` | Desktop 把它显示为 Limited |

### 2.2 “组”的现状：没有系统用户组

- **RBAC 角色组**：只由 scheduler 按 UserType 生成 `g, <id>, admin/users/limited`。`system/rbac/policy` 由 scheduler 整份重建，管理员不能修改（10_user_lifecycle 记为防越权设计）。没有可由管理员定义的组。
- **联系人分组**：`UserContactSettings.groups`，新用户默认带 `users`（`user_mgr.rs` 的 `DEFAULT_USERS_GROUP`）。`user.invite.create` 的 `groups` 参数写进这里。它只用于通讯录，不参与鉴权。
- **Users & Agents PRD 的“默认权限组”**（§6.7 Settings、§9.2 邀请、§9.3 新建“设置默认权限组……创建完成后自动加入默认基础组”）在实现中被做成了上面的联系人分组。Desktop 用户详情里的“默认分组”是写死的 `zone-members`（`users-agents/datamodel/transforms.ts`）。
- **App 可用性**：组名写死为 `admins`、`users`、`limited`、`guest` 四个（`validate_availability_rules`），另可按单个用户设规则。

### 2.3 消费方：改类型或组时必须同步

| 模块 | 位置 | 当前判定 |
| --- | --- | --- |
| verify-hub | `require_active_user_settings`、`validate_refresh_principal`、`reject_root_*`（[main.rs](../src/kernel/verify_hub/src/main.rs)） | 登录与续期都读 `users/<sub>/settings`；排除 Root、Guest |
| scheduler | `scheduler.rs` 枚举；`system_config_agent.rs` 的 `map_api_user_type`、`update_rbac` | 生成 `g, <id>, admin/users/limited` 和 `su_*` 组 |
| RBAC 基础策略 | [rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs) | 只有 `admin`、`users` 的用户策略；`limited` 无策略 |
| system-config | `is_privileged_user_type`（[sys_config_service main.rs](../src/kernel/sys_config_service/src/main.rs)） | `admin`、`root` 视为特权用户 |
| App 可用性 | `system_group_for_user_type`、`validate_availability_rules`、`user_is_active`（[app_availability.rs](../src/kernel/buckyos-api/src/app_availability.rs)） | 见 §2.2；`guest` 同时表示匿名请求 |
| Control Panel | `sys_auth_backend.rs` 构造 `RpcAuthPrincipal`；`user_mgr.rs` 的 `parse_user_type`、`user.create`、`user.invite.create`、`user.change_type`、改密码检查、`require_admin`；`agent_mgr.rs`；`app_installer.rs`、`app_servcie_mgr.rs`、`zone_mgr.rs`、`task_ops.rs` | 构造请求主体时必须读到 `users/<name>/settings`，否则 `InvalidToken`；Admin / Root 判定分散；Limited、Guest 不能创建 Agent |
| msg-center | `main.rs` 的 `load_zone_user_contact_seeds` / `build_zone_user_seed` / `sync_zone_user_contacts`；`msg_center.rs` 的 `is_local_recipient`；`owner_session.rs` 的 `caller_identity` | **所有 Active 账号（不分类型）同步进每个用户和 system 的通讯录，并注册为本地收件人**；无 profile 时回退为 `did:bns:<username>`；DID 主机名是 Zone 主机名的子域即视为本地收件人；用户主体的 DID 经 `resolve_user_did` 从 profile 取得 |
| HomeStation | [users.rs](../src/frame/homestation/src/users.rs)；`audience.rs`、`stream.rs`、`auth.rs` | 列出全部 active 账号，只有 admin、root、user 可写 zone feed；对远端 DID 已有按 owner 联系人授权的实现（§2.4） |
| Desktop | `api/user_mgr.ts`（5 值）；`api/account.ts` 的 `isLimitedUserType`（limited、guest）；`users-agents/datamodel`（`ZoneUserType` 只有 3 值，guest 显示为 limited）；`NewUserWizard` 只建 user；`changeUserType`；`settings/data/privacy.ts`、`ZonePage.tsx`、`filebrowser/data/nfsp/permissions.ts` | 多处按字符串判断类型 |
| WebSDK | `account.ts`、`sdk_core.ts`、`verify-hub-client.ts` | `user_type` 是自由字符串；`sdk_core` 会读 token 的 `user_type` claim，但 verify-hub 目前不写这个 claim |

### 2.4 已有先例：HomeStation 对无账号记录 DID 的授权

- 认证：远端读者出示 reader proof（`auth.rs` 的 `verify_reader_proof`），由该 DID 授权的密钥签名，`aud` 绑定本 Zone。本 Zone 不需要该读者的任何记录。
- 授权：`stream.rs` 的 `reader_relations` 读 owner 通讯录中该 DID 的 `friend` 标记与 `tags`；`audience.rs` 的 `AudienceSpec::Group` 按 tag 放行。
- 这与 §1.3 的 External 模型一致，是 T-4、T-5 的参照。注意 HomeStation 把联系人的 `tags` 当分组用，而 `UserContactSettings` 同时有 `groups` 和 `tags` 两个字段。

### 2.5 文档现状（互相冲突）

- UserType.md：limit user 标为“规划中”，指不允许改密码、未成年人一类的可登录用户；guest 指“非登录用户（匿名）”；另有未实现的 friends（非系统登录用户，有 SNS 读、评论写权限，接近 §1.3 的 External）。
- Users & Agents PRD：Limited 是“受限的可登录用户”，用于临时 Desktop、企业员工、未成年人；新建和邀请时要设置“默认权限组”。
- Security Arch L3：Restricted / Local / Guest User，“Zone 内创建的本地或受限账号，无全局 DID”。
- 10_user_lifecycle：正确指出 limited 等于零权限；但说 Guest“无映射、无任何权限”，与代码不符（代码回退为 `users`）。
- Session Only 设计：“guest user”指有稳定身份、可登录的受限用户，与 UserType.md 的匿名 guest 同名不同义。
- 混合 SSO PRD：Guest 是无须注册的匿名访客体验，不是账号；DID 联合登录后目标 Zone“可以”保存必要的本地 Profile。

### 2.6 已知缺陷

1. Guest 账号经 scheduler 回退进入 `users` 组，同时 verify-hub 拒绝它登录，语义自相矛盾。
2. PRD 的“默认权限组”被实现成联系人分组，不参与鉴权；系统中没有可管理的系统用户组。
3. 通讯录同步与本地收件不区分用户：Limited 客户会出现在全员通讯录里；任何二级 DID（`did:bns:<id>.<zone>`）按主机名就被当成本地收件人。
4. Control Panel、verify-hub、msg-center 识别用户主体时都依赖账号记录，External 在这些路径上一律失败。
5. 类型判断分散在 Rust 与 TypeScript 的十余处，并且直接比较字符串。

## 3. 决策（2026-10-21 已定）

### 3.1 决策总表

| 编号 | 决策 | 决定 |
| --- | --- | --- |
| T-1 | 类型集合 | 有账号记录的类型为 Root、Admin、User、Limited；删除 Guest；External 是无账号记录的主体分类 |
| T-2 | 四个维度的默认值与覆盖方式 | 按 §3.2 的表；可用 App 由系统用户组或单用户规则覆盖；通讯录由 T-6 的投影决定；改密码按单用户覆盖 |
| T-3 | 系统用户组的形态 | 组不带系统 RBAC 策略，是可被 App 可用性、资源授权和通讯录投影引用的标签；仅管理员维护成员 |
| T-4 | External 的表示与 v1 支持范围 | token 主体为原 DID 并带 External 标记；无账号记录，只保存会话、封禁与审计记录；v1 只有 verify-hub、msg-center 群路径、AI Workspace、HomeStation 接受 |
| T-5 | 资源方按 owner 联系人分组授权 | 新增“owner 的联系人分组”类授权对象，按已认证 DID 精确匹配；实现归混合 SSO SSO-13 |
| T-6 | DID、通讯录投影、本地收件与对外联合 | Admin / User / Limited 沿用二级 DID；通讯录 = 原始通讯录 + 管理员配置的系统用户组投影；`is_local_recipient` 不再只看主机名；对外联合只为 Admin、User 签发 |
| T-7 | 创建入口与类型变更 | `user.create` 支持 User、Limited 并可选组；邀请限 User、Limited；改类型限 Admin / User / Limited 之间 |
| T-8 | Root 的定位 | 保留，定义为不可登录的 Zone owner 账号记录，为未来 owner 可变预留 |
| T-9 | 匿名访问的表达 | 不是用户类型；保留 App 可用性 `guest` 伪用户与 AI Workspace `guest:` 主体 |

### 3.2 各项细节

**T-1 类型集合**

- 决定：有账号记录的类型为 `Root`、`Admin`、`User`、`Limited`，删除 `Guest`。`External` 不是账号记录上的类型值，而是请求主体的一种分类：已认证、但本 Zone 没有账号记录（§1.3）。PRD 的 Guest 是匿名访问（T-9）；UserType.md 的 friends 由 External 加资源授权表达，不单列。
- 不采用：保留 `Guest` 作为“可登录但不在通讯录中的受限账号”；为 External 写入最小账号记录。
- 影响：§2.3 全部消费方；混合 SSO §0 D-1、D-3。

**T-2 四个维度的默认值与覆盖方式**

| 维度 | Root | Admin | User | Limited | External |
| --- | --- | --- | --- | --- | --- |
| 能否登录 | 否（最高权限来自 Owner Key） | 能 | 能 | 能 | 只能经 DID 联合登录（混合 SSO SSO-09） |
| 可用 App | 不适用 | 全部 | 按现有动态规则 | 无，由系统用户组或单用户规则开放 | 无；只能进入资源方开放的入口 |
| 改 Profile / 密码 / 通讯录 | 不适用 | 能 | 能 | Profile 能；密码、通讯录默认否 | 不适用（本地无 Profile、密码、通讯录） |
| 出现在谁的通讯录中 | 不适用 | 默认投影规则下互见 | 默认投影规则下互见 | 默认不出现，由投影规则开放 | 不出现 |

- 覆盖方式：
  - 可用 App：系统用户组可以出现在 App 可用性规则中，另保留现有的单用户规则。
  - 通讯录：由 T-6 的系统用户组投影规则决定。原推荐的“组携带进入通讯录开关”按 T-6 决定改为投影。
  - 改密码：保留现有单用户 `allow_password_change`。
  - 改通讯录：按单用户覆盖（§3.3-C）。
  - 能否登录：由账号状态与凭据决定；凭据状态字段由混合 SSO SSO-01 定义。
- 不采用：维度全部由类型决定，不允许组覆盖。
- 补充决定：Limited 改 Profile 的 RBAC 依据见 §3.3-A；改通讯录见 §3.3-C。
- 影响：msg-center 通讯录与本地收件（T-6）、App 可用性、Desktop 的用户详情与新建向导。

**T-3 系统用户组的形态**

- 3a 组携带什么：组不带系统 RBAC 策略，是一个可被引用的身份标签。App 可用性规则可以选它；资源方授权可以指向它（例如 AI Workspace 授权给 `group:customer`，Message Hub 某会话允许该组）；通讯录投影规则以它为来源或目标（T-6）。组本身不带维度开关。由此带来的已知限制见 §3.3-B。
- 3b 成员关系与存储：UserType 组保持互斥；系统用户组可多选，Admin、User、Limited 都能加入，Root 不参与。组定义存 `system/user_groups/<id>`；成员关系写在用户 settings 的 `groups` 字段；scheduler 生成 `g, <user>, group:<id>`，`group:` 前缀避免与 `admin`、`users`、`limited` 及 `kernel`、`app` 等系统角色重名；App 可用性放开写死的四个组名。
- 3c 谁能把人放进组：只有管理员，经 Users & Agents 界面或具备管理员权限的脚本（如企业 EA 预建账号的脚本）。EB 公开注册不自动加入任何系统组，DJ 场景的权限用 AI Workspace 资源授权表达。
- 3d 现有“组”字段的处置：`user.invite.create`、`user.create` 的 `groups` 参数改为指系统用户组；`UserContactSettings.groups` 的默认 `users` 分组删除，Zone 成员互见改由默认投影规则表达（T-6）；删掉 Desktop 写死的 `zone-members`。
- 不采用：组携带从“权限目录”挑选的系统权限项；成员关系存在组记录里；分享入口可自动把人加入组。
- 影响：Users & Agents PRD 的“默认权限组”与“默认基础组”；Control Panel 新增组管理 RPC；scheduler；App 可用性；Desktop。

**T-4 External 的表示与 v1 支持范围**

- 决定：
  - 表示：只能经 DID 联合登录产生。verify-hub 签发的本 Zone token 以原 DID 为主体，并带 External 标记（claim 名在混合 SSO SSO-02 冻结）。
  - 状态：本 Zone 不保存 Profile。允许保存的只有 verify-hub 会话记录、按 DID 的封禁记录和审计记录。名字与头像从该 DID 的公开文档解析。PRD 中“可以保存必要本地 Profile”的选项按“不保存”处理。
  - 服务端路径：新增共享的主体解析函数，返回“有账号记录的用户”或“External DID”。verify-hub 续期、登出按 External 分支处理，不读账号记录。
  - v1 开放：verify-hub（续期、登出）、msg-center 群路径（以 DID 作为 `GroupActor`）、AI Workspace、HomeStation。其余服务默认拒绝 External，依赖混合 SSO D-4 的统一闸口。
  - 系统 RBAC：External 没有账号记录，scheduler 不会为它生成任何组，系统 RBAC 天然全拒。
- 不采用：External 写入最小账号记录。
- 影响：混合 SSO SSO-01、02、04、09、12、13。

**T-5 资源方按 owner 联系人分组授权**

- 决定：
  - AI Workspace 等资源方新增“owner 的某个联系人分组”这类授权对象。检查时以已认证 DID 向 msg-center 查询 owner 通讯录（含 T-6 的投影部分），需要一个窄接口（例如“某 DID 是否在 owner 的某分组中”），并给资源服务相应的服务间授权。
  - 只用已认证 DID 精确匹配，不使用联系人的 canonical 合并结果（Session Only 设计 §8：owner 私有合并不是授权证明）。
  - owner 修改通讯录即改变授权，这是预期行为，在分享 UI 上写明。
  - 统一联系人分组字段：`groups` 与 `tags` 二选一，HomeStation 与 AI Workspace 使用同一个。
  - 本文只定规则，实现归混合 SSO SSO-13。
- 不采用：v1 只支持按具体 DID 授权。
- 影响：混合 SSO SSO-13；HomeStation `reader_relations`；My Network 分组 UI。

**T-6 DID、通讯录投影、本地收件与对外联合**

- 决定：
  - DID：Admin、User、Limited 沿用现有二级 DID `did:bns:<id>.<zone>` 与 Zone 托管密钥。
  - 通讯录投影：用户看到的通讯录 = 自己的原始通讯录 + 系统用户组的投影。管理员决定哪些分组投影到哪些分组，例如“客服组”成员的通讯录中会看到“客户组”的成员。用户没有修改自己通讯录的权限时，看到的通讯录只剩系统投影。App 内部逻辑依赖通讯录分组时（HomeStation 的联系人分组受众、T-5 的授权），投影部分同样参与，会产生固定的影响。
  - 默认投影规则由 T-2 的默认值换算：Admin、User 两个类型组的成员互相投影，保持现在 Zone 成员互见的效果；Limited 与 External 不在默认规则中。这取代 msg-center 现在“所有 Active 账号同步进每个用户通讯录”的做法。
  - 本地收件：`is_local_recipient` 不再只看主机名后缀；修正无 profile 时回退为 `did:bns:<username>` 的问题。投影只决定通讯录可见性，不授予个人收件能力（§3.3-D）。不出现在任何投影中的 Limited 用户（如企业客户）不注册本地投递，经群存储参与会话（混合 SSO SSO-12）。
  - 对外联合：verify-hub 对外联合断言只为 Admin、User 签发；Limited 不签发，使目标 Zone 无法借本地受限账号向第三方 Zone 证明身份。
- 不采用：按组开关决定是否进入全员通讯录；Limited 的对外联合由组开关控制。
- 影响：混合 SSO §0 D-2、SSO-01 第 1、4 条、SSO-09 第 6 条、SSO-12；msg-center 通讯录；My Network；HomeStation。

**T-7 创建入口与类型变更**

- 决定：
  - 引导期：Root + Admin，不变。
  - `user.create`（管理员）：支持 User、Limited，可同时指定系统用户组；Admin 仍经改类型提升。满足 Users & Agents PRD 的本地 Limited 创建流程。
  - `user.invite.create`：只允许 User、Limited（是否允许直接邀请 Admin 保持现状），`groups` 按 T-3 指系统用户组。
  - External 不创建账号记录。若要让某个 External 成为本 Zone 成员，走现有邀请入驻流程，完成后即不再是 External。
  - `user.change_type`：只在 Admin、User、Limited 之间转换。类型或组变更后，旧会话不自动获得新权限。
  - Desktop 新建向导增加 Limited 与系统用户组选项；改类型界面只列可转换目标。
- 不采用：`user.create` 直接创建 Admin。
- 影响：Control Panel `user_mgr.rs`；Desktop `NewUserWizard`、`changeUserType`；`buckyos_admin` 手册。

**T-8 Root 的定位**

- 决定：保留 `Root` 类型，用于标记引导期的 `users/root` 记录。定义为“Zone owner 的账号记录，不可登录，不可创建或变更”；最高权限仍来自 Owner Key。Root 实际就是 Owner；单独保留 root 这个用户逻辑，是为了未来允许 Zone owner 变更。本期只补文档，不改代码。
- 不采用：删除 `Root` 类型和 `users/root` 记录。
- 影响：§2.3 中所有 Admin / Root 并列的判断（保持现状）；UT-10 文档。

**T-9 匿名访问的表达**

- 决定：匿名访问不是用户类型。保留 App 可用性里的 `guest` 伪用户（无账号记录的匿名请求）和 AI Workspace 的 `guest:` 资源主体，文档统一称“匿名访问者”。混合 SSO PRD 的 Guest 体验由后者承载。App 可用性组名是否从 `guest` 改为 `anonymous` 在 UT-05 实施时决定：改名能消除歧义，但要同步 Desktop 的 `app_mgr.ts` 和已保存的可用性策略。
- 不采用：verify-hub 为匿名访问者签发系统会话。
- 影响：混合 SSO §0 D-3、SSO-08。

### 3.3 补充决定（2026-10-21，按建议采纳）

以下几点由已定决策推出，或用于消除决策之间的张力。

- **A. Limited 改 Profile 的 RBAC 依据。** T-2 定 Limited 能改自己的 Profile，而 1.2 定 Limited 默认零权限。决定：给 `limited` 只加自身策略——读自己的 settings / profile、写自己的 profile，不含任何业务或资源权限。“零权限”指业务与资源权限。落点 UT-03。
- **B. 组不带系统 RBAC 策略的已知限制。** 按 T-3 3a，Limited 用户加入组后也得不到任何系统 RBAC 权限，例如个人邮箱（`obj://msg-center/inbox/*`）、自己的任务、上传。决定：v1 接受，列为已知限制；需要完整 Desktop 能力的人用 User 类型；以后若要组带系统权限，再启用 T-3 不采用的“权限目录”方案。写进 UT-10 文档。
- **C. 能否修改自己的通讯录。** 决定：作为维度 3 的一部分。默认值 Admin、User 能，Limited 否，可按单用户覆盖。没有该权限的用户看到的通讯录只有投影（T-6）。落点 UT-06、UT-08。
- **D. 投影规则的细节。** 决定：
  - 粒度：规则为“来源分组 → 目标分组”，单向；双向互见配两条规则。来源与目标可以是系统用户组，也可以是 UserType 组（默认规则需要）。
  - 呈现：投影条目在接收者通讯录中显示在以来源分组命名的分组下；接收者不能删除或编辑投影条目，只能在原始通讯录中另加备注。
  - 计算方式：读取时合成（原始通讯录 + 投影），不把投影写进每个人的原始通讯录，避免规则变更后残留。
  - 本地收件：投影只决定通讯录可见性，不授予个人收件能力；向未注册本地投递的用户（如企业客户）发私信时明确失败，引导改用群会话。
  - 落点 UT-02、UT-06、UT-08。

## 4. 决策后的实施任务

开发阶段按 AGENTS.md 不做旧数据兼容，但实施前要确认现有 DV Zone 中有没有 `type=guest` 的账号记录、以及联系人分组里的 `users` 默认分组，并给出处置方法。

- [x] **UT-01 共享类型**：`buckyos-api` 的 `UserType` 改为新集合；增加系统用户组与投影规则的共享类型；提供集中的判定函数（四个维度、是否特权、主体解析结果是账号还是 External），供各服务调用，替代散落的 `matches!` 与字符串比较。
- [x] **UT-02 系统用户组与投影规则**：组定义与投影规则的存储（T-3、T-6；投影规则按 §3.3-D 为单向的“来源分组 → 目标分组”，与组定义同存于 `system/user_groups/` 下）；Control Panel 管理 RPC（组的列出、创建、修改、删除、增删成员；投影规则的增删改），只允许管理员调用；scheduler 生成 `g, <user>, group:<id>`；App 可用性接受系统用户组；默认投影规则（Admin、User 互相投影）。
- [x] **UT-03 scheduler 与 RBAC**：删除自有 `UserType` 和 `map_api_user_type` 的 `_ => User` 回退，未知类型不进任何组并告警；按 §3.3-A 在 `rbac_config.rs` 给 `limited` 只加自身策略：`p, limited, obj://config/users/{limited}/settings, read`、`p, limited, obj://config/users/{limited}/profile, read|write`（`{limited}` 与现有 `{users}` 一样由 `keyGet3` 绑定到请求主体）；系统用户组不生成 `p` 规则。
- [x] **UT-04 verify-hub**：登录资格按类型 + 状态 + 凭据判定；External 的续期、登出不依赖账号记录（与混合 SSO SSO-02、09 一起实现）；对外联合只为 Admin、User 签发（T-6）。本期完成第一项（空密码不能登录）；后两项 verify-hub 尚无对应签发路径，判定 `UserType::can_federate_out` 已备好，随 SSO-02、09 实现。
- [x] **UT-05 Control Panel**：按 T-7 修改 `user.create`、`user.invite.create`、`user.change_type`；`groups` 参数语义；构造 `RpcAuthPrincipal` 时 External 明确拒绝（v1 不开放 Control Panel 业务方法）；`require_admin`、Agent 创建、App 安装、任务、Zone 管理改用 UT-01 的集中判定；用户列表返回系统用户组；按 T-9 决定 App 可用性 `guest` 组名是否改为 `anonymous`。
- [x] **UT-06 msg-center**：通讯录改为读取时合成“原始通讯录 + 投影”（§3.3-D），取代全员同步；投影条目只读；无改通讯录权限的用户（§3.3-C）写通讯录被拒；向未注册本地投递的用户发私信明确失败；本地收件注册与 `is_local_recipient` 按账号记录与收件能力判断，不再只看主机名；修正无 profile 时的 DID 回退；`caller_identity` 支持 External DID（T-4）；删除联系人默认 `users` 分组；为 T-5 提供“某 DID 是否在 owner 某分组中（含投影）”的窄接口。
- [x] **UT-07 HomeStation**：用户列表只含通讯录可见的成员；zone feed 写权限按新定义；`reader_relations` 使用 T-5 统一后的分组字段，并包含投影分组。
- [x] **UT-08 Desktop**：Users & Agents 增加系统用户组与投影规则管理（仅管理员），用户详情可设置“允许修改通讯录”（§3.3-C）；用户详情展示类型与所在组，删除写死的 `zone-members`；新建向导支持 Limited 与组；改类型只列可转换目标；My Network 区分原始联系人与投影条目，投影条目按来源分组展示且只读，无改通讯录权限时隐藏编辑入口；`isLimitedUserType`、`privacy.ts`、`ZonePage.tsx`、`filebrowser` 的判断改用新定义。
- [x] **UT-09 WebSDK**：记录 `user_type` 的合法取值与 External 标记；若 token 携带类型或受限标记，同步解析逻辑。
- [x] **UT-10 文档**：UserType.md 重写为用户类型、系统用户组与通讯录投影的权威定义（含 T-8 的 Root 说明与 §3.3-B 的已知限制），其它文档引用它；同步修订 Users & Agents PRD（默认权限组、组与投影管理）、Security Arch L3、10_user_lifecycle（修正 Guest 映射描述）、RBAC 权限设计、Control_Panel_Service、SSO.md、Session Only 设计。
- [x] **UT-11 cmd_skills**：按 AGENTS.md 检查 `buckyos_admin` 手册，补充用户类型、系统用户组和投影规则的管理命令与示例，或写明无需修改的依据。
- [x] **UT-12 混合 SSO TODO 回写**（2026-10-21）：§0 的 D-1、D-2、D-3 改为已定并引用本文；§0.5 的 scheduler 回退修复指向 UT-03；§2 用户类型行、SSO-01、04、07、08、09、12、13 中与本文冲突的条目已按决定改写。

## 5. 验证

- Rust（在 `buckyos/src` 运行）：`cargo test -p buckyos-api -p verify_hub -p control_panel -p scheduler -p msg_center -p homestation -p system_config -- --test-threads=1`。新增用例覆盖：
  - 每种类型的 RBAC 组生成、未知类型不入组、系统用户组生成 `group:` 行；
  - 四个维度的默认值与覆盖（Limited 的 App 可用性、改密码、改通讯录）；
  - Limited 读自己的 settings、读写自己的 profile 成功，读写其它用户或系统配置被拒；
  - 组与投影规则管理 RPC 只允许管理员；
  - 通讯录合成：默认规则下 Admin、User 互见，Limited 默认不可见，配置“客户组 → 客服组”后客服可见客户、客户不可见客服；投影条目只读、删除规则后不残留；无改通讯录权限时只剩投影且写入被拒；
  - 向未注册本地投递的用户发私信明确失败；
  - 本地收件只含有收件能力的用户，二级 DID 不再按主机名自动成为本地收件人；
  - `change_type` 的允许与拒绝路径；
  - External 主体在未开放服务上被拒绝；对外联合不为 Limited 签发。
- Desktop（在 `src/frame/desktop` 运行）：`pnpm check`、`pnpm lint`；`deno test --no-check --allow-read --allow-env tests/datamodel/users-agents-mapping.test.ts`；`pnpm exec playwright test tests/e2e/pages/users-agents.spec.ts`，并新增系统用户组与投影规则管理用例。
- 真实 Zone（在仓库根目录运行）：`uv run src/check.py` 确认激活后，执行 `bash test/local_user_dv.sh` 与 `src/frame/desktop` 的 `tests/dv/local-user.spec.ts`。补充记录：Limited 账号在加入组前后登录 Desktop 的可用 / 不可用功能；配置投影规则后双方通讯录的实际内容；RBAC 策略生成前后的差异。
- 未执行的检查如实标为“未验证”。

## 6. 实施记录（2026-10-21）

### 6.1 落点

| 条目 | 主要文件 |
| --- | --- |
| UT-01 | `buckyos-api/src/control_panel.rs`（`UserType` 去 Guest + 集中判定方法；`UserSettings.groups` / `allow_contact_edit` 与 `is_active`、`can_change_password`、`can_edit_contacts`、`group_refs`；`UserGroupDirectory`、`ZoneUserPrincipal`、`ControlPanelClient::load_user_groups` / `resolve_user_principal`；删除 `UserContactSettings.groups`）、新文件 `user_groups.rs`（`UserGroupRef`、`ContactProjectionRule`、默认规则、`project_contacts`）、`app_availability.rs`（`match_availability_group_rule`、External 标记 claim） |
| UT-02 | Control Panel 新文件 `user_group_mgr.rs`（`user_group.*` 七个 RPC）、`user_mgr.rs`（`user.set_account_options`、`groups` 参数改指系统用户组）、`app_servcie_mgr.rs`（可用性规则引用的组必须已定义） |
| UT-03 | `scheduler/src/scheduler.rs`、`system_config_agent.rs`（`parse_user_settings_record`、`g, <user>, group:<id>`）、`rbac_config.rs`（`limited` 自身策略与 `owners/*/contacts`、`contact_membership` 两行）、`read_last_schedule_snapshot`、`sys_config_service` 的 `is_privileged_user_type` |
| UT-04 | `verify_hub/src/main.rs`（空存储密码拒绝密码登录） |
| UT-05 | `user_mgr.rs`、`sys_auth_backend.rs`（`reject_external_principal`）、`agent_mgr.rs`、`app_installer.rs`、`task_ops.rs`、`zone_mgr.rs`、`app_servcie_mgr.rs` 改用集中判定 |
| UT-06 | msg-center `contact_mgr.rs`（投影快照与读取合成）、`main.rs`（zone sync 构建目录、旧 `zone_user` 条目清理）、`msg_center.rs`（`ZoneAccounts`、本地收件、私信失败）、`owner_session.rs`（External）、`contact_rpc.rs`（写权限、`contact.check_membership`）；buckyos-api `msg_center_client.rs`（`Contact.projected_from`、`MailboxAddress::contacts_resource`）、`msg_contact.rs` |
| UT-07 | homestation `users.rs`、`contacts.rs` |
| UT-08 | Desktop `src/api/user_type.ts`（集中判定）、`api/user_mgr.ts`、`control_panel_mock.ts`、Users & Agents 系统用户组页与用户详情、新建向导、My Network 投影与只读模式、隐私页；e2e `tests/e2e/pages/user-groups.spec.ts` |
| UT-09 | websdk `account.ts`（`ZoneUserType`、`parseZoneUserType`、`isExternalPrincipal`）、`msg_center_client.ts`（`projected_from`、`isUserGroupRefTag`）及 dist |
| UT-10 | `product/users_and_agents/UserType.md` 重写为权威定义；Users & Agents PRD、Security Arch、10_user_lifecycle、RBAC 权限设计、Control_Panel_Service、SSO.md、Session Only 设计、Desktop 两份 My Network UI_DATAMODEL 同步 |
| UT-11 | `buckyos_admin.md` 补用户类型、系统用户组与投影 RPC；websdk 没有用户管理 CLI 模块，手册指向 Desktop 与 RPC |

### 6.2 实施时定下的细节

- External 标记：`principal_kind` 仍为 `user`，另带布尔 claim `external: true`（`TOKEN_EXTERNAL_CLAIM`）。没有采用 `principal_kind=external`：不认识它的服务会落入“非 User 主体放行”分支；现在这类服务按普通用户查账号记录，查不到即拒绝。已回写混合 SSO SSO-02。
- 投影规则存 `system/user_groups/_projections`（组 id 必须以字母开头，不会冲突）；该 key 不存在即默认规则（`type:admin`、`type:user` 两两互投，含自身）。
- App 可用性：精确用户规则 > 系统用户组规则（多个组之间 allow 优先）> UserType 组规则 > 策略默认。匿名伪组名保持 `guest`（T-9 的改名不做：`Guest` 类型删除后已无歧义，改名还要迁移已保存的策略）。
- 本地收件按类型：只有 Active 的 Admin、User 注册为本地收件人（Limited 没有邮箱 RBAC，见 §3.3-B）；Zone 账号不再按主机名算本地，非账号的子域 DID（Agent、telegram 别名）保持原规则。Zone 托管签名仍覆盖 Limited。
- 合成通讯录的访问级别：原始记录是 Block 时保持 Block，否则投影条目一律 Allow。旧行为里用户把 Zone 成员手动设成 Stranger 会被保留，现在不再生效，只有拉黑能挡住。
- §3.3-A 的补充：DV 发现 Limited 读不到自己的（只含投影的）通讯录——msg-center 原先要求整个 owner 作用域 `obj://msg-center/owners/<did>` 的权限。改为通讯录单独用 `obj://msg-center/owners/<did>/contacts`，`limited` 加 `read|write` 到该资源；msg-center 仍把 owner 绑定到调用者本人，并按 `can_edit_contacts` 拦写，所以单用户覆盖“允许改通讯录”对 Limited 也生效。会话、邮箱等其它 owner 数据仍不开放。
- DV 发现：scheduler 的 `system/scheduler/snapshot` 存着旧 `UserItem`（自有枚举序列化为 `"User"`），换成 buckyos-api 的 `UserType` 后每轮调度都反序列化失败，RBAC 不再刷新。改为快照读不了时告警并按无快照重新调度（`read_last_schedule_snapshot`），快照只是上一轮的缓存。
- 投影条目不带 `UserContactSettings.tags`（Zone 给用户的建议标签）；读取时合成的模型里它没有落点。
- 旧数据：DV 上没有 `type=guest` 记录；alice、bob 的 profile `system_contact.groups = ["users"]`、devtest 的 `["zone_user"]` 因字段删除在读取时被忽略，下次写 profile 时消失，无需迁移。msg-center 启动后对每个用户库清理一次旧的同步 `zone_user` 条目：带用户数据（备注、自加标签、拉黑）的保留为普通联系人，其余删除。

### 6.3 验证

- Rust（`buckyos/src`）：`CARGO_INCREMENTAL=0 cargo test -p buckyos-api -p verify_hub -p control_panel -p scheduler -p msg_center -p homestation -p system_config -- --test-threads=1` 全部通过：buckyos-api 256、control_panel 147、msg_center 170（1 ignored）、scheduler 74、system_config 34、verify_hub 19、homestation 18 及各集成测试，0 失败（scheduler、msg_center 在 DV 修复后又各自重跑通过）。
- Desktop（`src/frame/desktop`）：`pnpm check` 通过；`pnpm lint` 0 错误（9 个原有警告）；`deno test --no-check --allow-read --allow-env tests/datamodel/users-agents-mapping.test.ts` 4 通过；`pnpm exec playwright test tests/e2e/pages/user-groups.spec.ts` 7 通过（浅色、深色截图已核对）；分支另跑 users-agents、my-network、messagehub，49 个中 48 过，失败的 `messagehub.spec.ts`“wheel scrolling 375px”单独连跑 3 轮全过。
- websdk：`pnpm test` 505 通过，`pnpm build` 重建 dist。
- DV（test.buckyos.io，部署 system_config、verify_hub、scheduler、control_panel、msg_center、homestation、desktop）：
  - `bash test/user_groups_dv.sh`（新增）通过：RBAC 行 `g, <u>, limited` / `group:<id>` 生成且无组策略；Limited 能读自己的 settings/profile，读别人 settings、系统配置、改密码、管理组、建 Agent、列自己的会话都被拒；默认规则下 User 看不到 Limited、Limited 通讯录为空；配置 `group:customer → group:service` 后客服看到客户（`projected_from` 与 tags 带来源组）、客户看不到客服；删除投影条目被拒（`system-relation`）；Limited 写通讯录被拒（`contact-edit-not-allowed`）；给 Limited 发私信 `post_send` 返回 `recipient-has-no-inbox`；删规则后投影条目消失；移出组后 RBAC 行消失，改类型拒绝 root/guest、改为 user 后 RBAC 更新。
  - `bash test/local_user_dv.sh` 通过；`test/test_msg_center/test_my_network.ts` 通过。
  - `tests/dv/local-user.spec.ts`：“SSO callback”通过；“admin creates a real local user …”的功能断言全部通过（顺带把早已过时的 chip 断言 `user`/`active` 改为界面实际显示的 `User`/`Active`），最后的控制台检查因 9 条“SSL certificate error when fetching the script”失败，是本机自签证书下 Service Worker 脚本拉取失败，与本次改动无关。
  - `test_messagehub_sessions.ts`：第 1 步通过；第 2 步要求 devtest 名下有 jarvis Agent，这台 DV 上不存在，未能验证。
  - DV 过程中发现并修复两处：Limited 读不到自己的通讯录、scheduler 旧快照反序列化失败（见 §6.2）。

### 6.4 未完成与风险

- External 的签发、续期、登出、对外联合断言与 msg-center 群存储收件（SSO-02、09、12）未实现；External 相关代码只有单测。
- Limited 用户经群会话收发消息依赖 SSO-12：其 `post_send` 目前因无 msg-center RBAC 被拒。
- 资源方按 owner 联系人分组授权（T-5）只提供了 msg-center 窄接口 `contact.check_membership`，AI Workspace 侧的授权对象随 SSO-13 实现。
- Desktop 里当前账号能否改通讯录、组名在页面内只读一次；管理员修改后，被改的用户要刷新页面。Desktop 本来没有编辑 App 可用性的界面，只改了展示。
- “按 `<名>.<zone>` 识别账号”的分支依赖 runtime，没有单测。

