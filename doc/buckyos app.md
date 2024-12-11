## buckyos中的App

### App的定义和分类

- Agent/Workflow App :用自然语言开发,移植自OpenDAN
- 纯Web页面App:用html/css开发,使用buckyos SDK访问系统功能
- Function App (无状态服务):之前BuckyCloud的 云端一体App,一个典型的无状态后端功能是"对一张照片进行处理".
- 有App Service的App,这类Service一般以Docker的形式存在(分平台),有一些APP也允许包含以文件夹存在的二进制包(该格式与系统服务相同)
- 有Client Package的App
- 针对BuckyOS进行特定扩展的System Plugin App

一般来说，buckyos中的app以web服务的方式与用户进行交互，这些web服务自己提供localhost的访问能力，通过gateway转发提供外部的访问能力

一个app要运行在OOD上，应该经过以下步骤：
- 发现，这里的"发现"主要讨论app的元数据应该包含哪些必要信息，通过这些信息可以进行以下的步骤
- 下载
- 安装
- 运行

## APP的安装
- 通过AppId安装,如安装时未制定版本号,则安装发布到当前OS Channel(硬件名+nightly | stable)的最新版本(HEAD)
- 首先 App Installer会把整个App的包(包含所有subpackage)下载到zone repo
- App Installer会收集用户的配置,并在SystemConfig中增加"已安装APP"的配置,配置如下

```json
// kv://apps/{userid}/apps/{app_id},app_id是不包含app的main-name,默认会用到域名里
{

  "app_info" : "jwt",
  "source" : "http://xxx.com/index.db",//是从哪个源得到的app_info
  "enable" : "true", //是否启用
 //安装配置,注意与app自己的setting区分,app没有权限修改本配置
  "data_mount_point" : "/opt/data", //$data_dir是系统基于appid和userid构造的,位于DFS上的目录.系统会自动备份该目录. 有状态应用需要将该目录与docker内部的一个目录关联
  "cache_mount_point" : "/opt/cache",//$data_dir是系统基于appid和userid构造的,位于DFS上的缓存目录.系统会尽量保留该目录以帮助应用提升性能.该配置可为空
  "local_cache_mount_point" : "/opt/tmp",//$local_cache_dir是系统基于appid和userid构造的,位于本地文件系统上的缓存目录.该目录可能随时被清理并且永远不会被备份.该配置可为空

  "data_mount_point" : "/srv",
  "cache_mount_point" : "/database/",
  "local_cache_mount_point" : "/config/",
  "max_cpu_num" : 4,
  "max_cpu_percent" : 80, 
  "memory_quota" : 1073741824, 
  "port_map" : {
     "80" : 20080
   },
  

  "permisons" : { //权限配置,这里的配置与app_info里的取交集
    "extra_dirs" : { //可以额外挂一组目录给app,由于这些目录的权限超过了docker的默认隔离范围,因此需要用户明确的授权才能给用户.
      "dir_1" : "docker inner path",
    },
    "permison_2" : "value_2",
  }
}
```
- App Installer会主动调用Scheduler的onAppInstall方法让其立刻执行一次调度,以让app的安装立刻生效.
  如未调用,Scheduler也会在自己的常规调度里处理应用安装(根据调度器的调度计划安排可能会有所延迟)
- 调度器会产生必要的配置,核心是
1. 对Zone-Gateway的配置修改,允许APP被公网访问
2. 对Node-Gateway的进行必要的配置修改,允许APP被Zone内访问
3. 对node-config进行必要的修改
4. node-daemon会根据node-config进行必要的自动安装,从zone-repo上下载app的相关docker img 或 二进制包并执行安装脚本

- 系统里完整的appid由 appid.username组成
除了部分系统应用,大部分应用都是与用户相关的.比如系统里的两个用户 ,可以互不干扰的使用同一个app的两个不同版本.
这个设计是站在
当系统的管理员账号安装应用时,username默认为空.


#### APP服务的启动
- node-daemon会发根据node-config的配置,以正确的方式用app loader启动app service.如果app service是以docker发行的,那么优先使用Docker.如果
node-daemon所在node不支持docker,则会根据权限配置判断是否可以加载平台相关的二进制文件夹并使用文件夹中启动脚本启动
- node-config中已经配置好了启动的所需要的参数,身份相关的信息Node-dameon会通过环境变量传递给
- 使用app-loader以docker方式启动的app-service,除了做数据隔离外,还会根据用户配置对app-service使用的计算资源进行限制. 
- 以二进制包方式安装的app有自己的启动脚本,这类APP通常是官方或制作商制作的,通过应用商店下载的应用默认不支持二进制模式分发.




### 使用(访问)APP

- 通过浏览器访问APP
这是最常用的方法.当APP启用zone外访问时.用户可以通过 https://$appid.$userid.$zoneid/$app_page.html 访问APP
如果是访问管理员用户(默认用户的)app,可以省略$userid
在zone内,可以通过 http://$node_ip:$app_port 的方式访问app service.该地址在浏览器中通常无法正常使用
用户和系统均可配置 app的快捷方式,以让app的url看起来更短
  比如appidA 配置了系统快捷方式 "photo".那么就可以通过https://photo.$zoneid/访问app
  如果appidB 配置了用户快捷方式 "video",那么就可以通过 https://video.$uername.$zoneid/访问app


### 在App之间共享状态

## 快速DEMO
10分钟移植一个基于docker发行的现有service到buckyOS 

### 创建目录

## 理解预装应用
- HomeStation :默认主页, 使用short name: $ (无子域名)
- System Control Panel 系统控制面板,使用short name: sys

### 预装应用的打包
- control panel: 打包时就拷贝到rootfs/bin
- home station: 
  > - 非windows时使用docker机制，docker pull在安装时进行
  > - windows下使用本地运行机制, install.py中下载程序zip包并解压。之后可能需要重打包


### control panel
这种app是一套纯http页面，与系统中现有的组件进行交互

#### 使用
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