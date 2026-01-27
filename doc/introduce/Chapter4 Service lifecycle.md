# 第四章：Service 的生与死

如果说第三章解决的是“Zone 怎么从 0 启动起来”，那这一章要解决的是另一个更贴近工程现实的问题：

一个 Service 是如何被定义、被调度、被拉起、被访问、被升级、被停止，最后在失败时如何自愈的？

在 BuckyOS 里，Service 的生命周期不是“docker run”这么简单。
它更像一个闭环系统：

1) 你写下意图（spec / install config / version policy）
2) 控制面把意图写进 system-config（唯一真相源）
3) scheduler 把“意图 + 现状”推导成确定性的动作（node_config / service_info / RBAC / gateway_config）
4) node-daemon 在每个节点上执行收敛，直到目标状态达成
5) 服务实例持续上报状态与端口，反过来成为下一轮调度的输入

这就是 Service 的“生”与“死”。

## 1. 先分清楚：BuckyOS 里有哪些 Service

源码目录（`src/README.md`）把 BuckyOS 的组件分成几层：

- kernel services：Zone 启动前就必须可用的服务（最关键的控制面链路）
- frame services：可扩展的系统服务，通常运行在容器里，可能缺失
- apps（app services）：用户态应用服务，默认不具备系统级权限

在“生命周期”视角下，它们的共同点是：

- 都会在 system-config 里有“被调度/被访问”的表示
- 都会通过 node-daemon 的收敛循环被拉到目标状态

差异主要在“权限边界、运行时类型（runtime_type）、访问路径选择策略”。

## 2. system-config：Service 生命周期的“唯一真相源”

`doc/arch/03_system_config.md` 把 system-config 定义为 Zone 的 KV 真相源。
对 Service 生命周期来说，你最需要记住的是这些关键 KV 路径（同样来自 `doc/arch/04_scheduler.md` 的整理）：

- `services/<service>/spec`：服务的“意图”（应该跑什么、怎么跑）
- `nodes/<node_id>/config`：scheduler 写给 node-daemon 的“执行计划”（本机应该拉起哪些实例、目标状态是什么）
- `services/<service>/instances/<node_id>`：实例上报（node-daemon/运行时写，scheduler 读）
- `services/<service>/info`：服务发现事实（scheduler 写，client/runtime/gateway 读）

只要你理解了这四类 key，后面的“生与死”就可以用非常工程化的方式推导出来。

## 3. 生：从 spec 到实例真正跑起来

### 3.1. 分发面 ready：先把“能跑的内容”准备好

在 BuckyOS 里，“交付与升级”是主链路的一部分（`doc/arch/05_delivery_pkg_system.md`）：

- repo-server 负责同步 pkg-index-db（meta-index-db / `meta_index.db`），并把目标版本准备成 ready
- ready 的语义是“Zone 内分发面准备完成”，而不是“某个节点拉到了镜像”

这个前置步骤把大量失败面收敛到了 repo-server 上：内容没准备好，就不应该让节点进入反复 deploy loop。

### 3.2. 控制面写入意图：spec 进入 system-config

当安装/升级意图写入 system-config 后，scheduler 才能在 dump 时看见输入变化，并推导出新的动作。

### 3.3. scheduler 推导：把意图翻译成 node_config + service_info

scheduler 的定位是“确定性的状态推导器”（`doc/arch/04_scheduler.md`）：

- 输入：system-config dump（全量状态快照）
- 输出：一组调度动作，并写回 system-config

其中最关键的两个输出是：

1) `nodes/<node_id>/config`：告诉 node-daemon 在这个节点上“应该跑哪些实例、目标状态是什么”。
2) `services/<service>/info`：告诉所有调用方“这个 service 现在可以通过哪些节点/端口访问”。

### 3.4. node-daemon 收敛：安装/部署/启动

node-daemon 是“节点收敛器”（`doc/arch/05_delivery_pkg_system.md`）：

- 读取 `nodes/<node_id>/config`
- 对每个 RunItem 执行收敛：NotExist → deploy → start / stop / restart

实现里，收敛语义被做成显式状态机：

- `ensure_run_item_state()`：`src/kernel/node_daemon/src/run_item.rs`
- `ServicePkg`：`src/kernel/node_daemon/src/service_pkg.rs`（封装 pkg-env load/install 与脚本执行上下文）

这一层决定了一个 Service“能不能真正跑起来”。

## 4. 活：实例上报、端口后置发现与服务发现事实

### 4.1. 端口不是固定的：ServiceInstanceReportInfo 才是事实

`doc/arch/04_scheduler.md` 特别强调了一个容易踩坑的点：

- scheduler 在 node_config 里给端口更多是 hint
- 实例启动时按“先到先得” bind，最终端口由实例决定
- 实例把真实端口写入 `ServiceInstanceReportInfo.service_ports`
- scheduler 汇总 instance report，写回 `services/<service>/info` 作为服务发现事实

这条管线的结论非常明确：

“客户端应该如何访问服务”的唯一依据是 `services/<service>/info`，而不是某个配置里看起来像固定的端口。

### 4.2. 访问路径选择：兼容优先，其次性能

运行时（`doc/arch/08_api_runtime.md`）把访问方式分成三层：

1) 公网/通用：`https://$zone_host/kapi/<service_name>`（典型：AppClient）
2) 最大兼容：`http://127.0.0.1:3180/kapi/<service_name>`（NodeGateway；典型：AppService）
3) 最佳性能：直连本机实例端口或直连 rtcp（典型：Kernel/KernelService）

核心实现入口：`src/kernel/buckyos-api/src/runtime.rs` 的 `get_zone_service_url()`。

你可以把它理解为：

- “访问入口”尽量稳定（NodeGateway 的 3180 / ZoneGateway 的 hostname）
- “怎么到达目标实例”由网关与 runtime 来解释（直连/rtcp/中转）

## 5. 死：停止、异常与收敛回路

在 BuckyOS 里，Service 的“死”同样是一条数据链路，而不是一次性命令：

- 控制面把目标状态写入 system-config（最终体现在 node_config 的 target_state）
- scheduler 把变化推导成新的 node_config
- node-daemon 通过 `ensure_run_item_state()` 把本机实例拉到 Stopped/Exited

当实例异常退出时，实例上报会改变，scheduler 下一轮会看到 input 变化，并可能产生新的动作（例如重新部署/重新调度/标记异常）。
这也是 BuckyOS 强调“收敛循环”的原因：系统不依赖一次性的成功，而依赖持续收敛到目标状态。

## 6. 再生：升级触发与 deploy loop 的边界

升级在 BuckyOS 里有两类触发（`doc/arch/05_delivery_pkg_system.md`）：

1) node_config 指定精确版本：升级/降级都是目标状态变更
2) node_config 指定非精确版本：由 pkg-index-db 最新版本变化触发（latest / semver range）

node-daemon 通过同步 index-db、load pkg、执行 deploy/start 脚本，把升级落到本机。

但如果你看到 node-daemon 出现“NotExist → deploy 失败 → 循环”，它往往不是 deploy 脚本本身坏了，而是：

- repo-server 的 ready 没完成
- 或者 index-db 视图不一致（索引领先于内容 / 节点与 repo-server 看到的索引不同）

理解这一点，才能在真实环境里把“Service 的生与死”当作一条可排障、可解释、可自动修复的工程链路。
