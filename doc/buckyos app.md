## buckyos中的App

一般来说，buckyos中的app以web服务的方式与用户进行交互，这些web服务自己提供localhost的访问能力，通过gateway转发提供外部的访问能力

一个app要运行在OOD上，应该经过以下步骤：
- 发现，这里的"发现"主要讨论app的元数据应该包含哪些必要信息，通过这些信息可以进行以下的步骤
- 下载
- 安装
- 运行

这里以已有的两个app作为例子：

### control panel
这种app是一套纯http页面，与系统中现有的组件进行交互

#### 发现
由于是纯页面，没有平台区别，这里应该至少包含：
- 包的source信息(url, fid)
- 从下边安装的需求来看，是否也要包含route name信息？对外提供服务应该是app的共通需求

#### 下载
通过source信息，下载app页面的压缩包

#### 安装
- 解压压缩包内容到指定文件夹
  > 现在所有的app都放在{BUCKYROOT}/bin下，是否考虑单独分一个{BUCKY_APP}(可以在{BUCKY_ROOT/bin/app})？
- 在gateway的配置文件里添加到这个文件夹的route
  > route的name从哪里来？是否也要放在元数据里？

#### 运行
不需要运行，通过gateway的route进行访问

### homestation
至少包含一个独立二进制，占用一个独有的端口提供服务
可能有以下的启动方式：
- 本机直接启动一个进程
- 通过docker启动
- 可能通过虚拟机镜像启动？
- 或者是一个远程服务地址（external service）？
#### 发现
在这里就要知道启动方式了，根据不同的启动方式，后续下载步骤也是不同的
- 本机启动：source信息是否要细化？不同的平台有不同的包
- docker：image信息
- 虚拟机镜像：镜像的source地址和虚拟机配置？
- external：用户填入一个远程地址？
#### 安装
- 本机启动：下载对应平台的包，解压到{BUCKY_APP}/{APP_NAME}, 包里应该包括启动脚本
- docker：docker pull image
- 虚拟机镜像：下载并创建虚拟机
- external：不需要
#### 启动
- 本机启动：找到{BUCKY_APP}/{APP_NAME}里的start/status/stop脚本
  > 这个脚本其实和启动kernel service的很像，是不是有可能统一成一个？不需要app自己再准备脚本了
- docker：通过统一脚本进行，所有的基于docker的app都用同一套脚本


## 现在的预置逻辑
目前的control panel和home station都是预置的
#### 发现
不需要，预置在默认的gateway.json和scheduler template配置里
#### 安装
- control panel: 打包时就拷贝到rootfs/bin
- home station: 
  > - 非windows时使用docker机制，docker pull在安装时进行
  > - windows下使用本地运行机制, install.py中下载程序zip包并解压。之后可能需要重打包
