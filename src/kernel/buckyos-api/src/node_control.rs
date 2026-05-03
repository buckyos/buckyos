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
//
// node_control 不负责：
// - 多节点顺序、屏障和分布式重启计划；这些属于 system_control。
// - Zone 级恢复拓扑、数据恢复和 Agent repair 编排；这些属于 system_control。
// - 证明用户数据完整性。当前版本只能给出“未发现明显风险 / 未检查 / 风险未知”等本机信号。
//
// 典型消费者：
// - `buckycli node check/start/stop/restart`
// - BuckyOS Desktop 安装器、卸载器、升级器
// - BuckyOS Desktop / tray 的本机服务管理入口
// - recovery system / diagnostic gateway 的本机动作入口
//
// 推荐入口：
// - Host OS 侧入口：native helper / buckycli / desktop helper 直接调用 node_control 实现。
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
//   - macOS: launchd / LaunchDaemon
//   - Linux: systemd
//   - Windows: 如果未来 installer 改为 Windows Service，也归入此类
// - ScheduledLauncher
//   通过定时/计划任务周期性拉起 node_daemon。当前 Windows 安装形态按这个模型处理。
// - DirectProcess
//   开发模式或临时安装模式，直接 spawn `$BUCKYOS_ROOT/bin/node-daemon/node_daemon`。
// - ContainerizedRuntime
//   当前设备以容器编排方式运行 BuckyOS Runtime；停止时必须纳入容器选择器。
// - Unknown
//   无法确认安装形态，只允许 check 和显式 blackbox stop。
//
// 当前安装包实现映射：
// - macOS:
//   - 使用 system LaunchDaemon，不是 LaunchAgent。
//   - plist: `/Library/LaunchDaemons/buckyos.service.plist`
//   - label: `buckyos.service`
//   - start: `launchctl bootstrap system <plist>` 后 `launchctl kickstart -k system/buckyos.service`
//   - stop/uninstall: `launchctl disable system/buckyos.service` + `launchctl bootout system <plist>`
//   - node_daemon 参数：`${BUCKYOS_ROOT}/bin/node-daemon/node_daemon --enable_active`
//   - env: `BUCKYOS_ROOT`、`HOME=/var/root`、固定 PATH、`RUST_BACKTRACE=1`
//   - stdout/stderr: `/var/log/buckyos.service.out.log`、`/var/log/buckyos.service.err.log`
//   - plist 设置 `RunAtLoad=true`、`KeepAlive=true`、`AbandonProcessGroup=true`
//   - preinstall 会检查 Docker CLI 和 root/LaunchDaemon 上下文中的 `docker info`。
//   - preinstall/uninstall 还会 best-effort 清理旧版 `/Library/LaunchAgents/buckyos.service.plist`。
// - Linux/Debian:
//   - 使用 systemd service。
//   - unit: `/etc/systemd/system/buckyos.service`
//   - ExecStart: `/opt/buckyos/bin/node-daemon/node_daemon --enable_active`
//   - WorkingDirectory: `/opt/buckyos/bin`
//   - User: `root`
//   - Restart: `always`
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
//   - 安装/卸载会删除计划任务、删除 Run 启动项，并为兼容旧版执行 `sc stop/delete buckyos`。
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
// - legacy_service_name
// - install_record_path
// - confidence
// - evidence
//
// 设计原则：
// - 已服务化时，启动/停止 node_daemon 优先走 Host OS 服务接口。
// - ScheduledLauncher 模式下，手动 start 仍然有意义，因为它可以跳过下一次调度窗口等待。
// - DirectProcess 模式下，start/stop 与现有 `src/start.py`、`src/stop.py` 的开发体验保持一致。
// - node_daemon 只负责管理其可见的 BuckyOS 服务；Host OS 服务管理器通常只知道 node_daemon，
//   不一定知道 node_daemon 拉起的子进程、容器和应用态服务。
// - macOS plist 使用 `AbandonProcessGroup=true`，所以 `launchctl bootout` 不能被视为完整 Runtime
//   清理；升级/卸载仍需要黑盒进程/容器检查兜底。
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
//   核心服务不可用，或标准控制面不可用但黑盒恢复路径仍可执行。
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
//    - macOS/Linux 默认 `/opt/buckyos`。
//    - Windows 优先读取 `HKCU\Environment\BUCKYOS_ROOT`，再读 `HKCU\Software\BuckyOS\InstallDir`，
//      兼容旧版 `HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment\BUCKYOS_ROOT`
//      和 `HKLM\Software\BuckyOS\BuckyOSServiceDir`。
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
//    - macOS: plist 存在、`launchctl print system/buckyos.service`、service 日志。
//    - Linux: unit 文件存在、`systemctl is-enabled/is-active/status buckyos.service`。
//    - Windows: 计划任务存在、Run 启动项存在、loader 脚本存在、注册表 root 存在。
//
// Green 判定要求：
// - 未激活设备：node_daemon + 3182 + activation HTTP 可用，可视为“待激活正常”。
// - 已激活设备：node_daemon + cyfs_gateway + system_config 可用，关键端口可达。
// - scheduler/verify_hub/control_panel 缺失时不应输出完全 Green；按影响范围给 Blue/Yellow。
// - 安装包形态下 HostControlState 异常时，即使当前进程还在，也不应输出完全 Green，因为重启后
//   可能无法自动恢复。
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
// - macOS ServiceManager:
//   - 校验 `/Library/LaunchDaemons/buckyos.service.plist`。
//   - `launchctl enable system/buckyos.service`。
//   - `launchctl bootstrap system <plist>`；若已 bootstrap，先 `bootout` 再 bootstrap。
//   - `launchctl kickstart -k system/buckyos.service`。
//   - 等待 `${BUCKYOS_ROOT}/bin/node-daemon/node_daemon --enable_active` 进程和 3182/3180。
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
//   只停止 Host OS 管理的 node_daemon 服务单元/计划任务，不保证清理子进程和容器。
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
//    - macOS: `launchctl disable system/buckyos.service` + `launchctl bootout system <plist>`。
//    - Linux: `systemctl stop buckyos.service`。
//    - Windows: 删除 `BuckyOSNodeDaemonKeepAlive` 计划任务和 `BuckyOSDaemon` Run 启动项。
// 3. 如果 node_daemon 控制面可达，先请求 graceful stop local runtime。
// 4. 等待默认 30 秒。
// 5. 如果仍有相关进程、端口或容器残留，执行 BlackboxForce。
// 6. 再跑一次 minimal check 验证。
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
// - macOS/Linux 安装脚本只通过 service manager 停 node_daemon，不会自动清理所有
//   `AbandonProcessGroup`/子进程/容器残留；node_control 的升级/卸载 stop 不能只包装现有脚本。
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
// 10. buckycli 映射
// =============================================================================
//
// 推荐命令：
// - buckycli node check --level standard --json
// - buckycli node start
// - buckycli node ensure-running --mode activation
// - buckycli node stop --mode graceful --json
// - buckycli node stop --mode graceful-then-force --timeout 30 --json
// - buckycli node stop --mode kill-all --dev --json
// - buckycli node restart --wait healthy --json
//
// 旧脚本兼容：
// - `src/check.py` => buckycli node check --level standard
// - `src/start.py` => buckycli node ensure-running --mode activation --update
// - `src/stop.py`  => buckycli node stop --mode kill-all --dev
//
// =============================================================================
// 11. 仍需确认的问题
// =============================================================================
//
// 1. 容器识别规则应优先使用 Docker label 还是命名约定。目前 `src/stop.py` 只杀
//    `devtest-*` 容器，产品环境需要更明确的边界。
// 2. node_daemon graceful stop 的 live API 名称和最小可用前提尚未固定。
// 3. Windows `stop.ps1` 与 `src/stop.py` 的进程清单需要对齐，尤其是 `workflow`。
// 4. macOS/Linux service stop 后是否应默认做一次黑盒残留清理，需要在“产品停止”和“升级/卸载停止”
//    之间保持不同策略。
// 5. DataSafetyState 当前版本默认应为 NotEvaluated；如果要在 stop 前阻断操作，需要 system_control
//    或专门的数据安全检查提供更强信号。
