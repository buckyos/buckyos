# Node Executor 设计

> 版本：v1.1-draft，2026-10-20
> 状态：设计稿。本文合并了三份来源，统一成一个组件设计；"当前代码事实"与"本文新增设计"在各节分开标注，代码事实以源码为准。
> 本次范围调整：Thunk 执行器整体后置；先完成 FunctionObject 执行契约及其与 Command Stream、恢复脚本的能力对比，再单独排期。首期聚焦 Command Stream。
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
| `src/kernel/node_daemon/src/app_loader.rs` | 把 `$BUCKYOS_ROOT/storage/` 以 rw 挂进 App/Agent 容器，容器内为 root | **Executor 的私有状态不能放 storage/** |
| `src/kernel/node_daemon/src/run_item.rs` | `RunItemControl{deploy, start, stop, get_state}` 与 `ensure_run_item_state` | Thunk 的"一个实例"语义可复用其状态观测方式，但 thunk 不是长驻服务 |
| `src/kernel/node_daemon/src/kevent_server.rs` | daemon 已托管 `/kapi/kevent` HTTP | 可复用 listener，不复用其信任假设 |
| `src/frame/agent_tool/src/runtime/mod.rs` | `AgentRuntime` 有 descriptor/open/describe_interrupted；registry 只构造 native/tmux/remote_ssh | `remote_node` 作为新 kind，Node 实现由宿主注入 |
| `src/frame/agent_tool/src/llm_bash.rs`、`tasks.rs` | `BashRunner::start` → `CommandHandle{wait, progress, kill, detach}`，auto shell 走进程内 task manager | Command Stream 客户端要实现远端 CommandHandle |
| `src/kernel/node_daemon/src/boot.rs`、`doc/arch/02_boot_and_activation.md` | 设备在 `node_identity` 里保存激活时的 owner key；Zone 不可达时从 DNS TXT 取 `ZoneBootConfig`（含 `owner`/`owner_key`/`extra_info`）并用该 key 验签 | 恢复声明的发现与验证机制**已存在**，本文只定义声明内容 |
| `cyfs-gateway/doc/rtcp/rtcp.md`、`rtcp rely.md` | Tunnel 上多路 Stream，每流独立 AEAD key；透明中转保持端到端点对点语义；tunnel key epoch 24 h 后需重建 | Command Stream 直接跑在 RTCP Stream 上；中转不需要在应用层复制流 |
| `BuckyOS Security Arch.md` §4.4 | Client Device 用 Administrator 签名的设备描述符证明身份 | 轻量客户端的设备身份已有路径 |
| `verify-hub`、`07_identity_and_rbac.md`；`notepads` agent-init 结论 | AppDID == AgentDID，App session token 即标识 Agent；verify-hub 对非 User 主体尚未校验 AgentSpec.binding | v1 流登录可直接用 App token；binding 校验是前置缺陷 |
| `src/frame/llm_context/src/prompt_engine.rs` | 模板 `__EXEC__` 默认 `allow_exec=false` | 生产未开启；远端 EXEC 需显式 call_id（§7.6） |

---

## 3. 设计原则

1. **不为 Agent 远程访问另造安全体系。** 身份用 DID/DeviceDocument/verify-hub token，信道用 RTCP，配置用 system-config + scheduler，权限用 RBAC + Node Config。
2. **连接身份、登录身份、操作权限三层分开。** 可信设备能建信道 ≠ 任意 Agent 能登录 ≠ 登录后能做任何事。免重复认证 ≠ 免操作授权检查。
3. **Node Config 是正常运行下唯一的授权事实来源。** 信任的是经既有可信分发交付的 scheduler 配置，不是任意同名 JSON。
4. **Owner Root Key 不进入日常运行依赖。** 它只在恢复态作为最终授权根（Security Arch §4.1）。
5. **最终可执行范围 ⊆ Node Config 授权 ∩ 宿主 OS 实际授予 ∩ Executor/适配层已实现。** 接口少不等于权限低；只读 Agent 的 Execute 不得产生与只读矛盾的效果。
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
设备级：对端设备公钥是否在准入范围内？              ← RTCP tunnel 认证 + Node Config.allowed_devices
身份级：这个 User/Agent 能否登录此 Stream？          ← 流首帧的 session token + AgentSpec/binding
操作级：已登录主体被允许执行哪些操作？                ← Node Config.profiles[*].ops / roots / run_as
```

中转设备只是中转者，其身份不能替代真实 Agent 的身份。一条 Tunnel 上的新 Stream 不继承前一个 Stream 的登录。

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
    "revision": "sha256:<normalized-settings>",
    "allowed_devices": ["<ood-did>", "<laptop-did>"],
    "profiles": {
      "owner-dev": {
        "run_as": { "uid": 1001, "gid": 1001 },
        "workdir_roots": ["/home/owner/workspaces"],
        "env": { "PATH": "/usr/local/bin:/usr/bin:/bin", "LANG": "C.UTF-8" },
        "principals": ["<agent-did>"],
        "ops": ["exec", "read", "write", "edit"]
      }
    },
    "limits": {
      "max_streams": 4, "max_concurrency": 2,
      "max_execution_ms": 3600000, "max_output_bytes_per_stream": 8388608,
      "max_file_bytes": 1048576
    }
  }
}
```

规则：
- 缺省关闭。开启要求至少一个完整 profile；`principals` 逐个匹配权威 Agent/User DID，不接受通配。
- `run_as`、roots、env 是管理员授权上限，请求不能自选 UID、shell、结果目录或扩大 roots。
- 形态 A 的 worker UID 非 0 且不同于 daemon；非 root daemon 返回 Capability，不以同 UID 执行。形态 C 的 `run_as` 无意义，由宿主 App 权限决定。
- roots 约束 cwd 和内置文件工具，**不约束 shell 内任意程序的 I/O**；OS 账号权限才是实际边界。不把 roots 描述成 sandbox。
- 未知字段、空白名单、不支持平台、非法 uid/gid 均拒绝启用；旧 NodeConfig 无该段时行为不变。

RBAC 派生：profile 授权的主体对本 Node 的 `executor.open_stream/exec/read/write/edit/cancel/query` 放行；删除授权、改 binding 或停用时撤销。设置管理权限与 Agent 调用权限分别定义；不给 App 侧读 Agent 私钥的权限。

### 5.5 授权与宿主权限取交集

Executor 是系统服务时可具有被授予的系统能力；嵌入普通 App 时不能超出 App 权限。厂商预装版、高权限版与普通安装版使用同一协议，但对外声明的能力不同（通过 §4.3 的 `host`）。

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
- Executor 在形态 A/B 监听 `127.0.0.1`，由 gateway 接入；形态 C 在进程内交付 Stream，不开本地端口。回环监听只解决入口暴露，不代替登录与授权。
- Agent 侧通过本机 cyfs-gateway 的 stream 转发（socks 或等价入口，`cyfs-gateway/doc/rtcp/rtcp_socks_proxy_example.md`）到达 `<node-did>:executor`；具体入口形式见 D-3。
- RTCP tunnel key epoch 24 h 后必须重建连接：长会话要能在新 Tunnel 上**重新打开 Stream 并重新登录**，不丢在途命令（见 7.4）。
- 不开明文 `/kapi/` 入口；不做 LAN HTTP fallback。

### 7.2 每个 Stream 一次登录

首帧 `login`：

```json
{ "v": 1, "type": "login", "token": "<verify-hub session token>", "profile": "owner-dev", "client": { "runtime_id": "node-dev", "run_id": "..." } }
```

Executor 检查：token 有效且非 bootstrap/refresh；`principal` 在 profile.principals 内；Agent 主体时核对 AgentSpec/AgentSettings.enabled 与 `binding.target_app_instance_id`；对端设备在 `allowed_devices`。通过后返回 `login_ok{ posture, caps, limits, server_epoch }`。

v1 凭据：AppDID == AgentDID，现有 App session token 即可标识 Agent（TODO §5.2 的 Agent 专属凭据交付推迟，见 D-2）。前置修复：verify-hub 对非 User 主体校验 AgentSpec.binding。

登录后流内不逐命令重复认证，但每个操作仍受 profile 约束；配置撤权时 Executor 主动关闭该主体的 Stream 并取消其活跃命令。

### 7.3 帧与四类操作

Stream 上是长度前缀的 JSON 控制帧 + 二进制数据帧（单帧 ≤ 16 KiB，与 RTCP 记录对齐）。

| 操作 | 请求 | 返回 | 约束 |
| --- | --- | --- | --- |
| `exec` | `{op_id, cmd, cwd, env_overrides, execution_timeout_ms}` | `started{pid}` → `stdout/stderr` 数据帧流 → `exited{code}` | cmd 作为数据经有界管道交给 shell，不进 argv；无交互 stdin（v1） |
| `read` | `{op_id, path, range}` | 数据帧流 + `done{hash}` | 节点侧解析 roots/symlink |
| `write` | `{op_id, path, expected_hash?}` + 数据帧 | `done{hash}` | 临时文件 + 原子替换；hash 不符 Conflict |
| `edit` | `{op_id, path, edits[], expected_hash?}` | `done{hash, diff_summary}` | 读、匹配、写在同一目标完成 |
| `cancel` | `{op_id}` | `cancelled` / `not_found` | 只针对本 Stream 持有的 handle |
| `query` | `{op_id}` | 当前状态 + 输出尾部 | 用于断线重连后的恢复 |

`op_id` 由客户端生成、按 `(run_id, call_id)` 稳定，Executor 持久去重：同 id 重投不重复执行，同 id 不同载荷 Conflict。

"只读 Agent"= profile.ops 不含 write/edit **且** exec 不可用或限于白名单命令（D-8）。接口数量不代表权限。

### 7.4 长命令、断线与恢复

- 沿用 agent_tool 的 auto/wait 与 `local:shell:<call_id>` 语义；`wait_ms` 只控制客户端等待，不停止命令；`max_execution_ms` 是节点硬上限。
- Stream 断开 ≠ 取消：已 `started` 的命令继续到结束或超时，输出按 `max_output_bytes_per_stream` 有界缓存在 Executor 本地目录，供重连后 `query` 读取。缓存保留期由 limits 给出。
- 客户端在 `<run_dir>/runtime_operations/<call-key>.json` 先持久 locator 再发请求；`describe_interrupted` 只查询，不重放、不杀旧 PID。
- 不做队列：目标离线时 `open_stream` 直接失败；需要离线持久交接的场景走 Task Dispatch Center，不在 Executor 里再造 spool。

### 7.5 worker 与权限边界（形态 A）

- 每个 `exec` 一个 worker 子进程：清 supplementary groups → setgid/setuid → 清 capabilities + no_new_privs → 清 env/fd；任一步失败即不执行。
- 文件 read/write/edit **也在 worker 身份下**完成，不在高权限 daemon 里做 I/O。
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
- descriptor 稳定 target：Node DID、profile、实际 UID/GID、canonical workdir、协议版本；恢复时任一变化即 RuntimeMismatch。
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
| 恢复 Command Stream | 按配置用 RTCP 连接指定设备并保持 Tunnel，再按 §7 登录；恢复配置列出允许的设备与主体 |

端点迁移不重新等待；改变 scope 或允许的主体需重新等待（D-11）。建立恢复 Tunnel 不自动降低执行态。

### 9.7 退出条件

- 本地触发可随时退出。
- 远程：恢复脚本显式 `exit_recovery`，或重新连上 Zone 且 scheduler 下发的 Node Config revision 新于进入 Frozen 时的 revision，且声明已撤销/过期。
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
  posture.json             执行态、恢复轮次、进入时间、等待截止
  recovery/seq.json        {round, local_seq, accepted[], exec_state{}, delivered{}}
  thunks/<thunk_obj_id>/   后续 Thunk 阶段候选目录，格式待 §6.2 定稿，首期不创建
  streams/<op_id>/         record.json（去重 + 状态）、stdout、stderr
```

- **不放 `storage/`**（app_loader 把 storage/ rw 挂进 root 容器）。
- 原子替换 + fsync 复用 `lib_opendan/src/fsutil.rs` 的做法（可下沉到 kit）；单写者 flock。
- 形态 B/C 使用平台等价的应用私有目录。
- 保留期与容量上限来自 limits；超限返回 ResultExpired/容量错误，不删除去重记录后重执行。

---

## 12. 配置与接线清单

| 位置 | 首期及既定后续阶段 | Thunk 后置项（§6.2 完成后再定） |
| --- | --- | --- |
| `buckyos-api/src/control_panel.rs` | `NodeConfig` 增加可选 `executor: Option<ExecutorConfig>`；缺省无 | `config.thunks` 与 `ThunkDispatch` 的 schema |
| `buckyos-api` 新文件 | `ExecutorConfig`/settings、Command Stream 帧 DTO；不引用 agent_tool/libopendan | FunctionObject / Thunk 的完整执行协议 |
| scheduler | settings.executor → config.executor 派生 + RBAC 精确规则 | Thunk 提交授权、唯一节点归属、dispatch backend、状态回收 |
| 新 crate `src/kernel/node_executor` | admission、stream server、worker；P2 增加恢复 posture/recovery；不依赖 libopendan | thunk runners，与已有执行组件的复用边界 |
| node_daemon | 薄装配：reconcile 时交 `config.executor` 给 Executor；stream listener；worker 子命令入口；修全局 waitpid | Thunk 配置收敛与执行状态回报 |
| workflow / TaskMgr | 无本次 Thunk 接线要求 | TaskId 协议、runner 归属、epoch/revision、结果与取消链路 |
| cyfs-gateway 配置 | 本机 `executor` 目标 → `127.0.0.1:<port>` 的 stream 路由；Agent 侧 stream 转发入口 | — |
| agent_tool / libopendan | `remote_node` kind、`NodeCommandHandle`、文件工具代理、locator、recovery reader、factory | — |
| verify-hub | 非 User 主体校验 AgentSpec.binding | — |
| control-panel / WebUI | `settings.executor`、`devices/<name>/executor_config` 编辑；设备能力展示 | — |

---

## 13. 实施分期与验收

每阶段默认关闭；不提前暴露只有传输没有授权/去重的执行接口。P0 聚焦来源②，配置权限是 Command Stream 开放的前置条件。来源③仍在 P2；来源①整体后置，完成 §6.2 的设计条件后另行排期，不阻塞 P0/P1/P2。

| 阶段 | 内容 | 退出标准 |
| --- | --- | --- |
| P0-A 配置与权限 | §5.4、§12：settings → config 派生、RBAC、verify-hub binding 校验 | 撤权立即生效；未授权主体零执行；完成后才开放 P0-B |
| P0-B Command Stream（形态 A，Linux） | §7.1–7.5：RTCP stream 路由、登录、exec/read/write/edit、去重、断线查询、worker 降权、waitpid 修复 | hosted xllm 注入 `remote_node` 完成 shell + 文件工具 |
| P1 Agent 侧完整接线 | §7.6：descriptor、locator、recovery reader、PromptExec 显式 call_id | Runner 崩溃后能查询不重放 |
| P1 轻量客户端（形态 B） | 桌面独立 Agent Daemon；`devices/<name>/executor_config`；上线/退出 | Jarvis 能在授权电脑上 read/edit 文件 |
| P2 恢复通道 | §8、§9：posture、声明发现、本地触发、等待期、签名脚本、恢复 Stream | 无头 OOD 在 Zone 不可达时可被 Owner 恢复 |
| P2 形态 C | 移动端嵌入；平台适配层 | 电视盒子配置场景 |
| 后续：Thunk（未排期） | 先定稿 §6.2 的 FunctionObject 执行契约与三路径分工，再重审 config.thunks、各类 runner、状态回报及 TaskMgr 接线 | 开工前完成契约评审；实现后的端到端标准据此制定，候选场景见下表 |
| 后续 | 完整 Session 远端运行（TODO §9.4）、Operator、Windows/macOS worker、跨 Zone | — |

Command Stream、轻量客户端与恢复通道的验收场景（随各自阶段执行）：

| 场景 | 必须观察到 |
| --- | --- |
| 对端设备不在 allowed_devices | Tunnel 可建，Stream 登录拒绝 |
| 同一 Tunnel 另一 Agent 开新 Stream | 不继承前一 Stream 身份与权限 |
| 只读 profile | write/edit 拒绝；exec 不可用或仅白名单 |
| 同 op_id 重投 / 不同载荷 | 副作用最多一次 / Conflict |
| Stream 断开时命令运行中 | 命令继续；重连后 query 得到状态与输出尾部 |
| RTCP 24 h key epoch 重建 | 新 Tunnel 重新登录后可查询旧 op_id |
| 子进程环境探针 | 无设备私钥/token/daemon env；文件工具 UID 与 shell 相同 |
| 密集短命令 + daemon reaper | 无 ECHILD、退出码丢失 |
| 撤权 / 禁用 / 改 run_as | 停止新准入、处理活跃命令、保留结果 |
| 旧 seq 有效签名再次提供 | 拒绝执行 |
| 服务器给未授权的大 seq | 本地进度不推进 |
| 脚本已执行但上报中断 | 结果重报，脚本不重跑 |
| 有效声明 + 设备在范围 | 进入 Frozen：新准入关闭，业务服务不停；等待期内无恢复执行 |
| 声明已过期 / 设备尚未查询到 | 不冻结 |
| 本地触发 | 立即退出 Frozen 或立即进入 Recovery |
| Zone 正常可达 | 日常不依赖 Owner key，不触碰恢复入口 |
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

合并了 TODO 的待定项、讨论稿 §11、2026-10-14/10-18 两次 review 提出的问题。"建议"是本文默认采用的方案；用户裁决后更新"状态"。

| # | 决策 | 选项 | 建议 | 状态 |
| --- | --- | --- | --- | --- |
| D-1 | v1 真实消费方 | a. 独立 xllm（hosted）；b. OpenDAN Session；c. Workflow `func::` | a；c 整体后置，先完成 §6.1–6.2 再排期；Session 后置 | Thunk 后置已定；a 待定 |
| D-2 | Agent 流登录凭据 | a. 现有 App session token（AppDID==AgentDID）；b. TODO §5.2 的 Agent 专属 IdentityRoots + token provider | a；b 作为已知风险 | 待定 |
| D-3 | Command Stream 传输 | a. RTCP Stream + 本机 gateway stream 转发；b. TODO §5.1 的 S2S kRPC over HTTP 3181 | a（双向流、与轻量客户端同一路径、中转由 RTCP 解决） | 待定 |
| D-4 | 离线持久队列 | a. 无（目标离线即失败）；b. TODO 的 spool；c. 复用 Task Dispatch Center | a；需要时 c | 待定 |
| D-5 | Executor 核心代码位置 | a. 新 crate `kernel/node_executor`，node_daemon 与 Agent Daemon 共用；b. TODO 的 `libopendan::node_agent_runtime`（node_daemon → libopendan） | a（轻量客户端不能拉 libopendan） | 待定 |
| D-6 | `Device` 类设备的配置位置 | a. `devices/<name>/executor_config`（control-panel 写）；b. 让 scheduler 也为 Device 生成 nodes/ 条目 | a | 待定 |
| D-7 | Thunk 结果回报通道 | a. `nodes/<node>/thunk_state` + scheduler 收敛；b. 节点直接 kRPC 调 scheduler；c. 节点直接写 TaskMgr | a 为候选；随 FunctionObject 结果契约和 TaskMgr runner 归属一起重审 | 后置待定 |
| D-8 | 只读 Agent 的 exec | a. 不开放 exec；b. 命令白名单；c. 开放但记录 | a 为默认，b 后续 | 待定 |
| D-9 | 恢复脚本签名密钥 | a. Owner Root Key；b. 激活时固化的独立恢复签名 key（公钥进 node_identity） | b（符合 Root Key 不入日常；泄露面分离） | 待定 |
| D-10 | §8.4 签名字段/作用域/失败是否消耗序号 | — | 定稿时随 D-9 一起定 | 待定 |
| D-11 | 等待期细则 | 起算点、可信时间源、重启处理、哪些修改重新等待、本地触发是否可跳过 | 进入 Frozen 起算；持久化；端点迁移不重算、scope/主体变更重算；本地触发可跳过 | 待定 |
| D-12 | Frozen 是否也响应"Zone 可达但权限被盗" | a. 只在不可达时；b. 在线设备也响应止损声明 | a（v1）；b 另立设计 | 待定 |
| D-13 | 私钥泄露后的撤销/替换 | 依赖 BNS/OwnerDocument 轮换（Reset & Unbind §8.2） | 不在本文解决，只留接口：恢复信任根可被新 OwnerDocument 替换 | 待定 |
| D-14 | 轻量客户端的 Gateway 复用 | 已有独立 gateway 的设备如何复用；多隔离 App 的协议栈实例与设备身份如何协调 | 一台设备一个设备身份；多 App 共享本机 gateway，嵌入式仅在无 gateway 时启用 | 待定 |
| D-15 | 已登录 Stream 遇到撤权/执行态变化 | a. 立即关流并取消；b. 允许收尾 | a | 待定 |
| D-16 | `Script` 的执行环境 | 复用 Command Stream worker、aios 容器或独立环境 | 先定义语言、依赖与权限契约，再比较；原 aios 方案仅作候选 | 后置待定 |
| D-17 | Operator 类型 | 暂不支持 | 维持 | 已定 |

---

## 15. 已知风险（按"先能力后安全"原则登记，不阻塞 v1）

- App token 标识 Agent 依赖 AppDID==AgentDID 的约定；若未来一个 App 承载多个 Agent，需要 D-2b。
- roots 不是 sandbox；同一 profile 的多个 Agent 共享一个 OS 权限域。
- Frozen 的 DoS 面（§9.4）；等待期不能区分合法用户与持同一 key 的攻击者（§9.5）。
- 形态 C 的后台运行、权限与各平台能力未验证；cyfs-gateway-lib 在移动端的可嵌入性未验证。
- 非 root daemon 下 Command Stream 不可用（返回 Capability），桌面安装版需单独处理。
- Windows/macOS 的 worker 降权未设计。
- 恢复端点的结果真实性在 D-10 定稿前只依赖 HTTPS。

---

## 16. 与来源文档的关系

- 本文发布后，`notepads/node-agent-runtime-todo.md` 与 `notepads/BuckyOS Node Executor设计思路.md` 降为历史输入；与本文冲突处以本文为准。TODO 中更细的 wire 字段、状态机与崩溃窗口分析（其 §6–§8）在 P0-B 实施时按 D-3 的传输结论迁移到本文或协议文档。
- Thunk 的后置范围与 `doc/workflow/executor list.md` 的后续阶段方向一致；本文 §6.3–6.6 保留旧执行器方案作比较输入，待 §6.2 定稿后更新，不作为首期实施契约。
- `BuckyOS 系统故障的诊断和恢复(beta3).md` §10.3 的第二诊断链路由本文 §9 具体化；该文 §11 的 Agent 修复前置条件对本文来源①的 OPTask 与来源②的修复类命令同样适用。
- `Reset & Unbind.md` §2.3 的物理控制权要求对应本文 §9.1 本地触发；§7.1 对远程擦除的保守态度同样适用于远程恢复脚本的能力边界。
