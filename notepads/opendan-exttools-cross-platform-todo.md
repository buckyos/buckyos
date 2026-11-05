# Agent Runtime 基线工具全平台统一分发 TODO（三层模型，待 Review）

> 2026-11-05，v2。v1 针对 [runtime 生命周期 TODO R16](opendan-agent-runtime-lifecycle-todo.md#6-实施任务分期) 细化 R-S1 / R-S3，把 standard 分发、Zone 共享可选包、Desktop Lite 隔离、macOS VM 自初始化都装了进来。v2 按 owner 的**三层模型**（§0）与**隔离讨论**（§0.2）重排：三层是骨架；隔离只有“核心路径钉死”和“Agent 不污染用户环境”两件事，对容器、Desktop Lite、bare Linux host 一套约定；范围拆成三期加两份分出去的 TODO，第一期只改 Rust、现在可开工。尚未实施；以当前仓库源码为依据（§2 的事实已逐条核过）。相关：[skills 体系 TODO §3.6.1 C](opendan-skills-system-todo.md)（S19 等本文第一期）、[path_usage.md](../doc/path_usage.md)、[Session Runtime](<../doc/opendan/OpenDAN Agent-Session-Runtime.md>) §2.1 / §3.2 / §3.3、[Desktop Lite TODO](buckyos-desktop-lite-todo.md)、[native Agent Service TODO](opendan-native-agent-service-todo.md)。

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

## 1. 目标与范围

三期，加两份分出去的 TODO：

| 期 | 交付 | 依赖 |
| --- | --- | --- |
| **一期（只改 Rust，现在可开工）** | `EXTENDED_TOOLS` 清单；`check_baseline` 按声明选集合；`tools_check` 记命中路径与版本；`xagent runtime baseline --level`；plan 全表显示扩展层状态；回写 runtime TODO R-S1 / R-S3 | 无。解除 skills S19（ExtTools 命令拒装）与 plan 全表的依赖 |
| **二期：原生安装器** | macOS / Windows / Linux 同一账户下安装、升级、修复 ExtTools 到 `$BUCKYOS_ROOT/tools/`；owner API / CLI / runtime 卡片；BuckyOS 发起的进程环境约定（§3.6）落到三种形态 | 一期；T0 可行性 |
| **三期：standard 分发（= runtime 裁决 22 的“安装包改造”）** | Agent 标准工具的产物、安装、升级、修复三平台统一；host / aios 共用组装器 | 二期的安装器与环境约定 |
| 分出去 | Zone 共享可选包（v1 §3.6 / E1）→ 新 TODO；macOS VM 由 Agent 自初始化（v1 §3.8 / E3）→ 新 TODO。两者与 R16 没有实现依赖 | — |

保留的约束：`extended ⊇ standard`；owner 的 baseline 声明与实际探测分离，安装成功不自动改声明，不改已冻结 Session；`agent-session` 属于 standard，Desktop Lite 与 bare host 也装（runtime 裁决 22）；form B daemon 设备不支持安装；没有受管 BuckyOS 安装根的 SSH 目标拒绝安装、仍可探测。

与 runtime 裁决 22 的关系：三期就是它说的“安装包改造”，但本文只覆盖 Agent 两层工具的构建 / 安装 / 升级链路，不做其它服务的全面安装包重构；22 里“综合各基础服务需求统一调整”的那部分由安装包 TODO 另起，本文三期作为它的输入之一。

## 2. 现有实现与缺口（2026-11-05 核对）

| 位置 | 已有 | 本文要补 |
| --- | --- | --- |
| `lib_opendan/src/baseline.rs` | `STANDARD_TOOLS`（28 条，含 `session_helper_of`）、`required_commands(overlay)`、`missing_probe` / `missing_locally`、host 入口 `install_entries` 硬链接或复制、`run_deno_command(root, …)` 钉死 Deno、`session_identity_env`、`AGENT_TOOLS_BIN = bin/agent-tools`、`AGENT_TOOLS_LIBEXEC = libexec/buckyos-agent` | `EXTENDED_TOOLS`；探测返回命中路径与版本 |
| `lib_opendan/src/runtime/mod.rs` | `check_baseline` 只查 standard（`required_commands(record.is_private())`）；`check_commands` → `ToolsCheck {present, missing}` 只记名字；`runtime_exec / runtime_put / runtime_op`，私有侧前置 `<agent_root>/tools`；`runtimes_view` 已渲染 `tools_check.present` 与 `usage.skill_commands` | 按声明选集合；`ToolsCheck` 加 `resolved: {name: {path, version, provided_by}}`；安装编排不复用 `runtime_exec` 的短超时 |
| `lib_opendan/src/xagent_cli.rs` | `runtime baseline` / `check --commands` / `op`；`task state\|cancel` | `baseline --level`；`ext-tools status\|install\|check` |
| `build_aios` | Ubuntu 24.04；apt：`python3`（系统 3.12，uv 用它）、`ffmpeg`、`imagemagick`（**IM6**，入口 `convert / identify / compare`，没有 `magick`）、`poppler-utils`、`tesseract-ocr`；Node 官方 tarball（`NODE_VERSION`）；Deno `v2.9.2` 在镜像里；pandoc 不在镜像 | 扩展层版本来源与 `EXTENDED_TOOLS` 一致性测试 |
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

**单一来源用仓库现有模式**：Rust 常量 `baseline::EXTENDED_TOOLS` 手写，`publish/exttool/tools.json`（能力契约）由单测校验与常量一致，和 `STANDARD_TOOLS` ↔ `commands/*.ts` 的测试同一做法；不做生成器。锁文件（按 OS / arch 的版本、下载源、SHA256、许可证）只在二期打包时需要，一期不建。

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

**推荐而非核心**（不计入 `extended` 判定，`tools_check` 可报）：`git`、`curl`、`jq`、`tree`、`unzip`、`xz`。Windows 上随包带 git / Python 的包体与许可证不小，T0 决定是否升为核心。Session Runtime 文档已说 `jq` 不在 standard、纯标准环境用 Deno 替代，维持。`bash`、`rsync`、FreeCAD、编译工具链、Docker、pnpm、字体、额外 OCR 数据都是可选，可选项失败不否定核心。

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

- **原生（Desktop Lite 与 bare Linux host 一样）**：安装根是用户自己的目录，Agent Service、安装器、Agent 子进程同一账户。安装器是 Agent Service 进程里的一个串行化任务，**不是权限边界**，没有特权 worker；不可写时返回具体权限问题，禁止静默 sudo。owner 命令 / API 是入口，session helper 不开放安装。
- **容器**：API 返回 `image_managed` 与配套镜像版本 / 升级入口；不能在只读卷内原地更新。
- **node-daemon 作为服务跑的 Windows / macOS / Linux host（非 Lite）**：执行器身份与安装根所有者可能不同，这是唯一真正有权限缺口的形态。首版**不支持安装**，`extended` 保持声明，API 返回原因（“安装根 `<path>` 由 `<user>` 拥有，请在该机器上以 BuckyOS 管理身份安装”）。runtime R-S1 原文“跑了 node-daemon 的节点装到各自 `$BUCKYOS_ROOT/tools/`”据此改为“同账户的原生形态”。
- **form B、普通 SSH、只读 grant、禁用 / 过期 / retired 目标**：拒绝，不回退到私有。

入口重新读当前 grant（`install = system`）、目标平台、安装根与写权限，不凭 Session 冻结快照授权。安装器只接受锁定的受信资产，防路径穿越与符号链接逃逸；离线包验证同一摘要。R-U3 原语只用于普通身份可做的上传、检查与日志归档。

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

### 二期：原生安装器与环境约定

- [ ] **T0 可行性**：逐工具核三个平台的发行源、许可证、动态依赖、签名；决定 IM7 还是 `magick / convert` 二选一、推荐项是否升核心；产出锁文件草案与支持矩阵（首版：Linux x86_64 / aarch64 容器 + bare host，macOS arm64 Lite，Windows x86_64 Lite；macOS x86_64 随 CI 顺带，Windows arm64 不支持）。缺资产的平台明确阻塞，不输出 stub。
- [ ] **E0 目录与映射定稿**：容器 / 原生的 `tools/`、instance、Agent 缓存目录逐项列出；path_usage.md 更新；验收容器重建后自装工具仍在、rootfs 非持久写入不被误报成功。
- [ ] **T2 构建发行**：`publish/exttool/` 与 `build_exttool` 加 macOS / Windows / Linux 原生打包与 CI；保留 Docker prepare-context 接口；包带库、数据、launcher、许可证。验收：干净环境离线解包跑最小 smoke，不借构建机 PATH。
- [ ] **T3 原生安装器**：同账户任务：staging、校验、锁、journal、激活、失败恢复、版本保留；Windows 文件占用；node-daemon 作服务的 host 返回不支持原因。验收：不可写根不走 sudo；两 Agent 同机并发安装不损坏公共目录；旧版本可运行。
- [ ] **T4 环境约定落地**：§3.6 的 PATH 顺序与环境变量在 `node_executor ops.rs`、`entrypoint.sh`、`runtime_exec`、session overlay 四处一致；同名冲突检查；用户预装不同版本 python / node / deno 时 Agent 用受管版本、用户终端不变。验收：安装前后比较用户 PATH、shell profile、全局包配置无变化。
- [ ] **T5 编排 / API / CLI**：§3.7；复用 Agent task；owner 身份校验；拒绝 session 安装与不支持目标；安装不改声明。验收：长任务跨服务重启可查询。
- [ ] **T6 UI**：卡片状态、安装、完整检查；容器 image-managed；同机共享安装事实。
- [ ] **T7 文档**：Session Runtime §3、Jarvis 手册中 ImageMagick 旧命令与额外依赖、path_usage.md、本文 §2。

依赖：T0 与 E0 并行 → T2 → T3 → T5 → T6；T4 依赖 E0，可与 T2 并行；T7 最后。T0 若证实某平台成本过高，带具体缺项回 Review 改清单，不自行降标准。

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
| 3 | bare Linux host 作私有允许，与 Lite 同一原生安装器；不再有特权 worker；node-daemon 作服务的 host 首版不支持安装 | 是 | 待确认 |
| 4 | 范围拆三期 + 两份分出去的 TODO；一期只改 Rust 先开工 | 是 | 待确认 |
| 5 | `EXTENDED_TOOLS` 首版 = aios 现有集合；`git / curl / jq / tree / unzip / xz` 为推荐；ImageMagick 首版 `magick` 优先 `convert` 回退，IM7 由 T0 定 | 是 | 待确认 |
| 6 | 单一来源 = Rust 常量 + json 一致性测试，不做生成器 | 是 | 待确认 |
| 7 | 首版平台：Linux x86_64 / aarch64（容器 + bare host）、macOS arm64 Lite、Windows x86_64 Lite | 是 | 待确认 |
| 8 | 安装编排复用 Agent task，机器级 `tools/installs/` 记安装事实 | 是 | 待确认 |
| 9 | baseline 声明与探测分离；安装完成由 owner 单独决定是否声明 `extended` | 是 | ✅ 沿用 runtime TODO |
| 10 | 三期 = runtime 裁决 22 的安装包改造的 Agent 工具部分；其它服务的安装包重构另起 | 是 | 待确认 |

Review 通过后：一期立即执行；二期在 T0 / E0 后；三期与安装包 TODO 对齐后再排。runtime 生命周期 TODO 的其它 P2 / P3 / P4 任务不随之启动。
