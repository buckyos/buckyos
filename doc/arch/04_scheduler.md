# 04. scheduler（确定性调度 / 写回 node_config、service_info、rbac）

scheduler 的定位是“确定性的状态推导器（deterministic derivation engine）”：
- 输入：system-config（KV）里的系统当前状态：用户/设备/服务 spec/节点配置/实例上报等。
- 输出：一组可执行的调度动作（instances、service_info、rbac policy、gateway 配置等），并写回 system-config，形成新的“唯一真相源”。

在 BuckyOS 里，scheduler 的价值不只是 placement，而是把“能跑起来 + 能被访问 + 谁能访问”的一致性配置一次性推导出来（这也是它与常见系统最大的差异点之一，见 `new_doc/arch/01_overview.md`）。

## 读者的心智模型：scheduler 是一个“读-算-写”的纯函数循环

notepads 明确强调了调度器的幂等性：相同输入应得到相同输出（`new_doc/ref/notepads/scheduler.md`）。
实现里也尽量把 scheduler 组织成一种纯函数式形态：
- “输入快照”来自 system-config dump。
- “输出动作”被翻译为 KV 事务写回。
- scheduler 自己的内部状态也落盘为 snapshot，作为下一轮的对比基准。

相关代码锚点：
- 主循环与写回：`src/kernel/scheduler/src/system_config_agent.rs`
- 核心调度逻辑（NodeScheduler / SchedulerAction）：`src/kernel/scheduler/src/scheduler.rs`

## 关键数据结构（理解 scheduler 必须知道的最小视图）

本节只列出“看懂主链路所必需”的字段与用途。

### 1) node_config（scheduler 写；node-daemon 读）
对应定义：`src/kernel/buckyos-api/src/control_panel.rs`
- `NodeConfig.kernel/apps/frame_services`：该节点应该运行哪些实例、目标状态是什么。
- `NodeConfig.state`：节点目标状态（Running/Stopped/Maintenance...）。

### 2) instance 上报（node-daemon 写；scheduler 读）
对应定义：`src/kernel/buckyos-api/src/app_mgr.rs`
- `ServiceInstanceReportInfo.service_ports`：实例“最终监听端口”的事实来源（端口后置发现）。
- `ServiceInstanceReportInfo.state/last_update_time/pid`：健康与可用性判断的基础。

### 3) service_info（scheduler 写；client/runtime/gateway 读）
对应定义：`src/kernel/buckyos-api/src/app_mgr.rs`
- `ServiceInfo.selector_type`：当前实现主要是 random。
- `ServiceInfo.node_list`：每个节点上的 endpoint 信息（包含 `ServiceNode.service_port`）。

### 4) scheduler snapshot（scheduler 写；scheduler 读）
- snapshot KV key：`system/scheduler/snapshot`（读写发生在 `src/kernel/scheduler/src/system_config_agent.rs`）。
- snapshot 数据结构：`NodeScheduler`（定义在 `src/kernel/scheduler/src/scheduler.rs`）。

这份 snapshot 的目的不是“容灾备份”，而是：让下一轮调度可以对比上一轮结果，减少无效写入，并保持推导过程可解释。

## 关键 KV 路径（从 system-config 的视角理解 scheduler）

scheduler 主要在这些路径上读写（以当前实现为准）：
- `nodes/<node_id>/config`：写 node_config（由 node-daemon 执行收敛）。
- `services/<service>/instances/<node_id>`：读实例上报（`ServiceInstanceReportInfo`，由 node-daemon 写）。
- `services/<service>/info`：写 service_info（由 selector/runtime/gateway 读取）。
- `system/rbac/policy`：写 RBAC policy（需要时刷新）。
- `nodes/<node_id>/gateway_config`：写 node_gateway_config（按 service_info 重建）。
- `system/scheduler/snapshot`：读写 scheduler 快照。

上面的读写逻辑集中在 `src/kernel/scheduler/src/system_config_agent.rs`，service_info 的写回在 `src/kernel/scheduler/src/service.rs`。

## Scheduler Loop 伪代码（基于当前实现）

注意：`new_doc/ref/notepads/scheduler.md` 里描述“等待 10 秒或唤醒”，而当前实现是 5 秒一个 tick（`src/kernel/scheduler/src/system_config_agent.rs`）。

对应实现：`src/kernel/scheduler/src/system_config_agent.rs` 的 `schedule_loop`。

```text
schedule_loop(is_boot):
  loop:
    sleep(5s)

    input = system_config.dump_configs_for_scheduler()

    // 1) 构造 scheduler_ctx（NodeScheduler）
    (scheduler_ctx, device_list) = create_scheduler_by_system_config(input)

    // 2) 读取上一次调度的 snapshot
    last_snapshot = input.get("system/scheduler/snapshot")

    // 3) 核心调度：基于当前 ctx 与 last_snapshot 推导 action_list
    action_list = scheduler_ctx.schedule(last_snapshot)

    // 4) 将 action_list 翻译为 KVAction（node_config / service_info / ...）
    tx_actions = {}
    need_update_gateway_node_list = {}
    need_update_rbac = false
    for action in action_list:
      tx_actions += schedule_action_to_tx_actions(action, ...)

    // 5) 拓扑变化时，额外刷新 RBAC / gateway_config
    if is_boot or last_snapshot is None:
      need_update_rbac = true

    if last_snapshot exists and (nodes/specs/users changed):
      need_update_rbac = true
      if nodes changed:
        need_update_gateway_node_list = all_nodes

    if need_update_rbac:
      tx_actions += update_rbac(input, scheduler_ctx)

    if need_update_gateway_node_list not empty:
      tx_actions += update_node_gateway_config(need_update_gateway_node_list, scheduler_ctx, input)

    // 6) 事务写回
    system_config.exec_tx(tx_actions)

    // 7) 保存 snapshot（用于下一轮对比）
    system_config.write("system/scheduler/snapshot", serialize(scheduler_ctx))

    if is_boot:
      break
```

其中：
- action → service_info 的写回发生在 `src/kernel/scheduler/src/service.rs` 的 `update_service_info()`。
- action → node_gateway_config 的重建发生在 `src/kernel/scheduler/src/system_config_agent.rs` 的 `update_node_gateway_config()`（会遍历 `scheduler_ctx.service_infos`）。

## 端口分配 + 实例上报 -> service_info 的管线（必须显式理解）

这是 BuckyOS 在家庭/边缘环境里最关键、也最容易被误解的一条链路（范式来源：`new_doc/ref/notepads/scheduler.md`）。

### 1) scheduler 在 instance 侧给出“端口 hint”，但不承诺最终端口
- 在调度阶段，scheduler 会为 ReplicaInstance 构造 `service_ports`（更像端口 hint / 期望端口），其分配逻辑可见 `src/kernel/scheduler/src/scheduler.rs` 的 `alloc_replica_instance_port()`。
- 这些端口信息会进入 node_config（`src/kernel/scheduler/src/system_config_agent.rs` 会把 node_config 作为 KVAction 写回到 `nodes/<node_id>/config`）。

### 2) node-daemon/实例启动时按“先到先得” bind，随后上报真实端口
- 实例启动后，实际监听端口由实例自身决定（从起始端口开始尝试 bind，成功即使用），并写入上报结构的 `ServiceInstanceReportInfo.service_ports`。
- `ServiceInstanceReportInfo` 定义见 `src/kernel/buckyos-api/src/app_mgr.rs`。

### 3) scheduler 汇总 instance report，生成 service_info 作为“服务发现事实”
- scheduler dump system-config 时会读到 `services/<service>/instances/<node_id>`，并把 `ServiceInstanceReportInfo.service_ports` 纳入调度上下文（见 `src/kernel/scheduler/src/system_config_agent.rs` 的 `create_scheduler_by_system_config()`）。
- 随后调度器会生成 `SchedulerAction::UpdateServiceInfo`，并最终写回 `services/<service>/info`（见 `src/kernel/scheduler/src/service.rs` 的 `update_service_info()`）。
- 写回的 `ServiceInfo/ServiceNode` 结构定义见 `src/kernel/buckyos-api/src/app_mgr.rs`。

这条管线的结论是：
- “客户端应该如何访问服务”的唯一依据是 `services/<service>/info`（service_info），而不是某个配置里看起来“像是固定”的端口。

## RBAC 由 scheduler 生成/刷新（但存在传播延迟）

RBAC 的 model/base_policy 初始化发生在 boot 阶段，随后 policy 通常由 scheduler 构造/刷新：
- 概念说明：`new_doc/ref/notepads/rbac.md`
- boot 写入 base_policy/model：`src/kernel/scheduler/src/main.rs`
- 常规循环更新 policy：`src/kernel/scheduler/src/system_config_agent.rs`（`update_rbac()`）

## 常见坑（与当前实现强相关）

### 1) stale policy：RBAC “不是立刻生效”（约 10s cache）
- 权限的读取带 cache，存在“旧 policy 还能被用一小段时间”的窗口（notepads 记录最长约 10s）。
- 现象与定位建议见 `new_doc/arch/09_pitfalls.md`，概念来源见 `new_doc/ref/notepads/rbac.md`。

### 2) 不要过度假设“端口固定”
- service spec 里的端口更像 hint；最终端口以 `ServiceInstanceReportInfo.service_ports` 与 `ServiceInfo.node_list[*].service_port` 为准。
- 范式说明见 `new_doc/ref/notepads/scheduler.md`；结构定义见 `src/kernel/buckyos-api/src/app_mgr.rs`。

### 3) snapshot churn：无效写入会放大 system-config 压力
- scheduler 会周期性 dump 全量配置，并写入 `system/scheduler/snapshot`（`src/kernel/scheduler/src/system_config_agent.rs`）。
- 如果 snapshot 里包含“变化密度很大”的字段（例如把高频资源监控数据塞进 scheduler 输入），会导致每轮都认为状态改变，进而触发无效写回。
- notepads 也提醒过“如何判断系统状态未变化”的困难（`new_doc/ref/notepads/scheduler.md`）。
