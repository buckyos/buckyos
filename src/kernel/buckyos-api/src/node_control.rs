// Node Control API
//
// node_control 是“当前设备”视角的本机控制面设计。
//
// 这份文件当前仍是协议/实现设计文档，不是已经导出的 Rust API。落地成 kRPC 类型时，
// 应按 buckyos-api 现有模式补齐 serde request/response、client、handler 和 dispatcher。
//
// =============================================================================
// 0. 当前版本设计结论
// =============================================================================
//
// 这版 node_control 的主要目标不是构建完整自动化诊断系统，而是替代开发期
// `src/start.py`、`src/check.py`、`src/stop.py` 中与本机 BuckyOS Runtime 有关的能力，
// 同时支撑 BuckyOS Desktop 安装、卸载、升级时的本机停止/重启需求。
// 平台安装包的当前实现以 `src/publish` 为准：
// - macOS: `src/publish/macos_pkg/scripts/buckyos_*`
// - Linux/Debian: `src/publish/deb_template/DEBIAN/*`
// - Windows: `src/publish/make_local_win_installer.py` 和 `src/publish/win_pkg/scripts/*`
//
// 关键修正：
// - `start` 不应只理解为 spawn 进程。正式语义应是让本机 BuckyOS Runtime 进入期望运行态，
//   即 `ensure_running`。
// - 如果当前安装形态已经注册到 Host OS 的服务管理器，应优先通过服务管理器启动/停止
//   node_daemon；如果是 Windows 计划任务或开发模式直启，则走对应 launcher。
// - node_daemon 是本机 run item 的确定性收敛器，不是 run item 的生命周期父进程。
//   node_daemon 崩溃或被 Host OS 重启时，已经启动的 run item 应继续运行；系统进入
//   “controller down, workload alive”的降级态，而不是把 workload 一起杀掉。
// - `stop` 不能只依赖 BuckyOS 内部控制链路，因为故障时 scheduler -> node_config ->
//   node_daemon 这条路径可能已经不可用。必须保留黑盒停止路径，能识别并停止本机进程、
//   容器和服务。
// - `check` 当前只做两层：先探测系统当前是否可通过标准接口运行/待激活；如果接口不可用，
//   再按二进制、进程、端口、日志做黑盒诊断。
// - Green 必须是有价值的“正常”信号；Yellow/Orange/Red 要给出下一步方向，但 Beta2 不追求
//   完整自动化根因定位。
//
// =============================================================================
// 1. 作用边界
// =============================================================================
//
// node_control 负责：
// - 当前 Host OS 上的 BuckyOS Runtime 控制。
// - 当前设备上的 node_daemon、kernel service、frame service、local app service。
// - BuckyOS 完全不可用时，通过 Host OS API 做黑盒诊断和恢复。
// - Desktop 安装/卸载/升级时的 stop -> update/uninstall -> start 流程。
// - 输出结构化 check report、operation 状态和审计信息。
// - 区分 node_daemon 自身生命周期和 run item 生命周期，避免把 node_daemon crash restart
//   等同于 BuckyOS Runtime stop。
//
// node_control 不负责：
// - 多节点顺序、屏障和分布式重启计划；这些属于 system_control。
// - Zone 级恢复拓扑、数据恢复和 Agent repair 编排；这些属于 system_control。
// - 证明用户数据完整性。当前版本只能给出“未发现明显风险 / 未检查 / 风险未知”等本机信号。
//
// 典型消费者：
// - BuckyOS Desktop 安装器、卸载器、升级器
// - BuckyOS Desktop / tray 的本机服务管理入口
// - recovery system / diagnostic gateway 的本机动作入口
//
// 推荐入口：
// - Host OS 侧入口：native helper / installer / desktop helper 直接调用 node_control 实现。
// - BuckyOS 运行中增强入口：`/kapi/node-daemon` 下的 `node.*` 方法。
// - 最小诊断入口：`/diag/v1/node/*` 只暴露只读子集或 last status，不依赖完整 BuckyOS。
//
// =============================================================================
// 2. Host Control Model
// =============================================================================
//
// node_control 所有 start/stop/restart 都要先识别当前安装形态。不要从平台名直接推导最终
// 行为，平台名只能作为默认候选；真实结果应来自 installer 记录、service manager 查询、
// 计划任务查询、rootfs/bin 检查和当前进程状态。
//
// enum NodeHostControlModel
// - ServiceManager
//   node_daemon 已注册为 Host OS 服务。
//   - macOS: launchd 用户 LaunchAgent（gui/<uid> domain）
//   - Linux: systemd
//   - Windows: 如果未来 installer 改为 Windows Service，也归入此类
// - ScheduledLauncher
//   通过定时/计划任务周期性拉起 node_daemon。当前 Windows 安装形态按这个模型处理。
// - DirectProcess
//   开发模式或临时安装模式，直接 spawn `node_daemon_binary_path()`。macOS 上只有显式
//   `DirectProcessOnly`（前台调试）才走这里，未绑定用户的安装不会退回直接 spawn。
// - ContainerizedRuntime
//   当前设备以容器编排方式运行 BuckyOS Runtime；停止时必须纳入容器选择器。
// - Unknown
//   无法确认安装形态，只允许 check 和显式 blackbox stop。
//
// 当前安装包实现映射：
// - macOS（Desktop，A-desktop）:
//   - node_daemon 是 `${BUCKYOS_ROOT}/bin/node-daemon/BuckyOS Runtime.app` 的主可执行文件，
//     Bundle ID / launchd label 都是 `com.buckyos.runtime`；已安装产物只有 bundle 内这一份。
//   - 一个安装实例绑定一个 OS 用户：`${BUCKYOS_ROOT}/etc/desktop_runtime.json`
//     记录 `{user, uid, home}`，installer、App、node_control 都只读这一份。
//   - 用户 LaunchAgent：`<home>/Library/LaunchAgents/com.buckyos.runtime.plist`，
//     `LimitLoadToSessionType=Aqua`，参数 `--enable_active --desktop_daemon`，
//     env 为 `BUCKYOS_ROOT`、绑定用户 `HOME`、服务 PATH，日志写 `logs/node_daemon/launchd.*.log`。
//     `RunAtLoad`/`KeepAlive`/`AbandonProcessGroup` 为 true，`ExitTimeOut=30` 留给 Executor 有界退出。
//   - 登录自启偏好 = LaunchAgents 目录里有没有这份 plist；关闭自启后手动启动从
//     `local/node_daemon/com.buckyos.runtime.plist` bootstrap。node_control 从不 `launchctl disable`，
//     看到 disabled 只可能来自用户/系统设置（Login Items），如实报告，不反复 enable。
//   - start: 写 plist → 未加载则 `launchctl bootstrap gui/<uid> <plist>` → `kickstart gui/<uid>/<label>`。
//   - stop: `launchctl bootout gui/<uid>/<label>`（不改自启偏好），再按请求范围做实例内黑盒清理。
//   - 实例内进程：uid 为绑定用户且可执行文件位于 `${BUCKYOS_ROOT}` 下；不按全机进程名 kill。
//   - 旧版 `/Library/LaunchDaemons/buckyos.service.plist` 只作冲突检测与清理对象，不作运行入口。
// - Linux/Debian:
//   - 使用 systemd service。
//   - unit: `/etc/systemd/system/buckyos.service`
//   - ExecStart: `/opt/buckyos/bin/node-daemon/node_daemon --enable_active`
//   - WorkingDirectory: `/opt/buckyos/bin`
//   - User: `root`
//   - Restart: `always`
//   - KillMode: `process`，systemd 只停止主进程 node_daemon，不级联清理同 cgroup 内的
//     native run item，保证 node_daemon 崩溃/重启不杀已启动 workload。
//   - postinst: `systemctl stop`、`daemon-reload`、`enable`、`start`
//   - preinst: 如果 `/opt/buckyos/bin/` 存在，先 `systemctl stop buckyos.service` 再删除 bin。
// - Windows:
//   - 当前安装包不注册新的 Windows Service。
//   - 使用计划任务 `BuckyOSNodeDaemonKeepAlive` 每 1 分钟执行一次 keepalive。
//   - 同时写入当前用户启动项 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\BuckyOSDaemon`。
//   - keepalive 通过 `wscript.exe //B //NoLogo <root>\scripts\node_daemon_loader.vbs`
//     调用 `node_daemon_loader.ps1`，如果 `node_daemon.exe` 已运行则直接退出，否则 hidden 启动
//     `<root>\bin\node-daemon\node_daemon.exe --enable_active`。
//   - root 记录在 `HKCU\Environment\BUCKYOS_ROOT`、`HKCU\Software\BuckyOS\InstallDir`、
//     `HKCU\Software\BuckyOS\BuckyOSUserDir` 和 `HKCU\Software\BuckyOS\InstDir_buckyos`。
//   - 安装/卸载会删除计划任务、删除 Run 启动项。
//   - 如果能解析旧 root，优先执行 `<root>\bin\stop.ps1`；否则 fallback 到
//     `taskkill /F /IM node_daemon.exe`。
//
// struct NodeHostControlState
// - model: NodeHostControlModel
// - platform
// - buckyos_root
// - node_daemon_binary
// - service_unit_name
// - scheduled_task_name
// - run_key_name
// - install_record_path
// - confidence
// - evidence
//
// 设计原则：
// - 已服务化时，启动/停止 node_daemon 优先走 Host OS 服务接口。
// - Host OS 服务管理器只应该保障 node_daemon 自动恢复，不应该在 node_daemon crash/restart 时
//   自动停止 run item。run item 的目标态由 node_daemon 恢复后继续 reconcile。
// - ScheduledLauncher 模式下，手动 start 仍然有意义，因为它可以跳过下一次调度窗口等待。
// - DirectProcess 模式下，start/stop 与现有 `src/start.py`、`src/stop.py` 的开发体验保持一致。
// - node_daemon 只负责管理其可见的 BuckyOS 服务；Host OS 服务管理器通常只知道 node_daemon，
//   不一定知道 node_daemon 拉起的子进程、容器和应用态服务。
// - macOS plist 使用 `AbandonProcessGroup=true` 符合“node_daemon 不拥有 run item”的目标；
//   因此 `launchctl bootout` 只应被视为停止 controller，不是完整 Runtime 清理。
//   升级/卸载仍需要显式黑盒进程/容器检查兜底。
// - Linux systemd 若使用默认 control-group kill 语义，可能与“controller crash 不杀 run item”
//   目标冲突；node_control 的 host_control check 应报告 unit policy 是否满足该目标。
// - Windows ScheduledLauncher 会周期性拉起 node_daemon；黑盒停止前必须先删除计划任务和 Run 启动项，
//   否则进程可能在 1 分钟窗口内被重新拉起。
//
// =============================================================================
// 3. 共享模型
// =============================================================================
//
// enum NodeControlTarget
// - LocalRuntime
//   当前设备上的完整 BuckyOS Runtime。包含 node_daemon、cyfs_gateway、kernel/frame services、
//   local app services，以及由 BuckyOS 拉起的相关容器。
// - NodeDaemon
//   只针对 node_daemon 进程或服务单元。
// - KernelService { service_name }
//   当前设备上的单个 kernel service，例如 system-config、scheduler、verify-hub。
// - FrameService { service_name }
//   当前设备上的单个 frame service，例如 control-panel、repo-service、msg-center。
// - LocalAppService { app_id, service_name }
//   当前设备上的本地 app service 或被调度到本机的 app service。
// - ExternalDependency { dependency_name }
//   Docker Engine、cyfs-gateway、系统网络、service manager 等软依赖。
//
// enum NodeAvailabilityLayer
// - HostUnreachable
//   连 Host OS 侧控制入口都不可达，只能依赖用户本地操作或物理恢复方式。
// - HostReachable
//   Host OS 可用，BuckyOS 可作为黑盒检查。
// - ActivationEndpointReachable
//   未激活状态下 node_daemon 的 activation endpoint 可访问，例如 127.0.0.1:3182。
// - NodeDaemonReachable
//   node_daemon 控制面可访问，但完整 kernel 不一定可用。
// - ControllerDownWorkloadAlive
//   node_daemon 不可达，但本机 run item 仍在运行。这是预期降级态，不是自动清理触发条件。
//   推荐动作是恢复 node_daemon 并重新进入 reconcile，而不是默认 kill workload。
// - MinimalKernelReachable
//   NodeGateway、system-config、node_daemon 至少部分可用。
// - FullRuntimeReachable
//   核心服务和完整运行面基本可用。
// - Unknown
//
// enum NodeFaultLevel
// - Green
//   本机处于预期运行态或预期待激活态，核心检查通过。
// - Blue
//   局部应用或非关键服务异常，例如 control-panel 不可用但核心运行面可用。
// - Yellow
//   本机降级但仍有可操作路径，例如二进制存在但某些进程/端口缺失。
// - Orange
//   controller 不可用但 workload 仍可提供部分服务，或标准控制面不可用但黑盒恢复路径仍可执行。
// - Red
//   关键恢复能力不可用、数据风险明显，或继续强控制可能扩大损害。
//
// enum DataSafetyState
// - NotEvaluated
//   当前版本未检查数据安全。这是 node_control check 的默认数据状态。
// - Safe
// - LikelySafe
// - Unknown
// - AtRisk
// - Damaged
//
// enum Confidence
// - Confirmed
// - High
// - Medium
// - Low
//
// struct NodeControlContext
// - request_id
// - caller_appid
// - caller_user_did
// - source_ip
// - is_local_console
// - is_recovery_mode
// - is_installer_flow
// - reason
//
// struct NodeOperationRef
// - operation_id
// - target
// - action
// - status
// - created_at
// - next_poll_after_ms
//
// enum NodeOperationStatus
// - Planned
// - WaitingForApproval
// - Running
// - Verifying
// - Succeeded
// - Failed
// - RolledBack
// - Cancelled
//
// =============================================================================
// 4. check：本机只读诊断
// =============================================================================
//
// 方法名建议：
// - node.check
// - node.check_minimal
// - node.get_last_status
// - node.get_capabilities
// - node.detect_host_control
//
// struct NodeCheckRequest
// - target: NodeControlTarget
// - level: NodeCheckLevel
// - include_logs: bool
// - include_ports: bool
// - include_processes: bool
// - include_binaries: bool
// - include_host_control: bool
// - user_report: Option<UserFaultReport>
//
// enum NodeCheckLevel
// - Minimal
//   不依赖完整系统，只读 last status、activation endpoint、核心端口和 node_daemon 进程。
// - Basic
//   对应当前 `src/check.py` 主体能力：activation、进程、端口、关键二进制。
// - Standard
//   追加 HostControlModel、rootfs/bin 检查、关键配置文件和常见日志诊断。
// - Deep
//   预留给 Beta3：DNS/证书/Gateway/Relay/存储摘要等高级诊断，但仍保持只读。
//
// struct NodeCheckReport
// - overall: NodeFaultLevel
// - availability_layer: NodeAvailabilityLayer
// - data_safety: DataSafetyState
// - confidence: Confidence
// - title
// - summary
// - buckyos_root
// - platform
// - host_control: NodeHostControlState
// - activated: bool
// - activation_ready: bool
// - live_probe: NodeLiveProbeReport
// - binary_checks: Vec<NodeBinaryCheck>
// - process_checks: Vec<NodeProcessCheck>
// - port_checks: Vec<NodePortCheck>
// - service_checks: Vec<NodeServiceCheck>
// - config_checks: Vec<NodeConfigCheck>
// - log_findings: Vec<NodeDiagnosticFinding>
// - recommended_actions: Vec<NodeRepairActionProposal>
// - capabilities: Vec<NodeControlCapability>
//
// struct NodeLiveProbeReport
// - activation_endpoint_ok
// - node_gateway_ok
// - system_config_ok
// - node_daemon_control_ok
// - runtime_version
// - detail
//
// 当前版本 check 流程：
// 1. 解析 BUCKYOS_ROOT。
//    - 优先使用进程环境变量 `BUCKYOS_ROOT`。
//    - macOS 其次读当前用户 LaunchAgent plist 里的 `BUCKYOS_ROOT`（自定义根）。
//    - macOS/Linux 默认 `/opt/buckyos`。
//    - Windows 依次 fallback 到 `%APPDATA%\buckyos`、`%USERPROFILE%\buckyos`、`C:\buckyos`。
// 2. 检查 `$BUCKYOS_ROOT/etc/node_identity.json` 判断 activated / activation pending。
// 3. 如果未激活：
//    - 检查 node_daemon 进程。
//    - 检查 3182 端口和 HTTP 响应。
//    - 进程 + 3182 + HTTP ok 时，返回 activation_ready。
// 4. 如果已激活：
//    - 先探测标准运行接口：NodeGateway(3180)、system-config(3200)、node_daemon 控制面。
//    - 再检查 node_daemon、cyfs_gateway、system_config、scheduler、verify_hub、control_panel。
//    - control_panel 缺失按 Blue/Warning，不直接判定本机不可恢复。
//    - cyfs_gateway 缺失且 80/3180 不可达时，检查 cyfs_gateway binary 并给出 build/install 建议。
//    - 分析常见日志信号，例如 churn、permission denied、service login failed。
// 5. 同时检查 HostControlState：
//    - macOS: 用户绑定、`launchctl print gui/<uid>/com.buckyos.runtime`、登录自启、Login Items
//      禁用状态、旧 LaunchDaemon 残留，以及 Runtime 写出的 `local/node_daemon/desktop_status.json`
//      （GUI 会话与截图/输入权限，只能由 Runtime 进程自己探测）。
//    - Linux: unit 文件存在、`systemctl is-enabled/is-active/status buckyos.service`。
//    - Windows: 计划任务存在、Run 启动项存在、loader 脚本存在、注册表 root 存在。
//
// Green 判定要求：
// - 未激活设备：node_daemon + 3182 + activation HTTP 可用，可视为“待激活正常”。
// - 已激活设备：node_daemon + cyfs_gateway + system_config 可用，关键端口可达。
// - scheduler/verify_hub/control_panel 缺失时不应输出完全 Green；按影响范围给 Blue/Yellow。
// - 安装包形态下 HostControlState 异常时，即使当前进程还在，也不应输出完全 Green，因为重启后
//   可能无法自动恢复。
// - node_daemon 不可达但 run item 仍在运行时，不能判定 Green；应报告
//   ControllerDownWorkloadAlive，并推荐恢复 node_daemon/reconcile。
//
// =============================================================================
// 5. ensure_running / start：本机进入目标运行态
// =============================================================================
//
// 方法名建议：
// - node.ensure_running
// - node.start
// - node.start_operation
//
// `node.start` 可以作为 CLI/兼容命令保留，但协议语义应按 `ensure_running` 理解。
//
// struct NodeStartRequest
// - target: NodeControlTarget
// - mode: NodeStartMode
// - host_control_policy: NodeHostControlPolicy
// - update_policy: NodeUpdatePolicy
// - preflight: NodePreflightPolicy
// - wait: NodeWaitPolicy
// - reason
//
// enum NodeStartMode
// - Normal
//   已激活设备的正常启动/恢复运行。
// - Activation
//   未激活设备启动 node_daemon --enable_active，暴露 node active endpoint。
// - Recovery
//   进入本机恢复模式，尽量只启动最小诊断/恢复面。
// - DesktopDaemon
//   desktop_daemon 模式，管理本机 local app。
// - SafeMode
//   启动最小 kernel，不自动拉起 app/frame service。
//
// enum NodeHostControlPolicy
// - Auto
//   根据 detect_host_control 结果选择 ServiceManager、ScheduledLauncher 或 DirectProcess。
// - PreferServiceManager
// - PreferScheduledLauncher
// - DirectProcessOnly
// - RefuseIfUnknown
//
// enum NodeUpdatePolicy
// - NoUpdate
// - UseInstalled
// - UpdateBeforeStart
// - ReinstallBeforeStart
//
// enum NodePreflightPolicy
// - Skip
// - Basic
//   检查 buckyos_root、node_daemon binary、端口冲突、权限。
// - Strict
//   追加 rootfs 完整性、关键配置、磁盘空间、数据写入风险检查。
//
// enum NodeWaitPolicy
// - NoWait
// - UntilProcessStarted
// - UntilActivationReady
// - UntilMinimalKernelReady
// - UntilHealthy
// - TimeoutSecs(u64)
//
// struct NodeStartResponse
// - accepted: bool
// - operation: NodeOperationRef
// - immediate_report: Option<NodeCheckReport>
//
// start / ensure_running 执行阶段建议：
// - DetectHostControl
// - Preflight
// - StopConflictingProcesses
// - UpdateOrInstall
// - StartViaServiceManager / TriggerScheduledLauncher / SpawnDirectProcess
// - WaitForReadiness
// - Verify
//
// 当前平台 start 细化：
// - macOS ServiceManager（用户 LaunchAgent）:
//   - 读取 `etc/desktop_runtime.json`；未绑定用户、绑定其他用户、Login Items 已禁用、
//     旧 LaunchDaemon 残留都拒绝启动并说明原因，不退回 DirectProcess。
//   - 重写 plist（自启开时在 LaunchAgents，关时在 `local/node_daemon/`）。
//   - 未加载则 `launchctl bootstrap gui/<uid> <plist>`，随后 `launchctl kickstart gui/<uid>/<label>`。
//   - 不 `enable`/`disable`，不改登录自启偏好。
// - Linux ServiceManager:
//   - 校验 `/etc/systemd/system/buckyos.service`。
//   - `systemctl daemon-reload`。
//   - `systemctl enable buckyos.service`。
//   - `systemctl start buckyos.service`。
//   - 等待 node_daemon 进程和 3182/3180。
// - Windows ScheduledLauncher:
//   - 校验 root、`scripts/node_daemon_loader.vbs`、`scripts/node_daemon_loader.ps1` 和
//     `bin/node-daemon/node_daemon.exe`。
//   - 如果计划任务不存在，创建 `BuckyOSNodeDaemonKeepAlive`，`/SC MINUTE /MO 1`。
//   - 写入 Run 启动项 `BuckyOSDaemon`。
//   - 立即 `schtasks /Run /TN BuckyOSNodeDaemonKeepAlive`，不用等下一分钟。
//   - 等待 node_daemon 进程和 3182/3180。
//
// 与旧 `src/start.py` 的映射：
// - `--skip-update` => NodeUpdatePolicy::NoUpdate
// - 默认 update => NodeUpdatePolicy::UpdateBeforeStart
// - `--all` / `--reinstall` => NodeUpdatePolicy::ReinstallBeforeStart
// - 当前脚本直接启动 node_daemon --enable_active => NodeStartMode::Activation + DirectProcess
//
// =============================================================================
// 6. stop：本机受控停止
// =============================================================================
//
// 方法名建议：
// - node.stop
// - node.stop_operation
// - node.blackbox_stop
//
// stop 的正式语义：
// - 让当前设备上的目标停止到请求指定的状态。
// - 产品默认优先 graceful；安装/卸载/升级场景允许 graceful timeout 后 force。
// - dev/recovery 才允许 kill-all 作为默认兜底。
//
// struct NodeStopRequest
// - target: NodeControlTarget
// - mode: NodeStopMode
// - host_control_policy: NodeHostControlPolicy
// - blackbox_policy: NodeBlackboxStopPolicy
// - data_safety_policy: NodeDataSafetyPolicy
// - timeout_secs
// - force_after_timeout: bool
// - preflight: NodePreflightPolicy
// - reason
//
// enum NodeStopMode
// - Graceful
//   优先通过 node_daemon 控制面请求服务退出。
// - GracefulThenForce
//   先友好退出，超时后进入黑盒强制停止。Desktop 安装/卸载/升级的默认模型。
// - Quiesce
//   停止高风险写入，保留最小诊断面。
// - StopHostService
//   只停止 Host OS 管理的 node_daemon 服务单元/计划任务。按设计不应该清理 run item；
//   如果底层 service manager 会级联清理 run item，应在 preflight 中报告风险或拒绝。
// - BlackboxForce
//   不依赖 BuckyOS 控制面，识别并停止相关进程、容器和服务。
// - KillAll
//   开发/恢复兜底，等价于旧 `src/stop.py` 的杀干净行为。
// - PrepareReset
//   为 Mode-2 reset 或恢复流程做本机停机准备。
//
// enum NodeBlackboxStopPolicy
// - Disabled
// - BuckyOSProcessesOnly
//   只停止已知 BuckyOS 进程名。
// - IncludeManagedContainers
//   同时停止 BuckyOS 管理或命名明确匹配的容器。
// - IncludeHostService
//   同时停止 Host OS service / launchd / systemd / scheduled task。
// - FullRuntime
//   进程、容器、Host service 都纳入目标，但仍只限当前设备 BuckyOS Runtime。
//
// enum NodeDataSafetyPolicy
// - NoCheck
// - BestEffortCheck
// - RequireNoCriticalWrite
// - RequireRecoveryPoint
// - RefuseIfDataRiskUnknown
//
// struct NodeStopResponse
// - accepted: bool
// - operation: NodeOperationRef
// - immediate_report: Option<NodeCheckReport>
//
// Desktop 安装/卸载/升级默认停止逻辑：
// 1. DetectHostControl。
// 2. 先关闭 Host OS 自动拉起机制：
//    - macOS: `launchctl bootout gui/<uid>/com.buckyos.runtime`；旧版残留的
//      `system/buckyos.service` 也一并 bootout。
//    - Linux: `systemctl stop buckyos.service`。
//    - Windows: 删除 `BuckyOSNodeDaemonKeepAlive` 计划任务和 `BuckyOSDaemon` Run 启动项。
// 3. 如果 node_daemon 控制面可达，先请求 graceful stop local runtime。
// 4. 等待默认 30 秒。
// 5. 如果仍有相关进程、端口或容器残留，执行 BlackboxForce。
// 6. 再跑一次 minimal check 验证。
//
// 普通 node_daemon crash/restart 逻辑：
// - 不执行 BlackboxForce。
// - 不主动停止 run item。
// - 优先恢复 node_daemon，再由 node_daemon 根据 node_config reconcile run item。
// - 如果 node_daemon 持续失败，check 报告 ControllerDownWorkloadAlive，提示用户当前 workload
//   可能仍在提供服务，但已失去本机控制器收敛能力。
//
// 黑盒停止必须能识别：
// - node-daemon / node_daemon
// - scheduler
// - verify-hub / verify_hub
// - system-config / system_config
// - cyfs-gateway / cyfs_gateway
// - filebrowser
// - smb-service / smb_service
// - repo-service / repo_service
// - control-panel / control_panel
// - aicc
// - task_manager
// - kmsg
// - msg_center
// - opendan
// - workflow
// - BuckyOS/devtest 命名或 label 明确匹配的容器
//
// 当前实现差异：
// - `src/stop.py` 会杀 `workflow`，但 Windows 安装包携带的 `src/rootfs/bin/stop.ps1` 当前没有
//   `workflow`。node_control 落地时应统一进程清单，或把平台差异显式写进 capability。
// - macOS `bootout` 只停 controller（`AbandonProcessGroup=true`），不自动清理 run item；这符合
//   node_daemon crash 不杀 workload 的目标，升级/卸载仍需实例内黑盒清理。
// - Linux systemd unit 已显式使用 `KillMode=process`，与 macOS `AbandonProcessGroup=true`
//   一样只让 Host OS 管理 node_daemon 本身；升级/卸载需要完整停止 Runtime 时仍不能只包装
//   `systemctl stop buckyos.service`，必须走黑盒进程/容器清理。
//
// 与旧 `src/stop.py` 的映射：
// - 开发命令：NodeStopMode::KillAll + NodeBlackboxStopPolicy::FullRuntime + NoCheck
// - 产品停止：NodeStopMode::Graceful + RequireNoCriticalWrite
// - Desktop 升级：NodeStopMode::GracefulThenForce + IncludeManagedContainers + timeout_secs=30
// - 恢复停机：NodeStopMode::Quiesce 或 PrepareReset
//
// =============================================================================
// 7. restart：本机重启计划
// =============================================================================
//
// 方法名建议：
// - node.restart
// - node.restart_plan
//
// restart 不应简单等价于 stop + start；它要记录完整计划、控制路径和验证结果。
//
// struct NodeRestartRequest
// - target
// - stop_mode
// - start_mode
// - host_control_policy
// - blackbox_policy
// - data_safety_policy
// - update_policy
// - wait
// - reason
//
// struct NodeRestartPlan
// - operation_id
// - target
// - host_control
// - phases
// - risk
// - expected_downtime_secs
// - rollback_strategy
//
// =============================================================================
// 8. reset / recovery：本机特殊模式
// =============================================================================
//
// 方法名建议：
// - node.enter_recovery
// - node.prepare_mode2_reset
// - node.factory_reset_prepare
// - node.export_diagnostic_bundle
//
// enum NodeResetMode
// - FactoryReset
//   出厂重置。是否清除用户数据必须由硬件/installer/native helper 明确执行。
// - Mode2KeepUserData
//   保留用户数据，清理系统状态，让设备回到待激活或可重新加入状态。
//
// 注意：
// - node_control 只定义本机动作和前置检查。
// - 多设备重建、恢复拓扑、恢复用户数据属于 system_control。
//
// =============================================================================
// 9. Operation 查询与审计
// =============================================================================
//
// 方法名建议：
// - node.operation_get
// - node.operation_cancel
// - node.operation_approve
// - node.operation_list
//
// struct NodeOperation
// - id
// - target
// - action
// - status
// - host_control
// - risk
// - phases
// - created_at
// - updated_at
// - audit_log
// - verification_report
//
// enum NodeOperationPhaseKind
// - DetectHostControl
// - Preflight
// - ConfirmRecoveryPoint
// - Quiesce
// - StopViaControlPlane
// - StopHostService
// - StopContainers
// - KillProcesses
// - Update
// - StartViaServiceManager
// - TriggerScheduledLauncher
// - SpawnDirectProcess
// - Verify
// - Rollback
//
// =============================================================================
// 10. Host 生命周期入口
// =============================================================================
//
// 生产环境由 installer / updater / service manager 调用 host lifecycle API。
// 源码开发环境仍使用 `src/check.py`、`src/start.py` 和 `src/stop.py`。
//
// =============================================================================
// 11. 仍需确认的问题
// =============================================================================
//
// 1. 容器识别规则应优先使用 Docker label 还是命名约定。目前 `src/stop.py` 只杀
//    `devtest-*` 容器，产品环境需要更明确的边界。: /tmp/buckyos/run.plist 有记录
// 2. node_daemon graceful stop 的 live API 名称和最小可用前提尚未固定。还未设计
// 3. Windows `stop.ps1` 与 `src/stop.py` 的进程清单需要对齐，尤其是 `workflow`：通过/tmp/buckyos/run.plist 解决
// 4. macOS/Linux service stop 后是否应默认做一次黑盒残留清理，需要在“产品停止”和“升级/卸载停止”
//    之间保持不同策略。 ：要做的，node_daemon设计上停止后，不会杀他拉起的run item
// 5. DataSafetyState 当前版本默认应为 NotEvaluated；如果要在 stop 前阻断操作，需要 system_control
//    或专门的数据安全检查提供更强信号。: 下个版本再做

// =============================================================================
// Implementation
// =============================================================================
//
// 当前版本目标：在正式环境下替代 src/start.py、src/stop.py、src/check.py 中
// 与本机 BuckyOS Runtime 有关的能力，并支撑 Desktop 安装/卸载/升级时的本机停止
// 与重启需求。所有跨平台动作走 std::process::Command，避免引入额外依赖。
//
// 关键差异：
// - stop 不再写死进程清单。优先读取 /tmp/buckyos/run.plist（由 node_daemon 维护），
//   驱动 kernel_service 进程清单和 app_service 容器清单；run.plist 不可用时退回
//   保底清单（与历史 stop.py 对齐，含 workflow）。
// - check 同时报告 HostControlState；安装包形态下若 Host OS 服务管理器异常，
//   即使当前进程在跑也不会判 Green。
// - start / stop 优先走 Host OS 服务管理器（launchctl / systemctl / schtasks）；
//   失败或不可用时退回直接 spawn / 黑盒杀。

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::process_util::{hide_background_child_console, hide_child_console};

use serde::{Deserialize, Serialize};

const TCP_PROBE_TIMEOUT: Duration = Duration::from_millis(1500);
const HTTP_PROBE_TIMEOUT: Duration = Duration::from_millis(2000);

// -----------------------------------------------------------------------------
// 平台 / 控制模型类型
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodePlatform {
    MacOS,
    Linux,
    Windows,
    Unknown,
}

impl NodePlatform {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            NodePlatform::MacOS
        } else if cfg!(target_os = "linux") {
            NodePlatform::Linux
        } else if cfg!(target_os = "windows") {
            NodePlatform::Windows
        } else {
            NodePlatform::Unknown
        }
    }

    pub fn exe_suffix(&self) -> &'static str {
        match self {
            NodePlatform::Windows => ".exe",
            _ => "",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeHostControlModel {
    ServiceManager,
    ScheduledLauncher,
    DirectProcess,
    ContainerizedRuntime,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Confidence {
    Confirmed,
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeHostControlState {
    pub model: NodeHostControlModel,
    pub platform: NodePlatform,
    pub buckyos_root: PathBuf,
    pub node_daemon_binary: PathBuf,
    pub service_unit_name: Option<String>,
    pub service_plist_path: Option<PathBuf>,
    pub scheduled_task_name: Option<String>,
    pub run_key_name: Option<String>,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
    pub service_enabled: Option<bool>,
    pub service_active: Option<bool>,
    /// macOS: the bound user's LaunchAgent; `None` when the root is not bound.
    pub launch_agent: Option<MacLaunchAgentState>,
    /// macOS: the Runtime bundle is installed but no OS user owns it yet.
    pub pending_user_binding: bool,
    /// macOS: a pre-LaunchAgent system job is still registered (conflict, not an entry).
    pub legacy_service: Option<PathBuf>,
}

// -----------------------------------------------------------------------------
// Check / Fault 模型
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeFaultLevel {
    Green,
    Blue,
    Yellow,
    Orange,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeAvailabilityLayer {
    HostUnreachable,
    HostReachable,
    ActivationEndpointReachable,
    NodeDaemonReachable,
    ControllerDownWorkloadAlive,
    MinimalKernelReachable,
    FullRuntimeReachable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckItem {
    pub name: String,
    pub status: CheckStatus,
    pub summary: String,
    pub details: Vec<String>,
}

impl CheckItem {
    fn new(name: impl Into<String>, status: CheckStatus, summary: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status,
            summary: summary.into(),
            details: Vec::new(),
        }
    }

    fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.details.push(detail.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NodeLiveProbeReport {
    pub activation_endpoint_ok: bool,
    pub node_gateway_ok: bool,
    pub system_config_ok: bool,
    pub node_daemon_control_ok: bool,
    pub detail: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticFinding {
    pub severity: CheckStatus,
    pub title: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeCheckReport {
    pub overall: NodeFaultLevel,
    pub availability_layer: NodeAvailabilityLayer,
    pub buckyos_root: PathBuf,
    pub platform: NodePlatform,
    pub host_control: NodeHostControlState,
    pub activated: bool,
    pub activation_ready: bool,
    pub live_probe: NodeLiveProbeReport,
    pub checks: Vec<CheckItem>,
    pub log_findings: Vec<DiagnosticFinding>,
    pub title: String,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NodeCheckOptions {
    pub include_logs: bool,
    pub include_host_control: bool,
}

impl NodeCheckOptions {
    pub fn standard() -> Self {
        Self {
            include_logs: true,
            include_host_control: true,
        }
    }

    pub fn basic() -> Self {
        Self {
            include_logs: false,
            include_host_control: true,
        }
    }
}

// -----------------------------------------------------------------------------
// Start / Stop 请求
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStartMode {
    Normal,
    Activation,
    Recovery,
    DesktopDaemon,
    SafeMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeHostControlPolicy {
    Auto,
    PreferServiceManager,
    PreferScheduledLauncher,
    DirectProcessOnly,
    RefuseIfUnknown,
}

#[derive(Debug, Clone)]
pub struct NodeStartRequest {
    pub mode: NodeStartMode,
    pub host_control_policy: NodeHostControlPolicy,
    pub buckyos_root: Option<PathBuf>,
    pub stop_conflicting: bool,
    pub reason: Option<String>,
}

impl Default for NodeStartRequest {
    fn default() -> Self {
        Self {
            mode: NodeStartMode::Normal,
            host_control_policy: NodeHostControlPolicy::Auto,
            buckyos_root: None,
            stop_conflicting: true,
            reason: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NodeStartReport {
    pub used_model: NodeHostControlModel,
    pub actions: Vec<String>,
    pub started_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStopMode {
    Graceful,
    GracefulThenForce,
    StopHostService,
    BlackboxForce,
    KillAll,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeBlackboxStopPolicy {
    Disabled,
    BuckyOSProcessesOnly,
    IncludeManagedContainers,
    IncludeHostService,
    FullRuntime,
}

#[derive(Debug, Clone)]
pub struct NodeStopRequest {
    pub mode: NodeStopMode,
    pub blackbox_policy: NodeBlackboxStopPolicy,
    pub host_control_policy: NodeHostControlPolicy,
    pub timeout_secs: u64,
    pub buckyos_root: Option<PathBuf>,
    pub reason: Option<String>,
}

impl Default for NodeStopRequest {
    fn default() -> Self {
        Self {
            mode: NodeStopMode::GracefulThenForce,
            blackbox_policy: NodeBlackboxStopPolicy::IncludeManagedContainers,
            host_control_policy: NodeHostControlPolicy::Auto,
            timeout_secs: 30,
            buckyos_root: None,
            reason: None,
        }
    }
}

impl NodeStopRequest {
    /// 旧 src/stop.py 行为：开发兜底 kill-all。
    pub fn dev_kill_all() -> Self {
        Self {
            mode: NodeStopMode::KillAll,
            blackbox_policy: NodeBlackboxStopPolicy::FullRuntime,
            host_control_policy: NodeHostControlPolicy::Auto,
            timeout_secs: 5,
            buckyos_root: None,
            reason: Some("dev kill-all".into()),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct NodeStopReport {
    pub stopped_processes: Vec<String>,
    pub stopped_containers: Vec<String>,
    pub host_service_actions: Vec<String>,
    pub remaining: Vec<String>,
    pub actions: Vec<String>,
}

// -----------------------------------------------------------------------------
// 进程清单（保底清单 + run.plist 动态清单）
// -----------------------------------------------------------------------------

const PROCESS_KILL_BASELINE: &[&str] = &[
    "node-daemon",
    "node_daemon",
    "scheduler",
    "verify-hub",
    "verify_hub",
    "system-config",
    "system_config",
    "cyfs-gateway",
    "cyfs_gateway",
    "filebrowser",
    "smb-service",
    "smb_service",
    "nfs-server",
    "nfs_server",
    "aiworkspace",
    "homestation",
    "repo-service",
    "repo_service",
    "control-panel",
    "control_panel",
    "aicc",
    "task_manager",
    "task-manager",
    "kmsg",
    "msg_center",
    "msg-center",
    "opendan",
    "workflow",
];

const NODE_DAEMON_ALIASES: &[&str] = &["node-daemon", "node_daemon"];
const CYFS_GATEWAY_ALIASES: &[&str] = &["cyfs-gateway", "cyfs_gateway"];
const SYSTEM_CONFIG_ALIASES: &[&str] = &["system-config", "system_config"];
const SCHEDULER_ALIASES: &[&str] = &["scheduler"];
const VERIFY_HUB_ALIASES: &[&str] = &["verify-hub", "verify_hub"];
const CONTROL_PANEL_ALIASES: &[&str] = &["control-panel", "control_panel"];

const PORT_NODE_GATEWAY_HTTP: u16 = 3180;
const PORT_NODE_DAEMON_ACTIVATION: u16 = 3182;
const PORT_SYSTEM_CONFIG: u16 = 3200;
const PORT_VERIFY_HUB: u16 = 3300;
const PORT_CONTROL_PANEL: u16 = 4020;
const PORT_ZONE_GATEWAY_HTTP: u16 = 80;

// -----------------------------------------------------------------------------
// run.plist 读取
// -----------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct RunPlistItemRaw {
    item_name: String,
    item_kind: String,
    #[serde(default)]
    run_state: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct RunPlistRaw {
    #[serde(default)]
    items: BTreeMap<String, RunPlistItemRaw>,
}

#[derive(Debug, Clone)]
pub struct RunPlistEntry {
    pub name: String,
    pub kind: String,
    pub run_state: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunPlistSnapshot {
    pub path: PathBuf,
    pub items: Vec<RunPlistEntry>,
}

impl RunPlistSnapshot {
    pub fn kernel_service_aliases(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.kind == "kernel_service")
            .map(|i| i.name.clone())
            .collect()
    }

    pub fn frame_service_aliases(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.kind == "frame_service")
            .map(|i| i.name.clone())
            .collect()
    }

    /// app_service 容器命名约定：`{owner}#{app_name}` -> `{owner}-{app_name}`。
    pub fn app_service_container_names(&self) -> Vec<String> {
        self.items
            .iter()
            .filter(|i| i.kind == "app_service")
            .map(|i| i.name.replace('#', "-"))
            .collect()
    }

    pub fn all_process_aliases(&self) -> Vec<String> {
        let mut out: Vec<String> = self.kernel_service_aliases();
        out.extend(self.frame_service_aliases());
        out
    }
}

/// macOS keeps the run list inside the installation it belongs to (one bound
/// user per root); other platforms keep the historical shared location.
pub fn run_plist_path(buckyos_root: &Path) -> PathBuf {
    if cfg!(target_os = "windows") {
        std::env::temp_dir().join("buckyos").join("run.plist")
    } else if cfg!(target_os = "macos") {
        node_daemon_local_dir(buckyos_root).join("run.plist")
    } else {
        PathBuf::from("/tmp/buckyos/run.plist")
    }
}

pub fn read_run_plist(buckyos_root: &Path) -> Option<RunPlistSnapshot> {
    let path = run_plist_path(buckyos_root);
    let content = std::fs::read_to_string(&path).ok()?;
    let raw: RunPlistRaw = serde_json::from_str(&content).ok()?;
    let items = raw
        .items
        .into_values()
        .map(|item| RunPlistEntry {
            name: item.item_name,
            kind: item.item_kind,
            run_state: item.run_state,
        })
        .collect();
    Some(RunPlistSnapshot { path, items })
}

// -----------------------------------------------------------------------------
// 路径 / 二进制
// -----------------------------------------------------------------------------

pub fn resolve_buckyos_root() -> PathBuf {
    if let Ok(value) = std::env::var("BUCKYOS_ROOT") {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if NodePlatform::current() == NodePlatform::MacOS {
        if let Some(root) = current_user_home()
            .map(|home| macos_launch_agent_plist_path(&home))
            .and_then(|plist| std::fs::read_to_string(plist).ok())
            .and_then(|text| plist_env_value(&text, "BUCKYOS_ROOT"))
        {
            return PathBuf::from(root);
        }
    }
    match NodePlatform::current() {
        NodePlatform::Windows => {
            if let Ok(appdata) = std::env::var("APPDATA") {
                if !appdata.is_empty() {
                    return PathBuf::from(appdata).join("buckyos");
                }
            }
            if let Ok(profile) = std::env::var("USERPROFILE") {
                if !profile.is_empty() {
                    return PathBuf::from(profile).join("buckyos");
                }
            }
            PathBuf::from("C:\\buckyos")
        }
        _ => PathBuf::from("/opt/buckyos"),
    }
}

pub fn node_daemon_binary_path(buckyos_root: &Path) -> PathBuf {
    let dir = buckyos_root.join("bin").join("node-daemon");
    if cfg!(target_os = "windows") {
        dir.join("node_daemon.exe")
    } else if cfg!(target_os = "macos") {
        macos_runtime_bundle_path(buckyos_root)
            .join("Contents")
            .join("MacOS")
            .join("node_daemon")
    } else {
        dir.join("node_daemon")
    }
}

pub fn macos_runtime_bundle_path(buckyos_root: &Path) -> PathBuf {
    buckyos_root
        .join("bin")
        .join("node-daemon")
        .join(MACOS_RUNTIME_BUNDLE)
}

/// node_daemon's host-local state (`local/node_daemon`): executor ledger,
/// run list, desktop status, the manual-start LaunchAgent plist.
pub fn node_daemon_local_dir(buckyos_root: &Path) -> PathBuf {
    buckyos_root.join("local").join("node_daemon")
}

// -----------------------------------------------------------------------------
// macOS Desktop Runtime：绑定用户与用户 LaunchAgent
// -----------------------------------------------------------------------------

pub const MACOS_RUNTIME_BUNDLE: &str = "BuckyOS Runtime.app";
pub const MACOS_RUNTIME_LABEL: &str = "com.buckyos.runtime";
pub const MACOS_RUNTIME_ARGS: &[&str] = &["--enable_active", "--desktop_daemon"];
/// The daemon's service PATH (both Homebrew prefixes). The Executor's tool
/// PATH is configured by node_executor, not inherited from here.
pub const MACOS_SERVICE_PATH: &str =
    "/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";
pub const DESKTOP_RUNTIME_BINDING_FILE: &str = "desktop_runtime.json";
pub const DESKTOP_STATUS_FILE: &str = "desktop_status.json";
const DESKTOP_STATUS_STALE_SECS: u64 = 120;
pub const MACOS_LEGACY_DAEMON_PLIST: &str = "/Library/LaunchDaemons/buckyos.service.plist";
pub const MACOS_LEGACY_LABEL: &str = "buckyos.service";
/// Runtime-writable directories the bound user must own; the Runtime bundle
/// (`bin/node-daemon`) stays installer-owned.
const MACOS_RUNTIME_DIRS: &[&str] = &[
    "etc",
    "security",
    "data",
    "storage",
    "local",
    "local/node_daemon",
    "logs",
    "logs/node_daemon",
    "bin/agent-tools",
];

/// The OS user an installation belongs to (`etc/desktop_runtime.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopRuntimeBinding {
    pub user: String,
    pub uid: u32,
    pub home: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MacLaunchAgentState {
    pub binding: DesktopRuntimeBinding,
    /// `gui/<uid>`
    pub domain: String,
    pub label: String,
    /// The plist start bootstraps: the LaunchAgents copy while autostart is on.
    pub plist_path: PathBuf,
    /// Login autostart preference: the plist is in `~/Library/LaunchAgents`.
    pub autostart: bool,
    /// The bound user has a GUI (Aqua) launchd domain, i.e. is logged in.
    /// `None` when the caller may not inspect that user's domain.
    pub gui_session: Option<bool>,
    pub loaded: Option<bool>,
    pub pid: Option<u32>,
    /// Turned off by the user or the system (Login Items); node_control never
    /// disables the job itself.
    pub disabled: bool,
}

#[derive(Debug, Clone)]
pub struct OsUser {
    pub name: String,
    pub uid: u32,
    pub gid: u32,
    pub home: PathBuf,
}

pub fn desktop_binding_path(buckyos_root: &Path) -> PathBuf {
    buckyos_root.join("etc").join(DESKTOP_RUNTIME_BINDING_FILE)
}

pub fn read_desktop_binding(buckyos_root: &Path) -> Option<DesktopRuntimeBinding> {
    let text = std::fs::read_to_string(desktop_binding_path(buckyos_root)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn macos_launch_agent_plist_path(home: &Path) -> PathBuf {
    home.join("Library")
        .join("LaunchAgents")
        .join(format!("{}.plist", MACOS_RUNTIME_LABEL))
}

fn macos_manual_plist_path(buckyos_root: &Path) -> PathBuf {
    node_daemon_local_dir(buckyos_root).join(format!("{}.plist", MACOS_RUNTIME_LABEL))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// The one LaunchAgent definition every entry (installer, App, dev scripts)
/// writes for a bound root.
pub fn render_macos_launch_agent(buckyos_root: &Path, binding: &DesktopRuntimeBinding) -> String {
    let root = xml_escape(&buckyos_root.to_string_lossy());
    let logs = buckyos_root.join("logs").join("node_daemon");
    let mut arguments = format!(
        "        <string>{}</string>\n",
        xml_escape(&node_daemon_binary_path(buckyos_root).to_string_lossy())
    );
    for arg in MACOS_RUNTIME_ARGS {
        arguments.push_str(&format!("        <string>{}</string>\n", arg));
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>AssociatedBundleIdentifiers</key>
    <array>
        <string>{label}</string>
    </array>
    <key>ProgramArguments</key>
    <array>
{arguments}    </array>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>WorkingDirectory</key>
    <string>{root}</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>BUCKYOS_ROOT</key>
        <string>{root}</string>
        <key>HOME</key>
        <string>{home}</string>
        <key>PATH</key>
        <string>{path}</string>
        <key>RUST_BACKTRACE</key>
        <string>1</string>
    </dict>
    <key>StandardOutPath</key>
    <string>{out}</string>
    <key>StandardErrorPath</key>
    <string>{err}</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>ThrottleInterval</key>
    <integer>10</integer>
    <key>ExitTimeOut</key>
    <integer>30</integer>
    <key>AbandonProcessGroup</key>
    <true/>
</dict>
</plist>
"#,
        label = MACOS_RUNTIME_LABEL,
        arguments = arguments,
        root = root,
        home = xml_escape(&binding.home.to_string_lossy()),
        path = MACOS_SERVICE_PATH,
        out = xml_escape(&logs.join("launchd.out.log").to_string_lossy()),
        err = xml_escape(&logs.join("launchd.err.log").to_string_lossy()),
    )
}

/// `EnvironmentVariables.<key>` of a plist rendered by `render_macos_launch_agent`.
fn plist_env_value(text: &str, key: &str) -> Option<String> {
    let marker = format!("<key>{}</key>", key);
    let rest = &text[text.find(&marker)? + marker.len()..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = start + rest[start..].find("</string>")?;
    Some(xml_unescape(rest[start..end].trim())).filter(|value| !value.is_empty())
}

#[cfg(unix)]
fn passwd_entry(name: Option<&str>, uid: u32) -> Option<OsUser> {
    use std::ffi::{CStr, CString};
    let mut entry: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buffer = vec![0 as libc::c_char; 16 * 1024];
    let mut found: *mut libc::passwd = std::ptr::null_mut();
    let rc = match name {
        Some(name) => {
            let name = CString::new(name).ok()?;
            unsafe {
                libc::getpwnam_r(
                    name.as_ptr(),
                    &mut entry,
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut found,
                )
            }
        }
        None => unsafe {
            libc::getpwuid_r(
                uid,
                &mut entry,
                buffer.as_mut_ptr(),
                buffer.len(),
                &mut found,
            )
        },
    };
    if rc != 0 || found.is_null() {
        return None;
    }
    let text = |ptr: *const libc::c_char| {
        unsafe { CStr::from_ptr(ptr) }
            .to_string_lossy()
            .into_owned()
    };
    Some(OsUser {
        name: text(entry.pw_name),
        uid: entry.pw_uid,
        gid: entry.pw_gid,
        home: PathBuf::from(text(entry.pw_dir)),
    })
}

#[cfg(not(unix))]
fn passwd_entry(_name: Option<&str>, _uid: u32) -> Option<OsUser> {
    None
}

pub fn lookup_os_user(name: &str) -> Option<OsUser> {
    passwd_entry(Some(name), 0)
}

/// The account of this process's effective uid.
pub fn current_os_user() -> Option<OsUser> {
    passwd_entry(None, current_euid())
}

pub fn current_euid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

fn current_user_home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(|| passwd_entry(None, current_euid()).map(|user| user.home))
}

fn launchctl(args: &[&str]) -> (bool, String) {
    let mut command = Command::new("launchctl");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match hide_child_console(&mut command).output() {
        Ok(out) => (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).to_string(),
        ),
        Err(_) => (false, String::new()),
    }
}

fn parse_launchctl_pid(print: &str) -> Option<u32> {
    print
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("pid = "))
        .and_then(|value| value.trim().parse().ok())
}

fn parse_launchctl_disabled(print_disabled: &str, label: &str) -> bool {
    let quoted = format!("\"{}\"", label);
    print_disabled
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(&quoted))
        .any(|line| line.ends_with("disabled") || line.ends_with("true"))
}

fn detect_macos_launch_agent(
    buckyos_root: &Path,
    binding: DesktopRuntimeBinding,
) -> MacLaunchAgentState {
    let domain = format!("gui/{}", binding.uid);
    let installed = macos_launch_agent_plist_path(&binding.home);
    let autostart = installed.is_file();
    let plist_path = if autostart {
        installed
    } else {
        macos_manual_plist_path(buckyos_root)
    };
    let mut state = MacLaunchAgentState {
        domain,
        label: MACOS_RUNTIME_LABEL.to_string(),
        plist_path,
        autostart,
        gui_session: None,
        loaded: None,
        pid: None,
        disabled: false,
        binding,
    };
    let euid = current_euid();
    if euid != 0 && euid != state.binding.uid {
        return state;
    }
    let (loaded, print) = launchctl(&["print", &format!("{}/{}", state.domain, state.label)]);
    state.loaded = Some(loaded);
    state.pid = parse_launchctl_pid(&print);
    state.gui_session = Some(loaded || launchctl(&["print", &state.domain]).0);
    let (_, disabled) = launchctl(&["print-disabled", &state.domain]);
    state.disabled = parse_launchctl_disabled(&disabled, &state.label);
    state
}

fn ensure_agent_controllable(agent: &MacLaunchAgentState) -> Result<(), String> {
    let euid = current_euid();
    if euid == 0 || euid == agent.binding.uid {
        return Ok(());
    }
    Err(format!(
        "BuckyOS Runtime here belongs to OS user {} (uid {}); it cannot be controlled from uid {}",
        agent.binding.user, agent.binding.uid, euid
    ))
}

#[cfg(unix)]
fn give_to(path: &Path, uid: u32, gid: u32) -> std::io::Result<()> {
    if current_euid() == 0 {
        std::os::unix::fs::lchown(path, Some(uid), Some(gid))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn give_to(_path: &Path, _uid: u32, _gid: u32) -> std::io::Result<()> {
    Ok(())
}

fn write_owned_file(path: &Path, content: &[u8], uid: u32, gid: u32) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    if !parent.is_dir() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create {} failed: {}", parent.display(), e))?;
        give_to(parent, uid, gid)
            .map_err(|e| format!("chown {} failed: {}", parent.display(), e))?;
    }
    let temporary = parent.join(format!(
        ".{}.tmp",
        path.file_name().and_then(OsStr::to_str).unwrap_or("file")
    ));
    std::fs::write(&temporary, content)
        .map_err(|e| format!("write {} failed: {}", temporary.display(), e))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o644));
    }
    give_to(&temporary, uid, gid)
        .map_err(|e| format!("chown {} failed: {}", temporary.display(), e))?;
    std::fs::rename(&temporary, path)
        .map_err(|e| format!("replace {} failed: {}", path.display(), e))
}

fn write_macos_launch_agent(
    buckyos_root: &Path,
    binding: &DesktopRuntimeBinding,
    plist: &Path,
    actions: &mut Vec<String>,
) -> Result<(), String> {
    let gid = passwd_entry(None, binding.uid)
        .map(|user| user.gid)
        .unwrap_or(20);
    let content = render_macos_launch_agent(buckyos_root, binding);
    if std::fs::read_to_string(plist).ok().as_deref() != Some(content.as_str()) {
        write_owned_file(plist, content.as_bytes(), binding.uid, gid)?;
        actions.push(format!("wrote LaunchAgent {}", plist.display()));
    }
    Ok(())
}

#[cfg(unix)]
fn chown_tree(path: &Path, uid: u32, gid: u32, skip: Option<&Path>) -> std::io::Result<()> {
    if skip == Some(path) {
        return Ok(());
    }
    std::os::unix::fs::lchown(path, Some(uid), Some(gid))?;
    if std::fs::symlink_metadata(path)?.is_dir() {
        for entry in std::fs::read_dir(path)? {
            chown_tree(&entry?.path(), uid, gid, skip)?;
        }
    }
    Ok(())
}

/// Installer-side (root) preparation of a root for its bound user: the user
/// owns everything the Runtime writes, the bundle stays root-owned, and a
/// ledger left by a root (A-system) node_daemon is set aside, not adopted.
#[cfg(unix)]
fn prepare_macos_root_ownership(
    buckyos_root: &Path,
    user: &OsUser,
    actions: &mut Vec<String>,
) -> Result<(), String> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    for dir in MACOS_RUNTIME_DIRS {
        let path = buckyos_root.join(dir);
        std::fs::create_dir_all(&path)
            .map_err(|e| format!("create {} failed: {}", path.display(), e))?;
    }
    let ledger = node_daemon_local_dir(buckyos_root).join("executor");
    if std::fs::symlink_metadata(&ledger)
        .map(|m| m.uid() == 0)
        .unwrap_or(false)
    {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let aside = ledger.with_file_name(format!("executor.legacy-root-{}", stamp));
        std::fs::rename(&ledger, &aside)
            .map_err(|e| format!("set aside {} failed: {}", ledger.display(), e))?;
        actions.push(format!(
            "set aside root executor ledger (not migrated): {}",
            aside.display()
        ));
    }
    let bundle_module = buckyos_root.join("bin").join("node-daemon");
    chown_tree(buckyos_root, user.uid, user.gid, Some(&bundle_module))
        .map_err(|e| format!("chown {} failed: {}", buckyos_root.display(), e))?;
    if bundle_module.exists() {
        chown_tree(&bundle_module, 0, 0, None)
            .map_err(|e| format!("chown {} failed: {}", bundle_module.display(), e))?;
    }
    let security = buckyos_root.join("security");
    std::fs::set_permissions(&security, std::fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("chmod {} failed: {}", security.display(), e))?;
    actions.push(format!(
        "{} owned by {} (uid {}), Runtime bundle kept root-owned",
        buckyos_root.display(),
        user.name,
        user.uid
    ));
    Ok(())
}

#[cfg(not(unix))]
fn prepare_macos_root_ownership(
    _buckyos_root: &Path,
    _user: &OsUser,
    _actions: &mut Vec<String>,
) -> Result<(), String> {
    Err("Desktop Runtime binding is macOS only".into())
}

#[cfg(unix)]
fn path_owner(path: &Path) -> Option<u32> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|m| m.uid())
}

#[cfg(not(unix))]
fn path_owner(_path: &Path) -> Option<u32> {
    None
}

/// Bind a root to one OS user and install its login LaunchAgent (autostart
/// on). Run as root by the installer, or by that user for a root it already
/// owns (development). Does not start the Runtime.
pub fn macos_register_runtime(buckyos_root: &Path, user: &str) -> Result<Vec<String>, String> {
    let target = lookup_os_user(user).ok_or_else(|| format!("unknown OS user: {}", user))?;
    if target.uid == 0 {
        return Err("the Desktop Runtime cannot be bound to root".into());
    }
    if let Some(existing) = read_desktop_binding(buckyos_root) {
        if existing.uid != target.uid {
            return Err(format!(
                "{} is already bound to OS user {} (uid {}); unregister it first",
                buckyos_root.display(),
                existing.user,
                existing.uid
            ));
        }
    }
    let euid = current_euid();
    if euid != 0 && euid != target.uid {
        return Err(format!(
            "only root or {} can bind this Runtime to {}",
            user, user
        ));
    }
    let identity = buckyos_root.join("etc").join("node_identity.json");
    if let Some(owner) = path_owner(&identity).filter(|uid| *uid != 0 && *uid != target.uid) {
        return Err(format!(
            "{} belongs to uid {}; another OS user cannot take over this node",
            identity.display(),
            owner
        ));
    }
    let mut actions = Vec::new();
    if euid == 0 {
        prepare_macos_root_ownership(buckyos_root, &target, &mut actions)?;
    } else {
        if path_owner(buckyos_root) != Some(target.uid) {
            return Err(format!(
                "{} is not owned by {}; bind it with sudo (installer) instead",
                buckyos_root.display(),
                user
            ));
        }
        for dir in MACOS_RUNTIME_DIRS {
            let path = buckyos_root.join(dir);
            std::fs::create_dir_all(&path)
                .map_err(|e| format!("create {} failed: {}", path.display(), e))?;
        }
    }
    let binding = DesktopRuntimeBinding {
        user: target.name.clone(),
        uid: target.uid,
        home: target.home.clone(),
    };
    let body = serde_json::to_vec_pretty(&binding).map_err(|e| e.to_string())?;
    write_owned_file(
        &desktop_binding_path(buckyos_root),
        &body,
        target.uid,
        target.gid,
    )?;
    actions.push(format!(
        "bound {} to OS user {} (uid {}, home {})",
        buckyos_root.display(),
        binding.user,
        binding.uid,
        binding.home.display()
    ));
    write_macos_launch_agent(
        buckyos_root,
        &binding,
        &macos_launch_agent_plist_path(&binding.home),
        &mut actions,
    )?;
    Ok(actions)
}

/// Remove the login item and the binding of a root (uninstall).
pub fn macos_unregister_runtime(buckyos_root: &Path) -> Result<Vec<String>, String> {
    let mut actions = Vec::new();
    let Some(binding) = read_desktop_binding(buckyos_root) else {
        actions.push(format!("{} is not bound", buckyos_root.display()));
        return Ok(actions);
    };
    let agent = detect_macos_launch_agent(buckyos_root, binding.clone());
    ensure_agent_controllable(&agent)?;
    if agent.loaded == Some(true) {
        run_quiet(
            "launchctl",
            &["bootout", &format!("{}/{}", agent.domain, agent.label)],
            &mut actions,
        );
    }
    for path in [
        macos_launch_agent_plist_path(&binding.home),
        macos_manual_plist_path(buckyos_root),
        desktop_binding_path(buckyos_root),
    ] {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|e| format!("remove {} failed: {}", path.display(), e))?;
            actions.push(format!("removed {}", path.display()));
        }
    }
    Ok(actions)
}

/// Login autostart only; whether the Runtime runs now is left unchanged.
pub fn macos_set_autostart(buckyos_root: &Path, enabled: bool) -> Result<Vec<String>, String> {
    let binding = read_desktop_binding(buckyos_root)
        .ok_or_else(|| format!("{} is not bound to an OS user", buckyos_root.display()))?;
    let agent = detect_macos_launch_agent(buckyos_root, binding.clone());
    ensure_agent_controllable(&agent)?;
    let installed = macos_launch_agent_plist_path(&binding.home);
    let mut actions = Vec::new();
    if enabled {
        write_macos_launch_agent(buckyos_root, &binding, &installed, &mut actions)?;
    } else if installed.exists() {
        std::fs::remove_file(&installed)
            .map_err(|e| format!("remove {} failed: {}", installed.display(), e))?;
        actions.push(format!("removed login item {}", installed.display()));
    }
    if agent.disabled {
        actions.push(
            "note: BuckyOS Runtime is turned off in System Settings > General > Login Items".into(),
        );
    }
    Ok(actions)
}

/// The Runtime's own view of its desktop session and privacy permissions
/// (`local/node_daemon/desktop_status.json`, written by node_daemon in
/// desktop mode) and its age in seconds.
pub fn read_desktop_status(buckyos_root: &Path) -> Option<(serde_json::Value, u64)> {
    let path = node_daemon_local_dir(buckyos_root).join(DESKTOP_STATUS_FILE);
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    let age = modified.elapsed().map(|d| d.as_secs()).unwrap_or_default();
    let value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    Some((value, age))
}

/// Processes of this installation: owned by its user, executable under its root.
struct InstanceScope {
    uid: u32,
    roots: Vec<String>,
}

impl InstanceScope {
    fn contains(&self, process: &ProcessInfo) -> bool {
        process.pid != std::process::id()
            && process.uid == Some(self.uid)
            && self
                .roots
                .iter()
                .any(|root| process.command.starts_with(root.as_str()))
    }
}

fn instance_scope(host: &NodeHostControlState) -> Option<InstanceScope> {
    if host.platform != NodePlatform::MacOS {
        return None;
    }
    let uid = host
        .launch_agent
        .as_ref()
        .map(|agent| agent.binding.uid)
        .unwrap_or_else(current_euid);
    let mut roots = vec![format!("{}/", host.buckyos_root.display())];
    if let Ok(real) = host.buckyos_root.canonicalize() {
        let real = format!("{}/", real.display());
        if !roots.contains(&real) {
            roots.push(real);
        }
    }
    Some(InstanceScope { uid, roots })
}

/// node_executor's worker subcommand: workers share the node_daemon binary
/// but are not controllers.
const EXECUTOR_WORKER_ARG: &str = "--executor-worker";

fn instance_node_daemons<'a>(
    host: &NodeHostControlState,
    processes: &'a [ProcessInfo],
) -> Vec<&'a ProcessInfo> {
    match instance_scope(host) {
        Some(scope) => processes
            .iter()
            .filter(|p| {
                scope.contains(p)
                    && p.command.ends_with("/node_daemon")
                    && !p.args.contains(EXECUTOR_WORKER_ARG)
            })
            .collect(),
        None => find_processes(processes, NODE_DAEMON_ALIASES),
    }
}

// -----------------------------------------------------------------------------
// 进程列表 / 端口列表
// -----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    /// Executable path on macOS (`ps -o comm`), short name on Linux.
    pub command: String,
    pub args: String,
    pub uid: Option<u32>,
}

fn normalize_name(value: &str) -> String {
    let mut normalized = value.trim().to_lowercase().replace('_', "-");
    for suffix in [".exe", ".cmd", ".bat"] {
        if normalized.ends_with(suffix) {
            normalized.truncate(normalized.len() - suffix.len());
            break;
        }
    }
    normalized
}

fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = hide_child_console(&mut command).output().ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

pub fn collect_processes() -> Vec<ProcessInfo> {
    if cfg!(target_os = "windows") {
        return collect_processes_windows();
    }
    // One column per call keeps paths with spaces (the Runtime bundle) intact.
    let args_by_pid: BTreeMap<u32, String> = run_capture("ps", &["-axo", "pid=,args="])
        .map(|raw| {
            parse_ps_rows(&raw, 0)
                .into_iter()
                .map(|(pid, _, rest)| (pid, rest))
                .collect()
        })
        .unwrap_or_default();
    let raw = match run_capture("ps", &["-axo", "pid=,uid=,comm="]) {
        Some(text) => text,
        None => return Vec::new(),
    };
    parse_ps_rows(&raw, 1)
        .into_iter()
        .map(|(pid, fields, command)| {
            let args = args_by_pid
                .get(&pid)
                .cloned()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| command.clone());
            ProcessInfo {
                pid,
                command,
                args,
                uid: fields.first().and_then(|v| v.parse().ok()),
            }
        })
        .collect()
}

/// `pid <fields...> <rest of line>` rows of `ps -o ...=`.
fn parse_ps_rows(raw: &str, fields: usize) -> Vec<(u32, Vec<String>, String)> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let mut rest = line.trim_start();
        let mut values = Vec::new();
        for _ in 0..=fields {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            values.push(rest[..end].to_string());
            rest = rest[end..].trim_start();
        }
        let Ok(pid) = values[0].parse::<u32>() else {
            continue;
        };
        out.push((pid, values.split_off(1), rest.trim_end().to_string()));
    }
    out
}

fn collect_processes_windows() -> Vec<ProcessInfo> {
    let script = "Get-CimInstance Win32_Process | Select-Object ProcessId, Name, CommandLine | ConvertTo-Csv -NoTypeInformation";
    let raw = match run_capture("powershell", &["-NoProfile", "-Command", script]) {
        Some(text) => text,
        None => return Vec::new(),
    };
    let mut lines = raw.lines();
    let _ = lines.next();
    let mut out = Vec::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // CSV with "" quoting; small ad-hoc parser
        let fields = parse_csv_line(line);
        if fields.len() < 3 {
            continue;
        }
        let pid: u32 = match fields[0].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let command = fields[1].clone();
        let args = if fields[2].is_empty() {
            command.clone()
        } else {
            fields[2].clone()
        };
        out.push(ProcessInfo {
            pid,
            command,
            args,
            uid: None,
        });
    }
    out
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if in_quotes {
            if ch == '"' {
                if let Some('"') = chars.peek() {
                    current.push('"');
                    chars.next();
                } else {
                    in_quotes = false;
                }
            } else {
                current.push(ch);
            }
        } else {
            match ch {
                ',' => {
                    fields.push(std::mem::take(&mut current));
                }
                '"' => in_quotes = true,
                _ => current.push(ch),
            }
        }
    }
    fields.push(current);
    fields
}

fn process_command_basename(p: &ProcessInfo) -> String {
    let name = Path::new(&p.command)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(&p.command);
    normalize_name(name)
}

fn process_args0_basename(p: &ProcessInfo) -> String {
    let first = p.args.split_whitespace().next().unwrap_or(&p.command);
    let name = Path::new(first)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(first);
    normalize_name(name)
}

pub fn process_matches<S: AsRef<str>>(p: &ProcessInfo, aliases: &[S]) -> bool {
    let normalized: Vec<String> = aliases.iter().map(|s| normalize_name(s.as_ref())).collect();
    let base = process_command_basename(p);
    let arg0 = process_args0_basename(p);
    if normalized
        .iter()
        .any(|alias| alias == &base || alias == &arg0)
    {
        return true;
    }
    let haystack = normalize_name(&p.args);
    normalized
        .iter()
        .any(|alias| !alias.is_empty() && haystack.contains(alias.as_str()))
}

pub fn find_processes<'a, S: AsRef<str>>(
    processes: &'a [ProcessInfo],
    aliases: &[S],
) -> Vec<&'a ProcessInfo> {
    processes
        .iter()
        .filter(|p| process_matches(*p, aliases))
        .collect()
}

pub fn probe_tcp(port: u16) -> bool {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    TcpStream::connect_timeout(&addr, TCP_PROBE_TIMEOUT).is_ok()
}

/// 简易 HTTP GET，仅用作健康探测；返回 (是否收到 HTTP 响应, status code)。
pub fn probe_http(port: u16, path: &str) -> (bool, Option<u16>) {
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let mut stream = match TcpStream::connect_timeout(&addr, TCP_PROBE_TIMEOUT) {
        Ok(s) => s,
        Err(_) => return (false, None),
    };
    let _ = stream.set_read_timeout(Some(HTTP_PROBE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(HTTP_PROBE_TIMEOUT));
    let req = format!(
        "GET {} HTTP/1.0\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\n\r\n",
        path, port
    );
    if stream.write_all(req.as_bytes()).is_err() {
        return (false, None);
    }
    let mut buf = [0u8; 256];
    let read_bytes = match stream.read(&mut buf) {
        Ok(n) => n,
        Err(_) => return (false, None),
    };
    if read_bytes == 0 {
        return (false, None);
    }
    let head = std::str::from_utf8(&buf[..read_bytes]).unwrap_or("");
    if !head.starts_with("HTTP/") {
        return (true, None);
    }
    let status = head
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok());
    (true, status)
}

pub fn get_port_listener(port: u16) -> Option<String> {
    if cfg!(target_os = "windows") {
        let raw = run_capture("netstat", &["-ano", "-p", "tcp"])?;
        for line in raw.lines() {
            let lower = line.to_lowercase();
            if !lower.contains("listening") {
                continue;
            }
            if line.contains(&format!(":{}", port)) {
                return Some(line.trim().to_string());
            }
        }
        return None;
    }

    if let Some(raw) = run_capture("lsof", &["-nP", &format!("-iTCP:{}", port), "-sTCP:LISTEN"]) {
        let mut lines = raw.lines();
        let _ = lines.next();
        if let Some(line) = lines.next() {
            return Some(line.trim().to_string());
        }
    }
    if let Some(raw) = run_capture("ss", &["-lntp"]) {
        let needle = format!(":{}", port);
        for line in raw.lines() {
            if line.contains(&needle) {
                return Some(line.trim().to_string());
            }
        }
    }
    None
}

// -----------------------------------------------------------------------------
// Host control 检测
// -----------------------------------------------------------------------------

pub fn detect_host_control_state() -> NodeHostControlState {
    detect_host_control_state_at(resolve_buckyos_root())
}

pub fn detect_host_control_state_at(buckyos_root: PathBuf) -> NodeHostControlState {
    let node_daemon = node_daemon_binary_path(&buckyos_root);
    let platform = NodePlatform::current();
    let mut evidence = Vec::new();
    let mut model = NodeHostControlModel::Unknown;
    let mut confidence = Confidence::Low;
    let mut service_unit_name = None;
    let mut service_plist_path = None;
    let mut scheduled_task_name = None;
    let mut run_key_name = None;
    let mut service_enabled = None;
    let mut service_active = None;
    let mut launch_agent = None;
    let mut pending_user_binding = false;
    let mut legacy_service = None;

    match platform {
        NodePlatform::MacOS => {
            let legacy = PathBuf::from(MACOS_LEGACY_DAEMON_PLIST);
            if legacy.is_file() {
                evidence.push(format!(
                    "legacy system LaunchDaemon still registered: {} (reinstall or uninstall removes it)",
                    legacy.display()
                ));
                legacy_service = Some(legacy);
            }
            if let Some(binding) = read_desktop_binding(&buckyos_root) {
                let agent = detect_macos_launch_agent(&buckyos_root, binding);
                model = NodeHostControlModel::ServiceManager;
                confidence = Confidence::High;
                service_unit_name = Some(agent.label.clone());
                service_plist_path = Some(agent.plist_path.clone());
                service_enabled = Some(!agent.disabled);
                service_active = agent.loaded.map(|loaded| loaded && agent.pid.is_some());
                evidence.push(format!(
                    "bound to OS user {} (uid {}); {}/{} autostart={} loaded={:?} pid={:?}",
                    agent.binding.user,
                    agent.binding.uid,
                    agent.domain,
                    agent.label,
                    agent.autostart,
                    agent.loaded,
                    agent.pid
                ));
                if agent.disabled {
                    evidence
                        .push("turned off in System Settings > General > Login Items".to_string());
                }
                match agent.gui_session {
                    Some(false) => evidence.push(format!(
                        "{} has no GUI login session; the Runtime starts at the next login",
                        agent.binding.user
                    )),
                    None => evidence.push(format!(
                        "uid {} cannot inspect {}'s launchd domain",
                        current_euid(),
                        agent.binding.user
                    )),
                    Some(true) => {}
                }
                launch_agent = Some(agent);
            } else if node_daemon.is_file() {
                pending_user_binding = true;
                confidence = Confidence::Medium;
                evidence.push(format!(
                    "BuckyOS Runtime installed at {} but not bound to an OS user yet",
                    macos_runtime_bundle_path(&buckyos_root).display()
                ));
            }
        }
        NodePlatform::Linux => {
            let unit = PathBuf::from("/etc/systemd/system/buckyos.service");
            if unit.is_file() {
                evidence.push(format!("found systemd unit: {}", unit.display()));
                service_plist_path = Some(unit.clone());
                service_unit_name = Some("buckyos.service".to_string());
                model = NodeHostControlModel::ServiceManager;
                confidence = Confidence::High;
                if let Some(out) = run_capture("systemctl", &["is-enabled", "buckyos.service"]) {
                    let trimmed = out.trim();
                    service_enabled = Some(trimmed == "enabled" || trimmed == "static");
                    evidence.push(format!("systemctl is-enabled: {}", trimmed));
                }
                if let Some(out) = run_capture("systemctl", &["is-active", "buckyos.service"]) {
                    let trimmed = out.trim();
                    service_active = Some(trimmed == "active");
                    evidence.push(format!("systemctl is-active: {}", trimmed));
                }
            } else if node_daemon.is_file() {
                model = NodeHostControlModel::DirectProcess;
                confidence = Confidence::Medium;
                evidence.push(format!(
                    "no systemd unit; binary present at {}",
                    node_daemon.display()
                ));
            }
        }
        NodePlatform::Windows => {
            let task = "BuckyOSNodeDaemonKeepAlive".to_string();
            let task_present =
                run_capture("schtasks", &["/Query", "/TN", &task, "/FO", "LIST"]).is_some();
            let run_key = "BuckyOSDaemon".to_string();
            run_key_name = Some(run_key.clone());
            if task_present {
                model = NodeHostControlModel::ScheduledLauncher;
                confidence = Confidence::High;
                scheduled_task_name = Some(task.clone());
                evidence.push(format!("scheduled task present: {}", task));
            } else if node_daemon.is_file() {
                model = NodeHostControlModel::DirectProcess;
                confidence = Confidence::Medium;
                evidence.push(format!(
                    "no scheduled task; binary present at {}",
                    node_daemon.display()
                ));
            }
        }
        NodePlatform::Unknown => {
            evidence.push("unknown platform".to_string());
        }
    }

    NodeHostControlState {
        model,
        platform,
        buckyos_root,
        node_daemon_binary: node_daemon,
        service_unit_name,
        service_plist_path,
        scheduled_task_name,
        run_key_name,
        confidence,
        evidence,
        service_enabled,
        service_active,
        launch_agent,
        pending_user_binding,
        legacy_service,
    }
}

// -----------------------------------------------------------------------------
// Check 实现
// -----------------------------------------------------------------------------

pub fn node_check(
    buckyos_root_override: Option<PathBuf>,
    options: NodeCheckOptions,
) -> NodeCheckReport {
    let host_control = match buckyos_root_override {
        Some(root) => detect_host_control_state_at(root),
        None => detect_host_control_state(),
    };
    let buckyos_root = host_control.buckyos_root.clone();
    let etc_dir = buckyos_root.join("etc");
    let bin_dir = buckyos_root.join("bin");
    let log_root = buckyos_root.join("logs");
    let node_identity = etc_dir.join("node_identity.json");

    let mut checks: Vec<CheckItem> = Vec::new();
    let mut log_findings: Vec<DiagnosticFinding> = Vec::new();
    let processes = collect_processes();

    let activated = node_identity.is_file();
    if activated {
        checks.push(CheckItem::new(
            "Activation State",
            CheckStatus::Ok,
            format!("Activated: {}", node_identity.display()),
        ));
    } else {
        checks.push(
            CheckItem::new(
                "Activation State",
                CheckStatus::Warn,
                format!("Not found: {}", node_identity.display()),
            )
            .with_detail("Treating system as activation-pending."),
        );
    }

    // node_daemon 进程
    let node_daemon_procs = instance_node_daemons(&host_control, &processes);
    if !node_daemon_procs.is_empty() {
        let pids: Vec<String> = node_daemon_procs
            .iter()
            .take(5)
            .map(|p| p.pid.to_string())
            .collect();
        checks.push(
            CheckItem::new(
                "node_daemon Process",
                CheckStatus::Ok,
                format!("Found {} process(es)", node_daemon_procs.len()),
            )
            .with_detail(format!("PID: {}", pids.join(", "))),
        );
    } else {
        checks.push(CheckItem::new(
            "node_daemon Process",
            CheckStatus::Fail,
            "No node_daemon/node-daemon process found",
        ));
    }

    let mut live_probe = NodeLiveProbeReport::default();
    let mut activation_ready = false;

    if !activated {
        let port_open = probe_tcp(PORT_NODE_DAEMON_ACTIVATION);
        let (http_ok, status) = probe_http(PORT_NODE_DAEMON_ACTIVATION, "/");
        live_probe.activation_endpoint_ok = port_open && http_ok;
        live_probe.detail.push(format!(
            "activation: tcp_open={}, http_ok={}, status={:?}",
            port_open, http_ok, status
        ));
        if port_open {
            checks.push(CheckItem::new(
                "3182 Activation Port",
                CheckStatus::Ok,
                "3182 is reachable",
            ));
        } else {
            checks.push(CheckItem::new(
                "3182 Activation Port",
                CheckStatus::Fail,
                "3182 is not reachable",
            ));
        }
        activation_ready = !node_daemon_procs.is_empty() && port_open && http_ok;
    } else {
        // 核心进程
        for (label, aliases, severity_when_missing) in [
            (
                "cyfs_gateway Process",
                CYFS_GATEWAY_ALIASES,
                CheckStatus::Fail,
            ),
            (
                "system_config Process",
                SYSTEM_CONFIG_ALIASES,
                CheckStatus::Fail,
            ),
            ("scheduler Process", SCHEDULER_ALIASES, CheckStatus::Fail),
            ("verify_hub Process", VERIFY_HUB_ALIASES, CheckStatus::Fail),
            (
                "control_panel Process",
                CONTROL_PANEL_ALIASES,
                CheckStatus::Warn,
            ),
        ] {
            let found = find_processes(&processes, aliases);
            if !found.is_empty() {
                let pids: Vec<String> = found.iter().take(5).map(|p| p.pid.to_string()).collect();
                checks.push(
                    CheckItem::new(
                        label,
                        CheckStatus::Ok,
                        format!("Found {} process(es)", found.len()),
                    )
                    .with_detail(format!("PID: {}", pids.join(", "))),
                );
            } else {
                checks.push(CheckItem::new(
                    label,
                    severity_when_missing,
                    format!("No process matching {:?} found", aliases),
                ));
            }
        }

        // 端口
        let ports: &[(&str, u16, CheckStatus)] = &[
            (
                "zone_gateway_http",
                PORT_ZONE_GATEWAY_HTTP,
                CheckStatus::Fail,
            ),
            (
                "node_gateway_http",
                PORT_NODE_GATEWAY_HTTP,
                CheckStatus::Fail,
            ),
            ("system_config", PORT_SYSTEM_CONFIG, CheckStatus::Fail),
            ("verify_hub", PORT_VERIFY_HUB, CheckStatus::Fail),
            ("control_panel", PORT_CONTROL_PANEL, CheckStatus::Warn),
        ];
        let mut port_results: BTreeMap<u16, bool> = BTreeMap::new();
        for (label, port, severity) in ports {
            let open = probe_tcp(*port);
            port_results.insert(*port, open);
            let listener = get_port_listener(*port);
            let mut item = CheckItem::new(
                format!("Port {}", port),
                if open { CheckStatus::Ok } else { *severity },
                if open {
                    format!("{} is reachable", label)
                } else {
                    format!("{} is not reachable", label)
                },
            );
            if let Some(line) = listener {
                item = item.with_detail(format!("Listener: {}", line));
            }
            checks.push(item);
        }

        live_probe.node_gateway_ok = *port_results.get(&PORT_NODE_GATEWAY_HTTP).unwrap_or(&false);
        live_probe.system_config_ok = *port_results.get(&PORT_SYSTEM_CONFIG).unwrap_or(&false);
        live_probe.node_daemon_control_ok = !node_daemon_procs.is_empty();

        // cyfs_gateway 二进制 fallback 提示
        let cyfs_gw_running = !find_processes(&processes, CYFS_GATEWAY_ALIASES).is_empty();
        let port80 = *port_results.get(&PORT_ZONE_GATEWAY_HTTP).unwrap_or(&false);
        let port_node_gw = *port_results.get(&PORT_NODE_GATEWAY_HTTP).unwrap_or(&false);
        if !cyfs_gw_running || !port80 || !port_node_gw {
            let suffix = if cfg!(target_os = "windows") {
                ".exe"
            } else {
                ""
            };
            let candidates = [
                bin_dir
                    .join("cyfs-gateway")
                    .join(format!("cyfs_gateway{}", suffix)),
                bin_dir
                    .join("cyfs_gateway")
                    .join(format!("cyfs_gateway{}", suffix)),
                bin_dir
                    .join("cyfs-gateway")
                    .join(format!("cyfs-gateway{}", suffix)),
            ];
            let exists = candidates.iter().find(|p| p.exists());
            if let Some(p) = exists {
                checks.push(CheckItem::new(
                    "cyfs_gateway Binary",
                    CheckStatus::Ok,
                    format!("cyfs_gateway executable exists: {}", p.display()),
                ));
            } else {
                let mut item = CheckItem::new(
                    "cyfs_gateway Binary",
                    CheckStatus::Fail,
                    "cyfs_gateway executable was not found",
                );
                for c in &candidates {
                    item = item.with_detail(format!("checked: {}", c.display()));
                }
                checks.push(item);
            }
        }
    }

    if options.include_logs && log_root.is_dir() {
        log_findings.extend(scan_log_findings(&log_root));
    }

    // host control 报告
    if options.include_host_control {
        let host_status = match host_control.model {
            NodeHostControlModel::ServiceManager => {
                let active = host_control.service_active.unwrap_or(false);
                if active {
                    (
                        CheckStatus::Ok,
                        "Service manager unit is active".to_string(),
                    )
                } else if host_control.platform != NodePlatform::Unknown {
                    (
                        CheckStatus::Warn,
                        "Service manager unit not active".to_string(),
                    )
                } else {
                    (
                        CheckStatus::Info,
                        "Service manager state unknown".to_string(),
                    )
                }
            }
            NodeHostControlModel::ScheduledLauncher => (
                CheckStatus::Ok,
                format!(
                    "Scheduled launcher present: {}",
                    host_control.scheduled_task_name.clone().unwrap_or_default()
                ),
            ),
            NodeHostControlModel::DirectProcess => (
                CheckStatus::Info,
                "DirectProcess: no host service registration".to_string(),
            ),
            NodeHostControlModel::ContainerizedRuntime => (
                CheckStatus::Info,
                "Containerized runtime detected".to_string(),
            ),
            NodeHostControlModel::Unknown => {
                (CheckStatus::Warn, "Host control model unknown".to_string())
            }
        };
        let host_status = macos_host_status(&host_control).unwrap_or(host_status);
        let mut item = CheckItem::new("Host Control", host_status.0, host_status.1);
        for ev in &host_control.evidence {
            item = item.with_detail(ev.clone());
        }
        checks.push(item);
        if host_control.launch_agent.is_some() {
            checks.push(desktop_session_check(&buckyos_root));
        }
    }

    let availability_layer = compute_availability_layer(activated, &live_probe, &node_daemon_procs);
    let overall = compute_overall_level(&checks, &host_control, activated, activation_ready);
    let (title, summary) = summarize(activated, activation_ready, &overall, &availability_layer);

    NodeCheckReport {
        overall,
        availability_layer,
        buckyos_root,
        platform: host_control.platform,
        host_control,
        activated,
        activation_ready,
        live_probe,
        checks,
        log_findings,
        title,
        summary,
    }
}

fn macos_host_status(host: &NodeHostControlState) -> Option<(CheckStatus, String)> {
    if host.platform != NodePlatform::MacOS {
        return None;
    }
    if host.legacy_service.is_some() {
        return Some((
            CheckStatus::Fail,
            "A pre-LaunchAgent system service competes with this Runtime; reinstall or uninstall to remove it"
                .to_string(),
        ));
    }
    if host.pending_user_binding {
        return Some((
            CheckStatus::Warn,
            "BuckyOS Runtime is installed but not bound to an OS user; open BuckyOS Runtime to finish setup"
                .to_string(),
        ));
    }
    let agent = host.launch_agent.as_ref()?;
    let autostart = if agent.autostart {
        "login autostart on"
    } else {
        "login autostart off"
    };
    Some(if agent.disabled {
        (
            CheckStatus::Warn,
            "BuckyOS Runtime is turned off in System Settings > General > Login Items".to_string(),
        )
    } else if agent.gui_session == Some(false) {
        (
            CheckStatus::Info,
            format!(
                "Waiting for {} to log in ({})",
                agent.binding.user, autostart
            ),
        )
    } else if agent.pid.is_some() {
        (
            CheckStatus::Ok,
            format!(
                "Running in {} as pid {:?} ({})",
                agent.domain,
                agent.pid.unwrap_or_default(),
                autostart
            ),
        )
    } else if agent.loaded == Some(true) {
        (
            CheckStatus::Warn,
            format!("Loaded in {} but not running ({})", agent.domain, autostart),
        )
    } else {
        (CheckStatus::Warn, format!("Not running ({})", autostart))
    })
}

/// GUI session and privacy permissions as the Runtime itself observed them.
fn desktop_session_check(buckyos_root: &Path) -> CheckItem {
    let Some((status, age)) = read_desktop_status(buckyos_root) else {
        return CheckItem::new(
            "Desktop Session",
            CheckStatus::Info,
            "The Runtime has not reported desktop readiness yet",
        );
    };
    if age > DESKTOP_STATUS_STALE_SECS {
        return CheckItem::new(
            "Desktop Session",
            CheckStatus::Info,
            format!(
                "Last desktop report is {}s old (Runtime stopped or not in desktop mode)",
                age
            ),
        );
    }
    let caps = &status["capabilities"];
    let state = caps["ui"]["state"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let level = match state.as_str() {
        "ready" => CheckStatus::Ok,
        "permission_required" => CheckStatus::Warn,
        _ => CheckStatus::Info,
    };
    let mut item = CheckItem::new("Desktop Session", level, format!("GUI: {}", state));
    for name in ["capture", "input"] {
        let reason = caps[name]["reason"].as_str().unwrap_or("ready");
        item = item.with_detail(format!("{}: {}", name, reason));
    }
    if let Some(uid) = status["uid"].as_u64() {
        item = item.with_detail(format!("runtime uid: {}", uid));
    }
    item
}

fn compute_availability_layer(
    activated: bool,
    live: &NodeLiveProbeReport,
    node_daemon_procs: &[&ProcessInfo],
) -> NodeAvailabilityLayer {
    if !activated {
        if live.activation_endpoint_ok {
            return NodeAvailabilityLayer::ActivationEndpointReachable;
        }
        return NodeAvailabilityLayer::HostReachable;
    }
    if live.system_config_ok && live.node_gateway_ok {
        return NodeAvailabilityLayer::FullRuntimeReachable;
    }
    if live.system_config_ok || live.node_gateway_ok {
        return NodeAvailabilityLayer::MinimalKernelReachable;
    }
    if !node_daemon_procs.is_empty() {
        return NodeAvailabilityLayer::NodeDaemonReachable;
    }
    NodeAvailabilityLayer::ControllerDownWorkloadAlive
}

fn compute_overall_level(
    checks: &[CheckItem],
    host_control: &NodeHostControlState,
    activated: bool,
    activation_ready: bool,
) -> NodeFaultLevel {
    let has_fail = checks.iter().any(|c| c.status == CheckStatus::Fail);
    let has_warn = checks.iter().any(|c| c.status == CheckStatus::Warn);

    if !activated {
        return if activation_ready {
            NodeFaultLevel::Green
        } else if has_fail {
            NodeFaultLevel::Yellow
        } else {
            NodeFaultLevel::Yellow
        };
    }

    if host_control.model == NodeHostControlModel::ServiceManager {
        if matches!(host_control.service_active, Some(false)) {
            return NodeFaultLevel::Yellow;
        }
    }

    if has_fail {
        return NodeFaultLevel::Orange;
    }
    if has_warn {
        return NodeFaultLevel::Blue;
    }
    NodeFaultLevel::Green
}

fn summarize(
    activated: bool,
    activation_ready: bool,
    overall: &NodeFaultLevel,
    layer: &NodeAvailabilityLayer,
) -> (String, String) {
    if !activated {
        if activation_ready {
            return (
                "Activation Ready".into(),
                "node_active is serving on this machine and the system is waiting for activation"
                    .into(),
            );
        }
        return (
            "Not Running".into(),
            "system is not activated and node_active is not serving".into(),
        );
    }
    let title = match overall {
        NodeFaultLevel::Green => "Running",
        NodeFaultLevel::Blue => "Running With Warnings",
        NodeFaultLevel::Yellow => "Booting Or Degraded",
        NodeFaultLevel::Orange => "Abnormal",
        NodeFaultLevel::Red => "Critical",
    };
    let summary = format!("availability layer: {:?}", layer);
    (title.to_string(), summary)
}

// 简化的日志扫描（保留 check.py 的 churn / permission 信号）。
fn scan_log_findings(log_root: &Path) -> Vec<DiagnosticFinding> {
    let mut findings = Vec::new();
    for service in ["scheduler", "node_daemon", "node-daemon"] {
        let dir = log_root.join(service);
        if !dir.is_dir() {
            continue;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let mut log_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(OsStr::to_str) == Some("log"))
            .collect();
        log_files.sort_by_key(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
        });
        log_files.reverse();
        let mut pids: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        for f in log_files.iter().take(10) {
            if let Some(name) = f.file_name().and_then(OsStr::to_str) {
                if let Some(captured) = extract_pid_from_log_name(name) {
                    pids.insert(captured);
                }
            }
        }
        if pids.len() > 2 {
            findings.push(DiagnosticFinding {
                severity: CheckStatus::Warn,
                title: format!("{} may be restarting repeatedly", service),
                detail: format!(
                    "Recent log files under {} map to {} different PIDs.",
                    dir.display(),
                    pids.len()
                ),
            });
        }
    }
    findings
}

fn extract_pid_from_log_name(name: &str) -> Option<String> {
    if !name.ends_with(".log") {
        return None;
    }
    let stem = &name[..name.len() - 4];
    let last_sep = stem.rfind(|c: char| c == '_' || c == '-')?;
    let candidate = &stem[last_sep + 1..];
    if !candidate.is_empty() && candidate.chars().all(|c| c.is_ascii_digit()) {
        Some(candidate.to_string())
    } else {
        None
    }
}

// -----------------------------------------------------------------------------
// CLI 渲染
// -----------------------------------------------------------------------------

pub fn print_check_report(report: &NodeCheckReport) {
    println!("BuckyOS Local Runtime Check");
    println!("- Platform: {:?}", report.platform);
    println!("- BUCKYOS_ROOT: {}", report.buckyos_root.display());
    println!(
        "- Host Control: model={:?} confidence={:?} unit={:?} task={:?}",
        report.host_control.model,
        report.host_control.confidence,
        report.host_control.service_unit_name,
        report.host_control.scheduled_task_name
    );
    if let Some(agent) = &report.host_control.launch_agent {
        println!(
            "- Desktop Runtime: user={} uid={} {}/{} autostart={} loaded={:?} pid={:?} disabled={}",
            agent.binding.user,
            agent.binding.uid,
            agent.domain,
            agent.label,
            agent.autostart,
            agent.loaded,
            agent.pid,
            agent.disabled
        );
        println!(
            "- Runtime: {}",
            report.host_control.node_daemon_binary.display()
        );
    } else if report.host_control.pending_user_binding {
        println!("- Desktop Runtime: installed, not bound to an OS user");
    }
    println!(
        "- Activated: {}, Activation Ready: {}",
        report.activated, report.activation_ready
    );
    println!(
        "- Overall: {:?} | Availability: {:?}",
        report.overall, report.availability_layer
    );
    println!("- Status: {} ({})", report.title, report.summary);

    println!("\nChecks");
    for item in &report.checks {
        let prefix = match item.status {
            CheckStatus::Ok => "[OK]",
            CheckStatus::Warn => "[WARN]",
            CheckStatus::Fail => "[FAIL]",
            CheckStatus::Info => "[INFO]",
        };
        println!("{} {}: {}", prefix, item.name, item.summary);
        for d in &item.details {
            println!("  - {}", d);
        }
    }

    if !report.log_findings.is_empty() {
        println!("\nDiagnostics");
        for f in &report.log_findings {
            let prefix = match f.severity {
                CheckStatus::Ok => "[OK]",
                CheckStatus::Warn => "[WARN]",
                CheckStatus::Fail => "[FAIL]",
                CheckStatus::Info => "[INFO]",
            };
            println!("{} {}: {}", prefix, f.title, f.detail);
        }
    }
}

// -----------------------------------------------------------------------------
// Start
// -----------------------------------------------------------------------------

pub fn node_start(req: NodeStartRequest) -> Result<NodeStartReport, String> {
    let mut host = match req.buckyos_root.clone() {
        Some(root) => detect_host_control_state_at(root),
        None => detect_host_control_state(),
    };

    if host.platform == NodePlatform::MacOS
        && req.host_control_policy != NodeHostControlPolicy::DirectProcessOnly
    {
        if host.legacy_service.is_some() {
            return Err(format!(
                "{} still registers the old system service; reinstall BuckyOS or remove it first",
                MACOS_LEGACY_DAEMON_PLIST
            ));
        }
        if host.launch_agent.is_none() {
            return Err(
                "BuckyOS Runtime is not bound to an OS user; open BuckyOS Runtime (or run `node_daemon service register`) to finish setup"
                    .into(),
            );
        }
    }

    let chosen_model = pick_start_model(&req.host_control_policy, host.model)?;
    host.model = chosen_model;

    if req.stop_conflicting && req.mode != NodeStartMode::Recovery {
        // 软停 node_daemon 旧进程，避免端口冲突；不杀其他 run item。macOS 只停本实例里
        // 不归 launchd 管的 controller。
        let managed = host.launch_agent.as_ref().and_then(|agent| agent.pid);
        let processes = collect_processes();
        for proc in instance_node_daemons(&host, &processes) {
            if Some(proc.pid) != managed {
                let _ = kill_process_by_pid(proc.pid);
            }
        }
    }

    let mut actions = Vec::new();
    let mut started_pid: Option<u32> = None;

    match chosen_model {
        NodeHostControlModel::ServiceManager => match host.platform {
            NodePlatform::MacOS => start_via_launch_agent(&host, &mut actions)?,
            NodePlatform::Linux => start_via_systemd(&host, &mut actions)?,
            _ => {
                started_pid = Some(spawn_node_daemon_direct(&host, req.mode, &mut actions)?);
            }
        },
        NodeHostControlModel::ScheduledLauncher => {
            start_via_scheduled_task(&host, &mut actions)?;
        }
        NodeHostControlModel::DirectProcess => {
            started_pid = Some(spawn_node_daemon_direct(&host, req.mode, &mut actions)?);
        }
        NodeHostControlModel::ContainerizedRuntime => {
            return Err("ContainerizedRuntime start is not implemented".into());
        }
        NodeHostControlModel::Unknown => {
            return Err("Unknown host control model; refusing start".into());
        }
    }

    Ok(NodeStartReport {
        used_model: chosen_model,
        actions,
        started_pid,
    })
}

fn pick_start_model(
    policy: &NodeHostControlPolicy,
    detected: NodeHostControlModel,
) -> Result<NodeHostControlModel, String> {
    match policy {
        NodeHostControlPolicy::Auto => match detected {
            NodeHostControlModel::Unknown => Ok(NodeHostControlModel::DirectProcess),
            other => Ok(other),
        },
        NodeHostControlPolicy::PreferServiceManager => Ok(NodeHostControlModel::ServiceManager),
        NodeHostControlPolicy::PreferScheduledLauncher => {
            Ok(NodeHostControlModel::ScheduledLauncher)
        }
        NodeHostControlPolicy::DirectProcessOnly => Ok(NodeHostControlModel::DirectProcess),
        NodeHostControlPolicy::RefuseIfUnknown => {
            if detected == NodeHostControlModel::Unknown {
                Err("Host control model unknown; refusing".into())
            } else {
                Ok(detected)
            }
        }
    }
}

fn spawn_node_daemon_direct(
    host: &NodeHostControlState,
    mode: NodeStartMode,
    actions: &mut Vec<String>,
) -> Result<u32, String> {
    if !host.node_daemon_binary.is_file() {
        return Err(format!(
            "node_daemon binary not found: {}",
            host.node_daemon_binary.display()
        ));
    }

    let mut cmd = Command::new(&host.node_daemon_binary);
    if matches!(
        mode,
        NodeStartMode::Activation | NodeStartMode::Normal | NodeStartMode::DesktopDaemon
    ) {
        cmd.arg("--enable_active");
    }
    if mode == NodeStartMode::DesktopDaemon {
        cmd.arg("--desktop_daemon");
    }
    cmd.env("BUCKYOS_ROOT", &host.buckyos_root);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_background_child_console(&mut cmd);

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                libc::setsid();
                Ok(())
            });
        }
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("spawn node_daemon failed: {}", e))?;
    let pid = child.id();
    actions.push(format!(
        "spawned node_daemon (pid={}) from {}",
        pid,
        host.node_daemon_binary.display()
    ));
    Ok(pid)
}

fn start_via_launch_agent(
    host: &NodeHostControlState,
    actions: &mut Vec<String>,
) -> Result<(), String> {
    let agent = host
        .launch_agent
        .as_ref()
        .ok_or_else(|| "BuckyOS Runtime is not bound to an OS user".to_string())?;
    ensure_agent_controllable(agent)?;
    if agent.disabled {
        return Err(
            "BuckyOS Runtime is turned off in System Settings > General > Login Items; turn it on there to start it"
                .into(),
        );
    }
    if agent.gui_session == Some(false) {
        return Err(format!(
            "{} has no GUI login session; the Runtime starts when that user logs in",
            agent.binding.user
        ));
    }
    write_macos_launch_agent(
        &host.buckyos_root,
        &agent.binding,
        &agent.plist_path,
        actions,
    )?;
    if agent.loaded != Some(true) {
        let plist = agent.plist_path.to_string_lossy();
        run_must("launchctl", &["bootstrap", &agent.domain, &plist], actions)?;
    }
    run_must(
        "launchctl",
        &["kickstart", &format!("{}/{}", agent.domain, agent.label)],
        actions,
    )
}

fn start_via_systemd(host: &NodeHostControlState, actions: &mut Vec<String>) -> Result<(), String> {
    let unit = host
        .service_unit_name
        .clone()
        .unwrap_or_else(|| "buckyos.service".to_string());
    run_quiet("systemctl", &["daemon-reload"], actions);
    run_quiet("systemctl", &["enable", &unit], actions);
    run_must("systemctl", &["start", &unit], actions)?;
    Ok(())
}

fn start_via_scheduled_task(
    host: &NodeHostControlState,
    actions: &mut Vec<String>,
) -> Result<(), String> {
    let task = host
        .scheduled_task_name
        .clone()
        .unwrap_or_else(|| "BuckyOSNodeDaemonKeepAlive".to_string());
    run_must("schtasks", &["/Run", "/TN", &task], actions)?;
    Ok(())
}

fn run_quiet(program: &str, args: &[&str], actions: &mut Vec<String>) {
    let label = format!("{} {}", program, args.join(" "));
    let mut command = Command::new(program);
    command.args(args);
    match hide_child_console(&mut command).output() {
        Ok(out) if out.status.success() => actions.push(format!("ok: {}", label)),
        Ok(out) => actions.push(format!(
            "warn: {} exit={:?} stderr={}",
            label,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        )),
        Err(e) => actions.push(format!("error: {} ({})", label, e)),
    }
}

fn run_must(program: &str, args: &[&str], actions: &mut Vec<String>) -> Result<(), String> {
    let label = format!("{} {}", program, args.join(" "));
    let mut command = Command::new(program);
    command.args(args);
    let out = hide_child_console(&mut command)
        .output()
        .map_err(|e| format!("execute {} failed: {}", label, e))?;
    if !out.status.success() {
        return Err(format!(
            "{} exit={:?} stderr={}",
            label,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    actions.push(format!("ok: {}", label));
    Ok(())
}

// -----------------------------------------------------------------------------
// Stop
// -----------------------------------------------------------------------------

pub fn node_stop(req: NodeStopRequest) -> Result<NodeStopReport, String> {
    let mut host = match req.buckyos_root.clone() {
        Some(root) => detect_host_control_state_at(root),
        None => detect_host_control_state(),
    };
    if let Some(agent) = &host.launch_agent {
        ensure_agent_controllable(agent)?;
    }

    if req.host_control_policy == NodeHostControlPolicy::RefuseIfUnknown
        && host.model == NodeHostControlModel::Unknown
    {
        return Err("host control unknown; refusing stop".into());
    }
    if req.host_control_policy == NodeHostControlPolicy::DirectProcessOnly {
        host.model = NodeHostControlModel::DirectProcess;
    }

    let mut report = NodeStopReport::default();

    // Stage 1: 关闭自动拉起。
    let stop_host = matches!(
        req.mode,
        NodeStopMode::Graceful
            | NodeStopMode::GracefulThenForce
            | NodeStopMode::StopHostService
            | NodeStopMode::KillAll
    ) && matches!(
        req.blackbox_policy,
        NodeBlackboxStopPolicy::IncludeHostService | NodeBlackboxStopPolicy::FullRuntime
    ) || matches!(
        req.mode,
        NodeStopMode::StopHostService | NodeStopMode::KillAll | NodeStopMode::GracefulThenForce
    );

    if stop_host {
        stop_host_service(&host, &mut report);
    }

    if matches!(req.mode, NodeStopMode::StopHostService) {
        return Ok(report);
    }

    // Stage 2: graceful — 此版本只是“已关闭自动拉起后等待 timeout”。
    // node_daemon 不需要进一步主动停 run item，由 Host OS 服务管理器/上层负责或直接到黑盒。
    if matches!(
        req.mode,
        NodeStopMode::Graceful | NodeStopMode::GracefulThenForce
    ) {
        let waited = wait_for_node_daemon_exit(&host, req.timeout_secs);
        report
            .actions
            .push(format!("waited {}s for node_daemon graceful exit", waited));
    }

    // Stage 3: 黑盒强停。
    let blackbox_enabled = matches!(
        req.mode,
        NodeStopMode::GracefulThenForce | NodeStopMode::BlackboxForce | NodeStopMode::KillAll
    ) && req.blackbox_policy != NodeBlackboxStopPolicy::Disabled;

    if blackbox_enabled {
        blackbox_stop(&req, &host, &mut report);
    }

    // Stage 4: 残留检查。
    let processes = collect_processes();
    report.remaining = runtime_processes(&host, &processes, &[])
        .into_iter()
        .map(|proc| format!("pid={} {}", proc.pid, proc.command))
        .collect();

    Ok(report)
}

fn stop_host_service(host: &NodeHostControlState, report: &mut NodeStopReport) {
    match (host.platform, host.model) {
        (NodePlatform::MacOS, _) => {
            // bootout only unloads the job for this session: the login item and its
            // autostart preference stay as they are.
            if let Some(agent) = &host.launch_agent {
                if agent.loaded != Some(false) {
                    run_quiet(
                        "launchctl",
                        &["bootout", &format!("{}/{}", agent.domain, agent.label)],
                        &mut report.host_service_actions,
                    );
                }
            }
            if host.legacy_service.is_some() {
                run_quiet(
                    "launchctl",
                    &["bootout", &format!("system/{}", MACOS_LEGACY_LABEL)],
                    &mut report.host_service_actions,
                );
            }
        }
        (NodePlatform::Linux, NodeHostControlModel::ServiceManager) => {
            let unit = host
                .service_unit_name
                .clone()
                .unwrap_or_else(|| "buckyos.service".to_string());
            run_quiet(
                "systemctl",
                &["stop", &unit],
                &mut report.host_service_actions,
            );
        }
        (NodePlatform::Windows, NodeHostControlModel::ScheduledLauncher) => {
            if let Some(task) = &host.scheduled_task_name {
                run_quiet(
                    "schtasks",
                    &["/Delete", "/TN", task, "/F"],
                    &mut report.host_service_actions,
                );
            }
            // Run key
            if let Some(run_key) = &host.run_key_name {
                run_quiet(
                    "reg",
                    &[
                        "delete",
                        "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run",
                        "/V",
                        run_key,
                        "/F",
                    ],
                    &mut report.host_service_actions,
                );
            }
        }
        _ => {
            report
                .host_service_actions
                .push("no host service action for this model".to_string());
        }
    }
}

fn wait_for_node_daemon_exit(host: &NodeHostControlState, timeout_secs: u64) -> u64 {
    let mut waited = 0;
    let step = Duration::from_secs(1);
    while waited < timeout_secs {
        let processes = collect_processes();
        if instance_node_daemons(host, &processes).is_empty() {
            return waited;
        }
        std::thread::sleep(step);
        waited += 1;
    }
    waited
}

/// Processes a blackbox stop may act on: on macOS everything of this
/// installation (bound user, executable under the root); elsewhere the known
/// BuckyOS process names plus `extra` from run.plist.
fn runtime_processes<'a>(
    host: &NodeHostControlState,
    processes: &'a [ProcessInfo],
    extra: &[String],
) -> Vec<&'a ProcessInfo> {
    if let Some(scope) = instance_scope(host) {
        return processes.iter().filter(|p| scope.contains(p)).collect();
    }
    let mut alias_set: std::collections::BTreeSet<String> = PROCESS_KILL_BASELINE
        .iter()
        .map(|s| normalize_name(s))
        .collect();
    for alias in extra {
        alias_set.insert(normalize_name(alias));
    }
    let aliases: Vec<String> = alias_set.into_iter().collect();
    find_processes(processes, &aliases)
}

fn blackbox_stop(req: &NodeStopRequest, host: &NodeHostControlState, report: &mut NodeStopReport) {
    let snapshot = read_run_plist(&host.buckyos_root);
    if let Some(snap) = &snapshot {
        report.actions.push(format!(
            "loaded run.plist from {}: {} items",
            snap.path.display(),
            snap.items.len()
        ));
    } else {
        report
            .actions
            .push("run.plist unavailable; using baseline kill list".to_string());
    }

    // 进程清单：macOS 为本实例进程；其他平台为保底 + run.plist 中 kernel/frame service 名。
    let extra = snapshot
        .as_ref()
        .map(|snap| snap.all_process_aliases())
        .unwrap_or_default();
    let processes = collect_processes();
    for proc in runtime_processes(host, &processes, &extra) {
        match kill_process_by_pid(proc.pid) {
            Ok(_) => report
                .stopped_processes
                .push(format!("pid={} {}", proc.pid, proc.command)),
            Err(e) => report
                .actions
                .push(format!("kill pid={} failed: {}", proc.pid, e)),
        }
    }

    // 容器：根据策略和平台决定是否处理。
    let want_containers = matches!(
        req.blackbox_policy,
        NodeBlackboxStopPolicy::IncludeManagedContainers | NodeBlackboxStopPolicy::FullRuntime
    );
    if want_containers {
        let mut targets: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        if let Some(snap) = &snapshot {
            for name in snap.app_service_container_names() {
                targets.insert(name);
            }
        }
        // 兼容历史脚本 devtest-* 容器（如 run.plist 不可用时的开发场景）。
        if targets.is_empty() && !cfg!(target_os = "windows") {
            if let Some(out) = run_capture("docker", &["ps", "-a", "--format", "{{.Names}}"]) {
                for line in out.lines() {
                    let name = line.trim();
                    if name.starts_with("devtest-") {
                        targets.insert(name.to_string());
                    }
                }
            }
        }
        // 也支持 buckyos label
        if let Some(out) = run_capture(
            "docker",
            &["ps", "-aq", "--filter", "label=buckyos.app_instance_id"],
        ) {
            for line in out.lines() {
                let id = line.trim();
                if !id.is_empty() {
                    targets.insert(id.to_string());
                }
            }
        }

        for target in targets {
            let mut command = Command::new("docker");
            command.args(["rm", "-f", &target]);
            let r = hide_child_console(&mut command).output();
            match r {
                Ok(out) if out.status.success() => {
                    report.stopped_containers.push(target);
                }
                Ok(out) => report.actions.push(format!(
                    "docker rm -f {} failed: {}",
                    target,
                    String::from_utf8_lossy(&out.stderr).trim()
                )),
                Err(e) => report
                    .actions
                    .push(format!("docker rm -f {} error: {}", target, e)),
            }
        }
    }

    // legacy: KillAll 模式下额外 fallback 用 killall/taskkill 强一遍（macOS 不按全机进程名杀）。
    if matches!(req.mode, NodeStopMode::KillAll) && instance_scope(host).is_none() {
        for alias in PROCESS_KILL_BASELINE {
            let _ = kill_process_by_name(alias);
        }
        let _ = host; // suppress unused on some configs
    }
}

fn kill_process_by_pid(pid: u32) -> Result<(), String> {
    if cfg!(target_os = "windows") {
        let mut command = Command::new("taskkill");
        command.args(["/F", "/PID", &pid.to_string()]);
        let out = hide_child_console(&mut command)
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).to_string());
        }
    } else {
        #[cfg(unix)]
        unsafe {
            if libc::kill(pid as libc::pid_t, libc::SIGTERM) != 0 {
                let err = std::io::Error::last_os_error();
                return Err(err.to_string());
            }
        }
    }
    Ok(())
}

pub fn kill_process_by_name(name: &str) -> Result<bool, String> {
    if cfg!(target_os = "windows") {
        let exe_name = format!("{}.exe", name);
        let mut command = Command::new("taskkill");
        command.args(["/F", "/IM", &exe_name]);
        let out = hide_child_console(&mut command)
            .output()
            .map_err(|e| e.to_string())?;
        Ok(out.status.success())
    } else {
        let mut command = Command::new("killall");
        command.arg(name);
        let out = hide_child_console(&mut command)
            .output()
            .map_err(|e| e.to_string())?;
        Ok(out.status.success())
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_plist_path_is_per_root_on_macos() {
        let root = Path::new("/opt/buckyos");
        let path = run_plist_path(root);
        if cfg!(target_os = "windows") {
            assert!(path.ends_with("run.plist"));
        } else if cfg!(target_os = "macos") {
            assert_eq!(
                path,
                PathBuf::from("/opt/buckyos/local/node_daemon/run.plist")
            );
        } else {
            assert_eq!(path, PathBuf::from("/tmp/buckyos/run.plist"));
        }
    }

    fn binding(home: &str) -> DesktopRuntimeBinding {
        DesktopRuntimeBinding {
            user: "alice".into(),
            uid: 501,
            home: PathBuf::from(home),
        }
    }

    #[test]
    fn launch_agent_plist_escapes_paths_and_round_trips_the_root() {
        let root = Path::new("/Volumes/My Disk/Bucky & <OS>");
        let text = render_macos_launch_agent(root, &binding("/Users/a'b"));
        assert!(text.contains("<string>com.buckyos.runtime</string>"));
        assert!(text.contains("<string>Aqua</string>"));
        assert!(text.contains("<string>--enable_active</string>"));
        assert!(text.contains("<string>--desktop_daemon</string>"));
        assert!(text.contains("/Volumes/My Disk/Bucky &amp; &lt;OS&gt;"));
        assert!(!text.contains("Bucky & <OS>"));
        assert!(text.contains("<string>/Users/a&apos;b</string>"));
        assert_eq!(
            plist_env_value(&text, "BUCKYOS_ROOT").as_deref(),
            Some("/Volumes/My Disk/Bucky & <OS>")
        );
        assert_eq!(
            plist_env_value(&text, "HOME").as_deref(),
            Some("/Users/a'b")
        );
        if cfg!(target_os = "macos") {
            assert!(text.contains(
                "/Volumes/My Disk/Bucky &amp; &lt;OS&gt;/bin/node-daemon/BuckyOS Runtime.app/Contents/MacOS/node_daemon"
            ));
        }
        assert!(text.contains("logs/node_daemon/launchd.err.log"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn rendered_launch_agent_is_a_valid_plist() {
        let dir = std::env::temp_dir().join(format!("nc-plist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let plist = dir.join("agent.plist");
        std::fs::write(
            &plist,
            render_macos_launch_agent(Path::new("/opt/bucky os"), &binding("/Users/alice")),
        )
        .unwrap();
        let status = Command::new("plutil")
            .arg("-lint")
            .arg(&plist)
            .status()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert!(status.success());
    }

    #[test]
    fn ps_rows_keep_spaces_in_the_last_column() {
        let raw =
            "  101   501 /opt/b/bin/node-daemon/BuckyOS Runtime.app/Contents/MacOS/node_daemon\n\
                   bad line\n\
                   7 0 /sbin/launchd\n";
        let rows = parse_ps_rows(raw, 1);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 101);
        assert_eq!(rows[0].1, vec!["501".to_string()]);
        assert_eq!(
            rows[0].2,
            "/opt/b/bin/node-daemon/BuckyOS Runtime.app/Contents/MacOS/node_daemon"
        );
        assert_eq!(parse_ps_rows("42 /bin/sh -c x\n", 0)[0].2, "/bin/sh -c x");
    }

    #[test]
    fn launchctl_output_parsing() {
        let print = "gui/501/com.buckyos.runtime = {\n\tactive count = 1\n\tstate = running\n\tpid = 4242\n}";
        assert_eq!(parse_launchctl_pid(print), Some(4242));
        assert_eq!(parse_launchctl_pid("state = not running"), None);
        let disabled =
            "disabled services = {\n\t\"com.buckyos.runtime\" => disabled\n\t\"x\" => enabled\n}";
        assert!(parse_launchctl_disabled(disabled, "com.buckyos.runtime"));
        assert!(!parse_launchctl_disabled(disabled, "x"));
        assert!(parse_launchctl_disabled(
            "\"com.buckyos.runtime\" => true",
            "com.buckyos.runtime"
        ));
        assert!(!parse_launchctl_disabled("", "com.buckyos.runtime"));
    }

    #[test]
    fn instance_scope_matches_only_this_users_root() {
        let scope = InstanceScope {
            uid: 501,
            roots: vec!["/opt/buckyos/".into()],
        };
        let process = |pid, uid, command: &str| ProcessInfo {
            pid,
            command: command.into(),
            args: command.into(),
            uid: Some(uid),
        };
        assert!(scope.contains(&process(10, 501, "/opt/buckyos/bin/scheduler/scheduler")));
        assert!(!scope.contains(&process(11, 502, "/opt/buckyos/bin/scheduler/scheduler")));
        assert!(!scope.contains(&process(12, 501, "/opt/buckyos2/bin/scheduler/scheduler")));
        assert!(!scope.contains(&process(13, 501, "/usr/local/bin/scheduler")));
        assert!(!scope.contains(&process(
            std::process::id(),
            501,
            "/opt/buckyos/bin/node-daemon/BuckyOS Runtime.app/Contents/MacOS/node_daemon"
        )));
    }

    #[test]
    fn desktop_binding_round_trips_and_drives_detection() {
        let root = std::env::temp_dir().join(format!("nc-binding-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("etc")).unwrap();
        assert!(read_desktop_binding(&root).is_none());
        let bound = DesktopRuntimeBinding {
            user: "someone-else".into(),
            uid: current_euid().wrapping_add(1),
            home: root.join("home"),
        };
        std::fs::write(
            desktop_binding_path(&root),
            serde_json::to_vec(&bound).unwrap(),
        )
        .unwrap();
        assert_eq!(read_desktop_binding(&root), Some(bound.clone()));
        let host = detect_host_control_state_at(root.clone());
        if cfg!(target_os = "macos") {
            let agent = host.launch_agent.clone().expect("bound agent");
            assert_eq!(host.model, NodeHostControlModel::ServiceManager);
            assert_eq!(agent.domain, format!("gui/{}", bound.uid));
            assert!(!agent.autostart);
            assert_eq!(
                agent.plist_path,
                root.join("local/node_daemon/com.buckyos.runtime.plist")
            );
            if current_euid() != 0 {
                assert_eq!(agent.loaded, None);
                assert!(ensure_agent_controllable(&agent).is_err());
                assert!(node_start(NodeStartRequest {
                    buckyos_root: Some(root.clone()),
                    stop_conflicting: false,
                    ..NodeStartRequest::default()
                })
                .is_err());
            }
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn normalize_handles_underscore_and_exe() {
        assert_eq!(normalize_name("Node_Daemon.exe"), "node-daemon");
        assert_eq!(normalize_name("control-panel"), "control-panel");
    }

    #[test]
    fn run_plist_parses_app_service_container_names() {
        let raw = r#"{
            "version": 1,
            "updated_at": 1,
            "items": {
                "scheduler": {"item_name":"scheduler","item_kind":"kernel_service","target_state":"Running","observed_state":null,"run_state":"started","last_error":null,"updated_at":1},
                "devtest#buckyos_jarvis": {"item_name":"devtest#buckyos_jarvis","item_kind":"app_service","target_state":"Running","observed_state":null,"run_state":"started","last_error":null,"updated_at":1}
            }
        }"#;
        let parsed: RunPlistRaw = serde_json::from_str(raw).expect("parse");
        let snapshot = RunPlistSnapshot {
            path: PathBuf::from("/tmp/test"),
            items: parsed
                .items
                .into_values()
                .map(|i| RunPlistEntry {
                    name: i.item_name,
                    kind: i.item_kind,
                    run_state: i.run_state,
                })
                .collect(),
        };
        assert_eq!(snapshot.kernel_service_aliases(), vec!["scheduler"]);
        assert_eq!(
            snapshot.app_service_container_names(),
            vec!["devtest-buckyos_jarvis"]
        );
    }

    #[test]
    fn extract_pid_from_log_name_works() {
        assert_eq!(
            extract_pid_from_log_name("scheduler_12345.log").as_deref(),
            Some("12345")
        );
        assert_eq!(
            extract_pid_from_log_name("node-daemon-998.log").as_deref(),
            Some("998")
        );
        assert!(extract_pid_from_log_name("scheduler.log").is_none());
    }

    #[test]
    fn pick_start_model_refuse_when_unknown() {
        let err = pick_start_model(
            &NodeHostControlPolicy::RefuseIfUnknown,
            NodeHostControlModel::Unknown,
        );
        assert!(err.is_err());
    }
}
