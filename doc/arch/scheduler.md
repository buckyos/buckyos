# scheduler

这份文档只写当前仓库里的 scheduler 实现细节，默认读者已经知道 BuckyOS 的基本概念。对外说明看编号文档；这里主要回答三个问题：

- 现在的代码到底怎么跑
- 哪些行为是当前实现特有的，不要想当然
- 改 scheduler 时哪些文件必须一起动

## 代码锚点

- 核心状态机：`src/kernel/scheduler/src/scheduler.rs`
- 调度循环 / snapshot / KV 写回：`src/kernel/scheduler/src/system_config_agent.rs`
- `services/<spec>/info` 写回：`src/kernel/scheduler/src/service.rs`
- 测试：`src/kernel/scheduler/src/scheduler_test.rs`

## 当前实现的边界

当前 scheduler 的 contract 很简单：

- 输入：`create_scheduler_by_system_config()` 从 system_config dump 组装出的 `NodeScheduler`
- 输出：`Vec<SchedulerAction>`
- 落地：`schedule_action_to_tx_actions()` 把 action 翻译成 KV 事务
- 快照：`system/scheduler/snapshot`

`scheduler.rs` 头部注释里那句“纯函数式调度核心”基本就是贡献者该遵守的边界：

- 调度核心只负责“给定快照推导动作”
- 不在核心逻辑里直接写 system_config
- 不把执行细节、业务流程细节继续塞进 scheduler

## schedule loop 现状

实际 loop 在 `schedule_loop()`，当前是每 5 秒一轮，不是旧文档里的 10 秒。

每轮做的事情：

1. dump system_config
2. 组装 `NodeScheduler`
3. 读取 `system/scheduler/snapshot`
4. 调 `scheduler.schedule(last_snapshot)`
5. action -> KVAction
6. 视情况附加 RBAC / gateway_config 更新
7. `exec_tx()`
8. 保存新的 snapshot

支持 boot 单轮模式和常驻 loop 模式。

## `NodeScheduler::schedule()` 的真实行为

### Step1: `resort_nodes()`

这里只做很窄的节点状态收敛：

- `New -> Prepare`
- `Removing -> Deleted`

没有更复杂的 node maintenance / drain / replace 逻辑。

另一个要记住的行为：

- 小系统优化已经启用
- 当节点数 `<= 7` 且 Step1 产生 action，本轮直接返回，不继续做 Step2/Step4

所以小系统里新节点刚加入时，spec 实例化可能天然延后一轮。

### Step2: `schedule_spec_change()`

这里只在 `is_spec_changed(last_snapshot)` 为真时执行。

当前真正会触发 Step2 的只有这些差异：

- spec 数量变化
- `state`
- `required_cpu_mhz`
- `required_memory`
- `node_affinity`
- `network_affinity`

注意：下面这些字段现在**不会**触发 Step2：

- `best_instance_count`
- `need_container`
- `required_gpu_tflops`
- `required_gpu_mem`
- `service_ports_config`

也就是说，如果你改了这些字段并期望重新分配实例，必须先补 `is_spec_changed()`。

Step2 当前只处理两种 spec 状态：

- `New`
- `Deleted`

对应行为：

- `New`：选点、创建 `ReplicaInstance`、把 spec 改成 `Deployed`
- `Deleted`：删除关联实例、把 spec 保持为 `Deleted`

当前没有“Deployed steady-state reconcile”这层能力。几个直接后果：

- `Deployed` 服务不会因为 `best_instance_count` 改变而自动扩缩
- 实例掉线后，当前实现只会在 Step4 把它从 `service_info` 里摘掉，不会自动补新实例
- 纯节点拓扑变化不会触发现有实例重排

这是当前实现最容易被误判的地方。

### 放置算法

当前放置逻辑仍然很朴素：

- 过滤：`filter_node_for_instance()`
- 打分：`score_node()`

过滤条件目前只有：

- `node.state == Ready`
- `node_type` 必须是 `OOD` 或 `Server`
- 需要容器时必须 `support_container`
- CPU / 内存 / GPU 资源足够
- `node_affinity` 命中

打分只看三类因素：

- 分配后剩余资源比例
- 当前空闲比例
- `network_affinity` 命中加分

当前已实现一个影子资源账本 `shadow_nodes`：

- 同轮多个 spec 分配时，先在影子节点里扣减资源
- 避免同一轮多个新服务把同一节点超卖

### Step3

还没做。别把旧文档里的资源动态调整、迁移等内容当成现状。

### Step4: `calc_service_infos()`

这里只负责对外发布的 `ServiceInfo`，不负责补实例。

判定条件：

- 只认 `InstanceState::Running`
- `now - last_update_time < 90`

如果实例不满足上面两个条件，就只会发生一件事：

- 不再进入 `service_info`

不会自动触发新的实例化。

启动期还有一个明确的 bootstrap 行为：

- 创建实例时直接给 `last_update_time` 写当前时间
- 用来打破早期服务发现依赖环
- 后续真实心跳会接管这个值

## `SchedulerAction` 到 KV 的映射

主要看 `schedule_action_to_tx_actions()`：

- `ChangeNodeStatus` -> `nodes/<node>/config.state`
- `ChangeServiceStatus` -> `services/<spec>/spec` 或 `users/<uid>/apps/<app>/spec`
- `InstanceReplica` -> 写目标节点 `node_config`
- `RemoveInstance` -> 从目标节点 `node_config` 删除实例
- `UpdateServiceInfo` -> 重写 `services/<spec>/info`

额外副作用不在 `SchedulerAction` 里，而是在 loop 里附加：

- 刷新 `system/rbac/policy`
- 刷新 `nodes/<node>/gateway_config`

所以如果你新增 action，除了 `scheduler.rs`，还要记得补写回层。

## 现在不要碰的约定

### 不要手改 `replica_instances` / `service_infos`

对外控制系统状态的入口仍然应该是：

- 改 spec
- 改节点目标状态

不要把 runtime 或 control_panel 的新逻辑做成“直接改 instance / service_info”。下一轮调度会把这类手改覆盖掉，或者让 snapshot 对比变得很难解释。

### 端口事实以 `service_info` 为准

当前链路里：

- 实例创建时写的是目标端口配置
- 真实可访问端口以后续上报和 `service_info` 汇总结果为准

如果你在 gateway / selector / runtime 侧接入服务发现，不要把 `node_config` 里的端口当最终事实。

## 改代码时通常要一起改的地方

### 改调度输入字段

至少检查这几处：

- `scheduler.rs` 里的核心结构
- `create_scheduler_by_system_config()`
- `is_spec_changed()`
- 对应测试

如果少改了 `is_spec_changed()`，很容易出现“字段改了但 scheduler 没反应”。

### 改实例化行为

至少检查：

- `schedule_spec_change()`
- `filter_node_for_instance()`
- `score_node()`
- `instance_app_service()` / `instance_service()`
- `RemoveInstance` 对应的 uninstance 路径

### 改服务发现或入口路由

至少检查：

- `calc_service_infos()`
- `service.rs::update_service_info()`
- `update_node_gateway_config()`

## 当前缺失能力

按现在代码，下面这些都还不是完整能力：

- OPTask 调度闭环
- `UpdateInstance` 的完整落地
- `Deployed` 状态下的副本自愈 / 自动扩缩
- 实例迁移
- 资源动态调整
- `Disable` / `Abnormal` 的完整收敛
- 复杂节点运维流程

如果你要做其中任何一项，最好先把“当前没有这层 reconcile”写清楚，再动实现。

## 和 `function_instance` 文档的关系

新的 `function_instance` 文档对这个文件最有用的地方，不是理论，而是给了一个明确约束：

- 调度器应该继续做“确定性分配器”
- 工作流依赖、业务语义、人机交互不该继续下沉到 scheduler
- cache / memoization 也不该变成 scheduler 的内部状态

对当前代码来说，可以直接落成两个贡献者约束：

### 1. 把 `CreateOPTask` 当遗留占位，不要继续做重

`SchedulerAction::CreateOPTask` 现在不是稳定主路径。新需求如果继续往 OPTask 里塞复杂流程，只会让后续迁移更难。

### 2. 新的异步执行语义应尽量保持“显式输入 + 显式资源 + 幂等边界”

这和 `function_instance` 的方向一致，也和当前 `NodeScheduler` 的纯推导结构兼容。未来真的切到 `fun_instance`，更可能替换的是执行原语和执行层，而不是把 scheduler 再做成一个懂业务编排的中心。

## 相关文档

- 对外架构说明：`doc/arch/04_scheduler.md`
- 当前代码的最终真相：`src/kernel/scheduler/src/scheduler.rs`
- 下一阶段方向：`doc/arch/使用function_instance实现分布式调度器.md`
