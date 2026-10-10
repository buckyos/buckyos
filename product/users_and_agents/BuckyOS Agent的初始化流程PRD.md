# BuckyOS Agent 的初始化流程 PRD

| 文档项 | 内容 |
| --- | --- |
| 版本 | v1.0 |
| 日期 | 2026-10-10 |
| 适用范围 | BuckyOS 中 Agent 的首次创建与启用，重点覆盖预装 Jarvis 引导入口 |
| 面向角色 | 产品、交互设计、前端、账号与权限服务、OpenDAN、Message Hub、测试人员 |
| 需求依据 | 本轮关于 Agent 初始化机制与交互流程的连续讨论；2026-10-09 对仓库实现的现状核对（附录 A） |
| 关联文档 | [Users & Agents PRD](Users_Agents_PRD.md)（本文细化其 §9.4 Add Agent）、[用户类型](UserType.md)、[MessageHub Web UI PRD](../message_hub/MessageHub_Web_UI_PRD.md)、[Jarvis Build-in 能力思考](../Jarvis%20Desing.md) |
| 文档边界 | 正文定义产品行为与验收要求，不规定具体 API、存储目录和进程调度实现。附录 A 记录现状与差距；附录 B 是本期实现方案，供实施的 code agent 使用，与正文冲突时以正文为准 |

> **核心变化：系统预装 OpenDAN 运行能力和 Jarvis RootFS 模板，但不替用户预先创建 Jarvis 身份。用户通过统一的 Agent 创建向导，建立身份、确认权限、选择运行实现，并按需绑定外部消息通道。创建成功后，桌面引导图标转为该 Agent 实例的入口。**

## 1. 背景与目标

### 1.1 当前问题

当前 Jarvis 以预装 App 的方式呈现。预装过程中，系统不仅准备运行组件，还提前建立 Agent 身份，并将身份与特定运行实现、Jarvis RootFS 绑定。这使用户第一次进入时看到的是一个已经就绪的 Jarvis，而没有经历创建和授权过程。

底层架构实际上已经能够区分身份、承载服务与 Agent 文件集合，但产品交互仍将它们写死在一起，产生以下问题：

- **身份由系统代建。** 用户可能并不希望自己的 Agent 叫 Jarvis，也未主动确认头像、账号和权限。在一个 Zone 存在多个用户时，多个同名 Jarvis 更难区分。
- **配置缺少明确的设置时机。** 用户需要到管理面板修改系统替其决定的配置，却不了解这些设置之间的关系。
- **扩展能力没有体现在流程中。** 后续替换 Loader、安装其他 Agent 模板或接入更多消息通道，都缺少统一的创建入口。

### 1.2 产品目标

1. 将 Agent 从“预装后自动存在的 App”调整为“用户主动创建的独立网络身份及其运行实例”。
2. 普通用户使用默认值即可完成初始化，不因扩展能力而承担复杂配置负担。
3. 高级用户能够理解并选择 Agent Loader、RootFS 模板、权限与消息通道。
4. 从第一次启用起明确 Owner、权限上限及消息触发边界。
5. 复用标准账号创建、权限配置和系统安装器，避免另外维护一套同类流程。
6. 形成从桌面预装图标到个性化 Agent 入口的完整闭环。

### 1.3 本期不解决的问题

本期不新增完整的 Agent 应用市场、Loader 开发平台或多通道管理中心；不要求在初始化向导中一次绑定多个通道；不实现 Lark/飞书通道本身；不设计已创建 Agent 更换 Loader 或模板时的完整迁移流程。

预装旧版本已生成的 Jarvis 如何迁移，不由本向导自动推断或处理。不得在升级时未经用户确认删除、重建或覆盖已有身份、消息和运行数据。

### 1.4 与既有文档的关系

- 《Users & Agents PRD》§9.4 只保留了 Add Agent 入口并列出完整流程的大致范围，本文是它的细化。两处不同以本文为准：本期 Owner 固定为创建者，不提供“设置 Owner”；创建成功后的落点按本文 §9 执行。
- 《用户类型》中 Admin、User、Limited 的划分，是本文 §5.3 权限上限和 §3.4 创建资格的依据。
- 《Jarvis Build-in 能力思考》提出 Jarvis “默认只响应主人，即使在群聊中也默认只听主人”，本文 §5.4–§5.5 将其落为可验收的默认通信规则。
- MessageHub PRD 中 tunnel 与 Session 的模型不因本文改变。本文只规定 Agent 创建时如何绑定一个外部通道。

## 2. 核心模型与术语

### 2.1 概念定义

| 概念 | 定义及产品含义 |
| --- | --- |
| Agent Identity | Agent 在 BuckyOS 网络中的账号身份，包含唯一用户名、DID、Profile 及权限关系；不等同于某个模板或某个运行进程。 |
| Owner | 拥有该 Agent 的用户。本期创建流程默认由当前用户为自己创建，Owner 即当前创建者，不增加代他人指定 Owner 的步骤。 |
| Zone Owner | Zone 的所有者角色。与“某个 Agent 的 Owner”不是同一个概念。 |
| Agent Loader | 承载、加载和运行 Agent 的服务实现。当前可选实现为 OpenDAN；符合 Message Hub 接入要求的其他服务可以形成扩展实现。 |
| Agent RootFS 模板 | 用于构建 Agent 实例的提示词、配置及相关文件集合。Jarvis 是预装模板，不是预先存在的 Agent 身份。 |
| Agent 实例 | 创建完成后，由身份、Owner、权限、Loader、模板与实例配置共同构成的可运行 Agent。 |
| Message Hub | Agent 接收和处理消息所依托的系统消息接入机制。 |
| Message Center | 系统内置的网页消息交互界面，用户完成 Agent 创建后即可通过它与 Agent 交互。 |
| Message Tunnel | 接入 Telegram 等外部应用的消息通道，不建立第二套 Agent 身份或独立权限体系。 |
| PIKG | 本文对可安装软件包的统一称呼。模板通过现有 App Service／系统包安装器安装；界面名称应与系统现有产品保持一致。 |

“Jarvis Runtime”在本流程中统一改称 **Jarvis RootFS 模板**，面向普通用户的字段标签可简化为“Agent 模板”。OpenDAN 是 Loader 实现，Jarvis 是模板，两者不与 Agent 实例的身份混用。

**运行配置是一个阶段，不应再拆成彼此重复的 Runtime 选择和 Loader 选择。** 本期只需要选择 Agent Loader，再选择与其兼容的 RootFS 模板。

### 2.2 身份与实现分离

```text
Agent 身份：用户名 / DID / Profile / Owner / 权限
                         │
                         └── 关联运行配置
                                  ├── Agent Loader：OpenDAN 等
                                  ├── Agent RootFS 模板：Jarvis 等
                                  └── 实例设置及可选 Message Tunnel
```

Agent 的网络身份应保持稳定。运行实现和模板是可扩展、可调整的关联配置，不能因为桌面显示名变化、模板更新或运行进程重启而自动生成另一个身份。

同一模板可用于创建多个不同身份的 Agent。它们的账号、Owner、实例设置和运行数据不能因为共享模板而混用。

### 2.3 预装边界

| 系统预装的内容 | 系统不应代替用户完成的内容 |
| --- | --- |
| OpenDAN Loader／运行能力 | 创建具体 Agent 的账号与 DID |
| Jarvis RootFS 模板 | 将具体用户与 Agent 建立所有权及授权关系 |
| 桌面上的 Jarvis 初始化引导入口 | 替用户确认个性化身份、共享范围及外部通道配置 |

预装的模板和引导入口面向 Zone 内每个可以创建 Agent 的用户，而不只面向 Zone Owner（见 §3.4）。

Zone 激活向导同样遵守这条边界。激活时还没有任何 Agent 身份，因此不再收集 Jarvis 的 Telegram Bot Token 等 Agent 通道参数，这些参数改在 Agent 创建向导的第三步填写。激活时仍可选填 Owner 本人的 Telegram 账号：它属于 Owner 的 User Profile，正好供第三步检查使用。

## 3. 范围、默认值与统一入口

### 3.1 本期选项基线

以下状态以本轮需求讨论为基线，表示本期要达到的选项，不描述当前代码。当前实现见附录 A。

| 配置项 | 本期要求 | 扩展方向 |
| --- | --- | --- |
| Agent Loader | 下拉框仅有 OpenDAN 可选 | 增加其他兼容 Message Hub 的 Agent Service |
| RootFS 模板 | 默认安装且可选的是 Jarvis | 安装其他兼容模板后可在同一下拉框选择 |
| 系统内消息入口 | 创建后可使用 Message Center | 不依赖外部通道才能使用 |
| 外部消息通道 | Telegram 可配置 | 向导本期只添加一个，系统模型允许多个 |
| 规划中的通道 | 展示 Lark/飞书，标注“暂未支持”且不可选择 | 未来实现及包扩展后启用 |

Lark/飞书作为一个规划通道项展示，具体接入实现及命名由通道模块统一；本期不额外扩展其他渠道的支持承诺。

### 3.2 默认值

| 项目 | 默认行为 |
| --- | --- |
| 昵称 | 从 Jarvis 引导入口进入时预填 Jarvis，允许修改或留空 |
| 用户名 | 可给出建议值，但必须由统一账号规则校验可用性；不预先占用账号 |
| 头像 | 使用模板或系统默认头像，鼓励用户更换，但不强制上传 |
| 公开简介 | 可预填模板介绍，允许修改或留空 |
| 角色设定补充 | 可留空；未填写也应能正常运行 |
| 系统操作权限 | 默认在创建者权限范围内对齐创建者能力，允许高级调整，但不得越权 |
| 允许其他用户使用 | **关闭** |
| 允许加入群聊 | **关闭** |
| Loader | OpenDAN |
| RootFS 模板 | Jarvis |
| 自动应用 RootFS 模板更新 | **开启** |
| 外部 Message Tunnel | 不默认绑定；整个步骤可跳过 |

群聊开关采用默认关闭，是为了让用户主动决定是否将私人 Agent 带入群聊，也避免“已加入群但其他成员不能使用”带来的认知与社交压力。它与“允许其他用户使用”独立配置。

### 3.3 两个入口使用同一套向导

| 入口 | 进入方式 | 差异 |
| --- | --- | --- |
| 桌面预装 Jarvis 图标 | 尚未关联已创建实例时进入 Add Agent | 带入 Jarvis 的预填信息与入口来源 |
| Users and Agents → Add Agent | 用户主动添加 Agent | 使用同一向导；可使用系统默认值，不强制始终选择 Jarvis |

不得分别维护“Jarvis 专属初始化”和“通用 Add Agent”两套流程。两者的表单、校验、权限规则、创建操作和错误处理必须复用。

### 3.4 创建资格与数量

| 项目 | 要求 |
| --- | --- |
| 可以创建的用户 | Zone Owner、Admin 和普通 User 都可以为自己创建 Agent，规则与普通用户为自己安装 App 一致：只能为自己创建，不需要管理员，也不需要提权确认。有效权限按 §5.3 受创建者上限约束。 |
| Limited 用户 | 本期不能创建 Agent：桌面上不显示 Jarvis 引导入口，Users and Agents 的 Add Agent 显示禁用原因。 |
| 引导入口的范围 | 每个可以创建 Agent 的用户，在自己的桌面上看到自己的 Jarvis 引导入口，并能在向导中选到 Jarvis 模板。 |
| 每个用户的 Agent 数量 | 产品模型允许同一用户创建多个 Agent，包括用同一模板创建多个。 |
| 同一模板创建多个 | 本期即支持。每个 Agent 都基于所选模板构造一个属于它自己的运行时 App，各自独立运行；不同 Agent 的权限不同，因此刻意隔离。它们的身份、资料、会话、数据和模板更新设置互不影响。 |

## 4. 用户流程与页面组织

### 4.1 逻辑流程

```text
桌面 Jarvis 引导入口 / Users and Agents → Add Agent
                         │
                         ▼
              第一步：身份与权限
                         │
                         ▼
              第二步：运行配置
              Loader → RootFS 模板 → 更新策略
              展示必要配置摘要
                         │
                         ▼
              第三步：外部消息通道（可跳过）
                  ├── 不添加：跳过
                  └── 添加：检查 Owner 社交身份并填写参数
                         │
                         ▼
                    确认创建
                         │
                         ▼
               创建中 → 成功 / 失败原因
                         │
                         ▼
               成功后关闭：更新桌面入口
```

前两步完成，表示已经收集齐使用内置 Message Center 所需的必要配置，**不表示 Agent 已经创建或启动**。只有最终提交并成功后，才展示“可以开始使用”。

### 4.2 页面与组件的关系

上述是产品逻辑顺序，不要求前端必须使用固定数量的物理页面。

为复用标准用户管理，第一步可以拆为“账号资料”和“Agent 权限”两个子页面：先复用普通账号表单，再展示 Agent 特有设置。不要因为组件拆分重复收集用户名、头像等信息，也不要在对外流程中增加含义重复的步骤。

进入确认页之前，用户应能返回修改已填信息。选择模板、跳转安装器或进入个人资料补充社交身份后返回时，应尽量保留当前向导草稿。

### 4.3 加载、空态与异常态

各步都要处理以下状态，不能只设计数据齐全时的页面：

| 场景 | 页面行为 |
| --- | --- |
| 用户名正在检查 | 字段旁显示检查中；检查完成前不能进入下一步 |
| 用户名不符合规则或已被占用 | 在字段下说明原因；建议值已被占用时，给出另一个可用的建议，例如附加 Owner 用户名 |
| 没有可用的 Loader，或 Loader 列表读取失败 | 说明当前系统没有可用的 Agent Loader，不能继续；提供重试 |
| 所选 Loader 没有兼容模板 | 模板下拉框显示空态，说明原因，并提供“安装其他 Agent 模板”入口 |
| 模板列表读取失败 | 显示失败原因与重试，不回退成写死的 Jarvis 选项 |
| Owner 社交身份正在检查或检查失败 | 对应通道暂不可选；失败时提供重试，仍可跳过此步 |
| 头像上传失败 | 保留原头像并说明原因，不阻止用户使用默认头像继续 |
| 登录会话过期 | 提示重新登录；重新登录后回到当前步骤，草稿不丢失 |
| 创建请求被拒绝（如 Limited 用户、用户名在提交时已被占用） | 停留在确认页并说明原因，可返回修改，不丢失任何输入 |

## 5. 第一步：身份与权限

### 5.1 身份与 Profile 表单

表单复用标准账号的字段、布局、校验与资源选择组件。

| 字段 | 必填规则 | 交互及约束 |
| --- | --- | --- |
| 用户名 | 必填 | 使用系统既有的英文账号命名规则；与 Zone 内其他用户、Agent 共用账号唯一性约束，不另建 Agent 专用的重名空间。Agent 的用户名会成为它的主机名，因此还必须是合法的 DNS 标签，并且不能与已分配的 App 主机名重复。 |
| DID | 系统生成 | 用户输入有效用户名后展示 DID 预览；具体格式遵循现有身份规则，不在本流程中另创 DID 格式。最终创建时再次校验。 |
| 昵称 | 可选，鼓励填写 | 支持中文、日文等用户熟悉的语言；可与模板名称不同；不承担账号唯一性职责。 |
| 头像 | 有默认值 | 优先展示所选模板默认头像，允许使用已有头像组件修改。 |
| 公开简介 | 可选 | 供其他用户查看该 Agent 身份时了解其用途；是否可见仍遵循账号 Profile 的既有可见性规则。 |
| 角色设定补充 | 可选 | 作为 Agent 基础提示词的实例级补充，不是公开简介；不能由此赋予系统权限。 |
| Owner | 自动确定 | 展示当前创建用户，不与 Agent 用户名或 Zone Owner 混淆。 |

**头像与用户名是识别不同 Agent 的关键要素。** 不能因为当前模板只有 Jarvis，就把所有用户创建的实例固定为同一用户名或不可替换的头像。

昵称的显示规则统一为：

```text
显示名称 = 非空昵称；未设置昵称时回退为用户名
```

回退不使用 DID，也不强制回退到模板名称 Jarvis。本文所指用户名是账号标识，不是内部数据库主键。

### 5.2 公开简介与角色设定补充分开

公开简介回答“别人看到的这个 Agent 是什么”；角色设定补充回答“这个 Agent 默认如何与我工作”。两者必须分别保存、分别展示，不能将公开简介直接当成全部角色提示词，也不能将角色设定补充直接发布到公开 Profile。

角色设定补充示例：

```text
默认使用日语回答。
优先给出结论，再提供必要的解释。
与我交流时语气自然，不使用过多客套话。
```

Jarvis 模板自身必须具有可工作的基础提示词。用户不填角色补充也能完成创建，不能因此被要求手工编写一套完整 Agent Prompt。

### 5.3 系统权限

核心规则：

> 用户创建的 Agent，其有效权限不得超过该用户自身可授予的权限范围。

Zone Owner 创建的 Agent 可以拥有相应 Zone 管理能力；管理员创建的 Agent 可以承担其权限范围内的管理操作；普通用户创建的 Agent 不得获得管理员或 Zone Owner 权限。

默认配置应支持用户通过 Agent 进行原本能在控制面板完成的系统操作。高级模式允许收窄和调整权限，复用标准账号的权限面板与后端校验，不另行构建互不一致的 Agent 权限体系。

本期不提供收窄权限：向导只读展示“此 Agent 的权限与你一致，不会超出你的权限”。权限上限由现有鉴权机制保证，Agent 运行时的每次访问都同时校验运行时应用和 Owner 的权限。

不得只在界面中隐藏高权限选项。创建请求和后续工具执行仍需由系统权限机制约束。Agent 的角色补充、模板提示词或外部请求都不能提升这项权限上限。

### 5.4 两个通信开关

| 开关 | 默认 | 关闭时 | 开启时 |
| --- | --- | --- | --- |
| 允许其他用户使用此 Agent | 不勾选 | 非 Owner 的请求不触发 Agent，直接过滤且不作响应 | 展开共享访问策略，按发送者分组、允许类别与标准流程处理 |
| 允许此 Agent 加入群聊 | 不勾选 | 不接受群聊使用；群消息不能触发 Agent | 允许通过系统现有群聊机制加入，并继续执行触发与权限规则 |

允许加入群聊不等于允许其他成员使用，也不等于自动加入某个群。用户可以只打开群聊开关，保持 Owner-only。

群聊开关关闭时，Owner 发出的入群邀请也不能被自动接受，系统应提示 Owner 先打开这个开关。开关打开后，Owner 邀请 Agent 入群可以沿用现有的自动接受规则，其他人的邀请仍需 Owner 确认。

### 5.5 Owner-only 的触发规则

| 场景 | 处理规则 |
| --- | --- |
| Owner 直接向 Agent 发送消息 | 允许触发 |
| 非 Owner 直接发送消息，即使知道 Agent DID | 直接过滤，不进入常规 Agent 推理或执行流程，不回复 |
| 未允许加入群聊时收到群消息 | 不允许触发 |
| 已允许加入群聊，Owner 在群内明确 @Agent | 允许触发，并附带当前群中适当范围的可见上下文 |
| 已允许加入群聊，Owner 发言但没有 @Agent | 默认不触发 |
| 已允许加入群聊，非 Owner 在群内 @Agent | Owner-only 模式下仍不触发 |
| 群内其他人的历史消息出现在 Owner 请求的上下文中 | 可以作为理解当前任务的资料，不因此获得独立触发权或 Owner 权限 |

群聊请求不能只传入 Owner 的一条孤立消息。Message Hub／上下文组装层需要携带该群内允许读取、与当前消息关联的上下文；具体条数、时间窗口和裁剪预算在技术设计中确定。

触发者身份和上下文中的发言者身份必须保留。引用、转发、消息正文中的姓名或“我是主人”的声明，不能代替消息来源验证。

### 5.6 共享访问高级设置与 Router

打开“允许其他用户使用”后，界面应提供一套可直接使用的默认共享策略，并允许高级调整。配置需要说明不同用户分组可以请求哪些事情、可以获知哪些信息，以及应进入哪类标准处理流程。

```text
非 Owner 请求
    → 校验真实来源与通道身份
    → 识别发送者分组及允许范围
    → 低成本 Decision Model / Router 进行请求分类
    → 允许类别：进入对应标准处理流程
    → 未匹配、未授权或不能可靠分类：拒绝处理
```

共享模式不是把 Agent 的全部能力直接开放给其他人。Router 的职责是先分类、再路由，减少无必要的高成本主模型调用。

提示词可用于描述分组约定、信息边界与标准处理方式，但**提示词和模型分类不是底层权限检查的替代品**。即使某条消息被分入允许类别，读取数据和执行工具仍需按该请求人的授权范围处理，不能借用 Owner 的全部权限。

具体默认类别、用户分组映射及 Decision Model 选型另行定义。本 PRD 不将讨论中提到的候选模型固化为依赖，也不假定其实际成本与识别效果已经验证。若当前实现尚无可用默认共享策略，不得以“全部放行”作为替代。

默认共享策略交付之前，“允许其他用户使用”开关显示为暂未支持且不可开启，处理方式与 §7.2 中未实现的通道一致。

## 6. 第二步：运行配置

### 6.1 页面目标与字段

页面标题建议为“运行配置”，依次配置以下内容：

| 字段 | 类型 | 默认与说明 |
| --- | --- | --- |
| Agent Loader | 必选下拉框 | 当前为 OpenDAN，即使只有一个实现也保留选择结构 |
| Agent 模板 | 必选下拉框 | 默认 Jarvis，底层对应 RootFS 模板；仅展示可用且兼容的已安装模板 |
| 模板信息 | 只读详情 | 展示模板名称、简介、版本等系统已有元数据，帮助区分模板与实例 |
| 自动应用模板更新 | 复选框 | 默认勾选 |
| 配置摘要 | 只读 | 汇总已填写的身份、权限和运行选择，提示外部通道为可选步骤 |

本期不因为“运行配置”一词额外增加模型供应商、模型 API Key 或推理模型选择的必填页面。这些不属于本轮确认的初始化步骤。

### 6.2 Loader 扩展说明

Loader 旁提供问号／帮助入口，解释：

- OpenDAN 是当前提供的 Agent 加载运行实现。
- 企业或高级开发者可以编写其他符合 Message Hub 接入要求的 Agent Service。
- 开发指南应覆盖服务接入、身份与权限衔接、消息收发及调试方法，并可包含 Mock Test Agent Service 示例。

当前未安装或尚未支持的 Loader 不得显示为可选的有效实现。开发指南入口应指向实际可用内容；未交付的指南用明确状态说明，不放置失效跳转。

### 6.3 模板选择与安装入口

选定 Loader 后，下拉框读取与其兼容的已安装 Agent RootFS 模板。当前仅有 Jarvis，但控件必须真实连接可用模板数据，不能是视觉上的假下拉框。

模板列表旁提供“安装其他 Agent 模板”的说明与入口，导向现有 App Service／包安装器。安装后返回向导，应能够刷新列表并选择新模板，尽量不丢失前面填写的身份和权限草稿。

本期通过 buckyos CLI 安装 Agent 模板，App Service 中的安装界面待各类 PIKG 安装模式稳定后再做。在此之前，该入口给出 CLI 安装说明，并提供刷新模板列表的操作。

选择模板后，应保留用户已经主动编辑的昵称、头像、角色设定补充等实例配置。不得不经提示用模板默认值覆盖用户输入。

### 6.4 自动应用 RootFS 模板更新

开关建议文案：**自动应用此模板的更新**。

- **勾选，默认行为：** 当对应的已安装 PIKG 模板包更新后，该 Agent 跟随应用模板中的提示词与相关逻辑更新。
- **取消勾选：** 固定使用创建时选定的模板版本。此后该模板包发布或安装新版本，不自动改变这个 Agent 所使用的模板内容。

这里的“自动应用”指跟随已安装模板包的更新，不等于同时启用模板包下载、全系统自动升级或任意来源的远程更新。

取消勾选时展示风险提示，但不强制禁止：

> 关闭后，此 Agent 将固定使用当前模板版本，不再自动获得后续提示词、行为优化及相关修复。你需要自行维护这部分更新。

必须区分模板内容与实例数据。模板更新不得重置 Agent 的 DID、Owner、头像、昵称、角色设定补充、权限、通道绑定、消息历史或记忆等实例数据。固定版本也必须具有实际保留和加载能力，不能只在界面记录一个开关，而底层仍引用不断变化的“最新模板”。

模板与实例之间采用复制、引用、分层继承还是其他组织方式，以及更新切换和失败恢复的技术机制，由实现方案确定。

### 6.5 第二步结束时的提示

建议展示：

> 必要配置已完成。完成创建后，你即可在 BuckyOS Message Center 中与「显示名称」交流。下一步可选绑定 Telegram 等外部消息通道。

这里不能提前显示“Agent 已创建”或“现在可以使用”。实际创建统一由最终确认动作触发，或由后台受控地完成已暂存的创建流程。

## 7. 第三步：外部 Message Tunnel，可跳过

### 7.1 定位

外部通道用于让用户在 Telegram 等应用中使用同一个 Agent。它不是 Agent 创建的前置条件，也不替代内置 Message Center。

本期向导最多添加一个通道。底层需要允许一个 Agent 关联多个通道，后续添加和管理可进入既有管理界面，不在本向导中设计多通道编排。

页面必须有清楚的“跳过”操作。不添加通道时，不要求用户补充任何外部账号或凭据。

### 7.2 通道选择及可用状态

| 通道 | 状态 | 交互 |
| --- | --- | --- |
| Telegram | 按当前实现提供配置 | 先检查 Owner 的对应社交身份，满足条件后可选择 |
| Lark/飞书 | 规划中，暂未实现 | 展示禁用项和原因，不允许提交为有效配置 |

未实现的项使用明确的不可用样式，不做成点击后无说明地跳回其他选项。通道选择旁设置帮助入口，说明未来可通过 PIKG 扩展支持的 Message Tunnel；不将未来扩展能力描述为本期已经实现。

### 7.3 先检查 Owner 的社交身份

添加通道之前，系统必须检查**当前 Agent Owner 的 User Profile**是否具有该渠道可用于发送者识别的社交身份信息。

```text
进入通道步骤 / 检查某通道
    │
    ├── 通道未实现
    │      └── 不可选择，显示“暂未支持”
    │
    ├── 通道已实现，但 Owner 未配置对应身份
    │      └── 不可选择，说明原因并引导到用户个人资料
    │
    └── 通道已实现，Owner 身份满足条件
           └── 允许选择 → 填写通道参数 → 检查配置
```

不能把 Owner 的社交身份与 Agent 的通道参数混为一谈。例如，Telegram 场景同时涉及：

- Owner 的 Telegram 账号身份：用来判断消息是否由主人发出。
- Agent 的 Telegram 通道配置：用来建立 Agent 的收发入口。

只填 Agent 通道参数，不能证明系统已经知道主人在这个通道里是谁。

### 7.4 缺少身份时的反馈

建议文案：

> 你尚未在个人资料中配置 Telegram 身份，系统无法识别哪些 Telegram 消息来自你。请先补充该身份，或跳过此步骤，使用内置 Message Center。

提供“前往个人资料”及“跳过此步骤”。补充身份后返回，应重新检查条件；不得要求用户从头重填整个 Agent 向导。

仅有相似昵称、头像或消息正文中的自称，不足以建立 Owner 映射。应复用系统已有的社交身份标识及校验机制，具体身份字段与绑定可信度由通道实现说明。

### 7.5 参数填写与状态提示

通道参数按照对应通道已有配置要求动态展示，不在通用向导中硬编码所有渠道的字段。涉及 Token 等秘密信息时应遮蔽显示，不能出现在最终摘要、普通错误提示或公开 Profile 中。

页面应区分以下状态，不能只用一个笼统的勾选标识：

| 状态 | 含义 |
| --- | --- |
| Owner 身份已配置 | User Profile 中已有可用于该通道识别的身份 |
| 通道参数已填写 | 必要字段完整，仍可能需要进一步校验 |
| 配置检查通过 | 已通过当前实现所支持的检查，不代表已完成最终创建 |
| 通道已绑定 | 最终创建过程已经完成该项绑定 |

第三步点击下一步前，只校验用户本次选择提交的通道。选择“跳过”意味着最终不提交通道绑定，不能留下半填写却被自动启用的通道。

### 7.6 权限一致性

外部通道接入后的消息必须沿用第一步确定的权限。

Owner-only 模式下，只有通过可信通道来源信息映射到 Owner 的消息可以触发 Agent；其他消息直接过滤。无法识别发送者时不能默认视为 Owner，也不能为了“先用起来”临时放开所有人。

即使用户打开了共享访问，本期添加通道仍要求先配置 Owner 的渠道身份，不能跳过主人识别这项前提。共享访问是在主人身份明确后的额外策略，而不是识别失败的补救方式。

## 8. 确认、创建与状态反馈

### 8.1 最终确认页

统一展示此前收集的信息，使用户能够在提交前核对：

| 摘要分组 | 展示内容 |
| --- | --- |
| 身份 | 头像、显示名称、用户名、DID 预览、Owner、公开简介 |
| 角色 | 是否填写角色设定补充，可展开查看；不标为公开资料 |
| 系统权限 | 权限角色与高级配置摘要，明确不能超出创建者范围 |
| 通信权限 | 是否仅 Owner 可用，是否允许加入群聊，是否启用共享策略 |
| 运行配置 | Loader、RootFS 模板、选定版本、是否自动应用更新 |
| 外部通道 | 未添加，或通道类型、Owner 身份检查结果及非敏感配置摘要 |

操作为“上一步”和“创建”。提交前再次检查关键字段及当前用户授权状态。用户名在表单中通过检查，不代表最终提交时一定仍可用。

### 8.2 点击创建后的用户体验

点击“创建”后留在当前流程中，进入创建状态面板，显示加载动画与可理解的状态。创建期间禁止重复提交。

创建工作应覆盖以下逻辑事项；它们不是强制规定的 API 调用顺序：

1. 校验并建立 Agent 账号、DID 和 Owner 关系。
2. 应用 Profile、角色设定补充、系统权限与通信策略。
3. 关联 Loader、RootFS 模板和更新策略，准备实例运行配置。
4. 配置用户选择的外部通道；未选择则跳过。
5. 启用 Agent，并确认内置 Message Center 的消息入口已具备可用条件。
6. 持久化创建结果及对应入口关联。

状态面板至少覆盖：

| 状态 | 页面行为 |
| --- | --- |
| 创建中 | 显示加载状态，防止重复点击，不提前显示成功 |
| 创建成功 | 显示成功标识、最终显示名称及“关闭”操作 |
| 创建失败 | 展示具体失败事项与可理解的原因，提供与实际状态相匹配的处理操作 |

“账号已经写入”不等于“Agent 创建成功”。不能在运行配置或必要权限仍未就绪时向用户报告全部完成。

### 8.3 失败与部分完成

以下是为确保流程可交付而补充的工程要求，不指定必须使用事务或哪种任务框架。

- **用户名冲突：** 明确定位到用户名字段，允许修改后继续，保留其他输入。
- **权限校验失败：** 显示当前可授予的范围，返回权限配置，不自动升级账号权限。
- **模板或 Loader 不可用：** 保留身份草稿，提示重新选择或先安装所需组件。
- **实例准备或启动失败：** 不显示可用成功，不切换桌面图标为一个不可用的最终入口。
- **外部通道失败：** 展示 Agent 与通道各自状态；允许重试，或由用户明确选择不绑定通道继续。不得静默忽略用户选定通道的失败。
- **重复提交、超时后重试：** 复用同一次创建结果或恢复同一创建流程，不能生成重复账号、重复实例或重复通道。

如果已有基础账号或实例被创建，后续失败必须能够恢复或清理。不能把一个半初始化 Agent 当作普通账号永久留在系统中，同时让向导下一次因为用户名被占用而无法继续。

### 8.4 暂存、取消与恢复

产品逻辑上，最终“创建”是用户提交的明确节点。前面的表单优先用于收集与校验配置。

若复用已有账号创建接口，需要在较早步骤就生成账号对象，也允许采用该实现方式，但必须将其纳入统一创建流程：标记为未完成、不可对话，不以默认高权限提前对外运行，并在取消或失败时有明确的清理／恢复机制。

页面刷新或连接中断后，界面应以服务端实际状态为准查询创建结果，不以重新提交所有步骤作为唯一恢复手段。不得在不知道实际结果时直接声明失败并重新生成身份。

创建进行中关闭向导窗口，不等于取消创建。后台按已提交的内容继续执行；用户再次点击引导入口时，回到这次创建的状态面板，而不是开始新的向导。

## 9. 创建成功后的桌面闭环

### 9.1 图标从引导入口变成实例入口

用户从桌面 Jarvis 图标进入，完成创建并点击“关闭”后，原位置的入口完成以下变化：

| 元素 | 创建前 | 创建后 |
| --- | --- | --- |
| 显示名称 | Jarvis | Agent 的昵称；没有昵称时使用用户名 |
| 图标图像 | Jarvis 默认图标 | 用户最终选择的 Agent 头像 |
| 目标 | 带预填参数的 Add Agent 向导 | 本次创建的 Agent 的主页：优先打开 Loader 提供的主页（OpenDAN 为其 WebUI 首页）；Loader 不提供时，打开 Users and Agents 中该 Agent 的详情页 |
| 关联对象 | 初始化入口／模板预设 | 稳定的 Agent 实例标识 |

示例：用户设置用户名为 `xiaobai`、昵称为“小白”，并选择自定义头像。创建成功关闭后，桌面原来的 Jarvis 图标显示为“小白”及新头像；再次点击进入小白的 Agent 主页。

该变化体现为**快捷入口的目标关联从创建引导变成 Agent 实例**，同时更新显示名称和图像。它不改变 Agent DID，也不使用昵称作为快捷入口的内部主键。

### 9.2 实例判断与多用户隔离

入口需要知道自己是否已有明确关联的 Agent 实例。有有效关联时直接打开实例；没有关联时进入创建引导。不能只按“系统里是否存在一个叫 Jarvis 的账号”判断完成状态。

一个 Zone 可以有多个用户及多个 Agent。桌面快捷入口的关联需要处于正确的用户／桌面作用域，不能因用户 A 创建“小白”，就把用户 B 的 Jarvis 引导入口替换成 A 的私人 Agent。

多个实例可能采用相同昵称或同一模板，身份与入口定位仍必须使用稳定标识，不能凭头像、昵称或模板名匹配。

### 9.3 其他入口创建时的边界

通过 Users and Agents 创建 Agent，同样使用上述创建逻辑，成功后在账号管理中显示该实例。创建另一个 Agent 不得覆盖桌面上已经关联好的实例入口。

本期必须交付的是“从预装桌面引导进入后，成功创建并替换该入口”的闭环。通过管理面板创建额外 Agent 时，是否自动新增快捷方式、是否可主动设为默认 Agent，不在本期强制范围内。

### 9.4 入口状态

| 入口状态 | 判断依据 | 点击行为 | 显示 |
| --- | --- | --- | --- |
| 未创建 | 没有关联实例，也没有进行中的创建 | 打开带 Jarvis 预填信息的向导；有草稿时恢复草稿 | Jarvis 默认名称与图标 |
| 创建进行中 | 已提交，尚未得到最终结果 | 打开这次创建的状态面板 | Jarvis 默认名称与图标，可附加进行中标识 |
| 创建失败 | 已提交，结果为失败 | 打开状态面板，显示失败事项，提供重试或取消 | Jarvis 默认名称与图标，可附加失败标识 |
| 已关联 | 关联的 Agent 实例存在且可用 | 打开该实例的 Agent 主页 | Agent 的显示名称与头像 |
| 已关联但停用或运行异常 | 关联实例存在，但已停用或不健康 | 仍打开实例主页，并在主页显示状态 | Agent 的显示名称与头像 |
| 关联实例已删除 | 关联的 Agent 已被删除 | 回到“未创建”，可以重新创建 | 恢复 Jarvis 默认名称与图标 |

实例停用或运行异常时，入口不能退回引导状态，否则用户会再创建一个重复的 Agent。

Agent 的昵称或头像在 Agent 主页、Users and Agents 等处修改后，桌面入口应同步更新。

## 10. 数据边界与复用要求

### 10.1 产品数据分组

以下描述产品需要区分的信息，不规定字段名、表结构或请求格式。

| 数据组 | 主要内容 | 归属与边界 |
| --- | --- | --- |
| 账号身份 | 账号类型、用户名、DID、Owner | 复用标准账号系统，明确类型为 Agent |
| Profile | 昵称、头像、公开简介 | 复用用户 Profile 管理与可见性规则 |
| 实例角色补充 | 用户提供的默认语言、风格等指令 | Agent 私有实例配置，不混入公开简介或可共享模板 |
| 系统授权 | 角色、权限范围、创建者约束 | 复用标准权限机制，操作时继续检查 |
| 消息访问策略 | Owner-only、群聊开关、共享分组与路由规则 | Agent 通信策略，所有入口一致执行 |
| 运行配置 | Loader 选择与必要关联信息 | 指向实际加载运行实现 |
| 模板关联 | 模板标识、使用版本、自动应用更新设置 | 区分模板与实例数据，支持实际版本冻结 |
| 通道绑定 | 通道类型、配置、Owner 社交身份引用 | 复用通道模块，秘密信息单独保护 |
| 创建状态 | 创建操作进度、结果、失败事项及关联资源 | 用于恢复、重试、防止重复创建 |
| 桌面关联 | 入口来源、所属用户／桌面、目标实例 | 不通过昵称反查身份 |

### 10.2 优先复用的能力

| 已有能力 | 本流程如何使用 |
| --- | --- |
| 普通用户账号创建 | 复用身份字段、用户名检查、DID 生成、头像与资料组件；实际创建的是 Agent 类型账号 |
| 用户权限面板 | 复用角色及高级权限配置，在创建者上限内授权 |
| Users and Agents | 作为通用创建和后续管理入口 |
| App Service／PIKG 安装器 | 用于安装其他 Agent RootFS 模板，未来承载可扩展通道相关包 |
| 用户社交身份管理 | 查询并引导配置 Owner 的通道身份，不为 Agent 向导再复制一套用户绑定页 |
| Message Hub／Message Center | 统一消息来源、触发规则和系统内交互入口 |
| Agent 主页与桌面快捷方式 | 复用已有实例主页和入口更新机制 |

底层可采用“创建 Agent 账号 → 配置 Agent 特有权限 → 关联运行实现 → 配置通道”的顺序。组件复用不能成为权限暂时放开、错误状态被掩盖或产生孤立账号的理由。

### 10.3 权限与上下文的实现约束

Owner-only 的来源检查应在进入常规 Agent 推理与工具执行前完成，而非让 Agent 先读完消息后再自行决定是否回应。

群聊上下文必须携带其来源语义：Owner 的直接请求与其他成员的历史内容不能合并为具有相同指令权限的文本。共享模式同样不得把分类器的结果直接当作高权限执行凭证。

创建时的权限配置不是永久绕过后续鉴权的通行证。后续系统操作继续服从现有账号、委托和工具授权规则，避免因缓存创建时的权限而获得已失效的能力。

## 11. 页面文案与交互一致性

### 11.1 建议文案

| 位置 | 建议文案 |
| --- | --- |
| 引导页说明 | 为你创建一个专属 Agent。设置身份和权限后，即可通过 Message Center 使用。 |
| 昵称说明 | 显示给你和其他用户的名称，可以使用中文等语言；不填写时显示用户名。 |
| 公开简介说明 | 介绍这个 Agent 的用途，按个人资料可见性规则展示。 |
| 角色补充说明 | 告诉 Agent 你期望的默认语言和交流方式。不填写也能正常工作。 |
| 共享开关说明 | 默认只有你能触发此 Agent。开启后，将按共享策略处理其他用户的请求。 |
| 群聊开关说明 | 允许加入群聊不代表其他人可以使用；私人模式下仍需由你在群内 @它。 |
| 模板字段帮助 | 模板提供 Agent 的基础提示词和运行文件，不决定它的用户名或昵称。 |
| 模板安装入口 | 安装其他 Agent 模板 |
| 第三步标题 | 绑定外部消息通道（可跳过） |
| 第三步跳过 | 暂不添加，使用 Message Center |
| 规划通道状态 | 暂未支持 |
| 创建按钮 | 创建 Agent |
| 成功提示 | 「显示名称」已创建，可以开始使用。 |

### 11.2 一致性与可用性要求

必填字段、禁用原因、错误定位、帮助入口和返回操作应与系统标准账号表单保持一致。不可用选项需同时有文字说明，不能仅靠颜色表达。

角色补充、共享访问高级规则和开发者帮助默认不占据主流程的大量空间。普通用户应能主要关注身份、权限提示及默认运行选择，高级用户再按需展开。

长昵称、多语言昵称、无昵称和默认头像均需正常展示。确认页、桌面图标、Users and Agents、Agent 主页，以及 Message Center 中的联系人与会话名称，应采用相同的显示名称规则。

## 12. 验收标准

以下用例以产品行为为准，不依赖某一种前端页面拆分或后端 API 设计。

| 编号 | 场景 | 验收结果 |
| --- | --- | --- |
| AC-01 | 全新系统完成预装，但用户未启用 Agent | 存在 OpenDAN、Jarvis 模板及引导入口；不因预装自动建立具体 Jarvis 身份 |
| AC-02 | 点击尚未关联实例的 Jarvis 桌面图标 | 进入与 Add Agent 相同的向导，并带入 Jarvis 预填信息 |
| AC-03 | 从 Users and Agents 点击 Add Agent | 使用同一套字段、校验、权限与创建逻辑，不存在功能不一致的专属流程 |
| AC-04 | 输入与现有用户或 Agent 冲突的用户名 | 按统一账号规则报错；不能创建第二个冲突账号 |
| AC-05 | 输入有效用户名 | 展示符合现有规则的 DID 预览，提交时再次校验唯一性 |
| AC-06 | 昵称填写“小白”，用户名为 `xiaobai` | 最终使用“小白”作为显示名称；昵称留空时使用 `xiaobai` |
| AC-07 | 用户不改头像、不填角色补充 | 可使用默认头像及模板基础提示词正常完成创建 |
| AC-08 | 用户分别填写公开简介与角色补充 | 两者分别存储、分别使用；角色补充不被直接发布到公开 Profile |
| AC-09 | 管理员或普通用户尝试创建超过自身权限的 Agent | 前端与后端均阻止越权；修改请求参数也不能突破上限 |
| AC-10 | Zone Owner 按默认配置创建 | Agent 可以在其可授予范围内承担系统配置，不被固定降为无法管理系统的普通助手 |
| AC-11 | 查看首次创建的通信默认值 | “允许其他用户使用”“允许加入群聊”均未勾选 |
| AC-12 | Owner-only 模式下，非 Owner 知道 DID 并直接发送消息 | 不触发常规 Agent 处理，不回复 |
| AC-13 | Owner-only 模式下，允许群聊且 Owner 在群内 @Agent | 允许触发，输入包含本群允许读取的相关上下文 |
| AC-14 | Owner 未 @Agent，或非 Owner 在群内 @Agent | Owner-only 群聊模式下不触发 |
| AC-15 | 群聊上下文包含他人指令或冒充 Owner 的内容 | 作为上下文处理，不因此获得独立触发权或 Owner 权限 |
| AC-16 | 打开共享访问 | 展开默认共享策略；非 Owner 请求按类别进入标准流程，未授权或未匹配请求被拒绝 |
| AC-17 | 打开 Loader 下拉框 | 当前仅 OpenDAN 可选；不额外要求再选择另一层 Runtime |
| AC-18 | 安装第二个兼容 Agent 模板后返回向导 | 列表可刷新并选择新模板；既有身份与权限草稿不被无故清空。本期通过 CLI 安装模板后，在向导中刷新验收 |
| AC-19 | 查看模板更新设置 | 默认勾选；取消勾选时明确提示不能自动获得后续模板更新 |
| AC-20 | 一个实例跟随模板更新，另一个实例固定版本 | 模板包更新后，前者按更新策略应用，后者继续使用固定版本；实例资料和数据均不被模板重置 |
| AC-21 | 完成运行配置，但尚未最终点击创建 | 只提示必要配置已完成，不宣称 Agent 已可使用 |
| AC-22 | 跳过外部通道并创建 | 不要求外部账号与 Token；成功后可通过内置 Message Center 交互 |
| AC-23 | Owner 未在 Profile 配置 Telegram 身份 | Telegram 不可选，显示原因和补充身份入口；仍可跳过此步 |
| AC-24 | Owner 补充 Telegram 身份后返回 | 重新检查通过后允许选择通道，不要求从头填写向导 |
| AC-25 | 只有 Agent 通道参数，没有 Owner 渠道身份 | 不允许把通道视为配置完成或绕过主人识别 |
| AC-26 | 查看 Lark/飞书选项 | 显示暂未支持且不可提交，不表现为已实现的可用通道 |
| AC-27 | 在本向导添加通道 | 最多添加一个；数据关联不排斥初始化后绑定更多通道 |
| AC-28 | 通道 Owner 映射有效后收到外部消息 | Owner 可按权限使用；无法映射为 Owner 的消息不能在私人模式下触发 |
| AC-29 | 在确认页查看摘要或在失败页查看错误 | 不泄露 Token 等秘密信息；不将草稿校验勾选误报为最终绑定完成 |
| AC-30 | 连续点击创建，或超时后重试 | 同一次操作不产生重复身份、实例与通道；页面反馈与实际状态一致 |
| AC-31 | 必要配置或启动失败 | 当前面板明确说明失败项；不报告可用成功，不把桌面引导变成失效的成功入口 |
| AC-32 | 外部通道绑定失败，但基础 Agent 已配置 | 清楚显示部分结果；可重试通道，或用户明确跳过后完成，不静默忽略 |
| AC-33 | 提前建立了账号，随后取消或后续步骤失败 | 账号保持未完成且不可对话；能够恢复或清理，不留下阻塞重试的孤立账号 |
| AC-34 | 从桌面引导创建“小白”并点击关闭 | 原位置显示“小白”及所选头像；再次点击进入实际 Agent 主页 |
| AC-35 | 用户 A 完成创建，Zone 中还有用户 B | B 的私人入口不会被改为 A 的 Agent；不靠同名 Jarvis 判断绑定 |
| AC-36 | 已有桌面绑定后再从账号管理创建其他 Agent | 原有快捷入口不被无提示覆盖；新实例在账号管理中正确显示 |
| AC-37 | 全新激活一个 Zone | 激活向导不要求填写 Jarvis 的 Bot Token 等 Agent 通道参数 |
| AC-38 | 非 Zone Owner 的普通用户登录桌面 | 能看到自己的 Jarvis 引导入口并完成创建；Agent 的权限不超出该用户 |
| AC-39 | 同一用户用同一模板创建第二个 Agent | 能正常创建并运行；两个 Agent 的资料、会话、数据和模板更新设置互不影响，删除其中一个不影响另一个 |
| AC-40 | 默认共享策略尚未交付时查看共享开关 | 显示暂未支持且不可开启，不提供“全部放行”的替代 |
| AC-41 | 群聊开关关闭时，Owner 邀请 Agent 入群 | 不自动接受，并提示需要先打开群聊开关 |
| AC-42 | 创建进行中关闭向导，再次点击引导入口 | 回到这次创建的状态面板，不开始新的创建 |
| AC-43 | 桌面入口关联的 Agent 被停用，或被删除 | 停用时仍打开实例主页并显示状态；删除后入口恢复为 Jarvis 引导 |
| AC-44 | 在 Agent 主页或 Users and Agents 修改昵称或头像 | 桌面入口、Users and Agents、Message Center 中的名称与头像按同一规则更新 |
| AC-45 | 向导各步遇到模板列表为空、读取失败或登录过期 | 按 §4.3 显示对应状态；不出现写死的假选项，重新登录后草稿仍在 |
| AC-46 | Limited 用户登录桌面并打开 Users and Agents | 桌面没有 Jarvis 引导入口；Add Agent 显示禁用原因；直接调用创建接口也被拒绝 |
| AC-47 | 删除一个 Agent 后，用同一用户名重新创建 | 新 Agent 不继承旧 Agent 的会话、记忆和本地修改 |
| AC-48 | 普通 User 为自己创建 Agent | 不需要管理员参与，也不弹出提权确认；不能为其他用户创建 |

## 13. 发布范围与待细化事项

### 13.1 本期必须交付

本期的闭环包括：取消预装代建身份、激活向导不再收集 Agent 通道参数、面向每个可创建 Agent 的用户提供模板与引导入口、统一创建向导、身份与权限配置、OpenDAN 与模板选择、有效的模板更新开关、可跳过的单通道绑定、最终创建结果反馈，以及桌面入口转换。

本期界面可以提前呈现扩展结构，但不能用不可用的空入口替代核心能力：例如，模板下拉框必须来自实际安装结果；固定版本必须真正固定；已启用的共享模式必须具备可执行的默认策略。

### 13.2 技术与交互细化项

下表的“本期结论”已经确定，具体做法见附录 B；“不得改变的产品约束”在后续迭代中同样有效。

| 事项 | 本期结论 | 不得改变的产品约束 |
| --- | --- | --- |
| RootFS 组织与版本管理 | 模板复制到每个 Agent 的 AgentRoot；关闭自动更新后不再同步，AgentRoot 中的副本就是固定版本，模板更新时也不再为它重建 App（B.5、B.7） | 自动跟随与版本固定真实有效；更新不覆盖身份和实例数据 |
| 创建一致性 | 以 Agent 安装记录作为创建状态的唯一真相，后台驱动可恢复；构造的 App 安装完成前不写 AgentSpec（B.5） | 未完成不能冒充成功；重试不能生成重复或孤立账号 |
| 同模板多实例的承载 | 每个 Agent 基于模板构造一个独立的 App，并安装成它自己的运行时实例；构造 App 的 AppDID 就是 AgentDID（B.2、B.5） | 不通过共享身份或共享实例数据来“复用” |
| Loader 与模板的兼容关系 | Loader 本期只有 OpenDAN，所有 Agent 类型的 App 都由它加载；模板来自系统内置和用户用 CLI 安装两处，列表读自模板 PIKG（B.3、B.5） | 两个下拉框都来自真实数据，不写死 |
| Agent Profile 的归属 | 存放在身份层，与 AgentSpec 并列，所有界面读同一份（B.4） | 所有界面使用同一份资料和同一显示名称规则；更换 Loader 或模板不丢失资料 |
| 角色设定补充的存放 | 存放在 Agent Settings，由 Loader 写成 AgentRoot 中不属于模板的文件后注入（B.7） | 写入补充不能导致模板文件停止接收更新 |
| 桌面入口的持久化 | 关联关系记在 Agent 自己的 Settings 中，桌面据此计算入口状态（B.9） | 按用户隔离；能显示 Agent 头像；换设备或浏览器后保持一致 |
| Agent 主页的指向 | 已定，见 §9.1 | 始终指向本次创建的实例；目标不可用时有明确回退 |
| Limited 用户的创建资格 | 本期不允许，见 §3.4 | 无论是否允许，都不能越过该用户自身的权限 |
| 激活向导的通道步骤 | 删除 Bot Token；Owner 本人的 Telegram 账号保留为可选项（B.6） | 激活时不为尚不存在的 Agent 收集 Bot Token |
| 通道身份 | 复用 User Profile 中的 Telegram 账号；msg-center 在入站时把对应的端点映射到 Owner（B.8） | 不能仅凭昵称或消息内容冒充 Owner；缺少身份时不开放通道选择 |
| 群上下文组装 | Owner @Agent 时，附带同一群会话中最近 20 条已读消息，保留发言者身份（B.7） | 不是只传单条 Owner 消息，也不让上下文变成越权指令 |
| 安装器与指南跳转 | 本期用 CLI 安装模板，“安装其他 Agent 模板”给出 CLI 说明和刷新操作；开发指南未交付时显示状态说明（B.3、B.9） | 优先复用现有入口，避免丢失草稿和失效帮助链接 |
| 存量 Agent 与额外快捷方式 | 开发阶段不做迁移，用全新安装验证；额外快捷方式本期不做 | 不擅自重建已有身份，不覆盖其他用户或其他实例 |
| 共享默认策略 | 本期不做，开关禁用 | 非 Owner 不借用 Owner 全部权限；未匹配不能默认放行 |
| 共享群聊触发 | 本期不做 | Owner-only 规则不得被改变；共享触发规则需明确后再开放 |

### 13.3 效果观察

上线后可关注从进入向导到创建成功的完成率、各步中断与错误分布、选择跳过外部通道的比例，以及创建后首次通过 Message Center 发起交互的成功情况。具体目标值应依据实际试用结果制定，不在本 PRD 中虚构基线。

如采集流程统计，仅记录必要的操作状态和非敏感错误类别，不采集角色补充正文、通道秘密信息或私人消息内容。

## 14. 最终体验定义

普通用户的默认路径是：打开桌面 Jarvis 图标，确认或修改自己的 Agent 身份，了解默认权限，保留 OpenDAN、Jarvis 模板和自动更新设置，跳过外部通道，确认创建。成功后，原来的 Jarvis 入口变成用户命名并选择头像的专属 Agent。

高级用户在同一流程中调整身份与权限、安装和选择其他模板、决定是否跟随模板更新，并在 Owner 社交身份明确的前提下接入外部消息通道。

**预装的是能力与模板；用户创建的是身份与实例。简单路径不牺牲权限边界，扩展路径不再依赖预装时写死的配置。**

## 附录 A：当前实现与本文的差距

核对日期为 2026-10-09，依据是仓库源码、manifest 和脚本，没有采用 `doc/`、`notepads/` 中的描述。路径相对于 `buckyos/src/`。

> 本附录记录的是实施前的现状。附录 B 已于 2026-10-09 实施，实施结果与偏离见附录 C。

本附录只记录现状，供排期和技术设计参考，不是对实现方式的要求。实现变化后应同步更新或删除。

### A.1 预装与身份创建

| 本文要求 | 当前实现 | 差距 |
| --- | --- | --- |
| 预装不代建身份（§2.3） | scheduler 在 Zone boot 时执行 `add_default_agents`，为 Zone Owner 生成 `jarvis.<zone>` DID、密钥和 AgentSpec，先暂存在 `system/scheduler/bootstrap_agents/`；Jarvis 应用安装完成后，由 `recover_bootstrap_agent_provisions` 在一个事务里写入 `users/<owner>/agents/<agent_id>/`（`kernel/scheduler/src/system_config_builder.rs`、`install_plan_executor.rs`） | 需要去掉 boot 阶段的代建。这套“先暂存、条件满足后一次写入、启动时可恢复”的做法，可供 §8 的统一创建流程复用 |
| 模板与引导入口面向每个用户（§2.3、§3.4） | Jarvis 应用只为 Zone Owner 预装（`frame/control_panel/src/pre_install_reconciler.rs`），其他用户没有 Jarvis 应用 | 需要按用户提供模板和引导入口 |
| 激活不收集 Agent 通道参数（§2.3） | 激活向导有 `JarvisMsgTunnelStep`，同时收集 Jarvis 的 Telegram Bot Token 和 Owner 的 Telegram 账号，由 `build_msg_center_settings` 写入 msg-center 设置（`kernel/node_active/src/components/steps/`、`system_config_builder.rs`） | 需要去掉 Bot Token 的收集 |
| 统一的创建操作（§8） | 控制面板的 `agent.create` 只允许管理员调用，写入旧的 `agents/<id>/{doc,key,settings}`，不写 AgentSpec 和运行绑定，OpenDAN 无法加载这样创建的 Agent；Owner 写成 `did:bns:<user_id>`，不是 Owner 的真实 DID（`frame/control_panel/src/user_mgr.rs`）。除 scheduler 外，没有生产代码写 AgentSpec | 需要新的创建操作：普通用户可以为自己创建，并写入 AgentSpec 与运行绑定 |
| 用户与 Agent 共用用户名空间（§5.1） | `user.create` 只检查 `users/<id>/settings`，`agent.create` 只检查 `agents/<id>/doc`；用户 DID 和 Agent DID 都是 `<name>.<zone>` 的形式，同名会冲突；`jarvis` 不是保留名 | 需要统一的唯一性检查 |
| 同模板多实例（§2.2、§3.4） | AgentSpec 绑定到模板应用的实例 `jarvis.buckyos.bns.did@<owner>`；OpenDAN 一个进程只托管一个 Agent，同一实例绑定两个 Agent 时拒绝启动（`frame/opendan/src/main.rs` 的 `wait_for_bound_agent`） | 同一用户目前只能用 Jarvis 模板运行一个 Agent。附录 B 改为每个 Agent 构造独立 App 后不再受此限制 |
| 已有 Jarvis 的处理（§1.3） | 没有检测或迁移逻辑 | 与 §13.2 的存量项一起决定 |

### A.2 Profile、权限与运行配置

| 本文要求 | 当前实现 | 差距 |
| --- | --- | --- |
| Agent 与用户共用 Profile 机制（§5.1、§10.1） | AgentDocument 没有昵称和头像字段。Jarvis 的名称来自模板 `agent.toml` 的 `display_name`，Owner 的修改存在 AgentRoot 的 `.meta/profile.json`，只有 OpenDAN WebUI 读写（`frame/opendan/src/home.rs`）。Message Center 中 Agent 的名称回退为 DID 主机名 | 需要身份层的 Profile 和统一的显示名称 |
| 复用账号表单（§5.1、§10.2） | `NewUserWizard` 只有用户名、显示名和密码，没有头像，用户类型固定为 user。桌面没有头像选择组件，OpenDAN WebUI 中有一个裁剪头像组件。Users and Agents 的文案没有接入 i18n | 需要补头像和 i18n，并把可复用的表单部分抽出来 |
| 复用权限面板（§5.3） | 没有可复用的角色或权限面板；`user.change_type` 有接口，没有界面 | 需要新建，Agent 与用户共用 |
| 权限不超过创建者（§5.3） | OpenDAN 以 `app:<app_id>@<owner>` 身份运行，RBAC 同时校验应用与用户两个主体，实际权限不超过 Owner。没有针对单个 Agent 收窄权限的能力，也没有委托机制 | 上限已经满足；“高级模式收窄权限”需要新做 |
| Loader 与兼容模板列表（§6.1、§6.3） | 没有 Loader 登记。OpenDAN 内置在 aios worker 镜像中，模板包声明 `app_type = agent` 时由该镜像加载 | 两个下拉框都没有数据来源 |
| 自动应用更新与固定版本（§6.4） | 系统内置的 PIKG 变化后，会自动提交应用更新（`frame/control_panel/src/app_installer.rs` 的 `submit_preinstall`）。OpenDAN 按文件把模板同步到 AgentRoot，本地改过的文件不再被覆盖（`frame/opendan/src/rootfs.rs`）。没有版本固定 | 固定版本需要新做。按文件同步意味着角色补充不能直接写进模板文件 |
| 角色设定补充（§5.2） | 没有对应字段或文件。身份提示词来自 AgentRoot 的 `role.md` 和 `self.md` | 需要单独存放并注入 |

### A.3 消息触发与通道

| 本文要求 | 当前实现 | 差距 |
| --- | --- | --- |
| Owner-only（§5.5、§10.3） | msg-center 按联系人等级分箱：Friend 进 INBOX，陌生人进 REQUEST_BOX，OpenDAN 只读 INBOX。Agent 的 Owner 自动成为 Friend；但只要该 Agent 绑定了 Telegram Bot，Zone 内所有用户都会被同步成它的 Friend（`frame/msg_center/src/main.rs` 的 `sync_zone_user_contacts`），这些人的消息会进入 LLM。Jarvis 的 `chat_route` 提示词默认对话者就是 Owner | 需要在推理前按 Owner 过滤，并去掉“绑定 Bot 即全员 Friend”的同步 |
| 群聊开关与 @ 触发（§5.4、§5.5） | Owner 的入群邀请会被自动接受（`frame/msg_center/src/group_service.rs` 的 `member_consent`）。Jarvis 模板没有 `msg.group` 规则，群消息停在 INBOX 不处理；`behaviors/groupchat_route.toml` 没有被引用，仍是旧结构。没有 @ 触发过滤，也不附带群上下文 | 开关、@ 触发和群上下文都需要新做 |
| 共享访问（§5.6） | 只有联系人等级和临时授权，没有分组策略、Router 或请求分类 | 交付前按 §5.6 禁用开关 |
| Owner 的 Telegram 身份映射（§7.3、§7.6） | Owner 的 Telegram 账号存在 User Profile 的 `private_extra.system_contact.bindings`，格式为 `user:<id>`。但 Telegram 入站消息总被解析成 `did:msgtunnel:...` 影子联系人，等级为陌生人，不会对应到 Owner；双方的账号格式也不一致（入站为裸 `<id>`）。结果是 Owner 本人的 Telegram 私聊也进入 REQUEST_BOX | 需要可靠的来源映射 |
| Agent 的 Bot 配置（§7.5） | Bot Token 存在 `services/msg-center/settings` 的 `telegram_tunnel.bindings`，只在激活时写入，之后没有界面或接口修改；控制面板 `agent.set_msg_tunnel` 写入的字段，msg-center 不读取；每个 Agent 最多一个 Bot | 需要能由创建流程写入的通道绑定 |
| 补充 Owner 社交身份（§7.4） | Users and Agents 中 Social Accounts 的“添加”只在前端插入示例数据，刷新即丢失；没有“Owner 是否已配置某通道身份”的专门查询 | 需要真实的添加与检查 |
| Lark/飞书（§7.2） | 不存在 | 与本文一致，只展示为暂未支持 |
| 未完成的 Agent 不可对话（§8.4） | 没有“初始化未完成”状态；`agents/<id>/settings.state` 除 `deleted` 外不起作用 | 需要新增 |

### A.4 桌面入口与界面

| 本文要求 | 当前实现 | 差距 |
| --- | --- | --- |
| 桌面入口转为实例入口（§9） | 桌面没有 Jarvis 专属图标。Jarvis 应用经 `apps.list` 出现在 Zone Owner 的桌面，点击后以内嵌网页打开它的 Web 入口，即 OpenDAN WebUI。没有修改快捷方式名称、图像或目标的接口；图标只用内置图标表，忽略应用图标地址；布局存在浏览器 localStorage，键名不含用户（`frame/desktop/src/models/layout.ts`） | 入口关联、显示 Agent 头像、按用户持久化都需要新做 |
| Add Agent 入口（§3.3） | 按钮已存在，点击只提示 “Agent creation is coming soon.”；Users and Agents 不接受启动参数，桌面图标无法直接打开向导（`frame/desktop/src/app/users-agents/`） | 需要接入向导和启动参数 |
| 安装其他模板（§6.3） | 系统对话框 `sysdlg/app_installer` 可以在任意桌面窗口内打开并返回结果，不离开当前窗口 | 可直接复用，有利于保留草稿 |
| 进入 Message Center（§6.5） | `openAppWindow('messagehub', …)` 可以按 Agent DID 打开会话 | 可直接复用 |
| 按实例标识定位（§9.2） | Users and Agents 数据层把 `agents[0]` 当作“那个 Agent”；任务管理用 `appId.includes('jarvis')` 判断；scheduler 把 Telegram 绑定的 owner 固定解析为 Jarvis DID | 支持多个 Agent 后需要改为按实例标识 |

## 附录 B：本期实现方案

本附录面向实施本期功能的 code agent：正文规定产品行为，本附录规定本期的做法。两者冲突时以正文为准，并把冲突记录下来。路径相对于 `buckyos/src/`（CLI 源码在同级仓库 `buckyos-websdk/cli/`），代码依据见附录 A。

项目处于开发阶段，按 `AGENTS.md` 不做旧数据兼容：旧的 `agents/<id>/…` 路径和 boot 阶段的 Jarvis 代建直接删除，用全新安装验证。

### B.1 已定决策

| # | 决策 | 依据 |
| --- | --- | --- |
| D1 | Zone Owner、Admin、普通 User 都能为自己创建 Agent，规则与普通用户为自己安装 App 一致：只能为自己创建，不需要管理员，不需要提权确认 | 2026-10-09 用户确认 |
| D2 | 激活向导删除 Jarvis Bot Token；Owner 本人的 Telegram 账号保留为可选项 | 2026-10-09 用户确认 |
| D3 | 共享策略交付前，“允许其他用户使用”开关禁用 | 2026-10-09 用户确认 |
| D4 | 桌面入口打开 Loader 提供的 Agent 主页，Loader 不提供时打开 Users and Agents 中的 Agent 详情页 | 2026-10-09 用户确认 |
| D5 | 创建 Agent = 基于模板为它构造一个独立的 App，作为创建者的 App 安装，再写入绑定到这个 App 实例的 AgentSpec。构造 App 的 AppDID 就是 AgentDID（B.2） | 2026-10-09 用户确认 |
| D6 | 同一用户可以用同一模板创建多个 Agent。每个 Agent 有自己的 App 实例和容器，这是刻意的：不同 Agent 的权限不同，需要隔离运行 | 2026-10-09 用户确认 |
| D7 | Limited 用户本期不能创建 Agent | 2026-10-09 用户确认 |
| D8 | 本期不提供单个 Agent 的权限收窄，只读展示 | 实现决策：RBAC 已同时校验运行时应用与 Owner，上限天然满足 |
| D9 | 模板版本固定在 AgentRoot 层：关闭自动更新后，OpenDAN 不再从包目录同步 AgentRoot，模板更新时也不再为它重建 App | 2026-10-09 用户确认 |
| D10 | 安装 Agent 类型的 PIKG 改为“注册模板”，不再部署运行时实例。本期通过现有 CLI 链路安装模板；App Service 的安装界面待各类 PIKG 安装模式稳定后再做 | 2026-10-09 用户确认 |
| D11 | 不实现存量迁移 | 开发阶段原则 |

### B.2 构造的 App

每个 Agent 对应一个构造出来的 App，由 control_panel 在创建时生成，并以创建者为 owner 安装。

BNS 只支持一级子名（`did:bns:subname.zonename`，见 `buckyos-base/doc/bns.md`），而 AgentDID 本来就是 Zone 的一级子名，所以直接用 AgentDID 作为构造 App 的 AppDID：

| 项 | 规则 |
| --- | --- |
| AppDID | 等于 AgentDID，即 `<zone method>:<agent_name>.<zone id>`。例如 Zone 为 `test.buckyos.io`、Agent 名为 `xiaobai` 时，结果为 `did:web:xiaobai.test.buckyos.io`；BNS Zone 下为 `did:bns:xiaobai.<zonename>` |
| AppId | 等于 AgentId，例如 `xiaobai.test.buckyos.io`；AppInstanceId 为 `<agent_id>@<owner>` |
| AppDoc | 复制模板的 AppDoc，并改写以下字段：<br>• `app_did`：AgentDID<br>• `name`：Agent 名<br>• `show_name`：Agent 的显示名称<br>• `owner`：Zone DID，即 `app_did.upper_did()`，满足现有安装校验<br>• `base_on`：模板 AppDoc 的 ObjectId，用于追溯<br>• `version`：沿用模板版本<br>• `pkg_list.agent`：`pkg_id` 改为 `all.agent.<agent_id>#<version>`，并填入新的 `pkg_objid` |
| 同一 DID 的两份文档 | AgentDocument 与构造的 AppDoc 共用一个 DID，按文档类型区分，与 Zone DID 同时带 zone-doc 和 owner-doc 的模式一致。本地授权覆盖按 `(DID, APP_DID_DOC_TYPE)` 注册，不影响 AgentDocument 的解析 |
| 包 | 包元数据（PACKAGE_META）按新 AppId 重建，名称为 `all.agent.<agent_id>`，版本与 AppDoc 相同；payload（`agent.tar.gz`）原样复用。包命名空间由 AppId 决定，所以模板的包 ObjectId 不能直接引用 |
| PIKG | 用 `frame/control_panel/src/pikg.rs` 的 `PikgReader` 读取模板 PIKG，用 `PikgBuilder` 写出新的 PIKG（写法参照 `app_installer.rs` 的 `build_publish_pikg`），再用 `app_staging.rs` 的 `stage_preinstall_file(..., owner, ...)` 放入该 Owner 的 immutable staging。需要放开 `canonical_preinstall_path` 只允许 `data/cache` 的限制，让模板目录（B.3）也能作为来源 |
| 授权 | 为 AgentDID 注册 `set_local_authority_override`，文档类型为 `APP_DID_DOC_TYPE`。这个覆盖只存在于进程内存中，所以 control_panel 启动时、以及每次对这些 App 运行 Resolve（重试、更新）之前，都要重新注册。注册来源是 AppSpec 中保存的 AppDoc |
| 安装 | 走 `submit_preinstall` 的内部安装路径：SYSTEM_INTERNAL + auto_confirm，owner 为创建者，并显式标明“构造 App”意图，使 B.3 的模板分支不会拦截它。需要泛化两处：一是去掉 `Deleted → user_removed` 的提前返回，否则同名重建会被拦住；二是把预装专用的 intent 和幂等键改为通用写法 |
| 主机名 | app_name 取 AppId 的首个标签，也就是 Agent 名。Zone Owner 的 Agent 主机名为 `xiaobai`，其他用户为 `xiaobai-<owner>`，Agent 的 Web 主页就在这个主机名下。App 的网关条目以单段主机名为键，scheduler 为 Agent 写入的条目以带点的 `agent_id` 为键，两者不冲突 |

### B.3 Agent 模板

**模板来源**

| 来源 | 范围 | 存放 |
| --- | --- | --- |
| 系统内置 | Zone 内所有可创建 Agent 的用户 | rootfs 中的 PIKG（`data/cache/…`），登记在 `system/install_settings.agent_templates`（B.6） |
| 用户通过 CLI 安装 | 安装者本人，与 App 的安装范围一致 | PIKG 副本放在 `data/srv/control-panel/agent_templates/<owner>/<app_id>/<digest>.pikg`；记录写在 `users/<owner>/agent_templates/<app_id>`，内容为 `{app_did, app_doc_object_id, version, digest, pikg_path, installed_at}` |

`data/srv` 不在全新安装的清理范围内；staging 中的文件最多保留 24 小时，所以不能直接拿来当模板存放。

**安装（D10）**
- CLI 现有链路不变：先 `buckyos app fetch <pikg> --plan <f>`，再 `buckyos app install <pikg> --plan <f>`。对应的 kRPC 调用依次是 `apps.staging.finalize`、`apps.plan.recompute`、`apps.details`、`apps.submit`。未发布的模板需要加 `--policy local-developer`。
- 在 control_panel `handle_apps_submit`（`app_installer.rs:3620-3780`）中、完成 inspect 和指纹检查之后、调用 `decide_app_submit_action` 之前加一个分支：如果 AppDoc 的类型是 `AppType::Agent`，并且不是构造 App 的内部安装，就注册模板，不创建安装任务，返回 `{action, task_id: null}`。CLI 对 `task_id` 为空的结果原样输出，无需修改。
- 注册时，把 staging 中已经校验过的 PIKG 复制到模板目录，并写入记录。与已有记录比较：
  - AppDoc ObjectId 相同，返回 `satisfied`。
  - 不存在，返回 `template_registered`。
  - 不同，替换 PIKG 和记录，返回 `template_updated`，并触发 B.5 的模板更新。
- 模板分支不要求目标节点就绪，也不经过卸载保护，因为模板没有 AppSpec。
- 本地授权解析的结果不带 `document_version`，所以没有版本单调检查，以 ObjectId 是否变化为准。
- `submit_preinstall` 目前要求 `apps.submit` 必定生成任务。Agent 类型的 PIKG 已经移出 `pre_install_apps`（B.6），所以不会再走到这里；如果 `pre_install_apps` 中仍出现 Agent 类型，应直接报配置错误。

**删除**
- 本期不提供删除模板的入口。已有 Agent 引用的模板如果被删除，这些 Agent 只是不再收到更新。

### B.4 数据模型与 RBAC

Agent 的记录放在 `users/<owner>/agents/<agent_id>/` 下；构造的 App 按普通 App 存放在 `users/<owner>/apps/<agent_id>/`。

| 键 | 内容 | 写入方 | 读取方 |
| --- | --- | --- | --- |
| `spec` | 现有 `AgentSpec`，类型不变；`binding.target_app_instance_id` 为 `<agent_id>@<owner>` | control_panel，在构造的 App 安装完成后写入 | scheduler（网关、RBAC）、OpenDAN、msg-center、control_panel |
| `key` | Agent 私钥 PEM，与原 bootstrap 写法相同 | control_panel | 运行时不读取 |
| `install_record` | 扩展后的 `AgentInstallRecord`，是创建状态和模板来源的唯一真相（B.5） | control_panel | control_panel、桌面（经 `agent.list`） |
| `settings` | 见下表 | control_panel（创建、`agent.update`） | OpenDAN、msg-center、control_panel |
| `profile` | `{display_name, avatar, bio}`；`avatar` 为不超过 192px 的 data URL | control_panel、OpenDAN `agent.profile_set` | 所有界面；msg-center 用作联系人名称 |
| `info` | `{agent_doc_object_id, generation, loaded_at, template_version}` | OpenDAN 加载成功后写入 | control_panel（就绪判断、展示当前模板版本） |

`settings` 字段（JSON，snake_case）：

| 字段 | 默认 | 说明 |
| --- | --- | --- |
| `enabled`、`auto_start` | `true` | 现有字段 |
| `allow_other_users` | `false` | 本期 control_panel 拒绝设为 `true` |
| `allow_group` | `false` | 群聊开关 |
| `role_supplement` | `""` | 角色设定补充正文 |
| `template_auto_update` | `true` | D9 |
| `desktop_entry` | 无 | 从桌面引导入口创建时为 `"jarvis_guide"` |
| `msg_tunnels` | `[]` | `[{platform: "telegram", bot_token, bot_account_id?}]`，本期最多一项 |

需要删除的旧内容：
- 旧路径 `agents/<id>/{doc,key,settings}`，以及读写它们的代码：control_panel 的 `agent.*`，msg-center `load_zone_agent_documents` 和 `agent_owner` 中的旧分支，`rbac_config.rs` 中 `obj://config/agents/...` 规则。
- OpenDAN 的 `.meta/profile.json`。

**RBAC**

AppId 等于 AgentId 后，现有 `agent` 角色的规则可以直接把权限精确到这一个 Agent：匹配器中的 `r.sub == "app:" + keyGet3(r.obj, p.obj, p.sub)` 要求 `app:<app_id>` 与路径中的 `{agent}` 一致。
- scheduler 生成 RBAC 时（`kernel/scheduler/src/system_config_agent.rs:1525-1544`），对每个 AgentSpec 在现有两行之外增加 `g, app:<binding 的 app_id>, agent`。保留 `agent_runtime`：OpenDAN 靠它列出 `users/<owner>/agents` 并读取 spec。
- `DEFAULT_RBAC_POLICY` 的 `agent` 段已有 settings 和 info 的读写、spec 的读取，只需新增一行：

```text
p, agent, obj://config/users/{user}/agents/{agent}/profile,read|write,allow
```

- 参照现有用例（`rbac_config.rs` 约 `:875-925`）补测试：运行时 App 能读写自己 Agent 的 settings、profile、info，不能读写同一 Owner 名下其他 Agent 的这些键，也不能读 key。
- Agent 模板记录 `users/<owner>/agent_templates/*` 由 control_panel 以服务身份写入；用户自己可读，沿用 `users/{self}/*` 的读权限。

### B.5 创建、删除与模板更新（control_panel）

新增或改写以下 kRPC 方法。建议把 Agent 相关代码从 `frame/control_panel/src/user_mgr.rs` 移到新文件 `agent_mgr.rs`，并在 `main.rs` 注册。

| 方法 | 作用 |
| --- | --- |
| `agent.check_name {name}` | 返回 `{available, reason?, suggestion?, agent_did}`，检查规则见下 |
| `agent.list_templates {}` | 返回系统内置模板和调用者自己安装的模板（B.3），每项包含 `{template_id, source: "bundled" \| "installed", app_did, name, description, version, icon, loader: "opendan"}`，信息读自模板 PIKG 的 AppDoc |
| `agent.create {idempotency_key, name, profile, role_supplement, allow_group, template_id, template_auto_update, desktop_entry?, msg_tunnel?}` | 校验后写入保留记录，启动创建驱动，立即返回 `{agent_id, agent_did, status}` |
| `agent.create.status {agent_id}` | 返回安装记录中的状态、当前步骤、错误，以及构造 App 安装任务的进度 |
| `agent.create.retry {agent_id, skip_tunnel?}` | 从失败步骤继续；`skip_tunnel` 表示用户放弃绑定通道并完成创建 |
| `agent.create.cancel {agent_id}` | 只在写入 `spec` 之前可用；清除保留记录，已装好的构造 App 一并卸载 |
| `agent.list`、`agent.get` | 只读 `users/*/agents/*`。返回 profile、settings（去掉 `bot_token`）、安装状态、binding、构造 App 的实例 id 和 web host，也包括尚未完成的创建 |
| `agent.update` | 修改 settings 中允许修改的字段；本期界面只用 `allow_group` |
| `agent.profile.get`、`agent.profile.set` | 读写 `profile` |
| `agent.delete` | 按下文“删除”执行 |

`agent.set_msg_tunnel` 和 `agent.remove_msg_tunnel` 改为读写 `settings.msg_tunnels`；本期界面不使用。

**权限**
- 调用者只能为自己创建，Owner 即调用者。
- Limited 用户被拒绝。
- 授权只在入口检查一次；之后的驱动步骤用 control_panel 的服务身份执行，因为调用者的 token 可能已经过期。

**名称检查**

`agent.check_name`、`agent.create` 与 `user.create` 共用同一套检查：
- 字符规则：Agent 名是一个 DNS 标签，即 `[a-z0-9-]`、长度 1–63、首尾不能是 `-`。
- 不能是保留名：`root`、`system`、`admin`、`guest`、`_`、`www`、`sys`、`homestation`。
- 不能已经存在 `users/<name>/settings`。
- 任何用户名下都不能已有 AgentId 为 `<name>.<zone>` 的记录，进行中的创建也算。
- 不能等于 `system/app_registry` 中已分配的 App 或实例主机名，也不能等于 shortcut。

建议值：从桌面引导进入时先建议 `jarvis`，被占用时依次建议 `<owner>-jarvis`、`<owner>-jarvis-2`……

**创建驱动**

把 `AgentInstallState` 从 `Bound | Removed` 扩展为 `Provisioning | Bound | Ready | Failed | Removed`。`AgentInstallRecord` 增加以下字段：
- 驱动状态：`step`、`last_error {step, code, message, retryable}`、`idempotency_key`、`request_fingerprint`、`pending_spec`、`runtime_task_id?`、`tunnel_state`、`created_at`。
- 模板来源：`template_id`、`template_source`、`template_app_did`、`template_app_doc_object_id`、`template_version`。

| 步骤 | 动作 | 成功后状态 |
| --- | --- | --- |
| 1. 保留 | 生成密钥与 AgentDocument（`owner` 取 Owner 的真实 DID），计算 AgentSpec（binding 为 `<agent_id>@<owner>` 和 `www`），存入 `pending_spec`。用一个事务以 `Create` 写入 `install_record`、`key`、`settings`、`profile`，同名并发会失败。此时还没有 `spec`，所以 Agent 不会被加载、路由或对话 | `Provisioning / runtime` |
| 2. 构造并安装 App | 按 B.2 生成 PIKG、注册本地授权、提交内部安装，并等待安装任务完成 | `Provisioning / bind` |
| 3. 绑定 | 用一个事务从 `pending_spec` 创建 `spec`，并把记录改为 `Bound`（对记录 revision 做 CAS）。然后调用 `refresh_rbac_by_scheduler` | `Bound / tunnel` 或 `Bound / start` |
| 4. 通道 | 只在提交了 `msg_tunnel` 时执行：调用 msg-center 的 `reload_settings`，让它读到新的 bot 绑定（B.8），并记录 `tunnel_state` | `Bound / start` |
| 5. 就绪 | 等待 `info` 出现，且其中 `agent_doc_object_id`、`generation` 与 `spec` 一致；超时默认 5 分钟（首次拉取 aios 镜像可能较慢） | `Ready` |

- **驱动实现**：control_panel 内的后台任务，形式参照 `app_install_runner.rs` 的 `startup_scan` 和 `sweep_loop`。启动时先为所有构造的 App 重新注册本地授权，再扫描所有处于 `Provisioning` 或 `Bound` 的记录并继续；`agent.create` 和 `agent.create.retry` 时立即调度。每一步都必须幂等，并通过 CAS 更新记录。
- **失败**：写入 `Failed`，并在 `last_error` 中记录失败步骤。第 3 步之前失败，名称仍被保留，直到用户重试或取消。第 4 步失败时 Agent 已可用，状态面板分别显示 Agent 和通道的状态，用户可以重试，或用 `skip_tunnel` 完成。
- **幂等**：相同 `idempotency_key` 和相同请求返回已有记录；记录仍存在时，换一个 key 用同名创建，按名称冲突处理。

**删除**（`agent.delete`，以及第 3 步之前的 `agent.create.cancel`）

按以下顺序执行：
1. 把 `install_record` 改为 `Removed`，删除 `spec`、`key`、`settings`、`profile`、`info`。
2. 刷新 RBAC。
3. 调用 msg-center 的 `reload_settings`。
4. 卸载构造的 App：先删 AgentSpec，`uninstall_app` 的 `ensure_runtime_has_no_agent_bindings` 检查才会通过。
5. 删除 `install_record`。

- 卸载会保留 `Deleted` 状态的 AppSpec、Registry 分配和数据目录。同名重建会复用同一个 AppDID、主机名和数据目录，由 B.7 的 AgentRoot 身份保护保证新 Agent 不继承旧数据（AC-47）。
- 删除与卸载之间，OpenDAN 处于“没有绑定 Agent”的状态，按 B.7 持续等待，不会反复重启。

**模板更新**

- 触发来源有两个：
  - 内置模板：control_panel 的 10 分钟预装扫描中，对每个内置模板计算当前 PIKG 的 AppDoc ObjectId，与上次记录比较。
  - 用户安装的模板：B.3 的注册返回 `template_updated` 时立即触发。
- 对使用该模板、`template_auto_update = true`、且 `install_record` 中 `template_app_doc_object_id` 与当前值不同的 Agent，按 B.2 用新模板重建 PIKG（AppDID 不变），以 `APP_UPDATE` 提交内部更新；成功后更新 `install_record` 中的模板来源字段。内置模板影响所有用户的 Agent，用户安装的模板只影响安装者本人的 Agent。
- 关闭自动更新的 Agent 不重建（D9）。

### B.6 scheduler、内置模板与激活

**scheduler**
- 删除 `add_default_agents`、`DEFAULT_JARVIS_APP_DID`、`BootstrapAgentProvision`、`recover_bootstrap_agent_provisions`、`resolve_jarvis_agent_did`，以及它们在 `kernel/scheduler/src/main.rs` 中的调用（`:147`、`:303`）和相关测试（包括 `system_config_builder.rs` 约 `:1297-1340`，以及 `main.rs` 约 `:485-494` 断言 Jarvis 在 `pre_install_apps` 中的用例）。
- RBAC 生成增加 `g, app:<app_id>, agent`（B.4）。

**内置模板**
- 在 `system/install_settings` 中新增 `agent_templates`（`{template_id: {schema_version, pikg_path}}`），与 `pre_install_apps` 平级；把 Jarvis 从 `rootfs/etc/scheduler/boot.template.toml` 的 `pre_install_apps` 移进去。`bucky_project.yaml` 中的打包配置保留。
- `pre_install_reconciler.rs` 不安装模板本身：模板只是构造 App 的来源，不作为运行时实例运行。

**激活**
- 删除 `jarvis_msg_tunnel_config` 中的 Bot Token。涉及 `kernel/node_active/` 的 `types.ts`、`ActiveWizard.tsx`、`components/steps/JarvisMsgTunnelStep.tsx`、`active_lib.ts`，`kernel/node_daemon/src/active_server.rs`，`make_config.ts`、`active.ts`，以及 scheduler 的 `StartConfigSummary` 和 `build_msg_center_settings`。
- Owner 的 Telegram 账号改为中性的可选字段，例如 `owner_telegram_account_id`，仍写入 `users/<owner>/profile` 的 `private_extra.system_contact.bindings`。写入时用裸账号 id，同时修正 `normalize_telegram_contact_account_id`。在激活界面中放在哪一步由实现决定。
- 激活写入的 msg-center 设置改为 `telegram_tunnel {enabled: true, gateway.mode: "bot_api", bindings: []}`，不再因为没有 token 而使用 `dry_run`。实现时需确认 `bot_api` 模式在没有任何绑定时可以正常运行。

### B.7 OpenDAN、libopendan 与 Jarvis 模板

**Loader（`frame/opendan/`）**
- `main.rs` 的 `wait_for_bound_agent`：没有绑定的 Agent 时持续等待，不退出；超过一个时仍然报错。同时读取该 Agent 的 `settings` 和 `profile`。
- `LoaderEnv` 增加 `owner_did`（取 `spec.agent_doc.owner`）和 settings。
- AgentRoot 身份保护：在 `.meta/identity.json` 中记录 `{agent_did, agent_doc_object_id}`。不一致时，把旧目录移到 `agents/.archived/<agent_id>-<时间戳>`，然后重新初始化（AC-47）。
- 模板固定：`template_auto_update = false` 且 AgentRoot 已初始化时，跳过 `rootfs::sync_from_package`（`loader.rs:63-80`），并记录日志。
- 角色补充：把 `settings.role_supplement` 写入 `<agent_root>/.meta/role_supplement.md`，内容为空时删除该文件。
- 写 `info`：Loader 启动成功后写入 `info`。`template_version` 取本次同步来源的包版本；处于固定版本时保持原值。
- Profile：`home.rs` 的 `agent.profile` 和 `agent.profile_set` 改为读写系统配置中的 `profile`。名称回退为 Agent 用户名，不再用模板的 `display_name`。

**libopendan（`frame/lib_opendan/`）**
- `state/behaviors.rs` 的 `FsBehaviorCatalog::identity`：在 `role` 之后追加 `.meta/role_supplement.md`，并把该文件加入 `revision()`。
- Owner 过滤：在 `bridge/msg.rs` 的 `MsgBridgeCtx` 中增加 `owner: DID`。`route_msg_record` 先确定发送者身份：`record.ingress` 的 `extra.principal_did` 存在时用它，否则用 `record.from`。身份不是 Owner 就返回 `Drop`，这样 `ui.rs` 只做 `mark_read`，不创建 Session，也不调用 LLM。为此补单元测试。

**群聊（`ui.rs` 与 Jarvis 模板）**
- `allow_group = false` 时，群消息只做 `mark_read`。
- `allow_group = true` 时，只有发送者身份是 Owner 且 `msg.mentions.dids` 包含 Agent DID 才触发；其他群消息只做 `mark_read`。
- 触发时，先通过 `MailService` 的新方法（基于 `MsgCenterClient::list_box_by_time`）取同一 mailbox 最近 20 条已读消息，把它们和触发消息依次投入同一队列，再调用 `ensure_task`。重复的对象会按 ObjId 去重。
- Jarvis 模板（`apps/jarvis_runtime/agent/`）：增加 `[[loader.ui]] on = "msg.group"` 规则和 `[session.group]`，并按当前 behavior 结构参照 `chat_route` 重写 `behaviors/groupchat_route.toml`。本期不支持 Telegram 群。

### B.8 msg-center

- **识别 Agent**：`owner_session.rs` 的 `agent_owner` 和 `SessionTokenVerifier` 改为读取 `users/*/agents/*/spec`（Owner 取 `agent_doc.owner`），同时读取 `settings.allow_group`。删除旧分支，`user_may_observe` 随之生效。
- **入群同意**：`group_service.rs` 的 `member_consent` 中，Agent 分支在 `allow_group = false` 时返回明确的错误 `agent_group_disabled`。Owner 代 Agent 调用 `group.accept_invitation` 时，返回同样的错误。`allow_group = true` 时沿用现有规则：Owner 的邀请自动接受，其他人的邀请需要 Owner 确认。同步更新两个测试 mock 和相关用例。
- **好友同步**：删除 `main.rs` 中 `sync_zone_user_contacts` 的 `.chain(collect_sync_owner_dids(...))`，以及 `collect_sync_owner_dids` 本身。
- **联系人名称**：`sync_zone_agent_contacts` 使用 `profile.display_name`，没有时回退为 Agent 用户名。
- **Bot 绑定来源**：在 30 秒一次的同步中，从 `users/*/agents/*/settings.msg_tunnels` 收集 Telegram 绑定（`owner_did` 取 Agent DID），与 `services/msg-center/settings` 中的全局配置合并；只有绑定集合变化时才重建执行器。`reload_settings` 立即触发一次。
- **Owner 识别**：在 `tg_tunnel.rs` 的入站处理（约 `:1284-1301`、`:2481-2506`）中，对私聊用 `telegram:<id>` 查询该 Agent 范围内的联系人绑定。命中的联系人是 Agent 的 Owner 时，写入 `IngressContext.extra.principal_did = <Owner DID>`。`decide_inbox_kind`（`msg_center.rs:991-1011`）在有 principal 时按 principal 判断进入哪个收件箱。消息的 `from` 仍是端点 DID，回复路径和按通道划分的 Session 不变。
- **账号格式**：在 `build_zone_user_seed` 中去掉 Telegram 账号的 `user:` 前缀。

### B.9 桌面

**创建向导**
- 新增内置应用 `agent-setup`：在 `registry.tsx`、`mock/data.ts` 的 catalog、`DESKTOP_BUILTIN_APP_IDS` 中注册，文案放在新的 `i18n/agent-setup.ts`。
- 启动参数为 `{source: "jarvis_guide" | "users_agents", agent_id?}`；带 `agent_id` 时直接显示该次创建的状态面板。
- 向导内部要有自己的 `WindowDialogProvider`（参照 `AppInstaller.tsx:1486-1495`），否则打开其他对话框时向导会被卸载、状态丢失。

**第一步：身份与权限**
- 用户名：输入后防抖调用 `agent.check_name`，并显示 DID 预览。
- 昵称、头像、公开简介：头像选择组件从 OpenDAN WebUI 的 `Home.tsx` 移植。
- 角色设定补充默认折叠；Owner 只读。
- 权限摘要只读（D8）。
- “允许其他用户使用”禁用并显示暂未支持；“允许加入群聊”可选。

**第二步：运行配置**
- Loader 下拉框只有 OpenDAN，旁边有帮助说明。
- 模板下拉框来自 `agent.list_templates`，标出“系统内置”或“我安装的”，下方显示模板信息。
- 自动更新复选框，取消时显示风险提示。
- 配置摘要。
- “安装其他 Agent 模板”：本期展开为 CLI 安装说明（`buckyos app fetch` 和 `buckyos app install` 两条命令），并提供“刷新模板列表”按钮。App Service 的安装界面交付后，再改为打开安装对话框（D10）。

**第三步：外部通道**
- 可以跳过。
- 用 `user.get` 读取本人 Profile：有 Telegram 绑定时 Telegram 才可选；否则说明原因，并提供“前往个人资料”，用启动参数打开 Users and Agents 的 Self 页。
- Lark/飞书 禁用。
- Bot Token 遮蔽显示，并做格式检查。

**确认与状态面板**
- 确认页分组展示摘要，提交 `agent.create`。
- 状态面板轮询 `agent.create.status`，提供重试、跳过通道和取消；成功时显示显示名称与“关闭”。
- 草稿保存在组件状态中，并按用户 id 写入 localStorage，读写都要 try/catch。

**桌面引导入口**
- 新增内置项 `agent-guide`，使用 Jarvis 图标和名称，加入桌面与移动端的默认布局。
- 状态从 `agent.list` 计算：取本人名下 `settings.desktop_entry == "jarvis_guide"` 的 Agent，按 §9.4 决定显示和点击行为。
- 已就绪时，点击用 `openAppWindow("<agent_id>@<owner>")` 打开构造 App 的 Web 入口；没有 `web_hosts` 时，打开 Users and Agents 的 Agent 详情页。
- 状态在窗口获得焦点、向导关闭后，以及每 30 秒刷新一次。
- `runtime_type == "agent"` 的 App 实例（即各 Agent 的构造 App）不放进桌面布局，但保留其 App 定义，供 `openAppWindow` 使用。
- `AppIcon` 支持图片：在 `AppDefinition` 中增加 `iconUrl`，并更新 `DesktopRoute.tsx`、`DesktopWindowContainer.tsx`、`SystemSidebar.tsx`、`StandaloneAppTitleBar.tsx` 中的调用。
- Limited 用户不显示该入口。

**Users and Agents**
- Add Agent 打开 `agent-setup`（`source: "users_agents"`）；Limited 用户显示禁用原因。
- 面板支持启动参数 `{entityId}` 和 `{view: "self"}`，写法参照 `messagehub/launch.ts`。
- Self 页 Social Accounts 的 Telegram 添加和删除改为真实调用 `user.set_msg_tunnel` 和 `user.remove_msg_tunnel`。
- Agent 详情页：显示 profile 和安装状态，增加“允许加入群聊”开关（调用 `agent.update`），提供删除操作（调用 `agent.delete`）。
- 删除把 `agents[0]` 当作“那个 Agent”的用法，修正 `agentStateToStatus` 对未知状态的映射，并为文案接入 i18n。

**其他**
- `api/task_mgr.ts` 中按 `appId.includes('jarvis')` 的判断，改为按 Agent 运行时类型或 binding 判断。

### B.10 实施顺序与验证

| 里程碑 | 内容 | 验证（Rust 命令在 `src/` 下运行） |
| --- | --- | --- |
| M1 身份、模板与创建 | B.2–B.6 | `cargo test -p buckyos-api -- --test-threads=1`、`cargo test -p control_panel -- --test-threads=1`、`cargo test -p scheduler -- --test-threads=1`。<br>新增用例覆盖：构造 PIKG（AppDoc 与包元数据校验通过）；Agent 类型 PIKG 的 submit 走模板分支（注册、satisfied、更新），而构造 App 的内部安装不被拦截；名称检查；驱动各步骤的恢复与幂等；删除顺序；RBAC 只允许访问自己的 Agent。<br>全新安装后：用 CLI 安装一个模板；用脚本调用 `agent.create`，确认构造 App 被安装、`spec` 被写入、`info` 出现；再用同一模板创建第二个 Agent |
| M2 运行时与消息 | B.7、B.8 | `cargo test -p libopendan -- --test-threads=1`、`cargo test -p opendan -- --test-threads=1`、`cargo test -p msg_center -- --test-threads=1`。新增用例覆盖 Owner 过滤、群 @ 触发与上下文、`agent_group_disabled`、Telegram principal 映射、模板固定、AgentRoot 归档 |
| M3 桌面 | B.9 | 在 `frame/desktop` 下运行 `pnpm check`、`pnpm lint`，datamodel Deno 测试，`pnpm test:e2e`（mock）。`tests/e2e/pages/users-agents.spec.ts` 中断言 “coming soon” 的用例要改写为向导流程 |
| M4 DV | 按 §12 验收 | `uv run start.py --all` 全新安装，`uv run src/check.py` 确认激活。用 devtest 和一个普通用户各走一遍桌面引导创建，再从 Users and Agents 用同一模板各建第二个 Agent；用 CLI 安装第二个模板后，在向导中刷新并选用它。Telegram 相关用例需要真实 Bot Token |

需要同步更新的已有测试和脚本：
- `test/test_control_panel/test_user_mgr.ts`：用的是旧 `agent.*`。
- `test/test_opendan/*`、`src/debug_jarvis.sh`、`rootfs/bin/service_debug.tsx`：它们假设存在 AppId 为 `jarvis.buckyos.bns.did` 的 Jarvis 实例；改为先创建 Agent，再使用它的 AppId（即 AgentId）。
- `kernel/scheduler/` 中与 Jarvis bootstrap 有关的测试。
- msg-center 中 `member_consent` 相关测试。

### B.11 已知风险

| 风险 | 后果 | 后续可选措施 |
| --- | --- | --- |
| 构造的 App 依赖进程内存中的本地授权覆盖 | 漏掉重新注册时，重试和更新会在 Resolve 阶段失败；catalog 方式的 `apps.upgrade` 对这些 App 无效 | 把本地授权持久化，或由 Zone 解析器提供构造 App 的 AppDoc |
| 模板与构造 App 的更新没有版本单调检查 | 用 CLI 装回旧版本模板时，自动更新的 Agent 会被“更新”到旧版本 | 比较 `AppDoc.version` 的语义版本 |
| 删除 Agent 不清理数据目录和 Registry 分配 | 磁盘占用残留；同名重建复用旧目录，由 AgentRoot 身份保护兜底 | 卸载时按需删除数据 |
| Bot Token 明文存放在 Agent Settings | Owner、管理员和该运行时都能读到 | 移入专门的密钥存储 |
| `reload_settings` 重建全部 Telegram 执行器 | 新建一个 Agent 时，其他 Agent 的 Telegram 短暂中断 | 按绑定增量更新 |
| 固定版本的 AgentRoot 与新版 OpenDAN 不兼容 | 模板配置结构变化后可能加载失败 | 用户重新打开自动更新；Loader 给出明确错误 |
| 身份文本在 Session 创建时冻结 | 修改角色补充后，只有新 Session 生效 | 提供刷新 Session 身份的操作 |
| 之后分配 App 主机名时不检查 Agent 名 | 可能与已有 Agent 的主机名冲突 | Registry 分配时纳入 Agent 名 |
| 构造 App 首次安装可能较慢 | 状态面板长时间停在“创建中” | 显示安装任务的进度；超时后可以重试 |
| 旧 Zone 中 boot 代建的 Jarvis 不迁移 | 这类 Jarvis 不会关联到桌面入口 | 产品要求迁移时再做 |

### B.12 本期不做

- App Service 中的 Agent 模板安装界面，以及模板的删除入口。
- 单个 Agent 的权限收窄。
- 共享访问策略与 Router。
- Lark/飞书 通道、Telegram 群、一个 Agent 绑定多个通道。
- 创建后修改角色补充和模板更新设置（`allow_group` 除外）。
- 管理员为其他用户创建 Agent。
- 存量迁移与额外的桌面快捷方式。

## 附录 C：实施记录（2026-10-09）

附录 B 已全部实施，未提交 git。本附录记录落点、与附录 B 的偏离和尚未完成的验证。路径相对于 `buckyos/src/`。

### C.1 落点

| 范围 | 主要文件 |
| --- | --- |
| 共享类型与 RBAC | `kernel/buckyos-api/src/app_schema.rs`（扩展 `AgentInstallRecord`；新增 `AgentSettings`、`AgentProfile`、`AgentRuntimeInfo`、`AgentTemplateRecord` 与键函数）、`app_install.rs`（`SystemInstallSettings.agent_templates`）、`rbac_config.rs` |
| control_panel | 新增 `frame/control_panel/src/agent_mgr.rs`（名称检查、模板、构造 PIKG、创建驱动、删除、模板更新、`agent.*` 方法）；`app_installer.rs`（`submit_internal_install`、模板分支）、`pre_install_reconciler.rs`、`pikg.rs`、`user_mgr.rs`、`main.rs` |
| scheduler 与激活 | `kernel/scheduler/src/{system_config_builder,install_plan_executor,system_config_agent,main}.rs`、`rootfs/etc/scheduler/boot.template.toml`；`kernel/node_active/`（`OwnerTelegramStep.tsx` 取代 `JarvisMsgTunnelStep.tsx`）、`kernel/node_daemon/src/active_server.rs`、`make_config.ts`、`active.ts` |
| OpenDAN | `frame/opendan/src/{main,loader,records,rootfs,home,ui}.rs`；`frame/lib_opendan/src/{bridge/msg.rs,protocol/input.rs,runner/inputs.rs,runner/drive.rs,runner/input_view.rs,state/behaviors.rs}`；`apps/jarvis_runtime/agent/{agent.toml,role.md,behaviors/groupchat_route.toml}` |
| msg-center | 新增 `frame/msg_center/src/zone_agent.rs`；`owner_session.rs`、`group_service.rs`、`main.rs`、`msg_tunnel.rs`、`msg_center.rs`、`contact_mgr.rs`、`tg_tunnel.rs` |
| 桌面 | 新增 `frame/desktop/src/app/agent-setup/`（向导、`guide.ts` 入口状态）、`api/account.ts`、`api/control_panel_mock.ts`、`i18n/agent-setup.ts`；改写 `api/user_mgr.ts`、`api/task_mgr.ts`、`app/users-agents/`、MessageHub 的 `agent_group_disabled` 提示 |
| 脚本与测试 | `debug_jarvis.sh`（`--agent`）、`rootfs/bin/service_debug.tsx`（`agents <owner>`）、`test/test_opendan/agent_target.ts`、`test/test_control_panel/test_user_mgr.ts` |

### C.2 与附录 B 的偏离

| 项 | 实际做法 | 原因 |
| --- | --- | --- |
| 群上下文投递（B.7） | libopendan 协议新增 `MsgDelivery.context`：上下文消息不单独组成批次，跟随其后的第一条输入进入同一批次 | 20 条上下文加触发消息超过默认批次上限，逐条投递时可能出现只含他人消息的批次单独触发 Turn，违反 §5.5、§10.3 |
| Agent 名字（§5.1） | Loader 在 `.meta/role_supplement.md` 开头写入“Your name is <显示名称> (account `<用户名>`)”，再接角色设定补充；Jarvis 模板的 `role.md` 不再写死 “You are Jarvis” | 模板不决定 Agent 的名字，否则改名后的 Agent 仍自称 Jarvis |
| 名称保留（B.5） | 新增 `services/control_panel/agent_names/<agent_id>` = `{owner_user_id}`，与 `install_record` 同事务创建和删除 | 不同用户并发创建同名 Agent 时只允许一个成功，并能按 AgentId 找到 Owner |
| 错误返回（B.5） | `ReasonError("<code>: …")`，界面按包含 `<code>:` 识别；错误码包括 `limited_user`、`name_conflict`、`sharing_unsupported`、`owner_identity_missing`、`invalid_bot_token`、`telegram_bot_invalid`、`telegram_unreachable` 等 | 与现有 kRPC 错误形式一致 |
| 删除（B.5） | `agent.delete` 同步完成第 1 步后返回，其余步骤由后台驱动执行；删除中的 Agent 不再出现在 `agent.list` | 卸载构造 App 可能较慢 |
| 通道校验（B.5、B.8） | 第 4 步由 control_panel 调 Telegram `getMe` 校验 Token，并把 bot id 写入 `bot_account_id`；msg-center 对单个坏 Bot 只跳过，不再整体回滚 | `reload_settings` 不再能反映单个 Token 的错误 |
| Agent 邮箱观察（B.8） | 只有该 Agent 的 Owner 能观察其邮箱，Admin 也不能 | 多用户 Agent 下原规则会泄露他人与其 Agent 的私聊 |
| 入群检查（B.8） | `agent_group_disabled` 覆盖全部加入类方法：`accept_invitation`、`request_join`、`accept_session_invitation`、`submit_guest_request` | 只拦 `accept_invitation` 可被其他加入方式绕过 |
| 用户名规则（B.5） | `user.create` 改用共享名称检查后，含 `_` 或 `.` 的用户名被拒绝 | 用户与 Agent 共用名字空间且必须是 DNS 标签（§5.1） |
| AgentRoot 认领（B.7） | 没有 `.meta/identity.json` 的已有目录直接认领，不归档；记录不一致或文件损坏时才归档 | 手工准备的 AgentRoot 与测试夹具需要直接使用 |
| 模板分支（B.3） | 分支位于 inspect 之后，inspect 取不到默认节点时会先失败 | 复用 inspect 的信任与包完整性检查 |
| 桌面图标（B.9） | 仓库没有 Jarvis 图标资源，引导入口使用通用图标；Bot Token 不写入本地草稿 | 资源缺失；秘密信息不落浏览器存储 |

### C.3 验证与未完成项

- 已通过：`cargo test -p buckyos-api`、`control_panel`、`scheduler`、`msg_center`、`libopendan`、`opendan`（均 `--test-threads=1`），`node_daemon` 的 `active_server` 用例；`kernel/node_active` 的 `tsc`、`pnpm build`、`pnpm test`；`frame/opendan/web` 与 `frame/desktop` 的 `pnpm check`、`pnpm lint`、mock e2e；desktop datamodel Deno 测试；`uv run buckyos-build.py` 全量构建；`./build_aios --local-test`。
- M4 DV（2026-10-09，全新安装的 devtest / test.buckyos.io，Agent 容器使用 `./build_aios --local-test` 构建的本地镜像）已通过：

| 验收项 | 结果 |
| --- | --- |
| AC-01 | 全新 Zone 没有任何 Agent，`agent.list_templates` 只有内置 Jarvis |
| AC-02、AC-34、AC-35、AC-38、AC-48 | 普通用户 dave 在真实桌面点击 Jarvis 引导入口，`jarvis` 已被占用时向导自动换成 `dave-jarvis`，填昵称“小戴”、跳过通道后创建成功，关闭后入口显示“小戴”；全程约 25 秒（`frame/desktop/tests/e2e/real/agent-setup.real.spec.ts`） |
| AC-04、AC-05 | 用户名与任何用户名下的 Agent 冲突时报 `user_exists` / `agent_exists` 并给出建议；大写、保留名分别报 `invalid` / `reserved` |
| AC-06、AC-07、AC-08、AC-22 | Owner 经 Message Center 发消息约 5 秒收到回复；Agent 自称“小白”，按角色补充用中文回答 |
| AC-12 | 非 Owner 直接发消息（即使自称主人）不回复、不建 Session |
| AC-13、AC-14、AC-15、AC-41 | 群聊开关关闭的 Agent 被邀请时返回 `agent_group_disabled`；开启后，非 Owner 的 @ 与 Owner 未 @ 都不触发；Owner @ 时 Agent 总结了群内他人消息，并说明他人的指令只是背景信息、没有执行 |
| AC-18 | `buckyos app fetch/install --policy local-developer` 安装第二个模板返回 `template_registered`、不建任务；只有安装者在模板列表中看到它 |
| AC-20 | 模板发布 0.1.1 后，开启自动更新的 Agent 约 10 秒内重建并同步 AgentRoot，固定版本的 Agent 保持 0.1.0；更新不改变 AgentRoot 身份 |
| AC-23、AC-25、AC-29、AC-32 | Owner 没有 Telegram 身份时拒绝提交通道（`owner_identity_missing`）；Token 格式错误被拒；无效 Token 在通道步骤失败（`telegram_bot_invalid`），`agent.get` 不含 Token；选择跳过后完成创建并移除通道 |
| AC-39 | 同一 Owner 用同一模板创建第二个 Agent，各自容器、名字与角色补充互不影响 |
| AC-44 | 在 OpenDAN 主页或控制面板修改资料，对方立即读到同一份 |
| AC-47 | 删除后约 35 秒释放名字；同名重建时旧 AgentRoot 归档到 `agents/.archived/`，新 Agent 没有旧 Session |

- 未在 DV 中验证：真实 Telegram Bot 的消息收发与 Owner 映射（只有单测）；登录过期后回到向导；手机端；App Service 中的模板安装界面（本期不做）。
- 已知风险见 B.11，另有：AgentDocument 的 ObjectId 一变就归档 AgentRoot（重签文档或轮换密钥时会被当作另一个 Agent）；角色补充、名字和 `template_auto_update` 只在 Loader 启动时读取；Owner 删除 Telegram 身份后，msg-center 联系人中的旧绑定不会被移除。

### C.4 DV 中发现并修复的问题

| 问题 | 原因 | 修复 |
| --- | --- | --- |
| 构造 App 的安装一直停在“waiting for trust resolution” | name-client 先查 Zone Resolver，它对本 Zone 子名（构造 App 的 AppDID）直接回答 Missing，本地授权覆盖没有机会生效 | control_panel 解析本 Zone 子名形式的 AppDID 时关闭 Zone Resolver（`app_install_resolver.rs`） |
| 安装任务等待信任解析时，状态面板一直显示创建中 | 驱动把 Waiting 阶段当作仍在运行 | 暂停或带错误等待的任务记为 `Stalled`：创建步骤报失败，用户重试时恢复原任务而不是重新提交（`agent_mgr.rs`） |
| 恢复暂停的安装任务时报 `encode transaction progress` | `completed_stages` 为空时被序列化省略，全量 patch 补成 `null`，回读时类型错误；所有从 Resolve 重试的暂停任务都会触发 | 全量 patch 中 `completed_stages` 写空数组（buckyos-api `app_install.rs`） |
| 失败后重新提交构造 App 安装报 `idempotency_conflict` | 内部安装的任务 id 由意图和 PIKG 摘要决定，同一 PIKG 重提会撞上失败的旧任务 | 构造 App 的安装意图带上被取代的失败任务（`InternalInstall::AgentRuntime.supersedes`） |
| Agent 在群里回复报 `sender-mismatch` | msg-center 以 App token 的用户身份作为群消息发送者，Agent 的运行时 App 无法以 Agent 身份发群消息 | App token 的 AppId 等于发送方 AgentId、且 Agent 的 Owner 就是该 token 的用户时，按 Agent 身份发送（`group_service.rs`） |
| `agent.get` 的 `template.loaded_version` 为空 | 部署的执行规格中包 id 固定为 `#pkg:<objid>`，OpenDAN 取不到版本 | OpenDAN 改从自己的 AppSpec 读取 AppDoc 版本 |

