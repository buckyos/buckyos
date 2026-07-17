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
- `models`、`patterns`、`defaults` 本质上都是模型参数匹配规则：`models` 精确匹配原始 `model.id`，`patterns` 按规则和顺序匹配，`defaults` 在前两者都失败后兜底匹配。三者必须统一存储在 `model_param_rules` 表/数据集合中，通过 `match_type: exact | pattern | default` 区分，并按 exact -> pattern(priority) -> default 的顺序保证单一最终命中。
- 统一存储不改变客户端下发格式。最终下发给客户端和发布预览导出的 JSON 必须物化为 AICC driver metadata document：`schema_version`、`provider_driver`、`name`、`protocol_family`、`base_url`、`revision`、`models`、`patterns`、`defaults`、`variants`、`version_rules`、`signature`。其中 `match_type=exact` 输出到 `models[]`，`match_type=pattern` 按 priority 稳定排序输出到 `patterns[]`，`match_type=default` 输出到 `defaults`。
- `models`、`patterns`、`defaults` 使用同一套 model-meta 字段，至少包括模型 id/pattern、api_types、logical_mounts、capabilities、context_limits、pricing、成本/延迟估算和质量/成本/延迟等级。`pricing` 必须带币种；如果 provider 运行时可动态返回价格，客户端以 provider 动态价格优先，币种和汇率由客户端统一处理。
- `provider_key`、Nick Rules、Logical Directory、Dictionaries 和运营 overlay 都是管理/编辑/校验输入；发布预览可以展示它们的影响，但最终客户端 driver metadata JSON 不得原样携带这些中间概念或后台字段。
- exact、pattern、default 只在 Models 页面作为 `model_param_rules` 管理。Resolver Rules 页面不管理 `model_param_rules`，只管理 `variants` 和 `version_rules`。
- `exclude` 是 exact/pattern `model_param_rule` 的属性。`exclude=true` 时该规则作为发布排除规则保留，其它参数字段不参与当前发布语义但仍可保存，便于取消 exclude 后恢复；default 不设置 exclude。
- Provider 的 models、patterns、defaults、variants、version_rules 一律使用白名单引用；禁用通过对象自身的 `exclude=true` 表达，不再保留 Selection Rules 或 include 候选集合。
- `variants`、`version_rules` 仍是数组；每个数组元素应作为独立记录展示、选择和编辑。存储和 mock seed 使用 `metadata_variants`、`metadata_version_rules` 等类型表/数据集合组织。不能只保存页面布局需要的最终聚合结果。
- exact/pattern/default、variants、version_rules 必须按类型使用不同 schema、不同字段定义、不同列表列配置和不同详情编辑视图。
- Mock 数据必须模拟后端持久化表结构。页面需要的计数、命中数量、发布样例、风险摘要、树形视图和最终 JSON 片段，必须通过 `mock/api.ts`、`datamodel/selectors.ts` 或等价接口计算得到；这些接口后续应能演进为真实后端 API，不得把 mock seed 做成迎合 Web 页面布局的预计算结果。
- 文档中使用的 A/B 只是架构讨论简称，UI 上不得直接展示“A 服务”“B 服务”“A/B service role”等字样。用户界面应使用“技术参数”“运营参数”“技术源”“同步源”“发布源 revision”“运营 revision”等面向用户可理解的名称。
- `api_type`、`capability`、目录、provider、model 等受控字段在应用到对象时必须严格匹配已有字典或对象集合。主路径优先使用下拉框、combobox、单选/多选选择器和批量选择控件，避免自由文本输入导致 key 拼写错误。
- 字典项应用时，大多数 capability/api_type 属性按 bool 语义处理为“支持/不支持”；少数值属性字段必须提供带 schema 校验的结构化输入，不得只依赖 JSON 文本。
- JSON / YAML / Markdown 等长文本视图必须提供复制和下载导出；长文本输入必须支持粘贴和本地文件上传导入。JSON 视图是辅助检查器，不作为复杂对象的主编辑路径。
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
- mock seed 按文档定义的表格/数据集合组织，包括 providers、model_param_rules、metadata_variants、metadata_version_rules、provider 源对象引用、model_nicks、logical directories、api_type/capability dictionaries；不保存只为页面展示而预计算的最终结果。
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
  - Kind 只保留：`origin` 表示原厂 provider；`aggregator` 表示 OpenRouter 这类聚合多个原厂 inventory 的 provider。协议兼容性由 `protocol_family` 表达，不再使用 `compatible_proxy`。
  - Basic 页 `Name` 是 provider 的 UI 展示名，由用户输入，忽略大小写后必须在 Provider 中唯一，只允许英文字母、数字、下划线和连词符。`Driver` 是该 Provider 代表规则的唯一 id，下发给客户端的文件名使用它，最终 JSON 写入 `provider_driver` 字段；新建 provider 时由 `Name.toLowerCase()` 构造。`Protocol family` 是客户端连接该 provider 的 wire protocol，创建 provider 时必须从当前支持协议中选择，客户端依据它判定如何和模型商服务器通信。
  - 步骤顺序为 `Basic -> Models -> Model params -> Pattern order -> Variants / Version rules -> Variant/version params -> Logical mounts -> Nick rewrite -> Preview`。编辑已有 provider 复用同一向导且 provider key 由服务端自动编号、只读。
  - 编辑已有 provider 时，向导必须回填并以完整集合替换该 provider 的 scoped models、variants、version rules、nick rules；不得将“新建向导”的默认来源对象混入现有配置。
  - Models 步骤选择已有模型匹配规则，不新建规则；新建和编辑 exact/pattern/default `model_param_rules` 放在 Model params 步骤。
  - Models 步骤使用三列表：原厂 providers 单选聚焦列表（展示已选/总匹配规则数）、选中原厂的模型匹配规则列表（支持逐项选择/反选和全选/反选/清空）、所有已选匹配规则列表（展示总数）。
  - Model params 步骤可为前面已选模型新建或编辑 model_param_rule，可按 origin provider 组织；`model_driver` 是本步骤可编辑的模型规则参数字段，用于 OpenRouter、FAL 等聚合 provider 保留逐模型上游归属；批量编辑只能修改参数字段，不能批量改写匹配身份字段（`model.id`、`pattern`、`default`）。本步骤不编辑 priority，pattern 优先级只在 Pattern order 步骤通过列表顺序维护。
  - Model params 执行完成后，exact/pattern/default 匹配身份必须唯一，且只能有一个 default；来自多个原厂的 default 需要先拆成可区分 pattern 后再创建新的 default。
  - Model params 后设置独立 Pattern order 步骤，用列表顺序表达 pattern 匹配优先级；发布前按该顺序写入 priority。
  - Variants / Version rules 步骤复用 Models 的三列选择交互；Variant/version params 步骤用于新建和编辑 variant/version rule 草稿，Variants 与 Version 使用两个独立 Tab 和类型专属参数面板。每个 Tab 的待编辑对象选择方式与 Model params 一致，支持多选、Pending edit list、Discard 和 Apply；type 不可原地修改，本步骤不编辑 priority，也不批量编辑 logical mounts/auto_mounts。Variant 面板编辑 `provider_options`；Version rule 面板编辑完整谓词和参数，包括 `family`、`tier`、`model_pattern`、`tier_tokens`、`exclude_tier_tokens`、`version_rank.prefix`、`stability.unstable_tokens`、`stability.current_requires_stable`、`exclude_snapshot_date_suffix`。`tier_tokens`、`exclude_tier_tokens`、`stability.unstable_tokens` 是自由文本 token 数组，不来自字典。`current_mount` 和 `version_mount` 通过物化 logical directory 树单选路径，保存为 driver metadata 的 mount 字符串。
  - Logical mounts 是 Add Provider 向导中唯一的 `logical_mounts`/`auto_mounts` 批量挂载编辑步骤，覆盖 Model params 产生的 models、patterns、defaults，以及 Variant/version params 产生的 version rules；models/patterns/defaults 写入 `logical_mounts`，version rules 写入 `auto_mounts`。variants 在该步骤暂不配置，保留来源或草稿中的既有 `logical_mounts`。
  - Models 按原厂 provider 展示已存在的 models、patterns、defaults；defaults 也是可选对象。中列和已选列按三类 Tab 展示。新建规则仅在 Model params。
  - Variants / Version rules 按原厂 provider 展示已经存在的 `metadata_variants`、`metadata_version_rules`，通过两个 Tab 独立选择；新建只在 Variant/version params。复制来源对象时保存其 source key，后续源对象变更可识别依赖。
  - Resolver Rules 页面使用 Variants 与 Version rules 两个 Tab，并按 provider 筛选；两类对象的编辑表单和 logical mounts 面板保持分离。
  - Model params 和 Variant/version params 不重复批量编辑 `logical_mounts` 或 version rule `auto_mounts`；models/patterns/defaults 的 `logical_mounts` 和 version rule 的 `auto_mounts` 只在独立 Logical mounts 步骤编辑。Variant 的 `logical_mounts` 在 Add Provider 向导中暂不配置。Version rule 的 `current_mount`/`version_mount` 是单值挂载字段，在 Variant/version params 中通过目录树选择。数值能力如 `max_context_tokens` 只有选中相应 capability 后才显示数值输入。
  - Model params 按 source original provider 与 models/patterns/defaults Tab 展示已选对象；手工新增对象归入目标 provider。可跨分组勾选多个对象，对 API type 或 capability 执行字段级批量覆盖，未涉及字段保持每条对象原值。
  - Model params 使用临时编辑会话：Apply 提交当前选择对象的参数变更并清空选择，Discard 恢复该会话开始时当前选择对象的参数。多选时禁止改写 match identity 字段，`Model selector` 必须逐项赋值。
  - 保存前校验 match identity 唯一性；每个 provider 最多一个 default。引用多个原厂 default 时必须先将它们改为带 origin 范围的 pattern，再新增一个目标 provider default。
  - Logical mounts 使用与 Model params 一致的上下布局：上方是待编辑对象选择区，包含 provider 列、非空的 models/patterns/defaults/version rules Tab 和 Pending edit list；下方是完整目录树和 Selected paths 列表。默认选中当前 provider 下第一个有对象的 Tab。对象可跨 provider/Tab 勾选；目录树按全部目标已选/部分目标已选/未选展示三态，点击部分选中路径会变为实选中，点击实选中路径会变为未选中。右侧 Selected paths 只列出全部目标实选路径，点击路径会取消该路径的实选中状态。Apply 时实选路径加到全部目标、未选路径从全部目标移除、虚选路径保持各自原状；Discard 只丢弃尚未 Apply 的路径选择变更，保留当前待编辑对象选择并恢复到选择路径前的目录树状态。
  - Nick rewrite 是为了复用原厂模型匹配规则而引入的发布时重命名中间概念。OpenRouter 这类 provider 可以只记录选中的原厂 models、patterns、variants、version rules 及多条 rewrite 规则，在最终 driver metadata 下发时把原厂模型名或模型 pattern 改写成自身 inventory 名称，而不是为每个模型复制一份改名后的规则。exact models 下发为改写后的 `models[].id`，patterns 下发为改写后的 `patterns[].pattern`；default 不下发 selector，仅作为参数兜底规则参与预览。
  - Nick Rules 独立页是跨 provider 的规则库，交互与向导 Nick rewrite 对齐：新建时选择 `Origin prefix`（`* -> prefix/{model}`）或 `Exact/pattern mapping`，再编辑规则并在 models、patterns、variants、version rules 范围内预览影响；default 只展示参数兜底预览，不下发 selector。
  - Nick rewrite 必须支持多条规则，按 provider、original provider、exact/pattern selector 和 priority 预览最终 published id。OpenRouter 这类聚合 provider 应能用多条规则表达 OpenAI、Claude、Gemini 等不同上游前缀。Variants 没有匹配字段时使用默认 selector `*`，该 `*` 也参与 rewrite；version rules 的通配字段是 `content.model_pattern`，发布预览和最终下发都重写该字段。
  - API type、capability、logical mounts 是 `model_param_rules` 的模型参数字段。向导中可以提供批量默认值，但必须能进入逐模型或逐规则编辑，并最终保存到 exact/pattern/default model_param_rule。
  - 展示匹配失败模型、重复 published id、缺失原厂 meta、key 字段风险
  - OpenRouter 风格 provider 必须能通过该向导完整走通
- Models 页面：
  - 创建全局/原厂 exact、pattern、default model_param_rule
  - 创建 provider 专属 exact、pattern、default model_param_rule override
  - 按 exact / pattern / default 分组浏览和编辑
  - exact 规则精确匹配原始 `model.id`
  - pattern 规则按 selector 和 priority 顺序匹配，避免一个 `model.id` 最终命中多项
  - default 规则是 `model_param_rules` 中不带 selector 和 priority 的兜底规则，用于 exact 和 pattern 全部匹配失败后的 fallback 参数
  - exact/pattern 可设置 `exclude`；`exclude=true` 时其它参数字段当前不参与发布语义，但保留等待恢复
  - 可以从 exact、pattern、default 任意一种规则复制创建另一种类型的新规则，但不能原地修改旧规则的 `match_type`
  - 从 pattern 创建 exact 时，继承参数字段，把 `model_id_selector` 从 pattern selector 改为一个或多个精确原始 `model.id`；从 default 创建 exact/pattern 时，继承 fallback 参数字段并补充目标 `model_id_selector`，pattern 还必须补充 priority
  - 当来源规则类型和目标规则类型不一致时，WebUI 必须明确提示目标类型、来源类型和会被改写/清空的字段，并要求用户确认；最终 `match_type` 以用户选择的目标类型为准
  - 按 provider、original provider、api type、capability、model id 搜索和过滤
  - 新增或删除全局/原厂 model_param_rule 前展示受影响 provider 列表
  - mock API/selectors 必须从 `model_param_rules` 计算出发布 JSON 的 `models`、`patterns`、`defaults` 三个字段，并验证 models 精确命中优先、patterns 前高后低、defaults 最后兜底
- Provider 白名单仅由向导中对 models、patterns、defaults、variants、version rules 的源对象选择维护。exact/pattern 的 `exclude=true` 是已引用对象自身的禁用语义。
- Nick Rules 页面：
  - exact nick
  - 使用与 Provider Wizard 的 Nick rewrite 步骤一致的交互：点击 Add 新建规则草稿，编辑 original provider、exact/pattern selector、rewrite template 和 priority，保存后进入规则列表。
  - 按 original provider 和 exact/pattern selector 维护多条规则；original provider 必须明确，不提供 `All` 作为可提交值。
  - 批量 prefix/suffix/rewrite
  - 预览 models、patterns、variants、version_rules 的 `source selector -> published selector`。version_rules 预览使用 `content.model_pattern`，variants 无 selector 时使用 `*`。与 Provider Wizard 的区别是页面独立管理所有 provider 的 nick 规则，不绑定正在创建的单个 provider。
- Publish Preview：
  - provider/model diff
  - 命中模型数量
  - key 字段风险区
  - published JSON 片段
- 技术 Diagnostics 基础能力：
  - provider/model key 冲突
  - 白名单引用对象的 selector 空命中
  - nick rewrite 冲突
  - published id 重名
  - 缺失 api_type/capability 字典项
  - warning 可跳转到目标 provider/model/rule/nick

验证：

- 表单使用 `react-hook-form + zod`。
- Playwright 覆盖：
  - 通过 Provider Wizard 创建 OpenRouter 风格 provider
  - 选择 source models/patterns/defaults
  - 配置 nick rule
  - 进入 publish preview 并看到预期 mock diff
  - nick 冲突或空命中 warning 可跳转定位

### Round 3：技术参数 Resolver Rules / Logical Directory / Dictionaries

目标：补齐技术侧复杂 metadata 维护能力。

范围：

- Resolver Rules 页面：
  - variants
  - version_rules
  - original provider 是明确匹配维度，表单和筛选中不得提供 `All` 作为可提交选项；需要全局作用域时用 provider scope 表达，不用空 original provider 表达。
  - 进入页面后按 variants / version_rules 分组浏览；每种类型使用独立的表格列、筛选项、详情面板、创建表单和编辑表单
  - variants/version_rules 以数组发布；UI 都按独立记录表格展示、选择和编辑
  - 每种类型使用独立 Zod schema 校验，不能用一个宽松 JSON schema 覆盖所有类型
  - global scope 和 provider scope
  - 创建、复制、编辑、禁用、删除
  - mock JSON/schema 校验；JSON 视图作为辅助检查器，不作为主编辑路径
  - 页面展示的命中数量、nick rewrite 预览和发布 JSON 片段必须通过 mock API/selectors 从表格化 mock seed 计算得到
  - JSON 视图和导出 provider 全量信息时必须按发布格式展示 variants/version_rules；`models`、`patterns`、`defaults` 的主编辑和预览入口在 Models / Publish 中
  - 命中预览
  - nick rewrite 预览
  - rewrite 冲突和空命中 warning
- Logical Directory 页面：
  - 最上方提供丰富的筛选检索区域，可筛选目录，也可筛选目录下包含的模型
  - 次下方用面包屑展示当前目录路径
  - 左侧展示目录结构和目录属性
  - 目录树必须从 `model_param_rules.logical_mounts`、variants/version_rules 自动挂载项和显式 logical directory 记录共同物化，不能只固定展示 LLM/Image/Audio 等少数顶层目录
  - 中间展示匹配到的项目列表，包括目录项和模型项
  - 右侧展示选中项目详情，选中目录时展示目录详情，选中模型时展示模型详情
  - “最上方筛选检索模式”和“按目录路径展示模式”互斥；进入筛选检索时清晰显示当前为搜索结果，进入目录路径浏览时清空或挂起搜索结果
  - 目录树和面包屑浏览
  - 已挂载 model 列表
  - 已挂载 model 列表必须支持搜索、分页或虚拟列表，避免大量模型挤在小尺寸多选框中
  - 新增、重命名、移动、删除目录的 mock 流程
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
  - 编辑 variant/version_rule 并执行预览
  - 批量给 model 添加 capability
  - logical directory 风险提示
  - 逻辑目录搜索模式与路径浏览模式互斥
  - 字典项应用只能选择已有项，无法提交拼写错误 key

### Round 4：技术参数 Import Plan 和发布工作流

目标：把 `import plan -> pending changes -> diff -> publish` 做成完整 mock 流程。

范围：

- Import Plan 页面/工作流：
  - 粘贴或从本地文件导入 YAML / Markdown 文本
  - mock 解析支持的 actions，首版至少覆盖：
    - `upsert_provider`
    - `disable_provider`
    - `upsert_model_param_rule`
    - `delete_model_param_rule`
    - `include_models`
    - `exclude_models`
    - `set_model_nick`
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
  - 对 `upsert_model_param_rule` 展示 match_type、selector、priority、最终 fallback 行为和命中样例；对 `upsert_variant`、`upsert_version_rule` 展示来源记录、selector、priority、nick rewrite 后的发布 selector
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
- Resolver Rules overlay：
  - 对 variants/version_rules 做禁用或运营字段覆盖；model_param_rule 运营字段由 Models 运营页面维护

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

## 6. 验证事实

- `cmd /c "npm -C src\frame\provider_metadata_cloud\web run build"` 应通过；当前环境会提示 Node.js 22.11.0 低于 Vite 建议的 22.12+，但不阻止构建。
- Playwright 覆盖应按当前 authoring 规则维护：`provider_key`、`nick_key`、`variant_key`、`version_rule_key` 是自动编号且只读，用例不得通过填写 key 输入框创建对象。
- Provider Wizard、Resolver Rules、Nick Rules 等创建流程使用系统生成 key；测试应读取系统生成值或断言只读展示。

## 7. 实现事实

mock WebUI 按独立 package 方式开发，本文档、PRD、DataModel 和 mock 实现使用同一事实口径：

1. Models 页面是 exact/pattern/default `model_param_rules` 的唯一主编辑入口。
2. Resolver Rules 页面只管理 variants/version_rules。
3. Provider Wizard 通过白名单选择 source rules；exclude 由规则自身的 `model_param_rules.exclude=true` 表达。
4. Provider Wizard 按 `Basic -> Models -> Model params -> Pattern order -> Variants / Version rules -> Variant/version params -> Logical mounts -> Nick rewrite -> Preview` 排列，且可编辑已有 provider（provider key 自动编号且不可改）。

## 8. Authoring 约束（beta 2.2）

1. Provider 模板只能选择一个具体 provider，或选择“从零创建”；不提供 `All`。Kind 只保留 `origin` 与 `aggregator`，协议兼容性由 `protocol_family` 表达。`Name` 是忽略大小写唯一的展示名；`Driver` 是 driver metadata document 的唯一 `provider_driver`，由 `Name.toLowerCase()` 构造并作为下发文件名；`Protocol family` 是客户端 wire protocol，允许多个 provider 复用。
2. `provider_key`、`nick_key`、`variant_key`、`version_rule_key` 自动编号，不提供字段级修改。Provider 编辑复用新增向导，回填当前 provider 的 scoped models、variants、version rules、nick rules，保存时以完整 scoped set 替换。
3. Models/Variants/Version rules 都是白名单源对象选择，不在选择步骤创建对象。Models 必须包含 models、patterns、defaults；每个 Tab 展示已选/总数。参数步骤采用与 Models 相同的三栏目标选择区加下方参数编辑区：批量编辑只覆盖明确赋值字段，字段相同才显示共同值，未赋值字段保留原值；match identity 不能批量改写，`model selector` 必须逐项赋值，Model params 不编辑 priority；`original_provider` 全部为当前 provider 时只读显示，否则只能通过按钮设为当前 provider。Apply 应用当前批次并清空目标，Discard 恢复当前批次参数。参数改动实际不同于来源时才生成当前 provider 专属对象；`max_context_tokens` 仅在 capability 被选中时显示。
4. Model params 之后必须有 Pattern order，用顺序而不是裸数字配置 pattern priority。最终匹配身份必须唯一，每个 provider 只能有一个 default；多个来源 default 先转换为可区分 pattern，再创建新 default。
5. Variants 与 Version rules 分开选择、分开参数 Tab；Variant/version params 的每个 Tab 使用与 Model params 相同的待编辑对象多选、Pending edit list、Discard 和 Apply 交互。该步骤不编辑 priority，也不批量编辑 logical mounts/auto_mounts；Variant 面板编辑 `provider_options`，Version rule 面板编辑完整匹配谓词和参数，不只展示 `model_pattern`。`tier_tokens`、`exclude_tier_tokens`、`stability.unstable_tokens` 使用自由文本 token 输入；`current_mount` 和 `version_mount` 通过物化目录树单选。它们的 key 自动编号且 type 不可原地改变。
6. Logical mounts 是 Add Provider 向导中唯一的 `logical_mounts`/`auto_mounts` 批量挂载编辑步骤，覆盖 models/patterns/defaults 和 version rules；models/patterns/defaults 写入 `logical_mounts`，version rules 写入 `auto_mounts`，variants 在该步骤暂不配置。该步骤使用完整目录树和三态跨对象编辑，右侧 Selected paths 只显示实选路径；Apply 写入当前实选/未选路径语义，Discard 丢弃尚未 Apply 的路径选择变更并恢复目录树状态。Version rule 的 `current_mount`/`version_mount` 只在参数面板中作为单值挂载选择，不属于该批量挂载集合。Logical Directory 页面面包屑可点击、路径不得产生双斜杠，目录子项必须完整物化；暂不提供“为目录批量添加模型”。
7. Nick rewrite 是发布期映射而非对象拷贝，支持 origin-prefix 和 exact/pattern mapping，多条规则作用于 models、patterns、variants、version rules；exact models 重写 `models[].id`，patterns 重写 `patterns[].pattern`，default 无下发 selector。无 selector variant 的默认 selector 是 `*`，`*` 本身也可被重写；version rules 重写 `content.model_pattern`。Nick Rules 页面复用该编辑器，新增时指定 provider，编辑时 provider 不可变。
8. 系统不存在 Selection Rules 导航、路由、表或持久化。`exclude=true` 是 exact/pattern 规则自身的禁用语义。任何来源对象变更或删除都必须由 reverse reference 产生“同步引用 provider”告警。
9. Providers、Models、Nick Rules、Resolver Rules、Dictionaries 在浏览模式只能查看 Inspector；编辑模式才显示行级编辑/删除。Resolver Rules 使用 Variants/Version rules 双 Tab、按 provider 检索，provider/original provider 创建后不可改。
10. Logical Directory 必须从 logical_mounts、variants/version_rules 自动挂载项和显式目录共同物化树。
11. 长文本查看和输入必须补齐复制、下载、上传导入能力。
12. 所有可能增长的列表必须补齐分页、搜索和筛选。

## 9. Origin Identity 更新（beta 2.2）

本次 provider-driver-metadata 下发必须解决“同一个物理模型在不同服务渠道下挂载到相同逻辑目录”的问题。最终 driver metadata 使用 `schema_version: 2`，`{driver}` 和 `{model}` 在 logical mounts 中不再表示当前服务渠道的 driver/model id，而是表示解析后的模型原厂 provider 和原厂模型名。

OpenRouter 这类聚合平台的模型 id 仍然作为 provider-native selector 使用，例如 `openai/gpt-5.5` 用于命中 OpenRouter 的接口模型；同时发布 JSON 必须额外物化 `origin_mappings`，把 provider-native model id 映射到原厂身份，例如 `driver=openai`、`model=gpt-5.5`。这样 OpenRouter 提供的 `openai/gpt-5.5` 和 OpenAI 官方提供的 `gpt-5.5` 可以挂载到同一个逻辑路径。

Nick Rules 在 WebUI 中继续表示旧的 Nick rewrite，用于构造 provider-native selector，例如 `openai/{model}`。beta 2.2 额外新增 Origin mappings authoring 规则，用于生成最终下发的 `origin_mappings`。Origin mappings 支持 template 和 regex 两种模式；regex 模式使用标准 regex 语法，命名捕获组固定为 `(?<driver>...)` 和 `(?<model>...)`；template 模式中只使用 `<driver>` 和 `<model>` 占位，不复用 JSON 里已有的 `{driver}` / `{model}`，避免把 regex 转义和 JSON 模板语义混在一起。

WebUI 页面按 Tab 拆分为 `Nick Rules`、`Origin mappings`、`Origin provider aliases`。`Origin mappings` 需要编辑 match 模式、origin template 或 regex、priority，以及 driver/model transforms；`Origin provider aliases` 需要编辑 alias 到 driver 的归一化表。Add Provider 向导也必须提供同样的 origin mappings、transforms 和 provider aliases 编辑入口。

`Mapping preview` 是三个 Tab 共享的底部预览，不属于任一 Tab；预览卡片必须展示原厂 provider、原厂/source model id 和发布后的 provider-native id。`Origin mappings` 编辑区右侧展示最终下发的 `origin_mappings` JSON，Add Provider 向导使用相同布局。

最终下发 JSON 新增 `origin_provider_aliases` 和 `origin_mappings`。`origin_provider_aliases` 用于把 OpenRouter 返回的 provider 前缀归一化到系统认可的 driver id；`origin_mappings` 由 WebUI 的 Origin mappings authoring 规则计算得到，不再从 Nick rewrite 规则隐式推导。暂不支持 dynamic alias，聚合商提供的软链接模型由 `patterns[].exclude=true` 排除，不新增 `exclude_patterns` 字段。
