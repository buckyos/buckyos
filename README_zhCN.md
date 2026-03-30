# BuckyOS Beta2 (0.6.0) 发布！

Beta2 是 BuckyOS 在 AI 时代的一次大更新，主要新增功能如下：

- 两个新的内核组件：`kmsgqueue` + `kevent`，配合使用可以实现高性能的分布式事件通知
- 新增了完整的 BuckyOS Desktop WebUI
- 完成 [OpenDAN](https://github.com/fiatrete/OpenDAN-Personal-AI-OS) 的移植（使用 Rust 重新实现）
  - 内置 Agent Jarvis
  - 提供了 UI-Session <-> WorkSession 的基础体系
  - 实现 Agent-Behavior Loop，相比 skills 能更准确地支持一些“行为模式”
  - 基于“意图引擎”重新设计 Agent Tool，并完成了必要的元工具实现
  - 升级了 Agent Memory 系统，使用 `set_memory` + `topic` 组合，同时支持自动 memory 查询/压缩，以及 Agent 基于文件系统的手工查找
  - 支持基于 TODO List 的 SubAgent 体系
  - 提供 Runtime Sandbox，Agent 之间可以完全可控地隔离
- 新增 AI Computer Center，统一实现集群 AI 能力管理和模型路由
- 新增 Msg Center，对 DID 实体提供统一的 Message Inbox/Outbox 管理，为规划中的两个默认应用 Message Hub 和 Home Station 提供底层支持
  - Msg Center 支持 Msg Tunnel 扩展（已完整支持 Telegram API）
- 新增 Workflow 引擎，支持 Agent-Human-Loop，并作为 Agent 意图引擎的底层（*该组件目前在开发中）
- 完全重构了 ndn-lib 的 Named Store 存储层
- 重新实现 repo-service，从过去的“app 源”升级为通用的“数字内容管理与分发基础服务”
- 已经完成了 CYFS（基于 `cyfs://` 的分布式文件系统）的内核开发，计划在 Beta3 启用
- cyfs-gateway 也有多处更新，丰富了 Server 配置，进一步增强了 process-chain 的能力
- BuckyOS 的集群路由 process-chain 已重写，更加模块化，能够支持更丰富的网关安全能力，从源头保护系统安装
- Rtcp 协议安全升级中，计划在 Beta2 的前两次迭代中完成
- 支持虚拟机管理，并可以把虚拟机分配给 Agent 使用（*该功能目前开发中）
- 调度器支持 Function Instance，取代原规划的 OPTask（*该功能目前开发中）
- BuckyOS TypeScript SDK 成为一等公民，将得到和 Rust SDK 相同的功能（*进行中）
  - 开发者可以选择 TypeScript 或 Rust 来开发 BuckyOS Native App
- 增加了支持 Harness Engineering 的基础设施，我们会在这个版本完全切换到 AI-Native 的开发工作流

**加入我们的征程吧！欢迎随时提交 issue 或 pull request！让我们共同构建下一代分布式 Personal AI 操作系统！**

Beta2 首个版本发布后，我们将进入快速迭代状态，期望每周都有带来用户体验改进的版本发布。
内核方向，我们正在以“第一个商用级、Zero OP 的个人分布式私有云”为目标，推进数据可靠性和系统自恢复相关的工作。该版本规划为 Beta3，计划在 4 月底发布。

## 开始使用

首先获取活跃代码：
[https://github.com/buckyos/buckyos/discussions/70](https://github.com/buckyos/buckyos/discussions/70)

从源码安装是了解 BuckyOS 的好方法，也是迈向贡献的第一步。BuckyOS 支持在 macOS / Linux / Windows 上完成构建。

```bash
git clone https://github.com/buckyos/buckyos.git
```

clone 完成后，先安装 `uv`。仓库根目录现在带有 `pyproject.toml`，因此主开发脚本可以直接通过 `uv run` 拉起 `buckyos-devkit`，不需要先手工创建项目 venv：

如果本机还没准备好开发环境，可以先执行 `python3 devenv.py`。脚本会按当前平台安装 `uv`、`deno`、`tmux` 以及其他基础依赖。

```bash
cd buckyos
uv run src/buckyos-build.py --no-build-web-apps
```

开始构建：构建前可以参考 `devenv.py` 搭建环境。我们主要依赖 `Rust 工具链、Node.js + pnpm、Python 3.12、uv、Deno、tmux、docker.io`。安装成功后执行下面命令开始构建。

### Step 1. 构建 cyfs-gateway

目前 BuckyOS 依赖 cyfs-gateway，因此在运行前需要先从源码构建 cyfs-gateway：

```bash
cd ~/
git clone https://github.com/buckyos/cyfs-gateway.git
cd cyfs-gateway/src
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git" buckyos-build
uvx --from "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git" buckyos-install --all
```

### Step 2. 构建 buckyos

回到 BuckyOS 目录，执行下面命令：

```bash
cd buckyos/src
uv run ./buckyos-build.py
uv run buckyos-install --all
```

### Step 3. 启动 buckyos

首次安装：

```bash
uv run ./start.py --reinstall release
```

源码安装不会自动将 BuckyOS 加入自启服务。后续如需手工启动，请执行：

```bash
uv run ./start.py
```

**注意：千万不要再次执行上面的 `uv run ./start.py --reinstall release`，这会导致系统被 soft reset。**

`start.py` 里实际执行的是下面命令，你可以手工把该命令加入当前系统的自启服务列表：

```bash
sudo /opt/buckyos/bin/node-daemon/node_daemon --enable_active
```

#### 常见坑点与排查（过渡期）

- **经常需要 `cargo update`**：尤其是新环境或依赖锁漂移时。
- **`make_config.py` 依赖 `buckycli`**：通常需要先在 buckyos repo 执行 `buckyos-build && buckyos-install`，确保 `buckycli` 就绪。

### 源码目录的常用脚本

- 下面脚本只进行 Rust 部分的构建：

```bash
cd src
uv run ./buckyos-build.py --no-build-web-apps
```

- 下面脚本只更新编译产物并启动 `/opt/buckyos`：

```bash
cd src
uv run ./start.py
```

- 下面脚本基于指定的配置组重装 BuckyOS：

```bash
cd src
uv run ./start.py --reinstall $group_name
```

如果 `group_name` 为空，则使用空配置文件启动 BuckyOS，此时进入待激活状态。

目前系统带有几组常用配置文件：

- `release`（正式环境，使用 buckyos.ai 的 SN 设施）
- `dev`（无 SN 的开发测试配置，不依赖任何本机外组件）
- `alice.ood1`、`bob.ood1`、`charlie.ood1`：3 个预设身份，均使用计划部署到虚拟测试环境的 `devtests.org` 环境
- `sn`：虚拟测试环境中的 SN

## BuckyOS 的愿景

- `Internet is BuckyOS`：通过新的去中心化（也必然是开源的）基础设施，构建新的 dApp 生态。应用之间的信息将更加互联，模块化更好，也更适合 AI。它能够支持构建比今天复杂一个数量级的应用，同时将构建和运行成本再降低一个数量级。（生产力提升 100 倍）
- 互联网的基础设施不可以被公司掌握。去中心化的基础设施可以彻底消除平台税和不公平的平台规则。通过 Token 机制，让基础平台被开发者、传教者、所有用户和资本共同拥有，分享税收，并共同协定更公平的平台规则。
- `kill app` 的底层逻辑是“通过 LLM 解决信息筛选的匮乏”。用 AI 来构建信息是少部分人的需求，用 AI 来筛选信息则是所有人都需要的。AI 运用常识帮助用户筛选其接收的信息，解决当今社会的信息茧房问题。用户使用后效果明显，对社会有正面意义。对 AI 行业来说，通过 CYFS 链接所有用户的 KnowledgeBase 形成的语义网络，也能帮助 LLM 在实时、准确的信息基础上得到更好的结果。

### 了解更多 BuckyOS 内容

- BuckyOS 架构设计（敬请期待）
- Hello BuckyOS!（敬请期待）
- BuckyOS dApp 开发者手册（敬请期待）
- BuckyOS 贡献者指南（敬请期待）

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

- **0.6.0 Beta2：** 2.5%（2026 年 Q1，本次发布，迭代开发中）
- **0.7.0 Beta3：** 2.5%（计划于 2026 年 4 月底发布）

## 许可证

BuckyOS 是一个自由、开源、去中心化的系统，鼓励厂商基于 BuckyOS 构建商业产品，促进公平竞争。我们的许可选择旨在实现生态系统共赢、保持去中心化核心、保护贡献者利益，并构建一个可持续的生态系统。我们采用双许可证模式：一方面是基于 LGPL 的传统许可，要求内核修改需遵守 GPL（允许封闭源代码应用，但这些应用不能成为核心系统组件）；另一方面是基于 SourceDAO 的许可。当发行 DAO Token 的组织使用 BuckyOS 时，必须根据该许可证向 BuckyOS DAO 捐出一部分 Token。

目前还没有完全符合我们需求的许可证，因此在 DEMO 阶段我们暂时使用 BSD 许可证。我认为当 PoC 完成后，我们肯定会准备好正式的许可证。
