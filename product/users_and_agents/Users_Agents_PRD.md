# BuckyOS Users & Agents PRD

## 1. 文档说明

- **文档名称**：BuckyOS Users & Agents 产品需求文档
- **产品名称**：Users & Agents
- **产品定位**：系统内部实体管理器（Entity Manager）
- **技术域名称**：DID 管理 / Profile / Settings / RBAC / Agent 运行管理
- **文档目标**：明确 Users & Agents 的产品边界，沉淀可用于产品评审、交互设计与原型制作的需求
- **适用范围**：BuckyOS 桌面端优先，兼顾移动端轻量适配
- **关联模块**：Verify Hub、DID / BNS、Profile、Settings、RBAC、Agent Runtime、MessageHub、My Network

本次修订的核心变化是：Users & Agents 不再承担好友、联系人、已加入外部 Group 和个人关系网络维护能力。这些需求迁移到独立的 **My Network** App。

---

## 2. 产品定位与边界

### 2.1 Users & Agents 的职责

Users & Agents 是当前 Zone 内部实体的管理器，管理本系统内可被配置、授权或运行的主体，包括：

- User
- Agent
- Self-hosted Group

主要关注：

- 实体身份与 DID
- Profile 信息
- Social Accounts
- Settings
- Security & Account
- 权限、角色与可用应用
- 在线、健康及运行状态
- 本系统创建、持有或托管的 Group 及成员规模

Users & Agents 可以展示“我创建或托管的 Group 有多少成员”等信息，但不承担个人外部关系维护。

### 2.2 不属于 Users & Agents 的范围

以下能力不再放在 Users & Agents 中：

- 添加好友
- 维护联系人
- 查看我加入了哪些外部 Group
- 申请加入或退出外部 Group
- 将外部联系人组织进个人联系人组
- 管理关注、好友、成员等外部关系
- 维护 Contacts、Known Contacts、Contact Collections、Dynamic Views

这些能力由 **My Network** 统一负责。

### 2.3 必须区分的产品语义

产品和交互中必须明确区分：

```text
管理一个系统内部实体
!=
与一个外部实体建立关系
!=
把一个已有联系人放入某个分组
```

对应的典型动作是：

```text
Add User / Add Agent
!=
Add Friend / Add Contact
!=
Add to Collection
```

Users & Agents 中的 Add 入口只用于创建或邀请系统内部实体。用户想添加好友、联系人或加入外部 Group 时，应进入 My Network。

---

## 3. 产品目标

### 3.1 目标

Users & Agents 需要满足以下目标：

- 让用户清楚理解当前 Zone 内有哪些可管理实体；
- 统一 User、Agent、Self-hosted Group 的身份展示框架；
- 承载系统级权限、配置、账号安全和运行状态管理；
- 为 DID / BNS 登录、Profile 展示和公共主页预览提供入口；
- 让 Agent 与 User 在身份层面并列展示，同时突出 Agent 的运行状态；
- 支持家庭网络到企业级规模的搜索、筛选和批量管理基础能力。

### 3.2 设计原则

1. **内部实体优先**  
   Users & Agents 只管理当前 Zone 内部可配置、可授权、可运行的实体。

2. **User 与 Agent 使用统一身份框架**  
   User 与 Agent 都是 Principal，都可以拥有 Avatar、Profile、DID、Social Accounts、Settings 和 Security & Account。

3. **Agent 运行状态前置**  
   Agent 本质上是服务。Owner 进入详情页时，需要优先看到它是否正常运行、是否繁忙、当前是否有任务。

4. **DID 是个人主页入口，不是第一屏技术概念**  
   普通用户首先理解的是个人主页、头像、简介和公开信息。DID Document 属于高级信息，应后置。

5. **Settings 与 Profile 明确分区**  
   Profile 是对外展示信息；Settings 是当前 Zone 内部的账号、权限和系统约束。

6. **规模化管理能力前置**  
   列表页必须支持搜索、筛选和状态识别，避免实体数量增长后只能靠滚动查找。

---

## 4. 核心概念模型

### 4.1 Principal

Users & Agents 中的核心抽象是 Principal：

```text
Principal
├── User
└── Agent
```

User 和 Agent 在互联网身份层面尽量保持一致：

- Avatar
- Display Name
- Bio
- Profile
- DID
- Social Accounts
- Settings
- Security & Account

差异主要体现在：User 关注登录、账号与权限；Agent 关注 Owner、运行状态、任务与服务健康。

### 4.2 User

User 是当前 Zone 中可以被授权、配置或登录的用户主体。

本期需要覆盖：

- **Admin**：可登录用户中权限最高者，承担日常管理。
- **User**：普通可登录用户，主要使用系统和 App。
- **Limited User**：受限制的可登录用户，适合临时开放 Desktop 使用、企业员工账号、未成年人账号等场景。

`Root` 不作为产品内日常可登录用户展示。Root 更接近 Zone 外的最高控制能力，只用于关键操作、恢复、迁移和最高权限签名。

本期新建 User 需要支持两条路径：

1. **邀请一级 BNS / DID 用户加入当前 Zone**  
   这是推荐路径。用户拥有跨 Zone 识别的身份，管理员只能发起邀请，不能代替目标用户完成加入确认。

2. **创建本地账户**  
   适合临时使用、低成本试用、企业员工账号、未准备好独立 BNS 身份的用户等场景。

### 4.3 Agent

Agent 是由用户或系统创建、托管和运行的智能服务主体。

Agent 需要展示：

- Agent 身份信息
- Owner
- DID
- Profile
- Social Accounts
- Settings
- Security & Account
- Running Tasks
- Queued Tasks
- Health Status
- Last Active
- Uptime 或其他关键运行指标

Agent 没有传统用户密码。它的安全与账号区域应表达为 Owner、授权、密钥、会话、可用能力和运行约束。

### 4.4 Self-hosted Group

Self-hosted Group 是当前 Zone 创建、持有或托管的 Group 实体。

Users & Agents 可以继续展示和管理：

- Group 基本信息
- 简介
- Owner
- 成员数量
- 服务或托管状态
- 权限、安全及 DID 信息

以下内容不属于 Users & Agents，应迁移到 My Network：

- 我加入了哪些外部 Group
- 申请加入外部 Group
- 退出外部 Group
- 外部 Group 中的社交关系维护

### 4.5 Contact 不是 User

Contact / Friend 是个人关系网络中的外部关系对象，不是当前 Zone 的系统用户。

例如 Home Station 中“好友给我留评论”不应被建模为“给好友创建一个系统用户”。它应由 My Network 维护关系，由具体 App 根据 DID、好友关系、评论策略和风控规则决定是否允许写入。

### 4.6 Guest / Anonymous Request

Guest 表示没有登录信息的访问请求，等价于 `Session Token = None`。

Guest 的默认语义是匿名读访问。系统默认不应允许匿名写入；如果某个 App 需要开放匿名评论或匿名提交，应作为该 App 自己的公开写入策略处理，并明确风控与审核边界。

---

## 5. 信息架构

### 5.1 页面总体结构

Users & Agents 使用稳定的两栏结构：

```text
实体列表 | 实体详情
```

不再在 Users & Agents 中设计联系人集合浏览模式，也不再使用三栏集合结构。

### 5.2 第一栏：实体列表

实体列表展示当前 Zone 内可直接管理的实体。

建议分组：

- Self
- Agents
- Users
- Self-hosted Groups

设计要求：

- 不展示联系人明细项；
- 不展示联系人分组树；
- 不展示“我的好友”“我加入的外部 Group”等关系入口；
- 支持搜索、筛选和状态识别；
- 实体卡片风格应与 MessageHub、My Network 中的公共实体展示组件保持一致。

### 5.3 顶部操作区

列表页右上角至少提供：

- Add User
- Add Agent
- Search / Filter

User 与 Agent 的创建逻辑不同，入口必须分开。

当前尚未实现 Agent 创建流程时，点击 Add Agent 可展示 `Coming Soon`，但入口应保留，以建立正确产品心智。

### 5.4 搜索与筛选

当前一屏约能展示 9 个实体，小型家庭网络基本够用；企业级场景会产生较长列表，因此搜索与筛选必须前置。

建议能力：

- 按名称搜索；
- 按 DID 搜索；
- 按类型过滤：User、Agent、Self-hosted Group；
- 按角色过滤：Admin、User、Limited；
- 按标签过滤；
- 按在线状态过滤；
- 按 Agent 健康状态过滤；
- 按 Group 托管或运行状态过滤。

移动端优先采用“点击搜索图标后展开搜索条”的交互。

### 5.5 实体卡片

不同实体保持统一基础身份视觉语言，但突出最重要的信息。

User 卡片：

- Avatar
- Display Name
- Bio
- DID 或身份标签
- 在线 / 账号状态

Agent 卡片：

- Avatar
- Display Name
- Bio
- Owner
- 运行状态
- 当前任务状态

Self-hosted Group 卡片：

- Group Name
- Bio
- Owner / Host
- 成员数
- 托管或运行状态

---

## 6. User 详情页

### 6.1 页面结构顺序

User 详情页从上到下为：

1. User Card
2. Profile
3. Social Accounts
4. Settings
5. Security & Account
6. DID Document

该顺序对应用户认知：

```text
我是谁
-> 关于我
-> 我在其他网络中的身份
-> 我的使用偏好
-> 我的账号与安全
-> 底层 DID 信息
```

Settings 与 Security & Account 不应相隔过远。DID Document 属于高级信息，应放在页面最后，必要时可折叠到 Advanced 或 Developer Options 中。

### 6.2 User Card

User Card 表示用户在常规场景中被快速查看时的身份摘要，例如鼠标悬停在头像上时显示的 Hover Card。

建议包含：

- Avatar
- Display Name
- 一句话简介
- 少量公开身份标签
- View Profile 入口

User Card 不等同于完整的公共 Profile。

### 6.3 头像与简介编辑

- 当前用户的头像必须可编辑。
- 当前用户的一句话简介必须可编辑。
- 查看其他用户时仅展示，不提供编辑入口。

### 6.4 Profile Inline Edit

Profile 中每个 Item 应支持点击后立即编辑，即 Inline Edit。

可编辑字段包括：

- Display Name
- Bio
- Location
- Organization
- Website
- 其他公开资料

用户无需先进入全局 Edit Mode。

Profile 可以合并展示本地 Profile 与 BNS / DID 来源信息，不要求用户理解本地来源和 BNS / 链上来源的技术区别。当字段来自 BNS / 链上来源时，修改时可以提示可能存在更高确认成本或生效延迟。

### 6.5 Preview 公共主页

原有 Edit 按钮改为 **Preview**。

Preview 打开独立公共详情页，用于展示：

> 其他人通过该用户的 DID 或可分享链接访问时，将看到哪些信息。

公共页要求：

- 仅展示用户设置为公开的信息；
- 页面链接可分享；
- 可通过 DID 解析或映射访问；
- 与编辑页明确分离。

由此形成三层身份展示：

```text
User Card：快速认识我
Public Profile：完整了解我
Profile Editor：管理我自己
```

### 6.6 Social Accounts

Social Accounts 是 DID Profile 中对传统互联网身份的补充，不应以 Message、Binding 等技术命名主导用户心智。

添加社交账号的首要表达是：

> 完善我的 DID 个人主页，让别人可以了解我在传统互联网中的身份。

第二层用途才是帮助好友、群组、Agent 或系统确认某个传统互联网账号与该 DID 的对应关系。

列表能力：

- 公开 / 不公开快速切换；
- 删除；
- 查看平台和账号标识；
- 查看验证状态。

平台列表必须可扩展，不应写死。可能包括 GitHub、X、Telegram、Discord、LinkedIn、Mastodon、WeChat、Email、Phone 等。

数据层应至少区分：

- 是否公开展示；
- 是否可用于身份识别。

UI 第一阶段可先提供公开 / 不公开开关，复杂用途控制后续扩展。

### 6.7 Settings

Settings 描述用户在当前系统内必须满足的要求和约束。

包括：

- 用户类型；
- 账号状态；
- 登录凭证状态；
- 密码策略；
- 是否允许修改密码；
- 可用应用；
- 默认权限组；
- Limited User 限制项。

Settings 由系统、管理员或安全流程维护，不应被包装成普通个人资料编辑。

### 6.8 Security & Account

Security & Account 面向账号与安全操作。

建议包含：

- 修改密码；
- 重置密码；
- Passkey / Wallet 登录设置；
- 登录会话；
- 账号恢复；
- 安全事件；
- 禁用或启用账号。

管理员修改账号安全设置时应有明确权限和审计边界。

### 6.9 DID Document

DID Document 是高级信息，用于满足开发者、管理员和高级用户检查身份文档的需求。

展示要求：

- 默认后置；
- 可折叠；
- 可复制 DID；
- 可查看签发方、更新时间和关键声明；
- 不在第一屏用技术细节干扰普通用户。

---

## 7. Agent 详情页

### 7.1 页面结构顺序

Agent 详情页从上到下为：

1. Agent Card
2. State Card
3. Profile
4. Social Accounts
5. Settings
6. Security & Account
7. DID Document

Agent 与 User 使用统一身份框架，但运行状态必须前置。

### 7.2 Agent Card

建议包含：

- Avatar / Logo
- Agent Name
- 一句话简介
- DID
- Owner
- 公开身份标签
- View Profile 入口

### 7.3 State Card

State Card 紧跟 Agent Card，至少展示：

- Running Tasks
- Queued Tasks
- Health Status
- Last Active
- Uptime
- 最近错误或异常状态

当前放在页面较下方的运行统计信息应前移。

### 7.4 Profile

Profile 展示 Agent 对外描述和能力摘要。

建议字段：

- Display Name
- Bio
- Capabilities
- Owner Statement
- Website
- Public Profile Visibility

### 7.5 Social Accounts

Agent 也可以拥有 Social Accounts，用于表达 Agent 在传统互联网或外部服务中的公开身份。

例如：

- GitHub bot account
- X / Telegram / Discord account
- Email
- Webhook identity

Social Accounts 的展示、公开开关、删除和验证状态规则与 User 保持一致。

### 7.6 Settings

Agent Settings 描述 Agent 在当前系统内的配置和约束。

建议包含：

- Owner；
- 可用能力；
- 可访问资源；
- 默认权限；
- 运行环境；
- 自动启动策略；
- 任务并发限制；
- 安全边界。

### 7.7 Security & Account

Agent 没有传统用户密码。该区域应表达为：

- Owner 授权；
- Service Token；
- Key / Secret 管理；
- Session 策略；
- 权限升级控制；
- 禁用或暂停 Agent。

### 7.8 DID Document

Agent DID Document 同样作为高级信息后置展示。

---

## 8. Self-hosted Group 详情页

Self-hosted Group 是系统内部托管或由当前用户创建的 Group 实体。

页面建议包含：

- Group Card；
- Group Profile；
- Owner / Host；
- 成员数量；
- 成员规模摘要；
- 托管状态；
- 与 MessageHub 的可交互状态；
- Settings；
- Security & Permissions；
- DID Document。

Users & Agents 仅管理本系统托管的 Group 实体本身。用户加入的外部 Group、外部 Group 关系、申请加入和退出流程，统一由 My Network 管理。

---

## 9. 关键用户流程

### 9.1 Add User

Add User 是系统级能力，必须明确告诉用户：

- 这是在当前 Zone 中创建或邀请一个真实可登录的用户；
- 该用户会占用系统资源；
- 如果目标只是添加联系人或好友，不应使用这个入口，应进入 My Network。

创建路径：

1. 邀请一级 BNS / DID 用户加入当前 Zone；
2. 创建本地账户。

### 9.2 邀请一级 BNS / DID 用户

管理员侧流程：

1. 管理员点击 Add User。
2. 选择“邀请一级 BNS / DID 用户”。
3. 设置默认用户类型、默认权限组、可用 App 和有效期。
4. 系统生成邀请链接，并在当前 Zone 中创建 pending user / pending invitation 记录。
5. 邀请链接只表达“当前 Zone 邀请某个一级 BNS / DID 用户加入”，不代表该用户已经加入。

目标用户流程：

1. 用户打开邀请链接。
2. 页面展示 target zone 信息、邀请来源、请求的用户类型、默认权限组、可用 App、有效期和数据风险。
3. 用户选择或输入自己的一级 BNS / DID。
4. 钱包或 BNS 管理工具展示确认页，提示会把 target zone 写入该 BNS ownerconfig 的 `binded_zone_list`。
5. 用户使用自己的 root key 确认并提交 BNS ownerconfig 更新。
6. 当前 Zone 查询 BNS，确认该 BNS / DID 的 ownerconfig 已包含 target zone。
7. 当前 Zone 将 pending user 激活，写入本地 User Settings、默认权限组和可用 App 设置。
8. 用户返回当前 Zone 的 Verify Hub 登录页，使用钱包、Passkey、自己的 Zone 或其它支持的登录凭证登录。
9. Verify Hub 签发当前 Zone 内可用的 Session Token。

安全原则：

- 绝不能要求用户在当前系统输入其外部身份体系的原始密码；
- `binded_zone_list` 更新本身就是 root key 级别的确认；
- BNS 绑定证明“该身份同意加入当前 Zone”，Session Token 证明“当前浏览器会话已经登录为该身份”。

如果用户暂时没有自己的 Zone，也可以先把一级身份加入当前 Zone 使用。产品需要说明：这是低成本启动路径，不是最高安全路径；后续应支持将数据迁移到用户自己的 Zone。

### 9.3 创建本地账户

本地账户适合：

- 临时使用；
- 低成本试用；
- 企业员工账号；
- 未准备好独立 BNS 身份的用户；
- 需要受限 Desktop 登录的场景。

流程建议：

1. 管理员点击 Add User。
2. 选择“创建本地账户”。
3. 填写用户名、显示名、用户类型和初始凭证。
4. 设置默认权限组、可用 App 和限制项。
5. 创建完成后自动加入默认基础组。

Desktop 临时登录默认应使用 `Limited` 用户类型，并限制修改密码等敏感能力。

### 9.4 Add Agent

Add Agent 必须作为独立入口存在。

当前尚未实现完整 Agent 创建流程时，点击后可展示 `Coming Soon`。后续完整流程应至少覆盖：

- 选择 Agent 模板或创建方式；
- 设置 Owner；
- 配置 Profile；
- 设置运行环境；
- 设置权限和能力范围；
- 完成后进入 Agent 详情页查看运行状态。

### 9.5 编辑 Profile 与 Preview

用户在 Profile 区域逐项 Inline Edit。

点击 Preview 进入公共主页预览，确认外部用户通过 DID 或分享链接看到的信息。

### 9.6 管理 Settings 与 Security

Settings 和 Security & Account 是系统管理能力，需要明确权限边界。

普通用户可以管理自己的公开资料和部分账号安全设置。管理员可以管理本 Zone 内用户的类型、状态、权限、可用 App 和限制项。高风险操作应进入确认流程。

---

## 10. 与其他应用的关系

### 10.1 My Network

My Network 负责外部关系管理，包括 Friends、Contacts、Joined Groups、Contact Collections 和 Dynamic Views。

Users & Agents 与 My Network 的关系：

- Users & Agents 管理系统内部实体；
- My Network 管理用户的外部关系网络；
- 两者可以复用头像、名称、DID、公共 Profile、实体卡片和 Hover Card；
- 不能把 Contact 当成 Zone User，也不能把 Add Friend 放进 Users & Agents。

### 10.2 MessageHub

MessageHub 负责消息与会话。

协同建议：

- User、Agent、Group 的头像、名称和在线状态应保持一致；
- MessageHub 可以从 Users & Agents 打开系统实体详情；
- 好友请求、对话关系和外部联系人整理不在 Users & Agents 中处理。

### 10.3 Home Station

Home Station 以 Feed 和内容互动为主。

好友评论、公开内容读取和 App 级写入授权，应由 Home Station 结合 My Network 的关系数据和 Verify Hub 的身份认证完成，不应通过创建系统 User 解决。

---

## 11. V1 原型范围

### 11.1 本期必须支持

- Users & Agents 首页；
- 实体列表；
- Add User；
- Add Agent 入口；
- 搜索与筛选入口；
- Self 详情页；
- Agent 详情页；
- 本空间用户详情页；
- Self-hosted Group 详情页；
- User Card / Agent Card；
- User Profile Inline Edit；
- 头像与一句话简介编辑；
- Preview 公共主页；
- Social Accounts 列表、公开开关和删除；
- Settings 与 Security & Account 相邻展示；
- Agent State Card 前置；
- DID Document 后置；
- 一级 BNS / DID 用户邀请加入流程；
- 本地 Limited User 创建流程。

### 11.2 本期明确移出

- 我的好友；
- Contacts / Known Contacts；
- 我加入的外部 Group；
- 手工 Contact Collections；
- Dynamic Views；
- 联系人导入；
- Add Friend / Add Contact；
- Join Group；
- Remove from Collection；
- Delete Contact / Unfriend。

上述能力进入 My Network。

### 11.3 本期保留入口但可不完整实现

- Add Agent 完整创建流程；
- 深度的 Social Account 验证；
- 多账号快速切换；
- 用户从当前 Zone 迁移到自己独立 Zone 的数据迁移流程；
- 去中心化多方共同 Host Group。

---

## 12. 建议实施优先级

### P0：先解决产品边界

- 明确 Users & Agents 只负责内部实体管理；
- 从 Users & Agents 移除好友、联系人、外部 Group 和 Collection 需求；
- 区分 Add User、Add Agent、Add Friend、Add to Collection；
- 建立 My Network 一级 App 入口。

### P1：优化核心详情页

- User 头像与简介可编辑；
- Profile 字段支持 Inline Edit；
- Edit 改为 Preview；
- Social Accounts 支持公开开关和删除；
- Agent State Card 前移；
- Settings 与 Security & Account 相邻；
- DID Document 后置。

### P2：补齐规模化管理能力

- 实体列表搜索与筛选；
- 移动端搜索展开交互；
- 按类型、角色、标签和状态过滤；
- 企业级实体数量下的快速定位能力。

---

## 13. 验收要点

修改后的产品应满足以下判断：

1. 用户能清楚理解 Users & Agents 管理的是“本系统内部实体”。
2. 用户不会在 Users & Agents 中寻找“添加好友”或“加入外部 Group”。
3. Add User 与 Add Agent 是两个独立入口。
4. User 与 Agent 详情页视觉框架一致。
5. Agent 的运行状态在详情页足够突出。
6. 用户能通过 Preview 明确看到自己的 DID 公共主页。
7. Social Accounts 被理解为 DID Profile 的组成部分，而不是技术绑定配置。
8. Settings、Security & Account 和 DID Document 的层级清晰。
9. 企业级实体数量增加后，仍可通过搜索和筛选快速定位。
10. My Network、Home Station 和 MessageHub 能复用 Users & Agents 的公共实体展示能力，但不混用产品职责。

---

## 14. 关键开放问题

1. Add Agent 的首版创建流程是否只支持模板创建，还是支持空白 Agent？
2. Self-hosted Group 是否需要在 Users & Agents 中支持成员明细管理，还是只展示成员规模并跳转到专门 Group 管理页？
3. Social Accounts 的验证状态由哪个服务提供，是否需要统一可信验证机制？
4. BNS / 链上来源 Profile 字段修改涉及成本时，用户支付与确认路径如何设计？
5. 企业级场景中是否需要批量禁用用户、批量调整角色等管理能力？

---

## 15. 最终产品判断

Users & Agents 不应被设计成通讯录或关系网络工具，而应被设计成：

- 当前 Zone 的内部实体管理中心；
- User、Agent、Self-hosted Group 的统一身份与配置入口；
- DID 公共主页、Profile、Settings、Security 和运行状态的系统级管理界面；
- My Network、MessageHub、Home Station 可复用的实体信息来源。

它的核心价值不在于“把所有人与群都列出来”，而在于帮助用户理解并管理：

```text
哪些主体属于当前系统，
它们是谁，
它们拥有什么权限，
它们是否健康运行，
以及它们对外展示什么身份。
```
