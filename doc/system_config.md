System Config

# 目标

统一配置管理：为所有上层应用提供一个统一的配置管理接口。
实时更新：支持配置的实时更新，无需重启应用。

# 技术栈

底层存储：etcd，一个高可用的分布式键值存储系统，用于保存配置数据。etcd 可替换
通信： etcd 节点内部通信，RESTful 或 gRPC，systemconfig 和上层通信

# etcd key 设置

```
/devices
/systeminfo
/services/<service_name>
/nodes/<node_name>
/nodes/list  # 所有节点列表 [node_name1, node_name2]

```

# 组件

buckycli 一些初始化工作,import node
node_deamon 维护 node 信息，包括 node_name, ip, port, status, last_update_time
systeminfo 由 system-updater 读取和收集 nodes 信息后更新

web-service 每次被请求时，直接读取 etcd 中的最新配置信息, 通过接口返回给 web
web
