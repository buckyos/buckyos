# 系统里的关键数据(理解备份与恢复)

/config/ 保存在SystemConfig里的KV数据（结构化的)，读多写少
cyfs:// 保存在系统分布式文件系统的（非结构化）数据，系统会尽力维护数据的可靠性和可用性。单机版(桌面版)会直接映射到 $buckyos_root/data/ 下

> /config/ 统一变成 /config/原路径（只读,写入需要通过SDK API）
> cyfs:// 映射到标准的linux路径风格
> 系统内部的路径都是linux风格的，windows的路径用wsl风格 /mnt/c/xxx 但我们不鼓励使用
> 应用层来说，他们看到的是一个sandbox中的文件系统，看不到Node Host的文件系统。在这个sandbox文件系统中，所有文件夹的结构是固定（不会受到buckyos desktop版本安装目录选择的干扰）
> 用户存储配额由res_pool管理
> 详细的路径映射规范见 path_usage.md

## 基本术语

1. Info: 保存上报的信息,除了了上报方,对其他人只读
    DeviceInfo, (NodeInfo) 目前没有区分DeviceInfo和NodeInfo,所以暂时没有NodeInfo 

2. Settings:允许用户调整的功能配置,调度器(系统)不会自动修改
        AppSettings
        ServerSettings
        UserSettings
        SystemSettings
3. Config:调度器(系统)自动构建的运行配置,不允许用户手动修改
        AppConfig, ServerConfig，根据ServiceInstance实时刷新的权重信息
        NodeConfig, 调度器产生的，运行在指定设备上的 实例配置，包含AppInstanceCopnfig和KernelServiceInstanceConfig
            - AppInstanceConfig
            - KernelServiceInstanceConfig
            - ServiceInstanceConfig

4. Doc:对系统来说只读的一些可验证的配置,通常有明确的签发人
    保存在doc中的内容不能修改 ,修改需要走重发布流程
    UserDoc，创建用户时导入/创建
    DeviceDoc，设备加入集群时创建，一般以JWT的形式存在，需要Zone Owner的有效签名
    AppDoc 由发行者创建（由于量太大，已知的AppDoc一般是通过Repo的meta-index-db管理，已经安装的AppDoc会嵌入到AppConfig中
    ServiceDoc 由发行者创建
    AgentDoc，由发行者构建

## 权限控制

系统权限控制的基本4元组是(userid,appid,action,target_res_path)
"用户使用应用想目标资源发起了动作）
权限模式取`交集`模式，当userid和appid同时拥有权限时，权限判定才能成功。

RBAC权限管理基本只对服务生效，本地文件系统走传统的基于文件系统的权限控制。我们目前只区分：

- 不运行在docker里的进程：原则上有所有权限，通过开发规范控制自己的访问
- 运行在docker里的进程：能访问的目录严格受到docker磁盘挂载逻辑的限制

### 内部用户权限

按权限由高到低:

- limit_user 受限用户 （目前未支持)
- user 普通用户(userid):普通用户，能读所有用户数据，可写用户的非敏感数据
- sudo_user:普通用户的管理模式，可以修改用户的所有数据
- admin 管理员用户，可以读取系统的所有数据，写系统的非敏感数据
- root: zone-owner，sudo admin ,可以读写系统的所有数据。

### 外部用户权限

考虑到外部用户关系的应用相关复杂性，目前系统只区分3个级别
我们希望这些权限可以由应用来自行管理。

- guest 匿名外部用户
- contact 已知信息的联系人
- friendOf(user_id) 系统内某个用户的好友

### 如何正确的处理sudo?

普通用户的sudo主要是防止用户误操作自己的敏感数据

- 比如安装应用不需要sudo,但是删除应用需要sudo(因为会导致数据丢失)
- sudo权限的获取? 类似Linux UAC，用户需要再登陆一次。
管理员用户的sudo主要是在对系统进行关键修改时需要
为了保护root私钥，root权限很少使用,使用时必须使用秘钥(目前系统里只有修改zoo_config是需要root权限的),使用Root权限的场合通常都要签名. 系统永远不保存root的私钥,因此系统不可能自动化的执行任何root权限操作

### 各种服务权限（由高到底）

kernel:内核服务，未指定appid视在内核态工作,可以读写系统的所有数据
services:系统服务，
app:应用服务,用用户绑定
fun_instance:用户安装的,只在工作流中存在的扩展应用,比如为照片处理增加一种算法.这类应用不会常驻后台,按需拉起

## 系统数据

/config/boot/config 系统ZoneConfig,该配置是所有人可读的
/config/system/verify_hub/key 
系统启动时构造的verify-hub私钥，公钥在boot/config里

/config/system/rbac/model 系统RBAC模型数据，一般不修改
/config/system/rbac/base_policy 系统RBAC的基础策略，一般不修改
/config/system/rbac/policy 实际的系统RBAC策略，由调度器根据base_policy和系统的当前用户生成

cyfs://$zone_id/srv/library/ 
zone级别的库数据,对zone外不可见,zone内默认所有用户都有读写权限,但是只对自己创建的文件有删除权限。映射到 $buckyos_root/data/srv/library/

### NDN数据

从规划上来说,我们的目标DCFS的底层就是NDN，其数据存储底层是一致的，只是用不同的结构来展示。
- DFS，传统的树结构访问
- NDN，提供面向未来的图结构访问，并能与cyfs://的ndn访问直接兼容

目前，需要能被cyfs://协议访问的数据需要放入named_mgr, named_mgr需要占用独立的存储空间。
get_buckyos_named_data_dir(mgr_id: &str) 
$buckyos_root/storage/ndn/$mgr_id/ --> cyfs://ndn/$mgr_id/ 

## KernelService相关数据

目前暂时不区分KernelService和FrameService. 

#### 配置数据

/config/services/$servic_id/config 服务的运行状态,由调度器构造，包含了高效访问该服务的关键信息
/config/services/$servic_id/settings 服务自己的配置,一般由服务自己的面板配置。

#### 服务数据

get_buckyos_service_data_dir($service_name) --> $buckyos_root/data/var/$service_name --> cyfs://$zone_id/var/$service_name/
服务有读写权限,系统卸载的时候会删除

#### 服务持久数据

get_buckyos_service_home_dir($service_name) --> $buckyos_root/data/srv/$service_name --> cyfs://$zone_id/srv/$service_name/
服务有读写权限,系统卸载的时候不会删除

#### 缓存数据

重要缓存是非用户创建的，对应用完整性有显著帮助的数据。在空间足够的情况下，系统会尽量保存这类数据。比如对一个社交服务来说，其它用户的头像都应该保存在Cache中。

get_buckyos_service_cache_dir($service_name) --> $buckyos_root/data/cache/$service_name --> cyfs://$zone_id/cache/$service_name/
服务有读写权限,系统卸载的时候会删除

#### host-node-path(通常不鼓励使用)

get_buckyos_service_local_data_dir($service_name) --> /opt/buckyos/local/$service_name
该目录不会映射到cyfs上,面向高级开发者提供高性能本地存储选项

get_buckyos_service_local_cache_dir($service_name) --> /tmp/buckyos/$service_name
本地缓存数据等价于传统的/tmp数据，用来保存计算的中间结果。系统会定时清理本地缓存数据


## 用户数据

用户数据分两类：

- 用户个人数据：保存在 cyfs://$zone_id/home/$userid/ 下，包括个人文件、应用数据等
- 用户配置数据：保存在 /config/users/$userid/ 下，包括用户设置和应用设置

> 应用可以申请访问用户个人数据中的特定目录（如 home/$userid/Photos）
> zone级共享数据（如 srv/library/）不属于用户数据，由zone级备份覆盖

站在用户的角度,只需要备份下面两个目录下的数据就足够了
/config/users/$userid/ 用户的系统配置数据
cyfs://$zone_id/home/$userid/  用户的全部个人数据

`用户数据不是和Zone绑定`的,可以通过导出+导入的方法将用户数据迁移到另一个Zone.

### 配置数据

/config/users/$userid/settings 需要sudo才能写
/config/users/$userid/apps/$appid/settings 普通权限即可读写

### 个人数据（传统的home文件夹)

cyfs://$zone_id/home/$userid/ -> $buckyos_root/data/home/$userid/
用户的全部私有个人数据,可以授权部分文件夹给指定应用访问

### 应用数据

cyfs://$zone_id/home/$userid/.local/share/$appid/ -> $buckyos_root/data/home/$userid/.local/share/$appid/
app有读写权限的永久数据区,谨慎写入。默认在app卸载时删除,用户可选保留

### 应用临时数据
/tmp/buckyos/$appid


## 访客(好友)数据和权限

默认可以访问Zone通过NDN公开的数据


## 设备数据

### /config/devices/$device_id/info 

设备的实时状态信息,由node_daemon汇报,对访问来说最重要的信息就是实时ip地址信息
device得到自己的ip地址信息并不是一个简单的事情

### /config/devices/$device_id/doc 

设备的DID Docment(JWT),由集群的Owner添加设备时写入。所有人都可读


## node相关

/config/nodes/$device_id/config 节点的运行配置信息,由调度器构建
/config/nodes/$device_id/gateway_config 节点的网关配置,由调度器构建


## 保留的name

name要放在域名里，因此首先要是一个合法的域名（注意不能包含下划线和.)

保留的名字:
owner,root,admin,user,device,ood,kernel,service,services,app,guest,
node-daemon,scheduler,system-config,verify-hub,repo-service,control-panel,
library,publish,home,cache,var,storage,local,tmp,logs
应用名必须是 `发行商-应用名` 形式,发行商名字中不能包含-





