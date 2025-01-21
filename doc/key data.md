# 系统里的关键数据(理解备份与恢复)

## 基本术语
1. info: 保存上报的信息,出了上报方,对其他人只读
    DeviceInfo, (NodeInfo) 目前没有区分DeviceInfo和NodeInfo,所以暂时没有NodeInfo 
    
2. Settings:会允许用户调整的功能配置,调度器(系统)不会自动修改
        AppSettings
        ServerSettings
        UserSettings
        SystemSettings
    
3. Config:调度器(系统)自动构建的运行配置,不允许用户手动修改
        AppConfig, ServerConfig
        AppInstanceCopnfig/KernelServiceInstanceConfig/NodeConfig
        InstanceConfig通常是NodeConfig的一部分
        
4. Doc:对系统来说只读的一些可验证的配置,通常由明确的签发人
    保存在doc中的内容不能修改 ,修改需要走重发布流程
    DeviceDoc
    AppDoc 由发行者创建
    ServiceDoc 由发行者创建

## 权限配置
按权限由高到低:

用户(userid): 
root
administrators
users //如何处理sudo?
limite_users

如何正确的处理sudo?
普通用户的sudo主要是防止用户误操作自己的敏感数据
    比如安装应用不需要sudo,但是删除应用需要sudo(因为会导致数据丢失)
    sudo权限的获取? 需要再登陆一次

管理员用户的sudo主要是在对系统进行关键修改时需要

Root权限很少使用,使用时必须使用秘钥(目前系统里只有修改zoo_config是需要root权限的),使用Root权限的场合通常都要签名. 系统永远不保存root的私钥,因此系统不可能自动化的执行任何root权限操作

应用(appid):
kernel:内核服务
services:系统服务
apps:应用服务,用用户绑定
func:用户安装的,只在工作流中存在的扩展应用,比如为照片处理增加一种算法.这类应用不会常驻后台,按需拉起

## 系统数据

kv://system/rbac/model 系统RBAC模型
kv://system/rbac/policy 系统RBAC策略
kv://boot/config 系统的引导配置,该配置是所有人可读的

dfs://sys/data 系统的一些关键数据,一般是内核服务会读取和保存在此,会映射到/opt/buckyos/data/sys/


## 服务相关
kv://services/$servic_id/config 服务的运行状态,一般由调度器读写
kv://services/$servic_id/settings 服务自己的配置,一般由服务自己的面板配置

权限信息:(注意服务的servic_id都是srv_开头的)
g,$servic_id,services


## 应用相关
kv://users/$userid/apps/$appid/config 应用的运行配置
kv://users/$userid/apps/$appid/settings 应用的语义配置(一般由应用的配置面板配置,如为空则为默认配置)

权限: (注意应用的appid不能和userid相同,目前系统无法区分这种情况)
g,$appid,apps


## 用户数据
用户数据一般通过系统服务访问
应用可以申请访问用户数据
站在用户的角度,只需要备份下面两个目录下的数据就足够了
kv://users/$userid/ 用户的系统配置数据
dfs://users/$userid/ 用户的全部个人数据

kv://users/$userid/ 用户的系统配置数据
dfs://library/ 属于root用户的,所有人都有读权限和发布权限的目录,只有ROOT有写权限和删除权限 会映射到 /opt/buckyos/data/library/
dfs://users/$userid/home/ 用户的全部私有个人数据,会映射到 /opt/buckyos/data/$userid/home/
dfs://users/$userid/appdata/$appid/ 应用数据,会映射到 /opt/buckyos/data/$userid/$appid/

dfs://users/$userid/cache/$appid/ 应用缓存数据,会映射到 /opt/buckyos/cache/$userid/$appid/

/opt/buckyos/tmp/$userid/$appid ,app的本地cace数据,

权限:
g,$userid,users
g,$userid,administrators (管理员)

## 访客(好友)数据和权限
用户总是可以管理自己的好友信息
系统会允许哪些用户可以邀请其它用户"进驻"本Zone,即在 kv://users/$userid/  和 dfs://users/$userid/能有信息
外部用户统一使用did做userid
基于通讯录的分组


## 设备数据
kv://devices/$device_id/info 设备的实时状态信息,由node_daemon汇报
kv://devices/$device_id/doc 设备的DID Document,一般是身份信息

权限:
g,$device_id,devices
一般node_daemon使用设备的身份发起请求


## node相关
kv://nodes/$device_id/config 节点的运行配置信息,由调度器构建
kv://nodes/$device_id/info 节点的状态信息,由node_daemon汇报
kv://nodes/$device_id/gateway 节点的网关配置,由调度器构建






