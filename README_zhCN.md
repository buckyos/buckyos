# BuckyOS 启动！


## Why BuckyOS?

今天运行在Cloud(Server)上的Service与我们的生活关系密切，人们已经很难再日常生活中离开Service了。然而，确没有专门为运行Service设计的操作系统 。

已经有CloudNative了？ CloudNatvie是为商业公司或大型组织设计的，专门运行Service的操作系统，这种系统普通人很难安装和使用。从历史的角度来看，传统的SystemV操作系统(Unix)一开始运行在IBM的小型机上，离普通人也很遥远，但在iOS以后，普通人就可以很轻松的使用现代操作系统：管理好自己的应用软件和数据并长期稳定运行，而不用懂很多高深的技术。iOS开创了人人都可以使用个人软件的新时代。

今天，Service对所有的人都很重要，人们应拥有安装和使用Service的自由（我们把这种Service称作Personal Service）。不依赖商业公司可以独立运行的Personal Service又被称作dApp，整个Web3工业已经有非常多人投身于此，基于区块链和智能合约技术做了大量的工作。我们认为实现这个目标最简单直接的方法是人们可以购买消费级的Server，该Server已经安装好了OS, 随后人们可以在这个OS上简单的安装应用：该应用同时包含Client和Service，相关数据也只会保存在用户拥有的Server上。该OS的操作简单易懂，还要保障Service的高可靠和高可用。当发生故障时，通常只需要替换损坏的硬件就可以让系统恢复工作，而不需要依赖专业的IT Support。

BuckyOS就是为了解决这些问题而诞生的，将成为CloudOS领域的"iOS", 开创人人都拥有自己的Personal Service的互联网新时代。

## BuckyOS的目标

Buckyos是面向终端用户的Open Source Cloud OS (Network OS).其首要设计目标是，然让消费者能拥有一个自己的集群/云（为了和常规的术语进行区分，我们把这个集群称作Personal Zone，简称Zone），消费者自己家里的所有设备，以及设备上的计算资源都接入这个集群。消费者可以像安装App一样在自己的Zone内安装Service. 基于buckyos,用户可以拥有可以访问自己的所有数据、设备、和服务。未来在Zone内如果有了足够的算力，也可拥有Local LLM，并在此之上拥有为真正的AI Agent。

BuckyOS有下面几个关键的设计目标：

`开箱即用：`普通用户购买了搭载了BuckyOS的商用PersonalServer产品后，可以非常简单的完成初始设置。其基本流程是：安装BuckyOS Control App -> 创建身份（本质上是类似BTC钱包地址的去中心身份）-> 设置Zone ID (域名) -> 将设备插上电源和网线并打开 -> 在App中发行待激活的设备 -> 激活该设备并添加到自己的Zone中 -> 应用默认设置 -> 启动默认Service(至少有一个网络文件系统）。

`Service Store:`  管理运行在BuckyOS上的Service,和管理iOS上的App一样简单。通过Service Store还要构建健康的dApp生态：实现开发者、用户、设备制造商的三赢。BuckyOS有完整的Service权限控制体系，Service运行在特指的容器里，完全可控，确保用户的数据隐私安全。

`高可靠：`数据是数字时代人们最重要的资产，不丢数据是今天用户选择使用Service而不是Software的重要原因。BuckyOS必须给未来的商用Personal Server 产品设计合理的硬件架构，以确保在任意一块硬盘损坏（这是不可避免）的情况下，都不会丢失任何数据。同时BuckyOS也要构造开放的Backup System, 根据数据的重要度，选择不同的备份任务，并备份到不同的备份源上。备份必然是有成本的，开放的Backup System保障了用户选择的权利。当系统的处于完全备份状态时，即使系统的硬件全部损毁，也能在新的硬件上完全恢复。

`Access Any Where:` 用户的Personal Server通常部署在自己家里，局域网的访问肯定是高速而稳定的：比如家庭安保摄像头把重要的视频保存在Personal Server提供的海量存储空间中，就肯定比今天保存在Cloud上更快速和稳定。但更多的时候，我们希望运行在手机上的App Client能随时连上Personal Server.BuckyOS系统设计的一个重要目标，就是让所有的Service都可以透明的得到这个特性。我们主要是通过3种方法来实现该目标:

1. 更好的集成IPv6
2. 集成P2P协议，尽可能的实现NAT穿透
3. 鼓励用户在公网部署Gateway Node。通过流量转发实现Acess Any Where.

`Zero Operation：`随着时间的流逝，任何系统都可能损坏，或则需要根据实际情况调整。BuckyOS通过定义一些简单的流程，帮助没有专业运维能力的普通消费者能自主完成必要的运维操作，保障系统的可用性，可靠性、可扩展性。

1. 硬件损坏，但未导致故障：购入新设备，用相同的硬件名激活新设备来替换旧设备，等待新设备的状态变成正常后，拔掉有故障的旧设备。
2. 硬件损坏，且导致故障：如果着急，可以立刻启用云端虚拟设备，并从备份中恢复，让系统进入可用状态。随后购买新设备，用相同的硬件名激活新设备来替换旧设备，等待新设备的状态变成正常。
3. 存储空间不够：购买新设备，激活后系统可用空间增加。

`Zero Depend:` BuckyOS的运行不依赖任何商业公司或任何中心化的基础设施。站在用户的角度，就是不用担心其购买的，搭载了BuckyOS的Personal Server的厂商倒闭后会有功能不正常的问题。BuckyOS的标准开源发行版里使用去中心的基础设施，比如集成了去中心存储（ERC7585）以实现去中心化的备份。BuckyOS的商业发行版中可以集成一些付费的订阅服务（比如传统的备份订阅服务），但这些服务都必须给用户选择另一个供应商的权利。


`升级到高可用:`  考虑到Personal Server的家庭属性，BuckyOS通常安装在由3-5台Server组成的小型集群上。在这么小规模的集群里，我们的trade-off是尽量保证高可靠，而不去保证高可用。这意味着我们允许系统在一些时候进入只读或不可用状态，只要用户进行了一些简单的运维操作后系统能恢复可用即可。但当BuckyOS被安装在几十台甚至上百台服务器组成的中型集群时，这个集群通常是中小型企业拥有，可以在有简单的IT Suppor的支持下，配置成高可用状态：在预期的硬件故障发生时，系统依旧保持完整的可用。

## 系统架构设计

我已经对BuckyOS的整体架构进行了设计，我想是可以实现上述目标的。BuckyOS的完整系统架构肯定需要深度的讨论与反复的迭代，我们会有一篇持续更新的文档来专门讨论他。在这里，我想站在原点，尽可能宏观的来讲述整个架构。让第一次接触BuckyOS的工程师可以在最短的时间里对BuckyOS的关键设计有一个比较粗略的理解。随着BuckyOS的迭代，系统的具体的设计会不断调整，但我有自信，本文提到的一些最基础的设计原则和对主要流程的框架性思考会有很长的生命力。

在本文完成时，BuckyOS已经完成了demo版本(0.1),因此很多设计也得到了一定程度的验证。不少设计已经可以通过DEMO简陋的代码观察一二。

### 一些基本概念

![BuckyOS的典型拓扑](./doc/pic/buckyos-Desc.svg)

参考上面这个典型的拓扑图，理解BuckyOS中的一些最重要的基本概念：   
- 一组物理服务器组成了集群，在这个集群上安装了BuckyOS，这个集群变成了一个Zone。
- Zone由逻辑Node组成。比如这个集群里的Server上运行了6个虚拟机,分别组成了互相隔离的ZoneA和ZoneB，一个正式的Zone，至少由3个Node组成。

`Zone:  安装了BuckyOS的集群被称作一个Zone`,集群的设备在物理上通常属于同一个组织。按BuckyOS的假定场景，这个组织通常是家庭和小企业，因此Zone里的设备通常都大多接入同一个局域网。BuckyOS本身支持多用户，一个Zone可以为多个用户服务，但同一个逻辑用户（用DID标识）只能存在于一个Zone中。

每个Zone都有一个全网唯一的ZoneId. ZoneId是一个人类友好的名字，首选是一个域名，比如buckyos.org 可以看成一个ZoneId. BuckyOS也会原生的支持Blockchain Base的名字系统，比如ENS.任何人通过ZoneId可以查询得到当前的公钥、配置文件(`ZoneConfig`)、和配置文件的签名。拥有Zone当前公钥对应的私钥即拥有了Zone的最高权限。

`Node: 组成Zone的Server被称作Node。`Node可以是物理Server，也可以一个虚拟Server。同一个Zone内的任意两个Node不能运行在同一个物理Server上。

每个Node有一个Zone内唯一的NodeId. NodeId是一个人类友好的可读名字，可以用$node_id.zone_id的方式准确的指向一个Node。在已经正常运行的Zone内可以通过NodeId查询到Node的公钥、配置文件(`NodeConfig`)、和配置文件的签名。Node的私钥通常保存在Node的私有存储区并定期更换，运行在Node上的BuckyOS内核服务使用该私钥来周期性的声明身份并得到正确的权限。



### 系统的启动
下面是BuckyOS的从系统启动到应用服务启动的关键流程介绍：

1. 每个Node都独立的启动node_daemon进程。下面的流程同时发生在Zone内所有的Node上。
2. node_daemon进程在启动的时候会根据node_identity配置，知道自己所在的zone_id和自己的node_id. 无法读取到这个配置的node说明没有加入任何Zone。
3. 通过nameservice组件，基于zone_id查询zone_config。
4. 根据zone_config准备etcd. etcd是BuckyOS系统中最重要的基础组件，为系统提供了可靠的一致性结构化存储能力。
5. etcd服务初始化成功意味着BuckyOS引导成功。随后node_daemon会通过读取保存在etcd上的node_config进一步启动内核服务和应用服务。
6. 应用服务的进程都是state less的，因此可以运行在任意Node上。应用服务通过访问内核服务(DFS,system_config)来管理状态。
7. BuckyOS通过cyfs-gateway内核服务，向Zone外暴露特定的应用服务和系统服务，
8. 当系统发生了改变后，buckyOS的调度器会工作，修改特定node的node_config，让这个改动生效
9. Node_config的改变会导致3类事情的发生：
    - a. 内核服务进程在某个Node上启动或停止
    - b.应用服务进程在某个Node上启动或停止
    - c.在某个Node上执行/取消一个特定的运维任务（比如数据迁移）
10. 增加新的设备、安装新的应用、系统发生故障、系统设置修改都可能会引起系统的改变。
11. 系统改变后BuckyOS调度器会开始工作，调度器会重新分配哪些进程在哪些Node上运行，哪些数据保存在哪些Node上。
12. 调度对99%的应用来说都是透明的，调度器能让系统更充分的发挥硬件的能力，提高系统的可靠性、性能和可用性。 

从实现的角度来说，运行中的BuckyOS是一个分布式系统，肯定由一系列运行在Node上进程组成的。通过理解这些最重要的进程的核心逻辑，可以进一步的理解BuckyOS:

![BuckyOS的MainLoop](./doc/pic/buckyos-MainLoop.svg)


`“任何操作系统本质上都是循环”`,上图展示了BuckyOS里最重要的两个循环。 


Node Loop:最重要的内核模块"node_daemon"的主要逻辑。该循环的核心目的是根据node_config来管理运行在当前node上的进程和运维任务。该流程图还展示了node_daemon的引导启动etcd流程。

scheduling loop: 传统操作系统最重要的循环，处理重要的系统事件，更新node_config后通过NodeLoop来实现目的。限于篇幅这里没法举具体的例子，但经过我们的推演，上面两个循环配合可以简单可靠的实现BuckyOS的关键系统设计目标。 

上述双循环设计还有如下优势：
1. 低频调度：只有有事件发生的时候才需要启动调度器，减少系统资源的消耗。
2. 调度器可崩溃：因为更新NodeConfig是一个原子操作，所以调度器在运行过程可以随时崩溃。而且系统里只需要有一个调度器进程，不需要做复杂的分布式设计。即使系统发生脑裂了，NodeLoop也会忠实的安装上一次系统的状态持续的工作下去。
3. 调度逻辑可扩展,大规模系统的特定调度可人工参与：Node Loop根本不关心node_config是怎么生成的。对大规模的复杂系统难以编写自动化的调度逻辑，可由专业人士来处理调度，让调度器变成一个纯粹的node_config构建辅助工具。
4. 简单可靠: NodeLoop在运行过程中，不涉及到任何网络通信操作，是完全独立的。这个结构没有对BuckyOS的分布式状态一致性有任何额外的假设。

理解了上述流程后，我们再来整体的看一下BuckyOS的架构分层和关键组件：

### 架构分层和关键组件



![BuckyOS的系统架构图](./doc/pic/buckyos-Architecture.svg)



BuckyOS的系统架构一共有三层：
```
1. User Apps:

App-services that are managed by users on a daily basis and run in user-mode. Can be developed in any language
BuckyOS user-mode isolation is guaranteed by App-Container
App-services cannot depend on each other.
    
2. System Frame（Kernel） Services：

Frame-Service is a kernel-mode service of the system, which exposes the system functions to the App-Services through the kernel RPC.
It can be extended under the operation of the system administrator. The extension logic is similar to installing new kmods in Linux.
Frame-service also runs in a container most of the time, but this container is definitely not a virtual machine.
    
3. Kernel Models:

The Kernel Models is not extensible. The purpose of this layer is to prepare the environment for the Kernel Service. 
As the most basic component of the system, it will enter a stable state as soon as possible, and modifications at this layer should be minimized. 
Some basic components of this layer can load System Plugin to expand functions, such as pkg_loader can support new pkg download protocols through the pre-defined SystemPlugin.

```
这里我们从底层向上，简单的对各个组件做一下讲解。

`etcd:` 成熟的分布式kv存储服务，运行在2n+1个节点上，实现了BuckyOS系统核心状态的分布式一致性。是BuckyOS最重要的基础服务，系统里所有重要的结构化数据都保存在etcd里。保存在etcd里的数据高度敏感，因此我们设计了可靠的安全设施来保证只有得到了授权的组件才有机会访问etcd。

`backup_client:`是BuckyOS Backup Service的重要部分。在node boot阶段，会根据zone_config里的配置，尝试从Zone外的backup server上恢复etcd的数据。Backup client提供的etcd data restore能力，实现了BuckyOS的异地数据可靠性。

`name_service_client:`该组件是BuckyOS Name Service的重要部分，可以根据给定的NAME(zoneid,nodeid,DID 等等）解析对应的信息。在etcd 未启动前，name_service_client的解析主要是查询互联网的公共基础设施：域名系统和ENS（可通过BuckyOS的System Plugin机制扩展新的后端）。在etcd启动后，name_service_client还会基于etcd上的配置信息进行解析。Name_service_client的查询接口兼容传统的DNS协议。
name_service_client的目标是成为一个独立的，去中心的开源DNS Server。

`node_daemon:`BuckyOS中重要的基础服务，运行NodeLoop。Zone内的Server要成为一个正常的Node，就需要正常运行 node_daemon. 其核心流程前文已经讲述，这里就不多写了。

`machine_active_daemon:`该服务严格意义上不是BuckyOS的一部分，属于BuckyOS的BIOS。该服务用来支持server激活成为node的过程（获得node identity配置），我们鼓励硬件厂商根据自己的产品特定设计更友好的设备激活服务来取代machine_active_daemon. 

`cyfs-gateway:`BuckyOS中的重要基础服务，实现了BuckyOS内的SDN逻辑。其功能相对比较复杂，我们会有一篇专门的文章来详细的介绍cyfs-gateway。cyfs-gateway的长期目标是成为一个独立的，开源的web3 gateway,通过可扩展的框架满足 Http Gateway / VPN / VNet / Parent Control等一系列网络管理的需求。

`system_config:` 基础的内核组件(lib), 对etcd的访问进行了一些更加语义化的封装。系统里的所有组件都应该通过system_config来使用etcd。

`acl_config:` 基础的内核组件，对基于system_config的ACL权限管理逻辑进行了进一步的封装。

`kLog:`BuckyOS中重要的基础服务，为所有的组件提供了可靠的日志记录功能。kLog除了在开发阶段帮助开发者定位复杂的问题，在生产环境中，BuckyOS通过kLog来实现系统里最重要的故障发现、性能重平衡等功能。
从原理上，kLog也基于raft协议，但与etcd的写少读多不同的是,kLog写多读少。我们希望不远的未来能合并kLog和etcd, 让系统更简单可靠。

`pkg_loader:`基础的内核组件，其核心功能有两个：根据pkg_id加载pkg,根据pkg_id从repo-server上下载pkg.  

Pkg可以理解为一个软件包(类似apk), 其pkg_id除了友好名称外，还可以包含版本、Hash值等丰富信息。在现代的开发过程中,包管理都是一个重要的基础组件，相信所有的工程师都在apt/pip/npm/cargo 等工具中体会到了其便利。BuckyOS包含了完整的pkg 基础设施，来支持系统里不同类型组件的下载、安装、升级、加载、卸载，并提供了足够的可扩展设计。我们会有一篇文章详细的说明BuckyOS中整个pkg system的设计。

`dfs:`文件系统一直以来都是操作系统最重要的基础设施，BuckyOS也不例外。dfs是BuckyOS中最重要的Frame-Service，为其上的所有组件提供了可靠的数据管理服务。这是一个非常复杂的基础服务，其可靠性、稳定性和性能对BuckyOS的最终体验影响都很大。BuckyOS支持在安装的时候选择不同的分布式文件系统（一旦选定无法实时切换），我们先选择使用GlusterFS作为DFS的后端实现。现阶段dfs的主要任务是根据BuckyOS的需要，对dfs的接口进行仔细的设计，保障切换DFS的实现后端对上层是透明的。

从我过去经验来看，我们非常有必要为新的硬件和相对小规模的集群定制一个专门设计的分布式文件系统，我们把这个文件系统命名为"dcfs" （DeCentralized FileSystem）, 该系统完成测试后会成为DFS的默认后端. dcfs是一个独立的项目，我们会在那里专门讨论他的设计。

`rdb:` 关系数据库是传统服务的重要基础服务。我们有可能未来会在BuckyOS中提供一个标准的RDB给所有的应用开发者。现在有了DFS，已经可以帮助AppService把自己用的sqlite/mysql/redis/mongoDB移植到 BuckyOS上来了。

`kRPC：`Kernel-Switch RPC的缩写。提供了一套基础设施，来可靠的鉴定frame-service调用者的身份，并支持frame-service的开发者根据该身份信息查询ACL查询，实现对BuckyOS核心资源的访问权限控制和隔离。该组件还提供了一些开发辅助工具，帮助frame-service的开发者用通用的方法暴露自己实现的功能(类似gRPC).

`control_panel:`基于kRPC向App-Service暴露了SytemConfig的读写接口，还包含了默认的BuckyOS系统管理WebUI。该服务是一个产品向的功能密集型服务，迭代速度较快。Control_panel修改完系统配置后，会等待scheduler发现这些修改并使之生效。

`Scheduler:` 这个服务可以称作是BuckyOS的大脑，如前文所述，该组件实现了scheduling loop。是BuckyOS作为一个分布式操作系统一系列高级能力的来源。在其它组件保持简单可靠的情况下，Scheduler是系统里唯一一个允许有“查询分布式系统的状态->进行分析->做出决定”逻辑的服务。


BuckyOS里还会预装一些extension frame-service ,用来提供一些必要的产品功能，因为不涉及到系统的核心设计，这里就不详细介绍了。

### 应用的安装和运行

我们最后再以应用为中心，介绍一些关键的流程：

![BuckyOS的App Install流程](./doc/pic/buckyos-App%20Install.svg)

上述流程说明了应用的安装和启动流程。值得注意的点是对Zero Dpendency & Zero Trust原则的实现。

1. 系统在安装pkg的时候，肯定需要外部服务的支持才能下载pkg.这并不会让系统去彻底的依赖一个外部服务，在系统默认的外部pkg repo server停止工作的情况下，用户可以通过配置另一个pkg repo server让系统继续工作。
2. 由于pkg_id本身可以包含hash信息，因此我们可以实现Zero Trust：从任何源下载的pkg都是可验证的。
3. 当首次安装完成后，系统会在内部的repo-server里保存已安装的app-pkg,这些数据作为系统数据的一部分也会被备份。系统运行过程中,node 需要下载app pkg到本地，这个过程只依赖zone内的repo server,没有外部依赖。

下面进入到应用的启动流程。

![BuckyOS的App Start流程](./doc/pic/buckyos-App%20Container.svg)

上图描述了应用容器的启动流程，并说明了kRPC服务的权限控制是如何实现的。

BuckyOS希望尽量降低AppService的开发门槛：
a. 兼容容器化的应用，只需对app的权限进行配置就可以运行在buckyos上。通过该方法任何现存服务只要能容器化就能移植到buckyos上运行。    
b. 配置触发器（兼容fast-cgi），通过per request per process,可以进一步降低应用开发的难度，减少app service占用的资源。   
c. 基于buckyos app sdk开发（改造）的应用，使用BuckyOS SDK可以访问所有frame service提供的功能。    
    
## 一些编码原则

BuckyOS选择使用Rust作为主要的开发语言。得益于Rust对过去系统编程领域的总结，我们不用再去花时间介绍传统系统研发在资源(内存)管理、多线程处理、异常处理方面的一些基本原则。但BuckyOS本质上是一个分布式系统，有更大的本质复杂度。我尽可能的提炼了一些分布式系统的开发原则，希望大家能够理解并写出更高质量的代码。不符合下面原则的代码是不会被合入的~


### 简单可靠

组件的功能边界清楚，依赖关系简单，减少心智负担。
能不在内核中实现的功能就不在内核中实现，能不在系统服务中实现的功能就不在系统服务中实现。
基础组件要尽量作为独立产品设计，而不是构建一个巨大的互相依赖的系统。这会让我们无法看到真正的组件的边界。

编写显然可靠的代码：分布式系统的复杂性几乎无法依靠海量的测试实现可靠性。对于一些高可靠性要求的基础组件，应该致力减少其代码，通过让代码人人都能读懂实现显然的可靠。模块的实现应尽力减少review代码时的心智负担，谨慎的添加我们专有的概念。谨慎的选择一样简单可靠的第三方库，并保持review。为了实现该特性模块划分时可以为简单性放弃一些可复用性。对分布式系统来说，DRY永远没有KISS总要。

### 警惕对全局状态的潜在依赖

基于当前集群的状态进行判断并做出决定是一种下意识的流程设计，但这通常并不是一个好主意：分布式系统里的全局状态，是难以完整且实时的得到的，而基于该状态下发的指令，也很难做到完整的，实时的执行。

### Let it crash

在分布式系统中不要害怕Crash,当出现异常（预期外的任何情况）时，留下日志，然后crash(退出)。
不做任何重试，也不要尝试“拉起”任何自己的依赖项。系统里只有极少数组件有资格重试或启动另一个进程，这些逻辑都应该被反复的Review。

### log first

日志系统是分布式系统中的重要基础设施，不但在开发阶段为开发者诊断BUG服务，更是在生产环境里进行故障判断和性能分析的基础设施。
理解日志规范(尤其是info级别以上)，并仔细设计日志的输出，是所有开发者的首要任务。

### 小心，不要放大请求

在分布式系统中，我们为了响应一个请求，通常需要发起更多的请求。这种放大是合理的，但需要注意是可控。比如为了处理一个文件下载请求，我们要发起3个DFS系统的请求。要千万小心不确定性的放大：比如向系统里的所有符合条件的节点发起文件下载请求，或则无终止的重试。

直觉性的“查询-控制”操作，在分布式系统繁忙（也是资源最紧张的的时候）也会带来潜在的放大问题。因为查询本质上也是一个请求，也会消耗系统的资源。

深刻理解拥塞控制，对系统的吞吐能力有预期，及时发行性能瓶颈并主动的进行拥塞控制。


### Zero Dpendency & Zero Trust

减少对Zone外设施的依赖，让一定需要访问Zone的Server时，要反复思考其必要性，减少频率，设计可替代设施。   
不要信任任务Zone外的Server，换句话说，不要急于返回的来源建立信任，而是善于使用公私钥体系对返回的内容进行验证。思考内容的作者是谁，而不是内容的来源是谁，并在此技术上建立验证体系。

### 对处理链有完整的理解，减少隐式的中间层 （没有潜规则）

有一句老话叫“没有什么设计问题不能通过增加一个中间层来解决”，但这个架构思路并不适合分布式系统。修改一个复杂系统时，由于担心对现有系统产生影响而增加了一个新的中间层，这会让分布式系统走进复杂度的绝望。如果系统能保持简单，那么就总是可以站在完整的流程上找到解决问题的最佳位置。

为了解决一个眼下的问题，增加Cache，增加Queue都是治标不治本的。一个处理链路上，只能有一个Cache。

### 对数据落盘充满敬畏

数据是用户最宝贵的支持，当决定写入一个需要持久化的数据时，请区分该数据是结构化数据还是非结构化数据，深刻的思考是否应该通过一个成熟的底层服务来完成状态保存的工作。如果一定需要自己直接操作磁盘，那么这部分代码必须有最完整的设计和最完整的测试进行支持。


### 靠底层组件完成ACID保证，不要自己做分布式事务

分布式事务是分布式系统中最难的问题之一，我们鼓励开发者尽量避免使用分布式事务。如果一定要使用，那么请使用已经成熟的分布式事务服务，千万不要自己做：99.99% 你不能正确的实现。

### 根据网络请求的范式选择合适的套路

BuckyOS使用的cyfs-gateway对网络协议的套路(tcp/udp/http)进行了扩展，增加了对NamedObject(NDN)语义的支持。正确的理解网络请求的范式，可以让系统更加高效的处理网络请求，减少系统的资源消耗。






## 使用SourceDAO进行开源协作

"Open source organizations have a long history and brilliant achievements. Practice has proved that an open source organization can achieve the goal of writing better code only by working in the virtual world. We believe that software development work is very suitable for DAO. We call this DAO for decentralized organizations to jointly develop software as SourceDAO." ---- from the White Paper of CodeDAO(https://www.codedao.ai)

SourceDAO offers a complete design for the DAO-ification of an open source project. After several iterations, the CYFS Core Dev Team has essentially completed the corresponding implementation of smart contracts. Here I use OpenDAN as an example for a brief introduction, the detailed design can refer to the white paper above. Due to my background, I have a rather fundamentalist attitude towards open source (I highly agree with GPL and GNU), and the starting point of SourceDAO also comes from Bitcoin's assumption of "man is evil if not restrained". Some designs may be considered extreme, but I believe you will understand.

Basic Operation Process

1. Create an organization, design goals and initial DANDT distribution, set initial members, and establish the initial Roadmap.
2. The Roadmap explains the relationship between system maturity and token release: the more mature the system, the more tokens are released. From the perspective of software engineering, the Roadmap outlines the rough plan of the project, dividing it into five stages: PoC, MVP, Alpha, Beta, Formula (Product Release), each of which has a DANDT release plan.
3. Development as mining: This is the main stage for the DAO organization to achieve its goals. The community must work together to advance the Roadmap to the next stage. The DAO sets plans according to the standard project management process and regularly calculates the contribution value of project participants. After project acceptance, contributors will receive DANDT according to their contribution ratio.
4. DANDT can also be used for market behavior to increase the popularity of the project. The main incentive principles are to incentivize new users (like engineers who Star us on Github) or to design fission rewards for those who bring new engineers and new users to the project.
5. Holding DANDT allows participation in DAO governance.
6. Financing can be carried out based on DANDT to obtain other types of resources for the DAO.



目前BuckyOS DAO的合约计划部署在 Ploygen上。总量为21亿，简称为`BDT` (Buckyos Dao Token). 为可增发Token。

根据SourceDAO的设计要求，首次部署的时候还需要设计初始版本计划，以让所有人了解BDT的基本释放速度，还需要建立第一个委员会（至少有3个人,我想我们可以从DEMO的贡献者总中选出）。任何智能合约的部署都是一个严肃的事情，我想我会专门开一个issue列出所有必要的细节。这里只是列出一个大概的版本计划：


基本思路：基本按一个季度一个版本来，每年有一个主版本 5% Token
```
---- 2024年 ----
0.1 Demo            2.5% (Done)
0.2 PoC             2.5% 
0.3 Pre-Alpha       5% （首个完整版本）
0.4 Alpha           2.5% (2024Q4）

---- 2025年 ----
0.5 Beta            2.5%
0.6 RC              5% （首个公共发布版本）
0.7 First Release!  2.5% （2025年Q3）
```

## 许可证

BUckyOS是一个自由，开源，去中心的系统，鼓励所有的厂商都可以基于BuckyOS构造自己的商业产品。因此采用双许可证。一个是传统的GPL Base的许可证，当传统的开源项目使用BuckyOS时使用。另一个是SouceDAO Base的许可证，当一个issue了Token的组织使用BuckyOS时使用，按该许可证的要求，该组织需要将自己issue的Token的一部分捐赠给BuckyOS DAO。
