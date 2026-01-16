# CI 整理
基本流程

触发CI后，尽快构建可以release 的安装包
基于上述pkg，进入自动换验证环节
由人完成通过验证的pkg的正式发布


## 构建

### 安装包类型

- buckyos desktop 手工安装包
- buckyos 全平台免维护包
一般由专用硬件搭载，基于pkg system进行自动升级。
- cli工具(devkits) 用 类似apt install 


### 通用流程

- 构建(cargo update && buckyos-build)
- 制作rootfs
  - buckyos-install
  - make_config.py release
  - 集成其它组件: download artifact 
    - 每个artifact通常是由另一个CI流程制作的
    - artifact通常已经有数字签名，最好是已经发布
- 对rootfs里的必要组件进行数字签名
- 使用目标平台配置，制作安装包 
- 对安装包进行数字签名（这个能在git action完成么？）

### 全平台包为什么复杂？

- 全平台包需要在repo data里包含这个版本所有的pkg,因此需要整合所有平台的编译结果
- 为了实现 device<-ood<-source的升级流程，需要构造正常的 pkg-env / pkg-meta-db
- pkg system基于ndm作为底层，因此还需要构造正确的初始ndm数据文件夹
- ci版本的pkg和pkg-meta-db，都未发布，处于“本地领先状态”


## 验证

- git action单元测试(git action触发:cargo test) 
- 基于安装包的指定平台测试（使用生产环境）
  - 除了一次性验证以外，也有涵盖了所有典型环境的测试用户，自动进行覆盖安装 （这些测试节点应该很方便通过回滚到特定版本）
  - 打开自动升级的节点，只会对发布后的版本进行验证。这些节点重点验证alwasy run,能及时发现生产系统的服务异常

- 基于安装包的稳定二进制+虚拟机配置，在虚拟机环境完成测试颜值


## 人工发布

buckyos 基于 cyfs://名字体系构建，发布app(pkg)通常意味着

- 更新一些did-document,比如 did:bns:$appname.buckyos ，让其指向最新版本。该机制主要是在低层给与了内容发布者脱离收录者独立更新的能力
- 构建发布的pkg,pkgid是  $channel-$aarch-$os.buckyos_appname,构建pkg后会得到pkg-meta(pkg meta也是一个fileobj)
- 上传fileobj到OOD的
- 调用repo-service api,在meta-index-db中，更新 version->meta-obji

还有两种传统发布：

1. 发布到github release
2. 发布到网站的2个url,通常是最新版本和详细版本(0.5.1+build260115这种)

通常发布到github release的频率会更高。发布到github release的app通常可以被其它app集成


## BuckyOS 相关发布

### cyfs-gateway （安装BuckyOS Service一定会安装)

平台:Linux CLI
Windows / OSX 平台通过BuckyOS Desktop Service分发

### buckyos 的默认应用

我们应该给应用开发提供标准的CI流程
当需要的时候，buckyos可以选择集成特定应用的nightly-release(或head release)

- filebrowser


### BuckyOS App （包含在BuckyOS Desktop中）

平台: Windows,OSX, Android

### Buckycli (buckyos-devkit?)

面向开发者的开发环境构建？这个需要设计

### BuckyOS Desktop Service （包含在BuckyOS Desktop中）


平台:Windows, OSX, 以系统服务形式运行
依赖: Docker / orbstack,要确认相关配置允许docker被系统服务访问，并允许mount /opt/buckyos/的相关目录
应该再安装前对依赖项目进行检查

未来开放多节点后可以装在无docker的环境，但只能运行系统应用 （OOD必须支持docker,系统里至少要有一个节点支持docker)

### BuckyOS

在Linux发行版上运行，是buckyos service原生的运行方式。

- 出厂预装安装，
- 使用该发行版支持的软件包安装(deb, rpm, pkg)
- 下载一个.sh脚本安装 (类似rustup)



