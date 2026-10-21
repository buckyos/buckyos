# Node Executor 设计

> 版本：v1.2-draft，2026-10-21
> 状态：设计稿。本文合并了三份来源，统一成一个组件设计；"当前代码事实"与"本文新增设计"在各节分开标注，代码事实以源码为准。
> 本次范围调整：Thunk 执行器整体后置；先完成 FunctionObject 执行契约及其与 Command Stream、恢复脚本的能力对比，再单独排期。首期聚焦 Command Stream。
> 实施：P0（P0-A 配置与权限、P0-B Command Stream）已按本文实现，落点、wire 固化与实现差异见 §17；2026-10-21 已在单 OOD DV Zone 验证（§17.7）。
> 首期裁决：Linux 形态 A + hosted xllm；严格的一对一 App/Agent 身份映射；设备准入由 cyfs-gateway process-chain 完成，Executor 不接收 remote device 身份；授权快照、跨 Stream 操作归属、持久 store_id 与最小恢复链路一起交付。远端 PromptExec 首期关闭。
> 来源：
> 1. 本文件旧版（node_Executor 需求）：执行 Scheduler 下发的 ThunkObject（Function Instance）。
> 2. `notepads/node-agent-runtime-todo.md`（2026-10-13）：Agent 在指定 Node 上执行 shell/文件工具的 `remote_node` 运行时。
> 3. `notepads/BuckyOS Node Executor设计思路.md`（2026-10-18）：两种执行模式、轻量 Agent Daemon、最终恢复通道。
> 相关文档：`doc/BuckyOS 系统故障的诊断和恢复(beta3).md`（故障分层、第二诊断链路、Agent 修复前置条件）、`doc/arch/Reset & Unbind.md`（物理控制权、Headless）、`doc/arch/BuckyOS Security Arch.md`（L0–L3 用户层级、S0–S3 软件层级、Client Device）、`doc/task_mgr/task_mgr.md`（谁创建谁执行）、`doc/workflow/executor list.md`（func:: / optask::）、`cyfs-gateway/doc/rtcp/rtcp.md` 与 `rtcp rely.md`。

---

## 0. 一句话定位

**Node Executor 是设备本地的执行组件：接收经过授权的执行请求，按设备当前执行态、请求来源和权限配置完成执行，并以可查询、可恢复的方式返回结果。**

规划中的三类请求来源如下。各来源分别完成准入与结果适配，底层进程管理能力按需复用；来源①的完整执行契约待定，不作为首期的依赖：

```text
                 ┌────────────────────────────────┐
 Scheduler /     │ ① ThunkObject（后续阶段）          │   候选交付：Node Config
 Workflow ──────▶│                                │
                 │                                │
 Agent ─────────▶│ ② Command Stream（登录会话）      │──▶ 准入 ─▶ 执行 ─▶ 结果/状态
 (Jarvis 等)     │                                │        │        │        │
                 │                                │     身份/权限  worker   任务/对象/流
 Owner 恢复端 ──▶│ ③ 签名脚本（SEQ 顺序拉取）        │
                 └────────────────────────────────┘
```

它不是：新的调度器、新的身份中心、默认拥有全设备权限的远程 Shell、通用多租户沙箱平台、任务队列系统。

---

## 1. 术语与命名对照

三份来源用了不同名字，本文统一如下。实现时以本表为准，不再新增别名。

| 本文名称 | 含义 | 来源中的旧称 | 代码落点（建议） |
| --- | --- | --- | --- |
| Node Executor | 设备本地执行组件（本文主体） | node_Executor、NodeAgentRuntimeService、Node Executor | 新 crate `src/kernel/node_executor`（lib），node_daemon 内嵌 |
| 执行请求 | 一次被要求执行的工作单元，三类之一 | thunk / command / operation / 签名命令 | 各来源独立 DTO；公共执行描述按实际复用需要定义 |
| Thunk 路径 | 来源①：Scheduler 下发 ThunkObject，整体后置 | node_Executor 需求全文 | 后续 `node_executor::thunk`，待 §6.2 完成 |
| Command Stream | 来源②：Agent 在可信隧道上的登录会话 | 模式二、remote_node、Command Stream | `node_executor::stream` |
| 恢复脚本 | 来源③：按 SEQ 拉取的完整签名脚本 | 模式一、签名命令 | `node_executor::recovery` |
| 执行态（Posture） | 设备当前允许哪类来源、信任哪些公钥 | 安全等级 | `Posture::{Normal, Frozen, Recovery}` |
| Agent Daemon | 非完整 BuckyOS 设备上的受控组件（独立服务或嵌入 App 的 Library） | Agent Daemon（暂名） | 复用 `node_executor` crate + `cyfs-gateway-lib` |
| `remote_node` | agent_tool 侧的 AgentRuntime kind，Command Stream 的客户端 | NodeAgentRuntime | `agent_tool::runtime` + libopendan factory |
| worker | 以目标 OS 身份实际运行命令/脚本的子进程 | 降权 worker、node_runner | `node_executor::worker` |

"安全等级"一词在 `BuckyOS Security Arch.md` 里已用于用户层级（L0 Root/Owner … L3 Guest）和软件层级（S0–S3），故本文把设备的执行姿态改叫**执行态**，避免混用。

设备角色沿用代码中的 `DeviceType`（`buckyos-base/src/name-lib/src/device.rs`）：`OOD`、`Node`、`Device`（即 Client）、`Sensor`、`Browser`。讨论稿中的"Client"对应 `Device`。本文不新增设备类型；"Agent 可控 Client"是 `Device` 上的一项**能力声明**（见 §4.3）。

---

## 2. 当前代码事实（本节只记录已核对的现状）

| 入口 | 事实 | 对本设计的意义 |
| --- | --- | --- |
| `src/kernel/buckyos-api/src/thunk_object.rs` | 已定义 `FunctionType{ExecPkg, Script(lang), OPTask(lang), Operator}`、`FunctionObject`（content/is_pure/timeout/requirements/affinity/params_type/result_type）、`ThunkObject{fun_id, params, metadata}`、`ThunkExecutionResult` 与状态枚举；`params_type` 仍有迁移标准 JSON Schema 的 TODO | 已有类型骨架，完整执行契约尚未定稿；来源①后置，不提前冻结字段语义 |
| `src/kernel/scheduler/src/thunk_runner.rs` | `DefaultThunkRunner::run_thunk` 做参数初步检查、选节点并返回 `Dispatched` 回执；默认 ObjId 检查只判断字符串非空；`ExternalTaskThunkDispatchBackend` 只返回 `"next_step": "external dispatcher should bind task to runner/node"`，**没有把 thunk 真正送到节点** | 后续需补对象校验与执行闭环，首期不接通此 backend |
| `src/kernel/scheduler/src/scheduler.rs` | 有 `OPTask{id, creator_id, body, max_timeout_sec, status, start_time}`；`check_node_op_tasks` 是空实现 | OPTask 状态回收的位置已预留 |
| `doc/task_mgr/task_mgr.md` §5.7、§7 | `scheduler.dispatch_thunk` 只有 schema、无生产者；beta2.2 起 TaskMgr 不含调度语义，"谁创建，谁执行" | 节点不能靠扫描 TaskMgr 领活；结果回写要经任务 owner |
| `src/kernel/workflow/src/task_tracker.rs`、`src/kernel/task_manager/src/server.rs` | workflow 的 Thunk 子任务绑定 workflow 为 executor；TaskMgr 校验结果写入者是否为绑定的 runner | 后续若由 scheduler 写回，需先定义执行权交接；旧 workflow 文档的回报链路不能直接作为实现契约 |
| `src/kernel/node_daemon/src/node_daemon.rs` | 约 5 秒一轮收敛；`NodeConfig{node_id, node_did, kernel, apps, frame_services, state}`；Unix 上 `reap_exited_children_nonblocking` 做全局 `waitpid(-1)` | 需要新增 config 段；全局 waitpid 与 worker 的 Child 冲突，是前置修复 |
| `src/kernel/node_daemon/src/app_loader.rs` | 部分 Worker 仍把宿主 `$BUCKYOS_ROOT/storage/` 以 rw 挂入容器；Agent 已有独立 storage 映射 | **Executor 的私有状态不能依赖全局 storage/ 的访问隔离** |
| `src/kernel/node_daemon/src/run_item.rs` | `RunItemControl{deploy, start, stop, get_state}` 与 `ensure_run_item_state` | Thunk 的"一个实例"语义可复用其状态观测方式，但 thunk 不是长驻服务 |
| `src/kernel/node_daemon/src/kevent_server.rs` | daemon 已托管 `/kapi/kevent` HTTP | 可复用 listener，不复用其信任假设 |
| `src/frame/agent_tool/src/runtime/mod.rs` | `AgentRuntime` 有 descriptor/open/describe_interrupted；registry 只构造 native/tmux/remote_ssh | `remote_node` 作为新 kind，Node 实现由宿主注入 |
| `src/frame/agent_tool/src/llm_bash.rs`、`tasks.rs` | `BashRunner::start` → `CommandHandle{wait, progress, kill, detach}`，auto shell 走进程内 task manager | Command Stream 客户端要实现远端 CommandHandle |
| `src/kernel/node_daemon/src/boot.rs`、`doc/arch/02_boot_and_activation.md` | 设备在 `node_identity` 里保存激活时的 owner key；Zone 不可达时从 DNS TXT 取 `ZoneBootConfig`（含 `owner`/`owner_key`/`extra_info`）并用该 key 验签 | 恢复声明的发现与验证机制**已存在**，本文只定义声明内容 |
| `cyfs-gateway/doc/rtcp/rtcp.md`、`rtcp rely.md` | Tunnel 上多路 Stream，每流独立 AEAD key；透明中转保持端到端点对点语义；tunnel key epoch 24 h 后需重建 | Command Stream 直接跑在 RTCP Stream 上；中转不需要在应用层复制流 |
| `BuckyOS Security Arch.md` §4.4 | Client Device 用 Administrator 签名的设备描述符证明身份 | 轻量客户端的设备身份已有路径 |
| `src/kernel/node_daemon/src/app_loader.rs`、`buckyos-api/src/runtime.rs`、`verify_hub/src/main.rs` | 设备签发的 App LoginAssertion 换取 App session；App token 的 sub 是 owner，target 是完整 AppInstance；既有 RBAC 按 sub 与 target.authorization_key() 判定 | token 不能直接当 AgentDID；按 §7.2 显式映射、校验 binding，保留原 claims 做 RBAC 并按 Agent 检查 profile |
| `buckyos-api/src/app_schema.rs`、`agent_permissions.rs`、`scheduler/src/system_config_agent.rs` | AgentSpec 未强制 AppDID==AgentDID，已有共享 runtime 的只读权限测试；permission_mode 已存在 | 首期 Executor 显式限制一对一同 DID 绑定，权限与 permission_mode 取交集，不把共享 runtime 写成尚不存在的能力 |
| `cyfs-gateway-lib/src/stack/rtcp_stack.rs`、`stack/mod.rs`（cyfs-gateway 仓库） | RTCP 的认证设备身份可用于 process-chain；普通 stream 转发不向下游交付该身份；更新 handler 不自动关闭已有转发连接 | 设备 ACL 和撤销已有转发由 gateway 负责；Executor 只处理登录主体和操作权限 |
| `src/bucky_project.yaml` | 普通覆盖安装保留 data_paths，卸载/全新安装的 clean_paths 包含 local/ | §11 必须区分覆盖升级与账本重置；持久 store_id 防止旧调用跨账本重放 |
| `src/frame/llm_context/src/prompt_engine.rs`、`agent_tool/src/runtime/mod.rs` | 模板 `__EXEC__` 默认 allow_exec=false，PromptExec 路径尚无持久 call_id 契约 | 远端 PromptExec 首期返回 Capability（§7.6） |

---

## 3. 设计原则

1. **不为 Agent 远程访问另造安全体系。** 身份用 DID/DeviceDocument/verify-hub token，信道用 RTCP，配置用 system-config + scheduler，权限用 RBAC + Node Config。
2. **连接身份、登录身份、操作权限三层分开。** 可信设备能建信道 ≠ 任意 Agent 能登录 ≠ 登录后能做任何事。免重复认证 ≠ 免操作授权检查。
3. **Node Config 是正常运行下唯一的授权事实来源。** 信任的是经既有可信分发交付的 scheduler 配置，不是任意同名 JSON。
4. **Owner Root Key 不进入日常运行依赖。** 它只在恢复态作为最终授权根（Security Arch §4.1）。
5. **最终可执行范围 ⊆ Node Config 授权 ∩ Agent permission_mode ∩ 宿主 OS 实际授予 ∩ Executor/适配层已实现。** 首期 ReadOnly Agent 不开放 exec/write/edit，profile 不能放宽全局只读限制。
6. **不承诺 exactly-once 副作用。** 持久去重阻止重复派发；启动后的未知状态如实返回，不自动重跑（遵循长命令 TODO 的 OS 父子进程语义）。
7. **先提供当前确实需要的能力。** v1 不做审批系统、通用沙箱、队列、跨 Zone。早期安全顾虑写入 §15 已知风险，不预先加隔离机制。
8. **协议定义机制，实现决定策略。** 独立服务与嵌入 Library 共享协议与核心逻辑，不形成两套 Executor。
9. **Thunk 执行器整体后置。** 先完成 §6.1 的能力对比和 §6.2 的 FunctionObject 契约；首期不增加 `config.thunks`、Thunk runner 或 TaskMgr 回报链路，不要求 Command Stream 先包装成 FunctionObject。

---

## 4. 设备角色与部署形态

### 4.1 角色与受控能力是两个维度

| 维度 | 问题 | 由谁决定 |
| --- | --- | --- |
| 集群资源 | 设备是否把计算/存储交给集群调度（OOD/Node） | 激活时的 DeviceType + scheduler |
| 受控执行 | 设备是否接受 Agent/Scheduler 的执行请求 | 设备上的 Executor 能力声明 + 管理员配置 |

一台 Node 可以不开放 Agent 执行；一台 `Device`（电脑、电视盒子）可以开放 Agent 执行而不接受容器部署、不要求全天在线。

### 4.2 三种部署形态

| 形态 | 宿主 | 适用设备 | 可用来源 | 宿主权限 |
| --- | --- | --- | --- | --- |
| A. node_daemon 内置模块 | node_daemon 进程（S1 内核组件） | OOD / Node | 首期②；P2③；①另行排期 | 系统级（可切换 OS 用户） |
| B. Agent Daemon 独立服务 | 独立常驻进程，内含 cyfs-gateway-lib + node_executor | 桌面电脑、能装后台服务的终端 | ②（③ 看是否有后台保障） | 安装时授予 |
| C. Agent Daemon 嵌入 App | 单进程 App 内以 Library 形式嵌入 | Android/iOS/盒子等不便装服务的设备 | ② | 不超过 App 自身权限 |

```text
形态 C（单进程）
┌──────────────────────────────────────┐
│ UI：设备状态、受控开关、权限说明         │
│ cyfs-gateway-lib：设备身份、RTCP、Stream │
│ node_executor：登录、权限、命令、结果流   │
│ 平台适配层：用宿主 App 已获授权的本地能力 │
└──────────────────────────────────────┘
```

运行一个本机 Gateway 或嵌入 Gateway Library，不等于承担 Zone 网关角色，也不因此成为 OOD。

### 4.3 能力声明

`Device` 要成为"Agent 可控"，在其 DeviceDocument 的 `extra_info` 中声明（字段名为建议）：

```json
{ "executor": { "proto": 1, "sources": ["stream"], "ops": ["read", "write", "edit", "exec"], "host": "app" } }
```

`host` 取值 `system` / `service` / `app`，表示形态 A/B/C，用于向 Agent 如实呈现能力上限。声明只表示"实现了什么"，不等于"被授权了什么"；授权见 §5.4。

### 4.4 典型需求：把设备配置交给 Jarvis

用户换了影音设备，在设备上启用 Agent Daemon App，对 Jarvis 说"把音频输出改成光纤、关掉动态补偿"。Jarvis 通过 Command Stream 读取当前状态并在授权范围内调整。设备实际开放哪些能力，取决于宿主权限与平台适配层实现，不预设普通 App 能改所有系统设置。

---

## 5. 统一安全模型

### 5.1 两种授权环境

| 环境 | 信任根 | 配置来源 | 依赖 Owner Root Key |
| --- | --- | --- | --- |
| 正常 Zone（Posture=Normal） | 设备身份 + verify-hub token + RBAC | scheduler 下发的 Node Config | 否 |
| 最终恢复（Posture=Frozen/Recovery） | 激活时固化的 owner key（或独立恢复 key，见 D-9） | 公共名字设施上的 Owner 签名声明 + 恢复端点配置 | 是（仅授权状态变化与恢复脚本） |

Scheduler 是正常体系的最高配置来源；Owner 是正常体系之外的最终授权根。二者不是并行争夺控制权的两个管理者。

### 5.2 三层检查（Command Stream 路径）

```text
设备级（gateway）：RTCP 认证设备是否获准访问 executor？ ← process-chain + config.executor.allowed_devices
身份级（Executor）：App token 能否映射为获准的 Agent？ ← 流首帧登录 + 完整 AppInstance/AgentSpec/binding
操作级（Executor）：该 Agent 当前能执行哪些操作？      ← profile / RBAC / permission_mode / OS 身份
```

设备级检查在 gateway 的 process-chain 中完成，先判定再转发；到达 Executor 的 Stream 不包含 remote device 身份。Executor 不接收客户端自报的设备 DID，也不增加 peer context 转交协议。中转设备不能替代 RTCP 认证端点或登录 Agent 的身份；一条 Tunnel 上的新 Stream 不继承前一个 Stream 的登录。

### 5.3 执行态（Posture）

| 执行态 | 进入条件 | 允许来源 | 信任的公钥 |
| --- | --- | --- | --- |
| Normal | 默认；Zone 可达且 Node Config 有效 | ②；①待后续实现 | system-config 中的 Device/Agent 文档、verify-hub trust keys |
| Frozen | 确认有效恢复声明且本设备在范围内（§9） | 无（只保留查询/验证/等待的最小逻辑） | 恢复信任根 |
| Recovery | Frozen + 等待期满 + 恢复授权仍有效 | ③，以及恢复配置明确允许的 ② | 恢复信任根 + 恢复配置列出的设备/主体 |

执行态、恢复信任根公钥和 SEQ 进度属于协议依赖的可信状态，必须持久化在 Executor 私有目录（§11）。如何防止高权限程序篡改它们是厂商/运行环境的责任，协议不规定手段，但不能因此省略条件。

### 5.4 Node Config 承载正常授权

来源①整体后置。候选交付方式是由 scheduler 写 `nodes/<node>/config.thunks`；启用前须按 §6.2 定义调用者、FunctionObject 与目标节点的授权规则，Node Config 中出现任务只代表派发结果。

来源②的授权是 `nodes/<node>/config.executor`（由管理员可写的 `nodes/<node>/settings.executor` 确定性派生）。`Device` 类设备没有 nodes/ 条目，使用 `devices/<name>/executor_config`（D-6）。字段示例（设计，非现有 schema）：

```json
{
  "executor": {
    "enabled": true,
    "revision": "sha256:<normalized-authorization-inputs>",
    "generation": 1,
    "auth_valid_until_ms": 0,
    "allowed_devices": ["<ood-did>", "<laptop-did>"],
    "principal_bindings": {
      "<agent-did>": {
        "app_instance_id": "<app-id>@<owner>",
        "agent_generation": 1, "binding_generation": 1,
        "enabled": true, "app_installed": true,
        "permission_mode": "read_write"
      }
    },
    "profiles": {
      "owner-dev": {
        "run_as": { "uid": 1001, "gid": 1001 },
        "workdir_roots": ["/home/owner/workspaces"],
        "env": { "PATH": "/usr/local/bin:/usr/bin:/bin", "LANG": "C.UTF-8" },
        "principals": ["<agent-did>"],
        "ops": ["open_stream", "describe", "exec", "read", "write", "edit", "query", "cancel"]
      }
    },
    "limits": {
      "max_streams": 4, "max_concurrency": 2,
      "max_execution_ms": 3600000, "max_output_bytes_per_stream": 8388608,
      "max_file_bytes": 1048576,
      "result_retention_secs": 604800, "max_records": 100000,
      "max_store_bytes": 2147483648
    }
  }
}
```

`auth_valid_until_ms` 示例中的 0 为占位，实际必须是 scheduler 发布的未来截止时间；输入依赖与续期规则见 §5.6。`principal_bindings` 是 scheduler 核对权威文档及一对一绑定后生成的必需映射，generation/revision/有效期也由 scheduler 生成，管理员不能经 settings 自报。`allowed_devices` 只由 gateway process-chain 消费，Executor 不据此校验流。profile.ops 使用裸动作名，RBAC 动作加 executor. 前缀；控制操作同样必须列入授权。

规则：
- 缺省关闭。开启要求至少一个完整 profile；首期 `principals` 逐个匹配权威 Agent DID，不接受通配或 User/普通 App 直登。
- `run_as`、roots、env 是管理员授权上限，请求不能自选 UID、shell、结果目录或扩大 roots。
- 形态 A 的 worker UID 非 0 且不同于 daemon；非 root daemon 返回 Capability，不以同 UID 执行。形态 C 的 `run_as` 无意义，由宿主 App 权限决定。
- roots 约束 cwd 和内置文件工具，**不约束 shell 内任意程序的 I/O**；OS 账号权限才是实际边界。不把 roots 描述成 sandbox。
- 未知字段、空白名单、非法 uid/gid 均拒绝启用；未实现 worker 的平台仅开放控制能力，执行请求返回 Capability；旧 NodeConfig 无该段时行为不变。

RBAC 派生：对资源 `obj://node/<node_name>/executor/<profile_id>`，生成通过 §7.2 映射的原 token owner 与绑定 `app:<AppId>` 两侧的精确规则；动作是 `executor.open_stream/describe/exec/read/write/edit/cancel/query`。沿用既有 RBAC 的 `(sub, target.authorization_key())` 检查；不把原 token 的 sub 改成 AgentDID，也不伪造 JWT。AgentDID 与完整 AppInstance 的校验由 Executor 的 profile/binding 准入补足，不能只依赖不含 owner 的 app 授权键。操作权限取 profile.ops、AgentSpec.permission_mode 与 RBAC 的交集；ReadOnly 拒绝 exec/write/edit。query/cancel 还要求 §7.3 的操作归属一致。删除授权、改 binding、收紧 permission_mode 或停用都触发撤权。设置管理权限与 Agent 调用权限分别定义；不给 App 侧读 Agent 私钥的权限。

### 5.5 授权与宿主权限取交集

Executor 是系统服务时可具有被授予的系统能力；嵌入普通 App 时不能超出 App 权限。厂商预装版、高权限版与普通安装版使用同一协议，但对外声明的能力不同（通过 §4.3 的 `host`）。

### 5.6 授权快照、失联与撤权时界

- scheduler 派生的授权快照覆盖 executor settings、设备准入所用的权威设备身份、被授权 Agent 的 spec/binding/generation、AgentSettings.enabled、permission_mode、对应 App 安装状态和相关 RBAC 输入。一对一映射还要检查同一 runtime 的全部现存 binding，包含未启用或未列入 profile 的 Agent；不能只读取白名单内的 spec。映射、启用状态与有效权限作为同一快照交付，Executor 不临时拼接多个来源的旧数据。
- 任一依赖变化都触发重新派生；revision 是规范化授权输入的摘要，generation 是持久、单调的授权版本。续租不改变这两个值，hash 不用于比较版本新旧。管理员先提交 settings；scheduler 按一致的权威输入版本构造 config.executor、gateway 配置与派生 RBAC，检查输入版本未变后在同一 system-config 事务发布，不回写 settings。消费者不能接受旧 generation 的迟到更新覆盖新授权。
- 同一快照分别产生 gateway process-chain 的设备准入规则和 Executor 的主体/profile 规则；二者带同一 revision，独立完成各自的拒绝与撤销。更新期间不承诺跨组件原子切换，必须满足下面的传播上限；控制面能区分两侧是否已应用。
- P0 的工程默认值：scheduler 和节点消费者均以变更通知加最多 5 秒一次的权威读取兜底，读取绕过旧缓存；控制面可用时，从授权变更提交到两侧应用的端到端上限为 15 秒。scheduler 只在成功核对全部授权输入后续期，快照最长有效 30 秒，以 auth_valid_until_ms 表达。同 generation/revision 的权威快照允许更新截止时间，不能因内容版本未变而忽略续租；相同或更早截止时间的重复/迟到快照及本地缓存不延长有效期。重启须取得有效快照后开放入口；接收端使用剩余有效期和单调计时，时钟异常不能延长租期。这些数值是首期实现与验收默认值，不是现有代码保证。
- Executor 应用收紧后的快照时，先阻止受影响主体的新操作和尚未启动的操作，再关闭其 Stream、请求取消活跃操作。终态按实际退出证据提交，不能把取消请求视为已经停止。快照过期同样关闭入口与 Stream 并请求取消活跃操作；历史结果保留，取得新鲜授权后才能再次读取。过期不自动进入 Frozen/Recovery。
- gateway 应用设备 ACL 撤销时，拒绝新的 executor Stream，并关闭受影响设备已有的 executor 转发连接；仅替换新连接 handler 不满足验收。快照过期时关闭全部 executor 转发入口及连接。设备 ACL 撤销在 Executor 看来是普通断线，已启动操作按 §7.4 继续；它不等同于 Agent/profile 撤权，不要求 Executor 按设备取消操作。
- token 自然到期时关闭对应登录流，客户端取得新 token 后重新登录；单纯 token 到期不取消已启动操作。新操作、query/cancel 均需有效登录和当前授权；主体撤权、快照过期与 token 到期分别按上述规则处理。

---

## 6. 来源①：ThunkObject（整体后置，待 FunctionObject 定义）

本节保留后续设计输入。FunctionObject 的 Rust 类型已存在，但代码/包的解析、运行环境、权限、输入输出及副作用语义尚未形成完整契约。Thunk 的 schema 接线、scheduler 派发、节点 runner、TaskMgr 回报与端到端验收整体后置，完成 §6.2 后再单独排期；不列入 P0/P1/P2 的交付要求。这与 `doc/workflow/executor list.md` 将 `func::` / `optask::` 列为后续阶段的方向一致。

### 6.1 三条路径的脚本执行能力对比

`FunctionType::Script(lang)` 与 `OPTask(lang)` 已用 `content` 表达脚本内容；Command Stream 的 `exec` 也能向 shell 提交脚本；来源③则执行完整签名的脚本。三者都能表达具有副作用的程序，实际能力受各自授权与宿主权限约束。

| 比较项 | Thunk / FunctionObject（待定稿） | Command Stream（§7） | 恢复脚本（§8–9） |
| --- | --- | --- | --- |
| 提交单元 | FunctionObject 定义代码/包及类型，ThunkObject 绑定 `fun_id`、本次参数和 metadata | 已登录主体直接提交 `exec{op_id, cmd, cwd, env_overrides, execution_timeout_ms}` | 签名覆盖脚本、seq、目标范围与恢复轮次 |
| 定位与调度 | scheduler 根据资源/亲和性选节点；定义对象的发布、解析与 `fun_id` 校验仍需明确 | 调用方选定 Node/profile；目标离线即失败 | 恢复声明指定目标范围，设备主动拉取 |
| 授权依据 | 提交者、函数定义与目标节点的授权关系待定，OPTask 的宿主权限需单独定义 | 正常 Zone 身份、设备准入、profile 与 OS 身份取交集 | 恢复信任根、执行态、等待期及当前签名授权 |
| 运行环境 | 脚本解释器、依赖、包入口、容器/宿主选择与 OS 身份尚未形成完整契约 | 首期为 profile 指定的 OS 身份与允许的 env、经 roots 校验的请求 cwd 下的 shell；roots 不隔离 shell I/O | 运行于恢复条件下；解释器与必要依赖必须在故障设备上可用 |
| 输入与结果 | 具名参数和 `Fixed/Object/Stream/Iterator` 结果类型；序列化、交付、完成条件待明确 | 命令、文件数据；返回退出状态、stdout/stderr 或文件工具结果 | 脚本与执行回执；accepted、exec_state、delivered 分开持久化 |
| 副作用与重试 | `is_pure`、结果缓存、执行实例与重试 attempt 的关系待定义 | `op_id` 去重；启动后未知状态只查询，不自动重跑 | 每轮 seq 单调推进；结果重报不触发脚本重跑 |

复用边界：可按实际需要共享进程启动、输出收集、超时、取消和本地执行记录；各来源保留自己的准入、操作标识和结果适配。FunctionObject 的 `is_pure` 是声明，不能仅凭它认定任意脚本无副作用、可缓存或可自动重试。Command Stream 与恢复脚本先按各自契约实现；是否把其中某类调用包装成 FunctionObject，留到定义完整后决定。

### 6.2 Thunk 开发前的设计条件

1. **定义与代码绑定**：明确各 `func_type` 的 `content` 格式、包/代码引用、入口与版本解析，以及 `fun_id` 如何绑定实际执行的 FunctionObject；确定 registry / NamedStore 的解析和校验责任。
2. **运行环境与权限**：定义解释器、依赖、cwd/env、宿主或容器选择、OS 身份和资源上限；明确提交者对函数与目标节点的授权，尤其是 OPTask 的宿主执行权限。与 §7/§8 对照后再确定共享执行组件。
3. **输入输出契约**：补全参数 schema、ObjId 可用性检查、四类结果的传输与终结语义；确定业务结果、stdout/stderr 与 Executor 状态的边界，明确超时单位及失败表示。
4. **副作用与实例语义**：定义 `is_pure`、幂等性、缓存、重试 attempt 和取消的关系；明确 Thunk 身份及跨节点唯一派发记录，避免丢回执后的重试在另一节点再次执行。
5. **任务归属与回报**：明确 workflow / scheduler 谁绑定 TaskMgr runner、谁提交结果和取消，统一 TaskId、runner_epoch 与 revision；现有 scheduler 请求仍用整数 TaskId，TaskMgr 已用字符串 TaskId，需在接入时一起处理。

上述契约定稿并完成对比后，再重审以下候选方案；既有类型、runner hint 或流程草图不作为提前接通执行的依据。

### 6.3 候选交付流程

```text
Workflow/Agent 创建 Thunk 任务（TaskMgr）并调用 scheduler.run_thunk
   ↓
scheduler 选节点，把 thunk 写入 nodes/<node>/config.thunks[<thunk_obj_id>]（期望状态）
   ↓ 可选：kevent 通知加速唤醒
node_daemon 收敛循环发现新 thunk → Node Executor 准入 → 启动 runner
   ↓
Executor 把 ThunkExecutionResult 写到 nodes/<node>/thunk_state/<thunk_obj_id>（观测状态，小）
   ↓
scheduler check_node_op_tasks 读取观测状态 → 写回 TaskMgr → 从 config.thunks 移除
```

候选方案的考虑：
- 与 app/service 一样，Node Config 是期望状态、节点上报是观测状态，scheduler 收敛二者，符合 `04_scheduler.md` 的模型。
- 节点不扫描 TaskMgr 领活；图中的 scheduler 写回以 §6.2 的 runner 归属方案成立为前提。若保留 workflow 为 runner，需改为由 workflow 接收回报并提交结果。
- kevent 只是加速通道，不是真相源（dispatch center §6.1 同样原则）。

观测状态只放 `ThunkExecutionResult`（结果对象 id / 小结果值 / url / error / metrics），stdout 等大输出写入 Executor 本地目录并按需发布为 Named Object，不进 system-config（TODO §11 的结论）。

### 6.4 四类 FunctionType 的候选运行方式

以下沿用旧需求作为比较输入，运行环境与参数协议均待 §6.2 定稿。

| FunctionType | 含义 | 候选运行位置 | 候选参数传递 | 排期 |
| --- | --- | --- | --- | --- |
| `OPTask(lang)` | 运维脚本，会影响 NodeState，一般是 bash | 旧方案为宿主直接运行；实际 OS 身份与授权待定 | `THIS_THUNK` 环境变量 = thunk JSON | 整体后置 |
| `ExecPkg` | 启动一个 Pkg 进程，把 ThunkObject 当参数 | 评估复用 app_loader 的 runtime 判定 | 同上 | 整体后置 |
| `Script(lang)` | 脚本（如 Python） | 旧方案为 aios 的 `node_executor` 容器；待与 Command Stream worker 比较 | 同上 | 整体后置 |
| `Operator` | 算子，为本地 FineTune/训练预留 | 待定 | 待定 | 暂不支持 |

Func 执行协议草图（沿用旧需求；stdout 与业务结果的分离方式待定）：

```text
env THIS_THUNK=<thunk_json_str>
func_exe > <exec_dir>/executor_result.json      # 内容为 ThunkExecutionResult 的 JSON
```

候选参数校验：Executor 启动前校验 `FunctionObject.params_type`，`ObjId` 参数须在 NamedStore 存在，`CheckByRunner` 由 runner 自查；scheduler 当前默认检查仍为占位逻辑。候选结果交付：`result_type` 为 `Object` 时，runner 把结果写入 Named Store 并填 `result_obj_id`；其余结果类型随 §6.2 一起定义。

### 6.5 候选状态管理

- 单实例与重试的具体作用域按 §6.2 定义；节点侧拟以 `thunk_obj_id` 为键持久记录 `{task_id, pid, exec_dir, started_at, deadline}`，重复出现在 config 中不重启。跨节点派发还需 scheduler 持久归属记录。
- 超时按 `FunctionObject.timeout` 与 `OPTask.max_timeout_sec`；超时发 SIGTERM，宽限后 SIGKILL，状态置 `Failed(timeout)`。
- 取消：scheduler 从 `config.thunks` 移除且观测状态未终结 → Executor 取消并置 `Cancelled`。
- Executor 不管理 `task.data` 的业务内容；Func 实现不写时，data 只有标准 metadata。
- daemon 重启后：已有 `starting`/`running` 记录但无法确认进程归属的 thunk，报 `Failed(unknown)`，不重跑（原则 6）。

### 6.6 OPTask 的候选用途

`optask::<pkg>@<device>` 的候选用途是低频运维、修复、测试（executor list.md）。恢复点、限定范围和计划记录沿用 beta3 §11.1；执行授权仍须按 §6.2 定义。该用途与 Command Stream 修复命令、来源③签名脚本有交集，后续需确认独立入口的必要性和权限差异，OPTask 不单独提前交付。

---

## 7. 来源②：Command Stream（Agent 路径）

### 7.1 传输：RTCP Stream，点对点优先，允许透明中转

```text
Agent 进程 ──本机 cyfs-gateway（stream 转发入口）──▶ RTCP Tunnel ──▶ 目标设备 gateway ──▶ 127.0.0.1:<executor-port>
                                                   （直连，或经 OOD/SN 透明中转）
```

- 信道认证 = RTCP tunnel 的设备密钥认证；每个 Stream 独立 AEAD key。中转走 RTCP 透明中转，端点仍是一条点对点 Tunnel，**不在应用层复制两条隧道的流**（修正讨论稿 §6.5）。
- gateway 的 process-chain 使用 RTCP 认证得到的设备身份，在转发前执行 allowed_devices 检查；缺失可信身份、规则无效或授权快照过期均拒绝。该身份到 gateway 为止，不放进 Executor 的 login、请求或执行账本。
- Executor 在形态 A/B 监听 `127.0.0.1`，由 gateway 接入；形态 C 经过相同 gateway 准入后在进程内交付 Stream，不开本地端口。本机直接连接也必须通过相同 token/profile/RBAC 检查；回环来源不表示已认证远端设备。设备 ACL 约束远端 gateway 入口，不承担本机进程隔离。
- Agent 侧通过本机 cyfs-gateway 的 stream 转发（socks 或等价入口，`cyfs-gateway/doc/rtcp/rtcp_socks_proxy_example.md`）到达 `<node-did>:executor`；具体入口形式见 D-3。
- RTCP tunnel key epoch 24 h 后必须重建连接：长会话要能在新 Tunnel 上**重新打开 Stream 并重新登录**，不丢在途命令（见 7.4）。
- 不开明文 `/kapi/` 入口；不做 LAN HTTP fallback。

### 7.2 每个 Stream 一次登录

首帧 `login`：

```json
{ "v": 1, "type": "login", "token": "<verify-hub session token>", "profile": "owner-dev", "client": { "runtime_id": "node-dev", "run_id": "..." } }
```

v1 仅接受现有 App session token，按以下顺序完成严格映射（D-2）：

1. 使用既有 verifier 校验签名、issuer/trust key、有效期和 session claims；只接受 `token_use=Session`、`principal_kind=App`、`target_kind=App`。bootstrap/LoginAssertion、refresh 以及 User/Agent/System/Device kind 均不能登录首期 Executor。
2. 保留不可变的已验证 claims；从 target 取得完整 canonical AppInstanceId，要求 token.sub 等于其 owner_user_id。不能把 owner sub 当作 User 直登，也不能将 app authorization key 当作完整实例身份。
3. 使用当前有效授权快照中的 principal_bindings 映射；该映射必须来自该 owner 的权威 AgentSpec，且恰好一个现存 Agent 绑定到该完整 AppInstance。唯一性计入停用的 Agent，不能通过停用其他 Agent 绕过共享 runtime 限制。scheduler 校验 spec/document/binding 一致性与 generation，Executor 核对映射中的 owner、AppInstance、enabled/app_installed，并要求绑定 AppDID == AgentDID。没有绑定、多绑定、跨 owner、DID 不符或停用均拒绝，不临时回源拼接另一版本的 AgentSpec。
4. 将该 AgentDID 作为 canonical principal，逐项匹配 profile.principals；按 §5.4 对原 claims 做 RBAC，并与 permission_mode、profile 和宿主能力取交集。客户端自报身份、runtime_id 和 run_id 均不能替代认证。

通过后返回 `login_ok{ posture, caps, limits, revision, generation, auth_valid_until_ms, store_id, server_epoch }`。`store_id` 是持久账本身份；`server_epoch` 只标识本次 Executor 进程，不能代替 store_id。Executor 不检查 remote device；该检查已由 gateway process-chain 完成。

前置修复限定为 verify-hub 对 Agent-backed App 的 session 签发和 refresh 校验有效 binding/启用状态（AppType::Agent 或有 AgentSpec 引用的 App target）。普通 App 无 Agent binding 时保留既有登录流程，System/Device 等主体不统一强制查 AgentSpec；一对一同 DID 的限制在 Executor 准入实施。Agent 专属凭据交付仍后置，不改变其他服务的身份语义。

登录后不逐命令重复验 JWT，但每次操作都检查登录未过期、当前授权快照和 profile/操作归属；授权更新、快照过期和 token 到期按 §5.6 处理。使用已验证主体复用授权逻辑，不构造假 token。

### 7.3 帧与四类操作

Stream 上是长度前缀的 JSON 控制帧 + 二进制数据帧（单帧 ≤ 16 KiB）。RTCP 提供字节流，应用层必须处理拆包/粘包，不依赖 RTCP 加密记录恰好对应应用帧。控制帧带版本、type 和 request_id；操作请求带 store_id、op_id、run_id、call_id，二进制帧带 op_id、通道与绝对 byte offset。帧头编码、完整 DTO 和错误枚举由 CodeAgent 随客户端/服务端一起固化，不能改变本节的身份与提交语义。

登录后的 `describe` 返回稳定 descriptor、远端 RuntimeInfo 和 store_id，不执行用户命令。以下表格省略共有身份与关联字段：

| 操作 | 请求 | 返回 | 约束 |
| --- | --- | --- | --- |
| `exec` | `{cmd, cwd, env_overrides, execution_timeout_ms}` | `accepted` → `started{pid}` → stdout/stderr → `exited{code}` | cmd 作为数据经有界管道交给 shell，不进 argv；无交互 stdin（v1） |
| `read` | `{path, range}` | `accepted` → 数据帧流 + `done{hash}` | 节点侧解析 roots/symlink；持久保存本次读取结果 |
| `write` | `{path, expected_hash?, content_length, content_hash}` + 数据帧 + `upload_end` | `accepted` → `done{hash}` | 完整上传后才接受；目标目录临时文件 + 原子替换；hash 不符 Conflict |
| `edit` | `{path, edits[], expected_hash?}` | `accepted` → `done{hash, diff_summary}` | 读、匹配、写在同一目标完成 |
| `cancel` | 操作定位字段 | 当前状态 + `cancel_requested` / 已有终态 / NotFound | 当前仍获授权的原主体可跨 Stream 取消；确认停止后才报告 cancelled |
| `query` | 操作定位字段 + `{channel?, offset?, max_bytes?}` | 当前状态、持久工具结果或指定输出片段 | 返回 next_offset、retained_from、truncated、eof；不重新执行原工具 |

操作归属与准入：

- `actor = (AgentDID, canonical AppInstanceId)` 来自已验证登录；profile、runtime_id 与 run_id 绑定到登录上下文。操作归属于该主体和运行上下文，不归属于 Stream 或 remote device。
- `op_id` 由客户端首次发送前生成并持久化，同一次 run/call 不变。去重键为 `(store_id, actor, profile_id, runtime_id, op_id)`；另约束 `(store_id, actor, profile_id, runtime_id, run_id, call_id)` 只能对应一个操作，不能换 op_id 或 kind 重做副作用。
- 请求摘要覆盖校验并补齐默认值后的完整操作、目标、上下文和 write 内容摘要，map 规范排序；不含 token、request_id、连接信息或 server_epoch。先认证并检查归属，再查去重：同键同摘要返回既有状态，同键不同摘要 Conflict；其他主体不能查询、取消或接管该操作。
- 不做待执行队列。请求完整、授权有效、执行槽和磁盘预算已预留后，原子发布不可变请求、摘要和 starting 记录并 fsync，然后才能返回 accepted、启动 worker。无槽返回 Busy，容量不足返回 CapacityExceeded；重复操作查询不占新执行槽。
- write 上传先进入有界临时区，受 max_file_bytes、总磁盘预算和 30 秒上传期限约束；收齐内容、核对长度/hash、收到 upload_end 后再次检查授权与去重，再进入 starting。未完成的上传不改目标、不返回 accepted；断线或超时丢弃，若客户端显式重试须以原 ID/内容完整重传，不自动续传。cancel 只取消已接受操作；取消尚在上传的调用先中止上传，不能把 NotFound 当作上传以后不会提交的保证。
- 有效操作集受 permission_mode 和 profile 双重限制。ReadOnly 一律禁止 write/edit/exec；首期不实现命令白名单例外（D-8）。

### 7.4 长命令、断线与恢复

- 沿用 agent_tool 的 auto/wait 与 `local:shell:<call_id>` 语义；`wait_ms` 只控制客户端等待，不停止命令；`max_execution_ms` 是节点硬上限。
- Stream 断开 ≠ 取消：已经 accepted 的操作继续到结束或超时，包括 read/write/edit。gateway 因设备 ACL 撤销而关流同样遵循此规则；Agent/profile 撤权与授权快照过期则按 §5.6 主动取消。
- 操作状态为 `starting → running → succeeded / failed / timed_out / cancelled / unknown`。starting 表示唯一启动机会已消费，必须在 spawn 前持久化；能证明未执行的启动失败记 failed。重启发现 starting/running，若没有匹配操作键/摘要的持久结果则记 unknown，不恢复派发；已有不可变 result 时据此恢复终态。所有终态（含 unknown/cancelled）不可改写；终态发布顺序为 fsync 已保留输出/文件结果、原子持久发布 result、更新 state、发送终态。没有 accepted 响应不证明服务端未接受，客户端丢响应后先 query。
- 活跃 handle 归 Executor 的操作管理器。新 Stream 重新登录的原主体在当前仍有权限时可 query/cancel 旧操作；spawn、登记 handle、取消和撤权通过同一串行控制门。取消先持久记录 cancel_requested，确认受管执行退出后才发布终态；自然完成的既有终态不改写。daemon 重启后没有可信 handle 的 unknown 不支持按历史 PID 取消；取消不回滚已经发生的副作用或文件替换。
- stdout/stderr 各保留前 max_output_bytes_per_stream 字节，超限继续 drain 并标记 truncated；offset 是原输出的绝对字节位置，不循环复用。read 的数据和其他文件工具结果同样有界持久化，query 读取原结果，不再读一次可能已变化的目标文件。结果过期返回 ResultExpired 和墓碑终态；查询不能被输出背压或执行槽耗尽阻塞。
- 客户端在 `<run_dir>/runtime_operations/<call-key>.json` 原子保存规范请求、摘要和绑定 Node/profile/runtime/run/call/op/store 的 locator，write 内容或不可变内容引用一起保存，fsync 后才发请求。`describe_interrupted` 只查询，不重放、不杀旧 PID；NotFound、unknown 或 StoreMismatch 都不能触发自动换 ID 重跑。
- 不做离线队列：目标离线时 open_stream 直接失败；离线持久交接走 Task Dispatch Center。本地执行日志与去重墓碑仍须保留，无队列不等于无持久账本。

### 7.5 worker 与权限边界（形态 A）

- 每个 `exec` 一个 worker 子进程：清 supplementary groups → setgid/setuid → 清 capabilities + no_new_privs → 清 env/fd；任一步失败即不执行。
- 文件 read/write/edit **也在 worker 身份下**完成，不在高权限 daemon 里做 I/O。
- 路径/roots/symlink 校验在目标 worker 实际 I/O 处完成，以受控目录句柄避免校验后路径被调包；Executor 内同一 canonical path 的写操作串行化。临时文件 + rename 防止半写，expected_hash 检测冲突，但不承诺与外部 shell 写入之间的事务/CAS 隔离。
- env 从 profile 构造再叠加允许的覆盖；禁止注入 daemon token、设备密钥、`BUCKYOS_*` 身份变量、控制侧 Session 路径。
- 输出由父进程经有界管道收集；cancel 不被大输出堵塞；杀 worker 不等于 shell 已停，缺确认即 unknown。
- 前置修复：`node_daemon.rs::reap_exited_children_nonblocking` 的全局 `waitpid(-1)` 改为按 Child 独占 wait。

### 7.6 与 agent_tool / libopendan 的接线

```yaml
runtime:
  kind: remote_node
  id: node-dev
  workdir: /home/owner/workspaces/demo
  remote_node: { node_did: <device-did>, profile_id: owner-dev }
```

- 配置里不放 endpoint、token、私钥；信道与凭据由 libopendan 宿主注入。
- `RuntimeRegistry` 遇到 `remote_node` 而宿主未注入实现时报 Capability；基础 CLI 不回退 native。
- 文件工具采用"完整工具调用远端执行"：客户端 read/write/edit 代理保留工具名与 schema，一次 call_id 对应一次 `op_id`。
- descriptor 稳定 target：Node DID、profile、实际 UID/GID、canonical workdir、store_id、协议版本；除 store_id 变化返回 StoreMismatch 外，其他目标变化返回 RuntimeMismatch。server_epoch、连接 nonce 和快照续期时间不进入稳定 descriptor。远端 workdir 由节点解析，控制侧不得本地 canonicalize 或 flock。
- hosted xllm 的 descriptor、持久 locator、NodeCommandHandle 和 recovery reader 全部纳入 P0；wait/progress/kill/detach 分别映射等待/查询/取消/停止等待，detach 不发送 cancel。目标工具 schema、auto/wait 语义保持一致，文件工具不回退 Runner 本地执行。
- NodeBashRunner.start 的后台执行寿命由显式 execution_timeout_ms 与节点 max_execution_ms 取小值；未指定时使用节点上限，不能机械映射现有 BashRunRequest.timeout_ms。auto 的 wait_ms 只控制客户端等待；同步 run/wait 的 deadline 到期经 handle 请求取消，仍按实际确认结果返回，不把网络超时伪装成命令执行超时。
- **远端 PromptExec 首期关闭**：remote_node 遇到模板 __EXEC__/PromptExec 请求，在实际执行和模型请求之前返回 Capability；不能临时编造 call_id 绕过持久调用契约。待 run/call journal 与恢复语义完成后单独开放，不影响 native 等已有 runtime。
- **完整 OpenDAN Session 远端运行是后续阶段**（TODO §9.4）：未完成 Session helper 部署、workspace 分离与窄范围凭据前，Session 选择 `remote_node` 在推理前返回 Capability。

### 7.7 轻量客户端的上线与退出（形态 B/C）

- 长期身份（已加入 Zone、已授权）与当前在线（App 打开、受控开关开启）分开。退出或关闭受控功能 → 关闭 Stream 入口、断开 Tunnel、Agent 侧不再把它视为可用 Runtime。
- 退出后的在途命令：形态 C 随 App 终止；形态 B 按 7.4 规则跑完或超时。不得把"Agent 已无法连接"等同于"设备上一切已停止"。
- 正常退出、崩溃、系统挂起、断网的可观测时间不同；Agent 侧以 Tunnel 存活 + 最近一次 `login_ok` 判断可用，不以历史在线为准。

---

## 8. 来源③：签名脚本（恢复路径）

### 8.1 交互

```text
Executor（Recovery 态）                 恢复端点（HTTPS 等）
   ├── GET /recovery/<device>/head ──────────▶│
   │◀── {seq, script, sig} ───────────────────┤
   │  验签（恢复信任根）、检查 seq > local_seq、检查目标范围
   │  执行脚本
   ├── POST /recovery/<device>/result {seq, status, output_digest, sig_by_device} ─▶│
   ├── 再次 GET head ───────────────────────▶│  需要继续时发布更大 seq
```

- "完整签名"= 脚本内容 + seq + 目标范围（zone / device DID 列表）+ 恢复轮次 id 一并被签名；不接受只签 URL。
- 控制恢复端点或 HTTPS 不等于拥有命令授权。

### 8.2 SEQ

- 严格递增，不要求连续；本地进度**只因验签通过并被接受执行的命令而推进**，不因服务器声称更大数字而更新。
- 旧 seq 的有效签名不再执行；已消费的当前 seq 再次查到也不重执。
- 序列作用域：按 `(zone, 恢复轮次 id)`，设备级进度持久化在 §11 目录；重启不丢。

### 8.3 三种事实分开

`accepted`（已验签接受）、`exec_state`（running / exited{code} / unknown）、`delivered`（结果是否成功上报）各自持久。断电重启后：已 accepted 但 exec_state unknown 的 seq 不重跑，上报 unknown；结果重报与脚本重跑分开。

### 8.4 待补细节

签名字段与作用域、失败是否消耗序号、结果真实性（设备密钥签回执）、进度推进时点。列入 D-10。

---

## 9. 最终恢复通道

与 `BuckyOS 系统故障的诊断和恢复(beta3).md` §10.2–10.3 合并：本节是其"第二诊断链路"在 BuckyOS 身份体系下的具体化，并回答该文 §14 第 5 问（第二链路连到哪）。

### 9.1 触发：两种入口，缺一不可

| 入口 | 证明 | 等待期 | 适用 |
| --- | --- | --- | --- |
| 本地触发 | 物理按钮 / 本机 App / 激活口特殊指令（beta3 §10.3；Reset & Unbind §2.3 的物理控制权） | 无 | 有人在场、Zone 可达或不可达均可 |
| 远程声明 | 公共名字设施上的 Owner 签名恢复声明 | 有 | 无人在场、设备无显示输入、Zone 不可达 |

讨论稿只写了远程声明。本地触发是对"Owner 私钥泄露"唯一不依赖同一把钥匙的第二因子，必须保留。

### 9.2 远程声明的发现与验证

复用 ZoneBootConfig 机制（02_boot_and_activation.md）：声明放在 DNS TXT 的 ZoneBootConfig `extra_info.recovery` 或同机制的独立记录，由 owner key 签名；设备用激活时固化在 `node_identity` 的 owner key 验签。声明内容（建议）：

```json
{ "round": "<uuid>", "scope": { "zone": true } | { "devices": ["<did>"] }, "endpoint": "https://...", "iat": 0, "exp": 0, "wait_secs": 86400 }
```

- `exp` 必填；过期声明无效，避免遗留声明在每次瞬时断连时触发冻结。
- 公共设施只承担发现与分发；DNS 账号权限不替代验签。
- "立即"受 DNS TTL 与设备查询周期限制；离线设备不视为已冻结。

### 9.3 "Zone 不可达"的定义

按 beta3 §4.4 的可用性层级，不另造谓词：

| 设备 | 不可达判定 |
| --- | --- |
| Node / Device | 连续 N 个周期无法完成到 verify-hub/system-config 的认证请求（经 gateway 与 SN 均失败） |
| OOD | 自身 L2 最小内核（gateway + system-config + node_daemon）健康检查失败，或 system-config 多数派不可达 |

单 OOD Zone 里"OOD 即 Zone"，故 OOD 用自检而不是外部可达性。

### 9.4 Frozen 的范围

Frozen 只收窄**本设备 Executor 的执行面**：关闭来源①②的新准入，取消活跃 Command Stream。**不停止已运行的业务服务，不停止 node_daemon 收敛循环**（beta3"控制爆炸半径"）。App 级冻结只约束 App 自身暴露的能力。

Frozen 的 DoS 风险：持有 Owner key 者可让全 Zone 设备进入 Frozen 而无需等待。缓解：范围如上收窄；`exp` 必填；本地触发可立即退出 Frozen（D-11）。

### 9.5 等待期

设备进入 Frozen 后强制等待 `wait_secs`（24 h 为示例值）再进入 Recovery。计时持久化，重启后继续而非重置；可信时间来源列入 D-11。等待期只提供干预窗口，不提供纠错权：合法用户与攻击者持同一把 key 时设备无法区分，私钥泄露后的撤销/替换另行设计。

### 9.6 恢复端点与两条路径

Owner 在声明里给出端点，设备主动连接取恢复配置，然后：

| 路径 | 行为 |
| --- | --- |
| 签名脚本 | 按 §8 顺序执行 |
| 恢复 Command Stream | 按配置用 RTCP 连接指定设备并保持 Tunnel；设备名单由 gateway process-chain 检查，主体与能力规则交 Executor。恢复态凭据与授权契约在 P2 定稿，不能要求故障中的 verify-hub 继续签发正常态 App token |

端点迁移不重新等待；改变 scope 或允许的主体需重新等待（D-11）。建立恢复 Tunnel 不自动降低执行态，也不绕过 gateway process-chain；恢复配置的生效、到期和退出由两侧按各自职责收敛，Executor 仍不接收 remote device 身份。

### 9.7 退出条件

- 本地触发可随时退出。
- 远程：恢复脚本显式 `exit_recovery`，或重新连上 Zone 且 scheduler 下发的授权 generation 大于进入 Frozen 时记录的 generation，且声明已撤销/过期。revision hash 只判断内容是否相同，不比较新旧；P2 必须定义控制面重建时 generation 的连续性。
- 恢复期间信道中断保持 Recovery 态直到 `exp`。

### 9.8 运行保障分级

普通 App 停止后不能持续查询声明；形态 B/C 不对无人值守恢复作保证。厂商预装或有后台保障的版本才承担该职责。协议支持与运行保障分开声明。

---

## 10. 设备加入生命周期（轻量客户端）

复用既有激活与 Client Device 机制：`Device` 经激活流程取得设备身份与 Zone 归属（`设备激活协议.md`、Security Arch §4.4 Administrator 签名描述符）；管理员在 control-panel 写 `devices/<name>/executor_config`；设备连上 Zone 后拉取配置，启动 Gateway/Executor，向 Agent 体系呈现 Runtime。

```text
安装/预装/启用 Agent Daemon → 激活取得设备身份与 Zone 归属 → 取得 executor_config
→ 启动 Gateway + Executor → 连接允许的设备/可信 OOD → Agent 发现 Runtime 在线
→ Agent 打开 Command Stream 并登录 → 在授权范围内使用
```

呈现 Runtime 不等于 scheduler 可以向它部署服务；Runtime 可访问性与资源可调度性分离。

---

## 11. 持久状态与目录

```text
$BUCKYOS_ROOT/local/node_daemon/executor/        （形态 A；0700，daemon 私有）
  meta.json                {schema_version, node_did, store_id}
  operations/<key-hash>/   不可变 request/digest、state、result、stdout/stderr、文件结果
  tombstones/<key-hash>/   actor/key/digest/终态；正文清理后仍去重
  tmp/                    未接受请求/上传的有界临时内容
  posture.json             P2：执行态、恢复轮次、进入时间、等待截止、授权 generation
  recovery/seq.json        P2：{round, local_seq, accepted[], exec_state{}, delivered{}}
  thunks/<thunk_obj_id>/   后续 Thunk 阶段候选目录，格式待 §6.2 定稿，首期不创建
```

- **不放全局 `storage/`**（部分 Worker 仍会获得宿主全局 storage/ 的 rw 映射）。
- 原子替换 + fsync 复用 `lib_opendan/src/fsutil.rs` 的做法（可下沉到 kit）；单写者 flock。内部 key-hash 从规范身份元组生成，不直接用客户端字符串拼目录；检查路径、owner/mode，重建去重索引完成前不开放执行。
- 形态 B/C 使用平台等价的应用私有目录。
- 普通关闭、重启、覆盖升级必须保留账本与 store_id。按现有安装配置清理 local/ 的卸载、全新安装/重装视为账本重建；需要延续旧操作的升级不能走该清理路径。显式重置或整个账本丢失后生成新 store_id；meta 缺失但存在其他记录、记录损坏或未知 schema 则拒绝执行并进入 degraded，不能按空库自举。node_did 不符拒绝打开，不能把其他节点的备份当本机账本。
- login/describe 返回 store_id，descriptor、客户端 locator、所有操作与 query/cancel 都绑定它。旧账本请求返回 StoreMismatch，客户端禁止自动接受新 store_id 重投旧调用。server_epoch 随 Executor 重启变化，store_id 不随正常重启变化。
- 结果默认保留 7 天；GC 先持久发布墓碑，再删除正文/大结果，查询返回 ResultExpired 与原终态。同一 store_id 下不按 TTL/FIFO 丢弃墓碑，最多 100000 条操作/墓碑、总容量默认 2 GiB；这些 limits 的工程默认值可由管理员调整，但不能通过降低上限隐式丢弃去重事实。
- 准入前预留请求、输出、文件结果和终态空间，临时上传也计入总预算。满额返回 CapacityExceeded、停止新准入；磁盘写失败时继续 drain 输出并有界取消活跃执行，保留已有可验证记录。无法持久提交成功结果时不得报告成功，缺终态证据的操作按 unknown 处理。
- P2 的 posture/recovery 状态不随 Command Stream 账本重置自动清零；恢复信任根与 seq 在重装/重置时的保留或重新激活契约须在 P2 定稿。初始化新的 store_id 不是接受旧恢复脚本的依据。

---

## 12. 配置与接线清单

| 位置 | 首期及既定后续阶段 | Thunk 后置项（§6.2 完成后再定） |
| --- | --- | --- |
| `buckyos-api/src/control_panel.rs` | `NodeConfig` 增加可选 `executor: Option<ExecutorConfig>`；缺省无 | `config.thunks` 与 `ThunkDispatch` 的 schema |
| `buckyos-api` 新文件 | `ExecutorConfig`/settings、Command Stream 帧 DTO；不引用 agent_tool/libopendan | FunctionObject / Thunk 的完整执行协议 |
| scheduler | 统一授权快照派生、generation/续期、精确 RBAC 与事务发布；覆盖 Agent binding/enabled/permission_mode 和设备 ACL 变化 | Thunk 提交授权、唯一节点归属、dispatch backend、状态回收 |
| 新 crate `src/kernel/node_executor` | admission、stream server、worker；P2 增加恢复 posture/recovery；不依赖 libopendan | thunk runners，与已有执行组件的复用边界 |
| node_daemon | 薄装配：reconcile 时交主体/profile 授权快照给 Executor；刷新/过期、stream listener、worker 子命令入口；修全局 waitpid | Thunk 配置收敛与执行状态回报 |
| workflow / TaskMgr | 无本次 Thunk 接线要求 | TaskId 协议、runner 归属、epoch/revision、结果与取消链路 |
| cyfs-gateway 配置及接线 | process-chain 消费设备 ACL/快照有效期，在转发前判定；撤权时关闭受影响 executor 转发；本机 `executor` → `127.0.0.1:<port>` 路由及 Agent 侧入口。不向 Executor 转交 peer 身份 | — |
| agent_tool / libopendan | `remote_node` kind、`NodeCommandHandle`、文件工具代理、locator、recovery reader、factory | — |
| verify-hub | Agent-backed App 签发/refresh 校验 binding 与启用状态；不改变普通 App/System/Device 的既有登录要求 | — |
| control-panel / WebUI | `settings.executor`、`devices/<name>/executor_config` 编辑；设备能力展示 | — |
| 安装/重置与管理员手册 | 明确 local/ 账本的保留与清空入口、store_id 变化、StoreMismatch 与容量处理；同步 buckyos_admin | — |

---

## 13. 实施分期与验收

每阶段默认关闭；不提前暴露只有传输没有授权/去重的执行接口。P0 聚焦来源②的 Linux 形态 A + hosted xllm，配置权限是开放前置条件，descriptor/locator/基本恢复查询属于同一个首期交付。来源③仍在 P2；来源①整体后置，完成 §6.2 的设计条件后另行排期，不阻塞 P0/P1/P2。

| 阶段 | 内容 | 退出标准 |
| --- | --- | --- |
| P0-A 配置与权限 | §5.4–5.6、§12：统一快照、RBAC、严格 App→Agent 映射、verify-hub binding 校验、gateway process-chain 设备准入/撤销 | 控制面正常时撤权 15 秒内应用；30 秒租期失联到期停用；未授权设备零转发、主体零执行；完成后才开放 P0-B |
| P0-B Command Stream 最小闭环（形态 A，Linux） | §7、§11：RTCP、exec/read/write/edit、worker/waitpid、持久操作与 store_id；hosted xllm factory/descriptor/locator/CommandHandle/recovery reader | shell + 文件工具可用；断线重连可查可取消；Runner 崩溃恢复只查询不重放；账本重建拒绝旧调用；远端 PromptExec 返回 Capability |
| P1 轻量客户端（形态 B） | 桌面独立 Agent Daemon；`devices/<name>/executor_config`；上线/退出 | Jarvis 能在授权电脑上 read/edit 文件 |
| P2 恢复通道 | §8、§9：posture、声明发现、本地触发、等待期、签名脚本、恢复 Stream | 无头 OOD 在 Zone 不可达时可被 Owner 恢复 |
| P2 形态 C | 移动端嵌入；平台适配层 | 电视盒子配置场景 |
| 后续：Thunk（未排期） | 先定稿 §6.2 的 FunctionObject 执行契约与三路径分工，再重审 config.thunks、各类 runner、状态回报及 TaskMgr 接线 | 开工前完成契约评审；实现后的端到端标准据此制定，候选场景见下表 |
| 后续 | 远端 PromptExec（先完成持久 run/call 契约）、完整 Session 远端运行（TODO §9.4）、Operator、Windows/macOS worker、跨 Zone | — |

Command Stream、轻量客户端与恢复通道的验收场景（随各自阶段执行）：

| 场景 | 必须观察到 |
| --- | --- |
| 对端设备不在 allowed_devices | gateway process-chain 在转发前拒绝 executor Stream；Executor 未收到 login；其他业务 Tunnel 不因此被关闭 |
| 设备允许但 Agent 未授权 / 普通 App 或 User token 直登 | Executor 拒绝；自报设备 DID/Agent DID 不能改变结果 |
| App token 的 owner/完整 binding 不符 / 同 runtime 多 Agent（含停用 Agent） | Executor 拒绝；普通 App/System/Device 的系统登录仍可用 |
| 同一 Tunnel 另一 Agent 开新 Stream | 不继承前一 Stream 身份与权限 |
| 只读 profile / Agent 全局 ReadOnly 配上可写 profile | write/edit/exec 均拒绝；profile 不能放宽 permission_mode |
| 同 op_id 重投 / 不同载荷 / 同 run/call 换 op_id | 同一调用不启动第二次 / Conflict / 拒绝第二操作；跨主体不可查询或接管 |
| Stream 断开时操作运行中 / RTCP 24 h key epoch 重建 | 操作继续；原主体重登后可按 byte offset 查询原结果，仍有权限且 handle 有效时可取消 |
| write 上传中断 / upload_end 后响应丢失 | 未完整接受时目标无变化；响应丢失先 query，已接受操作不自动重投；恢复 read 不重新读取目标 |
| starting/running/result 各窗口崩溃 | 有持久结果则恢复；否则 unknown，不重新派发，不按旧 PID 取消 |
| Runner 崩溃 / store 清空重建 / 普通覆盖升级 | locator 先于请求持久化；重建返回 StoreMismatch；覆盖升级保留 store_id/去重事实 |
| 并发槽满 / 大输出 / 墓碑容量满 / 磁盘写失败 | Busy 或 CapacityExceeded；输出有界且 query/cancel 可用；不删除去重事实换取继续执行，不虚报成功 |
| 子进程环境探针 | 无设备私钥/token/daemon env；文件工具 UID 与 shell 相同 |
| 密集短命令 + daemon reaper | 无 ECHILD、退出码丢失 |
| Agent/profile 撤权 / 禁用 / 改 run_as 或 binding | 节点应用快照时先阻止新启动，再关流/取消；控制面正常时提交后 15 秒内应用；保留结果 |
| 设备 ACL 撤销 | gateway 关闭该设备已有的 executor 转发且拒绝新流；Executor 不获知设备身份，已启动命令按断线契约继续 |
| 控制面失联 / 重读旧快照 / token 自然过期 | 租期不因缓存续命，30 秒有效期内到期关闭并取消；token 自然到期只关登录流，重新登录后恢复查询 |
| remote_node 的模板 PromptExec | 在命令或模型请求发出前返回 Capability，不能临时构造 call_id 执行 |
| 旧 seq 有效签名再次提供 | 拒绝执行 |
| 服务器给未授权的大 seq | 本地进度不推进 |
| 脚本已执行但上报中断 | 结果重报，脚本不重跑 |
| 有效声明 + 设备在范围 | 进入 Frozen：新准入关闭，业务服务不停；等待期内无恢复执行 |
| 声明已过期 / 设备尚未查询到 | 不冻结 |
| 本地触发 | 立即退出 Frozen 或立即进入 Recovery |
| Zone 正常可达 | 日常不依赖 Owner key，不触碰恢复入口 |
| 恢复 Stream 的设备不在恢复名单 | gateway process-chain 拒绝；Executor 不自行判断设备身份 |
| App 版本权限低 | 同一协议下只开放宿主支持的能力 |

Thunk 后续阶段的候选验收场景（不纳入 P0/P1/P2）：

| 场景 | 待契约定稿后验证 |
| --- | --- |
| FunctionObject 解析并执行 | 定义内容与 fun_id 一致；实际环境、参数及结果符合已定稿契约 |
| 重复派发 / 回执丢失后重新选节点 / daemon 重启 | 全局归属与节点去重共同阻止重复执行；unknown 不自动重跑 |
| OPTask 超时 / 取消 | 确认实际进程状态，并由合法 runner 向 TaskMgr 提交结果 |
| workflow `func::` 端到端 | 提交授权、派发、执行、结果及任务归属形成完整闭环 |

验证命令（实施阶段，在 `src/` 下）：

```bash
cargo test -p node_executor
cargo test -p node_daemon -- --test-threads=1
cargo test -p scheduler -- --test-threads=1
cargo test -p buckyos-api -- --test-threads=1
cargo test -p agent_tool --lib
cargo test -p libopendan -- --test-threads=1
cargo check --workspace
```

---

## 14. 决策项

合并了 TODO 的待定项、讨论稿 §11、2026-10-14/10-18 两次 review 及本次开工评审。2026-10-21 已确认首期的六项裁决；设备准入按用户裁决完全放在 gateway process-chain。已定项是实施约束，后续阶段的待定项不阻塞 P0。超时、容量等注明的工程默认值用于实现与验收，不表示既有代码已支持。

| # | 决策 | 选项 | 结论 / 待定建议 | 状态 |
| --- | --- | --- | --- | --- |
| D-1 | v1 真实消费方 | a. 独立 xllm（hosted）；b. OpenDAN Session；c. Workflow `func::` | a，Linux 形态 A；最小恢复一并交付；c 与完整 Session 后置 | 已定 |
| D-2 | Agent 流登录凭据 | a. 现有 App session token；b. Agent 专属 IdentityRoots + token provider | a；完整 AppInstance 必须只有一个现存 Agent 绑定，且该 Agent enabled、同 DID；拒绝共享 runtime 和其他 token kind，保留原 claims 做 RBAC（§7.2） | 已定 |
| D-3 | Command Stream 传输及设备准入 | a. RTCP Stream + gateway；b. S2S kRPC over HTTP 3181 | a；gateway process-chain 执行设备 ACL，Executor 无 remote device 身份、不接收 peer context | 已定 |
| D-4 | 离线持久队列 | a. 无；b. TODO 的 spool；c. Task Dispatch Center | a；无执行槽直接 Busy；保留持久操作账本，需要离线交接时走 c | 已定 |
| D-5 | Executor 核心代码位置 | a. 新 crate `kernel/node_executor`，node_daemon 与 Agent Daemon 共用；b. `libopendan::node_agent_runtime` | a，Executor 不依赖 libopendan | 已定 |
| D-6 | `Device` 类设备的配置位置 | a. `devices/<name>/executor_config`（control-panel 写）；b. 让 scheduler 也为 Device 生成 nodes/ 条目 | a | 待定 |
| D-7 | Thunk 结果回报通道 | a. `nodes/<node>/thunk_state` + scheduler 收敛；b. 节点直接 kRPC 调 scheduler；c. 节点直接写 TaskMgr | a 为候选；随 FunctionObject 结果契约和 TaskMgr runner 归属一起重审 | 后置待定 |
| D-8 | 只读 Agent 的 exec | a. 不开放 exec；b. 命令白名单；c. 开放但记录 | a；全局 permission_mode 与 profile 取交集，ReadOnly 禁 exec/write/edit，首期无白名单例外 | 已定 |
| D-9 | 恢复脚本签名密钥 | a. Owner Root Key；b. 激活时固化的独立恢复签名 key（公钥进 node_identity） | b（符合 Root Key 不入日常；泄露面分离） | 待定 |
| D-10 | §8.4 签名字段/作用域/失败是否消耗序号 | — | 定稿时随 D-9 一起定 | 待定 |
| D-11 | 等待期细则 | 起算点、可信时间源、重启处理、哪些修改重新等待、本地触发是否可跳过 | 进入 Frozen 起算；持久化；端点迁移不重算、scope/主体变更重算；本地触发可跳过 | 待定 |
| D-12 | Frozen 是否也响应"Zone 可达但权限被盗" | a. 只在不可达时；b. 在线设备也响应止损声明 | a（v1）；b 另立设计 | 待定 |
| D-13 | 私钥泄露后的撤销/替换 | 依赖 BNS/OwnerDocument 轮换（Reset & Unbind §8.2） | 不在本文解决，只留接口：恢复信任根可被新 OwnerDocument 替换 | 待定 |
| D-14 | 轻量客户端的 Gateway 复用 | 已有独立 gateway 的设备如何复用；多隔离 App 的协议栈实例与设备身份如何协调 | 一台设备一个设备身份；多 App 共享本机 gateway，嵌入式仅在无 gateway 时启用 | 待定 |
| D-15 | 已登录 Stream 遇到撤权/执行态变化 | a. 关流并取消；b. 允许收尾 | 主体/profile 撤权、快照过期或执行态收紧用 a；设备 ACL 由 gateway 关流，token 自然到期也只关流，二者按断线契约允许已接受操作收尾 | 已定 |
| D-16 | `Script` 的执行环境 | 复用 Command Stream worker、aios 容器或独立环境 | 先定义语言、依赖与权限契约，再比较；原 aios 方案仅作候选 | 后置待定 |
| D-17 | Operator 类型 | 暂不支持 | 维持 | 已定 |
| D-18 | 授权刷新与失联 | 无期限旧配置 / 有效期内快照 | 统一授权快照、hash + 单调 generation；5 秒权威轮询、15 秒正常传播上限、30 秒租期为 P0 工程默认值（§5.6） | 已定 |
| D-19 | 操作归属与崩溃恢复 | Stream 持有 / Executor 持久操作 | Executor 归属 actor/profile/runtime/run/op；跨 Stream query/cancel；starting 先持久化；无终态证据则 unknown，不重放（§7.3–7.4） | 已定 |
| D-20 | 账本身份与重建 | 仅进程 epoch / 持久 store_id | store_id 绑定 descriptor/locator/请求；覆盖升级保留，重建新 ID，旧调用 StoreMismatch；墓碑不自动清除，容量满拒绝（§11） | 已定 |
| D-21 | 首期恢复与 PromptExec | 恢复后置 / 与 hosted xllm 同期 | descriptor/locator/recovery reader 纳入 P0；远端 PromptExec 首期 Capability，持久 run/call 契约完成后另行开放 | 已定 |

---

## 15. 已知风险（按"先能力后安全"原则登记，不阻塞 v1）

- 当前代码允许共享 runtime；首期 Executor 明确拒绝，只接受唯一同 DID 绑定。开放共享 runtime 或原生 Agent token 需单独定义凭据与操作归属，不能放宽校验直接接入。
- roots 不是 sandbox；同一 profile 的多个 Agent 共享一个 OS 权限域。
- 设备 ACL 只约束 gateway 的远端入口；宿主进程仍处于本机 OS 信任边界。撤设备准入关闭连接不等于取消已接受操作，需停止操作时撤销 Agent/profile 授权。
- 正常授权采用有界传播和短租期，控制面不可用会暂停执行入口并取消活跃操作；断线可恢复不代表执行可以永久脱离授权控制面。
- Frozen 的 DoS 面（§9.4）；等待期不能区分合法用户与持同一 key 的攻击者（§9.5）。
- 形态 C 的后台运行、权限与各平台能力未验证；cyfs-gateway-lib 在移动端的可嵌入性未验证。
- 命令与文件执行仅支持 Linux root daemon；非 root Linux、Windows/macOS 保留 `open_stream` / `describe` / `query` / `cancel` 控制能力，执行、文件操作及带 workdir 的 describe 返回 Capability。Linux worker 的降权、进程组与 `/proc/self/fd` 实现按目标平台隔离编译。
- Windows/macOS 的 worker 降权未设计。
- 恢复端点的结果真实性在 D-10 定稿前只依赖 HTTPS。

---

## 16. 与来源文档的关系

- 本文发布后，`notepads/node-agent-runtime-todo.md` 与 `notepads/BuckyOS Node Executor设计思路.md` 降为历史输入；与本文冲突处以本文为准。本次已将 TODO §6–§8 中仍适用的操作归属、持久 starting、崩溃窗口、输出游标、store_id 和墓碑规则纳入 §7/§11；不沿用 S2S peer 绑定、离线 spool/queued 恢复或 node_daemon 依赖 libopendan 的旧方案。CodeAgent 固化 wire 编码与 DTO 时遵循本次裁决，不能重新选择这些语义。
- Thunk 的后置范围与 `doc/workflow/executor list.md` 的后续阶段方向一致；本文 §6.3–6.6 保留旧执行器方案作比较输入，待 §6.2 定稿后更新，不作为首期实施契约。
- `BuckyOS 系统故障的诊断和恢复(beta3).md` §10.3 的第二诊断链路由本文 §9 具体化；该文 §11 的 Agent 修复前置条件对本文来源①的 OPTask 与来源②的修复类命令同样适用。
- `Reset & Unbind.md` §2.3 的物理控制权要求对应本文 §9.1 本地触发；§7.1 对远程擦除的保守态度同样适用于远程恢复脚本的能力边界。

---

## 17. P0 实施记录（2026-10-21）

P0-A（配置与权限）与 P0-B（Command Stream 最小闭环，Linux 形态 A + hosted xllm）的代码已完成，单元/集成测试通过，并已在单 OOD 的 DV Zone 验证（17.7）。P1/P2 依赖的 D-6、D-9～D-13 仍待定，Thunk 按 §6 后置，均未实施。

### 17.1 代码落点

| 位置 | 内容 |
| --- | --- |
| `buckyos-api/src/node_executor.rs` | `NodeSettings{executor}`、`ExecutorSettings`、`ExecutorConfig`（revision/generation/auth_valid_until_ms/principal_bindings/profiles/limits）、`ExecutorOp`、`granted_ops`（profile.ops ∩ permission_mode）、端口与租期常量；`NodeConfig.executor: Option<ExecutorConfig>` |
| `buckyos-api/src/node_executor_proto.rs` | 帧编解码、全部控制消息 DTO、错误枚举、`ExecutorOpSpec` 与请求摘要 |
| `buckyos-api/src/agent_permissions.rs` | `check_agent_runtime_session`（verify-hub 用） |
| `kernel/node_executor`（新 crate，不依赖 libopendan） | `store`（账本）、`auth`（快照租期、§7.2 映射、RBAC 收窄）、`ops`（准入/去重/槽位/串行控制门/执行/终态发布）、`spawn` + `worker`（降权子进程）、`stream`（每 Stream 协议处理）、`lib`（`NodeExecutor`：apply_config、撤权/过期收敛、listener）；`src/bin/node_executor_worker.rs` 仅供测试 |
| `kernel/scheduler/src/executor_auth.rs` | 快照派生、generation/续租、执行器 RBAC 规则；接入 `build_schedule_plan` 与 `update_rbac` |
| `kernel/node_daemon/src/node_executor_host.rs` | 形态 A 装配：verify-hub token 校验与 RBAC 适配、5 s 拉取快照、首次出现快照时打开账本与 `127.0.0.1:3184` 监听、gateway ACL 文件、撤销时调用 gateway `close_connections`；`main.rs` 的 `--executor-worker` 入口；`service_pkg.rs` 用按 pid 的 `reap_detached_children` 取代全局 `waitpid(-1)` |
| `kernel/verify_hub/src/main.rs` | App principal 的 login_by_jwt 与 refresh 调 `check_agent_runtime_session` |
| `rootfs/etc/boot_gateway.yaml`、`rootfs/etc/node_executor_acl.json` | `EXECUTOR_ACL` collection、`node_rtcp` 的 `node_executor` 守卫块、`node_executor_entry`（0.0.0.0:3185）+ `node_executor_socks` server；ACL 模板列入 `bucky_project.yaml` data_paths |
| `frame/agent_tool/src/runtime/node.rs` | `remote_node` 运行时：`NodeExecutorConnector`（宿主注入）、`NodeClient`、`NodeBashRunner`/`NodeCommandHandle`、`RemoteNodeFiles`、locator、`NodeRecovery`（describe_interrupted 与 task recovery reader）、`SocksNodeConnector`、`BuckyosNodeConnector` |
| agent_tool 其余 | `RuntimeConfig.remote_node`、`RuntimeRegistry::from_config_with`、`XllmDeps.node_executor`、`FileBackend::{write_whole, edit_once}`、`RuntimeRecoveryReader`、shell 说明文本；xllm CLI 默认注入 `BuckyosNodeConnector` |
| libopendan / opendan | Session 选 `remote_node` 时在绑定前返回 Capability；`zone_xllm_deps` 注入 `SocksNodeConnector` |
| cyfs-gateway 仓库 | `ConnectionInfo` 记录 RTCP 对端设备与目标端口，`ConnectionManager::stop_connections_where`，控制命令 `close_connections{dest_port, keep_source_devices}` |

### 17.2 授权快照（§5.4–5.6 的实现取舍）

- 管理员输入是 `nodes/<node>/settings` 的 `executor` 字段（键原先不存在，本次新增 `NodeSettings`）；未知字段拒绝。scheduler 每轮（约 5 s）从同一份 dump 派生：`allowed_devices` 只保留 `devices/*/info` 中存在的设备 DID；profile 里的每个 principal 必须恰好对应一个 AgentSpec，且该 AgentSpec 的 runtime 实例在全部 AgentSpec（含停用的）中只被引用一次、AppDID == AgentDID，否则不进 `principal_bindings`；`enabled` 来自 AgentSettings（缺省 true），`app_installed` 来自 App spec 的安装状态。
- revision = 规范化派生结果的 sha256；revision 变化时 generation+1、租期 = now+30 s；revision 不变且剩余不足 20 s 时只更新截止时间；settings 删除或失效时发布一次 `enabled=false` 的新 generation。generation 变化会触发同事务内的 RBAC 重算。
- **与设计的差异**：`exec_tx` 只支持单键 revision 检查，未实现"输入版本未变"的多键校验；改为同一份 dump 派生并在同一事务发布，dump 与提交之间的 settings 变更在下一轮（≤5 s）生效。
- RBAC：对 `obj://node/<node>/executor/<profile>` 生成 `p, <owner>, …, ^executor\.(…)$, allow` 与 `p, app:<AppId>, …` 两条，动作只含 granted_ops；规则插在只读 App 的 `obj://*` deny 之前，使只读 Agent 的 read/query 仍可用、exec/write/edit 被拒（测试用真实 casbin 模型验证）。
- 节点侧：`AuthState` 只接受更大 generation 或同 generation/revision 的更晚截止时间；剩余时间按收到时计算并封顶 30 s，之后用单调时钟；250 ms 看门狗在到期时关闭全部 Stream、取消活跃操作。新 generation 到达时逐个重算 Stream 授权：主体/profile/run_as/roots/env/binding 变化或权限缩小即关闭（Revoked），并取消不再被授权的活跃操作；结果按实际退出发布。

### 17.3 Command Stream wire（固化）

- 帧：`kind:u8 | len:u32be | payload`；kind 1 为 JSON 控制帧（≤1 MiB），kind 2 为数据帧 `channel:u8 | offset:u64be | op_id_len:u16be | op_id | bytes`（数据 ≤16 KiB）。**差异**：设计写"单帧 ≤16 KiB"，控制帧放宽到 1 MiB，以容纳 exec 脚本与 edit 文本。
- 客户端消息 `login / describe / exec / read / write / upload_end / edit / cancel / query / ping`，服务端消息 `login_ok / describe_ok / accepted / status / query_ok / started / finished / error / closing / pong`，均为 `type` 标签、拒绝未知字段。错误码：Unauthenticated、Forbidden、InvalidArgument、Capability、Conflict、Busy、CapacityExceeded、Disabled、Unavailable、AuthExpired、ResultExpired、StoreMismatch、NotFound、ProtocolError、Internal，并带 `effect_unknown`。
- login 绑定 runtime_id 与 run_id；操作必须带与 login 相同的 run_id 和当前 store_id（否则 InvalidArgument / StoreMismatch）。摘要覆盖 runtime/run/call/op 与 `ExecutorOpSpec`（可选字段只补协议默认值，不补随配置变化的值）。
- write：`write` 头 → upload 数据帧（offset 连续）→ `upload_end`，回复挂在 write 的 request_id 上；30 s 上传期限，断线/超时丢弃临时内容。
- query 的数据以 base64 内联返回（每次 ≤256 KiB，`wait_ms` ≤30 s）；exec 的 started/输出/finished 只推送给提交它的 Stream，尽力而为，队列满即丢，权威结果以 query 为准。

### 17.4 执行与账本

- 账本 `$BUCKYOS_ROOT/local/node_daemon/executor/`：`meta.json`、`owner.lock`（flock 单写者）、`operations/<key-hash>/{request.json,state.json,result.json,stdout,stderr,content,upload}`、`tombstones/<key-hash>.json`、`tmp/`。key-hash = sha256(`["op", store_id, agent_did, app_instance_id, profile, runtime_id, op_id]`)，另以 run/call 元组做第二唯一键。记录在 tmp 下写全并 fsync 后整目录 rename 发布，之后才回复 accepted；result.json 写一次。重启时 starting/running 而无结果的记 unknown（effect_unknown），meta 缺失但有记录、node_did 不符、记录损坏均拒绝打开（节点日志报错，执行器保持关闭）。
- exec：先以目标身份跑一次 worker 解析 cwd（须在 roots 内），再在串行控制门内检查取消、直接 spawn `bash --noprofile --norc -c <wrapper>`（std 做 setgroups/setgid/setuid，pre_exec 设 no_new_privs、清 ambient caps、`close_range(CLOEXEC)`，独立进程组），脚本经 stdin 读入后 stdin 改为 /dev/null，不进 argv。**差异**：shell 进程本身就是降权 worker，没有再套一层持有 shell 的 worker。stdout/stderr 各保留前 N 字节、超出继续 drain；leader 退出后最多再等 500 ms 读管道；超时/取消对进程组 SIGTERM，2 s 后 SIGKILL，以 wait 确认后才发布 timed_out/cancelled；正常结束不清理后台进程。
- 文件操作：node_daemon 以 `--executor-worker` 重入自身、以 profile 身份执行；路径在打开后的 fd（`/proc/self/fd`）上校验是否在 roots 内；写入为同目录临时文件 + fsync + renameat + 目录 fsync；`write` 可创建 roots 内缺失的父目录；同一 canonical 路径的写串行；expected_hash 不符为 Conflict。read/edit 的内容快照写入 `content`，query 读取的是快照而非现场文件。文件操作一旦开始不可中断，取消请求之后按自然结果发布。
- 容量：准入前按操作预留空间（exec 为 2×输出上限，文件为 max_file_bytes），计入 max_records（含墓碑）与 max_store_bytes，超出 CapacityExceeded；GC 每 60 s 先写墓碑再删目录，墓碑不按 TTL 删除。
- 非 root 的 daemon 只声明 open_stream/describe/query/cancel，exec/read/write/edit 返回 Capability。

### 17.5 gateway 与 Agent 侧

- 设备准入在 `node_rtcp` 的 `node_executor` 块（priority 0）：目标端口 3184 时要求 `REQ.source_device_id` 在 `EXECUTOR_ACL` 中，否则 reject，通过则固定 `forward tcp:///127.0.0.1:3184`；其他端口仍走原 default 块。**差异**：ACL 不由 scheduler 写进 gateway_config，而由 node_daemon 从已交付的同一快照（同 revision）写 `etc/node_executor_acl.json`（json_map，每次访问重读）；快照过期、停用或缺失时写空表，从而在控制面不可达时也能本地收紧。设备从 ACL 移除或快照失效时，node_daemon 调 gateway `close_connections` 关闭不在新名单中的 3184 转发（需要带本次改动的 cyfs-gateway；旧 gateway 返回未知命令时仅记日志）。
- Agent 入口：gateway 新增 `0.0.0.0:3185` 的 TCP stack，只接受来源 `127.*` 与 `172.*`（容器网桥）的连接，交给 SOCKS5 server：只允许目标端口 3184；目标为 `127.0.0.1` 时 DIRECT（本机执行器），否则 `PROXY rtcp://<node-host>/`。`SocksNodeConnector` 据 runtime 的 `device_config` 判断目标是否本机，token 用 runtime 当前的 App session token。
- `remote_node`：配置 `{kind: remote_node, workdir, remote_node: {node_did, profile_id}}`，未注入 connector 时 Capability；workdir 由节点解析（descriptor 记 node_did/profile/uid/gid/canonical workdir/store_id/协议版本）。每次工具调用 = 一个远端操作：发送前在 `<run_dir>/runtime_operations/<call>.json` fsync 保存 locator（同 call 重入复用 op_id，内容不同则拒绝）；read_file/read 是一次远端 read，write_file/edit_file 走 `FileBackend::write_whole/edit_once` 在节点上整体执行；shell 的 auto 转 task 后，新进程经 `RuntimeRecoveryReader` 按 locator 只读查询。`start` 用节点 `max_execution_ms`，`run`（同步/模板）用请求超时并在到期时 cancel。无 call_id 的模板 `__EXEC__` 返回 Capability；libopendan Session 选 `remote_node` 在绑定前返回 Capability。

### 17.6 验证与未完成项

已执行（均通过）：`cargo test -p node_executor`（store 2 + worker 1 + 端到端 8：登录映射、降权与环境、去重与 Conflict、断线续查与跨 Stream 取消、文件工具与 symlink 越界、只读与槽位、撤权、快照过期与 token 到期、上传中断；以及 agent_tool `remote_node` 运行时对真实执行器的 1 个端到端用例）、`cargo test -p scheduler executor_auth`（3）、`cargo test -p buckyos-api node_executor agent_runtime_sessions`、`cargo test -p node_daemon detached_reaper`、`cargo test -p agent_tool --lib`、`cargo test -p libopendan --lib sessions_refuse_remote_node`、cyfs-gateway `test_node_executor_guard_block` 与 `stop_connections_where_closes_only_selected_peers`。执行器测试需要 root（worker 降到 uid 65534）。

未执行 / 未完成：
- control-panel/WebUI 编辑 `settings.executor`、设备能力展示；`devices/<name>/executor_config`（D-6）与形态 B/C；恢复通道（P2）；Thunk。
- cyfs-gateway 改动仅在本地工作区，未提交、未推送；BuckyOS 侧以原始 kRPC 方法名调用，不依赖新的 cyfs-gateway-api 版本。

### 17.7 DV 验证（2026-10-21，单 OOD Zone `test.buckyos.io`）

部署 release 构建的 node_daemon、scheduler、verify-hub、cyfs-gateway 与新 `boot_gateway.yaml`，`start.py --skip-update` 重启（原文件备份在 `/opt/buckyos-backup-node-executor-20261021-085421/`）。主体为当时唯一的 Agent `r03share.test.buckyos.io`（owner bob），profile `dv` 以 uid 65534 在 `/opt/buckyos-dv-executor/ws` 执行。驱动工具：`cargo build -p node_executor --example dv_probe`（经 verify-hub 铸造真实 App token，直连或经 SOCKS 入口执行）。

| 场景 | 观察结果 |
| --- | --- |
| 部署后 Zone 与 Agent | check.py 全部 OK；gateway 加载新块并监听 3185；r03share 容器经新 verify-hub 校验登录成功 |
| 写入 settings | 约 6 s 后出现 generation 1、30 s 租期、`principal_bindings` 含 r03share；RBAC 尾部出现 owner 与 `app:` 两条规则；ACL 文件 revision 与快照一致；node_daemon 打开账本、监听 `127.0.0.1:3184` |
| 真实 App token 直连 | login_ok、describe（canonical workdir、uid/gid、store_id）、exec 以 nobody 运行，环境只有 profile 变量与 HOME/USER/LOGNAME，无 BUCKYOS_*/token；新文件属主 nobody |
| SOCKS 入口（本机 DIRECT） | `CONNECT 127.0.0.1:3184` 经 gateway 到达执行器；`cyfs_gateway reload` 热加载修正后的规则 |
| Agent 容器内（来源 172.17.0.3） | `host.docker.internal:3185` → DIRECT → login_ok 为 r03share，命令以 nobody 执行 |
| hosted xllm（宿主机 `agent_tool xllm` + mock LLM，运行时 `remote_node`，真实 App token） | shell 工具在节点执行，结果回到模型；`runtime_operations/<call>.json` 在发送前落盘且 store_id 与节点账本一致 |
| 乱码 token / 非 Agent App（buckyos-systest）的 token | unauthenticated / forbidden |
| 运行中撤销主体 | settings 提交后 4.7 s 关闭 Stream；`sleep 90` 被 SIGTERM，账本终态 `cancelled (Revoked)`，保留已产生的 stdout；generation 2、执行器 RBAC 规则同事务删除 |
| 停用 | generation 3 `enabled=false`；ACL 置空；node_daemon 调用 gateway `close_connections` 返回 `{"closed":0}`；登录返回 unavailable |
| 带在途操作重启 Zone | store_id 不变；在途操作记 `unknown`（effect_unknown），不重跑；遗留的 `sleep` 进程未被追杀 |

DV 中发现并已修正：SOCKS hook 对 IP 目标只有 `REQ.target.type = "ip"` 与 `REQ.target.ip`，没有 `host`，原规则因此把本机目标 REJECT；已改为按 `type`/`ip` 判断。

未覆盖：
- 跨节点 RTCP 路径：单 OOD Zone 没有第二台设备；以节点名经 RTCP 连回本机失败（gateway 报 `resolve remote device did:web:ood1… canonical did:dev failed`），同节点按设计走 DIRECT。`node_executor` 守卫块只在 cyfs-gateway 单测里用真实 process-chain 引擎验证过，`close_connections` 关闭存活 RTCP 转发也只有单测（DV 调用时没有存活的转发）。
- 快照租期在控制面失联时到期（单测覆盖，未在 DV 停 scheduler 验证）；ReadOnly Agent 的 DV（未改动现有 Agent 的 permission_mode）。
- 验证结束后已把 `nodes/ood1/settings.executor` 置为 `enabled=false`（快照 generation 5、ACL 为空）；新二进制保持部署。重新启用只需把 `enabled` 改回 true。
