# BuckyOS AI Workspace 扩展实现思路：Render-Block 与 Block-control

> 状态：v0.1 讨论稿，2026-10-28。基线：脑图实施提交 `8f30ad030`（PROTOCOL 1.1、第一个 Block-control）之后的工作区源码，事实依据见[脑图实施记录与验收报告](<脑图实施记录与验收报告.md>)与[脑图对象方案讨论](<脑图对象方案讨论.md>) v0.4。本文把脑图实施中落下来的机制整理成**两条扩展路线**，并给出用户在 Workspace UI 里插入 / 导入扩展的交互思路。§7 是需要裁决的问题。
>
> 读法沿用[连接线实现方案讨论](<连接线实现方案讨论.md>)：先给结论，再给现状，再给方案与 UI，最后是裁决表。§2 描述的是现状，每条都能在源码里找到。

## 1. 结论

AI Workspace 的"一个对象"由两层组成：**Block 决定一格之内画什么，Block-control 决定一组 Block 怎么排、怎么连、怎么操作**。脑图实施证明这两层可以独立扩展，也证明了两层各有一个"核心函数"：

| 路线 | 核心 | 扩展的是什么 | 产物形态 | 典型例子 |
| --- | --- | --- | --- | --- |
| **路线一：扩展 Block** | **render-block**：`(RenderContext) → 视图`，即 `BlockDefinition.Static`（及其 View / Editor 变体） | 一个新的表现：给定 Cell、绑定数据、模式与尺寸，画出什么 | 代码注册的 `BlockDefinition`，或工作区里的 `buckyos.block-def` 实体（`declarative` / `html`） | KPI 卡、图表、日历、嵌入第三方可视化、许愿格 HTML 结果 |
| **路线二：Block-control** | **solve + Chrome**：`(outline 子树, config) → 子节点的 auto 分量` 和 `(Laid) → 容器级外观` | 一种新的组合方式：把已有 Block 当节点，容器拥有布局、隐式边、键盘与工具栏 | 代码注册的 `ContainerDefinition` + 共享模块里的 `ControlDescriptor`，容器 kind 即 control id | 脑图（已实现）、组织结构图、看板、时间线、Button 类组合控件 |

两条路线的分界只有一个问题：**新东西需不需要自己排布一组子 Block？** 不需要就是 Block（哪怕它内部很复杂，用 HTML 画），需要就是 control（哪怕节点只是普通文字）。control 不画节点，节点永远是 Block；Block 不排兄弟，排布永远属于父容器（脑图方案 §5.2 "布局由父容器拥有"）。

现状对两条路线的支持程度不同：

- **路线一**机制完整（第二期 §10 的两个扩展点 + 第一期的 `BlockRegistry`），**缺的是产品入口**：插入目录的"扩展"页签读 `buckyos.block-def`，空态文案说"可在数据源中导入"，但 Desktop 里没有任何导入 block-def 的入口，定义只能由许愿格 HTML 结果生成或由 Agent `doc.commit` 写入。
- **路线二**机制刚落地（脑图 M0–M4），**缺的是泛化**：内核对容器 `config` 的校验是 mindmap 专用的硬编码，`layout.algorithm` 只有 `tree`，模板子节点（R23）的 Chrome 接口没落地，`control.import / export` 只认大纲格式。第二个 control 现在要碰内核、WASM、共享模块、Desktop、服务四处。

本文的建议（§5、§6）：路线一先补**导入与管理 UI**和 block-def 的**自述 / 目录元数据**；路线二先做**内核 config 校验按 kind 分派**和一个通用的 **`composite` control**（fixed 布局 + slots，子节点是任意 Block），这样"基于已有 Block 组合一个新对象"不必每次都发版。

## 2. 现状：脑图实施留下的扩展机制

### 2.1 Block 扩展（路线一的基础）

**注册表与契约**（`ui/blocks/registry.ts`）。前端是"哪个 Renderer 显示哪种数据"的权威（第二期 D6）：Cell 记 `view.type` + `view.version`，`BlockRegistry.resolve` 解析到一个 `BlockDefinition`，解析失败按原因（`unknown_renderer` / `unsupported_version` / `type_not_accepted` / `source_required` / `definition_missing` / `invalid_definition` / `unsupported_api` / `invalid_config` / `data_unavailable`）回退到通用只读视图。内核只校验引用存在、Renderer 标识格式合法、配置 ≤ 64 KiB，不判断 Renderer 支持什么数据；**新增 Renderer 不需要改后台**。

`BlockDefinition` 的字段按职责分组，就是 render-block 契约的全貌：

| 组 | 字段 | 说明 |
| --- | --- | --- |
| 身份 | `type`、`version`、`title` | `type` 即 `view.type`，版本不匹配不静默升级 |
| 数据契约 | `accepts`、`allowNoSource`、`pureUi` | 接受的数据类型；无源 Block（形状、框）只在 BlockTree |
| **渲染**（核心） | `Static`（必需）、`Simplified`、`View`、`Editor`、`Inspector` | 模式 × 状态正交（§10.2）：宿主给 `RenderContext`（cell、payload、source、mode、view、selected、hovered、editorActive、capabilities、readOnlyReason、dataState、size、zoom、activateEditor、openEntity、setEditorToolbar、annotate…），Renderer 不碰客户端与存储 |
| 交互 | `actions`、`toolbar`、`hover`、`keys`（经 actions 的 `key`） | 宿主统一放进就近工具栏、悬浮项、菜单 |
| 几何 | `defaultSize`、`resize`、`shape`、`anchors`、`chrome` | 连接线用 `shape` / `anchors`，`chrome: none` 允许画出格子外 |
| 插入 | `create`、`catalog`（group / description / needs / variants / standard） | 一次插入返回一组 Operation；`catalog` 决定它在插入目录里怎么出现 |
| 成本 | `cost: { editor, html }` | 宿主限制同时挂载的编辑器 / HTML 运行时数量（§9.3 第 8 条） |
| 定义实体 | `definitionKind`、`configSchema`、`configFields` | 宿主型 Block（`declarative` / `html`）从文档里的定义实体取 title / accepts / default_size / config_schema |

**三档实现**：

1. **代码注册**（随 Desktop 发布）：内置对象与扩展样本 `ui/blocks/samples.tsx`（指标、柱状图）。能用 React、能用全部字段。
2. **声明式定义**（`buckyos.block-def` 实体，`kind: declarative`，`ui/extensions/declarative.tsx` 解释）：不执行代码；词汇只有六种 item（text / value / metric / bar / list / badge）、`stack | row` 布局、`view` 变体、动作只能映射到三个既有命令（`open_source` / `activate_editor` / `open_definition`）、inspector 字段。季度经营分析模板的 KPI 卡是现成例子（`api/demos.ts` `def-kpi`）。
3. **HTML 定义**（`kind: html`，`ui/extensions/HtmlBlockHost.tsx` + `htmlRuntime.ts` + `bridge.ts`）：同源 iframe（第二期 D16：隔离样式与崩溃，不是安全边界，Owner 是信任根），`window.aiws` v2 API：`bindings`、`input(name).rows/fields/markdown/props/bytes`、`resolve/outline/find`、`watch`、`table(name).upsert/setColumn`、`record(name).set`、`text(name).setMarkdown`、`createBlock`、`batch`、`upload`、`snapshot`、`notify`。编辑与查看模式默认显示上次报告的静态快照，显式激活才挂载。

**定义实体**（内核 `types.rs` `validate_block_def`）：payload 键只允许 `def_id`（Renderer id 格式）、`version`（默认 1）、`kind`、`title`、`description`、`accepts`、`allow_no_source`、`default_size`、`declarative`、`html { html, css?, js?, api_version? }`、`config_schema`、`actions`、`inspector`。Cell 通过 `def_ref` 引用定义，引用阻止删除；定义随导出包、Fork、离线副本携带。许愿格的 `aiws.result.html` 已经会把 HTML 产物落成一个 block-def（`def_id = wish.<hash>`）加一个 Cell（`store/src/wish/plan.rs`）。

**插入目录对扩展的处理**（`ui/canvas/catalog.ts`、`InsertCatalog.tsx`、`ObjectToolbar.tsx`）：`extensionEntries()` 扫 outline 里可读的 `buckyos.block-def`，以 `def_kind` 找宿主 Block（`declarative` / `html`），进"扩展"分组；左侧对象工具栏显示用户固定的扩展（`user_state` `ui:pinned-defs`）和"工作区扩展"入口；数据源模式对 block-def 只有详情页（`DataDetail.tsx` `BlockDefDetail`：kind、accepts、HTML 风险提示、原文折叠）。

### 2.2 Block-control（路线二的基础）

**容器定义**（`ui/controls/registry.ts` `ContainerDefinition`），对照 Bolt control 的五块（脑图方案 §6.2）：

| Bolt | `ContainerDefinition` | 脑图里的落点 |
| --- | --- | --- |
| `attr_def` | `configSchema` | direction / edge / spacing / branch_colors |
| `objtemplate/children` | `children: { accepts, containers, slots? }` | `any`、允许嵌套 control、无 slot |
| 子对象位置公式 | `layout: { direction, algorithm, solve }` | `tree`，`solve` 来自共享模块 `@controls/solve` |
| 皮与边 | `Chrome`（画布）、`Flow`（流式页整体渲染） | 隐式边、折叠柄、pin 标记、嵌套框；`FlowMindmap` |
| `method_def` + `eventlist` | `create`、`toolbar`、`keys`、`Inspector`、`describe` | 插入建容器 + 根；Tab / Enter / 方向键 / Ctrl+/ / Ctrl+D |
| 接口说明 | `agent: ControlDescriptor` | 共享模块 `controls/mindmap.ts` 的 Markdown 自述 |

**共享模块** `src/frame/aiworkspace/controls/`（浏览器与 Deno 同用，无依赖）：`contract.ts`（`CONTROL_KINDS`、`TEXT_VIEWS`，与 `core.json` 一致）、`placement.ts`（`"auto"`）、`tree.ts`（四方向树布局、折叠、pin）、`measure.ts`（文字测量，Deno 侧用等宽近似）、`solve.ts`（从 outline 解出一个 control，嵌套自底向上）、`edges.ts`、`outline.ts` / `opml.ts`（大纲互转）、`svg.ts`（静态图）、`drawio.ts`（物化）、`catalog.ts` + `mindmap.ts`（目录与自述，生成 `aiws/controls.json`）。

**内核**（`core/src/{types,plan,read,model}.rs`，`controls/core.json` 经 `include_str!` 共享）：`control_kinds` 列表决定哪些容器 kind 是 control；`validate_container` 校验 `layout`（隐含 `{ mode: "tree" }`）与 `config`；`child_allowed` 允许 control 内的 Cell 再带子 Cell；`check_placement` 允许 `"auto"` 分量（`x`/`y` 只在 control 直接子节点，`w`/`h` 只给 control 与 `text_views`）；`slot` 同父唯一；outline 投影 `config` / `slot` / `side` / `branch_color`；PROTOCOL 1.1。

**Desktop 接入点**：`layout.ts`（`Laid.control / node / origin`，`placeControl`）、`useSurfaceLayout.ts`、`RenderHost.tsx`（Chrome、节点拖动与预览、折叠柄、手柄规则）、`CanvasView.tsx`（键盘接管、工具栏、重挂、粘贴、大纲文件导入）、`clipboard.ts`、`surfaceOps.ts`、`flowOps.ts`、`FlowSurface.tsx`、`drawioExport.ts`、`state/outline.ts`、`presentation/*`。grep 到 26 个文件引用 `controlRegistry` / `isControlKind` / `@controls/*`，**但其中绝大多数是按 `isControlKind` 与 `controlRegistry.get(kind)` 泛化的**：新增一个 control 只需注册，不必再改这些文件（脑图实施 M2 就是这样接的）。

**服务与 Agent**：许愿格通用 `control` 结果（`aiws.result.control` / 模型 `put_result`，`kind` 必须在 `control_kinds` 里，支持 `under` 扩展某节点）；`control.list / describe / import / export`；websdk CLI `buckyos aiws controls list|describe`、`import`、`export`；手册只列目录，Agent 需要时再取自述（渐进式披露）。DV 已证明 Agent 只凭手册能在指定画布建图并扩展分支。

### 2.3 两层的关系

- control 的节点就是 Block，经 `BlockRegistry` 解析；节点可以是扩展 Block（declarative / html），也可以是另一个 control（R24）。
- control 不转发任何东西：选择、批注、锁、连接线绑定、原地改字都落在节点 Block 上。
- 非文字节点的 `w`/`h` 必须是数值（`check_placement`）；扩展 Block 作为节点时同样如此，因为只有 `text_views` 的尺寸能从 `title` 算出。
- Block 不能自己声明 `auto`，也不能按 id 引用兄弟（脑图方案 §5.3）。HTML Block 的 `createBlock` 只能在自己旁边建兄弟 Block，不能在某个 control 下建子节点。

## 3. 路线一：扩展 Block，核心是实现 render-block

### 3.1 定位

路线一回答"这一格里画什么"。它适合：**内容由绑定数据决定、自身是一个整体、不需要排布兄弟**的对象。判断标准：能否用"一个 Cell + 一个或几个数据绑定"描述完整？能就是 Block。

render-block 是这条路线的全部核心：给定 `RenderContext`，产出视图；配置、动作、工具栏、Inspector 都是围绕它的声明。它必须遵守的三条（第二期 §9.3、§10.2）：

1. **静态优先**：`Static` 必需，无媒体播放、无代码、不执行许愿格；小尺寸时宿主用 `Simplified` 或占位。
2. **模式正交**：不给 `View` / `Editor` 就回退到 `Static`，绝不继承另一个模式的写入行为。
3. **写入经宿主**：所有写都是 Operation 经 `WorkspaceStore.submit`，复用权限、锁、撤销、保存状态。

### 3.2 三档的能力边界

| 能力 | 代码注册 | `declarative` 定义 | `html` 定义 |
| --- | --- | --- | --- |
| 发布方式 | 随 Desktop 发版 | 随文档（数据树实体） | 随文档 |
| 谁能产出 | 开发者 | 人、Agent、许愿格（可规划） | 人、Code Agent、许愿格（`aiws.result.html` 已有） |
| 渲染能力 | 任意 React | 六种 item、两种布局 | 任意 HTML / CSS / JS，可联网 |
| 数据绑定 | 任意 | 一个 `source`（record / table） | 多个命名 `bindings`（input / result） |
| 编辑器 / 工具栏 / Inspector | 全部 | inspector 字段、三个固定动作 | 自己画；工具栏项直到声明 `tools` 前显示为标签 |
| 连接线锚点 `shape` / `anchors` | 可定义 | 宿主默认 | 宿主默认 |
| 流式页 | 可定义 | 同画布 | 快照 / 激活 |
| 离线 | 是 | 是 | 是（快照） |
| 手机 | 是 | 是 | 只显示快照 |
| 性能成本 | 自报 | 低 | `cost.html`，受同时挂载上限 |
| 安全 | 受审代码 | 不执行代码 | 同源无沙箱（D16），Owner 承担 |

结论：**声明式是"安全的小词汇"，HTML 是"能力的大词汇"**。不要把 declarative 扩成布局引擎或表达式语言（脑图方案 §5.7 的边界同样适用于这里）；需要更强表现时直接走 HTML。

### 3.3 缺口与建议

| # | 缺口（现状） | 建议 |
| --- | --- | --- |
| B1 | block-def 没有**目录元数据与自述**：`validate_block_def` 的键里没有 icon / group / when / agent；插入目录里扩展只能显示标题与 `description`，Agent 也没有像 control 那样的 `describe` 可读 | 给 `BlockDefPayload` 加 `catalog { group?, icon?, when?, standard? }` 与 `agent`（Markdown 自述，对齐 `ControlDescriptor` 的 summary / when / formats / agent）；内核 `check_keys` 放开，大小计入 64 KiB；`aiws controls` 之外加 `aiws blocks list|describe`（读工作区 block-def，经服务），或并入统一的 `aiws catalog` |
| B2 | HTML `createBlock` **只能建兄弟** | 加 `parent` 参数（`under: <binding 或 entity_id>`），允许在 control 节点下创建子 Block（placement 写 `auto`），与 `aiws.result.control` 的 `under` 同语义；内核已允许 control 内 Cell 带子 Cell |
| B3 | 导入的 HTML 定义**没有快照**，编辑 / 查看模式只能显示占位 | 导入向导（§6.2）末尾"运行一次生成快照"；定义包（§6.2）可带 `snapshot.png` 作初始快照 |
| B4 | 定义**更新策略**不明：同 `def_id` 新版本是新实体还是原地改？Cell `view.version` 跟谁走 | 定下来：原地改定义 = 所有 Cell 一起变（Bolt 的模板语义）；要并存就用新 `def_id`。导入遇到同 `def_id` 时让用户选"替换 / 另存为新定义 / 取消"（§6.2 第 3 步） |
| B5 | 扩展 Block 作为脑图节点**不能 auto 尺寸** | 接受。手册与自述里已写"非文字节点要给尺寸"，block-def 的 `default_size` 就是节点尺寸 |
| B6 | declarative **词汇与数据源种类**少（record / table） | 不扩布局；可按需加 item 种类（sparkline、progress、image）和 richtext / asset 绑定；每加一种都是解释器改动，随 Desktop 发版 |
| B7 | 没有 **block-def 文件格式**与导入 / 导出 | 定义 `*.block.json`（单个 block-def payload）与 `*.block.zip`（payload + README + snapshot + 示例数据），见 §6.2 |

### 3.4 落点

| 层 | 文件 |
| --- | --- |
| 内核 | `core/src/types.rs` `validate_block_def`（新键）、`schemas/*.json`、README |
| 共享模块 | 新 `controls/blocks.ts`（block-def 文件格式的解析 / 校验，浏览器与 Deno 同用；名字待定，见 Q4） |
| Desktop | `api/types.ts` `BlockDefPayload`、`ui/canvas/catalog.ts` `extensionEntries`（读 `catalog` 元数据）、`InsertCatalog.tsx`（导入按钮、详情）、`ui/sources/DataDetail.tsx`（更新 / 导出 / 被谁使用）、`ui/extensions/bridge.ts` `createBlock`（`parent`）、i18n |
| 服务 | `block.import / export`（或并入 `control.*` 为 `catalog.*`）、许愿格 `aiws.result.html` 可带 `catalog` |
| CLI / 手册 | `aiws import` 按扩展名分派：`.md` / `.opml` → control，`.block.json` / `.block.zip` → block-def；`aiworkspace_template_dev.md` 加一条 |

## 4. 路线二：基于已有 Block，实现 Block-control

### 4.1 定位

路线二回答"一组 Block 怎么排、怎么连、怎么操作"。它适合：**结构是 UI 的、内容绑数据**（脑图方案 §4.3）——节点可以是任意 Block，结构只在 BlockTree，数据源模式不显示结构。判断标准：去掉子节点后"它"还剩什么？剩的只是布局规则、边和键盘行为，就是 control。

一个 control 的构成，按"必须 / 可选"：

| 部分 | 必须 | 说明 |
| --- | --- | --- |
| `kind`（容器 kind = control id） | 是 | 要进 `controls/core.json` 的 `control_kinds`，内核与 Desktop 同读 |
| `ControlDescriptor`（共享模块） | 是 | summary / when / formats / agent 自述；进 `catalog.ts` 的 `CONTROLS` |
| `layout.solve` | 是 | 纯函数，只读 outline 子树与 config，只填 `auto` 分量，单向（§5.7） |
| `create` | 是 | 容器 + 初始内容子节点，一次提交 |
| `Chrome` | 推荐 | 隐式边、外框、模板子节点（R23）；没有就只有子 Block |
| `toolbar` / `keys` / `Inspector` | 可选 | 容器级与节点级 |
| `Flow` | 可选 | 流式页整体渲染；缺省按 group |
| 专用节点 Block | 可选 | 脑图加了 `mindnode`；多数 control 直接用已有 Block（便签、形状、卡片、扩展 Block）作节点 |
| 交换格式 | 可选 | `control.import / export` 的解析 / 格式化函数 |
| 内核 config 校验 | 是 | 现为 mindmap 硬编码，见 K1 |

### 4.2 实现第二个 control 要碰的层（现状）

按脑图 M0–M4 的实际路径，一个新 control 现在要做：

1. **共享模块**：`controls/<kind>.ts`（自述 + 布局）、`catalog.ts` 注册、`core.json` 加 kind、`deno run -A tools/build_catalog.ts` 重生成 `aiws/controls.json`、布局快照测试。
2. **内核**：因为 `core.json` 变了要重编译；`validate_container` 加该 kind 的 `config` 校验与隐含 `layout.mode`；`cargo test`；`bash wasm/build.sh` 重建 WASM（离线路径与 Desktop 内核同一份）。
3. **Desktop**：`ui/controls/<kind>/definition.tsx`（`ContainerDefinition`）、`Chrome.tsx`、可选 `Flow`、`registerControls.ts` 注册、i18n、CSS。`layout.ts` / `RenderHost` / `CanvasView` / 剪贴板 / 导出等不改。
4. **服务**：许愿格 `control` 结果与 `control.*` 自动认新 kind（都查 `is_control_kind`），但 `control.import / export` 的格式分派与 `wish` 的 children 解释目前都是"大纲 → 树"；非树形 control 要加自己的格式。
5. **CLI / 手册**：不改（目录驱动）。

四处里真正"重"的是 1 和 2：**改 `core.json` 等于发一次内核**。这就是 K1 / K4 要解的问题。

### 4.3 缺口与建议

| # | 缺口 | 建议 |
| --- | --- | --- |
| K1 | 内核 **config 校验 mindmap 专用**（`types.rs` `validate_container` 的 `match kind`） | 把每个 control 的 `config_schema`（JSON Schema）放进 `controls/core.json`（或 `controls/<kind>.json` 由 `core.json` 索引），内核用 schema 校验，Desktop `configSchema` 直接复用同一份。新 control 的内核改动收缩为"加一行 + 重编译" |
| K2 | **`layout.algorithm` 只有 `tree`** | 按需加具名算法，每个都遵守 §5.7（单向、只读自身数据、不组合）：`fixed`（固定相对坐标，Button 类）、`stack`（单列 / 单行堆叠，看板的一列）、`grid`（等宽列，看板整体 = grid 套 stack，靠 R24 嵌套而不是算法组合）、`timeline`（按节点 `config.at` 排一条轴） |
| K3 | **模板子节点**（R23）的 Chrome 接口没落地 | 随第一个需要"皮"的 control（Button / 卡片壳）落地：`Chrome` 收到 `slots` 解析结果，按定义绘制不落实体的部分；运行态放组件 state 或 `user_state` |
| K4 | **用户 / Agent 定义的 control 必须发版** | 不做数据态的 `control-def`（布局算法是代码，内核要认 kind）。改为提供一个内置通用 kind **`composite`**：`layout.algorithm = fixed`，`children.slots` 来自容器 `config.slots`（每个 slot 一个相对矩形与允许的 view），`Chrome` 画 `config.frame` 描述的皮。一份 `composite` 的 config 就是一个"用已有 Block 拼出来的新对象"，可以由许愿格 / Agent / 导入文件产出，不发版。这是路线二"基于已有 Block"最直接的体现 |
| K5 | **`control.import / export` 只认大纲** | 按 kind 分派：树形 control 共用 outline / OPML；`composite` 用 JSON（容器 config + 子 Block 列表）；格式列表写在 `ControlDescriptor.formats` 里，CLI 按它提示 |
| K6 | 自述与目录只在共享模块（代码） | 保持。目录是渐进式披露的第一层，必须稳定、可测试（`catalog_test.ts` 已校验 `controls.json` 是否最新）；`composite` 实例的"自述"是其 config 的 `description`，由 `control.describe <entity>` 读实例而非 kind |
| K7 | 文字测量两端有差（浏览器 canvas / Deno 等宽） | 已接受（脑图方案 §5.4）。新 control 的节点若非文字 Block 不受影响 |

### 4.4 落点

| 层 | 文件 |
| --- | --- |
| 共享模块 | `controls/core.json`（schema 索引）、`controls/<kind>.ts`、`controls/catalog.ts`、`controls/{fixed,stack,grid}.ts`（算法）、`controls/solve.ts`（按 `algorithm` 分派，现为 tree 直连）、`tools/build_catalog.ts` |
| 内核 | `core/src/types.rs` `validate_container`（schema 校验替代 match）、`plan.rs` `check_placement`（`fixed` 下 `x`/`y` 的 `auto` 语义不变）、`core/tests/<kind>.rs` + fixtures、`wasm/build.sh` |
| Desktop | `ui/controls/<kind>/*`、`ui/controls/registry.ts`（`layout.algorithm` 类型放开）、`registerControls.ts`、`ui/canvas/catalog.ts`（`composite` 实例作为可插入条目，见 §6.4） |
| 服务 | `server/src/control.rs`（格式分派）、`store/src/wish/results.rs` `control`（children 的非树解释） |
| CLI / 手册 | `aiws import --kind composite x.json`；手册目录自动带上 |

## 5. 选型：什么时候走哪条

| 需求 | 路线 | 理由 |
| --- | --- | --- |
| 一张图表 / 一张卡 / 一个嵌入页，数据来自一个或几个绑定 | 一（html 或 declarative） | 一格之内，不排兄弟 |
| 许愿格的程序产物要有交互 | 一（html，`aiws.result.html` 已有） | 同上 |
| 节点是便签 / 文字 / 图片，要自动排布并画边 | 二（tree / stack / grid） | 布局属于父容器 |
| 看板：列 + 卡片，卡片可拖到别的列 | 二（grid 套 stack，卡片是任意 Block） | 结构是 UI 的，卡片内容各自绑数据 |
| 一个"按钮"：背景 + 文字 + 图标，点击触发动作 | 二（`composite` + slots）或一（html） | 子对象要能单独选中、改字、绑数据时选二；只要外观时选一 |
| 要让 Agent / 许愿格直接产出，不发版 | 一（block-def）或二（`composite` 实例） | 两者都是数据态 |
| 要新的布局算法 | 二，且必须发版 | 算法是代码，内核要认 kind |

一条反例：**不要**用 HTML Block 自己画一棵树然后在里面排子项——那是把 control 做进了 Block，失去了选择、批注、锁、连接线、Agent 自述，也违反"Block 不排兄弟"。需要排布就做 control，哪怕节点只是普通文字。

## 6. UI：用户如何插入 / 导入扩展 Block

### 6.1 现状

- **插入目录**（主菜单"插入"、左侧"＋全部对象"、空白处右键）：八个页签 全部 / 文字 / 数据 / 版式 / AI / 样本 / **扩展** / 已有数据；顶部搜索；详情页按 `needs` 引导选数据或选文件；顶部有"导入 draw.io…"、"导入文件"、"导入文件夹"按钮。
- **扩展页签**只列当前工作区可读的 `buckyos.block-def`；空态文案"这个工作区还没有 Block 定义（扩展）。可在数据源中导入或由许愿格生成"——**前半句没有对应功能**（Desktop 没有任何导入 block-def 的代码；数据源模式对 block-def 只有详情页）。
- **左侧对象工具栏**：标准对象（含脑图，`STANDARD_ORDER` 里有 `mindmap`）→ 用户固定的扩展（`ui:pinned-defs`）→ "工作区扩展"入口 → "＋全部对象"。
- **control 的插入**：目录 `layout` 分组里的 `control:<kind>` 条目，插入即建容器 + 根并进入编辑；把 `.md` / `.opml` 文件拖到画布或经"导入文件"按钮会直接变成脑图（`CanvasView.handleFiles` → `importOutline`）；选中富文本里的嵌套列表可"转成脑图"。
- **跨工作区**：导出包（`share` / `personal_backup`）携带 block-def；"从模板创建工作区"会带上模板里的定义（KPI 卡）；"把模板添加到当前工作区"是 UI 改进 §6.2 列明的未做项。

### 6.2 导入扩展到工作区（新增）

**文件格式**（B7）：

- `*.block.json`：一个 block-def 的 payload（`def_id`、`kind`、`title`、`description`、`accepts`、`default_size`、`declarative` 或 `html`、`config_schema`、`catalog`、`agent`）。用内核同一份 `validate_block_def` 校验（浏览器走 WASM，与离线提交同路径）。
- `*.block.zip`：`block.json` + `README.md`（给人读）+ `snapshot.png`（初始快照）+ 可选 `sample/`（示例数据，一份 `operations.json`）。首版可以只做 `.block.json`。
- `*.control.json`（路线二的 `composite` 实例，K4）：容器 config + 子 Block 列表，经 `control.import --kind composite`。

**入口三处**，共用一个导入向导：

1. 插入目录顶部"导入"组加"**导入扩展…**"，与"导入 draw.io…"并列；扩展页签空态的文案改成按钮"导入扩展…"。
2. 数据源模式左栏"＋"菜单加"**导入 Block 定义**"；详情页加"更新定义…"（同一向导，预选"替换"）。
3. **拖放**：把 `.block.json` / `.block.zip` 拖到画布，`handleFiles` 按扩展名分派（与 `.md` / `.opml` → 脑图、`.drawio` → 导入同一处判定）。拖到画布的额外含义是"导入后立即在落点插入一个实例"。

**向导步骤**：

| 步 | 内容 | 失败处理 |
| --- | --- | --- |
| 1 解析 | 读文件，`validate_block_def`；HTML 定义检查 `api_version` 是否被本宿主支持 | 列出错误码与位置，不落任何实体 |
| 2 预览 | 名称、版本、kind、接受的数据类型、默认尺寸、`description`；HTML 定义显示 D16 风险提示（现有文案 `defHtmlRisk`）与代码体积；有快照显示快照 | — |
| 3 冲突 | 同 `def_id` 已存在：**替换**（原地改 payload，所有使用它的 Block 一起变，B4）/ **另存为新定义**（改 `def_id` 后缀）/ 取消 | 只读工作区或无 `structure` 能力：按钮禁用并说明原因（与插入同策略） |
| 4 落点 | 默认 `data/` 根（与许愿格生成的定义一致）；可选放入 `data/extensions/` 文件夹（首次创建） | — |
| 5 完成 | 一次 `doc.commit`：`entity.create buckyos.block-def`（+ 快照资产 `asset.import_object`）；勾选"立即插入到画布"则同一提交里按 `needs` 走插入目录的详情步骤（选数据 / 放置） | 提交失败按现有保存异常流程 |

**导入即一次提交**，可撤销；离线时与其它提交一样进 WASM 副本并在恢复后提交。

### 6.3 插入已在工作区的扩展

保持现有路径，补三件事：

- 扩展条目显示 `catalog.icon`（B1）而不是统一的拼图图标；详情页显示 `when`（何时用）与 `agent` 摘要的第一段。
- 详情页"使用此定义的 Block：n 个"，点击在画布 / 数据源里定位（`def_ref` 已有反向信息）。
- 固定到左侧工具栏（已有 `ui:pinned-defs`）在详情页加开关，不只在工具栏右键。

### 6.4 control 与 `composite` 实例的插入

- 新注册的 control 自动进目录 `layout` 分组与标准对象排序表（`STANDARD_ORDER` 需要手工加名字才会靠前）。
- `composite` 实例（K4）作为"可插入的扩展"出现在**扩展页签**：来源是工作区里标记为模板的 `composite` 容器（`config.template = true`，放在 `data/extensions/` 下不显示在画布），插入 = 复制该容器与其子 Block（`clipboard.ts` 的子树复制已能做）。这样"把几个已有 Block 拼成一个新对象再复用"的用户路径是：画布上摆好 → 选中 → 右键"组合为控件…" → 命名、设 slot → 保存为模板 → 出现在扩展页签。
- 文件导入统一走 §6.2 的向导：`.md` / `.opml` → 脑图（已有）；`.control.json` → `composite`；`.block.json` → block-def。

### 6.5 管理与只读

- 数据源模式 block-def 详情页：更新定义、导出 `.block.json`、被谁使用、删除（有 `def_ref` 时拒绝并列出引用者，现有规则）。
- 查看模式、无写权限、服务停止同步：导入与插入一起禁用，工具提示说明原因（UI 改进 §7.2）。
- 手机：不提供导入；HTML 扩展只显示快照。
- 发布页 / 访客：HTML 定义以 Owner 的会话运行是已知风险（第二期 §15 R1、第四期 §4），导入向导的风险提示要指出"访客打开也会运行"。

### 6.6 Agent 与许愿格的对应入口

- Agent：`buckyos aiws import x.block.json --workspace <ws>`（分派到 `block.import`）、`buckyos aiws blocks list|describe <def_id>`（B1）；手册 `aiworkspace_template_dev.md` 加一条"扩展 Block"，与"Block-controls"条目并列。
- 许愿格：`aiws.result.html` 已产出 block-def + Cell；补 `catalog` / `agent` 字段的透传，使许愿格产出的扩展在目录里有图标与说明，并且能被后续 wish 以 `describe` 读到。

## 7. 待裁决项

| # | 问题 | 建议 |
| --- | --- | --- |
| Q1 | block-def 更新语义：原地替换还是新实体（B4） | 原地替换（Bolt 模板语义），并存用新 `def_id` |
| Q2 | 是否做 `composite` 通用 control（K4），还是每个组合控件都发版 | 做；它是"基于已有 Block"路线不发版的唯一办法 |
| Q3 | 内核 config 校验改为 JSON Schema（K1）是否接受增加 Rust 侧 schema 校验依赖 | 接受；block-def 的 `config_schema` 前端已用 `z.fromJSONSchema`，内核统一后两端一份 |
| Q4 | block-def 文件格式解析放共享模块 `controls/` 还是新目录 `blocks/` | 新目录 `src/frame/aiworkspace/blocks/`，`controls/` 保持只管 control |
| Q5 | 导入扩展的默认落点：`data/` 根还是 `data/extensions/` | `data/extensions/`，许愿格生成的定义仍随结果组 |
| Q6 | `aiws` CLI 动词：`blocks list|describe` 单独，还是与 `controls` 合为 `catalog` | 单独；两者读的来源不同（control 读代码目录，block 读工作区） |
| Q7 | 首版 `*.block.zip` 是否做 | 不做，先 `.block.json`；快照用"导入后运行一次"补 |
| Q8 | HTML `createBlock` 的 `parent` 是否允许指向其它 control | 允许，受 `child_allowed` 与 `tree.move` 规则约束 |

## 8. 建议的实施顺序

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| P0 | §6.2 导入向导（`.block.json`）+ 扩展页签 / 数据源 / 拖放三处入口 + 详情页更新与导出；B1 的 `catalog` / `agent` 字段（内核键放开） | 导入季度经营分析的 KPI 卡定义到空工作区并插入；同 `def_id` 二次导入走三选一；Playwright 覆盖三处入口与只读禁用 |
| P1 | K1 内核 config schema 化；K2 `fixed` / `stack` 算法；K4 `composite` control 与"组合为控件"右键；§6.4 实例进扩展页签 | 用便签 + 图片 + 文字拼一个卡片控件，保存为模板后插入两次，改模板不影响已插入实例；许愿格 `control` 结果产出一个 `composite` |
| P2 | B2 `createBlock(parent)`；K5 格式分派与 `.control.json`；CLI `blocks` 动词与手册；`.block.zip` 与快照 | Agent 只凭手册把一个 HTML 定义导入并在脑图节点下创建子 Block |

## 9. 参考

- [脑图对象方案讨论](<脑图对象方案讨论.md>) v0.4：§4.3、§5.7、§6.2、§8.1、R21–R24。
- [脑图实施记录与验收报告](<脑图实施记录与验收报告.md>)：§2 交付物（文件落点）、§6 未完成、§8 DV。
- [第二期规划](<BuckyOS AI Workspace 第二期规划.md>) §10：两个扩展点、模式正交、Block 定义实体、HTML 扩展（D16）、扩展样本。
- [UI 改进](<BuckyOS AI Workspace UI改进.md>) §6.2–§6.3、§7.1–§7.2：新建 / 插入对话框、左侧工具栏、插入状态。
- 源码：`src/frame/desktop/src/app/aiworkspace/ui/{blocks/registry.ts,controls/registry.ts,canvas/catalog.ts,canvas/InsertCatalog.tsx,extensions/*}`；`src/frame/aiworkspace/{controls/,core/src/types.rs,README.md}`；`buckyos-websdk/cli/modules/aiws.ts`。
