# BuckyOS Agent UI 升级 TODO

**2026-11-10 晚：本单已按下文实施（基线 `7c91504b6`，未提交，真实 Zone 未验）。逐项结果、与本文不同的决定、没做与没验的在 §8 最后两条，任务状态在 §4.0，接口的实际形状在 §3 的状态列。§2.1 的推定 13 的两处修正、推定 15、16 是按建议实施的，owner 随后确认都按建议（D30）；§2.1、§2.3–§2.5、§5、§7 里这三处的“待确认”“等过目”字样是确认之前写的。§1 的“现状核对”和 §2.3、§2.4 里“待实施”“今天”的说法是实施之前的，留作对照。**

状态：2026-11-09 起草；同日 owner 在文中批注了 D1–D16，已整理成 §2 并回写任务。**2026-11-10 按提交 `27aea9efb`（工作树干净）复核并更新了本文；同日 owner 批注了 §2.1，整理为 D17–D20 并回写。** 现在是 38 项任务：做完的只有 RT1 的后端（随 runtime TODO 一起做的）；RT2、RT3、RT4、DX4 各做了一部分；SK1 按 D19 移出本单，另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)；SC0（所有 Session 带输入队列，D20）、SC9（聊天命令 `/clean` 等，D18）、SC10（撤回输入，D25）、SC11（重启后的异常中断与手工恢复，D27）是新增的；其余都没有实施。逐项状态见 §4.0。整理 D17–D20 时的七条推定，owner 同日答复：stop 要分“针对消息的”和“针对 Session 的”两种，Goal Session 的手工 disable 叫休眠（D21，表在 §2.3），其余按建议（D22）。展开 D21 时的四条推定（§2.1 第 8–11 条），owner 同日在文中逐条批注，都按建议（D29）。**同日 owner 又从用户角度给了一份有效语义（§2.5），并答复了由它引出的五个问题，整理为 D23–D27：前台消息不要暂停、后台 Work 里只有 plan 收消息、撤回输入按建议、Goal 休眠后不发起下一次推理、重启后一律手工恢复。** 展开时的三条推定（§2.1 第 12–14 条）owner 同日确认，并定了本单的范围：自动恢复是下一期的，这一期把手动控制的语义做对（D28）。据此按代码把手动控制逐项对了一遍，补了几处漏洞：第 13 条的判定有两处修正，另有两条新的推定（第 15、16 条），都在 §2.1 等过目。依据 [BuckyOS Agent UI 升级规划](<BuckyOS Agent UI 升级规划.md>)（下称“规划”，11-09 之后没有改过）逐节对照源码写成。任务后面的（Dn）表示它按哪条裁决做。

范围：Agent 首页（OpenDAN WebUI）与聊天侧的 Session 侧面板（Desktop MessageHub），以及这两处界面需要的后端接口（libopendan、opendan、control_panel、msg_center）。目标管理（规划 §5）本轮沿用现有 UI，不在任务里。

读法：

- 路径相对 `buckyos/`。§1 的事实于 2026-11-10 按 `27aea9efb` 复核；位置以符号名为准，实施前重读。
- 基线是提交 `27aea9efb`。起草时还没提交的 MessageHub UI 优化已在 `dd13b0c70`；Skills 体系在 `174dc06f7`；runtime 生命周期在 `35e72ce06`、`b6c57704a`、`27aea9efb`。
- 按 `AGENTS.md`：不做旧版本兼容；改协议、字段、存储结构时同步检查 `buckyos-api` 共享类型、前后端和文档；改 UI DataModel 的 TS 文件触发 `harness/trigger_rules.md` §3.4 的追加检查。
- 两份关联 TODO 与规划重叠，按裁决各管各的部分，现在都已实施并提交：[Skills 体系改进计划](opendan-skills-system-todo.md)（下称 skills TODO，状态在它的 §0）交付了 Skills 的分组、禁用、安装与导入，以及 opendan WebUI 的 Skills 页（D5–D7）；[Runtime 工作模型与生命周期](opendan-agent-runtime-lifecycle-todo.md)（下称 runtime TODO，实施与 DV 记录在它的 §11、§12）交付了设备目录与分配（D4），并连带做了本单 RT1 的后端。Runtime 在本单里只剩 Agent 首页一侧的界面（§4.3）；Skills 页与规划的形态差距按 D19 另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)，本单不做（§4.4）。气泡下的 Work Session 卡片见 [done/messagehub-worksession-ui-todo.md](done/messagehub-worksession-ui-todo.md)。

## 0. 术语对照

规划里的词在代码里叫什么。实施和写测试时用右边的名字。

| 规划用词 | 代码里的对应 | 位置 |
| --- | --- | --- |
| Agent 首页 | OpenDAN WebUI 的 Home（每个 Agent 一份，由该 Agent 的 opendan 进程提供） | `src/frame/opendan/web/src/pages/Home.tsx`；数据经 kRPC `/api/opendan`（同一服务在 Zone 内是 `/kapi/<agent_id>`） |
| Session 侧面板 | MessageHub 的详情栏：`detailsTarget` 切换的 `EntityDetails` / `SessionDetails` | `src/frame/desktop/src/app/messagehub/MessageHubView.tsx` |
| 感知 | `PerceptionRecord`，按写入会话分文件 | `<agent_root>/state/perception/<sid>.jsonl`；`lib_opendan/src/protocol/agent_state.rs` |
| 认知 | `MemoryItem`（Memory Graph，全 Agent 一份） | `<agent_root>/memory/`；`agent_tool/src/agent_memory/model.rs` |
| 整理 | 内置 Goal `memory_maintenance` 的 consolidation | `lib_opendan/src/host/maintenance.rs`、`runner/consolidation.rs` |
| Provider Runtime | 内置私有 runtime `private`（旧名 `container` / `local`），永远启用。带容器的产品上它就是 Agent 的容器（Linux 恒是）；原生运行只在 Lite | `PRIVATE_RUNTIME`，`lib_opendan/src/protocol/runtime.rs` |
| Owner 的 Default Runtime | owner default runtime：`AgentSettings.default_runtime_id`，没设就是私有。不是一台特殊的 `host`；settings 里没有的 runtime 不投影 | `src/kernel/buckyos-api/src/app_schema.rs` |
| 分配给 Agent 的 Runtime | `AgentSettings.runtimes` 里的条目 `{enabled, target}`（`Node` 或 `Executor` 目标）。权限、ComputerUse、基线、期限是 runtime 自己的配置，不在条目里 | 同上；投影成 `RuntimeRecord` 在 `protocol/runtime.rs` 的 `builtin_runtimes` |
| CMD Skill | work skill：`cmd_skills/<name>/SKILL.md`（+ 可选 `references/`），`SkillKind::Work`。纯文本，不带命令；依赖的命令写在 front matter 的 `commands`，由扩展包提供 | `lib_opendan/src/skills.rs`、`protocol/behavior.rs` |
| UI Skills | `scope` 含 `ui` 的 work skill。新对话取哪几份，缺省是 `agent.toml` 的 `[session.ui]` / `[session.group]` `default_cmd_skills`（模板里只有 `ui_core`），owner 可以改（`skills.ui_set`，记在 `.meta/skills_state.json`） | `src/apps/jarvis_agent/agent/agent.toml`、`lib_opendan/src/skills.rs` |
| Plan Skills | `plan_skills/<name>/SKILL.md`，`SkillKind::Plan`；Jarvis 包自带三份，由 `plan_simple` 的分析步按需读取 | `src/apps/jarvis_agent/agent/plan_skills/`、`lib_opendan/src/skills.rs` |
| UI Session / Chat Session | `SessionKind::Ui`，靠 `route_key`（Agent 的 mailbox 地址）绑定会话 | `src/frame/opendan/src/ui.rs` |
| 针对消息的 stop / 针对 Session 的 stop | `ControlCommand::Stop` 的 `scope: turn` / `scope: session`。后者的含义随会话类型不同，见 §2.3（D21） | `lib_opendan/src/protocol/input.rs`、`runner/inputs.rs` 的 `apply_control` |
| 休眠（Goal） | Goal 的手工 disable：Goal 状态 `paused`，由 `goals.set_status` 设置。界面今天叫“暂停”。和会话的 `control(pause)` 不是一回事 | `lib_opendan/src/protocol/goal.rs` 的 `GoalStatus`、`runner/goal.rs` 的 gate |
| 异常中断（可恢复） | **今天没有这个状态**：宿主重启后把未结束的 Session 全部自动接着跑。目标是用会话的暂停状态（`state.paused`）加专门的原因表示，手工 `resume`（D27，SC11） | `lib_opendan/src/host/mod.rs` 的 `Supervisor::tick`、`protocol/state.rs` 的 `Paused` |
| 撤回输入 | **今天没有**。消息一侧有撤回关系（MsgObject 的 `relates_to: redact`，MessageHub 的“撤回 / 删除”发的就是它），Agent 的桥不处理（D25，SC10） | `desktop/src/app/messagehub/ConversationView.tsx` 的 `sendRelation`；`lib_opendan/src/bridge/msg.rs` |
| Default Session 指针 | **没有存储的指针**：同一 `route_key` 下最新的未结束 UI Session 就是当前会话 | `UiModule::session_for` |
| Work Session | `SessionKind::Work`：simple（`work_simple`）、plan（class `plan_simple`）、explore | `lib_opendan/src/work_card.rs` |
| Todo 列表 | plan 会话的 `todos.json` | `lib_opendan/src/todos.rs`、`agent_tool/src/todo_tools.rs` |
| 占位消息 | 带 `agent_task` 的消息，指向该 Turn 的 TaskMgr task（`opendan.agent_turn/v1`） | `src/frame/opendan/src/tasks.rs`；Desktop `conversation/tasks/`、`conversation/worksessions/` |
| 系统提示词版本 | Session 创建时冻结的 `prompt.frozen`（`FrozenPrompt`）与 behavior catalog 的差异 | `lib_opendan/src/state/behaviors.rs` 的 `freeze_config`、`revision` |
| 近期产物 | 交付进 Agent `artifacts` 桶的文件（`obj://bucket/...`）。**不是** Agent State 的 `artifacts.*`（那是 Work 结果的版本与采纳记录） | `src/frame/opendan/src/objects.rs`、`lib_opendan/src/runner/reports.rs` |

## 1. 现状核对

2026-11-10 按 `27aea9efb` 复核。§1.2、§1.6、§1.8 与起草时一致；§1.1、§1.3、§1.4、§1.5、§1.7 因 skills TODO 和 runtime TODO 的实施有变化，已按现在的代码改写，其中 §1.3、§1.4 是整节重写。

### 1.1 Agent 首页

| 事实 | 位置 |
| --- | --- |
| 首页左列依次是资料卡、工作概况、目标、Skills 卡片、已知工作区、Runtime；右列是 Session 树。没有 Memory 区块 | `pages/Home.tsx` 的 `HomePage`；`pages/Skills.tsx` 的 `SkillsCard` |
| 路由只有首页、`#/session/<sid>`、`#/goal/<goal_id>` 和四个观测页（`#/sessions`、`#/skills`、`#/agent`、`#/loader`）。详情页和 Skills 页用的都是观测页的标签布局，不是首页的手机样式。Skills 页和目标详情页的文案走 i18n，其余观测页是硬编码英文 | `web/src/App.tsx` |
| 所有 `useQuery` 缺省 4 秒轮询一次（`REFRESH_MS`） | `web/src/lib.ts` |
| 数据层是 `OpenDanDataModel` 接口，kRPC 与 mock 各一份实现；e2e 跑在 mock 上 | `web/src/model/{datamodel,krpc,mock,types}.ts`、`web/tests/e2e/` |
| 首页专用方法（`agent.profile`、`usage.models`、`ui.bindings`、`home.workspaces`、`runtimes.settings` / `settings_set` / `remove` / `impact`，以及 owner 用的 `skills.*`）在 opendan 服务里直接实现，不经过 Agent State 门面 | `src/frame/opendan/src/home.rs`、`service.rs` 的 `call` |
| 每个方法都要归入 `is_read` 或 `is_operation`（或门面的 `is_write`），只读授权的 Agent App 按这三张表放行；`permission_tests` 枚举了全部方法。另有 `is_owner_only`：列在里面的方法任何 Agent 的 App 身份都不能调，现在是 skills 的安装、删除、禁用、改 UI 集合、导入、丢弃草稿 | `service.rs` |
| Session 的三个操作：`session.stop {sid, reason?, scope?}`、`session.decide`、`session.post`。没有 pause / resume / delete。session 范围的 stop 遇到没有输入队列的 Session（例如聊天里直接建的 simple Work）时，改由 Supervisor 打断它的驱动，返回 `{index: null, stopped: true}`（`27aea9efb`；之前 owner 停不了这类 Work） | `service.rs`；`lib_opendan/src/host/mod.rs` 的 `Supervisor::stop` |

### 1.2 Memory

| 事实 | 位置 |
| --- | --- |
| 读记忆的 kRPC 只有 `cognition.query {session, query}` 和 `cognition.read {session, reference}`，都必须给 Session id，可见性按该 Session 的授权算 | `lib_opendan/src/state/krpc.rs`、`state/cognition.rs` 的 `FsCognition::caller` |
| `query` 没有检索词、tag、对象时返回 `not_triggered`；最多 50 条认知；分页游标在任何新感知写入后失效 | `lib_opendan/src/memory/recall.rs` |
| 没有“列出全部”“总数”接口。内部有不受限的读：`Memory::graph_view()`、`GraphView::items()`、`Memory::load_perceptions(None)`（`pub(super)`）、`Memory::pending_work()` | `lib_opendan/src/memory/{mod,perception,consolidation}.rs` |
| 认知只能由持有 `memory_consolidation` 租约的整理 Goal 写。`SetStatus … deleted` 会写一个新修订，历史修订留在日志里，不能复活 | `agent_tool/src/agent_memory/{model,state}.rs`；`Memory::consolidator` |
| 认知被删后，读过它的 Session 会收到一行 `hint withdrawn (deleted), do not rely on it: <原陈述>` | `lib_opendan/src/runner/memory_batch.rs` 的 `line` |
| 感知不能删行：只能在整理提交里处置为 `discarded`，之后 `cleanup` 清掉正文，留下标记。已被吸收或处置的感知不能再处置 | `memory/consolidation.rs`、`state/perception.rs` 的 `clear_records` |
| 不走批次直接提交处置的现成样板是 `Consolidator::sweep_expired`；在服务里取租约再操作的样板是 `artifacts.decide` 的 `decide_as` | `memory/consolidation.rs`、`state/krpc.rs` |
| 最近一次成功整理的时间在整理 Goal 的 state 里：`MaintenanceState.last_successful_consolidation_at_ms`。Goal id 不是固定字符串，要从 `goals.list` 里按 `extensions.opendan.builtin == "memory_maintenance"` 找。目标详情页已经在显示它 | `protocol/goal.rs`；`web/src/pages/GoalDetail.tsx` |
| WebUI 现有的记忆查询在观测页，要手填 Session id | `web/src/pages/AgentState.tsx` |

### 1.3 Runtime

| 事实 | 位置 |
| --- | --- |
| 首页 `RuntimeList` 仍是每条一张大卡片：连接方式、可用性、默认（配置的默认被停用时标“Owner 默认（已关闭）”）、内建、ComputerUse 与基线（只读）、授权根目录、期限、用量、工具检查、扩展包；操作只有检查、启停、设为默认。卡片下有一句提示，说这些配置去 Desktop 的“设置 > Agent Runtimes”改 | `web/src/pages/Workspaces.tsx` 的 `RuntimeCard` |
| 没有移除入口：数据层已有 `removeRuntime`（kRPC 与 mock），页面没有调用。`retired` 的记录和当前的混在同一个列表里，置灰并标“已退役”，没有折叠的历史 | 同上；`web/src/model/` |
| 每张可用卡片的 `RuntimeExtPackages` 每 4 秒调一次 `runtimes.ext_tools`，节点 runtime 上会跑探测命令 | 同上 |
| settings 里没有的 runtime 不投影，`host` 占位记录已去掉（runtime TODO §12.3 裁决 3） | `protocol/runtime.rs` 的 `builtin_runtimes` |
| `runtimes.settings` 同时返回生效的 `default_runtime_id`（配置的那台被停用时落到私有）和 owner 配置的 `configured_default_runtime_id`，界面分得出“没配置”和“配置了但停用” | `home.rs` 的 `runtime_settings` |
| 停用一个 runtime 不再清 `default_runtime_id`，重新启用后默认恢复。私有 runtime 及其别名（Lite 下指向本机的条目）不能停用、不能移除，后端拦 | `home.rs` 的 `set_runtime_settings`、`remove_runtime`；`app_schema.rs` 的 `validate_runtimes` |
| 上面有正在运行的 Session 时，停用和移除都被拒绝（`sessions_in_progress: …`，列出 sid），没有绕过的参数。“正在运行”指已经开始（不是 `created`）且这台 runtime 当前可用。`runtimes.impact` 的 `blocking` / `running_sessions` 给出同一结论，另列未结束的全部 Session、等待的 Todo、Workspace、用量；这几项只展示，不拦。WebUI 停用前先取它，被拦时弹出说明并列出这些 Session | `home.rs` 的 `runtime_impact`、`not_in_use`；`RuntimeCard` 的 `toggle` |
| `runtimes.remove` 从这个 Agent 的 settings 里拿掉条目，默认指向它时一并清空。记录转 `retired`；再分配回到 `active`，`usage` 保留 | `home.rs` 的 `remove_runtime`；`lib_opendan/src/state/runtimes.rs` 的 `ensure_builtin` |
| runtime 停用、被移除、到期或不应答时，Loader 给冻结在它上面、还没结束的 Session 打标记（登记表条目的 `runtime_unavailable`，值是原因）并通知 owner 一次；条目带 `runtime_id` | `opendan/src/loader.rs` 的 `mark_runtime_sessions` |
| 设备目录与分配在 control_panel：`agent.runtime_catalog`（节点和 daemon 设备，带 `assigned_to`）、`agent.update {runtimes}`（节点与 `exec-<executor_id>` 设备，一台设备同时只给一个 Agent 启用）、`agent.runtime_enable`、`node.owner_agents_set`。起草时“daemon 设备进不了 Agent 的 runtime 列表”的缺口已经补上 | `src/frame/control_panel/src/{agent_runtimes.rs,agent_daemon.rs,agent_mgr.rs}` |
| Desktop 的目录页是 Settings → Agent Runtimes，页面 id `agent-runtimes`；Add device 已从 Cluster Manager 搬到这里。Desktop 内部用 `openSettings(page)` 打开并定位（launch payload 传给 `SettingsShell` 的 `initialPage`）。URL 上的 `?settings=<page>` 深链仍没有：`DesktopRoute.tsx` 只处理 `?agent_setup` 和系统打开 | `desktop/src/app/settings/{launch.ts,SettingsAppPanel.tsx,pages/AgentRuntimesPage.tsx,components/layout/navigation.ts}`、`desktop/src/desktop/DesktopRoute.tsx` |

### 1.4 Skills

| 事实 | 位置 |
| --- | --- |
| 三类：`SkillKind::Work`、`Plan`、`Goal`。work skill 带 `scope`（`work` / `ui` / `ui,work`），规划里 UI 组与 Work 组的分界就是它。目录是 `cmd_skills/<name>/SKILL.md`、`plan_skills/…`、`goal_skills/…`，各自可带 `references/`；纯文本，带 `tools/` 的包被拒绝 | `protocol/behavior.rs`、`skills.rs` |
| Jarvis 包自带 13 份 work 手册（`ui_core` 的 scope 是 `ui`，`share` 是 `ui, work`）和 3 份 plan skill，没有 goal skill | `src/apps/jarvis_agent/agent/{cmd_skills,plan_skills}/` |
| `SkillMeta.origin` 是 `package`（随 Agent 包同步）、`installed`（skill 包）、`imported`（owner 装的草稿）、`local`。另有 `enabled`、`disabled {by, at_ms, reason}`、`usage`、`size`、`references`、`commands`、`baseline`、`platforms` | `protocol/behavior.rs` 的 `SkillMeta` |
| 禁用、UI 集合、使用记录、审计记在 `.meta/skills_state.json`，安装记录在 `.meta/skills.json`。任何来源都能禁用，只对之后新建的 Session 生效；卸载只对 `installed` / `imported` | `skills.rs` |
| kRPC：`skills.list {kind?}`、`skills.state`、`skills.ui_get` / `ui_set`、`skills.status {runtime_id?}`、`skills.install_plan`、`skills.install {package \| draft, plan_digest, replace?}`、`skills.remove`、`skills.set_enabled`、`skills.import`（文本交给一个 `skill_import` Work 产出草稿，不安装）、`skills.drafts` / `draft_file` / `draft_discard`。安装、删除、禁用、改 UI 集合、导入、丢弃草稿只有 owner 能调 | `opendan/src/{home.rs,service.rs}`；一览在 `opendan/README.md`「Skills」 |
| 读正文只有 `behaviors.cmd_skill {name}`（work 手册）。plan / goal skill 没有读正文的 kRPC | `state/krpc.rs` |
| `skills.status` 对指定的 runtime（缺省私有）现场跑一次检查，每个 work skill 一条：`unmet`（基线或平台不满足的原因）、`commands`（`ready` / `ready_system` / `partial`，逐条命令是否存在、由谁提供、哪个扩展包能补上）。只覆盖 front matter 写了 `commands:` 的手册 | `home.rs` 的 `skills_status`、`skills.rs` 的 `commands_status` |
| 内置手册里 `ui_core`、`buckyos_admin`、`share`、`xllm` 仍没写 `commands:`，页面上显示“手册没有声明命令” | `src/apps/jarvis_agent/agent/cmd_skills/` |
| WebUI 已有 Skills 页（skills TODO 的 S14）。首页 `SkillsCard` 显示“N 可用 / M 禁用”并链到 `#/skills`。页面按 work / plan / goal 三张表列出（work 表里 `scope` 含 `ui` 的排前，有 scope 列），每行有来源、大小、使用次数、所选 runtime 上的可用性、禁用（可填原因）或启用、卸载；下面依次是 UI 集合、从 `.pikg` 安装（浏览器内解析，先出安装计划与风险，owner 确认后才装）、草稿、文本导入、审计 | `web/src/pages/Skills.tsx`、`web/src/pikg.ts`、`web/tests/e2e/skills.spec.ts` |
| 这个页面与规划的差距：放在观测页的标签布局里，是宽表格，不是首页的手机样式；没有单个 skill 的详情页，看不到手册正文；计数不分组；停用项留在原表里置灰，没有折叠的“已停用”小节；没有 URL 入口（CLI 的 `buckyos skill install` 支持 URL）。这些按 D19 不在本单，另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md) | 同上 |
| Desktop 没有 Skills 页面；命令行是 `buckyos skill … --agent <agent_id>` | `../buckyos-websdk/cli/modules/skill.ts` |
| skills TODO 的实现还没有在真实 Zone 上验证过 | skills TODO §0 |

### 1.5 工作区

| 事实 | 位置 |
| --- | --- |
| 区块标题“已知工作区”（i18n 键 `knownWorkspaces`，`data-testid="known-workspaces"`），默认全部展开，按 `updated_at_ms` 排序，每条是完整卡片 | `web/src/pages/Workspaces.tsx` 的 `WorkspaceList`、`WorkspaceCard` |
| 已有“创建”“导入”两个表单，字段齐全（runtime、目录、名称、描述、URL），一步提交 | 同上的 `WorkspaceForm` |
| `workspaces.create` / `workspaces.import` 提交时访问目标 runtime 上的目录，失败返回错误；没有“只检测不登记”的接口 | `lib_opendan/src/state/workspaces.rs` |
| 记录没有“最近使用时间”字段；首页已经加载了 Session 列表，可以按绑定该工作区的 Session 的最近更新时间推算 | `protocol/workspace.rs` 的 `WorkspaceRecord` |
| 只读或有期限的 runtime 上 `workspaces.create` / `import` 报 `runtime_no_workspace`，信息里给出去处。表单的 runtime 候选只列能放工作区的：启用、可用、可写、没有期限 | `state/workspaces.rs`；`Workspaces.tsx` 的 `mayHoldWorkspaces` |

### 1.6 MessageHub 侧面板

| 事实 | 位置 |
| --- | --- |
| 详情栏一次只显示一种：`entity` / `session` / `message` / `profile`。桌面端空间够时内嵌在右侧（宽 320），否则是抽屉；手机端是全屏页 | `MessageHubView.tsx` |
| 没有 Info 按钮。入口是：点头像和名字开 `EntityDetails`，点副标题或 ⋮ 菜单的“编辑信息”开 `SessionDetails` | `ConversationView.tsx` |
| 手机端没有任何滑动手势代码。规划写的“沿用左滑”没有可沿用的实现 | — |
| `EntityDetails` 顶部是 80px 头像区；其中的静音、置顶按钮没有 `onClick`，点了没反应 | `EntityDetails.tsx` |
| 静音是按会话存的（`ui.muted`）。实体级静音是把该实体的全部会话都设上（`updateEntityPreferences`），之后新建的会话不继承 | `MessageHubView.tsx`、`api/store.ts` |
| 没有 Agent 专用的面板。`CodeAssistantAppPanel` 复用了 `ConversationView`、`SessionSidebar`、`EntityDetails`，改 props 时要保证它还能编译 | `src/frame/desktop/src/app/codeassistant/` |
| 访问 Agent 的 OpenDAN 服务已有现成客户端：`agentServiceClient(agentDid)`，用登录者自己的权限，只有 Agent 的 owner 调得通 | `conversation/worklog/source.ts` |
| 头部和会话列表里的运行状态行读 `ui_session` 的 `typing` / `active` / `status_line`，后端没有任何代码写这些键，只有 mock 在填 | `api/store.ts` 的 `runtimeFor` |
| 输入框只在没有写权限时不可用，和 Agent 是否在忙没有关联。回答态（最新消息是 `agent_ask`）会把输入锁成回答 | `ConversationView.tsx` 的 `canSend`；`conversation/input/` |
| 附件已有下载、收藏、预览；没有按文件转发，没有“打开所在位置”，没有会话文件列表 | `conversation/media/` |
| “隐藏会话”“内嵌 Chatbox”在 msg_center 和 MessageHub 里都不存在 | — |

### 1.7 会话控制与 UI Session

| 事实 | 位置 |
| --- | --- |
| `ControlCommand` 已有 `Pause` / `Resume` / `Stop {scope: session \| turn}`，CLI 能发，浏览器只能通过通用的 `sessions.post_input` 自己拼记录 | `protocol/input.rs`、`xagent_cli.rs` |
| pause 之后 Session 不再推理：运行中的 run 在下一个推理边界停下，Turn 保持打开；新到的输入留在队列。**pause 会级联到未结束且有输入队列的子 Session**，resume 只恢复因这次级联暂停的那些 | `runner/inputs.rs` 的 `apply_control`、`cascade_pause` |
| pause / resume 是投进输入队列的控制记录，没有队列的 Session 收不到：级联跳过它们，直接投会报 `has no input queue`。有队列的是 UI、Goal、self_check、`plan_simple` 类、Todo 派发的 Work（`queued`）和 `--interactive` 的 Work；聊天里直接建的 simple Work（`--behavior work_simple`，`work` 类）没有。stop 已经有不走队列的旁路（§1.1），pause / resume 没有 | `template.rs`、`todos.rs`、`xagent_cli.rs` 的 `create-worksession`；`state/registry.rs` 的 `post_input` |
| 聊天里唯一注册的命令是 `/stop`，映射成 session 范围的 stop：UI Session 以 stopped 结束，下一条消息新开一代 UI Session，重新冻结提示词 | `ui.rs` 的 `bridge_ctx`；`lib_opendan/src/bridge/msg.rs` |
| 回答态的 Stop 按钮发的就是文本 `/stop`；TaskMgr 取消对非 Goal 的 Session 也是 session 范围的 stop | `ConversationComposer.tsx`；`tasks.rs` 的 `CancelBridge` |
| Goal Session 是唯一的特例：不论 `scope`，stop 都只结束当前 Activation，会话随 Goal 结束。Goal 另有自己的状态 `active / paused / achieved / archived`，`paused` 时会话还在、不再开新的 Activation；入口是 `goals.set_status`、CLI 的 `agent-session goal pause \| resume` 和 WebUI 目标页的“暂停”按钮 | `runner/inputs.rs` 的 `apply_control`；`protocol/goal.rs`、`runner/goal.rs` 的 gate；`web/src/pages/GoalDetail.tsx` |
| session 范围的 stop 会停掉该 Session 所有未结束的子 Session | `runner/drive.rs` 的 `stop_children` |
| turn 范围的 stop 只关当前 Turn，Session 继续等待。实现里有 `turn_stop && !site.child` 的条件；只有 Goal Session 的测试覆盖。UI Turn 停在 `analyze_intent` 子 context 里时的行为没有验证过 | `runner/outcome.rs`；`lib_opendan/tests/goal_session.rs` |
| 暂停的 Turn 在 TaskMgr 里仍是 Running 或 Waiting，只是 message 写成 `paused: …`。TaskMgr 有 `Paused` 状态，但 opendan 的 `TaskService` 没有对应方法 | `tasks.rs` 的 `update` |
| 一个 UI Turn 创建的 Work Session 卡片已有只读接口 `session.work_cards {sid, task_id}`：卡片状态含 `paused`，plan 卡带 Todo 列表，Todo 绑定的子 Session 带一层卡片（不再往下）。`task_id` 必填，没有“整个会话的全部未结束工作” | `work_card.rs`、`service.rs` |
| `session.read` 已返回 `config.frozen.{catalog_rev, frozen_at_ms, frozen_by}`，`behaviors.revision` 返回当前 catalog 的修订。修订是对 `role.md`、角色补充、`self.md`、全部 behaviors、i18n、各份 `cmd_skills/<name>/SKILL.md` 的路径、大小、mtime 取哈希，任何一个文件变了都会变；`catalog_rev` 在冻结时记下。哈希不含 `references/`、`plan_skills/`，也不含 `.meta/skills_state.json`：owner 改了 UI 集合或禁用了某份手册，修订不变，新会话冻结到的手册却变了 | `service.rs` 的 `read_session`；`state/behaviors.rs` 的 `revision`、`freeze_config` |
| 提交状态时先写 `state.json` 再上报登记表，而 `session_for` 只看登记表。停止后紧接着发的消息可能被投给已结束的旧 Session（11-06 记录的既有竞态；11-10 复核，这两处代码都没有变） | `runner/shared.rs` 的 `commit_and_report`；`ui.rs` 的 `session_for` |
| 登记表没有删除 Session 的操作 | `lib_opendan/src/state/mod.rs` |
| stop 是打断，不等在飞的调用返回：推理在中断边界停下，工具执行期间由监视任务看到队列里的 stop 后发中断，turn 范围和 session 范围都一样。pause 没有这个监视，只在下一个推理边界生效 | `runner/drive.rs` 的 `StopMonitor`；`runner/hook.rs` 的 `boundary` |
| 推理边界只看 stop、Turn stop 和 pause，不看 Goal 状态。Goal 的闸门只在驱动循环的提交点读，所以置 `paused` 或归档之后，正在进行的 Activation 会继续推理到这个 run 结束 | `runner/hook.rs` 的 `boundary`；`runner/goal.rs` 的 `gate`、`runner/drive.rs` |
| 宿主启动后第一次扫描登记表，为它驱动的每个未结束 Session 重新起驱动循环，停在 run 中途的从检查点继续。不经过控制命令，不需要用户确认；断电、崩溃、正常重启走同一条路径，没有“异常中断”状态。这次扫描和以后每一轮是同一段代码，没有“启动后第一次”的标记 | `lib_opendan/src/host/mod.rs` 的 `Supervisor::tick` |
| 暂停标记是 `state.paused = {at_ms, reason, by, in_run}`，只有 `resume` 清它，stop 不清。登记表只投影成布尔的 `status.paused`，不带原因；`status.turn_open` 表示有打开的 Turn | `protocol/state.rs` 的 `Paused`；`protocol/agent_state.rs` |
| 用户的话转给后台工作只有一条路：UI Session 的 Agent 用 `agent-session post` 转给 plan 会话，提示词里写明不转给 plan 派出的 Work。`session.post` 是同一个投递的 kRPC 入口 | `src/apps/jarvis_agent/agent/behaviors/chat_route.toml`、`cmd_skills/ui_core/SKILL.md` |
| MessageHub 一对一会话里可以对任何一条消息发撤回请求：一条空正文、带 `relates_to: {rel: redact, target}` 的消息，对方的客户端自行遵守。Agent 的桥没有处理 redact 的分支；按它的噪声规则，这种空正文的消息会被丢掉，Session 不知道有过撤回（读代码得出，没有跑用例确认） | `ConversationView.tsx` 的 `sendRelation`、`capabilities`；`lib_opendan/src/bridge/msg.rs` 的 `noise` |
| 输入记录的 `key` 就是消息的 ObjId，和撤回关系的 `target` 是同一个值 | `protocol/input.rs` 的 `RejectReason::InvalidEnvelope` 说明 |
| Work Session 在两次 Turn 之间等自己发起的工作（跟踪的 task、要等的子会话）时，每隔 `tick_secs` 产生一次 tick，tick 会打开一个 Turn、推理一次。内置 `work` 类是 30 秒，`plan_simple` 是 600 秒。tick 不看子会话是不是暂停着。UI Session 没有 tick | `runner/ticks.rs` 的 `poll_ticks`；`src/apps/jarvis_agent/agent/agent.toml` |
| “只有别人的输入才能推动它”在状态里是 `needs_input`：`run_state` 为 waiting、不是 Goal Session、没有在等自己发起的工作、没有暂停。空闲的 UI Session 是这种；两次 Activation 之间的 Goal Session 不是，它等的是下一次运行机会 | `protocol/state.rs` 的 `needs_input`、`follows_work` |
| resume 只在这个 Session 自己是暂停的时候才往下级联，并且只恢复暂停原因正好是“父会话被暂停”那一句的子会话。对没有暂停的 Session 发 resume 什么都不做，它名下暂停着的子会话不动 | `runner/inputs.rs` 的 `apply_control`、`cascade_pause` |
| Goal 置 `paused` 不影响它派生的 Work（`origin.goal`）：这些 Work 照常跑到结束。只有 Goal 归档或不存在时宿主才停它们 | `host/mod.rs` 的 `stopped_from_above`；`host/goals.rs` |
| Agent 首页把一个 Session 算作“进行中”的条件：`run_state` 是 running 或 ready，或有打开的 Turn，或 waiting 且不是 `needs_input`。等子任务的 Work、两次 Activation 之间的 Goal Session 都算进行中，空闲的 UI Session 不算 | `web/src/pages/Home.tsx` 的 `isWorking` |

### 1.8 产物

| 事实 | 位置 |
| --- | --- |
| 报告的 `artifacts`、回复里的附件经 `ObjectStore` 存进 named store，绑进桶 `user/<agent_id>/<owner>/artifacts`，key 是 `<对象 id 前 16 位>/<名字>` | `opendan/src/objects.rs`、`runner/reports.rs` |
| 没有记录“哪个 Session、什么时候交付了哪个文件”的索引，也没有相应接口。桶本身可以 `list_objects`，但条目不带 Session | `buckyos-api/src/bucket_client.rs` |
| 首页工作区卡片里的“最近产物”读的是 `artifacts.list`（`ArtifactHead`），是另一个概念 | `Workspaces.tsx` |

## 2. 裁决记录

2026-11-09 owner 在文中逐条批注，整理如下。“按建议”指采纳起草时的建议，建议内容已写进“裁决”一栏。规划 §四 的待定项已并入。D17–D20 是 2026-11-10 owner 对 §2.1 的批注，“owner：”后面是批注原话，其后是整理者的展开。D23–D27 是同日 owner 给出用户语义（§2.5）之后对五个问题的答复，D28 是对随后三条推定的确认，D29 是对展开 D21 时四条推定的批注，写法相同。

| # | 问题 | 裁决 | 落到 |
| --- | --- | --- | --- |
| D1 | Memory “删除”的含义 | 按建议。接受逻辑删除：认知写一个 `deleted` 的新修订，历史保留；界面仍叫“删除”，确认框说明“Agent 不再使用它，历史修订保留”；owner 删除的认知，撤回提示行不再带原陈述。感知只有未整理的能删（处置为 `discarded` 并清正文）；已吸收的标“已整理为认知”，不提供删除，引导去删对应认知。owner 补充：影响大的是认知，用户可以手工删掉一条。所以认知删除是必做项，感知删除走同一个接口顺带提供 | ME2、ME4 |
| D2 | Memory 的可见范围、计数口径与首页预览 | owner：首页按“最近添加”看，最近 5 条认知加 3 条感知就够。认知在前。其余按建议：首页用不受限视图（服务已验证调用者是 owner），每条显示 `scope.subjects`；认知总数 = 状态 `active` 或 `disputed` 且不是 `salience`；感知总数 = 正文未清理的全部，另给“待整理”数 | ME1、ME3 |
| D3 | 停用 Owner 默认 runtime 后是否保留默认设置 | 按建议。保留 `default_runtime_id`，停用期间由 `effective_default_runtime()` 落到私有，重新启用后恢复；放宽 `validate_runtimes`，补调度器测试 | RT1 |
| D4 | 设备在哪里添加和分配，Agent 首页管什么 | 不按建议。owner：之前的规划更完整；Agent 首页主要站在 Agent 的视角讨论 enable / disable / delete。因此设备目录与分配以 [runtime TODO](opendan-agent-runtime-lifecycle-todo.md) 的两步流程为准（§0.6、§4.6，任务 E1–E3、E5、E7），本单不另做“最小子集”；本单只做 Agent 首页一侧：这个 Agent 的 runtime 列表，能启用、停用、删除 | RT1–RT4 |
| D5 | Skills 四组怎么得到 | 不在本单定。owner：Skills 体系升级有专门的 [skills TODO](opendan-skills-system-todo.md)，可能还没完成，界面可以先占位 | §4.4（原 SK1，后被 D19 取代） |
| D6 | 停用项的位置与计数口径（规划待定） | 按建议，作为界面要求移交 skills TODO（停用状态由它实现）：首页计数只算已启用的，停用数另写“· N 已停用”；Skills 页每组末尾有折叠的“已停用”小节，在原处重新启用；任何来源都能停用，只有 `installed` 能卸载；UI 组首版只读 | §4.4 对照表 |
| D7 | 安装器与 LLM 扫描 | 归 skills TODO（owner：这也是那份文档里的内容）。规划对安装器的要求作为界面要求移交 | §4.4 对照表 |
| D8 | 暂停当前消息是否暂停后台 Work | 不级联。owner：暂停当前消息，暂停的是正在处理这条输入的那个 Turn；它创建的 Work Session 都在后台运行，要暂停它们走暂停 Session 的流程。**前半句被 D23 取代：前台消息不再有暂停。** 后半句仍然有效：后台 Work 的暂停走暂停 Session 的流程 | SC1、MH5 |
| D9 | “停止当前消息”的含义 | 与 D8 同理，只作用于当前 Turn。owner：停止当前消息是友好停止，立刻完成 current turn。即不再发起新的推理和工具调用，当前 Turn 就此收尾，Session 和上下文都在，已创建的 Work 不受影响 | SC2、MH4 |
| D10 | 输入框何时禁用 | 按建议。只按“看得到的未结束占位”判定：最新一条带 `agent_task` 的 Agent 消息，其 task 运行中或已暂停时禁用；等待子任务、等待外部时不禁用；回答态优先，照常能答；占位出现之前不禁用；首次点击直接暂停。**D23 之后没有暂停按钮，“首次点击直接暂停”作废，控制按钮是“停止”；“已暂停”只剩异常中断这一种来源（D27）** | MH4 |
| D11 | Clean History 的做法 | 按建议。(a) 新动作叫“开始新对话”，归档 Agent 侧会话，消息保留可见，时间线插一条分隔；MessageHub 原有的“清空聊天记录”不变。(b) 有未结束 Work 时拒绝，提示先停止或等待。(c) 新一代会话保持懒创建（下一条消息到来时才建）。(d) 不引入存储的默认会话指针，“归档”就是以 stopped 结束并在 worklog 记原因 | SC6、MH6 |
| D12 | 子会话树的范围与默认展开 | 按建议。只列当前这一代 UI Session 下未结束的 Work；默认展开到 Todo 列表，Todo 下的子会话默认收起；递归到 4 层为止 | SC5、MH3 |
| D13 | 近期产物 | 按建议。AgentRoot 里加只追加的交付索引；“当前会话”= 同一 `route_key` 的各代 UI Session 及其后代交付的；区块放在面板下部、子会话树之后，默认收起；“更多”菜单首版给下载、收藏、转发、定位到消息，“打开所在位置”等 Files 能按桶浏览再做；隐藏会话等内嵌 Chatbox 出现再接，索引先把会话信息记全 | AR1、MH8 |
| D14 | 提示词版本的比较与刷新 | 按建议。标签按内容比（冻结的身份文本、behaviors、cmd_skills 正文对 catalog 当前内容），由服务端给结论；压缩时刷新只对 UI / group 会话做，Work 不刷新；先核对压缩时系统前缀的重建方式再定实现 | SC7、MH7 |
| D15 | Session 删除的范围 | 按建议。只允许删已结束的 Work / self_check 会话，连同其全部已结束的后代；有未结束后代时拒绝；UI 会话和 Goal 会话不可删；接受用量统计随之减少，确认框写明 | SC8、HM3 |
| D16 | 统一侧面板的适用范围与入口 | 按建议。本轮只对“对方是自己名下 Agent”的会话启用新面板；桌面端 Info 按钮开关，记住上次状态，不默认展开；手机端保留点标题进入，另加左滑手势。owner 补充：群和联系人的侧栏也要优化，但不在这次任务中 | MH1 |
| D17 | `/stop`、回答态的 Stop 按钮、TaskMgr 取消停什么 | owner：停当前 Turn（ui session 不能停）。这三处都只关当前 Turn，会话和上下文留着，已创建的 Work 不受影响（同 D9）。UI Session 不会被“停止”结束，只经归档（D11、D18 的 clean）结束 | SC2、SC6、MH4 |
| D18 | tunnel 渠道怎么“开始新对话”，`new` 与 `clean` 的关系 | owner：tunnel 如果本身有多 session 能力，那么 new 和 clean 的含义是不同的，但如果 tunnel 本身是单 session 的，两个操作意思相同（clean 语义：保存当前 chatsession 对应的 agent uisession，创建新的 uisession 并关联到当前 chatsession 上）。clean 就是 D11 的“开始新对话” | SC6、SC9、MH6 |
| D19 | Skills 页与规划的形态差距由谁补 | owner：skills 内部页面的修改做个独立 todo，本单不做。已另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)，原 SK1 和对照表移到那里 | §4.4 |
| D20 | 没有输入队列的 Work 怎么暂停 | owner：这么看来 agent session 还是应该都有队列为好。只是这个队列不一定希望投递 user message。即所有 Session 都带输入队列，控制命令一律走队列；队列收不收 user message 是另一个属性 | SC0、SC1、MH5 |
| D21 | stop 的两种含义 | owner：要区分针对消息的 stop 和针对 session 的 stop。目前看来不同类型的 session 的 stop 的意义并不完全相同。Goal Session 是可以手工 disable 的（应该用休眠这个词）。展开成 §2.3 的表：针对消息的 stop 只关当前 Turn；针对 Session 的 stop 按会话类型各有含义，UI Session 没有（D17），Goal Session 是休眠，Work 是结束 | SC2、SC6、MH4、MH5、HM3 |
| D22 | 整理 D17–D20 时的其余推定 | owner：其它的按你的建议。即 §2.1 的第 2–7 条照推定执行；第 1 条由 D21 取代 | SC0、SC6、SC9 |
| D23 | 前台消息要不要暂停 | owner：不要暂停。产品上这个操作没意义，（但断电恢复其实要求这个状态语义上是存在的）。占位消息和输入框上不提供“暂停”，规划 §9.1 的暂停、D8 的前半句、D10 的“首次点击直接暂停”作废。会话的暂停状态仍然存在：异常中断就是它（D27），这时提供“恢复”和“停止”。pause 保持 Session 级，不加不级联的选项 | SC1、MH4 |
| D24 | 后台 Work 收不收用户消息 | owner：按设计，plan_simple 才可以收消息（纯 worksession 是不行的），这是一个刻意的区分。§2.1 推定 7 不变。发消息纠正、暂停后发消息再恢复只对 plan 成立；纯 Work 只能暂停、恢复、终止，要改方向就终止后重新发起 | SC0、MH5 |
| D25 | 撤回输入：停止并删掉这条输错的消息 | owner：按你的建议吧。建议是：消息还没被处理时直接丢掉，等于没输入过；已经开始处理的，停止，再记一条“用户撤回了这条输入”，时间线上撤回这条消息，已经执行的步骤留在历史里，让模型知道自己做过什么 | SC10、MH4 |
| D26 | Goal 休眠时正在进行的 Activation | owner：不发起下一次推理就好。即不打断在飞的这次推理和它的工具，跑完后不再发起下一次，当前 Activation 就此收尾（读法是 §2.1 推定 12，D28 已确认） | SC2 |
| D27 | 断电或重启后的恢复 | owner：现在都手工恢复吧，自动恢复留到下一期再做。重启前正在工作的 Session 变成“异常中断（可恢复）”，用户手工恢复，按任务树恢复；断电、崩溃、正常重启不区分 | SC11、SC3、MH4、MH5、HM3 |
| D28 | 展开 D25–D27 时的三条推定，以及本单在控制上的范围 | owner：需要我确认的三条都可以按你的建议。自动恢复是下一期的，这次 TODO 把手动控制语义做对。即 §2.1 的第 12–14 条照推定执行。为“做对”按代码逐项核对后，第 13 条的判定有两处修正，另有第 15、16 两条新推定，见 §2.1 | SC2、SC9、SC10、SC11、MH4、MH5、HM3 |
| D29 | 展开 D21 时的四条推定（§2.1 第 8–11 条） | owner 在这四条下面各批注了一句：好，按你的建议。即：UI Session 收到针对 Session 的 stop 时拒绝，不降级；Goal Session 收到针对 Session 的 stop 就是让它的 Goal 休眠；“休眠”先改界面和文档的用词，状态值 `paused` 和 CLI 的 `goal pause \| resume` 不改名；TaskMgr 取消在 UI Session 和 Goal Session 上是针对消息的 stop，在 Work 上仍结束这个 Work | SC2、HM3 |
| D30 | 推定 13 的两处修正、推定 15、推定 16 | owner（实施之后，2026-11-10 晚）：推定 13 修正、15、16 都按你的建议。即：等自己发起的工作的 Work 也算中断、在等用户回答的不算；Goal 休眠时它派生的未结束 Work 一并暂停、唤醒时恢复；tunnel 上有 `/resume`，普通消息到了中断的对话时回一次提示。三处都已按此实施，不用改代码 | SC2、SC9、SC11 |

### 2.1 按裁决推定、待确认

这一节原有六条（起草时的四条推定，加 11-10 复核时新增的两条）。owner 于 2026-11-10 批注了其中四条，去向如下：

| 原条目 | 去向 |
| --- | --- |
| 1 `/stop`、回答态的 Stop 按钮、TaskMgr 取消也只停当前 Turn | 已裁决，D17 |
| 2 tunnel 渠道怎么“开始新对话” | 已裁决，D18；新增任务 SC9 |
| 3 D4 的“删除”是“从这个 Agent 移除” | 没有批注。已按此实施并在本机 Zone 验过：`runtimes.remove` 把记录转为 `retired`，设备还在用户级目录里，可以再分配（runtime TODO §12.1）。owner 11-09 晚的裁决（那份文档 §12.3 第 3、4 条）沿用了这个含义 |
| 4 D5 的“先占位”做到只读为止 | 已失效：skills TODO 在本单动手之前交付了完整的 Skills 页 |
| 5 Skills 页与规划的差距由谁补 | 已裁决，D19；SK1 移出本单 |
| 6 没有输入队列的 Work 怎么暂停 | 已裁决，D20；新增任务 SC0 |

整理 D17–D20 时按批注的意思推出了七条。owner 同日答复：第 1 条不照推定，要区分两种 stop（D21）；其余按建议（D22）。原文留在这里，任务里写的“§2.1 推定 N”仍指这个编号。

1. **（已被 D21 取代）D17 管到所有来源的 stop。** 原推定是停止命令到了 UI Session 一律悄悄降成只关当前 Turn，和 Goal Session 今天的做法一样。owner 的意见是两种 stop 要分开，各类会话的 Session 级 stop 含义不同，见 §2.3 和下面的推定 8–11。
2. **（已确认，下同）归档不是 stop，但结局仍记 `stopped`。** D11 (d) 定的是“归档就是以 stopped 结束并在 worklog 记原因”，SC6 原来的做法是投一条 session 范围的 stop。D17 之后这条路对 UI Session 不通了，归档要有自己的路径；旧会话最后的状态仍是 `stopped` 加归档原因，不新增结局类型。（SC6）
3. **clean 的“创建新的 UI Session 并关联”仍是懒创建。** 按 D11 (c)：归档后不立刻建，下一条消息到来时才建新一代，按同一个 `route_key` 关联到这个聊天会话；中间这个聊天会话没有 UI Session。用户看到的结果与批注一致。（SC6）
4. **Agent 一侧把 `/new` 和 `/clean` 都当 clean。** Telegram 今天是单 session：每个 bot 加聊天固定一个会话 id。MessageHub 是多 session：new 是现有的“新建会话”，clean 是 MH6 的“开始新对话”，都是界面操作，不走聊天命令。Agent 一侧建不了聊天会话，所以多 session 渠道的 new 只能由渠道自己做：哪天某个 tunnel 有了这种能力，由它在消息到 Agent 之前处理 `new`。副作用是在 MessageHub 里手打 `/new` 也会被当成 clean；要让它等于“新建会话”，得由 MessageHub 在发送前拦下来，本单没有排。（SC9）
5. **`/clean` 同样受 D11 (b) 约束。** 有未结束的 Work 时拒绝，回一句话并列出它们。tunnel 上没有子会话树，用户只能在对话里让 Agent 停掉这些 Work，或者等它们结束。（SC9）
6. **`/cancel` 与 `/stop` 同义。** Telegram 菜单里的 `cancel` 写的是 “Cancel current response”，注册成 stop 的别名。菜单里的 `info`、`list`、`switch`、`help` 不在本单。（SC9）
7. **D20 的“不投递 user message”沿用今天的边界。** 用现成的 `session.input_policy` 表达。哪些 Session 收 user message 与今天能不能投一致：今天没有队列的 Session（聊天里直接建的 simple Work、`skills.import` 起的 Work）有了队列以后只收控制命令；UI、Goal、self_check、plan、Todo 会话、`--interactive` 的 Work 不变。（SC0）D24 再次确认了这一条：plan 收消息、纯 Work 不收是刻意的区分。

下面四条是把 D21 展开成 §2.3 和 SC2 时推出来的。owner 2026-11-10 在每一条下面批注了“好，按你的建议”，四条都照推定执行（D29）。

8. **（已确认，D29）UI Session 收到针对 Session 的 stop 时拒绝，不降级。** 返回明确的错误，说明 UI Session 不能停、要清空上下文用“开始新对话”。不照搬 Goal Session 今天“悄悄降成只停当前 Turn”的做法，否则两种 stop 又混在一起。（SC2）
9. **（已确认，D29）Goal Session 收到针对 Session 的 stop，就是让它的 Goal 休眠。** 走今天 Goal 状态 `paused` 的那条路径（`goals.set_status`）：会话和状态都留着，不再开新的 Activation，唤醒后继续。今天这种 stop 是悄悄降成只停当前 Activation。（SC2）
10. **（已确认，D29）“休眠”先改用词，不改标识。** 界面文案和文档里 Goal 的“暂停 / 恢复”改成“休眠 / 唤醒”，和会话的“暂停”（SC1）分开。状态值 `paused`、CLI 的 `agent-session goal pause | resume`、kRPC 的参数先不改名。目标页的改版不在本单，这里只动这几个词。（SC2）
11. **（已确认，D29）TaskMgr 取消按 task 属于哪类会话来分。** 被取消的是某个 Turn 的 task。在 UI Session 和 Goal Session 上它是针对消息的 stop；在 Work 上保持今天的行为，结束这个 Work，因为在用户眼里那个 task 就是这件工作。（SC2）

下面三条是展开 D25–D27 时推出来的。owner 2026-11-10 确认都按建议（D28）。第 13 条确认之后按代码核对出两处漏洞，修正写在它下面，等过目。

12. **（已确认，D28）D26 的“下一次推理”指下一次模型调用，不是下一次 Activation。** 置休眠后，在飞的这次推理和它的工具跑完，到下一个推理边界当前 Activation 就以 stopped 收尾。今天的行为是 Activation 继续推理到自然结束（§1.7）。归档（删除 Goal）按同一条原则处理。（SC2）
13. **（已确认，D28；两处修正也已确认，D30）哪些 Session 算“正在工作”。** 确认时的写法是：宿主启动后第一次扫描时，有打开的 Turn 且没有暂停的未结束 Session，不分类型；两次 Turn 之间的不算，照常等输入、等调度。内置的 self_check 和记忆整理 Goal 同样适用，重启后用户发来的新消息照常处理，Goal 到了下一次运行机会照常激活，这几点不变。两处修正：
    - **等自己发起的工作的 Work 也算，即使这一刻没有打开的 Turn。** 原写法把它归到“两次 Turn 之间、不算”。但这种 Work 会自己产生 tick，每次 tick 推理一次（`work` 类 30 秒一次，plan 600 秒一次，§1.7）。按原写法，重启后 plan 的 Todo 会话停住了，plan 自己却每十分钟推理一次，没人看着就一直烧下去；一个在等长任务的 simple Work 更是每 30 秒一次。这与“手工恢复之前不推理”相反。改成：未结束、没有暂停、不是在等用户的 Work 都算，包括刚建好还没开始的。这样一棵任务树要么整棵停住，要么整棵在跑。
    - **在等用户回答的不算。** 提了问题等回答的会话，Turn 是开着的，按原写法会被标成中断。这时用户照常回答，回答进了队列却没有任何反应。它本来就不会自己推理，用户一回答就是用户自己让它继续，不需要再点一次恢复。
    - 修正后的判定：Work Session 看“未结束、没暂停、不是在等用户”；UI、Goal、self_check Session 看“有打开的 Turn 且不是在等用户回答”。（SC11）
14. **（已确认，D28）撤回的边界。** 只有两种输入能撤回：还没被消费的，和它的 Turn 还开着的。Turn 已经关闭、Agent 已经回复的不能撤回，界面上不给入口；这时 MessageHub 原有的“撤回 / 删除”只改时间线，不动 Agent 的历史。（SC10、MH4）

下面两条是按 D28 的“把手动控制语义做对”核对代码时推出来的。owner 2026-11-10 晚确认都按建议（D30）。

15. **（已确认，D30）Goal 休眠时，它派生的未结束 Work 一并暂停，唤醒时恢复。** 今天 Goal 置 `paused` 只是不再开新的 Activation，它之前派出去的 Work 照常跑到结束（§1.7），只有归档才停它们。owner 对休眠的要求是“不推理了，省 token”（§2.5 第 3 项），派生的 Work 接着推理就不符合。做法比照父会话暂停的级联：休眠时给这些 Work 发会话暂停，原因注明是 Goal 休眠；唤醒时只恢复因此暂停的。不同意的话就保持今天的行为，并在目标页上写明“已经派出的工作会跑完”。（SC2）
16. **（已确认，D30）tunnel 渠道要有恢复的入口：`/resume`。** 重启时如果某个对话的消息正处理到一半，这个 UI Session 会停在异常中断。MessageHub 里占位消息上有“恢复”“停止”；Telegram 之类没有界面，用户再发消息只会进队列，Agent 像是没了反应。补两样：聊天命令 `/resume`，恢复这个对话被中断的 Turn 和它名下被中断的 Work；普通消息到了一个中断的 UI Session 时，回一句提示，说明上一条因重启中断，发 `/resume` 继续或 `/stop` 放弃。（SC9、SC11）

### 2.2 之后在别处作出、影响本单的裁决

| 来源 | 裁决 | 对本单的影响 |
| --- | --- | --- |
| runtime TODO §12.3 裁决 1（11-09 晚） | Linux 必定有容器，Agent 的私有 runtime 就是容器；原生运行只在不带容器的产品（Lite） | RT2 里“私有 runtime 同时是 Owner 的机器，只显示一行”的情形只在 Lite 出现 |
| 同上，裁决 3 | 不要 `host` 占位。这个概念叫 owner default runtime，由 `default_runtime_id` 决定，没设就是私有 | RT2 不用再过滤占位记录；界面文案用“Owner 默认” |
| 同上，裁决 4 | 有在用的 Session 就不让停用，也不让移除，没有 `confirm` | 取代 runtime TODO 裁决 27 的“有影响先列出再确认”。RT2 的停用、RT4 的删除都是：被拦时列出这些 Session，没有“仍要继续” |
| 同上，裁决 5 | runtime 掉线按网络错误处理，不加定期探活 | 首页显示的可用性只反映上一次使用或检查的结果 |
| runtime TODO 裁决 28（11-09） | skill 是纯文本，不带命令也不带安装脚本；带命令的做成扩展包 | 规划 §3 的“Skill 与命令可独立安装”“扫描可拆解的 CMD Skills”按这个读：命令来自扩展包和 runtime 的基线，Skills 页只显示手册依赖的命令是否可用、哪个扩展包能补上 |

### 2.3 stop 的目标语义（D17、D21，待实施）

本节是 UI 升级的目标方案，不是现行协议。其中 §2.1 的推定 8–11（D29）和 12–14（D28）已确认，推定 15、16 和推定 13 的两处修正还在等确认。现有控制合同统一见 [Session Control Protocol](<../doc/opendan/protocol/Session Control Protocol.md>)；具体差异与裁决状态见 §2.4。

| 会话类型 | 针对消息的 stop：关当前 Turn | 针对 Session 的 stop |
| --- | --- | --- |
| UI Session（含群聊） | 关掉正在处理的这条消息；会话和上下文留着，等下一条；已创建的 Work 不受影响（D9） | 没有。UI Session 不能停，结束它只有归档（clean，D11、D18）；收到时拒绝（D29） |
| Goal Session | 结束当前这次 Activation，Goal 照常等下一次机会。今天已经是这样 | 休眠：手工 disable。Goal 不再被激活，直到唤醒；会话和它的状态都留着。对应今天 Goal 的 `paused` 状态（D29）。置休眠后不再发起下一次推理，正在进行的 Activation 在下一个推理边界收尾（D26、D28）。它派生的未结束 Work 一并暂停，唤醒时恢复（推定 15） |
| Work Session（simple / plan / explore） | 关当前 Turn，Work 还在，之后的 tick 或子任务结束会再开 Turn。界面不提供这个入口 | 以 stopped 结束，连同它未结束的子会话。今天已经是这样 |

- **针对消息的入口：** 聊天命令 `/stop`、`/cancel`（SC9）；回答态的 Stop 按钮和占位消息上的“停止”（MH4）；TaskMgr 取消 UI / Goal 会话的 Turn task（D29）。接口是 `session.stop {scope: "turn"}`。
- **停止一条消息不停它已经发起的 Work（D8、D9）。** 所以停止或撤回之后，界面要把这条消息发起的、还没结束的 Work 列出来，当场能停（MH4）。否则用户以为停住了，后台还在按错的输入推理。
- **针对 Session 的入口：** 子会话树节点上的“停止”（MH5，节点都是 Work）；Agent 首页 Session 详情页的按钮（HM3，按会话类型显示）；Goal 的休眠在目标页，就是现有的“暂停”按钮，改名。接口是 `session.stop {scope: "session"}`；Goal 的休眠也可以直接调 `goals.set_status`。
- **stop 是立刻打断（§2.5 的 1a）：** 在飞的推理和工具都不等，今天已经是这样，不改。
- **撤回是针对消息的 stop 再加一步（D25，SC10）：** 停止之外，把这条输入记成“用户已撤回”，时间线上撤回这条消息。
- **“暂停 / 恢复”是第三件事（SC1）：** 会话暂时不再推理，随时接着来，对各类会话含义相同。界面只在后台 Work 上提供暂停，前台消息没有（D23）。Goal 的手工 disable 不叫暂停而叫休眠，就是为了不和它混。
- **“异常中断”是暂停状态的另一个来源（D27，SC11）：** 重启前正在工作的 Session 重启后停在暂停状态，原因标明是中断，用户手工恢复。前台消息虽然没有暂停按钮，UI Session 也会因此处在这个状态，这时占位上是“恢复”和“停止”。
  - 哪些算正在工作（推定 13 及其修正）：后台 Work 只要没结束、没暂停、不是在等用户，都算；UI、Goal、self_check 会话只在处理一条消息或一次 Activation 的中途才算。在等用户回答的、空闲的对话、两次 Activation 之间的 Goal 不算。
  - 恢复按任务树往下：恢复一个节点，连同它名下因同一次中断停住的子会话。对话本身没有中断、只是它名下的 Work 中断了的时候，对话照常能用，Work 在子会话树和 Agent 首页上恢复，两处都有“全部恢复”。
  - 中断的会话可以不恢复、直接停止。
- self_check 会话本单不涉及停止语义的变化；异常中断对它同样适用（推定 13）。

### 2.4 与现有控制协议的差异（2026-11-10 文档核对）

现有设计已区分 Turn 停止、Session 终止、可恢复的 Session 暂停和 Goal 调度暂停，分层继续沿用。本次只整理文档，不改变代码、命令字段或 schema，也不把推定自动升级成裁决。以下变更实施前按裁决状态处理，实施时同步 [Session Control Protocol](<../doc/opendan/protocol/Session Control Protocol.md>)，输入、目录和架构文档引用该合同。

| 事项 | 现有合同 | 本单目标 / 推定 | 状态与任务 |
| --- | --- | --- | --- |
| 前台停止入口 | `stop(scope=turn)` 已有，保留 Session、不停子 Session；聊天 `/stop` 今天仍映射成 session 范围 | `/stop`、回答态 Stop、相关 Turn task 的取消选择 turn 范围 | D9、D17 已裁决；TaskMgr 按会话类型分派已确认（D29）。SC2、SC9 |
| UI Session 终止 | 协议允许 `stop(scope=session)` 使 UI Session 结束 | UI 只通过 clean 归档；直接 Session stop 报错 | D17 已裁决；收到时拒绝、不降级已确认（D29）。SC2、SC6 |
| Goal 的 Session 级 stop | 任意 stop 只结束当前 Activation；Goal 的 `paused` 另行禁止新 Activation，已打开的 Turn 自然结束 | 将 Session 级 stop 映射成 Goal 休眠 | D21 要求区分停止与休眠；映射和只改用词都已确认（D29）。SC2；当前仍使用独立 Goal 状态操作 |
| 在飞调用 | stop 会请求中断当前执行，并按停止收尾路径尝试取消外部 task；不能保证所有外部任务均收到取消或已经停下 | 不变，保持直接打断。owner 11-10 的用户语义是“立刻停止推理，可以尽快输入下一条消息”（§2.5 的 1a） | 不改执行语义，SC2 只改入口发哪种 stop。本行原来写的目标是“等在飞调用返回后关闭 Turn”，与 1a 相反，已改 |
| pause / resume 级联 | pause 默认向未结束且有队列的子级联；resume 只恢复因父级而暂停的子；当前无 `cascade` 字段 | pause 不变，不新增 `cascade` 字段：前台消息不提供暂停，原来要靠它实现的“只暂停当前消息”没有了。resume 要扩展：也恢复因同一次异常中断而停下的子 | D23 取代 D8 的前半句。SC1 只加接口；resume 的扩展在 SC11 |
| 控制队列 | 队列可选；无队列 Session 只有宿主停止路径，没有 pause / resume 旁路 | 所有 Session 有控制队列。收不收 user message 沿用今天的边界：后台 Work 里只有 plan 收 | D20、D24 已裁决。SC0；属于输入与控制合同变更 |
| `/cancel` | 无独立 Session cancel 原语 | 聊天 `/cancel` 注册为 `/stop` 别名 | D22 确认的推定 6。SC9；不新增底层命令 |
| 撤回输入 | 没有。worklog 不改写；消息的 redact 关系到了桥被当作噪声 | 新增一条以输入 `key` 为目标的控制：未消费的丢弃；Turn 还开着的，停止并记“用户已撤回” | D25 已裁决；边界已确认（推定 14，D28）。SC10；属于新增控制命令 |
| Goal 休眠的生效点 | 推理边界不看 Goal 状态，置 `paused` 后当前 Activation 推理到自然结束 | 下一个推理边界收尾，不再发起下一次推理 | D26 已裁决；读法已确认（推定 12，D28）。SC2；属于执行语义变更 |
| Goal 休眠与它派生的 Work | Goal 置 `paused` 不影响派生的 Work，它们跑到结束；只有归档停它们 | 休眠时一并暂停，唤醒时恢复因此暂停的 | 推定 15，待确认。SC2；属于 Goal 生命周期与暂停级联的变更 |
| 宿主重启后的恢复 | 未结束的 Session 全部自动接着跑，没有“异常中断”状态 | 重启前正在工作的停在暂停状态并标明是中断，手工 resume，按任务树级联；自动恢复下一期 | D27 已裁决；判定范围已确认（推定 13，D28），其后的两处修正待过目。SC11；属于宿主恢复行为与暂停合同的变更 |
| tunnel 上的恢复入口 | 聊天命令只有 `/stop` | 增加 `/resume`；普通消息到了中断的 UI Session 时回一句提示 | 推定 16，待确认。SC9、SC11；不新增底层命令 |

共同约束：投递或 `control_applied` 不等于所有工具与子工作已经停下；停止不回滚外部副作用。暂停中的 Turn 被 stop 关闭后，Session 的暂停标记仍在，需要 resume 才允许继续推理；异常中断是例外，停止一个中断的 Turn 时一并清掉中断标记（SC11）。已结束 Session 拒绝 stop / pause / resume；重复 pause、未暂停时 resume 才是无效果的控制。SC1–SC3 的界面与 TaskMgr 状态映射须保留这些区别。

### 2.5 用户角度的有效语义（owner，2026-11-10）

owner 从用户角度列出的有效控制，是 D23–D27 的出发点。每一项对到哪条控制、今天支持到哪一步，表在 [Session Control Protocol](<../doc/opendan/protocol/Session Control Protocol.md>) §9，这里只列语义和落到的任务。

| # | 用户遇到的情况 | 用户想要 | 任务 |
| --- | --- | --- | --- |
| 1a | 发出消息后发现不对，例如输错了关键信息 | 立刻停止推理，马上能输入下一条；历史里看得到旧输入，没有 Agent 的回复 | SC2、MH4 |
| 1b | 同上 | 立刻停止，并删掉这条输错的消息，相当于没输入过 | SC10、MH4 |
| 2a | 看到后台 Work Session 做错了 | 发消息纠正它的方向（只对 plan，D24） | 今天已有：对话里说，Agent 转给 plan |
| 2b | 同上 | 先停下它的推理，发消息，再恢复（只对 plan，D24） | SC0、SC1、MH5 |
| 2c | 同上 | 终止这个后台任务，之后可能再发消息触发新的 | MH5（后端已有） |
| 3 | Goal 没有效果 | 休眠或删除，不再推理 | SC2 |
| 4 | 断电后恢复 | 正在工作的 Session 变成“异常中断（可恢复）”，手工按任务树恢复；自动恢复下一期（D27） | SC11、SC9、MH4、MH5、HM3 |

owner 的推论：所有控制都做不到“消除一条错误输入带来的影响”。工具用了就是用了，能做到的只有停止推理、省下 token。界面文案不要让人以为“停止”“撤回”会撤销已经做了的事。

D28 定了这一期的标准：手动控制的语义要做对。检验办法是对上表每一项问同一个问题：用户做了这个操作之后，还有没有哪里在他不知道的情况下接着推理。2026-11-10 按代码核对的结果：

| 项 | 核对出的问题 | 处理 |
| --- | --- | --- |
| 1a、1b | 停止一条消息不停它已经发起的 Work（D8、D9 的裁决），这些 Work 接着按错的输入推理，用户不一定知道 | 停止或撤回后在占位下列出它们，当场能停（MH4） |
| 2b | 暂停 plan 之后，对话里的 Agent 还转不转用户的话，取决于提示词里“还在运行”怎么读，没有核对 | 提示词写明暂停中的 plan 照样转（SC1） |
| 3 | Goal 休眠后，正在进行的 Activation 推理到自然结束；它派生的 Work 照常跑到结束 | 前者在推理边界收尾（D26、D28）；后者一并暂停（推定 15）。都在 SC2 |
| 4 | 只按“有打开的 Turn”判定的话，等子任务的 Work 重启后会被自己的 tick 重新带起推理 | 判定修正（推定 13 的修正，SC11） |
| 4 | 在等用户回答的会话被标成中断后，用户的回答没有反应 | 判定修正（同上） |
| 4 | 恢复只从暂停着的节点往下级联。对话没中断、名下的 Work 中断了的时候，没有一处能一次恢复 | 子会话树和 Agent 首页加“全部恢复”，界面逐个对顶层的中断节点发 resume，不改协议（MH5、HM3） |
| 4 | tunnel 渠道没有恢复入口，中断后用户发消息没有反应 | `/resume` 和一句提示（推定 16，SC9、SC11） |
| 4 | 登记表的状态晚于 `state.json` 提交，断电时可能是旧的 | 判定由驱动循环拿到 lease 后按 `state.json` 做，登记表只用来找出要看的 Session（SC11） |

2a、2c 没有发现问题：转发的路径在提示词里已有（没有跑用例），终止会连同未结束的子会话一起停。

## 3. 接口变更一览

供前后端并行时对齐。名称可在实施时微调，改了要回写本表。除注明外都在 opendan 服务（`service.rs` + `home.rs` 或新模块），并归入 `is_read` / `is_operation` 与 `permission_tests`。

| 方法 | 读写 | 形状 | 任务 | 状态（11-10） |
| --- | --- | --- | --- | --- |
| `home.summary` | 读 | `{sessions: {working, waiting, failed, total}, memory: {perceptions, pending, cognitions, last_consolidation_at_ms}, goals: {active}, skills: {work, plan, goal, disabled}, runtimes: {enabled, unavailable}}` | HM2 | 已做（另有 `sessions.interrupted`） |
| `memory.overview` | 读 | `{cognitions: {current}, perceptions: {total, pending}, last_consolidation_at_ms, maintenance_goal_id, recent: {cognitions: MemoryEntry[], perceptions: MemoryEntry[]}}`；`recent` 是最近添加的 5 条认知和 3 条感知 | ME1 | 已做 |
| `memory.list {layer, limit, cursor?}` | 读 | `{entries: MemoryEntry[], next_cursor?}`，按添加时间倒序；`MemoryEntry = {layer, reference, summary, kind, state, at_ms, revised_at_ms?, subjects, session_id?, deletable, absorbed_into?}` | ME1 | 已做（`MemoryEntry` 多 `revision?`、`cleared?`，`absorbed_into` 是数组；见 §8） |
| `memory.read {reference}` | 读 | 认知带来源视图，感知带原记录；不要 Session | ME1 | 已做 |
| `memory.forget {reference, expected_revision?, reason?}` | 操作 | 认知 → 新修订 `deleted`；感知 → `discarded` 并清理。整理 Goal 正在运行时返回 busy | ME2 | 已做（只有 owner 能调） |
| `runtimes.settings` | 读 | `{runtimes: {<id>: {enabled, target}}, default_runtime_id, configured_default_runtime_id}`；后一项是 owner 配置的原始值 | RT1 | 已有 |
| `runtimes.settings_set {runtime_id, enabled?, default?}` | 操作 | 停用不清默认；拒绝停用私有及其别名；上面有正在运行的 Session 时拒绝停用（`sessions_in_progress`） | RT1 | 已有 |
| `runtimes.remove {runtime_id}` | 操作 | 从这个 Agent 移除一个已分配的 runtime，记录转 `retired`；拒绝私有及其别名；有正在运行的 Session 时拒绝。返回新的设置 | RT1、RT4 | 已有 |
| `runtimes.impact {runtime_id}` | 读 | `{runtime_id, owner_default, blocking, running_sessions, active_sessions, pending_todos, workspaces, usage}` | RT2、RT4 | 已有（runtime TODO R7） |
| `session.pause {sid, reason?}` / `session.resume {sid, reason?}` | 操作 | 返回 `{index}`，同 `session.stop`。没有 `cascade` 参数（D23）。SC0 之后对任何未结束的 Session 都能投；界面只在后台 Work 上用 pause，resume 还用来恢复异常中断的 Session（SC11） | SC1、SC11 | 已做 |
| `session.retract {sid, key}`（名称暂定，要不要加见 SC10） | 操作 | 撤回一条输入，`key` 是消息的 ObjId。未消费的丢弃；Turn 还开着的，停止并记“用户已撤回”；Turn 已关闭的拒绝并说明。返回走了哪一种。主入口是 MessageHub 的 redact 消息，由桥转成控制 | SC10 | 不加：入口是 owner 的 redact，桥转成 `control(retract)`；界面靠 Turn task 的状态变化就够（§8） |
| `session.stop {sid, reason?, scope?}`（已有方法，语义变化） | 操作 | `scope: turn` 是针对消息的 stop；`scope: session` 按会话类型：UI 拒绝，Goal 休眠，Work 结束（D21，§2.3）。SC0 之后不再有“没有队列”的旁路，一律返回 `{index}` | SC0、SC2 | 已做 |
| `session.post {sid, text}`（已有方法，语义变化） | 操作 | 目标 Session 不收 user message 时拒绝并说明（D20） | SC0 | 已做 |
| `ui.conversation {peer_did, session_id?}` | 读 | `{current: UiSessionView \| null, generations: [{session_id, created_at_ms, finished_at_ms?, outcome?}]}`；`UiSessionView = {session_id, run_state, paused, needs_input, one_line_status, open_turn?: {turn, task_id}, unfinished_works, prompt: {frozen_at_ms, current: bool, changed: string[]}}`；`paused` 要带来源（用户暂停、父级暂停、异常中断），形状随 SC11 定 | SC4、SC7、SC11 | 已做（`current` 另带 `route_key`） |
| `session.work_tree {sid, unfinished_only?}` | 读 | `{agent_did, session_id, complete, sessions: WorkNode[]}`；`WorkNode` 是 `WorkCard` 加 `children: WorkNode[]`，Todo 绑定的卡片同样递归 | SC5 | 已做 |
| `ui.rotate {sid}` | 操作 | clean：归档当前 UI Session，下一条消息新建一代。有未结束 Work 时返回错误并带数量；成功返回 `{archived: sid}`，返回时登记表已显示结束 | SC6 | 已做（返回 `{archived, finished}`） |
| 聊天命令 `/clean`、`/new`、`/cancel`、`/resume` | — | `/clean`、`/new` 做与 `ui.rotate` 相同的事并回一句结果；`/cancel` 同 `/stop`；`/resume` 恢复这个对话被中断的 Turn 和它名下被中断的 Work（推定 16） | SC9、SC11 | 已做 |
| `sessions.delete {sid}` | 写 | 返回被删的 sid 列表（含后代） | SC8 | 已做（返回 `{deleted: [sid…]}`） |
| `deliveries.recent {limit, before?, route_key?}` | 读 | `{entries: [{at_ms, name, obj_id, url, mime?, size?, session_id, ui_session_id?, route_key?, source: report \| reply}], next_before?}` | AR1 | 已做（`before` / `next_before` 是行位置） |
| Desktop 深链 `?settings=<page>` | — | 打开 Settings 并定位到指定页面 | RT3 | 已做 |

Skills 的其余接口见 `src/frame/opendan/README.md`「Skills」，设备目录与分配的接口见 runtime TODO §11，本表不列。读 plan / goal 手册正文的方法随 Skills 页移到[独立 TODO](opendan-skills-page-ui-todo.md)。`session.stop` 对没有队列的 Session 的旁路（`27aea9efb`）在 SC0 之后不再需要。

## 4. 任务

每项写目标、落点、要点、验收。

### 4.0 任务状态一览（2026-11-10 晚，实施之后；基线 `7c91504b6`，未提交）

| 任务 | 状态 | 说明 |
| --- | --- | --- |
| HM1、HM3 | 已做 | opendan WebUI，mock e2e 通过，真实后端未验 |
| HM2 | 已做 | `home.summary`；WebUI 首页仍从 `sessions.query` 自己算（口径相同），用它的是 MessageHub 面板 |
| ME1–ME4 | 已做 | 后端组件测试通过；界面 mock e2e 通过 |
| RT1 | 已做 | 随 runtime TODO 提交（`35e72ce06`，按裁决的调整在 `27aea9efb`），本机 Zone DV 通过。调度器刷新的延迟没有量 |
| RT2、RT3、RT4 | 已做 | 首页瘦列表 + `#/runtime/<id>` 详情页 + 历史；Desktop 的 `?settings=<page>` 深链见 MH 一行 |
| SK1 | 不在本单 | 按 D19 移到[独立 TODO](opendan-skills-page-ui-todo.md) |
| WS1、WS2 | 已做 | |
| SC0–SC11 | 已做 | libopendan / opendan 全量回归通过（基线已知的 2 条除外）。推定 13 的两处修正、推定 15、16 按建议实施，owner 已确认（D30） |
| AR1 | 已做 | 回复正文里 `<attachment>` 标签带的文件没进索引（§8） |
| MH1–MH8 | 已做 | Desktop，mock e2e 通过，真实 Zone 未验；偏差在 §8 最后一条 |
| DX1、DX3、DX4 | 已做 | 协议文档、README、手册、skills TODO 的回链 |
| DX2 | 已做 | `UI_DATAMODEL.md` v0.18、PRD §12.8、Current UI Model Data §5.1 |

真实 Zone 上一项都没有验过：后端要重建 aios 镜像，见 §6.3。

### 4.1 首页骨架（HM）

- [x] **HM1 详情页外壳与路由。** `App.tsx` 增加首页样式的详情路由：`#/memory`、`#/runtime/<id>`、`#/workspaces`。这些页面用首页的样式（`home` 类、手机优先、中英文走 `i18n.ts`），顶部一个返回首页的链接；不要用观测页的标签布局。抽一个共用外壳组件，做成别的页面也能套用。`#/skills` 现在是观测页的一个标签（skills TODO 交付的页面），本单不动它（D19）；它以后搬不搬进这个外壳由 Skills 页的独立 TODO 定。验收：手机宽度下各页不横向滚动；刷新保持在当前页。
- [x] **HM2 `home.summary`。** `home.rs` 新方法，一次返回 §3 的摘要，供首页概况和 MessageHub 面板上部共用。Memory 部分复用 ME1 的缓存；Session 计数的口径与 `Home.tsx` 的 `isWorking` / `isWaiting` / `isFailed` 一致（把判定挪到服务端或保持两边同一份定义，二选一并写测试）。归入 `is_read`。
- [x] **HM3 首页布局调整与 Session 删除入口。**（D15）左列顺序：资料卡、工作概况、Memory、目标、Skills、工作区、Runtime。现在只缺 Memory 这一块（ME3），`SkillsCard` 已经在这个位置。`SessionDetail.tsx` 对可删除的会话显示“删除”按钮，二次确认，调用 SC8。同一页现有的 Stop 按钮按 §2.3 分会话类型（D21，依赖 SC2）：UI 会话上是“停止当前消息”，发 turn 范围，没有打开的 Turn 时不可点；Goal 会话上是“休眠”；Work 上仍是“停止”。异常中断的会话（D27，依赖 SC11）在首页的 Session 树和详情页上标“异常中断”，有“恢复”和“停止”按钮，恢复一个节点连同它名下一起中断的子会话；首页顶部在有这类会话时给一行提示和数量，带“全部恢复”：界面找出中断的会话里最靠上的那些（父会话没有中断的），逐个调 `session.resume`，剩下的靠级联。内置的 self_check 和记忆整理 Goal 的会话只在这里看得到，所以这里的入口不能省（推定 13）。暂停着的会话都写明来源：用户暂停、随父会话暂停、异常中断、随 Goal 休眠暂停（推定 15）；最后一种提示去唤醒那个 Goal。验收：未结束的会话、UI 会话没有删除按钮；删除后回到首页且列表不再有它；三类会话上 Stop 按钮的文案和效果各如上述；重启后中断的会话在首页可见并能恢复。

### 4.2 Memory（ME）

- [x] **ME1 只读接口。**（D2）
  - 落点：`lib_opendan/src/memory/` 新增 owner 视图（建议单独文件），提供 `overview`、`list_perceptions`、`list_cognitions`、按引用读取；`opendan/src/home.rs`（或新模块）暴露 `memory.overview` / `memory.list` / `memory.read`。不进 Agent State 门面：只有 owner 的界面用。
  - 排序按“最近添加”（D2）：认知从 `Memory::graph_view()` 取，按添加时间倒序（`GraphView::item_created_at`，即首个修订写入的时间）；之后被修订过的显示当前内容，并带修订时间。感知用 `load_perceptions(None)`（改可见性或加包装），按 `at_ms` 倒序，排除 `is_runtime()` 的旧记录。状态用 Graph 的 dispositions 加 `cleared` 推出（待整理、暂缓、已吸收、已丢弃）。
  - `overview.recent` 固定给最近添加的 5 条认知和 3 条感知；数量做成常量。
  - 游标用 `(排序键, 引用)`，不要用现有的 `PageCursor`（任何新感知都会让它失效）。
  - `overview` 按 `MemorySnapshot::token()` 缓存，快照没变不重新扫文件。`last_consolidation_at_ms` 取整理 Goal state 的 `last_successful_consolidation_at_ms`，同时返回该 Goal 的 id。
  - 验收：组件测试覆盖空库、只有感知、有删除和替代修订的认知、清理后的感知；翻页期间写入新感知不丢不重；`cargo test -p libopendan --test memory_component`、`cargo test -p opendan`。
- [x] **ME2 删除。**（D1）
  - 认知删除是必做项（owner：影响大的是认知，用户可以手工删掉一条）；未整理的感知走同一个接口顺带提供。先做认知这条路径并验收，再接感知。
  - 落点：`memory/consolidation.rs` 给 `Consolidator` 加 owner 发起的提交，仿照 `sweep_expired`：认知用 `SetStatus deleted`（带 `expected_revision`，不符返回冲突），感知用 `discarded` 处置后跑 `cleanup()`。服务端仿照 `decide_as` 取 `memory_consolidation` 租约，取不到返回 busy。`produced_by` 标明是 owner 操作，原因写进提交。
  - owner 删除不应推动“最近整理时间”：界面读的是 Goal state 里的时间，本路径不要去写它。
  - 已是终态的感知（已吸收、重复、已丢弃）拒绝，错误里带承接它的认知引用。
  - `runner/memory_batch.rs` 的撤回行：状态为 `deleted` 时不输出原陈述。
  - 验收：删认知后 `memory.list` 不再有它，`cognition.read` 返回 `deleted`，读过它的 Session 下一批收到不带原文的撤回行；整理 Goal 持有租约时删除返回 busy 且库不变；重复删除返回冲突。
- [x] **ME3 首页 Memory 区块。** （D2）`Home.tsx` 新卡片：认知总数、感知总数（附待整理数）、最近整理时间（点击去整理 Goal 的详情页）；下方先是最近添加的 5 条认知，再是最近添加的 3 条感知，每条显示摘要、时间、范围。整块右上角“查看全部”去 `#/memory`。这块不要 4 秒轮询：用较长间隔或只在进入页面和手动刷新时取。数据层、mock、类型同步补齐。验收：e2e 断言计数、预览条数、空状态、整理 Goal 不存在时的显示。
- [x] **ME4 Memory 详情页。**（D1）`#/memory`：认知、感知两个独立列表（手机上用页签，认知在前），按添加时间倒序，滚动加载；每条可展开看详情（`memory.read`）；删除按钮带确认，busy 时提示稍后再试并保留条目。不做添加和编辑。验收：e2e 覆盖翻页、删除成功、busy、已吸收感知没有删除按钮。

### 4.3 Runtime（RT）

本节只做 Agent 首页这一侧（D4）。设备怎么添加、怎么分配给 Agent，runtime TODO 已按两步流程做完（用户级目录 `agent.runtime_catalog`、daemon 设备两步化、Desktop 目录页、CLI，即它的 E1、E2、E5、E7；ssh 上提的 E3 按它的裁决 23 不做）。规划里“设备从 Settings → Cluster 管理添加”一句，现在对应 Desktop 的 Settings → Agent Runtimes（Add device 已从 Cluster Manager 搬过去）。本节的 RT2 是那份文档 R-E6 / E6 的细化：按规划改成“瘦列表 + 详情页”。停用和移除的规则以 §2.2 为准。

- [x] **RT1 设置语义修正与移除接口。**（D3、D4）已做：随 runtime TODO 实施并提交（`35e72ce06`），按 §2.2 的裁决 3、4 调整后在 `27aea9efb`。
  - 做了：`runtimes.settings` 返回 `configured_default_runtime_id`；停用不清 `default_runtime_id`（`home.rs` 的 `set_runtime_settings`、`app_schema.rs` 的 `validate_runtimes`、control_panel `agent_runtimes.rs` 的 `apply_update`）；私有 runtime 及其别名不能停用、不能移除；settings 里移除的条目记录转 `retired`，`usage` 保留，再分配回到 `active`；`runtimes.remove {runtime_id}`，默认指向它时一并清空，随后立即重新投影；`runtimes.settings_set` 的参数收成 `{runtime_id, enabled?, default?}`（runtime TODO R19）。
  - 与原文不同：停用和移除遇到正在运行的 Session 直接拒绝，没有确认参数（§2.2）；设备 runtime 的启用经 control panel 的 `agent.runtime_enable` 做独占检查；`host` 占位取消；原生运行只在 Lite，`native_agent_node` 在带容器的产品上恒为 `None`。
  - 测试：`opendan/tests/loader.rs` 的 `the_owner_switches_assigned_runtimes_and_work_in_progress_is_told`（settings、impact、remove、标记与通知，经 kRPC）；`lib_opendan/tests/runtimes.rs`（移除后退役并可回来、默认停用后保留）；`buckyos-api` 的 `native_agent_node_follows_the_default_runtime`（含默认被停用、带容器的产品）。本机 Zone 的 DV 见 runtime TODO §12.1 的 “R19 / RT1” 一行。
  - 没做：opendan 这条路径改完 settings 不会触发调度器刷新（control_panel 的 `agent.update` 会），节点上给这个 Agent 的执行权限要等调度器下一轮才收回。停用和移除后的这段延迟没有量；不可接受就补上刷新。
- [x] **RT2 首页列表瘦身与详情页。** 依赖 HM1（RT1 已做）。已有的部分：卡片上 ComputerUse 与基线已经只读，停用前先取 `runtimes.impact`，被拦时列出 Session。下面各条里没有注明“已有”的都还没做。
  - 首页每行只有名称、一行说明、启停开关。分三类显示：Provider（`connector.type == 'native'`，无开关，标“始终启用”）、Owner 默认（`runtime_id == configured_default_runtime_id`）、其他已分配（settings 里有条目）。`host` 占位记录已经取消，不用过滤。Lite 下私有 runtime 同时是 Owner 的机器（记录的 `connector.aliases` 不空），只显示一行并加标注；带容器的产品上不会出现这种情形。
  - 列表末尾是折叠的“历史”：`retired` 的记录，显示名称、最后使用时间和用过它的 Session 数，点进详情只读。到期的 runtime 不进历史：它仍在当前列表里，标“已到期”，续期后恢复（runtime TODO 裁决 26）。
  - `#/runtime/<id>` 承接 `RuntimeCard` 现有的全部内容和操作（检查、设为默认、基线、ComputerUse、扩展包、工具检查、用量与最近 Session）。runtime 自己的配置（权限、ComputerUse 档位、声明基线、期限）在这里只读显示，改它要去 Desktop 的 Settings → Agent Runtimes：它们是 runtime 的声明，不按 Agent 设（runtime TODO 裁决 24、§7.2 推定 2、§10）。所以详情页上的操作只有检查、设为默认、启停和移除（基线与 ComputerUse 的两个下拉已经去掉）。`runtimes.ext_tools` 只在详情页请求。
  - 停用是可逆的。规则按 §2.2 的裁决 4：上面有正在运行的 Session 时不能停用，界面列出这些 Session，没有“仍要停用”；没有就直接生效。等待的 Todo 和绑定的工作区只显示，不拦。`RuntimeCard` 的 `toggle` 已经是这个行为（已有），搬到新的行和详情页上即可。
  - 重写 `tests/e2e/home.spec.ts` 里断言 runtime 卡片的三条用例（Lite 单行、声明的环境与工具检查、进行中的 Session 拦住停用）和 `mock.ts` 的对应数据。验收对应规划场景“Owner 没有 Default Runtime”：只见 Provider 一行；停用后条目仍在、可重新启用，默认标记不丢。
- [x] **RT3 “添加 Runtime”引导与深链。** 首页 Runtime 区块的“添加 Runtime”打开说明：设备在 Desktop 的 runtime 目录里添加，并在那里分配给 Agent；这里只管这个 Agent 用不用它。按钮跳到 `${profile.desktop_url}/?settings=<page>`（在桌面窗口里要跳出 frame，做法同 `messageHubHref` 的 `outerTarget`）；Zone 外没有 `desktop_url` 时只显示说明。目标页面是 Settings → Agent Runtimes，页面 id `agent-runtimes`（`navigation.ts` 的 `AGENT_RUNTIMES_SETTINGS_PAGE`），已经落地。Desktop 侧只差 URL 这一步：在 `DesktopRoute.tsx` 处理 `?settings=<page>`，仿照 `?agent_setup`，校验页面 id 后调现成的 `openSettings(page)`；从 launch payload 到 `SettingsShell` 的 `initialPage` 这一段已有。首页现在只有一句文字提示（`runtimeManagedHint`），没有按钮；这句文案还写着去“用户与 Agent”里设置，一并改成目录页。验收：从 Agent 首页点击后 Desktop 打开 Settings 并停在目标页。
- [x] **RT4 从 Agent 移除 runtime。**（D4；“删除”就是从这个 Agent 移除，见 §2.1 去向表第 3 行）依赖 RT1、RT2。
  - 已有：后端 `runtimes.remove`、`runtimes.impact`，WebUI 数据层的 `removeRuntime`（kRPC 与 mock）。没有任何页面调用它。
  - 详情页对 Owner 默认和其他已分配的 runtime 提供“删除”，Provider 没有。先取 `runtimes.impact`：有正在运行的 Session 时不能删，列出它们，没有“仍要删除”（§2.2 裁决 4）。能删时确认框写明：只是不再让这个 Agent 使用它，设备还在目录里，可以再分配；并列出绑定在它上面的工作区、等待的 Todo、用过它的 Session 数和最近一个，这些只展示。
  - 删除后该条进入“历史”。
  - 原文要等的目录页已经可用（Settings → Agent Runtimes 能把它再分配回来），入口可以直接放出。
  - 验收：删除后首页当前列表不再有它，“历史”里有；新建 Work 指定该 runtime 被拒；目录页重新分配后回到当前列表，`usage` 还在。

### 4.4 Skills（SK）

本单在 Skills 上不做任何事（D19）。经过是这样的：

- Skills 的分组、禁用、安装与导入由 skills TODO 交付（D5–D7），opendan WebUI 的 Skills 页和首页的 Skills 卡片也是它交付的（§1.4）。
- 原来的 SK1 是在那之前先放一个只读的占位页，页面先到了，占位页不需要了。
- 那个页面与规划 §3 在形态上还有差距：不是手机样式、没有单个 skill 的详情页与正文、计数不分四组、停用项没有折叠小节、安装入口没有合成一个。按 D19，这些另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)，原 SK1 的内容和要求对照表已移到那里（任务 SP1–SP4，还没有裁决）。

对本单的影响只有两处：HM1 的外壳要做成 Skills 页以后也能套用；HM2 的 `home.summary` 里 Skills 只给计数。规划的验收场景“安装了 Skill，Provider Runtime 缺少依赖命令”用现有页面就能验（§6.4）。

### 4.5 工作区（WS）

- [x] **WS1 区块改版。** 标题改“工作区”（i18n 键和 `data-testid` 一并改名，更新 e2e）。默认收起，收起时显示最近使用的 5 张小卡片：标题、Runtime 名称、目录、Remote URL（有才显示）、描述（一行）。“最近使用”取绑定该工作区的 Session 的最近更新时间，没有 Session 的用记录的 `updated_at_ms`。Runtime 和目录始终显示，用来区分同名工作区。“查看全部”去 `#/workspaces`，那里保留现有完整卡片（检查、改 URL、迁移）。预览数量是规划的待定项，先定 5 并做成常量。
- [x] **WS2 手工添加向导。** 把“创建”“导入”两个表单合成一个分步向导：第 1 步选 Runtime，写明“这一步决定 Agent 在哪台机器上访问目录”，只列可用且允许工作区的 runtime；第 2 步选“已有目录”或“新建目录”并填目录、Remote URL、标题、描述；第 3 步提交并显示检测结果。提交仍用 `workspaces.import` / `workspaces.create`，失败时停在第 3 步显示原因并可返回修改。实施时确认导入失败不会留下登记，并记录目录里是否会留下元数据文件。检测细则等工作区专项文档，本项不新增检测逻辑。验收对应规划场景“用户在指定 Runtime 中手工添加工作区”。

### 4.6 会话控制后端（SC）

本节按 §2.4 区分已有协议与待实施变更。控制语义先在 `doc/opendan/protocol/Session Control Protocol.md` 同步；输入、持久字段、SDK / CLI、服务与界面按实际影响联动，不只更新界面用词。§2.1 的推定 8–11（D29）和 12–14（D28）已确认；涉及推定 15、16 和推定 13 两处修正的部分仍按待确认项处理。

- [x] **SC0 所有 Session 都带输入队列。**（D20；11-10 新增）控制命令（pause、resume、stop、decide）一律走队列；队列收不收 user message 是另一个属性。SC1、MH5 的前置。
  - 现状：`work` 模板的输入是 `InputChannel::None`，有队列的只有 UI、Goal、self_check、`plan_simple` 类、Todo 派发（`queued`）和 `--interactive` 的 Session（§1.7）。
  - 落点：`lib_opendan/src/template.rs`（`work` 的缺省输入、`settle_input`）；`api.rs` 的 `create_session`、`create_sub_session`（`SubSessionSpec.queued` 变成恒真后去掉）；`opendan/src/service.rs` 的 `import_skills`（它显式要了 `InputChannel::None`）。
  - 不收 user message 用现成的 `session.input_policy` 表达：值为 `none` 时，msg 和 Input event 在消费时被拒，记一条 `input_rejected: input_policy`（Session Input Protocol）。今天没有任何模板设置它。今天没有队列的 Session 改成“有队列、不收 user message”，其余不变（§2.1 推定 7）。投递方（`session.post`、`agent-session post`）在投递时就按目标的策略拒绝并说明，不要等消费时才落一条拒收记录。
  - 模板里的 `input = "none" | "queue"`（`agent.toml` 的 `[session.plan_simple]`、`opendan_agent_dev` 手册的示例）今天同时决定两件事：有没有队列，用户的消息能不能转给它。队列恒有以后这个键只剩后一个含义；是保留键名换含义，还是改成直接写 `input_policy`，实施时定并写回。`opendan/src/config.rs` 里 `[[loader.ui]]` 规则对“会话类必须有队列”的检查随之改成“必须收 user message”。
  - 首批材料怎么进要一并定。今天有队列的 Session 首批 `--msg` 是投进队列的，`create_session` 不允许有队列时带 `prompt.initial_inputs`；不收 msg 的 Session 得继续走 `initial_inputs`，这条检查要放宽。
  - 把“有没有队列”当判据的地方逐个改：`runner/inputs.rs` 的 `cascade_pause`；`runner/drive.rs` 的 `stop_children`；`host/mod.rs` 两处（等输入却没有通道的 Session 什么时候再看一眼）；`api.rs` 的 `decide_unqueued`（`xagent ctl decide` 对无队列 Session 走它）；`opendan/src/service.rs` 里 `session.stop` 的 Supervisor 旁路。`runner/outcome.rs` 的 `nobody_answers` 已经按“没有输入源或不等用户”判断，不用动。新建的 Session 走不到这些分支以后，按不做兼容的约定删掉；升级前建的、还没结束的无队列 Session 不管。
  - 没有队列客户端的宿主（`xagent` 不带 `--queue-dir`、也不在 Zone 里）今天建不出带队列的 Session。是缺省给它本地文件队列，还是这种宿主保留无队列，实施时定并写回。
  - kmsg 上每个 Session 多一个队列和一个订阅，驱动者在 Session 不再收输入时删掉（现有规则）。量一下 Todo 会话很多时的开销，记入 §8。
  - 同步 `doc/opendan/protocol/Session Control Protocol.md`、`Session Input Protocol.md`、`Session Directory Protocol.md`，以及 `opendan/README.md`「操作」一节里讲“没有队列的 Session”的几句。
  - 验收：聊天里直接建的 simple Work 能 pause、resume、stop，三者都返回 `index`；向它 `session.post` 文本被拒并说明原因；它的首批消息照常进入第一个 Turn；plan 暂停时级联到它名下每一个未结束的子会话。依赖无队列行为的用例（`opendan/tests/loader.rs` 的 `the_owner_stops_a_work_session_without_a_queue` 等）改过。全量 `cargo test -p libopendan -p opendan --no-fail-fast -- --test-threads=1`。
- [x] **SC1 pause / resume 接口。**（D8 的后半句、D20、D23）依赖 SC0。只加接口，不改控制命令：`service.rs` 增加 `session.pause` / `session.resume`，归入 `is_operation`。后台 Work 的暂停走现有的 Session 暂停流程，级联到它自己的子会话，行为不变。前台消息不提供暂停（D23），所以原来计划的不级联选项不做，`ControlCommand::Pause` 不加字段。SC0 之后所有 Session 都有队列，这两个方法对任何未结束的 Session 都能投，级联也不再跳过谁。`session.resume` 同时是恢复异常中断的入口，级联规则的扩展在 SC11。协议文档里 pause / resume 的合同不变，只在 `opendan/README.md` 补新方法。
  - 暂停后发消息再恢复（§2.5 的 2b）还差提示词这一环：`behaviors/chat_route.toml` 和 `ui_core` 手册写的是把用户的话转给“还在运行”的 plan。plan 暂停着的时候 Agent 可能当它不在运行，转而另起一次分析。把这句改成“没有结束的 plan，包括暂停中的”，并说明暂停中的 plan 恢复后才处理这条消息，回复用户时照实说。核对对话里那行工作状态对暂停中的 plan 显示什么，Agent 要能从它看出是暂停。
  - 验收：暂停一个带 Todo 会话的 plan，它名下未结束的子会话都暂停；向暂停中的 plan 投一条消息，消息留在队列，resume 后 Turn 从检查点继续并处理这条消息；plan 暂停期间在对话里说一句修改，Agent 把它转给这个 plan，没有另起新的工作；对聊天里直接建的 simple Work 能 pause、resume；现有 pause 测试不变。
- [x] **SC2 区分针对消息的 stop 和针对 Session 的 stop。**（D9、D17、D21、D26；语义表在 §2.3）
  - 针对消息的 stop（`scope: turn`）：立刻打断在飞的推理和工具，不再发起新的，当前 Turn 以 stopped 关闭，占位改成“已停止”，Session 回到等待，上下文保留，用户马上能发下一条（§2.5 的 1a）。打断是今天已有的行为（§1.7），这一条不改执行语义。这个 Turn 已经做过的步骤和工具结果留在历史里，下一条消息到来时模型看得到。
  - 先写测试确认现状：UI Turn 在 `analyze_intent` 子 context 里收到 turn 范围 stop 时，Turn 是否以 stopped 关闭、Session 是否回到等待、占位是否被改成“已停止”文案（Agent 包 `i18n` 的 `[outbound] stopped`）。按结果修 `runner/outcome.rs` 的 `turn_stop && !site.child` 分支。
  - 把针对消息的入口都改成发 turn 范围：`/stop`（`bridge/msg.rs` 今天发的是 session 范围）；`CancelBridge`（`tasks.rs`）对 UI Session 改用 turn 范围，Goal 已经是，Work 保持 session 范围（D29）。没有打开的 Turn 时不生效，`/stop` 这时回一句提示。
  - 针对 Session 的 stop（`scope: session`）在 `runner/inputs.rs` 的 `apply_control` 按会话类型分派。今天那里只有一条特例：Goal Session 一律降成 turn。改成（D29）：UI Session 拒绝，并说明要清空上下文用“开始新对话”；Goal Session 让它的 Goal 休眠，走 `goals.set_status` 置 `paused` 的同一条路径；Work 不变。`session.stop` 对 UI Session 在服务端直接返回错误，不要等消费时才记一条未生效。
  - Goal 休眠的生效点（D26；读法已确认，D28）：置休眠后不再发起下一次推理。今天推理边界（`runner/hook.rs` 的 `boundary`）只看 stop、Turn stop 和 pause，Goal 的闸门只在驱动循环的提交点读，所以当前 Activation 会推理到自然结束。改成在推理边界也看 Goal 状态：已休眠或已归档时在这个边界打断，Activation 以 stopped 收尾并照常写回 Goal State。在飞的这次推理和它的工具不打断。每个边界多读一次 `goal.json`，只对 Goal Session 做。休眠不走 `control(pause)`：Goal Session 自己的 `state.paused` 不动，唤醒后等下一次运行机会，不续上被收尾的那次 Activation。
  - Goal 派生的 Work（推定 15，待确认）：休眠时，这个 Goal 派出去、还没结束的 Work（`origin.goal`）一并暂停，暂停原因注明是 Goal 休眠；唤醒时只恢复因此暂停的，用户自己暂停的不动。今天宿主只在 Goal 归档时停这些 Work（`host/mod.rs` 的 `stopped_from_above`），休眠时不管，它们照常推理到结束。落点在宿主看 Goal 状态的那一轮（`host/goals.rs`）：发现 Goal 是 `paused` 就给这些 Work 投会话暂停，回到 `active` 就投恢复，做法比照 `cascade_pause`。Work 的暂停照常级联到它自己的子会话。这是暂停的又一个来源，登记表的来源字段（SC11）要能表示。唤醒之前这些 Work 的结果不会回到 Goal。
  - 用词：Goal 的手工 disable 叫“休眠”，恢复叫“唤醒”。opendan WebUI 目标详情页今天写的是“暂停 / 已暂停 / 恢复”（`web/src/i18n.ts` 的 `opPause`、`gsPaused`、`opResume` 等，按钮在 `GoalDetail.tsx`），改掉；英文文案实施时定。状态值 `paused` 和 CLI 的 `goal pause | resume` 不改名（D29）。
  - 更新 `doc/opendan/protocol/Session Control Protocol.md` 的停止作用域、生效边界与 Goal 映射，联动 Directory / Input 的字段与消费说明；更新 `opendan/tests/loader.rs` 相关用例、`ui_core` 手册里对 `/stop` 的说明、`opendan/README.md`「操作」一节。
  - 验收：停止一条正在处理的消息后，同一个 UI Session 接着处理下一条消息，上下文还在，这个 Turn 之前已创建的 Work 不受影响。对 UI Session 发 session 范围的 `session.stop` 被拒，会话不变。对 Goal Session 发 session 范围的 stop 后，Goal 的状态是 `paused`，不再有新的 Activation，唤醒后恢复；发 turn 范围的只结束当前 Activation。Activation 进行中让 Goal 休眠，它在下一个推理边界以 stopped 结束，之后没有新的模型调用。Goal 有一个正在跑的派生 Work 时让它休眠，这个 Work 暂停，不再有模型调用；唤醒后 Work 继续，结束时 Goal 收到结果（推定 15）。对 Work 发 session 范围的 stop 仍以 stopped 结束。
- [x] **SC3 暂停状态进 TaskMgr。** `TaskService` 增加暂停上报，`TurnTasks::update` 在 Session 暂停时把 Turn task 置为 TaskMgr 的 `Paused`，恢复后回到 Running。异常中断（SC11）走同一个状态，task 的 message 写明是中断；占位消息靠它显示“异常中断”（MH4）。TaskMgr 一侧发起的 Pause / Resume 请求不桥接（Task Center 上不出现这两个按钮，或出现但不生效，实施时确认并记录）。验收：暂停后 `getTask` 的状态是 Paused，Desktop 的 `taskNodeState` 得到 `paused`。
- [x] **SC4 `ui.conversation`。** 给定对话方和 MessageHub 的会话 id，返回当前这一代 UI Session 的状态和历代列表（匹配规则：`outbound` 的 `to` 等于对话方；`to_session` 等于会话 id，或会话 id 是 `dm:` 开头且 `to_session` 为空）。当前一代取未结束的最新一条。归入 `is_read`。这是面板下部、提示词标签、Clean History 入口共用的一次读取。
- [x] **SC5 `session.work_tree`。**（D12）在 `work_card.rs` 增加按 Session 汇总的树：该 Session 的全部子 Work（可只取未结束的），plan 的 Todo 绑定卡片递归，非 Todo 的子 Work 挂在 `children`，深度上限 4。`complete` 的含义同 `work_cards`。现有 `session.work_cards` 不变。验收：两个并列 Work、一个 plan 带三个 Todo、其中一个 Todo 的会话又建了子 Work 的夹具，结构与预期一致；全部结束后 `unfinished_only` 返回空且 `complete` 为真。
- [x] **SC6 归档当前 UI Session（clean）。**（D11、D17、D18）
  - 语义（D18）：保存当前聊天会话对应的 UI Session，再有消息时新建一代 UI Session 并关联到同一个聊天会话。MessageHub 的“开始新对话”（MH6）和聊天命令 `/clean`（SC9）都是它。新一代是懒创建的，按同一个 `route_key` 关联（D11 c，§2.1 推定 3）。
  - `ui.rotate {sid}`：确认是本进程驱动的 UI Session；有未结束子 Work 时拒绝并返回数量；否则结束旧会话，等登记表显示结束后返回（有上限，超时返回进行中）。
  - 结束旧会话不能再用 stop 控制命令：SC2 之后针对 Session 的 stop 到了 UI Session 会被拒绝（D17、D21）。归档要有自己的路径，两个候选：新增一条只给归档用的控制命令；或者用 Supervisor 的 `StopSignal`（`Supervisor::stop` 现成的做法）。选哪条实施时定并写回。旧会话的结局仍记 `stopped`，worklog 记下是归档以及谁发起的（D11 d，§2.1 推定 2）。
  - 修既有竞态：`session_for` 选当前会话时，对登记表里“未结束”的候选再看一眼 `state.json`，已请求停止或已结束的不再投递；该 inbox 的记录留到旧会话结束后由新一代接收。对应用例 `inbox_messages_are_answered_along_the_same_conversation` 的偶发超时。
  - 旧会话的目录、worklog、Session id 都保留，Memory 里的原文引用继续可用。
  - 验收对应规划场景“Clean History 后回查旧 Memory”：归档后下一条消息由新的 Session id 处理，`session.read` 旧 sid 仍能读到 worklog；归档后立刻发消息不丢。
- [x] **SC7 提示词版本状态与压缩时刷新。**（D14）
  - 第一步（只读）：在 `state/behaviors.rs` 加比较函数，把 `prompt.frozen` 的身份文本、已冻结 behaviors、cmd_skills 与 catalog 当前内容逐项比较，返回是否一致和不一致的项目名；`ui.conversation` 的 `prompt` 用它。比较的另一边是“现在新建一个同类会话会冻结到什么”：除了文件内容，还要算上 owner 改过的 UI 集合和被禁用的手册（`.meta/skills_state.json`），这两样不反映在 `behaviors.revision` 里（§1.7）。
  - 第二步：读 `runner/live.rs` 的上下文重写和 `runner/history.rs`，确认压缩后系统前缀是怎么重建的，把结论写回本单 §8，再实现：UI / group 会话在压缩提交点重新冻结（替换 `prompt.frozen`，worklog 记 `control_applied`），Work 会话不动。重新冻结后 cmd_skill 的可用性按冻结规则重算。
  - 验收对应规划场景“Agent root fs 的系统提示词更新”：改 `role.md` 后旧会话 `prompt.current` 为假、新会话为真；旧会话发生一次压缩后变为真，且压缩前后的对话不重复不丢。
- [x] **SC8 删除 Session。**（D15）登记表增加删除：只接受已结束的 Work / self_check，后代全部已结束才允许，连同后代一起删除目录和登记项；`state/perception/<sid>.jsonl` 不动（Memory 的来源）；登记表旁的 `.runtime_unavailable/<sid>` 标记一并清掉。门面增加 `sessions.delete` 并归入 `is_write`；`permission_tests` 现在把它当作未归类的方法断言被拒（`read_only_agent_cannot_edit_profiles_or_call_unclassified_methods`），一并改。验收：有未结束后代时拒绝；UI、Goal 会话拒绝；删除后 `sessions.query`、`session.read` 都查不到。
- [x] **SC9 聊天命令 `/clean`、`/new`、`/cancel`。**（D17、D18；11-10 新增）依赖 SC2、SC6。给没有界面的渠道用，今天就是 Telegram。
  - 现状：`opendan/src/ui.rs` 的 `bridge_ctx` 只注册了 `stop`。桥已经有“由宿主自己处理的命令”这条输出（`SlashCommand::App` → `MsgBridgeOutput::AppCommand`），`drain` 收到后只确认、什么都不做。Telegram 的菜单（`msg_center/src/tg_tunnel.rs` 的 `TG_BUILTIN_COMMANDS`）登记了 `new`、`clean`、`stop`、`cancel`、`info`、`list`、`switch`、`help`，除 `stop` 外发出去都被当成普通文字交给模型。
  - `clean`：注册为 `SlashCommand::App`，`drain` 里对这条消息所在聊天会话的当前 UI Session 做 SC6 的归档，回一句结果。有未结束的 Work 时拒绝，回复里列出它们（D11 b，§2.1 推定 5）。没有当前 UI Session 时回“已经是新对话”。
  - `new`：Agent 一侧与 `clean` 相同（§2.1 推定 4）。Telegram 是单 session：`build_telegram_ui_session_id(bot, chat)` 给每个聊天固定一个会话 id。
  - `cancel`：注册成 `stop` 的别名（§2.1 推定 6）。
  - `resume`（推定 16，待确认；依赖 SC11）：对这条消息所在对话的当前 UI Session 发 `control(resume)`，恢复被中断的 Turn，并级联到它名下被中断的 Work。UI Session 自己没有中断、只是名下的 Work 中断了的时候，逐个恢复这些 Work 里最靠上的。回一句结果：恢复了几个；没有可恢复的就说没有。Telegram 菜单里今天没有这一条，加上。
  - Telegram 菜单的说明：`clean` 现在写的是 “Delete current session and create a new one”，与“保存”的语义不符，改掉；`new` 在单 session 下与 `clean` 的说明对齐。`info`、`list`、`switch`、`help` 仍然没有处理，不在本项。
  - 回复的发送路径与 SC2 里 `/stop` 的提示共用，文案进 Agent 包的 `i18n`。更新 `ui_core` 手册和 `opendan/README.md`。
  - 验收：opendan 集成测试里发 `/clean`，下一条消息由新的 Session id 处理，旧 sid 仍可读；有未结束的 Work 时被拒且会话不变；`/new` 与 `/clean` 结果相同；`/cancel` 与 `/stop` 结果相同；重启后对中断的对话发 `/resume`，被中断的 Turn 继续并给出回复，发 `/stop` 则放弃它、下一条消息照常处理。真实 Zone 上要配一个 Telegram bot 才能走，没有条件就如实标未验。
- [x] **SC10 撤回输入。**（D25；11-10 新增）依赖 SC2。§2.5 的 1b：停止并删掉这条输错的消息。
  - 现状：没有撤回输入的控制。MessageHub 发出的撤回请求（redact 关系消息）到了桥被当作噪声，Session 不知道（§1.7）。
  - 控制：`ControlCommand` 新增一条，以被撤回输入的 `key`（消息的 ObjId）为目标，名称暂定 `retract`。`runner/inputs.rs` 的 `apply_control` 按目标输入的状态分三种：
    - 还在队列里、没有被消费：丢弃，不进任何 Turn，记一条 `input_rejected`，拒绝原因新增 `retracted`。这是真正的“没输入过”。
    - 已被消费、它的 Turn 还开着：按针对消息的 stop 收尾（SC2），worklog 记一条撤回记录。下一个 Turn 的历史里，这条用户消息后面带一行“用户撤回了这条输入”，措辞进 Agent 包的 `i18n`。历史由 worklog 重建（`runner/history.rs`），worklog 不改写，所以这个 Turn 已执行的步骤都还在。Turn 合并了多条消息时，只把被撤回的那条标成撤回，Turn 照样停止。
    - Turn 已经关闭：记 `applied = false`，不动历史（推定 14，已确认）。
  - 入口：桥（`lib_opendan/src/bridge/msg.rs`）认出 owner 发来的 redact，目标是它投递过的消息时转成这条控制，不再当噪声丢掉。MessageHub 现有的“撤回 / 删除”就成了入口，不用另做发送路径。界面需要同步知道走了哪一种时，再加 kRPC `session.retract {sid, key}`（§3）；先确认只靠 Turn task 的状态变化够不够，够就不加。tunnel 渠道没有撤回入口，本项不做。
  - 时间线：用户那条消息按 MessageHub 现有的 redact 显示。占位消息由 Agent 一侧撤回，这一问一答都不再显示；做不到时占位改成“已撤回”文案。选哪种实施时定并写回。
  - 感知：如果这条输入在撤回前已经被记进感知流，错的信息还会进记忆。实施时核对捕获发生在哪个点；已经记下的按 ME2 的同一条路径处置为 `discarded`。
  - 同步 `Session Control Protocol.md` §3、§9.2，`Session Input Protocol.md`（新的拒绝原因与消费规则），重新导出 schema，更新 `fixtures/14_input_bus` 的拒绝原因；`ui_core` 手册加一句撤回的效果和边界。
  - 验收：Agent 正忙时发出的第二条消息在被消费前撤回，之后任何 Turn 的历史里都没有它；正在处理的消息被撤回后 Turn 以 stopped 关闭，下一条消息的历史里有这条输入、已执行的步骤和撤回标记；已回复的消息被撤回，历史不变；不是 owner 发的 redact 不生效。
- [x] **SC11 重启后的异常中断与手工恢复。**（D27、D23 的括号；11-10 新增）依赖 SC1 的 `session.resume`。
  - 现状：宿主启动后把未结束的 Session 全部自动接着跑，没有“异常中断”状态（§1.7）。
  - 判定（推定 13，D28 已确认；两处修正待过目）：宿主进程启动后的第一次登记表扫描里，由本宿主驱动、没有暂停的未结束 Session，按类型看：
    - Work Session：不是在等用户的都算，不管这一刻有没有打开的 Turn。包括推理中、工具执行中、在等跟踪的 task 或子会话（`follows_work`）、刚建好还没开始的。等自己发起的工作的 Work 必须算进来：它会按 `tick_secs` 自己产生 tick 并推理（§1.7），不停住的话 plan 的子会话中断了，plan 自己还每十分钟推理一次。
    - UI、Goal、self_check Session：有打开的 Turn，且不是在等用户回答。空闲的对话、两次 Activation 之间的 Goal 不算，新消息和下一次运行机会照常处理。
    - 在等用户回答的都不算（提了问题、回答态）：它不会自己推理，用户一回答就照常继续。标成中断的话，回答进了队列却没有反应。实施时确认这种会话在 state 里是什么形状（`needs_input`，还是把问题当成跟踪的 task），判定按那个字段写。
    - 判定按 `state.json` 做，不按登记表：提交时先写 state 再上报登记表（§1.7），断电时登记表可能还是上一步的。登记表只用来找出要看的 Session；驱动循环拿到 lease、读到 state 之后再判，判定和写标记是同一步。
    - `Supervisor::tick` 今天没有“第一次”的标记，要加。之后新建的 Session、驱动循环出错后重起的都不算。断电、崩溃、正常重启不区分。
  - 表示：沿用暂停状态（D23 的括号）。写 `state.paused`，原因是专门的中断原因，`by` 是宿主；worklog 记 `control_applied{command: pause}`，来源写法同驱动者自己的停止请求（`input.src: "_runner"`）。写 state 要持有 lease，所以是驱动循环在这次启动时先写标记、再停住，不是 Supervisor 直接改文件。标记写入之后不推理、不重放工具，检查点恢复留到 resume。
  - 原因要让界面分得出：登记表的 `status.paused` 今天是布尔值，改成带来源（用户暂停、父级暂停、异常中断；推定 15 成立的话还有随 Goal 休眠暂停）。`session.read`、`session.work_cards`、`session.work_tree`、`ui.conversation` 跟着带出；opendan WebUI 和 Desktop 里读 `paused` 的地方一并改。
  - 恢复：`resume` 清掉标记，从检查点继续，走现有的恢复路径；在途的工具调用按“结果未知”物化，不重放。按任务树恢复：resume 一个 Session 时，级联到它名下原因同为这次中断的未结束子 Session。`apply_control` 的 `Resume` 分支今天只认原因是 `parent session <sid> was paused` 的子会话，扩展它。只恢复子会话不带动父级。
  - 级联的起点必须是一个暂停着的 Session，这一点不改：今天对没有暂停的 Session 发 resume 什么都不做（§1.7）。会出现父会话没中断、子会话中断了的情形：空闲的对话名下有中断的 Work，两次 Activation 之间的 Goal 名下有中断的派生 Work，在等用户回答的 Work 名下有中断的子会话。这时由调用方找出中断的会话里最靠上的那些，逐个发 resume：界面的“全部恢复”（HM3、MH5）和 `/resume`（SC9）都这么做。不为此改 resume 的合同。
  - 停止：中断状态下可以不恢复、直接停。针对消息的 stop 关掉这个 Turn，并清掉中断标记；今天 stop 不清 `state.paused`，不改的话 UI Session 会一直停在暂停。用户暂停的标记不受这条影响，仍然只有 resume 能清。Work 的 Session 级 stop 照常结束，连同它名下中断的子会话。中断的 Goal Session 被置休眠时同样处理：关掉中断的那次 Activation 并清掉中断标记，否则唤醒之后它还停在暂停，等不到下一次运行机会。
  - 中断期间到达的输入留在队列，这是暂停的现有行为。有界面的渠道上输入框是禁用的（MH4）。tunnel 渠道上用户照样能发：普通消息到了一个中断的 UI Session 时，在入队之外回一句提示，说明上一条因重启中断，发 `/resume` 继续或 `/stop` 放弃；同一次中断只提示一次（推定 16）。落点在 `opendan/src/ui.rs` 投递消息的地方，回复的发送路径与 `/stop` 的提示共用。
  - 通知：这次扫描之后给 owner 发一次通知，说明中断了几个会话、去哪里恢复，做法仿照 `opendan/src/loader.rs` 的 `mark_runtime_sessions`。
  - 自动恢复不做（D27，下一期）。中断原因和时间要记全，下一期按配置自动 resume 时用得上。
  - 影响面：开发和 DV 里 `start.py` 重启、重建容器之后，进行中的会话都会停在中断，要手工恢复。依赖“重启后自动接着跑”的用例（`opendan/tests/loader.rs`、`lib_opendan/tests/` 里经 Supervisor 的恢复用例）逐个看。驱动层的检查点恢复规则（协议 fixtures 的 `next`）不变，变的是宿主什么时候开始推进。`xagent` 命令行直接驱动的 Session 不经过 Supervisor，行为不变。每个宿主只处理自己驱动的 Session。
  - 同步 `Session Control Protocol.md` §5、§7、§8、§9.2，`Session Directory Protocol.md` 的恢复规则，`Agent State Protocol.md` 的登记表字段，重新导出 schema；`opendan/README.md`。
  - 验收：准备一个停在 run 中途的 Work、一个带两个进行中 Todo 会话且自己正在两次 Turn 之间的 plan、一个两次 Turn 之间在等长任务的 simple Work、一个提了问题在等用户回答的会话、一个空闲的 UI Session（名下挂着前面那个 plan）、一个重启前被用户暂停的 Work。重启宿主后：
    - 前三组都是暂停且来源是异常中断。等过一个 tick 周期，没有新的模型调用，plan 和等长任务的 Work 也没有。
    - 在等回答的会话没有被标成中断，回答之后照常继续。
    - 空闲的 UI Session 不受影响，新消息照常处理；用户暂停的那个来源没有被改写。
    - 对那个空闲的 UI Session 发 resume 不生效，它名下的 plan 仍是中断。
    - resume plan 之后它和两个 Todo 会话都继续，已执行的工具不重放。
    - 按“全部恢复”的做法对剩下的中断会话里最靠上的逐个发 resume，全部继续。
    - 对中断的 UI Turn 发针对消息的 stop，会话不再暂停，下一条消息照常处理。
    - 状态已经写进 `state.json`、登记表还没更新时断电（夹具里把登记表改回上一步），判定结果以 state 为准。
    - 经 tunnel 给中断的对话发一条普通消息，收到一次提示；发 `/resume` 后被中断的 Turn 继续，随后处理这条消息。

### 4.7 产物索引（AR）

- [x] **AR1 交付索引与查询。**（D13）
  - 在交付成功的地方追加一条记录（落点在 `runner/reports.rs` 调用 `ObjectStore` 之后、能拿到 Session 上下文的位置；`--dev` 没有对象存储时不写）。记录交付时间、名字、对象 id、`obj://` 地址、类型和大小、交付它的 Session、它所属的顶层 UI Session 与 `route_key`、来源是报告还是回复。同一对象同一 Session 重复交付只记一次。
  - 文件放在 AgentRoot 的 state 下，只追加；写明格式并补读写测试（`harness/trigger_rules.md` §3.2）。
  - `deliveries.recent`：按时间倒序分页；给 `route_key` 时只返回该对话的。归入 `is_read`。
  - 验收：Work 的报告带一个文件、UI 回复带一个附件，两条都能查到且会话归属正确；按 `route_key` 过滤只剩本对话的。

### 4.8 MessageHub 侧面板（MH）

共同要求：面板的数据只在面板打开时取；刷新沿用 `conversation/worksessions/watch.ts` 的做法（TaskMgr 树提示驱动，15 秒兜底，`complete` 后停止）；OpenDAN 调用统一走 `agentServiceClient`；调用失败（不是 owner、Agent 离线）时相应区块显示不可用，不影响面板其余部分。每项都要补 mock（`mock/` 下新增 OpenDAN 侧的假数据源）、i18n（`src/i18n/messagehub.ts` 中英文）、`UI_DATAMODEL.md`。

- [x] **MH1 统一面板外壳。**（D16）新组件（例如 `AgentSessionPanel.tsx`），上下两部分，中间可滚动；`MessageHubView.tsx` 的 `detailsTarget` 增加一种取值。桌面端在 `ConversationView` 头部加 Info 按钮，开关状态记在本地；手机端全屏，保留点标题进入，新增从会话视图左滑进入、右滑返回（MessageHub 里没有现成手势，注意不要和消息列表的滚动、语音录制的按压冲突）。只对“对方是自己名下 Agent 的会话”启用；其余会话入口和详情栏不变。头像、名字、副标题的点击在 Agent 会话里都指向新面板。`CodeAssistantAppPanel` 保持可编译。验收：`messagehub-ux.spec.ts` 增加桌面开关、抽屉模式、手机全屏和手势的用例；非 Agent 会话的现有用例不变。
- [x] **MH2 上部：Agent 全局区。** 依赖 HM2。压缩的头部（小头像、名字、在线状态、DID 可复制）；状态摘要一行或一组小数字（进行中、等待、失败、感知与认知数、最近整理时间），来自 `home.summary`；“打开 Agent 首页”用现成的 `openAgentHome`。实体设置：免打扰和置顶接到 `updateEntityPreferences`（顺带修好 `EntityDetails` 里没接线的两个按钮）；实体处于免打扰时，新建的会话继承静音（改 `api/store.ts` 建会话的路径）。验收：免打扰后未读总数和托盘不再计入该 Agent，新建会话也是静音的。
- [x] **MH3 下部：当前会话与子会话树。** 依赖 SC4、SC5。显示会话标题和描述、Agent 侧状态（处理中、空闲、异常中断、等待回答，加一句话状态）、当前是第几代会话。“Sub Sessions”区块用 `session.work_tree {unfinished_only: true}` 渲染：卡片行复用 `WorkSessionCards.tsx` 的 `WorkCardRow` 和 Todo 列表组件，把它们改成能递归；默认展开到 Todo 列表。点卡片打开该 Session 的工作日志（现有的消息详情页）。没有未结束的 Work 时显示空状态。现有 `SessionDetails` 的内容（成员、偏好、共享状态、管理、技术信息）收进面板底部的折叠区，不丢功能。验收对应规划场景“一个聊天会话创建多个后台工作”。
- [x] **MH4 消息控制与输入框联动。**（D9、D10、D23、D25、D27）依赖 SC2、SC3、SC10、SC11。
  - 在占位消息的任务区（`MessageTaskArea` 和 `MessageAgentWork` 的头部）加按钮。运行中：“停止”和“停止并撤回”，没有“暂停”（D23）。异常中断（SC11）：“恢复”“停止”，并写明是重启造成的中断。停止调 `session.stop {scope: "turn"}`；撤回对用户那条消息走 MessageHub 现有的 redact（SC10）；恢复调 `session.resume`。Session id 和 Agent DID 取自 Turn task 的 input（现有的 `taskWorklogTarget`）。点击后到状态变化前显示“停止中”“恢复中”。
  - “停止并撤回”的确认文案写明边界（§2.5 的推论）：Agent 已经做了的事不会撤销，只是不再继续，这条消息从对话里拿掉。Turn 已经结束的消息上不给这个入口（推定 14，已确认）；消息菜单里原有的“撤回 / 删除”不变，只改时间线。
  - 停止不停这条消息已经发起的 Work（D8、D9）。停止或撤回之后，如果这个 Turn 建过 Work 且还有没结束的，在占位下写明“这条消息发起的 N 个后台任务还在运行”，逐个给“停止”（调 `session.stop {scope: "session"}`，同 MH5 的节点控制）。数据用现成的 `session.work_cards {sid, task_id}`。确认框里也先说一句有几个后台任务不会跟着停。
  - 输入框：从时间线和任务状态推出“前台消息状态”（放在 `conversation/input/inputState.ts` 旁边，纯函数，可单测），运行中或异常中断时禁用发送，并在输入框上方显示原因和同样的控制按钮；回答态优先。`ConversationComposer` 增加禁用入参。已排队未发出的消息保持排队。
  - （D17）回答态的 Stop 按钮改调 `session.stop {scope: "turn"}`，不再发 `/stop` 文本。
  - 把 `MessageTaskArea` 第一行的英文实时状态（`working (analyze_intent)` 等）本地化，复用 `worksessions/model.ts` 的 `localizeLine`。
  - 验收：规划场景“用户暂停正在分析的消息”按 D23 改成停止。处理中不能发送；停止后可发送且上下文还在，时间线上旧输入还在、没有 Agent 的回复；停止并撤回后旧输入也不在；停止一条已经建了 Work 的消息后，占位下列出这个 Work，点停止后它结束；Turn 进入等待子任务后可发送；异常中断时不能发送，恢复后继续，停止后可发送。deno 数据模型测试覆盖状态函数的全部分支。
- [x] **MH5 子会话树节点控制。**（D8、D20、D24、D27、D28）依赖 MH3、SC0、SC1、SC11。对应 §2.5 的 2b、2c 和 4。树上不加直接发消息的入口：纠正方向的消息由用户在对话里说，UI Session 的 Agent 转给 plan（今天已有）；plan 暂停时这条消息留在它的队列里，恢复后处理。纯 Work 不收消息（D24），节点上只有暂停、恢复、停止。异常中断的节点标明来源，有“恢复”“停止”；恢复一个节点连同它名下一起中断的子会话（SC11），所以恢复 plan 就恢复了它的 Todo 会话。后台 Work 的暂停走 Session 的暂停流程：未结束的节点上有“暂停”，已暂停的有“恢复”“停止”，调 `session.pause`（缺省级联，暂停 plan 就暂停它的 Todo 会话）/ `session.resume` / `session.stop`。停止要二次确认。直接对 Todo 的会话发控制不会写 plan 的 `TodoAttempt` 暂停标记（那是 `agent-session todo pause` 写的），卡片状态以会话自己的 `paused` 为准；验收时确认 plan 的下一次 tick 不把它当异常。已停止的节点上放一个“在 Agent 首页查看”的链接，删除在那边做。SC0 之后所有 Session 都有队列，树上每个未结束的节点都能暂停、恢复、停止，界面不用区分节点的种类。plan 暂停期间对话里的 Agent 照样把用户的话转给它，这一环是提示词的改动，在 SC1；这里验收时走一遍。树上有中断的节点时，区块顶部给一行提示和“全部恢复”：界面找出中断的节点里最靠上的那些，逐个调 `session.resume`，剩下的靠级联（SC11）。对话本身没有中断、只是名下的 Work 中断了的时候，这是面板里唯一一处能一次恢复的地方。
- [x] **MH6 Clean History 入口。**（D11、D18）依赖 SC6。面板下部的会话控制区加“开始新对话”，调 `ui.rotate`，与聊天命令 `/clean` 是同一个操作；MessageHub 是多 session 的，new 对应现有的“新建会话”，不动。确认框说明：Agent 从全新上下文开始，旧会话归档可查，消息记录不删除。有未结束 Work 时按钮禁用并说明原因。成功后在时间线插入一条本地分隔标记（按归档时间定位，取自 `ui.conversation` 的历代列表，刷新后仍在）。头部 ⋮ 菜单的“清空聊天记录”保持原样，两处文案要能让人分清。调用期间禁用输入框，返回后恢复。
- [x] **MH7 提示词版本标签。** 依赖 SC7 第一步。面板下部显示“与 Agent 当前版本一致”或“Agent 版本已更新”，后者的提示里写“开始新对话或下次历史压缩后生效”，并列出变化的项目名。标为高级信息，放在折叠区。
- [x] **MH8 近期产物区块。**（D13）依赖 AR1。默认收起，展开后“全部”“当前会话”两个页签，按时间倒序，滚动加载。每条显示缩略图或类型图标、名字、时间、来源会话。“更多”菜单：下载、收藏、转发（按单个文件构造转发内容，复用 `store.forward` 和 `ForwardDialog`）、定位到消息（仅当前会话且消息已加载或可加载时）。预览复用现有的媒体预览。对象读取走哪条访问路径（msg-center 的 ObjectAccess 需要消息上下文，“全部”页签里别的会话的产物没有）在实施第一步确认并写回 §8。

### 4.9 文档与手册（DX）

- [x] **DX1 协议文档。** `doc/opendan/protocol/Session Control Protocol.md`（控制合同主入口：stop 的作用域、Goal 休眠的生效点及归档的关系、撤回输入、异常中断与恢复。目标已经写在它的 §9，实施时回写 §3–§8 并把 §9 的现状栏改掉）、`Session Input Protocol.md`（所有 Session 都有队列与 `input_policy` 的用法，引用控制合同）、`Session Directory Protocol.md`（输入通道与控制的持久表示）、`Agent State Protocol.md`（交付索引、Session 删除）、`src/frame/opendan/README.md`（新方法、`/stop` 的新含义、`/clean` / `/new` / `/cancel`、产物索引）。runtime 的移除与退役、`runtimes.settings*` / `remove` / `impact` 已由 runtime TODO 写进 Agent State Protocol 和 README，本项不再包含。
- [x] **DX2 界面文档。** `src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md`（新面板、前台消息状态、子会话树与产物的数据通路；顺带更正 §4.4 关于运行状态键由 OpenDAN 写入的说法，后端没有写入方）、`product/message_hub/MessageHub_Web_UI_PRD.md` §12、`product/message_hub/MessageHub_Current_UI_Model_Data.md`。
- [x] **DX3 cmd_skills 联动。** 按 `AGENTS.md` 的表逐项核对并在提交说明里写结论：`ui_core`（`/stop` 语义、`/clean` / `/new` / `/cancel` / `/resume`、撤回的效果与边界、后台工作的暂停与重启后的中断）、`behaviors/chat_route.toml`（暂停中的 plan 照常转发，改动在 SC1）、`opendan_agent_dev`（Session 行为、输入队列与 `input_policy`、新增的服务方法；它的模板示例里有 `input = "queue"`）。本单不改 skill 的安装 kRPC 和 AgentRoot 布局。
- [x] **DX4 回写关联 TODO。** 这一项只改文档。runtime TODO 一侧已做，skills TODO 一侧没做。
  - skills TODO：原计划是把界面要求写进去让那边照着做，但那边已经实施（它的文档里没有提到本单）。按 D19，页面形态的差距另立了 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)；这里只剩在 skills TODO 的 §0 加一行指过去。
  - runtime TODO：在 R-E6 / E6 注明 Agent 首页的形态按规划改为“瘦列表（三类）+ 详情页 + 历史”，由本单 RT1–RT4 实施；在 E5 注明 Agent 首页“添加 Runtime”会深链到目录页；在 R-D1 / R-D3 注明 Agent 视角的“删除”是 `runtimes.remove`。（2026-11-09 已回写：那份文档的 §9.2，以及 R-E6、R-D1、R-D3、E5、E6 各行的状态列。它的裁决 24、26、27 和 §7.2 的答复已写回本单的 RT1、RT2。）

## 5. 顺序与并行

裁决已定（§2，含 11-10 的 D17–D29）。§2.1 的第 8–11 条（D29）和第 12–14 条（D28）已确认。还在等确认的：第 13 条的两处修正，影响 SC11 的判定范围；第 15 条，影响 SC2 里 Goal 派生 Work 的那一条；第 16 条，影响 SC9 的 `/resume` 和 SC11 的提示。动到它们之前先看有没有新的批注。

1. **后端先行，三条线互不相干，可并行：** Memory（ME1 → ME2）；会话控制（SC0 → SC1 → SC2 → SC3，SC4、SC5 可同时做，SC6、SC7、SC8 在其后，SC9 在 SC2、SC6 之后，其中的 `/resume` 在 SC11 之后；SC10 在 SC2 之后，SC11 在 SC1 之后）；AR1。RT1 已做。SC0 改的是 Session 的基础假设，单独一批做完并跑全量回归，再叠后面的。
2. **首页：** HM1 最先；ME3、ME4、RT2、WS1、WS2 在各自依赖完成后并行；RT3 随时可做，RT2、RT4 的后端都已经有了，只差界面。它们都改 `Home.tsx`、`i18n.ts`、`model/*`，分给多个 Agent 时要约定合并顺序，或由一个 Agent 串行做。
3. **MessageHub：** MH1 最先；MH2、MH3、MH4 并行；MH5、MH6、MH7、MH8 跟在各自依赖后。它们都改 `MessageHubView.tsx`、`messagehub.css`、`i18n/messagehub.ts`，同样注意合并。
4. **原来要等另外两份 TODO 的部分已经不用等：** 设备目录与分配（runtime TODO E1、E2、E5、E7）和 Skills 的分组、禁用、安装（skills TODO）都已提交，RT4 的界面入口可以直接做，对应的两个规划验收场景可以走全流程（§6.4）。
5. **同一批上线：** SC2（`/stop` 只停当前 Turn）和 SC9（`/clean`）。只上前者，Telegram 的用户就没有办法清空上下文了。SC11 和“恢复”的入口也要同一批：至少 HM3 的那一条，有 tunnel 用户的话还要 `/resume`（SC9）。只上 SC11，重启后会话停住，却没有地方恢复。

前端可以先按 §3 的形状写 mock 开工，不必等后端合入。

## 6. 验证

### 6.1 命令

Rust（在 `src/` 下）：

```bash
cargo test -p libopendan -- --test-threads=1          # 全量；并行跑时 plan_simple_async、runtimes 会因负载超时
cargo test -p libopendan --test memory_component -- --test-threads=1
cargo test -p libopendan --test runtimes -- --test-threads=1
cargo test -p libopendan --test xagent_cli skills_are -- --test-threads=1
cargo test -p agent_tool agent_memory -- --test-threads=1
cargo test -p opendan -- --test-threads=1
cargo test -p buckyos-api -- --test-threads=1
```

改了 control_panel、scheduler、msg_center 的，跑对应 package（名称以各自 `Cargo.toml` 为准）。

OpenDAN WebUI（在 `src/frame/opendan/web/`）：`pnpm check`、`pnpm lint`、`pnpm test:e2e`。

Desktop（在 `src/frame/desktop/`）：`pnpm check`、`pnpm lint`、`pnpm test:e2e`，以及数据模型测试（`package.json` 里没有对应脚本，直接跑）：

```bash
deno test --no-check --config tests/datamodel/messagehub.deno.json --allow-all tests/datamodel/messagehub-*.test.ts
```

### 6.2 基线就失败的用例

以下是 11-06 至 11-10 的记录，动手前先在未改动的工作树上复核一遍，不要把它们算成回归。11-10 只重跑了下面标明的三条 Rust 用例，其余都是各单当时的记录，这次没有重跑：

- Desktop e2e：`messagehub-agent-task.spec.ts:575`、`messagehub-group.spec.ts:309`、`messagehub-media.spec.ts:173`、`messagehub.spec.ts` 的 “attachments over the sendable size”；并行时 `messagehub-actions.spec.ts:87`、`my-network.spec.ts:350` 偶发失败，串行通过。
- Desktop deno：`messagehub-guest.test.ts` 有 7 条失败。Desktop `pnpm lint`：11-09 的记录是 4 个 error，在没有动过的两个文件里（更早的记录是 5 个，在 ai-center、homestation、settings）。
- libopendan：`tests/xagent_cli.rs` 的 `a_sub_session_is_created_and_followed_through_the_shell_tool`、`xllm_takes_over_the_live_run_and_xagent_commits_its_end`，报 “Session tool directory …/src/tools/bin is unavailable”，是开发树里缺这个目录。skills TODO 那次全量还记了 `tests/fixtures.rs` 的 f04、f06、f07、f12、f15；runtime TODO 当晚的全量记录里没有它们，以复核为准。
- agent_tool：3 个 `grep_tool` 用例（本机没有 `rg`）。
- OpenDAN WebUI e2e：`home.spec.ts` 的 “relocation keeps identity…”；`goals.spec.ts` 在本地时间接近午夜时失败（“下一次机会”跨天后显示成日期）。
- 不再失败的：`a_session_token_reaches_only_its_own_session_methods`（libopendan `tests/session_runtime.rs`）和 `a_session_process_elsewhere_reaches_its_session_through_the_service`（opendan `tests/loader.rs`）起草时记为基线失败，11-10 在 `27aea9efb` 上单独跑，通过。`inbox_messages_are_answered_along_the_same_conversation` 同一次也通过；它本来就是偶发超时，一次通过不说明 SC6 要修的竞态没有了。
- `cargo test` 遇到失败就停，全量回归加 `--no-fail-fast`；libopendan 加 opendan 全量一次约 25 分钟。

### 6.3 真实环境

- OpenDAN 在 paios 容器里，后端和 WebUI 的改动要 `./build_aios --local-test` 再 `uv run src/start.py --skip-update` 才生效；也可以用 `./debug_jarvis.sh devtest --agent <agent_id>` 在宿主机上托管单个 Agent。
- Desktop 对真实 Zone 的调试：`VITE_CP_USE_MOCK=false VITE_ZONE_PROXY=https://test.buckyos.io VITE_ZONE_PROXY_IP=127.0.0.1 npx vite --host 127.0.0.1 --port 4175 --strictPort`，再 `MESSAGEHUB_REAL_E2E=1 npx playwright test --config=playwright.real.config.ts`。`playwright.real.config.ts` 注释里的环境变量已过期。真实用例会给 Agent 发消息，消耗 token。
- 本机开发 Zone 上 11-09 的 DV 用过的做法：owner 的 token 用 `control-panel` 的 `auth.login` 取，Agent 的 kRPC 走 `https://<zone 域名>/kapi/<agent_id>`；只换容器里的 opendan 时，`./build_aios --local-test` 之后删掉该 Agent 的容器（`docker rm -f buckyos-app-<agent 名>`），node_daemon 会用新镜像重建。环境与结果见 runtime TODO §12。
- 未在真实 Zone 上验过的项目要在 §8 如实写明，不能用 mock 通过代替。

### 6.4 规划验收场景对照

| 规划 §三 的场景 | 任务 | 验证方式 |
| --- | --- | --- |
| Owner 没有 Default Runtime | RT1、RT2 | WebUI e2e（mock）；真实 Zone 上新建未选 host 的 Agent 查看首页 |
| 新设备未分配，随后分配给 Agent | runtime TODO E1、E2、E5（已做）；本单 RT1（已做）、RT2、RT4 | 后端和目录页一侧 11-09 晚已在本机 Zone 验过（runtime TODO §12.1：签发 → 目录里未分配 → 分配 → 投影 → 启停 → revoke 后转 `retired`）。首页一侧等 RT2、RT4 的界面做完再走：分配后首页出现 → 停用 → 重新启用 → 删除进历史 |
| 安装了 Skill，Provider Runtime 缺少依赖命令 | skills TODO 的 Skills 页（已做）；详情页在 Skills 页的独立 TODO，不在本单 | 现有页面已经能验：安装 `test/skills_pikg_sample/` 改出的、`commands` 里写了一条不存在命令的包，列表里该行标出缺的是哪条。mock e2e（`skills.spec.ts`）有覆盖，真实 Zone 上没验过 |
| 手工添加工作区 | WS2 | WebUI e2e；真实 Zone 上对私有 runtime 的一个已有目录走一遍 |
| 一个聊天会话创建多个后台工作 | SC0、SC5、MH3、MH5 | Desktop e2e（mock 会话 `session-coder-work` 扩充）；真实 Zone 发一个会建 plan 的请求，再发一个直接建 simple Work 的请求，在树上分别暂停、恢复、停止 |
| 用户暂停正在分析的消息（按 D23 改成停止，没有暂停） | SC2、SC3、SC10、MH4 | Desktop e2e 与 deno 状态函数测试；真实 Zone 走停止、停止并撤回两条路径 |
| 规划里没有、来自 §2.5：暂停后台 plan，发消息纠正，再恢复 | SC0、SC1、MH5 | opendan 集成测试；真实 Zone 上暂停一个 plan，在对话里说一句修改，确认消息留在队列，恢复后被处理 |
| 规划里没有、来自 §2.5：停止一条已经发起了后台 Work 的消息 | SC2、MH4 | Desktop e2e：停止后占位下列出这个 Work，点停止后它结束；真实 Zone 上发一个会建 Work 的请求，建好后停止这条消息，确认 Work 被列出并能停 |
| 规划里没有、来自 §2.5：Goal 进行中让它休眠 | SC2 | libopendan 测试（`tests/goal_session.rs`）：休眠后没有新的模型调用，Activation 以 stopped 结束；有派生 Work 在跑时休眠，Work 暂停，唤醒后继续（推定 15） |
| 规划里没有、来自 §2.5：断电或重启后手工恢复 | SC11、SC9、HM3、MH4、MH5 | opendan 集成测试（重启 Supervisor）；真实 Zone 上让一个 plan 跑着，`start.py --skip-update` 重启，等过 plan 的一个 tick 周期（10 分钟）确认没有自动推理，再从首页和侧面板各恢复一次 |
| 内嵌 Chatbox 会话完成一个产物 | AR1、MH8 | 只能验“全部”与“当前会话”的归属；隐藏会话不存在，这一条标为未覆盖 |
| Clean History 后回查旧 Memory | SC6、SC9、MH6 | opendan 集成测试（`ui.rotate` 和 `/clean` 两个入口）；真实 Zone 归档后用 `agent-session recall --read` 读旧引用 |
| Agent root fs 的系统提示词更新 | SC7、MH7 | libopendan 测试；真实 Zone 改 `role.md` 后看标签，触发一次压缩再看 |

## 7. 不做与风险

**不做：**

- 目标管理界面（规划 §5，等 Goal 模块重构）。
- Memory 的手工添加和编辑（规划首版范围外）。
- Skills 的分组、停用、卸载、安装器与 LLM 扫描，以及 Plan Skills 的格式、选择闭环、self-improve：全部在 skills TODO（D5–D7），已交付。
- Skills 页的界面形态（手机样式、详情页与正文、四组计数、“已停用”小节、安装入口合并）：另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)（D19）。
- 设备的添加与分配：用户级 runtime 目录、daemon 设备两步化、Desktop 目录页、CLI，全部在 runtime TODO（D4），已交付；ssh 登记上提那边裁决不做。
- 群和联系人会话的侧栏改版（D16，owner 已说明需要优化，另行安排）。
- 隐藏会话与内嵌 Chatbox。
- TaskMgr 一侧发起的暂停和恢复。
- Telegram 菜单里的 `info`、`list`、`switch`、`help`，以及 tunnel 的多 session 能力（D18 只定了有这种能力时 new 与 clean 不同）。
- 前台消息的暂停（D23）：规划 §9.1 的“暂停当前消息”不做，连同不级联的 pause。
- 重启后的自动恢复及其配置（D27，下一期）。
- 用消息纠正纯 Work（D24）；子会话树上直接给 plan 发消息的入口；tunnel 渠道的撤回入口。

**风险：**

- **SC0 改的是 Session 的基础假设（D20）。** “没有队列”今天在好几处被当作判据（子会话怎么停、等输入的 Session 何时再看、验收怎么落、首批材料走哪条路），模板键 `input` 的含义也要变。它影响所有建 Session 的路径和大量现有用例，要单独一批做并跑全量回归；没有队列客户端的开发宿主怎么办还没定。
- **停止语义变更影响面大（D9、D17、D21）。** 两种 stop 分开之后，结束一个 UI Session 只剩归档这一条路，Goal Session 的 Session 级 stop 从“只停当前 Activation”变成休眠。所有发 stop 的地方都要对一遍它发的是哪一种：`/stop`、TaskMgr 取消、回答态按钮、`xagent ctl stop`、WebUI 的 Stop 按钮、父会话结束时对子会话的 stop。SC2 和 SC9 要同一批上线，否则 Telegram 的用户没有办法清空上下文。
- **“暂停”和“休眠”容易混。** 会话的暂停（SC1）和 Goal 的休眠是两件事，但 Goal 的状态值、CLI 动词今天都叫 pause。已定只改界面用词（D29），所以文档和手册里要写清楚对应关系。D27 之后暂停状态又多了一个来源（异常中断），界面上三样东西要各有各的说法：用户暂停、异常中断、Goal 休眠。推定 15 成立的话，Goal 休眠还会让它派生的 Work 进入会话暂停，这是第四种来源，要写明是随 Goal 休眠停的。
- **重启后不再自动接着跑（D27，SC11），影响面不小。** 每次重启宿主，包括升级、重建容器、开发时的 `start.py`，进行中的会话都会停住，等人恢复。没有人看着的 Agent（Goal 驱动的长期工作、内置的记忆整理）会一直停到 owner 来点；自动恢复要到下一期。现有测试和 DV 流程里凡是默认“重启后自动继续”的都要改。恢复入口必须和 SC11 同一批上线（§5）。
- **撤回容易被理解成撤销（D25）。** 能撤回的只是输入和它在对话里的痕迹，已经执行的工具、已经写下的文件都留着（§2.5 的推论）。确认文案和 `ui_core` 手册都要写明；这条输入如果已经被记进感知，还要另外处置（SC10）。
- **中断的判定比“有打开的 Turn”宽（推定 13 的修正）。** 等子任务的 Work 也算，所以一次重启会把所有进行中的后台任务都停住，不只是正好在推理的那几个。这是有意的：不这样的话它们会被自己的 tick 重新带起来。在等用户回答的不算，实施时要先确认这种状态在 state 里怎么认，认错了会把提了问题的会话也停住。
- **停止一条消息不停它发起的 Work（D8、D9），这一点用户未必想得到。** 本单靠界面补：停止后列出来、当场能停（MH4）。tunnel 上没有这块界面，用户只能在对话里让 Agent 去停。
- **Goal 休眠带停派生的 Work 还没有确认（推定 15）。** 不做的话，休眠之后已经派出的 Work 会继续推理到结束，目标页上要写明。
- **“开始新对话”被拒绝的情形可能很常见（D11）。** 长期跑着后台 Work 的 Agent 会一直点不了。tunnel 上更明显：那里没有子会话树，用户只能在对话里让 Agent 停掉这些 Work（§2.1 推定 5）。实际使用中如果是这样，要回头考虑把未结束的 Work 改挂到新会话。
- **压缩时刷新提示词（SC7 第二步）没有核对过可行性。** 压缩发生在 run 内，系统前缀的重建方式本单没有读透；如果代价过大，标签仍可先上线，刷新另起一单。
- **Memory 列表的开销。** 感知按会话分文件，全量扫描随会话数增长；ME1 的缓存和分页是必须的，不是优化。
- **关联实现的真实环境验证不全。** skills TODO 的实现没有在真实 Zone 上验过任何一项。runtime TODO 一侧，opendan WebUI 的 runtime 卡片与 Session 徽标、Desktop 的签发和分配对话框、设备独占与换人都没验（它的 §12.4）。本单的界面建在它们上面，遇到问题先分清是哪一层的。
- **首页与 MessageHub 两处的任务共用文件多**（`Home.tsx`、`i18n.ts`、`model/*`；`MessageHubView.tsx`、`messagehub.css`）。并行实施时冲突概率高，见 §5。
- **顺带发现、不在范围内：** `web/tests/real/smoke.spec.ts` 仍期待已不存在的 `perception.cursor`、`perception.backlog`；`service.rs` 的 `is_read` 里也还列着它们。Telegram tunnel 菜单里的 `info`、`list`、`switch`、`help` 没有处理（`new`、`clean`、`cancel` 已列入 SC9）。MessageHub 头部和会话列表的运行状态行读的 `typing` / `active` / `status_line` 没有后端写入方，真实环境里恒为空；本单的面板不依赖它，是否让 OpenDAN 写这些键或撤掉这一行另行决定。

## 8. 实施记录

（实施后填写：落点、与本单不同的决定、测试结果、未验证项。）

- **2026-11-09，RT1 的后端已随 runtime TODO 实施（已提交 `35e72ce06`）**，因为它是那边 R19 的前置：`configured_default_runtime_id`、停用不清默认、拒绝停用 / 移除私有及其别名、`runtimes.remove`、settings 里移除的条目转 `retired`。`runtimes.settings_set` 的参数当时是 `{runtime_id, enabled?, default?, confirm?}`（`confirm` 当晚按裁决去掉，见下一条），`runtimes.impact` 也已有。用例在 `opendan/tests/loader.rs` 与 `lib_opendan/tests/runtimes.rs`。RT1 里“量一下 opendan 路径改完 settings 到节点收回权限的延迟”没有做。详见 [runtime TODO](opendan-agent-runtime-lifecycle-todo.md) §11。RT2 的一部分也已在 opendan WebUI 上：ComputerUse 与基线只读、停用前取影响清单；瘦列表、详情页、RT3、RT4 的界面仍未做。Desktop 目录页的页面 id 是 `agent-runtimes`，可用 `openSettings(page)` 打开，`?settings=<page>` 的 URL 深链（RT3）没做。
- **2026-11-09 晚，runtime TODO 的本机 Zone DV 和 owner 裁决改了 RT1 的行为（`b6c57704a`、`27aea9efb`）**：去掉 `host` 占位；停用和移除遇到正在运行的 Session 直接拒绝，`confirm` 取消；原生运行只在 Lite；`session.stop` 能停没有输入队列的 Work。裁决整理在本单 §2.2，RT1 的 DV 结果在 runtime TODO §12.1。
- **2026-11-09，skills TODO 实施并提交（`174dc06f7`）**，交付了 opendan WebUI 的 Skills 页和首页的 Skills 卡片。本单原来的 SK1（只读占位页）因此不做，SK1 改写为补齐规划的形态。
- **2026-11-10，按 `27aea9efb` 复核全文，只改了本文档，没有改代码。** §1 按现在的代码更新；新增 §2.2、§4.0；§2.1 增加第 5、6 条；§3 加了状态列；RT1–RT4、SK1、SC1、SC7、SC8、MH5、DX1、DX3、DX4 的文字按现状改过。核对源码后确认其余各项都没有实施：opendan 服务里没有 `home.summary`、`memory.*`、`session.pause` / `resume`、`ui.conversation`、`session.work_tree`、`ui.rotate`、`sessions.delete`、`deliveries.recent`；WebUI 没有首页样式的详情路由和 Memory 区块，工作区区块没有改；MessageHub 的详情栏仍是 `entity` / `session` / `message` / `profile` 四种，没有 Agent 面板、Info 按钮和占位消息上的控制按钮。§6.2 里重跑了三条 Rust 用例，都通过；其余基线记录、WebUI 与 Desktop 的 e2e 这次没有跑。
- **2026-11-10，owner 批注了 §2.1，整理为 D17–D20 并回写，只改文档。** D17：stop 对 UI Session 只停当前 Turn（SC2、MH4 去掉“待确认”，改为在 `apply_control` 一处判断）。D18：clean 的语义与 tunnel 上 new / clean 的关系（SC6 改写，归档不再走 stop；新增 SC9）。D19：Skills 页的形态另立 [独立 TODO](opendan-skills-page-ui-todo.md)，SK1、对照表和读手册正文的接口移过去，HM1 去掉两条 Skills 路由。D20：所有 Session 都带输入队列（新增 SC0；SC1、MH5 去掉对无队列 Work 的特殊处理）。整理时的七条推定在 §2.1，等确认。为写 SC0、SC9 核对过的代码事实：`session.input_policy` 已存在但没有模板使用；桥已有 `SlashCommand::App` / `AppCommand`，`drain` 不处理；Telegram 每个聊天固定一个会话 id，菜单命令除 `stop` 外 tunnel 和 Agent 都不处理。
- **2026-11-10，owner 答复了上一条里的七条推定，只改文档。** 第 1 条不照推定：要区分针对消息的 stop 和针对 Session 的 stop，各类会话的 Session 级 stop 含义不同，Goal Session 的手工 disable 叫休眠（D21）；其余六条按建议（D22）。据此新增 §2.3 的语义表，SC2 改写成“区分两种 stop”（原来是让 UI Session 把所有 stop 都降成 turn），HM3 加了 Session 详情页 Stop 按钮分类型的一条，§0、§1.7、§3、§7 相应更新。展开 D21 时的四条推定在 §2.1 第 8–11 条，等确认。核对过的代码事实：`apply_control` 今天只有“Goal Session 的 stop 一律降成 turn”这一条特例；Goal 的手工 disable 已有，是状态 `paused`（`goals.set_status`，目标详情页的“暂停”按钮）。
- **2026-11-10，owner 从用户角度给了一份有效语义，并答复了由它引出的五个问题，只改文档。** 语义记在 §2.5，逐项对到控制和现状的表在 `Session Control Protocol.md` §9（协议文档的这部分同日已改，§9.2 记了五条裁决）。五条答复整理为 D23–D27：前台消息不要暂停；后台 Work 里只有 plan 收消息，是刻意的区分；撤回输入按建议；Goal 休眠后不发起下一次推理；重启后一律手工恢复，自动恢复下一期。据此：SC1 去掉不级联选项，只剩接口；SC2 保持直接打断（§2.4 原来写的“等在飞调用返回”与用户语义 1a 相反，已改），并加上 Goal 休眠的生效点；新增 SC10（撤回输入）、SC11（异常中断与手工恢复）；MH4 去掉暂停按钮，加上撤回和中断后的恢复；MH5、HM3、SC3 加上异常中断；§0、§1.7、§2.3、§2.4、§3、§5、§6.4、§7 相应更新。展开时的三条推定在 §2.1 第 12–14 条，等确认。为此核对过的代码事实都补在 §1.7 末尾：stop 对两种范围都是直接打断；推理边界不看 Goal 状态；宿主启动后全部自动恢复，扫描没有“第一次”的标记；登记表的暂停只有布尔值；用户的话只经 UI Session 的 Agent 转给 plan；MessageHub 的 redact 到了桥被当作噪声。最后一条和“恢复后先跑完被打断的那一批再接收消息”（引自 Session Input Protocol §3）没有跑用例确认。
- **2026-11-10，owner 确认了上一条里的三条推定，并定了范围：自动恢复下一期，这一期把手动控制的语义做对（D28）。只改文档。** 按这句话把 §2.5 的每一项对着代码问了一遍“操作之后还有没有哪里在接着推理”，结果记在 §2.5 末尾的表里。改动：
  - 推定 12、14 记为已确认。推定 13 确认之后发现两处漏洞并修正，等过目：等自己发起的工作的 Work 即使没有打开的 Turn 也算中断，否则它会被自己的 tick 重新带起推理（`work` 类 30 秒、plan 600 秒）；在等用户回答的不算，否则用户的回答没有反应。判定改为按 `state.json` 做，登记表只用来枚举。
  - 新增推定 15（Goal 休眠时一并暂停它派生的 Work，唤醒时恢复）和推定 16（tunnel 上的 `/resume` 和一句提示），等确认。
  - SC1 加上提示词的改动（暂停中的 plan 照常转发）；SC2 加上派生 Work 的暂停；SC9 加 `/resume`；SC11 重写判定，写明恢复的级联只从暂停着的节点开始、“全部恢复”由调用方逐个发，中断的 Goal Session 被置休眠时清掉中断标记，tunnel 上回提示；MH4 在停止后列出这条消息发起的 Work；MH5、HM3 加“全部恢复”。§1.7、§2.3、§2.4、§3、§5、§6.4、§7 相应更新。
  - 为此核对过的代码事实补在 §1.7 末尾五行：tick 的规则与间隔、`needs_input` 的定义、resume 级联的条件、Goal `paused` 不影响派生 Work、首页“进行中”的判定。都是读代码得出的，没有跑用例。
  - 没有核对清楚的：提了问题在等用户回答的会话在 state 里是什么形状（是 `needs_input`，还是把问题当成跟踪的 task），SC11 的判定要先确认这一点；对话里那行工作状态对暂停中的 plan 显示什么（SC1）。
- **2026-11-10，owner 在 §2.1 第 8–11 条下面逐条批注“好，按你的建议”，整理为 D29，只改文档。** 四条的内容已经写在 SC2 和 HM3 里，这次没有改任务的做法，只把各处的“推定 8–11、待确认”改成已确认：§2 的裁决表、§2.1、§2.3、§2.4、§4.6、SC2、§5、§7。批注原话并进了 D29 一行。还在等确认的只剩第 13 条的两处修正和第 15、16 条。
- **2026-11-10 晚，按本单实施（基线 `7c91504b6`，未提交，未在真实 Zone 上验）。** 后端的会话控制（SC0–SC11）在主工作树做；Memory / 产物索引 / `home.summary`、opendan WebUI、Desktop MessageHub 三条线并行做完后合入。下面按“落点 / 与本单不同的决定 / 没做与没验”记，测试结果在本条末尾。
  - **按建议实施的三处，owner 当晚已确认都按建议（D30）**；落点留作记录：
    - 推定 13 的两处修正（判定）：`runner/drive.rs` 的 `mark_interrupted`。要改回“只看打开的 Turn”，改那里的 `working` 一个表达式。
    - 推定 15（Goal 休眠带停派生 Work）：`host/goals.rs` 的 `sync_dormant_work`（`reconcile` 里一处调用）和 `runner/inputs.rs` 的 `pause_source` 里 `goal` 一支。去掉调用即回到“派出的 Work 跑完”；`tests/goal_session.rs` 的 `a_paused_goal_holds_its_inputs_until_resumed` 要跟着改回去，目标页要写明。
    - 推定 16（`/resume` 与提示）：`opendan/src/ui.rs` 的 `command` 里 `resume` 一支、`tell_interrupted`、`bridge_ctx` 的注册；Telegram 菜单多的一条在 `msg_center/src/tg_tunnel.rs`。
  - **SC0。** 宿主有队列客户端时每个 Session 都有队列。模板键 `input` 保留键名换含义：`queue` = 收 msg / event，`none` = 只收控制命令（建会话时落成 `session.input_policy = none`），没有改成直接写 `input_policy`。只收控制的 Session 首批材料仍走 `prompt.initial_inputs`（`create_session` 的检查放宽成“队列收消息时才不许带”；`_bootstrap` 源不受 `input_policy` 限制）。投递方的拒绝放在登记表的 `post_input`（所有生产者都经过它），`session.post`、`agent-session post`、`xagent run --msg` 都在投递时得到 “takes no messages”。
    - 与本单不同：**没有队列客户端的宿主保留无队列**（不在 Zone 里、也没有 `--queue-dir` 的 `xagent`），没有给它缺省的本地文件队列。所以 `cascade_pause`、`stop_children`、`host/mod.rs` 两处、`decide_unqueued` 里的“无队列”判据保留，只对这种宿主生效；`session.stop` 的 Supervisor 旁路删了。`SubSessionSpec.queued` 没有去掉：它现在的含义是“这个子会话收 msg / event”（Todo 派发用），不再是“有没有队列”。
    - 没做：Todo 会话很多时 kmsg 上每个 Session 多一个队列和订阅的开销没有量。
  - **SC1。** `session.pause` / `session.resume`。对话里那行工作状态原来只显示 `run_state`，暂停中的 plan 显示成 `waiting`，Agent 看不出来；现在显示 `paused` 或 `interrupted by a restart, waits to be resumed`（`ActiveSession.paused`、`runner/background.rs`）。`chat_route.toml` 和 `ui_core` 写明没有结束的 plan（含暂停中的）照样转发。“plan 暂停期间在对话里说一句修改，Agent 转给这个 plan”只改了提示词，没有用真实模型验。
  - **SC2。** `StopScope` 多一个 `archive`（见 SC6）。`apply_control` 按会话类型分派；`session.stop` 对 UI 会话的 session 范围、对没有打开 Turn 的 turn 范围在服务端直接报错。Goal Session 的 session 范围 stop 由驱动者以投递者的身份调 `goals.set_status(paused)`。推理边界和驱动循环都看 Goal 状态（`runner/goal.rs` 的 `stop_if_dormant`）。`xagent ctl stop` 加了 `--scope session|turn|archive`。UI Turn 停在工具调用中途时的 turn 范围 stop 有用例；`analyze_intent` 子 context 里的没有单独写用例（`turn_stop && !site.child` 那一支没有动）。
  - **SC3。** TaskMgr 的 `Paused` 只能经“请求控制 + 确认”进入，而且要 task 的 control profile 允许。做法：会话暂停时把 profile 的 pause / resume 置为可用，再自己请求并确认 Pause；恢复时请求并确认 Resume。副作用是 Task Center 上这个 task 会出现暂停 / 恢复按钮：TaskMgr 一侧发起的请求不转成会话控制，只会在会话自己暂停或恢复时被一并确认。真实 TaskMgr 上没有验（测试里的假 TaskService 用的是缺省空实现）。
  - **SC4、SC5。** `ui.conversation`（`current` 另带 `route_key`）、`session.work_tree`。树里 Todo 的旧 attempt 不作为 plan 的子节点列出。
  - **SC6。** 归档选了“stop 的第三种作用域”这条路（`stop {scope: archive}`，只对 UI Session 生效），没有用 Supervisor 的 `StopSignal`：这样旧会话的 worklog 里有谁、为什么归档，在飞的调用也照 stop 的规矩被打断。`ui.rotate` 等旧会话结束最多 20 秒，返回 `{archived, finished}`。竞态的修法照本单：`session_for` 对未结束的候选再读 `state.json`，已请求停止或已结束的不投递，记录留到下一轮扫描。
  - **SC7。** 比较的做法是把“现在新建一个同类会话会冻结到什么”整个重算一遍（`state/behaviors.rs` 的 `refrozen`，UI 会话的手册取模板当前的缺省，也就是 owner 的 UI 集合，禁用的自然不在里面），再逐项比（`frozen_changes`）。第二步核对的结论：系统前缀在每个新 run 建上下文时由 `prompt.frozen` 装配，压缩不改它；所以刷新做在“新建 run 之前”（`runner/drive.rs` 的 `refresh_frozen`）：UI 会话的摘要比冻结时间新，就重新冻结并记 `control_applied{prompt_refrozen}`。结果是压缩之后的**下一个 run** 才用上新提示词（压缩发生在建上下文的过程中，那个 run 的前缀已经定了）。run 内的就地压缩不换前缀。
  - **SC8。** `sessions.delete` 归入门面的 `is_write`。已结束但还能验收的 Work 也能删，它的队列没有主动回收。
  - **SC9。** 命令的回复不经过 Session，由宿主直接发（`ui.rs` 的 `say`），文案在 Agent 包 `i18n/<lang>.toml` 的 `[commands]`，包里没有时用内置英文。控制和撤回只投给对话当前的 UI Session，不会为此新建一代。
  - **SC10。** `control(retract {key})`。**没有加 `session.retract` kRPC**：入口是 owner 的 redact，界面看 Turn task 的状态就够。
    - 实现上的两处与设想不同：正在处理的输入按打开的 Turn 上新记的 `open_turn.msg_keys` 认（run 进行中，Turn 的输入记录还没刷进会话 worklog，按 worklog 查不到）；“下一个 Turn 知道这条被撤回了”靠一条待注入的观察事件放进下一批输入的背景（对话沿用同一个上下文时不重建历史，历史里的注记到不了模型），重建历史时另有一行英文注记。这句话是给模型看的，没有进 i18n。Turn 合并了多条消息时不单独标出是哪一条。
    - 占位消息：没有做成“一问一答都不显示”，也没有单独的“已撤回”文案，沿用“已停止”（`[outbound] stopped`）；用户那条消息按 MessageHub 现有的 redact 显示。
    - 感知：用户的输入不会自动进感知流，感知只在模型自己 `perceive` 时写。被撤回的 Turn 里模型已经记下的不自动处置，owner 可以在 Memory 页删（未整理的感知可删）。
    - `fixtures/14_input_bus` 没有动：`retracted` 是运行时由 control 产生的拒绝，不是记录解析阶段的。
  - **SC11。** 标记由驱动循环在拿到 lease 后写（`mark_interrupted`），Supervisor 只负责告诉“这是重启后的恢复期”（第一次登记表扫描结束之前起的驱动循环都带这个标志）。补了本单没写的两点：只对宿主这次启动之前就存在的 Session 判定（恢复期内新建的不算）；退避中的 run（可重试错误之后）算正在工作，它到点会自己推理。“在等用户回答”在 state 里的形状核对过：就是 `needs_input` 那个式子（`waiting` 且不在等自己发起的工作），提问经通知发出的 Work 也是它。标记之后的恢复步骤（对齐快照、未知结果的物化）在暂停期间的驱动里照常做，不等 resume；推理要等 resume。owner 的通知在第一次扫描结束 5 秒后发一次。
    - 顺带修了一处既有缺口：驱动被中止（宿主 `shutdown`）时，监视 stop 的后台任务没有随之结束，lease 一直被它拿着（`StopMonitor` 加了 `Drop`）。真实重启不受影响，同进程内重启宿主的测试受影响。
  - **Memory（ME1、ME2）。** 落点 `memory/owner.rs`、`Consolidator::forget`、`opendan/src/overview.rs`。与 §3 不同：`MemoryEntry` 多 `revision?`（认知的当前修订，删除时当 `expected_revision`）和 `cleared?`，`absorbed_into` 是数组；感知的 `state` 多一种 `duplicate`；`perceptions.pending` 含暂缓的；冲突没有专门的错误 kind（`kind: other`，message 以 `memory_conflict:` 开头），租约被占是标准的 `busy`；`memory.forget` 只有 owner 能调（进了 `is_owner_only`）。`memory.read` 返回 Memory 的读视图原样，WebUI 按 JSON 显示。
  - **AR1。** `state/deliveries.jsonl`（格式在 Agent State Protocol §6.1）。`before` / `next_before` 是行位置。没进索引的：回复正文里 `<attachment>` 标签带的文件（它在宿主把回复转成消息时才变成对象）；进了索引的回复附件只有结构化回复的 `artifacts`。
  - **opendan WebUI。** 新文件 `web/src/shell.tsx`、`pages/Memory.tsx`、`pages/Runtimes.tsx`、`runtimes.ts`；`Workspaces.tsx`、`Home.tsx`、`SessionDetail.tsx`、`Sessions.tsx`、数据层四个文件改过。首页概况没有接 `home.summary`（Session 树本来就要整份列表），口径一致：暂停着的不算进行中。WebUI 上没有“暂停”入口（暂停在 MessageHub 的子会话树上）。WS2 核对的结论：导入在写身份文件之前失败的什么都不留；登记这一步失败时目录里会留下 `.opendan-workspace.json`，再次导入同一目录能自愈；create 还可能留下新建的空目录。
  - **文档与手册。** `Session Control Protocol.md` §3–§8 重写（§9 留作裁决记录），`Session Input Protocol.md`、`Session Directory Protocol.md`、`Agent State Protocol.md`、`opendan/README.md`、`xAgent.md`、架构设计各补了对应的段落；schema 重新导出（`control_command`、`registry_entry`、`session_config`、`session_input`、`session_state`）。手册：`ui_core`（`/stop` 等命令、撤回、暂停与中断）、`opendan_agent_dev`（`input` 键的新含义、stop 的作用域、重启后的中断）、`behaviors/chat_route.toml`、Jarvis 的 `i18n` 加 `[commands]`；Telegram 菜单改了 `clean` / `new` / `cancel` 的说明并加了 `resume`。本单没有改 skill 的安装 kRPC 和 AgentRoot 布局，其它手册不受影响。
  - **没验的。** 真实 Zone 上全部没验（后端要 `./build_aios --local-test` 重建镜像）；真实 TaskMgr 上的 Paused；Telegram；真实模型下暂停中的 plan 的转发；opendan 这条路径改 settings 后调度器刷新的延迟（RT1 遗留）。升级到这一版后第一次重启，所有进行中的会话都会停在中断，要手工恢复。
  - **测试结果（2026-11-10 晚）。** `cargo test -p libopendan -p opendan --no-fail-fast -- --test-threads=1`：569 通过，2 失败，6 忽略。失败的两条是 §6.2 记的基线失败（`tests/xagent_cli.rs` 的 `a_sub_session_is_created_and_followed_through_the_shell_tool`、`xllm_takes_over_the_live_run_and_xagent_commits_its_end`，缺 `src/tools/bin`）。新增用例：`lib_opendan/tests/session_control.rs`（8 条：只收控制的队列、对话只能归档、未消费输入的撤回、处理中消息的停止与撤回、重启中断与按树恢复、停止中断的消息、删除、压缩后重新冻结）、`tests/work_card.rs` 的 `the_work_below_a_session_is_a_tree`、`tests/goal_session.rs` 的 `a_goal_put_to_sleep_mid_activation_infers_no_more`、`tests/deliveries.rs`、`tests/memory_component.rs` 两条、`opendan/tests/loader.rs` 的 `a_restart_interrupts_the_message_in_progress_until_resume`。按新合同改过的既有用例：依赖“重启后自动继续”的（`goal_session.rs` 两条、`memory_maintenance.rs` 的崩溃恢复、`loader.rs` 的 `registered_sessions_are_hosted_and_recovered`）现在先手工 resume；对 UI 会话用 session 范围 stop 的改成归档或 turn 范围；断言“没有队列”的改成“只收控制”；`a_paused_goal_holds_its_inputs_until_resumed` 改成验证派生 Work 随 Goal 休眠暂停。`cargo check -p msg_center` 通过。opendan WebUI：`pnpm check`、`pnpm lint`、`pnpm build` 通过，`pnpm test:e2e` 49 条全过（mock，Chromium）。
  - **Desktop（MH1–MH8、RT3 的深链、DX2）。** 新目录 `desktop/src/app/messagehub/agentpanel/`（面板、数据源、时间线分隔），`conversation/input/foregroundState.ts`（前台消息状态，纯函数）、`conversation/tasks/TurnControls.tsx`、`conversation/worksessions/WorkControls.tsx`、`swipe.ts`、`mock/agentpanel.ts`；`WorkSessionCards` 改成能递归；`DesktopRoute.tsx` 处理 `?settings=<page>`。
    - 与本单不同：回答态的 Stop 只对自己名下的 Agent 调 `session.stop {scope: turn}`，别人的 Agent 仍发 `/stop` 文本（调不了它的服务）。时间线上“开始新对话”的分隔只来自 `ui.conversation`，面板关着不读，所以要在这个浏览器打开过面板之后才有（历代结束时间记在 localStorage）。点子会话树上的卡片开的是详情栏里的工作日志（新目标 `worklog`），返回回到面板。⋮ 菜单的“编辑信息”仍开原来的会话详情。面板宽 380（其它详情 320）。近期产物只有类型图标，没有缩略图。已停止的节点不在 `unfinished_only` 的树里，树下留一行“已停止”和去 Agent 首页的链接。手势用例放在新 spec，没有进 `messagehub-ux.spec.ts`。
    - “停止并撤回”撤哪条消息：Turn task 的 input 现在多带 `input_keys`（这个 Turn 的消息 ObjId，`runner/turn_task.rs`、`opendan/src/tasks.rs`），前端优先用它；没有时退回“占位之前用户最新的一条”。
    - MH8 的对象读取路径：预览和下载走 Agent `artifacts` 桶的绑定（owner 自己的桶权限），不经 msg-center 的对象路由。“收藏”“定位到消息”只在已加载的时间线里找得到携带该对象的消息时可用。归档后没有当前一代时“当前会话”页签不可用（`route_key` 取自 `ui.conversation` 的 `current`）。
    - 验证：`pnpm check` 通过；动过的文件 lint 0 error（全量仍有基线的 4 个 error，在 ai-center 和 homestation）；deno `messagehub-*.test.ts` 111 通过、7 失败（都在 `messagehub-guest.test.ts`，基线），新增 `messagehub-agent-panel.test.ts` 10 条通过；e2e 新增 `messagehub-agent-panel.spec.ts` 18 条，连同 agent-task 一起 `--workers=2` 复跑 34 通过、3 失败，其中 `messagehub-agent-task.spec.ts:567` 是基线（原 :575），另两条串行重跑通过。全部 `messagehub*` spec 在与后端形状对齐之前整体跑过一次：198 通过、4 失败，都是 §6.2 的基线。全量 `pnpm test:e2e` 只在高负载下跑过一次（524 通过、27 失败）：`ai-center:4`、`ai-center:119`、`desktop:330`、`filebrowser-review:93`、`ndm-upload-endpoint` 全部、`my-network:350` 串行重跑仍失败，这些文件本单没有动，但没有在未改动的工作树上确认它们是不是基线。
    - 没验：真实 Zone；只测了 Chromium，手势是合成的 touch 事件；转发一个只带 `obj_id` 的产物后对方能不能读到；直接暂停 Todo 的会话后 plan 的下一次 tick 会不会把它当异常；改 UI DataModel 属 `harness/trigger_rules.md` §3.4 的高影响改动（读放大的核对：面板关着不读，会话状态和工作树最多 15 秒一次，产物展开才读）。
- **2026-11-10 晚，owner 确认推定 13 的两处修正、推定 15、16 都按建议（D30），只改文档。** 三处实施时就是按建议做的，代码不用动。本单没有待确认的推定了。
