# CHECK Behavior Review（详细版）

## 1. CHECK 的职责边界

依据 `opendanv2.md` 8.4：
- CHECK 负责把 `COMPLETE` 的 TODO 进行验证并转为 `DONE`
- 发现问题不做修复，立即标记 `CHECK_FAILED`，然后切 `ADJUST`
- Bench 类型允许在 CHECK 阶段从 `WAIT` 直接进入 `DONE`
- CHECK 完成后要么进入下一个 `DO`，要么 `ADJUST`，要么 `END`

结论：CHECK 是“验证与分流器”，不是执行/修复器。

## 2. 状态机与 todo 引擎约束（代码事实）

`workspace/todo.rs` 的合法状态迁移要点：
- `COMPLETE -> DONE` 合法
- `COMPLETE -> CHECK_FAILED` 合法
- `WAIT -> DONE` 仅对 `Bench` 合法
- `DONE` 不可再迁移
- `CHECK_FAILED` 可回到 `IN_PROGRESS/COMPLETE/FAILED`

因此 CHECK 输出时：
- 普通 Task 必须先完成（`COMPLETE`）再转 `DONE`
- Bench 可在 CHECK 直接收敛（`WAIT -> DONE`）但需要验证证据

## 3. CHECK 复杂点

### 3.1 目标选择复杂

CHECK 不是只看一个 todo：
- 需要优先覆盖 `COMPLETE`
- 还要兼顾 `Bench + WAIT` 的验证场景
- 同时避免误把尚未进入检查条件的任务标记为 `DONE`

### 3.2 只验不修复杂

验证失败后不能在 CHECK 内修复：
- 只能记录失败证据（note/reason/last_error）
- 将状态置 `CHECK_FAILED`
- 立即切 `ADJUST`

### 3.3 分流复杂

CHECK 结束时可能有三种去向：
- `ADJUST`：有任何 check 失败
- `do`：仍有可推进任务
- `END`：全部收敛（通常全部 `DONE`，或无后续可执行任务）

## 4. check.yaml 的设计决策

### 4.1 输入策略

输入优先使用：
- `last_step_summary`：连续检查上下文
- `new_msg`：吸收用户补充的验收依据
- `workspace.todolist.__OPENDAN_ENV(params.todo)__`：支持定点检查
- `workspace.todolist.next_ready_todo` / `current_todo`：无显式参数时兜底

同时通过 memory 高配 `workspace_todo + workspace_worklog`，保证可见完整 TODO 与执行证据。

### 4.2 工具策略（只读偏验证）

CHECK 默认允许：
- `read/exec`：读取产物与跑验证命令
- `worklog_manage/get_session/load_memory`：补充上下文与证据
- `todo_manage`：仅在必要时查询（状态变更仍优先走 `todo` 字段）

不鼓励写工具（`write/edit`）做修复。

### 4.3 输出策略

- 通过：对已验证项写 `update:Txxx -> DONE`
- 失败：写 `update:Txxx -> CHECK_FAILED`，并附 `reason`（必要时 `last_error`）
- 行为切换：
  - 任一失败：`next_behavior = adjust`
  - 无失败且仍有待执行任务：`next_behavior = do`
  - 全部收敛：`next_behavior = END`

## 5. 风险与建议

- 风险1：验证证据不足导致误判  
建议：CHECK step 内先做“证据收集子步”，再落状态变更

- 风险2：把修复逻辑混入 CHECK  
建议：规则中显式禁止“修复型 actions”，仅允许验证/观察动作

- 风险3：分流时机不一致  
建议：统一采用“先判失败，再判是否有待执行任务，最后 END”的优先级

