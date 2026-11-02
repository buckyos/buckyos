# Agent Work Session TodoList

版本 4 · 2026-11-02 · 当前实现：`agent_tool::todo_tools`（数据模型、规则与存储）、`libopendan::todos` 与 `runner::todos`（调度、派发、回写、更新与控制的投递）、`runner::observe`（推理前的背景）、`xagent` 的 `todo` 命令（`agent-session todo …`）、Jarvis `plan_simple` / `work_simple` / `explore` behavior。实施记录见 [notepads/opendan-plan-simple-async-todo.md](../../notepads/opendan-plan-simple-async-todo.md) §8；之前的 PLAN/DO 执行器（`next_behavior = do`、Review、串行准入）已删除。

## 1. 定位

TodoList 是 plan session 的**持久决策依据**：要做什么、按什么顺序（`after`）、每个 Todo 的每次执行（attempt）在哪个 work session 里跑、它报告了什么。TodoList 不是通用任务系统：plan 管列表，宿主按列表调度，执行由独立的 work session 承担。

设计要点：

1. **plan 只读、只管列表**：plan session 读材料、增删改 Todo、干预运行中的 Todo、最后 `report(is_end=true)`；它不改文件、不跑构建、不产出交付物——干活一律是 Todo。
2. **满足条件立刻执行**：Todo 的前置都 `completed` 时，宿主立刻为它建 work session，不需要 plan 派发。调度点是列表的每次修改（plan 的 `todo` 命令）与每次子 session 结束的回写（批次边界、推理前），所以上游结束后下游在 plan 下一次推理之前就已开始。
3. **依赖缺省串行**：`todo add` 不带 `--after` 时依赖“上一个”（seq 最大的未取消 Todo）；`--after none` 无依赖、立刻可执行。串行是依赖链，不是全局锁：没有依赖关系的 Todo 并行，并发由宿主上限兜底。
4. **没有 Review**：执行成功即满足依赖；验证是一条 Todo（通常 `todo explore "verify …" --after <id>`），它的报告就是证据。plan 认为目标达成（或不可达成）就可以结束，宿主不按列表收敛拒绝结束。
5. **身份可追溯**：Todo → attempt → `child_sid` → 报告，全部持久化，不靠模型记忆或从自然语言里猜。

| 概念 | 归属 | 用途 |
| --- | --- | --- |
| Todo | plan session（`todos.json`） | plan 的工作项与决策记录 |
| attempt | plan 的一个 Sub Session（work session） | 一次执行：`work_simple`（`todo add`）或 `explore`（`todo explore`） |
| 系统级 Task | task-mgr | Turn 的 task、后台命令等；不是 Todo 的执行通道 |

## 2. 存储与写者

```text
<SessionDir>/.opendan_agent_session/
  todos.json      TodoList（schema opendan.todo_list/1，整文件原子替换）
  todos.lock      列表锁（flock；只用于排序写者，进程退出即释放）
```

- **单写者**：plan session 的宿主（runner）及其 run 的工具进程（`agent-session todo …`，同一 lease 下）。work session 永远不写 plan 的列表；owner 侧（`xagent todo list/show`）只读。
- 每次修改都是“加锁 → 读整个列表 → 修改 → 原子替换”。派发时锁一直持有到绑定写回，别人不会看到半完成的派发。
- 没有 `todos.json` 等同于空列表。

## 3. 数据模型

```jsonc
{
  "schema": "opendan.todo_list/1",
  "next_seq": 6,              // ID 计数器：单调、持久，cancel 后也不复用
  "rev": 17,                  // 列表修订号：每次修改 +1
  "todos": [{
    "todo_id": "T03", "seq": 3, "rev": 5,
    "status": "pending | running | blocked | completed | failed | timeout | stopped | cancelled",
    "mode": "serial | explore",
    "title": "…", "content": "…",          // 任务正文（可 update）
    "accept": ["验收标准"], "scope": "作用范围", "evidence": ["已知证据"], "skills": ["≤8"],
    "after": ["T01", "T02"],                // 这些 Todo completed 后才执行
    "blocked_by": ["T01"],                  // after 中失败 / 超时 / 停止 / 取消的 Todo：等 plan 决定
    "block_reason": "…",                    // 派发失败的原因（update / retry 清除）
    "batch": null,
    "runtime_id": null, "workspace": null,  // 执行位置：session-runtime 与 inherit | none | <id>，相互独立
    "behavior": null,                       // work session 的入口 behavior：首次派发时写入，重试沿用
    "attempts": [{
      "attempt": 1, "mode": "serial | explore",
      "child_sid": "work-…", "dispatch": "dispatching | dispatched",
      "runtime_id": "…", "workspace_id": "…", "baseline": "standard | extended", "behavior": "work_simple | explore | …",
      "status": "running | completed | failed | timeout | stopped",
      "started_at_ms": 0, "ended_at_ms": 0,
      "paused_at_ms": 0, "resumed_at_ms": 0,  // todo pause / resume
      "error": "…", "summary": "一句话结果", "artifacts": ["cyfile:… | 路径 (稳定副本)"],
      "report": { "session_id": "work-…", "report_id": "…" },
      "reason": "这次 attempt 的原因（重试理由）"
    }],
    "updates": [{ "at_ms": 0, "reason": "…", "fields": ["content", "accept"], "attempt": 1, "by": "…" }],
    "retry": null,            // 待重试：下一次 attempt 的理由
    "cancel_reason": null,
    "created_at_ms": 0, "updated_at_ms": 0
  }]
}
```

- `status` 由记录推导：有 `cancel_reason` → `cancelled`；有 attempt 在跑 → 它的状态；等待 attempt（尚无 attempt 或待重试）→ 有 `blocked_by` / `block_reason` 时 `blocked`，否则 `pending`；否则等于最新 attempt 的状态。
- 依赖满足 = `after` 中每个 Todo 的 `status == completed`（待重试的上游是 `pending`，下游继续等）。上游终态非成功 → 下游 `blocked`（`blocked_by`）；上游被 retry 或下游被改 `after` → 下游回到 `pending`。
- 重试保留旧 attempt 与旧报告；迟到的结果只写回它自己的 attempt，永远不覆盖新 attempt。
- 列表摘要（`TodoList::summary()`，如 `5 todos: 3 completed, 1 failed, 1 running (T05)`）在 plan 结束时写进 worklog（`control_applied{todo_summary}`）与结束 Turn 的 task 结果，不作门。

## 4. 调度

`todos::schedule` 是宿主的一个调度点：按创建顺序取所有就绪的 Todo（`TodoList::ready()`：`pending` 且依赖满足）依次派发，直到并发上限挡住一个为止。plan session 已暂停、正在停止或已结束时不派发。

调度点：

- 工具进程：`todo add / explore / update / retry / cancel` 完成后在同一进程里调度，命令输出带 `scheduled: [{result: dispatched | held | blocked, …}]`——一口气建 5 个 Todo 时，无依赖的那些在命令返回前就已开始。
- runner：每次把子 session 的已提交状态写回列表（drive 开头、每个输入批次边界、推理前的背景观察）之后——上游 `completed` 立刻派下游，并发闸释放时补派。

派发结果：

| 结果 | 情形 | 列表 |
| --- | --- | --- |
| `dispatched` | work session 已建好并绑定 | attempt `running` |
| `held` | 未结束的子 session 数达到宿主 `--max-children`（工具进程从 `LIBOPENDAN_MAX_CHILDREN` 读）或 `session.policy.max_sub_sessions` | 不记录，Todo 保持 `pending`，下一个调度点再试 |
| `blocked` | work session 不能按 Todo 的要求建立（runtime 不满足 cmd_skill、Workspace 不存在、嵌套过深等 `InvalidArgument`） | attempt 撤回，Todo `blocked` 并记 `block_reason`；plan `todo update` / `todo retry` 后重新可派 |

## 5. 派发

一次派发：

1. 加锁；attempt 号 = 下一个（或沿用一个未完成的 `dispatching` attempt）；work session 的幂等 key = `todo:<todo_id>:<attempt>`（再经 `create_sub_session` 与父 sid 组合），其 sid 在创建前即可算出（`api::sub_session_id`）。
2. 并发闸（只对尚不存在的 work session）：挡住 → `held`，不写任何东西。
3. 写 attempt：`dispatching`、`child_sid`（提交点之前先落盘）。
4. `create_sub_session`：`class = work`、入口 behavior `work_simple`（`todo add`）或 `explore`（`todo explore`，或 Todo 记录的 behavior）、Workspace 按 Todo 的 `workspace`（缺省：`todo add` 继承 plan 的，读写；`todo explore` 无）、runtime 按 Todo 的 `runtime_id`（缺省继承 plan 冻结的 runtime）、`report = final`、**带输入队列**（`SubSessionSpec.queued`：只接收 plan 的 Todo 事件与控制，不是 inbox，等待策略仍是 `finish_failed`）；objective 与首条消息都是 Todo 正文（含所依据的上游结果、报告引用与 artifacts）。
5. 写回 `dispatched`（解析出的 runtime / workspace / baseline / behavior），解锁。

崩溃恢复：drive 开头与每个批次边界对账——有 sid 对应的 work session → 绑定；没有 → 撤回为 `pending`，下一个调度点用同一个 key 重派，不重复建。

work session 是独立的 Work Session：自己的 sid、目录、历史、预算、报告；不读不写 plan 的列表。它的结果由 plan 的宿主依据登记表的已提交状态回写：`succeeded → completed`；`failed → failed`（Turn 预算耗尽时 `timeout`）；`stopped → stopped`。它需要回答时 attempt 仍是 `running`，plan 从输入里得知。

## 6. 运行中的 Todo：update / pause / resume / stop

plan **只从 Todo 的角度**影响运行中的工作，宿主把它传导到该 attempt 的 work session：

- `todo update <id> --reason <why> …`：正文类字段（title / content / accept / scope / evidence）在任何状态（cancelled 除外）都可改；`after / runtime / workspace / skills` 只在没有 attempt 在跑时可改。改动与原因记进 `updates`。有 attempt 在跑时，工具进程向它的 work session 队列投一条 Active 事件：`source = todo:<id>`、`event = updated`、`summary = "Todo Tx was updated (<字段>): <原因>"`、`data_ref = <plan sid>#<id>@<attempt>`、key `<plan sid>:<id>:<attempt>#updated@<rev>`（同一修订只投一次）。work session 取入时，宿主把 Todo 当前的完整任务正文接在 summary 后（`todos::expand_update`）；`work_simple` 的 `on_input` 有 `is_todo_update` 分支。work session 在更新取入之前不能以 `report(is_end=true)` 结束（被拒绝并说明更新在等它）。更新事件打开 work session 的下一个 Turn（Turn 已开着时并入该 Turn），这个 Turn 的 task 以更新为标题。
- `todo pause <id> --reason` / `todo resume <id> --reason`：attempt 记 `paused_at_ms / resumed_at_ms`，向 work session 发 `control(pause)` / `control(resume)`（语义见 [OpenDAN Agent Session架构设计](OpenDAN%20Agent%20Session架构设计.md)：暂停 = 不再推理，在飞行的调用返回、写入检查点后停住，Turn 保持打开；恢复 = 从检查点续推理）。对没有 attempt 在跑的 Todo 被拒绝。
- `todo stop <id> --reason`：向 work session 发 `control(stop)`；attempt 由回写记为 `stopped`，Todo 要 `todo retry` 才再派（不自动回 pending）。

## 7. plan 的输入与视图

plan session 是多 Turn 的 Work Session，每个输入打开一个 Turn：

| 输入 | 来源 |
| --- | --- |
| 首条：目标 + runtime 上下文 | 创建者 |
| work session 结束：`work session … of todo Tx (attempt n) finished (<outcome>): <一句话>` + 它之后的 Todo 的去向（已开始 / blocked）+ 读报告的方法 | `poll_children`（`is_child_end`） |
| work session 需要回答 / 决定 | `poll_children`（`is_child_needs_input`） |
| 用户消息（对话转发：`agent-session post <plan sid> --msg …`，只有 `--class plan_simple` 的 plan 有队列） | 队列（`is_user_msg`） |
| tick（工作在跑时每 `tick_secs`；`[session.plan_simple]` 为 600） | `timer:task_watch` |
| 纠正（回复既没结束也不等任何东西） | runner |

宿主在每个输入批次和每次推理前同步列表（并调度）。Todo 没有背景行，也没有自己的事件：它只是创建 work session 的辅助，状态变化要么来自 work session 的结束 / 需要输入事件（事件正文带 Todo、attempt 和后续 Todo 的去向），要么来自 plan 自己的命令（命令输出说明变化）。plan 的输入模板首行放 `{{ background_context }}`，背景只有半订阅状态与在跟任务的新输出（Session Input Protocol §6.6）。

细节按需：`agent-session todo show <id> --report`（最新 attempt 的 work session `final_report`）；`agent-session read-session <child_sid> --report --worklog <n>`；`agent-session todo list`（`<todo_list rev>` 紧凑视图）。

## 8. 结束由 plan 判断

- 只有 `report(is_end=true)` 结束 plan session；宿主不检查列表是否收敛，也不要求“听取”每个子 session——它们的结束本来就是输入。
- plan 结束时仍在跑的 attempt 记为 `stopped`（原因“the plan session ended / was stopped”），它们的 work session 被停掉（有队列的收到 `stop`；结束的父 session 的报告子 session 由宿主的 ChildDriver 停止）。
- `report(is_end=false)` 只记进度。

## 9. 命令

在 Session 的 shell 里（`agent-session` = `xagent`，Session 来自 `$OPENDAN_SESSION_ID`）：

```text
todo add <title> (--content <t> | --content - | --content-file <p>) [--accept <c>]... [--scope <t>] [--evidence <t>]... [--skill <s>]...
         [--after <todo_id>|none]... [--batch <b>] [--runtime <id>] [--workspace inherit|none|<id>] [--behavior <b>]
                                                   # 工作（work_simple）；缺省依赖上一个；就绪即自动开始
todo explore <title> --content <t> [同上选项]       # 只读调查（explore，无 workspace）；缺省无依赖
todo update <todo_id> --reason <t> [--title] [--content] [--accept <c>]... [--clear-accept] [--scope] [--evidence <t>]...
         [--after <id>|none]... [--runtime] [--workspace] [--skill <s>]...
todo pause|resume|stop <todo_id> --reason <t>
todo retry <todo_id> --reason <t> | todo cancel <todo_id> --reason <t> | todo clean --reason <t>
todo list [--json] | todo show <todo_id> [--report]
```

`todo dispatch`、`todo review`、`agent-session wait`、`create-worksession --wait` 都不存在（调用时返回说明）。owner 侧只读：`xagent todo list <sid> [--json]`、`xagent todo show <todo_id> --session <sid> [--report]`。写命令只能在该 Session 自己的 run 里执行（退出码 2）。

环境（由 Session 运行时注入或继承宿主进程）：`OPENDAN_SESSION_ID`、`OPENDAN_AGENT_DID`、`OPENDAN_AGENT_ROOT`（或 `OPENDAN_AGENT_STATE_URL`）、`LIBOPENDAN_WHO`（驱动者身份）、`LIBOPENDAN_QUEUE_DIR`（开发用文件队列）、`LIBOPENDAN_MAX_CHILDREN`（宿主的并发上限）。

## 10. 不支持的能力

- 不删除 Todo（`cancel` / `clean` 只改状态并记理由）。
- 不做任意依赖图：只有 `after`（“这些 Todo completed 后才执行”）；成环的 `after` 被拒绝。
- 已运行的 attempt 不接受改 runtime / workspace / skills / after；换机器 = `todo stop` + `todo update` + `todo retry`，或新 Todo。
- work session 的队列不是 inbox：不向它转发用户消息或任意文本（转给 plan）。
- 执行者与 owner 侧不写列表。

## 11. 验证

- 组件：`cargo test -p agent_tool --lib todo_tools`（缺省依赖链、`--after none`、就绪规则、上游失败 → blocked → retry 解锁、派发撤回 / 失败、update 字段规则与历史、pause 标记、摘要、锁）。
- plan session：`cargo test -p libopendan --test plan_simple_async -- --test-threads=1`（一口气建列表与依赖顺序、上游结束先于 plan 推理派下游、验证 Todo、只读 plan 与读写的 work session、上游失败 → blocked → retry、结束时停止在跑的工作、并发闸挡住与补派、用户消息 → `todo update` → work session 取入更新、`todo pause/resume/stop`、plan 暂停时输入积压与级联）；`--test plan_simple`（类与函数契约、注入前后崩溃恢复）。
- 真实模型：`test/plan_do_todo_dv/`（README）。
