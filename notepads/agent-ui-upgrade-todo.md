# BuckyOS Agent UI 升级 TODO

状态：2026-11-09 起草；同日 owner 在文中批注了 D1–D16，已整理成 §2 并回写任务。**2026-11-10 按提交 `27aea9efb`（工作树干净）复核并更新了本文；同日 owner 批注了 §2.1，整理为 D17–D20 并回写。** 现在是 36 项任务：做完的只有 RT1 的后端（随 runtime TODO 一起做的）；RT2、RT3、RT4、DX4 各做了一部分；SK1 按 D19 移出本单，另立 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)；SC0（所有 Session 带输入队列，D20）和 SC9（聊天命令 `/clean` 等，D18）是新增的；其余都没有实施。逐项状态见 §4.0。整理 D17–D20 时的七条推定，owner 同日答复：stop 要分“针对消息的”和“针对 Session 的”两种，Goal Session 的手工 disable 叫休眠（D21，表在 §2.3），其余按建议（D22）。展开 D21 时又有四条推定等确认，见 §2.1 的第 8–11 条。依据 [BuckyOS Agent UI 升级规划](<BuckyOS Agent UI 升级规划.md>)（下称“规划”，11-09 之后没有改过）逐节对照源码写成。任务后面的（Dn）表示它按哪条裁决做。

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

### 1.8 产物

| 事实 | 位置 |
| --- | --- |
| 报告的 `artifacts`、回复里的附件经 `ObjectStore` 存进 named store，绑进桶 `user/<agent_id>/<owner>/artifacts`，key 是 `<对象 id 前 16 位>/<名字>` | `opendan/src/objects.rs`、`runner/reports.rs` |
| 没有记录“哪个 Session、什么时候交付了哪个文件”的索引，也没有相应接口。桶本身可以 `list_objects`，但条目不带 Session | `buckyos-api/src/bucket_client.rs` |
| 首页工作区卡片里的“最近产物”读的是 `artifacts.list`（`ArtifactHead`），是另一个概念 | `Workspaces.tsx` |

## 2. 裁决记录

2026-11-09 owner 在文中逐条批注，整理如下。“按建议”指采纳起草时的建议，建议内容已写进“裁决”一栏。规划 §四 的待定项已并入。D17–D20 是 2026-11-10 owner 对 §2.1 的批注，“owner：”后面是批注原话，其后是整理者的展开。

| # | 问题 | 裁决 | 落到 |
| --- | --- | --- | --- |
| D1 | Memory “删除”的含义 | 按建议。接受逻辑删除：认知写一个 `deleted` 的新修订，历史保留；界面仍叫“删除”，确认框说明“Agent 不再使用它，历史修订保留”；owner 删除的认知，撤回提示行不再带原陈述。感知只有未整理的能删（处置为 `discarded` 并清正文）；已吸收的标“已整理为认知”，不提供删除，引导去删对应认知。owner 补充：影响大的是认知，用户可以手工删掉一条。所以认知删除是必做项，感知删除走同一个接口顺带提供 | ME2、ME4 |
| D2 | Memory 的可见范围、计数口径与首页预览 | owner：首页按“最近添加”看，最近 5 条认知加 3 条感知就够。认知在前。其余按建议：首页用不受限视图（服务已验证调用者是 owner），每条显示 `scope.subjects`；认知总数 = 状态 `active` 或 `disputed` 且不是 `salience`；感知总数 = 正文未清理的全部，另给“待整理”数 | ME1、ME3 |
| D3 | 停用 Owner 默认 runtime 后是否保留默认设置 | 按建议。保留 `default_runtime_id`，停用期间由 `effective_default_runtime()` 落到私有，重新启用后恢复；放宽 `validate_runtimes`，补调度器测试 | RT1 |
| D4 | 设备在哪里添加和分配，Agent 首页管什么 | 不按建议。owner：之前的规划更完整；Agent 首页主要站在 Agent 的视角讨论 enable / disable / delete。因此设备目录与分配以 [runtime TODO](opendan-agent-runtime-lifecycle-todo.md) 的两步流程为准（§0.6、§4.6，任务 E1–E3、E5、E7），本单不另做“最小子集”；本单只做 Agent 首页一侧：这个 Agent 的 runtime 列表，能启用、停用、删除 | RT1–RT4 |
| D5 | Skills 四组怎么得到 | 不在本单定。owner：Skills 体系升级有专门的 [skills TODO](opendan-skills-system-todo.md)，可能还没完成，界面可以先占位 | §4.4（原 SK1，后被 D19 取代） |
| D6 | 停用项的位置与计数口径（规划待定） | 按建议，作为界面要求移交 skills TODO（停用状态由它实现）：首页计数只算已启用的，停用数另写“· N 已停用”；Skills 页每组末尾有折叠的“已停用”小节，在原处重新启用；任何来源都能停用，只有 `installed` 能卸载；UI 组首版只读 | §4.4 对照表 |
| D7 | 安装器与 LLM 扫描 | 归 skills TODO（owner：这也是那份文档里的内容）。规划对安装器的要求作为界面要求移交 | §4.4 对照表 |
| D8 | 暂停当前消息是否暂停后台 Work | 不级联。owner：暂停当前消息，暂停的是正在处理这条输入的那个 Turn；它创建的 Work Session 都在后台运行，要暂停它们走暂停 Session 的流程 | SC1、MH4、MH5 |
| D9 | “停止当前消息”的含义 | 与 D8 同理，只作用于当前 Turn。owner：停止当前消息是友好停止，立刻完成 current turn。即不再发起新的推理和工具调用，当前 Turn 就此收尾，Session 和上下文都在，已创建的 Work 不受影响 | SC2、MH4 |
| D10 | 输入框何时禁用 | 按建议。只按“看得到的未结束占位”判定：最新一条带 `agent_task` 的 Agent 消息，其 task 运行中或已暂停时禁用；等待子任务、等待外部时不禁用；回答态优先，照常能答；占位出现之前不禁用；首次点击直接暂停 | MH4 |
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
7. **D20 的“不投递 user message”沿用今天的边界。** 用现成的 `session.input_policy` 表达。哪些 Session 收 user message 与今天能不能投一致：今天没有队列的 Session（聊天里直接建的 simple Work、`skills.import` 起的 Work）有了队列以后只收控制命令；UI、Goal、self_check、plan、Todo 会话、`--interactive` 的 Work 不变。（SC0）

下面四条是把 D21 展开成 §2.3 和 SC2 时推出来的，owner 的话里没有明说，等确认。不同意的在这里批注，改动只涉及括号里的任务。

8. **UI Session 收到针对 Session 的 stop 时拒绝，不降级。** 返回明确的错误，说明 UI Session 不能停、要清空上下文用“开始新对话”。不照搬 Goal Session 今天“悄悄降成只停当前 Turn”的做法，否则两种 stop 又混在一起。（SC2）
9. **Goal Session 收到针对 Session 的 stop，就是让它的 Goal 休眠。** 走今天 Goal 状态 `paused` 的那条路径（`goals.set_status`）：会话和状态都留着，不再开新的 Activation，唤醒后继续。今天这种 stop 是悄悄降成只停当前 Activation。（SC2）
10. **“休眠”先改用词，不改标识。** 界面文案和文档里 Goal 的“暂停 / 恢复”改成“休眠 / 唤醒”，和会话的“暂停”（SC1）分开。状态值 `paused`、CLI 的 `agent-session goal pause | resume`、kRPC 的参数先不改名。目标页的改版不在本单，这里只动这几个词。（SC2）
11. **TaskMgr 取消按 task 属于哪类会话来分。** 被取消的是某个 Turn 的 task。在 UI Session 和 Goal Session 上它是针对消息的 stop；在 Work 上保持今天的行为，结束这个 Work，因为在用户眼里那个 task 就是这件工作。（SC2）

### 2.2 之后在别处作出、影响本单的裁决

| 来源 | 裁决 | 对本单的影响 |
| --- | --- | --- |
| runtime TODO §12.3 裁决 1（11-09 晚） | Linux 必定有容器，Agent 的私有 runtime 就是容器；原生运行只在不带容器的产品（Lite） | RT2 里“私有 runtime 同时是 Owner 的机器，只显示一行”的情形只在 Lite 出现 |
| 同上，裁决 3 | 不要 `host` 占位。这个概念叫 owner default runtime，由 `default_runtime_id` 决定，没设就是私有 | RT2 不用再过滤占位记录；界面文案用“Owner 默认” |
| 同上，裁决 4 | 有在用的 Session 就不让停用，也不让移除，没有 `confirm` | 取代 runtime TODO 裁决 27 的“有影响先列出再确认”。RT2 的停用、RT4 的删除都是：被拦时列出这些 Session，没有“仍要继续” |
| 同上，裁决 5 | runtime 掉线按网络错误处理，不加定期探活 | 首页显示的可用性只反映上一次使用或检查的结果 |
| runtime TODO 裁决 28（11-09） | skill 是纯文本，不带命令也不带安装脚本；带命令的做成扩展包 | 规划 §3 的“Skill 与命令可独立安装”“扫描可拆解的 CMD Skills”按这个读：命令来自扩展包和 runtime 的基线，Skills 页只显示手册依赖的命令是否可用、哪个扩展包能补上 |

### 2.3 stop 的目标语义（D17、D21，待实施）

本节是 UI 升级的目标方案，包含 §2.1 推定 8–11，不是现行协议。现有控制合同统一见 [Session Control Protocol](<../doc/opendan/protocol/Session Control Protocol.md>)；具体差异与裁决状态见 §2.4。

| 会话类型 | 针对消息的 stop：关当前 Turn | 针对 Session 的 stop |
| --- | --- | --- |
| UI Session（含群聊） | 关掉正在处理的这条消息；会话和上下文留着，等下一条；已创建的 Work 不受影响（D9） | 没有。UI Session 不能停，结束它只有归档（clean，D11、D18）；收到时拒绝（推定 8） |
| Goal Session | 结束当前这次 Activation，Goal 照常等下一次机会。今天已经是这样 | 休眠：手工 disable。Goal 不再被激活，直到唤醒；会话和它的状态都留着。对应今天 Goal 的 `paused` 状态（推定 9、10） |
| Work Session（simple / plan / explore） | 关当前 Turn，Work 还在，之后的 tick 或子任务结束会再开 Turn。界面不提供这个入口 | 以 stopped 结束，连同它未结束的子会话。今天已经是这样 |

- **针对消息的入口：** 聊天命令 `/stop`、`/cancel`（SC9）；回答态的 Stop 按钮和占位消息上的“停止”（MH4）；TaskMgr 取消 UI / Goal 会话的 Turn task（推定 11）。接口是 `session.stop {scope: "turn"}`。
- **针对 Session 的入口：** 子会话树节点上的“停止”（MH5，节点都是 Work）；Agent 首页 Session 详情页的按钮（HM3，按会话类型显示）；Goal 的休眠在目标页，就是现有的“暂停”按钮，改名。接口是 `session.stop {scope: "session"}`；Goal 的休眠也可以直接调 `goals.set_status`。
- **“暂停 / 恢复”是第三件事（SC1）：** 会话暂时不再推理，随时接着来，对各类会话含义相同。Goal 的手工 disable 不叫暂停而叫休眠，就是为了不和它混。
- self_check 会话本单不涉及，行为不变。

### 2.4 与现有控制协议的差异（2026-11-10 文档核对）

现有设计已区分 Turn 停止、Session 终止、可恢复的 Session 暂停和 Goal 调度暂停，分层继续沿用。本次只整理文档，不改变代码、命令字段或 schema，也不把推定自动升级成裁决。以下变更实施前按裁决状态处理，实施时同步 [Session Control Protocol](<../doc/opendan/protocol/Session Control Protocol.md>)，输入、目录和架构文档引用该合同。

| 事项 | 现有合同 | 本单目标 / 推定 | 状态与任务 |
| --- | --- | --- | --- |
| 前台停止入口 | `stop(scope=turn)` 已有，保留 Session、不停子 Session；聊天 `/stop` 今天仍映射成 session 范围 | `/stop`、回答态 Stop、相关 Turn task 的取消选择 turn 范围 | D9、D17 已裁决；TaskMgr 按类型分派仍含推定 11。SC2、SC9 |
| UI Session 终止 | 协议允许 `stop(scope=session)` 使 UI Session 结束 | UI 只通过 clean 归档；直接 Session stop 报错 | D17 已裁决；报错方式为推定 8。SC2、SC6 |
| Goal 的 Session 级 stop | 任意 stop 只结束当前 Activation；Goal 的 `paused` 另行禁止新 Activation，已打开的 Turn 自然结束 | 将 Session 级 stop 映射成 Goal 休眠 | D21 要求区分停止与休眠；具体映射为推定 9，文案为推定 10。SC2；当前仍使用独立 Goal 状态操作 |
| 在飞调用 | stop 会请求中断当前执行，并按停止收尾路径尝试取消外部 task；不能保证所有外部任务均收到取消或已经停下 | 不再发起新调用，等待在飞调用返回后关闭 Turn | D9 的友好停止目标；SC2 属于执行语义变更，需明确等待时长、取消能力与“停止中”状态 |
| pause / resume 级联 | pause 默认向未结束且有队列的子级联；resume 只恢复因父级而暂停的子；当前无 `cascade` 字段 | 暂停前台消息可传 `cascade=false`，后台 Work 继续 | D8 已裁决。SC1；属于新增控制选项 |
| 控制队列 | 队列可选；无队列 Session 只有宿主停止路径，没有 pause / resume 旁路 | 所有 Session 有控制队列，是否接受 user message 另定 | D20 已裁决。SC0；属于输入与控制合同变更 |
| `/cancel` | 无独立 Session cancel 原语 | 聊天 `/cancel` 注册为 `/stop` 别名 | D22 确认的推定 6。SC9；不新增底层命令 |

共同约束：投递或 `control_applied` 不等于所有工具与子工作已经停下；停止不回滚外部副作用。暂停中的 Turn 被 stop 关闭后，Session 的暂停标记仍在，需要 resume 才允许继续推理。已结束 Session 拒绝 stop / pause / resume；重复 pause、未暂停时 resume 才是无效果的控制。SC1–SC3 的界面与 TaskMgr 状态映射须保留这些区别。

## 3. 接口变更一览

供前后端并行时对齐。名称可在实施时微调，改了要回写本表。除注明外都在 opendan 服务（`service.rs` + `home.rs` 或新模块），并归入 `is_read` / `is_operation` 与 `permission_tests`。

| 方法 | 读写 | 形状 | 任务 | 状态（11-10） |
| --- | --- | --- | --- | --- |
| `home.summary` | 读 | `{sessions: {working, waiting, failed, total}, memory: {perceptions, pending, cognitions, last_consolidation_at_ms}, goals: {active}, skills: {work, plan, goal, disabled}, runtimes: {enabled, unavailable}}` | HM2 | 未做 |
| `memory.overview` | 读 | `{cognitions: {current}, perceptions: {total, pending}, last_consolidation_at_ms, maintenance_goal_id, recent: {cognitions: MemoryEntry[], perceptions: MemoryEntry[]}}`；`recent` 是最近添加的 5 条认知和 3 条感知 | ME1 | 未做 |
| `memory.list {layer, limit, cursor?}` | 读 | `{entries: MemoryEntry[], next_cursor?}`，按添加时间倒序；`MemoryEntry = {layer, reference, summary, kind, state, at_ms, revised_at_ms?, subjects, session_id?, deletable, absorbed_into?}` | ME1 | 未做 |
| `memory.read {reference}` | 读 | 认知带来源视图，感知带原记录；不要 Session | ME1 | 未做 |
| `memory.forget {reference, expected_revision?, reason?}` | 操作 | 认知 → 新修订 `deleted`；感知 → `discarded` 并清理。整理 Goal 正在运行时返回 busy | ME2 | 未做 |
| `runtimes.settings` | 读 | `{runtimes: {<id>: {enabled, target}}, default_runtime_id, configured_default_runtime_id}`；后一项是 owner 配置的原始值 | RT1 | 已有 |
| `runtimes.settings_set {runtime_id, enabled?, default?}` | 操作 | 停用不清默认；拒绝停用私有及其别名；上面有正在运行的 Session 时拒绝停用（`sessions_in_progress`） | RT1 | 已有 |
| `runtimes.remove {runtime_id}` | 操作 | 从这个 Agent 移除一个已分配的 runtime，记录转 `retired`；拒绝私有及其别名；有正在运行的 Session 时拒绝。返回新的设置 | RT1、RT4 | 已有 |
| `runtimes.impact {runtime_id}` | 读 | `{runtime_id, owner_default, blocking, running_sessions, active_sessions, pending_todos, workspaces, usage}` | RT2、RT4 | 已有（runtime TODO R7） |
| `session.pause {sid, reason?, cascade?}` / `session.resume {sid, reason?}` | 操作 | 返回 `{index}`，同 `session.stop`。`cascade` 缺省 true；暂停当前消息时传 false。SC0 之后对任何未结束的 Session 都能投 | SC1 | 未做 |
| `session.stop {sid, reason?, scope?}`（已有方法，语义变化） | 操作 | `scope: turn` 是针对消息的 stop；`scope: session` 按会话类型：UI 拒绝，Goal 休眠，Work 结束（D21，§2.3）。SC0 之后不再有“没有队列”的旁路，一律返回 `{index}` | SC0、SC2 | 未做 |
| `session.post {sid, text}`（已有方法，语义变化） | 操作 | 目标 Session 不收 user message 时拒绝并说明（D20） | SC0 | 未做 |
| `ui.conversation {peer_did, session_id?}` | 读 | `{current: UiSessionView \| null, generations: [{session_id, created_at_ms, finished_at_ms?, outcome?}]}`；`UiSessionView = {session_id, run_state, paused, needs_input, one_line_status, open_turn?: {turn, task_id}, unfinished_works, prompt: {frozen_at_ms, current: bool, changed: string[]}}` | SC4、SC7 | 未做 |
| `session.work_tree {sid, unfinished_only?}` | 读 | `{agent_did, session_id, complete, sessions: WorkNode[]}`；`WorkNode` 是 `WorkCard` 加 `children: WorkNode[]`，Todo 绑定的卡片同样递归 | SC5 | 未做 |
| `ui.rotate {sid}` | 操作 | clean：归档当前 UI Session，下一条消息新建一代。有未结束 Work 时返回错误并带数量；成功返回 `{archived: sid}`，返回时登记表已显示结束 | SC6 | 未做 |
| 聊天命令 `/clean`、`/new`、`/cancel` | — | `/clean`、`/new` 做与 `ui.rotate` 相同的事并回一句结果；`/cancel` 同 `/stop` | SC9 | 未做（今天只注册了 `/stop`） |
| `sessions.delete {sid}` | 写 | 返回被删的 sid 列表（含后代） | SC8 | 未做 |
| `deliveries.recent {limit, before?, route_key?}` | 读 | `{entries: [{at_ms, name, obj_id, url, mime?, size?, session_id, ui_session_id?, route_key?, source: report \| reply}], next_before?}` | AR1 | 未做 |
| Desktop 深链 `?settings=<page>` | — | 打开 Settings 并定位到指定页面 | RT3 | Desktop 内的 `openSettings(page)` 已有；URL 参数未做 |

Skills 的其余接口见 `src/frame/opendan/README.md`「Skills」，设备目录与分配的接口见 runtime TODO §11，本表不列。读 plan / goal 手册正文的方法随 Skills 页移到[独立 TODO](opendan-skills-page-ui-todo.md)。`session.stop` 对没有队列的 Session 的旁路（`27aea9efb`）在 SC0 之后不再需要。

## 4. 任务

每项写目标、落点、要点、验收。

### 4.0 任务状态一览（2026-11-10，`27aea9efb`）

| 任务 | 状态 | 说明 |
| --- | --- | --- |
| HM1、HM2 | 未做 | Skills 页进了观测页的标签布局，没有首页样式的详情外壳 |
| HM3 | 未做 | Skills 卡片已经在目标和工作区之间；缺 Memory 卡片和 Session 删除入口。11-10 按 D21 加了一条：Session 详情页的 Stop 按钮分会话类型 |
| ME1–ME4 | 未做 | |
| RT1 | 已做 | 随 runtime TODO 提交（`35e72ce06`，按裁决的调整在 `27aea9efb`），本机 Zone DV 通过。调度器刷新的延迟没有量 |
| RT2 | 部分 | 已有：ComputerUse 与基线只读、停用前取影响清单。未做：瘦列表、三类、折叠的历史、详情页 |
| RT3 | 部分 | 已有：Desktop 内的 `openSettings('agent-runtimes')`。未做：URL 深链、首页的“添加 Runtime” |
| RT4 | 部分 | 后端和 WebUI 数据层已有；界面入口未做。它等的前置（目录页）已经落地 |
| SK1 | 不在本单 | 原来的占位页被 skills TODO 的 Skills 页取代；补齐规划形态的工作按 D19 移到[独立 TODO](opendan-skills-page-ui-todo.md) |
| WS1、WS2 | 未做 | |
| SC0 | 未做 | 11-10 按 D20 新增：所有 Session 都带输入队列。SC1 和 MH5 的前置 |
| SC1–SC8 | 未做 | SC2 按 D17、D21 改写成“区分两种 stop”，SC6 按 D17、D18 改写 |
| SC9 | 未做 | 11-10 按 D18 新增：聊天命令 `/clean`、`/new`、`/cancel` |
| AR1 | 未做 | |
| MH1–MH8 | 未做 | |
| DX1–DX3 | 未做 | 随各项实施 |
| DX4 | 部分 | runtime TODO 一侧 11-09 已回写；skills TODO 一侧只差加一行指向独立 TODO |

### 4.1 首页骨架（HM）

- [ ] **HM1 详情页外壳与路由。** `App.tsx` 增加首页样式的详情路由：`#/memory`、`#/runtime/<id>`、`#/workspaces`。这些页面用首页的样式（`home` 类、手机优先、中英文走 `i18n.ts`），顶部一个返回首页的链接；不要用观测页的标签布局。抽一个共用外壳组件，做成别的页面也能套用。`#/skills` 现在是观测页的一个标签（skills TODO 交付的页面），本单不动它（D19）；它以后搬不搬进这个外壳由 Skills 页的独立 TODO 定。验收：手机宽度下各页不横向滚动；刷新保持在当前页。
- [ ] **HM2 `home.summary`。** `home.rs` 新方法，一次返回 §3 的摘要，供首页概况和 MessageHub 面板上部共用。Memory 部分复用 ME1 的缓存；Session 计数的口径与 `Home.tsx` 的 `isWorking` / `isWaiting` / `isFailed` 一致（把判定挪到服务端或保持两边同一份定义，二选一并写测试）。归入 `is_read`。
- [ ] **HM3 首页布局调整与 Session 删除入口。**（D15）左列顺序：资料卡、工作概况、Memory、目标、Skills、工作区、Runtime。现在只缺 Memory 这一块（ME3），`SkillsCard` 已经在这个位置。`SessionDetail.tsx` 对可删除的会话显示“删除”按钮，二次确认，调用 SC8。同一页现有的 Stop 按钮按 §2.3 分会话类型（D21，依赖 SC2）：UI 会话上是“停止当前消息”，发 turn 范围，没有打开的 Turn 时不可点；Goal 会话上是“休眠”；Work 上仍是“停止”。验收：未结束的会话、UI 会话没有删除按钮；删除后回到首页且列表不再有它；三类会话上 Stop 按钮的文案和效果各如上述。

### 4.2 Memory（ME）

- [ ] **ME1 只读接口。**（D2）
  - 落点：`lib_opendan/src/memory/` 新增 owner 视图（建议单独文件），提供 `overview`、`list_perceptions`、`list_cognitions`、按引用读取；`opendan/src/home.rs`（或新模块）暴露 `memory.overview` / `memory.list` / `memory.read`。不进 Agent State 门面：只有 owner 的界面用。
  - 排序按“最近添加”（D2）：认知从 `Memory::graph_view()` 取，按添加时间倒序（`GraphView::item_created_at`，即首个修订写入的时间）；之后被修订过的显示当前内容，并带修订时间。感知用 `load_perceptions(None)`（改可见性或加包装），按 `at_ms` 倒序，排除 `is_runtime()` 的旧记录。状态用 Graph 的 dispositions 加 `cleared` 推出（待整理、暂缓、已吸收、已丢弃）。
  - `overview.recent` 固定给最近添加的 5 条认知和 3 条感知；数量做成常量。
  - 游标用 `(排序键, 引用)`，不要用现有的 `PageCursor`（任何新感知都会让它失效）。
  - `overview` 按 `MemorySnapshot::token()` 缓存，快照没变不重新扫文件。`last_consolidation_at_ms` 取整理 Goal state 的 `last_successful_consolidation_at_ms`，同时返回该 Goal 的 id。
  - 验收：组件测试覆盖空库、只有感知、有删除和替代修订的认知、清理后的感知；翻页期间写入新感知不丢不重；`cargo test -p libopendan --test memory_component`、`cargo test -p opendan`。
- [ ] **ME2 删除。**（D1）
  - 认知删除是必做项（owner：影响大的是认知，用户可以手工删掉一条）；未整理的感知走同一个接口顺带提供。先做认知这条路径并验收，再接感知。
  - 落点：`memory/consolidation.rs` 给 `Consolidator` 加 owner 发起的提交，仿照 `sweep_expired`：认知用 `SetStatus deleted`（带 `expected_revision`，不符返回冲突），感知用 `discarded` 处置后跑 `cleanup()`。服务端仿照 `decide_as` 取 `memory_consolidation` 租约，取不到返回 busy。`produced_by` 标明是 owner 操作，原因写进提交。
  - owner 删除不应推动“最近整理时间”：界面读的是 Goal state 里的时间，本路径不要去写它。
  - 已是终态的感知（已吸收、重复、已丢弃）拒绝，错误里带承接它的认知引用。
  - `runner/memory_batch.rs` 的撤回行：状态为 `deleted` 时不输出原陈述。
  - 验收：删认知后 `memory.list` 不再有它，`cognition.read` 返回 `deleted`，读过它的 Session 下一批收到不带原文的撤回行；整理 Goal 持有租约时删除返回 busy 且库不变；重复删除返回冲突。
- [ ] **ME3 首页 Memory 区块。** （D2）`Home.tsx` 新卡片：认知总数、感知总数（附待整理数）、最近整理时间（点击去整理 Goal 的详情页）；下方先是最近添加的 5 条认知，再是最近添加的 3 条感知，每条显示摘要、时间、范围。整块右上角“查看全部”去 `#/memory`。这块不要 4 秒轮询：用较长间隔或只在进入页面和手动刷新时取。数据层、mock、类型同步补齐。验收：e2e 断言计数、预览条数、空状态、整理 Goal 不存在时的显示。
- [ ] **ME4 Memory 详情页。**（D1）`#/memory`：认知、感知两个独立列表（手机上用页签，认知在前），按添加时间倒序，滚动加载；每条可展开看详情（`memory.read`）；删除按钮带确认，busy 时提示稍后再试并保留条目。不做添加和编辑。验收：e2e 覆盖翻页、删除成功、busy、已吸收感知没有删除按钮。

### 4.3 Runtime（RT）

本节只做 Agent 首页这一侧（D4）。设备怎么添加、怎么分配给 Agent，runtime TODO 已按两步流程做完（用户级目录 `agent.runtime_catalog`、daemon 设备两步化、Desktop 目录页、CLI，即它的 E1、E2、E5、E7；ssh 上提的 E3 按它的裁决 23 不做）。规划里“设备从 Settings → Cluster 管理添加”一句，现在对应 Desktop 的 Settings → Agent Runtimes（Add device 已从 Cluster Manager 搬过去）。本节的 RT2 是那份文档 R-E6 / E6 的细化：按规划改成“瘦列表 + 详情页”。停用和移除的规则以 §2.2 为准。

- [x] **RT1 设置语义修正与移除接口。**（D3、D4）已做：随 runtime TODO 实施并提交（`35e72ce06`），按 §2.2 的裁决 3、4 调整后在 `27aea9efb`。
  - 做了：`runtimes.settings` 返回 `configured_default_runtime_id`；停用不清 `default_runtime_id`（`home.rs` 的 `set_runtime_settings`、`app_schema.rs` 的 `validate_runtimes`、control_panel `agent_runtimes.rs` 的 `apply_update`）；私有 runtime 及其别名不能停用、不能移除；settings 里移除的条目记录转 `retired`，`usage` 保留，再分配回到 `active`；`runtimes.remove {runtime_id}`，默认指向它时一并清空，随后立即重新投影；`runtimes.settings_set` 的参数收成 `{runtime_id, enabled?, default?}`（runtime TODO R19）。
  - 与原文不同：停用和移除遇到正在运行的 Session 直接拒绝，没有确认参数（§2.2）；设备 runtime 的启用经 control panel 的 `agent.runtime_enable` 做独占检查；`host` 占位取消；原生运行只在 Lite，`native_agent_node` 在带容器的产品上恒为 `None`。
  - 测试：`opendan/tests/loader.rs` 的 `the_owner_switches_assigned_runtimes_and_work_in_progress_is_told`（settings、impact、remove、标记与通知，经 kRPC）；`lib_opendan/tests/runtimes.rs`（移除后退役并可回来、默认停用后保留）；`buckyos-api` 的 `native_agent_node_follows_the_default_runtime`（含默认被停用、带容器的产品）。本机 Zone 的 DV 见 runtime TODO §12.1 的 “R19 / RT1” 一行。
  - 没做：opendan 这条路径改完 settings 不会触发调度器刷新（control_panel 的 `agent.update` 会），节点上给这个 Agent 的执行权限要等调度器下一轮才收回。停用和移除后的这段延迟没有量；不可接受就补上刷新。
- [ ] **RT2 首页列表瘦身与详情页。** 依赖 HM1（RT1 已做）。已有的部分：卡片上 ComputerUse 与基线已经只读，停用前先取 `runtimes.impact`，被拦时列出 Session。下面各条里没有注明“已有”的都还没做。
  - 首页每行只有名称、一行说明、启停开关。分三类显示：Provider（`connector.type == 'native'`，无开关，标“始终启用”）、Owner 默认（`runtime_id == configured_default_runtime_id`）、其他已分配（settings 里有条目）。`host` 占位记录已经取消，不用过滤。Lite 下私有 runtime 同时是 Owner 的机器（记录的 `connector.aliases` 不空），只显示一行并加标注；带容器的产品上不会出现这种情形。
  - 列表末尾是折叠的“历史”：`retired` 的记录，显示名称、最后使用时间和用过它的 Session 数，点进详情只读。到期的 runtime 不进历史：它仍在当前列表里，标“已到期”，续期后恢复（runtime TODO 裁决 26）。
  - `#/runtime/<id>` 承接 `RuntimeCard` 现有的全部内容和操作（检查、设为默认、基线、ComputerUse、扩展包、工具检查、用量与最近 Session）。runtime 自己的配置（权限、ComputerUse 档位、声明基线、期限）在这里只读显示，改它要去 Desktop 的 Settings → Agent Runtimes：它们是 runtime 的声明，不按 Agent 设（runtime TODO 裁决 24、§7.2 推定 2、§10）。所以详情页上的操作只有检查、设为默认、启停和移除（基线与 ComputerUse 的两个下拉已经去掉）。`runtimes.ext_tools` 只在详情页请求。
  - 停用是可逆的。规则按 §2.2 的裁决 4：上面有正在运行的 Session 时不能停用，界面列出这些 Session，没有“仍要停用”；没有就直接生效。等待的 Todo 和绑定的工作区只显示，不拦。`RuntimeCard` 的 `toggle` 已经是这个行为（已有），搬到新的行和详情页上即可。
  - 重写 `tests/e2e/home.spec.ts` 里断言 runtime 卡片的三条用例（Lite 单行、声明的环境与工具检查、进行中的 Session 拦住停用）和 `mock.ts` 的对应数据。验收对应规划场景“Owner 没有 Default Runtime”：只见 Provider 一行；停用后条目仍在、可重新启用，默认标记不丢。
- [ ] **RT3 “添加 Runtime”引导与深链。** 首页 Runtime 区块的“添加 Runtime”打开说明：设备在 Desktop 的 runtime 目录里添加，并在那里分配给 Agent；这里只管这个 Agent 用不用它。按钮跳到 `${profile.desktop_url}/?settings=<page>`（在桌面窗口里要跳出 frame，做法同 `messageHubHref` 的 `outerTarget`）；Zone 外没有 `desktop_url` 时只显示说明。目标页面是 Settings → Agent Runtimes，页面 id `agent-runtimes`（`navigation.ts` 的 `AGENT_RUNTIMES_SETTINGS_PAGE`），已经落地。Desktop 侧只差 URL 这一步：在 `DesktopRoute.tsx` 处理 `?settings=<page>`，仿照 `?agent_setup`，校验页面 id 后调现成的 `openSettings(page)`；从 launch payload 到 `SettingsShell` 的 `initialPage` 这一段已有。首页现在只有一句文字提示（`runtimeManagedHint`），没有按钮；这句文案还写着去“用户与 Agent”里设置，一并改成目录页。验收：从 Agent 首页点击后 Desktop 打开 Settings 并停在目标页。
- [ ] **RT4 从 Agent 移除 runtime。**（D4；“删除”就是从这个 Agent 移除，见 §2.1 去向表第 3 行）依赖 RT1、RT2。
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

- [ ] **WS1 区块改版。** 标题改“工作区”（i18n 键和 `data-testid` 一并改名，更新 e2e）。默认收起，收起时显示最近使用的 5 张小卡片：标题、Runtime 名称、目录、Remote URL（有才显示）、描述（一行）。“最近使用”取绑定该工作区的 Session 的最近更新时间，没有 Session 的用记录的 `updated_at_ms`。Runtime 和目录始终显示，用来区分同名工作区。“查看全部”去 `#/workspaces`，那里保留现有完整卡片（检查、改 URL、迁移）。预览数量是规划的待定项，先定 5 并做成常量。
- [ ] **WS2 手工添加向导。** 把“创建”“导入”两个表单合成一个分步向导：第 1 步选 Runtime，写明“这一步决定 Agent 在哪台机器上访问目录”，只列可用且允许工作区的 runtime；第 2 步选“已有目录”或“新建目录”并填目录、Remote URL、标题、描述；第 3 步提交并显示检测结果。提交仍用 `workspaces.import` / `workspaces.create`，失败时停在第 3 步显示原因并可返回修改。实施时确认导入失败不会留下登记，并记录目录里是否会留下元数据文件。检测细则等工作区专项文档，本项不新增检测逻辑。验收对应规划场景“用户在指定 Runtime 中手工添加工作区”。

### 4.6 会话控制后端（SC）

本节按 §2.4 区分已有协议与待实施变更。控制语义先在 `doc/opendan/protocol/Session Control Protocol.md` 同步；输入、持久字段、SDK / CLI、服务与界面按实际影响联动，不只更新界面用词。涉及 §2.1 推定 8–11 的部分仍按待确认项处理。

- [ ] **SC0 所有 Session 都带输入队列。**（D20；11-10 新增）控制命令（pause、resume、stop、decide）一律走队列；队列收不收 user message 是另一个属性。SC1、MH5 的前置。
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
- [ ] **SC1 pause / resume 接口与不级联选项。**（D8）依赖 SC0。暂停当前消息 = 对 UI Session 发不级联的 pause：只挡住正在处理这条输入的 Turn，它建出的 Work 在后台照常运行。Work 的暂停走现有的 Session 暂停流程（级联到它自己的子会话，行为不变）。`ControlCommand::Pause` 增加是否级联（缺省级联）；`apply_control` 据此决定是否调 `cascade_pause`。`service.rs` 增加 `session.pause` / `session.resume`，归入 `is_operation`。SC0 之后所有 Session 都有队列，这两个方法对任何未结束的 Session 都能投，级联也不再跳过谁。同步 `doc/opendan/protocol/Session Control Protocol.md` 里的 pause / resume 与级联合同，Input / Directory 只联动输入和持久字段说明。验收：对有未结束子 Work 的 UI Session 发不级联的 pause，子 Work 的 `paused` 仍为空；resume 后 Turn 从检查点继续；缺省 pause 的现有测试不变。
- [ ] **SC2 区分针对消息的 stop 和针对 Session 的 stop。**（D9、D17、D21；语义表在 §2.3）
  - 针对消息的 stop（`scope: turn`）目标行为是友好停止：收到后不再发起新的推理和工具调用，在飞的调用返回后当前 Turn 立刻收尾并以 stopped 关闭，占位改成“已停止”，Session 回到等待，上下文保留。
  - 先写测试确认现状：UI Turn 在 `analyze_intent` 子 context 里收到 turn 范围 stop 时，Turn 是否以 stopped 关闭、Session 是否回到等待、占位是否被改成“已停止”文案（Agent 包 `i18n` 的 `[outbound] stopped`）。按结果修 `runner/outcome.rs` 的 `turn_stop && !site.child` 分支。
  - 把针对消息的入口都改成发 turn 范围：`/stop`（`bridge/msg.rs` 今天发的是 session 范围）；`CancelBridge`（`tasks.rs`）对 UI Session 改用 turn 范围，Goal 已经是，Work 保持 session 范围（§2.1 推定 11）。没有打开的 Turn 时不生效，`/stop` 这时回一句提示。
  - 针对 Session 的 stop（`scope: session`）在 `runner/inputs.rs` 的 `apply_control` 按会话类型分派。今天那里只有一条特例：Goal Session 一律降成 turn。改成：UI Session 拒绝，并说明要清空上下文用“开始新对话”（推定 8）；Goal Session 让它的 Goal 休眠，走 `goals.set_status` 置 `paused` 的同一条路径（推定 9）；Work 不变。`session.stop` 对 UI Session 在服务端直接返回错误，不要等消费时才记一条未生效。休眠时正在进行的 Activation 怎么收尾，先看 `goals.set_status` 今天的行为再定，写回 §8。
  - 用词：Goal 的手工 disable 叫“休眠”，恢复叫“唤醒”。opendan WebUI 目标详情页今天写的是“暂停 / 已暂停 / 恢复”（`web/src/i18n.ts` 的 `opPause`、`gsPaused`、`opResume` 等，按钮在 `GoalDetail.tsx`），改掉；英文文案实施时定。状态值 `paused` 和 CLI 的 `goal pause | resume` 不改名（推定 10）。
  - 更新 `doc/opendan/protocol/Session Control Protocol.md` 的停止作用域、生效边界与 Goal 映射，联动 Directory / Input 的字段与消费说明；更新 `opendan/tests/loader.rs` 相关用例、`ui_core` 手册里对 `/stop` 的说明、`opendan/README.md`「操作」一节。
  - 验收：停止一条正在处理的消息后，同一个 UI Session 接着处理下一条消息，上下文还在，这个 Turn 之前已创建的 Work 不受影响。对 UI Session 发 session 范围的 `session.stop` 被拒，会话不变。对 Goal Session 发 session 范围的 stop 后，Goal 的状态是 `paused`，不再有新的 Activation，唤醒后恢复；发 turn 范围的只结束当前 Activation。对 Work 发 session 范围的 stop 仍以 stopped 结束。
- [ ] **SC3 暂停状态进 TaskMgr。** `TaskService` 增加暂停上报，`TurnTasks::update` 在 Session 暂停时把 Turn task 置为 TaskMgr 的 `Paused`，恢复后回到 Running。TaskMgr 一侧发起的 Pause / Resume 请求不桥接（Task Center 上不出现这两个按钮，或出现但不生效，实施时确认并记录）。验收：暂停后 `getTask` 的状态是 Paused，Desktop 的 `taskNodeState` 得到 `paused`。
- [ ] **SC4 `ui.conversation`。** 给定对话方和 MessageHub 的会话 id，返回当前这一代 UI Session 的状态和历代列表（匹配规则：`outbound` 的 `to` 等于对话方；`to_session` 等于会话 id，或会话 id 是 `dm:` 开头且 `to_session` 为空）。当前一代取未结束的最新一条。归入 `is_read`。这是面板下部、提示词标签、Clean History 入口共用的一次读取。
- [ ] **SC5 `session.work_tree`。**（D12）在 `work_card.rs` 增加按 Session 汇总的树：该 Session 的全部子 Work（可只取未结束的），plan 的 Todo 绑定卡片递归，非 Todo 的子 Work 挂在 `children`，深度上限 4。`complete` 的含义同 `work_cards`。现有 `session.work_cards` 不变。验收：两个并列 Work、一个 plan 带三个 Todo、其中一个 Todo 的会话又建了子 Work 的夹具，结构与预期一致；全部结束后 `unfinished_only` 返回空且 `complete` 为真。
- [ ] **SC6 归档当前 UI Session（clean）。**（D11、D17、D18）
  - 语义（D18）：保存当前聊天会话对应的 UI Session，再有消息时新建一代 UI Session 并关联到同一个聊天会话。MessageHub 的“开始新对话”（MH6）和聊天命令 `/clean`（SC9）都是它。新一代是懒创建的，按同一个 `route_key` 关联（D11 c，§2.1 推定 3）。
  - `ui.rotate {sid}`：确认是本进程驱动的 UI Session；有未结束子 Work 时拒绝并返回数量；否则结束旧会话，等登记表显示结束后返回（有上限，超时返回进行中）。
  - 结束旧会话不能再用 stop 控制命令：SC2 之后针对 Session 的 stop 到了 UI Session 会被拒绝（D17、D21）。归档要有自己的路径，两个候选：新增一条只给归档用的控制命令；或者用 Supervisor 的 `StopSignal`（`Supervisor::stop` 现成的做法）。选哪条实施时定并写回。旧会话的结局仍记 `stopped`，worklog 记下是归档以及谁发起的（D11 d，§2.1 推定 2）。
  - 修既有竞态：`session_for` 选当前会话时，对登记表里“未结束”的候选再看一眼 `state.json`，已请求停止或已结束的不再投递；该 inbox 的记录留到旧会话结束后由新一代接收。对应用例 `inbox_messages_are_answered_along_the_same_conversation` 的偶发超时。
  - 旧会话的目录、worklog、Session id 都保留，Memory 里的原文引用继续可用。
  - 验收对应规划场景“Clean History 后回查旧 Memory”：归档后下一条消息由新的 Session id 处理，`session.read` 旧 sid 仍能读到 worklog；归档后立刻发消息不丢。
- [ ] **SC7 提示词版本状态与压缩时刷新。**（D14）
  - 第一步（只读）：在 `state/behaviors.rs` 加比较函数，把 `prompt.frozen` 的身份文本、已冻结 behaviors、cmd_skills 与 catalog 当前内容逐项比较，返回是否一致和不一致的项目名；`ui.conversation` 的 `prompt` 用它。比较的另一边是“现在新建一个同类会话会冻结到什么”：除了文件内容，还要算上 owner 改过的 UI 集合和被禁用的手册（`.meta/skills_state.json`），这两样不反映在 `behaviors.revision` 里（§1.7）。
  - 第二步：读 `runner/live.rs` 的上下文重写和 `runner/history.rs`，确认压缩后系统前缀是怎么重建的，把结论写回本单 §8，再实现：UI / group 会话在压缩提交点重新冻结（替换 `prompt.frozen`，worklog 记 `control_applied`），Work 会话不动。重新冻结后 cmd_skill 的可用性按冻结规则重算。
  - 验收对应规划场景“Agent root fs 的系统提示词更新”：改 `role.md` 后旧会话 `prompt.current` 为假、新会话为真；旧会话发生一次压缩后变为真，且压缩前后的对话不重复不丢。
- [ ] **SC8 删除 Session。**（D15）登记表增加删除：只接受已结束的 Work / self_check，后代全部已结束才允许，连同后代一起删除目录和登记项；`state/perception/<sid>.jsonl` 不动（Memory 的来源）；登记表旁的 `.runtime_unavailable/<sid>` 标记一并清掉。门面增加 `sessions.delete` 并归入 `is_write`；`permission_tests` 现在把它当作未归类的方法断言被拒（`read_only_agent_cannot_edit_profiles_or_call_unclassified_methods`），一并改。验收：有未结束后代时拒绝；UI、Goal 会话拒绝；删除后 `sessions.query`、`session.read` 都查不到。
- [ ] **SC9 聊天命令 `/clean`、`/new`、`/cancel`。**（D17、D18；11-10 新增）依赖 SC2、SC6。给没有界面的渠道用，今天就是 Telegram。
  - 现状：`opendan/src/ui.rs` 的 `bridge_ctx` 只注册了 `stop`。桥已经有“由宿主自己处理的命令”这条输出（`SlashCommand::App` → `MsgBridgeOutput::AppCommand`），`drain` 收到后只确认、什么都不做。Telegram 的菜单（`msg_center/src/tg_tunnel.rs` 的 `TG_BUILTIN_COMMANDS`）登记了 `new`、`clean`、`stop`、`cancel`、`info`、`list`、`switch`、`help`，除 `stop` 外发出去都被当成普通文字交给模型。
  - `clean`：注册为 `SlashCommand::App`，`drain` 里对这条消息所在聊天会话的当前 UI Session 做 SC6 的归档，回一句结果。有未结束的 Work 时拒绝，回复里列出它们（D11 b，§2.1 推定 5）。没有当前 UI Session 时回“已经是新对话”。
  - `new`：Agent 一侧与 `clean` 相同（§2.1 推定 4）。Telegram 是单 session：`build_telegram_ui_session_id(bot, chat)` 给每个聊天固定一个会话 id。
  - `cancel`：注册成 `stop` 的别名（§2.1 推定 6）。
  - Telegram 菜单的说明：`clean` 现在写的是 “Delete current session and create a new one”，与“保存”的语义不符，改掉；`new` 在单 session 下与 `clean` 的说明对齐。`info`、`list`、`switch`、`help` 仍然没有处理，不在本项。
  - 回复的发送路径与 SC2 里 `/stop` 的提示共用，文案进 Agent 包的 `i18n`。更新 `ui_core` 手册和 `opendan/README.md`。
  - 验收：opendan 集成测试里发 `/clean`，下一条消息由新的 Session id 处理，旧 sid 仍可读；有未结束的 Work 时被拒且会话不变；`/new` 与 `/clean` 结果相同；`/cancel` 与 `/stop` 结果相同。真实 Zone 上要配一个 Telegram bot 才能走，没有条件就如实标未验。

### 4.7 产物索引（AR）

- [ ] **AR1 交付索引与查询。**（D13）
  - 在交付成功的地方追加一条记录（落点在 `runner/reports.rs` 调用 `ObjectStore` 之后、能拿到 Session 上下文的位置；`--dev` 没有对象存储时不写）。记录交付时间、名字、对象 id、`obj://` 地址、类型和大小、交付它的 Session、它所属的顶层 UI Session 与 `route_key`、来源是报告还是回复。同一对象同一 Session 重复交付只记一次。
  - 文件放在 AgentRoot 的 state 下，只追加；写明格式并补读写测试（`harness/trigger_rules.md` §3.2）。
  - `deliveries.recent`：按时间倒序分页；给 `route_key` 时只返回该对话的。归入 `is_read`。
  - 验收：Work 的报告带一个文件、UI 回复带一个附件，两条都能查到且会话归属正确；按 `route_key` 过滤只剩本对话的。

### 4.8 MessageHub 侧面板（MH）

共同要求：面板的数据只在面板打开时取；刷新沿用 `conversation/worksessions/watch.ts` 的做法（TaskMgr 树提示驱动，15 秒兜底，`complete` 后停止）；OpenDAN 调用统一走 `agentServiceClient`；调用失败（不是 owner、Agent 离线）时相应区块显示不可用，不影响面板其余部分。每项都要补 mock（`mock/` 下新增 OpenDAN 侧的假数据源）、i18n（`src/i18n/messagehub.ts` 中英文）、`UI_DATAMODEL.md`。

- [ ] **MH1 统一面板外壳。**（D16）新组件（例如 `AgentSessionPanel.tsx`），上下两部分，中间可滚动；`MessageHubView.tsx` 的 `detailsTarget` 增加一种取值。桌面端在 `ConversationView` 头部加 Info 按钮，开关状态记在本地；手机端全屏，保留点标题进入，新增从会话视图左滑进入、右滑返回（MessageHub 里没有现成手势，注意不要和消息列表的滚动、语音录制的按压冲突）。只对“对方是自己名下 Agent 的会话”启用；其余会话入口和详情栏不变。头像、名字、副标题的点击在 Agent 会话里都指向新面板。`CodeAssistantAppPanel` 保持可编译。验收：`messagehub-ux.spec.ts` 增加桌面开关、抽屉模式、手机全屏和手势的用例；非 Agent 会话的现有用例不变。
- [ ] **MH2 上部：Agent 全局区。** 依赖 HM2。压缩的头部（小头像、名字、在线状态、DID 可复制）；状态摘要一行或一组小数字（进行中、等待、失败、感知与认知数、最近整理时间），来自 `home.summary`；“打开 Agent 首页”用现成的 `openAgentHome`。实体设置：免打扰和置顶接到 `updateEntityPreferences`（顺带修好 `EntityDetails` 里没接线的两个按钮）；实体处于免打扰时，新建的会话继承静音（改 `api/store.ts` 建会话的路径）。验收：免打扰后未读总数和托盘不再计入该 Agent，新建会话也是静音的。
- [ ] **MH3 下部：当前会话与子会话树。** 依赖 SC4、SC5。显示会话标题和描述、Agent 侧状态（处理中、空闲、已暂停、等待回答，加一句话状态）、当前是第几代会话。“Sub Sessions”区块用 `session.work_tree {unfinished_only: true}` 渲染：卡片行复用 `WorkSessionCards.tsx` 的 `WorkCardRow` 和 Todo 列表组件，把它们改成能递归；默认展开到 Todo 列表。点卡片打开该 Session 的工作日志（现有的消息详情页）。没有未结束的 Work 时显示空状态。现有 `SessionDetails` 的内容（成员、偏好、共享状态、管理、技术信息）收进面板底部的折叠区，不丢功能。验收对应规划场景“一个聊天会话创建多个后台工作”。
- [ ] **MH4 消息控制与输入框联动。**（D8、D9、D10）依赖 SC1–SC3。
  - 在占位消息的任务区（`MessageTaskArea` 和 `MessageAgentWork` 的头部）加按钮：运行中显示“暂停”；已暂停显示“恢复”“停止”。暂停调 `session.pause {cascade: false}`，停止调 `session.stop {scope: "turn"}`。Session id 和 Agent DID 取自 Turn task 的 input（现有的 `taskWorklogTarget`）。点击后到状态变化前显示“暂停中”“停止中”。
  - 输入框：从时间线和任务状态推出“前台消息状态”（放在 `conversation/input/inputState.ts` 旁边，纯函数，可单测），运行中或已暂停时禁用发送并在输入框上方显示原因和同样的控制按钮；回答态优先。`ConversationComposer` 增加禁用入参。已排队未发出的消息保持排队。
  - （D17）回答态的 Stop 按钮改调 `session.stop {scope: "turn"}`，不再发 `/stop` 文本。
  - 把 `MessageTaskArea` 第一行的英文实时状态（`working (analyze_intent)` 等）本地化，复用 `worksessions/model.ts` 的 `localizeLine`。
  - 验收对应规划场景“用户暂停正在分析的消息”：暂停后不能发送；恢复后继续；停止后可发送且上下文还在；Turn 进入等待子任务后可发送。deno 数据模型测试覆盖状态函数的全部分支。
- [ ] **MH5 子会话树节点控制。**（D8、D20）依赖 MH3、SC0、SC1。后台 Work 的暂停走 Session 的暂停流程：未结束的节点上有“暂停”，已暂停的有“恢复”“停止”，调 `session.pause`（缺省级联，暂停 plan 就暂停它的 Todo 会话）/ `session.resume` / `session.stop`。停止要二次确认。直接对 Todo 的会话发控制不会写 plan 的 `TodoAttempt` 暂停标记（那是 `agent-session todo pause` 写的），卡片状态以会话自己的 `paused` 为准；验收时确认 plan 的下一次 tick 不把它当异常。已停止的节点上放一个“在 Agent 首页查看”的链接，删除在那边做。SC0 之后所有 Session 都有队列，树上每个未结束的节点都能暂停、恢复、停止，界面不用区分节点的种类。
- [ ] **MH6 Clean History 入口。**（D11、D18）依赖 SC6。面板下部的会话控制区加“开始新对话”，调 `ui.rotate`，与聊天命令 `/clean` 是同一个操作；MessageHub 是多 session 的，new 对应现有的“新建会话”，不动。确认框说明：Agent 从全新上下文开始，旧会话归档可查，消息记录不删除。有未结束 Work 时按钮禁用并说明原因。成功后在时间线插入一条本地分隔标记（按归档时间定位，取自 `ui.conversation` 的历代列表，刷新后仍在）。头部 ⋮ 菜单的“清空聊天记录”保持原样，两处文案要能让人分清。调用期间禁用输入框，返回后恢复。
- [ ] **MH7 提示词版本标签。** 依赖 SC7 第一步。面板下部显示“与 Agent 当前版本一致”或“Agent 版本已更新”，后者的提示里写“开始新对话或下次历史压缩后生效”，并列出变化的项目名。标为高级信息，放在折叠区。
- [ ] **MH8 近期产物区块。**（D13）依赖 AR1。默认收起，展开后“全部”“当前会话”两个页签，按时间倒序，滚动加载。每条显示缩略图或类型图标、名字、时间、来源会话。“更多”菜单：下载、收藏、转发（按单个文件构造转发内容，复用 `store.forward` 和 `ForwardDialog`）、定位到消息（仅当前会话且消息已加载或可加载时）。预览复用现有的媒体预览。对象读取走哪条访问路径（msg-center 的 ObjectAccess 需要消息上下文，“全部”页签里别的会话的产物没有）在实施第一步确认并写回 §8。

### 4.9 文档与手册（DX）

- [ ] **DX1 协议文档。** `doc/opendan/protocol/Session Control Protocol.md`（控制合同主入口：pause 不级联、stop 的作用域与生效边界、Goal 休眠及归档的关系）、`Session Input Protocol.md`（所有 Session 都有队列与 `input_policy` 的用法，引用控制合同）、`Session Directory Protocol.md`（输入通道与控制的持久表示）、`Agent State Protocol.md`（交付索引、Session 删除）、`src/frame/opendan/README.md`（新方法、`/stop` 的新含义、`/clean` / `/new` / `/cancel`、产物索引）。runtime 的移除与退役、`runtimes.settings*` / `remove` / `impact` 已由 runtime TODO 写进 Agent State Protocol 和 README，本项不再包含。
- [ ] **DX2 界面文档。** `src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md`（新面板、前台消息状态、子会话树与产物的数据通路；顺带更正 §4.4 关于运行状态键由 OpenDAN 写入的说法，后端没有写入方）、`product/message_hub/MessageHub_Web_UI_PRD.md` §12、`product/message_hub/MessageHub_Current_UI_Model_Data.md`。
- [ ] **DX3 cmd_skills 联动。** 按 `AGENTS.md` 的表逐项核对并在提交说明里写结论：`ui_core`（`/stop` 语义、`/clean` / `/new` / `/cancel`、暂停）、`opendan_agent_dev`（Session 行为、输入队列与 `input_policy`、新增的服务方法；它的模板示例里有 `input = "queue"`）。本单不改 skill 的安装 kRPC 和 AgentRoot 布局。
- [ ] **DX4 回写关联 TODO。** 这一项只改文档。runtime TODO 一侧已做，skills TODO 一侧没做。
  - skills TODO：原计划是把界面要求写进去让那边照着做，但那边已经实施（它的文档里没有提到本单）。按 D19，页面形态的差距另立了 [Skills 页界面形态 TODO](opendan-skills-page-ui-todo.md)；这里只剩在 skills TODO 的 §0 加一行指过去。
  - runtime TODO：在 R-E6 / E6 注明 Agent 首页的形态按规划改为“瘦列表（三类）+ 详情页 + 历史”，由本单 RT1–RT4 实施；在 E5 注明 Agent 首页“添加 Runtime”会深链到目录页；在 R-D1 / R-D3 注明 Agent 视角的“删除”是 `runtimes.remove`。（2026-11-09 已回写：那份文档的 §9.2，以及 R-E6、R-D1、R-D3、E5、E6 各行的状态列。它的裁决 24、26、27 和 §7.2 的答复已写回本单的 RT1、RT2。）

## 5. 顺序与并行

裁决已定（§2，含 11-10 的 D17–D22）。§2.1 还有四条推定等确认（第 8–11 条），都只影响 SC2 和 HM3 里 Stop 按钮那一条，动到它们之前先看有没有新的批注。

1. **后端先行，三条线互不相干，可并行：** Memory（ME1 → ME2）；会话控制（SC0 → SC1 → SC2 → SC3，SC4、SC5 可同时做，SC6、SC7、SC8 在其后，SC9 在 SC2、SC6 之后）；AR1。RT1 已做。SC0 改的是 Session 的基础假设，单独一批做完并跑全量回归，再叠后面的。
2. **首页：** HM1 最先；ME3、ME4、RT2、WS1、WS2 在各自依赖完成后并行；RT3 随时可做，RT2、RT4 的后端都已经有了，只差界面。它们都改 `Home.tsx`、`i18n.ts`、`model/*`，分给多个 Agent 时要约定合并顺序，或由一个 Agent 串行做。
3. **MessageHub：** MH1 最先；MH2、MH3、MH4 并行；MH5、MH6、MH7、MH8 跟在各自依赖后。它们都改 `MessageHubView.tsx`、`messagehub.css`、`i18n/messagehub.ts`，同样注意合并。
4. **原来要等另外两份 TODO 的部分已经不用等：** 设备目录与分配（runtime TODO E1、E2、E5、E7）和 Skills 的分组、禁用、安装（skills TODO）都已提交，RT4 的界面入口可以直接做，对应的两个规划验收场景可以走全流程（§6.4）。
5. **同一批上线：** SC2（`/stop` 只停当前 Turn）和 SC9（`/clean`）。只上前者，Telegram 的用户就没有办法清空上下文了。

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
| 用户暂停正在分析的消息 | SC1–SC3、MH4 | Desktop e2e 与 deno 状态函数测试；真实 Zone 走暂停、恢复、停止三条路径 |
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

**风险：**

- **SC0 改的是 Session 的基础假设（D20）。** “没有队列”今天在好几处被当作判据（子会话怎么停、等输入的 Session 何时再看、验收怎么落、首批材料走哪条路），模板键 `input` 的含义也要变。它影响所有建 Session 的路径和大量现有用例，要单独一批做并跑全量回归；没有队列客户端的开发宿主怎么办还没定。
- **停止语义变更影响面大（D9、D17、D21）。** 两种 stop 分开之后，结束一个 UI Session 只剩归档这一条路，Goal Session 的 Session 级 stop 从“只停当前 Activation”变成休眠。所有发 stop 的地方都要对一遍它发的是哪一种：`/stop`、TaskMgr 取消、回答态按钮、`xagent ctl stop`、WebUI 的 Stop 按钮、父会话结束时对子会话的 stop。SC2 和 SC9 要同一批上线，否则 Telegram 的用户没有办法清空上下文。
- **“暂停”和“休眠”容易混。** 会话的暂停（SC1）和 Goal 的休眠是两件事，但 Goal 的状态值、CLI 动词今天都叫 pause。只改界面用词的话（§2.1 推定 10），文档和手册里要写清楚对应关系。
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
