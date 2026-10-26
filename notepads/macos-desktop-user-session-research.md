macOS Desktop 随用户登录启动：GAP 预研
===================================

日期：2026-10-25。性质：源码预研，未实施运行行为变更。

基线：`buckyos` HEAD `c3e4afb80`，同时核对当前工作树；`BuckyOSApp` HEAD `449ed80`。工作树存在正在进行的 runtime 调整，本文明确区分已有代码、未提交代码与建议；不把设计文档中的完成状态当作平台实测结果。

目标依据是 [Node Executor 的 A-desktop 定义](../doc/arch/node_exector.md#42-三种部署形态)、[集成裁决 R1/R2](agent-runtime-session-workspace-integration-todo.md) 和 [2026-10-25 session-runtime 裁决 S4](opendan-primary-runtime-tools-todo.md)：Desktop 的 node-daemon 在 owner 的 OS 用户交互会话中运行，host-native-runtime 可以成为标准 session-runtime，使用该用户的软件和桌面能力。

**结论：方向与现有 runtime 架构一致，主要缺口在产品接线，但改动超过替换一个 plist。** CurrentUser 执行、共享 Computer Driver、owner_agents 策略种子已经有代码；正式 macOS 安装、桌面管理、运行目录和权限仍按 root 系统服务设计。另有标准工具分发正在调整，必须作为 host 标准环境的联动依赖。

建议的最小运行链路：

```text
指定 OS 用户登录 GUI 会话
  → 用户 LaunchAgent（gui/<uid>，Aqua）
  → node_daemon --enable_active --desktop_daemon
  → Node Executor（CurrentUser）+ 共享 macOS Computer Driver
  → host session-runtime 的命令、文件与 GUI 软件操作
```

BuckyOSApp 负责状态、控制和授权引导；其窗口是否打开不必成为 runtime 存活条件。采用用户 LaunchAgent 仍可由 launchd 保活，关键是用户 GUI domain 与身份，而非停止使用 launchd。Apple 对 [Daemon / Agent 的上下文说明](https://developer.apple.com/library/archive/documentation/MacOSX/Conceptual/BPSystemStartup/Chapters/DesigningDaemons.html) 支持这一选择。

**已经具备、可以复用的部分**

| 能力 | 当前代码事实 | 对本任务的意义 |
| --- | --- | --- |
| Desktop 身份与 Driver 装配 | [node_executor_host.rs:144](../src/kernel/node_daemon/src/node_executor_host.rs#L144) 检查 desktop 参数、平台支持和非 root；173 行选择 CurrentUser，182 行注入共享 Driver | 不必重写 executor 或另装 agent-daemon；启动参数、身份必须正确 |
| owner Agent 默认策略 | [node_executor_host.rs:204](../src/kernel/node_daemon/src/node_executor_host.rs#L204) 在策略缺失时以 HOME 为 roots，设置 computer=control、run_as=None；已有 enabled=false 不被覆盖 | 正确 HOME 是配置输入，不能继续继承 installer 的 root HOME |
| GUI 会话和授权检查 | [native.m:47](../src/kernel/computer_driver/src/native.m#L47) 检查 audit session、console UID、锁屏、截图和输入权限；[desktop.rs:164](../src/kernel/computer_driver/src/desktop.rs#L164) 提供 not-ready 原因 | 会话/权限探测已有基础，缺口是安装归属、引导、产品状态与真实验收 |
| 环境根覆盖 | [buckyos-kit/path.rs:42](../../buckyos-base/src/buckyos-kit/src/path.rs#L42) 和 node_control 支持 BUCKYOS_ROOT | 可以复用现有布局机制，但不能只改一个启动进程的变量 |

Computer Driver 当前声明 macOS 14+，主要提供 visual observation、点击、键入、按键和滚动；语义 Accessibility observation 仍 unsupported，见 [desktop.rs:174](../src/kernel/computer_driver/src/desktop.rs#L174)。登录启动不会自动扩大这套能力范围。

**Windows 的参照边界**

当前 Windows 使用 [HKCU Run + 每分钟 Scheduled Task](../src/publish/win_pkg/scripts/buckyos_postinstall.ps1#L7)，通过 [VBS launcher](../src/publish/win_pkg/scripts/node_daemon_loader.vbs#L22) 隐藏启动 daemon；不是 BuckyOSApp 托管 daemon，也不是 Windows Service。

可以借鉴用户启动与保活的职责划分，但不能把 Windows 当作全部已验证的完成样板：VBS 的命令只有 `--enable_active`（30 行），同样缺少新 desktop Driver 装配所需的 `--desktop_daemon`；进程查重不区分用户/session（54 行）。Windows Executor 默认 CurrentUser，因此不代表 Driver 和 owner 策略种子也一定生效。另外安装器有管理员提权，计划任务没有显式指定交互会话 principal，实际 token/session 还需实测。

**GAP 清单**

P0 表示达成用户会话与 GUI 使用目标的前置项；P1 表示形成可维护安装包的交付项。带“验证”的项是尚未确认的平台行为，不等于已发现必现失败。

| 编号 / 优先级 | 当前事实与具体缺口 | 建议补齐及验收结果 |
| --- | --- | --- |
| G1 / P0：启动入口和参数 | [buckyos_postinstall:7](../src/publish/macos_pkg/scripts/buckyos_postinstall#L7) 创建 `/Library/LaunchDaemons/buckyos.service.plist`；28–37 行操作 `system`；65–66 行只有 `--enable_active` | 注册指定用户的 GUI LaunchAgent，加入 `--desktop_daemon`，用实际用户 HOME/UID。只加 UserName、仅 su/sudo 降权或放进 user domain 都不能作为已进入 GUI 会话的证据。安装时无 GUI 用户，应有延迟到首次登录/首次打开 App 完成注册的流程；不能沿用 15 秒内必须找到进程的成功条件 |
| G2 / P0：运行根与所有权 | [postinstall:54](../src/publish/macos_pkg/scripts/buckyos_postinstall#L54) logs chown root；74 行 HOME=/var/root；81 行输出到 /var/log。默认根仍为 `/opt/buckyos`。Executor [ensure_private_dir:103](../src/kernel/node_executor/src/fsutil.rs#L103) 要求 store 归当前 UID 所有 | 明确程序、配置、密钥、data/storage/local/cache/logs 及运行中更新所需目录的所有权；用户可写状态目录，私钥保持私有。可以保留 `/opt` 布局并调整必要 ownership，也可转用户根，但不要用全树 777。若换根，同步 App、CLI、开发脚本、容器挂载和更新/卸载的路径发现。旧 root executor store 不能直接原样复用 |
| G3 / P0：跨仓启停与状态控制 | 主库 [node_control.rs:1430](../src/kernel/buckyos-api/src/node_control.rs#L1430) 只识别系统 plist；2153/2330 行固定 system 启停。[BuckyOSApp vendored 副本:1489](../../BuckyOSApp/src-tauri/src/node_control.rs#L1489) 有相同假设 | 两份实现同时支持用户启动项及 UID/domain/启用状态。否则新安装被判 DirectProcess，托盘显示 dev，启动走直接 spawn，停止不能卸载 Agent，可能与 KeepAlive 冲突。注册状态、进程存活、GUI readiness 必须分开报告 |
| G4 / P0：GUI 授权及应用身份 | Driver 已能报告 screen_capture_permission_required / accessibility_permission_required，但当前 App/安装链路未形成授权引导。node_daemon 是 `/opt` 独立可执行文件，BuckyOS.app 是另一个组件；不能假定给 App 授权自动覆盖 daemon、worker、shell 子命令 | 对正式产物验证实际 TCC 归属、稳定签名/标识及更新后授权连续性；明确用户给哪个程序授权。按实际操作分别验证截图、输入、受保护文件；需要 AppleScript/Apple Events 时再验证 Automation 的 usage description/entitlement 和目标 App 授权。拒绝/撤销后返回具体未就绪原因，授权后能恢复 |
| G5 / P0：用户环境与标准工具交付 | [spawn.rs:335](../src/kernel/node_executor/src/spawn.rs#L335) 使用 bash --noprofile --norc 并 env_clear；[ops.rs:777](../src/kernel/node_executor/src/ops.rs#L777) 的 Unix 默认 PATH 仅 `/usr/local/bin:/usr/bin:/bin`，HOME 从 passwd 补充。节点种子的 env 为空 | 分别定义 daemon 服务环境和 host exec 环境；在目标节点提供标准工具的真实入口、PATH 和 Agent/Session 上下文，覆盖 Apple Silicon Homebrew 等必要路径。不能认为安装 plist 的 PATH 会自动进入 Executor shell，也不能要求用户手工打开终端后才可用 |
| G6 / P0 验证：基础服务在用户身份下可用 | 当前 [preinstall:24](../src/publish/macos_pkg/scripts/buckyos_preinstall#L24) 强制 root 可 docker info；[boot_gateway.yaml:68](../src/rootfs/etc/boot_gateway.yaml#L68) 仍监听 80。postinstall 注释把绑定 80/443 作为 root 理由 | 保留容器功能时用目标用户检查 Docker CLI/context/socket，处理登录后 Docker/OrbStack 延迟就绪。验证 gateway、激活、身份读写、包更新及所启用服务能以普通用户运行；低端口按支持的 macOS 版本实测，不能仅凭旧注释断定必须 root，也不能未经验证直接删功能 |
| G7 / P0：节点归属与会话边界 | 现有根目录和端口是单节点模型；[node_control.rs:1101](../src/kernel/buckyos-api/src/node_control.rs#L1101) 使用全局 `/tmp/buckyos/run.plist`；[2033 行](../src/kernel/buckyos-api/src/node_control.rs#L2033) 按全局进程名杀冲突进程。desktop_mode 只判非 root，实际 GUI 检查在 Driver 层 | 首版建议明确一个 OS 用户拥有一个 Desktop 节点，用户级注册；不要在 `/Library/LaunchAgents` 无条件让所有用户争用同一节点身份/根/端口。查重、状态文件、停止范围绑定该用户和安装实例。验证快速用户切换/SSH/锁屏，区分 CurrentUser 与桌面已就绪 |
| G8 / P1：完整生命周期 | 当前 plist KeepAlive 与 AbandonProcessGroup 都为 true；[stop_osx.sh:39](../src/rootfs/bin/stop_osx.sh#L39) 按名称 killall。App Quit 只退出 UI，托盘另有 start/stop/restart | 保留“退出 UI 不等于停节点”的既有职责，并明确临时停止、禁用登录启动、重启、注销的区别。受管进程停机与 KeepAlive 配合，注销后节点下线；锁屏保留进程但 GUI not ready。验证子进程/容器是否残留，不泛杀用户自行打开的软件 |
| G9 / P1：更新、卸载与打包校验 | [uninstall.py:368](../src/uninstall.py#L368) 清系统 LaunchDaemon；[verify-pkg:922](../src/publish/make_local_osx_pkg.py#L922) 强制匹配 bootstrap system、kickstart system 和 LaunchDaemon/root Docker 检查 | 模板、生成逻辑、校验、preinstall、覆盖安装、卸载一起改。更新前停用户 job，更新后按原启用状态恢复；卸载先 unregister/bootout 再删程序。开发阶段默认全新安装，不增加旧数据迁移；仍需识别同机残留旧系统 job，避免双入口 |
| G10 / P1：平台验收与诊断 | [macOS 包测试:13](../src/publish/tests/test_make_local_osx_pkg.py#L13) 只有目标路径用例；[CI:357](../.github/workflows/dev-tests.yml#L357) Unix 以 sudo start.py 启动；macOS 没有等价的用户登录安装场景 | 添加真实 GUI 会话安装 DV；诊断显示 UID、HOME、启动 domain、运行根、identity_mode、Driver readiness、授权缺项和工具基线。pgrep 到进程或 Rust 测试通过均不足以证明此目标达成 |

G4 还包含一个容易遗漏的网络差异：Apple [TN3179](https://developer.apple.com/documentation/technotes/tn3179-understanding-local-network-privacy) 明确 root / launchd daemon 的本地网络自动许可不适用于 LaunchAgent；非 SMAppService 注册的 Agent 可用 `AssociatedBundleIdentifiers` 关联应用。切换后要验证本地网络提示、用户拒绝、授权恢复以及 gateway/局域网节点发现，不能只验证 localhost。该关联用于平台识别，不能据此推断所有 TCC 权限自动继承。

G5 与当前 runtime 工作的交界也必须明确：[baseline.rs:68](../src/frame/lib_opendan/src/baseline.rs#L68)（本次核对时为未跟踪新文件）已经列出 agent_tool、xagent、agent-session、buckyos、aicc-tool 和媒体工具；[runtime/mod.rs:773](../src/frame/lib_opendan/src/runtime/mod.rs#L773) 的未提交调整让 remote_node 依赖节点自己的 PATH，不再注入控制侧本地 overlay。现有 [bucky_project.yaml:123](../src/bucky_project.yaml#L123) 主要分发 bin/opendan 和 buckyos CLI，尚不能据此认定宿主标准工具与 helper 全部闭环。这部分需要与进行中的 host 标准基线工作对齐，不要在安装任务里再造第二套工具体系。

G8 可复用 [NodeExecutor::shutdown:326](../src/kernel/node_executor/src/lib.rs#L326) 的停准入、取消及等待逻辑；当前 node-daemon 未调用它，而 [service_pkg.rs:961](../src/kernel/node_daemon/src/service_pkg.rs#L961) 的服务启动会 detached/setsid。因此注销和正常停机需要验证并补齐管理接线，不能只依赖父进程退出。异常后未知执行的恢复沿用 Executor 现有语义，不自动重放副作用。

**建议的实现取舍与顺序**

1. 先做用户 LaunchAgent 的垂直验证：指定 owner，正确参数/HOME/可写目录，在 GUI 会话中启动实际 node_daemon；验通基础服务、host exec、Driver inspect 和一次 GUI 软件操作。这样可尽早暴露低端口、Docker、TCC 归属这些无法从 Linux 静态阅读确认的点。
2. 首版可沿用现有 pkg 与程序布局，由用户级 LaunchAgent 管理节点，减少重打包范围；同时把应用归属和授权入口设计清楚。若采用 bundle 内 helper + SMAppService，则一起调整 BuckyOSApp 包布局、注册/卸载和后台项目状态。Apple [SMAppService](https://developer.apple.com/documentation/servicemanagement/smappservice?language=objc) 支持 macOS 13+ 的 LoginItem/LaunchAgent 管理，但这不是替换两条 launchctl 命令即可完成的方案；它也不是达成 GUI 会话启动的唯一前提。
3. 同步主库和 BuckyOSApp 的 node_control，完成启停、待登录、被禁用、退出 UI、注销和重启语义；再完成 pkg verify、升级/卸载与文档。
4. 与 host runtime 工作共用标准工具清单、节点工具入口及 baseline 检查；最后用新安装包执行真实 host session-runtime 的端到端验收。

实施前要固定的产品选择是：节点归属哪个 OS 用户；沿用 `/opt` 还是用户运行根；登录后台 runtime 是否同时显示托盘。它们不影响“必须运行在用户 GUI 会话”的已定目标。建议首版采用指定单用户节点、runtime 独立保活、UI 按既有行为独立退出；不引入多用户共用节点的额外设计。

不将“整个 Desktop 去 Docker”并入这次安装改造。S4/S5 允许 host session-runtime 无 Docker，但其他服务脱离 Docker 是独立工作；本任务应按实际保留的服务验证用户身份和启动时序。Linux/OOD 的 A-system 路线也不需要随本次改变。没有要求兼容旧 Session/存储格式，按仓库默认进行全新安装验证。

**验收场景**

| 场景 | 应检查的结果 |
| --- | --- |
| 已登录用户全新安装；无 GUI 用户安装后再登录 | 注册目标正确，实际 UID/HOME 正确，system domain 无竞争入口；等待登录不是安装失败 |
| 重启到登录页，再登录；注销再登录 | 未登录不运行 Desktop 节点，登录后恢复；注销后状态/端口/受管进程清理符合约定 |
| 托盘 start/stop/restart；退出 UI；daemon 崩溃 | 不出现双实例；手动停止不立即被 KeepAlive 拉起；退出 UI 保留 runtime；崩溃可恢复 |
| 第二个 OS 用户、快速切换、锁屏、SSH 启动 | 不抢同一节点身份/数据；不控制其他用户桌面；准确报告 interactive_session_unavailable / desktop_locked |
| 截图、输入、文件、Apple Events（如使用）、局域网权限的允许/拒绝/撤销 | 权限归属对应实际执行组件；失败可解释，重新授权可恢复；不能用 Terminal/sudo 运行成功替代安装后成功 |
| 不打开终端的冷登录；Homebrew 两种架构路径；含空格路径 | host exec 找到工具；标准基线通过；agent-session/xagent 能访问正确 Agent/Session 上下文与服务 |
| Docker 尚未启动，然后就绪；gateway 和激活链路 | 服务能重试收敛，用户 Docker context 正确，节点/Agent 真正可用 |
| 覆盖安装、更新、卸载；已有旧系统 job 的测试机 | 无并发替换/自动重启竞争，清理正确，授权连续性有记录，不留下可自动复活的启动项 |

影响入口集中在 `buckyos/src/publish`、`node_control.rs`（两仓）、运行目录发现/权限、host 环境与工具分发；基本启动改造不要求新 runtime kind 或新执行协议。落实时同步 `buckyos_admin`、`buckyos_dev_release`、`opendan_agent_dev` 手册的运行路径、启停、诊断和平台说明。

本次只新增此预研记录，未改安装器、App 或 runtime 实现；未执行构建、安装、卸载、GUI/TCC 或跨平台测试。已核对源码路径、当前安装/控制调用链、相关 manifest/测试/CI 和 Apple 平台文档。平台可用性结论必须由上述 macOS GUI DV 补齐。
