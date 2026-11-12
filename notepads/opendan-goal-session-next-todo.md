# Goal Session 继续推进 TODO

日期：2026-11-10

状态：A 组 2026-11-10 实施并在本机 Zone 补验（§7）。**2026-11-11 实施了 H、B、C0、D1、D5 的一半、E4、E5、F1、F2、G1、G2（记录在 §10，未提交）。** 剩下的：D3（workflow 计划后端，依赖 workflow 的修复）、F3（等一个跑了一周的 Goal 的数据，DV 先量到一组见 §10.6）与按裁决推后的 D4、E1、E2、E3（等日历 / 待办接入）。DV 之后 owner 定了硬保障下限 1 小时（§10.6，已实施）。D5 的固定提醒那一半按 owner 11-11 的要求改由 Goal 承担（§10.7，已实施未提交）。裁决原文在 §8、§9。

本单是 Goal 剩余工作的总入口：排顺序，标明依赖和裁决状态，并补上此前没有任何 TODO 承接的项。已有详细方案的不在这里重写，只给指针，实施时按原文的方案做：

- [Goal Session 对齐全异步模型 TODO](opendan-goal-session-async-todo.md)（下称“异步单”）：Q-G*、Q-U*、Q-S* 的选项与建议，§8 的实施清单。
- [Jarvis 默认 Goal Session TODO](jarvis-default-goal-session-todo.md)（下称“默认 Goal 单”）：P1 / P2 的 T05、T06，验收 V06、V10–V16。
- [Goal Session TODO](done/opendan-goal-session-todo.md)（下称“第一段”）：§9 的第二段输入、§11.5 的遗留。

现状不在本单重复，见[架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>) §15.7（2026-11-10 按提交 `667468998` 核对的实现、与设计的差异和未实现项）。

读者：owner（裁决与排序）和后续实施的 code agent。路径相对 `buckyos/`；`lib_opendan/` 指 `src/frame/lib_opendan/src/`，`jarvis/` 指 `src/apps/jarvis_agent/agent/`。行号是 2026-11-10 的源码，实施前核对。

## 0. 结论先行

1. Goal 的框架已经完成并提交：Goal 对象、Activation、派生 Work、预算、休眠、两个内置 Goal、Goals 页面。剩下的工作分七组（A–G）。
2. 不需要裁决、现在就能做的只有 A 组：把 11-02 已裁决的“Goal 建的 Work 一律 `work_simple`”落地，补第一段的一处遗留，补真实 Zone 上的验证。
3. 对用户最可见的缺口有两个，owner 2026-11-10 对它们的处理（§9）：
   - Goal 有话要说时只能发通知，对话里的 Agent 不知道也不开口。交付的接口是命令，先把通知命令的命令行定下来（C0），对话通路是它背后的实现，以后再调。
   - 没有提醒：今天对 Agent 说“每天九点提醒我”得到的是 `unsupported`。改用户的待办和日历要等这些对象接入，接入前没有这种交付方法（D4、E2、E3 搁置）。按日历时间给 Goal 检查机会不受影响（D1）。
4. 建议顺序 A → H → B → C → D → E → F → G。H（触发门槛，2026-11-10 裁决）排在最前：它定的是 Goal 什么时候推理、一天能花多少，B 里的预算口径和残留任务是它的一部分。B 余下的排在 C、D 前面，因为 C、D 会让 Goal 跑得更多，先把账算对。
5. 2026-11-10 owner：已有问题基本按建议走（§3.1、§3.2 的建议即结论，例外已在表里标出）。§3.4 的四项同日也定了，没有待裁决的问题。

## 1. 分组与依赖

| 组 | 内容 | 依赖 | 裁决状态 |
| --- | --- | --- | --- |
| A 收尾（已完成） | Q-G4 落地、第一段遗留、真实 Zone 补验 | 无 | 不需要 |
| H 触发门槛 | 硬保障、硬间隔、预算（每个 Goal 的和 Agent 总量）、检查脚本四道门槛；事件触发收口；预算告知 Agent；内置 Goal 改成检查脚本 | 吸收 B3、B6 | 已裁决（§8） |
| B 运行语义 | Activation 墙钟上限、禁用 `ask_user`、宿主缺省 behavior、Goal 头并入输入（B3、B6 并入 H） | 无 | 按建议 |
| C 告诉用户 | 交付的接口是命令：先定通知命令的命令行（C0）；default chat session 等是命令背后的实现，以后再调 | C0 无依赖 | C0 已裁决；其余不需要现在定 |
| D 时间 | cron 与时区、workflow 计划后端（follow_up 已取消，对象订阅不排期；timer 对象和定时意图等日历 / 待办接入） | D3 依赖 workflow 的修复 | 按建议 |
| E 内置 Goal | self_check 退役、结晶 Goal 的开关与验证；整理 Goal 的 P1 / P2 和日程管家等日历 / 待办接入 | E2、E3 依赖日历 / 待办的命令 | N1、N8 按建议；E1 整体推后 |
| F 上下文与方法 | Goal Skill 运行时读取、内置 goal skill（含用 app 交付）、Goal 压缩 | 无 | N2 按建议；首批 goal skill 已定；N3 等数据 |
| G 治理 | 就绪性检查、页面（修订差异、数量提示、成本、编辑） | 无 | N4、N5 按建议 |
| 不排期 | XLLM 结晶、离线 Self Correct、对象订阅驱动 Goal、子 Work 预算预留、跨节点与多 Session | — | N6、N7 |

## 2. 任务

### A. 收尾（不需要裁决）

- [x] **A1 Goal 建的 Work 用 `work_simple`。**（已实施，见 §7） 2026-11-02 已裁决（异步单 Q-G4）。今天 `goal.toml` 不带 `--behavior`，取到的是 `[session.work] default_behavior = "plan_simple"`，建出来的是 plan 加 Todo 子会话，子会话不计入 Goal 的 Work 数。
  - 落点：`jarvis/behaviors/goal.toml:18` 的 `create-worksession` 示例加 `--behavior work_simple`。宿主侧的缺省（提示词漏写时不退回 plan）是 B4，等裁决。
  - 验收：`cargo test -p opendan`（behavior 能解析）、`cargo test -p libopendan --test goal_session`；DV 里 Goal 建出的 Work 冻结的 behavior 是 `work_simple`，名下没有 Todo 子会话。
  - 手册：核对 `opendan_agent_dev` 有没有描述 Goal 建 Work 的形状，有就同步。
- [x] **A2 第一段的遗留。**（已实施，见 §7）
  - Goal Session 因 Workspace 绑定失效而 failed 时不写 Activation 记录（`lib_opendan/runner/drive.rs:252` 的 `fail_workspace` 没有 Goal 分支）。先写用例复现：Goal 的 `scope.workspace` 失效后，Goal 页面上看不看得到原因，restart 能不能恢复；再决定是补一条 `failed` 的 Activation 记录，还是只更新 Goal State 的 `last_error`。
  - 不做的两条：Activation 预算不含 `max_tool_iterations`（每个 run 仍受 behavior 的上限，记为已知）；Goal Session 内的感知不带 `goal_ref`（已解决，`runner/consolidation.rs:1188`）。
- [x] **A3 真实 Zone 补验。**（四条都通过，发现的问题见 §7.3） 只验证不改代码，发现的问题另记。证据放 `notepads/evidence/goal-session-next/<date>/`，结束时归档测试 Goal。
  1. 容器形态（`./build_aios --local-test` 后 `uv run src/start.py --skip-update`）：整理 Goal 播种一次、无材料时不调用模型；等到一次由 30 分钟定时触发（不是手动 activate）的真实整理。
  2. WebUI 经 zone 网关登录：休眠、唤醒、归档、reopen、restart 都能做，内置 Goal 不能归档。
  3. 休眠语义：Activation 进行中置休眠，在下一个推理边界收尾；派生的 Work 随之暂停，唤醒后恢复；对 Goal Session 发 Session 级 stop 等于休眠。
  4. 把 `[session.goal.crystallize]` 打开跑一次结晶 Goal，看它对真实模型的表现（读了哪些 Session、有没有越权改写）。

### H. 触发门槛（2026-11-10 裁决，未实施）

裁决原文在 §8，设计写在架构设计 §12.1.1。要点：一次运行机会要不要变成一次推理，由四道门槛决定。

| 门槛 | 含义 | 缺省 | 谁能改 |
| --- | --- | --- | --- |
| 硬保障 | 距上次推理超过这个时间，无条件推理一次，压过预算和检查脚本。用途是兜底：Agent 把检查脚本写坏了也有机会醒来修。可以关 | 72 小时 | 用户 |
| 硬间隔 | 两次推理之间不能小于这个时间 | 15 分钟 | 用户 |
| 预算 | 一个周期内这个 Goal 能用的 token，输入加输出，含它派出的 Work。用完不推理，直到周期重置或硬保障到点。另有一个 Agent 的后台总量，所有 Goal 和它们的 Work 共用 | 每个 Goal 24 小时 200 万；Agent 总量 500 万 | 用户 |
| 检查 | 时间表（可以比较密）拉起检查脚本，返回 true 才推理，返回 false 或出错都不推理 | 每 5 分钟，下限 60 秒；内置整理 Goal 每 30 分钟 | 用户配置，Agent 可调时间表、可写脚本 |

正常路径是时间表拉起检查、检查通过后推理；硬间隔和预算只是拦截；硬保障只在极端情况触发。事件（Work 结束、用户转来的消息、问题得到回答）也能唤醒 Goal，不过检查脚本，但受硬间隔和预算保护。

现状对照：硬保障没有；硬间隔是 `schedule.min_interval_secs`（Jarvis 缺省 60 秒），预算是 `budget.period`（通用 Goal 缺省不设 token 上限），两者今天都只拦定时触发；检查只有整理 Goal 写在宿主里的那一个（`lib_opendan/host/maintenance.rs::decide`）。

- [x] **H0 事件触发收口**（前置）。Goal 只为这几种事件醒：派生的 Work 结束、Work 里真正要人回答的问题、用户转来的消息、问题得到回答、手动激活。要堵的三处：
  - Activation 关闭后残留的任务结束时打开新的 Activation（原 B6，异步单 Q-G7 的建议 (a)：Activation 关闭时放手）。
  - Work 等自己的后台任务时的 `needs_input` 也唤醒 Goal（§7.3 第 2 条）。
  - Work 自己的任务结束事件混进 Goal 的原因（§7.3 第 3 条，来源先查清）。
- [x] **H1 配置与协议。** Goal 定义里表达四道门槛，直接改现有字段，不做兼容。
  - 硬保障：新增（秒，空表示关）。硬间隔：沿用 `schedule.min_interval_secs`，缺省改 900，含义改为管所有触发。预算：`budget.period` 的缺省改为 86400 秒、200 万 token。
  - Agent 总量：一个 Agent 一天的后台 token 上限，所有 Goal 的推理和它们派出的 Work 共用，不含对话和用户直接发起的 Work。放在 Agent 的配置里（`agent.toml [session.goal]` 一级），不在某个 Goal 的定义里。缺省 500 万，够两三个 Goal 各自跑满（整理者的建议，owner 2026-11-10 表示已有问题基本按建议走）。
  - 检查：时间表（今天的 `every_secs`，D1 做完后可以是 cron）加脚本引用；缺省每 5 分钟，下限 60 秒，Agent 只能在下限以上调。内置整理 Goal 维持每 30 分钟。
  - 权限：前三道只有 owner 侧和创建它的 UI Session 能改（现状）。检查的脚本和时间表 Goal Session 自己也能改，新增入口（`agent-session goal check set …`，经驱动者写入，保持单写者），每次修改记入 `changes.jsonl`。
  - 落点：`lib_opendan/protocol/goal.rs`（`GoalScheduleSpec`、`PeriodBudget`）、`template.rs` 的 `GoalDefaults`、`state/goals.rs` 的权限、`jarvis/agent.toml [session.goal]`、`xagent_cli.rs`。
- [x] **H2 统一闸门。** 今天的准入链只管定时触发（`lib_opendan/host/goals.rs` 的 `AdmissionCheck`），事件触发不过任何门槛。改成所有触发源在打开 Activation 之前过同一个闸门。顺序是实施建议，owner 没有单独裁决：
  1. Goal 休眠或已结束：不推理。休眠压过硬保障。
  2. 距上次推理不到硬间隔：不推理。
  3. 距上次推理超过硬保障：推理，跳过 4、5。
  4. 这个 Goal 的周期预算用完，或 Agent 的后台总量用完：不推理。
  5. 触发源是时间表时跑检查脚本：true 推理，false 或出错不推理。事件触发不跑脚本。
  - 被 2、4 拦下的事件留在队列里，放行时合并成一次 Activation（`runner/goal.rs` 的 `Gate::Paused` 已经能把输入留在队列里，照这个做法加一种“等待”）。时间表被拦下的那一次直接跳过，不补。
  - 手动激活是 owner 的显式操作，不过门槛。
  - 每次拦截记原因和次数，供 H5 和页面使用。
- [x] **H3 检查脚本的执行。**
  - 在 Agent 的私有容器里用 Deno 跑，只能是 TypeScript。用标准层 Tool 自带的 Deno（跑 `buckyos-agent` 命令的那个，`lib_opendan/baseline.rs::run_deno_command`），不用扩展包里的：扩展包没装好时检查不会全部变成出错，还能用同一份 `deno.json` 直接 import websdk。
  - 权限：给读和网络，不给 `--allow-run` 和 `--allow-write`。检查只观察不行动：它不消耗 token、不留 Activation 记录，不能让它做事。身份与这个 Goal Session 相同；带超时和输出上限。
  - 约定（实施建议）：脚本默认导出一个函数，`check(ctx, state)` 返回 `{run, reason?, state?}`。`reason` 作为这次 Activation 的原因交给模型，模型不用重查；`state` 由宿主保存、下次传回，脚本没有写权限也能记住上次看到哪。`ctx` 至少有 Goal 的 id、作用范围、上次推理和上次检查的时间。
  - 出错（抛异常、超时、返回值不合约定）不推理，记最后的错误和连续次数。不加写入时试跑，也不加出错后的修复推理（§8 第 5 条），兜底只有硬保障。
  - 给 Agent 一条按宿主同样方式试跑的命令（`agent-session goal check run`，同样的 Deno、权限和 `ctx`，不改任何状态），`goal.toml` 写明改完脚本用它测。这是让“写完自己测”做得到的工具，不是写入门槛。
  - 条件一直为真时宿主不兜底：脚本一直返回 true 的代价由硬间隔和预算封顶，Goal 从 H5 的信息里看得到自己在空转，自己改脚本。
- [x] **H4 硬保障。** 距上次推理超过保障时间时无条件给一次运行机会，原因单列（`guarantee`），带上检查最近的结果、连续出错的次数和最后一次错误，Goal 才知道要修什么。休眠的 Goal 不触发。
- [x] **H5 预算口径与告知。**
  - 周期预算含派生 Work 的 token，运行中也计入（原 B3，异步单 Q-G5 的建议 (a)）：`lib_opendan/goal.rs` 的 `current_usage`、`work_admission` 读各 Work 的 `usage.jsonl`。
  - Goal 头的 `<budget>`（`runner/goal.rs` 的 `render_builtin`）补上：本周期已用多少（自己的和 Work 的分开）、周期什么时候重置、本周期被预算和硬间隔各拦下几次、检查跑了几次及其结果（true、false、出错）、最近连续几次 `no_action`、当前的检查时间表。
  - 预算用完时正在运行的派生 Work 在下一个推理边界暂停，周期重置后恢复，只恢复因此暂停的。做法照休眠（`lib_opendan/host/goals.rs::sync_dormant_work`），暂停来源另加一种（今天是 `user / parent / interrupted / goal`）。Agent 总量用完时对所有 Goal 的 Work 同样处理。
  - Agent 总量：统计所有 Goal 在同一周期里的用量（各 Goal State 的 `usage` 加运行中 Work 的用量），在闸门里与每个 Goal 的预算一起判断；Goal 头里把 Agent 总量的已用和上限也列出来。
  - `goal.toml` 写明：这是用户给这个 Goal 背地里干活的全部 token；看到预算吃紧或空转多，就调低检查频率或收紧检查条件。
- [x] **H6 内置 Goal 改成同一套。**
  - 包里的脚本只是初始状态（N11 的裁决）：内置 Goal 和普通 Goal 一样可以改自己的检查脚本和时间表，随着使用演化。因此硬保障对内置 Goal 照常打开。包升级不覆盖 Goal 已经改过的脚本；页面上给一个恢复成包内版本的操作（实施建议）。
  - 整理 Goal：`host/maintenance.rs::decide` 里“有没有要整理的材料”的判断搬进检查脚本，初始脚本随 Jarvis 包分发。脚本拿数据要一个只读 kRPC（Memory 的版本摘要、待处理与到期数、到期复核数），今天没有，要加；全量检查本身仍在 Rust 里。
  - 清理重试、未完成回执这些不需要模型的维护留在宿主，不算推理触发。`schedule.json.gate` 的水位随之精简。
  - 结晶 Goal 加检查脚本（手册的使用记录有没有变化），避免空跑：A3 量到它一次 `no_action` 用了 18.2 万 token。
  - `tests/memory_maintenance.rs` 里 gate 的用例跟着迁移。“无材料不调用模型”改为“无材料时只有硬保障触发的那一次”：缺省 72 小时一次。默认 Goal 单的 V01 和 Agent State Protocol §9.7 要跟着改。
- [x] **H7 页面与文档。** Goal 详情页显示四道门槛的配置、检查脚本、最近的检查结果和拦截原因，owner 能改前三道（与 G2 的页面内编辑一起做）。同步 Agent State Protocol §9、schema、fixture `16_goal_session`、架构设计 §12.1.1 与 §15.7。
- 验收（`tests/goal_session.rs` 用手动时钟，脚本用测试替身和真实 Deno 各一组）：
  - 四道门槛各自生效；硬保障压过预算和检查；休眠压过硬保障；手动激活不过门槛。
  - 硬间隔内到达的 Work 结束事件留在队列里，间隔到了合并成一次 Activation；预算用完时同理，周期重置后放行。
  - 脚本里执行命令和写文件都被 Deno 拒绝；脚本出错不推理且页面看得到；`state` 能往返。
  - 运行中的 Work 的 token 让周期用量增长；Goal 头里有 H5 列的各项。
  - 预算用完时运行中的 Work 在推理边界暂停，周期重置后恢复；用户自己暂停的不被恢复。
  - 两个 Goal 合起来超过 Agent 总量时都不再推理，各自的硬保障仍然触发。
  - 内置整理 Goal 无材料时，除了硬保障那一次不调用模型；它改过的检查脚本在包升级后还在。
  - DV：建一个带检查脚本的测试 Goal（例如“某个目录里出现新文件”），看时间表只在条件成立时触发推理；故意把脚本写坏，看它静默到硬保障（把保障时间调短），再由 Goal 自己修好。

### B. Goal 运行语义对齐全异步模型

方案在异步单 §3、§7，落点在它的 §8 A1–A5，验收在它的 E1。owner 2026-11-10：按建议走。B3、B6 已随同日的裁决并入 H 组。

- [x] **B1 Activation 墙钟上限**（Q-G1，建议维持“跟到底”并加上限，缺省 1800 秒）。`ActivationBudget.max_wall_secs`（`lib_opendan/protocol/goal.rs:221`）、`runner/goal.rs::activation_exhausted`；跟进提示和 report 被拒的文案对 Goal 另写；`goal.toml` 补一句长命令的说明。
- [x] **B2 Goal Session 里去掉 `ask_user`**（Q-G2，建议宿主层剔除并提示用 `result.questions`）。今天只靠 `goal.toml` 的一句话挡着，真被调用时 Activation 会挂到用户答复或过期。入口 `lib_opendan/baseline.rs:83`，按会话过滤的位置实施前核对。
- [x] **B3 运行中 Work 的用量计入周期**（Q-G5，2026-11-10 已裁决为计入，并入 H5）。
- [x] **B4 宿主缺省 behavior**（Q-G4c，建议父是 Goal Session 且没给 `--behavior` 时取 `work_simple`，显式给别的照样允许）。`lib_opendan/api.rs::create_sub_session`，常量 `todos.rs::WORK_BEHAVIOR`。
- [x] **B5 Goal 头并入输入模板，Work 结局只出现一次**（Q-G6）。今天 Goal 头是一条独立的 User 消息，同一个 Work 结局在原因、`<works>`、事件渲染三处出现。fixture `16_goal_session` 要重新生成。
- [x] **B6 Activation 关闭后不再跟残留任务**（Q-G7，并入 H0）。今天不可取消的任务结束时会绕过准入链打开新的 Activation。

### C. 告诉用户（结果交付的第一种）

owner 2026-11-10（§9 第 1 条）：通知用户这件事，把命令行定下来，背后的实现以后可以调整。所以 Goal 告诉用户的接口是一条命令，不是结果里的字段：异步单 Q-U3、Q-U7 的建议（改成 `result.tell`、删掉 `notify send`）不再采用。异步单 §4 设计的 default chat session、由对话里的 Agent 说出来、带 URL 的泛通知，都是这条命令背后的实现，不需要改 Goal 的提示词和协议，可以以后再做。

- [x] **C0 通知命令的命令行**（2026-11-10 已裁决，按下面的建议）。今天是 `agent-session notify send --title <t> [--message <text>] [--level quiet|normal|timely|urgent] [--expires-in <dur>] [--key <k>]`（`lib_opendan/xagent_cli.rs:122`），`goal.toml` 和 `skill_crystallize.toml` 都在用。要做的只有加 `--url` 和把语义写进帮助与 `goal.toml`：
  - 名字和现有参数不动。语义写明是“把一件事告诉用户，渠道由系统决定”，命令行里不出现渠道。
  - 只加一个 `--url`：点开后落到哪。Goal 为用户做了页面或 app（§9 第 3 条）、产出了报告时，靠它把人带过去。
  - 需要用户回答的事仍然写进结果的 `questions`，不走这条命令。
- 以后调整实现时再做（方案在异步单 §4 与 §8 的 A6–A9，届时 Q-U2、U4–U6、U8、U10–U13 按异步单的建议）：
  - default chat session：每个 (Agent, owner) 一个 MessageHub 原生 1:1 对话。
  - `notify send` 的内容交给 default chat 的 UI Session，在事件打开的 Turn 里用对话口吻说出来；再发一条带 URL 的泛通知，不抄送进对话。
  - 频率上限、可以不说、不发占位、压缩后保留 Goal 引用。
  - 验收时按手机宽度走一遍：目标用户没有 PC，通知点开要直接落在该看的地方。

### D. 时间：日历时间表、生产调度、timer

- [x] **D1 cron 与时区**（Q-S3）。`GoalScheduleSpec`（`lib_opendan/protocol/goal.rs:284`）加 `cron` 与 `timezone`，与 `every_secs` 二选一；`LocalScheduleBackend`（`lib_opendan/host/goals.rs`）按墙上时间计槽；`goal create` 加对应参数。时区来源的顺序按默认 Goal 单 §5.1。不依赖 workflow，可以先做。需要时区库时先确认。
- **D2 follow_up：已取消**（N13，2026-11-10）。Goal 能调检查的时间表，检查脚本能自己判断“到没到时间”，不再需要让 Goal 在结果里声明“下次什么时候再看”。
- [ ] **D3 workflow 计划后端。** 生产定时换到 workflow 的计划任务，本地后端只留给 dev 和测试。H 组之后它承载的是检查时间表：到点由 OpenDAN 跑检查脚本，不是直接推理。
  - 先修 workflow 的四个问题（第一段 §9.1，是 10-11 的记录，workflow 在 11 月改过，实施前复核）：`opendan.command` 的 fire task 没有执行方；成功 fire 后按旧的 fire 时间算下一次，停机后会逐个补发；`buckyos-api/src/workflow_service.rs` 的类型与服务端不一致；`load_schedules` 只读第一页。
  - 再做 `WorkflowScheduleBackend`：`every_secs → RunEvery`、cron → `Cron`，目标 `opendan.command{goal.activate}`。
  - 改 workflow 按 `harness/Service Dev Loop.md`；完整的 IANA 时区要引入时区库，先确认。
- **D4 timer 对象：等日历 / 待办接入**（§9 第 2 条）。2026-11-11 owner 要求单独讨论方案，讨论稿在 [opendan-d4-timer-reminder-todo.md](./opendan-d4-timer-reminder-todo.md)，问题是“crond 还是日历”，建议做成提醒对象加 crond，未裁决。改用户的待办、日历是接入之后的命令，现在不支持就没有这种交付方法；Q-S1、S7、S8 届时再定。原设想：到点不需要推理的提醒，不是 Goal。方案在异步单 §5.2 场景 1 与 §8 D1：workflow 的 `Once` / `Cron` 加发通知的目标、按 owner 加 key 的幂等创建、单次发生的跳过与改动，以及建、列、取消的命令和手册。依赖 D3 里 workflow 的修复。
- [x] **D5 意图分析的定时分支**（Q-S4）。到点要判断的一半见 §10.2；固定提醒那一半原定等 D4，2026-11-11 owner 在 DV 里要求由 Goal 承担，改为检查脚本到点发通知、不推理（§10.7）。
- **D6 对象订阅驱动 Goal：改为不排期**（N13，2026-11-10）。“定期去看某个对象变没变”由检查脚本承担；真有等不起一个检查周期的事件再回来做，届时声明放在哪见 N6。

### E. 内置 Goal

- **E1 整理 Goal 的 P1（候选发现与确认）：整体推后**（2026-11-10 裁决），和 E2 一起等日历 / 待办接入。
  - 原因：它发现的候选里，日程现在落不了地；“建一个 Goal”又和“不强迫用户分成多个 Goal”有张力。届时候选里加一类“给已有的 Goal 增加一项职责”。
  - 留着的挂点：`lib_opendan/runner/consolidation.rs:854`（提交后发布候选）、`runner/goal.rs:921`（问题与候选的关联，`candidate_id` 今天恒为空）；类型已冻结，页面已有候选列表的位置。方案在默认 Goal 单 T05，验收 V06、V10–V13。
- **E2 整理 Goal 的 P2：落地，等日历 / 待办接入**（§9 第 2 条）。默认 Goal 单 T05 的后两条（依赖 D3、D4），验收 V16 和 V12、V13 的其余断言。T06（用模型做预检，验收 V14、V15）已取消（N13，2026-11-10）：H 组改用不花 token 的检查脚本，`[session.goal.maintenance] decision` 配置项（`lib_opendan/template.rs:76`）随 H6 删除。
- **E3 日程管家：等日历 / 待办接入**（§9 第 2 条）。它要管的是用户当天的 timer，依赖 D4。届时按“Goal 是珍贵的资源”的原则（§8 第 10 条）先看它是独立的 Goal 还是默认 Goal 的一项职责；Q-S6、S8、S9、S10 那时再定。方案在异步单 §5.2 与 §8 D6。
- [x] **E4 self_check 退役**（N1）。它今天不运行（`[loader.self_check]` 打开只会报“本版本不可用”），但类型、模板、behavior 都在，也没有独立的 TODO。
  - 删除：`SessionKind::SelfCheck`、内建模板 `self_check` 与 `_self_check_timer`（`lib_opendan/template.rs:213`）、sid 前缀 `sc-`、`[loader.self_check]`（`src/frame/opendan/src/config.rs:71`、`loader.rs:420`、`jarvis/agent.toml:41`）、`jarvis/behaviors/self_check.toml`、WebUI 的 `SessionKind`（`src/frame/opendan/web/src/model/types.ts:1`）。
  - Notebook 一并退役（Memory 需求 TD-09）：`agent_tool/src/agent_notebook.rs`、`lib_opendan/state/` 里的 `notebook_append`、`xagent_cli.rs` 与 `agent_tool_cli_dev` 的入口、`opendan/src/service.rs`。唯一的 behavior 调用方是 `self_check.toml`。
  - 文档：`doc/opendan/xAgent.md` 的模板表、架构设计 §15.3 与 §15.5、Session Control Protocol 里提到 self_check 的几处。
- [x] **E5 结晶 Goal 的开关与验证**（N8）。今天只能改 `agent.toml [session.goal.crystallize]`，页面上没有开关。验证见 A3 第 4 条。

### F. Goal 的上下文与方法

- [x] **F1 Goal Skill 运行时读取**（N2）。`goal_skills/` 能安装和列出，运行时没有任何路径读它；Goal 头的 `<current_skills>`（`lib_opendan/runner/goal.rs:424`）只列名字和一个自由文本的 `ref`。这是 Skills 单 §3.5 留给 Goal 的那一条：`ref` 指向 `goal_skills/<name>/SKILL.md` 时，让 Goal 能可靠地读到正文。同时让 `goal create` 与 `goal skills add` 能按名字从 `agent-session skill list --kind goal` 里选。
- [x] **F2 内置 goal skill。** Jarvis 包今天没有 goal skill（`jarvis/goal_skills/` 目录不存在）。首批两份（2026-11-10 裁决）：
  - **怎么写检查脚本**：只读不做、用 `state` 记住上次看到哪、`reason` 写给下一次推理看、写完用试跑命令测（配合 H3）。
  - **怎么选交付方式**：通知、为这个 Goal 做一个 app、AIWorkspace 页面。做 app 的步骤交给 Work，指向 work skill `buckyos_app_dev`、`aiworkspace_template_dev`；做好以后用 `notify send --url` 把人带过去（C0）。
  - 两份都按 owner 给的范式写（§9 第 5 条）：**Goal 的早期运行应该为后面的重复交付准备好必要的工具**。前几次 Activation 花预算把检查脚本、交付用的 app 或页面、常用的脚本建起来，后面的 Activation 只是用它们，每次都很便宜。`goal.toml` 的“Every activation”一节也要加上这一条，否则没读 skill 的 Goal 不会这么做。
  - 实施建议：Goal 给自己准备了哪些工具，要在每次 Activation 的 Goal 头里看得到。检查脚本已经在 H5 列的信息里；app 的 URL、脚本的路径建议记进 `current_skills`（`name=ref`，和 F1 一起做），restart 开新上下文时这部分要保留，今天 restart 会把 Skills 重置回定义里的初始值。
  - 软件项目维护与 Issue 跟踪那一份放到第二批。写法按 `doc/sdk/skill-dev-quickstart.md`；回归 `cargo test -p libopendan --lib skills`。
- [ ] **F3 Goal 的压缩策略**（N3）。今天恢复所需的内容都在每次重建的 Goal 头里，历史走通用压缩；连续的 `no_action` Activation 让历史逐轮增长，直到通用压缩触发（第一段 DV 量到每轮约 1k token，那之后历史重建改过，数字要重新量）。先量一个跑了一周的 Goal 的历史构成，再定方案。

### G. 治理与管理面

- [x] **G1 创建 Work 的就绪性检查**（N4）。设计 §3.1、§14.5 要求 UI 和 Goal 建 Work 走同一套检查；今天两个入口都只有提示词约束。
- [x] **G2 Goals 页面**（N5）。`src/frame/opendan/web/src/pages/{Goals,GoalDetail}.tsx`，e2e 在 `tests/e2e/goals.spec.ts`。
  - 修订前后的差异：两个修订的字段对比，数据已有（`goals.revision`）。
  - Goal 数量提示：活动 Goal 超过阈值时在 Goals 区域提示，阈值待定（设计 §13.4 的候选是 16）。
  - 成本：详情页按周期显示用量的变化，数据在 Activation 记录里。
  - 页面内编辑：owner 侧的 `goals.revise` 已经可调，页面今天不提供表单。

### 不排期

- XLLM 结晶 Goal（设计 §9.4）：面向企业客服，不作为个人 Agent 的默认 Goal。
- 离线 Self Correct Goal（设计 §9.5）：前置是工具意图记录，今天没有。旧的现场打造脚手架的去留见 N7。
- 子 Work 的预算预留、费用维度（设计 §12.2、§12.3）：等 H5 之后有了真实的用量数据再定。Agent 总预算已提到 N9。
- 跨节点运行与多 Session 协作（设计 §12.4）。

## 3. 待裁决

### 3.1 已有的问题（选项与理由在异步单 §7）

owner 2026-11-10：已有问题基本按建议走。下表的“结论”就是原来的建议；与同日的裁决冲突的两类单独标出。

| 组 | 问题 | 结论 |
| --- | --- | --- |
| B | Q-G1 Activation 内的长任务 | 维持跟到底，加墙钟上限，缺省 1800 秒 |
| B | Q-G2 Goal 里的 `ask_user` | 宿主层剔除 |
| B | Q-G3 问题答案的回流 | 先不动 |
| B | Q-G4c 宿主要不要强制 `work_simple` | 改缺省，不拒绝显式指定 |
| H | Q-G5 预算口径 | 计入，见 H5 |
| B | Q-G6 Activation 的输入形状 | Goal 头并入输入模板 |
| H | Q-G7 残留任务 | Activation 关闭时放手，见 H0 |
| C | Q-U3、Q-U7 怎么声明要告诉用户的事 | **不按原建议**：接口是命令，不是 `result.tell`，`notify send` 保留（§9 第 1 条） |
| C | Q-U2、U4–U6、U8–U13 | 是命令背后的实现，调整实现时按异步单的建议，现在不需要定 |
| D | Q-S3 Goal 的时间表 | cron 加时区；follow_up 已取消（N13） |
| D、E | Q-S1、S2、S4–S10 timer 对象、定时意图、日程管家、例行任务 | **搁置**：等日历 / 待办接入（§9 第 2 条），届时连同“Goal 是珍贵的资源”一起重看 |

### 3.2 本单新增

owner 2026-11-10：基本按建议走。N3 要先量数据，N6 随 D6 不排期；N1 删的是 Agent 自己的 Notebook（只有 `self_check` 在用），不是 §9 第 2 条说的用户的 notebook。

| N | 问题 | 选项 | 建议（即结论） |
| --- | --- | --- | --- |
| N1 | self_check 什么时候删 | (a) 现在就删，Notebook 一并退役；(b) 等 D4、E3 承接了提醒再删 | **(a)**。它本来就不运行，留着只是让各处多一个要照顾的会话类型；提醒的缺口不因为它在而变小 |
| N2 | Goal Skill 的正文怎么进上下文 | (a) 正文进 Goal 头；(b) Goal 头只给能直接读的路径，由 Goal 按需读；(c) 像 work skill 那样在 Session 创建时冻结进提示词 | **(b)**。Goal 头每次 Activation 都重发，正文进去成本随 Skills 数量线性增长；(c) 与“Goal 运行中增删 Skills”冲突 |
| N3 | Goal 的压缩方案 | (a) 关闭的 Activation 在历史里折叠成它的结论，只保留最近几次的完整推理；(b) 维持通用压缩，只给压缩提示 | 量完数据再定；倾向 **(a)**，因为恢复所需内容已在 Goal 头里，历史的价值只是最近几次怎么做的 |
| N4 | 就绪性检查做到哪 | (a) 只靠提示词（现状）；(b) 宿主做机械检查：同一 Goal 下不重复建任务书相同且未结束的 Work、任务书不能等于 Goal 的 objective；(c) 建 Work 前过一次模型判定 | **(b)**。(c) 每次建 Work 多一次推理，不值 |
| N5 | 页面内编辑到什么程度 | (a) 不提供，改目标走对话；(b) 只开放预算和时间表；(c) 全部字段 | **(b)**。目标文本的修改需要澄清，适合对话；预算和时间表是调参，适合表单 |
| N6 | 对象订阅在哪声明（D6 已改为不排期，暂不需要定） | (a) Goal 定义里加 `watch`，宿主绑定；(b) Goal Session 自己 `subscribe`，随 epoch 丢失 | **(a)**。定义是真相，restart 之后订阅还在 |
| N7 | 旧的现场打造脚手架（`agent_tool` 的 `llm_tool_carft`）| (a) 删除；(b) 留着等 Self Correct | **(a)**，如果确认运行时确实没有接入它。设计 §9.5.1 已经放弃现场打造 |
| N8 | 结晶 Goal 的开关放在哪 | (a) Skills 页；(b) Goals 页；(c) 维持改 `agent.toml` | **(a)**。它产出的草稿也在 Skills 页 |

### 3.3 H 组的问题（2026-11-10 已全部裁决）

| N | 问题 | 裁决 | 落在哪 |
| --- | --- | --- | --- |
| N9 | 要不要 Agent 的总量上限 | 加。缺省值没有给，实施前要定 | H1、H2、H5 |
| N10 | 预算用完时，正在跑的派生 Work 怎么办 | 在下一个推理边界暂停，周期重置后恢复 | H5 |
| N11 | 内置 Goal 的检查脚本和硬保障 | 包里的脚本只是初始状态，默认 Goal 也要能自我演化；整理者据此把硬保障对内置 Goal 也打开 | H6 |
| N12 | 检查时间表的缺省和下限 | 缺省 5 分钟，下限 60 秒；内置整理 Goal 30 分钟 | H1 |
| N13 | D2（follow_up）、D6（对象订阅）、T06（模型预检） | 取消 D2 和 T06，D6 改为不排期 | D、E2 |

### 3.4 最后四项（2026-11-10 已全部裁决）

| # | 问题 | 裁决 | 落在哪 |
| --- | --- | --- | --- |
| 1 | 通知命令的命令行 | 按建议：保留 `agent-session notify send` 和现有参数，只加 `--url`；渠道不进命令行；要人回答的事仍走结果里的 `questions` | C0 |
| 2 | E1 的范围 | 整体推后，和 E2 一起等日历 / 待办接入 | E1 |
| 3 | 首批内置 goal skill | 按建议两份：怎么写检查脚本、怎么选交付方式。范式：Goal 的早期运行为后面的重复交付准备好工具 | F2 |
| 4 | Goal 的压缩方案（N3） | 按建议：H 组做完、有一个跑了一周的 Goal 之后量数据再定 | F3 |

## 4. 验证

在 `src/` 下，按改动范围选：

```bash
cargo test -p libopendan --test goal_session -- --test-threads=1
cargo test -p libopendan --test memory_maintenance -- --test-threads=1
cargo test -p libopendan --test session_control -- --test-threads=1
cargo test -p libopendan --test notify -- --test-threads=1
cargo test -p libopendan --test fixtures -- --test-threads=1
cargo test -p opendan -- --test-threads=1
# 改 workflow 时
cargo test -p workflow -- --test-threads=1
```

WebUI 在 `src/frame/opendan/web` 下跑 `pnpm check`、`pnpm lint`、`pnpm test:e2e`。

DV 用本机 Zone：只验宿主侧逻辑用 `./debug_jarvis.sh devtest --agent <agent_id>`；涉及容器内的 OpenDAN 或 WebUI 用 `./build_aios --local-test` 加 `uv run src/start.py --skip-update`。改了 Jarvis 的 behavior 要重新打包模板才会进到已部署的 Agent。mock 通过不算真实模型或真实 Zone 通过，没跑的写明没跑。

## 5. 实施时守住的边界

- 第一段 §10 的禁止项仍然有效：不用 TimerBridge 驱动 Goal，不在 Goal 内部做自我唤醒，不把 Goal 定义复制到 Goal 头之外当真相，不让 Activation 失败结束 Goal Session，不把 `no_action` 记成达成，不把“没观察到”记成“没有变化”。
- Goal 不自己写进对话（异步单 §4.3 的选项 C 已否决）。
- skill 是纯文本，只有 owner 能安装：内置 goal skill 随 Jarvis 包分发，Goal 自己不安装任何东西。
- 群消息、联系人策略不在本单范围；tell 只进 owner 的 default chat，不进群。
- 改 Goal 的协议字段（定义、结果、预算、时间表）时同步：`lib_opendan/protocol/goal.rs`、schema（`xagent schema doc/opendan/protocol/schema`）、fixture、Agent State Protocol §9、WebUI 的类型。默认允许 breaking change，不做旧字段兼容。
- 引入新依赖（时区库等）先确认。

## 6. 完成一项之后

- 在本单勾选，并在对应的原 TODO（异步单、默认 Goal 单）里记实施记录。
- 回写架构设计 §15.7：从“未实现”移到“已实现”，或改“与设计不同的地方”；涉及控制语义的回写 Session Control Protocol。
- 按 `AGENTS.md` 的 cmd_skills 联动表检查手册：Goal 建 Work 的方式、通知与 tell、定时相关的改动会影响 `opendan_agent_dev`、`ui_core`。

## 7. A 组实施记录（2026-11-10，未提交）

### 7.1 A1

- `jarvis/behaviors/goal.toml` 的 `create-worksession` 示例加了 `--behavior work_simple`，其余文字没有动。宿主侧不兜底（B4 待裁决）。
- 手册：`opendan_agent_dev`、`ui_core` 都没有描述 Goal 建 Work 的形状，不需要改。
- 验证：`cargo test -p opendan`（behavior 能解析）、`cargo test -p libopendan --test goal_session` 通过；本机 Zone 上真实模型建出的 Work 冻结的 behavior 是 `work_simple`，名下没有子会话。

### 7.2 A2

复现（`tests/goal_session.rs`，改之前的行为）：Goal 的 `scope.workspace` 目录被移走后，下一次运行机会让 Goal Session 以 `failed` 结束；Goal 仍是 `active`，Goal State 的 `last_error` 为空，没有 Activation 记录，详情页上看不到任何原因，只有 Loader 的日志和内存里的错误列表每个 tick 记一条“goal session … finished while the goal is active; restart the goal”。目录恢复后 restart 能恢复（新 epoch、新 Session）；目录没恢复就 restart，新 Session 建不出来，原因同样只在 Loader 的错误列表里。

决定：两种都做，按失效发生的时机分。

- Activation 进行中失效：这次 Activation 记为 `failed`，`end_reason` 是失效原因，用量照常计入周期。
- 没有 Activation 在进行（两次运行机会之间）：不造一条空的 Activation 记录，只把原因写进 Goal State 的 `last_error`。
- 两种情况下 `last_error` 都带 `kind = workspace_binding_invalid`、原来的绑定和一句 `recovery`（修好 Workspace 后 restart）。详情页本来就显示 `last_error`，不用改页面。restart 后第一次 Activation 的 Goal 头带着这条错误，Activation 无错完成后清掉。
- Goal 的状态不变，Session 仍然以 `failed` 结束（Workspace 绑定失效对任何 Session 都是终局），恢复靠 restart。

落点：`lib_opendan/runner/goal.rs::fail_session`；`runner/drive.rs::fail_workspace` 改为拿 `Shared`，三个调用点（drive 开始时的检查、运行中发现、drive 返回的错误）都走它，之后立即 `flush` 把 Goal State 和 Activation 记录发布出去。drive 开始时的检查原来在 `Shared` 建好之前就返回，现在推迟到建好之后（不绑定 runtime、不打开输入队列）。

用例：`a_lost_workspace_is_told_by_the_goal_state_and_restart_recovers`、`a_workspace_lost_mid_activation_closes_it_as_failed`。回归：`goal_session`（28）、`workspace_binding`（9）、`fixtures`（19）、`memory_maintenance`（12）、`session_control`（8）、`cargo test -p opendan` 全部通过。

没有做的：

- 宿主每个 tick 仍然为这个 Goal 报一次“session finished while the goal is active”（日志和 Loader 的错误列表），没有去重。
- Workspace 没恢复就 restart 时，新 Session 建不出来的原因只在 Loader 的错误列表里，不在 Goal 页面上（带状态 restart 时旧的 `last_error` 还在，说的是同一件事）。
- 失效前已经投进队列的运行机会随 Session 结束而丢掉，不会转给 restart 之后的 Session。
- 真实 Zone 上没有验这一条（需要弄坏一个真实的 Workspace），只有集成测试。

### 7.3 A3

证据和逐条结论在 `notepads/evidence/goal-session-next/2026-11-10/README.md`。四条都通过：整理 Goal 播种一次、无材料不调用模型、3 次由定时触发的真实整理；WebUI 经网关登录后的休眠、唤醒、restart、归档、reopen，内置 Goal 不能归档；Activation 进行中置休眠在下一个推理边界收尾、Work 随之暂停并在唤醒后恢复、Session 级 stop 等于休眠；结晶 Goal 对真实模型跑了一次（读了 6 个 Work，`no_action`，没有改写任何手册）。

发现的问题，都没有处理：

1. 被停掉的 Activation 留下的任务结束时，唤醒后打开新的 Activation。就是 B6，真实 Zone 上确认了。
2. Work 的 `needs_input`（`work_simple` 的 Work 在等自己的后台任务时关闭 Turn）也给 Goal 一次运行机会，Goal 为此读一遍没有新内容的报告。没有 TODO 承接，建议并入 B6 或 B5 一起看：Goal 只该为 Work 的结束和真正要人回答的问题醒来。
3. Work 自己的任务结束事件（`task:local:shell:…:terminal`）出现在 Goal 的 Activation 原因里。来源没有查。
4. 成本：Goal 里每跟一条长命令就带着整段上下文推理一次，测试 Goal 三次 Activation 用了 37.8 万 token；结晶 Goal 一次 `no_action` 用了 18.2 万 token。前者和 B1 有关，后者和 E5 有关（打开开关之前要让 owner 知道一次的量级）。
5. 结晶 Goal 的开关关掉以后，Goal 仍显示为 `active`（gate 是 `disabled`），Session 留着。和 E5（N8）一起定页面上怎么显示。

没有验的：手机宽度与真机；结晶 Goal 的“修订手册”和“写草稿”路径（这次材料不够，没有触发）。

留在本机 Zone 的：两个已归档的测试 Goal、打开开关时播种的内置结晶 Goal（开关已还原）；Zone 运行的是包含 A1、A2 改动的构建。

## 8. 触发门槛的裁决记录（2026-11-10，owner）

owner 用一张脑图（“goal 怎么触发推理？定义门槛”）给出四道门槛，随后在讨论里答复了整理者提的问题。引号里是原话，其后是它对应的条目。

1. 脑图的四个节点：
   - “用户配置：是否有硬保障（多少时间至少推理一次），默认 72 小时。”
   - “用户配置：硬间隔，两次执行之前不能小于时间（默认 15 分钟）。”
   - “用户配置：预算控制，预算用完不推理，除非到达硬保障。默认为 24 小时 2 百万（输入加输出）。”
   - “用户配置 + Agent 可调节计划任务（timer）拉起（频度可高一点）。自定义检查：脚本返回 true 则推理，异常不推理。这个脚本 Agent 可以自己写。”
   - 对应 H 组开头的表。脑图里的 timer 指 Goal 的检查时间表，不是异步单里“到点不推理的 timer 对象”（D4），文档里叫“检查时间表”。
2. “硬保障是如果距离上次推理超过这个时间，无论如何都推理一次（防止 agent 把自己的通过脚本写崩了没地方修）。相当于正常逻辑主要是 timer + 脚本检查，偶尔推理被硬间隔或预算拦下，极端情况触发保障。” → H2 的顺序、H4。
3. “脚本在 agent 的私有容器里的 deno 跑，只能写 ts。” → H3。
4. “不给 run 和写（现在的特化的检查是否有感知的逻辑就能移动过去了），系统自带的默认 goal 也要这么改造。” → H3 的权限、H6。
5. “我觉得现代 agent 也一定会写完后测试的，不用加机制。”针对的是整理者提的两层保护：写入脚本时宿主先试跑、脚本出错后给一次修复推理。两层都不做 → H3。
6. “现在有最小间隔 + 预算保护，可以唤醒。”针对的是 Work 结束、用户转来的消息、问题得到回答这三种事件要不要直接唤醒 Goal → H2。
   - 整理者的读法：事件可以唤醒，同样过硬间隔和预算，被拦下的留在队列里。如果本意是事件不受硬间隔限制，只改 H2 第 2 步对事件的适用。
   - 按这个读法，一个要连续派 4 个 Work 的 Goal 在缺省的 15 分钟硬间隔下至少要 1 小时；要快的 Goal 由用户把它的硬间隔调小。
7. “算。用户的视角就是‘背地里就给你这么多 token 干活’。这个信息要传递给 agent，让他能有机会根据预算使用情况调整四路。”针对的是派生 Work 的 token 算不算进预算 → H5。
   - 整理者的读法：“四路”指第四道门槛，也就是检查的时间表和脚本。
   - 同一个问题里还问了要不要 Agent 的总量上限，下一轮答了，见第 8 条。
8. “好的。可以加个 Agent 总量。”（N9）→ H1、H2、H5。缺省值没有给。
9. “好。”（N10：预算用完时正在跑的 Work 在推理边界暂停，周期重置后恢复）→ H5。
10. “这只是初始状态，要允许默认 goal 也会自我演化。而且 goal 是珍贵的资源，不应该强迫用户分成多个。”（N11，针对整理者的建议“内置 Goal 的脚本随包、Agent 不能换、硬保障关闭”）→ H6。
    - 整理者的展开：既然内置 Goal 能改自己的脚本，硬保障就要对它打开，“无材料不调用模型”变成“无材料时只有硬保障那一次”；包升级不覆盖它改过的脚本。这两点 owner 没有单独说。
    - 后一句是一条更宽的原则，不只管 N11：凡是靠多建 Goal 来分摊职责的方案都要按它重看，已知的有 E3（日程管家作为第二个内置 Goal）和异步单的 Q-S2（每个例行任务一个 Goal）。这是整理者的理解，这两处 owner 还没有裁决。
11. “好。”（N12）→ H1。
12. “好。”（N13）→ D2、T06 取消，D6 不排期。

第 6、7 条里整理者的两个读法，在给 owner 的回复里单独列出并请他纠正，owner 下一轮没有提出异议。

整理者的实施建议，owner 没有单独裁决，实施前有异议可以改：

- 闸门的先后（H2），其中休眠压过硬保障、手动激活不过门槛。
- 用标准层的 Deno，不用扩展包里的（H3）。
- 脚本约定成函数，`state` 由宿主保存并传回（H3）。
- 提供一条按宿主同样方式试跑的命令（H3）。
- 条件一直为真时宿主不兜底，靠硬间隔、预算和 H5 的信息让 Goal 自己发现（H3）。
- 不需要模型的维护（清理重试、未完成回执）留在宿主（H6）。

## 9. 结果交付与其余问题（2026-11-10，owner）

1. “已有问题我看基本可以按你的建议走。” → §3.1、§3.2 的建议即结论；Agent 总量的缺省按建议取 500 万。与下面三条冲突的建议不采用，已在 §3.1 标出。
2. Goal 的结果交付，owner 的三点：
   1. “通知用户，确定命令行后，实现可以调整。” → C 组改为先定命令行（C0），default chat session 等是命令背后的实现。
   2. “修改用户的 todolist / 日历 / notebook 这些接入后也是命令而已，现在不支持就没有这些交付方法。” → D4、E2、E3 等接入；E1 的范围见 §3.4 第 2 项。
   3. “buckyos 的特性：agent 可以为了一个 goal 做一个 app（URL），这个 app 可以是标准的 ts 写的，也可以是基于 aiworkspace 的。不过这都属于 goal skill 的部分。” → F2；C0 的 `--url` 是把人带到这个 app 的手段。

整理者的理解，owner 没有单独说：

- 交付方法都是命令，所以不为交付在 Goal 的结果里加字段（`result.tell` 不做）；`questions` 是状态不是交付，保留在结果里。
- 第 2.2 条让 Q-S 一组问题（timer 对象、日程管家、定时意图里的固定提醒）一起搁置，不是按原建议实施。
- 第 2.2 条说的 notebook 是用户的笔记（世界对象），不是 N1 要删的 Agent Notebook。

§3.4 四项的答复（同日）：

3. “按你的建议。”（通知命令行）→ C0。
4. “这就是我纠结的地方，整体推后比较好。”（E1）→ E1。
5. “按你的建议，我觉得从范式上来说，goal 的早期运行应该为后面的重复交付准备好必要的工具。”（首批 goal skill）→ F2。
   - 整理者的展开：这条范式不只写进两份 goal skill，也写进 `goal.toml`；准备好的工具要在 Goal 头里看得到，restart 时要保留。后两点 owner 没有单独说。
6. “按你的建议。”（压缩等数据）→ F3。

## 10. H、B、C0、D1 及其余的实施记录（2026-11-11，未提交）

按 H → B → C0 → D1 → 其余的顺序实施。没做的：D3（workflow 计划后端）、F3（等数据）、按裁决推后的 D4、E1、E2、E3，D5 的固定提醒那一半。真实 Zone 上的 DV 情况见 §10.6。

### 10.1 H 组

- **协议**（`protocol/goal.rs`）：`GoalScheduleSpec` 加 `cron`、`timezone`、`check`（`GoalCheckScript{version, sha256, source, package, updated_at_ms}`）、`guarantee_secs`；`min_interval_secs` 含义改为对所有触发生效。`ActivationBudget.max_wall_secs`。`GoalState` 加 `consecutive_no_action`、`holding`、`holds`。`GoalSchedule` 去掉 `gate` 和 `every_secs`，加 `timetable`、`last_check`、`check_errors`、`last_check_error`、`check_state`、`stats`、`guarantee_posted_for_ms`、`switched_off`。`ActivationReasonKind::Guarantee`，`PauseSource::Budget`。`MaintenanceGate` 删除，`MemoryStatus` 新增。
- **缺省**（`template.rs`、Jarvis `agent.toml`）：硬间隔 900 秒、硬保障 259200 秒、检查时间表 300 秒（下限 60）、周期 24 小时 200 万 token、Agent 总量 `[session.goal.agent_budget]` 24 小时 500 万、Activation 墙钟 1800 秒。`[session.goal.maintenance] decision` 删除。
- **H0**：Activation 关闭时清掉 `watched_tasks`（`runner/goal.rs::close_activation`，`outcome.rs` 的 `commit_run_end` / `commit_kept` 不再在 Turn 关闭后把任务加回来），残留任务的结束不打开 Activation（B6）。§7.3 第 3 条的来源查清了：Goal 的工具结果里出现过 Work 的任务 id（`note_task`），run 结束后被当成自己的任务跟着；同一处修掉。Goal Session 的子 Session 只有收消息（`input_policy != none` 且 `wait_user_msg = allowed`）时它的 `needs_input` 才唤醒 Goal（`children.rs::answerable`）。
- **H1 / H2**：闸门在 `goal.rs::gate`（休眠 → 硬间隔 → 硬保障 → 周期预算 / Agent 总量），检查只对时间表（宿主 `host/goals.rs::scheduled`）。事件在 Goal Session 自己的驱动里过闸门（`runner/goal.rs::hold_events`，`drive.rs` 选好批次后调用）：被拦下的不消费，下一次驱动再看，`holding` 只在开始拦截时记一次；手动激活、定时检查、硬保障、epoch 开始不过这道闸。Goal Session 改检查脚本和时间表走新的 `GoalStore::set_check`（kRPC `goals.set_check`，CLI `goal check set`），记 `changes.jsonl` 的 `kind = check`，不增加内容修订号；脚本存 `checks/<n>.ts`。owner 的 `revise` 改 `schedule` 时保留 `schedule.check`。
- **H3**：`goal_check.rs` + `goal_check_runner.ts`（`include_str!` 进二进制，写到 `<AgentRoot>/.cache/goal_checks/`）。Deno 只给 `--allow-read --allow-net --allow-env`，30 秒超时，日志和 `state` 各 64 KiB 上限。Deno 的查找顺序：Agent 镜像的 `$BUCKYOS_ROOT/bin/opendan/deno`（容器里只有这一个）、宿主安装的 `libexec/buckyos-tool/runtime/deno`、程序旁边、`PATH`。`ctx.host.call` 是宿主经脚本的标准输入输出提供的只读调用（`memory.status`）。CLI `goal check run [--script]` 按宿主同样的方式试跑，不改任何状态；`goal check show` 看脚本、时间表和最近的检查。
- **H4**：宿主每个 tick 看硬保障，每个 quiet 段（按“上次推理或 epoch 开始”计）只投一次 `guarantee`，原因里带最近一次检查、连续出错次数和最后一次错误。
- **H5**：周期用量 = 这个 Goal 所有 Session（各 epoch 的 Goal Session、Work 及其子 Session）`usage.jsonl` 里落在本周期的 token（`goal.rs::period_usage`），`work_admission` 和闸门都用它，另判 Agent 总量（`agent_usage`）。预算用完时宿主（`sync_budget_work`，每个 Goal 最多 30 秒看一次）向运行中的 Work 投 `pause`（`reason = goal <id> used up its budget of the period`，来源 `budget`），周期重置后只恢复这一来源的。Goal 头的 `<budget>` 加自己与 Work 分开的用量、重置时刻、墙钟上限、Agent 总量；新的 `<gate>` 列硬间隔、硬保障、检查时间表与脚本版本、本周期检查与拦截的次数、最近一次检查、连续出错、连续 `no_action`。`goal.toml` 写明预算的含义和怎么调检查。
- **H6**：`host/maintenance.rs::decide` 和 `schedule.json.gate` 删除，判断搬进 `goal_checks/memory_maintenance.ts`（随 Jarvis 包分发）；结晶 Goal 加 `goal_checks/skill_crystallize.ts`（使用记录里新增 3 个 Work，或新增且满一周）。播种时用 `set_check` 写成 Goal 的脚本（`source = package:<path>`）；以后播种时只在 Goal 还用包内版本且包里的变了时替换。owner 可 `restore_package`。机械维护由宿主在每个检查槽 drive 一次。AgentRoot 布局加 `goal_checks/`。
- **H7**：文档见 §10.5；页面见 §10.4。

### 10.2 B、C0、D1、D5、E、F、G

- **B1**：`max_wall_secs`；到点后不再开 run 段，以 `budget_exhausted` 关闭；Goal 的 report 被拒（还有自己的任务在跑）时文案另写（`reports.rs`）。`goal.toml` 加了长命令交给 Work 的说明。
- **B2**：Goal Session 的 bin 里放 `ask_user` 的 tombstone（`runtime/mod.rs::bin_plan_with`），提示改用 `questions`。
- **B4**：`api.rs::create_sub_session`：父是 Goal Session、没给 `--behavior` 且类缺省有 behavior 时取 `work_simple`。
- **B5**：Goal 头放在输入消息开头，同一条 User 消息；历史里替换成一行（`ReceiptPart.brief`）。`<works>` 只列未结束的 Work。`PART_GOAL` 删除。fixture `16_goal_session` 重新生成（只替换这一个）。
- **C0**：`notify send --url <link>`（写进 `NotifySource.link`，只接受 `https://`、`http://` 或 `/` 开头），帮助与 `goal.toml` 写明语义。
- **D1**：`goal_time.rs`（5 段 cron，`chrono-tz` 已是依赖，没有新增依赖）；`Timetable` 统一 every / cron，`LocalScheduleBackend` 按墙上时间计槽（槽 = 匹配分钟的 UTC 分钟数）；`goal create / revise / check set` 加 `--cron --timezone`，没给时区时取调用会话的用户时区，再缺省 UTC。
- **D5（一半）**：`analyze_intent.toml`：到点要判断的请求建带时间表的 Goal；固定提醒仍 `unsupported`。
- **E4**：删除 `SessionKind::SelfCheck`、内建模板 `self_check`、`[loader.self_check]`、`behaviors/self_check.toml`；Agent Notebook 一并删除（`agent_tool::agent_notebook`、`agent_tool_cli_dev` 的 `agent-notebook` 子命令、`cognition.notebook_append` 与 kRPC、`xagent note`、AgentRoot 的 `notebook/` 目录）。
- **E5**：开关放进 owner 的 `.meta/skills_state.json`（`crystallize`），kRPC `skills.crystallize_set {enabled}`（owner 专用），覆盖 `agent.toml` 的值；关掉的内置 Goal 在 `schedule.json.switched_off` 标出。
- **F1**：Goal 头的 `<current_skills>` 给出每个 skill 正文可读的路径（`goal.rs::skill_path`）；`goal create --skill <name>` 和 `goal skills add <name>` 不带 ref 时取已安装 goal skill 的路径。restart（`facts`）保留 Goal 自己加的 skills。
- **F2**：Jarvis 包加 `goal_skills/goal_check_script`、`goal_skills/goal_delivery`；`goal.toml` 加“早期运行准备工具、之后便宜”的范式和 goal skill 的用法。
- **G1**：`api.rs::check_ready`，UI 和 Goal 两个入口都走它：任务书不能为空；Goal 的 Work 另有两条（N4 原文只针对 Goal）：同一个 Goal 下不能再建任务书相同（规范化空白和大小写后相同）且未结束的 Work，任务书不能等于 Goal 的 objective。最初对话入口也查重，xagent_lib / xagent_cli 里四个用例同一父会话下建同名子会话被拦，收回到只对 Goal 查。
- **G2**：见 §10.4。

### 10.3 与本单方案不同的地方

1. 硬保障字段为空表示取 Agent 的缺省（72 小时），`0` 才是关闭（本单写的是“空表示关”）：和其它字段的“空取缺省”一致。
2. 检查脚本拿数据的“只读 kRPC”做成两处：宿主经脚本的标准输入输出提供 `memory.status`（脚本不需要网络和身份就能拿到），同名的 Agent State kRPC 也加了（页面、Goal Session 可以看）。
3. 检查脚本得到 `--allow-env`（websdk 要靠环境变量里的身份），本单只写了读和网络。
4. Goal Session 改检查用 `GoalStore::set_check`（goal.json 的锁下 CAS 写），不是经驱动者的控制输入；单写者靠 GoalStore 的锁保证。
5. B5 没有把 Goal 头做成输入模板变量，而是放在模板输出的前面（同一条消息）。现在没有哪个 behavior 需要把它放到别处。
6. 没有材料、没有时间表也没有脚本的普通 Goal 只会被事件、手动和硬保障唤醒；有时间表没有脚本的 Goal 每个槽都推理（受硬间隔和预算封顶），与本单“条件一直为真时宿主不兜底”一致。
7. 带检查脚本、不是 `--start` 创建的 Goal，epoch 开始时先跑一次检查；没有脚本的 reopen / restart 照旧直接投运行机会。

### 10.4 页面（H7、G2、E5）

`src/frame/opendan/web`（`model/{types,datamodel,krpc,mock}.ts`、`goals.ts`、`lib.ts`、`i18n.ts`、`runtimes.ts`、`pages/{GoalDetail,Goals,Skills,SessionDetail,Home}.tsx`、e2e `goals.spec.ts`、`skills.spec.ts`、`mock.spec.ts`、`sessions.spec.ts`、`home.spec.ts`）：

- Goals 列表：关掉的内置 Goal 显示“已关闭”，排在休眠之后、不计入活动数；“下一次机会”带上次检查的结果，cron 的显示表达式和时区；活动 Goal 超过 16 个时提示 Goal 是珍贵的资源、可以并进已有的 Goal。
- 详情页的门槛面板：硬保障（时长 / 缺省 / 关）、硬间隔、周期预算（已用 / 上限 / 剩余，Goal 与 Work 分开，Work 数，窗口与重置时刻）、检查（时间表、脚本版本与来源和包内路径、上次检查、连续出错与最后一次错误、本周期的检查与拦截次数、硬保障次数、连续 `no_action`、正在被拦的事件）；“查看检查脚本”“恢复包内脚本”“编辑预算与时间表”。旧的 gate 显示删除，整理回执保留。
- 编辑表单（N5 (b)）：预算、硬间隔、硬保障经 `goals.revise`（带打开时的修订号，冲突时提示并重载），检查时间表经 `goals.set_check`；`every_secs` 小于 60 在页面上就拒绝。清空时间表走 `revise`（`set_check` 不能清空 `every_secs` / `cron`）。
- 修订差异：与上一修订逐字段对比（12 个字段，只列不同的）。成本：本周期用量与最近 7 天按天的 token。
- Skills 页：经验结晶的开关（说明它做什么、一次约 18 万 token、当前值来自包缺省还是 owner）。
- 暂停来源 `budget` 的文字；self_check 从会话类型、mock 和用例里删除。
- 已知的不足：页面上的缺省值（15 分钟、3 天、200 万、5 分钟）是写死的，Agent 在 `agent.toml` 里改了缺省时显示不对；列表每次刷新多一个请求（为了知道哪些内置 Goal 关掉了）；修订冲突的路径 mock 测不到。
- 验证：`pnpm check`、`pnpm lint` 通过，`pnpm test:e2e` 56 个全部通过（mock，Chromium，含一组 390×844 手机宽度不横向滚动的检查）。真实 Zone、真机没有验。

### 10.5 文档

- [Agent State Protocol](<../doc/opendan/protocol/Agent State Protocol.md>) §9（文件与字段、§9.4 改写为预算与门槛、权限、kRPC、§9.7 / §9.8）；[Session Control Protocol](<../doc/opendan/protocol/Session Control Protocol.md>) 加暂停来源 `budget`、去掉 self_check；Session Directory Protocol、xAgent.md 去掉 self_check；`build-in agent-tool手册.md` 的 agent-notebook 一节标为已删除。
- [架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>) §12.1.1 标为已实施并写了实现上的取舍，§12.2、§15.3、§15.5、§15.7、§17 的相关行同步。
- 手册：`opendan_agent_dev` 写了 `[session.goal]` 的新键与检查脚本，删了 `notebook/`；`ui_core` 不涉及（它不描述 Goal 的门槛和定时，`notify send` 也不在 UI 手册里）。异步单 §8.1、默认 Goal 单 §11.8 记了落点。

### 10.6 验证

- `cargo test -p libopendan -p opendan -p agent_tool --no-fail-fast -- --test-threads=1`：goal_session 37、memory_maintenance 13、fixtures 19、memory_replay、session_control、notify、workspace_binding、xagent_lib、loader 等全部通过。失败的只有环境问题：`agent_tool` 的 3 个 grep 用例和 `agent_tool_cli_dev` 的 1 个（本机没有 `rg`），`xagent_cli` 的 2 个（`src/tools/bin` 不存在，恢复被拦，早已存在）。
- 新增用例：`goal_session.rs` 的 `the_check_script_decides_scheduled_inferences_and_keeps_its_state`、`the_hard_interval_holds_events_and_merges_them_into_one_activation`、`the_hard_guarantee_wakes_a_goal_past_budget_and_check_but_not_a_dormant_one`、`the_agent_total_holds_every_goal_but_their_guarantees`、`a_closed_activation_lets_go_of_its_tasks`、`checks_run_on_deno_with_read_and_network_only`（真实 Deno：执行命令、写文件被拒，`state` 往返、超时、返回值不合约定）、`the_period_limit_skips_fires_pauses_work_and_the_next_period_resumes_it`（改写原 V07）、`work_is_refused_when_it_repeats_running_work_or_the_whole_goal`、`a_cron_timetable_fires_on_the_wall_clock_of_its_zone`、`goal_skills_are_readable_from_the_header_and_survive_a_restart`；`memory_maintenance.rs` 迁到检查脚本（真实 Deno 跑包里的脚本），加 `without_material_only_the_hard_guarantee_infers` 和包升级 / 自改 / 恢复的断言；`goal_time.rs` 3 个单元测试。
- `uv run buckyos-build.py` 通过；`./build_aios --local-test` 通过。WebUI 见 §10.4。
- 真实 Zone DV（证据 `notepads/evidence/goal-session-next/2026-11-11/README.md`）：升级后内置整理 Goal 拿到包里的脚本、在容器的 Deno 上 6 个多小时 14 次检查无误且没有调用模型；测试 Goal 检查为真才推理；脚本写坏后静默到硬保障、由 Goal 自己修好；页面经网关在 390 / 1280 宽度显示门槛。
- 没有验的：事件被硬间隔 / 预算拦下再合并（只有集成测试）、预算用完时 Work 的暂停与恢复（只有集成测试）、cron 时间表（只有集成测试）、页面的编辑表单与结晶开关的提交、手机真机、Workspace 失效。DV 之后的两处小修（硬保障原因里的时长写法、过滤 Deno 的 TLS 警告）没有重新部署。

DV 发现：**硬保障压过预算，设得短时会无视预算持续推理**。测试 Goal 的硬保障设成 600 秒，环境闲置约 6.5 小时里被唤醒 38 次，周期预算 150 万用完后照样触发，共用了约 220 万 token（DV 参数不当所致）。2026-11-11 owner 裁决：**硬保障下限 1 小时**。已实施：`GUARANTEE_MIN_SECS = 3600`，`schedule.guarantee_secs` 只能是 0（关）或不小于 3600，创建、修订（`state/goals.rs::check_schedule`）和 `agent.toml [session.goal] guarantee_secs`（`template.rs`）都拒绝更小的值，运行时对已存的更小值按 3600 算（`goal.rs::guarantee_secs`）；页面表单同样校验。用例加在 `the_hard_guarantee_wakes_a_goal_past_budget_and_check_but_not_a_dormant_one` 和 e2e `goals.spec.ts`。

F3 的数据：同一个 Goal 连续 `no_action` 时每次 Activation 从 2.5 万涨到 7.8 万 token（历史每次约长 2k），第 37 次触发通用压缩后回落到 5.9 万。

### 10.7 固定提醒由 Goal 承担（2026-11-11，owner 追加，未提交）

起因：owner 在 DV 环境对 Jarvis 说“工作日的下午3点-6点之间，每50分钟提醒我站起来活动10分钟”，得到“目前我还不支持自动发送固定时间的重复”。原话：“按道理goal session可以很容易支持这种主动通知逻辑”。拒绝来自 `analyze_intent.toml` 里 D5 留下的那一行（固定提醒 → `unsupported`，等 D4）。

做法（整理者的选择，owner 没有单独裁决）：意图分析为提醒建 Goal；Goal 第一次 Activation 把提醒写进检查脚本；检查结果加一个 `notify` 字段，到点时由宿主发通知，不打开 Activation。没有选“每次提醒推理一次、由 Goal 调 `notify send`”：DV 量到一次 `no_action` 的 Activation 就要 2.5 万到 7.8 万 token，例子里一周 20 次提醒会用掉百万级 token，也不符合“早期运行为重复交付准备工具”（§9 第 5 条）。

- **检查结果**：`{run, reason?, state?, notify?}`，`notify = {title, message?, url?, level?}`（`goal_check.rs::CheckNotify`，`CheckVerdict::validate` 限制标题 200、正文 4000 字符，`url` 同 `notify send --url`）。宿主以这个 Goal 的名义发 `notice`（来源带 Goal Session、`goal_id`、Goal 标题），幂等键是运行机会的 key 加 `:notify`。发不出去算这次检查出错，`state` 不保存，下一次检查再决定。
- **被拦下的检查槽也跑检查**（改了 H2 的做法）：硬间隔、预算拦下一个槽，或硬保障占了这个槽时，有脚本的 Goal 照样跑检查，`ctx.held` 是原因。`run: false` 的结果照常生效；`run: true` 的结果整个搁置（不发通知，`state` 不变），下一次检查会再看到。不这样做的话，刚创建的提醒 Goal 在硬间隔（缺省 15 分钟）内的提醒会丢，Agent 后台总量被别的 Goal 用完时提醒也会停。槽本身仍然跳过、不补，计数不变。
- `ctx` 加 `held`、`timezone`（Goal 的 `schedule.timezone`）。`CheckRecord` 加 `notified`（发出的通知标题）、`held`；`GateStats` 加 `notified`。Goal 头 `<this_period>` 加 `notifications_by_checks`，`<last_check>` 带 `held` / `notified`。`goal check run` 的输出带 `notify`（只显示，不发）。
- **提示词与手册**：`analyze_intent.toml` 把提醒交给 Goal（目标里写清每个时刻、时区和内容，`--cron` 覆盖所有提醒时刻，一次性的用 achieve），不再 `unsupported`；`goal.toml` 写明到点要说的话放进检查脚本；`goal_check_script` 加“Reminders at known times”一节和一个跑过的例子（工作日 15:00 / 15:50 / 16:40 / 17:30，晚到 15 分钟内仍补发，重复检查不重发）；`goal_delivery` 加一条。
- **文档**：Agent State Protocol §9.4；架构设计 §8.1（实现现状）、§12.1.1（检查只观察，但可以让宿主发通知）、§17 表；《Agent 主动给用户发“消息”的方法总结》§11.2 记了实现现状；schema 重新生成（`goal_schedule`）。
- **页面**：门槛面板的本周期计数加“检查发出的通知”，最近检查显示被拦下与已发通知。
- **用例**：`goal_session.rs` 新增 `a_check_sends_reminders_without_inference_even_while_held`（硬间隔内照发、被拦下的 `true` 整个搁置、放行后随 Activation 发出、没有通知服务时算出错且 `state` 不变）；`checks_run_on_deno_with_read_and_network_only` 加真实 Deno 下 `notify` 的往返与三种不合约定。goal_session 38、memory_maintenance 13、fixtures 19 通过；WebUI check、lint、e2e 56 通过。`cargo test -p libopendan -p opendan` 并行跑时 `plan_simple_async` 5 个、`runtimes`、`session_runtime`、库内 `concurrent_reservations…`、`loader` 各 1 个失败，单线程（`--test-threads=1`）重跑都通过；`plan_simple_async` 在去掉这次改动的 HEAD 上并行跑同样失败，是用例之间的隔离问题，与本项无关。`xagent_cli` 的 2 个仍是 `src/tools/bin` 不存在。
- **DV（2026-11-11，owner 部署后自己发的提醒请求）**：意图分析建了 Goal `g-k0b915a12dc76c2022c6052cf`（maintain，`--cron "*/10 12-17 * * 1-5"`，America/Los_Angeles，提醒时刻 12:00 起每 50 分钟到 17:00）。创建时的一次 Activation 用了 92,412 token：按 `goal_check_script` 写脚本、模拟 10,080 分钟、宿主试跑两次、`goal check set` 安装。之后每 10 分钟跑一次检查，每次约 18 ms：13:50、14:00 两个槽在硬间隔内（`held=min_interval`）照跑；14:30:00.84 PST 检查返回 `notify`，宿主发出 `nt-738aa9a3c34c459587222bc1938bf960`。到 14:40 共检查 6 次，发通知 1 次，出错 0 次，创建后没有再推理。
  - DV 发现的问题：Agent 写的脚本只在整分钟命中时发，注释写着“never backfill”，没有照手册例子留 15 分钟的晚到窗口。`ctx.now_ms` 是检查实际运行的时刻，不是槽的时刻；宿主多个槽合并成一次触发（`missed > 1`）时同样如此。所以宿主在提醒那一分钟重启或延迟时，这次提醒会静默丢掉。可选的修法：`ctx` 带上槽的时刻，或在手册里把晚到窗口写成必须。待 owner 定。
- 没有做 / 没有验：通知的呈现仍是系统通知（default chat session 的口吻转述是 C 组后半，未做）；检查脚本写坏时提醒会静默停到硬保障（缺省 72 小时）才被 Goal 发现，对提醒类 Goal 偏长，是否要“连续出错 N 次就唤醒一次”待 owner 定（§8 第 5 条原先说不加机制）。

