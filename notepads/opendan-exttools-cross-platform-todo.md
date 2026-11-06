# Agent Runtime 基线工具全平台统一分发 TODO（三层模型，已 Review）

> 2026-11-06，v2.4：owner 裁决 `ext-core` 的发行、钉版与种子流程（§3.10，决策 21–24），T2 拆成 T2a–T2e，T0 收窄；工具清单的事实来源改为 [paios容器需求.md §6.4](../doc/app_service/paios容器需求.md)（owner：机制优先，首版 baseline 可以宽，但每条命令都要先进表）。2026-11-06，v2.3：一期已实施，结果与偏离见 §4.1。2026-11-05，v2.2。v2.2 按 owner 对 §6 的答复改：一期扩到 pikg 结构支持（决策 4 的读法：这次开工至少完成到扩展包的 pikg 结构，P1-6）、macOS x86_64 进首版平台（7）、§3.2 / §3.7 / 决策 8 清掉 v2 残留、补上源码核对出来的四个决策 15–18（期望状态投影、目录归属、`InstallGrant::System`、AppType 与子包键）与两个一期口径 19–20。v2.1 加 §0.3 与 §3.9：owner 从场景出发把 ExtTools 升级成**扩展包体系**（系统管理员为全 Zone 的 Agent 管理共用扩展包，包之间有依赖，勾选即装，长期运维），并据此把安装器定为 node_daemon 的 reconciler（§3.3）、skill 改为只声明依赖的命令（§3.9.3）。v1 针对 [runtime 生命周期 TODO R16](opendan-agent-runtime-lifecycle-todo.md#6-实施任务分期) 细化 R-S1 / R-S3，把 standard 分发、Zone 共享可选包、Desktop Lite 隔离、macOS VM 自初始化都装了进来。v2 按 owner 的**三层模型**（§0）与**隔离讨论**（§0.2）重排：三层是骨架；隔离只有“核心路径钉死”和“Agent 不污染用户环境”两件事，对容器、Desktop Lite、bare Linux host 一套约定；范围拆成三期加两份分出去的 TODO（v2 时第一期只改 Rust，v2.2 扩到 pikg 结构）。尚未实施；以当前仓库源码为依据（§2 的事实已逐条核过）。相关：[skills 体系 TODO §3.6.1 C](opendan-skills-system-todo.md)（S19 等本文第一期）、[path_usage.md](../doc/path_usage.md)、[Session Runtime](<../doc/opendan/OpenDAN Agent-Session-Runtime.md>) §2.1 / §3.2 / §3.3、[Desktop Lite TODO](buckyos-desktop-lite-todo.md)、[native Agent Service TODO](opendan-native-agent-service-todo.md)。

## 0. 三层模型与隔离（owner 2026-11-05 原话要点，本文骨架）

### 0.1 三层

| 层 | 内容 | 谁装 / 谁升级 | 生效范围 | 删掉会怎样 |
| --- | --- | --- | --- | --- |
| **核心（baseline / standard）** | Agent 标准命令（`agent_tool`、`xagent`、`agent-session`、`buckyos`、AICC 与媒体服务命令，`baseline::STANDARD_TOOLS`）及其运行依赖（固定版本 Deno、TS 源码与 vendor、SDK CLI） | 随系统：容器随 aios 镜像，原生随 BuckyOS 安装包。**不能独立升级** | 该机器上所有 Agent | 系统不可用，不存在“删掉”的情形 |
| **扩展（ExtTools / extended）** | ffmpeg、ImageMagick、poppler、tesseract、pandoc、python3 / uv、node、deno 等本地加工工具 | zone-owner 装，**可独立升级**；容器是 exttool 卷，原生是 `$BUCKYOS_ROOT/tools/` 下的受管目录 | 装在哪台机器上，**那台机器上所有 Agent 都有效** | 只丢 `extended`，核心照旧（核心不依赖扩展层的任何二进制，§3.4） |
| **Agent 自装（skill 命令）** | `tools/<skill>/bin`、skill 的 `install.toml` 装的东西、uv / npm 项目依赖 | Agent 自己，受 `grant.install` 约束 | 只对这个 Agent | 删掉 Agent 的持久卷，**核心能力不受影响** |

“保障 Agent 私有 runtime 的 baseline 在所有环境下可用”是第一层的唯一目标；第二层允许缺、允许旧；第三层随 Agent 走。

### 0.2 隔离：只有两件事，三种形态一套约定

owner 允许 Linux 下不用容器、把自己的 host 完整给自己的 Agent 做私有 runtime（先能力再安全）。这时“Agent 用的 python / deno 是哪一个”要分三种命令回答，而且答案今天就已经不一样：

| 命令 | 从哪取 | 现状 |
| --- | --- | --- |
| 核心工具的依赖（Deno、TS 源码、SDK） | **永远不走 PATH**：launcher 固定到 `$BUCKYOS_ROOT/libexec/buckyos-tool/runtime/deno` 等绝对路径 | ✅ `baseline::run_deno_command(root, …)`；exttool 卷只带 deno-cache 不带 deno 二进制 |
| 扩展工具（python3、ffmpeg…） | **按 PATH 顺序**：BuckyOS 发起的进程前置 `$BUCKYOS_ROOT/tools/bin`，装了 ExtTools 用它的，没装落到系统的 | ✅ 容器 `entrypoint.sh` 前置；❌ host 执行器未前置 |
| 项目里的 Python / Node 版本 | 由 `uv` / 项目锁决定，解释器装进 Agent 自己的缓存；ExtTools 的 python3 只服务临时脚本 | 约定 |

所以需要的隔离只有两条，容器、Desktop Lite（macOS / Windows）、bare Linux host 共用，实现点都在执行器构造子进程环境的地方：

1. **用户环境不污染 Agent 核心**：核心路径钉死（上表第一行）。这是唯一真正要紧的隔离，它是“路径钉死”不是沙箱。
2. **Agent 不污染用户环境**：写盘纪律 + 环境变量。自装只写 `<agent_root>/tools` 与 instance 目录；`DENO_DIR`、`UV_CACHE_DIR`、`npm_config_prefix` 一类指到 Agent 自己的缓存；不写 shell profile、全局 PATH、系统 site-packages；**不改 `HOME`**。

推论：

- 扩展层的失败模式是“某个命令用不了，一串 skill 不可用”，所以它的检查可以宽：用户 PATH 里碰巧有 ffmpeg 让 skill 能跑，在“先能力再安全”下不是坏事，只要 `tools_check` 记下**命中的路径与版本**，让 `extended` 是 ExtTools 提供还是系统碰巧有能分辨。核心的检查必须严：命中路径不在 `$BUCKYOS_ROOT` 下就是 standard 不完整。
- 文件访问、凭据这类安全隔离 PATH 做不到；owner 选择给整台机器就是接受这一点，本文不宣称做到了。要强隔离另选容器 / VM / 独立账户。
- bare Linux host 作私有与 Desktop Lite 同一账户、同一安装根，**用同一个原生安装器**：三个 OS 一个安装器，不再有“Lite 专属”，也不再需要 v1 的“固定用途特权 worker”（§3.3）。

### 0.3 扩展包场景（owner 2026-11-05 补充）

1. 系统管理员管理 BuckyOS 上**所有 Agent 共用的扩展包**；ExtTools 核心只是其中一个包。包之间允许依赖（装 B 必须先装 A）。在管理页勾选新包后**自动触发下载安装**。这是长期运维工作，机制要设计好。
2. 四个要回答的问题：扩展包列表怎么定义、一个包是什么格式（pikg？）；有了具体需求后怎么构建和发布一个包；包装好并对 Agent 生效后 Agent 多了一组命令，skill 的状态怎么看——反过来，skill 只说明自己依赖的命令，系统凭对扩展包的知识反推“装哪个包能让这个 skill 可用”；skill 可用不完全依赖扩展包，推荐值不 block，用户可以在自己的 host 上用自己的办法装。
3. 答案在 §3.9；对 v2 的修正：安装器不在 Agent Service 里而是 node_daemon 的 reconciler（§3.3），`EXTENDED_TOOLS` 是“核心扩展包”的 `provides` 在 Rust 里的镜像（§3.1），skill 加机器可读的 `commands`（§3.9.3，改 skills TODO 待裁决 14 的答案）。

## 1. 目标与范围

三期，加两份分出去的 TODO：

| 期 | 交付 | 依赖 |
| --- | --- | --- |
| **一期（Rust + pikg 结构，现在可开工）** | Rust：`EXTENDED_TOOLS` 清单；`check_baseline` 按声明选集合；`tools_check` 记命中路径与版本；`xagent runtime baseline --level`；plan 全表显示扩展层状态；host 执行器前置 `tools/bin`；回写 runtime TODO R-S1 / R-S3。pikg 结构（决策 4）：`AppType::ExtTools` 与 `ext-manifest.json` 进 buckyos-api 与 websdk `pikg init / doctor`，`ext-core` 按此格式重打并能 `app publish` 到 Repo，control panel 对 `ext_tools` 的提交路由只收发布、拒绝当 App 装 | 无。解除 skills S19（ExtTools 命令拒装）与 plan 全表的依赖；二期的包体系直接在此格式上建 |
| **二期：期望状态 + 安装器** | Zone 级期望状态与节点投影（§3.3）、node_daemon reconciler 在容器节点 / bare Linux / macOS / Windows 上安装、升级、修复到 `$BUCKYOS_ROOT/tools/`；管理页勾选即装；skill 的命令依赖 → 推荐包；BuckyOS 发起的进程环境约定（§3.6）落到三种形态 | 一期；T0 可行性 |
| **三期：standard 分发（= runtime 裁决 22 的“安装包改造”）** | Agent 标准工具的产物、安装、升级、修复三平台统一；host / aios 共用组装器 | 二期的安装器与环境约定 |
| 分出去 | Zone 共享可选包（v1 §3.6 / E1）→ 新 TODO；macOS VM 由 Agent 自初始化（v1 §3.8 / E3）→ 新 TODO。两者与 R16 没有实现依赖 | — |

保留的约束：`extended ⊇ standard`；owner 的 baseline 声明与实际探测分离，安装成功不自动改声明，不改已冻结 Session；`agent-session` 属于 standard，Desktop Lite 与 bare host 也装（runtime 裁决 22）；form B daemon 设备不支持安装；没有受管 BuckyOS 安装根的 SSH 目标拒绝安装、仍可探测。

与 runtime 裁决 22 的关系：三期就是它说的“安装包改造”，但本文只覆盖 Agent 两层工具的构建 / 安装 / 升级链路，不做其它服务的全面安装包重构；22 里“综合各基础服务需求统一调整”的那部分由安装包 TODO 另起，本文三期作为它的输入之一。

## 2. 现有实现与缺口（2026-11-05 核对）

| 位置 | 已有 | 本文要补 |
| --- | --- | --- |
| `lib_opendan/src/baseline.rs` | `STANDARD_TOOLS`（31 条，含 `session_helper_of`）、`required_commands(overlay)`、`missing_probe` / `missing_locally`、host 入口 `install_entries` 硬链接或复制、`run_deno_command(root, …)` 钉死 Deno、`session_identity_env`、`AGENT_TOOLS_BIN = bin/agent-tools`、`AGENT_TOOLS_LIBEXEC = libexec/buckyos-agent` | `EXTENDED_TOOLS`；探测返回命中路径与版本 |
| `lib_opendan/src/runtime/mod.rs` | `check_baseline` 只查 standard（`required_commands(record.is_private())`）；`check_commands` → `ToolsCheck {present, missing}` 只记名字；`runtime_exec / runtime_put / runtime_op`，私有侧前置 `<agent_root>/tools`；`runtimes_view` 已渲染 `tools_check.present` 与 `usage.skill_commands` | 按声明选集合；`ToolsCheck` 加 `resolved: {name: {path, version, provided_by}}`；安装不经 `runtime_exec`（§3.3） |
| `lib_opendan/src/xagent_cli.rs` | `runtime baseline` / `check --commands` / `op`；`task state\|cancel` | `baseline --level`；`ext-tools status\|install\|check` |
| `build_aios` | Ubuntu 24.04；`RUNTIME_PACKAGES`（apt）：`bash curl git jq rsync tree unzip xz-utils python3`（系统 3.12，uv 用它）、`ffmpeg`、`imagemagick`（**IM6**，入口 `convert / identify / compare`，没有 `magick`）、`poppler-utils`、`tesseract-ocr`；Node 官方 tarball（`NODE_VERSION`）；Deno `v2.9.2` 在镜像里；pandoc 不在镜像。官方清单与场景的事实来源是 [paios容器需求.md §6.4](../doc/app_service/paios容器需求.md) | 扩展层版本来源与 `EXTENDED_TOOLS` 一致性测试 |
| `publish/exttool/`（`build_exttool`） | ubuntu builder → busybox seed；`deno-cache` / `uv-cache` 预热（只带缓存不带 deno 二进制）；pandoc 3.12.1 固定 SHA（amd64 / arm64）；FreeCAD 可选，缺省 `exit 127` 的 stub；卷 `buckyos-exttool-<id12>` 首次挂载时复制 | macOS / Windows / Linux 原生包；锁文件 |
| `publish/aios/entrypoint.sh` | `BUCKYOS_EXTTOOL_DIR=$BUCKYOS_ROOT/tools`，`tools/bin` 前置 PATH | 与 §3.6 的顺序表对齐 |
| `node_executor/src/ops.rs`、`node_daemon/src/node_executor_host.rs` | 执行器构造 PATH / env；启动时 `opendan install-agent-tools` 物化 `bin/agent-tools` 入口 | host 前置 `tools/bin`；环境变量约定；`describe` 报安装根 |
| `src/buckyos-build.py::_prepare_agent_tools`、`src/tools/prepare_sdk_tool_distribution.py` / `build_sdk_tool_distribution.py` | 复制 TS 命令与 vendor 到 `libexec/buckyos-agent`，重写 import；SDK / CLI 分发 | 三期：host / aios 共用组装器 |
| `opendan/src/home.rs`、`service.rs`、`web/src/pages/Workspaces.tsx` | runtime 设置与卡片 | owner 只读 status / check API；卡片显示扩展层状态与管理页链接 |
| `buckyos-api/src/app_doc.rs` | `AppType` 有 `Service / dapp / Web / Agent / Aiws`，按类型校验子包；`Aiws` 是“只装包、不建 AppSpec、不起服务”的先例；`SubPkgList` 有闲置的 `agent_tools` 键（没有消费者） | `AppType::ExtTools` 与校验；`agent_tools` 键删除或归三期（决策 18） |
| `control_panel/src/app_installer.rs`、`aiws_pkg.rs` | `app_submit_route` 按 `(AppType, origin)` 分派，`Aiws` 走 `RegisterAiwsPackage`（控制面板直接交给 aiworkspace 服务，没有节点级收敛） | `ExtTools` 的路由：Rpc 发布到 Repo，Preinstall / AgentRuntime 拒绝 |
| `buckyos-websdk/cli/modules/pikg.ts` | `APP_KINDS = static-web / script / docker / agent-skills / aiws-controls`，`doctor` 已有 | 加 `ext-tools` kind 与 doctor 规则（跨仓库） |
| `node_daemon/src/node_daemon.rs`、`paios_images.rs`、`app_loader.rs` | node_daemon 只读 `nodes/<n>/config \| gateway_config \| gateway_info`，节点目标状态全由 scheduler 推进 `NodeConfig`；paios / exttool 镜像按 manifest pin 校验，`prepare_exttool` 播种卷 | 期望状态经 scheduler 投影成 `NodeConfig.ext_tools`（决策 15）；reconciler 沿 `paios_images.rs` 拆 `ext_tools.rs` |
| `doc/path_usage.md` | `/opt/buckyos/tools/{store,deno-cache,uv-cache}` 只读；`/opt/buckyos/instance/{pkg,cache,tools/bin,tools/<agent_id>/<sid>}` 可写 | 原生落点与容器稳定路径一致（§3.2） |
| Session Runtime 文档 §3.2 / §3.3 | 列了 aios 附加工具与 ExtTool 工具；“`extended` 是任意额外环境”的旧描述 | 改为 `EXTENDED_TOOLS` 核心清单 + 可选 |

`runtime_put` 不等于有写 `$BUCKYOS_ROOT/tools` 的权限；`runtime_exec` 有超时上限且没有持久 task（runtime R-W5 的“超时转 task”未做）。所以 runtime TODO R-S1 原文“经 `runtime_put / runtime_exec` 装到目标”不成立，一期回写。

## 3. 方案

### 3.1 扩展层清单（`EXTENDED_TOOLS`）

**单一来源用仓库现有模式**：Rust 常量 `baseline::EXTENDED_TOOLS` 手写，核心扩展包 `ext-core` 的 `ext-manifest.json`（§3.9.1）`provides` 由单测校验与常量一致，和 `STANDARD_TOOLS` ↔ `commands/*.ts` 的测试同一做法；不做生成器。`extended` 基线 = `ext-core` 已装并核验；其它扩展包不改变 baseline，只改变 `tools_check` 与 skill 状态。锁文件（按 OS / arch 的版本、下载源、SHA256、许可证）只在二期打包时需要，一期不建。

清单的事实来源是 [paios容器需求.md §6.4](../doc/app_service/paios容器需求.md) 表 B（v2.4 起；下表是当时的快照，以 §6.4 为准；ImageMagick 已定 IM7，`magick` 为主入口）。首版核心 = **aios 今天已有的那组**，不扩：

| 能力 | 保证的命令 | 约束 |
| --- | --- | --- |
| 音视频 | `ffmpeg`、`ffprobe` | smoke：最小生成与解析 |
| 图像 | `magick` **或** `convert` + `identify` + `compare` | 首版二选一：手册写 `magick` 优先、回退 `convert`；aios 是 IM6，统一到 IM7 要换 AppImage / 静态构建并改 Session Runtime §3.2 与 `aigc_*` 手册，T0 决定是否做 |
| PDF | `pdftotext`、`pdftoppm`、`pdfinfo` | 携带动态库与数据 |
| OCR | `tesseract` | 核心含 `eng` 与 `osd`；中文数据包列可选 |
| 文档转换 | `pandoc` | `pandoc-lua / server` 不保证 |
| Python | `python3`、`uv` | Linux 容器是系统 3.12；原生包随包；项目解释器由 uv 管 |
| JS / TS | `node`、`npm`、`npx`、`deno` | `deno` 可与核心层共用同一版本文件，但核心 launcher 钉绝对路径，删扩展层不伤核心 |
| 常用辅助 | `git`、`curl`、`jq`、`tree`、`unzip`、`xz` | aios 今天就装着（`RUNTIME_PACKAGES`），所以在 `ext-core` 里；Windows 上随包带 git 的包体与许可证不小，T0 核成本，过高则在 Windows payload 里降为可选并在 `platforms` 注明 |

`bash`、`rsync` 是 Linux 特有，不要求 macOS / Windows 提供；FreeCAD、编译工具链、Docker、pnpm、字体、额外 OCR 数据都是可选（各自另成扩展包），可选项失败不否定核心。Session Runtime 文档说的 `jq` 不在 **standard**（核心层）仍然对：它在扩展层。

清单条目：工具 id、保证的命令名、分类、核心 / 推荐 / 可选、版本探测命令、无网络 smoke。

### 3.2 包与目录

产物 `ext-core-<ver>-<platform>.tar.gz`（五平台同一格式，v2.4 按 §3.10 定；原 `exttools-…tar.zst` / Windows zip 作废），附 `manifest.json`、校验和、许可证。格式以“目标平台无需预装扩展工具即可解包”为准。

**稳定路径三种形态一致**，与 path_usage.md 对齐：

```text
$BUCKYOS_ROOT/tools/
  bin/                          # 稳定入口（容器：卷内；原生：受管安装器维护）
  store/<tool>/...              # 工具本体
  deno-cache/  uv-cache/        # 预热缓存（只读 lower）
  installs/<install_id>.json    # 安装事实，机器级；不含身份凭据
  staging/<version>-<digest>/   # 原生安装的暂存，校验与 smoke 通过后切到 bin/ 与 store/
  current.json                  # 原生：已激活版本与摘要
```

原生先写 `staging/`，校验通过后切换；失败不覆盖旧版。POSIX 可原子切目录；Windows 用版本指针并处理文件占用，不假定 symlink 权限。保留上一版本供恢复。只读包目录与可写缓存分离，缓存走 instance 约定。容器节点同一套：reconciler 在节点级 `buckyos-tools` 卷内做 staging 切换，`ext-core` seed 镜像只在卷为空时播种一次（§3.3），不再按 exttool 镜像 id 一个卷。

### 3.3 安装权限与执行通路

**安装器 = node_daemon 的 reconciler**（v2.1 修正，取代 v2 的“Agent Service 里的串行任务”）。理由：node_daemon 今天已经拥有 `$BUCKYOS_ROOT`（paios 镜像与 exttool 卷的播种、`install-agent-tools` 的入口物化都在它手里），在每种形态里都是安装根的所有者：容器节点上它管 docker 卷；bare Linux host、Desktop Lite（macOS / Windows）上它以安装 BuckyOS 的账户跑；node-daemon 作为系统服务跑的 host 上它就是管理身份。于是 v2 担心的“执行器身份与安装根所有者不同”的权限缺口不存在：**安装从来不经执行器身份**，Agent Service 与 Agent 子进程只读安装事实。

- **入口**：系统管理员在管理页 / CLI 写 Zone 级期望状态（§3.9.2）；每个节点的 node_daemon 对照投影到本机 `NodeConfig.ext_tools` 的期望状态（决策 15）与本机 `tools/installs/`，缺的下载安装、多的（被取消勾选且无依赖者）卸载、版本不符的升级。owner 的 Agent Home / runtime 卡片只显示本机状态与“去管理页勾选”的链接，不直接触发安装。session helper 不开放安装。
- **容器节点**：node_daemon 把包解进节点级卷 `buckyos-tools`（首次由 `ext-core` seed 镜像播种，保留今天的离线首启）；容器只读挂到 `/opt/buckyos/tools`。不再按 exttool 镜像 id 一个卷，不在只读卷内原地更新：reconciler 写卷、容器重启后看到新内容（§3.2 的 staging 切换在卷内做）。
- **原生（bare Linux host、Desktop Lite、node-daemon 服务 host）**：同一套 reconciler 写 `$BUCKYOS_ROOT/tools/`；不可写时在节点状态里报具体权限问题，禁止静默 sudo。
- **form B、普通 SSH 目标**：没有 node_daemon，没有受管安装根，**不参与扩展包**；仍可探测，`tools_check` 报 `provided_by: system`。
- **期望状态到节点的通路（决策 15）**：node_daemon 今天只读本机 `nodes/<n>/config | gateway_config | gateway_info`，节点目标状态一律由 scheduler 推导。扩展包不例外：scheduler 把 `system/ext_tools/desired` 与节点平台相乘，投影成 `NodeConfig.ext_tools: {pkg_id: {version, pikg_digest, payload | unsupported}}`；node_daemon 对照它与本机 `tools/installs/` 收敛，不直读 Zone 级键，RBAC 不加规则；`unsupported` 在 scheduler 侧判定。
- **目录归属（决策 16）**：不持久化 `system/ext_tools/catalog`。control panel 的 Resolve 直接问 Repo 已发布的 `ext_tools` AppDoc（`app install` 的来源解析已有），期望状态记 `{version, pikg_digest}`，reconciler 按 digest 经 Repo / NDM 取包；skill 反推（§3.9.3）要的 `provides` 由同一 Resolve 读 `ext-manifest.json`。Lite 档位有完整 system-config / scheduler / control_panel / Repo（`make_config.ts` 只是 `support_container: false`），这条通路在 Lite 上成立。

reconciler 只接受 Repo 已发布、签名校验过的 pikg（§3.9.1），防路径穿越与符号链接逃逸；离线导入的包验证同一摘要。runtime TODO R-S1 原文“经 `runtime_put / runtime_exec` 装到目标”据此改为“node_daemon reconciler”；R-U3 原语只用于 Agent 自装层（第三层）。

### 3.4 核心路径钉死（第一层的隔离）

- 核心命令的 launcher 用 `$BUCKYOS_ROOT` 相对的**绝对路径**取 Deno、TS 源码、SDK，不查 PATH；现状已是，三期的组装器不得退化成 PATH 查找。
- 核心检查除“命令存在”外再验**命中路径在 `$BUCKYOS_ROOT` 之下**（`bin/agent-tools`、`libexec/`、session overlay）。用户 PATH 里同名的 `xagent` / `buckyos` 不算 standard 就绪，并在 `tools_check.resolved[name].provided_by = system` 里报出来。
- 核心不依赖扩展层：exttool 卷不带 deno 二进制（现状），原生包的 `tools/store/deno` 与核心 `libexec/buckyos-tool/runtime/deno` 可以是同一版本的两份文件，删扩展层不伤核心。

### 3.5 扩展层检查与声明

- `check_baseline` 按声明选集合：`standard` → 标准集合；`extended` → 标准 + `EXTENDED_TOOLS` 核心。后台检查只做有界的存在 / 版本检查；安装后与显式 `ext-tools check --full` 跑完整 smoke。
- `ToolsCheck` 加 `resolved: {name: {path, version, provided_by: buckyos_root | exttools | agent | system}}`；`BaselineCheck` 同样记 standard 命中路径。探测命令在 `missing_probe` 基础上加 `command -v` / `Get-Command` 的路径输出与 `--version`，仍是一条 shell。
- `caps.baseline` 不自动升降；安装状态独立记版本、摘要、平台、阶段、错误、核验时间。可选工具必须真实运行检查，stub（`exit 127`）不计成功。plan 全表每行多一段“ext: ok | partial(missing …) | none | system”，不展开日志。冻结与 skill 匹配按原规则（skills S19 用 `EXTENDED_TOOLS` 拒绝 skill 自装清单内命令）。

### 3.6 BuckyOS 发起的进程环境约定（三种形态一套）

落点：执行器构造子进程环境处（`node_executor ops.rs`、容器 `entrypoint.sh`、`runtime_exec` 的私有分支、session bin overlay）。

PATH 顺序，前者遮蔽后者：

```text
session overlay / <agent_root>/tools/<skill>/bin   # Agent 自装，只对本 Agent
$BUCKYOS_ROOT/tools/bin                            # 扩展层
$BUCKYOS_ROOT/bin/agent-tools                      # 核心入口（其依赖已钉死，不靠此处顺序）
目标机器自己的 PATH                                  # 系统与用户的工具
```

环境变量：`DENO_DIR`、`UV_CACHE_DIR`、`UV_PYTHON_INSTALL_DIR`、`npm_config_prefix`、`npm_config_cache`、`PIP_CACHE_DIR` 指到 Agent 的 instance / cache 目录（容器：`/opt/buckyos/instance/cache/…`；原生：`<agent_root>/.cache/…`，E0 定）；`HOME` / `USERPROFILE` **不改**；不写 shell profile、全局 PATH、系统 site-packages、全局 npm 配置。只影响 BuckyOS 发起的执行，不把私有机器的 PATH 注入远端（runtime M4）。

同名冲突：Agent 自装的命令与核心命令同名时安装拒绝（防遮蔽身份 / 控制入口）；与扩展层同名允许，`tools_check` 报 `provided_by: agent`。

`InstallGrant::System`（`app_schema.rs` 注释“System-wide packages (ExtTools, package managers)”）随本节重定义（决策 17）：Agent 从不装扩展包，`System` 只表示 skill 的 `install.toml` 允许写到 `<agent_root>/tools` 之外、仍不写 `$BUCKYOS_ROOT/tools`（skills S19 的 `writes`）；grant UI 文案与 runtime TODO R-U9 随之改。

“环境不冲突”是目录与进程环境约定，不是 OS 权限隔离；owner 把整台机器给 Agent 时，Agent 在授权内仍可访问用户文件。文档与 UI 文案不得暗示已做到强隔离。

### 3.7 Agent 侧 API、CLI（只读）

```text
xagent runtime ext-tools status <id> [--json]
xagent runtime ext-tools check <id> [--full] [--json]
xagent runtime baseline [--level standard|extended] [--json] [--check]
```

- Agent 侧没有 install：安装由 node_daemon reconciler 做（§3.3），Agent Service、session helper、owner API 都不触发；`status` 读本机 `tools/installs/` 与节点状态，`check --full` 跑完整 smoke。安装状态不进 Agent task（决策 8 作废）。
- owner API（opendan `home.rs` / `service.rs`）加 status / check；CLI 与 UI 共用；`agent-session` 不加。用户级 `buckyos runtime` 命令归 runtime TODO E7；管理动作 `buckyos ext-tools list|enable|disable|status` 在 X4。
- UI：runtime 卡片显示本机扩展层版本 / 状态与“去管理页勾选”的链接；容器显示“随节点卷管理”。E6 搬 Home 后复用。

### 3.9 扩展包体系（回答 §0.3 的四个问题）

#### 3.9.1 扩展包是什么：一种 pikg

用 pikg，不另造格式。理由：签名、DID、Repo 发布、`app install` 的来源解析（本地 `.pikg` / URL / Catalog DID）、按 os / arch 选子包、`docker-image` 子包来源、InstallPlan 的幂等与状态都是现成的；`agent-skills` 已经是这样一个子类。

- **AppDoc**：`AppType::ExtTools`（`app_type = ext_tools`，新值，校验同 `Aiws`：只接受本类子包、不建 AppSpec、不起服务；决策 18）；子包键 `ext_tools_<os>_<arch>`（v2.3 修正：`pkg_list` 是 map，一个键放不下多个平台；闲置的 `agent_tools` 键已删除，决策 18），一个平台一个 payload（`linux-x86_64`、`linux-aarch64`、`darwin-arm64`、`darwin-x86_64`、`windows-x86_64`，缺的平台就是“该平台不支持”）；payload 是 `tar.gz`，内容就是解到 `tools/store/<pkg_id>/` 下的树加 `bin/` 里的 launcher。Linux 容器节点不再需要 `docker-image` 子包：reconciler 解 tar 进卷。
- **`ext-manifest.json`**（payload 根，`pikg doctor` 校验，Repo 投影进目录）：

  ```json
  {
    "schema": "buckyos.ext_tools/1",
    "id": "ext-media", "version": "1.2.0",
    "requires": [{"id": "ext-core", "version": ">=1.0"}],
    "conflicts": [],
    "provides": [
      {"command": "ffmpeg",  "version_probe": "ffmpeg -version", "smoke": "smoke/ffmpeg.sh"},
      {"command": "ffprobe", "version_probe": "ffprobe -version"}
    ],
    "platforms": ["linux-x86_64", "linux-aarch64", "darwin-arm64", "darwin-x86_64", "windows-x86_64"],
    "size_bytes": {"linux-x86_64": 123456789},
    "license": "LGPL-2.1", "desc": "音视频加工：ffmpeg / ffprobe"
  }
  ```

  `provides` 是系统对“哪个包提供哪个命令”的全部知识来源（§3.9.3）。`requires` 是包级依赖，装 B 自动带 A；`conflicts` 处理同名命令（两个包提供同一命令必须显式 conflicts，目录里不能同时启用）。包不声明 `writes`：扩展包只写 `tools/store/<id>` 与 `tools/bin`，这是格式约束不是声明。
- **`ext-core`** 是目录里第一个包，内容就是 §3.1 的核心清单；`EXTENDED_TOOLS` 常量与它的 `provides` 一致性测试绑定。今天的 `publish/exttool/` seed 镜像变成 `ext-core` 的 Linux payload 构建脚本（§3.9.2）。
- Agent 层（第三层）不是扩展包：skill 自装走 skills TODO 的 `install.toml`，不进这个体系。

#### 3.9.2 列表、期望状态与构建发布

**扩展包目录（Zone 级）**：就是 Repo 里已发布的 `ext_tools` AppDoc，不另投影 `system/ext_tools/catalog`（决策 16）；管理页列表与 Resolve 直接查 Repo，取最新版本、平台、`provides`、`requires`、大小、desc；官方包随 release 预置，Zone owner 也能 `app publish` 自己构建的包（“行业专用环境”就是这样来的）。

**期望状态（管理员勾选的结果）**：`system/ext_tools/desired/<pkg_id> = {version, pikg_digest, enabled, by, at_ms}`。写入规则：勾选 B 时 resolver 做拓扑排序，`requires` 链上没勾的一起写 enabled 并在确认页列出；取消 A 时有启用中的依赖者则拒绝并列出；升级就是改 `version`。写入由 control panel 做（RBAC：zone owner / 系统管理员），走和 App 安装一样的 Resolve → Inspect → 确认 → 提交，但不建 AppSpec、不起服务，提交的是期望状态而不是 InstallPlan；scheduler 再把它投影到每个节点的 `NodeConfig.ext_tools`（决策 15）。

**每节点状态**：`nodes/<n>/ext_tools/<pkg_id> = {version, digest, stage: queued | downloading | verifying | staging | checking | active | failed | unsupported, error, at_ms}` 由 node_daemon reconciler 写；本机 `tools/installs/<pkg_id>.json` 是同一内容的本地副本（§3.2）。`unsupported` = 该节点平台没有 payload。管理页按包 × 节点显示；Agent 的 runtime 卡片显示本机那一列。

**构建与发布（有了具体需求后的路径）**：

1. `buckyos pikg init --kind ext-tools --name ext-xxx --source ./ext-xxx`（v2.3：`--name` 就是清单 `id`）：生成目录骨架 `ext-manifest.json` + `payload/<platform>/{store/<id>/, bin/, smoke/}`。
2. 各平台 payload 的来源自定：官方包在 `publish/exttool/<pkg_id>/build.sh`（现有 `install_pandoc.sh` 就是 `ext-core` 里 pandoc 的那段），CI 矩阵按 `platforms` 构建；第三方可手工放二进制。锁定版本、下载源、SHA256 写在 `build.sh` 顶部，和今天 pandoc 的做法一样。
3. `buckyos pikg doctor --catalog <目录>`：`provides` 的每个命令在对应 payload 的 `bin/` 里有 launcher 且可执行；launcher 不含构建机绝对路径；`smoke` 脚本存在；`requires` 的 id 在目录里；同名命令要么 `conflicts` 要么拒绝。doctor 不连 Zone，后两条要 `--catalog`（本地一批 `<id>/ext-manifest.json`，如 `publish/exttool`），Zone 目录上的同一检查归 X2 的 Resolve。
4. `pikg build / pack` 本地校验 → `app publish <工程目录> --app-doc dapp_dist/APPDOC.json`（control panel 重打并把 PIKG 文件 pin 进 Repo，Repo candidate）→ owner 签名发布，与 App 一致。
5. 管理页看到新版本 → 勾选 / 升级 → reconciler 安装 → `check` → 状态 active → `tools_check` 重跑 → skill 状态重算（§3.9.3）。

长期运维就是 2 → 5 的循环；回滚 = 期望状态改回旧版本，节点保留上一版 payload（§3.2）。

#### 3.9.3 skill 与扩展包：skill 只声明命令，系统反推包

- **skill 声明**：front matter 加机器可读的 `commands: ffmpeg, ffprobe, magick`（只写命令名，逗号分隔）；`requires` 继续给人读；`baseline / platforms` 不变。这改掉 skills TODO 待裁决 14 的答案（原建议“只放 `install.toml`”）：owner 明确 skill 应说明自己依赖的命令，而 runtime R-W6 的“不加字段”针对的是运行位置（`runs_on`），不是依赖声明。官方手册 `aigc_*`、`aiworkspace_run`（`jq`）等补上。
- **系统知识**：Repo 里 `ext_tools` 包的 `provides`（经 control panel 的 Resolve 读取）给出 command → 包 的映射；`tools_check.resolved` 给出本机每个命令的实际来源（`exttools | system | agent | 缺`）。
- **反推**：`skills.install_plan`（skills S18）对每个 `commands` 条目查目标 runtime 的 `tools_check.resolved`；缺的查目录 `provides` → 得到 `recommended_packages: [{id, version, requires_chain, provides: [缺的命令]}]`；命中多个包时按“已启用的依赖最少”排序；没有任何包提供的命令报“无扩展包提供，需自备”。推荐同时出现在 CLI 的 install plan、WebUI Skills 页与 `skill list --all` 的状态列。
- **skill 状态**（skills_state 的 `install.runtimes[id]` 扩展）：`commands: {ffmpeg: {present: true, provided_by: exttools}, magick: {present: false}}`、`recommended_packages`。三种可见状态：`ready`（命令齐）、`partial(缺 x, y；建议 ext-media)`、`ready(system)`（命令齐但来自用户自备）。扩展包 active / 卸载 → node 状态变 → runtime `tools_check` 重跑 → skills 状态重算（runtime R6 / skills S21 的事件路径，这就是它的第一个真实触发源）。plan 全表与 `## Commands` 末尾说明用同一数据。
- **方向反过来也成立**：管理页勾选一个包时，可以列出“Zone 内哪些 Agent 的哪些 skill 会因此从 partial 变 ready”（扫各 Agent 的 skills_state），这是运维看得见收益的地方；首版可只做包 → 命令 → skill 名的静态反查。

#### 3.9.4 不 block

- skill 安装永远不因缺命令失败：无自装命令的 skill 确认后直接 `enabled`，状态里带 `partial` 与推荐；有自装命令的按 skills TODO §3.6.1 C。
- 冻结仍只按 `baseline / platforms` 过滤（runtime M5）；`partial` 的 skill 照样能被 `--cmd-skill` 点名，手册末尾写“缺 `magick`，建议 owner 安装 ext-image，或在该 runtime 上自备”。
- 用户在自己的 host 上用自己的方法装了 ffmpeg：`check` 命中 `provided_by: system`，skill 变 `ready(system)`，不要求走扩展包；`extended` 声明仍由 owner 决定，不自动改。
- 扩展包对所有 Agent 生效，首版不做 Agent 级的包绑定；paios 容器需求 §9 的“按授权渲染 `tools/bin`”保留为将来按 Agent 限制的机制，不在本轮。

### 3.10 `ext-core` 的发行、钉版与种子（T2 细化；owner 2026-11-06 裁决 21–24）

工具清单的事实来源是 [paios容器需求.md §6.4](../doc/app_service/paios容器需求.md) 表 B / 表 C；本节只定流程。两个现成模板：GitHub Release 套 `.github/workflows/agent-daemon.yml` 的双 release；安装包集成套 `build-all.yml` 的 `resolve-paios-images`（本次构建用本次片段、没构建继承指针、校验 URL、塞进每个平台 payload）。

**发行（T2a）**：新工作流 `.github/workflows/ext-core.yml`。触发：`publish/exttool/ext-core/**` 的 PR 只构建并上传 artifact；tag `ext-core-v<ver>` 或带 `publish` 的 dispatch 发布。版本号 = `ext-manifest.json` 的 `version`，与 `src/VERSION` 解耦。发布两个 release：不可变的 `ext-core-v<ver>` 与滚动的 `ext-core-latest`（带 `latest.json`）；同版本已有资产则拒绝重发。资产三样，三种消费者共用同一份字节：

- 五个 `ext-core-<ver>-<platform>.tar.gz`，内容就是 pikg 子包 payload 树（`ext-manifest.json`、`bin/`、`smoke/`、`store/ext-core/`），解到 `tools/` 即可用；
- `manifest.json`：每平台的 `file / sha256 / size / path / url`，格式对齐 `paios_images.json`；
- `ext-core-<ver>.pikg`。Zone Repo 的“官方包随 release 预置”与安装包里的种子必须同一个 `pikg_digest`，X2 的期望状态才对得上。

payload 构成（决策 21）：有官方归档的工具（pandoc、python-build-standalone、uv、node、deno、jq、Windows 的 MinGit）由 `build.sh` 逐工具下载、固定版本与 SHA256，落 `store/ext-core/<tool>/`；没有官方二进制的一组（ffmpeg、ImageMagick 7、poppler、tesseract、git、curl、tree、unzip、xz）装进一个 conda-forge 前缀 `store/ext-core/conda/`，`conda-lock.yml` 锁版本与来源，conda-pack 重定位；自己编译只留给重定位失败的平台。`bin/` 的 launcher 只指 `store/` 内的相对路径，**不再转发 PATH**（替换 §4.1 偏离 4 的临时做法）；Windows 的 launcher 是 `.cmd` 或 exe shim，`pikg doctor` 对 Windows payload 按去扩展名比对 `provides`。组装可在一台 Linux runner 上完成（官方归档直接下载；conda 前缀用 micromamba `--platform` 建，或改在各平台原生 runner 建，T0 定）；**门禁是五平台矩阵离线解包跑 `smoke/*.sh`**，任一平台失败不发布。CI 同时把资产镜像到 R2 `ext-tools/ext-core/<ver>/<platform>/<sha256>.tar.gz` 与 `ext-tools/ext-core/<ver>/manifest.json`（决策 22）。许可证文件随 payload 带。

**钉版与 resolve（T2b）**：buckyos 不跟 `ext-core-latest`。`build-all.yml` 加 `resolve-ext-tools`：读仓库里 `ext-manifest.json` 的 `version`，取 R2 上该版本的 `manifest.json`，HEAD 校验每个 tar 的 size，写成 `bin/ext-tools/ext_tools.json` 放进每个平台 payload（与 `bin/paios-images/paios_images.json` 并列；同样只在 `buckyos/buckyos` 的 main 上钉，fork 处于本机模式）。该版本还没发 release 时 main 构建失败：先发 `ext-core` 再提版本号。升级 `ext-core` = 一个改 `ext-manifest.json` 版本并发 release 的 PR，安装包可重现。同一任务加脚本检查：§6.4 表 B ↔ `ext-manifest.json` `provides` ↔ `build.sh` / `conda-lock.yml` 的锁定清单。

**打包机与在线安装（T2c）**：sn-business 加 `package-kit/stage_ext_tools.py`，与 `stage_paios_images.py` 同构：读 staging 里的 `ext_tools.json`，按本平台从 R2 取 tar 到缓存并校验 size + sha256，复制到 `bin/ext-tools/`；三个 builder 在组装 staging 后调用。pack-installer `verify-pkg --require-ext-tools`。pve-pack-system `linux.sh` 用 `path` 拼当前 base URL 下载，校验失败在动旧安装之前中止；`onlinepack.py` 发布前 HEAD 校验。**安装器只放文件，不解包到 `tools/`**，对应 paios 的“安装器不执行 docker load”。安装时不访问 github.com。

**种子（T2e，X3 的子集；决策 23）**：node_daemon 启动时，若本机 `tools/installs/ext-core.json` 不存在且 `bin/ext-tools/` 里有本平台的 tar，则校验 sha256 → 解到 `tools/staging/<ver>-<digest>/` → 跑 `smoke/`（可配置跳过）→ 切换 `bin/`、`store/`、写 `installs/ext-core.json` 与 `current.json`。容器节点同一套，播进 `buckyos-tools` 卷，取代 exttool seed 镜像里 pandoc 那段；`uv-cache`、`deno-cache` 仍由 seed 镜像播（去向在 X3 定）。`installs` 已存在就不动：装过之后以 `NodeConfig.ext_tools` 期望状态为准，X3 之前就是“不动”。所以升级安装包带了 `ext-core` 1.2 而 Zone 期望 1.1 时，节点不会被安装包覆盖。macOS 解包要避免 quarantine 传播（curl 下载的归档带 `com.apple.quarantine`，bsdtar 会传给每个文件，Gatekeeper 拦未签名的 ffmpeg）：解包后清 xattr 或用不传播的解包路径，T2e 验收项。

**开发环境（T2d）**：“无 CMD 才更新”落在同一原语上。`src/tools/prepare_ext_tools.py`（与 `prepare_sdk_tool_distribution.py` 并列）：`bin/ext-tools/` 没有本平台 tar 才按钉住的版本从 R2 下载并校验，`--latest` 可选读 `ext-core-latest/latest.json`，`--force` 重下；`buckyos-build.py` 与 `start.py --all` 调用；`start.py` 的就绪检查照 `_sdk_tool_distribution_ready` 写，缺时提示命令而不是静默。解包仍由 node_daemon 的种子做，所以开发机与安装包走的是同一条路径。开发时改了 `ext-core` 本身：`./build.sh` 产出本地 payload 后 `prepare_ext_tools.py --local <payload 目录>` 打成同格式 tar 放进 `bin/ext-tools/`，删 `tools/installs/ext-core.json` 重启即重播。

**容器**：`build_exttool` 的 Dockerfile 改为 COPY Linux 的 payload tar（不再在镜像里跑 `install_pandoc.sh`）；T2 落地后 `build_aios` 的 `RUNTIME_PACKAGES` 去掉 `ext-core` 的命令（ffmpeg、imagemagick、poppler-utils、tesseract-ocr、python3、git、jq、curl、tree、unzip、xz）与官方 Node / Deno 安装，只留 `bash`、`rsync`、`ca-certificates` 与 §6.4 表 C `ext-office` 那组；§4.1 偏离 5 的“容器 ext 状态是 system”随之变 `exttools`。

### 3.8 三期：standard 分发（原 v1 §3.1.1，保留要点）

- 清单沿用 `STANDARD_TOOLS`，导出机器可读 manifest，每条标 provider、入口类型（native multicall / TS / SDK CLI / session overlay）、版本、运行依赖、smoke；与 `commands/*.ts` 的一致性测试保留。
- 产物 `agent-tools-<version>-<os>-<arch>`：multicall 提供程序、入口、`buckyos-agent` TS 源码与 vendor、SDK / CLI、固定版本 Deno、离线依赖；解压后不依赖源码树或构建机缓存；与 OpenDAN / SDK / 协议记共同 release id。
- 复用 SDK 分发的 manifest、固定 Deno 与准备脚本，不另造下载流程。抽共享组装器供 `buckyos-build.py` 与 `build_aios` 使用；保留 `bin/agent-tools`、`libexec/buckyos-agent`、`libexec/buckyos-tool` 的契约；禁止把构建路径写进 launcher。
- 入口：POSIX 链接；Windows hardlink / copy，不依赖 symlink 特权；保留参数边界、退出码、Unicode、stdio 与取消语义；不加经 shell 拼接参数的包装层。PowerShell、cmd、POSIX shell 分别验证真实入口。
- 身份：保留 `session_identity_env` 与经 Session token 取服务身份；安装包不带 token / 私钥；`agent-session` 进 standard 包但只有私有 default session 的 helper 工作。
- 安装与升级：随系统首次安装与升级部署；`install-agent-tools` 保留为入口物化 / 修复步骤，不代替 payload 部署与版本校验；缺入口或依赖时报 standard 不完整；升级 staging 后切换，处理 Windows exe 占用，需要重启时由服务生命周期协调。standard 缺失时 `extended` 汇总仍失败。

## 4. 任务

### 一期（Rust + pikg 结构，现在可开工）

- [x] **P1-1 `EXTENDED_TOOLS`**：`baseline.rs` 加常量（§3.1 核心 + 推荐标记）、`publish/exttool/ext-core/ext-manifest.json`（§3.9.1 格式，就是 `ext-core` 的 manifest，P1-6 同一份）与一致性单测；`required_commands` 加 level 参数。验收：常量与 json 不一致时测试失败。
- [x] **P1-2 按声明检查**：`check_baseline` 按 `caps.baseline` 选集合；`BaselineCheck` / `ToolsCheck` 加 `resolved {path, version, provided_by}`；探测脚本（POSIX / PowerShell）输出路径与版本；核心命中路径不在 `$BUCKYOS_ROOT` 下记 `provided_by: system` 且 standard 不算就绪。验收：`runtimes.rs` 与 `remote_node_runtime` 用例覆盖 standard / extended、system 命中、stub 127。
- [x] **P1-3 CLI 与视图**：`xagent runtime baseline --level`、`check --commands` 输出路径版本；`runtimes_view` 的 plan 全表加 ext 状态段；WebUI 卡片显示 `baseline_check` 的 ext 部分（只读）。
- [x] **P1-4 回写**：runtime TODO R-S1（安装通路改 §3.3）、R-S3（已做）、裁决 22（三期 = 安装包改造）；Session Runtime 文档 §3.2 / §3.3 改为核心 + 推荐 + 可选；skills TODO S19 的依赖标“已解除”。
- [x] **P1-5 host 执行器前置 `tools/bin`**（§3.6 的 PATH 顺序，先只做顺序，不做环境变量）：`node_executor ops.rs`；验收：host 上 `check_baseline(extended)` 与真实 session shell 看到同一 ffmpeg。
- [x] **P1-6 pikg 结构支持（原 X1，决策 4）**：`AppType::ExtTools` 与按类型校验、子包键 `ext_tools`、`agent_tools` 键的去留（buckyos-api `app_doc.rs`）；`ext-manifest.json` schema 进 [PIKG 格式规范](../doc/pikg/)；websdk `pikg init --kind ext-tools` 骨架与 `doctor` 规则（§3.9.2 第 3 条，跨仓库同步）；control panel：`app.publish` 发布到 Repo，`app_submit_route(ExtTools, _)` 一律拒绝（v2.3：`app publish` 本来就不经 `app_submit_route`）；`publish/exttool/ext-core/` 按此格式重打（Linux payload 复用今天的 seed 构建，含 `install_pandoc.sh`），`app publish` 到开发 Zone 的 Repo 回读通过。验收：`pikg doctor` 对缺 launcher、同名命令无 `conflicts` 报错；`app install` 一个 `ext_tools` 包被拒并说明走管理页；`cargo test -p buckyos-api`、`-p control_panel` 与 websdk 测试通过。
- [x] **P1-7 一期口径**：后台 `check_baseline / tools_check` 只记命中路径，版本在 `check --full` 与安装后记（决策 19）；开发态 helper 来自可执行文件旁（`missing_on` 的 `beside`）记 `provided_by: dev`，算 standard 就绪但在视图里标出（决策 20）。

### 4.1 一期实施记录（2026-11-06）

验证结果：

| 项 | 结果 |
| --- | --- |
| `cargo test -p libopendan --no-fail-fast -- --test-threads=1` | 1 项失败：`session_runtime::a_session_token_reaches_only_its_own_session_methods`，在 `ea6db44c2` 上同样失败（早先这里写“通过”是漏看，已更正）。并行跑时 `plan_simple_async`、`runtimes` 等会因机器负载失败，串行通过。`baseline` 12 项；`runtimes.rs` 的私有 runtime 用例覆盖 agent / system 命中与 stub 127；`xagent_cli` 覆盖 `--level`、`--check`、system 命中、dev。`state::registry::id_tests::concurrent_reservations_*` 在高负载并行时偶发 `Busy`，重跑通过 |
| `cargo test -p node_executor` | 通过。新增 `tests/host_ext_tools.rs`（真实 executor，current user）：`tools/bin` 优先、check 与 `runtime_exec` 看到同一 ffmpeg、stub、版本保留、用户 PATH 的 `xagent` 记 system。`remote_node_runtime` 要 root，本机跳过，未跑 |
| `cargo test -p buckyos-api`、`-p node_daemon` | 通过 |
| `cargo test -p control_panel` | 214 过 1 败。`sso_identity::tests::account_identity_carries_*` 在未改动的 HEAD 上同样失败 |
| `cargo test -p opendan` | `a_session_process_elsewhere_*` 失败，在未改动的 HEAD 上同样失败；其余通过 |
| `cargo check --workspace --tests` | 通过 |
| websdk `pnpm run check:cli`、`test:cli`（139 项）、`test:cli-conformance` | 通过 |
| `tools/tests`（98 项） | 通过（2 skipped） |
| OpenDAN WebUI `pnpm check`、`pnpm lint`、`home.spec.ts` | check / lint 通过；e2e 7 过 1 败，`relocation keeps identity` 在未改动的 HEAD 上同样失败 |
| 开发 Zone 实测（只更新了 control_panel 与 CLI 分发） | `ext-core` 的 `pikg doctor --catalog`、`build`、`pack`、`info` 通过；`app publish` 成功，`repo.resolve("ext-core.buckyos.bns.did.pikg")`、`repo.list` 回读到记录（`app_type=ext_tools`、`version=1.0.0`、`status=pinned`）；`app install` 报 `EXT_TOOLS_NOT_AN_APP`，`app fetch` 的 `target_issues` 给出同样说明；缺 launcher 的工程被服务端拒绝；同版本重发被拒 |
| `remote_node_runtime`（root） | 5 项通过 |
| 空 Zone DV（`start.py --all --wipe-data`，全量构建 + `./build_aios --local-test`，新建 Agent `extv`） | 私有 runtime：Available，`unmanaged` 为空，标准命令全在 `$BUCKYOS_ROOT` 下；`runtimes.check private --full` 由容器内 opendan 执行，21 个系统工具取到版本、无 stub，`ext: partial`（缺 pandoc，这台机器没有 ExtTool 卷）。host（`agent.update` 启用）：`describe` 报 `buckyos_root=/opt/buckyos`，loader 记 `standard tools complete`。在 `tools/bin` 放占位 ffmpeg，不重启 executor：`check --full` 记 `exttools` 和版本；改成退 127 记 stub 并计入缺失；后台 check 沿用；删除后回落 `/usr/bin/ffmpeg`（`system`）。**P1-5 验收**：真实模型驱动的工作会话（`runtime` 冻结为 `host`，`kind: remote_node`）的 `shell` 原始输出与 check 同路径同版本，PATH 为 `tools/bin:bin/agent-tools:…`。`ext-core` 在空 Repo 发布成功，`list` 回读 `status=pinned`、`app_type=ext_tools`；`app install` 报 `EXT_TOOLS_NOT_AN_APP`；去掉 CLI 拦截后 `apps.submit` 由服务端拒绝（`UNSUPPORTED_TARGET`，`product_ext_tools_package`）。真实 WebUI bundle 配真实服务渲染两张 runtime 卡片，无页面错误（未经网关 SSO，token 由测试注入） |

未验证：ExtTool 卷挂载后的路径（pandoc 记 `exttools`；这台机器没有 `paios/exttool` 镜像，要先 `./build_exttool`）；PowerShell 探测和 `agent-session run` 的 `& '...'` 只验了生成的文本，没在 Windows 上跑。ext-core 的 ImageMagick、tesseract smoke 在 aios 镜像的一次性容器里跑过，退出 0；测到的是镜像自带的工具，不是包里的 payload。

DV 期间看到、未修改：

- 同一工程重发同版本报 “already pinned with other content”，即使内容没变：`build_final_app_doc_for_publish` 每次写 `last_update_time`，AppDoc id 变，`app_doc_id` 又在文件对象 meta 里，文件 id 跟着变。拒绝本身符合版本不可变，文案不准确；CLI 没有 unpin，发布后要改只能升版本。
- `share: compose` 报 `no_candidate_model`（AICC `helper.llm_chat`），与本 TODO 无关。

DV 期间看到、随后修复：

- `agent-session run -- sh -c 'a; b'` 丢引号（`c88404aaf` 起把 argv 用空格拼成一行）。现在多个词是命令和它的参数，边界保留（POSIX 单引号，Windows `& '...'`）；一个词才是交给目标 shell 的命令行。重建镜像后由真实模型会话在 host 上验过 `sh -c`、单词命令行和带空格、`$`、单引号的参数。
- `agent-session run` 和 `copy` 走的 `usable_runtime` 只看记录、不重新探测（`c88404aaf`）：第一个 Agent 启用 host 时 loader 的探测可能早于 executor 打开，记录停在 `runtime_unavailable`，这两个命令一直退 6，直到 owner 点 Check 或有新 session 绑定 host。11-06 两次启用碰到一次。现在和 `freeze_runtime` 共用 `lookup_rechecked`，过了 `RUNTIME_RECHECK_MS` 就重新探测。重建镜像后没再碰到自然竞争，把记录改回卡住状态验证：真实会话的 `run` 退 0，记录自动变回 `available`。`runtime list` 和卡片仍显示记录的旧状态，要等下一次使用或 Check 才更新。
- runtime 卡片上扩展命令缺失说两遍。现在每行只说自己一层：“Tools missing” 只列标准命令；扩展行在声明 `extended` 且缺命令时用警告色。

上线顺序：`describe` 新增 `buckyos_root`，`ExecutorRuntimeInfo` 是 `deny_unknown_fields`，所以更新 node_daemon 的同时必须重建 aios 镜像（`./build_aios --local-test`），否则容器里旧 opendan 解析不了 host runtime 的 describe。aios 镜像把 AICC 入口从 `/usr/local/bin` 挪到了 `/opt/buckyos/bin/agent-tools`，新 `xagent runtime baseline --check` 在旧布局上会报 unmanaged。

相对正文的偏离与补充（都已改回上文）：

1. **子包键**：`pkg_list` 是 map，放不下多个平台，改为 `ext_tools_<os>_<arch>`。
2. **发布路由**：P1-6 写的 `(ExtTools, Rpc)` 发布到 Repo 与“`app install` 被拒”冲突：`app install` 走的正是 `apps.submit` 的 Rpc 来源，而 `app publish` 本来就走 `app.publish`。所以 `app_submit_route(ExtTools, _)` 一律拒绝，发布落在 `scan_publish_sources`。
3. **`app publish` 的两个既有 bug**：`2985ff934` 之后 `repo.store` 只收 file / dir / chunk，`app publish` 存 `pkg`、`appdoc` 对所有 App 类型都失败；payload 超过 32 MiB 时 `PackageMeta.content` 是 `clist:`，PIKG 自校验失败。改为把自校验过的 PIKG 文件 pin 进 Repo（`<AppId>.pikg/<version>`，带 `app_type`、`app_doc_id`），`content` 用整个 payload 的 chunk id。版本不可变，同版本重发要先 unpin（CLI 没有 unpin）。
4. **Linux payload 不是自包含的**：`ext-core` 只有 pandoc 随包，其余命令是转发到 PATH 上下一个同名命令的 launcher，机器上没有就退 127。这满足“provides 每条有 launcher”，但裸 Linux host 从这个包得不到 ffmpeg；真正随包是 T2（§3.10：launcher 改为只指 `store/`，不再转发 PATH）。在 X3 之前没有东西安装这个包（§3.10 的种子 T2e 是 X3 的子集）。seed 镜像和容器行为都没变。
5. **容器的 `ext` 状态是 `system`**：ffmpeg 等在 aios 镜像里（`/usr/bin`），只有 pandoc 来自卷。这是真实情况，T2 之后才会变 `ok`。没有 ExtTool 卷的开发安装实测是 `partial`（缺 pandoc）。
6. **aios 镜像布局**：§3.4 的严格检查要求标准命令在 `$BUCKYOS_ROOT` 下，所以入口挪到 `/opt/buckyos/bin/agent-tools`，手册同步。
7. **检查总是带上扩展层**：声明 `standard` 的 runtime 也探测 `EXTENDED_TOOLS`，否则 plan 全表写不出 `ext:` 段；`ok` 只按声明算。
8. **`xagent runtime check --full`**：决策 19 要有地方记版本，一期先放在这里（T5 的 `ext-tools check --full` 之后复用）；只跑版本探测，不跑 smoke。`full` 记下的 stub 在路径不变时被后台检查沿用。
9. **格式约束比正文严**：`bin/` 下每个文件都要在 `provides` 里；payload 只能有 `ext-manifest.json`、`bin/`、`smoke/`、`store/<id>/`；工程根的清单是唯一来源，payload 根放副本。
10. **`InstallGrant::System`（决策 17）**：一期只改了注释和 UI 文案，行为随 skills S19。

留给二期：

- X2 Resolve：目录查询可用 `repo.list` 按名字前缀，靠记录里的 `app_type=ext_tools` 过滤；但 Repo 记录只有 PIKG 文件，`provides` 要读 PIKG 里的 payload，可能要把 `provides` 投影进文件对象的 meta。
- X3 reconciler：uv / deno 预热缓存不在 `ext-core` 包里（格式只允许 `store/<id>`），还在 seed 镜像，切换时要定去向；FreeCAD 占位入口同理。
- X3 要拒绝 `provides` 与 `STANDARD_TOOLS` 同名的包：`tools/bin` 排在 `bin/agent-tools` 前面，能遮蔽标准命令。现在只有 `tools_check` 报 unmanaged，TS 侧不知道这份清单。
- `block_inspection` 对同一拒绝原因会写两遍 `target_issues`（既有问题，不影响判定）。

### 二期：扩展包体系、安装器与环境约定

- [x] **X1 扩展包格式**：并入一期 P1-6。
- [ ] **X2 期望状态与投影**：control panel `ext_tools.set_desired`（Resolve 查 Repo、拓扑排序、依赖自动勾选、卸载拒绝、RBAC，记 `pikg_digest`）；scheduler 投影 `NodeConfig.ext_tools`（决策 15）；`nodes/<n>/ext_tools/<pkg>` 状态 schema。落点：`control_panel/`、`scheduler/`、`buckyos-api` 的 `NodeConfig` 与 system config 键 / RBAC。
- [ ] **X3 node_daemon reconciler**：对照期望状态下载（经 Repo / NDM）、校验、staging、激活、卸载、升级、回滚，写节点状态与 `tools/installs/`；容器节点写 `buckyos-tools` 卷、`ext-core` seed 镜像只做首启播种；三平台原生同一代码。取代 v2 的 T3。落点：`node_daemon/src/paios_images.rs` 拆出 `ext_tools.rs`。
- [ ] **X4 管理页与 CLI**：Desktop / control panel 的“扩展包”页（包 × 节点状态、勾选、升级、依赖提示、受益 skill 静态反查）；`buckyos ext-tools list|enable|disable|status`。Agent runtime 卡片只显示本机状态与链接。
- [ ] **X5 skill 命令依赖**：`SkillMeta.commands` 解析；`skills.install_plan` 的 `recommended_packages`；skills_state 的 `commands / recommended_packages`；扩展包状态变化 → `tools_check` → skills 重算（与 runtime R6 / skills S21 同一条路）；官方手册补 `commands`。落点：`protocol/behavior.rs`、`skills.rs`、skills TODO S18 / S20 / S21。

- [ ] **T0 可行性（收窄）**：核 §6.4 表 B 标“核”的格子（conda-forge 的 tesseract 是否自带 eng / osd、win-64 的 tree / unzip、IM7 旧名）；conda-pack 前缀在 Linux / macOS / Windows 各做一次干净机器重定位并跑 smoke；给出五平台 payload 的压缩前后体积；ImageMagick 已定 IM7（表 B）。缺资产的平台明确阻塞，不输出 stub。
- [ ] **E0 目录与映射定稿**：容器 / 原生的 `tools/`、instance、Agent 缓存目录逐项列出；path_usage.md 更新；验收容器重建后自装工具仍在、rootfs 非持久写入不被误报成功。
- [ ] **T2a release 工作流**：`.github/workflows/ext-core.yml`（§3.10）：`build.sh` 五平台组装、smoke 矩阵门禁、双 release、R2 镜像。验收：干净环境离线解包跑 `smoke/*.sh`，不借构建机 PATH；`ext-core-latest/latest.json` 可读；同版本重发被拒。
- [ ] **T2b 钉版与 resolve**：`build-all.yml` 的 `resolve-ext-tools` 与 `bin/ext-tools/ext_tools.json`；版本未发 release 时 main 失败；表 B ↔ manifest ↔ 锁定清单的一致性脚本。验收：三平台 payload 都带清单且 URL 校验通过。
- [ ] **T2c 打包机与在线安装**：sn-business `stage_ext_tools.py`、pack-installer `--require-ext-tools`、pve-pack-system `linux.sh` 与 `onlinepack.py`。验收：全量包 `verify-pkg` 校验 tar；在线安装篡改 tar 时在动旧安装前中止；安装过程不访问 github.com。
- [ ] **T2d 开发环境**：`src/tools/prepare_ext_tools.py`、`buckyos-build.py` / `start.py --all` 接入、就绪检查、`--local`。验收：`start.py --all` 第二次不重下；删 `installs` 记录重启即重播。
- [ ] **T2e node_daemon 种子（X3 子集）**：`installs` 缺失才播，校验、staging、切换、写记录；容器节点播进 `buckyos-tools` 卷；`build_exttool` 改 COPY payload；macOS quarantine。验收：原生三平台装后 `xagent runtime check --full` 全部 `exttools`；容器里同一命令同一版本；已装节点不被新安装包覆盖。
- [ ] **T3 安装器细节（并入 X3）**：staging、校验、锁、journal、激活、失败恢复、版本保留；Windows 文件占用。验收：不可写根不走 sudo 而是报状态；同节点并发 reconcile 不损坏公共目录；旧版本可运行。
- [ ] **T4 环境约定落地**：§3.6 的 PATH 顺序与环境变量在 `node_executor ops.rs`、`entrypoint.sh`、`runtime_exec`、session overlay 四处一致；同名冲突检查；用户预装不同版本 python / node / deno 时 Agent 用受管版本、用户终端不变。验收：安装前后比较用户 PATH、shell profile、全局包配置无变化。
- [ ] **T5 Agent 侧 API / CLI**：§3.7 的 `ext-tools status | check` 与 owner API；没有 install，安装不改声明。验收：节点状态变化后 Agent 侧 `status` 与 `tools_check` 一致。
- [ ] **T6 UI**：卡片状态、安装、完整检查；容器 image-managed；同机共享安装事实。
- [ ] **T7 文档**：Session Runtime §3、Jarvis 手册中 ImageMagick 旧命令与额外依赖、path_usage.md、本文 §2。

依赖：一期 P1-6 定格式 → T0 与 E0 并行 → T2a（release）→ T2b（钉版）→ T2c / T2d / T2e 并行，整体与 X2 并行 → X3（含 T3，吸收 T2e）→ X4 / T5 / T6；X5 依赖 X2 的 Resolve 与一期的 `tools_check.resolved`，可与 X3 并行；T4 依赖 E0，可与 T2 并行；T7 最后。T2c 要改 sn-business、pack-installer、pve-pack-system 三个外部仓库并在打包机上 `git pull`，与 paios M4 同样的部署顺序。T0 若证实某平台成本过高，带具体缺项回 Review 改清单，不自行降标准。

### 三期：standard 分发

- [ ] **S0 盘点与契约**：`STANDARD_TOOLS` 每条 provider 在三平台的产物、依赖、身份路径；导出 manifest；固定 release id。
- [ ] **S1 共享组装与发行**：host / aios 共用组装器，接入 SDK 分发脚本；Linux 镜像 payload 与 macOS / Windows / Linux 原生包。验收：脱离源码树、空 Deno 缓存、无系统 Deno / Python / ExtTools 时全部标准入口 smoke 通过。
- [ ] **S2 安装 / 修复 / 升级**：系统首次安装、节点物化、容器构建、原生安装器加 `agent-tools` 包类型；版本一致性、Windows 占用、回滚。验收：新装即满足 standard；破损可修复；升级失败保留旧版；ExtTools 卸载 / 升级不破坏 standard。
- [ ] **S3 端到端验收**：三平台 native 分派、TS / SDK 执行、session overlay、节点 Session 身份传递、容器布局；至少一次真实 Zone AICC 调用，不以 `--help` 代替。

### 分出去的

- **Zone 共享可选包**（v1 §3.6 / E1）：owner 发布可重定位工具包或派生 paios 镜像，多 Agent 只读复用、各自缓存；需要 `/usr`、动态链接器、系统包数据库的东西走派生镜像，不靠“有 Agent 卷”就当持久。另起 TODO，依赖本文一期的清单格式与 E0。
- **macOS VM 由 Agent 自初始化**（v1 §3.8 / E3）：系统只提供可用 VM 与一份版本化的“macOS 私有 runtime 初始化要求”文档，Agent 自行完成安装、配置、验证与维护；不预装、不新增专用恢复服务。另起 TODO，依赖三期的发行资产说明。

## 5. 验收

按期分开，做完一期就能宣称一期：

**一期**：`cargo test -p libopendan baseline`、`--test xagent_cli`、`runtimes.rs` 用例通过；声明 `extended` 而缺一个核心工具时检查失败；stub 不计成功；用户 PATH 里的同名核心命令记 `system` 且 standard 不就绪；检查不改声明与已冻结 Session；plan 全表能看到 ext 状态；pikg 结构按 P1-6 的验收项。

**二期**：三个平台在无预装扩展工具的干净环境离线安装并跑媒体、图像、PDF、OCR、Pandoc、Python、JS smoke；Windows 覆盖空格与中文路径、不依赖 Developer Mode；过期 / 只读 / form B / SSH / node-daemon 服务 host 拒绝并给原因；摘要错误、错误架构、磁盘不足、中途退出、文件占用、并发安装都保持旧版可运行；重启后查询一致；真实 work shell、`agent-session run --runtime` 与 probe 看到同一版本，私有 PATH 不带到目标机；用户 PATH、profile、全局配置安装前后无差；预装不同版本工具的机器上 Agent 用受管版本、用户终端不变。

**三期**：干净系统不装 ExtTools、不预装 Deno / Python、不访问源码与构建缓存，安装 Agent 工具包后全部标准入口 smoke 通过（含空格 / 中文参数、重定向、退出码、重定位）；真实 Zone 身份注入与调用；单机 session 仍不能用跨 runtime helper；standard-only 新装、升级 / 修复、加装 ExtTools、ExtTools 回退、标准层缺失时 extended 汇总失败六种组合。

回归：`cargo test -p libopendan`、runtime 相关集成测试；脚本变更 `uv run python -m unittest discover -s tools/tests -p 'test_*.py'`；Web 变更 `pnpm check` / `pnpm lint` 与受影响 e2e；`src/` 下按仓库 AGENTS.md 执行。一期结果见 §4.1；二期、三期只完成源码核对与设计，未核实上游资产。

## 6. Review 决策点

| # | 决策 | 建议值 | 状态 |
| --- | --- | --- | --- |
| 1 | 三层模型（核心随系统 / 扩展 owner 独立升级且全 Agent 生效 / Agent 自装在持久卷）作为骨架 | 是 | ✅ owner 2026-11-05 |
| 2 | 隔离 = 核心路径钉死 + Agent 不污染用户环境，三种形态一套；不宣称 OS 级隔离 | 是 | ✅ owner 2026-11-05（讨论结论） |
| 3 | bare Linux host 作私有允许；安装统一由 node_daemon 做，没有特权 worker，也没有“node-daemon 服务 host 不支持”的例外（见 12） | 是 | ✅ owner 2026-11-05：按建议 |
| 4 | 范围拆三期 + 两份分出去的 TODO；**一期 = Rust + pikg 结构支持**，先开工 | 是 | ✅ owner 2026-11-05：一次把机制建立好，后面专注研究 baseline / ext-tools 带哪些 cmd、什么版本，并通过 pikg 更新；读法：这次开工至少完成到扩展包的 pikg 结构（P1-6） |
| 5 | `EXTENDED_TOOLS` 首版 = aios 现有集合（含 `git / curl / jq / tree / unzip / xz`，它们本来就在镜像里）；ImageMagick 首版 `magick` 优先 `convert` 回退，IM7 由 T0 定 | 是 | ✅ owner 2026-11-05：先按现状；正式清单后面要用正式文档确认（T0 的产出之一） |
| 6 | 单一来源 = Rust 常量 + json 一致性测试，不做生成器 | 是 | ✅ owner 2026-11-05：按建议 |
| 7 | 首版平台：Linux x86_64 / aarch64（容器 + bare host）、macOS arm64 **与 x86_64** Lite、Windows x86_64 Lite | 是 | ✅ owner 2026-11-05：按建议，macOS 要支持 x86（开发 VM 只有 x86） |
| 8 | ~~安装编排复用 Agent task，机器级 `tools/installs/` 记安装事实~~ | — | ❌ 作废（随 12）：安装状态记在 `nodes/<n>/ext_tools` 与本机 `tools/installs/`，不走 Agent task；§3.7 只读 |
| 9 | baseline 声明与探测分离；安装完成由 owner 单独决定是否声明 `extended` | 是 | ✅ 沿用 runtime TODO |
| 10 | 三期 = runtime 裁决 22 的安装包改造的 Agent 工具部分；其它服务的安装包重构另起 | 是 | 待确认 |
| 11 | 扩展包 = pikg 子类 `app_type = ext_tools`，payload 为按平台的 tar.gz + `ext-manifest.json`（`provides / requires / conflicts`）；`ext-core` 是第一个包 | 是 | ✅ owner 2026-11-05 |
| 12 | 安装器 = node_daemon reconciler，对照 Zone 级期望状态 `system/ext_tools/desired`；容器节点写 `buckyos-tools` 卷，seed 镜像只做首启播种；Agent Service 不触发安装 | 是 | ✅ owner 2026-11-05（v2 第 3 条据此作废） |
| 13 | skill 加机器可读 `commands` front matter；系统凭目录 `provides` 反推推荐包；推荐不 block，自备命令按 `provided_by: system` 计可用 | 是 | ✅ owner 2026-11-05（§0.3 第 3、4 点） |
| 14 | 扩展包首版对所有 Agent 生效，不做 Agent 级绑定；`tools/bin` 按授权渲染留作将来 | 是 | ✅ owner 2026-11-05：后续才会有 Agent 可以用哪些 cmd / skills 的配置，目前全生效 |
| 15 | 期望状态经 scheduler 投影成 `NodeConfig.ext_tools`，node_daemon 不直读 Zone 级键（§3.3） | 是 | ✅ owner 2026-11-05：按建议 |
| 16 | 不持久化 `system/ext_tools/catalog`；Resolve 直接查 Repo，期望状态记 `pikg_digest`（§3.3 / §3.9.2） | 是 | ✅ owner 2026-11-05：按建议 |
| 17 | `InstallGrant::System` 重定义为“skill 自装可写 Agent 目录之外”，不再指 ExtTools（§3.6） | 是 | ✅ owner 2026-11-05：按建议 |
| 18 | 新 `AppType::ExtTools` + 子包键 `ext_tools`；闲置的 `agent_tools` 子包键一期删除（§3.9.1） | 是 | ✅ owner 2026-11-05：按建议 |
| 19 | 后台检查只记命中路径，版本在 `check --full` 与安装后记（P1-7） | 是 | ✅ owner 2026-11-05：按建议 |
| 20 | 开发态 helper 来自可执行文件旁记 `provided_by: dev`，算 standard 就绪（P1-7） | 是 | ✅ owner 2026-11-05：按建议 |
| 21 | `ext-core` payload 构成：有官方归档的工具逐个下载并固定 SHA256；没有官方二进制的一组装进一个 conda-forge 前缀（conda-lock 锁、conda-pack 重定位）；自己编译只留给重定位失败的平台（§3.10） | 是 | ✅ owner 2026-11-06：按建议 |
| 22 | GitHub Release（`ext-core-v<ver>` + `ext-core-latest`）是真相源，CI 镜像到 R2；安装包打包、在线安装、开发环境都从 R2 取，安装时不访问 github.com（§3.10） | 是 | ✅ owner 2026-11-06：按建议 |
| 23 | 安装器只把 tar 放到 `bin/ext-tools/`，由 node_daemon 在 `tools/installs/ext-core.json` 缺失时播种（X3 子集）；装过之后以期望状态为准，安装包不覆盖（§3.10） | 是 | ✅ owner 2026-11-06：按建议 |
| 24 | 清单策略：机制优先；首版 `ext-core` 可以宽，实现要的工具都可以先放进去，但每条命令必须先写进 [paios容器需求.md §6.4](../doc/app_service/paios容器需求.md) 表 B；各扩展包的内容同样以 §6.4 表 C 为唯一事实来源 | 是 | ✅ owner 2026-11-06 原话：“核心是先把机制建立起来，具体 baseline 和各个扩展包里有哪些工具要详细文档化” |

15–20 已按建议值闭合（owner 2026-11-05）；21–24 闭合（owner 2026-11-06）；只剩 10 待确认，不影响一期与二期排期。

Review 已通过（2026-11-05，除 10 外全部闭合）：一期立即执行；二期在 T0 / E0 后；三期与安装包 TODO 对齐后再排。runtime 生命周期 TODO 的其它 P2 / P3 / P4 任务不随之启动。
