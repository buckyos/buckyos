# OpenDAN Agent Service 原生运行 TODO（owner Agent 选择 host-native session-runtime 时）

日期：2026-10-25

状态：方向与 §5 四项均已由用户确定（2026-10-25，见 §0 N1–N4）；N1–N7 已实施并在本机单 OOD DV 通过（未提交），见 §6。

路径均相对 `buckyos/`。相关文档：[Desktop 无 Docker 研究报告](buckyos-desktop-without-docker-research.md)（本方向是其 §5 步骤 ③“原生 owner Agent”，但触发条件不同）、[Session Runtime 与 Workspace 解耦](opendan-primary-runtime-tools-todo.md)（host 标准环境、`bin/agent-tools`）、[Agent Runtime 集成](agent-runtime-session-workspace-integration-todo.md)（`owner_agents`、派生 profile）。

## 0. 决策

| # | 决策（用户，2026-10-25） | 直接推论 |
| --- | --- | --- |
| N0 | **zone owner 的 Agent 选择 host-native-runtime 时，Agent Service 不再基于 paios 镜像启动。** | 触发条件是这个 Agent 的 runtime 选择，不是节点有没有 Docker。Agent Service（opendan Loader）作为 node_daemon 管理的原生进程跑在 host runtime 所在的节点上；同一个 Zone 里其它 Agent（非 owner、或没选 host 的 owner Agent）照旧在 paios 容器里。 |
| N1 | 触发条件以 **默认 runtime 是 host** 为准（§2）。 | 只启用 host、默认仍是 container 的 Agent 继续在容器里。 |
| N2 | 原生 Agent Service 以 node_daemon 的身份运行；**本机开发环境（Linux、root）这样是对的**。为避免 Linux 上的权限问题，**Linux 下创建 Agent 时不选 host-runtime**。 | 创建默认值：host 节点是 Linux 时默认 runtime 是 container（用户仍可显式选 host）；Windows / macOS（Desktop，node_daemon 是登录用户）默认 host。 |
| N3 | 原生模式下 `container` runtime 不可用；**本机容器可用时，创建 Agent 时让用户选择**（host 或 container）。 | control-panel 的 runtime 目录给出容器是否可用；Desktop 创建向导在两者都可用时显示选择，只有一种可用时不显示。 |
| N4 | 默认 runtime 在 host ↔ container 间切换即重启 Agent Service，同意。 | 进行中的 Turn 按既有恢复机制继续。 |

## 1. 为什么成立（2026-10-25 对照源码与本机 DV）

- **Agent Service 本身不执行模型命令。** Session 的命令、文件读写都在它的 session-runtime 上：`host` 经 Node Executor 到宿主机，以节点 `owner_agents` 派生的 profile 身份执行，标准命令来自宿主的 `$BUCKYOS_ROOT/bin/agent-tools`（primary-runtime TODO T2）。容器只在 Session 选 `container` runtime 时才是执行环境。
- **宿主已经有原生 OpenDAN。** DV 上 `/opt/buckyos/bin/opendan/` 有静态链接的多入口 `opendan`、`xagent`、`agent_tool` 与 `web/`，`bin/agent-tools/` 是指向它的入口（node_daemon 启动执行器前布置）。
- **身份契约与 Docker 无关。** `build_runtime_env`（`kernel/node_daemon/src/app_loader.rs`）在 node_daemon 进程里用设备私钥签 `BUCKYOS_APP_TOKEN`，连同 `BUCKYOS_ZONE_CONFIG / THIS_DEVICE / APP_ID / APP_DID / APP_INSTANCE_ID / OWNER_USER_ID / DATA_DIR …` 交给容器；opendan 启动只依赖这些 env 与 `--agent-bin`（`frame/opendan/src/main.rs::zone_env`）。原生进程拿同一套 env 即可，不需要新身份方案，也不用 `--trust-loopback`。
- **调度器已经读得到选择。** `scheduler/src/executor_auth.rs` 为派生 `owner_agents` profile 读取 `users/<owner>/agents/<id>/settings`（`AgentSettings.runtimes / default_runtime_id`）、`nodes/<node>/settings`、`ZONE_OWNER_USER_ID_KEY`；AgentId 与构造 App 的 AppId 相同。
- **现在的阻碍**（都是“Agent ⇒ 容器”的硬编码）：
  - 调度器 `system_config_agent.rs::create_service_spec_by_app_config`：`need_container = is_agent` 无条件；
  - `scheduler/src/app.rs::build_app_service_config`：节点支持容器时允许只有 agent 包，不支持时要求 `<arch>_<os>_app` / `script` 包；
  - AppLoader `resolve_runtime`：Agent 必须 `device_supports_container()`，`start_agent` → `run_worker_container`，`stop_agent` 删容器，`status` 走 `docker inspect`；
  - opendan `package_root` 兜底写死 `/opt/buckyos/bin/<appid>`；Loader 总是把 Agent 进程所在环境投影为 `container` runtime（`exec: native`）。

## 2. 定义：什么算“选择 host-native-runtime”

同时满足：

1. Agent 的 owner 是 zone owner（`AgentSpec.binding.target_app_instance_id.owner_user_id() == zone owner`）；
2. `AgentSettings.effective_default_runtime()` 是一个 `AgentRuntimeTarget::Node { node_id }` 的 runtime（即 `host`）；
3. 该节点的 `nodes/<node_id>/settings.executor` 开启且 `owner_agents.enabled`（调度器为这个 Agent 在该节点派生出 profile）。

满足时：放置节点 = `node_id`，运行方式 = 原生；任一不满足：照旧容器。只“启用”host 但默认仍是 `container` 的 Agent 仍在容器里（§5 第 1 项）。

## 3. 设计

### 3.1 共享类型（buckyos-api）

- `AppServiceInstanceConfig` 增加 `run_mode: AppRunMode { Container, Native }`（serde 缺省 `Container`）。
- 纯函数 `native_agent_node(input: &HashMap<String, String>, app_spec: &AppServiceSpec) -> Option<String>`：按 §2 从 system-config 输入得出原生放置节点；调度器的 spec 推导、实例化与 `executor_auth` 共用同一判断，不在多处重复解析。

### 3.2 调度器

- `create_service_spec_by_app_config` 拿到输入后：Agent 且 `native_agent_node` 有值 → `need_container = false`，`node_affinity = node_id`；否则维持现状。
- `build_app_service_config`：原生放置时接受只有 `agent` 包的 AppDoc（不检查 docker / `<arch>_<os>_app`），写 `run_mode = Native`。
- Agent settings 或节点 settings 变化后重新推导：实例的 `run_mode` / 节点变化即改写节点配置（容器 ↔ 原生切换走 node_daemon 的 stop / start）。
- 放置节点离线或 `owner_agents` 被关：不回退到容器，记为不可调度并在 control-panel 显示原因（研究报告 G6：至少不能让这个 spec 拖垮整轮调度）。

### 3.3 node_daemon（AppLoader）

- `RuntimeType::NativeAgent`：由实例 `run_mode = Native` 选中，与 `support_container` 无关。
- **deploy**：与容器 Agent 相同，确保 agent 包与 agent_skills 包已安装。
- **start**：先清理该实例遗留的容器（容器 → 原生切换）；以 `build_runtime_env` 的同一套 env 启动 `$BUCKYOS_ROOT/bin/opendan/opendan --appid <app_id> --agent-bin <agent 包 full_path> --service-port <port>`，路径类 env 全部换成宿主路径（`BUCKYOS_DATA_DIR` = `app_data_dir()`、日志 / storage 目录、`BUCKYOS_AGENT_SKILLS_PKG_DIR` = skills 包 full_path），`BUCKYOS_TOOL_ENDPOINT = http://127.0.0.1:3180`，`BUCKYOS_HOST_GATEWAY = 127.0.0.1`，`BUCKYOS_APP_TYPE = agent`，另设 `OPENDAN_SERVICE_MODE = native` 告诉 Loader 没有容器。独立进程组，stdout / stderr 进 `logs/apps/<instance>/`，pid 与启动时间写进实例本地目录。
- **status**：pid 存活且 exe / cmdline 是这个实例（防 pid 复用），可加服务端口探测；**stop**：对进程组 SIGTERM，超时 SIGKILL（Windows 用 Job Object，Desktop 阶段再做）。原生 → 容器切换时由 stop 结束进程。
- 进程身份 = node_daemon 的身份：Desktop 上是登录用户（integration TODO R1）；Linux 服务器 OOD 上是 root（§5 第 2 项）。
- 不复用 Node Executor 托管主进程：它是一次 exec 一个 shell 的执行器，没有保活与重启语义（研究报告 §3）。

### 3.4 opendan / libopendan

- `OPENDAN_SERVICE_MODE = native` 时，Loader 把 `container` 内建 runtime 投影为不可用（说明“Agent Service 原生运行在 <node>，没有 Agent 容器”），而不是把 Agent 进程所在的宿主环境当作 `exec: native` 的 runtime：后者会以 node_daemon 的身份（root）直接在宿主执行模型命令，绕过节点 `owner_agents` 的 `run_as` 与 roots。冻结在 `container` 上的旧 Session 按仓库规则绑定失败，不做迁移。
- `package_root` 兜底改为 `$BUCKYOS_ROOT/bin/<appid>`；`session_cli` / `session_helpers` / `web_dir` 已按可执行文件旁的位置取，原生下就是 `bin/opendan/`。
- Agent 进程的控制侧 helper（`session.helper` 转发的 `agent-session` 子命令）在宿主以服务身份运行：只是 `xagent` 的受限命令，不是任意 shell。

### 3.5 control-panel 与 UI

- Agent 详情页（Desktop、OpenDAN WebUI 运行环境卡片）显示 Agent Service 的运行方式与位置（“原生 · ood1” / “容器”）；在默认 runtime 的 host ↔ container 切换处说明：切换会以另一种方式重启 Agent Service，冻结在另一种 runtime 上的 Session 不再可用。
- `agent.update` 不需要新字段：运行方式由 §2 推导，不让用户单独选。

## 4. 实施任务

- [x] **N1 共享类型**：`AppRunMode`、`AppServiceInstanceConfig.run_mode`、`native_agent_node`；单测覆盖 §2 三个条件的正反例。
- [x] **N2 调度器**：spec 推导与实例化；settings 变化翻转运行方式；放置节点不可用时 spec 级不可调度、不影响其它 spec；`executor_auth` 改用同一判断。测试沿用 `executor_auth` 的输入表做法。
- [x] **N3 node_daemon**：`NativeAgent` 的 deploy / start / status / stop / preview；切换时清理另一种方式的遗留；用临时目录里的假 `opendan` 脚本测进程启停、pid 复用防护与 env。
- [x] **N4 opendan**：`OPENDAN_SERVICE_MODE` 下 `container` runtime 不可用、`host` 为默认；`package_root` 兜底；`opendan/tests/loader.rs` 增加原生模式用例。
- [x] **N5 UI**：control-panel 返回运行方式，Desktop 与 OpenDAN WebUI 显示与切换提示。
- [x] **N6 文档与手册**：`doc/app_service/rust_app_loader.md`、`doc/app_service/paios容器需求.md`（“Agent 默认在容器边界内”的口径）、AgentRuntime / Agent State 文档；cmd_skills `buckyos_admin`（诊断：原生 Agent 的进程与日志位置）、`opendan_agent_dev`；研究报告 §5 步骤 ③ 注明触发条件已改为 runtime 选择。
- [x] **N7 DV**（Linux 单 OOD，devtest 为 owner，`owner_agents` 已由 dev 配置种入）：新建 Agent 默认 host → 没有 `buckyos-app-<agent>` 容器、宿主有该实例的 `opendan` 进程、hello 有回复、`host` Session 在宿主以 profile 身份执行；把默认改为 container → 进程结束、容器启动、仍能对话；改回 host 再验一次；重启 node_daemon 后原生进程被重新拉起，进行中的 Turn 按既有恢复机制继续。

## 5. 待确认（用户）——已于 2026-10-25 全部确认，结论见 §0 N1–N4

1. **触发条件**：以“默认 runtime 是 host”为准（建议，本文 §2），还是“启用了 host”即原生。后者会让默认用容器的 Agent 也没有容器可用。
2. **Linux 服务器 OOD 上的进程身份**：原生 Agent Service 以 node_daemon 的身份（root）运行，按“先能力后安全”记为已知风险（建议）；还是以 `owner_agents.run_as` 运行（AgentRoot 等数据目录的属主与权限要随之调整）。Desktop 上 node_daemon 本就是登录用户，两者一致。
3. **原生模式下的 `container` runtime**：投影为不可用（建议）；需要 Linux 容器环境时另行设计“按需容器 runtime”，不在本次范围。
4. **切换即重启**：默认 runtime 在 host ↔ container 间切换会重启 Agent Service（进行中的 Turn 被打断后按恢复机制继续），可以接受。

## 6. 实施记录（2026-10-25，未提交）

按 §0 N0–N4 实施。与 §3 的出入：

- **共享判断** `buckyos_api::native_agent_node(input, owner_user_id, app_id)`（`app_schema.rs`）：§2 三条全满足才返回节点。`AppServiceInstanceConfig.run_mode: AppRunMode`（`app_mgr.rs`，缺省 `container` 不写出）。`executor_auth` 的 profile 派生没有改：它判断的是“这个 Agent 能不能用该节点”，与 Agent Service 跑在哪里无关。
- **调度器**：`create_service_spec_by_app_config` 原生时 `need_container = false`、`node_affinity = node:<id>`（节点项新增 `labels: ["node:<id>"]`，借用既有的 affinity 过滤）；`app::app_run_mode` 给实例写 `run_mode`，`reconcile_app_instance_specs` 在 settings 变化后改写已有实例的 `apps/<id>/run_mode`；`is_spec_changed` 比较 `need_container`。§3.2 的“放置节点不可用时 spec 级不可调度”没有单做：沿用现有调度失败处理（研究报告 G6 仍待做）。
- **node_daemon**：`RuntimeType::NativeAgent`；pid 记录在 `$BUCKYOS_ROOT/local/app_native/<runtime_key>.json`（pid + 进程启动时间，防 pid 复用），状态不做端口探测；opendan 自己的日志在 `logs/opendan/opendan.<pid>.log`（不是 §3.3 写的 `logs/apps/<instance>/`）。进程管理复用 `service_pkg.rs` 的 `spawn_detached / stop_process_tree / read_process_identity`。
- **libopendan / opendan**：`builtin_runtimes(.., native_service)`；原生时 `container` 记录 `exec = null`、`capabilities = {native_service: true}`、`default = false`，`observe` 给出原因（含“把容器设为默认可回到容器”）。`OPENDAN_SERVICE_MODE=native` 由 Loader 读取；`package_root` 兜底用 `get_buckyos_root_dir()/bin`。
- **control-panel**（N2/N3 的创建默认值）：`agent_runtimes.rs` `container_available(kv)`（有 `support_container` 的 ood/server 设备）、`host_by_default = host 开放 && (host.os != linux || 没有容器节点)`；`apply_create_defaults` 只在 `host_by_default` 时启用 host 并设为默认；`agent.runtime_directory` 多返回 `os / container_available / host_by_default / agent_service {mode, node_id}`；没有容器节点时把默认设为 `container` 被拒（`container_unavailable`）。
- **Desktop 创建向导**：权限页“Agent 运行位置”两张选项卡（宿主节点 / Agent 容器），只在 host 开放且有容器节点时出现；草稿 `hostRuntimeEnabled: boolean | null`（null = 跟机器默认）；`model.ts` `canChooseRuntime / runsOnHost`；确认页文案改为“Agent 服务在该节点原生运行，不使用容器”。OpenDAN WebUI 的运行环境卡片原本就能“设为默认”，只补了切换会重启 Agent 服务的说明；Desktop Agent 详情页没有切换入口，未加。
- **文档**：`doc/app_service/rust_app_loader.md` §9.3.1、`paios容器需求.md` §8.2、`Agent State Protocol.md` §10、`AgentRuntime 设计.md`、cmd_skills `buckyos_admin` / `opendan_agent_dev`。

**验证**：`cargo test -p buckyos-api native_agent_node`、`-p scheduler`（含 `an_owner_agent_on_its_host_runtime_runs_natively`）、`-p node_daemon test_app_loader`（含 `a_native_agent_*` 两项，假 opendan 脚本）、`-p libopendan --test runtimes`（含 `a_natively_run_agent_service_has_no_container_runtime`、`a_node_runtime_refused_at_start_is_used_once_its_executor_answers`）、`-p control_panel`（含 `linux_hosts_default_to_the_container_and_the_owner_chooses`）；Desktop `pnpm check`、`deno test --config tests/datamodel/messagehub.deno.json --no-check -A tests/datamodel/agent-setup.test.ts`、`npx playwright test tests/e2e/pages/agent-setup.spec.ts`（18 项，含 Linux 默认容器、无容器节点两项）。

**DV（2026-10-25，本机单 OOD，devtest 为 zone owner，Jarvis 默认 runtime 已是 `host`，`nodes/ood1/settings` 的 `owner_agents` run_as 65534）**：

- 部署：`uv run buckyos-build.py -s node_daemon scheduler control_panel opendan agent_tool xagent opendan_web desktop`、`./build_aios --local-test`、`uv run start.py`（**不能**带 `--skip-update`：它不安装新编出的宿主二进制，第一次就因此还在跑旧调度器）。
- 原生启动：调度器 `need_container=false`、节点配置 `run_mode: native`；没有 `buckyos-app-jarvis` 容器，宿主进程 `opendan --appid jarvis.test.buckyos.io --agent-bin bin/pkgs/all.agent.jarvis…/<hash>.pkg`，记录 `local/app_native/<key>.json`；Loader 日志 `runtimes container=RuntimeUnavailable, host=Available (default)`；`agent.runtime_directory` 返回 `agent_service {mode: native, node_id: ood1}`、`host_by_default: false`（Linux）。
- 对话：hello 4.1 s 回复；“运行 id -u 和 pwd” → `65534` 与宿主 `host-runtime/.opendan/<agent>/sessions/<sid>`（命令以 profile 身份在宿主执行，不是 root）。
- 切换：`agent.update {default_runtime_id: container}` 9 s 内原生进程结束、记录删除、容器起来（`container=Available (default)`），对话正常；改回 `host` 3 s 内容器删掉、原生进程起来，对话正常。
- 只重启 node_daemon：原生进程不受影响、被新 node_daemon 按记录认领（同一 pid，未重复启动），之后对话正常。
- DV 修掉的问题：Loader 启动时恢复的两个 UI Session 与 Goal Session 同时 bind，节点执行器默认 `max_concurrency = 2`，总有一份写拿到 `Busy` → 三个 bind 每 5 s 同步失败（`no free execution slot`）。`agent_tool` 的 remote_node 客户端改为对 `Busy` 退避重提（见 [llm_context TODO](llm-context-agent-runtime-todo.md) §11.3），之后 0 次。
- 已知现象：开机时 opendan 可能比网关早起来，登录 `127.0.0.1:3180` 失败就退出，node_daemon 下一轮再拉起，约半分钟后稳定（容器模式下同样依赖网关，只是容器启动慢掩盖了）。没有改。
- 未验：Windows / macOS（Desktop 登录用户身份）、新建 Agent 的创建默认值只做了单测与 e2e（DV 上没有新建 Agent）。

