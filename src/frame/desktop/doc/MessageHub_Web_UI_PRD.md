# MessageHub Web UI 产品需求文档（PRD）

- 文档版本：v0.1（基于口述记录整理）
- 产品名称：MessageHub
- 文档类型：Web UI / 跨端一致性设计 PRD
- 当前范围：以 **Web UI** 为主，同时约束 Desktop / Mobile 的一致信息架构
- 目标读者：产品、前端、客户端、后端、Agent Runtime、设计团队

---

## 1. 文档目的

本文档用于整理 MessageHub 的核心 UI 需求，形成一个可持续扩展的产品规格说明。MessageHub 不是传统意义上的 IM 聊天页，而是面向 Agent 时代的消息与协作入口，承担用户与人、Agent、群组、系统服务等“实体”之间的统一交互职责。

本文档重点覆盖：

1. MessageHub 的产品定位与入口形态
2. 三层 UI 结构（Entity List / Conversation View / Details）
3. 多实体、多 Session、多会话类型的交互模型
4. 以聊天为第一阶段形态的 Conversation View 需求
5. 对未来 Workspace / 协作型会话的可扩展约束

---

## 2. 产品定位

### 2.1 核心定位

MessageHub 是用户在 Agent 时代处理所有消息的核心 UI 组件。

它是：

- 用户与所有实体之间的统一消息入口
- 用户高频驻留的主界面之一
- 消息、任务、上下文、协作的聚合枢纽

这里的“实体”包括但不限于：

- 人（Person）
- Agent
- 群组（Group）
- 系统服务（System Service）
- 其他可向用户收发消息的对象

### 2.2 产品目标

MessageHub 需要满足以下目标：

1. 作为用户在 AI 时代的主要注意力承载界面之一
2. 统一用户与不同通信协议、不同对象类型之间的消息体验
3. 以尽可能一致的流程承载聊天、任务、协作、观察、管理等操作
4. 通过统一心智模型降低多协议、多 Session、多对象带来的复杂度

### 2.3 非目标

当前阶段 MessageHub 不以以下内容为主要目标：

- 不承担复杂业务系统的全部 UI
- 不替代独立 App / 专业工作台的全部能力
- 不以“极致流式输出体验”作为核心优化方向
- 不以“强层级文件系统式导航”作为主要交互模式

---

## 3. 适用平台与入口形态

### 3.1 浏览器 Web 模式

MessageHub 在 Web 端应支持作为一个独立浏览器 Tab 长时间驻留。

要求：

- 可作为主页面长期打开
- 刷新与恢复时尽量保持上下文连续性
- 支持会话状态恢复

### 3.2 Desktop 模式

在桌面模式下，MessageHub 应作为独立窗口存在。

要求：

- 可作为用户主要窗口之一
- 可从桌面图标、Agent 图标等入口直接打开
- 在空间允许的情况下可使用更高密度分栏布局

### 3.3 Mobile App 模式

在移动端，MessageHub 应作为 App 内固定主 Tab 之一存在。

要求：

- 作为稳定入口存在，不应被弱化为深层功能页
- UI 心智接近主流 IM 应用，但能力上支持 Agent / 多 Session / 管理扩展

### 3.4 跨端统一原则

无论平台如何变化，以下原则必须保持一致：

1. MessageHub 始终是核心入口
2. 用户都以“我和各种实体交互的地方”来理解它
3. 实体、会话、详情三层结构保持一致
4. UI 结构尽量稳定，平台差异主要体现在能力增强而不是结构变化

---

## 4. 核心设计原则

### 4.1 弱结构、强入口

系统内部允许存在复杂结构，但不应把复杂性直接暴露给用户。默认视图应尽量接近熟悉的列表与会话模式。

### 4.2 扁平优先、层次可选

默认以扁平列表为主，仅在必要时支持展开子结构，避免用户产生过强的目录式操作负担。

### 4.3 Conversation 是中心

大多数深度操作都应从 Conversation View 向外展开，而不是散落在多个入口中。

### 4.4 一致性优先于局部最优

即使不同对象、不同会话类型能力差异很大，也应尽量让用户看到相似的结构与相似的操作路径。

### 4.5 面向扩展的数据模型驱动 UI

MessageHub 需要优先设计稳定的 UI DataModel，以兼容多协议、多消息类型、多会话类型，而不是围绕单一聊天组件做局部拼接。

---

## 5. 信息架构总览

MessageHub 的整体信息架构由三层构成：

```text
Panel A：Entity List（实体列表）
Panel B：Conversation View（会话视图）
Panel C：Details（实体 / Session 详情）
```

### 5.1 层级关系

```text
Entity List
  -> Conversation View
    -> Details
```

### 5.2 设计含义

- Panel A 解决“我在和谁交互”
- Panel B 解决“我当前在做什么”
- Panel C 解决“这个对象 / 这段会话到底是什么，以及我还能怎么观察或管理它”

---

## 6. 核心概念定义

### 6.1 Entity（实体）

任何可以与用户进行消息交互的对象都属于 Entity。

包括：

- Person
- Agent
- Group
- Service
- 其他协议映射对象

### 6.2 Sub-Entity（子实体）

某些实体之下可能存在持久存在的子实体，例如：

- 公司群下的多个 topic
- 群组下的子群
- 组织下的频道

子实体本身也是实体。

### 6.3 Session（会话实例）

Session 是用户与某个实体之间的具体上下文载体。

一个实体至少存在一个默认 Session，但通常允许存在多个 Session。

### 6.4 Conversation Type（会话类型）

Conversation View 的具体表现形态由会话类型决定。当前版本先实现聊天型会话，但架构必须支持未来扩展为：

- Chat Conversation
- Workspace / 协作型 Conversation
- Task-oriented Conversation
- 文档型 Conversation

### 6.5 Managed Entity（域内实体）

由系统内模型管理、并允许用户进行管理操作的实体，例如：

- 用户自身
- Agent
- 系统内对象

### 6.6 External Entity（域外实体）

由外部协议或外部系统引入，仅支持有限观察与轻量标注的实体。

---

## 7. Panel A：实体列表（Entity List）

### 7.1 定位

实体列表是用户进入 MessageHub 后默认看到的核心入口视图，尤其在移动端应作为默认显示内容。

其目标是帮助用户快速完成三件事：

1. 识别当前有哪些可交互实体
2. 判断最近发生了什么
3. 尽快进入下一步会话

### 7.2 展示内容

每个 Entity Item 至少应包含以下信息：

- 图标 / 头像
- 标题 / 名称
- 最近一条消息摘要
- 最近活动相关提示信息
- 未读状态
- 优先级或标签提示

可选展示：

- 时间戳
- 协议来源
- 静音 / 置顶 / 特殊状态标记

### 7.3 列表特征

实体列表本质上是一个高密度、可扫读的列表型 UI，应参考用户熟悉的 IM 软件体验，但在模型上要兼容更多对象类型与更多状态信息。

要求：

- 用户应能在很短时间内识别每个 item 是什么
- 用户应能快速知道最近一次活动内容
- 优先级与关注状态要有清晰呈现

### 7.4 顶部区域：搜索与过滤

实体列表顶部应提供轻量搜索入口。

默认状态：

- 一个占用空间较小的搜索条

交互后：

- 列表整体下移或让出空间
- 展示最近搜索记录
- 展示可点击的 Tag / Filter

### 7.5 Tag / Filter

支持两类过滤条件：

#### 7.5.1 系统标签

例如：

- unread
- pinned
- important

#### 7.5.2 用户标签

例如：

- company
- project
- family
- 自定义分类标签

标签的定位是轻量过滤，而不是重型目录管理。

### 7.6 列表结构：扁平优先 + 可选树形

默认情况下，实体列表应是扁平的。

但在必要时允许呈现轻量树形结构，以支持父实体与子实体之间的关系。

例如：

```text
Company Group
  ├─ Topic A
  ├─ Topic B
```

### 7.7 子实体交互方式

当前推荐方式为：**原地展开**。

即点击可展开的父实体时，在当前列表中展开其子实体，而不是切换到完全新的列表页。

原因：

- 上下文连续
- 用户不容易迷失层级
- 更符合“弱结构”的总体设计原则

备选方式：

- 点击后以子列表替换当前列表

该方式可以保留为后续特殊场景可选实现，但不作为当前推荐默认策略。

### 7.8 移动端默认行为

在移动端中，MessageHub 默认进入实体列表。

点击某个实体后进入该实体的 Conversation View。

---

## 8. Panel B：Conversation View（会话视图）

### 8.1 定位

Conversation View 是 MessageHub 的核心执行区，用于展示和承载用户与某个实体之间的具体交互。

它不是单纯的聊天框，而是面向任务、消息与上下文的统一会话界面。

### 8.2 总体布局

Conversation View 采用上下结构：

```text
上部：历史消息区 / 时间线
下部：输入区 / Composer
```

### 8.3 历史消息区

历史消息区用于按时间线展示当前 Session 下的所有会话内容。

目标：

- 支持扫读
- 支持回溯
- 支持对 Agent 输出结果进行观察
- 支持展示状态类消息、结果类消息以及普通消息

### 8.4 输入区

输入区是用户发起内容输入的主要区域。

其整体结构在各平台上应尽量一致，但具体输入能力可按平台增强。

基础能力：

- 文本输入
- 发送动作

扩展能力按平台支持：

- 文件上传
- 图片上传
- 拍照
- 视频录制
- 语音输入 / 语音发送
- 剪贴板粘贴等增强能力

### 8.5 多平台输入能力差异

#### 8.5.1 Mobile

能力通常最强，应优先支持：

- 文本
- 图片
- 视频
- 语音
- 拍照 / 录制

#### 8.5.2 Web

通常支持：

- 文本
- 文件上传
- 图片粘贴
- 麦克风（受浏览器能力限制）

#### 8.5.3 Desktop

整体能力接近 Web，但未来可扩展更多系统能力。

### 8.6 组件实现策略

Conversation View 中的聊天内容区属于复杂组件，推荐基于成熟的第三方 Web UI 组件体系进行封装与增强。

但实现重点不应放在某一个现成聊天 UI 上，而应优先抽象统一的 UI DataModel，以便承接：

- 多协议消息
- Agent 输出
- 系统事件
- 临时状态消息
- 不同类型的会话视图

### 8.7 消息兼容性目标

MessageHub 属于聚合型 UI，应尽可能兼容世界上已有 IM 协议可承载的主要内容类型。

需兼容的内容至少包括：

- 文本
- 富文本
- 图片 / 视频 / 文件
- 一次性状态信息
- 追踪 / 状态类消息
- 任务结果类消息

### 8.8 Streaming（流式输出）策略

系统支持流式输出，但它不是当前版本的核心优化方向，也不应被视为 Agent 场景下的默认交互模式。

原因：

1. Agent 工作通常耗时更长
2. 用户多数情况下更关注结果而非 token 级过程
3. 相比“看它逐字输出”，Agent 场景更适合“查看状态 + 获取结果”

因此：

- 支持流式输出
- 不将其作为设计和性能优化的主要目标
- 不把它作为 Agent 消息的默认强依赖体验

### 8.9 Conversation View 与实体类型绑定

Conversation View 与实体类型、会话类型是相关联的。

不同类型的实体以及不同目的的会话，理论上应允许拥有不同的 Conversation UI。

例如：

- 一对一聊天：传统消息时间线
- 群组讨论：消息时间线为主
- 协作型群组：未来可能发展为 Workspace 视图
- Agent 工作区：可能包含更多产物、环境、结果观察能力

当前阶段要求：

- 先实现聊天型 Conversation View
- 架构上预留不同会话类型的扩展能力

---

## 9. Session 模型与多 Session 设计

### 9.1 基本原则

一个实体可以拥有多个 Session。

Session 是一等公民，不是附属功能。

### 9.2 一个实体为什么会有多个 Session

#### 9.2.1 Agent 场景

Agent 的多 Session 最容易理解。用户往往需要针对不同任务保留不同上下文。

例如：

- Session A：写代码
- Session B：写文档
- Session C：排查问题

#### 9.2.2 多协议聚合场景

同一个联系人可能被聚合了多个通信渠道：

- Email
- Telegram
- WhatsApp
- Discord

此时这些渠道会天然映射为多个默认 Session。

#### 9.2.3 协议内线程 / Topic 场景

部分协议支持 thread、topic 或类似机制，这类结构也可能映射为 Session。

### 9.3 每个实体至少有一个默认 Session

无论实体类型如何，系统都应保证该实体至少存在一个默认 Session。

### 9.4 Session 与子实体的区别

#### 子实体

- 是实体
- 持久存在
- 在实体层级中展示

#### Session

- 不是实体本体
- 是实体下的上下文容器
- 是否显式创建、显式销毁取决于协议与业务逻辑

### 9.5 Session 生命周期

Session 不一定有明确的“创建”与“删除”动作。

可能的来源包括：

- 协议自动生成
- 用户加入后可见
- 某一主题开始后被动出现
- 外部系统映射生成

当前要求：

- UI 必须支持 Session 的存在、切换与观察
- 不要求所有 Session 都具备完整的显式管理生命周期

### 9.6 人与人之间的多 Session

系统协议层可支持人与人之间的多 Session，但当前不建议将其作为主要产品交互方式进行强调。

### 9.7 DID / 标准用户场景

当对方是系统内标准用户，拥有 DID、Zone 或 Personal Server 时，可通过系统标准 Message Object 与 HTTP 等协议直接建立消息通信。

此时默认仍应存在一个标准 Session。

---

## 10. Conversation 内的 Session 导航

### 10.1 移动端结构

在移动端，用户从实体列表点击进入某个实体后，Conversation View 默认展示该实体最近活跃的 Session。

### 10.2 Session 当前状态展示

在 Conversation View 的标题栏中应展示当前所处 Session 的标题或标识信息。

### 10.3 Session 列表入口

在移动端，返回按钮旁应提供一个菜单入口。

点击后：

- 从左侧拉出 Session Sidebar
- 展示该实体下所有可见 Session

### 10.4 Session Sidebar 内容

Session Sidebar 至少展示：

- Session 标题
- Session 类型或来源（可选）
- 最近活动提示（可选）
- 当前选中状态

### 10.5 桌面端布局

在桌面端，由于空间更充足，可考虑以下布局：

#### 方案 A：三列

```text
Entity List | Session List | Conversation View
```

#### 方案 B：两列复合

```text
(Entity List + Session List) | Conversation View
```

#### 方案 C：直接打开内嵌 Session List 的 Conversation Panel

适用于从桌面 Agent 图标直接打开某个 Agent 的会话窗口。

当前建议：

- 保持与移动端一致的内嵌式 Session 结构
- 在桌面端允许 Session List 常驻显示以提高效率

---

## 11. 会话类型扩展（Conversation Type）

### 11.1 当前阶段

当前版本只实现聊天型 Conversation。

### 11.2 未来扩展方向

未来不同类型的群聊或不同目的的会话，应允许使用不同类型的 Conversation View，而不被“纯消息时间线”限制。

可能的方向包括：

- 讨论型 Conversation：围绕消息交流
- 协作型 Conversation：围绕文档、文件、产物协作
- 工作区型 Conversation：更接近 Workspace
- Agent 任务型 Conversation：围绕任务状态、结果、环境观察组织 UI

### 11.3 架构要求

需要确保：

- Conversation View 渲染能力与 Conversation Type 解耦
- 未来切换到协作型 UI 时，不需要推翻当前三层信息架构
- 用户依然通过相似路径完成“进入会话 -> 查看内容 -> 查看详情 / 管理”的操作

---

## 12. Panel C：Details（Entity / Session 详情）

### 12.1 定位

Details 是会话视图内的第三层，用于查看和管理当前 Entity 或当前 Session 的详细信息。

它不是顶级入口，而是从 Conversation View 内部进入的深层上下文页面。

### 12.2 入口方式

#### 12.2.1 Entity Details

用户在 Conversation View 中点击标题栏上的头像或实体标识后进入。

#### 12.2.2 Session Details

用户点击标题栏中的 Session 标题或详情入口后进入。

### 12.3 设计原则

Details 必须放在 Conversation View 的上下文中，而不是做成平行的独立系统。

原因：

1. 用户心智更自然
2. 避免出现第二套难以记忆的管理入口
3. 用户在对话中发现问题后可立即进入详情观察或管理，再快速返回

### 12.4 Entity Details：域外实体

对于域外实体，Details 以观察和轻量标注为主。

支持内容：

- 基础信息展示
- 备注
- 标签
- 协议来源或公开资料

通常不支持深度管理。

### 12.5 Entity Details：域内实体

对于域内实体，进入的已不只是“详情页”，而可能是管理入口。

#### 12.5.1 用户自身

点击用户自身时，应进入其个人信息编辑 / 管理页面。

#### 12.5.2 Agent

点击 Agent 时，应进入 Agent 管理相关页面或区域，至少可扩展承载：

- Agent 基本信息编辑
- Agent 环境信息
- RootFS / Workspace 管理
- 过往工作记录观察
- 产物观察与管理

Agent Details 不应仅被理解为“个人资料页”，而应是后续工作区、环境、产物等能力的统一管理入口。

### 12.6 Session Details

Session Details 以只读观察为主。

可包含：

- Session 标题
- Session 来源
- Session 类型
- Session 元数据
- 参与者（如适用）
- 关联协议 / thread / topic 信息

可选支持的轻量编辑：

- 重命名
- 标签
- 置顶 / 标记

但总体上，Session Details 不应承担复杂管理职责。

### 12.7 未来与 Workspace 的关系

当未来引入协作型 Conversation 或工作区型视图时，Details 结构仍应保持尽量一致。

也就是说：

- 多人共享的 Workspace
- Agent 的工作区
- 用户进入的实体 / 会话详情

都应复用相近的信息架构与交互路径，以降低用户学习成本。

---

## 13. UI DataModel 要求

### 13.1 总体要求

MessageHub UI 需要建立统一的前端数据模型，覆盖：

- 实体
- 子实体
- Session
- 消息
- 状态
- Details 扩展信息

### 13.2 设计目标

1. 协议无关
2. UI 组件渲染与底层协议解耦
3. 支持未来扩展 Conversation Type
4. 支持多平台输入输出差异映射
5. 支持外部 IM 与内部 Message Object 并存

### 13.3 Message Object 的 UI 映射要求

消息层至少需要支持以下维度：

- 消息类型
- 内容载荷
- 时间信息
- 发送者 / 来源
- 会话归属
- 展示状态
- 临时状态 / 一次性状态
- 可选的任务状态或跟踪信息

### 13.4 状态类消息

UI 需要对状态类内容保留统一抽象，例如：

- processing
- running
- finished
- failed
- ephemeral typing / transient state

当前阶段不要求定义最终字段级 schema，但要求前后端接口与 UI 组件设计必须为其预留标准承载位置。

---

## 14. 用户流程

### 14.1 主流程：进入并继续会话

```text
进入 MessageHub
  -> 查看实体列表
  -> 选择实体
  -> 打开最近活跃 Session 的 Conversation View
  -> 继续输入 / 查看历史
```

### 14.2 切换 Session

```text
进入某实体 Conversation View
  -> 点击标题栏旁菜单
  -> 打开 Session Sidebar
  -> 选择目标 Session
  -> 切换内容区
```

### 14.3 查看 Entity 详情

```text
在 Conversation View 中
  -> 点击标题栏头像 / 实体名
  -> 打开 Entity Details
```

### 14.4 查看 Session 详情

```text
在 Conversation View 中
  -> 点击 Session 标题 / 详情入口
  -> 打开 Session Details
```

### 14.5 桌面端快捷进入 Agent 会话

```text
点击桌面上的 Agent 图标
  -> 直接打开该 Agent 的 Conversation Panel
  -> Panel 内嵌 Session List
```

---

## 15. 权限与可编辑性原则

### 15.1 Entity 维度

- 域外实体：主要只读，允许备注、打标签等轻操作
- 域内实体：可进入管理页面

### 15.2 Session 维度

- 默认以只读观察为主
- 仅在协议或业务允许时开放轻量编辑

### 15.3 输入能力维度

输入能力由平台决定，但整体交互结构不变。

---

## 16. 设计约束与实现建议

### 16.1 保持用户熟悉感

虽然 MessageHub 的内部模型比传统 IM 更复杂，但首屏交互需要足够像用户熟悉的聊天软件，以降低上手门槛。

### 16.2 避免过度暴露系统复杂度

以下复杂度应尽可能被系统吸收，而不是直接扔给用户：

- 多协议聚合
- 多 Session 来源差异
- 子实体与 Session 的结构差异
- 管理态与观察态切换

### 16.3 Conversation 为中心的扩展路径

未来不论是 Agent 工作区、群组协作、产物观察还是环境管理，都尽量从 Conversation View 向外自然展开。

### 16.4 组件与模型分层

建议工程实现上分为：

1. 协议适配层
2. 统一数据模型层
3. Conversation / List / Details 组件层
4. 跨端能力适配层

---

## 17. 当前版本范围（建议 V1）

### 17.1 必做

1. MessageHub 作为独立 Web 页面 / Tab 存在
2. 实体列表
3. 搜索入口与基础 Tag 过滤
4. 扁平列表 + 基础子实体展开能力
5. 聊天型 Conversation View
6. 消息历史区 + 输入区
7. 多 Session 模型与 Session 切换
8. 移动端隐藏式 Session Sidebar
9. 基础 Entity Details
10. 基础 Session Details（只读）
11. 桌面端可支持更高密度布局

### 17.2 可后置

1. 协作型 Conversation View
2. 深度 Agent Workspace UI
3. 复杂 Session 管理能力
4. 完整的流式输出体验优化
5. 更复杂的会话类型模板系统

---

## 18. 待补充 / 待决策项

以下内容在当前口述记录中尚未完全定稿，后续应补充：

1. 消息级 schema 的字段定义
2. Entity / Session / Message 的接口草案
3. Session 排序规则
4. 搜索范围：仅实体、实体+消息、还是全局统一搜索
5. Tag 的编辑与管理机制
6. 未读、优先级、提醒等 attention 模型
7. Agent 状态类消息的具体展示规范
8. 会话列表与实体列表在桌面端的最终视觉布局
9. Entity Details 与独立管理页面之间的跳转边界
10. 协议映射与默认 Session 命名规则

---

## 19. 一句话总结

MessageHub 的目标不是做一个更复杂的聊天窗口，而是建立一个在 Agent 时代可持续扩展的统一交互入口：

- 入口上像 IM 一样直觉
- 模型上支持 Entity / Session / Conversation Type 的扩展
- 路径上以 Conversation 为中心自然展开到观察、管理与协作
- 体验上尽量统一，降低多对象、多协议、多能力带来的认知负担

