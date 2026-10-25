# BuckyOS Desktop 无 Docker 安装可行性分析 TODO

日期：2026-10-25（同日按源码 review 修订，见 §9）

状态：**源码级可行性研究已完成，见 [研究报告](buckyos-desktop-without-docker-research.md)（2026-10-25）：建议 D2 先行、D1 后补、D3 不独立立项；实施顺序见报告 §5；待用户拍板项见报告 §7。平台实测与代码实施尚未开始。**

源码参考：`33b3ca39e`。§5 的 GAP 已在该版本逐条核对过行号；后续分析应以届时源码为准重新确认。研究报告新增 G6–G9，并补 G2 第三层（`build_app_service_config`）。

## 1. 核心问题

**把功能限制在什么程度的时候，允许在无 Docker 环境下安装 BuckyOS Desktop？**

本研究要回答的不是"能否把全部功能原生化"，而是：在 Windows / macOS 没有 Docker、OrbStack、WSL 的机器上，Desktop 保留哪一档能力就算"可以安装"，这一档需要改哪些代码，放弃的能力如何向用户明确表达。目标是降低容器引擎、虚拟化和镜像准备造成的安装门槛；提升幅度需要后续用数据验证，不预设数值。

研究产出三件事：

1. 一个**限制档位**的裁决（§2），以及"允许安装"的判定标准。
2. 该档位下必须打通的**代码缺口清单**（§5）和实施顺序（§6 R6）。
3. 该档位的**验收矩阵**（§7）与留给深度分析的风险问题（§8）。

本 TODO 不冻结新增协议、配置字段或安装包布局。相关任务：[OpenDAN Standard / Target Runtime](opendan-primary-runtime-tools-todo.md)、[Node Agent Runtime](node-agent-runtime-todo.md)、[Runtime / Session / Workspace 集成](agent-runtime-session-workspace-integration-todo.md)。这些任务覆盖更广的执行模型；本研究只取其中无 Docker Desktop 的必要依赖，不把完整双端 runtime 改造变成安装目标的前置条件。

## 2. 候选限制档位（分析框架）

下表是分析用的档位划分，不是已批准的产品定义。每一档的"初步判断"来自源码线索（§5），不是实测结论。

| 档位 | 保留能力 | 放弃能力 | 必须打通的缺口（§5 编号） | 初步判断 |
| --- | --- | --- | --- | --- |
| **D0 不限制** | 与容器节点等价：任意 App、任意 Agent | 无 | 需要容器引擎 | 即现状；不在本研究范围 |
| **D1 受限工作（目标档位）** | 内置系统服务；Zone owner 的 OpenDAN 原生运行；产品允许的 script host app；内置 cmd_skills 承诺的常见多媒体；AIWS hosted 模式 | 第三方 App；需要容器的其他 Agent / 应用；ExtTool | G1、G2、G3、G4、G5 + 私有工具包（§4） | 两层运行时改造 + 工具交付，是本研究的主线 |
| **D2 原生 Agent、无应用** | 内置系统服务；owner OpenDAN 原生；常见多媒体；AIWS hosted | D1 放弃项 + 全部 AppLoader 应用（含 script host app） | G1、G2、G4、G5 + 私有工具包 | 比 D1 少 G3；若 script host app 的产品需求不急，可作为第一步 |
| **D3 仅系统服务** | 内置系统服务；Desktop 外壳；激活与身份；访问 Zone 内其他节点上的 Agent | 本机不运行任何 Agent 和应用 | G1、G4、G5 + 安装器改造（§5 安装硬前置）+ G6/G7；"加入已有 Zone"还需 G8 的设备加入流程 | 代码改动最小；但单机用户没有 Agent，产品价值取决于 Zone 是否已有容器节点。研究结论：Desktop 今天只能作为 Zone 的 OOD 激活，D3 不单独立项 |

**"允许安装"的判定标准**（每档相同）：

- 干净机器上安装成功、激活成功，不出现隐式的容器下载、启动或探测失败。
- 该档位保留的能力至少完成一个默认任务（D1/D2：owner Agent 完成一个媒体任务并交付；D3：Desktop 能用 Zone 内其他节点的 Agent）。
- 放弃的能力在所有入口（UI、API/CLI、调度）被明确拒绝并说明原因，不进入反复调度或静默切换为宿主执行。
- 用户能区分"本机不支持"与"安装/权限/服务故障"。

研究顺序建议：先按 D1 做依赖链与缺口分析；若 G3（script host app 原生化）代价过大，退到 D2 作为首个可交付档位；D3 只作为底线对照，不单独立项。

## 3. 已确定的产品方向与边界

| 对象 | 无 Docker Desktop 的研究目标 |
| --- | --- |
| 内置系统服务 | 原生运行，能够完成安装、激活、身份配置、服务发现和默认功能闭环；逐项排查隐藏容器依赖。 |
| Zone owner 的 OpenDAN | 作为产品组件原生运行，以本机为默认标准工作环境；具备 Agent Tool 和正确的 BuckyOS / AICC 访问上下文。`debug_jarvis.sh` 与 `opendan --dev` 已有原生调试模式可复用，但不能直接作为生产生命周期方案。 |
| AppLoader 管理的应用 | 本 Desktop 仅支持产品允许的 script host app，不允许安装第三方 App；系统组件和原生 OpenDAN 的 Rust 可执行文件不受「脚本应用」这一形式限制。 |
| 其他需要容器的 Agent / 应用 | 保留在符合条件的容器节点上使用 paios 的路径；本 Desktop 不承载这些实例。没有合适节点时明确不可部署，不静默切换为宿主执行。 |
| 自有脚本工具 | **裁决（本文）**：按 TS + websdk / Deno 方向交付，不把 uv / Python 作为这些工具的运行前置条件；不以全 Rust 重写为本项目条件。这是用户既定原则（Agent 工具优先 TS + websdk，源码随 runtime 给 Agent 读改），此前未写进任何关联 TODO，此处记为裁决。注意 [build_aios](../build_aios) 仍为 Script app 安装 uv 与 Python，这是容器镜像的事，不受本裁决影响。 |
| Windows shell | 使用实际 PowerShell 环境和匹配的命令示例，不要求用户安装 Bash、Git Bash 或 WSL。**tmux 已决定全线移除**（2026-10-25 用户裁决）：正式的 Agent Runtime（Node Executor 的进程 / 任务管理）落地后，原来基于 tmux 的执行隔离已无意义，所有平台都不再依赖 tmux，自然也不新增 Windows tmux 仿制实现。tmux runtime（`agent_tool` 的 `runtime/tmux.rs`）与 [runtime/mod.rs](../src/frame/lib_opendan/src/runtime/mod.rs) 的 Session tmux 绑定已于 2026-10-25 删除，`kind: tmux` 为配置错误；[Standard / Target Runtime TODO](opendan-primary-runtime-tools-todo.md) §1.1 等处的 tmux 口径已同日回写。 |
| 容器替代方案 | Docker Desktop、OrbStack、WSL 以及 node_daemon 已有的 **wslc** 路径（§5 G4）都不是用户前置条件。"无 Docker"同时意味着不自动落到 wslc。 |
| 扩展环境 | ExtTool 为可选扩展，默认无需支持；内置 cmd_skills 承诺的常见多媒体能力仍属于默认交付范围。 |
| 系统已有环境 | 使用产品私有 runtime / 工具，不覆盖用户已安装的 Deno、Node、Python 等环境，不依赖修改全局 PATH。 |

"脚本形式"和"产品允许安装"是两个不同条件，第三方脚本包不能因采用 Deno 就自动获得安装资格。Agent 临时生成的脚本、AIWS 程序、用户已有的软件与持久安装的第三方 App 也不是同一对象，具体执行与安装边界留待研究明确。

本地禁装第三方 App 不自动等同于整个 Zone 禁止在其他节点安装；远端安装策略、入口和权限需单独确认。

## 4. 标准工具基线：从 cmd_skills 落到可交付清单

工具能力以[内置 cmd_skills](../src/apps/jarvis_agent/agent/cmd_skills)及实际调用链为依据。此前「完整对齐 paios 预装工具」的要求应落实为逐项对照清单：标明原生交付、平台等价入口、可选扩展与明确不支持项，不能只安装 Deno 后就宣称对齐，也不能机械复制 Linux 系统包列表。aios 镜像当前安装的系统工具见 [build_aios](../build_aios) 开头的包列表（jq、ffmpeg、imagemagick、poppler-utils、tesseract-ocr、python3 / uv；pandoc 走 ExtTool）。

| 能力 | 基线要求 / 待核对内容 |
| --- | --- |
| Agent 核心工具 | 核对 `agent-session`、`xagent`、`agent_tool`、`xllm`、`llm_understand_media` 等实际入口，保障 Session 上下文、命令发现和结果交付。 |
| BuckyOS / AICC | 复用私有 Deno 与 SDK/CLI 分发，补齐 `buckyos`、AICC 和媒体脚本工具、导入配置及平台 launcher；使用 Agent / AppService 身份与会话授权。 |
| 视频 / 音频分析与处理 | 默认交付 FFmpeg、ffprobe。视频理解已有抽帧/探测依赖；音频转码、视频裁剪/拼接、滤镜、音轨与字幕操作按手册形成真实样例。 |
| 图像处理 | 当前 [aigc_image.md](../src/apps/jarvis_agent/agent/cmd_skills/aigc_image.md) 使用 ImageMagick 的 `convert` / `identify` / `compare`（`aigc_audio.md` 也用到 `convert`）；明确平台发行版本、规范入口及同名命令冲突处理（Windows 自带 `convert.exe`），或同步改写手册采用已验证的等价实现。 |
| 编解码器、字体与数据 | 验证手册实际需要的编码器、滤镜、字体和附属文件，不能用 `--version` 成功替代功能可用。[aigc_video.md](../src/apps/jarvis_agent/agent/cmd_skills/aigc_video.md) 已记录容器只有 DejaVu、无 CJK 字体，中文烧录字幕不可用；原生交付要决定随包带 CJK 字体还是只承诺软字幕。 |
| AIWS | 覆盖运行领取、素材读取、Deno 程序执行、结果提交、预览及交付；[aiworkspace_run.md](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_run.md) 示例使用 `jq`，应决定随包交付或改写示例，不能默认为无此依赖。 |
| 文档 / OCR | 确认普通用户默认支持的文档范围与 AICC 能力。现有 `ocr_image` 调用 AICC，不能据此承诺本地离线 OCR；Poppler / Tesseract / Pandoc 是否需要交付由实际能力承诺决定。 |
| 开发与重型扩展 | 区分普通用户默认能力与开发手册中的 Cargo、Docker、其他语言工具链及 ExtTool 前提；逐项声明差异，不要求默认安装所有开发环境。 |

基线记录至少包括：能力、对应 skill / 调用点、可执行入口、OS/架构、版本或发布约束、传递依赖、获取/升级方式和功能验收样例。当前 skill 的 `requires` 是一行说明文本（且多数写的是"agent container: /usr/local/bin/buckyos"这类容器路径），不应假定已有机器可执行的完整依赖清单。

"自有工具不依赖 uv"不代表可以删除仓库构建工具、现有容器入口或其他应用对 Python 的依赖；本研究首先收敛最终用户的默认运行依赖。

## 5. 已发现的实现 GAP 与复查入口

以下已在 `33b3ca39e` 核对过行号，但仍是源码线索，不是平台实测结论。前五项（G1–G5）是决定档位可行性的关键缺口，§2 的表格按它们编号。

### 5.1 关键缺口（决定档位）

| 编号 | 缺口 | 源码证据 | 影响档位 |
| --- | --- | --- | --- |
| **G1** | **激活把 `support_container` 写死为 true**。激活向导 [active_lib.ts:306](../src/kernel/node_active/active_lib.ts#L306) 固定输出 `support_container: true`，[active_server.rs:360](../src/kernel/node_daemon/src/active_server.rs#L360) 原样写进 DeviceDocument。调度器 [scheduler.rs:1221](../src/kernel/scheduler/src/scheduler.rs#L1221)、[scheduler/app.rs:56](../src/kernel/scheduler/src/app.rs#L56) 与 AppLoader [app_loader.rs:868](../src/kernel/node_daemon/src/app_loader.rs#L868) 都以此判断。 | 无 Docker 的 Desktop 激活后被当成容器节点，调度器照样往上放容器 App / Agent，到运行时才失败，正是 §7 要避免的"循环失败"。devnet 工具 [zone.rs:1068](../../buckyos-base/src/name-lib/src/zone.rs#L1068) 同样写死（优先级低）。 | D1 D2 D3 全部 |
| **G2** | **Agent 运行类型没有原生分支，且是三层**。AppLoader [app_loader.rs:815-822](../src/kernel/node_daemon/src/app_loader.rs#L815-L822) 对 `AppType::Agent` 只有"支持容器则 `RuntimeType::Agent`，否则 NotSupport"；调度器 [system_config_agent.rs:85](../src/kernel/scheduler/src/system_config_agent.rs#L85) 无条件 `need_container = is_agent`，非 Agent 应用的放行又只看 author 字符串是否为 `did:web:buckyos.*`；实例化 [app.rs:50-83](../src/kernel/scheduler/src/app.rs#L50-L83) 在 `!support_container` 分支只接受 `<arch>_<os>_app` / `script` 包，Agent 包只有 `agent` 键。研究核实：进程契约（`build_runtime_env` 的 env + 设备私钥签的 `BUCKYOS_APP_TOKEN`）与 Docker 无关，身份不需要新方案。 | "owner 原生 OpenDAN"是新路径而不是放开现有机制：调度器要允许 owner Agent 落到非容器节点，AppLoader 要有原生 Agent 的 deploy/start/status/stop。三层缺一不可。 | D1 D2 |
| **G3** | **Desktop 本地应用链路名义原生、实际进容器**。`--desktop_daemon` 的 `desktop_daemon_main`（[node_daemon.rs:1151](../src/kernel/node_daemon/src/node_daemon.rs#L1151)）读 `applist.json` 走 [local_app_mgr.rs](../src/kernel/node_daemon/src/local_app_mgr.rs)（注释写"本地 app 不支持 docker"），但 `is_local_app()` 选到 HostScript 后，deploy 在 [app_loader.rs:417-423](../src/kernel/node_daemon/src/app_loader.rs#L417-L423) 准备 worker 镜像与 ExtTool volume，`start_host_script`（[app_loader.rs:1169](../src/kernel/node_daemon/src/app_loader.rs#L1169)）仍调用 `run_worker_container`。停止、状态、卸载同样基于 Docker 标签。 | script host app 要真正原生需要一整套新的生命周期实现；`applist.json` / `LocalAppInstanceConfig` 是"产品允许清单"的现成落点；"首次启动不拉镜像"（R5）要改的就是这里的 deploy。 | D1 |
| **G4** | **Windows 已有 wslc 容器路径**。[wslc.rs](../src/kernel/node_daemon/src/wslc.rs) 按 `BUCKYOS_CONTAINER_RUNTIME` 或自动探测 `wslc.exe`，app_loader 多处（镜像列表、`host.wslc.internal`、运行参数）分支。Windows 安装器只查 docker CLI，不查 wslc。 | "无 Docker"的探测与拒绝必须同时覆盖 wslc，否则会悄悄落到 WSL；G1 修正后的 `support_container` 也要和 wslc 探测对齐。 | D1 D2 D3 |
| **G5** | **三个"Desktop / 节点身份"信号互不等价且没有一个表达"本机无容器能力"**：`--desktop_daemon` 标志（node_daemon 与 [node_executor_host.rs:249](../src/kernel/node_daemon/src/node_executor_host.rs#L249) 的 `desktop_build`）、DeviceDocument 的 `support_container` / `device_type`、Desktop 安装包本身。调度器与 Node Executor 用不同的角色 / 能力条件。**研究核实：macOS plist 与 Windows loader 都只传 `--enable_active`，没有任何发行包传 `--desktop_daemon`。** | 需要定义"无容器 Desktop 节点"由哪个信号表达、何时写入（激活还是安装），其余信号如何派生。 | D1 D2 D3 |
| **G6**（研究新增） | **一个不可调度 spec 让整轮调度失败并每 5 秒重试**：[scheduler.rs:860](../src/kernel/scheduler/src/scheduler.rs#L860) `reconcile_spec_instances(...)?`，[1151-1158](../src/kernel/scheduler/src/scheduler.rs#L1151-L1158) 无候选节点返回 Err；[system_config_agent.rs:2537-2543](../src/kernel/scheduler/src/system_config_agent.rs#L2537-L2543) 整轮放弃。 | 无容器 OOD 上只要有一个 `need_container` spec，所有调度动作停摆。这是"不循环失败"的真正阻塞点，必须先于 G2 修。 | D1 D2 D3 |
| **G7**（研究新增） | **预装的 `buckyos-systest` 需要容器**：[boot.template.toml](../src/rootfs/etc/scheduler/boot.template.toml) 预装 systest（script 包、author `did:bns:buckyos`），[system_config_agent.rs:86-100](../src/kernel/scheduler/src/system_config_agent.rs#L86-L100) 的免容器规则只认 `did:web:buckyos.*`。text-editor 是 Static web，不建 spec。 | 无容器节点上与 Agent 无关地触发 G6。Desktop 发行不预装，或修 author 放行规则。 | D1 D2 D3 |
| **G8**（研究新增） | **没有把 Desktop 作为非 OOD 设备加入已有 Zone 的路径**：激活只产出 `ood1`（[active_server.rs:352](../src/kernel/node_daemon/src/active_server.rs#L352)）；websdk `provision.ts` 只建 OOD 并写死 `support_container = true`（:412、:482）；非 OOD boot 分支存在但 `get_system_config_client(is_ood=false)` 是 `unimplemented!()`（[node_daemon.rs:1319-1328](../src/kernel/node_daemon/src/node_daemon.rs#L1319-L1328)）。 | D3 的"访问 Zone 内其他节点上的 Agent"没有前置流程。 | D3 |
| **G9**（研究新增） | **启动期镜像预加载**：[node_daemon.rs:1750](../src/kernel/node_daemon/src/node_daemon.rs#L1750) 在激活前就 `paios_images::spawn_preload()`；pinned 清单存在时 `docker load` + ExtTool seeder 容器，失败指数退避到 300 s 无限重试（[paios_images.rs:947-1007](../src/kernel/node_daemon/src/paios_images.rs#L947-L1007)）。发行包不 `pull`，只从 `bin/paios-images/` 的 tar 加载。 | 无容器包不得携带清单，且 preload 按能力信号短路；顺带省掉 ≈ 0.5 GB 镜像 tar。 | D1 D2 D3 |

### 5.2 其他缺口

| 领域 | 当前线索与需要解决的问题 |
| --- | --- |
| 应用许可与包匹配 | [app_install_planner.rs](../src/frame/control_panel/src/app_install_planner.rs)、[app_installer.rs](../src/frame/control_panel/src/app_installer.rs) 及 AppLoader 的包选择不等于产品安装许可。`Local` loader 也不等于可信内置应用；调度器的 author 字符串放行（G2）同理。 |
| macOS 安装硬前置 | [buckyos_preinstall](../src/publish/macos_pkg/scripts/buckyos_preinstall) 以 LaunchDaemon/root 身份检查 `docker info`；[make_local_osx_pkg.py](../src/publish/make_local_osx_pkg.py) 还校验安装脚本必须包含 Docker 检查片段，两处要一起调整。 |
| Windows 安装硬前置 | [installer.nsi.in](../src/publish/win_pkg/installer.nsi.in) 的 `CheckDockerDesktopForUser` 只对服务组件生效，由 [make_local_win_installer.py](../src/publish/make_local_win_installer.py) 以 `SERVICE_PREREQUISITES` 渲染；安装器已有组件模型，无 Docker 档位可以作为组件变体而不是新安装器。 |
| 发布与镜像耦合 | [build-all.yml](../.github/workflows/build-all.yml)、[build-macos.yml](../.github/workflows/build-macos.yml)、[build-windows.yml](../.github/workflows/build-windows.yml)、[bucky_project.yaml](../src/bucky_project.yaml) 存在 paios 镜像准备/分发链路，Desktop 发行目标需要单独梳理。 |
| 工具交付不完整 | [SDK 工具分发](../src/tools/build_sdk_tool_distribution.py) 已打包私有 Deno；但 [build_aios](../build_aios) 和 [aicc-tool launcher](../publish/aios/aicc-tool) 中仍有镜像专属的工具装配、固定路径与 shell 入口，不能假定宿主已全部具备。 |
| Windows 适配残留 | [llm_bash.rs](../src/frame/agent_tool/src/llm_bash.rs) 已识别 Windows / PowerShell（`native` 在 Windows 构建、`remote_node` 按目标 OS）；实际 GAP 在 [bin_overlay.rs](../src/frame/lib_opendan/src/runtime/bin_overlay.rs) 生成的 `#!/bin/sh` helper、[runtime/mod.rs](../src/frame/lib_opendan/src/runtime/mod.rs) 的工具探测，以及 [plan.toml](../src/apps/jarvis_agent/agent/behaviors/plan.toml) 等提示词的 Bash 语法。 |
| runtime 语义与能力选择 | [protocol/runtime.rs](../src/frame/lib_opendan/src/protocol/runtime.rs) 的内置 `container` / `host` 与 `native` / `remote_node` 绑定不是产品部署形态的一一对应；原生启动不会自动修正 Session 冻结、Workspace、工具路由或 AIGC 手册中的容器前提。 |
| 原生运行身份与服务访问 | [macOS postinstall](../src/publish/macos_pkg/scripts/buckyos_postinstall)、[Windows loader](../src/publish/win_pkg/scripts/node_daemon_loader.vbs)、[runtime_config.ts](../src/tools/buckyos-agent/lib/runtime_config.ts)（owner 默认 `devtest`）需要核对 OS 用户、HOME、gateway 与 token。调试模式的 localhost / devtest / trust-loopback 不能成为正式安装的身份方案。 |
| AIWS 执行模式 | [wish/exec.rs](../src/frame/aiworkspace/server/src/wish/exec.rs) 区分 `service` / `local` / `hosted`；[runner.rs:90-93](../src/frame/aiworkspace/server/src/wish/runner.rs#L90-L93) 的 service Deno 权限只有 read/write(output)/net/env，不含 `allow-run`。本机安装 FFmpeg 不代表所有 AIWS 程序都能启动它；D1/D2 的 AIWS 闭环默认走 hosted。 |

## 6. 研究任务与实施候选项

以下复选框均表示待办。先按 §2 的档位形成范围和风险结论，再确定实施顺序；不把候选方案视为已经批准的最终设计。

### R0：冻结档位与研究依据

- [x] 按 D1 列出必须运行的系统服务、owner Agent、许可 script host app 和普通用户场景，画出从安装到首次任务完成的依赖链；标出哪些环节只在 D1 需要（G3），哪些 D2/D3 也需要（G1、G4、G5）。→ 报告 §1、§2、§5（内置系统服务无隐藏容器依赖；只有 G3 是 D1 独有）。
- [x] 对照当前 paios、内置 cmd_skills、实际工具调用和新运行结构，产出工具/能力差异清单，明确每项由原生、可选扩展或远端节点承担。→ 报告 §4.2。
- [x] 明确支持的 Windows/macOS 版本、CPU 架构、PowerShell 版本与安装账号类型；分别记录已有实现、源码推断和未验证事项。→ 报告 §4.1 平台矩阵（源码推断，最低 OS 版本未声明）。
- [ ] 定义安装成功、激活成功、owner Agent ready、首个任务成功的指标及失败分类，记录当前基线；验证去除 Docker 前置后是否产生新的主要失败来源。→ 报告 §6 给出检查点；基线数据需平台实测。
- [x] 给出档位裁决：D1 可一次到位，还是 D2 先行、script host app 后补。→ 报告 §0：D2 先行、D1 后补、D3 不独立；待用户确认。

### R1：节点能力、调度与原生 AppLoader

- [ ] **G1 / G5**：定义"无容器 Desktop 节点"由哪个信号表达，激活时按真实探测或产品模式写入 `support_container`（含 wslc 探测，G4），而不是写死 true；核对调度器、AppLoader、Node Executor 对该信号的读取一致。
- [ ] **G2**：为 owner 原生 OpenDAN 设计调度器侧（`need_container` 对 owner Agent 的例外或新的能力声明）与 AppLoader 侧（原生 Agent 的 deploy/start/status/stop/restart/update/uninstall）的一致路径；明确包类型、入口、OS/架构匹配与 runtime 选择。
- [ ] **G3**（D1）：以 `applist.json` / `LocalAppInstanceConfig` 为产品允许清单的落点，设计真正不调用 Docker 的 script host app 生命周期，去除 worker image、ExtTool volume 和 Docker inspect 假设；研究如何可靠识别产品允许的包，不能仅信任自报类型或 author 字符串。
- [ ] 明确不支持应用的拒绝位置、原因和可恢复状态；有其他合适节点时遵循显式部署策略，没有时不反复调度或偷换为 host 执行。

### R2：原生进程生命周期、身份与恢复

- [ ] 明确系统 daemon、桌面登录用户和 owner OpenDAN 的运行身份、目录、授权与启动关系；分别定义开机、登录/注销、Desktop UI 关闭、节点停止和 Agent 停止的行为。
- [ ] 定义进程实例识别、健康检查、日志、退出状态、崩溃拉起和子进程清理；不能只记录 PID 或按进程名批量停止用户程序。
- [ ] 覆盖长任务、取消、升级替换、断连/重启后的恢复与未知结果；tmux 移除后全部平台复用 Node Executor 的操作记录、查询和取消能力，保留任务查询、结果归属和明确的中断语义，不宣称等价于 tmux 的进程存活保证。
- [ ] 装配正式 Agent / AppService 身份、Session 上下文、gateway、token 续期及权限失败处理；不借用 owner 的个人凭据或调试授权绕过服务鉴权。

### R3：私有工具包与平台适配

- [ ] 复用现有 Deno/SDK 分发，将自有 Agent 脚本、FFmpeg/ffprobe、图像工具、必要字体及资源纳入可重复构建的工具交付清单；核对动态库和架构完整性。
- [ ] 明确私有安装目录、版本选择、进程级 PATH、缓存及依赖解析规则，验证用户已有不同版本工具时仍调用配套版本，不污染全局环境。
- [ ] 修正固定 `/opt/...`、`host.docker.internal`、Unix launcher、`#!/bin/sh` helper 和 Bash 示例中的平台假设，按实际 OS/shell 提供入口；适配 PowerShell 参数引用、退出码、路径分隔符及 Windows 命令发现。
- [ ] 调整内置 cmd_skills / behaviors 的容器前提与平台示例（`requires` 中的容器路径、`convert`/`jq` 等依赖），按实际标准环境选择可执行步骤；核对 `ui_core`、`aigc_*`、`aiworkspace_*`、`buckyos_admin`、`opendan_agent_dev` 和相关开发手册。
- [ ] 定义工具包校验、升级/失败回退、版本诊断与功能自检；区分「工具已安装」「可执行」「服务可访问」「场景能完成」，不只检查文件存在。

### R4：默认多媒体与 AIWS 闭环

- [ ] 从现有媒体手册抽取图片处理、音视频探测/转码/裁剪/拼接、抽帧理解、字幕等最小样例，验证实际产物、编码器、字体（含 CJK 烧录字幕）及中文路径。
- [ ] 验证 BuckyOS/AICC 的理解、生成、任务轮询、下载及附件交付；模型/provider/网络依赖独立说明，不把私有工具齐全表述为离线 AI 能力齐全。
- [ ] 分别定义 AIWS `service` / `local` / `hosted` 的能力；优先评估由 hosted Agent 完成媒体处理的闭环，不以向所有脚本开放无限制子进程权限作为默认答案。
- [ ] 验证素材路径、Workspace、临时文件、结果提交、预览与最终交付；明确本地/服务端文件归属、大文件空间占用、取消与中间产物清理。

### R5：安装、激活、升级与发布

- [ ] 为无 Docker Desktop 梳理独立发行内容与前置检查，调整 macOS/Windows 安装器及其生成/校验脚本（优先利用 Windows 安装器已有的组件模型）；首次启动和激活流程同样不拉镜像、不调用 Docker、不自动落到 wslc。
- [ ] 解除该 Desktop 发行目标与 paios 镜像准备、清单和大体积镜像分发的强耦合；保留其他容器发行目标所需的构建与发布能力。
- [ ] 核对 Desktop 外壳、启动/停止脚本、诊断与卸载中的 Docker / wslc 探测和清理逻辑，区分硬前置、可选操作与无 Docker 时的正常行为。
- [ ] 研究工具包/原生二进制的签名、公证、系统运行库、权限提示及更新失败处理；安装结束后给出可操作的默认能力诊断。
- [ ] 定义重启、升级、卸载/重装的数据保留和残留进程处理；按当前开发阶段原则，不默认增加旧容器版本兼容层，必要迁移另列决策。

### R6：最小验证与实施拆分

- [ ] 先验证一条纵向链路：干净系统安装 → 激活 → 原生 owner OpenDAN → PowerShell/macOS shell 调工具 → AICC → 一个媒体任务 → AIWS 交付。
- [ ] 在该链路中验证许可脚本应用原生启动（D1）、第三方 App 拒绝、重启/取消和失败诊断，再按缺口拆分代码工作包与依赖关系。
- [x] **默认实施顺序**（研究报告 §5 修订）：① G1/G4/G5/G9 节点能力信号 → ② G6/G7 调度器单 spec 失败隔离 + 不可调度状态外显 + systest 预装处置 → ③ G2 原生 owner Agent（调度器 + 实例化 + AppLoader 子进程）+ `/opt/buckyos` 硬编码 → ④ 安装器变体与 Desktop 预装清单（§5.2，macOS 需同时改为 LaunchAgent）→ ⑤ 私有工具包与 cmd_skills 适配（R3/R4）→ ⑥ G3。①–④ 达到 D2 可安装，⑤ 达到 D2 默认任务，⑥ 达到 D1。
- [ ] 与 Standard / Target Runtime 任务明确共享部分和独立部分，避免改部署模式时静默改变已有 Session 的执行端、工作目录与恢复位置。
- [ ] 为后续改动列出源码、SDK/协议、UI、打包和 cmd_skills 的联动范围，以及自动检查与真实平台验收分工。

## 7. 后续实现的验收矩阵

这些是目标，当前均未完成验收。Windows 与 macOS 分别执行并记录 OS/架构、工具版本、运行身份、样例结果和未覆盖项。标注"D1"的行只在 D1 档位验收。

| 场景 | 通过标准 |
| --- | --- |
| 干净机器 | 无 Docker/OrbStack/WSL/wslc、无用户自备 Deno/uv/FFmpeg，仍可安装、激活并让 owner Agent 完成默认任务；不存在隐式容器下载、启动或探测。 |
| 已有用户环境 | 用户已有不同版本 Deno/Node/Python/媒体工具和自定义 PATH，产品使用配套工具，用户原有配置保持可用。 |
| 节点能力 | 激活后 DeviceDocument 的容器能力与本机真实情况一致；调度器不向本机放置需要容器的 App / Agent。 |
| 应用范围（D1） | 许可 script host app 完成原生生命周期；从各安装/更新入口尝试第三方 App 均按策略拒绝，修改包类型或 author 不能绕过。 |
| 容器能力缺失 | 需要容器的实例在本 Desktop 明确不可部署；有符合条件节点时按明确策略处理，无合适节点时不循环失败或原生兜底。 |
| Agent 工具与鉴权 | 直接命令、脚本及子进程均发现配套工具；AICC/系统服务按正确 Agent 身份访问，token 过期/权限不足有准确结果。 |
| 平台语义 | Windows PowerShell、macOS 实际 shell 下示例可执行；中文/空格路径、参数转发、退出码及 Windows 路径边界通过测试，无 Bash/tmux 前置。 |
| 常见多媒体 | 手册承诺的图片、音频、视频样例产物正确；编码器/字体可用，媒体理解和生成结果能交付用户，外部模型服务状态单独记录。 |
| AIWS | 支持模式下可领取任务、读取素材、加工并提交/预览结果；不支持的模式准确报告能力差异，不错误宣称已运行。 |
| 生命周期 | 重启、注销、休眠恢复、进程崩溃、取消、升级和卸载符合已定义语义；不误杀无关进程，不丢已提交产物，不重放结果未知的副作用。 |
| 故障与边界 | 缺工具、包损坏、磁盘不足、端口占用、网络/provider 不可用等有可定位结果；用户能区分安装问题、权限问题和服务问题。 |

## 8. 留给后续 Agent 的深度风险分析

以下仅列问题和要求，不预判风险等级，也不以原生调试成功代替生产可行性。

- [ ] **权限与隔离**：owner 身份与 OS 账号如何对应？原生 shell、文件工具、readonly/禁用 host 配置是否实际约束执行？系统 daemon（macOS LaunchDaemon root / Windows 服务）权限是否泄漏给 Agent 或脚本？**产出形式**：按"先能力后安全"的既定原则，这一条的产出是"已知风险"清单和产品告知文案，不是新的隔离、审批或沙箱机制；哪些限制能兑现、哪些需明确告知，分别列出。
- [ ] **应用许可边界**：第三方 App、skill 自带工具、依赖下载、Agent 临时脚本和 AIWS 程序如何区分？许可检查能否覆盖更新和直接 API 调用？Desktop 本地限制如何与 Zone 远端部署共存？
- [ ] **生命周期与持久性**：进程树、后台任务、Session 恢复和升级替换有什么平台差异？已有 runtime 名称/冻结绑定如何处理，才能避免任务在错误机器或目录恢复？
- [ ] **工具供应与安装收益**：私有工具包的体积、动态依赖、架构覆盖、签名、公证、分发条件与更新成本是否抵消 Docker 门槛下降的收益？是否存在必须改变能力承诺的工具（如 CJK 字幕、离线 OCR）？
- [ ] **媒体与 AIWS**：大文件、并发、解析失败、磁盘耗尽、codec/字体差异，以及 AICC 可用性如何影响默认场景？不同 AIWS 执行模式的权限和可重复性如何保证？
- [ ] **节点与用户模型**：Desktop 同时承担 OOD 时有哪些额外要求？多 OS 用户、多 Desktop、owner 切换、其他 Agent 请求及机器离线时如何确定运行位置和不可用范围？

后续分析应提交：**最新源码证据 → 可复现失败场景 → 影响范围/严重度 → 方案选择与代价 → 验证方式 → 未决决策**。明确区分阻断目标档位的问题、可以接受的产品限制和可延后事项，最终回答 §1 的核心问题：选哪一档、最小实施顺序、仍需确认的问题。

## 9. 修订记录

- 2026-10-25 初稿：研究目标与任务清单。
- 2026-10-25 按源码 review 修订：标题与 §1 改为可行性分析框架；新增 §2 限制档位 D0–D3 与"允许安装"判定标准；§5 新增关键缺口 G1（激活写死 `support_container`）、G2（Agent 无原生分支且两层）、G3（Desktop 本地应用链路进 worker 容器）、G4（wslc 路径）、G5（身份信号不等价）并补行号；§3 确认"tmux 全线移除"为用户裁决并补理由与待删代码位置，把 Deno 方向记为本文裁决；§6 R6 增加默认实施顺序；§8 权限条按"先能力后安全"限定产出形式。

- 2026-10-25 完成源码级研究：产出 [buckyos-desktop-without-docker-research.md](buckyos-desktop-without-docker-research.md)；§5.1 新增 G6（调度整轮失败）、G7（systest 预装需容器）、G8（无非 OOD 加入路径）、G9（启动期镜像预加载），G2 补第三层与身份结论，G5 补"发行包不传 `--desktop_daemon`"；§2 D3 行补依赖；R0 四项与 R6 实施顺序按报告勾销/修订；§3 "内置系统服务逐项排查隐藏容器依赖"已核实为无。

- 2026-10-25 tmux 删除回写：§3 "Windows shell" 行改为 tmux 代码已删除、Standard / Target Runtime TODO 已回写，去掉指向已删文件的链接（代码删除是同日的独立改动，不属于本研究）。

本次仅修订研究 TODO 并新增研究报告，没有修改 runtime、AppLoader、安装器、协议或 cmd_skills 实现；未执行平台运行、安装或媒体功能测试。
