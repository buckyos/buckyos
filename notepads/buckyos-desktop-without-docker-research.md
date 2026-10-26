# BuckyOS Desktop 无 Docker 安装可行性分析（研究报告）

日期：2026-10-25。对应任务：[buckyos-desktop-without-docker-todo.md](buckyos-desktop-without-docker-todo.md)。

后续（2026-10-25，用户决定）：zone owner 的 Agent 选择 host-native-runtime 时 Agent Service 不基于 paios 镜像启动——§5 步骤 ③ 的触发条件改为 Agent 的 runtime 选择（默认 runtime 是 host），已实施并在 Linux 单 OOD DV 通过，见 [opendan-native-agent-service-todo.md](opendan-native-agent-service-todo.md) §6。

源码基线：`33b3ca39e`（HEAD，与 TODO 一致，行号未漂移）。本报告只做源码级分析与推演，**没有做 Windows / macOS 平台实测**；凡标注"推断"的结论都要在 R6 纵向链路验证时复核。

## 0. 结论（回答 §1 核心问题）

**裁决建议：D2 先行（原生 owner Agent、无 AppLoader 应用），D1 后补，D3 不单独立项。**

| 问题 | 结论 | 依据 |
| --- | --- | --- |
| 选哪一档 | **D2**。D1 多出的 G3 是一整套新的 script host app 原生生命周期，而产品文档明确 ScriptHost 的承诺是"开发者只面向统一 Linux 容器环境"（[paios容器需求.md §8.1](../doc/app_service/paios容器需求.md)）；当前仓内唯一的 script app 是测试应用 `buckyos-systest`。D1 的收益目前不足以抵消代价。 | §2 G3、§5 |
| D3 为何不独立 | Desktop 今天**就是 Zone 的 OOD**（激活只会产出 `ood1`），没有任何向导/CLI 能把一台 Desktop 作为非 OOD 设备加入已有 Zone；D3 所需的"用 Zone 内其他节点的 Agent"没有现成的加入路径。且无容器 OOD 上预装的 systest 会让调度器每 5 秒整轮失败（G7）。D3 只作为"D2 去掉 Agent"的底线对照。 | §2 G8、G7 |
| 最小实施顺序 | ① G1/G5/G4 节点能力信号 → ② G6 调度器单 spec 失败隔离 + 不可调度状态外显 → ③ G2 原生 owner Agent（调度器 + AppLoader 子进程）+ `/opt/buckyos` 硬编码 → ④ 安装器变体与 Desktop 预装清单 → ⑤ 私有工具包 + cmd_skills 适配 → ⑥ G3。①–④ 达到"可安装、可激活、owner Agent 原生 ready"，⑤ 达到 D2 "完成一个媒体任务"，⑥ 达到 D1。 | §5 |
| 仍需用户确认 | 节点能力信号的落点（复用 `support_container` 还是新增 `capbilities` 项）；Desktop 预装 systest 的去留；已有 Docker 版 Desktop 的升级/迁移是否纳入；`--desktop_daemon` 是否由安装器固定传入。 | §7 |

"允许安装"判定标准（TODO §2）在 D2 下的落点：

- 干净机器安装、激活不触碰容器 → 需要 ④ 安装器 + ① 信号 + 启动期 `paios_images::spawn_preload` 在无容器节点上短路（§2 G9）。
- owner Agent 完成一个媒体任务 → 需要 ③ + ⑤。
- 放弃能力在所有入口被明确拒绝、不循环 → 需要 ②（当前任何不可调度 spec 都会让整轮调度失败并每 5 秒重试，是"循环失败"的根因）。
- 用户能区分"本机不支持"与"故障" → 需要 ② 把"无候选节点"记成 spec 级状态并由 control-panel 展示。

## 1. 现状的两个基本事实

这两点是 TODO 原文没有写明、但决定全部分析的前提：

1. **Desktop = 单机 OOD。** 激活服务固定创建名为 `ood1`、`device_type = "ood"` 的设备（[active_server.rs:352-360](../src/kernel/node_daemon/src/active_server.rs#L352-L360)；`DeviceDocument::new` 默认 `device_type: "ood"`，[device.rs:164](../../buckyos-base/src/name-lib/src/device.rs#L164)）。所谓 Desktop 节点在调度器眼里就是一个 `NodeType::OOD`，owner 的 Jarvis 实例被调度到它自己身上并在 aios 容器里运行。
2. **调度器只向 OOD / Server 放实例。** [scheduler.rs:1212-1214](../src/kernel/scheduler/src/scheduler.rs#L1212-L1214) 直接过滤掉 `desktop` 等其他类型；RBAC 也只给 OOD/Server 节点组（[system_config_agent.rs:1550-1558](../src/kernel/scheduler/src/system_config_agent.rs#L1550-L1558)）。因此"把 Desktop 标成 `desktop` 类型"不是解法：那样它既不会收到 Agent，也不会收到任何应用。

由此，"无 Docker 的 Desktop"在当前架构里只能是 **一台 `support_container = false` 的 OOD**。下面的缺口都按这个前提核对。

## 2. 关键缺口（源码证据 → 失败场景 → 影响）

G1–G5 沿用 TODO 编号并补充证据；G6–G9 是本次新发现。

### G1 激活写死 `support_container = true`（已核实，补充三处）

- 向导 [active_lib.ts:306](../src/kernel/node_active/active_lib.ts#L306)；服务端 [active_server.rs:360](../src/kernel/node_daemon/src/active_server.rs#L360) 原样写入，并在 [923-927](../src/kernel/node_daemon/src/active_server.rs#L923-L927) 校验 DeviceDocument 与 topology 一致。`GatewayTopology.support_container` 的 serde 默认也是 true（[active_server.rs:117-118,129](../src/kernel/node_daemon/src/active_server.rs#L117-L129)）。
- 其他写死点：websdk `provision.ts:412` 与 `provision.ts:482`（后者注释"true is the serde default and is skipped"）、[zone.rs:1068](../../buckyos-base/src/name-lib/src/zone.rs#L1068)、调度器测试种子 [scheduler/main.rs:734](../src/kernel/scheduler/src/main.rs#L734)。`DeviceDocument` 字段本身 `skip_serializing_if = "is_true"`（[device.rs:131-132](../../buckyos-base/src/name-lib/src/device.rs#L131-L132)），意味着"false"是唯一会被序列化出来的值，老文档一律按 true 解析。
- 读取点一致：[scheduler.rs:1221](../src/kernel/scheduler/src/scheduler.rs#L1221)、[app.rs:56](../src/kernel/scheduler/src/app.rs#L56)、[app_loader.rs:853-868](../src/kernel/node_daemon/src/app_loader.rs#L853-L868)（读 `runtime.device_config.support_container`）。

失败场景：无 Docker 机器完成激活 → DeviceDocument 声称支持容器 → owner 建 Agent → 调度器把实例派到本机 → AppLoader `prepare_worker_image` 执行 `docker`/`wslc` 失败 → `ensure_run_item_state` 每轮重试。影响：D1/D2/D3 全部；是"隐式容器探测失败"的直接来源。

### G2 Agent 没有原生分支，且是两层（已核实，补充第三层）

- 调度器：`need_container = is_agent` 无条件（[system_config_agent.rs:83-85](../src/kernel/scheduler/src/system_config_agent.rs#L83-L85)）；非 Agent 应用只有 author 为 `did:web:buckyos.ai|io|org` 且某个包无 docker 镜像时才免容器（86-100）。
- 实例化：即便调度放行，[app.rs:50-83](../src/kernel/scheduler/src/app.rs#L50-L83) 在 `!support_container` 分支要求 `<arch>_<os>_app` 或 `script` 包；Agent 包只有 `agent` 键（[jarvis APPDOC](../src/apps/jarvis_agent/dapp_dist/APPDOC.json) `pkg_list: {agent}`）→ `app_pkg_name not found`。这是 TODO 没写到的**第三层**。
- AppLoader：[app_loader.rs:812-822](../src/kernel/node_daemon/src/app_loader.rs#L812-L822) Agent 必须 `device_supports_container()`，否则 `NotSupport`；`start_agent`（1212-1261）→ `run_worker_container`；`stop_agent` 删容器（1263-1265）；`status` 走 `worker_expectation` + `docker inspect`（484-560）。

但进程契约本身与 Docker 无关（lifecycle 子报告 §1、§3）：`build_runtime_env`（[app_loader.rs:1420-1547](../src/kernel/node_daemon/src/app_loader.rs#L1420-L1547)）在 node_daemon 进程内用设备私钥签 `BUCKYOS_APP_TOKEN`（`generate_service_login_assertion`），连同 `BUCKYOS_ZONE_CONFIG / THIS_DEVICE / HOST_GATEWAY / KEVENT_DAEMON_ADDR / APP_ID / APP_DID / APP_INSTANCE_ID / OWNER_USER_ID / DATA_DIR / TOOL_ENDPOINT / PKG_DIR` 一起交给容器；opendan 进程启动只依赖这些 env + `--agent-bin`（[opendan/main.rs:234-355](../src/frame/opendan/src/main.rs#L234-L355)）。`debug_jarvis.sh` 已经证明同一套身份可以在宿主进程上登录 verify-hub 并工作，只是它靠两个调试捷径：直接读设备私钥 PEM、`--trust-loopback`（[service.rs:130-132](../src/frame/opendan/src/service.rs#L130-L132)：loopback 无 token 即 owner）。产品化不需要新身份方案，只需要 node_daemon 新增一个"以子进程方式起 `bin/opendan/opendan`"的 `RuntimeType`，并把 `BUCKYOS_HOST_GATEWAY` 设为 `127.0.0.1`。

需要同时修的硬编码：[opendan/main.rs:283](../src/frame/opendan/src/main.rs#L283) 兜底 `/opt/buckyos/bin/<appid>`；`container_app_data_dir`（app_loader.rs:3374-3376）固定 `/opt/buckyos/...`；`get_buckyos_root_dir()` 在 Windows 为 `%APPDATA%\buckyos`、其他平台 `/opt/buckyos`（[slog path.rs:3-15](../src/kernel/slog/src/util/path.rs#L3-L15)，macOS 没有独立分支）；`xagent` / `agent_tool` 是 opendan 的符号链接（build_aios:626-627），Windows 需按文件名复制（`invoked_name()` 已剥 `.exe`，main.rs:485-495）。

影响：D1、D2。工作量集中在 node_daemon（新 RuntimeType 的 deploy/start/status/stop）与调度器（spec 维度），opendan 侧改动小。

### G3 Desktop 本地应用链路名义原生、实际进容器（已核实）

[app_loader.rs:825-832](../src/kernel/node_daemon/src/app_loader.rs#L825-L832) local app → `HostScript`；deploy 准备 worker image + ExtTool volume（417-423）；`start_host_script` → `run_worker_container`（1169-1200）；停止/状态同 Docker 标签。`bin/applist.json` 随包交付但内容是 `{}`（[src/rootfs/bin/applist.json](../src/rootfs/bin/applist.json)），即**今天没有任何本地 app 走这条链路**。Local app 的 env 只有 `local_app_instance_config`，没有 `BUCKYOS_APP_TOKEN` / AppService 身份（app_loader.rs:1534-1544）。

产品层面：[paios容器需求.md §8.1](../doc/app_service/paios容器需求.md) 规定 ScriptHost 开发者"不需要自行处理三平台差异"、依赖进实例卷。原生化等于撕掉这个承诺。结论：G3 代价大、当前收益小 → D1 后补。

### G4 Windows 的 wslc 路径（已核实）

[wslc.rs:4-27](../src/kernel/node_daemon/src/wslc.rs#L4-L27)：`BUCKYOS_CONTAINER_RUNTIME=docker|wslc` 显式选择，否则 Windows 上自动在 PATH 与 `%ProgramFiles%\WSL\wslc.exe` 查找；app_loader 分支在 1681、3560-3562（`host.wslc.internal`）、3646、3690-3700。[doc/windows_wslc.md](../doc/windows_wslc.md) 明言 Windows 找到 `wslc.exe` 即优先用它，且"完整 Windows 安装、Zone 激活和 Jarvis/LLM 对话尚未进行端到端验证"。

失败场景：用户机器装过 WSL（Windows 11 常见）但没有 Docker → 安装器的 Docker 检查放行/跳过 → node_daemon 自动落到 wslc → 实际在 WSL 里跑 Linux 容器，违反"无 Docker 同时意味着不自动落到 wslc"。修法：G1 的能力探测必须把 wslc 一并计入"容器能力"，并且 D2 档位下 node_daemon 启动时按产品模式**禁用**自动探测（显式 `BUCKYOS_CONTAINER_RUNTIME=none` 或配置项），而不是只靠安装器不检查。

### G5 三个身份信号互不等价（已核实，情况比 TODO 写的更差）

- `--desktop_daemon`（[node_daemon/main.rs:56-58](../src/kernel/node_daemon/src/main.rs#L56-L58)）控制两件事：按 `applist.json` 维护本地 app（[node_daemon.rs:1151,1307-1313](../src/kernel/node_daemon/src/node_daemon.rs#L1151)），以及 Node Executor 的 A-desktop 形态（[node_executor_host.rs:144-156](../src/kernel/node_daemon/src/node_executor_host.rs#L144-L156)：`desktop_build && computer_driver::SUPPORTED && euid != 0`）与 `owner_agents` 种子（206-247）。
- **仓内没有任何安装脚本或启动脚本传这个标志**：`grep desktop_daemon src/publish src/start.py` 为空，`start.py:334` 只传 `--enable_active`。即当前发行的 Desktop 包在 node_daemon 看来与服务器 OOD 完全相同；Node Executor 的"桌面模式"目前只有手工启动才生效（安装器侧待 §4 合并确认）。
- `support_container` / `device_type` 在 DeviceDocument 里；`capbilities: HashMap<String,i64>` 字段存在但全空（[device.rs:133-135](../../buckyos-base/src/name-lib/src/device.rs#L133-L135)）。

结论：需要一个在激活时写入 DeviceDocument、node_daemon 与调度器都能读到的持久信号；`--desktop_daemon` 是进程参数，不能作为唯一真相。建议见 §5 步骤 ①。

### G6（新）一个不可调度 spec 让整轮调度失败，每 5 秒重试

[scheduler.rs:860](../src/kernel/scheduler/src/scheduler.rs#L860) `reconcile_spec_instances(...)?` → [create_replica_instance 1151-1158](../src/kernel/scheduler/src/scheduler.rs#L1151-L1158) 在无候选节点时返回 `Err("No suitable node found")` → `schedule()` 整体 Err → [system_config_agent.rs:2537-2543](../src/kernel/scheduler/src/system_config_agent.rs#L2537-L2543) 记错误返回 → `schedule_loop` `continue`，5 秒后重来（2641 起）。

影响：无容器 OOD 上只要存在一个 `need_container` 的 spec（owner 的 Agent、或 G7 的 systest），**所有**后续调度动作（新装应用、服务状态变更、网关节点列表更新）都停摆，日志每 5 秒一条错误。这是 TODO §7 "不循环失败"要求的真正阻塞点，必须在 G2 之前修：按 spec 隔离失败、把 spec 记成"无候选节点"状态并暴露给 control-panel。

### G7（新）Desktop 预装的 `buckyos-systest` 需要容器

[boot.template.toml](../src/rootfs/etc/scheduler/boot.template.toml) 预装 `text-editor`（web，`selector_type` Static → 不建 spec，[system_config_agent.rs:191](../src/kernel/scheduler/src/system_config_agent.rs#L191)）与 `buckyos-systest`（`app_type: dapp`、`pkg_list: {script}`、`selector_type: single`、**author `did:bns:buckyos`**）。因 author 不是 `did:web:buckyos.*`，systest 的 `need_container = true`；它在今天的容器节点上以 HostScript 进 aios worker 运行。在无容器 OOD 上它直接触发 G6 的整轮失败，与 Agent 无关。处置选项：Desktop 发行的 install_settings 不预装 systest；或修正 author 放行规则为可信来源判断（TODO §5.2 "author 字符串放行"同题）；两者都要做 G6 兜底。

### G8（新）没有"把 Desktop 加入已有 Zone"的路径（影响 D3）

node_daemon 有非 OOD 角色的 boot 分支（[boot.rs:22-47](../src/kernel/node_daemon/src/boot.rs#L22-L47)、[node_daemon.rs:2100-2170](../src/kernel/node_daemon/src/node_daemon.rs#L2100-L2170)），但：激活服务只产出 `ood1`；websdk `provision.ts` 只按 `oodName` 建设备；`get_system_config_client(is_ood=false)` 是 `unimplemented!()` 死代码（[node_daemon.rs:1319-1328](../src/kernel/node_daemon/src/node_daemon.rs#L1319-L1328)）；control-panel 没有"添加设备"RPC。D3 的"访问 Zone 内其他节点上的 Agent"要先有设备加入流程，这超出本研究范围。

### G9（新）启动期镜像预加载

node_daemon 进程一开始就 `paios_images::spawn_preload()`（[node_daemon.rs:1750](../src/kernel/node_daemon/src/node_daemon.rs#L1750)）。发行包不 `pull`，而是从 `bin/paios-images/` 的 tar `load`（[start.py:262-266](../src/start.py#L262-L266) 文档字符串、[windows_wslc.md](../doc/windows_wslc.md) "由 node-daemon 在自己的上下文里从包内 tar 执行 load"）。无容器节点上这一步必须按能力信号短路，否则每次启动都会调用 `docker`/`wslc` 并报错；同时 Desktop 发行包可以不再携带这些 tar（体积问题见 §4 合并）。具体行为见 §4 安装器子报告。

### 其他已核实的次要点

- control-panel 的容器概览与容器操作 RPC 直接 `docker ...`（[main.rs:81-83](../src/frame/control_panel/src/main.rs#L81-L83)、zone_mgr.rs:900-1000）；无 Docker 时只是该 RPC 报错（软失败），UI 需按能力信号隐藏。
- 内核与框架服务（system_config、verify_hub、scheduler、cyfs-gateway、msg-center、aiworkspace、homestation、control-panel 等）由 node_daemon 直接作为本机进程运行，`kernel_mgr.rs` / `frame_service_mgr.rs` 没有 docker 引用；`ServiceSpecType::Kernel` 的 `need_container = false`（system_config_agent.rs:136）。**内置系统服务原生运行这一项没有隐藏容器依赖**（安装/激活/身份/服务发现）。
- Jarvis 只是模板（`agent_templates`），由 owner 在 control-panel 创建时实例化为 `app_type: agent`、`expected_instance_count: 1` 的 AppServiceSpec（agent_mgr.rs:3684）；启动阶段没有 Agent spec，所以 boot 调度不受 G6 影响，首次失败发生在建 Agent 之后（或 systest 预装任务完成之后）。
- 文档口径：[impl_buckyos_packaging.md §9.1/9.2](../doc/CI/impl_buckyos_packaging.md) 把 Docker Desktop / OrbStack 写成安装硬前置；[paios容器需求.md R-45、§8.2](../doc/app_service/paios容器需求.md) 以"Windows/macOS + Docker Desktop"为目标环境并规定"Agent 默认仍运行在受限容器边界内"。D2 落地时这些文档要同步改口径。

## 3. 原生 owner Agent 的运行时与状态影响（R2 / Standard-Target Runtime 共享部分）

来自 lifecycle 子报告 §4–§5，已抽样核对：

- **Node Executor 不是进程监督器。** 它是"一次 exec 一个 shell、一次文件操作一个 worker、操作有持久状态机"的受控执行器（spawn.rs、ops.rs），没有"让某个服务进程保持存活/重启"的概念；Stream 断开 ≠ 取消，重启后 running 记 unknown（node_exector.md §7.4）。因此它**适合承载 Agent 的子命令**（Session `host` runtime 已经这样做），**不适合托管 opendan 主进程**。原生 opendan 的拉起、健康检查、崩溃重启应放在 node_daemon 的 RunItem 层（与内核服务同一机制），不新造一层。
- **Desktop 的进程身份形态已定**（integration TODO §0 R1/R2）：node_daemon 随用户登录、运行在交互会话、CurrentUser。原生 opendan 作为 node_daemon 子进程天然继承该身份；这同时意味着 Agent 拥有该桌面用户的全部权限（node_exector.md §16 对 Windows 的说明），按"先能力后安全"原则写入已知风险。
- **Session 冻结绑定会全部失效。** `container` runtime 的 Session workdir 是容器内绝对路径 `/opt/buckyos/data/home/<owner>/.local/share/<app>/agents/<id>/sessions/<sid>`，原生进程在 `%APPDATA%\buckyos\...`（Windows）下 canonicalize 后与 `Binding.workdir` 不等 → `RuntimeMismatch` / `WorkspaceBindingInvalid`（[runtime/mod.rs:697-705](../src/frame/lib_opendan/src/runtime/mod.rs#L697-L705)）。`runtime_host` 用 `AGENT_TOOL_HOST_ID = <device.id>/<app_instance_id>`（[opendan/main.rs:468-482](../src/frame/opendan/src/main.rs#L468-L482)）则能匹配；但回退路径 `/etc/hostname` + `/etc/machine-id`（[agent_tool runtime/mod.rs:544-551](../src/frame/agent_tool/src/runtime/mod.rs#L544-L551)）在 Win/mac 不存在。按"DV 可全新安装、不写兼容"原则：新安装不受影响；从 Docker 版升级的 Desktop 需要决定是清 Session 还是写迁移（§7）。
- **`host` runtime 的 Session 不受影响**：冻结为 `remote_node` 指向本机 node_did，工作目录在 `<HOME>/.opendan/...`，`SocksNodeConnector` 对本机 DID 直接用 `127.0.0.1`（node.rs:1801-1804）。原生 Agent 之后，`container` 与 `host` 两个内置 runtime 在 Desktop 上指向同一台机器、同一身份，[protocol/runtime.rs:17-21](../src/frame/lib_opendan/src/protocol/runtime.rs#L17-L21) 的语义注释（"the agent's container" / "the machine the container runs on"）需要重写，但不必合并两者：`container` 仍表示"Agent 进程自身环境"，只是现在它就是宿主。
- **tmux**：用户已裁决全线移除；tmux runtime 与 Session 绑定 tmux 的规则已于 2026-10-25 删除，不在本研究内适配。
- **AIWS**（R4）：`service` 模式由 aiworkspace 服务进程自己起 Deno，`--allow-read=<workdir> --allow-write=<workdir>/output --allow-net --allow-env=AIWS_HOST,AIWS_TOKEN`、无 `allow-run`（[runner.rs:70-130](../src/frame/aiworkspace/server/src/wish/runner.rs#L70-L130)），Deno 来源 `AIWS_DENO` > `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno[.exe]` > PATH（40-58），与 Agent 位置无关；`hosted` = 给 owner 指定的 Agent 建 Goal，由 Agent 用 `buckyos aiws run-claim/...` CLI 领取并用自己的工具执行（[exec.rs:295-332](../src/frame/aiworkspace/server/src/wish/exec.rs#L295-L332)），验收报告注明托管执行"未在真实 Zone 验证"。原生 Agent 后，hosted run 的程序在 owner 电脑上以登录身份执行；D2 的 AIWS 闭环默认走 hosted 的前提是 `buckyos` CLI（websdk dist + 私有 Deno）在原生环境可用（见 §4 工具包）。

## 4. 安装器、发布链路与工具基线

### 4.1 安装器与首次启动（R5 / TODO §5.2 核实）

分类：**(a)** 硬前置、安装或启动中止；**(b)** 可选行为、静默降级；**(c)** 隐式拉取/启动容器。

| 环节 | 事实 | 分类 | 无容器变体要改的点 |
| --- | --- | --- | --- |
| macOS preinstall | [buckyos_preinstall:24-35,72](../src/publish/macos_pkg/scripts/buckyos_preinstall#L24-L35) 以 root 跑 `command -v docker` + `docker info`（15 s 超时），失败 `exit 1`，帮助文案只推荐 OrbStack；只挂在 `buckyos` 服务组件，BuckyOSApp 单独可装（[make_local_osx_pkg.py:433-434](../src/publish/make_local_osx_pkg.py#L433-L434)）。37-51 行 best-effort 清理旧容器。 | (a) | 删除/条件化 24-35、72 行 |
| macOS verify | [make_local_osx_pkg.py:941-951](../src/publish/make_local_osx_pkg.py#L941-L951) 要求 preinstall **同时包含**五个字串（`checking Docker availability for LaunchDaemon/root`、`command -v docker`、`docker info`、`OrbStack`、`https://orbstack.dev/download`），缺一则 verify 失败；770-771 行禁止引用 `/var/run/docker.sock`。 | (a) 构建期 | 与 preinstall 同步改，按产品模式切换校验集 |
| macOS postinstall | [buckyos_postinstall:56-93](../src/publish/macos_pkg/scripts/buckyos_postinstall#L56-L93) 写 `/Library/LaunchDaemons/buckyos.service.plist`，`ProgramArguments = [node_daemon, --enable_active]`，`HOME=/var/root`，root 运行；203-215 行 `pgrep` 等待 15 s 否则安装失败。不碰 Docker。 | — | **root LaunchDaemon 与 D2 冲突**（§3、§7 第 6 条）：原生 Agent 需要以登录用户运行，即 integration TODO R1 的 LaunchAgent 改造要一并做 |
| Windows 组件模型 | [bucky_project.yaml:206-221](../src/bucky_project.yaml#L206-L221) 两组件 `BuckyOSApp`（bundle）与 `buckyos`（`system_service: true`）；[make_local_win_installer.py:723-729](../src/publish/make_local_win_installer.py#L723-L729) 只对 `system_service` 组件渲染 `SERVICE_PREREQUISITES`。 | — | 已有组件开关，无容器档位可作为构建开关/组件变体而不是新安装器（TODO 判断成立） |
| Windows Docker 检查 | [installer.nsi.in:88-102](../src/publish/win_pkg/installer.nsi.in#L88-L102) `CheckDockerDesktopForUser`：`docker --version` 失败 code 1、`docker version` 失败 code 2；[380-411](../src/publish/win_pkg/installer.nsi.in#L380-L411) 交互 Abort/Retry/Ignore（Ignore 只打开下载页再回到检查，**没有跳过出口**），静默 ErrorLevel 20/21。同一块还查 VC++ 运行库（30）、停旧实例、端口 3180/80/443（40）。 | (a) | 把 Docker 块从模板拆出按产品模式渲染；保留 VC++/端口检查；同步 `tests/test_make_local_win_installer.py:196` |
| Windows 不查 wslc | 安装器与 ps1 脚本中无 `wslc` / `BUCKYOS_CONTAINER_RUNTIME`；运行时却优先用 wslc（G4）。 | (c) | 见 G4：运行时按能力信号关闭自动探测 |
| Windows 启动方式 | [buckyos_postinstall.ps1:7-24](../src/publish/win_pkg/scripts/buckyos_postinstall.ps1#L7-L24)：每分钟一次的计划任务 `BuckyOSNodeDaemonKeepAlive` + HKCU Run 键，经 [node_daemon_loader.vbs:22-34](../src/publish/win_pkg/scripts/node_daemon_loader.vbs#L22-L34) 以安装用户身份起 `node_daemon.exe --enable_active`；不是 Windows 服务（`--as_win_srv` 存在但未用）。**不传 `--desktop_daemon`**（G5）。 | — | Windows 已是"登录用户、交互会话"形态，符合 integration TODO R1；只差标志/信号 |
| 首启镜像预加载 | [paios_images.rs:947-1007](../src/kernel/node_daemon/src/paios_images.rs#L947-L1007) `spawn_preload`：清单不存在或非 pinned 直接返回；pinned 时后台 `docker image inspect` → 校验 tar → `docker load` → ExtTool seeder 容器，失败按 5 s 起指数退避到 300 s **无限重试**并每次 `warn!`。调用点在激活之前（node_daemon.rs:1750）。两种模式都不 `pull`。 | (c) | 无容器发行包不放 `paios_images.json`，且 `spawn_preload` 按能力信号短路 |
| 停止 / 卸载 / 黑盒停止 | `stop_osx.sh:17-30`、`rootfs/bin/stop.py:58-96`（Windows 直接 return）、`rootfs/bin/stop.ps1:22-38`、`node_control.rs:2440-2480 blackbox_stop`（默认策略 `IncludeManagedContainers`）都调 `docker ps/rm`，失败只打印。 | (b) | 无 docker 时不记错即可 |
| control-panel 容器概览 | zone_mgr.rs:900-1000 直接 `docker`。 | (b) | UI 按能力信号隐藏 |
| Desktop 外壳 | BuckyOS.app / buckyosapp.exe 来自外部仓 `BuckyOSApp`（[make_local_pkg.py:41,934-947](../src/publish/make_local_pkg.py#L41)），本仓无法核对其 Docker 探测与启动参数。 | 未定 | 需到 BuckyOSApp 仓核对 |

**发布链路耦合**（R5 第二条）：[build-all.yml:144-239](../.github/workflows/build-all.yml#L144-L239) 组装 `paios_images.json` 并以 `paios_images` 输入传给三平台 job；[build-macos.yml:85-92](../.github/workflows/build-macos.yml#L85-L92) / [build-windows.yml:100-108](../.github/workflows/build-windows.yml#L100-L108) 只拷清单、不下载 tar；tar 由 CD 打包机用 sn-business 的 `stage_paios_images.py` 从 R2 取（[impl_buckyos_packaging.md:359-360](../doc/CI/impl_buckyos_packaging.md#L359-L360)）。`make_local_pkg.py --paios-images {none,local}` 默认 `none`，`verify_paios_images(required=False)` 清单缺失即跳过。结论：**安装包产出本身不依赖镜像**，解耦只需 CD 对无容器变体不 stage tar、不传 `--require-paios-images`。体积参考（testdata 清单）：aios amd64 ≈ 437 MB、aarch64 ≈ 426 MB、exttool ≈ 98 MB，是无容器包可省掉的部分。

**平台矩阵（R0 第三条，源码推断）**：CI 目标 `macos-15` / `macos-15-intel`、`windows-11-arm` / `windows-2025`（[build-macos.yml:28](../.github/workflows/build-macos.yml#L28)、[build-windows.yml:28](../.github/workflows/build-windows.yml#L28)），即 macOS arm64+x86_64、Windows x86_64+arm64；Deno 在 CI 为 v2.9.2（workflow `DENO_VERSION`），本机构建的 `distribution.json` 为 2.9.4（容器 `publish/aios/buckyos:35-40` 要求主版本一致，次版本可不同）。安装账号：macOS 需管理员（pkg 装 LaunchDaemon），Windows `RequestExecutionLevel admin`（installer.nsi.in:25）。PowerShell：`llm_bash` 优先 PATH 上的 `pwsh.exe`，否则 Windows PowerShell 5.1（[llm_bash.rs:1045-1063](../src/frame/agent_tool/src/llm_bash.rs#L1045-L1063)）。最低 OS 版本未在仓内声明。

### 4.2 工具基线：cmd_skills 实际依赖什么（TODO §4 清单）

来源：12 份 [cmd_skills](../src/apps/jarvis_agent/agent/cmd_skills) 全文、behaviors、runtime 代码、`build_aios` 包列表。**`requires` 没有被机器判读**：只在 `behavior.rs:304` 解析成字串、`xagent_cli.rs:1312` 打印；绑定时探测的是 `cfg.runtime.requirement.tools`（[config.rs:716-721](../src/frame/lib_opendan/src/protocol/config.rs#L716-L721)），而整个 lib_opendan/opendan 没有任何地方写入它 → ffmpeg/deno/ImageMagick 从未被探测过（TODO §4 末段的判断成立）。手册体系预设 AIGC 命令只存在于 `container` runtime（plan.toml:41、goal.toml:20、analyze_intent.toml:82 让模型 `--runtime container`），原生 Agent 后这个预设要反过来。

容器预装 vs 手册实际使用（`build_aios:19-24` RUNTIME_PACKAGES + Node v24 + uv + Deno v2.9.2；ExtTool 卷 = pandoc + uv/deno 缓存 + 可选 FreeCAD）：

| 二进制 | 镜像有 | 手册/代码直接用 | 判定 |
| --- | --- | --- | --- |
| ffmpeg / ffprobe（libx264、aac、libmp3lame、libass、concat、zoompan、tpad、amix、mov_text） | 是 | aigc_audio、aigc_video、`llm_understand_media`（视频抽帧 [llm_understand_media.rs:1577-1630](../src/frame/agent_tool/src/llm_understand_media.rs#L1577-L1630)；非 jpeg/png/webp/bmp 图片转 PNG [image_fit.rs:398-440](../src/frame/agent_tool/src/llm_understand_media/image_fit.rs#L398-L440)），裸名走 PATH | **必带** |
| ImageMagick `identify` `convert` `compare` | 是 | aigc_image:25-51（aigc_audio 也用 `convert`） | **必带**；Windows `convert` 与 `System32\convert.exe` 同名，改手册为 IM7 `magick identify/convert/compare` 更稳 |
| jq | 是 | aiworkspace_run:19-20；容器版 `buckyos` 启动器自身 | 带，或改手册用 `buckyos`/Deno 解析 |
| deno | 是 | 全部 buckyos-agent 命令、`buckyos` CLI、aiws 程序宿主、派生脚本 | 宿主已有 `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno[.exe]`，复用 |
| websdk Tool 分发 `libexec/buckyos-tool/{cli,dist,distribution.json}` | 容器只读挂宿主的 | `buckyos`、`aicc-tool` 的 `deno.json` 都指向 `/opt/buckyos/libexec/buckyos-tool/dist/node.mjs` | 宿主已有；需把绝对路径改成相对 `$BUCKYOS_ROOT`（`BUCKYOS_AICC_TOOL_ROOT` 已可覆盖安装目录） |
| 22 个 buckyos-agent 命令（gen_image、text_to_speech、resume_aicc_task、materialize_resource …） | `/usr/local/bin/<cmd>` → `aicc-tool` sh 启动器（`basename "$0"` 分派，[aicc-tool:4-31](../publish/aios/aicc-tool#L4-L31)，无 `--allow-run`） | aigc_* 全部 | 需要原生入口：Windows `.cmd`/`.ps1` shim 或 `deno compile`；macOS 可继续 sh |
| bash / sh | 是 | `llm_bash` 非 Windows 固定 `/bin/bash`；[bin_overlay.rs:156-164,185-190](../src/frame/lib_opendan/src/runtime/bin_overlay.rs#L156-L164) 生成的 session helper（`agent-session`、`llm_understand_media`、墓碑）全是 `#!/bin/sh` 脚本，PATH 用 `':'` 分割（:212）；plan.toml:21 的 heredoc 语义 | **Windows 缺口**：`llm_bash` 已走 PowerShell（[llm_bash.rs:137-144,1065-1090](../src/frame/agent_tool/src/llm_bash.rs#L137-L144)，测试 2078-2099 期望 `.ps1` shim），但 bin_overlay 没有 Windows 分支；macOS 自带 bash 3.2 够用 |
| xagent / agent_tool | 与 opendan 同目录的符号链接 | `agent-session`、`llm_understand_media`、`agent_tool xllm` | 必须与 `opendan` 同目录（`beside_exe`，opendan/main.rs:122-126）；Windows 用 `.exe` 复制 |
| git | 是 | buckyos_admin:242（核对运行版本源码） | 可选；Windows 带便携版或要求用户装 |
| CJK 字体 | **无**（镜像无任何 fonts-* 包，DejaVu 来自依赖链） | aigc_video:69 烧录字幕 | 桌面系统自带 CJK 字体（微软雅黑/苹方），但静态 ffmpeg 的 libass 需 fontconfig 找到系统字体目录；或只承诺软字幕 `mov_text` |
| sha256sum、`env -S` shebang | coreutils | buckyos_admin:237、aigc_image:58 | Windows/macOS 无 `sha256sum`；`env -S` Windows 无效；改手册 |
| python3 / uv、node/npm、poppler-utils、tesseract-ocr、pandoc、curl、rsync（tmux 已于 2026-10-25 从镜像移除） | 是 | **无任何手册直接调用**（python 只以禁令出现；OCR 走 AICC `ocr_image`；PDF 直接送模型） | 不带。与 TODO §3 "自有工具不依赖 uv/Python" 裁决一致，且 Script App 的 uv/Deno 依赖留在容器 |
| docker / cargo / pnpm | 否 | 手册标为宿主开发工作 | 不带（设计如此） |

**建议的最小私有工具包（D2）**：ffmpeg + ffprobe（含 libass/lame/x264 的 full 构建）、ImageMagick 7（`magick` + 三个别名）、jq、buckyos-agent 命令的原生入口、websdk Tool（已有）+ Deno（已有）、xagent/agent_tool/opendan（已有，Windows 改复制）；可选 git；字体按 §7 决定。安装位置 `$BUCKYOS_ROOT/libexec/` 或 `bin/opendan/tools/`，由 node_daemon 启动原生 Agent 时注入进程级 PATH（`runtime/mod.rs:104-111,812-818` 已有 session bin → skill tools → agent tools 的 PATH 分层，可再加一层"产品工具包"），不改全局 PATH、不覆盖用户已有版本（TODO §3 原则）。

**cmd_skills / behaviors 需要适配的点（R3 第四条）**：12 份手册的 `requires` 全部写的是 `aios agent container` / `/usr/local/bin/buckyos` 路径；`buckyos_admin.md:20,229` 写死 `http://host.docker.internal:3180`；`aigc_*` 的 `/opt/buckyos/bin/opendan/buckyos-agent/...` 绝对路径与 `deno run --config /opt/...`；`aigc_image:58-71` 的 `#!/usr/bin/env -S` + heredoc + `chmod +x`；`aigc_video:55-62` 的 shell 变量代入 filter 字串；`plan.toml:21` 的 bash heredoc 说明；`buckyos_admin.md:54-55,147`、`xllm.md:97`、`opendan_agent_dev.md:12,89` 是仅有的正面写 Windows/PowerShell 的段落。runtime_config.ts 的 `ownerUserId` 默认 `devtest`、网关默认 `host.docker.internal`（[runtime_config.ts:27-53](../src/tools/buckyos-agent/lib/runtime_config.ts#L27-L53)）只在没有 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 时生效，原生 Agent 的 session 会注入该 token，故不是正式身份问题，但默认值应改成报错而非 `devtest`。

**AICC 边界（R4 第二条）**：aigc_* 的生成/理解/OCR 全部是 AICC 调用，私有工具包只覆盖本地转码、抽帧、拼接与检查；"工具齐全"不等于"离线 AI 能力齐全"，手册与安装后诊断要分开报告 provider 可用性。

## 5. 方案选择与代价（实施顺序的依据）

| 步骤 | 内容 | 方案与代价 | 达成 |
| --- | --- | --- | --- |
| ① 能力信号（G1/G4/G5/G9） | 激活时写真实容器能力；node_daemon 启动按信号决定是否触碰 docker/wslc | **方案 A（建议）**：保留 `support_container` 语义="本机可运行容器"，激活向导按产品模式（Desktop 无容器包 → false）写入，不做运行时探测；node_daemon 读到 false 时跳过 `spawn_preload`、禁用 wslc 自动发现、AppLoader 一律 `NotSupport`。另在 `DeviceDocument.capbilities` 增加 `agent_native`（i64=1）表示"允许 owner Agent 原生运行"，由 Desktop 安装包/激活写入。**方案 B**：运行时探测 docker/wslc 决定 `support_container`。B 会让同一台机器装 WSL 后行为突变，且与"不自动落到 wslc"冲突，不建议。代价：active_lib.ts / active_server.rs / websdk provision / name-lib 四处 + node_daemon 读取。 | D1/D2/D3 共用 |
| ② 调度器失败隔离（G6/G7） | `schedule_spec_change` 按 spec 捕获 `No suitable node`，记 `ServiceSpecState::Unschedulable`（或 instance 级状态）并继续其他 spec；control-panel 显示原因 | 必须先做，否则 ③ 的任何失败都表现为整 Zone 调度停摆。顺带决定 systest 的去留（G7）。代价：scheduler.rs + system_config_agent.rs 的状态落库 + control-panel 读。 | D1/D2/D3 |
| ③ 原生 owner Agent（G2） | 调度器：Agent spec 的容器要求改为"容器 **或** 节点有 `agent_native` 且 spec.owner 为节点 owner"；`build_app_service_config` 的 `!support_container` 分支接受 `agent` 包。AppLoader：新增 `RuntimeType::NativeAgent`，deploy = ensure_pkg_installed（agent + skills）；start = 以 `build_runtime_env` 同一套 env 启动 `bin/opendan/opendan --appid --agent-bin --service-port`，`BUCKYOS_HOST_GATEWAY=127.0.0.1`，记录 pid/epoch；status = 进程存活 + 服务端口健康；stop = 终止进程树（Windows Job Object / Unix 进程组，可复用 node_executor spawn.rs 的做法）。opendan：去掉 `/opt/buckyos` 兜底、Windows 下 `xagent`/`agent_tool` 复制而非符号链接。 | 两层都要改但都是"加分支"，不动现有容器路径。风险在进程生命周期（§3）与 Windows 路径（`%APPDATA%\buckyos`、空格/中文路径、`#!/bin/sh` helper，见 §4 工具基线）。 | D2 核心 |
| ④ 安装器与预装（§4） | Windows 组件模型新增"无容器"变体；macOS 去掉 preinstall 的 `docker info` 与 make_local_osx_pkg 的校验；发行包不带 `bin/paios-images`；Desktop 的 `system/install_settings` 不预装 systest；安装器/启动脚本传 `--desktop_daemon`（若 §7 决定如此） | 见 §4 合并后的清单。 | D2 可安装 |
| ⑤ 私有工具包与 cmd_skills（R3/R4） | 见 §4 工具基线 | 见 §4。 | D2 默认任务 |
| ⑥ G3 | script host app 原生生命周期 + 许可清单 | 后补。 | D1 |

与相关 TODO 的边界：① ② ④ 是本研究独有；③ 的 AppLoader 子进程与 [Node Agent Runtime](node-agent-runtime-todo.md) / [Runtime 集成](agent-runtime-session-workspace-integration-todo.md) 共享"Desktop node_daemon 以登录用户身份运行"的前提，但不要求先完成双端 runtime 改造；⑤ 与 [Standard / Target Runtime](opendan-primary-runtime-tools-todo.md) T1/T2 的工具清单是同一份，应只维护一份。

## 6. 验证方式（R6 纵向链路的具体检查点）

平台实测尚未进行；下列是实施后每步的最小验证，Windows 与 macOS 各跑一遍。

1. 干净 VM（无 Docker/WSL/wslc/Deno）：安装 → 激活 → 查 `etc/node_identity.json` 对应 DeviceDocument `support_container=false` 且 `capbilities.agent_native=1`；node_daemon 日志无 `docker`/`wslc` 调用（grep `paios_images`、`docker run`）。
2. 建 Agent：`users/<owner>/agents/<id>/spec` 出现后 5 秒内 `nodes/ood1/config` 含该实例且 runtime 为 NativeAgent；调度器日志无 `No suitable node`；systest 预装任务按 §7 决定的方式结束（不预装或明确 Unschedulable）。
3. 原生 opendan 进程以登录用户运行；`BUCKYOS_APP_TOKEN` 换到 verify-hub 会话成功；MessageHub 对话有回复；重启 node_daemon 后进程被重新拉起且 Session 可继续。
4. 工具：Agent 在 PowerShell / zsh 下调用 `buckyos`、`xllm`、ffmpeg/ffprobe、图像工具，路径含空格与中文；完成一个 aigc 媒体任务并以 obj_id 交付。
5. 拒绝路径：安装 filebrowser（docker 包）→ control-panel 显示"本机不支持容器应用"而非一直 installing；装了 WSL 的机器上再次启动不落到 wslc。
6. AIWS hosted：许愿格任务由原生 Agent 领取并提交结果。

## 7. 未决决策（需用户拍板）

1. **能力信号落点**：§5 方案 A（`support_container=false` + `capbilities.agent_native`）还是改用 `device_type="desktop"` 并放开调度器类型过滤。后者会牵动 RBAC、网关路由与 NodeType 语义，不建议。
2. **systest 预装**：Desktop 发行不预装，还是修 author 放行规则（`did:bns:buckyos` 应否等同 `did:web:buckyos.*`）。
3. **`--desktop_daemon`**：由安装器/登录项固定传入（使 Node Executor A-desktop 与 `owner_agents` 种子生效），还是改为从能力信号派生。当前发行包都不传。
4. **升级/迁移**：已装 Docker 版 Desktop 的用户升级到无容器版时，已有 Agent 实例卷、Session 绑定（§3）如何处理；按"不写兼容"原则是否接受重建。
5. **Windows 路径根**：`%APPDATA%\buckyos` 作为 Agent 数据/工具根是否接受（漫游配置文件可能同步大体积数据），或改用 `%LOCALAPPDATA%`。
6. **macOS 进程形态**：integration TODO R1 决定 Desktop node_daemon 改为 LaunchAgent/登录项但"安装包改动不在本次范围"；原生 Agent 以 root LaunchDaemon 身份运行是不可接受的（owner home、TCC），因此 D2 在 macOS 上**依赖**这项安装包改动一并完成。

## 8. 对 TODO 的修订建议

- §2 表格 D3 行：补"需先有非 OOD 设备加入流程（G8）"，并标注 D3 的"仅系统服务"同样被 G6/G7 阻塞。
- §5.1 增加 G6–G9；G2 补第三层（`build_app_service_config`）。
- §6 R6 默认顺序改为 §5 的 ①–⑥（在 G2 之前插入调度器失败隔离）。
- §3 "内置系统服务逐项排查隐藏容器依赖"：已核实为无（§2 次要点），可勾掉。
