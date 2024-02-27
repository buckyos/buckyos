# Mile Stones

## Zone管理
Zone的配置分公开配置和私有配置两部分
编辑配置文件并保存到合约上
由编辑工具保障配置文件的正确性，并在进行危险编辑时发出强烈的警告


## Machine激活 
Machine激活后，可以启动NodeDaemon并有足够的配置连上Zone内的etcd集群

## Zone引导成功 （etcd集群启动成功）
如果只有1个OOD，则3个VM都在同一个机器上，随着OOD的加入，逐步吧etcd master vm迁移到不同的机器上
注意运行etcd的Node可能在不同的内网，可以通过cyfs gateway进行互联（本地tcp代理）

etcd master的可靠性是整个系统的基础，需要设计测试来看看这种架构的故障恢复能力

## GlusterFS集群启动成功
文件系统是可替换的，在DCFS完成开发之前，我们使用GlusterFS或CEPH

## kubernetes集群启动成功 

## kb8s app 部署成功 （核心里程碑）



## M*N 的Zone部署成功
创建1个虚拟Zone，其Device运行在多个物理Zone之上
