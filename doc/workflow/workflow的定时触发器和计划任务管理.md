# Workflow 的定时触发器和计划任务管理

## 1. 背景与结论

OpenDAN 和 Workflow 都需要“计划任务”能力。例如：

```text
每天凌晨 3 点，扫描一下新增的图片。
```

这个需求表面是定时器，实际包含一组可恢复的业务流程：

```text
到点触发
  -> 找出上次扫描后新增的图片
  -> 分批处理图片
  -> 记录 cursor / last_scan_at
  -> 汇总结果
  -> 失败重试 / 下次补偿
  -> 必要时通知用户
```

因此计划任务不应简单放进 OpenDAN，也不应让 TaskMgr 从被动账本变成主动调度器。更合理的分层是：

- **Workflow 管 trigger / schedule 的真相源与编排语义**：cron、timezone、misfire、next_fire_at、fire record、subtask creation、补偿策略。
- **TaskMgr 保持被动任务总账**：每个 schedule 必须镜像为 root task；每次 fire 在 root task 下创建一个按 schema 渲染的 subtask，发布状态事件。
- **OpenDAN AgentTool CLI 只做入口**：提供 crontab-compatible 的命令行工具，把规则提交给 Workflow schedule API。

一句话结论：

> 计划任务的通用 owner 应是 Workflow 的 Schedule/Trigger 层；TaskMgr 是可观察面和执行账本；OpenDAN 使用 crontab 工具，不拥有调度系统。

## 2. 设计目标

1. 支持 crontab 风格的周期触发，包括标准 5 字段 cron 和常用 `@daily` / `@hourly` 等别名。
2. 支持一次性（run_at)、周期性 (run_every)、禁用、恢复、删除、手动 run now。
3. 支持 Workflow 定义的定时触发，也支持 OpenDAN command 这类非 Workflow subtask。
4. 保持 TaskMgr 被动：TaskMgr 不负责解析 cron，不负责扫描 due schedule，不负责派生下一次 run。
5. 每一次执行都落入 TaskMgr 任务树，天然获得状态、事件、权限、进度、UI、历史追踪。
6. 支持 missed run 补偿策略，避免服务重启或机器休眠后丢失关键周期任务。
7. 支持幂等 run creation，避免同一个 fire_time 被重复触发。

## 3. 非目标

1. 不在 OpenDAN 内实现独立计划任务数据库。
2. 不把 TaskMgr 改造成通用 cron scheduler。
3. 不要求所有 schedule 都必须执行 Workflow DAG；简单 OpenDAN command 可以作为 fire subtask template。
4. P0 不实现复杂日历语义，例如每月最后一个工作日、节假日、工作日历。
5. P0 不支持秒级 cron；默认使用传统 crontab 的分钟粒度。

## 4. 分层模型

### 4.1 Workflow Schedule Store

Workflow Service 新增 schedule/trigger 存储，作为计划任务定义的真相源。

它负责：

- 保存 schedule definition。
- 解析 cron 并计算 `next_fire_at`。
- 在服务启动和周期 tick 时扫描 due schedule。
- 根据 misfire 策略补偿错过的 fire。
- 幂等创建 schedule fire / fire subtask。
- 把 schedule 和每次执行镜像到 TaskMgr。

### 4.2 TaskMgr 镜像与任务树

每个 schedule 必须镜像为一个 TaskMgr root task。root task 创建失败时，enabled schedule 创建不能静默成功；fire 时 root task 不可用也必须返回明确错误或把 schedule 置为 `error`。

```text
workflow/schedule: scan-new-images daily 03:00
└── fire subtask: 2026-05-27T03:00:00-07:00
    ├── task_type = workflow/run | agent.delegate | workflow.send_message | ...
    └── data = render(template.data_template, fire context)
```

TaskMgr 中的 schedule root task 是可观察面，不是调度真相源。Workflow 更新它的状态和 `data.schedule` 摘要。fire subtask 创建后，由拥有该 `task_type` 的 Service 接管执行（beta2.2 起没有 TaskMgr runner 路由，执行者信息在 data 业务字段里）；schedule manager 不承担业务执行。

### 4.3 AgentTool CLI

OpenDAN 的计划任务 CLI 只和 Workflow schedule API 交互。对 Agent 来说，它是一个 crontab-compatible 工具；底层是否创建 workflow schedule、service schedule 或 command schedule 不暴露给 prompt。

## 5. Schedule 资源模型

建议新增一等资源 `WorkflowSchedule`：

```json
{
  "schedule_id": "sch_...",
  "owner": {
    "user_id": "did:...",
    "app_id": "opendan"
  },
  "name": "scan-new-images",
  "description": "每天凌晨 3 点扫描新增图片",
  "status": "enabled",
  "schedule": {
    "kind": "cron",
    "expr": "0 3 * * *",
    "timezone": "America/Los_Angeles",
    "calendar": "standard",
    "start_at": null,
    "end_at": null
  },
  "target": {
    "task_type": "workflow.run",
    "name_template": "workflow/run: ${schedule.name} [${fire.fire_id}]",
    "data_template": {
      "workflow_run": {
        "workflow_id": "wf_scan_images",
        "input": {
          "album": "camera-roll",
          "cursor_ref": "schedule_state.last_scan_cursor"
        },
        "trigger": {
          "schedule_id": "${schedule.schedule_id}",
          "fire_id": "${fire.fire_id}",
          "fire_time": "${fire.fire_time}",
          "manual": "${fire.manual}"
        }
      }
    }
  },
  "state": {
    "next_fire_at": "2026-05-28T03:00:00-07:00",
    "last_fire_at": "2026-05-27T03:00:00-07:00",
    "last_task_id": 456,
    "last_run_id": "run_...",
    "consecutive_failures": 0
  },
  "policy": {
    "misfire": "run_once",
    "max_parallel_runs": 1,
    "catch_up_limit": 1,
    "jitter_sec": 0
  },
  "task_mirror": {
    "root_task_id": 123,
    "root_id": "123"
  },
  "created_at": 1779870000,
  "updated_at": 1779870000
}
```

### 5.1 `status`

| 状态 | 说明 |
| --- | --- |
| `enabled` | 正常参与 due scan |
| `paused` | 保留定义但不触发 |
| `archived` | 软删除，不再触发，历史 run 仍可查 |
| `error` | schedule 定义或 target 校验失败，需要人工修复 |

### 5.2 `schedule.kind`

P0 支持：

| kind | 说明 |
| --- | --- |
| `cron` | crontab 5 字段表达式 |
| `once` | 一次性触发，触发后自动 archived 或 completed |

P1 可以扩展：

| kind | 说明 |
| --- | --- |
| `interval` | 每 N 分钟/小时触发 |
| `event` | 外部事件触发，和 cron 并列为 trigger |

### 5.3 `target` / fire subtask template

`target` 是 fire subtask template，而不是 schedule manager 内部的业务分支。字段含义：

| 字段 | 说明 |
| --- | --- |
| `task_type` | 触发时创建的 TaskMgr subtask 类型，例如 `workflow.run`、`agent.delegate`、`workflow.send_message` |
| `name_template` | fire subtask 名称模板，可引用 `${schedule.schedule_id}`、`${schedule.name}`、`${fire.fire_id}`、`${fire.fire_time}`、`${fire.manual}` |
| `data_template` | fire subtask 的 TaskData 模板，按 fire context 渲染后写入 subtask |

Workflow fire subtask 固定使用 `parent_id = schedule.task_mirror.root_task_id`，`root_id = schedule.task_mirror.root_id`。外部不允许在模板里覆盖这两个绑定。

`workflow.run` 是最重要的主路径。`agent.delegate` 用于 dcrontab task，`workflow.send_message` 用于 dcrontab remind。

fire record 只记录调度事实与 subtask 关联：`fire_id`、`fire_time`、`manual`、`status`、`task_id`、可选 `run_id`、`error`。业务执行结果不写入 fire record 总账，而是落在对应 subtask 的 status / data / message 中。

### 5.4 `policy.misfire`

| 策略 | 说明 |
| --- | --- |
| `skip` | 服务离线期间错过的触发不补偿，只计算下一次 |
| `run_once` | 如果错过一次或多次，只补一次，推荐默认值 |
| `catch_up` | 按错过的 fire_time 逐次补偿，受 `catch_up_limit` 限制 |
| `manual` | 标记为 missed，等待用户或 Agent 手动处理 |

默认使用 `run_once`，避免长期离线后瞬间创建大量 run。

## 6. 触发器执行语义

### 6.1 Due Scan

Workflow Service 内部有一个 schedule loop：

```text
loop:
  now = current_time()
  due_schedules = list enabled schedules where next_fire_at <= now
  for schedule in due_schedules:
    acquire schedule lock
    compute due fire_times with misfire policy
    create fire records idempotently
    create target run/request idempotently
    update next_fire_at / last_fire_at
    mirror to TaskMgr
  sleep until nearest next_fire_at or fallback interval
```

实现可以先用短周期 tick，后续再优化成精确 timer。这里的 timer 是 Workflow 内部实现细节，不是系统对外语义。

### 6.2 幂等 fire key

每次触发必须有稳定 fire key：

```text
fire_key = schedule_id + normalized_fire_time
```

对同一个 `fire_key`：

- 只能创建一个 fire record。
- 只能创建一个主 run request。
- 重启后重复扫描不得重复执行。

### 6.3 并发控制

`max_parallel_runs` 控制同一 schedule 的同时运行数。

- `max_parallel_runs = 1` 时，如果上一轮未终态，下一轮按 misfire 策略处理。
- 对“扫描新增图片”这类增量任务，默认应为 1，避免 cursor 并发写冲突。
- 如果用户明确设置大于 1，Workflow 必须要求 target 声明可并发或幂等边界。

### 6.4 Run Input 模板

创建 Workflow Run 时应注入 trigger 上下文：

```json
{
  "trigger": {
    "kind": "schedule",
    "schedule_id": "sch_...",
    "fire_time": "2026-05-27T03:00:00-07:00",
    "cron": "0 3 * * *",
    "timezone": "America/Los_Angeles",
    "manual": false
  }
}
```

target input 可以引用 schedule state，但引用解析由 Workflow 完成，不交给 TaskMgr。

## 7. TaskMgr 映射

### 7.1 Root Task

创建 schedule 时，Workflow 可创建或更新一个 TaskMgr root task：

```json
{
  "name": "workflow/schedule/scan-new-images",
  "task_type": "workflow/schedule",
  "status": "Running",
  "data": {
    "schedule": {
      "schedule_id": "sch_...",
      "name": "scan-new-images",
      "expr": "0 3 * * *",
      "timezone": "America/Los_Angeles",
      "status": "enabled",
      "next_fire_at": "2026-05-28T03:00:00-07:00",
      "last_fire_at": "2026-05-27T03:00:00-07:00"
    },
    "workflow": {
      "workflow_id": "wf_scan_images"
    }
  }
}
```

状态映射：

| Schedule status | Task status |
| --- | --- |
| `enabled` | `Running` |
| `paused` | `Paused` |
| `archived` | `Canceled` 或保留 `Completed`，由 UI 策略决定 |
| `error` | `Failed` |

### 7.2 Execution Task

每次触发后，Workflow 创建 run task 并挂到 root task 下：

```text
parent_id = schedule root task id
root_id = schedule root task root_id
task_type = workflow/run
```

这样 UI 能按 schedule root 看到全部历史执行。

### 7.3 TaskMgr 需要补充的能力

当前 TaskMgr 主要发布更新事件。为了让 schedule root 下的新 run 能被订阅方及时发现，建议补：

1. task create event：创建 task 后发布 `change_kind = "create"`。
2. root fanout：child task create event 也发布到 `/task_mgr/{root_id}`。
3. create event payload 至少包含 `task_id`、`parent_id`、`root_id`、`task_type`、`data` 摘要。

这仍然是被动事件能力，不要求 TaskMgr 主动调度。

## 8. API 需求

Workflow Service 建议新增以下 RPC：

| Method | 说明 |
| --- | --- |
| `workflow.create_schedule` | 创建 schedule，返回 `schedule_id` 和 root task |
| `workflow.update_schedule` | 修改 cron、timezone、target、policy 等字段 |
| `workflow.get_schedule` | 获取单个 schedule |
| `workflow.list_schedules` | 按 owner/status/target/name 过滤 |
| `workflow.pause_schedule` | 暂停 |
| `workflow.resume_schedule` | 恢复并重新计算 next_fire_at |
| `workflow.archive_schedule` | 软删除 |
| `workflow.run_schedule_now` | 手动触发一次，生成 fire record 和 run |
| `workflow.get_schedule_history` | 查询 fire/run 历史 |
| `workflow.validate_schedule` | 解析 cron、计算后续触发时间、校验 target |

### 8.1 `validate_schedule`

该接口给 Agent CLI 和 UI 使用，不产生持久副作用。

返回示例：

```json
{
  "valid": true,
  "normalized_expr": "0 3 * * *",
  "timezone": "America/Los_Angeles",
  "next_fire_times": [
    "2026-05-28T03:00:00-07:00",
    "2026-05-29T03:00:00-07:00",
    "2026-05-30T03:00:00-07:00"
  ],
  "warnings": []
}
```

## 9. Crontab 兼容要求

P0 支持：

```text
* * * * *
@hourly
@daily
@weekly
@monthly
@yearly
@annually
@reboot
TZ=Asia/Shanghai
CRON_TZ=America/Los_Angeles
```

约束：

- `@reboot` 映射为 Workflow service 启动后的一次 trigger，必须幂等。
- 环境变量行只影响后续 crontab 行，导入时要固化到 schedule timezone/env。
- 不支持传统 crontab 的 `%` stdin 语义，P0 明确报错。
- 不支持 username 字段的 system crontab 格式，P0 只支持 user crontab 五字段。

## 10. 示例：每天凌晨 3 点扫描新增图片

用户通过 Agent CLI 创建：

```bash
agent_tool crontab add "0 3 * * *" --name scan-new-images --workflow wf_scan_images --input album=camera-roll
```

底层创建：

```json
{
  "name": "scan-new-images",
  "schedule": {
    "kind": "cron",
    "expr": "0 3 * * *",
    "timezone": "America/Los_Angeles"
  },
  "target": {
    "kind": "workflow.run",
    "workflow_id": "wf_scan_images",
    "input": {
      "album": "camera-roll"
    }
  },
  "policy": {
    "misfire": "run_once",
    "max_parallel_runs": 1
  }
}
```

Workflow 到点创建 run。Workflow 定义内部负责：

- 读取上次 cursor。
- 列出新增图片。
- 并发 map 处理。
- 写回 cursor。
- 汇总与通知。

TaskMgr 只展示和记录：

- 这个 schedule 的当前状态。
- 每一次 run 的执行状态。
- 每个 step / shard 的进度和结果。

## 11. 实现建议

P0 最小闭环：

1. Workflow schedule store：SQLite/RDB 或现有 store，保存 `WorkflowSchedule`。
2. cron parser：优先用已有 Rust crate；新增依赖前需要确认。
3. schedule loop：周期扫描 due schedule。
4. schedule API：实现 create/list/show/pause/resume/archive/run_now/validate。
5. TaskMgr mirror：创建 `workflow/schedule` root task 和 `workflow/run` child task。
6. Agent CLI：实现 crontab-compatible facade。
7. DV test：创建每分钟 schedule，验证到点创建 run 和 TaskMgr 任务树。

P1：

1. import/export crontab。
2. missed run 的 `manual` 和 `catch_up` 完整策略。
3. UI schedule 管理页。
4. event trigger 与 cron trigger 统一。
5. schedule-level audit log。

## 12. 验收标准

1. 用户能创建 `0 3 * * *` 的 Workflow schedule。
2. Workflow Service 重启后不会丢失 schedule。
3. 到点后只创建一个对应 fire_time 的 run。
4. TaskMgr 中能看到 schedule root task 和 run 子任务。
5. 暂停 schedule 后不会继续触发。
6. 恢复 schedule 后正确计算下一次触发时间。
7. missed run 默认只补一次。
8. OpenDAN CLI 不需要知道 TaskMgr 细节即可创建和管理计划任务。
