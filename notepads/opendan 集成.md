# OpenDAN 集成

1. 作为一个正式的服务加如果buckyos
- 调度器添加 service doc
- bucky_project添加编译选项
- src/rootfs/bin/opendan 添加启动脚本
2. 参考repo_service，编写标准的日志初始化和buckyos_runtime初始化代码
3. 将AI相关的xxx_client集成到buckyos_runtime中，可以通过runtime.get_xxx_client得到
4. Review代码里使用aicc_client,task_mgr_client,msg_queue_client,msg_center_client的地方，该用runtime.get_xxx_client得到


## 正式的各个服务初始化流程

msg_center 要正确初始化
task_mgr 在 get_buckyos_service_data_dir放置db



## 新增的settings
BuckyOS通过统一的方法来保存各个服务的settings,（并将会在control panel中提供修改UI）

aicc 通过 runtime.get_my_settings 得到关键配置，并根据配置初始化各个provider (首先就是不要从env中获取openai api token)
msg_center 通过 runtime.get_my_settings 得到tunnel配置(bot token -> inbox owner did)

从初始化逻辑中推导出这两个service 的settings格式，并完成实现

## 实现Jarvis


