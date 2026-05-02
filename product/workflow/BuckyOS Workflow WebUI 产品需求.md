# BuckyOS Workflow WebUI 产品需求

> 版本：v0.2（与 [workflow service.md](../../doc/workflow/workflow%20service.md) v0 / [wokflow engine.md](../../doc/workflow/wokflow%20engine.md) v0.4 对齐）
> 日期：2026-05-02
> 状态：草案 / 待评审

---

## 1. 背景

BuckyOS 中的 Workflow 是一套用于定义、编排和执行自动化处理管线的能力。Workflow 由两层组件支撑：

- **Workflow Engine（双层架构）**：编排器（Expr Tree Evaluator）+ Thunk 调度器，定义见 [wokflow engine.md](../../doc/workflow/wokflow%20engine.md)。
- **Workflow Service**：Zone 内的标准服务，负责 Workflow Definition 管理、Run（运行实例）驱动，并把 Run / Step / Thunk / Map shard 映射为 task_manager 任务树。设计见 [workflow service.md](../../doc/workflow/workflow%20service.md)。

按 Workflow Service 的设计原则，**Workflow 运行期的所有交互（进度、待办、审批、回退、通知）都由 TaskMgr UI + OS 通知中枢承担**；用户对一次 Run 的干预（approve / modify / reject / retry / skip / abort / rollback）通过在 TaskMgr UI 中写对应 Step / Run task 的 TaskData 完成，不调用 workflow 私有 RPC。

因此 Workflow WebUI 的职责非常聚焦：**它只承担与"运行"无关的部分** —— Definition 的查看、导入、绑定，以及 Workflow Graph（DSL 拓扑）的可视化。所有运行期数据钻取一律跳转到 TaskMgr UI。

Workflow 通常不作为独立产品存在，而是挂载到具体应用（如 KnowledgeBase、AI 文件系统、Script App）声明的挂载点上，由应用决定何时触发 `workflow.create_run`。WebUI 的核心价值是为高级用户、应用开发者、Agent 提供一个高信息密度的 Definition 管理与挂载点配置入口。

---

## 2. 产品定位

Workflow WebUI 是 BuckyOS 中面向高级用户的 Workflow Definition 管理控制台。

核心定位：

1. **Definition 视图，不是运行视图**：WebUI 展示的是"应用挂载了什么 Workflow（Definition）、Definition 的图结构是什么"；运行期状态（Run / Step / Thunk 进度、错误、待办）一律由 TaskMgr UI 承担。
2. **以查看为主**：用户首先要看清楚有哪些 Definition、绑定到哪些应用挂载点、图结构是什么、节点配置是什么、最近有哪些 Run。
3. **高信息密度**：目标用户偏高级，需要在有限界面中快速理解 Workflow 拓扑与挂载关系。
4. **Definition 与挂载点绑定分离**：Workflow Definition 是引擎一等资源（DSL + 编译产物 + 静态分析报告）；"挂载点绑定"是产品层数据，把 Definition 引用到 App 声明的某个挂载点上。
5. **应用驱动**：Workflow 通常挂在应用声明的挂载点上，由应用调用 `workflow.create_run` 触发执行。
6. **为编辑能力预留空间**：当前版本以只读 + 导入为主，但 UI 与数据结构需要为节点参数编辑（生成新 Definition version）和未来 Amendment 协助界面留出扩展。
7. **AI 辅助创建**：不鼓励用户从零用图形界面创建 Workflow，而是提供提示词，引导用户使用 ChatGPT 或其他现代 AI 工具生成符合 schema 的 Workflow DSL/JSON。

---

## 3. 目标用户

### 3.1 高级用户 / 系统管理员

需要查看系统中 Workflow 的挂载关系、Definition 静态分析告警，并能替换某些应用的处理管线。

典型诉求：

- 查看某个应用当前挂载点上绑定了哪个 Definition。
- 查看某个 Definition 的节点结构与节点配置。
- 导入别人提供的 Workflow DSL/JSON。
- 把某个 Definition 绑定到应用的指定挂载点上。
- 从挂载点视图跳转到 TaskMgr UI 查看最近的 Run 详情。

### 3.2 应用开发者

在开发应用时，需要声明 Workflow 挂载点，并确认应用注册的 Definition 是否正确出现在系统中。

典型诉求：

- 确认应用安装后是否正确通过 `workflow.submit_definition` 注册 Definition。
- 检查应用声明的挂载点是否可见，默认绑定是否生效。
- 调试应用触发的 Run：从 WebUI 跳转到 TaskMgr 查看 Run 任务树。

### 3.3 Agent / 自动化构建者

使用 Agent 创建 Script App、生成 Workflow DSL，并把 Definition 挂载到应用或脚本应用中。

典型诉求：

- 通过提示词生成符合 Workflow Engine schema 的 DSL/JSON。
- 调用 `workflow.dry_run` 自检后再 `submit_definition`。
- 自动创建 Script App 与挂载点绑定。
- 在 WebUI 中查看 Agent 创建出的 Definition 结构。

---

## 4. 核心概念

### 4.1 概念表（与引擎/服务对齐）

| 概念 | 说明 | 引擎 / 服务对应 |
|---|---|---|
| Workflow Definition | DSL/JSON 形式的 Workflow 定义。Service 入库前必须通过 `compile_workflow` + `analyze_workflow`，分析报告中存在 Error 级别 issue 时拒绝写入；同名 Definition 多次更新形成只增不减的 version 链。 | `WorkflowDefinition`，`workflow.submit_definition` 主键 |
| Definition Status | Definition 的生命周期状态：`draft` / `active` / `archived`。`archived` 不允许新建 Run，对已有 Run 无影响。 | Service 资源模型 §2.1 |
| Analysis Report | 编译期 + 静态分析输出，含 Error / Warn / Info 三级 issue（引用闭合、死节点、idempotent 一致性、output_mode 与 for_each 兼容性等）。 | Service `analysis` 字段 |
| App Workflow Mount Point | 应用在 manifest 中声明的 Workflow 挂载点，包括 id、说明、是否必需、是否允许为空、默认 Definition 引用。 | 产品层概念，建议通过 system_config 注册 |
| Mount Point Binding | "把某个 Definition 绑定到某个 App 挂载点" 的产品层元数据。UI 上称作"应用 Workflow 实例"。**注意：这不是引擎的运行实例，仅是引用关系；真正的执行实例是每次触发产生的 Run。** | 产品层数据；旧 PRD 中"Workflow Instance"实际指此 |
| Workflow Run | 一次具体执行。Run 持有节点状态、节点输出、事件流、人类介入记录；状态枚举：`Created` / `Running` / `WaitingHuman` / `Completed` / `Failed` / `Paused` / `Aborted` / `BudgetExhausted`。 | 引擎 `WorkflowRun`、Service `workflow.create_run` |
| Run Task Tree | 每个 Run 在 task_manager 中对应一棵任务树：`workflow/run`（根） → `workflow/step` / `workflow/map_shard`（子） → `workflow/thunk`（孙）。运行期 UI 由 TaskMgr UI 承担。 | Service §6.3 |
| TaskNode（Step） | Workflow Graph 中的执行节点，对应一个 Step。具有 executor、input、output、type（`autonomous` / `human_confirm` / `human_required`）、`output_mode`、`idempotent`、`skippable`、`guards` 等字段。 | DSL §3.2，Expr Tree `Apply` / `Await` |
| ControlNode | Workflow Graph 中的流转控制节点：`branch`（穷举枚举分支）、`parallel`（并行 + join 策略）、`for_each`（受 `output_mode` 约束的有界迭代）。 | DSL §3.4，Expr Tree `Match` / `Par` / `Map` |
| Edge | Workflow Graph 中的有向连接。`branch.paths` / `parallel.branches` 是隐式边，不重复声明。 | DSL §3.5 |
| Executor Reference | 节点的 executor 字段。实际定义形如 `service::aicc.complete` / `http::x.y` / `appservice::z` / `operator::json.pick` / `func::<objid>`；语义链接形如 `/agent/mia` / `/skill/fs-scanner` / `/tool/...`，运行前由 registry 展开。 | DSL §3.2，Service §6.1 |
| Output Mode | `single` / `finite_seekable` / `finite_sequential`，决定下游 for_each 是否可并行/重试粒度。 | DSL §3.2.1 |
| Guards | 节点或 Workflow 级约束：`budget`（max_tokens / max_cost_usdb / max_duration）、`permissions`、`retry`（max_attempts / backoff / fallback）。 | DSL §3.7 |
| Human Action | 用户对 Step / Run task 写入的 TaskData，含 `human_action.kind`：`approve` / `modify` / `reject` / `retry` / `skip` / `abort` / `rollback`。**入口在 TaskMgr UI，不在 Workflow WebUI。** | Service §3.3 |
| Amendment | 运行时计划修改协议。Agent 在执行中可提交对当前 Run 的图修改（`insert_after` / `insert_before` / `replace` / `remove`），需审批后生成 `plan_version + 1`。 | DSL §3.8，Service `workflow.submit_amendment` |
| Script App | 一类用户 / Agent 创建的轻量应用，用于声明 Workflow 触发点和挂载点；在 WebUI 中与普通应用同列展示。 | 产品概念 |

### 4.2 与运行期的清晰边界

Workflow WebUI **不展示**：单次 Run 的 Step 列表、Thunk 进度、节点执行 payload、错误栈、待审批表单、回退按钮、通知。这些一律在 TaskMgr UI 中通过任务树呈现。

Workflow WebUI **展示**：Definition 的图结构、节点配置、挂载关系、最近 Run 列表（仅作索引，每条都跳转到 TaskMgr UI）、静态分析报告、AI 提示词、导入/绑定流程。

---

## 5. 产品目标

### 5.1 MVP 目标

1. 用户可以在左侧组织树中查看 Workflow Definition 库（含状态：draft / active / archived）和各应用下的挂载点绑定。
2. 用户可以选中一个 Definition 或挂载点，在主区域以图形方式查看 Workflow Graph（基于 Expr Tree 展开的 nodes + edges）。
3. 用户可以点击节点查看节点配置（按 TaskNode / ControlNode 分别呈现）。
4. 主区域同时展示该 Definition 的静态分析报告（Error / Warn / Info）。
5. 只读模式为默认模式，移动端也可使用。
6. 桌面端在挂载点视图下展示该挂载点最近 N 条 Run 列表（来自 `workflow.list_runs`），每条跳转到 TaskMgr UI 中对应的 `workflow/run` 任务。
7. 用户可以导入 Workflow DSL/JSON 文件、URL 或粘贴文本，服务端通过 `workflow.dry_run` 编译并做静态分析，校验通过则 `submit_definition` 入库。
8. 合法导入的 Definition 默认进入 `draft` 状态的 Definition 库。
9. 用户可以把 Definition 绑定到应用声明的挂载点上，覆盖默认绑定或恢复默认。
10. 用户可以获取一段用于 AI 生成 Workflow DSL/JSON 的提示词（含当前系统的 schema_version、可用 executor 列表、Reference 文法、示例）。
11. UI 与接口设计需要为未来"节点参数编辑生成新 Definition version"和"Amendment 提议入口"预留扩展。

### 5.2 非目标 / 暂不实现

1. 不提供完整的从零图形化创建 Workflow 能力。
2. 不提供复杂的拖拽式 Workflow 图编辑器。
3. **不在 Workflow WebUI 中展示运行期任务面板（待办、审批、进度条、回退按钮）**——这一面完全归 TaskMgr UI。
4. 不重新实现 task_manager 已经提供的 pause / resume / cancel / 进度上报等通用能力。
5. **不在 Workflow WebUI 中实现 HumanAction 录入**。用户处理 `WaitingHuman` 节点的入口是 TaskMgr UI（写 TaskData）；WebUI 仅可在 Run 列表上显示是否处于等待状态，并提供 deep link 跳转。
6. 不实现复杂的 Script App 可视化开发器。
7. 不承诺提供非常完善的编译错误定位；至少需要返回 AnalysisReport 中的 issue 列表（含 severity、message、可选 node_id / path）。
8. 不实现 Amendment 的可视化提交器（一期 Amendment 由 Agent 通过 RPC 提交；WebUI 可只读展示版本链）。

---

## 6. 信息架构

### 6.1 总体布局

桌面端采用三块核心区域：

```text
┌─────────────────────────────────────────────────────────────┐
│ 顶部栏：标题 / 当前 Definition 或挂载点 / 操作按钮 / 模式切换 │
├───────────────┬─────────────────────────────────────────────┤
│ 左侧组织树     │ 主区域：Workflow Graph / 节点配置面板         │
│               │                                             │
│ - Definitions │ React Flow 画布                              │
│ - App A       │ 选中节点后显示节点配置                         │
│ - App B       │ 顶部条带显示静态分析摘要                       │
├───────────────┴─────────────────────────────────────────────┤
│ 底部区域：最近 Run 列表（仅挂载点视图，桌面端默认显示）         │
│   每条 Run 显示状态摘要 + "在 TaskMgr 中打开"链接              │
└─────────────────────────────────────────────────────────────┘
```

移动端采用全屏查看优先：

```text
┌─────────────────────────────┐
│ 顶部栏：返回 / 当前 Definition │
├─────────────────────────────┤
│ Workflow Graph 全屏显示       │
│                              │
│ 节点配置：Bottom Sheet        │
│ 组织树：抽屉式侧栏            │
└─────────────────────────────┘
```

### 6.2 左侧组织树

一级目录包括：

1. **Definitions（Definition 库）**
   - 存放所有 `draft` / `active` 状态的 Definition；`archived` 默认折叠或过滤。
   - 系统内置 Definition 也保留在此（标记 source=`system`）。
   - 用户导入的 Definition 默认进入此处（source=`user_imported`，初始状态 `draft`）。
   - 应用注册（source=`app_registered`）和 Agent 生成（source=`agent_generated`）的 Definition 也在此聚合。

2. **应用目录**
   - 每个已安装的应用一个目录。
   - 目录下展示应用 manifest 声明的 Workflow 挂载点。
   - 挂载点下展示当前绑定的 Definition；未绑定的挂载点展示为空状态（区分 `allowEmpty: true/false`）。

3. **Script Apps**
   - Script App 与普通应用并列展示。

示例：

```text
Workflow
├── Definitions
│   ├── kb_default_import_pipeline (system, active)
│   ├── image_thumbnail_pipeline (system, active)
│   └── my_imported_pipeline (user_imported, draft, ⚠ 2 warnings)
├── KnowledgeBase
│   ├── document_import_pipeline ←→ kb_default_import_pipeline
│   └── log_process_pipeline ←→ Empty (optional)
├── AI File System
│   ├── thumbnail_generate_pipeline ←→ image_thumbnail_pipeline
│   └── file_index_pipeline ←→ custom_file_index_pipeline
└── Script Apps
    └── File Organizer
        └── default_pipeline ←→ file_organizer_pipeline
```

### 6.3 组织树节点状态

| 状态 | 展示要求 |
|---|---|
| Definition: `draft` | 草稿图标，提示"未发布，应用挂载点列表中不出现" |
| Definition: `active` | 正常图标，可被绑定到挂载点 |
| Definition: `archived` | 灰色图标，默认不展示 / 折叠 |
| Definition 静态分析有 Error | 红色错误图标 + 计数；不允许绑定 |
| Definition 静态分析有 Warn | 黄色警告图标 + 计数 |
| Definition 来源 | `system` / `user_imported` / `app_registered` / `agent_generated` 用 badge 区分 |
| Mount Point: 已绑定 | 展示绑定的 Definition 名称 + version |
| Mount Point: 未绑定（allowEmpty） | "Empty"，提供"从 Definitions 添加"入口 |
| Mount Point: 未绑定（!allowEmpty） | 醒目提示"必需挂载未配置" |
| Mount Point: 系统默认 | 标识 System Default，便于替换后恢复 |
| Mount Point: 最近 Run 失败 | 在挂载点旁标注 Run 失败摘要 + 跳转 TaskMgr |

---

## 7. 核心页面与交互

### 7.1 Definition / 挂载点查看页

#### 页面目标

让用户选中一个 Definition 或挂载点后，可以快速理解其图结构、节点配置、静态分析与最近 Run。

#### 页面组成

1. 顶部栏
   - 当前对象：Definition 名称（含 version、状态、来源）或 "AppX / mount_point_id"。
   - 当前模式：Read-only / Edit（一期仅 Read-only）。
   - 操作按钮：导入 Definition、绑定到挂载点、替换绑定、恢复默认绑定、复制 AI 提示词、查看版本链、更多。

2. 静态分析摘要条
   - 显示 Analysis Report 中的 Error / Warn / Info 计数。
   - 点击展开 issue 列表（severity、message、node_id、path）。

3. React Flow 画布
   - 展示 Workflow Graph 的 nodes + edges。
   - TaskNode 与 ControlNode 用不同形状/颜色区分；ControlNode 三种类型（branch / parallel / for_each）各有独立标识。
   - 隐式边（branch.paths、parallel.branches）显式画出，并标注条件值（如 `decision == "approved"`）。
   - `human_confirm` / `human_required` 节点用人形图标突出显示。
   - `output_mode != single` 的 TaskNode 标注模式（如"Seekable / 47k items"）。
   - 支持缩放、拖动画布、fit view、定位到选中节点。
   - 默认只读，不允许拖拽改变结构。

4. 节点配置面板
   - 点击节点后显示。
   - 一期只读；编辑预留参考 7.2 §编辑预留。
   - 字段按 TaskNode / ControlNode 分别呈现，详见 7.2。

5. 最近 Run 列表（仅挂载点视图）
   - 数据源：`workflow.list_runs` filtered by `(definition_id, mount_point)`。
   - 每条仅展示 Run 摘要（run_id、status、trigger_source、started_at、duration、错误摘要）+ "在 TaskMgr 中打开" 深链。
   - **不展示 Step 级详情、不展示进度条、不提供 approve/retry 按钮**——这些都在 TaskMgr UI 完成。
   - Definition 视图（未绑定到挂载点）下不展示运行记录。

#### 交互要求

- 选中左侧树中的对象后，主区域加载图结构。
- 选中节点后，右侧或浮层展示节点配置。
- 点击画布空白处取消节点选中。
- 只读模式下所有图结构和节点参数均不可修改。
- 当对象为 Definition 时，不展示 Run 列表，仅展示"该 Definition 当前被哪些挂载点引用"。
- 当对象为挂载点时，展示绑定的 Definition 图 + 最近 Run 列表。
- 数据加载失败时展示错误状态和重试入口。

---

### 7.2 节点配置面板

#### 页面目标

让用户理解某个节点在 Workflow Graph 中的定义、参数和行为。

#### TaskNode（Step）属性分组

| 分组 | 字段 |
|---|---|
| 基本信息 | `id`、`name`、`description`、Step `type`（autonomous / human_confirm / human_required） |
| Executor | `executor` 字段、是否为语义链接（`/agent/`、`/skill/`、`/tool/`）、registry 展开后的实际定义、namespace（service / http / appservice / operator / func） |
| 输入 | `input_schema`、`input` 中的字面值与 Reference（`${node.output.field}`）、Reference 解析的来源节点 |
| 输出 | `output_schema`、`output_mode`（single / finite_seekable / finite_sequential）、运行时进度字段（仅展示 schema 解释，不展示真实进度——真实进度在 TaskMgr UI） |
| 行为 | `idempotent`、`skippable`、`subject_ref`（仅 human_confirm）、`prompt`（仅人类节点） |
| Guards | `budget.max_tokens` / `max_cost_usdb` / `max_duration`、`permissions`、`retry.max_attempts` / `backoff` / `fallback` |
| 静态分析告警 | 与本节点相关的 Analysis Report issue（含 severity） |
| Schema 信息（折叠） | input / output schema 的完整 JSON Schema 视图、`$ref` 来源、`$defs` 引用 |

#### ControlNode 属性分组

- **branch**：`on`（引用的字段路径）、`paths`（穷举的枚举值 → 目标节点）、`max_iterations`、当前是否存在静态分析告警（如未穷举）。
- **parallel**：`branches` 列表、`join` 策略（`all` / `any` / `n_of_m` 含 `n`）、输出合并示意（按分支 id 为 key 的 object）。
- **for_each**：`items`（引用的集合输出）、`steps`（迭代体）、`max_items`、`concurrency`、上游 `output_mode` 的实际值与"是否被静态分析降级"（finite_sequential 时强制 concurrency=1）、Checkpoint 语义说明。

#### 编辑预留

一期默认只读。技术结构需要支持后续按钮 / 表单切换：

- 修改后保存路径：调用 `workflow.dry_run` 校验 → 确认后调用 `workflow.submit_definition` 生成新 version（不就地修改已有 Definition；运行中 Run 仍绑定旧 version）。
- 也可"另存为新 Definition"。
- **不在 WebUI 内修改运行中 Run 的节点状态**——这通过 TaskMgr UI 写 `human_action` TaskData 完成；如需结构性修改，由 Agent 通过 `workflow.submit_amendment` 提交，UI 仅展示版本链。

---

### 7.3 最近 Run 列表

#### 页面目标

帮助用户从挂载点视角快速进入 TaskMgr UI 查看某次 Run 详情。

#### 数据来源

- `workflow.list_runs?owner=...&definition_id=...&mount_point=...&limit=20`
- 每条 Run 同时是 task_manager 中一个 `workflow/run` 任务的根。

#### 展示字段（仅摘要）

| 字段 | 说明 | 数据源 |
|---|---|---|
| Run ID | Run 主键，同时也是 root task id 关联键 | workflow service |
| 状态 | `Created` / `Running` / `WaitingHuman` / `Completed` / `Failed` / `Paused` / `Aborted` / `BudgetExhausted` | workflow service（同步自 task_manager） |
| 触发来源 | `app` / `manual` / `agent` / `system` | workflow service |
| 开始时间 / 结束时间 / 耗时 | — | workflow service |
| 等待人类节点数 | 非零时高亮 + 提示用户去 TaskMgr UI 处理 | workflow service `human_waiting_nodes` |
| 错误摘要 | 失败时简短错误 | workflow service |
| 操作 | 仅一个：在 TaskMgr 中打开 → 跳转 `taskmgr/<root_task_id>` | — |

#### 跳转行为

- 不在 WebUI 内展示 Step / Thunk / Map shard 详情；TaskMgr UI 通过任务树承担。
- 不在 WebUI 内提供 retry / cancel / approve 等按钮——这些通过 TaskMgr UI 写 TaskData 完成。

---

### 7.4 Workflow Definition 导入

#### 页面目标

让用户把外部 Workflow DSL/JSON 导入系统，作为 Definition 入库（默认 `draft`）。

#### 入口

- 左侧组织树顶部的"添加"按钮。
- Definitions 目录上的"导入 Workflow"。
- 空状态中的"导入第一个 Workflow"。

#### 导入方式

1. 本地文件导入：选择 DSL/JSON 文件 → 前端上传给服务端。
2. URL 导入：输入 URL → 服务端拉取（限制大小、超时、防 SSRF）。
3. 粘贴文本导入：直接粘贴 DSL/JSON（适合从 AI 工具复制）。

#### 导入流程

```text
用户提交 DSL/JSON
        ↓
服务端 workflow.dry_run（编译 + 静态分析）
        ↓
判断 AnalysisReport 是否有 Error
   ┌────┴────┐
   │         │
有 Error    无 Error
   │         │
展示 issue   workflow.submit_definition (status=draft)
列表 + 行号  返回 definition_id + 完整 AnalysisReport
   │         │
用户修改后   进入 Definitions 目录
重试         可继续执行"绑定到挂载点"
```

#### 编译校验返回

- `success`：bool
- `analysis.issues`：Issue 列表（severity: Error / Warn / Info、message、node_id?、path?、line/col?）
- 即使错误定位不完善，也必须明确告诉用户"导入失败"以及基础原因。

#### 导入成功后的默认行为

- 默认 `status = draft`，`source = user_imported`。
- 用户确认后可通过"激活"操作转为 `active`，或直接执行"绑定到挂载点"（绑定时如非 active 自动激活）。

---

### 7.5 从 Definition 绑定到应用挂载点

#### 页面目标

把某个 Definition 绑定到 App 声明的挂载点上，形成挂载关系。挂载关系是产品层元数据，触发执行时由应用调用 `workflow.create_run(definition_id, input)`。

#### 前提

- 应用 manifest 中声明了挂载点（id、说明、required、allowEmpty、defaultDefinitionId）。
- 系统启动 / 应用安装时，挂载点的默认绑定已注入。
- Definition 必须 `status = active` 且无 Error 级别静态分析 issue。

#### 典型场景

以 AI 文件系统为例：

- 应用声明 `thumbnail_generate_pipeline` 挂载点，默认绑定到 `image_thumbnail_pipeline`。
- 用户可换绑到自定义 Definition；如果挂载点 `allowEmpty=true`，也可解绑。
- 应用处理文件时检查该挂载点是否有绑定，有则 `workflow.create_run`，无则按内置兜底逻辑处理。

#### 交互流程

```text
用户选中 Definition
        ↓
点击"绑定到挂载点"
        ↓
选择目标应用 → 选择目标挂载点
        ↓
若挂载点已有绑定，提示替换影响
（展示当前 Definition 名称 / version / 来源；展示新 Definition 名称 / version；
 提示"已 / 仍未运行的 Run 不受影响（绑定锁定到创建时的 version）"）
        ↓
确认 → 写入挂载点绑定表
        ↓
左侧应用目录更新
```

#### 替换与恢复

- 替换前必须展示当前绑定与新绑定的 Definition 名称、version、来源。
- 系统默认绑定（`defaultDefinitionId`）保留恢复入口。
- 替换后可恢复到系统默认或上一次绑定。
- **替换不影响进行中的 Run**：每个 Run 在 `create_run` 时把 `definition_id + version` 锁定到自己的状态，绑定换了不会改写已存在 Run 的执行计划（与 Service §2.1 一致）。

---

### 7.6 AI 生成 Workflow 提示词

#### 页面目标

不要求用户从零理解 Workflow DSL，而是提供结构化提示词，引导用户通过 AI 生成可导入的 DSL/JSON。

#### 入口

- "新建 Workflow"按钮。
- Definitions 空状态。
- 导入弹窗中的"使用 AI 生成"。

#### 交互方式

点击"新建 Workflow"后不进入空白画布编辑器，而是展示一段提示词。提示词由服务端动态拼装（根据当前 zone 实际可用的 executor / schema_version），至少包含：

- 当前支持的 `schema_version`（来自 [wokflow engine.md] §3.1）。
- DSL 顶层结构：`id`、`name`、`trigger`、`steps`、`nodes`、`edges`、`guards`、`defs`。
- TaskNode 字段说明（含 `executor`、`type`、`input_schema`、`output_schema`、`output_mode`、`idempotent`、`skippable`、`subject_ref`、`prompt`、`guards`）。
- ControlNode 三种类型的字段说明（`branch.on/paths/max_iterations`、`parallel.branches/join`、`for_each.items/steps/max_items/concurrency`）。
- Reference 文法：`${node_id.output[.field[.sub]]}`，**不支持表达式 / 函数 / 字符串拼接**。
- Edge 模型：`{from, to}`，分支路径与并行分支隐式声明，无需重复在 edges 列出。
- 当前 zone 可用的 executor 列表（来自 executor registry：`service::*`、`http::*`、`appservice::*`、`operator::*`、`/agent/*`、`/skill/*`、`/tool/*`），含每个 executor 的 input/output schema 摘要。
- 命名规范、`output_mode` 与 `for_each.concurrency` 的兼容性约束。
- 一个完整可工作的示例（KB import 等）。
- 输出要求：只输出可解析的 JSON，不输出额外解释；遵循 schema_version。

#### 用户流程

```text
用户点击"新建 Workflow"
        ↓
系统展示动态拼装的提示词（含本 zone 可用 executor）
        ↓
用户复制提示词到 ChatGPT 或其他 AI 工具
        ↓
用户描述自己的 Workflow 需求
        ↓
AI 生成 Workflow JSON
        ↓
用户复制结果并进入导入流程
        ↓
服务端 workflow.dry_run 校验
        ↓
通过则 workflow.submit_definition 入库（draft）
```

---

### 7.7 Script App 与独立 Workflow 应用能力

如果用户希望基于 Workflow 开发出一个近似独立运行的软件（例如文件整理工具），仅有 Definition 不够，还需要一个驱动点。这个驱动点是 Script App：

- Script App 在系统中作为一类应用出现在左侧应用目录中。
- Script App 声明 Workflow 挂载点（与普通 App 一样）。
- Script App 定义触发逻辑（定时 / 文件变更 / 手动按钮 / 系统事件 / Webhook 等），由 Script App runtime 在合适时机调用 `workflow.create_run`。
- Workflow WebUI 负责查看和替换挂载在 Script App 上的 Definition；查看运行情况则跳转 TaskMgr UI。

#### 当前版本要求

一期不实现完整 Script App 创建器，但需要预留：

- Script App 在左侧组织树中作为应用目录的一种出现。
- Script App 的挂载点展示方式与普通应用一致。
- Agent 创建的 Script App 与其挂载的 Definition 应能在 WebUI 中被查看。
- 提供面向 Agent 的提示词入口，用于创建脚本驱动的 Workflow 应用（Script App + Definition + 挂载关系）。

#### 后续版本方向

- 支持用户手工创建 Script App，声明触发条件。
- 支持在 Script App 中可视化定义挂载点。
- 支持 Agent 直接在系统中完成 Script App + Definition + 绑定的端到端创建。

---

## 8. 功能需求列表

### WF-01 左侧 Workflow 组织树

**需求描述**：提供左侧组织树，管理 Definition 库与应用挂载点绑定。

**功能点**

- 展示 Definitions 目录（draft / active，archived 默认折叠）。
- 展示已安装应用目录，及其声明的 Workflow 挂载点。
- 展示挂载点当前绑定的 Definition（含 version），以及"已绑定 / 未绑定 / 必需未配置"状态。
- 支持搜索或过滤（按 owner / source / status / 关键字）。
- 支持手动刷新。

**验收标准**

- 用户可定位到任意 Definition / 挂载点。
- 未绑定挂载点不被隐藏；`required` 但未绑定的项有醒目提示。
- 选中后主区域加载详情。

---

### WF-02 Definition 管理

**需求描述**：Definition 库展示与基础管理。

**功能点**

- 展示系统内置 / 应用注册 / 用户导入 / Agent 生成的 Definition。
- 展示 Definition 的 `status` 与 `source` badge。
- 支持查看 Definition 的图结构与静态分析报告。
- 支持把 Definition 绑定到应用挂载点。
- 支持 `archive_definition`（软删除）；不支持物理删除（保留审计）。
- 系统内置 Definition 不允许 archive。
- 支持查看 Definition 的 version 链。

**验收标准**

- 导入成功的 Definition 默认出现在 Definitions 目录（draft）。
- Definition 被选中时主区域展示图结构 + 静态分析摘要。
- Definition 视图不展示 Run 列表（Run 与挂载点关联，不与 Definition 直接关联）。

---

### WF-03 应用挂载点展示

**需求描述**：展示应用 manifest 中声明的挂载点及绑定状态。

**功能点**

- 展示应用名称、应用 ID。
- 展示挂载点 id、说明、`required`、`allowEmpty`、`defaultDefinitionId`。
- 展示挂载点当前绑定的 Definition（含 version）。
- 支持替换绑定 / 恢复默认绑定 / 解绑（仅 `allowEmpty=true`）。

**验收标准**

- 用户能看出某应用的所有可用挂载点。
- 用户能看出某挂载点是否已绑定 Definition、绑定到哪个 version。
- 用户能把 Definition 绑定到指定挂载点。

---

### WF-04 Workflow Graph 图形展示

**需求描述**：用 React Flow 展示 Definition / 当前挂载绑定的 Workflow Graph。

**功能点**

- 展示 TaskNode 与 ControlNode（branch / parallel / for_each）；不同形状/颜色区分。
- 展示显式 edges 与隐式 edges（branch.paths、parallel.branches）。
- 节点徽标：`output_mode`（非 single）、`type`（human_confirm / human_required）、静态分析 severity。
- 支持缩放、拖动、fit view、定位选中节点。
- 默认只读，不允许拖拽改变结构。
- 大图（>200 节点）支持基本性能优化（懒渲染或虚拟化）。

**验收标准**

- 用户可以看到完整图结构，含控制节点与隐式边。
- 选中节点后展示节点配置。
- 只读模式不能修改图结构。

---

### WF-05 节点配置查看

**需求描述**：点击节点后展示该节点的详细配置。

**功能点**

- 按 TaskNode / ControlNode 分别使用 7.2 中定义的字段分组。
- 节点级静态分析告警（severity + message）。
- 支持复制 node id / 关键字段。
- 高级字段（schema、$defs 展开）可折叠。

**验收标准**

- 节点字段展示与引擎 DSL 定义一致。
- ControlNode 的字段不被混到 TaskNode 模板中。
- 只读模式下没有可误触的保存行为。

---

### WF-06 静态分析报告展示

**需求描述**：展示 Definition 的 AnalysisReport。

**功能点**

- 顶部条带聚合显示 Error / Warn / Info 计数。
- 展开后按 severity、node_id、message 排序展示。
- 点击 issue 自动定位到对应节点（如有 node_id）。

**验收标准**

- 有 Error 的 Definition 不允许执行"绑定到挂载点"操作。
- 用户能直观看到 issue 与节点的对应关系。

---

### WF-07 编辑模式预留

**需求描述**：当前以只读为主，但需要为后续节点参数编辑预留。

**功能点**

- 顶部预留"进入编辑模式"入口，一期隐藏或禁用。
- 节点配置面板结构支持从只读切换为表单。
- 编辑后保存路径：`workflow.dry_run` 校验 → `workflow.submit_definition` 生成新 version。
- 不在 WebUI 中编辑 Run；运行中干预走 TaskMgr UI（写 TaskData）。

**验收标准**

- 当前版本不会误导用户认为可编辑。
- 技术结构允许后续扩展。

---

### WF-08 Workflow Definition 导入

**需求描述**：通过文件、URL、文本导入 DSL/JSON。

**功能点**

- 支持本地文件 / URL / 粘贴文本。
- 服务端 `workflow.dry_run` 校验（编译 + 静态分析）。
- 通过后 `workflow.submit_definition` 入库为 draft。
- 失败展示 AnalysisReport issue 列表，附 severity / message / node_id / path。

**验收标准**

- 合法导入默认进入 Definitions 目录（draft）。
- 含 Error 的导入不会持久化，且展示明确错误提示。
- 导入成功后用户可继续执行"绑定到挂载点"。

---

### WF-09 Definition 绑定到挂载点

**需求描述**：把 Definition 绑定到 App 声明的挂载点。

**功能点**

- 从 Definition 发起"绑定到挂载点"。
- 选择应用 → 选择挂载点。
- 已有绑定时，明确展示新旧 Definition 名称 / version。
- 写入挂载点绑定表；左侧组织树更新。
- 不影响进行中的 Run（Run 锁定 version）。

**验收标准**

- 替换已有绑定前必须有确认提示。
- 替换后左侧树和详情页状态一致。
- Definition 含 Error 级别 issue 时禁止绑定。

---

### WF-10 最近 Run 列表（仅挂载点视图）

**需求描述**：展示挂载点最近的 Run 列表，仅作 TaskMgr UI 跳转索引。

**功能点**

- 数据来源 `workflow.list_runs`（按 definition + mount point 过滤）。
- 展示 Run 摘要字段（见 7.3）。
- 每条 Run 提供"在 TaskMgr 中打开"深链。
- 不展示 Step / Thunk 详情，不提供 retry / approve / cancel 按钮。

**验收标准**

- 挂载点页面能看到最近 N 条 Run。
- 点击跳转到 TaskMgr UI 对应根任务。
- Definition 视图（未绑定挂载点）不展示 Run 列表。

---

### WF-11 TaskMgr UI 跳转

**需求描述**：Workflow WebUI 不承担 Run 详情展示，只负责跳转。

**功能点**

- Run 列表条目携带 `root_task_id`。
- 跳转 URL 形如 `taskmgr/<root_task_id>`，由 TaskMgr UI 渲染整棵任务树。
- TaskMgr UI 根据 task type（`workflow/run` / `workflow/step` / `workflow/map_shard` / `workflow/thunk`）和 TaskData 渲染按钮、表单、subject 预览。

**验收标准**

- 用户从 Run 列表点击后能看到完整任务树。
- TaskMgr UI 中可执行 approve / modify / reject / retry / skip / abort / rollback 而无需回到 WebUI。

---

### WF-12 AI 生成提示词

**需求描述**：提供动态拼装的提示词，引导用户用 AI 生成 DSL/JSON。

**功能点**

- "新建 Workflow"展示提示词，而非空白图编辑器。
- 提示词由服务端动态拼装：当前 schema_version、可用 executor 列表、当前 zone 自定义 schema 片段。
- 支持复制 / 查看示例。
- 与导入弹窗联动，"AI 生成完成后粘贴回导入"。

**验收标准**

- 用户能复制完整提示词。
- 提示词内容覆盖 7.6 §提示词内容要求。
- AI 输出可被 `workflow.dry_run` 直接消费。

---

### WF-13 移动端只读查看

**需求描述**：移动端支持查看，不承担复杂编辑。

**功能点**

- Workflow Graph 全屏显示。
- 左侧树改为抽屉。
- 节点配置使用 Bottom Sheet。
- 默认不展示底部 Run 列表（可从菜单进入）。
- 操作按钮简化，编辑入口隐藏。

**验收标准**

- 移动端可打开并查看图。
- 移动端可点击节点查看配置。
- 移动端不出现复杂编辑入口。

---

### WF-14 Amendment 版本链查看（只读）

**需求描述**：当 Run 经历 Amendment 后，提供版本链只读视图。

**功能点**

- 在 Run 列表条目上展示当前 `plan_version`，比初始版本高时高亮。
- 点击展开 Amendment 历史（来自 service 的 Amendment 记录）：每个 Amendment 的 `submitted_by`、`submitted_at_step`、`operations`、`reason`、审批结果。
- 不在 WebUI 中提交 Amendment；提交动作由 Agent 通过 `workflow.submit_amendment` 完成。

**验收标准**

- 含 Amendment 的 Run 在列表上能识别。
- 用户能看到每次 Amendment 的修改内容与原因。

---

### WF-15 错误状态与空状态

**需求描述**：对各种异常情况提供清晰反馈。

**状态列表**

| 状态 | 处理方式 |
|---|---|
| 没有任何 Definition | 展示空状态、导入入口、AI 提示词入口 |
| 应用没有声明挂载点 | 展示"该应用暂无 Workflow 挂载点" |
| 挂载点 allowEmpty 为空 | 展示 Empty + "从 Definitions 添加"入口 |
| 挂载点 required 未配置 | 红色提示"必需挂载未配置" |
| Definition 编译 / 静态分析失败 | 展示 issue 列表 + 重新导入入口 |
| Definition 含 Error 级 issue | 禁止绑定 + 引导到 issue 详情 |
| 图结构加载失败 | 展示重试 |
| 最近 Run 列表为空 | 展示"暂无运行记录" |
| TaskMgr UI 跳转失败 | 展示 root task id + 复制入口 |
| 服务端不可达 | 展示降级提示，区分 workflow service 不可达 与 task_manager 不可达 |

---

## 9. 数据模型建议

### 9.1 WorkflowDefinition

```ts
type WorkflowDefinition = {
  id: string;                  // wf-<ulid>，与 service 主键一致
  schemaVersion: string;       // 引擎 DSL schema_version，如 "0.4"
  name: string;
  description?: string;
  version: number;             // 同 name 下递增的 version
  owner: { userId: string; appId?: string };
  source: 'system' | 'user_imported' | 'app_registered' | 'agent_generated';
  status: 'draft' | 'active' | 'archived';
  definitionRef: string;       // DSL JSON 引用（named object id 或 service-local id）
  analysis: AnalysisReport;
  createdAt: string;
  updatedAt: string;
  tags?: string[];
};

type AnalysisReport = {
  issues: AnalysisIssue[];
  errorCount: number;
  warnCount: number;
  infoCount: number;
};

type AnalysisIssue = {
  severity: 'error' | 'warn' | 'info';
  code: string;                // 引擎定义的 issue code
  message: string;
  nodeId?: string;
  path?: string;               // JSON Pointer，e.g. "steps[3].input.path"
  line?: number;
  column?: number;
};
```

### 9.2 AppWorkflowMountPoint

```ts
type AppWorkflowMountPoint = {
  id: string;                  // 在应用内唯一
  appId: string;
  name: string;
  description?: string;
  required: boolean;
  allowEmpty: boolean;
  defaultDefinitionId?: string;
  // 当前绑定（产品层元数据，存在 system_config 或专属表中）
  currentBinding?: {
    definitionId: string;
    definitionVersion: number;
    boundAt: string;
    boundBy: string;
  };
};
```

### 9.3 WorkflowGraphView（UI 渲染用）

```ts
type WorkflowGraphView = {
  definitionId: string;
  definitionVersion: number;
  schemaVersion: string;
  nodes: WorkflowGraphNode[];
  edges: WorkflowGraphEdge[];
  analysisIssuesByNode: Record<string, AnalysisIssue[]>;
};

type WorkflowGraphNode =
  | TaskNodeView
  | ControlNodeView;

type TaskNodeView = {
  kind: 'task';
  id: string;
  name: string;
  description?: string;
  stepType: 'autonomous' | 'human_confirm' | 'human_required';
  executor?: {
    raw: string;                          // 原始引用，如 "/skill/fs-scanner"
    resolvedNamespace?: 'service' | 'http' | 'appservice' | 'operator' | 'func';
    resolvedTarget?: string;              // registry 展开后的实际定义
  };
  inputSchema?: unknown;                  // JSON Schema
  outputSchema: unknown;
  outputMode: 'single' | 'finite_seekable' | 'finite_sequential';
  idempotent: boolean;
  skippable: boolean;
  subjectRef?: { nodeId: string; fieldPath: string[] };
  prompt?: string;
  guards?: NodeGuards;
  inputBindings: Array<
    | { kind: 'literal'; value: unknown }
    | { kind: 'reference'; nodeId: string; fieldPath: string[] }
  >;
  position?: { x: number; y: number };
};

type ControlNodeView =
  | { kind: 'control'; id: string; controlType: 'branch';
      on: { nodeId: string; fieldPath: string[] };
      paths: Record<string, string>;
      maxIterations?: number;
      position?: { x: number; y: number };
    }
  | { kind: 'control'; id: string; controlType: 'parallel';
      branches: string[];
      join: { strategy: 'all' | 'any' | 'n_of_m'; n?: number };
      position?: { x: number; y: number };
    }
  | { kind: 'control'; id: string; controlType: 'for_each';
      items: { nodeId: string; fieldPath: string[] };
      steps: string[];
      maxItems: number;
      concurrency: number;
      effectiveConcurrency: number;        // 受上游 output_mode 约束后的实际并发
      degradedReason?: string;
      position?: { x: number; y: number };
    };

type WorkflowGraphEdge = {
  id: string;
  source: string;
  target: string | null;
  // implicit=true 表示 branch.paths / parallel.branches 隐式边（UI 仍画出）
  implicit?: boolean;
  conditionLabel?: string;     // branch 路径上的枚举值
};

type NodeGuards = {
  budget?: { maxTokens?: number; maxCostUsdb?: number; maxDuration?: string };
  permissions?: string[];
  retry?: { maxAttempts: number; backoff: 'fixed' | 'exponential'; fallback: 'human' | 'abort' };
  timeout?: string;
};
```

### 9.4 WorkflowRunSummary（用于 Run 列表，详情走 TaskMgr）

```ts
type WorkflowRunSummary = {
  runId: string;
  rootTaskId: string;          // task_manager workflow/run 任务 id
  definitionId: string;
  definitionVersion: number;
  planVersion: number;         // 经 Amendment 后递增
  status:
    | 'created' | 'running' | 'waiting_human' | 'completed'
    | 'failed' | 'paused' | 'aborted' | 'budget_exhausted';
  triggerSource: 'app' | 'manual' | 'agent' | 'system';
  appId?: string;
  mountPointId?: string;
  humanWaitingNodes: string[]; // 非空时高亮 + 引导到 TaskMgr
  startedAt: string;
  finishedAt?: string;
  durationMs?: number;
  errorSummary?: string;
  taskmgrUrl: string;          // 跳转 TaskMgr UI 的深链
};
```

### 9.5 AmendmentSummary（只读视图）

```ts
type AmendmentSummary = {
  runId: string;
  planVersion: number;
  submittedBy: string;
  submittedAtStep: string;
  approvalStatus: 'pending' | 'approved' | 'rejected';
  reason?: string;
  operations: AmendmentOp[];
};

type AmendmentOp =
  | { op: 'insert_after'; afterNode: string; newSteps: unknown[]; newEdges: unknown[] }
  | { op: 'insert_before'; beforeNode: string; newSteps: unknown[]; newEdges: unknown[] }
  | { op: 'replace'; nodeId: string; replacement: unknown }
  | { op: 'remove'; nodeId: string };
```

---

## 10. 接口需求建议

> 命名沿用 buckyos-api 风格（`workflow.<method>`），与 [workflow service.md] §3 对齐。下面同时给出对应的 REST 路径建议，仅供前端参考。

### 10.1 获取组织树

```
RPC:  workflow_webui.get_tree
REST: GET /api/workflow/tree
```

返回：

- Definitions 列表（含 status / source / analysis 计数）。
- 已安装应用列表 + 各应用的挂载点列表 + 当前绑定。
- 状态摘要（每个挂载点的最近 Run 简况）。

> 实现上可由 Workflow Service + system_config 两个数据源拼装，避免 WebUI 同时调多个后端。

### 10.2 获取 Definition 图（含静态分析）

```
RPC:  workflow.get_definition (返回 definition + compiled + analysis)
      → 前端从 compiled / definition 派生 WorkflowGraphView
REST: GET /api/workflow/definitions/{id}
```

> 不复用 `workflow.get_run_graph`：那是给 UI 渲染**运行时**展开后的 Graph，本入口给 Definition 视图用。

### 10.3 获取 Run 时图（仅运行视图需要）

```
RPC:  workflow.get_run_graph
REST: GET /api/workflow/runs/{runId}/graph
```

> 一期 WebUI 不直接调用——Run 详情统一跳转 TaskMgr UI。保留接口供未来 deep link 还原 graph 高亮态。

### 10.4 获取 Executor / Schema 元数据

```
RPC:  workflow.list_executors  (registry 视图)
      workflow.get_executor_schema (按 executor id 返回 input/output schema)
REST: GET /api/workflow/executors
      GET /api/workflow/executors/{id}/schema
```

返回：

- registry 中可用的 executor 列表（namespace / id / 描述 / input_schema / output_schema）。
- 用于节点配置面板渲染、导入提示词拼装、未来表单生成。

### 10.5 编译校验（dry run）

```
RPC:  workflow.dry_run
REST: POST /api/workflow/dry-run
```

请求：

```json
{
  "definition": { /* DSL JSON */ },
  "sourceType": "file | url | text"
}
```

返回：

```json
{
  "success": true,
  "compiled": { /* CompiledWorkflow（前端可直接拿来渲染图） */ },
  "analysis": {
    "issues": [
      { "severity": "warn", "code": "for_each_concurrency_downgraded",
        "message": "concurrency forced to 1 due to finite_sequential upstream",
        "nodeId": "batch_ingest" }
    ],
    "errorCount": 0, "warnCount": 1, "infoCount": 0
  }
}
```

### 10.6 提交 Definition

```
RPC:  workflow.submit_definition
REST: POST /api/workflow/definitions
```

请求：

```json
{
  "definition": { /* DSL JSON */ },
  "name": "kb_import_pipeline",
  "tags": [],
  "initialStatus": "draft"
}
```

返回：`WorkflowDefinition`（含 id / version / analysis）。

> 服务端先 `compile + analyze`；含 Error 时拒绝写入。

### 10.7 列出 Definitions / 单个查询 / 归档

```
workflow.list_definitions       → GET  /api/workflow/definitions
workflow.get_definition         → GET  /api/workflow/definitions/{id}
workflow.archive_definition     → POST /api/workflow/definitions/{id}/archive
```

### 10.8 挂载点绑定

```
RPC:  workflow_webui.bind_mount_point
REST: POST /api/workflow/mount-points/{appId}/{mountPointId}/binding
```

请求：

```json
{
  "definitionId": "wf-...",
  "definitionVersion": 3,
  "replaceExisting": true
}
```

返回：更新后的 `AppWorkflowMountPoint`。

> 这是产品层接口，不是 Workflow Service 资源；建议由 system_config + 一层薄 service 实现。

### 10.9 恢复默认绑定 / 解绑

```
POST /api/workflow/mount-points/{appId}/{mountPointId}/restore-default
DELETE /api/workflow/mount-points/{appId}/{mountPointId}/binding   // 仅 allowEmpty
```

### 10.10 列出 Run（仅摘要）

```
workflow.list_runs?owner=...&definitionId=...&mountPointId=...&limit=20
REST: GET /api/workflow/runs
```

返回：`WorkflowRunSummary[]`。

> 详情请求一律重定向到 TaskMgr：`task_manager.get_task(rootTaskId)`。

### 10.11 Amendment 历史（只读）

```
GET /api/workflow/runs/{runId}/amendments
```

返回 `AmendmentSummary[]`。

### 10.12 AI 提示词

```
GET /api/workflow/ai-prompt
```

返回：基于当前 zone executor registry 与 schema_version 动态拼装的提示词文本。

### 10.13 不在一期实现的接口

- `PATCH /api/workflow/instances/{instanceId}/nodes/{nodeId}/properties`：旧版 PRD 中的"节点参数就地修改"。一期改为"通过 dry_run + submit_definition 生成新 version"，原接口不开放。
- `workflow.approve_step` / `workflow.pause_run` / `workflow.abort_run`：按 Service §3.2/§3.3 设计原则，**这些 RPC 不存在**；用户操作通过 TaskMgr UI 写 TaskData 完成。WebUI 不调用此类接口。

---

## 11. 权限与安全

### 11.1 权限

| 权限 | 能力 |
|---|---|
| workflow.read | 查看 Definition、挂载点绑定、Workflow Graph、节点配置、Run 摘要 |
| workflow.import | 调用 `workflow.dry_run` + `workflow.submit_definition` |
| workflow.bind | 修改挂载点绑定（Definition ↔ MountPoint） |
| workflow.archive | `archive_definition` |
| workflow.edit | 触发新 version 提交（一期可与 import 共用） |

> Workflow Service 自身 ACL（Definition / Run owner、stakeholders 写 TaskData 的 task_manager ACL）由 Service 层校验；WebUI 仅按上面这些粗粒度权限决定 UI 元素显隐。

### 11.2 URL 导入安全

URL 导入由 Workflow Service / 网关处理，已有限制（大小、超时、SSRF 防护、Content-Type 校验、`compile + analyze` 通过前不入库），WebUI 不重复实现。

### 11.3 Workflow 定义安全

- 不允许未知 executor（registry 命中率必须为 100%；不命中视为 Error）。
- Guards 中的 permissions 与应用 manifest 声明的权限交叉校验，越权拒绝绑定。
- 导入完成后**不自动激活、不自动绑定、不自动执行**：默认 draft，需用户显式激活并绑定。
- 触发 Run 由应用自身完成，WebUI 不提供"立即运行"按钮（一期）。

---

## 12. 响应式设计要求

### 桌面端

- 优先展示高信息密度。
- 左侧组织树常驻，主画布常驻。
- 节点配置面板可常驻或抽屉。
- 挂载点视图下底部 Run 列表常驻。
- 支持键盘快捷操作（可选）：`/` 聚焦搜索、`f` fit view、`Esc` 取消选中。

### 平板端

- 左侧组织树可收起。
- 节点配置面板以右侧抽屉为主。
- Run 列表可折叠。

### 手机端

- 只读查看优先。
- Workflow Graph 全屏。
- 组织树使用抽屉。
- 节点配置使用 Bottom Sheet。
- 不提供复杂编辑。
- Run 列表默认隐藏到菜单中。

---

## 13. 交互细节

### 13.1 默认模式

- Definition / 挂载点视图默认 Read-only。
- 编辑入口在一期隐藏；如展示，需明确标识"即将支持"。

### 13.2 图结构操作

- 缩放、拖动、fit view、定位选中节点。
- 不支持拖动节点改变保存后布局（除非未来启用编辑）。
- 隐式边（branch.paths / parallel.branches）画出并标注条件 / 分支名。

### 13.3 节点选择

- 点击节点高亮，配置面板自动更新。
- 一期不支持多选。
- 点击空白处取消选择。

### 13.4 Definition / 挂载点 / Run 区分

所有页面顶部明确指示当前查看的是：

- **Definition**：DSL 定义视图，不触发执行；只展示图结构 + 静态分析。
- **挂载点**：Definition + 应用绑定关系；展示 Run 列表（深链 TaskMgr）。
- **Run**：不在 WebUI 中展示详情，所有 Run 链接跳转 TaskMgr UI。

### 13.5 替换提醒

替换挂载点绑定时明确提示：

- 当前应用 / 挂载点 / 是否 required / 是否 allowEmpty。
- 原 Definition 名称 + version + source。
- 新 Definition 名称 + version + source。
- 替换后可能影响的应用行为。
- "进行中的 Run 不受影响（绑定锁定到创建时的 version）"。
- 是否可以恢复默认。

### 13.6 等待人类节点的视觉提示

挂载点视图的 Run 列表中，`humanWaitingNodes` 非空的 Run 用醒目色高亮，并提示"在 TaskMgr 中处理"。**不在 WebUI 内提供 approve / reject 按钮**。

---

## 14. 性能要求

- 常见 Definition 图（≤100 节点）能在 200ms 内完成首次渲染（不含网络）。
- 大型 Definition（>200 节点）支持懒渲染或虚拟化。
- Run 列表分页或限制条数（默认 20 条，最大 100）。
- 左侧组织树支持搜索，避免应用 / Definition 数量较多时难以定位。
- Executor schema、$defs 按需加载。

---

## 15. 可观测性与日志

WebUI 与对应后端服务应记录关键操作：

| 操作 | 日志字段 |
|---|---|
| 导入 Definition | userId、source（file/url/text）、analysis 摘要、errorCount/warnCount |
| 提交 Definition | definitionId、version、status |
| 绑定挂载点 | appId、mountPointId、oldDefinitionId+version、newDefinitionId+version |
| 解绑 / 恢复默认 | appId、mountPointId、操作类型 |
| 查看 Run | runId、rootTaskId（用于 trace 与 TaskMgr 对齐） |
| 编译失败 | 摘要错误码 + nodeId |

> Run 执行期日志、节点级 trace、Thunk 进度等仍由 Workflow Service / Scheduler / TaskMgr 各自承担，WebUI 不重复记录。

---

## 16. 版本规划

### V1：查看、导入、绑定（MVP）

- 左侧组织树（Definitions / 应用 / Script Apps / 挂载点）。
- React Flow 只读图（含 ControlNode、隐式边、output_mode 标注）。
- 节点配置面板（按 TaskNode / ControlNode 字段集分别呈现）。
- 静态分析报告聚合 + 节点级提示。
- 桌面端 Run 列表（深链 TaskMgr UI）。
- 文件 / URL / 文本导入；服务端 dry_run + submit_definition。
- Definition 绑定到挂载点（含替换 / 恢复默认 / 解绑）。
- AI 生成提示词（动态拼装）。
- Amendment 历史只读视图。
- 移动端只读查看。

### V1.5：节点参数轻量编辑

- 节点参数表单化编辑。
- 保存路径：dry_run 校验 → submit_definition 新 version → 可一键将挂载点绑定切到新 version。
- Run 列表中显示 plan_version。
- 更完善的编译错误定位（行列号 / JSON Pointer）。

### V2：图形化编辑、Amendment 协作、Script App 创建

- 图形化创建 / 修改节点连接（生成新 Definition）。
- Amendment 提议 UI（用户可在 Run 视图发起提议，由 Agent / Owner 审批）——**审批入口仍在 TaskMgr UI**，WebUI 仅承担提议构造。
- 可视化创建 Script App（声明触发条件 / 挂载点）。
- Agent 端到端：自动 submit_definition + 创建 Script App + 绑定。
- 长 Run 的可视化辅助（仅展示，操作仍走 TaskMgr）。

---

## 17. 验收标准总览

### 基础查看

- 用户打开 Workflow WebUI 后能看到 Definitions 与应用目录。
- 用户可以选中 Definition 或挂载点查看图结构。
- 用户可以点击节点查看配置；TaskNode 与 ControlNode 字段集正确分别呈现。
- 静态分析告警在顶部条带与节点上同时可见。
- 只读模式下无法误修改。

### 应用挂载

- 应用 manifest 声明的挂载点全部可见，包括 required / allowEmpty 标识。
- 用户可以把 Definition 绑定到挂载点。
- 替换前展示新旧 Definition 名称 + version；替换不影响进行中 Run。
- 用户可以恢复默认绑定 / 解绑（受 allowEmpty 约束）。

### 导入

- 文件 / URL / 文本三种方式都可导入。
- 服务端 dry_run 校验，含 Error 不入库。
- 合法导入默认进入 draft 状态；用户可激活后绑定。
- 错误提示包含 severity / message / 可选 nodeId / path。

### 运行视图边界（关键）

- WebUI **不提供** approve / retry / cancel / pause / resume 等 Run 操作按钮。
- WebUI **不展示** Step / Thunk / Map shard 详情。
- 最近 Run 列表每条都可一键跳转到 TaskMgr UI 对应根任务。
- Run 等待人类时仅高亮提示 + 引导用户去 TaskMgr UI。

### 移动端

- 手机端可查看图与节点配置。
- 手机端不出现复杂编辑入口。

### AI 生成

- 提示词动态拼装包含：当前 schema_version、可用 executor 列表（含 schema 摘要）、Reference 文法、ControlNode 字段、output_mode 兼容性、完整示例、输出格式要求。
- 用户复制后通过 AI 生成的结果能被 dry_run 直接消费。

### 与服务对齐

- WebUI 不调用任何 Workflow Service 不存在的 RPC（不调用 `approve_step` / `pause_run` / 节点属性 PATCH 等被废弃接口）。
- Run 摘要中的 `rootTaskId` 与 task_manager `workflow/run` 任务 id 一致。
- Definition 含 Error 级 issue 时无法绑定到任何挂载点。

---

## 18. 待确认问题

1. ~~Workflow DSL/JSON 的正式格式、节点类型列表和 Schema 来源是什么？~~ → 已对齐 [wokflow engine.md] §3，schema 由 service `workflow.list_executors` / `get_executor_schema` 返回。
2. 应用声明 Workflow 挂载点的注册协议是什么？建议在 app manifest 中扩展 `workflow_mount_points` 字段，安装时由 app installer 写入 system_config；待与 Apps 团队确认。
3. ~~系统默认 Definition 是否必须在 Definitions 目录中全部可见？~~ → 是（source=`system`，可被替换后保留恢复入口）。
4. Definition 的删除（archive 之外）、重命名是否进入 V1？建议 archive 进 V1，重命名（version + 1 写新 name）作为 V1.5。
5. ~~Run 是否允许用户手动触发，还是完全由应用触发？~~ → 一期 WebUI 不提供"立即运行"按钮；手动触发由应用自身或 TaskMgr UI 决定（应用可通过 `workflow.create_run` 触发；TaskMgr UI 可在父任务上"重跑"，由 task_manager 决定实现）。
6. Run 列表展示多少条合适？默认 20 条，最大 100。
7. TaskMgr UI 的详情页路由参数：`taskmgr/<root_task_id>`；待与 TaskMgr 团队对齐 query 参数（建议附带 `from=workflow_webui`）。
8. URL 导入是否支持认证链接？建议一期仅支持公开 URL；认证链接走"先下载到本地再上传"。
9. 节点参数中的表达式是否需要语法高亮？V1 不需要（一期不存在表达式，仅 Reference 字符串）；Reference 高亮可在 V1.5 引入。
10. ~~修改 Instance 是否自动生成新版本？~~ → 是（V1.5 起，所有节点参数修改都通过 `submit_definition` 生成新 version；不就地修改）。
11. **AppWorkflowMountPoint 绑定关系存放在哪？** 待定：建议存 system_config 下的 `apps/<appId>/workflow_bindings`，由专属薄 service 写入并供 WebUI 读写。
12. **Amendment 审批 UI 走 TaskMgr 还是 WebUI？** 一期通过 TaskMgr UI 写 TaskData 完成（与其它 HumanAction 一致）；WebUI 仅展示历史。

---

## 19. 一句话总结

BuckyOS Workflow WebUI 是 Workflow **Definition** 的高信息密度管理面板：它展示 Definition 与应用挂载点的关系，用 React Flow 呈现 DSL 拓扑（含 TaskNode + ControlNode + 隐式边 + output_mode + 静态分析告警），用节点配置面板还原引擎实际字段集，通过导入与 AI 提示词机制支持 Definition 引入；运行期的所有交互（待办、审批、回退、进度、通知）一律由 TaskMgr UI 承担，WebUI 仅做 Run 列表索引与深链跳转。
