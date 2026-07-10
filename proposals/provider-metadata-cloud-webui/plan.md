# Provider Metadata Cloud WebUI 实施计划

状态：Draft

关联文档：

- `doc/aicc/provider-driver-cloud-update-design.md`
- `product/ai_center/Provider_Metadata_Cloud_WebUI_PRD.md`
- `src/frame/aicc/driver_metadata/`
- `harness/SKILLS/webui-prototype/SKILL.md`

## 1. 任务确认

实现 Provider Metadata Cloud WebUI 的 mock-first 前端原型。

这是一个独立的云服务前端。实现阶段不得注册到 desktop，不得把 provider metadata cloud 业务代码写入 `src/frame/desktop/src/app/ai-center`，原型阶段不得依赖真实后端服务。

首版实现使用 `src/frame/aicc/driver_metadata/*.json` 派生 mock 数据。后端服务、公有 GET API、KRPC 协议、持久化 RDB schema 实现，以及客户端 remote update 三方合并逻辑，都不属于本 WebUI 原型计划范围。

## 2. 硬约束

- 目标目录：`src/frame/provider_metadata_cloud/web/`。
- 不修改 desktop app registry。
- 不把业务代码放到 `src/frame/desktop/src/app/ai-center`。
- 后端集成开始前，不发起真实网络 API 请求。
- 引入新依赖前必须先和用户确认。
- 使用 desktop 已有框架和风格方向：
  - React 19
  - TypeScript
  - Vite
  - Tailwind CSS
  - BuckyOS `--cp-*` CSS variables
  - Lucide React icons
  - 必要时使用 React Router DOM
  - React hooks / Context
  - 表单使用 `react-hook-form` + `zod`
  - BuckyOS 自定义 i18n 模式
- 代码必须模块化组织。页面私有组件先留在页面目录，跨页面复用后再移动到共享组件目录。
- PC 端编辑模式优先采用高密度表格。Provider、Model、Rule、Nick、Pattern、Variant、Version Rule 等数量较多的对象必须支持分页、搜索、筛选和批量选择。
- 浏览模式可以采用卡片或卡片+列表的混合方式，以兼顾移动端布局；但进入编辑模式后，PC 主路径仍以表格为主。
- `defaults` 保持当前 driver metadata 的非数组 object 形式，作为匹配失败或未收录模型的统一保底参数。
- `patterns`、`variants`、`version_rules` 仍是数组；每个数组元素应作为独立记录展示、选择和编辑。当前存储和发布仍可使用数组 JSON，但 UI 主路径不能只展示一整段长 JSON。
- Metadata Blocks 即使在 mock 或后端实现中统一落到一个存储表，也必须按 block type 使用不同 schema、不同字段定义、不同列表列配置和不同详情编辑视图。block type 一旦创建后不可修改；如果统一表导致校验、查询或编辑复杂度过高，允许在实现阶段拆成多个表或多个数据集合。
- 文档中使用的 A/B 只是架构讨论简称，UI 上不得直接展示“A 服务”“B 服务”“A/B service role”等字样。用户界面应使用“技术参数”“运营参数”“技术源”“同步源”“发布源 revision”“运营 revision”等面向用户可理解的名称。
- `api_type`、`capability`、目录、provider、model 等受控字段在应用到对象时必须严格匹配已有字典或对象集合。主路径优先使用下拉框、combobox、单选/多选选择器和批量选择控件，避免自由文本输入导致 key 拼写错误。
- 字典项应用时，大多数 capability/api_type 属性按 bool 语义处理为“支持/不支持”；少数值属性字段必须提供带 schema 校验的结构化输入，不得只依赖 JSON 文本。
- 技术侧和运营侧都必须提供可定位、可跳转的 warning/diagnostics 能力。发布前必须能定位 key 字段风险、selector 空命中、nick rewrite 冲突、字典引用影响和逻辑目录破坏性变更。

## 3. 推荐代码组织

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

## 4. 分轮交付计划

每一轮都应留下一个可运行、可评审的前端版本。优先做小步可验证交付，不做一次性的大型未验证 UI 堆叠。

### Round 1：独立壳和基础闭环

目标：验证独立包、desktop 风格 shell、mock seed 方向和核心 UI 数据形态。

范围：

- 创建 `src/frame/provider_metadata_cloud/web/`。
- 添加与 desktop 对齐的 Vite / React / TypeScript / Tailwind 配置。
- 添加 `App.tsx`、`routes.tsx`、i18n provider、theme provider 和基础布局。
- 实现技术参数视图 / 运营参数视图切换；内部可用 service role 表达权限，但 UI 不直接显示 A/B。
- 实现桌面三栏控制台壳：
  - 顶部栏
  - 左侧导航
  - 主工作区
  - 右侧检查器
- 实现移动端单栏浏览壳。
- 从 `driver_metadata/*.json` 派生初始 mock 数据。
- 补齐云服务层 mock 数据：
  - 技术发布 revision / 运营发布 revision
  - edit session
  - pending changes
  - change logs
  - warnings
  - ops overlays
- 建立最小 UI DataModel 代码骨架：
  - `datamodel/types.ts`
  - `datamodel/schemas.ts`
  - `datamodel/selectors.ts`
  - `datamodel/diff.ts`
  - 字段所有权、key 字段、字典项、selector、warning 的基础类型
  - 页面表单从 Zod schema 推导输入类型，后续 Round 持续演进
- 实现页面：
  - Dashboard
  - Providers
  - Models
  - Publish Preview
  - Change Logs
- 实现浏览模式、编辑模式、pending changes 和基础 mock 发布预览。
- PC 编辑模式下，Providers、Models 和 Change Logs 等数量较多的页面使用分页表格；浏览模式可提供卡片视图以兼顾移动端。
- 添加基础 `zh-CN` 和 `en-US` 文案。

验证：

- dev server 能在无后端依赖下启动。
- 新包 build/typecheck 通过。
- Playwright smoke 覆盖页面渲染和无 console error。
- 至少一个 happy path 能从编辑动作进入 Publish Preview。

非目标：

- 完整技术参数编辑表单。
- 完整运营 overlay 编辑表单。
- Import Plan 解析器。
- 完整发布向导。

### Round 2：技术参数 Provider / Model / Rule / Nick 编辑

目标：让 `provider-metadata-tech-service` 的主要技术维护路径可用。

范围：

- Providers 页面：
  - 创建 provider
  - 从已有 provider 模板创建
  - 编辑 provider 详情字段
  - 对高风险 key 字段展示字段级 unlock UI
- Provider Wizard：
  - 作为新增 provider / 聚合 provider 的主路径
  - 步骤覆盖 provider 基本信息、原厂/模型选择、selection rule、nick rewrite、api_type/capability 初始标记、logical mount 建议和发布预览入口
  - 展示匹配失败模型、重复 published id、缺失原厂 meta、key 字段风险
  - OpenRouter 风格 provider 必须能通过该向导完整走通
- Models 页面：
  - 创建全局 model metadata
  - 创建 provider 专属 model override
  - 按 provider、original provider、api type、capability、model id 搜索和过滤
  - 新增或删除全局 model meta 前展示受影响 provider 列表
- Selection Rules 页面：
  - include/exclude origin
  - include/exclude pattern
  - 预览命中模型
- Nick Rules 页面：
  - exact nick
  - 批量 prefix/suffix/rewrite
  - 预览 `source_model_id -> published id`
- Publish Preview：
  - provider/model diff
  - 命中模型数量
  - key 字段风险区
  - published JSON 片段
- 技术 Diagnostics 基础能力：
  - provider/model key 冲突
  - selection rule 空命中
  - nick rewrite 冲突
  - published id 重名
  - 缺失 api_type/capability 字典项
  - warning 可跳转到目标 provider/model/rule/nick

验证：

- 表单使用 `react-hook-form + zod`。
- Playwright 覆盖：
  - 通过 Provider Wizard 创建 OpenRouter 风格 provider
  - 配置 selection rule
  - 配置 nick rule
  - 进入 publish preview 并看到预期 mock diff
  - nick 冲突或空命中 warning 可跳转定位

### Round 3：技术参数 Metadata Blocks / Logical Directory / Dictionaries

目标：补齐技术侧复杂 metadata 维护能力。

范围：

- Metadata Blocks 页面：
  - patterns
  - defaults 作为单个保底参数 object 管理，用于匹配失败或未收录模型的 fallback
  - variants
  - version_rules
  - 进入页面后先按 block type 分组浏览；每种类型使用独立的表格列、筛选项、详情面板、创建表单和编辑表单
  - 创建时选择 block type，创建后 block type 冻结，不允许在编辑中改成其他类型
  - 每种 block type 使用独立 Zod schema 校验，不能用一个宽松 JSON schema 覆盖所有类型
  - patterns/variants/version_rules 以数组存储和发布，但 UI 按数组元素拆成独立记录表格展示、选择和编辑
  - global scope 和 provider scope
  - 创建、复制、编辑、禁用、删除
  - mock JSON/schema 校验；JSON 视图作为辅助检查器，不作为主编辑路径
  - 命中预览
  - nick rewrite 预览
  - rewrite 冲突和空命中 warning
- Logical Directory 页面：
  - 最上方提供丰富的筛选检索区域，可筛选目录，也可筛选目录下包含的模型
  - 次下方用面包屑展示当前目录路径
  - 左侧展示目录结构和目录属性
  - 中间展示匹配到的项目列表，包括目录项和模型项
  - 右侧展示选中项目详情，选中目录时展示目录详情，选中模型时展示模型详情
  - “最上方筛选检索模式”和“按目录路径展示模式”互斥；进入筛选检索时清晰显示当前为搜索结果，进入目录路径浏览时清空或挂起搜索结果
  - 目录树和面包屑浏览
  - 已挂载 model 列表
  - 新增、重命名、移动、删除目录的 mock 流程
  - 批量添加/移除 model
  - 一个 model 可挂载到多个目录
  - 目录 key/path 重复校验、空目录提示、断链引用提示
  - 破坏性逻辑变更前展示受影响 model 数和路径样例
- Dictionaries 页面：
  - api_type 字典
  - capability 字典
  - 新增字典项
  - 删除/重命名前展示引用数和样例
  - 批量给过滤后的 model 结果标记 api_type/capability
  - 字典项应用到 model 或 rule 时必须通过严格匹配的下拉框/combobox 选择
  - api_type/capability 批量标记支持单选、多选和取消标记，bool 属性用开关或勾选表达
  - 少数值属性字段使用结构化输入，提供数值范围、枚举、单位和默认值校验
  - 提供 model × api_type/capability 的矩阵或等价批量核对视图，便于工程师核实覆盖情况和发现漏标/误标

验证：

- Playwright 覆盖：
  - 编辑 metadata block 并执行预览
  - 批量给 model 添加 capability
  - logical directory 风险提示
  - 逻辑目录搜索模式与路径浏览模式互斥
  - 字典项应用只能选择已有项，无法提交拼写错误 key

### Round 4：技术参数 Import Plan 和发布工作流

目标：把 `import plan -> pending changes -> diff -> publish` 做成完整 mock 流程。

范围：

- Import Plan 页面/工作流：
  - 粘贴或导入 YAML / Markdown 文本
  - mock 解析支持的 actions，首版至少覆盖：
    - `upsert_provider`
    - `disable_provider`
    - `upsert_model_meta`
    - `override_model_meta`
    - `include_models`
    - `exclude_models`
    - `set_model_nick`
    - `upsert_pattern`
    - `upsert_defaults`
    - `upsert_variant`
    - `upsert_version_rule`
    - `set_logical_mounts`
    - `upsert_logical_directory`
    - `delete_logical_directory`
    - `move_logical_directory`
    - `set_api_types`
    - `upsert_api_type`
    - `delete_api_type`
    - `set_capabilities`
    - `upsert_capability`
    - `delete_capability`
  - 未支持 action 必须展示明确错误，不能静默忽略
  - 展示 action 列表
  - 展示命中预览和校验错误
  - 对所有 selector 展示命中数量和样例
  - 对删除/重命名 api_type 或 capability、删除/移动逻辑目录、修改 key 字段展示引用关系、影响对象数量和风险提示
  - 对 `upsert_defaults`、`upsert_variant`、`upsert_version_rule` 展示 source block、selector、priority、nick rewrite 后的发布 selector
  - 将解析后的 actions 分发到 edit session pending changes
  - Import Plan 不能直接发布
- Publish Wizard：
  - diff
  - 影响范围
  - schema validation
  - key 字段风险确认
  - 测试建议
  - 发布说明输入
  - 二次确认
  - 写入 mock change log
- 草稿恢复：
  - 未完成草稿提示
  - 继续 / 发布 / 放弃 mock 操作
- revision conflict mock：
  - base revision 过期横幅
  - 禁止直接发布
  - 重新预览动作

验证：

- Playwright 覆盖：
  - 导入 update plan
  - 保存草稿
  - 恢复草稿
  - 发布成功
  - 查看生成的 change log

完成含义：

- 技术参数管理 mock-first MVP 已足够进入产品评审。

### Round 5：运营参数同步源和运营编辑

目标：让 `provider-metadata-ops-service` 的主要运营路径可用，同时保持技术字段和运营字段所有权隔离。

范围：

- 技术源页面：
  - 技术参数服务 URL
  - 测试连接
  - 手动同步
  - 发布源 revision / 运营 revision
  - stale 状态
  - 同步失败时保留上一版可用 A cache
- 运营 Providers 页面：
  - 技术字段只读展示
  - 编辑运营字段：
    - disabled
    - recommendation level
    - display priority
    - routing policy tag
    - ops note
  - 展示客户端可见结果预览
- 运营 Models 页面：
  - 技术字段只读展示
  - 编辑运营字段：
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
- Blocks overlay：
  - 对 pattern/defaults/variants/version_rules 做禁用或运营字段覆盖

验证：

- 运营参数 UI 不能 unlock 或编辑技术字段。
- Playwright 覆盖：
  - 配置技术源
  - 同步
  - 禁用 provider/model
  - 编辑运营字段

### Round 6：运营参数 Bulk Operations / Warnings / Publish Preview

目标：补齐运营参数高频运营和发布风控流程。

范围：

- Bulk Operations 页面：
  - 按 provider 过滤
  - 按 original provider 过滤
  - 按 model id pattern 过滤
  - 按 api_type 过滤
  - 按 capability 过滤
  - 按 recommendation level 过滤
  - 按价格区间过滤
  - 按 routing weight 区间过滤
  - 批量启用/禁用
  - 批量设置 recommendation level
  - 批量设置 display priority
  - 按百分比批量调价
  - 批量设置 input/output price
  - 批量设置 routing weight
  - 清除 pricing override
  - 确认页展示命中数量、样例、修改前后对比和客户端可见性影响
- Warnings 页面：
  - A sync failed
  - A data stale
  - A JSON/schema warning
  - B overlay 污染 A-only 字段
  - provider/model merge skipped
  - invalid pricing
  - routing weight 超出建议范围
  - 从 warning 跳转到目标对象
- 运营 Publish Preview：
  - 最终 published JSON
  - provider/model 下发数量
  - 被 B disabled 移除的 provider/model
  - overlay 命中统计
  - 污染字段丢弃统计
  - stale 发布确认

验证：

- Playwright 覆盖：
  - 批量调价
  - warning 跳转
  - stale 发布确认
  - 最终 JSON export preview

完成含义：

- 运营参数管理 mock-first MVP 已足够进入产品评审。

### Round 7：UI 质量、移动端、状态和 i18n 收敛

目标：把原型从功能可用推进到可评审质量。

范围：

- 375px 左右移动端：
  - 只支持浏览
  - 仅在 PRD 允许处支持紧急禁用
  - 不支持普通编辑
  - 不支持复杂批量操作
  - 不支持发布
  - 列表/详情单栏导航可用
- 数据状态：
  - loading
  - empty
  - error
  - retry
  - stale
  - warning
- i18n：
  - 完整 `zh-CN`
  - 完整 `en-US`
  - 切换语言不丢失过滤条件、选中对象或 edit session
- 视觉质量：
  - 按钮、卡片、表格无文字溢出
  - 无不合理重叠
  - 状态同时使用文字和视觉指示
  - 合适位置使用图标控件
  - 保持高密度运营工具布局，不做营销页
- 可访问性和交互基础：
  - focus-visible 状态
  - 核心按钮/输入框可键盘操作
  - 无 console error

验证：

- Playwright 覆盖：
  - 移动端浏览模式
  - 空态
  - 错误和重试态
  - i18n 切换
  - 核心页面无 console error

### Round 8：UI DataModel 文档和集成准备

目标：从收敛后的 mock 原型中提取稳定 UI DataModel，为后续后端集成做准备。

范围：

- 创建 `doc/aicc/provider-metadata-cloud-webui-datamodel.md`。
- 文档包含：
  - overview
  - 支持的页面/视图
  - 原型实际使用的 TypeScript interfaces
  - 输入 Zod schemas
  - loading/error/stale 状态定义
  - 分页策略
  - 筛选和排序字段
  - 派生/聚合字段
  - 技术字段/运营字段所有权规则
  - 字段稳定性分类：
    - Frozen
    - Extensible
    - Volatile
  - mock 数据契约
  - 非法输入样例
  - 未来 API/KRPC mapping notes
- 整理 mock API 边界：
  - pages 调 store/hooks
  - store 调 `mock/api.ts`
  - 后续真实后端替换 mock API 时，页面不应大改

验证：

- build/typecheck 通过。
- 已配置 lint 时 lint 通过。
- Playwright suite 通过。
- DataModel 文档必须匹配当前实现，不能写成猜测式后端 DTO。

完成含义：

- mock-first WebUI 原型完整交付。
- 项目可以进入后端集成规划或实现阶段。

## 5. 跨轮验证规则

每一轮都必须遵守：

- app 保持独立可运行。
- 不调用真实后端 API。
- 不修改 desktop registry。
- 不把 provider metadata cloud 代码加入 `src/frame/desktop/src/app/ai-center`。
- 面向用户的文本应走 i18n。
- 新表单使用 `react-hook-form + zod`。
- 受控字段应用路径必须通过下拉框、combobox、单选/多选或等价选择控件完成，并由 schema 校验禁止提交不存在的 provider/model/directory/api_type/capability key。
- 一个数据视图出现后，应尽快补齐 normal、loading、empty、error、warning/stale 状态。
- Playwright 测试聚焦本轮新增流程。

## 6. 建议的 session 交接格式

每个实现 session 结束时记录：

- 完成的轮次和范围。
- 修改的文件。
- 如何运行 app。
- 验证命令和结果。
- 下一轮已知缺口。
- 是否偏离本计划，以及偏离原因。

推荐简短交接示例：

```text
Completed: Round 2 partial - A Providers and Selection Rules.
Changed: src/frame/provider_metadata_cloud/web/src/pages/providers, pages/selection-rules, datamodel/schemas.ts.
Run: npm -C src/frame/provider_metadata_cloud/web run dev.
Verified: npm -C ... run build; npx playwright test tests/e2e/flows/a-provider-openrouter.spec.ts.
Next: finish Nick Rules and Publish Preview risk section.
```

## 7. 当前下一步

从 Round 1 开始：

1. 在 `src/frame/provider_metadata_cloud/web/` 下创建独立 package。
2. 复制/适配 desktop 的 build、Tailwind、theme 和 i18n 模式。
3. 实现 cloud console shell 和 mock seed 层。
4. 添加 Dashboard、Providers、Models、Publish Preview、Change Logs 页面。
5. 运行 build/typecheck 和 Playwright smoke。
