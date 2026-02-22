
# Todo List 模块需求文档

## 0. 文档信息

* 模块名：Todo List / TODO 管理模块
* 适用范围：OpenDAN Runtime（Workshop/Workspace/Session/Behavior Step-loop）
* 依赖机制：

  * step-loop：在每个 behavior step 末尾应用 `todo_delta` 写入 workspace side effects
  * Prompt 组合：Memory 包含 Workspace Todo；Input 包含 Current Todo Details
  * PDCA：DO/CHECK 对 todo 状态有硬约束；Bench 在 CHECK 有特殊规则
  * SubAgent：可读取 parent todo 并更新“自己的 todo 状态”，且可 append worklog
  * Crash Recovery：必须可恢复 workspace 侧 todo 进度
  * local_workspace 并发：同一 local_workspace 只允许一个 session RUNNING，且可强化为等待对方 todo 完成/失败


---

## 1. 背景与目标

### 1.1 背景

Todo List 是 OpenDAN Runtime 用于把“计划—执行—验收—调整（PDCA）”落到可观测、可恢复、可协作的数据结构。它是 Workspace UI 和 Prompt Memory/Input 的关键数据源，同时也是 SubAgent 并行协作的主线“任务分解载体”。

### 1.2 目标

1. **可由 PLAN 一次性初始化完整 TodoList**（不依赖手工拼 deps）
2. **Agent 输出的 todo_delta 足够短**，默认只写“我要把哪个任务改成什么状态/追加什么 note”
3. 支撑 PDCA：DO 将 todo 推进到 `COMPLETE/FAILED`；CHECK 将 `COMPLETE->DONE` 或 `CHECK_FAILED`；Bench 在 CHECK 可从 `WAIT->DONE`
4. 支撑 SubAgent：SubAgent 只更新自己负责的 todo，并全链路审计
5. Workspace UI：可查看/筛选/干预 todo、notes、状态流转历史
6. Crash Recovery：任何时刻都能恢复 todo 进度，且变更可追溯（oplog）

### 1.3 非目标

* Todo 模块不执行工具/Action（由 Behavior/Tool/ActionExecutor/TaskMgr 执行）
* 不强制某一种工作流；但必须满足 Jarvis 默认 PDCA 的运行约束


---

## 2. 角色与典型使用场景

### 2.1 角色

* **Root Agent（Jarvis）**：PLAN 初始化 todo；DO/CHECK 更新状态；写 note 总结结果
* **SubAgent**：并行执行子任务，只更新“自己负责的 todo 状态/notes”
* **User（Workspace UI）**：查看 todo/worklog/subagent；可干预 todo（改优先级、改 assignee、强制 DONE/取消等）
* **System（Runtime）**：应用 todo_delta、校验状态机/权限、落库 sqlite + oplog、生成审计记录、做恢复

### 2.2 场景

1. 新任务进入：resolve_router -> PLAN，PLAN 输出 init todo list
2. DO 多 step：每 step 产出少量 delta（update + note）
3. CHECK：对 COMPLETE 任务验收，产出 DONE/CHECK_FAILED；Bench 在 CHECK 运行集成测试并 DONE
4. SubAgent 并行：web-agent/desktop-agent 执行子任务并回写自己的 todo
5. 用户 UI 干预：调整优先级/取消某任务/手动标记 DONE

---

## 3. 核心业务规则与状态机需求

## 3.1 TodoStatus（必须支持）

* `WAIT`：等待执行/等待依赖/等待 CHECK（Bench 常用）
* `IN_PROGRESS`：执行中
* `COMPLETE`：DO 阶段完成，等待 CHECK 验收
* `FAILED`：DO 阶段失败
* `DONE`：CHECK 验收通过
* `CHECK_FAILED`：CHECK 验收失败，进入 ADJUST

> 约束：系统必须校验状态迁移合法性，非法迁移必须拒绝并产生可解释错误（供下一 step observation 使用）。

## 3.2 TodoType（必须支持）

* `Task`：普通任务
* `Bench`：验收/集成测试任务（仅在 CHECK 阶段执行/完成）

## 3.3 状态迁移（与 PDCA 对齐，必须实现）

* PLAN：创建任务（默认 `WAIT`），可设置 skills/priority/labels/assignee
* DO：将任务推进到 `COMPLETE` 或 `FAILED`（允许多次重试，系统维护 attempts）
* CHECK：

  * 普通任务：`COMPLETE -> DONE` 或 `COMPLETE -> CHECK_FAILED`
  * Bench 特例：允许 `WAIT -> DONE`（失败则 `WAIT -> CHECK_FAILED`）

---

## 4. 数据模型需求（字段归属标注）

* **A（Agent/UI）**：由 Agent（todo_delta）或用户 UI 提供/修改
* **S（System）**：系统生成/维护（Agent 不得写）
* **A→S**：Agent 提意图，系统校验后落事实

---

## 4.1 TodoItem（任务项）

### 必需字段（按你最新定义）

| 字段             | 类型         | 归属                     | 说明                                                  |
| -------------- | ---------- | ---------------------- | --------------------------------------------------- |
| `id`           | string     | **S**                  | 全局唯一 ULID/UUIDv7，sqlite 主键                          |
| `todo_code`    | string     | **S**                  | workspace 内短号：`T001`…（让 Agent 写 `update:T001`）      |
| `workspace_id` | string     | **S**                  | 从 session/workspace 注入                              |
| `session_id?`  | string     | **S**                  | 创建/主要归因的 session（系统注入）                              |
| `title`        | string     | **A**                  | 一句话目标                                               |
| `description?` | string     | **A**                  | 详细说明/验收标准                                           |
| `type`         | TodoType   | **A**                  | `Task` / `Bench`                                    |
| `status`       | TodoStatus | **A→S**                | 状态更新由 delta 触发，系统校验后写入                              |
| `labels?`      | string[]   | **A**                  | 分类标签（UI 过滤/搜索）                                      |
| `skills?`      | string[]   | **A**                  | PLAN 初始化 skills（字符串列表）                              |
| `assignee?`    | DID        | **A（可选）/S（默认）**        | 不写则等于 `created_by.did`（系统补齐）                        |
| `priority?`    | int        | **A**                  | **越小越紧急**（0 最紧急）                                    |
| `deps?`        | string[]   | **A（少用）/S（Bench 可推导）** | 依赖（尽量少写；见第 5 节“deps 低出错策略”）                         |
| `estimate?`    | object     | **A**                  | `{hp?, walltime_ms?, tokens?}`                      |
| `attempts?`    | int        | **S**                  | 系统维护：失败/重试次数                                        |
| `last_error?`  | object     | **S**                  | `{code, message, trace_id}` 从 action/tool/task 结果生成 |
| `notes`        | TodoNote[] | **A→S（append-only）**   | 工作记录（建议用 note op 追加，不允许整段覆盖）                        |
| `created_at`   | int64(ms)  | **S**                  | 系统写入                                                |
| `updated_at`   | int64(ms)  | **S**                  | 系统写入                                                |
| `created_by`   | ActorRef   | **S**                  | 系统从执行主体注入（root/sub/user）                            |

---

## 4.2 TodoNote

| 字段           | 类型        | 归属      | 说明                  |       |                 |
| ------------ | --------- | ------- | ------------------- | ----- | --------------- |
| `author`     | DID       | **S**   | 系统注入（防伪造）           |       |                 |
| `kind?`      | string    | **A→S** | `result             | error | note`，缺省 `note` |
| `content`    | string    | **A**   | 内容                  |       |                 |
| `created_at` | int64(ms) | **S**   | 系统写入                |       |                 |
| `trace_id?`  | string    | **S**   | 可选：关联 TaskMgr trace |       |                 |

---

## 5. todo_delta 协议需求（Agent 视角：短、稳、不易错）

### 5.1 设计原则

* Agent 默认只输出最短 ops，不需要关心 workspace_id/version/op_id/actor/时间戳等
* 引用使用 `todo_code`（如 `T001`），避免 ULID 难写
* 初始化列表必须由 PLAN 完成，并且可以一次性构造完整 list
* deps 不是必填：默认顺序驱动；Bench deps 可由系统推导，降低出错率

### 5.2 最简 delta 结构（Agent 输出）

```jsonc
{
  "ops": [
    { /* op object */ }
  ]
}
```

系统在 apply 时自动补齐：workspace_id、actor、session_id、op_id、ts、版本信息等。

### 5.3 op 字段与语法（必须支持）

#### 5.3.1 更新状态

```json
{
  "op": "update:T001",
  "to_status": "COMPLETE",
  "reason": "实现完成，等待 CHECK 验收"
}
```

* `op`：`update:<todo_code>`
* `to_status`：目标状态
* `reason`：人类可读原因（必须写入审计/Worklog）

#### 5.3.2 追加 note（必须支持）

```json
{
  "op": "note:T001",
  "kind": "result",
  "content": "已生成产物 ...；关键结果 ... "
}
```

约束：notes 为 append-only；系统必须在 sqlite notes 表追加，不允许覆盖历史。

#### 5.3.3 PLAN 初始化完整列表（必须支持）

```jsonc
{
  "op": "init",
  "mode": "replace",
  "items": [
    {
      "title": "……",
      "type": "Task",
      "skills": ["git", "bash"],
      "priority": 0,
      "labels": ["setup"],
      "description": "验收标准……",
      "assignee": "did:od:jarvis",
      "deps": ["@prev"]   // 可选
    },
    {
      "title": "集成测试",
      "type": "Bench",
      "priority": 10
    }
  ]
}
```

* `mode=replace`：替换当前 todo list（新任务的 PLAN 默认）
* `mode=merge`：保留未完成项，仅追加/调整（长期 workspace 可用）
* 系统必须为 items 自动分配 `id` 和 `todo_code`（T001…），并写入 order 表（保持 PLAN 的 items 顺序）

### 5.4 deps “低出错策略”（必须落到实现）

1. **deps 非必填**：不写 deps 时，以 `order + priority` 作为默认推进顺序
2. **Bench deps 自动推导**：若 Bench 没写 deps，系统自动推导为“所有排在它前面的非 Bench todo”（或至少所有未完成普通任务），确保集成测试在 CHECK 执行时机合理
3. deps 语法糖（建议实现，降低 Agent 错误率）：

* `[]`：依赖前一个 todo ,不写默认的就是这个行为
* `["T001","T003"]`：显式依赖短号（系统需校验存在性）

---

## 6. 系统侧 apply 需求：`apply_todo_delta(delta, session)`

### 6.1 调用时机（必须）

* 必须在每个 behavior step 末尾、Workspace side effects 阶段调用（若 llm_result.todo_delta 存在）

### 6.2 apply 行为（必须）

系统必须在一个事务内完成：

1. 解析 `ops[]`
2. 将 `T001` 解析成 sqlite 中对应 todo（workspace_id + todo_code 唯一）
3. 权限校验（root/sub/user/system）
4. 状态机校验（PDCA + Bench 特例）
5. 写 sqlite（items/notes/deps/order/meta/version）
6. 写 `todo_applied_ops`（幂等去重）
7. 事务成功后 append `oplog.jsonl` 一行（审计与重放）
8. 同步生成/追加 Worklog（用户可见审计摘要）

### 6.3 错误返回（必须）

* todo_code 不存在：`NOT_FOUND`
* 非法状态迁移：`INVALID_TRANSITION`
* 权限不足：`FORBIDDEN`
* 事务失败：`INTERNAL_ERROR`
  错误必须可被写入下一 step observation（供 Agent 修正输出）。

---

## 7. 持久化需求：`oplog.jsonl + todo.sqlite`

### 7.1 文件结构（必须）

```
$workspace_root/todo/
  ├── todo.sqlite
  └── oplog.jsonl
```

### 7.2 oplog.jsonl（必须）

* append-only，每行一个 JSON
* 记录至少：ts、op_id、workspace_id、session_id、actor、ops、before/after 版本、result

示例：

```jsonc
{
  "ts": 1761187200000,
  "op_id": "01J...ULID",
  "workspace_id": "ws_xxx",
  "session_id": "sess_xxx",
  "actor": { "kind": "root_agent", "did": "did:od:jarvis" },
  "ops": [ { "op": "update:T001", "to_status": "COMPLETE", "reason": "..." } ],
  "before_version": 12,
  "after_version": 13,
  "result": "applied"
}
```

### 7.3 sqlite 表结构（必须提供，作为实现验收的一部分）

> 说明：SQLite 字段里对 array/object 使用 JSON TEXT 存储；deps/notes 用单独表便于查询与追溯。

#### 7.3.1 元信息表

```sql
CREATE TABLE IF NOT EXISTS todo_meta (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
/* key:
   - version: integer string
*/
```

#### 7.3.2 todo_items 主表（必须）

```sql
CREATE TABLE IF NOT EXISTS todo_items (
  id TEXT PRIMARY KEY,                 -- S: ULID/UUIDv7
  workspace_id TEXT NOT NULL,           -- S
  session_id TEXT,                      -- S

  todo_code TEXT NOT NULL,              -- S: "T001"
  title TEXT NOT NULL,                  -- A
  description TEXT,                     -- A
  type TEXT NOT NULL,                   -- A: Task|Bench
  status TEXT NOT NULL,                 -- A→S: WAIT|IN_PROGRESS|COMPLETE|FAILED|DONE|CHECK_FAILED

  priority INTEGER,                     -- A: smaller = more urgent
  labels_json TEXT,                     -- A: JSON array
  skills_json TEXT,                     -- A: JSON array
  assignee_did TEXT,                    -- A/S default

  estimate_json TEXT,                   -- A: JSON object
  attempts INTEGER NOT NULL DEFAULT 0,  -- S
  last_error_json TEXT,                 -- S: JSON object

  created_at INTEGER NOT NULL,          -- S: ms
  updated_at INTEGER NOT NULL,          -- S: ms
  created_by_kind TEXT NOT NULL,        -- S
  created_by_did TEXT,                  -- S

  UNIQUE(workspace_id, todo_code)
);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_status
  ON todo_items(workspace_id, status);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_priority
  ON todo_items(workspace_id, priority, updated_at);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_assignee
  ON todo_items(workspace_id, assignee_did);

CREATE INDEX IF NOT EXISTS idx_todo_items_ws_updated
  ON todo_items(workspace_id, updated_at DESC);
```

#### 7.3.3 deps 表（推荐，必须在需求中定义）

```sql
CREATE TABLE IF NOT EXISTS todo_deps (
  workspace_id TEXT NOT NULL,
  todo_id TEXT NOT NULL,
  dep_todo_id TEXT NOT NULL,
  PRIMARY KEY (workspace_id, todo_id, dep_todo_id)
);

CREATE INDEX IF NOT EXISTS idx_todo_deps_ws_todo
  ON todo_deps(workspace_id, todo_id);
```

#### 7.3.4 notes 表（必须）

```sql
CREATE TABLE IF NOT EXISTS todo_notes (
  note_id TEXT PRIMARY KEY,          -- S: ULID
  workspace_id TEXT NOT NULL,
  todo_id TEXT NOT NULL,

  author_did TEXT NOT NULL,          -- S
  kind TEXT NOT NULL DEFAULT 'note', -- A→S
  content TEXT NOT NULL,             -- A
  created_at INTEGER NOT NULL,       -- S

  session_id TEXT,                   -- S
  trace_id TEXT                      -- S
);

CREATE INDEX IF NOT EXISTS idx_todo_notes_ws_todo_time
  ON todo_notes(workspace_id, todo_id, created_at DESC);
```

#### 7.3.5 顺序表（必须，支撑“deps 非必填、按列表推进”）

```sql
CREATE TABLE IF NOT EXISTS todo_order (
  workspace_id TEXT NOT NULL,
  pos INTEGER NOT NULL,
  todo_id TEXT NOT NULL,
  PRIMARY KEY (workspace_id, pos),
  UNIQUE (workspace_id, todo_id)
);
```

#### 7.3.6 幂等表（必须，保证重放安全）

```sql
CREATE TABLE IF NOT EXISTS todo_applied_ops (
  op_id TEXT PRIMARY KEY,
  workspace_id TEXT NOT NULL,
  session_id TEXT,
  actor_did TEXT,
  applied_at INTEGER NOT NULL,
  ops_json TEXT NOT NULL
);
```

---

## 8. 工具/API 需求（给 Runtime/UI/LLM 工具层）

> LLM 主路径是输出 `llm_result.todo_delta`；但 UI/系统也需要调用同一套能力。

必须提供的接口能力（不限定语言/框架）：

1. `todo.list(workspace_id, filters, limit, offset) -> {items[], version}`
2. `todo.get(workspace_id, todo_ref(T001|id)) -> {item, notes[], deps[], version}`
3. `todo.apply_delta(workspace_id, delta, actor_ctx) -> {ok, new_version, errors[]}`
4. `todo.query_pending(workspace_id, states=[...]) -> {has_pending, counts_by_status}`（给 local_workspace 并发锁/调度用）
5. `todo.render_for_prompt(workspace_id, token_budget) -> string`（Memory：Workspace Todo）
6. `todo.render_current_details(session_id) -> string`（Input：Current Todo Details）

---

## 9. Prompt 集成需求（必须）

### 9.1 Memory：Workspace Todo（必须）

* 系统必须把 Workspace Todo 作为 Memory 段的一部分进行注入（受 token 预算控制）
* 输出格式要求：

  * 必须包含 `todo_code`（T001）
  * 必须包含 `title/status/assignee/priority`
  * 优先展示未完成（WAIT/IN_PROGRESS/COMPLETE）与高优先级项

### 9.2 Input：Current Todo Details（必须）

* 系统 Input 段必须能提供“当前 todo 详情”模板位（如 session 选择了 current todo）
* 建议包含：

  * title/description/acceptance
  * deps 完成情况
  * 最近 notes（top K）
  * 最近 last_error（如有）

---

## 10. Workspace UI 需求（必须）

Workspace UI 必须支持：

1. Todo 列表视图：筛选（status/type/assignee/labels）、排序（priority/updated_at）、搜索（title/description）
2. Todo 详情页：字段展示 + notes 时间线 + deps + 状态流转历史（可由 oplog/worklog 派生）
3. 用户干预（写入同一 apply 通道）：

   * 改 priority / labels / assignee / description
   * 强制标记 DONE / 取消（若实现 CANCEL）
   * 追加 note
4. 并行可观测：显示 SubAgent 负责哪些 todo、其最后更新时间与状态

> UI 与 Agent 修改必须走同一 `apply_delta`，确保审计一致。

---

## 11. 权限与审计需求（必须）

### 11.1 权限规则（默认要求）

* Root Agent：可创建/更新任何 todo
* SubAgent：只允许更新 `assignee == 自己` 的 todo（状态 + note），禁止修改 assignee/skills 等关键字段（除非 policy 放宽）
* User：可查看全部；可干预（按产品策略可更强）
* System：可做修复/回放/一致性修正

### 11.2 审计（必须）

* 每个 op 必须：

  * 写入 `oplog.jsonl`（append-only）
  * 写入 sqlite（事实状态）
  * 生成可读 worklog 条目（用户可见“谁在何时把 T001 从 A 改到 B，原因/链接 trace”）

---

## 12. 并发与一致性需求（必须）

### 12.1 session 串行、subagent 并行（背景约束）

* 同一 session 的 step 串行
* SubAgent 并行写 todo 的情况下，系统必须保证 sqlite 写入原子性与幂等性

### 12.2 local_workspace 锁联动（必须支持查询）

* 若两个 session 绑定同一个 local_workspace，同一时刻只能一个 RUNNING
* 可强化策略：等待另一个 session 的 todo “全部 COMPLETE/FAILED/DONE…”（实现上用 `todo.query_pending` 支撑）

---

## 13. Crash Recovery 需求（必须）

* 系统重启后必须恢复 workspace 侧 todo 进度（sqlite snapshot）
* 如 sqlite 与 oplog 不一致，必须可用 oplog 重放/校验
* `todo_applied_ops` 必须保证幂等：重复 op 不会重复生效


---

## 14. 非功能需求（NFR）

1. **性能**：

* `todo.list` 在 10k 条 todo 下，常用筛选/排序查询应可在可接受延迟内完成（靠索引保障）

2. **可靠性**：

* apply_delta 必须事务化：要么全部写入成功，要么失败回滚

3. **可观测**：

* 任一 todo 变更可追溯到 actor/session/trace

4. **可扩展**：

* skills/labels/estimate 允许以 JSON 扩展，不破坏兼容

5. **安全**：

* author/created_by 等身份字段必须由系统注入，不允许 Agent 伪造

---

## 15. 验收标准（可直接用于测试用例）

### AC-1 PLAN 初始化

* 当 PLAN 输出 `init(mode=replace, items=[...])`
* 系统 apply 后 sqlite 中存在 N 条 todo_items
* 每条都有 `id`、连续 `todo_code(T001..)`、`created_at/updated_at/created_by`
* order 表 pos 与 items 顺序一致

### AC-2 极简 update

* 当 DO 输出 `{op:"update:T001", to_status:"COMPLETE"}`
* 系统必须把对应 item 状态改为 COMPLETE，并在 oplog 写一行审计记录

### AC-3 Bench 特例

* 当 CHECK 对 Bench 输出 `{op:"update:T0xx", to_status:"DONE"}` 且之前为 WAIT
* 系统必须允许并成功（或失败到 CHECK_FAILED），且写入审计

### AC-4 notes 追加

* 当输出 `{op:"note:T001", content:"..."}`
* sqlite todo_notes 新增一条记录，todo_items 不覆盖旧 notes

### AC-5 SubAgent 权限

* SubAgent 尝试更新非自己 assignee 的 todo：apply 必须拒绝 FORBIDDEN，并写审计（可选：写拒绝事件）

### AC-6 Crash Recovery

* 在 apply 成功后模拟崩溃重启
* sqlite 中 todo 状态不丢失；必要时可用 oplog 校验一致

---
