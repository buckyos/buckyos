# buckyos 系统研发计划

## 系统的3个阶段目标（不包括AI部分）的主要需求

### 第一阶段 私有存储集群 (Alpha版规划)

- 核心是集群管理、存储系统的优化、备份与恢复,文件的分享。这些核心功能都能在通过 App/浏览器 + 设备的方式，提供完备已用的UI。（可以参考iStoreOS)。尤其是站在与NAS对标的角度，要做非常完善的产品设计，可以支持产品销售。（不扣分）

- 通过设备销售预设的3个节点是：存储服务器，随身Wifi，公网Gateway。随身Wifi的基本功能是在家的时候作为第二节点，存储空间较小，主要用于“热数据的副本”。其基础操作系统大概率是Android。但用户随时携带时可用电池供电，在弱网（比如飞机上）可对文件系统提供只读访问，能读取最近使用的热数据和最近刚刚写入的新数据。这在大部分情况下都是够用的。

- 这是系统的第一个正式版本，因此我们还需要建立完整的系统内核框架，尤其是u/k区分，以及完备的基础权限控制体系，基础系统管理上支持用户态app.server的安装和更新。

- 备份系统和DMC打通后，可以提供真正的去中心备份生态。Token激励是我们首个版本可以销售的主要加分项。

- 文件的分享通过应用的方式来开发，这是我们早期主要的拉新手段。文件的公网发布需要用户在公网有Gateway Node，可以给我们带来一些潜在的订阅收入。

- 我们的系统架构应该已于在传统的云端运行，可以和容易的在云端创建一个规模的集群。并能通过备份恢复逻辑从云端迁移到边缘。

- 我们需要在生产环境下细粒度的使用GlusterFS,这需要我们能明确的通过配置，实现我们所需要的核心功能。我们自己研发的dcfs-lite会是我们的核心技术护城河，专为私有存储云场景设计：配置更灵活，运行更稳定，运维更简单，性能更好。

## 第二阶段 扩展：软件上兼容OpenDAN，硬件上可以与安防(AI)摄像头结合 

- 主要通过外部合作的方式完成，考验我们系统的可扩展性
- 该场景有传统AI+GenAI的新能力需求。我们的Library 应用要支持用AI管理海量的多媒体文件。
- 将OpenDAN所需要的AI Compute Kernel功能集成。

## 第三阶段 持续完善：与下载、串流等流量业务整合 

- NAS必然是要拥有下载功能的。我们肯定要基于buckyos的app.service框架首先移植一个下载软件
- 强大的RTC基础能力，是未来具有实时感知能力的本地AI Agent的重要基础能力。
- 充分发挥P2P的经验优势，在这个方向上会从实际场景出发继续整合BDT
- 传输证明 + Token化的BitTorrent，实现 种子文件分享、下载加速，上传的Token化。
- 传统的VPN业务，通过传输证明支持雨燕的业务 (Gateway向下兼容今天的Clash生态），希望能承接这个庞大的，已有生态的大量用户。
- 云主机：在VPN的基础上，进一步扩展到云主机生态。用户不但可以更简单的RDP连接自己的设备，还可以更好的出借自己的设备。
- VPN和云主机业务主要通过外部合作方式推进


## Alpha阶段的主要组件

- Kernel Moels
  - [ ] *node_daemon
    - [ ] *app & service loader,实现正式的权限管理和容器隔离
    - [ ] node task execute system,通常用来执行运维任务，如果绕不过去就要实现
  - system config(base on etcd)  
    - [ ] *system-config lib,@alexsunxl,A1
    - [ ] *ACL System
  - system status 用于实现系统状态监控
  - kRPC
    - [ ] *kRPC libs
    - [ ] *授权认证中心
  - kLog
    - [ ] *kLog lib
    - [ ] *kLog server
  - kMQ
  - pkg system
    - [ ] *完善libs，便于其它组件使用
    - [ ] 与task system的集成
    - [ ] *与ACL系统集成
- Kernel Services
  - [ ] *scheduler
  - [ ] *Task Manager
  - DFS
    - [ ] *glusterFS 与ACL集成
    - DCFS (单独列出)
  - dApp manager
    - [ ] *basic API support（源管理,已安装管理,权限配置，安装器）
    - [ ] *in-zone pkg repo server
  - backup system （单独列出）
  - cyfs-gateway (单独列出)
- Frame Services
  - [ ] *smb-service，与ACL集成
  - [ ] *k8s-service,与ACL集成
  - [ ] *http-fs-service,与ACL集成
  - [ ] Notify Manager
  - [ ] *User Inbox
  - [ ] dApp Store
  - [ ] *Contorl panel 根据界面需求，提供基本的系统管理功能（含Web页）
- CyberChat App（暂时命名）
  - [ ] *账号管理
  - [ ] *名字管理
  - [ ] *zone管理
  - [ ] *存储管理(纯展示)
  - [ ] *File UI
- Web2.5 Services
  - [ ] *Web3 通用lib设计
  - [ ] *账号管理+签名服务(含签名历史记录)
  - [ ] *did解析与名字解析 (基于cyfs-gateway)
  - [ ] 名字申请与管理
  - [ ] gateway服务(订阅管理)
  - [ ] *http backup server
  - [ ] 云端zone支持:两种思路
  - [ ] Web2.5 => Web3 迁移
- *BuckyOS Backup Suite
  - [ ] UI
  - [ ] backup basic libs
  - [ ] http target client
  - [ ] dmc target client
  - [ ] dmc target server
- CYFS Gateway
  - [ ] *支持buckyos的demo需求: 基于TAP Device的VPN
- CI/CD *支持
  - [ ] *nightly CI/CD系统
  - [ ] *快速云端开发环境搭建
- DCFS
- SDK


### ALC

基础权限: 
Kernel 可以访问所有数据，拥有所有权限,可以被所有人依赖
Frame 只有自己容器的权限，可以被所有人依赖
dApp 只有自己的权限，不能被人依赖（因为会有潜在的数据泄露风险），dApp安装时可以要求依赖frame service,一并安装














