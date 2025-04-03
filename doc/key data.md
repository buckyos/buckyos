# 系统里的关键数据(理解备份与恢复)

kv:// 保存在SystemConfig里的KV数据（结构化的)，读多写少
dfs:// 保存在系统分布式文件系统的（非结构化）数据，系统会尽力维护DFS数据的可靠性和可靠性。当以单OOD模式运行时，会映射到单OOD的特定目录(在/opt/buckyos/data )
fs:// 指定设备的本机文件系统，使用时一般还需要指定磁盘ID。 fs://一般只开放给系统服务做存储使用，对应用服务来说，这个分区

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

4. Doc:对系统来说只读的一些可验证的配置,通常由明确的签发人
    保存在doc中的内容不能修改 ,修改需要走重发布流程
    UserDoc，创建用户时导入/创建
    DeviceDoc，设备加入集群时创建，一般以JWT的形式存在，需要Zone Owner的有效签名
    AppDoc 由发行者创建（由于量太大，已知的AppDoc一般是通过Repo的meta-index-db管理，已经安装的AppDoc会嵌入到AppConfig中
    ServiceDoc 由发行者创建

## 权限控制

系统权限控制的基本4元组是(userid,appid,action,target_res_path)
"用户使用应用想目标资源发起了动作）
权限模式取`交集`模式，当userid和appid同时拥有权限时，权限判定才能成功。

RBAC权限管理基本只对服务生效，本地文件系统(fs://)走传统的基于文件系统的权限控制。我们目前只区分：

- 不运行在docker里的进程：原则上有所有权限，通过开发规范控制自己的访问
- 运行在docker里的进程：能访问的fs://目录严格受到docker磁盘挂载逻辑的限制

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
func:用户安装的,只在工作流中存在的扩展应用,比如为照片处理增加一种算法.这类应用不会常驻后台,按需拉起

## 系统数据

kv://boot/config 系统ZoneConfig,该配置是所有人可读的
kv://system/verify_hub/key 
系统启动时构造的verify-hub私钥，公钥在boot/config里

kv://system/rbac/model 系统RBAC模型数据，一般不修改
kv://system/rbac/base_policy 系统RBAC的基础策略，一般不修改
kv://system/rbac/policy 实际的系统RBAC策略，由调度器根据base_policy和系统的当前用户生成

dfs://library/ 
属于zone(或则说是属于root的),所有人都有读权限和发布权限的目录,只有ROOT有写权限和删除权限 会映射到 /opt/buckyos/data/library/

### NDN数据

从规划上来说,我们的目标DCFS的底层就是NDN，其数据存储底层是一致的，只是用不同的结构来展示。
- DFS，传统的树结构访问
- NDN，提供面向未来的图结构访问，并能与cyfs://的ndn访问直接兼容

目前，需要能被cyfs://协议访问的数据需要放入named_mgr, named_mgr需要占用独立的存储空间。
get_buckyos_named_data_dir(mgr_id: &str) 
/opt/buckyos/data/ndn/$mgr_id/ --> dfs://ndn/$mgr_id/ 

## KernelService相关数据

目前暂时不区分KernelService和FrameService. 

#### 配置数据

kv://services/$servic_id/config 服务的运行状态,由调度器构造，包含了高效访问该服务的关键信息
kv://services/$servic_id/settings 服务自己的配置,一般由服务自己的面板配置。

#### 服务数据

get_buckyos_service_data_dir($service_name) --> /opt/buckyos/data/$service_name/ --> dfs://system/data/$service_name/

get_buckyos_service_local_data_dir($service_name,$diskid)->/opt/buckyos/local/$diskid/$service_name
该目录不会映射到dfs上,如果不指定diskid(或使用默认磁盘）则直接是local/$service_name
我们推荐的OOD配置,不会在单机的fs层面逐渐复杂的存储池，而是吧这个能力统一交给DFS


#### 缓存数据

重要缓存是非用户创建的，对应用完整性有显著帮助的数据。在空间足够的情况下，系统会尽量保存这类数据。比如对一个社交服务来说，其它用户的头像都应该保存在Cache中。

get_buckyos_service_cache_dir($service_name) --> /opt/buckyos/cache/$service_name/ --> dfs://system/cache/$service_name

本地缓存数据则等价与传统的/tmp数据，用来保存计算的中间结果。系统会定时清理本地缓存数据

get_buckyos_service_local_cache_dir($service_name) --> /opt/buckyos/tmp/$service_name


## 用户数据

用户数据分两类：

- 用户自己管理的用户数据(library)
- 委托应用服务管理的应用数据(AppData)
- 应用可以申请访问用户数据 (library/photo)
- 对用户来说，应用数据可以是

站在用户的角度,只需要备份下面两个目录下的数据就足够了
kv://users/$userid/ 用户的系统配置数据
dfs://users/$userid/  用户的全部个人数据

`用户数据不是和Zone绑定`的,可以通过导出+导入的方法将用户数据迁移到另一个Zone.

### 配置数据

kv://users/$userid/settings 需要sudo才能写
kv://users/$userid/apps/$appid/settings 普通权限即可读写

### 个人数据（传统的home文件夹)

dfs://users/$userid/home/ -> fs://opt/buckyos/data/$userid/home/
用户的全部私有个人数据,可以授权部分文件夹给指定应用访问

### 应用数据

fs://opt/buckyos/data/$userid/$appid/ -> dfs://users/$userid/appdata/$appid/ 

### 应用缓存数据
fs://opt/buckyos/cache/$userid/$appid/ -> dfs://users/$userid/cache/$appid/
 
### 应用本地缓存数据：
/opt/buckyos/tmp/$userid/$appid


## 访客(好友)数据和权限

默认可以访问Zone通过NDN公开的数据


## 设备数据

### kv://devices/$device_id/info 

设备的实时状态信息,由node_daemon汇报,对访问来说最重要的信息就是实时ip地址信息
device得到自己的ip地址信息并不是一个简单的事情

### kv://devices/$device_id/doc 

设备的DID Docment(JWT),由集群的Owner添加设备时写入。所有人都可读


## node相关

kv://nodes/$device_id/config 节点的运行配置信息,由调度器构建
kv://nodes/$device_id/gateway_config 节点的网关配置,由调度器构建


## 保留的name

name要放在域名里，因此首先要是一个合法的域名（注意不能包含下划线和.)

保留的名字:
owner,root,admin,user,device,ood,kernel,service,services,app,guest,
node-daemon,scheduler,system-config,verify-hub,repo-service,control-panel
应用名必须是 `发行商-应用名` 形式,发行商名字中不能包含-





