# Agent Runtime 基线工具全平台统一分发 TODO（三层模型，待 Review）

> 2026-11-05，v2.1。v2.1 加 §0.3 与 §3.9：owner 从场景出发把 ExtTools 升级成**扩展包体系**（系统管理员为全 Zone 的 Agent 管理共用扩展包，包之间有依赖，勾选即装，长期运维），并据此把安装器定为 node_daemon 的 reconciler（§3.3）、skill 改为只声明依赖的命令（§3.9.3）。v1 针对 [runtime 生命周期 TODO R16](opendan-agent-runtime-lifecycle-todo.md#6-实施任务分期) 细化 R-S1 / R-S3，把 standard 分发、Zone 共享可选包、Desktop Lite 隔离、macOS VM 自初始化都装了进来。v2 按 owner 的**三层模型**（§0）与**隔离讨论**（§0.2）重排：三层是骨架；隔离只有“核心路径钉死”和“Agent 不污染用户环境”两件事，对容器、Desktop Lite、bare Linux host 一套约定；范围拆成三期加两份分出去的 TODO，第一期只改 Rust、现在可开工。尚未实施；以当前仓库源码为依据（§2 的事实已逐条核过）。相关：[skills 体系 TODO §3.6.1 C](opendan-skills-system-todo.md)（S19 等本文第一期）、[path_usage.md](../doc/path_usage.md)、[Session Runtime](<../doc/opendan/OpenDAN Agent-Session-Runtime.md>) §2.1 / §3.2 / §3.3、[Desktop Lite TODO](buckyos-desktop-lite-todo.md)、[native Agent Service TODO](opendan-native-agent-service-todo.md)。

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
| **一期（只改 Rust，现在可开工）** | `EXTENDED_TOOLS` 清单；`check_baseline` 按声明选集合；`tools_check` 记命中路径与版本；`xagent runtime baseline --level`；plan 全表显示扩展层状态；回写 runtime TODO R-S1 / R-S3 | 无。解除 skills S19（ExtTools 命令拒装）与 plan 全表的依赖 |
| **二期：扩展包体系 + 安装器** | 扩展包格式（pikg 子类）、Zone 级目录与期望状态、node_daemon reconciler 在容器节点 / bare Linux / macOS / Windows 上安装、升级、修复到 `$BUCKYOS_ROOT/tools/`；管理页勾选即装；skill 的命令依赖 → 推荐包；BuckyOS 发起的进程环境约定（§3.6）落到三种形态 | 一期；T0 可行性 |
| **三期：standard 分发（= runtime 裁决 22 的“安装包改造”）** | Agent 标准工具的产物、安装、升级、修复三平台统一；host / aios 共用组装器 | 二期的安装器与环境约定 |
| 分出去 | Zone 共享可选包（v1 §3.6 / E1）→ 新 TODO；macOS VM 由 Agent 自初始化（v1 §3.8 / E3）→ 新 TODO。两者与 R16 没有实现依赖 | — |

保留的约束：`extended ⊇ standard`；owner 的 baseline 声明与实际探测分离，安装成功不自动改声明，不改已冻结 Session；`agent-session` 属于 standard，Desktop Lite 与 bare host 也装（runtime 裁决 22）；form B daemon 设备不支持安装；没有受管 BuckyOS 安装根的 SSH 目标拒绝安装、仍可探测。

与 runtime 裁决 22 的关系：三期就是它说的“安装包改造”，但本文只覆盖 Agent 两层工具的构建 / 安装 / 升级链路，不做其它服务的全面安装包重构；22 里“综合各基础服务需求统一调整”的那部分由安装包 TODO 另起，本文三期作为它的输入之一。

## 2. 现有实现与缺口（2026-11-05 核对）

| 位置 | 已有 | 本文要补 |
| --- | --- | --- |
| `lib_opendan/src/baseline.rs` | `STANDARD_TOOLS`（31 条，含 `session_helper_of`）、`required_commands(overlay)`、`missing_probe` / `missing_locally`、host 入口 `install_entries` 硬链接或复制、`run_deno_command(root, …)` 钉死 Deno、`session_identity_env`、`AGENT_TOOLS_BIN = bin/agent-tools`、`AGENT_TOOLS_LIBEXEC = libexec/buckyos-agent` | `EXTENDED_TOOLS`；探测返回命中路径与版本 |
| `lib_opendan/src/runtime/mod.rs` | `check_baseline` 只查 standard（`required_commands(record.is_private())`）；`check_commands` → `ToolsCheck {present, missing}` 只记名字；`runtime_exec / runtime_put / runtime_op`，私有侧前置 `<agent_root>/tools`；`runtimes_view` 已渲染 `tools_check.present` 与 `usage.skill_commands` | 按声明选集合；`ToolsCheck` 加 `resolved: {name: {path, version, provided_by}}`；安装编排不复用 `runtime_exec` 的短超时 |
| `lib_opendan/src/xagent_cli.rs` | `runtime baseline` / `check --commands` / `op`；`task state\|cancel` | `baseline --level`；`ext-tools status\|install\|check` |
| `build_aios` | Ubuntu 24.04；`RUNTIME_PACKAGES`（apt）：`bash curl git jq rsync tree unzip xz-utils python3`（系统 3.12，uv 用它）、`ffmpeg`、`imagemagick`（**IM6**，入口 `convert / identify / compare`，没有 `magick`）、`poppler-utils`、`tesseract-ocr`；Node 官方 tarball（`NODE_VERSION`）；Deno `v2.9.2` 在镜像里；pandoc 不在镜像。官方清单与场景的事实来源是 [paios容器需求.md §6.4](../doc/app_service/paios容器需求.md) | 扩展层版本来源与 `EXTENDED_TOOLS` 一致性测试 |
| `publish/exttool/`（`build_exttool`） | ubuntu builder → busybox seed；`deno-cache` / `uv-cache` 预热（只带缓存不带 deno 二进制）；pandoc 3.12.1 固定 SHA（amd64 / arm64）；FreeCAD 可选，缺省 `exit 127` 的 stub；卷 `buckyos-exttool-<id12>` 首次挂载时复制 | macOS / Windows / Linux 原生包；锁文件 |
| `publish/aios/entrypoint.sh` | `BUCKYOS_EXTTOOL_DIR=$BUCKYOS_ROOT/tools`，`tools/bin` 前置 PATH | 与 §3.6 的顺序表对齐 |
| `node_executor/src/ops.rs`、`node_daemon/src/node_executor_host.rs` | 执行器构造 PATH / env；启动时 `opendan install-agent-tools` 物化 `bin/agent-tools` 入口 | host 前置 `tools/bin`；环境变量约定；`describe` 报安装根 |
| `src/buckyos-build.py::_prepare_agent_tools`、`src/tools/prepare_sdk_tool_distribution.py` / `build_sdk_tool_distribution.py` | 复制 TS 命令与 vendor 到 `libexec/buckyos-agent`，重写 import；SDK / CLI 分发 | 三期：host / aios 共用组装器 |
| `opendan/src/home.rs`、`service.rs`、`web/src/pages/Workspaces.tsx` | runtime 设置与卡片 | owner 安装 API；卡片显示扩展层状态 |
| `doc/path_usage.md` | `/opt/buckyos/tools/{store,deno-cache,uv-cache}` 只读；`/opt/buckyos/instance/{pkg,cache,tools/bin,tools/<agent_id>/<sid>}` 可写 | 原生落点与容器稳定路径一致（§3.2） |
| Session Runtime 文档 §3.2 / §3.3 | 列了 aios 附加工具与 ExtTool 工具；“`extended` 是任意额外环境”的旧描述 | 改为 `EXTENDED_TOOLS` 核心清单 + 可选 |

`runtime_put` 不等于有写 `$BUCKYOS_ROOT/tools` 的权限；`runtime_exec` 有超时上限且没有持久 task（runtime R-W5 的“超时转 task”未做）。所以 runtime TODO R-S1 原文“经 `runtime_put / runtime_exec` 装到目标”不成立，一期回写。

## 3. 方案

### 3.1 扩展层清单（`EXTENDED_TOOLS`）

**单一来源用仓库现有模式**：Rust 常量 `baseline::EXTENDED_TOOLS` 手写，核心扩展包 `ext-core` 的 `ext-manifest.json`（§3.9.1）`provides` 由单测校验与常量一致，和 `STANDARD_TOOLS` ↔ `commands/*.ts` 的测试同一做法；不做生成器。`extended` 基线 = `ext-core` 已装并核验；其它扩展包不改变 baseline，只改变 `tools_check` 与 skill 状态。锁文件（按 OS / arch 的版本、下载源、SHA256、许可证）只在二期打包时需要，一期不建。

首版核心 = **aios 今天已有的那组**，不扩：

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

产物 `exttools-<version>-<os>-<arch>.tar.zst`（Windows zip），附 manifest、校验和、许可证。格式以“目标平台无需预装扩展工具即可解包”为准。

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

原生先写 `staging/`，校验通过后切换；失败不覆盖旧版。POSIX 可原子切目录；Windows 用版本指针并处理文件占用，不假定 symlink 权限。保留上一版本供恢复。只读包目录与可写缓存分离，缓存走 instance 约定。容器继续按镜像 id 播种只读卷，新包随 exttool 镜像发布并重建卷，不套原生的 `current.json`。

### 3.3 安装权限与执行通路

**安装器 = node_daemon 的 reconciler**（v2.1 修正，取代 v2 的“Agent Service 里的串行任务”）。理由：node_daemon 今天已经拥有 `$BUCKYOS_ROOT`（paios 镜像与 exttool 卷的播种、`install-agent-tools` 的入口物化都在它手里），在每种形态里都是安装根的所有者：容器节点上它管 docker 卷；bare Linux host、Desktop Lite（macOS / Windows）上它以安装 BuckyOS 的账户跑；node-daemon 作为系统服务跑的 host 上它就是管理身份。于是 v2 担心的“执行器身份与安装根所有者不同”的权限缺口不存在：**安装从来不经执行器身份**，Agent Service 与 Agent 子进程只读安装事实。

- **入口**：系统管理员在管理页 / CLI 写 Zone 级期望状态（§3.9.2）；每个节点的 node_daemon 对照期望状态与本机 `tools/installs/`，缺的下载安装、多的（被取消勾选且无依赖者）卸载、版本不符的升级。owner 的 Agent Home / runtime 卡片只显示本机状态与“去管理页勾选”的链接，不直接触发安装。session helper 不开放安装。
- **容器节点**：node_daemon 把包解进节点级卷 `buckyos-tools`（首次由 `ext-core` seed 镜像播种，保留今天的离线首启）；容器只读挂到 `/opt/buckyos/tools`。不再按 exttool 镜像 id 一个卷，不在只读卷内原地更新：reconciler 写卷、容器重启后看到新内容（§3.2 的 staging 切换在卷内做）。
- **原生（bare Linux host、Desktop Lite、node-daemon 服务 host）**：同一套 reconciler 写 `$BUCKYOS_ROOT/tools/`；不可写时在节点状态里报具体权限问题，禁止静默 sudo。
- **form B、普通 SSH 目标**：没有 node_daemon，没有受管安装根，**不参与扩展包**；仍可探测，`tools_check` 报 `provided_by: system`。

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

“环境不冲突”是目录与进程环境约定，不是 OS 权限隔离；owner 把整台机器给 Agent 时，Agent 在授权内仍可访问用户文件。文档与 UI 文案不得暗示已做到强隔离。

### 3.7 安装编排、API、CLI

```text
xagent runtime ext-tools status <id> [--json]
xagent runtime ext-tools install <id> [--version <v>] [--package <local-file>] [--json]
xagent runtime ext-tools check <id> [--full] [--json]
xagent runtime baseline [--level standard|extended] [--json] [--check]
```

- `install` 返回一个 **Agent task**（复用 `agent-session task state|cancel` 与 task 通知通路，不另造状态机）；阶段 downloading / verifying / staging / checking / activating / succeeded / failed 写进机器级 `tools/installs/<install_id>.json`，Agent 侧 `runtime_ops` 只存引用。服务重启后按 journal 判定，不盲目重放；每目标一把安装锁；同版本同摘要已验证时幂等成功。
- owner API（opendan `home.rs` / `service.rs`）加 status / install / check；CLI 与 UI 共用；`xagent` 直连模式也校验 owner 身份；`agent-session` 不加 install。用户级 `buckyos runtime` 命令归 runtime TODO E7。
- UI：runtime 卡片加扩展层版本 / 状态 / 安装按钮；容器显示“随镜像管理”；owner 默认与私有同机时共享安装事实。E6 搬 Home 后复用。

### 3.9 扩展包体系（回答 §0.3 的四个问题）

#### 3.9.1 扩展包是什么：一种 pikg

用 pikg，不另造格式。理由：签名、DID、Repo 发布、`app install` 的来源解析（本地 `.pikg` / URL / Catalog DID）、按 os / arch 选子包、`docker-image` 子包来源、InstallPlan 的幂等与状态都是现成的；`agent-skills` 已经是这样一个子类。

- **AppDoc**：`app_type = ext_tools`（新类型，约束同 aiws 那样“只接受本类子包”）；`sub_pkgs` 按 selector 一个平台一个 payload（`linux-x86_64`、`linux-aarch64`、`darwin-arm64`、`windows-x86_64`，缺的平台就是“该平台不支持”）；payload 是 `tar.gz`，内容就是解到 `tools/store/<pkg_id>/` 下的树加 `bin/` 里的 launcher。Linux 容器节点不再需要 `docker-image` 子包：reconciler 解 tar 进卷。
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
    "platforms": ["linux-x86_64", "linux-aarch64", "darwin-arm64", "windows-x86_64"],
    "size_bytes": {"linux-x86_64": 123456789},
    "license": "LGPL-2.1", "desc": "音视频加工：ffmpeg / ffprobe"
  }
  ```

  `provides` 是系统对“哪个包提供哪个命令”的全部知识来源（§3.9.3）。`requires` 是包级依赖，装 B 自动带 A；`conflicts` 处理同名命令（两个包提供同一命令必须显式 conflicts，目录里不能同时启用）。包不声明 `writes`：扩展包只写 `tools/store/<id>` 与 `tools/bin`，这是格式约束不是声明。
- **`ext-core`** 是目录里第一个包，内容就是 §3.1 的核心清单；`EXTENDED_TOOLS` 常量与它的 `provides` 一致性测试绑定。今天的 `publish/exttool/` seed 镜像变成 `ext-core` 的 Linux payload 构建脚本（§3.9.2）。
- Agent 层（第三层）不是扩展包：skill 自装走 skills TODO 的 `install.toml`，不进这个体系。

#### 3.9.2 列表、期望状态与构建发布

**扩展包目录（Zone 级）**：Repo 里 `app_type = ext_tools` 的发布投影成 `system/ext_tools/catalog/<pkg_id>`（最新版本、平台、`provides`、`requires`、大小、desc）；官方包随 release 预置，Zone owner 也能 `app publish` 自己构建的包（“行业专用环境”就是这样来的）。

**期望状态（管理员勾选的结果）**：`system/ext_tools/desired/<pkg_id> = {version, enabled, by, at_ms}`。写入规则：勾选 B 时 resolver 做拓扑排序，`requires` 链上没勾的一起写 enabled 并在确认页列出；取消 A 时有启用中的依赖者则拒绝并列出；升级就是改 `version`。写入由 control panel 做（RBAC：zone owner / 系统管理员），走和 App 安装一样的 Resolve → Inspect → 确认 → 提交，但不建 AppSpec、不起服务，提交的是期望状态而不是 InstallPlan。

**每节点状态**：`nodes/<n>/ext_tools/<pkg_id> = {version, digest, stage: queued | downloading | verifying | staging | checking | active | failed | unsupported, error, at_ms}` 由 node_daemon reconciler 写；本机 `tools/installs/<pkg_id>.json` 是同一内容的本地副本（§3.2）。`unsupported` = 该节点平台没有 payload。管理页按包 × 节点显示；Agent 的 runtime 卡片显示本机那一列。

**构建与发布（有了具体需求后的路径）**：

1. `buckyos pikg init --kind ext-tools --id ext-xxx --source ./ext-xxx`：生成目录骨架 `ext-manifest.json` + `payload/<platform>/{store/<id>/, bin/, smoke/}`。
2. 各平台 payload 的来源自定：官方包在 `publish/exttool/<pkg_id>/build.sh`（现有 `install_pandoc.sh` 就是 `ext-core` 里 pandoc 的那段），CI 矩阵按 `platforms` 构建；第三方可手工放二进制。锁定版本、下载源、SHA256 写在 `build.sh` 顶部，和今天 pandoc 的做法一样。
3. `buckyos pikg doctor`：`provides` 的每个命令在对应 payload 的 `bin/` 里有 launcher 且可执行；launcher 不含构建机绝对路径；`smoke` 脚本存在；`requires` 的 id 在目录里；同名命令要么 `conflicts` 要么拒绝。
4. `pikg build / pack` → `app publish`（Repo candidate）→ owner 签名发布，与 App 一致。
5. 管理页看到新版本 → 勾选 / 升级 → reconciler 安装 → `check` → 状态 active → `tools_check` 重跑 → skill 状态重算（§3.9.3）。

长期运维就是 2 → 5 的循环；回滚 = 期望状态改回旧版本，节点保留上一版 payload（§3.2）。

#### 3.9.3 skill 与扩展包：skill 只声明命令，系统反推包

- **skill 声明**：front matter 加机器可读的 `commands: ffmpeg, ffprobe, magick`（只写命令名，逗号分隔）；`requires` 继续给人读；`baseline / platforms` 不变。这改掉 skills TODO 待裁决 14 的答案（原建议“只放 `install.toml`”）：owner 明确 skill 应说明自己依赖的命令，而 runtime R-W6 的“不加字段”针对的是运行位置（`runs_on`），不是依赖声明。官方手册 `aigc_*`、`aiworkspace_run`（`jq`）等补上。
- **系统知识**：`system/ext_tools/catalog/*.provides` 给出 command → 包 的映射；`tools_check.resolved` 给出本机每个命令的实际来源（`exttools | system | agent | 缺`）。
- **反推**：`skills.install_plan`（skills S18）对每个 `commands` 条目查目标 runtime 的 `tools_check.resolved`；缺的查目录 `provides` → 得到 `recommended_packages: [{id, version, requires_chain, provides: [缺的命令]}]`；命中多个包时按“已启用的依赖最少”排序；没有任何包提供的命令报“无扩展包提供，需自备”。推荐同时出现在 CLI 的 install plan、WebUI Skills 页与 `skill list --all` 的状态列。
- **skill 状态**（skills_state 的 `install.runtimes[id]` 扩展）：`commands: {ffmpeg: {present: true, provided_by: exttools}, magick: {present: false}}`、`recommended_packages`。三种可见状态：`ready`（命令齐）、`partial(缺 x, y；建议 ext-media)`、`ready(system)`（命令齐但来自用户自备）。扩展包 active / 卸载 → node 状态变 → runtime `tools_check` 重跑 → skills 状态重算（runtime R6 / skills S21 的事件路径，这就是它的第一个真实触发源）。plan 全表与 `## Commands` 末尾说明用同一数据。
- **方向反过来也成立**：管理页勾选一个包时，可以列出“Zone 内哪些 Agent 的哪些 skill 会因此从 partial 变 ready”（扫各 Agent 的 skills_state），这是运维看得见收益的地方；首版可只做包 → 命令 → skill 名的静态反查。

#### 3.9.4 不 block

- skill 安装永远不因缺命令失败：无自装命令的 skill 确认后直接 `enabled`，状态里带 `partial` 与推荐；有自装命令的按 skills TODO §3.6.1 C。
- 冻结仍只按 `baseline / platforms` 过滤（runtime M5）；`partial` 的 skill 照样能被 `--cmd-skill` 点名，手册末尾写“缺 `magick`，建议 owner 安装 ext-image，或在该 runtime 上自备”。
- 用户在自己的 host 上用自己的方法装了 ffmpeg：`check` 命中 `provided_by: system`，skill 变 `ready(system)`，不要求走扩展包；`extended` 声明仍由 owner 决定，不自动改。
- 扩展包对所有 Agent 生效，首版不做 Agent 级的包绑定；paios 容器需求 §9 的“按授权渲染 `tools/bin`”保留为将来按 Agent 限制的机制，不在本轮。

### 3.8 三期：standard 分发（原 v1 §3.1.1，保留要点）

- 清单沿用 `STANDARD_TOOLS`，导出机器可读 manifest，每条标 provider、入口类型（native multicall / TS / SDK CLI / session overlay）、版本、运行依赖、smoke；与 `commands/*.ts` 的一致性测试保留。
- 产物 `agent-tools-<version>-<os>-<arch>`：multicall 提供程序、入口、`buckyos-agent` TS 源码与 vendor、SDK / CLI、固定版本 Deno、离线依赖；解压后不依赖源码树或构建机缓存；与 OpenDAN / SDK / 协议记共同 release id。
- 复用 SDK 分发的 manifest、固定 Deno 与准备脚本，不另造下载流程。抽共享组装器供 `buckyos-build.py` 与 `build_aios` 使用；保留 `bin/agent-tools`、`libexec/buckyos-agent`、`libexec/buckyos-tool` 的契约；禁止把构建路径写进 launcher。
- 入口：POSIX 链接；Windows hardlink / copy，不依赖 symlink 特权；保留参数边界、退出码、Unicode、stdio 与取消语义；不加经 shell 拼接参数的包装层。PowerShell、cmd、POSIX shell 分别验证真实入口。
- 身份：保留 `session_identity_env` 与经 Session token 取服务身份；安装包不带 token / 私钥；`agent-session` 进 standard 包但只有私有 default session 的 helper 工作。
- 安装与升级：随系统首次安装与升级部署；`install-agent-tools` 保留为入口物化 / 修复步骤，不代替 payload 部署与版本校验；缺入口或依赖时报 standard 不完整；升级 staging 后切换，处理 Windows exe 占用，需要重启时由服务生命周期协调。standard 缺失时 `extended` 汇总仍失败。

## 4. 任务

### 一期（只改 Rust，现在可开工）

- [ ] **P1-1 `EXTENDED_TOOLS`**：`baseline.rs` 加常量（§3.1 核心 + 推荐标记）、`publish/exttool/tools.json` 与一致性单测；`required_commands` 加 level 参数。验收：常量与 json 不一致时测试失败。
- [ ] **P1-2 按声明检查**：`check_baseline` 按 `caps.baseline` 选集合；`BaselineCheck` / `ToolsCheck` 加 `resolved {path, version, provided_by}`；探测脚本（POSIX / PowerShell）输出路径与版本；核心命中路径不在 `$BUCKYOS_ROOT` 下记 `provided_by: system` 且 standard 不算就绪。验收：`runtimes.rs` 与 `remote_node_runtime` 用例覆盖 standard / extended、system 命中、stub 127。
- [ ] **P1-3 CLI 与视图**：`xagent runtime baseline --level`、`check --commands` 输出路径版本；`runtimes_view` 的 plan 全表加 ext 状态段；WebUI 卡片显示 `baseline_check` 的 ext 部分（只读）。
- [ ] **P1-4 回写**：runtime TODO R-S1（安装通路改 §3.3）、R-S3（已做）、裁决 22（三期 = 安装包改造）；Session Runtime 文档 §3.2 / §3.3 改为核心 + 推荐 + 可选；skills TODO S19 的依赖标“已解除”。
- [ ] **P1-5 host 执行器前置 `tools/bin`**（§3.6 的 PATH 顺序，先只做顺序，不做环境变量）：`node_executor ops.rs`；验收：host 上 `check_baseline(extended)` 与真实 session shell 看到同一 ffmpeg。

### 二期：扩展包体系、安装器与环境约定

- [ ] **X1 扩展包格式**：`app_type = ext_tools` 的 AppDoc 约束、`ext-manifest.json` schema（`provides / requires / conflicts / platforms`）、`pikg init --kind ext-tools` 骨架、`pikg doctor` 规则（§3.9.1 / §3.9.2 第 3 条）；`ext-core` 按此格式重打，`EXTENDED_TOOLS` 与其 `provides` 的一致性测试接上。落点：`buckyos-websdk/cli/modules/pikg.ts`、App 安装协议 §2.2、`publish/exttool/ext-core/`。
- [ ] **X2 目录与期望状态**：Repo 投影 `system/ext_tools/catalog`；control panel `ext_tools.set_desired`（拓扑排序、依赖自动勾选、卸载拒绝、RBAC）；`nodes/<n>/ext_tools/<pkg>` 状态 schema。落点：`control_panel/`、system config 键与 RBAC、`buckyos-api`。
- [ ] **X3 node_daemon reconciler**：对照期望状态下载（经 Repo / NDM）、校验、staging、激活、卸载、升级、回滚，写节点状态与 `tools/installs/`；容器节点写 `buckyos-tools` 卷、`ext-core` seed 镜像只做首启播种；三平台原生同一代码。取代 v2 的 T3。落点：`node_daemon/src/paios_images.rs` 拆出 `ext_tools.rs`。
- [ ] **X4 管理页与 CLI**：Desktop / control panel 的“扩展包”页（包 × 节点状态、勾选、升级、依赖提示、受益 skill 静态反查）；`buckyos ext-tools list|enable|disable|status`。Agent runtime 卡片只显示本机状态与链接。
- [ ] **X5 skill 命令依赖**：`SkillMeta.commands` 解析；`skills.install_plan` 的 `recommended_packages`；skills_state 的 `commands / recommended_packages`；扩展包状态变化 → `tools_check` → skills 重算（与 runtime R6 / skills S21 同一条路）；官方手册补 `commands`。落点：`protocol/behavior.rs`、`skills.rs`、skills TODO S18 / S20 / S21。

- [ ] **T0 可行性**：逐工具核三个平台的发行源、许可证、动态依赖、签名；决定 IM7 还是 `magick / convert` 二选一、推荐项是否升核心；产出锁文件草案与支持矩阵（首版：Linux x86_64 / aarch64 容器 + bare host，macOS arm64 Lite，Windows x86_64 Lite；macOS x86_64 随 CI 顺带，Windows arm64 不支持）。缺资产的平台明确阻塞，不输出 stub。
- [ ] **E0 目录与映射定稿**：容器 / 原生的 `tools/`、instance、Agent 缓存目录逐项列出；path_usage.md 更新；验收容器重建后自装工具仍在、rootfs 非持久写入不被误报成功。
- [ ] **T2 构建发行**：`publish/exttool/` 与 `build_exttool` 加 macOS / Windows / Linux 原生打包与 CI；保留 Docker prepare-context 接口；包带库、数据、launcher、许可证。验收：干净环境离线解包跑最小 smoke，不借构建机 PATH。
- [ ] **T3 安装器细节（并入 X3）**：staging、校验、锁、journal、激活、失败恢复、版本保留；Windows 文件占用。验收：不可写根不走 sudo 而是报状态；同节点并发 reconcile 不损坏公共目录；旧版本可运行。
- [ ] **T4 环境约定落地**：§3.6 的 PATH 顺序与环境变量在 `node_executor ops.rs`、`entrypoint.sh`、`runtime_exec`、session overlay 四处一致；同名冲突检查；用户预装不同版本 python / node / deno 时 Agent 用受管版本、用户终端不变。验收：安装前后比较用户 PATH、shell profile、全局包配置无变化。
- [ ] **T5 Agent 侧 API / CLI**：§3.7 缩为只读：`ext-tools status | check`，install 入口改为指向管理页（X4）；`xagent runtime ext-tools install` 只保留开发态直连；安装不改声明。验收：节点状态变化后 Agent 侧 `status` 与 `tools_check` 一致。
- [ ] **T6 UI**：卡片状态、安装、完整检查；容器 image-managed；同机共享安装事实。
- [ ] **T7 文档**：Session Runtime §3、Jarvis 手册中 ImageMagick 旧命令与额外依赖、path_usage.md、本文 §2。

依赖：T0 与 E0 并行 → X1 → T2（按 X1 格式打包）→ X2 → X3（含 T3）→ X4 / T5 / T6；X5 依赖 X2 的目录与一期的 `tools_check.resolved`，可与 X3 并行；T4 依赖 E0，可与 T2 并行；T7 最后。T0 若证实某平台成本过高，带具体缺项回 Review 改清单，不自行降标准。

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

**一期**：`cargo test -p libopendan baseline`、`--test xagent_cli`、`runtimes.rs` 用例通过；声明 `extended` 而缺一个核心工具时检查失败；stub 不计成功；用户 PATH 里的同名核心命令记 `system` 且 standard 不就绪；检查不改声明与已冻结 Session；plan 全表能看到 ext 状态。

**二期**：三个平台在无预装扩展工具的干净环境离线安装并跑媒体、图像、PDF、OCR、Pandoc、Python、JS smoke；Windows 覆盖空格与中文路径、不依赖 Developer Mode；过期 / 只读 / form B / SSH / node-daemon 服务 host 拒绝并给原因；摘要错误、错误架构、磁盘不足、中途退出、文件占用、并发安装都保持旧版可运行；重启后查询一致；真实 work shell、`agent-session run --runtime` 与 probe 看到同一版本，私有 PATH 不带到目标机；用户 PATH、profile、全局配置安装前后无差；预装不同版本工具的机器上 Agent 用受管版本、用户终端不变。

**三期**：干净系统不装 ExtTools、不预装 Deno / Python、不访问源码与构建缓存，安装 Agent 工具包后全部标准入口 smoke 通过（含空格 / 中文参数、重定向、退出码、重定位）；真实 Zone 身份注入与调用；单机 session 仍不能用跨 runtime helper；standard-only 新装、升级 / 修复、加装 ExtTools、ExtTools 回退、标准层缺失时 extended 汇总失败六种组合。

回归：`cargo test -p libopendan`、runtime 相关集成测试；脚本变更 `uv run python -m unittest discover -s tools/tests -p 'test_*.py'`；Web 变更 `pnpm check` / `pnpm lint` 与受影响 e2e；`src/` 下按仓库 AGENTS.md 执行。当前只完成源码核对与设计；未构建包、未跑产品测试、未核实上游资产。

## 6. Review 决策点

| # | 决策 | 建议值 | 状态 |
| --- | --- | --- | --- |
| 1 | 三层模型（核心随系统 / 扩展 owner 独立升级且全 Agent 生效 / Agent 自装在持久卷）作为骨架 | 是 | ✅ owner 2026-11-05 |
| 2 | 隔离 = 核心路径钉死 + Agent 不污染用户环境，三种形态一套；不宣称 OS 级隔离 | 是 | ✅ owner 2026-11-05（讨论结论） |
| 3 | bare Linux host 作私有允许；安装统一由 node_daemon 做，没有特权 worker，也没有“node-daemon 服务 host 不支持”的例外（见 12） | 是 | 待确认 |
> 按建议
| 4 | 范围拆三期 + 两份分出去的 TODO；一期只改 Rust 先开工 | 是 | 待确认 |
> 一次把机制建立好，后面可以专注于研究 baseline/ext-tools里带哪些cmd，cmd是什么版本，并通过pikg更新

| 5 | `EXTENDED_TOOLS` 首版 = aios 现有集合（含 `git / curl / jq / tree / unzip / xz`，它们本来就在镜像里）；ImageMagick 首版 `magick` 优先 `convert` 回退，IM7 由 T0 定 | 是 | 待确认 |
> 先按现状吧，这个后面肯定要用正式的文档确认一下的

| 6 | 单一来源 = Rust 常量 + json 一致性测试，不做生成器 | 是 | 待确认 |
> 按建议

| 7 | 首版平台：Linux x86_64 / aarch64（容器 + bare host）、macOS arm64 Lite、Windows x86_64 Lite | 是 | 待确认 |
> 按建议,macos要支持x86 (我们的虚拟机开发环境只有x86)

| 8 | 安装编排复用 Agent task，机器级 `tools/installs/` 记安装事实 | 是 | 待确认 |

| 9 | baseline 声明与探测分离；安装完成由 owner 单独决定是否声明 `extended` | 是 | ✅ 沿用 runtime TODO |
| 10 | 三期 = runtime 裁决 22 的安装包改造的 Agent 工具部分；其它服务的安装包重构另起 | 是 | 待确认 |
| 11 | 扩展包 = pikg 子类 `app_type = ext_tools`，payload 为按平台的 tar.gz + `ext-manifest.json`（`provides / requires / conflicts`）；`ext-core` 是第一个包 | 是 | 待确认 |
| 12 | 安装器 = node_daemon reconciler，对照 Zone 级期望状态 `system/ext_tools/desired`；容器节点写 `buckyos-tools` 卷，seed 镜像只做首启播种；Agent Service 不触发安装 | 是 | 待确认（v2 第 3 条据此作废） |
| 13 | skill 加机器可读 `commands` front matter；系统凭目录 `provides` 反推推荐包；推荐不 block，自备命令按 `provided_by: system` 计可用 | 是 | ✅ owner 2026-11-05（§0.3 第 3、4 点） |
| 14 | 扩展包首版对所有 Agent 生效，不做 Agent 级绑定；`tools/bin` 按授权渲染留作将来 | 是 | 待确认 |

Review 通过后：一期立即执行；二期在 T0 / E0 后；三期与安装包 TODO 对齐后再排。runtime 生命周期 TODO 的其它 P2 / P3 / P4 任务不随之启动。
