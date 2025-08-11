# BuckyOS Alpha3(0.4.1) 发布！

Alpha3 是计划中的最后一个 Alpha 版本，其主要目标是完成 Alpha2 中遗留的功能，通过建立更完善的测试体系来提升系统稳定性，并为 Beta 版本的公开测试做好准备。

Alpha3 计划实现的主要功能包括：

* 修复上一个版本中为赶进度而匆忙开发的内核组件，确保其健壮性，并完善测试用例
* 改进标准的分布式开发环境，并基于该环境构建测试用例
* 优化代码仓库结构，为 cyfs-gateway 的独立产品化做准备
* 构建完整的 Nightl-channel，并支持自动版本更新
* 改进 ndn-lib，新增对 fileobject 的 chunklist 支持
* 为 ndn-lib 增加对容器与 DirObject（Git 模式）的基本支持支持 
* 支持通过 USB4 在 Mac 上连接并访问 smba 服务
* 支持在 BuckyOS 中快速添加 Docker URL，并通过 appID.\$zoneID 方式访问应用
* backup/cyfs-gateway独立产品化的规划移至beta1
* 考虑到dfs的集成问题，暂时搁置system_config的etcd后端计划

加入我们的征程吧！欢迎随时提交 issue 或 pull request！让我们共同构建下一代分布式AI操作系统！

目前我们正处于Alpha3的版本DAO验收阶段，我们计划下周开启Beta1的研发工作。Beta1是BuckyOS是一个关键的版本，这个版本我们将按计划完成与[OpenDAN](https://github.com/fiatrete/OpenDAN-Personal-AI-OS)进行整合，在BuckyOS里提供OpenDAN所需要的关键AI能力。


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
git clone https://github.com/buckyos/buckyos.git && cd buckyos && python3 devenv.py && python3 src/build.py
```

构建脚本完成后，本地机器上的安装就完成了（为了方便，默认包含了测试身份信息）。运行以下命令启动初始状态下的 BuckyOS：

```bash
sudo /opt/buckyos/bin/node_daemon --enable_active
```

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
- dev 
- dev_no_docker

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

- **0.1 Demo：** 2.5%（已完成）
- **0.2 PoC：** 2.5%（已完成）
- **0.3 Alpha1：** 2.5%（已完成）
- **0.4 Alpha2：** 2.5%（实际发布日期2025年4月，已完成）

#### 2025

- **0.4.1 Alpha3：** 2.5%（本次发布）
- **0.5 Beta1:** 5%（2025年10月 首个公开发行版本）
- **0.6 Beta2:** 2.5%（2025 年 Q4）


## 许可证

BuckyOS 是一个自由、开源、去中心化的系统，鼓励厂商基于 BuckyOS 构建商业产品，促进公平竞争。我们的许可选择旨在实现生态系统共赢、保持去中心化核心、保护贡献者利益，并构建一个可持续的生态系统。我们采用双许可证模式：一方面是基于 LGPL 的传统许可，要求内核修改需遵守 GPL（允许封闭源代码应用，但这些应用不能成为核心系统组件）；另一方面是基于 SourceDAO 的许可。当 DAO-token 发行组织使用 BuckyOS 时，必须根据该许可证捐出部分 Token 给 BuckyOS DAO。

目前还没有完全符合我们需求的许可证，因此 DEMO 阶段我们将暂时使用 BSD 许可证。我认为当 PoC 完成后，我们肯定会准备好正式的许可证。