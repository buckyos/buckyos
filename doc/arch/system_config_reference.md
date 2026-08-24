# system-config Key Reference

本文记录当前代码中由系统组件写入或依赖的 system-config key。信息以仓库当前实现为准，主要跟踪 `SystemConfigClient::set/create/append/delete/set_by_json_path/exec_tx` 以及 scheduler 的初始化和调度写入路径。

system-config 是 Zone 内的 KV 真相源。value 在存储层是字符串，系统通常把 JSON、DID Document、JWT/PEM 等序列化后写入；具体类型由 key 的所属组件约定。

## 读写边界

- scheduler 调度时通过 `dump_configs_for_scheduler` 读取 `boot/`、`devices/`、`users/`、`services/`、`system/`、`nodes/` 前缀。
- system-config 服务内部保留 `__meta/` 前缀，不属于业务 key。
- `services/` 与 `system/rbac/` 在 `SystemConfigClient` 有短 TTL 缓存。
- 写入 `boot/config` 或 `system/rbac/*` 后，system-config 服务会刷新安全状态，包括信任 key 和 RBAC。
- `services/control_panel/...` 使用下划线目录名；服务本身的 spec 是 `services/control-panel/spec`，两者当前都存在。

## boot/

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `boot/config` | `ZoneConfig`，包含 Zone DID、Owner、OOD/SN 列表、verify-hub 公钥等启动身份信息。 | scheduler 首次启动初始化 | system-config、scheduler、DID resolver、verify-hub 信任刷新。是 Zone 身份与信任根之一。 |

`boot/config` 是否存在也是 scheduler 判断是否需要首次初始化的标志。

## devices/

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `devices/<device_id>/doc` | 设备 DID Document。OOD 的初始值来自启动配置中的设备文档。 | scheduler 初始化；后续设备管理流程 | system-config DID resolver、scheduler、权限与身份校验流程。 |
| `devices/<device_id>/info` | `DeviceInfo`，设备运行时上报信息，包含设备 doc、资源信息、网络观测等。 | node-daemon 周期上报 | scheduler 用它识别可调度节点、判断节点类型和在线状态。 |

`devices/<device_id>/info` 属于运行时 info，通常由对应节点自己写入；其它组件不应把它当成用户设置。

## users/

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `users/root/settings` | root 用户 `UserSettings`。 | scheduler 初始化 | system-config 权限加载、Control Panel 用户视图。 |
| `users/<user_id>/settings` | `UserSettings`，包含用户类型、状态、资源池、密码哈希、`is_local`、是否允许改密码等账号主数据，不包含显示名或联系方式。 | scheduler 初始化管理员；control-panel 创建/更新用户和邀请接受流程 | scheduler 用它识别用户及角色；system-config 用于特权用户文档加载；smb-service 读取用户列表。 |
| `users/<user_id>/profile` | `UserPrivateProfile`，用户私有 Profile，包含显示名以及系统 contact 私有扩展。 | scheduler 初始化管理员；control-panel 创建/接受用户及 profile 更新流程 | Control Panel 用户资料视图；普通用户可自行读写。 |
| `users/<user_id>/doc` | 用户 DID Document / Owner Document。 | scheduler 初始化管理员；control-panel 创建/接受用户 | system-config DID resolver、登录和权限流程。 |
| `users/<user_id>/key` | 用户私钥材料。 | control-panel 创建用户 | 用户身份管理。敏感数据。 |
| `users/<user_id>/apps/<app_id>/spec` | `AppServiceSpec`，以 AppId 为 key、以 AppInstanceId 绑定 Owner；AppName/AppHostName/AppIndex 是 registry 同 revision 的只读投影。 | scheduler InstallPlan executor | scheduler 调度、rdb_mgr、Control Panel 展示和生命周期管理。Control Panel 安装器不得直接创建该 key。 |
| `users/<user_id>/agents/<agent_id>/spec` | `AgentSpec`，保存 AgentDocument snapshot、AgentId、generation 和指向普通 runtime AppInstanceId/service_name 的 `AgentServiceBinding`。 | scheduler bootstrap/Agent 安装事务 | gateway、service discovery、RBAC、Control Panel Agent 管理。 |
| `users/<user_id>/apps/<app_id>/install_record` | `InstallRecord`：DID/AppDoc 快照、Package Meta ObjectId、PIKG digest、目标 DeploymentIdentity、task/fingerprint 和安装状态。 | scheduler InstallPlan executor；Control Panel 后续状态收敛 | 安装审计、升级判断和状态聚合。 |
| `users/<user_id>/agents/<agent_id>/install_record` | `AgentInstallRecord`，记录 AgentId、AgentDoc ObjectId、binding target、generation 和绑定状态。 | scheduler bootstrap/Agent 安装事务 | Agent binding 审计与管理。 |
| `users/<user_id>/apps/<app_id>/settings` | app 自有 settings JSON。schema 由 app 约定。 | app runtime 通过 `update_my_settings`；也可由管理界面写入 | app runtime 通过 `get_my_settings` 读取。 |
| `users/<user_id>/agents/<agent_id>/settings` | Agent app 自有 settings JSON。schema 由 Agent app 约定。 | Agent runtime 或管理流程 | Agent runtime 读取自己的配置。 |
| `users/<user_id>/apps/<app_id>/info` | app 自有 info 路径，RBAC 中允许 app 写。当前未发现系统组件固定 schema。 | app 自己 | app 自有运行信息。 |
| `users/<user_id>/agents/<agent_id>/info` | Agent app 自有 info 路径，RBAC 中允许 Agent 写。当前未发现系统组件固定 schema。 | Agent app 自己 | Agent 自有运行信息。 |
| `users/<user_id>/samba/settings` | `UserSambaInfo`，包含 `is_enable` 和 SMB 密码。 | SMB/用户设置流程 | smb-service 读取后生成本机 SMB 用户配置。 |
| `users/<user_id>/desktop/<session_id>/_meta` | 桌面 UI session 元数据，包含名称和创建/更新时间。 | control-panel UI session manager | 桌面状态同步与管理。 |
| `users/<user_id>/desktop/<session_id>/<state_key>` | 桌面 UI session 的单项状态 JSON。已知 state key 包括 `appearance`、`window_layout`、`app_items_layout`、`widgets_layout`。 | control-panel UI session manager | 桌面状态恢复。 |

`users/<user_id>/apps/<app_id>/spec` 与 `users/<user_id>/agents/<agent_id>/spec` 分别保存普通 App desired state 和 Agent identity/binding。beta 2.2 不读取旧 `config` 或旧 Agent AppServiceSpec。

## agents/

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `agents/<agent_id>/doc` | Agent DID Document。Jarvis 初始 doc 写在 `agents/buckyos_jarvis/doc`。 | scheduler 初始化；control-panel Agent 身份创建 | system-config DID resolver、opendan、Agent 身份校验。 |
| `agents/<agent_id>/key` | Agent 私钥 PEM。Jarvis 初始 key 写在 `agents/buckyos_jarvis/key`。 | scheduler 初始化；control-panel Agent 身份创建 | Agent 身份签名。敏感数据。 |
| `agents/<agent_id>/settings` | Agent 全局设置。Jarvis 初始值包含 `enabled`、`auto_start`。control-panel 还会维护绑定信息。 | scheduler 初始化；control-panel Agent 管理 | Agent 管理和绑定查询。 |

`agents/<agent_id>/...` 是 Agent 身份层配置；Agent 服务调度规格在 `users/<user_id>/agents/<agent_id>/spec`。

## services/

### 服务规格、实例和派生信息

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/<service_id>/spec` | `KernelServiceSpec`，系统/框架服务期望状态。 | scheduler 初始化；少量管理流程可更新 state | scheduler 调度服务；rdb_mgr 读取服务 RDB 需求。 |
| `services/<service_id>/instances/<node_id>` | `ServiceInstanceReportInfo`，单个节点上的实例上报。普通 App 的 `service_id` 精确等于 canonical AppInstanceId。 | service runtime / node-daemon | scheduler 聚合成 `services/<service_id>/info`，并以 DeploymentIdentity/generation 拒绝旧报告覆盖新部署。 |
| `services/<service_id>/info` | `ServiceInfo`，scheduler 派生出的服务可用节点列表和 selector 信息。 | scheduler | gateway、服务发现、调用方选择服务节点。 |
| `services/<service_id>/settings` | 服务自有 settings JSON。通用路径，schema 由服务约定。 | 服务 runtime 或管理界面 | 对应服务通过 runtime 读取。 |

scheduler 初始化当前会创建这些系统服务 spec：

- `services/control-panel/spec`
- `services/verify-hub/spec`
- `services/scheduler/spec`
- `services/task-manager/spec`
- `services/kmsg/spec`
- `services/aicc/spec`
- `services/msg-center/spec`
- `services/workflow/spec`
- `services/repo-service/spec`
- `services/smb-service/spec`

其中 `task-manager`、`repo-service`、`msg-center`、`aicc` 的 `spec.spec_config.rdb_instances` 带有默认 RDB instance 配置。

### gateway

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/gateway/settings` | `ZoneGatewaySettings`，目前主要是 `shortcuts`。 | rootfs boot seed；scheduler shortcut mutation executor | scheduler 生成 gateway 派生配置；shortcut 与默认 hostname 通过 AppRegistry revision 串行化。 |
| `services/control_panel/app_availability/policies/<app_instance_id>` | `AppAvailabilityPolicy`，显式用户/组/guest 规则和 revision。 | Control Panel（Owner session，CAS） | apps.list/check、verify-hub、scheduler gateway access mode 投影。 |

`services/gateway/spec` 当前不是 scheduler 初始化的系统服务 spec。

### verify-hub

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/verify-hub/settings` | `VerifyHubSettings`，初始化包含 `trust_keys` 空数组。 | scheduler 初始化 | verify-hub 相关设置的保留位置。当前信任 key 主要从 `boot/config` 和 RBAC 刷新逻辑派生。 |

verify-hub 的私钥不在 `services/verify-hub/settings`，而在 `security/verify-hub/key`。

### aicc

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/aicc/settings` | AICC provider 和路由配置。包含 provider 实例、模型、默认模型、alias、feature、image model、`routing_config` 等。 | scheduler 初始化；aicc 管理接口；control-panel AI provider 管理 | aicc 启动和 reload 时读取；Control Panel AI 设置页读取和更新。 |

当前 AICC 代码识别的 provider section 包括 `sn-ai-provider`、`openai`、`google`、`gemini`、`google_gemini`、`google_gemini`、`claude`、`anthropic`、`minimax`、`fal`。

### msg-center

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/msg-center/settings` | msg-center 设置。当前初始化包含 `telegram_tunnel`，其中有 `enabled`、`tunnel_did`、`tunnel_id`、ingress/egress 支持、gateway mode、bindings。 | scheduler 初始化 | msg-center 启动和 reload 时读取。 |

### repo-service

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/repo-service/settings` | `RepoServiceSettings`，包含 `remote_source` 和 `enable_dev_mode`。 | scheduler 初始化 | repo-service 读取远端 repo source 和开发模式。 |
| `services/repo-service/pkg_list` | 平台包更新状态表。初始化时 node-daemon、buckycli 相关包状态为 `"no"`。 | scheduler 初始化 | repo-service / 包管理流程使用。 |

### control-panel

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/control_panel/settings/locale` | Control Panel locale 字符串。 | control-panel | Control Panel UI 设置。 |
| `services/control_panel/user_invites/<invite_id>` | 用户邀请记录。包含邀请目标、状态、过期时间等邀请流程数据。 | control-panel user manager | 邀请列表、接受邀请、用户创建流程。 |
| `services/control_panel/ai_models/policies` | AI model policy 辅助配置。 | control-panel AI settings | Control Panel AI 模型管理。 |
| `services/control_panel/ai_models/provider_overrides` | provider override 辅助配置。 | control-panel AI settings | Control Panel AI 模型管理。 |
| `services/control_panel/ai_models/model_catalog` | 模型目录辅助配置。 | control-panel AI settings | Control Panel AI 模型管理。 |
| `services/control_panel/ai_models/provider_secrets` | provider secret 辅助配置。 | control-panel AI settings | Control Panel AI 模型管理。敏感数据。 |

AI provider 的运行时主配置仍是 `services/aicc/settings`；`services/control_panel/ai_models/*` 是 Control Panel 侧的辅助文档。

### smb-service

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `services/smb-service/latest_users` | `Vec<SmbUserItem>`，当前已应用到本机 SMB 的用户快照。 | smb-service | smb-service 下次同步时比较差异。 |
| `services/smb-service/latest_smb_items` | `Vec<SmbItem>`，当前已应用到本机 SMB 的共享项快照。 | smb-service | smb-service 下次同步时比较差异。 |

这两个 key 是 smb-service 的 applied-state 快照，不是用户期望配置。用户期望配置来自用户设置和 SMB 业务配置。

## nodes/

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `nodes/<node_id>/config` | `NodeConfig`，某节点的目标状态。包含 `node_id`、`node_did`、`kernel`、`apps`、`frame_services`、`state`。 | scheduler 初始化和每轮调度 | node-daemon 读取后安装、启动、停止、收敛本机服务和 app。 |
| `nodes/<node_id>/gateway_info` | `NodeGatewayInfo`，scheduler 派生的新 gateway 视图，包含 node_info、app_info、service_info、route map、routes、DID IP hints、trust key 等。 | scheduler | node-daemon / node gateway 读取并生成本机 gateway 运行配置。 |
| `nodes/<node_id>/gateway_config` | 较旧的低层 gateway 配置 JSON。初始化为空；scheduler 会根据 SN、TLS/ACME、static web app 等重新生成。 | scheduler | node-daemon / cyfs-gateway 兼容读取。 |

`nodes/<node_id>/config` 是 scheduler 写入的自包含目标 revision。`apps` map key 精确等于 canonical AppInstanceId `{app_id}@{owner_user_id}`，不包含 NodeId；value 的 `NodeExecutionSpec` 包含 node-daemon 所需的 AppDID、AppDoc ObjectId、packages、permission、service config 和精确 deployment generation。node-daemon 不回读 AppSpec、AppDoc 或 Registry 拼装执行配置。

## system/

| Key | 内容 | 主要写入方 | 主要读取方/意义 |
| --- | --- | --- | --- |
| `system/install_settings` | 安装期 seed 配置。`pre_install_apps` 每项只包含严格版本化的 `pikg_path` 和 `PreInstallPlanSeed`；raw path 是 `$BUCKYOS_ROOT` 相对路径。 | rootfs boot template / scheduler 初始化原样导入 | Control Panel 登录且 InstallRunner 启动后消费；canonicalize 到 `data/cache`、复制到 immutable staging 并生成标准安装任务。Scheduler 不打开 PIKG，也不为普通预装 App 生成 execution record。 |
| `system/control_panel/pre_install_apps/<app_id>` | 预装 seed 尚未成功创建标准 Task 时的最小观测状态，记录 path、digest、task/fingerprint 或 structured error。Task 创建后 TaskManager 是唯一 Stage 真相源。 | Control Panel PreInstallReconciler | Control Panel 诊断和后台低频重试；Scheduler 不读取。 |
| `system/app_registry` | 严格 versioned `AppRegistry`，保存稳定 AppName、AppHostName 和按 AppInstance 分配的 AppIndex。 | scheduler InstallPlan/shortcut executor（唯一 writer，完整 JSON CAS） | scheduler 校验所有 AppSpec 投影并分配默认 hostname/端口索引。 |
| `system/scheduler/install_plan_executions/<execution_key>` | `InstallPlanExecutionRecord`，保存 claim、commit point、registry/spec revision、错误和幂等结果。 | scheduler | submit/status/cancel/retry、重启恢复和 CAS 冲突处理。 |
| `system/system_pkgs` | 系统包信息。当前初始化为空对象。 | scheduler 初始化 | 包管理保留路径。 |
| `security/verify-hub/key` | verify-hub 私钥 PEM。 | scheduler 初始化 | verify-hub 启动时读取，用于 token 签发。敏感数据。 |
| `system/rbac/policy` | Casbin policy 文本。包含初始策略、用户角色、节点角色、服务/app/kernel 分组等。 | scheduler 初始化和重建；control-panel 用户流程会追加用户分组；buckycli 旧流程会追加 app 分组 | system-config RBAC、verify-hub / 权限判断。 |
| `system/scheduler/snapshot` | scheduler 内部 `NodeScheduler` 状态快照。 | scheduler | scheduler 重启后恢复调度状态。 |

`system/rbac/policy` 是动态派生 key。scheduler 会根据当前用户、设备、service spec、app spec 重新生成动态 tail，因此不应把手工追加内容当作长期稳定来源。

## 关键派生链路

### 首次初始化

scheduler 首次启动时，如果 `boot/config` 不存在，会合并 rootfs boot template 和启动配置，创建初始化 key：

1. 写入 Zone 身份：`boot/config`。
2. 写入管理员、OOD、内置 Agent：`users/*`、`devices/*`、`agents/*`。
3. 写入系统服务 spec 和 settings：`services/*/spec`、`services/*/settings`。
4. 创建空 `system/app_registry`，原样保留预装 seed；此时不读取 PIKG，不创建普通 AppSpec、InstallRecord、安装 Task 或 scheduler execution record。Control Panel 启动后才把 seed 转换为标准 Installer Task，最终完整 InstallPlan 仍由 scheduler transaction 创建 Registry/AppSpec/InstallRecord。
5. 写入初始 node target：`nodes/<ood>/config`、`nodes/<ood>/gateway_config`、`nodes/<ood>/gateway_info`。
6. 写入安全和调度基础数据：`system/rbac/policy`、`security/verify-hub/key`、`system/system_pkgs`。

### 调度循环

scheduler 从 system-config dump 当前状态后执行确定性推导：

1. 从 `devices/<node>/info`、`users/<user>/settings`、`services/<service>/spec`、普通 AppSpec 与 AgentSpec binding 识别调度输入，并先严格校验 AppRegistry/schema。
2. 从 `services/<service>/instances/<node>` 聚合运行实例状态。
3. 更新 `nodes/<node>/config`，让 node-daemon 收敛本机目标状态。
4. 生成 `services/<service>/info`、`nodes/<node>/gateway_info`、`nodes/<node>/gateway_config`。
5. 重建 `system/rbac/policy` 动态部分。
6. 状态变化时保存 `system/scheduler/snapshot`。

### 实例上报和服务发现

运行中的服务实例上报到：

```text
services/<service_id>/instances/<node_id>
```

scheduler 聚合后写：

```text
services/<service_id>/info
```

gateway 和服务发现逻辑应优先使用 scheduler 派生的 `services/<service_id>/info`，而不是直接枚举 instance report。

## 禁止的旧路径

| Key | 状态 | 说明 |
| --- | --- | --- |
| `users/<user_id>/apps/<app_id>/config` | 已删除 | beta 2.2 不兼容读取；必须从空 SystemConfig 构造。 |
| `zone/apps/*`、`system/apps/<app_id>/spec` | 已删除 | 普通 App 只在真实 Owner 的 `users/<owner>/apps` 下；SystemBuiltin 不伪造普通 AppSpec。 |
| `services/<service_id>/<config_name>` | 通用服务私有路径 | runtime 提供 helper 生成该路径，但固定语义只应按具体服务文档解释。 |
| `users/<user_id>/apps|agents/<app_id>/<config_name>` | 通用 app 私有路径 | runtime 提供 helper 生成该路径，固定语义只应按 app 自己的 schema 解释。 |

## 维护规则

- 新增系统级 key 时，应在本文添加 key、value 类型、写入方、读取方和是否为敏感数据。
- 改 `spec`、`settings`、`info`、`config` 的字段时，需要同步检查 scheduler、node-daemon、Control Panel、gateway、文档和共享类型。
- 用户可调项优先放在 `settings`；运行时上报放在 `info` 或 `instances`；scheduler 派生目标放在 `nodes/<node>/config`、`services/<service>/info`、`nodes/<node>/gateway_info`。
- 敏感数据当前包括私钥、provider secret、用户/Agent key、verify-hub key。新增敏感 key 时必须同步检查 RBAC policy。


### App Installer 专用 key

| Key | 说明 |
|---|---|
| `system/app_installer/app_index_seq` | app_index 分配序列（control-panel 用 exec_tx CAS 递增，修复扫描 max+1 并发竞态；首次使用时以现有 spec 扫描结果为基线）。 |
