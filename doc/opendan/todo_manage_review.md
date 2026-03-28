# todo_manage AgentTool Code Review

## 一、架构概览

`todo_manage` 是 OpenDan 工作区内的任务管理工具，实现位于 `src/frame/opendan/src/workspace/todo.rs`（约 4000 行），通过 `AgentWorkshop` 在 `workshop.rs` 中注册。核心能力：

- **7 个 action**：`list` / `get` / `apply_delta` / `query_pending` / `render_for_prompt` / `render_current_details` / `get_next_ready_todo`
- **持久化**：SQLite + oplog.jsonl
- **PDCA 状态机**：Task/Bench 类型，6 种状态（WAIT/IN_PROGRESS/COMPLETE/FAILED/DONE/CHECK_FAILED）
- **Delta 操作**：`init` / `update:Txxx` / `note:Txxx`

---

## 二、优点

### 1. 领域建模清晰

- `TodoType` / `TodoStatus` / `ActorKind` 等枚举定义明确
- `validate_transition` 显式约束状态流转，避免非法转换
- `assert_subagent_permission` 区分 root_agent / sub_agent 权限

### 2. 幂等与审计

- `todo_applied_ops` 记录 op_id，`has_applied_op` 实现幂等
- oplog.jsonl 记录每次 apply，便于审计与回放

### 3. 依赖与顺序

- `todo_deps` 表维护依赖关系
- `get_next_ready_todo` 通过 `NOT EXISTS` 子查询保证依赖满足后再取任务
- `@prev` 语法支持 init 时隐式依赖前一项

### 4. 异步与阻塞隔离

- `run_db` 使用 `task::spawn_blocking` 将 SQLite 操作放到线程池，避免阻塞 async runtime

### 5. 事件发布

- 状态变更通过 `KEventClient` 发布 `TodoStatusChangedEvent`，便于外部订阅

---

## 三、问题与建议

### 1. Bug：priority 排序忽略 asc 参数

**位置**：`todo.rs` 约 2194–2201 行

```rust
"priority" => {
    sql.push_str(" ORDER BY i.priority IS NULL ASC, i.priority");
    if filters.asc {
        sql.push_str(" ASC");
    } else {
        sql.push_str(" ASC");  // ← 两分支相同，asc=false 时也应支持 DESC
    }
```

**建议**：`asc=false` 时使用 `DESC`，与 `order` / `updated_at` 分支一致。

---

### 2. 文件体积过大

- `todo.rs` 约 4000 行，职责较多
- 建议拆分：
  - `todo_schema.rs`：建表、索引
  - `todo_delta.rs`：`apply_todo_delta`、`apply_init_op`、`apply_update_op`、`apply_note_op`
  - `todo_query.rs`：`list_todo_items`、`get_todo_detail`、`query_pending_counts`、`list_for_prompt`、`get_next_ready_todo`
  - `todo_render.rs`：`render_workspace_todo_text`、`render_current_todo_text`

---

### 3. Delta 解析与 op 格式

- `DeltaOp::parse` 中 `op` 支持 `update:T001`、`note:T002` 等格式，但错误信息只写 `init/update:Txxx/note:Txxx`，对 `update:T001` 这种写法不够直观
- 建议在错误信息或文档中明确示例，减少误用

---

### 4. ApplyDeltaInput 参数来源分散

- `delta` 可从 `args.delta`、`args.todo_delta` 或 `args` 本身读取
- `ops` 可从 `delta.ops` 或 `args.ops` 读取
- 建议统一约定（例如只认 `delta.ops`），并在文档中说明，避免行为不一致

---

### 5. 错误类型分层

- 当前 `DomainError` 与 `AgentToolError` 混用，`apply_single_op` 返回 `DomainError`，上层再转换
- 建议：保持 `DomainError` 作为内部领域错误，在 `apply_todo_delta` 边界统一转为 `AgentToolError`，便于上层统一处理

---

### 6. SQL 注入与 LIKE 转义

- `escape_like` 已用于 `label`、`query` 的 LIKE 条件
- 建议确认所有用户输入在拼接 SQL 前都经过转义或参数化

---

### 7. 并发与锁

- 每个 workspace 一个 SQLite 文件，`run_db` 每次打开新连接
- 高并发下可能出现 SQLite `BUSY`，可考虑：
  - 连接池
  - 或对同一 workspace 使用单连接 + 队列

---

### 8. opendan_tools.md 与实现不一致

- `opendan_tools.md` 中 `todo_manage` 的 args_schema 只有 `ops`、`workspace_id`
- 实际实现需要 `action`、`delta.ops` 等
- 建议同步更新文档，避免调用方按文档传参出错

---

## 四、Bash 环境下接受度较高的 Todo 工具

### 1. Taskwarrior（最主流）

- **特点**：功能完整，支持项目、标签、截止时间、依赖、自定义报告
- **存储**：SQLite
- **社区**：活跃，约 60+ 贡献者
- **适用**：需要完整 GTD/Kanban 流程的用户

### 2. todo.sh（Todo.txt CLI）

- **特点**：纯 Bash，轻量，约 6000 GitHub stars
- **存储**：`todo.txt` 纯文本
- **格式**：`(A) 任务内容 +project @context`
- **适用**：偏好纯文本、简单工作流的用户

### 3. 可借鉴点

| 能力         | Taskwarrior | todo.sh | todo_manage |
|--------------|-------------|---------|-------------|
| 依赖         | ✅          | ❌      | ✅          |
| 项目/标签    | ✅          | ✅      | ✅ (labels) |
| 纯文本存储   | ❌          | ✅      | ❌ (SQLite) |
| 自然语言语法 | ✅          | 部分    | ❌          |
| 幂等/审计    | 部分        | ❌      | ✅          |

`todo_manage` 在幂等、审计、依赖、PDCA 状态机方面已经超过传统 CLI 工具，更适合 Agent 场景。若希望更贴近人类使用习惯，可考虑：

- 支持类似 `task add "xxx"` 的简化语法
- 提供 `todo.txt` 格式的导入/导出，便于与现有工具互操作

---

## 五、总结

`todo_manage` 设计合理，领域建模、幂等、审计、依赖和权限控制都做得较好。主要改进点：

1. 修复 `priority` 排序的 `asc` 逻辑
2. 拆分 `todo.rs` 以降低复杂度
3. 统一并文档化参数来源（`delta` / `ops`）
4. 同步 `opendan_tools.md` 与实现
5. 评估 SQLite 并发策略（连接池或串行化）
