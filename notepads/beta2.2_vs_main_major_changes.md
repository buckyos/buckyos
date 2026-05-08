# beta2.2 相对 main 的大块改动对比

## 对比范围

- 基准分支：`main`
- 目标分支：`beta2.2`
- 对比方式：`git diff main...beta2.2`
- merge-base：`2a15abb2946a2408240b3d3285df94a7b250a31c`
- beta2.2 当前提交：`b4b7bb99`，提交信息为 `buckycli support node control cmds`
- 总体规模：685 个文件变动，约 162611 行新增、36706 行删除
- 版本号：`src/VERSION` 和 workspace package 从 `0.6.0` 升到 `0.6.1`

`main` 正好是 `beta2.2` 的 merge-base，因此下面内容可以理解为 beta2.2 这条发布分支从 main 分出来后的全部大块新增和重构。

## 总体结论

`beta2.2` 不是一个小修分支，而是把 BuckyOS 往“AI-Native 开发循环 + Web Desktop + 完整内核模块边界”推进的一次大版本分支。最明显的变化有三类：

1. 用户界面从旧的 `control_panel/web` 迁移到新的 `src/frame/desktop`，旧 Control Panel 前端被删除，后端被拆分成多个管理模块。
2. 内核和系统服务补齐了多项基础能力，包括 `workflow`、`klog_daemon`、`node_control`、`zone_route_builder`、`kevent service`、`rdb_mgr`。
3. AICC、Message Center、Repo Service、Task Manager、Node Daemon 等服务围绕 AI、Workflow、RDB 和 Desktop 体验做了成体系扩展。

## 主要改动地图

| 大块 | 主要目录 | 性质 | 影响 |
| --- | --- | --- | --- |
| Web Desktop / Control Panel | `src/frame/desktop`、`src/frame/control_panel` | 新前端 + 后端拆分 | 影响用户入口、内置应用、部署路径 |
| AICC | `src/frame/aicc`、`doc/aicc`、`test/aicc_test` | Provider / Router / Usage Log 扩展 | 影响模型调用、路由、计费/日志、测试方式 |
| Workflow | `src/kernel/workflow`、`doc/workflow`、`test/workflow_test` | 新 kernel service | 影响 Agent-human-loop、任务编排、AICC adapter |
| Klog / Klog Daemon | `src/kernel/klog`、`src/kernel/klog_daemon` | Raft 化、持久化、集群管理 | 影响日志一致性、部署端口、运维测试 |
| Node / Scheduler / Gateway | `src/kernel/node_daemon`、`src/kernel/scheduler`、`doc/arch/gateway` | 路由、探测、启动收敛增强 | 影响节点启动、服务路由、tunnel 选择 |
| BuckyOS API | `src/kernel/buckyos-api` | 新共享类型和客户端 | 影响前后端、服务间协议、CLI |
| Message Center | `src/frame/msg_center`、`doc/message_hub` | Self-host group + RDB | 影响群组、联系人、消息存储 |
| RDB 迁移 | `repo_service`、`task_manager`、`msg_center`、`buckyos-api` | 从本地 DB 走 RDB instance 抽象 | 影响存储后端、schema、部署配置 |
| 构建和发布 | `src/buckyos-build.py`、`publish/aios`、`publish/exttool` | 交叉编译、镜像、rootfs 安装调整 | 影响开发构建、Docker/PAIOS 镜像 |

## 1. Web Desktop 替代旧 Control Panel Web

### 主要变化

- 新增完整的 React/Vite Desktop 前端：`src/frame/desktop`。
- 删除旧前端：`src/frame/control_panel/web`。
- `src/bucky_project.yaml` 不再把 `control_panel_web` 作为 web 模块构建，改为构建 `desktop`，但安装路径仍落到 `bin/control-panel/web/`，保持运行时路径兼容。
- Desktop 内置 app 明显扩展，入口在 `src/frame/desktop/src/app/registry.tsx`，包括：
  - `ai-center`
  - `app-service`
  - `files`
  - `messagehub`
  - `homestation`
  - `task-center`
  - `users-agents`
  - `workflow`
  - `settings`
  - `diagnostics`
  - `market`
  - `studio`
  - `codeassistant`

### Control Panel 后端变化

旧的 `src/frame/control_panel/src/main.rs` 被大幅拆分，新增多个后端模块：

- `aicc_settings.rs`
- `app_servcie_mgr.rs`
- `dashboard.rs`
- `message_hub.rs`
- `sys_auth_backend.rs`
- `sys_log_mgr.rs`
- `sys_settings.rs`
- `ui_session_mgr.rs`
- `user_mgr.rs`
- `zone_mgr.rs`

这说明 beta2.2 的 Control Panel 后端仍然存在，但职责从“一个大 main.rs + 旧前端”转成“为新 Desktop 提供管理 API 的 frame service”。

### 需要注意

- 这是用户入口层面的最大改动。任何依赖旧 `control_panel/web` 目录、旧路由、旧静态资源路径的脚本或文档都需要复查。
- 新 Desktop 大量页面仍包含 mock store 和 prototype 数据，不能简单等同为后端能力已经全部生产化。

## 2. AICC 从 Provider Adapter 扩展到 Router / Registry / Usage Log

### 主要变化

`src/frame/aicc` 新增或大幅扩展了以下模块：

- `model_types.rs`
- `model_registry.rs`
- `model_router.rs`
- `model_scheduler.rs`
- `model_session.rs`
- `default_logical_tree.rs`
- `aicc_usage_log_db.rs`
- `fal.rs`
- `sn_ai_provider.rs`

现有 provider 也有较大变化：

- `openai.rs`
- `gimini.rs`
- `claude.rs`
- `minimax.rs`

配套文档新增：

- `doc/aicc/aicc_api设计.md`
- `doc/aicc/aicc_router.md`
- `doc/aicc/aicc_provider_plan.md`
- `doc/aicc/aicc_usage_log_db_requirements.md`
- `doc/aicc/aicc 逻辑模型目录.md`

测试侧也从旧的 remote runner 模式调整为 Deno/TS 测试：

- 删除 `test/aicc_test/aicc_remote_runner.ts`
- 新增 `test/aicc_test/test_fal.ts`
- 新增 `test/aicc_test/test_list_models.ts`
- 大幅扩展 `test/aicc_test/aicc_smoke.ts`

### 影响判断

beta2.2 里的 AICC 已经不只是“调用若干模型 provider”，而是开始形成模型注册、模型路由、会话、调度、用量记录和 SN provider 的组合能力。它也是 Workflow 的重要下游能力，`workflow` service 已经注册 `service::aicc.*` adapter。

### 需要注意

- AICC 协议、数据结构、测试入口都有联动改动。修改 AICC 时需要同时检查 `buckyos-api`、Desktop AI Center、`doc/aicc` 和 `test/aicc_test`。
- provider 侧新增 FAL 和 SN AI provider，部署环境里的 key、endpoint、用量日志表都可能需要同步配置。

## 3. 新增 Workflow Kernel Service

### 主要变化

新增 workspace member：

- `src/kernel/workflow`

新增部署模块：

- `src/bucky_project.yaml` 中加入 `workflow`
- 安装到 `bin/workflow/`

Workflow service 的入口在 `src/kernel/workflow/src/main.rs`。它装配：

- workflow DSL / compiler / analyzer
- in-memory definition store 和 run store
- orchestrator
- task manager tracker
- executor registry
- AICC service adapter
- run subscription manager
- kRPC HTTP server

关键模块包括：

- `orchestrator.rs`
- `compiler.rs`
- `dsl.rs`
- `analysis.rs`
- `executor_adapter.rs`
- `task_tracker.rs`
- `adapters/aicc.rs`
- `server.rs`

配套文档和 DV 测试：

- `doc/workflow/workflow service.md`
- `doc/workflow/wokflow engine.md`
- `doc/workflow/executor list.md`
- `test/workflow_test/workflow_dv.ts`
- `test/workflow_test/testcases.md`

### 影响判断

这是 beta2.2 的核心新增内核能力之一。Workflow 把 Agent 意图、AICC 调用、Task Manager 可视化和 human-loop 串起来，当前实现已经支持 run、step、map shard、thunk 等状态同步到 task_manager。

### 需要注意

- 目前 `main.rs` 里明确写到一期仍使用进程内 dispatcher / object store，持久化是后续工作。这意味着当前能力更偏 MVP/服务骨架，不应假设所有 workflow runtime 状态已经具备可靠持久化。
- Workflow 与 Scheduler 的 `ThunkObject`、`FunctionObject`、`run_thunk` 能力相关，协议或类型改动必须联查 `buckyos-api`、`scheduler`、`task_manager`。

## 4. Klog / Klog Daemon 大幅增强

### 主要变化

`klog` 从旧的较简单日志模块扩展为带 Raft 语义、状态机、持久化和 RPC 的内核组件。新增或大改内容包括：

- `src/kernel/klog/src/logs/sqlite.rs`
- `src/kernel/klog/src/logs/rocksdb.rs`
- `src/kernel/klog/src/state_store/rocksdb.rs`
- `src/kernel/klog/src/state_machine/machine.rs`
- `src/kernel/klog/src/rpc/client.rs`
- `src/kernel/klog/src/rpc/server.rs`
- `src/kernel/klog/src/service/mod.rs`
- `src/kernel/klog/src/util/persist_format.rs`

新增 daemon：

- `src/kernel/klog_daemon`

`klog_daemon` 包含：

- config loading
- cluster bootstrap / auto-join
- lifecycle / graceful shutdown
- admin API
- raft 网络端口
- inter-node 转发端口
- local client RPC 端口
- 本地 benchmark 工具 `klog_bench`

新增大量集成测试：

- `admin_semantics.rs`
- `cluster_identity.rs`
- `failover.rs`
- `forwarding.rs`
- `membership.rs`
- `multi_node_rw.rs`
- `restart_recovery.rs`
- `single_node.rs`

### 影响判断

Klog 是 beta2.2 里改动规模最大的内核基础设施之一。它开始承担分布式日志、meta kv、强读、leader forwarding、request-id 幂等、RocksDB/SQLite 后端、快照和成员管理等能力。

### 需要注意

- `klog_daemon/readme.md` 已明确四类端口：Raft 控制面、inter-node 业务转发、admin 管理面、本机 client RPC。部署和 gateway 路由需要按这四类端口配置。
- 新增 `openssl` vendored、RocksDB 相关构建路径，macOS 交叉编译脚本也为此做了额外处理。

## 5. Node Daemon / Scheduler / Gateway 路由增强

### Node Daemon

`src/kernel/node_daemon` 新增：

- `boot.rs`
- `finder.rs` 大改
- `gateway_tunnel_probe.rs`
- `gateway_name_provider.rs`
- `kevent_server.rs`
- `node_exector.rs`
- `run_plist.rs`

主要方向是：

- 启动/boot 流程增强
- finder 结果带 DID cache
- 网络 probe / tunnel probe
- run item plist 支持
- node executor 支持
- node_daemon 内提供 kevent server

### Scheduler

`src/kernel/scheduler` 新增：

- `zone_route_builder.rs`
- `thunk_runner.rs`

`zone_route_builder` 会根据 device doc、network observation、probe 结果、SN、ZoneGateway 等信息构造 forward plan 和 route candidate。

`thunk_runner` 与 Workflow / FunctionObject / OP Task 方向联动。

### Gateway 配置和测试

网关文档和配置明显扩展：

- `doc/arch/gateway/readme.md`
- `doc/arch/gateway/service selector.md`
- `doc/arch/gateway/test zone_route_build.md`
- `doc/arch/gateway/boot_gateway的配置生成.md`
- `doc/arch/gateway/服务的多链路选择.md`
- `src/rootfs/etc/boot_gateway.yaml`
- `src/test/test_boot_gatweay/*`

### 影响判断

beta2.2 在“节点如何被发现、服务如何通过多链路访问、路由计划如何生成”上做了系统性增强。这与 ZoneGateway、NodeGateway、SN、rtcp tunnel、network observation 都有关。

### 需要注意

- 这里是协议、配置、文档、测试一起动的区域。修改 route / gateway 相关字段时，需要联查 scheduler、node_daemon、rootfs 配置和 `doc/arch/gateway`。
- `src/test/test_boot_gatweay` 目录名里仍是 `gatweay` 拼写，脚本或文档引用时要按现有目录名处理。

## 6. buckyos-api 共享类型和客户端扩展

`src/kernel/buckyos-api` 新增或扩展很多共享 API：

- `aicc_usage_log.rs`
- `group_mgr.rs`
- `msg_center_client.rs`
- `network_observation.rs`
- `node_control.rs`
- `rdb_mgr.rs`
- `repo_client.rs`
- `system_contorl.rs`
- `thunk_object.rs`
- `workflow_dsl.rs`
- `workflow_runtime.rs`
- `workflow_service.rs`
- `workflow_types.rs`

同时 `sn_client.rs` 被删除，SN 相关类型改由 `cyfs-gateway-api` re-export。

### 影响判断

beta2.2 把很多原来分散在服务内部的协议和模型上移到了 `buckyos-api`。这对前后端、CLI、kernel/frame service 的编译边界都有影响。

### 需要注意

- `system_contorl.rs` 文件名目前是 `contorl`，不是 `control`。这是现状，不要在未确认影响面前直接重命名。
- `node_control.rs` 当前文件头写明大量内容仍是“协议/实现设计文档，不是已经导出的 Rust API”，但 `buckycli node` 已经开始调用其中能力。后续落地时要避免文档型代码和真实 API 边界混乱。

## 7. Message Center 支持 Self-host Group，并迁移到 RDB

### 主要变化

`src/frame/msg_center` 新增：

- `group_mgr.rs`

`group_mgr.rs` 头部说明它对应 `doc/message_hub/Self-Host-Group.md`，并通过 `MsgBoxDbMgr` 持久化到共享 sqlite/postgres 数据库。

相关变化还包括：

- `contact_mgr.rs` 大改
- `msg_box_db.rs` 大改
- `msg_center.rs` 大改
- `buckyos-api/src/group_mgr.rs`
- `buckyos-api/src/msg_center_client.rs`

### 影响判断

Message Center 从消息流和联系人管理扩展到 self-host group 的 DID collection / group doc / member proof / subgroup / expansion snapshot 体系。

### 需要注意

- group 数据按 `host_owner_key` 做多租户隔离。
- group 的 Active membership 依赖 `GroupMemberProof`，目前证明验证委托给 proof signer，代码接受 canonical JSON proof。
- 修改 group schema 或 group API 时需要同步 `doc/message_hub/Self-Host-Group.md` 和 `buckyos-api`。

## 8. RDB Instance 抽象被更多服务采用

### 主要变化

新增 `buckyos-api/src/rdb_mgr.rs`，并把多个服务接到 RDB instance：

- `repo_service` 的 `repo_db.rs` 改为 `sqlx::AnyPool`，支持 sqlite/postgres，并从 service spec 解析 `REPO_SERVICE_RDB_INSTANCE_ID`。
- `task_manager` 的 `task_db.rs` 大改。
- `msg_center` 的 `msg_box_db.rs` 大改。
- AICC 新增 `aicc_usage_log_db.rs`。

`Cargo.toml` 新增 workspace dependency：

- `sqlx`

### 影响判断

这是存储层抽象的重要变化。beta2.2 开始把系统服务的数据存储从服务内部本地实现，迁移到统一 RDB instance 模型。

### 需要注意

- schema、连接串、后端类型会通过 service spec / RDB manager 传递。部署配置和测试环境需要同步。
- sqlite/postgres 的 SQL placeholder 和 DDL 执行差异已经在部分代码里处理，新增 SQL 时应复用已有 render/split 模式。

## 9. App Loader、HostScript、PAIOS / AIOS 镜像和构建发布

### App Loader / Runtime

相关改动包括：

- `src/kernel/node_daemon/src/app_loader.rs` 大改
- `src/rootfs/bin/service_debug.tsx` 大改
- `src/apps/sys_test` 重构
- `HostScript Type App` 相关提交

这对应 beta2.2 规划里的 Script 类 AppService / PAIOS 统一镜像方向。

### 构建和发布

构建发布变化包括：

- `src/buckyos-build.py` 增加 macOS 到 Linux target 的交叉编译环境处理。
- `build_aios` 大改。
- 新增 `build_exttool`。
- `publish/aios/Dockerfile` 大改，并新增 `publish/aios/entrypoint.sh`。
- 新增 `publish/exttool/Dockerfile` 和 `publish/exttool/install_freecad.sh`。
- 新增 `rust-toolchain.toml`。
- 删除 `uv.lock`。

依赖分支也从多个 `main` 切到 `beta2.2`：

- `buckyos-base`
- `cyfs-ndn`
- `cyfs-gateway`

同时新增：

- `buckyos-http-server`
- `cyfs-gateway-api`
- `openssl` vendored
- `sqlx`

### 影响判断

beta2.2 的构建链路与外部依赖分支绑定更强，且为了 klog/RocksDB/openssl/交叉编译做了较多环境处理。CI、发布镜像、开发机本地构建都可能受到影响。

## 10. CLI 新增 node 控制命令

`src/tools/buckycli/src/node_cmd.rs` 新增本机 node runtime 控制命令：

- `buckycli node check`
- `buckycli node start`
- `buckycli node ensure-running`
- `buckycli node stop`
- `buckycli node restart`
- `buckycli node detect-host-control`

设计目标是逐步替代开发期 `start.py` / `stop.py` / `check.py` 中的本机 Runtime 控制能力，并支撑 Desktop 安装、升级、卸载流程。

需要注意的是，当前仓库仍保留 Python 脚本，CLI 是新增能力，不代表旧脚本已经完全删除。

## 11. 测试和文档扩展

### 测试新增

- `src/test/test_boot_gatweay`
- `test/workflow_test`
- `test/test_control_panel`
- `test/test_helpers`
- `test/aicc_test` 重构
- `src/frame/desktop/tests/e2e`
- `klog` / `klog_daemon` 大量 Rust 集成测试

### 文档新增

新增文档集中在：

- `doc/aicc`
- `doc/workflow`
- `doc/arch/gateway`
- `doc/message_hub`
- `src/frame/desktop/doc`
- `product`
- `proposals`
- `notepads`
- `harness/human-rules/AIOPS.md`

`notepads/beta2.2,beta3规划.md` 明确写到 beta2.2 的目标是进入 AI-Native 开发循环，并稳定 BuckyOS 的概念抽象和模块边界。这与本分支的大块改动方向一致。

## 合并和后续 Review 重点

1. Desktop 与 Control Panel：确认旧 `control_panel/web` 删除后，所有构建、安装、静态资源路径和登录流程都指向新 Desktop。
2. AICC：重点 review provider/router/session/usage log 的协议兼容性，以及 Desktop AI Center 与后端真实 API 的差距。
3. Workflow：确认当前 in-memory store 的生命周期边界，避免把 MVP 状态误认为生产级持久化。
4. Klog：重点 review 端口部署、gateway 路由、RocksDB/SQLite 后端、强读和 leader forwarding 语义。
5. RDB：确认每个服务的 RDB instance schema、连接串、sqlite/postgres 差异和迁移路径。
6. Node / Gateway：重点 review network observation、probe、forward plan、boot gateway 配置的字段联动。
7. Build：确认 `beta2.2` 依赖分支、`uv.lock` 删除、macOS cross compile、vendored openssl 对 CI 和 release 的影响。

## 快速复查命令

```bash
git diff --shortstat main...beta2.2
git diff --dirstat=files,10,cumulative main...beta2.2
git diff --stat main...beta2.2
git log --oneline --no-merges main..beta2.2
```

按模块继续看：

```bash
git log --oneline --no-merges main..beta2.2 -- src/frame/desktop src/frame/control_panel
git log --oneline --no-merges main..beta2.2 -- src/frame/aicc doc/aicc test/aicc_test
git log --oneline --no-merges main..beta2.2 -- src/kernel/workflow doc/workflow test/workflow_test
git log --oneline --no-merges main..beta2.2 -- src/kernel/klog src/kernel/klog_daemon
git log --oneline --no-merges main..beta2.2 -- src/kernel/node_daemon src/kernel/scheduler doc/arch/gateway
```
