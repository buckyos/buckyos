# 为什么要做demo
由于系统的整体设计并不简单，因此我计划通过一个DEMO来说明整个系统的设计思路，帮助更愿意通过代码来了解系统设计的朋友

## 目标
Demo的目标是完成一个简单的原型，启动后可以启动分布式文件系统并提供smb://，并且能通过命令行增加、减少设备。


## Demo 实现简化的部分
1. DEMO会简化权限控制
2. DEMO会简化 一些K/U 切换逻辑，但依旧会在必要的去区分K/U
3. DEMO会简化错误处理

## Demo 涉及的主要流程
1. Zone的配置、启动、配置修改（增加设备）
2. 在启动的Zone中运行DFS，并通过smb://$zoneid/ 提供服务
3. 通过在Zone中增加一台VPS，让smb://$zoneid/ 可以很好的在公网访问 （良好的客户端会自动的选择公网、内网ip)

## Demo 涉及的核心组件
nameservice (使用dns保存全局配置)
node_daemon
sys_config + etcd
pkg_mgr（可选？）
dfs
    ceph ? 其它选项？
    dcfs @ fuse?


bucky_cli
    1. 产生dns配置
    2. 直接修改etcd

## 实现的基本原则：
1. 在demo目录下完成
2. 除未来可在正式项目中使用的基础库外，每个组件尽量用单文件实现
3. 做简单的设计即可，可以看成是一个BuckyOS的教学版本实现

## Power By AI
在DEMO这个规模里，尝试进行AI friendly的工程配置，尽量让AI来编写更多的代码。
目前还是传统的使用ChatGPT代替Google获得示例代码，再通过Copliot来提升代码的编写效率。
也许应该在不那么着急，且更独立的基础组件里来尝试更AI Friendly的工程配置。
