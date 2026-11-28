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

## 0.1 文档修订记录

- **2026-05-29 概念对齐**：本 PRD 初版在定义阶段对 AICC 的理解落后于工程实现。原文把 AICC 当作"带 fallback 链的模型聚合器"，实际 AICC 已是"**任务化逻辑目录 + 调度器 + 运行时可观测平面**"。本次修订按工程真相源重写了核心概念（§3）、信息架构（§4）、用量分类（§6.5）、Providers/Models 概念边界（§7/§8）、整个 Routing（§9）与数据模型（§10），并新增 Route Trace 可观测能力。**交互设计（尤其 Routing 与 Models 页）需据此重做。**
  - 概念真相源：`doc/aicc/aicc 逻辑模型目录.md`、`doc/aicc/aicc_router.md`、`src/frame/aicc/src/model_types.rs`、`src/frame/aicc/src/model_session.rs`、`src/frame/aicc/src/default_logical_tree.rs`。
- **2026-11-27 用户视角的路由**：路由对用户来说是一张**用途表**（用途 → 一个精确模型 `model@provider`），默认路由稳定，Provider 不可用不改变路由、系统不自动换模型；换模型只来自手工配置的“不可用时”策略。删除 fallback 模式（`parent` / `target_*` 等）、`runtime_failover`、`allow_fallback`、`cost_first` 和请求携带的 `session_overlay`；`llm.fallback` 改为默认关闭的“兜底 LLM”。新增用途页、路由策略组、添加 Provider 时的用途覆盖步骤、Provider 的“标记为故障”。改动见 §3.5、§3.6、§3.8、§3.10、§4.1、§5.2、§5.5、§6.3、§7、§9、§10、§12。
  - 规范：[aicc_router.md](../../doc/aicc/aicc_router.md) §9、§11、§12；设计与裁决：[AICC 用户视角的模型路由改进 TODO](../../notepads/aicc-user-centric-routing-todo.md)。

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
- 需要按用途选模型、设置 Provider 不可用时的策略、为不同 App / 会话准备不同的路由策略组

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

> ⚠️ 本节于 2026-05-29 按 AICC 工程实现重写。AICC 不是"模型聚合器"，而是"任务化逻辑目录 + 调度器 + 可观测平面"。理解 §3.1 的三层命名是理解整个 AI Center 的前提。

## 3.1 三层模型命名体系（最核心）

AICC 里不存在"模型名"这一个层次，而是三层。原 PRD 把它们压平成"逻辑名 + 具体名"两层，丢掉了中间的 Provider 逻辑挂点，这是后续 Routing/Models 概念全部错位的根源。

| 层 | 名称 | 例子 | 含义 | 谁用 |
|---|---|---|---|---|
| **L3 任务抽象目录** | Logical Role Path | `llm.plan` `llm.code` `image.txt2img` `audio.tts` | 按"要做什么任务"组织的调用入口，不绑定厂商 | **Agent / App 调用时用这个** |
| **L2 Provider 逻辑挂点** | Logical Mount | `llm.opus` `llm.gemini-pro` `llm.haiku` | 某 Provider 内稳定的 tier/角色，永远指向该角色"当前最新版" | 路由树内部软链 / 用户偏好 |
| **L1 物理精确型号** | Exact Model | `claude-opus-4.7@anthropic` `gpt-5.5@openai` | 带版本快照的具体型号，格式 `provider_model_id@provider_instance_name` | 复现 / 锁定 / 审计 / Provider 实际执行 |

调用链：Agent 请求 `llm.plan` → 路由树展开到若干 L2 挂点（`llm.opus` 等，带权重）→ 解析到 L1 精确型号 `claude-opus-4.7@anthropic` → 实际执行。

**对 UI 的影响**：Routing 页要展示的是这棵三层目录树，不是"逻辑名 → 候选列表"的扁平表；Model 标识必须用 `model@provider_instance` 全名（裸模型名会重名）。

---

## 3.2 API Type（受控能力枚举）

每个精确模型声明它支持哪些 `api_type`，这是请求/响应 schema 的契约。**Provider 不能自定义 api_type**，只能从受控集合里选，新增要主版本升级。

命名空间与典型 api_type：

- `llm`：`llm.chat`（事实标准）、`llm.completion`（历史）
- `embedding`：`embedding.text`、`embedding.multimodal`
- `rerank`：`rerank`
- `image`：`image.txt2img / img2img / inpaint / upscale / bg_remove`
- `vision`：`vision.ocr / caption / detect / segment`（结构化输出；要自由文本走 `llm.chat` 传 image block）
- `audio`：`audio.tts / asr / music / enhance`
- `video`：`video.generate / edit / extend / upscale`（文本、首尾帧、参考素材都由 `video.generate` 承载）
- `agent`：`agent.computer_use`（占位，高速演进中）

**对 UI 的影响**：用量分类（原 PRD 的 text/image/audio/video 四桶）要换成这套 namespace，否则 embedding/rerank/vision/ocr 无处归类。

---

## 3.3 Provider 与 Provider Instance

原 PRD 把 Provider 当"厂商枚举"（一类一个）。AICC 里 Provider 是**实例（instance）**，同一厂商/驱动可有多个实例，用四个正交维度描述：

- `provider_instance_name`：实例唯一名（精确模型名的 `@` 后缀就是它）
- `provider_type`：`local_inference` | `cloud_api` | `proxy_unknown`
- `provider_driver`：实际驱动（openai / claude / gemini / minimax / fal / sn ...）
- `provider_origin`：`system_config` | `user_config` | `builtin` | `provider_claimed`

Provider 通过 **inventory** 声明自己提供哪些模型（`ProviderInventory.models[]`），每个模型用 `logical_mounts` 把自己挂到 L2 逻辑挂点。**模型发现 = 读 inventory**，不是单独一个"discover"动作。

**对 UI 的影响**：同厂商多 Key / 多 Endpoint / 多账号（Phase3 自己想要的能力）的前提，就是把 Provider 建模成实例。本地模型也是一个实例（见 §3.7）。

---

## 3.4 Model Metadata 富对象

模型不是一个名字字符串，而是一个富对象。这些字段正是"模型聚合器"UI 最该展示、原 PRD 却完全没建模的东西：

- 身份：`provider_model_id`、`exact_model`、`logical_mounts[]`、`api_types[]`、`parameter_scale`
- `capabilities`：`streaming` / `tool_call` / `json_schema` / `web_search` / `vision` / `max_context_tokens` / `max_output_tokens`
- `attributes`：`local`、`privacy`(local/cloud/private_safe/public_cloud)、`quality_score`、`latency_class`、`cost_class`、`tier`(flagship/mid/nano)
- `pricing`：`input_token_usd` / `output_token_usd` / `cache_input_token_usd`
- `health`：`status`(available/degraded/unavailable)、`p50/p95_latency_ms`、`error_rate_5m`、`quota_state`(normal/near_limit/exhausted)

---

## 3.5 逻辑目录树（Logical Directory / routing 配置）

默认路由由一棵**树**展开得出，而非列表：

- 系统配置 `services/aicc/settings` 的 `routing`（原字段名 `session_config`）：`logical_tree`（`path -> LogicalNode`）、`global_exact_model_weights`、`provider_weights`、`routing_commands`、`policy`，以及管理员层的用途配置 `usecases`（§3.10）
- `LogicalNode`：`children`（子节点）、`items`（候选，`name -> {target, weight}`）、`item_overrides`、`exact_model_weights`、`policy`
- 候选 `target` 可以指向**另一个逻辑路径**（递归），也可以是精确型号
- 选择靠**权重（weight）**，不是简单优先级排序；展开只看静态条件（库存、能力、Provider 启用、凭据、策略），不看健康状态
- **目录之间没有回退**：目录没有候选就返回无候选，不回退到父目录或别的目录。唯一的例外是默认关闭的兜底 LLM（§3.10）
- `locked` 锁定：管理员锁定的策略或用途格子，用户层、策略组和请求都不能改

这棵树是内部结构，只在“路由细节”（§9.4）里出现；用户看到和调整的是用途表（§3.10、§9.3）。

内置默认树见 `default_logical_tree.rs`（revision `builtin-aicc-router-v2`）。Provider inventory 可用 `item_overrides` 把精确模型挂到角色目录，而不被内置树覆盖。

---

## 3.6 调度 Profile 与 Policy

默认路由"在候选里怎么选"由 profile + policy 控制（**不是拖优先级**）。它们是路由细节里的高级旋钮；普通用户直接在用途表里选模型（§3.10）：

- `SchedulerProfile`：`latency_first` | `quality_first` | `balanced`（默认）| `local_first` | `strict_local`；`cost_first` 已删除
- 每个 profile 是一组权重：`cost / quality / preference / cache / local`；健康、延迟等运行观测不参与排序
- 请求携带的 `RoutePolicy` **只能收紧**：`local_only`、`no_fallback`（本次关闭用途的“不可用时”策略）、`allowed / blocked_provider_instances`（允许列表与管理员的取交集）、更低的 `max_cost`、`max_latency_ms`（按观测延迟的硬门槛）；未被管理员锁定时可选 `fast` / `balanced` / `quality` profile。请求不能选用途的模型，也不能打开“不可用时”策略

---

## 3.7 本地模型 = 本地 Provider Instance

> 这条直接决定 PRD 的 Models / Providers 切分方式。

产品决策：**Models 页管理"装在本地的模型"，Providers 页管理"云端 Provider"**。但在 AICC 后端，二者**统一为 provider instance**——本地模型就是一个 `provider_type=local_inference`、精确名后缀 `@local` 的实例，它的模型同样带 `logical_mounts`、同样进同一棵逻辑目录树参与路由。

所以前端 Models / Providers 是对**同一份 inventory 的两个视图切分**（按 `provider_type` / `attributes.local`），不是两套独立数据。本地模型不是"路由之外的东西"，它和云端模型在同一棵树里按 profile 竞争（`local_first` / `strict_local` profile 就是为它服务的）。

---

## 3.8 Route Trace（路由解释，新增可观测概念）

每次请求 AICC 都产出一份 `RouteTrace`。这是相对各家 Provider 后台最强的差异化，原 PRD 完全没有这个概念：

- `requested_model` + `requested_model_type`(exact/logical) → `resolved_logical_path` → `selected_exact_model` + `selected_provider_instance_name`
- `ranked_candidates[]`（每个候选的打分与是否选中）、`filtered_candidates[]`（被过滤原因）
- 用途来源：这次用的模型来自哪一层（默认 / 管理员 / 我 / 某个策略组）、命中的 App 或会话策略组、“不可用时”策略及其来源
- `fallback`：答复不是来自用途自己的模型时出现。`kind = coverage` 表示用途没有模型、由兜底 LLM 提供；`kind = unavailable` 表示原模型不可用、按“不可用时”策略换成了另一个模型（`from` / `to` / `policy`）
- `scheduler_profile`
- `user_summary`：面向普通用户的人话版（"用了 Claude Opus，因为这是规划任务的高质量首选"；"gpt-codex@openai 不可用，按‘同一个模型换 Provider’改用 gpt-codex@openrouter"）

---

## 3.9 Usage / Balance / PricingMode

- **Usage**：基本单位 token，落库为 `UsageEvent`（见 §10），可按 tenant / app / agent / session / provider / model / api_type / 时间聚合。
- **PricingMode**：`per_token` | `subscription` | `free_quota` | `unknown`——三种计费模式的余额展示逻辑完全不同，原 PRD 只考虑了 per_token。
- **Balance / Credit**：Provider 可查询余额则显示（否则 Unknown）；SN Router 用 Credit 体系，避免多币种/多充值渠道复杂度直接暴露给用户。成本不可得时退化为 "Estimated / Usage only"。

---

## 3.10 用途表、不可用时策略与路由策略组（2026-11-27 新增）

> 这是用户理解和调整路由的唯一入口。规范见 [aicc_router.md](../../doc/aicc/aicc_router.md) §9、§11、§12。

- **用途（usecase）**：逻辑目录里的 29 个任务入口（`llm.chat`、`llm.code`、`image.txt2img`、`audio.tts`……，完整列表见 §9.5）。每个用途路由到**一个精确模型** `model@provider`。
- **默认路由是稳定的**：相同的配置和库存得到相同的模型。只有添加、删除、启用、停用 Provider，Provider 模型列表刷新、Model Driver metadata 更新，以及管理员的权重命令会改变它；Provider 故障、健康、延迟、价格估算、请求大小都不会。
- **层次**：`默认 < 管理员层（Zone 默认）< 用户层（我）< App 绑定的策略组 < 会话绑定的策略组`。每个用途有两格——“模型”和“不可用时”——各由设了它的最高一层决定；没设就跟随下一层。管理员可以锁定格子，锁定后更高的层不能改。某一层选的模型已不在库存、对当前用户不可见或不满足用途要求时，这一格跟随下一层，并在用途表上标出。
- **“不可用时”策略（`on_unavailable`）**：

| 值 | 界面文案 | 含义 |
|---|---|---|
| `none` | 不切换（默认） | 返回 Provider 的真实错误 |
| `same_model` | 同一个模型换 Provider | 换到别的 Provider 上的同一个模型 |
| `not_pricier` | 换下一个不更贵的模型 | 把出故障的 Provider 视作停用、重新路由；只接受每项单价都已知且不更高的模型，价格未知的不算 |
| `next_model` | 换下一个模型 | 把出故障的 Provider 视作停用、重新路由，用得到的第一个模型 |

每次请求最多换一次，只在同模型重试之后、因超时 / 5xx / 连接失败、模型下线、上游额度耗尽、用户额度规则拒绝或 Provider 被标记为故障而失败时触发；参数错误、认证失败、权限拒绝、取消和已产生用量的失败从不切换。请求直接写精确模型时从不切换。只接了一个 Provider 时没有可换的，这是预期的。
- **兜底 LLM（`llm.fallback`）**：默认关闭，只有管理员能打开。打开后，七个 LLM 用途中没有任何模型的由它提供，并在会话里显示警告；已覆盖的用途不受影响，Provider 故障也不会转到它。
- **标记为故障**：管理员对出问题的 Provider 手工标记。路由和用途表都不变，但请求不再发给它：配置了“不可用时”策略的用途直接用备选，没配置的立即报错。长期故障应停用 Provider（默认路由随之重算）。
- **路由策略组**：用户可以建多个策略组，每组是一份“用途 → 模型 + 不可用时”的配置，内容和用户层相同。策略组绑定到 App（或 Agent）后对通过它发出的调用生效；绑定到 LLM 会话后对该会话和它的子会话生效，会话绑定优先于 App 绑定。策略组只属于建它的用户、只对这个用户的调用生效。

---

## 4. 产品边界与整体结构

## 4.1 总体信息架构（IA）

```text
AI Center
├── Home / Usage（默认首页）
│   ├── AI Status（含 Provider/Model 健康 health、配额 quota；未覆盖的用途、被标记故障的 Provider、最近的策略切换）
│   ├── Balance / Credit
│   ├── Usage Summary（按 api_type namespace 分类）
│   ├── Time Series
│   ├── By Provider / Model / App / Agent / Session
│   ├── Route Trace / 最近路由解释（新增可观测）
│   └── Drill-down Table
│
├── Providers（云端 Provider 实例视图：provider_type = cloud_api）
│   ├── Provider List
│   ├── Add Provider Wizard
│   └── Provider Detail（inventory / 模型富信息 / 健康 / 标记为故障）
│
├── Models（本地模型视图：同一份 inventory 中 provider_type = local_inference 的实例）
│   ├── Local Models List
│   ├── Empty State
│   └── Jump to Store
│
├── 用途（Usecases，普通用户可见，放在 Routing 前面）
│   ├── 用途表（视角：我 / 某个策略组 / 某个 App；管理员另有“Zone 默认”）
│   ├── 编辑面板（选模型、选“不可用时”策略、恢复为跟随下一层）
│   └── 兜底 LLM（管理员开关）
│
├── 路由策略组（Routing Groups）
│   ├── 策略组列表与编辑（复用用途表）
│   └── 绑定到 App / Agent
│
└── 路由细节（Routing，逻辑目录树，高级；写操作只对管理员显示）
    ├── Directory Overview（三层目录树，只读）
    ├── 权重（items weight、Provider 权重、routing commands）
    ├── Scheduler Profile / Policy
    └── Route Trace Explorer
```

> **重要**：Providers 与 Models 不是两套独立数据，而是对同一份 Provider inventory 的两个视图切分（按 `provider_type` / `attributes.local`）。详见 §3.7。用户调整路由的对象是用途表（§3.10、§9.3）；三层目录树只在“路由细节”里作为默认路由的解释出现。

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
- 用途表使用默认路由；只接一个 Provider 时“不可用时”策略没有可换的模型

### 状态 C：已启用（多 Provider / 多模型）
表现：

- Usage 中出现多维度统计
- 用途页可以为每个用途选模型、设置“不可用时”策略；策略组与路由细节开放
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
- 用途覆盖：添加 Provider 的首要目标是让每个用途都有模型
- 同步策略确认
- 加入系统资源池

### 用途覆盖
在验证之后、确认之前，向导对比“现在”和“加上这个 Provider 以后”整个 Zone 的用途表（全部 29 个用途，数据来自 `provider.validate` 的 `usecase_coverage`），分三组列出：

- **新覆盖的用途**：原来没有模型，加上以后有了
- **默认模型会变的用途**：显示旧模型 → 新模型。每条可以取消勾选，取消就把旧模型写进管理员层，这个用途保持原样（来源显示“管理员”）
- **仍未覆盖的用途**：给出原因（没有接入该类模型 / 有模型但不满足用途要求，例如上下文不够、没有 tool call）

确认页显示摘要（新覆盖几个、变化几个、仍未覆盖几个）。添加之后默认路由直接按新库存生效，不需要另外确认。

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

### 普通模式（用途页）
- 任何能使用 AI 的用户都能打开用途页，看到**对自己生效的**用途表：每个用途现在用哪个模型、这个选择来自哪一层、Provider 出问题时会发生什么、是否已覆盖（§9.3）
- 系统按默认路由给每个用途选好模型；用户不改也能用
- 用户可以为自己改某个用途的模型或“不可用时”策略，也可以恢复为跟随下一层；管理员锁定的格子显示锁、不能改
- 管理员在“Zone 默认”视角编辑管理员层，并管理兜底 LLM 开关

### 高级模式
- 路由策略组：建多份用途配置，绑定到 App / Agent，或在会话设置里绑定到 LLM 会话（§9.4）
- 路由细节：三层逻辑目录树（L3 任务目录 / L2 Provider 挂点 / L1 精确型号，见 §3.1）、候选 **weight**、Provider 权重、**Scheduler Profile** 与 Policy（非"优先级排序"），只对管理员显示写操作
- 通过 Route Trace 解释每次实际路由结果，包括来源层、是否由兜底 LLM 提供、是否按策略切换
- 支持未来的多 Key / 多账号

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
- 当前可用 Provider 实例数
- 当前可用 Model 数
- **模型健康概览**：available / degraded / unavailable 各计数（来自 `ModelHealth.status`）
- **配额预警**：处于 near_limit / exhausted 的模型/实例数（来自 `QuotaState`）
- **未覆盖的用途**：数量，以及其中由兜底 LLM 提供的数量；点进用途页
- **被标记故障的 Provider**：名称与标记时间；点进 Provider 详情
- **最近的策略切换**：最近一次“不可用时”策略切换的用途、原模型 → 实际模型、原因；点进用量明细（按“由策略切换”筛选）
- 最近一次 inventory 刷新是否正常（可选）

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

### 分类维度（按 api_type namespace，对齐 §3.2）
- `llm`（chat / completion）
- `embedding`（text / multimodal）
- `rerank`
- `image`（txt2img / img2img / inpaint / upscale / bg_remove）
- `vision`（ocr / caption / detect / segment）
- `audio`（tts / asr / music / enhance）
- `video`（generate / edit / extend / upscale）
- `agent`（computer_use，预留）

说明：

- 原 PRD 的 text/image/audio/video 四桶装不下 embedding / rerank / vision，统一改用 api_type namespace。UI 上可默认折叠到一级 namespace，展开看二级 api_type。
- 尽管底层计费口径不同（见 §3.9 PricingMode），系统层统一抽象为 Usage 统计；成本可为 Token 或 Token Equivalent 上的估算值。

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
- 调用失败时显示 Provider 返回的真实原因（Provider、精确模型、上游状态与原文）；系统不会自动换到别的模型
- 临时故障可以“标记为故障”，长期故障建议停用（§7.4）

### 成本不可用
- 允许显示 Token 统计
- 成本区域显示“Estimated unavailable”

---

## 7. 功能需求：Providers

## 7.1 模块定位

Providers 模块用于接入和管理**云端 Provider 实例**（`provider_type = cloud_api`）。这是当前阶段 AI Center 的核心入口。

概念对齐（见 §3.3）：

- 这里管理的是 **Provider 实例（instance）**，不是"厂商枚举"。同一厂商可有多个实例（多 Key / 多 Endpoint / 多账号）。
- 每个实例的模型来自它的 **inventory**（`ProviderInventory.models[]`），即"模型发现 = 读 inventory"。
- 本地模型实例（`provider_type = local_inference`）虽然同属 inventory，但归到 Models 页展示（§8）。Providers 页只列云端实例。

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
- 是否已被标记为故障
- 操作入口

操作包括：

- 查看详情
- 刷新模型列表
- 更新认证信息
- 重新授权
- 停用 / 删除 Provider：确认框先列出会受影响的用途——由谁接替、哪些会变成未覆盖（用 `routing.usecases` 的 `without_provider` 预演“没有这个 Provider 时”的用途表，和当前表对比）

---

## 7.3 Add Provider Wizard

## 7.3.1 设计原则

- 必须是向导，不是一次性大表单
- 先选类型，再展开字段
- 创建前必须做连接验证和模型发现
- 创建前展示用途覆盖的变化（§5.2）
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

### Step 4：用途覆盖

对比“现在”和“加上以后”整个 Zone 的用途表（§5.2）：

- 新覆盖的用途
- 默认模型会变的用途（旧模型 → 新模型），每条可以取消勾选；取消的条目在创建时把旧模型写进管理员层
- 仍未覆盖的用途及原因

窄屏下每个用途一张卡片，旧模型和新模型竖排。

---

### Step 5：创建确认

确认页显示：

- Provider 类型
- 认证方式
- 连接状态
- 已发现模型数量
- 用途覆盖摘要：新覆盖 / 变化（其中保持旧模型的条数）/ 仍未覆盖
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
- 健康状态与最近的失败（只用于展示和告警，不改变路由）
- 是否已被标记为故障

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
- **标记为故障**（开关，管理员）：说明文案“路由不变；配置了不可用策略的用途直接改用备选，其余直接报错；长期故障请停用 Provider”。标记和清除都会记事件
- 停用 / 删除 Provider（确认框列出受影响的用途，见 §7.2）

---

## 8. 功能需求：Models

## 8.1 模块定位

Models 模块管理 **本地已安装模型**，不负责本地模型下载实现本身。

概念对齐（见 §3.7）：

- 本地模型在 AICC 后端就是一个 `provider_type = local_inference`、精确名后缀 `@local` 的 **Provider 实例**。Models 页是对同一份 inventory 的"本地视图"，不是独立数据源。
- 本地模型同样带 `logical_mounts`、同样进逻辑目录树参与路由，和云端模型在同一棵树里按 profile 竞争（`local_first` / `strict_local`）。因此 Models 页的模型项也应能展示其挂载的逻辑路径与健康状态。

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

## 9. 功能需求：Routing（用途表、策略组与路由细节）

> ⚠️ 本节于 2026-05-29 整体重写，2026-11-27 按“用户视角的路由”再次重写：用户理解和调整的是**用途表**，三层目录树退到“路由细节”。规范见 [aicc_router.md](../../doc/aicc/aicc_router.md) §9、§11、§12；设计与裁决见 [AICC 用户视角的模型路由改进 TODO](../../notepads/aicc-user-centric-routing-todo.md) §3、§4.9。

## 9.1 模块定位

Routing 区由三页组成，按手机宽度设计：

| 页 | 谁能用 | 内容 |
|---|---|---|
| **用途**（导航里放在其它路由页前面） | 所有能使用 AI 的用户 | 对自己生效的用途表；编辑自己的层；管理员另有“Zone 默认”视角和兜底 LLM 开关 |
| **路由策略组** | 所有能使用 AI 的用户（只看、只改自己的组） | 组列表、编辑（复用用途表）、绑定到 App / Agent |
| **路由细节**（原 Routing 页） | 所有人可看，写操作只对管理员显示 | 三层目录树、权重、profile、Route Trace Explorer |

数据来自 kRPC `routing.usecases`、`routing.usecase.set`、`routing.groups.*`、`routing.bindings.*`；路由细节沿用 `routing.get` / `routing.update` / `routing.preview`（§10.5）。

---

## 9.2 核心抽象（对齐 §3.5、§3.10）

- **用途表**：29 个用途，每个用途 → 一个精确模型 `model@provider`，外加一行兜底 LLM（`llm.fallback`）。
- **两格**：每个用途有“模型”和“不可用时”两格，各由设了它的最高一层决定：`默认 < 管理员层（Zone 默认）< 用户层（我）< App 绑定的策略组 < 会话绑定的策略组`。管理员层可以锁定格子。
- **模型只能选精确模型**：第一期不能只选模型、不选 Provider；多个 Provider 提供同一个模型时，用“同一个模型换 Provider”策略表达。选的模型必须对当前用户可见、API 类型与用途一致、满足用途要求（上下文、tool call 等）。
- **“不可用时”**：`none` 不切换（默认）/ `same_model` 同一个模型换 Provider / `not_pricier` 换下一个不更贵的模型 / `next_model` 换下一个模型（§3.10）。
- **默认路由**仍由逻辑目录树按权重展开得出（§3.5）。示例（内置树片段）：

```text
llm.plan                       # L3 任务目录，profile=quality_first
├── opus      → llm.opus       weight 2.5   # L2 挂点
├── gemini    → llm.gemini-pro weight 2.4
├── qwen_max  → llm.qwen-max   weight 1.8
└── deepseek  → llm.deepseek-pro weight 1.5
        ↓ 各 L2 挂点再解析到 L1
   llm.opus → claude-opus-4.7@anthropic
```

权重决定“接入了哪些模型时默认用谁”，不是故障时的切换顺序：`llm.opus` 的 Provider 临时故障不会让 `llm.plan` 改用 Gemini。

---

## 9.3 普通模式（用途页）

用途表每一行：

| 用途 | 当前模型 | 来源 | 不可用时 | 状态 |
| --- | --- | --- | --- | --- |
| 编码 `llm.code` | `gpt-codex-5.2@openai` | 管理员 | 同一个模型换 Provider → `gpt-codex-5.2@openrouter` | 已覆盖 |
| 总结 `llm.summarize` | `gpt-5.5-mini@sn` | 默认 | 不切换 | 已覆盖 |
| 规划 `llm.plan` | `gpt-5.5-mini@sn` | 兜底 LLM | 不切换 | 未覆盖：没有满足上下文要求的模型，由兜底 LLM 提供 |
| 生成视频 `video.generate` | — | — | — | 未覆盖：没有接入能生成视频的模型 |

- **来源**：决定这一格的那一层——默认 / 管理员 / 我 / 策略组名。被管理员锁定的格子显示锁，不能改。
- **不可用时**：同时显示策略和**按现在的配置会换到谁**（假设当前模型所在的 Provider 不可用时预演出来）。算不出时写原因：“没有已知价格不高于它的模型”（提示管理员补价格或改用“换下一个模型”）或“没有别的模型”。
- **状态**：已覆盖 / 由兜底 LLM 提供 / 未覆盖，未覆盖时写原因（没有接入该类模型 / 有模型但不满足用途要求）。某一层选的模型不能用时标出原因——“所选模型已不可用”“你无权使用”“不满足用途要求”——这一格跟随下一层。
- **视角**：顶部切换“我 / 某个策略组 / 某个 App”；管理员多一个“Zone 默认”，编辑的是管理员层。“某个 App”视角只读，显示该 App 绑定的策略组生效时的样子。
- **编辑面板**（点一行打开）：
  - 选模型：只列出对自己可见、满足用途要求的模型，按家族分组，标价格
  - 选“不可用时”：四个选项各附一句解释，实时显示“会换到谁”；`not_pricier` 与 `next_model` 的说明里写明“系统按已知价格判断，价格未知的模型不会被选为‘不更贵’”，`next_model` 提示可能换到更贵的模型
  - 恢复为跟随下一层（清除这一格）
  - 管理员在“Zone 默认”视角可以锁定格子
  - 写操作带版本号，版本冲突时提示刷新
- **兜底 LLM 行**：单独一行，默认关闭，只有管理员能开关。打开的含义是“这个 Zone 至少有一个 LLM 可以用来维护系统”：七个 LLM 用途中没有模型的由它提供，使用它的会话显示警告（例如“规划没有配置模型，正在使用兜底模型 X”）。它不替代故障的 Provider。
- 选模型的交互后续会继续优化，帮用户完成手工调整。

---

## 9.4 高级模式（路由策略组与路由细节）

**路由策略组页**：

- 组列表：名称、包含的用途数、已绑定的 App / Agent
- 编辑：复用用途页的表与编辑面板，只能改组内容，不能锁定
- 绑定到 App：从 App / Agent 列表选；同一个 App 只能绑定一个组。删除组时一并解除它的 App 绑定
- 会话绑定不在这里：在会话自己的设置里（OpenDAN WebUI / Jarvis 对话的会话菜单）选择策略组，对该会话和子会话生效；AI Center 不列出会话绑定
- 第一期管理员不建共享组、不替所有用户给 App 绑组，只用管理员层 + 锁定

**路由细节页**（原 Routing 页）：

- 浏览三层目录树（L3 / L2 / L1），解释默认路由为什么选了这个模型
- 管理员编辑候选 **weight**、Provider 权重与 routing commands（高级调权，影响默认展开）
- 管理员层的 **Scheduler Profile**（latency_first / quality_first / balanced / local_first / strict_local）与 **Policy**：`local_only`、`allowed / blocked_provider_instances`、`max_estimated_cost`
- 理解 `locked`：管理员锁定的字段，用户层、策略组和请求不可改（UI 需置灰并解释）
- Route Trace Explorer：来源层、命中的策略组、是否由兜底 LLM 提供、是否按策略切换
- 未来支持：多 Key、多账号（预留）

> 交互提示：weight 调整建议用滑杆/数值而非拖拽排序；profile 是单选；目录树用可展开的树形控件而非平铺表格。

---

## 9.5 一级 namespace 与用途

一级 namespace（每个带默认 profile，见 §3.2 / 逻辑目录文档 §二）：
`decision`、`llm`、`embedding`、`rerank`、`image`、`vision`、`audio`、`video`、`agent_runtime`、`multimodal`(占位)。

用途表的 29 个用途：

- `decision`
- `llm.chat`、`llm.plan`、`llm.code`、`llm.swift`、`llm.summarize`、`llm.translate`、`llm.vision`
- `embedding.text`、`embedding.multimodal`、`rerank`
- `image.txt2img`、`image.img2img`、`image.inpaint`、`image.upscale`、`image.bg_remove`
- `vision.ocr`、`vision.caption`、`vision.detect`、`vision.segment`
- `audio.tts`、`audio.asr`、`audio.music`、`audio.enhance`
- `video.generate`、`video.edit`、`video.extend`、`video.upscale`
- `agent_runtime.computer_use`

另有一行兜底 LLM `llm.fallback`。规格和家族目录（如 `llm.gpt-mini`）仍可以被请求直接使用，但不出现在用途表里。

---

## 9.6 路由规则展示建议

用户在界面上不应只看到抽象字段，而应看到可理解的规则预览 + 实际路由解释（Route Trace）：

- 规则预览：`llm.code` → `gpt-codex-5.2@openai`（来源：管理员）；不可用时同一个模型换 Provider，会换到 `gpt-codex-5.2@openrouter`
- 实际解释（来自最近一次 Route Trace 的 `user_summary`）：`llm.code` → 本次选中 `gpt-codex-5.2@openai`；未切换
- 切换解释：`gpt-codex-5.2@openai` 超时，按“同一个模型换 Provider”改用 `gpt-codex-5.2@openrouter`（用量记录里标出原模型）
- 兜底解释：`llm.plan` 没有配置模型，本次由兜底 LLM 的 `gpt-5.5-mini@sn` 提供
- 异常解释：`llm.summarize` 不切换，选中的 Provider 失败时返回它的真实错误（Provider、精确模型、上游状态与原文），不改写成“没有可用模型”；没有接入满足要求的模型时才返回无候选，并指向用途页

---

## 9.7 移动端处理

用途页、策略组页和添加 Provider 的用途覆盖步骤都要在手机宽度下好用：

- 用途表在窄屏下变成每个用途一张卡片，“当前模型 / 来源 / 不可用时 / 状态”竖排
- 编辑面板使用全屏页或底部抽屉
- 视角切换用顶部分段控件或下拉
- 路由细节默认只展示概要与只读信息，复杂配置折叠分组

---

## 10. 数据模型建议

> 本节按 AICC 工程结构重写。这是产品视角的抽象，**字段名以 `src/frame/aicc/src/model_types.rs` 与 `model_session.rs` 为准**。前端通过 `src/frame/desktop/src/api` 适配层映射，前后端两套模型属正常分工——但前端的**概念**必须覆盖这里的全部对象，否则对应 UI 不会被设计出来。

## 10.1 ProviderInventory（取代旧 ProviderConfig）

Provider 是实例，模型来自 inventory。

```ts
type ProviderInventory = {
  provider_instance_name: string                 // 精确模型名的 @ 后缀
  provider_type: "local_inference" | "cloud_api" | "proxy_unknown"
  provider_driver: string                        // openai / claude / gemini / minimax / fal / sn ...
  provider_origin: "system_config" | "user_config" | "builtin" | "provider_claimed"
  version?: string
  inventory_revision?: string
  models: ModelMetadata[]
}
```

---

## 10.2 ModelMetadata（取代旧 Model，富对象）

```ts
type ModelMetadata = {
  provider_model_id: string
  exact_model: string                            // "claude-opus-4.7@anthropic"
  parameter_scale?: string
  api_types: ApiType[]                           // "llm" | "embedding.text" | "image.txt2img" | ...
  logical_mounts: string[]                       // 挂到哪些 L2 逻辑挂点，如 ["llm.opus"]
  capabilities: {
    streaming: boolean
    tool_call: boolean
    json_schema: boolean
    web_search: boolean
    vision: boolean
    max_context_tokens?: number
    max_output_tokens?: number
  }
  attributes: {
    local: boolean
    privacy: "local" | "cloud" | "private_safe" | "public_cloud" | "unknown"
    quality_score?: number
    latency_class: "fast" | "normal" | "slow" | "unknown"
    cost_class: "low" | "medium" | "high" | "unknown"
    // tier(flagship/mid/nano) 见逻辑目录文档，UI 可用于分档展示
  }
  pricing: {
    input_token_usd?: number
    output_token_usd?: number
    cache_input_token_usd?: number
  }
  health: {
    status: "available" | "degraded" | "unavailable"
    p50_latency_ms?: number
    p95_latency_ms?: number
    error_rate_5m?: number
    quota_state: "normal" | "near_limit" | "exhausted" | "unknown"
  }
}
```

---

## 10.3 ProviderAccountStatus（余额 / 计费）

```ts
type ProviderAccountStatus = {
  provider_instance_name: string
  usage_supported: boolean
  cost_supported: boolean
  balance_supported: boolean
  pricing_mode: "per_token" | "subscription" | "free_quota" | "unknown"   // 决定余额展示逻辑
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

  provider_instance_name: string
  exact_model: string                            // 实际执行的精确型号
  requested_model: string                        // 调用方请求的（多为 L3 逻辑路径）
  api_type: ApiType                              // 取代旧 category 四桶

  tenant_id: string
  app_id?: string
  agent_id?: string
  session_id?: string

  tokens_in?: number
  tokens_out?: number
  token_equivalent?: number

  estimated_cost?: number
  status: "success" | "failed"
  fallback_from?: string                         // 被“不可用时”策略替换掉的原精确模型；用量页可按此筛选
}
```

---

## 10.5 Routing 配置 / LogicalNode / 用途配置（取代旧 LogicalModelConfig）

管理员的路由配置存在系统配置 `services/aicc/settings` 的 `routing` 字段（原名 `session_config`）；用户层、策略组和 App 绑定存在 AICC 的数据库；会话绑定存在 OpenDAN 的会话配置（`route_group`）。

```ts
type AiccRouting = {                             // settings.routing
  logical_tree: Record<string, LogicalNode>      // path -> node，默认路由的展开结构
  global_exact_model_weights: Record<string, number>
  provider_weights: Record<string, number>
  routing_commands: RoutingCommand[]
  policy: PolicyConfig
  usecases: Record<string, UsecaseRoute>         // 管理员层（Zone 默认）
}

type LogicalNode = {
  children: Record<string, LogicalNode>
  items?: Record<string, { target: string; weight: number }>   // target 可为逻辑路径或精确型号
  item_overrides?: Record<string, { target?: string; weight?: number }>
  exact_model_weights: Record<string, number>
  policy?: PolicyConfig                          // 字段可被 { value, locked } 锁定
}

type PolicyConfig = {
  profile?: SchedulerProfile                     // latency_first | quality_first | balanced（默认）| local_first | strict_local
  local_only?: boolean
  allowed_provider_instances?: string[]
  blocked_provider_instances?: string[]
  max_estimated_cost?: Money
}

type UsecaseRoute = {                            // 管理员层、用户层、策略组共用
  model?: string | { value: string; locked: boolean }          // 精确模型 model@provider；locked 只在管理员层有效
  on_unavailable?: OnUnavailable | { value: OnUnavailable; locked: boolean }
  enabled?: boolean                              // 只用于 llm.fallback，只能写在管理员层
}

type OnUnavailable = "none" | "same_model" | "not_pricier" | "next_model"

type RoutingGroup = {
  group_id: string
  name: string
  usecases: Record<string, UsecaseRoute>
  version: number
  app_ids: string[]                              // 已绑定的 App / Agent
}

type UsecaseRow = {                              // routing.usecases 的一行
  usecase: string
  model?: string
  model_source?: string                          // default | zone | user | group:<id> | fallback_llm
  on_unavailable: OnUnavailable
  on_unavailable_source: string
  model_locked: boolean
  on_unavailable_locked: boolean
  fallback_to?: string                           // 当前模型的 Provider 不可用时会换到谁
  coverage: "covered" | "fallback_llm" | "uncovered"
  coverage_reason?: "no_model_for_api_type" | "requirements_unmet"
}
```

字段以 `src/kernel/buckyos-api/src/aicc_routing.rs` 为准。

---

## 10.6 RouteTrace（新增，路由可观测）

```ts
type RouteTrace = {
  request_id: string
  session_id?: string
  api_type: ApiType
  requested_model: string
  requested_model_type: "exact" | "logical"
  resolved_logical_path?: string
  selected_exact_model?: string
  selected_provider_instance_name?: string
  ranked_candidates: Array<{
    exact_model: string
    final_score?: number
    score_inputs?: {
      cost: number
      quality: number
      preference: number
      cache: number
      local: number
    }
    selected: boolean
  }>
  filtered_candidates: Array<{ exact_model: string; reason: string }>
  usecase?: {                                    // 经用途表路由时
    usecase: string
    usecase_source: string                       // default | zone | user | group:<id>
    on_unavailable: OnUnavailable
    on_unavailable_source: string
    app_group?: string
    session_group?: string
  }
  fallback?: {                                   // 答复不是来自用途自己的模型
    kind: "coverage" | "unavailable"             // 由兜底 LLM 提供 / 按“不可用时”策略切换
    from: string                                 // 用途（coverage）或失败的精确模型（unavailable）
    to: string                                   // llm.fallback 或实际使用的精确模型
    policy?: OnUnavailable
  }
  scheduler_profile: SchedulerProfile
  user_summary?: {                               // 面向普通用户的人话版
    display_name: string
    model_family: string
    provider_origin: "cloud" | "local" | "proxy_unknown"
    reason_short: string
    was_fallback: boolean                        // 由兜底 LLM 提供
    was_failover: boolean                        // 按“不可用时”策略切换
  }
  warnings: string[]
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
- 用途页访问率与修改率、策略组创建率
- 未覆盖用途数（按 Zone）、兜底 LLM 打开率
- Provider 自动同步开启率

---

## 11.2 运行时指标

- Provider 可用率
- “不可用时”策略切换次数、标记故障的时长
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
│   ├── Providers / Add / Step 4（用途覆盖）
│   ├── Providers / Add / Step 5
│   ├── Providers / Detail
│   ├── Models / Empty
│   ├── Models / List
│   ├── Routing / Usecases
│   ├── Routing / Groups
│   └── Routing / Details
│
└── 02 Mobile
    ├── Home / Disabled
    ├── Home / Enabled
    ├── Providers / List
    ├── Providers / Add / Step 1
    ├── Providers / Add / Step 2
    ├── Providers / Add / Step 3
    ├── Providers / Add / Step 4（用途覆盖）
    ├── Providers / Add / Step 5
    ├── Providers / Detail
    ├── Models / Empty
    ├── Models / List
    ├── Routing / Usecases
    ├── Routing / Groups
    └── Routing / Details
```

---

## 12.3 Frame 命名规范

建议统一使用：

- `AI Center / Desktop / Home / Enabled`
- `AI Center / Desktop / Providers / Wizard / Step 1`
- `AI Center / Mobile / Providers / Detail`
- `AI Center / Mobile / Routing / Usecases`

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

#### Step 4：Usecase Coverage
三组列表：新覆盖的用途、默认模型会变的用途（旧 → 新，带勾选框）、仍未覆盖的用途（带原因）。

#### Step 5：Review & Create
摘要卡（含用途覆盖摘要）+ 操作按钮。

---

## E. Provider Detail

### 桌面布局建议

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Header: Provider Name                              [Edit] [Refresh]  │
├──────────────────────────────────────────────────────────────────────┤
│ [Connection Status] [Auth Status] [Balance/Credit] [Model Count]    │
│ [Marked Faulty toggle + 说明]                                       │
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

## G. Routing（按 §9 重做：用途页 / 策略组 / 路由细节）

### 桌面布局建议

用途页：
- 顶部：视角切换（我 / 策略组 / App；管理员多“Zone 默认”）
- 主体：用途表（用途 / 当前模型 / 来源 / 不可用时 → 会换到谁 / 状态），锁定的格子显示锁
- 兜底 LLM 单独一行，管理员可开关
- 点一行打开右侧编辑面板：选模型（按家族分组、标价格）、选“不可用时”（四个选项各附解释并实时显示“会换到谁”）、恢复为跟随下一层

策略组页：
- 左侧组列表，右侧复用用途表编辑组内容，下方“已绑定的 App / Agent”与绑定按钮

路由细节页（高级）：
- 左侧：**三层目录树**可展开控件（L3 → L2 → L1），非平铺表格
- 右侧（选中某 L3 角色路径时）：
  - Scheduler Profile 单选
  - 候选 items + **weight**（滑杆/数值，不是拖拽排序）
  - Policy：local_only / allowed·blocked instances / max_estimated_cost
  - `locked` 字段置灰并解释；写操作只对管理员显示
- 底部/抽屉：Route Trace Explorer（ranked / filtered candidates、来源层、兜底 / 策略切换）

---

## 12.9 页面布局：移动版

## A. 导航方式

### 推荐方式
- 顶部 App Bar
- 主要页面采用分段或底部导航
- `用途` 页可直接进入；策略组与路由细节作为“更多 / 高级”入口，避免与 Home / Providers / Models 抢主导航优先级
- 用途表在窄屏下变成每个用途一张卡片，“当前模型 / 来源 / 不可用时 / 状态”竖排

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
用途覆盖：每个用途一张卡片，旧模型和新模型竖排，变化项带勾选框

#### Step 5
确认摘要 + 创建按钮


## 关键交互指导

### Stepper
- 桌面：顶部水平 Stepper
- 移动：顶部简化 Stepper + “步骤 X / 5”

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
2. Providers / Add Wizard（Step 1~5）
3. Home / Enabled（默认首页）
4. Providers / List + Detail
5. Models / Empty + List
6. Routing / Usecases
7. Routing / Groups + Details

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
- Step 4：检查用途覆盖
- Step 5：确认并创建

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

## 13.5 用途与故障
- “不可用时”选项：不切换 / 同一个模型换 Provider / 换下一个不更贵的模型 / 换下一个模型
- “不更贵”说明：系统按已知价格判断，价格未知的模型不会被选为“不更贵”
- 算不出备选：没有已知价格不高于它的模型 / 没有别的模型
- 未覆盖：没有接入能做这件事的模型 / 有模型但不满足用途要求
- 兜底 LLM 开关：打开后，没有配置模型的 LLM 用途由兜底模型提供，使用它的会话会显示警告
- 会话警告：规划没有配置模型，正在使用兜底模型 X
- 标记为故障：路由不变；配置了不可用策略的用途直接改用备选，其余直接报错；长期故障请停用 Provider

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

6. **Routing 交互需整体重做（2026-05-29 新增）**
   - 旧"逻辑名 + 扁平候选 + 拖拽优先级"模型作废，改为三层目录树 + weight + profile（§9）
   - 2026-11-27 再改：用户入口改为用途表（用途页、策略组页），目录树退到路由细节；fallback mode 已删除，改为每个用途的“不可用时”策略
   - api 适配层（`src/frame/desktop/src/api`）需新增 `routing.usecases` / `routing.usecase.set` / `routing.groups.*` / `routing.bindings.*` 与 RouteTrace 查询的映射

7. **Models 与 Providers 的边界（2026-05-29 新增）**
   - 二者是同一份 inventory 的两个视图（按 `provider_type`），不是独立数据。前端需共享一份 inventory 状态，避免两套缓存不一致

8. **新增可观测面 Route Trace 的数据量**
   - 每次调用一条 trace，需确认采样/保留策略，避免首页拉全量 trace；建议只展示最近 N 条 + 按 session 钻取

---

## 16. 里程碑建议

## Phase 1：可用首版
- 首启启用页
- Add Provider Wizard
- Home / Usage（基础用量 + Credit）
- Providers 列表与详情
- Models 空状态与跳转
- 用途页（查看对自己生效的用途表）

## Phase 2：增强版
- Provider 余额与费用展示增强
- App / Agent / Session 深钻
- 模型自动同步能力完整化
- 用途页编辑、路由策略组、路由细节
- 更细粒度的错误解释

## Phase 3：高级版
- 多 Key / 多账号策略
- 并行多模型与高级路由
- 用户策略（可见性、价格系数、5h / 24h / 7d limit）、Credit 价目表、余额与充值（见 [AICC 用户策略与 Credit 计费 TODO](../../notepads/aicc-user-policy-credit-todo.md) §6，取代 [Token Quota PRD](AI_Center_Quota_PRD.md) 的额度与可见性页面）
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
