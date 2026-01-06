# BuckyOS Beta1(0.5.1) 发布！


Beta1 实现的主要功能包括：

* 核心修改:将网络内核迁移到新版的，基于process-chain的cyfs-gateway上。至此，BuckyOS的网络内核基本稳定
* 新增BuckyOSApp，用户使用Web3.0钱包的方式管理自己的私钥，并完善了整个激活流程
* 对Beta阶段面向的主要场景:在Windows/OSX上安装进行了系统性的优化
* 继续完善调度器，让调度器的核心逻辑更加独立，复杂调度可以理论先行完成实现
* 引入了buckyos-devkit辅助开发工具，优化了日常开发测试的流程
* 基于buckyos-devkit,完善了基于multipass的常规复杂场景的开发循环
* 完善resolve-did机制
* 基于新的ndn-lib核心对象，调整pkg-meta
* 基于新版本的pkg-meta，实现App安装协议


加入我们的征程吧！欢迎随时提交 issue 或 pull request！让我们共同构建下一代分布式AI操作系统！

随着Beta1版本的发布，我们已经进入了Beta1.2版本的研发阶段。Beta1.2版本计划在2026年2月15日发布，这是一个快速迭代版本，基本没有内核级别的修改

- 完成与buckyos backup suite的集成。实现系统的整体备份和恢复
- 内核完成对OPTask的支持（备份和恢复依赖该设施）
- 完成system control service服务，完成系统控制面板UI
- 完成App安装协议的相关UI，做好对首个大型killapp `gitpot.ai`的支持
- 集成slog,klog等已经完成的基础服务
- 完成`gitpot.ai`需要的一些AI基础设施
- 整理buckyos的所有文档

## 开始使用

首先获取活跃代码：  
[https://github.com/buckyos/buckyos/discussions/70](https://github.com/buckyos/buckyos/discussions/70)

### 无 Docker 安装方式

我们知道大家都喜欢 Docker！

然而，由于 BuckyOS 可以视为“无需 IT 支持的家庭 Kubernetes 部署”，其底层依赖容器技术，但不应当在 Docker 内部运行。为了提供类似 Docker 的体验，BuckyOS 将所有二进制文件都发布为静态链接文件，因此在 99% 的情况下，你不会遇到“依赖问题”。

### 使用 deb 安装

适用于使用 apt 和 WSL2 的 x86_64 Linux 发行版。根据你的网络速度，此过程大约需要 5-10 分钟。

运行以下命令下载并安装 buckyos.deb：

```bash
wget https://www.buckyos.ai/static/buckyos_amd64.deb && dpkg -i ./buckyos_amd64.deb
```

如果你在 ARM 设备（如树莓派）上安装，请使用 buckyos_aarch64.deb：

```bash
wget https://www.buckyos.ai/static/buckyos_aarch64.deb && dpkg -i ./buckyos_aarch64.deb
```

安装过程中将自动下载依赖项和默认应用的 Docker 镜像，因此请确保你的网络连接稳定且能访问 apt/pip/Docker 仓库。

在安装过程中，你可能会看到一些权限错误，但大多数都不是关键问题。安装完成后，打开浏览器访问：

```
http://<你的服务器IP>:3180/index.html
```

你将看到 BuckyOS 的启动设置页面，按照指示完成设置即可！在 Alpha 测试阶段，使用 `sn.buckyos.ai` 的中继和 D-DNS 服务需要邀请码（点击此处获取邀请码），你可以从我们的 issue 页面获得。（如果你拥有自己的域名并已在路由器上设置端口转发，则无需使用 `sn.buckyos.ai` 的任何服务，可直接尝试，无需邀请码）

#### 端口与访问入口速查（Beta1）

- **首次打开 Web 启动页**：默认 `http://<设备IP>:3180/index.html`
- **待激活设备发现/激活服务**：默认 `http://<设备IP>:3182/`（更多见 [`notepads/设备激活协议.md`](notepads/设备激活协议.md)）

### Windows 安装

敬请期待。

### macOS 安装

敬请期待。

### 在不支持 .deb 的 Linux 上安装

敬请期待。


## 在虚拟机上安装

我们正在准备相关镜像，以支持在 Windows、macOS 以及没有 WSL 环境的主流 NAS 品牌上运行 BuckyOS。我们承诺在 Alpha2 发布之前完成这项工作。

## 从源码安装

从源码安装是了解 BuckyOS 的好方法，也是迈向贡献的第一步。通过从源码安装，你还可以在 macOS 上安装 BuckyOS。


```bash
git clone https://github.com/buckyos/buckyos.git
```

clone 完成后，需要安装 `buckyos-devkit`（它会提供 `buckyos-build` / `buckyos-install` 等命令）。建议在项目目录创建并激活 venv 后再安装：

```bash
cd buckyos
python3 -m venv venv
source venv/bin/activate
python3 -m pip install -U "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"
```

开始构建：构建前可以参考 `devenv.py` 搭建环境。我们主要依赖 `Rust 工具链、Node.js + pnpm、Python3.12、docker.io` 工具链。

```
cd buckyos
buckyos-build
```


构建完成后，通过下面命令完成开发机安装：

- **首次安装/全量重装（推荐）**：会执行 clean + install_app_data + 更新 modules 构建产物
- **日常增量覆盖**：只覆盖更新 modules 构建产物

```bash
# 首次安装 / 全量重装（推荐）
buckyos-install --all

# 日常增量（不带 --all）
# buckyos-install
```

然后生成配置组（下面的 `release/dev/...` 是“配置组名称”，用于生成一套可运行的系统配置）：

```bash
python3 make_config release
```

目前cyfs-gateway是独立构造的，因此在运行前先需要从源码构造cyfs-gateway
```bash
git clone https://github.com/buckyos/cyfs-gateway.git
cd cyfs-gateway
buckyos-build
buckyos-install --all
```

#### 过渡期 Dev 一条龙（从 clone 到生成 sn 测试组配置）

如果你是新环境从零开始，且需要构建安装 **buckyos + cyfs-gateway** 并生成 **sn** 测试组配置（含 3 个 OOD 身份），可以直接按下面顺序执行（这份顺序来自社区整理的 issue 说明，强烈建议照做）：

- 参考：[`buckyos/issues/321`](https://github.com/buckyos/buckyos/issues/321)

```bash
# 0) 准备工作目录
mkdir -p ~/work && cd ~/work

# 1) clone repos
git clone https://github.com/buckyos/buckyos.git
git clone https://github.com/buckyos/cyfs-gateway.git

# 2) 安装 buckyos-devkit（提供 buckyos-build / buckyos-install 命令）
cd ~/work/buckyos
python3 -m venv venv
source venv/bin/activate
python3 -m pip install -U "buckyos-devkit @ git+https://github.com/buckyos/buckyos-devkit.git"

# 3) （过渡期常见坑）更新 Rust 依赖锁
cargo update

# 4) 在 buckyos repo 构建 + 全量安装
buckyos-build
buckyos-install --all

# 5) 在 cyfs-gateway repo 构建 + 全量安装
cd ~/work/cyfs-gateway
cargo update
buckyos-build
buckyos-install --all

# 6) 回到 buckyos repo：生成 sn 所需的 3 个 OOD 身份 + 生成 sn 配置
cd ~/work/buckyos
python3 make_config.py alice.ood1
python3 make_config.py bob.ood1
python3 make_config.py charlie.ood1
python3 make_config.py sn
```

补充说明（非常重要）：

- `buckyos-build` / `buckyos-install` **不是**仓库自带脚本，它们来自 Python 包 `buckyos-devkit`。
- `--app=<name>` 是可选参数：
  - `buckyos-build --app=<name>`：只构建某个 app
  - **不传 `--app`**：默认会对 `bucky_project.yaml` 中定义的**所有 apps**执行 build/install
- `--all` 语义：
  - `buckyos-install --all`：全量重装（clean → install_app_data → 更新 modules 构建结果）
  - `buckyos-install`（不带 `--all`）：增量覆盖（只复制 modules 构建结果）

#### 常见坑点与排查（过渡期）

- **经常需要 `cargo update`**：尤其是新环境或依赖锁漂移时。
- **`buckyos-devkit` 更新快**：过渡期通常需要从 git 安装最新版（见上面的 pip 命令）。
- **`make_config.py` 依赖 `buckycli`**：通常需要先在 buckyos repo 执行 `buckyos-build && buckyos-install` 确保 `buckycli` 就绪。
- **测试 CA 根证书复用是隐式行为**：方便浏览器安装，但清理/切换环境时容易困惑（后续我们会在工具链里补“reset 指令”）。

启动！使用下面命令即可进入和安装包一样的激活流程。因为是开发启动，所以激活成功后不会自动重启，还需要再执行一次该命令让buckyos以激活模式运行
```bash
sudo /opt/buckyos/bin/node-daemon/node_daemon --enable_active
```

说明：不同发布/平台的二进制目录可能是 `node-daemon/node_daemon` 或 `node_daemon/node_daemon`；如果一个路径不存在，换另一个即可。

### 源码目录的常用脚本

- 下面脚本只进行rust部分的构建
```bash
cd src
python3 build.py --no-build-web-apps
```

- 下面脚本用只更新编译的二进制文件后启动/opt/buckyos
```bash
cd src
python3 start.py
```

- 下面脚本基于指定的配置组重装buckyos
```bash
cd src
python3 start.py -reinstall $group_name
```
如果group_name为空，则用空配置文件启动buckyos,此时进入待激活状态。
目前系统带有两组配置文件
- release （在正式环境中运行，使用buckyos.ai的sn设施）
- dev (无sn的开发测试配置，不依赖任何本机外的组件)
- alice.ood1,bob.ood1,charlie.ood1,3个预设身份，均使用计划部署到虚拟测试环境的devtests.org环境
- sn 虚拟测试环境中的sn

#### App 安装协议与 UI 设计（文档入口）

- App 安装协议（第三方网页如何触发安装、分享安装等）：[`notepads/app安装协议.md`](notepads/app安装协议.md)
- 安装流程 UI 草案（安装确认/高级配置/进度/失败/分享/信任机制等）：[`notepads/app安装UI.md`](notepads/app安装UI.md)

#### SN 虚拟机测试环境（sntest）

如果你要跑 sn 虚拟机测试环境（`sntest`），并在 `buckyos (ood) + cyfs-gateway (sn)` 两个工程间迭代构建/更新，可参考：[`notepads/sntest环境使用.md`](notepads/sntest环境使用.md)


老的 `python3 start.py --all`脚本现在等价于  `python3 start.py --reinstall dev`


## BuckyOS 的愿景

- Internet is BuckyOS,  通过新的去中心（必然是开源的）基础设施，构建新的dApp生态，app之间的信息更加互联，模块化更好，对AI更有好。能支持构建比现在复杂一数量级的应用，其构建和运行成本也会下降一个数量级。（生产力提升100倍）
- 互联网的基础设施不可以被公司掌握。去中心的基础设施可以彻底消除平台税和不公平的平台规则。通过Token的方式让基础平台被开发者、传教者、所有用户、资本共同拥有。分享税收，共同协定更公平的平台规则
- kill app的底层逻辑是“通过LLM解决信息筛选的匮乏”，用AI来构建信息是少部分人的需求，用AI来筛选信息则是所有人都需要的。AI运用常识帮助用户筛选其接收的信息。解决当今社会的信息茧房的问题。用户使用后效果明显， 对社会有正面意义。对AI行业来说，通过cyfs链接所有用户的KnowledgeBase形成的 语义网络，也能帮助LLM在实时、准确的信息基础上得到更好的结果。

### 了解更多 BuckyOS 内容

- BuckyOS 架构设计（敬请期待）
- Hello BuckyOS!（敬请期待）
- BuckyOS dApp 开发者手册（敬请期待）
- BuckyOS 贡献者指南(敬请期待)


## 下一代 GPL：创建全新开源协作模型

“开源组织有着悠久的历史和辉煌的成就。实践证明，仅在虚拟世界中协作，就能编写出更好的代码。我们相信软件开发工作非常适合采用 DAO 模式。我们称这种由去中心化组织共同开发软件的 DAO 为 SourceDAO。” —— 引自 CodeDAO 白皮书 (https://www.codedao.ai)

BuckyOS的开源社区通过DAO的方式来运作，我们的目标是解决开源付出没有回报，甚至白嫖的问题：

- 编码挖矿，通过利益相关提高版本发布质量
- 通过类似GPL的传染机制，形成上下游的共同利益结构
- 通过智能合约的自动分账，让支持世界稳定运行的基础库的贡献者能得到稳定且长久的收入（这是他们应得的）
治理上，通过统一持币，一致的利益关系来统一用户和开发者的共识，在共同的利益共识下做出理智的决定。（吵架也是和自己人吵架）

`公开，透明，来去自由（人人可参与），结果导向`

SourceDAO 是基于以上理念的我们的开源 DAO 智能合约。更多详情请访问 [https://dao.buckyos.org/](https://dao.buckyos.org/)。

## 初步版本计划

#### 2024

- **0.1 Demo：** 2.5%（2024年6月 已完成）
- **0.2 PoC：** 2.5%（2024年9月 已完成）
- **0.3 Alpha1：** 2.5%（2024年12月 已完成）

#### 2025
- **0.4 Alpha2：** 2.5%（2025年3月，已完成）
- **0.4.1 Alpha3：** 2.5%（2025年9月 已完成）
- **0.5.1 Beta1:** 4%（2025年12月 本次发布 首个公开发行版本）

#### 2026

- **0.5.2 Beta1.2:** 1%（2026 年 Q1）
- **0.6 Beta2** 2.5% （2026年Q2）


## 许可证

BuckyOS 是一个自由、开源、去中心化的系统，鼓励厂商基于 BuckyOS 构建商业产品，促进公平竞争。我们的许可选择旨在实现生态系统共赢、保持去中心化核心、保护贡献者利益，并构建一个可持续的生态系统。我们采用双许可证模式：一方面是基于 LGPL 的传统许可，要求内核修改需遵守 GPL（允许封闭源代码应用，但这些应用不能成为核心系统组件）；另一方面是基于 SourceDAO 的许可。当 DAO-token 发行组织使用 BuckyOS 时，必须根据该许可证捐出部分 Token 给 BuckyOS DAO。

目前还没有完全符合我们需求的许可证，因此 DEMO 阶段我们将暂时使用 BSD 许可证。我认为当 PoC 完成后，我们肯定会准备好正式的许可证。