# OpenDAN Todo 驱动的 PLAN DO 实施 TODO

日期：2026-10-16（2026-10-16 按实现 review 修订：双执行路径、PLAN 全局视图、不做决策即等待、工具归属）

状态：已实施（2026-10-16）。T01–T09 完成；Mock 测试与真实模型 DV 均已通过，记录见 §8。llm_context 层的修改单列在 [llm-context-plan-do-yield-todo.md](llm-context-plan-do-yield-todo.md)，先于本文实施。

目标：让 TodoList 成为 PLAN 的持久决策依据，并给 PLAN 一个**全局视角**：它看到的是每个 Todo 的目标、执行形态、状态、一句话结果和报告引用，而不是执行细节；据此决定下一步是串行做一个 Todo，还是并行派出一组探索，再在结果回来后 Review、补充、重做或结束。并行探索和报告回流属于本次必交能力，不能只恢复串行 `todo add → do` 就宣告完成。

依据：

- 用户要求：探索类 Work Session 应能创建可并行工作的 Todo；Review 时可以沿 Todo 查看对应 sub-session 的 report；这是 PLAN 高效决策的核心。PLAN 不应被执行细节淹没：细节按需进入，不默认进入上下文。
- [Agent TodoList](<../doc/opendan/Agent TodoList.md>)：Todo 的任务边界、持久记录和结果回流。本文扩展其中“第一个未完成 Todo”的串行假设，实施时同步更新原文。
- [xAgent](../doc/opendan/xAgent.md)：Sub Session（§4.10–4.16）、子 context 调度（§3）、状态回流、等待、报告与验收。
- [Agent Session Directory Protocol](<../doc/opendan/protocol/Session Directory Protocol.md>)：Session 状态、报告与恢复协议。

路径均相对 `buckyos/`。本 TODO 不要求引入新的任务服务或通用 DAG 调度器，优先组合现有 Todo 存储、子 context、Sub Session、ChildDriver、报告和状态提交机制。

## 1. 当前问题与复用基础

本次 PR #667 Review 的 AICC 日志 `/opt/buckyos/logs/aicc/aicc.531903.log` 中，第 109 行 PLAN 只写文本计划和 `<report>`，随后设置 `<next_behavior>do</next_behavior>`；第 113 行直接创建 DO 子上下文。后续第 146 → 149、307 → 311 行重复该路径。没有 Todo 创建调用，对应 Work Session 目录没有 `todos.json`。

提交 `08709a376` 的 OpenDAN 重构删除了 PLAN 必须 `todo add`、DO 读取 `session.current_todo` 和完成 Todo 的流程，替换成 report 与继承 steps 交接。当前部署与源码一致。

以下为 2026-10-16 对照源码核实的结果：

| 入口 | 已有能力与缺口 |
| --- | --- |
| `src/apps/jarvis_agent/agent/behaviors/plan.toml`、`do.toml` | 只靠自然语言 report 交接（plan.toml “DO only sees your steps and that report”）；DO 的 `[entry]` 是 `create_sub_context` + `inherit = steps`，system 要求读“inherited steps 里最后一个 report”。没有 Todo 创建、选择、Review 和完成约束。 |
| `src/frame/agent_tool/src/todo_tools.rs` | 有 session 级 `todos.json` 与 add/current/list/finish/show/clean。**未接入生产**：`TodoTool` 只在 `agent_tool_cli_dev` 注册；Session 的 PATH 只有 `<sid>/.runtime/bin` 与 `<agent_root>/tools` 两层，镜像里的 `agent_tool` 二进制在 `/opt/buckyos/bin/opendan`，不在其中。缺口：`current` 取整个列表第一个非终态项（含已派给子 Session 的项）；`is_terminal` 不含 `blocked`，一个 blocked 项会永久占住 current；`finish` 无状态迁移保护，终态可被覆写；`next_todo_id = len()+1`，`clean` 后会生成与存活记录相同的 ID；无 `cancelled`/`stopped`。报错文案与文档用 `WORK_SESSION_ID`/`AGENT_ROOTFS`，运行时注入的是 `OPENDAN_SESSION_ID`/`OPENDAN_AGENT_ROOT`。 |
| `src/frame/lib_opendan/src/runner/outcome.rs` `classify_done` | `next_behavior=do` 只查嵌套深度就构造 `ChildCall{task: None}` 进入子 context，无 Todo 准入。`refuse_handover` 对父 run 是 `recoverable: false` 并把 Turn 标 Failed，**不能**用作“可修正错误”。无队列 Work 的 run 若既无 `next_behavior` 也无结束 report，以 `missing_end_report` 不可恢复失败；`hold_for_children` 只在试图成功结束时挂起等子 Session（Turn 保持打开，`waiting_for = Children`）。本次把这条“无决策”分支改为：有未结束的探索子 Session 时等待，否则仍报错。 |
| `src/frame/lib_opendan/src/runner/assembler.rs` | `session.current_todo` 固定为 null。`<sub_task>` 块已把 `ChildCall.task` 渲染给子 context（无 task 时写 “Continue the work handed over…”），是串行 Todo 正文的现成载体。 |
| `src/frame/llm_context/src/step_record.rs` | `render_inherited_step_record` 只取 `thought` 或 `assistant_text`，遗漏 `self_report`。现有测试 `inherited_self_report_does_not_render_as_action` 断言的正是该行为，修复时需有意改动。 |
| `src/frame/lib_opendan/src/api.rs` 的 `create_sub_session` | 已有独立 sid、SessionDir、origin、幂等创建（key 优先 `(parent, run_id, call_id)`，也接受显式 `sub.key`）、`max_sub_sessions`（默认 4）与 `max_session_depth`（默认 2）、模型和 runtime 配置继承。同一次 shell 调用若不给显式 key，多个子任务会合并成同一个 sid。 |
| `src/frame/lib_opendan/src/bin/xagent.rs` `agent-session create-worksession` | 已有 `--key`、`--objective`、`--msg`、`--class`、`--behavior`、`--workspace none`、`--report final|progress|none`、`--wait`。是“建 Todo 即派子 Session”的直接落点。 |
| `src/frame/lib_opendan/src/host/mod.rs`、`runner/children.rs`、`runner/reports.rs` | 已有真实子会话并发推进（`ChildDriver`，`max_child_concurrency` 默认 4，`xagent --max-children`）、状态观察（`poll_children` 以 `child:<sid>:<attention>@<rev>` 去重）、子结束事件作为 `_child` 内部输入进入父的下一批次、结束时拒绝未收齐子 Session 的最终报告（`unsettled_children`）、报告的稳定 `ReportSubmission.id`、`read-session <sid> --report`。 |
| `ctl decide` | 有队列路径不依赖 artifact；无队列路径（`decide_without_queue`）无 `artifact_id` 直接报错。默认子 Session 不带 `--interactive`、无队列，父无法对探索子 Session 接受或拒绝。 |
| Workspace | `workspace:<id>` 是每 Session 独占租约；`ReadOnly` 绑定在本地 shell 被拒绝。探索子 Session 须 `--workspace none`。 |

`delegateTask` 的占位后端不作为本次真实子会话派发路径。

## 2. 执行与决策契约

### 2.0 PLAN 的全局视角

- PLAN 在每个决策边界看到一份紧凑的 TodoList 视图：每项的 id、标题、执行形态（串行 / 探索）、状态、当前 attempt、`child_sid`、一句话 summary、`report_ref`、Review 状态。不包含子会话的 worklog、工具输出或推理过程。
- 细节按需进入：`todo show <id> --report` 读取绑定执行的完整报告；更深的证据经 `agent-session read-session <child_sid>` 由 PLAN 主动读取。宿主不把这些自动注入。
- 探索子 Session 默认 `--report final`：只有“需要关注”和“结束”进入 PLAN 的输入，进度不进。
- PLAN 用这份视图做三类事：串行做一个 Todo（`next_behavior = do`）、并行派一组探索（派发工具）、Review 并建后续 Todo。等待不是一个决定：探索都在跑、没有可进 DO 的 Todo 时，PLAN 不做决策就是等待。

### 2.1 Todo 与执行形态

- TodoList 归父 Work Session 所有，PLAN 管理计划与执行选择。
- 一个 Todo 的每次执行尝试是下列两种形态之一，Review 入口统一在 Todo 记录上：
  - **串行执行**：`next_behavior = do`，在本 Session 内以子 context 执行，父 run 挂起。绑定记录 `(run_id, call_id)` 与 `process_done` 结果；Todo 正文经 `ChildCall.task` 进入 DO 的 `<sub_task>`。
  - **探索执行**：由派发工具创建独立子 Work Session（自己的 sid、目录、历史、预算、报告），可以多个并行。绑定记录 `child_sid`。
- 双向引用必须持久化：父 sid、todo_id、attempt、执行形态、`run_id`/`call_id` 或 `child_sid`。执行身份不能靠模型记住或从自然语言报告猜测。
- 串行 DO 读取的是宿主明确绑定的 Todo 快照，不自行从列表挑选；探索子 Session 的任务由创建时的 objective 与首条消息承载，不读父列表。
- 子 Session 结束后，由父宿主在批次边界依据登记表的已提交状态回写父 Todo；子不写父 `todos.json`。父列表只有一个写者：父 Session 的宿主及其工具层（同一 lease 下）。
- 通用 `llm_context` 仍只负责推理与 context 切换；Todo 准入、派发、回写放在 Work Session 宿主（libopendan）及 `agent-session` 工具层。`agent_tool::todo_tools` 退为数据模型与存储库。

### 2.2 串行与并行

- 默认串行：PLAN 设 `next_behavior = do`，宿主在交接时做 Todo 准入：选出**第一个未终态且未绑定探索执行的 Todo**，把正文、验收标准、约束填入 `ChildCall.task`，标记 running 与 attempt，再进入 DO。没有可执行 Todo 时：若有探索子 Session 在跑，按下文“等待”处理；若没有，返回**可修正**错误并保持 PLAN 的 run 继续（不能复用 `refuse_handover`，它会结束 Turn）。
- 并行由 PLAN 显式选择一组相互独立的探索 Todo。派发工具一步完成“建 Todo + 创建子 Session + 写回绑定”：`agent-session todo explore --title … --content … [--accept …] [--skill …]`，内部调用 `create_sub_session`，key 为 `(parent_sid, todo_id, attempt)`，默认 `--workspace none`、`--report final`、入口 behavior 为探索类 behavior（见 §2.5）。也允许对已有 pending Todo 调用 `todo dispatch <id>`。
- 一个批次先派发全部成员，再等待；同一 shell 批次内的多次派发因显式 key 不会合并 sid。
- **等待 = 不做决策**。不提供批量等待工具，PLAN 也不需要选“等哪一个”。规则：PLAN 的 run 以阶段 `<report>`（写明在等什么、依据什么）结束，没有 actions、没有 `next_behavior`、不是结束报告；此时若存在未结束的探索子 Session，宿主让 run 结束、Turn 保持打开、`waiting_for = Children{sids}`（复用 `hold_for_children` 的形态）。任一探索子 Session 到终态，其结束事件按现有 `_child` 内部输入进入同一 Turn 的下一批次，PLAN 带着更新后的视图继续决策；一个回来就可以 Review 一个，不必等全部。没有探索在跑时的无决策仍按现有 `missing_end_report` 处理。实现时核对 llm_context 的“空决策纠错”不会拒绝“只有阶段 report”的步骤。这是对 `classify_done` 无 report 分支的 Session 语义改动，需同步 xAgent 与 Session Directory Protocol。
- 并发限制：`session.policy.max_sub_sessions` 与宿主 `max_child_concurrency` 两道闸。超限时派发工具返回可观察、可修正的错误，PLAN 可以等待后重试；不能静默排队后宣称并行。
- 探索子 Session 使用独立可写 SessionDir；只读输入靠复制快照或文件权限，不用 `ReadOnly` workspace 绑定。保留现有 `workspace:<id>` 独占租约语义。
- 批次外的后续 Todo 由 PLAN 在 Review 后创建。首版无需任意依赖图，但必须能表达“两个探索完成并被 Review 后，才串行执行验证或汇总”。

### 2.3 Todo 的最小持久信息

字段名可按已有协议收敛，下列语义不能省略：

| 信息 | 要求 |
| --- | --- |
| 身份与排序 | 稳定且不复用的 todo_id（单调计数器持久化，不按列表长度生成）、父 sid、创建顺序、修订号。 |
| 任务正文 | title、content、验收标准、作用范围、已知证据、必要 skills；skills 沿用已有数量限制。 |
| 执行形态与派发 | `serial` 或 `explore`；被选择的 Todo、所属批次、派发状态、并发上限命中记录。 |
| 执行尝试 | attempt、形态、`run_id`/`call_id` 或 `child_sid`、开始和结束时间、结果状态、失败原因；重试保留旧尝试及报告。 |
| 报告引用 | 一句话 summary、稳定 `report_ref`（探索：`(child_sid, report_id)`；串行：`(run_id, report_id)`）、artifact refs。不能只有不可追溯的摘要。 |
| PLAN Review | 已读报告版本、接受 / 拒绝 / 需补证据、理由、后继 Todo 引用。执行成功与 PLAN 接受分别记录。 |

状态与映射：

- Todo 状态：`pending | running | completed | failed | timeout | blocked | stopped | cancelled`。终态为 `completed | failed | timeout | stopped | cancelled`；`blocked` 非终态但不参与串行 current 选择，需 PLAN 显式处理。
- 探索执行由子 Session 登记表状态推导：`outcome=succeeded → completed`，`failed → failed`（Turn 为 `budget_exhausted` 时记 `timeout`），`stopped → stopped`，`waiting_for = input/decision → blocked`。串行执行由 `process_done` 的 `child_status`（ok / failed）推导。
- 允许少量“派发中”状态用于崩溃恢复，但不维护与 Session 终态各自独立的事实。
- `cancelled` 只对未派发的 Todo 可用，并记录理由；`clean` 的语义改为批量 cancel pending，不删除记录、不复用 ID。

### 2.4 PLAN 输入与报告 Review

- PLAN 开始及等待返回、子状态变化后的决策边界，自动获得 §2.0 的紧凑视图，经 `assembler` 的宿主变量装配（替代固定为 null 的 `session.current_todo`）。视图按 Todo 修订号与子状态 `rev` 去重，历史消息稳定追加，不为刷新列表重写上下文。
- 新完成 Todo 的报告必须可直接读取：视图里给 summary 与 `report_ref`；PLAN 用 `todo show <id> --report` 读全文。不能只注入一句“子任务成功”就默认接受。
- PLAN 的下一次决策说明依据了哪些 Todo 报告、哪些结论已验证、哪些存在冲突或缺口。补充探索与重试必须落成新的 Todo 或新的 attempt，并记录原因。
- 子会话 outcome=succeeded 只证明本次执行结束；PLAN 必须 Review 后才能把结果作为后续任务依据。
- Review 的持久化：`agent-session todo review <id> --accept|--reject|--need-evidence --reason …`，写 Todo 记录；对探索子 Session 同时写 Session acceptance。无队列且无 artifact 的子 Session 需补宿主侧写入路径（父持子 lease 或经登记表写 acceptance），不能只在提示词里要求 `ctl decide`。
- Work Session 最终成功前，必须确认目标达成、所需 Todo 已收敛且被接受。未派发的无效 Todo 可以 cancel 并记录理由；不能抹掉已经执行过的分支及报告。

### 2.5 完整任务交接与恢复

- 串行 DO 的初始输入经 `<sub_task>` 包含完整 Todo 正文、验收标准、约束、必要证据和绑定身份（todo_id、attempt）。DO 的 system 改为“执行 `<sub_task>` 中绑定的 Todo”，不再依赖“inherited steps 里最后一个 report”。
- 探索子 Session 的 objective 与首条消息包含同样内容。入口 behavior 另建（如 `explore`，参照 `work_simple` 的直接执行形态），DO 不作为子 Session 入口：子 Session 没有继承历史，DO 的 `create_sub_context` 入口模式也不适合作为 Session 入口。
- 修复继承历史遗漏 `self_report`，并覆盖普通 behavior report 交接。Todo 正文使用显式输入承载，历史渲染不作为唯一载体。
- 派发幂等身份 `(parent_sid, todo_id, attempt)` 经 `create-worksession --key` 的既有分支。
- 提交次序：写 Todo 派发记录（dispatching）→ 创建子 Session → 写回绑定（running）→ 子结束后由父宿主回写结果。崩溃后按登记表对账：有 key 对应的子 Session 则补绑定，不重复派发；旧 attempt 的迟到结果不覆盖新 attempt。
- 父停止时复用 stop 级联；重启后父处于 `waiting_for = Children` 的 Turn 由子结束事件唤醒，接续已有子会话与 Review。禁止通过伪造成功或清空列表解除等待。

## 3. 实施清单

- [x] **T01 协议与存储**：收敛 Todo 身份、两种执行形态的绑定、批次、结果引用、状态与映射、Review 语义；持久单调 ID；状态迁移保护；父列表单写者和恢复策略；对齐环境变量名。
- [x] **T02 工具接线**：命令面落在 `agent-session todo …`（add / list / show [--report] / explore / dispatch / review / cancel / retry），`agent_tool::todo_tools` 退为库；同步 help、schema、模型可见命令说明；确认 Session PATH 可见。
- [x] **T03 准入、派发与等待**：`next_behavior=do` 的 Todo 准入与 `ChildCall.task` 填充（新增可修正错误路径）；探索派发复用 `create_sub_session` 与 ChildDriver；“无决策即等待”：`classify_done` 无 report 分支在有未结束探索子 Session 时转为 `waiting_for = Children`；并发限制、幂等绑定与崩溃恢复。
- [x] **T04 结果与 Review**：父宿主在批次边界回写子结果；`report_ref` 与报告读取；`todo review` 持久化并补无队列无 artifact 的 acceptance 写入；最终结束前的收敛检查。
- [x] **T05 上下文与提示词**：紧凑 TodoList 视图装配；PLAN 提示词恢复“先建 Todo，再决定串行或并行”；DO 改为执行 `<sub_task>` 绑定 Todo；新建探索入口 behavior；修复 inherited report 丢失；同步模板与部署打包来源（`src/apps/jarvis_agent/agent/behaviors/`）。
- [x] **T06 可观察性**：`todo list --json`、Todo → attempt → (`child_sid` | `run_id`) → report/history 查询链；CLI 可检查 JSON 与可读摘要；WebUI 可后续完善。
- [x] **T07 自动化验证**：实现第 6 节的状态、恢复、交接与并发测试，覆盖实际进程推进和持久结果。
- [x] **T08 真实模型验收**：创建第 4 节固定测试任务与第 5 节入口，真实运行 `xagent`，保存全过程证据和人工 Review 记录。
- [x] **T09 文档对齐**：更新 Agent TodoList（current 定义、状态、CLI、环境变量）、xAgent 与 Session Directory Protocol（Todo 与子 Session / 子 context 的关系、无决策即等待的 Turn 语义）、工具手册、Session 协议及示例；移除“report 即任务”的矛盾说明。

`agent_tool` 不得反向依赖 `libopendan`。协议改动需核对 Fs/kRPC AgentState、CLI、Session schema、报告读者与相关 WebUI 类型；不为旧开发版本新增兼容分支。

## 4. 典型真实任务

### 4.1 任务场景

创建一个固定、无外部网络数据依赖的小型 Python 库存预留服务 Review fixture。实际推理使用真实 LLM，文件读取、复现、Todo 调度与报告提交均走真实工具和 `xagent`。

fixture 使用 Python 标准库，至少包含：

| 文件 | 业务契约与预置问题 |
| --- | --- |
| `README.md` | 明确库存不足不得修改库存；幂等键应在租户内生效；相同租户相同请求不可重复扣减。 |
| `inventory.py` | `reserve` 先扣减，再判断不足并返回失败，未回滚；例如库存 5、请求 8 后错误地留下 -3。 |
| `idempotency.py` | 缓存仅以 request_id 为键，遗漏 tenant_id；两个租户使用相同请求 ID 时错误复用结果。 |
| `service.py` | 将两个模块接到同一预留接口，包含调用顺序和可用于复现的公开入口。 |
| `tests/test_happy_path.py` | 只覆盖正常预留和同租户重试，能够通过，避免把现成红灯直接当最终发现。 |

fixture 的已知答案与自动验收代码放在测试驱动侧，不放进模型可读任务目录。每次运行生成新副本并记录内容 hash，所有子任务基于同一快照。源文件只读，复现脚本和报告写各自 SessionDir。

### 4.2 给父 PLAN 的完整任务

```text
Review 本次运行提供的库存预留服务快照，判断实现是否符合 README 中的契约。
不要修改被审查源码。所有新增复现脚本和报告写入各自会话工作目录。

先建立可查询的 TodoList，把两个独立探索项作为并行探索派出：
1. 检查库存数量校验、失败路径及状态一致性。
2. 检查幂等请求、租户隔离及接口调用链。
两项各自使用独立的探索子 Work Session。每个 Todo 写清范围、验收标准与证据要求，
只执行其指定范围，发现范围外线索时交回 PLAN，不自行扩张任务。

每个探索报告需要给出文件和行号、触发条件、最小复现、实际与预期结果，
区分已证实问题与尚未证实的猜测，并登记复现脚本及输出。

探索派出后不要空转：没有可做的事就只写阶段报告说明在等什么，探索返回会唤醒你。
读取并 Review 两份报告，再据报告创建一个后续验证 Todo 并串行执行：
从服务公开入口复核候选问题，补一个不应被误报的正常路径，判断发现是否成立。
如果证据不足或分支失败，先安排补充探索，不得直接宣布全部完成。

验证返回后，写最终 review.md：列出已证实发现、被排除的猜测、验证范围，
并为每项发现给出 Todo ID、执行引用（子 Session ID 或 run ID）、原始报告与复现证据引用。
最后保存 decisions.md，记录你基于哪些子报告接受、否定或追加了什么工作。
```

任务实际输入中必须提供 fixture 的可访问路径，不能让模型猜目录。测试驱动不得替模型预建 Todo、填入报告或在模型之外执行最终 Review 决策。

### 4.3 预期执行轨迹

```text
父 PLAN 建立 Todo A、B 并作为探索派出（两个独立子 Session），以阶段报告结束本轮 → Turn 等待子 Session
  ├─ A → 探索子 Session A → 库存一致性报告
  └─ B → 探索子 Session B → 幂等与租户隔离报告
父 PLAN 被 A/B 结束事件唤醒，读取两份报告并记录 Review（可先到先 Review）
  └─ C → next_behavior = do（串行子 context）→ 公开接口复核与正常路径验证报告
父 PLAN Review C → 最终 review.md 与 decisions.md
```

A、B、C 是验收中的逻辑名称，不硬编码运行时生成的 Todo ID。该轨迹同时覆盖探索与串行两种执行形态。若模型发现合理的新缺口，可以增加 Todo，但不得省略并行探索和基于报告的后续验证。

### 4.4 并行和结果证据

- A、B 的 child_sid 必须不同，各自有真实 LLM 请求、工具调用、报告和执行历史。
- 在第一个探索结束前，另一个探索必须已开始真实推理及工具执行；以宿主/子会话记录的时间戳证明执行区间重叠。两个 `pending` 条目或同一 shell 中的后台命令不算两个并行 Todo。
- 测试可提供有截止时间的双分支 barrier helper，要求两个探索先各自登记开始再共同放行，稳定暴露串行派发问题。barrier 只协调测试，不生成分析结果，不替代真实请求和工具记录。
- 父 PLAN 在 A、B 结束前处于 `waiting_for = Children`，没有在此期间派出 C 或进入 DO；唤醒它的是子结束事件，不是轮询工具。
- C 的创建或放行必须晚于 A、B 报告提交及父 PLAN Review。C 以串行形态执行，其 `<sub_task>` 包含被 Review 的报告引用和待验证问题，不能只是重发最初总目标。
- 每个分支包含一个仅出现在 Todo 正文中的具体验收要求；检查探索子 Session 的首条输入、DO 的 `<sub_task>` 和产物都保留该要求，防止再次发生“只有 intent、正文丢失”。
- 父 PLAN 的上下文不包含子 Session 的 worklog 或工具输出；全文报告只在 PLAN 调用 `todo show --report` 后出现。
- 最终 Review 应证实两个预置缺陷，说明正常路径的结果，且能从最终发现回到对应 Todo、执行引用、子报告和实际复现输出。
- 另提供失败变体：让一个探索的必要输入缺失或工具明确失败。PLAN 必须识别该分支失败并补充安排或如实失败，不能接受空报告或把列表直接标成全部完成。

## 5. xagent 运行与观察入口

### 5.1 必须交付的文件

新增 `test/plan_do_todo_dv/`，至少包含：

- `README.md`：前置条件、真实 provider 配置、完整命令、退出码与证据位置。
- `task.md`、fixture 源文件及其正常路径测试。
- `main.py`：prepare、run、verify 三个阶段；run 必须启动真实 `xagent`，保留 stdout、stderr、退出码与 Session ID。
- 验收脚本：从实际 Todo、Session、worklog、报告与产物验证第 4、6 节，不接受仅匹配最终报告中的“成功”字样。
- 与仓库 `test/run.py` 的发现和执行方式兼容（目录下 `main.py`）。真实模型用例显式启用，缺少 provider 或凭据时明确失败或标记未运行，不能静默切成 mock 后宣称 DV 通过。

prepare 阶段创建独立 AgentRoot、SessionDir 和文件队列目录，使用待验收的 Jarvis PLAN/DO/探索模板与真实 Todo 工具。允许配置模型和 fixture 路径，不允许换成绕过正式实现的测试专用派发提示词。保留 behavior 冻结快照与配置版本，避免运行旧部署模板。

Todo 命令统一由 `agent-session`（即 `xagent` 的 Session 内子命令）提供，prepare 记录对应 `xagent` 产物路径与 hash 即可。

### 5.2 真实命令约束

以下 `xagent` 参数已有实现；fixture 与隔离配置由新增 prepare 阶段生成。验收 README 必须把这些步骤落实成可复制的一次完整运行，不留未解释的占位符：

```bash
# 在 buckyos/ 下构建待验收的二进制。
cargo build --manifest-path src/Cargo.toml -p libopendan --bin xagent

# prepare 生成这些目录/文件，并告知构建产物 xagent 的实际绝对路径。
export OPENDAN_AGENT_ROOT="$PLAN_DO_CASE/agent"
export OPENDAN_AGENT_DID="did:bns:jarvis.todo-test"
export LIBOPENDAN_WHO="app:xagent-test@local"
export LIBOPENDAN_QUEUE_DIR="$PLAN_DO_CASE/kmsg"

"$PLAN_DO_XAGENT" new --class work --behavior plan \
  --objective "$(cat "$PLAN_DO_CASE/task.txt")" \
  --parent "$PLAN_DO_CASE/sessions" \
  --llm-context "@$PLAN_DO_CASE/llm-context.json" \
  --no-run > "$PLAN_DO_CASE/create.json"

PLAN_DO_SID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["session_id"])' "$PLAN_DO_CASE/create.json")"

"$PLAN_DO_XAGENT" run "$PLAN_DO_SID" --until finished --max-children 4 \
  > "$PLAN_DO_CASE/drive.json" 2> "$PLAN_DO_CASE/drive.stderr.log"

"$PLAN_DO_XAGENT" status "$PLAN_DO_SID" --worklog 200 --report --run --events \
  > "$PLAN_DO_CASE/parent-status.json"
"$PLAN_DO_XAGENT" list > "$PLAN_DO_CASE/sessions.json"
```

`PLAN_DO_CASE` 必须指向本次新建的测试目录，`PLAN_DO_XAGENT` 指向刚构建的产物。自动入口记录 `run` 的退出码，并在失败时同样采集状态；退出 0 不替代 Todo 与报告断言。

并发与隔离配置：

- 父会话不绑定 workspace；探索子 Session 由派发工具以 `--workspace none` 创建，各用自己的 SessionDir。
- `session.policy.max_sub_sessions`（默认 4）与 `xagent run --max-children`（默认 4）都要 ≥ 2；V04 的超限测试分别压这两道闸。
- `max_session_depth` 默认 2：探索子 Session 深度 1，其内不再派子 Session。

真实 provider 的配置要求：

- 使用可用的 AICC，或配置了真实端点和凭据环境变量的 provider；凭据不写入报告。
- `--llm-context` 在 `new` 时落盘，`run` 不接受该配置覆盖；子 Session 继承父配置。
- 核对 behavior 的模型覆盖。Jarvis PLAN/DO 默认选择 `llm.plan/llm.code`，探索 behavior 的 `preferred` 一并核对；直连 provider 时须在隔离配置中映射成实际可用模型，不能只改基础 `model` 就认为已生效。
- 同时保留使 Todo、子会话工具和延迟等待可用的配置；不能用一个过小的 provider JSON 覆盖掉正式运行所需能力。记录最终父子配置以便核对。
- 提供合理的 token、工具次数和测试总时长限制；达到限制按失败记录。

### 5.3 运行时观察与 Review

在第二个终端中，沿用相同 AgentRoot、身份和队列配置，用 `xagent list --active` 观察活动会话，用 `xagent status <父sid> --worklog 200 --events` 观察父决策。由 `xagent list` 中的 `origin.parent_session` 找到子 sid，对每个子运行：

```bash
"$PLAN_DO_XAGENT" status "$PLAN_DO_CHILD_SID" --report --worklog 200 --run
```

新增的 Todo 查询接口（`xagent todo list <父sid> --json`，或等价的 owner 侧入口）必须直接展示 Todo → attempt → (`child_sid` | `run_id`) → `report_ref`。验收脚本保存完整 worklog 和报告，不能因 `--worklog 200` 的展示上限丢掉关键早期证据。

一次运行的证据目录至少保存：

```text
create.json / drive.json / drive.stderr.log
config/                     脱敏配置、behavior 冻结版本、fixture hash
parent/                     Todo 快照、完整 worklog、状态、PLAN 决策、每个决策边界的 TodoList 视图
children/<child_sid>/       Todo 绑定、状态、完整 worklog、报告、复现产物
serial/<run_id>/            串行 Todo 的 sub_task 输入、process_done 结果、报告
timeline.json               真实派发、推理、工具、等待、报告与 Review 时间线
review.md / decisions.md    父会话最终产物的导出副本
verification.json           逐条断言与失败原因
```

## 6. 验收矩阵

| 编号 | 场景 | 必须满足的结果 |
| --- | --- | --- |
| V01 | 无 current Todo 时进入 DO | 有探索在跑：按等待处理；没有：宿主返回可修正错误，PLAN 的 run 继续；两种情况 Turn 都不失败，且没有无绑定的 DO 子 context。 |
| V02 | 单个串行 Todo | 先持久化 Todo，再绑定 `(run_id, call_id)`；DO 的 `<sub_task>` 含完整正文；完成后可读取报告，PLAN 有 Review 记录。 |
| V03 | 两个并行探索 Todo | 不同子 sid、独立目录和绑定、真实执行区间重叠；串行 current 不选中已派发的探索 Todo。 |
| V04 | 并发上限与 Workspace | `max_sub_sessions` 与 `--max-children` 超限各自可观察且可恢复；共享写 Workspace 不绕锁；`--workspace none` 的探索能真正并行。 |
| V05 | 完整输入交接 | Todo 正文、验收标准、约束与 report 不丢失；只在正文出现的测试要求被探索子 Session 和串行 DO 执行。 |
| V06 | 等待与报告驱动下一步 | PLAN 无决策时 Turn 进入 `waiting_for = Children` 而不是失败；子结束事件唤醒 PLAN；消费 A/B 报告并记录判断后才进入 DO；C 的 `<sub_task>` 使用具体证据而非仅重复总目标。 |
| V07 | 分支失败或证据冲突 | 不误标成功；PLAN 补充探索、拒绝结果或如实结束，旧报告可追溯。 |
| V08 | 派发与结果幂等 | 重放命令/事件不多建子会话，不重复 Review；同一 shell 批次的不同 Todo 不合并 sid。 |
| V09 | 崩溃恢复 | 在派发记录、子 Session 创建、绑定写回、父等待中、子完成、结果回写各边界中断后可接续，不漏结果、不重复执行、不重复唤醒。 |
| V10 | 重试与清理 | 新 attempt 保留旧报告；迟到结果不覆盖新结果；ID 不复用；cancel 只作用于未派发项且不删除历史。 |
| V11 | 停止与最终结束 | 父停止级联到探索子 Session 并关闭等待中的 Turn；存在必要且未收敛或未 Review 的 Todo 时不能成功结束。 |
| V12 | PLAN 视图边界 | 每个决策边界的 TodoList 视图只含 §2.0 字段；父上下文不含子 worklog 或工具输出；全文报告只在 `todo show --report` 后出现。 |
| V13 | 真实 CLI 与模型 | 第 4 节任务和失败变体通过 `xagent` 运行；证据包含真实请求与工具结果，CLI 可逐项查看报告。 |

验证分层：组件测试覆盖 ID、状态、绑定与写入原子性；Runner/CLI 集成测试覆盖准入、派发、等待、report、故障恢复和权限身份（可在 `src/frame/lib_opendan/tests/` 现有 crash / report / xagent_cli 用例上扩展）；真实模型 DV 验证模型是否实际使用 TodoList 分解、并行、Review 并决策。Mock 测试与真实 DV 分开记结果。

## 7. 完成条件

- [x] 实现清单 T01–T09 完成，验收矩阵逐项有结果和证据路径（§8.3）。
- [x] 第 4 节任务可按 README 从干净目录通过 `xagent` 重跑，至少保存一次正常案例和一次失败处理案例的真实运行记录（§8.5）。
- [x] Review 者可从父 Todo 列表进入每个执行的报告及复现证据，并能看出 PLAN 为什么选择串行或并行、为什么创建下一项 Todo（`xagent todo list <sid> --json`、`parent/views.json`、`decisions.md`）。
- [x] 明确记录真实并发是否发生、模型与预算、各 Todo 用时、失败或重试原因；不以最终生成一份 review.md 作为唯一成功标准（§8.5）。
- [x] 原有普通 Work、子会话、behavior context 切换、报告与停止/恢复测试没有回归（§8.4）。
- [x] 回写本文实施记录，列出变更入口、测试命令、真实运行 Session ID、证据目录和剩余限制（§8）。

## 8. 实施记录（2026-10-16）

### 8.1 落点

| 范围 | 落点 |
| --- | --- |
| llm_context（先行，单列） | [llm-context-plan-do-yield-todo.md](llm-context-plan-do-yield-todo.md)：继承 Step 渲染 `<report>`；`CheckpointHook::review_decision` + `StepDecision {Handover, Yield}` / `DecisionVerdict`（宿主在 Step 生效前接受或可修正地拒绝交接与“只有阶段报告”的让出）；xllm Session 协议文本 |
| T01 数据模型与存储 | `agent_tool/src/todo_tools.rs` 重写为库：`TodoList`（`opendan.todo_list/1`，持久单调 `next_seq`、`rev`）、`TodoRecord`、`TodoAttempt`（serial: `caller`/`run_id`；explore: `child_sid`/`dispatch`）、`TodoReview`、`ReportRef`、`ChildObservation`；状态推导与迁移保护、`serial_pick`、`begin/end_serial`、`begin/bind/withdraw_explore`、`observe_child`、`review`、`cancel/clean/retry`、`unconverged`、`render_view`；`TodoStore`（`todos.lock` flock + 原子替换）。删除 `TodoTool` / `delegateTask` / workspace `tasks.json`（`agent_tool_cli_dev` 去掉注册与旧测试） |
| 协议 | `protocol/behavior.rs` `[entry] todo`；`config.rs` `BehaviorEntry.todo`（只允许 `create_sub_context`）；`state.rs` `ChildCall.todo: TodoBinding`、`SessionState.todo_view_rev`；`protocol/mod.rs` 导出 `todo_list` schema |
| T02 命令面 | `bin/xagent.rs` `cmd_todo`：`add / explore / dispatch / list [--json] / show [--report] / review / cancel / clean / retry`；`--content -`（stdin）/ `--content-file`；Session 外只读（`xagent todo list <sid>`、`show --session`）；`ctl decide` 无队列路径改为 `api::decide_unqueued`；`host()` 与 OpenDAN Loader 导出 `LIBOPENDAN_MAX_CHILDREN` |
| T03 准入、派发、等待 | `src/todos.rs`（`dispatch`、`observe`、`sync`、`read_report`、`save_submission`、`explore_key`）；`api.rs` `sub_session_id`、`decide_unqueued`；`runner/todos.rs`（`review_handover`、`review_yield`、`settle`、`bind_serial`、`serial_returned`、`view_for_batch`、`current`、`unconverged`）；`runner/hook.rs` `review_decision`；`runner/outcome.rs`（Done 后 `settle`、`hand_over` 前 `bind_serial`、`commit_run_end` 回写串行结果与 brief、`one_line` 等待文案）；`runner/reconcile.rs` 交接重做也走 `settle` |
| T04 结果与 Review | 探索：`drive.rs` 在 drive 开头与每个批次边界 `todos::sync`，`poll_children` 交付结束事件前再对账；`children.rs` 结束事件改为 Todo 摘要 + `todo show` 指引；Review 写子 Session acceptance（`decide_unqueued`）；`reports.rs::validate` 结束前检查列表收敛 |
| T05 上下文与提示词 | `assembler.rs` `InputMaterial.todos / current_todo`、内建模板的 `<todo_list>` 块、变量 `todos_text / session.todos / session.current_todo`；`live.rs`/`receipts.rs` 以 receipt `extra.todo_rev` 提交 `todo_view_rev`；Jarvis `plan.toml`（TodoList 决策规则、命令、等待、Review、heredoc 传正文）、`do.toml`（执行 `<sub_task>`，`[entry] todo = true`）、新增 `explore.toml`（function_call 探索入口） |
| T06 可观察性 | `todo list --json`（Todo → attempt → `child_sid` / `run_id` → `report_ref`）、`todo show --report`；DV `collect` 的 timeline / views |
| T07 自动化验证 | `lib_opendan/tests/plan_do_todo.rs`（10 个，见 §8.3）；`agent_tool` `todo_tools::tests`（5）；llm_context behavior_loop / step_record 新用例 |
| T08 真实模型 | `test/plan_do_todo_dv/`（README、task.md、fixture、main.py prepare/run/collect/verify） |
| T09 文档 | `doc/opendan/Agent TodoList.md` 重写为 v2（当前实现）；`xAgent.md` §4.15 无决策即等待、§4.16 无队列验收、新增 §4.17；`Session Directory Protocol.md`（目录、`entry.todo`、`call.todo`、`todo_view_rev`、`todos.json` 写者、等待与结束规则）；`build-in agent-tool手册.md` §8；fixtures 与 schema 重生成（新增 `todo_list.schema.json`） |

### 8.2 与本文不同或补充的决定

1. **“无决策”需要 llm_context 支持**：behavior loop 里只有阶段报告的 Step 原本不会结束循环（会再推理），所以“等待 = 不做决策”经新的 `review_decision` 钩子实现。宿主只在“顶层 context 且有未收齐的子 Session”时接受让出；没有可等的就不表态（阶段报告照旧、继续，与现有 `report.rs` 用例一致），连续第三个只有阶段报告的回复才拒绝（真实空转在 mock 中出现过，见 8.6）。
2. **可修正错误**经同一钩子在 Step 生效前拒绝交接，不是 `refuse_handover`；`settle` 里的 `refuse_handover` 只是没有钩子的执行者（如崩溃后重做）的兜底。
3. **串行准入多一条规则**：有已结束但未 Review 的结果时，`next_behavior = do` 先被拒绝，要求先 Review——把“Review 后才能作为后续依据”落到机制上。依赖用 `--after`（被接受后才就绪），足以表达“两个探索 Review 后再串行验证”。
4. **报告引用**：串行 attempt 的报告副本存 `.opendan_agent_session/reports/<id>/submission.json`（DO 的 run 目录之后会被清理）；`process_result` 与 `outcome(process_done)` 只带一句话与 `todo show` 指引，报告全文按需读取。探索的报告即子 Session 的 `final_report`。
5. **探索派发**持列表锁完成“记录 → 创建 → 绑定”，sid 由 `api::sub_session_id` 预先算出；宿主对账只处理锁空闲时遗留的 `dispatching`。并发闸在工具进程内判断（`LIBOPENDAN_MAX_CHILDREN` + `max_sub_sessions`），超限退出码 3。
6. **视图去重**按列表 `rev`（子状态变化由回写体现为 rev 变化），`todo_view_rev` 随 receipt 提交、恢复时随 receipt 重放。
7. **Review 与子 Session acceptance**：accept → accepted，reject → discarded，need_evidence 不动；无队列子 Session 由驱动者身份持子锁直接写（`decide_unqueued`，`ctl decide` 共用）。
8. **正文传递**：真实运行发现 PLAN 把含反引号的正文放在双引号里，bash 命令替换静默吞掉了验收标记（模型自己用 `todo show` 发现并补派了 T03/T04）。于是加了 `--content -`（配合 `<<'EOF'`）与 `--content-file`，并在 PLAN 提示词里写明；第二次失败变体运行中 PLAN 已改用 heredoc，正文完整。

### 8.3 验收矩阵结果

| 编号 | 结果 | 证据 |
| --- | --- | --- |
| V01 | 通过 | `do_without_a_ready_todo_is_corrected_or_waits`：无 Todo 无探索 → 一个纠错 Step、同 run 继续、Turn 不失败、无 DO run；有探索在跑 → `outcome.kind = wait` |
| V02 | 通过 | `explorations_in_parallel_then_a_reviewed_serial_todo`：串行 attempt 绑定 `caller = (run, step:n)`、记录 DO `run_id`、报告可读、Review 记录；DV T05 |
| V03 | 通过 | 同上用例 + Gate barrier 证明区间重叠；串行选择跳过探索 Todo（`todo_tools::serial_pick_rules`）；DV `parallel.overlap` |
| V04 | 通过 | `dispatch_limits_are_observable_and_recoverable`（`max_sub_sessions = 1` 与 `LIBOPENDAN_MAX_CHILDREN = 1` 各一遍：`dispatched:false`、`refusals`、空出后 `todo dispatch` 成功）；探索 `--workspace none` 并行 |
| V05 | 通过 | 探索首条输入与 DO `<sub_task>` 含只在正文出现的标记（mock：MARK-*；DV：ACCEPT-A/B/C，见 `marker.*`）；继承 Step 带 `<report>`（llm_context 单测） |
| V06 | 通过 | mock 与 DV：`plan.waited`、`plan.woken_by_child_end`（`_child` 输入）、串行 Todo 晚于 Review、`<sub_task>` 含已 Review 的报告引用；未 Review 时交接被拒（规则 3） |
| V07 | 通过 | `a_failed_exploration_is_rejected_and_retried`；DV 失败变体（两次运行） |
| V08 | 通过 | `tool_layer_replays_and_crashes`：同一 shell 批次两个 sid、运行中重复 dispatch 被拒且不建子、Review 重放空操作（`todo_tools` 单测） |
| V09 | 通过 | 工具进程：`todo:after_dispatch_record`（对账撤回）、`todo:after_child_created`（重派/对账绑定同一子）；runner 子进程：`serial_bind_and_writeback_survive_a_crash`（`todo:after_serial_bind`、`todo:after_serial_writeback`：一个 attempt、DO 动作只执行一次、回写后不重跑 DO）；`a_host_killed_while_waiting_resumes_the_same_explorations`（kill -9 等待中的宿主，新宿主接续同一批子 Session、同一 Turn、各 Review 一次） |
| V10 | 通过 | 重试保留旧 attempt 与报告、迟到结果不覆盖、ID 不复用、cancel 只对未执行项（`todo_tools` 单测 + V07 用例） |
| V11 | 通过 | `end_needs_a_converged_list_and_stop_cascades`：未 Review 时结束报告被拒（可修正）；父停止 → 子 stopped、等待的 Turn 关闭、对账后 Todo `stopped` |
| V12 | 通过 | mock：视图出现、PLAN 请求中无子工具输出、报告正文只在 `todo show <id> --report` 之后；DV：`view.no_child_tool_output_injected`、`view.report_not_injected`、`view.reports_read_on_demand`（PLAN 读到的每份报告都经 `todo show --report`）、`view.compact_fields_only` |
| V13 | 通过 | §8.5 三次真实运行 |
| 空转上限 | 通过 | `idle_progress_reports_are_bounded` |

### 8.4 验证命令（在 `src/`）

- `cargo test -p llm_context`（227）；`cargo test -p agent_tool --lib`（含 `todo_tools` 5、`behavior_protocol_tests` 4）
- `cargo test -p libopendan --test plan_do_todo -- --test-threads=1`（10）
- `cargo test -p libopendan -p opendan -- --test-threads=1`；`cargo test -p agent_tool -p llm_context`；`cargo check --workspace --tests --exclude node_daemon`（最终结果见 8.4.1）
- 协议改动后：`cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures`
- 真实模型：`PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --variant normal|failure`（`test/plan_do_todo_dv/README.md`）

### 8.5 真实运行（CLIProxyAPI `http://127.0.0.1:8317/v1`，`llm.plan / llm.code / llm.chat → gpt-6-sol`，证据根目录 `/home/lzc/project/plan_do_todo_dv_runs/`）

| 案例 | Session | 结果 | 用时 / tokens | 过程 |
| --- | --- | --- | --- | --- |
| normal `20261016T233435-normal` | `work-20261017T063436-b6f049a973af4ec3ab9eafbd664ae60e` | succeeded，verify 38/38 | 370 s / 610,511（父 510,051，38 rounds，5 runs） | T01/T02 并行探索（重叠：两者首个工具调用都在 889.2–889.6 s，首个结束在 910.7 s）；PLAN 用 `todo show` 发现 T01/T02 正文里的验收标记被 shell 吞掉，补派 T03/T04（正文合规），以阶段报告等待两次、被 `_child` 唤醒、逐个 Review（4 个 accept，T01/T02 作辅助）；T05 串行公开入口复核（`after = T03,T04`，`ACCEPT-C-3K9`），T06 串行撰写 review.md / decisions.md。各 Todo 用时（attempt 开始到回写）：T01 27 s、T02 41 s、T03 27 s、T04 27 s、T05 40 s、T06 208 s |
| failure `20261016T234220-failure`（作废） | `work-20261017T064228-1e10c3d454cc4382bdc1f4ea48d8f374` | succeeded，verify 24/24 | 464 s / 726,293 | fixture 里残留 `__pycache__/idempotency.cpython-313.pyc`（我先前在 fixture 源目录跑过单测），补充探索 T05 借字节码绕过了“缺失输入”。PLAN 的处理本身正确（驳回 T02/T04、补派探索、最终报告写明 pyc 限制），但场景不纯，已修 fixture（删 `__pycache__`、prepare 忽略 `*.pyc`）并重跑 |
| failure `20261016T235057-failure` | `work-20261017T065101-8cb3aee2517043d9b8a31b0f243865ff` | succeeded，verify 21/21 | 298 s / 584,546 | 新提示词下 PLAN 用 `--content - <<'EOF'` 传正文，2 个探索并行（重叠：首个工具调用 877.4–877.8 s，首个结束 913.1 s）；B 分支如实报告 `ModuleNotFoundError`、“无法验证或否定契约 2、3”，PLAN 以“准确的阻断报告”接受、未把租户问题当作已证实；串行复核与撰写文档；最终报告：库存缺陷成立，缺少 `idempotency.py`、租户隔离与幂等行为“无法判断” |

人工 Review（我逐项核对了 normal 案例的 `review.md`、`decisions.md`、`parent/todo_view.xml`、`timeline.json`）：两项预置缺陷均被证实并定位正确（`inventory.py:19` 先扣减后检查且不回滚；`idempotency.py` 的键只有 `request_id`），正常路径与同租户重试被明确排除为误报，非正数量未被误报；每项发现可回到 Todo ID、子 Session / run ID、原始报告、复现脚本与输出；`decisions.md` 说明了补派 T03/T04 的原因。failure 案例的最终结论与证据一致，没有接受空报告或把列表直接标成全部完成。

verify 脚本的修订（如实记录）：第一次 verify normal 案例为 33/36，三项失败经核查都是校验脚本的范围错误，不是系统缺陷——(1) 把 `ACCEPT-C-3K9` 要求套到了每个串行 Todo（T06 是写文档的 Todo）；(2)(3) 把 DO（T06 的子 context，合法读取子目录以撰写 review.md）的动作结果算进了 PLAN 的上下文，且把 `ls -la` 等通用行当作泄漏。修订后 V12 只看 PLAN 自己 run 的内容，“宿主注入”只看输入批次，PLAN 读取报告的每一次都要求经 `todo show --report` / `read-session`。重跑的失败案例又暴露两处假阳性（一句话摘要本就按设计出现在结束事件里；PLAN 自己 `<runtime>` 里的路径），以及 `failure.branch_not_accepted_blindly` 原本靠“Todo 数 > 3”这一弱判据通过，已改成按分支报告是否如实报阻断、PLAN 是驳回/补充还是只以阻断接受并如实报告来判断。三个案例在最终脚本下分别为 38/38、24/24、21/21。

### 8.5.1 回归（最终代码，2026-10-17）

- `cargo test -p libopendan -p opendan -- --test-threads=1`：368 通过、0 失败（含重生成的 fixtures 与 `plan_do_todo` 10 个）。
- `cargo test -p agent_tool -p llm_context`：466 通过、0 失败。
- `cargo check --workspace --tests --exclude node_daemon`：通过。

### 8.6 坑与剩余限制

- 测试里的模型脚本是同步闭包，`std::thread::sleep` 做 barrier 会占住 tokio worker，让排在其后的子 Session 看起来“串行”；改为异步 `Gate`（`LlmClient` 包装）后真实重叠。
- 对账竞态（已修）：批次边界的 sync 读到子 Session 仍在运行，紧接着 `poll_children` 看到它已结束并交付了结束事件，列表停在 running，PLAN 永远“等待”，又因阶段报告不结束循环而空转。现在交付子结束事件前再 sync 一次，并加了连续阶段报告的上限。
- `pkill -f <pattern>` 会匹配到执行它的 shell 自己（又踩一次），用 `ps … | awk` 取 pid。
- 探索 attempt 的 `ended_at_ms` 是父宿主回写的时间（批次边界），不是子 Session 结束的时间；精确区间看子 Session worklog（DV 的 `timeline.json`）。
- 探索子 Session 的 `blocked`（等用户回答）需要 PLAN 显式处理；目前没有“从父侧回答探索子 Session 的提问”的 Todo 命令（可 `agent-session post`，但默认探索子 Session 无队列）。
- 列表不可编辑正文（按设计）；正文写错只能补新 Todo 或重试（真实运行中 PLAN 正是这样做的）。
- WebUI 尚未展示 TodoList（T06 说“可后续完善”）。
- 真实运行用直连 OpenAI 兼容 provider，模型别名在案例副本里映射；没有在 DV Zone / AICC 上跑。Jarvis 镜像未重建，部署中的 OpenDAN 仍是旧版。
