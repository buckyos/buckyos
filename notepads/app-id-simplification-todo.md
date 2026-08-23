# BuckyOS App ID 简化与新定义 TODO

> 面向下一步 CodeAgent。
>
> 本文冻结 beta 2.2 breaking change 中 App 身份、安装实例、Node 副本、短域名和 Package namespace 的新关系。
>
> 本轮不保留 `AppInstallationId`、`ZoneInstalled`、旧 `appid = AppDoc.name`、旧 `<installation_id>@<owner>` 等兼容路径；实现时必须同步共享类型、Control Panel、scheduler、node-daemon、verify-hub、SDK、WebUI/CLI、PIKG/PackageEnv 和协议文档。
>
> P0 已于 2026-08-23 完成；权威协议见 `doc/App 安装协议.md`。

---

## 0. 目标完成态

把当前混杂的：

```text
AppDoc.name / appid
App DID
AppInstallationScope
AppInstallationId
AppClass::ZoneInstalled
app_instance_id
replica_instance_id
package namespace
sub hostname
```

收敛为下面四层：

```text
AppDID                         全网唯一的 App 身份
  ↓ to_raw_host_name()
AppId                          AppDID 的 DNS/path-safe、可逆表达
  + owner_user_id
  ↓
AppInstanceId                  Zone 内某个用户安装的 App 实例
  + node_id
  ↓
Replica                        scheduler 在某个 Node 上部署的副本
```

版本、发布对象和部署世代不再进入 App 身份：

```text
AppDID / AppId                 App 产品身份，跨版本稳定
AppDoc ObjectId + version      精确发布版本
DeploymentIdentity             AppInstance 当前部署的精确世代
Package Meta ObjectId          精确 package 内容版本
```

完成后必须满足：

- 一个用户对同一个 AppDID 只有一个 AppSpec，因此同一时刻只有一个活动版本。
- 不同用户安装同一个 AppDID 时形成完全独立的 AppInstance。
- Zone Owner 安装的 App 仍是普通用户 App；是否对 Zone 全体用户开放由 availability policy 表达。
- App 短域名、AppIndex 只是 Zone 内的持久分配结果，不参与 App 身份。
- AppDoc 不再继承 PackageMeta，不再声明身份字段 `app_name/name`。
- App 自有 package namespace 只由 AppId 决定；所有部署 package 必须精确绑定 Package Meta ObjectId。
- NodeConfig 已经包含 Node 范围，`apps` 的 key 不再重复拼接 `node_id`。
- AgentDID 是 Agent 身份；Agent 的运行实现通过显式 binding 指向一个普通 App Service，不把 AgentDID 伪装成 AppDID。
- Control Panel 只生成并提交 InstallPlan；只有 scheduler 可以执行 InstallPlan、分配 Registry 并发布 desired state。
- beta 2.2 只支持从空 SystemConfig 构造，不读取、不迁移 beta 2.1 及更早的 App 安装数据。

---

## 1. 冻结术语与定义

### 1.1 AppDID

`AppDID` 是 App 的全网唯一身份，使用 canonical DID 字符串：

```text
did:web:filebrowser.buckyos.ai
did:bns:filebrowser.buckyos
```

冻结规则：

- AppDID 标识 App 产品，不标识版本。
- AppDID 必须能够解析 `(AppDID, doc_type = "app")` 得到权威 AppDoc。
- AppDID 必须使用 BuckyOS 支持的 **hostname form**；首版 App 身份不接受会被 `to_raw_host_name()` 截断的 DID path form。
- AppDID 不允许 fragment。
- `to_raw_host_name()` 的结果必须是 canonical lowercase ASCII DNS hostname，只允许非空的 `[a-z0-9-]` label 和 `.` 分隔；拒绝端口、`%` 编码、`_`、空 label 和任何路径字符。
- `did:web` 的 `.did` 保留域规则必须与 name-lib 一致，保证 Web DID 不会占用非 Web DID 的 `*.{method}.did` raw hostname。
- 必须增加 round-trip 验证：`DID::from_str(to_raw_host_name(app_did)) == app_did`；不满足则拒绝作为 AppDID。

### 1.2 AppId

`AppId` 是 AppDID 的 raw hostname 表达：

```text
AppId = AppDID.to_raw_host_name()
```

例子：

```text
did:web:filebrowser.buckyos.ai
  -> filebrowser.buckyos.ai

did:bns:filebrowser.buckyos
  -> filebrowser.buckyos.bns.did
```

冻结规则：

- `AppDID` 是身份真相；`AppId` 是确定性、可逆的 key/hostname 表达。
- 不在 AppDoc 中保存重复的 `app_id`；从 `app_did` 统一派生并在配置边界验证。
- `AppId` 不再等于 `AppDoc.name`，也不是展示名称。
- `AppId` 可用于 SystemConfig key、App 私有数据目录、package namespace 和固定环境变量的值。
- 禁止各模块自行解析或拼接 AppId；统一复用 name-lib 的 raw-hostname helper。
- 反向转换统一调用现有 `DID::from_str(raw_hostname)`；不再为 App 身份另造一套 parser。
- 必须为多 label 非 Web DID 增加测试，例如 `filebrowser.buckyos.bns.did -> did:bns:filebrowser.buckyos`；若现有 `DID::from_str` 不能通过严格往返，则修正 name-lib 的 hostname 解析和 AppDID profile 校验。

### 1.3 AppInstanceId

`AppInstanceId` 表示 Zone 内某个 Owner 安装的 App：

```text
AppInstanceId = {app_id}@{owner_user_id}
```

例子：

```text
filebrowser.buckyos.ai@alice
filebrowser.buckyos.ai@bob
filebrowser.buckyos.bns.did@alice
```

冻结规则：

- AppInstanceId 在一个 Zone 的 SystemConfig 命名空间内唯一；跨 Zone 的完整身份是 `(zone_did, app_instance_id)`。
- canonical AppId 和 username 都不得包含 `@`，统一用 `rsplit_once('@')` 解析。
- AppInstanceId 不包含版本、AppDoc ObjectId、NodeId、AppIndex 或短域名。
- AppInstanceId 是 scheduler、service discovery、gateway、RBAC、availability、SSO/token 的 App 目标身份。
- `AppServiceSpec.user_id` 重命名为 `owner_user_id`。
- 删除 `AppInstallationId` 和 `AppInstallationScope`；不得保留 hash ID 与新 ID 双轨。

### 1.4 AgentDID 与 Agent Service binding

AgentDID 是 Agent 本身的身份；Agent 的代码、package 和进程由一个普通 App Service 提供。二者关系类似“一个权威域名/服务入口指向一个具体 App Service”，不能因为复用了 App 调度链路就把 AgentDID 当成 App 产品 DID。

```text
AgentDID
  ↓ to_raw_host_name()
AgentId
  ↓ AgentServiceBinding
AppInstanceId + service_name
  ↓
scheduler / NodeConfig / node-daemon 中的普通 App Service
```

设计方向：

- `AgentDocument.id`/AgentDID 是 Agent 身份真相，`AgentId = agent_did.to_raw_host_name()`。
- Agent runtime 的产品身份仍由独立 AppDID/AppDoc 表达，例如 Jarvis Agent 可以绑定到 OpenDAN/Jarvis runtime AppInstance。
- `users/{owner}/agents/{agent_id}/spec` 保存 AgentDoc snapshot 与 `AgentServiceBinding`，不是另一份 `AppServiceSpec`。
- `AgentServiceBinding` 至少包含 `agent_did`、`agent_doc_object_id`、`target_app_instance_id`、`service_name` 和 generation；具体 schema 必须在 Phase 1 冻结。
- scheduler 根据 binding 把 AgentDID 对应的 gateway/service endpoint 投影到目标 App Service；node-daemon 只运行目标 AppInstance，不创建第二套 Agent 容器身份。
- 一个 AppInstance 可以承载一个或多个 AgentDID；Agent 的密钥、授权和消息身份始终使用 AgentDID，不能退化为 runtime AppDID/AppInstanceId。
- Agent 的创建/删除与 runtime App 的安装/卸载是两个生命周期；删除 Agent binding 不自动卸载共享 runtime App，卸载 runtime App 前必须检查 Agent binding 引用。
- Agent 不进入 `AppRegistry.apps/instances`；只有它绑定的普通 AppInstance 获得 AppName、AppHostName 和 AppIndex。

### 1.5 Replica

Replica 是 AppInstance 在某个 Node 上的调度副本：

```text
ReplicaKey = (app_instance_id, node_id)
```

冻结规则：

- `replica_instance_id` 不再是对外稳定身份。
- scheduler 内部如需字符串日志表达，可使用 `{app_instance_id}@{node_id}`，但不得把它作为新的协议主键传播。
- 当前一个 AppInstance 在同一个 Node 上最多一个副本；若未来支持同 Node 多副本，再显式增加 `replica_slot`。
- NodeConfig 的父路径已经包含 NodeId，因此 `NodeConfig.apps` 直接以 AppInstanceId 为 key。

### 1.6 DeploymentIdentity

DeploymentIdentity 表示某个 AppInstance 当前部署的精确内容世代：

```text
DeploymentIdentity {
    app_instance_id,
    task_id,
    app_doc_object_id,
    spec_generation,
    pikg_digest?,
}
```

冻结规则：

- 删除其中的 `installation_id`。
- AppDoc ObjectId 或最终批准配置变化时增加 `spec_generation`。
- scheduler desired、NodeConfig scheduled、runtime report、static evidence 必须携带并精确比较同一个 DeploymentIdentity。

### 1.7 AppName、AppHostName、AppIndex

为避免重新混入身份，固定下列含义：

```text
app_name           Zone 内为 AppDID 分配的稳定、尽量短的 DNS label
app_host_name      为 AppInstance 分配的默认短域名 label
shortcut_hostname  管理员配置、指向 AppInstanceId 的额外入口
app_index          Zone 内为 AppInstanceId 分配的稳定数字顺序号
```

规则：

- `app_name/app_host_name/app_index` 都不是 App 身份。
- AppName 只用于子域名，不进入 AppDoc、PackageId、数据目录、鉴权或进程身份。
- AppName 按 AppDID/AppId 分配一次，卸载后继续保留。
- AppHostName 和 AppIndex 按 AppInstanceId 分配一次，卸载后继续保留。
- AppIndex 继续按旧规则参与 instance port 计算；不同 Owner 的同一 AppDID 必须得到不同 AppIndex。
- 只有显式管理员清理操作可以释放分配；清理前必须确认无 AppSpec、gateway shortcut、运行副本或安装任务引用。

### 1.8 System service identity

系统服务继续允许使用非 DID 字符串作为服务身份：

```text
ServiceIdentityString = canonical AppDID | SystemServiceId

starts_with("did:")  -> 普通 App，解析 AppDID 并派生 AppId
otherwise             -> SystemBuiltin/SystemServiceId
```

冻结规则：

- 分类必须在调用 `DID::from_str` 之前完成；`DID::from_str` 会把普通 hostname 推断成 Web DID，不能用“能否 parse”为依据区分系统服务。
- SystemServiceId 只是系统内置服务字符串，不创建 AppInstanceId，不进入用户 AppSpec、AppRegistry 或普通安装流程。
- 对普通 App，协议和 token 中的 `appid` 固定表示 raw-hostname AppId；对系统 principal，允许同一兼容字段保存 SystemServiceId，但必须由 principal kind 明确标识为 system。
- RBAC、gateway 和 service discovery 的共享类型必须保留 `App`/`System` 分支，禁止把 SystemServiceId 包装成虚假的 AppDID。

---

## 2. SystemConfig 新路径

### 2.1 AppSpec 与 InstallRecord

协议语义上的 AppDID 在实际路径中使用其 AppId/raw-hostname 表达：

```text
users/{owner_user_id}/apps/{app_id}/spec
users/{owner_user_id}/apps/{app_id}/install_record

users/{owner_user_id}/agents/{agent_id}/spec
users/{owner_user_id}/agents/{agent_id}/install_record
```

其中：

```text
app_id   = app_did.to_raw_host_name()
agent_id = agent_did.to_raw_host_name()
```

约束：

- key 中的 AppId 必须与 Spec 内 `app_did.to_raw_host_name()` 完全相等。
- key 中的 AgentId 必须与 AgentSpec 内 `agent_did.to_raw_host_name()` 完全相等；AgentSpec 通过 `AgentServiceBinding` 引用目标 AppInstanceId，不复制目标 AppServiceSpec。
- 一个路径只保存一个 desired Spec；升级原位替换该 Spec，不创建版本路径。
- AppDoc 的发布真相源仍是 AppDID resolver/Repo/NamedObject；SystemConfig 不新增独立的 `.../app_doc` 路径。
- 安装完成后，`/spec` 内的 AppSpec 保存本次已验证的不可变 AppDoc snapshot，并 pin 精确 `app_doc_object_id`、selected package objids 和 DeploymentIdentity。snapshot 与 ObjectId 不一致时拒绝读取。
- `/install_record` 只保存安装/升级 workflow 状态、scheduler 执行结果与审计信息，不是 App 当前 desired state 的真相源。
- App mutation 以 AppInstanceId 为 ownership key，Agent mutation 以 AgentId 为 ownership key，通过 CAS 串行化。

删除：

```text
zone/apps/{installation_id}/spec
zone/apps/{installation_id}/install_record
users/{owner}/apps/{installation_id}/...
users/{owner}/agents/{installation_id}/...
```

### 2.2 Node 副本配置

```text
nodes/{node_id}/config
  .apps[{app_instance_id}] = AppServiceInstanceConfig
```

`AppServiceInstanceConfig` 至少包含：

```text
target_state
node_id
node_execution_spec
service_ports_config
deployment
```

其中 `node_execution_spec` 是 scheduler 从 AppSpec 解析出的最小节点执行投影，包含精确 package ObjId、runtime、mount、permission 和 expose 所需字段；不得再嵌入整份 AppServiceSpec 或 AppDoc。

NodeConfig 的事务边界：

- NodeConfig 是 node-daemon 单次 reconcile 的完整、自包含 desired transaction；node-daemon 执行时不得再回读 AppSpec、AppDoc、AppRegistry 来补字段。
- scheduler 每次写 `nodes/{node_id}/config` 时，必须把本 revision 执行所需的 AppDoc 有效字段、精确 package ObjId、runtime、mount、permission、service/expose 配置和 DeploymentIdentity 全部复制/投影进 `node_execution_spec`。
- `node_execution_spec` 是执行投影，不是新的产品身份或发布真相源；它必须携带来源 AppDoc ObjectId 和 spec generation，以便拒绝混合世代。
- scheduler 必须构造完整的新 NodeConfig 后一次提交，不能让 node-daemon 观察到只更新 deployment、尚未更新 package/runtime 配置的中间状态。
- Phase 1 必须冻结 `NodeExecutionSpec` 的精确 serde schema、`schema_version` 和 AppSpec -> NodeExecutionSpec 投影函数；node-daemon 只依赖该共享类型。

副本配置的持久化结论：

- 某个 Node 的 scheduled replica desired config 只保存在 `nodes/{node_id}/config.apps[{app_instance_id}]`。
- scheduler 内存状态和 `system/scheduler/snapshot` 都是可重建缓存，不是第二真相源。
- `services/{app_instance_id}/info` 是服务发现派生结果，`instances/*`/`static_evidence/*` 是运行证据，都不是副本配置。

不得再出现：

```text
nodes/{node}/config.apps["{app_instance_id}@{node_id}"]
```

### 2.3 Runtime 与服务发现

```text
services/{app_instance_id}/info
services/{app_instance_id}/instances/{node_id}
services/{app_instance_id}/static_evidence/{node_id}
```

其中：

- `/info` 是 scheduler 生成的服务发现聚合结果。
- `/instances/{node}` 是 node-daemon/runtime 的实际状态上报。
- `/static_evidence/{node}` 是 Static Web 精确部署内容和 gateway 就绪证据。
- 以上路径全部按 DeploymentIdentity 拒绝旧世代上报。

### 2.4 App registry

所有分配记录保存在 **一个 SystemConfig JSON value** 中，不为每个 DID、hostname 或 index 创建独立 KV：

```text
system/app_registry
```

scheduler 是唯一 writer。Control Panel、gateway builder 和展示层可以读取，但不得直接修改。

所有会占用默认 hostname namespace 的 mutation 也必须由 scheduler 串行协调：普通 App InstallPlan 和管理员 shortcut mutation 均先提交 scheduler。Shortcut 的持久真相仍是 gateway settings，但 Control Panel 不得绕过 scheduler 直接写入。

### 2.5 InstallPlan 执行边界

`InstallPlan` 的含义是“提交给 scheduler 执行的不可变计划”，不是 Control Panel 可以自行落地的部署描述。

| 组件 | 所有权 |
|---|---|
| Control Panel | source staging、DID/AppDoc/package 校验、生成并提交 InstallPlan、展示 task 状态 |
| scheduler | 校验/claim InstallPlan、分配 registry、发布 AppSpec/InstallRecord、生成完整 NodeConfig 和 gateway 派生配置 |
| node-daemon | 只执行 NodeConfig 并上报与 DeploymentIdentity 对应的实际状态 |

冻结约束：

- Control Panel 不得直接分配或写入 AppName、AppHostName、AppIndex、instance port，不得直接写 AppSpec、NodeConfig、registry 或 scheduler 生成的 gateway config。
- InstallPlan 只包含申请者确定的输入：AppDID、owner、AppDoc snapshot/ObjectId、selected package ObjId、批准的设置、placement policy、task_id 和 fingerprint；所有 Zone/Node 分配结果都是 scheduler execution result。
- plan 被 scheduler 成功 claim 之前不算 installed；Inspect/Plan 阶段不得产生 durable desired state。
- scheduler 以 `(app_instance_id, task_id, plan_fingerprint)` 做幂等执行。同一 plan 重放返回同一结果；同一 AppInstanceId 上不同 fingerprint 的并发计划必须通过 CAS 串行化或明确拒绝。
- scheduler claim 后在一个受 registry revision 保护的 SystemConfig transaction 中提交 registry 分配和对应的 AppSpec/InstallRecord/派生配置；事务失败不得发布部分 desired state。
- NodeConfig 可以在后续调度轮次独立重建，但每次写入仍必须满足 2.2 的完整 Node transaction 约束。
- cancel、retry、scheduler restart 后的 resume、commit point 前后错误语义必须在 Phase 1 的 InstallPlan execution protocol 中冻结。
- Control Panel 与 scheduler 之间的提交/查询接口可以使用 kRPC 或 TaskManager 现有任务入口，但只能有一个协议真相源，禁止同时保留“Control Panel 本地执行”和“scheduler 执行”两条路径。

---

## 3. App registry 持久数据格式

### 3.1 Overview

服务所有者：scheduler。

`system/app_registry` 持久保存 AppDID/AppId 到 AppName，以及 AppInstanceId 到默认 AppHostName、AppIndex 的稳定分配关系。记录在 App 卸载后仍保留，确保同一 App 或 AppInstance 再次安装时得到相同名字、hostname 和 index。

### 3.2 Data classification

| 数据 | 分类 | 真相源/说明 |
|---|---|---|
| AppName 分配 | Durable | `system/app_registry.apps`，卸载后仍保留 |
| AppHostName/AppIndex 分配 | Durable | `system/app_registry.instances`，卸载后仍保留 |
| Shortcut 配置 | Durable | 继续以 gateway settings 为真相源，registry 不复制 |
| App 是否已安装 | Derived | 由 `users/*/apps/*/spec` 决定，不写 registry |
| Agent service binding | Durable | `users/*/agents/*/spec`，引用 AppInstanceId，不复制进 registry |
| hostname 反向索引 | Disposable | scheduler 读完整 registry 后在内存构建 |
| scheduler snapshot | Rebuildable | `system/scheduler/snapshot`，不是分配真相源 |
| gateway_info/config | Rebuildable | scheduler 从 Spec、registry、service info 重建 |

### 3.3 Storage strategy

- registry 是 Zone 控制面分配配置，需要与 SystemConfig Spec/gateway 事务边界协作，因此使用平台 SystemConfig，而不是 scheduler 私有 RDB。
- 采用一个 versioned JSON object，符合“单 writer、整体读取、数量有限、需要原子分配”的访问模式。
- scheduler 不绑定任何具体数据库后端。
- 所有更新必须使用 SystemConfig CAS；禁止 `get -> 修改 -> blind set`。
- 若将来 registry 超过 SystemConfig 单值大小或查询规模不再适合整体 JSON，必须另起 schema proposal，不能静默拆成每项一个 KV。

### 3.4 Schema definition

建议首版共享类型：

```rust
pub struct AppRegistry {
    pub schema_version: u32,              // 固定首版 1
    pub next_app_index: u32,
    pub apps: BTreeMap<AppId, AppAllocation>,
    pub instances: BTreeMap<AppInstanceId, AppInstanceAllocation>,
    pub updated_at: u64,
}

pub struct AppAllocation {
    pub app_did: DID,
    pub app_name: String,
    pub allocated_at: u64,
}

pub struct AppInstanceAllocation {
    pub app_id: AppId,
    pub owner_user_id: String,
    pub app_host_name: String,
    pub app_index: u16,
    pub allocated_at: u64,
}
```

JSON 示例：

```json
{
  "schema_version": 1,
  "next_app_index": 13,
  "apps": {
    "filebrowser.buckyos.ai": {
      "app_did": "did:web:filebrowser.buckyos.ai",
      "app_name": "filebrowser",
      "allocated_at": 1787400000
    },
    "filebrowser.buckyos.bns.did": {
      "app_did": "did:bns:filebrowser.buckyos",
      "app_name": "filebrowser-buckyos",
      "allocated_at": 1787400100
    }
  },
  "instances": {
    "filebrowser.buckyos.ai@alice": {
      "app_id": "filebrowser.buckyos.ai",
      "owner_user_id": "alice",
      "app_host_name": "filebrowser",
      "app_index": 10,
      "allocated_at": 1787400200
    },
    "filebrowser.buckyos.ai@bob": {
      "app_id": "filebrowser.buckyos.ai",
      "owner_user_id": "bob",
      "app_host_name": "filebrowser-bob",
      "app_index": 11,
      "allocated_at": 1787400300
    }
  },
  "updated_at": 1787400300
}
```

一致性约束：

- registry 顶层和 allocation 类型使用 strict deserialize/`deny_unknown_fields`；需要新增字段时先升级 `schema_version`。
- `next_app_index` 初始为 1，表示下一个从未分配的 AppIndex；值到 3471 表示旧端口区间已耗尽，不能 wrap 或扫描复用空洞。
- `apps` 的 map key 必须等于 `app_did.to_raw_host_name()`。
- `apps[*].app_name` 全局唯一，且不能是系统保留 hostname。
- `instances` 的 map key 必须等于 `{app_id}@{owner_user_id}`。
- `instances[*].app_id` 必须存在于 `apps`。
- `instances[*].app_host_name` 全局唯一，且不能与系统保留 hostname、其它默认 hostname 冲突。
- `instances[*].app_index` 全局唯一、单调分配、不复用，并满足旧端口公式的 u16 范围约束。
- AppName 与 AppHostName 共享同一个默认 hostname namespace；唯一允许的同名，是 Zone Owner 对应 AppInstance 的 AppHostName 等于该 App 自己的 AppName。
- shortcut 仍由 gateway settings 管理；设置或编译 shortcut 时必须同时检查 registry 的默认 hostname。
- registry 不保存 active/deleted 状态；是否安装由 AppSpec 决定。
- `allocated_at/updated_at` 统一为 Unix timestamp seconds；它们只用于审计，不参与身份或分配算法。

### 3.5 Schema version

- 首版 `schema_version = 1`，存于 `system/app_registry` JSON 顶层。
- unknown version 必须 fail closed，scheduler 不得覆盖。
- beta 2.2 使用 no-compat 初始化：实现切换时删除/重建旧分配数据，不编写旧 AppInstallationId 到新 registry 的兼容读取。
- 新 registry 建立后属于 durable data；后续 schema 变化必须显式升级版本。

### 3.6 Upgrade and recovery

- 当前版本：`No-compat`。beta 2.2 只从空 SystemConfig 初始化；启动/安装脚本必须删除旧 App Spec、scheduler snapshot、旧分配序列和旧 NodeConfig，禁止就地读取或迁移。
- registry 更新失败：本轮调度事务失败，不生成临时 hostname/index，不部分写 gateway。
- scheduler 重启：从 registry 恢复既有分配，不重新计算已有 AppName/AppHostName/AppIndex。
- 全新 bootstrap 必须先创建空 `system/app_registry`，再把预装普通 App/runtime 作为 bootstrap InstallPlan 提交 scheduler；builder 不得绕过 scheduler 直接预填普通 App runtime spec 或 AgentSpec。
- `system/app_registry` 缺失但已有任何 AppSpec、AgentSpec 或 NodeConfig，说明 bootstrap 不完整，必须 fail closed 并要求重新从零构造；本版本不提供旧数据恢复/重建 RPC。
- registry JSON 损坏或 version unknown：停止 App allocation/gateway 派生并报告错误，禁止自动清空。
- 正常运行后的显式 allocation 清理仍需 admin 工具/RPC，先做引用检查，再通过 scheduler CAS 修改完整 registry；这不是旧版本迁移工具。

### 3.7 Extensibility

冻结字段：

- AppId 的 raw-hostname 算法。
- AppInstanceId 的 `{app_id}@{owner_user_id}` 结构。
- AppName/AppHostName/AppIndex 分配后不因卸载或安装顺序变化而改变。
- AppIndex 不复用。

可扩展字段：

- allocation 的审计信息、分配原因、手工备注。
- hostname kind/policy 等展示元数据。
- future schema_version 中增加额外 map；禁止改变现有字段语义。

首版不增加自由 `extra` JSON；需要扩展时升级 schema，避免 registry 成为无约束杂物箱。

### 3.8 Query patterns

scheduler 只需要：

1. 读取完整 registry，按 AppId 查 AppName。
2. 按 AppInstanceId 查 AppHostName/AppIndex。
3. 在内存建立 `app_name -> AppId`、`app_host_name -> AppInstanceId` 反向索引做冲突检查。
4. 为新 AppDID/AppInstance 分配记录后 CAS 写回完整 JSON。
5. 构造所有 Node 的 gateway_info/config。

Zone 内 App 数量预期较小，整体 JSON 读取和 O(n) 内存反向索引是接受的。不得为了“查询方便”额外建立另一套持久反向索引真相源。

### 3.9 Registry 字段的只读投影

AppRegistry 是 AppName、AppHostName、AppIndex 的唯一分配真相源，但允许 scheduler 为执行便利把它们复制到下游配置：

- AppServiceSpec 中的 `app_index`、默认 expose hostname，以及 NodeExecutionSpec/gateway config 中对应字段，都是 scheduler 生成的只读投影。
- InstallPlan、Control Panel 表单和普通 App mutation API 不得接受调用者指定这些字段。
- scheduler 每次生成 AppSpec、NodeConfig 或 gateway config 时从同一 registry revision 复制；投影值与 registry 不一致时必须拒绝提交或在下一轮重建，不能反向用投影覆盖 registry。
- 读取 API 可以直接返回投影以减少 join，但协议文档必须标明 `read_only/derived_from = system/app_registry`。
- AgentServiceBinding 自身不复制 AppName/AppHostName/AppIndex；其 gateway 投影使用目标 AppInstance 的 registry allocation。

---

## 4. AppName、AppHostName 和 AppIndex 分配算法

### 4.1 AppName

AppName 从 AppId 的 label 渐进扩展，目标是尽量短且稳定：

```text
filebrowser.buckyos.ai
  -> filebrowser
  -> filebrowser-buckyos
  -> filebrowser-buckyos-ai
  -> filebrowser-<stable-hash-suffix>

filebrowser.buckyos.bns.did
  -> filebrowser
  -> filebrowser-buckyos
  -> filebrowser-buckyos-bns
  -> filebrowser-buckyos-bns-did
  -> filebrowser-<stable-hash-suffix>
```

要求：

- 按 registry 当前已保留的 AppName、AppHostName、gateway shortcut 和系统保留名选择第一个可用候选。
- 分配结果与安装顺序有关，但一旦分配永久稳定。
- 候选必须经过 DNS label 校验：小写、`[a-z0-9-]`、不以 `-` 开始/结束、最多 63 字符。
- 超长候选使用截断 base + stable hash suffix；hash 输入必须是 canonical AppDID。
- `_`、`www`、`sys` 和其它 gateway/system 保留名必须集中定义并拒绝。

### 4.2 AppHostName

首选候选：

```text
Zone Owner 安装：{app_name}
其它用户安装：   {app_name}-{owner_user_dns_label}
```

要求：

- Zone Owner 必须从权威 `zone_owner_user_id` 判断，禁止用 `root/system` 字符串猜测。
- `owner_user_dns_label` 使用统一、确定性的 DNS-safe 转换；转换可能碰撞时追加稳定 hash。
- 首选 hostname 被其它 AppName、AppHostName、shortcut 或系统名保留时，继续追加 Owner 或 AppInstance 的稳定 hash 后缀。
- Zone Owner 的 AppHostName 允许与同一 AppId 的 AppName 相等；除此之外不能跨类型重名。
- 最终 AppHostName 写入 registry，后续不重新计算。
- Zone Owner 发生变化时不得自动重命名已分配 AppHostName。

### 4.3 AppIndex

- AppIndex 按 AppInstanceId 分配；不同 Owner 安装同一 AppDID 时分别分配。
- AppIndex 全 Zone 唯一、单调递增、不复用，并作为只读投影写入 AppSpec/NodeExecutionSpec。
- 首版继续执行旧端口规则，不在本次重构中引入新的动态端口分配协议：

```text
www instance port = BASE_APP_PORT + app_index * 16
其它 service port = InstallPlan/AppDoc 中显式批准的 expose_port
```

- `BASE_APP_PORT = 10000` 时必须保证结果落在 u16；当前公式下 `app_index <= 3470`。达到上限必须拒绝新分配并报告容量错误，禁止溢出或回收旧 index。
- 同一 AppInstance 在不同 Node 使用相同 instance port；端口 namespace 是 Node-local，因此不冲突。
- 同一 Node 上不同 AppInstance 依靠不同 AppIndex 避免 `www` 端口冲突；scheduler 写 NodeConfig 前仍要检查与 SystemBuiltin、显式 expose_port 和已有 replica 的冲突。
- `app_index == 0` 只保留给旧规则已有的显式端口/系统场景，普通 AppInstance 不分配 0。

### 4.4 Shortcut

- shortcut 的目标必须是精确 AppInstanceId。
- shortcut 配置继续保存在 gateway settings，不复制进 registry。
- shortcut mutation 必须提交 scheduler 串行执行；写 gateway settings 和 scheduler 编译 gateway 时都必须检查：系统保留名、registry 默认 AppHostName、其它 shortcut。
- shortcut 删除不影响默认 AppHostName 分配。

---

## 5. Zone App 删除方案

删除：

```text
AppClass::ZoneInstalled
ZONE_APP_PREFIX
SYSTEM_APP_OWNER_ID 作为普通安装 App 的 owner
zone/apps/*
```

新规则：

- 所有普通 App runtime 都由真实 Owner User 安装，存入 `users/{owner}/apps/...`；Agent 身份和 binding 存入 `users/{owner}/agents/...`。
- Zone 必须提供权威、稳定的 `zone_owner_user_id`，并验证它对应 Zone Owner 身份。
- Zone Owner 安装的 App 默认仍属于该 Owner 的数据空间。
- “Zone 全体用户可用”改为显式 availability policy，不再由 AppClass 推导。
- 生命周期管理权限根据调用者是否为 Owner、Zone Owner 或 Admin 判定，不根据 ZoneInstalled enum 判定。
- SystemBuiltin 不进入普通安装模型；非 DID 的 SystemServiceId 继续由系统服务/内置 registry 管理，不创建虚假的普通用户 AppSpec。

必须同步审计：

- `apps.list/get` 的 management origin 和 availability match。
- install/update/start/stop/remove 的权限判断。
- guest/public expose 的规则。
- Desktop 是否向其它用户展示 Zone Owner 共享的 App。
- App 数据目录和 token 中的 owner_user_id。

---

## 6. AppDoc 独立 schema

### 6.1 设计结论

AppDoc 不再 flatten/inherit PackageMeta。AppDoc 是 App 发布文档，PackageMeta 是某个部署 package 的内容文档，两者职责不同。

AppDoc 不再定义身份字段 `app_name/name`。名称分工：

```text
AppDID                     唯一身份
AppId                      AppDID raw hostname
presentation/show_name     展示文本
AppName                    Zone 内短域名分配
PackageMeta.name           package namespace 中的包名
```

### 6.2 新 AppDoc 最小职责

下一步实现前必须在 `doc/App 安装协议.md` 冻结独立 AppDoc schema，至少覆盖：

```text
doc_type = "app"
did: AppDID
version: App semantic version
presentation/show_name
pkg_list
selector/runtime/sdk requirements
permissions
service_config_tips
签发/owner/controller 所需的 named-object envelope 信息
```

要求：

- AppDoc ObjectId 继续使用 canonical JSON/JCS + `appdoc` ObjType。
- `did` 必填，且与 resolver 返回的 AppDID 完全一致。
- 不再校验 `AppDoc.name == AppDID first label`。
- PackageMeta 的 `name/version/content/deps` 不得通过 flatten 混入 AppDoc 顶层。
- PIKG 的 `APPDOC.json`、builder、resolver、Repo/NamedStore 读取、签名与对象 ID 全部同步新 schema。
- AppDoc version 标识语义版本；DID document revision/version 与其分开。

### 6.3 P0 Gate：冻结 AppDoc format

这是实现 Phase 1 的前置 P0 TODO。在以下产出完成并 review 通过前，不得开始删除 `PackageMeta` flatten 或修改线上/fixture 的 `APPDOC.json`：

- [x] 在 `doc/App 安装协议.md` 给出完整 JSON Schema：字段名、类型、required/optional、default、unknown-field 策略和 `schema_version`。
- [x] 冻结 AppDoc named-object envelope：owner、controller、author/signer、verification method、签名覆盖范围及验证失败语义。
- [x] 冻结 ObjectId material：哪些 envelope/body 字段参与 JCS，ObjType 固定值，以及语义相同输入的 canonicalization 规则。
- [x] 冻结 `pkg_list`/SubPkgDesc、selector、runtime、SDK requirement、permissions、service config tips 的精确结构，不引用 `PackageMeta` 隐式字段。
- [x] 明确 resolver 返回的 DID Document、AppDoc snapshot、AppDoc ObjectId、PackageMeta ObjectId 之间的校验顺序。
- [x] Rust serde 与 TypeScript PIKG validator 共用相同字段语义，并提供至少一份 canonical APPDOC.json、JCS bytes、ObjectId 和签名 golden fixture。
- [x] 为缺字段、unknown field、错误 signer/controller、AppDID 不一致、ObjectId 不一致和旧 flatten 格式增加拒绝测试。
- [x] 明确 `AppType::Agent` 只表示该 App product 是 Agent runtime，不表示 AppDoc 本身是 AgentDID Document；Agent 身份使用 1.4 的 AgentDocument/AgentServiceBinding。

---

## 7. PackageId 与 PackageEnv 新约束

### 7.1 Package namespace

App 自有 package namespace 直接使用 AppId：

```text
package_namespace = app_did.to_raw_host_name()
```

例子：

```text
did:web:filebrowser.buckyos.ai
  -> filebrowser.buckyos.ai

did:bns:filebrowser.buckyos
  -> filebrowser.buckyos.bns.did
```

允许的 package name：

```text
[qualifier.]<app_id>
[qualifier.]<sub_package_name>.<app_id>
```

AppDoc 使用语义版本 PackageId，并在 `SubPkgDesc.pkg_objid` 单独保存 Package Meta ObjectId：

```text
[qualifier.]<app_id>#<version> + pkg_objid
[qualifier.]<sub_package_name>.<app_id>#<version> + pkg_objid
```

进入部署投影的 exact PackageId 使用 PackageEnv 已冻结的 ObjectId selector（version 由已验证的 AppDoc/PackageMeta 保留）：

```text
[qualifier.]<app_id>#pkg:<64 lowercase hex>
[qualifier.]<sub_package_name>.<app_id>#pkg:<64 lowercase hex>
```

其中：

- qualifier 只能来自 PackageEnv 明确支持的枚举，例如 `all` 或目标平台 qualifier。
- sub_package_name 是单个安全 label，不允许 `.`、`/`、`..`、绝对路径或 qualifier 保留字。
- 判断 namespace 时必须使用已知 AppId 做完整 suffix 匹配，不能继续按固定 dot 数量或只取最后一段。
- AppDoc 自有 SubPkgDesc 必须落在该 namespace；第三方依赖可以使用其它 namespace，但不能取得当前 App 的友好目录或 gateway server 名。

### 7.2 多版本共存

所有安装计划中的部署 PackageId 必须携带 Package Meta ObjectId。PackageEnv 用精确 ObjectId 安装和加载严格目录：

```text
/opt/buckyos/bin/pkgs/{full_package_name}/{package_meta_objid_filename}
```

因此 Alice 使用 v1、Bob 使用 v2 时可以在同一 Node 共存。

友好路径：

```text
/opt/buckyos/bin/{app_id}
/opt/buckyos/bin/{sub_package_name}.{app_id}
```

只表示 PackageEnv 的当前 friendly/latest alias，不是 runtime 的版本真相。node-daemon 启动 App 时必须使用 AppSpec pin 的 Package Meta ObjectId 解析严格目录，禁止通过 mutable friendly link 选择版本。

### 7.3 PackageEnv 必改行为

- 识别 qualifier 后只移除最前面的 `<qualifier>.`，保留后面的完整 dotted package name。
- 禁止使用 `split('.').last()` 生成 friendly path；否则 `all.web.filebrowser.buckyos.ai` 会错误退化成 `ai`。
- `all` 和当前平台 qualifier 的剥离语义必须一致。
- `PackageId::get_unique_name`、gateway dir package server name、PackageEnv friendly link 必须使用同一 parser。
- strict load 必须按 Package Meta ObjectId 返回不可变目录。
- archive 解包独立检查 entry path、绝对路径、`..` 和 symlink escape；package namespace 校验不能替代 archive 安全校验。

### 7.4 安装前校验

在写 AppSpec、materialize package 或触发 scheduler 前完成：

- [ ] AppDID canonical + raw-hostname round-trip 校验。
- [ ] AppDoc DID 与 resolver snapshot 一致。
- [ ] 每个自有 SubPkgDesc 的 PackageId name 属于 AppId namespace。
- [ ] 每个部署 PackageId 携带 Package Meta ObjectId。
- [ ] Package Meta 回读 ObjectId 与计划一致。
- [ ] PackageMeta.name 与 SubPkgDesc.pkg_id.name 完全一致。
- [ ] PackageMeta version 与 PackageId version 一致。
- [ ] qualifier 与目标 OS/arch selector 一致。
- [ ] subpackage name 和 archive 内容满足安全约束。

---

## 8. SDK、数据目录、进程与鉴权契约

### 8.1 SDK 身份字段

固定环境变量：

```text
BUCKYOS_APP_DID             canonical AppDID
BUCKYOS_APP_ID              AppDID raw hostname
BUCKYOS_APP_INSTANCE_ID     {app_id}@{owner_user_id}
BUCKYOS_OWNER_USER_ID       AppInstance owner
BUCKYOS_DATA_DIR            已解析的私有数据目录
BUCKYOS_APP_TOKEN           当前 AppInstance token（如运行类型需要）
```

删除/替换：

- 动态 `<FULL_APPID>_TOKEN` 环境变量名。
- 把 `appid` 理解为 AppDoc.name 的逻辑。
- 通过短域名或 AppName 推断 App 身份。

### 8.2 数据目录

AppId 是 raw hostname，可作为 AppDID 的稳定目录表达：

```text
$BUCKYOS_ROOT/data/home/{owner_user_id}/.local/share/{app_id}
$BUCKYOS_ROOT/data/cache/{owner_user_id}/{app_id}
```

要求：

- 目录跨 App 版本稳定。
- 不使用 AppName、AppHostName 或 AppIndex 作为数据目录。
- mount、RDB、local cache 和 instance volume 全部以同一 `(app_id, owner_user_id)` 解析。
- App SDK 只能从固定环境变量读取自身身份和目录，不自行解析 NodeConfig。

### 8.3 Docker/进程资源名

AppInstanceId 是语义身份，但 Docker container/volume、systemd unit 或平台文件名可能有更窄字符集。

- SystemConfig key、协议字段、环境变量、日志和支持 `@` 的本地路径直接使用 canonical AppInstanceId，不再引入 `full_appid` 等第二逻辑身份。
- Docker container/volume 和 systemd unit 不能直接使用 AppInstanceId：`@` 有非法字符或特殊语义，且完整 AppId + Owner 可能超过长度限制。
- 统一实现 `AppInstanceId -> RuntimeKey`：

```text
RuntimeKey = lowercase_hex(sha256(utf8(canonical_app_instance_id)))
container  = buckyos-app-{RuntimeKey}
volume     = buckyos-instance-{RuntimeKey}
```

- 使用完整 64 hex digest，不截断；输入字节、前缀和 lowercase 编码必须跨平台一致，并增加 golden vector。
- RuntimeKey/容器名只是可重算的本地控制柄，不进入 SystemConfig 主键、token、RBAC、service discovery 或公开协议，不增加新的身份概念。
- Docker label/annotation 中必须保存完整 AppInstanceId；诊断和回收先通过 label 校验完整身份，不能只相信容器名。
- 禁止各模块自行 `replace('@', '-')`、截断 AppInstanceId 或使用 AppName/AppHostName 生成运行时控制柄。

### 8.4 Token/RBAC/Gateway

- token 必须精确绑定 AppInstanceId；不能只比较 AppId/AppDID。
- `appid` claim 对普通 App 固定表示 raw-hostname AppId；system principal 按 1.8 保存显式 SystemServiceId，并由 principal kind 区分。
- 非系统 App token 同时携带并校验 `app_owner_user_id`。
- gateway host entry 保存 AppId、AppDID（如诊断需要）和精确 AppInstanceId。
- RBAC principal 以 AppInstanceId 区分不同 Owner 安装；AppId 只可用于同一 AppDID 的产品级策略。

---

## 9. 共享类型重构 TODO

### P0：身份类型

- [x] 在 name-lib 冻结 AppDID hostname profile、`to_raw_host_name -> DID::from_str` round-trip 和 `.did` 保留规则；补多 label BNS case。
- [x] 新增强类型 `AppId`，构造时必须由 canonical AppDID 派生或严格反解验证。
- [x] 新增强类型 `AppInstanceId { app_id, owner_user_id }`，Display/FromStr 统一实现。
- [x] 删除 `AppInstallationId`、`AppInstallationScope` 及所有 derive/parse helper。
- [x] 删除公开 `replica_instance_id` 类型/拼接规则，scheduler 使用结构化 `(AppInstanceId, NodeId)`。
- [x] `AppServiceSpec.user_id` 重命名为 `owner_user_id`。
- [x] `AppServiceSpec.app_id()` 返回 AppId，不再返回 AppDoc.name。
- [x] DeploymentIdentity 改为绑定 AppInstanceId。
- [x] 新增显式 `App | System` service identity 分支；在调用 DID parser 前识别非 `did:` SystemServiceId。

### P0：Agent identity/service binding

- [x] 冻结 `AgentId` 强类型和 AgentDID raw-hostname round-trip。
- [x] 冻结 `AgentSpec` 与 `AgentServiceBinding` schema/generation，明确 AgentDoc snapshot/ObjectId 和目标 AppInstanceId/service_name。
- [x] 将当前 Agent `AppServiceSpec` 拆成 Agent identity/binding 与普通 runtime AppSpec；`AppType::Agent` 只保留 runtime product 语义。
- [x] 定义 Agent binding 对 gateway、service discovery、RBAC 和 token 的投影；Agent principal 始终使用 AgentDID。
- [x] 增加 runtime App 卸载前的 Agent binding 引用检查，以及共享一个 runtime 的多 Agent 测试。

### P0：AppClass 与 Zone App

- [x] 删除 `AppClass::ZoneInstalled` 和 `zone/apps` 分支。
- [x] 评估 normal installed App 是否还需要 `AppClass::UserInstalled`；如只剩一个普通安装类型，删除整个字段。
- [x] SystemBuiltin 保持非 DID SystemServiceId/系统 registry 语义，不伪造普通用户安装记录。
- [x] 增加权威 `zone_owner_user_id` 读取接口。
- [x] 将 Zone 全用户可用语义迁移为 availability policy。

### P0：AppDoc

- [x] 先完成 6.3 的 AppDoc format P0 Gate，并将完整 schema/golden fixture 合入 `doc/App 安装协议.md`。
- [x] AppDoc 从 PackageMeta 完全解耦。
- [x] 删除 AppDoc.name 与 AppDID label 相等校验。
- [x] 重建 AppDoc builder、strict deserialize、NamedObject/JCS ObjectId。
- [x] 同步 PIKG app.json -> APPDOC.json 构造和校验。
- [x] 明确 AppDoc owner/controller/signature envelope，不从 PackageMeta 继承。

### P0：SystemConfig schema

- [x] App/Agent Spec 和 InstallRecord 路径改为 AppId/AgentId。
- [x] NodeConfig.apps key 改为 AppInstanceId。
- [x] service info/instances/static evidence 改为 AppInstanceId。
- [x] app mutation ownership key 改为 AppInstanceId。
- [x] Agent mutation ownership key 改为 AgentId，AgentSpec 只保存 AgentDocument snapshot 和 AgentServiceBinding。
- [x] 删除所有 `zone/apps` 列举、fallback 和旧路径候选。
- [x] 新增 `AppRegistry` 共享类型及严格 schema/version 校验。
- [x] AppIndex 移入 AppInstance allocation，并把 AppSpec/NodeExecutionSpec 内同名字段标为 scheduler-only 只读投影。
- [x] 冻结 `NodeExecutionSpec` 精确 schema/version 和 AppSpec 投影函数，保证 NodeConfig 自包含事务。
- [x] bootstrap 先创建空 registry，再通过 scheduler 执行预装 InstallPlan；禁止 builder 直接预填普通 App runtime spec 或 AgentSpec。

### P0：InstallPlan execution protocol

- [x] 冻结 Control Panel -> scheduler 的 plan submit/status/cancel/retry 协议及 TaskManager 对应关系。
- [x] InstallPlan schema 删除 AppName/AppHostName/AppIndex 和 scheduler 分配的 `www` instance port；显式批准的非 `www` expose_port 仍可作为计划输入。
- [x] 冻结 scheduler claim key、幂等键、commit point、CAS 冲突、重启恢复和失败结果。
- [x] 删除 Control Panel deployer 直接写 AppSpec、NodeConfig、registry 和 gateway 派生配置的路径。
- [x] shortcut mutation 同样提交 scheduler，保证默认 hostname namespace 只有一个串行协调者。

---

## 10. 实现阶段与主要入口

### Phase 1：冻结协议和共享类型

- `src/kernel/buckyos-api/src/app_doc.rs`
- `src/kernel/buckyos-api/src/app_install.rs`
- `src/kernel/buckyos-api/src/app_mgr.rs`
- `src/kernel/buckyos-api/src/app_availability.rs`
- name-lib DID helper 所在依赖仓库
- `doc/App 安装协议.md`

产出：6.3 AppDoc format gate 通过；新 AppDoc、AppId、AppInstanceId、AgentServiceBinding、SystemServiceId/App-System identity、DeploymentIdentity、AppRegistry、NodeExecutionSpec schema 编译通过；InstallPlan scheduler execution protocol 已冻结。

### Phase 2：PIKG 与 Package namespace

- `src/tools/buckyos-tool/modules/pikg.ts`
- `src/frame/control_panel/src/app_package_namespace.rs`
- `src/frame/control_panel/src/app_install_planner.rs`
- package-lib `package_id.rs` / `env.rs`
- node-daemon PackageEnv 加载入口

产出：Web/BNS AppId namespace、带 ObjId 的多版本严格加载、dotted friendly name 均通过测试。

### Phase 3：Control Panel 安装与管理

- `src/frame/control_panel/src/app_install_driver.rs`
- `src/frame/control_panel/src/app_install_deployer.rs`
- `src/frame/control_panel/src/app_installer.rs`
- `src/frame/control_panel/src/app_servcie_mgr.rs`
- `src/frame/control_panel/src/sys_auth_backend.rs`

产出：Control Panel 只生成/提交 InstallPlan 和读取执行状态；所有普通 App 进入 `users/{owner}/apps/{app_id}`，同用户同 AppDID 原位升级，不再生成 InstallationId，也不直接写 desired state。

### Phase 4：scheduler 与 registry

- `src/kernel/scheduler/src/app.rs`
- `src/kernel/scheduler/src/scheduler.rs`
- `src/kernel/scheduler/src/system_config_agent.rs`
- `src/kernel/scheduler/src/system_config_builder.rs`

产出：scheduler 幂等执行 InstallPlan，单 writer CAS registry、稳定 AppName/AppHostName/AppIndex，生成自包含 NodeConfig；AppIndex 按 AppInstance 分配并继续执行旧端口公式。

### Phase 5：node-daemon、runtime、SDK 与鉴权

- `src/kernel/node_daemon/src/app_loader.rs`
- `src/kernel/node_daemon/src/app_mgr.rs`
- `src/kernel/node_daemon/src/node_daemon.rs`
- `src/kernel/buckyos-api/src/runtime.rs`
- `src/kernel/verify_hub/src/main.rs`
- boot gateway 配置和 SDK 文档

产出：数据目录、RuntimeKey/容器、环境变量、JWT/RBAC、gateway 全部使用新定义；AgentDID binding 指向普通 App Service，不再从旧 appid/InstallationId 猜身份。

### Phase 6：前端、CLI、文档与清理

- `src/frame/desktop/**`
- `src/tools/buckycli/**`
- `src/tools/buckyos-tool/**`
- `doc/control_panel/Control_Panel_Service.md`
- `doc/arch/system_config_reference.md`
- `doc/arch/10_user_lifecycle_and_permissions.md`
- `doc/arch/11_env_contract.md`
- `doc/sdk/runtime-login.md`

产出：删除旧字段、旧路径、旧注释、旧 fixture 和兼容测试；安装/启动脚本明确清空旧 SystemConfig 并从零构造，文档与当前实现一致。

---

## 11. 验收测试矩阵

### DID/AppId

- [ ] `did:web:filebrowser.buckyos.ai <-> filebrowser.buckyos.ai` 严格往返。
- [ ] `did:bns:filebrowser.buckyos <-> filebrowser.buckyos.bns.did` 严格往返。
- [ ] 反向统一使用 `DID::from_str(raw_hostname)`，不会把多 label `*.bns.did` 错判成 did:web。
- [ ] DID path form、fragment、非法/冲突 `.did` Web hostname 被拒绝作为 AppDID。
- [ ] AppDoc 不含 name 仍能构造稳定 AppDoc ObjectId。

### 安装与身份

- [ ] Alice/Bob 安装同一 AppDID，产生两个不同 AppInstanceId。
- [ ] Alice 不能同时创建同一 AppDID 的两个 Spec。
- [ ] Alice 从 v1 升级 v2 后 AppId/AppInstanceId、数据目录、AppName/AppHostName/AppIndex 不变。
- [ ] 卸载再安装后 registry 分配不变。
- [ ] Zone Owner 安装不产生 ZoneInstalled 或 `zone/apps` 记录。
- [ ] Zone 全用户访问只能通过 availability policy 获得。
- [ ] Control Panel 生成 Plan 后没有 AppSpec/registry/NodeConfig side effect；只有 scheduler claim/execute 后才进入 installed desired state。
- [ ] 同一 `(AppInstanceId, task_id, fingerprint)` 重放得到同一执行结果，不重复分配 index/hostname。
- [ ] beta 2.2 从空 SystemConfig 先创建 registry，再执行 bootstrap InstallPlan；带旧 App/NodeConfig 数据启动时明确拒绝并要求全新构造。

### Agent 与 System service

- [ ] AgentDID/AgentId 与 runtime AppDID/AppInstanceId 分别保存，不互相伪装。
- [ ] 两个 AgentDID 可以绑定同一 runtime AppInstance，删除一个 binding 不停止共享 runtime。
- [ ] runtime App 卸载前存在 Agent binding 时被拒绝。
- [ ] Agent gateway/service endpoint 精确指向 binding 的 AppInstanceId/service_name，Agent token principal 仍是 AgentDID。
- [ ] `did:*` 字符串进入普通 App 分支；`kernel`、`control-panel` 等非 DID 字符串进入 SystemServiceId 分支，且不会被 `DID::from_str` 推断成 did:web。

### Registry 与 hostname

- [ ] 多个相同首 label 的 AppDID 得到渐进增长且唯一的 AppName。
- [ ] Zone Owner 默认 host 可省略 Owner；普通 Owner 默认带 DNS-safe suffix。
- [ ] AppName/Owner 拼接冲突时得到稳定 hash fallback。
- [ ] `_`、`www`、`sys`、shortcut 和默认 hostname 冲突全部拒绝。
- [ ] 并发分配只有一个 CAS 成功，重试后 registry 无重复 name/index/host。
- [ ] registry 损坏或 unknown schema version 时 fail closed，不覆盖数据。

### Scheduler/Node/runtime

- [ ] NodeConfig.apps key 精确等于 AppInstanceId，不包含 NodeId。
- [ ] node-daemon 只读取单个 NodeConfig revision 即可执行，不回读 AppSpec/AppDoc/Registry；配置中不存在跨 generation 混合字段。
- [ ] runtime report 路径为 `services/{app_instance_id}/instances/{node}`。
- [ ] 不同 Owner 的同一 AppDID 获得不同 AppIndex，在同 Node 按旧公式得到不同 `www` port。
- [ ] `app_index = 3470` 仍可分配合法端口，下一 index 明确返回容量错误且不复用旧 index。
- [ ] old DeploymentIdentity report 不能覆盖新部署状态。
- [ ] Static Web 不依赖虚假运行副本，但 gateway/static evidence 绑定精确 AppInstanceId 和 Package ObjId。
- [ ] AppSpec/NodeExecutionSpec/gateway 中 AppName/AppHostName/AppIndex 只读投影与同一 registry revision 一致，调用者不能指定或回写。

### PackageEnv

- [ ] 主包、subpackage、`all`、各平台 qualifier 通过新 namespace 校验。
- [ ] 非当前 AppId namespace 的自有 package 在 Inspect 阶段被拒绝。
- [ ] PackageId 缺少 Package Meta ObjectId 时不能进入 Deploy。
- [ ] Alice v1/Bob v2 在同 Node 安装到不同严格目录并分别启动正确内容。
- [ ] dotted package name 剥离 qualifier 后保留完整 name，不退化为最后一段。
- [ ] friendly link 改变不会改变已经 pin ObjId 的运行实例。
- [ ] archive path/symlink escape 被拒绝且不会留下半安装目录。

### SDK/Auth

- [ ] `BUCKYOS_APP_DID/APP_ID/APP_INSTANCE_ID/OWNER_USER_ID/DATA_DIR` 值符合新定义。
- [ ] 数据目录按 `(owner_user_id, app_id)` 隔离并跨升级稳定。
- [ ] RuntimeKey 使用 canonical AppInstanceId 的完整 SHA-256 lowercase hex golden vector；Docker/systemd 名无非法字符，label 可恢复完整 AppInstanceId。
- [ ] 登录和 token verification 精确比较 AppInstanceId，不能只比较 AppId。
- [ ] Desktop 同名 AppDID、不同 Owner 的 launcher 不合并。

---

## 12. Definition of Done

- [ ] 代码中不存在 `AppInstallationId`、`AppInstallationScope` 或 `ZoneInstalled`。
- [ ] 普通 App 的持久路径使用 AppId、运行目标使用 AppInstanceId；Agent 路径使用 AgentId 并通过 AgentServiceBinding 指向 AppInstanceId。
- [ ] `appid` 对普通 App 只表示 AppDID raw hostname；System principal 的兼容字段由明确 kind 标识为 SystemServiceId。
- [ ] AppDoc 不继承 PackageMeta，不含身份性 `name/app_name`。
- [ ] scheduler 是 `system/app_registry` 唯一 writer，完整 JSON 使用 CAS 更新并带 schema_version。
- [ ] Control Panel 不直接执行 InstallPlan；App desired state、registry allocation、NodeConfig 和 gateway 派生配置都由 scheduler 执行/生成。
- [ ] AppName/AppHostName/AppIndex 卸载后不自动释放；AppIndex 按 AppInstance 分配并继续使用旧端口公式。
- [ ] NodeConfig 不再保存带 NodeId 后缀的 App map key。
- [ ] NodeConfig 是自包含事务，NodeExecutionSpec 包含 node-daemon 所需的完整执行投影和精确 generation。
- [ ] PackageId 自有 namespace、PackageMeta name、精确 ObjId 和严格目录形成闭环。
- [ ] SDK 数据目录、进程名、Token、RBAC、gateway 不再混用展示名、短域名和身份。
- [ ] beta 2.2 安装只从空 SystemConfig 构造，不含旧 AppInstallationId/SystemConfig migration 或兼容读取。
- [ ] 协议、共享类型、前后端、工具、fixtures、单测和 DV Test 同步完成。
- [ ] `cargo test`、`uv run buckyos-build.py` 和 App Installer/升级/多用户 DV Test 通过。
