// System Control API
//
// 系统控制 api 是以整个集群为视角的控制逻辑。
// 很多接口通常依赖系统的一些基础服务的正常工作。
// 包含系统处于特殊模式下的一些控制命令。
//
// 注意：文件名当前为 `system_contorl.rs`，这里先沿用现有命名。
//
// 设计目标：
// - 表达 Zone / Personal Server 级别的故障诊断、重启、恢复、重置、Agent 修复。
// - 把 beta3 故障处理规划中的“用户视角故障状态机”落成控制面协议。
// - 依赖 BuckyOS 最小内核可用：至少需要 Gateway/SystemConfig/NodeDaemon 这类基础控制面
//   能读状态、发指令、记录 operation。
// - 不把分布式系统的整体 reboot 简化成“所有节点一起 stop/start”。
// - 所有复杂修复动作都必须具备：分类、置信度、恢复点、授权、执行计划、验证和审计。
//
// 作用边界：
// - 以“整个 Zone / Personal Server”为边界。
// - 依赖 BuckyOS 最小内核，不处理“系统完全不可用”的黑盒控制；那是 node_control 的职责。
// - 依赖不同可用层级：
//   - L0/L1 系统完全不可用或仅诊断 Gateway/静态状态可读：不进入 system_control live API，
//     只能由 node_control、诊断 Gateway 静态接口或 Recovery System 处理。
//   - L2 Gateway + SystemConfig + NodeDaemon：system_control 的最低可用层级，可读拓扑，
//     可发起有限节点动作。
//   - L3 WorkflowHub + Scheduler：可执行授权运维脚本和 Agent 修复。
//   - L4/L5 完整控制面：可管理系统服务、应用服务、恢复流程和数据修复。
// - 本机强控制动作由 node_control 执行；system_control 负责计划、协调、状态聚合。
//
// 典型消费者：
// - BuckyOS Desktop / App 的故障中心
// - control-panel 的系统诊断与恢复页面
// - Agent 自动修复框架
// - buckycli system check/restart/recover/reset
// - 官方支持 / ticket / 第二诊断链路
//
// 推荐 kRPC 路径：
// - 完整控制面：`/kapi/control-panel` 或未来 `/kapi/system-control`
// - Scheduler/WorkflowHub 执行动作：system_control 创建 operation，再下发 FunctionInstance
// - 最小诊断 Gateway：`/diag/v1/system/*` 只暴露 system_control 之前写出的快照/last status；
//   它不是 system_control live API，也不应要求 SystemConfig/NodeDaemon 当前可用。
//
// =============================================================================
// 1. 故障状态模型
// =============================================================================
//
// enum FaultEventState
// - Normal
// - SuspectedFault
// - Classifying
// - DataRisk
// - AvailabilityRisk
// - ServiceRisk
// - Warning
// - SafePoint
// - DiagnoseReachability
// - GuidedAction
// - RecoveryMode
// - ManualRepair
// - AgentRepair
// - Verify
// - Resolved
// - Rollback
// - Escalate
//
// enum FaultLevel
// - Green
//   已恢复或正常。
// - Blue
//   应用或局部功能异常。
// - Yellow
//   降级但可用，例如存储桶不可用但数据仍有保护。
// - Orange
//   系统不可访问或核心服务不可用，但数据安全大概率可确认。
// - Red
//   数据损坏/丢失确认，或系统关键恢复能力不可用。
//
// enum FaultSource
// - SystemDetected
// - UserReported
// - AgentDetected
// - RemoteSupport
// - ExternalMonitor
//
// enum FaultConfidence
// - Confirmed
// - High
// - Medium
// - Low
//
// enum DataSafetyState
// - Safe
// - LikelySafe
// - Unknown
// - AtRisk
// - Damaged
//
// enum AffectedScope
// - Zone
// - Node { node_id }
// - SystemService { service_name, nodes }
// - App { app_id, users, nodes }
// - DataSet { data_scope }
// - Link { source, target }
//
// struct FaultEvent
// - fault_id
// - level
// - state
// - source
// - confidence
// - data_safety
// - affected_scope
// - title
// - user_summary
// - technical_summary
// - first_seen_at
// - last_seen_at
// - last_healthy_at
// - evidence
// - recommended_actions
// - recovery_points
// - related_operations
//
// =============================================================================
// 2. check：系统级只读诊断
// =============================================================================
//
// 方法名建议：
// - system.check
// - system.check_link
// - system.check_data_safety
// - system.get_fault_center
// - system.get_minimal_status
//
// struct SystemCheckRequest
// - scope: SystemCheckScope
// - level: SystemCheckLevel
// - include_nodes: bool
// - include_services: bool
// - include_apps: bool
// - include_link_diagnostics: bool
// - include_data_safety: bool
// - include_recent_data_summary: bool
// - user_report: Option<UserFaultReport>
//
// enum SystemCheckScope
// - Full
// - Reachability
// - DataSafety
// - Kernel
// - SystemServices
// - Apps
// - BackupAndRecovery
// - Node { node_id }
// - Service { service_name }
// - App { app_id }
//
// enum SystemCheckLevel
// - Minimal
//   在 system_control live API 中，Minimal 仍然要求最小内核可用，用于读取 SystemConfig
//   和 node_daemon 汇总状态。若只有 last heartbeat、last healthy status、拓扑快照、故障转储
//   可读，应走诊断 Gateway 静态接口或 node_control，不视为 system_control 可用。
// - Basic
//   读取 SystemConfig 中的拓扑、node/service/app instance 状态。
// - Standard
//   增加链路、Gateway、Relay、DNS/证书、核心服务健康检查。
// - Deep
//   增加数据安全摘要、备份点、对象校验、应用数据边界等高级诊断。
//
// struct SystemCheckReport
// - overall: FaultLevel
// - state: FaultEventState
// - data_safety: DataSafetyState
// - confidence: FaultConfidence
// - availability_layer: SystemAvailabilityLayer
// - title
// - user_summary
// - technical_summary
// - last_healthy_at
// - topology_snapshot
// - node_reports
// - service_reports
// - app_reports
// - link_reports
// - data_safety_report
// - backup_report
// - fault_events
// - recommended_actions
// - capabilities
//
// enum SystemAvailabilityLayer
// - L0PhysicalUnreachable
//   设备、网络或 Host OS 控制入口不可达；system_control 不可用。
// - L1DiagnosticSnapshotReachable
//   仅诊断 Gateway 或静态状态文件可读；system_control live API 不可用。
// - L2MinimalKernelReachable
//   Gateway + SystemConfig + NodeDaemon 可用；system_control 的最低运行前提。
// - L3ControlPlaneReachable
// - L4SystemServiceFault
// - L5AppFault
// - Normal
// - Unknown
//
// 系统级 check 的用户结论优先级：
// - 第一优先级：数据是否安全。
// - 第二优先级：系统最低还能做什么。
// - 第三优先级：下一步推荐动作。
// - 第四优先级：高级用户诊断细节。
//
// =============================================================================
// 3. start：系统级进入运行态
// =============================================================================
//
// 方法名建议：
// - system.start
// - system.start_plan
//
// system.start 的正式语义：
// - 让 Zone 或指定系统范围进入期望运行态。
// - 通常只在恢复流程、重新激活后恢复拓扑、系统级服务恢复时使用。
// - 不负责本机 spawn 进程；本机动作由 node_control 执行。
//
// struct SystemStartRequest
// - target: SystemControlTarget
// - mode: SystemStartMode
// - plan_policy: SystemPlanPolicy
// - preflight: SystemPreflightPolicy
// - wait: SystemWaitPolicy
// - reason
//
// enum SystemControlTarget
// - Zone
// - Node { node_id }
// - NodeGroup { node_ids }
// - SystemService { service_name, nodes? }
// - App { app_id, users?, nodes? }
// - RecoveryTopology
//
// enum SystemStartMode
// - Normal
// - Recovery
// - SafeMode
// - RestoreTopology
// - RestoreUserDataOnly
// - RestoreUserDataAndApps
//
// enum SystemPlanPolicy
// - DryRunOnly
// - PlanThenRequireApproval
// - AutoIfLowRisk
//
// enum SystemPreflightPolicy
// - Basic
// - RequireRecoveryPoint
// - RequireDataSafetyKnown
// - Strict
//
// struct SystemStartPlan
// - operation_id
// - target
// - mode
// - required_availability_layer
// - phases
// - affected_scope
// - risk
// - rollback_strategy
// - approval_required
//
// =============================================================================
// 4. stop：系统级停止/降级
// =============================================================================
//
// 方法名建议：
// - system.stop
// - system.quiesce
// - system.stop_plan
//
// system.stop 的正式语义：
// - 系统级 stop 不应该默认“停止整个 Zone”。
// - 常见用途是进入只读/维修/恢复模式，或停止指定服务/应用的分布式实例。
//
// struct SystemStopRequest
// - target
// - mode: SystemStopMode
// - data_safety_policy: SystemDataSafetyPolicy
// - plan_policy
// - wait
// - reason
//
// enum SystemStopMode
// - QuiesceWrites
//   停止高风险写入，进入维修诊断状态。
// - StopService
//   停止指定系统服务的实例。
// - StopApp
//   停止指定应用服务。
// - EnterMaintenance
//   系统进入维修模式，保留诊断面。
// - EmergencyFreeze
//   数据风险场景下冻结相关写入面，优先保护数据。
//
// enum SystemDataSafetyPolicy
// - RequireSafeOrLikelySafe
// - RequireRecoveryPoint
// - RequireUserApprovalIfUnknown
// - EmergencyAllowed
//
// 不建议 P0 暴露：
// - system.stop Zone --force
// - system.kill_all
// 这类动作应该停留在 node_control 的本机 dev/recovery 兜底能力里。
//
// =============================================================================
// 5. restart / reboot：分布式重启计划
// =============================================================================
//
// 方法名建议：
// - system.restart_plan
// - system.restart_execute
// - system.restart_status
//
// beta3 文档强调：整体 reboot 是复杂、高风险动作。
//
// struct SystemRestartRequest
// - target
// - strategy: SystemRestartStrategy
// - data_safety_policy
// - preflight
// - approval_policy
// - reason
//
// enum SystemRestartStrategy
// - SingleNode
// - RollingNodes
// - ServiceRolling
// - FullZoneWithBarriers
//
// struct SystemRestartPlan
// - operation_id
// - target
// - ordered_steps
// - barriers
// - timeout_policy
// - protection_script_policy
// - rollback_strategy
// - failure_handling
//
// struct RestartBarrier
// - name
// - wait_for
// - timeout_secs
// - failure_policy
//
// 设计要求：
// - 执行前协调保护脚本，避免“刚停又被拉起”。
// - 多节点不能同时盲目重启。
// - 每个阶段都要 check/verify。
// - 失败后要明确停在哪个阶段，下一步是什么。
//
// =============================================================================
// 6. reset：系统重置
// =============================================================================
//
// 方法名建议：
// - system.reset_plan
// - system.reset_execute
//
// enum SystemResetMode
// - FactoryReset
//   出厂重置；是否清除用户数据取决于设备/installer/native helper。
// - Mode2KeepUserData
//   保留用户数据，清理系统状态，设备回到待激活或可重新加入状态。
// - SystemStateReset
//   重置系统状态，但保留用户数据和可重新索引的数据。
//
// struct SystemResetRequest
// - mode
// - target
// - preserve_user_data: bool
// - preserve_app_data: bool
// - require_recovery_point: bool
// - reason
//
// struct SystemResetPlan
// - affected_nodes
// - affected_services
// - data_boundary_report
// - recovery_point_requirement
// - user_visible_warning
// - rollback_strategy
//
// =============================================================================
// 7. recovery / restore：系统恢复
// =============================================================================
//
// 方法名建议：
// - system.list_recovery_points
// - system.recover_plan
// - system.recover_execute
// - system.recover_status
//
// enum SystemRecoveryMode
// - RunningDataRecovery
//   系统内核运行中，从备份/冗余恢复用户数据。
// - RebuildThenRestoreUserData
//   全系统重建后，只恢复用户数据。
// - RebuildThenRestoreUserDataAndApps
//   全系统重建后，恢复用户数据 + 应用 + 应用状态。
// - AppDataRecovery
//   针对单个 app 的应用数据恢复/导入验证。
//
// struct RecoveryPointSummary
// - recovery_point_id
// - created_at
// - system_version
// - topology_version
// - data_scope
// - app_versions
// - verified: bool
// - compatibility
//
// struct SystemRecoveryRequest
// - mode
// - recovery_point_id
// - target
// - version_policy
// - data_policy
// - app_policy
// - reason
//
// enum RecoveryVersionPolicy
// - UseCurrent
// - UseBackupCompatible
// - UseFactoryVersion
// - AllowSelectedVersion
//
// 恢复流程要求：
// - 先恢复空系统和拓扑，再恢复数据。
// - 完整恢复失败时，允许退到“仅恢复用户数据”。
// - 恢复场景允许安装确定版本；正常运行时不鼓励版本回退。
//
// =============================================================================
// 8. app repair：应用排障动作
// =============================================================================
//
// 方法名建议：
// - system.app_repair_plan
// - system.app_repair_execute
//
// enum AppRepairAction
// - Restart
// - Stop
// - BackupAppData
// - ClearAppData
// - ReinstallKeepUserData
// - ReinstallWithVersion
// - ImportAppDataAndVerify
// - GenerateFaultReport
//
// struct AppRepairRequest
// - app_id
// - user_id
// - action
// - data_policy
// - version_policy
// - reason
//
// 设计要求：
// - 必须区分用户数据、应用数据、缓存、可重建索引。
// - 卸载/重装默认不能删除用户数据。
// - 允许备份应用数据后清空重装，再导入验证是否复现崩溃。
//
// =============================================================================
// 9. data safety：数据安全诊断与修复入口
// =============================================================================
//
// 方法名建议：
// - system.data_check
// - system.data_scrub_plan
// - system.data_recover_plan
//
// enum DataFaultKind
// - UserReportedMissing
// - UserReportedCorrupted
// - ObjectHashMismatch
// - DatabaseOpenFailed
// - BucketUnavailable
// - BackupMissing
// - FsBufferRisk
//
// struct DataSafetyReport
// - state
// - confidence
// - backup_point_summary
// - recent_data_activity_summary
// - unbacked_data_summary
// - affected_buckets
// - object_check_summary
// - recommended_actions
//
// 用户安心信息：
// - 最近 24 小时修改文件摘要。
// - 最近备份点时间和范围。
// - 最近健康心跳。
// - 当前在线设备拓扑。
// - 存储桶/对象校验摘要。
//
// =============================================================================
// 10. Agent / 运维脚本修复
// =============================================================================
//
// 方法名建议：
// - system.repair_plan
// - system.repair_execute
// - system.repair_approve
// - system.repair_status
//
// enum RepairExecutor
// - UserGuided
// - SystemScript
// - Agent
// - RemoteSupport
//
// struct RepairActionProposal
// - action_id
// - title
// - user_summary
// - technical_summary
// - executor
// - risk
// - affected_scope
// - preconditions
// - requires_recovery_point
// - requires_user_approval
// - rollback_strategy
// - verification_plan
//
// 运维脚本要求：
// - 声明输入、输出、影响范围、前置条件、回滚策略。
// - 支持 dry-run / preflight。
// - 支持超时、中断和失败状态上报。
// - 不允许默认扩大权限或跨越数据边界。
//
// =============================================================================
// 11. ticket / 状态页 / 第二诊断链路
// =============================================================================
//
// 方法名建议：
// - system.ticket_create
// - system.statuspage_check
// - system.remote_support_enable
// - system.remote_support_disable
//
// struct DiagnosticBundleRequest
// - scope
// - include_logs
// - include_topology
// - include_link_report
// - include_data_summary
// - privacy_level
//
// enum RemoteSupportMode
// - Official
// - Vendor
// - UserManaged
//
// 第二诊断链路要求：
// - 默认关闭。
// - 用户明确授权。
// - 有有效期。
// - 有审计日志。
// - 可撤销。
//
// =============================================================================
// 12. Operation 查询与审计
// =============================================================================
//
// 方法名建议：
// - system.operation_get
// - system.operation_list
// - system.operation_cancel
// - system.operation_approve
//
// struct SystemOperation
// - id
// - action
// - target
// - status
// - state
// - risk
// - phases
// - created_at
// - updated_at
// - approval
// - audit_log
// - verification_report
// - rollback_report
//
// enum SystemOperationPhaseKind
// - Classify
// - Preflight
// - ConfirmRecoveryPoint
// - Plan
// - WaitForApproval
// - Execute
// - Verify
// - Rollback
// - Escalate
//
// =============================================================================
// 13. buckycli 映射
// =============================================================================
//
// 推荐命令：
// - buckycli system check --scope full --json
// - buckycli system check --scope reachability --json
// - buckycli system fault list --json
// - buckycli system restart-plan --target zone --json
// - buckycli system recover list-points --json
// - buckycli system recover plan --mode user-data-only --point <id> --json
// - buckycli system app repair --app <app_id> --action reinstall-keep-user-data --json
// - buckycli system ticket create --from-report <report_id> --json
