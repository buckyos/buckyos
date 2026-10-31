# BuckyOS AI Workspace 扩展实现思路：控件库、控件定义与两条实现路线

> 状态：v0.9，2026-10-30。**P1 已实施**（2026-10-30 当日：内核 Q15 拆层、composite、`def_dep`、`tree.clone`、服务 `ext.pack` 与依赖闭包、websdk / CLI；Desktop 宿主与 DV 记录见实施记录 P1 节），见 §8 P1 行。**P0 已实施**（已由用户提交 `63765d12d`，真实 Zone DV 两例通过），落点、测试与实现中的出入见 [扩展控件实施记录与验收报告](<BuckyOS AI Workspace 扩展控件实施记录与验收报告.md>) 与 [任务单 §12.1](<../../notepads/aiworkspace-extension-p0-todo.md>)。本版按实际实现调整后续计划：§1.5 改写为 P0 后的差距；§3.3 / §4.3 / §7.2 / §7.3 / §7.4.3 / §7.5 标注各缺口、裁决项与风险的现状；§6.1 / §6.2 / §6.6 / §6.7 / §6.8 补“P0 实现”说明；新增 §6.10 典型开发流程 demo 与 [hello-controls/](<hello-controls/README.md>) 例子目录（home 文件内联提交、源码目录平铺、Desktop 不打 PIKG、CLI 动词）；新增 §7.6 P0 后的新裁决项 Q20–Q25（**用户同日裁决**：Q20 服务端打包、Q21–Q23 按建议、Q24 放开独立域名开发操作、Q25 不做，已回写 §1.5 / §3.3 / §6.8 / §6.10 / §7.5.4 / §8 与 hello-controls；P1 任务单已写：[notepads/aiworkspace-extension-p1-todo.md](<../../notepads/aiworkspace-extension-p1-todo.md>)）；§8 重写 P0 行、增补“P0 遗留”并按实现重排 P1–P3。历史：v0.6 增补 §7.3 与 Q14–Q17；v0.7 增补 §7.4 开工前裁决清单；用户在 §7.4.1 裁决 A1–A7，§7.5 评估影响；v0.8 按用户确认的“编辑 → build 原子部署”模型改写 D6、§1、§1.3、§1.5、§3.3、§3.4、§6.5–§6.8、Q11、Q14、§7.3、§8、§9。以“多人协作的 Delphi，可以安装控件包”为产品模型；开发者在 home 目录编辑控件源码、build 进自己的 Workspace 调试，再多选控件导出为全局可用或 PIKG 包，后续签名、发布与传播。两级控件库见 §1.1，开发 / 调试见 §6.7，导出见 §6.8，PIKG 控件包与 Workspace 模板包预留见 §6.9，已确认边界与待定细节见 §7，对数据逻辑内核的接触点见 §7.3，开工前裁决清单见 §7.4。源码基线为脑图实施提交 `8f30ad030` 之后的工作区加 P0 未提交集（2026-10-30 复核）。P0 交付的部分以实施记录与 README 为准；P1 以后仍是设计方向，不构成已实施记录。
>
> §2 与标为“现状”的段落描述当前源码；§1、§3–§8 中 P0 已交付的部分（render 定义、`ext.*` 方法、build / pull / watch、本机库、`aiws` 包、引入与实例化）已是现状，权威契约在 aiworkspace README “Extension controls” 与 `schemas/{component,aiws-manifest,block-def}.schema.json`，本文与其不一致处以 README 为准；其余（组合控件、事件、来源升级、模板）仍是目标方案。PIKG 已有的包管理能力与待实现的 AIWS 本机控件目录接入分别说明。

## 1. 结论

AI Workspace 按**多人协作的可视化设计器**组织扩展：用户从自己的 BuckyOS 本机控件库选择控件，引入 Workspace，再共同创建、配置和运行控件实例。设计主线是 **PIKG 分发 → 本机控件库 → Workspace 控件定义 → 控件实例 → 协作运行**。

开发者的标准循环是工程师熟悉的编辑 → build：**在自己 home 目录的源码文件夹里用任意编辑器修改控件源码 → `build` 同步校验并原子提交进自己的 Workspace，实例即时更新 → 在 Workspace 选择一个或多个控件导出 → 全局可用，或得到 `.pikg` → 后续签名、发布与传播**。日常调试不要求打包、重新引入或重新插入实例；build 失败不提交，Workspace 保持上一个可用版本；导出时才冻结供复用的版本（§6.7、§7.5）。

PIKG 需要正式支持 Workspace 扩展内容，并为模板包保留共同结构：**Workspace 模板包 = 控件包的公共内容 + 工作区结构 / 实例、初始数据与参数**。首期交付控件包，在包清单、类型识别和消费方分派中预留模板入口；模板创建功能可后续实现。

### 1.1 两级控件库（已确认）

| 层次 | 职责 | 变化影响范围 |
| --- | --- | --- |
| BuckyOS 本机控件库 | BuckyOS 通过 PIKG 安装“本机可以插入的所有可用 AIWS 控件”；也接收从开发 Workspace 显式导出为“全局可用”的控件包，提供可发现、可选择的控件及资源 | 该 BuckyOS 的可用控件来源 |
| Workspace 控件集合 | 有相应 Workspace 权限的人，从各自本机控件库选择并引入控件定义及必要依赖；集合属于工作区 | 该 Workspace 的所有协作者 |
| 控件实例 | 从 Workspace 中的定义创建，保存实例属性、数据绑定和内容子节点 | 工作区中的一个具体对象 |

例如：甲的 BuckyOS 有一组控件，甲选 A、B 引入 Workspace；乙从自己的 BuckyOS 选 C、D 引入同一 Workspace，工作区得到 A、B、C、D。甲乙随后都能使用这四个控件。丙即使没有通过 PIKG 安装这些控件包，也能在支持相应宿主 API 的客户端使用工作区已经拥有的控件。

这里的“本机”是当前用户所连接、提供控件来源的 BuckyOS；浏览器客户端与 Workspace 所在服务可以在别处。不能只枚举 Workspace 服务所在机器的控件库来代替各协作者的来源。

PIKG 负责分发与本机安装，AIWS 负责选择性引入与工作区内管理。有相应工作区权限的人都可引入，入口不限定 Owner，也不以操作 Workspace 所在机器的包管理权限为前提。本机包升级、卸载不会自动改变已引入的工作区控件；从本机库升级工作区内的包来源版本需要显式操作。修改控件源码走"home 目录编辑 → build"的开发流程（§6.7）。

“全局可用”指控件进入当前用户 BuckyOS 的本机控件库，供其他 Workspace 选择引入。开发者在 Workspace 多选控件后，可以导出并安装到该库，也可以只生成 `.pikg` 文件；两者共用同一份包内容（§6.8）。显式导出才更新本机库，后续 build 仍只影响开发 Workspace，已引入它的其他 Workspace 不自动升级。

### 1.2 同一控件体系，两条实现路线

**Block 决定一格之内画什么，Block-control 决定一组 Block 怎么排、怎么连、怎么操作。** 两条路线共享定义身份、属性、绑定、目录、自述和更新规则，分别提供渲染与组合实现。下表的代码接口是已有基础，工作区组合定义是本方案新增部分。

| 路线 | 核心 | 扩展的是什么 | 产物形态 | 典型例子 |
| --- | --- | --- | --- | --- |
| **路线一：扩展 Block** | **render-block**：`(RenderContext) → 视图`，即 `BlockDefinition.Static`（及其 View / Editor 变体） | 一个新的表现：给定 Cell、绑定数据、模式与尺寸，画出什么 | 代码注册的 `BlockDefinition`，或工作区里的 `buckyos.block-def` 实体（`declarative` / `html`） | KPI 卡、图表、日历、嵌入第三方可视化、许愿格 HTML 结果 |
| **路线二：Block-control** | **solve + Chrome**：`(outline 子树, config) → 子节点的 auto 分量` 和 `(Laid) → 容器级外观` | 一种新的组合方式：把已有 Block 当节点，容器拥有布局、隐式边、键盘与工具栏 | 已有代码注册的 `ContainerDefinition`；目标新增工作区组合定义，实例由通用 `composite` 宿主承载 | 脑图（已实现）、组织结构图、看板、时间线、Button 类组合控件 |

两条路线的分界只有一个问题：**新东西需不需要自己排布一组子 Block？** 不需要就是 Block（哪怕它内部很复杂，用 HTML 画），需要就是 control（哪怕节点只是普通文字）。control 不画节点，节点永远是 Block；Block 不排兄弟，排布永远属于父容器（脑图方案 §5.2 "布局由父容器拥有"）。

新建一个组合控件类型，可以复用已有布局算法。`composite` 是这种定义的实现方式；每个客户卡片、按钮都不应成为内核编译期的一个新 kind。首版复用已有或本期新增的具名算法，安装任意新算法代码的运行时契约另行设计。

### 1.3 定义、实例、片段与 Workspace 模板

| 对象 | 保存的内容 | 关系 |
| --- | --- | --- |
| 控件定义 | 稳定身份、来源版本、build 修订与 `derived.build` 摘要、build 通过的源码快照（`buckyos.dir`）与实现资源；公开接口及运行实现由该快照解析得到 | 存在 Workspace 的数据树，由实例引用；编辑入口是开发者 home 目录中的工作副本，`pull` / `build` 在两者间同步（§6.7） |
| 控件实例 | 定义引用、实例属性覆盖、命名数据绑定、事件绑定、位置和用户内容 | 存在 BlockTree；复制实例时分配新身份，保留定义关系 |
| 可复用片段 | 一组对象及其初始化内容的可复制描述 | 插入后独立，不承担控件类型与统一升级的职责 |
| Workspace 模板 | 可复用的工作区结构 / 实例图、初始数据、参数及其使用的控件集合 | 模板包复用控件包结构；创建 Workspace 时引入定义并实例化，包安装本身不创建工作区 |

“定义为控件”与“保存为片段”是不同操作。组合控件的模板外观来自定义，背景、焦点框等按 R23 由 Chrome 绘制；用户内容 slots 中的 Block 是实际文档实体。定义保存在 `data/extensions/`，实际 control 容器与 Cell 仍只能放在 BlockTree，不能把标记为模板的容器直接放进数据树。

引入时记录控件的稳定身份、来源版本与摘要、源码 / 实现资源及依赖；文件导入与工作区内生成也有各自来源。类型身份需要命名空间，显示名称不参与去重；Workspace 实体 ID 是本地引用身份。包是分发单位，可以选其中几个控件，但必须带齐其依赖闭包。工作区保留 build 通过的源码快照与实现，后续打开、Fork、导出、协作者 `pull` 都不依赖贡献者的机器仍在线。外部业务数据与联网服务仍遵守自身绑定和可用性规则。

普通外观与行为修改在 build 通过后原子更新当前 Workspace 中引用该定义的实例，保留实例属性覆盖、绑定和用户内容。改变 `kind`、必填属性、绑定或内容结构时，源码仍可保存在 home，但 build 列出受影响实例并整次失败，须显式处理迁移。新默认值只影响未被实例覆盖的属性。

区分宿主 API / Renderer 版本、来源包 / 控件发布版本和工作区 build 修订。build 不要求递增来源发布版本；它产生新的工作区修订并标记本地改动，不修改上游 PIKG 产物。重新引入或升级来源包不能静默覆盖这些改动。发布版本并存策略见 Q1，build 的校验与原子性见 §6.7。

### 1.4 设计器与协作运行契约

控件公开接口应可供人和 Agent 同时使用：属性有类型、默认值、编辑方式；绑定有名称、接受的数据类型及必填规则；动作 / 事件有参数和可绑定的宿主操作。例如按钮暴露 `text`、`enabled` 和 `click`，实例把 `click` 绑定到一个动作或程序。工具栏、快捷键是设计器入口，不能完整替代这个接口。

设计态点击用于选择、移动和配置；运行态点击触发业务行为。“设计 / 运行”与当前 `edit / view / show`、读写权限分别建模，运行能力不能仅由画布模式推断。公共端及回放的规则见 §6.5。

- **共享状态**：实例属性、绑定、内容和业务结果经宿主提交。目标是不同属性可独立合并、同一属性检测冲突；当前整块 `config` 的版本检查尚不足以满足这一点。个人撤销也应保留他人的无关修改。
- **个人状态**：选择、hover、按下、焦点等放本地状态；需要记忆的个人偏好放 `user_state`，不污染共享实例。
- **业务执行**：渲染、布局和订阅更新可在各客户端运行；一次点击以当前操作者身份发起动作，需要共享的执行复用 command / run 记录与幂等机制。其他端观察结果，不能因挂载或收到同步事件而重复产生副作用。

### 1.5 当前差距（P0 后，2026-10-30）

- **路线一**：P0 已交付本机控件库、“工作区控件”管理与引入入口、实例初始化表单（`config_schema` 子集 + 命名 `bindings`）、declarative 命名绑定；定义可由 home 源码 build、从库引入、许愿格或 Agent `doc.commit` 写入。尚缺：HTML `parent`、事件 / 动作绑定、来源包升级与结构迁移（今天同身份异来源一律 `EXT_IDENT_EXISTS`）、Desktop 产出 `.pikg`（§7.6 Q20 裁决：服务端 `ext.pack`，P1）。
- **路线二**：未动。内核 `kind` 仍是闭集 `declarative | html`（`composite` 只在 `component.json` schema 里留位），`control_kinds` 仍是编译期表，`types.rs` 校验入口未按 D01 方案 C 拆层（Q15），`deps` 必须为空（定义 → 定义依赖边未加，Q16）。组合定义是 P1 的主体。**P1 后**：已按 §8 P1 行落地（`kind: composite` + 通用宿主 kind `composite`、`types.rs` 拆为 `validate_primitive` / `validate_schema`、`def_dep` 强边、`tree.clone`），见实施记录 P1 节。
- **共同缺口**：模板包只预留（清单 `workspace_templates` 非空即拒绝）；签名 / 发布闭环未接；属性级协作与撤销、设计 / 运行态、公共端激活规则未做；Agent 凭手册建控件、跨 Zone 中转引入、旧 `manifest_version` 包三项 DV 未跑。
- **开发闭环**：已跑通（build / pull / watch、诊断到行列、基线冲突、实例兼容检查、`unchanged` 不提交）。实现与设计假设的三处出入要在后续设计里承接：home 文件在 named store 里没有对象，build 由客户端读字节内联（≤ 4 MiB / 次，§7.6 Q22）；源码目录只能平铺（Q23，P1 放开成员名含 `/`）；工作区独立域名上 Desktop 关闭了构建类操作，但网关按路径转发 `/nfs/v1` 且子域会话是用户态 token，Q24 裁决 P1 放开并 DV。

后续顺序：先清 P0 遗留（§8 “P0 遗留”），再做组合控件（P1），再事件 / 升级（P2）。**P1 后（2026-10-30）**：P2 与 P3 合为一单推进，见 [P2 + P3 任务单](<../../notepads/aiworkspace-extension-p2p3-todo.md>)（M0 契约 → M1 属性级协作 → M2 事件 / 运行态 ∥ M3 升级 / 迁移 → M4 parent / Agent 组合结果 ∥ M5 模板包 → M6 验证与遗留 DV → M7 回写）；新裁决 Q26–Q33 列在其 §1.2，用户同日裁决全部按建议（Q32 模板包改为快照式，§6.9 / hello-controls 的声明式提案在 M0 改写）。

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
3. **HTML 定义**（`kind: html`，`ui/extensions/HtmlBlockHost.tsx` + `htmlRuntime.ts` + `bridge.ts`）：同源 iframe（第二期 D16，不是安全边界），`window.aiws` v2 API：`bindings`、`input(name).rows/fields/markdown/props/bytes`、`resolve/outline/find`、`watch`、`table(name).upsert/setColumn`、`record(name).set`、`text(name).setMarkdown`、`createBlock`、`batch`、`upload`、`snapshot`、`notify`。自由画布编辑默认显示快照，显式激活后挂载；查看模式静态显示；现有非公开 show 默认静态，允许标记为 live 的节点显式激活。流式页编辑目前直接激活 Editor（`FlowSurface.tsx`，受挂载预算限制），尚未统一为静态优先。实际请求使用当前访问者的会话或 guest 凭据。

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
| `method_def` + `eventlist` | 目前只有 `create`、`toolbar`、`keys` 等设计器入口；公开方法 / 事件绑定待补 | 插入建容器 + 根；Tab / Enter / 方向键 / Ctrl+/ / Ctrl+D |
| 接口说明 | `agent: ControlDescriptor` | 共享模块 `controls/mindmap.ts` 的 Markdown 自述 |

**共享模块** `src/frame/aiworkspace/controls/`（浏览器与 Deno 同用，无依赖）：`contract.ts`（`CONTROL_KINDS`、`TEXT_VIEWS`，与 `core.json` 一致）、`placement.ts`（`"auto"`）、`tree.ts`（四方向树布局、折叠、pin）、`measure.ts`（文字测量，Deno 侧用等宽近似）、`solve.ts`（从 outline 解出一个 control，嵌套自底向上）、`edges.ts`、`outline.ts` / `opml.ts`（大纲互转）、`svg.ts`（静态图）、`drawio.ts`（物化）、`catalog.ts` + `mindmap.ts`（目录与自述，生成 `aiws/controls.json`）。

**内核**（`core/src/{types,plan,read,model}.rs`，`controls/core.json` 经 `include_str!` 共享）：`control_kinds` 列表决定哪些容器 kind 是 control；`validate_container` 校验 `layout`（隐含 `{ mode: "tree" }`）与 `config`；`child_allowed` 允许 control 内的 Cell 再带子 Cell；`check_placement` 允许 `"auto"` 分量（`x`/`y` 只在 control 直接子节点，`w`/`h` 只给 control 与 `text_views`）；`slot` 同父唯一；outline 投影 `config` / `slot` / `side` / `branch_color`；PROTOCOL 1.1。

**Desktop 接入点**：`layout.ts`（`Laid.control / node / origin`，`placeControl`）、`useSurfaceLayout.ts`、`RenderHost.tsx`、`CanvasView.tsx`、`clipboard.ts`、`surfaceOps.ts`、`flowOps.ts`、`FlowSurface.tsx`、`drawioExport.ts`、`state/outline.ts`、`presentation/*`。部分路径已按注册表分派，但还不能保证新增 control 只需注册：例如 `clipboard.ts` 把 control 子节点粘贴到画布时固定包装为 `mindmap`，`CanvasView.tsx` 仍直接调用脑图的 `addNode`。泛化范围要覆盖这些操作。

**服务与 Agent**：许愿格通用 `control` 结果（`aiws.result.control` / 模型 `put_result`，`kind` 必须在 `control_kinds` 里，支持 `under` 扩展某节点）；`control.list / describe / import / export`；websdk CLI `buckyos aiws controls list|describe`、`import`、`export`；手册只列目录，Agent 需要时再取自述（渐进式披露）。DV 已证明 Agent 只凭手册能在指定画布建图并扩展分支。

### 2.3 两层的关系

- control 的节点就是 Block，经 `BlockRegistry` 解析；节点可以是扩展 Block（declarative / html），也可以是另一个 control（R24）。
- control 不转发任何东西：选择、批注、锁、连接线绑定、原地改字都落在节点 Block 上。
- 非文字节点的 `w`/`h` 必须是数值（`check_placement`）；扩展 Block 作为节点时同样如此，因为只有 `text_views` 的尺寸能从 `title` 算出。
- Block 不能自己声明 `auto`，也不能按 id 引用兄弟（脑图方案 §5.3）。HTML Block 的 `createBlock` 只能在自己旁边建兄弟 Block，不能在某个 control 下建子节点。

### 2.4 PIKG 的现有边界

> 2026-10-30：第一条“类型与安装”已由 P0 解决——Rust `AppType::Aiws`（wire `aiws`，唯一子包 `pkg_list.aiws`，`derived_selector_for_key` 平台无关）、TS allowlist、product 准入（任何版本只接受用户提交进库）、Installer 路由 `RegisterAiwsPackage` → aiworkspace `ext.install { mode: installer }`；CLI `pikg init --kind aiws-controls`。“内容与依赖”按 AIWS 清单落地（`aiws-manifest.json`，`deps` 暂为空）。签名 / 发布与模板包两条仍是现状。以下为 P0 前的记录。

PIKG 已有归档、内容摘要、对象引用和自包含校验基础，但尚未支持 Workspace 控件包 / 模板包的完整消费流程：

- **类型与安装**：WebSDK `pikg_protocol.ts` 仅接受 `doc_type: app`，`app_type` 为 service / dapp / web / agent / agent_skills；Rust `buckyos-api/src/app_doc.rs` 的 `AppType` 为前四类。Rust PIKG reader 按 AppDoc 解析，Installer 对 Agent 分流后，其余进入普通 App 安装。新增 Workspace 资源用途需要联动共享类型、两端校验及消费方分派。
- **内容与依赖**：现有 `.pikg` 容器、PackageMeta、content index 和 payload 摘要可复用；CLI 完整包要求自包含，不接受 `source_url` 或非空第三方 `PackageMeta.deps`。控件依赖需冻结并打入包内，原控件身份 / 来源记录放在 AIWS 内容清单，不能直接套用普通 App 的外部子包依赖。
- **签名与发布**：当前 CLI pack 生成 `APPDOC.json` candidate；info 可读取 JWT 形式但不完成离线身份验签，发布状态也未检查。现有 CLI 没有完整的 `pikg sign` / 正式发布闭环。§6.8 的“导出后签名发布”是目标流程，需要沿用命名对象与 Resolve/Verify 信任体系补齐入口。
- **Workspace 内容**：当前 `store/src/export.rs` 导出 `.bcanvas`，提供 share / personal_backup；内置模板由 Desktop `api/demos.ts` 创建 Workspace 后提交内容。这些能力可供后续模板实例化参考，不能视为已经支持 PIKG Workspace 模板包。

## 3. 路线一：扩展 Block，核心是实现 render-block

### 3.1 定位

路线一回答"这一格里画什么"。它适合：**内容由绑定数据决定、自身是一个整体、不需要排布兄弟**的对象。判断标准：能否用"一个 Cell + 一个或几个数据绑定"描述完整？能就是 Block。

render-block 是这条路线的渲染核心：给定 `RenderContext`，产出视图；控件的公开接口、实例化与生命周期遵循 §1 的共同契约。渲染侧必须遵守的三条（第二期 §9.3、§10.2）：

1. **静态优先**：`Static` 必需，无媒体播放、无代码、不执行许愿格；小尺寸时宿主用 `Simplified` 或占位。
2. **模式正交**：不给 `View` / `Editor` 就回退到 `Static`，绝不继承另一个模式的写入行为。
3. **写入经宿主**：所有写都是 Operation 经 `WorkspaceStore.submit`，复用权限、锁、撤销、保存状态。

### 3.2 三档的能力边界（现状）

| 能力 | 代码注册 | `declarative` 定义 | `html` 定义 |
| --- | --- | --- | --- |
| 发布方式 | 随 Desktop 发版 | 随文档（数据树实体） | 随文档 |
| 谁能产出 | 开发者 | 人、Agent、许愿格（可规划） | 人、Code Agent、许愿格（`aiws.result.html` 已有） |
| 渲染能力 | 任意 React | 六种 item、两种布局 | 任意 HTML / CSS / JS，可联网 |
| 数据绑定 | 任意 | 一个 `source`（record / table） | 多个命名 `bindings`（input / result） |
| 编辑器 / 工具栏 / Inspector | 全部 | inspector 字段、三个固定动作 | 自己画；工具栏项直到声明 `tools` 前显示为标签 |
| 连接线锚点 `shape` / `anchors` | 可定义 | 宿主默认 | 宿主默认 |
| 流式页 | 可定义 | 同画布 | 查看显示快照；编辑目前自动激活 |
| 离线 | 是 | 是 | 是（快照） |
| 手机 | 是 | 是 | 只显示快照 |
| 性能成本 | 自报 | 低 | `cost.html`，受同时挂载上限 |
| 执行边界 | 随宿主发布的代码 | 不执行扩展代码 | 同源无沙箱（D16），具有当前访问者的同源能力 |

目标中的 PIKG 分发与选择性引入覆盖可携带的定义和实现。代码注册的 React 组件目前仍随 Desktop 发布，不能仅打成 PIKG 就自动获得动态加载能力；首版范围见 Q8。declarative 保持有限的表现词汇，需要自定义表现可以走 HTML，需要组合可操作的子 Block 则走路线二。

### 3.3 缺口与建议

| # | 缺口（现状） | 建议 |
| --- | --- | --- |
| B1 | 定义缺少目录元数据与 Agent 自述 | 补 `catalog`（分组、图标、用途）与 `agent`；两条实现路线共用对外描述，目录显式区分本机可引入与工作区已引入。元数据形状与大小需校验，不能只放开键名 |
| B2 | HTML `createBlock` 只能建兄弟 | 拟加 `parent`，接受绑定或实体目标；对应 `aiws.result.control` 的 `under` 场景，受结构、slot、placement 和目标权限约束。非文字节点仍给数值尺寸 |
| B3 | 新实例可能没有 HTML 快照 | 定义可携带目录预览，实例可引用初始预览；实际数据快照由扩展主动调用 `snapshot` 上报。“运行一次”不保证生成快照；缺失时显示占位，不阻止导入定义 |
| B4 | 来源升级与源码迭代未区分 | 普通源码修改经 build 同步校验后原子生效；包来源升级显式进行。两者均检查实例接口，必要迁移单独确认；失败保留最后可用活动修订。并存策略见 Q1，开发循环见 §6.7 |
| B5 | 扩展 Block 作为脑图节点**不能 auto 尺寸** | 接受。手册与自述里已写"非文字节点要给尺寸"，block-def 的 `default_size` 就是节点尺寸 |
| B6 | declarative **词汇与数据源种类**少（record / table） | 不扩布局；可按需加 item 种类（sparkline、progress、image）和 richtext / asset 绑定；每加一种都是解释器改动，随 Desktop 发版 |
| B7 | 没有定义交换格式与依赖清单 | 文件入口与 PIKG 本机来源共用定义校验、依赖闭包和 Workspace 注册。`.block.json` / `.block.zip` 作为导入和交换载体，日常编辑直接修改工作区文件，见 §6.2、§6.7 |
| B8 | 插入详情只选单个 source，无法填写必填 config / 命名 bindings | 从定义生成初始化步骤：应用默认值，编辑实例属性，逐项映射绑定并验证；允许先只引入定义，稍后再插入实例 |
| B9 | 尚未接入 PIKG 安装结果作为本机 AIWS 控件目录 | 复用 BuckyOS 包基础设施提供目录发现与所选控件读取；资源传入目标 Workspace 后按普通资产引用保留。跨 BuckyOS 传递不携带来源会话凭据 |
| B10 | 缺 build 链路：定义与 home 源码目录无关联，服务无同步校验 / 原子提交入口 | 新 RPC `ext.build`（输入文件 `obj_id` 清单，服务经网关以调用者 token 取字节、校验、一次 commit，同步返回诊断）；客户端共享的 build / pull / watch 驱动；Text Editor 与内容打开协议零改动（A1 / A2，§6.7、§7.5） |
| B11 | 尚无从 Workspace 多选控件、导出为全局可用或 PIKG 的开发出口 | 冻结所选定义的源码、实现和依赖闭包；共用打包流程，分别接本机控件库安装与 .pikg 文件输出；签名与发布随后进行，见 §6.8 |
| B12 | PIKG 类型和消费方未覆盖 Workspace 扩展，模板包无共同契约 | 增加 Workspace 资源用途识别、内容清单校验和本机库安装分派；控件包与模板包共享控件 / 资源结构，模板入口预留，见 §6.9 |

**P0 后状态（2026-10-30）**：B1、B8、B9、B10、B11、B12 已交付（B11 的 Desktop 出口是“设为对我所有工作区可用”与把 bundle 写到 home，`.pikg` 今天由 CLI 生成；Q20 裁决后由服务 `ext.pack` 生成，Desktop 经 krpc 调用）；B4 的“源码修改经 build 生效”已交付，“包来源升级”仍待 P2；B7 的交换载体最终是 bundle 目录（`aiws-manifest.json` + `controls/<dir>/`）与 `.pikg`，`.block.json` / `.block.zip` 不再单独设计（§7.6 Q21）；B3 的目录预览只有 `catalog.icon`，实例快照规则未变；B2、B6 未动（B2 属 P3，B6 按需）。

### 3.4 落点

> 实际落点见实施记录 §2。与本表的差别：共享模块落在 buckyos-websdk `src/aiws_ext.ts`（子路径 `buckyos/aiws_ext`，Desktop 与 CLI 共用），不在 `aiworkspace/controls/` 旁；bundle 校验与 `component.json` 规则在 Rust（`store/src/{ext,library}.rs`）与 TS 各有一份、以测试向量对齐；Desktop 入口是 `ui/extensions/{ExtensionsDialog,InstanceForm}.tsx` 与 `controlSources.ts`。P1 的组合定义沿用这个“Rust 校验为准、TS 预检”的双实现格局。

| 层 | 文件 |
| --- | --- |
| 内核 | `core/src/types.rs` 定义元数据与引用校验；`schemas/`、README；新增字段与协议版本一起评估 |
| 共享模块 | 两种实现共用的定义 / 来源 / 依赖解析与校验（目录落点 Q4），供浏览器与 Deno 使用 |
| Desktop | `api/types.ts`、`ui/canvas/catalog.ts`、`InsertCatalog.tsx`：两级目录与实例初始化；`DataDetail.tsx`：更新、导出、引用者；`ui/extensions/bridge.ts` 与属性编辑宿主 |
| 开发工具链 | Text Editor 与 websdk `content.ts` 零改动；新增 websdk 共享的 build / pull / watch 驱动（NFSP `list` / `uploadFile` / `watch` + `ext.build`），Desktop 的"编辑控件源码…"用 `filesOpenLaunch` 打开 home 目录、构建按钮与诊断面板，CLI `aiws` 模块接 NFSP |
| 服务 / SDK | 本机控件目录读取、Workspace 引入 / 更新与多选导出协议；资产暂存、依赖闭包、包内引用及 PIKG 构建 / 本机安装接入；同步 `buckyos-api`、websdk，不把拟议方法名当已有接口 |
| PIKG / 安装 | WebSDK `cli/modules/pikg.ts`、`pikg_protocol.ts`；BuckyOS `src/kernel/buckyos-api/src/app_doc.rs`、`src/frame/control_panel/src/{pikg,app_installer}.rs`：用途识别、校验与资源库分派；模板清单预留，签名发布按统一信任体系补齐 |
| CLI / 手册 | 区分查询本机库、引入 Workspace、列工作区控件、创建实例、多选导出到本机库 / PIKG；实现时同步 `aiworkspace_template_dev.md`、`aiworkspace_run.md` 及涉及的 PIKG 开发 / 发布手册 |

## 4. 路线二：基于已有 Block，实现 Block-control

### 4.1 定位

路线二回答"一组 Block 怎么排、怎么连、怎么操作"。它适合：**结构是 UI 的、内容绑数据**（脑图方案 §4.3）——节点可以是任意 Block，结构只在 BlockTree，数据源模式不显示结构。判断标准：去掉子节点后"它"还剩什么？剩的只是布局规则、边和键盘行为，就是 control。

当前代码注册的 control 构成如下。这里的 `kind = control id` 是已有实现约束；工作区自定义控件以定义身份区分，通用宿主承载实例（K4）。

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
3. **Desktop**：`ui/controls/<kind>/definition.tsx`（`ContainerDefinition`）、`Chrome.tsx`、可选 `Flow`、`registerControls.ts` 注册、i18n、CSS；检查 `layout.ts`、`RenderHost`、`CanvasView`、剪贴板和导出中的脑图专用分支，不能假定只注册就完成。
4. **服务**：许愿格 `control` 结果与 `control.*` 自动认新 kind（都查 `is_control_kind`），但 `control.import / export` 的格式分派与 `wish` 的 children 解释目前都是"大纲 → 树"；非树形 control 要加自己的格式。
5. **CLI / 手册**：目录驱动减少修改；引入新格式与实例化接口时仍需同步参数、示例和手册。

当前每增加一种代码注册 kind 都要更新内核。目标是为宿主能力保留这条发布路径，同时允许大量组合控件仅由 Workspace 定义描述（K4）。

### 4.3 缺口与建议

| # | 缺口 | 建议 |
| --- | --- | --- |
| K1 | 内核 config 校验仍是 mindmap 专用 | 区分宿主结构约束与控件属性 schema；结构和引用由内核校验，定义中的属性 schema 供前后端一致使用。是否引入 Rust JSON Schema 校验依赖、支持哪个子集见 Q3 |
| K2 | 实际布局求解只有 tree | 增加 fixed，按需要加 stack / grid；继续遵守脑图 §5.7 的单向布局与 R24 嵌套。新组合类型引用已有算法；任意新算法代码的包加载不在首版 |
| K3 | 模板外观（R23）与内容 slots 尚未形成接口 | 定义描述模板外观及公开属性映射，由 Chrome 绘制；内容 slots 指向真实子 Block。临时交互状态不写回定义；模板外观更新不得重建用户内容 |
| K4 | 缺少工作区组合控件定义与实例关系 | 组合源码描述公开属性、绑定、slots、动作 / 事件、模板外观、布局和依赖，保存后生成运行投影；通用 `composite` 实例引用稳定定义身份。新定义无需增加内核 kind，build 不重建实例。实体形态见 Q2 |
| K5 | 定义交换与内容导入未区分 | `.md` / `.opml` 仍是内容导入；组合定义的交换载体保存接口、实现和依赖，进入共同引入流程；片段复制另行标识，不能混用 |
| K6 | 目录只覆盖代码定义，Agent 无工作区组合类型自述 | 本机目录描述可引入类型；Workspace 目录描述已引入定义，两条实现路线共用。实例查询补充当前属性与绑定，不以 config.description 代替类型契约 |
| K7 | 文字测量两端有差（浏览器 canvas / Deno 等宽） | 已接受（脑图方案 §5.4）。新 control 的节点若非文字 Block 不受影响 |
| K8 | 复制 / 插入 / 拖动中仍有脑图假设 | 按宿主能力分派；从 composite 复制子节点不能强制包装为 mindmap。实例复制需明确私有内容克隆、共享绑定保留、引用重映射及 slot 处理 |

**P0 后状态（2026-10-30）**：K1–K8 均未动。P0 带来的前提变化：控件的属性 schema（A7 子集）已在服务 `ext.build` 校验、Desktop 据它生成表单并校验值，内核不校验——这是 K1 要的“结构 / 属性分层”中属性的一半，结构的一半（Q15 拆 `types.rs`）留给 P1；K4 的 build 链路（源码 → `ext.build` → 定义实体 → 实例热更新）已为 render 定义跑通，组合定义只需加 `kind: composite` 的解析、`composition.json` 校验与 composite 宿主；K8 的实例复制重映射与 Q17 一起做。

### 4.4 落点

> P1 以 P0 落点为基：组合源码校验进 `store/src/ext.rs`（与 render 定义同一次 build），`kind` 放开与 composite 实例的结构校验进 `core/src/types.rs` 并重建 WASM；TS 侧在 `aiws_ext.ts` 预检，Desktop 加通用 composite 宿主（`ui/controls/`）。

| 层 | 文件 |
| --- | --- |
| 共享模块 | `controls/core.json` 保留宿主能力；`controls/solve.ts` 求解分派；新增 fixed 等算法；组合定义解析与共同定义模块衔接 |
| 内核 | `core/src/types.rs`、`plan.rs`：组合定义、实例引用、slot / placement、依赖与删除约束；`core/tests/`、fixtures、WASM 同步 |
| Desktop | `ui/controls/registry.ts`、通用 composite 宿主、Chrome、属性 / 事件编辑；`catalog.ts` 列定义；`CanvasView`、剪贴板、流式页与导出能力分派 |
| 服务 / SDK | `server/src/control.rs`、`store/src/wish/`：定义引入与实例化分离，支持内容初始化；读写协议同步 Rust API / websdk |
| CLI / 手册 | 复用两级目录和共同引入流程，暴露组合定义接口与实例操作；实现时同步两份 AIWorkspace 手册 |

## 5. 选型：什么时候走哪条

| 需求 | 路线 | 理由 |
| --- | --- | --- |
| 一张图表 / 一张卡 / 一个嵌入页，数据来自一个或几个绑定 | 一（html 或 declarative） | 一格之内，不排兄弟 |
| 许愿格的程序产物要有交互 | 一（html，`aiws.result.html` 已有） | 同上 |
| 节点是便签 / 文字 / 图片，要自动排布并画边 | 二（tree / stack / grid） | 布局属于父容器 |
| 看板：列 + 卡片，卡片可拖到别的列 | 二（grid 套 stack，卡片是任意 Block） | 结构是 UI 的，卡片内容各自绑数据 |
| 一个按钮：背景 + 文字 + 图标，点击触发动作 | 二（组合定义 + composite 宿主）或一（html） | 共同暴露属性与 click 事件；内部实现按是否需要可操作的子 Block 选择 |
| 要让 Agent / 许愿格直接产出可复用控件，不发版 | render 定义或组合定义 | 定义存入 Workspace，创建实例保留定义关系 |
| 多人各自贡献已安装的控件 | 任一路线 | 各自通过 PIKG 获得本机来源，选择性引入同一 Workspace |
| 要新的布局算法 | 宿主能力扩展 | 首版随宿主发布；与只复用算法的新控件定义分开 |

当节点需要 Workspace 原生的选择、批注、锁、连接线和 Agent 寻址时，应使用真实子 Block 与 control。仅作为整体展示的树形可视化可以用 HTML，其内部 DOM 不自动获得这些节点能力。

## 6. UI：引入、实例化、开发与导出控件

### 6.1 现状

> 2026-10-30：扩展页签空态已改为指向“从本机库添加 / 新建控件”，并有对应功能；“工作区控件…”入口在主菜单“插入 → 扩展控件”分组末尾（再加一个顶层项，小窗口里“关闭工作区”会被挤出窗口）、插入目录扩展页签与数据源详情三处。以下为 P0 前的记录。

- **插入目录**（主菜单"插入"、左侧"＋全部对象"、空白处右键）：八个页签 全部 / 文字 / 数据 / 版式 / AI / 样本 / **扩展** / 已有数据；顶部搜索；详情页按 `needs` 引导选数据或选文件；顶部有"导入 draw.io…"、"导入文件"、"导入文件夹"按钮。
- **扩展页签**只列当前工作区可读的 `buckyos.block-def`；空态文案"这个工作区还没有 Block 定义（扩展）。可在数据源中导入或由许愿格生成"——**前半句没有对应功能**（Desktop 没有任何导入 block-def 的代码；数据源模式对 block-def 只有详情页）。
- **左侧对象工具栏**：标准对象（含脑图，`STANDARD_ORDER` 里有 `mindmap`）→ 用户固定的扩展（`ui:pinned-defs`）→ "工作区扩展"入口 → "＋全部对象"。
- **control 的插入**：目录 `layout` 分组里的 `control:<kind>` 条目，插入即建容器 + 根并进入编辑；把 `.md` / `.opml` 文件拖到画布或经"导入文件"按钮会直接变成脑图（`CanvasView.handleFiles` → `importOutline`）；选中富文本里的嵌套列表可"转成脑图"。
- **跨工作区**：导出包（`share` / `personal_backup`）携带 block-def；"从模板创建工作区"会带上模板里的定义（KPI 卡）；"把模板添加到当前工作区"是 UI 改进 §6.2 列明的未做项。

### 6.2 引入控件到 Workspace（新增）

> **P0 实现**：主入口“从本机库添加…”已交付（`ext.list_library` → 多选 → `ext.import`：同身份同来源 `satisfied`，异来源 `EXT_IDENT_EXISTS`，本地改动不被覆盖）；跨 Zone 由客户端中转（自己 Zone 的 `ext.library_bundle` → 目标 Workspace 的 `ext.build` 内联文件 + `provenance`），服务侧已测、三 Zone DV 未跑。“从文件导入…”没有做：交换载体就是 bundle 目录与 `.pikg`；Desktop 导入 `.pikg` 的路径是上传到自己的 `exports` 桶再 `ext.install { pikg: { bucket_id, key } }`（§7.6 Q21）。下文 `.block.json` / `.control.json` / `.block.zip` 不再作为独立格式。

**主要入口**是“从本机控件库添加…”。目录来自当前用户的 BuckyOS，展示 PIKG 已安装包中可导出的 AIWS 控件；支持按包浏览、按控件多选，列出版本、依赖、所需宿主 API 和已引入状态。主界面仍以“工作区控件”为可直接插入的目录。

从开发 Workspace 导出为“全局可用”的控件经同一本机安装 / 登记流程进入这个目录，与安装外部 PIKG 得到的控件使用相同的发现和引入机制。

**交换入口**保留“从文件导入…”。拟议的 `.block.json` 是单个 render 定义的交换载体，`.control.json` 是组合定义的交换载体；都需要身份、公开接口、实现与依赖描述。`.block.zip` 可装定义、源码、README、预览及资源。精确 schema 在 P0 冻结；这些扩展名不代表已有 CLI 支持，也不代替 PIKG 本机安装。`.md` / `.opml` 导入脑图内容的现有语义不变。引入后日常编辑直接打开工作区内源码（§6.7），不重复走文件导入。

两种来源共用引入流程，入口放在插入目录、工作区控件管理页及数据源模式。定义文件拖到画布时，进入同一流程并预选“引入后插入实例”。下表描述完整目标：P0/P1 的外部引入只支持新定义和来源版本的精确复用，异版本报告冲突；不限制对已引入控件 pull → 改 → build。P2 开放来源升级、结构迁移和公共事件绑定；属性与事件的数据契约在 P0 冻结，尚不支持的事件要求明确显示并禁止触发。

| 步 | 内容 | 失败 / 冲突处理 |
| --- | --- | --- |
| 1 选择与解析 | 读取本机所选控件或交换文件，解析定义、实现及依赖闭包；共用内核 / WASM 的数据校验，再检查宿主能力 | 明确列出缺失依赖、非法定义和不支持能力；本次引入不落半套定义 |
| 2 预览 | 显示身份、版本、来源、属性与绑定要求、用途、依赖、预览和代码执行说明 | 同名按稳定身份区分，不自动覆盖；支持查看待引入的依赖 |
| 3 已有定义处理 | 按身份、来源发布版本与来源摘要匹配；匹配时复用当前工作区定义，保留本地源码改动；不同来源版本进入升级（P2），列出本地改动和影响实例 | 外部包声称相同发布版本却带不同来源摘要应拒绝。合法源码保存产生工作区修订，不套用此规则；升级不得静默覆盖本地改动 |
| 4 准备实例（可选） | 默认只引入定义；选“立即插入”时填写属性、命名绑定、事件绑定并选落点 | 必填参数未完成则暂不创建实例；可以退回仅引入定义 |
| 5 资源就绪 | 上传并 stage 缺少的实现资源、图片或预览；定义默认保存到 data/extensions/ | 资源失败不提交定义；来源凭据不写入工作区，重试复用已就绪资源 |
| 6 提交 | 一次文档提交创建 / 更新定义、依赖引用及可选实例；更新所需迁移包含在同一事务中 | 服务端复核权限与版本；冲突无部分生效，撤销沿用现有冲突检查 |

`asset.import_object` 是独立的在线 RPC，不能作为 `doc.commit` Operation。原子性指资源就绪后的定义、引用和实例提交；上传暂存及清理沿用资产生命周期。引入多项定义时按稳定身份去重，并在最终提交时处理并发引入，不能只依赖前端预检。

离线只支持无需上传新资源、定义与依赖已可读取的本地提交；纯内联 JSON 可复用 WASM 副本路径。含新资源的控件需在线准备，本期不承诺离线上传队列。恢复同步时重新检查权限与定义版本。实例运行后上报快照是后续写入，不属于引入事务。

### 6.3 从工作区控件创建实例

工作区目录同时展示 render 定义和组合定义。当前用户本机是否安装来源 PIKG 包不影响使用已引入的定义；客户端缺少相应宿主能力时保留实例数据并显示回退，禁止依赖该能力的编辑，不能换成本机的另一个版本。

- 条目显示图标、用途、版本及来源；详情展示公开属性、命名绑定与事件、自述以及引用实例。
- 插入时计算定义默认值与实例覆盖后的有效属性，只把显式修改保存为覆盖；“恢复默认”清除相应覆盖。按绑定名逐项选择数据或创建定义允许的初始数据，必填项与 schema 校验通过后才创建实例。
- 实例自己的内容数据按定义的归属规则创建；显式共享的数据绑定保留。插入两个实例不能意外共享各自应独立的便签 / 富文本内容。
- 实例保留工作区定义引用。定义更新后，模板外观与未覆盖默认值按更新规则变化；实例覆盖与用户内容保留。
- 固定到工具栏仍使用个人 `user_state`。工作区的控件集合及定义更新对协作者共享。

### 6.4 定义组合控件与保存片段

“定义为控件…”的路径：选择画布对象 → 命名控件类型 → 声明公开属性、绑定、事件和内容 slots → 选择布局及模板外观 → 在 home 源码目录生成可编辑源码并 build 出组合定义 → 创建实例并通过 §6.7 的编辑 → build 循环调试。可视化定义工具也写同一份 home 源码再 build，不能与文件各自维护一份可编辑配置。

组合定义中保存可序列化的模板 / 初始化描述和依赖，实体形态见 Q2；实际 composite 容器与子 Cell 留在 BlockTree。按 R23，皮肤由 Chrome 绘制，内容 slots 使用真实 Block。公开属性如何映射到内部外观和 slots，需要在定义中明确。原选区是否替换为新实例由创建流程显式选择，不能在提取定义时删除已有用户内容。

“保存为片段…”保存独立复制描述，插入后不跟随片段更新。实例复制与片段插入都要区分私有内容克隆、外部共享绑定保留、内部引用重映射。当前剪贴板仅在部分同画布场景克隆数据，不能直接当作完整模板实例化实现。

Agent / 许愿格可以直接产出工作区定义，也可以只产出实例或片段。三者在结果类型与 UI 中明确区分。控件先在 Workspace 内调试，复用时通过 §6.8 多选导出到本机库或 PIKG；签名、发布与传播在得到包之后进行。

### 6.5 管理、权限与运行

- 工作区控件列表提供多选“导出控件…”，详情提供单控件导出、更新、依赖与实例引用者、移除。导出可选“全局可用”或“生成 PIKG 文件”（§6.8）。移除被实例或其他定义引用的控件应拒绝并列出依赖；本机 PIKG 卸载不删除工作区定义。
- 实例与定义详情提供“编辑控件源码…”（尚无工作副本时先 `pull` 到 home，再打开目录）、“构建”、watch 开关，并显示 build 修订、`derived.build` 摘要与最近一次 build 的诊断。build 改的是本工作区的定义，所有引用实例受影响；只想修改一个实例的行为时先显式派生定义。
- 引入与实例化沿用目标位置的结构权限；更新还需相应定义及迁移内容的写权限。按钮依据当前能力显示状态，最终由服务端裁决；不增加 Owner 专属门槛。具体能力映射见 Q9。
- 查看模式不提供结构修改；离线能力按 §6.2 判断，不把正常离线与权限失效、同步错误混为一谈。后两者按现有保存异常流程处理。
- 手机首版不提供从本机引入入口；已引入控件按现有移动端能力显示，HTML 保留静态预览。
- 设计态 / 运行态遵循 §1.4；自由画布、流式页的 HTML 激活规则要统一验收，修正当前流式编辑自动挂载的例外。
- 当前同源 HTML 执行使用访问者的身份与能力，不会因其是访客而获得 Owner 会话。引入定义不等于执行定义。公共端 / 回放按第四期规划采用静态快照或受控状态；是否开放 live 扩展由相应运行契约裁决，不能照搬现有非公开 show 的 live 行为。

### 6.6 Agent、CLI 与许愿格

现有 `buckyos aiws controls list|describe` 是代码 control 的目录，`aiws import` / `export` 处理大纲内容。目标接口至少提供：查询当前 BuckyOS 本机库、读取控件自述、选择性引入目标 Workspace、查询工作区控件、创建实例、读取 / 修改源码、显式升级来源版本，以及将所选定义导出到本机库或 PIKG 文件。两个目录范围必须可区分，两条实现路线在同一工作区目录可发现。方法名、参数与 CLI 动词在 P0 定义，不能把草案示例当成可执行命令。**P0 已定**：`aiws build | pull | ext-export | library list|install|remove | ext-import`（`import` / `export` 仍是大纲动词），`pikg init --kind aiws-controls`；来源升级的动词留 P2。

`aiws.result.html` 已能产出定义与 Cell；后续补来源 / 自述、初始化契约与共同校验。组合结果补定义引用及内容初始化，不能仍把所有 control 结果解释成大纲树。Agent 的读取、写入与执行使用自身授权，不借用贡献控件者的凭据。

Agent 与人编辑同一份 home 源码目录（经网关 NFSP 或同机本地文件），通过同一个 `ext.build` 提交，走同一条校验 / 原子生效链路；一次 build 天然把多个文件的变更合并为一次提交。诊断与 build 修订可查询，Agent 不会只知道“文件已保存”却不知道控件是否已经采用它。CLI 至少需要 `build`、`pull` 两个动词。

实现时同步 `aiworkspace_template_dev.md`（发现、引入、定义、源码调试、多选导出、本机复用、来源升级）和 `aiworkspace_run.md`（绑定、执行身份、幂等、结果），PIKG 构建、签名与发布接入同时核对 `buckyos_app_dev.md` / `buckyos_dev_release.md`，保持 SDK / CLI / 服务协议一致。本次只更新设计，现有运行手册不提前写入未实现命令。

### 6.7 用户开发与调试：编辑 home 目录源码，build 原子生效

**开发模型是工程师熟悉的"编辑 → build"（D6 修订，A1 / A2 裁决，评估见 §7.5）：控件源码放在开发者 home 目录下的普通文件夹，用任意编辑器修改（系统 Text Editor、IDE、Agent）；`build` 把一次源码快照提交给 aiworkspace 服务同步校验，通过则在一次 `doc.commit` 中原子更新定义、源码快照与实现资源，引用它的实例随之更新；失败则不提交，Workspace 保持上一个可用版本。** 日常调试不要求打包、重新引入、重新插入实例或刷新整个工作区；Desktop / CLI 提供 watch 模式自动触发 build，接近"保存即生效"的体验。

#### 源码目录与 Workspace 的关系

| 位置 | 内容 | 角色 |
| --- | --- | --- |
| home 工作副本 | `/home/<owner>/AI Workspace/<workspace>/controls/<control>/` 下的 `component.json`、实现文件与资源 | 编辑入口。只是建议布局：build 以显式参数或个人 `user_state`（`aiws:src:<defId>`）定位目录，定义里不存绝对路径，因为各人 home 不同 |
| Workspace 快照 | `data/extensions/<def>/` 下的定义实体 + 源码快照（`buckyos.dir`，成员指向 FileObject）+ 实现资源 | build 通过的"已部署"状态。供实例渲染、导出、Fork、协作者 `pull` 与回放使用，不是编辑入口 |

两者的关系类似工作副本与仓库：`pull` 把 Workspace 快照写回 home，`build` 把 home 提交进 Workspace。home 中未 build 的改动对协作者不可见。

P0 的源码目录**只支持平铺**（`buckyos.dir` 成员名不能含 `/`）、≤ 64 个文件，`.` 开头的文件归客户端（`.aiws-build.json` 记基线 `def_rev` / `dir_rev`）；子目录见 §7.6 Q23。

#### 标准流程（两条路线共用）

1. **新建或引入**：在"工作区控件"中新建 render / 组合定义，或从本机库引入。Desktop 在 home 建目录骨架（`component.json` + 最小实现，NFSP `mkdir` / `uploadFile`）并生成一个测试实例。对已有定义点"编辑控件源码…"：尚无工作副本时先 `pull`，再用 File Browser / Text Editor 打开目录。入口明确说明本工作区内哪些实例会受影响；需要独立变体时先派生定义。
2. **编辑文件**：任意编辑器。文件保存只改 home 中的文件，不影响 Workspace 与协作者。先用最小样例跑通，再补公开属性、数据绑定、外观和行为；组合控件同时声明内容 slots、模板与布局。
3. **build**：Desktop 按钮、CLI `buckyos aiws build <dir> --workspace <ws> [--watch|--dry-run|--force]`（`<dir>` 是本地目录或 Zone 路径；已实现）或 watch 自动触发。客户端用 NFSP `list` 收集目录文件：原生 home 文件在 named store 里没有对象（NFSP 报的 `obj_id` 是 `sha256:` 内容摘要），由客户端用自己的会话读出、`base64` 内联（一次 ≤ 4 MiB）；只有 `cyfile:` 对象按 `obj_id` 引用，服务以调用者 token 经网关取字节。服务校验、一次 commit，同步返回诊断（JSON 错误到行列）；输入摘要与上次相同时答 `unchanged`，不提交。CLI 的 `build` 也接受本地目录（Agent 容器、开发机）。
4. **观察**：commit 经 `doc.get_changes.touched` 到达各客户端，Cell 订阅 `e:<defId>` 重解析，激活中的 HTML 实例按新的实现引用重新挂载，组合实例重算布局 / 外观；实例 ID、属性覆盖、绑定及用户内容不变。未激活实例下次激活采用新实现；静态预览标明对应 build 修订，旧快照不能冒充新代码的运行结果。
5. **修正**：build 失败只返回诊断，不提交任何内容；改完再 build。首版无需 IDE 或断点调试器。
6. **验证使用场景**：用至少两个实例检查默认值与覆盖、不同绑定和私有内容；在另一协作者客户端确认同一 build 修订。触发一次动作后再 build，验证重载不重复执行该动作。
7. **多选导出并复用**：调试完成后在"工作区控件"中选一个或多个控件导出（§6.8）。导出内容是 Workspace 快照，home 中未 build 的改动不会进入包。

两条路线编辑的对象不同，但 build 生效的规则相同。render-block 一行已按 P0 冻结（`component.json` 的 `entry`：`html { html, css?, js? }` 或 declarative 的单个 JSON 文件；`assets` 列随快照携带、不内联；见 README 与 `schemas/component.schema.json`）；Block-control 一行仍是建议。

| 路线 | home 源码示例 | build 后应观察到的变化 |
| --- | --- | --- |
| render-block | `component.json` 描述接口与资源入口；`view.html`、`style.css`、`behavior.js` 提供 HTML 实现，或 `declarative.json` 提供声明式实现 | 修改显示公式、样式或支持的事件处理后，既有 Cell 使用新实现，后续交互执行新行为 |
| Block-control | `component.json` 描述接口；`composition.json` 描述模板、slots、具名布局及属性映射；支持脚本事件后可引用 `behavior.js` | 修改间距、模板外观或支持的交互后，既有 composite 实例更新；不重新生成用户内容 Block |

这里不允许源码任意突破宿主能力：declarative 仍使用既有词汇，组合定义首版复用具名布局算法。P1 先完成组合描述文件的 build 生效，通用脚本事件与公共事件绑定随 P2 契约接入；不支持的脚本能力须在 build 时明确报错，不能悄悄忽略。

#### build 的校验与原子性

- **输入与输出**：一次 build 的输入是目录全部文件的 `obj_id` 清单加目标 Workspace 当前的定义 / 依赖修订；输出是一次 commit 或一份诊断。不存在"源码已保存但未生效"的共享中间状态，原 `source_revision` / `active_revision` 双修订合一为 build 修订。定义上由服务在同一 commit 内以系统主体经 `entity.set_derived` 写 `derived.build { input_digest, builder_version, built_at }`，不占个人撤销步骤（Q14 收窄后的形态）。
- **校验内容**：`component.json` 结构与身份（A3）、HTML `api_version`、declarative 词汇、大小限制、`config_schema` 子集（A7）、资源引用存在；以及同 Workspace 内依赖者的接口兼容。删除被实例使用的 slot、破坏必填属性 / 绑定或改变内容结构时列出受影响实例，整次 build 失败，不自动迁移数据，也不留半套。
- **并发**：commit 对定义键与 `dir` 成员带 `expect`；基于旧快照的 build 得到 `REVISION_CONFLICT`，提示先 `pull`。两人同时改同一控件按先到先得，不以最后写入静默覆盖。
- **权限**：目标 folder 的 `structure` 与定义的 `update`；home 文件读取由调用者 token 在 NDM 网关判定，服务不越权读取（P0 实现：内联字节由客户端自己的 NFSP 会话读出，网关判定只对 `cyfile:` 对象生效）。
- **重载保留实例，清理旧运行时**：注销旧事件订阅与定时器，丢弃旧 runtime 的迟到 UI 回调；临时局部状态可重置，持久属性、绑定与内容保留。运行期个别实例报错需显示具体诊断，不能让该客户端自行切回另一个版本。
- **重载不等于再次执行业务**：加载阶段只建立视图与事件处理；已经提交的 command / run 记录其使用的 build 修订并继续原执行，下一次新事件才用新实现。收到同步、重新挂载或重连都不能自动重放已发生的业务动作。该生命周期约束需要实现；当前同源 HTML 执行并未提供隔离任意脚本副作用的保证。
- **已发布快照与历史回放**固定自己的 build 修订，不跟随当前 Workspace 的后续 build。

#### 协作与多人

- **协作者改源码**：`pull` → 改 → `build`，git 式而非 Google Docs 式（§7.5.4 R1）。跨 Zone 协作者同样：home 在各自 Zone，build 经目标 Workspace 的服务提交。Workspace 快照保证导出、Fork 与新协作者不依赖任何开发者的机器在线。
- **Agent**：与其他客户端相同，经网关 NFSP 用自己的 token 读写 home，再跑 CLI build；与 OOD 同机或在容器内时直接写本地目录，CLI `build <本地目录>` 把文件内联上传，不依赖 nfs-server 扫描（§7.5.4 R3 已不适用）。诊断与 build 修订可查询，Agent 不会只知道"文件已保存"却不知道控件是否已经采用它。
- **所有入口共享规则**：人、可视化设计器与 Agent 的 build 走同一权限、校验与版本检查。
- build 只影响当前 Workspace。显式选择"导出为全局可用"才把一次导出结果安装到本机库；生成 PIKG、签名 / 发布与其他 Workspace 的来源升级也都是各自的操作（§6.8）。

### 6.8 导出控件：全局可用或生成 PIKG

导出是开发循环的正式出口。开发者先在 home 目录编辑、build 进自己的 Workspace 调试，再从“工作区控件”中多选定义，一次导出成一个控件包；render 与组合控件可以混选。从画布实例进入时，先解析其定义并去重。实际导出对象是可复用的控件定义及依赖。

```text
home 目录编辑源码 → build 进自己的 Workspace → 调试实例
  → 选择一个或多个控件 → 导出控件
      （服务返回定义 + 源码快照 + 依赖闭包的 bundle → 客户端写到 home 临时目录如 AI Workspace/exports/<pkg>/ → pikg pack，A6）
      ├─ 全局可用：安装到自己的 BuckyOS 本机控件库
      │    → 在其他 Workspace 选择引入
      └─ 生成 .pikg：取得控件包文件
           → 后续签名 → 发布 / 传播 → 他人安装并选择引入
```

两个出口使用同一套 PIKG 产物构建和控件清单。“全局可用”在构建后接本机开发态安装 / 登记；“生成 PIKG 文件”返回可保存、可继续签名的包。用户不需要为了本机复用而先完成对外发布，导出也不自动执行发布者签名或上传发布。这是目标产品流程；AIWS 控件包的元数据、构建适配和本机开发态安装支持需在 Q12 落实，不能视为当前 Installer 已支持。

> **P0 实现**：`ext.export` 返回 bundle；CLI `aiws ext-export <def,…> --workspace <ws> --out <dir> [--pikg] [--install]` 在本地目录 `pikg init/build/pack`，`--install` 把 `.pikg` 绑进自己的 `exports` 桶再 `ext.install`。Desktop 的两个出口是“设为对我所有工作区可用”（`ext.install { from: { workspace_id, entity_ids }, package }`，服务端直接入库，内容与 `.pikg` 相同）与“把 bundle 写到我的 home”（随后显示 `ext-export --pikg` 命令）；浏览器里不生成 `.pikg`；Q20 裁决：打包挪到服务端 `ext.pack`（store / server 已有 `tar` + `flate2`），产物进调用者 `exports` 桶，Desktop 下载或直接 `ext.install { pikg }`，签名仍在 CLI（P1 L4）。Installer 路径 `buckyos app install x.pikg` 已通（不建 AppSpec、`app list` 不列）。“home 有未 build 改动时提示”未实现（§8 P0 遗留 L3）。

导出过程需要明确以下内容：

- **所选定义与依赖**：展示用户选中的控件，以及自动补齐的控件、源码和资源依赖，按稳定身份及修订去重。一个包可包含多个控件；缺失或冲突的依赖显示具体原因，不生成半套包。
- **固定导出修订**：基于一次固定的工作区状态，冻结 build 通过的源码快照、实现和全部依赖。导出的是 Workspace 快照而不是 home 文件；home 工作副本有未 build 的改动时提示，不能把未校验的源码混进包；导出期间继续编辑或 build 不改变已冻结的内容。
- **可独立使用的内容**：包携带定义、可编辑源码、所需实现 / 资源、公开接口、目录自述与声明的初始化模板。Workspace 实体引用转为包内引用，引入新 Workspace 时重新映射；本机库与 PIKG 都不能依赖开发 Workspace 持续在线。实例绑定的业务数据、运行记录和会话凭据不默认进入控件包；所需业务数据由新实例重新绑定。
- **包身份与版本**：导出时填写或确认本次包身份、发布版本及控件清单，保留源码修订和上游来源记录。修改后的内容使用新的发布版本或派生身份，不能在同一发布身份 / 版本下替换内容；原来源摘要保留作溯源记录。
- **完成结果**：本机出口显示已进入控件库的包和控件；文件出口提供 `.pikg` 及内容摘要。两者的控件内容应一致，后续修改源码须重新导出才会产生新结果。签名绑定导出内容对应的身份与内容图，改变内容后须重新打包并签名；PIKG 签名与发布入口沿用统一信任体系补齐，现状见 §2.4。

“全局可用”增加的是当前用户 BuckyOS 的可引入控件来源，既有 Workspace 继续使用自己的定义副本。导出、安装、签名与发布的结果分别显示；获得包文件、进入本机库、完成签名和已对外发布是不同状态。

### 6.9 PIKG 支持：控件包与 Workspace 模板包

**将两者设计为同一类 Workspace 扩展内容：控件包提供公共部分，Workspace 模板包在其上增加工作区模板入口。** 复用 PIKG 的容器、内容身份、完整性校验与签名信任体系，AIWS 负责控件定义和模板内容的语义校验。PIKG 包描述需能显式识别该资源用途；最终类型值与字段在 Q13 冻结。

#### 共用内容结构

以下为逻辑结构建议，不是当前 AppDoc / PackageMeta 已支持的 wire 字段。建议在声明的 payload 内维护一份有版本的 AIWS 清单，两类包使用相同的控件和资源表达；不在多个子包之间复制维护同一控件定义。

| 内容 | 控件包 | Workspace 模板包 |
| --- | --- | --- |
| 公共描述 | 清单版本、包用途 / 能力、所需宿主 API、目录信息 | 复用，并声明所需模板实例化能力 |
| 控件集合（概念上为 `controls`） | 一个或多个可选择的根控件，以及完整的控件依赖闭包；带源码、确定实现修订和公开接口 | 原样复用，模板按稳定身份及固定修订引用同一集合；其中可复用控件仍可单独选择引入 |
| 实现与资源 | 源码、运行实现、图片等文件，按内容去重并纳入 PIKG 内容校验 | 同一资源集合同时供控件与模板使用 |
| Workspace 模板入口（概念上为 `workspace_templates`） | 缺省或空 | 一个或多个模板；描述 Surface / BlockTree、控件实例、数据结构、显式初始数据、创建参数与外部绑定要求 |

“超集”指模板包扩展公共内容模型。只使用内置控件的 Workspace 模板允许自带控件集合为空，通过宿主能力要求声明内置依赖；不需要虚构一个控件来满足包结构。这里的 Workspace 模板也区别于 composite 的模板外观，以及单个控件的初始化模板。

控件依赖图与模板内容图分别收集：前者包含其他控件、源码和运行资源；后者包含实例、连线和明确选入的初始数据。不能因测试实例绑定了业务表，就把整张表自动算进控件定义依赖。两种图最终共用包内资源和引用映射，完整依赖冻结到同一次导出快照，原控件发布身份及来源仍可追踪。

首版继续采用自包含包；需要的第三方控件内容作为快照一并携带，由 AIWS 清单表达依赖关系，不要求 PIKG 新增跨包在线依赖安装。模板包必须能够脱离开发 Workspace 使用。

#### 安装、引入与模板创建

| 动作 | 结果与边界 |
| --- | --- |
| 安装控件包 | 校验用途及内容，登记到当前用户 BuckyOS 本机控件库；不生成普通 AppSpec，也不启动服务或实例 |
| 安装 Workspace 模板包（预留） | 复用控件安装，再登记本机模板目录入口；安装本身不创建 / 修改 Workspace，也不执行模板动作 |
| 选择控件引入 Workspace | 从任一种包的公共控件清单选择定义，复用 §6.2 的依赖、权限、版本与资源提交流程 |
| 从模板创建 Workspace（后续） | 用户选模板并填写参数 / 绑定，按所需控件闭包引入定义，再创建结构、实例和初始数据；重复创建得到相互独立的工作区 |

模板实例化时，包内局部实体 ID 映射成目标 Workspace 的实体 ID，统一重映射父子关系、`def_ref`、数据引用、命名绑定和连线端点。不可变资产可复用；模板私有可变数据每次独立创建，外部业务数据通过参数绑定。开发 Workspace 的权限成员、会话凭据、运行记录不随模板隐式复制；创建模板实例也不自动重放业务动作。将模板添加到已有 Workspace 的合并规则后续单独设计。

#### 本期支持与模板预留

P0 的交付范围是 **PIKG 控件包的构建 / 读取 / 校验、本机库安装与 Workspace 引入**，并在公共清单、包用途 / 能力识别及分派中为模板保留位置。模板编辑、模板导出和“从模板创建”的完整功能不因预留结构而视为已经实现。

尚不支持模板实例化的客户端应明确显示该能力未支持；可以在公共清单版本受支持、内容校验通过时显式选择包内控件，但不能静默把安装模板包当作安装纯控件包并报告模板可用。未知必需能力或清单版本必须有明确的拒绝 / 回退结果。 **向下兼容原则（Q19 裁决）**：正式版向下兼容，用旧版本宿主 / 清单开发的控件要能在新版本上引入与 build；清单与 `component.json` 带版本号，新版本只加可选字段、不改既有字段语义，旧字段废弃时保留读取路径；`schemas/` 先追到 PROTOCOL 1.1，新增 block-def 键不升 PROTOCOL。

实现需要联动 PIKG 开发态输入、TS / Rust 描述类型与 reader、安装服务的资源分派、本机目录、AIWS 导出 / 引入和 SDK / CLI。新增用途不能只放宽 TypeScript 的类型列表，也不能仅往当前固定的 `PACKAGE_META.json` 加字段。格式、签名与安装协议的权威文档仍由 PIKG 文档体系维护；本节定义 AIWS 的消费要求。

### 6.10 典型开发流程 demo：P3 做完后开发者看到什么

> 2026-10-30 增补。完整例子在 [hello-controls/](<hello-controls/README.md>)：`hello-kpi`（declarative）与 `hello-card`（HTML）两个控件按 P0 冻结契约写成、已过 `aiws_ext.ts` 预检，`aiws-manifest.json` 的摘要是真值；`hello-board`（composite）已按 P1 冻结契约回写（`manifest_version: 2`、`deps`、`composition.json`），`team-board` 模板仍是 P3 的**提案形态**（带 `_comment`）。例子不要求跑起来，目的是把“先干什么、再干什么、源码长什么样”一次摆出来。本节是它的缩写。

**一句话**：写 → build → 插实例 → 改 / 再 build（多人 pull → 改 → build）→ 多选导出 → 装进库 / 发 `.pikg` → 别人引入 → 升级 → 做成模板。前八步 P0 已可用，组合是 P1，事件与升级是 P2，模板是 P3。

| 步 | 开发者做什么 | 看到什么 | 阶段 |
| --- | --- | --- | --- |
| 1 新建 | Desktop “工作区控件… → 新建控件…”（名字、`declarative` / `html`、作者 DID）；或 CLI `pikg init --kind aiws-controls` | home 里一个普通目录 `AI Workspace/<ws>/controls/<name>/`：`component.json` + 实现文件 + `AGENT.md`（+ 客户端私有 `.aiws-build.json`）；画布上一个测试实例 | P0 |
| 2 写源码 | 任意编辑器 / IDE / Agent 改文件 | `component.json` 是接口（`ident`、`kind`、`config_schema`、`bindings`、`entry`、`deps: []`）；`declarative.json` 几条 item（`bind: "tasks.count"`），或 `view.html` / `style.css` / `behavior.js` 只跟 `aiws.*` 对话 | P0 |
| 3 build | “构建” / “保存即构建” / `buckyos aiws build <dir> --workspace <ws>` | 成功：一次 Commit（origin `build`），实例立刻更新，返回 `version` 与 `derived.build`；失败：`BUILD_FAILED` + 到文件行列的 `diagnostics`，什么都不提交；接口收窄时列出受影响 `instances` | P0 |
| 4 插实例 | 插入目录 → 扩展页签 → 控件 → 初始化表单 | 表单由 `config_schema` + `bindings` 生成（必填校验）；Cell 只存覆盖：`def_ref` + `bindings { tasks: { entity_id } }` + `config` | P0 |
| 5 迭代 / 多人 | 改 → build；协作者 `pull` → 改 → `build` | 实例覆盖、绑定、位置不动；基线过期得 `REVISION_CONFLICT / BUILD_BASE_STALE`，提示先 pull | P0 |
| 6 HTML 与事件 | `behavior.js` 用 `aiws.input / watch / record().set / notify / ready` | 写一律经宿主（权限、锁、撤销）；**P2** 再加 `component.json` 的 `events` 与 `aiws.emit`，实例侧 `on: { <event>: { run } }`，设计态选中 / 运行态触发，重载不重放 | P0 / P2 |
| 7 组合 | 选中几个 Block → “定义为控件…”，或手写 `composition.json` | `kind: composite`、`deps: ["did:…#hello-card"]`；`composition.json` 只说 `layout`（具名算法 + 参数）、`slots`（用户内容放哪、收什么）、`chrome`（由 config 驱动的外框）；子 Block 仍在 BlockTree；改后面板实例重排、内容不重建 | P1 |
| 8 导出 | 多选 → “导出控件…” → 全局可用 / bundle；或 `aiws ext-export <def,…> --pikg --install` | bundle = `aiws-manifest.json` + `controls/<name>/<文件>`；`.pikg` = AppDoc `app_type: aiws` + 唯一子包 `aiws`；导出的是 Workspace 快照，home 未 build 改动应提示（遗留 L3）；Desktop 产 `.pikg` = 调服务 `ext.pack`（Q20 已裁决，P1 L4） | P0 |
| 9 复用 | 对方 `buckyos app install x.pikg`（不建 AppSpec）→ “从本机库添加…” → 插实例 | 定义带 `provenance` 进 `data/extensions`；对方 `pull` 后可改并 build 进自己的工作区，身份仍是作者的；跨 Zone 由客户端 `library_bundle` → 目标 `ext.build` 中转 | P0 |
| 10 升级 | 新版本装进库后，控件详情“升级…” | 列本地改动、受影响实例、接口变化（收窄要迁移），一次 Commit 替换，覆盖与内容保留；今天只会 `EXT_IDENT_EXISTS` | P2 |
| 11 模板 | 把结构 + 实例 + 初始数据存为模板，随控件进包 | `workspace_templates` 一项（`requires.controls`、`params`、`data`、`surfaces`）；“从模板创建工作区”= 引入闭包 + 一次内核级子图导入（Q17）；安装包本身不建工作区 | P3 |
| 12 Agent | 手册 + CLI：`library list` → `describe` → `ext-import` → `doc.commit` 写 Cell；改控件 `pull` → 改 → `build <本地目录>` | 与人同一条链路、同一套权限；许愿格 HTML 结果 `pull` 时合成 `component.json` | P0 |

**P3 完成后的入口总表**见例子 README §2（Desktop / CLI / Agent 三列）。**对 P1–P3 任务单的要求**：`composition.json`、`events` / `on`、升级流程、模板项四个提案形态要在各自任务单的 M0 冻结成 schema 并回写例子目录，使例子始终是“当前契约 + 下一阶段提案”的样子；P0 部分已经如此。

## 7. 已确认边界与待裁决项

### 7.1 已确认的模型

| # | 决定 | 依据 |
| --- | --- | --- |
| D1 | BuckyOS 通过 PIKG 安装本机可用 AIWS 控件库 | 用户确认 |
| D2 | 有相应权限的人都可向 Workspace 引入控件；引入后的定义及依赖属于 Workspace | 用户确认 |
| D3 | 各协作者从自己的 BuckyOS 选择控件，共同形成 Workspace 控件集合；甲 A/B、乙 C/D 得到 A/B/C/D | 用户确认 |
| D4 | 本机库与 Workspace 集合分别管理；已引入定义及依赖留在 Workspace，本机升级 / 卸载不自动修改它 | 两级模型的约束 |
| D5 | 两条路线作为同一控件体系的实现方式；控件定义、实例和独立片段分别处理 | 本轮设计方向 |
| D6 | 控件源码放在开发者 home 目录，可用任意编辑器 / IDE / Agent 修改；`build` 同步校验并把通过的快照原子提交进 Workspace，实例即时更新，失败不提交；日常调试无需打包 / 引入；watch 模式可自动 build | 用户 2026-10-29 确认：工程师熟悉的编辑 → build 模型（原子化部署），取代原"Workspace 内文件保存即生效"；影响评估见 §7.5 |
| D7 | 开发者在自己的 Workspace 调试后，多选控件导出为全局可用或 PIKG；后续再对 PIKG 签名、发布 / 传播 | 用户确认；全局范围按两级库模型解释为当前用户 BuckyOS 本机控件库 |
| D8 | PIKG 需要支持 Workspace 扩展控件，并为 Workspace 模板保留空间；模板包在控件包的公共内容上扩展 | 用户确认；本轮只预留模板契约，优先交付控件包 |

### 7.2 实施前需冻结的细节

以下为建议，未因本次文档更新而批准新增依赖或宣告协议已实现；最后一列是 2026-10-30 对照实现的状态。

| # | 问题 | 建议 | P0 后状态（2026-10-30） |
| --- | --- | --- | --- |
| Q1 | 来源版本升级与多版本共存 | 首版同一 Workspace 对同一控件类型使用一个定义及活动实现，来源包升级显式进行；兼容源码修改保存后自动切换，不能要求每次另点更新。需要独立变体时派生新类型身份。依赖冲突拒绝并解释，不要求整个来源 PIKG 包只能有一个版本 | 已定（B1）：一 Workspace 一身份一定义；build 写 `version` + 1；异来源引入拒绝，升级是 P2 |
| Q2 | 组合定义的持久化实体与引用 | 优先评估扩展 buckyos.block-def 承载组合实现，复用定义引用和资源闭包；必要时独立实体。通用 composite 宿主只增加一次，用户类型不进入编译期 kind 表 | **P1 已实施**：block-def `kind: composite` + 内联 `composition`；实例是 `buckyos.container { kind: "composite", def_ref }`，`core.json` 的 `control_kinds` 只多一个宿主 kind `composite`（K4） |
| Q3 | 属性 schema 的共同校验 | 选择前后端一致的 JSON Schema 子集；评估 Rust / WASM 依赖、大小和错误行为后决定。整块 config 校验与属性级协作操作分别设计 | 已定（A7）：子集在服务 `ext.build` 与 Desktop 两端校验，内核不校验；Rust 无 schema crate |
| Q4 | 共享定义解析的目录落点 | 同一模块承载两种实现的公共定义与依赖解析；布局算法仍在 controls/。具体目录随 Q2 定，避免两套格式漂移 | 已定：共享模块是 websdk `src/aiws_ext.ts`；Rust 在 `store/src/{ext,library}.rs` 各一份规则 |
| Q5 | 定义在数据树的落点 | 引入定义默认 data/extensions/；许愿格现有定义可仍随结果组，目录按类型发现。真实 control 实例不得放数据目录 | 已定：默认 `data/extensions`，首次 build 建；许愿格定义留原处并补 `ident` |
| Q6 | 本机目录、工作区目录与 CLI | 显式表达来源范围，统一公开接口描述；本机读取复用 PIKG 安装结果，含 Workspace 导出的开发包；工作区读取文档定义。控件身份命名空间、目录元数据及多选导出 / 引入入口在 P0 冻结 | 已定：`ext.list_library`（本机）与 outline `ident`（工作区）；CLI `aiws library` / `ext-import` / `ext-export` |
| Q7 | 交换文件与资源范围 | 首版支持 PIKG 控件包导出、纯 JSON 交换和本机来源的资源传递；额外的 .block.zip 交换可后置，PIKG 导出及依赖闭包不能后置。预览可缺省，运行不会自动补出快照 | 调整：交换载体 = bundle 目录 + `.pikg`；纯 JSON `.block.json` 与 `.block.zip` 不再单独做（§7.6 Q21） |
| Q8 | 首版可携带实现与宿主 API | 先支持 declarative、HTML 与组合定义引用既有算法；任意 React 插件或新布局代码动态加载另列。导出器与浏览器均需声明能力与回退行为 | 已定：declarative（API 1）与 HTML（host API 2）；composite P1 |
| Q9 | 权限、属性更新及定义升级前置条件 | 复用现有能力，明确引入 / 更新 / 迁移所需权限；属性更新以属性为冲突单位，依赖旧定义的写入携带版本条件，离线重放也检查 | 已定（B3）：引入 = folder `structure` + 定义 `update`，导出 = `export`；属性级冲突 P2 |
| Q10 | HTML parent 与事件执行接口 | parent 支持合法 control / 内容节点；动作、事件参数和绑定以共同契约描述，持久副作用复用 command / run。设计 / 运行、公开 / 非公开的激活规则分别验收 | 未动（P2 / P3） |
| Q11 | 源码目录、build 协议与诊断 | **已由 A1 / A2 裁决收窄**：源码事实来源是 home 目录，Workspace 只存 build 通过的快照；不再有 Text Editor 可写目标、双修订与异步校验。P0 冻结：home 目录布局建议与 `component.json`、`ext.build` 入参（文件 `obj_id` 清单、目标 folder、期望修订）与出参（commit 或诊断）、`derived.build` 字段、`pull` 与 watch 行为、CLI 动词。复用 `dir.replace` / `dir.set_member` 的 `expect`、变更订阅与 NDM 网关授权取对象；内联定义进入源码编辑时物化到 home 再 build | 已实施：`ext.build` 入参 `files[{name, obj_id \| base64}]`、`expect{def_rev, dir_rev}`、`dry_run`、`force`；home 文件内联而非 `obj_id`（§7.6 Q22） |
| Q12 | 多选导出与 PIKG 的接入契约 | P0 冻结包内控件清单 / 源码布局、依赖去重和引用映射，复用归档与内容身份校验；落实本机开发态安装 / 登记及其权限和版本规则。签名发布沿用统一信任体系补齐，导出与本机复用不以前置发布者签名为要求 | 已实施：`aiws-manifest.json` + `controls/<dir>/`，`digest` 为文件哈希表的 sha256；`ext.install` 三种来源 |
| Q13 | Workspace 扩展用途与模板超集结构 | P0 冻结 AppDoc / 包用途的表示方式、两端类型校验与资源安装分派；控件清单与资源为公共部分，预留有版本的模板入口、能力声明及内容图引用。模板可无自带控件；完整模板创建与合并流程后续设计，不能以普通 App 安装代替 | 已实施：`AppType::Aiws`；`workspace_templates` 预留且非空拒绝 |
| Q14 | build 结果作为派生的共享状态 | **随 A2 收窄**：没有异步任务，也就没有迟到结果；`active_revision` 概念合并进 build 修订。服务在 build 的同一 commit 内以系统主体经 `entity.set_derived` 写 `derived.build { input_digest（全部文件 obj_id + 依赖修订 + builder_version）, builder_version, built_at }`，不新增写通道，不占个人撤销步骤；重放只看原语形状，不重新执行校验。见 §7.3 第 1 条 | 已实施：同一 Commit 内 `set_derived` 写 `derived.build`，只认 build / import / 撤销 |
| Q15 | D01 方案 C 的实际拆分 | composite 宿主与定义声明的属性 schema 是方案 C 的第一个真实消费者：用户控件类型不进 `control_kinds` / 编译期 kind 表，内核只校验结构与引用（容器 kind、slot 唯一、placement 分量、`def_ref` 存在），config 值域由定义的 schema 在准入时校验，副本重放不执行业务 schema。P0 前先把 `types.rs` 的校验入口按 T0-1 分类表拆成“原语 / 引用”与“业务 Schema”两层，Q3 的 JSON Schema 子集在此之上选。见 §7.3 第 2 条 | **P1 已实施**：`init_entity` / `validate_update` 经 `validate_primitive`（原语 + 引用，重放与准入都走）与 `validate_schema`（业务，仅 `PlanEnv.schema_checks`）；`ws.import` 与副本重放关闭 schema；composite 的 config 值与 composition 含义只在 `ext.build` 与客户端校验；`types::schema_validations()` 哨兵测试证明重放不进业务校验（`core/tests/composite.rs`）。Cell 等 schema 类校验器内部的引用检查仍随其校验器只在准入执行 |
| Q16 | 引用边种类与依赖图 | 现有 `RefEdge.kind` 只有 `bind` / `owns`（及 `def_ref` 阻止删除、AssetRef 进闭包）。新增“定义 → 定义依赖”“定义 → 源码目录 / 入口”两种边，并冻结每种边对删除（阻止 / 级联 / 置空）、导出与 Fork 闭包（是否随包）、重新校验传播（依赖变化触发使用者）的约束；依赖环在 `prepare` 阶段拒绝。核心设计 §16.7 的依赖图在内核落到这张边表上，不另建索引。见 §7.3 第 3 条 | **P1 已实施**：block-def `deps: [{ ident, entity_id }]`（只有 build / import / 内部 / 重放 Commit 能写）→ 强边 `def_dep`（阻止删除、导出 / Fork 闭包随包）；依赖环在规划阶段拒绝（新错误码 `REFERENCE_CYCLE`）；被依赖定义收窄接口时 build 列出依赖者并整次失败；`ext.export` package 按 `def_dep` 自动带依赖去重，`ext.import` 依赖在前 |
| Q17 | 子图导入与 ID 重映射 | 模板实例化、引入时“Workspace 实体引用转为包内引用再重新映射”、复制组合实例的私有内容，都是“把带内部引用的子图写入另一处并重写 `def_ref`、父子、绑定与连线端点”。冻结为内核级操作（`doc.commit` 的一组 Operation 或 WASM 共享的规划函数），许愿格、CLI、Desktop 剪贴板复用同一实现；整包 `ws.import` 继续保留 ID 不走此路径。见 §7.3 第 4 条 | **P1 已实施**：`tree.clone { source_id, parent_id, order_key?, placement?, slot?, content: "private", id_seed? }`（`plan.rs`，WASM 共享同一规划）；新 id = `c` + `sha256("<id_seed>\n<旧 id>")` 前 26 位十六进制，`id_seed` 缺省 `<seq>.<op 序号>`，同一请求在服务与副本得到同一组 id；`def_ref` 保留、自有数据（`owns`）经 `entity.clone` 克隆进目标 Surface 内容目录并改绑、共享数据保持共享、两端都在子树内的连线随克隆、跨界连线不复制；规范化 op 记 `id_seed` 与 `new_ids`，Commit 结果给 `cloned[].new_ids`。`ws.import` 仍保留 ID 不走它 |

### 7.3 对数据逻辑内核的影响

> 2026-10-29 增补。结论：**扩展路线不引入新原语，不要求升 FORMAT，不改变存储、保留与 `.bcanvas` 包格式**；但它是第一个把 D01 方案 C 与“派生的共享状态”逼成现实的消费者，有四处接触数据语义，已列为 Q14–Q17，冻结文档里要给出位置。

**不受影响的部分**

| 层 | 现状 | 扩展带来的内容 | 结论 |
| --- | --- | --- | --- |
| 原语 | 树节点、版本格对象、Table、RichText、AssetRef、资源目录 `buckyos.dir`（与 DirObject 同构） | 定义是 `buckyos.block-def` 实体，源码快照是 `dir` 里的文件对象，依赖是引用边 | 不新增原语 |
| 存储与保留 | `doc.sqlite` / `local.sqlite`，对象在 named store，`assets` / `objects` / `exports` 三桶绑定保留集（[Durable Data Schema](<BuckyOS AiWorkspace Durable Data Schema.md>)） | 定义、源码、资源都是普通实体与资产，自然进入 content_root 闭包 | 不改表、不改桶 |
| 导出包 | `.bcanvas` = manifest + NamedObject 闭包（第一期 §4.4，`FORMAT_VERSION` 1.0） | 工作区 Fork / 自包含导出要求携带控件实现、依赖与资源 | 现有闭包已覆盖；PIKG 控件包是并列的第二种包格式，不合并 |
| 写入路径 | 一切修改经 `doc.commit` | 引入、build、pull | build 就是一次 commit；pull 只读 Workspace 写 home，不经 commit |

**四处接触点**

1. **派生的共享状态（Q14）**。按 A2，校验与提交同步完成，Workspace 里只有 build 通过的快照，不存在"已保存未生效"的中间状态；`derived.build` 是服务在同一 commit 内写的派生摘要，要求共享、持久、不进个人撤销栈。这与许愿结果一样属于“系统主体写入、绑定输入摘要的派生更新”，现有同类路径是 `entity.set_derived` 与 `entity_versions.generated`。要冻结的只是摘要构成；异步任务与迟到结果的拒绝规则已不需要。
2. **D01 方案 C 必须真正拆开（Q15）**。今天 `control_kinds` 是 `core.json` 固定列表，`validate_container` 是脑图专用 Rust 校验器，重放也执行它们。composite 宿主一旦存在，重放语义就从“内核校验全部业务规则”变成“内核校验结构，schema 校验由定义提供”。D01 已决定方向，T0-1 已做分类表，但 `types.rs` 的入口还没拆；扩展是逼它拆的第一个场景，也是四条里改动最大的一条。
3. **引用边种类扩大（Q16）**。现在只有 `bind` / `owns` 两种边加 `def_ref` 的删除阻止。扩展要加定义到定义、定义到源码目录的依赖，并要求依赖变化触发使用者重新校验、导出时按依赖去重、循环要识别。核心设计 §16.7 设想的依赖图在内核就是这张边表的扩展，不另建索引。
4. **子图导入带 ID 重映射（Q17）**。模板实例化与引入时的引用重映射需要把带内部引用的子图复制进另一个 Workspace 并重写全部引用。今天整包导入保留 ID，剪贴板重映射只在前端。应成为内核级操作，否则许愿格、CLI 与 Desktop 各做一套。

**P0 后状态（2026-10-30）**：第 1 条已落地（`derived.build`，只认 build / import / 撤销 Commit）；第 3 条只落了“定义 → 源码目录”的 `def_source` 强边（删除阻止、导出 / Fork 随闭包）；第 2、4 条未动，是 P1 的内核工作。P0 验证了“不新增原语、不升 FORMAT、不改桶”的结论：Workspace 侧新增的只有 block-def 键、一种引用边、Commit origin `build`；本机库是服务自己的 `library.sqlite` 与每用户 `library` 桶，不在 Workspace 数据模型内。

**对冻结文档的结构建议**：核心设计 §3 的运行目录与 manifest 草案整体替换为第一期 §4.4 加 Durable Data Schema 的现状（核心设计 §3 已于 2026-10-29 加现状说明）；扩展层单独成章，只引用内核原语与上面四个接口，不改内核；PIKG 控件包与 `.bcanvas` 并列描述。这样冻结文档本身就能证明“扩展对数据逻辑影响不大”。

### 7.4 开工前裁决清单（2026-10-29 对照源码）

> 本节把 Q1–Q17 按"是否阻塞 P0 第一行代码"重新分组，并补 Q18、Q19。代码现状列来自当日源码核对（内核 `core/src/{types,plan,model}.rs`、Desktop `ui/{blocks,extensions,canvas}`、服务 `server/src/{control,lib}.rs`、`store/src/export.rs`、PIKG `pikg_protocol.ts` / `app_doc.rs` / `app_installer.rs`、Text Editor `src/fs/*`）。**裁决列由用户填写**；填入后 §7.2 对应行以本节为准，再据此写 notepads TODO。

#### 7.4.1 P0 必须先裁决（决定代码写在哪个仓、改哪个协议）

| # | 对应 | 问题 | 代码现状 | 为什么不定就不能开工 | 建议 | 裁决 |
| --- | --- | --- | --- | --- | --- | --- |
| A1 | Q11 前半 | 源码文件怎样让 Text Editor 写回 Workspace | websdk `content.ts` 的 `TransferableContentRef` 只有 `cyfs-path` / `object-id` 两种；Text Editor `documentStore.ts` 只写 NFSP 路径（`splitPath` 要求 `cyfs:///`），`object-id` 打开永远只读（`workspace.ts:159`）；aiworkspace 没有任何 NFSP 路径。内核侧已就位：`buckyos.dir` 成员各有 `m:<name>` 修订，`dir.set_member` 带必填 `expect`（plan.rs:862-881） | 跨 websdk、text_editor、aiworkspace 三仓的协议改动，决定 Text Editor 要不要新存储后端。A）给 `OpenRequest.source` 加新 ref 种类 `{workspace_id, dir_entity_id, name, expect}`，Text Editor 增加 AIWS 存储后端，成员修订当 etag；B）nfs-server 把 Workspace `buckyos.dir` 挂成 NFSP 路径，Text Editor 零改动。A 需核实 dapp 的 app token 能否调 aiworkspace RPC（text_editor 目前只有 `user/home` 权限） | A。B 把 Workspace 的提交语义、权限与锁藏到文件系统后面，冲突与修订无法对上 | 已裁决：一个思路是，aiws的目录，要挂到 workspace owner的home目录下（相当于原来windows的my document),这样用户可以用传统的ide思路来编写相关代码？；模型与影响见 §7.5 |

| A2 | Q11 后半 / Q14 | 校验与 `active_revision` 在哪一层执行 | Q14 已定由服务以系统主体经 `entity.set_derived` 写入。但 store 没有提交后任务机制：唯一的服务端派生写入是 wish planner，且在同一次规划内同步完成；变更通知只有 `doc.wait_changes`（只给 head_seq）+ `doc.get_changes`（带 touched 实体） | 剩余决定：校验是"提交事务内同步"还是"提交后异步任务 + 第二次 `set_derived` commit"。前者让保存等校验、含依赖闭包时变慢且失败会卡住保存；后者要新建 store 内的任务队列与重启恢复 | 异步任务：commit 落库后对 touched 的 block-def / dir 成员入队，结果以系统主体单独 commit `set_derived`，摘要不符即丢弃（Q14）。Desktop 不用新通道：Cell 已订阅 `e:<defId>` 重解析（`useBlockContext.ts:63`），`HtmlActive` 以 `html` 子对象 JSON 为 remount key（`HtmlBlockHost.tsx:44`），只需把 key 换成活动实现引用 | 已裁决：我觉得一致性问题还是同步比较好，相当于源码不是保存后立刻生效的，而是有一个命令行（类似build)执行后，才会针对调试的workspace生效。；模型与影响见 §7.5 |

| A3 | Q6 前半 | 控件稳定身份与去重主键 | `def_id` 只查格式 `[a-z0-9][a-z0-9._-]{0,63}`、内核不查唯一（types.rs:1513-1517）；wish 用 `wish.<hash>`，demo 用 `sample.kpi-card`；`view.type` 是宿主 `html` / `declarative` 而非定义；`def_ref.version` 字段存在但无人读；实体 `name` 同级唯一（plan.rs:438-450） | §6.2 第 3 步"按身份、来源版本、摘要匹配"目前没有可用主键；64 字符放不下 DID 命名空间 | 身份 = `<包 AppDID 或 owner>/<def_id>`；引入时用实体 `name` 存身份放在 `data/extensions/` 下，借内核"同级 name 唯一"换并发去重，不加新校验；`def_ref.version` 用于 pin 来源版本 | 已裁决：按buckyos的惯例，控件由作者did共同构造完整的唯一名字 |

| A4 | Q13 | PIKG 用途在 AppDoc 上怎么表示 | Rust `AppDocWire` 为 `deny_unknown_fields`，`AppType` 只有 Service / dapp / Web / Agent；TS allowlist 同样严格；`app_submit_route` 按 app_type 分派（Agent + Rpc → 注册模板，其余 → 普通安装）；但 `SubPkgList.others` 允许任意子包 key，`agent_skills` 是 TS 侧类型不是 Rust 值 | 三条路工作量差一个量级：A）新 `AppType::WorkspaceExtension`（Rust 枚举 + product 准入 + 新 route，像 Agent template）；B）沿用现有 app_type + 约定子包 key `aiws_controls`；C）绕过 App Installer，像 `buckyos skill install` 那样 TS 校验后直接 POST 给 aiworkspace 服务 | A。B 会让普通 App 安装路径误吞控件包；C 拿不到统一的签名 / 发布与安装记录 | 已裁决：增加新的类型 (aiws?)， aiws相关的扩展估计是未来buckyos最主要的pikg. |

| A5 | Q6 后半 | 本机控件库由谁持有、谁提供目录 | 没有通用"已安装包索引"；Agent 模板的现成模式是 control-panel 存 `users/{owner}/agent_templates/{app_id}` KV + zone bucket `agent_templates`（agent_mgr.rs:664-729）；`system:aiworkspace` 在 RBAC `frame` 组可读 `users*` | 决定 Desktop 调谁的 RPC 列目录，以及"导出为全局可用"写到哪。另外 `asset.import_object{obj_id}` 只认目标 Zone 网关可读的对象，跨 Zone 引入必须走客户端下载再上传 `{bucket_id,key}` | control-panel 照 agent_templates 做 `users/{owner}/aiws_controls/{app_id}` + bucket，提供 `aiws.list_controls` 类 RPC；aiworkspace 服务只读不存；跨 Zone 由客户端中转 | 已裁决：由aiws系统服务持有。在本机安装 aiws控件扩展或模版扩展，都是安装到系统服务里。workspace只有在用到的时候才会插入 |

| A6 | Q12 | 多选导出在哪一层构建 PIKG | 无选择性导出 RPC，`doc.export` 只能整库 `.bcanvas`（`export.rs:184-291`，`closure` 已能算资源闭包）；Rust 只有 PIKG reader（`pikg.rs`），writer 全在 TS CLI `pack`；websdk `AiWorkspaceClient` 连 export / assets / changes 的封装都没有 | 服务端建包要新写 Rust writer；客户端建包要新 RPC 吐出"定义 + 源码 + 依赖闭包"bundle，再由 Desktop / CLI 共用 TS `pack` | 新 RPC `doc.export_defs{entity_ids}` 复用 `closure` 产出 bundle，TS 侧建包；"全局可用"= 建包后调 A5 的登记 RPC；websdk 顺带补 export / assets / changes 封装 | 已裁决：导出到临时目录，再打包 |

| A7 | Q3 / Q15 | `config_schema` 支持的 JSON Schema 子集 | Desktop 已在 `resolve` 用 `z.fromJSONSchema` 校验 `config`（registry.ts:297-311）；内核只检查是对象、从不校验；core 无 schema crate；Q15 已定内核只校验结构 | B8"从定义生成初始化表单"和引入校验都要知道支持哪些关键字；子集定了才能写表单生成器与手册 | 子集 = `type / enum / default / required / title / description / minimum / maximum / minLength / maxLength`，前端校验，内核继续不校验；写进 types.rs 注释与手册 | 已裁决：按你的建议吧 |

#### 7.4.2 代码已给出答案，按建议即定（用户不改即视为采纳）

| # | 对应 | 代码现状 | 结论 |
| --- | --- | --- | --- |
| B1 | Q1 | block-def 有整数 `version`，wish 重跑时 `set_keys` +1；`def_ref.version` 可 pin | 一 Workspace 一身份一定义，接受 Q1 建议 |
| B2 | Q5 | `read::resolve_path` 已能解析 `/data/extensions`（read.rs:379-396）；block-def 可放任何 folder；目录 `extensionEntries()` 扫全 outline | 接受；wish 结果留在 `<wish>-out-data` 不迁移 |
| B3 | Q9 | 能力集 read / append / update / delete / structure / comment / export / manage / run，无角色；`entity.create` 要父节点 `structure`，`set_keys` / `dir.*` 要 `update`，stage 资产要 `update｜structure`，导出要 `export` | 引入 = `data/extensions` 的 `structure` + 定义 `update`；导出 = `export`；不新增 scope |
| B4 | Q4 / Q7 / Q8 | P0 只有 render 定义；`.bcanvas` 闭包已覆盖资产 | 共享定义模块先放 `src/frame/aiworkspace/controls/` 旁新目录；交换范围与可携带实现按建议取 |
| B5 | Q16 | 内核强边已有 `def`（阻止删除）、`asset`、`member` 三种（model.rs:238-240），不止 `bind` / `owns` | P0 只需"定义 → 源码目录"一条边，可用 def 实体对 dir 的引用 + dir 既有 `member` / `asset` 强边表达；"定义 → 定义"依赖边 P1 随 Q2 加 |
| B6 | 变更传播 | `doc.get_changes.touched` 到实体粒度，Desktop `VersionMap` 按 `e:<id>` 分发 | 不需要新通道 |

#### 7.4.3 可推迟到 P1 / P2 再定

| # | 对应 | 说明 |
| --- | --- | --- |
| C1 | Q2 | P0 已留位：`component.json` schema 的 `kind` 注明 `composite` 为 P1 保留，内核闭集未动。P1 要：先按 Q15 拆 `types.rs` 校验层，再放开 `kind`、给 composite 体加结构校验并重建 WASM；`store/src/ext.rs` 解析 `composition.json`；`aiws_ext.ts` 预检；Desktop 通用 composite 宿主。顺序不能反，否则 composite 的业务 schema 会进重放路径 |
| C2 | Q10 | HTML `parent` 与事件接口属 P2 / P3；P0 的 HTML host API 为 2，加事件接口时升到 3 并保留 2 的读取路径（Q19） |
| C3 | Q17 | P0 如预期：`ext.import` 只重建定义与源码目录，无子图重映射。P1 组合实例复制与 P3 模板实例化都要内核级重映射操作 |

#### 7.4.4 文档未列、代码会逼出来的问题

| # | 问题 | 代码现状 | 建议 | 裁决 |
| --- | --- | --- | --- | --- |
| Q18 | declarative 读不到命名绑定 | `declarative.tsx` 只读单一 `context.source`；`CellPayload.bindings` 仅 HTML bridge 消费（bridge.ts:31-36）；Desktop 没有任何写 `bindings` 的 UI，只有 wish planner 写 | P0 的"render 实例初始化"二选一：只给 HTML 做多绑定，或让 declarative 解释器也认 `bindings`。建议后者，否则 B8 的绑定表单对 declarative 无意义 | 已裁决：按你的建议 |
| Q19 | `schemas/` 漂移与 PROTOCOL 版本 | `commit-request.schema.json` 仍写 `protocol_version 1.0`，selector 枚举缺 `tree_children / tree_edge / folder_member / table_view / table_query`；`model.rs:213` 引用不存在的 `schemas/references.json`；新增 block-def 字段是否升 PROTOCOL 1.2 | P0 加字段前先把 schemas 追到 1.1；新增字段只扩 block-def payload 键名，不升 PROTOCOL | 已裁决：原理上正式版是向下兼容的。我们要尽量保证用旧版本开发的控件，能在新版本上用 |
**裁决顺序建议**：先 A1、A2（决定 text_editor 与 store 各开什么工），再 A4、A5（决定 control-panel 改动），A3、A6、A7 与 Q18、Q19 可在写 TODO 时一并定。

### 7.5 A1 / A2 裁决的影响评估（2026-10-29）

> 用户在 §7.4.1 的裁决：A1 源码目录放 Workspace owner 的 home 目录下（类似 Windows 的 My Documents），用传统 IDE 思路编辑；A2 校验同步执行，源码不是保存即生效，而是运行一个类似 `build` 的命令后才对调试中的 Workspace 生效。本节评估这两条合起来的实际含义、改动面与代价。以下事实均为当日源码核对。

#### 7.5.1 两条裁决合起来的模型

| 项 | 原 §6.7 方案 | A1 + A2 之后 |
| --- | --- | --- |
| 源码事实来源 | Workspace 数据树 `data/extensions/<def>/` 下的 `buckyos.dir` | owner home 下的普通目录，建议 `/home/<owner>/AI Workspace/<workspace>/controls/<control>/`（与 Desktop 的 `/home/<user>/Desktop` 惯例一致） |
| Workspace 保存什么 | 可编辑源码 + `source_revision` + `active_revision` 双修订 | **只保存 build 通过的快照**：定义实体 + 源码快照（`buckyos.dir`，供导出 / Fork / pull）+ 实现资源。源码修订恒等于活动修订 |
| 写入路径 | Text Editor 保存 → `dir.set_member` → 服务异步校验 → `set_derived` | 任何编辑器改 home 文件 → `build` → 服务同步校验 → 一次 `doc.commit`（失败则不提交，旧实现自然保留） |
| 生效时机 | 保存成功 | build 成功；Desktop / CLI 可提供 watch 模式自动触发 build 以接近"保存即生效" |
| 协作者改源码 | 直接改 Workspace 内文件 | `pull` 快照到自己的 home → 改 → `build`；旧基线 build 被 `expect` 拒绝（git 式） |

原 D6 "直接编辑 Workspace 内文件、保存后自动生效" 与此不一致。**用户 2026-10-29 确认采用编辑 → build 模型（原子化部署），D6 已按 §7.1 新文本改写**，§6.7 全节与 §7.5.6 所列位置已随之修订（v0.8）。

> 确认

#### 7.5.2 为什么改动面反而更小

| 层 | 原方案 A 要做 | A1 + A2 要做 |
| --- | --- | --- |
| websdk `content.ts` | 新增 ref 种类并让所有内容处理器认识 | **零改动**：源码就是 `cyfs-path` |
| Text Editor | 新存储后端、新 etag 语义、权限核实（dapp 的 app token 能否调 aiworkspace RPC） | **零改动**：`cyfs-path` 已可写，etag / 冲突 / 草稿恢复 / NFSP watch 都现成 |
| 内核 | `source_revision` / `active_revision` 字段、依赖重校验传播、迟到结果拒绝 | **零新原语**：build = 一次 commit（block-def create / set_keys + `dir.replace` + 资产引用）。Q11 的双修订消失；Q14 退化为同一 commit 内服务写的 `derived.build {input_digest, builder_version, built_at}`，不再需要异步任务队列与重启恢复 |
| store | 提交后任务队列 | 不需要；A2 的同步正好 |
| Desktop 热更新 | 不变 | 不变：Cell 已订阅 `e:<defId>`（`useBlockContext.ts:63`），`HtmlActive` remount key 改为活动实现引用即可 |

#### 7.5.3 需要新写的部分（改动面清单）

| # | 层 | 内容 | 依据的现状 |
| --- | --- | --- | --- |
| W1 | aiworkspace 服务 | 新 RPC `ext.build {workspace_id, target_folder, files: [{name, obj_id}], options}`：按清单经 NDM 网关取字节、校验（`component.json` 结构、HTML `api_version`、declarative 词汇、大小、同 Workspace 内依赖者的接口兼容）、以调用者身份一次 commit、同步返回诊断（文件 / 行列）。**服务不读 NFSP 路径**：Rust 侧只有 `nfs_server_client.rs` 的 settings 结构，没有 NFSP 客户端；文件由客户端给 `obj_id`，服务用现有 `authorize_object`（`lib.rs:822-836`）+ 网关 `get_object` 取字节。home 文件本来就是 FileObject（`NodeInfo.ident.obj_id`），所以是零拷贝，不需再上传 | `asset.import_object{obj_id}` 已走同一授权路径 |
| W2 | websdk 共享模块 | build 驱动（Desktop 与 CLI 共用）：NFSP `list` 源码目录 → 收集每个文件的 `obj_id` → 调 W1 → 呈现诊断；`pull`：把 Workspace 源码快照用 NFSP `uploadFile` 写回 home；watch 模式：NFSP watch SSE（lossy，收到 `resync` 先 re-list）+ debounce + 内容摘要相同不提交 | `NfspClient` 已有 list / uploadFile / watch；CLI 目前只有 `share.ts` 用它，`aiws.ts` 要新接 |
| W3 | Desktop | 控件详情"编辑源码…"= 用 `filesOpenLaunch` 打开 home 中的源码目录（File Browser 已能按路径启动）；"构建"按钮与 watch 开关；诊断面板；"从本机库引入 / 新建控件"时在 home 建目录骨架（NFSP `mkdir` 已有） | `desktopFolderActions.ts:5` |
| W4 | 导出（A6） | 服务新 RPC 吐出所选定义 + 源码快照 + 依赖闭包的 bundle；客户端写到 `/home/<owner>/AI Workspace/exports/<pkg>/` 再 `pikg pack`。服务不能写 NFSP，所以 A6 "导出到临时目录再打包" 的落点是客户端 | `export.rs` 的 `closure` 可复用 |
| W5 | 手册与 Agent | `aiworkspace_template_dev.md` 加 新建 / pull / build / export 流程。Agent 与其他客户端一样经网关 `/nfs/v1` 用自己的 token 读写 home 目录，再跑 CLI build；与 OOD 同机时也可直接写本地 `data/home/<owner>/...`（AgentRoot 已在同一棵 home 树下），两条路都要可用 | `opendan_agent_dev.md:41`。**用户原则**：所有系统服务都对 Agent 可用，Agent 权限一般大于普通 App；DV 曾记录容器内会话产物直连 `172.17.0.1:4110` 失败（nfs-server 只监听 `127.0.0.1`，main.rs:227），应按此原则修复（走网关），不在本设计里绕行 |

> **P0 修正（2026-10-30）**：W1 的“home 文件本来就是 FileObject，零拷贝”不成立——原生 home 导出上 NFSP 报的 `obj_id` 是 `sha256:` 内容摘要，named store 里没有对象；`ext.build` 因此接受 `{ name, base64 }`，由客户端读出内联（≤ 4 MiB / 次），`obj_id` 路径只对 `cyfile:` 对象保留。W4 的 Desktop 一半只写 bundle，`pikg pack` 在 CLI。W5 的 Agent 路径改为 CLI `build <本地目录>` 内联上传，容器内不再依赖 nfs-server 可达；“凭手册建控件”DV 未跑。

#### 7.5.4 代价与风险（需接受或另定）

| # | 风险 | 说明 | 建议 |
| --- | --- | --- | --- |
| R1 | 协作模型变为 git 式 | 源码事实来源在 owner home，其他协作者（尤其跨 Zone）不能直接改；要 pull → 改 → build。两人同时 build 同一控件时后者被 `dir` 成员 `expect` 拒绝，需重新 pull。§1.3 "源码存 Workspace、是开发时的编辑入口" 要改为 "Workspace 存快照，home 存工作副本" | 接受。这正是 "多人协作的 Delphi" 中源码管理的常态；Workspace 快照保证导出 / Fork / 新协作者不依赖开发者机器在线 |
| R2 | "保存即生效"体验降级 | 多一步 build；每次 build 是一次 commit，历史变多 | watch 模式 + debounce + 摘要相同不提交；commit 带 `origin: build` 便于历史过滤 |
| R3 | 文件尚无 `obj_id` | 经 NFSP 保存（Text Editor、CLI、Agent 走网关）在 `commitFile` 后立即有 `obj_id`；只有直接写 OOD 本地文件（IDE 挂载、同机 Agent）要等 nfs-server 扫描（默认 `scan_interval_secs` 30 s）或主动 `stat`。`ref.type === 'live'` 时无 `obj_id` | **P0 后不适用**：home 文件由客户端读字节内联，不依赖 `obj_id` 与扫描 |
| R4 | 多文件取字节 | 网关 `get_object` 与 `authorize_object` 只接受 FileObject，`asset.import_object` 明确拒绝目录。一个控件多文件就是多次调用 | **P0 后只对 `cyfile:` 对象成立**；内联文件一次请求带全部，受 4 MiB 上限（R8） |
| R5 | home 布局与 Workspace 改名 | 目录名若用 Workspace 标题会随改名漂移；定义里不能存绝对路径（各人 home 不同） | 默认 `AI Workspace/<标题>/controls/<def_id>/`，仅作建议；每人用 `user_state` 记 `aiws:src:<defId>` → 路径；build 不依赖该记录 |
| R6 | 权限 | build 需目标 folder `structure` + 定义 `update`；home 文件读取由调用者 token 在网关判定，服务不越权。Desktop 内用用户会话 token，没有方案 A 的 dapp app token 疑问 | 无需新能力 |
| R7 | 依赖者重校验 | 被依赖控件 build 时，同 Workspace 内依赖者的接口兼容在同一次 build 内同步检查；不兼容则整次 build 失败并列出依赖者 | 与 A2 同步语义一致，不需要 Q14 的迟到结果拒绝规则 |
| R8 | 内联上限 4 MiB / 次 | 大图片、字体等资源走内联会超限直接 `BUILD_FAILED`；`assets` 文件进快照也占定义之外的对象空间 | §7.6 Q22 已裁决按建议：超限文件先经 `ws.stage_object` / 资产上传成对象再按 `obj_id` 引用；上限保留；hello-controls 说明资源逻辑 |
| R9 | 工作区独立域名 | Desktop 在 `aiws-<id>.<zone>` 上关闭新建 / 编辑 / 构建 / 写 bundle（`onWorkspaceHost()`），库引入与“全局可用”照常 | §7.6 Q24 裁决放开：网关按路径把 `/nfs/v1/*` 交给 nfs-server、子域 token 是用户态，风险降为“待子域 DV” |
| R10 | 作者 DID 来源 | Desktop 拿不到用户真实 DID，默认 `did:bns:<用户名>`（表单可改，CLI `--owner` 同）；许愿格定义作者记 `did:aiws:<workspace_id>` | §7.6 Q25 裁决不做：工作区内信任名单 + 对外 PIKG 签名，默认值不改、不迁移；风险关闭 |

#### 7.5.5 与其余裁决的衔接

- **A4 新 AppType `aiws`** + **A5 本机库由 aiws 系统服务持有**：Installer 的 `app_submit_route` 对 `aiws` 类型新增一条路由，像 `RegisterAgentTemplate` 调 agent_mgr 那样调用 aiworkspace 服务的 `ext.install`；aiworkspace 服务用自己的 bucket 存包并提供 `ext.list_library`。模板包同路。"导出为全局可用" = W4 bundle 后直接调 `ext.install`，不经 Installer。
- **A3 身份 = 作者 DID + 控件名**：`def_id` 现有 64 字符正则放不下 DID，W1 校验时把身份存到新键 `ident {author_did, name}`（types.rs 需放开），`def_id` 保留为宿主内短 id；实体 `name` 存身份字符串借同级唯一去重。
- **A6 导出到临时目录再打包**：落点见 W4。

#### 7.5.6 连带要改的文档位置

已于 v0.8 修订：D6（§7.1）、§1 开发循环与 §1.3 控件定义行、§1.5 开发闭环、§3.3 B4 / B10、§3.4 开发工具链行、§4.3 K4、§6.2、§6.4、§6.5 "编辑控件源码…"、§6.6、§6.7 全节、§6.8 导出流程、§7.2 Q11 与 Q14、§7.3 写入路径行与第 1 条、§8 P0 / P2 与回归项、§9 对 Text Editor 与 NFSP 的引用。§7.4.1 A1 / A2 行的"代码现状 / 建议"列保留为裁决前的记录。

### 7.6 P0 实施后的新裁决项（2026-10-30）

> 实现中按代码事实做的决定已写进 README 与任务单 §12.1（13 条：`source_dir` / `def_source`、CLI 动词、平铺目录、home 文件内联、`ext.install` 三种来源、许愿格作者、origin `build`、Desktop 不打 PIKG、默认作者 DID、`aiws` 准入、本地目录 build、DV 修正、主菜单入口），不再重复裁决。下面是实现暴露出来、P1 开工前要定的事。**用户已于 2026-10-30 裁决**（最后一列，原话加实现读法），回写见表后。

| # | 问题 | 现状 | 建议 | 裁决 |
| --- | --- | --- | --- | --- |
| Q20 | Desktop 在哪里生成 `.pikg` | `pikg pack`（ZIP 外壳 + `aiws` 子包 tar.gz + APPDOC）的确定性写入在 CLI `pikg_archive.ts`（Node / Deno），浏览器只能写 bundle 到 home 再提示 CLI 命令；Rust 侧只有读取（`library.rs` 解 tar.gz、Installer 解 PIKG） | 把写入与 `APPDOC` 组装挪进 websdk 共享模块，Desktop 直接产出 `.pikg` | **后端打包**。aiws 后端是服务，Desktop 经 krpc 调它，由后端复用 SDK 的打包工具。读法：新增 `ext.pack { workspace_id, entity_ids, package }`，服务在 `ext.export` 的 bundle 上组装 APPDOC 并写 `.pikg`（store / server 已依赖 `tar` + `flate2`，按 `pikg_archive.ts` 的确定性规则写，同一 bundle 两端字节一致），产物落到调用者的 `exports` 桶；Desktop 给“下载 `.pikg`”与“直接入库”（后者复用 `ext.install { pikg }`）。不签名（私钥在用户手里），签名仍是 CLI `pikg sign` |
| Q21 | 文件导入与交换格式 | 没有 `.block.json` / `.block.zip`；交换载体是 bundle 目录 + `.pikg`；Desktop 没有“从文件导入” | 不再做 `.block.*`。Desktop “导入 `.pikg` 文件”= 上传到自己的 `exports` 桶 → `ext.install { pikg: { bucket_id, key } }`（服务已支持，零新 RPC）；bundle 目录只给 CLI / Agent | **按建议** |
| Q22 | 内联上限与大资源 | `ext.build` 一次内联 ≤ 4 MiB（整次 build 累计）；`assets` 文件进源码快照；超限直接 `BUILD_FAILED` | 超限文件由客户端先 `ws.stage_object`（或资产上传）成对象再按 `obj_id` 引用，build 入参不变；上限保留，不做分批 | **按建议**，并要求 hello-controls 例子说明**自带资源的控件**的开发逻辑（已补：README 第 6 步“自带资源”，含今天 HTML 运行时拿不到 `assets` 的事实与 P1 `aiws.asset()` 提案） |
| Q23 | 源码子目录 | `buckyos.dir` 成员名不能含 `/`，源码只能平铺 ≤ 64 文件；组合控件与带 `assets/` 的 HTML 控件会要子目录 | P1 随 composite 一起：成员名允许 `/`（成员名编码相对路径，不做嵌套 dir），README / schema / `pull` / bundle `controls/<dir>/` 同步 | **按建议，先简单**：成员名编码相对路径即可，不做嵌套目录 |
| Q24 | 独立域名上的开发操作 | Desktop 在 `aiws-<id>.<zone>` 上按 `onWorkspaceHost()` 关闭新建 / 编辑 / 构建 / 写 bundle（watch 同），库引入与“全局可用”照常 | 维持关闭并提示“到桌面里打开” | **放开**。用户判断 `/nfs/v1` 这类接口按 URL 路由、不看子域名。对照源码成立：网关 `get_app_info_from_req` 之后仍执行 `get_service_info_from_req`，`/nfs/v1/*` 无条件改写 `SERVICE_ID=nfs-server`（boot_gateway.yaml）；子域会话由 `prepare_app_launch { service_id }` 委托，token 必须是 `principal_kind: user`（sys_auth_backend.rs），nfs-server 文件面把它当 `Actor::User`，不触发 D13 对 App / Agent 会话的拒绝。P0 的“home 不可达”是 Desktop 侧假设，不是网关事实。P1：去掉 `onWorkspaceHost()` 门，在真实子域 DV 新建 / build / watch / 写 bundle |
| Q25 | 作者 DID 的真实来源 | Desktop 默认 `did:bns:<用户名>`、CLI `--owner` 同；许愿格 `did:aiws:<workspace_id>`；身份一旦进库 / 包即固定 | 用户 DID 在会话里可读后自动填真实 DID；旧默认值保持可读，不做迁移 | **不做**。作者 DID 在本工作区内只需进本工作区的信任名单即可（它是身份标签，不验真）；真给别人用时走 PIKG 标准签名流程，作者由包签名证明。因此不自动填、不迁移，R10 关闭 |

**裁决回写**：Q20 把 `.pikg` 生成从 Desktop / websdk 挪到服务（§3.3、§6.8、§6.10 第 8 步、§8 L4）；Q21 不变；Q22 落到 §8 L5 与 hello-controls；Q23 留在 P1 组合控件末尾；Q24 由“维持关闭”改为 P1 放开并 DV（§1.5、§7.5.4 R9、§8 L5）；Q25 关闭 R10，§8 L5 不再含“作者 DID 真实来源”。

## 8. 建议的实施顺序与验收

| 阶段 | 内容 | 验收 |
| --- | --- | --- |
| P0 | **2026-10-30 已实施（两仓未提交）**：render 定义（declarative / html）、`ext.build / export / install / list_library / uninstall / library_bundle / import`、本机库（`library.sqlite` + 每用户 `library` 桶）、`AppType::Aiws` + Installer 路由、websdk `aiws_ext` + CLI、Desktop 工作区控件对话框与实例初始化表单、手册。落点、测试与 DV 见 [扩展控件实施记录与验收报告](<BuckyOS AI Workspace 扩展控件实施记录与验收报告.md>)，出入见 [任务单 §12.1](<../../notepads/aiworkspace-extension-p0-todo.md>) | 已验收：build 后实例更新、失败不提交、基线冲突拒绝、收窄列实例、`unchanged` 不提交；两账号各自从库引入同一工作区，未装包一方可插实例；CLI 全链路与 `app install` 入库不建 AppSpec。未验收项见下方“P0 遗留” |
| P1 | **2026-10-30 已实施**：内核（Q15 拆层、`kind: composite`、composite 容器与 slot、`def_dep` + `REFERENCE_CYCLE`、`tree.clone`、成员名含 `/`）、服务（`composition.json` 校验、`deps` 解析与 `DEP_MISSING`、依赖者重校验、导出 / 库 / 引入的依赖闭包、`ext.pack`）、websdk / CLI（`manifest_version: 2`、`checkComposition`、递归源码、超限上传、`pikg init --composite`）均已落地并测试通过；Desktop 通用 composite 宿主与各入口、DV 结果以 [扩展控件实施记录与验收报告](<BuckyOS AI Workspace 扩展控件实施记录与验收报告.md>) P1 节为准，与任务单的出入见 [P1 任务单 §13](<../../notepads/aiworkspace-extension-p1-todo.md>)。原计划：**任务单：[notepads/aiworkspace-extension-p1-todo.md](<../../notepads/aiworkspace-extension-p1-todo.md>)（2026-10-30，M0 契约冻结 → M1 遗留 → M2 内核 → M3 服务 → M4 websdk/CLI → M5 Desktop → M6 手册/DV/例子回写）。前段：P0 遗留**（下表 L1–L6）。**主体：组合控件**——Q15 拆 `types.rs` 校验层（先于放开 `kind`）→ `kind: composite` + `composition.json`（模板外观、slots、具名布局、属性映射）在 `store/src/ext.rs` 与 `aiws_ext.ts` 双实现校验 → 定义 → 定义依赖边与 `deps` 非空（Q16：依赖环拒绝、依赖者兼容检查扩到定义、导出闭包带依赖）→ 内核级子图重映射（Q17）→ Desktop 通用 composite 宿主、Chrome / slots、fixed 及按需 stack、“定义为控件”→ 源码子目录（Q23） | 两个客户卡片实例分别绑定数据，改 `composition.json` 并 build 后布局 / 外观更新，私有内容不串改或重建；复制子节点不变成脑图。混选 render 与组合定义导出，共享依赖去重，在新 Workspace 可分别实例化；被依赖控件收窄接口时 build 列出依赖定义并失败 |
| P2 | **任务单（与 P3 合并，2026-10-30；Q26–Q33 用户同日裁决全部按建议，含 Q32 模板改快照式）：[notepads/aiworkspace-extension-p2p3-todo.md](<../../notepads/aiworkspace-extension-p2p3-todo.md>)。** 公共事件绑定与脚本行为（HTML host API 升 3、保留 2）、设计 / 运行边界、属性级协作与撤销、显式来源包升级与结构迁移（替换今天的 `EXT_IDENT_EXISTS` 拒绝：同身份异来源进入升级流程，列本地改动与受影响实例）、执行幂等、离线旧定义检查 | 甲改颜色、乙改文字均保留；一次点击只执行一次，改行为文件并 build 后下一次点击用新实现，重载不重放旧动作。包升级 / 迁移保留实例覆盖与内容；旧客户端写入冲突可诊断 |
| P3 | **任务单同上（M4–M6；模板按 Q32 建议改为快照式）。** HTML `parent`、更多布局、Agent 组合结果；移动端、工作区发布 / 回放和宿主能力不足的验证；Workspace 模板包（创建 / 实例化，复用 Q17 重映射与 P0 的 `workspace_templates` 预留） | Agent 通过手册发现 / 引入 / 实例化控件；缺少宿主能力时可读回退且数据保留；公共端 / 回放不执行现场业务动作；模板：安装不建工作区、两次创建内容独立、定义与实例引用正确重映射 |

Workspace 模板包的完整导出、目录与实例化已排入 P3；P0 的清单预留（`workspace_templates`，非空即拒绝）与 `ext.import` 的引入流程是它的基础，不另建打包格式。

**P0 遗留（P1 前段，先于组合控件）**

| # | 项 | 来源 |
| --- | --- | --- |
| L1 | 三项 DV：跨 Zone 中转引入（三 Zone VM 组）、Agent 只凭手册在本地目录建控件并 `build`、旧 `manifest_version` 包（先造一个兼容夹具）；记录 Agent 访问 aiworkspace / nfs-server 的路径，不可达按 bug 修 | 任务单 §9 (3)(4)(5) |
| L2 | M5 完成条件的“丙”：第三个未装包账号在共享工作区看到并插入 A / B / C / D 四个控件（DV 只跑了两账号、两个控件） | 实施记录 §5 |
| L3 | 导出 / 入库前检测 home 有未 build 改动并提示（对比 `collectSources` 的内容摘要与 `derived.build.input_digest`） | 本节回归项，未实现 |
| L4 | 服务端 `ext.pack` 生成 `.pikg`（Q20：Desktop 经 krpc 调用、产物进 `exports` 桶、与 CLI `pikg pack` 字节一致）与 Desktop 导入 `.pikg` 文件（Q21：上传 `exports` 桶 → `ext.install`） | §7.6 |
| L5 | 超限资源走对象引用（Q22）；独立域名放开新建 / build / watch / 写 bundle 并在真实子域 DV（Q24）；HTML 运行时向 iframe 提供 `assets`（`aiws.asset(name)`，hello-controls 第 6 步提案） | §7.6 |
| L6 | 提交两仓改动；既有无关失败（control_panel `sso_identity` scopes、Desktop i18n 两项、e2e CN17 与 `i18n.spec.ts:133`）单独处理 | 实施记录 §4 |

跨阶段必须保留以下回归：

- 本机 PIKG 更新或卸载后，既有 Workspace 保持原定义；显式来源升级才采用新包。build 只改变本工作区，不反向修改本机库或其他工作区。
- 工作区 Fork / 自包含导出携带控件实现、依赖和资源；贡献者离线不影响已携带内容。离线读取仍受本地缓存完备程度约束，联网控件如实显示数据不可用。
- 引入（`ext.import`）与 build 都是在线 RPC（P0）；离线只渲染已引入定义，不承诺离线提交定义。
- build 的 `unchanged`（输入摘要相同）与 watch 的 debounce 不产生空 Commit；`derived.build` 只能由 build / import / 撤销写入，人的 Commit 不能冒充构建记录。
- `ext.install` 三种来源（自己的 `exports` 桶、Installer 的 staged 对象、工作区直接入库）得到同一库记录与 `digest`；同 `pkg_id` 同版本异内容拒绝；新版本替换旧版本而 Workspace 保留副本。
- 控件定义更新保留实例覆盖与内容；独立片段更新不传播到已插入对象。两者分别验收。
- 默认值、必填 config、多个命名绑定和事件参数的初始化均有覆盖，不能只用无配置的 KPI 卡证明可安装控件模型成立。
- 编辑 render 行为或组合描述文件并 build 后，无需重新打包、引入、插入实例或整页刷新；未激活实例下次采用新实现，旧快照可辨认。watch 模式下保存文件即自动 build。
- 错误源码可保存在 home，但 build 失败不提交、不覆盖 Workspace 中的实现；新加入协作者 `pull` 到的就是当前实现。被依赖控件 build 时同 Workspace 内依赖者一并校验，不兼容则整次失败；基于旧快照的 build 被 `expect` 拒绝并提示 pull，不静默覆盖他人改动。
- 连续 build、撤销、重连和实例重载均不重复触发已提交业务动作；运行中的动作引用的实现修订仍可读取。
- 多选控件导出的本机库内容与 PIKG 内容一致；包带齐共享依赖及源码，开发 Workspace 离线时仍能在新 Workspace 引入，实例业务数据不随包混入。
- 后续 build 不修改已导出的控件包或本机库；再次导出使用明确的新修订 / 版本。导出内容始终是 Workspace 快照，home 有未 build 改动时必须提示；未签名导出、本机复用和后续签名 / 发布分别验收。
- PIKG 新用途在 TS / Rust 校验和安装消费方保持一致，篡改 / 缺失依赖均失败；控件包安装不创建 AppSpec 或启动服务。签名与发布验收单独证明，不能以 pack / info 成功代替。
- 模板预留覆盖“带自定义控件”和“只用内置控件”两种结构，未支持的模板能力明确显示。后续模板功能验收至少包含：安装不建工作区、两次创建内容独立、定义与实例引用正确重映射。

## 9. 参考

- [hello-controls/](<hello-controls/README.md>)：典型开发流程 demo 与源码形态（P0 / P1 部分已过预检，`hello-board` composite 已冻结；事件 / 模板为提案），§6.10 的配套。
- [扩展控件实施记录与验收报告](<BuckyOS AI Workspace 扩展控件实施记录与验收报告.md>)：P0 落点、测试、DV 与未完成；[P0 任务单 §12.1](<../../notepads/aiworkspace-extension-p0-todo.md>)：实现中的 13 条出入；[P1 任务单](<../../notepads/aiworkspace-extension-p1-todo.md>)：P0 遗留 + 组合控件（M0–M6）。
- 权威契约：[aiworkspace README “Extension controls”](<../../src/frame/aiworkspace/README.md>)、`schemas/{component,composition,aiws-manifest,block-def}.schema.json`；共享驱动 [buckyos-websdk `src/aiws_ext.ts`](<../../../buckyos-websdk/src/aiws_ext.ts>)；服务 `store/src/{ext,library}.rs`、`server/src/ext.rs`；PIKG `aiws` 类型 [app_doc.rs](<../../src/kernel/buckyos-api/src/app_doc.rs>) 与 [control_panel/src/aiws_pkg.rs](<../../src/frame/control_panel/src/aiws_pkg.rs>)；Desktop `ui/extensions/{ExtensionsDialog,InstanceForm}.tsx`。
- [脑图对象方案讨论](<脑图对象方案讨论.md>) v0.4：§4.3、§5.7、§6.2、§8.1、R21–R24。
- [内核架构改进](<BuckyOS AiWorkspace 内核架构改进.md>) D01（方案 C：原语 / 业务 Schema 分层）、T0-1（校验器分类表）、D06 / D19（保留与配额）；[Durable Data Schema](<BuckyOS AiWorkspace Durable Data Schema.md>)：三桶与保留集；[核心设计 §3 现状说明](<BuckyOS AI Workspace 实现的核心设计.md>)、[第一期内置对象详细设计 §4.4](<BuckyOS AI Workspace 第一期内置对象详细设计.md>)：`.bcanvas` 权威格式。§7.3 的结论依据这些文本。
- 内核引用边与派生更新：[RefEdge / refs_to](<../../src/frame/aiworkspace/core/src/model.rs>)、[entity.set_derived 与删除时的引用检查](<../../src/frame/aiworkspace/core/src/plan.rs>)。
- [脑图实施记录与验收报告](<脑图实施记录与验收报告.md>)：§2 交付物（文件落点）、§6 未完成、§8 DV。
- [第二期规划](<BuckyOS AI Workspace 第二期规划.md>) §10：两个扩展点、模式正交、Block 定义实体、HTML 扩展（D16）、扩展样本。
- [UI 改进](<BuckyOS AI Workspace UI改进.md>) §6.2–§6.3、§7.1–§7.2：新建 / 插入对话框、左侧工具栏、插入状态。
- [PIKG 文档入口](<../pikg/README.md>)：构建、包格式、安装与签名 / 发布的职责归属；本文新增 AIWS 控件导出契约，不另行定义 PIKG 格式。
- [PIKG 当前格式](<../pikg/format.md>)、[AppDoc 类型](<../../src/kernel/buckyos-api/src/app_doc.rs>)、[PIKG reader](<../../src/frame/control_panel/src/pikg.rs>)、[安装分派](<../../src/frame/control_panel/src/app_installer.rs>)：当前类型、内容校验和消费方边界；新增 Workspace 用途需联动实现。
- [Workspace 导出](<../../src/frame/aiworkspace/store/src/export.rs>)、[内置模板创建](<../../src/frame/desktop/src/app/aiworkspace/api/demos.ts>)：已有内容快照与创建基础，不等于已支持 PIKG 模板。
- Text Editor 源码：[存储与版本检查](<../../src/apps/text_editor/src/fs/documentStore.ts>)、[文档可写性与保存流程](<../../src/apps/text_editor/src/model/workspace.ts>)、[内容打开与变更检查](<../../src/apps/text_editor/src/main.tsx>)。按 A1 零改动：源码经 `cyfs-path` 普通保存。[NFSP 客户端](<../../../buckyos-websdk/src/nfsp.ts>) 的 `list` / `uploadFile` / `watch` 供 build / pull / watch 驱动（P0 实际由客户端读字节内联，`NodeInfo.obj_id` 只对 `cyfile:` 对象使用）。
- 工作区文件与重载基础：[dir.replace / dir.set_member](<../../src/frame/aiworkspace/core/src/plan.rs>)（build 的 commit 复用其 `expect`）、[NDM 网关授权取对象](<../../src/frame/aiworkspace/server/src/lib.rs>)（`authorize_object`，`ext.build` 取字节复用）、[Workspace 变更订阅](<../../src/frame/desktop/src/app/aiworkspace/state/store.ts>)、[HTML 宿主](<../../src/frame/desktop/src/app/aiworkspace/ui/extensions/HtmlBlockHost.tsx>)。
- 源码：`src/frame/desktop/src/app/aiworkspace/ui/{blocks/registry.ts,controls/registry.ts,canvas/catalog.ts,canvas/InsertCatalog.tsx,extensions/*}`；`src/frame/aiworkspace/{controls/,core/src/types.rs,README.md}`；`buckyos-websdk/cli/modules/aiws.ts`。
