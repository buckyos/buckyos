## 调度器核心逻辑

核心逻辑是“将系统里的服务实例化，确定在哪些节点(设备)上运行哪些服务"
随着系统里的服务配置变化，系统里节点的变化，上述结果也会不断的改变。

通用调度器被称作PodScheduler,其核心逻辑如下:

- 定义被调度对象 (Pod)
- 定义资源载体 (Node)
- 定义Pod 实例化结果 (Instance)
- 定义调度任务(OPTask)，实现Instance的迁移
- 调度算法的主要部分
  - 实例化调度算法：分配资源：构造PodInstance，将未部署的Pod绑定到Node上
  - 反实例化：及时释放资源
  - 动态调整：根据运行情况，系统资源剩余情况，相对动态的调整Pod能用的资源
  - 动态调整也涉及到实例的迁移
  - 识别新Node，并加入系统资源池
  - 排空Node，系统可用资源减少
  - 释放Node，Node在系统中删除
- 产生一致性配置
  - 根据系统里的用户/设备/应用 的情况，构造最终的rbac policy表
  - 根据instance的运行情况，构造service_info和selector

## Instance化
实例化是调度器最重要的行为，运行中的buckyos根本上一个instance的集合
调度器只关心Instance的构造，并将Instance加入到node_config中，随后node_main_loop会根据instance的配置运行执行代码
站在调度器的角度，实例化的主要工作有
- 构造instance(PodInstance)
- 将instance加入到node_config中
- 将instance加入到node_gateway_config中，以让其可以被访问

## service port分配

一个Node上的端口是一种典型的需要分配的资源。对于大部分服务来说，都可以通过

Instance端：

- 根据node_config启动instance
- 启动时基于启始port开始尝试bind,bind成功得到实际端口
- instance定义上报instance_info,包含实际端口

调度器：

- 根据instance_info+service_settings构造service_info

客户端：

- 通过service_info选择可用的service url,其中包含实际端口。

为了方便测试，端口号的启始分配尽量符合某种规律（比如app port和app index有关）

这两步来允许instance自由选择端口。

### 不允许自由选择的端口

- system_config的端口（固定是3980）
- 

### 范式

所以在Node上的强占式资源都可以通过上述“先到先得+上报+调度器整理+客户端选择”的逻辑来使用

## 资源限制(分配)

传统的OS,调度器的核心是给特定进程分配时间片(CPU资源）。BuckyOS的调度器通过一致的设计，可以管理广义的系统资源。
调度器的一个重要工作是对资源的使用继续管理，资源的分配有2种

- 累积性限制的资源，常见的有 存储空间，总流量
- 瞬时限制的资源，常见的有CPU/内存占用。
  (可以把瞬时限制的资源，看做一个清空时间极短的 累计性限制资源来看)。

调度器只对资源的限制进行分配，并不负责执行。由运行容器来执行资源的限制，调度器并不假设这些限制必定能正确执行。基本逻辑如下

- 调度器查看系统资源的实际使用情况
- 调度器查看系统资源的分配情况
- 调度器查查每个Instance的资源使用情况
- 对Instance的资源限制进行调整
- 减少Instance来释放资源
- 迁移Instance来让资源使用更平衡
- 准备下一步的资源分配池

### 资源限制的两种方式

- 绝对限制，基于数值的限制
- 峰值比例限制，在资源限制时相当于没有限制，只有当产生竞争的时候该限制才会有效。 最典型的时上传带宽限制
  如果instance1的带宽限制权重为10，instance2的带宽限制权重为20，那么当系统总带宽为30MB时，理想的情况下instance1使用了10MB带宽，instance2使用了20MB带宽

## OPTask

调度器对node进行管理的需求

node只需要加入集群后，就可以完全零运维，后续的运维操作点

1. 无法启动的(硬件)故障（无法启动操作系统，无法启动node_daemon)
2. 无法接入网络（连接到任意OOD），这通常是因为网络拓扑结构变化引起的

定义OPTask：OPTask可以扩展，通常是一个python脚本。注意OPTask都是幂等的，并尽量是事务的

常见会导致故障的OPTask

- 升级操作系统
- 修改操作系统的驱动
- 修改操作系统的关键数据（这个和升级操作系统很类似）

===> 需要一个定制的，hyper-2 层操作系统。总是可以有效的回滚操作状态到上一个有效的版本

根据不同的常见操作系统发行版，实现一些通用的OPTask

## zone-gateway的确定
如果不特别说明，所有的ood都默认是zone-gateway
用户可以手工将任意node设置为zone-gateway
系统的zone-gateway列表保存在zone_config中（boot/config
Zone-gateway列表的第一个，是默认zone-gateway
SN转发流量到默认zone-gateway
端口转发，用户需要将端口转发的目标配置为默认zone-gateway
(什么时候确定是不是zone-gateway? 所有的ood都是zone-gateway有什么问题？zone-gateway的配置是不是都相同？)


## 调度器的幂等性

为了减少复杂度，调度器的各种算法都是幂等性的，也就是说基于相同的 Pod/Node/Instance 集合，调度器必然应该得到相同的结果.
另一方面，这种幂等性也意味着没有局部调度算法：调度器不会基于某个集合的特定改变来构造算法。

调度器循环

```
loop:
    等待10秒或唤醒
    读取系统状态
    如果状态没改变，则continue
    创建调度器
    调度结果 = 执行调度算法
    如果调度结果与上次调度结果相同，则continue
    执行调度结果（写入system_config)
    保存系统状态
    保存调度结果
```

上述循环有两个提前退出点，用来优化系统性能

- 如何判断系统的状态未发生变化(instance如果包含资源使用情况，那么未发生变化非常难)
- 如何判断调度结果未发生变化（减少一次无效写入）


## 调度服务的接口

### 强制调度

唤醒loop即可。对大部分的添加用户/添加设备/添加应用 的用户操作，底层都可以出发一次强制调度来让修改立刻生效

### control_panel的后端

对system_config进行业务级修改有2种方法

- 在client使用system_config_client的事务，结合业务逻辑修改一组config（封装通常在control_panel里）
- 由scheduler实现control_panel的后端来提供更强一致性的系统状态改变（通常涉及到某种计算）。

### control_panel接口一览

首先要对各种userid/deviceid/appid 的命名合法性进行限制，去掉非法字符，不允许是保留id, 去重的时候是无视大小写的

#### 应用管理

- 添加应用 

- 删除应用

- 启动应用

- 停止应用

- 修改应用settings

settings的内容由应用服务读取

- 修改应用的install_config

install_config由系统读取，应用无法感知自己的install_config


#### 用户管理

- 添加用户

- 删除用户

- disable用户

- enable用户

- 导出用户数据

- 导入用户数据

#### 设备管理

- 添加设备
构建device_doc并由owner签名
将device_doc加入到devices/$deviceid/doc 目录
如果是node（将运行node_daemon），则创建 nodes/$deviceid/config, nodes/$deviceid/gateway_config目录（均有默认值）

等待node_daemon启动，会自动更新device_info
等待scheduler工作，会根据device_info在增加新的NodeItem,并等待调度新的Instance上来


- 删除设备

- 停用设备

- 启用设备

- 修改设备的能力配置 (DeviceConfig)

- 调整设备的Settings (影响调度器对设备的使用)

#### 系统设置


#### 系统服务管理
系统服务通常无法手工安装和删除
部分系统服务允许手工启用/停用


- 修改服务配置

- 重启服务

- 启用服务

- 停用服务

