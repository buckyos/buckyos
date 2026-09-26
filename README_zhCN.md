# BuckyOS Beta2.2 (0.7.0)

[English](README.md) | **简体中文**

BuckyOS 是一个开源的个人 AI 操作系统，将你的设备、应用、数据和 AI Agent 整合到一个 **Zone** 中，构成由你掌控的个人云。

当前源码面向 **Beta2.2 / 0.7.0**，计划于 **2026 年 10 月 15 日**正式上线。这是一次包含不兼容变更的版本更新：系统配置、身份、应用打包和 Agent 配置均有调整，不保证与早期 Beta 版本向后兼容。

## Beta2.2 的主要功能

- **Web Desktop 与 Control Panel**：统一的桌面环境，支持应用窗口、设置、用户与 Agent 管理、AI Center、Task Center 和应用管理。
- **FileBrowser 与 Preview**：内置文件浏览和预览界面。FileBrowser 通过 `nfs-server` 执行文件操作，后台复制任务由 Task Manager 管理。
- **MessageHub 与 Message Center**：内置消息应用，以 `msg-center` 为后端，支持会话历史、附件、已读状态，以及会话归档、恢复和删除。服务层提供基于 DID 的收发箱、联系人、自托管群组、原生消息通信和 Telegram 通道。
- **OpenDAN 与 Jarvis**：基于 Rust 的 Agent Runtime，支持可配置的会话类别与行为循环、记忆和工作区工具、消息与事件路由、任务委派，以及请求人工输入。Jarvis 以带版本号的 `.pikg` 应用包交付，提示词、行为配置和翻译资源位于 [`src/apps/jarvis_runtime/agent`](src/apps/jarvis_runtime/agent)。
- **AI Compute Center（AICC）**：提供服务商与模型管理、逻辑模型路由、用量日志，以及文本和媒体能力的适配器。桌面中的 AI Center 提供相应的管理界面。
- **Workflow 与 Task Manager**：支持工作流定义与运行、计划任务、任务进度与控制，以及人工审批或介入。Workflow 和 OpenDAN 与 Task Manager、`kevent` 集成，交换任务状态更新。
- **应用交付与 SDK/CLI**：支持 `.pikg` 包、经过校验的安装计划、按用户区分的应用实例和网关路由。TypeScript SDK 与 `buckyos` CLI 一起构建并随系统分发；Rust 服务使用 `buckyos-api`。
- **系统基础设施**：`system-config`、调度器和 `node-daemon` 管理系统目标状态与部署；`verify-hub` 和 RBAC 负责登录与授权。`kmsg` 和 `kevent` 提供消息与事件通知，`repo-service` 和命名对象存储支持内容交付。
- **开发工作流**：包含各平台的构建与打包流水线、本地开发验证（DV）测试，以及 [`harness/`](harness/README.md) 中的贡献者指南。

### 当前限制与进行中的工作

Beta2.2 仍在持续完善。服务或界面的存在并不代表所有规划能力均已完成：

- 当前运行的 Workflow 服务将定义、运行状态和对象存储保存在内存中。计划任务定义已镜像到 Task Manager，但工作流的持久化恢复，以及 `func::*` 到调度器的调用链路仍待完成。
- 调度器已有 FunctionObject/Thunk 支持，但核心中仍保留 OPTask，尚未完成用 Function Instance 替代 OPTask 的计划。
- Telegram 是当前仓库已实现的外部消息通道。Lark 和其他外部通道仍属于后续工作。

**欢迎提交 issue 和 pull request，一起构建下一代个人 AI 操作系统。**

## 开始使用

源码支持在 macOS、Linux 和 Windows 上构建。以下命令使用 macOS/Linux shell。BuckyOS Desktop 是面向 Mac/Windows 的桌面发行版；Linux 构建面向服务器和开发环境。

### 第 1 步：获取源码并准备环境

将以下仓库克隆到同一父目录，统一使用 `main` 分支：

```bash
git clone --branch main https://github.com/buckyos/buckyos.git
git clone --branch main https://github.com/buckyos/cyfs-gateway.git
cd buckyos
```

需要开发 SDK/CLI 时，可在同一父目录执行 `git clone --branch main https://github.com/buckyos/buckyos-websdk.git`。该仓库是可选的；缺少时构建会使用 npm 已发布的 `buckyos@latest`。

构建需要稳定版 Rust 工具链、当前平台的 C/C++ 构建工具、Python 3.12+、`uv`、Node.js 及 npm 和 pnpm，以及 Deno。容器应用需要 Docker，开发工具会用到 tmux。当前 CI 使用 Node.js 24、pnpm 10.13.1 和 Deno 2.9.2。

可以先查看并运行 [`devenv.py`](devenv.py)，安装当前平台的基础依赖：

```bash
python3 devenv.py
```

根目录的 `pyproject.toml` 让 `uv run` 自动解析 `buckyos-devkit`，无需手工创建单独的虚拟环境。

### 第 2 步：构建并安装 cyfs-gateway

从 `buckyos` 仓库根目录执行：

```bash
cd ../cyfs-gateway/src
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main" buckyos-build
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git@main" buckyos-install --all
cd ../../buckyos/src
```

### 第 3 步：构建 BuckyOS

在 `buckyos/src` 下执行：

```bash
uv run buckyos-build.py
```

该脚本先准备包含 Deno 运行时的 SDK/CLI 分发包，再调用 devkit 构建。同级的 `buckyos-websdk` 目录存在时从源码构建，不存在时下载 npm 已发布的 `buckyos@latest`。如果 SDK 源码位于其他位置，可通过 `BUCKYOS_SDK_TOOL_SOURCE` 指定。使用 `--skip-web` 或 `-s <module>` 时也会执行这一步。使用发布包需要 npm 和 Deno，无需 SDK 源码或 pnpm；从源码构建还需要 pnpm。本地源码构建失败时直接报错，不会回退到发布包。

构建产物汇总到 `src/rootfs`。**`buckyos-build.py` 不会更新已安装的运行环境。** 使用 `start.py` 将最新产物复制到安装目录并重启系统。发布构建所用的预构建 SDK/CLI 输入，参见 [`src/readme.md`](src/readme.md)。

### 第 4 步：初始化并启动 Zone

从下面两种初始化方式中选择一种，命令均在 `buckyos/src` 下执行。

**本地开发：**

```bash
uv run start.py --all
uv run check.py
```

`--all` 使用 `dev` 配置：Owner 为 `devtest`，Zone 为 `test.buckyos.io`，身份已预先配置，无需手工激活或 SN 中转。在开发机上访问 `http://test.buckyos.io`，并确保该域名及应用子域名解析到本地运行环境。应用开发步骤与测试登录信息见[应用开发指南](doc/sdk/app-dev-quickstart.md)。

**首次安装并通过界面激活：**

```bash
uv run start.py --reinstall release
uv run check.py
```

该命令使用 `buckyos.ai` 环境初始化一个待激活系统，访问 `http://127.0.0.1:3182` 完成激活。`nightly` 配置组则使用 `buckyos.io` 环境。

**仅在明确需要重新初始化系统时使用 `--all` 或 `--reinstall`。** 它们会重置配置和运行状态。安装布局会保留 `data/home`、`data/srv` 和 `storage`，因此重新初始化不等于彻底清空数据。切换 Beta 版本或重置已有系统前，请先备份。

日常更新与重启使用：

```bash
uv run start.py
```

`start.py` 会停止已知的 BuckyOS 进程，更新已安装产物，然后在后台启动 `node_daemon --enable_active`。运行根目录在 macOS/Linux 上默认为 `/opt/buckyos`，在 Windows 上默认为 `%APPDATA%\buckyos`，可通过 `BUCKYOS_ROOT` 覆盖。请确保当前用户具有运行目录与端口所需的权限。源码启动流程不会注册宿主机自启服务。

如果已有运行环境由 systemd、launchd 或 Windows 保活服务托管，请在安装或更新产物前，通过对应的服务管理器停止它。单独执行 `stop.py` 不会禁用自动重启。源码开发与 BuckyOS Desktop 测试应使用独立环境，避免争用相同的运行目录、服务和端口。

### 常用开发命令

在 `buckyos/src` 下执行：

| 用途 | 命令 |
| --- | --- |
| 构建系统与 Web UI | `uv run buckyos-build.py` |
| 跳过 Web UI 构建（仍会构建 SDK/CLI） | `uv run buckyos-build.py --skip-web` |
| 构建指定模块 | `uv run buckyos-build.py -s <module>` |
| 更新已安装产物并重启 | `uv run start.py` |
| 仅重启，不更新产物 | `uv run start.py --skip-update` |
| 检查激活状态与运行健康状况 | `uv run check.py` |
| 停止本地进程 | `uv run stop.py` |
| 在前台调试 Jarvis | `./debug_jarvis.sh` |
| 运行 Rust 单元测试 | `cargo test` |

启动所需的开发环境后，在仓库根目录列出并运行 DV 测试：

```bash
uv run src/check.py
uv run test/run.py --list
uv run test/run.py -p aicc_test
```

### 配置组

`start.py --reinstall <group>` 为指定环境重新生成配置。当前配置组定义在 [`src/devenv_config.ts`](src/devenv_config.ts) 和 [`src/make_config.ts`](src/make_config.ts) 中：

| 配置组 | 用途 |
| --- | --- |
| `dev`、`devtest_ood1` | 预先配置的本地 DV Zone，域名为 `test.buckyos.io` |
| `release` | 使用 `buckyos.ai` 环境的待激活系统 |
| `nightly` | 使用 `buckyos.io` 环境的待激活系统 |
| `vmtest` | 不预置身份的 VM 激活测试 |
| `alice.ood1`、`bob.ood1`、`charlie.ood1`、`dave.ood1` | 分布式测试环境的预设身份与网络场景 |
| `devtests_ood1`、`sn_web` | 测试环境使用的 `devtests.org` OOD |

SN 配置生成已移至 `cyfs-gateway/src/make_sn_config.ts`；BuckyOS 的 `make_config.ts` 不再支持 `sn` 和 `sn_server`。

## BuckyOS 的愿景

- `Internet is BuckyOS`：通过新的去中心化（也必然是开源的）基础设施，构建新的 dApp 生态。应用之间的信息将更加互联，模块化更好，也更适合 AI。它能够支持构建比今天复杂一个数量级的应用，同时将构建和运行成本再降低一个数量级。（生产力提升 100 倍）
- 互联网的基础设施不可以被公司掌握。去中心化的基础设施可以彻底消除平台税和不公平的平台规则。通过 Token 机制，让基础平台被开发者、传教者、所有用户和资本共同拥有，分享税收，并共同协定更公平的平台规则。
- `kill app` 的底层逻辑是“通过 LLM 解决信息筛选的匮乏”。用 AI 来构建信息是少部分人的需求，用 AI 来筛选信息则是所有人都需要的。AI 运用常识帮助用户筛选其接收的信息，解决当今社会的信息茧房问题。用户使用后效果明显，对社会有正面意义。对 AI 行业来说，通过 CYFS 链接所有用户的 KnowledgeBase 形成的语义网络，也能帮助 LLM 在实时、准确的信息基础上得到更好的结果。

### 了解更多 BuckyOS 内容

- [架构与核心概念](doc/arch/README.md)
- [应用开发快速入门](doc/sdk/app-dev-quickstart.md)
- [Rust API Runtime](doc/sdk/buckyos-api-runtime.md)
- [OpenDAN Agent 开发](doc/sdk/OpenDAN_Agent_Dev_Guide.md)
- [源码目录与 SDK/CLI 构建](src/readme.md)
- [运行时目录](doc/path_usage.md)
- [贡献者工作流](harness/README.md)与[仓库开发约定](AGENTS.md)

## 下一代 GPL：创建全新开源协作模型

“开源组织有着悠久的历史和辉煌的成就。实践证明，仅在虚拟世界中协作，就能编写出更好的代码。我们相信软件开发工作非常适合采用 DAO 模式。我们称这种由去中心化组织共同开发软件的 DAO 为 SourceDAO。” —— 引自 CodeDAO 白皮书（[https://www.codedao.ai](https://www.codedao.ai)）

BuckyOS 的开源社区通过 DAO 的方式来运作，我们的目标是解决开源付出没有回报，甚至被白嫖的问题：

- 编码挖矿，通过利益相关提高版本发布质量
- 通过类似 GPL 的传染机制，形成上下游的共同利益结构
- 通过智能合约的自动分账，让支持世界稳定运行的基础库贡献者得到稳定且长久的收入（这是他们应得的）

治理上，通过统一持币、一致的利益关系来统一用户和开发者的共识，在共同的利益共识下做出理智的决定。（吵架也是和自己人吵架）

`公开，透明，来去自由（人人可参与），结果导向`

SourceDAO 是基于以上理念构建的开源 DAO 智能合约。更多详情请访问 [https://dao.buckyos.org/](https://dao.buckyos.org/)。

## 初步版本计划

#### 2024

- **0.1 Demo：** 2.5%（2024 年 6 月已完成）
- **0.2 PoC：** 2.5%（2024 年 9 月已完成）
- **0.3 Alpha1：** 2.5%（2024 年 12 月已完成）

#### 2025

- **0.4 Alpha2：** 2.5%（2025 年 3 月已完成）
- **0.4.1 Alpha3：** 2.5%（2025 年 9 月已完成）
- **0.5.1 Beta1：** 4%（2025 年 12 月已完成）

#### 2026

- **0.6.0 Beta2：** 4%（2026 年 4 月已完成）
- **0.7.0 Beta2.2：** 7.5%（计划于 2026 年 10 月 15 日正式上线）
- **0.8.0 Beta3：** 2.5%（完整的分布式内核版本，计划于 2026 年底上线）

**0.8.0 / Beta3** 的目标是交付完整的分布式内核，计划于 2026 年底上线。相关工作包括分布式存储集成、备份恢复、数据可靠性和系统自恢复。

## 许可证

BuckyOS 是一个自由、开源、去中心化的系统，鼓励厂商基于 BuckyOS 构建商业产品，促进公平竞争。我们的许可选择旨在实现生态系统共赢、保持去中心化核心、保护贡献者利益，并构建一个可持续的生态系统。我们采用双许可证模式：一方面是基于 LGPL 的传统许可，要求内核修改需遵守 GPL（允许封闭源代码应用，但这些应用不能成为核心系统组件）；另一方面是基于 SourceDAO 的许可。当发行 DAO Token 的组织使用 BuckyOS 时，必须根据该许可证向 BuckyOS DAO 捐出一部分 Token。

目前还没有完全符合我们需求的许可证，因此在 DEMO 阶段我们暂时使用 BSD 许可证。我认为当 PoC 完成后，我们肯定会准备好正式的许可证。
