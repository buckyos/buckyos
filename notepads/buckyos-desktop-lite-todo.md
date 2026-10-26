# BuckyOS Desktop Lite 恢复实施 TODO

日期：2026-10-25。

状态：**产品方向已确定，待按本文完成 Lite 集成与验收。本文是后续 codeagent 的实施清单，不表示精简版已经可用。**

源码核对基线：`e3aebad51` 加当前工作树改动。当前已有未提交的原生 Agent Service 等实现；接手时先检查 `git status` 和相关源码，保留在途工作，按缺口补齐，不从旧研究报告重新实现一套。

## 1. 产品范围与约束

本任务恢复 **BuckyOS Desktop Lite**：在没有容器运行环境的电脑上，保留 Desktop、基础系统服务、产品预置的 script host app 和一个原生 Agent，并允许增加 static web app。

| 对象 | Lite 的确定行为 |
| --- | --- |
| 容器环境 | 安装、激活、运行、停止均不要求 Docker、OrbStack、WSL 或 wslc；也不自动安装、启动或回退到这些环境。电脑即使已安装容器工具，Lite 的能力范围也不改变。 |
| apploader / script host app | 原生加载产品预置的 script host app。App 自己读取运行配置并处理本地监听地址与端口；不依赖容器端口映射。 |
| 应用扩展 | 不允许新增 script host app 或 docker app；允许安装、更新、卸载 static web app。预置脚本应用的更新只能沿受控的产品清单进行，不开放任意宿主应用安装。 |
| agentloader / OpenDAN | 在宿主机原生加载 AgentRootFS，运行正式 Agent Service，接入现有身份、消息、状态和工具链。 |
| Agent 数量 | **整个 Lite 系统最多同时运行一个 Agent，且该 Agent 只有一个服务实例。** 不是每用户一个、每节点一个；同一 Agent 的多个 UI / Work / Goal Session 不计为多个 Agent。 |
| 完整版 | 通过明确的产品模式区分。完整版原有容器应用和多 Agent 能力继续按其配置运行。 |

实施边界：优先完成全新安装，不增加完整版到 Lite 的数据迁移或旧 Session 兼容。首版沿用当前原生 Agent 链路，运行 Zone owner 的 Agent、默认使用本机 host runtime；非 owner 原生 Agent、多节点扩容和远端容器调度不作为 Lite 的实现前提或绕过限制的出口。

“只能运行一个”不等于必须删除其它 Agent 的静态资料。采用唯一运行槽位约束启用和部署；当前创建流程若会自动启用 Agent，就必须在创建时占槽并拒绝第二个会启动的创建请求。不要求另做多 Agent 切换管理功能。

## 2. 与已有工作的关系

- [无 Docker 可行性研究](buckyos-desktop-without-docker-research.md)、[原研究 TODO](buckyos-desktop-without-docker-todo.md) 用作背景。本文确定的“预置脚本 + 可扩展静态网页 + 单 Agent”取代旧档位中的候选范围，不再把 D2 / 无应用作为最终交付目标。
- [原生 Agent Service TODO](opendan-native-agent-service-todo.md) 已记录在途实现：复用 `AppRunMode::Native`、`native_agent_node`、`RuntimeType::NativeAgent` 和 `OPENDAN_SERVICE_MODE=native`。这些机制不是 Lite 专属，也不等于 Lite 已实现。
- [标准工具与 Session Runtime TODO](opendan-primary-runtime-tools-todo.md) 已有标准工具清单、`opendan install-agent-tools`、宿主工具目录和 Session 上下文。以当前源码清单为准，不照搬旧研究中“所有镜像工具都要带”的建议。
- macOS 的运行身份和安装启动链依赖 [macOS Desktop 安装与运行 TODO](macos-desktop-install-runtime-todo.md)。Lite 需要该用户会话方案落地，不另设计一套 root 常驻启动链。

当前源码与待补缺口：

| 入口 | 当前事实 | Lite 要补的工作 |
| --- | --- | --- |
| [app_loader.rs](../src/kernel/node_daemon/src/app_loader.rs) | `HostScript` 的 deploy 仍准备 worker image / ExtTool，start 调 `run_worker_container`，stop/status 仍面向容器。 | 恢复完整的原生脚本生命周期，不能只改分支名称或绕过 Docker 检查。 |
| [service_pkg.rs](../src/kernel/node_daemon/src/service_pkg.rs) | 已有宿主进程启动、身份检查和进程树停止基础。 | 优先复用，补日志、就绪判断、跨平台与异常恢复。 |
| [app_schema.rs](../src/kernel/buckyos-api/src/app_schema.rs)、[scheduler/app.rs](../src/kernel/scheduler/src/app.rs) | 已有原生 Agent 选择与实例 `run_mode`；选择依赖 owner、默认 Node runtime 和 executor 配置。 | Lite 激活与创建默认值必须满足该契约，关闭容器选择，并加全系统单 Agent 约束。 |
| [OpenDAN main](../src/frame/opendan/src/main.rs)、[loader](../src/frame/opendan/src/loader.rs) | 已支持正式 Zone 模式、AgentRootFS 装配，以及原生模式下禁用 container runtime。 | 验证 Lite 下的身份、端口、工具、Session 恢复和健康状态；单进程绑定一个 Agent 不代表系统限额。 |
| [system_config_agent.rs](../src/kernel/scheduler/src/system_config_agent.rs)、[node_daemon.rs](../src/kernel/node_daemon/src/node_daemon.rs) | static web 已独立生成目录路由、物化 web 包并上报部署证据，无需启动 App 进程。 | 保留完整安装和访问链；混合包、更新不能绕过 Lite 限制。 |
| [active_lib.ts](../src/kernel/node_active/active_lib.ts)、[active_server.rs](../src/kernel/node_daemon/src/active_server.rs) | 激活客户端仍默认 `support_container: true`；daemon 启动仍调用镜像预载入口。 | 增加一致的 Lite 产品模式与激活校验，关闭所有容器旁路。 |
| [app_installer.rs](../src/frame/control_panel/src/app_installer.rs)、[agent_mgr.rs](../src/frame/control_panel/src/agent_mgr.rs) | 已有安装计划、来源区分、Agent 创建幂等和事务预留。 | 增加 Lite 准入与原子运行槽位，复用这些机制。 |

## 3. 实施任务

以下复选框表示 **Lite 任务尚待验收**，即使其中复用的底层代码已经存在也不能直接勾选。每项交付应记录改动入口、验证命令、结果和未验证平台。

### T0 / P0：统一产品模式、预置清单和能力来源

- [ ] 在现有安装/节点配置中落实显式 Lite 模式，由发行包贯穿激活、system-config、scheduler、node-daemon、control-panel 和 Desktop。字段名在实现时统一确定；不要各模块分别读取不同环境变量推断版本，也不要仅靠 `--desktop_daemon` 判断 Lite。
- [ ] Lite 激活必须产生 `support_container=false`，后端核验产品约束，不能由客户端传 `true` 绕过。缺少 Lite 所需配置时明确报错，不默认为完整版。保留既有 OOD / Node 类型语义，避免为产品档位发明新节点类型。
- [ ] 将“容器能力”“是否允许新增脚本应用”“是否允许 static web”“Agent 运行上限 1”区分表达。`support_container=false` 本身不能表达全部 Lite 策略，也不应限制其它无容器节点的既有能力。
- [ ] 列出 Lite 的具体预置 script host app 及可信包身份、入口、解释器、服务端口和更新来源，写入受版本管理的产品清单。许可不能只看 author 前缀、包类型或调用者声称的 `preinstall` 标志。
- [ ] 核对 [boot.template.toml](../src/rootfs/etc/scheduler/boot.template.toml)、[devenv_config.ts](../src/devenv_config.ts) 和 [bucky_project.yaml](../src/bucky_project.yaml) 的种子/模块：static text-editor、systest 脚本应用、Jarvis 模板分别按实际类型处理。要保留的脚本必须完成原生适配；不交付的组件从 Lite 种子移除，不能留下永远 installing 的任务。
- [ ] 接手时用 `git log` / `git show` 查历史宿主脚本加载实现，记录可复用的入口/参数约定；实现服从当前包、身份和进程协议，不恢复整套旧 loader。

完成条件：从安装到后台 API 读到同一份 Lite 能力和预置清单；即使宿主有 Docker，产品行为仍固定为 Lite。

### T1 / P0：应用准入与 static web 扩展

主要入口：[app_installer.rs](../src/frame/control_panel/src/app_installer.rs)、[app_install_planner.rs](../src/frame/control_panel/src/app_install_planner.rs)、[app_install_engine.rs](../src/frame/control_panel/src/app_install_engine.rs)、scheduler 与 AppLoader。

- [ ] `apps.inspect` 给出不支持的原因；提交、执行、重试、批量更新和预装再次校验。覆盖 RPC/CLI、本地包和在线包来源，不能只隐藏 UI 按钮。
- [ ] 拒绝不在产品清单内的新 script host app、所有 docker app 及其它未开放的服务执行形态。包同时提供 script/docker/native 变体时，不能通过 runtime fallback 获得宿主执行权限。
- [ ] 保留真正 static web 的安装、升级、卸载和 Desktop 入口。按解析后的 AppDoc / execution plan 检查，而不是看到一个 `web` 子包就放行；含服务的混合包、静态包升级为脚本/容器服务都必须被拦截。
- [ ] 区分用户应用提交、产品预装与系统构造 Agent App 的内部流程，复用现有 `AppSubmitOrigin` 等来源模型。唯一 Agent 的内部部署有明确许可，不能因为“禁装 App”被一并禁止；内部来源不能被普通 RPC 参数伪造。
- [ ] scheduler 与 node-daemon 复核同一策略；直接写入不支持的 spec、启动本地 `applist.json` 或恢复旧任务时，也不运行额外脚本/容器服务。
- [ ] 保留 `build_static_web_app_host_entry`、`ensure_web_apps_installed` 和部署证据反馈：安装任务完成、网关目录路由、访问、升级内容刷新及卸载清理全部闭环。

完成条件：新增 static web 可用；任何入口的新 script/docker 都在启动前被明确拒绝，既有系统服务和唯一 Agent 不受误伤。

### T2 / P0：apploader 原生加载预置 script host app

主要入口：[app_loader.rs](../src/kernel/node_daemon/src/app_loader.rs)、[service_pkg.rs](../src/kernel/node_daemon/src/service_pkg.rs)；脚本入口现行约定参考 [entrypoint.sh](../publish/aios/entrypoint.sh) 的 `buckyos_script.json` / 语言及入口解析。

- [ ] 明确原生脚本运行分支及其选择条件，覆盖 deploy/start/stop/status/preview。deploy 只准备实际需要的包和私有解释器，不准备 paios 镜像、ExtTool 卷或执行 docker/wslc 命令。
- [ ] 沿用当前脚本入口和参数契约，核对 Deno/Python 等实际依赖。只打包预置清单所需的 runtime；没有合适入口或解释器时给出具体错误，不能假设开发机 PATH 就是产品环境。
- [ ] 复用正式 App 身份环境，包括 `build_runtime_env` 的 token、owner、AppInstanceId 与 `app_instance_config`；将 pkg/data/log/storage 路径和网关地址装配为宿主值。使用明确 cwd 和进程级 PATH，支持空格、中文路径，不改系统全局环境。
- [ ] **端口契约**：将 `service_ports_config` 等最终运行配置传给 App，App 自行绑定配置中的宿主监听端口。网关路由、服务发现与 App 实际监听一致；原容器 `inner_port → instance_port` 映射不再存在。复用既有配置模型，不另加透明端口转发层。
- [ ] 为每个预置脚本应用补端口配置读取；多服务应用逐项校验。端口冲突明确失败并报告 app/service/address/port，不能静默换随机端口、杀掉占用端口的其它程序，或仅因 PID 存活就报 ready。
- [ ] 原生生命周期覆盖：幂等启动、PID/启动身份核验、标准输出与错误日志、有界优雅退出及进程树清理、崩溃重启退避、daemon 重启接管/收敛、更新前先停旧进程。Windows 与 Unix 使用各自可靠的进程管理方式。
- [ ] 接通健康与部署状态反馈；未监听、配置错误、进程退出都能结束或更新安装任务状态。完整版现有 HostScript 容器路径按产品模式继续工作。

完成条件：至少一个实际预置脚本应用在无容器机器上可访问，重启/更新/停止均可收敛；端口冲突可诊断且不拖住其它服务。

### T3 / P0：接通 OpenDAN 原生 AgentRootFS 与唯一运行槽位

主要入口：[agent_mgr.rs](../src/frame/control_panel/src/agent_mgr.rs)、[agent_runtimes.rs](../src/frame/control_panel/src/agent_runtimes.rs)、[app_schema.rs](../src/kernel/buckyos-api/src/app_schema.rs)、scheduler、AppLoader、OpenDAN main/loader。

- [ ] 复用当前 `native_agent_node` 与 `AppRunMode::Native` 链路：Lite 创建 owner Agent 时默认选择本机 Node host runtime，节点 executor 与 `owner_agents` 配置满足既有授权条件；服务实际由 node-daemon 托管原生 OpenDAN。
- [ ] 原生 OpenDAN 使用正式 Zone/AppService 登录，按现有 `AgentSpec.binding` 定位 Agent；不得以 `--dev`、`--trust-loopback` 或固定 `devtest` 替代产品身份。
- [ ] 沿用 AgentRootFS：`$BUCKYOS_ROOT/data/home/<owner>/.local/share/<app_id>/agents/<agent_id>/`。区分只读 Agent 包与可写 RootFS，复用模板同步和本地修改保留机制；验证 skills、Agent State、Session、消息桥和 WebUI 都引用同一 Agent。
- [ ] Lite 禁用 container runtime 的选择、修改、恢复与执行，不把宿主冒充成 `container`。Session 执行继续走配置好的 host Node Executor / profile，不绕过既有 roots、身份和授权。
- [ ] 用现有 system-config 事务和创建预留机制实现唯一运行槽位：按 Lite 系统统一计数，创建/启用原子占槽，重复幂等请求仍指向同一 Agent，不能先 count 再 create。预留、运行中、崩溃等待恢复都占用槽位。
- [ ] 明确释放规则：未成功创建的预留回滚；已运行 Agent 必须完成禁用/删除、撤销调度并确认服务停止后才能释放。崩溃或离线不能自动腾槽导致旧实例恢复时出现两个 Agent。复用现有状态机，不另建一套通用配额服务。
- [ ] scheduler 限制目标 Agent 身份与副本数为 1；node-daemon 启动时复核槽位并处理残留进程，防止并发创建、任务重试、重启、直接 spec 写入产生第二实例。数量限制不落到 Session 层。
- [ ] 补齐当前 NativeAgent 路径的 Lite 验收：正式端口就绪、健康探测、进程身份、日志、停止子进程、daemon/系统重启及 Session 恢复。PID 存活不能单独证明 Agent Service ready。
- [ ] 核验 Windows 控制侧命令发现与 helper：`main.rs::beside_exe("xagent")` 的 `.exe` 处理、[bin_overlay.rs](../src/frame/lib_opendan/src/runtime/bin_overlay.rs) 的 shell 入口/PATH 分隔符，以及 host Session 的 helper 转发。修复实际可达的残留平台假设，复用已有多入口程序，不重复实现标准工具分发。

完成条件：唯一 Agent 原生加载 RootFS 并完成对话/工具任务；并发启动第二个 Agent 被后端拒绝；同一 Agent 的多个 Session 仍能正常工作。

### T4 / P0：调度收敛与彻底移除容器运行依赖

- [ ] [system_config_agent.rs](../src/kernel/scheduler/src/system_config_agent.rs) 中的 `need_container` 与 [app.rs](../src/kernel/scheduler/src/app.rs) 的包选择按 Lite 清单和原生运行方式推导。现有特定官方 author 放行逻辑不等价于产品预置许可。
- [ ] [scheduler.rs](../src/kernel/scheduler/src/scheduler.rs) 对不可部署的 spec 记录明确状态与原因，并继续收敛其它 spec；覆盖 `No suitable node` 等错误，避免一个不支持的应用让整轮调度提前失败。
- [ ] Lite 禁用 [paios_images.rs](../src/kernel/node_daemon/src/paios_images.rs) 预载、镜像/ExtTool 准备和 [wslc.rs](../src/kernel/node_daemon/src/wslc.rs) 的自动选择。入口在激活前也要可判断产品模式；仅不打包镜像清单不足以证明无容器依赖。
- [ ] 检查 NativeAgent 的共享环境构造、预览、状态、切换清理路径；即使复用了完整版代码，也不能在 Lite 下探测/清理容器。检查 `start.py`、`stop.py`、[node_control.rs](../src/kernel/buckyos-api/src/node_control.rs)、安装/卸载脚本和控制面板容器概览。
- [ ] 不支持的容器操作直接给出产品能力说明，不进入无限重试；标准 UI 不显示不可用的容器选择或要求安装 Docker 的修复提示。

完成条件：正常安装/启停/运行路径无容器命令调用；一个非法应用或占用端口不会阻塞基础服务、静态网页和 Agent 的其它状态收敛。

### T5 / P0：发行包、桌面运行身份和工具交付

- [ ] 在 [bucky_project.yaml](../src/bucky_project.yaml)、[publish](../src/publish) 和 [.github/workflows](../.github/workflows) 的现有链路中产出可区分的 Lite 包。Lite 不要求 paios 镜像 tar/manifest 或 ExtTool，标准工具、预置脚本和静态资源不得因精简被误删。
- [ ] macOS 的 [preinstall](../src/publish/macos_pkg/scripts/buckyos_preinstall) 与 [pkg 校验](../src/publish/make_local_osx_pkg.py)、Windows 的 [NSIS 模板](../src/publish/win_pkg/installer.nsi.in) 与 [打包校验](../src/publish/make_local_win_installer.py) 按产品模式去除 Docker 硬前置；同步安装文案与模板测试。
- [ ] macOS 合并既定 Runtime.app / 用户 LaunchAgent 路线，验证真实 UID、HOME、目录可写性、GUI 会话及基础服务监听；Windows 验证登录用户启动和 `--desktop_daemon` / executor 配置。高权限安装进程不能被直接当作日常 Agent 执行身份。
- [ ] 联动 [BuckyOSApp](../../BuckyOSApp) 的启动、停止、状态和权限引导，保证它控制的是同一套 Lite 节点；系统低端口、登录项、路径变化按平台实测处理。
- [ ] 复用 [STANDARD_TOOLS](../src/frame/lib_opendan/src/baseline.rs)、`opendan install-agent-tools` 和 [node_executor_host.rs](../src/kernel/node_daemon/src/node_executor_host.rs) 的 `$BUCKYOS_ROOT/bin/agent-tools`。SDK/CLI、私有 Deno、buckyos-agent 源码与入口完整交付，在无开发工具的机器上检查 baseline。
- [ ] FFmpeg、ImageMagick 等按当前 standard/extended 契约与产品承诺交付；缺失的扩展能力如实显示，不把容器镜像工具全部列成 Lite 硬前置。AICC/provider 可用性单独验证，不把本地工具存在当作 AI 调用成功。
- [ ] 更新 Desktop 创建向导、应用安装页和诊断说明：显示 Lite 能力、唯一 Agent 槽位占用原因、原生服务位置、端口/工具错误。同步受影响的 `buckyos_admin`、`buckyos_app_dev`、`opendan_agent_dev`、`ui_core` 等 cmd_skills 与配置示例。

完成条件：干净 Windows/macOS 环境可安装与激活 Lite，不需用户先装容器或开发工具；标准工具以正确用户和 Agent 上下文执行。支持的 OS/架构逐项记录结果，不以 Linux 开发机通过代替桌面验证。

## 4. 执行顺序与验收

建议顺序：**T0 → T1（准入）+ T4（调度/容器入口）→ T2（原生脚本）+ T3（单 Agent 集成）→ T5 → 完整 DV**。T5 中的 macOS 用户会话依赖可并行推进。每阶段先完成一个可运行的纵向场景，再扩大到产品预置清单；不把“只启动了 Agent”当作 Lite 交付。

| 场景 | 通过条件 |
| --- | --- |
| 干净安装 | 无 Docker/WSL/wslc 的受支持 Windows/macOS 环境：安装 → 激活 → Desktop 可用，基础服务健康，Lite 能力一致。 |
| 容器工具已存在 | 相同 Lite 安装在已有容器工具的环境，仍不启用容器或增加产品能力；受控测试记录确认无容器命令被调用。 |
| 预置脚本 | 一个真实预置 App 读取配置监听实际端口，经网关访问；重启、升级、停止、崩溃恢复与日志可验证。 |
| 端口错误 | 预先占用配置端口；App 明确失败、状态可诊断，其它服务继续工作；释放端口后按既定重试流程恢复。 |
| 拒绝扩展 | UI/RPC/CLI、本地/在线包、重试/更新均拒绝新 script/docker 和混合包绕过；无残留运行进程或假成功任务。 |
| static web | 新增 → 安装完成 → Desktop/网关访问 → 更新看到新内容 → 卸载路由/内容回收；未启动 App 服务进程。 |
| 原生 Agent | 正式身份登录、RootFS 持久化、MessageHub 对话、host 工具调用及至少一次可验证的产物交付；无 Agent 容器。 |
| 单 Agent | 两个不同创建/启用请求并发时只允许一个获得运行资格；幂等重试不重复占槽；禁用/删除后正确释放；重启/恢复不出现第二实例。 |
| Session 与执行身份 | 同一 Agent 可运行多个 Session；host 执行受现有 profile 约束；container runtime 不可选且不能被旧设置恢复。 |
| 失败隔离 | 注入不支持的 App spec / 不可用 host 配置，报告对应错误，其它系统服务与 static web 继续收敛。 |
| 完整版回归 | 完整版已有 Docker/HostScript/Agent 路径、安装器检查和多 Agent 行为未被 Lite 限制污染。 |

验证分层：

- 单元/集成：产品策略和真实执行计划分类、原子槽位并发/幂等/回收、进程启停/残留识别、端口配置与健康、调度错误隔离、安装器模板分支。优先扩展已有用例，不只测试枚举值或 mock 返回成功。
- 修改相应模块后，从 `buckyos/src/` 按影响范围运行 `cargo test -p buckyos-api`、`cargo test -p node_daemon`、`cargo test -p scheduler`、`cargo test -p control_panel`、`cargo test -p opendan` 及相关 `libopendan` 用例，沿用仓库要求的 `-- --test-threads=1`。构建/打包脚本、WebUI 使用各自现有测试入口与构建命令。
- 最终 DV 使用实际 Lite 安装产物和新配置，记录 OS/架构、产品版本、进程身份、路由/监听端口、关键日志和失败样例。真实 provider、macOS 权限、Windows 进程树等未实测项明确列出，不能由单测通过推定。

## 5. 完成标准

- [ ] T0–T5 的 Lite 验收项完成，预置清单中的每个脚本 App 均完成原生适配。
- [ ] 全链在无容器环境可运行；static web 扩展和单 Agent 限制都由后端保证。
- [ ] Windows/macOS 支持矩阵、安装包、工具基线、诊断与 cmd_skills 口径一致。
- [ ] 相关测试、构建和平台 DV 有可复核记录，完整版回归完成；剩余限制明确记录。

本次仅新增实施 TODO，并核对当前源码入口与文档链接；没有修改运行代码、安装器或产品配置，也未执行 Lite 构建与平台 DV。
