# TaskManager 服务（TaskMgr 2.0）

TaskMgr 2.0 子系统：为整个 BuckyOS 提供统一、稳定、可寻址的 `Task` 抽象。
总设计规范见 `doc/task_mgr/task-mgr 2.0.md`。

一个 Task 从"已承诺处理"到"由机器或人完成"始终是同一个公开对象
（`/tasks/{task_id}`），不因分发、换 Runner、等待授权或暂停而切换。

## 模块结构

```text
TaskMgr 2.0 subsystem
├── Task Core（/kapi/task-manager, RDB: task-mgr-main, schema v7）
│   ├── server.rs      命令式状态写入、控制请求、权限计算、KEvent 发布
│   ├── task_store.rs  CAS 事务命令层：一次性 Result、Terminal 吸收态、
│   │                  runner epoch fencing、每次变更一条 task_event
│   ├── acl.rs         Policy v1 计算（additive allow grants + boundary）
│   └── json_schema.rs Input/Result 的最小 JSON-Schema 子集校验
└── Task Dispatch Center（/kapi/task-dispatcher, RDB: task-dispatcher-main, v3）
    ├── dispatch_db.rs 稳定队列顺序、DeliveryAttempt 日志、注册与租约
    └── service.rs     可恢复 Saga（先建 Task 后排队）、确定性投递
                       （offer -> bind -> activate）、startup recovery、
                       取消收敛 sweep
```

## 关键协议事实

- 共享类型与客户端在 `buckyos-api`（`task_mgr.rs` / `task_dispatcher.rs`）。
- Task ID 是 URL-safe opaque string；调用方不得解析。
- Input 创建后不可变；Result 一次性提交；重新执行 = 新 idempotency key 新 Task。
- App Runner 写操作携带 `(app_instance_id, runner_epoch, expected_revision)` 双重 fencing。
- Runner 不轮询、不 claim：Dispatcher 按注册的 RunnerFunction 主动
  `offer_task`/`activate_task`（幂等，activate 前禁止业务副作用）。
- KEvent（`/task_mgr/{task_id}`、`/task_mgr/tree/{root_id}`）只是加速；
  真相源是 RDB snapshot + `task_event`。

## 测试

```bash
cargo test -p task_manager
```

覆盖：Result/Terminal/epoch/revision 并发竞态、HumanSet 单赢家提交、树级
控制传播、ACL scope/boundary/字段裁剪、schema 校验、Dispatcher saga 崩溃
恢复、Busy/Rejected/超时重排队、审批门与取消收敛。
