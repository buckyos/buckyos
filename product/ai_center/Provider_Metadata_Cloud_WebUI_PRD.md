# Provider Metadata Cloud WebUI PRD

- 产品名称：Provider Metadata Cloud WebUI
- 文档类型：PRD（产品需求文档）
- 状态：Draft
- 适用版本：beta 2.2 breaking change
- 关联设计：`doc/aicc/provider-driver-cloud-update-design.md`
- 适用端：桌面 Web 优先，移动端只支持浏览和紧急禁用

---

## 1. 背景

Provider Driver Metadata 云更新体系包含两类云服务：

- **A 服务 `provider-metadata-tech-service`**：维护 provider、model、pattern、defaults、variants、version_rules、api_type、capability、逻辑目录等技术事实与 key 字段。
- **B 服务 `provider-metadata-ops-service`**：从 A 服务同步技术发布结果，在其基础上叠加运营 overlay，最终生成可下发给客户端的完整发布 JSON。

说明：A 服务 / B 服务只是本文档为了讨论架构职责使用的简称，不能作为 WebUI 的直接展示文案。用户界面应使用“技术参数”“运营参数”“技术源”“同步源”“发布源 revision”“运营 revision”等名称。

WebUI 的目标是让管理员能够安全地浏览、编辑、预览、发布 provider-driver metadata，并让每次变更都有快照、diff、影响范围、测试建议和审计日志。

本文档只描述云服务管理端 WebUI，不描述客户端三方合并逻辑和公开 GET API 的具体实现。

---

## 2. 产品定位

Provider Metadata Cloud WebUI 是 AICC provider-driver metadata 的云端管理控制台。

核心定位：

1. **默认只读，显式编辑**：所有增删改都必须进入编辑模式，并绑定一次 `edit_session`。
2. **技术/运营职责分离**：技术参数管理技术事实，运营参数管理运营 overlay。运营参数视图不允许修改技术字段。
3. **发布前可解释**：发布前必须展示 diff、影响范围、风险字段、污染字段 warning 和生成的测试建议。
4. **高密度专业工具**：技术参数视图面向工程师，允许 JSON/schema、规则、批量选择、模式匹配等专业操作。
5. **低代码运营工具**：运营参数视图面向运营管理员，主要通过表单、开关、批量调整和预设策略完成配置。
6. **国际化内建**：UI 必须支持语言切换，首版至少支持简体中文和英文。

---

## 3. 目标用户

### 3.1 技术参数管理员：工程师

画像：

- 理解 provider、endpoint、protocol family、model id、JSON、schema、pattern、capability 等概念。
- 能阅读 diff、JSON schema 校验错误和发布 JSON。
- 可能使用 AI 或脚本生成导入计划。

核心诉求：

- 快速新增或修正 provider/model 技术元数据。
- 批量维护模型能力、api_type、logical mounts 和 model nick。
- 构造 OpenRouter 等聚合 provider 的模型选择规则。
- 在发布前确认 key 字段变更、规则命中范围和客户端影响。

设计原则：

- 信息密度高，列表和详情尽量并排。
- 提供 JSON 视图、schema 校验、规则命中预览、批量操作。
- 对 key 字段设置显式解锁和二次确认，不隐藏技术细节。

### 3.2 运营参数管理员：运营管理员

画像：

- 不要求编程知识。
- 理解价格、推荐级别、路由权重、上下架、灰度、展示优先级等运营概念。
- 通常基于技术参数已经发布的 provider/model 做调整。

核心诉求：

- 配置技术源 URL 并查看同步状态。
- 在不改技术字段的前提下，禁用 provider/model，调整价格、推荐级别、routing weight、展示优先级。
- 预览最终下发给客户端的 JSON 和发布影响。
- 看懂异常、过期、污染字段 warning，并知道是否可以继续发布。

设计原则：

- 避免暴露 JSON 编辑器作为主路径。
- 用表单、开关、数字输入、分段控件、批量编辑抽屉完成配置。
- 技术字段可浏览但只读，并用视觉分组说明其来自技术源。

---

## 4. 产品目标

### 4.1 MVP 目标

1. 技术参数视图和运营参数视图都提供浏览模式和编辑模式，默认进入浏览模式。
2. 进入编辑模式时创建 `edit_session`，顶部固定显示 base revision、操作者、编辑状态和未发布变更数量。
3. 技术参数视图支持 provider、model、model rules、model nicks、defaults fallback、patterns/variants/version_rules、logical directory、api_type、capability 的浏览、搜索、详情和编辑。
4. 技术参数视图支持以已有 provider/model 为模板创建新对象。
5. 技术参数视图支持导入 YAML/Markdown 更新计划，导入后进入待提交状态，不直接发布。
6. 运营参数视图支持配置技术源 URL、手动同步、查看发布源 revision/运营 revision、查看 stale 状态。
7. 运营参数首版运营字段固定为 disabled、pricing override、routing weight、recommendation level、display priority，并支持 provider/model/pattern/block 的运营 overlay 编辑和批量调整。
8. 发布前必须提供 diff、影响范围、风险确认、测试建议和发布摘要。
9. 发布后写入 change log，并可从变更历史查看 diff、导入文档、操作者、发布时间、from/to revision。
10. UI 支持简体中文和英文切换，语言选择持久化到当前账号偏好。
11. 支持保存未完成草稿；用户下次登录时提示其工作区存在未完成草稿，并允许逐个继续编辑、发布或放弃。

### 4.2 非目标

1. 不在运营参数视图提供技术字段编辑能力。
2. 不提供拖拽式模型路由图编辑器。
3. 不提供客户端本地合并冲突处理 UI。
4. 不把公开 GET API 的调用统计做成完整监控平台；只在首页展示关键健康状态。
5. 不在首版支持多人实时协同编辑。同一 revision 上的并发编辑通过提交前 revision 冲突处理。

---

## 5. 信息架构

### 5.1 全局布局

桌面端采用三栏工作台：

```text
┌──────────────────────────────────────────────────────────────┐
│ 顶部栏：服务标识 / revision / 同步状态 / 语言 / 账号 / 模式   │
├───────────────┬──────────────────────────────┬───────────────┤
│ 左侧导航       │ 主工作区                      │ 右侧详情/检查器 │
│ - Dashboard   │ 列表、树、规则表、预览、diff    │ 字段、JSON、日志 │
│ - Providers   │                              │               │
│ - Models      │                              │               │
│ - Rules       │                              │               │
│ - Publish     │                              │               │
│ - Logs        │                              │               │
└───────────────┴──────────────────────────────┴───────────────┘
```

布局要求：

- 顶部栏固定，编辑模式下显示明显的编辑状态条。
- 左侧导航宽度固定，支持折叠为图标。
- 主工作区承载列表、树、批量操作、diff 和发布流程。
- 右侧详情面板可收起，选中对象后展示字段详情、引用关系、JSON 预览和风险提示。
- PC 端编辑模式优先使用高密度分页表格，适用于 provider、model、selection rule、nick rule、pattern、variant、version_rule、change log 等数量较多的对象；表格必须支持搜索、筛选、排序、分页和批量选择。
- 浏览模式可以使用卡片或卡片+列表混合布局，以兼顾移动端阅读和概览；但 PC 编辑主路径不得只依赖卡片。
- 移动端采用单栏：导航抽屉 + 列表页 + 详情页 + 底部操作栏；移动端只支持浏览和紧急禁用，不支持发布、复杂批量编辑和 JSON 编辑。

### 5.2 技术参数导航

| 一级模块 | 页面 | 说明 |
|---|---|---|
| Dashboard | 概览 | 发布状态、schema version、provider/model 数量、最近变更、待处理 warning。 |
| Providers | Provider 列表 / 详情 | 原厂 provider、聚合 provider、compatible proxy 的技术主数据管理。 |
| Models | Model 列表 / 详情 | 全局默认 model meta 和 provider 专属 model meta。 |
| Selection Rules | 模型选择规则 | provider 白名单、黑名单、include/exclude origin、include/exclude pattern。 |
| Nick Rules | Model Nick | 精确 nick 和 pattern rewrite，支持批量前缀、后缀、替换。 |
| Metadata Blocks | Defaults / Patterns / Variants / Version Rules | 按全局和 provider scope 管理 resolver 规则块。 |
| Logical Directory | 逻辑目录 | 目录树、模型挂载、批量添加/移除、目录属性。 |
| Dictionaries | API Types / Capabilities | 受控字典管理和批量标记模型能力。 |
| Import Plan | 导入计划 | YAML/Markdown 导入、解析、命中预览、分发为待提交变更。 |
| Publish | 预览与发布 | Diff、影响范围、风险字段、测试建议、二次确认、发布。 |
| Change Logs | 变更历史 | 审计日志、diff、导入附件、发布 revision。 |

### 5.3 运营参数导航

| 一级模块 | 页面 | 说明 |
|---|---|---|
| Dashboard | 概览 | 发布源 revision、运营 revision、同步状态、stale 提示、发布状态、warning 数。 |
| 技术源 | 技术源 | 技术参数服务 URL、最近同步时间、手动刷新、同步结果。 |
| Providers | Provider 运营 | 浏览技术字段，只编辑禁用、推荐级别、展示优先级、运营备注。 |
| Models | Model 运营 | 浏览技术字段，只编辑价格覆盖、routing weight、推荐级别、禁用、灰度。 |
| Blocks | Pattern / Block Overlay | 对 pattern/defaults/variants/version_rules 做禁用或运营字段覆盖。 |
| Bulk Operations | 批量调整 | 按 provider、api_type、capability、价格区间、名称搜索批量调整运营参数。 |
| Warnings | Warning 中心 | 污染字段、同步失败、合并失败、stale 数据、跳过对象。 |
| Publish Preview | 下发预览 | 最终发布 JSON、对象数量、移除项、overlay 命中、客户端可见效果。 |
| Change Logs | 变更历史 | 审计日志、diff、发布 revision。 |

### 5.4 前端模块划分与代码组织

首版 WebUI 先做 mock-first 独立云服务前端，不接真实后端。代码必须放在独立目录，建议为 `src/frame/provider_metadata_cloud/web/`，不注册到 desktop app registry，不复用 desktop 内部业务状态，不向 `src/frame/desktop/src/app/ai-center` 写入 provider metadata cloud 代码。可以复制或抽取 desktop 的通用视觉模式、Tailwind 配置、i18n provider、Shell/Sidebar/MobileTabBar 组织方式，但不得让该云服务依赖 desktop app 的业务模块。

推荐顶层结构：

```text
src/frame/provider_metadata_cloud/web/
  package.json
  index.html
  vite.config.ts
  tailwind.config.js
  playwright.config.ts
  src/
    main.tsx
    App.tsx
    routes.tsx
    runtime/
      env.ts
      permissions.ts
    i18n/
      provider.tsx
      dictionaries.ts
    theme/
      provider.tsx
      tokens.ts
    mock/
      driverMetadataSeed.ts
      providerCloudSeed.ts
      api.ts
      latency.ts
    datamodel/
      types.ts
      schemas.ts
      selectors.ts
      diff.ts
    state/
      ProviderMetadataStore.tsx
      useProviderMetadataStore.ts
    layout/
      CloudConsoleShell.tsx
      TopBar.tsx
      Sidebar.tsx
      MobileNav.tsx
      InspectorPanel.tsx
      navigation.ts
    pages/
      dashboard/
      tech-source/
      providers/
      models/
      selection-rules/
      nick-rules/
      metadata-blocks/
      logical-directory/
      dictionaries/
      import-plan/
      publish/
      warnings/
      bulk-operations/
      change-logs/
    workflows/
      edit-session/
      publish-wizard/
      import-plan-wizard/
      provider-wizard/
      bulk-operation-wizard/
    components/
      data-table/
      detail-panel/
      json-viewer/
      diff-viewer/
      forms/
      status/
      empty-state/
      confirmation/
    tests/
      e2e/
        pages/
        flows/
```

入口与框架约束：

- `App.tsx` 只负责 provider、theme、i18n、store 和路由装配；页面分发由 `routes.tsx` 或独立 `PageRouter` 完成，参考 desktop 的 `AppPanel + Shell + PageRouter` 模式。
- `layout/` 只放云服务控制台通用布局：顶部栏、左右栏、移动导航、详情检查器和导航定义；不得包含具体 provider/model 业务表单。
- 技术参数视图和运营参数视图可以共用同一个前端包，内部通过 `serviceRole: "A" | "B"` 切换导航、权限和页面可编辑字段；该字段不得直接展示到 UI。技术字段与运营字段行为由 datamodel/schema/permissions 统一约束。
- 所有页面默认可在 mock 数据下独立渲染；真实 API 未实现前，任何页面不得直接发起真实网络请求。

页面模块边界：

- 每个左侧导航项必须对应 `pages/<module>/` 下的独立页面模块，至少包含 `index.tsx` 和本页面私有子组件；跨页面复用后才移动到 `components/`。
- 技术参数导航页分别落到 `dashboard/`、`providers/`、`models/`、`selection-rules/`、`nick-rules/`、`metadata-blocks/`、`logical-directory/`、`dictionaries/`、`import-plan/`、`publish/`、`change-logs/`。
- 运营参数导航页分别落到 `dashboard/`、`tech-source/`、`providers/`、`models/`、`metadata-blocks/` 或 `blocks/`、`bulk-operations/`、`warnings/`、`publish/`、`change-logs/`。
- 同名页面可以共享页面骨架，但技术字段编辑、运营 overlay 编辑、只读字段展示必须拆成独立组件，避免通过大量条件分支混在同一组件内。

向导与工作流模块：

- 所有向导必须放在 `workflows/<name>/`，不得塞进页面文件。首版至少拆出 `provider-wizard`、`import-plan-wizard`、`publish-wizard`、`bulk-operation-wizard`。
- 每个向导目录至少包含 `WizardShell.tsx`、`steps.ts`、每一步独立 `Step*.tsx`、`schema.ts` 和 `types.ts`；表单使用 `react-hook-form + zod`，schema 是 UI 输入约束的单一事实来源。
- 发布预览、diff、风险确认、stale 确认、key 字段解锁确认属于 `publish-wizard`，由技术参数视图/运营参数视图注入不同检查项。
- 导入计划向导只把 YAML/Markdown action 解析为 mock edit session 中的待提交变更，不直接发布。

共享组件边界：

- `components/data-table/`：分页表、列配置、筛选条、批量选择栏。
- `components/detail-panel/`：字段分组、只读/可编辑字段、引用关系、右侧检查器片段。
- `components/json-viewer/`：发布 JSON、对象 JSON、schema error 定位。
- `components/diff-viewer/`：字段 diff、影响范围、risk section、测试建议。
- `components/forms/`：受控输入、开关、数字步进、分段控件、字段级解锁控件。
- `components/status/`：revision badge、sync status、warning badge、字段所有权 badge、edit session badge。
- `components/empty-state/` 和 `components/confirmation/`：空态、错误态、确认弹窗。

Mock 数据与状态层：

- `mock/driverMetadataSeed.ts` 从 `src/frame/aicc/driver_metadata/*.json` 的字段形态构造内置 provider/model/pattern/defaults/variants/version_rules 样本；不直接 import 运行时后端代码。
- `mock/providerCloudSeed.ts` 在 driver metadata 样本上补齐云服务需要的发布源 revision、运营 revision、provider key、edit session、change log、warnings、ops overlay、stale cache 等 UI 数据。
- `mock/api.ts` 模拟异步读取、保存草稿、预览发布、发布成功/失败、同步技术源、导入计划解析等行为，并覆盖正常、空、加载、错误、stale、schema warning、污染字段 warning 状态。
- `state/` 只封装前端 store 和 selector，不包含组件渲染；后续真实后端接入时只替换 mock api provider，不改页面模块边界。

测试组织：

- Playwright 测试放在该独立 WebUI 包自己的 `tests/e2e/` 下。
- `tests/e2e/pages/` 按导航页面覆盖基础渲染、空态、错误态和移动端只读行为。
- `tests/e2e/flows/` 覆盖技术参数新增 OpenRouter provider、技术参数批量标记 capability、运营参数配置技术源、运营参数批量调整运营价格、发布预览与确认。
- 验证命令随包独立运行，不能要求启动 desktop 或真实后端。

---

## 6. 通用交互模式

### 6.1 浏览模式

浏览模式是默认状态。

可执行操作：

- 搜索、筛选、排序、查看详情。
- 查看发布 JSON 预览。
- 查看引用关系、规则命中结果、warning、change log。
- 导出当前对象 JSON 或 diff。

不可执行操作：

- 新增、删除、修改字段。
- 发布。
- 导入计划应用到待提交状态。

### 6.2 编辑模式

进入编辑模式流程：

1. 用户点击「进入编辑」。
2. 系统展示当前 published revision 和风险提示。
3. 用户确认后，服务保存当前发布状态快照并创建 `edit_session`。
4. 顶部栏进入编辑状态，显示 base revision、操作者、session id、未发布变更数。

编辑模式规则：

- 普通字段可按服务所有权编辑。
- key 字段默认只读，即使已进入编辑模式也不能直接改。
- key 字段包括 `provider.name`、`provider.base_url`、`model.id`、`original_provider`、`nick`、pattern/block nick 以及稳定 key。
- 修改 key 字段必须点击字段级「解锁 key 字段」，填写原因，并在发布确认页单独确认影响。
- 运营参数视图中的技术字段永远只读，不提供解锁入口。

退出编辑模式：

- 「保存草稿」：保留 edit session，不发布。
- 「放弃变更」：丢弃 edit session。
- 「预览发布」：进入 diff 和影响分析。
- 「发布」：完成二次确认后写 change log 并生成新 revision。

草稿续编辑：

- 保存草稿后，`edit_session` 保持 `editing` 状态并持久保存当前待提交变更。
- 用户下次登录时，如果存在未完成草稿，Dashboard 顶部显示草稿工作区提示。
- 草稿列表按服务、base revision、更新时间、操作者、变更数量展示。
- 用户可以逐个选择「继续编辑」「进入发布预览」或「放弃草稿」。
- 如果草稿的 base revision 已落后于当前 published revision，继续编辑前必须先重新预览并处理 revision 冲突。

### 6.3 发布前检查

发布前页面必须包含：

- 变更摘要：新增、修改、删除、禁用对象数量。
- 影响范围：provider 数、model 数、规则块数、逻辑目录影响、api_type/capability 影响。
- key 字段风险区：列出所有 key 字段变更、影响样例、需要确认的复选框。
- 运营污染字段区：列出被丢弃的技术字段和来源。
- Schema 校验结果：Error 阻止发布，Warning 允许发布但必须展示。
- 规则命中样例：批量规则至少展示命中数量和前 N 条样例。
- patterns/variants/version_rules 的 nick 重写样例：展示重写前 selector、重写后 selector、命中模型和冲突检查结果；defaults 展示 fallback object 的发布结果。
- 生成的测试建议：按变更类型生成需要回归的 provider/model/routing 场景。
- 发布说明输入框：必填，写入 change log summary。

发布按钮启用条件：

- 没有 Error 级校验问题。
- 所有 key 字段风险项已确认。
- patterns/variants/version_rules 不存在无法重写、重写后冲突或空命中的 Error；defaults schema 校验失败是 Error。
- 发布说明已填写。
- 当前 base revision 未被其他发布覆盖；如已覆盖，必须重新打开编辑会话或执行 rebase 预览。

### 6.4 导入计划

导入入口支持：

- 上传 `.yaml`、`.yml`、`.md`。
- 粘贴文本。
- 从 URL 拉取文本。

导入流程：

1. 解析 `schema_version`、`kind`、`target_service`、`base_revision`、`actions`。
2. 校验 `target_service` 是否与当前服务一致。
3. 校验 action 是否允许当前服务执行。
4. 为每条 action 生成可视化预览和命中结果。
5. 用户选择全部应用或部分应用到当前 edit session。
6. 系统把 action 分发到对应页面的待提交变更，不直接发布。

技术参数导入强调 schema、selector 和 key 字段检查。运营参数导入强调运营字段白名单，不展示复杂 JSON patch 作为主路径。

---

## 7. 技术参数页面需求

### 7.1 Dashboard

核心组件：

- 发布状态卡片：published revision、schema version、最近发布时间、最近操作者。
- 数据规模卡片：providers、models、rules、metadata blocks、logical directories。
- 风险卡片：key 字段待确认数、schema warning、未发布 edit session。
- 最近变更列表：最近 10 条 change log。
- 快捷入口：新增 provider、导入计划、进入发布预览。

### 7.2 Providers

列表字段：

- Provider key
- Name
- Provider driver
- Protocol family
- Base URL
- Provider kind
- Enabled
- Model 数量
- 最近更新时间

筛选：

- provider kind
- provider driver
- protocol family
- enabled 状态
- 是否聚合 provider

详情页分区：

- 基础字段：provider key、name、driver、base_url、kind、protocol_family。
- 模型选择摘要：include/exclude rules、命中模型数、黑名单模型数。
- 专属规则：provider scope 下的 patterns/variants/version_rules，以及 defaults fallback 覆盖。
- JSON 预览：合成前 provider 技术数据。
- 引用关系：关联 models、nick rules、logical mounts。

主要操作：

- 新增原厂 provider。
- 新增聚合 provider。
- 从现有 provider 复制。
- 编辑普通字段。
- 字段级解锁 key 字段。
- 禁用 provider 技术发布。
- 查看该 provider 的合成结果预览。

### 7.3 Models

列表字段：

- Model key
- Model ID
- Original provider
- Provider scope
- Model driver
- API types
- Capabilities 摘要
- Context limits
- Pricing source
- Exclude

详情页分区：

- 身份字段：model_id、original_provider、provider_key、model_driver。
- 能力字段：api_types、logical_mounts、capabilities、context_limits。
- 价格字段：参考 pricing。
- Provider 专属覆盖：与全局默认 meta 的 diff。
- 发布预览：最终进入 provider 发布 JSON 的样例。

主要操作：

- 新增全局默认 model meta。
- 新增 provider 专属 model meta。
- 从现有 model 复制。
- 批量标记 api_type。
- 批量标记 capability。
- 批量设置 logical mounts。
- 删除 provider 专属覆盖。

新增或删除全局 model meta 前，必须显示受影响 provider 列表。

### 7.4 Selection Rules

页面形态：

- 左侧 provider 选择器。
- 中间规则表。
- 右侧命中预览。

规则类型：

- `allow`
- `deny`
- `include_origin`
- `exclude_origin`
- `include_pattern`
- `exclude_pattern`

交互要求：

- 新增规则时实时显示命中数量。
- 调整优先级时预览最终命中结果。
- 删除规则前显示将新增或移除的模型样例。
- 黑名单命中的模型在技术参数发布中保留但设置 `exclude=true`。

### 7.5 Nick Rules

支持能力：

- 精确 nick。
- pattern nick。
- 批量加前缀。
- 批量加后缀。
- 替换片段。
- pattern rewrite。

预览要求：

- 展示 `source_model_id -> published id` 映射。
- 显示重复 nick、冲突 nick、空结果。
- 显示受影响 logical directory、patterns/variants/version_rules selector、defaults fallback 覆盖和 provider 发布样例。

### 7.6 Metadata Blocks

对象：

- defaults
- patterns
- variants
- version_rules

页面要求：

- 按 global scope 和 provider scope 切换。
- Metadata Blocks 页面必须先按类型分组浏览。`defaults`、`patterns`、`variants`、`version_rules` 使用不同的列表列、筛选项、详情面板、创建表单和编辑表单；不能用同一个宽松 JSON 编辑器作为主路径覆盖所有类型。
- 创建 Metadata Block 时选择类型，创建后类型冻结，不允许在编辑中改成其他类型。
- 每种 Metadata Block 类型必须使用独立输入 schema 校验；schema error 需要能定位到对应字段。
- `defaults` 保持当前 driver metadata 的非数组 object 形式，是匹配失败或未收录模型统一使用的保底参数；UI 提供单个 defaults object 的结构化编辑和 JSON 辅助预览。
- `patterns`、`variants`、`version_rules` 是数组；每个数组元素必须作为独立记录展示、选择和编辑，PC 编辑模式使用分页表格承载，不允许主路径只展示整段数组 JSON。
- `patterns`、`variants`、`version_rules` 列表必须展示类型、scope、source、nick/名称、selector、priority、enabled、更新时间、命中数量。
- `patterns`、`variants`、`version_rules` 详情提供结构化表单和 JSON 视图，结构化表单必须包含该元素的身份字段、selector、priority、enabled 和 content/patch；JSON 视图用于辅助检查，不作为主编辑路径。
- 支持从全局/原厂记录引用创建 provider 专属记录；管理员修改字段后，该记录成为 provider 专属配置。
- `model_id_selector` 输入使用原始 `model.id`，保存和发布预览必须展示重写后的 published model id selector。
- variants 必须显式配置 base model 匹配 selector，不能只靠 variant name 隐式关联模型。
- 保存前执行 JSON/schema 校验。
- 支持复制、禁用、删除、回滚到上一发布版本。
- 命中预览必须同时展示原始命中模型、`source_model_id -> published id` nick 映射、重写后的 pattern/variant/version_rule 发布样例，以及重复 nick、空命中、重写冲突；defaults 预览展示其作为 fallback object 的最终发布值。

### 7.7 Logical Directory

页面布局：

- 最上方是筛选检索区域，支持筛选目录，也支持筛选目录下包含的模型。
- 筛选检索区域下方展示面包屑，用于表示当前按目录路径浏览时的目录路径。
- 左侧展示目录结构和目录属性。
- 中间展示匹配到的项目列表，包括目录项和模型项。
- 右侧展示选中项目详情；选中目录时展示目录详情，选中模型时展示模型详情。
- “筛选检索模式”和“按目录路径展示模式”互斥。进入筛选检索模式时，主列表展示搜索结果并明确提示当前不再按单一路径浏览；进入目录路径浏览时，清空或挂起搜索条件。

能力：

- 新增、删除、重命名、移动子目录。
- 修改目录属性。
- 批量添加模型到目录。
- 批量移除模型。
- 一个模型可挂到多个目录。
- 目录 key/path 重复、空目录、断链引用必须给出 warning 或阻止提交。
- 删除或移动目录前显示受影响模型数和路径样例。

### 7.8 Dictionaries

API Types：

- 维护 api_type 字典。
- 支持新增。
- 删除和重命名是高风险操作，必须显示引用模型数量和样例。
- 支持把筛选结果批量标记为支持某个 api_type。
- api_type 应用到 model、rule 或批量操作时，必须通过下拉框、combobox、单选/多选或等价选择控件严格选择已有字典项，不允许通过自由文本提交不存在的 key。

Capabilities：

- 维护 capability 字典、类型、默认展示信息。
- 支持新增。
- 删除和重命名是高风险操作，必须显示引用模型数量和样例。
- 支持把筛选结果批量标记为支持某个 capability。
- capability 应用到 model、rule 或批量操作时，必须通过下拉框、combobox、单选/多选或等价选择控件严格选择已有字典项，不允许通过自由文本提交不存在的 key。
- 大多数 capability/api_type 属性按 bool 语义表达为支持/不支持，使用开关、复选框或批量勾选；少数值属性字段必须提供结构化输入、单位、范围和 schema 校验。
- 提供 model × api_type/capability 的矩阵或等价批量核对视图，方便工程师发现漏标、误标和异常组合。

---

## 8. 运营参数页面需求

### 8.1 Dashboard

核心组件：

- 发布源 revision / 运营 revision 对照。
- 技术源同步状态：正常、同步中、失败、stale。
- 最近一次成功同步时间。
- 当前 overlay 数量。
- 禁用 provider/model 数量。
- 合并 warning 数量。
- 最近发布记录。

### 8.2 技术源

字段：

- 技术参数服务 URL。
- 最近成功同步的发布源 revision。
- 最近同步时间。
- 同步错误信息。
- 缓存状态。

操作：

- 修改技术参数服务 URL。
- 测试连接。
- 手动刷新。
- 查看最近拉取摘要。

交互要求：

- 修改 URL 后不自动发布，只更新当前 edit session。
- 手动刷新失败时保留上一版可用技术源缓存，并在顶部显示 stale。
- 如果发布源 revision 变化，运营参数视图应提示当前 overlay 将基于新的技术源数据重新预览。

### 8.3 Providers 运营

列表字段：

- Provider name
- Provider driver
- Provider kind
- 技术启用状态
- 运营禁用状态
- 推荐级别
- 展示优先级
- Model 总数
- 已禁用 model 数
- Warning

详情页分区：

- A 技术字段：只读。
- B 运营字段：可编辑。
- Overlay 预览：展示 B 对该 provider 的覆盖。
- 客户端可见结果：该 provider 是否会下发。

可编辑字段：

- disabled
- recommendation level
- display priority
- routing policy tag
- ops note

### 8.4 Models 运营

列表字段：

- Published model id
- Source model id
- Provider
- Original provider
- API types
- Capability 摘要
- A reference pricing
- B price override
- Routing weight
- Recommendation level
- 运营禁用状态

筛选：

- provider
- original provider
- api_type
- capability
- 是否禁用
- 是否有价格覆盖
- 推荐级别
- warning

详情页分区：

- A 技术字段：只读。
- 运营价格：input/output price、currency、unit、source。
- Routing：weight、cost_class、latency_class、quality_score。
- 推荐展示：推荐级别、展示优先级、灰度状态。
- 发布预览：最终下发 model JSON。

可编辑字段：

- disabled
- pricing override
- routing weight
- cost_class
- latency_class
- quality_score
- recommendation level
- display priority
- rollout strategy
- ops note

### 8.5 Bulk Operations

批量选择条件：

- provider
- original provider
- model id 模糊匹配
- api_type
- capability
- 当前推荐级别
- 当前价格区间
- 当前 routing weight 区间

批量操作：

- 启用/禁用。
- 设置推荐级别。
- 设置展示优先级。
- 按百分比调整价格。
- 直接设置 input/output price。
- 设置 routing weight。
- 清除运营价格覆盖。

确认页必须展示：

- 命中数量。
- 前 N 条样例。
- 修改前后字段对比。
- 是否影响客户端下发。

### 8.6 Warnings

Warning 类型：

- A 同步失败。
- A 数据 stale。
- A JSON schema 校验 warning。
- 运营 overlay 包含技术字段污染。
- 单个 provider/model 合并失败并被跳过。
- 价格字段格式异常。
- routing weight 超出建议范围。

交互：

- Warning 按严重程度、对象类型、发布时间筛选。
- 点击 warning 跳转到对应对象。
- 污染字段 warning 展示被丢弃字段，不允许一键保留。

### 8.7 Publish Preview

内容：

- 最终发布 JSON 预览。
- provider/model 下发数量。
- B 禁用导致移除的 provider/model。
- overlay 命中统计。
- 技术字段污染丢弃统计。
- stale 状态提示。

操作：

- 导出发布 JSON。
- 导出 diff。
- 进入发布确认。

---

## 9. 多语言需求

### 9.1 支持范围

首版必须支持：

- 简体中文：`zh-CN`
- 英文：`en-US`

后续语言扩展不得改业务字段 schema。

### 9.2 语言切换入口

入口位置：

- 顶部栏右侧语言菜单。
- 登录页语言菜单。

行为：

- 用户切换语言后当前页面即时刷新文案，不丢失编辑状态。
- 语言偏好保存到账号设置。
- 未登录时保存到浏览器本地偏好。
- 如果账号偏好和浏览器偏好冲突，登录后以账号偏好为准。

### 9.3 国际化内容范围

必须国际化：

- 导航、按钮、表单标签、表格列名。
- 状态、错误、warning、确认弹窗。
- 发布流程中的风险说明和测试建议标题。
- 空状态、加载状态。
- 帮助文本和字段说明。

不翻译：

- provider key、model id、api_type、capability key、logical mount、revision、JSON 字段名。
- 导入计划正文中的用户内容。
- change log 中管理员填写的 summary。

### 9.4 文案原则

中文：

- 面向技术参数管理员可使用技术术语，如 provider、model、schema、revision、capability。
- 面向运营参数管理员优先使用业务词，如「禁用」「推荐级别」「运营价格」「同步状态」。

英文：

- 保持字段名和 API 术语稳定，例如 Provider、Model、Revision、Overlay、Published JSON。
- 运营参数视图避免使用 Patch、Schema、Resolver 等非必要技术词作为主路径文案。

---

## 10. 权限与安全

权限要求：

- 访问管理端需要账号登录。
- 更新配置需要 `metadata.update` 授权。
- 发布需要二次确认。
- Change log 只追加，不允许修改历史记录。

建议权限拆分：

| 权限 | 技术参数 | 运营参数 | 说明 |
|---|---|---|---|
| `metadata.read` | 是 | 是 | 浏览数据、发布 JSON、change log。 |
| `metadata.update` | 是 | 是 | 创建 edit session、编辑草稿。 |
| `metadata.publish` | 是 | 是 | 发布新 revision。 |
| `metadata.key_field.update` | 是 | 否 | 解锁并修改 key 字段；不需要独立审批流。 |
| `metadata.tech_source.update` | 否 | 是 | 修改技术源 URL。 |

安全要求：

- 发布确认必须重新校验当前用户权限。
- 长时间未操作的 edit session 应提示刷新或重新登录。
- 已保存草稿的 edit session 在用户重新登录后应主动提示，不因登录会话结束而丢失。
- 技术字段/运营字段所有权必须由服务端校验，不能只依赖前端禁用控件。
- 运营 overlay 中出现技术字段时，发布阶段丢弃污染字段并记录 warning。

---

## 11. 状态与错误

### 11.1 Edit Session 状态

| 状态 | UI 表现 | 可执行操作 |
|---|---|---|
| editing | 顶部编辑条；可作为未完成草稿恢复 | 保存草稿、预览、放弃。 |
| previewed | 发布预览完成 | 返回编辑、发布确认、导出 diff。 |
| approved | 已确认待发布 | 发布、返回预览。 |
| published | 已发布 | 查看 change log。 |
| discarded | 已放弃 | 只读查看摘要。 |

### 11.2 Revision 冲突

当用户基于旧 revision 编辑，但期间已有新 revision 发布：

- 顶部显示冲突提示。
- 禁用直接发布。
- 用户可选择放弃当前 session，或执行重新预览。
- 重新预览必须重新计算 diff 和影响范围。

### 11.3 Stale 状态

运营参数视图无法拉取技术源最新数据但有上一版缓存时：

- Dashboard 显示 stale。
- Provider/Model 列表仍可浏览上一版 A 数据。
- 发布确认页必须显示 stale 风险。
- Stale 状态下允许运营参数发布，但发布确认页必须要求管理员确认使用的发布源 revision。
- Change log 必须记录发布时使用的发布源 revision 和 stale 状态。

---

## 12. 关键流程

### 12.1 技术参数新增 OpenRouter 聚合 Provider

1. 工程师进入技术参数 Providers。
2. 点击「新增聚合 Provider」。
3. 填写 name、provider_driver、base_url、protocol_family、provider_kind。
4. 进入 Selection Rules，按 original provider 或 model pattern 选择模型。
5. 进入 Nick Rules，设置 `openai/` 等前缀或 pattern rewrite。
6. 查看合成预览，确认 `source_model_id` 和发布 id。
7. 进入发布预览，检查 diff、命中数量、key 字段风险。
8. 填写发布说明并发布。

### 12.2 技术参数批量标记 Capability

1. 工程师进入 Models。
2. 使用 provider、api_type、model id 搜索筛选模型。
3. 点击批量操作「添加 capability」。
4. 选择 capability 并预览命中样例。
5. 应用到 edit session。
6. 发布前查看受影响模型数和测试建议。

### 12.3 运营参数调整推荐级别和价格

1. 运营管理员进入运营参数 Models。
2. 按 provider 和 capability 筛选目标模型。
3. 进入 Bulk Operations。
4. 设置推荐级别和价格调整规则。
5. 确认命中数量、样例、修改前后对比。
6. 进入 Publish Preview 查看最终客户端可见 JSON。
7. 填写发布说明并发布。

### 12.4 运营参数禁用异常 Provider

1. 运营管理员进入 Providers。
2. 找到异常 provider。
3. 打开详情，切换 disabled。
4. 系统显示该 provider 下将不再下发的 model 数量。
5. 进入发布预览。
6. 发布后运营 revision 更新，客户端下一次拉取时移除该 provider。

### 12.5 运营参数配置技术源

1. 运营管理员进入技术源。
2. 输入技术参数服务 URL。
3. 点击测试连接。
4. 测试成功后点击手动刷新。
5. 系统显示发布源 revision 和同步摘要。
6. 如有 overlay 受影响，提示进入 Publish Preview 重新确认。

---

## 13. 数据展示约定

### 13.1 字段分组

技术参数字段分组：

- Identity：key、name、model_id、original_provider。
- Driver：provider_driver、model_driver、protocol_family。
- Capability：api_types、capabilities、context_limits。
- Rules：selection rules、patterns、defaults、variants、version_rules。
- Publish：enabled、exclude、revision、updated_at。

运营参数字段分组：

- 技术源：来自技术参数服务的只读字段。
- Ops Overlay：disabled、pricing override、routing、recommendation、display priority。
- Publish Result：最终是否下发、下发 JSON 预览、warning。

### 13.2 颜色与状态

状态必须同时使用文字和视觉，不得只靠颜色表达。

建议状态：

- Published：已发布 / Published
- Editing：编辑中 / Editing
- Stale：数据过期 / Stale
- Warning：警告 / Warning
- Blocked：阻止发布 / Blocked
- Disabled by A：技术禁用 / Disabled by A
- Disabled by B：运营禁用 / Disabled by B

---

## 14. 验收标准

### 14.1 通用验收

1. 用户进入任一服务后台时默认是浏览模式。
2. 未进入编辑模式时，所有写操作按钮不可用或不出现。
3. 进入编辑模式后，顶部显示 base revision、操作者、编辑状态。
4. 发布前必须看到 diff、影响范围、schema 校验和发布说明输入框。
5. 发布后 change log 可查看 from revision、to revision、operator、summary、diff 附件。
6. 简体中文和英文切换后，导航、按钮、表单、状态、错误提示完成切换。
7. 切换语言不丢失当前筛选、选中对象和 edit session。

### 14.2 技术参数验收

1. 可以新增 provider，并在发布预览中看到 provider 发布 JSON。
2. 可以新增全局 model meta，并在发布前看到受影响 provider 列表。
3. 可以为 provider 增加 include/exclude 规则，并预览命中模型。
4. 可以批量设置 nick，并看到 `source_model_id -> published id` 映射。
5. 可以编辑 defaults fallback object，并可以编辑 patterns/variants/version_rules 的数组元素，执行 schema 校验、命中预览和 nick 重写预览。
6. 每个 provider 可以为 patterns/variants/version_rules 各自配置多条记录，并能从全局/原厂记录引用后修改为 provider 专属配置；defaults 只保留单个 provider fallback object。
7. 可以批量为模型添加 api_type 或 capability。
8. 修改 key 字段必须字段级解锁，并在发布确认页进入独立风险区。

### 14.3 运营参数验收

1. 可以配置技术参数服务 URL、测试连接、手动刷新。
2. Dashboard 展示发布源 revision 和运营 revision。
3. 技术字段在运营参数视图中只读，无法通过 UI 解锁。
4. 可以禁用 provider/model，并在发布预览中看到最终不下发。
5. 可以批量调整运营价格、routing weight 和推荐级别。
6. overlay 中出现技术字段时，发布预览显示污染字段 warning。
7. 可以导出最终发布 JSON。

---

## 15. 已确认约束与风险

已确认约束：

- 运营参数首版运营字段最小集合固定为 disabled、pricing override、routing weight、recommendation level、display priority。
- 移动端只允许浏览和紧急禁用，不允许发布。
- `metadata.key_field.update` 不需要独立审批流，只需要字段级解锁和发布确认页风险确认。
- Stale 状态下允许运营参数发布，发布记录必须写入使用的发布源 revision 和 stale 状态。
- 长时间更新支持保存草稿；用户下次登录时应提示未完成草稿，并允许逐个继续编辑、发布或放弃。
- patterns/variants/version_rules 与 models 一样按 provider scope 管理；通常引用全局/原厂配置，修改字段后成为 provider 专属配置，并且每个 provider 每类对象都允许多条记录。
- patterns/variants/version_rules 中的 model selector 使用原始 `model.id` 编辑，但发布预览和最终 JSON 必须按 model nick 改写为客户端可见 id。
- defaults 保持非数组 object 形态，作为匹配失败或未收录模型统一使用的保底参数；首版 UI 必须提供结构化编辑，不再按数组式多记录模型设计。
- Metadata Block 类型创建后冻结。即使后端或 mock 数据统一存储，UI 也必须按类型提供不同视图和不同 schema；如果统一存储导致校验、查询或编辑复杂度过高，允许实现阶段拆分为多个表或多个数据集合。
- api_type、capability、目录、provider、model 等受控字段在应用时必须严格匹配已有字典或对象集合，主路径不得依赖自由文本输入。

主要风险：

- 技术字段/运营字段所有权如果只在前端限制，长期会产生污染字段，必须服务端强校验。
- key 字段修改会影响客户端三方合并、endpoint 匹配、model id/nick trace，需要在 diff 中始终单独展示。
- patterns/variants/version_rules 如果没有随 model nick 同步重写，会造成发布 JSON 中模型规则失效；发布确认页必须展示重写链路和冲突检查结果。defaults 不参与数组式 selector 重写，但必须展示 fallback object 的最终发布值。
- Metadata Block 类型差异如果只用一个宽松 JSON 表单承载，会隐藏非法字段、漏校验和误发布风险；必须保持按类型 schema 校验和按类型编辑体验。
- api_type/capability 如果允许自由文本录入，容易产生拼写错误和不可诊断的能力标记漂移；必须通过受控选择和引用检查减少输入错误。
- 运营参数管理员不具备编程知识，若主路径暴露过多 JSON/schema 细节，会显著增加误操作概率。
- 多语言切换若翻译 provider/model key，会破坏技术对象识别；所有机器 key 必须保持原文。
