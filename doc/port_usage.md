# BuckyOS 使用的端口

> 以代码为准。核心常量定义在 `src/kernel/buckyos-api/src/` 各 client 模块，
> kernel/frame service 的实际监听端口由 scheduler 在
> `src/kernel/scheduler/src/system_config_builder.rs` 中分配。

## 系统保留端口

系统需要为 buckyos 保留的端口（这些端口被占用会导致异常）。

| 端口 | 用途 | 访问范围 |
| --- | --- | --- |
| 80 / 443 | zone-gateway 的 http / https 端口 | 可以完全打开 |
| 2980 | rtcp stack 默认端口（`DEFAULT_RTCP_PORT`） | 可以完全打开 |
| 3180 | node-gateway(cyfs-gateway) http 端口 | 建议只允许本机访问 |
| 3181 | node_daemon kevent 订阅端口（main / http） | 仅本机 |
| 3182 | device active 协议用的 http 端口 | 仅局域网 |
| 3183 | kevent native 端口 | 仅本机 |
| 3200 | system_config 端口 | 仅局域网 |

## 内核服务端口（kernel service）

内核服务端口由 scheduler 固定分配。

| 端口 | 服务 | kapi |
| --- | --- | --- |
| 3300 | verify_hub | kapi/verify_hub |
| 3380 | task_mgr (task-manager) | kapi/task |
| 3400 | scheduler | kapi/scheduler |

## 框架服务端口（frame service, 4000-4999）

| 端口 | 服务 | kapi |
| --- | --- | --- |
| 4000 | repo-service | kapi/repo |
| 4020 | control-panel | kapi/control_panel |
| 4030 | kmsg | kapi/kmsg |
| 4040 | aicc | kapi/aicc |
| 4050 | msg-center | kapi/msg_center |
| 4060 | opendan | kapi/opendan |
| 4070 | workflow-service | kapi/workflow |
| 4100 | smb-service | - |

## 应用服务端口（app service）

应用服务端口通常是动态端口，无需特意保留。

基址 `BASE_APP_PORT = 10000`，端口号通常按 `10000 + appindex * 10` 分配。
