# Users & Agents 需求修改意见

> 本文基于当前 UI 原型评审记录整理。核心建议是：**Users & Agents 聚焦系统内部实体管理，并将好友、联系人及外部群组关系拆分为独立的 My Network App。**

## 1. 修改目标

当前原型将内部实体管理、联系人管理、群组成员管理和外部社交关系维护放在同一套页面中，导致“添加”“删除”“分组”等操作存在明显的语义冲突。

本轮修改应达到以下目标：

1. 明确 Users & Agents 的产品边界，使其专注于系统内部实体的身份、配置、权限和运行状态。
2. 将好友、联系人、外部群组及关系网络维护拆分为独立的 **My Network App**。
3. 统一 User 与 Agent 的身份展示框架，同时保留 Agent 特有的运行状态信息。
4. 强化 DID 个人主页的用户心智，弱化过早暴露的底层技术概念。
5. 为移动端和企业级规模补齐搜索、筛选、批量操作等基础能力。

---

## 2. 产品边界调整

### 2.1 Users & Agents 的职责

Users & Agents 应定位为 **系统内部实体管理器（Entity Manager）**，管理本系统内可被配置、授权或运行的主体，包括：

- User
- Agent
- Self-hosted Group

主要关注：

- 实体身份与 DID
- Profile 信息
- 权限与安全
- 配置项
- 在线、健康及运行状态
- 本系统托管的 Group 及成员规模

Users & Agents 可以保留“我创建或托管的 Group 有多少成员”等信息，但不再承担：

- 添加好友
- 维护联系人
- 查看我加入了哪些外部 Group
- 将外部联系人组织进个人联系人组
- 管理关注、好友、成员等外部关系

### 2.2 My Network 的职责

建议拆出独立的 **My Network App**，专门负责 **外部关系管理（Relationship Management）**。

其管理对象包括：

- Friends
- Contacts / Known Contacts
- 我加入的外部 Group
- 联系人分组
- 系统或 Agent 生成的关系视图
- 未来可能扩展的 Followers、Following、Organization Membership 等关系

核心行为包括：

- 添加好友或联系人
- 接受、拒绝关系请求
- 加入或退出外部 Group
- 将联系人加入或移出分组
- 搜索和浏览关系网络
- 管理公开关系信息及关系可见性

### 2.3 拆分原则

必须明确区分：

```text
管理一个系统内部实体
≠
与一个外部实体建立关系
≠
把一个已有联系人放入某个分组
```

对应的典型语义为：

```text
Add User / Add Agent
≠
Add Friend / Add Contact
≠
Add to Group
```

---

## 3. Users & Agents 列表页修改

### 3.1 创建入口

User 与 Agent 的创建逻辑不同，入口必须分开：

- Add User
- Add Agent

当前尚未实现 Agent 创建流程时，点击 Add Agent 可展示 `Coming Soon`，但入口应先保留，以建立正确的产品心智。

### 3.2 搜索与筛选

当前一屏约能展示 9 个实体，小型家庭网络基本够用，但企业级场景会产生较长列表。因此列表页右上角除两个添加入口外，还应提供搜索/筛选入口。

建议交互：

- 默认显示搜索图标或 Search / Filter 按钮。
- 点击后展开为搜索条。
- 支持按名称、DID、类型、角色、标签和状态搜索。
- 可按 User、Agent、Group、在线状态等条件过滤。
- 移动端优先采用“点击图标后展开搜索条”的交互。

### 3.3 实体卡片

不同实体应保持统一的基础身份视觉语言，但突出各自最重要的信息：

- User：头像、名称、简介、身份状态。
- Agent：头像、名称、简介、Owner、运行状态。
- Self-hosted Group：名称、简介、成员数、托管或运行状态。

---

## 4. User 详情页修改

### 4.1 页面结构顺序

建议从上到下调整为：

1. User Card
2. Profile
3. Social Accounts
4. Settings
5. Security & Account
6. DID Document

该顺序对应用户认知：

```text
我是谁
→ 关于我
→ 我在其他网络中的身份
→ 我的使用偏好
→ 我的账号与安全
→ 底层 DID 信息
```

Settings 与 Security & Account 不应相隔过远。DID Document 属于高级信息，应放在页面最后，必要时可折叠到 Advanced 或 Developer Options 中。

### 4.2 User Card

User Card 表示用户在常规场景中被快速查看时的身份摘要，例如鼠标悬停在头像上时显示的 Hover Card。

建议包含：

- Avatar
- Display Name
- 一句话简介
- 少量公开身份标签
- View Profile 入口

User Card 不等同于完整的公共 Profile。

### 4.3 头像与简介编辑

- 当前用户的头像必须可编辑。
- 当前用户的一句话简介必须可编辑。
- 查看其他用户时仅展示，不提供编辑入口。

### 4.4 Profile 改为逐项编辑

当前统一的 Edit 入口不够自然。Profile 中每个 Item 应支持点击后立即编辑，即 Inline Edit。

例如：

- Display Name
- Bio
- Location
- Organization
- Website
- 其他公开资料

用户无需先进入全局 Edit Mode。

### 4.5 Edit 改为 Preview

原有 Edit 按钮建议改为 **Preview**。

Preview 打开独立的公共详情页，用于展示：

> 其他人通过该用户的 DID 或可分享链接访问时，将看到哪些信息。

公共页要求：

- 仅展示用户设置为公开的信息。
- 页面链接可分享。
- 可通过 DID 解析或映射访问。
- 与编辑页明确分离。

由此形成三层身份展示：

```text
User Card：快速认识我
Public Profile：完整了解我
Profile Editor：管理我自己
```

---

## 5. Social Accounts 修改

### 5.1 命名调整

当前类似 Message、Binding 等命名过于技术化，建议统一改为：

- Social Accounts
- 中文：社交账号

其定位不是消息系统配置，而是 DID Profile 中对传统互联网身份的补充。

### 5.2 列表能力

每个社交账号应在详情页直接支持：

- 公开 / 不公开快速切换
- 删除
- 查看平台和账号标识
- 查看验证状态（如适用）

平台列表必须可扩展，不应写死。可能包括 GitHub、X、Telegram、Discord、LinkedIn、Mastodon、WeChat、Email、Phone 等。

### 5.3 添加账号的用户心智

添加社交账号的首要表达应是：

> 完善我的 DID 个人主页，让别人可以了解我在传统互联网中的身份。

其第二层用途才是：

> 帮助好友、群组、Agent 或系统确认某个传统互联网账号与该 DID 的对应关系。

不应一开始就用“绑定”“识别”“验证”等技术语言主导流程。

建议说明文案方向：

> 把你在其他平台使用的账号添加到 DID 个人主页。你可以决定哪些账号公开展示，哪些仅用于身份识别。

### 5.4 可见性与用途

“非公开”不一定等于“完全不使用”。数据层应至少区分：

- 是否公开展示
- 是否可用于身份识别

UI 第一阶段可先提供公开/不公开开关，复杂用途控制后续再扩展。

---

## 6. Agent 详情页修改

### 6.1 与 User 统一身份框架

Agent 与 User 在互联网身份层面应尽量少做差异化。两者都可以具备：

- Avatar
- Profile
- DID
- Social Accounts
- Settings
- Security & Account

其统一抽象可理解为：

```text
Principal
├── User
└── Agent
```

### 6.2 运行状态前置

Agent 本质上是服务。Owner 进入详情页后，最关心的是它是否正常运行、是否繁忙、当前是否有任务。

建议页面顺序：

1. Agent Card
2. State Card
3. Profile
4. Social Accounts
5. Settings
6. Security & Account
7. DID Document

State Card 应紧跟 Agent Card，至少展示：

- Running Tasks
- Queued Tasks
- Health Status
- Last Active
- Uptime 或其他关键运行指标

当前放在页面较下方的运行统计信息应前移。

---

## 7. Self-hosted Group 在 Users & Agents 中的保留范围

Users & Agents 可以继续展示和管理系统内部托管的 Group，包括：

- Group 基本信息
- 简介
- Owner
- 成员数量
- 服务或托管状态
- 权限、安全及 DID 信息

但以下内容应移动到 My Network：

- 我加入了哪些外部 Group
- 申请加入外部 Group
- 退出外部 Group
- 外部 Group 中的社交关系维护

树形关系不应成为当前界面的主设计。某个 Item 与另一个 Item 的父子或关联关系，可继续通过 Connection 表达，暂不强化树形导航。

---

## 8. 新增 My Network App

### 8.1 产品定位

My Network 是 Personal AIOS 中统一维护个人社交关系的一级应用。

用户价值为：

> 在 DID 体系中，好友关系和群组关系不应在每个平台重复维护。用户只需在 My Network 中维护一次，其他应用共同使用。

### 8.2 名称建议

候选名称：

- My Network
- Networks
- Friends & Groups
- Friends

建议暂定 **My Network**，原因是其未来管理范围不只包含好友，还可能包括 Group、Organization、Followers、Following 和信任关系。

### 8.3 一级功能

建议至少包含：

- Contacts / Known Contacts
- Friends
- Groups
- Joined Groups
- Contact Collections
- Dynamic Views
- Add Friend / Add Contact
- Join Group
- 搜索与筛选

“添加好友”应是 My Network 中明确、易发现的一级入口，不应隐藏在 Users & Agents 或某个 Collection 详情页深处。

---

## 9. My Network 中的集合模型

UI 上可统一称为 Collection，但底层应区分两类。

### 9.1 Static Collection

静态联系人组，成员明确保存。

特点：

- 可编辑
- 可增删成员
- 可重命名
- 可填写简介

示例：

- Family
- Friends
- Customers
- Investors

### 9.2 View Collection

基于条件动态计算的视图。

特点：

- 只读
- 成员随条件动态变化
- 不允许手工增删成员
- 由系统自动创建，或由 Agent 根据自然语言规则创建

示例：

- 最近访问过我的主页
- 最近联系过的人
- 最近活跃的联系人
- 使用 GitHub 的联系人

两类 Collection 必须在图标和详情页交互上明确区分。

### 9.3 内置 Contacts 视图

系统应提供一个不可删除的基础视图，可命名为：

- Contacts
- Known Contacts
- My Friends

其含义是：

> 所有已经被用户添加到个人关系网络中的联系人。

新增好友或联系人后，应自动出现在该视图中。

---

## 10. Collection 创建与导入

### 10.1 创建向导

在 Collection 列表页点击 Add 时，建议进入简单向导，提供两类方式：

1. Manual Group
2. Import Group

### 10.2 Manual Group

手工创建只需输入：

- Group Name
- 可选 Description

创建后进入标准详情页，再从已有 Contacts 中添加成员。

### 10.3 Import Group

导入方式可包括：

- 从现有联系人中搜索、过滤并选择
- 从文件导入

从现有联系人导入的流程：

```text
Search / Filter
→ Result List
→ Select All 或选择部分
→ 输入组名
→ Create
```

文件导入可支持 CSV、TXT 等简单格式。系统内部完成解析、匹配和去重，不应给用户增加过多心理负担。

### 10.4 已进入某个 Collection 后的导入

在具体 Collection 详情页内，目标组已经明确，因此无需再次进入创建向导。

流程应直接为：

```text
Import
→ 选择文件或来源
→ 解析与去重
→ 加入当前 Collection
```

---

## 11. Collection 详情页交互

### 11.1 基础结构

建议包含：

- Group Name
- Description
- 成员统计
- Search
- Sort
- Import
- Add
- 可滚动成员列表

### 11.2 Add 的语义

在某个联系人分组中点击 Add，默认含义应是：

> 从系统已有 Contacts 中选择成员加入该组。

不应默认要求用户直接输入一个陌生用户的名字，也不应在该入口中承担复杂的“添加好友”流程。

### 11.3 桌面端批量选择

支持标准的：

- Ctrl 多选
- Shift 连续选择

选中后出现批量操作区，例如：

- Remove from Collection
- Move To
- Copy To

### 11.4 移动端批量选择

移动端默认不显示 Checkbox。

进入选择模式的方式：

- 长按 Item
- 点击 Item 右侧三点菜单后选择 Select

进入选择模式后：

- Item 切换为 Checkbox
- 顶部操作区切换为批量操作区
- 提供移出、移动、复制等操作

### 11.5 删除语义

必须严格区分：

```text
Remove from Collection
≠
Delete Contact
≠
Unfriend
```

在普通分组中删除成员，只表示将该联系人移出当前分组。

真正删除联系人或解除好友关系，应在联系人详情或系统级 Contacts 视图中完成，并提供更强的确认提示。

---

## 12. 与其他应用的关系

My Network 应与另外两个社交类应用共享同一份关系数据。

### 12.1 Home Station

定位：

- 关注 Feed List
- 以“读”为主
- 支持一对多的信息发布和消费

### 12.2 Message Hub

定位：

- 传统 IM 的替代
- 支持一对一和群组沟通

### 12.3 My Network

定位：

- 维护关系
- 管理好友、联系人和群组连接
- 作为其他社交应用的统一关系数据源

三者关系：

```text
My Network：管理关系
Home Station：基于关系获取和发布内容
Message Hub：基于关系进行沟通
```

后台应共享统一的 Social Graph，避免用户在多个应用中重复维护联系人和群组。

---

## 13. 建议实施优先级

### P0：先解决产品边界

- 明确 Users & Agents 只负责内部实体管理。
- 从 Users & Agents 移除添加好友、外部联系人管理和已加入外部 Group 的入口。
- 建立 My Network 一级 App 入口。
- 区分 Add User、Add Agent、Add Friend、Add to Group。

### P1：优化核心详情页

- User 头像与简介可编辑。
- Profile 字段支持 Inline Edit。
- Edit 改为 Preview。
- Social Accounts 支持公开开关和删除。
- Agent State Card 前移。
- Settings 与 Security & Account 相邻。
- DID Document 后置。

### P2：补齐规模化管理能力

- 实体列表搜索与筛选。
- Collection 搜索、排序和批量选择。
- Static Collection 与 View Collection 区分。
- 内置不可删除的 Contacts 视图。
- 文件导入、搜索导入及自动去重。

### P3：扩展关系网络

- Agent 通过自然语言创建动态 View。
- Followers / Following。
- Organization Membership。
- 共同联系人和关系推荐。
- 更细粒度的公开性与身份识别用途控制。

---

## 14. 验收要点

修改后的产品应满足以下判断：

1. 用户能清楚理解 Users & Agents 管理的是“本系统内部的实体”。
2. 用户能在 My Network 首页快速找到“添加好友”入口。
3. 用户不会把“删除联系人”和“从分组移除”误认为同一操作。
4. User 与 Agent 详情页视觉框架一致，但 Agent 的运行状态足够突出。
5. 用户能通过 Preview 明确看到自己的 DID 公共主页。
6. 社交账号被理解为个人 DID Profile 的组成部分，而不是技术绑定配置。
7. 企业级实体数量增加后，仍可通过搜索和筛选快速定位。
8. Home Station 与 Message Hub 能复用 My Network 中维护的关系数据。

---

## 15. 最终建议

本轮不应继续在现有 Users & Agents 页面中叠加联系人和 Collection 的复杂交互，而应先完成产品职责拆分：

```text
Users & Agents
= 内部实体、身份、配置、权限和运行状态

My Network
= 外部联系人、好友、群组及关系网络

Home Station
= 内容与 Feed

Message Hub
= 消息与会话
```

这一拆分能够消除当前“添加 Item”含义不清的问题，也能把 Personal AIOS 的 DID 社交网络能力提升为真正的一级产品能力。
