# AGENTS

当前工程版本由 `src/VERSION`、`src/Cargo.toml` 和 `src/bucky_project.yaml` 维护，目前均为 `0.7.0`。默认处于允许 breaking change 的开发阶段，除非任务明确要求，不做旧版本兼容实现。

## BuckyOS 开发常用命令

### 环境与构建入口

- Rust workspace 位于 `src/Cargo.toml`，Cargo 命令在 `src/` 下执行；在仓库根目录执行时使用 `--manifest-path src/Cargo.toml`。
- Python 开发脚本使用 `uv run`，依赖由根目录 `pyproject.toml` 管理，要求 Python 3.12+，通过 Git `main` 分支引入 `buckyos-devkit`。
- 完整构建需要 Rust、当前平台的 C/C++ 构建工具、Node.js/npm、pnpm 和 Deno；运行容器应用需要 Docker。环境准备入口见 `README_zhCN.md` 和 `devenv.py`。
- `src/buckyos-build.py` 是仓库构建入口：先准备 SDK/CLI 分发包，再调用 devkit 的 `buckyos-build`。直接运行 devkit 命令会绕过这一步。
- 同级 `buckyos-websdk` 仓库存在时从源码构建 SDK/CLI；不存在时使用 npm 发布的 `buckyos@latest`。本地源码构建失败会中止，不回退到发布包。可通过 `BUCKYOS_SDK_TOOL_SOURCE` 指定源码位置、`BUCKYOS_SDK_TOOL_DENO` 指定 Deno。
- `--skip-web` 和 `-s` 构建也会准备 SDK/CLI。使用预构建分发包时，`BUCKYOS_SDK_TOOL_ARTIFACT`、`BUCKYOS_SDK_TOOL_RELEASE_MANIFEST`、`BUCKYOS_SDK_TOOL_DENO`、`BUCKYOS_SDK_TOOL_SBOM` 四项必须齐全；单独设置 Deno 路径不会进入预构建模式。具体判断以 `src/buckyos-build.py` 为准。

### 开发命令

**以下命令均在 `buckyos/src/` 下运行。**

```bash
# 构建并汇总产物到 rootfs，不更新已安装的运行环境
uv run buckyos-build.py
# 指定构建模块，名称取自 bucky_project.yaml 的 modules
uv run buckyos-build.py -s scheduler
# 跳过 Web 模块，仍会准备 SDK/CLI
uv run buckyos-build.py --skip-web
# 停止现有系统、更新安装文件，再启动 node_daemon；不会自动 build
uv run start.py
# 只重启，跳过安装文件更新
uv run start.py --skip-update
# 全新安装，生成 dev 配置并启动（devtest / test.buckyos.io）
uv run start.py --all
# 全新安装，生成 release 配置并启动，进入待激活状态
uv run start.py --reinstall release
# 检查激活状态、进程、端口和日志
uv run check.py
# 在已有 Zone 中以前台宿主机进程调试某个已创建的 Agent（默认 owner 为 devtest，默认取其名下第一个 ready 的 Agent）
./debug_jarvis.sh devtest --agent <agent_id>
# 停止 BuckyOS 进程和相关应用容器
uv run stop.py
```

`start.py` 默认要求 `rootfs/libexec/buckyos-tool/` 中已有构建生成的分发包。`--all` 与 `--reinstall` 走全新安装流程，清理范围由 `bucky_project.yaml` 的模块映射和 `clean_paths` 决定，不能当成普通重启；`data/home/`、`data/srv/`、`storage/` 属于保留数据。`start.py` 会调用 `stop.py`：后者按进程名停止服务、删除 `buckyos-app-*` 容器，并在非 Windows 平台停止 `devtest-*` 容器。

`cyfs-gateway` 需要单独构建和安装；仅构建本仓库不会生成网关可执行文件。以下命令从 `buckyos/src/` 出发，在同级网关仓库中运行（Bash）：

```bash
(
  cd ../../cyfs-gateway/src &&
  uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main" buckyos-build &&
  uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main" buckyos-install
)
```

`debug_jarvis.sh` 会构建 `opendan`、`libopendan`、`agent_tool_cli_dev`，停止该 Agent 的构造 App 容器并占用其服务端口；退出后由 node-daemon 恢复容器。Agent 包读取 `src/apps/jarvis_agent/agent/`。更多参数和独立 `--dev` 模式见 `src/frame/opendan/README.md`。

### paios 镜像的本地开发迭代

涉及容器内的 OpenDAN、`agent_tool`、`xagent`、OpenDAN WebUI、Worker 工具或入口脚本时，需要重建 `paios/aios` 镜像。以下命令在 **`buckyos/` 仓库根目录**运行：

```bash
./build_aios --local-test
uv run src/start.py --skip-update
```

`--local-test` 默认只构建本机架构，使用 `local/aios-test` 镜像仓库名，复用 Cargo 构建产物且不推送镜像；构建成功后为已安装的 BuckyOS 写入 `$BUCKYOS_ROOT/etc/devenv.json` 的 `aios` 覆盖（未设置 `BUCKYOS_ROOT` 时为 `/opt/buckyos`）。随后重启会重建应用容器，使其使用新镜像；每轮修改重复上述命令即可。需要同步更新宿主机服务时，先构建对应模块，再运行不带 `--skip-update` 的 `start.py`。

区分两份本地配置：仓库根目录的 `devenv.json` 为普通 `build_aios` / `build_exttool` 提供默认镜像仓库名；node-daemon 实际读取的是 `$BUCKYOS_ROOT/etc/devenv.json`，例如 `{"aios": "local/aios-test", "exttool": "local/exttool-test"}`。字段填写不带 tag 的仓库名，运行时自动追加 `:latest-amd64` 或 `:latest-aarch64`。这些是个人开发配置，通常不提交到 Git。`start.py --all` 会保留含本地镜像覆盖的运行时 `devenv.json`，并在命令末尾提示；要恢复默认的 `paios/aios` / `paios/exttool`，手动删除对应覆盖键后重启。

修改 ExtTool 内容时，使用 `./build_exttool --image local/exttool-test --arch amd64`（ARM64 使用 `aarch64`），并在运行时 `devenv.json` 设置 `exttool`。已有的 `buckyos-exttool` Docker 卷不会因镜像重建而自动刷新；需要停止相关容器、确认卷内容可丢弃后移除该卷，再启动以重新填充。

### 测试与验证

**以下命令在 `buckyos/src/` 下运行。**

```bash
# Rust workspace 测试，与 .github/workflows/dev-tests.yml 的单测入口一致
cargo test -- --test-threads=1
# 只验证某个 package；名称取自对应 Cargo.toml，而非目录名或构建模块名
cargo test -p buckyos-api -- --test-threads=1
# 构建/安装/启动脚本及 SDK/CLI 分发包的 Python 测试
uv run python -m unittest discover -s tools/tests -p 'test_*.py'
```

**以下 DV/集成测试入口在仓库根目录运行。**

```bash
# 运行依赖本机 Zone 的测试前，确认已激活且服务可用
uv run src/check.py
# 列出 runner 能识别的测试模块
uv run test/run.py --list
# 执行指定测试模块（可重复传入 -p）
uv run test/run.py -p workflow_test
```

`test/run.py` 按目录中的 `package.json`、`Cargo.toml`、`main.py`、`test_*.py` 或 shell 脚本选择入口，不会自动启动 Zone，也不会发现所有嵌套测试。Node 模块会执行 `pnpm install`，若依赖 `buckyos` 还会执行 `pnpm update buckyos`，随后运行 `pnpm test`；注意其依赖和锁文件可能变化。具体服务、账号、外部 provider 等前置条件应阅读测试目录的 README 和配置。

`src/test/` 并非全部是脱离 DV 环境的单测。例如 `test_rbac` 的运行入口是 `cargo run -p test_rbac`，需要在线系统；`src/test/run_rpc_tests.sh` 会先检查相关端口。

WebUI 在各自目录运行 `pnpm install`、`pnpm build`。Desktop 和 OpenDAN WebUI 还提供 `pnpm check`、`pnpm lint`、`pnpm test:e2e`；实际可用命令以对应 `package.json` 为准。仓库默认忽略 `pnpm-lock.yaml`，不能假定新 checkout 已有锁文件。

## 目录地图

| 路径 | 当前用途 |
| --- | --- |
| `src/Cargo.toml` | Rust workspace 成员、共享依赖与本地 patch；并非 `src/` 下所有 Rust 项目都属于 workspace。 |
| `src/kernel/` | 内核服务与库，包括 `node_daemon`、`sys_config_service`、`verify_hub`、`scheduler`、`workflow`、`task_manager`、`kmsg`、`kevent` 等。 |
| `src/kernel/buckyos-api/` | Rust runtime、服务客户端、共享协议与类型；修改跨服务接口时优先检查这里。 |
| `src/frame/` | 系统服务与 Agent 基础设施，包括 `control_panel`、`aicc`、`msg_center`、`repo_service`、`nfs_server`、`aiworkspace`、`opendan` 等。 |
| `src/frame/lib_opendan/` | Cargo package 为 `libopendan`，承载 Agent State、Session 驱动和 `xagent`；与 `llm_context`、`agent_tool` 协作。 |
| `src/frame/desktop/` | Desktop 和控制面板 WebUI，构建后安装到 `bin/control-panel/web/`。 |
| `src/frame/opendan/web/`、`src/kernel/node_active/` | OpenDAN WebUI、节点激活 WebUI。 |
| `src/apps/` | 内置应用包：`sys_test` 与 `jarvis_agent`；`jarvis_agent` 是系统内置的 Agent 模板（登记在 `system/install_settings.agent_templates`），配置、提示词和行为模板在 `jarvis_agent/agent/`。 |
| `src/rootfs/` | 部署模板与构建产物汇总目录；修改前区分手写配置、脚本与生成的二进制/Web/SDK 产物。 |
| `src/bucky_project.yaml` | devkit 的构建模块、安装映射、数据保留/清理规则和发布配置。 |
| `src/tools/` | SDK/CLI 分发包构建脚本、脚本测试、Agent 辅助工具等；系统 `buckyos` CLI 分发到 `rootfs/libexec/buckyos-tool/`。 |
| `src/make_config.ts`、`src/devenv_config.ts` | 本地开发/发布配置生成与开发环境种子事实；VM 配置和模板另见 `src/dev_configs/`。 |
| `src/test/`、`test/` | Rust 测试程序/fixtures/网关测试，以及仓库级 DV、集成测试；按各入口判断环境依赖。 |
| `.github/workflows/`、`publish/` | CI 构建与验证流程、镜像和发布资源。 |
| `doc/arch/`、`doc/sdk/`、`doc/<module_name>/` | 架构、SDK/runtime 使用说明与模块设计；文档可能滞后，当前行为以代码为准。 |
| `product/`、`proposals/`、`notepads/` | 产品需求、开发任务 Spec/分工、临时工作记录；不直接代表已实现能力。 |
| `harness/` | Agent 协作流程；先查 `README.md`，再按任务选择 `SKILLS/`、`process_rules/`、`checklists/` 和 `trigger_rules.md`。 |

上述目录中如果存在 README，应优先阅读以了解更具体的用途，再核对代码。

### BuckyOS 运行时目录 `$BUCKYOS_ROOT`

优先使用 `BUCKYOS_ROOT` 环境变量；默认是 `/opt/buckyos/`，Windows 开发脚本默认使用 `%APPDATA%\buckyos`。排查时以 `check.py` 输出和实际进程环境为准。

- **`logs/`**：服务日志；**`etc/`**：配置、`node_identity.json` 等；**`security/`**：私钥、keyref 等身份材料。
- **`data/home/`**：用户数据；**`data/srv/`**：服务持久数据及 Zone 共享数据；**`storage/`**：内核持久存储。
- **`data/var/`**、**`data/cache/`**、**`local/`**：服务运行数据、缓存、本机服务数据，生命周期与持久用户数据不同。
- **`data/home/<owner_user_id>/.local/share/<app_id>/agents/<agent_id>/`**：宿主机上的 OpenDAN AgentRootFS。每个 Agent 基于模板构造一个独立 App，其 AppId 等于 AgentId（如 `xiaobai.test.buckyos.io`），AppInstanceId 为 `<agent_id>@<owner>`；AgentId 从实际 `AgentSpec` 读取，不能用短名代替。

路径与容器映射详见 `doc/path_usage.md`，Agent 路径装配见 `src/frame/opendan/src/main.rs`。诊断先运行 `check.py`：未激活时重点检查 `3182`，已激活时检查网关 `80/3180`、system-config `3200`、verify-hub `3300`、control-panel `4020`。脚本退出码为 0 也可能只是 `Activation Ready`，执行 DV 前还需确认输出中的激活状态和服务状态。

## 通用处理原则

- 默认处于开发阶段，除非明确要求，不要做任何向下兼容实现
- 开始时用具体文字说明任务理解和修改范围。
- 优先使用任务需求中明确要求的skill,如果没指定，查找skills的路径要优先使用`harness/SKILLS`。
- 组合优于发明，总是选择新增代码更少的实现方式：优先复用已有组件、类型、脚本、依赖和既有模式。
- 引入新的依赖项或通用组件时，必须和用户确认。
- BuckyOS 家族仓库统一遵循此规则：所有从 GitHub Git 仓库引入的源码或包依赖，必须显式使用分支名；当前统一使用 `main`。Cargo/uv 依赖写 `branch = "main"`，pip/uvx Git URL 写 `@main`，npm Git URL 写 `#main`，克隆依赖仓库时指定 `--branch main`。依赖声明禁止使用 `rev`、提交 SHA 或 `tag` 固定版本，也不要省略分支。
- 修改 GitHub Git 依赖时，同步检查直接依赖、家族仓库之间的传递依赖、构建脚本、CI、测试和文档中的安装命令，避免同一 crate 因来源不同而产生类型冲突。锁文件由包管理器生成的提交 SHA 可以保留，不要手工删除或改写。
- 修改范围尽量小，不修改任务范围外的文件，不覆盖已有的无关改动。
- 只通过分析当前仓库文件来确定“当前实现”；以源码、manifest、脚本和测试为依据，不把产品规划或历史笔记当作已实现能力。
- 当来自不同源的信息冲突时，更相信新的信息。git commit的时间比文件修改时间更可靠。
- 直接修改代码！除非明确要求，不做兼容性处理，不添加注释。
- 不要破坏基础系统的可用性：代码改动应保持 `cargo test`、`buckyos-build.py` 可通过。先运行与改动相关的检查，再按影响范围验证构建和 DV；纯文档修改核对路径、命令和差异即可。未执行的检查必须说明，不能当成已通过。
- 改协议、字段、命名、存储结构时，必须检查服务实现、`buckyos-api` 共享类型、SDK 消费方、前后端和文档是否联动。
- 新增或调整服务时，同时检查 `src/Cargo.toml`、`src/bucky_project.yaml`、`src/rootfs/` 和 scheduler 的接入；具体流程见 `harness/Service Dev Loop.md` 与 `harness/trigger_rules.md`。

### 完成任务后，至少应能回答：

- 改了什么
- 为什么这样改
- 跑了什么验证
- 还有什么风险或未验证项

如果是较大任务，还应说明：

- 主要改动入口文件
- 是否影响文档、协议、共享类型、数据结构、第三方依赖

## cmd_skills 联动维护

cmd_skills 是 Agent 完成一类事的命令手册（加可选命令），是对应功能的交付物，不是一次性文档。机制与写法见 [OpenDAN Agent 开发指南](doc/sdk/OpenDAN_Agent_Dev_Guide.md) §2.3–2.4。

- **位置**：内置手册源码在 `src/apps/jarvis_agent/agent/cmd_skills/<name>.md`（随 Jarvis 的 `agent` 子包分发）；front matter 的 `name` / `description` / `requires` 是发现索引，由 `agent-session skill list`（`src/frame/lib_opendan/src/skills.rs`）输出。选择说明在 Jarvis 的 `behaviors/{analyze_intent,goal,plan}.toml`，UI 默认手册在 `agent.toml` 的 `default_cmd_skills`；冻结在 `src/frame/lib_opendan/src/state/behaviors.rs`，装配在 `runner/assembler.rs`（`## Commands`），命令 PATH 在 `runtime/mod.rs`（`tools/<skill>/bin`）。skill 包样例在 `test/skills_pikg_sample/`。
- **工具实现原则**：访问 BuckyOS 的 Agent 工具优先用 TypeScript 基于 buckyos-websdk 编写，源码随 runtime 可读、Agent 可据此派生。人也会用的系统、App、发布命令放 websdk `cli/modules`（`buckyos` 命令唯一源码）；Agent 专用的配方类工具（AIGC、媒体合成）放 `src/tools/buckyos-agent`；某个 skill 自带的命令放 skill 包的 `tools/<skill>/bin`。xllm 等 runtime 层保留 Rust。
- **同一变更内检查手册**：修改用户可见的 CLI、API、配置、运行路径或发布流程时，按下表找出受影响的手册，在同一变更中更新手册与示例；判断不受影响时写明依据，不能只因代码编译通过就跳过。跨仓库（websdk、devkit、cyfs-gateway）的实现变化也要检查 BuckyOS 分发的手册。手册指向当前源码或命令 schema（`--help`、`buckyos command describe`），不复制会过期的接口说明。

| 实现变更 | 检查的手册与验证 |
| --- | --- |
| system-config、Control Panel、身份/RBAC、scheduler/node-daemon、日志诊断、路径及版本信息、容器内 Tool 挂载与 launcher | `buckyos_admin`：设置生效、权限失败、诊断与源码版本核验、容器内 `buckyos` 可用 |
| websdk SDK API、`buckyos-agent` 的 `commands/` 范本与 `lib/` | 引用这些源码的手册（`aigc_*`、`buckyos_admin`、`opendan_agent_dev`）：范本仍可照抄运行，派生工具示例可执行 |
| AICC 协议、provider 能力、媒体 CLI、Task/结果/下载和消息附件 | `aigc_image`、`aigc_audio`、`aigc_video`：真实生成、长任务恢复、交付可访问 |
| xllm 参数、模型路由、工具开关、上下文/behavior 协议 | `xllm`：示例调用、结构化结果、上下文与执行边界 |
| SDK/Tool、PIKG/AppDoc、devkit、Installer/Repo/BNS、系统/镜像发布 | `buckyos_dev_release`、`buckyos_app_dev` 及相关场景：构建、包验证、测试安装、发布回读 |
| AIWorkspace schema/RPC、对象/资源、renderer、Wish/Block 执行器、模板格式 | `aiworkspace_template_dev`（暂缓，恢复后生效）：新建、导出导入、引用和运行 |
| skill 安装 kRPC、PIKG 的 `agent_skills` 子包、AgentRoot 的 `cmd_skills/` / `goal_skills/` / `tools/` 布局 | `opendan_agent_dev` 及所有内置手册的打包位置：安装、列表、选择、删除 |
| AgentRootFS、Loader、Session/Behavior、cmd_skill 冻结/装配、PATH/helper、模板和扩展分发 | `opendan_agent_dev`、`ui_core` 及受影响的专项手册：发现/选择/执行、冻结与升级 |

- **完成条件**：提交说明列出受影响的 skills、修改或无需修改的依据、验证命令与结果、尚未实测的 provider / 平台。删除失效命令，不用兼容说明掩盖当前版本不可用；同步生成来源与分发产物，不能只改打包副本。
- **回归**：手册纯文字改动核对路径、命令和链接；CLI、协议、runtime 或打包变更运行相关用例（`cargo test -p libopendan --test xagent_lib work_sessions_take`、`--test xagent_cli skills_are`、`cargo test -p opendan`）和最小场景。真实生成、发布等需要环境的测试如实标为未验证，不能用 mock 通过替代。

## 常用术语与领域知识

- **Zone**：BuckyOS 为了区别于传统“集群/云”而使用的术语，指用户拥有的逻辑云/集群，是系统管理和调度的基本范围。
- **OOD (Owner Online Device)**：Zone 内的核心节点形态，承载 `system-config`、`scheduler` 等关键能力；单 OOD 或 `2n+1` OOD 都属于同一套模型。
- **Node**：Zone 内的一台设备节点。OOD 也是一种特殊 Node，普通 Node 主要负责运行应用和系统服务。
- **ZoneGateway**：Zone 的对外访问入口，负责公网访问、HTTPS/TLS 终止、子域名路由和暴露策略控制。
- **NodeGateway**：每台节点本地的统一网关能力，通常就是本机 `cyfs-gateway`，常见一致入口为 `127.0.0.1:3180`。（docker内不同)
- **SN (Super Node)**：公网协助节点，为 Zone 提供 DDNS、证书挑战和转发/中继能力。
- **OpenDAN**：Agent runtime 基础设施；当前 `opendan` 是 Agent Loader，一个进程托管一个 Agent，核心 Session/State 能力在 `libopendan` 等库中。Zone 内以 AppService 身份运行，支持容器和宿主机调试；独立 `--dev` 模式使用本地文件队列。
- **Jarvis**：系统内置的 Agent 模板（RootFS 模板），包在 `src/apps/jarvis_agent/`。系统不预建 Agent 身份；用户通过 Add Agent 向导（桌面 Jarvis 引导入口或 Users and Agents）为自己创建 Agent，control_panel 基于模板为它构造独立 App，由 OpenDAN 加载。
- **App / AppInstance / Agent**：AppId 标识应用，AppInstanceId 标识某用户的应用实例；Agent 拥有独立 DID/AgentId，通过 `AgentSpec.binding` 绑定 runtime 的 AppInstanceId 与 service name，不能把 Agent 身份等同于 App 身份。共享定义见 `src/kernel/buckyos-api/src/app_schema.rs`。
- **DID**：BuckyOS 的身份基础设施。仓库里最常见的是 `User(Owner) DID`、`Device DID`、`Zone DID`,`Agent DID` 四类。
  - **Config 在身份语境中的特殊含义**：受 CYFS 历史命名影响，代码里的 `UserConfig`、`DeviceConfig` 往往本质上就是 DID Document，而不是普通运行配置。
- **system-config**：Zone 的 KV 真相源，不只是配置中心；用户、设备、服务、RBAC、调度结果都会写在这里。
- **scheduler**：确定性的状态推导器。它从 `system-config` 读取系统现状，推导出 `node_config`、`service_info`、`rbac`、`gateway_config` 等结果，再写回 `system-config`。
- **node-daemon**：节点收敛器。它读取本机 `node_config`，负责安装、部署、启动、停止、升级，把节点拉到目标状态。
- **verify-hub**：统一登录和 token 签发中心。服务通常依赖它签发的 `session_token` 做身份验证，再结合 RBAC 做授权。
- **kRPC / KRPC**：BuckyOS 系统服务之间最常见的 RPC 方式，很多系统服务都暴露在 `/kapi/<service_name>` 下。
- **Info / Settings / Config / Doc**：仓库里反复使用的数据分类：
  - `Info`：运行时上报信息，通常由上报方写，其它角色只读。
  - `Settings`：用户可调配置，系统不会自动改。
  - `Config`：系统自动构造的运行配置，用户不应手改。
  - `Doc`：带签发人、可验证、通常只读的文档对象，如 `DeviceDoc`、`AppDoc`、`ServiceDoc`。


## 开发 Tips：避免常见错误

- Python 脚本优先复用 `buckyos-devkit` 的跨平台能力，遵循当前脚本的 Windows/Unix 分支。
- 目录名、Cargo package、构建模块名和安装目录名不一定相同：例如 `kernel/sys_config_service` 对应 package/module `system_config`，安装到 `bin/system-config/`；`frame/lib_opendan` 的 package 是 `libopendan`，构建模块 `xagent` 取其可执行产物。
- 修改生成内容时先找到源头：WebUI 源码、SDK 源码、Jarvis Agent 包或配置模板，避免只改 `rootfs`/`dist` 中下次构建会覆盖的副本。
- `src/devenv_config.ts` 的种子定义与 `cyfs-gateway/src/devenv_config.ts` 有同步约定；任务涉及种子时检查该约定及两个消费入口，不能只修改生成配置。
