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

- **A 服务 `provider-metadata-tech-service`**：维护 provider、model_param_rules、variants、version_rules、api_type、capability、逻辑目录等技术事实与 key 字段。
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
3. 技术参数视图支持 provider、model_param_rules、model nicks、variants/version_rules、logical directory、api_type、capability 的浏览、搜索、详情和编辑。
4. 技术参数视图支持以已有 provider/model 为模板创建新对象。
5. 技术参数视图支持导入 YAML/Markdown 更新计划，导入后进入待提交状态，不直接发布。
6. 运营参数视图支持配置技术源 URL、手动同步、查看发布源 revision/运营 revision、查看 stale 状态。
7. 运营参数首版运营字段固定为 disabled、pricing override、routing weight、recommendation level、display priority。provider 与 model/model_param_rule 的运营字段分别在 Providers、Models 运营页面维护；variants/version_rules 的运营 overlay 在 Rule Overlay 页面维护；批量调整入口面向 model/model_param_rule 运营字段。
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
- PC 端编辑模式优先使用高密度分页表格，适用于 provider、model、nick rule、pattern、variant、version_rule、change log 等数量较多的对象；表格必须支持搜索、筛选、排序、分页和批量选择。
- 浏览模式可以使用卡片或卡片+列表混合布局，以兼顾移动端阅读和概览；但 PC 编辑主路径不得只依赖卡片。
- 移动端采用单栏：导航抽屉 + 列表页 + 详情页 + 底部操作栏；移动端只支持浏览和紧急禁用，不支持发布、复杂批量编辑和 JSON 编辑。

### 5.2 技术参数导航

| 一级模块 | 页面 | 说明 |
|---|---|---|
| Dashboard | 概览 | 发布状态、schema version、provider/model 数量、最近变更、待处理 warning。 |
| Providers | Provider 列表 / 详情 | 原厂 provider、聚合 provider 的技术主数据管理。 |
| Models | Model 参数规则 | 全局/原厂和 provider 专属 `model_param_rules`，统一管理 exact、pattern、default。 |
| Nick Rules | Model Nick | 精确 nick 和 pattern rewrite，支持批量前缀、后缀、替换。 |
| Resolver Rules | Variants / Version Rules | 按全局和 provider scope 管理 variants 和 version rules。 |
| Logical Directory | 逻辑目录 | 目录树、已挂载对象浏览、目录属性。 |
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
| Rule Overlay | Variants / Version Rules Overlay | 对 variants、version_rules 做禁用或运营字段覆盖；model_param_rule 的运营字段由 Models 运营页面维护。 |
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
      nick-rules/
      resolver-rules/
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
- 技术参数导航页分别落到 `dashboard/`、`providers/`、`models/`、`nick-rules/`、`resolver-rules/`、`logical-directory/`、`dictionaries/`、`import-plan/`、`publish/`、`change-logs/`。
- 运营参数导航页分别落到 `dashboard/`、`tech-source/`、`providers/`、`models/`、`resolver-rules/`、`bulk-operations/`、`warnings/`、`publish/`、`change-logs/`。
- 同名页面可以共享页面骨架，但技术字段编辑、运营 overlay 编辑、只读字段展示必须拆成独立组件，避免通过大量条件分支混在同一组件内。

向导与工作流模块：

- 所有向导必须放在 `workflows/<name>/`，不得塞进页面文件。首版至少拆出 `provider-wizard`、`import-plan-wizard`、`publish-wizard`、`bulk-operation-wizard`。
- 每个向导目录至少包含 `WizardShell.tsx`、`steps.ts`、每一步独立 `Step*.tsx`、`schema.ts` 和 `types.ts`；表单使用 `react-hook-form + zod`，schema 是 UI 输入约束的单一事实来源。
- Provider Wizard 用于创建原厂 provider 或聚合 provider。Models、Variants/Version rules 步骤从已有源对象建立白名单引用；Nick rewrite 步骤维护多条 source-to-published 重写规则；模型参数步骤编辑或生成 exact/pattern/default `model_param_rules`，其中 `model_driver`、api_types、capabilities 属于模型规则字段。Add Provider 向导的 Logical mounts 独立步骤只批量编辑 models/patterns/defaults 的 `logical_mounts` 和 version rules 的 `auto_mounts`；variants 在该步骤暂不配置。
- Provider Wizard 可以提供批量默认值，但必须允许逐模型或逐规则复核。选择原厂对象、非原厂模板或完全新建对象后，管理员都可以编辑生成的 provider 专属对象。
- 发布预览、diff、风险确认、stale 确认、key 字段解锁确认属于 `publish-wizard`，由技术参数视图/运营参数视图注入不同检查项。
- 导入计划向导只把 YAML/Markdown action 解析为 mock edit session 中的待提交变更，不直接发布。

共享组件边界：

- `components/data-table/`：分页表、列配置、筛选条、批量选择栏。
- `components/detail-panel/`：字段分组、只读/可编辑字段、引用关系、右侧检查器片段。
- `components/json-viewer/`：发布 JSON、对象 JSON、schema error 定位；长文本必须提供复制、下载导出和清晰的滚动/折叠视图。
- `components/diff-viewer/`：字段 diff、影响范围、risk section、测试建议。
- `components/forms/`：受控输入、开关、数字步进、分段控件、字段级解锁控件。
- `components/status/`：revision badge、sync status、warning badge、字段所有权 badge、edit session badge。
- `components/empty-state/` 和 `components/confirmation/`：空态、错误态、确认弹窗。

Mock 数据与状态层：

- `mock/driverMetadataSeed.ts` 从 `src/frame/aicc/driver_metadata/*.json` 的字段形态构造内置 provider、model_param_rules、metadata_variants、metadata_version_rules 等表格化样本；不直接 import 运行时后端代码。
- `mock/providerCloudSeed.ts` 在 driver metadata 样本上补齐云服务需要的发布源 revision、运营 revision、provider key、edit session、change log、warnings、ops overlay、stale cache 等持久化数据。mock seed 必须按文档中的表格/数据集合组织，不能保存只迎合 Web 页面布局的最终运算结果。
- `mock/api.ts` 模拟异步读取、保存草稿、预览发布、发布成功/失败、同步技术源、导入计划解析等行为，并覆盖正常、空、加载、错误、stale、schema warning、污染字段 warning 状态。页面需要的计数、命中数量、发布样例、风险摘要、目录树、搜索结果和最终 JSON 片段必须通过 `mock/api.ts`、selector 或等价 mock 模块接口从表格化数据计算得到；这组接口后续应能演进为真实后端 API。
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
- 复制或导出当前对象 JSON、发布 JSON、diff、导入计划解析结果。

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
- key 字段包括 `provider.name`、`provider.base_url`、`model.id`、`original_provider`、`nick`、model_param_rule/variant/version_rule nick。`provider_key`、`nick_key`、`variant_key`、`version_rule_key` 由服务端自动编号，不提供字段级修改。
- 修改其它 key 性质属性必须在发布确认页单独确认影响；自动编号 key 不提供字段级修改。
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
- model_param_rules 的命中样例：展示 exact、pattern、default 的命中顺序、命中模型和冲突检查结果；variants 展示重写前后 `model_id_selector`，version_rules 展示重写前后 `content.model_pattern`，并展示命中模型和冲突检查结果。
- 生成的测试建议：按变更类型生成需要回归的 provider/model/routing 场景。
- 发布说明输入框：必填，写入 change log summary。

发布按钮启用条件：

- 没有 Error 级校验问题。
- 所有 key 字段风险项已确认。
- model_param_rules 不存在冲突命中、无 default 兜底或 schema 校验失败的 Error；variants 的 `model_id_selector` 和 version_rules 的 `content.model_pattern` 不存在无法重写、重写后冲突或空命中的 Error。
- 发布说明已填写。
- 当前 base revision 未被其他发布覆盖；如已覆盖，必须重新打开编辑会话或执行 rebase 预览。

### 6.4 导入计划

导入入口支持：

- 上传 `.yaml`、`.yml`、`.md`。
- 粘贴文本。

导入流程：

1. 解析 `schema_version`、`kind`、`target_service`、`base_revision`、`actions`。
2. 校验 `target_service` 是否与当前服务一致。
3. 校验 action 是否允许当前服务执行。
4. 为每条 action 生成可视化预览和命中结果。
5. 用户选择全部应用或部分应用到当前 edit session。
6. 系统把 action 分发到对应页面的待提交变更，不直接发布。

技术参数导入强调 schema、selector 和 key 字段检查。运营参数导入强调允许运营覆盖的字段集合，不展示复杂 JSON patch 作为主路径。

---

## 7. 技术参数页面需求

### 7.1 Dashboard

核心组件：

- 发布状态卡片：published revision、schema version、最近发布时间、最近操作者。
- 数据规模卡片：providers、models、rules、resolver rules、logical directories。
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
- 模型选择摘要：按 models、patterns、defaults、variants、version rules 分类的白名单源对象数量，以及 `model_param_rules.exclude=true` 的发布排除数量。
- 专属规则：provider scope 下的 model_param_rules、variants/version_rules。
- JSON 预览：合成前 provider 技术数据。
- 引用关系：关联 models、nick rules、logical mounts。

主要操作：

- 新增原厂 provider。
- 新增聚合 provider。
- 从现有 provider 复制。
- 编辑普通字段。
- 查看自动编号 key 和 key 性质属性的只读风险摘要。
- 禁用 provider 技术发布。
- 查看该 provider 的合成结果预览。

### 7.3 Models

列表字段：

- Model key
- Match type：exact / pattern / default
- Model ID
- Original provider
- Provider scope
- Model driver
- Priority：仅 pattern 使用
- API types
- Capabilities 摘要
- Context limits
- Pricing source
- Exclude：仅 exact/pattern 使用

详情页分区：

- 身份字段：model_id、original_provider、provider_key、model_driver。
- 能力字段：api_types、logical_mounts、capabilities、context_limits。
- 价格字段：参考 pricing。
- Provider 专属覆盖：与全局默认 meta 的 diff。
- 发布预览：最终进入 provider 发布 JSON 的样例。

主要操作：

- 新增全局/原厂 exact、pattern、default `model_param_rule`。
- 新增 provider 专属 exact、pattern、default `model_param_rule`。
- 从现有 exact、pattern 或 default 复制创建新的目标类型规则。
- 为 provider 添加模型时，也可以从已存在的 exact/pattern/default 规则复用参数，再补充目标类型要求的字段；该操作创建新的 `model_param_rule`，不修改来源规则。来源类型和目标类型不一致时，必须提示用户确认类型将变更，并展示 selector、priority 等会被改写或清空的字段。
- 为 exact/pattern 设置 `exclude=true`；该规则作为发布排除规则保留，其它参数字段仍可保存，便于取消 exclude 后恢复。default 不能设置 exclude。
- 批量标记 api_type。
- 批量标记 capability。
- 批量设置 logical mounts。
- 删除 provider 专属覆盖。

新增或删除全局/原厂 `model_param_rule` 前，必须显示受影响 provider 列表。`models` 页面是 exact、pattern、default 的唯一主编辑入口；发布 JSON 仍按 `models[]`、`patterns[]`、`defaults` 物化展示。

### 7.4 Nick Rules

支持能力：

- 精确 nick。
- pattern nick。
- 按 original provider 维护多条 nick 规则，并按 priority 预览最终命中。
- 批量加前缀。
- 批量加后缀。
- 替换片段。
- pattern rewrite。
- 聚合 provider 可以用多条 Nick rewrite 规则表达不同上游前缀，例如 OpenRouter 可将 OpenAI 源模型发布为 `openai/{model}`，Claude 源模型发布为 `anthropic/{model}`，Gemini 源模型发布为 `google/{model}`。

预览要求：

- 展示 `source selector -> published selector` 映射；exact model 使用 `model.id`，pattern 使用 wildcard，variant 无 selector 时使用 `*`，version rule 使用 `content.model_pattern`。
- 显示重复 nick、冲突 nick、空结果。
- 显示受影响 logical directory、model_param_rules、variants/version_rules selector 和 provider 发布样例。

### 7.5 Resolver Rules

对象：

- variants
- version_rules

页面要求：

- 按 global scope 和 provider scope 切换。
- Resolver Rules 页面只管理 `variants` 和 `version_rules`。`model_param_rules` 的 exact、pattern、default 统一在 Models 页面管理。
- `variants` 和 `version_rules` 使用独立表或独立数据集合。每种类型使用不同的列表列、筛选项、详情面板、创建表单和编辑表单；不能用同一个宽松 JSON 编辑器作为主路径覆盖所有类型。
- 每种 resolver rule 类型必须使用独立输入 schema 校验；schema error 需要能定位到对应字段。
- `variants`、`version_rules` 是数组；每个数组元素必须作为独立记录展示、选择和编辑，PC 编辑模式使用分页表格承载，不允许主路径只展示整段数组 JSON。
- `variants`、`version_rules` 列表必须展示类型、scope、source、nick/名称、selector、priority、enabled、更新时间、命中数量。
- `variants`、`version_rules` 详情提供结构化表单和 JSON 视图，结构化表单必须包含该元素的身份字段、selector、priority、enabled 和 content/patch；JSON 视图用于辅助检查，不作为主编辑路径。
- 支持从全局/原厂记录引用创建 provider 专属记录；管理员修改字段后，该记录成为 provider 专属配置。
- variants 的 `model_id_selector` 输入使用原始 `model.id` 或 wildcard pattern，保存和发布预览必须展示重写后的 published model selector。
- version_rules 的通配字段是 `content.model_pattern`，输入使用原始 `model.id` 或 wildcard pattern，保存和发布预览必须展示重写后的 published model pattern。列表筛选可保留 `model_id_selector` 作为 `content.model_pattern` 的镜像字段，但它不是 version rule 的主通配字段。
- variants 可配置 base model 匹配 selector；未配置时使用默认 selector `*`，并与显式 selector 一样参与 nick rewrite，`*` 本身也可以被重写。
- 保存前执行 JSON/schema 校验。
- 支持复制、禁用、删除、回滚到上一发布版本。
- 命中预览必须同时展示原始命中模型、`source selector -> published selector` nick 映射、重写后的 variant `model_id_selector` 和 version rule `content.model_pattern` 发布样例，以及重复 nick、空命中、重写冲突。
- Provider 全量 JSON 视图和导出必须按 `models`、`patterns`、`defaults` 分开展示；该预览可在 Models、Publish 或右侧检查器中提供，但不作为 Resolver Rules 的主编辑对象。

### 7.6 Logical Directory

页面布局：

- 最上方是筛选检索区域，支持筛选目录，也支持筛选目录下包含的模型。
- 筛选检索区域下方展示面包屑，用于表示当前按目录路径浏览时的目录路径。
- 左侧展示目录结构和目录属性。目录树应从 `model_param_rules.logical_mounts`、variants/version_rules 自动挂载项和显式 logical directory 记录共同物化，不能只固定展示 LLM/Image/Audio 等少数顶层目录。
- 中间展示匹配到的项目列表，包括目录项和模型项。
- 右侧展示选中项目详情；选中目录时展示目录身份、来源、路径和引用关系，选中模型时展示模型详情。
- “筛选检索模式”和“按目录路径展示模式”互斥。进入筛选检索模式时，主列表展示搜索结果并明确提示当前不再按单一路径浏览；进入目录路径浏览时，清空或挂起搜索条件。
- 面包屑的每一级路径可点击；根路径与子路径拼接不得产生双斜杠。按路径浏览时，中间列表展示当前目录的全部直属子目录和模型。

能力：

- 新增、删除、重命名、移动子目录。
- 修改目录属性。
- 已挂载模型列表必须支持搜索、分页或虚拟列表，不得使用高度很小的原生多选框作为大量模型的主操作控件。
- 一个模型可挂到多个目录。
- 目录 key/path 重复、空目录、断链引用必须给出 warning 或阻止提交。
- 删除或移动目录前显示受影响模型数和路径样例。

### 7.7 Dictionaries

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
3. 填写 Name、base_url、protocol_family、provider_kind；系统由 `Name.toLowerCase()` 构造 `provider_driver`。
4. 在 Models、Variants/Version rules 中选择已有源对象，形成 provider 白名单引用。
5. 在 Model params、Variant/version params 中配置或创建 provider 专属对象，并在 Logical mounts 中配置 models、patterns、defaults 和 version rules 的批量挂载目录。
6. 进入 Nick Rules，按 original provider 维护多条 nick rewrite，例如 OpenAI、Claude、Gemini 使用不同 published id 前缀。
7. 查看合成预览，确认 `source_model_id`、发布 id 和模型参数规则命中。
8. 进入发布预览，检查 diff、命中数量、key 字段风险。
9. 填写发布说明并发布。

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
- Rules：model_param_rules、variants、version_rules、nick rules。
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
2. 可以新增全局/原厂 exact model_param_rule，并在发布前看到受影响 provider 列表。
3. 可以为 provider 增加 include 规则，并可以通过 exclude 操作批量生成 exact/pattern `model_param_rule.exclude=true`，同时预览命中模型和发布结果。
4. 可以批量设置 nick，并看到 `source selector -> published selector` 映射。
5. 可以编辑 model_param_rules 的 exact、pattern、default 规则，并可以编辑 variants/version_rules 的数组元素，执行 schema 校验、命中预览和 nick 重写预览。
6. 每个 provider 可以为 model_param_rules 配置多条 exact/pattern 记录和单条 default 记录，并能从全局/原厂记录引用后修改为 provider 专属配置；variants/version_rules 各自允许多条记录。
7. 可以为 exact/pattern model_param_rule 设置 `exclude=true`，并保留其它参数字段以便取消 exclude 后恢复。
8. 可以批量为模型添加 api_type 或 capability。
9. 修改 key 字段必须字段级解锁，并在发布确认页进入独立风险区。

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
- model_param_rules 按 provider scope 管理；通常引用全局/原厂配置，修改字段后成为 provider 专属配置。exact 精确匹配 `model.id`，pattern 按 selector 和 priority 顺序匹配，default 每个 scope 最多一条。
- model_param_rules 中的 selector 使用原始 `model.id` 编辑，但发布预览和最终 JSON 必须按 model nick 改写为客户端可见 id；default 不需要 selector。
- default 是 `model_param_rules` 中不带 selector 和 priority 的兜底规则，参数内容按非数组结构化表单编辑，用于 exact 和 pattern 全部匹配失败后的 fallback。
- exact、pattern、default 必须统一存储在 `model_param_rules` 表或数据集合中，通过 `match_type` 区分；它们只在 Models 页面管理。variants、version_rules 单独存储，并在 Resolver Rules 页面管理。UI 必须按类型提供不同视图和不同 schema。
- 最终下发给客户端的 JSON 必须是 AICC driver metadata document，包含 `schema_version`、`provider_driver`、`name`、`protocol_family`、`base_url`、`revision`、`models`、`patterns`、`defaults`、`variants`、`version_rules`、`signature`。不能因为后端统一存储而把 exact/pattern/default 合并下发；发布确认页必须展示这些字段的物化结果和匹配优先级。
- `name`、`provider_driver`、`protocol_family` 和 `base_url` 是客户端可消费字段：`name` 是 provider 的 UI 展示名，忽略大小写后必须唯一，只允许英文字母、数字、下划线和连词符；`provider_driver` 是该 provider 规则包唯一 id，由 Add Provider 向导用 `Name.toLowerCase()` 构造，并作为下发文件名和 JSON `provider_driver` 字段；`protocol_family` 是客户端连接模型商服务器的 API 协议族；`base_url` 用于区分兼容 provider 和 endpoint 匹配。
- `models`、`patterns`、`defaults` 使用相同 model-meta 字段；模型限长通过 `context_limits` 表达，模型参考价格通过带币种的 `pricing` 表达。provider 运行时能动态返回价格时，客户端以动态价格优先；币种和汇率由客户端统一处理。
- `provider_key`、Nick Rules、Logical Directory、Dictionaries 和运营 overlay 字段是管理/编辑/校验输入；发布预览可以展示它们如何影响最终结果，但客户端 driver metadata JSON 不得原样携带这些中间概念或后台字段。

## 16. Authoring 约束（beta 2.2）

- `provider_key`、`nick_key`、`variant_key` 和 `version_rule_key` 自动编号；创建后的 key 不可修改。Provider 编辑使用与新增相同的向导并回填当前值。
- Template 仅能选一个具体 provider，或不选模板从零创建；Kind 仅为 `origin` / `aggregator`。Basic 页中 `Name` 由用户输入且忽略大小写唯一；`provider_driver` 是 metadata document 内的 driver id，由 `Name.toLowerCase()` 构造；`protocol_family` 是创建 provider 时选择的客户端 wire protocol，并写入最终下发 JSON。
- 向导顺序固定为 `Basic -> Models -> Model params -> Pattern order -> Variants / Version rules -> Variant/version params -> Logical mounts -> Nick rewrite -> Preview`。系统不存在 Selection Rules 页面或持久化规则。
- 参数编辑采用目标选择区加参数编辑区；跨对象只应用明确改动的字段。Models/Patterns/Defaults 的匹配身份唯一且每个 provider 仅允许一个 default。Pattern 顺序是用户可见的 priority 表达。
- Model params 的目标选择区沿用 Models 的 provider / type Tab / selected 三栏布局；新建对象归入当前 provider，空 provider 与空 Tab 不展示。多选字段仅在所有目标值相同时显示，model selector 必须逐项赋值，Model params 不编辑 priority；`original_provider` 全部为当前 provider 时只读显示，否则只能通过按钮设为当前 provider。Apply 提交当前批次并清空选择，Discard 恢复当前批次参数；`max_context_tokens` 只在对应 capability 已选中时显示。
- Variants / Version rules 与 Models 使用相同的白名单选择布局，分别从已有对象选择；Variant/version params 分为两个类型 Tab 和独立参数面板，每个 Tab 的待编辑对象选择方式与 Model params 一致，支持多选、Pending edit list、Discard 和 Apply。type 不可原地变更，本步骤不编辑 priority，也不批量编辑 logical mounts/auto_mounts。Variant 面板编辑 `provider_options`；Version rule 面板编辑完整匹配谓词和参数，`tier_tokens`、`exclude_tier_tokens`、`stability.unstable_tokens` 使用自由文本 token 输入，`current_mount` 和 `version_mount` 通过物化目录树单选。Version rule 的列表、详情和预览展示完整匹配谓词，而不是仅展示 `model_pattern`。
- Logical mounts 使用与 Model params 一致的目标选择区加目录选择区布局，只对 Models、Patterns、Defaults、Version rules 使用完整目录树和三态批量编辑，负责模型规则的 `logical_mounts` 和 version rule 的 `auto_mounts`；Variants 在该步骤暂不配置。右侧 Selected paths 只显示全部目标实选路径，Apply 写入实选/未选路径语义，Discard 丢弃尚未 Apply 的路径选择变更并恢复目录树状态。Version rule 的 `current_mount`/`version_mount` 是参数面板中的单值挂载选择，不属于批量挂载集合。Logical Directory 只管理目录及已挂载对象浏览，暂不提供目录批量添加模型。
- Nick rewrite 是 source-to-published 的映射，支持 origin-prefix 与 exact/pattern mapping，作用于 models、patterns、variants、version rules；exact models 重写 `models[].id`，patterns 重写 `patterns[].pattern`，default 无下发 selector。variant 无 selector 时按 `*` 参与，`*` 本身也可以被重写；version rules 重写 `content.model_pattern`。来源对象变更或删除时必须告警其所有引用 provider。
- Providers、Models、Nick Rules、Resolver Rules、Dictionaries 的浏览详情均只读；只有编辑模式暴露行级编辑、删除操作。Resolver Rules 分 Variants/Version rules 两个 Tab，Nick Rules 和 Resolver Rules 都支持按 provider 检索。
- mock 数据必须按 provider、model_param_rules、metadata_variants、metadata_version_rules 等文档表格组织。页面布局需要的最终展示值必须通过 mock API/selectors 计算，不得在 seed 中直接保存迎合页面布局的预计算结果。
- api_type、capability、目录、provider、model 等受控字段在应用时必须严格匹配已有字典或对象集合，主路径不得依赖自由文本输入。

主要风险：

- 技术字段/运营字段所有权如果只在前端限制，长期会产生污染字段，必须服务端强校验。
- key 字段修改会影响客户端三方合并、endpoint 匹配、model id/nick trace，需要在 diff 中始终单独展示。
- model_param_rules 的 pattern selector、variant selector、version rule `content.model_pattern` 如果没有随 model nick 同步重写，会造成发布 JSON 中模型规则失效；发布确认页必须展示重写链路和冲突检查结果。default 规则不参与 selector 重写，但必须展示最终发布参数值。
- 规则类型差异如果只用一个宽松 JSON 表单承载，会隐藏非法字段、漏校验和误发布风险；必须保持按类型 schema 校验和按类型编辑体验。exact/pattern/default 应统一存储在 `model_param_rules` 并由 Models 页面管理，variants/version_rules 应作为独立对象在 Resolver Rules 页面管理。
- api_type/capability 如果允许自由文本录入，容易产生拼写错误和不可诊断的能力标记漂移；必须通过受控选择和引用检查减少输入错误。
- 运营参数管理员不具备编程知识，若主路径暴露过多 JSON/schema 细节，会显著增加误操作概率。
- 多语言切换若翻译 provider/model key，会破坏技术对象识别；所有机器 key 必须保持原文。

## 17. Origin Identity 产品要求（beta 2.2）

Provider Metadata Cloud WebUI 必须保证同一个物理模型无论由官方 provider 还是聚合平台 provider 提供服务，都能落到同一个逻辑目录身份下。逻辑挂载模板中的 `{driver}` 和 `{model}` 表示模型原厂 provider 和原厂模型名，不表示当前服务渠道的 provider driver 或聚合平台 model id。

OpenRouter 这类聚合平台仍然需要保留其 provider-native model id 用于真实调用和规则命中；例如 OpenRouter 的 `openai/gpt-5.5` 应作为该渠道的 selector 下发，但它的 origin identity 应解析为 `driver=openai`、`model=gpt-5.5`。OpenAI 官方渠道的 `gpt-5.5` 也应解析到相同 origin identity，因此两者最终挂载路径一致。

WebUI 中的 Nick Rules 页面继续保留 Nick rewrite 能力，用于把复用的源模型 selector 构造成 provider-native selector。beta 2.2 在同页按 Tab 拆分为 `Nick Rules`、`Origin mappings`、`Origin provider aliases` 三个编辑区；Origin mappings 必须支持 template/regex 两种模式，并允许编辑 driver/model transforms；Origin provider aliases 必须允许新增、编辑和删除 alias 到 driver 的归一化规则。Mapping preview 是三个 Tab 共享的底部预览，不隶属于任一 Tab，并必须展示每个命中对象的 original provider、model id 和 published id。Origin mappings Tab 右侧必须展示最终将发布的 `origin_mappings` JSON。Add Provider 向导使用相同的 Tab、底部预览和右侧 JSON 预览结构。发布预览必须展示 `schema_version: 2`、`origin_provider_aliases`、`origin_mappings` 以及 models/patterns/version rules 中被物化后的 provider-native selector。

产品暂不支持 dynamic alias。聚合平台返回的软链接类模型由我们自己的逻辑目录系统承担，WebUI 只需通过 exact/pattern 规则的 `exclude=true` 在发布前过滤，不新增单独的 `exclude_patterns` 配置。

验收标准：

1. 同一个物理模型在官方 provider 和 OpenRouter provider 中能够解析出相同的 `{driver}` / `{model}`。
2. 发布预览中可以看到 provider-native selector 和 origin mapping 的对应关系。
3. 下发 JSON 不包含 WebUI 内部 authoring 字段，只包含客户端可消费的 schema v2 字段。
4. dynamic alias 不进入最终 metadata，排除语义通过 `patterns[].exclude=true` 表达。
