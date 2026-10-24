# Agent Work Session TodoList

版本 2 · 2026-10-16 · 当前实现：`agent_tool::todo_tools`（数据模型与存储）、`libopendan::todos` 与 `runner::todos`（派发、回写、准入、视图）、`xagent` 的 `todo` 命令（`agent-session todo …`）、Jarvis `plan` / `do` / `explore` behavior。实施记录见 [notepads/opendan-plan-do-todo-list-todo.md](../../notepads/opendan-plan-do-todo-list-todo.md)。

## 1. 定位

TodoList 是 Work Session 的 PLAN 的**持久决策依据**。PLAN 把目标拆成 Todo，为每个 Todo 选择执行形态，收回每次执行的结果，Review 后再决定下一步。TodoList 不是通用任务系统，也不是依赖图调度器：它是 PLAN 的计划与记忆，执行由 Session 已有的机制承担（子 context、Sub Session）。

设计要点：

1. **树形 Plan-Do**：执行者只负责把自己领到的那一件事做完并报告；下一步做什么只由 PLAN 决定。执行者不选 Todo、不改列表、不决定状态机去向。
2. **全局视角**：PLAN 在每个决策边界看到一份紧凑的 TodoList 视图（每个 Todo 的目标、形态、状态、一句话结果、报告引用、Review 状态），不含执行细节；细节按需读取（§7）。
3. **两种执行形态**：串行执行（本 Session 的子 context，PLAN 的 run 挂起等它返回）与探索执行（独立的 Sub Session，可并行）。Review 的入口统一在 Todo 记录上。
4. **执行成功 ≠ 结果被接受**：每次执行的结果都要 PLAN Review；Session 只有在列表收敛后才能成功结束。
5. **身份可追溯**：Todo → attempt → (`child_sid` | `(run_id, call_id)`) → 报告，全部持久化，不靠模型记忆或从自然语言里猜。

与 Task、Sub Session 的关系：

| 概念 | 归属 | 用途 |
| --- | --- | --- |
| Todo | 一个 Work Session（`todos.json`） | PLAN 的工作项与决策记录 |
| 串行 attempt | 本 Session 的一次子 context 调用 | Todo 由本 Session 的 Todo 执行器（DO）完成 |
| 探索 attempt | 本 Session 的一个 Sub Session | Todo 由独立 Work Session 完成，可并行 |
| 系统级 Task | task-mgr | Turn 的 task、后台命令等；不是 Todo 的执行通道 |

旧设计中的 `addTodo / currentTodo / finishTodo / delegateTask` 工具、workspace 级 `tasks.json` 与“执行者自己 `todo done`”均已删除：执行者不写列表，结果由宿主依据已提交状态回写。

## 2. 存储与写者

```text
<SessionDir>/.opendan_agent_session/
  todos.json                       TodoList（schema opendan.todo_list/1，整文件原子替换）
  todos.lock                       列表锁（flock；只用于排序写者，进程退出即释放）
  reports/<report_id>/submission.json   串行 attempt 的报告副本（不随 run 目录清理而消失）
```

- **单写者**：父 Session 的宿主（runner）及其 run 的工具进程（`agent-session todo …`，同一 lease 下）。Sub Session 永远不写父列表；owner 侧（`xagent todo list/show`）只读。
- 每次修改都是“加锁 → 读整个列表 → 修改 → 原子替换”。派发探索时锁一直持有到绑定写回，宿主因此不会看到半完成的派发。
- 没有 `todos.json` 等同于空列表。旧格式（数组）不兼容、不迁移。

## 3. 数据模型

```jsonc
{
  "schema": "opendan.todo_list/1",
  "next_seq": 4,              // ID 计数器：单调、持久，cancel 后也不复用
  "rev": 17,                  // 列表修订号：每次修改 +1（PLAN 视图按它去重）
  "todos": [{
    "todo_id": "T03", "seq": 3, "rev": 5,
    "status": "pending | running | blocked | completed | failed | timeout | stopped | cancelled",
    "mode": "serial | explore",
    "title": "…", "content": "…",          // 任务正文
    "accept": ["验收标准"], "scope": "作用范围", "evidence": ["已知证据"], "skills": ["≤3"],
    "after": ["T01", "T02"],                // 这些 Todo 的结果被接受后才可执行（串行准入）
    "batch": null,                          // 一起派发的探索批次（可选）
    "attempts": [{
      "attempt": 1, "mode": "serial | explore",
      "caller": { "run_id": "…", "call_id": "step:7" },   // 串行：被哪个 PLAN Step 的交接准入
      "run_id": "…",                                      // 串行：执行它的子 context run
      "child_sid": "work-…", "dispatch": "dispatching | dispatched",   // 探索
      "status": "running | blocked | completed | failed | timeout | stopped",
      "started_at_ms": 0, "ended_at_ms": 0, "error": "…",
      "summary": "一句话结果", "artifacts": ["cyfile:… | 路径 (稳定副本)"],
      "report": { "session_id": "…", "run_id": "…(串行)", "report_id": "…" },
      "reason": "这次 attempt 的原因（重试理由）"
    }],
    "reviews": [{ "attempt": 1, "report_id": "…", "verdict": "accept | reject | need_evidence",
                  "reason": "…", "followups": ["T05"], "by": "…", "at_ms": 0 }],
    "refusals": [{ "at_ms": 0, "reason": "并发上限命中等" }],
    "retry": null,            // 待重试：下一次 attempt 的理由
    "cancel_reason": null,
    "created_at_ms": 0, "updated_at_ms": 0
  }]
}
```

- `status` 由记录推导：有 `cancel_reason` → `cancelled`；有待重试或尚无 attempt → `pending`；否则等于最新 attempt 的状态。
- 终态：`completed | failed | timeout | stopped | cancelled`。`blocked` 非终态，但不参与串行选择，需 PLAN 显式处理。
- Review 状态（视图中的 `review`）：`none`（无已结束 attempt）| `pending`（最新 attempt 已结束、未 Review）| `accepted | rejected | need_evidence`。
- 重试保留旧 attempt、旧报告与旧 Review；迟到的结果只写回它自己的 attempt，永远不覆盖新 attempt。

## 4. 串行执行（`next_behavior = do`）

Todo 执行器是声明了 `[entry] todo = true` 的 behavior（只能是 `create_sub_context`；Jarvis 的 `do`）。

1. **准入**（PLAN 的 Step 设 `next_behavior` 指向 Todo 执行器时，由宿主判断）：
   - 有已结束但未 Review 的结果 → 拒绝，要求先 Review（可修正）；
   - 有“就绪”的串行 Todo（第一个 `pending`、`mode = serial`、`after` 中的 Todo 全部被接受）→ 接受；
   - 没有就绪的串行 Todo、但有正在运行的探索（未收齐的子 Session）→ 视为等待（§6）；
   - 都没有 → 拒绝（可修正）。
   拒绝发生在 Step 生效之前：该 Step 成为纠错 Step，同一个 run 继续推理（计入连续错误），Turn 不失败，不产生 DO context。机制见 [llm_context 决策钩子](../../notepads/llm-context-plan-do-yield-todo.md)。
1b. **执行载体**（2026-10-23，集成 TODO §3.5）：Todo 可带 `runtime_id` 与 `workspace`（`inherit | none | <id>`；Workspace 决定 runtime，二者冲突在创建子 Session 时拒绝）。就绪的串行 Todo 指向本 Session 之外的 runtime 或 Workspace（`todos::needs_own_session`）时不绑定子 context：宿主在同一处把它派发为**自己的 Work Session**（入口 behavior 默认 `work_simple`，幂等 key 仍是 `todo:<id>:<attempt>`，attempt 记 `child_sid` 与解析出的 `runtime_id / workspace_id`），PLAN 的这次交接变成等待（§6）；子 Session 结束后在批次边界回写为该 attempt 的结果（`end_serial`），之后照常 Review。串行顺序、Review 与并发上限都不变，不把任务改成可并行的探索。
2. **绑定**：交接提交之前，就绪 Todo 开始一个 attempt，绑定 `(PLAN run_id, step:<step_index>)`，Todo 正文（任务、验收标准、范围、证据、所依据的已接受结果及其报告引用、重试理由）写入 `ChildCall.task`，绑定写入 `ChildCall.todo = {todo_id, attempt}`。同一 Step 的交接重做时得到同一个 attempt。
3. **执行**：DO 的 `<sub_task>` 就是这份正文（Todo 正文是显式输入，不依赖历史渲染）。DO 以 `<report end="true">` 结束。
4. **回写**：子 context 返回的同一次提交之前，attempt 写入结果：`ok → completed`、预算耗尽 → `timeout`、其它 → `failed`（`needs_user_input` 记为失败并写明问题）；summary 取报告首行，报告副本存入 `reports/<id>/submission.json`，`report_ref = (session, run_id, report_id)`。
5. **PLAN 收到的返回**：`process_result` 只有一句话结果与“用 `todo show <id> --report` 读全文”，不含报告全文；worklog 的 `outcome(process_done)` 同样只记这句话。

用 `call_behavior` 显式带 `task` 调用 DO 不经过 Todo 准入。

## 5. 探索执行（`todo explore` / `todo dispatch`）

派发一步完成“建 Todo（或取已有的 pending Todo）+ 创建子 Session + 写回绑定”：

1. 加锁；attempt 号 = 下一个（或沿用一个未完成的 `dispatching` attempt）；子 Session 的幂等 key = `todo:<todo_id>:<attempt>`（再经 `create_sub_session` 与父 sid 组合），其 sid 在创建前即可算出（`api::sub_session_id`）。
2. 写 attempt：`dispatching`、`child_sid`（提交点之前先落盘）。
3. 并发闸：父 Session 未结束的子 Session 数达到宿主 `--max-children`（工具进程从 `LIBOPENDAN_MAX_CHILDREN` 读）或 `session.policy.max_sub_sessions` → 撤回 attempt、记 `refusals`，Todo 保持 `pending`，命令返回 `dispatched: false` 与原因（退出码 3）。不静默排队。
4. `create_sub_session`：`class = work`、入口 behavior `explore`（可 `--behavior` 改）、Workspace 按 Todo 的 `workspace`（缺省 `none`）、runtime 按 Todo 的 `runtime_id`（缺省继承父 Session 冻结的 runtime）、`report = final`；objective 与首条消息都是 Todo 正文。
5. 写回 `dispatched`，解锁。

同一 shell 批次中的多次派发因 key 不同得到不同的 sid。崩溃恢复（宿主在 drive 开头与每个批次边界对账，也可重跑 `todo dispatch`）：有 sid 对应的子 Session → 绑定；没有 → 撤回（`refusals` 记“派发被中断”）。不重复派发。

探索子 Session 是独立的 Work Session：自己的 sid、目录、历史、预算、报告；深度 1（`max_session_depth = 2`），不读父列表、不写父列表。它的结果由父宿主在**批次边界**依据登记表的已提交状态回写（`outcome = succeeded → completed`；`failed → failed`，Turn 预算耗尽时 `timeout`；`stopped → stopped`；等输入 / 等决定 → `blocked`）。子 Session 的结束事件交给父 Session 时，回写已经发生（同一批次之前再对账一次）。

## 6. 等待 = 不做决策

PLAN 没有要做的事、只等探索返回时，回复**只有一个阶段 `<report>`**（无 actions、无 sendmsg、无 next_behavior）。宿主的判断：

- 顶层 context 且有未收齐的子 Session → 接受：run 结束，Turn 保持打开，`waiting_for = {kind: children, refs: [sid…]}`（worklog `outcome.kind = wait`）。任一子 Session 结束，其结束事件作为 `_child` 内部输入并入同一个 Turn 的下一个批次，PLAN（新 run，`inherit = recent_dialogue` 带着会话历史）带着更新后的视图继续——一个回来就可以 Review 一个。
- 没有可等的 → 不表态：阶段报告照常记录，context 继续推理（原有语义）；连续第三个只有阶段报告的回复被拒绝并说明原因（防空转）。
- `next_behavior = do` 而没有就绪 Todo、但有探索在跑 → 同样按等待处理。

不提供批量等待工具，PLAN 也不选“等哪一个”。父 Session 停止时停止级联到探索子 Session，等待中的 Turn 随之关闭；之后的对账把这些 attempt 记为 `stopped`。

## 7. PLAN 的视图与按需细节

每个顶层 context 的输入批次（开始、等待返回、子 context 返回）在列表修订号变化时自动附上视图（内建模板放在输入之后；自定义模板变量 `todos_text`、`session.todos`、`session.current_todo`）：

```xml
<todo_list rev="9">
<todo id="T01" mode="explore" status="completed" attempt="1" child_sid="work-…" report_ref="work-…#3f2…" review="pending">库存一致性 — 库存不足时扣减未回滚</todo>
<todo id="T03" mode="serial" status="pending" review="none" after="T01,T02">公开入口复核</todo>
</todo_list>
```

视图只含上述字段，不含 worklog、工具输出或推理过程。已展示的修订号记在 `state.todo_view_rev`（随批次 receipt 提交），同一修订不重复展示，历史消息稳定追加。子 Session 的结束事件也只说“Todo Tx attempt n 结束：一句话结果；用 `todo show Tx --report` 读报告”。

细节按需进入：`agent-session todo show <id> --report` 读最新 attempt 的完整报告（探索：子 Session 的 `final_report`；串行：`reports/<id>/submission.json`）；更深的证据 `agent-session read-session <child_sid> --report --worklog <n>`。

## 8. Review 与收敛

- `agent-session todo review <id> --accept|--reject|--need-evidence --reason <t> [--followup <id>]...`：Review 最新已结束的 attempt，记录读过的报告版本。同一 Review 重放是空操作；对已 Review 的 attempt 给出不同结论被拒绝（应重试或建后续 Todo）。
- 探索 attempt 的 Review 同时写子 Session 的 acceptance：accept → `accepted`，reject → `discarded`，need-evidence 不变。子 Session 没有输入队列也没有 artifact 时，由父的工具进程以驱动者身份持子 Session 的锁直接写（`api::decide_unqueued`，与 `ctl decide` 无队列路径相同）。
- 补充探索与重试落成新的 Todo 或新的 attempt（`todo retry <id> --reason`），并记录原因。
- **结束检查**：顶层的结束报告（`<report end="true">` / `report(is_end)`）在列表未收敛时被拒绝（可修正）。收敛 = 每个 Todo 要么 `cancelled`（带理由），要么最新 attempt 已结束且已 Review 为 accept / reject；`pending`、执行中、未 Review、`need_evidence` 都不算收敛。`cancel` 只用于未在执行的 Todo，不删除记录。

## 9. 命令

在 Session 的 shell 里（`agent-session` = `xagent`，Session 来自 `$OPENDAN_SESSION_ID`）：

```text
todo add <title> --content <t> [--accept <criterion>]... [--scope <t>] [--evidence <t>]... [--skill <s>]...
         [--after <todo_id>]... [--batch <b>] [--runtime <id>] [--workspace inherit|none|<id>]   # 串行 Todo；指到别的 runtime/Workspace 时在自己的 Work Session 里串行执行
todo explore <title> --content <t> [同上选项] [--behavior <b>] [--class <c>]   # 建 Todo 并派发探索
todo dispatch <todo_id> [--behavior <b>] [--class <c>]               # 派发已有的 pending Todo
todo list [--json]                                                    # 紧凑视图 / 完整 JSON
todo show <todo_id> [--report]
todo review <todo_id> (--accept | --reject | --need-evidence) --reason <t> [--followup <todo_id>]...
todo cancel <todo_id> --reason <t> | todo clean --reason <t> | todo retry <todo_id> --reason <t>
```

owner 侧只读：`xagent todo list <sid> [--json]`、`xagent todo show <todo_id> --session <sid> [--report]`。写命令只能在该 Session 自己的 run 里执行（退出码 2）。

退出码：0 成功；2 参数 / 规则错误（不存在、状态不允许、在 Session 外写）；3 派发被并发闸拒绝（Todo 保持 pending）；5 Review 已记录但子 Session 的 acceptance 未写入（重跑同一 Review）。

环境（由 Session 运行时注入或继承宿主进程）：`OPENDAN_SESSION_ID`、`OPENDAN_AGENT_DID`、`OPENDAN_AGENT_ROOT`（或 `OPENDAN_AGENT_STATE_URL`）、`LIBOPENDAN_WHO`（驱动者身份）、`LIBOPENDAN_QUEUE_DIR`（开发用文件队列）、`LIBOPENDAN_MAX_CHILDREN`（宿主的并发上限）。

## 10. 不支持的能力

- 不修改 Todo 正文（计划变化就建新 Todo 或重试）。
- 不删除 Todo（`cancel` / `clean` 只改状态并记理由）。
- 不做任意依赖图：只有 `--after`（“这些结果被接受后才执行”），足以表达“探索完成并 Review 后再串行验证或汇总”。
- 调度方式只有串行与探索；执行载体是本 Session 的子 context，或（Todo 指向别的 runtime/Workspace 时）独立 Work Session——不能把串行 Todo 改成并行探索来换机器。
- 已运行的 attempt 不接受改 runtime；换机器 = 新 attempt（`todo retry`）或新 Todo。
- 执行者与 owner 侧不写列表。

## 11. 验证

- 组件：`cargo test -p agent_tool --lib todo_tools`（ID、状态、重试、锁）。
- Runner / CLI：`cargo test -p libopendan --test plan_do_todo -- --test-threads=1`（并行探索、等待、Review、串行 DO、可修正拒绝、并发闸、失败重试、重放、工具进程与 runner 崩溃窗口、停止级联、结束检查、空转上限）。
- 真实模型：`test/plan_do_todo_dv/`（README）。
