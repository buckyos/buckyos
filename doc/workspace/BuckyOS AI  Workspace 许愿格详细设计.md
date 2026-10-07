# BuckyOS AI Workspace 许愿格详细设计

> 状态：设计稿 v0.1，2026-10-07。本文在第二期已实现的 Workspace、Mock 许愿格和版本机制上设计真实 xllm 执行器；不表示本文新增能力已经实现。
>
> 实现基线：提交 `701b9906`（AI workspace support canvas mode）及当前工作区源码。第二期范围与交付记录见[第二期规划](<BuckyOS AI Workspace 第二期规划.md>)和[第二期实施记录与验收报告](<BuckyOS AI Workspace 第二期实施记录与验收报告.md>)。本文仅完善设计，不修改现有实现。

## 1. 原始意图与本次范围

许愿格保存“运行一次 AIGC / AI 推理”所需的信息：用户的原始提示词、准确引用 Workspace 数据的 context 提示词、输入引用、执行器及输出规则。它是可以检查、重跑和追溯来源的数据实体，在画布上通过普通 Block 展示。

保留原始构想的四个步骤：

1. **分析需求**：从 Workspace 中找到用户所指的真实数据源，把自然语言需求翻译成完整的 context 提示词，检查输入是否就位。
2. **执行需求**：使用 context 提示词和本次固定的输入快照，运行 AI 推理，产生候选结果。
3. **安置结果**：将结果写入固定的命名结构（如 `$container/$blockname`），或每次新建一组。覆盖可查看历史并回滚，新建便于比较和选择。
4. **跟踪变化**：记录生成时实际依赖的数据及版本；上游改变后，许愿格和结果 Block 一起显示需要刷新。

本次只接入 **xllm，无 global memory**。分析与执行分别创建独立的 xllm Run；执行阶段只接收已确认的分析结果和显式输入，不隐式继承分析阶段的对话历史，也不接入 Agent 的长期记忆、Session、工作日志或自主调度。“两遍”指两个业务阶段，每个阶段内部可以有多轮推理和工具调用。

### 1.1 本文采用的实现选择

| 问题 | 首版选择 | 原因 |
| --- | --- | --- |
| xllm 运行位置 | aiworkspace 服务侧异步执行，复用现有 Rust SDK | 浏览器不承担模型凭据、进程和长任务生命周期 |
| Workspace 如何进入 context | 导出可读数据的本地快照目录，同时提供数据树和 BlockTree 的索引 | 先打通可检查、可复现的路径；避免模型自行拼装数据库和内部 RPC |
| 第一遍输出 | 结构化的分析结果：context 提示词、输入绑定、缺失项、输出约定 | 名称解析与数据完整性可以由宿主校验 |
| 第二遍输出 | 声明式结果列表，沿用并扩展现有 `ResultSpec` | 宿主机械生成数据实体、Block、依赖记录和一次 Commit |
| 是否直接修改 BlockTree | 首版不开放；作为以后独立的编辑执行模式 | 自由修改的写入范围、预览和回滚语义不同，不能与生成结果混用 |
| 是否自动重跑 | 只在用户显式分析、执行或重试时运行 | 多客户端收到变化事件不会重复消耗模型资源 |
| did-object 接入 | 预留同一读取适配层，首版不依赖它 | 当前 did-object 通用适配器并不等于已有 Workspace 对象协议 |

首版应交付文本、Record、表格，以及 xllm 能实际产出的 SVG / 文件资产结果。真实图像、音视频生成需要相应模型或工具能力，不能把接通文本推理等同于已接通这些服务。Mock 保留作确定性测试，`agent-work-session` 继续仅作协议预留。

## 2. 当前实现与接入前置工作

### 2.1 直接复用的基础

| 能力 | 当前代码与语义 |
| --- | --- |
| 两棵树 | `data` 下存数据；`surfaces` 下存 Surface / Group / Cell。产品中的画布 Block 在协议中仍是 `buckyos.cell` |
| 许愿格实体 | `buckyos.wish` 已有 `prompt`、`analysis`、`inputs`、`executor`、`output`、`output_mode`、`executor_config`、`last_run` |
| 两遍执行与应用 | Desktop 的 `WishService` 已有 `analyze → execute → plan → apply`；当前 `executorFor` 只接受 `mock`，候选保存在浏览器内存 |
| 结果与画布 | 现有 `ResultSpec` 支持 richtext / record / table / image / video；Mock 的 video 是逐帧预览。宿主创建结果文件夹、画布 Group 和 Cell |
| 依赖与新鲜度 | `entity.set_derived`、`entities.derived_json`、`doc.freshness`、`doc.relations`；core 与 WASM 共享判断 |
| 提交与撤销 | 普通 Command / Commit、读集前置条件、写入权限、锁、幂等记录、补偿撤销 |
| 版本 | 生成结果登记到 `entity_versions`；`doc.list_versions` 列表、`doc.restore_version` 返回恢复操作，由调用方提交 |
| 服务侧加工 | `proc.start/get/apply/cancel` 与 `local.sqlite.runs` 已存在，当前只运行同步的 `mock.task-summary@1` |
| xllm | `XllmTask::prepare`、`XllmRun::start/execute/resume`、`XllmInterrupter`、RunStore、结构化结果与用量记录已实现 |

主要依据：[WishService.ts](../../src/frame/desktop/src/app/aiworkspace/ui/wish/WishService.ts)、[类型校验](../../src/frame/aiworkspace/core/src/types.rs)、[新鲜度内核](../../src/frame/aiworkspace/core/src/freshness.rs)、[服务侧加工](../../src/frame/aiworkspace/store/src/proc.rs)、[版本读取与恢复](../../src/frame/aiworkspace/store/src/reads.rs)、[xllm SDK 说明](../llm_context/xllm_rust_sdk.md)及其[实现](../../src/frame/agent_tool/src/xllm.rs)。

### 2.2 真实执行器不能直接继承的 Mock 简化

以下是本次接入需要补齐的具体边界，不将它们视为已有的完整实现：

| 当前行为 | 本设计要求 |
| --- | --- |
| `snapshotInput` 接收 selector，但未据此裁剪读取；表格使用 `best_effort`、只取首个 1,000 行页面 | 按声明范围完整读取；在一致快照上分页，明确完整性，不静默截断 |
| UI 可以切换 `fixed`，但执行取快照仍按当前值读取并记录 `follow` | 实际读取指定历史对象，固定对象缺失时阻止执行 |
| 表格读集主要记录成员与字段值版本；文件夹展开没有完整的成员集合前置条件 | 补齐字段类型、筛选条件、容器成员等决定输入含义的版本 |
| 输入缺失时分析会过滤掉该引用；执行只检查剩余输入，且禁止零输入 | 缺失项作为 blocker 保留；合法的纯提示词任务可以有零输入 |
| 执行主要保护输入和 `last_run`，未冻结所有许愿格配置与输出决策 | 分析、执行、预览、应用均绑定配置基准；运行中改提示词或输出目标不会误用旧候选 |
| 候选、busy 状态在浏览器内存 | xllm 运行和候选在服务端持久化；重开页面可查询，取消能中断实际运行 |
| 表格覆盖重建记录 ID；表格历史恢复明确返回不支持 | 结果表按稳定逻辑键更新；补齐表格恢复后才开放完整的表格覆盖路径 |
| 新鲜度已有 fixed 分支，但不是固定历史对象可用性的完整校验；配置变化主要体现为 `needs_analysis` | 固定快照、配置摘要、结果自身人工修改分别判断，不能把旧配置生成的结果显示为最新 |

第二期已有的接口、结构与验收继续保留；这些补齐项和真实 xllm 接入一起验收。

## 3. 组件职责与一次运行

```mermaid
flowchart LR
    UI[许愿格 UI / WorkspaceStore] --> Host[aiworkspace 运行宿主]
    Host --> Snapshot[可读索引与输入快照]
    Snapshot --> Analyze[xllm Run A：分析]
    Analyze --> Analysis[校验并写回 analysis / inputs]
    Analysis --> Execute[xllm Run B：执行]
    Execute --> Candidate[候选结果与资产]
    Candidate --> Planner[结果规划与预览]
    Planner --> Commit[普通 Commit]
    Commit --> Data[数据实体 / Cell / derived / last_run]
    Data --> Freshness[core 新鲜度与版本历史]
    Freshness --> UI
```

- **UI / WishService**：编辑提示词、选择输入与输出方式、触发阶段、展示进度和预览、处理人工修改选择；把服务侧接受的 Commit 纳入同一保存状态和撤销栈。
- **运行宿主**：绑定当前调用者，创建快照，校验分析及结果，记录运行，调度 xllm，处理取消、恢复、资产与应用。模型输出始终只是候选。
- **ContextBuilder**：从正式的读取投影构造材料，保留实体与版本身份；不直接把 `doc.sqlite` 暴露给模型。
- **xllm 适配器**：组装两阶段 system / user context，选择已配置的模型与工具，返回业务 JSON、文件产物和执行记录。它不分配 Workspace 实体 ID，也不提交文档。
- **结果规划器**：复用现有 `WishService.plan` 的规则，将确定性部分收敛到服务端可用、可测试的实现；不维护第二套结果写入规则。
- **core / store**：继续负责合法性、权限、版本前置条件、整体提交、撤销与持久化。模型不能绕过这些规则。

xllm 的等待不能发生在 Workspace 的单写者互斥锁或 SQLite 写事务中。只在捕获基准、写入运行状态、准备或应用 Commit 时短暂持锁；期间用户仍可正常编辑。

## 4. 数据与状态模型

### 4.1 共享的许愿格

沿用 `buckyos.wish`，不再创建第二种“真实许愿格”实体。

| 字段 | 内容与变更语义 |
| --- | --- |
| `prompt` | 用户原始需求，保留原文 |
| `executor` | 本次新增可执行值 `xllm`；`mock` 继续用于测试 |
| `executor_config` | 模型选择器、工具配置档引用和预算。模型凭据、运行机路径不入文档 |
| `inputs` | 实体 ID、selector、label、`follow/fixed`；固定输入必须绑定实际快照对象 |
| `analysis` | 原始需求副本、context 提示词、警告；新增分析协议版本、状态、blockers、输出约定、宿主计算的有效性基准 |
| `output` | 已有 `container_id/name/type/surface_id`，分别表达数据落点和画布落点 |
| `output_mode` | `overwrite` 或 `new` |
| `last_run` | 最近一次成功应用的执行摘要、读集、结果身份、缺失结果；失败尝试不覆盖它 |

本设计新增的 `analysis` 子字段为 `schema_version`、`status`、`blockers`、`output_contract`、`basis`、`run_id`。`basis` 由宿主计算，至少覆盖原始提示词、执行器及有效配置、最终输入绑定、context 提示词和输出约定的摘要。不能用模型自报的摘要证明分析仍有效。当前 `analysis` 是严格键校验，实施时必须同步扩展 core、TS 类型与 fixtures。

改变需求、输入范围/版本策略、执行器或输出类型约定，标记“需要重新分析”。只改变结果安放位置、画布坐标或覆盖/新建方式，可以复用分析，但旧执行候选不得悄悄跟随新的输出设置。

`last_run` 新增 `config_digest` 和 `result_bindings`。后者保存本结果组的逻辑结果名到数据实体 / Cell 的对应，表格结果还保存逻辑记录键到 record ID 的映射。它们随结果应用一起写入，不能只存于运行数据库，否则导入后无法稳定刷新。映射也受现有 payload 大小限制；大表应使用可重建的稳定键映射，超限时明确拒绝，不无限扩张 last_run。

### 4.2 部署本地的运行记录

复用 `local.sqlite.runs` 管理本次工作，不把进度、日志和模型中间输出不断写入共享文档。以下是拟新增的逻辑字段，具体拆列或放入现有 JSON 字段由实现确定：

| 内容 | 必须保留的信息 |
| --- | --- |
| 身份 | Workspace、wish、调用者、阶段、业务 run_id、关联的 xllm run_id |
| 配置基准 | epoch、捕获时的 head_seq、许愿格相关 key revisions、analysis/config digest |
| 输入证据 | manifest、读集、固定对象引用、范围与完整性、输入资产摘要 |
| 输出 | 校验后的分析或 `ResultSpec[]`、候选摘要、上传资产映射、plan digest |
| 应用 | 预览时的输出版本、人工修改决策、Commit 请求及幂等键、最终 commit_id |
| 诊断 | 状态、阶段、时间、用量、错误、警告、取消标记和恢复依据 |

Run 的日志与输入快照按调用者可见性读取，不因另一个人能看见许愿格就自动公开整份推理日志。每次查询或下载产物仍检查当前权限。运行记录、临时目录和凭据不随 Workspace 导出或 Fork；已应用的结果和依赖记录随文档保存。

### 4.3 候选与依赖记录

候选沿用现有 `Candidate` 的核心含义，新增持久化、配置和预览基准：

```text
Candidate = 已校验的结果声明
          + 输入快照及读集
          + 许愿格配置基准
          + 输出身份 / 内容基准
          + 资产映射
          + 本次确认的应用计划摘要
```

每个结果仍通过 `entity.set_derived` 保存 `wish_id/run_id/executor/inputs/generated_rev`，新增 `config_digest` 和稳定的 `result_key`。`generated_rev` 由内核根据本次写入填写。`last_run.read_set` 与结果 `derived.inputs` 来自同一份宿主证据，不能由模型填写，也不能从“结果生成时间”反推。

`config_digest` 对共享的需求、context 提示词、输入绑定及版本策略、执行器配置和输出内容约定做规范化摘要；不包含 last_run、分析时间、实时读到的 follow 版本、画布位置及结果放置方式，避免应用本身或移动结果使内容立即过期。部署侧实际模型/工具配置另存运行证据，不让 core 依赖隐含的服务配置才能判断文档状态。

新字段涉及文档格式、物化、导出/导入、WASM 与 TS 的同时更新；按仓库当前开发规则统一更新格式与 fixtures，不新增旧版兼容分支。运行协议版本与文档格式版本分别管理。

## 5. 让 xllm 读懂 Workspace

### 5.1 两棵树都要提供，但身份绑定到数据

用户说“左边的销售表”“上一格的分镜”“角色组里的图片”，首先指向画布 Block。模型需要 BlockTree 来理解位置和名称，再通过 `Cell.source_ref` 找到数据树中的真实数据。

提供两种互相可定位的索引：

| 索引 | 必要信息 |
| --- | --- |
| 数据树 | entity_id、type_id、名称/标题、父节点、可读路径、字段定义或内容摘要、来源状态 |
| 画布树 | surface_id、Group/Cell 关系、Block 标题、source_ref、renderer，以及需要解释“左右/组内”时的 placement |

同一份数据在多个画布显示时，去重为一个数据输入；Block 的样式、位置通常不是生成依赖。只有需求明确要求读取布局时才记录相应结构依据。纯 UI Block 没有 `source_ref`，不能误当成可读取的正文。

名称和路径用于发现、展示；持久绑定使用 `entity_id`，表格使用稳定的 record/field ID，富文本范围使用内部 block ID。重名时返回候选并要求补充选择，不能任选一个；数据绑定完成后，重命名不应使引用失效。本文的 `$container/$blockname` 是逻辑路径表达，不是 shell 变量或可执行字符串。

### 5.2 首版：本地快照目录

每个阶段使用独立的工作目录，下面是拟采用的投影格式，不是当前已有的导出命令：

```text
<run-workdir>/
  .llm_context                 宿主生成的本阶段配置
  request.json                 原始需求、阶段、分析/输出约定
  context/
    manifest.json              Workspace/epoch/快照标识、实体映射、版本、完整性
    data-tree.json             可读的数据树索引
    block-tree.json            可读的画布关系与 source_ref
    entities/<entity_id>/
      meta.json                类型、显示名称、源身份、selector、版本
      content.md               富文本的可读投影（同时保留规范结构）
      content.json             Record、富文本 AST 或类型特定结构
      schema.json              表格字段、类型、选项及其 ID
      rows.jsonl               表格记录，保留 record_id 与按 field_id 编码的值
    assets/<object_id>         本次允许读取的实际资产
  output/                      xllm 产生的文件；尚未成为 Workspace 资产
```

文件名使用宿主生成的 ID，用户标题只进入元数据。所有路径解析在宿主侧完成；不得把模型输出的任意绝对路径当成可上传文件。此目录是临时材料，不是 Workspace 的另一个持久真相源；修改投影文件不会修改 Workspace。

分析阶段先提供可读索引、已声明输入的 schema 和有标记的摘要，需要更多正文时再由宿主读取。执行阶段重新捕获已绑定输入的完整快照，工具只能访问这份快照及本次产物目录；不会在运行中无声切回实时数据。

按需读取拟通过 `XllmDeps.with_host_tool` 注册阶段专用的只读工具，交给 ContextBuilder 在同一个分析快照中读取并补充投影。它不接受模型传入的调用者身份或实时数据库路径。工具名与参数在 W0 冻结；现有 `read_file` 只负责已经物化的文件，不能承担尚未实现的在线 Workspace 访问。

首版可用 xllm 的 `read_file` 读取投影；计算任务可按执行配置档启用本地 shell 处理这些材料。开启 shell 只代表有本地计算能力，xllm 的 `filesystem_policy: workspace` 并不是对 shell 的操作系统隔离。运行环境不注入 Workspace 写入凭据，也不提供提交工具；若部署开放通用 shell，其系统权限边界由部署负责，不能把“生成候选”宣称为任意 shell 都无副作用。

### 5.3 读取适配层与 did-object 路线

ContextBuilder 内部统一使用 `list/resolve/read/query/read_asset` 语义，复用 `doc.outline/resolve/read/query` 和资产读取规则。它负责权限、快照和读集，不让模型自己构造版本号。

以后可以把同一能力暴露为命令行或 did-object 的只读 property/action：请求必须携带本次运行绑定和快照标识，返回数据及版本证据；分页游标也绑定快照。在线读取和本地投影不能各自发明版本语义。

现有 `agent-did-object-lib` 有通用对象解析与 action 调用，但 Workspace 的 DID 寻址、object profile、快照参数和权限映射尚需实现。首版不编造已可用的 `did:workspace:...` 地址，也不以打通全套 did-object 为接入 xllm 的前置条件。

## 6. 输入快照与读集

### 6.1 一致性边界

宿主在短事务或固定的只读数据库快照中，一起取得许愿格配置、所需数据和对应版本，然后释放 Workspace 写锁，再进行文件投影及模型调用。不能先读值、过一段时间再读版本，并把两者拼成同一快照。

大表可以在固定快照上分页、流式写文件；不能在持有 Workspace 单写者锁时等待模型。快照必须有完整性证据：选择条件、字段范围、行数、是否完整、限制原因及文件摘要。分析阶段的抽样写明 `sampled`；第二阶段需要完整数据而未取得时，应阻止执行或让用户缩小范围。

对 URL 数据源，只有适配器确实支持一致快照和可验证版本时才纳入严格读集。当前只有 fixture 适配器的能力不能当成任意 HTTP 数据源能力；无法保证版本时给出明确的不可执行原因。

### 6.2 依赖如何落到现有版本格

| 实际读取范围 | 读集依据 |
| --- | --- |
| 整个富文本或 Record | 实体 `content_rev` |
| Record 的指定属性 | `doc_key`（如 `p:amount`），同时保护依赖的 schema |
| 富文本中的指定块 | `richtext_block` 的 hash；若含顺序/成员含义，另保护对应结构 |
| 指定表格单元格 | `table_cell` + 所用字段的 `table_field_type`，必要时加 `table_field` |
| 指定列的完整数据 | `table_members` + 所用列的 `table_field_values` / 类型版本 |
| 按条件筛选的表 | 成员版本、筛选/排序字段的值与类型版本，以及返回值实际读取的版本；保护会使未入选行进入结果的条件 |
| 文件夹输入 | 文件夹成员集合 + 展开的子项读集；不能只保护已经存在的子项 |
| 资产 | AssetRef 内容版本 + 已校验的 object_id / 字节摘要 |
| 显式读取画布布局 | Block 的布局/绑定配置及结构依据；普通数据任务不加入这类依赖 |

上述表格版本格已在 `core::plan::resolve_cell` 中存在；**文件夹成员与画布结构的通用 selector 尚不存在**，需要补充。建议新增 `tree_children`（当前直接子项身份集合的规范 hash）和 `tree_edge`（该节点的 `struct_rev`）；执行对嵌套文件夹展开时逐层记录。成员集合变化使依赖过期，普通 Block 移动不影响仅依赖数据内容的任务。

输入中的“选择什么数据”和 derived 中的“比较哪些版本格”分开处理。动态表查询需要新增并校验输入选择器（拟为 `table_query`，含 filter/sorts/fields）；ContextBuilder 将它展开成上表已有的成员、字段及单元格版本格，不能直接把查询对象传给当前 resolve_cell 并假定可比较。文件夹无法完整读取所声明范围时报告输入不完整，不能把有权限的子集伪装成全部。

读取器记录的是实际交给模型或计算工具的数据范围。首版可以将完整导出的选定输入作为保守读集，不必追踪 shell 究竟读取了哪些字节；但不能把整个 Workspace 都导出后声称只依赖模型主动列出的几个输入。

当前 wish / derived 输入各有 200 项限制。读集应先按相同实体和 selector 去重，必要时把大量单元格依赖提升为字段或整实体版本并说明粒度变粗；仍超限则要求缩小范围。不能截去读集来凑数。

### 6.3 跟随当前与固定版本

- **follow**：本次使用固定快照执行，应用时与当前版本比较；应用后继续随输入变化显示过期。
- **fixed**：用户选择一个可寻址的历史对象，绑定 `object_id`，从该对象及其文件/资产闭包读取。不能只把当前版本标成 fixed。
- 固定输入仍检查当前读取权限、历史对象与资产可用性；新版本出现不使固定内容过期。不得因实时实体的某个 selector 消失，就误把仍完整可读的固定对象判断为内容丢失。
- `doc.resolve` 已有 `fixed_revision + object_id` 路径；ContextBuilder 还需将历史对象解码为各类型的输入快照。无法物化的历史范围必须显式拒绝。
- 应用前置条件只对 follow 输入比较实时版本；fixed 输入校验固定对象可用性，不要求当前 head 与旧版本相等。

新鲜度的 fixed 分支也需核对固定对象，并按固定快照对应的依赖解释上游，不能直接遍历实时输入的最新依赖记录。固定引用所需历史对象纳入保留和包闭包；若包没有携带，则导入后标为不可用，不能改成跟随当前。

epoch 与对象 hash 是证据的一部分。运行期间发生恢复导入或 Workspace 换代时，旧候选失效；`head_seq` 用于定位快照，不作为所有任务的整文档冲突条件。否则移动一个无关 Block 也会使运行冲突。

## 7. 第一遍：分析需求

### 7.1 输入与提示词组成

分析阶段 system 包含：许愿格的职责、两棵树及身份规则、可用的读取方式、缺失/歧义处理、支持的结果类型，以及 `wish.analysis.v1` 输出规范。原始提示词、数据目录与内容材料放在 user/context 材料中，不混成宿主规则。

给模型的任务是：

1. 解释用户希望得到什么；已有的显式输入优先作为定位线索。
2. 从可见的目录中定位真实数据，用实体 ID 和 selector 绑定，不发明数据源。
3. 写出能够独立执行的 context 提示词，明确每个输入的角色、要读的范围、转换要求和结果类型/逻辑名称。
4. 报告缺失、歧义、不可读取和不支持的能力；区分阻止执行的 blocker 与可以接受的 warning。

对零输入的纯创作需求，返回 `inputs: []` 且状态为 ready 是合法的；依赖来自需求和执行配置。对“根据销售表”但找不到表的需求，不能退化为零输入任务。

### 7.2 结构化输出

以下为新增的模型输出协议示例；revision、object_id 的合法性与最终 basis 由宿主补齐和验证：

```json
{
  "schema_version": "wish.analysis.v1",
  "status": "ready",
  "context_prompt": "读取输入 tasks 的完整任务记录，按 status 与 due 判断未完成和逾期项，生成 summary 摘要与 statistics 统计。结论须注明依据；日期按 request 中的评估日期解释。",
  "inputs": [
    {
      "entity_id": "tasks",
      "label": "任务表",
      "selector": { "kind": "entity" },
      "version": { "mode": "follow" }
    }
  ],
  "output_contract": {
    "results": [
      { "name": "summary", "type": "richtext", "title": "任务摘要" },
      { "name": "statistics", "type": "record", "title": "任务统计" }
    ]
  },
  "blockers": [],
  "warnings": []
}
```

`status` 为 `ready | needs_input`；blocker 使用结构化的 `code/message/input_label/candidates`，候选只包含调用者有权读取的对象。字段名仍须经 schema 解析映射到真实 field ID。日期、币种、单位等影响计算的解释应进入明确参数或 context 提示词，不能每次运行从环境中隐式变化。

若用户明确选择 fixed，模型只能引用宿主目录中提供的版本候选；宿主解析为最终 `object_id`。上下文内出现的每个数据输入必须在 `inputs` 中有绑定。

### 7.3 宿主校验与写回

模型返回后按顺序校验 JSON/schema、引用范围、存在及可读性、selector 与字段类型、固定对象可用性、输出类型能力及生成依赖环。不能只验证输入 ID 存在。

直接或间接使用本许愿格已有结果作为输入会形成生成环，应拒绝；嵌套文件夹也要展开检查。生成依赖遍历有 visited 集合和预算，触及预算时显示无法确认，不能视为无环。普通数据引用成环不等同于生成依赖环。

分析完成后，将 `analysis` 与归一化 `inputs` 作为一次普通 Commit 写回；点击“分析”即触发该写回，无需再增加一次普通分析确认。写回保护分析开始时的 prompt、executor/config、原输入及 analysis 版本。用户中途改了需求，旧结果只保留为本地运行记录，不能覆盖新分析。

`needs_input` 可以写回供用户修正，但会阻止“执行”；缺失输入保留在 blockers 中，不能过滤后把分析改成 ready。执行前再次验证分析 basis 和输入可用性。

## 8. 第二遍：执行 context 提示词

### 8.1 独立的 LLM context

执行阶段创建新的 xllm Run，包含：

1. 宿主的结果协议、支持类型、命名规则和允许的产物目录。
2. 原始提示词和已确认的 context 提示词，后者作为本次具体执行任务。
3. 由第一遍确定、第二遍重新固定的数据快照及 manifest。
4. 输出约定和可供参考的原结果结构。旧结果正文只有在确有需要且显式纳入上下文时才能读取；不能通过它偷偷引入自依赖。

若分析依赖的 schema 或绑定条件已失效，先重新分析；只有数据值变化且原约定仍有效时可以用新值执行。日期等由宿主在 request 中明确固定。

context 提示词通常描述如何读取和处理数据，不复制分析时看到的统计值。若分析确实把某份正文中的规则或具体值写进提示词，宿主在 analysis.basis 中保留对应的语义读集；这些值变化必须重新分析，且生成结果的依赖包含这部分依据，不能拿旧提示词配新快照冒充一致输入。

执行阶段发现缺少已声明范围之外的数据，应返回需要补充输入，更新分析后再执行；不能自己读取实时 Workspace 并漏记依赖。

### 8.2 结果声明

沿用现有 `ExecuteResult.results` 与 `ResultSpec`，增加显式的 `wish.results.v1` 版本。模型返回逻辑结果，而不是 `entity.create` 等可直接执行的原始 Command：

```json
{
  "schema_version": "wish.results.v1",
  "results": [
    {
      "name": "summary",
      "type": "richtext",
      "title": "任务摘要",
      "content": { "markdown": "# 任务摘要\n\n本次统计与需要关注的事项……" }
    },
    {
      "name": "statistics",
      "type": "record",
      "title": "任务统计",
      "content": {
        "schema": {
          "properties": [
            { "key": "open", "name": "未完成", "type": "number" }
          ]
        },
        "props": { "open": 7 }
      }
    }
  ],
  "summary": "生成任务摘要和统计卡片",
  "warnings": [],
  "assumptions": []
}
```

示例中的数字只是协议样例，真实值必须来自本次输入。实际输出必须符合 analysis 中的输出约定；这里与前一阶段的 summary、statistics 对应。动态数量的输出须由约定显式允许，不能任意扩展任务。

结果类型的首版约束：

| 类型 | 结果内容与宿主处理 |
| --- | --- |
| richtext | Markdown 或规范 AST；宿主使用现有富文本 codec 校验和转为操作，明确支持的 Markdown 子集 |
| record | schema + props；必须满足现有字段类型与值校验 |
| table | fields + rows；新增 `record_keys`，与 rows 等长且唯一，宿主将逻辑记录键映射为稳定 record ID |
| image / asset | SVG 或 `output/` 下的文件产物引用；宿主检查真实字节、类型、大小后上传，构造 AssetRef。通用文件引用是新增能力 |
| video | 现有 Mock 逐帧预览仍标 simulated；没有真实媒体工具时不向模型承诺真实视频生成 |

`name` 是稳定的逻辑结果键，`title` 是显示名称。一次输出内 name 必须唯一、不可包含路径穿越或目录分隔符。renderer/config 可以作为受校验的展示建议；默认使用已有 Block 类型。首版不把任意 AI 生成 HTML/JS 当作结果代码直接运行。

### 8.3 校验与失败处理

xllm 的 `json` 选项可校验 JSON 语法，`TaskOverrides.json_schema` 会传给 Provider，但 SDK 本身不保证完成业务 schema 校验。宿主必须再次检查必需字段、类型、唯一名称、输出约定、数量/字节限制、文件路径及结果结构。

只有 xllm 正常完成、没有提取/JSON 错误且业务校验通过，才进入待应用。不能用退出码为 0、状态 completed 或“看起来像 JSON”代替上述检查。

无效输出可在同一输入快照上进行一次有预算的格式修复；修复仍失败则结束，保留原始输出供诊断。修复不得更换输入、扩大权限或直接应用部分结果。Provider 失败、超时、取消或预算耗尽都保留已有 Workspace 结果。

## 9. 命名、覆盖与结果身份

### 9.1 数据容器与画布 Group

结果的归属同时包含数据树中的 folder 和 BlockTree 中的 Group。数据实体写入 folder，Cell 通过 source_ref 引用数据并放入 Group；不能把数据实体作为 Cell 的 child。

首版沿用现有结果组结构，即使只有一个结果也使用一个结果 folder；UI 可以把它呈现为单个结果 Block。直接绑定到任意已有数据实体的“单实体覆盖目标”另行扩展，不暗中改变现有输出模型。

覆盖使用固定输出名称，新建使用现有的 `结果 #n` 规范，必要时在显示标题附加生成时间。原构想中的 `$生成时间.逻辑block名` 用于区别运行批次；持久身份由 run_id、逻辑结果键及保存的映射确定，不能单靠时间戳避免碰撞。

### 9.2 覆盖规则

- 同一逻辑 name、同一结果类型，保留数据 entity_id；已有 Cell、跨 Surface 引用和下游输入继续指向同一数据。
- 使用 `result_bindings` 查找已有结果；不能仅由标题或 `slugOf(name)` 推断身份。生成的新 ID 需检查碰撞，重试应用必须复用同一映射。
- 切换输出模式或目标时，先按目标容器、来源 wish 和 derived 的 group/result_key 确定结果组，不能套用上一组映射。遗漏结果的映射可由其持久 derived 重建；遇到同名但不属于本许愿格的内容，报告目标冲突，不直接接管。
- 本次新增的逻辑结果创建新实体和 Cell；本次缺少的旧结果进入 `missing`，不自动删除，也不重定向下游引用。
- 同一逻辑 name 的类型改变时，提示结构不兼容；用户可以选择整组新建或更换逻辑 name。不能把 TableSource 原地变成 RichText。
- 结果重新生成不重置用户已调整的 Cell 位置、大小与视图配置。新建的 Cell 才使用模型建议和宿主布局规则。
- 表格以稳定 field ID 与 `record_keys` 合并。更新已有记录时保留 record ID；删行、删字段、改类型和删除属性须进入结构差异预览并显式确认，不能套用 Mock 的删光重建策略。

### 9.3 新建规则

每次运行创建新的数据 folder、画布 Group、结果实体和 Cell；旧组保持原有内容、位置与依赖。新结果不得自动成为下游许愿格的新输入，用户手动调整输入后重新分析。

新建名称序号在应用时校验，不能让两个客户端分配同一个结果组。`last_run` 基准保护同一许愿格的并发应用：同一基准的两个候选只接受一个；另一方如需保留，重新预览并显式选择新建。

“采用这组结果并自动调整所有下游”不在首版范围。比较与选择由画布普通操作及输入绑定完成。

## 10. 预览与一次原子应用

### 10.1 预览必须是实际将提交的计划

宿主将结果编译为普通 Operations，返回新增/更新/保留/缺失项、结构变化、目标位置、需要处理的人工修改、资产与警告，并生成 `plan_digest`。预览阶段不写共享文档。

资产可以提前上传或暂存，但 `plan/get` 应可重复读取，不因每次打开预览重复上传。运行记录保存内容摘要到 object_id 的映射；未应用的资产按现有保留规则清理。资产上传成功不等于结果应用成功。

### 10.2 应用时的保护集合

| 保护对象 | 校验要求 |
| --- | --- |
| Workspace | 仍是同一 workspace_id 与 epoch |
| 输入 | follow 的读集未变；fixed 对象、资产仍可用；当前权限允许读取 |
| 许愿格 | prompt、analysis、inputs、executor/config、output/output_mode 与候选基准一致 |
| 并发运行 | `last_run` 的 key revision 未变 |
| 现有输出 | 用户预览/作出人工修改选择时的内容、schema 与必要结构仍一致 |
| 输出目标 | folder、Surface、结果 Group 仍存在、归属正确，满足当前创建/更新权限 |
| 应用意图 | plan digest 与人工选择对应；没有被服务器悄悄换成另一份计划 |

现有内容版本格继续用 Commit `preconditions` 和 operation 的 `expect`；新增结构条件在 core 中统一实现。输出基准不能等到点击应用时才“刷新为最新”，否则会无声覆盖运行期间的人工修改。

### 10.3 人工修改的三种处理

结果当前内容版本与 `derived.generated_rev` 不一致时，展示：

1. **保留人工修改**：内容不变，按第二期语义写入本次读集并标 `kept_manual: true`。UI 明确说明这是“用户确认沿用”，不是模型重新生成或验证过该内容。
2. **替换**：用户看过差异后覆盖；仍保护其确认时的输出版本。
3. **新建**：把候选另存，保留人工结果；采用明确的新结果身份和名称，不覆盖原引用。

用户作出选择后若又有人修改该结果，应用仍冲突，需要重新确认，不能重复利用过期的“替换”选择。

### 10.4 Commit 内容与幂等

一次成功的执行应用写入：结果数据变更、必要的新 Cell / Group、每个结果的 derived、许愿格 last_run 与 result_bindings。它们构成同一个 Commit 和撤销单位；任何权限、版本或类型校验失败，都不写入部分结果。

`proc.apply` 内部继续调用普通 `Workspace::commit`，使用调用者身份和 UI session_id，遵守锁与权限。客户端把 accepted commit_id 交给 WorkspaceStore 的保存状态与 UndoCoordinator，不能出现“服务端写成功但本地不可撤销”。

开始运行与应用分别使用稳定幂等键；同键不同参数或不同 plan digest 拒绝。应用请求超时后先通过提交记录查询，再返回已接受结果或重发原请求，不能再调用一次模型。`doc.sqlite` 提交与 `local.sqlite` 运行状态不是同一个事务：服务重启后用已保存的提交幂等键对账，已提交但未登记 succeeded 的运行补记成功。

取消与应用在同一 Workspace 写入协调边界裁决：取消先成立则禁止应用；Commit 先接受则返回“已经应用”，后续通过普通撤销恢复。

## 11. 版本、新鲜度与上下游

### 11.1 状态是多个维度

UI 同时展示执行状态、保存状态、新鲜度、人工修改标志；“正在执行”或“执行失败”不能遮住旧结果已经过期的事实。

| 新鲜度 | 含义 |
| --- | --- |
| `current` | 本结果的有效配置与所有跟随输入证据一致 |
| `stale` | 配置或直接依赖的数据发生变化 |
| `upstream_stale` | 直接输入值未变，但它本身的生成依据已失效 |
| `unavailable` | 已声明输入、范围或必需的固定对象/资产不存在 |
| `unknown` | 权限、离线材料或版本证据不足，无法确认 |
| `none` | 尚无已应用的生成结果或依赖记录 |

在 core 中扩展 `config_digest` 比较，使提示词/模型/输入约定变化同时体现在结果和许愿格上；`needs_analysis` 仍单独保留，说明下一步应该先分析。

许愿格主状态以 **最近成功应用的结果组** 为准。历史新建组各自保留新鲜度，不能因历史比较组已经过期而让刚生成的新组永远显示过期。当前实现会汇总 produced 引用；这里需按 last_run/result_bindings 区分当前组与历史组。回滚到旧运行的结果，则显示该结果实际记录的旧配置和依赖状态。

多阶段任务沿依赖链传播过期提示，但不自动执行。深度或数据预算不足时返回 unknown，不能把未检查完的链视为 current。依赖中的名称、详情和错误按调用者权限过滤。

### 11.2 历史恢复

复用 `doc.list_versions → doc.restore_version → 普通 Commit`。恢复产生新版本，保留实体身份，内容与当时的 derived 一起恢复，并按当前输入重新计算新鲜度；恢复旧结果不应把它标成这次模型的新输出。

当前恢复以实体为单位，表格恢复未实现。因此本次需要：

- 在开放表格覆盖前补齐其历史物化与恢复：field/record ID、类型、值及相关元数据保持语义一致，恢复仍做引用和权限检查。
- 多结果组回滚时，根据所选生成运行列出的结果版本汇总恢复计划，作为一个 Commit 预览与应用。后续新建但所选版本中没有的结果列出供处理，不自动删除。
- 组回滚所需的“结果实体 → 版本”清单须能从持久历史取得，不能只依赖不导出的 xllm Run。可在应用 Commit 元数据和版本索引中关联 run_id，不必建立第二套历史系统。

导出/Fork 当前只带当前内容，不携带全部 `entity_versions` 历史行；本次仍不承诺完整历史随包迁移。固定引用需要的对象闭包单独保留，导入后明确历史可用范围。导出时已过期的结果仍保留过期事实，不能因版本号重映射变成最新。

## 12. 服务接口与 xllm 生命周期

### 12.1 扩展现有 proc 接口

复用既有 `proc.*`，增加拟定 program `wish.xllm@1`，不新增与 Commit 平行的写入链。下表是本次拟扩展契约，不是目前可调用的完整参数：

| 方法 | xllm 许愿格参数与返回 |
| --- | --- |
| `proc.start` | `program=wish.xllm@1`，params 含 wish_id、stage=`analyze/execute`；稳定 idempotency_key。从服务端读取文档配置，返回 run_id 和当前状态 |
| `proc.get` | run_id；返回阶段、状态、警告、用量摘要，以及有权读取的分析/候选/预览与 plan_digest |
| `proc.apply` | run_id、session_id、plan_digest、人工处理 choices、稳定应用幂等键；analysis 阶段提交分析，execute 阶段提交结果 |
| `proc.cancel` | run_id；标记取消并中断关联的 xllm，报告是否已应用 |
| `proc.list`（新增） | 按 wish_id 查询当前调用者的最近运行，支持分页，供重开页面恢复入口 |
| `proc.resume`（新增） | 明确恢复可继续的中断/暂停运行；沿用快照与配置，不自动重新取数 |

`proc.start` 需改为创建记录后尽快返回，后台 worker 再执行模型；不能把当前同步 Mock 方法直接改成在锁内等待 xllm。服务重启会识别无人持有执行权的 running 记录，标记中断供恢复，不能重新开始一轮收费推理。

沿用现有 kRPC 业务结果封装：错误在 `result` 内，应用继续体现 accepted/conflict/rejected；模型错误与提交冲突分开。`proc.get` 是读取，不隐式启动模型或应用结果。分析自动写回由触发该分析的 UI 流程调用 apply；页面关闭后可在运行列表中继续处理。

同一主体重复发送相同 start 请求应返回同一运行；相同键却传入不同 wish/stage/config 基准时拒绝。当前 runs 的幂等查重需补参数摘要验证。

### 12.2 运行状态

```text
queued → snapshotting → running → validating → waiting_confirmation
                                               ↓
                                            applying → succeeded

执行中可到：interrupted / paused / failed / cancelled
应用时可到：conflict / rejected（候选仍可查看，旧结果不变）
```

这是宿主业务状态，不等于 xllm 的 RunStatus。xllm completed 表示推理阶段结束，业务上还可能在校验或待应用；只有 Commit accepted 才是已应用成功。分析阶段成功仅表示分析写回，不更新许愿格的 last_run 结果摘要。

恢复继续相同 xllm Run、工具集、输入快照和预算记录；重试创建新 Run。配置或输入已经变化时仍可查看/完成旧候选，但应用按前置条件冲突；UI 引导重新分析或执行。取消后才到达的模型返回不得重新进入待应用。

浏览器断开不取消服务端运行，重连通过 proc.list/get 恢复状态。用户主动取消才触发 `XllmInterrupter`；应用完成后取消不回滚，撤销使用 Workspace 的普通机制。

### 12.3 SDK 调用与配置边界

使用现有 Rust SDK：

```text
生成受控的阶段目录与 .llm_context
→ XllmTask::prepare(workdir, TaskInput, TaskOverrides, deps)
→ XllmRun::start(prepared, deps)
→ execute()
→ 读取 RunRecord / build_result_view
→ 宿主业务校验
```

`TaskInput` 分别传入原始需求/执行提示词和材料；`TaskOverrides` 设置模型、JSON 提取、预算及阶段特定 system；`RunObserver` 提供可展示的进度摘要。首版使用 native runtime；其他 runtime 不作为首版验收前提。

运行目录采用服务管理的位置，明确生成工具列表、provider/model、runs_dir、system 和预算；校验最终合并配置，避免 `prepare` 沿父目录发现的 `.llm_context` 注入无关提示词或工具。两个阶段各有独立上下文，恢复只读取该阶段 Run 的记录，绝不把其他许愿格 Run 当作记忆。

默认走已配置的 BuckyOS / AICC 模型选择器，例如逻辑名 `llm.chat`；精确模型选择沿用 SDK 能力。部署模型配置档映射到服务侧凭据引用，导入文档不能提供 API key、任意本地配置路径或切换运行身份。执行记录记录有效模型、配置摘要及实际用量，保证复查时能知道这次运行用了什么。

预算至少包括模型上下文/输出 token、工具迭代、总时长、输入字节/行数、结果数量和资产大小。必须在服务端受部署上限约束；当前 Commit 默认 8 MiB、10,000 operations，单资产默认 32 MiB，超过限制需提前报错或缩小任务，不能拆成不可整体撤销的多次隐式应用。

## 13. UI、离线与典型流程

### 13.1 UI 沿用现有许愿格面板

1. 输入原始需求，选择 xllm 配置、输出目标与覆盖/新建。
2. 点击“分析”，展示解析出的真实输入、context 提示词、缺失项和输出约定；引用可定位到数据树和相关画布。
3. 补全输入后点击“执行”，显示阶段、耗时、用量和取消入口。运行期间仍显示旧结果及其新鲜度。
4. 查看候选预览，处理人工修改与结构差异，点击“应用”。失败或冲突时保留候选供检查。
5. 结果在目标画布出现；可以整体撤销、查看版本、重新生成或用新建方式比较。

输入引用可以手动纠正；手动修改会让旧分析失效，随后重新分析。context 提示词首版可检查和复制；若开放直接编辑，也必须重新计算 basis 和执行前校验，不能留下“提示词变了但证据没变”的状态。

分析/执行可以在无画布的情况下产生数据结果；如 output 指定了 Surface，该 Surface 消失或无权写入时应用失败并提示修正，不能静默落到另一个画布。查看模式维持第二期策略，不能通过隐藏快捷键触发写回或应用。

### 13.2 离线语义

离线可以编辑许愿格配置、查看已经缓存的结果与依赖；xllm 分析、执行、恢复、资产上传及服务端候选应用需要联网。未同步的本地文档修改应先完成同步再启动运行，不能让服务端按旧提示词悄悄生成。

重连不自动执行；先同步文档并重新核对配置和新鲜度，再由用户决定下一步。离线新鲜度缺少必要对象或版本时显示 unknown。候选存在于服务端不等于已经写入文档，也不排入通用离线 Commit 队列。

### 13.3 最小完整案例

以“读取任务表和项目说明，生成任务摘要”为例：

- 分析把两个画布 Block 解析到 `tasks` 和 `project-notes`，保存两个数据输入及 summary 输出约定。
- 执行固定两个输入的快照，生成 summary 候选。用户移动任务表 Block 不影响应用；修改实际读取的任务状态则应用冲突。
- 应用创建 summary 数据与 Cell，并记录两份输入的版本。项目说明改变后，数据树、许愿格、summary 在各画布上的展现均提示过期。
- 若另一个许愿格用 summary 生成周报，summary 尚未刷新时周报显示上游过期；刷新顺序由用户显式控制。
- 人工修改 summary 后刷新，出现保留/替换/新建选择；取消保留原内容，覆盖后可通过版本历史恢复旧结果。

## 14. 实施落点与验收

### 14.1 分阶段落地

| 阶段 | 工作 | 退出条件 |
| --- | --- | --- |
| W0：补齐契约 | 两阶段 JSON schema、analysis basis、result_bindings、config digest、运行状态、RPC 扩展、输入和输出限制 | core/TS/Session/fixtures 对齐，新增与已有字段边界清楚 |
| W1：数据与结果基础 | 一致快照、selector/fixed、文件夹成员与布局版本格、依赖环、表格稳定身份及历史恢复、结果规划器 | 使用确定性执行器覆盖真实接入将用到的全部读写语义 |
| W2：xllm 分析与执行 | ContextBuilder、目录投影、SDK 适配、服务端异步运行、取消/恢复、候选持久化与资产 | 两个独立 Run 产生可检查结果，无 global memory，不阻塞 Workspace 编辑 |
| W3：完整 UI 与应用 | 运行查询恢复、预览/人工决策、原子应用、保存/撤销、结果组历史、新鲜度 | 正常 UI 跑通最小案例，重开页面、并发和失败路径成立 |
| W4：真实环境验收 | AICC 模型、权限与重启故障、规模边界、多阶段任务 | 报告区分 mock、真实模型、未配置能力和未验证项 |

代码职责继续放在已有模块内：

| 位置 | 修改方向 |
| --- | --- |
| `aiworkspace/core` | wish/derived schema、新增版本 selector、依赖和新鲜度、结果操作规划中的纯逻辑 |
| `aiworkspace/store` | 快照物化、运行/候选持久化、历史与资产闭包、幂等应用及恢复对账 |
| `aiworkspace/server` | 异步任务编排、xllm 适配与调用身份、proc 分派；不在 store/core 中引入模型网络调用 |
| `aiworkspace/wasm`、schemas、fixtures | 共享类型、校验、新鲜度、导入导出及离线读取同步更新 |
| Desktop `ui/wish`、`api`、`state` | 服务端运行适配、面板、候选预览、保存/撤销；Mock 与真实执行器共享结果语义 |
| `buckyos-api/src/aiworkspace_client.rs` | 核对服务配置与调用契约；现有薄客户端无需为每个 RPC 另造一套类型 |
| SDK / CLI 消费方 | 若扩展公开命令，同步更新请求参数、状态和错误处理；did-object 后续也消费同一契约 |

### 14.2 必须验证的场景

| 编号 | 场景 | 通过标准 |
| --- | --- | --- |
| W01 | 分析用户提示词并定位重名/跨画布数据 | 绑定真实数据 ID；重名要求补充选择；同数据多 Block 不重复取数 |
| W02 | 缺失输入与纯提示词任务 | 缺失项不被静默过滤；合法零输入可以执行 |
| W03 | 大于 1,000 行的表与查询范围 | 不截断；完整性、selector、筛选/类型/成员读集正确 |
| W04 | 固定旧版本后修改或删除实时范围 | 使用指定历史内容；可用性与权限按 fixed 规则判断，不回退到 live |
| W05 | 分析/执行期间修改提示词、输入或输出配置 | 旧分析不覆盖新需求；旧候选不能无声应用到新目标 |
| W06 | 输入变化与无关布局变化 | 相关变化冲突/过期；无关布局、视口及无关字段不误报 |
| W07 | 文件夹增删成员、多级生成环 | 新增成员会使依赖失效；间接环被拒绝，遍历有界 |
| W08 | 非法 JSON、错误字段、重复结果名、路径越界、超限产物 | 宿主拒绝，无部分文档写入；格式修复有明确预算 |
| W09 | 覆盖、新建、结果数量/类型变化 | 身份稳定；missing 不删除；类型冲突明确；新组不改下游引用 |
| W10 | 表格刷新与回滚 | 保留 field/record ID 和相关引用；结构差异有预览；历史恢复可撤销 |
| W11 | 人工修改后保留/替换/新建，确认后再被他人修改 | 三种选择语义准确；二次变化仍冲突 |
| W12 | 两客户端应用、重发 start/apply | 同基准只接受一个；同键同结果，不重复调用模型或创建结果 |
| W13 | 应用接受后进程退出、响应丢失 | 通过 Commit 幂等记录对账；重启可恢复 succeeded，不重复写入 |
| W14 | 取消与模型返回/应用竞态 | 取消后的返回不能复活候选；已经应用如实返回并提供撤销 |
| W15 | 运行时关闭页面、服务重启及显式 resume | 可重新查询；只恢复原快照/配置；不自动重跑 |
| W16 | 权限撤回、受限用户读取运行及关系 | 输入/日志/资产/引用不越权；应用按当前调用者权限与锁校验 |
| W17 | 最新结果与历史比较组、固定上游、多阶段依赖 | 每组状态准确；旧比较组不污染最新组；递归证据不足为 unknown |
| W18 | 导出、导入、Fork、缺少历史快照 | 当前结果与依赖恢复；过期不洗白；运行/凭据不入包；缺失固定对象可解释 |
| W19 | 离线修改后重连启动 | 先同步后执行；未联网不伪装为已运行/已应用 |
| W20 | 真实 xllm 两阶段运行 | 保存有效模型、格式化输出、用量和取消证据；不依赖 Mock 固定返回或浏览器内存 |

实施时先用可控 LLM 返回与确定性 fixtures 验证语义，再用已配置的真实模型验证集成；不以真实模型逐字相同作为测试断言。持久化、幂等与权限断言检查 RPC/数据库结果，不能只看 DOM。

建议验证入口（以下是实施时的检查计划，本次文档编辑未运行）：

```bash
# buckyos/src
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace
cargo test -p agent_tool --lib xllm -- --test-threads=1

# buckyos/src/frame/desktop
pnpm check
pnpm build
pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/wish.spec.ts
```

变更 core/WASM 后先通过已有 `wasm/build.sh` 重新生成绑定，再验证离线与浏览器；涉及资产、权限、包和布局的既有回归按影响范围执行。真实 Zone / AICC 的可用模型、配置和运行身份在验收记录中列出，不用独立测试后台代替真实环境结论。

## 15. 首版之外的扩展

- **直接编辑模式**：若允许模型返回任意 BlockTree/数据树编辑计划，应独立声明可读/可写范围、操作白名单、完整读写集、差异预览和副作用语义。仍经普通 Commit，不让模型边推理边修改实时 Workspace。
- **did-object / CLI 读取**：把 ContextBuilder 的快照访问协议映射为正式对象能力；不会因此自动获得提交权限。
- **Agent work-session 与全局记忆**：需要新的执行器和上下文来源契约，不隐式加入 xllm。
- **自动刷新与调度**：需要明确计算归属、去重、预算及用户授权；本次只显示依赖变化。
- **真实媒体工具、AI 生成扩展代码、自动采用对比组**：分别补充模型/工具、扩展宿主或引用调整规则，不以 Mock 演示代替实现。

## 修订记录

| 日期 | 内容 |
| --- | --- |
| 2026-10-07 | v0.1：在手写构想上补全真实 xllm 两阶段执行、上下文投影、输入版本、结果协议、命名与应用、运行恢复、实施边界及验收；对照第二期源码列出需补齐的 Mock 简化 |
