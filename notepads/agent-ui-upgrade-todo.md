# BuckyOS Agent UI 升级 TODO

状态：2026-11-09 起草；同日 owner 在文中批注了 D1–D16，已整理成 §2 并回写任务。**已裁决，未实施。** §2.1 有四条是起草者按批注推出来的，等确认。依据 [BuckyOS Agent UI 升级规划](<BuckyOS Agent UI 升级规划.md>)（下称“规划”）逐节对照 2026-11-09 工作树源码写成。任务后面的（Dn）表示它按哪条裁决做。

范围：Agent 首页（OpenDAN WebUI）与聊天侧的 Session 侧面板（Desktop MessageHub），以及这两处界面需要的后端接口（libopendan、opendan、control_panel、msg_center）。目标管理（规划 §5）本轮沿用现有 UI，不在任务里。

读法：

- 路径相对 `buckyos/`。行号是 2026-11-09 工作树的近似值，以符号名为准，实施前重读。
- 工作树里有未提交的 MessageHub UI 优化（`src/frame/desktop/src/app/messagehub/` 多个文件、`src/frame/msg_center/src/{msg_center.rs,owner_session.rs}` 的 `msg.clear_session`、`src/kernel/buckyos-api/src/msg_center_client.rs`）。以工作树为基线，不得覆盖。
- 按 `AGENTS.md`：不做旧版本兼容；改协议、字段、存储结构时同步检查 `buckyos-api` 共享类型、前后端和文档；改 UI DataModel 的 TS 文件触发 `harness/trigger_rules.md` §3.4 的追加检查。
- 两份已有 TODO 与规划重叠，按裁决由它们负责各自的部分，本单不重复设计：[Skills 体系改进计划](opendan-skills-system-todo.md)（下称 skills TODO）负责 Skills 的分组、停用、安装器（D5–D7），本单只做占位页；[Runtime 工作模型与生命周期](opendan-agent-runtime-lifecycle-todo.md)（下称 runtime TODO）负责设备目录与分配（D4），本单只做 Agent 首页一侧。气泡下的 Work Session 卡片见 [done/messagehub-worksession-ui-todo.md](done/messagehub-worksession-ui-todo.md)。

## 0. 术语对照

规划里的词在代码里叫什么。实施和写测试时用右边的名字。

| 规划用词 | 代码里的对应 | 位置 |
| --- | --- | --- |
| Agent 首页 | OpenDAN WebUI 的 Home（每个 Agent 一份，由该 Agent 的 opendan 进程提供） | `src/frame/opendan/web/src/pages/Home.tsx`；数据经 kRPC `/api/opendan`（同一服务在 Zone 内是 `/kapi/<agent_id>`） |
| Session 侧面板 | MessageHub 的详情栏：`detailsTarget` 切换的 `EntityDetails` / `SessionDetails` | `src/frame/desktop/src/app/messagehub/MessageHubView.tsx` |
| 感知 | `PerceptionRecord`，按写入会话分文件 | `<agent_root>/state/perception/<sid>.jsonl`；`lib_opendan/src/protocol/agent_state.rs` |
| 认知 | `MemoryItem`（Memory Graph，全 Agent 一份） | `<agent_root>/memory/`；`agent_tool/src/agent_memory/model.rs` |
| 整理 | 内置 Goal `memory_maintenance` 的 consolidation | `lib_opendan/src/host/maintenance.rs`、`runner/consolidation.rs` |
| Provider Runtime | 内置私有 runtime `private`（旧名 `container` / `local`），永远启用 | `PRIVATE_RUNTIME`，`lib_opendan/src/protocol/runtime.rs` |
| Owner 的 Default Runtime | `AgentSettings.default_runtime_id` | `src/kernel/buckyos-api/src/app_schema.rs` |
| 分配给 Agent 的 Runtime | `AgentSettings.runtimes` 里的条目（`Node` 或 `Executor` 目标） | 同上；投影成 `RuntimeRecord` 在 `protocol/runtime.rs` 的 `builtin_runtimes` |
| CMD Skill | `cmd_skills/<name>.md`（+ 可选 `tools/<name>/bin`），`SkillKind::Cmd` | `lib_opendan/src/skills.rs`、`protocol/behavior.rs` |
| UI Skills | `agent.toml` 的 `[session.ui]` / `[session.group]` `default_cmd_skills`，今天只有 `ui_core` | `src/apps/jarvis_agent/agent/agent.toml`、`lib_opendan/src/template.rs` |
| Plan Skills | **不存在** | — |
| UI Session / Chat Session | `SessionKind::Ui`，靠 `route_key`（Agent 的 mailbox 地址）绑定会话 | `src/frame/opendan/src/ui.rs` |
| Default Session 指针 | **没有存储的指针**：同一 `route_key` 下最新的未结束 UI Session 就是当前会话 | `UiModule::session_for` |
| Work Session | `SessionKind::Work`：simple（`work_simple`）、plan（class `plan_simple`）、explore | `lib_opendan/src/work_card.rs` |
| Todo 列表 | plan 会话的 `todos.json` | `lib_opendan/src/todos.rs`、`agent_tool/src/todo_tools.rs` |
| 占位消息 | 带 `agent_task` 的消息，指向该 Turn 的 TaskMgr task（`opendan.agent_turn/v1`） | `src/frame/opendan/src/tasks.rs`；Desktop `conversation/tasks/`、`conversation/worksessions/` |
| 系统提示词版本 | Session 创建时冻结的 `prompt.frozen`（`FrozenPrompt`）与 behavior catalog 的差异 | `lib_opendan/src/state/behaviors.rs` 的 `freeze_config`、`revision` |
| 近期产物 | 交付进 Agent `artifacts` 桶的文件（`obj://bucket/...`）。**不是** Agent State 的 `artifacts.*`（那是 Work 结果的版本与采纳记录） | `src/frame/opendan/src/objects.rs`、`lib_opendan/src/runner/reports.rs` |

## 1. 现状核对

### 1.1 Agent 首页

| 事实 | 位置 |
| --- | --- |
| 首页左列依次是资料卡、工作概况、目标、已知工作区、Runtime；右列是 Session 树。没有 Memory、Skills 区块 | `pages/Home.tsx` 的 `HomePage` |
| 路由只有首页、`#/session/<sid>`、`#/goal/<goal_id>` 和三个观测页（`#/sessions`、`#/agent`、`#/loader`）。详情页用的是观测页的英文标签布局，不是首页的手机样式 | `web/src/App.tsx` |
| 所有 `useQuery` 缺省 4 秒轮询一次（`REFRESH_MS`） | `web/src/lib.ts` |
| 数据层是 `OpenDanDataModel` 接口，kRPC 与 mock 各一份实现；e2e 跑在 mock 上 | `web/src/model/{datamodel,krpc,mock,types}.ts`、`web/tests/e2e/` |
| 首页专用方法（`agent.profile`、`usage.models`、`ui.bindings`、`home.workspaces`、`skills.status`、`runtimes.settings*`）在 opendan 服务里直接实现，不经过 Agent State 门面 | `src/frame/opendan/src/home.rs`、`service.rs` 的 `call` |
| 每个方法都要归入 `is_read` 或 `is_operation`（或门面的 `is_write`），只读授权的 Agent App 按这三张表放行；`permission_tests` 枚举了全部方法 | `service.rs` |
| Session 的三个操作：`session.stop {sid, reason?, scope?}`、`session.decide`、`session.post`。没有 pause / resume / delete | `service.rs` |

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
| 首页 `RuntimeList` 每条是一张大卡片：连接方式、可用性、默认、基线、ComputerUse、授权根目录、用量、工具检查、扩展包、检查按钮、启停、设为默认、基线和 ComputerUse 下拉 | `web/src/pages/Workspaces.tsx` 的 `RuntimeCard` |
| 每张可用卡片的 `RuntimeExtPackages` 每 4 秒调一次 `runtimes.ext_tools`，节点 runtime 上会跑探测命令 | 同上 |
| settings 里没有 `host` 时，投影仍会加一条禁用的占位记录“Owner's desktop” | `protocol/runtime.rs` 的 `builtin_runtimes` |
| `default_runtime_id` 未设置或被停用时，`effective_default_runtime()` 落到私有 runtime，`private` 记录带 `default: true`；`runtimes.settings` 返回的也是解析后的值，界面分不出“没配置” | `app_schema.rs`；`home.rs` 的 `runtime_settings` |
| 停用一个 runtime 会把指向它的 `default_runtime_id` 清掉，重新启用不恢复 | `home.rs` 的 `set_runtime_settings`；`app_schema.rs` 的 `validate_runtimes` |
| Desktop Lite（原生模式）下，settings 里指向本机的条目是 `private` 的别名。界面不显示启停开关，但后端不拦：`runtimes.settings_set {runtime_id: "private", enabled: false}` 会解析到别名并停用它 | `home.rs` 的 `set_runtime_settings` |
| 从 settings 里移除的条目，其记录不会退役，停留在最后的状态 | `lib_opendan/src/state/runtimes.rs` 的 `ensure_builtin` |
| 节点 runtime 的分配走 control_panel：`agent.runtime_directory`、`agent.update {runtimes}`，只能建 `Node` 目标 | `src/frame/control_panel/src/{agent_runtimes.rs,agent_mgr.rs}` |
| Settings → Cluster 的 Add device 把选中的 Agent 写进执行器 profile 的 `principals`，不写 `AgentSettings.runtimes`。仓库里没有任何代码写 `AgentRuntimeTarget::Executor`，所以 daemon 设备不会出现在 Agent 的 runtime 列表里 | `control_panel/src/agent_daemon.rs`；`desktop/src/app/settings/pages/{ClusterManagerPage,AgentDevices}.tsx` |
| Desktop 没有打开 Settings 指定页面的深链；现成的做法是 `?agent_setup=…` 经 `desktopUIStore.openAppWindow` 带 launch payload | `desktop/src/desktop/DesktopRoute.tsx`、`desktop/src/app/settings/SettingsAppPanel.tsx`、`desktop/src/app/settings/components/layout/SettingsShell.tsx` |

### 1.4 Skills

| 事实 | 位置 |
| --- | --- |
| 只有两类：`SkillKind::Cmd`、`SkillKind::Goal`。没有 plan skill、`scope` 字段、停用状态 | `protocol/behavior.rs` |
| `SkillMeta.origin` 是 `package`（随 Agent 包同步）、`installed`（`.meta/skills.json` 有记录）、`local` | `skills.rs` 的 `list` |
| 已有 kRPC：`skills.list {kind?}`、`behaviors.cmd_skill {name}`（只读 cmd 手册正文）、`skills.install {package, replace?}`（要传解包后的文件表）、`skills.remove {kind, name}`（只能删 `installed`）、`skills.status {runtime_id?}`、`skills.install_plan {package, runtime_id?}` | `state/krpc.rs`、`home.rs` |
| `skills.status` 缺省对私有 runtime 现场跑一次检查，按 skill 返回每条命令是否存在、由谁提供、哪个扩展包能补上。只覆盖 front matter 写了 `commands:` 的手册 | `home.rs` 的 `skills_status`、`skills.rs` 的 `commands_status` |
| 内置手册里 `ui_core`、`buckyos_admin`、`share`、`xllm` 没写 `commands:`，所以查不到命令状态 | `src/apps/jarvis_agent/agent/cmd_skills/` |
| `goal_skills/*.md` 没有读取正文的 kRPC，运行时也没有任何路径读它 | `runner/goal.rs` |
| `.pikg` 的解析只接受文件系统路径（Rust `skills::read_pikg`、websdk `pikg_archive.ts`），浏览器里不能直接复用；没有 URL、文本导入，没有 LLM 扫描 | `skills.rs`；`../buckyos-websdk/cli/modules/skill.ts` |
| WebUI 和 Desktop 都没有 Skills 页面 | — |

### 1.5 工作区

| 事实 | 位置 |
| --- | --- |
| 区块标题“已知工作区”（i18n 键 `knownWorkspaces`，`data-testid="known-workspaces"`），默认全部展开，按 `updated_at_ms` 排序，每条是完整卡片 | `web/src/pages/Workspaces.tsx` 的 `WorkspaceList`、`WorkspaceCard` |
| 已有“创建”“导入”两个表单，字段齐全（runtime、目录、名称、描述、URL），一步提交 | 同上的 `WorkspaceForm` |
| `workspaces.create` / `workspaces.import` 提交时访问目标 runtime 上的目录，失败返回错误；没有“只检测不登记”的接口 | `lib_opendan/src/state/workspaces.rs` |
| 记录没有“最近使用时间”字段；首页已经加载了 Session 列表，可以按绑定该工作区的 Session 的最近更新时间推算 | `protocol/workspace.rs` 的 `WorkspaceRecord` |

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
| pause 之后 Session 不再推理：运行中的 run 在下一个推理边界停下，Turn 保持打开；新到的输入留在队列。**pause 会级联到所有未结束的子 Session**，resume 只恢复因这次级联暂停的那些 | `runner/inputs.rs` 的 `apply_control`、`cascade_pause` |
| 聊天里唯一注册的命令是 `/stop`，映射成 session 范围的 stop：UI Session 以 stopped 结束，下一条消息新开一代 UI Session，重新冻结提示词 | `ui.rs` 的 `bridge_ctx`；`lib_opendan/src/bridge/msg.rs` |
| 回答态的 Stop 按钮发的就是文本 `/stop`；TaskMgr 取消对非 Goal 的 Session 也是 session 范围的 stop | `ConversationComposer.tsx`；`tasks.rs` 的 `CancelBridge` |
| session 范围的 stop 会停掉该 Session 所有未结束的子 Session | `runner/drive.rs` 的 `stop_children` |
| turn 范围的 stop 只关当前 Turn，Session 继续等待。实现里有 `turn_stop && !site.child` 的条件；只有 Goal Session 的测试覆盖。UI Turn 停在 `analyze_intent` 子 context 里时的行为没有验证过 | `runner/outcome.rs`；`lib_opendan/tests/goal_session.rs` |
| 暂停的 Turn 在 TaskMgr 里仍是 Running 或 Waiting，只是 message 写成 `paused: …`。TaskMgr 有 `Paused` 状态，但 opendan 的 `TaskService` 没有对应方法 | `tasks.rs` 的 `update` |
| 一个 UI Turn 创建的 Work Session 卡片已有只读接口 `session.work_cards {sid, task_id}`：卡片状态含 `paused`，plan 卡带 Todo 列表，Todo 绑定的子 Session 带一层卡片（不再往下）。`task_id` 必填，没有“整个会话的全部未结束工作” | `work_card.rs`、`service.rs` |
| `session.read` 已返回 `config.frozen.{catalog_rev, frozen_at_ms}`，`behaviors.revision` 返回当前 catalog 的修订。修订是对 `role.md`、角色补充、`self.md`、全部 behaviors、i18n、全部 cmd_skills 的路径、大小、mtime 取哈希，任何一个文件变了都会变；`catalog_rev` 只记首次冻结 | `service.rs` 的 `read_session`；`state/behaviors.rs` |
| 提交状态时先写 `state.json` 再上报登记表，而 `session_for` 只看登记表。停止后紧接着发的消息可能被投给已结束的旧 Session（11-06 记录的既有竞态，实施前复核是否仍在） | `runner/shared.rs` 的 `commit_and_report`；`ui.rs` 的 `session_for` |
| 登记表没有删除 Session 的操作 | `lib_opendan/src/state/mod.rs` |

### 1.8 产物

| 事实 | 位置 |
| --- | --- |
| 报告的 `artifacts`、回复里的附件经 `ObjectStore` 存进 named store，绑进桶 `user/<agent_id>/<owner>/artifacts`，key 是 `<对象 id 前 16 位>/<名字>` | `opendan/src/objects.rs`、`runner/reports.rs` |
| 没有记录“哪个 Session、什么时候交付了哪个文件”的索引，也没有相应接口。桶本身可以 `list_objects`，但条目不带 Session | `buckyos-api/src/bucket_client.rs` |
| 首页工作区卡片里的“最近产物”读的是 `artifacts.list`（`ArtifactHead`），是另一个概念 | `Workspaces.tsx` |

## 2. 裁决记录

2026-11-09 owner 在文中逐条批注，整理如下。“按建议”指采纳起草时的建议，建议内容已写进“裁决”一栏。规划 §四 的待定项已并入。

| # | 问题 | 裁决 | 落到 |
| --- | --- | --- | --- |
| D1 | Memory “删除”的含义 | 按建议。接受逻辑删除：认知写一个 `deleted` 的新修订，历史保留；界面仍叫“删除”，确认框说明“Agent 不再使用它，历史修订保留”；owner 删除的认知，撤回提示行不再带原陈述。感知只有未整理的能删（处置为 `discarded` 并清正文）；已吸收的标“已整理为认知”，不提供删除，引导去删对应认知。owner 补充：影响大的是认知，用户可以手工删掉一条。所以认知删除是必做项，感知删除走同一个接口顺带提供 | ME2、ME4 |
| D2 | Memory 的可见范围、计数口径与首页预览 | owner：首页按“最近添加”看，最近 5 条认知加 3 条感知就够。认知在前。其余按建议：首页用不受限视图（服务已验证调用者是 owner），每条显示 `scope.subjects`；认知总数 = 状态 `active` 或 `disputed` 且不是 `salience`；感知总数 = 正文未清理的全部，另给“待整理”数 | ME1、ME3 |
| D3 | 停用 Owner 默认 runtime 后是否保留默认设置 | 按建议。保留 `default_runtime_id`，停用期间由 `effective_default_runtime()` 落到私有，重新启用后恢复；放宽 `validate_runtimes`，补调度器测试 | RT1 |
| D4 | 设备在哪里添加和分配，Agent 首页管什么 | 不按建议。owner：之前的规划更完整；Agent 首页主要站在 Agent 的视角讨论 enable / disable / delete。因此设备目录与分配以 [runtime TODO](opendan-agent-runtime-lifecycle-todo.md) 的两步流程为准（§0.6、§4.6，任务 E1–E3、E5、E7），本单不另做“最小子集”；本单只做 Agent 首页一侧：这个 Agent 的 runtime 列表，能启用、停用、删除 | RT1–RT4 |
| D5 | Skills 四组怎么得到 | 不在本单定。owner：Skills 体系升级有专门的 [skills TODO](opendan-skills-system-todo.md)，可能还没完成，界面可以先占位 | SK1 |
| D6 | 停用项的位置与计数口径（规划待定） | 按建议，作为界面要求移交 skills TODO（停用状态由它实现）：首页计数只算已启用的，停用数另写“· N 已停用”；Skills 页每组末尾有折叠的“已停用”小节，在原处重新启用；任何来源都能停用，只有 `installed` 能卸载；UI 组首版只读 | §4.4 移交清单 |
| D7 | 安装器与 LLM 扫描 | 归 skills TODO（owner：这也是那份文档里的内容）。规划对安装器的要求作为界面要求移交 | §4.4 移交清单 |
| D8 | 暂停当前消息是否暂停后台 Work | 不级联。owner：暂停当前消息，暂停的是正在处理这条输入的那个 Turn；它创建的 Work Session 都在后台运行，要暂停它们走暂停 Session 的流程 | SC1、MH4、MH5 |
| D9 | “停止当前消息”的含义 | 与 D8 同理，只作用于当前 Turn。owner：停止当前消息是友好停止，立刻完成 current turn。即不再发起新的推理和工具调用，当前 Turn 就此收尾，Session 和上下文都在，已创建的 Work 不受影响 | SC2、MH4 |
| D10 | 输入框何时禁用 | 按建议。只按“看得到的未结束占位”判定：最新一条带 `agent_task` 的 Agent 消息，其 task 运行中或已暂停时禁用；等待子任务、等待外部时不禁用；回答态优先，照常能答；占位出现之前不禁用；首次点击直接暂停 | MH4 |
| D11 | Clean History 的做法 | 按建议。(a) 新动作叫“开始新对话”，归档 Agent 侧会话，消息保留可见，时间线插一条分隔；MessageHub 原有的“清空聊天记录”不变。(b) 有未结束 Work 时拒绝，提示先停止或等待。(c) 新一代会话保持懒创建（下一条消息到来时才建）。(d) 不引入存储的默认会话指针，“归档”就是以 stopped 结束并在 worklog 记原因 | SC6、MH6 |
| D12 | 子会话树的范围与默认展开 | 按建议。只列当前这一代 UI Session 下未结束的 Work；默认展开到 Todo 列表，Todo 下的子会话默认收起；递归到 4 层为止 | SC5、MH3 |
| D13 | 近期产物 | 按建议。AgentRoot 里加只追加的交付索引；“当前会话”= 同一 `route_key` 的各代 UI Session 及其后代交付的；区块放在面板下部、子会话树之后，默认收起；“更多”菜单首版给下载、收藏、转发、定位到消息，“打开所在位置”等 Files 能按桶浏览再做；隐藏会话等内嵌 Chatbox 出现再接，索引先把会话信息记全 | AR1、MH8 |
| D14 | 提示词版本的比较与刷新 | 按建议。标签按内容比（冻结的身份文本、behaviors、cmd_skills 正文对 catalog 当前内容），由服务端给结论；压缩时刷新只对 UI / group 会话做，Work 不刷新；先核对压缩时系统前缀的重建方式再定实现 | SC7、MH7 |
| D15 | Session 删除的范围 | 按建议。只允许删已结束的 Work / self_check 会话，连同其全部已结束的后代；有未结束后代时拒绝；UI 会话和 Goal 会话不可删；接受用量统计随之减少，确认框写明 | SC8、HM3 |
| D16 | 统一侧面板的适用范围与入口 | 按建议。本轮只对“对方是自己名下 Agent”的会话启用新面板；桌面端 Info 按钮开关，记住上次状态，不默认展开；手机端保留点标题进入，另加左滑手势。owner 补充：群和联系人的侧栏也要优化，但不在这次任务中 | MH1 |

### 2.1 按裁决推定、待确认

下面四条是起草者按批注的意思推出来的，批注本身没有明说。任务已按推定写好；不同意的在这里批注，改动只涉及括号里的任务。

1. **`/stop`、回答态的 Stop 按钮、TaskMgr 取消也改成只停当前 Turn。** D9 的批注只说了“停止当前消息”。这三处在用户眼里也是“停掉这条消息”，今天却会结束整个 UI Session 并停掉它的 Work，所以按同一原则处理。（SC2、MH4）
2. **tunnel 渠道怎么“开始新对话”。** 上一条落地后，Telegram 等渠道的用户没有办法清空上下文。Telegram 菜单里已经登记了没有处理函数的 `new`、`clean`，可以把其中一个接到 SC6。本单暂未列任务。
3. **D4 的“删除”是“从这个 Agent 移除”。** 移除后记录转为 `retired` 留作历史（runtime TODO 裁决 21），设备本身还在用户级目录里，可以再分配。不是销毁设备。（RT4）
4. **D5 的“先占位”做到只读为止。** 首页有 Skills 入口，进去能看已安装的手册、正文和依赖命令的可用状态，全部用现有接口；分组、停用、卸载、安装的位置摆好但不可用。如果占位只要一张卡片，SK1 可以再砍。（SK1）

## 3. 接口变更一览

供前后端并行时对齐。名称可在实施时微调，改了要回写本表。除注明外都在 opendan 服务（`service.rs` + `home.rs` 或新模块），并归入 `is_read` / `is_operation` 与 `permission_tests`。

| 方法 | 读写 | 形状 | 任务 |
| --- | --- | --- | --- |
| `home.summary` | 读 | `{sessions: {working, waiting, failed, total}, memory: {perceptions, pending, cognitions, last_consolidation_at_ms}, goals: {active}, skills: {cmd, goal}, runtimes: {enabled, unavailable}}` | HM2 |
| `memory.overview` | 读 | `{cognitions: {current}, perceptions: {total, pending}, last_consolidation_at_ms, maintenance_goal_id, recent: {cognitions: MemoryEntry[], perceptions: MemoryEntry[]}}`；`recent` 是最近添加的 5 条认知和 3 条感知 | ME1 |
| `memory.list {layer, limit, cursor?}` | 读 | `{entries: MemoryEntry[], next_cursor?}`，按添加时间倒序；`MemoryEntry = {layer, reference, summary, kind, state, at_ms, revised_at_ms?, subjects, session_id?, deletable, absorbed_into?}` | ME1 |
| `memory.read {reference}` | 读 | 认知带来源视图，感知带原记录；不要 Session | ME1 |
| `memory.forget {reference, expected_revision?, reason?}` | 操作 | 认知 → 新修订 `deleted`；感知 → `discarded` 并清理。整理 Goal 正在运行时返回 busy | ME2 |
| `runtimes.settings` | 读 | 增加 `configured_default_runtime_id: string \| null`（原始设置值） | RT1 |
| `runtimes.settings_set` | 操作 | 停用不再清默认；拒绝停用私有 runtime 的别名 | RT1 |
| `runtimes.remove {runtime_id}` | 操作 | 从这个 Agent 移除一个已分配的 runtime，记录转 `retired`；拒绝私有及其别名。返回新的设置 | RT1、RT4 |
| `session.pause {sid, reason?, cascade?}` / `session.resume {sid, reason?}` | 操作 | 返回 `{index}`，同 `session.stop`。`cascade` 缺省 true；暂停当前消息时传 false | SC1 |
| `ui.conversation {peer_did, session_id?}` | 读 | `{current: UiSessionView \| null, generations: [{session_id, created_at_ms, finished_at_ms?, outcome?}]}`；`UiSessionView = {session_id, run_state, paused, needs_input, one_line_status, open_turn?: {turn, task_id}, unfinished_works, prompt: {frozen_at_ms, current: bool, changed: string[]}}` | SC4、SC7 |
| `session.work_tree {sid, unfinished_only?}` | 读 | `{agent_did, session_id, complete, sessions: WorkNode[]}`；`WorkNode` 是 `WorkCard` 加 `children: WorkNode[]`，Todo 绑定的卡片同样递归 | SC5 |
| `ui.rotate {sid}` | 操作 | 归档当前 UI Session。有未结束 Work 时返回错误并带数量；成功返回 `{archived: sid}`，返回时登记表已显示结束 | SC6 |
| `sessions.delete {sid}` | 写 | 返回被删的 sid 列表（含后代） | SC8 |
| `deliveries.recent {limit, before?, route_key?}` | 读 | `{entries: [{at_ms, name, obj_id, url, mime?, size?, session_id, ui_session_id?, route_key?, source: report \| reply}], next_before?}` | AR1 |
| Desktop 深链 `?settings=<page>` | — | 打开 Settings 并定位到指定页面 | RT3 |

Skills 的接口（分组、停用、安装计划、读 goal skill 正文）由 skills TODO 定义，设备目录与分配的接口由 runtime TODO 定义，本表不列。

## 4. 任务

每项写目标、落点、要点、验收。

### 4.1 首页骨架（HM）

- [ ] **HM1 详情页外壳与路由。** `App.tsx` 增加首页样式的详情路由：`#/memory`、`#/runtime/<id>`、`#/skills`、`#/skill/<kind>/<name>`、`#/workspaces`。这些页面用首页的样式（`home` 类、手机优先、中英文走 `i18n.ts`），顶部一个返回首页的链接；不要用观测页的标签布局。抽一个共用外壳组件。验收：手机宽度下各页不横向滚动；刷新保持在当前页。
- [ ] **HM2 `home.summary`。** `home.rs` 新方法，一次返回 §3 的摘要，供首页概况和 MessageHub 面板上部共用。Memory 部分复用 ME1 的缓存；Session 计数的口径与 `Home.tsx` 的 `isWorking` / `isWaiting` / `isFailed` 一致（把判定挪到服务端或保持两边同一份定义，二选一并写测试）。归入 `is_read`。
- [ ] **HM3 首页布局调整与 Session 删除入口。**（D15）左列顺序：资料卡、工作概况、Memory、目标、Skills（SK1 的占位卡片）、工作区、Runtime。`SessionDetail.tsx` 对可删除的会话显示“删除”按钮，二次确认，调用 SC8。验收：未结束的会话、UI 会话没有删除按钮；删除后回到首页且列表不再有它。

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

本节只做 Agent 首页这一侧（D4）。设备怎么添加、怎么分配给 Agent，按 runtime TODO 的两步流程实施（用户级目录 `agent.runtime_catalog`、daemon 设备两步化、ssh 上提、Desktop 目录页、CLI，即 R-E1–R-E4、R-E8，任务 E1–E3、E5、E7），本单不重复设计。规划里“设备从 Settings → Cluster 管理添加”一句，以那份文档 E5 的目录页（Settings → Agent Runtimes，合并 Cluster Manager 的 Add device）为准。本节的 RT2 是那份文档 R-E6 / E6 的细化：按规划改成“瘦列表 + 详情页”。

- [ ] **RT1 设置语义修正与移除接口。**（D3、D4）
  - `runtimes.settings` 增加原始的 `configured_default_runtime_id`。
  - 停用不再清 `default_runtime_id`：改 `home.rs` 的 `set_runtime_settings`、`app_schema.rs` 的 `validate_runtimes`、control_panel `agent_runtimes.rs` 的 `apply_update`；核对调度器 `native_agent_node` 在默认被停用时的结果并补测试。
  - 拒绝停用私有 runtime 及其别名（Desktop Lite）。
  - settings 里已移除的条目，其记录转为 `retired`（`state/runtimes.rs` 的 `ensure_builtin`），保留 `usage`。
  - 新增 `runtimes.remove {runtime_id}`：拒绝私有及其别名；删掉 `settings.runtimes[id]`，`default_runtime_id` 指向它时一并清空；随后立即重新投影（同 `runtimes.settings_set` 的做法）。归入 `is_operation`。
  - opendan 这条路径改完 settings 不会触发调度器刷新（control_panel 的 `agent.update` 会），节点上给这个 Agent 的执行权限要等调度器下一轮才收回。实施时量一下停用和移除后的延迟，记入 §8；不可接受就补上刷新。
  - `home.rs` 的 runtime 设置今天没有测试，本项补上：停用再启用后默认仍在；停用别名被拒；移除后记录退役、默认清空；移除私有被拒。
- [ ] **RT2 首页列表瘦身与详情页。** 依赖 RT1、HM1。
  - 首页每行只有名称、一行说明、启停开关。分三类显示：Provider（`connector.type == 'native'`，无开关，标“始终启用”）、Owner 默认（`runtime_id == configured_default_runtime_id`）、其他已分配（settings 里有条目）。settings 里没有条目的 `host` 占位记录不显示。Lite 下私有 runtime 同时是 Owner 的机器，只显示一行并加标注。
  - 列表末尾是折叠的“历史”：`retired` 的记录，显示名称、最后使用时间和用过它的 Session 数，点进详情只读。
  - `#/runtime/<id>` 承接 `RuntimeCard` 现有的全部内容和操作（检查、设为默认、基线、ComputerUse、扩展包、工具检查、用量与最近 Session）。`runtimes.ext_tools` 只在详情页请求。
  - 停用是可逆的，开关直接生效，不弹确认（runtime TODO R-D1 要求停用前给影响清单，等它的 R7）。
  - 重写 `tests/e2e/home.spec.ts` 里断言 runtime 卡片的两条用例和 `mock.ts` 的对应数据。验收对应规划场景“Owner 没有 Default Runtime”：只见 Provider 一行；停用后条目仍在、可重新启用，默认标记不丢。
- [ ] **RT3 “添加 Runtime”引导与深链。** 首页 Runtime 区块的“添加 Runtime”打开说明：设备在 Desktop 的 runtime 目录里添加，并在那里分配给 Agent；这里只管这个 Agent 用不用它。按钮跳到 `${profile.desktop_url}/?settings=<page>`（在桌面窗口里要跳出 frame，做法同 `messageHubHref` 的 `outerTarget`）；Zone 外没有 `desktop_url` 时只显示说明。目标页面是 runtime TODO E5 的目录页；它落地前先指向 Settings → Cluster（Add device 现在在那里）。Desktop 侧在 `DesktopRoute.tsx` 处理 `?settings=<page>`，仿照 `?agent_setup`；`SettingsAppPanel` 从 launch payload 取初始页面传给 `SettingsShell`（它今天把当前页存在本地 state 里，没有初始页入参）。验收：从 Agent 首页点击后 Desktop 打开 Settings 并停在目标页。
- [ ] **RT4 从 Agent 移除 runtime。**（D4，§2.1 第 3 条）依赖 RT1、RT2。
  - 详情页对 Owner 默认和其他已分配的 runtime 提供“删除”，Provider 没有。确认框写明：只是不再让这个 Agent 使用它，设备还在目录里，可以再分配；已经绑定它的进行中 Session 不会迁移（现状，runtime TODO R-D2 未做）。
  - 确认框列出影响：绑定在它上面的工作区（现有的 `workspaces.runtime_impact`）、用过它的 Session 数和最近一个（记录的 `usage`）。完整的影响清单等 runtime TODO R7 的 `runtimes.impact`。
  - 删除后该条进入“历史”。
  - 界面上的删除入口等 runtime TODO E5 的目录页可用后再放出：在那之前删掉了没有界面能加回（只能手工调 `agent.update`）。后端和 mock 上的用例可以先做。
  - 验收：删除后首页当前列表不再有它，“历史”里有；新建 Work 指定该 runtime 被拒；目录页重新分配后回到当前列表。

### 4.4 Skills（SK）

Skills 的分组、停用、安装器都归 skills TODO（D5–D7），本单只做占位页，不新增后端。

- [ ] **SK1 占位入口与只读页。**（D5，§2.1 第 4 条）依赖 HM1。只用现有接口：`skills.list`、`behaviors.cmd_skill`、`skills.status`。
  - 首页卡片“Skills 管理”：显示已安装的 CMD Skill 数和 Goal Skill 数，点击进入 `#/skills`。四组的分组计数等 skills TODO。
  - `#/skills`：页面按 UI / Work / Plan / Goal 四个版位排。分组字段到位之前，UI 和 Work 两个版位合成一个“CMD Skills”列表（现有全部 cmd 手册），Goal 版位列 goal skill，Plan 版位显示“暂无”。每行是名称、说明、来源、是否就绪。就绪状态来自 `skills.status`（会现场跑检查，属于操作类）：进入页面取一次，不轮询，提供“重新检查”。
  - `#/skill/cmd/<name>`：手册正文；依赖命令逐条显示可用、缺失、由谁提供，缺失时显示能补上它的扩展包；适用基线与平台；安装记录。没写 `commands:` 的手册显示“未声明依赖命令”，不要显示成“就绪”。Goal skill 今天没有读正文的接口，详情只显示名称和说明。
  - “安装 Skill”“停用”“卸载”的入口按规划的位置摆好，置灰并注明“随 Skills 体系升级提供”。
  - 页面组件按 `group` 和 `enabled` 两个字段渲染，今天由前端填缺省值；skills TODO 的接口到位后只换数据来源。
  - 验收对应规划场景“安装了 Skill，Provider Runtime 缺少依赖命令”：用 CLI 装一个声明了不存在命令的包，列表里显示已安装，详情指出缺的是哪条命令。

**移交给 skills TODO 的界面要求。** 来自规划 §3 和裁决 D6、D7，由 DX4 回写到那份文档（主要落在它的 S14，以及 S01、S02–S04、S06、S12、S13、S18）：

1. 四组与计数：首页按 UI / Work / Plan / Goal 显示各组已启用的数量。UI Skills 系统内置、首版只读；Work Skills 在创建 Work 时可选；Plan Skills 在 Plan 阶段加载，以查看为主；Goal Skills 保留入口，与 Goal 模块重构衔接。
2. 停用与卸载（D6）：停用保留安装记录并从日常使用列表隐藏，卸载解除安装。首页计数只算已启用的，停用数另写“· N 已停用”；Skills 页每组末尾有折叠的“已停用”小节，在原处重新启用；任何来源都能停用，只有 `installed` 能卸载。
3. 安装器（D7）：一个安装对话框，Package、URL、粘贴文本三个入口共用一套 LLM 扫描（识别用途、可拆出的 CMD Skill、命令依赖与风险），owner 确认后安装。首页卡片和 Skills 页都从这一个对话框进。浏览器里不能复用按路径解析 `.pikg` 的代码，服务端要有按字节解析的入口。
4. 详情：安装与就绪分开显示，依赖命令是否可用以 Provider Runtime（私有 runtime）为准。需要 goal skill 的读正文接口；`ui_core`、`buckyos_admin`、`share`、`xllm` 四份内置手册没写 `commands:`，补上后才能显示命令可用性。

### 4.5 工作区（WS）

- [ ] **WS1 区块改版。** 标题改“工作区”（i18n 键和 `data-testid` 一并改名，更新 e2e）。默认收起，收起时显示最近使用的 5 张小卡片：标题、Runtime 名称、目录、Remote URL（有才显示）、描述（一行）。“最近使用”取绑定该工作区的 Session 的最近更新时间，没有 Session 的用记录的 `updated_at_ms`。Runtime 和目录始终显示，用来区分同名工作区。“查看全部”去 `#/workspaces`，那里保留现有完整卡片（检查、改 URL、迁移）。预览数量是规划的待定项，先定 5 并做成常量。
- [ ] **WS2 手工添加向导。** 把“创建”“导入”两个表单合成一个分步向导：第 1 步选 Runtime，写明“这一步决定 Agent 在哪台机器上访问目录”，只列可用且允许工作区的 runtime；第 2 步选“已有目录”或“新建目录”并填目录、Remote URL、标题、描述；第 3 步提交并显示检测结果。提交仍用 `workspaces.import` / `workspaces.create`，失败时停在第 3 步显示原因并可返回修改。实施时确认导入失败不会留下登记，并记录目录里是否会留下元数据文件。检测细则等工作区专项文档，本项不新增检测逻辑。验收对应规划场景“用户在指定 Runtime 中手工添加工作区”。

### 4.6 会话控制后端（SC）

- [ ] **SC1 pause / resume 接口与不级联选项。**（D8）暂停当前消息 = 对 UI Session 发不级联的 pause：只挡住正在处理这条输入的 Turn，它建出的 Work 在后台照常运行。Work 的暂停走现有的 Session 暂停流程（级联到它自己的子会话，行为不变）。`ControlCommand::Pause` 增加是否级联（缺省级联）；`apply_control` 据此决定是否调 `cascade_pause`。`service.rs` 增加 `session.pause` / `session.resume`，归入 `is_operation`。同步 `doc/opendan/protocol/Session Input Protocol.md` 里 pause 和“pause 级联”两段。验收：对有未结束子 Work 的 UI Session 发不级联的 pause，子 Work 的 `paused` 仍为空；resume 后 Turn 从检查点继续；缺省 pause 的现有测试不变。
- [ ] **SC2 UI Session 的 turn 范围停止。**（D9）
  - 目标行为是友好停止：收到后不再发起新的推理和工具调用，在飞的调用返回后当前 Turn 立刻收尾并以 stopped 关闭，占位改成“已停止”，Session 回到等待，上下文保留。
  - 先写测试确认现状：UI Turn 在 `analyze_intent` 子 context 里收到 turn 范围 stop 时，Turn 是否以 stopped 关闭、Session 是否回到等待、占位是否被改成“已停止”文案（Agent 包 `i18n` 的 `[outbound] stopped`）。按结果修 `runner/outcome.rs` 的 `turn_stop && !site.child` 分支。
  - （§2.1 第 1 条，待确认）`/stop`（`bridge/msg.rs`）和 `CancelBridge`（`tasks.rs`）对 UI Session 改用 turn 范围；没有打开的 Turn 时 `/stop` 不生效并回一句提示。更新 `opendan/tests/loader.rs` 相关用例和 `ui_core` 手册里对 `/stop` 的说明。
  - 验收：停止一条正在处理的消息后，同一个 UI Session 接着处理下一条消息，上下文还在；这个 Turn 之前已创建的 Work 不受影响。
- [ ] **SC3 暂停状态进 TaskMgr。** `TaskService` 增加暂停上报，`TurnTasks::update` 在 Session 暂停时把 Turn task 置为 TaskMgr 的 `Paused`，恢复后回到 Running。TaskMgr 一侧发起的 Pause / Resume 请求不桥接（Task Center 上不出现这两个按钮，或出现但不生效，实施时确认并记录）。验收：暂停后 `getTask` 的状态是 Paused，Desktop 的 `taskNodeState` 得到 `paused`。
- [ ] **SC4 `ui.conversation`。** 给定对话方和 MessageHub 的会话 id，返回当前这一代 UI Session 的状态和历代列表（匹配规则：`outbound` 的 `to` 等于对话方；`to_session` 等于会话 id，或会话 id 是 `dm:` 开头且 `to_session` 为空）。当前一代取未结束的最新一条。归入 `is_read`。这是面板下部、提示词标签、Clean History 入口共用的一次读取。
- [ ] **SC5 `session.work_tree`。**（D12）在 `work_card.rs` 增加按 Session 汇总的树：该 Session 的全部子 Work（可只取未结束的），plan 的 Todo 绑定卡片递归，非 Todo 的子 Work 挂在 `children`，深度上限 4。`complete` 的含义同 `work_cards`。现有 `session.work_cards` 不变。验收：两个并列 Work、一个 plan 带三个 Todo、其中一个 Todo 的会话又建了子 Work 的夹具，结构与预期一致；全部结束后 `unfinished_only` 返回空且 `complete` 为真。
- [ ] **SC6 归档当前 UI Session（Clean History）。**（D11）
  - `ui.rotate {sid}`：确认是本进程驱动的 UI Session；有未结束子 Work 时拒绝并返回数量；否则投递 session 范围 stop（原因标明是 owner 归档），等登记表显示结束后返回（有上限，超时返回进行中）。
  - 修既有竞态：`session_for` 选当前会话时，对登记表里“未结束”的候选再看一眼 `state.json`，已请求停止或已结束的不再投递；该 inbox 的记录留到旧会话结束后由新一代接收。对应用例 `inbox_messages_are_answered_along_the_same_conversation` 的偶发超时。
  - 旧会话的目录、worklog、Session id 都保留，Memory 里的原文引用继续可用。
  - 验收对应规划场景“Clean History 后回查旧 Memory”：归档后下一条消息由新的 Session id 处理，`session.read` 旧 sid 仍能读到 worklog；归档后立刻发消息不丢。
- [ ] **SC7 提示词版本状态与压缩时刷新。**（D14）
  - 第一步（只读）：在 `state/behaviors.rs` 加比较函数，把 `prompt.frozen` 的身份文本、已冻结 behaviors、cmd_skills 与 catalog 当前内容逐项比较，返回是否一致和不一致的项目名；`ui.conversation` 的 `prompt` 用它。
  - 第二步：读 `runner/live.rs` 的上下文重写和 `runner/history.rs`，确认压缩后系统前缀是怎么重建的，把结论写回本单 §8，再实现：UI / group 会话在压缩提交点重新冻结（替换 `prompt.frozen`，worklog 记 `control_applied`），Work 会话不动。重新冻结后 cmd_skill 的可用性按冻结规则重算。
  - 验收对应规划场景“Agent root fs 的系统提示词更新”：改 `role.md` 后旧会话 `prompt.current` 为假、新会话为真；旧会话发生一次压缩后变为真，且压缩前后的对话不重复不丢。
- [ ] **SC8 删除 Session。**（D15）登记表增加删除：只接受已结束的 Work / self_check，后代全部已结束才允许，连同后代一起删除目录和登记项；`state/perception/<sid>.jsonl` 不动（Memory 的来源）。门面增加 `sessions.delete` 并归入 `is_write`。验收：有未结束后代时拒绝；UI、Goal 会话拒绝；删除后 `sessions.query`、`session.read` 都查不到。

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
  - （§2.1 第 1 条，待确认）回答态的 Stop 按钮改调 turn 范围的停止，不再发 `/stop` 文本。
  - 把 `MessageTaskArea` 第一行的英文实时状态（`working (analyze_intent)` 等）本地化，复用 `worksessions/model.ts` 的 `localizeLine`。
  - 验收对应规划场景“用户暂停正在分析的消息”：暂停后不能发送；恢复后继续；停止后可发送且上下文还在；Turn 进入等待子任务后可发送。deno 数据模型测试覆盖状态函数的全部分支。
- [ ] **MH5 子会话树节点控制。**（D8）依赖 MH3、SC1。后台 Work 的暂停走 Session 的暂停流程：未结束的节点上有“暂停”，已暂停的有“恢复”“停止”，调 `session.pause`（缺省级联，暂停 plan 就暂停它的 Todo 会话）/ `session.resume` / `session.stop`。停止要二次确认。直接对 Todo 的会话发控制不会写 plan 的 `TodoAttempt` 暂停标记（那是 `agent-session todo pause` 写的），卡片状态以会话自己的 `paused` 为准；验收时确认 plan 的下一次 tick 不把它当异常。已停止的节点上放一个“在 Agent 首页查看”的链接，删除在那边做。
- [ ] **MH6 Clean History 入口。**（D11）依赖 SC6。面板下部的会话控制区加“开始新对话”，确认框说明：Agent 从全新上下文开始，旧会话归档可查，消息记录不删除。有未结束 Work 时按钮禁用并说明原因。成功后在时间线插入一条本地分隔标记（按归档时间定位，取自 `ui.conversation` 的历代列表，刷新后仍在）。头部 ⋮ 菜单的“清空聊天记录”保持原样，两处文案要能让人分清。调用期间禁用输入框，返回后恢复。
- [ ] **MH7 提示词版本标签。** 依赖 SC7 第一步。面板下部显示“与 Agent 当前版本一致”或“Agent 版本已更新”，后者的提示里写“开始新对话或下次历史压缩后生效”，并列出变化的项目名。标为高级信息，放在折叠区。
- [ ] **MH8 近期产物区块。**（D13）依赖 AR1。默认收起，展开后“全部”“当前会话”两个页签，按时间倒序，滚动加载。每条显示缩略图或类型图标、名字、时间、来源会话。“更多”菜单：下载、收藏、转发（按单个文件构造转发内容，复用 `store.forward` 和 `ForwardDialog`）、定位到消息（仅当前会话且消息已加载或可加载时）。预览复用现有的媒体预览。对象读取走哪条访问路径（msg-center 的 ObjectAccess 需要消息上下文，“全部”页签里别的会话的产物没有）在实施第一步确认并写回 §8。

### 4.9 文档与手册（DX）

- [ ] **DX1 协议文档。** `doc/opendan/protocol/Session Input Protocol.md`（pause 不级联、UI Session 的 stop 范围）、`Agent State Protocol.md`（交付索引、Session 删除、runtime 移除与退役）、`src/frame/opendan/README.md`（新方法、`/stop` 的新含义、产物索引）。
- [ ] **DX2 界面文档。** `src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md`（新面板、前台消息状态、子会话树与产物的数据通路；顺带更正 §4.4 关于运行状态键由 OpenDAN 写入的说法，后端没有写入方）、`product/message_hub/MessageHub_Web_UI_PRD.md` §12、`product/message_hub/MessageHub_Current_UI_Model_Data.md`。
- [ ] **DX3 cmd_skills 联动。** 按 `AGENTS.md` 的表逐项核对并在提交说明里写结论：`ui_core`（`/stop` 语义、暂停）、`opendan_agent_dev`（Session 行为、新增的服务方法）。本单不改 skill 的安装 kRPC 和 AgentRoot 布局。
- [ ] **DX4 回写关联 TODO。** 这一项只改文档，可以最先做。
  - skills TODO：把 §4.4 的“移交给 skills TODO 的界面要求”四条写进去（主要是 S14），注明 SK1 的占位页已按 `group` / `enabled` 字段预留。
  - runtime TODO：在 R-E6 / E6 注明 Agent 首页的形态按规划改为“瘦列表（三类）+ 详情页 + 历史”，由本单 RT1–RT4 实施；在 E5 注明 Agent 首页“添加 Runtime”会深链到目录页；在 R-D1 / R-D3 注明 Agent 视角的“删除”是 `runtimes.remove`。

## 5. 顺序与并行

裁决已定（§2）。§2.1 的四条推定各自只影响括号里的任务，动到那些任务前先看有没有新的批注。

1. **后端先行，三条线互不相干，可并行：** Memory（ME1 → ME2）；会话控制（SC1 → SC2 → SC3，SC4、SC5 可同时做，SC6、SC7、SC8 在其后）；RT1 与 AR1。
2. **首页：** HM1 最先；ME3、ME4、RT2、SK1、WS1、WS2 在各自依赖完成后并行；RT3 随时可做。它们都改 `Home.tsx`、`i18n.ts`、`model/*`，分给多个 Agent 时要约定合并顺序，或由一个 Agent 串行做。
3. **MessageHub：** MH1 最先；MH2、MH3、MH4 并行；MH5、MH6、MH7、MH8 跟在各自依赖后。它们都改 `MessageHubView.tsx`、`messagehub.css`、`i18n/messagehub.ts`，同样注意合并。
4. **依赖另外两份 TODO 的部分：** RT4 的界面入口等 runtime TODO E5；设备目录与分配是 runtime TODO E1–E3、E5；Skills 的分组、停用、安装器是 skills TODO。这些落地之前，对应的规划验收场景只能验到本单这一侧（§6.4）。

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

以下是 11-06 至 11-09 的记录，动手前先在未改动的工作树上复核一遍，不要把它们算成回归：

- Desktop e2e：`messagehub-agent-task.spec.ts:575`、`messagehub-group.spec.ts:309`、`messagehub-media.spec.ts:173`、`messagehub.spec.ts` 的 “attachments over the sendable size”；并行时 `messagehub-actions.spec.ts:87`、`my-network.spec.ts:350` 偶发失败，串行通过。
- Desktop deno：`messagehub-guest.test.ts` 有 7 条失败；`eslint .` 有 5 个 error 在 ai-center、homestation、settings。
- libopendan：`a_session_token_reaches_only_its_own_session_methods`（`tests/session_runtime.rs`）。
- opendan（`tests/loader.rs`）：`a_session_process_elsewhere_reaches_its_session_through_the_service`；`inbox_messages_are_answered_along_the_same_conversation` 偶发超时（SC6 要修的竞态）。
- `cargo test` 遇到失败就停，全量回归加 `--no-fail-fast`。

### 6.3 真实环境

- OpenDAN 在 paios 容器里，后端和 WebUI 的改动要 `./build_aios --local-test` 再 `uv run src/start.py --skip-update` 才生效；也可以用 `./debug_jarvis.sh devtest --agent <agent_id>` 在宿主机上托管单个 Agent。
- Desktop 对真实 Zone 的调试：`VITE_CP_USE_MOCK=false VITE_ZONE_PROXY=https://test.buckyos.io VITE_ZONE_PROXY_IP=127.0.0.1 npx vite --host 127.0.0.1 --port 4175 --strictPort`，再 `MESSAGEHUB_REAL_E2E=1 npx playwright test --config=playwright.real.config.ts`。`playwright.real.config.ts` 注释里的环境变量已过期。真实用例会给 Agent 发消息，消耗 token。
- 未在真实 Zone 上验过的项目要在 §8 如实写明，不能用 mock 通过代替。

### 6.4 规划验收场景对照

| 规划 §三 的场景 | 任务 | 验证方式 |
| --- | --- | --- |
| Owner 没有 Default Runtime | RT1、RT2 | WebUI e2e（mock）；真实 Zone 上新建未选 host 的 Agent 查看首页 |
| 新设备未分配，随后分配给 Agent | runtime TODO E1–E3、E5；本单 RT1、RT2、RT4 | 目录页落地后在真实 Zone 走全流程：添加设备 → 分配 → 首页出现 → 停用 → 重新启用 → 删除进历史。之前只能用 `agent.update` 手工分配，验首页这一侧 |
| 安装了 Skill，Provider Runtime 缺少依赖命令 | SK1（安装器本身在 skills TODO） | 用 CLI 安装 `test/skills_pikg_sample/` 改出的、声明了一条不存在命令的包，在占位页详情里看到缺的是哪条 |
| 手工添加工作区 | WS2 | WebUI e2e；真实 Zone 上对私有 runtime 的一个已有目录走一遍 |
| 一个聊天会话创建多个后台工作 | SC5、MH3 | Desktop e2e（mock 会话 `session-coder-work` 扩充）；真实 Zone 发一个会建 plan 的请求 |
| 用户暂停正在分析的消息 | SC1–SC3、MH4 | Desktop e2e 与 deno 状态函数测试；真实 Zone 走暂停、恢复、停止三条路径 |
| 内嵌 Chatbox 会话完成一个产物 | AR1、MH8 | 只能验“全部”与“当前会话”的归属；隐藏会话不存在，这一条标为未覆盖 |
| Clean History 后回查旧 Memory | SC6、MH6 | opendan 集成测试；真实 Zone 归档后用 `agent-session recall --read` 读旧引用 |
| Agent root fs 的系统提示词更新 | SC7、MH7 | libopendan 测试；真实 Zone 改 `role.md` 后看标签，触发一次压缩再看 |

## 7. 不做与风险

**不做：**

- 目标管理界面（规划 §5，等 Goal 模块重构）。
- Memory 的手工添加和编辑（规划首版范围外）。
- Skills 的分组、停用、卸载、安装器与 LLM 扫描，以及 Plan Skills 的格式、选择闭环、self-improve：全部在 skills TODO（D5–D7）。本单只有占位页 SK1。
- 设备的添加与分配：用户级 runtime 目录、daemon 设备两步化、ssh 登记上提、Desktop 目录页、CLI，全部在 runtime TODO（D4）。
- 群和联系人会话的侧栏改版（D16，owner 已说明需要优化，另行安排）。
- 隐藏会话与内嵌 Chatbox。
- TaskMgr 一侧发起的暂停和恢复。

**风险：**

- **停止语义变更影响面大（D9，§2.1 第 1、2 条）。** `/stop` 改成只停当前 Turn 之后，Telegram 等 tunnel 的用户没有办法清空上下文，直到给它们接上“开始新对话”。两件事最好同一批上线。
- **“开始新对话”被拒绝的情形可能很常见（D11）。** 长期跑着后台 Work 的 Agent 会一直点不了。实际使用中如果是这样，要回头考虑把未结束的 Work 改挂到新会话。
- **压缩时刷新提示词（SC7 第二步）没有核对过可行性。** 压缩发生在 run 内，系统前缀的重建方式本单没有读透；如果代价过大，标签仍可先上线，刷新另起一单。
- **Memory 列表的开销。** 感知按会话分文件，全量扫描随会话数增长；ME1 的缓存和分页是必须的，不是优化。
- **两个验收场景要等别的 TODO 才能走通。** “新设备未分配，随后分配给 Agent”依赖 runtime TODO 的目录页；Skills 的四组、停用、安装器依赖 skills TODO。那两份文档的排期决定规划整体何时验收。
- **首页与 MessageHub 两处的任务共用文件多**（`Home.tsx`、`i18n.ts`、`model/*`；`MessageHubView.tsx`、`messagehub.css`）。并行实施时冲突概率高，见 §5。
- **顺带发现、不在范围内：** `web/tests/real/smoke.spec.ts` 仍期待已不存在的 `perception.cursor`、`perception.backlog`；`service.rs` 的 `is_read` 里也还列着它们。Telegram tunnel 注册了 new、clean、cancel 等菜单命令但没有处理。MessageHub 头部和会话列表的运行状态行读的 `typing` / `active` / `status_line` 没有后端写入方，真实环境里恒为空；本单的面板不依赖它，是否让 OpenDAN 写这些键或撤掉这一行另行决定。

## 8. 实施记录

（实施后填写：落点、与本单不同的决定、测试结果、未验证项。）
