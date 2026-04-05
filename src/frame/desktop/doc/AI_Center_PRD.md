# AI Center PRD

- 产品名称：AI Center
- 文档类型：PRD（产品需求文档）
- 输出格式：Markdown
- 适用范围：桌面版 + 移动版
- 关联系统：BuckyOS / OpenDAN / Agent Runtime / AI Completion Service

---

## 0. 文档目的

本文档用于将当前关于 **AI Center** 的产品想法，整理成一份可供以下角色直接使用的统一文档：

- 产品经理：确认范围、信息架构、交互主线
- 设计师：据此建立 Figma 信息层和关键页面原型
- 前端工程师：按页面结构与组件层次实现
- 后端工程师：按数据模型、状态模型、事件模型落库与提供接口
- Agent / Runtime 工程师：明确 AI 能力如何接入、路由、统计与展示

---

## 1. 产品概述

## 1.1 产品定位

AI Center 是 BuckyOS 中的 **系统级 AI 能力控制中心**，用于统一配置、管理、观察、路由系统中的全部 AI 能力。

一句话定义：

> AI Center = 系统级 AI Control Plane + Usage / Balance Console + AI Routing 配置入口

它在系统中的角色，类似于：

- NAS 中的 Storage 管理中心
- 系统设置里被单独提升出来的一类基础能力面板
- 模型聚合器 + Provider 管理器 + AI 用量控制台

---

## 1.2 为什么需要 AI Center

当前 AI 能力不是某一个 App 的功能，而是整个系统的基础设施能力。  
系统中的 Agent、App、Session 都可能调用 AI，因此必须有一个统一入口，负责：

1. 启用 AI 能力
2. 接入外部 Provider
3. 管理本地模型
4. 定义逻辑模型与路由
5. 统计用量、估算成本、展示余额
6. 让用户知道「谁在用、用了多少、花在哪」

---

## 1.3 核心目标

### 面向用户
- 用尽可能低的门槛启用 AI 能力
- 让用户清楚看到余额、Credit、Token、模型消耗
- 让用户知道哪个 App / Agent / Session 用掉了资源
- 降低 AI 配置复杂度，优先支持向导式接入

### 面向系统
- 把 Provider / Model / Routing 收敛成统一抽象
- 让 Agent 调用逻辑模型，而不是硬编码具体模型名
- 为用量统计、成本估算、未来的配额控制打基础

---

## 1.4 非目标

当前版本中，AI Center **不负责** 以下内容：

- 不作为聊天产品主界面
- 不作为模型商店本体（本地模型安装入口跳转至 Store）
- 不完整复制各家 Provider 的账单后台
- 不在当前阶段直接承担复杂权限管控或计费结算系统
- 不在当前阶段实现完整本地模型下载流程

---

## 2. 目标用户与使用模式

## 2.1 普通用户

特点：

- 首次启用 AI
- 不理解 Provider、Protocol、Routing 等概念
- 更关心：
  - 能不能用
  - 还剩多少额度
  - 花了多少钱
  - 需要去哪里充值

需求：

- 首启向导简单
- 默认 Provider 优先推荐
- 不暴露复杂路由逻辑
- 首页直接看到可用状态、余额、用量

---

## 2.2 高级用户 / 开发者 / Power User

特点：

- 能理解 Provider、API Key、Endpoint、Logical Model
- 可能接入多个 Provider、多个 Key、多个模型
- 需要自定义逻辑名、优先级、Fallback、模型策略

需求：

- Provider 配置可精细化
- 支持自定义 Provider
- 支持逻辑模型配置
- 支持按模型 / App / Session 深钻使用情况

---

## 2.3 两种模式设计原则

| 模式 | 默认 | 适用对象 | 设计原则 |
|---|---|---|---|
| 普通模式 | 是 | 普通用户 | 向导式、少概念、低心智负担 |
| 高级模式 | 否 | 高级用户/开发者 | 完整控制、完整可观测、完整配置 |

---

## 3. 核心概念定义

## 3.1 Provider

AI 算力来源。既可以是云端 API Provider，也可以是系统默认 Router。

示例：

- SN Router（系统默认）
- OpenAI
- Anthropic
- Google
- OpenRouter
- Custom Provider

---

## 3.2 Model

实际可调用的模型实例。

分为：

- 本地模型（Local Model）
- 云端模型（Cloud Model）

---

## 3.3 Logical Model（逻辑模型名）

系统内部使用的逻辑别名，不直接绑定某一个具体模型，而是绑定一组候选模型与优先级策略。

示例：

- `llm.chat`
- `llm.plan`
- `llm.code`
- `txt2img`

Agent 与 App 调用时，优先调用 Logical Model，而不是写死 `gpt-4.x / opus / gemini` 等具体名字。

系统里一个逻辑模型名有几个配置:
1）默认设置：指向另一个模型名，这个模型名也可以是逻辑名字
2）fallback：当默认设置不可用的时候，使用哪个模型

根据当前配置，所有的逻辑模型名都可以解析出最终的实际模型名

---

## 3.4 Usage

系统视角的 AI 使用量 (基本单位是token,如果provider提供支持，可以转换为金额）。  
不是照搬 Provider 后台，而是基于系统运行时的调用记录，对 AI 消耗做统一聚合。

维度包括：

- 总量
- 分类
- Provider
- Model
- App
- Agent
- Session
- 时间

---

## 3.5 Balance / Credit

用户的余额信息，分为两类：

1. Provider 可直接提供的Token余额 （无法查询则在是Unknown)
2. 系统默认 Router 使用的 Credit 余额

其中 SN Router 采用 Credit 体系，避免多币种和多充值渠道复杂度直接暴露给用户。

---

## 4. 产品边界与整体结构

## 4.1 总体信息架构（IA）

```text
AI Center
├── Home / Usage（默认首页）
│   ├── AI Status
│   ├── Balance / Credit
│   ├── Usage Summary
│   ├── Time Series
│   ├── By Provider
│   ├── By Model
│   ├── By App / Agent / Session
│   └── Drill-down Table
│
├── Providers
│   ├── Provider List
│   ├── Add Provider Wizard
│   └── Provider Detail
│
├── Models
│   ├── Local Models List
│   ├── Empty State
│   └── Jump to Store
│
└── Routing（高级）
    ├── Simple Overview
    ├── Logical Models
    ├── Priority / Fallback
    └── Advanced Mapping
```

---

## 4.2 默认首页规则

### 未启用 AI 时
进入 AI Center，默认显示 **Enable / Add Provider 向导入口页**。

### 已启用 AI 时
进入 AI Center，默认显示 **Home / Usage** 页。

---

## 4.3 系统状态

### 状态 A：未启用
判定条件：

- 无 Provider
- 且无可用 Model

表现：

- AI 功能未开启
- 首页显示启用说明与引导
- 强制用户至少添加一个 Provider，再进入后续能力

### 状态 B：已启用（仅一个 Provider）
表现：

- 首页可查看用量与余额
- Providers 中可继续新增 Provider
- Routing 用默认逻辑模型配置

### 状态 C：已启用（多 Provider / 多模型）
表现：

- Usage 中出现多维度统计
- Routing 高级能力开放
- 支持 Provider、Model、App、Session 的深度观察

---

## 5. 关键体验主线

## 5.1 首次进入 AI Center

### 触发条件
系统检测到 AI 尚未启用：

- 没有 Provider
- 没有可用模型

### 目标
让用户完成 AI 能力的第一次接入。

### 流程
1. 用户进入 AI Center
2. 系统展示启用说明页
3. 用户点击“开始启用”
4. 进入 Add Provider Wizard
5. 用户至少完成一个 Provider 的创建
6. 系统拉取模型列表并成功启用
7. 跳转到 Home / Usage

### 产品要求
- 若未完成至少一个 Provider，用户无法真正使用 AI 能力
- 支持优先推荐 SN Router 或推荐 Provider
- 错误需可解释，不允许模糊失败

---

## 5.2 新增 Provider

### 目标
让外部 AI 能力被系统纳入统一资源池。

### 本质
新增 Provider 不是单纯“保存一份配置”，而是一个完整的接入流程：

- 配置
- 校验
- 模型发现
- 同步策略确认
- 加入系统资源池

---

## 5.3 查看用量与成本

### 目标
用户不仅看见 Token，还能看见钱和余额。

### 首页优先级
对已启用用户，首页优先展示：

1. AI 状态
2. Balance / Credit
3. Usage Summary
4. Time Series
5. 深钻维度

---

## 5.4 管理(本地)模型

### 当前策略
本地模型能力尚未正式开启，但 UI 必须预留。

### 设计要求
- 仅展示本地已安装模型
- 提供“安装模型”入口
- 点击后跳转至 Store / Model Store
- 不在 Models 页做两套安装流程

---

## 5.5 管理逻辑模型与路由

### 普通模式
- 默认隐藏复杂路由逻辑
- 系统自动生成默认逻辑模型映射
- 用户只感知“默认可用模型策略”

### 高级模式
- 开放逻辑模型列表
- 可管理优先级与 Fallback
- 可把逻辑名映射到多个具体模型
- 支持未来的多 Key / 多账号 / 多路由策略

---

## 6. 功能需求：Home / Usage

## 6.1 模块定位

Home / Usage 是 AI Center 的默认首页，也是用户感知价值最高的一页。

它回答四个核心问题：

1. AI 是否已启用
2. 我还剩多少可用额度 / Credit
3. 我已经用了多少
4. 是谁用掉了这些资源

---

## 6.2 首页模块结构

### 顶部摘要区
- AI Status
- SN Router Credit
- Estimated Cost
- Provider Balance Overview

### 中部可视化区
- Usage Trend（按时间）
- Category Breakdown
- Provider Breakdown
- Model Breakdown

### 下部分析区
- By App / Agent / Session
- 明细表格
- 过滤器与时间范围切换

---

## 6.3 AI Status 卡片

显示内容：

- AI 是否已启用
- 当前可用 Provider 数
- 当前可用 Model 数
- 默认路由状态（可选）
- 最近一次模型同步是否正常（可选）

---

## 6.4 Balance / Credit

### 目标
把“Token 用量”翻译成用户真正关心的信息。

### 必须显示
- SN Router 剩余 Credit
- SN Router 本期已用 Credit
- SN Router 充值入口

### 尽量显示
- Provider 可查询余额
- Provider 估算费用

### 展示策略
- 能精确显示就精确显示
- 不能精确显示就明确标注“Estimated”
- 无法获取时明确展示“Unavailable / Usage only”

---

## 6.5 Usage Summary

### 总览维度
- 今日用量
- 本月用量
- 累计用量
- 总 Token 数
- 总请求数（可选）
- 总估算成本

### 分类维度
- AI Text
- Image Generation
- Audio / Speech
- Video（预留）

说明：

尽管底层计费口径不同，系统层统一抽象为 Usage 统计；成本可为 Token 或 Token Equivalent 上的估算值。

---

## 6.6 By Provider

支持查看：

- 各 Provider 的用量占比
- 各 Provider 的估算费用
- 各 Provider 当前状态

---

## 6.7 By Model

支持查看：

- 各模型使用量
- 各模型估算费用
- 各模型占比

示例：

- GPT-4.x / GPT-5.x 系列
- Claude Opus / Sonnet
- Gemini 系列
- 本地模型（未来）

---

## 6.8 By App / Agent / Session

这一层是 AI Center 的关键差异化能力。

支持从调用者视角观察 AI 消耗：

- 哪个 App 用得最多
- 哪个 Agent 用得最多
- 哪个 Session 用掉最多

层级建议：

```text
App
└── Agent
    └── Session
```

用途：

- 追踪资源去向
- Debug 高消耗行为
- 为未来配额控制与审计打基础

---

## 6.9 Time Series

支持时间粒度：

- 按小时
- 按日
- 按周
- 按月

支持切换指标：

- 总 Usage
- 按 Provider
- 按 Model
- 按 App / Agent

---

## 6.10 明细表与深钻

用户可从首页进入深钻：

- Provider -> Model -> App -> Session
- App -> Agent -> Session -> 调用记录

表格字段建议：

- 时间
- Provider
- Model
- Category
- App
- Agent
- Session
- Tokens In
- Tokens Out
- Estimated Cost
- Status

---

## 6.11 首页空状态与异常状态

### 未启用
- 说明为什么现在不可用
- 主 CTA：添加 Provider / 开始启用

### 无数据
- 显示“尚无调用记录”
- 引导用户进入 App / Agent 使用 AI

### Provider 失效
- 卡片显示授权失效
- 提供“重新授权 / 更新 Key”

### 成本不可用
- 允许显示 Token 统计
- 成本区域显示“Estimated unavailable”

---

## 7. 功能需求：Providers

## 7.1 模块定位

Providers 模块用于接入和管理所有外部 AI 能力来源。  
这是当前阶段 AI Center 的核心入口。

---

## 7.2 Provider 列表页

列表项需显示：

- Provider 名称
- Provider 类型
- 连接状态
- 授权状态
- 模型数量
- 最近同步时间
- 余额 / Credit 摘要（如可用）
- 操作入口

操作包括：

- 查看详情
- 刷新模型列表
- 更新认证信息
- 重新授权
- 删除 Provider

---

## 7.3 Add Provider Wizard

## 7.3.1 设计原则

- 必须是向导，不是一次性大表单
- 先选类型，再展开字段
- 创建前必须做连接验证和模型发现
- 创建时默认开启模型列表自动同步

---

## 7.3.2 向导步骤

### Step 1：选择 Provider 类型

推荐顺序：

1. SN Router（优先推荐）
2. OpenAI
3. Anthropic
4. Google
5. OpenRouter
6. Custom Provider

展示方式：

- 卡片式
- 每张卡片含：名称、说明、适用对象、推荐标签（若有）

---

### Step 2：填写接入信息

#### A. SN Router
偏账号接入：

- 账号状态 / 激活状态
- Credit 状态
- 登录 / 绑定动作（如需要）

特点：
- 字段最少
- 更适合普通用户

#### B. OpenAI
支持两种接入方式：

1. API Token
2. OAuth / 联合登录（需技术验证其持久性与续期策略）

字段示例：

- Provider Name（可默认）
- API Token（或登录授权）
- 可选 Endpoint
- 可选组织信息

#### C. 其他标准 Provider
字段一般包括：

- Provider Name
- API Token / Key
- 可选 Endpoint
- 可选组织信息（视 Provider 而定）

#### D. Custom Provider
最复杂，需完整展示：

- Provider Name
- Endpoint URL
- Protocol Type
- API Key / Token

### Protocol Type
当前文档按抽象方案记录为：

- OpenAI-compatible
- Anthropic-compatible
- Google-compatible

> 注：如工程层协议命名与上述不同，以工程枚举为准，但产品层需保留“协议类型选择”的交互。

---

### Step 3：连接验证与模型发现

创建 Provider 之前必须执行：

1. 基础连通性检查
2. 认证有效性检查
3. 模型列表拉取
4. 能力识别（是否支持余额、是否支持用量）

必须让用户看到：

- 验证结果
- 已发现模型列表
- 当前配置是否可用

---

### Step 4：创建确认

确认页显示：

- Provider 类型
- 认证方式
- 连接状态
- 已发现模型数量
- 自动同步模型列表：开 / 关（默认开）

完成后：

- Provider 进入系统资源池
- 模型进入可用池
- 首页状态切换为 AI 已启用
- Usage / Balance 开始按该 Provider 汇总

---

## 7.3.3 自动同步模型列表

默认策略：

- 新增 Provider 时默认勾选“自动同步模型列表”

原因：

- Provider 模型会新增、下线、改名、权限变化
- 系统应尽量保持“最新可用模型视图”

Provider 详情页中需显示：

- 上次同步时间
- 当前同步状态
- 手动刷新按钮
- 自动同步开关

---

## 7.3.4 错误提示要求

错误必须可解释，不能只有“添加失败”。

错误类型示例：

- Token 无效
- Endpoint 不可达
- 协议不兼容
- 模型列表获取失败
- OAuth 已过期
- 余额 / 用量能力不可用（允许继续）

---

## 7.4 Provider 详情页

需展示以下内容：

### 基础信息
- Provider 名称
- Provider 类型
- 接入方式
- Endpoint（如适用）
- 创建时间

### 运行状态
- 连接状态
- 授权状态
- 上次验证时间
- 上次模型同步时间
- 模型同步状态

### 模型信息
- 已发现模型列表
- 模型数量
- 支持刷新
- 自动同步开关

### 余额与用量
- 当前余额 / Credit（若支持）
- 估算费用
- 最近使用情况

### 操作
- 更新 API Key
- 重新授权
- 刷新模型
- 进入充值页（如适用）
- 删除 Provider

---

## 8. 功能需求：Models

## 8.1 模块定位

Models 模块当前仅管理 **本地已安装模型**，不负责本地模型下载实现本身。

---

## 8.2 当前版本需求

展示：

- 本地已安装模型列表
- 模型名称
- 模型大小（如可用）
- 状态
- 最近使用时间（可选）

操作：

- 启用 / 停用
- 删除
- 查看详情（可选）
- 安装模型（跳转至 Store）

---

## 8.3 关键约束

- 不做第二套安装流程
- 点击“安装模型”统一跳转 Store / Model Store
- 即使当前本地模型功能未正式开放，也必须保留 UI 位置与扩展口

---

## 8.4 空状态

当没有本地模型时：

- 显示“尚未安装本地模型”
- 提供“前往 Store 安装模型”

---

## 9. 功能需求：Routing（高级）

## 9.1 模块定位

Routing 是 AI Center 的高级能力区，用于将系统逻辑模型名映射到具体可用模型集合上。

---

## 9.2 核心抽象

Logical Model 不是一个具体模型，而是一组候选模型与优先级列表。

例如：

```text
ai.plan
  1. OpenAI / GPT-x
  2. Anthropic / Opus
  3. Local / xxx

ai.chat
  1. SN Router / Default Chat
  2. OpenAI / GPT-x-mini
```

---

## 9.3 普通模式

默认策略：

- Routing 页面可以只显示“当前系统使用的默认逻辑模型”
- 不展示复杂编辑器
- 提供“进入高级模式”入口

---

## 9.4 高级模式

支持配置：

- 逻辑模型名
- 绑定的候选模型列表
- 优先级排序
- Fallback
- 未来支持：
  - 多 Key 自动切换
  - 多账号轮转
  - 并行多模型（预留）

---

## 9.5 示例逻辑名

- `llm.chat`
- `llm.plan`
- `llm.code`
- `llm.vision`

---

## 9.6 路由规则展示建议

用户在界面上不应只看到抽象字段，而应看到可理解的规则预览：

示例：

- `ai.plan` -> 优先使用 OpenAI GPT-X，失败后切到 Claude Opus
- `ai.chat` -> 优先使用 SN Router 默认模型

---

## 9.7 移动端处理

由于 Routing 高级配置较复杂，移动端建议：

- 默认只展示概要与只读信息
- 编辑动作使用全屏页或底部抽屉
- 长列表排序使用拖拽手柄
- 对复杂配置进行折叠分组

---

## 10. 数据模型建议

> 本节为产品视角的数据抽象，最终以后端模型为准。

## 10.1 ProviderConfig

```ts
type ProviderConfig = {
  id: string
  name: string
  provider_type: "sn_router" | "openai" | "anthropic" | "google" | "openrouter" | "custom"
  auth_mode?: "api_key" | "oauth"
  endpoint?: string
  protocol_type?: "openai_compatible" | "anthropic_compatible" | "google_compatible"
  api_key_ref?: string
  oauth_session_ref?: string
  auto_sync_models: boolean
  created_at: string
}
```

---

## 10.2 ProviderStatus

```ts
type ProviderStatus = {
  provider_id: string
  is_connected: boolean
  auth_status: "ok" | "expired" | "invalid" | "unknown"
  usage_supported: boolean
  balance_supported: boolean
  discovered_models: string[]
  model_sync_status: "ok" | "syncing" | "failed"
  last_verified_at?: string
  last_model_sync_at?: string
}
```

---

## 10.3 ProviderAccountStatus

```ts
type ProviderAccountStatus = {
  provider_id: string
  usage_supported: boolean
  cost_supported: boolean
  balance_supported: boolean
  usage_value?: number
  estimated_cost?: number
  balance_unit?: "usd" | "credit"
  balance_value?: number
  topup_url?: string
}
```

---

## 10.4 UsageEvent

```ts
type UsageEvent = {
  id: string
  timestamp: string

  provider_id: string
  model_name: string
  category: "text" | "image" | "audio" | "video"

  app_id?: string
  agent_id?: string
  session_id?: string

  tokens_in?: number
  tokens_out?: number
  token_equivalent?: number

  estimated_cost?: number
  status: "success" | "failed"
}
```

---

## 10.5 LogicalModelConfig

```ts
type LogicalModelConfig = {
  name: string
  candidates: Array<{
    provider_id: string
    model_name: string
    priority: number
    enabled: boolean
  }>
}
```

---

## 11. 指标与埋点建议

## 11.1 产品指标

- AI Center 首次启用完成率
- Add Provider 成功率
- Provider 验证失败原因分布
- 首页访问率
- Balance 卡片点击率
- Top Up 点击率
- Routing 高级模式启用率
- Provider 自动同步开启率

---

## 11.2 运行时指标

- Provider 可用率
- Provider 模型同步成功率
- Usage 统计延迟
- Balance 拉取成功率
- OAuth 失效率 / 续期率

---

## 12. Figma 风格 UI 布局指导

> 本节用于指导设计师在 Figma 中建立页面结构、Frame 命名、栅格、间距与响应式规则。  
> 原则上使用系统设置风格的稳重视觉，不做聊天产品式强情绪表达。

## 12.1 视觉设计原则

1. **系统级而非营销级**
   - 看起来像“系统能力面板”，不是活动页
   - 信息清晰、层级稳定、可读性优先

2. **成本与状态优先**
   - 用户最关心余额、Credit、可用性、消耗
   - 顶部摘要必须稳定、可扫读

3. **渐进式暴露**
   - 普通模式先看到简单结果
   - 高级配置按页面或开关展开，不一次性堆满字段

4. **观察优先**
   - Usage / Balance / 状态卡片优先级高于深层设置

---

## 12.2 Figma 文件建议结构

```text
AI Center
├── 00 Foundations
│   ├── Tokens
│   ├── Grids
│   ├── Icons
│   └── Components
│
├── 01 Desktop
│   ├── Home / Disabled
│   ├── Home / Enabled
│   ├── Providers / List
│   ├── Providers / Add / Step 1
│   ├── Providers / Add / Step 2
│   ├── Providers / Add / Step 3
│   ├── Providers / Add / Step 4
│   ├── Providers / Detail
│   ├── Models / Empty
│   ├── Models / List
│   ├── Routing / Overview
│   └── Routing / Advanced
│
└── 02 Mobile
    ├── Home / Disabled
    ├── Home / Enabled
    ├── Providers / List
    ├── Providers / Add / Step 1
    ├── Providers / Add / Step 2
    ├── Providers / Add / Step 3
    ├── Providers / Add / Step 4
    ├── Providers / Detail
    ├── Models / Empty
    ├── Models / List
    ├── Routing / Overview
    └── Routing / Advanced
```

---

## 12.3 Frame 命名规范

建议统一使用：

- `AI Center / Desktop / Home / Enabled`
- `AI Center / Desktop / Providers / Wizard / Step 1`
- `AI Center / Mobile / Providers / Detail`
- `AI Center / Mobile / Routing / Advanced`

这样有利于后续开发对照和原型评审。

---

## 12.4 栅格系统

## 桌面版（Desktop）

### 推荐 Frame
- 1440 宽主设计稿
- 内容区域最大宽度：1280
- 左侧导航栏：240
- 主内容左右内边距：32

### 栅格
- 12 列
- Gutter：24
- Column 自适应
- 模块间垂直间距：24 / 32

### 使用建议
- 摘要卡片区可做 4 卡并列
- 图表区可做 8 + 4 或 4 + 4 + 4
- 下方表格全宽

---

## 平板版（Tablet，可选中间稿）

### 推荐 Frame
- 1024 宽
- 8 列栅格
- 左右边距：24
- Gutter：20

### 使用建议
- 卡片区变为 2 x 2
- 复杂表格改为卡片 + 抽屉详情

---

## 移动版（Mobile）

### 推荐 Frame
- 390 宽（主稿）
- 可兼容 360 / 375 / 414

### 栅格
- 4 列
- 左右边距：16
- Gutter：12
- 卡片默认满宽堆叠

### 使用建议
- 所有摘要信息采用纵向堆叠
- 明细表格改为列表卡片
- 详情、筛选、编辑均优先使用全屏页或底部抽屉

---

## 12.5 间距与尺寸规范

建议采用 8pt 体系，并使用 Figma Variables 管理：

- 4：超紧凑
- 8：控件内间距
- 12：小组件间距
- 16：标准卡片内边距
- 24：模块间距
- 32：大模块间距

组件尺寸建议：

- 主按钮高度：
  - 桌面：36 / 40
  - 移动：44
- 输入框高度：
  - 桌面：40
  - 移动：44
- 卡片圆角：
  - 默认：12
  - 重点卡片：16
- 图表最小高度：
  - 桌面：220
  - 移动：180

---

## 12.6 文字层级建议

建议使用 4 级文本层次：

- Page Title：页面标题
- Section Title：模块标题
- Card Title：卡片标题
- Body / Meta：正文和辅助信息

不在 PRD 中强制具体字号，但需在 Figma 中建立清晰层级和 Token。

---

## 12.7 通用组件清单

建议在 Figma 中先搭建以下组件：

### 页面容器
- App Shell / Desktop
- App Shell / Mobile

### 导航
- Sidebar Item
- Top Bar
- Mobile Bottom Nav（可选）
- Segment Tabs

### 信息卡片
- Status Card
- Credit Card
- Cost Card
- Provider Summary Card
- Usage Summary Card

### 图表卡片
- Trend Chart Card
- Breakdown Chart Card
- Ranked List Card

### 列表与表格
- Provider Row / Card
- Model Row / Card
- Usage Detail Row
- Session Detail Card

### 表单
- Input
- Password Input
- Select
- Radio Group
- Stepper
- Validation Result Box
- Toggle

### 状态反馈
- Empty State
- Error State
- Loading Skeleton
- Success Banner

---

## 12.8 页面布局：桌面版

## A. Home / Usage（已启用）

### 桌面布局建议

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Top Bar: AI Center                                                  │
├──────────────┬───────────────────────────────────────────────────────┤
│ Sidebar      │ Page Header                                          │
│ - Home       ├───────────────────────────────────────────────────────┤
│ - Providers  │ [Status] [SN Credit] [Estimated Cost] [Balance]      │
│ - Models     ├───────────────────────────────────────────────────────┤
│ - Routing    │ [Usage Trend (8 cols)] [Category Breakdown (4 cols)] │
│              ├───────────────────────────────────────────────────────┤
│              │ [By Provider] [By Model] [By App/Agent]              │
│              ├───────────────────────────────────────────────────────┤
│              │ [Filters]                                             │
│              │ [Detail Table / Drill-down Table]                     │
└──────────────┴───────────────────────────────────────────────────────┘
```

### 版式规则
- 顶部 4 个摘要卡固定可见
- Usage Trend 占主视区最大面积
- Breakdown 卡片采用统一高度
- 明细表放在下方，支持滚动

### 建议默认内容顺序
1. Status
2. Credit
3. Estimated Cost
4. Provider Balance
5. Trend
6. Provider / Model / App Breakdown
7. 明细

---

## B. Home / Disabled（未启用）

### 桌面布局建议

```text
┌────────────────────────────────────────────────────────────┐
│ Page Header                                                │
├────────────────────────────────────────────────────────────┤
│ Empty Illustration / Status Icon                          │
│ AI 功能尚未启用                                            │
│ 需要至少添加一个 Provider 才能开始使用                    │
│ [开始启用]  [了解 Provider]                                │
└────────────────────────────────────────────────────────────┘
```

### 交互要求
- 主 CTA 为“开始启用”
- 次 CTA 为“了解 Provider”
- 下方可展示支持的 Provider 卡片预览

---

## C. Providers / List

### 桌面布局建议

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Page Header: Providers                         [Add Provider]        │
├──────────────────────────────┬───────────────────────────────────────┤
│ Provider List (5 cols)       │ Provider Detail Preview (7 cols)      │
│ - SN Router                  │ - Status                              │
│ - OpenAI                     │ - Auth                                │
│ - Anthropic                  │ - Models                              │
│ - Custom 1                   │ - Balance / Usage                     │
│                              │ - Actions                             │
└──────────────────────────────┴───────────────────────────────────────┘
```

### 规则
- 桌面优先使用分栏布局
- 点击左侧列表项，右侧更新详情预览
- 若无选中，则右侧显示引导信息

---

## D. Providers / Add Wizard

### 桌面建议：全屏页，而非小弹窗
原因：
- 表单复杂
- 要展示验证与模型发现
- 未来会有较多状态和错误解释

#### Step 1：Choose Provider Type
上部显示 Stepper，下部卡片栅格展示 Provider 类型。

#### Step 2：Connection Setup
左侧为表单，右侧可显示说明、示意、字段帮助。

#### Step 3：Validation & Model Discovery
主体为状态列表 + 已发现模型区域。

#### Step 4：Review & Create
摘要卡 + 操作按钮。

---

## E. Provider Detail

### 桌面布局建议

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Header: Provider Name                              [Edit] [Refresh]  │
├──────────────────────────────────────────────────────────────────────┤
│ [Connection Status] [Auth Status] [Balance/Credit] [Model Count]    │
├──────────────────────────────┬───────────────────────────────────────┤
│ Basic Info                   │ Models                                │
│ - Type                       │ - model 1                             │
│ - Endpoint                   │ - model 2                             │
│ - Auth Mode                  │ - sync status                         │
│ - Last Verified              │ - auto sync toggle                    │
├──────────────────────────────┴───────────────────────────────────────┤
│ Recent Usage / Cost Summary                                         │
└──────────────────────────────────────────────────────────────────────┘
```

---

## F. Models

### 桌面布局建议
- 顶部标题 + “前往 Store 安装模型”
- 下方本地模型表格
- 若为空，展示空状态说明

---

## G. Routing

### 桌面布局建议

普通模式：
- 逻辑模型列表
- 当前系统默认映射只读展示

高级模式：
- 顶部模式切换
- 左侧逻辑模型列表
- 右侧候选模型优先级配置
- 支持拖拽排序和启停

---

## 12.9 页面布局：移动版

## A. 导航方式

### 推荐方式
- 顶部 App Bar
- 主要页面采用分段或底部导航
- `Routing` 作为“更多 / 高级”入口，避免与 Home / Providers / Models 抢主导航优先级

### 推荐主导航
- Home
- Providers
- Models
- More / Advanced

---

## B. Home / Usage（已启用）

### 移动布局建议

基本逻辑就是把桌面的sliderbar放到顶部的tab,然后桌面的每个具体的tab的content panel要自己适配移动端

```text
┌────────────────────────────┐
│ App Bar: AI Center         │
├────────────────────────────┤
│ Home ｜ Providers | Models │
├────────────────────────────┤
│ Panel Content              │
└────────────────────────────┘
```


## Providers / Add Wizard

### 移动版原则
- 必须全屏分步骤
- 每一步单独一屏
- 顶部显示步骤进度
- 底部固定 CTA

#### Step 1
卡片列表选择 Provider 类型

#### Step 2
表单纵向排列，分组折叠高级项

#### Step 3
显示验证结果与模型发现列表；模型列表可滚动

#### Step 4
确认摘要 + 创建按钮


## 关键交互指导

### Stepper
- 桌面：顶部水平 Stepper
- 移动：顶部简化 Stepper + “步骤 X / 4”

### 验证反馈
- 连接验证与模型发现要可视化展示结果
- 用列表逐项显示：
  - Endpoint reachable
  - Auth valid
  - Models discovered
  - Balance capability available / unavailable

### 深钻
- 桌面：同页 drill-down
- 移动：新页或底部抽屉

### 过滤器
- 桌面：表格上方横向排列
- 移动：进入筛选抽屉

---

## 12.12 Figma 原型优先产出顺序

建议设计优先顺序如下：

1. Home / Disabled（首启）
2. Providers / Add Wizard（Step 1~4）
3. Home / Enabled（默认首页）
4. Providers / List + Detail
5. Models / Empty + List
6. Routing / Overview
7. Routing / Advanced

---

## 13. 交互文案建议（可直接用于原型）

## 13.1 首启
- 标题：AI 功能尚未启用
- 描述：需要至少添加一个 AI Provider，才能开始使用系统中的 AI 能力。
- 主按钮：开始启用

---

## 13.2 Add Provider
- 标题：添加 Provider
- 副标题：把外部 AI 服务接入 AI Center
- Step 1：选择一种 Provider 类型
- Step 2：填写连接信息
- Step 3：验证连接并发现模型
- Step 4：确认并创建

---

## 13.3 验证成功
- 已成功连接到 Provider
- 已发现可用模型
- 模型列表将自动保持同步

---

## 13.4 空状态
- 尚未安装本地模型
- 前往 Store 安装模型
- 尚无 AI 调用记录
- 当前 Provider 无法提供余额信息

---

## 14. 兼容性与响应式要求

## 14.1 桌面版要求
- 优先使用分栏信息密度
- 支持复杂表格和并列图表
- 支持 Hover、Inline Actions、右侧详情预览

## 14.2 移动版要求
- 以“单列纵向堆叠 + 全屏编辑”为主
- 不依赖 Hover
- 避免横向滚动表格
- 复杂配置使用分步骤或折叠组
- 保证 Provider 添加流程可完整在手机端完成

## 14.3 一致性要求
- 核心信息架构一致
- 关键术语一致
- 不同端只改变呈现方式，不改变概念和流程定义

---

## 15. 风险与待确认问题

1. **OpenAI 联合登录**
   - 当前记为“OAuth / 联合登录”
   - 需进一步确认技术实现、会话时效与续期策略

2. **Provider 余额能力不统一**
   - 各家 API 能力差异大
   - UI 必须允许部分能力缺失

3. **成本估算准确性**
   - 系统显示的费用可能与最终账单不完全一致
   - 需明确“Estimated Cost”标识

4. **本地模型尚未上线**
   - 当前以 UI 预留为主
   - 安装流程先统一跳转 Store

5. **Protocol Type 枚举**
   - 当前以产品抽象表达
   - 需与工程实现枚举最终对齐

---

## 16. 里程碑建议

## Phase 1：可用首版
- 首启启用页
- Add Provider Wizard
- Home / Usage（基础用量 + Credit）
- Providers 列表与详情
- Models 空状态与跳转
- Routing 只读概要

## Phase 2：增强版
- Provider 余额与费用展示增强
- App / Agent / Session 深钻
- 模型自动同步能力完整化
- Routing 高级编辑
- 更细粒度的错误解释

## Phase 3：高级版
- 多 Key / 多账号策略
- 并行多模型与高级路由
- 配额 / 限额控制
- 更强审计与权限体系

---

## 17. 最终结论

AI Center 不是单一的设置页，而是一个系统级 AI 控制中心：

- 对普通用户，它是 AI 的启用入口、余额中心、用量总览
- 对高级用户，它是 Provider 管理器、模型聚合器、路由控制台
- 对系统，它是 Agent Runtime 的 AI Control Plane 和 Observability Layer

当前版本最关键的体验顺序应是：

1. **先让用户顺利启用**
2. **再让用户清楚看到余额 / Credit / 用量**
3. **最后逐步开放复杂的模型与路由能力**

---
