# BuckyOS 系统架构设计（基于当前实现）

本文档集是一份“基于当前代码实现”的系统架构说明，目标是让读者在不阅读全部源码的情况下，理解 BuckyOS 的交付形态、核心组件边界、关键主链路，以及与常见系统（传统单机服务、K8s、家庭 NAS、P2P 应用）相比差异最大的设计点。

约束与优先级（发生歧义时）：实际代码 > notepads > doc。

## 目标与边界
- 面向个人/家庭/小团队的“Zero-OPS”分布式系统，提供 Zone 级别的统一管理与应用交付。
- 覆盖系统交付与运行主链路：引导/激活、启动、调度、安装升级、访问与权限。
- 不展开业务应用内部逻辑与具体 UI 交互细节。

## 核心概念（快速索引）
- Zone：用户拥有的逻辑云/集群，包含多个设备。
- OOD：Zone 内的核心节点形态（可以是 1 个，也可以是 2n+1 个的集合），承载 system-config 等关键能力。
- Node：普通设备节点，可运行应用或系统服务。
- ZoneGateway：对外访问入口（通常由 OOD 承担，也可为独立节点）。
- NodeGateway：每台节点的本地网关能力（基于 cyfs-gateway），提供 127.0.0.1:3180 的一致入口。

## 阅读导航（推荐顺序）

1) 总览与差异点
- `new_doc/arch/01_overview.md`

2) 启动与激活（Secure Boot / ZoneBootConfig / OOD 连接）
- `new_doc/arch/02_boot_and_activation.md`

3) 系统配置：system-config（KV Source of Truth）
- `new_doc/arch/03_system_config.md`

4) 调度：scheduler（确定性调度 + 写回 node_config / service_info / rbac）
- `new_doc/arch/04_scheduler.md`

5) 交付与升级：pkg-system / repo-server / pkg-env / node-daemon
- `new_doc/arch/05_delivery_pkg_system.md`

6) 网络与访问：cyfs-gateway / NodeGateway / ZoneGateway / SN
- `new_doc/arch/06_network_and_gateways.md`

7) 身份与权限：verify-hub / session-token / RBAC
- `new_doc/arch/07_identity_and_rbac.md`

8) API Runtime：buckyos-api-runtime（服务发现、登录、调用路径选择）
- `new_doc/arch/08_api_runtime.md`

9) 常见踩坑与工程建议
- `new_doc/arch/09_pitfalls.md`

10) 参考材料（从原目录集成，便于交叉链接；不保证最新，以代码为准）
- `new_doc/ref/README.md`
- `new_doc/ref/notepads/`
- `new_doc/ref/doc/`

## 术语与端口（当前实现的关键常量）
- system-config 服务主端口：`3200`（见 `src/kernel/sys_config_service/src/main.rs`）
- 激活服务端口：`3182`（见 `src/kernel/node_daemon/src/active_server.rs`）
- NodeGateway 默认入口端口：`3180`（见 `src/kernel/buckyos-api/src/runtime.rs`）
- RBAC 策略缓存传播延迟：最长约 `10s`（见 `notepads/rbac.md`）

## 参考入口（非强制）
- notepads（实现随笔与设计动机）：`notepads/`
- 旧文档（可能过时，需以代码为准）：`doc/`
