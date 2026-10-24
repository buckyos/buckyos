# Agent Runtime 集成 Session / Workspace 实施 TODO

日期：2026-10-23

状态：2026-10-23 T1–T6 代码已实施（未提交、未 DV），实施记录见文末“实施进度”。基线核对到工作区 `0c0675962`（agent-daemon 已有 macOS 14+ 与 Windows 当前用户会话的桌面 Driver）。

## 0. 开工评审裁决（2026-10-23）

开工前 review 发现四项设计缺口，已拍板如下；本文后续各节按此修订，与此冲突的旧表述以本节为准。

| # | 缺口 | 裁决 |
| --- | --- | --- |
| R1 | `node_executor/src/lib.rs::open_internal` 只在 `identity_mode == CurrentUser` 时开放 ComputerInspect/Observe/Act；Unix 上 node-daemon 默认 SwitchUser，macOS 安装为 root LaunchDaemon；桌面 Driver 目前只在 agent-daemon 进程里 | **Desktop 的 node-daemon 以“随用户登录、运行在交互会话、CurrentUser 身份”的桌面模式运行。** Windows 已是这种形态（HKCU Run 启动、CurrentUser 默认）；macOS 改为同样形态（LaunchAgent/登录项），**安装包改动不在本次范围**，本次只做 node-daemon 代码侧的桌面模式与 Driver 注入；Linux 无 GUI，不做 ComputerUse，继续形态 A 的 run_as 降权语义。桌面 Driver 从 agent-daemon 抽成共享实现供 node-daemon 复用，不要求安装独立 agent-daemon |
| R2 | 形态 A 的 `run_as` 专用账号读不到 owner 的 home/TCC 目录，与“操作 owner 的文件”不匹配 | 同 R1：桌面模式下 profile 不带 `run_as`，以 owner 登录身份执行，权限边界与 node_exector.md §16 对 Windows 的说明一致（Agent 拥有该桌面用户全部权限，roots 只约束 cwd 与内置文件工具）。Linux 服务器/OOD 保持 run_as |
| R3 | `ExecutorOp` 只有 Exec/Read/Write/Edit，`FileBackend` 没有 list/stat/mkdir/rename；`state/workspaces.rs` 的 discover/check/relocate/metadata 原子写依赖这些本地 fs 操作 | **Workspace 管理留在 Agent State（控制侧）；runtime 侧只是一个目标目录。** 登记、workspace_id、URL、revision、租约、冲突规则都在控制侧文件完成；目标侧只需“目录存在/规范化、读写 `.opendan-workspace.json`、建目录”这几个最小操作。按需最小扩展 agent-runtime 接口（新增 op 必须同时进 wire、worker、RBAC op 与账本去重），保持简洁；远端 runtime 的 discover 首期只接受显式目录，不做 roots 扫描 |
| R4 | T2 让 control-panel 为每个 Desktop Agent 写 `nodes/<node>/settings.executor`，与管理员形成双写者；per-Agent profile 的归属、CAS、RBAC 写权限都没定 | **control-panel 不写 node settings。** 节点策略归管理员：`settings.executor` 新增 `owner_agents` 节（是否默认向本节点 owner 的 Agent 开放、默认 roots、默认 ops/capability、`run_as` 可选），Desktop 激活时由系统种子写入并打开；Agent 的选择归 `AgentSettings`（已启用 runtime、默认 runtime、capability）；scheduler 把两者相乘派生 per-Agent profile（`agent-<短哈希>`，满足 `is_valid_profile_id`）、RBAC 与租约。管理员关闭 `owner_agents` 即整体收回，节点上没有该节就是“尚未初始化”，`enabled=false` 是“管理员已禁用” |

其余风险（跨节点 RTCP、Windows WSLC 容器→宿主路径、macOS TCC/完全磁盘访问、D-6）不阻塞开工，列入第 5 节“DV 重点验证”，在本机 PVE 上用多操作系统 VM 覆盖。

## 1. 目标与范围

将已有 llm-context AgentRuntime 和 Node Executor 接入 Agent 的日常使用链路：**系统配置 runtime → 为 Agent 启用 → Session 选择并冻结 → Workspace 绑定 runtime、目录和项目 URL → PLAN 为 Todo 指定执行环境**。

- Desktop 版中，BuckyOS owner 创建 Agent 时默认启用并选择 `host`，使 Jarvis 可以在 owner 的电脑上执行命令、操作文件，并使用配置允许的 ComputerUse 能力。默认值必须由服务端落实，UI 展示实际配置与可用状态。
- `host` 复用 node-daemon 内嵌的 Node Executor，不要求 owner 为本机再安装、配对一个独立 agent-daemon。Desktop 的 node-daemon 运行在 owner 的交互会话中、以 owner 身份执行（§0 R1/R2）：Windows 现状即如此；macOS 依赖安装包改为随登录运行（本次范围外，完成前 macOS host 的执行与 ComputerUse 都如实报告不可用）；Linux 不做 ComputerUse。远端设备继续复用已有 Executor 体系。
- ComputerUse 是 runtime 的**可配置 capability**，不是新增 runtime kind；沿用已有 shell/CLI 路线，不增加专用模型 tool。桌面 Driver 复用 agent-daemon 已实现的 macOS/Windows 实现，抽成共享代码由 node-daemon 注入。
- 每个面向 Agent 的 UI Session 都有明确的默认执行 runtime；新建 Work Session 可由所选 Workspace 决定 runtime。
- Workspace 的产品语义为 `runtime + directory + url`。runtime、目录必选，URL 可选但鼓励填写，用于跨 Agent 交接和识别关联项目。
- 同一 Session / llm-context run 中不热切换 runtime。本期将“Work Session 根据 Workspace 切换 runtime”落实为创建或进入绑定该 Workspace 的新 Work Session；已有会话需要换目标时创建新 Session 并交接。

本 TODO 负责产品与执行链路的集成。ComputerUse 底层协议、Driver、图片回传的详细实现继续遵循 [ComputerUse 技术草案](<../doc/llm_context/BuckyOS Agent-Runtime 集成ComputerUse 技术草案.md>)；尚未完成的部分作为依赖列出，不能据 daemon 子集实现宣称整个桌面闭环已完成。不扩展通用沙箱、任意 runtime 热迁移或新的任务调度服务。

## 2. 当前实现：可复用部分与实际缺口

以下依据源码；历史 TODO 和技术草案中的完成状态不能替代代码核对。

| 入口 | 当前事实 | 本次接入点 |
| --- | --- | --- |
| [agent_tool/runtime/mod.rs](../src/frame/agent_tool/src/runtime/mod.rs)、[node.rs](../src/frame/agent_tool/src/runtime/node.rs) | 已实现 `native / tmux / remote_ssh / remote_node`；`from_config_with` 构造 Node runtime 需要 `XllmDeps.node_executor`。`Capabilities` 只有 tools/network/os | 复用执行体和 connector，补 capability 配置与产品 runtime 到执行配置的解析 |
| [libopendan/protocol/runtime.rs](../src/frame/lib_opendan/src/protocol/runtime.rs)、[state/runtimes.rs](../src/frame/lib_opendan/src/state/runtimes.rs) | 已有 `state/runtimes/<id>.json` 登记与 CAS；container 映射 native，host 的 exec=None；`observe` 仅认当前宿主的 Container 可用 | host 从占位变成系统配置投影；分开启用状态、授权和实时可用性 |
| [node_executor_host.rs](../src/kernel/node_daemon/src/node_executor_host.rs)、[executor_auth.rs](../src/kernel/scheduler/src/executor_auth.rs) | node-daemon 已内嵌 Executor，但只在 config.executor.enabled 时打开；scheduler 从 nodes 的 settings.executor 派生配置、租约和 RBAC | 补 Desktop owner 默认初始化。当前“有 node-daemon”不等于“host 已配置可用” |
| [node_executor.rs](../src/kernel/buckyos-api/src/node_executor.rs)、[node_executor/lib.rs](../src/kernel/node_executor/src/lib.rs)、[agent-daemon/daemon.rs](../src/kernel/buckyos_agent_daemon/src/daemon.rs) | 已有 ComputerInspect/Observe/Act 与 `open_with_computer`；`open_internal` 只在 `identity_mode == CurrentUser` 时开放 Computer ops；`ExecutorOptions::default` 在非 Windows 取 SwitchUser；agent-daemon 以 CurrentUser 注入 Driver；node-daemon（`node_executor_host.rs::open_executor`）调普通 `open`，未注入 Driver | node-daemon 增加桌面模式（CurrentUser + Driver 注入），按 §0 R1 按平台生效 |
| [computer.rs](../src/kernel/buckyos-api/src/computer.rs)、[computer/desktop.rs](../src/kernel/buckyos_agent_daemon/src/computer/desktop.rs)、[native_windows.cpp](../src/kernel/buckyos_agent_daemon/src/computer/native_windows.cpp) | 已有动作、观察、binding、能力与 readiness 模型；macOS 14+ 与 Windows 主显示器 visual 子集有原生实现（见 [agent_daemon.md](../doc/arch/agent_daemon.md)），Linux unsupported；Driver 代码在 agent-daemon crate 内，Windows 通过 `build.rs` 编译 C++ 桥接 | 把 Driver 与平台桥接抽成 node-daemon 可依赖的共享实现；能力开关映射现有操作授权，按实际平台报告 supported/ready |
| [app_schema.rs](../src/kernel/buckyos-api/src/app_schema.rs)、[agent_mgr.rs](../src/frame/control_panel/src/agent_mgr.rs) | `AgentSettings`、创建/更新请求尚无 enabled/default runtime；`AgentServiceBinding` 绑定的是 Loader App 服务 | 在可变 Settings 增加执行 runtime 选择，避免与 Agent 身份/服务绑定混用 |
| [agent_daemon.rs](../src/frame/control_panel/src/agent_daemon.rs)、[AgentDevices.tsx](../src/frame/desktop/src/app/settings/pages/AgentDevices.tsx) | 已有系统 Executor 设备目录、在线查询、配对和 none/observe/control 权限设置；独立 daemon 与内嵌 Node 是两条来源 | 汇聚系统已配置目标，不另造一套设备注册/配对体系 |
| [libopendan/runtime/mod.rs](../src/frame/lib_opendan/src/runtime/mod.rs) | `runtime_for_session / session_runtime` 显式拒绝 remote_node，`bind_or_verify` 拒绝 remote_ssh；workspace workdir 用本机 canonicalize；`--runtime` 目前仅约束身份，并不读取登记的 exec 选择执行体 | 真正接通 Session factory、目标路径、helper、凭据和恢复，不能只移除拒绝分支 |
| [protocol/workspace.rs](../src/frame/lib_opendan/src/protocol/workspace.rs)、[state/workspaces.rs](../src/frame/lib_opendan/src/state/workspaces.rs) | 已有 WorkspaceLocation(runtime_id,directory)、磁盘 metadata、登记、revision/location_revision；没有 URL，文件操作仍有本地假设 | 扩展现有模型和目标文件访问，不新增另一种 Workspace |
| [api.rs](../src/frame/lib_opendan/src/api.rs)、[protocol/config.rs](../src/frame/lib_opendan/src/protocol/config.rs)、[opendan/ui.rs](../src/frame/opendan/src/ui.rs) | 已有 UI/子 Session 创建、workspace binding、runtime requirement 和持久 binding；子 Session 继承父 runtime 配置 | 统一选择/冻结规则，显式目标优先于继承，并检查冲突；UI route 复用旧会话时保留其绑定 |
| [todo_tools.rs](../src/frame/agent_tool/src/todo_tools.rs)、[todos.rs](../src/frame/lib_opendan/src/todos.rs)、[runner/todos.rs](../src/frame/lib_opendan/src/runner/todos.rs) | Serial 是同 Session 的 DO 子 context；Explore 是独立子 Session；Todo 没有 runtime/workspace；探索当前使用 SubWorkspace::None | Todo 加执行环境意图，跨 runtime 使用独立 Work Session，保留原串行/并行语义 |
| [Workspaces.tsx](../src/frame/opendan/web/src/pages/Workspaces.tsx)、[MessageHub sessionApi.ts](../src/frame/desktop/src/app/messagehub/datamodel/sessionApi.ts) | OpenDAN 已展示 runtime/workspace，但 runtime 列表只读、缺创建表单；MessageHub 创建 msg session 未传执行 runtime | 补启用管理、创建/导入表单和真正 Agent UI Session 的绑定，区分消息会话与执行会话 |

## 3. 必须统一的选择与配置契约

### 3.1 系统目录、Agent 启用与真实目标

1. **系统目录是可选 runtime 的来源。** 复用 `nodes/<node>/settings.executor`、`services/control_panel/agent_executors` 等现有配置及在线发现，提供统一的 runtime 列表视图；当前 Agent 的本地登记是投影/缓存，不是另一个授权真相源。独立 Executor 的在线列表不能替代内嵌 Node 的配置来源，也不能把临时离线的已配置项删除。
2. **Agent Settings 保存选择，节点策略保存许可，scheduler 派生授权（§0 R4）。** `AgentSettings` 增加 `enabled_runtime_ids`、`default_runtime_id` 和按 runtime 的 capability 配置；最终字段按共享类型收敛。默认项必须属于已启用集合，启用项必须来自系统允许该 owner/Agent 使用的目录。节点侧 `nodes/<node>/settings.executor` 新增 `owner_agents`（默认开放、默认 roots、默认 ops、可选 run_as），只有管理员和系统种子写它；control-panel 只写 AgentSettings。scheduler 对“节点开放 × Agent 启用 × AgentPermissionMode”派生 per-Agent profile（id `agent-<短哈希>`）、principal_bindings、RBAC 与租约，沿用现有 generation/revision 机制。Agent 不能用自行提交的 exec/endpoint 获得新授权。
3. **目录项指向明确执行目标。** 记录稳定 runtime 标识、显示名、目标 Node DID 或 executor_id、profile、执行配置来源和能力。沿用 `RuntimeRecord`，补齐引用语义；系统引用与 Agent 局部 `host` 别名的映射必须可追踪。
4. **host 表示 owner 选定的桌面宿主。** 在创建时解析并保存实际目标；容器迁移、重启或设备上线顺序改变不能把 host 改指另一台电脑。Desktop 的 Docker/虚拟机环境也必须识别 owner 原生 OS，不能把 Linux VM 自动当作 owner 的 Windows/macOS 桌面。
5. **产品 ID 与底层 kind 分开。** `host` 经 node-daemon 的现有 `remote_node {node_did, profile_id}` 执行；`container` 可映射 native。native 只表示 Runner 所在环境，不能用它代表容器外宿主。外接 agent-daemon 的 WSS Executor 入口与 Node connector 分别复用，未接通完整 Session 的目标不得标为可执行。
6. **设置与状态分开。** enabled 是配置；supported、authorized、online/ready 及失败原因是观测结果。临时不可用不修改用户设置。禁用默认项时要求同一更新选择另一个已启用默认项；系统撤销或下线后保留原绑定并报告不可用，不能静默回退 container 或其它设备。

Desktop owner 新建 Agent 的默认链路：Desktop 激活时系统种子写入该节点的 `owner_agents` 并打开 → owner 创建 Agent，control-panel 按节点策略在 AgentSettings 中写入 host 启用/默认/capability → scheduler 派生 per-Agent profile、配置、租约、RBAC → node-daemon 收敛 → 探测能力 → Agent runtime 投影可用。复用 Agent 创建的幂等与状态机制；默认初始化只写 AgentSettings，可重试；管理员已设置的 roots/权限不被 Agent 侧改写；owner 在 AgentSettings 里关掉的 host 不会在重启或重试创建时被重新打开。

仅 Desktop owner 创建场景默认启用 host；其他用户、服务器或 `--dev` 环境沿用其明确配置，不把节点上的所有 Agent 自动加入授权。节点没有 `owner_agents` 节是“尚未初始化”，`owner_agents.enabled=false` 是“管理员已禁用”，两者 UI 文案与行为都要区分。

### 3.2 ComputerUse capability

- 复用现有 none/observe/control 语义：none 不授予 Computer 操作；observe 允许 inspect/observe；control 再允许 act。动作带 after feedback 时仍要求 observe 权限。Desktop 新 Agent 继承系统 host 的 capability 默认配置，用户可在管理 runtime 时单独调整。
- 将系统允许、Agent 配置、AgentPermissionMode、profile/RBAC、平台实际支持及 readiness 合并为有效能力。配置开关与 Driver 的 `supported/ready/reason` 分开返回；只读 Agent 不能因 host 默认启用获得 exec/write/edit/ComputerAct。
- capability 按 Agent + runtime 生效。当前 `ExecutorProfile.ops` 是 profile 级，不能直接修改共享 ops 而改变其它 Agent 的权限；按 §0 R4 由 scheduler 派生 per-Agent profile 落实差异。
- 禁用必须进入服务端准入/执行端权限链，不能只隐藏 UI 或命令说明。关闭 ComputerUse 后 shell/文件能力按各自配置继续工作；这不是对任意 shell 程序的 OS 级隔离承诺。
- 平台矩阵（§0 R1）：**Windows** node-daemon 已在用户桌面会话以 CurrentUser 运行，注入共享 Driver 后即可 inspect/observe/act，首期只承诺 agent-daemon 已实现的主显示器 visual 子集；**macOS** node-daemon 代码支持同样的桌面模式（非 root、CurrentUser、注入 Driver、TCC 权限由 `doctor`/inspect 报告），但安装包改为随登录运行前，root LaunchDaemon 下的 host 对无 run_as 的 owner profile 执行返回 Capability、ComputerUse 报告 unsupported/not ready；**Linux** host 保持形态 A（SwitchUser + run_as），ComputerUse 固定 unsupported。node-daemon 的桌面模式按“进程是否 root / 是否在交互会话”判定并写进 RuntimeDescriptor，不靠安装包类型猜；不控制另一个登录用户的桌面，不在 Session 0 / 安全桌面输入。
- Driver 共享方式：把 `buckyos_agent_daemon/src/computer/{desktop.rs,mod.rs,native_windows.cpp}` 与 `build.rs` 的桥接编译抽成 node-daemon 与 agent-daemon 共用的 crate 或模块；输入协调锁（macOS `/tmp/buckyos-computer-<uid>.lock`、Windows `session-<id>.lock`）继续按用户会话共用，两种 daemon 同机并存时互斥而不是各写一份。
- runtime/Session 对外展示有效 capability，提示词与 cmd_skill 发现据此决定如何使用。`computer_session_id`/桌面 binding 与 OpenDAN Session ID 保持各自语义。
- 依照 ComputerUse 草案核对 `buckyos computer` CLI、SDK、图片附件到模型及恢复链路；未接通时记录依赖，不把“截图生成成功”当成“Agent 已看到截图”。

### 3.3 Session 选择、冻结与恢复

建议统一解析入口，供 API、CLI、UI 和 Todo 派发调用；不要各自合并一份 runtime JSON。

| 场景 | runtime 选择规则 |
| --- | --- |
| 新 UI Session，无 Workspace | 显式选择的已启用 runtime；未指定则取 Agent 默认项。Desktop owner 默认 host |
| 新 Work Session，有 Workspace | 采用 Workspace 的 runtime；若另有显式 runtime，必须一致 |
| 子 Work Session / Todo，有显式目标 | 解析所选 Workspace/runtime，不被父 llm_context.runtime 覆盖；冲突立即拒绝 |
| 子 Work Session，无显式目标 | 继承父 Session 已冻结的 runtime；仍检查当前授权 |
| 已有 Session 恢复 | 使用已冻结配置与目标；不重新读取 Agent 默认值来换目标 |

- 在首次推理/执行前持久化选择结果：runtime 引用及配置版本、实际 Node/Executor/profile、执行 workdir、Workspace binding（若有）和执行描述。沿用现有 binding/run 持久化机制；ID 相同但实际目标变化也应被检测到。`api.rs::same_session_identity` 当前未比较 runtime，须把显式执行意图纳入幂等冲突检查；重试只复用原冻结结果，不用新默认值重绑。
- 子任务的 Workspace 选择必须能表达 `inherit / none / <id>`。先解析最终 Workspace，再核对 runtime；只指定 runtime 却与继承 Workspace 冲突时，要求显式选择 `none` 或目标 Workspace，不能默默丢弃父绑定。现有探索入口默认 `none` 的行为保留。
- 模板/behavior 声明的 runtime 与显式选择冲突时返回可修正错误，不能覆盖用户选择。`__EXEC__`、shell、文件工具在同一目标环境执行；MCP/宿主工具保留自己的实际位置，并明确描述。
- 控制侧 SessionDir、状态、快照、日志留在 Agent Loader；执行目录属于目标 runtime。没有 Workspace 的 host UI Session 也要有目标侧默认工作目录/独立 Session 工作目录，不能把容器内 SessionDir 直接传给宿主。
- 处理远端 PATH、`agent-session`/Todo/report 等 helper、cmd_skill 内容和凭据：优先复用现有 OpenDAN 服务与受限客户端入口，让控制侧操作仍由控制侧完成；目标工具只获得当前 Agent/Session 所需凭据。不得把整份 AgentRootFS 或刷新凭据复制到目标目录来绕开接线。
- `RemoteNodeRuntime::open` 当前拒绝非空 path_prefix。先完成目标侧 helper/工具可达性和控制侧调用路径，再开放完整 Session；文件、提示词模板和 helper 都需要真实运行验证。
- 同一 Session 中修改 runtime、Workspace location 或底层目标均不受支持。切换创建新 Session，交接 objective、Workspace URL、必要摘要和报告引用；父 UI/PLAN Session 保持自己的原 runtime。
- 禁用/撤权阻止后续新调用；已发出的操作按现有 query/cancel 和操作状态处理，不假定立即回滚。resume 不能自动重发未知副作用，失败不能落到 Runner 本地执行。
- MessageHub 的 msg session 只是消息层对象。必须追到 OpenDAN UI Session 的创建/恢复处保存默认 runtime，并覆盖非 MessageHub 入口；不能仅给聊天列表增加一个显示字段。

### 3.4 Workspace = runtime + directory + url

- 延用 `WorkspaceLocation {runtime_id, directory}`；新增可选 `url` 项目关联字段，贯通 WorkspaceCreate/Import/Metadata/Record/Update、`.opendan-workspace.json`、registry、discover 和交接输出。URL 可更新，目录移动仍用现有 location_revision/CAS 语义。
- **分层（§0 R3）**：Workspace 的登记、身份、URL、revision、独占租约（`lock_path("workspace:<id>")`）和 relocation 冲突规则全部留在 Agent State 的控制侧文件，不随 runtime 变化；runtime 侧只是目标目录。目标侧需要的操作收敛为：目录存在性与规范化（复用 `Describe(workdir)` 的 canonical workdir）、读/写 `.opendan-workspace.json`（复用 Read/Write、`write_whole`）、创建目录。只有建目录与“判定路径是目录/文件/不存在”需要新增 op；新增时同时进 `ExecutorOp`、wire、worker、RBAC action 与账本去重，`FileBackend` 对应加最小方法，native 后端直接落本地 fs。不为远端引入 rename/remove/list 等更多 op。
- 创建/导入必须选择 Agent 已启用 runtime。`directory` 的绝对性由 `buckyos_api::is_absolute_node_path` 按目标 OS 判定，规范化、存在性和 metadata 读写在目标完成；Windows 路径不能交给 Linux Runner 的 `Path::is_absolute/canonicalize` 判定。远端 runtime 的 discover 首期只接受显式目录（import 路径），roots 扫描只对 native 保留。
- runtime+directory 是物理工作位置，workspace_id 仍是稳定身份；URL 是关联信息，不是目录、凭据、授权或唯一键。相同 GitHub URL 可以对应不同 Agent、runtime 和工作副本；填 URL 不自动 clone，也不自动抓取凭据。
- 首期 URL 支持明确校验的 http/https 项目链接（例如 GitHub 仓库页面），可为空，不接受带用户名密码的 URL。若后续需 Git SSH remote 等来源类型，单独扩展，避免把任意文本当可点击链接。
- URL 随目标 metadata 被 discover/import 读取，跨 Agent 交接能看到“关联项目 + 当前 runtime/目录”。不要靠解析 description 或读取 Git remote 临时猜测。
- 保持 Workspace 现有独占租约、访问模式、metadata 身份和 relocation 冲突规则；父/子 Session 不能同时占有同一可写 workspace。跨 Agent 的相同 URL 不代表已取得目录租约；共享同一物理目录须核对跨 Agent 冲突能力，未实现时明确拒绝或使用独立副本。

### 3.5 PLAN / Todo 的执行位置

- Todo 创建和查看支持可选 runtime_id 和 Workspace 选择（`inherit / none / <id>`）；Workspace 决定 runtime，二者冲突拒绝。没有 Workspace 决定目标、也未指定 runtime 时继承父 Session，而不是重新取 Agent 当前默认项；串行 Todo 默认继承父 Workspace，探索 Todo 默认 none。
- Todo 保存用户意图；每次 attempt 保存解析后的目标及执行绑定，报告保留实际执行位置。retry 生成新 attempt，不改旧 attempt/报告；同一派发键重试必须回到同一 child_sid 和冻结目标。
- 同 runtime、同工作目录且符合现有绑定的串行 Todo，继续走当前 DO 子 context。
- 需要不同 runtime 或不同 Workspace 的 Todo，创建独立 Work Session。**串行任务仍串行等待和 Review，不能为换 runtime 把任务改成可并行的 Explore。** 最小扩展应区分调度方式（serial/explore）与执行载体（本 Session 子 context/独立 Work Session），复用 ChildDriver、报告回流和既有幂等派发。
- 独立 Session 使用合适的入口 behavior；不能直接把依赖父 context 的 DO 模板作为新 Session 入口。子任务拿到明确 objective、验收条件、Workspace URL 和自身 runtime 信息。
- PLAN 紧凑视图增加 runtime、Workspace/URL、失败原因和 report_ref；不注入全部执行细节。Todo dispatch、恢复对账、取消、最终收敛检查要同时覆盖两种载体，子 Session 不直接写父 todos.json。
- 已运行 attempt 不接受 runtime 修改。需要换机器时创建新 attempt/后续 Todo，并保留原结果及未知操作的定位信息。

## 4. 实施任务与依赖

### T1：收敛配置和共享协议

- [ ] 核对第 2 节基线，明确系统 runtime 目录的两个已有来源及稳定 ID/host 映射；使用现有 Control Panel/OpenDAN 接口扩展，不新建独立服务。
- [ ] 在 `AgentSettings`、创建/更新请求及对应 TypeScript 类型增加 enabled/default runtime 与 capability 配置；补服务端校验、更新 revision/幂等指纹及读回。
- [ ] `ExecutorSettings` 增加 `owner_agents` 节与校验；scheduler `executor_auth` 按 §3.1 第 2 条派生 per-Agent profile、principal_bindings、RBAC 与租约，revision 覆盖 AgentSettings 输入；Desktop 激活种子写入该节。
- [ ] 按 §3.4 分层定下 `ExecutorOp`/`FileBackend` 的最小新增（建目录、路径类型判定），在 wire、worker、RBAC、账本去重与 `RemoteNodeFiles` 同步落实。
- [ ] 扩展 RuntimeRecord/状态视图、Workspace URL、Session 解析结果、Todo 意图与 attempt 记录；实现一个共用的选择/冲突检查入口。
- [ ] 同步 schema/fixtures、API 消费方和错误码；需要改变持久格式时显式升版，按仓库规则不添加旧格式迁移/兼容分支。本任务不引入第三方依赖。

验收：API/CLI/UI 使用同一选择规则，不能提交系统目录外的 runtime、禁用默认项或绕过只读权限。

### T2：Desktop host 默认配置与 capability 生效（依赖 T1）

- [ ] 在 `control_panel::agent_mgr` 的创建流程按节点 `owner_agents` 策略写入 AgentSettings 的 host 默认值，幂等重试、失败状态和 agent update 路径一致；control-panel 不写 node settings。
- [ ] 沿 Node settings(`owner_agents`) × AgentSettings → scheduler/executor_auth → config/RBAC/租约 → node_executor_host 链路启用 owner host；桌面模式 profile 不带 run_as、以 owner 身份执行，Linux 保持 run_as。
- [ ] OpenDAN 启动/刷新时从系统目录与 Agent Settings 投影 runtime，改造 `opendan/src/loader.rs` 的 `ensure_builtin` 装配，替换 host.exec=None 和 observe 的非 Container 硬编码；探测失败保留具体原因。
- [ ] node-daemon 增加桌面模式：非 root/交互会话时 `ExecutorOptions.identity_mode = CurrentUser`，`open_with_computer` 注入共享 Driver（macOS/Windows），Linux 与 root 场景保持现状；把 Driver 与 Windows C++ 桥接从 agent-daemon 抽成共享代码；`describe`/inspect 报告 host 形态、identity_mode 与 Driver readiness。
- [ ] 补齐 capability 配置到执行端的授权更新/撤销；对 macOS 安装包未切换、Linux 无 GUI、Windows 未支持的动作子集如实报告，不把“Agent 创建成功”作为“所有能力 ready”。

验收：无需额外安装 daemon，owner 新建 Agent 即得到已启用的 host，基础命令/文件能力可实用；Windows 上 ComputerUse 按有效配置工作；macOS 在安装包切换前报告具体不可用原因。

### T3：完整 Session 使用 host（依赖 T1、T2）

- [ ] 接通 libopendan 的 runtime factory 与 `from_config_with`/Node connector，统一 RunnerDeps、XllmDeps 和 resume 的构造路径。
- [ ] 拆开控制路径和执行路径，补无 Workspace 的 host Session 默认 cwd、目标文件 backend、模板执行、实际环境信息及目标侧产物的引用/读取。
- [ ] 落实 helper/命令/PATH/凭据方案，覆盖 `agent-session`、Todo、报告、cmd_skill 读取；完成后再替换 remote_node/remote helper 的拒绝逻辑。
- [ ] 在所有 UI Session、Work Session、子 Session 创建路径解析并冻结绑定；接通 `opendan/src/ui.rs`、`lib_opendan/src/host/mod.rs` 与创建幂等检查，补配置变更、目标变化、失联、重启和撤权的处理。
- [ ] 调整 `api.rs::create_sub_session` 对 `SubWorkspace::Id` 的限制：允许 Work 父会话为新独立子 Session 选择已授权的其它 Workspace，父绑定不变；保留访问检查和独占租约，不能只支持 UI 父会话选择 Workspace。

验收：从真实容器内 OpenDAN 发起命令与文件操作，实际落到选定 owner OS；会话续跑与恢复保持同一目标，helper 可完成完整工作流。

### T4：Workspace 创建、导入和项目关联（依赖 T1、T3）

- [ ] 按 §3.4 分层改造 state/workspaces 与 runtime binding：控制侧登记/租约/revision 不动，目标侧的存在性、规范化、`.opendan-workspace.json` 读写和建目录经 runtime 的 `FileBackend`/Describe 完成；远端 discover 只接受显式目录；relocate/check 用目标侧判定替换 `canonicalize/symlink_metadata`。
- [ ] 贯通 URL 字段和修改语义，包括 `opendan/src/home.rs::Home::workspaces` 手写投影；保持 workspace_id、location_revision、操作幂等和租约约束。
- [ ] 扩展 OpenDAN RPC、CLI、Web datamodel；新增 Workspace 创建/导入表单和 runtime 选择，relocate 不再允许任意填写未启用 runtime ID。

验收：host Workspace 的 metadata 确实位于宿主目录；另一个 Agent 导入后可见项目 URL，使用自己的已授权 runtime，旧 Session 不因目录重定位而改绑。

### T5：PLAN 为 Todo 指定 runtime（依赖 T3、T4）

- [ ] 扩展 `agent-session todo` 创建/查看/派发及 `create-worksession` 目标参数，按第 3.5 节选择执行载体；不单靠提示词让模型自行遵守。
- [ ] 修改 serial admission、子 Session 派发、reconcile、报告回流和 Review，保留原并发上限、单写者、租约及 `(parent_sid,todo_id,attempt)` 幂等。
- [ ] 为跨 runtime 串行 Todo 补正确的等待/唤醒；更新 PLAN 紧凑视图和 Jarvis plan/do/子任务入口模板。

验收：PLAN 在 host 会话创建另一个已启用 runtime 的串行 Todo，子 Work Session 正确执行并回报，父会话目标不变，后继串行任务不会提前执行。

### T6：用户入口与手册（依赖 T1–T5，可并行推进 UI）

- [ ] Desktop `agent-setup` 的 model/API/确认页展示默认 host 与能力状态；现有 `RuntimePage.tsx` 的 Loader/模板配置与执行 runtime 明确区分。
- [ ] OpenDAN `Workspaces.tsx::RuntimeList` 提供系统可选项、Agent 启用/停用、默认 runtime 和 capability 设置；Workspace 卡片显示可点击 URL、目标和可用状态。
- [ ] MessageHub 与 OpenDAN Session 详情显示实际冻结的 runtime；新会话按默认/显式选择初始化，切换动作创建新 Session。模型也能在上下文中看到实际 runtime、Workspace 与有效能力。
- [ ] 联动 `opendan/src/service.rs`、`lib_opendan/src/state/krpc.rs`、Web datamodel、Desktop API、同级 websdk 的 Executor SDK/CLI；从源码更新分发，避免只改 dist/rootfs。
- [ ] 更新 [AgentRuntime 设计](<../doc/llm_context/AgentRuntime 设计.md>)、[Session 协议](<../doc/opendan/protocol/Session Directory Protocol.md>)、[Agent TodoList](<../doc/opendan/Agent TodoList.md>) 和相关 schema/fixtures。
- [ ] 同一实现变更检查 Jarvis `cmd_skills` 中的 `opendan_agent_dev`、`ui_core`、`xllm`、`buckyos_admin`；ComputerUse 手册按实际可执行命令补齐，不提前宣传未完成能力。

## 5. 验收矩阵

| 场景 | 必须证明的结果 |
| --- | --- |
| Desktop owner 新建 Agent | 服务端默认 host；Node profile/RBAC 配置生效；容器内 Jarvis 可读写宿主测试目录并运行命令 |
| 用户关闭 host 后重启/重试创建流程 | 不被默认初始化重新启用；现有绑定保留且新执行被拒绝 |
| 系统目录与 Agent enable | 不在目录中或未获授权的 runtime 无法启用；未启用项不能创建 Workspace/Session，服务端同样拒绝 |
| UI Session 默认与恢复 | MessageHub 进入的真实 Agent Session 和其它 UI 入口均有冻结目标；修改 Agent 默认项只影响新 Session |
| 无 Workspace 的 host Session | cwd 属于目标 OS，helper 能创建 Todo、提交报告；控制状态仍由原 Agent 管理 |
| Workspace runtime 冲突 | 显式 runtime 与 Workspace 不一致在推理/副作用前拒绝；不存在静默覆盖 |
| 跨 OS 的 Workspace | Linux Runner 可正确处理目标 Windows/macOS 目录，不进行本地 canonicalize 或意外本地写入 |
| URL 与交接 | create/import/discover/update 后 URL 一致；跨 Agent 看到同一关联 URL，各自 runtime/目录保持独立 |
| Workspace 租约/移动 | 同一可写位置并发冲突被拒绝；移动更新 location_revision，旧 Session 不自动接受新位置 |
| capability 开关与只读 Agent | none/observe/control 在真实执行端生效；只读模式拒绝 exec/write/edit/act；关闭 GUI 不误禁允许的文件操作 |
| 多 Agent 共用目标/profile | A=observe、B=control；调整或关闭 A 的 capability 不改变 B 的权限；管理员关闭 `owner_agents` 后两者同一 generation 内收回 |
| 平台权限或桌面不可用 | 区分不支持、未授权、未 ready；不能把 node-daemon 存在当作桌面可操作 |
| ComputerUse 真实闭环 | Windows 桌面会话 node-daemon：observe → 模型收到图片 → act → 再观察并核验；操作账本与恢复身份可查询；同机 agent-daemon 与 node-daemon 共用输入协调锁互斥 |
| 跨 runtime 串行 Todo | 独立 child_sid，父 runtime 不变，串行顺序/Review 保持；不自动转并行 Explore |
| runtime-only 子任务与继承 Workspace | 冲突时拒绝；显式 none/目标 Workspace 可建子 Session；同派发键换目标被拒绝 |
| 并行 Todo 和恢复 | 各子任务目标冻结；派发前后崩溃只恢复同一 attempt/child，不重复执行；报告仍能追到实际目标 |
| 目标离线、撤权或替换 | 明确失败/等待与原操作状态；不回退 native、不换另一设备、不重放未知动作 |

实施时优先扩展现有测试入口，按实际改动选择运行；以下是已存在的 package/测试目标，不代表本次已执行：

```bash
# 在 buckyos/src/ 下执行；相关改动阶段分别运行
cargo test -p agent_tool --lib runtime::
cargo test -p node_executor --test remote_node_runtime
cargo test -p node_executor --test computer
cargo test -p libopendan --test runtimes --test workspaces --test workspace_binding
cargo test -p libopendan --test plan_do_todo
cargo test -p libopendan --test xagent_lib --test xagent_cli
cargo test -p opendan
cargo test -p control_panel
cargo test -p scheduler
```

补 node-daemon Driver/helper 以及 Desktop/OpenDAN UI 的针对性测试，Web 命令以各 package.json 为准。最终需真实 Desktop owner、容器化 OpenDAN 与 host runtime 的端到端证据；记录平台、账号模式、实际目标、未支持或跳过项。mock 或直接运行独立 xllm 不能代替完整 Session 验收。

### 5.1 DV 重点验证（PVE 多操作系统 VM）

以下是开工 review 中未拍板、留给 DV 证明的风险点；在本机 PVE 上分别起 Windows 11、macOS（可用时）、Linux 桌面/服务器 VM 覆盖，每项记录 VM、安装形态、节点身份与结果。

| 风险点 | 当前依据 | DV 要证明 |
| --- | --- | --- |
| 跨节点 RTCP | node_exector.md §17.7：以节点名经 RTCP 连回本机失败（`resolve remote device … canonical did:dev failed`），只验证了同节点 DIRECT | OOD VM 上的容器化 OpenDAN 经 RTCP 到另一台 Windows/Linux VM 的 node-daemon 执行器 login/exec/read/write；撤销后 `close_connections` 真正关闭存活转发 |
| Windows WSLC 容器→宿主执行器 | DV 只验过 Linux Docker 的 `host.docker.internal:3185 → DIRECT 127.0.0.1:3184`；`doc/windows_wslc.md` 只说明容器端口绑定 Windows loopback | WSLC 内 Agent 容器能经宿主 NodeGateway 到达 3184；Docker Desktop 形态同样验证 |
| Windows 桌面会话 node-daemon | 安装器用 HKCU Run 启动、`ExecutorOptions::default` 为 CurrentUser；注入 Driver 后的 ComputerUse 未在 node-daemon 内验过 | 登录后 node-daemon 自动起、inspect ready、锁屏/切换用户/UAC 安全桌面时拒绝输入；Agent 以 owner 身份读写 owner 文件 |
| macOS 桌面模式 | 安装包仍为 root LaunchDaemon（范围外）；TCC 目录与完全磁盘访问未在真机验证（node_exector.md §16） | 在手工以用户身份运行 node-daemon 的 VM/真机上验证 CurrentUser 路径、TCC 授权提示与 `doctor`/inspect 报告；root 形态下确认报告为不可用而非误报可用 |
| D-6 `Device` 类设备配置位置 | 非 Node 的独立 agent-daemon 目标只有 `services/control_panel/agent_executors`，不是 scheduler 派生的授权真相 | 独立 agent-daemon 目标在 runtime 目录中的启用/授权链是否闭合；未闭合则按 §3.1 第 5 条不标为可执行 |
| 工作区目标侧最小 op | 新增建目录/路径判定 op 后的 RBAC、账本去重与 Windows 路径语义 | Linux 容器内 Agent 在 Windows host 目录创建/导入 Workspace，metadata 落在宿主，revision 与租约在控制侧一致 |

本次为纯文档变更，只核对源码入口、链接、命令目标和差异；不运行构建/DV，不更新 cmd_skill 或生成产物。完成定义是 T1–T6 接通并通过上表对应验收，而非仅让 UI 出现 host 选项。

## 6. 实施进度（2026-10-23）

| 任务 | 状态 | 落点 |
| --- | --- | --- |
| T1 | 完成 | `buckyos-api`：`AgentSettings.runtimes/default_runtime_id`（`AgentRuntimeSettings/Target`、`ComputerAccess`、`validate_runtimes`）、`ExecutorSettings.owner_agents`（`OwnerAgentsPolicy::profile_for`）、`ExecutorOp::Mkdir/Stat` + wire（`ExecutorPathRequest`、`ExecutorPathType`、`describe` 的 `workdir_roots/identity_mode/desktop`）、`owner_agent_profile_id`；`node_executor` worker/ops/stream/store 支持两 op；`agent_tool` `FileBackend::{path_type,mkdir}`、`RemoteOp::{Mkdir,Stat}`、`RemoteNodeTarget`、`ControlSide`/`XllmDeps.runtime_control`；`libopendan` `RuntimeRecord v2`（`default/computer/RemoteNode`）、`Workspace* v2`（`url`）、`RuntimeSection.frozen`；`scheduler::executor_auth` 派生 `agent-<hash>` profile（测试 `owner_agents_policy_derives_per_agent_profiles`） |
| T2 | 代码完成，DV 未做 | `kernel/computer_driver` 共享 crate（agent-daemon 改为依赖）；`node_daemon::node_executor_host` 桌面模式（`desktop_mode`、`CurrentUser`+Driver 注入、`seed_owner_agents`）；`control_panel::agent_runtimes`（`runtime_directory`、`apply_create_defaults`、`apply_update`、`agent.runtime_directory` RPC、`agent.update` 扩展）；`opendan::loader::project_runtimes` + `watch_runtime_settings` |
| T3 | 代码完成，DV 未做 | `libopendan::api::freeze_runtime`（统一选择规则，写 `runtime.frozen`，幂等比较 runtime）；`runtime::runtime_for_session` 异步按冻结 exec 构造 `remote_node`、`node_workdir` 默认目录、`bind_or_verify` 节点分支、`open_session_env` 的 `ControlSide`；`create_sub_session` 继承/显式 runtime 与任意已登记 Workspace；`runner/reports` 节点产物按引用 |
| T4 | 代码完成，DV 未做 | `state/workspaces.rs` 重写为 `Target{Local,Node}` 分层；URL 贯通 create/import/discover/update/query/home 投影；CLI `--url`、`set-url`；OpenDAN WebUI 创建/导入/URL/relocate 表单与 runtime 启停/默认/ComputerUse；kRPC `runtimes.settings[_set]` |
| T5 | 代码完成 | `todo_tools` `NewTodo/TodoRecord.{runtime_id,workspace}`、`TodoAttempt.{runtime_id,workspace_id}`、`begin_serial_session`、`AttemptTarget`；`todos::needs_own_session/dispatch_serial`；`runner/todos::{bind_serial,settle}` 跨目标串行 Todo 走独立 Work Session 并等待；CLI/plan.toml 文案 |
| T6 | 代码完成 | Desktop `agent.runtime_directory` 类型与 API、agent-setup 确认页“运行位置”、Agent 详情 runtime 行；websdk `ExecutorOperation` 加 `mkdir/stat/computer_*`；文档：`doc/arch/node_exector.md`、`AgentRuntime 设计.md`、Session Directory / Agent State / Agent Workspace Protocol、Agent TodoList；手册 `opendan_agent_dev`、`buckyos_admin` |

未做 / 未验证：§5 验收矩阵全部 DV 项（真实 Desktop owner、容器内 OpenDAN 经 host 执行、ComputerUse 闭环、PVE 多 OS）；macOS/Windows 编译未在本机验证（Linux 开发机）；MessageHub 侧仅沿用 UI Session 创建路径冻结 runtime，未增加会话详情的 runtime 展示；websdk CLI 未加 `mkdir/stat` 子命令；`xllm` 手册未改（接口未变）。
