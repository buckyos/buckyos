# Agent-Human-Loop Workflow 引擎需求文档

> **版本**: 0.2.0-draft
> **状态**: RFC / 社区讨论稿
> **变更**: 基于社区评审反馈，修复了 Flow 语义未闭合、human_confirm 审查对象不明确、Reference 语法不完整、skip/rollback 与副作用一致性等关键问题。引入 Node Graph 统一建模、Executor 协议、事件 Envelope 等规范。
> **上下文**: 本文档基于 OpenDAN × BuckyOS × cyfs:// 北极星方向文档，定义 Agent-Human-Loop Workflow 引擎的需求与 DSL 规范。

---

## 1. 引擎要解决的问题

Agent 引入了根本性的不确定性——LLM 的输出不可完全预测，工具调用可能失败，很多决策需要人类判断。传统工作流引擎（n8n / Airflow / Temporal）假设流程在设计时已知且确定，无法处理"Agent 执行到一半要换策略"或"人类临时改主意回退三步"的场景。

同时，纯对话模式的 Agent 缺乏结构化的可观测性——人类无法一眼看到全局进度、无法在明确的锚点上审查和干预、无法事后审计执行轨迹。

本引擎要解决的核心问题是：

**让 Agent 动态生成结构化的执行计划，由引擎驱动执行，人类通过结构化视图观察、审查与干预——同时保证计划在运行前可被静态分析和理解。**

### 1.1 不解决的问题（明确排除）

- **不是通用 BPM / BPMN 引擎**：不追求企业级流程建模的完备性。
- **不管模型选择**：用什么 LLM、怎么 fallback 是 Agent/Skill 内部的事。
- **不管 UI 渲染**：引擎输出结构化状态数据，UI 层自行渲染（Web 看板、飞书卡片、移动端通知均可）。
- **不做跨 Owner 权限**：第一版只处理单 Owner 下的 Agent 协作。

### 1.2 第一个用户：KB 构建

引擎的第一个使用场景不是外部企业，而是 OpenDAN 自身的 Knowledge Base 子系统。Mia（知识库管理员 Agent）根据用户私有数据的规模、类型和质量，动态生成数据治理 Workflow，通过本引擎驱动执行。用户通过结构化视图观察 10TB 级数据如何被分批、清洗、标注、Embedding 并编入 KB。

这个场景天然覆盖了引擎的核心需求：长时间运行、批量进度追踪、人类异步抽检、质量不达标时局部回退与重做、资源消耗可见。先用这个场景把引擎做出来，再推广到外部业务场景。

---

## 2. 核心设计决策

### 2.1 DSL 管结构，Agent 管智能

这是整个设计最关键的 trade-off。

因为停机问题，图灵完备的脚本无法被静态分析——你无法在不执行它的情况下确定它会走哪些步骤、调用哪些工具、在哪里停下来等人。而"人类通过结构化视图审查计划"的前提恰恰是：**在运行之前，就能把执行结构展开成一个确定的有限步骤图。**

因此引擎采用受限 DSL 而非脚本语言来描述执行计划：

- **DSL（外层）**：描述步骤的编排与流转。确定性的、可静态分析的、可渲染为可视化视图的。
- **Agent/Skill（内层）**：在每个步骤内部执行。不确定的、可任意复杂的、但对引擎来说是黑盒。

引擎不需要图灵完备的编排语言，因为图灵完备的部分由 LLM 提供——Agent 在步骤内部可以调 LLM、做多轮推理、试错、换策略，但引擎只关心这个步骤最终输出了什么、花了多少资源、成功还是失败。

### 2.2 配置可被 Agent 生成，也可被人类阅读

DSL 有三个读者：Agent（生成者）、引擎（执行者）、人类（审查者）。语义模型以 JSON Schema 为规范，呈现可以有多个视图：Agent 生成 JSON，引擎校验 schema，人类看到渲染后的看板或流程图。如需人类手写配置，可提供 YAML 前端做转换，但规范以 JSON Schema 为准。

### 2.3 Pipeline 是 Workflow 的退化特例

传统 KB 领域的 Pipeline（线性、确定性、无人类介入）是本引擎的一个特例：当所有步骤都是 `autonomous` 类型、无条件分支、无 plan amendment 时，运行时行为与 Pipeline 完全相同。不需要两套机制。

---

## 3. DSL 语义模型

### 3.1 Workflow Graph：统一的节点-边模型

Workflow 的执行结构是一个**有向图（Workflow Graph）**，图中有两类节点：

- **TaskNode**：对应一个 Step，是实际执行工作的节点。拥有 executor、input、output。
- **ControlNode**：对应一个流转控制结构（branch / parallel / for_each），不执行业务逻辑，只负责路由、展开或汇合。

两类节点共享同一个 ID 命名空间。在 DSL 配置中，TaskNode 定义在 `steps` 数组中，ControlNode 定义在 `nodes` 数组中。`edges` 定义节点之间的有向连接。

**所有节点（无论 Task 还是 Control）都可以作为 edge 的 source 或 target。** 这是本 DSL 与上一版的关键区别——不再有"flow block 既是描述又是节点"的歧义。

```json
{
  "schema_version": "0.2.0",
  "id": "kb-ingest-media-library",
  "name": "媒体素材库 KB 导入",
  "description": "扫描 NAS 素材库，分类处理并编入知识库",

  "trigger": { ... },
  "steps": [ ... ],
  "nodes": [ ... ],
  "edges": [ ... ],
  "guards": { ... }
}
```

**顶层字段说明**：

| 字段 | 必需 | 说明 |
|------|------|------|
| `schema_version` | 是 | DSL 版本号，用于兼容性校验 |
| `id` | 是 | 全局唯一标识 |
| `name` | 是 | 人类可读名称 |
| `description` | 否 | 人类可读描述 |
| `trigger` | 是 | 触发方式 |
| `steps` | 是 | TaskNode 列表（所有步骤定义） |
| `nodes` | 否 | ControlNode 列表（branch / parallel / for_each）。纯顺序 Workflow 可以没有 ControlNode |
| `edges` | 是 | 有向边列表，定义节点间的流转关系 |
| `guards` | 否 | 全局约束（预算、权限、超时） |
| `defs` | 否 | 可复用的 schema 片段（类似 JSON Schema `$defs`），用于避免重复定义 |

**Workflow Graph 的终止条件**：当一个 Run 中不存在任何处于 `running` 或 `waiting_human` 状态的节点、且不存在任何状态为 `ready` 的节点时，Run 进入终止状态（`completed` 或 `failed`，取决于是否有 `failed` 节点）。

### 3.2 Step / TaskNode（执行节点——唯一的业务执行单元）

每个 Step 是引擎调度的原子单位。引擎不关心 Step 内部发生了什么，只关心输入、输出、状态。

```json
{
  "id": "scan_source",
  "name": "扫描数据源",
  "executor": "skill/fs-scanner",
  "type": "autonomous",
  "input": {
    "path": "/mnt/nas/media"
  },
  "input_schema": {
    "type": "object",
    "properties": {
      "path": { "type": "string" }
    },
    "required": ["path"]
  },
  "output_schema": {
    "type": "object",
    "properties": {
      "total_files": { "type": "integer" },
      "by_type": {
        "type": "object",
        "additionalProperties": { "type": "integer" }
      },
      "total_size_bytes": { "type": "integer" }
    },
    "required": ["total_files", "by_type", "total_size_bytes"]
  },
  "idempotent": true,
  "skippable": false,
  "guards": {
    "timeout": "30m",
    "max_cost_usdb": 0.01
  }
}
```

**字段说明**：

| 字段 | 必需 | 说明 |
|------|------|------|
| `id` | 是 | 节点唯一标识（与 ControlNode 共享命名空间，全局唯一） |
| `name` | 是 | 人类可读名称 |
| `executor` | 条件 | 执行者，格式为 `agent/<name>` / `skill/<name>` / `tool/<name>`。`type` 为 `human_required` 时不需要 |
| `type` | 是 | 执行类型（见 3.3） |
| `input` | 否 | 输入数据，可包含 Reference（见 3.6） |
| `input_schema` | 否 | 输入的 JSON Schema 约束。当提供时，引擎在静态分析阶段校验上游 output_schema 与本 step input_schema 的类型兼容性 |
| `output_schema` | 是 | 输出的 JSON Schema 约束。引擎用于运行时校验，人类用于理解产出 |
| `subject_ref` | 条件 | 仅 `human_confirm` 类型使用。指向本次审查的对象（见 3.3.1） |
| `prompt` | 否 | 当 `type` 为 `human_confirm` 或 `human_required` 时，展示给人类的提示信息 |
| `idempotent` | 否 | 默认 `true`。声明此步骤是否幂等。影响恢复和回退策略（见 4.4, 4.6） |
| `skippable` | 否 | 默认 `true`。声明人类是否被允许跳过此步骤。不可跳过的步骤人类无法执行 skip 操作 |
| `guards` | 否 | 步骤级约束，覆盖全局约束 |

### 3.3 Step Type（步骤类型——仅三种，仅关于决策权归属）

Step type 只描述"谁有权决定这一步的结果"，不涉及流转控制（流转控制由 ControlNode 负责）。

| 类型 | 决策权 | 引擎行为 |
|------|--------|----------|
| `autonomous` | Agent 决定 | 调用 executor，完成后自动流转。人类可随时中断 |
| `human_confirm` | 人类守门 | 向人类展示审查对象（subject），等待人类 approve / reject / modify |
| `human_required` | 人类执行 | 引擎发出通知，暂停等待人类完成输入 |

#### 3.3.1 `human_confirm` 的审查对象（subject）语义

`human_confirm` 步骤必须通过 `subject_ref` 明确指定人类审查的对象。`subject_ref` 是一个 Reference（见 3.6），指向某个前置步骤的输出。

人类操作的语义：

- **approve**：`subject_ref` 指向的对象原样通过，写入本步骤的 `output.final_subject`
- **modify**：人类修改后的版本写入 `output.final_subject`
- **reject**：`output.final_subject` 为空，`output.decision` 为 `"rejected"`

本步骤的 `output_schema` 必须包含 `final_subject` 字段（类型与 `subject_ref` 指向的 schema 一致）和 `decision` 字段。后续步骤应始终引用 `${review_step.output.final_subject}` 而非直接引用被审查步骤的输出，以确保数据流中使用的永远是"人类确认后的版本"。

```json
{
  "id": "review_plan",
  "name": "审核处理计划",
  "type": "human_confirm",
  "subject_ref": "${plan.output}",
  "prompt": "请审核 Mia 生成的素材库处理计划。",
  "output_schema": {
    "type": "object",
    "properties": {
      "decision": {
        "type": "string",
        "enum": ["approved", "rejected", "need_revision"]
      },
      "final_subject": { "$ref": "#/defs/PlanOutput" },
      "feedback": { "type": "string" }
    },
    "required": ["decision", "final_subject"]
  }
}
```

### 3.4 ControlNode（控制节点——三种，仅关于流转路由）

ControlNode 不执行业务逻辑，只负责路由、展开或汇合。每个 ControlNode 有唯一 `id`，与 TaskNode 共享命名空间，可以作为 edge 的 source 或 target。

#### 3.4.1 branch（分支——必须枚举）

```json
{
  "id": "plan_review_branch",
  "type": "branch",
  "on": "${review_plan.output.decision}",
  "paths": {
    "approved": "parallel_process",
    "rejected": "revise_plan",
    "need_revision": "revise_plan"
  },
  "max_iterations": 3
}
```

**关键约束**：

- **分支必须穷举**：`paths` 中必须覆盖 `on` 引用的 output_schema 中该字段的所有 `enum` 值。引擎在加载配置时校验。
- **不允许 fallthrough**：没有 `default` 或 `else`。如果运行时出现未覆盖的值，引擎报错并暂停等待人类处理。
- **`max_iterations`**：当分支形成回路时（如 rejected → revise → review → 再次 branch），此字段限制最大回路次数。达到上限后引擎暂停，交人类决定。

branch 节点的输入来自 edge（上游节点的完成触发它），它本身不持有输出——它只读取 `on` 引用的值并路由到对应的 target 节点。

#### 3.4.2 parallel（并行）

```json
{
  "id": "parallel_process",
  "type": "parallel",
  "branches": ["process_videos", "process_images", "process_docs"],
  "join": "all"
}
```

| `join` 值 | 语义 |
|-----------|------|
| `all` | 所有分支完成后继续 |
| `any` | 任一分支完成后继续，其余取消 |
| `n_of_m` | N 个分支完成后继续（需额外指定 `n`） |

每个并行分支独立执行、独立失败。

**输出合并规则**：parallel 节点汇合后产生的输出是一个 object，**以分支 TaskNode 的 id 为 key**：

```json
{
  "process_videos": { "processed": 5000, "failed": 3, "quality_score": 0.92 },
  "process_images": { "processed": 12000, "failed": 15, "quality_score": 0.87 },
  "process_docs": { "processed": 800, "failed": 0, "quality_score": 0.95 }
}
```

后续步骤引用：`${parallel_process.output.process_videos.processed}`。严禁扁平合并（字段名冲突会导致不可审计的数据覆盖）。

#### 3.4.3 for_each（有界迭代）

```json
{
  "id": "batch_ingest",
  "type": "for_each",
  "items": "${plan.output.batches}",
  "steps": ["ingest_batch", "validate_batch"],
  "max_items": 1000,
  "concurrency": 5
}
```

**关键约束**：

- **`items` 必须引用前置步骤的输出**：在该步骤完成前，for_each 无法展开；完成后，items 数量确定，引擎可以展开并渲染完整视图。
- **`max_items`**：硬上限。如果 items 数量超过此值，引擎报错暂停。防止 Agent 生成的计划意外产生海量迭代。
- **`concurrency`**：最大并行度。默认为 1（串行）。

**实例化 ID 规则**：for_each 内部步骤在运行时实例化为 `{step_id}[{index}]`，例如 `ingest_batch[0]`、`ingest_batch[1]`。index 从 0 开始，对应 items 数组的下标。

**输出形状**：for_each 节点的整体输出是一个数组，按 items 顺序排列，每个元素是该轮迭代中最后一个步骤的输出：

```json
[
  { "status": "ok", "records_ingested": 1200 },
  { "status": "ok", "records_ingested": 980 },
  ...
]
```

**引用约束**：DSL 的 Reference 不支持按索引引用单个迭代的输出（因为索引在运行前未知）。如果后续步骤需要聚合 for_each 的结果（如统计总数、筛选失败项），**必须在 for_each 之后接一个聚合 Step**，由 Skill 完成聚合逻辑。后续步骤引用聚合 Step 的输出，而非直接引用 for_each。

### 3.5 Edges（有向边——节点间的连接）

边是 Workflow Graph 中节点之间的显式连接。

```json
{
  "edges": [
    { "from": "scan", "to": "plan" },
    { "from": "plan", "to": "review_plan" },
    { "from": "review_plan", "to": "plan_review_branch" },
    { "from": "revise_plan", "to": "review_plan" },
    { "from": "parallel_process", "to": "quality_report" },
    { "from": "quality_report", "to": "human_qa" },
    { "from": "human_qa", "to": "qa_branch" },
    { "from": "build_index", "to": null }
  ]
}
```

- `from` 和 `to` 必须是合法的 TaskNode 或 ControlNode 的 id。
- `to: null` 表示终止边（该节点完成后 Workflow 可能结束）。
- branch 节点的 `paths` 值隐式定义了从 branch 到各 target 的条件边，**不需要**在 edges 中重复声明。
- parallel 节点的 `branches` 隐式定义了从 parallel 到各分支 TaskNode 的边，**不需要**在 edges 中重复声明。汇合边（分支 → parallel 节点的"完成"）由引擎根据 `join` 策略自动管理。

**设计理由**：将 edges 从嵌套的 flow block 中解耦为独立的顶层列表，使得 Workflow Graph 的拓扑结构一目了然，静态分析（可达性、终止性、循环检测）直接在 edge list 上做图算法即可。

### 3.6 Reference（数据引用——只读不算）

步骤之间传递数据的唯一方式。

**表面语法**（用于 JSON 配置中的字符串值）：

```
${node_id.output}                  -- 引用整个输出对象
${node_id.output.field}            -- 引用输出的一级字段
${node_id.output.field.sub_field}  -- 引用输出的嵌套字段
```

**正规文法**：

```
Reference     = "${" NodeRef "}"
NodeRef       = node_id "." "output" FieldPath?
FieldPath     = ("." field_name)+
node_id       = [a-z][a-z0-9_]*
field_name    = [a-zA-Z][a-zA-Z0-9_]*
```

**引擎内部表示**（解析后的结构体，用于静态分析）：

```json
{ "$ref": { "node": "scan", "io": "output", "path": "" } }
{ "$ref": { "node": "scan", "io": "output", "path": "total_files" } }
{ "$ref": { "node": "scan", "io": "output", "path": "by_type.video" } }
```

**强制约束**：

- **不允许表达式求值**：没有 `${a.output.count + 1}` 或 `${a.output.name.toUpperCase()}`。
- **不允许函数调用**：没有 `${format(a.output.date)}`。
- **不允许字符串拼接**：没有 `${"prefix_" + a.output.id}`。
- Reference 就是一个指针，指向某个节点输出的某个字段，引擎做字段级的取值传递。

如果需要数据转换（格式化、筛选、聚合、拼接），那是一个独立的 Step，由 Skill 执行。

**设计理由**：数据引用只读不算，保证引擎可以在运行前分析出完整的数据依赖图——谁依赖谁的输出、哪些步骤可以并行、类型是否兼容。

### 3.7 Guard（约束与门控）

附加在 Step 或 Workflow 上的约束条件。让人类在审查计划时就能看到风险边界，不需要读懂每一步的逻辑就能做风险判断。

```json
{
  "guards": {
    "budget": {
      "max_tokens": 100000,
      "max_cost_usdb": 0.5,
      "max_duration": "2h"
    },
    "permissions": ["fs.read", "fs.write", "network.feishu", "kb.write"],
    "retry": {
      "max_attempts": 3,
      "backoff": "exponential",
      "fallback": "human"
    }
  }
}
```

| 约束类型 | 说明 |
|----------|------|
| `budget.max_tokens` | Token 消耗上限 |
| `budget.max_cost_usdb` | 费用上限（USDB） |
| `budget.max_duration` | 时间上限 |
| `permissions` | 该步骤/Workflow 需要的权限声明 |
| `retry.max_attempts` | 失败重试次数 |
| `retry.backoff` | 重试策略（`fixed` / `exponential`） |
| `retry.fallback` | 重试耗尽后的行为（`human` = 暂停交人处理 / `abort` = 终止） |

**预算耗尽语义**：当某一步或全局预算耗尽时，引擎暂停整个 Workflow，通知人类。人类可以追加预算并继续，或终止。**绝不允许超预算静默继续执行。**

### 3.8 Amendment（计划修改——运行时协议）

Amendment 不是 DSL 语法的一部分，而是引擎提供的运行时协议。Agent 在执行某一步时，可以向引擎提交 plan amendment。

```json
{
  "type": "amendment",
  "submitted_by": "agent/mia",
  "submitted_at_step": "generate_plan",
  "operations": [
    {
      "op": "insert_after",
      "after_node": "validate_batch",
      "new_steps": [
        {
          "id": "dedup_check",
          "name": "重复数据检测",
          "executor": "skill/dedup-detector",
          "type": "autonomous",
          "output_schema": {
            "type": "object",
            "properties": {
              "duplicates_found": { "type": "integer" },
              "duplicates_removed": { "type": "integer" }
            }
          }
        }
      ],
      "new_edges": [
        { "from": "dedup_check", "to": "next_step_id" }
      ]
    }
  ],
  "reason": "扫描发现数据源中重复率超过 30%，建议在验证后增加去重步骤",
  "approval_required": true
}
```

**Amendment 操作类型**：

| 操作 | 说明 |
|------|------|
| `insert_after` | 在指定节点后插入新节点和边 |
| `insert_before` | 在指定节点前插入新节点和边 |
| `replace` | 替换指定节点（仅限未执行的节点） |
| `remove` | 删除指定节点（仅限未执行的节点） |

**关键约束**：

- Amendment 中新增的节点和边必须符合 DSL schema（引擎校验），且修改后的 Workflow Graph 必须通过完整的静态分析（见第 6 节）。
- 不能修改已完成的节点（历史不可篡改）。
- 每次 Amendment 生成一个新的 plan version，引擎保留完整版本历史。
- `approval_required`：是否需要人类审批。Workflow 可在全局 guards 中设置默认策略（如 `amendment_auto_approve: false`）。

### 3.9 可复用定义（defs）

Workflow 配置中可包含 `defs` 字段，用于定义可复用的 schema 片段，避免重复定义。语义与 JSON Schema 的 `$defs` 一致。

```json
{
  "defs": {
    "PlanOutput": {
      "type": "object",
      "properties": {
        "strategy_summary": { "type": "string" },
        "video_batches": { "type": "array" },
        "image_batches": { "type": "array" },
        "doc_batches": { "type": "array" },
        "estimated_tokens": { "type": "integer" },
        "estimated_duration": { "type": "string" }
      },
      "required": ["strategy_summary", "video_batches", "image_batches", "doc_batches"]
    }
  }
}
```

步骤的 `output_schema` 或 `input_schema` 中可使用 `{ "$ref": "#/defs/PlanOutput" }` 引用。

---

## 4. 引擎运行时

### 4.1 Run（一次具体执行）

一个 Workflow 配置的一次具体执行称为一个 Run。

```
Run 状态机：

created → running → waiting_human → running → ... → completed
                 ↘ failed → (retry | human_intervene | abort)
                 ↘ paused → (resume | abort)
                 ↘ budget_exhausted → (top_up | abort)
```

Run 在生命周期中可以在 `running` 和 `waiting_human` 之间反复切换。这就是 "Loop" 的体现。

**终止条件**：当 Workflow Graph 中不存在任何处于 `running`、`waiting_human` 或 `ready` 状态的节点时，Run 进入终止状态。若所有"可达终止边"上的节点均为 `completed`，则 Run 状态为 `completed`；否则为 `failed`。

### 4.2 Step 执行状态

每个 TaskNode 在一个 Run 中有独立的状态：

```
pending → ready → running → completed
                         ↘ failed → (retrying → running | waiting_human | aborted)
                         ↘ waiting_human → (completed | running)
                         ↘ skipped
```

| 状态 | 说明 |
|------|------|
| `pending` | 前置依赖未满足，尚不可执行 |
| `ready` | 前置依赖已满足，等待引擎调度 |
| `running` | 正在执行 |
| `completed` | 执行完成，输出已通过 output_schema 校验 |
| `failed` | 执行失败 |
| `retrying` | 失败后重试中 |
| `waiting_human` | 等待人类输入/确认 |
| `skipped` | 被人类跳过（仅限 `skippable: true` 的步骤） |
| `aborted` | 被人类或引擎终止 |

### 4.3 Skip 的输出语义

当人类跳过一个步骤时：

- 步骤状态变为 `skipped`。
- 步骤输出为 `null`。
- **`skippable` 约束**：只有声明了 `skippable: true`（默认）的步骤允许被跳过。关键步骤应设为 `skippable: false`。
- **下游兼容性要求**：如果一个步骤可能被跳过（`skippable: true`），任何引用该步骤输出的下游步骤的 `input_schema`（如果提供了）必须能接受 `null` 值。引擎在静态分析阶段检查此兼容性。如果没有提供 `input_schema`，引擎发出警告但不阻断。

### 4.4 人类介入机制

人类介入不是"一个特殊节点"，而是贯穿整个执行过程的能力。

#### 4.4.1 三种介入时机

| 时机 | 触发方 | 说明 |
|------|--------|------|
| 预设介入点 | Workflow 配置 | 步骤 type 为 `human_confirm` 或 `human_required` |
| Agent 请求介入 | Agent | Agent 执行中发现自己不确定/超出能力，通过引擎 API 请求暂停 |
| 人类主动介入 | 人类 | 人类在观察视图中随时可以暂停任何正在执行的步骤 |

#### 4.4.2 人类操作及其对状态的影响

| 操作 | 适用状态 | 对当前步骤的影响 | 对数据的影响 | 对后续步骤的影响 |
|------|----------|-----------------|-------------|-----------------|
| `approve` | `waiting_human` | → `completed` | `output.final_subject` = subject 原值 | 后续步骤变为 `ready` |
| `reject` | `waiting_human` | → `completed` | `output.decision` = `"rejected"`，`output.final_subject` = `null` | 触发 branch 的 rejected 路径 |
| `modify` | `waiting_human` | → `completed` | `output.final_subject` = 人类修改后的版本 | 后续步骤变为 `ready` |
| `retry` | `waiting_human` / `failed` | → `running`（attempt +1） | 清除当前输出，重新执行 | 后续步骤保持 `pending` |
| `skip` | `waiting_human` / `ready` | → `skipped` | output = `null` | 后续步骤变为 `ready`（需处理 null 输入） |
| `rollback` | 任意 | 目标步骤及之后所有步骤 → `pending`，输出清除 | 见 4.6 回退规则 | 全部重置为 `pending` |
| `take_over` | `running` / `waiting_human` | type 临时变为 `human_required` | 人类提供输出 | 正常流转 |
| `abort` | 任意 | 当前步骤 → `aborted` | 保留已有输出（用于审计） | 全部 → `aborted` |
| `amend_plan` | 任意 | 不影响当前步骤 | 不影响 | 后续未执行步骤按 Amendment 修改 |
| `top_up_budget` | `budget_exhausted` | 恢复 → 之前状态 | 不影响 | 不影响 |

#### 4.4.3 通知与交互通道

引擎本身不实现 UI 或消息推送。引擎通过事件系统（见 4.8）输出结构化的"人类行动请求"，由外部适配层路由到具体通道：

- OpenDAN 对话界面
- 飞书/钉钉/微信机器人
- Web 看板
- 邮件
- 移动端推送

### 4.5 持久化要求

每个 Run 必须持久化的最小状态集：

| 数据 | 说明 |
|------|------|
| Workflow 配置快照 | 含所有 Amendment 版本历史 |
| 每个 Node 的当前状态 | 上表中的状态枚举值 |
| 每个 Step 的输入和输出 | 用于回退重做和审计 |
| 人类介入记录 | 谁、什么时候、做了什么操作、附带什么理由 |
| 资源消耗记录 | 每个 Step 的 token 用量、耗时、费用 |
| 副作用回执 | 副作用型步骤返回的 `side_effect_receipt`（见 4.7） |
| 事件日志 | 所有状态变迁的完整时序记录 |

**恢复语义**：引擎进程重启后，必须能从持久化状态恢复所有 Running 状态的 Run，从最后一个已知状态点继续执行。对于 `idempotent: false` 的步骤，恢复时引擎不自动重新执行，而是标记为 `waiting_human`，由人类决定是否重做。

### 4.6 回退（Rollback）规则

回退是高风险操作，尤其涉及已产生副作用的步骤时。引擎采用以下规则：

**规则 1：回退范围**。rollback 指定一个 target_step_id，该步骤及其在 Workflow Graph 中所有可达的后续步骤的状态重置为 `pending`，输出清除。

**规则 2：副作用保护**。如果回退范围内存在 `idempotent: false` 且状态为 `completed` 的步骤，引擎**不允许自动回退穿越该步骤**。引擎暂停回退操作，进入 `waiting_human`，向人类展示：哪些副作用步骤会被重做、它们的 `side_effect_receipt` 是什么（如消息 ID、文件 hash、交易号），由人类明确确认后才继续回退。

**规则 3：回退不删除历史**。回退操作本身作为一个事件记录在事件日志中，包含：回退发起人、目标步骤、被重置的步骤列表、被重置步骤的原始输出快照。历史不可篡改，只是"从某个点重新开始"。

### 4.7 Executor 协议

Executor 是 Step 的实际执行者。引擎与 Executor 之间通过标准协议通信。

**引擎 → Executor 的调用参数（最小集合）**：

| 参数 | 说明 |
|------|------|
| `run_id` | 当前 Run 的唯一标识 |
| `step_id` | 当前 Step 的唯一标识 |
| `attempt` | 当前尝试次数（从 1 开始，重试时递增）。`run_id + step_id + attempt` 构成幂等键 |
| `input` | 引擎解析 Reference 后的实际输入数据 |
| `budget_remaining` | 本步骤剩余可用预算（token、USDB、时间） |

**Executor → 引擎的返回结果（最小集合）**：

| 字段 | 必需 | 说明 |
|------|------|------|
| `status` | 是 | `success` / `failed` / `request_human` |
| `output` | 条件 | `status` 为 `success` 时必需，须符合 output_schema |
| `error` | 条件 | `status` 为 `failed` 时必需，结构化错误信息 |
| `metrics` | 否 | 资源消耗：`{ tokens_used, duration_ms, cost_usdb }` |
| `side_effect_receipt` | 条件 | `idempotent: false` 的步骤必须返回。包含副作用的标识信息（如消息 ID、文件路径、交易号），用于审计和去重 |
| `human_request` | 条件 | `status` 为 `request_human` 时，描述需要人类帮助的内容 |

**取消协议**：引擎可以向正在执行的 Executor 发送 `cancel` 信号（如 parallel join:any 场景）。Executor 应在合理时间内停止执行并返回 `{ status: "cancelled" }`。如果 Executor 不响应取消，引擎在超时后强制标记步骤为 `aborted`。

### 4.8 事件系统

引擎通过事件系统与外部世界通信。所有事件都是结构化的 JSON。

**统一事件 Envelope**：

所有事件必须包含以下标准字段，以保证消费端可稳定实现幂等消费、断点续拉和审计重建：

```json
{
  "event_id": "evt-uuid-xxx",
  "type": "step.completed",
  "ts": "2026-03-01T14:30:00Z",
  "run_id": "run-abc-123",
  "plan_version": 1,
  "seq": 42,
  "actor": "engine",
  "node_id": "scan",
  "attempt": 1,
  "payload": { ... }
}
```

| 字段 | 必需 | 说明 |
|------|------|------|
| `event_id` | 是 | 事件唯一标识（UUID） |
| `type` | 是 | 事件类型 |
| `ts` | 是 | 事件时间（RFC 3339） |
| `run_id` | 是 | 所属 Run |
| `plan_version` | 是 | 事件发生时的计划版本号 |
| `seq` | 是 | 单 Run 内严格递增的序号，用于排序、重放与去重 |
| `actor` | 是 | 事件发起者（`engine` / `agent/<name>` / `human/<id>`） |
| `node_id` | 否 | 相关的节点 ID（Run 级事件可无） |
| `attempt` | 否 | 相关的尝试次数 |
| `payload` | 否 | 事件类型特定的附加数据 |

**核心事件类型**：

| 事件 | 触发时机 |
|------|----------|
| `run.created` | Run 创建 |
| `run.started` | Run 开始执行 |
| `run.completed` | Run 正常完成 |
| `run.failed` | Run 失败终止 |
| `run.paused` | Run 被暂停 |
| `run.aborted` | Run 被终止 |
| `step.started` | Step 开始执行 |
| `step.completed` | Step 完成 |
| `step.failed` | Step 失败 |
| `step.skipped` | Step 被跳过 |
| `step.waiting_human` | Step 等待人类介入 |
| `step.progress` | Step 执行进度更新（用于长时间步骤和 for_each 的批量进度） |
| `step.rollback` | Step 被回退（含原始输出快照） |
| `amendment.proposed` | Agent 提交 plan amendment |
| `amendment.approved` | Amendment 被批准 |
| `amendment.rejected` | Amendment 被拒绝 |
| `budget.warning` | 预算消耗达到阈值（如 80%） |
| `budget.exhausted` | 预算耗尽 |
| `human.action` | 人类执行了操作（含操作类型和理由） |

### 4.9 对外 API

引擎对外暴露的核心 API。分两类消费者：Agent（提交和修改计划）和人类/UI（观测和干预）。

#### 4.9.1 Agent 侧 API

| 接口 | 说明 |
|------|------|
| `POST /workflow/submit` | 提交一个 Workflow 配置，创建 Run |
| `POST /run/{id}/amendment` | 提交 plan amendment |
| `POST /run/{id}/step/{step_id}/request_human` | Agent 主动请求人类介入 |
| `POST /run/{id}/step/{step_id}/output` | Agent 提交步骤执行结果 |
| `POST /run/{id}/step/{step_id}/progress` | Agent 报告步骤进度 |

#### 4.9.2 人类 / UI 侧 API

| 接口 | 说明 |
|------|------|
| `GET /run/{id}` | 获取 Run 完整状态（含所有节点状态、当前进度） |
| `GET /run/{id}/graph` | 获取当前 Workflow Graph 的可视化结构（静态展开的节点图） |
| `GET /run/{id}/history` | 获取完整事件历史（支持 `since_seq` 参数做断点续拉） |
| `POST /run/{id}/step/{step_id}/approve` | 批准步骤输出 |
| `POST /run/{id}/step/{step_id}/reject` | 拒绝步骤输出 |
| `POST /run/{id}/step/{step_id}/modify` | 修改步骤输出后继续 |
| `POST /run/{id}/step/{step_id}/skip` | 跳过步骤 |
| `POST /run/{id}/step/{step_id}/retry` | 重试步骤 |
| `POST /run/{id}/step/{step_id}/take_over` | 接管步骤 |
| `POST /run/{id}/rollback/{target_node_id}` | 回退到指定节点 |
| `POST /run/{id}/pause` | 暂停 Run |
| `POST /run/{id}/resume` | 恢复 Run |
| `POST /run/{id}/abort` | 终止 Run |
| `POST /run/{id}/amendment/{amid}/approve` | 批准 Amendment |
| `POST /run/{id}/amendment/{amid}/reject` | 拒绝 Amendment |
| `POST /run/{id}/budget/top_up` | 追加预算 |

#### 4.9.3 外部系统集成 API

引擎也可作为被调用方接入企业已有系统（如通过 n8n 的 HTTP Request 节点触发）：

```
POST /agent/task
{
  "agent": "jarvis",
  "task": "根据客户需求选择素材组合",
  "context": { ... },
  "callback_url": "https://n8n.company.com/webhook/xxx",
  "budget": { "max_tokens": 50000, "max_cost_usdb": 0.2 }
}

→ 返回 task_id
→ 执行过程中如需人类确认，通过 Msg Center 发到企业 IM
→ 完成后 POST callback_url 返回结果
```

---

## 5. 完整示例：KB 素材库导入

以下示例展示 Mia 为一个 MCN 企业的 10TB 素材库导入任务生成的 Workflow 配置。

```json
{
  "schema_version": "0.2.0",
  "id": "kb-ingest-mcn-media-20260301",
  "name": "MCN 素材库导入",
  "description": "扫描 NAS 素材库，分类处理视频/图片/文档并编入知识库",

  "trigger": { "type": "manual" },

  "guards": {
    "budget": {
      "max_tokens": 5000000,
      "max_cost_usdb": 50.0,
      "max_duration": "72h"
    },
    "permissions": [
      "fs.read:/mnt/nas/media",
      "kb.write",
      "network.feishu"
    ],
    "amendment_auto_approve": false
  },

  "defs": {
    "PlanOutput": {
      "type": "object",
      "properties": {
        "strategy_summary": { "type": "string" },
        "video_batches": { "type": "array" },
        "image_batches": { "type": "array" },
        "doc_batches": { "type": "array" },
        "estimated_tokens": { "type": "integer" },
        "estimated_duration": { "type": "string" }
      },
      "required": ["strategy_summary", "video_batches", "image_batches", "doc_batches"]
    }
  },

  "steps": [
    {
      "id": "scan",
      "name": "扫描数据源",
      "executor": "skill/fs-scanner",
      "type": "autonomous",
      "input": { "path": "/mnt/nas/media" },
      "output_schema": {
        "type": "object",
        "properties": {
          "total_files": { "type": "integer" },
          "by_type": { "type": "object" },
          "total_size_bytes": { "type": "integer" },
          "sample_paths": { "type": "object" }
        },
        "required": ["total_files", "by_type", "total_size_bytes"]
      },
      "skippable": false,
      "guards": { "timeout": "30m" }
    },
    {
      "id": "plan",
      "name": "生成处理计划",
      "executor": "agent/mia",
      "type": "autonomous",
      "input": {
        "data_profile": "${scan.output}"
      },
      "output_schema": { "$ref": "#/defs/PlanOutput" },
      "skippable": false
    },
    {
      "id": "review_plan",
      "name": "审核处理计划",
      "type": "human_confirm",
      "subject_ref": "${plan.output}",
      "prompt": "请审核 Mia 生成的素材库处理计划。包含视频、图片、文档三类数据的分批处理策略和资源预估。",
      "output_schema": {
        "type": "object",
        "properties": {
          "decision": {
            "type": "string",
            "enum": ["approved", "rejected", "need_revision"]
          },
          "final_subject": { "$ref": "#/defs/PlanOutput" },
          "feedback": { "type": "string" }
        },
        "required": ["decision", "final_subject"]
      },
      "skippable": false
    },
    {
      "id": "revise_plan",
      "name": "修订处理计划",
      "executor": "agent/mia",
      "type": "autonomous",
      "input": {
        "original_plan": "${review_plan.output.final_subject}",
        "feedback": "${review_plan.output.feedback}"
      },
      "output_schema": { "$ref": "#/defs/PlanOutput" }
    },
    {
      "id": "process_videos",
      "name": "处理视频素材",
      "executor": "skill/video-kb-ingest",
      "type": "autonomous",
      "input": {
        "batches": "${review_plan.output.final_subject.video_batches}"
      },
      "output_schema": {
        "type": "object",
        "properties": {
          "processed": { "type": "integer" },
          "failed": { "type": "integer" },
          "quality_score": { "type": "number" }
        }
      },
      "idempotent": false,
      "guards": {
        "max_cost_usdb": 20.0,
        "retry": { "max_attempts": 2, "fallback": "human" }
      }
    },
    {
      "id": "process_images",
      "name": "处理图片素材",
      "executor": "skill/image-kb-ingest",
      "type": "autonomous",
      "input": {
        "batches": "${review_plan.output.final_subject.image_batches}"
      },
      "output_schema": {
        "type": "object",
        "properties": {
          "processed": { "type": "integer" },
          "failed": { "type": "integer" },
          "duplicates_removed": { "type": "integer" },
          "quality_score": { "type": "number" }
        }
      },
      "idempotent": false,
      "guards": { "max_cost_usdb": 15.0 }
    },
    {
      "id": "process_docs",
      "name": "处理文档素材",
      "executor": "skill/doc-kb-ingest",
      "type": "autonomous",
      "input": {
        "batches": "${review_plan.output.final_subject.doc_batches}"
      },
      "output_schema": {
        "type": "object",
        "properties": {
          "processed": { "type": "integer" },
          "failed": { "type": "integer" },
          "quality_score": { "type": "number" }
        }
      },
      "idempotent": false,
      "guards": { "max_cost_usdb": 10.0 }
    },
    {
      "id": "quality_report",
      "name": "生成质量报告",
      "executor": "agent/mia",
      "type": "autonomous",
      "input": {
        "video_result": "${parallel_process.output.process_videos}",
        "image_result": "${parallel_process.output.process_images}",
        "doc_result": "${parallel_process.output.process_docs}"
      },
      "output_schema": {
        "type": "object",
        "properties": {
          "summary": { "type": "string" },
          "overall_quality": { "type": "number" },
          "sample_entries": { "type": "array" },
          "issues": { "type": "array" }
        }
      }
    },
    {
      "id": "human_qa",
      "name": "人工质量抽检",
      "type": "human_confirm",
      "subject_ref": "${quality_report.output}",
      "prompt": "请抽检 KB 导入结果。Mia 已生成质量报告和抽样数据，请确认质量是否达标。",
      "output_schema": {
        "type": "object",
        "properties": {
          "decision": {
            "type": "string",
            "enum": ["accepted", "partial_redo", "full_redo"]
          },
          "final_subject": {
            "type": "object",
            "properties": {
              "summary": { "type": "string" },
              "overall_quality": { "type": "number" },
              "sample_entries": { "type": "array" },
              "issues": { "type": "array" }
            }
          },
          "feedback": { "type": "string" }
        },
        "required": ["decision", "final_subject"]
      },
      "skippable": false
    },
    {
      "id": "build_index",
      "name": "构建知识库索引",
      "executor": "skill/kb-index-builder",
      "type": "autonomous",
      "input": { "scope": "full" },
      "output_schema": {
        "type": "object",
        "properties": {
          "index_size": { "type": "integer" },
          "entry_count": { "type": "integer" }
        }
      },
      "skippable": false
    }
  ],

  "nodes": [
    {
      "id": "plan_review_branch",
      "type": "branch",
      "on": "${review_plan.output.decision}",
      "paths": {
        "approved": "parallel_process",
        "rejected": "revise_plan",
        "need_revision": "revise_plan"
      },
      "max_iterations": 3
    },
    {
      "id": "parallel_process",
      "type": "parallel",
      "branches": ["process_videos", "process_images", "process_docs"],
      "join": "all"
    },
    {
      "id": "qa_branch",
      "type": "branch",
      "on": "${human_qa.output.decision}",
      "paths": {
        "accepted": "build_index",
        "partial_redo": "parallel_process",
        "full_redo": "plan"
      },
      "max_iterations": 2
    }
  ],

  "edges": [
    { "from": "scan", "to": "plan" },
    { "from": "plan", "to": "review_plan" },
    { "from": "review_plan", "to": "plan_review_branch" },
    { "from": "revise_plan", "to": "review_plan" },
    { "from": "parallel_process", "to": "quality_report" },
    { "from": "quality_report", "to": "human_qa" },
    { "from": "human_qa", "to": "qa_branch" },
    { "from": "build_index", "to": null }
  ]
}
```

---

## 6. 静态分析要求

引擎在接收 Workflow 配置时（无论来自 Agent 还是人类），必须在执行前完成以下静态分析。任何校验失败都必须拒绝执行并返回明确错误信息。

| 校验项 | 说明 |
|--------|------|
| Schema 合法性 | 所有字段符合 DSL JSON Schema |
| Node ID 唯一性 | steps 和 nodes 中无重复 ID |
| 引用完整性 | 所有 Reference 中的 node_id 存在，且在 Workflow Graph 中位于当前节点的上游 |
| 类型兼容性 | 引用的字段路径在被引用节点的 output_schema 中存在且类型兼容。如果当前步骤提供了 input_schema，校验上游 output 与本步骤 input 的类型匹配 |
| 分支穷举性 | branch 的 paths 覆盖 `on` 引用字段的所有 enum 值 |
| 无无界循环 | 所有回路路径都有 `max_iterations` 约束 |
| Skip 兼容性 | 对于 `skippable: true` 的步骤，引用其输出的下游步骤能接受 null（如果提供了 input_schema 则强制校验，否则发出警告） |
| 权限声明完整 | 所有 executor 需要的权限在 guards.permissions 中声明 |
| 预算一致性 | 步骤级预算之和不超过全局预算（警告级，非阻断） |
| 可达性 | 所有节点都可从起始节点到达（无孤立节点） |
| 终止性 | 存在至少一条从起始节点到终止边（`to: null`）的路径 |
| subject_ref 合法性 | `human_confirm` 步骤的 `subject_ref` 引用的节点位于上游，且输出 schema 与 `final_subject` 的 schema 一致 |

---

## 7. 实现路径建议

### 7.1 最小可用版本（v0.1）

只实现以下子集：

- **节点类型**：TaskNode（`autonomous` + `human_confirm`）+ ControlNode 仅 `branch`
- **Edges**：完整支持
- **无 Amendment**：计划一旦批准不可运行时修改
- **无 for_each / parallel**
- **subject_ref + final_subject**：完整支持（这是 human_confirm 可用的前提）
- **Executor 协议**：最小集合（input/output/metrics/side_effect_receipt）
- **事件 Envelope**：完整支持（event_id / ts / seq / actor 等标准字段）
- **持久化**：文件系统 JSON（不引入数据库依赖）
- **事件系统**：内存事件总线 + 日志文件

用 Mia 的 KB 构建场景（小规模数据集，如几百个文档）验证核心循环：Agent 生成计划 → 人类审核（approve/modify subject）→ 引擎逐步执行 → 人类确认质量 → 完成或回退。

### 7.2 第二版（v0.2）

- 加入 `for_each`（支持批量处理场景，含实例化 ID 和聚合 Step 规范）
- 加入 `step.progress` 事件（支持长时间步骤的进度追踪）
- 加入 Amendment 机制
- 持久化升级为嵌入式数据库（如 SQLite）

### 7.3 第三版（v0.3）

- 加入 `parallel`（含 keyed 输出合并和 cancel 协议）
- 加入预算实时追踪与耗尽处理
- 对外 API 稳定化，支持外部系统集成（callback 模式）
- 加入 Workflow 模板的发布与安装（通过 cyfs://）

---

## 8. 成功标准

1. Agent（Mia/Jarvis）能在**一次 LLM 调用**内生成符合 DSL schema 的合法 Workflow 配置。
2. 引擎能在加载配置时**完成全部静态分析**并给出明确的通过/失败报告。
3. 人类能通过 `GET /run/{id}/graph` **随时获取可渲染的结构化节点图**，无需阅读 JSON 配置原文。
4. `human_confirm` 步骤中，人类**清楚知道自己在审查什么**（subject_ref），approve/modify/reject 的语义无歧义。
5. 任何 Run 在任意时刻被中断（进程崩溃、设备重启），恢复后能从**最后一个已持久化的状态点**继续，且 `idempotent: false` 的步骤不会被自动重执行。
6. 一个 Run 完成后，从事件历史中能**完整重建执行轨迹**：每步做了什么、花了多少、人类在哪里介入了什么。事件序列可通过 `seq` 字段可靠重放。
7. 预算约束**真的生效**：超出额度时流程暂停而不是继续消耗。
8. 回退操作在遇到 `idempotent: false` 的已完成步骤时，**必须停下来等人类确认**，不会静默重复副作用。
9. 同一个 Workflow 配置可以被**另一个 OpenDAN 实例安装并运行**，不依赖原作者的环境（executor 可用的前提下）。

---

## 9. 术语表

| 术语 | 定义 |
|------|------|
| **Workflow** | 一个完整的执行计划配置，由 DSL 描述 |
| **Workflow Graph** | Workflow 的执行结构，由 TaskNode、ControlNode 和 Edge 构成的有向图 |
| **Run** | Workflow 的一次具体执行实例 |
| **TaskNode / Step** | 执行业务逻辑的节点，引擎调度的原子执行单位 |
| **ControlNode** | 负责流转路由的节点（branch / parallel / for_each），不执行业务逻辑 |
| **Edge** | 节点之间的有向连接，定义执行流转方向 |
| **Reference** | 节点间数据传递的只读指针（`${node_id.output.field}`） |
| **Guard** | 附加在 Node 或 Workflow 上的约束条件 |
| **Amendment** | Agent 在运行时提交的计划修改请求 |
| **Executor** | Step 的执行者（`agent/*`、`skill/*`、`tool/*`、`human`） |
| **subject_ref** | `human_confirm` 步骤中，指向人类审查对象的 Reference |
| **side_effect_receipt** | 副作用型步骤返回的标识信息，用于审计和去重 |
| **Pipeline** | Workflow 的退化特例：所有步骤 autonomous、无分支、无 amendment |
| **Agent-Human-Loop** | 以人类确认/协作作为约束的 Agent 工作流闭环 |
| **DSL** | 受限的领域特定语言，保证可静态分析，不图灵完备 |
