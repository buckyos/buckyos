# BuckyOS My Network PRD

## 1. 文档说明

- **文档名称**：BuckyOS My Network 产品需求文档
- **产品名称**：My Network
- **产品定位**：个人外部关系管理器（Relationship Management）
- **技术域名称**：Social Graph / Contacts / Friends / Groups / Collections / DID Relationship
- **文档目标**：承接从 Users & Agents 拆分出来的好友、联系人、外部 Group 和关系网络管理需求
- **适用范围**：BuckyOS 桌面端优先，兼顾移动端轻量适配
- **关联模块**：Users & Agents、MessageHub、Home Station、Verify Hub、DID / BNS、Profile、Agent Runtime

My Network 是 Personal AIOS 中统一维护个人社交关系的一级应用。

用户价值为：

> 在 DID 体系中，好友关系和群组关系不应在每个平台重复维护。用户只需在 My Network 中维护一次，其他应用共同使用。

---

## 2. 产品定位与边界

### 2.1 My Network 的职责

My Network 负责外部关系管理，管理对象包括：

- Friends
- Contacts / Known Contacts
- 我加入的外部 Group
- 联系人分组
- 系统或 Agent 生成的关系视图
- 未来可能扩展的 Followers、Following、Organization Membership 等关系

核心行为包括：

- 添加好友或联系人；
- 接受、拒绝关系请求；
- 加入或退出外部 Group；
- 将联系人加入或移出分组；
- 搜索和浏览关系网络；
- 管理公开关系信息及关系可见性；
- 导入联系人；
- 通过 Agent 创建动态关系视图。

### 2.2 不属于 My Network 的范围

以下能力不由 My Network 直接管理：

- 创建当前 Zone 的系统 User；
- 创建或配置 Agent；
- 管理 Agent 运行状态；
- 管理当前 Zone 内部 User 的 Settings、Security & Account；
- 管理 Self-hosted Group 的托管状态、服务状态和系统权限；
- 调整当前 Zone 的 RBAC。

这些能力属于 Users & Agents 或其它系统管理应用。

### 2.3 必须区分的产品语义

My Network 中必须明确区分：

```text
Add Friend / Add Contact
!=
Add User
!=
Add to Collection
```

同样必须区分：

```text
Remove from Collection
!=
Delete Contact
!=
Unfriend
```

在普通分组中删除成员，只表示将该联系人移出当前分组。真正删除联系人或解除好友关系，应在联系人详情或系统级 Contacts 视图中完成，并提供更强确认提示。

---

## 3. 产品目标

### 3.1 目标

My Network 需要满足以下目标：

- 让用户能快速找到 Add Friend / Add Contact 入口；
- 统一管理好友、联系人、已加入外部 Group 和联系人分组；
- 为 Home Station、MessageHub 和其它社交 App 提供统一 Social Graph；
- 支持联系人导入、去重、来源追踪和整理；
- 支持 Static Collection 与 View Collection；
- 支持桌面端和移动端的搜索、筛选、排序、批量操作；
- 为 Agent 通过自然语言创建关系视图预留结构。

### 3.2 设计原则

1. **关系维护集中化**  
   用户只在 My Network 中维护一次关系，其它 App 复用。

2. **好友、联系人、Group 和 Collection 语义清晰**  
   不把系统 User、联系人、Group 成员、Collection 成员混成同一种操作。

3. **先浏览与管理，再暴露协议细节**  
   用户首先理解的是“我的联系人和群组”，不是底层 Social Graph 数据结构。

4. **导入优先于手工重建**  
   用户通常已有历史联系人体系，My Network 应优先支持导入、同步、匹配和去重。

5. **删除语义必须谨慎**  
   从分组移除、删除联系人、解除好友关系是不同风险等级的操作。

6. **移动端保持轻量**  
   移动端优先支持查看、搜索、添加、基础整理和选择模式，不承载复杂桌面批量操作。

---

## 4. 核心概念模型

### 4.1 Social Graph

Social Graph 是 My Network 的底层关系数据模型，描述当前用户与外部实体之间的关系。

可能包含：

- Contact 关系；
- Friend 关系；
- Joined Group 关系；
- Block / Mute 等负向关系；
- Collection 归属；
- Followers / Following；
- Organization Membership；
- Agent 生成的动态视图规则。

后台应共享统一 Social Graph，避免用户在多个应用中重复维护联系人和群组。

### 4.2 Contact / Known Contact

Contact 是用户维护的个人关系对象，不是系统 `UserType`。

它可能是：

- 原生 DID 联系人；
- 从 Telegram / Email 等外部来源同步而来；
- 从 CSV / TXT / XML / 手机通讯录导入而来；
- 尚未完成归并的影子联系人 / 候选联系人。

Contact 可以只有单向记录，不一定是双向好友。

### 4.3 Friend

Friend 是更强的关系状态。

#### 单向关系

- 我添加了对方；
- 对方未必添加我；
- 我可以读取其公开资料；
- 我给对方发消息，对方未必能看到或愿意接收。

#### 双向关系

- 双方都已确认彼此；
- 可以放心地双向通信；
- 更接近传统“好友”状态。

该模型适合去中心化体系，能够降低垃圾消息问题。

### 4.4 Joined Group

Joined Group 表示当前用户加入的外部 Group。

它可以是：

- DID Group；
- MessageHub 可交互 Group；
- 外部组织或社区；
- 未来支持的跨 Zone Group。

Joined Group 不等于当前 Zone 托管的 Self-hosted Group。Self-hosted Group 的系统配置在 Users & Agents 中管理；加入、退出和浏览外部 Group 关系在 My Network 中管理。

### 4.5 Collection

Collection 是联系人和关系条目的组织视图，UI 上可统一称为 Collection，但底层应区分两类。

#### Static Collection

静态联系人组，成员明确保存。

特点：

- 可编辑；
- 可增删成员；
- 可重命名；
- 可填写简介。

示例：

- Family
- Friends
- Customers
- Investors

#### View Collection

基于条件动态计算的视图。

特点：

- 只读；
- 成员随条件动态变化；
- 不允许手工增删成员；
- 由系统自动创建，或由 Agent 根据自然语言规则创建。

示例：

- 最近访问过我的主页；
- 最近联系过的人；
- 最近活跃的联系人；
- 使用 GitHub 的联系人。

两类 Collection 必须在图标和详情页交互上明确区分。

### 4.6 内置 Contacts 视图

系统应提供一个不可删除的基础视图，可命名为：

- Contacts
- Known Contacts
- My Friends

建议暂定 **Contacts**，含义是：

> 所有已经被用户添加到个人关系网络中的联系人。

新增好友或联系人后，应自动出现在该视图中。

---

## 5. 信息架构

### 5.1 一级入口

My Network 首页应至少包含以下一级功能：

- Contacts / Known Contacts
- Friends
- Groups
- Joined Groups
- Contact Collections
- Dynamic Views
- Requests
- Add Friend / Add Contact
- Join Group
- Search / Filter

“添加好友”应是 My Network 中明确、易发现的一级入口，不应隐藏在 Users & Agents 或某个 Collection 详情页深处。

### 5.2 页面模式

My Network 使用关系浏览模式：

```text
关系导航 | 关系条目列表 | 条目详情
```

桌面端可以使用三栏结构：

- 第一栏：关系导航；
- 第二栏：当前视图的条目列表；
- 第三栏：选中条目的详情。

移动端采用单页层级导航：

```text
视图列表 -> 条目列表 -> 条目详情
```

### 5.3 第一栏：关系导航

建议包含：

- Contacts
- Friends
- Joined Groups
- Requests
- Collections
- Dynamic Views

第一栏只展示关系视图入口，不混入 Users & Agents 的系统实体管理入口。

### 5.4 第二栏：条目列表

第二栏用于展示当前视图中的全部条目，采用高密度单行列表。

职责：

- 浏览当前视图内全部元素；
- 搜索当前视图；
- 按来源、类型、状态筛选；
- 支持排序；
- 支持多选整理；
- 作为批量操作和管理操作的主场景。

表现规则：

- 统一使用单行列表，强调密度和可扫描性；
- 支持来源后缀、状态图标、备注、未整理标识；
- 支持桌面端多选、右键菜单、批量移动、批量删除、批量合并候选等能力。

### 5.5 第三栏：条目详情

第三栏展示第二栏当前选中元素的完整详情。

可能展示的详情类型包括：

- Contact 详情；
- Friend 详情；
- Joined Group 详情；
- Collection 详情；
- View Collection 规则详情；
- 未来扩展对象详情。

第三栏不改变当前关系浏览上下文，用户始终知道自己从哪个视图进入。

### 5.6 搜索与筛选

My Network 必须支持全局搜索和当前视图内搜索。

搜索维度：

- 名称；
- DID / BNS；
- 备注；
- 来源；
- 联系方式；
- Group 名称；
- Collection 名称。

筛选维度：

- Contact / Friend / Group；
- 单向 / 双向关系；
- 来源平台；
- 最近联系；
- 最近活跃；
- 是否有 DID；
- 是否有未整理来源；
- 是否在某个 Collection 中。

---

## 6. 首页与核心入口

### 6.1 首页

首页应直接展示用户关系网络概况。

建议包含：

- Contacts 数量；
- Friends 数量；
- Joined Groups 数量；
- Requests 数量；
- 最近新增联系人；
- 最近活跃关系；
- 常用 Collections；
- Add Friend / Add Contact；
- Join Group；
- Import。

### 6.2 Add Friend / Add Contact

Add Friend / Add Contact 是一级入口。

支持方式：

- 输入 DID / BNS；
- 粘贴分享链接；
- 从已有 Social Account 信息查找；
- 从邮箱 / 手机号发现 DID；
- 从外部平台同步结果中选择；
- 从导入文件中添加。

添加流程应明确关系语义：

- 添加为 Contact；
- 发送 Friend Request；
- 接受或拒绝关系请求；
- 形成单向或双向关系。

### 6.3 Join Group

Join Group 是加入外部 Group 的入口。

支持方式：

- 输入 Group DID / BNS；
- 粘贴邀请链接；
- 从推荐或搜索结果加入；
- 接受他人邀请。

流程应包含：

- 展示 Group 基本信息；
- 展示 Owner / Host；
- 展示成员规模；
- 展示公开资料和风险提示；
- 提交加入请求；
- 查看 pending / accepted / rejected 状态；
- 退出 Group。

### 6.4 Requests

Requests 集中展示关系请求。

包括：

- 收到的 Friend Request；
- 发出的 Friend Request；
- Group Join Request；
- Group Invite；
- Agent 或系统建议的关系确认。

用户可在此接受、拒绝、忽略或撤回请求。

---

## 7. Contact 详情页

Contact 详情页用于查看和管理个人联系人。

建议包含：

1. **基础身份**
   - 名称
   - 备注
   - 来源标识
   - DID（如有）
   - 是否为双向关系

2. **Profile**
   - 头像
   - 昵称
   - 简介
   - 公开资料
   - DID 公共主页入口

3. **来源信息**
   - 导入来源
   - 最近同步来源
   - 历史导入批次
   - 创建时间 / 最近更新时间

4. **Reachability**
   - DID
   - Telegram
   - Email
   - Phone
   - 其它已知可达通道

5. **所属关系**
   - 所属 Collections
   - 是否在 Contacts 中
   - 是否为 Friend
   - 是否与某些 Joined Group 存在关系

6. **整理操作**
   - Add to Collection
   - Remove from Collection
   - Delete Contact
   - Unfriend
   - Block / Unblock
   - Merge Contacts
   - 交给 Agent 自动整理

### 7.1 来源后缀展示

对于从外部系统同步来的名字，可在视图层使用来源后缀，例如：

- `Bob.telegram`
- `Alice.imported`

其作用不是长期暴露“脏数据”，而是帮助用户理解来源、避免命名冲突，并为后续合并提供依据。

---

## 8. Joined Group 详情页

Joined Group 详情页用于查看和管理用户加入的外部 Group。

建议包含：

- Group 名称；
- Group DID；
- Owner / Host 信息；
- 成员数量；
- 公开简介；
- 我在该 Group 中的状态；
- 加入时间；
- 是否可在 MessageHub 中发消息；
- 跳转到 MessageHub；
- 退出 Group；
- 关系可见性设置。

该页面不负责管理当前 Zone 托管服务的运行状态。托管状态属于 Users & Agents 中的 Self-hosted Group。

---

## 9. Collection 创建与导入

### 9.1 创建向导

在 Collection 列表页点击 Add 时，进入简单向导，提供两类方式：

1. Manual Group
2. Import Group

这里的 Group 是联系人整理分组，不等于 DID Group 或 MessageHub 群。

### 9.2 Manual Group

手工创建只需输入：

- Group Name
- 可选 Description

创建后进入标准详情页，再从已有 Contacts 中添加成员。

### 9.3 Import Group

导入方式可包括：

- 从现有联系人中搜索、过滤并选择；
- 从文件导入。

从现有联系人导入的流程：

```text
Search / Filter
-> Result List
-> Select All 或选择部分
-> 输入组名
-> Create
```

文件导入可支持 CSV、TXT、XML 等简单格式。系统内部完成解析、匹配和去重，不应给用户增加过多心理负担。

### 9.4 已进入某个 Collection 后的导入

在具体 Collection 详情页内，目标组已经明确，因此无需再次进入创建向导。

流程应直接为：

```text
Import
-> 选择文件或来源
-> 解析与去重
-> 加入当前 Collection
```

---

## 10. Collection 详情页

### 10.1 基础结构

Collection 详情页建议包含：

- Collection Name；
- Description；
- 成员统计；
- Collection 类型：Static / View；
- Search；
- Sort；
- Import；
- Add；
- 可滚动成员列表；
- 批量操作区。

### 10.2 Add 的语义

在某个联系人分组中点击 Add，默认含义应是：

> 从系统已有 Contacts 中选择成员加入该组。

不应默认要求用户直接输入一个陌生用户的名字，也不应在该入口中承担复杂的“添加好友”流程。

### 10.3 Static Collection 交互

Static Collection 支持：

- 重命名；
- 修改 Description；
- Add 成员；
- Import 成员；
- Remove from Collection；
- Move To；
- Copy To；
- 删除 Collection。

删除 Collection 不等于删除其中的联系人。

### 10.4 View Collection 交互

View Collection 支持：

- 查看规则；
- 查看成员；
- 搜索和排序；
- 复制为 Static Collection；
- 编辑规则（仅系统或 Agent 允许时）；
- 删除视图（仅非内置视图）。

View Collection 不允许手工 Add 或 Remove 单个成员。

---

## 11. 批量选择与删除语义

### 11.1 桌面端批量选择

支持标准的：

- Ctrl 多选；
- Shift 连续选择；
- 右键菜单；
- 快捷批量操作。

选中后出现批量操作区，例如：

- Remove from Collection；
- Move To；
- Copy To；
- Add to Collection；
- Merge；
- Delete Contact。

### 11.2 移动端批量选择

移动端默认不显示 Checkbox。

进入选择模式的方式：

- 长按 Item；
- 点击 Item 右侧三点菜单后选择 Select。

进入选择模式后：

- Item 切换为 Checkbox；
- 顶部操作区切换为批量操作区；
- 提供移出、移动、复制等操作。

### 11.3 删除语义

必须严格区分：

```text
Remove from Collection
!=
Delete Contact
!=
Unfriend
```

#### Remove from Collection

只把联系人从当前 Collection 中移出。

适用：

- Static Collection；
- 当前 Collection 不是 Contacts 基础视图；
- 风险低，确认成本低。

#### Delete Contact

从个人关系网络中删除该联系人。

影响：

- 从 Contacts 基础视图中移除；
- 从相关 Static Collections 中移除；
- 保留必要的审计或历史来源记录；
- 不一定撤销对方对我的关系。

该操作需要明确确认。

#### Unfriend

解除好友关系。

影响：

- 取消双向 Friend 状态；
- 可能仍保留 Contact；
- 可能影响 MessageHub 和 Home Station 的关系策略。

该操作需要更强确认提示。

---

## 12. 联系人导入

### 12.1 导入原则

导入是联系人系统的主入口，优先级高于手工逐个添加。

原则：

- 用户今天不太可能从零开始重建联系人体系；
- 大多数联系人来自历史系统迁移或外部同步；
- 导入过程必须尽量安全、可理解、不覆盖旧数据；
- 导入结果需要保留来源追踪；
- 重复项应进入候选合并流程，而不是直接覆盖。

### 12.2 导入结果默认落点

导入完成后，联系人首先进入：

- Contacts 基础视图；
- 必要时可由用户继续加入某个 Static Collection。

不应导入后直接把联系人塞进 Users & Agents。

### 12.3 支持方式

桌面端：

- CSV 导入；
- TXT 导入；
- XML 导入；
- 其它常见通讯录导出格式。

移动端后续：

- 导入系统通讯录；
- 从手机通讯录读取手机号等信息。

### 12.4 导入后处理

导入后需要处理：

- 新导入联系人如何与历史联系人整合；
- 是否自动识别重复项；
- 是否产生影子联系人或候选合并项；
- 是否保留来源追踪；
- 如何保证不会随便覆盖现有资料。

---

## 13. DID 发现与外部通道联系人

### 13.1 外部通道联系人

对于 Telegram、Email 等外部通道联系人，产品逻辑应尽量遵循来源系统本身的关系模型。

例如：

- 如果用户已经绑定 Telegram，那么 Telegram 联系人应主要通过 Telegram 同步得到；
- 不建议单独提供“手输 Telegram 账号直接加好友”的主入口；
- 可以允许用户把 Telegram 身份作为单向记录加入联系人体系中。

### 13.2 手机号 / 邮箱发现 DID

系统未来可以支持“已知手机号 / 邮箱，但不知道 DID”时的发现流程。

基本思路：

- 用户本地拥有对方手机号 / 邮箱；
- 系统基于标准规则计算哈希；
- 去公开系统中匹配 DID / BNS 相关索引；
- 若匹配成功，自动补全对方 DID。

产品注意事项：

- 这是“发现 DID”的能力，不等于自动建立双向关系；
- 需要明确隐私提示；
- 可信度与匹配置信度应可视化。

---

## 14. Agent 生成的关系视图

Agent 可以帮助用户整理关系网络，但需要明确区分自动视图和手工分组。

能力方向：

- 根据自然语言创建 View Collection；
- 推荐重复联系人合并；
- 推荐加入某个 Collection；
- 自动识别最近活跃联系人；
- 自动识别使用某平台的联系人；
- 清理无效条目；
- 解释每个推荐依据。

Agent 生成的 View Collection 默认只读。用户可以将其复制为 Static Collection 后手工编辑。

---

## 15. 与其他应用的关系

My Network 应与 Home Station 和 MessageHub 共享同一份关系数据。

### 15.1 Home Station

定位：

- 关注 Feed List；
- 以“读”为主；
- 支持一对多的信息发布和消费。

Home Station 可使用 My Network 判断：

- 谁是好友；
- 谁可以评论；
- 谁需要审核；
- 哪些内容可见。

### 15.2 MessageHub

定位：

- 传统 IM 的替代；
- 支持一对一和群组沟通。

MessageHub 可使用 My Network 判断：

- 会话对象是否为联系人；
- 是否为双向好友；
- 是否被拉黑；
- Joined Group 是否可进入会话；
- 好友请求状态。

### 15.3 Users & Agents

定位：

- 管理当前 Zone 内部实体；
- 管理 User、Agent、Self-hosted Group 的身份、配置、权限和运行状态。

My Network 不创建系统 User，不管理 Agent 运行状态。

三者关系：

```text
My Network：管理关系
Home Station：基于关系获取和发布内容
MessageHub：基于关系进行沟通
Users & Agents：管理系统内部实体
```

---

## 16. V1 原型范围

### 16.1 本期必须支持

- My Network 首页；
- Contacts 基础视图；
- Friends 视图；
- Joined Groups 视图；
- Requests 视图；
- Static Collection 列表与详情；
- View Collection 列表与详情；
- Add Friend / Add Contact；
- Join Group；
- Import；
- Contact 详情页；
- Joined Group 详情页；
- Collection 创建向导；
- Collection 内 Add 成员；
- Remove from Collection；
- Delete Contact；
- Unfriend；
- 搜索、筛选、排序；
- 桌面端多选；
- 移动端选择模式。

### 16.2 本期保留入口但可不完整实现

- Agent 根据自然语言创建 View Collection；
- 自动联系人合并；
- 手机号 / 邮箱发现 DID；
- Followers / Following；
- Organization Membership；
- 共同联系人和关系推荐；
- 更细粒度的公开性与身份识别用途控制。

---

## 17. 建议实施优先级

### P0：产品边界与一级入口

- 建立 My Network 一级 App 入口；
- 从 Users & Agents 承接 Contacts、Friends、Joined Groups、Collections；
- Add Friend / Add Contact 成为一级入口；
- 区分 Add User、Add Friend、Add to Collection；
- 明确 Remove from Collection、Delete Contact、Unfriend。

### P1：核心关系管理

- Contacts 基础视图；
- Friends 状态；
- Joined Groups；
- Requests；
- Contact 详情；
- Joined Group 详情；
- Collection 详情；
- 文件导入和来源追踪。

### P2：规模化整理能力

- 搜索、筛选、排序；
- 桌面端批量选择；
- 移动端选择模式；
- Static Collection 与 View Collection 区分；
- 自动去重候选；
- 复制 View 为 Static Collection。

### P3：智能关系网络

- Agent 创建动态 View；
- Followers / Following；
- Organization Membership；
- 共同联系人；
- 关系推荐；
- DID 发现。

---

## 18. 验收要点

修改后的产品应满足以下判断：

1. 用户能在 My Network 首页快速找到 Add Friend / Add Contact。
2. 用户能清楚理解 My Network 管理的是外部联系人、好友、Group 和关系网络。
3. 用户不会把 Add Friend 理解为 Add User。
4. 用户不会把 Remove from Collection 理解为 Delete Contact。
5. Static Collection 与 View Collection 在视觉和交互上明确区分。
6. Contacts 基础视图不可删除，新增联系人会自动进入该视图。
7. Joined Groups 管理加入和退出外部 Group，不管理当前 Zone 的托管状态。
8. Home Station 与 MessageHub 能复用 My Network 的关系数据。
9. 桌面端支持批量选择，移动端支持选择模式。
10. 导入联系人不会直接覆盖历史数据，并保留来源追踪。

---

## 19. 关键开放问题

1. Contacts、Known Contacts、My Friends 三个名称中，首版最终采用哪一个？
2. Friend Request 与 Contact 添加是否使用同一个向导，还是拆成两个入口？
3. 外部 Group 搜索和发现由 My Network 自建，还是复用 MessageHub / DID 目录能力？
4. Delete Contact 后是否保留历史消息、评论和互动记录的引用？
5. Agent 自动整理建议的解释性、回滚机制和权限边界如何定义？
6. 手机号 / 邮箱发现 DID 的隐私提示和匹配置信度如何展示？

---

## 20. 最终产品判断

My Network 应被设计成 Personal AIOS 的统一关系网络入口：

- 管理好友、联系人和已加入外部 Group；
- 管理联系人分组和动态关系视图；
- 承担 Add Friend、Join Group、Import、整理、去重和删除语义；
- 为 Home Station 和 MessageHub 提供共享 Social Graph；
- 与 Users & Agents 清晰分工。

最终边界应保持为：

```text
Users & Agents
= 内部实体、身份、配置、权限和运行状态

My Network
= 外部联系人、好友、群组及关系网络

Home Station
= 内容与 Feed

MessageHub
= 消息与会话
```
