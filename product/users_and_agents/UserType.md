# 用户类型、系统用户组与通讯录投影

本文是用户类型的权威定义：每种用户能否登录、能用哪些 App、能否改自己的 Profile / 密码 / 通讯录、会出现在谁的通讯录里，以及权限从哪里来。verify-hub、scheduler / RBAC、Control Panel、msg-center、HomeStation、Desktop、WebSDK 对同一种用户给出一致的行为。决策过程见 `notepads/user-type-definition-todo.md`（2026-10-21）。

## 1. 主体分类

| 分类 | 有无账号记录 | 说明 |
| --- | --- | --- |
| UserType：Root / Admin / User / Limited | 有（`users/<id>/settings`） | 互斥的逻辑 RBAC 组，每个账号恰好属于一个；换组就是换类型 |
| External | 无 | 已认证、但本 Zone 没有账号记录的 DID |
| 匿名访问者 | 无 | 没有登录态的请求，不是用户类型 |

判定 External 只看本 Zone 有没有该用户的账号记录：持有外部全局 DID、但已被邀请入驻本 Zone 的用户有账号记录，类型是 User 或 Limited，不是 External。

以下概念都不是用户类型：

| 概念 | 落点 |
| --- | --- |
| 匿名访问者 | App 可用性中的 `guest` 伪组（`user_id="guest"` 且无账号记录）；AI Workspace 的 `gt_` 令牌 → `guest:` 资源主体 |
| Session Guest | 群 / Session 的成员角色；Limited 客户、External 都可以获得 |
| 联系人与联系人分组 | My Network / msg-center 按 owner 分库的 contact，分组字段统一为 `tags`；叠加系统用户组投影（§5）；资源方可以引用它授权，但它不是系统用户组 |
| Agent | 独立 AgentId DID，`principal_kind=agent`，归属某个用户 |
| 混合 SSO 的专属邀请 / 自助注册账号 | 本地账号，类型为 Limited |
| 混合 SSO 的 DID 访客 | 未入驻本 Zone 时即 External |

## 2. UserType

### 2.1 四个维度的默认值

| 维度 | Root | Admin | User | Limited | External |
| --- | --- | --- | --- | --- | --- |
| 能否登录 | 否（最高权限来自 Owner Key） | 能 | 能 | 能 | 只能经 DID 联合登录 |
| 可用 App | 不适用 | 全部 | 按现有动态规则 | 无，由系统用户组或单用户规则开放 | 无；只能进入资源方开放的入口 |
| 改 Profile / 密码 / 通讯录 | 不适用 | 能 | 能 | Profile 能；密码、通讯录默认否 | 不适用（本地无 Profile、密码、通讯录） |
| 出现在谁的通讯录中 | 不适用 | 默认投影规则下互见 | 默认投影规则下互见 | 默认不出现，由投影规则开放 | 不出现 |

覆盖方式：

- 能否登录：账号状态为 Active 且类型不是 Root；密码登录还要求已保存密码凭证（空密码不能登录）。
- 可用 App：App 可用性规则可以选系统用户组（`group:<id>`），另保留单用户规则（§4.3）。
- 改密码、改通讯录：单用户覆盖 `allow_password_change`、`allow_contact_edit`（`user.set_account_options`，`null` 恢复类型默认）。
- 通讯录可见性：由投影规则决定（§5）。

### 2.2 Root

Root 就是 Zone owner 的账号记录（`users/root`），不可登录，不可创建、提升或变更，不加入系统用户组，也不生成 RBAC 组。最高权限来自 Zone 外的 Owner Key（精确主体名 `root`），系统不能真的“以 root 登录”。单独保留 root 账号逻辑，是为了未来允许 Zone owner 变更。

相比 root，admin 即使 sudo 也少了这些权限：

- 无法调整 zone-config（最重要的是 OOD 与 gateway 配置）；
- 无法操作用户自己的链上资产；
- 无法解密用 root 密钥参与加密的数据（通常是敏感备份）。

### 2.3 Admin / su_admin

可登录用户中权限最大者，Zone owner 日常也用它登录。能读系统配置和自己的 home 目录，不能读其它用户的数据；管理用户、App、Zone 与任务。敏感操作需要 sudo（verify-hub 能签发的最高权限，需要再次输入密码）。RBAC 组 `admin`，sudo 组 `su_admin`。

### 2.4 User / su_user

最常见的可登录用户：使用系统和 App，只能读写自己名下的数据，不能管理系统。对使用二级 DID 的用户来说，这是主要情况。二级 DID 也可以是管理员——权限组是权限组，鉴权是鉴权。RBAC 组 `users`，sudo 组 `su_users`。

### 2.5 Limited

零权限底座。Limited 用户默认没有任何业务与资源权限，RBAC 组 `limited` 只有自身策略：读自己的 `users/<id>/settings`、读写自己的 `users/<id>/profile`，以及自己的通讯录（`obj://msg-center/owners/*/contacts`，msg-center 再把 owner 绑定到调用者本人，并按“能否改通讯录”拦写操作），不含会话、邮箱等其它 owner 数据。管理员把它加入系统用户组后，经 App 可用性、资源授权和通讯录投影获得能力。企业客户（Limited + customer 组）、企业员工、未成年人、临时 Desktop 账号都用 Limited 加不同的组表达。

Limited 的会话是受限会话（token 带 `restricted: true` 与 `scopes`，混合 SSO 2026-10-22 起）：系统服务默认拒绝，只有显式开放的方法可用——本人 settings / profile / 通讯录 / 改密 / 桌面状态（`account_self`）、msg-center 群路径、AI Workspace、HomeStation。类型升级不会让已签发的会话增权，要获得新权限须重新登录。Limited 不签发 sudo。

已知限制（v1 接受）：系统用户组不带系统 RBAC 策略，Limited 加入组后仍不能使用个人邮箱（`obj://msg-center/inbox/*`）、自己的任务、上传等系统能力，也不注册个人消息投递，只经群会话参与消息。需要完整 Desktop 能力的人应使用 User 类型。以后如果要让组带系统权限，再引入“权限目录”。

## 3. External 与匿名访问者

### 3.1 External

- 只能经 DID 联合登录产生。verify-hub 签发的本 Zone token 以原 DID 为主体（`sub`），`principal_kind` 仍为 `user`，另带 `external: true` 标记（`TOKEN_EXTERNAL_CLAIM`）与受限会话上限（不含 `account_self`），联合会话最长 30 天。不认识该标记的服务按普通用户查账号记录，查不到即拒绝，不会落入“非 User 主体放行”的分支。
- 本 Zone 不保存 External 的 Profile，只允许保存 verify-hub 会话记录、按 DID 的封禁记录和审计记录；名字与头像从该 DID 的公开文档解析。
- 服务端用共享的主体解析（`ControlPanelClient::resolve_user_principal`）得到“有账号记录的用户”或“External DID”。
- v1 只有 verify-hub（续期、登出）、msg-center 群路径（以 DID 作为 `GroupActor`）、AI Workspace、HomeStation 接受 External；Control Panel 等其余服务一律拒绝。External 没有账号记录，scheduler 不为它生成任何组，系统 RBAC 天然全拒。
- 要让某个 External 成为本 Zone 成员，走邀请入驻流程，完成后即有账号记录、不再是 External。

UserType.md 早期的 friends（非系统登录用户，有 SNS 读、评论写权限）由 External 加资源授权表达，不单列类型。

### 3.2 匿名访问者

匿名访问不是用户类型，verify-hub 不为它签发系统会话。系统默认拒绝匿名访问，只有明确公开的入口允许：App 可用性里的 `guest` 伪组规则，以及 AI Workspace 公开链接的 `guest:` 资源主体（混合 SSO PRD 的 Guest 体验由后者承载）。

## 4. 系统用户组

### 4.1 形态

系统用户组由管理员在 Users & Agents 中维护，不是用户在 My Network 中维护的个人联系人分组。组本身不带系统 RBAC 策略，是可被引用的身份标签：

- App 可用性规则可以选它；
- 资源方授权可以指向它（例如 AI Workspace 授权给 `group:customer`，Message Hub 某会话允许该组）；
- 通讯录投影规则以它为来源或目标（§5）。

组引用的字符串形式：`group:<id>` 表示系统用户组，`type:admin`、`type:user`、`type:limited` 表示 UserType 组（投影规则需要）。

### 4.2 存储与成员关系

- 组定义：`system/user_groups/<id>`（`UserGroup`：`group_id`、`name`、`description`），`id` 为 `[a-z][a-z0-9_-]`，最长 32 个字符。
- 成员关系：用户 settings 的 `groups` 字段，可多选；Admin、User、Limited 都能加入，Root 不参与。
- RBAC：scheduler 为每个已定义的组生成 `g, <user>, group:<id>`，不生成任何 `p` 规则。`group:` 前缀避免与 `admin`、`users`、`limited` 及 `kernel`、`app` 等系统角色重名。
- 只有管理员能把人放进组：经 Users & Agents 界面，或具备管理员权限的脚本（如企业为专属邀请预建账号）。自助注册不自动加入任何系统组，分享入口也不能自动把人加入组。
- 管理 RPC（Control Panel，仅管理员）：`user_group.list`、`user_group.create`、`user_group.update`、`user_group.delete`、`user_group.set_members`、`user_group.set_projections`、`user_group.reset_projections`；`user.create` / `user.invite.create` 的 `groups` 参数指系统用户组。组或成员变化后 Control Panel 立即刷新 RBAC 并通知 msg-center。

### 4.3 App 可用性中的组

规则判定顺序：App owner 隐式允许 → 精确用户规则 → 系统用户组规则（用户所在的多个组之间 allow 优先）→ UserType 组规则（`admins` / `users` / `limited`）→ 策略默认。匿名请求只看 `guest` 伪组规则。

## 5. 通讯录投影

用户看到的通讯录 = 自己的原始通讯录 + 系统用户组投影。

- 规则：单向的“来源组 → 目标组”，来源组成员出现在目标组成员的通讯录里；双向互见配两条规则。来源与目标可以是系统用户组，也可以是 UserType 组。规则存 `system/user_groups/_projections`。
- 默认规则（该 key 不存在时）：Admin、User 两个类型组的成员互相投影，保持 Zone 成员互见；Limited 与 External 不在默认规则中。例如配置 `group:customer → group:service` 后，客服能看到客户，客户看不到客服。
- 呈现：投影条目在接收者通讯录中显示在以来源组命名的分组下（条目 `tags` 含来源组引用，`projected_from` 列出来源组）；接收者不能删除或编辑投影条目，只能在原始通讯录中另加备注。
- 计算：msg-center 读取时合成，不把投影写进每个人的原始通讯录，规则变更后不残留。
- 改通讯录的权限：维度 3 的一部分，Admin、User 默认能，Limited 默认否，按单用户覆盖。没有该权限的用户看到的通讯录只有投影，写通讯录被拒。
- 影响：App 内部逻辑依赖通讯录分组时（HomeStation 的联系人分组受众、资源方按 owner 联系人分组授权），投影部分同样参与，会产生固定的影响。
- 本地收件：投影只决定通讯录可见性，不授予个人收件能力。只有 Admin、User 账号注册为 msg-center 本地收件人；向未注册本地投递的用户（如 Limited 客户）发私信明确失败，引导改用群会话。DID 主机名是 Zone 子域不再自动视为本地收件人。

### 5.1 资源方按 owner 联系人分组授权

AI Workspace、HomeStation 等资源方可以把“owner 的某个联系人分组”作为授权对象：检查时以已认证 DID 向 msg-center 查询 owner 通讯录（含投影部分），只用已认证 DID 精确匹配，不使用联系人的 canonical 合并结果。owner 修改通讯录即改变授权，这是预期行为，分享 UI 上要写明。

## 6. 创建入口与类型变更

- 引导期：Root + Admin。
- `user.create`（管理员）：支持 User、Limited，可同时指定系统用户组与改密码 / 改通讯录覆盖；Admin 只能经改类型提升。
- `user.invite.create`：类型为 User 或 Limited（混合 SSO P-2，2026-10-21），Admin 经 `user.change_type` 提升；`groups` 指系统用户组。
- `user.change_type`：只在 Admin、User、Limited 之间转换。类型或组变更后，旧会话不自动获得新权限。
- External 不创建账号记录。

## 7. DID 与对外联合

- Admin、User、Limited 沿用二级 DID `did:bns:<id>.<zone>` 与 Zone 托管密钥（本地账号），或入驻时绑定的外部 DID。
- verify-hub 的对外联合断言只为 Admin、User 签发（`UserType::can_federate_out`）；Limited 不签发，使目标 Zone 无法借本地受限账号向第三方 Zone 证明身份。

## 8. 代码中的集中判定

不要在服务里直接比较类型字符串，使用 `buckyos-api` 的集中判定：

| 判定 | 落点 |
| --- | --- |
| 类型解析、是否特权、RBAC 组、能否登录、能否建 Agent、能否对外联合、能否接收私信、改密码 / 改通讯录默认值 | `UserType` 方法（`control_panel.rs`） |
| 账号是否有效、生效的改密码 / 改通讯录权限、所在组引用 | `UserSettings::is_active` / `can_change_password` / `can_edit_contacts` / `group_refs` |
| 组引用、投影规则、默认规则、按查看者计算投影 | `user_groups.rs`（`UserGroupRef`、`ContactProjectionRule`、`project_contacts`） |
| 组目录与投影规则读取、用户主体解析（账号 / External） | `ControlPanelClient::load_user_groups`、`resolve_user_principal` |
| App 可用性的组规则 | `app_availability.rs` 的 `match_availability_group_rule` |

Desktop 与 WebSDK 的类型判断同样集中在各自的判定模块。

## 让某个(传统)app提供公共服务的方法：

- app设置为公共访问(gateway）不会拦截 ：这在buckyos看来是让app拥有了特殊权限
- app可拥有自己的用户管理组件
- app基于自己的权限管理系统管理信息的读写，单能管理的信息局限在app data范围内

## 用户是否有顶级bns name?

不管有没有顶级的bns name,用户账号都可以用did表达
doc_type = user 用户的公共profile
doc_type = owner 用户作为zone-owner的profile, 不一定是公开的（可以加密了）

系统不在意用户是否有bns name，因为这取决于用户向外分析自己的方法
如果用户用的是  did:bns:$username, 分享自己的个人主页，那么于用户进行交互的时候，就要经历一个resolve_default_zone的过程
如果用户用的是 did:bns:$username.$zonename的方法，那么就明确的对外说在这个zone上可以拿到自己的个人资料，并且说明自己是一个子账号

zone内是否保留用户的私钥，也是产品选择

## 用户如何“声明一个内容”是我创建的（用户的私钥如何使用）

- 用root密钥签名 （100%手工，效力最强）
- 用controller密码签名（这个密钥是否放到zone上外部不太介意，半手工）
- 用bind-zone的controller密码签名 （日常普通信息发布）

推论：除非用户是二级did,否则都需要通过更新自己的ownerconfig来让zone拥有可用的私钥

## zone外登陆(SSO)

用户的binded zone为current-zone
要验证用户登陆的zone为target-zone

1）使用钱包登陆
2）使用用户的current-zone登陆
3）不在target-zone上建账号：联合登录后即是 External（无账号记录），权限只来自资源方授权；需要成为本 Zone 成员时走邀请入驻

## 通过did或的用户的profile

profile是一个json

1）先从current-zone上获得profile
2) 获得用户保存在bns上的profile(不依赖任何zone)

合并json，如果都有的字段以BNS上的为准。

> 普通用户总是拥有100%的权利，基于root密钥来修改自己的profile

## 用户管理相关的核心流程

### 新建

- 在system_config中添加必要的配置
  - user settings: 登录密码、用户类型、所在系统用户组
  - user profile: 社交网络类的信息
- 普通用户:更新自己在BNS上的zone-binded
- 二级用户:自动创建全套的OwnerConfig资料 
- 在DFS上创建用户数据目录（一般是首次使用时按需创建的）


### 开放注册

允许did符合条件的用户自助完成账户开通的流程，开通后可以正常在系统中登陆
(目前暂不支持）

### 登陆

登录的目标，是让 Client 得到一个可以保存并用于后续请求的 Session Token。

Session Token 表示一个已经完成认证的用户会话，通常包含以下信息：

1. 当前用户身份，例如 User ID / DID。
2. 当前 App ID。
3. 过期时间、nonce 等会话属性。
4. 签发方信息，通常是 Verify Hub。

业务服务拿到 Session Token 后，只需要验证 token 并结合 RBAC 做权限判断，不需要理解用户最初是通过密码、钱包、Passkey 还是其它凭证完成登录的。因此，对鉴权方来说，登录方式的扩展应当是透明的。

从结果上看，系统需要支持两类登录方式。

第一类是直接使用钱包或私钥签发凭证。用户可以基于自己的 Root Key / Control Key，通过钱包私钥签发可被系统接受的 Session Token 或登录凭证。这种方式安全敏感，尤其不适合随意签发长效 token；但如果钱包自己管理了相应的安全策略，系统层面可以接受这种由高权限私钥证明的身份。

第二类是通过 Verify Hub 登录。这里的登录流程是日常主路径：用户向 Verify Hub 提交某种登录凭证，Verify Hub 验证通过后，签发标准 Session Token。后续系统服务只关心这个 token 是否可信，以及 token 中声明的用户、App 和权限上下文。

Verify Hub 登录页需要支持用户输入完整 DID，也可以支持输入系统内的 username。为了避免冒名或伪造身份，登录输入最终必须能映射到一个明确的 DID。Verify Hub 会读取 User Settings 和用户状态，判断该用户是否存在、是否允许登录，以及是否具备当前登录方式所需的凭证。

当前最基础的方式是用户名 / DID + 密码登录。只要 User Settings 中存在对应的密码凭证，并且用户状态允许登录，Verify Hub 就可以验证凭证并签发 Session Token。

Verify Hub 登录页也可以支持钱包登录，但这里的钱包登录和“用户直接用钱包签发最终 Session Token”不是一回事。更合理的流程是：

1. 钱包对一个短期登录 challenge 或短期登录 token 签名。
2. Verify Hub 验证钱包签名。
3. Verify Hub 把该证明换成自己签发的标准 Session Token。

从产品设计上看，这种钱包登录可以优先用于“忘记密码”或账号恢复流程。日常登录不应鼓励用户频繁暴露 Root Key / Control Key 的签名能力。未来如果钱包体验足够好，或者与系统本机 KeyPass、Passkey 等机制集成，也可以把它作为 Verify Hub 支持的一种普通凭证类型。

因此，登录能力的扩展点应集中在 Verify Hub 的统一登录页上。只要 User Settings 中能描述某种凭证与用户 DID 的绑定关系，Verify Hub 就可以增加新的验证方式；对后续业务服务和 RBAC 鉴权逻辑来说，仍然只看到标准 Session Token。


### SSO

SSO 可以看成是 Verify Hub 登录能力在 Zone 之间的扩展。

这里有两个角色：

- Current Zone：用户当前绑定或拥有的 Zone。
- Target Zone：用户希望登录并执行操作的目标 Zone。

典型流程如下：

1. 用户在 Target Zone 输入或选择一个 DID。
2. Target Zone 通过 DID 解析得到用户的 Current Zone，以及该 Zone 的 Verify Hub 信息。
3. Target Zone 拉起 Current Zone 的 Verify Hub 登录页。
4. 用户在自己的 Current Zone 完成认证，例如输入用户名 / 密码，或使用钱包、Passkey 等方式。
5. Current Zone 的 Verify Hub 签发一个联合登录凭证。
6. Target Zone 验证该凭证，并把它映射到一个明确的 DID 或本地用户账号。
7. Target Zone 的 Verify Hub 再签发本 Zone 内可用的 Session Token。

用户最终要在 Target Zone 内访问资源，因此最终使用的仍然应是 Target Zone 签发的 Session Token，并继续接受 Target Zone 的 RBAC 和 scope 约束。Current Zone 的联合登录凭证只负责证明“这个用户是谁”，不自动代表他在 Target Zone 里拥有什么权限。

这与传统 Google SSO 的本质类似：Target Zone 信任一个外部身份提供方完成身份认证。区别在于，BuckyOS 体系内的身份提供方可以是用户自己的 Zone，而不是固定的中心化平台；身份提供方的发现和信任根来自 DID、ZoneConfig 和 Verify Hub 公钥。

对 BuckyOS 内部跨 Zone 登录来说，DID 通常是明确的。因为系统凭证的核心是 username / DID，每个可登录用户都应该能映射到一个确定的 DID。

如果未来支持 Gmail 等传统外部账号作为联合登录方式，也必须在登录完成后绑定到一个 DID。绑定方式有两种：

1. 绑定到一个已经存在的 DID，相当于为该 DID 增加一种新的登录方式。
2. 作为 Target Zone 内的本地账号创建一个二级 DID。



### 改变用户状态

- 改变用户类型（`user.change_type`，只在 Admin、User、Limited 之间）
- 改变所在系统用户组（`user_group.set_members`）
- 改变profile

### 用户数据管理

对app service来说，不假设自己能看到一个用户还是多用户的数据，这是系统管理员在安装时的配置，下面是几种典型的模型

单服务(容器)多用户:最常见，但是管理员可以随时调整用户是否能使用该app
单服务(容器)单用户：是上面情况的特例，app service还可以通过在app-id中包含owner-user-id实现隔离（一份app代码，但是用多个不同的容器启动，每个容器只为一个用户服务，此时app.owner恒为userid）

对于app开发者，应该声明自己是那种模式,来方便管理员在安装的时候选择正确的模型.默认是多用户模式
注意app的owner和app的当前用户是两个概念。owner是app的安装者

默认情况下，用户之间的数据都是完全隔离的（包括管理员），只有root用户可以看到所有的数据。
使用root用户，可以导出任意用户的用户数据，这是一个典型的运维需求，而不是日常使用需求。
常规的用户数据导出（管理员可以辅助），应该用用户的登录密码加密。比如用户被禁用后，普通管理员可以协助导出的用户数据。


### sudo机制

SUDO 机制是由 Verify-Hub 提供的,通过一个特殊的提权对话框，要求管理员用户输入密码。在输入密码之后Verify-Hub  会签发一个 SUDO Session Token.

后续在发起请求时，就可以在请求中带上这个 Session Token。这就是 SUDO 的基本机制。Verify-Hub 的 Sudo 授权 token 通常都是时间比较短的（3分钟）,而且有可能在 Sudo 的时候，会有明确的操作边界

sudo 的执行权限一样会受到 AppID 的限制。也就是说，其实对于非系统类的应用来讲，申请调用这个权限的意义不太大，因为它还是会被 AppID 限制住。

所以说，一般都是在类似于 Control Panel 这种系统 UI 中，即它的 AppID 本来就具有大权限的情况下，给 sudo 才有意义。
