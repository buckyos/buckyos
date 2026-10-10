# Agent Workspace Protocol

## 1. 范围

本协议落实 [OpenDAN Agent Workspace 设计](<../OpenDAN Agent Workspace 设计.md>) 的首轮本地实现。Workspace 是独立于 Session 的长期目录；稳定身份不由目录路径决定。Agent State 保存 Known Workspaces，Session 保存创建时解析的位置快照。

本轮 Runtime 引用为 `local`，表示当前 Agent State 所在主机可访问的文件系统，native 和 tmux 执行实例都映射到它。登记及 Session 快照用 `runtime_host` 固定本地宿主身份；通过 kRPC 返回的另一宿主上的同名路径不能被当成本机目录执行。它不是 tmux Session ID。尚未接入远程 Workspace helper、Host/Container 注册表、跨主机锁或文件同步；不支持的 Runtime 明确失败。

## 2. 数据分类与存储

| 数据 | 分类 | 位置与保留语义 |
| --- | --- | --- |
| 项目文件、Workspace 身份元信息 | 持久数据 | 所选 Runtime 内目录；Session 完成、归档、取消登记均不删除 |
| Workspace 登记、位置修订、可用性观察 | 持久数据 | AgentRoot 下 `state/workspaces/`；列表直接读取登记，不扫描项目目录 |
| Session Workspace 快照 | 持久数据 | `session_config.json.workspace_binding` 和 `binding.json.workspace` |
| 锁占用 | 运行状态 | 本地 OS 锁；磁盘锁文件保留代次与持有者信息，不通过删除锁文件释放 |
| 工具日志、报告和运行证据 | Session 数据 | 沿用 Session 协议；归档 Workspace 不清理这些记录 |

直接使用文件系统是本组件的语义要求：身份必须随用户目录一起移动；Agent State 沿用现有 Fs/kRPC 协议。此次没有新增数据库、服务或第三方依赖，不改变安装与 scheduler 接入。

## 3. 目录元信息

目录根下 `.opendan-workspace.json` 是版本 1 JSON；字段见 `schema/workspace_metadata.schema.json`：`version`、`workspace_id`、`name`、`description`、`created_at_ms`、`operation_id`。不写入 Runtime 地址、凭据、私有 Notes 或 Session 锁。

元信息携带稳定 ID 和共享描述；Agent State 登记才是本 Agent 已确认位置的来源。Runtime/目录只取本次受支持访问环境和规范化路径，不从不可信元信息取得授权。损坏内容、未知版本、符号链接元信息均失败，不当成“文件缺失”覆盖初始化。

创建/导入带 `operation_id`；重复调用保留身份。元信息落盘但登记未完成时，重试可以继续登记，不能产生新 ID 或删除用户目录。没有元信息的现存目录可通过显式导入纳管；普通打开不会自动纳管。

## 4. 登记与修订

`WorkspaceRecord` 使用必填 `version: 1`，未知登记版本拒绝读取。记录包括稳定 ID、共享名称/描述缓存、`location {runtime_id,directory}`、`usage`（`private` / `collaborative`）、`lifecycle`（`active` / `archived`）、独立的 `availability`、`revision`、`location_revision`、来源/登记者/时间、可选策略引用、私有 Notes、冲突和错误观察。

可用性取值为 `available`、`missing`、`runtime_unavailable`、`permission_denied`、`invalid_metadata`、`conflict`。它是最近检查结果，不是下次执行的保证。`revision` 用于条件更新；只有位置改变才增加 `location_revision`，描述/观察刷新不使正常运行的绑定失效。每次 `check`（即使可用性未变）和 Runtime 状态刷新都会增加 `revision`；条件更新须使用最新修订。

重新定位要求新目录携带同一身份、旧路径确定不存在且无歧义，并检查期望修订和未结束关联。旧 Runtime 离线不能作为移动证明；两个可访问目录持有相同 ID 时记录冲突，不抢占原位置。归档和取消登记检查未结束工作及任务；均不删除物理目录。`restore` 只把生命周期恢复为 active，不修复目录、Runtime 或执行权限。

## 5. Session 绑定与执行

`session_config` 升为 `/7`，`binding` 升为 `/4`。`workspace` 为 `{workspace_id, access}`，不再接受 `kind:agent/external` 和路径引用；`workspace_binding` 持久保存稳定 ID、登记/位置修订、Runtime/目录、`runtime_host` 和访问模式。

创建时解析并核验；无 Workspace 的任务使用自身 SessionDir。另行给出的 Runtime workdir 必须与所选目录一致，不能覆盖 Workspace 决策。已有 Session 的配置和 binding 均不得改为另一个 Workspace、目录或访问模式；子 Session 的继承也不能扩大访问能力。

启动、恢复和工具调用前核验登记、位置修订、目录元信息和执行环境。绑定 Workspace 的 run 不允许独立 `xllm --resume` 绕过校验和写锁，必须由 `xagent run` 驱动。原绑定不可访问或失效时，Session 终态失败并保留原 binding 和执行证据；不重建目录、不改绑、不因访问恢复而重启失败 Session。后续工作须创建新 Session。

本轮单写者协调仅覆盖同一 Agent State 的受控 Session。锁不隔离用户编辑器、任意 shell 派生进程或其他 Agent/Runtime；权限仍由 Runtime 和操作系统落实。未核实的后台任务/在途调用不得作为已完成处理；本轮只读绑定的执行明确失败，因为本地 shell 不能落实只读隔离；父子 Session 同时写同一目录时返回忙碌，需要串行交接。父任务只等待已登记的同 Workspace 子任务时，驱动者提交等待状态并释放锁；宿主优先推进最内层任务，完成后重新核验父任务绑定并继续。交接逐级核验等待快照、调用及子任务关系，不能把任意 Paused run 或未知后台写入当作已结束；停止请求会传递给正在推进的子任务。Worklog/报告提供调用归因，尚不提供完整文件变更审计、自动回滚或交付适配；失败不代表副作用撤销。

## 6. 管理接口与 CLI

`AgentStateClient.workspaces()` 在 Fs、转发和 kRPC 客户端统一提供：`query` / `lookup` / `create` / `import` / `discover` / `check` / `update` / `archive` / `unregister` / `runtime_impact` / `set_runtime_available`。kRPC 方法前缀为 `workspaces.`，字段由同名 Rust 请求类型与 schema 定义。已有 RPC 认证边界继续适用；登记与策略引用不自动赋予访问权限。

```bash
xagent workspace list
xagent workspace create --runtime local --directory /projects/demo --name Demo --description "长期项目" --key create-demo-1
xagent workspace import --runtime local --directory /projects/existing --name Existing --key import-existing-1
xagent workspace get <workspace_id>
xagent workspace check <workspace_id>
xagent workspace discover --runtime local --directory /projects/moved --expected-revision <revision>
xagent workspace archive <workspace_id> --expected-revision <revision>
xagent workspace restore <workspace_id> --expected-revision <revision>
xagent workspace unregister <workspace_id> --expected-revision <revision>
xagent workspace runtime-impact local
xagent workspace set-runtime local unavailable
xagent workspace set-runtime local available
xagent new --objective "更新项目" --workspace <workspace_id> --no-run
agent-session create-worksession --objective "一次性系统操作" --workspace none
```

命令需要与 xagent 相同的 Agent 身份与 State 连接环境。`create` / `import` 要求 `--key`，重试使用相同 key；默认协作属性为 collaborative，可显式 `--usage private`。`set-runtime` 管理 Workspace 对该 Runtime 的访问状态，不销毁实际 Runtime 或目录。

## 7. 版本与升级

开发阶段采用 breaking change，不双读双写旧索引。旧 `index.json`、`workspaces/bindings.json` 和 `session_workspace_bindings.json` 不作为新登记来源。保留原目录，按明确路径逐个 `workspace import`，检查返回 ID 后用新 Session 开展工作。损坏或冲突记录需先人工明确目录身份；工具不会猜测或覆盖。

旧 Session `/6` 不能用新版继续驱动；保留现场和报告。未来元信息/登记变更必须增加版本并定义迁移；稳定 ID、位置修订、绑定不可变语义不能静默改变。

## 8. 查询与消费方

列表从登记集合读取，可按 Runtime、生命周期、可用性和文本过滤；不会因远端离线隐藏长期项目。查询/影响分析采用本地登记扫描，本轮无大型目录统计或远程内容遍历。Homepage 展示登记位置、协作属性、可用性与归档状态，提供检查和重新定位入口；执行占用来自 Session 登记，只作为提示。

Session 模板、子 Session、产物关联、CLI 和 Homepage 均使用稳定 ID。共享目录元信息和 Homepage 列表不包含 Agent 私有 Notes 或凭据；授权的 Agent State 管理 RPC（含检查和重新定位）返回完整登记，Homepage 不展示其中的私有 Notes。新增同步/交付适配和分布式协调需另行定义能力和验收，不能从本地测试推导远程可用。
