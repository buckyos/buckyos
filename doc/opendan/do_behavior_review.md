# DO Behavior Review（详细版）

## 1. 目标与约束

基于 `opendanv2.md` 8.3，DO 的职责是：
- 持续推进当前 TODO，直到 `Complete` 或 `Failed`
- 多 step 迭代执行：每步依据上一轮 action/tool 结果调整策略
- 有依赖阻塞或缺少输入时进入等待
- 末段包含自检，并允许一次自修复；多次失败后标记失败
- 下一行为应进入 `CHECK`

这意味着 DO 不是“单步完成器”，而是“执行循环控制器”。

## 2. 运行时行为回顾（结合 agent loop）

从 `agent.rs` 的执行流程看，DO 行为每轮包含：
1. `generate_input`：按 `do.yaml.input` 渲染输入；如果没有任何可用输入，当前轮直接跳过并进入等待态  
2. LLM 推理：得到 `llm_result`（含 `next_behavior/reply/todo/actions/...`）  
3. 执行 `actions`：由 runtime 顺序执行，默认失败即截断后续动作  
4. 写回副作用：
- `reply` 发消息
- `set_memory` 写记忆
- `todo` 通过 `todo_manage.apply_delta` 自动落库
5. 状态迁移：
- `next_behavior = check`：切到 CHECK
- `next_behavior = WAIT_FOR_MSG`：等待用户输入
- `next_behavior = END`：会话睡眠
- `next_behavior = None`：继续当前 DO 的下一 step，直到命中 `step_limit`

关键结论：DO 的“推进”主要由 `todo + actions + next_behavior` 三者协同完成。

## 3. 来自现有 TODO Review 的问题点

`notepads/todo.md` 的 Agent Loop 草稿体现了几个现实问题：
- DO 可能在中途因信息不足进入 `WAIT_FOR_MSG`
- DO 结束后通常应进入 CHECK，而不是直接 END
- 若输入源不足（无 `new_msg` 且无可读 todo 上下文），DO step 会被跳过

因此 `do.yaml` 设计必须保证：
- 输入模板尽量稳定拿到“当前 todo + 上一步摘要”
- 明确等待条件，避免空转
- 将“完成/失败判定”输出为可消费的 todo 变更

## 4. do.yaml 的设计决策

### 4.1 输入设计

DO 输入采用：
- `last_step_summary`：用于迭代推进
- `new_msg`：接收用户补充
- `workspace.todolist.__OPENDAN_ENV(params.todo)__`：支持 `do:todo=T001` 精准定位
- `workspace.todolist.next_ready_todo` + `current_todo`：无显式参数时自动回退到当前可执行 todo

目的：减少“无输入可渲染”导致的 step 跳过。

### 4.2 工具策略

DO 是执行主阶段，必须允许工具：
- 文件与命令：`exec/read/write/edit`
- 状态维护：`todo_manage/worklog_manage`
- 上下文辅助：`load_memory/get_session`
- 扩展能力：`create_sub_agent/list_external_workspaces/bind_external_workspace`

并通过 allow-list 控制边界，避免过宽权限。

### 4.3 状态机策略

DO 内规则：
- 默认 `next_behavior` 为空，持续执行下一个 step
- 当前 todo 达到完成/失败判定后，统一切 `check`
- 缺少关键输入或依赖未满足时，切 `WAIT_FOR_MSG`
- 行为超出 `step_limit` 时，fallback 到 `adjust`（由 `faild_back` 托底）

### 4.4 质量控制

强制要求每个 TODO 在结束前至少完成一次：
- 自检（结果对照验收标准）
- 失败时单次自修复尝试
- 再失败才标记 Failed 并转 CHECK

## 5. 风险与后续建议

- 风险1：TODO 操作 schema 仍偏宽松，可能出现低质量 delta  
建议：后续在 prompt 增加更严格的 todo op 示例模板

- 风险2：工具输出噪声较大，可能影响下一 step 决策  
建议：在 step_summary 中固定“结论先行”格式

- 风险3：`do:todo=...` 与“next_ready_todo”并存时可能出现目标漂移  
建议：若存在 `params.todo`，优先锁定并忽略其它 todo 候选

