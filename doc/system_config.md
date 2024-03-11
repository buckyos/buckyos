
System Config

# 目标
统一配置管理：为所有上层应用提供一个统一的配置管理接口。
实时更新：支持配置的实时更新，无需重启应用。


# 技术栈
底层存储：etcd，一个高可用的分布式键值存储系统，用于保存配置数据。etcd可替换
通信： etcd 节点内部通信，RESTful或gRPC，systemconfig和上层通信


# 核心组件

## 配置管理
读取配置：提供API接口，允许上层应用查询特定的配置项。
更新配置：提供API接口，允许授权用户更新配置项。
监听配置变更：上层应用可以注册监听器，当配置项发生变化时，能够收到通知。

## 安全
访问控制：通过API密钥或OAuth等机制控制对配置管理API的访问。


## 开发和运行
日志对接，状态监控
测试



```
class ConfigClient:
    get_config(key)
    set_config(key, value)
    watch_config(key, callback)
```