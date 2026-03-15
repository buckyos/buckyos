# BuckyOS里的Path

## 应用服务视角

应用看到的Path是一个在标准的linux FS，我们在标准的linux FS上做的扩展有

> Agent也是一个应用，比如jarvis agent的权限 就是 appid = jarvis的权限

/opt/buckyos/bin/$appid ：我们鼓励应用开发把二进制文件放在这个目录，这样脱离容器运行时改动也很小（比如容器变成了VM）

/tmp/ => /tmp/buckyos/$appid : App临时数据,有读写权限
/config/* =>  buckyos的system_config : 只读,扩展的system config配置读取,遵循system_config配置。写入需要通过SDK API
    /config/boot/config => buckyos的system_config的/boot/config
/home/$username/ => cyfs://$zone_id/home/$username : 用户home目录，根据配置通常是只读或不可访问, 
/home/$username/.local/share/$appid => cyfs://$zone_id/home/$username/.local/share/$appid ：app有读写权限的永久数据区，谨慎写入。会在app卸载时用户可选删除
/mnt/alice.bns.did/pub/ => cyfs://alice.bns.did/pub/ ：buckyos支持的internet filesystem,由另一个zone配置权限
/srv/library/ => cyfs://$zone_id/srv/library/ : zone级别的库数据，对zone外不可见，zone内默认所有用户都有读写权限，但是只对自己创建的文件有删除权限
/srv/publish/ => cyfs://$zone_id/srv/publish/ : zone级别的分享数据，通常只读
/home/$username/shared/ =>cyfs://$zone_id/home/$username/shared/ : 用户级别的分享数据，通常只读

> 用户存储配额由res_pool管理,各目录的实际可用空间受用户res_pool配置限制

当应用关闭兼容性开关后，应用使用buckyos-sdk访问cyfs://,所有的到cyfs://的自动mount都会消失，本地文件读写只有/tmp/会成功

## 单机版特例

单机版(桌面版)不会运行cyfs FUSE daemon,所以直接映射到本地文件系统：
```
cyfs://$this_zone_id/ => $buckyos_root/data/下
```

桌面安装支持的和当前host-node的Home目录的融合，是通过link实现的，比如建立一个软链接
$buckyos_root/data/home/$username/Documents => c:/users/alice/Documents

## 系统服务视角(kernel/frame service)

服务通常需要通过buckyos-base的API把逻辑path映射到本机path(host-node-path)
服务不在docker里，所以可以直接通过目录转换访问本地目录，要切换到sdk上，方便后续对接cyfs 的fs api
用户通常不会手工管理服务，但系统有考虑面向单个服务提供“重新安装按钮“，用于修复一些问题


### 服务常用的目录

/config/services/$service_name/* :服务保存配置的在system_config
$buckyos_service_data => cyfs://$zone_id/var/$service_name/ => $buckyos_root/data/var/$service_name : 服务有读写权限,系统卸载的时候会删除
$buckyos_service_cache => cyfs://$zone_id/cache/$service_name/ => $buckyos_root/data/cache/$service_name : 服务有读写权限,系统卸载的时候会删除
$buckyos_service_home => cyfs://$zone_id/srv/$service_name/=> $buckyos_root/data/srv/$service_name : 服务有读写权限,系统卸载的时候不会删除

服务也可以使用host-node-path,使用下面两个(通常不鼓励使用):

$buckyos_service_local_cache => /tmp/buckyos/$service_name
$buckyos_service_local_data => /opt/buckyos/local/$service_name

## 内核服务视角

内核服务基本只和node-host-fs打交道
内核服务也可以使用buckyos_service_local_cache，buckyos_service_local_data

### 内核服务目录，这些目录通常只被内核服务访问,是System级别(Zone级别的)
$buckyos_root/data/srv/library/ => cyfs://$zone_id/srv/library/ : zone内共享的资料库
$buckyos_root/data/srv/publish/ => cyfs://$zone_id/srv/publish/ : zone级别的分享数据
$buckyos_root/storage : 内核基础设施在本机的持久化存储(dcfs chunks、named_store,未来可能包含dRDB数据),卸载不会删除
$buckyos_root/etc : 内核配置区,覆盖安装和卸载时的操作是逐个文件定义的


## 安装卸载整理

- 更新：二进制目录 $buckyos_root/bin/apps and service
- 更新时不删除： 默认都不删除
- 软重置: 通过control panel实现，这个是强业务逻辑
- 覆盖安装：覆盖安装的特点是 “如果不存在就复制“
  - 减少rootfs/bin/ 目录外的文件，这意味着应用本身还依赖"必须存在的外部数据" (rootfs是buckyos_root模版) 
- 卸载时不删除 
  - $buckyos_root/data/home/ (用户个人数据)
  - $buckyos_root/data/srv/ (服务持久数据 + zone共享数据)
  - $buckyos_root/storage (内核基础设施在各host-node上的持久化存储)
  - 根据 $buckyos_root/bin/applist.json 决定保留哪些host-node-local-app的数据
- 卸载时删除
  - $buckyos_root/bin/ (二进制文件,删除前先读取applist.json)
  - $buckyos_root/data/var/ (服务运行数据)
  - $buckyos_root/data/cache/ (服务缓存数据)
  - $buckyos_root/local/ (服务本地数据)
  - $buckyos_root/etc/ (系统配置,删除前自动执行身份备份)
  - $buckyos_root/logs/ (日志)
  - /tmp/buckyos/ (应用和服务的临时数据,在$buckyos_root外)

## 软重置
- 重新安装所有服务（更新二进制文件+重置服务）
  - 删除 $buckyos_service_data
  - 删除 $buckyos_service_cache
  - 保留 $buckyos_service_home
- 删除所有应用和应用的数据
- 保留分布式系统的拓扑 （内核数据保留）
- 保留分布式存储（DFS+KV+RDB)
- 保留用户数据

执行细节：待定
